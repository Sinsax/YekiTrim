<template>
  <div>
    <div v-if="loadError" class="error-banner"><Icon name="alert" :size="15" /> {{ loadError }}</div>

    <div class="grid cols-main">
      <!-- 左：播放器 + 字幕轨道 + 切片标记 -->
      <div>
        <div class="card player-card">
          <div class="row spread mb-8">
            <b class="t" style="margin:0">原视频</b>
            <span class="muted small mono">{{ inputName || '未记录输入文件' }}</span>
          </div>
          <div v-if="src" class="video-shell">
            <video
              ref="videoRef"
              controls
              playsinline
              preload="metadata"
              class="preview-video"
              :src="src"
              @timeupdate="currentTime = $event.target.currentTime"
              @play="playing = true"
              @pause="playing = false"
              @loadedmetadata="onMeta"
            ></video>
            <!-- 字幕叠层：与精修预览同一套样式（72px@1920 缩放、同色同描边） -->
            <div v-if="activeSub" class="cap-overlay">{{ activeSub.text }}</div>
          </div>
          <div v-else class="empty small">暂无输入视频（先在项目设置里填写 input_file，或跑一次音频提取）</div>
          <div class="row mt-8">
            <button class="mini-btn" @click="togglePlay">{{ playing ? '暂停' : '播放' }}（空格）</button>
            <span class="muted small mono">{{ fmtTime(currentTime) }}</span>
          </div>
        </div>

        <div class="card mt-12">
          <div class="row spread mb-6">
            <b class="t" style="margin:0">字幕轨道</b>
            <span class="muted small" :class="{ err: saveState === 'error' }">{{ saveHint }}</span>
          </div>
          <SubtitleTrack
            :segments="segments"
            :duration="duration"
            :current-time="currentTime"
            :mark-in="markIn"
            :mark-out="markOut"
            @save="onTrackSave"
            @seek="seekTo"
          />
        </div>
      </div>

      <!-- 右：预处理 + 切片（动作集中在一列；切片块紧贴预处理下方，别占左列的竖高） -->
      <div class="side-col">
        <div class="card card-blk">
          <b class="t">预处理</b>
          <div class="row" style="gap:6px;flex-wrap:wrap;margin:8px 0 10px">
            <span class="chip" :class="chipClass('transcribe')" :title="transcribeChipTitle">转写 {{ chipText('transcribe') }}</span>
            <span class="chip" :class="chipClass('detect')">事件检测 {{ chipText('detect') }}</span>
          </div>

          <!-- ASR 模式不在这里选：固定按配置里的 mode（默认 hybrid），改法在设置页 -->

          <!-- 输入素材：弹幕 / 字幕都是**随时可换的输入**（值写进 vtrim.toml），不是产物。
               弹幕决定热度评分与 clip.dm.srt；字幕是事件检测的输入（有现成字幕就不跑 ASR）。 -->
          <div class="assets">
            <div class="asset-row">
              <span class="asset-name">弹幕</span>
              <span class="asset-path mono" :title="assetTitle(dm)">{{ assetText(dm) }}</span>
              <span class="chip" :class="assetChipClass(dm)">{{ assetChip(dm) }}</span>
              <button class="mini-btn" @click="openPicker('danmaku')">选择…</button>
              <button v-if="dm && dm.value" class="mini-btn" title="清除（不再使用弹幕）" @click="clearAsset('danmaku')">清除</button>
            </div>
            <div class="asset-row">
              <span class="asset-name">字幕</span>
              <span class="asset-path mono" :title="assetTitle(tx)">{{ assetText(tx) }}</span>
              <span class="chip" :class="assetChipClass(tx)">{{ assetChip(tx) }}</span>
              <span
                v-if="tx && tx.value && tx.external"
                class="chip a"
                title="字幕来自外部文件（这个工程没跑过 ASR）"
              >外部字幕</span>
              <button class="mini-btn" @click="openPicker('transcript')">选择…</button>
              <button v-if="tx && tx.value" class="mini-btn" title="清除（回落到 output/ 里的转写产物）" @click="clearAsset('transcript')">清除</button>
            </div>
            <div class="hint mt-6">{{ assetsHint }}</div>
          </div>

          <!-- 转写 + 事件检测合成一个按钮：两段仍是**两个任务**依次跑（转写完再起检测），
               所以后一段失败只需再点一次，已经跑完的前一段不会重跑。
               ⚠️ 按钮**只受 busy 限制**（不再是"跑完就永久置灰"）：两段都完成后它就是
               「重新检测」（点之前二次确认）。转写一生只跑一次 —— 换字幕请用上面那一行。 -->
          <button class="btn btn-primary run-btn" :disabled="busy" :title="prepareTitle" @click="runPrepare">
            <span v-if="busy" class="spinner"></span>
            {{ prepareLabel }}
          </button>

          <div v-if="job.id" class="job-box mt-12">
            <div class="row spread small">
              <b>{{ kindText(job.kind) }}</b>
              <span class="chip" :class="statusClass(job.status)">{{ statusText(job.status) }}</span>
            </div>
            <div class="muted small mt-2">{{ job.message || '运行中' }}</div>
            <div v-if="job.total > 0" class="progress mt-6"><div :style="{ width: jobPct + '%' }"></div></div>
            <button class="mini-btn mt-6" @click="openJobs()">查看任务</button>
          </div>

          <div class="hint mt-8">{{ prepareHint }}</div>
        </div>

        <!-- 切片：标记起止 → 填标题 →「切片」即建事件并开始剪切（没填标题会打开表单）。
             原来散在左列最下面的切片行 + 预处理卡里的「手动添加片段」入口都收进这一块。 -->
        <div class="card card-blk mt-12">
          <b class="t">切片</b>
          <div class="row" style="gap:6px;margin:8px 0 6px">
            <button class="mini-btn" style="flex:none" @click="mark('in')">标记入点</button>
            <input class="fld-input slice-time" :value="fmtTime(markIn)" readonly title="入点（当前播放位置）">
          </div>
          <div class="row" style="gap:6px;margin-bottom:8px">
            <button class="mini-btn" style="flex:none" @click="mark('out')">标记出点</button>
            <input class="fld-input slice-time" :value="fmtTime(markOut)" readonly title="出点（当前播放位置）">
          </div>
          <input
            v-model="label"
            class="fld-input"
            placeholder="片段标题"
            title="标题只用于人工查看（events.json / clip.json / 卡片展示）"
            @keyup.enter="sliceNow"
          >
          <div class="row" style="gap:6px;margin-top:8px">
            <button class="btn btn-primary" style="flex:1;justify-content:center"
                    :disabled="!canSlice || busy"
                    :title="canSlice ? '按标记的起止建事件并立即开始剪切' : '先标记入点和出点'"
                    @click="sliceNow">切片</button>
            <button class="mini-btn" :disabled="!canSlice" title="补类型 / 详情（标题已填则直接切）" @click="openAdd">更多设置</button>
          </div>
          <div class="hint mt-6">
            标题只用于人工查看，不影响目录与文件名（固定 <span class="mono">clip-NN/clip.mp4</span>）；没填标题会打开表单。
          </div>
        </div>
      </div>
    </div>

    <!-- 选择弹幕 / 字幕文件：候选来自项目目录（服务端扫），admin 还能浏览服务器任意目录 -->
    <AppModal :show="picker.show" :title="pickerTitle" wide @close="closePicker">
      <div class="muted small mb-6">
        项目目录里的候选（同名 = 与视频文件名相同）。选中后写进
        <span class="mono">vtrim.toml</span>，随时可以换。
      </div>
      <div class="file-box">
        <div v-if="!candidateList.length" class="empty small">项目目录里没有找到匹配的文件（admin 可以在下面换目录找）</div>
        <div
          v-for="c in candidateList"
          :key="c.rel"
          class="file-row"
          :class="{ selected: pickerRel === c.rel }"
          @click="pickCandidate(c.rel)"
        >
          <span class="mono flex-1">{{ c.rel }}</span>
          <span v-if="c.same_stem" class="chip a">同名</span>
          <span class="muted small" style="flex:none">{{ humanBytes(c.size) }}</span>
        </div>
      </div>

      <template v-if="isAdmin">
        <label class="mt-10">在服务器上浏览别的目录（可选）</label>
        <ServerDirPicker
          mode="file"
          :exts="pickerExts"
          :start-dir="assetsRoot"
          @update:dir="onPickerDir"
          @pick-file="pickServerFile"
        />
      </template>
      <div v-else class="hint mt-8">只能选项目目录内的文件 —— 上面列的就是项目里的全部候选。</div>

      <div v-if="pickerPicked" class="hint mt-8">已选：<span class="mono">{{ pickerLabel }}</span></div>

      <template #footer>
        <button class="btn" @click="closePicker">取消</button>
        <button class="btn btn-primary" :disabled="!pickerPicked || picker.busy" @click="applyPick">
          <span v-if="picker.busy" class="spinner"></span>
          使用所选
        </button>
      </template>
    </AppModal>

    <!-- 手动添加片段（轨道标记的落地表单） -->
    <AppModal :show="showAdd" title="添加片段" @close="showAdd = false">
      <div class="row">
        <div class="flex-1 minw-120"><label>开始（秒）</label><input v-model.number="start" type="number" step="0.1" min="0"></div>
        <div class="flex-1 minw-120"><label>结束（秒）</label><input v-model.number="end" type="number" step="0.1" min="0"></div>
      </div>
      <div class="row mt-8">
        <div class="flex-2 minw-180"><label>标题（仅人工查看，不影响文件名）</label><input v-model="label" placeholder="片段标题"></div>
        <div class="flex-1 minw-120">
          <label>类型</label>
          <select v-model="eventType">
            <option v-for="t in EVENT_TYPES" :key="t" :value="t">{{ TYPE_ZH[t] || t }}</option>
          </select>
        </div>
      </div>
      <label class="mt-8">详情（可选）</label>
      <textarea v-model="summary" rows="2" placeholder="给片段写点备注，留空也行"></textarea>
      <template #footer>
        <button class="btn" @click="showAdd = false">取消</button>
        <button class="btn btn-primary" :disabled="!canAddClip" @click="saveClip">生成片段</button>
      </template>
    </AppModal>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Icon from '../components/Icon.vue'
