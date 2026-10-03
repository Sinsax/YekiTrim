<template>
  <div class="login">
    <div class="login-card">
      <div class="logo">
        <!-- 品牌标：用户提供的正式 logo（1024² 去透明边后居中方形）。
             资源在 src/assets/logo.png，由 Vite 打包（带 hash）；favicon 那份在 public/。
             ⚠️ 不要再改回内联 SVG —— 那是正式 logo 到位前的临时占位。 -->
        <img :src="logoUrl" alt="YekiTrim" />
      </div>
      <h1>YekiTrim</h1>
      <div class="sub">切片工作台 · 账号登录</div>

      <div class="error-banner" v-if="error">{{ error }}</div>

      <form @submit.prevent="submit">
        <div class="fld">
          <label for="login-user">用户名</label>
          <input id="login-user" v-model="username" autocomplete="username" :disabled="busy" />
        </div>
        <div class="fld">
          <label for="login-pass">密码</label>
          <input
            id="login-pass"
            v-model="password"
            type="password"
            autocomplete="current-password"
            :disabled="busy"
          />
        </div>
        <button class="btn btn-primary" style="width:100%;justify-content:center" type="submit" :disabled="busy || !username || !password">
          <span v-if="busy" class="spinner"></span>
          {{ busy ? '登录中…' : '登 录' }}
        </button>
      </form>

      <div class="hint mt-12">
        账号由管理员创建。首次部署的管理员密码见服务启动日志，或数据目录下的
        <span class="mono">initial-admin-password.txt</span>（首次登录成功后自动删除）。
      </div>
      <!-- 只在注册开放时露出入口（读 /api/health 的能力位；服务端仍是最终裁判） -->
      <div v-if="registrationEnabled" class="row mt-8">
        <router-link class="muted small" to="/register">没有账号？用邀请码注册</router-link>
      </div>
    </div>
  </div>
</template>

<script setup>
import { onMounted, ref } from 'vue'
import logoUrl from '../assets/logo.png'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../api/client'
import { login } from '../stores/auth'

const route = useRoute()
const router = useRouter()
const username = ref('')
const password = ref('')
const busy = ref(false)
const error = ref('')
/** 注册总闸（`[auth] registration_enabled`）：关着就不显示「用邀请码注册」入口 */
const registrationEnabled = ref(false)

onMounted(async () => {
  try {
    const h = await api('/api/health')
    registrationEnabled.value = h?.registration_enabled === true
  } catch {
    registrationEnabled.value = false
  }
})

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
