<template>
  <div>
    <div class="page-head">
      <div>
        <span class="chip p">精修</span>
        <h1 class="mt-2">{{ dir ? title : '选择要精修的片段' }}</h1>
        <div class="sub">{{ dir || '从片段库选择一个已剪切片段进入精修编辑器' }}</div>
      </div>
      <div class="actions"></div>
    </div>

    <!-- 无切片：空态引导（不报错不白屏） -->
    <div v-if="!clips.length && !loading" class="card card-blk" style="max-width:520px;margin:48px auto;text-align:center;padding:36px">
      <b style="font-size:15px">还没有可精修的切片</b>
      <p class="small muted" style="margin:8px 0 16px">先在「切片」页生成片段，再回到这里逐段精修</p>
      <router-link class="btn btn-primary" :to="`/p/${pid}/slices`">去切片</router-link>
    </div>

    <template v-else>
      <div v-if="loading" class="card"><div class="skeleton"></div></div>

      <!-- 未初始化 -->
      <div v-else-if="!ready" class="card">
        <h2>未初始化精修项目</h2>
        <p class="muted small">将解析字幕/弹幕并创建 polish.toml。</p>
        <div class="row mt-8">
          <button class="btn btn-primary" :disabled="jobRunning" @click="initProject">
            <Icon name="sparkles" :size="14" /> 初始化
          </button>
        </div>
      </div>

      <template v-else>
        <!-- 顶部两卡：生成与导出（含贴纸来源） + 片段列表 -->
        <div class="top-cards">
          <div class="card">
            <div class="row spread mb-8">
              <h2 class="m-0 card-h2"><Icon name="sparkles" :size="15" /> 生成与导出</h2>
              <span class="muted small">LLM 完成后自动执行后处理</span>
            </div>
            <div class="row" style="gap:8px">
              <button class="btn btn-primary" :disabled="jobRunning" @click="runLlmJob"><Icon name="sparkles" :size="13" /> LLM 剪辑</button>
              <button class="btn" :disabled="jobRunning" @click="doSilence"><Icon name="volume" :size="13" /> 去静音</button>
              <button class="btn" :disabled="jobRunning" @click="renderModal = true"><Icon name="film" :size="13" /> 渲染导出</button>
            </div>
            <div v-if="job.id && (jobRunning || jobFinished)" class="job-box mt-8">
              <div class="row spread small">
                <b>{{ jobLabel(job.kind) }}</b>
                <div class="row">
                  <span v-if="jobRunning" class="muted small mono">运行 {{ jobTimeText }}</span>
                  <button v-if="jobRunning" class="mini-btn danger" :disabled="canceling" @click="cancelJob"><Icon name="x" :size="10" /> 中断</button>
                  <span class="chip" :class="jobChipClass">{{ jobStatusText }}</span>
                </div>
              </div>
              <div v-if="job.total > 0" class="progress mt-6"><div :style="{ width: pctJob + '%' }"></div></div>
              <div class="muted small mt-6 mono">{{ job.error || job.log || (job.status === 'succeeded' ? '任务完成' : jobRunning ? '运行中…' : '') }}</div>
            </div>

            <div class="dash"></div>
            <div class="row spread mb-6">
              <b class="t" style="margin:0">贴纸来源</b>
              <span class="muted small">LLM 可用的贴图来源</span>
            </div>
            <div class="ss-chips">
              <span v-for="s in stickerSource" :key="s" class="ss-chip">
                {{ s }}
                <button class="ss-x" title="移除" @click="removeStickerSource(s)"><Icon name="x" :size="10" /></button>
              </span>
              <span v-if="!stickerSource.length" class="muted small">未配置来源</span>
            </div>
            <div class="row mt-8">
              <input v-model="ssInput" class="ss-input flex-1" placeholder="emoji 或贴纸包名，回车添加" @keyup.enter="addStickerSource">
              <button class="mini-btn" @click="addStickerSource"><Icon name="plus" :size="11" /> 添加</button>
            </div>
            <div class="ss-available mt-8">
              <div class="small muted mb-4">可用来源（点击启用 / 停用）</div>
              <div v-for="src in stickerSources" :key="src.name" class="ss-avail-item"
                   :class="{ on: stickerSource.includes(src.name) }"
                   :title="src.tags.length ? 'tags: ' + src.tags.join(' / ') : src.description"
                   @click="toggleStickerSource(src.name)">
                <b>{{ src.name }}</b>
                <span class="muted small">{{ src.builtin ? '内置 emoji' : (src.tags.length + ' tags') }}</span>
              </div>
              <div v-if="!stickerSources.length" class="muted small">贴纸库为空（程序目录 stickers/ 下放置贴纸包）</div>
            </div>
          </div>

          <div class="card card-blk">
            <div class="row spread mb-8">
              <h2 class="m-0 card-h2"><Icon name="list" :size="15" /> 片段（{{ clips.length }}）</h2>
              <span class="muted small">点击切换</span>
            </div>
            <div class="frag-list">
              <div
                v-for="(c, i) in clips"
                :key="c.name"
                class="frag-row"
                :class="{ on: c.name === clipName }"
                @click="gotoClip(c.name)"
              >
                <span class="frag-title">{{ i + 1 }} · {{ clipRow(c).title }}</span>
                <span class="chip" :class="clipRow(c).chip">{{ clipRow(c).state }}</span>
                <button
                  v-if="!clipRow(c).initialized"
                  class="mini-btn"
                  @click.stop="initClip(c)"
                >初始化</button>
              </div>
            </div>
          </div>
        </div>

        <!-- 中部：时间轴/预览 + 九宫格（字幕编辑在时间轴块上，自动保存） -->
        <div class="layout mt-16">
          <div class="left">
            <div class="card">
              <div class="row spread">
                <h2 class="m-0 card-h2"><Icon name="video" :size="16" /> 视频预览（preview 效果）</h2>
                <span class="muted small">字幕 / 贴纸 / 运镜 / 弹幕 / 音效实时预览</span>
              </div>
              <PolishPreview ref="previewRef" :key="previewKey" :dir="dir" :config="cfg" :subtitles="subs" :danmaku="dm" :meta-duration="cfg?.duration || 0" :wgpu-render="wgpuRender" @time="onPreviewTime" />
            </div>

            <div class="card">
              <div class="row spread">
                <h2 class="m-0 card-h2"><Icon name="edit" :size="15" /> 时间轴编辑器</h2>
                <span class="muted small">拖拽移动 / 右缘缩放 / 双击删除 / 泳道 ＋ 添加</span>
              </div>
              <TimelineEditor ref="timelineRef" :dir="dir" :current-time="currentTime" @seek="seekTo" @save="onTimelineSave" @toggle="togglePreviewPlay" @subtitle-saved="reloadSubtitles" @mute-saved="onMuteSaved" />
            </div>
          </div>

          <!-- 右栏：九宫格布局；字幕编辑已并入时间轴编辑器并自动保存 -->
          <div class="right">
            <div class="card panel-card">
              <div class="row spread mb-8">
                <h2 class="m-0 card-h2"><Icon name="settings" :size="15" /> 九宫格布局</h2>
              </div>
              <div v-for="r in layoutRegions" :key="r.key" class="my-8">
                <div class="small muted mb-4">{{ REGION_LABELS[r.key] || r.key }}<span v-if="(r.value.grid || []).length > 1" class="muted">（可多选）</span></div>
                <div class="grid-3">
                  <div v-for="cell in GRID_CELLS" :key="cell" class="grid-cell"
                       :class="{ active: (r.value.grid || []).includes(cell) }"
                       @click="toggleGridCell(r.key, cell, r.value)">{{ cell }}</div>
                </div>
              </div>
              <div v-if="!layoutRegions.length" class="empty small">暂无布局配置</div>
              <div class="row mt-8">
                <button class="btn btn-primary" :disabled="jobRunning" @click="processByLayout">
                  <Icon name="sparkles" :size="13" /> 按当前布局处理
                </button>
              </div>
              <div class="muted small mt-8">保存布局后点击处理，会按当前九宫格重新计算贴纸等位置</div>
            </div>
          </div>
        </div>

        <div class="mt-16"><CoverDesigner :dir="dir" @changed="refreshClipEntry" /></div>

        <!-- 产物整行：成片 + 封面 + 输出目录 -->
        <div class="card mt-16 artifact-bar">
          <div class="row spread">
            <div class="row" style="gap:10px">
              <b class="t" style="margin:0">产物</b>
              <a v-if="renderedUrl && !renderedFailed" class="mini-btn" :href="renderedUrl" download>
                <Icon name="download" :size="12" /> 成片 {{ renderedName }}
              </a>
              <span v-else class="muted small">未渲染</span>
              <a v-if="coverUrl && !coverFailed" class="mini-btn" :href="coverUrl" download>
                <Icon name="download" :size="12" /> 封面 {{ coverName }}
              </a>
              <span v-else class="muted small">无封面</span>
            </div>
            <button class="mini-btn" @click="copyOutDir">打开输出目录（复制路径）</button>
          </div>
          <div class="row mt-8" style="gap:12px">
            <img v-if="coverUrl && !coverFailed" :src="coverUrl" class="cover-thumb" alt="封面" @error="coverFailed = true">
            <video v-if="renderedUrl && !renderedFailed" :src="renderedUrl" controls playsinline class="video-bar" @error="renderedFailed = true"></video>
            <div v-if="!(coverUrl && !coverFailed) && !(renderedUrl && !renderedFailed)" class="muted small">渲染导出或生成封面后在这里查看</div>
          </div>
        </div>
      </template>
    </template>

    <!-- 渲染导出弹窗 -->
    <AppModal :show="renderModal" title="渲染导出" @close="renderModal = false">
      <label>输出文件路径（留空使用 polish.toml 的 project.output）</label>
      <input v-model="renderOutput" :placeholder="dir ? dir + '/export.mp4' : ''">
      <template #footer>
        <button class="btn btn-primary" :disabled="jobRunning" @click="doRender">开始渲染</button>
        <button class="btn" @click="renderModal = false">取消</button>
      </template>
    </AppModal>
  </div>
