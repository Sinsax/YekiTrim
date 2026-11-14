<template>
  <div class="lqs">
    <div class="lqs-head">
      <b class="t m-0">快速配置</b>
      <span class="muted small"
            :title="providerMode
              ? '只改当前生效的那一份；多份接口 / 协议 / 参数 / 提示词在下面的「高级配置」里'
              : '保存后会顺带测一次连通性'">填地址与密钥，保存后自动测一次</span>
    </div>

    <div class="lqs-grid">
      <label class="lqs-field">
        <span>服务商</span>
        <select v-model="presetId" :title="'选一个常用服务商，会填入常用 API 地址与模型提示（只是提示，不是能力表）'"
                @change="onPresetChange($event.target.value)">
          <option v-for="p in LLM_PRESETS" :key="p.id" :value="p.id">{{ p.label }}</option>
        </select>
      </label>
      <label class="lqs-field">
        <span>API 地址</span>
        <input v-model.trim="formBaseUrl" type="text" name="llm-base-url" autocomplete="off"
               placeholder="https://api.deepseek.com/v1" spellcheck="false">
      </label>
      <label class="lqs-field">
        <span>API 密钥</span>
        <input v-model="formKey" type="password" name="llm-api-key" autocomplete="new-password"
               :placeholder="keyPlaceholder" :title="keyTitle">
      </label>
      <label class="lqs-field">
        <span>模型</span>
        <input v-model.trim="formModel" type="text" list="llm-quick-models" placeholder="如 deepseek-chat" spellcheck="false"
               title="可手打模型 id；下拉候选来自「获取模型」与预设提示">
        <datalist id="llm-quick-models">
          <option v-for="m in suggestions" :key="m.id" :value="m.id">{{ m.name }}</option>
        </datalist>
      </label>
    </div>

    <div class="lqs-actions">
      <button class="mini-btn" type="button" :disabled="busy || !loaded"
              title="按上面的 API 地址取模型清单：内置目录命中就不联网，没命中才真探一次 /models"
              @click="doFetch">{{ fetching ? '获取中…' : '获取模型' }}</button>
      <button class="btn btn-primary" type="button" :disabled="busy || !loaded"
              title="先保存（沿用这一页的保存链路），保存成功后自动测一次连通性"
              @click="saveAndTest">{{ saving ? '保存并测试…' : '保存并测试' }}</button>
      <span v-if="fetchNote" class="muted small lqs-hint" :title="fetchTitle">{{ fetchNote }}</span>
      <span class="lqs-spacer"></span>
      <span v-if="status.text" class="lqs-status" :class="status.kind" :title="status.title">{{ status.text }}</span>
    </div>
    <div v-if="!loaded" class="muted small">配置未加载，暂时不能保存</div>
  </div>
</template>

<script setup>
// LlmQuickSetup — AI 模型的四字段快配：服务商 / API 地址 / API 密钥 / 模型 + [获取模型] + [保存并测试]。
//
// 两种后端形态都支持（由父层按 `Array.isArray(llm.providers)` 给 `providerMode`）：
//  · provider 模式：改 `form.llm.providers` 里**当前生效的那一份**（没有就新建），并设为 active；
//  · legacy 模式：改顶层 `[llm] base_url / api_key / model`。
// 保存**不在这里重写**：父层把既有的保存链路（buildLlmPatch + PUT + 回读 + 错误横幅）作为
// `persist` 传进来。本组件只负责「填 → 落盘 → 测一次」并显示结果（保存失败/测试失败都常驻行内）。
//
// ⚠️ 密钥只写不读：服务端不回明文，输入框永远空着；**留空 = 不修改**（父层保存时省略这个键）。
// ⚠️ 静态预设（presets.js）只提供 base_url / 模型提示，**不是能力表** —— 别在这儿写能力。
import { computed, ref, watch } from 'vue'
import { fetchModels, testConnection, MODEL_SOURCE_ZH } from '../../api/llm'
import { LLM_PRESETS, getPreset, presetForUrl } from './presets'

