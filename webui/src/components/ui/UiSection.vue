<template>
  <section class="usec">
    <div v-if="title || $slots.actions" class="usec-head">
      <div class="usec-head-main">
        <h2 class="usec-title">{{ title }}</h2>
        <p v-if="description" class="usec-desc">{{ description }}</p>
      </div>
      <div v-if="$slots.actions" class="usec-actions"><slot name="actions" /></div>
    </div>
    <slot />
  </section>
</template>

<script setup>
// 分区：标题 + 描述 + 右侧动作 + 内容。用来替代各页手写的
// `<div class="section-title row spread">…`（那些的间距与字重各不相同）。
defineProps({
  title: { type: String, default: '' },
  description: { type: String, default: '' }
})
</script>

<style scoped>
.usec { display:flex; flex-direction:column; gap:var(--s-3); }
.usec + .usec { margin-top:var(--s-6); }
.usec-head { display:flex; align-items:flex-start; gap:var(--s-3); flex-wrap:wrap; }
.usec-head-main { min-width:0; flex:1 1 auto; }
.usec-title { margin:0; font-size:var(--fs-lg); font-weight:var(--fw-semi); color:var(--ink); }
.usec-desc { margin:3px 0 0; font-size:var(--fs-sm); color:var(--ink-dim); }
.usec-actions { display:flex; align-items:center; gap:var(--s-2); flex-wrap:wrap; flex:none; }
</style>