import AppModal from '../components/AppModal.vue'
import ServerDirPicker from '../components/ServerDirPicker.vue'
import SubtitleTrack from '../components/prepare/SubtitleTrack.vue'
import { projectData } from '../api/projectData'
import { humanBytes } from '../stores/project'
import { isAdmin } from '../stores/auth'
import { watchJob, kindText, statusClass, statusText, busyOn, outKey, QUEUED_HINT } from '../api/jobs'
import { useJobs } from '../composables/useJobs'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import { openJobs } from '../stores/ui'
import { EVENT_TYPES, TYPE_ZH, inputMediaUrl, redetectWarning } from '../utils'

const route = useRoute()
const router = useRouter()
const pid = computed(() => route.params.pid || '')

const src = ref('')
const inputName = ref('')
const videoDuration = ref(0)
const videoRef = ref(null)
const currentTime = ref(0)
// 当前时间点正在显示的字幕（叠层渲染，效果对齐精修预览）
const activeSub = computed(() => segments.value.find(
  (s) => currentTime.value >= (Number(s.start) || 0) && currentTime.value < (Number(s.end) || 0)
) || null)
const playing = ref(false)
const loadError = ref('')

const segments = ref([])
/** 轨道总长：视频元数据时长与最后一条字幕取大者（视频还没加载时只能靠字幕兜底） */
const duration = computed(() => Math.max(
  videoDuration.value,
  segments.value.reduce((m, s) => Math.max(m, Number(s.end) || 0), 0)
))
const status = ref(null)
const markIn = ref(0)
const markOut = ref(0)
const showAdd = ref(false)
const start = ref(0)
const end = ref(10)
const label = ref('')
const summary = ref('')
const eventType = ref('other')