const props = defineProps({
  /** 配置是否真的读到了（读失败时保存会被父层拒绝，这里也要禁用） */
  loaded: { type: Boolean, default: false },
  /** true = 后端回了 `providers` 数组（多配置）；false = legacy 顶层字段 */
  providerMode: { type: Boolean, default: false },
  /** 父层 reactive 的 `form.llm.providers`（provider 模式下原地改） */
  providers: { type: Array, default: null },
  /** 当前生效的 provider id（`[llm] active`） */
  activeId: { type: String, default: '' },
  /** legacy 模式的两个字段（provider 模式下不用） */
  baseUrl: { type: String, default: '' },
  model: { type: String, default: '' },
  /** legacy 模式有没有已存密钥（只影响占位符文案） */
  keySet: { type: Boolean, default: false },
  /** async () => ({ ok: boolean, error?: string })：父层跑既有保存链路并回结果 */
  persist: { type: Function, required: true }
})

const emit = defineEmits(['update:activeId', 'update:baseUrl', 'update:model', 'update:apiKey'])

/** 行内文案预算（用户口径 ≤20 字）；细节一律进 title。 */
const INLINE_MAX = 20
function shortText(s) {
  const t = String(s == null ? '' : s)
  return t.length > INLINE_MAX ? t.slice(0, INLINE_MAX - 1) + '…' : t
}

const presetId = ref('custom')
const formBaseUrl = ref('')
const formModel = ref('')
const formKey = ref('')
/** [获取模型] 拿回来的候选（不落盘，只做 datalist） */
const fetched = ref([])
const fetching = ref(false)
const saving = ref(false)
const fetchNote = ref('')
const fetchTitle = ref('')
/** 保存/测试的行内结果：{ kind: '' | 'busy' | 'ok' | 'err', text, title } */
const status = ref({ kind: '', text: '', title: '' })

const busy = computed(() => fetching.value || saving.value)
const preset = computed(() => getPreset(presetId.value))
const currentProvider = computed(() => {
  if (!props.providerMode) return null
  const list = Array.isArray(props.providers) ? props.providers : []
  return list.find((p) => p && p.id === props.activeId) || list[0] || null
})
/** 已存密钥：provider 模式看当前那份的 `api_key_set`，legacy 模式看父层给的位 */
const keySetNow = computed(() => (props.providerMode
  ? !!(currentProvider.value && currentProvider.value.api_key_set)
  : props.keySet))
const keyPlaceholder = computed(() => (keySetNow.value ? '已设置（留空 = 不修改）' : 'sk-…'))
const keyTitle = computed(() => (keySetNow.value
  ? '已存有密钥（服务端不回明文）。留空 = 不动它；要改就直接填新的'
  : '还没存密钥；填一个再「获取模型」'))

/** datalist 候选 = 探到的模型 + 预设里的模型提示（去重；可手打任何 id）。 */
const suggestions = computed(() => {
  const out = []
  const seen = new Set()
  for (const m of fetched.value) {
    const id = String((m && m.id) || '').trim()
    if (!id || seen.has(id)) continue
    seen.add(id)
    out.push({ id, name: (m && m.name) || '' })
  }
  const hint = String((preset.value && preset.value.model) || '').trim()
  if (hint && !seen.has(hint)) out.push({ id: hint, name: preset.value.label || '' })
  return out
})

function setStatus(kind, text, title) {
  status.value = { kind: String(kind || ''), text: shortText(text), title: String(title || '') }
}
function setFetchNote(text, title) {
  fetchNote.value = shortText(text)
  fetchTitle.value = String(title || '')
}

const urlOf = (p) => String((p && p.base_url) || '').trim()
const modelOf = (p) => String((p && p.model) || '').trim()

