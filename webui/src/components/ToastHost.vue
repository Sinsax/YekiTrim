<template>
  <!-- ⚠️ 定位与堆叠**只有 app.css 一份**（`.toast-wrap` + `.toast`）。
       这里曾经也写了一遍 `.toast`（卡片底/边距），而全局那份带着 `position:fixed; left:50%` ——
       两套各覆盖一半属性，子元素仍是 fixed、脱离容器的 flex 流，多条消息会全部叠在屏幕中央。 -->
  <div class="toast-wrap" aria-live="polite" aria-atomic="false">
    <div
      v-for="t in toastState.items"
      :key="t.id"
      class="toast"
      :class="t.type"
      :role="t.type === 'err' ? 'alert' : 'status'"
    >{{ t.message }}</div>
  </div>
</template>

<script setup>
import { toastState } from '../stores/toast'
</script>
