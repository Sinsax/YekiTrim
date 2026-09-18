<template>
  <aside class="side" :class="{ mini: collapsed }">
    <div class="side-brand" @click="go('/workbench')">
      <div class="brand-logo">V</div>
      <div v-if="!collapsed">
        <div class="brand-name">V-TRIM</div>
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

    <!-- 折叠态：项目首字图标轨（悬停看全名，点击直达该项目当前阶段） -->
    <div v-else class="rail-list">
      <button v-for="p in filteredProjects" :key="p.id" class="rail-proj"
              :class="{ on: String(p.id) === String(route.params.pid) }"
              :title="`${p.name} · ${runningOf(p.id) ? '运行中' : stepLabelOf(p)}`"
              @click="openProject(p)">
        <span class="ini">{{ initialOf(p.name) }}</span>
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
      </template>
      <template v-else>
        <button v-for="e in entries" :key="e.key" class="rail-item" :class="{ on: current === e.key }"
                :title="e.label" @click="go(e.path)">
          <Icon :name="e.icon" :size="16" />
          <span v-if="e.key === 'home' && hasRunning" class="rdot"></span>
        </button>
      </template>
    </div>
  </aside>
</template>

<script setup>
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import ProjectNode from './ProjectNode.vue'
import Icon from '../Icon.vue'
import { authState } from '../../stores/auth'
import { useJobs } from '../../composables/useJobs'
import { ui, toggleSide } from '../../stores/ui'
import { STEPS, stageInfo } from '../../utils'
import { filteredProjects, isExpanded, toggleExpand, wb } from '../../stores/workbench'

defineProps({
  projectCount: { type: Number, default: 0 }
})
const emit = defineEmits(['open', 'remove', 'logout', 'open-jobs'])

const route = useRoute()
const router = useRouter()
const { jobs, hasRunning } = useJobs()

/** 折叠态：只留品牌 / 项目首字 / 全局入口图标（状态在 stores/ui.js，localStorage 持久化） */
const collapsed = computed(() => ui.sideCollapsed)

const isAdmin = computed(() => authState.user?.role === 'admin')
const current = computed(() => {
  if (route.name === 'workbench') return 'home'
  if (route.name === 'ingest') return 'ingest'
  if (route.name === 'settings') return 'settings'
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
.rail-proj.on { background:linear-gradient(135deg,var(--main),var(--main-deep)); color:#fff; box-shadow:var(--shadow-pink); }
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
.rail-user .av { width:26px; height:26px; border-radius:50%; font-size:12px; color:#fff;
  background:linear-gradient(135deg,var(--main),var(--pink-light));
  display:flex; align-items:center; justify-content:center; }
.rdot { position:absolute; top:2px; right:2px; width:8px; height:8px; border-radius:50%;
  background:var(--red); border:1.5px solid var(--card); }
</style>
