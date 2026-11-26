// components/llm/extra.js — 「额外请求参数」的**唯一校验/序列化实现**。
//
// 2026-11-26：`extra_params` 从"全局一份"改成"**每个模型一份**"（[llm.models].extra_params），
// 全局那份退化成兜底。两处都要解析同样的 JSON，规则抄两份必然漂，所以收在这里：
//
//   · 必须是 **JSON 对象**（数组 / 标量都不行）；
//   · **已知结构化键一律拒绝**（temperature / max_tokens / … / response_format）——
//     否则"手写 thinking"会和推理档位互相打架（D4 的老口径，别放宽）；
//   · **不许出现 null**：TOML 没有 null，写进 llm.toml 会让整份配置解析失败
//     （2026-10-19 P0：一个坏类型就把整份配置打回默认值，含 api_key）。

/** 这些键必须走结构化设置（温度 / 上限 / 推理 / JSON），不能塞进逃逸口。 */
export const KNOWN_EXTRA_KEYS = [
  'temperature', 'max_tokens', 'max_completion_tokens', 'top_p', 'reasoning',
  'reasoning_effort', 'thinking', 'thinking_budget', 'response_format'
]

/** JSON 里有没有 null（含嵌套）：TOML 表达不了，直接拒。 */
function hasNull(v) {
  if (v === null) return true
  if (Array.isArray(v)) return v.some(hasNull)
  if (v && typeof v === 'object') return Object.values(v).some(hasNull)
  return false
}

/**
 * 文本 → `{ value, error }`。
 * 空文本 = 清空（value=null, 无错）；非法时 **value=null 且带 error** —— 调用方据此
 * 既不要写半份值，也要把错误摆出来（保存通道会因此拦住）。
 */
export function parseExtraObject(text) {
  const t = String(text == null ? '' : text).trim()
  if (!t) return { value: null, error: '' }
  let obj
  try { obj = JSON.parse(t) } catch (e) { return { value: null, error: '不是合法 JSON' } }
  if (!obj || typeof obj !== 'object' || Array.isArray(obj)) {
    return { value: null, error: '必须是 JSON 对象' }
  }
  const bad = Object.keys(obj).filter((k) => KNOWN_EXTRA_KEYS.includes(k))
  if (bad.length) return { value: null, error: '已知键请用结构化设置：' + bad.join(' / ') }
  if (hasNull(obj)) return { value: null, error: 'TOML 没有 null，删掉值为 null 的键' }
  return { value: obj, error: '' }
}

/** 对象 → 编辑框里的文本（空对象 / null ⇒ 空串）。 */
export function stringifyExtra(obj) {
  if (!obj || typeof obj !== 'object' || !Object.keys(obj).length) return ''
  try { return JSON.stringify(obj, null, 2) } catch (e) { return '' }
}
