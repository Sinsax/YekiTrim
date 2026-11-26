<script setup>
// LlmModelList — 「模型配置」子页**右栏下半**：**某一个接口**的模型清单。
//
// 用户口径（2026-11-26）：右栏 = 对应 Provider 的模型列表，「在对应的位置添加模型或 Provider」——
// 于是这张表**只画当前这一份接口的模型**，添加模型的三条路（取候选 / 手填 / 能力手填）都在这里，
// 不再有"展开折叠块 → 加入 → 回上面选"的往返。
//
// 两条硬口径（沿用 L3）：
//  ① **「取候选」只填建议、不改生效值**：探到的候选进下面的候选区，点「加入」才**落盘**（保存后才写 llm.toml）。
//     历史病根：以前每取一次就往文件里写一批，清单越攒越多。
//  ② **清单里的值说了算**（catalog 只是建议）：行内「能力（手填）」写的是 [[llm.models]].overrides，盖过 catalog。
//
// ⚠️ models 是父层的 reactive **全量数组**（本组件按 provider 过滤后原地增删改），保存仍走父层的 saveLlm。
import { computed, reactive, ref } from 'vue'
import Icon from '../Icon.vue'
import { confirmDialog } from '../../stores/dialog'
import { MODEL_SOURCE_ZH, fetchModels } from '../../api/llm'
import { LEVEL_VOCAB, LEVEL_ZH, normalizeCaps } from './caps.js'
import { parseExtraObject, stringifyExtra } from './extra.js'

const props = defineProps({
  /** 父层 reactive 的 `form.llm.models`（**全量**，本组件按 providerId 过滤） */
  models: { type: Array, required: true },
  /** 这一栏画哪一份接口的模型 */
  providerId: { type: String, default: '' },
  /** 该接口的地址 / 草稿密钥（探针用：没保存的也能探） */
  baseUrl: { type: String, default: '' },
  apiKey: { type: String, default: '' },
  /** 方言注册表（`/api/config/llm/dialects` 的 dialects；推理写法的候选） */
  dialects: { type: Array, default: () => [] }
})
const emit = defineEmits(['probe'])

/** 这一份接口名下的行（同一个对象引用 —— 删除时用 indexOf 找回全量里的位置） */
const rows = computed(() => props.models.filter((m) => String(m.provider || '') === String(props.providerId || '')))

/** 探到的候选：{ state: idle|loading|done|error, models: [], note } —— **失败 ≠ 空列表** */
const remote = reactive({ state: 'idle', models: [], note: '' })
const fetching = ref(false)
/** 过滤词（按 id / 显示名，大小写不敏感） */
const filter = ref('')
const candFilter = ref('')
/** 每行"能力（手填）"是否展开：`provider/id`（空 id 用行号兜底）→ bool */
const open = reactive({})
const rowKey = (m, i) => (m && m.provider ? m.provider : '') + '/' + ((m && m.id) || ('#' + i))

const chosen = (id) => rows.value.some((m) => String(m.id || '').trim() === String(id || '').trim())
const candList = () => {
  const list = remote.models || []
  const q = String(candFilter.value || '').trim().toLowerCase()
  if (!q) return list
  return list.filter((m) => String(m.id || '').toLowerCase().includes(q) || String(m.name || '').toLowerCase().includes(q))
}

/** 「取候选」：按**这一份接口**的地址探一次（表命中不联网；表外才真发） */
async function doFetch() {
  if (fetching.value) return
  const base = String(props.baseUrl || '').trim()
  if (!base) { remote.state = 'error'; remote.note = '先在这份接口的「填密钥」里填好地址，再来取候选'; return }
  fetching.value = true
  remote.state = 'loading'
  remote.note = ''
  try {
    const r = await fetchModels(base, props.apiKey, props.providerId)
    remote.state = 'done'
    remote.models = r.models
    remote.note = '来源：' + (MODEL_SOURCE_ZH[r.source] || r.source) + '（' + r.models.length + ' 支）'
      + (r.note ? ' · ' + r.note : '')
    // 探针带回的能力交给父层（只影响展示：滑块上限 / 档位候选），**不落盘**
    emit('probe', { providerId: props.providerId, models: r.models })
  } catch (e) {
    remote.state = 'error'
    remote.models = []
    remote.note = (e && e.message) ? e.message : String(e)
  } finally {
    fetching.value = false
  }
}

