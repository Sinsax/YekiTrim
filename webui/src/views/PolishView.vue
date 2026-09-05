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

    <!-- 未选择片段：可精修片段选择器 -->
    <div v-if="!dir" class="grid cols-4">
      <div v-for="c in clips" :key="c.dir" class="pcard" @click="pickClip(c)">
        <div class="head">
          <div class="sticker pink"><Icon name="scissors" :size="18" /></div>
          <div>
            <h3>{{ c.meta?.title || c.name }}</h3>
            <div class="path">{{ c.name }} · {{ Math.round(c.duration_secs || 0) }}s</div>
          </div>
        </div>
        <img v-if="c.video_exists" :src="thumbSrc(c)" class="thumb" alt="" @error="$event.target.style.display='none'">
        <div class="foot">
          <span class="name">{{ c.meta?.title ? c.name : '' }}</span>
          <button class="mini-btn" @click.stop="pickClip(c)">打开精修 <Icon name="arrow-right" :size="11" /></button>
        </div>
      </div>
      <div v-if="!clips.length && !loading" class="empty">
        <div class="empty-icon"><Icon name="film" :size="24" /></div>
        暂无可精修片段，请先在片段页剪切
      </div>
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
        <!-- 顶排：动作（含贴纸配置）+ 状态 -->
        <div class="top-cards">
          <div class="card">
            <div class="action-grid">
              <div class="col">
                <div class="row spread mb-8">
                  <h2 class="m-0 card-h2"><Icon name="sparkles" :size="15" /> 动作</h2>
                  <span class="muted small">LLM 生成后自动 process</span>
                </div>
                <div class="row">
                  <button class="btn btn-primary" :disabled="jobRunning" @click="runLlmJob"><Icon name="sparkles" :size="13" /> LLM 生成</button>
                  <button class="btn" :disabled="jobRunning" @click="renderModal = true"><Icon name="film" :size="13" /> 渲染导出</button>
                  <button class="btn save-btn" :class="{ dirty: editDirty }" :disabled="jobRunning" @click="saveEdits">
                    <Icon name="save" :size="13" /> {{ editDirty ? '保存编辑*' : '保存编辑' }}
                  </button>
                </div>
                <div v-if="jobRunning || job.status === 'failed'" class="job-box mt-8">
                  <div class="row spread small">
                    <b>{{ jobLabel(job.kind) }}</b>
                    <span class="chip" :class="job.status === 'failed' ? 'r' : 'b'">{{ job.status }}</span>
                  </div>
                  <div v-if="job.total > 0" class="progress mt-6"><div :style="{ width: pctJob + '%' }"></div></div>
                  <div class="muted small mt-6 mono">{{ job.error || job.log || '运行中…' }}</div>
                </div>
              </div>
              <div class="col">
                <div class="row spread mb-8">
                  <h2 class="m-0 card-h2"><Icon name="image" :size="14" /> 贴纸配置</h2>
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
            </div>
          </div>

          <div class="card">
            <div class="row spread mb-8">
              <h2 class="m-0 card-h2"><Icon name="check" :size="15" /> 状态</h2>
              <div class="row">
                <span class="muted small">版本</span>
                <button class="mini-btn" :class="{ selected: version !== 'silent' }" @click="switchVersion('original')">原版</button>
                <button class="mini-btn" :class="{ selected: version === 'silent' }" @click="switchVersion('silent')">消音版</button>
              </div>
            </div>
            <div class="chips">
              <span class="chip" :class="clipEntry?.processed ? 'g' : 'n'">{{ clipEntry?.processed ? '已 process' : '未 process' }}</span>
              <span class="chip" :class="clipEntry?.rendered ? 'g' : 'n'">{{ clipEntry?.rendered ? '已渲染' : '未渲染' }}</span>
              <span class="chip" :class="clipEntry?.cover ? 'g' : 'n'">{{ clipEntry?.cover ? '有封面' : '无封面' }}</span>
            </div>
          </div>
        </div>

        <div class="layout mt-16">
          <div class="left">
            <div class="card">
              <div class="row spread">
                <h2 class="m-0 card-h2"><Icon name="video" :size="16" /> 视频预览（preview 效果）</h2>
                <span class="muted small">字幕 / 贴纸 / 运镜 / 弹幕 / 音效实时预览</span>
              </div>
              <PolishPreview :key="previewKey" :dir="dir" :config="cfg" :subtitles="subs" :danmaku="dm" :meta-duration="cfg?.duration || 0" @time="onPreviewTime" />
            </div>

            <div class="card">
              <div class="row spread">
                <h2 class="m-0 card-h2"><Icon name="edit" :size="15" /> 时间轴编辑器</h2>
                <span class="muted small">拖拽移动 / 右缘缩放 / 双击删除 / 泳道 ＋ 添加</span>
              </div>
              <TimelineEditor ref="timelineRef" :dir="dir" :current-time="currentTime" @seek="seekTo" @save="onTimelineSave" />
            </div>
          </div>

          <!-- 右栏：布局 / 字幕 二合一切换，拉伸到与时间轴对齐 -->
          <div class="right">
            <div class="card panel-card">
              <div class="seg-toggle row mb-8">
                <button class="mini-btn" :class="{ selected: panel === 'grid' }" @click="panel = 'grid'">
                  <Icon name="settings" :size="12" /> 九宫格布局
                </button>
                <button class="mini-btn" :class="{ selected: panel === 'subs' }" @click="panel = 'subs'">
                  <Icon name="list" :size="12" /> 字幕列表
                </button>
              </div>

              <div v-show="panel === 'grid'">
                <div v-for="r in layoutRegions" :key="r.key" class="my-8">
                  <div class="small muted mb-4">{{ REGION_LABELS[r.key] || r.key }}<span v-if="(r.value.grid || []).length > 1" class="muted">（可多选）</span></div>
                  <div class="grid-3">
                    <div v-for="cell in GRID_CELLS" :key="cell" class="grid-cell"
                         :class="{ active: (r.value.grid || []).includes(cell) }"
                         @click="toggleGridCell(r.key, cell, r.value)">{{ cell }}</div>
                  </div>
                </div>
                <div v-if="!layoutRegions.length" class="empty small">暂无布局配置</div>
              </div>

              <div v-show="panel === 'subs'" class="panel-body">
                <SubtitleEditor :segments="subs" :current-time="currentTime" @save="saveSubs" @play="seekTo" />
                <div class="muted small mt-8">与项目预览共用同一份字幕数据</div>
              </div>
            </div>
          </div>
        </div>

        <!-- 产物：底部独立区块 -->
        <div class="card mt-16">
          <div class="row spread mb-8">
            <h2 class="m-0 card-h2"><Icon name="image" :size="15" /> 产物</h2>
            <a v-if="renderedUrl" class="btn" :href="renderedUrl" download><Icon name="download" :size="13" /> 下载成片</a>
          </div>
          <div v-if="coverUrl || renderedUrl" class="product-grid">
            <img v-if="coverUrl" :src="coverUrl" class="cover-img" alt="封面" @error="onCoverError">
            <video v-if="renderedUrl" :src="renderedUrl" controls playsinline class="video-lg product-video" @error="onRenderedError"></video>
          </div>
          <div v-else class="empty small">暂无产物，渲染导出或生成封面后显示</div>
        </div>

        <div class="mt-16"><CoverDesigner :dir="dir" /></div>
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
import { ref, computed, reactive, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import Icon from '../components/Icon.vue'
import AppModal from '../components/AppModal.vue'
import PolishPreview from '../components/PolishPreview.vue'
import TimelineEditor from '../components/TimelineEditor.vue'
import CoverDesigner from '../components/CoverDesigner.vue'
import SubtitleEditor from '../components/SubtitleEditor.vue'
import { mediaUrl } from '../utils'
import { api } from '../api/client'
import {
  fetchProject, fetchSubtitles, saveSubtitles, saveProjectPatch,
  selectVersion, initPolish, runLlm, runProcess, runRender, watchJob,
  fetchStickerSources,
  GRID_CELLS, REGION_LABELS,
} from '../polish/api'

const route = useRoute()
const router = useRouter()
const routeCfg = decodeURIComponent(route.params.cfg || '')
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
const currentTime = ref(0)
const previewKey = ref(0)
const panel = ref('grid') // 右栏二合一切换：grid 布局 | subs 字幕
const renderModal = ref(false)
const renderOutput = ref('')
const coverName = ref('cover.jpg')
const renderedName = ref('export.mp4')
const timelineRef = ref(null)
const savedSegs = ref('')      // 上次保存的时间轴 JSON 快照（脏状态比对）
const savedSubsJson = ref('')  // 上次保存的字幕 JSON 快照

const job = reactive({ id: '', kind: '', status: '', current: 0, total: 0, log: '', error: '' })
let jobClose = null

const version = computed(() => clipEntry.value?.version || 'original')
const jobRunning = computed(() => job.status === 'running' || job.status === 'queued' || job.status === 'awaiting_input')
const pctJob = computed(() => job.total > 0 ? Math.round(job.current * 100 / job.total) : 0)
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
const coverUrl = computed(() => clipEntry.value?.cover ? mediaUrl(dir.value, coverName.value) : '')
const renderedUrl = computed(() => clipEntry.value?.rendered ? mediaUrl(dir.value, renderedName.value) : '')

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
  currentTime.value = Number(t) || 0
}

