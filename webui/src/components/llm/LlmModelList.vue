<script setup>
// LlmModelList — L3 五 IA 的 **④ 模型列表**（独立折叠、与「高级配置」平级）。
//
// 它是 D1 的那张清单：`[[llm.models]]` —— **一份真相**。链路（③）与接口（①）都只**引用**它的 id。
//
// 两条硬口径：
//  ① **「获取模型」只填建议、不改生效值**（S3/F8）：探针/内置目录回的候选只进右列，
//     用户点「加入清单」才**落盘**。历史病根：以前每取一次就往文件里写一批，清单越攒越多。
//  ② **清单里的值说了算**（catalog 只是建议）：行内的「能力（手填）」写的是
//     `[[llm.models]].overrides`（thinking_wire / max_output_wire / temperature / reasoning / input），
//     它盖过 catalog；改完保存才生效。
//
// ⚠️ models 是父层的 reactive 数组（本组件原地增删改），保存仍走父层的 saveLlm。
// ⚠️ 这里**没有**模型弹层/下拉浮层：清单就是一张表（"模型选择三处入口"要消灭到底）。
import { reactive, ref } from 'vue'
import Icon from '../Icon.vue'
import { confirmDialog } from '../../stores/dialog'
import { MODEL_SOURCE_ZH, fetchModels } from '../../api/llm'
import { LEVEL_VOCAB, LEVEL_ZH } from './caps.js'

const props = defineProps({
  /** 父层 reactive 的 `form.llm.models`（v4 是数组） */
  models: { type: Array, required: true },
  /** 可选接口（新行要从这里选归属） */
  providers: { type: Array, default: () => [] },
  /** 当前接口 id（「获取模型」默认按它探；新行也默认挂它） */
  activeId: { type: String, default: '' },
  /** 当前接口的地址 / 草稿密钥（探针用：没保存的也能探） */
  baseUrl: { type: String, default: '' },
  apiKey: { type: String, default: '' },
  /** 方言注册表（`/api/config/llm/dialects` 的 dialects；推理写法的候选） */
  dialects: { type: Array, default: () => [] }
})
const emit = defineEmits(['probe'])

/** 探到的候选：{ state: idle|loading|done|error, models: [], note } —— **失败 ≠ 空列表** */
const remote = reactive({ state: 'idle', models: [], note: '' })
const fetching = ref(false)
/** 过滤词（按 id / 显示名，大小写不敏感） */
const filter = ref('')
/** 每行"能力（手填）"是否展开：provider/id → bool */
const open = reactive({})
const rowKey = (m) => String(m.provider || '') + '/' + String(m.id || '')

const apiKeySet = (id) => {
  const p = props.providers.find((x) => x.id === id)
  return !!(p && p.api_key_set)
}
const chosen = (id) => props.models.some((m) => String(m.id || '').trim() === String(id || '').trim()
  && String(m.provider || '') === String(props.activeId || ''))
const filtered = () => {
  const list = remote.models || []
  const q = String(filter.value || '').trim().toLowerCase()
  if (!q) return list
  return list.filter((m) => String(m.id || '').toLowerCase().includes(q)
    || String(m.name || '').toLowerCase().includes(q))
}

/** 「获取模型」：按当前接口的地址探一次（表命中不联网；表外才真发） */
async function doFetch() {
  if (fetching.value) return
  const base = String(props.baseUrl || '').trim()
  if (!base) { remote.state = 'error'; remote.note = '先在上面「当前接口」填 API 地址'; return }
  fetching.value = true
  remote.state = 'loading'
  remote.note = ''
  try {
    const r = await fetchModels(base, props.apiKey, props.activeId)
    remote.state = 'done'
    remote.models = r.models
    remote.note = '来源：' + (MODEL_SOURCE_ZH[r.source] || r.source) + '（' + r.models.length + ' 支）'
      + (r.note ? ' · ' + r.note : '')
    // 探针带回的能力交给父层（只影响展示：滑块上限 / 档位候选），**不落盘**
    emit('probe', { providerId: props.activeId, models: r.models })
  } catch (e) {
    remote.state = 'error'
    remote.models = []
    remote.note = (e && e.message) ? e.message : String(e)
  } finally {
    fetching.value = false
  }
}