/** 加入清单：只写 id / 显示名 / 归属 / 探到的能力（catalog）；其余键一律不编 */
function add(id) {
  const m = (remote.models || []).find((x) => String(x.id || '') === String(id || ''))
  if (!m || chosen(m.id)) return
  props.models.push({
    id: String(m.id || '').trim(),
    provider: String(props.providerId || ''),
    label: String(m.name || ''),
    catalog: m.caps && typeof m.caps === 'object' ? m.caps : null,
    overrides: null
  })
}
/** 手填一行（服务器什么都不说时的兜底） */
function addManual() {
  if (!props.providerId) return
  props.models.push({ id: '', provider: String(props.providerId || ''), label: '', catalog: null, overrides: null })
}
async function removeModel(m) {
  const ok = await confirmDialog('从模型清单里删除「' + (m.id || '(未命名)') + '」？（保存后生效）',
    { title: '删除模型', danger: true })
  if (!ok) return
  const i = props.models.indexOf(m)
  if (i >= 0) props.models.splice(i, 1)
}

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
/** 清单里那份 catalog **归一后**的能力（catalog 的键名有好几种写法，别在这儿各判一次） */
function catalogCaps(m) {
  return normalizeCaps(m.catalog || null)
}
/** 输入模态：overrides.input 是数组；没手填 = 跟随 catalog */
function inputOf(m) {
  if (m.overrides && Array.isArray(m.overrides.input)) return m.overrides.input
  const c = catalogCaps(m)
  const out = []
  if (!c || (c.input && c.input.text !== false)) out.push('text')
  if (c && c.input && c.input.image) out.push('image')
  return out.length ? out : ['text']
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
  const c = catalogCaps(m) || {}
  const input = inputOf(m)
  const bits = []
  bits.push(input.length ? input.map((x) => (x === 'image' ? '图片' : '文字')).join('+') : '无输入')
  const lv = Object.keys(levelsOf(m)).length || (Array.isArray(c.levels) ? c.levels.length : 0)
  bits.push(lv ? '推理 ' + lv + ' 档' : '无推理档')
  if (c.maxOutput) bits.push('上限 ' + c.maxOutput)
  const nExtra = m.extra_params && typeof m.extra_params === 'object' ? Object.keys(m.extra_params).length : 0
  if (nExtra) bits.push('额外 ' + nExtra + ' 键')
  return bits.join(' · ')
}

/* ── 额外请求参数（**每个模型一份**，2026-11-26）──────────────────────────
   `_extraText` 是编辑框里的原文（瞬态，不落盘；buildModels 只认 extra_params），
   `_extraErr` 非空 ⇒ 这一行不能保存（宁可这次不落盘，也不要把坏 JSON 写进 llm.toml）。 */
