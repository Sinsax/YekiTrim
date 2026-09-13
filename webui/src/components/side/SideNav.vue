<template>
  <aside class="side">
    <div class="side-brand" @click="go('/workbench')">
      <div class="brand-logo">V</div>
      <div>
        <div class="brand-name">V-TRIM</div>
        <div class="brand-sub">切片工作台</div>
      </div>
    </div>

    <button v-if="isAdmin" class="new-btn" @click="emit('new-project')">
      <Icon name="plus" :size="13" /> 新建项目
    </button>

    <div class="side-filter">
      <input v-model.trim="wb.filter" placeholder="过滤项目…" />
    </div>

    <div class="side-list">
      <div class="side-cap">项目（{{ filteredProjects.length }}）</div>
      <div v-if="wb.loading && !wb.projects.length" class="hint">加载中…</div>
      <div v-else-if="!filteredProjects.length" class="hint">
        {{ wb.projects.length ? '无匹配项目' : '还没有项目' }}
      </div>
      <ProjectNode
        v-for="p in filteredProjects"
        :key="p.id"
        :p="p"
        :expanded="isExpanded(p.id)"
        :viewing="viewingStep"
        :running="runningOf(p.id)"
        @toggle="toggleExpand"
        @open="(e) => emit('open', e)"
        @remove="(p) => emit('remove', p)"
      />
    </div>

    <div class="side-foot">
      <div class="user-chip" @click="emit('logout')" title="点击登出">
        <span class="av">{{ (authState.user?.username || '?')[0].toUpperCase() }}</span>
        <span class="nm">
          <b>{{ authState.user?.username }}</b>
          <span class="muted">{{ isAdmin ? '管理员' : '用户' }} · 点击登出</span>
        </span>
      </div>
      <div v-for="e in entries" :key="e.key" class="foot-item" :class="{ on: current === e.key }" @click="go(e.path)">
        {{ e.label }}
        <span v-if="e.key === 'jobs' && hasRunning" class="dot"></span>
        <span v-if="e.badge" class="chip n foot-chip">{{ e.badge }}</span>
      </div>
    </div>
  </aside>
</template>

<script setup>
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Icon from '../Icon.vue'
import ProjectNode from './ProjectNode.vue'
import { authState } from '../../stores/auth'
import { useJobs } from '../../composables/useJobs'
import { filteredProjects, isExpanded, toggleExpand, wb } from '../../stores/workbench'

const props = defineProps({
  projectCount: { type: Number, default: 0 }
})
const emit = defineEmits(['open', 'remove', 'logout', 'new-project', 'open-jobs'])

const route = useRoute()
const router = useRouter()
const { jobs, hasRunning } = useJobs()

const isAdmin = computed(() => authState.user?.role === 'admin')
const current = computed(() => {
  if (route.name === 'workbench') return 'home'
  if (route.name === 'ingest') return 'ingest'
  if (route.name === 'settings') return 'settings'
  return 'project'
})
const viewingStep = computed(() => (route.name?.startsWith('project-') ? String(route.name).split('-')[1] : ''))

/** 全局入口列表（role 过滤；存储/管理属于 P5，未实现前不显示假入口）。 */
const entries = computed(() => {
  const list = [
    { key: 'home', label: '首页', path: '/workbench' },
    { key: 'project', label: '当前项目', path: currentProjectPath.value },
    { key: 'ingest', label: '入库', path: '/ingest' },
    { key: 'settings', label: '设置', path: '/settings', admin: true }
  ]
  return list.filter((e) => !e.admin || isAdmin.value)
})

const currentProjectPath = computed(() => {
  const pid = wb.last?.pid || wb.projects[0]?.id
  return pid ? `/p/${encodeURIComponent(pid)}/${wb.last?.step || 'prepare'}` : '/workbench'
})

/** 项目 → 正在跑的任务（任务带 project 标签）。 */
function runningOf(pid) {
  const j = (jobs.value || []).find(
    (x) => (x.status === 'running' || x.status === 'queued') && String(x.project) === String(pid)
  )
  return j ? { kind: j.kind, current: j.current || 0, total: j.total || 0 } : null
}

async function go(path) {
  if (path === '/jobs') {
    emit('open-jobs')
    return
  }
  router.push(path)
}

defineExpose({ openJobs: () => emit('open-jobs') })
</script>

<style scoped>
.side { width:264px; flex:none; display:flex; flex-direction:column; gap:8px; padding:12px 10px; border-right:1px solid var(--line); background:var(--card); height:100vh; position:sticky; top:0; z-index:10; }
.side-brand { display:flex; align-items:center; gap:9px; padding:2px 6px 6px; cursor:pointer; }
.brand-logo { width:34px; height:34px; border-radius:50%; background:linear-gradient(135deg,var(--main),var(--pink-light)); display:flex; align-items:center; justify-content:center; font-size:16px; color:#fff; box-shadow:var(--shadow-pink); }
.brand-name { font-size:14px; font-weight:700; color:var(--main-deep); line-height:1.2; }
.brand-sub { font-size:9px; color:var(--ink-faint); letter-spacing:.1em; }
.new-btn { display:flex; align-items:center; justify-content:center; gap:6px; width:100%; padding:8px; border-radius:var(--r-full); border:none; background:linear-gradient(135deg,var(--main),var(--main-deep)); color:#fff; font-size:13px; font-weight:600; cursor:pointer; box-shadow:var(--shadow-pink); }
.side-filter input { width:100%; padding:6px 10px; font-size:12px; }
.side-list { flex:1; overflow-y:auto; display:flex; flex-direction:column; gap:2px; }
.side-cap { font-size:10px; color:var(--ink-faint); letter-spacing:.08em; padding:4px 6px; }
.hint { padding:10px; font-size:12px; color:var(--ink-dim); }
.side-foot { border-top:1px dashed var(--line-soft); padding-top:8px; display:flex; flex-direction:column; gap:2px; }
.user-chip { display:flex; align-items:center; gap:8px; padding:6px 8px; border-radius:10px; cursor:pointer; }
.user-chip:hover { background:var(--bg-soft); }
.av { width:26px; height:26px; border-radius:50%; background:var(--main-soft); color:var(--pink-text); display:flex; align-items:center; justify-content:center; font-size:12px; font-weight:700; }
.user-chip .nm { display:flex; flex-direction:column; line-height:1.25; min-width:0; }
.user-chip .nm b { font-size:12px; }
.user-chip .nm .muted { font-size:10px; }
.foot-item { display:flex; align-items:center; gap:6px; padding:7px 9px; border-radius:10px; font-size:13px; color:var(--ink-dim); cursor:pointer; }
.foot-item:hover { background:var(--bg-soft); color:var(--ink); }
.foot-item.on { background:var(--main-soft); color:var(--pink-text); font-weight:600; }
.dot { width:6px; height:6px; border-radius:50%; background:var(--red); }
.foot-chip { margin-left:auto; }
</style>
