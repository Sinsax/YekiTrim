<script setup>
// 数值设置项：**一行装下「标签 · 滑块 · 数字框」**。
//
// 为什么还要能直接填：滑块有 step 栅格（chunk_duration 步进 5s、max_tokens 步进 256、
// num_workers 步进 1…），**栅格之外的值拖不出来**（想设 63s 就只能眼睁睁看它跳 60/65）。
// 手填的值只在 `[min, max]` 上夹、**不吸附 step** —— step 是滑块的语义，不是配置的语义。
//
// 布局：标签在左（`flex:1`，太长就自己换行），滑块只占**这一格宽度的 1/3**，数字框收在行尾。
// 格子太窄（LLM 那几格）时标签改成独占一行，另见样式里的 `@container`。
// 以前滑块吃满整格、读数被顶到最右，一行里三样东西各在一处、还被拉得很空（见 AGENTS.md 的约定块）；
// 现在一行读完，表单也紧。数字框仍在**行尾**（`label` 的位置约定是「读数在右端」，别挪到滑块左边）。
import { computed } from 'vue'

const props = defineProps({
  modelValue: { type: Number, default: 0 },
  min: { type: Number, default: 0 },
  max: { type: Number, default: 100 },
  step: { type: Number, default: 1 },
  label: { type: String, default: '' },
  unit: { type: String, default: '' },   // 显示用单位（s / dB / 字…），不参与取值
  hint: { type: String, default: '' },
  disabled: { type: Boolean, default: false }
})
const emit = defineEmits(['update:modelValue'])

/** 显示值：直接显示实际数值（不再 toFixed，手填的小数位要原样看得见） */
const text = computed(() => String(props.modelValue ?? ''))

function clamp(v) {
  const n = Number(v)
  if (!Number.isFinite(n)) return props.modelValue
  return Math.min(props.max, Math.max(props.min, n))
}
function commit(v) { emit('update:modelValue', clamp(v)) }

function onSlide(e) { commit(e.target.value) }

// 手填：清空 / 非数字 → 还原显示、**不写模型**（否则退格清空的一瞬间就把 60 变成 0）。
// 夹回来的值可能等于原值（填 999 被夹到 2）→ 模型没变化、Vue 不会回写 DOM，
// 这里手动同步一次，免得框里一直留着 999。
function onType(e) {
  const raw = String(e.target.value ?? '').trim()
  const v = Number(raw)
  if (raw === '' || !Number.isFinite(v)) { e.target.value = text.value; return }
  const c = clamp(v)
  emit('update:modelValue', c)
  e.target.value = String(c)
}
</script>

<template>
  <!-- 一行：标签 · 滑块（格宽 1/3）· 数字框；hint 需要时另起一行 -->
  <div class="slf">
    <span class="slf-name">{{ label }}</span>
    <input class="slf-range" type="range" :value="modelValue" :min="min" :max="max" :step="step"
           :disabled="disabled" :aria-label="label" @input="onSlide">
    <span class="slf-val">
      <input class="slf-num" type="number" inputmode="decimal" :value="text"
             :min="min" :max="max" :step="step" :disabled="disabled" :aria-label="label"
             :title="`可直接填写（${min} ~ ${max}）`"
             @change="onType" @keydown.enter="$event.target.blur()">
      <span v-if="unit" class="slf-unit">{{ unit }}</span>
    </span>
    <div v-if="hint" class="muted small slf-hint">{{ hint }}</div>
  </div>
</template>

<style scoped>
.slf { display:flex; flex-wrap:wrap; align-items:center; gap:2px 8px; min-width:0; container-type:inline-size; }
/* 标签文本：`label` 元素那套（12px / ink-dim / margin-bottom）不能直接用 —— 这里不是块级标签。
   `flex:1 1 0` 而**不是 `auto`**：basis 为 auto 时基数是标签的 max-content（长标签整行都是它），
   滑块与数字框会被一起挤到第二行 —— 那正是「读数掉到下一行、和滑块错位」的成因。
   basis 取 0 则标签自己缩着换行，永远是「滑块+数字框」先占位。 */
.slf-name { flex:1 1 0; min-width:0; overflow-wrap:anywhere;
  font-size:12px; line-height:1.35; color:var(--ink-dim); }
/* 滑块 = **整格宽的 1/3**（以前是 100%）。flex-basis 会盖掉 app.css 里 input[type=range] 的 width:100% */
.slf-range { flex:0 0 33.333%; min-width:0; }
.slf-val { display:inline-flex; align-items:baseline; gap:2px; flex:none; }
/* 数字框：常态「隐形」（不抢视觉，与从前的只读读数同貌），hover/focus 才现出可编辑的样子 */
.slf-num { width:7ch; padding:1px 4px; text-align:right; font-size:12.5px; font-weight:600;
  font-variant-numeric:tabular-nums; background:transparent; border:1px solid transparent;
  border-radius:var(--r-sm); color:var(--ink); }
.slf-num:hover { border-color:var(--line); background:var(--card); }
.slf-num:focus { border-color:var(--main); box-shadow:0 0 0 3px var(--main-soft); }
/* 手填允许落在 step 栅格之外（那是本组件存在的理由），别让浏览器把这种值画成「非法」 */
.slf-num:invalid { box-shadow:none; }
.slf-num:invalid:focus { box-shadow:0 0 0 3px var(--main-soft); }
/* 步进交给 ↑/↓ 键，不要那对 spinner：7ch 宽放不下，会把数字挤出去 */
.slf-num::-webkit-outer-spin-button, .slf-num::-webkit-inner-spin-button { -webkit-appearance:none; margin:0; }
.slf-num { -moz-appearance:textfield; appearance:textfield; }
.slf-unit { font-size:12.5px; font-weight:600; color:var(--ink); }
.slf-num:disabled, .slf-range:disabled { opacity:.5; cursor:not-allowed; }
/* 说明文字独占下一行（`flex:1 0 100%` 把它挤到换行处），别和滑块抢同一行 */
.slf-hint { flex:1 0 100%; margin-top:2px; }
/* 窄格（LLM 那几格只有 ~190px）一行塞不下三样东西：标签只剩 ~60px，`temperature` 会被从词中间切断。
   这时让标签独占一行（两行仍是「滑块 · 数字框」同行），`max-width` 量的是 `.slf` 自己的宽度 ——
   `container-type` 就在 `.slf` 上，容器查询按**容器宽度**而不是视口宽度分流（媒体查询在这里没用）。 */
@container (max-width: 220px) {
  .slf-name { flex:1 0 100%; }
}
</style>
