<template>
  <div class="sub-editor">
    <!-- 字幕属性：**常驻横排**在时间轴上方（用户要求：不要右侧竖面板）。
         未选中时字段禁用 + 一句提示，位置固定不跳、不占轨道宽度。
         操作与精修预览的「字幕属性」同款：内容 / 起止秒数 / 当前设为入点·出点 / 拆分 / 删除。 -->
    <div class="props-bar" :class="{ 'is-empty': !selSeg }">
      <b class="pb-title">字幕属性</b>

      <div class="pb-item pb-text">
        <label>内容</label>
        <input
          ref="textRef"
          :value="draftText"
          :disabled="!selSeg"
          placeholder="这条字幕的文本"
          @input="onTextInput"
        >
      </div>

      <div class="pb-item pb-num">
        <label>开始</label>
        <input :value="fmtNum(selStart)" :disabled="!selSeg" type="number" step="0.01" min="0" @change="onTimeChange('start', $event)">
      </div>
      <div class="pb-item pb-num">
        <label>结束</label>
        <input :value="fmtNum(selEnd)" :disabled="!selSeg" type="number" step="0.01" @change="onTimeChange('end', $event)">
      </div>

      <div class="pb-actions">
        <button class="mini-btn" :disabled="!selSeg" title="把当前播放时间设为这条字幕的开始" @click="setStartToCurrent">当前设为入点</button>
        <button class="mini-btn" :disabled="!selSeg" title="把当前播放时间设为这条字幕的结束" @click="setEndToCurrent">当前设为出点</button>
        <button class="mini-btn" :disabled="!selSeg || !canSplit" title="以当前播放时间把这条字幕拆成两条" @click="splitAtCurrent">拆分</button>
        <button class="mini-btn" :disabled="!selSeg" title="跳到这条字幕的开始" @click="seekSelected">定位</button>
        <button class="mini-btn danger" :disabled="!selSeg" title="删除这条字幕" @click="deleteSelected">删除</button>
      </div>

      <span class="pb-meta muted small">
        <template v-if="selSeg">{{ fmtSec(selStart) }} → {{ fmtSec(selEnd) }} · 时长 {{ fmtSec(Math.max(0, selEnd - selStart)) }}</template>
        <template v-else>未选中：点轨道上的字幕块，在这里改文本与入点 / 出点</template>
      </span>
    </div>

    <!-- 滚动式字幕轨道：像素刻度 + 横向滚动（与精修时间轴同一套结构：左侧 sticky 标签列 + 右侧可滚轨道），
         不再把整场（可能 3 小时）压进一个百分比泳道 —— 那样每句只有零点几像素，没法编辑。 -->
    <div
      ref="bodyRef"
      class="sub-body"
      @wheel.passive="markUserScroll"
      @pointerdown="markUserScroll"
      @scroll.passive="onScroll"
    >
      <div class="sub-labels">
        <div class="sub-label">
          <span>字幕</span>
          <button class="lane-add" title="在当前位置新增一条字幕" @click="addAtCurrent"><Icon name="plus" :size="11" /></button>
        </div>
        <div class="sub-label ruler-label">时间</div>
      </div>
      <div class="sub-tracks" :style="{ width: trackWidth + 'px' }">
        <div class="s-lane" @pointerdown.self="seekFromEvent">
          <!-- 切片标记（入点→出点）在轨道上高亮，和下方「标记入点/出点」联动 -->
          <div v-if="markWidth >= 1" class="mark-band" :style="{ left: markLeft + 'px', width: markWidth + 'px' }"></div>
          <div
            v-for="b in shownBlocks"
            :key="b.i"
            class="block"
            :class="{ selected: b.i === selected, active: b.i === activeIndex }"
            :style="{ left: b.left + 'px', width: b.width + 'px' }"
            :title="b.title"
            @click.stop="selectSeg(b.i)"
            @dblclick.stop="deleteSeg(b.i)"
            @pointerdown.stop="startDrag(b.i, $event)"
          >
            <span class="rz l" title="拖动调整开始时间" @pointerdown.stop="startResize(b.i, 'start', $event)"></span>
            <span class="seg-text">{{ b.text }}</span>
            <span class="rz r" title="拖动调整结束时间" @pointerdown.stop="startResize(b.i, 'end', $event)"></span>
          </div>
          <div class="playhead" :style="{ left: playheadLeft + 'px' }"></div>
        </div>
        <div class="s-lane ruler-lane" @pointerdown="onRulerPointerDown">
          <div v-for="tk in timeTicks" :key="tk.t" class="ruler-tick" :style="{ left: tk.left + 'px' }">
            <span class="ruler-time">{{ tk.label }}</span>
          </div>
          <div class="playhead" :style="{ left: playheadLeft + 'px' }"></div>
        </div>
      </div>
    </div>

    <div class="track-foot">
      <span class="muted small">{{ segments.length }} 段字幕 · 共 {{ fmtSec(span) }}</span>
      <span class="muted small hide-narrow">点块在上面编辑 · 拖动平移 · 拖左右缘改起止 · 双击删除 · 点标尺跳转</span>
      <span class="spacer"></span>
      <button class="mini-btn" :disabled="scaleIdx <= 0" title="缩小（同屏显示更长）" @click="zoom(-1)">－</button>
      <span class="muted small mono zoom-num">{{ scale }} px/s</span>
      <button class="mini-btn" :disabled="scaleIdx >= SCALES.length - 1" title="放大（看清每一句）" @click="zoom(1)">＋</button>
      <button class="mini-btn" title="缩到最小档（整场结构一目了然；长录播不会真的挤进一屏）" @click="fitAll">全览</button>
    </div>
  </div>
