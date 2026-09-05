<template>
  <div class="timeline">
    <div v-if="!segments.length" class="empty small mt-6">暂无时间轴数据，点「分段」泳道右侧 ＋ 添加</div>
    <div v-for="lane in lanes" :key="lane.type" class="lane">
      <div class="lane-label">
        <span>{{ lane.label }}</span>
        <button class="add-lane" title="添加事件" @click="addEvent(lane.type)"><Icon name="plus" :size="11" /></button>
      </div>
      <div class="lane-track">
        <div class="playhead" :style="{ left: (currentTime || 0) * scale + 'px' }"></div>
        <div
          v-for="(block, i) in laneBlocks(lane)"
          :key="lane.type + '-' + i"
          class="block"
          :class="[block.cls, { selected: selected && selected.lane === lane.type && selected.si === block.si && selected.ei === block.ei }]"
          :style="{ left: block.left + 'px', width: block.width + 'px', background: block.color }"
          :title="block.title"
          @click="seek(block.time)"
          @pointerdown.stop="startPointer($event, lane, block, i)"
          @dblclick.stop="block.si != null && block.ei != null ? deleteEvent(block.si, block.ei) : null"
        >
          <span v-if="block.resizable" class="resize" @pointerdown.stop="startPointer($event, lane, block, i, true)"></span>
          {{ block.label }}
        </div>
      </div>
    </div>

    <div v-if="selected" class="props-panel card">
      <div class="row spread">
        <b>{{ selectedSeg ? '分段属性' : '事件属性' }}</b>
        <button class="mini-btn" @click="selected = null">关闭</button>
      </div>
      <template v-if="selectedSeg">
        <label>备注</label>
        <input v-model="selectedSeg.note" placeholder="分段备注">
        <div class="row">
          <div class="flex-1"><label>开始（秒）</label><input v-model.number="selectedSeg.start" type="number" step="0.1" min="0"></div>
          <div class="flex-1"><label>结束（秒）</label><input v-model.number="selectedSeg.end" type="number" step="0.1"></div>
        </div>
      </template>
      <template v-else-if="selectedEv">
        <label>类型</label>
        <input v-model="selectedEv.type" placeholder="事件类型">
        <div class="row">
          <div class="flex-1"><label>时间（秒）</label><input v-model.number="selectedEv.time" type="number" step="0.1" min="0"></div>
          <div class="flex-1"><label>时长（秒）</label><input v-model.number="selectedEv.duration" type="number" step="0.1" min="0.1"></div>
        </div>
        <label v-if="selectedEv.keyword !== undefined">keyword</label>
        <input v-if="selectedEv.keyword !== undefined" v-model="selectedEv.keyword" placeholder="贴纸关键词">
        <label v-if="selectedEv.name !== undefined">name</label>
        <input v-if="selectedEv.name !== undefined" v-model="selectedEv.name" placeholder="资源名">
        <div class="row mt-6">
          <button class="mini-btn" @click="duplicateSelected">复制</button>
          <button class="mini-btn danger" @click="deleteSelected">删除</button>
        </div>
      </template>
      <div class="row mt-8">
        <button class="btn btn-primary" @click="emitSave">保存修改</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { api } from '../api/client'
import { confirmDialog, promptDialog } from '../stores/dialog'
import Icon from './Icon.vue'
import { TIMELINE_COLORS } from '../utils'

const props = defineProps({
  dir: { type: String, required: true },
  currentTime: { type: Number, default: 0 }
})
const emit = defineEmits(['seek', 'save'])
const segments = ref([])
const scale = ref(60)
const duration = ref(60)
const selected = ref(null)
const selectedSeg = computed(() => {
  if (!selected.value || selected.value.ei !== -1) return null
  return segments.value[selected.value.si] || null
})
const selectedEv = computed(() => {
  if (!selected.value || selected.value.ei == null || selected.value.ei < 0) return null
  const seg = segments.value[selected.value.si]
  return seg && seg.events ? seg.events[selected.value.ei] || null : null
})

