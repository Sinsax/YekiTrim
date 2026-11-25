// api/llm.js — LLM 配置域的接口：模型发现（内置目录 / 真探）、模型能力、保存与连通测试。
//
// 服务端契约（v4 / L2-L3）：
//   GET  /api/llm/models?base_url=&api_key=&provider=   → { source: "catalog"|"probe", models: [{id,name,caps?}], note? }
//   GET  /api/config/llm/catalog?base_url=&model=       → { source, caps: {...} }（只读内置表：不联网、不落盘）
//   GET  /api/config/llm/effective?link=<link>          → 该链路**合并后的生效值**（链路表格的灰字）
//   PUT  /api/config/llm { patch | raw }                → patch 是**合并**语义：JSON null = 删键；空 api_key = 不修改
//   POST /api/llm/test  { provider?, base_url?, api_key?, model? } → { ok, latency_ms, model, provider }
//
// ⚠️ 失败一律**抛出**（这里不做兜值）：设置页要把原因写出来 —— 「获取模型」静默失败 =
//    用户以为按钮是坏的（探失败服务端回 502 + {ok:false,error}，api/client.js 会带着那句 error 抛）。
import { api } from './client'

/** 这次模型清单**怎么来的**：按钮与状态行都要照实说（catalog 不联网 / probe 真发一次 /models）。 */
export const MODEL_SOURCE_ZH = {
  catalog: '内置目录（不联网）',
  probe: '联网探测（真发一次 /models）'
}

/**
 * 内置能力表里的模型清单。返回 { source, models: [{id,name,caps?}], note }。
 * @param {string} [providerId] 多配置模式下的 provider id —— **必须带**：没带时后端会用
 *   `active` 那份的已存 key；带了就按这份配置走。
 */
export async function fetchModels(baseUrl, apiKey, providerId) {
  let q = 'base_url=' + encodeURIComponent(String(baseUrl || '').trim())
  // 草稿 key：刚填还没保存时探针要用它，否则会拿旧 key 换一个 401
  const k = String(apiKey || '').trim()
  if (k) q += '&api_key=' + encodeURIComponent(k)
  const pid = String(providerId || '').trim()
  if (pid) q += '&provider=' + encodeURIComponent(pid)
  const r = await api('/api/llm/models?' + q, { label: '获取 LLM 模型列表' })
  const models = Array.isArray(r && r.models) ? r.models.filter((m) => m && m.id) : []
  return {
    source: r && r.source === 'probe' ? 'probe' : 'catalog',
    models,
    // 探到 200 但一个模型都没报 = **不是失败**（服务端会给一句 note），照实显示 + 允许手打
    note: (r && r.note) || ''
  }
}

/** 某个 (base_url, model) 的能力。真没有时抛错，调用方退化（不静默当"没有能力"）。 */
export async function fetchCatalog(baseUrl, model) {
  const q = 'base_url=' + encodeURIComponent(String(baseUrl || '').trim())
    + '&model=' + encodeURIComponent(String(model || '').trim())
  return api('/api/config/llm/catalog?' + q, { label: '读取模型能力' })
}

/** 协议清单（`GET /api/config/llm/protocols` → `{ protocols: [{id,label}] }`）。
 * 「API 协议」就是 provider 的 `protocol` 字段（请求怎么发）；**失败抛出** —— 调用方退回内置只读清单并标注。 */
export async function fetchProtocols() {
  const r = await api('/api/config/llm/protocols', { label: '读取 API 协议清单' })
  return Array.isArray(r && r.protocols) ? r.protocols.filter((p) => p && p.id) : []
}

/**
 * 一条链路的**生效值**（L2）：链路表格里"没覆盖的格子"显示的灰字。
 *
 * ⚠️ 规则只在后端算（`LlmV4::link_snapshot`），前端**不许自己算一遍** ——
 *    L1 之前「界面显示的与真发的不一致」就是同一套规则抄了两份。
 * @param {string} link detect / polish / correct / title / utility
 */
export async function fetchEffective(link) {
  return api('/api/config/llm/effective?link=' + encodeURIComponent(String(link || '').trim()),
    { label: '读取链路生效值' })
}

/** 保存 LLM 配置（patch 语义）。api_key 在**没填**时由调用方整个省略 —— 合并语义下等于不修改。 */
export async function saveLlm(patch) {
  return api('/api/config/llm', { method: 'PUT', body: JSON.stringify({ patch }), label: '保存 LLM 配置' })
}

/**
 * 连通测试：服务端发一次最小请求。
 *
 * **草稿也能测**（L3）：body 里的 base_url / api_key / model 非空就优先用它们，空的回落已存那份，
 * 所以"填好 key 先试一下、通了再保存"这条路是通的（不用先落盘）。
 * @param {{provider?: string, base_url?: string, api_key?: string, model?: string}} [draft]
 */
export async function testConnection(draft = {}) {
  const body = {}
  for (const k of ['provider', 'base_url', 'api_key', 'model']) {
    const v = String(draft[k] || '').trim()
    if (v) body[k] = v
  }
  return api('/api/llm/test', { method: 'POST', body: JSON.stringify(body), label: 'LLM 连通测试' })
}