</template>

<script setup>
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import Icon from '../Icon.vue'
import { confirmDialog } from '../../stores/dialog'
import { toast } from '../../stores/toast'
import { fmtClock } from '../../utils'

const props = defineProps({
  segments: { type: Array, default: () => [] },
  duration: { type: Number, default: 0 },
  currentTime: { type: Number, default: 0 },
  /** 切片标记：入点 / 出点（秒），在轨道上高亮 */
  markIn: { type: Number, default: 0 },
  markOut: { type: Number, default: 0 }
})
const emit = defineEmits(['save', 'seek'])

/** 缩放档位（像素/秒）：0.5 = 整场全览，120 = 逐字级 */
const SCALES = [0.5, 1, 2, 3, 5, 8, 12, 20, 30, 45, 60, 90, 120]
const scaleIdx = ref(4)
const scale = computed(() => SCALES[scaleIdx.value])
const userZoomed = ref(false)   // 用户手动缩放过 → 不再自动改档

const bodyRef = ref(null)
const textRef = ref(null)
const bodyW = ref(900)
const scrollLeft = ref(0)

const selected = ref(-1)
const draftText = ref('')

const maxEnd = computed(() => props.segments.reduce((m, s) => Math.max(m, Number(s.end) || 0), 0))
// 轨道总长只增不减：拖动最后一条字幕时 duration 会跟着缩，若直接用它做总长，
// 轨道宽度会边拖边抖（maxEnd 变小 → 总长变小 → 时间↔像素换算整体变化）。
const hardSpan = ref(0)
watch(() => Math.max(props.duration || 0, maxEnd.value, 1), (v) => {
  if (v > hardSpan.value) hardSpan.value = v
}, { immediate: true })
const span = computed(() => Math.max(hardSpan.value, 1))
const trackWidth = computed(() => Math.max(span.value * scale.value, 320))
const playheadLeft = computed(() => Math.max(0, Math.min(trackWidth.value, (Number(props.currentTime) || 0) * scale.value)))

const markLeft = computed(() => Math.max(0, (Number(props.markIn) || 0) * scale.value))
const markWidth = computed(() => Math.max(0, ((Number(props.markOut) || 0) - (Number(props.markIn) || 0)) * scale.value))

