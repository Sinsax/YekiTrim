<!--
  LlmEndpointCard — L3 五块 IA 的 **① 当前接口**。

  内容（用户口径，顺序即屏幕顺序）：接口下拉（= [llm] active）+ 名称 + base_url + 密钥（密文框 + 已设置徽标）
  + 模型选择（**一处**：候选来自模型清单 ④ 与「获取模型」的建议）+ 一行能力 chip + 「测试这个接口」。

  数据流：providers 是父层的 reactive 数组，本组件**原地改**当前那一份（与 LlmParamFields 同一条绑法）；
  只有"换哪一份"经 emit 回父层（它要跟着重算能力）。本组件**不保存**、不发 PUT ——
  保存仍走父层的 saveLlm（buildLlmPatch + PUT + 回读校验 + 错误横幅）。

  ⚠️ 「测试这个接口」**不要求先保存**：草稿（地址 / 还没落盘的 key / 手选的模型）直接进 POST /api/llm/test 的 body。
     这是配置重构 §2-⑨ 的原症状（"想试一个 key 得先落盘"），别退回"先存再测"。
  ⚠️ 密钥**只写不读**：服务端只回 api_key_set 布尔 ⇒ 输入框永远是空的，留空 = 不修改这个键。
-->
<script setup>
import { computed, ref } from 'vue'
import Icon from '../Icon.vue'
import { testConnection } from '../../api/llm'
import { inputZh } from './caps.js'

const props = defineProps({
  /** 配置真的读到了（读失败时父层会拒保存，这里只用来禁用按钮） */
  loaded: { type: Boolean, default: false },
  /** 父层 reactive 的 `form.llm.providers`（本组件原地改当前那一份） */
  providers: { type: Array, required: true },
  /** 当前在用的接口 id（= [llm] active） */
  activeId: { type: String, default: '' },
  /** 模型候选（④ 的清单 + 探针建议；可手打任何 id） */
  modelOptions: { type: Array, default: () => [] },
  /** 当前模型的能力（caps.js::normalizeCaps 归一后；null = 未知） */
  caps: { type: Object, default: null },
  /** 能力来源那行小字（内置目录 / 联网探测 / 未收录） */
  capsText: { type: String, default: '' },
  /** 能力读失败的原因（照实显示，别让用户以为"这个模型本来就没这些旋钮"） */
  capsError: { type: String, default: '' }
})
const emit = defineEmits(['update:activeId'])

/** 当前那一份（active 指错就退第一份 —— 与后端 active_provider() 同一条口径） */
const cur = computed(() => {
  const list = Array.isArray(props.providers) ? props.providers : []
  return list.find((p) => p && p.id === props.activeId) || list[0] || null
})
const keySet = computed(() => !!(cur.value && cur.value.api_key_set))
const keyPlaceholder = computed(() => (keySet.value ? '已设置（留空 = 不修改）' : 'sk-…'))
const keyTitle = computed(() => (keySet.value
  ? '已存有密钥（服务端不回明文）。留空 = 不动它；要改就直接填新的'
  : '还没存密钥。填好这里可以**直接**点「测试这个接口」，不用先保存'))

/** 行内文案预算（用户口径 ≤20 字）；细节一律进 title。 */
function shortText(s) {
  const t = String(s == null ? '' : s)
  return t.length > 20 ? t.slice(0, 19) + '…' : t
}
/** 测试结果：{ kind: ''|'busy'|'ok'|'err', text, title } —— 常驻行内，不是一闪就没的 toast */
const status = ref({ kind: '', text: '', title: '' })
const testing = ref(false)

/** 草稿测试：非空的项才进 body（空的回落服务端已存那份）。 */
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
    const bits = []
    if (r && typeof r.latency_ms === 'number') bits.push(r.latency_ms + 'ms')
    if (r && r.model) bits.push('模型 ' + r.model)
    status.value = { kind: 'ok', text: shortText('连通正常'), title: bits.join(' · ') }
  } catch (e) {
    // **不许静默**：失败原因留在行内（title 里放全文）
    status.value = { kind: 'err', text: shortText('连接失败'), title: (e && e.message) ? e.message : String(e) }
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
      还没有任何接口 —— 到下面的「高级配置 → 接口管理」新建一份（地址 / 密钥 / 模型都在这里填）。
    </div>
    <template v-else>
      <div class="ep-grid">
        <label class="ep-field">
          <span>接口</span>
          <select :value="cur.id" title="当前用的是哪一份接口（[llm] active）—— 增删/改名在「高级配置 → 接口管理」"
                  @change="emit('update:activeId', $event.target.value)">
            <option v-for="p in providers" :key="p.id" :value="p.id">{{ p.name || p.id }}</option>
          </select>
        </label>
        <label class="ep-field">
          <span>显示名</span>
          <input v-model="cur.name" type="text" placeholder="给人看的名字（如 DeepSeek 主账号）">
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
        <label class="ep-field">
          <span>模型</span>
          <input v-model.trim="cur.model" type="text" list="llm-endpoint-models" spellcheck="false"
                 placeholder="如 deepseek-chat"
                 title="可手打模型 id；下拉候选来自「模型列表」与「获取模型」的建议">
          <datalist id="llm-endpoint-models">
            <option v-for="m in modelOptions" :key="m.id" :value="m.id">{{ m.label || m.id }}</option>
          </datalist>
        </label>
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
        <span class="ep-hint">不用先保存：按上面填的（含还没落盘的密钥）发一次最小请求</span>
        <span class="ep-spacer"></span>
        <span v-if="status.text" class="ep-status" :class="status.kind" :title="status.title">{{ status.text }}</span>
      </div>
    </template>
  </div>
</template>

<style scoped>
.ep { display:flex; flex-direction:column; gap:8px; min-width:0; }
.ep-grid { display:grid; grid-template-columns:minmax(0,1fr); gap:8px 12px; }
/* 内容列够宽才两列（与设置页其他表单同一条容器查询口径：量的是内容列，不是视口） */
@container (min-width: 620px) {
  .ep-grid { grid-template-columns:repeat(2, minmax(0,1fr)); }
}
.ep-field { display:flex; flex-direction:column; gap:3px; min-width:0; }
.ep-field > span { font-size:11.5px; color:var(--ink-dim); display:flex; align-items:center; gap:6px; }
.ep-field > input, .ep-field > select { width:100%; min-width:0; }
.ep-badge { font-size:10px; line-height:1; padding:3px 5px; border-radius:999px;
  background:var(--main-soft); color:var(--pink-text); font-weight:600; }
.ep-caps { display:flex; align-items:center; gap:6px; flex-wrap:wrap; min-width:0; }
.ep-chip { font-size:10.5px; line-height:1; padding:4px 6px; border-radius:999px;
  background:var(--bg-soft); border:1px solid var(--line-soft); color:var(--ink-dim); white-space:nowrap; }
.ep-actions { display:flex; align-items:center; gap:8px; flex-wrap:wrap; min-width:0; }
.ep-hint { font-size:11px; color:var(--ink-faint); min-width:0; overflow-wrap:anywhere; }
.ep-spacer { flex:1 1 auto; min-width:4px; }
.ep-status { font-size:11.5px; font-weight:600; }
.ep-status.ok { color:var(--green-deep); }
.ep-status.err { color:var(--pink-text); }
.ep-status.busy { color:var(--ink-faint); }
.ep-err { font-size:11.5px; color:var(--pink-text); }
</style>
