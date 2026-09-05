<template>
  <div class="toast-wrap">
    <div v-for="t in toastState.items" :key="t.id" class="toast" :class="t.type">{{ t.message }}</div>
  </div>
</template>

<script setup>
import { toastState } from '../stores/toast'
</script>

<style scoped>
.toast-wrap { position:fixed; top:18px; right:18px; z-index:200; display:flex; flex-direction:column; gap:8px; }
.toast { padding:8px 16px; border-radius:999px; background:var(--card); border:1px solid var(--line); box-shadow:var(--shadow); font-size:13px; }
.toast.ok { border-color:var(--green); color:#2FA07E; }
.toast.err { border-color:var(--red); color:var(--red); }
</style>
