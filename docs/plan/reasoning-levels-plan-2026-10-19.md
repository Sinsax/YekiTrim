# 推理档位：接入更多标准（2026-10-19）

> 起因：用户要求「研究 `http://192.168.0.107:7863/v1` 的推理档位是怎么获取的，给出整体方案，
> 尽可能接入更多标准的推理挡位」。

## 一、实测：那家网关怎么暴露档位（证据）

`GET /v1/models`（Bearer 后 200，21 个模型）里**每支模型自带能力字段** —— 它**自描述**，不用猜：

| 字段 | 出现 | 观测到的取值 |
|---|---|---|
| `reasoning_supported_efforts` | 19/21 | `["medium"]`×6 · `["low","high","max"]`×3 · `["low","medium","high"]`×2 · `["high"]`×2 · `["low","high"]`×2 · `["low","high","xhigh"]`×2 · `["high","xhigh"]`×1 |
| `reasoning_default_effort` | 13/21 | `high`×11 · `medium`×1 · `max`×1 |
| `reasoning_effort` | 8/21 | `medium`×6 · `high`×2（**疑似当前生效值**） |
| `can_disable_thinking` | 7/21 | `true`（缺 = 关不掉） |
| `only_reasoning` | 18/21 | `true`（**只会推理**，没有"不思考"这一档） |
| `supports_reasoning` / `supports_images` / `supports_tool_call` | 21 | 全 `true` |
| `reasoning_summary` | 21 | 全 `auto` |
| `context_length` / `max_output_tokens` / `max_allowed_size` | 21 | 数字 |
| `credits` / `vendor` / `tags` / `is_default` | 19/21/8/1 | `"x0.11"` / `f`·`e`·`j` / ? / ? |

**档位词表是 `low` / `medium` / `high` / `xhigh` / `max`** —— 注意有 **`max`**，而我们的
`ReasoningLevel` 只有 `off|minimal|low|medium|high|xhigh`（`max` 当年从 pi-ai 数据里被裁掉了）。

### 请求侧：它**不校验**（这条决定了整个方案）

对 `cn:deepseek-v4.1-flash` 各发一次 1-token 请求（`max_tokens: 1`，看回包里有没有 `reasoning_content`）：

| 发什么 | HTTP | 有 `reasoning_content`？ |
|---|---|---|
| 基线（不发任何推理参数） | 200 | **有** |
| `reasoning_effort: "max"`（它在清单里） | 200 | **有** |
| `reasoning_effort: "minimal"`（**不在**清单里） | 200 | **没有** |
| `reasoning: {"effort":"high"}`（OpenRouter 名） | 200 | 有 |
| `thinking: {"type":"disabled"}`（DeepSeek 名） | 200 | **没有** |
| `enable_thinking: false`（Qwen 名） | 200 | 有（**它不认这个键**） |

⇒ 三条硬结论：
1. **不认识的值不会报错**（`minimal` 也 200，只是行为变了）⇒ **不能靠错误信息学档位**；
2. ⇒ **唯一可靠的档位来源就是 `/v1/models` 的 `reasoning_supported_efforts`**；
3. 请求侧它同时认 `reasoning_effort`（OpenAI 名）与 `thinking.type`（DeepSeek 名）两个方言，
   `enable_thinking` 不认 —— 正好说明**"一家网关可能认多种方言"**，方言与网关不是一对一。

## 二、整体方案（四层）

### 第 1 层：档位词表补齐（`ReasoningLevel`）
`off | minimal | low | medium | high | xhigh | max`，顺序即强弱。`max` 必须有 —— 这家网关在用。
`Minimal`/`Max` 只是**词**，发不发由能力面决定（既有纪律不变：表里没有的档一个都不发）。

### 第 2 层：能力**读取器注册表**（与方言注册表同构，这是"接入更多标准"的关键）
```rust
pub struct CapsReader {
    pub id: &'static str,                 // "workbuddy" / "openrouter" / "litellm" / "ollama" / "generic"
    pub label: &'static str,
    pub read: fn(&Value) -> Option<PartialCaps>,   // 单位置条目 → 能力片段（认不出就 None）
}
pub const READERS: &[CapsReader] = &[ … ];
```
`/v1/models` 的每个条目**依次过所有 reader**，取并集（后者只补前者没说的字段）。要接的读者：

| reader | 认的字段 | 备注 |
|---|---|---|
| **`workbuddy`（本次实测）** | `reasoning_supported_efforts` / `can_disable_thinking` / `only_reasoning` / `reasoning_default_effort` / `supports_reasoning` / `supports_images` / `context_length` / `max_output_tokens` / `supports_tool_call` | 信息最全，**已跑通** |
| `openrouter` | `architecture.input_modalities` / `architecture.modality` / `supported_parameters` / `top_provider.context_length` | 已在做（`supported_parameters` 推断） |
| `litellm` | `/model/info` 的 `model_info.supports_reasoning` / `max_input_tokens` / `mode` | **走另一个路径**，需要探测时先试它 |
| `ollama` | `/api/tags` + `/api/show` 的 `capabilities: ["completion","tools","thinking","vision"]` / `model_info.*.context_length` | 路径不同（无 `/v1/models` 语义） |
| `generic` | `input_modalities` / `modalities` / `capabilities.{vision,reasoning}` / `context_window` / `max_output_tokens` | 兜底 |

