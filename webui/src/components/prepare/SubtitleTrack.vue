<template>
  <div class="lane-wrap">
    <div class="lane" ref="laneRef" @pointerdown.self="seekFromEvent">
      <div
        v-for="(s, i) in segments"
        :key="i"
        class="seg"
        :class="{ editing: editing === i, active: isActive(s) }"
        :style="styleOf(s)"
        :title="`${fmt(s.start)} - ${fmt(s.end)}｜点击编辑文本，拖动调整时间`"
        @pointerdown.stop="startDrag(i, $event)"
        @click.stop="beginEdit(i)"
      >
        <input
          v-if="editing === i"
          ref="inputRef"
          v-model="draft"
          class="seg-input"
          @keydown.enter.prevent="commit(i)"
          @keydown.esc.prevent="cancel"
          @blur="commit(i)"
          @pointerdown.stop
        />
        <span v-else class="seg-text">{{ s.text }}</span>
      </div>
      <div class="playhead" :style="{ left: pct(currentTime) + '%' }"></div>
    </div>
    <div class="lane-legend muted small">
      <span>{{ segments.length }} 段字幕</span>
      <span>点击块编辑文本 · 拖动块平移时间 · 点空白跳转</span>
    </div>
  </div>
</template>

<script setup>
import { computed, nextTick, ref } from 'vue'

const props = defineProps({
  segments: { type: Array, default: () => [] },
  duration: { type: Number, default: 0 },
  currentTime: { type: Number, default: 0 }
})
const emit = defineEmits(['save', 'seek'])

const laneRef = ref(null)
const editing = ref(-1)
const draft = ref('')
const inputRef = ref(null)
let drag = null

const span = computed(() => Math.max(props.duration || 0, maxEnd.value, 1))
const maxEnd = computed(() => props.segments.reduce((m, s) => Math.max(m, Number(s.end) || 0), 0))

function pct(t) {
  return Math.min(100, Math.max(0, ((Number(t) || 0) / span.value) * 100))
}

function styleOf(s) {
  const left = pct(s.start)
  const width = Math.max(0.4, pct(s.end) - left)
  return { left: left + '%', width: width + '%' }
}

function isActive(s) {
  const t = Number(props.currentTime) || 0
  return t >= (Number(s.start) || 0) && t <= (Number(s.end) || 0)
}

function fmt(t) {
  const v = Math.max(0, Number(t) || 0)
  const m = Math.floor(v / 60)
  const sec = (v % 60).toFixed(1)
  return `${m}:${sec.padStart(4, '0')}`
}

function seekFromEvent(e) {
  const rect = laneRef.value?.getBoundingClientRect()
  if (!rect || !rect.width) return
  const ratio = (e.clientX - rect.left) / rect.width
  emit('seek', Math.max(0, ratio * span.value))
}

function beginEdit(i) {
  if (drag?.moved) return
  editing.value = i
  draft.value = props.segments[i]?.text || ''
  nextTick(() => {
    const el = Array.isArray(inputRef.value) ? inputRef.value[0] : inputRef.value
    el?.focus()
    el?.select?.()
  })
}

function cancel() {
  editing.value = -1
  draft.value = ''
}

function commit(i) {
  if (editing.value !== i) return
  const next = props.segments.map((s, idx) => (idx === i ? { ...s, text: draft.value } : s))
  editing.value = -1
  emit('save', next)
}

// ---- 拖动平移时间 ----
function startDrag(i, e) {
  const rect = laneRef.value?.getBoundingClientRect()
  if (!rect) return
  const perPx = span.value / rect.width
  drag = { i, x0: e.clientX, moved: false, seg: { ...props.segments[i] }, perPx }
  window.addEventListener('pointermove', onMove)
  window.addEventListener('pointerup', onUp, { once: true })
}

function onMove(e) {
  if (!drag) return
  const dx = (e.clientX - drag.x0) * drag.perPx
  if (Math.abs(e.clientX - drag.x0) > 3) drag.moved = true
  const dur = Math.max(0.1, (drag.seg.end || 0) - (drag.seg.start || 0))
  const start = Math.max(0, (drag.seg.start || 0) + dx)
  const next = props.segments.map((s, idx) =>
    idx === drag.i ? { ...s, start, end: start + dur } : s
  )
  emit('save', next, { transient: true })
}

function onUp() {
  window.removeEventListener('pointermove', onMove)
  const moved = drag?.moved
  drag = null
  if (moved) emit('save', props.segments, { committed: true })
}
</script>

<style scoped>
.lane-wrap { display:flex; flex-direction:column; gap:4px; }
.lane { position:relative; height:46px; border-radius:10px; background:var(--bg-soft); border:1px solid var(--line); overflow:hidden; touch-action:none; }
.seg { position:absolute; top:4px; bottom:4px; border-radius:7px; background:#5E7FD8; color:#fff; font-size:10px; display:flex; align-items:center; overflow:hidden; padding:0 4px; cursor:text; user-select:none; }
.seg-text { white-space:nowrap; overflow:hidden; text-overflow:ellipsis; pointer-events:none; }
.seg.active { outline:2px solid var(--main); }
.seg.editing { z-index:3; box-shadow:var(--shadow-hover); }
.seg-input { width:100%; height:100%; padding:0 4px; font-size:11px; border:none; border-radius:6px; background:#fff; color:var(--ink); }
.playhead { position:absolute; top:0; bottom:0; width:2px; background:var(--red); pointer-events:none; }
.lane-legend { display:flex; justify-content:space-between; gap:8px; }
</style>
