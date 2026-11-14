<template>
  <!-- 登录页不套壳 -->
  <router-view v-if="isLogin" />

  <div v-else class="shell">
    <button
      v-if="mobileNavOpen"
      class="nav-mask"
      aria-label="关闭导航"
      @click="closeMobileNav"
    ></button>
    <SideNav
      ref="sideRef"
      class="side-nav"
      :class="{ 'mobile-open': mobileNavOpen }"
      :mobile-open="mobileNavOpen"
      :local-no-auth="localNoAuth"
      @open="openProject"
      @remove="removeProject"
      @logout="doLogout"
      @open-jobs="openJobs()"
      @close-mobile="closeMobileNav"
    />

    <div ref="mainRef" class="main" @scroll.passive="onMainScroll">
      <div class="crumbs">
        <button class="mini-btn mobile-menu" aria-label="打开导航" title="打开导航" @click="mobileNavOpen = true">
          <Icon name="menu" :size="17" />
        </button>
        <template v-if="route.name === 'workbench'">
          <!-- 这里是**页面标题**（h1）：工作台页内不再重复一个同名 h1，
               吸顶栏因此同时承担"我在哪"与无障碍文档标题两个职责。 -->
          <h1 class="crumb-title">首页</h1>
        </template>
        <template v-else-if="route.name === 'ingest'">
          <!-- 一级文案用术语表的「采集」；旧词「入库」进副标题（隐藏 ≠ 删除） -->
          <b>采集</b><span class="sep">/</span>
          <span class="muted small">入库 · 录制导入 · 自动导入规则</span>
        </template>
        <template v-else-if="route.meta?.settings">
          <b>设置</b><span class="sep">/</span><span class="muted small">{{ route.meta.title }}</span>
        </template>
        <template v-else-if="route.params.pid">
          <span class="crumb-proj" :title="currentProject?.name">{{ currentProject?.name || ('项目 #' + route.params.pid) }}</span>
          <span class="sep">/</span>
          <b>{{ stepName }}</b>
          <span v-if="currentProject?.source?.label" class="muted small crumb-source">
            {{ currentProject.source.label }}
          </span>
        </template>
        <span class="crumb-spacer"></span>
        <button class="mini-btn cmdk-trigger" :title="'快速跳转（' + modKeyLabel + '）'" aria-label="快速跳转" @click="paletteOpen = true">
          <Icon name="search" :size="13" />
          <span class="cmdk-trigger-label">跳转</span>
          <UiKbd :label="modKeyLabel" class="cmdk-kbd" />
        </button>
        <button class="mini-btn task-entry" title="查看运行任务" @click="openJobs()">
          <Icon name="bell" :size="14" />
          <span class="task-label">任务</span>
          <span v-if="hasRunning" class="running-label">进行中</span>
          <span v-if="hasRunning" class="red-dot"></span>
        </button>
      </div>

      <div v-if="authState.mustChangePassword" class="pwd-banner">
        <Icon name="alert" :size="14" />
        <span class="flex-1">仍在使用初始密码，请尽快修改</span>
        <button class="mini-btn" @click="openChangePassword">立即修改</button>
      </div>

      <!-- 换页进场：**只做透明度**，不加 transform ——
           `transform` 会给 position:fixed 的后代造出新的包含块（`.to-top`/浮层会瞬间错位）。
           用两个内容相同的 keyframes 交替触发，是因为同一个动画名重复赋值不会重播。 -->
      <main class="content" :class="animClass">
        <!-- 按**项目 id** 做 key：同一步骤下换项目时 route record 相同，vue-router 会复用组件实例，
             setup/onMounted 不再执行 —— 于是整页还停在旧项目（只有面包屑变了），
             而预处理器保存时用的是**新** pid，会把 A 的字幕写进 B（用户实测"两个项目会串"）。
             跨步骤切换本来就靠组件类型不同重挂，这里的 key 不改变那条路径；
             也不能用 fullPath 做 key（那样精修页换片段也会整块重挂，丢掉原地切换）。 -->
        <router-view :key="route.params.pid || route.name" />
      </main>

      <!-- 回到顶部：长页面（首页 / 切片墙）滚到一半时唯一省力的退路 -->
      <button v-show="showTop" class="to-top" title="回到顶部" aria-label="回到顶部" @click="scrollTop">
        <Icon name="arrow-up" :size="16" />
      </button>
    </div>

    <JobDrawer :show="ui.jobs" @close="closeJobs()" />
    <AppModal :show="showChangePassword" title="修改密码" @close="showChangePassword = false">
      <div class="error-banner" v-if="pwdError">{{ pwdError }}</div>
      <div class="mb-12"><label for="pwd-old">当前密码</label><input id="pwd-old" v-model="pwdOld" type="password" /></div>
      <div class="mb-12"><label for="pwd-new">新密码（至少 8 位）</label><input id="pwd-new" v-model="pwdNew" type="password" /></div>
      <div class="mb-12"><label for="pwd-new2">确认新密码</label><input id="pwd-new2" v-model="pwdNew2" type="password" /></div>
      <div class="row mt-12">
        <button class="btn btn-primary" :disabled="pwdBusy" @click="submitPassword">保存</button>
        <button class="btn" :disabled="pwdBusy" @click="showChangePassword = false">取消</button>
      </div>
    </AppModal>
    <CommandPalette :show="paletteOpen" @close="paletteOpen = false" />
    <ToastHost />
    <AppDialog />
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Icon from './components/Icon.vue'
import UiKbd from './components/ui/UiKbd.vue'
import CommandPalette from './components/CommandPalette.vue'
import SideNav from './components/side/SideNav.vue'
import { localNoAuth as localNoAuthFromServer } from './stores/health'
import JobDrawer from './components/jobs/JobDrawer.vue'
import AppModal from './components/AppModal.vue'
import ToastHost from './components/ToastHost.vue'
import AppDialog from './components/AppDialog.vue'
import { authState, changePassword, logout } from './stores/auth'
import { toast } from './stores/toast'
import { confirmDialog } from './stores/dialog'
import { STEPS } from './utils'
import { useJobs, onJobDone } from './composables/useJobs'
import { closeJobs, openJobs, ui } from './stores/ui'
import { loadProjectData, loadProjects, projectById, removeProject as apiRemoveProject, rememberLast, invalidate } from './stores/workbench'

