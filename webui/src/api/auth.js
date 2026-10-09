// 认证 API（M1）：登录 / 登出 / 当前用户 / 改密。
//
// 会话走 cookie（`yekitrim_sid`，HttpOnly）；写请求的 CSRF 注入由 `api/client.js` 统一完成，
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
  // 邀请码自助注册：成功即登录（服务端同时下发会话 cookie）。
  // 失败一律 403（码错/码用过/未开放都不区分），文案由服务端给。
  register: (username, password, invite) =>
    api('/api/auth/register', {
      method: 'POST',
      silent401: true,
      body: JSON.stringify({ username, password, invite })
    }),
  logout: () => api('/api/auth/logout', { method: 'POST' }),
  changePassword: (oldPassword, newPassword) =>
    api('/api/auth/password', {
      method: 'PUT',
      body: JSON.stringify({ old_password: oldPassword, new_password: newPassword })
    }),

  // ---- 账号与配额（**仅 admin**；非 admin 调会 403）----
  //
  // `listUsers` 只回展示字段（id / username / role / disabled / quota_bytes / created_at），
  // **不含口令哈希**。
  listUsers: () => api('/api/admin/users'),
  // ⚠️ `quotaBytes = 0` 的含义是「**跟随全局默认**」，**不是**「不限」——
  // 只有账号级与全局级都是 0 时才真的不限。别在 UI 上把 0 显示成「无限」。
  // 负数会被服务端拒成 400（不静默夹成 0）。
  setQuota: (username, quotaBytes) =>
    api(`/api/admin/users/${encodeURIComponent(username)}/quota`, {
      method: 'PUT',
      body: JSON.stringify({ quota_bytes: quotaBytes })
    })
}
