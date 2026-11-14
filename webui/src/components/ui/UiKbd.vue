<template>
  <!-- 快捷键角标：`⌘K` / `Esc` / `Enter`。用 <kbd> 语义，样式与正文区分但不抢眼。 -->
  <kbd class="ukbd"><slot>{{ label }}</slot></kbd>
</template>

<script setup>
// ⚠️ prop 名**不能叫 `key`**：那是 Vue 的保留属性（虚拟 DOM 的 diff key），
// `<UiKbd key="Esc" />` 会被当成 diff key，props 里永远是 undefined ⇒ 渲染出一个**空方框**
// （实测踩过：命令面板底部的快捷键提示全变成空白胶囊）。用 `label`，或用默认插槽写内容。
defineProps({ label: { type: String, default: '' } })
</script>

<style scoped>
.ukbd { display:inline-flex; align-items:center; justify-content:center; min-width:18px; height:18px;
  padding:0 5px; border-radius:var(--r-xs); border:1px solid var(--line);
  border-bottom-width:2px; background:var(--surface-2); color:var(--ink-faint);
  font-family:var(--mono); font-size:10.5px; line-height:1; white-space:nowrap; }
</style>