const round2 = (v) => Math.round((Number(v) || 0) * 100) / 100
function fmtSec(v) {
  const n = Number(v)
  return (Number.isFinite(n) ? n : 0).toFixed(2) + 's'
}
function fmtNum(v) {
  return String(round2(v))
}

// ── 可见区虚拟化：3 小时转写可能有几千条，全量渲染会卡；只渲染视口 ±2s 的块 ──
const shownBlocks = computed(() => {
  const sc = scale.value
  const from = scrollLeft.value / sc - 2
  const to = (scrollLeft.value + bodyW.value) / sc + 2
  const out = []
  props.segments.forEach((s, i) => {
    const start = Number(s.start) || 0
    const end = Math.max(start, Number(s.end) || start)
    if (end < from || start > to) return
    const text = String(s.text || '').replace(/\s+/g, ' ').trim()
    out.push({
      i,
      left: start * sc,
      width: Math.max((end - start) * sc, 2),
      text,
      title: `${fmtClock(start)} - ${fmtClock(end)}｜${text || '(空)'}`
    })
  })
  return out
})

const activeIndex = computed(() => {
  const t = Number(props.currentTime) || 0
  return props.segments.findIndex((s) => t >= (Number(s.start) || 0) && t < (Number(s.end) || 0))
})

// 刻度间距按缩放选，保证相邻刻度 ≥ 72px；同样只画可见区（3 小时全画会有上千个刻度）
const TICK_STEPS = [0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600]
const timeTicks = computed(() => {
  const sc = scale.value
  const step = TICK_STEPS.find((s) => s * sc >= 72) || 3600
  const from = Math.max(0, Math.floor(scrollLeft.value / sc / step) * step)
  const to = Math.min(span.value, (scrollLeft.value + bodyW.value) / sc)
  const out = []
  for (let t = from; t <= to + 1e-6; t = Math.round((t + step) * 1000) / 1000) {
    out.push({ t, left: t * sc, label: step < 1 ? fmtSec(t) : fmtClock(t) })
  }
  return out
})

// ── 滚动：跟随播放头（用户刚滚过 / 刚拖过指针的 2s 内不抢） ──────
let userScrollTimer = null
/** 用户接管视口（滚轮、拖动标尺指针）：2 秒内不自动跟随播放头。
 *  拖动期间每次 pointermove 都会刷新这个窗口 —— 所以整个拖动过程窗口都不会动。 */
const userScrolling = ref(false)
function markUserScroll() {
  userScrolling.value = true
  if (userScrollTimer) clearTimeout(userScrollTimer)
  userScrollTimer = setTimeout(() => { userScrolling.value = false; userScrollTimer = null }, 2000)
}
function onScroll() {
  const el = bodyRef.value
  if (!el) return
  scrollLeft.value = el.scrollLeft
  if (el.clientWidth) bodyW.value = el.clientWidth
}
function scrollToTime(t, ratio = 0.3) {
  const el = bodyRef.value
  if (!el) return
  const anchor = Math.max(0, el.clientWidth * ratio)
  el.scrollLeft = Math.max(0, (Number(t) || 0) * scale.value - anchor)
  scrollLeft.value = el.scrollLeft
}
watch(() => props.currentTime, (t) => {
  if (userScrolling.value) return
  scrollToTime(t)
})

