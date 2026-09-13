import { createRouter, createWebHashHistory } from 'vue-router'
import { api } from './api/client'
import { scopeQ, setScope } from './api/scope'
import { STAGE_EVENTS_READY } from './utils'
import { authState, isAuthed, loadMe } from './stores/auth'
import { lastPath } from './stores/workbench'

// `/p/:pid` 入口守卫：预处理（事件检测）已完成 → 切片页，否则 → 预处理页。
async function projectEntryGuard(to) {
  setScope(to.params.pid)
  try {
    const st = await api(`/api/project-data/status?${scopeQ(to.params.pid)}`)
    if ((st.current_stage || 0) >= STAGE_EVENTS_READY) {
      return { name: 'project-slices', params: to.params }
    }
  } catch {}
  return { name: 'project-prepare', params: to.params }
}

const routes = [
  { path: '/login', name: 'login', component: () => import('./views/LoginView.vue') },
  { path: '/', redirect: () => lastPath() },
  { path: '/workbench', name: 'workbench', component: () => import('./views/WorkbenchView.vue') },
  {
    path: '/p/:pid',
    children: [
      { path: '', name: 'project-root', beforeEnter: projectEntryGuard, component: () => import('./views/PrepareView.vue') },
      { path: 'prepare', name: 'project-prepare', component: () => import('./views/PrepareView.vue') },
      { path: 'slices', name: 'project-slices', component: () => import('./views/ProjectSlicesView.vue') },
      { path: 'polish', name: 'project-polish', component: () => import('./views/PolishView.vue') },
      { path: 'polish/:clip', name: 'project-polish-clip', component: () => import('./views/PolishView.vue') },
      { path: 'settings', name: 'project-settings', component: () => import('./views/ProjectSettingsView.vue') }
    ]
  },
  { path: '/ingest', name: 'ingest', component: () => import('./views/IngestView.vue') },
  { path: '/settings', name: 'settings', meta: { admin: true }, component: () => import('./views/SettingsView.vue') },

  // ---- 旧路径 redirect（v5 之前的路由全部保留兼容） ----
  { path: '/projects', redirect: '/workbench' },
  // 旧路由的 :cfg 是 config 路径；新作用域只认数字项目 id，路径形式一律回首页
  { path: '/project/:cfg', redirect: (to) => /^\d+$/.test(to.params.cfg) ? `/p/${to.params.cfg}` : '/workbench' },
  { path: '/project/:cfg/:rest(.*)', redirect: (to) => /^\d+$/.test(to.params.cfg)
    ? `/p/${to.params.cfg}/${to.params.rest || 'prepare'}`.replace(/\/preview$/, '/prepare')
    : '/workbench' },
  { path: '/polish', redirect: '/workbench' },
  { path: '/jobs', redirect: '/workbench' },
  { path: '/models', redirect: '/settings' },
  { path: '/:pathMatch(.*)*', redirect: '/workbench' }
]

const router = createRouter({
  history: createWebHashHistory(),
  routes
})

// 全局守卫：**没有免登录分支**（服务端 §3.5.1）。启动先问一次 `/api/me`，401 → 登录页记回跳。
router.beforeEach(async (to) => {
  if (to.name === 'login') {
    if (!authState.ready) await loadMe()
    return isAuthed() ? { path: '/workbench' } : true
  }
  if (!authState.ready) await loadMe()
  if (!isAuthed()) {
    return { name: 'login', query: { redirect: to.fullPath } }
  }
  if (to.meta?.admin && authState.user?.role !== 'admin') {
    return { path: '/workbench' }
  }
  return true
})

export default router
