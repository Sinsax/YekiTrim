//! HTTP 响应的解压（**B站的怪癖**：裸 deflate）。
//!
//! ## 为什么不是给 reqwest 加 feature
//!
//! B站弹幕接口（`comment.bilibili.com/{cid}.xml`）返回 `Content-Encoding: deflate`，
//! 但载荷是 **raw deflate（zlib wbits = -15）** —— 没有 zlib 头、没有 adler32 校验。
//! 这**不符合 HTTP 规范**（规范要求 deflate = zlib 包装）。后果：
//!
//! - `reqwest` 的 `deflate` feature 按规范走 zlib 包装，**解不开**；
//! - 实测 `Accept-Encoding: identity` 会被服务端**忽略**，强制压缩；
//! - 所以只能手工解，而且要按 raw 风味优先试。
//!
//! 解码器用 `flate2`（已在 `Cargo.lock`，经 png/ureq/zip 引入 —— 提升为直接依赖不引入新传递依赖）。

use std::io::Read;

use anyhow::{Context, Result};

/// 解压选择的结果，用于日志与测试（出问题时能一眼看出走的是哪条路）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    /// 载荷本来就是明文（服务端没压缩）
    Plain,
    /// 裸 deflate（B站实际行为）
    RawDeflate,
    /// zlib 包装（规范行为，兜底用）
    Zlib,
    /// gzip
    Gzip,
}

/// 解开一个可能被压缩的响应体。
///
/// 顺序刻意是"**先看魔数，再用 B站的实际风味兜底**"：
/// 1. 明文（`<?xml` / `<i>` / 合法 UTF-8 文本）→ 原样返回，服务端哪天不压缩了也不会炸；
/// 2. gzip / zlib 有固定魔数 → 直接按它解；
/// 3. 否则按**裸 deflate** 解（B站走的就是这条）；
/// 4. 裸 deflate 也失败 → 再试 zlib（有些中间层会加上头）。
///
/// 任何一步都不 panic：垃圾字节返回 `Err`，由调用方决定是报错还是重试。
pub fn inflate(bytes: &[u8]) -> Result<(Vec<u8>, Flavor)> {
    if bytes.is_empty() {
        return Ok((Vec::new(), Flavor::Plain));
    }

    // 1) 已经是明文？只认窄判据（XML 声明或根标签开头），避免把二进制误判成明文。
    let head = bytes.iter().take(64).copied().collect::<Vec<u8>>();
    if let Ok(text) = std::str::from_utf8(&head) {
        let t = text.trim_start_matches('\u{feff}').trim_start();
        if t.starts_with("<?xml") || t.starts_with("<i>") || t.starts_with("<i ") {
            return Ok((bytes.to_vec(), Flavor::Plain));
        }
    }

    // 2) 有魔数的格式：gzip = 1f 8b；zlib = 78 {01,5e,9c,da}
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(bytes)
            .read_to_end(&mut out)
            .context("gzip 解压失败")?;
        return Ok((out, Flavor::Gzip));
    }
    if bytes.len() >= 2 && bytes[0] == 0x78 && matches!(bytes[1], 0x01 | 0x5e | 0x9c | 0xda) {
        if let Some(out) = try_zlib(bytes) {
            return Ok((out, Flavor::Zlib));
        }
    }

    // 3) B站的实际风味：裸 deflate
    if let Some(out) = try_raw_deflate(bytes) {
        return Ok((out, Flavor::RawDeflate));
    }

    // 4) 兜底：也许是个带头的 zlib（某些 CDN 会重包装）
    if let Some(out) = try_zlib(bytes) {
        return Ok((out, Flavor::Zlib));
    }

    anyhow::bail!(
        "无法解压响应（{} 字节，头 4 字节 = {}）：既不是明文 XML，也不是 deflate/zlib/gzip",
        bytes.len(),
        bytes
            .iter()
            .take(4)
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

fn try_raw_deflate(bytes: &[u8]) -> Option<Vec<u8>> {
    decode_strict(bytes, false)
}

fn try_zlib(bytes: &[u8]) -> Option<Vec<u8>> {
    decode_strict(bytes, true)
}

/// **严格**解码：必须读到流结束（`Status::StreamEnd`）**且**把输入消费干净。
///
/// ⚠️ 不要用 `flate2::read::*Decoder::read_to_end` —— 它**不要求流正常结束**：
/// 实测 `b"random ascii that is not xml or deflate"` 会被它"解出"一段非空输出并返回 `Ok`，
/// 于是 [`inflate`] 把一坨垃圾当成解压成功交出去，上层只会看到"0 条弹幕"（静默）。
///
/// 严格判定在实测两侧都成立：
/// - 真实 B站载荷（`cid=122300483`，145,077 字节）→ `eof=true / unused_data=0`，通过；
/// - 4 组垃圾字节（全零、`ff fe fd fc`、随机 ASCII、假 zlib 魔数）→ 全部 `Err`。
fn decode_strict(bytes: &[u8], zlib_header: bool) -> Option<Vec<u8>> {
    use flate2::{Decompress, FlushDecompress, Status};

    let mut d = Decompress::new(zlib_header);
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len().saturating_mul(4).max(64));
    loop {
        let (tin, tout) = (d.total_in() as usize, d.total_out() as usize);
        // `decompress_vec` 要求还有剩余容量；满了就扩容再继续
        if out.len() == out.capacity() {
            out.reserve(out.len().max(1024));
        }
        let status = d
            .decompress_vec(&bytes[tin..], &mut out, FlushDecompress::Finish)
            .ok()?;
        match status {
            Status::StreamEnd => break,
            // 输入输出都没有推进 → 输入不足或数据非法，判失败（顺便防死循环）
            _ if d.total_in() as usize == tin && d.total_out() as usize == tout => return None,
            _ => {}
        }
    }
    // 尾部还剩没消费的字节 → 不是一条干净的流
    if d.total_in() as usize != bytes.len() {
        return None;
    }
    (!out.is_empty()).then_some(out)
}