function pickClip(c) {
  router.replace({ query: { ...route.query, dir: c.dir } })
}
function clearDir() {
  router.replace({ query: { ...route.query, dir: undefined } })
}

async function load() {
  if (route.query.dir) {
    dir.value = decodeURIComponent(route.query.dir)
  } else if (clipName.value && routeCfg) {
    try {
      const st = await api(`/api/project-data/status?config=${encodeURIComponent(routeCfg)}`)
      dir.value = st.output_dir + '/clips/' + clipName.value
    } catch { dir.value = '' }
  } else {
    dir.value = ''
  }
  if (!dir.value) {
    ready.value = false
    cfg.value = null
    loadClips()
    return
  }
  loading.value = true
  try {
    cfg.value = await fetchProject(dir.value, true)
    title.value = cfg.value?.project?.name || cfg.value?.source?.video || '精修编辑器'
    ready.value = true
  } catch {
    ready.value = false
  }
  try {
    subs.value = (await fetchSubtitles(dir.value)).segments || []
  } catch { subs.value = [] }
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
  return `/api/project-data/clips/${encodeURIComponent(rank)}/thumb?config=${encodeURIComponent(routeCfg)}`
}

async function loadClips() {
  loading.value = true
  try {
    if (!routeCfg) { clips.value = []; return }
    const list = await api(`/api/project-data/clips?config=${encodeURIComponent(routeCfg)}`)
    clips.value = (Array.isArray(list) ? list : []).filter((c) => c.video_exists)
  } catch { clips.value = [] } finally {
    loading.value = false
  }
}