const route = useRoute()
const router = useRouter()
const { hasRunning } = useJobs()

const pwdOld = ref('')
const pwdNew = ref('')
const pwdNew2 = ref('')
const pwdBusy = ref(false)
const pwdError = ref('')
const showChangePassword = ref(false)
const mobileNavOpen = ref(false)
/** 全局快速跳转面板（⌘K / Ctrl+K） */
const paletteOpen = ref(false)
/** 快捷键角标：Mac 系显示 ⌘K，其它（Linux/Windows）显示 Ctrl+K —— 只写 ⌘K 会让多数用户以为没有快捷键。 */
const modKeyLabel = /Mac|iPhone|iPad|iPod/i.test((navigator.platform || '') + ' ' + (navigator.userAgent || '')) ? '⌘K' : 'Ctrl+K'
/** `.main` 是独立滚动容器：回到顶部要滚它，不是 window */
const mainRef = ref(null)
const showTop = ref(false)
/** 换页进场的 class 在 a/b 两档间交替（见模板注释） */
const animClass = ref('')

function onMainScroll(e) {
  showTop.value = e.target.scrollTop > 480
}
function scrollTop() {
  mainRef.value?.scrollTo({ top: 0, behavior: 'smooth' })
}

function closeMobileNav() {
  mobileNavOpen.value = false
}

function onShellKeydown(event) {
  if (event.key === 'Escape') closeMobileNav()
  // ⌘K / Ctrl+K：全局快速跳转。阻止默认（Chrome 的 ⌘K 是地址栏搜索）
  if ((event.metaKey || event.ctrlKey) && String(event.key).toLowerCase() === 'k') {
    event.preventDefault()
    paletteOpen.value = !paletteOpen.value
  }
}

function onWideScreen(event) {
  if (event.matches) closeMobileNav()
}

const wideScreen = window.matchMedia('(min-width: 761px)')

// 路由变化后收起抽屉（点了项目 / 切了页就不该留一层遮住内容）
watch(() => route.fullPath, closeMobileNav)

// 换页（不是换片段）时回到顶部：`.main` 是独立滚动容器，不做这件事时
// 从长首页点进项目会直接落在新页面的中段，看起来像"没加载完"。
// ⚠️ 只看 `route.name`：精修页换片段是同一个 name，滚动位置要保留。
watch(() => route.name, () => {
  mainRef.value?.scrollTo({ top: 0 })
  showTop.value = false
  animClass.value = animClass.value === 'page-in-a' ? 'page-in-b' : 'page-in-a'
})

// 抽屉打开时锁住背景滚动（否则移动端下滑会把内容带着走，抽屉看起来"浮"在滚动页上）
watch(mobileNavOpen, (open) => {
  document.body.style.overflow = open ? 'hidden' : ''
})

