<template>
  <div>
    <div class="card">
      <h2>预处理</h2>
      <div class="preprocess-row">
        <div class="pre-col">
          <div>
            <label>ASR 模式</label>
            <select v-model="asrMode" @change="saveMode(true)">
              <option value="single">单模型</option>
              <option value="hybrid">双模型</option>
            </select>
          </div>
          <div class="muted small">{{ asrModeHint }}</div>
          <div class="pre-actions">
            <button class="btn btn-primary" @click="run('transcribe')">开始转写</button>
            <button class="btn" @click="correct" :disabled="!hasTranscript">LLM 订正字幕</button>
          </div>
        </div>

        <div class="pre-col">
          <h3>事件检测</h3>
          <button class="btn" :disabled="!canDetect" @click="run('detect')">事件检测</button>
          <div class="muted small">需要先完成字幕转写</div>
        </div>
      </div>
    </div>

    <div v-if="job.id" class="card mt-8">
      <div class="row spread small">
        <b>{{ job.kind }}</b>
        <div class="row">
          <span class="muted small mono">{{ job.message || job.log || '' }}</span>
          <span class="chip" :class="job.status === 'failed' ? 'r' : (job.status === 'succeeded' ? 'g' : 'b')">{{ jobStatusText }}</span>
        </div>
      </div>
      <div v-if="job.total > 0" class="progress mt-6"><div :style="{ width: jobPct + '%' }"></div></div>
    </div>

    <div class="preview-layout">
      <div class="left">
        <div class="card">
          <h2>视频预览</h2>
          <video v-if="src" ref="videoRef" controls playsinline preload="metadata" class="preview-video" :src="src" @timeupdate="currentTime = $event.target.currentTime"></video>
          <div v-else class="empty small">暂无输入视频</div>
        </div>

        <div class="card">
          <h2 class="card-h2"><Icon name="scissors" :size="15" /> 添加片段</h2>
          <div class="row">
            <div class="flex-1 minw-120"><label>开始时间（秒）</label><input v-model.number="start" type="number" step="0.1" min="0"></div>
            <div class="flex-1 minw-120"><label>结束时间（秒）</label><input v-model.number="end" type="number" step="0.1" min="0"></div>
            <button class="mini-btn" @click="takeStart">取当前为开始</button>
            <button class="mini-btn" @click="takeEnd">取当前为结束</button>
          </div>
          <div class="row mt-8">
            <div class="flex-2 minw-180"><label>标题</label><input v-model="label" placeholder="片段标题"></div>
            <div class="flex-1 minw-120"><label>类型</label>
              <select v-model="eventType">
                <option v-for="t in types" :key="t" :value="t">{{ typeZh[t] || t }}</option>
              </select>
            </div>
          </div>
          <label class="mt-8">详情</label>
          <textarea v-model="summary" rows="2"></textarea>
          <div class="row mt-8">
            <button class="btn btn-primary" @click="saveClip" :disabled="!canAddClip">生成片段</button>
            <span class="muted small">保存后自动剪切并出现在片段页</span>
          </div>
        </div>
      </div>

      <div class="right card">
        <div class="row spread"><h2 class="m-0">字幕</h2><span class="muted small">{{ segments.length }} 段</span></div>
        <SubtitleEditor :segments="segments" :current-time="currentTime" @save="saveSubs" @play="playAt" />
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, reactive, computed, onUnmounted, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../api/client'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import Icon from '../components/Icon.vue'
import SubtitleEditor from '../components/SubtitleEditor.vue'
import { EVENT_TYPES, TYPE_ZH, inputMediaUrl } from '../utils'
import { watchJob } from '../polish/api'

const route = useRoute()
const router = useRouter()
const cfg = String(route.params.pid || '')
const src = ref('')
const videoRef = ref(null)
const currentTime = ref(0)
const start = ref(0)
const end = ref(10)
const label = ref('')
const summary = ref('')
const eventType = ref('other')
const types = EVENT_TYPES
const typeZh = TYPE_ZH
const asrMode = ref('single')
const segments = ref([])
const asrModeHint = computed(() => asrMode.value === 'hybrid' ? 'SenseVoice 打轴 + qwen3 填词' : 'SenseVoice')
const hasTranscript = computed(() => segments.value.length > 0)
const canDetect = computed(() => hasTranscript.value)
const canAddClip = computed(() => {
  return !!(label.value.trim() && summary.value.trim() && end.value > start.value)
})

const cfgQ = 'project=' + encodeURIComponent(cfg)
const job = reactive({ id: '', kind: '', status: '', current: 0, total: 0, message: '' })
let jobClose = null
const jobRunning = computed(() => job.status === 'running' || job.status === 'queued')
const jobPct = computed(() => job.total > 0 ? Math.round(job.current * 100 / job.total) : 0)
const jobStatusText = computed(() => ({ queued:'排队中', running:'运行中', succeeded:'完成', failed:'失败', cancelled:'已取消' }[job.status] || job.status))

async function load() {
  try {
    const st = await api(`/api/project-data/status?${cfgQ}`)
    if (st.input_abs) src.value = inputMediaUrl(st.input_abs)
  } catch {}
  try {
    const d = await api(`/api/project-data/config?${cfgQ}`)
    asrMode.value = d.config?.asr?.mode || 'single'
  } catch {}
  try {
    const d = await api(`/api/project-data/transcript?${cfgQ}`)
    segments.value = d.segments || []
    segments.value.sort((a, b) => (Number(a.start) || 0) - (Number(b.start) || 0))
  } catch {}
}

