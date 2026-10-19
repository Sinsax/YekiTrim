<script setup>
// LlmParamFields — 一个模型的**可调参数**（能力驱动：只画这个模型真支持的控件）。
//
// 数据流：能力（caps）由父层给（api/llm.js::fetchCatalog 或模型列表带回，经 ./caps.js 归一化）；
// 本组件**不联网、不自己推能力**。
//
// 三条纪律（每条都对着一个真踩过的坑）：
//  ① **档位清单只来自 caps.levels**（就是那个对象的键）—— 写死一份 off/low/high 正是这轮要消灭的东西；
//     「跟随默认」用**勾选框**表达，不占一个 option（否则下拉里就混进了不是档位的东西，
//     且"选项数 == levels 条数"这条判据会失真）。
//     ⚠️ **关不掉就别摆**：`can_disable:false` / `only_reasoning:true` ⇒ 摘掉 off 并写一句灰字原因；
//        这两个字段**缺失/null = 未声明 ⇒ 按老行为**（R4，表里有没有 off 就照画）。
//  ② caps.temperature === 'unsupported' ⇒ 温度滑块**禁用 + 行内写明原因**（只挂 title = 用户拖不动只能猜）。
//  ③ 能力**未知**（null / hit:false）⇒ 只画通用控件，并写明"能力未知 / 未收录"；绝不猜一份档位出来。
//     ⚠️ **"未声明" ≠ "不支持"**：`levels: {}` / `temperature` 缺字段时照实写「未声明（默认…）」，
//        不要画一个空下拉、也不要把控件灰掉假装"这家不支持"（那会让人以为功能坏了）。
//  ④ **生成能力（输入模态）是勾选框**：勾选状态由父层给（它要写进 [llm.models."<id>"].input），
//     本组件只 emit。默认值 = 能力表的 `input`；**不勾图片时给一句灰字说明**（别用禁用态写死 ——
//     用户要能自己勾上覆盖，这正是"能获取当然最好、获取不到就手填"那条路）。
//
// ⚠️ target 是父层的 **reactive 行对象**（全局 form.llm.params 或某条链路的覆盖行），本组件只改它的属性、
//    不替换它 —— 所以父层不用写一串 update: 事件（设置页 16 处数值项也是这个绑法）。
import { computed } from 'vue'
import SliderField from '../SliderField.vue'
import { INPUT_ZH, LEVEL_ZH, disableBlocked, disableReason, visibleLevels } from './caps.js'

const props = defineProps({
  /** 归一化后的能力（caps.js::normalizeCaps）；null = 未知 */
  caps: { type: Object, default: null },
  /** 直接绑定的参数对象（父层 reactive；只改属性） */
  target: { type: Object, required: true },
  /** 画哪几个控件（按链路那几行没有 json_mode / input：它们写的是模型级与全局的键） */
  fields: { type: Array, default: () => ['input', 'reasoning', 'temperature', 'max_tokens', 'json_mode'] },
  /** json_mode 的**真值**由父层算（那是 extra_params.response_format 的一个视图） */
  jsonMode: { type: Boolean, default: false },
  /** 生成能力（输入模态）：勾选状态由父层算（能力表的值，或用户手填的覆盖） */
  inputText: { type: Boolean, default: true },
  inputImage: { type: Boolean, default: false },
  /** 服务端**声明过** input 没有（没声明 = 用的是保守默认，灰字里要说清） */
  inputDeclared: { type: Boolean, default: false },
  /** 同一页会出现多份（全局 + 四条链路）⇒ id 要能区分 */
  idPrefix: { type: String, default: 'llm-p' },
  /** 按链路那几份：**不重复全局已经说过的解释**（同一句"未收录/未声明"在页面上会重复 8 次）。
   *  只留"会影响这一条怎么选"的提示（关不掉 / 文件里是 off 但关不掉）。 */
  compact: { type: Boolean, default: false }
})
const emit = defineEmits(['update:jsonMode', 'update:inputText', 'update:inputImage'])

const show = (f) => props.fields.includes(f)

/** 档位清单：**服务端给的键**（caps.js 已按规格排好序），关不掉时摘掉 off；不补齐、不裁剪别的档 */
const levels = computed(() => visibleLevels(props.caps))
/** 服务端声明的默认档（「跟随默认」的落点）—— 只用于显示，不改用户的值 */
const defaultLevel = computed(() => (props.caps && props.caps.defaultLevel) || '')
/** 关不掉的原因（空串 = 不显示那行灰字） */
const capsDisableNote = computed(() => disableReason(props.caps))
const tempUnsupported = computed(() => !!props.caps && props.caps.temperature === 'unsupported')
const jsonUnsupported = computed(() => !!props.caps && props.caps.jsonMode === false)
/** 输出上限的能力值只做**滑块的 max**（它不是每请求默认值）；未知就沿用老界面的 65536 */
const maxOut = computed(() => Math.max(256, (props.caps && props.caps.maxOutput) || 65536))
const maxOutHint = computed(() => {
  const c = props.caps && props.caps.maxOutput
  if (!c) return ''
  // 配置里的默认值（65536）可能**高过这支模型的上限**：照实说出来，别把超限的值藏起来
  // （滑块的 max 就是这个上限，拖一下自然落回；**不**在加载时偷偷改用户的值）
  const v = Number(props.target.max_tokens)
  return (Number.isFinite(v) && v > c) ? '模型上限 ' + c + '，当前超了' : '模型上限 ' + c
})
/** 能力表里**没收录**这对 (host, model)（hit:false）—— 与"还没拿到能力"(caps=null) 是两回事 */
const capsUnlisted = computed(() => !!props.caps && props.caps.hit === false)

