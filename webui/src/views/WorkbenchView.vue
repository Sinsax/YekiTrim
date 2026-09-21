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

    <!-- ═════ 切片总览：全部项目的切片卡片预览（进行中 / 已完成导出 两个列表） ═════ -->
    <HomeClipsOverview />

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

      <!-- ③ B 站链接：粘贴 → 解析预览 → 勾选分P → 下载 -->
      <div v-if="source === 'bilibili'" class="src-body">
        <!-- 下载器没装：先解决这个（管理员一键装，普通用户只能等） -->
        <div v-if="dl.ready === false" class="bili-gate">
          <div class="row" style="gap:8px; align-items:flex-start">
            <Icon name="alert" :size="15" />
            <div class="flex-1">
              <b>下载器未就绪</b>
              <div class="muted small mt-4">
                B 站下载需要 <span class="mono">yt-dlp</span>（{{ dl.license || 'Unlicense' }}，
                {{ dl.pinned_version ? '版本 ' + dl.pinned_version : '内置' }}）。
                <template v-if="isAdmin">点右边按钮下载并校验（约 17 MB，只下一次）。</template>
                <template v-else>请联系管理员在「设置 → 系统」里安装后再试。</template>
              </div>
              <div v-if="dlError" class="error-banner mt-6">{{ dlError }}</div>
            </div>
            <button v-if="isAdmin" class="btn btn-primary" :disabled="dlBusy" @click="installDownloader">
              <span v-if="dlBusy" class="spinner"></span> 下载并校验
            </button>
          </div>
        </div>

      <template v-else>
        <!-- 存放位置没配置：管理员给去处（仅管理员可见这张卡）；普通用户提交时后端会拒绝并提示联系管理员 -->
        <div v-if="isAdmin && rootConfigured === false" class="bili-gate">
          <div class="row" style="gap:8px; align-items:flex-start">
            <Icon name="alert" :size="15" />
            <div class="flex-1">
              <b>尚未配置项目存放位置</b>
              <div class="muted small mt-4">
                下载前需要先确定项目落到哪个目录。去
                <router-link to="/settings/integrations">设置 → 下载</router-link>
                里配置「项目存放位置」后即可开始下载。
              </div>
            </div>
            <router-link class="btn btn-primary" to="/settings/integrations">去配置</router-link>
          </div>
        </div>
        <div class="row" style="gap:8px">
            <input
              v-model="biliUrl"
              class="flex-1"
              :disabled="biliBusy"
              placeholder="粘贴视频链接或 BV 号，例如 https://www.bilibili.com/video/BV1xx  /  BV1xx"
              @keyup.enter="resolveBili"
            >
            <button class="btn btn-primary" :disabled="biliBusy || !biliUrl.trim()" @click="resolveBili">
              <span v-if="biliBusy && !biliInfo" class="spinner"></span> 解析
            </button>
          </div>
          <div v-if="biliError" class="error-banner mt-8"><Icon name="alert" :size="15" /> {{ biliError }}</div>
          <div v-if="biliHint" class="hint mt-6">{{ biliHint }}</div>

          <!-- 预览卡 -->
          <div v-if="biliInfo" class="bili-preview">
            <img v-if="biliInfo.cover" class="bili-cover" :src="biliInfo.cover" alt="" referrerpolicy="no-referrer">
            <div class="bili-meta flex-1">
              <b class="bili-title" :title="biliInfo.title">{{ biliInfo.title }}</b>
              <div class="muted small mt-4">
                {{ biliInfo.uploader || '（未知 UP 主）' }}
                <template v-if="biliInfo.duration_secs"> · {{ fmtClock(biliInfo.duration_secs) }}</template>
                <template v-if="biliInfo.danmaku_total"> · 弹幕 {{ fmtCount(biliInfo.danmaku_total) }}</template>
                <template v-if="biliInfo.view_total"> · 播放 {{ fmtCount(biliInfo.view_total) }}</template>
              </div>
              <div class="muted small mt-4">
                {{ biliInfo.pages.length }} 个分P · 已选 {{ pickedPages.length }} 个 ·
                预计占用 <b>{{ fmtBytes(pickedEstimate) }}</b>
                <span class="muted">（清晰度上限 {{ biliInfo.quality }}，在设置页改）</span>
              </div>
            </div>
          </div>

          <!-- 多P 勾选：默认全不选（P 多的时候全下会撑爆配额） -->
          <div v-if="biliInfo?.pages?.length" class="bili-pages">
            <div class="row spread bili-pages-head">
              <span class="muted small">
                <template v-if="biliInfo.pages.length > 1">选择要下载的分P（各建一个项目，可随时再下别的）</template>
                <template v-else>单 P 视频</template>
              </span>
              <div v-if="biliInfo.pages.length > 1" class="row" style="gap:6px">
                <button class="mini-btn" @click="pickAllPages">全选</button>
                <button class="mini-btn" @click="pickedPages = []">清空</button>
              </div>
            </div>
            <label
              v-for="p in biliInfo.pages"
              :key="p.index"
              class="bili-page"
              :class="{ on: pickedPages.includes(p.index) }"
            >
              <input type="checkbox" :value="p.index" v-model="pickedPages">
              <span class="mono bili-pnum">P{{ p.index }}</span>
              <span class="bili-pname flex-1" :title="p.part">{{ p.part || '（无标题）' }}</span>
              <span class="muted small">{{ p.duration_secs ? fmtClock(p.duration_secs) : '-' }}</span>
              <span class="muted small bili-pest">{{ p.estimate_bytes ? fmtBytes(p.estimate_bytes) : '-' }}</span>
            </label>
          </div>

          <div v-if="biliInfo" class="row mt-8" style="gap:8px">
            <span class="muted small flex-1">
              下载完成后自动建项目；<b>弹幕会一并抓取</b>（事件检测的热度排序要用）。
              <template v-if="!pickedPages.length">请先勾选分P。</template>
            </span>
            <button
              class="btn btn-primary"
              :disabled="biliSubmitting || !pickedPages.length || (isAdmin && rootConfigured === false)"
              @click="submitBili"
            >
              <span v-if="biliSubmitting" class="spinner"></span>
              开始下载{{ pickedPages.length > 1 ? `（${pickedPages.length} 个）` : '' }}
            </button>
          </div>

          <!-- 已提交的结果 -->
          <div v-if="biliSubmitted" class="bili-done">
            <div class="row spread">
              <b class="small">已提交 {{ biliSubmitted.accepted.length }} 个下载</b>
              <router-link class="mini-btn" to="/ingest">查看进度</router-link>
            </div>
            <div v-for="a in biliSubmitted.accepted" :key="a.session_id" class="muted small mt-4">
              · P{{ a.page }} {{ a.title }}
            </div>
            <div v-for="s in biliSubmitted.skipped" :key="s.session_id" class="muted small mt-4">
              · P{{ s.page }} 已跳过（{{ s.reason }}）
            </div>
          </div>

          <p class="muted small mt-8">
            当前只支持单个视频链接与它的分P。番剧 / 课程 / UP 主空间 / 收藏夹批量**不支持** —— 链接认不出来会明确报错。
          </p>
        </template>
      </div>
    </div>

    <!-- 表情包（本地贴纸库）：全局资产，仅管理员 —— 普通用户整块不显示 -->
    <StickerPacks v-if="isAdmin" />

  </div>
