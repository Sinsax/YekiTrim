<template>
  <div ref="rootRef" class="timeline" tabindex="0">
    <!-- 头部一行：左侧是调用方给的标题/提示（slot），右侧是去静音（含撤销）+ 状态。
         这样页面不用再单独放一个卡片标题行 —— 时间轴本来就占竖高（用户要求精简）。 -->
    <div class="tl-head">
      <div class="tl-head-title"><slot name="head" /></div>
      <div class="row" style="gap:8px">
        <template v-if="segments.length">
          <button
            class="mini-btn"
            :class="{ selected: silenceApplied }"
            :disabled="silenceBusy"
            :title="silenceApplied
              ? '移除自动加入的静音屏蔽，恢复手动添加的屏蔽区间'
              : '自动检测片段里的长停顿，加入「屏蔽」轨道；导出时剪掉，原始视频不动'"
            @click="toggleSilence"
          >
            <Icon name="sparkles" :size="11" />
            {{ silenceBusy ? '处理中…' : (silenceApplied ? '撤销去静音' : '去静音') }}
          </button>
          <span class="muted small">
            {{ silenceApplied ? `已自动屏蔽 ${silenceCount} 段静音` : '一键跳过片段里的长停顿' }}
          </span>
        </template>
      </div>
    </div>
    <!-- 属性面板：用 Teleport 投到页面给的挂载点（PolishView 把它放在视频左侧，与时间轴分离）。
         选中状态与全部编辑逻辑仍留在这个组件里，页面只决定它出现在哪儿。
         挂载点必须在 TimelineEditor 之前渲染出来（同一个父模板里靠前即可）。 -->
    <Teleport :to="propsTarget">
      <!-- 属性面板：点击时间轴上的块后在这里编辑 -->
      <div class="props-panel card">
        <div class="row spread">
          <b>{{ !selected ? '属性' : (selectedMute ? '屏蔽属性' : (selectedSub ? '字幕属性' : (selectedSeg ? '分段属性' : '事件属性'))) }}</b>
          <button v-if="selected" class="mini-btn" @click="selected = null">关闭</button>
        </div>
        <div v-if="!selected" class="props-empty muted small">点击时间轴上的分段 / 字幕 / 屏蔽 / 事件块，在这里编辑属性</div>
        <template v-else>
      <template v-if="selectedSeg">
        <label>备注</label>
        <input v-model="selectedSeg.note" placeholder="分段备注">
        <div class="row">
          <div class="flex-1"><label>开始（秒）</label><input v-model.number="selectedSeg.start" type="number" step="0.1" min="0"></div>
          <div class="flex-1"><label>结束（秒）</label><input v-model.number="selectedSeg.end" type="number" step="0.1"></div>
        </div>
        <div class="row mt-6">
          <button class="mini-btn danger" @click="deleteSelected">删除</button>
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
      <template v-else-if="selectedSub">
        <label>字幕内容</label>
        <textarea v-model="selectedSub.text" rows="2"></textarea>
        <div class="row">
          <div class="flex-1"><label>开始（秒）</label><input v-model.number="selectedSub.start" type="number" step="0.01" min="0"></div>
          <div class="flex-1"><label>结束（秒）</label><input v-model.number="selectedSub.end" type="number" step="0.01"></div>
        </div>
        <div class="row mt-6">
          <button class="mini-btn" title="以当前播放时间作为字幕开始" @click="setSelectedStartToCurrent('subtitles')">将当前时间设为入点</button>
          <button class="mini-btn" title="以当前播放时间作为字幕结束" @click="endSelectedSubtitleAtCurrent">将当前时间设为出点</button>
          <button class="mini-btn" @click="splitSelectedSubtitle">按当前时间拆分</button>
          <button class="mini-btn danger" @click="deleteSelectedSubtitle">删除</button>
        </div>
      </template>
      <template v-else-if="selectedMute">
        <div class="row">
          <div class="flex-1"><label>开始（秒）</label><input v-model.number="selectedMute.start" type="number" step="0.01" min="0"></div>
          <div class="flex-1"><label>结束（秒）</label><input v-model.number="selectedMute.end" type="number" step="0.01"></div>
        </div>
        <div class="row mt-6">
          <button class="mini-btn" title="以当前播放时间作为屏蔽开始" @click="setSelectedStartToCurrent('mutes')">将当前时间设为入点</button>
          <button class="mini-btn" title="以当前播放时间作为屏蔽结束" @click="endSelectedMuteAtCurrent">将当前时间设为出点</button>
          <button class="mini-btn" @click="splitSelectedMute">按当前时间拆分</button>
          <button class="mini-btn danger" @click="deleteSelectedMute">删除</button>
        </div>
      </template>
      <div class="row mt-8">
        <button class="btn btn-primary" @click="selectedSub ? saveSubtitleChanges() : (selectedMute ? saveMuteChanges() : emitSave())">保存修改</button>
      </div>
        </template>
      </div>
    </Teleport>

    <div v-if="!segments.length" class="empty small mt-6">暂无时间轴数据，请先运行 LLM 自动生成分段</div>
    <template v-else>
      <div ref="bodyRef" class="timeline-body" @wheel.passive="markUserScroll" @pointerdown="markUserScroll">
      <div class="timeline-labels">
        <div v-for="lane in lanes" :key="'label-' + lane.type" class="lane-label">
          <span>{{ lane.label }}</span>
          <div class="lane-adds">
            <button v-if="lane.type !== 'segments'" class="add-lane" title="添加事件" @click="addEvent(lane.type)"><Icon name="plus" :size="11" /></button>
            <button v-else class="add-lane mute-add" title="添加屏蔽区间" @click="addMuteAtCurrent"><Icon name="scissors" :size="11" /></button>
          </div>
        </div>
        <div class="lane-label ruler-label">时间</div>
      </div>
      <div ref="tracksRef" class="timeline-tracks">
        <div v-for="lane in lanes" :key="'track-' + lane.type" class="lane">
          <div class="lane-track" :style="{ width: trackWidth + 'px' }">
            <svg v-if="lane.type === 'subtitles'" class="wave-layer" :width="trackWidth" height="26">
              <rect v-for="(bar, bi) in waveBars" :key="bi" :x="bar.x" :y="bar.y" :width="bar.w" :height="bar.h" fill="rgba(37,99,235,0.7)" />
            </svg>
            <div class="playhead" :style="{ left: (currentTime || 0) * scale + 'px' }"></div>
            <div
              v-for="(block, i) in laneBlocks(lane)"
              :key="lane.type + '-' + i"
              class="block"
              :class="[block.cls, { selected: isBlockSelected(lane, block) }]"
              :style="{ left: block.left + 'px', width: block.width + 'px', background: block.color }"
              :title="block.title"
              @click="onBlockClick(lane, block)"
              @pointerdown.stop="startPointer($event, lane, block, i)"
              @dblclick.stop="onBlockDblClick(block)"
            >
              <span v-if="block.resizable" class="resize" @pointerdown.stop="startPointer($event, lane, block, i, true)"></span>
              {{ block.label }}
            </div>
          </div>
        </div>
        <div class="lane ruler-lane">
          <div ref="rulerTrackRef" class="lane-track ruler-track" :style="{ width: trackWidth + 'px' }"
               @pointerdown="onRulerPointerDown">
            <div v-for="(tk, i) in timeTicks" :key="'tick' + i" class="ruler-tick" :style="{ left: tk.left + 'px' }">
              <span class="ruler-time">{{ tk.label }}</span>
            </div>
            <div class="playhead ruler-playhead" :style="{ left: (currentTime || 0) * scale + 'px' }"></div>
          </div>
        </div>
      </div>
      </div>
    </template>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { api } from '../api/client'
