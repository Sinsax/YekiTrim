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
          :title="s.desc"
          @click="!s.disabled && (source = s.key)"
        >
          <Icon :name="s.icon" :size="15" />
          <b>{{ s.title }}</b>
          <span v-if="s.badge" class="chip n">{{ s.badge }}</span>
        </div>
      </div>

      <!-- ① 服务器文件（仅管理员可见；非管理员连来源按钮都没有） -->
      <div v-if="source === 'local' && isAdmin" class="src-body">
        <ServerDirPicker
          mode="file"
          :start-dir="localDir"
          :selected="localInput"
          @update:dir="localDir = $event"
          @pick-file="pickLocalFile"
        />
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
      </div>

      <!-- ② 录播姬录制：待处理的会话 + 扫描 -->
      <div v-if="source === 'recorder'" class="src-body">
        <!-- 用户唯一要做的：填直播间。未对接录播姬时整块置灰，且**不给管理员向的配置指引** -->
        <div class="room-add" :class="{ off: roomsAvailable === false }">
          <div class="row" style="gap:8px">
            <input
              v-model="roomInput"
              class="flex-1"
              :disabled="roomsAvailable !== true || roomBusy"
              placeholder="直播间号或链接，例如 12345 / https://live.bilibili.com/12345"
              @keyup.enter="addRoom"
            >
            <button class="btn btn-primary" :disabled="roomsAvailable !== true || roomBusy || !roomInput.trim()" @click="addRoom">
              <span v-if="roomBusy" class="spinner"></span> 添加
            </button>
          </div>
          <div v-if="roomsAvailable !== null" class="hint mt-4">
            <template v-if="roomsAvailable">开播自动录制，录完入库到你的项目；一个直播间只能由一个人登记。</template>
            <template v-else>录播姬未接入，暂时不能添加</template>
          </div>
        </div>

        <!-- 我登记的直播间：每行可展开看该房间录了哪些文件（单个导入） -->
        <div v-if="rooms.length" class="room-list">
          <div v-for="r in rooms" :key="r.room_id" class="room-item">
            <div class="room-row">
              <button class="mini-btn room-toggle" :title="expanded[r.room_id] ? '收起录像' : '展开录像'"
                      :aria-expanded="expanded[r.room_id] ? 'true' : 'false'" @click="toggleRoom(r)">
                {{ expanded[r.room_id] ? '▾' : '▸' }}
              </button>
              <b class="mono room-id">{{ r.room_id }}</b>
              <span class="muted small room-title" :title="r.title">{{ r.title || '（未取到标题）' }}</span>
              <span class="chip" :class="roomChip(r)">{{ roomState(r) }}</span>
              <span v-if="recOf(r).files" class="muted small room-rec"
                    :title="recOf(r).latest ? '最新 ' + fmtTime(recOf(r).latest) : ''">
                已录 {{ recOf(r).files }} 个 · {{ fmtBytes(recOf(r).bytes) }}
              </span>
              <button class="mini-btn danger" :disabled="roomBusy" @click="removeRoom(r)">删除</button>
            </div>

            <div v-if="expanded[r.room_id]" class="room-files">
              <div v-if="roomFiles[r.room_id]?.loading" class="row muted small">
                <span class="spinner"></span> 读取录像目录…
              </div>
              <div v-else-if="roomFiles[r.room_id]?.error" class="error-banner">{{ roomFiles[r.room_id].error }}</div>
              <template v-else>
                <div v-if="!roomFiles[r.room_id]?.files.length" class="muted small">
                  录像目录里没有这个房间的文件（还没录过，或已导入后被搬走）。
                </div>
                <div v-else class="row spread file-list-head">
                  <span class="muted small">共 {{ roomFiles[r.room_id].total }} 个录像</span>
                  <button class="mini-btn" :disabled="roomFiles[r.room_id].loading" @click="loadRoomFiles(r, true)">刷新</button>
                </div>
                <div v-for="f in roomFiles[r.room_id]?.files || []" :key="f.rel" class="rec-row">
                  <span class="mono small rec-name" :title="f.rel">{{ f.name }}</span>
                  <span class="muted small rec-meta">{{ fmtBytes(f.size) }} · {{ fmtTime(f.mtime) }}</span>
                  <span class="chip" :class="recChip(f)">{{ f.state_label }}</span>
                  <button v-if="f.state === 'new'" class="mini-btn primary"
                          :disabled="importing[f.rel]" @click="importFile(r, f)">
                    <span v-if="importing[f.rel]" class="spinner"></span> 导入
                  </button>
                  <router-link v-else-if="f.project_id" class="mini-btn" :to="`/p/${f.project_id}/slices`">打开项目</router-link>
                </div>
                <div v-if="roomFiles[r.room_id].shown < roomFiles[r.room_id].total" class="muted small mt-6">
                  只显示了最新的 {{ roomFiles[r.room_id].shown }} 个。
                </div>
              </template>
            </div>
          </div>
        </div>
        <div v-else-if="!roomsLoading" class="muted small mt-8">
          还没有登记直播间。填一个房间号，开播时会自动录制并入库到你的项目。
        </div>

        <!-- 管理员：待分配兜底 + 扫描 + 接入状态；普通用户完全不显示这一块 -->
        <template v-if="isAdmin">
          <div class="row mt-12" style="gap:8px">
            <button class="btn" :disabled="scanning" @click="scanNow">
              <span v-if="scanning" class="spinner"></span> 扫描录制目录
            </button>
            <router-link class="mini-btn" to="/ingest">全部记录与导入规则</router-link>
            <button class="mini-btn" :disabled="recorderTesting" @click="testRecorder">
              <span v-if="recorderTesting" class="spinner"></span> 测试录播姬连接
            </button>
          </div>
          <div v-if="recorderInfo" class="hint mt-6">{{ recorderInfo }}</div>
          <div v-if="ingestReady === false" class="hint mt-6">
            录播姬尚未接入：在 <b>设置页 →「录播姬（采集）」</b> 里填录像目录与 API 地址、生成 webhook token
            （命令行等价：<span class="mono">vtrim ingest token</span> +
            <span class="mono">[ingest.recorder] root</span>）。
            未归属或入库失败的录像在 <router-link to="/ingest">全部记录与导入规则</router-link> 里处理。
          </div>
        </template>
      </div>

      <!-- ③ B 站链接（P4 未实现，如实标注） -->
      <div v-if="source === 'bilibili'" class="src-body">
        <div class="empty small">
          B 站链接导入（输入视频地址 → 下载 → 建项目）属于后续版本（P4），当前不可用。
          <template v-if="isAdmin">现在可以用「服务器文件」把已下载好的录像建成项目。</template>
          <template v-else>你的项目由管理员指派，或让管理员配置录播姬自动入库。</template>
        </div>
      </div>
    </div>

    <!-- 表情包（本地贴纸库）：全局资产，仅管理员 —— 普通用户整块不显示 -->
    <StickerPacks v-if="isAdmin" />

  </div>
