<!--
  LlmLinkTable — L3 五 IA 的 **③ 按链路覆盖**：行 = 链路，列 = 接口 / 模型 / 温度 / 输出上限 / 推理 / 状态。

  用户口径（配置重构 §3.3-3）：「未覆盖的格子显示灰字实值（来自 /api/config/llm/effective），
  点开才可编辑 —— 一眼能看出哪条链路跟谁、谁被改过」。

  三条纪律：
  ① **灰字只能来自后端快照**（`fetchEffective` → `LlmV4::link_snapshot`）。前端自己按 defaults 算一遍
     就是同一套规则抄第二份 —— L1 之前"界面显示的与真发的不一致"正是这么来的。
  ② **"没有值"用 null 表达**（保存时发 null = 删键 = 跟随），不是空串：空串会把 `max_tokens = ""`
     写进 llm.toml，下次解析失败把整份配置打回默认值（P0 事故）。
  ③ **点开不写值**：点一下只是长出控件，控件里显示的起步值就是灰字那份；真改了才落进 rows（= 覆盖）。
     想撤回就点控件旁的「清除覆盖」—— 那才是"删这个键"。

  ⚠️ rows 是**父层的 reactive 对象**（本组件原地改属性，与 LlmParamFields 同一条绑法），
     所以父层的 dirty 判定与 buildLlmPatch 直接读它，不用一串 update: 事件。

  2026-11-26（交互整改）：
  · **点开有退路了**：以前 close() 只挂在「跟随」上 ⇒ "点错了想关掉"的唯一出路是把覆盖删掉。
    现在 Esc / 点空白处都能收起，且**不改值**；「跟随」随之改名「清除覆盖」（它才是删键）。
  · **模型候选按本行的接口过滤**：以前是全清单 datalist ⇒ A 家的模型能被选给 B 家
    （后端不校验，跑起来才 404）。选中别家的模型会当场标黄。
  · **推理档位按本行生效模型给**：以前复用"当前接口那支模型"的档位，检测用 A、精修改 B 时是错的。
-->
<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { LEVEL_ZH } from './caps.js'
import { testConnection } from '../../api/llm'
import LlmModelCombobox from './LlmModelCombobox.vue'

const props = defineProps({
  /** [{ key, label, hint }] —— 行由后端 `override_links` 给（correct 只隐藏不删） */
  links: { type: Array, default: () => [] },
  /** 覆盖表单：link → { provider, model, temperature, max_tokens, reasoning }（null/'' = 跟随） */
  rows: { type: Object, required: true },
  /** 生效快照：link → /api/config/llm/effective 的返回（null = 还没取到 / 取失败） */
  effective: { type: Object, default: () => ({}) },
  /** 可选接口（列 = 接口） */
  providers: { type: Array, default: () => [] },
  /** 可选模型（列 = 模型；候选来自 ④ 的清单，带 provider 用于按行过滤） */
  modelOptions: { type: Array, default: () => [] },
  /** 当前接口 id（= [llm] active）—— 行里没写 provider 时的回落 */
  activeId: { type: String, default: '' },
  /** 探到的候选 [{ id, label, provider }] —— 按行的接口过滤后进下拉（带「未加入」标记） */
  probeModels: { type: Array, default: () => [] },
  /** 推理档位候选（当前接口那支模型的；某行模型不一样时用 levelsByModel 覆盖） */
  levels: { type: Array, default: () => [] },
  /** model id → 档位数组（来自清单的 catalog / 探针）；取不到就回落 levels */
  levelsByModel: { type: Object, default: () => ({}) }
})
const emit = defineEmits(['pick-model'])

