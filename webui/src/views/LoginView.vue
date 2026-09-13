<template>
  <div class="login-wrap">
    <div class="login-card">
      <div class="login-brand">
        <div class="brand-logo">V</div>
        <div>
          <div class="brand-name">V-TRIM</div>
          <div class="brand-sub">切片管理台</div>
        </div>
      </div>

      <div class="error-banner" v-if="error">{{ error }}</div>

      <form @submit.prevent="submit">
        <div class="mb-12">
          <label for="login-user">用户名</label>
          <input id="login-user" v-model="username" autocomplete="username" :disabled="busy" />
        </div>
        <div class="mb-16">
          <label for="login-pass">密码</label>
          <input
            id="login-pass"
            v-model="password"
            type="password"
            autocomplete="current-password"
            :disabled="busy"
          />
        </div>
        <button class="btn btn-primary login-btn" type="submit" :disabled="busy || !username || !password">
          <span v-if="busy" class="spinner"></span>
          {{ busy ? '登录中…' : '登录' }}
        </button>
      </form>

      <p class="small muted mt-16 m-0">
        账号由管理员创建。首次部署的管理员密码见服务启动日志，
        或数据目录下的 <span class="mono">initial-admin-password.txt</span>（首次登录成功后自动删除）。
      </p>
    </div>
  </div>
</template>

<script setup>
import { ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { login } from '../stores/auth'

const route = useRoute()
const router = useRouter()
const username = ref('')
const password = ref('')
const busy = ref(false)
const error = ref('')

async function submit() {
  if (busy.value) return
  busy.value = true
  error.value = ''
  try {
    await login(username.value.trim(), password.value)
    const redirect = typeof route.query.redirect === 'string' ? route.query.redirect : '/'
    router.replace(redirect.startsWith('/') ? redirect : '/')
  } catch (e) {
    // 后端文案直出：密码错 / 账号禁用 / 限流（尝试过多，15 分钟后再试）
    error.value = e?.message || '登录失败'
    password.value = ''
  } finally {
    busy.value = false
  }
}
</script>

<style scoped>
.login-wrap { min-height:100vh; display:flex; align-items:center; justify-content:center; padding:24px; }
.login-card { width:min(92vw,380px); background:var(--card); border:1px solid var(--line); border-radius:var(--r); padding:26px 24px; box-shadow:var(--shadow); }
.login-brand { display:flex; align-items:center; gap:10px; margin-bottom:20px; }
.brand-logo { width:38px; height:38px; border-radius:50%; background:linear-gradient(135deg,var(--main),var(--pink-light)); display:flex; align-items:center; justify-content:center; font-size:18px; color:#fff; box-shadow:var(--shadow-pink); }
.brand-name { font-size:16px; font-weight:700; color:var(--main-deep); line-height:1.2; }
.brand-sub { font-size:9px; color:var(--ink-faint); letter-spacing:.1em; }
.login-btn { width:100%; justify-content:center; }
</style>
