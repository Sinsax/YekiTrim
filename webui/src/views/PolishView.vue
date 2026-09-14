<template>
  <div>
    <div class="page-head">
      <div>
        <span class="chip p">精修</span>
        <h1 class="mt-2">{{ dir ? title : '选择要精修的片段' }}</h1>
        <!-- 有片段时不再占一行 sub（片段条 + 面包屑已说明是哪个片段），把竖高留给预览 -->
        <div v-if="!dir" class="sub">从片段库选择一个已剪切片段进入精修编辑器</div>
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

      <!-- 列表模式：没有上次操作记录 → 只显示片段列表 + 空态，视频/编辑器不加载 -->
      <div v-else-if="!clipName" class="card card-blk">
        <div class="row spread mb-8">
          <h2 class="m-0 card-h2"><Icon name="list" :size="15" /> 片段（{{ clips.length }}）</h2>
          <span class="muted small">点击一个片段进入精修编辑器</span>
        </div>
        <FragmentList :rows="fragRows" :current="clipName" @goto="gotoClip" @init="initClipByName" />
        <div class="empty small mt-12">未选择片段：编辑器与视频会在选中片段后加载。</div>
      </div>

      <!-- 未初始化：初始化卡 + 片段列表（否则无法切换/初始化其他片段） -->
      <div v-else-if="!ready" class="grid cols-main">
        <div class="card">
          <h2>未初始化精修项目</h2>
          <p class="muted small">将解析字幕/弹幕并创建 polish.toml。</p>
          <div class="row mt-8">
            <button class="btn btn-primary" :disabled="jobRunning" @click="initProject">
              <Icon name="sparkles" :size="14" /> 初始化
            </button>
          </div>
        </div>
        <div class="card card-blk">
          <div class="row spread mb-8">
            <h2 class="m-0 card-h2"><Icon name="list" :size="15" /> 片段（{{ clips.length }}）</h2>
            <span class="muted small">点击切换，或在行内初始化</span>
          </div>
          <FragmentList :rows="fragRows" :current="clipName" @goto="gotoClip" @init="initClipByName" />
        </div>
      </div>

      <template v-else>
        <!-- 片段条（一行 chip）：把它压到最扁才腾得出「预览 + 时间轴」同屏的竖高 -->
        <div class="card frag-strip">
          <FragmentList chips :rows="fragRows" :current="clipName" @goto="gotoClip" @init="initClipByName" />
        </div>

        <!-- 左列 = 预览 + 时间轴（上下相邻，一起占满首屏）；右栏 = 生成与导出 -->
        <div class="grid cols-main mt-16 edit-grid">
          <div class="left-col">
            <div class="card">
              <div class="row spread">
                <h2 class="m-0 card-h2"><Icon name="video" :size="16" /> 视频预览（preview 效果）</h2>
                <span v-if="selectRegion" class="chip p">选框模式 · 运镜已暂停</span>
                <span v-else class="muted small">字幕 / 贴纸 / 运镜 / 弹幕 / 音效实时预览</span>
              </div>
              <PolishPreview ref="previewRef" :key="previewKey" :dir="dir" :config="cfg" :subtitles="subs" :danmaku="dm" :meta-duration="cfg?.duration || 0" :wgpu-render="wgpuRender" :select-mode="!!selectRegion" :select-rect="selRect" :select-label="selLabel" @time="onPreviewTime" @update:select-rect="onSelectRect" />
              <div v-if="selectRegion" class="hint mt-8">在预览里拖框体平移、拖 8 个角/边缩放、空白拖拽重画；右侧「区域编辑」里保存或清除。</div>
            </div>

            <!-- 时间轴（标题/提示由 slot 交给组件，省掉独立卡片头；去静音只在时间轴上，右侧不再重复） -->
            <div class="card tl-card">
              <TimelineEditor ref="timelineRef" :key="'tl' + segKey" :dir="dir" :current-time="currentTime" @seek="seekTo" @save="onTimelineSave" @toggle="togglePreviewPlay" @subtitle-saved="reloadSubtitles" @mute-saved="onMuteSaved">
                <template #head>
                  <b>时间轴编辑器</b>
                  <span class="muted small">拖拽移动 / 右缘缩放 / 双击删除 / 泳道 ＋ 添加</span>
                </template>
              </TimelineEditor>
            </div>
          </div>

          <div class="card">
            <div class="row spread mb-8">
              <h2 class="m-0 card-h2"><Icon name="sparkles" :size="15" /> 生成与导出</h2>
              <span class="muted small">LLM 完成后自动执行后处理</span>
            </div>

            <!-- LLM 剪辑（参数：区域编辑 / 贴纸来源）—— 整块在「渲染导出」上方 -->
            <div class="gen-block">
              <div class="row spread mb-6">
                <b class="t" style="margin:0"><Icon name="sparkles" :size="12" /> LLM 剪辑</b>
                <span class="muted small">下列为参数</span>
              </div>

              <!-- 参数 1：区域编辑（人物 / 弹幕区域） -->
              <div class="small muted mb-4">区域编辑<span class="hint"> · 运镜聚焦</span></div>
              <div class="row mb-6" style="gap:6px">
                <button class="mini-btn" style="flex:1;justify-content:center" :class="{ selected: selectRegion === 'liver' }" :disabled="!ready"
                        title="框选主播人物：运镜聚焦到框中心" @click="toggleSelect('liver')">
                  <Icon name="crop" :size="11" /> 人物
                </button>
                <button class="mini-btn" style="flex:1;justify-content:center" :class="{ selected: selectRegion === 'danmaku_area' }" :disabled="!ready"
                        title="框选弹幕显示区域（供运镜参考）" @click="toggleSelect('danmaku_area')">
                  <Icon name="crop" :size="11" /> 弹幕区域
                </button>
              </div>
              <template v-if="selectRegion">
                <div class="row spread small mb-4">
                  <span class="mono">{{ rectText }}</span>
                  <span v-if="hasFreeRect" class="chip p">自由选框</span>
                  <span v-else class="chip n">九宫格默认</span>
                </div>
                <div class="row mb-6" style="gap:6px">
                  <button class="mini-btn primary" :disabled="jobRunning" @click="applySelect"><Icon name="check" :size="11" /> 保存选框</button>
                  <button class="mini-btn" :disabled="jobRunning || !hasFreeRect" @click="clearSelect">清除选框</button>
                </div>
                <div class="hint mb-6">{{ regionHint }}</div>
              </template>
              <div v-else class="hint mb-8">点「人物」或「弹幕区域」后到左侧预览里框选。</div>

              <!-- 参数 2：贴纸来源 -->
              <div class="small muted mb-4">贴纸来源<span class="hint"> · LLM 可用的贴图来源</span></div>
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

              <!-- 动作（去静音只在时间轴头部，那里还带「撤销」，此处不再重复） -->
              <button class="btn mt-12" style="width:100%;justify-content:center" :disabled="jobRunning" @click="runLlmJob"><Icon name="sparkles" :size="13" /> LLM 剪辑</button>
            </div>

            <!-- 导出（在 LLM 参数块下方） -->
            <button class="btn btn-primary mt-12" style="width:100%;justify-content:center" :disabled="jobRunning" @click="renderModal = true"><Icon name="film" :size="13" /> 渲染导出</button>

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
          </div>
        </div>
        <!-- 时间轴已在左列（预览正下方），保证最大化窗口下两者同屏 -->

        <div class="mt-16"><CoverDesigner :key="'cv' + coverKey" :dir="dir" @changed="refreshClipEntry" /></div>

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
      <label>输出文件（**项目内相对路径**，留空使用 polish.toml 的 project.output）</label>
      <input v-model="renderOutput" placeholder="output/clips/clip-01/export.mp4">
      <p class="hint mt-6">只接受项目内相对路径；绝对路径会被服务端拒绝（P2 起不再允许任意输出位置）。</p>
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
import FragmentList from '../components/polish/FragmentList.vue'
import CoverDesigner from '../components/CoverDesigner.vue'
import { mediaUrl } from '../utils'
import { api } from '../api/client'
import { scopeQ } from '../api/scope'
import {
  fetchProject, fetchSubtitles, saveSubtitles, saveProjectPatch,
  initPolish, runLlm, runProcess, runRender,
  fetchStickerSources,
  REGION_LABELS,
} from '../api/polish'
import { freeRect, regionRect } from '../polish/engine'
import { watchJob } from '../api/jobs'

