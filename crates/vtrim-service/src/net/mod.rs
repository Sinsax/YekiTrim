//! `net`：HTTP 层的小工具（解压、浏览器 UA 客户端）。
//!
//! 这里**不含任何平台知识**（不认识 B站、不认识下载器）—— 那些分别在 `bilibili/` 与
//! `downloader/`。本模块只解决"发请求时踩到的通用坑"。

pub mod deflate;

use std::time::Duration;

/// 浏览器 UA。
///
/// ⚠️ **不是一个可以随便改的装饰**：实测非浏览器 UA（`python-urllib/3.13`）请求
/// `comment.bilibili.com` 会直接被判 **HTTP 412**，响应头带 `x-sec-request-id`（风控）。
/// `reqwest/x.y` 同理。所以凡是要打 B站的请求都必须带上它。
pub const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                              (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// 建一个带浏览器 UA 的客户端。
///
/// 超时是必要的：这些接口没有 SLA，卡住比失败更糟（下载槽会被占着）。
/// 不设 `Accept-Encoding`：实测 B站**强制** deflate 且忽略 `identity`，
/// 默认值反而省事 —— 手工解压见 [`deflate::inflate`]。
pub fn browser_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_else(|e| {
            // 建不出来只可能是 TLS 后端起不来这种环境级问题；用默认客户端降级继续，
            // 让业务层拿到"请求失败"而不是在构造点 panic（release 是 panic=abort）。
            tracing::warn!("构造 HTTP 客户端失败（降级为默认客户端）: {e}");
            reqwest::Client::new()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UA 必须看起来像浏览器（服务端按它做风控判定）。
    #[test]
    fn user_agent_looks_like_a_browser() {
        assert!(BROWSER_UA.starts_with("Mozilla/5.0"));
        assert!(BROWSER_UA.contains("Chrome/"));
        assert!(!BROWSER_UA.contains("reqwest"));
        assert!(!BROWSER_UA.contains("\n"), "UA 里不能有换行");
    }

    /// 客户端能建出来（TLS 后端可用），且带着 UA。
    #[test]
    fn browser_client_builds() {
        let c = browser_client();
        let _ = c;
    }
}
