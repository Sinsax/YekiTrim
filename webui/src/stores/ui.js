// 全局 UI 开关（弹层这类"跨组件触发"的状态放这里，避免 router-view 事件透传）。
import { reactive } from 'vue'

export const ui = reactive({
  jobs: false
})

export function openJobs() {
  ui.jobs = true
}

export function closeJobs() {
  ui.jobs = false
}