/** 加入清单：只写 id / 显示名 / 归属 / 探到的能力（catalog）；其余键一律不编 */
async function add(id) {
  const m = (remote.models || []).find((x) => String(x.id || '') === String(id || ''))
  if (!m || chosen(m.id)) return
  props.models.push({
    id: String(m.id || '').trim(),
    provider: String(props.activeId || ''),
    label: String(m.name || ''),
    catalog: m.caps && typeof m.caps === 'object' ? m.caps : null,
    overrides: null
  })
  if (!open[rowKey({ id: m.id, provider: props.activeId })]) open[rowKey({ id: m.id, provider: props.activeId })] = false
}
/** 手填一行（服务器什么都不说时的兜底） */
function addManual() {
  if (!props.providers.length) return
  props.models.push({ id: '', provider: String(props.activeId || ''), label: '', catalog: null, overrides: null })
}
async function removeModel(i) {
  const m = props.models[i]
  const ok = await confirmDialog('从模型清单里删除「' + ((m && m.id) || '(未命名)') + '」？（保存后生效）',
    { title: '删除模型', danger: true })
  if (!ok) return
  props.models.splice(i, 1)
}
/** 接口被删掉时父层会摘掉对应行；这里只保证下拉里有一个"当前值"可选 */
const providerIds = () => props.providers.map((p) => p.id)

/** ---- 手填能力（overrides）---- */
function ov(m) {
  if (!m.overrides) m.overrides = {}
  return m.overrides
}
function setOv(m, key, val) {
  const o = ov(m)
  if (val === '' || val === null || val === undefined) delete o[key]
  else o[key] = val
}
/** 输入模态：overrides.input 是数组；没手填 = 跟随 catalog */
function inputOf(m) {
  if (m.overrides && Array.isArray(m.overrides.input)) return m.overrides.input
  const c = m.catalog && m.catalog.input
  return Array.isArray(c) ? c : ['text']
}
function toggleInput(m, kind, on) {
  const cur = new Set(inputOf(m))
  if (on) cur.add(kind); else cur.delete(kind)
  ov(m).input = ['text', 'image'].filter((k) => cur.has(k))
}
/** 档位表：overrides.reasoning 是 { level: wire } */
function levelsOf(m) {
  const o = m.overrides && m.overrides.reasoning
  return o && typeof o === 'object' ? o : {}
}
function addLevel(m) {
  const used = new Set(Object.keys(levelsOf(m)))
  const next = LEVEL_VOCAB.find((v) => !used.has(v)) || ''
  ov(m).reasoning = Object.assign({}, levelsOf(m), { [next]: next })
}
function setLevelKey(m, from, to) {
  const cur = levelsOf(m)
  const out = {}
  for (const [k, v] of Object.entries(cur)) out[k === from ? to : k] = k === from ? (v === from ? to : v) : v
  ov(m).reasoning = out
}
function setLevelWire(m, level, wire) {
  ov(m).reasoning = Object.assign({}, levelsOf(m), { [level]: wire })
}
function removeLevel(m, level) {
  const out = Object.assign({}, levelsOf(m))
  delete out[level]
  if (Object.keys(out).length) ov(m).reasoning = out
  else delete ov(m).reasoning
}
/** 一行能力摘要（≤20 字；细节进 title） */
function summary(m) {
  const c = m.catalog || {}
  const input = inputOf(m)
  const bits = []
  bits.push(input.length ? input.map((x) => (x === 'image' ? '图片' : '文字')).join('+') : '无输入')
  const lv = Object.keys(levelsOf(m)).length || (Array.isArray(c.levels) ? c.levels.length : 0)
  bits.push(lv ? '推理 ' + lv + ' 档' : '无推理档')
  if (c.max_output) bits.push('上限 ' + c.max_output)
  return bits.join(' · ')
}
function summaryTitle(m) {
  const c = m.catalog || {}
  const bits = ['catalog 来自内置目录 / 联网探测（建议值）；下面「能力（手填）」优先']
  if (c.context_window) bits.push('上下文 ' + c.context_window)
  if (c.max_output) bits.push('输出上限 ' + c.max_output)
  if (c.thinking_wire) bits.push('推理写法 ' + c.thinking_wire)
  if (c.max_output_wire) bits.push('输出字段 ' + c.max_output_wire)
  return bits.join('；')
}
const dialectZh = (id) => {
  const d = props.dialects.find((x) => x.id === id)
  return d ? (d.label || d.id) : id
}
</script>

