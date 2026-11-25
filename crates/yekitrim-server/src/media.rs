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
// 这里曾有一个不带缓存头的 `serve_file(path, mime, range)` —— 四个文件出口都在用它，
// 于是每次进精修页/打开贴纸选择器都要把字体与贴纸**整份重下**（局域网里就是「有延迟」）。
// 已由 `serve_file_from_headers` 取代（ETag/304 + 文本类 gzip），**别再把它加回来**。

/// **带条件缓存的那条路**：ETag/304 + 文本类 gzip（`serve_file` 的推荐替代）。
///
/// # 为什么要有它（2026-11-21，用户报「局域网访问精修有延迟」）
///
/// `/api/media/*` 早在 2026-10-02 就补了 ETag/304（见 `api/project_data.rs::media_get`），
/// 但**另外四个走 `serve_file` 的文件出口一个都没补**：
///
/// | 出口 | 典型体积 | 以前 |
/// |---|---|---|
/// | `/api/polish/font`（预览注册的字体字节） | CJK 单支 **5–20 MB** | 每次进精修页/刷新整份重下 |
/// | `/api/polish/sticker-file`、`/polish/stickers/:pack/files/:name` | 单张 GIF 最大几 MB | 贴纸选择器每次打开整份重下 |
/// | `/api/polish/asset`（素材库视频/字体预览） | 同上 | 每次预览整份重下 |
/// | `/api/polish/video-thumb` | 几百 KB | 同上 |
///
/// 「没有 `Cache-Control` / `ETag` / `Last-Modified`」的响应**没有条件请求可发** ——
/// 浏览器只能每次都把字节重新拉一遍。本机 loopback 上这只是几十毫秒，
/// **在局域网（尤其 Wi-Fi）上就是几秒到几十秒**，用户感知到的正是「访问精修有延迟」。
///
/// # 三条硬约束（与 `serve_file_cond` 同一套，别各写一遍）
///
/// ① **有 `Range` 头就不参与 304**：算 ETag 要 `metadata`，而媒体元素每次 seek 都带 Range、
///    打出一串请求；更重要的是对一个分段请求回 304 会让播放器当成「整份还有效」。
///    所以这里 `range.is_some() ⇒ etag = None`（`serve_file_cond` 先判 ETag 再判 Range）。
/// ② **有 `Range` 头一律不压**（压缩后字节与「原始文件第 start..end 字节」错位 ⇒ 花屏）。
/// ③ `Cache-Control` 用 **`no-cache`**（= 用之前回来校验），不是 max-age：文件改了 mtime 就变、
///    ETag 自然失效；写成 long-cache 会让「重切了片段但预览还是旧的」。
pub(crate) async fn serve_file_cached(
    path: &Path,
    mime: &str,
    range_header: Option<&str>,
    if_none_match: Option<&str>,
    accept_encoding: Option<&str>,
) -> Response {
    let etag = if range_header.is_some() {
        None
    } else {
        tokio::fs::metadata(path).await.ok().map(|m| file_etag(&m))
    };
    serve_file_cond(path, mime, range_header, etag.as_deref(), if_none_match, accept_encoding).await
}

