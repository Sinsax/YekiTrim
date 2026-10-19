<script setup>
// LlmProviders — **provider 列表 + 展开编辑器**（照 DSH 的层级：列表 → 只展开一份 → 模型行 → 档位行）。
//
// 数据：父层 reactive 的 `providers` 数组 + `activeId`（= `[llm] active`）。本组件**原地改**它们
// （与 LlmParamFields 同一条绑法）；只有"切换 active"经 emit 回父层。
//
// ⚠️ **R4**：本组件只在**后端回了 `providers` 字段**时由父层渲染（`Array.isArray(llm_config.providers)`）。
//    没有那个字段 ⇒ 父层继续用老的单配置界面，本文件一行都不参与。
// ⚠️ **紧凑口径**（用户：「精简紧凑一点」）：行高 ≤32px、操作按钮图标化 + `title`、
//    次要项（显示名/地址/协议/上下文窗口…）全收进折叠里；同一句解释只说一次。
import { reactive, ref } from 'vue'
import Icon from '../Icon.vue'
import { confirmDialog } from '../../stores/dialog'
import { toast } from '../../stores/toast'
import { MODEL_SOURCE_ZH, fetchModels } from '../../api/llm'
import { LEVEL_VOCAB, LEVEL_ZH } from './caps.js'
import { protocolLabel } from './protocols.js'

const props = defineProps({
  /** 父层的 reactive 数组：`form.llm.providers`（本组件原地增删改） */
  providers: { type: Array, required: true },
  /** 当前在用的 provider id（`[llm] active`） */
  activeId: { type: String, default: '' },
  /** API 协议清单（后端 `/config/llm/protocols`；拿不到时父层给内置清单） */
  protocols: { type: Array, default: () => [] },
  /** 'server' | 'builtin' —— 界面上照实标清单来源 */
  protocolsSource: { type: String, default: '' }
})
const emit = defineEmits(['update:activeId'])

/** **同一时间只展开一份**；再点同一个 = 折叠 */
const expandedId = ref('')
/** provider.id → 「自定义设置」是否展开（默认折叠 = 次要项默认看不见） */
const subOpen = reactive({})
/** provider.id → 正在获取模型 / 状态行 */
const busy = reactive({})
const note = reactive({})
/** `${pid}/${mid}` → 模型行 / 推理等级是否展开 */
const openModel = reactive({})
const openLevels = reactive({})

const key = (pid, mid) => String(pid) + '/' + String(mid)
const canExpand = (p) => !!p && !!p.id