const cur = computed(() => String(props.target.reasoning ?? ''))
/** 文件里写的是 off，但这个模型关不掉（灰字里照实说，别把 off 塞回下拉） */
const offBlocked = computed(() => disableBlocked(props.caps) && cur.value === 'off')
/** 指定了档位（写这个键）还是跟随默认（删这个键）。关不掉的 off 不算"选中了某一档"。 */
const hasLevel = computed(() => !!cur.value && !offBlocked.value)
/** 手改出来的、这个模型没有的档位：照实显示（补一条），不静默改写（关不掉的 off 除外） */
const strayLevel = computed(() => (hasLevel.value && !levels.value.includes(cur.value)) ? cur.value : '')

const thinkingTitle = computed(() => {
  if (props.caps == null) return '还没拿到这个模型的能力（先「获取模型」或填好 base_url + model）'
  if (!levels.value.length) return '能力表里这个模型没有推理档位，所以没有这个控件'
  const bits = ['档位清单来自能力面（内置目录 / 联网探测 / 手填）']
  bits.push(defaultLevel.value
    ? '勾「跟随默认」= 不写 [llm.params] reasoning 这个键，落点是服务端声明的默认档 ' + defaultLevel.value
    : '勾「跟随默认」= 不写 [llm.params] reasoning 这个键（由链路默认 / extra_params 决定）')
  if (capsDisableNote.value) bits.push(capsDisableNote.value)
  return bits.join('；')
})
const thinkNote = computed(() => {
  if (props.caps == null) return '能力未知，未列推理档位'
  // ⚠️ **未声明**（能力面没给清单）≠ **不支持**：写清楚"默认不发推理参数"，而不是"这家没有"
  if (!levels.value.length) return '该模型未声明推理档位（默认不发推理参数）'
  return ''
})
/** 温度：**未声明**时说"照发"，别说成"不支持"（后者才禁用控件） */
const tempHint = computed(() => {
  if (tempUnsupported.value) return '该模型不支持温度，已禁用'
  if (props.caps && props.caps.temperature === 'unknown') return '模型未声明温度（照发）'
  return ''
})
/** 工具调用：**只展示**（不是开关）。null = 未声明。 */
const toolCallText = computed(() => {
  if (!props.caps) return ''
  if (props.caps.toolCall == null) return '工具调用：未声明'
  return props.caps.toolCall ? '工具调用：支持' : '工具调用：不支持'
})
const inputImageTitle = computed(() => {
  const base = props.inputDeclared
    ? '能力表/探针说这个模型不吃图片'
    : '能力没声明图片（保守默认：只吃文字）'
  return base + ' —— 勾上就写进 [llm.models."<当前模型>"].input 覆盖它（保存后生效）'
})
const jsonTitle = computed(() => '写 [llm] extra_params.response_format = {"type":"json_object"}；清掉它就是不要求 JSON'
  + '（[llm.params] json_mode 是一等公民键，可在「编辑源文件」里写）')

function toggleInherit(inherit) {
  if (inherit) { props.target.reasoning = ''; return }
  // 取消「跟随默认」= 真的写一个档位：取服务端给的第一个（通常 off），用户随即可以改
  props.target.reasoning = levels.value[0] || ''
}
</script>

