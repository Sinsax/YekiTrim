<template>
  <div class="sub-list" :style="{ maxHeight: height }">
    <div v-if="!segments.length" class="muted small sub-empty">暂无字幕</div>
    <div v-for="(seg, i) in segments" :key="keyOf(seg)" class="sub-row" @click="emit('play', seg)">
      <textarea :value="seg.text" @input="onTextInput(seg, $event.target.value)" rows="1" class="sub-input" @click.stop></textarea>
      <div class="sub-meta" :class="{ 'has-end': showEnd }">
        <span class="time-cell" :title="editableTime ? '点击编辑开始时间' : ''" @click.stop="startEdit(i, 'start')">
          <input v-if="editing && editing.i === i && editing.k === 'start'" ref="editInput" v-model="draft"
                 class="time-input" type="number" step="0.1" @click.stop
                 @keyup.enter="commit(i, 'start')" @blur="commit(i, 'start')">
          <span v-else class="time">{{ fmtClock(seg.start) }}</span>
        </span>
        <span v-if="showEnd" class="time-cell end" title="点击编辑结束时间" @click.stop="startEdit(i, 'end')">
          <input v-if="editing && editing.i === i && editing.k === 'end'" ref="editInput" v-model="draft"
                 class="time-input" type="number" step="0.1" @click.stop
                 @keyup.enter="commit(i, 'end')" @blur="commit(i, 'end')">
          <span v-else class="time">{{ fmtClock(seg.end) }}</span>
        </span>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, nextTick } from 'vue'
import { fmtClock } from '../utils'

const props = defineProps({
  segments: { type: Array, default: () => [] },
  height: { type: String, default: '55vh' },
  // 扩展能力（精修编辑器用；项目字幕页不传则保持原行为）
  showEnd: { type: Boolean, default: false },      // 时间行右下角显示结束时间
  editableTime: { type: Boolean, default: false }  // 点击开始/结束时间可编辑（秒）
})
const emit = defineEmits(['play'])

/**
 * 稳定的行 key（**不要退回下标**）。
 *
 * 下标 key + `sortByStart()`（改开始/结束时间后会重排）会让"第 i 行"换一条字幕：
 * 焦点留在原 DOM 节点上，光标与正在编辑的文本会落到**另一条**字幕上（表里不一）。
 * 这里用 WeakMap 按**对象身份**发号：重排只换顺序、不换身份 ⇒ key 跟着对象走；
 * 文本输入是原地改字段（对象不变），所以打字时不会重挂、不会丢焦点。
 */
const segIds = new WeakMap()
let nextSegId = 1
function keyOf(seg) {
  if (!seg || typeof seg !== 'object') return 'x' + String(seg)
  let id = segIds.get(seg)
  if (!id) { id = 's' + nextSegId++; segIds.set(seg, id) }
  return id
}

function sortByStart() {
  props.segments.sort((a, b) => (Number(a.start) || 0) - (Number(b.start) || 0))
}

function onTextInput(seg, value) {
  seg.text = value
  // 预览引擎优先读 html 字段；普通文本编辑时必须同步，否则预览仍显示旧字幕
  seg.html = value
}

const editing = ref(null) // { i, k }
const draft = ref('')
const editInput = ref(null)

function startEdit(i, k) {
  if (!props.editableTime) return
  const seg = props.segments[i]
  if (!seg) return
  editing.value = { i, k }
  draft.value = String(Math.round((Number(seg[k]) || 0) * 100) / 100)
  nextTick(() => {
    const el = Array.isArray(editInput.value) ? editInput.value[0] : editInput.value
    el && el.focus && el.focus()
  })
}

function commit(i, k) {
  if (!editing.value) return
  const seg = props.segments[i]
  if (seg) {
    let n = Number(draft.value)
    if (!Number.isFinite(n) || n < 0) n = 0
    seg[k] = Math.round(n * 100) / 100
    sortByStart()
  }
  editing.value = null
}
</script>

<style scoped>
.sub-list { overflow-y:auto; border:1px solid var(--line); border-radius:var(--r-sm); padding:8px; }
.sub-row { display:flex; flex-direction:column; gap:2px; padding:5px 6px; border-bottom:1px dashed var(--line-soft); cursor:pointer; transition:background .12s; }
.sub-row:hover { background:var(--bg-soft); }
.sub-row:last-child { border-bottom:none; }
.sub-meta { display:flex; align-items:center; }
.sub-meta.has-end { justify-content:space-between; }
.time { font-size:10px; line-height:1.4; color:var(--ink-faint); }
.time-cell { flex:none; cursor:default; }
.time-cell.end { text-align:right; }
.sub-meta.has-end .time-cell { cursor:pointer; }
.sub-meta.has-end .time-cell:hover .time { color:var(--main-deep); }
.time-input { width:76px; padding:1px 6px; font-size:11px; }
.sub-input { width:100%; font-size:12px; resize:none; }
.sub-empty { padding:8px 6px; }
</style>
