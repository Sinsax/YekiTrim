<template>
  <!-- 页头：全应用统一的三段之一（页头 / 工具条 / 内容）。
       以前每个页面自己写 `<div class="page-head"><h1>…</h1>…`，字号与间距各写各的。
       ⚠️ 吸顶上下文栏里的 `<h1>` 是**壳**的标题（App.vue），页面级标题要按页面语义二选一：
       两者同时存在时必须文案不同，否则会出现"首页 / 首页"上下各一遍。 -->
  <header class="uph" :class="{ compact }">
    <div class="uph-main">
      <div class="uph-row">
        <span v-if="eyebrow" class="uph-eyebrow">{{ eyebrow }}</span>
        <h1 class="uph-title">{{ title }}</h1>
        <slot name="meta" />
      </div>
      <p v-if="description" class="uph-desc">{{ description }}</p>
    </div>
    <div v-if="$slots.actions" class="uph-actions"><slot name="actions" /></div>
  </header>
</template>

<script setup>
defineProps({
  title: { type: String, required: true },
  description: { type: String, default: '' },
  /** 标题左侧的小标签（"出片""设置"这类），不是必填 */
  eyebrow: { type: String, default: '' },
  /** 紧凑档：省掉描述行、标题降到 20px（精修页这类竖向空间紧张的页面用） */
  compact: { type: Boolean, default: false }
})
</script>

<style scoped>
.uph { display:flex; align-items:flex-end; gap:var(--s-3); flex-wrap:wrap; margin-bottom:var(--s-4); }
.uph-main { min-width:0; flex:1 1 auto; }
.uph-row { display:flex; align-items:center; gap:var(--s-2); flex-wrap:wrap; }
.uph-eyebrow { flex:none; padding:1px 8px; border-radius:var(--r-full);
  background:var(--main-soft); color:var(--pink-text); font-size:var(--fs-xs); font-weight:var(--fw-semi); }
.uph-title { margin:0; font-size:var(--fs-2xl); font-weight:var(--fw-semi);
  letter-spacing:-.01em; line-height:1.25; color:var(--ink);
  overflow:hidden; text-overflow:ellipsis; white-space:nowrap; max-width:100%; }
.uph-desc { margin:6px 0 0; font-size:var(--fs-base); color:var(--ink-dim); line-height:1.5; }
.uph-actions { display:flex; align-items:center; gap:var(--s-2); flex-wrap:wrap; flex:none; }
.uph.compact .uph-title { font-size:var(--fs-xl); }
@media (max-width:720px) {
  .uph { align-items:flex-start; }
  .uph-title { font-size:var(--fs-xl); white-space:normal; }
  .uph-actions { width:100%; }
}
</style>
