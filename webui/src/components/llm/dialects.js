// components/llm/dialects.js — **方言清单**：推理参数怎么发 / 输出上限写哪个字段名。
//
// 两个来源（同一个形状 `{ id, label, max_output_field }`）：
//   ① 后端 `GET /api/config/llm/dialects`（Rust `yekitrim_llm::dialect::DIALECTS` 注册表）；
//   ② 那条接口还没做 / 回的清单是空的 ⇒ 退回下面这份**内置只读清单**（手抄自注册表，只用来把 id 翻成
//      人话、并给下拉列候选）。界面上会标注「内置」—— 不静默假装是服务端给的。
//
// ⚠️ **值是 id**（写进 `[llm.models."<id>"].thinking_wire`），label 只给人看。
// ⚠️ 同一个 id 可以有多行（`max_output_field` 不同，见 Rust 注册表）：下拉按 **id** 去重，
//    输出上限那栏按 **字段名** 去重。
export const CUSTOM = '__custom__'

/**
 * 内置只读清单 —— **逐行**对应 `crates/yekitrim-llm/src/dialect.rs::DIALECTS`（2026-10-19 的 19 行）。
 * ⚠️ 它是**兜底**：后端 `GET /api/config/llm/dialects` 一旦接上就以服务端那份为准；两边漂了以 Rust 为准，
 *    这里只负责"接口还没做时，下拉里也有东西可选、且标注是内置的"。
 */
export const BUILTIN_DIALECTS = [
  { id: 'openai_effort', label: '推理档位（reasoning_effort）', max_output_field: 'max_completion_tokens' },
  { id: 'openai_effort', label: '推理档位（reasoning_effort）', max_output_field: 'max_tokens' },
  { id: 'thinking_type', label: 'thinking.type（开/关）', max_output_field: 'max_completion_tokens' },
  { id: 'thinking_type', label: 'thinking.type（开/关）', max_output_field: 'max_tokens' },
  { id: 'qwen_enable_thinking', label: 'enable_thinking（通义/百炼）', max_output_field: 'max_completion_tokens' },
  { id: 'openrouter_effort', label: 'reasoning.effort（OpenRouter）', max_output_field: 'max_completion_tokens' },
  { id: 'none', label: '不发推理参数', max_output_field: 'max_tokens' },
  { id: 'none', label: '不发推理参数', max_output_field: 'max_completion_tokens' },
  // P2 补的标准（同样"两个字段名各一行"，与 Rust 表一一对应）
  { id: 'thinking_budget', label: 'thinking.budget_tokens（Anthropic/Cohere）', max_output_field: 'max_tokens' },
  { id: 'thinking_budget', label: 'thinking.budget_tokens（Anthropic/Cohere）', max_output_field: 'max_completion_tokens' },
  { id: 'reasoning_max_tokens', label: 'reasoning.max_tokens（OpenRouter）', max_output_field: 'max_tokens' },
  { id: 'reasoning_max_tokens', label: 'reasoning.max_tokens（OpenRouter）', max_output_field: 'max_completion_tokens' },
  { id: 'chat_template_kwargs', label: 'chat_template_kwargs（vLLM/SGLang）', max_output_field: 'max_tokens' },
  { id: 'chat_template_kwargs', label: 'chat_template_kwargs（vLLM/SGLang）', max_output_field: 'max_completion_tokens' },
  { id: 'thinking_token_budget', label: 'thinking_token_budget（SGLang）', max_output_field: 'max_tokens' },
  { id: 'thinking_token_budget', label: 'thinking_token_budget（SGLang）', max_output_field: 'max_completion_tokens' },
  { id: 'ollama_think', label: 'think（Ollama）', max_output_field: 'max_tokens' },
  { id: 'ollama_think', label: 'think（Ollama）', max_output_field: 'max_completion_tokens' },
  { id: 'gemini_thinking', label: 'Gemini thinkingConfig（注册不发送）', max_output_field: 'max_tokens' }
]

/** 下拉候选：按 id 去重（同 id 多行只留第一条的 label）。 */
export function dialectChoices(list) {
  const out = []
  for (const d of (Array.isArray(list) ? list : [])) {
    if (!d || !d.id) continue
    const id = String(d.id)
    if (out.some((x) => x.id === id)) continue
    out.push({ id, label: String(d.label || id) })
  }
  return out
}

/** 输出上限字段的候选：两条标准在前，再补清单里出现过的别的字段名（去重、稳定顺序）。 */
export function maxOutputChoices(list) {
  const out = []
  const push = (v) => { const s = String(v || '').trim(); if (s && !out.includes(s)) out.push(s) }
  push('max_tokens')
  push('max_completion_tokens')
  for (const d of (Array.isArray(list) ? list : [])) push(d && d.max_output_field)
  return out
}

/** 某个 id 的人话（认不出就原样返回 id —— 别猜）。 */
export function dialectLabel(list, id) {
  const hit = dialectChoices(list).find((d) => d.id === id)
  return hit ? hit.label : String(id || '')
}
