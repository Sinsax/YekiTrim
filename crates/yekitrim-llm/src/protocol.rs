//! **协议注册表**：一份 provider 的「请求怎么发」—— 路径 + 鉴权头。
//!
//! # 与 `dialect` 的分工（别混）
//!
//! * **protocol**（本文件）：请求**发到哪条路径**（`/chat/completions` / `/v1/messages`）、
//!   **怎么鉴权**（`Authorization: Bearer` / `x-api-key` / `?key=`）；
//! * **dialect**（`dialect.rs`）：**推理参数怎么拼**（`reasoning_effort` / `thinking.type` …）。
//!
//! 一家 API 标准 = **一条 protocol + 一条 dialect + 一份请求体/回包映射**（映射在 [`crate::adapters`]）。
//! 两层分开的理由：同一族协议（OpenAI 兼容）能配不同方言，而同一套方言也能跑在不同族协议上。
//!
//! # 加一家协议 = 加一条表项
//!
//! 请求体/回包形状不同的（Anthropic / Gemini / Ollama / Responses）还要在 [`crate::adapters`]
//! 加一个分支 —— 本文件只描述「发到哪、怎么鉴权」。
//!
//! ⚠️ **认不出的 `api` 要报错，不许静默回退成 openai-chat**：那会让人以为在用别的协议
//! （表现是"配了但没生效"，最难查）。校验在 `LlmConfig::provider_problems`（core 侧），
//! 写入接口拿它回 400。

/// 鉴权头的写法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStyle {
    /// `Authorization: Bearer <key>`（OpenAI 那一族）
    Bearer,
    /// `?key=<key>` 查询参数（Gemini 原生）
    QueryKey,
}

impl AuthStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::QueryKey => "query-key",
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
    /// 对话补全的路径（拼在 base_url 后面）。`{model}` 占位符会被模型 id 替换（Gemini 要）。
    pub chat_path: &'static str,
    /// 模型清单的路径（探针用）。
    pub models_path: &'static str,
    /// 鉴权头写法。
    pub auth: AuthStyle,
    /// 每个请求都要带的固定头（现在三家都是空；将来某家要版本头时用）。
    pub extra_headers: &'static [(&'static str, &'static str)],
}

/// 全部协议。**加一家 = 加一行**（请求体形状不同的再在 adapters 加分支）。
pub const PROTOCOLS: &[Protocol] = &[
    Protocol {
        id: "openai-chat",
        label: "OpenAI Chat Completions",
        chat_path: "/chat/completions",
        models_path: "/models",
        auth: AuthStyle::Bearer,
        extra_headers: &[],
    },
    Protocol {
        id: "gemini-native",
        label: "Gemini（Google 原生）",
        chat_path: "/models/{model}:generateContent",
        models_path: "/models",
        auth: AuthStyle::QueryKey,
        extra_headers: &[],
    },
    Protocol {
        id: "openai-responses",
        label: "OpenAI Responses",
        chat_path: "/responses",
        models_path: "/models",
        auth: AuthStyle::Bearer,
        extra_headers: &[],
    },
];

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
    fn all_three_are_registered_and_labelled() {
        for want in ["openai-chat", "gemini-native", "openai-responses"] {
            let p = protocol(want).unwrap_or_else(|| panic!("{want} 应该在注册表里"));
            assert!(!p.label.is_empty(), "{want} 要有界面名");
            assert!(!p.chat_path.is_empty() && !p.models_path.is_empty());
        }
        assert_eq!(default_protocol().id, "openai-chat");
    }

    #[test]
    fn unknown_protocol_is_none_not_a_fallback() {
        // ⚠️ ollama / anthropic 走**兼容层**（openai-chat + 对应 base_url），不单独注册 ——
        //    判据钉住"它们不是协议"：别为了方便把兼容层当协议加回来。
        assert!(protocol("ollama-native").is_none());
        assert!(protocol("anthropic-messages").is_none());
        assert!(protocol("deepseek-native").is_none(), "认不出 ⇒ None（调用方报错，别回退）");
        assert!(protocol("").is_none());
        // 大小写/空白不敏感
        assert_eq!(protocol("  Gemini-Native ").map(|p| p.id), Some("gemini-native"));
    }

    #[test]
    fn auth_follows_the_family() {
        assert_eq!(protocol("openai-chat").unwrap().auth, AuthStyle::Bearer);
        assert_eq!(protocol("gemini-native").unwrap().auth, AuthStyle::QueryKey);
        assert_eq!(protocol("openai-responses").unwrap().auth, AuthStyle::Bearer);
    }
}
