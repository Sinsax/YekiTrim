<template>
  <div class="useg" role="tablist" :aria-label="ariaLabel">
    <button
      v-for="o in options"
      :key="o.value"
      type="button"
      role="tab"
      class="useg-item"
      :class="{ on: o.value === modelValue }"
      :aria-selected="o.value === modelValue ? 'true' : 'false'"
      :title="o.title || o.label"
      :disabled="o.disabled"
      @click="$emit('update:modelValue', o.value)"
    >
      <Icon v-if="o.icon" :name="o.icon" :size="13" />
      <span>{{ o.label }}</span>
      <span v-if="o.badge != null" class="useg-badge">{{ o.badge }}</span>
    </button>
  </div>
</template>

<script setup>
// 分段控件：一组**互斥**选项（快速/精修、已剪切/未剪切、时间/热度…）。
// 各页面以前各写一套（`.mode-seg` / `.wb-tabs` / `.subfilters`），选中态与键盘行为都不一致。
// 契约不变：受控 `modelValue` + `update:modelValue`，点击才发事件（不做内部状态）。
import Icon from '../Icon.vue'

defineProps({
  modelValue: { type: [String, Number, Boolean], default: '' },
  /** `{ value, label, icon?, title?, badge?, disabled? }[]` */
  options: { type: Array, required: true },
  ariaLabel: { type: String, default: '视图切换' }
})
defineEmits(['update:modelValue'])
</script>

<style scoped>
.useg { display:inline-flex; align-items:center; gap:2px; padding:3px;
  background:var(--surface-2); border:1px solid var(--line-soft); border-radius:var(--r-full); }
.useg-item { display:inline-flex; align-items:center; gap:5px; padding:4px 12px;
  border:none; background:none; border-radius:var(--r-full); color:var(--ink-dim);
  font-size:var(--fs-sm); font-weight:var(--fw-medium); cursor:pointer; white-space:nowrap;
  transition:background-color var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease); }
.useg-item:hover:not(:disabled) { color:var(--ink); }
.useg-item.on { background:var(--card); color:var(--pink-text); font-weight:var(--fw-semi); box-shadow:var(--e-1); }
.useg-item:disabled { opacity:.45; cursor:not-allowed; }
.useg-badge { min-width:16px; padding:0 5px; border-radius:var(--r-full);
  background:var(--bg-soft); color:var(--ink-dim); font-size:10px; line-height:15px; text-align:center; }
.useg-item.on .useg-badge { background:var(--main-soft); color:var(--pink-text); }
@media (max-width:760px) {
  .useg { display:flex; width:100%; overflow-x:auto; scrollbar-width:none; }
  .useg::-webkit-scrollbar { display:none; }
  .useg-item { flex:none; }
}
</style>