</template>

<script setup>
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import Icon from '../components/Icon.vue'
import ServerDirPicker from '../components/ServerDirPicker.vue'
import StickerPacks from '../components/StickerPacks.vue'
import HomeClipsOverview from '../components/HomeClipsOverview.vue'
import { authState } from '../stores/auth'
import { createLocalProject } from '../stores/project'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import { ingestApi } from '../api/ingest'
import { fmtBytes, fmtTime, ingestState, setReady } from '../stores/ingest'
import { fmtClock, fmtCount } from '../utils'
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
      desc: '粘贴视频地址，下载完成后自动建项目（弹幕一并抓取）',
      icon: 'link',
      badge: ''
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

// ---- ③ B站链接（P4）----
//
// 交互形状：粘贴 → 解析（薄壳，不落库）→ 勾选分P → 提交（每 P 一个会话/项目）。
// 下载与建项目都在服务端 worker 里跑，这里只负责"提交"，进度去采集记录页看。
const dl = reactive({
  /** null = 还没问到（别先闪一下"未就绪"）；true/false = 下载器可用位 */
  ready: null,
  verified: false,
  kind: '',
  pinned_version: '',
  license: ''
})
const dlBusy = ref(false)
const dlError = ref('')

const biliUrl = ref('')
const biliBusy = ref(false)
const biliError = ref('')
const biliHint = ref('')
const biliInfo = ref(null)
/// 选中的分P。**默认全不选**：23P 全下会直接撑爆配额，必须让用户主动选。
const pickedPages = ref([])
const biliSubmitting = ref(false)
const biliSubmitted = ref(null)
/// 存放位置是否已显式配置（`[storage] projects_root`）。null = 还没问到 / 查询失败
/// （此时**不拦**，让后端兜底——前端查不到就别替后端做决定）。
/// 只有管理员会查（config 是 admin-only 端点），普通用户提交时由后端按角色给文案。
const rootConfigured = ref(null)

/// 只统计选中的 P（预览卡上的"预计占用"要跟着勾选实时变）。
const pickedEstimate = computed(() => {
  const info = biliInfo.value
  if (!info) return 0
  return info.pages
    .filter((p) => pickedPages.value.includes(p.index))
    .reduce((sum, p) => sum + (p.estimate_bytes || 0), 0)
})

async function loadDownloader() {
  try {
    const d = await ingestApi.downloaderStatus()
    Object.assign(dl, {
      ready: d?.ready === true,
      verified: d?.verified === true,
      kind: d?.kind || '',
      pinned_version: d?.pinned_version || '',
      license: d?.license || ''
    })
  } catch {
    // 读不到就当作"未知"，不拦用户（提交时服务端还会拦一次）
    dl.ready = null
  }
}

