<template>
  <div class="proj" :class="{ open: expanded, on: current }">
    <!-- 卡片两行（用户要求）：第一行只放箭头 + 项目名，第二行放来源徽标 + 状态 chip + 删除。
         单行时名字要跟「来源徽标 / 状态 chip / ×」分 268px 的侧栏宽度，长名基本只露出前几个字
         （`.nm` 是 nowrap + ellipsis）；徽标让到第二行后名字独占整行，能多显示约一倍字数。
         `×` 也一并下移：它虽然 opacity:0，但**一直占着布局宽度**，留在第一行会白吃掉名字的空间。

         ⚠️ 第一行是**真按钮**（不是可点 div）：原来整块 `.proj-head` 挂 @click，键盘 Tab 到不了，
         而里面还嵌着删除按钮 —— 直接给它 role="button" 会造成嵌套交互元素。
         现在「箭头 + 项目名」= 一个按钮，删除独立成另一个按钮，两者不再互相嵌套。 -->
    <div class="proj-head">
      <button
        class="proj-toggle"
        type="button"
        :aria-label="p.name"
        :aria-expanded="expanded ? 'true' : 'false'"
        :aria-controls="stepsId"
        :title="expanded ? '收起项目' : '展开项目'"
        @click="toggle"
      >
        <span class="tw" aria-hidden="true">▶</span>
        <span class="nm" @mouseenter="startMarquee" @mouseleave="stopMarquee">
          <span ref="nameTextEl" class="nm-text" :class="{ marquee: marqueeOn }" :style="marqueeStyle">{{ p.name }}</span>
        </span>
      </button>
      <div class="prow pmeta">
        <span class="src-badge" :class="badgeClass" :title="sourceLabel(p)">{{ sourceBadge(p) }}</span>
        <span class="chip badge" :class="progressChip">{{ currentLabel }}</span>
        <button
          class="mini-btn del"
          type="button"
          :aria-label="`移除项目：${p.name}`"
          title="从列表移除（不动目录里的任何文件）"
          @click.stop="emit('remove', p)"
        >×</button>
      </div>
    </div>
    <div v-show="expanded" :id="stepsId" class="proj-steps">
      <!-- 只标两件事：**当前位置**（选中的项目 + 该项目正在看的阶段）与**已完成阶段的打勾**。
           打勾**只有「预处理」**会拿到（见 nodeDone）：切片/精修是持续活动、没有终态。
           原来还有一档 `cur`（progress === i+1，走到的最远阶段）的粉底高亮，用户要求删掉 ——
           它和"我在哪"是两套信号，同时亮反而看不出哪个是当前。 -->
      <button
        v-for="s in STEPS"
        :key="s.key"
        class="snode"
        type="button"
        :class="{ done: nodeDone(s.key), viewing: current && viewing === s.key }"
        :aria-current="current && viewing === s.key ? 'page' : undefined"
        @click="emit('open', { pid: p.id, step: s.key })"
      >
        <span class="ic" aria-hidden="true">{{ nodeDone(s.key) ? '✓' : '' }}</span>{{ s.name }}
        <span v-if="runningPct(s.key) != null" class="pct">{{ runningPct(s.key) }}%</span>
      </button>
      <div class="snode size-node"><span class="size-tag">占用 {{ humanBytes(p.bytes) }}</span></div>
    </div>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue'
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

/** `aria-controls` 要指向真实的容器 id；一个侧栏里可能同时展开多个项目，必须带 pid 去重 */
const stepsId = computed(() => `proj-steps-${props.p.id}`)

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

/* ---- 项目名跑马灯 ----
   默认由 `.nm-text` 自己带省略号；只有鼠标移入后**实测**文字宽度超过可视宽度才启动。
   用 `transform` + `transition` 而不是循环动画：正向滚到末尾就停住，鼠标移走时 class
   撤销，浏览器按 transition 的反向缩短规则沿原路滚回起点（中途移开也能从当前位置返回）。 */
const NM_SPEED = 100 // px/s，比原来的逐帧观感快约 2.4 倍
const nameTextEl = ref(null)
const marqueeOn = ref(false)
const marqueeStyle = ref({})

function startMarquee() {
  const text = nameTextEl.value
  if (!text) return
  // 系统开了“减少动态效果”就不跑，保留省略号与 title 提示
  if (typeof window !== 'undefined' && window.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches) return
  const distance = text.scrollWidth - text.clientWidth
  if (distance <= 1) return
  marqueeStyle.value = {
    '--nm-shift': `-${distance}px`,
    '--nm-duration': `${Math.max(0.4, distance / NM_SPEED)}s`
  }
  marqueeOn.value = true
}

function stopMarquee() {
  marqueeOn.value = false
}

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
/* 只保留基准样式里没有的几处：两行卡片布局、行内删除按钮、当前项目/当前位置高亮、"占用"行 */
/* 两行卡片（样稿 `ia-demos/multiuser.html` 的 `.proj-head` 是单行 flex，没有这个态，所以写在 scoped）。
   只改主轴方向与行距：内边距 / 圆角 / 悬停底色仍走 app.css 的 `.proj-head` 基准值。
   scoped 规则的 `[data-v]` 让它 (0,2,0) 压过 app.css 的 `.proj-head` (0,1,0)。 */
.proj-head { flex-direction:column; align-items:stretch; gap:4px; }
.prow { display:flex; align-items:center; gap:8px; min-width:0; }
/* 第一行按钮：铺满整行、贴左，去掉按钮默认外观；箭头与名字的排布与原 `.prow` 一致 */
.proj-toggle { display:flex; align-items:center; gap:8px; width:100%; min-width:0;
  padding:0; background:none; border:none; color:inherit; text-align:left; cursor:pointer; }
/* 第二行缩进到名字的左缘：箭头宽 10px + 行内 gap 8px = 18px */
.pmeta { padding-left:18px; }
/* `×` 钉住不压缩：`.mini-btn` 没写 flex，默认是可收缩的（虽然它平时 opacity:0）。
   从第一行下移到第二行后，靠 margin-left:auto 顶到最右端。
   ⚠️ 键盘 focus 与触屏（没有 hover）也必须能看见它 —— 只写 `:hover` 时，
   键盘用户 Tab 到一颗完全透明的按钮上，属于"焦点不可见"。 */
.proj-head .del { padding:1px 7px; flex:none; opacity:0; margin-left:auto; }
.proj-head:hover .del,
.proj-head:focus-within .del { opacity:1; }
@media (hover:none) {
  .proj-head .del { opacity:.75; }
}
.badge { font-size:10px; }
/* 进度 chip（如「预处理 42%」）保持完整单词显示：不许被长名挤瘪换行 */
.proj-head .badge { flex:none; white-space:nowrap; }
/* 项目名跑马灯：`.nm-text` 静止时自己截断并显示省略号（与旧版 `.nm` 的观感一致）；
   只有 JS 实测溢出后加的 `.marquee` 才放开 overflow 并设置目标位移。
   `transition` 让正向滚到末尾后停住；移走时 class 撤销，再从当前位置滚回起点。 */
.nm-text { display:block; max-width:100%; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  transform:translateX(0); transition:transform var(--nm-duration,.4s) linear; }
.nm-text.marquee { overflow:visible; transform:translateX(var(--nm-shift,0px)); will-change:transform; }
.snode.viewing { background:var(--main-soft); color:var(--pink-text); font-weight:600; }
/* 当前项目整行高亮。光靠底色不够：`--main-soft`(#FAECEA) 与悬停底色 `--bg-soft`(#F0EAE4)
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
