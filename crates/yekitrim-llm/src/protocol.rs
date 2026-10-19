//! **协议注册表**：一份 provider 的「请求怎么发」—— 路径 + 鉴权头。
//!
//! # 与 `dialect` 的分工（别混）
//!
//! * **protocol**（本文件）：请求**发到哪条路径**（`/chat/completions` / `/models`）、
//!   **怎么鉴权**（`Authorization: Bearer` / `x-api-key`）；
//! * **dialect**（`dialect.rs`）：**推理参数怎么拼**（`reasoning_effort` / `thinking.type` …）。
//!
//! 一家 API 标准 = **一条 protocol + 一条 dialect**。两层分开的理由：同一族协议
//! （OpenAI 兼容）能配不同方言，而同一套方言也能跑在不同族协议上。
//!
//! # 加一家协议 = 加一条表项
//!
//! 本轮只有 `openai-chat`。以后接 Anthropic / Gemini **原生**时是
//! "加一条表项 + 一份请求体映射"，**不是**改配置结构 —— 配置里的 `api` 就是这个 id。
//!
//! ⚠️ **认不出的 `api` 要报错，不许静默回退成 openai-chat**：那会让人以为在用别的协议
//! （表现是"配了但没生效"，最难查）。校验在 `LlmConfig::provider_problems`（core 侧），
//! 写入接口拿它回 400。

/// 鉴权头的写法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStyle {
    /// `Authorization: Bearer <key>`（OpenAI 那一族）
    Bearer,
    /// `x-api-key: <key>`（Anthropic 那一族）
    XApiKey,
}

impl AuthStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::XApiKey => "x-api-key",
        }
    }
}

/// 一条协议：请求路径 + 鉴权方式。
#[derive(Debug, Clone, Copy)]
pub struct Protocol {
    /// 配置里 `api` 的取值（**稳定 ASCII id**）。
    pub id: &'static str,
    /// 给界面看的中文名（协议下拉用它）。
    pub label: &'static str,
    /// 对话补全的路径（拼在 base_url 后面）。
    pub chat_path: &'static str,
    /// 模型清单的路径（探针用）。
    pub models_path: &'static str,
    /// 鉴权头写法。
    pub auth: AuthStyle,
}

/// 全部协议。**加一家 = 加一行**。
pub const PROTOCOLS: &[Protocol] = &[Protocol {
    id: "openai-chat",
    label: "OpenAI Chat Completions",
    chat_path: "/chat/completions",
    models_path: "/models",
    auth: AuthStyle::Bearer,
}];

/// 按 id 找一条（大小写不敏感、去首尾空白）。**认不出 ⇒ `None`**（由调用方报错，别回退）。
pub fn protocol(id: &str) -> Option<&'static Protocol> {
    let want = id.trim();
    PROTOCOLS.iter().find(|p| p.id.eq_ignore_ascii_case(want))
}

/// 配置里 `api` 缺省时用的那条（**只在"用户根本没写"时用**；写错了要报错，见模块头）。
pub fn default_protocol() -> &'static Protocol {
    &PROTOCOLS[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_chat_is_registered_and_labelled() {
        let p = protocol("openai-chat").expect("本轮唯一的一条");
        assert_eq!(p.chat_path, "/chat/completions");
        assert_eq!(p.models_path, "/models");
        assert_eq!(p.auth, AuthStyle::Bearer);
        assert!(!p.label.is_empty(), "界面下拉要用 label");
        assert_eq!(default_protocol().id, "openai-chat");
    }

    #[test]
    fn unknown_protocol_is_none_not_a_fallback() {
        assert!(protocol("gemini-native").is_none(), "认不出 ⇒ None（调用方报错，别静默回退）");
        assert!(protocol("").is_none());
        // 大小写/空白不敏感
        assert_eq!(protocol("  OpenAI-Chat ").map(|p| p.id), Some("openai-chat"));
    }
}
