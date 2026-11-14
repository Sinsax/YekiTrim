<template>
  <!-- 指标块：一个数字 + 一个标签（+ 可选说明）。首页统计条 / 用量页共用。
       `tone` 只改数字颜色，背景一律走卡片，不做彩色大色块（"别把大面积 UI 染橙"）。 -->
  <div class="ustat" :class="[`t-${tone}`]">
    <span class="ustat-value">{{ value }}<span v-if="unit" class="ustat-unit">{{ unit }}</span></span>
    <span class="ustat-label">{{ label }}</span>
    <span v-if="hint" class="ustat-hint" :title="hint">{{ hint }}</span>
  </div>
</template>

<script setup>
defineProps({
  value: { type: [String, Number], required: true },
  label: { type: String, required: true },
  unit: { type: String, default: '' },
  hint: { type: String, default: '' },
  /** 'default' | 'ok' | 'warn' | 'main' */
  tone: { type: String, default: 'default' }
})
</script>

<style scoped>
.ustat { display:flex; flex-direction:column; gap:2px; min-width:0; }
.ustat-value { font-size:var(--fs-xl); font-weight:var(--fw-semi); line-height:1.2;
  color:var(--ink); font-variant-numeric:tabular-nums; }
.ustat-unit { margin-left:2px; font-size:var(--fs-sm); font-weight:var(--fw-medium); color:var(--ink-faint); }
.ustat-label { font-size:var(--fs-xs); color:var(--ink-faint); }
.ustat-hint { font-size:var(--fs-xs); color:var(--ink-faint); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.t-ok .ustat-value { color:var(--green-deep); }
.t-warn .ustat-value { color:var(--amber-deep); }
.t-main .ustat-value { color:var(--pink-text); }
</style>
