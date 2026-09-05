<template>
  <div class="steps" :class="size">
    <template v-for="i in 4" :key="i">
      <span class="dot" :class="dotClass(i)">{{ i < stage ? '✓' : i }}</span>
      <span v-if="i < 4" class="sl" :class="i < stage ? 'on' : ''"></span>
    </template>
  </div>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  stage: { type: Number, default: 0 },
  size: { type: String, default: 'normal' }
})

function dotClass(i) {
  if (i < props.stage) return 'done'
  if (i === props.stage) return 'now'
  return 'wait'
}
</script>

<style scoped>
.steps { display:flex; align-items:center; gap:4px; margin:10px 0 4px; }
.dot { width:20px; height:20px; border-radius:50%; display:flex; align-items:center; justify-content:center; font-size:10px; font-weight:700; flex:0 0 20px; }
.dot.done { background:var(--green-soft); color:var(--green); }
.dot.now { background:var(--main); color:#fff; box-shadow:0 0 0 3px var(--main-soft); }
.dot.wait { background:var(--card); border:2px dashed var(--ink-faint); color:var(--ink-faint); }
.sl { flex:1; height:5px; border-radius:3px; background:var(--line-soft); }
.sl.on { background:var(--main-glow); }
.steps.large .dot { width:28px; height:28px; flex-basis:28px; font-size:13px; }
.steps.large .sl { height:8px; }
</style>
