<template>
  <!-- 空态（v6）：图标环 + 标题 + 说明 + 主次动作。
      与旧 `AppEmptyState` 保持同一组接口名（icon/title/description/primary/secondary），
       便于逐页替换；新增 `tone` 与 `inline`（内嵌在卡片里的紧凑档）。 -->
  <div class="uempty" :class="{ inline }">
    <span v-if="icon" class="uempty-ic" :class="`t-${tone}`"><Icon :name="icon" :size="inline ? 16 : 22" /></span>
    <b class="uempty-title">{{ title }}</b>
    <p v-if="description" class="uempty-desc">{{ description }}</p>
    <div v-if="$slots.actions" class="uempty-actions"><slot name="actions" /></div>
  </div>
</template>

<script setup>
import Icon from '../Icon.vue'

defineProps({
  icon: { type: String, default: '' },
  title: { type: String, required: true },
  description: { type: String, default: '' },
  /** 'default' | 'ok' | 'warn' */
  tone: { type: String, default: 'default' },
  /** 内嵌档：没有卡片外框、更紧凑（放在已有卡片里时用） */
  inline: { type: Boolean, default: false }
})
</script>

<style scoped>
.uempty { display:flex; flex-direction:column; align-items:center; gap:6px;
  padding:var(--s-7) var(--s-4); text-align:center; }
.uempty.inline { padding:var(--s-5) var(--s-3); }
.uempty-ic { display:flex; align-items:center; justify-content:center; width:48px; height:48px;
  margin-bottom:var(--s-2); border-radius:var(--r-full);
  background:var(--main-soft); color:var(--pink-text); box-shadow:0 0 0 6px var(--surface-2); }
.uempty.inline .uempty-ic { width:34px; height:34px; margin-bottom:2px; box-shadow:0 0 0 4px var(--surface-2); }
.uempty-ic.t-ok { background:var(--green-soft); color:var(--green-deep); }
.uempty-ic.t-warn { background:var(--amber-soft); color:var(--amber-deep); }
.uempty-title { font-size:var(--fs-md); font-weight:var(--fw-semi); color:var(--ink); }
.uempty.inline .uempty-title { font-size:var(--fs-base); }
.uempty-desc { margin:0; max-width:44ch; font-size:var(--fs-sm); line-height:1.6; color:var(--ink-dim); }
.uempty-actions { display:flex; gap:var(--s-2); justify-content:center; flex-wrap:wrap; margin-top:var(--s-3); }
</style>
