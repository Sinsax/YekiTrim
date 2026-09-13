import { createApp } from 'vue'
import App from './App.vue'
import router from './router'
import { setUnauthorizedHandler, redirectToLogin } from './api/client'
import { clearAuth } from './stores/auth'
import './styles/tokens.css'
import './styles/app.css'

// 任意请求 401（会话过期）→ 清状态 + 回登录页（带回跳）。
// 在这里接线而不是在 client.js 里 import store：避免 client ↔ store 循环依赖。
setUnauthorizedHandler(() => {
  clearAuth()
  redirectToLogin()
})

createApp(App).use(router).mount('#app')
