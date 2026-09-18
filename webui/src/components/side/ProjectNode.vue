<template>
  <div class="proj" :class="{ open: expanded, on: current }">
    <div class="proj-head" @click="toggle">
      <span class="tw">▶</span>
      <span class="nm" :title="p.name">{{ p.name }}</span>
      <span class="src-badge" :class="badgeClass" :title="sourceLabel(p)">{{ sourceBadge(p) }}</span>
      <span class="chip badge" :class="progressChip">{{ currentLabel }}</span>
      <button class="mini-btn del" title="从列表移除（不动目录里的任何文件）" @click.stop="emit('remove', p)">×</button>
    </div>
    <div v-show="expanded" class="proj-steps">
      <!-- 只标两件事：**当前位置**（选中的项目 + 该项目正在看的阶段）与**已完成阶段的打勾**。
           打勾**只有「预处理」**会拿到（见 nodeDone）：切片/精修是持续活动、没有终态。
           原来还有一档 `cur`（progress === i+1，走到的最远阶段）的粉底高亮，用户要求删掉 ——
           它和"我在哪"是两套信号，同时亮反而看不出哪个是当前。 -->
      <div
        v-for="s in STEPS"
        :key="s.key"
        class="snode"
        :class="{ done: nodeDone(s.key), viewing: current && viewing === s.key }"
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
import { humanBytes, sourceBadge, sourceLabel } from '../../stores/project'
import { cacheOf, polishedCountOf, uncutEventsOf } from '../../stores/workbench'

const props = defineProps({
  p: { type: Object, required: true },
  expanded: { type: Boolean, default: false },
  /** 这是不是当前正在看的那个项目（侧栏高亮用；折叠轨那边也是同一判据） */
  current: { type: Boolean, default: false },
  /** 当前路由处在哪个阶段（`prepare`/`slices`/`polish`）。
   *  ⚠️ 它**不加项目限定** —— `SideNav` 对每个项目都传同一个值，所以高亮时必须再 `&& current`，
   *     否则多个项目同时展开时同名阶段会一起亮（用户报过：位置高亮要"选中的项目 + 它的阶段"）。 */
  viewing: { type: String, default: '' },
  /** 该项目正在跑的任务 `{kind, current, total, extra}`（没有则 null；`extra` = 另外还有几条） */
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
  // 并行后一个项目可能同时跑好几条任务：首条显示进度，其余用「+N」带过
  const more = props.running?.extra > 0 ? ` +${props.running.extra}` : ''
  return (pct != null ? `${name} ${pct}%` : name) + more
})

const progressChip = computed(() => (progress.value >= 3 ? 'g' : progress.value === 2 ? 'p' : 'a'))
const badgeClass = computed(() =>
  props.p.source?.kind === 'recorder' ? 'src-rec' : props.p.source?.kind === 'bilibili' ? 'src-bili' : 'src-local'
)

/**
 * 阶段节点打不打勾 —— **只有「预处理」会打 ✓**（用户要求）。
 *
 * 转写 + 事件检测是一次性里程碑（`stage >= 3`，转写与 events.json 都落了盘），
 * 做完就是做完了。**切片与精修没有真正意义上的"完成"**：随时能重新检测冒出未剪切事件、
 * 随时能补切一条、精修更是可以反复重新导出 —— 给它们打勾等于宣称一个不存在的终态。
 * `stageInfo().done` 里那两档（`slices` = 全部切完且无未剪切、`polish` = 全部出片）
 * 只是个瞬时状态，随时会翻回去，不适合作勾选语义。
 * 这两条的状态交给进度 chip（已切 N · 待切 M / 精修 N/M）与运行中的百分比表达。
 */
function nodeDone(key) {
  return key === 'prepare' && !!info.value.done[key]
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
/* 只保留基准样式里没有的几处：行内删除按钮、当前项目/当前位置高亮、"占用"行 */
/* `×` 也钉住不压缩：`.mini-btn` 没写 flex，默认是可收缩的，行挤的时候会把 × 裁掉（虽然它平时 opacity:0） */
.proj-head .del { padding:1px 7px; flex:none; opacity:0; }
.proj-head:hover .del { opacity:1; }
.badge { font-size:10px; }
/* 进度 chip（如「预处理 42%」）保持完整单词显示：不许被长名挤瘪换行 */
.proj-head .badge { flex:none; white-space:nowrap; }
.snode.viewing { background:var(--main-soft); color:var(--pink-text); font-weight:600; }
/* 当前项目整行高亮。光靠底色不够：`--main-soft`(#FDE3EC) 与悬停底色 `--bg-soft`(#FFEFE7)
   太接近，所以左边再加一条 `--main` 强调条、名字走 `--pink-text`。
   选择器必须比 `.proj-head:hover`（同为 scoped (0,2,0)）更特指，否则鼠标一放上去就被 hover 底色盖掉 */
.proj.on .proj-head { position:relative; background:var(--main-soft); }
.proj.on .proj-head::before { content:''; position:absolute; left:0; top:4px; bottom:4px; width:3px;
  border-radius:0 var(--r-full) var(--r-full) 0; background:var(--main); }
.proj.on .proj-head .nm { color:var(--pink-text); }
.proj.on .proj-head .tw { color:var(--main-deep); }
.snode.size-node { color:var(--ink-faint); font-size:11px; cursor:default; }
.snode.size-node:hover { background:transparent; color:var(--ink-faint); }
</style>