// ── 缩放 ────────────────────────────────────────────────────
/** 默认：一屏约 2 分钟（短视频直接整段可见）；用户按过 ＋/－/全览 后不再自动改。 */
function autoScale() {
  const w = Math.max(320, (bodyRef.value?.clientWidth || bodyW.value) - 12)
  const secs = span.value <= 150 ? span.value : 120
  const target = w / Math.max(20, secs)
  let idx = 0
  for (let i = 0; i < SCALES.length; i++) if (SCALES[i] <= target) idx = i
  scaleIdx.value = idx
}
function setScale(next) {
  const el = bodyRef.value
  const old = scale.value
  // 以视口中心的时间为锚点换档，缩放时不会跳到别处
  const centerTime = el ? (el.scrollLeft + el.clientWidth / 2) / old : (Number(props.currentTime) || 0)
  scaleIdx.value = Math.max(0, Math.min(SCALES.length - 1, next))
  nextTick(() => {
    const b = bodyRef.value
    if (!b) return
    b.scrollLeft = Math.max(0, centerTime * scale.value - b.clientWidth / 2)
    scrollLeft.value = b.scrollLeft
  })
}
function zoom(dir) {
  userZoomed.value = true
  setScale(scaleIdx.value + dir)
}
function fitAll() {
  userZoomed.value = true
  const w = Math.max(320, (bodyRef.value?.clientWidth || bodyW.value) - 12)
  const target = Math.min(SCALES[SCALES.length - 1], Math.max(SCALES[0], w / span.value))
  let idx = 0
  for (let i = 0; i < SCALES.length; i++) if (SCALES[i] <= target) idx = i
  setScale(idx)
}

// ── 选中与属性面板 ──────────────────────────────────────────
const selSeg = computed(() => (selected.value >= 0 ? props.segments[selected.value] || null : null))
const selStart = computed(() => round2(selSeg.value?.start))
const selEnd = computed(() => round2(selSeg.value?.end))
const canSplit = computed(() => {
  const t = Number(props.currentTime) || 0
  return !!selSeg.value && t > selStart.value + 0.01 && t < selEnd.value - 0.01
})

// 外部（重新转写 / 保存回报）改了文本才回填，避免打字过程中被自己的保存覆盖
watch(() => selSeg.value?.text, (t) => {
  const next = String(t ?? '')
  if (next !== draftText.value) draftText.value = next
})

/** 用 patch 覆盖当前选中段，emit 给父组件（父组件负责防抖落盘）。 */
function emitPatch(patch, opts = {}) {
  const i = selected.value
  const s = props.segments[i]
  if (!s) return
  const next = props.segments.slice()
  next[i] = { ...s, ...patch }
  emit('save', next, opts)
}

function selectIndex(i) {
  selected.value = i
  draftText.value = String(props.segments[i]?.text ?? '')
}

/** 拖拽/缩放结束的那一次 click 不该当成“选中”。用「一次性吞掉」而不是时间窗：
 *  时间窗会连带吞掉用户紧接着点另一个块的 legit click（实测：拖完 200ms 内点块选不中）。 */
let suppressClick = false
function selectSeg(i) {
  if (suppressClick) { suppressClick = false; return }
  selectIndex(i)
}

function onTextInput(e) {
  draftText.value = e.target.value
  emitPatch({ text: draftText.value })
}

function onTimeChange(field, e) {
  const el = e.target
  const v = Number(el.value)
  if (!Number.isFinite(v)) { el.value = fmtNum(field === 'start' ? selStart.value : selEnd.value); return }
  if (field === 'start') {
    const ns = round2(Math.max(0, Math.min(v, selEnd.value - 0.05)))
    emitPatch({ start: ns }, { committed: true })
    nextTick(() => { el.value = fmtNum(selStart.value) })
  } else {
    const ne = round2(Math.max(selStart.value + 0.05, v))
    emitPatch({ end: ne }, { committed: true })
    nextTick(() => { el.value = fmtNum(selEnd.value) })
  }
}

function seekSelected() {
  if (selSeg.value) emit('seek', selStart.value)
}

function setStartToCurrent() {
  if (!selSeg.value) return
  const t = round2(Math.max(0, Number(props.currentTime) || 0))
  if (t >= selEnd.value) { toast('当前时间已在出点之后，无法设为入点', 'err'); return }
  emitPatch({ start: t }, { committed: true })
}

function setEndToCurrent() {
  if (!selSeg.value) return
  const t = round2(Math.max(0, Number(props.currentTime) || 0))
  if (t <= selStart.value) { toast('当前时间在入点之前，无法设为出点', 'err'); return }
  emitPatch({ end: t }, { committed: true })
}