async function saveMode(silent = false) {
  try {
    const d = await api(`/api/project-data/config?${cfgQ}`)
    let toml = d.vtrim_raw || ''
    const modeLine = 'mode = "' + asrMode.value + '"'
    if (!/^\s*\[asr\]\s*$/m.test(toml)) {
      // 没有 [asr] 段：补表头后写入
      const base = toml.replace(/\s*$/, '')
      toml = (base ? base + '\n\n' : '') + '[asr]\n' + modeLine + '\n'
    } else {
      // 已有 [asr] 段：只在该段内替换/插入 mode，绝不追加第二个表头（重复表头会破坏 TOML）
      const lines = toml.split('\n')
      const head = lines.findIndex(l => /^\s*\[asr\]\s*$/.test(l))
      let end = lines.length
      for (let i = head + 1; i < lines.length; i++) {
        if (/^\s*\[/.test(lines[i])) { end = i; break }
      }
      let key = -1
      for (let i = head + 1; i < end; i++) {
        if (/^\s*mode\s*=/.test(lines[i])) { key = i; break }
      }
      if (key >= 0) {
        lines[key] = modeLine
      } else {
        // 插到该段最后一行内容之后（跳过段尾空行），避免写到下一个表头前
        let at = end
        while (at > head + 1 && !lines[at - 1].trim()) at--
        lines.splice(at, 0, modeLine)
      }
      toml = lines.join('\n')
    }
    await api(`/api/project-data/config?${cfgQ}`, { method: 'PUT', body: JSON.stringify({ raw: toml }) })
    if (!silent) toast('模式已保存')
  } catch (e) { toast(e.message, 'err') }
}

async function correct() {
  const ok = await confirmDialog('将对全部字幕启动 LLM 订正', { title: '启动 LLM 订正？' })
  if (!ok) return
  try {
    await api(`/api/project-data/correct?${cfgQ}`, { method: 'POST' })
    toast('LLM 订正已启动')
  } catch (e) { toast(e.message, 'err') }
}

function takeStart() {
  if (videoRef.value) start.value = Math.round((videoRef.value.currentTime || 0) * 10) / 10
}
function takeEnd() {
  if (videoRef.value) end.value = Math.round((videoRef.value.currentTime || 0) * 10) / 10
}

function playAt(seg) {
  if (videoRef.value) {
    videoRef.value.currentTime = seg.start
    videoRef.value.play()
  }
}

async function saveSubs() {
  try {
    await api(`/api/project-data/transcript?${cfgQ}`, { method: 'PUT', body: JSON.stringify({ segments: segments.value, language: 'zh' }) })
    toast('字幕已保存')
  } catch (e) { toast(e.message, 'err') }
}

async function run(stage) {
  if (jobRunning.value) { toast('已有任务在运行'); return }
  try {
    const r = await api(`/api/project-data/pipeline?${cfgQ}`, { method: 'POST', body: JSON.stringify({ stage, resume: 'auto' }) })
    Object.assign(job, { id: r.id, kind: '任务', status: 'running', current: 0, total: 0, message: '已启动 ' + stage })
    jobClose?.()
    jobClose = watchJob(r.id, {
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
      },
      onDone(s) {
        job.status = s.status
        jobClose?.()
        jobClose = null
        if (s.status === 'succeeded') toast(stage + ' 完成')
        else toast(stage + ' 失败：' + (s.error || s.message || '未知错误'), 'err')
        load()
      }
    })
    toast('已启动 ' + stage)
  } catch (e) { toast(e.message, 'err') }
}

async function saveClip() {
  if (!label.value) { toast('请填写标题'); return }
  if (end.value <= start.value) { toast('结束必须大于开始'); return }
  try {
    const created = await api(`/api/project-data/events?${cfgQ}`, { method: 'POST', body: JSON.stringify({
      label: label.value,
      start_sec: start.value,
      end_sec: end.value,
      event_type: eventType.value,
      summary: summary.value
    }) })
    const rank = created && created.event && created.event.rank
    if (rank) await api(`/api/project-data/events/${rank}/cut?${cfgQ}`, { method: 'POST' })
    toast('片段已生成，正在剪切')
    router.push('/polish')
  } catch (e) {
    toast(e.message, 'err')
  }
}

onMounted(load)
onUnmounted(() => {
  jobClose?.()
})
</script>

<style scoped>
.preprocess-row { display:grid; grid-template-columns:1fr 1fr; gap:12px; align-items:stretch; }
.pre-col { display:flex; flex-direction:column; gap:8px; padding:12px; border:1px solid var(--line); border-radius:12px; background:var(--bg-soft); }
.pre-actions { display:flex; flex-direction:column; gap:6px; }
.pre-actions .btn, .pre-col > .btn { width:100%; }
button:disabled { opacity:.45; cursor:not-allowed; }
.preview-layout { display:grid; grid-template-columns:minmax(0,16fr) minmax(0,9fr); gap:14px; align-items:start; margin-top:16px; }
.left { display:flex; flex-direction:column; gap:14px; min-width:0; }
.right { position:sticky; top:90px; max-height:calc(100vh - 120px); overflow:auto; }
.preview-video { width:100%; max-height:72vh; object-fit:contain; background:#000; border-radius:var(--r-sm); display:block; margin:0 auto; }
.card-h2 { display:flex; align-items:center; gap:7px; }
</style>