const job = reactive({ id: '', kind: '', status: '', current: 0, total: 0, message: '' })
let jobClose = null
let saveTimer = null

// 字幕自动保存状态（编辑内容/起止都即时落盘，用状态文案代替每次 toast 刷屏）
const saveState = ref('')      // '' | 'saving' | 'saved' | 'error'
const saveError = ref('')
const saveAt = ref('')
const saveHint = computed(() => {
  if (saveState.value === 'saving') return '保存中…'
  if (saveState.value === 'error') return '保存失败：' + saveError.value
  if (saveState.value === 'saved') return '已保存 ' + saveAt.value
  return '点块在上方属性条改文本与入点 / 出点，改动自动保存'
})

const jobRunning = computed(() => job.status === 'running' || job.status === 'queued')
const jobPct = computed(() => (job.total > 0 ? Math.round((job.current * 100) / job.total) : 0))
const runningStage = computed(() => (jobRunning.value ? jobStage.value : ''))
const jobStage = computed(() => {
  const k = job.kind || ''
  if (k === 'stage_detect') return 'detect'
  if (k === 'stage_transcribe' || k === 'pipeline') return 'transcribe'
  return ''
})
// 项目级作用域（`p{pid}:out`）：转写/检测/订正/剪切都写 output/*，互相排队。
// 别的项目在跑、或某个片段在精修，都不会让这个按钮变灰（并行能力）。
const { jobs: allJobs, reload: reloadJobs } = useJobs()
const scopeBusyJob = computed(() => busyOn(allJobs.value, [outKey(pid.value)]))
const busy = computed(() => jobRunning.value || !!scopeBusyJob.value)
const busyText = computed(() => (jobRunning.value ? kindText(job.kind) : scopeBusyJob.value ? kindText(scopeBusyJob.value.kind) : ''))
const hasTranscript = computed(
  () => segments.value.length > 0 || (status.value?.files || []).some((f) => String(f).startsWith('字幕'))
)
/** 已有检测结果（`status.files` 里带「事件」那一项）→ 重新检测要二次确认 */
const hasEvents = computed(() => hasFile('事件'))
/** 转写成功后自动接着跑检测（两段之间不留手动步骤） */
const chainDetect = ref(false)