function splitAtCurrent() {
  const s = selSeg.value
  if (!s) return
  if (!canSplit.value) { toast('当前播放时间不在这条字幕中间，无法拆分', 'err'); return }
  const t = round2(Number(props.currentTime) || 0)
  const i = selected.value
  const next = props.segments.slice()
  next[i] = { ...s, end: t }
  next.splice(i + 1, 0, { ...s, start: t, end: selEnd.value })
  selected.value = i + 1
  draftText.value = String(s.text ?? '')
  emit('save', next, { committed: true })
}

async function deleteSeg(i) {
  const s = props.segments[i]
  if (!s) return
  const ok = await confirmDialog(`删除这条字幕？「${String(s.text || '').slice(0, 24)}」`, { title: '删除字幕', danger: true })
  if (!ok) return
  const next = props.segments.filter((_, idx) => idx !== i)
  if (selected.value === i) { selected.value = -1; draftText.value = '' }
  else if (selected.value > i) selected.value -= 1
  emit('save', next, { committed: true })
}

function deleteSelected() {
  if (selected.value >= 0) deleteSeg(selected.value)
}

function addAtCurrent() {
  const t = round2(Math.max(0, Number(props.currentTime) || 0))
  const next = props.segments.slice()
  let i = next.findIndex((s) => (Number(s.start) || 0) > t)
  if (i < 0) i = next.length
  next.splice(i, 0, { start: t, end: round2(t + 3), text: '' })
  selected.value = i
  draftText.value = ''
  emit('save', next, { committed: true })
  nextTick(() => textRef.value?.focus?.())
}

// ── 拖拽：平移时间 / 拖左右缘改起止 ──────────────────────────
let drag = null
let resize = null

function startDrag(i, e) {
  if (e.button !== 0) return
  const s = props.segments[i]
  if (!s) return
  selectIndex(i)      // 拖动中就把属性面板切到这条（与精修时间轴一致）
  suppressClick = false
  drag = { i, x0: e.clientX, dx: 0, moved: false, seg: { ...s } }
  window.addEventListener('pointermove', onDragMove)
  window.addEventListener('pointerup', onDragUp)
  window.addEventListener('pointercancel', onDragUp)
}

/** 注意：终值用 drag.seg + 累积位移重算，不读 props.segments ——
 *  父组件对 transient 更新的处理是异步的，同一事件循环里 props 还是旧值。 */
function dragNext(d) {
  const dur = Math.max(0.05, (Number(d.seg.end) || 0) - (Number(d.seg.start) || 0))
  const start = Math.max(0, Math.min((Number(d.seg.start) || 0) + (d.dx || 0), Math.max(0, span.value - dur)))
  const next = props.segments.slice()
  next[d.i] = { ...next[d.i], start: round2(start), end: round2(start + dur) }
  return next
}
function onDragMove(e) {
  if (!drag) return
  drag.dx = (e.clientX - drag.x0) / scale.value
  if (Math.abs(e.clientX - drag.x0) > 3) drag.moved = true
  emit('save', dragNext(drag), { transient: true })
}
function onDragUp() {
  window.removeEventListener('pointermove', onDragMove)
  window.removeEventListener('pointerup', onDragUp)
  window.removeEventListener('pointercancel', onDragUp)
  const d = drag
  drag = null
  if (d?.moved) {
    suppressClick = true
    emit('save', dragNext(d), { committed: true })
  }
}

