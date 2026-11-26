<!--
  LlmEndpointCard — 「模型配置」右栏上半：**当前正在编辑的那一份接口**的连接信息。
  （原来叫「① 当前接口」，是 active 决定的；2026-11-26 起与"哪一份生效"解耦：
   providerId = 正在编辑，activeId = 当前使用，两者可以不同。）

  内容：显示名 / API 协议 / 地址 / 密钥（密文框 + 已设置徽标）/ 默认模型（combobox）/ 能力 chip / 测试 / 设为当前 / 删除。

  数据流：providers 是父层的 reactive 数组，本组件**原地改**当前那一份；只有"换哪一份"经 emit 回父层。
  本组件**不保存**、不发 PUT —— 保存仍走父层的 saveLlm。

  ⚠️ 「测试这个接口」**不要求先保存**：草稿（地址 / 还没落盘的 key / 手选的模型）直接进 POST /api/llm/test。
  ⚠️ 密钥**只写不读**：服务端只回 api_key_set 布尔 ⇒ 输入框永远是空的，留空 = 不修改这个键。
  ⚠️ **新建一份接口不改变 active**（父层的 addProvider 不碰它）：以前新建顺手把全局换了，
     "加个备用接口"会把所有没单独指定的任务一起切过去（真踩过）。
-->
<script setup>
import { computed, ref } from 'vue'
import Icon from '../Icon.vue'
import { fetchModels, testConnection } from '../../api/llm'
import { inputZh } from './caps.js'
import LlmModelCombobox from './LlmModelCombobox.vue'

const props = defineProps({
  /** 配置真的读到了（读失败时父层会拒保存，这里只用来禁用按钮） */
  loaded: { type: Boolean, default: false },
  /** 父层 reactive 的 `form.llm.providers`（本组件原地改 providerId 那一份） */
  providers: { type: Array, required: true },
  /** **正在编辑**哪一份（本地状态，不是 [llm] active） */
  providerId: { type: String, default: '' },
  /** 当前生效的是哪一份（= [llm] active） */
  activeId: { type: String, default: '' },
  /** API 协议清单（后端 /config/llm/protocols；拿不到时父层给内置清单） */
  protocols: { type: Array, default: () => [] },
  /** 'server' | 'builtin' —— 协议清单从哪来的（照实标，别让用户以为内置那份就是后端那份） */
  protocolsSource: { type: String, default: '' },
  /** 模型候选：[{ id, label, inList }]（inList=false = 探测到但还没进清单） */
  modelOptions: { type: Array, default: () => [] },
  /** 当前模型的能力（caps.js::normalizeCaps 归一后；null = 未知） */
  caps: { type: Object, default: null },
  /** 能力来源那行小字（内置目录 / 联网探测 / 未收录） */
  capsText: { type: String, default: '' },
  /** 能力读失败的原因（照实显示，别让用户以为"这个模型本来就没这些旋钮"） */
  capsError: { type: String, default: '' }
})
const emit = defineEmits(['pick-model', 'probe', 'set-active', 'remove'])

/** 正在编辑的那一份（认不出就退第一份） */
const cur = computed(() => {
  const list = Array.isArray(props.providers) ? props.providers : []
  return list.find((p) => p && p.id === props.providerId) || list[0] || null
})
const isActive = computed(() => !!cur.value && cur.value.id === props.activeId)
const keySet = computed(() => !!(cur.value && cur.value.api_key_set))
const keyPlaceholder = computed(() => (keySet.value ? '已设置（留空 = 不修改）' : 'sk-…'))
const keyTitle = computed(() => (keySet.value
  ? '已存有密钥（服务端不回明文）。留空 = 不动它；要改就直接填新的'
  : '还没存密钥。填好这里可以直接点「测试这个接口」，不用先保存'))

/** 行内文案预算（用户口径 ≤20 字）；细节一律进 title。 */
function shortText(s) {
  const t = String(s == null ? '' : s)
  return t.length > 20 ? t.slice(0, 19) + '…' : t
}

/* ── 默认模型：两处共用的 combobox（聚焦列全部；选中"未加入"或手打新 id 都 emit 给父层写进清单）── */
function onPickModel(m) {
  if (!cur.value || !m) return
  emit('pick-model', { providerId: cur.value.id, model: m.id, inList: m.inList === true })
}
function onCommitModel(v) {
  if (!cur.value || !v) return
  emit('pick-model', { providerId: cur.value.id, model: v, inList: false })
}

/**
 * 「取候选」：按上面填的地址/密钥取一次模型候选（内置目录命中就**不联网**）。
 * 取到的候选同时交给右栏的模型清单（父层 onProbeCaps → probeModels），那边也能「加入」。
 */