const route = useRoute()
const router = useRouter()
const routePid = route.params.pid || ''
const pid = ref(routePid)
const routeCfg = scopeQ(pid.value)
const dir = ref('')
const clipName = computed(() => route.params.clip || '')
// 记住最后操作过的片段：下次从侧栏进精修页自动回到它（每项目一个键）
const LAST_CLIP_KEY = computed(() => 'vtrim.polish.last.' + pid.value)
watch(clipName, (name) => {
  if (name) { try { localStorage.setItem(LAST_CLIP_KEY.value, name) } catch {} }
  // 片段切换只改路由，必须在这里触发加载（旧代码漏了：点片段行编辑器不切换）
  load()
})
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
/** 时间轴重挂载计数（仅当磁盘上的 segments/屏蔽轨道被服务端改写：LLM 剪辑 / 初始化 / 去静音）。
 *  静默刷新不再卸载编辑器，所以需要重挂载的组件自己带 key。 */
const segKey = ref(0)
/** 封面设计器重挂载计数（渲染导出会重新生成封面文件） */
const coverKey = ref(0)
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

// ---- 布局框选（取代九宫格）：只有「人物」「弹幕区域」两个区域 ----
// 贴纸落点由 process 按 layout.sticker_area 计算，不在 UI 里框选（要改就手改 polish.toml）。
const FRAMEABLE = ['liver', 'danmaku_area']
/** 正在框选的区域；'' = 不在框选模式（一个 ref 同时表达"选谁"和"是否在框选"） */
const selectRegion = ref('')
/** 编辑中的选框（整帧百分比），仅在框选模式下随拖拽更新 */
const selRect = ref({ x0: 0, y0: 0, x1: 100, y1: 100 })
const curRegion = computed(() => (cfg.value?.layout || {})[selectRegion.value] || null)
/** 该区域是否已有自由选框（否则回退到九宫格 grid） */
const hasFreeRect = computed(() => !!freeRect(curRegion.value))
const selLabel = computed(() => REGION_LABELS[selectRegion.value] || selectRegion.value)
const REGION_HINTS = {
  liver: '框住主播画面：运镜聚焦到框中心（比九宫格更准）。',
  danmaku_area: '框住直播间弹幕显示区域（供运镜参考）。'
}
const regionHint = computed(() => REGION_HINTS[selectRegion.value] || '')
/** 展示用矩形：框选模式看编辑值，否则看已保存的选框 / 九宫格包围盒 */
const shownRect = computed(() => {
  if (selectRegion.value) return selRect.value
  return regionRect(curRegion.value) || { x0: 0, y0: 0, x1: 100, y1: 100 }
})
const rectText = computed(() => {
  const r = shownRect.value
  const f = (v) => (Math.round((Number(v) || 0) * 10) / 10) + '%'
  return `${f(r.x0)}, ${f(r.y0)} → ${f(r.x1)}, ${f(r.y1)}`
})