⚠️ **一条纪律**：reader 返回 `PartialCaps`（**只说自己知道的**），合并时**不许用 None 覆盖已有值** ——
否则"认出一半"会把表里其余全抹掉（前端 `mergeCaps` 已经踩过这条，后端要同口径）。

### 第 3 层：方言注册表补条目（`dialect.rs`，只加表项）
现有 5 条（`openai_effort` / `thinking_type` / `qwen_enable_thinking` / `openrouter_effort` / `none`），
按"常见标准"再补这些（每条 = 一个 apply 函数 + 一行表项）：

| 方言 id | 请求形态 | 谁在用 |
|---|---|---|
| `openai_effort`（已有） | `reasoning_effort: "high"` | OpenAI / xAI / Groq / **本网关** |
| `thinking_type`（已有） | `thinking: {"type": "enabled"/"disabled"}` | DeepSeek / Anthropic 兼容层 / **本网关也认** |
| `thinking_budget` | `thinking: {"type":"enabled","budget_tokens": N}` | Anthropic 原生 / Cohere |
| `openrouter_effort`（已有） | `reasoning: {"effort": "high"}` | OpenRouter |
| `reasoning_max_tokens` | `reasoning: {"max_tokens": N}` | OpenRouter 另一档 |
| `qwen_enable_thinking`（已有） | `enable_thinking: bool` (+`thinking_budget`) | Qwen / DashScope |
| `chat_template_kwargs` | `chat_template_kwargs: {"enable_thinking": bool}` | vLLM / SGLang（模板开关） |
| `thinking_token_budget` | `thinking_token_budget: N` | SGLang |
| `ollama_think` | `think: true \| "high"` | Ollama |
| `gemini_thinking` | `generationConfig.thinkingConfig.{thinkingBudget,thinkingLevel}` | Gemini（**另一族 API**，接它要先有协议适配） |

### 第 4 层：无自描述时的兜底（**要用户显式点**）
有些服务器（裸 vLLM / 自建网关）**什么都不说**。此时：
1. 先用表（pi-ai 数据）；表也没有 ⇒ **不发推理参数**（既有纪律）；
2. 给一颗**「探测档位」**按钮（**不是自动**）：对候选档各发一次 1-token 请求，
   看 `reasoning_content` 有没有 / `completion_tokens` 差多少，得出"哪些档真的有效"；
   ⚠️ 会**花 token**、结论是启发式，必须在界面上写明；探测结果只用于当前会话，**不自动落盘**。

## 三、优先级与落点

| 优先级 | 做什么 | 落点 |
|---|---|---|
| **P0** | 补 `max` 档；`workbuddy` reader（本次实测那家）；档位选项 = 能力面给的清单（含 `can_disable_thinking` ⇒ 有没有"关闭"，`only_reasoning` ⇒ 关闭标成不可用并写原因） | `params.rs` / `catalog.rs` / `discovery.rs` / 前端 |
| **P1** | reader 泛化：`openrouter` / `generic` 补全；探不到能力时**显示"未知"而不是假装支持** | `discovery.rs` |
| **P2** | 方言补条目：`thinking_budget` / `chat_template_kwargs` / `thinking_token_budget` / `ollama_think`；`thinking_budget` 需要"N 从哪来"（按档位映射一个预算表） | `dialect.rs` |
| **P3** | LiteLLM `/model/info` 与 Ollama `/api/show` 两条**路径级**探测（不是字段级） | `discovery.rs` |
| **P4** | 「探测档位」按钮（显式、花钱、结论启发式） | 前端 + `discovery` |
| **P5** | Gemini / Anthropic **原生协议**（不是 OpenAI 兼容层）—— 那是另一个量级的工作（请求体、鉴权头、流式都不同） | 新模块 |

## 四、验收口径（每期都要有）
* 能力合并**不许 None 覆盖已有值**（一条单测：只回 `supports_images` 时，表里的档位/上下文照留）；
* 档位选项**必须由能力面派生**，不许写死（前端已有 `levels` 2 档 ⇒ 2 项的变异自证，继续用它）；
* 认不出的方言一律降级"不发"（已有 `wire::tests`，别放宽）；
* 请求体回归：v1→v2 那 12 条金色请求体**逐字节不变**（已存在，别弄红）。

## 五、本次实测的原始证据（可复跑）
* 探测脚本：`.workbuddy/probe-7863.py`（字段清单）/ `.workbuddy/probe-7863-wire.py`（六种请求形态对比）
* 端点：`http://192.168.0.107:7863/v1`，`owned_by=workbuddy`，21 个模型，
  配置里的模型是 `cn:deepseek-v4.1-flash`（`reasoning_supported_efforts: ["low","high","max"]`，
  `reasoning_default_effort: high`，`can_disable_thinking: true`，`only_reasoning: true`，`supports_images: true`）
* 另有一个 `http://192.168.0.107:8317/v1` 在全局 `llm.toml` 里（未探，疑似 LiteLLM 系 —— P3 时一并确认）
