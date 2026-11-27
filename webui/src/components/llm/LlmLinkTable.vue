<script setup>
// LlmLinkTable — 「链路配置」子页第一屏：行 = 链路，列 = **用哪支 / 温度 / 输出上限 / 推理**。
//
// 用户口径（2026-11-27）：
//  · **不再有"点开才编辑"和灰字**：数字直接是**滑块 + 数字框**，一眼可改；
//  · **没有测试列、没有状态列**：测试只在模型配置那边（connection 卡里）；
//  · **推理没有「跟随」这一档**：要么关、要么某个档，缺省 **关**（off）；
//  · 讲解文字一律不写（"没有提到就不要加"）。
//
// 两条纪律（沿用）：
//  ① 数字覆盖的语义是"没写 = 跟随 [llm.defaults]" ⇒ 值仍可能是 null。滑块**只有真的拖了/才写值**；
//     写过之后行尾出现一个 &#215;（**仅此时出现**），点它 = 恢复跟随（发 null = 删键）。
//  ② 「用哪支」仍是**一个下拉**（optgroup = 接口，值 = 接口|模型），一次写 provider + model；
//     配对由界面保证，后端 problems() 不再需要查"这支模型属不属于这个接口"。
//
// ⚠️ rows 是**父层的 reactive 对象**（本组件原地改属性），父层的 dirty 判定与 buildLlmPatch 直接读它。
import { computed } from 'vue'
import { LEVEL_ZH, LEVEL_VOCAB } from './caps.js'

const props = defineProps({
  /** [{ key, label, hint }] —— 行由后端 override_links 给（correct 只隐藏不删） */
  links: { type: Array, default: () => [] },
  /** 覆盖表单：link → { provider, model, temperature, max_tokens, reasoning }（null/'' = 跟随） */
  rows: { type: Object, required: true },
  /** 生效快照：link → /api/config/llm/effective 的返回（null = 还没取到 / 取失败） */
  effective: { type: Object, default: () => ({}) },
  /** 可选接口 */
  providers: { type: Array, default: () => [] },
  /** 可选模型（来自模型清单，带 provider 用于按接口分组） */
  modelOptions: { type: Array, default: () => [] },
  /** 当前接口 id（= [llm] active） */
  activeId: { type: String, default: '' },
  /** 推理档位候选（当前接口那支模型的；某行模型不一样时用 levelsByModel 覆盖） */
  levels: { type: Array, default: () => [] },
  /** model id → 档位数组 */
  levelsByModel: { type: Object, default: () => ({}) }
})

const rowOf = (key) => props.rows[key] || (props.rows[key] = {})
const snapOf = (key) => props.effective[key] || null
/** 这一行**实际会用**的模型（行覆盖 > 生效快照）*/
function effModel(key) {
  const r = String(rowOf(key).model || '').trim()
  if (r) return r
  const s = snapOf(key)
  return s && s.model ? String(s.model) : ''
}
/** 生效快照里的某个参数（没取到 → null）*/
function effParam(key, col) {
  const s = snapOf(key)
  const p = s && s.params ? s.params[col] : null
  return p === undefined ? null : p
}
/** 这一格是不是"跟随"（没写覆盖） */
function inherited(key, col) {
  const v = rowOf(key)[col]
  return v === null || v === undefined || v === ''
}

/* ── 用哪支 ───────────────────────────────────────────────────────── */
/** 这份接口下可选的模型：**只有清单里的**（要选探到的模型就先在模型配置里加进清单） */
function modelsOf(pid) {
  const id = String(pid || '')
  const all = Array.isArray(props.modelOptions) ? props.modelOptions : []
  return all.filter((m) => String(m.provider || '') === id)
    .map((m) => ({ id: m.id, label: m.label || '' }))
}
/** 当前选定值 = 接口|模型；没覆盖就是空串（跟随默认） */
function selKey(key) {
  const r = rowOf(key)
  const pid = String(r.provider || '').trim()
  const mid = String(r.model || '').trim()
  return pid && mid ? pid + '|' + mid : ''
}
/** 选一支 = 同时写 provider 与 model */
function pickModel(key, val) {
  const v = String(val || '')
  const row = rowOf(key)
  if (!v) { row.provider = ''; row.model = ''; return }
  const cut = v.indexOf('|')
  row.provider = v.slice(0, cut)
  row.model = v.slice(cut + 1)
}

/* ── 数字：滑块 + 数字框 ───────────────────────────────────────────── */
/** 显示值 = 覆盖 > 生效快照 > 兜底 */
function numOf(key, col, fallback) {
  if (!inherited(key, col)) return Number(rowOf(key)[col])
  const e = effParam(key, col)
  return e === null || e === undefined ? fallback : Number(e)
}
/** 写值：拖滑块/改数字都走这里（清空 = 回到跟随） */
function setNum(key, col, raw) {
  const s = String(raw == null ? '' : raw).trim()
  if (s === '') { rowOf(key)[col] = null; return }
  const n = Number(s)
  rowOf(key)[col] = Number.isFinite(n) ? n : null
}
function clearNum(key, col) { rowOf(key)[col] = null }

