// 服务端能力位（`GET /api/health`）。
//
// 目前只有一个：**本地免登录**（`--local-no-auth`，`yekitrim-tray` 双击起服务时就是它）。
// 它决定三处界面口径，所以必须**同源**，不能在每个用到的地方各拉一次：
//   ① 侧栏用户块收起「退出登录」（本地模式没有会话可退）；
//   ② 设置页隐藏「账号」子页（本地模式没有注册/邀请这回事）；
//   ③ 路由守卫把 `/settings/account` 挡回程序配置页（直接输网址也要挡）。
//
// ⚠️ **读失败当 `false`**（= 照旧显示那些入口）：别默认 true，否则一次请求抖动就把入口藏了。
// ⚠️ 结果**缓存整个会话**：这个位在服务端进程生命周期内是固定的，而换服务端必然伴随重新加载页面。
import { api } from '../api/client'

let pending = null
let cached = false

/** 是否是本地免登录服务端（只问一次；失败当 false）。 */
export function localNoAuth() {
  if (!pending) {
    pending = api('/api/health')
      .then((h) => {
        cached = h?.local_no_auth === true
        return cached
      })
      .catch(() => {
        cached = false
        return false
      })
  }
  return pending
}

/** 已经问出来过的那份答案（没问过时是 false —— 与"读失败"同一口径，别当成"已知非本地"）。 */
export function localNoAuthKnown() {
  return cached
}

/** 仅用于测试/切换服务端：丢掉缓存。 */
export function resetHealthCache() {
  pending = null
  cached = false
}
