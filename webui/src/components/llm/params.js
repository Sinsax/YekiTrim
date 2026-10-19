// components/llm/params.js — 请求参数面（`[llm.params]` / `[llm.providers.params]` / `overrides`）的
// **类型归一**：出 patch 前的唯一出口。
//
// 事故背景（2026-10-19，P0）：`llm.toml` 被写成了 `params = { json_mode = "", reasoning = "", … }`
// ⇒ 后端解析失败 ⇒ **整份 LLM 配置回退默认**（含 api_key）。空串/错类型一个字都不许进 patch。
//
// 纪律：**类型不对 / 空串 ⇒ 整条键不发**。
//   不是"发默认值"—— 发默认值会覆盖用户在源文件里手写的值；深合并语义下"不发" = 后端保留原值。
//   要**删掉**某个键是另一件事，由调用方显式发 `null`（patch 的 null = 删键），别混进这个函数。
import { LEVEL_VOCAB } from './caps.js'

/** 有限数字（数字，或"看起来是数字"的非空字符串）；其余一律 `undefined`（= 不发这个键）。 */
export function finiteNum(v) {
  if (typeof v === 'number') return Number.isFinite(v) ? v : undefined
  if (typeof v === 'string' && v.trim() !== '') {
    const n = Number(v.trim())
    return Number.isFinite(n) ? n : undefined
  }
  return undefined
}

/** temperature：`[0, 2]`，非有限数字 ⇒ 不发 */
export function normTemperature(v) {
  const n = finiteNum(v)
  if (n === undefined) return undefined
  return Math.min(2, Math.max(0, n))
}

/** max_tokens：`[256, 65536]` 的整数，非有限数字 ⇒ 不发 */
export function normMaxTokens(v) {
  const n = finiteNum(v)
  if (n === undefined) return undefined
  return Math.round(Math.min(65536, Math.max(256, n)))
}

/** reasoning：只允许合法档位 id（`off…max`）；空串 / 认不出的值 ⇒ 不发 */
export function normReasoning(v) {
  if (typeof v !== 'string') return undefined
  const s = v.trim()
  return LEVEL_VOCAB.includes(s) ? s : undefined
}

/**
 * 请求参数面归一：**只保留类型正确的键**。
 * @param {object|null|undefined} p 可能来自表单、旧的 localStorage 或坏的配置文件
 * @returns {{temperature?:number, max_tokens?:number, reasoning?:string, json_mode?:boolean}}
 */
export function normalizeParams(p) {
  const src = (p && typeof p === 'object') ? p : {}
  const out = {}
  const t = normTemperature(src.temperature)
  if (t !== undefined) out.temperature = t
  const m = normMaxTokens(src.max_tokens)
  if (m !== undefined) out.max_tokens = m
  const r = normReasoning(src.reasoning)
  if (r !== undefined) out.reasoning = r
  // json_mode：**只认布尔**（空串/null/0/'true' 都不认）—— 缺省就整条不发，别拿 false 去覆盖
  if (typeof src.json_mode === 'boolean') out.json_mode = src.json_mode
  return out
}