/** 当前区域的选框作为编辑初值（已是自由选框则用现值，否则取九宫格包围盒）。 */
function loadSelRect() {
  selRect.value = regionRect(curRegion.value) || { x0: 0, y0: 0, x1: 100, y1: 100 }
}

function onSelectRect(r) {
  selRect.value = r
}

/** 点区域按钮：同一个再点一次 = 退出框选；点另一个 = 切换区域。 */
function toggleSelect(region) {
  if (selectRegion.value === region) { selectRegion.value = ''; return }
  selectRegion.value = FRAMEABLE.includes(region) ? region : ''
  if (!selectRegion.value) return
  loadSelRect()
  // 定格当前帧，避免边播边框选
  previewRef.value?.pause?.()
}

const pctStr = (v) => (Math.round(Math.min(100, Math.max(0, Number(v) || 0)) * 100) / 100).toFixed(2) + '%'

/** 保存选框：写入 layout[区域].top_left/bottom_right（读取时优先于 grid）。
 *  只落盘、不起任务：这两处布局在渲染时直接读取，不需要重跑 process。 */
async function applySelect() {
  try {
    const lay = { ...(cfg.value?.layout || {}) }
    const region = { ...(lay[selectRegion.value] || {}) }
    const r = selRect.value
    region.top_left = { x: pctStr(Math.min(r.x0, r.x1)), y: pctStr(Math.min(r.y0, r.y1)) }
    region.bottom_right = { x: pctStr(Math.max(r.x0, r.x1)), y: pctStr(Math.max(r.y0, r.y1)) }
    lay[selectRegion.value] = region
    await saveProjectPatch(pid.value, dir.value, { layout: lay })
    cfg.value = { ...cfg.value, layout: lay }
    selectRegion.value = ''
    toast('选框已保存，渲染导出时生效')
  } catch (e) {
    toast(e.message, 'err')
  }
}

