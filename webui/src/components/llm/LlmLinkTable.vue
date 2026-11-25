<script setup>
// LlmLinkTable — L3 五 IA 的 **③ 按链路覆盖**：行 = 链路，列 = 接口 / 模型 / 温度 / 输出上限 / 推理 / 状态。
//
// 用户口径（配置重构 §3.3-3）：「未覆盖的格子显示灰字实值（来自 /api/config/llm/effective），
// 点开才可编辑 —— 一眼能看出哪条链路跟谁、谁被改过」。
//
// 三条纪律：
//  ① **灰字只能来自后端快照**（`fetchEffective` → `LlmV4::link_snapshot`）。前端自己按 defaults 算一遍
//     就是同一套规则抄第二份 —— L1 之前"界面显示的与真发的不一致"正是这么来的。
//  ② **"没有值"用 null 表达**（保存时发 null = 删键 = 跟随），不是空串：空串会把 `max_tokens = ""`
//     写进 llm.toml，下次解析失败把整份配置打回默认值（P0 事故）。
//  ③ **点开不写值**：点一下只是长出控件，控件里显示的起步值就是灰字那份；真改了才落进 rows（= 覆盖）。
//     想撤回就点控件旁的「跟随」—— 那才是"删这个键"。
//
// ⚠️ rows 是**父层的 reactive 对象**（本组件原地改属性，与 LlmParamFields 同一条绑法），
//    所以父层的 dirty 判定与 buildLlmPatch 直接读它，不用一串 update: 事件。
const props = defineProps({
  /** [{ key, label, hint }] —— 行由后端 `override_links` 给（correct 只隐藏不删） */
  links: { type: Array, default: () => [] },
  /** 覆盖表单：link → { provider, model, temperature, max_tokens, reasoning }（null/'' = 跟随） */
  rows: { type: Object, required: true },
  /** 生效快照：link → /api/config/llm/effective 的返回（null = 还没取到 / 取失败） */
  effective: { type: Object, default: () => ({}) },
  /** 可选接口（列 = 接口） */
  providers: { type: Array, default: () => [] },
  /** 可选模型（列 = 模型；候选来自 ④ 的清单） */
  modelOptions: { type: Array, default: () => [] },
  /** 推理档位候选（来自当前模型的能力面；空 = 只画「跟随」） */
  levels: { type: Array, default: () => [] }
})

import { reactive } from 'vue'
import { LEVEL_ZH } from './caps.js'

/** 哪个格子在编辑态：`${link}:${col}` → true */
const editing = reactive({})
const cellId = (key, col) => key + ':' + col
const isEditing = (key, col) => !!editing[cellId(key, col)]
function open(key, col) { editing[cellId(key, col)] = true }
function close(key, col) { editing[cellId(key, col)] = false }

const rowOf = (key) => props.rows[key] || (props.rows[key] = {})
const snapOf = (key) => props.effective[key] || null

/** 灰字：接口 = 生效那一段的名字（没取到快照就空） */
function effProvider(key) {
  const s = snapOf(key)
  if (!s || !s.provider) return ''
  return s.provider.name || s.provider.id || ''
}
function effModel(key) {
  const s = snapOf(key)
  return (s && s.model) || ''
}
/** 灰字：参数（`undefined` = 后端没给这个键 = 不发；null 也一样显示「不发」） */
function effParam(key, col) {
  const s = snapOf(key)
  const v = s && s.params ? s.params[col] : undefined
  return (v === undefined || v === null) ? '' : String(v)
}
/** 这一格现在是不是"继承了"（null / 空串 = 跟随） */
function inherited(key, col) {
  const v = rowOf(key)[col]
  return v === null || v === undefined || v === ''
}

/** 参数落值：空 ⇒ null（= 删键 / 跟随）；数字 ⇒ 该值 */
function setParam(key, col, raw) {
  const s = String(raw == null ? '' : raw).trim()
  rowOf(key)[col] = s === '' ? null : Number(s)
}

/** 已覆盖了几项（状态列） */
function overriddenCount(key) {
  const r = rowOf(key)
  let n = 0
  for (const col of ['provider', 'model', 'temperature', 'max_tokens', 'reasoning']) {
    if (!inherited(key, col)) n++
  }
  return n
}
/** 状态列文案：0 项 = 跟随默认；否则点名改了哪几项（行内 ≤20 字，细节进 title） */
const COLS_ZH = { provider: '接口', model: '模型', temperature: '温度', max_tokens: '上限', reasoning: '推理' }
function statusText(key) {
  const n = overriddenCount(key)
  if (!n) return '跟随默认'
  const names = Object.keys(COLS_ZH).filter((c) => !inherited(key, c)).map((c) => COLS_ZH[c])
  return '已改 ' + names.join('·')
}
function statusTitle(key) {
  const n = overriddenCount(key)
  if (!n) return '这条链路一个覆盖键都没写 ⇒ 接口跟 [llm] active、模型跟该接口的默认、参数跟 [llm.defaults]'
  return '这条链路写了 ' + n + ' 项覆盖（[llm.links.' + key + ']）；点每格旁边的「跟随」可以撤掉'
}
/** 推理那格的显示名 */
const levelZh = (v) => LEVEL_ZH[v] || v
</script>