// ---- 输入素材（弹幕 / 字幕）：值写进 vtrim.toml，随时可换 ----
const assets = ref(null)
const assetDirty = ref(false)
const assetsRoot = computed(() => assets.value?.root_dir || '')
const dm = computed(() => assets.value?.danmaku || null)
const tx = computed(() => assets.value?.transcript || null)
const picker = reactive({ show: false, kind: 'danmaku', rel: '', dir: '', name: '', busy: false })
const pickerTitle = computed(() => (picker.kind === 'danmaku' ? '选择弹幕文件' : '选择字幕文件'))
/** 弹幕只认 xml/ass/srt；字幕认 srt/json（与后端 `assets::classify` 同一张表） */
const pickerExts = computed(() => (
  picker.kind === 'danmaku' ? ['xml', 'ass', 'srt'] : ['srt', 'json']
))
const candidateList = computed(() => assets.value?.candidates?.[picker.kind] || [])
/** 候选（项目内相对路径）或服务器浏览选中的文件，二者取其一 */
const pickerRel = computed(() => picker.rel || '')
const pickerPicked = computed(() => picker.rel || picker.name)
const pickerLabel = computed(() => (
  picker.name ? `${picker.dir}\\${picker.name}` : picker.rel
))

/** 素材行的路径文案：配置值优先；没有配置值时显示 output/ 兜底的那份产物。 */
function assetText(s) {
  if (!s) return '—'
  const v = String(s.value || '').trim()
  if (v) return v
  const abs = String(s.abs || '')
  if (abs) {
    const name = abs.split(/[\\/]/).filter(Boolean).pop() || abs
    return 'output/' + name
  }
  // 什么都没设：路径格留个破折号，状态交给右边的 chip（两格都写「未设置」很吵）
  return '—'
}
function assetTitle(s) {
  if (!s) return '素材状态未加载'
  const abs = String(s.abs || '').trim()
  if (!abs) return '未设置'
  return abs + (s.exists ? '' : '（文件不存在）')
}
function assetChip(s) {
  if (!s) return '未知'
  if (!s.exists) return s.value ? '文件不存在' : '未设置'
  return s.value ? '已载入' : '自动'
}
function assetChipClass(s) {
  if (!s) return 'n'
  if (!s.exists) return s.value ? 'r' : 'n'
  return 'g'
}
const assetsHint = computed(() => {
  if (!assets.value) return '素材读取失败（不影响预处理，重进页面可再试）'
  if (assetDirty.value) return '素材已切换 —— 点下面的按钮重新检测才会生效。'
  return '弹幕决定热度评分与 clip.dm.srt；字幕是事件检测的输入（有现成字幕就不用跑 ASR）。两者随时可换。'
})
const transcribeChipTitle = computed(() => {
  if (tx.value?.value && tx.value?.external) return '字幕来自外部文件（这个工程没跑过 ASR）'
  return '转写只跑一次：完成后要换字幕，请用「字幕」那一行的「选择…」'
})

