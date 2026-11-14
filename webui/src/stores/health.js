// 服务端能力位（`GET /api/health`）。
//
// 这里是**整个前端唯一**的 /api/health 缓存：`local_no_auth`（本地免登录）与首页
// 「开始使用」检查（ffmpeg / ffprobe / dhampir_ready / llm / downloader）都从这里读，
// 别在每个用到的地方各拉一次。
//
// `local_no_auth` 决定三处界面口径，所以必须**同源**：
//   ① 侧栏用户块收起「退出登录」（本地模式没有会话可退）；
//   ② 设置页隐藏「账号」子页（本地模式没有注册/邀请这回事）；
//   ③ 路由守卫把 `/settings/account` 挡回程序配置页（直接输网址也要挡）。
//
// `multi_user`（`[auth] multi_user`，默认 false = 单人）决定**多人相关 UI** 的显隐：
//   ① 设置「系统」页的邀请码 / 账号配额（`InviteManager` / `UserQuotaManager`）；
//   ② 登录页「用邀请码注册」入口；
//   ③ 入库「指派给…」下拉、设置页 `multiOnly` 子页（见 `utils.js::SETTINGS_SECTIONS`）。
//   **隐藏 ≠ 删除**：接口与组件本体全部保留，`multi_user = true` 时原样回来。
//
// ⚠️ **读失败当 `false`**（= 单人优先）：`local_no_auth` 旧口径是"失败照旧显示"，
//    而 `multi_user` 反过来 —— 拿不准时**不显示**多人入口，别让抖动把邀请码露出来。
// ⚠️ 两个位复用**同一份** health 缓存：`localNoAuth()` / `multiUser()` 只有第一次真发请求。
//    `multi_user` 在服务端进程生命周期内是固定的（改配置要重启），而换服务端必然伴随重新加载页面。失败则允许下一次导航重试。
// ⚠️ `refreshHealth()` 是给首页「重新检查」用的：失败**保留上一份已知载荷**，
//    别把已经读到的 checklist 退回"未读取"。
import { api } from '../api/client'

let pending = null
let fetched = false
let cached = false
let cachedMultiUser = false
let payload = null

function fetchHealth() {
  if (!pending) {
    pending = api('/api/health')
      .then((h) => {
        payload = h || null
        cached = h?.local_no_auth === true
        cachedMultiUser = h?.multi_user === true
        fetched = true
        return payload
      })
      .finally(() => {
        pending = null
      })
  }
  return pending
}

/** 是否是本地免登录服务端（只问一次；失败当 false）。 */
export function localNoAuth() {
  if (fetched) return Promise.resolve(cached)
  return fetchHealth().then(
    () => cached,
    () => {
      cached = false
      return false
    }
  )
}

/** 已经问出来过的那份答案（没问过时是 false —— 与"读失败"同一口径，别当成"已知非本地"）。 */
export function localNoAuthKnown() {
  return cached
}

/**
 * 是否多用户部署（`GET /api/health.multi_user`，只问一次；失败/缺字段当 false = 单人）。
 * **复用同一份 health 缓存**：这里不会再发一次请求。
 */
export function multiUser() {
  if (fetched) return Promise.resolve(cachedMultiUser)
  return fetchHealth().then(
    () => cachedMultiUser,
    () => {
      cachedMultiUser = false
      return false
    }
  )
}

/** 已经问出来过的那份答案（没问过 / 读失败时是 false —— 单人优先，别当成"已知多人"）。 */
export function multiUserKnown() {
  return cachedMultiUser
}

/** 最近一次成功拉到的完整 health 载荷（没拉过 / 从未成功时是 null）。 */
export function healthPayload() {
  return payload
}

/**
 * 重新拉一次 `/api/health`（在飞合并）。返回完整载荷；
 * 失败时回落上一份已知载荷（从未成功过就是 null），**不抛** —— 调用方无需 try/catch。
 */
export async function refreshHealth() {
  try {
    return await fetchHealth()
  } catch {
    return payload
  }
}

/** 仅用于测试/切换服务端：丢掉缓存。 */
export function resetHealthCache() {
  pending = null
  fetched = false
  cached = false
  cachedMultiUser = false
  payload = null
}
