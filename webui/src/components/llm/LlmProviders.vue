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
import { MODEL_SOURCE_ZH, fetchModels, testConnection } from '../../api/llm'
import { LEVEL_VOCAB, LEVEL_ZH } from './caps.js'
import { protocolLabel } from './protocols.js'
import LlmParamFields from './LlmParamFields.vue'

const props = defineProps({
  /** 父层的 reactive 数组：`form.llm.providers`（本组件原地增删改） */
  providers: { type: Array, required: true },
  /** 当前在用的 provider id（`[llm] active`） */
  activeId: { type: String, default: '' },
  /** API 协议清单（后端 `/config/llm/protocols`；拿不到时父层给内置清单） */
  protocols: { type: Array, default: () => [] },
  /** 'server' | 'builtin' —— 界面上照实标清单来源 */
  protocolsSource: { type: String, default: '' },
  // ---- 「参数」那一节要的输入（原来挂在页面顶层「生成」组，2026-10-19 搬进来）----
  /** 当前模型的能力（「推理档位」下拉按它派生） */
  caps: { type: Object, default: null }
})
const emit = defineEmits(['update:activeId', 'probe'])

/** **同一时间只展开一份**；再点「编辑」同一个 = 折叠 */
const expandedId = ref('')
/** 展开后**定位到哪一块**：`${id}|io` 或 `${id}|models`（行尾两个入口各管一块） */
const focusBlock = ref('')
/** provider.id → 正在获取模型 / 状态行 */
const busy = reactive({})
const note = reactive({})
/** provider.id → 正在测这份接口的连通性 */
const testing = reactive({})
/** provider.id → **API 侧模型**清单的三态：idle（还没获取）/ loading / done / error
 *  ⚠️ **失败 ≠ 空列表**（本仓对"静默伪装"零容忍）：三态分别有文案，别让失败长得像"服务器没有模型"。
 *  这份清单**不进 TOML**（只有用户点「加入」才会写进 providers[].models）。 */
const api = reactive({})
/** provider.id → API 侧过滤词（按 id / 显示名即时筛，大小写不敏感） */
const filters = reactive({})
function apiState(p) {
  return api[p.id] || { state: 'idle', models: [], note: '' }
}
/** 这一支在不在「已选」里（= providers[].models） */
function chosen(p, id) {
  const want = String(id || '').trim()
  return (p.models || []).some((x) => String(x.id || '').trim() === want)
}
function apiFiltered(p) {
  const list = apiState(p).models || []
  const q = String(filters[p.id] || '').trim().toLowerCase()
  if (!q) return list
  return list.filter((m) => String(m.id || '').toLowerCase().indexOf(q) >= 0
    || String(m.name || '').toLowerCase().indexOf(q) >= 0)
}
/** 从 API 侧加入：**只**写 id/name/input（别的字段一律不落 TOML）；默认只吃文字（与新建模型同口径） */
function addFromApi(p, m) {
  const id = String((m && m.id) || '').trim()
  if (!id || chosen(p, id)) return
  p.models = p.models || []
  p.models.push({ id, name: (m && m.name) || id, context_window: null, max_output: null, input: ['text'], reasoning: null, levels: [] })
  if (!p.model) p.model = id
}
/** 只测**这一份**接口：成功/失败都写进行内那行字（别用 toast 一闪） */
async function testOne(p) {
  testing[p.id] = true
  note[p.id] = ''
  try {
    const r = await testConnection(p.id)
    note[p.id] = '连通正常' + (r && r.model ? '（' + r.model + '）' : '')
  } catch (e) {
    note[p.id] = '测试失败：' + ((e && e.message) || e)   // **不许静默**
  } finally {
    testing[p.id] = false
  }
}
/** `${pid}/${mid}` → 模型行 / 推理等级是否展开 */
const openModel = reactive({})
const openLevels = reactive({})

const key = (pid, mid) => String(pid) + '/' + String(mid)
const canExpand = (p) => !!p && !!p.id

