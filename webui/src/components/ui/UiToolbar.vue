<template>
  <!-- 工具条：过滤 / 搜索 / 排序 / 视图切换的**一行**。
       各页以前自己拼 `.row + mini-btn`，换行与间距不一致；这里统一"可换行、右端可推"。
       左侧 `default` 槽放主过滤；右侧 `end` 槽放排序/计数/视图切换（会自动靠右）。 -->
  <div class="utb" role="toolbar">
    <div class="utb-main"><slot /></div>
    <div v-if="$slots.end" class="utb-end"><slot name="end" /></div>
  </div>
</template>

<script setup>
// 无状态容器：只负责排版，不持有任何筛选值。
</script>

<style scoped>
.utb { display:flex; align-items:center; gap:var(--s-2); flex-wrap:wrap; margin-bottom:var(--s-3); min-width:0; }
.utb-main { display:flex; align-items:center; gap:6px; flex-wrap:wrap; min-width:0; }
.utb-end { display:flex; align-items:center; gap:6px; flex-wrap:wrap; margin-left:auto; }
@media (max-width:640px) {
  .utb-end { margin-left:0; width:100%; }
}
</style>