import { confirmDialog, promptDialog } from '../stores/dialog'
import { toast } from '../stores/toast'
import Icon from './Icon.vue'
import { TIMELINE_COLORS } from '../utils'

const props = defineProps({
  dir: { type: String, required: true },
  currentTime: { type: Number, default: 0 },
  /** 属性面板的 Teleport 挂载点（CSS 选择器）；默认由 PolishView 渲染在视频左侧 */
  propsTarget: { type: String, default: '#tl-props-host' }
})
const emit = defineEmits(['seek', 'save', 'toggle', 'subtitleSaved'])
const rootRef = ref(null)
const tracksRef = ref(null)
const bodyRef = ref(null)
const rulerTrackRef = ref(null)
const segments = ref([])
const subtitles = ref([])
const muteRanges = ref([])
const loudness = ref([])
const scale = ref(60)
const duration = ref(60)
const trackWidth = computed(() => Math.max((duration.value || 60) * scale.value, 600))
const timeTicks = computed(() => {
  const dur = duration.value || 60
  const step = dur > 300 ? 60 : dur > 120 ? 30 : dur > 60 ? 10 : 5
  const out = []
  for (let t = 0; t <= dur + 0.001; t += step) {
    out.push({ t, left: t * scale.value, label: t.toFixed(0) + 's' })
  }
  return out
})
const waveBars = computed(() => {
  const pts = loudness.value || []
  const bars = []
  for (let i = 0; i < pts.length; i++) {
    const p = pts[i]
    const t = Number(p.t) || 0
    if (t > (duration.value || 60)) break
    const db = Math.max(-60, Math.min(0, Number(p.db) || -60))
    const h = Math.max(2, ((db + 60) / 60) * 22)
    const w = Math.max(1, scale.value * 0.02)
    // 向下对齐：所有柱子从泳道底部向上生长（泳道 26px，留 4px 余量）
    bars.push({ x: t * scale.value, y: 26 - h, w, h })
  }
  return bars
})
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
const selectedSub = computed(() => {
  if (!selected.value || selected.value.lane !== 'subtitles') return null
  const i = selected.value.subIndex
  return i != null ? (subtitles.value[i] || null) : null
})
const selectedMute = computed(() => {
  if (!selected.value || selected.value.muteIndex == null) return null
  const i = selected.value.muteIndex
  return muteRanges.value[i] || null
})