// 登录 / 注册都是**壳外页面**（不套侧栏）。
// ⚠️ 曾经只认 'login'：注册页因此套在壳里，而 onMounted 的 loadProjects() 在未登录时
// 必然 401 → 统一跳登录 —— 直接打开 #/register（或从登录页点过来后刷新）会被顶回登录页，
// 邀请码注册整条路走不通。两个入口必须同一判据。
const isLogin = computed(() => route.name === 'login' || route.name === 'register')
const currentProject = computed(() => projectById(route.params.pid))
const stepName = computed(() => STEPS.find((s) => s.key === routeStep.value)?.name || '')
const routeStep = computed(() => {
  const name = String(route.name || '')
  // project-polish-clip（精修/:clip 片段编辑器）也算精修步，否则面包屑掉回「预处理」
  if (name === 'project-polish' || name === 'project-polish-clip') return 'polish'
  if (name === 'project-slices') return 'slices'
  if (name === 'project-settings') return ''
  return 'prepare'
})

// ---- 项目数据失效矩阵：任务结束 → 对应缓存失效 + 列表徽标刷新 ----
onJobDone((job) => {
  const pid = job.project
  const kind = job.kind || ''
  if (pid) {
    if (kind.startsWith('polish_')) invalidate(pid, 'polishMap')
    else if (kind === 'cut_all' || kind === 'cut_one') invalidate(pid, 'clips', 'events', 'status')
    else if (kind.startsWith('stage_') || kind === 'pipeline' || kind === 'correct' || kind.startsWith('ingest_')) {
      invalidate(pid, 'status', 'events', 'clips')
    }
    // 刷新失败已在 api()/workbench 里留痕（项目列表的 error 也由 store 记录），
    // 这里只是别让 rejection 冒出来
    loadProjectData(pid, { force: true }).catch(() => {})
  }
  loadProjects().catch(() => {})
})

watch(
  () => [route.params.pid, routeStep.value],
  ([pid, step]) => {
    if (pid) rememberLast(pid, step || 'prepare')
  },
  { immediate: true }
)

async function openProject({ pid, step }) {
  await router.push(`/p/${encodeURIComponent(pid)}/${step}`)
}

async function removeProject(p) {
  // 非破坏性（目录里一个文件都不动）→ 普通样式确认；但仍要确认，误点就从列表里没了
  const ok = await confirmDialog(
    '只把登记从服务器列表里去掉，不会动目录里的任何文件（yekitrim.toml、媒体、产物全部保留），随时可以重新导入。',
    { title: `移除项目「${p.name}」？` }
  )
  if (!ok) return
  try {
    await apiRemoveProject(p.id) // 内部已清前端本地痕迹（forgetProject）+ 刷新列表
    toast('已从列表移除（目录与文件保留在原处）')
    if (String(route.params.pid || '') === String(p.id)) router.push('/workbench')
  } catch (e) {
    toast(e.message, 'err')
  }
}



function openChangePassword() {
  pwdOld.value = ''
  pwdNew.value = ''
  pwdNew2.value = ''
  pwdError.value = ''
  showChangePassword.value = true
}

async function submitPassword() {
  if (pwdBusy.value) return
  pwdError.value = ''
  if (pwdNew.value !== pwdNew2.value) { pwdError.value = '两次输入的新密码不一致'; return }
  if (pwdNew.value.length < 8) { pwdError.value = '新密码至少 8 个字符'; return }
  pwdBusy.value = true
  try {
    await changePassword(pwdOld.value, pwdNew.value)
    showChangePassword.value = false
    toast('密码已修改，初始密码文件已删除')
  } catch (e) {
    pwdError.value = e?.message || '修改密码失败'
  } finally {
    pwdBusy.value = false
  }
}

async function doLogout() {
  const ok = await confirmDialog('退出当前账号？下次进入需要重新登录。', { title: '退出登录' })
  if (!ok) return
  closeJobs()
  await logout()
  router.replace({ name: 'login' })
}

/**
 * 本地免登录位（`GET /api/health.local_no_auth`）：由 `--local-no-auth` 打开
 * （`yekitrim-tray` 双击起服务时就是它）。拿来收起「退出登录」——本地模式下没有会话可退，
 * 那个入口只会把人带到不再需要的登录页。
 * ⚠️ 读失败就当作 false（照旧显示退出）——**别默认 true**，否则一次请求抖动就把入口藏了。
 */
const localNoAuth = ref(false)