/// 从响应头里猜风味（**只用于日志**，判定一律以 [`inflate`] 实际结果为准）。
pub fn declared_encoding(headers: &[(String, String)]) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-encoding"))
        .map(|(_, v)| v.trim().to_ascii_lowercase())
}

/// 一行日志用的摘要（别把整个 XML 打进日志）。
pub fn head_preview(bytes: &[u8], n: usize) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(n)]).replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
    use flate2::Compression;
    use std::io::Write;

    /// 一段**与真实响应同形**的样本（标签密集的小 XML）。
    ///
    /// 真实抓取的规模（2026-09-19 实测）：弹幕 XML 的裸 deflate 载荷 145,077 字节 →
    /// 解开 361,725 字节；实测条数最多的那个视频解出来也是 361KB 量级。
    /// 这里使用小样本（测试不该依赖 100KB+ 的二进制 fixture），**压缩前的形状一致**
    /// 就足以验证解码器的选择。
    ///
    /// ⚠️ 下面**故意不含任何 B站字样**：本模块是通用解压工具，不该为了测试去认识调用方
    /// （`scripts/check_module_boundaries.py` 会盯着这条边界）。真实响应的出处记在这里就够了。
    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?><i><server>example.invalid</server><limit>1000</limit><state>0</state><d p="0.60100,4,25,15138834,1604742895,0,9e5adeaa,40685126698926083,10">月背</d><d p="1.5,1,25,16777215,1604742896,0,abc123,40685126698926084,0">前方高能</d></i>"#;

    fn raw_deflate(data: &[u8]) -> Vec<u8> {
        let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }
    fn zlib_wrap(data: &[u8]) -> Vec<u8> {
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }
    fn gzip_wrap(data: &[u8]) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    /// ⚠️ 这是本模块存在的全部理由：**B站发的是裸 deflate，而 zlib 解码器解不开它**。
    #[test]
    fn bare_deflate_is_the_payload_we_must_handle() {
        let payload = raw_deflate(XML.as_bytes());
        // 前提自证：裸 deflate **没有** zlib 魔数（否则这个测试就没意义了）
        assert!(payload.len() >= 2);
        assert!(
            !(payload[0] == 0x78 && matches!(payload[1], 0x01 | 0x5e | 0x9c | 0xda)),
            "前提失败：这段载荷带了 zlib 头，样本不具代表性"
        );

        let (out, flavor) = inflate(&payload).unwrap();
        assert_eq!(flavor, Flavor::RawDeflate);
        assert_eq!(out, XML.as_bytes());
        assert!(String::from_utf8(out).unwrap().contains("<d p="));
    }

    /// 兜底：带 zlib 头的（规范写法）也要能解 —— 有些 CDN 会重包装。
    #[test]
    fn zlib_and_gzip_are_also_accepted() {
        let (out, f) = inflate(&zlib_wrap(XML.as_bytes())).unwrap();
        assert_eq!(f, Flavor::Zlib);
        assert_eq!(out, XML.as_bytes());

        let (out, f) = inflate(&gzip_wrap(XML.as_bytes())).unwrap();
        assert_eq!(f, Flavor::Gzip);
        assert_eq!(out, XML.as_bytes());
    }

    /// 服务端哪天不压缩了也不能炸（否则"B站改了行为"会表现成"全量下载失败"）。
    #[test]
    fn plain_xml_passes_through() {
        let (out, f) = inflate(XML.as_bytes()).unwrap();
        assert_eq!(f, Flavor::Plain);
        assert_eq!(out, XML.as_bytes());
        // 空响应不是错误（调用方会据"没有 <d p=" 去报业务错误）
        assert_eq!(inflate(b"").unwrap(), (Vec::new(), Flavor::Plain));
    }

    /// 垃圾字节 → `Err`，**不 panic**（这是网络输入，必须当不可信处理）。
    ///
    /// ⚠️ 回归钉子：这条曾经红过。用 `flate2` 的 `read_to_end` 解码时，
    /// 第 3 组（随机 ASCII）会被"解出"非空输出并判成功 —— 因为 `read_to_end`
    /// **不要求流正常结束**。修法是 [`decode_strict`]（要 `StreamEnd` + 输入消费干净）。
    #[test]
    fn garbage_errors_without_panic() {
        for junk in [
            &b"\x00\x01\x02\x03\x04\x05"[..],
            &b"\xff\xfe\xfd\xfc"[..],
            &b"random ascii that is not xml or deflate"[..],
            &[0x78, 0x9c, 0xde, 0xad, 0xbe, 0xef][..], // 假 zlib 魔数
        ] {
            let r = inflate(junk);
            assert!(r.is_err(), "垃圾字节不该被当成成功解压: {junk:?}");
            // 错误信息要带足够的诊断线索
            let msg = format!("{:#}", r.unwrap_err());
            assert!(msg.contains("无法解压"), "{msg}");
        }
    }

    /// 日志辅助：`content-encoding` 大小写不敏感；预览不能把整个 XML 打出来。
    #[test]
    fn helpers_are_log_safe() {
        let h = vec![("Content-Encoding".to_string(), "DEFLATE ".to_string())];
        assert_eq!(declared_encoding(&h).as_deref(), Some("deflate"));
        assert!(declared_encoding(&[]).is_none());
        let long = vec![b'a'; 10_000];
        let prev = head_preview(&long, 32);
        assert_eq!(prev.len(), 32);
    }
}
