// api/llm.js — LLM 配置域的接口：模型发现（内置目录 / 真探）、模型能力、保存与连通测试。
//
// 服务端契约（P3）：
//   GET  /api/llm/models?base_url=<url>            → { source: "catalog"|"probe", models: [{id,name}], note? }
//   GET  /api/config/llm/catalog?base_url=&model=  → { source, caps: {...} }（只读内置表：不联网、不落盘）
//   PUT  /api/config/llm { patch | raw }           → patch 是**合并**语义：JSON null = 删键；空 api_key = 不修改
//
// ⚠️ 失败一律**抛出**（这里不做兜值）：设置页要把原因写出来 —— 「获取模型」静默失败 =
//    用户以为按钮是坏的（探失败服务端回 502 + {ok:false,error}，api/client.js 会带着那句 error 抛）。
import { api } from './client'

/** 这次模型清单**怎么来的**：按钮与状态行都要照实说（catalog 不联网 / probe 真发一次 /models）。 */
export const MODEL_SOURCE_ZH = {
  catalog: '内置目录（不联网）',
  probe: '联网探测（真发一次 /models）'
}

/** 内置能力表里的模型清单。返回 { source, models: [{id,name}], note }。 */
export async function fetchModels(baseUrl) {
  const q = 'base_url=' + encodeURIComponent(String(baseUrl || '').trim())
  const r = await api('/api/llm/models?' + q, { label: '获取 LLM 模型列表' })
  const models = Array.isArray(r && r.models) ? r.models.filter((m) => m && m.id) : []
  return {
    source: r && r.source === 'probe' ? 'probe' : 'catalog',
    models,
    // 探到 200 但一个模型都没报 = **不是失败**（服务端会给一句 note），照实显示 + 允许手打
    note: (r && r.note) || ''
  }
}

/** 某个 (base_url, model) 的能力。P3 起这条不会 404；真没有时抛错，调用方退化（不静默当"没有能力"）。 */
export async function fetchCatalog(baseUrl, model) {
  const q = 'base_url=' + encodeURIComponent(String(baseUrl || '').trim())
    + '&model=' + encodeURIComponent(String(model || '').trim())
  return api('/api/config/llm/catalog?' + q, { label: '读取模型能力' })
}

/** 保存 LLM 配置（patch 语义）。api_key 在**没填**时由调用方整个省略 —— 合并语义下等于不修改。 */
export async function saveLlm(patch) {
  return api('/api/config/llm', { method: 'PUT', body: JSON.stringify({ patch }), label: '保存 LLM 配置' })
}

/** 保存 llm.toml 原文（「高级 · 源文件」那条路）。 */
export async function saveLlmRaw(raw) {
  return api('/api/config/llm', { method: 'PUT', body: JSON.stringify({ raw }), label: '保存 llm.toml 源文件' })
}

/** 连通测试：服务端拿当前 [llm] 配置发一次最小请求。 */
export async function testConnection() {
  return api('/api/llm/test', { method: 'POST', label: 'LLM 连通测试' })
}