const LANE_DEFS = [
  { type: 'segments', label: '分段', color: TIMELINE_COLORS.segments, resizable: true },
  { type: 'camera', label: '运镜', color: TIMELINE_COLORS.camera, resizable: true },
  { type: 'sticker', label: '贴纸', color: TIMELINE_COLORS.sticker, resizable: true },
  { type: 'fx', label: '视觉特效', color: TIMELINE_COLORS.flash, types: ['shake','flash','blur','zoom_bounce','vignette','hue_shift','stutter','split','noise','pulse','color_shift'], resizable: true },
  { type: 'overlay', label: '覆盖层', color: TIMELINE_COLORS.overlay, resizable: true },
  { type: 'sfx', label: '音效', color: TIMELINE_COLORS.sfx, resizable: true }
]
const DEFAULT_DUR = { camera: 1.0, sticker: 2.0, overlay: 1.5, sfx: 0.5 }
const FX_TYPES = ['shake','flash','blur','zoom_bounce','vignette','hue_shift','stutter','split','noise','pulse','color_shift']

const lanes = LANE_DEFS

function getDuration() {
  return duration.value || 60
}

function labelFor(ev) {
  if (ev.type === 'sticker') return ev.keyword || ev.name || '贴纸'
  if (ev.type === 'camera') return ev.zoom || '运镜'
  if (ev.type === 'sfx') return ev.name || '音效'
  if (ev.type === 'overlay') return ev.overlay_type || '覆盖'
  return ev.type
}

function eventDuration(ev) {
  if (ev.duration != null) return Math.max(0.1, Number(ev.duration) || 0.1)
  return DEFAULT_DUR[ev.type] || 0.4
}