const fetching = ref(false)
const fetchState = ref({ text: '', err: '', title: '' })
async function doFetchModels() {
  const p = cur.value
  if (!p || fetching.value) return
  const base = String(p.base_url || '').trim()
  if (!base) { fetchState.value = { text: '先填 API 地址', err: '', title: '' }; return }
  fetching.value = true
  fetchState.value = { text: '取候选…', err: '', title: '' }
  try {
    const r = await fetchModels(base, p.api_key, p.id)
    const models = Array.isArray(r.models) ? r.models : []
    emit('probe', { providerId: p.id, models })
    fetchState.value = models.length
      ? { text: '候选 ' + models.length + ' 支', err: '', title: r.note || '' }
      : { text: '', err: '服务器没返回模型（可手打 id）', title: r.note || '' }
  } catch (e) {
    const full = (e && e.message) ? e.message : String(e)
    fetchState.value = { text: '', err: '取候选失败', title: full }
  } finally {
    fetching.value = false
  }
}

/** 测试结果：{ kind: ''|'busy'|'ok'|'err', text, title } —— 常驻行内，不是一闪就没的 toast */
const status = ref({ kind: '', text: '', title: '' })
const testing = ref(false)

async function doTest() {
  if (testing.value || !cur.value) return
  const p = cur.value
  testing.value = true
  status.value = { kind: 'busy', text: '测试中…', title: '' }
  try {
    const r = await testConnection({
      provider: p.id,
      base_url: p.base_url,
      api_key: p.api_key,
      model: p.model
    })
    const ms = r && typeof r.latency_ms === 'number' ? r.latency_ms + 'ms' : ''
    const bits = []
    if (ms) bits.push(ms)
    if (r && r.model) bits.push('模型 ' + r.model)
    if (p.base_url) bits.push(p.base_url)
    status.value = {
      kind: 'ok',
      text: shortText(ms ? '连通正常 · ' + ms : '连通正常'),
      title: bits.join(' · ')
    }
  } catch (e) {
    const full = (e && e.message) ? e.message : String(e)
    const brief = full.split(/[；;\n]/)[0]
    status.value = { kind: 'err', text: shortText('失败 · ' + brief), title: full }
  } finally {
    testing.value = false
  }
}

/** 能力 chip：只列**这条接口当前模型**真有的维度；未知就说未知，不猜。 */
const chips = computed(() => {
  const c = props.caps
  if (!c) return []
  const out = []
  out.push('输入 ' + inputZh(c.input))
  out.push(c.levels && c.levels.length ? ('推理 ' + c.levels.length + ' 档') : '无推理档')
  out.push(c.temperature === 'unsupported' ? '温度 不支持'
    : (c.temperature === 'unknown' ? '温度 未声明' : '温度 支持'))
  if (c.maxOutput) out.push('上限 ' + c.maxOutput)
  if (c.jsonMode === true) out.push('JSON')
  return out
})
</script>

<template>
  <div class="ep">
    <div v-if="!cur" class="muted small">
      左边还没有接口 —— 点左下「+ 新建接口」。
    </div>
    <template v-else>
      <div v-if="!isActive" class="ep-warn">
        <Icon name="alert" :size="12" />
        <span>这一份不是当前使用的接口 —— 没单独指定的任务仍用另一份</span>
      </div>

      <div class="ep-grid">
        <label class="ep-field">
          <span>显示名</span>
          <input v-model="cur.name" type="text" placeholder="给人看的名字（如 DeepSeek 主账号）">
        </label>
        <label class="ep-field">
          <span>API 协议</span>
          <select :value="cur.protocol || (protocols[0] && protocols[0].id) || ''"
                  :title="'请求怎么发（协议注册表）；一般不用改。清单来源：' + (protocolsSource === 'server' ? '后端' : '内置')"
                  @change="cur.protocol = $event.target.value">
            <option v-for="o in protocols" :key="o.id" :value="o.id">{{ o.label || o.id }}</option>
            <option v-if="cur.protocol && !protocols.some((o) => o.id === cur.protocol)" :value="cur.protocol">
              {{ cur.protocol }}
            </option>
          </select>
        </label>
        <label class="ep-field">
          <span>API 地址</span>
          <input v-model.trim="cur.base_url" type="text" autocomplete="off" spellcheck="false"
                 placeholder="https://api.deepseek.com/v1">
        </label>
        <label class="ep-field">
          <span>API 密钥 <span v-if="keySet" class="ep-badge" title="已存有密钥（服务端不回明文）">已设置</span></span>
          <input v-model="cur.api_key" type="password" autocomplete="new-password"
                 :placeholder="keyPlaceholder" :title="keyTitle">
        </label>
        <div class="ep-field ep-field-model">
          <span>默认模型
            <span class="ep-sub" title="没在「哪个任务用哪支」里单独指定模型的链路，都用这一支">其余任务默认用它</span>
          </span>
          <div class="ep-model-row">
            <LlmModelCombobox v-model="cur.model" :options="modelOptions" aria-label="默认模型"
                              placeholder="如 deepseek-chat（可手打，也可从下拉里挑）"
                              @pick="onPickModel" @commit="onCommitModel" />
            <button class="ep-mini" type="button" :disabled="fetching"
                    title="按上面填的地址取一次候选（内置目录命中就不联网）"
                    @click="doFetchModels">
              {{ fetching ? '取候选…' : '取候选' }}
            </button>
            <span v-if="fetchState.text" class="ep-note">{{ fetchState.text }}</span>
            <span v-else-if="fetchState.err" class="ep-note err" :title="fetchState.title">{{ fetchState.err }}</span>
          </div>
        </div>
      </div>

      <div class="ep-caps">
        <span v-for="c in chips" :key="c" class="ep-chip">{{ c }}</span>
        <span v-if="!chips.length && capsError" class="ep-err" :title="capsError">能力读取失败</span>
        <span v-else-if="!chips.length" class="muted small">能力未读到（填好地址与模型后自动查）</span>
        <span v-else-if="capsText" class="muted small" :title="'能力来源：' + capsText">{{ capsText }}</span>
      </div>

      <div class="ep-actions">
        <button class="btn" type="button" :disabled="testing || !loaded" @click="doTest">
          <Icon name="play" :size="12" /> {{ testing ? '测试中…' : '测试这个接口' }}
        </button>
        <span v-if="status.text" class="ep-status" :class="status.kind" :title="status.title">{{ status.text }}</span>
        <span class="ep-spacer"></span>
        <button v-if="!isActive" class="ep-mini" type="button" title="让没单独指定的任务都用这一份"
                @click="emit('set-active', cur.id)">设为当前使用</button>
        <button class="ep-mini danger" type="button" title="删除这份接口（它的模型会一并摘掉，保存后生效）"
                @click="emit('remove', cur.id)">删除</button>
      </div>
    </template>
  </div>