/// 直接从请求头取 `Range` / `If-None-Match` / `Accept-Encoding` 并走 `serve_file_cached`。
///
/// **文件出口一律用它**：四个 handler 各自从 `HeaderMap` 抠三个头会漏（实测就漏了 ETag），
/// 而漏掉的症状是「每次进页面重下几 MB」，**不报错、日志里也看不出来**。
pub(crate) async fn serve_file_from_headers(
    path: &Path,
    mime: &str,
    headers: &axum::http::HeaderMap,
) -> Response {
    let get = |name: axum::http::header::HeaderName| headers.get(name).and_then(|v| v.to_str().ok());
    serve_file_cached(
        path,
        mime,
        get(axum::http::header::RANGE),
        get(axum::http::header::IF_NONE_MATCH),
        get(axum::http::header::ACCEPT_ENCODING),
    )
    .await
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
                // ⚠️ gzip 是同步 CPU 活（几 MB 字体压一遍要几十 ms），别留在 async worker 上 ——
                // 任务跑起来时并发请求一多，worker 被这类 CPU 活占满，整个服务都慢（2026-11-25 排查）。
                let raw_len = raw.len();
                // ⚠️ 只回传压缩后的字节：raw 已经 move 进去了，再回传一份是白拷贝（也留下过
                // 一个「接了却没人用」的绑定）。join 失败 ⇒ 空 Vec，由下面那条 !gz.is_empty()
                // 挡回「原样发送」分支 —— 以前写的是 gz.len() < raw_len，失败时会发空压缩体
                // （0 < raw_len 恒真），那是一个只在 panic 时才现形的 200 空响应。
                let gz = tokio::task::spawn_blocking(move || gzip_bytes(&raw))
                    .await
                    .unwrap_or_default();
                // 压不小就别压（小文件/已压过的内容）：体积更小才用压缩后的那份。
                if !gz.is_empty() && gz.len() < raw_len {
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
pub(crate) fn compressible_mime(mime: &str) -> Option<&'static str> {
    let m = mime.split(';').next().unwrap_or("").trim();
    match m {
        "application/wasm" => Some("gzip"),
        "application/javascript" | "text/javascript" => Some("gzip"),
        "text/css" => Some("gzip"),
        "text/html" | "text/plain" => Some("gzip"),
        "application/json" => Some("gzip"),
        // **未压缩的字体**（CJK 的 .ttf/.otf/.ttc 单支 5–20 MB）：字体本身没有容器压缩，
        // gzip 实测能砍掉 40–55%。冷启动那一次省下的是几 MB 的局域网搬运，
        // 之后靠 ETag 走 304（见 `serve_file_cached`）就不再压了。
        // ⚠️ **`font/woff` / `font/woff2` 不许收**：它们本来就是压缩格式，再压一遍只是烧 CPU。
        "font/ttf" | "font/otf" | "font/ttc" | "application/x-font-ttf" => Some("gzip"),
        // 注意 `image/svg+xml` **故意不收**：它虽含文本，但常被当作可缩放图形
        // 参与尺寸计算，压缩收益也小；收了要多想一层，不值。
        _ => None,
    }
}

/// 客户端是否接受 gzip。**认不出来就当不支持**（宁可发原始字节，也别发对方解不开的）。
pub(crate) fn accepts_gzip(accept_encoding: Option<&str>) -> bool {
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

pub(crate) fn gzip_bytes(raw: &[u8]) -> Vec<u8> {
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
pub(crate) fn etag_matches(if_none_match: &str, tag: &str) -> bool {
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
        // **未压缩的字体要压**（2026-11-21，局域网首屏）：CJK 单支 5–20 MB，gzip 砍 40–55%。
        assert_eq!(compressible_mime("font/ttf"), Some("gzip"));
        assert_eq!(compressible_mime("font/otf"), Some("gzip"));
        assert_eq!(compressible_mime("font/ttc"), Some("gzip"));
        // ⚠️ woff/woff2 本来就是压缩格式，**不许**收进来（压一遍只烧 CPU、几乎不省字节）
        assert_eq!(compressible_mime("font/woff"), None);
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

    /// 读整个响应体（小文件够用）。
    async fn body_of(res: Response) -> Vec<u8> {
        axum::body::to_bytes(res.into_body(), 8 * 1024 * 1024)
            .await
            .expect("body")
            .to_vec()
    }

    /// 造一份**可压**的文本（够大、够重复，gzip 才压得比原文小）。
    fn big_json() -> Vec<u8> {
        let mut s = String::from("{\"items\":[");
        for i in 0..2000 {
            if i > 0 {
                s.push(',');
            }
            s.push_str("{\"start\":1.5,\"end\":3.5,\"text\":\"你好世界\"}");
        }
        s.push_str("]}");
        s.into_bytes()
    }

    /// **局域网那次「第二次打开还是慢」的头号原因**（2026-11-21）：
    /// 没有 `ETag`/`Cache-Control` 的响应**没有条件请求可发** ⇒ 字体/贴纸/素材每次整份重下。
    /// 这条钉住三件事：首次 200 带校验符、带着校验符回来 304 空体、声明 gzip 时文本类要压。
    #[tokio::test]
    async fn cached_file_serving_sends_etag_then_304_and_gzips_text() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("d.json");
        let payload = big_json();
        std::fs::write(&f, &payload).unwrap();
        let tag = file_etag(&std::fs::metadata(&f).unwrap());

        // ① 首次全量：200 + ETag + `no-cache`（= 用之前回来校验，不是"不许缓存"）
        let res = serve_file_cached(&f, "application/json", None, None, None).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers().get(header::ETAG).and_then(|v| v.to_str().ok()), Some(tag.as_str()));
        assert_eq!(res.headers().get(header::CACHE_CONTROL).and_then(|v| v.to_str().ok()), Some("no-cache"));
        assert_eq!(body_of(res).await, payload, "没声明 gzip ⇒ 原样字节");

        // ② 带着同一个 ETag 再来 ⇒ 304 空体（省掉的就是那 5–20 MB）
        let res = serve_file_cached(&f, "application/json", None, Some(&tag), None).await;
        assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
        assert!(body_of(res).await.is_empty(), "304 不许带 body");

        // ③ 声明 gzip ⇒ 文本类要压，并且必须带 Vary（同一 URL 两种字节）
        let res = serve_file_cached(&f, "application/json", None, None, Some("gzip")).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers().get(header::CONTENT_ENCODING).and_then(|v| v.to_str().ok()), Some("gzip"));
        assert_eq!(res.headers().get(header::VARY).and_then(|v| v.to_str().ok()), Some("Accept-Encoding"));
        let gz = body_of(res).await;
        assert!(gz.len() * 4 < payload.len(), "压缩没生效：{} -> {}", payload.len(), gz.len());
        use std::io::Read as _;
        let mut back = Vec::new();
        flate2::read::GzDecoder::new(&gz[..]).read_to_end(&mut back).expect("解压失败");
        assert_eq!(back, payload, "压坏了比不压更糟");
    }

    /// **带 `Range` 时既不能 304、也不能压**（播放会花屏/卡死）——
    /// 与 `serve_file_cond` 的既有铁律同一条，只是现在多了一个入口，必须一起钉住。
    #[tokio::test]
    async fn range_requests_never_get_304_or_gzip() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("c.mp4");
        let payload: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&f, &payload).unwrap();
        let tag = file_etag(&std::fs::metadata(&f).unwrap());

        let res = serve_file_cached(&f, "video/mp4", Some("bytes=0-9"), Some(&tag), Some("gzip")).await;
        assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT, "带 Range 必须 206，不能 304");
        assert!(res.headers().get(header::CONTENT_ENCODING).is_none(), "Range 响应绝不能压");
        assert!(
            res.headers().get(header::ETAG).is_none(),
            "带 Range 时不算 ETag（媒体元素每次 seek 都打一串 Range，算了也没人用来 304）"
        );
        assert_eq!(body_of(res).await, payload[..10].to_vec(), "206 的字节必须是原始文件那一段");
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