function laneBlocks(lane) {
  const out = []
  const segs = segments.value
  segs.forEach((seg, si) => {
    if (lane.type === 'segments') {
      out.push({
        si, ei: -1,
        time: seg.start,
        left: seg.start * scale.value,
        width: Math.max((seg.end - seg.start) * scale.value, 8),
        label: seg.note || ('分段 ' + (si + 1)),
        title: '分段 ' + Number(seg.start).toFixed(1) + 's → ' + Number(seg.end).toFixed(1) + 's',
        color: TIMELINE_COLORS.segments,
        resizable: true,
        cls: 'type-segments'
      })
      return
    }
    ;(seg.events || []).forEach((ev, ei) => {
      const type = ev.type || ''
      const types = lane.types || [lane.type]
      if (!types.includes(type)) return
      const dur = eventDuration(ev)
      const t = Number(ev.time) || 0
      out.push({
        si, ei,
        time: t,
        left: t * scale.value,
        width: Math.max(dur * scale.value, 10),
        label: labelFor(ev),
        title: type + ' @ ' + t.toFixed(1) + 's',
        color: TIMELINE_COLORS[type] || lane.color,
        resizable: true,
        cls: 'type-' + type
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
  segments.value.push({
    start: Math.round(start * 10) / 10,
    end: Math.round((start + 5) * 10) / 10,
    fx: 'low',
    note: '新分段',
    events: []
  })
}

async function addEvent(laneType = null) {
  // 分段泳道的 ＋ 直接新增分段；其余泳道添加对应类型事件
  if (laneType === 'segments') { addSegment(); return }
  if (!segments.value.length) addSegment()
  const type = laneType && laneType !== 'fx' && laneType !== 'overlay' && laneType !== 'segments'
    ? laneType
    : (laneType === 'overlay'
        ? 'overlay'
        : (await promptDialog(`要添加的事件类型（可选：${FX_TYPES.join(' / ')}）`, { title: '添加事件', value: 'shake' })) || 'shake')
  const tRaw = await promptDialog('事件发生时间（秒）', { title: '添加事件', value: '0' })
  if (tRaw === null) return
  const t = parseFloat(tRaw) || 0
  segments.value[0].events.push({ type, time: Math.round(t * 10) / 10, duration: DEFAULT_DUR[type] || 0.5 })
}

function deleteEvent(si, ei) {
  const seg = segments.value[si]
  if (!seg || !seg.events) return
  seg.events.splice(ei, 1)
  selected.value = null
}

function duplicateSelected() {
  const ev = selectedEv.value
  if (!ev) return
  const seg = segments.value[selected.value.si]
  if (!seg) return
  const copy = { ...ev, time: (Number(ev.time) || 0) + 0.1 }
  seg.events.push(copy)
  selected.value = { lane: selected.value.lane, si: selected.value.si, ei: seg.events.length - 1 }
}

async function deleteSelected() {
  if (selectedSeg.value) {
    const ok = await confirmDialog('将删除当前分段及其全部事件', { title: '删除当前分段？', danger: true })
    if (!ok) return
    segments.value.splice(selected.value.si, 1)
    selected.value = null
    return
  }
  if (selectedEv.value) {
    deleteEvent(selected.value.si, selected.value.ei)
  }
}

let drag = null

function startPointer(e, lane, block, index, isResize = false) {
  e.preventDefault()
  e.stopPropagation()
  selected.value = { lane: lane.type, si: block.si, ei: block.ei }
  const segs = segments.value
  const item = { lane: lane.type, si: block.si, ei: block.ei, seg: segs[block.si] }
  const ev = item.ei != null ? item.seg.events[item.ei] : null
  const startX = e.clientX
  const dur = getDuration()

  if (lane.type === 'segments') {
    if (isResize) {
      const startEnd = item.seg.end
      const minEnd = item.seg.start + 0.1
      drag = {
        move(evt) {
          const dx = evt.clientX - startX
          const ne = Math.max(minEnd, Math.min(dur, startEnd + dx / scale.value))
          item.seg.end = Math.round(ne * 100) / 100
        },
        up() { drag = null }
      }
    } else {
      const startTime = item.seg.start
      const len = item.seg.end - item.seg.start
      const origEvents = (item.seg.events || []).map(x => ({ ...x }))
      drag = {
        move(evt) {
          const dx = evt.clientX - startX
          const ns = Math.max(0, Math.min(dur - len, startTime + dx / scale.value))
          const delta = ns - startTime
          item.seg.start = Math.round(ns * 100) / 100
          item.seg.end = Math.round((startTime + len + delta) * 100) / 100
          ;(item.seg.events || []).forEach((x, i) => {
            const old = origEvents[i] ? origEvents[i].time : 0
            x.time = Math.round(Math.min(dur, Math.max(0, old + delta)) * 100) / 100
          })
        },
        up() { drag = null }
      }
    }
  } else if (ev) {
    const startTime = ev.time || 0
    if (isResize) {
      const startDur = eventDuration(ev)
      drag = {
        move(evt) {
          const dx = evt.clientX - startX
          const nd = Math.max(0.1, startDur + dx / scale.value)
          ev.duration = Math.round(Math.min(nd, Math.max(0.1, dur - startTime)) * 100) / 100
        },
        up() { drag = null }
      }
    } else {
      drag = {
        move(evt) {
          const dx = evt.clientX - startX
          const nt = Math.max(0, Math.min(dur, startTime + dx / scale.value))
          ev.time = Math.round(nt * 100) / 100
        },
        up() { drag = null }
      }
    }
  }

  if (drag) {
    const onMove = (evt) => { if (drag) drag.move(evt) }
    const onUp = () => { if (drag && drag.up) drag.up(); removeListeners() }
    function removeListeners() {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onUp)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onUp)
  }
}

onUnmounted(() => { drag = null })

onMounted(async () => {
  await loadData()
})

defineExpose({ save: emitSave, segments })

watch(() => props.dir, async () => {
  await loadData()
})

async function loadData() {
  try {
    const cfg = await api('/api/polish/project?dir=' + encodeURIComponent(props.dir))
    segments.value = cfg?.segments || []
    const last = cfg?.segments?.[cfg.segments.length - 1]
    duration.value = cfg?.duration || last?.end || 60
  } catch (e) {}
}
</script>

<style scoped>
.timeline { width:100%; overflow-x:auto; }
.lane { display:flex; height:34px; border-bottom:1px solid var(--line-soft); }
.lane-label { width:96px; flex:none; display:flex; align-items:center; justify-content:space-between; padding:0 8px; font-size:12px; color:var(--ink-dim); }
.add-lane { background:none; border:none; color:var(--ink-faint); cursor:pointer; font-size:15px; line-height:1; padding:2px 4px; }
.add-lane:hover { color:var(--main-deep); }
.lane-track { position:relative; flex:1; min-width:600px; }
.playhead { position:absolute; top:0; bottom:0; width:2px; background:var(--main); z-index:5; pointer-events:none; }
.block { position:absolute; top:5px; height:24px; border-radius:4px; color:#fff; font-size:11px; line-height:24px; padding:0 6px; white-space:nowrap; overflow:hidden; cursor:grab; user-select:none; }
.block.selected { outline:2px solid #fff; z-index:3; }
.resize { position:absolute; right:0; top:0; width:8px; height:100%; cursor:ew-resize; background:rgba(255,255,255,0.35); border-radius:0 4px 4px 0; }
.props-panel { margin-top:10px; background:var(--card); border:1px solid var(--line); border-radius:12px; padding:12px; }
.props-panel label { margin-top:6px; }
.props-panel .row { margin-top:4px; }
</style>