/** 展开并定位到某一块（滚动 + 高亮）——「编辑」→ io，「模型」→ models */
function focusSection(id, block) {
  if (!id) return
  expandedId.value = id
  focusBlock.value = id + '|' + block
  // 展开是异步渲染的 ⇒ 下一帧再滚
  requestAnimationFrame(() => {
    const el = document.querySelector('.pv-panel[data-pv-id="' + id + '"] [data-pv-block="' + block + '"]')
    if (el && el.scrollIntoView) el.scrollIntoView({ block: 'nearest' })
  })
}
/** 行尾「编辑」：管**接口字段**；再点同一个 = 折叠（状态机与从前一致） */
function toggleEdit(id) {
  if (!id) return
  if (expandedId.value === id) { expandedId.value = ''; focusBlock.value = ''; return }
  focusSection(id, 'io')
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
    // ⚠️ 不给 reasoning 空串（P0：空串进 llm.toml ⇒ 整份配置回退默认）；键缺席即可
    params: { temperature: 0.3, max_tokens: 65536, json_mode: false },
    // 新的一份没有"原值"快照 ⇒ 保存时这些键都算改过（会写实，别让它落到后端的代码默认值）
    paramsOrig: null,
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
    // 新建默认**只吃文字**（用户口径）；图片仍由能力面决定 ⇒ 默认不勾
    input: ['text'], reasoning: null, levels: []
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
/** 「获取模型」= 走既有探针接口（表命中不联网 / 表外才探）填**API 侧**那列；
 *  ⚠️ **不再自动并进「已选」**：用户明确要"两个列表"，加不加由他点「加入」。 */
async function fetchAvailable(p) {
  const base = String(p.base_url || '').trim()
  if (!base) { note[p.id] = '先填 API 地址（上面「接口」那一块）'; return }
  busy[p.id] = true
  note[p.id] = ''
  api[p.id] = { state: 'loading', models: (apiState(p).models || []), note: '' }
  try {
    // ⚠️ 带 provider id：不带的话后端会去顶层 [llm] api_key 找钥匙（多配置下那里是空的）
    const r = await fetchModels(base, p.api_key, p.id)
    api[p.id] = { state: 'done', models: r.models, note: r.note || '' }
    // 探针这次带回的能力（每支模型可能带 caps）交给父层：**只影响展示**（模型上限/档位/输入类型），不写盘
    emit('probe', { providerId: p.id, models: r.models })
    note[p.id] = '来源：' + (MODEL_SOURCE_ZH[r.source] || r.source) + '（' + r.models.length + ' 支）'
  } catch (e) {
    // **失败 ≠ 空列表**：记在 error 态里，界面照实说 + 给重试
    api[p.id] = { state: 'error', models: [], note: (e && e.message) || String(e) }
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
/** 输入类型：**显式存过**就照存的；没存过（null）= 未知 ⇒ 默认只吃文字（新建模型也是这个默认） */
function inputOf(m) { return Array.isArray(m.input) ? m.input : ['text'] }
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
        <!-- 两条入口分开（用户口径）：「模型」= 这份接口的模型目录；「编辑」= 接口字段（名称/地址/密钥/协议）。
             「测试连接」也在这行上（只测这一份接口，结果是行内一行字，不是一闪而过的 toast）。 -->
        <button class="pv-models" :title="'为这份配置选择模型（点开已选 / API 侧两个列表）'"
                :aria-label="'为这份配置选择模型（' + (p.models || []).length + ' 个已选）'"
                @click="focusSection(p.id, 'models')">模型 {{ (p.models || []).length }}
          <span class="pv-caret-down">▾</span></button>
        <button class="pv-mini pv-test" :disabled="testing[p.id]"
                :title="'只测这一份接口（POST /api/llm/test?provider=' + p.id + '）'"
                @click="testOne(p)">{{ testing[p.id] ? '测试中…' : '测试连接' }}</button>
        <button class="pv-ico" :class="{ on: expandedId === p.id }" :title="expandedId === p.id ? '收起' : '编辑接口字段'"
                @click="toggleEdit(p.id)"><Icon :name="expandedId === p.id ? 'check' : 'edit'" :size="12" /></button>
        <button class="pv-ico danger" title="删除这份配置" @click="removeProvider(p)"><Icon name="trash" :size="12" /></button>
      </div>

      <!-- 展开：**两块**（接口 / 模型），一行一项、标签定宽；行尾两个入口各管一块 -->
      <div v-if="expandedId === p.id" class="pv-panel" :data-pv-id="p.id">
        <!-- ① 接口字段：四行平铺（「自定义设置」那层折叠已拆掉 —— 用户：和 api_key 放一起） -->
        <div class="pv-block" data-pv-block="io" :class="{ 'is-focus': focusBlock === p.id + '|io' }">
          <label class="pv-line">
            <span class="pv-lbl">显示名称</span>
            <input class="pv-input" v-model="p.name" placeholder="给人看的名字">
          </label>
          <label class="pv-line">
            <span class="pv-lbl">API 地址</span>
            <input class="pv-input" v-model="p.base_url" placeholder="https://api.deepseek.com/v1">
          </label>
          <label class="pv-line">
            <span class="pv-lbl">API 密钥</span>
            <input class="pv-input" type="password" autocomplete="off" v-model="p.api_key"
                   :placeholder="p.api_key_set ? '已设置（留空 = 不修改）' : 'sk-…'"
                   title="只写不读：服务端不回明文；留空 = 不动这个键">
          </label>
          <label class="pv-line">
            <span class="pv-lbl">API 协议</span>
            <select class="pv-input" v-model="p.api"
                    :title="'请求怎么发（协议注册表）。清单来源：' + (protocolsSource === 'server' ? '后端' : '内置')">
              <option v-for="o in protocols" :key="o.id" :value="o.id">{{ o.label || o.id }}</option>
            </select>
          </label>
        </div>

        <!-- ② 模型：**两个列表**（左「已选」= 会写进 TOML 的；右「API 侧」= 获取模型拿到的）
             ⚠️ 加入/移除只改 `providers[].models` 这一份数组（写 patch 时是**整份替换**）。 -->
        <div class="pv-block" data-pv-block="models" :class="{ 'is-focus': focusBlock === p.id + '|models' }">
        <div class="pv-lists">
          <div class="pv-list pv-picked" data-pv-list="picked">
            <div class="pv-line pv-well">
              <span class="pv-lbl">已选模型（{{ (p.models || []).length }}）</span>
              <span class="pv-spacer"></span>
              <button class="pv-mini" title="清空各模型手填的能力（回到由能力表决定）" @click="restoreDefaults(p)">恢复默认</button>
            </div>

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
            <div class="pv-line">
              <span class="pv-lbl">输入类型</span>
              <label class="pv-chk"><input type="checkbox" :checked="inputOf(m).includes('text')"
                     @change="toggleInput(m, 'text', $event.target.checked)"> 文字</label>
              <label class="pv-chk"><input type="checkbox" :checked="inputOf(m).includes('image')"
                     @change="toggleInput(m, 'image', $event.target.checked)"> 图片</label>
              <span v-if="!m.input" class="pv-muted" title="没存过 = 默认只吃文字（勾掉文字会显式存成「都不支持」）">默认只吃文字</span>
            </div>

            <!-- 推理档位：只对**这份接口正在用的那个模型**显示（档位清单来自 caps.levels） -->
            <LlmParamFields v-if="activeId === p.id && p.model && m.id === p.model"
                            :caps="caps" :target="p.params" :fields="['reasoning']"
                            :id-prefix="'llm-r-' + p.id" compact />
            <div v-else class="pv-note">推理档位跟着「使用中」的模型走（上面圆点选一个，再切到这份接口）</div>

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
              <button class="pv-mini" title="手填一个模型（服务器啥都不说时的兜底）" @click="addModel(p)">+ 添加模型</button>
              <span class="pv-spacer"></span>
              <span v-if="p.model" class="pv-muted" :title="'当前模型 id'">使用中：{{ p.model }}</span>
            </div>
          </div>

          <!-- 右：API 侧模型（点「获取模型」拿到的清单；**已选的不可重复加**） -->
          <div class="pv-list pv-remote" data-pv-list="remote">
            <div class="pv-line pv-well">
              <span class="pv-lbl">API 侧模型（{{ (apiState(p).models || []).length }}）</span>
              <span class="pv-spacer"></span>
              <input class="pv-input pv-filter" v-model="filters[p.id]" placeholder="过滤…"
                     title="按 id / 显示名即时筛（大小写不敏感）">
              <button class="pv-mini pv-get" :disabled="busy[p.id]"
                      title="按这份接口的地址取可用模型（没保存的地址/密钥也能取；表命中不联网，表外才真探）"
                      @click="fetchAvailable(p)">{{ busy[p.id] ? '获取中…' : '获取模型' }}</button>
            </div>
            <div v-if="note[p.id]" class="pv-note" :title="note[p.id]">{{ note[p.id] }}</div>
            <!-- 三态分开说，**失败 ≠ 空列表** -->
            <div v-if="apiState(p).state === 'idle'" class="pv-note">
              还没获取 —— 点「获取模型」按 API 地址取这份接口能用的模型
            </div>
            <div v-else-if="apiState(p).state === 'loading'" class="pv-note">获取中…</div>
            <div v-else-if="apiState(p).state === 'error'" class="pv-note pv-err" :title="apiState(p).note" data-pv-api-state="error">
              获取失败：{{ apiState(p).note }}
              <button class="pv-mini" @click="fetchAvailable(p)">重试</button>
            </div>
            <template v-else>
              <div v-if="!(apiState(p).models || []).length" class="pv-note">服务器没返回模型（可手填：左列「+ 添加模型」）</div>
              <div v-else-if="!apiFiltered(p).length" class="pv-note">没有匹配「{{ filters[p.id] }}」的模型</div>
              <div v-else class="pv-api-list">
                <div v-for="m in apiFiltered(p)" :key="m.id" class="pv-row pv-api-row" :data-pv-api="m.id">
                  <span class="pv-chkmark">{{ chosen(p, m.id) ? '☑' : '☐' }}</span>
                  <span class="pv-api-id" :title="m.id">{{ m.id }}</span>
                  <span class="pv-muted pv-api-name" :title="m.name || ''">{{ m.name || '' }}</span>
                  <span class="pv-spacer"></span>
                  <span v-if="chosen(p, m.id)" class="pv-muted">（已选）</span>
                  <button v-else class="pv-mini" title="加进左列（只写 id / 显示名 / 输入类型）"
                          @click="addFromApi(p, m)">加入</button>
                </div>
              </div>
            </template>
          </div>
        </div>
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
/* 两块（接口 / 模型）：块之间一条细分隔线；行尾入口点哪块就给哪块一点底色提示 */
.pv-block { min-width:0; padding:2px 4px 4px; border-radius:var(--r-sm); }
.pv-block + .pv-block { margin-top:4px; border-top:1px dashed var(--line-soft); padding-top:6px; }
.pv-block.is-focus { background:var(--bg-soft); }
.pv-models { flex:none; height:22px; padding:0 6px; font-size:11px; border-radius:var(--r-sm);
  border:1px solid var(--line); background:var(--card); color:var(--ink-dim); cursor:pointer; }
.pv-models:hover { border-color:var(--main); color:var(--main); background:var(--main-soft); }
.pv-caret-down { font-size:9px; opacity:.7; margin-left:2px; }
.pv-get { max-width:96px; }
.pv-test { flex:none; }
/* 模型块：两个列表。窄（默认）上下两段，≥1100px 视口才并排（760px 内容列下两栏各 ~376px） */
.pv-lists { display:grid; grid-template-columns:minmax(0,1fr); gap:6px 12px; min-width:0; }
@media (min-width:1100px) { .pv-lists { grid-template-columns:minmax(0,1fr) minmax(0,1fr); } }
.pv-list { min-width:0; display:flex; flex-direction:column; gap:2px; }
/* 模型行：id / 显示名**定宽**（以前 flex:1 会撑满整行，一屏只有一行） */
.pv-id { flex:0 1 260px; max-width:260px; min-width:0; }
.pv-disp { flex:0 1 200px; max-width:200px; min-width:0; }
.pv-model-row { justify-content:flex-start; }
/* API 侧 */
.pv-filter { flex:0 0 110px; max-width:110px; height:24px; }
.pv-api-list { display:flex; flex-direction:column; gap:0; max-height:260px; overflow:auto; min-width:0; }
.pv-api-row { height:28px; }
.pv-chkmark { flex:none; width:14px; text-align:center; font-size:11px; color:var(--ink-dim); }
.pv-api-id { flex:0 1 auto; min-width:0; max-width:55%; overflow:hidden; text-overflow:ellipsis;
  white-space:nowrap; font-size:11.5px; }
.pv-api-name { flex:0 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.pv-err { color:var(--pink-text); }
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
