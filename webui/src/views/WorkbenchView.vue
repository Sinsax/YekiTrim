<template>
  <div>
    <div class="page-head">
      <div>
        <h1>工作台</h1>
        <div class="sub">左侧项目树按「预处理 → 切片 → 精修」直达；这里负责把录像变成项目</div>
      </div>
      <div class="actions">
        <button class="btn" @click="refresh"><Icon name="refresh" :size="14" /> 刷新</button>
      </div>
    </div>

    <div v-if="wb.error" class="error-banner"><Icon name="alert" :size="15" /> {{ wb.error }}</div>

    <!-- ═════ 新增项目：先选来源，再走对应导入方式 ═════ -->
    <div class="card card-blk mb-12">
      <div class="row spread mb-8">
        <b class="t" style="margin:0"><Icon name="plus" :size="15" /> 新增项目</b>
        <span class="muted small">选一个来源，下面切换成对应的导入方式</span>
      </div>

      <div class="wiz-srcs">
        <div
          v-for="s in sources"
          :key="s.key"
          class="wiz-src"
          :class="{ on: source === s.key, off: s.disabled }"
          @click="!s.disabled && (source = s.key)"
        >
          <div class="big"><Icon :name="s.icon" :size="28" /></div>
          <b>{{ s.title }}</b>
          <div class="hint">{{ s.desc }}</div>
          <span v-if="s.badge" class="chip n mt-6">{{ s.badge }}</span>
        </div>
      </div>

      <!-- ① 服务器文件（管理员）：任意位置浏览 -->
      <div v-if="source === 'local'" class="src-body">
        <template v-if="isAdmin">
          <div class="row">
            <div class="flex-1 minw-240">
              <label>目录（服务器上的任意绝对路径，可留空去托管目录）</label>
              <input v-model.trim="localDir" placeholder="例如 F:/media/2025-09-12" @keyup.enter="browseLocal">
            </div>
            <button class="mini-btn mt-20" @click="browseLocal">浏览…</button>
            <button class="mini-btn mt-20" :disabled="!localDir" @click="goUpLocal">上一级</button>
          </div>
          <div class="row" style="gap:8px;margin-top:8px;flex-wrap:wrap" v-if="localRoots.length">
            <span class="muted small" style="flex:none">快捷入口</span>
            <button v-for="r in localRoots" :key="r.path" class="mini-btn" @click="jumpLocal(r.path)">{{ r.label }}</button>
          </div>
          <div v-if="fsError" class="error-banner mt-8">{{ fsError }}</div>
          <div class="file-box mt-8">
            <div v-if="fsLoading" class="row muted small"><span class="spinner"></span> 读取中…</div>
            <template v-else>
              <div v-for="d in fsDirs" :key="'d-' + d" class="file-row" @click="enterLocalDir(d)">
                <span class="file-icon"><Icon name="folder" :size="15" /></span><span class="mono">{{ d }}/</span>
              </div>
              <div
                v-for="f in fsFiles"
                :key="'f-' + f"
                class="file-row video"
                :class="{ selected: localInput === f }"
                @click="pickLocalFile(f)"
              >
                <span class="file-icon"><Icon :name="isVideoFile(f) ? 'film' : 'file'" :size="15" /></span>
                <span class="mono">{{ f }}</span>
                <span v-if="isVideoFile(f)" class="muted small">视频</span>
              </div>
              <div v-if="!fsDirs.length && !fsFiles.length" class="empty small">空目录</div>
            </template>
          </div>
          <div v-if="localDir" class="row mt-8" style="gap:8px">
            <span class="muted small flex-1">
              项目将建在 <span class="mono">{{ localDir }}</span>
              <template v-if="localInput">，输入文件 <span class="mono">{{ localInput }}</span></template>
            </span>
            <button class="btn btn-primary" :disabled="creating || !localDir" @click="createFromLocal">
              <span v-if="creating" class="spinner"></span> 新建项目
            </button>
          </div>
          <p class="muted small mt-8">
            只读服务器上已存在的文件，**不接受上传**；目录里已有 <span class="mono">vtrim.toml</span>
            时只登记/打开、不覆盖。
          </p>
        </template>
        <div v-else class="empty small">「服务器文件」来源仅管理员可用。你的项目由管理员指派，或让管理员配置录播姬自动入库。</div>
      </div>

      <!-- ② 录播姬录制：待处理的会话 + 扫描 -->
      <div v-if="source === 'recorder'" class="src-body">
        <template v-if="pending.length">
          <SessionRow v-for="r in pending" :key="r.session_id" :r="r" @changed="reloadIngest">
            <template #actions="{ r: row }">
              <template v-if="isAdmin">
                <select v-model="assignTo[row.session_id]" class="mini-select">
                  <option value="">指派给…</option>
                  <option v-for="u in users" :key="u.id" :value="u.id">{{ u.username }}</option>
                </select>
                <button class="mini-btn" :disabled="!assignTo[row.session_id]" @click="assign(row)">指派并入库</button>
              </template>
            </template>
          </SessionRow>
        </template>
        <div v-else class="empty small">
          没有待处理的录制。配置好录播姬后，录完一场会自动出现在这里；也可用下面的兜底扫描导入历史录像。
        </div>
        <div class="row mt-12">
          <button v-if="isAdmin" class="btn" :disabled="scanning" @click="scanNow">
            <span v-if="scanning" class="spinner"></span> 扫描录制目录
          </button>
          <router-link class="mini-btn" to="/ingest">全部记录与导入规则</router-link>
          <span v-if="ingestReady === false" class="muted small">
            录播姬尚未接入：本机执行 <span class="mono">vtrim ingest token</span> 并配置
            <span class="mono">[ingest.recorder] root</span>
          </span>
        </div>
      </div>

      <!-- ③ B 站链接（P4 未实现，如实标注） -->
      <div v-if="source === 'bilibili'" class="src-body">
        <div class="empty small">
          B 站链接导入（输入视频地址 → 下载 → 建项目）属于后续版本（P4），当前不可用。
          现在可以用「服务器文件」把已下载好的录像建成项目。
        </div>
      </div>
    </div>

    <!-- ═════ 录制导入（最近）：活动流，只看不进 ═════ -->
    <div class="card card-blk">
      <div class="row spread mb-8">
        <b class="t" style="margin:0">录制导入（最近）</b>
        <router-link class="mini-btn" to="/ingest">全部记录</router-link>
      </div>
      <SessionRow v-for="r in recent" :key="r.session_id" :r="r" @changed="reloadIngest" />
      <div v-if="!recent.length" class="hint">暂无导入记录</div>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import Icon from '../components/Icon.vue'
