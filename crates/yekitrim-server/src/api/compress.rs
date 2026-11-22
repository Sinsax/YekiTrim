//! API 响应的 **JSON gzip**（2026-11-21，局域网提速）。
//!
//! # 为什么要有
//!
//! 全仓唯一的压缩点是 `media::serve_file_cond`（文件出口 + dhampir wasm）。**`/api/*` 的 JSON 一律裸发**，
//! 而精修页每次进出、每次任务结束都会重打一串：`/polish/project?preview=1`（含 `loudness_curve` 点表）、
//! `/subtitles`、`/jobs`、`/sfx-catalog`、`/emoji-catalog`…… 本机是几毫秒，**局域网/Wi-Fi 上就是可感的等待**。
//!
//! # 三条护栏（缺一条都会出事）
//!
//! ① **只认 `Content-Type: application/json`**（白名单，不是黑名单）：`/api/stream` 是
//!    `text/event-stream`，被缓冲会让**任务进度整条卡住**（前端 `useJobs` 靠它刷新）。
//! ② **绝不吞掉大响应**：边读边计数，超过 [`MAX_LEN`] 就把"已经读掉的块 + 剩下的流"重新拼成 body 原样放行 ——
//!    **一个字节都不许丢**（判据 `capped_collect_gives_back_every_byte`）。
//! ③ **1 KiB 以下不压**：小 JSON 压完更大，还白搭一次分配。
//!
//! ⚠️ 与 `serve_file_cached` 不冲突：`/api/media/*` 是 video/image（mime 不在 JSON 白名单里），
//! 或者已经带了 `Content-Encoding`（[`should_compress`] 直接否）。

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use futures_util::StreamExt as _;

/// 低于它不值得压（压完往往更大）。
pub(super) const MIN_LEN: usize = 1024;
/// 高于它**原样放行**：不把大响应整份读进内存。
pub(super) const MAX_LEN: usize = 4 * 1024 * 1024;

/// 这条响应**有没有资格**被压（与长度无关的那部分判据）。
///
/// `accepts_gzip` 由调用方按 `Accept-Encoding` 先算好（`media::accepts_gzip`）。
pub(super) fn may_compress(
    status: StatusCode,
    headers: &HeaderMap,
    accepts_gzip: bool,
    is_head: bool,
) -> bool {
    if !accepts_gzip || is_head {
        return false;
    }
    if status == StatusCode::NOT_MODIFIED || status == StatusCode::NO_CONTENT {
        return false;
    }
    // 已经是压缩过的 / 分段响应 ⇒ 绝不能二次加工（Range 的字节必须与原始文件一一对应）
    if headers.contains_key(header::CONTENT_ENCODING) || headers.contains_key(header::CONTENT_RANGE) {
        return false;
    }
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/json"))
        .unwrap_or(false)
}

/// 完整判据（`may_compress` + 长度窗口）—— **测试打这条**，中间件分两步用上面那个。
///
/// ⚠️ 生产路径**不调**它（长度要收完才知道，中间件分两步走），所以挂 `cfg(test)` 免得报 dead_code。
#[cfg(test)]
pub(super) fn should_compress(
    status: StatusCode,
    headers: &HeaderMap,
    accepts_gzip: bool,
    is_head: bool,
    len: usize,
) -> bool {
    may_compress(status, headers, accepts_gzip, is_head) && (MIN_LEN..=MAX_LEN).contains(&len)
}

/// 边读边计数地收 body：超 [`MAX_LEN`] / 读失败 ⇒ **把已读的块与剩下的流接回去**原样交出。
///
/// 返回 `Err(body)` 表示"别压了，这是拼回去的原响应"。
pub(super) async fn collect_capped(body: Body) -> Result<Vec<u8>, Body> {
    let mut stream = body.into_data_stream();
    let mut chunks: Vec<axum::body::Bytes> = Vec::new();
    let mut total = 0usize;
    loop {
        match stream.next().await {
            Some(Ok(b)) => {
                total += b.len();
                chunks.push(b);
                if total > MAX_LEN {
                    return Err(rejoin(chunks, stream));
                }
            }
            Some(Err(_)) => return Err(rejoin(chunks, stream)),
            None => return Ok(chunks.concat()),
        }
    }
}

fn rejoin(
    chunks: Vec<axum::body::Bytes>,
    rest: axum::body::BodyDataStream,
) -> Body {
    let head = futures_util::stream::iter(chunks.into_iter().map(Ok::<_, axum::Error>));
    Body::from_stream(head.chain(rest))
}

