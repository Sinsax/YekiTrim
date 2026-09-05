<template>
  <div class="subtitle-editor">
    <div class="row mb-8">
      <button class="btn" @click="addSub"><Icon name="plus" :size="13" /> 添加字幕</button>
      <button class="btn btn-primary" @click="emit('save')"><Icon name="save" :size="13" /> 保存字幕</button>
    </div>
    <SubtitlePanel :segments="segments" :height="height" show-end editable-time @play="emit('play', $event)" />
  </div>
</template>

<script setup>
import { computed } from 'vue'
import Icon from './Icon.vue'
import SubtitlePanel from './SubtitlePanel.vue'

const props = defineProps({
  segments: { type: Array, required: true },
  currentTime: { type: Number, default: 0 },
  height: { type: String, default: '100%' }
})
const emit = defineEmits(['save', 'play'])

const start = computed(() => {
  const t = Number(props.currentTime) || 0
  if (t > 0) return Math.round(t * 100) / 100
  const last = props.segments[props.segments.length - 1]
  return last ? Math.round((last.end || 0) * 100) / 100 : 0
})

function addSub() {
  const s = start.value
  props.segments.push({ start: s, end: Math.round((s + 3) * 100) / 100, text: '' })
  // 简单提示由父页面负责（如果需要）
}
</script>

<style scoped>
.subtitle-editor { width:100%; }
</style>
