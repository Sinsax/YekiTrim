<template>
  <aside class="side">
    <div class="side-brand" @click="go('/workbench')">
      <div class="brand-logo">V</div>
      <div>
        <div class="brand-name">V-TRIM</div>
        <div class="brand-sub">切片工作台</div>
      </div>
    </div>

    <div class="side-filter">
      <input v-model.trim="wb.filter" placeholder="过滤项目…" />
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
          <span class="muted small">{{ isAdmin ? '管理员' : '用户' }} · 点击登出</span>
        </span>
      </div>
      <div v-for="e in entries" :key="e.key" class="foot-item" :class="{ on: current === e.key }" @click="go(e.path)">
        {{ e.label }}
        <span v-if="e.key === 'home' && hasRunning" class="dot"></span>
      </div>
    </div>
  </aside>
</template>

<script setup>
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import ProjectNode from './ProjectNode.vue'
import { authState } from '../../stores/auth'
import { useJobs } from '../../composables/useJobs'
import { filteredProjects, isExpanded, toggleExpand, wb } from '../../stores/workbench'

defineProps({
  projectCount: { type: Number, default: 0 }
})
const emit = defineEmits(['open', 'remove', 'logout', 'open-jobs'])

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
    { key: 'ingest', label: '入库', path: '/ingest' },
    { key: 'settings', label: '设置', path: '/settings', admin: true }
  ]
  return list.filter((e) => !e.admin || isAdmin.value)
})

/** 项目 → 正在跑的任务（任务带 project 标签）。 */
function runningOf(pid) {
  const j = (jobs.value || []).find(
    (x) => (x.status === 'running' || x.status === 'queued') && String(x.project) === String(pid)
  )
  return j ? { kind: j.kind, current: j.current || 0, total: j.total || 0 } : null
}

async function go(path) {
  router.push(path)
}

defineExpose({ openJobs: () => emit('open-jobs') })
</script>
