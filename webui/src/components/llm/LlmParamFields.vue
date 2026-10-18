<script setup>
// LlmParamFields — 一个模型的**可调参数**（能力驱动：只画这个模型真支持的控件）。
//
// 数据流：能力（caps）由父层给（api/llm.js::fetchCatalog 或模型列表带回，经 ./caps.js 归一化）；
// 本组件**不联网、不自己推能力**。
//
// 三条纪律（每条都对着一个真踩过的坑）：
//  ① **档位清单只来自 caps.levels**（就是那个对象的键，顺序也原样用）—— 写死一份 off/low/high
//     正是这轮要消灭的东西；「跟随默认」用**勾选框**表达，不占一个 option（否则下拉里就混进了
//     不是档位的东西，且"选项数 == levels 条数"这条判据会失真）。
//  ② caps.temperature === 'unsupported' ⇒ 温度滑块**禁用 + 行内写明原因**（只挂 title = 用户拖不动只能猜）。
//  ③ 能力**未知**（null / hit:false）⇒ 只画通用控件，并写明"能力未知 / 未收录"；绝不猜一份档位出来。
//
// ⚠️ target 是父层的 **reactive 行对象**（全局 form.llm 或某条链路的覆盖行），本组件只改它的属性、
//    不替换它 —— 所以父层不用写一串 update: 事件（设置页 16 处数值项也是这个绑法）。
import { computed } from 'vue'
import SliderField from '../SliderField.vue'
import { LEVEL_ZH } from './caps.js'

const props = defineProps({
  /** 归一化后的能力（caps.js::normalizeCaps）；null = 未知 */
  caps: { type: Object, default: null },
  /** 直接绑定的参数对象（父层 reactive；只改属性） */
  target: { type: Object, required: true },
  /** 画哪几个控件（按链路那几行没有 json_mode：它写的是全局 extra_params） */
  fields: { type: Array, default: () => ['thinking', 'temperature', 'max_tokens', 'json_mode'] },
  /** json_mode 的**真值**由父层算（那是 extra_params.response_format 的一个视图） */
  jsonMode: { type: Boolean, default: false },
  /** 同一页会出现多份（全局 + 四条链路）⇒ id 要能区分 */
  idPrefix: { type: String, default: 'llm-p' }
})
const emit = defineEmits(['update:jsonMode'])

const show = (f) => props.fields.includes(f)

/** 档位清单：**服务端给的键，原样不排序、不补齐** */
const levels = computed(() => (props.caps && Array.isArray(props.caps.levels)) ? props.caps.levels : [])
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

const cur = computed(() => String(props.target.thinking ?? ''))
/** 指定了档位（写这个键）还是跟随默认（删这个键） */
const hasLevel = computed(() => !!cur.value)
/** 手改出来的、这个模型没有的档位：照实显示（补一条），不静默改写 */
const strayLevel = computed(() => (hasLevel.value && !levels.value.includes(cur.value)) ? cur.value : '')

const thinkingTitle = computed(() => {
  if (props.caps == null) return '还没拿到这个模型的能力（先「获取模型」或填好 base_url + model）'
  if (!levels.value.length) return '能力表里这个模型没有推理档位，所以没有这个控件'
  return '档位清单来自模型能力表；勾「跟随默认」= 不写 [llm] thinking 这个键（由链路默认 / extra_params 决定）'
})
const thinkNote = computed(() => {
  if (props.caps == null) return '能力未知，未列推理档位'
  if (!levels.value.length) return '该模型没有推理档位'
  return ''
})
const jsonTitle = computed(() => '写 [llm] extra_params.response_format = {"type":"json_object"}；清掉它就是不要求 JSON')

function toggleInherit(inherit) {
  if (inherit) { props.target.thinking = ''; return }
  // 取消「跟随默认」= 真的写一个档位：取服务端给的第一个（通常 off），用户随即可以改
  props.target.thinking = levels.value[0] || ''
}
</script>

<template>
  <div class="lpf">
    <!-- 推理档位：清单 = caps.levels 的键（数据驱动）；「跟随默认」用勾选框（= 删键） -->
    <div v-if="show('thinking') && levels.length" class="lpf-line">
      <span class="lpf-name">推理档位</span>
      <label class="lpf-inherit" :title="thinkingTitle">
        <input type="checkbox" :checked="!hasLevel" @change="toggleInherit($event.target.checked)">
        <span>跟随默认</span>
      </label>
      <select v-if="hasLevel" :id="idPrefix + '-thinking'" v-model="target.thinking"
              class="lpf-sel" :title="thinkingTitle" aria-label="推理档位">
        <option v-for="lv in levels" :key="lv" :value="lv">{{ LEVEL_ZH[lv] || lv }}</option>
        <option v-if="strayLevel" :value="strayLevel">{{ LEVEL_ZH[strayLevel] || strayLevel }}（当前值）</option>
      </select>
      <span v-else class="muted small lpf-note">用链路默认</span>
    </div>
    <div v-else-if="show('thinking')" class="muted small lpf-note" :title="thinkingTitle">{{ thinkNote }}</div>

    <!-- 温度：不支持就**禁用 + 行内原因**（不是只挂 title） -->
    <SliderField v-if="show('temperature')" v-model="target.temperature" label="温度"
                 :min="0" :max="2" :step="0.1" :disabled="tempUnsupported"
                 :hint="tempUnsupported ? '该模型不支持温度，已禁用' : ''" />

    <!-- 输出上限：滑块上限 = 模型能力值（未知才回落到 65536） -->
    <SliderField v-if="show('max_tokens')" v-model="target.max_tokens" label="输出上限"
                 :min="256" :max="maxOut" :step="256" :hint="maxOutHint" />

    <!-- 要求 JSON 输出：写的是全局 extra_params.response_format（不是每条链路自己的键） -->
    <label v-if="show('json_mode')" class="lpf-check" :title="jsonTitle">
      <input type="checkbox" :checked="jsonMode" :disabled="jsonUnsupported"
             @change="emit('update:jsonMode', $event.target.checked)">
      <span>要求 JSON 输出</span>
    </label>

    <div v-if="capsUnlisted && show('thinking')" class="muted small lpf-note">
      能力表未收录，按通用参数
    </div>
  </div>
</template>

<style scoped>
.lpf { display:flex; flex-direction:column; gap:2px; min-width:0; }
.lpf-line { display:flex; flex-wrap:wrap; align-items:center; gap:2px 8px; min-width:0; }
.lpf-name { font-size:12px; color:var(--ink-dim); }
.lpf-inherit { display:inline-flex; align-items:center; gap:5px; font-size:12px; color:var(--ink-dim); cursor:pointer; }
.lpf-inherit input { margin:0; }
.lpf-sel { width:auto; min-width:112px; padding:3px 6px; font-size:12.5px; }
.lpf-note { overflow-wrap:anywhere; }
.lpf-check { display:flex; align-items:center; gap:7px; font-size:12.5px; color:var(--ink); cursor:pointer; }
.lpf-check input { margin:0; }
</style>
