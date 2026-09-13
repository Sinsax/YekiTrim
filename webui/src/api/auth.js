// 认证 API（M1）：登录 / 登出 / 当前用户 / 改密。
//
// 会话走 cookie（`vtrim_sid`，HttpOnly）；写请求的 CSRF 注入由 `api/client.js` 统一完成，
// 调用方不需要关心。
import { api } from './client'

export const authApi = {
  me: () => api('/api/me', { silent401: true }),
  login: (username, password) =>
    api('/api/auth/login', {
      method: 'POST',
      silent401: true,
      body: JSON.stringify({ username, password })
    }),
  logout: () => api('/api/auth/logout', { method: 'POST' }),
  changePassword: (oldPassword, newPassword) =>
    api('/api/auth/password', {
      method: 'PUT',
      body: JSON.stringify({ old_password: oldPassword, new_password: newPassword })
    })
}