/** 把父层当前值同步进草稿（预填当前值；密钥永远空）。 */
let lastTarget = null
function syncFromProps() {
  const p = currentProvider.value
  // 换了目标（切了 active provider / 切了模式）⇒ 上一个目标的候选与结果都作废。
  // ⚠️ 同一目标内的 props 更新（保存后的回读）**不**清结果 —— 否则刚写的"测试通过"会被自己擦掉。
  const target = props.providerMode ? 'p:' + String((p && p.id) || '') : 'legacy'
  const switched = lastTarget !== null && lastTarget !== target
  lastTarget = target
  formBaseUrl.value = props.providerMode ? urlOf(p) : String(props.baseUrl || '').trim()
  formModel.value = props.providerMode ? modelOf(p) : String(props.model || '').trim()
  formKey.value = ''
  presetId.value = presetForUrl(formBaseUrl.value)
  if (switched) {
    fetched.value = []
    setFetchNote('', '')
    setStatus('', '', '')
  }
}
watch(
  () => [
    props.providerMode,
    props.activeId,
    props.baseUrl,
    props.model,
    currentProvider.value && currentProvider.value.id,
    currentProvider.value && currentProvider.value.base_url,
    currentProvider.value && currentProvider.value.model
  ],
  syncFromProps,
  { immediate: true }
)

/** 换服务商 = 明确动作：填入该家的常用地址与模型提示（自定义什么都不动）。
 *  ⚠️ 值从 **event.target.value** 读（不依赖 v-model 与 @change 的回调先后）——
 *     同一事件上两个处理器谁先跑，Vue 没承诺顺序，读错就会"选了服务商但填的是上一家"。 */
function onPresetChange(id) {
  presetId.value = String(id || 'custom')
  const p = getPreset(presetId.value)
  if (!p) return
  if (p.base_url) formBaseUrl.value = p.base_url
  if (p.model) formModel.value = p.model
  fetched.value = []
  setFetchNote('', '')
  setStatus('', '', '')
}

async function doFetch() {
  if (busy.value || !props.loaded) return
  const base = formBaseUrl.value.trim()
  if (!base) {
    setFetchNote('先填 API 地址', '「API 地址」为空，没法取模型清单')
    return
  }
  fetching.value = true
  setFetchNote('获取中…', '')
  try {
    // ⚠️ provider 模式必须带 id（后端按这份配置自己的已存 key 取）；新配置还没保存时别带
    const pid = props.providerMode && currentProvider.value ? currentProvider.value.id : undefined
    const r = await fetchModels(base, formKey.value, pid)
    fetched.value = r.models
    const src = MODEL_SOURCE_ZH[r.source] || r.source || ''
    const kindZh = r.source === 'probe' ? '联网探测' : '内置目录'
    setFetchNote('候选 ' + r.models.length + ' 个 · ' + kindZh, ['来源：' + src, r.note].filter(Boolean).join(' · '))
    if (!formModel.value && r.models.length) formModel.value = r.models[0].id
  } catch (e) {
    // **失败 ≠ 空列表**：原因写在 title 里，行内只留一句
    setFetchNote('获取模型失败', (e && e.message) ? e.message : String(e))
  } finally {
    fetching.value = false
  }
}

/** legacy：把草稿交给父层的顶层字段（密钥留空 = 不发，父层保存时省略）。 */
function applyLegacy() {
  emit('update:baseUrl', formBaseUrl.value.trim())
  emit('update:model', formModel.value.trim())
  const k = formKey.value.trim()
  if (k) emit('update:apiKey', k)
}

function newProviderId(list) {
  const used = new Set(list.map((p) => String((p && p.id) || '').trim()))
  const stem = 'p-' + (preset.value && preset.value.id !== 'custom' ? preset.value.id : 'llm')
  if (!used.has(stem)) return stem
  let n = 2
  while (used.has(stem + '-' + n)) n++
  return stem + '-' + n
}

/**
 * provider：新建或更新**当前生效的那一份**，字段与 `LlmProviders.vue::blankProvider` 同形状
 * （params / paramsOrig / models 缺一不可，否则父层的 buildProviderPatch 会丢参数面）。
 */
