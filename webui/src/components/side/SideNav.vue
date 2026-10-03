<template>
  <aside class="side" :class="{ mini: collapsed }">
    <div class="side-brand" @click="go('/workbench')">
      <div class="brand-logo">
        <!-- 品牌标：与登录页同一份正式 logo（src/assets/logo.png）。不要再改回内联 SVG。 -->
        <img :src="logoUrl" alt="YekiTrim" />
      </div>
      <div v-if="!collapsed">
        <div class="brand-name">YekiTrim</div>
        <div class="brand-sub">切片工作台</div>
      </div>
      <button class="side-toggle" :title="collapsed ? '展开侧栏' : '折叠侧栏'"
              :aria-label="collapsed ? '展开侧栏' : '折叠侧栏'" @click.stop="toggleSide">
        <Icon :name="collapsed ? 'chevron-right' : 'chevron-left'" :size="14" />
      </button>
    </div>

    <template v-if="!collapsed">
      <div class="side-filter">
        <input v-model.trim="wb.filter" placeholder="过滤项目…" autocomplete="off" />
      </div>

      <div class="side-list">
        <div class="side-cap">项目（{{ filteredProjects.length }}）</div>
        <div v-if="wb.loading && !wb.projects.length" class="hint">加载中…</div>
        <div v-else-if="!filteredProjects.length" class="hint">
          {{ wb.projects.length ? '无匹配项目' : '还没有项目：去首页「新增项目」' }}
        </div>
        <ProjectNode
          v-for="p in filteredProjects"
          :key="p.id"
          :p="p"
          :expanded="isExpanded(p.id)"
          :current="String(p.id) === String(route.params.pid)"
          :viewing="viewingStep"
          :running="runningOf(p.id)"
          @toggle="toggleExpand"
          @open="(e) => emit('open', e)"
          @remove="(p) => emit('remove', p)"
        />
      </div>
    </template>

    <!-- 折叠态：项目标识图标轨（过滤数字/符号，多项目去重；悬停看全名，点击直达当前阶段） -->
    <div v-else class="rail-list">
      <button v-for="p in filteredProjects" :key="p.id" class="rail-proj"
              :class="{ on: String(p.id) === String(route.params.pid) }"
              :title="`${p.name} · ${runningOf(p.id) ? '运行中' : stepLabelOf(p)}`"
              @click="openProject(p)">
        <span class="ini">{{ projectLabelOf(p) }}</span>
        <span v-if="runningOf(p.id)" class="rdot"></span>
      </button>
      <div v-if="!filteredProjects.length" class="hint rail-hint"
           :title="wb.projects.length ? '过滤条件把项目都藏起来了' : '还没有项目'">·</div>
    </div>

    <div class="side-foot">
      <!-- 用户信息只做展示（点它不再登出，用户要求）；登出是右边那个按钮，且要二次确认 -->
      <div v-if="!collapsed" class="user-chip">
        <span class="av">{{ initialOf(authState.user?.username || '?') }}</span>
        <span class="nm">
          <b>{{ authState.user?.username }}</b>
          <span class="muted small">{{ isAdmin ? '管理员' : '用户' }}</span>
        </span>
        <button class="mini-btn side-logout" title="退出登录（会再确认一次）" @click="emit('logout')">
          <Icon name="logout" :size="12" /> 退出
        </button>
      </div>
      <button v-else class="rail-item rail-user" :title="`${authState.user?.username}（${isAdmin ? '管理员' : '用户'}）· 退出登录`"
              @click="emit('logout')">
        <span class="av">{{ initialOf(authState.user?.username || '?') }}</span>
        <Icon name="logout" :size="14" class="rail-out" />
      </button>

      <template v-if="!collapsed">
        <div v-for="e in entries" :key="e.key" class="foot-item" :class="{ on: current === e.key }" @click="go(e.path)">
          {{ e.label }}
          <span v-if="e.key === 'home' && hasRunning" class="dot"></span>
        </div>
        <!-- 主题切换：浅色（哥特浅色）/ 暗色（哥特暗色） -->
        <button class="foot-item theme-toggle" :title="themeTitle" @click="toggleTheme()">
          <Icon :name="ui.theme === 'dark' ? 'moon' : 'sun'" :size="13" />
          {{ ui.theme === 'dark' ? '暗色' : '浅色' }}
          <span v-if="!ui.themeIsManual" class="muted small">跟随系统</span>
        </button>
      </template>
      <template v-else>
        <button v-for="e in entries" :key="e.key" class="rail-item" :class="{ on: current === e.key }"
                :title="e.label" @click="go(e.path)">
          <Icon :name="e.icon" :size="16" />
          <span v-if="e.key === 'home' && hasRunning" class="rdot"></span>
        </button>
        <button class="rail-item" :title="themeTitle" @click="toggleTheme()">
          <Icon :name="ui.theme === 'dark' ? 'moon' : 'sun'" :size="16" />
        </button>
      </template>
    </div>
  </aside>
