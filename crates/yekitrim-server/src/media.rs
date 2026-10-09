//! Media 访问（多用户服务 P2）：只服务**本人项目内**的文件。
//!
//! URL 形态：
//! - 新契约（推荐）`/api/media/<project_id>/<项目内相对路径>`，例如
//!   `/api/media/7/output/clips/clip-01/clip.mp4`；
//! - 过渡期兼容：`/api/media/<绝对路径>`（旧前端 `mediaUrl(dir, file)` 的写法）——
//!   只有在 canonicalize 后落在**调用者自己的某个项目根**之内时才放行，否则 403/404。
//!
//! 归属校验在 `api::media_get` 里完成（依赖当前登录用户）。

use std::io::Write as _;
use std::path::Path;

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

/// 拆分 `/api/media/<...>` 的路径部分。
///
/// 返回 `(Some(project_id), 项目内相对路径)` 或 `(None, 原始路径)`（过渡期绝对路径）。
pub fn split_project_media(path: &str) -> (Option<i64>, String) {
    match path.split_once('/') {
        Some((head, rest)) => match head.parse::<i64>() {
            Ok(id) if id > 0 => (Some(id), rest.to_string()),
            _ => (None, path.to_string()),
        },
        None => match path.parse::<i64>() {
            Ok(id) if id > 0 => (Some(id), String::new()),
            _ => (None, path.to_string()),
        },
    }
}