function applyProvider() {
  const list = Array.isArray(props.providers) ? props.providers : null
  if (!list) return null
  let p = list.find((x) => x && x.id === props.activeId) || list[0] || null
  if (!p) {
    p = {
      id: newProviderId(list),
      name: '',
      api: 'openai-chat',
      base_url: '',
      api_key: '',
      api_key_set: false,
      model: '',
      // ⚠️ 不给 reasoning 空串（P0：空串进 llm.toml ⇒ 整份配置回退默认）；键缺席即可
      params: { temperature: 0.3, max_tokens: 65536, json_mode: false },
      // 新的一份没有"原值"快照 ⇒ 保存时这些键都算改过（会写实）
      paramsOrig: null,
      models: []
    }
    list.push(p)
    // 取回数组里的**响应式代理**再改：直接改 push 进去的裸对象不会触发父层的 watch/render
    p = list[list.length - 1]
  }
  const pst = preset.value
  p.name = (pst && pst.id !== 'custom') ? pst.label : (p.name || '自定义配置')
  p.base_url = formBaseUrl.value.trim()
  p.api = 'openai-chat'
  p.model = formModel.value.trim()
  const k = formKey.value.trim()
  if (k) p.api_key = k
  // 选中的模型登记进这一份的目录（与 LlmProviders 的「加入」同口径；已登记就不重复）
  const mid = p.model
  if (mid && !(p.models || []).some((m) => m && String(m.id || '').trim() === mid)) {
    p.models = p.models || []
    p.models.push({ id: mid, name: mid, context_window: null, max_output: null, input: ['text'], reasoning: null, levels: [] })
  }
  emit('update:activeId', p.id)
  return p.id
}

async function saveAndTest() {
  if (busy.value || !props.loaded) return
  saving.value = true
  setStatus('busy', '保存中…', '')
  let pid
  try {
    if (props.providerMode) {
      pid = applyProvider()
      if (!pid) { setStatus('err', '保存失败', '这份配置不是多接口形态，无法新建接口'); return }
    } else {
      applyLegacy()
      pid = undefined
    }
    // ⚠️ 先保存、再测试：测试读的是**服务端已存**的配置（草稿没落盘就测 = 测上一次的）
    const r = await props.persist()
    if (!r || r.ok !== true) {
      setStatus('err', '保存失败', (r && r.error) || '保存没有返回结果')
      return
    }
    setStatus('busy', '已保存 · 测试中…', '')
    try {
      const t = await testConnection(pid)
      const bits = ['连通正常']
      if (t && typeof t.latency_ms === 'number') bits.push(t.latency_ms + 'ms')
      if (t && t.model) bits.push('模型 ' + t.model)
      setStatus('ok', '已保存 · 测试通过', bits.join(' · '))
    } catch (e) {
      setStatus('err', '已保存 · 测试失败', (e && e.message) ? e.message : String(e))
    }
  } catch (e) {
    setStatus('err', '保存失败', (e && e.message) ? e.message : String(e))
  } finally {
    saving.value = false
  }
}
</script>

<style scoped>
.lqs { display:flex; flex-direction:column; gap:8px; min-width:0; }
.lqs-head { display:flex; align-items:baseline; gap:8px; flex-wrap:wrap; min-width:0; }
.lqs-head b { font-size:13px; }
.lqs-grid { display:grid; grid-template-columns:minmax(0,1fr); gap:8px 12px; }
/* 内容列（.settings-block，container-type:inline-size）够宽才两列 —— 与设置页其他表单同一条容器查询口径 */
@container (min-width: 620px) {
  .lqs-grid { grid-template-columns:repeat(2, minmax(0,1fr)); }
}
.lqs-field { display:flex; flex-direction:column; gap:3px; min-width:0; }
.lqs-field > span { font-size:11.5px; color:var(--ink-dim); }
.lqs-field > input, .lqs-field > select { width:100%; min-width:0; }
.lqs-actions { display:flex; align-items:center; gap:8px; flex-wrap:wrap; min-width:0; }
.lqs-hint { min-width:0; overflow-wrap:anywhere; }
.lqs-spacer { flex:1 1 auto; min-width:4px; }
.lqs-status { font-size:11.5px; font-weight:600; }
.lqs-status.ok { color:var(--green-deep); }
.lqs-status.err { color:var(--pink-text); }
.lqs-status.busy { color:var(--ink-faint); }
</style>
