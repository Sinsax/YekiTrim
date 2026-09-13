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
        <template v-else-if="route.name === 'settings'">
          <b>设置</b><span class="sep">/</span><span class="muted small">平台配置与模型</span>
        </template>
        <template v-else-if="route.params.pid">
          <span>{{ currentProject?.name || ('项目 #' + route.params.pid) }}</span>
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
        <router-view />
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
  if (name === 'project-polish') return 'polish'
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
  const ok = await confirmDialog('只注销项目（删 vtrim.toml），不会触碰媒体文件。', {
    title: `删除项目「${p.name}」？`,
    danger: true
  })
  if (!ok) return
  try {
    await apiRemoveProject(p.id)
    toast('项目已删除')
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
.pwd-banner { display:flex; align-items:center; gap:8px; padding:8px 22px; background:var(--amber-soft);
  color:var(--amber-deep); font-size:12px; border-bottom:1px solid var(--line); }
.content { padding:18px 22px 60px; position:relative; z-index:1; }
</style>