/** 哪个格子在编辑态：`${link}:${col}` → true */
const editing = reactive({})
const cellId = (key, col) => key + ':' + col
const isEditing = (key, col) => !!editing[cellId(key, col)]
function open(key, col) { editing[cellId(key, col)] = true }
function close(key, col) { editing[cellId(key, col)] = false }
/** 正在编辑的格子数：0 时全局监听直接短路（别为每次点击都遍历） */
const editingCount = computed(() => Object.keys(editing).filter((k) => editing[k]).length)
function closeAll() { for (const k of Object.keys(editing)) editing[k] = false }
/** 点空白处收起（输入框里的点击不算）。⚠️ 没有它 = 点错了只能删覆盖才能退出。 */
function onDocDown(e) {
  if (!editingCount.value) return
  const t = e && e.target
  if (t && typeof t.closest === 'function' && t.closest('.lt-edit')) return
  closeAll()
}
function onEsc(e) {
  if (e && e.key === 'Escape' && editingCount.value) { e.stopPropagation(); closeAll() }
}
onMounted(() => document.addEventListener('mousedown', onDocDown))
onBeforeUnmount(() => document.removeEventListener('mousedown', onDocDown))

const rowOf = (key) => props.rows[key] || (props.rows[key] = {})
const snapOf = (key) => props.effective[key] || null

/** 覆盖行里写的接口 id → 显示名（下拉里长什么样，这一格就长什么样） */
function providerName(key) {
  const id = String(rowOf(key).provider || '')
  const hit = (props.providers || []).find((p) => String(p.id || '') === id)
  return hit ? (hit.name || hit.id) : (id || '—')
}
/** 灰字：接口 = 生效那一段的名字（没取到快照就空） */
function effProvider(key) {
  const s = snapOf(key)
  if (!s || !s.provider) return ''
  return s.provider.name || s.provider.id || ''
}
/** 这一行**正在编辑的**接口（行里写了就以它为准 > 已保存快照 > 当前接口）—— 模型候选按它过滤。
 *  ⚠️ 顺序不能反过来：快照是"上次保存的"，行里的覆盖才是用户**现在**要的那一份；
 *     按快照判会把"刚把精修指到另一家、还没保存"误报成"模型属于另一家"。 */