import SessionRow from '../components/ingest/SessionRow.vue'
import { authState } from '../stores/auth'
import { createLocalProject, listTenantFiles } from '../stores/project'
import { toast } from '../stores/toast'
import { ingestApi } from '../api/ingest'
import { ingestState, loadRecords, setReady } from '../stores/ingest'
import { loadProjects, wb } from '../stores/workbench'
import { api } from '../api/client'

const isAdmin = computed(() => authState.user?.role === 'admin')

// 来源选择：不同来源 → 不同导入方式（本地浏览 / 录播姬待处理 / B站链接）
const source = ref('local')
const sources = computed(() => [
  {
    key: 'local',
    title: '服务器文件',
    desc: '浏览服务器上已有的录像，直接建项目',
    icon: 'folder',
    badge: isAdmin.value ? '' : '仅管理员',
    disabled: !isAdmin.value
  },
  {
    key: 'recorder',
    title: '录播姬录制',
    desc: '录完自动入库；历史/漏推送可扫描导入',
    icon: 'download',
    badge: ''
  },
  {
    key: 'bilibili',
    title: 'B 站链接',
    desc: '粘贴视频地址下载后建项目（后续版本）',
    icon: 'link',
    badge: 'P4',
    disabled: true
  }
])

// ---- ① 服务器文件 ----
const localDir = ref('')
const localParent = ref('')
const localInput = ref('')
const localRoots = ref([])
const fsDirs = ref([])
const fsFiles = ref([])
const fsLoading = ref(false)
const fsError = ref('')
const creating = ref(false)