<template>
  <div class="lpf">
    <!-- 生成能力（输入模态）：**勾选**，不是"支持/不支持"的只读展示 —— 勾错了用户能自己改回来 -->
    <div v-if="show('input')" class="lpf-line">
      <span class="lpf-name">生成能力</span>
      <label class="lpf-check" title="能吃文字输入（chat/completions 基本都支持）">
        <input type="checkbox" :checked="inputText" @change="emit('update:inputText', $event.target.checked)">
        <span>{{ INPUT_ZH.text }}</span>
      </label>
      <label class="lpf-check" :title="inputImageTitle">
        <input type="checkbox" :checked="inputImage" @change="emit('update:inputImage', $event.target.checked)">
        <span>{{ INPUT_ZH.image }}</span>
      </label>
    </div>
    <div v-if="show('input') && !inputImage" class="muted small lpf-note" :title="inputImageTitle">
      图片输入未声明（可勾上覆盖）
    </div>

    <!-- 推理档位：选项**完全由能力面派生**（caps.levels 的键；关不掉时已摘掉 off）；
         「跟随默认」用勾选框（= 删键），并在 label 里写清落点是哪一档 -->
    <div v-if="show('reasoning') && levels.length" class="lpf-line">
      <span class="lpf-name">推理档位</span>
      <label class="lpf-inherit" :title="thinkingTitle">
        <input type="checkbox" :checked="!hasLevel" @change="toggleInherit($event.target.checked)">
        <span>跟随默认{{ defaultLevel ? '（' + defaultLevel + '）' : '' }}</span>
      </label>
      <select v-if="hasLevel" :id="idPrefix + '-reasoning'" v-model="target.reasoning"
              class="lpf-sel" :title="thinkingTitle" aria-label="推理档位">
        <option v-for="lv in levels" :key="lv" :value="lv">{{ LEVEL_ZH[lv] || lv }}</option>
        <option v-if="strayLevel" :value="strayLevel">{{ LEVEL_ZH[strayLevel] || strayLevel }}（当前值）</option>
      </select>
      <span v-else class="muted small lpf-note">用链路默认</span>
    </div>
    <!-- ⚠️ hit:false 时**不再说第二遍**：「未收录 ⇒ 默认不发推理参数」那一句已经说明同一件事 -->
    <div v-else-if="show('reasoning') && !compact && !capsUnlisted" class="muted small lpf-note"
         :title="thinkingTitle">{{ thinkNote }}</div>
    <!-- 关不掉：能力面说了算 —— 不摆"关闭"那一档，灰字写清原因（别让用户以为界面漏了） -->
    <div v-if="show('reasoning') && capsDisableNote" class="muted small lpf-note" :title="thinkingTitle">
      {{ capsDisableNote }}
    </div>
    <!-- 文件里那份是 off、但这模型关不掉：照实说，别静默改用户的值 -->
    <div v-if="show('reasoning') && offBlocked" class="muted small lpf-note"
         title="配置里写的是 off；这个模型的能力面说关不掉思考 ⇒ 保存后按默认档发，建议改选一档">
      当前写的是 off，该模型关不掉
    </div>

    <!-- 温度：不支持就**禁用 + 行内原因**（不是只挂 title） -->
    <SliderField v-if="show('temperature')" v-model="target.temperature" label="温度"
                 :min="0" :max="2" :step="0.1" :disabled="tempUnsupported"
                 :hint="tempHint" />

    <!-- 输出上限：滑块上限 = 模型能力值（未知才回落到 65536） -->
    <SliderField v-if="show('max_tokens')" v-model="target.max_tokens" label="输出上限"
                 :min="256" :max="maxOut" :step="256" :hint="maxOutHint" />

    <!-- 要求 JSON 输出：写的是全局 extra_params.response_format（不是每条链路自己的键） -->
    <label v-if="show('json_mode')" class="lpf-check" :title="jsonTitle">
      <input type="checkbox" :checked="jsonMode" :disabled="jsonUnsupported"
             @change="emit('update:jsonMode', $event.target.checked)">
      <span>要求 JSON 输出</span>
    </label>

    <!-- 只在**全局**那一份说一次（compact = 按链路那几份，不再重复同一句） -->
    <div v-if="capsUnlisted && show('input')" class="muted small lpf-note"
         title="未收录 ⇒ 走保守默认：不发推理参数、温度照发、max_tokens；要改就在「高级 · 模型能力覆盖」里手填">
      未收录 ⇒ 默认不发推理参数
    </div>

    <!-- 能力摘要（**只展示**，不是开关）：工具调用。null = 未声明，别猜成"支持"。 -->
    <div v-if="show('input') && toolCallText" class="muted small lpf-note"
         title="能力面给的 tool_call，仅展示 —— 这一版不把工具调用接到请求里">
      {{ toolCallText }}
    </div>
  </div>
</template>

<style scoped>
.lpf { display:flex; flex-direction:column; gap:2px; min-width:0; }
/* 滑块别吃满整行：SliderField 的滑条固定占"所在格"的 1/3，而「生成」那一格有 1200+px
   ⇒ 之前是 413px 滑条 + 中间 700px 空白 + 行尾数字框（看着像少了一列）。给个宽度上限即可。 */
.lpf :deep(.slf) { max-width:560px; }
.lpf-line { display:flex; flex-wrap:wrap; align-items:center; gap:2px 8px; min-width:0; }
.lpf-name { font-size:12px; color:var(--ink-dim); }
.lpf-inherit { display:inline-flex; align-items:center; gap:5px; font-size:12px; color:var(--ink-dim); cursor:pointer; }
.lpf-inherit input { margin:0; }
.lpf-sel { width:auto; min-width:112px; padding:3px 6px; font-size:12.5px; }
.lpf-note { overflow-wrap:anywhere; }
.lpf-check { display:flex; align-items:center; gap:7px; font-size:12.5px; color:var(--ink); cursor:pointer; }
.lpf-check input { margin:0; }
</style>