onMounted(async () => {
  window.addEventListener('keydown', onShellKeydown)
  // Safari < 14 只有 addListener：直接调 addEventListener 会抛错、整个 onMounted 中断
  if (wideScreen.addEventListener) wideScreen.addEventListener('change', onWideScreen)
  else if (wideScreen.addListener) wideScreen.addListener(onWideScreen)
  if (!isLogin.value) loadProjects().catch(() => {})
  localNoAuth.value = await localNoAuthFromServer()
})

onUnmounted(() => {
  window.removeEventListener('keydown', onShellKeydown)
  if (wideScreen.removeEventListener) wideScreen.removeEventListener('change', onWideScreen)
  else if (wideScreen.removeListener) wideScreen.removeListener(onWideScreen)
  document.body.style.overflow = ''
})
</script>

<style scoped>
/* 壳：吸顶上下文栏只保留「我在哪」与「正在跑什么」。 */
.crumbs { position:sticky; top:0; z-index:9; min-height:58px; padding:11px 24px; margin:0 0 10px;
  background:var(--sticky-bg); backdrop-filter:blur(18px); border-bottom:1px solid var(--line-soft); }
.crumbs b { color:var(--ink); font-size:14px; font-weight:650; }
.crumbs .crumb-title { margin:0; font-size:14px; font-weight:650; line-height:1.4; color:var(--ink); }
.crumbs .sep { color:var(--ink-faint); }
.crumbs .crumb-proj { max-width:42vw; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;
  color:var(--main-deep); font-weight:600; }
.crumb-source { margin-left:6px; max-width:24vw; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.crumb-spacer { flex:1; }
.task-entry { min-height:30px; padding:4px 10px; }
.running-label { padding:0 6px; border-radius:var(--r-full); background:var(--main-soft);
  color:var(--main-deep); font-size:10px; line-height:18px; }
.task-entry .red-dot { display:none; }
.mobile-menu, .nav-mask { display:none; }
.cmdk-trigger { gap:5px; }
.cmdk-kbd { height:16px; font-size:10px; }
/* 回到顶部：贴在内容区右下（`.main` 是定位容器）。
   ⚠️ 2026-11-14 用户口径：**别贴底边**（贴底时正好压在页面最下方那条卡片边/浏览器底边附近，
   不好点）—— 上移到距底 ~200px 的位置。 */
.to-top { position:fixed; right:22px; bottom:200px; z-index:8; width:36px; height:36px;
  display:flex; align-items:center; justify-content:center;
  border-radius:var(--r-full); border:1px solid var(--line); background:var(--card);
  color:var(--ink-dim); box-shadow:var(--e-2); cursor:pointer;
  transition:color var(--dur-fast) var(--ease), border-color var(--dur-fast) var(--ease); }
.to-top:hover { color:var(--pink-text); border-color:var(--main); }
.pwd-banner { display:flex; align-items:center; gap:8px; padding:8px 24px; background:var(--amber-soft);
  color:var(--amber-deep); font-size:12px; border-bottom:1px solid var(--line); }
.content { width:100%; max-width:1640px; margin:0 auto; padding:16px 24px 64px; position:relative; z-index:1; }
.page-in-a { animation:page-in-a var(--dur-base) var(--ease); }
.page-in-b { animation:page-in-b var(--dur-base) var(--ease); }
@keyframes page-in-a { from { opacity:0; } to { opacity:1; } }
@keyframes page-in-b { from { opacity:0; } to { opacity:1; } }

@media (max-width:760px) {
  .nav-mask { display:block; position:fixed; inset:0; z-index:30; background:var(--scrim-soft);
    backdrop-filter:blur(2px); }
  .crumbs { min-height:52px; padding:9px 14px; margin-bottom:4px; gap:7px; }
  .mobile-menu { display:inline-flex; width:32px; height:32px; padding:0; justify-content:center; flex:none; }
  .crumbs .crumb-proj { max-width:38vw; }
  .crumb-source, .running-label { display:none; }
  .cmdk-trigger-label, .cmdk-kbd { display:none; }
  .cmdk-trigger { width:32px; height:32px; padding:0; justify-content:center; flex:none; }
  .to-top { right:12px; bottom:12px; }
  .task-entry { width:32px; height:32px; padding:0; justify-content:center; position:relative; }
  .task-label { display:none; }
  .task-entry .red-dot { display:block; position:absolute; top:3px; right:3px; margin:0; }
  .pwd-banner { padding:8px 14px; }
  .content { padding:12px 14px 44px; }
}

@media (max-width:420px) {
  .crumbs { gap:5px; }
  .crumbs .sep { display:none; }
  .crumbs .crumb-proj { max-width:34vw; }
}
</style>