<template>
  <div class="ml">
    <div class="ml-hint muted small">
      「获取模型」只填建议；点「加入清单」才落盘。链路与接口都从这张表里选模型。
    </div>

    <!-- 已加入的清单：一张表（v4：[[llm.models]]，(接口, id) 唯一） -->
    <div class="ml-scroll">
      <table class="ml-tbl">
        <thead>
          <tr>
            <th class="ml-c-id">模型 id</th>
            <th class="ml-c-label">显示名</th>
            <th class="ml-c-prov">接口</th>
            <th class="ml-c-caps">能力</th>
            <th class="ml-c-act"></th>
          </tr>
        </thead>
        <tbody>
          <template v-for="(m, i) in models" :key="i">
          <tr>
            <td><input v-model.trim="m.id" class="ml-input" placeholder="如 deepseek-chat" spellcheck="false"></td>
            <td><input v-model="m.label" class="ml-input" placeholder="（可空，空则用 id）"></td>
            <td>
              <select v-model="m.provider" class="ml-input" title="这份模型属于哪一份接口">
                <option v-for="p in providers" :key="p.id" :value="p.id">{{ p.name || p.id }}</option>
                <option v-if="!providerIds().includes(m.provider)" :value="m.provider">{{ m.provider || '（未归属）' }}</option>
              </select>
            </td>
            <td class="ml-caps" :title="summaryTitle(m)">{{ summary(m) }}</td>
            <td class="ml-act">
              <button class="ml-mini" :title="open[rowKey(m)] ? '收起手填能力' : '展开手填能力（盖过 catalog）'"
                      @click="open[rowKey(m)] = !open[rowKey(m)]">{{ open[rowKey(m)] ? '▾' : '▸' }}</button>
              <button class="ml-ico danger" title="从清单删除（保存后生效）" @click="removeModel(i)">
                <Icon name="trash" :size="12" />
              </button>
            </td>
          </tr>
          <tr v-if="open[rowKey(m)]" class="ml-ov-tr">
            <td colspan="5">
              <div class="ml-ov" :data-model="m.id">
                <div class="ml-ov-row">
                  <span class="ml-lbl">输入</span>
                  <label class="ml-chk"><input type="checkbox" :checked="inputOf(m).includes('text')"
                         @change="toggleInput(m, 'text', $event.target.checked)"> 文字</label>
                  <label class="ml-chk"><input type="checkbox" :checked="inputOf(m).includes('image')"
                         @change="toggleInput(m, 'image', $event.target.checked)"> 图片</label>
                  <span class="ml-spacer"></span>
                  <span class="ml-muted" title="catalog 是内置目录/探针给的建议值；手填优先">手填盖过 catalog</span>
                </div>
                <div class="ml-ov-row">
                  <span class="ml-lbl">温度</span>
                  <select class="ml-sel" :value="(m.overrides && m.overrides.temperature) || ''"
                          @change="setOv(m, 'temperature', $event.target.value)">
                    <option value="">跟随能力表</option>
                    <option value="supported">支持</option>
                    <option value="unsupported">不支持（不发）</option>
                  </select>
                  <span class="ml-lbl ml-lbl2">输出字段</span>
                  <select class="ml-sel" :value="(m.overrides && m.overrides.max_output_wire) || ''"
                          @change="setOv(m, 'max_output_wire', $event.target.value)">
                    <option value="">跟随方言</option>
                    <option value="max_tokens">max_tokens</option>
                    <option value="max_completion_tokens">max_completion_tokens</option>
                  </select>
                </div>
                <div class="ml-ov-row">
                  <span class="ml-lbl">推理写法</span>
                  <select class="ml-sel" :value="(m.overrides && m.overrides.thinking_wire) || ''"
                          @change="setOv(m, 'thinking_wire', $event.target.value)">
                    <option value="">跟随方言（自动判定）</option>
                    <option v-for="d in dialects" :key="d.id" :value="d.id">{{ dialectZh(d.id) }}</option>
                    <option v-if="m.overrides && m.overrides.thinking_wire && !dialects.some((d) => d.id === m.overrides.thinking_wire)"
                            :value="m.overrides.thinking_wire">{{ m.overrides.thinking_wire }}</option>
                  </select>
                  <span class="ml-spacer"></span>
                  <button class="ml-mini" title="加一档（档位 → 请求体里的写法）" @click="addLevel(m)">+ 档位</button>
                </div>
                <div v-for="(wire, level) in levelsOf(m)" :key="level" class="ml-ov-row ml-lv">
                  <select class="ml-sel" :value="level" title="内部档位（词表固定）"
                          @change="setLevelKey(m, level, $event.target.value)">
                    <option v-for="v in LEVEL_VOCAB" :key="v" :value="v">{{ LEVEL_ZH[v] || v }}</option>
                  </select>
                  <input class="ml-sel" :value="wire" placeholder="请求体里的写法（如 max）"
                         title="原样发给 provider；空串 = 支持但不发送" @input="setLevelWire(m, level, $event.target.value)">
                  <button class="ml-ico danger" title="删掉这一档" @click="removeLevel(m, level)">
                    <Icon name="x" :size="11" />
                  </button>
                </div>
              </div>
            </td>
          </tr>
          </template>
          <tr v-if="!models.length">
            <td colspan="5" class="ml-empty">清单还是空的 —— 右边「获取模型」挑几支，或手填一行。</td>
          </tr>
        </tbody>
      </table>

    </div>

    <div class="ml-foot">
      <button class="ml-mini" :disabled="!providers.length" title="手填一行（服务器什么都不说时的兜底）"
              @click="addManual">+ 手填一行</button>
      <span class="ml-spacer"></span>
      <input v-model="filter" class="ml-filter" placeholder="过滤候选…" title="按 id / 显示名即时筛">
      <button class="ml-mini" :disabled="fetching" title="按「当前接口」的地址取可用模型（没保存的地址/密钥也能取）"
              @click="doFetch">{{ fetching ? '获取中…' : '获取模型' }}</button>
    </div>

    <!-- 候选（**只做建议**，不落盘；失败 ≠ 空列表，三态分开说） -->
    <div v-if="remote.state !== 'idle'" class="ml-remote">
      <div class="ml-remote-head">
        <b>候选</b>
        <span v-if="remote.note" class="ml-muted" :title="remote.note">{{ remote.note }}</span>
      </div>
      <div v-if="remote.state === 'loading'" class="ml-muted">获取中…</div>
      <div v-else-if="remote.state === 'error'" class="ml-err" :title="remote.note" data-ml-remote="error">
        获取失败：{{ remote.note }}
        <button class="ml-mini" @click="doFetch">重试</button>
      </div>
      <div v-else-if="!remote.models.length" class="ml-muted">服务器没返回模型（可以手填一行）。</div>
      <div v-else-if="!filtered().length" class="ml-muted">没有匹配「{{ filter }}」的模型。</div>
      <div v-else class="ml-remote-list">
        <div v-for="m in filtered()" :key="m.id" class="ml-remote-row">
          <span class="ml-remote-id" :title="m.id">{{ m.id }}</span>
          <span class="ml-muted ml-remote-name" :title="m.name || ''">{{ m.name || '' }}</span>
          <span class="ml-spacer"></span>
          <span v-if="chosen(m.id)" class="ml-muted">（已在清单）</span>
          <button v-else class="ml-mini" title="加进清单（只写 id / 显示名 / 探到的能力）"
                  @click="add(m.id)">加入</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.ml { display:flex; flex-direction:column; gap:6px; min-width:0; }
