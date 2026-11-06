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
// ⚠️ **读失败当 `false`**（= 照旧显示那些入口）：别默认 true，否则一次请求抖动就把入口藏了。
// ⚠️ `localNoAuth()` 成功之后**不再重复请求**（这个位在服务端进程生命周期内是固定的，
//    而换服务端必然伴随重新加载页面）；失败则允许下一次导航重试。
// ⚠️ `refreshHealth()` 是给首页「重新检查」用的：失败**保留上一份已知载荷**，
//    别把已经读到的 checklist 退回"未读取"。
import { api } from '../api/client'

let pending = null
let fetched = false
let cached = false
let payload = null

function fetchHealth() {
  if (!pending) {
    pending = api('/api/health')
      .then((h) => {
        payload = h || null
        cached = h?.local_no_auth === true
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
  payload = null
}
