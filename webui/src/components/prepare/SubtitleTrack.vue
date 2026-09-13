<template>
  <div class="lane-wrap">
    <div class="lane" ref="laneRef" @pointerdown.self="seekFromEvent">
      <div
        v-for="(s, i) in segments"
        :key="i"
        class="seg"
        :class="{ editing: editing === i, active: isActive(s) }"
        :style="styleOf(s)"
        :title="`${fmt(s.start)} - ${fmt(s.end)}｜点击编辑文本，拖动平移，拖边缘改起止，双击删除`"
        @pointerdown.stop="startDrag(i, $event)"
        @click.stop="beginEdit(i)"
        @dblclick.stop="removeSeg(i)"
      >
        <span class="rz l" title="拖动调整开始时间" @pointerdown.stop="startResize(i, 'start', $event)"></span>
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
        <span class="rz r" title="拖动调整结束时间" @pointerdown.stop="startResize(i, 'end', $event)"></span>
      </div>
      <div class="playhead" :style="{ left: pct(currentTime) + '%' }"></div>
    </div>
    <div class="lane-legend muted small">
      <span>{{ segments.length }} 段字幕</span>
      <span>点击编辑文本 · 拖动平移 · 拖边缘改起止 · 双击删除 · 点空白跳转</span>
    </div>
  </div>
</template>

<script setup>
import { computed, nextTick, ref } from 'vue'
import { confirmDialog } from '../../stores/dialog'

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

let gestureEndAt = 0   // 拖拽/缩放松手后的 click 不该再打开文本编辑
function beginEdit(i) {
  if (drag?.moved) return
  if (Date.now() - gestureEndAt < 200) return
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

// ---- 双击删除（与精修时间轴一致：确认后删除并保存）----
async function removeSeg(i) {
  const s = props.segments[i]
  if (!s) return
  const ok = await confirmDialog(`删除这条字幕？「${(s.text || '').slice(0, 24)}…」`, { title: '删除字幕', danger: true })
  if (!ok) return
  const next = props.segments.filter((_, idx) => idx !== i)
  editing.value = -1
  emit('save', next, { committed: true })
}

// ---- 拖动边缘改起止（对齐精修时间轴的右缘缩放）----
// 注意：松手时的终值必须由 drag.seg + 累积位移重算，不能读 props.segments ——
// 父组件对 transient 更新的处理是异步的，同一事件循环里 props 还是旧值，提交会丢改动。
let resize = null
function resizeNext(r, dt) {
  const s = r.seg
  return props.segments.map((cur, idx) => {
    if (idx !== r.i) return cur
    if (r.edge === 'start') {
      const start = Math.min(Math.max(0, (s.start || 0) + dt), (s.end || 0.1) - 0.1)
      return { ...cur, start: Math.round(start * 100) / 100 }
    }
    const end = Math.max((s.start || 0) + 0.1, (s.end || 0) + dt)
    return { ...cur, end: Math.round(end * 100) / 100 }
  })
}
function startResize(i, edge, e) {
  const rect = laneRef.value?.getBoundingClientRect()
  if (!rect) return
  resize = { i, edge, x0: e.clientX, dt: 0, seg: { ...props.segments[i] }, perPx: span.value / rect.width, moved: false }
  window.addEventListener('pointermove', onResizeMove)
  window.addEventListener('pointerup', onResizeUp, { once: true })
}

function onResizeMove(e) {
  if (!resize) return
  resize.dt = (e.clientX - resize.x0) * resize.perPx
  if (Math.abs(e.clientX - resize.x0) > 2) resize.moved = true
  emit('save', resizeNext(resize, resize.dt), { transient: true })
}

function onResizeUp() {
  window.removeEventListener('pointermove', onResizeMove)
  const r = resize
  resize = null
  if (r?.moved) {
    gestureEndAt = Date.now()
    emit('save', resizeNext(r, r.dt), { committed: true })
  }
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

function dragNext(d, dx) {
  const dur = Math.max(0.1, (d.seg.end || 0) - (d.seg.start || 0))
  const start = Math.max(0, (d.seg.start || 0) + dx)
  return props.segments.map((s, idx) =>
    idx === d.i ? { ...s, start, end: start + dur } : s
  )
}

function onMove(e) {
  if (!drag) return
  drag.dx = (e.clientX - drag.x0) * drag.perPx
  if (Math.abs(e.clientX - drag.x0) > 3) drag.moved = true
  emit('save', dragNext(drag, drag.dx), { transient: true })
}

function onUp() {
  window.removeEventListener('pointermove', onMove)
  const d = drag
  drag = null
  if (d?.moved) {
    gestureEndAt = Date.now()
    emit('save', dragNext(d, d.dx || 0), { committed: true })
  }
}
</script>

<style scoped>
.lane-wrap { display:flex; flex-direction:column; gap:4px; }
.lane { position:relative; height:46px; border-radius:10px; background:var(--bg-soft); border:1px solid var(--line); overflow:hidden; touch-action:none; }
.seg { position:absolute; top:4px; bottom:4px; border-radius:7px; background:#5E7FD8; color:#fff; font-size:10px; display:flex; align-items:center; overflow:hidden; padding:0 4px; cursor:text; user-select:none; }
.seg-text { white-space:nowrap; overflow:hidden; text-overflow:ellipsis; pointer-events:none; }
.seg.active { outline:2px solid var(--main); }
/* 左右缘微区域拖拽改起止；hover 时高亮提示 */
.rz { position:absolute; top:0; bottom:0; width:6px; cursor:ew-resize; }
.rz.l { left:0; border-radius:7px 0 0 7px; }
.rz.r { right:0; border-radius:0 7px 7px 0; }
.rz:hover { background:rgba(255,255,255,.35); }
.seg.editing { z-index:3; box-shadow:var(--shadow-hover); }
.seg-input { width:100%; height:100%; padding:0 4px; font-size:11px; border:none; border-radius:6px; background:#fff; color:var(--ink); }
.playhead { position:absolute; top:0; bottom:0; width:2px; background:var(--red); pointer-events:none; }
.lane-legend { display:flex; justify-content:space-between; gap:8px; }
</style>
