// 认证状态（M1）。
//
// 认证路径与"单人还是多人"无关：**没有免登录分支**，单人部署＝只有一个 admin 账号的部署，
// 未登录访问任意页面都会被路由守卫送到登录页（服务端 §3.5.1）。
import { reactive } from 'vue'
import { authApi } from '../api/auth'
import { addClient } from './log'

export const authState = reactive({
  /** 是否已经问过服务端"我是谁"（启动时一次） */
  ready: false,
  user: null,
  mustChangePassword: false
})

export function isAuthed() {
  return !!authState.user
}

export function isAdmin() {
  return authState.user?.role === 'admin'
}

export function clearAuth() {
  authState.user = null
  authState.mustChangePassword = false
}

/** 启动引导：GET /api/me → 200 记录用户 / 401 视为未登录。 */
export async function loadMe() {
  try {
    const d = await authApi.me()
    authState.user = d?.user || null
    authState.mustChangePassword = !!d?.must_change_password
  } catch (e) {
    clearAuth()
    // 401 是"没登录"，正常；其它状态码（500/网络）也走到这里，必须留痕，
    // 否则服务端故障会伪装成"未登录"，用户只会看到被弹回登录页。
    if (e.status !== 401) {
      addClient('warn', `会话检查失败（${e.status || '网络错误'}）：${e.message}，按未登录处理`, 'auth')
    }
  } finally {
    authState.ready = true
  }
  return isAuthed()
}

export async function login(username, password) {
  const d = await authApi.login(username, password)
  authState.user = d?.user || null
  authState.mustChangePassword = !!d?.must_change_password
  authState.ready = true
  return authState.user
}

/**
 * 邀请码注册：成功后服务端已下发会话 cookie，这里与 login 一样落状态（注册即登录）。
 * 失败（未开放 / 码无效 / 用户名占用 / 限流）都抛错，文案直出给调用方。
 */
export async function register(username, password, invite) {
  const d = await authApi.register(username, password, invite)
  authState.user = d?.user || null
  authState.mustChangePassword = false
  authState.ready = true
  return authState.user
}

export async function logout() {
  try {
    await authApi.logout()
  } catch (e) {
    // 会话可能已过期：登出失败也要清本地状态
  }
  clearAuth()
}

export async function changePassword(oldPassword, newPassword) {
  await authApi.changePassword(oldPassword, newPassword)
  authState.mustChangePassword = false
}