function startResize(i, edge, e) {
  if (e.button !== 0) return
  const s = props.segments[i]
  if (!s) return
  selectIndex(i)
  suppressClick = false
  resize = { i, edge, x0: e.clientX, dt: 0, moved: false, seg: { ...s } }
  window.addEventListener('pointermove', onResizeMove)
  window.addEventListener('pointerup', onResizeUp)
  window.addEventListener('pointercancel', onResizeUp)
}
function resizeNext(r) {
  const next = props.segments.slice()
  const cur = next[r.i]
  if (!cur) return next
  if (r.edge === 'start') {
    const start = Math.max(0, Math.min((Number(r.seg.start) || 0) + r.dt, (Number(r.seg.end) || 0.1) - 0.05))
    next[r.i] = { ...cur, start: round2(start) }
  } else {
    const end = Math.max((Number(r.seg.start) || 0) + 0.05, (Number(r.seg.end) || 0) + r.dt)
    next[r.i] = { ...cur, end: round2(end) }
  }
  return next
}
function onResizeMove(e) {
  if (!resize) return
  resize.dt = (e.clientX - resize.x0) / scale.value
  if (Math.abs(e.clientX - resize.x0) > 2) resize.moved = true
  emit('save', resizeNext(resize), { transient: true })
}
function onResizeUp() {
  window.removeEventListener('pointermove', onResizeMove)
  window.removeEventListener('pointerup', onResizeUp)
  window.removeEventListener('pointercancel', onResizeUp)
  const r = resize
  resize = null
  if (r?.moved) {
    suppressClick = true
    emit('save', resizeNext(r), { committed: true })
  }
}

// ── 点空白 / 拖标尺跳转 ─────────────────────────────────────
/** 拖动（或点击）指针只改时间，**不搬窗口**：像素换算出的时间先夹进当前可见区间，
 *  再夹到轨道范围内。想看更远的地方用下方的横向滚动条滚过去 ——
 *  窗口只由滚动条（和播放跟随，见上）移动，拖指针时它一动不动。 */
function seekAt(clientX, el) {
  if (!el) return
  const rect = el.getBoundingClientRect()
  let t = (clientX - rect.left) / scale.value
  const b = bodyRef.value
  if (b && b.clientWidth) {
    const from = b.scrollLeft / scale.value
    const to = (b.scrollLeft + b.clientWidth) / scale.value
    t = Math.min(Math.max(t, from), Math.max(from, to))
  }
  emit('seek', round2(Math.max(0, Math.min(span.value, t))))
}
function seekFromEvent(e) {
  seekAt(e.clientX, e.currentTarget)
}
function onRulerPointerDown(e) {
  if (e.button !== 0) return
  e.preventDefault()
  const el = e.currentTarget
  markUserScroll()   // 拖动中（以及松手后 2s）视口不跟随播放头
  seekAt(e.clientX, el)
  const onMove = (ev) => { markUserScroll(); seekAt(ev.clientX, el) }
  const onUp = () => {
    window.removeEventListener('pointermove', onMove)
    window.removeEventListener('pointerup', onUp)
    window.removeEventListener('pointercancel', onUp)
  }
  window.addEventListener('pointermove', onMove)
  window.addEventListener('pointerup', onUp)
  window.addEventListener('pointercancel', onUp)
}

watch(() => props.segments.length, (n) => {
  if (!userZoomed.value) autoScale()
  if (!n) { selected.value = -1; draftText.value = '' }
  else if (selected.value >= n) selected.value = n - 1
})

onMounted(() => {
  if (bodyRef.value?.clientWidth) bodyW.value = bodyRef.value.clientWidth
  autoScale()
  nextTick(() => scrollToTime(Number(props.currentTime) || 0))
})
onUnmounted(() => {
  window.removeEventListener('pointermove', onDragMove)
  window.removeEventListener('pointerup', onDragUp)
  window.removeEventListener('pointercancel', onDragUp)
  window.removeEventListener('pointermove', onResizeMove)
  window.removeEventListener('pointerup', onResizeUp)
  window.removeEventListener('pointercancel', onResizeUp)
  if (userScrollTimer) clearTimeout(userScrollTimer)
  drag = null
  resize = null
})
</script>

<style scoped>
.sub-editor { width:100%; }

/* 字幕属性：常驻横排（贴在时间轴上方）。窄屏自动换行，字段顺序 = 内容 → 起止 → 动作 → 读数 */
.props-bar { display:flex; align-items:center; gap:8px; flex-wrap:wrap; margin-bottom:6px;
  padding:7px 10px; border:1px solid var(--line); border-radius:10px; background:var(--bg-soft); }
