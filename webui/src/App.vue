<template>
  <!-- 登录页不套壳：没有已登录用户，侧栏/任务抽屉都无意义 -->
  <router-view v-if="isLogin" />
  <div v-else class="app-shell">
    <div v-if="authState.mustChangePassword" class="pwd-banner">
      <Icon name="alert" :size="14" />
      <span class="flex-1">仍在使用初始密码，请尽快修改（初始密码会出现在服务日志与初始密码文件里）。</span>
      <button class="mini-btn" @click="openChangePassword">立即修改</button>
    </div>
    <header class="topbar">
      <a class="brand" href="#/">
        <div class="brand-logo">V</div>
        <div>
          <div class="brand-name">V-TRIM</div>
          <div class="brand-sub">切片管理台</div>
        </div>
      </a>
      <nav class="nav-pills">
        <router-link class="nav-pill" to="/">总览</router-link>
        <router-link class="nav-pill" to="/ingest">入库</router-link>
        <router-link class="nav-pill" to="/jobs">任务</router-link>
        <router-link v-if="isAdmin" class="nav-pill" to="/settings">设置</router-link>
      </nav>
      <div class="top-actions">
        <button class="bell" title="运行任务" @click="showJobs = !showJobs">
          <Icon name="bell" :size="16" />
          <i v-if="hasRunning" class="red-dot"></i>
        </button>
        <div class="user-area">
          <button class="user-chip" :title="authState.user?.username" @click="showUser = !showUser">
            <span class="user-name">{{ authState.user?.username }}</span>
            <span class="chip p">{{ roleLabel }}</span>
          </button>
          <div v-if="showUser" class="user-pop" @click="showUser = false">
            <button class="mini-btn" @click="openChangePassword">修改密码</button>
            <button class="mini-btn danger" @click="doLogout">登出</button>
          </div>
        </div>
      </div>
    </header>
    <main class="wrap">
      <router-view />
    </main>
    <ToastHost />
    <AppDialog />
    <div v-if="showJobs" class="job-drawer">
      <div class="row spread mb-8"><b>运行任务</b><button class="mini-btn" @click="showJobs = false">关闭</button></div>
      <JobList />
    </div>

    <AppModal :show="showChangePassword" title="修改密码" @close="showChangePassword = false">
      <div class="error-banner" v-if="pwdError">{{ pwdError }}</div>
      <div class="mb-12">
        <label for="pwd-old">当前密码</label>
        <input id="pwd-old" v-model="pwdOld" type="password" autocomplete="current-password" />
      </div>
      <div class="mb-12">
        <label for="pwd-new">新密码（至少 8 位）</label>
        <input id="pwd-new" v-model="pwdNew" type="password" autocomplete="new-password" />
      </div>
      <div class="mb-12">
        <label for="pwd-new2">确认新密码</label>
        <input id="pwd-new2" v-model="pwdNew2" type="password" autocomplete="new-password" />
      </div>
      <div class="row mt-12">
        <button class="btn btn-primary" :disabled="pwdBusy" @click="submitPassword">
          <span v-if="pwdBusy" class="spinner"></span>{{ pwdBusy ? '提交中…' : '保存' }}
        </button>
        <button class="btn" :disabled="pwdBusy" @click="showChangePassword = false">取消</button>
      </div>
    </AppModal>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Icon from './components/Icon.vue'
import ToastHost from './components/ToastHost.vue'
import AppDialog from './components/AppDialog.vue'
import AppModal from './components/AppModal.vue'
import JobList from './components/JobList.vue'
import { useJobs } from './composables/useJobs'
import { authState, changePassword, isAdmin, logout } from './stores/auth'
import { toast } from './stores/toast'

const { hasRunning } = useJobs()
const route = useRoute()
const router = useRouter()
const showJobs = ref(false)
const showUser = ref(false)

const isLogin = computed(() => route.name === 'login')
const roleLabel = computed(() => (authState.user?.role === 'admin' ? '管理员' : '用户'))

// ---- 改密 ----
const showChangePassword = ref(false)
const pwdOld = ref('')
const pwdNew = ref('')
const pwdNew2 = ref('')
const pwdBusy = ref(false)
const pwdError = ref('')

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
  if (pwdNew.value !== pwdNew2.value) {
    pwdError.value = '两次输入的新密码不一致'
    return
  }
  if (pwdNew.value.length < 8) {
    pwdError.value = '新密码至少 8 个字符'
    return
  }
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
  showUser.value = false
  await logout()
  router.replace({ name: 'login' })
}
</script>

<style scoped>
.app-shell { min-height:100vh; }
.pwd-banner { display:flex; align-items:center; gap:8px; padding:8px 20px; background:var(--amber-soft); color:var(--amber-deep); font-size:12px; border-bottom:1px solid var(--line); }
.topbar { position:sticky; top:0; z-index:30; display:flex; align-items:center; gap:12px; padding:10px 20px; background:rgba(255,247,243,.88); backdrop-filter:blur(12px); border-bottom:1px solid var(--line); flex-wrap:wrap; }
.brand { display:flex; align-items:center; gap:10px; text-decoration:none; color:inherit; }
.brand-logo { width:36px; height:36px; border-radius:50%; background:linear-gradient(135deg,var(--main),var(--pink-light)); display:flex; align-items:center; justify-content:center; font-size:17px; color:#fff; box-shadow:var(--shadow-pink); }
.brand-name { font-size:15px; font-weight:700; color:var(--main-deep); line-height:1.2; }
.brand-sub { font-size:9px; color:var(--ink-faint); letter-spacing:.1em; }
.nav-pills { display:flex; gap:3px; background:var(--card); border:1px solid var(--line); border-radius:var(--r-full); padding:4px; box-shadow:0 2px 10px rgba(90,60,70,.05); }
.nav-pill { padding:6px 15px; border-radius:var(--r-full); font-size:13px; color:var(--ink-dim); cursor:pointer; text-decoration:none; }
.nav-pill.router-link-active { background:linear-gradient(135deg,var(--main),var(--main-deep)); color:#fff; font-weight:600; }
.nav-pill:hover { background:var(--bg-soft); color:var(--ink); }
.top-actions { display:flex; gap:8px; align-items:center; margin-left:auto; }
.bell { position:relative; width:34px; height:34px; border-radius:50%; background:var(--card); border:1px solid var(--line); color:var(--ink-dim); display:flex; align-items:center; justify-content:center; text-decoration:none; cursor:pointer; transition:all .15s; }
.bell:hover { color:var(--main-deep); border-color:var(--main); }
.red-dot { position:absolute; top:3px; right:3px; width:8px; height:8px; border-radius:50%; background:var(--red); border:2px solid var(--bg); }
.user-area { position:relative; }
.user-chip { display:flex; align-items:center; gap:6px; padding:5px 10px; border-radius:var(--r-full); border:1px solid var(--line); background:var(--card); cursor:pointer; font-size:12px; color:var(--ink); }
.user-chip:hover { border-color:var(--main); }
.user-name { font-weight:600; max-width:130px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.user-pop { position:absolute; right:0; top:40px; display:flex; flex-direction:column; gap:6px; padding:10px; background:var(--card); border:1px solid var(--line); border-radius:var(--r-sm); box-shadow:var(--shadow-hover); z-index:95; }
.wrap { max-width:1360px; margin:0 auto; padding:20px 20px 80px; position:relative; z-index:1; }
.job-drawer { position:fixed; top:60px; right:16px; width:320px; max-height:70vh; overflow:auto; background:var(--card); border:1px solid var(--line); border-radius:var(--r); padding:14px; box-shadow:var(--shadow-hover); z-index:90; }
</style>