pub(super) async fn compress_json(req: Request, next: Next) -> Response {
    let accepts = req
        .headers()
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let is_head = req.method() == Method::HEAD;
    let res = next.run(req).await;

    // ---- 前置条件（长度要收完才知道，所以先过"资格"那一半）----
    let Some(ae) = accepts.as_deref() else { return res };
    if !may_compress(res.status(), res.headers(), crate::media::accepts_gzip(Some(ae)), is_head) {
        return res;
    }

    let (mut parts, body) = res.into_parts();
    let raw = match collect_capped(body).await {
        Ok(v) => v,
        Err(body) => return Response::from_parts(parts, body),
    };
    // 收完才知道长度：太小的不压（收的时候已经保证 ≤ MAX_LEN）
    if raw.len() < MIN_LEN {
        return Response::from_parts(parts, Body::from(raw));
    }
    let gz = crate::media::gzip_bytes(&raw);
    // 压不小就别压（很小的 JSON / 高熵内容）：体积更小才用压缩后的那份
    if gz.len() >= raw.len() {
        return Response::from_parts(parts, Body::from(raw));
    }
    parts
        .headers
        .insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    if let Ok(v) = HeaderValue::from_str(&gz.len().to_string()) {
        parts.headers.insert(header::CONTENT_LENGTH, v);
    }
    // 同一 URL 两种字节 ⇒ 少了 Vary 会让中间缓存串味（与静态壳同一条纪律）
    parts
        .headers
        .append(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    Response::from_parts(parts, Body::from(gz))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::test_app;

    fn hdr(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut m = HeaderMap::new();
        for (k, v) in pairs {
            m.insert(k.clone(), HeaderValue::from_str(v).unwrap());
        }
        m
    }

    fn json() -> HeaderMap {
        hdr(&[(header::CONTENT_TYPE, "application/json")])
    }

    /// **SSE 绝不能被压**（被缓冲 = 任务进度整条卡住）+ 其余几个否定分支。
    #[test]
    fn compression_whitelist_is_tight() {
        let ok = StatusCode::OK;
        assert!(should_compress(ok, &json(), true, false, 4096), "JSON 该压");
        assert!(should_compress(
            ok,
            &hdr(&[(header::CONTENT_TYPE, "application/json; charset=utf-8")]),
            true,
            false,
            4096
        ));
        let sse = hdr(&[(header::CONTENT_TYPE, "text/event-stream")]);
        assert!(!should_compress(ok, &sse, true, false, 4096), "SSE 被压就再也刷不动任务进度了");
        assert!(!should_compress(ok, &json(), false, false, 4096));
        assert!(!should_compress(ok, &json(), true, true, 4096), "HEAD 没有 body");
        let mut enc = json();
        enc.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        assert!(!should_compress(ok, &enc, true, false, 4096), "不许二次压缩");
        let mut range = json();
        range.insert(header::CONTENT_RANGE, HeaderValue::from_static("bytes 0-9/100"));
        assert!(!should_compress(ok, &range, true, false, 4096), "分段响应不许压");
        assert!(!should_compress(StatusCode::NOT_MODIFIED, &json(), true, false, 4096));
        assert!(!should_compress(StatusCode::NO_CONTENT, &json(), true, false, 4096));
        assert!(!should_compress(
            ok,
            &hdr(&[(header::CONTENT_TYPE, "video/mp4")]),
            true,
            false,
            4096
        ));
        assert!(!should_compress(ok, &json(), true, false, MIN_LEN - 1), "1 KiB 以下不压");
        assert!(!should_compress(ok, &json(), true, false, MAX_LEN + 1), "超过 4 MiB 原样放行");
    }

    /// **超限时必须一个字节都不丢**：交回的 body 要能原样读回全部内容。
    #[tokio::test]
    async fn capped_collect_gives_back_every_byte() {
        let total = MAX_LEN + 4096;
        let raw: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
        let body = Body::from(raw.clone());
        let rebuilt = match collect_capped(body).await {
            Ok(_) => panic!("超过 MAX_LEN 必须走原样放行那条"),
            Err(b) => b,
        };
        let back = axum::body::to_bytes(rebuilt, 64 * 1024 * 1024).await.expect("读回");
        assert_eq!(back.len(), raw.len(), "字节数丢了");
        assert_eq!(back.as_ref(), raw.as_slice(), "内容被改了 —— 这比不压严重得多");
        let small = vec![b'a'; 2048];
        let got = collect_capped(Body::from(small.clone())).await.expect("该收下");
        assert_eq!(got, small);
    }

    /// 端到端：小 JSON（`/health`）不该被压 —— 证明中间件挂上了，但没乱动普通响应。
    #[tokio::test]
    async fn small_json_passes_through_end_to_end() {
        let t = test_app().await;
        let req = t
            .req("GET", "/health")
            .header(header::ACCEPT_ENCODING, "gzip")
            .body(Body::empty())
            .expect("请求构造");
        let res = t.call(req).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            res.headers().get(header::CONTENT_ENCODING).is_none(),
            "1 KiB 以下的 JSON 不该压"
        );
    }
}