async function refreshClipEntry() {
  try {
    const list = routeCfg ? await api(`/api/project-data/polish?config=${encodeURIComponent(routeCfg)}`) : []
    const norm = (p) => String(p || '').replace(/\\/g, '/').toLowerCase()
    clipEntry.value = (list || []).find((p) => norm(p.dir) === norm(dir.value)) || null
  } catch { clipEntry.value = null }
}

// ---- 任务驱动 ----
function startJob(kind, call, { after } = {}) {
  if (jobRunning.value) { toast('已有任务在运行，请稍候', 'err'); return Promise.resolve(false) }
  return call().then((r) => {
    Object.assign(job, { id: r.id, kind, status: 'running', current: 0, total: 0, log: '', error: '' })
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
        if (s.status === 'succeeded') {
          toast(jobLabel(kind) + '完成')
          previewKey.value++
          after && after()
        } else if (s.status === 'failed') {
          toast(jobLabel(kind) + '失败：' + (job.error || s.message || '未知错误'), 'err')
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
  startJob('init', () => initPolish(dir.value), { after: load })
}

function runLlmJob() {
  startJob('llm', () => runLlm(dir.value), { after: load })
}

function doRender() {
  renderModal.value = false
  startJob('render', () => runRender(dir.value, renderOutput.value.trim()), { after: load })
}

// 保存编辑：时间轴分段 / 字幕，各自仅在变动时提交
async function saveEdits() {
  if (!editDirty.value) { toast('没有需要保存的修改'); return }
  let segsChanged = false
  const segs = timelineRef.value?.segments
  if (segs && JSON.stringify(segs) !== savedSegs.value) {
    try {
      await saveProjectPatch(dir.value, { segments: segs })
      savedSegs.value = JSON.stringify(segs)
      segsChanged = true
    } catch (e) { toast('保存时间轴失败：' + e.message, 'err'); return }
  }
  if (JSON.stringify(subs.value) !== savedSubsJson.value) {
    try {
      await saveSubtitles(dir.value, subs.value)
      savedSubsJson.value = JSON.stringify(subs.value)
      previewKey.value++
    } catch (e) { toast('保存字幕失败：' + e.message, 'err'); return }
  }
  toast('编辑已保存')
  if (segsChanged) {
    // 时间轴变更后隐式 process（完成后刷新预览）
    startJob('process', () => runProcess(dir.value), { after: load })
  }
}

function onTimelineSave(segments) {
  saveProjectPatch(dir.value, { segments })
    .then(() => {
      toast('时间轴已保存，自动执行后处理…')
      return startJob('process', () => runProcess(dir.value), { after: load })
    })
    .catch((e) => toast(e.message, 'err'))
}

// 九宫格多选：点击切换格子是否在区域 grid 内
async function toggleGridCell(key, cell, region) {
  try {
    const grid = [...(region?.grid || [])]
    const i = grid.indexOf(cell)
    if (i >= 0) grid.splice(i, 1)
    else grid.push(cell)
    if (!grid.length) { toast('至少保留一个格子', 'err'); return }
    const lay = { ...(cfg.value?.layout || {}) }
    lay[key] = { ...(region || {}), grid }
    await saveProjectPatch(dir.value, { layout: lay })
    cfg.value = { ...cfg.value, layout: lay }
  } catch (e) { toast(e.message, 'err') }
}

async function switchVersion(v) {
  if (v === version.value) return
  try {
    await selectVersion(dir.value, v)
    toast('已切换为 ' + (v === 'silent' ? '消音版' : '原版'))
    previewKey.value++
    await refreshClipEntry()
  } catch (e) { toast(e.message, 'err') }
}

async function saveSubs() {
  try {
    await saveSubtitles(dir.value, subs.value)
    savedSubsJson.value = JSON.stringify(subs.value)
    toast('字幕已保存')
    previewKey.value++
  } catch (e) { toast(e.message, 'err') }
}

// 添加字幕：开始 = 当前播放时间，结束 = +3s
// ---- 贴纸配置（sticker_source，直存 polish.toml）----
async function loadStickerSources() {
  try {
    stickerSources.value = (await fetchStickerSources()).sources || []
  } catch { stickerSources.value = [] }
}

async function patchStickerSource(list) {
  try {
    await saveProjectPatch(dir.value, { sticker_source: list })
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

function onCoverError(e) {
  if (coverName.value !== '_cover_frame.jpg') {
    coverName.value = '_cover_frame.jpg'
  } else {
    e.target.style.display = 'none'
  }
}
function onRenderedError() {
  if (renderedName.value === 'export.mp4') renderedName.value = 'vtuber-clip.mp4'
}

onMounted(() => {
  load()
  loadStickerSources()
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
.product-grid { display:grid; grid-template-columns:1fr 1fr; gap:16px; align-items:start; }
.cover-img { width:100%; border-radius:var(--r-sm); border:1px solid var(--line); display:block; }
.product-video { max-height:46vh; }
.thumb { width:100%; aspect-ratio:16/9; object-fit:cover; border-radius:var(--r-sm); margin-top:10px; border:1px solid var(--line); }
@media (max-width: 1100px) {
  .top-cards, .layout, .action-grid { grid-template-columns:1fr; }
}
</style>