.ml-hint { overflow-wrap:anywhere; }
.ml-scroll { overflow-x:auto; min-width:0; }
.ml-tbl { width:100%; border-collapse:collapse; font-size:12px; min-width:560px; }
.ml-tbl th { text-align:left; font-weight:600; font-size:11px; color:var(--ink-dim);
  padding:4px 6px; border-bottom:1px solid var(--line-soft); white-space:nowrap; }
.ml-tbl td { padding:2px 6px; border-bottom:1px dashed var(--line-soft); vertical-align:middle; }
.ml-c-id { width:34%; }
.ml-c-label { width:22%; }
.ml-c-prov { width:20%; }
.ml-c-act { width:56px; text-align:right; }
.ml-input { width:100%; min-width:0; height:26px; padding:2px 6px; font-size:11.5px; }
.ml-caps { font-size:11px; color:var(--ink-faint); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.ml-act { white-space:nowrap; }
.ml-mini { flex:none; height:22px; padding:0 7px; font-size:11px; border-radius:var(--r-sm);
  border:1px solid var(--line); background:var(--card); color:var(--ink); cursor:pointer; }
.ml-mini:hover { border-color:var(--main); color:var(--main); }
.ml-mini:disabled { opacity:.5; cursor:not-allowed; }
.ml-ico { flex:none; width:22px; height:22px; padding:0; display:inline-flex; align-items:center;
  justify-content:center; border:1px solid transparent; background:transparent; border-radius:var(--r-sm);
  color:var(--ink-dim); cursor:pointer; }
.ml-ico.danger:hover { color:var(--pink-text); border-color:var(--pink-text); }
.ml-empty { color:var(--ink-dim); font-size:11.5px; padding:8px 6px; }
.ml-ov { padding:4px 6px 6px 18px; border-left:2px solid var(--main-soft); margin:2px 0 6px;
  display:flex; flex-direction:column; gap:2px; }
.ml-ov-row { display:flex; align-items:center; gap:8px; min-height:28px; min-width:0; flex-wrap:wrap; }
.ml-lbl { flex:none; font-size:11.5px; color:var(--ink-dim); min-width:52px; }
.ml-lbl2 { min-width:64px; }
.ml-chk { display:inline-flex; align-items:center; gap:5px; font-size:11.5px; color:var(--ink); cursor:pointer; }
.ml-chk input { margin:0; }
.ml-sel { flex:none; height:24px; padding:1px 4px; font-size:11.5px; min-width:120px; width:auto; }
.ml-lv { padding-left:60px; }
.ml-spacer { flex:1 1 auto; min-width:4px; }
.ml-muted { font-size:11px; color:var(--ink-faint); min-width:0; overflow:hidden;
  text-overflow:ellipsis; white-space:nowrap; }
.ml-foot { display:flex; align-items:center; gap:8px; min-width:0; flex-wrap:wrap; }
.ml-filter { flex:0 1 140px; height:24px; padding:2px 6px; font-size:11.5px; }
.ml-remote { border:1px dashed var(--line-soft); border-radius:8px; padding:6px 8px; min-width:0; }
.ml-remote-head { display:flex; align-items:baseline; gap:8px; min-width:0; margin-bottom:4px; }
.ml-remote-list { display:flex; flex-direction:column; max-height:240px; overflow:auto; min-width:0; }
.ml-remote-row { display:flex; align-items:center; gap:8px; min-height:26px; min-width:0; }
.ml-remote-id { flex:0 1 auto; max-width:60%; min-width:0; overflow:hidden; text-overflow:ellipsis;
  white-space:nowrap; font-size:11.5px; }
.ml-remote-name { flex:1 1 auto; }
.ml-err { font-size:11.5px; color:var(--pink-text); overflow-wrap:anywhere; }
</style>