/** 清除选框：回到该区域的九宫格 grid（不动 grid 本身，可逆）。 */
async function clearSelect() {
  try {
    const lay = { ...(cfg.value?.layout || {}) }
    const region = { ...(lay[selectRegion.value] || {}) }
    delete region.top_left
    delete region.bottom_right
    lay[selectRegion.value] = region
    await saveProjectPatch(pid.value, dir.value, { layout: lay })
    cfg.value = { ...cfg.value, layout: lay }
    loadSelRect()
    toast('已清除选框，回到九宫格区域')
  } catch (e) {
    toast(e.message, 'err')
  }
}

// 重新加载配置后同步选框（框选模式中不打断拖拽）
watch(() => cfg.value?.layout, () => { if (!selectRegion.value) loadSelRect() })
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

/**
 * 加载当前片段。
 *
 * `silent`（任务完成后的刷新）= 不闪骨架屏、不卸载编辑器、不丢滚动位置，失败时保留旧数据；
 * 首次进入 / 切换片段仍走显式 loading（那本来就该换内容）。
 * 非 silent 时 `loading` 会把整个编辑器换成骨架屏（`v-if="loading"`），
 * 编辑器一旦被卸载，预览视频与滚动位置都会重置——看起来就像浏览器刷新了。
 */
async function load(opts = {}) {
  const silent = !!(opts && opts.silent)
  // loading 只在这里拨一次（loadClips 不再自己开关），避免中间出现 editor↔骨架屏 的抖动
  if (!silent) loading.value = true
  try {
    await loadClips()
    if (!clips.value.length) {
      dir.value = ''
      ready.value = false
      cfg.value = null
      return
    }
    // 无 :clip → 自动打开**上次操作过的**片段；没有记录（或已失效）→
    // 片段列表 + 空态（视频/编辑器都不加载），不再随手挑一个默认片段。
    let wanted = clipName.value
    if (!wanted) {
      let last = null
      try { last = localStorage.getItem(LAST_CLIP_KEY.value) } catch {}
      if (last && clips.value.some((c) => c.name === last)) wanted = last
    }
    const entry = wanted ? clips.value.find((c) => c.name === wanted) : null
    if (!entry) {
      if (clipName.value) toast(`片段 ${clipName.value} 不存在，请从列表重新选择`, 'err')
      dir.value = ''
      ready.value = false
      cfg.value = null
      return
    }
    dir.value = entry.dir || clipDirOf(wanted)
    if (clipName.value !== wanted) {
      router.replace({ name: 'project-polish-clip', params: { pid: pid.value, clip: wanted } })
    }
    try {
      cfg.value = await fetchProject(pid.value, dir.value, true)
      title.value = cfg.value?.project?.name || cfg.value?.source?.video || '精修编辑器'
      ready.value = true
    } catch (e) {
      // 静默刷新失败：保留旧画面但必须报出来，否则用户以为改动已生效
      if (!silent) ready.value = false
      else toast('刷新失败：' + e.message, 'err')
    }
    try {
      subs.value = (await fetchSubtitles(pid.value, dir.value)).segments || []
    } catch { if (!silent) subs.value = [] }
    subs.value.sort((a, b) => (Number(a.start) || 0) - (Number(b.start) || 0))
    try {
      const r = await fetch(mediaUrl(dir.value, 'data/danmaku.json'))
      if (r.ok) dm.value = await r.json()
    } catch { if (!silent) dm.value = [] }
    refreshSnapshots()
    await refreshClipEntry()
  } finally {
    if (!silent) loading.value = false
  }
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

/** 片段列表 + 每片段的精修状态。loading 由调用方（load / 任务回调）自行管理，这里不动。 */
async function loadClips() {
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
  // 绝对目录（服务端 scope 守卫校验归属），不要自己算相对前缀
  startJob('init', () => initPolish(pid.value, c.dir), { after: loadClips })
  toast('正在初始化 ' + c.name)
}

const fragRows = computed(() => clips.value.map((c) => ({ name: c.name, thumb: thumbSrc(c), ...clipRow(c) })))

function initClipByName(name) {
  const c = clips.value.find((x) => x.name === name)
  if (c) initClip(c)
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
  startJob('init', () => initPolish(pid.value, dir.value), { after: () => { segKey.value++; load({ silent: true }) } })
}

function runLlmJob() {
  // LLM 会重写 segments → 时间轴重挂载拉新数据
  startJob('llm', () => runLlm(pid.value, dir.value), { after: () => { segKey.value++; load({ silent: true }) } })
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
  // `output_file` 必须是**项目内相对路径**（服务端 jail 校验）；留空则用 polish.toml 里的 project.output
  // 渲染会重新生成封面文件 → 封面设计器重挂载拉新图
  startJob('render', () => runRender(pid.value, dir.value, renderOutput.value.trim()), { after: () => { coverKey.value++; load({ silent: true }) } })
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
    startJob('process', () => runProcess(pid.value, dir.value), { after: () => load({ silent: true }) })
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
      return startJob('process', () => runProcess(pid.value, dir.value), { after: () => load({ silent: true }) })
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

// 九宫格布局已由「布局框选」取代：区域矩形一律走 layout[region].top_left/bottom_right

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
/* 页面专属布局（样稿无同名类）：编辑区（预览+时间轴左列 / 生成与导出右栏）、
   LLM 剪辑参数块、紧凑片段条、贴纸来源选择器、产物体 */
/* 左列上下排列「预览 + 时间轴」，右栏保持自然高度（不跟着左列拉长） */
.edit-grid { align-items:start; }
.left-col { display:flex; flex-direction:column; gap:16px; min-width:0; }
.frag-strip { padding:8px 10px; }
/* 时间轴卡：自身 padding 收窄（内容已经很紧凑，竖高按 26px/泳道算过） */
.tl-card { padding:12px; }
.gen-block { border:1px solid var(--line); border-radius:var(--r-sm); padding:10px 12px; background:var(--bg-soft); }
.ss-chips { display:flex; flex-wrap:wrap; gap:6px; }
.ss-chip { display:inline-flex; align-items:center; gap:5px; padding:3px 6px 3px 10px; border-radius:var(--r-full); background:var(--main-soft); color:var(--main-deep); font-size:12px; font-weight:600; }
.ss-x { border:none; background:none; color:inherit; cursor:pointer; display:flex; padding:2px; border-radius:50%; }
.ss-x:hover { background:var(--red-soft); color:var(--red-deep); }
.ss-input { width:auto; }
.ss-available { border-top:1px dashed var(--line-soft); padding-top:8px; max-height:150px; overflow:auto; }
.ss-avail-item { display:flex; align-items:center; justify-content:space-between; gap:8px; padding:4px 8px; border-radius:8px; cursor:pointer; font-size:12px; }
.ss-avail-item:hover { background:var(--bg-soft); }
.ss-avail-item.on { color:var(--ink-faint); text-decoration:line-through; }
.job-box { border:1px solid var(--line); border-radius:var(--r-sm); padding:8px 10px; background:var(--bg-soft); }
.artifact-bar .cover-thumb { width:150px; border-radius:var(--r-sm); border:1px solid var(--line); }
.artifact-bar .video-bar { max-height:32vh; border-radius:var(--r-sm); }
.cover-img { width:100%; border-radius:var(--r-sm); border:1px solid var(--line); display:block; }
.thumb { width:100%; aspect-ratio:16/9; object-fit:cover; border-radius:var(--r-sm); margin-top:10px; border:1px solid var(--line); }
</style>