const VIDEO_EXTS = ['mp4', 'mkv', 'flv', 'mov', 'm4v', 'webm', 'avi', 'ts']
function isVideoFile(name) {
  return VIDEO_EXTS.includes(String(name).split('.').pop().toLowerCase())
}

/**
 * 读目录。
 * `enter` 传"要进入的子目录名"，**由服务端拼路径**——前端做字符串拼接会把服务端回传的
 * `\\?\F:\a` 拼成 `\\?\F:\a/b` 这种非法形式（Windows ERROR_INVALID_NAME 123）。
 */
async function browseLocal(dir = localDir.value, enter = '') {
  fsLoading.value = true
  fsError.value = ''
  try {
    const d = await listTenantFiles(dir, enter)
    localDir.value = d.dir || dir
    localParent.value = d.parent || ''
    localRoots.value = d.roots || []
    fsDirs.value = d.dirs || []
    fsFiles.value = d.files || []
    localInput.value = ''
  } catch (e) {
    fsDirs.value = []
    fsFiles.value = []
    fsError.value = '目录读取失败：' + e.message
  } finally {
    fsLoading.value = false
  }
}

function jumpLocal(p) {
  browseLocal(p)
}

function enterLocalDir(name) {
  browseLocal(localDir.value, name)
}

function goUpLocal() {
  if (localParent.value) browseLocal(localParent.value)
}

function pickLocalFile(name) {
  localInput.value = localInput.value === name ? '' : name
}

async function createFromLocal() {
  creating.value = true
  try {
    const r = await createLocalProject({ dir: localDir.value, input: localInput.value })
    toast('项目已创建')
    await loadProjects()
    if (r?.id) window.location.hash = `#/p/${r.id}/prepare`
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    creating.value = false
  }
}

// ---- ② 录播姬录制 ----
const assignTo = ref({})
const users = ref([])
const scanning = ref(false)
const ingestReady = computed(() => ingestState.ready)

/** 待处理的会话：未指派（待分配池）或失败，需要人做决定 */
const pending = computed(() =>
  (ingestState.records || []).filter((r) => !r.project_id && (r.state === 'unassigned' || r.state === 'failed'))
)
const recent = computed(() => (ingestState.records || []).slice(0, 4))

async function reloadIngest() {
  await loadRecords(isAdmin.value ? 'unassigned' : 'mine')
  if (isAdmin.value) {
    try {
      const all = await ingestApi.records('all')
      const merged = new Map()
      for (const r of [...(ingestState.records || []), ...(all || [])]) merged.set(r.session_id, r)
      ingestState.records = Array.from(merged.values())
    } catch {}
  }
}

async function loadUsers() {
  if (!isAdmin.value) return
  try {
    const d = await api('/api/admin/users')
    users.value = d.users || []
  } catch {
    users.value = authState.user ? [authState.user] : []
  }
}

async function assign(row) {
  const uid = assignTo.value[row.session_id]
  if (!uid) return
  try {
    await ingestApi.assign(row.session_id, Number(uid), row.auto_pipeline || 'detect')
    toast('已指派，正在入库')
    await reloadIngest()
    await loadProjects()
  } catch (e) {
    toast(e.message, 'err')
  }
}

async function scanNow() {
  scanning.value = true
  try {
    const r = await ingestApi.scanNow()
    toast(`新登记 ${r?.registered ?? 0} 个完整录像`)
    await reloadIngest()
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    scanning.value = false
  }
}

async function refresh() {
  await loadProjects()
  await reloadIngest()
}

onMounted(async () => {
  await loadProjects()
  try {
    const h = await api('/api/health')
    setReady(h?.ingest_ready !== false)
  } catch {}
  await Promise.all([loadUsers(), reloadIngest()])
  if (isAdmin.value) browseLocal(localDir.value)
})
</script>

<style scoped>
/* 只补基准样式里没有的：来源面板与磁贴内文字的对齐细节 */
.wiz-src .hint { margin-top:4px; }
.src-body { margin-top:14px; border-top:1px dashed var(--line-soft); padding-top:14px; }
</style>
