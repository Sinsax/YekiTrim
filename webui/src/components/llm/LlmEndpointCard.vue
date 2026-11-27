<!--
  LlmEndpointCard — 「模型配置」右栏上半：**当前正在编辑的那一份接口**的连接信息。

  内容（2026-11-27 用户口径，只留这些）：显示名 / API 协议 / 地址 / 密钥 / 测试 / 设为当前 / 删除。
  ⚠️ **默认模型、能力 chip 都删掉了**：模型在下面的模型清单里挑，链路那边再决定哪个任务用哪支 ——
     这里再放一个"默认模型"就是第三个入口（用户截图圈掉的就是它）。

  ⚠️ 「测试这个接口」**不要求先保存**：草稿（地址 / 还没落盘的 key）直接进 POST /api/llm/test。
  ⚠️ 密钥**只写不读**：服务端只回 api_key_set 布尔 ⇒ 输入框永远是空的，留空 = 不修改这个键。
  ⚠️ **新建一份接口不改变 active**：别再让它顺手把全局换了。
-->
<script setup>
import { computed, ref } from 'vue'
import Icon from '../Icon.vue'
import { testConnection } from '../../api/llm'

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
  /** 'server' | 'builtin' —— 协议清单从哪来的（照实标） */
  protocolsSource: { type: String, default: '' }
})
const emit = defineEmits(['set-active', 'remove'])

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
</script>

<template>
  <div class="ep">
    <div v-if="!cur" class="muted small">
      左边还没有接口 —— 点左下「+ 新建接口」。
    </div>
    <template v-else>
      <div v-if="!isActive" class="ep-warn">
        <Icon name="alert" :size="12" />
        <span title="任务链路各自指定了模型；这里只决定默认用哪一份接口">不是当前使用的接口</span>
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