<template>
  <div class="lt">
    <div class="lt-hint muted small">
      灰字 = **已保存的生效值**（来自 /api/config/llm/effective）；点一格才变成可编辑，改了才算覆盖。
    </div>
    <div class="lt-scroll">
      <table class="lt-tbl">
        <thead>
          <tr>
            <th class="lt-c-link">链路</th>
            <th class="lt-c-prov">接口</th>
            <th class="lt-c-model">模型</th>
            <th class="lt-c-num">温度</th>
            <th class="lt-c-num">输出上限</th>
            <th class="lt-c-reason">推理</th>
            <th class="lt-c-state">状态</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="l in links" :key="l.key" :data-link="l.key">
            <td class="lt-c-link">
              <b>{{ l.label }}</b>
              <span v-if="l.hint" class="lt-sub">{{ l.hint }}</span>
            </td>

            <!-- 接口 -->
            <td class="lt-c-prov">
              <span v-if="!isEditing(l.key, 'provider')" class="lt-cell"
                    title="点一下改成可编辑（不点 = 跟随当前接口）" @click="open(l.key, 'provider')">
                <span class="lt-grey">{{ effProvider(l.key) || '—' }}</span>
                <span v-if="!inherited(l.key, 'provider')" class="lt-mark" title="这条链路单独指定了接口">改</span>
              </span>
              <span v-else class="lt-edit">
                <select :value="rowOf(l.key).provider || ''" title="选「跟随当前接口」= 删掉这个键"
                        @change="rowOf(l.key).provider = $event.target.value">
                  <option value="">跟随当前接口</option>
                  <option v-for="p in providers" :key="p.id" :value="p.id">{{ p.name || p.id }}</option>
                </select>
                <button class="lt-mini" title="撤掉覆盖（发 null = 删键）"
                        @click="rowOf(l.key).provider = ''; close(l.key, 'provider')">跟随</button>
              </span>
            </td>

            <!-- 模型 -->
            <td class="lt-c-model">
              <span v-if="!isEditing(l.key, 'model')" class="lt-cell"
                    title="点一下改成可编辑（不点 = 跟随该接口的默认模型）" @click="open(l.key, 'model')">
                <span class="lt-grey" :title="effModel(l.key)">{{ effModel(l.key) || '—' }}</span>
                <span v-if="!inherited(l.key, 'model')" class="lt-mark" title="这条链路单独指定了模型">改</span>
              </span>
              <span v-else class="lt-edit">
                <input :value="rowOf(l.key).model || ''" list="lt-model-list" spellcheck="false"
                       placeholder="选一支模型 / 手打 id" title="留空 = 跟随该接口的默认模型"
                       @input="rowOf(l.key).model = $event.target.value">
                <button class="lt-mini" title="撤掉覆盖（发 null = 删键）"
                        @click="rowOf(l.key).model = ''; close(l.key, 'model')">跟随</button>
              </span>
            </td>

            <!-- 温度 -->
            <td class="lt-c-num">
              <span v-if="!isEditing(l.key, 'temperature')" class="lt-cell"
                    title="点一下改成可编辑（不点 = 跟随 [llm.defaults]）" @click="open(l.key, 'temperature')">
                <span class="lt-grey">{{ inherited(l.key, 'temperature') ? (effParam(l.key, 'temperature') || '不发') : rowOf(l.key).temperature }}</span>
                <span v-if="!inherited(l.key, 'temperature')" class="lt-mark">改</span>
              </span>
              <span v-else class="lt-edit">
                <input type="number" min="0" max="2" step="0.1"
                       :value="inherited(l.key, 'temperature') ? effParam(l.key, 'temperature') : rowOf(l.key).temperature"
                       title="0–2；清空 = 不发这个键（= 删键）"
                       @input="setParam(l.key, 'temperature', $event.target.value)">
                <button class="lt-mini" title="撤掉覆盖（发 null = 删键）"
                        @click="rowOf(l.key).temperature = null; close(l.key, 'temperature')">跟随</button>
              </span>
            </td>

            <!-- 输出上限 -->
            <td class="lt-c-num">
              <span v-if="!isEditing(l.key, 'max_tokens')" class="lt-cell"
                    title="点一下改成可编辑（不点 = 跟随 [llm.defaults]）" @click="open(l.key, 'max_tokens')">
                <span class="lt-grey">{{ inherited(l.key, 'max_tokens') ? (effParam(l.key, 'max_tokens') || '不发') : rowOf(l.key).max_tokens }}</span>
                <span v-if="!inherited(l.key, 'max_tokens')" class="lt-mark">改</span>
              </span>
              <span v-else class="lt-edit">
                <input type="number" min="256" max="65536" step="256"
                       :value="inherited(l.key, 'max_tokens') ? effParam(l.key, 'max_tokens') : rowOf(l.key).max_tokens"
                       title="256–65536；清空 = 不发这个键（= 删键）"
                       @input="setParam(l.key, 'max_tokens', $event.target.value)">
                <button class="lt-mini" title="撤掉覆盖（发 null = 删键）"
                        @click="rowOf(l.key).max_tokens = null; close(l.key, 'max_tokens')">跟随</button>
              </span>
            </td>

            <!-- 推理档位 -->
            <td class="lt-c-reason">
              <span v-if="!isEditing(l.key, 'reasoning')" class="lt-cell"
                    title="点一下改成可编辑（不点 = 不表态，由链路默认/方言决定）" @click="open(l.key, 'reasoning')">
                <span class="lt-grey">{{ inherited(l.key, 'reasoning') ? (levelZh(effParam(l.key, 'reasoning')) || '跟默认') : levelZh(rowOf(l.key).reasoning) }}</span>
                <span v-if="!inherited(l.key, 'reasoning')" class="lt-mark">改</span>
              </span>
              <span v-else class="lt-edit">
                <select :value="rowOf(l.key).reasoning || ''" title="留空 = 不表态（删这个键）"
                        @change="rowOf(l.key).reasoning = $event.target.value">
                  <option value="">跟默认</option>
                  <option v-for="lv in levels" :key="lv" :value="lv">{{ levelZh(lv) }}</option>
                  <option v-if="rowOf(l.key).reasoning && !levels.includes(rowOf(l.key).reasoning)"
                          :value="rowOf(l.key).reasoning">{{ levelZh(rowOf(l.key).reasoning) }}（当前值）</option>
                </select>
                <button class="lt-mini" title="撤掉覆盖（发 null = 删键）"
                        @click="rowOf(l.key).reasoning = null; close(l.key, 'reasoning')">跟随</button>
              </span>
            </td>

            <td class="lt-c-state" :title="statusTitle(l.key)">
              <span :class="['lt-state', { on: overriddenCount(l.key) > 0 }]">{{ statusText(l.key) }}</span>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <datalist id="lt-model-list">
      <option v-for="m in modelOptions" :key="m.provider + '/' + m.id" :value="m.id">{{ m.label || m.id }}</option>
    </datalist>
  </div>