pub fn json_error(status: StatusCode, msg: &str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

/// 解析 `Range: bytes=...`（只支持单区间）。
///
/// - `None`：没有 / 无法解析（多区间、非 bytes 单位、数字非法）→ 调用方按 200 全量返回
/// - `Some(None)`：语法合法但不可满足（`start >= total`、`end < start`、`bytes=-0`）→ 416
/// - `Some(Some((start, end)))`：闭区间，已按 `total` 收敛
///
/// 旧实现在 handler 里直接 `end - start_raw + 1`：`bytes=500-100` 会整数下溢
/// （debug 下 panic；release 下算出天文数字的 Content-Length，客户端挂死）。
pub(crate) fn parse_byte_range(header: Option<&str>, total: u64) -> Option<Option<(u64, u64)>> {
    let spec = header?.trim().strip_prefix("bytes=")?.trim();
    if spec.contains(',') {
        return None;
    }
    let (start_s, end_s) = spec.split_once('-')?;
    let (start_s, end_s) = (start_s.trim(), end_s.trim());
    if total == 0 {
        return Some(None);
    }
    if start_s.is_empty() {
        // bytes=-N：最后 N 个字节
        let n: u64 = end_s.parse().ok()?;
        if n == 0 {
            return Some(None);
        }
        return Some(Some((total.saturating_sub(n), total - 1)));
    }
    let start: u64 = start_s.parse().ok()?;
    if start >= total {
        return Some(None);
    }
    let end = if end_s.is_empty() { total - 1 } else { end_s.parse::<u64>().ok()? };
    if end < start {
        return Some(None);
    }
    Some(Some((start, end.min(total - 1))))
}

/// 分块读一个文件（64 KiB/块），`remaining = Some(n)` 时最多读 n 字节。
fn stream_file(
    file: tokio::fs::File,
    remaining: Option<u64>,
) -> impl futures_util::Stream<Item = Result<axum::body::Bytes, std::io::Error>> + Send {
    futures_util::stream::unfold((file, remaining), |(mut f, mut rem)| async move {
        let mut buf = vec![0u8; 64 * 1024];
        let want = match rem {
            Some(0) => return None,
            Some(r) => r.min(buf.len() as u64) as usize,
            None => buf.len(),
        };
        let n = match f.read(&mut buf[..want]).await {
            Ok(n) => n,
            Err(e) => return Some((Err(e), (f, rem))),
        };
        if n == 0 {
            return None;
        }
        buf.truncate(n);
        rem = rem.map(|r| r - n as u64);
        Some((Ok(axum::body::Bytes::from(buf)), (f, rem)))
    })
}

/// 按 HTTP Range 语义输出一个本地文件：200 全量 / 206 分段 / 416 不可满足。
///
/// `/api/media/*` 与 `/api/polish/stickers/*/files/*` 共用这一份——两处都是
/// "只服务服务端已有文件"，不该各写一遍分块与 Range 处理。
pub(crate) async fn serve_file(
    path: &Path,
    mime: &str,
    range_header: Option<&str>,
) -> Response {
    serve_file_cond(path, mime, range_header, None, None, None).await
}

/// 与 [`serve_file`] 同一条路，外加**响应压缩**与 **ETag/304** 两个可选行为。
///
/// 为什么需要这一层（2026-10-02，用户问「Dhampir 加载有什么提速办法」）：
/// Dhampir 的预览 wasm 是 **7.66 MB** 且**完全没压缩**，而这条 `serve_file` 是它和视频
/// 共用的唯一出口 ⇒ 每次进精修页都要原样搬 7.66 MB。实测同一份字节：
///
/// | | 体积 | 占比 |
/// |---|---|---|
/// | 原始 | 7.66 MB | 100% |
/// | gzip -6 | 1.43 MB | 19% |
/// | brotli-5 | 1.12 MB | 15% |
///
/// ⚠️ **不能无脑压缩**，两个理由都是硬的：
/// ① **视频走 Range**（`Accept-Ranges: bytes` + 206）。压缩后的字节与「原始文件的
///    第 start..end 字节」**不再一一对应**，对 Range 响应做压缩会直接破坏播放
///    （花屏/解码失败）。所以`有 Range 头 ⇒ 一律不压缩`。
/// ② **mp4/png/jpg 本来就压过了**，再 gzip 一遍要 CPU、几乎不省字节。
///    所以只对**文本类**（白名单见 [`compressible_mime`]）压。
///
/// `etag`（`Some` 时启用）：值是调用方算好的强校验符（本仓用
/// `"<len>-<mtime_nanos>"`，见 [`file_etag`]）。命中 `If-None-Match` 就回 **304 空体**。
/// 这条**不改变**「wasm 重建后刷新即新」的既有意图（见 `polish_dhampir_wasm` 的说明）：
/// 文件一变 mtime 就变 ⇒ ETag 就变 ⇒ 自然失效。它省掉的是**没变时**那 7.66 MB。
pub(crate) async fn serve_file_cond(
    path: &Path,
    mime: &str,
    range_header: Option<&str>,
    etag: Option<&str>,
    if_none_match: Option<&str>,
    accept_encoding: Option<&str>,
) -> Response {
    // `tokio::fs`：元数据查询走 blocking 池 —— 文件可能在网络盘上，而播放会打出一串
    // Range 请求，同步 stat 每个都占着 worker（见 api.rs 顶部那条并发规矩）
    let total = match tokio::fs::metadata(path).await {
        Ok(m) => m.len(),
        Err(_) => return json_error(StatusCode::NOT_FOUND, "文件不存在"),
    };
    // **ETag 命中 → 304**。必须排在 Range 之前：`If-None-Match` 的语义是「我这份还有效吗」，
    // 回了 304 就不该再考虑分段（RFC 9110：304 不带 body、不带 Content-Length）。
    if let (Some(tag), Some(inm)) = (etag, if_none_match) {
        if etag_matches(inm, tag) {
            return Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(header::ETAG, tag)
                .header(header::CACHE_CONTROL, "no-cache")
                .body(axum::body::Body::empty())
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }
    match parse_byte_range(range_header, total) {
        // 语法合法但不可满足（start >= total / end < start / 后缀长度为 0）→ 416
        Some(None) => {
            return (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{total}"))],
            )
                .into_response();
        }
        Some(Some((start, end))) => {
            let len = end - start + 1;
            let mut file = match tokio::fs::File::open(path).await {
                Ok(f) => f,
                Err(_) => return json_error(StatusCode::NOT_FOUND, "文件不存在"),
            };
            if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            let body = axum::body::Body::from_stream(stream_file(file, Some(len)));
            return Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, mime)
                .header(header::CONTENT_LENGTH, len)
                .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}"))
                .header(header::ACCEPT_RANGES, "bytes")
                .body(body)
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
        // 没有 Range 或无法解析（多区间、非 bytes 单位……）→ 200 全量
        None => {}
    }

    // ---- 200 全量：这里才考虑压缩（Range 已在上面各自返回，走不到这）----
    let mut builder = Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCEPT_RANGES, "bytes");
    if let Some(tag) = etag {
        builder = builder
            .header(header::ETAG, tag)
            // `no-cache` 不是「不许缓存」，而是「用之前先回来校验」——配合上面的 ETag，
            // 没变就 304（几乎零字节），变了就拿新的。**别写成 max-age 长缓存**：
            // 那会让「Dhampir 重建 wasm 后刷新即新」这条既有意图失效。
            .header(header::CACHE_CONTROL, "no-cache");
    }
    // 压缩只在**文本类**且**客户端声明支持**时做；视频（不在这张白名单里）照旧走原始字节。
    if let Some(enc) = compressible_mime(mime).filter(|_| accepts_gzip(accept_encoding)) {
        match tokio::fs::read(path).await {
            Ok(raw) => {
                let gz = gzip_bytes(&raw);
                // 压不小就别压（小文件/已压过的内容）：体积更小才用压缩后的那份。
                if gz.len() < raw.len() {
                    return builder
                        .header(header::CONTENT_ENCODING, enc)
                        .header(header::CONTENT_LENGTH, gz.len())
                        // ⚠️ 必须带 Vary：同一 URL 对不同 Accept-Encoding 会给不同字节，
                        // 少了它中间的缓存/代理会把压缩版喂给不支持 gzip 的客户端。
                        .header(header::VARY, "Accept-Encoding")
                        .body(axum::body::Body::from(gz))
                        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
                }
            }
            Err(_) => return json_error(StatusCode::NOT_FOUND, "文件不存在"),
        }
    }
    let file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "文件不存在"),
    };
    let body = axum::body::Body::from_stream(stream_file(file, None));
    builder
        .header(header::CONTENT_LENGTH, total)
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// 哪些 MIME 值得 gzip。**只收文本类**——视频/图片/字体一律不进。
///
/// 这条白名单就是「别把视频压坏」的那道闸：只有走这条路的响应才可能被压缩，
/// 而视频的 `video/mp4`、图片的 `image/*`、字体的 `font/*` 都不在其中。
fn compressible_mime(mime: &str) -> Option<&'static str> {
    let m = mime.split(';').next().unwrap_or("").trim();
    match m {
        "application/wasm" => Some("gzip"),
        "application/javascript" | "text/javascript" => Some("gzip"),
        "text/css" => Some("gzip"),
        "text/html" | "text/plain" => Some("gzip"),
        "application/json" => Some("gzip"),
        // 注意 `image/svg+xml` **故意不收**：它虽含文本，但常被当作可缩放图形
        // 参与尺寸计算，压缩收益也小；收了要多想一层，不值。
        _ => None,
    }
}

