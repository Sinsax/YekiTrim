<template>
  <div>
    <div class="page-head">
      <div>
        <h1>入库</h1>
        <div class="sub">录播姬录完自动变项目 · 房间映射决定归谁与自动跑到哪一步</div>
      </div>
      <div class="actions">
        <button class="btn" @click="reload"><Icon name="refresh" :size="14" /> 刷新</button>
        <button v-if="isAdmin" class="btn" :disabled="scanning" @click="scan">
          <span v-if="scanning" class="spinner"></span> 扫描录制目录
        </button>
      </div>
    </div>

    <div v-if="!ingestState.ready" class="empty mb-16">
      <div class="empty-icon"><Icon name="alert" :size="24" /></div>
      <div>录播姬接入尚未就绪（需要 <span class="mono">[ingest] webhook_enabled = true</span>
        与 <span class="mono">[ingest.recorder] root</span>）。</div>
      <div class="small muted mt-8">
        本机执行 <span class="mono">vtrim ingest token</span> 生成 webhook token，并把录播姬的录制目录填进配置。
      </div>
    </div>

    <div v-if="error" class="error-banner"><Icon name="alert" :size="15" /> {{ error }}</div>

    <!-- 录制导入 -->
    <div class="block-frame">
      <div class="section-title row spread">
        <span>录制导入</span>
        <span class="subnav">
          <button v-for="f in filters" :key="f.value" class="mini-btn"
                  :class="{ selected: filter === f.value }" @click="setFilter(f.value)">{{ f.label }}</button>
        </span>
      </div>
      <div class="card">
        <div v-if="ingestState.loading" class="row muted small"><span class="spinner"></span> 加载中…</div>
        <div v-else-if="!ingestState.records.length" class="muted small">暂无记录</div>
        <div v-else class="records">
          <SessionRow v-for="r in ingestState.records" :key="r.session_id" :r="r" @changed="load">
            <template #actions="{ r: row }">
              <template v-if="isAdmin && !row.owner_id">
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
          房间号见录播姬的目录名（通常是 RoomId）。录完自动归到你的账号，并按策略继续跑管线。
        </div>
        <div v-if="!draft.length" class="muted small">还没有规则：未映射的房间会进「待分配」，由管理员指派。</div>
        <table v-else class="rules">
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
import { api } from '../api/client'
import { authState } from '../stores/auth'
import { toast } from '../stores/toast'
import { PIPELINES, ingestApi } from '../api/ingest'
import {
  ingestState,
  loadRecords,
  loadRules,
  saveRules
} from '../stores/ingest'

const isAdmin = computed(() => authState.user?.role === 'admin')
const filters = [
  { value: 'mine', label: '我的' },
  { value: 'failed', label: '失败' },
  { value: 'unassigned', label: '待分配' }
]
const filter = ref('mine')
const draft = ref([])
const assignTo = reactive({})
const users = ref([])
const saving = ref(false)
const scanning = ref(false)
const hint = ref('')
const error = computed(() => ingestState.error)

async function load() {
  const scope = filter.value === 'mine' ? 'mine' : filter.value
  await loadRecords(scope)
  await loadRules()
  draft.value = ingestState.rules.map(r => ({ ...r }))
  if (isAdmin.value) {
    try {
      const me = await api('/api/me')
      users.value = [me.user]
      // 账号列表属于管理页（P5）；这里至少能指派给自己
    } catch {}
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

async function retry(r) {
  try {
    await ingestApi.retry(r.session_id)
    toast('已重新排队')
    load()
  } catch (e) {
    toast(e.message, 'err')
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

onMounted(load)
</script>

<style scoped>
.records { display:flex; flex-direction:column; }
.rec-row { display:flex; gap:10px; align-items:flex-start; padding:10px 0; border-bottom:1px dashed var(--line-soft); }
.rec-row:last-child { border-bottom:none; }
.mini-select { padding:4px 8px; font-size:12px; width:auto; }
.rules { width:100%; border-collapse:collapse; }
.rules th { text-align:left; font-size:12px; color:var(--ink-dim); font-weight:600; padding:4px 6px; }
.rules td { padding:4px 6px; }
.rules input[type="text"], .rules input:not([type]) { min-width:120px; }
</style>
