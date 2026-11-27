<template>
  <div>
    <!-- 一级文案按术语表用「采集」；旧词「入库」进副标题（docs/plan/webui-simplification-2026-11.md） -->
    <UiPageHeader title="采集" description="入库 · 录播姬录完自动变项目">
      <template #actions>
        <button class="btn" @click="reload"><Icon name="refresh" :size="14" /> 刷新</button>
        <button v-if="isAdmin" class="btn" :disabled="scanning" @click="scan">
          <span v-if="scanning" class="spinner"></span> 扫描录制目录
        </button>
      </template>
    </UiPageHeader>
    <!-- 上次兜底扫描的结果：失败时必须能看到原因（跨机最常见：共享不可读 / 填了对方机器上的路径） -->
    <div v-if="isAdmin && scanText" class="scan-line" :class="{ bad: scanStatus && scanStatus.ok === false }">
      {{ scanText }}
    </div>

    <!-- ⚠️ 只在**真的问了并且答否**（`=== false`）时显示：`null` = 还没问到。
         以前写成 `!ready`，直接打开 #/ingest 会先闪一句"未接入"（其实只是还没问）。 -->
    <div v-if="ingestState.ready === false" class="empty mb-16">
      <UiEmpty inline icon="alert" tone="warn" title="录播姬接入尚未就绪"
               description="需要 [ingest] webhook_enabled = true 与 [ingest.recorder] root；本机跑 yekitrim ingest token 生成 webhook token。">
        <template #actions>
          <button class="mini-btn" @click="loadHealth">重新检查</button>
        </template>
      </UiEmpty>
    </div>

    <div v-if="error" class="error-banner"><Icon name="alert" :size="15" /> {{ error }}</div>

    <!-- 录制导入 -->
    <div class="block-frame">
      <div class="section-title row spread">
        <span>录制导入</span>
        <UiSegmented :model-value="filter" :options="filterOptions" aria-label="记录筛选"
                     @update:model-value="setFilter" />
      </div>
      <div class="card">
        <UiSkeleton v-if="ingestState.loading" variant="card" :count="3" />
        <UiEmpty v-else-if="!ingestState.records.length" inline icon="download"
                 :title="emptyTitle" description="录播姬录完一场就会出现在这里" />
        <div v-else class="records">
          <SessionRow v-for="r in ingestState.records" :key="r.session_id" :r="r" @changed="load">
            <template #actions="{ r: row }">
              <!-- 「任务指派给谁」是多用户概念：单人（`multi_user` 默认 false）不渲染。
                   隐藏 ≠ 删除：`ingestApi.assign` 与 `/api/admin/ingest/assign` 都保留。 -->
              <template v-if="isAdmin && multiUser && !row.owner_id">
                <select v-model="assignTo[row.session_id]" class="mini-select">
                  <option value="">指派给…</option>
                  <option v-for="u in users" :key="u.id" :value="u.id">{{ u.username }}</option>
                </select>
                <button class="mini-btn" :disabled="!assignTo[row.session_id]" @click="assign(row)">指派</button>
              </template>
            </template>
          </SessionRow>
        </div>
      </div>
    </div>

    <!-- 自动导入规则 -->
    <div class="block-frame">
      <div class="section-title row spread">
        <span>自动导入规则（我的房间）</span>
        <button class="mini-btn primary" :disabled="saving" @click="addRule">新增房间</button>
      </div>
      <div class="card">
        <div class="muted small mb-8">
          房间号见录播姬目录名（通常是 RoomId）
        </div>
        <UiEmpty v-if="!draft.length" inline icon="list" title="还没有自动导入规则"
                 description="房间号填录播姬目录名里的 RoomId；未映射的房间会进「待分配」" />
        <div v-else class="table-scroll">
        <table class="tb rules">
          <thead>
            <tr><th>房间号</th><th>完成后自动执行</th><th>启用</th><th></th></tr>
          </thead>
          <tbody>
            <tr v-for="(r, i) in draft" :key="'r' + i">
              <td><input v-model.trim="r.room_id" placeholder="23058" /></td>
              <td>
                <select v-model="r.auto_pipeline">
                  <option v-for="p in PIPELINES" :key="p.value" :value="p.value">{{ p.label }}</option>
                </select>
              </td>
              <td><input type="checkbox" v-model="r.enabled" /></td>
              <td><button class="mini-btn danger" @click="draft.splice(i, 1)">删除</button></td>
            </tr>
          </tbody>
        </table>
        </div>
        <div class="row mt-12">
          <button class="btn btn-primary" :disabled="saving" @click="save">保存规则</button>
          <span class="muted small">{{ hint }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, reactive, ref } from 'vue'
import Icon from '../components/Icon.vue'
import SessionRow from '../components/ingest/SessionRow.vue'
import UiPageHeader from '../components/ui/UiPageHeader.vue'
import UiSegmented from '../components/ui/UiSegmented.vue'
import UiSkeleton from '../components/ui/UiSkeleton.vue'
import UiEmpty from '../components/ui/UiEmpty.vue'
import { authApi } from '../api/auth'
import { authState } from '../stores/auth'
import { toast } from '../stores/toast'
import { multiUserKnown, refreshHealth } from '../stores/health'
import { PIPELINES, ingestApi } from '../api/ingest'
import {
  ingestState,
  loadRecords,
  loadRules,
  saveRules,
  setReady
} from '../stores/ingest'

const isAdmin = computed(() => authState.user?.role === 'admin')
/**
 * 多人位（`/api/health.multi_user`，默认 false = 单人）：单人没有「指派给谁」这回事。
 * 路由守卫在任何视图挂载之前已经 await 过 `multiUser()`（同一份 health 缓存），
 * 所以这里取**已知值**就够 —— 不会先渲染出下拉再消失。
 */