/// 客户端是否接受 gzip。**认不出来就当不支持**（宁可发原始字节，也别发对方解不开的）。
fn accepts_gzip(accept_encoding: Option<&str>) -> bool {
    let Some(v) = accept_encoding else { return false };
    v.split(',').any(|part| {
        let mut it = part.split(';');
        let name = it.next().unwrap_or("").trim();
        if name != "gzip" && name != "*" {
            return false;
        }
        // `gzip;q=0` = 明确拒绝。
        !it.any(|p| p.trim().eq_ignore_ascii_case("q=0"))
    })
}

fn gzip_bytes(raw: &[u8]) -> Vec<u8> {
    // 6 是 zlib 默认档：实测 7.66 MB 的 wasm 在 -6 下就是 1.43 MB（19%），
    // 再往高档走到 -9 只多省几个百分点、CPU 翻倍——服务端每次都要压，不值。
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
    if enc.write_all(raw).is_err() {
        return Vec::new();
    }
    enc.finish().unwrap_or_default()
}

/// 由「长度 + mtime」算一个强 ETag。
///
/// ⚠️ **不用内容哈希**：wasm 每次 8 MB 全读一遍算 SHA 太贵，而我们要的只是
/// 「这份文件是不是上次那份」。`len + mtime_nanos` 对本仓的用法足够——
/// 重新构建必然改 mtime（同长度同 mtime 的不同内容在实践里不存在）。
pub(crate) fn file_etag(meta: &std::fs::Metadata) -> String {
    use std::time::UNIX_EPOCH;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("\"{}-{}\"", meta.len(), mtime)
}