</template>

<style scoped>
.lt { display:flex; flex-direction:column; gap:6px; min-width:0; }
.lt-hint { overflow-wrap:anywhere; }
/* 表格在窄屏**横滚**而不是硬挤：7 列各有语义，压窄只会把灰字截成省略号 */
.lt-scroll { overflow-x:auto; min-width:0; }
.lt-tbl { width:100%; border-collapse:collapse; font-size:12px; min-width:640px; }
.lt-tbl th { text-align:left; font-weight:600; font-size:11px; color:var(--ink-dim);
  padding:4px 6px; border-bottom:1px solid var(--line-soft); white-space:nowrap; }
.lt-tbl td { padding:3px 6px; border-bottom:1px dashed var(--line-soft); vertical-align:middle; }
.lt-tbl tr:last-child td { border-bottom:none; }
.lt-c-link { width:96px; }
.lt-c-prov { width:130px; }
.lt-c-model { width:180px; }
.lt-c-num { width:104px; }
.lt-c-reason { width:104px; }
.lt-c-state { width:104px; }
.lt-sub { display:block; font-size:10px; color:var(--ink-faint); }
.lt-cell { display:inline-flex; align-items:center; gap:4px; min-width:0; cursor:pointer;
  border-radius:var(--r-sm); padding:1px 3px; }
.lt-cell:hover { background:var(--bg-soft); }
.lt-grey { font-size:11.5px; color:var(--ink-faint); min-width:0; overflow:hidden;
  text-overflow:ellipsis; white-space:nowrap; max-width:100%; }
.lt-mark { flex:none; font-size:9.5px; line-height:1; padding:2px 4px; border-radius:999px;
  background:var(--main-soft); color:var(--pink-text); }
.lt-edit { display:inline-flex; align-items:center; gap:4px; min-width:0; }
.lt-edit input, .lt-edit select { height:24px; padding:2px 5px; font-size:11.5px; min-width:0; width:100%; }
.lt-mini { flex:none; height:22px; padding:0 6px; font-size:10.5px; border-radius:var(--r-sm);
  border:1px solid var(--line); background:var(--card); color:var(--ink-dim); cursor:pointer; }
.lt-mini:hover { border-color:var(--main); color:var(--main); }
.lt-state { font-size:11px; color:var(--ink-faint); }
.lt-state.on { color:var(--pink-text); font-weight:600; }
</style>