async function installDownloader() {
  dlBusy.value = true
  dlError.value = ''
  try {
    const r = await ingestApi.installDownloader()
    await loadDownloader()
    toast(r?.verified ? '下载器已安装并通过校验' : '下载器已安装（未通过 sha256 校验，请检查）', r?.verified ? 'ok' : 'err')
  } catch (e) {
    dlError.value = e?.message || '安装失败'
    toast(dlError.value, 'err')
  } finally {
    dlBusy.value = false
  }
}

async function resolveBili() {
  biliBusy.value = true
  biliError.value = ''
  biliHint.value = ''
  biliSubmitted.value = null
  try {
    const d = await ingestApi.resolveBilibili(biliUrl.value.trim())
    biliInfo.value = d
    // 单P 默认勾上（只有一个选项时让用户再点一次是多余的）；
    // 多P 一律留空 —— 由用户自己决定下哪几 P
    pickedPages.value = d.pages.length === 1 ? [d.pages[0].index] : []
    if (d.pages.length > 1) {
      biliHint.value = `这是个 ${d.pages.length} 分P 的合集，选好要下载的 P（每个 P 会建成一个项目）。`
    }
  } catch (e) {
    biliInfo.value = null
    pickedPages.value = []
    biliError.value = e?.message || '解析失败'
  } finally {
    biliBusy.value = false
  }
}

function pickAllPages() {
  pickedPages.value = (biliInfo.value?.pages || []).map((p) => p.index)
}

async function submitBili() {
  biliSubmitting.value = true
  biliError.value = ''
  try {
    const r = await ingestApi.submitBilibili(biliUrl.value.trim(), pickedPages.value)
    biliSubmitted.value = r
    const n = r?.accepted?.length || 0
    toast(n ? `已提交 ${n} 个下载，完成后自动建项目` : '没有新的下载（都已存在项目）')
    // 提交后会话就在采集记录页里，提示用户去哪儿看进度
    if (n) await loadRecords().catch(() => {})
  } catch (e) {
    biliError.value = e?.message || '提交失败'
    toast(biliError.value, 'err')
  } finally {
    biliSubmitting.value = false
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
    // 下载器就绪位顺便从 health 拿一次（省一个请求），再单独查一次细节（版本/许可）
    if (h?.downloader) Object.assign(dl, { ready: h.downloader.ready === true, verified: h.downloader.verified === true, kind: h.downloader.kind || '' })
  } catch {} // /api/health 是高频成功路径，失败由 api() 记日志即可（不挡首屏）
  await loadRooms()
  await loadDownloader()
  // 管理员顺路查一次存放位置是否已配置（config 是 admin-only；失败不拦，后端兜底）
  if (isAdmin.value) {
    try {
      const d = await ingestApi.config()
      rootConfigured.value = !!(d?.storage?.projects_root || '').trim()
    } catch {}
  }
})

// 从设置页配置完回来要重查（组件被 keep-alive / 复用时 onMounted 不会重跑）
const route = useRoute()
watch(() => route.fullPath, (to, from) => {
  if (isAdmin.value && to !== from && String(to).startsWith('/workbench')) {
    ingestApi.config()
      .then((d) => { rootConfigured.value = !!(d?.storage?.projects_root || '').trim() })
      .catch(() => {})
  }
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

/* ── B站链接向导 ── */
/* 下载器未就绪：整块给"怎么解决"，但不给普通用户一个点不动的安装按钮 */
.bili-gate {
  padding:10px 12px; border:1px solid var(--line-soft); border-radius:8px;
  background:var(--bg-soft, transparent);
}
.bili-preview { display:flex; gap:12px; margin-top:12px; align-items:flex-start; }
.bili-cover {
  width:160px; flex:none; border-radius:6px; border:1px solid var(--line-soft);
  aspect-ratio:16/9; object-fit:cover; background:var(--bg-soft, transparent);
}
.bili-meta { min-width:0; }
.bili-title {
  display:block; font-size:14px; line-height:1.4;
  overflow:hidden; text-overflow:ellipsis; display:-webkit-box;
  -webkit-line-clamp:2; -webkit-box-orient:vertical;
}
/* 分P 列表：可滚，P 多时不把页面撑长 */
.bili-pages {
  margin-top:10px; max-height:280px; overflow-y:auto;
  border:1px solid var(--line-soft); border-radius:8px; padding:4px 8px;
}
.bili-pages-head { padding:4px 0 6px; position:sticky; top:0; background:var(--bg, transparent); z-index:1; }
.bili-page {
  display:flex; align-items:center; gap:10px; padding:5px 4px; cursor:pointer;
  border-radius:6px; border-bottom:1px dashed var(--line-soft);
}
.bili-page:last-child { border-bottom:none; }
.bili-page:hover { background:var(--bg-soft, transparent); }
.bili-page.on { background:var(--bg-soft, transparent); }
.bili-pnum { flex:none; font-size:12px; min-width:34px; }
.bili-pname { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:13px; }
.bili-pest { flex:none; min-width:64px; text-align:right; }
.bili-done {
  margin-top:10px; padding:10px 12px; border:1px solid var(--line-soft);
  border-radius:8px; background:var(--bg-soft, transparent);
}
</style>