.props-bar.is-empty { border-style:dashed; }
.pb-title { font-size:12.5px; font-weight:700; flex:none; }
.pb-item { display:flex; align-items:center; gap:5px; min-width:0; }
.pb-item label { margin:0; font-size:11.5px; color:var(--ink-dim); flex:none; }
.pb-item input { width:86px; padding:5px 8px; font-size:12px; }
.pb-item input:disabled { background:var(--bg); color:var(--ink-faint); }
.pb-text { flex:1 1 220px; }
.pb-text input { width:100%; }
.pb-actions { display:flex; align-items:center; gap:6px; flex:none; }
.pb-meta { flex:0 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }

/* 滚动容器：左侧标签列 sticky，右侧轨道按 时长×缩放 像素展开（结构同精修时间轴） */
.sub-body { display:flex; overflow-x:auto; overflow-y:hidden; border:1px solid var(--line); border-radius:10px; background:var(--bg-soft); }
.sub-body::-webkit-scrollbar { height:9px; }
.sub-body::-webkit-scrollbar-thumb { background:var(--line); border-radius:999px; }
.sub-body::-webkit-scrollbar-track { background:transparent; }
.sub-labels { flex:none; position:sticky; left:0; z-index:6; background:var(--card); box-shadow:2px 0 8px rgba(90,60,70,.10); }
.sub-label { width:74px; height:26px; box-sizing:border-box; display:flex; align-items:center; justify-content:space-between;
  padding:0 8px; font-size:11.5px; color:var(--ink-dim); border-bottom:1px solid var(--line-soft); }
.sub-tracks { flex:none; min-width:0; }
.s-lane { position:relative; height:26px; border-bottom:1px solid var(--line-soft); }
.ruler-label { height:20px; font-size:10px; }
.ruler-lane { height:20px; border-bottom:none; cursor:col-resize; }
.ruler-tick { position:absolute; top:0; bottom:0; width:1px; background:rgba(0,0,0,.14); }
.ruler-time { position:absolute; top:1px; left:4px; font-size:9px; color:var(--ink-dim); white-space:nowrap; pointer-events:none; }
/* 字幕块：与精修时间轴的字幕块同色同字重（半透明蓝 + 深色粗体 + 白晕） */
.block { position:absolute; top:3px; height:20px; border-radius:4px; background:rgba(59,130,246,.35); color:#0f172a;
  font-weight:700; font-size:10px; line-height:20px; padding:0 6px; white-space:nowrap; overflow:hidden;
  cursor:grab; user-select:none; z-index:2; text-shadow:0 0 2px #fff, 0 0 2px #fff; }
.block.selected { outline:2px solid var(--main); z-index:3; }
.block.active { outline:1px solid var(--main-deep); }
.seg-text { pointer-events:none; }
.rz { position:absolute; top:0; bottom:0; width:6px; max-width:45%; cursor:ew-resize; z-index:1; }
.rz.l { left:0; border-radius:4px 0 0 4px; }
.rz.r { right:0; border-radius:0 4px 4px 0; }
.rz:hover { background:rgba(255,255,255,.45); }
.playhead { position:absolute; top:0; bottom:0; width:2px; background:var(--red); z-index:5; pointer-events:none; }
.mark-band { position:absolute; top:0; bottom:0; background:rgba(246,130,184,.30);
  border-left:1px solid var(--main); border-right:1px solid var(--main); z-index:1; pointer-events:none; }
.lane-add { background:none; border:none; color:var(--ink-faint); cursor:pointer; line-height:1; padding:2px 4px; display:flex; }
.lane-add:hover { color:var(--main-deep); }
.track-foot { display:flex; align-items:center; gap:6px; flex-wrap:wrap; margin-top:4px; }
.spacer { flex:1; }
.zoom-num { min-width:52px; text-align:center; }
.hide-narrow { display:inline; }

@media (max-width:1080px) {
  .hide-narrow { display:none; }
}
</style>