function toggleEdit(id) {
  if (!id) return
  expandedId.value = expandedId.value === id ? '' : id
}
function setActive(id) {
  if (id) emit('update:activeId', id)
}
function blankProvider(n) {
  return {
    id: 'p' + (n + 1) + '-' + Math.random().toString(36).slice(2, 6),
    name: '新配置 ' + (n + 1),
    api: (props.protocols[0] && props.protocols[0].id) || 'openai-chat',
    base_url: '',
    api_key: '',
    api_key_set: false,
    model: '',
    params: { temperature: 0.3, max_tokens: 65536, reasoning: '', json_mode: false },
    models: []
  }
}
function addProvider() {
  const p = blankProvider(props.providers.length)
  props.providers.push(p)
  expandedId.value = p.id
  if (!props.activeId) emit('update:activeId', p.id)
}
async function removeProvider(p) {
  const ok = await confirmDialog('删除配置「' + (p.name || p.id) + '」？（保存后生效）', { title: '删除配置', danger: true })
  if (!ok) return
  const i = props.providers.indexOf(p)
  if (i >= 0) props.providers.splice(i, 1)
  if (expandedId.value === p.id) expandedId.value = ''
  if (props.activeId === p.id && props.providers.length) emit('update:activeId', props.providers[0].id)
}
function addModel(p) {
  p.models = p.models || []
  const m = {
    id: '', name: '', context_window: null, max_output: null,
    input: null, reasoning: null, levels: []
  }
  p.models.push(m)
  openModel[key(p.id, p.models.length - 1)] = true
}
async function removeModel(p, i) {
  const m = p.models[i]
  const ok = await confirmDialog('从模型目录里删除「' + (m.id || '(未命名)') + '」？（保存后生效）', { title: '删除模型', danger: true })
  if (!ok) return
  p.models.splice(i, 1)
  if (p.model === m.id) p.model = p.models.length ? p.models[0].id : ''
}
/** 「恢复默认」= 把各模型**手填的能力**清空（回到"由能力表 / 探针决定"）。 */
function restoreDefaults(p) {
  for (const m of (p.models || [])) {
    m.context_window = null
    m.max_output = null
    m.input = null
    m.reasoning = null
    m.levels = []
  }
  note[p.id] = '已清空手填能力（回到能力表决定）'
}
/** 「获取可用模型」= 走既有探针接口（表命中不联网 / 表外才探），把新 id 并进目录 */
async function fetchAvailable(p) {
  const base = String(p.base_url || '').trim()
  if (!base) { note[p.id] = '先填 API 地址（「自定义设置」里）'; subOpen[p.id] = true; return }
  busy[p.id] = true
  note[p.id] = ''
  try {
    const r = await fetchModels(base, p.api_key)
    p.models = p.models || []
    const have = new Set(p.models.map((m) => String(m.id || '').trim()).filter(Boolean))
    let added = 0
    for (const m of r.models) {
      if (have.has(m.id)) continue
      p.models.push({ id: m.id, name: m.name || '', context_window: null, max_output: null, input: null, reasoning: null, levels: [] })
      added += 1
    }
    note[p.id] = '来源：' + (MODEL_SOURCE_ZH[r.source] || r.source) + ' · 新增 ' + added + ' 条'
      + (r.note ? ' · ' + r.note : '') + (r.models.length ? '' : '（没拿到模型，可手填）')
  } catch (e) {
    note[p.id] = '获取失败：' + ((e && e.message) || e)   // **不许静默**
  } finally {
    busy[p.id] = false
  }
}
function addLevel(m) {
  m.levels = m.levels || []
  const used = new Set(m.levels.map((l) => l.level))
  const next = LEVEL_VOCAB.find((v) => !used.has(v)) || ''
  m.levels.push({ level: next, wire_value: next })
}
function removeLevel(m, i) {
  m.levels.splice(i, 1)
}
function inputOf(m) { return Array.isArray(m.input) ? m.input : [] }
function toggleInput(m, kind, on) {
  const cur = new Set(inputOf(m))
  if (on) cur.add(kind)
  else cur.delete(kind)
  m.input = ['text', 'image'].filter((k) => cur.has(k))
}
function setLevel(l, v) {
  l.level = v
  // API 参数值默认跟档位同名（用户随后可改）——只在新值与旧值相同（还是自动的）时才跟
  if (!l.wire_value || LEVEL_VOCAB.includes(l.wire_value)) l.wire_value = v
}
</script>