/* ── 推理档位 ─────────────────────────────────────────────────────── */
/** 这一行的候选档位（按本行生效模型的能力；取不到就是外层给的那份） */
function levelsFor(key) {
  const id = String(rowOf(key).model || '').trim() || effModel(key)
  const lv = id ? (props.levelsByModel || {})[id] : null
  const out = Array.isArray(lv) && lv.length ? lv : (props.levels || [])
  return Array.isArray(out) ? out : []
}
/** 下拉候选：**关**永远在最前（缺省），然后是本模型认识的档；当前值认不出也要列出来 */
function levelChoices(key) {
  const set = new Set(['off'])
  for (const v of levelsFor(key)) if (v) set.add(String(v))
  const cur = rowOf(key).reasoning
  if (cur) set.add(String(cur))
  const order = LEVEL_VOCAB.filter((v) => set.has(v))
  const extra = [...set].filter((v) => !LEVEL_VOCAB.includes(v))
  return order.concat(extra)
}
/** 显示值：写了就按写的；没写 = **关**（用户口径：默认关，不给「跟随」） */
function levelOf(key) {
  const v = rowOf(key).reasoning
  return v ? String(v) : 'off'
}
function setLevel(key, v) { rowOf(key).reasoning = String(v || 'off') }

const levelZh = (v) => LEVEL_ZH[v] || v
const hasRow = computed(() => Array.isArray(props.links) && props.links.length > 0)
</script>

<template>
  <div class="lt">
    <div class="lt-scroll">
      <table class="lt-tbl">
        <thead>
          <tr>
            <th class="lt-c-link">链路</th>
            <th class="lt-c-model">用哪支</th>
            <th class="lt-c-num">温度</th>
            <th class="lt-c-num">输出上限</th>
            <th class="lt-c-reason">推理</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="l in links" :key="l.key" :data-link="l.key">
            <td class="lt-c-link">
              <b>{{ l.label }}</b>
              <span v-if="l.hint" class="lt-sub">{{ l.hint }}</span>
            </td>

            <td class="lt-c-model">
              <select class="lt-sel" :value="selKey(l.key)" :aria-label="'模型（' + l.label + '）'"
                      @change="pickModel(l.key, $event.target.value)">
                <option value="">未选</option>
                <optgroup v-for="p in providers" :key="p.id" :label="p.name || p.id">
                  <option v-for="m in modelsOf(p.id)" :key="p.id + '/' + m.id" :value="p.id + '|' + m.id">
                    {{ m.label || m.id }}
                  </option>
                </optgroup>
              </select>
            </td>

            <td class="lt-c-num">
              <div class="lt-sl">
                <input type="range" min="0" max="2" step="0.05" aria-label="温度"
                       :value="numOf(l.key, 'temperature', 0.7)"
                       @input="setNum(l.key, 'temperature', $event.target.value)">
                <input type="number" min="0" max="2" step="0.05" class="lt-num" aria-label="温度（数值）"
                       :value="numOf(l.key, 'temperature', 0.7)"
                       @input="setNum(l.key, 'temperature', $event.target.value)">
                <button v-if="!inherited(l.key, 'temperature')" class="lt-x" type="button"
                        title="恢复跟随默认" @click="clearNum(l.key, 'temperature')">&#215;</button>
              </div>
            </td>

            <td class="lt-c-num">
              <div class="lt-sl">
                <input type="range" min="256" max="65536" step="256" aria-label="输出上限"
                       :value="numOf(l.key, 'max_tokens', 16384)"
                       @input="setNum(l.key, 'max_tokens', $event.target.value)">
                <input type="number" min="256" max="65536" step="256" class="lt-num" aria-label="输出上限（数值）"
                       :value="numOf(l.key, 'max_tokens', 16384)"
                       @input="setNum(l.key, 'max_tokens', $event.target.value)">
                <button v-if="!inherited(l.key, 'max_tokens')" class="lt-x" type="button"
                        title="恢复跟随默认" @click="clearNum(l.key, 'max_tokens')">&#215;</button>
              </div>
            </td>

            <td class="lt-c-reason">
              <select class="lt-sel2" :value="levelOf(l.key)" :aria-label="'推理（' + l.label + '）'"
                      @change="setLevel(l.key, $event.target.value)">
                <option v-for="v in levelChoices(l.key)" :key="v" :value="v">{{ levelZh(v) }}</option>
              </select>
            </td>
          </tr>
          <tr v-if="!hasRow">
            <td colspan="5" class="lt-empty">后端没给链路清单（override_links 为空）</td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>

<style scoped>
.lt { min-width:0; }
.lt-scroll { overflow-x:auto; }
.lt-tbl { width:100%; border-collapse:collapse; font-size:12px; min-width:760px; }
.lt-tbl th { text-align:left; font-weight:500; font-size:11px; color:var(--muted);
  padding:6px 8px; border-bottom:1px solid var(--line); white-space:nowrap; }
.lt-tbl td { padding:6px 8px; border-bottom:1px solid var(--line-dim, var(--line)); vertical-align:top; }
.lt-c-link { width:150px; }
.lt-c-model { width:240px; }
.lt-c-num { width:230px; }
.lt-c-reason { width:120px; }
.lt-sub { display:block; font-size:11px; color:var(--muted); }
.lt-sel, .lt-sel2 { width:100%; font-size:12px; padding:3px 6px; }
.lt-sl { display:flex; align-items:center; gap:6px; }
.lt-sl input[type=range] { flex:1 1 auto; min-width:60px; }
.lt-num { width:68px; font-size:12px; padding:2px 4px; text-align:right; }
.lt-x { border:0; background:none; color:var(--muted); cursor:pointer; font-size:13px; line-height:1; padding:0 2px; }
.lt-x:hover { color:var(--danger, #e5484d); }
.lt-empty { color:var(--muted); font-size:12px; padding:10px 8px; }
</style>