const LANE_DEFS = [
  { type: 'segments', label: '分段 / 屏蔽', color: TIMELINE_COLORS.segments, resizable: true },
  { type: 'subtitles', label: '字幕', color: 'rgba(59,130,246,0.35)', resizable: false },
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
  if (lane.type === 'subtitles') {
    subtitles.value.forEach((sub, i) => {
      const start = Number(sub.start) || 0
      const end = Number(sub.end) || start
      const text = (sub.text || sub.html || '').replace(/<[^>]+>/g, ' ').trim()
      out.push({
        si: null,
        ei: null,
        subIndex: i,
        time: start,
        left: start * scale.value,
        width: Math.max((end - start) * scale.value, 10),
        label: text.length > 18 ? text.slice(0, 18) + '…' : text,
        title: '字幕 ' + Number(start).toFixed(2) + 's → ' + Number(end).toFixed(2) + 's ' + text,
        color: '#3b82f6',
        resizable: false,
        cls: 'type-subtitles'
      })
    })
    return out
  }
  if (lane.type === 'segments') {
    segments.value.forEach((seg, si) => {
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
    })
    muteRanges.value.forEach((m, i) => {
      const start = Number(m.start) || 0
      const end = Number(m.end) || start
      out.push({
        si: null,
        ei: null,
        muteIndex: i,
        time: start,
        left: start * scale.value,
        width: Math.max((end - start) * scale.value, 10),
        label: '屏蔽',
        title: '屏蔽 ' + Number(start).toFixed(2) + 's → ' + Number(end).toFixed(2) + 's',
        color: '#ef4444',
        resizable: false,
        cls: 'type-mutes'
      })
    })
    return out
  }
  const segs = segments.value
  segs.forEach((seg, si) => {
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

function isBlockSelected(lane, block) {
  const s = selected.value
  if (!s || s.lane !== lane.type) return false
  if (block.muteIndex != null) return s.muteIndex === block.muteIndex
  if (block.subIndex != null) return s.subIndex === block.subIndex
  return s.si === block.si && s.ei === block.ei
}

function seek(t) {
  emit('seek', t)
}

function emitSave() {
  emit('save', { segments: segments.value, mute_ranges: muteRanges.value })
}


function onBlockClick(lane, block) {
  if (block.subIndex != null) {
    // 点击字幕块只选中，不跳时间，避免破坏“当前时间”操作
    selected.value = { lane: lane.type, subIndex: block.subIndex }
    return
  }
  if (block.muteIndex != null) {
    selected.value = { lane: lane.type, muteIndex: block.muteIndex }
    return
  }
  seek(block.time)
}

function onBlockDblClick(block) {
  if (block.muteIndex != null) { deleteMuteAt(block.muteIndex); return }
  if (block.si != null && block.ei != null && block.ei >= 0) deleteEvent(block.si, block.ei)
}

function addSubtitleAtCurrent() {
  const start = Math.max(0, Number(props.currentTime) || 0)
  const sub = { start: Math.round(start * 100) / 100, end: Math.round((start + 3) * 100) / 100, text: '', html: '' }
  subtitles.value.push(sub)
  subtitles.value.sort((a, b) => (Number(a.start) || 0) - (Number(b.start) || 0))
  const idx = subtitles.value.indexOf(sub)
  selected.value = { lane: 'subtitles', subIndex: idx }
  saveSubtitleChanges()
}

// ── 快捷操作「去静音」────────────────────────────────────────────
// 由后端检测长停顿并写入「屏蔽」轨道（silence_ranges 标记自动加入的部分），
// 撤销时只移除这些区间，手动添加的屏蔽不受影响。
const silenceApplied = ref(false)
const silenceCount = ref(0)
const silenceBusy = ref(false)

function applySilenceState(cfg) {
  const recorded = Array.isArray(cfg?.silence_ranges) ? cfg.silence_ranges : []
  silenceApplied.value = recorded.length > 0
  silenceCount.value = recorded.length
}

async function toggleSilence() {
  if (silenceBusy.value) return
  silenceBusy.value = true
  const action = silenceApplied.value ? 'clear' : 'apply'
  try {
    const res = await api('/api/polish/silence', {
      method: 'POST',
      body: JSON.stringify({ dir: props.dir, action })
    })
    await loadData()
    emit('muteSaved', muteRanges.value)
    if (action === 'clear') {
      toast(`已撤销去静音，移除 ${res?.removed ?? 0} 段静音屏蔽`)
    } else if (res?.applied) {
      toast(`去静音：${res?.added ?? 0} 段静音已加入屏蔽轨道（共 ${res?.total ?? 0} 段）`)
    } else {
      toast('未检测到 2s 以上的长停顿')
    }
  } catch (e) {
    toast('去静音失败：' + e.message, 'err')
  } finally {
    silenceBusy.value = false
  }
}

function addMuteAtCurrent() {
  const start = Math.max(0, Number(props.currentTime) || 0)
  const m = { start: Math.round(start * 100) / 100, end: Math.round((start + 3) * 100) / 100 }
  muteRanges.value.push(m)
  muteRanges.value.sort((a, b) => (Number(a.start) || 0) - (Number(b.start) || 0))
  const idx = muteRanges.value.indexOf(m)
  selected.value = { lane: 'segments', muteIndex: idx }
  saveMuteChanges()
}

async function saveMuteChanges({ silent = false } = {}) {
  muteRanges.value.forEach(m => {
    const start = Math.max(0, Number(m.start) || 0)
    let end = Number(m.end)
    if (!Number.isFinite(end)) end = start + 3
    m.start = Math.round(start * 100) / 100
    m.end = Math.round(Math.max(start + 0.05, end) * 100) / 100
  })
  muteRanges.value.sort((a, b) => a.start - b.start)
  if (selectedMute.value) {
    const idx = muteRanges.value.indexOf(selectedMute.value)
    if (idx >= 0) selected.value = { lane: 'segments', muteIndex: idx }
  }
  try {
    await api('/api/polish/project', { method: 'PUT', body: JSON.stringify({ dir: props.dir, mute_ranges: muteRanges.value }) })
    emit('muteSaved', muteRanges.value)
    if (!silent) toast('屏蔽区间已保存')
  } catch (e) {
    toast('屏蔽区间保存失败：' + e.message, 'err')
  }
}

async function saveSubtitleChanges({ silent = false } = {}) {
  if (!subtitles.value.length) return
  subtitles.value.forEach(s => {
    s.start = Math.round((Number(s.start) || 0) * 100) / 100
    s.end = Math.round((Number(s.end) || (s.start + 3)) * 100) / 100
    if (!s.html) s.html = s.text || ''
  })
  subtitles.value.sort((a, b) => (Number(a.start) || 0) - (Number(b.start) || 0))
  if (selectedSub.value) {
    const idx = subtitles.value.indexOf(selectedSub.value)
    if (idx >= 0) selected.value = { lane: 'subtitles', subIndex: idx }
  }
  try {
    await api('/api/polish/subtitles', { method: 'PUT', body: JSON.stringify({ dir: props.dir, segments: subtitles.value }) })
    emit('subtitleSaved')
    if (!silent) toast('字幕已保存')
  } catch (e) {
    toast('字幕保存失败：' + e.message, 'err')
  }
}

async function deleteSelectedSubtitle() {
  if (!selectedSub.value) return
  const ok = await confirmDialog('删除这条字幕？', { title: '删除字幕', danger: true })
  if (!ok) return
  const idx = selected.value.subIndex
  if (idx >= 0) subtitles.value.splice(idx, 1)
  selected.value = null
  saveSubtitleChanges()
}

async function deleteSelectedMute() {
  if (!selectedMute.value) return
  const ok = await confirmDialog('删除这条屏蔽区间？', { title: '删除屏蔽区间', danger: true })
  if (!ok) return
  const idx = selected.value.muteIndex
  if (idx >= 0) muteRanges.value.splice(idx, 1)
  selected.value = null
  saveMuteChanges()
}

function deleteMuteAt(idx) {
  if (idx < 0 || idx >= muteRanges.value.length) return
  muteRanges.value.splice(idx, 1)
  selected.value = null
  saveMuteChanges()
}

async function endSelectedMuteAtCurrent() {
  const m = selectedMute.value
  if (!m) return
  const t = Math.max(0, Number(props.currentTime) || 0)
  if (t <= (Number(m.start) || 0)) {
    toast('当前时间必须大于屏蔽开始时间', 'err')
    return
  }
  m.end = Math.round(t * 100) / 100
  saveMuteChanges()
}

async function splitSelectedMute() {
  const m = selectedMute.value
  if (!m) return
  const t = Number(props.currentTime) || 0
  if (t <= m.start || t >= m.end) {
    toast('当前时间不在该屏蔽范围内，无法拆分', 'err')
    return
  }
  const second = { start: Math.round(t * 100) / 100, end: m.end }
  m.end = Math.round(t * 100) / 100
  muteRanges.value.push(second)
  muteRanges.value.sort((a, b) => (Number(a.start) || 0) - (Number(b.start) || 0))
  saveMuteChanges()
}

async function endSelectedSubtitleAtCurrent() {
  const sub = selectedSub.value
  if (!sub) return
  const t = Math.max(0, Number(props.currentTime) || 0)
  if (t <= (Number(sub.start) || 0)) {
    toast('当前时间必须大于字幕开始时间', 'err')
    return
  }
  sub.end = Math.round(t * 100) / 100
  saveSubtitleChanges()
}

async function splitSelectedSubtitle() {
  const sub = selectedSub.value
  if (!sub) return
  const t = Number(props.currentTime) || 0
  if (t <= sub.start || t >= sub.end) {
    toast('当前时间不在该字幕范围内，无法拆分', 'err')
    return
  }
  const second = { start: Math.round(t * 100) / 100, end: sub.end, text: sub.text || '', html: sub.text || '' }
  sub.end = Math.round(t * 100) / 100
  subtitles.value.push(second)
  subtitles.value.sort((a, b) => (Number(a.start) || 0) - (Number(b.start) || 0))
  saveSubtitleChanges()
}

// 在选中块开头前插入一段：以当前时间为新块开始；当前时间 >= 选中块开始（含已经跑到 end 之后）时静默失败
// 入点：把选中块的开始改到当前时间；当前时间 >= 块结束时间时静默失败
function setSelectedStartToCurrent(laneType) {
  const t = Math.max(0, Number(props.currentTime) || 0)
  if (laneType === 'subtitles') {
    const sub = selectedSub.value
    if (!sub) return
    const end = Number(sub.end) || 0
    if (t >= end) return
    sub.start = Math.round(t * 100) / 100
    saveSubtitleChanges()
    return
  }
  if (laneType === 'mutes') {
    const m = selectedMute.value
    if (!m) return
    const end = Number(m.end) || 0
    if (t >= end) return
    m.start = Math.round(t * 100) / 100
    saveMuteChanges()
  }
}

async function addEvent(laneType = null) {
  // 字幕泳道直接新增字幕；分段由 LLM 自动生成，不再允许手动新增分段
  if (laneType === 'subtitles') { addSubtitleAtCurrent(); return }
  if (!segments.value.length) {
    toast('暂无分段，请先运行 LLM 生成分段', 'err')
    return
  }
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
    emitSave()
    return
  }
  if (selectedEv.value) {
    deleteEvent(selected.value.si, selected.value.ei)
    emitSave()
  }
}

let drag = null

function startPointer(e, lane, block, index, isResize = false) {
  if (block.si == null) return
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

let rulerDragging = false
let userScrollTimer = null
const userScrolling = ref(false)

function markUserScroll() {
  userScrolling.value = true
  if (userScrollTimer) clearTimeout(userScrollTimer)
  userScrollTimer = setTimeout(() => {
    userScrolling.value = false
    userScrollTimer = null
  }, 2000)
}

function onTimelineKey(e) {
  if (e.code !== 'Space') return
  const el = rootRef.value
  if (!el) return
  const r = el.getBoundingClientRect()
  const x = e.clientX
  const y = e.clientY
  if (x < r.left || x > r.right || y < r.top || y > r.bottom) return
  const tag = (e.target?.tagName || '').toUpperCase()
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || e.target?.isContentEditable) return
  e.preventDefault()
  emit('toggle')
}

function seekFromRuler(e) {
  const track = rulerTrackRef.value
  if (!track || !scale.value) return
  const rect = track.getBoundingClientRect()
  const x = e.clientX - rect.left
  const t = Math.min(duration.value || 60, Math.max(0, x / scale.value))
  emit('seek', Math.round(t * 100) / 100)
}

function onRulerPointerDown(e) {
  if (e.button !== 0) return
  e.preventDefault()
  seekFromRuler(e)
  rulerDragging = true
  const onMove = (ev) => seekFromRuler(ev)
  const onUp = () => {
    rulerDragging = false
    window.removeEventListener('pointermove', onMove)
    window.removeEventListener('pointerup', onUp)
    window.removeEventListener('pointercancel', onUp)
  }
  window.addEventListener('pointermove', onMove)
  window.addEventListener('pointerup', onUp)
  window.addEventListener('pointercancel', onUp)
}

onUnmounted(() => {
  window.removeEventListener('keydown', onTimelineKey)
  if (userScrollTimer) clearTimeout(userScrollTimer)
  if (subAutoTimer) clearTimeout(subAutoTimer)
  if (muteAutoTimer) clearTimeout(muteAutoTimer)
  if (rulerDragging) {
    // listeners are removed by onUp normally; this is a safety path
    rulerDragging = false
  }
  drag = null
})

onMounted(async () => {
  window.addEventListener('keydown', onTimelineKey)
  await loadData()
})

watch(() => props.currentTime, (t) => {
  if (userScrolling.value) return
  const scroller = bodyRef.value
  if (!scroller || !scale.value) return
  const left = (t || 0) * scale.value
  // 播放时始终把播放头稳定在可视区约 30% 位置，避免只在边缘才跳
  const anchor = Math.max(0, scroller.clientWidth * 0.3)
  scroller.scrollLeft = Math.max(0, left - anchor)
})
let subAutoTimer = null
let muteAutoTimer = null
watch(() => {
  const s = selectedSub.value
  return s ? [s.start, s.end, s.text, s.html] : null
}, () => {
  if (!selectedSub.value) return
  if (subAutoTimer) clearTimeout(subAutoTimer)
  subAutoTimer = setTimeout(() => {
    if (selectedSub.value) saveSubtitleChanges({ silent: true })
  }, 600)
}, { deep: true })

watch(() => {
  const m = selectedMute.value
  return m ? [m.start, m.end] : null
}, () => {
  if (!selectedMute.value) return
  if (muteAutoTimer) clearTimeout(muteAutoTimer)
  muteAutoTimer = setTimeout(() => {
    if (selectedMute.value) saveMuteChanges({ silent: true })
  }, 600)
}, { deep: true })


defineExpose({ save: emitSave, segments })

watch(() => props.dir, async () => {
  await loadData()
})

async function loadData() {
  try {
    const cfg = await api('/api/polish/project?dir=' + encodeURIComponent(props.dir) + '&preview=1')
    segments.value = cfg?.segments || []
    muteRanges.value = Array.isArray(cfg?.mute_ranges) ? cfg.mute_ranges.map(m => ({ start: Number(m.start) || 0, end: Number(m.end) || 0 })) : []
    applySilenceState(cfg)
    const last = cfg?.segments?.[cfg.segments.length - 1]
    duration.value = cfg?.duration || last?.end || 60
    loudness.value = cfg?.loudness_curve || []
  } catch (e) {}
  try {
    const subRes = await api('/api/polish/subtitles?dir=' + encodeURIComponent(props.dir))
    subtitles.value = subRes?.segments || []
  } catch (e) { subtitles.value = [] }
}
</script>

<style scoped>
.timeline { width:100%; }
/* 头部一行（标题/提示 + 去静音）；原来标题在卡片头、去静音单独占一行，共省 ~70px */
.tl-head { display:flex; align-items:center; justify-content:space-between; gap:10px;
  flex-wrap:wrap; margin-bottom:6px; }
.tl-head-title { display:flex; align-items:center; gap:8px; flex-wrap:wrap; min-width:0; }
.tl-head-title :deep(b) { font-size:13px; }
.timeline-body { flex:1; min-width:0; display:flex; overflow-x:auto; align-items:stretch; }
.timeline-labels { flex:none; position:sticky; left:0; z-index:6; display:flex; flex-direction:column; background:var(--card); box-shadow:2px 0 8px rgba(90,60,70,.10); }
.timeline-tracks { flex:none; min-width:0; }
/* 泳道 34 → 26px（7 条泳道省 ~56px）；块 24 → 20px */
.lane { height:26px; border-bottom:1px solid var(--line-soft); position:relative;
  display:flex; align-items:center; gap:10px; padding:0 12px; box-sizing:border-box; }
.lane-label { width:96px; height:26px; box-sizing:border-box; flex:none; display:flex; align-items:center; justify-content:space-between; padding:0 8px; font-size:11.5px; color:var(--ink-dim); border-bottom:1px solid var(--line-soft); }
.add-lane { background:none; border:none; color:var(--ink-faint); cursor:pointer; font-size:14px; line-height:1; padding:2px 4px; }
.add-lane:hover { color:var(--main-deep); }
.lane-adds { display:flex; align-items:center; gap:2px; }
.mute-add { color:#ef4444; }
.mute-add:hover { color:#b91c1c; }
.lane-track { position:relative; height:26px; }
.wave-layer { position:absolute; top:0; left:0; pointer-events:none; opacity:1; z-index:1; }
.playhead { position:absolute; top:0; bottom:0; width:2px; background:var(--main); z-index:5; pointer-events:none; }
.block { position:absolute; top:3px; height:20px; border-radius:4px; color:#fff; font-size:10px; line-height:20px; padding:0 6px; white-space:nowrap; overflow:hidden; cursor:grab; user-select:none; z-index:2; }
.block.selected { outline:2px solid #fff; z-index:3; }
 .resize { position:absolute; right:0; top:0; width:8px; height:100%; cursor:ew-resize; background:rgba(255,255,255,0.35); border-radius:0 4px 4px 0; }
.ruler-label { height:20px; font-size:10px; }
.ruler-lane { height:20px; border-bottom:none; }
.ruler-track { height:20px; cursor:col-resize; }
.ruler-tick { position:absolute; top:0; bottom:0; width:1px; background:rgba(0,0,0,0.14); }
.ruler-time { position:absolute; top:1px; left:4px; font-size:9px; color:var(--ink-dim); white-space:nowrap; pointer-events:none; }
.ruler-playhead { position:absolute; top:0; bottom:0; width:2px; background:var(--main); z-index:5; pointer-events:none; }
.block.type-subtitles { background:rgba(59,130,246,0.35) !important; color:#0f172a; font-weight:700; opacity:1; text-shadow:0 0 2px #fff, 0 0 2px #fff; }
.block.type-mutes { background:rgba(239,68,68,0.42) !important; color:#7f1d1d; font-weight:700; }
/* 属性面板已 Teleport 到页面（视频左侧），这些样式跟着元素走（scoped 属性保留） */
.props-panel { width:232px; flex:none; background:var(--card); border:1px solid var(--line); border-radius:12px; padding:10px; overflow:auto; }
.props-empty { border:1.5px dashed var(--line); border-radius:10px; padding:12px 10px; }
.props-panel label { margin-top:6px; }
.props-panel .row { margin-top:4px; }
</style>
