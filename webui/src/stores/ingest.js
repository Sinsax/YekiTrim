// 入库状态（M3）：最近的录播姬会话 + 我的房间映射。
import { reactive } from 'vue'
import { ingestApi } from '../api/ingest'

export const ingestState = reactive({
  records: [],
  rules: [],
  loading: false,
  error: '',
  /** `/api/health.ingest_ready`：false 时前端显示配置引导卡 */
  ready: null
})

export async function loadRecords(scope = 'mine') {
  ingestState.loading = true
  ingestState.error = ''
  try {
    const d = await ingestApi.records(scope)
    ingestState.records = d?.records || []
  } catch (e) {
    ingestState.error = e?.message || '读取入库记录失败'
    ingestState.records = []
  } finally {
    ingestState.loading = false
  }
  return ingestState.records
}

export async function loadRules() {
  try {
    const d = await ingestApi.channelMap()
    ingestState.rules = d?.rules || []
  } catch (e) {
    ingestState.rules = []
  }
  return ingestState.rules
}

export async function saveRules(rules) {
  const d = await ingestApi.saveChannelMap(rules)
  ingestState.rules = d?.rules || []
  return ingestState.rules
}

export function setReady(v) {
  ingestState.ready = !!v
}

/** 状态徽标样式类（与服务端 state 一致）。 */
export function stateClass(state) {
  switch (state) {
    case 'open': return 'p'
    case 'closing': return 'a'
    case 'finalized': return 'g'
    case 'failed': return 'r'
    case 'unassigned': return 'b'
    default: return 'n'
  }
}

export function fmtTime(secs) {
  if (!secs) return ''
  const d = new Date(Number(secs) * 1000)
  const pad = (n) => String(n).padStart(2, '0')
  return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

export function fmtBytes(n) {
  const v = Number(n) || 0
  if (v < 1024) return v + ' B'
  const units = ['KB', 'MB', 'GB', 'TB']
  let x = v / 1024
  let i = 0
  while (x >= 1024 && i < units.length - 1) {
    x /= 1024
    i++
  }
  return x.toFixed(1) + ' ' + units[i]
}