/// `If-None-Match` 是否命中。支持 `*`、逗号分隔列表、以及弱校验前缀 `W/`。
fn etag_matches(if_none_match: &str, tag: &str) -> bool {
    let want = if_none_match.trim();
    if want == "*" {
        return true;
    }
    want.split(',').any(|c| {
        let c = c.trim();
        let c = c.strip_prefix("W/").unwrap_or(c);
        c == tag
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_byte_range_handles_edge_cases() {
        assert_eq!(parse_byte_range(Some("bytes=0-99"), 1000), Some(Some((0, 99))));
        assert_eq!(parse_byte_range(Some("bytes=500-"), 1000), Some(Some((500, 999))));
        assert_eq!(parse_byte_range(Some("bytes=-100"), 1000), Some(Some((900, 999))));
        // 尾部区间收敛到 total-1
        assert_eq!(parse_byte_range(Some("bytes=990-5000"), 1000), Some(Some((990, 999))));
        // 不可满足 → 416（旧实现的 `end - start` 会整数下溢）
        assert_eq!(parse_byte_range(Some("bytes=900-100"), 1000), Some(None));
        assert_eq!(parse_byte_range(Some("bytes=2000-"), 1000), Some(None));
        assert_eq!(parse_byte_range(Some("bytes=-0"), 1000), Some(None));
        // 无法解析 → 调用方按 200 全量返回
        assert_eq!(parse_byte_range(Some("bytes=abc"), 1000), None);
        assert_eq!(parse_byte_range(Some("items=0-1"), 1000), None);
        assert_eq!(parse_byte_range(Some("bytes=0-1,5-6"), 1000), None);
        assert_eq!(parse_byte_range(None, 1000), None);
    }

    #[test]
    fn compresses_only_text_like_mimes() {
        // Dhampir 的 wasm 与 engine.js 必须被压（否则每次进精修页搬 7.66 MB）
        assert_eq!(compressible_mime("application/wasm"), Some("gzip"));
        assert_eq!(compressible_mime("text/javascript"), Some("gzip"));
        assert_eq!(compressible_mime("application/javascript"), Some("gzip"));
        assert_eq!(compressible_mime("text/css"), Some("gzip"));
        // 带 charset 的写法也要认
        assert_eq!(compressible_mime("text/html; charset=utf-8"), Some("gzip"));
        // ⚠️ 视频/图片绝不能被压：对 Range 响应或已压过的字节做 gzip 会破坏播放
        assert_eq!(compressible_mime("video/mp4"), None);
        assert_eq!(compressible_mime("video/webm"), None);
        assert_eq!(compressible_mime("image/png"), None);
        assert_eq!(compressible_mime("image/jpeg"), None);
        assert_eq!(compressible_mime("font/woff2"), None);
        assert_eq!(compressible_mime("application/octet-stream"), None);
    }

    #[test]
    fn gzip_accept_encoding_parsing() {
        assert!(accepts_gzip(Some("gzip")));
        assert!(accepts_gzip(Some("gzip, deflate, br")));
        assert!(accepts_gzip(Some("deflate, gzip;q=0.8")));
        assert!(accepts_gzip(Some("*")));
        // `gzip;q=0` = 明确拒绝；`none` 不是 gzip
        assert!(!accepts_gzip(Some("gzip;q=0")));
        assert!(!accepts_gzip(Some("deflate")));
        assert!(!accepts_gzip(Some("br")));
        // 没有头 = 不知道 ⇒ 不压（宁发原始字节，也别发对方解不开的）
        assert!(!accepts_gzip(None));
        assert!(!accepts_gzip(Some("")));
    }

    #[test]
    fn gzip_actually_shrinks_wasm_like_payload() {
        // 用一段可压的字节模拟 wasm 的形态（真实 7.66 MB 实测压到 19%）
        let mut raw = Vec::new();
        for i in 0..200_000u32 {
            raw.extend_from_slice(&(i % 251).to_be_bytes());
        }
        let gz = gzip_bytes(&raw);
        assert!(!gz.is_empty(), "gzip_bytes 返回空 = 压不动，调用方会退化成不压");
        assert!(gz.len() < raw.len() / 2, "压缩没生效：{} -> {}", raw.len(), gz.len());
        // 解回来必须字节一致（压坏了比不压还糟）
        use std::io::Read as _;
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&gz[..])
            .read_to_end(&mut out)
            .expect("解压失败");
        assert_eq!(out, raw, "解压回来的字节与原文件不一致");
    }

    #[test]
    fn etag_matches_supports_star_list_and_weak_prefix() {
        let tag = "\"123-456\"";
        assert!(etag_matches("*", tag));
        assert!(etag_matches(tag, tag));
        assert!(etag_matches("\"other\", \"123-456\"", tag));
        // 弱校验前缀：RFC 允许 `W/"..."` 与强校验符比较
        assert!(etag_matches("W/\"123-456\"", tag));
        assert!(!etag_matches("\"999-1\"", tag));
        assert!(!etag_matches("", tag));
    }

    #[test]
    fn file_etag_changes_when_len_changes() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.bin");
        std::fs::write(&f, b"aaa").unwrap();
        let e1 = file_etag(&std::fs::metadata(&f).unwrap());
        std::fs::write(&f, b"aaaa").unwrap();
        let e2 = file_etag(&std::fs::metadata(&f).unwrap());
        assert_ne!(e1, e2, "长度变了 ETag 必须变（否则改造后刷新拿到旧 wasm）");
        assert!(e1.starts_with('\"') && e1.ends_with('\"'), "ETag 要带引号：{e1}");
    }

    #[test]
    fn splits_project_scoped_and_legacy_paths() {
        assert_eq!(
            split_project_media("7/output/clips/clip-01/clip.mp4"),
            (Some(7), "output/clips/clip-01/clip.mp4".to_string())
        );
        assert_eq!(split_project_media("12"), (Some(12), String::new()));
        // 旧前端：绝对路径（首段不是数字）
        let (id, rest) = split_project_media("F:/media/3d/output/clips/clip-01/clip.mp4");
        assert_eq!(id, None);
        assert_eq!(rest, "F:/media/3d/output/clips/clip-01/clip.mp4");
        // 0 / 负数不算项目 id，按旧路径处理
        assert_eq!(split_project_media("0/x").0, None);
        assert_eq!(split_project_media("-3/x").0, None);
    }
}