<template>
  <div class="pv">
    <!-- provider 列表：一行一份；同一时间只展开一份 -->
    <div v-for="p in providers" :key="p.id" class="pv-item">
      <div class="pv-row" :data-pv-id="p.id">
        <input class="pv-active" type="radio" :checked="activeId === p.id" :title="'设为当前使用这一份'"
               @change="setActive(p.id)">
        <span class="pv-name" :title="p.name || p.id">{{ p.name || p.id }}</span>
        <span class="pv-badge" title="用户自定义的配置">{{ p.builtin ? '内置' : '自定义' }}</span>
        <span class="pv-dot" :class="{ on: !!p.api_key_set }"
              :title="p.api_key_set ? '已配置 API 密钥' : '还没配 API 密钥'"></span>
        <span class="pv-muted" :title="'协议：' + protocolLabel(protocols, p.api)">{{ protocolLabel(protocols, p.api) }}</span>
        <span class="pv-spacer"></span>
        <button class="pv-ico" :class="{ on: expandedId === p.id }" :title="expandedId === p.id ? '收起' : '编辑'"
                @click="toggleEdit(p.id)"><Icon :name="expandedId === p.id ? 'check' : 'edit'" :size="12" /></button>
        <button class="pv-ico danger" title="删除这份配置" @click="removeProvider(p)"><Icon name="trash" :size="12" /></button>
      </div>

      <!-- 展开：紧凑，一节一行 -->
      <div v-if="expandedId === p.id" class="pv-panel" :data-pv-id="p.id">
        <label class="pv-line">
          <span class="pv-lbl">API 密钥</span>
          <input class="pv-input" type="password" autocomplete="off" v-model="p.api_key"
                 :placeholder="p.api_key_set ? '已设置（留空 = 不修改）' : 'sk-…'"
                 title="只写不读：服务端不回明文；留空 = 不动这个键">
        </label>

        <button class="pv-toggle" @click="subOpen[p.id] = !subOpen[p.id]"
                :aria-expanded="subOpen[p.id] ? 'true' : 'false'">
          <span class="pv-caret">{{ subOpen[p.id] ? '▾' : '▸' }}</span> 自定义设置
        </button>
        <div v-show="subOpen[p.id]" class="pv-sub" data-pv-sub>
          <label class="pv-line">
            <span class="pv-lbl">显示名称</span>
            <input class="pv-input" v-model="p.name" placeholder="给人看的名字">
          </label>
          <label class="pv-line">
            <span class="pv-lbl">API 地址</span>
            <input class="pv-input" v-model="p.base_url" placeholder="https://api.deepseek.com/v1">
          </label>
          <label class="pv-line">
            <span class="pv-lbl">API 协议</span>
            <select class="pv-input" v-model="p.api"
                    :title="'请求怎么发（协议注册表）。清单来源：' + (protocolsSource === 'server' ? '后端' : '内置')">
              <option v-for="o in protocols" :key="o.id" :value="o.id">{{ o.label || o.id }}</option>
            </select>
          </label>
        </div>

        <div class="pv-line pv-well">
          <span class="pv-lbl">模型目录</span>
          <span class="pv-muted">{{ (p.models || []).length }} 条</span>
          <span class="pv-spacer"></span>
          <button class="pv-mini" title="清空各模型手填的能力（回到由能力表决定）" @click="restoreDefaults(p)">恢复默认</button>
          <button class="pv-mini" :disabled="busy[p.id]" title="按 API 地址取可用模型（表命中不联网，表外才真探）"
                  @click="fetchAvailable(p)">{{ busy[p.id] ? '获取中…' : '获取可用模型' }}</button>
        </div>
        <div v-if="note[p.id]" class="pv-note" :title="note[p.id]">{{ note[p.id] }}</div>

        <div v-for="(m, mi) in (p.models || [])" :key="key(p.id, mi) + '-' + (m.id || 'new')" class="pv-model">
          <div class="pv-row pv-model-row" :data-pv-model="m.id">
            <input class="pv-active" type="radio" :checked="p.model === m.id" title="设为当前模型"
                   @change="p.model = m.id">
            <input class="pv-id" v-model="m.id" placeholder="模型 id（如 cn:vendor.model）" @change="p.model === '' && (p.model = m.id)">
            <input class="pv-disp" v-model="m.name" placeholder="显示名">
            <span class="pv-spacer"></span>
            <button class="pv-ico" :class="{ on: openModel[key(p.id, mi)] }"
                    :title="openModel[key(p.id, mi)] ? '收起' : '展开能力/档位'"
                    @click="openModel[key(p.id, mi)] = !openModel[key(p.id, mi)]">
              <span style="font-size:11px">{{ openModel[key(p.id, mi)] ? '▾' : '▸' }}</span>
            </button>
            <button class="pv-ico danger" title="删除这个模型" @click="removeModel(p, mi)"><Icon name="trash" :size="12" /></button>
          </div>

          <div v-if="openModel[key(p.id, mi)]" class="pv-model-panel" :data-pv-model-panel="m.id">
            <div class="pv-two">
              <label class="pv-line">
                <span class="pv-lbl">上下文窗口</span>
                <input class="pv-input" type="number" min="0" :value="m.context_window ?? ''"
                       placeholder="由能力表" @input="m.context_window = $event.target.value === '' ? null : Number($event.target.value)">
              </label>
              <label class="pv-line">
                <span class="pv-lbl">最大输出</span>
                <input class="pv-input" type="number" min="0" :value="m.max_output ?? ''"
                       placeholder="由能力表" @input="m.max_output = $event.target.value === '' ? null : Number($event.target.value)">
              </label>
              <div class="pv-line">
                <span class="pv-lbl">输入类型</span>
                <label class="pv-chk"><input type="checkbox" :checked="inputOf(m).includes('text')"
                       @change="toggleInput(m, 'text', $event.target.checked)"> 文字</label>
                <label class="pv-chk"><input type="checkbox" :checked="inputOf(m).includes('image')"
                       @change="toggleInput(m, 'image', $event.target.checked)"> 图片</label>
                <span v-if="!m.input" class="pv-muted" title="没手填 = 由能力表/探针决定">未声明</span>
              </div>
            </div>

            <div class="pv-line">
              <label class="pv-chk"><input type="checkbox" :checked="m.reasoning === true"
                     @change="m.reasoning = $event.target.checked"> 支持推理</label>
              <button class="pv-toggle pv-toggle-inline" @click="openLevels[key(p.id, mi)] = !openLevels[key(p.id, mi)]"
                      :aria-expanded="openLevels[key(p.id, mi)] ? 'true' : 'false'">
                <span class="pv-caret">{{ openLevels[key(p.id, mi)] ? '▾' : '▸' }}</span>
                推理等级（{{ (m.levels || []).length }} 档）
              </button>
              <span class="pv-spacer"></span>
              <button class="pv-mini" title="加一档（档位 → API 参数值）" @click="addLevel(m)">+ 添加档位</button>
            </div>
            <template v-if="openLevels[key(p.id, mi)]">
              <div v-for="(lv, li) in (m.levels || [])" :key="li" class="pv-row pv-level-row" :data-pv-level="lv.level">
                <select class="pv-input pv-lv-sel" :value="lv.level" @change="setLevel(lv, $event.target.value)"
                        title="内部档位（词表固定）">
                  <option v-for="v in LEVEL_VOCAB" :key="v" :value="v">{{ LEVEL_ZH[v] || v }}</option>
                </select>
                <input class="pv-input" v-model="lv.wire_value" placeholder="API 参数值（如 max）"
                       title="这一档在请求体里怎么写（原样发）">
                <button class="pv-ico danger" title="删除这一档" @click="removeLevel(m, li)"><Icon name="trash" :size="12" /></button>
              </div>
            </template>
          </div>
        </div>

        <div class="pv-line pv-well">
          <button class="pv-mini" title="往模型目录里加一条（手填）" @click="addModel(p)">+ 添加模型</button>
          <span class="pv-spacer"></span>
          <span v-if="p.model" class="pv-muted" :title="'当前模型 id'">使用中：{{ p.model }}</span>
        </div>
      </div>
    </div>

    <div class="pv-line pv-well">
      <button class="pv-mini" title="再加一份独立配置（各带自己的地址/密钥/模型目录）" @click="addProvider">+ 添加配置</button>
      <span class="pv-spacer"></span>
      <span class="pv-muted" :title="protocolsSource === 'server' ? '协议清单来自后端注册表' : '协议清单是内置的（后端还没提供）'">
        协议清单：{{ protocolsSource === 'server' ? '后端' : '内置' }}（{{ protocols.length }} 条）
      </span>
    </div>
  </div>