const prepareLabel = computed(() => {
  if (runningStage.value === 'transcribe') return `转写中 ${jobPct.value}%`
  if (runningStage.value === 'detect') return `检测中 ${jobPct.value}%`
  if (!hasTranscript.value) return '开始预处理'
  if (!hasEvents.value) return '开始事件检测'
  return '重新检测'
})
const prepareTitle = computed(() => {
  if (!hasTranscript.value) return '依次执行：转写 → 事件检测（转写会自动先提取音频）'
  if (!hasEvents.value) return '转写已完成，接着跑事件检测'
  return '重新检测：整份覆盖 events.json（已剪片段不受影响）'
})
const prepareHint = computed(() => {
  if (runningStage.value) return '正在跑，完成后按钮会回到可用状态。'
  if (!hasTranscript.value) {
    return '一次点击依次跑：转写 → 事件检测。两段是两个任务，后一段失败再点一次即可，已完成的转写不会重跑。'
  }
  if (!hasEvents.value) return '转写已完成，接着跑事件检测（会整份覆盖 events.json，已剪片段不受影响）。'
  return '转写已完成（不可重跑），事件检测可随时重跑 —— 换了弹幕或字幕后点这个按钮生效。'
})
const canSlice = computed(() => markOut.value > markIn.value)
const canAddClip = computed(() => !!(label.value.trim() && end.value > start.value))

function fmtTime(t) {
  const v = Math.max(0, Number(t) || 0)
  const m = Math.floor(v / 60)
  const s = (v % 60).toFixed(1)
  return `${String(m).padStart(2, '0')}:${s.padStart(4, '0')}`
}

/** chips：由 status.files + 运行中任务推导（待开始 / 进行中 % / ✓）。 */
function hasFile(prefix) {
  return (status.value?.files || []).some((f) => String(f).startsWith(prefix))
}
function chipText(key) {
  if (runningStage.value === key) return `${jobPct.value}%`
  // 转写这一格看的是**字幕文件**：音频只是转写的副产品，光有 wav 不算转写完。
  if (key === 'transcribe') return hasTranscript.value ? '✓' : '待开始'
  return hasFile('事件') ? '✓' : '待开始'
}
function chipClass(key) {
  const t = chipText(key)
  if (t === '✓') return 'g'
  if (t.endsWith('%')) return 'p'
  return 'n'
}

async function load() {
  loadError.value = ''
  try {
    const st = await projectData.status(pid.value)
    status.value = st
    inputName.value = st.input_file || ''
    if (st.input_abs) src.value = inputMediaUrl(st.input_abs)
  } catch (e) {
    loadError.value = '项目状态读取失败：' + (e?.message || '')
  }
  // ASR 模式不在这里读/写：固定按配置（默认 hybrid），改法在设置页
  try {
    const d = await projectData.transcript(pid.value, '读取字幕')
    segments.value = (d.segments || []).slice().sort((a, b) => (Number(a.start) || 0) - (Number(b.start) || 0))
  } catch {}
  saveState.value = ''
  await loadAssets()
}

/** 读输入素材（弹幕 / 字幕）的当前值与候选。读不到不该挡住整个页面，只少一块信息。 */
async function loadAssets() {
  try {
    assets.value = await projectData.assets(pid.value, '读取输入素材')
  } catch {
    assets.value = null
  }
}