</template>

<style scoped>
.ep { display:flex; flex-direction:column; gap:8px; min-width:0; }
/* 非当前接口：一条琥珀色提示，别让用户以为改了地址就生效 */
.ep-warn { display:flex; align-items:center; gap:6px; font-size:11px; line-height:1.4;
  padding:4px 8px; border-radius:var(--r-sm); background:var(--warn-soft, var(--bg-soft));
  color:var(--warn-text, var(--ink-dim)); border:1px solid var(--warn-line, var(--line-soft)); }
.ep-grid { display:grid; grid-template-columns:minmax(0,1fr); gap:8px 12px; }
/* 内容列够宽才两列（与设置页其他表单同一条容器查询口径：量的是内容列，不是视口） */
@container (min-width: 620px) {
  .ep-grid { grid-template-columns:repeat(2, minmax(0,1fr)); }
  /* 模型那格跨两列：候选列表要横向空间，挤在半栏里 id 会被截断 */
  .ep-grid .ep-field-model { grid-column:1 / -1; }
}
.ep-field { display:flex; flex-direction:column; gap:3px; min-width:0; }
.ep-field > span { font-size:11.5px; color:var(--ink-dim); display:flex; align-items:center; gap:6px; }
.ep-field > input, .ep-field > select { width:100%; min-width:0; }
.ep-sub { font-size:10.5px; color:var(--ink-faint); font-weight:400; }
.ep-badge { font-size:10px; line-height:1; padding:3px 5px; border-radius:999px;
  background:var(--main-soft); color:var(--pink-text); font-weight:600; }
.ep-model-row { display:flex; align-items:center; gap:6px; min-width:0; }
.ep-caps { display:flex; align-items:center; gap:6px; flex-wrap:wrap; min-width:0; }
.ep-chip { font-size:10.5px; line-height:1; padding:4px 6px; border-radius:999px;
  background:var(--bg-soft); border:1px solid var(--line-soft); color:var(--ink-dim); white-space:nowrap; }
.ep-actions { display:flex; align-items:center; gap:8px; flex-wrap:wrap; min-width:0; }
.ep-spacer { flex:1 1 auto; min-width:4px; }
.ep-mini { flex:none; height:24px; padding:0 8px; font-size:11.5px; border-radius:var(--r-sm);
  border:1px solid var(--line); background:var(--card); color:var(--ink-dim); cursor:pointer; }
.ep-mini:hover { border-color:var(--main); color:var(--main); }
.ep-mini.danger:hover { border-color:var(--pink-text); color:var(--pink-text); }
.ep-mini:disabled { opacity:.55; cursor:not-allowed; }
.ep-note { font-size:10.5px; color:var(--ink-faint); }
.ep-note.err { color:var(--pink-text); }
.ep-status { font-size:11.5px; font-weight:600; }
.ep-status.ok { color:var(--green-deep); }
.ep-status.err { color:var(--pink-text); }
.ep-status.busy { color:var(--ink-faint); }
.ep-err { font-size:11.5px; color:var(--pink-text); }
</style>
