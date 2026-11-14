// 全局 UI 开关（弹层这类"跨组件触发"的状态放这里，避免 router-view 事件透传）。
import { reactive } from 'vue'

const LS_SIDE = 'yekitrim.ui.sideCollapsed'
const LS_FRAG = 'yekitrim.ui.fragRows'

// 启动时同步读取：侧栏折叠态必须在首帧就生效（否则会先展开再收起来，闪一下）
function readSideCollapsed() {
  try {
    return localStorage.getItem(LS_SIDE) === '1'
  } catch {
    return false
  }
}

/** 精修页片段卡显示几行（1 | 2），同样首帧生效 */
function readFragRows() {
  try {
    return localStorage.getItem(LS_FRAG) === '2' ? 2 : 1
  } catch {
    return 1
  }
}


export const ui = reactive({
  jobs: false,
  /** 左侧栏是否折叠成图标轨（64px）；持久化在 localStorage */
  sideCollapsed: readSideCollapsed(),
  /** 精修页顶部片段卡：1 行 / 2 行 */
  fragRows: readFragRows(),
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

/** 片段卡 1 行 ↔ 2 行 */
export function toggleFragRows() {
  ui.fragRows = ui.fragRows === 2 ? 1 : 2
  try {
    localStorage.setItem(LS_FRAG, String(ui.fragRows))
  } catch {}
}