function openPicker(kind) {
  picker.kind = kind
  picker.rel = ''
  picker.name = ''
  picker.dir = assetsRoot.value
  picker.show = true
}

function closePicker() {
  picker.show = false
  picker.rel = ''
  picker.name = ''
}

/** 选一个候选（项目内相对路径） */
function pickCandidate(rel) {
  picker.rel = rel
  picker.name = ''
}

/**
 * 在服务器目录里选中的文件：只把**目录与文件名分开发给服务端**，前端不拼路径。
 * 服务端回传的目录可能是 `\\?\F:\a` 这种 verbatim 形式，前端拼成 `\\?\F:\a/b`
 * 在 Windows 上是 ERROR_INVALID_NAME（首页目录选择器同款教训）。
 */
function pickServerFile(name) {
  picker.name = name
  picker.rel = ''
}

/** 换了浏览目录就把已选的服务器文件清掉（否则会提交"新目录 + 旧文件名"）。 */
function onPickerDir(dir) {
  picker.dir = dir
  picker.name = ''
}

/** 提交选择：服务端写前会解析一遍（坏文件不落盘），返回条数供提示。 */
async function applyPick() {
  if (!pickerPicked.value || picker.busy) return
  const body = picker.name ? { dir: picker.dir, name: picker.name } : { path: picker.rel }
  picker.busy = true
  try {
    const r = await projectData.setAsset(pid.value, picker.kind, body)
    const n = Number(r?.count) || 0
    toast(picker.kind === 'danmaku' ? `弹幕已载入：${n} 条` : `字幕已载入：${n} 条`)
    closePicker()
    await load()
    assetDirty.value = true
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    picker.busy = false
  }
}

/** 清除选择 = 回落到自动（弹幕无；字幕落到 output/ 里的转写产物）。 */
async function clearAsset(kind) {
  const ok = await confirmDialog(
    kind === 'danmaku'
      ? '不再使用弹幕？热度评分会退化成只用字幕密度，切片也不再生成 clip.dm.srt（重新检测后才生效）。'
      : '清除字幕选择？会回落到 output/ 目录里的转写产物（没有的话检测就跑不了）。',
    { title: '清除素材选择' }
  )
  if (!ok) return
  try {
    await projectData.setAsset(pid.value, kind, { path: '' })
    toast('已清除')
    await load()
    assetDirty.value = true
  } catch (e) {
    toast(e.message, 'err')
  }
}

function togglePlay() {
  const v = videoRef.value
  if (!v) return
  if (v.paused) v.play(); else v.pause()
}

/** 视频元数据到位后才知道整场时长：轨道总长用它（没有它时靠最后一条字幕兜底）。 */
function onMeta(e) {
  videoDuration.value = Number(e?.target?.duration) || 0
}

function seekTo(t) {
  // 先记时间（轨道播放头/标记都读它），再试着拖动视频：
  // 媒体源没加载好时 `currentTime` 赋值可能被忽略甚至抛错，不能让跳转失效。
  const n = Math.max(0, Number(t) || 0)
  currentTime.value = n
  const v = videoRef.value
  if (!v) return
  try { v.currentTime = n } catch {}
}

function mark(which) {
  const t = Math.round((videoRef.value?.currentTime || currentTime.value || 0) * 10) / 10
  if (which === 'in') markIn.value = t
  else markOut.value = t
  start.value = markIn.value
  end.value = Math.max(markOut.value, markIn.value + 1)
}

/** 轨道编辑统一入口：`next` 是整份 segments；transient（拖动中）只更新本地，落盘由拖动结束/防抖触发。 */
function onTrackSave(next, opts = {}) {
  segments.value = next
  if (opts.transient) return
  clearTimeout(saveTimer)
  saveTimer = setTimeout(saveSubs, opts.committed ? 0 : 500)
}

async function saveSubs() {
  saveState.value = 'saving'
  try {
    await projectData.saveTranscript(pid.value, segments.value)
    saveState.value = 'saved'
    saveAt.value = new Date().toLocaleTimeString('zh-CN', { hour12: false })
  } catch (e) {
    saveState.value = 'error'
    saveError.value = e.message || '未知错误'
    toast('字幕保存失败：' + (e.message || ''), 'err')
  }
}