</template>

<script setup>
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import logoUrl from '../../assets/logo.png'
import ProjectNode from './ProjectNode.vue'
import Icon from '../Icon.vue'
import { authState } from '../../stores/auth'
import { useJobs } from '../../composables/useJobs'
import { ui, toggleSide } from '../../stores/ui'
import { toggleTheme } from '../../stores/theme'
import { STEPS, stageInfo, hashIndex } from '../../utils'
import { filteredProjects, isExpanded, toggleExpand, wb } from '../../stores/workbench'

defineProps({
  projectCount: { type: Number, default: 0 }
})
const emit = defineEmits(['open', 'remove', 'logout', 'open-jobs'])

const route = useRoute()
const router = useRouter()
const { jobs, hasRunning } = useJobs()

/** 折叠态：只留品牌 / 项目标识 / 全局入口图标（状态在 stores/ui.js，localStorage 持久化） */
const collapsed = computed(() => ui.sideCollapsed)

/** 主题切换按钮的提示：明说当前档位与"是否跟随系统" */
const themeTitle = computed(() => {
  const cur = ui.theme === 'dark' ? '暗色' : '浅色'
  const tail = ui.themeIsManual ? '点这里切换' : '当前跟随系统，点这里固定'
  return `主题：${cur}（${tail}）`
})

const isAdmin = computed(() => authState.user?.role === 'admin')
const current = computed(() => {
  if (route.name === 'workbench') return 'home'
  if (route.name === 'ingest') return 'ingest'
  // 设置页拆成了子页（settings-config / settings-llm…）：认 meta 标记，别写死路由名
  if (route.meta?.settings) return 'settings'
  return 'project'
})
const viewingStep = computed(() => (route.name?.startsWith('project-') ? String(route.name).split('-')[1] : ''))

/** 全局入口列表（role 过滤；存储/管理属于 P5，未实现前不显示假入口）。icon 供折叠轨用。 */
const entries = computed(() => {
  const list = [
    { key: 'home', label: '首页', path: '/workbench', icon: 'home' },
    { key: 'settings', label: '设置', path: '/settings', icon: 'settings', admin: true }
  ]
  return list.filter((e) => !e.admin || isAdmin.value)
})

/**
 * 项目 → 正在跑的任务（任务带 project 标签）。
 *
 * 并行之后一个项目可能同时有好几条（比如 clip-01 在渲染、clip-02 在跑 LLM）：
 * 取第一条显示进度，另外用 `extra` 标注"还有 N 条"，别让侧栏看起来只有一个任务。
 */
function runningOf(pid) {
  const list = (jobs.value || []).filter(
    (x) => (x.status === 'running' || x.status === 'queued') && String(x.project) === String(pid)
  )
  if (!list.length) return null
  const j = list[0]
  return { kind: j.kind, current: j.current || 0, total: j.total || 0, extra: list.length - 1 }
}

/** 项目首字（emoji/中文都按字符取，不用 charAt 以免切开代理对）。 */
function initialOf(s) {
  const ch = Array.from(String(s || '').trim())[0] || '·'
  return ch.toUpperCase()
}

/**
 * 折叠轨的项目标识：显示两个字，只取“有意义”的字符，过滤掉 `[` `]` `-` `:` 数字这些。
 *
 * 录播姬项目名通常是 `[2026-08-13 20-54-35-962][好困]`：
 * 过滤后只剩 `好困`，取末尾相邻两个字（标签通常在末尾），所以折叠后显示 `好困`。
 *
 * 多项目去重按项目 id 固定顺序分配，先到先得；同一项目永远拿同一个标识。
 */
const PROJECT_FALLBACK_CHARS = [
  '春', '夏', '秋', '冬', '东', '南', '西', '北',
  '风', '花', '雪', '月', '星', '海', '山', '云',
  '林', '川', '光', '影', '木', '水', '火', '土'
]
/** Unicode 字母（含 CJK）才算有意义；数字/标点/符号/emoji 都过滤掉。 */
const MEANINGFUL_CHAR_RE = /[\p{L}\p{Nl}]/u

function projectNameChars(name) {
  return Array.from(String(name || '')).filter((ch) => MEANINGFUL_CHAR_RE.test(ch))
}

/** 标题里的候选两字标识：末尾相邻两字优先，再往前滑。 */
function projectLabelCandidates(p) {
  const chars = projectNameChars(p.name)
  if (chars.length < 2) return []
  const out = []
  for (let i = chars.length - 2; i >= 0; i--) {
    out.push(chars[i] + chars[i + 1])
  }
  return out
}

