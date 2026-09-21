<template>
  <!-- 登录页不套壳 -->
  <router-view v-if="isLogin" />

  <div v-else class="shell">
    <SideNav
      ref="sideRef"
      @open="openProject"
      @remove="removeProject"
      @logout="doLogout"
      @open-jobs="openJobs()"
    />

    <div class="main">
      <div class="crumbs">
        <template v-if="route.name === 'workbench'">
          <b>首页</b>
        </template>
        <template v-else-if="route.name === 'ingest'">
          <b>入库</b><span class="sep">/</span>
          <span class="muted small">录制导入 · 自动导入规则</span>
        </template>
        <template v-else-if="route.meta?.settings">
          <b>设置</b><span class="sep">/</span><span class="muted small">{{ route.meta.title }}</span>
        </template>
        <template v-else-if="route.params.pid">
          <span class="crumb-proj" :title="currentProject?.name">{{ currentProject?.name || ('项目 #' + route.params.pid) }}</span>
          <span class="sep">/</span>
          <b>{{ stepName }}</b>
          <span v-if="currentProject?.source?.label" class="muted small" style="margin-left:8px">
            {{ currentProject.source.label }}
          </span>
        </template>
        <span style="flex:1"></span>
        <button class="mini-btn" title="运行任务" @click="openJobs()">
          <Icon name="bell" :size="13" /> 任务
          <span v-if="hasRunning" class="red-dot"></span>
        </button>
      </div>

      <div v-if="authState.mustChangePassword" class="pwd-banner">
        <Icon name="alert" :size="14" />
        <span class="flex-1">仍在使用初始密码，请尽快修改（初始密码会出现在服务日志与初始密码文件里）。</span>
        <button class="mini-btn" @click="openChangePassword">立即修改</button>
      </div>

      <main class="content">
        <!-- 按**项目 id** 做 key：同一步骤下换项目时 route record 相同，vue-router 会复用组件实例，
             setup/onMounted 不再执行 —— 于是整页还停在旧项目（只有面包屑变了），
             而预处理器保存时用的是**新** pid，会把 A 的字幕写进 B（用户实测"两个项目会串"）。
             跨步骤切换本来就靠组件类型不同重挂，这里的 key 不改变那条路径；
             也不能用 fullPath 做 key（那样精修页换片段也会整块重挂，丢掉原地切换）。 -->
        <router-view :key="route.params.pid || route.name" />
      </main>
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
    <ToastHost />
    <AppDialog />
  </div>
</template>

<script setup>
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Icon from './components/Icon.vue'
import SideNav from './components/side/SideNav.vue'
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

const isLogin = computed(() => route.name === 'login')
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
    '只把登记从服务器列表里去掉，不会动目录里的任何文件（vtrim.toml、媒体、产物全部保留），随时可以重新导入。',
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

onMounted(() => {
  if (!isLogin.value) loadProjects().catch(() => {})
})
</script>

<style scoped>
/* 壳：布局数值取自 multiuser.html（.shell/.side/.main/.crumbs），这里只补 sticky 吸顶 */
.crumbs { position:sticky; top:0; z-index:9; padding:12px 22px; margin:0 0 14px;
  background:rgba(255,247,243,.85); backdrop-filter:blur(10px); border-bottom:1px solid var(--line); }
.crumbs b { color:var(--main-deep); font-size:15px; }
.crumbs .sep { color:var(--ink-faint); }
/* 长项目名截断，别把「任务」按钮顶走 */
.crumbs .crumb-proj { max-width:42vw; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.pwd-banner { display:flex; align-items:center; gap:8px; padding:8px 22px; background:var(--amber-soft);
  color:var(--amber-deep); font-size:12px; border-bottom:1px solid var(--line); }
.content { padding:18px 22px 60px; position:relative; z-index:1; }
</style>
