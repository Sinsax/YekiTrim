import { createRouter, createWebHashHistory } from 'vue-router'
import ProjectLayout from './views/ProjectLayout.vue'
import { api } from './api/client'
import { scopeQ, setScope } from './api/scope'
import { STAGE_EVENTS_READY } from './utils'
import { authState, isAuthed, loadMe } from './stores/auth'

// 直接访问 /project/:pid（深链 / 刷新）时也按预处理进度决定入口页：
// 事件检测已完成 → 切片页，否则 → 预览页。
// M2：作用域是**项目 id**（不再是工程路径），服务端据此校验归属。
async function projectEntryGuard(to) {
  setScope(to.params.pid)
  try {
    const st = await api(`/api/project-data/status?${scopeQ(to.params.pid)}`)
    if ((st.current_stage || 0) >= STAGE_EVENTS_READY) {
      return { name: 'project-slices', params: to.params }
    }
  } catch {}
  return { name: 'project-preview', params: to.params }
}

const routes = [
  { path: '/login', name: 'login', component: () => import('./views/LoginView.vue') },
  { path: '/', name: 'home', component: () => import('./views/HomeView.vue') },
  { path: '/projects', redirect: '/' },
  {
    path: '/project/:pid',
    name: 'project',
    component: ProjectLayout,
    children: [
      { path: '', name: 'project-root', beforeEnter: projectEntryGuard },
      { path: 'settings', name: 'project-settings', component: () => import('./views/ProjectSettingsView.vue') },
      { path: 'preview', name: 'project-preview', component: () => import('./views/PreviewView.vue') },
      { path: 'slices', name: 'project-slices', component: () => import('./views/ProjectSlicesView.vue') },
      { path: 'slices/:clip', name: 'project-slice-polish', component: () => import('./views/PolishView.vue') },
      { path: 'polish', name: 'project-polish', component: () => import('./views/PolishView.vue') }
    ]
  },
  { path: '/polish', name: 'polish', component: () => import('./views/PolishProjectsView.vue') },
  { path: '/models', name: 'models', meta: { admin: true }, component: () => import('./views/ModelsView.vue') },
  { path: '/settings', name: 'settings', meta: { admin: true }, component: () => import('./views/SettingsView.vue') },
  { path: '/jobs', name: 'jobs', component: () => import('./views/JobsView.vue') }
]

const router = createRouter({
  history: createWebHashHistory(),
  routes
})

// 全局守卫：**没有免登录分支**（服务端 §3.5.1，单人部署同样要登录）。
// 首次进入先问一次 `/api/me`，401 → 登录页并记住回跳。
router.beforeEach(async (to) => {
  if (to.name === 'login') {
    // 已登录时不再停留在登录页
    if (!authState.ready) await loadMe()
    return isAuthed() ? { path: '/' } : true
  }
  if (!authState.ready) await loadMe()
  if (!isAuthed()) {
    return { name: 'login', query: { redirect: to.fullPath } }
  }
  // 平台级页面（全局设置 / 模型管理）仅管理员可见；P2 会把它们迁进 /api/admin/*
  if (to.meta?.admin && authState.user?.role !== 'admin') {
    return { path: '/' }
  }
  return true
})

export default router