</template>

<script setup>
import { ref, computed, reactive, onMounted, onUnmounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import Icon from '../components/Icon.vue'
import AppModal from '../components/AppModal.vue'
import PolishPreview from '../components/PolishPreview.vue'
import TimelineEditor from '../components/TimelineEditor.vue'
import CoverDesigner from '../components/CoverDesigner.vue'
import { mediaUrl } from '../utils'
import { api } from '../api/client'
import {
  fetchProject, fetchSubtitles, saveSubtitles, saveProjectPatch,
  initPolish, runLlm, runProcess, runRender, runSilence,
  fetchStickerSources,
  GRID_CELLS, REGION_LABELS, clipRelDir,
} from '../api/polish'
import { watchJob } from '../api/jobs'

const route = useRoute()
const router = useRouter()
const routePid = route.params.pid || ''
const pid = ref(routePid)
const routeCfg = 'project=' + encodeURIComponent(routePid)
const dir = ref('')
const clipName = computed(() => route.params.clip || '')
const title = ref('精修编辑器')
const loading = ref(false)
const ready = ref(false)
const cfg = ref(null)
const subs = ref([])
const dm = ref([])
const clips = ref([])
const clipEntry = ref(null)
/** 片段目录（规范化）→ `/api/project-data/polish` 行，供片段列表显示状态 */
const polishByDir = ref({})
const currentTime = ref(0)
const previewKey = ref(0)
const renderModal = ref(false)
const renderOutput = ref('')
const coverName = ref('cover.jpg')
const renderedName = ref('export.mp4')
const timelineRef = ref(null)
const previewRef = ref(null)
const savedSegs = ref('')      // 上次保存的时间轴 JSON 快照（脏状态比对）
const savedSubsJson = ref('')  // 上次保存的字幕 JSON 快照

const job = reactive({ id: '', kind: '', status: '', current: 0, total: 0, log: '', error: '' })
let jobClose = null
let jobTimer = null
let canceling = ref(false)
const jobStartMs = ref(0)
const jobElapsed = ref(0)
// 服务端是否带 --features wgpu-render 构建（决定 /api/polish/render-frame 是否可用，缺失时 501）
const wgpuRender = ref(false)

const jobRunning = computed(() => job.status === 'running' || job.status === 'queued')
const pctJob = computed(() => job.total > 0 ? Math.round(job.current * 100 / job.total) : 0)
const jobActive = computed(() => jobRunning.value)
const jobFinished = computed(() => ['succeeded', 'failed', 'cancelled'].includes(job.status))
const jobStatusText = computed(() => ({
  queued: '排队中', running: '运行中',
  succeeded: '完成', failed: '失败', cancelled: '已取消'
}[job.status] || job.status))
const jobChipClass = computed(() => {
  if (job.status === 'failed' || job.status === 'cancelled') return 'r'
  if (job.status === 'succeeded') return 'g'
  return jobRunning.value ? 'b' : 'n'
})
const jobTimeText = computed(() => {
  const sec = Math.max(0, Math.floor(jobElapsed.value))
  if (sec < 60) return sec + 's'
  const m = Math.floor(sec / 60)
  const s = sec % 60
  return m + 'm' + (s ? s + 's' : '')
})
// 贴纸配置 = polish.toml 顶层 sticker_source（LLM 生成时可选的贴图来源池）
const stickerSource = computed(() => cfg.value?.sticker_source || [])
const stickerSources = ref([]) // 全局可用来源（emoji + 贴纸库包）
const ssInput = ref('')
const layoutRegions = computed(() => {
  const src = cfg.value?.layout || {}
  // Object.entries 返回 [key, value] 数组，映射为对象避免 v-for 把索引当 key
  return Object.entries(src)
    .filter(([, v]) => v && typeof v === 'object')
    .map(([key, value]) => ({ key, value }))
})
// 产物优先使用后端返回的实际文件名（含 _1/_2 版本），未返回时回退固定名
const coverFailed = ref(false)
const renderedFailed = ref(false)
const coverUrl = computed(() => {
  if (clipEntry.value?.cover_file) return mediaUrl(dir.value, clipEntry.value.cover_file)
  return dir.value ? mediaUrl(dir.value, 'cover.jpg') : ''
})
const renderedUrl = computed(() => {
  if (clipEntry.value?.rendered_file) return mediaUrl(dir.value, clipEntry.value.rendered_file)
  return dir.value ? mediaUrl(dir.value, 'vtuber-clip.mp4') : ''
})

// 脏状态：时间轴分段或字幕与上次保存不一致时，保存编辑按钮换色提醒
const editDirty = computed(() => {
  if (!ready.value || !cfg.value) return false
  const segs = timelineRef.value?.segments
  if (segs && JSON.stringify(segs) !== savedSegs.value) return true
  return JSON.stringify(subs.value) !== savedSubsJson.value
})

function jobLabel(kind) {
  return { llm: 'LLM 生成', render: '渲染导出', process: '后处理', init: '初始化' }[kind] || kind
}

function onPreviewTime(t) {
  currentTime.value = Number(t) || 0
}
function seekTo(t) {
  // 字幕行点击传的是字幕对象；时间轴/进度条传的是数字秒
  const raw = (t && typeof t === 'object') ? (t.start ?? t.end) : t
  const n = Number(raw)
  currentTime.value = Number.isFinite(n) ? Math.round(n * 100) / 100 : 0
  previewRef.value?.seekTo(currentTime.value)
}
function togglePreviewPlay() {
  previewRef.value?.togglePlay()
}

function pickClip(c) {
  router.replace({ query: { ...route.query, dir: c.dir } })
}
function clearDir() {
  router.replace({ query: { ...route.query, dir: undefined } })
}

async function load() {
  // 入口语义（v5 §2.6）：无 :clip → 默认定位「精修中」片段（否则第一个）；
  // 无任何切片 → 空态引导；深链无效 → 回退默认片段 + toast。
  await loadClips()
  if (!clips.value.length) {
    dir.value = ''
    ready.value = false
    cfg.value = null
    return
  }
  const wanted = clipName.value || defaultClipName()
  const entry = clips.value.find((c) => c.name === wanted)
  if (!entry) {
    if (clipName.value) toast(`片段 ${clipName.value} 不存在，已回退到 ${defaultClipName()}`, 'err')
    dir.value = clipDirOf(defaultClipName())
  } else {
    dir.value = entry.dir || clipDirOf(wanted)
  }
  if (clipName.value !== wanted) {
    router.replace({ name: 'project-polish-clip', params: { pid: pid.value, clip: wanted } })
  }
  loading.value = true
  try {
    cfg.value = await fetchProject(pid.value, dir.value, true)
    title.value = cfg.value?.project?.name || cfg.value?.source?.video || '精修编辑器'
    ready.value = true
  } catch {
    ready.value = false
  }
  try {
    subs.value = (await fetchSubtitles(pid.value, dir.value)).segments || []
  } catch { subs.value = [] }
  subs.value.sort((a, b) => (Number(a.start) || 0) - (Number(b.start) || 0))
  try {
    const r = await fetch(mediaUrl(dir.value, 'data/danmaku.json'))
    if (r.ok) dm.value = await r.json()
  } catch { dm.value = [] }
  refreshSnapshots()
  await refreshClipEntry()
  loading.value = false
}

function refreshSnapshots() {
  savedSegs.value = JSON.stringify(cfg.value?.segments || [])
  savedSubsJson.value = JSON.stringify(subs.value)
}

function thumbSrc(c) {
  const rank = String(c.name || '').replace('clip-', '')
  return `/api/project-data/clips/${encodeURIComponent(rank)}/thumb?${routeCfg}`
}

function clipDirOf(name) {
  const hit = clips.value.find((c) => c.name === name)
  return hit?.dir || name
}

/** 默认片段：已 process 未渲染（"精修中"）优先，其次第一个未渲染，最后第一个。 */
function defaultClipName() {
  const list = clips.value
  const polishedOf = (name) => {
    const dir = (list.find((c) => c.name === name)?.dir || '').replace(/\\/g, '/').toLowerCase()
    return polishByDir.value[dir] || null
  }
  const mid = list.find((c) => {
    const p = polishedOf(c.name)
    return p && p.processed && !p.rendered
  })
  if (mid) return mid.name
  const unrendered = list.find((c) => {
    const p = polishedOf(c.name)
    return !p || !p.rendered
  })
  return (unrendered || list[0]).name
}

async function loadClips() {
  loading.value = true
  try {
    if (!routeCfg) { clips.value = []; return }
    const [list, polish] = await Promise.all([
      api(`/api/project-data/clips?${routeCfg}`).catch(() => []),
      api(`/api/project-data/polish?${routeCfg}`).catch(() => [])
    ])
    clips.value = (Array.isArray(list) ? list : []).filter((c) => c.video_exists)
    const m = {}
    ;(polish || []).forEach((p) => { m[String(p.dir || '').replace(/\\/g, '/').toLowerCase()] = p })
    polishByDir.value = m
  } catch {
    clips.value = []
  } finally {
    loading.value = false
  }
}

/** 片段列表行：`N · 标题` + 状态 chip（已精修 / 精修中 / 未初始化）。 */
function clipRow(c) {
  const dir = String(c.dir || '').replace(/\\/g, '/').toLowerCase()
  const p = polishByDir.value[dir] || null
  const initialized = !!p?.polish_ready
  const state = !initialized ? '未初始化' : (p.rendered ? '已精修' : '精修中')
  return {
    name: c.name,
    title: c.meta?.title || c.name,
    state,
    chip: state === '已精修' ? 'g' : state === '精修中' ? 'p' : 'n',
    initialized
  }
}

async function initClip(c) {
  startJob('init', () => initPolish(pid.value, clipRelDir(c.name)), { after: loadClips })
  toast('正在初始化 ' + c.name)
}

function gotoClip(name) {
  router.push({ name: 'project-polish-clip', params: { pid: pid.value, clip: name } })
}

async function reloadSubtitles() {
  try {
    subs.value = (await fetchSubtitles(pid.value, dir.value)).segments || []
    subs.value.sort((a, b) => (Number(a.start) || 0) - (Number(b.start) || 0))
    refreshSnapshots()
  } catch {}
}

async function refreshClipEntry() {
  coverFailed.value = false
  renderedFailed.value = false
  try {
    const list = routeCfg ? await api(`/api/project-data/polish?${routeCfg}`) : []
    const norm = (p) => String(p || '').replace(/\\/g, '/').toLowerCase()
    clipEntry.value = (list || []).find((p) => norm(p.dir) === norm(dir.value)) || null
  } catch { clipEntry.value = null }
}

// ---- 任务驱动 ----

function startJobTimer() {
  stopJobTimer()
  jobStartMs.value = Date.now()
  jobElapsed.value = 0
  jobTimer = setInterval(() => {
    if (!jobRunning.value) return
    jobElapsed.value = Math.floor((Date.now() - jobStartMs.value) / 1000)
  }, 1000)
}

function stopJobTimer() {
  if (jobTimer) {
    clearInterval(jobTimer)
    jobTimer = null
  }
  if (jobStartMs.value) {
    jobElapsed.value = Math.max(jobElapsed.value, Math.floor((Date.now() - jobStartMs.value) / 1000))
  }
}

async function cancelJob() {
  if (!job.id || !jobRunning.value) return
  canceling.value = true
  try {
    await api('/api/jobs/' + job.id + '/cancel', { method: 'POST' })
    // cancel 接口已同步把任务置为 cancelled；关掉订阅避免后台迟到的完成事件再把它改回成功
    if (jobClose) {
      jobClose()
      jobClose = null
    }
    job.status = 'cancelled'
    stopJobTimer()
    toast('任务已中断')
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    canceling.value = false
  }
}

function startJob(kind, call, { after } = {}) {
  if (jobRunning.value) { toast('已有任务在运行，请稍候', 'err'); return Promise.resolve(false) }
  return call().then((r) => {
    Object.assign(job, { id: r.id, kind, status: 'running', current: 0, total: 0, log: '', error: '' })
    startJobTimer()
    jobClose?.()
    jobClose = watchJob(r.id, {
      onEvent(e) {
        if (e.type === 'log') job.log = e.message
        else if (e.type === 'progress') { job.current = e.current; job.total = e.total }
        else if (e.type === 'failed') job.error = e.error
      },
      onSummary(s) {
        job.status = s.status
        job.current = s.current
        job.total = s.total
      },
      onDone(s) {
        job.status = s.status
        stopJobTimer()
        if (s.status === 'succeeded') {
          toast(jobLabel(kind) + '完成')
          previewKey.value++
          after && after()
        } else if (s.status === 'failed') {
          toast(jobLabel(kind) + '失败：' + (job.error || s.message || '未知错误'), 'err')
          after && after()
        } else if (s.status === 'cancelled') {
          toast(jobLabel(kind) + '已取消')
          after && after()
        }
      }
    })
    return true
  }).catch((e) => { toast(e.message, 'err'); return false })
}

async function initProject() {
  const ok = await confirmDialog('将解析字幕/弹幕并创建 polish.toml', { title: '初始化精修项目？' })
  if (!ok) return
  startJob('init', () => initPolish(pid.value, dir.value), { after: load })
}

function runLlmJob() {
  startJob('llm', () => runLlm(pid.value, dir.value), { after: load })
}

/** 去静音：把长停顿写入屏蔽轨道（不重编码、不改原始视频），随后自动 process。 */
function doSilence() {
  startJob('silence', () => runSilence(pid.value, dir.value), { after: load })
}

/** 打开输出目录：浏览器无法打开本地目录，复制路径并提示。 */
async function copyOutDir() {
  const p = dir.value || ''
  if (!p) return
  try {
    await navigator.clipboard.writeText(p)
    toast('输出目录路径已复制')
  } catch {
    toast(p)
  }
}

function doRender() {
  renderModal.value = false
  startJob('render', () => runRender(pid.value, dir.value, clipRelDir(clipName.value)), { after: load })
}

// 保存编辑：时间轴分段 / 字幕，各自仅在变动时提交
async function saveEdits() {
  if (!editDirty.value) { toast('没有需要保存的修改'); return }
  let segsChanged = false
  const segs = timelineRef.value?.segments
  if (segs && JSON.stringify(segs) !== savedSegs.value) {
    try {
      await saveProjectPatch(pid.value, dir.value, { segments: segs })
      savedSegs.value = JSON.stringify(segs)
      segsChanged = true
    } catch (e) { toast('保存时间轴失败：' + e.message, 'err'); return }
  }
  if (JSON.stringify(subs.value) !== savedSubsJson.value) {
    try {
      await saveSubtitles(pid.value, dir.value, subs.value)
      savedSubsJson.value = JSON.stringify(subs.value)
      previewKey.value++
    } catch (e) { toast('保存字幕失败：' + e.message, 'err'); return }
  }
  toast('编辑已保存')
  if (segsChanged) {
    // 时间轴变更后隐式 process（完成后刷新预览）
    startJob('process', () => runProcess(pid.value, dir.value), { after: load })
  }
}

function onTimelineSave(changes) {
  const segs = Array.isArray(changes) ? changes : (changes?.segments || [])
  const mutes = Array.isArray(changes) ? null : changes?.mute_ranges
  const patch = { segments: segs }
  if (mutes) patch.mute_ranges = mutes
  saveProjectPatch(pid.value, dir.value, patch)
    .then(() => {
      toast('时间轴已保存，自动执行后处理…')
      return startJob('process', () => runProcess(pid.value, dir.value), { after: load })
    })
    .catch((e) => toast(e.message, 'err'))
}

async function onMuteSaved(ranges) {
  try {
    await saveProjectPatch(pid.value, dir.value, { mute_ranges: ranges })
    if (cfg.value) cfg.value.mute_ranges = ranges
    previewKey.value++
    // 「去静音」会直接改写 polish.toml，这里同步刷新屏蔽段数状态
    await refreshClipEntry()
  } catch (e) {
    toast('屏蔽区间保存失败：' + e.message, 'err')
  }
}

// 九宫格多选：点击切换格子是否在区域 grid 内
function processByLayout() {
  startJob('process', () => runProcess(pid.value, dir.value), { after: load })
}

async function toggleGridCell(key, cell, region) {
  try {
    const grid = [...(region?.grid || [])]
    const i = grid.indexOf(cell)
    if (i >= 0) grid.splice(i, 1)
    else grid.push(cell)
    if (!grid.length) { toast('至少保留一个格子', 'err'); return }
    const lay = { ...(cfg.value?.layout || {}) }
    lay[key] = { ...(region || {}), grid }
    await saveProjectPatch(pid.value, dir.value, { layout: lay })
    cfg.value = { ...cfg.value, layout: lay }
  } catch (e) { toast(e.message, 'err') }
}

async function saveSubs() {
  try {
    await saveSubtitles(pid.value, dir.value, subs.value)
    savedSubsJson.value = JSON.stringify(subs.value)
    toast('字幕已保存')
    previewKey.value++
  } catch (e) { toast(e.message, 'err') }
}

// 添加字幕：开始 = 当前播放时间，结束 = +3s
// ---- 贴纸配置（sticker_source，直存 polish.toml）----
async function loadStickerSources() {
  try {
    stickerSources.value = (await fetchStickerSources(pid.value, dir.value)).sources || []
  } catch { stickerSources.value = [] }
}

// /api/health 的 wgpu_render：非 true（含 undefined）都视为未启用
async function loadHealth() {
  try {
    const h = await api('/api/health')
    wgpuRender.value = h?.wgpu_render === true
  } catch { wgpuRender.value = false }
}

async function patchStickerSource(list) {
  try {
    await saveProjectPatch(pid.value, dir.value, { sticker_source: list })
    cfg.value = { ...cfg.value, sticker_source: list }
    toast('贴纸配置已保存')
  } catch (e) { toast(e.message, 'err') }
}

function removeStickerSource(name) {
  patchStickerSource(stickerSource.value.filter((x) => x !== name))
}

function addStickerSource() {
  const name = ssInput.value.trim()
  if (!name) return
  if (stickerSource.value.includes(name)) { toast('该来源已启用'); return }
  ssInput.value = ''
  patchStickerSource([...stickerSource.value, name])
}

function toggleStickerSource(name) {
  if (stickerSource.value.includes(name)) patchStickerSource(stickerSource.value.filter((x) => x !== name))
  else patchStickerSource([...stickerSource.value, name])
}

function onCoverError() {
  coverFailed.value = true
}
function onRenderedError() {
  renderedFailed.value = true
}

onMounted(() => {
  load()
  loadStickerSources()
  loadHealth()
})
onUnmounted(() => {
  stopJobTimer()
  jobClose?.()
})
watch(dir, load)
</script>

<style scoped>
.top-cards { display:grid; grid-template-columns:minmax(0,1fr) 340px; gap:16px; align-items:start; }
.action-grid { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr); gap:18px; }
.top-cards .btn { padding:5px 13px; font-size:12px; }
.save-btn { margin-left:auto; }
.btn.dirty { background:var(--amber-soft); color:var(--amber-deep); border-color:var(--amber); box-shadow:none; }
.btn.dirty:hover { background:var(--amber); border-color:var(--amber); color:#fff; }
.ss-chips { display:flex; flex-wrap:wrap; gap:6px; }
.ss-chip { display:inline-flex; align-items:center; gap:5px; padding:3px 6px 3px 10px; border-radius:var(--r-full); background:var(--main-soft); color:var(--main-deep); font-size:12px; font-weight:600; }
.ss-x { border:none; background:none; color:inherit; cursor:pointer; display:flex; padding:2px; border-radius:50%; }
.ss-x:hover { background:var(--red-soft); color:var(--red-deep); }
.ss-input { width:auto; }
.ss-available { border-top:1px dashed var(--line-soft); padding-top:8px; max-height:150px; overflow:auto; }
.ss-avail-item { display:flex; align-items:center; justify-content:space-between; gap:8px; padding:4px 8px; border-radius:8px; cursor:pointer; font-size:12px; }
.ss-avail-item:hover { background:var(--bg-soft); }
.ss-avail-item.on { color:var(--ink-faint); text-decoration:line-through; }
.layout { display:grid; grid-template-columns:minmax(0,1fr) 340px; gap:16px; align-items:stretch; }
.left { display:flex; flex-direction:column; gap:16px; min-width:0; }
.right { display:flex; flex-direction:column; gap:12px; }
.card-h2 { display:flex; align-items:center; gap:7px; }
.seg-toggle .mini-btn.selected { background:var(--main-soft); color:var(--main-deep); border-color:var(--main); }
.add-sub { margin-left:auto; }
.panel-card { display:flex; flex-direction:column; flex:1; }
.panel-body { flex:1; min-height:0; display:flex; flex-direction:column; }
.panel-body :deep(.sub-list) { flex:1; }
.grid-cell { border:1px solid var(--line); border-radius:6px; padding:6px; text-align:center; font-size:10px; cursor:pointer; }
.grid-cell:hover { border-color:var(--main); }
.grid-cell.active { background:var(--main-soft); border-color:var(--main); color:var(--main-deep); }
.job-box { border:1px solid var(--line); border-radius:var(--r-sm); padding:8px 10px; background:var(--bg-soft); }
.frag-list { display:flex; flex-direction:column; gap:2px; max-height:320px; overflow:auto; }
.frag-row { display:flex; align-items:center; gap:8px; padding:7px 8px; border-radius:9px; cursor:pointer; }
.frag-row:hover { background:var(--bg-soft); }
.frag-row.on { background:var(--main-soft); }
.frag-row.on .frag-title { color:var(--pink-text); font-weight:600; }
.frag-title { flex:1; min-width:0; font-size:12px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.artifact-bar .cover-thumb { width:150px; border-radius:var(--r-sm); border:1px solid var(--line); }
.artifact-bar .video-bar { max-height:32vh; border-radius:var(--r-sm); }
.product-grid { display:grid; grid-template-columns:1fr 1fr; gap:16px; align-items:start; }
.cover-img { width:100%; border-radius:var(--r-sm); border:1px solid var(--line); display:block; }
.product-video { max-height:46vh; }
.thumb { width:100%; aspect-ratio:16/9; object-fit:cover; border-radius:var(--r-sm); margin-top:10px; border:1px solid var(--line); }
@media (max-width: 1100px) {
  .top-cards, .layout, .action-grid { grid-template-columns:1fr; }
}
</style>
