<template>
  <div class="proj" :class="{ open: expanded }">
    <div class="proj-head" @click="toggle">
      <span class="tw">▶</span>
      <span class="nm" :title="p.name">{{ p.name }}</span>
      <span class="src-badge" :class="badgeClass">{{ sourceBadge(p) }}</span>
      <span class="chip badge" :class="progressChip">{{ currentLabel }}</span>
      <button class="mini-btn del" title="删除项目（只删 vtrim.toml）" @click.stop="emit('remove', p)">×</button>
    </div>
    <div v-show="expanded" class="proj-steps">
      <div
        v-for="(s, i) in STEPS"
        :key="s.key"
        class="snode"
        :class="{ done: nodeDone(s.key), cur: progress === i + 1, viewing: viewing === s.key }"
        @click="emit('open', { pid: p.id, step: s.key })"
      >
        <span class="ic">{{ nodeDone(s.key) ? '✓' : '' }}</span>{{ s.name }}
        <span v-if="runningPct(s.key) != null" class="pct">{{ runningPct(s.key) }}%</span>
      </div>
      <div class="snode size-node"><span class="size-tag">占用 {{ humanBytes(p.bytes) }}</span></div>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { STEPS, stageInfo } from '../../utils'
import { humanBytes, sourceBadge } from '../../stores/project'
import { cacheOf, polishedCountOf, uncutEventsOf } from '../../stores/workbench'

const props = defineProps({
  p: { type: Object, required: true },
  expanded: { type: Boolean, default: false },
  viewing: { type: String, default: '' },
  /** 该项目正在跑的任务 `{kind, current, total}`（没有则 null） */
  running: { type: Object, default: null }
})
const emit = defineEmits(['open', 'remove', 'toggle'])

const detail = computed(() => {
  const c = cacheOf(props.p.id)
  if (!c.status) return null
  return { clips: c.clips, uncutEvents: uncutEventsOf(props.p.id), polishedCount: polishedCountOf(props.p.id) }
})

const info = computed(() => stageInfo(props.p, detail.value))
const progress = computed(() => info.value.progress)
const currentLabel = computed(() => {
  const name = STEPS[progress.value - 1]?.name || '预处理'
  const pct = runningPct(STEPS[progress.value - 1]?.key)
  return pct != null ? `${name} ${pct}%` : name
})

const progressChip = computed(() => (progress.value >= 3 ? 'g' : progress.value === 2 ? 'p' : 'a'))
const badgeClass = computed(() =>
  props.p.source?.kind === 'recorder' ? 'src-rec' : props.p.source?.kind === 'bilibili' ? 'src-bili' : 'src-local'
)

function nodeDone(key) {
  return !!info.value.done[key]
}

/** 任务与阶段的对应（pipeline/stage_* → 预处理，cut* → 切片，polish_* → 精修）。 */
function stageOfKind(kind) {
  if (!kind) return null
  if (kind.startsWith('polish_')) return 'polish'
  if (kind === 'cut_all' || kind === 'cut_one') return 'slices'
  if (kind === 'pipeline' || kind.startsWith('stage_') || kind === 'correct') return 'prepare'
  return null
}

function runningPct(step) {
  if (!props.running) return null
  if (stageOfKind(props.running.kind) !== step) return null
  const { current = 0, total = 0 } = props.running
  return total > 0 ? Math.round((current * 100) / total) : 0
}

function toggle() {
  emit('toggle', props.p.id)
}
</script>

<style scoped>
/* 只保留基准样式里没有的三处：行内删除按钮、当前查看高亮、"占用"行 */
.proj-head .del { padding:1px 7px; opacity:0; }
.proj-head:hover .del { opacity:1; }
.badge { font-size:10px; }
.snode.viewing { background:var(--bg-soft); }
.snode.size-node { color:var(--ink-faint); font-size:11px; cursor:default; }
.snode.size-node:hover { background:transparent; color:var(--ink-faint); }
</style>
