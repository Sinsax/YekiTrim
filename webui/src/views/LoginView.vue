<template>
  <div class="login">
    <!-- v7：居中单卡 → **左品牌 / 右表单** 分栏。左栏只做气质与一句话定位，
         不承载任何功能；≤860px 整块隐藏，回到原来的居中卡片。 -->
    <div class="login-shell">
      <aside class="login-brand" aria-hidden="true">
        <img class="login-brand-logo" :src="logoUrl" alt="" />
        <b class="login-brand-name">YekiTrim</b>
        <p class="login-brand-desc">把一场直播录播，剪成可以发布的片段。</p>
      </aside>
    <div class="login-card">
      <div class="logo">
        <!-- 品牌标：用户提供的正式 logo（1024² 去透明边后居中方形）。
             资源在 src/assets/logo.png，由 Vite 打包（带 hash）；favicon 那份在 public/。
             ⚠️ 不要再改回内联 SVG —— 那是正式 logo 到位前的临时占位。 -->
        <img :src="logoUrl" alt="YekiTrim" />
      </div>
      <h1>YekiTrim</h1>
      <!-- 宽屏下卡片里只留这一行（logo + 产品名与左侧品牌栏重复，见 scoped 里的 @media） -->
      <div class="sub">账号登录</div>

      <div class="error-banner shake" v-if="error" :key="errorSeq" role="alert">{{ error }}</div>

      <form @submit.prevent="submit">
        <div class="fld">
          <label for="login-user">用户名</label>
          <input id="login-user" ref="userRef" v-model="username" autocomplete="username" :disabled="busy" />
        </div>
        <div class="fld">
          <label for="login-pass">密码</label>
          <input
            id="login-pass"
            ref="passRef"
            v-model="password"
            type="password"
            autocomplete="current-password"
            :disabled="busy"
          />
        </div>
        <!-- ⚠️ 空表单**不要**置灰按钮：置灰时点它一点反应都没有（用户报「点了没反应」）。
             正确做法是让它可点，提交时给出可见错误并聚焦第一个空框（见 submit）。 -->
        <button class="btn btn-primary w-full jc-center" type="submit" :disabled="busy">
          <span v-if="busy" class="spinner"></span>
          {{ busy ? '登录中…' : '登 录' }}
        </button>
      </form>

      <div class="hint mt-12">
        账号由管理员创建；初始密码见数据目录下的
        <span class="mono">initial-admin-password.txt</span>
      </div>
      <!-- 只在**多人部署**且注册开放时露出入口（读 /api/health 的能力位；服务端仍是最终裁判）。
           单人（`multi_user` 默认 false）没有"用邀请码注册"这回事 —— 隐藏 ≠ 删除，
           `#/register` 路由与服务端开关原样保留。 -->
      <div v-if="multiUser && registrationEnabled" class="row mt-8">
        <router-link class="muted small" to="/register">没有账号？用邀请码注册</router-link>
      </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { onMounted, ref } from 'vue'
import logoUrl from '../assets/logo.png'
import { useRoute, useRouter } from 'vue-router'
import { login } from '../stores/auth'
import { healthPayload, multiUser as multiUserFromServer } from '../stores/health'

const route = useRoute()
const router = useRouter()
const username = ref('')
const password = ref('')
/** 空提交时把焦点送到第一个空框（错误文案在卡片顶部，光标落到位才算"有反馈"）。 */
const userRef = ref(null)
const passRef = ref(null)
const busy = ref(false)
const error = ref('')
/** 多人部署位（`/api/health.multi_user`，默认 false = 单人）：单人没有「用邀请码注册」入口 */
const multiUser = ref(false)
/** 注册总闸（`[auth] registration_enabled`）：关着就不显示「用邀请码注册」入口 */
const registrationEnabled = ref(false)

onMounted(async () => {
  // 复用 stores/health.js 的**唯一** health 缓存（别再这里直接 api('/api/health') 拉第二份）。
  // 单人优先：读失败 / 缺字段 ⇒ false ⇒ 入口不显示。
  multiUser.value = await multiUserFromServer()
  if (!multiUser.value) return
  // 注册总闸只在多人分支里才有意义（同一份缓存里的载荷，不会多发请求）。
  const h = healthPayload()
  registrationEnabled.value = h?.registration_enabled === true
})

/** 每次报错递增：错误条 `:key` 跟着变，元素重建，抖动动画才会再播一次。 */
const errorSeq = ref(0)
function showError(msg) {
  error.value = msg
  errorSeq.value += 1
}

async function submit() {
  if (busy.value) return
  error.value = ''
  // 空表单/缺一项：**给出可见错误 + 聚焦第一个空框**。以前按钮直接 `disabled`，
  // 点下去毫无反应（回车也一样），用户分不清是"没填"还是"坏了"。
  const name = username.value.trim()
  if (!name || !password.value) {
    showError(!name && !password.value
      ? '请输入用户名和密码'
      : (!name ? '请输入用户名' : '请输入密码'))
    ;(name ? passRef : userRef).value?.focus?.()
    return
  }
  busy.value = true
  try {
    await login(name, password.value)
    const redirect = typeof route.query.redirect === 'string' ? route.query.redirect : '/'
    router.replace(redirect.startsWith('/') ? redirect : '/')
  } catch (e) {
    // 后端文案直出：密码错 / 账号禁用 / 限流（尝试过多，15 分钟后再试）
    showError(e?.message || '登录失败')
    password.value = ''
  } finally {
    busy.value = false
  }
}
</script>

<style scoped>
.login-shell { display:grid; grid-template-columns:1fr auto 1fr; align-items:center;
  gap:var(--s-7); width:min(1040px,94vw); }
.login-brand { display:flex; flex-direction:column; align-items:flex-start; gap:var(--s-2);
  justify-self:end; max-width:360px; }
.login-brand-logo { width:64px; height:64px; object-fit:contain; }
.login-brand-name { font-size:var(--fs-2xl); font-weight:var(--fw-semi); color:var(--ink); letter-spacing:-.01em; }
.login-brand-desc { margin:0; font-size:var(--fs-base); line-height:1.7; color:var(--ink-dim); }
/* 入场：品牌栏与卡片各自淡入，错开 80ms，像是先亮起标题再弹出表单 */
.login-brand { animation:fadein .4s var(--ease) both; }
.login-card { animation:fadein .4s var(--ease) .08s both; }
@media (max-width:860px) {
  .login-shell { display:flex; justify-content:center; width:100%; padding:0 16px; }
  .login-brand { display:none; }
}
/* 宽屏：左侧已经有 logo + 产品名 + 一句话定位 ⇒ 卡片里**不再重复**（用户 2026-11-14：冗余），
   只留一行小标题「账号登录」把这张卡说清楚。窄屏品牌栏整块隐藏，这里恢复 logo + 名字。 */
@media (min-width:861px) {
  .login-card .logo, .login-card h1 { display:none; }
  .login-card .sub { margin-bottom:18px; font-size:var(--fs-md); font-weight:var(--fw-semi);
    color:var(--ink); }
}
</style>