function extraText(m) {
  if (typeof m._extraText === 'string') return m._extraText
  return stringifyExtra(m.extra_params)
}
function setExtra(m, text) {
  m._extraText = text
  const { value, error } = parseExtraObject(text)
  m._extraErr = error
  m.extra_params = error ? null : value
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
const shown = computed(() => {
  const q = String(filter.value || '').trim().toLowerCase()
  const list = rows.value
  return q ? list.filter((m) => String(m.id || '').toLowerCase().includes(q) || String(m.label || '').toLowerCase().includes(q)) : list
})
</script>

<template>
  <div class="ml">
    <!-- 工具条：添加模型的三条路都在这里（取候选 / 手填 / 能力手填），不用翻折叠块 -->
    <div class="ml-bar">
      <span class="ml-title">模型<span class="ml-cnt">{{ rows.length }} 支</span></span>
      <span class="ml-hint" title="「哪个任务用哪支」里的候选只来自这张表">链路只能从这里选</span>
      <span class="ml-spacer"></span>
      <input v-model="filter" class="ml-filter" placeholder="过滤…" aria-label="过滤本接口的模型">
      <button class="ml-mini" type="button" :disabled="fetching || !providerId"
              title="按这个接口的地址取一次候选（内置目录命中就不联网）"
              @click="doFetch">{{ fetching ? '取候选…' : '取候选' }}</button>
      <button class="ml-mini" type="button" :disabled="!providerId" title="手填一行（服务器什么都不说时的兜底）"
              @click="addManual">+ 添加模型</button>
    </div>

    <div class="ml-scroll">
      <table class="ml-tbl">
        <thead>
          <tr>
            <th class="ml-c-id">模型 id</th>
            <th class="ml-c-label">显示名</th>
            <th class="ml-c-caps">能力</th>
            <th class="ml-c-act"></th>
          </tr>
        </thead>
        <tbody>
          <template v-for="(m, i) in shown" :key="rowKey(m, i)">
          <tr>
            <td><input v-model.trim="m.id" class="ml-input" placeholder="如 deepseek-chat" spellcheck="false"></td>
            <td><input v-model="m.label" class="ml-input" placeholder="（可空，空则用 id）"></td>
            <td class="ml-caps" :title="summaryTitle(m)">{{ summary(m) }}</td>
            <td class="ml-act">
              <button class="ml-mini" :title="open[rowKey(m, i)] ? '收起手填能力' : '展开手填能力（盖过 catalog）'"
                      @click="open[rowKey(m, i)] = !open[rowKey(m, i)]">{{ open[rowKey(m, i)] ? '▾' : '▸' }}</button>
              <button class="ml-ico danger" title="从清单删除（保存后生效）" @click="removeModel(m)">
                <Icon name="trash" :size="12" />
              </button>
            </td>
          </tr>
          <tr v-if="open[rowKey(m, i)]" class="ml-ov-tr">
            <td colspan="4">
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
                  <select :value="(m.overrides && m.overrides.temperature) || ''" class="ml-sel"
                          @change="setOv(m, 'temperature', $event.target.value)">
                    <option value="">跟随能力表</option>
                    <option value="supported">支持</option>
                    <option value="unsupported">不支持（不发）</option>
                  </select>
                  <span class="ml-lbl ml-lbl2">输出字段</span>
                  <select :value="(m.overrides && m.overrides.max_output_wire) || ''" class="ml-sel"
                          @change="setOv(m, 'max_output_wire', $event.target.value)">
                    <option value="">跟随方言</option>
                    <option value="max_tokens">max_tokens</option>
                    <option value="max_completion_tokens">max_completion_tokens</option>
                  </select>
                </div>
                <div class="ml-ov-row">
                  <span class="ml-lbl">推理写法</span>
                  <select :value="(m.overrides && m.overrides.thinking_wire) || ''" class="ml-sel"
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

                <!-- 额外请求参数：**这一支模型专属**（2026-11-26 用户口径）——每支模型都能单独调 -->
                <div class="ml-ov-row">
                  <span class="ml-lbl">额外参数</span>
                  <span class="ml-muted" style="flex:1 1 auto; white-space:normal">
                    只放这一支模型专用的私有键（原样进请求体；同名键盖过全局那份）
                  </span>
                </div>
                <textarea class="mono ml-extra" rows="6" spellcheck="false"
                          :class="{ bad: !!m._extraErr }"
                          :value="extraText(m)"
                          placeholder="{ &quot;repetition_penalty&quot;: 1.05 }"
                          @input="setExtra(m, $event.target.value)"></textarea>
                <div v-if="m._extraErr" class="ml-err">额外参数：{{ m._extraErr }}</div>
              </div>
            </td>
          </tr>
          </template>
          <tr v-if="!rows.length">
            <td colspan="4" class="ml-empty">
              这一份接口还没有模型 —— 点上面「取候选」挑几支，或「+ 添加模型」手填一行。
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- 候选（**只做建议**，不落盘；失败 ≠ 空列表，三态分开说） -->
    <div v-if="remote.state !== 'idle'" class="ml-remote">
      <div class="ml-remote-head">
        <b>候选</b>
        <span v-if="remote.note" class="ml-muted" :title="remote.note">{{ remote.note }}</span>
        <span class="ml-spacer"></span>
        <input v-model="candFilter" class="ml-filter" placeholder="过滤候选…" aria-label="过滤候选">
        <button v-if="remote.state === 'error'" class="ml-mini" @click="doFetch">重试</button>
      </div>
      <div v-if="remote.state === 'loading'" class="ml-muted">取候选…</div>
      <div v-else-if="remote.state === 'error'" class="ml-err" :title="remote.note" data-ml-remote="error">
        取候选失败：{{ remote.note }}
      </div>
      <div v-else-if="!remote.models.length" class="ml-muted">服务器没返回模型（可以「+ 添加模型」手填）。</div>
      <div v-else-if="!candList().length" class="ml-muted">没有匹配「{{ candFilter }}」的模型。</div>
      <div v-else class="ml-remote-list">
        <div v-for="m in candList()" :key="m.id" class="ml-remote-row">
          <span class="ml-remote-id" :title="m.id">{{ m.id }}</span>
          <span class="ml-muted ml-remote-name" :title="m.name || ''">{{ m.name || '' }}</span>
          <span class="ml-spacer"></span>
          <span v-if="chosen(m.id)" class="ml-muted">已在清单</span>
          <button v-else class="ml-mini" title="加进清单（只写 id / 显示名 / 探到的能力）" @click="add(m.id)">加入</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.ml { display:flex; flex-direction:column; gap:6px; min-width:0; }
.ml-bar { display:flex; align-items:center; gap:6px; flex-wrap:wrap; min-width:0; }
.ml-title { font-size:12.5px; font-weight:700; }
.ml-cnt { margin-left:6px; font-size:10.5px; font-weight:400; color:var(--ink-faint); }
.ml-hint { font-size:10.5px; color:var(--ink-faint); }
.ml-spacer { flex:1 1 auto; min-width:4px; }
.ml-scroll { overflow-x:auto; min-width:0; }
.ml-tbl { width:100%; border-collapse:collapse; font-size:12px; min-width:420px; }
.ml-tbl th { text-align:left; font-weight:600; font-size:11px; color:var(--ink-dim);
  padding:4px 6px; border-bottom:1px solid var(--line-soft); white-space:nowrap; }
.ml-tbl td { padding:2px 6px; border-bottom:1px dashed var(--line-soft); vertical-align:middle; }
.ml-c-id { width:38%; }
.ml-c-label { width:26%; }
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
.ml-empty { color:var(--ink-dim); font-size:11.5px; padding:10px 6px; line-height:1.6; }
.ml-ov { padding:4px 6px 6px 18px; border-left:2px solid var(--main-soft); margin:2px 0 6px;
  display:flex; flex-direction:column; gap:2px; }
.ml-ov-row { display:flex; align-items:center; gap:8px; min-height:28px; min-width:0; flex-wrap:wrap; }
.ml-lbl { flex:none; font-size:11.5px; color:var(--ink-dim); min-width:52px; }
.ml-lbl2 { min-width:64px; }
.ml-chk { display:inline-flex; align-items:center; gap:5px; font-size:11.5px; color:var(--ink); cursor:pointer; }
.ml-chk input { margin:0; }
.ml-sel { flex:none; height:24px; padding:1px 4px; font-size:11.5px; min-width:120px; width:auto; }
.ml-lv { padding-left:60px; }
/* 每支模型一份的 JSON 编辑框：够高能看到内容，窄了也不横滚 */
.ml-extra { width:100%; min-height:96px; margin:2px 0 4px; font-size:11.5px; line-height:1.5;
  resize:vertical; }
.ml-extra.bad { border-color:var(--pink-text); }
.ml-muted { font-size:11px; color:var(--ink-faint); min-width:0; overflow:hidden;
  text-overflow:ellipsis; white-space:nowrap; }
.ml-filter { flex:0 1 140px; height:24px; padding:2px 6px; font-size:11.5px; }
.ml-remote { border:1px dashed var(--line-soft); border-radius:8px; padding:6px 8px; min-width:0; }
.ml-remote-head { display:flex; align-items:baseline; gap:8px; min-width:0; margin-bottom:4px; }
.ml-remote-list { display:flex; flex-direction:column; max-height:220px; overflow:auto; min-width:0; }
.ml-remote-row { display:flex; align-items:center; gap:8px; min-height:26px; min-width:0; }
.ml-remote-id { flex:0 1 auto; max-width:60%; min-width:0; overflow:hidden; text-overflow:ellipsis;
  white-space:nowrap; font-size:11.5px; }
.ml-remote-name { flex:1 1 auto; }
.ml-err { font-size:11.5px; color:var(--pink-text); overflow-wrap:anywhere; }
</style>
