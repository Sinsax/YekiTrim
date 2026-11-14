<template>
  <!-- 表单字段：标签 + 控件 + 说明 + 错误，四件套统一。
       设置页以前是 `label` + `input` + `.hint` 各写一遍，错误态只有顶部一条横幅。 -->
  <div class="ufld" :class="{ inline, error: !!error, disabled }">
    <label v-if="label" class="ufld-label" :for="forId">
      {{ label }}<span v-if="required" class="ufld-req" aria-hidden="true">*</span>
    </label>
    <div class="ufld-ctl"><slot /></div>
    <p v-if="error" class="ufld-err">{{ error }}</p>
    <p v-else-if="hint" class="ufld-hint" :title="hint">{{ hint }}</p>
  </div>
</template>

<script setup>
defineProps({
  label: { type: String, default: '' },
  hint: { type: String, default: '' },
  error: { type: String, default: '' },
  forId: { type: String, default: '' },
  required: { type: Boolean, default: false },
  disabled: { type: Boolean, default: false },
  /** inline：标签在左、控件在右（设置页里的开关型字段） */
  inline: { type: Boolean, default: false }
})
</script>

<style scoped>
.ufld { display:flex; flex-direction:column; gap:4px; min-width:0; }
.ufld.inline { flex-direction:row; align-items:center; gap:var(--s-3); }
.ufld.inline .ufld-ctl { flex:1 1 auto; min-width:0; }
.ufld-label { font-size:var(--fs-sm); font-weight:var(--fw-medium); color:var(--ink-dim); }
.ufld.inline .ufld-label { flex:none; min-width:88px; }
.ufld-req { margin-left:2px; color:var(--red); }
.ufld-hint { margin:0; font-size:var(--fs-xs); color:var(--ink-faint); line-height:1.5; }
.ufld-err { margin:0; font-size:var(--fs-xs); color:var(--red-deep); line-height:1.5; }
.ufld.error :deep(input), .ufld.error :deep(select), .ufld.error :deep(textarea) { border-color:var(--red); }
.ufld.disabled { opacity:.6; }
</style>