const multiUser = multiUserKnown()
const filters = [
  { value: 'mine', label: '我的' },
  { value: 'failed', label: '失败' },
  { value: 'unassigned', label: '待分配' }
]
const filter = ref('mine')
/** 分段控件的选项（`value` 与 `setFilter` 的入参一致）。
 *  **非 admin 不渲染「待分配」**：那是管理员池子，点了只会弹一句"只有管理员能看"。
 *  隐藏 ≠ 删除 —— `setFilter` 里那道守卫仍在，直接改状态也挡得住。 */
const filterOptions = computed(() => filters
  .filter((f) => isAdmin.value || f.value !== 'unassigned')
  .map((f) => ({ value: f.value, label: f.label })))
/** 空态文案随筛选档变：不同档位"没有记录"的含义不一样 */
const emptyTitle = computed(() => ({
  mine: '还没有导入记录',
  failed: '没有失败的记录',
  unassigned: '没有待分配的录像'
}[filter.value] || '暂无记录'))
const draft = ref([])
const assignTo = reactive({})
const users = ref([])
const saving = ref(false)
const scanning = ref(false)
/** 最近一次兜底扫描（`/api/admin/ingest` 的 `scan`；admin-only 接口，读缓存不触发探测） */
const scanStatus = ref(null)
const scanText = computed(() => {
  const s = scanStatus.value
  if (!s) return ''
  const when = s.at ? new Date(s.at * 1000).toLocaleString() : ''
  if (s.ok) return `上次扫描：${when} · 新登记 ${s.registered} 个`
  return `上次扫描失败（${when}）：${s.error || '未知原因'}${s.hint ? ' —— ' + s.hint : ''}`
})
const hint = ref('')
const error = computed(() => ingestState.error)

async function load() {
  const scope = filter.value === 'mine' ? 'mine' : filter.value
  await loadRecords(scope)
  await loadRules()
  draft.value = ingestState.rules.map(r => ({ ...r }))
  // 扫描状态（admin-only）：失败原因要留在页面上，别只在 toast 里闪一下
  if (isAdmin.value) {
    try {
      const d = await ingestApi.config()
      scanStatus.value = d?.scan || null
    } catch {
      scanStatus.value = null
    }
  }
  // 「指派给…」的候选账号：管理员走 `/api/admin/users` 全量（只回展示字段，不含口令）；
  // 普通管理员也至少能指派给自己。失败只退回自己（`api()` 已留痕，不弹错）。
  if (isAdmin.value && multiUser) {
    try {
      const d = await authApi.listUsers()
      users.value = Array.isArray(d?.users) ? d.users : []
    } catch {
      users.value = authState.user ? [authState.user] : []
    }
  } else if (isAdmin.value) {
    users.value = authState.user ? [authState.user] : []
  }
}

function setFilter(v) {
  if (v === 'unassigned' && !isAdmin.value) {
    toast('只有管理员能看待分配池', 'err')
    return
  }
  filter.value = v
  load()
}

function reload() {
  load()
}

function addRule() {
  draft.value.push({ room_id: '', auto_pipeline: 'detect', enabled: true })
}

async function save() {
  saving.value = true
  hint.value = ''
  try {
    const rules = draft.value
      .filter(r => String(r.room_id || '').trim())
      .map(r => ({ room_id: String(r.room_id).trim(), auto_pipeline: r.auto_pipeline, enabled: !!r.enabled }))
    await saveRules(rules)
    draft.value = ingestState.rules.map(r => ({ ...r }))
    toast('规则已保存')
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    saving.value = false
  }
}

async function assign(r) {
  const uid = Number(assignTo[r.session_id])
  if (!uid) return
  try {
    await ingestApi.assign(r.session_id, uid, r.auto_pipeline || 'detect')
    toast('已指派')
    load()
  } catch (e) {
    toast(e.message, 'err')
  }
}

async function scan() {
  scanning.value = true
  try {
    const d = await ingestApi.scanNow()
    toast(`扫描完成：新登记 ${d.registered} 个完整录像`)
    load()
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    scanning.value = false
  }
}

/** 能力位：决定上面那条「未接入」横幅显示不显示（null = 还没问到，别当 false）。
 *  ⚠️ 走 `stores/health.js` 的**唯一缓存**（别在这里再拉第二份 /api/health）：
 *  失败时它回落上一份已知载荷，从未拉到就返回 null ⇒ 保持 null，横幅不误报。 */
async function loadHealth() {
  const h = await refreshHealth()
  if (!h) return
  setReady(h.ingest_ready !== false)
}

onMounted(async () => {
  await loadHealth()
  await load()
})
</script>

<style scoped>
.records { display:flex; flex-direction:column; }
.rec-row { display:flex; gap:10px; align-items:flex-start; padding:10px 0; border-bottom:1px dashed var(--line-soft); }
.rec-row:last-child { border-bottom:none; }
.mini-select { padding:4px 8px; font-size:12px; width:auto; }
/* 窄屏：表格局部横向滚动，别让整页撑出横向滚动条 */
.table-scroll { overflow-x:auto; }
.rules { min-width:520px; }
.rules td { vertical-align:middle; }
.rules input:not([type="checkbox"]) { min-width:120px; }
.rules td:last-child { width:1%; white-space:nowrap; }
/* 上次扫描：成功一行灰字，失败要能看见原因 */
.scan-line { font-size:12.5px; color:var(--ink-dim); margin:-4px 0 12px; }
.scan-line.bad { color:var(--pink-text); }
</style>