</template>

<style scoped>
/* 紧凑口径：**一行一件事**、行高 ≤32px、标签左对齐定宽，次要项收进折叠 */
.pv { display:flex; flex-direction:column; gap:2px; min-width:0; }
.pv-item { min-width:0; }
.pv-row {
  display:flex; align-items:center; gap:6px; height:32px; min-width:0;
  padding:0 4px; border-radius:var(--r-sm); border:1px solid transparent;
}
.pv-row:hover { background:var(--bg-soft); }
/* ⚠️ app.css 里 `input,select{width:100%}` 对 **radio** 也生效（只有 checkbox 被豁免）——
   不写 width:auto 的话它撑满整行、把同一排的控件全推到右边外（实测：行 1254px 里 radio 占 1250px）。 */
.pv-active { margin:0; flex:none; width:auto; min-width:13px; height:auto; }
.pv-name { font-size:12.5px; font-weight:600; color:var(--ink); flex:none; max-width:180px;
  overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.pv-badge { flex:none; font-size:10.5px; line-height:1; padding:3px 5px; border-radius:999px;
  background:var(--main-soft); color:var(--pink-text); }
.pv-dot { flex:none; width:7px; height:7px; border-radius:50%; background:var(--ink-faint); }
.pv-dot.on { background:#3fa46a; }
.pv-muted { font-size:11px; color:var(--ink-faint); min-width:0; overflow:hidden;
  text-overflow:ellipsis; white-space:nowrap; }
.pv-spacer { flex:1 1 auto; min-width:4px; }
.pv-ico { flex:none; width:22px; height:22px; padding:0; display:inline-flex; align-items:center;
  justify-content:center; border:1px solid transparent; background:transparent; border-radius:var(--r-sm);
  color:var(--ink-dim); cursor:pointer; }
.pv-ico:hover { background:var(--card); border-color:var(--line); color:var(--ink); }
.pv-ico.on { border-color:var(--main); color:var(--main); }
.pv-ico.danger:hover { color:var(--pink-text); border-color:var(--pink-text); }
.pv-panel { padding:4px 4px 6px 26px; border-left:2px solid var(--main-soft); margin:2px 0 6px; }
.pv-line { display:flex; align-items:center; gap:8px; min-height:30px; min-width:0; }
.pv-lbl { flex:none; width:84px; font-size:11.5px; color:var(--ink-dim); }
.pv-input { flex:1 1 auto; min-width:0; height:26px; padding:2px 6px; font-size:12px; }
select.pv-input { height:26px; padding:1px 4px; }
.pv-toggle { display:inline-flex; align-items:center; gap:4px; height:28px; padding:0 6px;
  border:1px solid transparent; background:transparent; border-radius:var(--r-sm);
  font-size:12.5px; color:var(--ink); cursor:pointer; }
.pv-toggle:hover { background:var(--bg-soft); }
.pv-toggle-inline { height:24px; padding:0 4px; font-size:11.5px; color:var(--ink-dim); }
.pv-caret { font-size:10px; color:var(--ink-faint); }
.pv-sub { display:flex; flex-direction:column; gap:0; }
.pv-well { gap:8px; }
.pv-mini { flex:none; height:24px; padding:0 7px; font-size:11.5px; border-radius:var(--r-sm);
  border:1px solid var(--line); background:var(--card); color:var(--ink); cursor:pointer; }
.pv-mini:hover { border-color:var(--main); color:var(--main); }
.pv-mini:disabled { opacity:.5; cursor:not-allowed; }
.pv-note { font-size:11px; line-height:1.5; color:var(--ink-dim); padding:0 4px 2px 26px;
  overflow-wrap:anywhere; }
.pv-model { min-width:0; }
.pv-model-row { padding-left:4px; }
.pv-id { flex:1 1 45%; min-width:0; height:26px; padding:2px 6px; font-size:12px; }
.pv-disp { flex:1 1 30%; min-width:0; height:26px; padding:2px 6px; font-size:11.5px; color:var(--ink-dim); }
.pv-model-panel { padding:2px 4px 6px 26px; border-left:2px dashed var(--line-soft); margin-bottom:4px; }
.pv-two { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr); gap:0 12px; align-items:center; }
.pv-two > .pv-line:last-child { grid-column:1 / -1; }
.pv-chk { display:inline-flex; align-items:center; gap:5px; font-size:11.5px; color:var(--ink); cursor:pointer; }
.pv-chk input { margin:0; }
.pv-level-row { gap:6px; padding-left:0; }
.pv-lv-sel { flex:none; width:110px; }
@media (max-width:900px) {
  .pv-two { grid-template-columns:minmax(0,1fr); }
  .pv-lbl { width:72px; }
  .pv-name { max-width:120px; }
}
</style>
