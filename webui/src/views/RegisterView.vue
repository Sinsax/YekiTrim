<template>
  <div class="login">
    <div class="login-card">
      <div class="logo">Y</div>
      <h1>YekiTrim</h1>
      <div class="sub">切片工作台 · 邀请码注册</div>

      <div class="error-banner" v-if="error">{{ error }}</div>

      <!-- 总闸关着就别给表单：直接说清楚（服务端也会 403，这里只是别让人白填） -->
      <template v-if="enabled === false">
        <div class="empty small">当前未开放注册。请联系管理员生成邀请码（<span class="mono">yekitrim user invite</span>），
          或去设置页开启。</div>
        <router-link class="btn" style="width:100%;justify-content:center;margin-top:12px" to="/login">去登录</router-link>
      </template>

      <template v-else>
        <form @submit.prevent="submit">
          <div class="fld">
            <label for="reg-user">用户名</label>
            <input id="reg-user" v-model="username" autocomplete="username" :disabled="busy" placeholder="2~32 个字符">
          </div>
          <div class="fld">
            <label for="reg-pass">密码</label>
            <input
              id="reg-pass"
              v-model="password"
              type="password"
              autocomplete="new-password"
              :disabled="busy"
              placeholder="至少 8 位"
            />
          </div>
          <div class="fld">
            <label for="reg-invite">邀请码</label>
            <input id="reg-invite" v-model="invite" class="mono" :disabled="busy" placeholder="yekitrim_inv_…">
          </div>
          <button class="btn btn-primary w-full jc-center" type="submit" :disabled="busy || !canSubmit">
            <span v-if="busy" class="spinner"></span>
            {{ busy ? '注册中…' : '注 册' }}
          </button>
        </form>

        <div class="hint mt-12">
          邀请码由管理员生成，<b>一个码只能用一次</b>
        </div>
        <div class="row mt-8">
          <router-link class="muted small" to="/login">已有账号？去登录</router-link>
        </div>
      </template>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { register } from '../stores/auth'
import { refreshHealth } from '../stores/health'

const router = useRouter()
const username = ref('')
const password = ref('')
const invite = ref('')
const busy = ref(false)
const error = ref('')
/** null = 还没问到（别闪"未开放"）；false = 总闸关着 */
const enabled = ref(null)

const canSubmit = computed(() => !!username.value.trim() && !!password.value && !!invite.value.trim())

onMounted(async () => {
  // 走 stores/health.js 的**唯一缓存**（与登录页/设置页同源，别在这里再拉第二份）。
  // 读不到（null）就保持 `enabled = null` = "还没问到"：前端不当裁判，交给服务端判定。
  const h = await refreshHealth()
  if (h) enabled.value = h.registration_enabled === true
})

async function submit() {
  if (busy.value) return
  busy.value = true
  error.value = ''
  try {
    await register(username.value.trim(), password.value, invite.value.trim())
    router.replace('/')
  } catch (e) {
    // 服务端文案直出：注册未开放 / 邀请码无效或已被使用 / 用户名已被占用 / 尝试次数过多
    error.value = e?.message || '注册失败'
    password.value = ''
  } finally {
    busy.value = false
  }
}
</script>
