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
            <span class="muted small">点击块直接改文本，回车/失焦保存</span>
          </div>
          <SubtitleTrack
            :segments="segments"
            :duration="duration"
            :current-time="currentTime"
            @save="onTrackSave"
            @seek="seekTo"
          />
        </div>

        <div class="card mt-12">
          <div class="row" style="gap:8px;flex-wrap:wrap">
            <button class="mini-btn" @click="mark('in')">标记入点</button>
            <input class="fld" :value="fmtTime(markIn)" readonly placeholder="入点">
            <button class="mini-btn" @click="mark('out')">标记出点</button>
            <input class="fld" :value="fmtTime(markOut)" readonly placeholder="出点">
            <button class="btn btn-primary" :disabled="!canSlice" @click="openAdd">切片</button>
            <span class="muted small">标记起止后切片，片段进入「切片」页待切列表</span>
          </div>
        </div>
      </div>

      <!-- 右：预处理动作卡 -->
      <div class="card card-blk" style="align-self:start">
        <b class="t">预处理</b>
        <div class="row" style="gap:6px;flex-wrap:wrap;margin:8px 0 10px">
          <span class="chip" :class="chipClass('transcribe')">转写 {{ chipText('transcribe') }}</span>
          <span class="chip" :class="chipClass('correct')">LLM 订正 {{ chipText('correct') }}</span>
          <span class="chip" :class="chipClass('detect')">事件检测 {{ chipText('detect') }}</span>
        </div>

        <div class="row" style="gap:8px;margin-bottom:10px">
          <span class="small muted" style="flex:none">ASR 模式</span>
          <select v-model="asrMode" style="flex:1" @change="saveMode">
            <option value="single">single · SenseVoice</option>
            <option value="hybrid">hybrid · SenseVoice 打轴 + qwen3 填词</option>
          </select>
        </div>

        <button class="btn run-btn" :disabled="busy" @click="run('transcribe')">
          <span v-if="runningStage === 'transcribe'" class="spinner"></span>
          {{ runningStage === 'transcribe' ? `转写中 ${jobPct}%` : '开始转写' }}
        </button>
        <button class="btn run-btn" :disabled="busy || !hasTranscript" @click="correct">
          {{ runningStage === 'correct' ? '订正中…' : 'LLM 订正字幕' }}
        </button>
        <button class="btn btn-primary run-btn" :disabled="busy || !hasTranscript" @click="run('detect')">
          <span v-if="runningStage === 'detect'" class="spinner"></span>
          {{ runningStage === 'detect' ? `检测中 ${jobPct}%` : '开始事件检测' }}
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

        <div v-if="!hasTranscript" class="hint mt-8">需要先完成字幕转写才能检测事件</div>

        <button class="btn run-btn mt-12" @click="openAdd">手动添加片段</button>
      </div>
    </div>

    <!-- 手动添加片段（轨道标记的落地表单） -->
    <AppModal :show="showAdd" title="添加片段" @close="showAdd = false">
      <div class="row">
        <div class="flex-1 minw-120"><label>开始（秒）</label><input v-model.number="start" type="number" step="0.1" min="0"></div>
        <div class="flex-1 minw-120"><label>结束（秒）</label><input v-model.number="end" type="number" step="0.1" min="0"></div>
      </div>
      <div class="row mt-8">
        <div class="flex-2 minw-180"><label>标题</label><input v-model="label" placeholder="片段标题"></div>
        <div class="flex-1 minw-120">
          <label>类型</label>
          <select v-model="eventType">
            <option v-for="t in EVENT_TYPES" :key="t" :value="t">{{ TYPE_ZH[t] || t }}</option>
          </select>
        </div>
      </div>
      <label class="mt-8">详情</label>
      <textarea v-model="summary" rows="2"></textarea>
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
import SubtitleTrack from '../components/prepare/SubtitleTrack.vue'
import { projectData } from '../api/projectData'
import { watchJob, kindText, statusClass, statusText } from '../api/jobs'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import { openJobs } from '../stores/ui'
import { EVENT_TYPES, TYPE_ZH, inputMediaUrl } from '../utils'

const route = useRoute()
const router = useRouter()
const pid = computed(() => route.params.pid || '')

const src = ref('')
const inputName = ref('')
const duration = ref(0)
const videoRef = ref(null)
const currentTime = ref(0)
// 当前时间点正在显示的字幕（叠层渲染，效果对齐精修预览）
const activeSub = computed(() => segments.value.find(
  (s) => currentTime.value >= (Number(s.start) || 0) && currentTime.value < (Number(s.end) || 0)
) || null)
const playing = ref(false)
const loadError = ref('')

const segments = ref([])
const asrMode = ref('single')
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