/** 预处理主按钮：没转写就先转写（完成后自动接检测），已有转写就（重新）检测。
 *
 *  ⚠️ **转写一生只跑一次**：只要工程里已经有转写（ASR 产物或选中的外部字幕），
 *  这里永远不会再起转写 —— 想换字幕走「字幕」那一行的「选择…」。
 *  也没有"全流程重跑"入口：两段都完成时这个按钮只做重新检测。 */
async function runPrepare() {
  if (busy.value) { toast('该项目正在' + busyText.value + '，请稍候'); return }
  if (!hasTranscript.value) {
    chainDetect.value = true
    await startStage('transcribe')
  } else {
    await startStage('detect')
  }
}

async function startStage(stage) {
  if (busy.value) { toast('该项目正在' + busyText.value + '，请稍候'); chainDetect.value = false; return }
  // 重新检测会整份覆盖 events.json（clips/ 不动，见 utils.js 的 redetectWarning）：
  // 已经有检测结果时先二次确认，别让一次误点把上次的结果冲掉。
  if (stage === 'detect' && hasEvents.value) {
    const ok = await confirmDialog(redetectWarning(0), { title: '覆盖上一次的检测结果？', danger: true })
    if (!ok) { chainDetect.value = false; return }
  }
  try {
    const r = await projectData.pipeline(pid.value, { stage, resume: 'auto' })
    attachJob(r.id, stage === 'transcribe' ? 'stage_transcribe' : 'stage_detect', !!r.queued)
    if (r.queued) toast(QUEUED_HINT)
  } catch (e) {
    chainDetect.value = false
    toast(e.message, 'err')
  }
}

function attachJob(id, kind, queued = false) {
  Object.assign(job, { id, kind, status: queued ? 'queued' : 'running', current: 0, total: 0, message: queued ? '排队中' : '已启动' })
  // `watchJob()` 返回 `{ close() }` 对象（不是函数）：当函数调会抛错，
  // 且下面的订阅就不会执行 → 任务永远停在「运行中」。
  jobClose?.close?.()
  jobClose = watchJob(id, {
    onEvent(e) {
      if (e.type === 'progress') { job.current = e.current; job.total = e.total }
      if (e.type === 'log') job.message = e.message
      if (e.type === 'stage_started') job.message = e.stage + ' 运行中'
    },
    onSummary(s) {
      job.status = s.status
      job.current = s.current
      job.total = s.total
      job.message = s.message || job.message
      if (s.kind) job.kind = s.kind
    },
    onDone: async (s) => {
      job.status = s.status
      jobClose = null
      reloadJobs() // 别等 3 秒轮询：作用域门控要立刻松开
      if (s.status === 'succeeded') toast('任务完成')
      else toast('任务失败：' + (s.error || s.message || '未知错误'), 'err')
      await load()
      // 跑成功了，"素材已切换"那句提示就过期了（切过的弹幕/字幕这一轮已经用上）
      if (s.status === 'succeeded') assetDirty.value = false
      // 转写成功 → 接着起检测任务（转写失败就别往下走，留着让用户处理）
      const chained = chainDetect.value && kind === 'stage_transcribe'
      if (chained) {
        chainDetect.value = false
        if (s.status === 'succeeded') startStage('detect')
      }
    }
  })
  toast('已启动')
}

function openAdd() {
  start.value = markIn.value
  end.value = markOut.value > markIn.value ? markOut.value : markIn.value + 10
  showAdd.value = true
}

/** 切片行主按钮：标题已填 → 直接建事件并剪切（一次点击）；标题空 → 打开完整表单（类型/详情）。 */
function sliceNow() {
  if (!canSlice.value) { toast('先标记入点和出点'); return }
  if (!label.value.trim()) { toast('填个片段标题，或点「更多设置」补类型/详情'); openAdd(); return }
  doCreate({ start: markIn.value, end: markOut.value, label: label.value.trim(), summary: summary.value, type: eventType.value })
}

