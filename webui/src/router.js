import { createRouter, createWebHashHistory } from 'vue-router'
import { projectData } from './api/projectData'
import { setScope } from './api/scope'
import { SETTINGS_SECTIONS, STAGE_EVENTS_READY } from './utils'
import { localNoAuth, localNoAuthKnown } from './stores/health'
import { authState, isAuthed, loadMe } from './stores/auth'
import { addClient } from './stores/log'
import { lastPath } from './stores/workbench'

// `/p/:pid` 入口守卫：预处理（事件检测）已完成 → 切片页，否则 → 预处理页。
async function projectEntryGuard(to) {
  setScope(to.params.pid)
  try {
    // 作用域只经 api 层：这里以前自己拼 `project=${scopeQ(...)}`（门禁也没盖住 router.js），
    // 于是"查询串只在 api/ 里拼"这条不变式名存实亡。
    const st = await projectData.status(to.params.pid, '入口阶段判断')
    if ((st.current_stage || 0) >= STAGE_EVENTS_READY) {
      return { name: 'project-slices', params: to.params }
    }
  } catch (e) {
    // 状态读不到就退回预处理页；留一条痕，否则"点进去老是预处理页"看不出原因
    addClient('warn', `项目状态读取失败（${e.message}），已退回预处理页`, 'router')
  }
  return { name: 'project-prepare', params: to.params }
}

// ---- 设置页：按区块拆成子页 ----
// 五块内容都在同一个 SettingsView 里（表单状态、/api/config 加载与保存都在那一个文件），靠 props.section 切；
// 拆成独立组件要额外铺一层 props/emit，收益不抵成本。子页各自带 meta.admin —— 守卫读的是**最终落到**的
// 路由（`/settings` 那条 redirect 不参与判定），它落到最常用的「程序配置」。
const SETTINGS_ROUTES = SETTINGS_SECTIONS.map((s) => ({
  path: `/settings/${s.key}`,
  name: `settings-${s.key}`,
  meta: { admin: true, settings: true, title: s.label },
  props: { section: s.key },
  component: () => import('./views/SettingsView.vue')
}))

const routes = [
  { path: '/login', name: 'login', component: () => import('./views/LoginView.vue') },
  // 邀请码注册：与登录页一样不套壳；未登录可访问（服务端校验开关与邀请码）
  { path: '/register', name: 'register', component: () => import('./views/RegisterView.vue') },
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
  { path: '/settings', redirect: '/settings/config' },
  ...SETTINGS_ROUTES,

  // ---- 旧路径 redirect（v5 之前的路由全部保留兼容） ----
  { path: '/projects', redirect: '/workbench' },
  // 旧路由的 :cfg 是 config 路径；新作用域只认数字项目 id，路径形式一律回首页
  { path: '/project/:cfg', redirect: (to) => /^\d+$/.test(to.params.cfg) ? `/p/${to.params.cfg}` : '/workbench' },
  { path: '/project/:cfg/:rest(.*)', redirect: (to) => /^\d+$/.test(to.params.cfg)
    ? `/p/${to.params.cfg}/${to.params.rest || 'prepare'}`.replace(/\/preview$/, '/prepare')
    : '/workbench' },
  { path: '/polish', redirect: '/workbench' },
  { path: '/jobs', redirect: '/workbench' },
  { path: '/models', redirect: '/workbench' },
  { path: '/:pathMatch(.*)*', redirect: '/workbench' }
]

const router = createRouter({
  history: createWebHashHistory(),
  routes
})

// 全局守卫：**没有免登录分支**（服务端 §3.5.1）。启动先问一次 `/api/me`，401 → 登录页记回跳。
router.beforeEach(async (to) => {
  if (to.name === 'login' || to.name === 'register') {
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
  // 服务端能力位（本地免登录）**必须在任何视图挂载之前**就位：视图要靠它做**首帧**决策
  // （首页默认来源、设置页标签条收缩…），晚一步就会先闪一下另一种布局。
  // 它有缓存（store 内 pending），所以这里 await 不会每次导航都发请求。
  await localNoAuth()
  // 本地免登录模式没有"注册/邀请"这回事：直接输网址也要挡回程序配置页
  // （标签条里也过滤掉了那一项，见 SettingsView —— 两处都读同一个 health store）。
  if (to.name === 'settings-account' && localNoAuthKnown()) {
    return { path: '/settings/config' }
  }
  return true
})

export default router

// ---- 懒加载 chunk 失效兜底 ----
// dist 每次构建都会换 hash 名（vite 还会清空 outDir）。浏览器里跑着旧 index 时，
// 点侧栏/按钮切到**还没加载过的**路由 → import() 去取一个已被删掉的旧 chunk → 404，
// vue-router 中止这次导航（控制台会报 "Failed to fetch dynamically imported module"），
// 表现就是「点哪个按钮都不跳转，必须手动刷新」。这里检测到就自动硬刷一次拿新构建。
//
// sessionStorage 做一次性闸门：刷新后仍失败就不再刷（避免死循环）；
// 任意一次导航成功即清除，这样下一次发版还能再自动刷。
const RELOAD_KEY = 'yekitrim.chunkReload'
const CHUNK_ERR = /Failed to fetch dynamically imported module|Importing a module script failed|error loading dynamically imported module|Unable to preload|NOT_FOUND/i

router.onError((err) => {
  const msg = String((err && err.message) || err || '')
  if (!CHUNK_ERR.test(msg)) {
    console.error('[router] 导航失败：', err)
    return
  }
  let already = false
  try { already = sessionStorage.getItem(RELOAD_KEY) === '1' } catch {}
  if (already) {
    console.error('[router] 懒加载 chunk 仍取不到（已自动刷新过一次），请手动刷新：', err)
    return
  }
  try { sessionStorage.setItem(RELOAD_KEY, '1') } catch {}
  console.warn('[router] 前端已更新，自动刷新以取新构建…')
  window.location.reload()
})

router.afterEach(() => {
  try { sessionStorage.removeItem(RELOAD_KEY) } catch {}
})