const jobRunning = computed(() => job.status === 'running' || job.status === 'queued')
const jobPct = computed(() => (job.total > 0 ? Math.round((job.current * 100) / job.total) : 0))
const runningStage = computed(() => (jobRunning.value ? jobStage.value : ''))
const jobStage = computed(() => {
  const k = job.kind || ''
  if (k === 'correct') return 'correct'
  if (k === 'stage_detect') return 'detect'
  if (k === 'stage_transcribe' || k === 'pipeline') return 'transcribe'
  return ''
})
const busy = computed(() => jobRunning.value)
const hasTranscript = computed(
  () => segments.value.length > 0 || (status.value?.files || []).some((f) => String(f).startsWith('字幕'))
)
const canSlice = computed(() => markOut.value > markIn.value)
const canAddClip = computed(() => !!(label.value.trim() && summary.value.trim() && end.value > start.value))

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
  if (key === 'transcribe') return hasFile('音频') || hasFile('字幕') ? '✓' : '待开始'
  if (key === 'correct') return '可选'
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
  try {
    const d = await projectData.config(pid.value)
    asrMode.value = d.config?.asr?.mode || 'single'
  } catch {}
  try {
    const d = await projectData.transcript(pid.value)
    segments.value = (d.segments || []).slice().sort((a, b) => (Number(a.start) || 0) - (Number(b.start) || 0))
    duration.value = segments.value.reduce((m, s) => Math.max(m, Number(s.end) || 0), 0)
  } catch {}
}

async function saveMode() {
  try {
    await projectData.patchConfig(pid.value, { asr: { mode: asrMode.value } })
    toast('ASR 模式已保存')
  } catch (e) {
    toast(e.message, 'err')
  }
}

function togglePlay() {
  const v = videoRef.value
  if (!v) return
  if (v.paused) { v.play(); playing.value = true } else { v.pause(); playing.value = false }
}

function seekTo(t) {
  const v = videoRef.value
  if (!v) return
  v.currentTime = Math.max(0, t)
}

function mark(which) {
  const t = Math.round((videoRef.value?.currentTime || currentTime.value || 0) * 10) / 10
  if (which === 'in') markIn.value = t
  else markOut.value = t
  start.value = markIn.value
  end.value = Math.max(markOut.value, markIn.value + 1)
}

function onTrackSave(next, opts = {}) {
  segments.value = next
  if (opts.transient) return
  clearTimeout(saveTimer)
  saveTimer = setTimeout(saveSubs, opts.committed ? 0 : 400)
}

async function saveSubs() {
  try {
    await projectData.saveTranscript(pid.value, segments.value)
    toast('字幕已保存')
  } catch (e) {
    toast(e.message, 'err')
  }
}

async function run(stage) {
  if (busy.value) { toast('已有任务在运行'); return }
  try {
    const r = await projectData.pipeline(pid.value, { stage, resume: 'auto' })
    attachJob(r.id, stage === 'transcribe' ? 'stage_transcribe' : 'stage_detect')
  } catch (e) {
    toast(e.message, 'err')
  }
}

async function correct() {
  const ok = await confirmDialog('将对全部字幕启动 LLM 订正', { title: '启动 LLM 订正？' })
  if (!ok) return
  try {
    const r = await projectData.correct(pid.value)
    attachJob(r.id, 'correct')
  } catch (e) {
    toast(e.message, 'err')
  }
}

function attachJob(id, kind) {
  Object.assign(job, { id, kind, status: 'running', current: 0, total: 0, message: '已启动' })
  jobClose?.()
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
    onDone(s) {
      job.status = s.status
      jobClose = null
      if (s.status === 'succeeded') toast('任务完成')
      else toast('任务失败：' + (s.error || s.message || '未知错误'), 'err')
      load()
    }
  })
  toast('已启动')
}

function openAdd() {
  start.value = markIn.value
  end.value = markOut.value > markIn.value ? markOut.value : markIn.value + 10
  showAdd.value = true
}

async function saveClip() {
  if (!label.value.trim()) { toast('请填写标题'); return }
  if (end.value <= start.value) { toast('结束必须大于开始'); return }
  try {
    const created = await projectData.createEvent(pid.value, {
      label: label.value,
      start_sec: start.value,
      end_sec: end.value,
      event_type: eventType.value,
      summary: summary.value
    })
    const rank = created?.event?.rank
    if (rank) await projectData.cutOne(pid.value, rank)
    showAdd.value = false
    label.value = ''
    summary.value = ''
    toast('片段已生成，正在剪切')
    router.push(`/p/${encodeURIComponent(pid.value)}/slices`)
  } catch (e) {
    toast(e.message, 'err')
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
  jobClose?.()
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
.fld { width:96px; padding:5px 8px; font-size:12px; }
.run-btn { width:100%; justify-content:center; margin-bottom:8px; }
.hint { font-size:12px; color:var(--ink-dim); }
.job-box { padding:10px; border:1px dashed var(--line); border-radius:var(--r-sm); background:var(--bg-soft); }
</style>