async function saveClip() {
  if (end.value <= start.value) { toast('结束必须大于开始'); return }
  doCreate({ start: start.value, end: end.value, label: label.value.trim(), summary: summary.value, type: eventType.value })
}

/** 建事件（**标题是唯一必填**，纯展示元数据）→ 立刻剪切 → 进切片页。
 *  目录/文件名固定 `clips/clip-NN/clip.mp4`（见 vtrim-core cutting），标题只写进
 *  `events.json` / `clip.json(labels)` / `publish_config.json` 与界面展示。 */
async function doCreate({ start: s, end: e, label: lb, summary: sm, type: ty }) {
  if (!lb) { toast('请填写标题'); return }
  if (e <= s) { toast('结束必须大于开始'); return }
  try {
    const created = await projectData.createEvent(pid.value, {
      label: lb,
      start_sec: s,
      end_sec: e,
      event_type: ty,
      summary: sm
    })
    const rank = created?.event?.rank
    if (rank) await projectData.cutOne(pid.value, rank)
    showAdd.value = false
    label.value = ''
    summary.value = ''
    toast(`已生成片段「${lb}」，正在剪切`)
    router.push(`/p/${encodeURIComponent(pid.value)}/slices`)
  } catch (e2) {
    toast(e2.message, 'err')
  }
}

function onKey(e) {
  const tag = (e.target?.tagName || '').toLowerCase()
  if (tag === 'input' || tag === 'textarea' || tag === 'select') return
  if (e.code === 'Space') {
    e.preventDefault()
    togglePlay()
  }
}

onMounted(() => {
  load()
  window.addEventListener('keydown', onKey)
})
onUnmounted(() => {
  jobClose?.close?.()
  clearTimeout(saveTimer)
  window.removeEventListener('keydown', onKey)
})
</script>

<style scoped>
.player-card { display:flex; flex-direction:column; }
.video-shell { position:relative; container-type:inline-size; }
.video-shell .preview-video { display:block; width:100%; }
/* 与精修预览字幕同参：默认 72px @1920、#dcbda0、描边 12px #403c3b（types.rs 默认值） */
.cap-overlay { position:absolute; left:5%; right:5%; bottom:15%; text-align:center; pointer-events:none;
  font-weight:700; font-size:3.75cqw; line-height:1.45; color:#dcbda0;
  -webkit-text-stroke:0.625cqw #403c3b; paint-order:stroke fill; }
.preview-video { width:100%; max-height:62vh; object-fit:contain; background:#000; border-radius:var(--r-sm); display:block; }
/* 右列：预处理 + 切片两块上下相邻，顶部对齐（不被左列的视频拉长） */
.side-col { display:flex; flex-direction:column; align-self:start; min-width:0; }
/* 切片块里的入点/出点读数：与左侧「标记」按钮同一行，占满剩余宽度。
   注意 flex-basis 必须写成 0 —— 全局 `input { width:100% }` 会让基线是整行宽，
   按钮被挤到上一行（flex-wrap 下就变成两块各占一行）。 */
.slice-time { flex:1 1 0; width:auto; min-width:0; padding:4px 8px; font-size:12px; text-align:center; }
.run-btn { width:100%; justify-content:center; margin-bottom:8px; }
/* 输入素材两行：名称 | 路径（长名字省略）| 状态 chip | 动作。
   chip 与按钮都钉 `flex:none`（`.mini-btn` 默认可收缩，窄栏里会被压成一条）。 */
.assets { border-top:1px dashed var(--line); padding-top:9px; margin-bottom:10px; }
.asset-row { display:flex; align-items:center; gap:6px; margin-bottom:6px; }
.asset-name { flex:none; width:28px; font-size:12px; font-weight:600; color:var(--ink-dim); }
.asset-path { flex:1 1 auto; min-width:0; font-size:11px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.asset-row .chip, .asset-row .mini-btn { flex:none; }
.hint { font-size:12px; color:var(--ink-dim); }
.job-box { padding:10px; border:1px dashed var(--line); border-radius:var(--r-sm); background:var(--bg-soft); }
</style>
