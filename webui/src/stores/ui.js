// 全局 UI 开关（弹层这类"跨组件触发"的状态放这里，避免 router-view 事件透传）。
import { reactive } from 'vue'

const LS_SIDE = 'yekitrim.ui.sideCollapsed'
// ⚠️ 旧键 `yekitrim.ui.fragRows`（1 行 / 2 行）已废弃：用户 2026-11-22 改了语义 ——
// 「展开」= 横排显示全部片段，「收起」= 整条折起一个都不显示。旧值不再读取（默认展开）。
const LS_FRAG_OPEN = 'yekitrim.ui.fragOpen'

// 启动时同步读取：侧栏折叠态必须在首帧就生效（否则会先展开再收起来，闪一下）
function readSideCollapsed() {
  try {
    return localStorage.getItem(LS_SIDE) === '1'
  } catch {
    return false
  }
}

/** 精修页顶部片段卡**是否展开**（默认展开），同样首帧生效 */
function readFragOpen() {
  try {
    return localStorage.getItem(LS_FRAG_OPEN) !== '0'
  } catch {
    return true
  }
}


export const ui = reactive({
  jobs: false,
  /** 左侧栏是否折叠成图标轨（64px）；持久化在 localStorage */
  sideCollapsed: readSideCollapsed(),
  /** 精修页顶部片段卡：展开（横排一行）/ 收起（整条不显示） */
  fragOpen: readFragOpen(),
  /** 面包屑尾部（`App.vue` 渲染）：精修页放**当前片段标题**，其他页面留空走项目来源标签 */
  crumbTail: '',
})

export function openJobs() {
  ui.jobs = true
}

export function closeJobs() {
  ui.jobs = false
}

export function toggleSide() {
  ui.sideCollapsed = !ui.sideCollapsed
  try {
    localStorage.setItem(LS_SIDE, ui.sideCollapsed ? '1' : '0')
  } catch {}
}

/** 面包屑尾部（空串 = 回到项目来源标签）。**页面卸载必须清掉**，否则会带到别的页面。 */
export function setCrumbTail(text) {
  ui.crumbTail = String(text || '')
}

/** 片段卡 展开 ↔ 收起（收起 = 整条折起、一个片段都不显示） */
export function toggleFragOpen() {
  ui.fragOpen = !ui.fragOpen
  try {
    localStorage.setItem(LS_FRAG_OPEN, ui.fragOpen ? '1' : '0')
  } catch {}
}
