//! Media 访问（多用户服务 P2）：只服务**本人项目内**的文件。
//!
//! URL 形态：
//! - 新契约（推荐）`/api/media/<project_id>/<项目内相对路径>`，例如
//!   `/api/media/7/output/clips/clip-01/clip.mp4`；
//! - 过渡期兼容：`/api/media/<绝对路径>`（旧前端 `mediaUrl(dir, file)` 的写法）——
//!   只有在 canonicalize 后落在**调用者自己的某个项目根**之内时才放行，否则 403/404。
//!
//! 归属校验在 `api::media_get` 里完成（依赖当前登录用户）。

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
/// `/api/media/*` 与 `/api/admin/stickers/*/files/*` 共用这一份——两处都是
/// "只服务服务端已有文件"，不该各写一遍分块与 Range 处理。
pub(crate) async fn serve_file(path: &Path, mime: &str, range_header: Option<&str>) -> Response {
    // `tokio::fs`：元数据查询走 blocking 池 —— 文件可能在网络盘上，而播放会打出一串
    // Range 请求，同步 stat 每个都占着 worker（见 api.rs 顶部那条并发规矩）
    let total = match tokio::fs::metadata(path).await {
        Ok(m) => m.len(),
        Err(_) => return json_error(StatusCode::NOT_FOUND, "文件不存在"),
    };
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

    let file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "文件不存在"),
    };
    let body = axum::body::Body::from_stream(stream_file(file, None));
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, total)
        .header(header::ACCEPT_RANGES, "bytes")
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
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