/** 标题没有足够可用字或候选全被占时，从固定字池里按 id 稳定挑一个两字组合。 */
function fallbackProjectLabel(seed, used) {
  const n = PROJECT_FALLBACK_CHARS.length
  const total = n * n
  const start = hashIndex(seed, total)
  for (let k = 0; k < total; k++) {
    const idx = (start + k) % total
    const label = PROJECT_FALLBACK_CHARS[Math.floor(idx / n)] + PROJECT_FALLBACK_CHARS[idx % n]
    if (!used.has(label)) return label
  }
  return '项目'
}

const projectLabels = computed(() => {
  const map = {}
  const used = new Set()
  // 按 id 排序而不是按当前过滤/排序结果：列表顺序变化时每个项目的标识不能跟着变。
  const list = [...(wb.projects || [])].sort((a, b) => Number(a.id) - Number(b.id))
  for (const p of list) {
    let label = ''
    for (const c of projectLabelCandidates(p)) {
      if (!used.has(c)) {
        label = c
        break
      }
    }
    if (!label) label = fallbackProjectLabel(String(p.id ?? p.name ?? ''), used)
    used.add(label)
    map[String(p.id)] = label
  }
  return map
})

function projectLabelOf(p) {
  return projectLabels.value[String(p.id)] || '项目'
}

/** 粗粒度当前阶段（不看缓存，够图标轨用）。 */
function stepOf(p) {
  const pr = stageInfo(p).progress
  return pr >= 3 ? 'polish' : pr === 2 ? 'slices' : 'prepare'
}

function stepLabelOf(p) {
  return STEPS.find((s) => s.key === stepOf(p))?.name || '预处理'
}

function openProject(p) {
  emit('open', { pid: p.id, step: stepOf(p) })
}

async function go(path) {
  router.push(path)
}

defineExpose({ openJobs: () => emit('open-jobs') })
</script>

<style scoped>
/* 折叠轨（样稿无此态，只写样稿里没有的东西）：宽度由 app.css 的 `.side.mini` 给 */
.side-toggle { margin-left:auto; width:26px; height:26px; flex:none; border-radius:var(--r-full);
  border:1px solid var(--line); background:var(--card); color:var(--ink-dim);
  display:flex; align-items:center; justify-content:center; }
.side-toggle:hover { border-color:var(--main); color:var(--pink-text); background:var(--main-soft); }
.side.mini .side-brand { flex-direction:column; gap:6px; padding:12px 0 6px; }
.side.mini .side-toggle { margin-left:0; }

.rail-list { flex:1; min-height:0; overflow-y:auto; overflow-x:hidden;
  display:flex; flex-direction:column; align-items:center; gap:6px; padding:0 0 8px; }
.rail-proj { position:relative; width:40px; height:40px; flex:none; border-radius:var(--r-full);
  background:var(--bg-soft); color:var(--ink-dim); font-size:13px; font-weight:700;
  display:flex; align-items:center; justify-content:center; transition:.15s; }
.rail-proj:hover { background:var(--main-soft); color:var(--pink-text); }
.rail-proj.on { background:var(--grad-main); color:var(--on-main); box-shadow:var(--shadow-pink); }
.rail-hint { padding:4px 0; }

.side.mini .side-foot { align-items:center; padding:8px 0; }
/* 用户块只做展示（登出改成了右侧按钮 + 二次确认），光标别留成可点的手型 */
.user-chip { cursor:default; }
.side-logout { margin-left:auto; flex:none; padding:3px 8px; font-size:11.5px; }
/* 折叠轨的用户块：头像 + 退出图标上下两行（原来只有头像）。
   选择器必须比 `.rail-item`（同为 scoped (0,2,0) 且写在后面）更特指，否则 height:34px 会赢 */
.side-foot .rail-user { flex-direction:column; height:auto; padding:4px 0; gap:2px; }
.rail-user .rail-out { color:var(--ink-faint); }
.rail-user:hover .rail-out { color:var(--red-deep); }
.rail-item { position:relative; width:40px; height:34px; flex:none; border-radius:var(--r-sm);
  display:flex; align-items:center; justify-content:center; color:var(--ink-dim); }
.rail-item:hover { background:var(--bg-soft); color:var(--ink); }
.rail-item.on { background:var(--main-soft); color:var(--pink-text); }
.rail-user .av { width:26px; height:26px; border-radius:50%; font-size:12px; color:var(--on-main);
  background:linear-gradient(135deg,var(--main),var(--tan));
  display:flex; align-items:center; justify-content:center; }
.rdot { position:absolute; top:2px; right:2px; width:8px; height:8px; border-radius:50%;
  background:var(--red); border:1.5px solid var(--card); }
</style>
