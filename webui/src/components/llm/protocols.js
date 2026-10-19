// components/llm/protocols.js — **API 协议**清单（provider 的 `api` 字段：请求怎么发）。
//
// 来源：后端 `GET /api/config/llm/protocols`（Rust `Protocol` 注册表）。
// 那条接口还在并行实现 ⇒ 先用下面这份**内置只读清单**兜底，界面上标注「内置」。
// ⚠️ 协议（请求怎么发）与方言（参数怎么拼）是**两层**：别把 dialect id 塞进这里。
export const BUILTIN_PROTOCOLS = [
  { id: 'openai-chat', label: 'OpenAI Chat Completions' }
]

/** 协议 id → 人话（认不出就原样返回）。 */
export function protocolLabel(list, id) {
  const hit = (Array.isArray(list) ? list : []).find((p) => p && p.id === id)
  return hit ? String(hit.label || hit.id) : String(id || '')
}