</template>

<script setup>
import { computed, onMounted, reactive, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import ServerDirPicker from '../components/ServerDirPicker.vue'
import StickerPacks from '../components/StickerPacks.vue'
import { authState } from '../stores/auth'
import { createLocalProject } from '../stores/project'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import { ingestApi } from '../api/ingest'
import { fmtBytes, fmtTime, ingestState, setReady } from '../stores/ingest'
import { loadProjects, wb } from '../stores/workbench'
import { api } from '../api/client'

const isAdmin = computed(() => authState.user?.role === 'admin')

// 来源选择：不同来源 → 不同导入方式（本地浏览 / 录播姬待处理 / B站链接）
// 「服务器文件」是 admin-only 来源（`/api/fs/list`、`kind:"local"` 都对非管理员 403），
// 所以非管理员**整条不显示** —— 不给一个点不动、只写着"仅管理员"的入口。
const source = ref('recorder')
const sources = computed(() => {
  const list = []
  if (isAdmin.value) {
    list.push({ key: 'local', title: '服务器文件', desc: '浏览服务器上已有的录像，直接建项目', icon: 'folder', badge: '' })
  }
  list.push(
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
  )
  return list
})
// 选中的来源必须始终是"看得见且可用"的那一个（角色变化 / 列表变化时自动落到第一个可用的）
watch(sources, (list) => {
  if (!list.some((s) => s.key === source.value && !s.disabled)) {
    source.value = (list.find((s) => !s.disabled) || list[0])?.key || 'recorder'
  }
}, { immediate: true })

// ---- ② 我的直播间（录播姬）----
const rooms = ref([])
/// null = 还没问到（别先闪一下「未接入」）；true/false = 录播姬可用位
const roomsAvailable = ref(null)
const roomsLoading = ref(false)
const roomInput = ref('')
const roomBusy = ref(false)
const recorderTesting = ref(false)
const recorderInfo = ref('')

/** 我的直播间 + 录播姬可用位。普通用户的响应里没有 `reason`（不暴露管理配置）。 */
async function loadRooms() {
  roomsLoading.value = true
  try {
    const d = await api('/api/rooms')
    rooms.value = d?.rooms || []
    roomsAvailable.value = d?.available === true
    if (isAdmin.value && d?.reason) recorderInfo.value = d.reason
  } catch (e) {
    rooms.value = []
    roomsAvailable.value = false
    if (isAdmin.value) recorderInfo.value = '读取直播间失败：' + (e?.message || '')
  } finally {
    roomsLoading.value = false
  }
}

async function addRoom() {
  const v = roomInput.value.trim()
  if (!v || roomBusy.value || !roomsAvailable.value) return
  roomBusy.value = true
  try {
    await api('/api/rooms', { method: 'POST', body: JSON.stringify({ url: v }) })
    roomInput.value = ''
    toast('已添加直播间，开播时会自动录制')
    await loadRooms()
  } catch (e) {
    toast('添加失败：' + e.message, 'err')
  } finally {
    roomBusy.value = false
  }
}

async function removeRoom(r) {
  const ok = await confirmDialog(`删除直播间 ${r.room_id}？删除后不再自动录制（已录的片段与项目不受影响）。`, {
    title: '删除直播间？',
    danger: true
  })
  if (!ok) return
  roomBusy.value = true
  try {
    await api(`/api/rooms/${encodeURIComponent(r.room_id)}`, { method: 'DELETE' })
    toast('已删除')
    await loadRooms()
  } catch (e) {
    toast('删除失败：' + e.message, 'err')
  } finally {
    roomBusy.value = false
  }
}

function roomState(r) {
  if (r.recording) return '录制中'
  if (r.streaming) return '在播'
  if (!r.in_recorder) return '未在录播姬'
  return '未开播'
}
function roomChip(r) {
  if (r.recording) return 'r'
  if (r.streaming) return 'g'
  return 'n'
}

// ---- 房间行展开：该房间在录像目录里的文件（单个导入）----

/** 房间的录像汇总（服务端随 /api/rooms 一起回；没有就是 0）。 */
function recOf(r) {
  const rec = r?.recordings || {}
  return { files: rec.files || 0, bytes: rec.bytes || 0, latest: rec.latest || 0 }
}

/** 展开状态 + 每个房间的文件列表缓存（懒加载：点开才去读目录）。 */
const expanded = reactive({})
const roomFiles = reactive({})
const importing = reactive({})

async function loadRoomFiles(r, force = false) {
  const id = r.room_id
  if (roomFiles[id]?.loading) return
  if (roomFiles[id] && !force) return
  roomFiles[id] = { loading: true, error: '', files: [], total: 0, shown: 0 }
  try {
    const d = await api(`/api/rooms/${encodeURIComponent(id)}/recordings`, { label: '读取该房间的录像' })
    roomFiles[id] = {
      loading: false,
      error: '',
      files: d?.files || [],
      total: d?.total || 0,
      shown: d?.shown || 0
    }
  } catch (e) {
    roomFiles[id] = { loading: false, error: e.message, files: [], total: 0, shown: 0 }
  }
}

async function toggleRoom(r) {
  const id = r.room_id
  expanded[id] = !expanded[id]
  if (expanded[id]) await loadRoomFiles(r)
}

function recChip(f) {
  if (f.state === 'imported') return 'g'
  if (f.state === 'failed') return 'r'
  if (f.state === 'importing') return 'b'
  return 'n'
}

/** 导入**单个**录像：服务端建会话 + 归属本人 + 跑 auto_pipeline。 */
async function importFile(r, f) {
  if (importing[f.rel]) return
  importing[f.rel] = true
  try {
    await api(`/api/rooms/${encodeURIComponent(r.room_id)}/recordings/import`, {
      method: 'POST',
      body: JSON.stringify({ rel: f.rel }),
      label: '导入录像'
    })
    toast('已交给入库流程（跑完会出现在你的项目里）')
    await Promise.all([loadRoomFiles(r, true), loadRooms()])
  } catch (e) {
    toast('导入失败：' + e.message, 'err')
    await loadRoomFiles(r, true)
  } finally {
    importing[f.rel] = false
  }
}

async function testRecorder() {
  recorderTesting.value = true
  try {
    const d = await api('/api/admin/recorder/test', { method: 'POST', body: '{}' })
    recorderInfo.value = d?.ok
      ? `录播姬已连接 ${d.base}${d.version ? ' · v' + d.version : ''} · ${d.rooms} 个房间`
      : `未连接：${d?.error || '未知错误'}`
    toast(d?.ok ? '连接正常' : '连接失败', d?.ok ? '' : 'err')
  } catch (e) {
    toast('连接失败：' + e.message, 'err')
  } finally {
    recorderTesting.value = false
  }
}

// ---- ① 服务器文件（浏览逻辑在 ServerDirPicker，与「表情包 → 添加」共用一份）----
const localDir = ref('')
const localInput = ref('')
const creating = ref(false)

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
const scanning = ref(false)
const ingestReady = computed(() => ingestState.ready)

async function scanNow() {
  scanning.value = true
  try {
    const r = await ingestApi.scanNow()
    toast(`新登记 ${r?.registered ?? 0} 个完整录像`)
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    scanning.value = false
  }
}

async function refresh() {
  await loadProjects()
}

onMounted(async () => {
  await loadProjects()
  try {
    const h = await api('/api/health')
    setReady(h?.ingest_ready !== false)
  } catch {} // /api/health 是高频成功路径，失败由 api() 记日志即可（不挡首屏）
  await loadRooms()
})
</script>

<style scoped>
/* 来源选择已改紧凑一行，说明走 title 悬停 */
.src-body { margin-top:14px; border-top:1px dashed var(--line-soft); padding-top:14px; }
/* 「配置直播间」：录播姬不可用时整块置灰（用户只看到"暂时不能添加"，看不到管理配置） */
.room-add.off { opacity:.55; }
.room-list { margin-top:10px; }
.room-item { border-bottom:1px dashed var(--line-soft); }
.room-item:last-child { border-bottom:none; }
.room-row { display:flex; align-items:center; gap:10px; padding:6px 0; }
.room-toggle { flex:none; width:22px; padding:2px 0; text-align:center; }
.room-id { flex:none; font-size:13px; }
.room-title { flex:1; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.room-rec { flex:none; white-space:nowrap; }
/* 展开区：贴在房间行下面，左侧缩进对齐房间号 */
.room-files { margin:2px 0 8px 32px; padding-left:10px; border-left:2px solid var(--line-soft); }
.file-list-head { padding-bottom:2px; }
.rec-row { display:flex; align-items:center; gap:8px; padding:4px 0; flex-wrap:wrap; }
.rec-name { flex:1; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.rec-meta { flex:none; white-space:nowrap; }
</style>
