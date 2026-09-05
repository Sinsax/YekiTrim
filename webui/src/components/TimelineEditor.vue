<template>
  <div class="timeline">
    <div class="row spread" style="margin-bottom:6px">
      <span class="muted small">点击色块可跳转</span>
      <span class="row">
        <button class="mini-btn" @click="addSegment">添加分段</button>
        <button class="mini-btn" @click="addEvent">添加事件</button>
        <button class="mini-btn" @click="removeLastSegment">删除最后分段</button>
        <button class="mini-btn" @click="emitSave">保存时间轴</button>
      </span>
    </div>
    <div v-if="!segments.length" class="muted">暂无时间轴数据</div>
    <div v-for="lane in lanes" :key="lane.type" class="lane">
      <div class="lane-label">{{ lane.label }}</div>
      <div class="lane-track">
        <div
          v-for="(block, i) in laneBlocks(lane)"
          :key="lane.type + '-' + i"
          class="block"
          :style="{ left: block.left + 'px', width: block.width + 'px', background: block.color }"
          :title="block.title"
          @click="seek(block.time)"
          @mousedown.stop="block.si != null ? startDrag(block.si, $event) : null"
          @dblclick.stop="block.si != null && block.ei != null ? deleteEvent(block.si, block.ei) : null"
        >{{ block.label }}</div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted } from 'vue'
import { api } from '../api/client'

const props = defineProps({ dir: { type: String, required: true } })
const emit = defineEmits(['seek'])
const segments = ref([])
const scale = ref(60)
const lanes = [
  { type: 'segments', label: '分段', color: '#525252' },
  { type: 'camera', label: '运镜', color: '#2563eb' },
  { type: 'sticker', label: '贴纸', color: '#7c3aed' },
  { type: 'fx', label: '视觉特效', color: '#d97706' },
  { type: 'sfx', label: '音效', color: '#6d28d9' }
]

function laneBlocks(lane) {
  const out = []
  segments.value.forEach((seg, si) => {
    if (lane.type === 'segments') {
      out.push({
        si,
        time: seg.start,
        left: seg.start * scale.value,
        width: Math.max((seg.end - seg.start) * scale.value, 10),
        label: seg.note || '分段',
        title: seg.start + 's → ' + seg.end + 's'
      })
      return
    }
    ;(seg.events || []).forEach((ev, ei) => {
      const type = ev.type || ''
      const match = lane.type === 'camera' ? type === 'camera'
        : lane.type === 'sticker' ? type === 'sticker'
        : lane.type === 'sfx' ? type === 'sfx'
        : ['shake','flash','blur','zoom_bounce','vignette','hue_shift','stutter','split','noise','pulse','color_shift'].includes(type)
      if (!match) return
      const dur = ev.duration || 0.5
      out.push({
        si,
        ei,
        time: seg.start + (ev.time || 0),
        left: (seg.start + (ev.time || 0)) * scale.value,
        width: Math.max(dur * scale.value, 10),
        label: type,
        title: type + ' @ ' + (seg.start + (ev.time || 0)).toFixed(1) + 's'
      })
    })
  })
  return out
}

function seek(t) {
  emit('seek', t)
}

function emitSave() {
  emit('save', segments.value)
}

function addSegment() {
  const last = segments.value[segments.value.length - 1]
  const start = last ? last.end : 0
  segments.value.push({ start: Math.round(start * 10) / 10, end: Math.round((start + 5) * 10) / 10, fx: 'low', note: '新分段', events: [] })
}

function removeLastSegment() {
  if (!segments.value.length) return
  if (!confirm('删除最后一段？')) return
  segments.value.pop()
}

function addEvent() {
  if (!segments.value.length) {
    addSegment()
  }
  const type = prompt('事件类型', 'shake') || 'shake'
  const t = parseFloat(prompt('时间（秒）', '0')) || 0
  segments.value[0].events.push({ type, time: Math.round(t * 10) / 10, duration: 0.5 })
}

function deleteEvent(si, ei) {
  const seg = segments.value[si]
  if (!seg || !seg.events) return
  seg.events.splice(ei, 1)
}

let dragSegIndex = null
let dragStartX = 0
let dragStartTime = 0

function startDrag(si, e) {
  dragSegIndex = si
  dragStartX = e.clientX
  dragStartTime = segments.value[si].start
  window.addEventListener('mousemove', onDragMove)
  window.addEventListener('mouseup', endDrag)
}

function onDragMove(e) {
  if (dragSegIndex == null) return
  const delta = (e.clientX - dragStartX) / scale.value
  const seg = segments.value[dragSegIndex]
  const newStart = Math.max(0, dragStartTime + delta)
  const dur = seg.end - seg.start
  seg.start = Math.round(newStart * 10) / 10
  seg.end = Math.round((newStart + dur) * 10) / 10
}

function endDrag() {
  dragSegIndex = null
  window.removeEventListener('mousemove', onDragMove)
  window.removeEventListener('mouseup', endDrag)
}

onUnmounted(() => {
  window.removeEventListener('mousemove', onDragMove)
  window.removeEventListener('mouseup', endDrag)
})

onMounted(async () => {
  try {
    const cfg = await api('/api/polish/project?dir=' + encodeURIComponent(props.dir))
    segments.value = cfg?.segments || []
  } catch (e) {}
})
</script>

<style scoped>
.timeline { width:100%; overflow-x:auto; }
.lane { display:flex; height:34px; border-bottom:1px solid var(--line-soft); }
.lane-label { width:90px; flex:none; display:flex; align-items:center; padding:0 8px; font-size:12px; color:var(--ink-dim); }
.lane-track { position:relative; flex:1; min-width:600px; }
.block { position:absolute; top:5px; height:24px; border-radius:4px; color:#fff; font-size:11px; line-height:24px; padding:0 6px; white-space:nowrap; overflow:hidden; cursor:pointer; }
</style>