function effProviderId(key) {
  const row = rowOf(key)
  if (row.provider) return String(row.provider)
  const s = snapOf(key)
  if (s && s.provider && s.provider.id) return String(s.provider.id)
  return String(props.activeId || '')
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

/** 这一行可选的模型：**只给该接口名下的**（已登记的 + 这个接口刚探到的候选）。
 *  ⚠️ 不再回落"全清单" —— A 家的模型选给 B 家就是 model not found（后端 problems() 也不查这条）。 */
function modelOptionsFor(key) {
  const pid = effProviderId(key)
  const all = Array.isArray(props.modelOptions) ? props.modelOptions : []
  const out = all.filter((m) => String(m.provider || '') === pid)
    .map((m) => ({ id: m.id, label: m.label || '', inList: true }))
  const seen = new Set(out.map((x) => x.id))
  for (const p of (props.probeModels || [])) {
    if (!p || !p.id || String(p.provider || '') !== pid || seen.has(p.id)) continue
    out.push({ id: p.id, label: p.label || '', inList: false })
    seen.add(p.id)
  }
  return out
}
/** 这一行现在**实际会用**的模型（行覆盖 > 生效值）*/
function rowModel(key) {
  return String(rowOf(key).model || '').trim() || effModel(key)
}
/** 选中/手打一支模型后通知父层：清单里没有就就地写进去（省掉"回模型配置页加一遍"） */
function onPickRow(key, m) {
  if (!m || !m.id) return
  emit('pick-model', { providerId: effProviderId(key), model: String(m.id), inList: m.inList === true })
}
function onCommitRow(key, v) {
  if (!v) return
  emit('pick-model', { providerId: effProviderId(key), model: String(v), inList: false })
}

/* ── 测试：按**这一行**的接口 + 模型发一次最小请求 ──────────────────────────
   「测试这个接口」只测当前接口的默认模型 ⇒ 四条链路各用不同模型时没有任何入口验证。
   结果常驻行内（延迟 / 失败短因），整表可以一键全测。 */
const testState = reactive({})
const testingAll = ref(false)
async function testRow(key) {
  const pid = effProviderId(key)
  const model = rowModel(key)
  if (!pid) { testState[key] = { ok: false, text: '没接口', title: '这一行还没有生效的接口' }; return }
  const p = (props.providers || []).find((x) => String(x.id || '') === pid) || null
  testState[key] = { busy: true, text: '测试中…' }
  try {
    const r = await testConnection({
      provider: pid,
      base_url: p ? p.base_url : '',
      api_key: p ? p.api_key : '',
      model
    })
    const ms = r && typeof r.latency_ms === 'number' ? r.latency_ms + 'ms' : ''
    testState[key] = { ok: true, text: ms || '通', title: '模型 ' + (r && r.model ? r.model : model) }
  } catch (e) {
    const full = (e && e.message) ? e.message : String(e)
    testState[key] = { ok: false, text: '失败', title: full }
  }
}
async function testAll() {
  if (testingAll.value) return
  testingAll.value = true
  try {
    for (const l of props.links) await testRow(l.key)
  } finally {
    testingAll.value = false
  }
}
/** 行里指定的模型**不属于**这个接口：显式说出来（后端 problems() 不校验这条，错配只在跑任务时 404） */
function modelMismatch(key) {
  const id = String(rowOf(key).model || '').trim()
  if (!id) return ''
  const pid = effProviderId(key)
  const hit = (props.modelOptions || []).find((m) => String(m.id || '') === id)
  if (!hit) return ''
  return String(hit.provider || '') === pid ? '' : '这支模型属于另一家接口'
}
/** 这一行的推理档位：**按该行生效模型**的能力（取不到才回落全局那份） */
function levelsFor(key) {
  // 先把"这一行自己写的模型"算进来：改了模型但还没保存时，档位候选也该跟着换成新模型的
  const id = String(rowOf(key).model || '').trim() || effModel(key)
  const lv = id ? (props.levelsByModel || {})[id] : null
  return Array.isArray(lv) && lv.length ? lv : (props.levels || [])
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
/** 状态列文案：只报**几项**（≤20 字）；改了哪几项进 title —— 每格本来就挂了「改」标记，
 *  再在这里复述一遍就是同一件事说三遍。 */
const COLS_ZH = { provider: '接口', model: '模型', temperature: '温度', max_tokens: '上限', reasoning: '推理' }
function statusText(key) {
  const n = overriddenCount(key)
  return n ? '已改 ' + n + ' 项' : '跟随默认'
}
function statusTitle(key) {
  const n = overriddenCount(key)
  if (!n) return '这条链路一个覆盖键都没写 ⇒ 接口跟 [llm] active、模型跟该接口的默认、参数跟 [llm.defaults]'
  const names = Object.keys(COLS_ZH).filter((c) => !inherited(key, c)).map((c) => COLS_ZH[c])
  return '这条链路写了 ' + n + ' 项覆盖（' + names.join(' / ') + '）；点每格旁的「清除覆盖」可以撤掉'
}
/** 推理那格的显示名 */
const levelZh = (v) => LEVEL_ZH[v] || v
</script>

<template>
  <div class="lt" @keydown="onEsc">
    <div class="lt-top">
      <span class="lt-hint muted small">
        灰字 = 已保存的生效值（后端算）。点一格才可编辑，Esc 或点空白处取消；「清除覆盖」才是删键。
      </span>
      <span class="lt-spacer"></span>
      <button class="lt-mini" type="button" :disabled="testingAll" title="按每行生效的接口与模型各发一次最小请求"
              @click="testAll">{{ testingAll ? '测试中…' : '全部测试' }}</button>
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
            <th class="lt-c-test">测试</th>
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
                <!-- 有覆盖就显示**你改的值**（title 里给已保存的生效值）—— 温度/上限两格一直是这个口径，
                     接口与模型以前只显示生效值 ⇒ 改完看不出来自己改了什么 -->
                <span class="lt-grey" :title="inherited(l.key, 'provider') ? effProvider(l.key) : ('生效值：' + (effProvider(l.key) || '—'))">
                  {{ inherited(l.key, 'provider') ? (effProvider(l.key) || '—') : providerName(l.key) }}
                </span>
                <span v-if="!inherited(l.key, 'provider')" class="lt-mark" title="这条链路单独指定了接口">改</span>
              </span>
              <span v-else class="lt-edit">
                <select :value="rowOf(l.key).provider || ''" title="选「跟随当前接口」= 删掉这个键"
                        @change="rowOf(l.key).provider = $event.target.value">
                  <option value="">跟随当前接口</option>
                  <option v-for="p in providers" :key="p.id" :value="p.id">{{ p.name || p.id }}</option>
                </select>
                <button class="lt-mini" title="撤掉覆盖（发 null = 删键）"
                        @click="rowOf(l.key).provider = ''; close(l.key, 'provider')">清除覆盖</button>
              </span>
            </td>

            <!-- 模型 -->
            <td class="lt-c-model">
              <span v-if="!isEditing(l.key, 'model')" class="lt-cell"
                    title="点一下改成可编辑（不点 = 跟随该接口的默认模型）" @click="open(l.key, 'model')">
                <span class="lt-grey" :title="inherited(l.key, 'model') ? effModel(l.key) : ('生效值：' + (effModel(l.key) || '—'))">
                  {{ inherited(l.key, 'model') ? (effModel(l.key) || '—') : rowOf(l.key).model }}
                </span>
                <span v-if="!inherited(l.key, 'model')" class="lt-mark" title="这条链路单独指定了模型">改</span>
                <span v-if="modelMismatch(l.key)" class="lt-bad" :title="modelMismatch(l.key) + '：请求会打到 ' +
                      (effProvider(l.key) || '当前接口') + '，多半是 model not found'">!</span>
              </span>
              <span v-else class="lt-edit">
                <LlmModelCombobox :model-value="rowOf(l.key).model || ''" :options="modelOptionsFor(l.key)"
                                  placeholder="选一支模型 / 手打 id" :aria-label="'模型（' + l.label + '）'"
                                  @update:model-value="rowOf(l.key).model = $event"
                                  @pick="onPickRow(l.key, $event)" @commit="onCommitRow(l.key, $event)" />
                <button class="lt-mini" title="撤掉覆盖（发 null = 删键）"
                        @click="rowOf(l.key).model = ''; close(l.key, 'model')">清除覆盖</button>
              </span>
              <div v-if="modelMismatch(l.key)" class="lt-warn">{{ modelMismatch(l.key) }}</div>
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
                        @click="rowOf(l.key).temperature = null; close(l.key, 'temperature')">清除覆盖</button>
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
                        @click="rowOf(l.key).max_tokens = null; close(l.key, 'max_tokens')">清除覆盖</button>
              </span>
            </td>

            <!-- 推理档位：候选按**本行生效模型**的能力（不是当前接口那支） -->
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
                  <option v-for="lv in levelsFor(l.key)" :key="lv" :value="lv">{{ levelZh(lv) }}</option>
                  <option v-if="rowOf(l.key).reasoning && !levelsFor(l.key).includes(rowOf(l.key).reasoning)"
                          :value="rowOf(l.key).reasoning">{{ levelZh(rowOf(l.key).reasoning) }}（当前值）</option>
                </select>
                <button class="lt-mini" title="撤掉覆盖（发 null = 删键）"
                        @click="rowOf(l.key).reasoning = null; close(l.key, 'reasoning')">清除覆盖</button>
              </span>
            </td>

            <!-- 测试：按**这一行**的接口 + 模型发一次最小请求（「测试这个接口」只测默认模型） -->
            <td class="lt-c-test">
              <button class="lt-test" type="button" :disabled="!!(testState[l.key] && testState[l.key].busy)"
                      :title="(testState[l.key] && testState[l.key].title) || '用这一行的接口与模型发一次最小请求'"
                      @click="testRow(l.key)">测</button>
              <span v-if="testState[l.key]" class="lt-testres"
                    :class="{ ok: testState[l.key].ok === true, err: testState[l.key].ok === false }"
                    :title="testState[l.key].title || ''">{{ testState[l.key].text }}</span>
            </td>

            <td class="lt-c-state" :title="statusTitle(l.key)">
              <span :class="['lt-state', { on: overriddenCount(l.key) > 0 }]">{{ statusText(l.key) }}</span>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>

<style scoped>
.lt { display:flex; flex-direction:column; gap:6px; min-width:0; }
.lt-top { display:flex; align-items:baseline; gap:8px; flex-wrap:wrap; min-width:0; }
.lt-top .lt-spacer { flex:1 1 auto; min-width:8px; }
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
.lt-c-test { width:118px; white-space:nowrap; }
.lt-c-state { width:96px; }
/* 行内测试：结果常驻（延迟 / 失败），不是一闪就没的 toast */
.lt-test { height:22px; padding:0 7px; font-size:10.5px; border-radius:var(--r-sm);
  border:1px solid var(--line); background:var(--card); color:var(--ink-dim); cursor:pointer; }
.lt-test:hover:not(:disabled) { border-color:var(--main); color:var(--main); }
.lt-test:disabled { opacity:.55; cursor:not-allowed; }
.lt-testres { margin-left:6px; font-size:10.5px; color:var(--ink-faint); }
.lt-testres.ok { color:var(--green-deep); }
.lt-testres.err { color:var(--pink-text); }
.lt-sub { display:block; font-size:10px; color:var(--ink-faint); }
.lt-cell { display:inline-flex; align-items:center; gap:4px; min-width:0; cursor:pointer;
  border-radius:var(--r-sm); padding:1px 3px; }
.lt-cell:hover { background:var(--bg-soft); }
.lt-grey { font-size:11.5px; color:var(--ink-faint); min-width:0; overflow:hidden;
  text-overflow:ellipsis; white-space:nowrap; max-width:100%; }
.lt-mark { flex:none; font-size:9.5px; line-height:1; padding:2px 4px; border-radius:999px;
  background:var(--main-soft); color:var(--pink-text); }
/* 模型/接口错配：红点 + 行内一句话（别只挂 title，用户不会去 hover） */
.lt-bad { flex:none; width:14px; height:14px; border-radius:50%; background:var(--pink-text);
  color:var(--card); font-size:10px; line-height:14px; text-align:center; font-weight:700; }
.lt-warn { font-size:10px; color:var(--pink-text); line-height:1.3; margin-top:1px; }
/* 编辑态**不许折行**：折了之后那一行会突然长高，7 行表的节奏就乱了。
   取消不用按钮 —— Esc / 点空白处收起（见 .lt-hint），按钮只留「清除覆盖」。 */
.lt-edit { display:inline-flex; align-items:center; gap:4px; min-width:0; flex-wrap:nowrap; }
.lt-edit input, .lt-edit select { height:24px; padding:2px 5px; font-size:11.5px; min-width:40px; width:auto;
  flex:1 1 auto; }
.lt-mini { flex:none; height:22px; padding:0 6px; font-size:10.5px; border-radius:var(--r-sm);
  border:1px solid var(--line); background:var(--card); color:var(--ink-dim); cursor:pointer; }
.lt-mini:hover { border-color:var(--main); color:var(--main); }
.lt-state { font-size:11px; color:var(--ink-faint); }
.lt-state.on { color:var(--pink-text); font-weight:600; }
</style>
