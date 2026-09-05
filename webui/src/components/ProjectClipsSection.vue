<template>
  <div>
    <div v-if="loadError" class="error-banner"><Icon name="alert" :size="15" /> {{ loadError }}</div>

    <div class="layout" :class="{ 'drawer-closed': !drawerOpen }">
      <div class="grid cols-3">
        <div
          v-for="c in cutClips"
          :key="c.dir"
          :data-clipname="c.name"
          class="pcard"
          @click="previewClip(c)"
        >
          <span class="ribbon">{{ typeZh[eventMap[c.name]?.event_type] || eventMap[c.name]?.event_type || 'other' }}</span>
          <div class="head">
            <div>
              <h2>{{ eventMap[c.name]?.label || c.meta?.title || c.name }}</h2>
            </div>
          </div>
          <img v-if="hasThumb(c)" :src="thumbSrc(c)" class="thumb" alt="海报" @error="$event.target.style.display='none'">
          <div class="chips mt-8">
            <span class="chip b">{{ Math.round(c.duration_secs) }}s</span>
            <span class="chip n">{{ Math.round(c.size_bytes / 1024 / 1024) }}MB</span>
            <span v-if="c.silenced" class="chip p">已静音压缩</span>
            <span v-if="polishMap[c.dir]?.processed" class="chip g">已精修</span>
            <span v-if="polishMap[c.dir]?.rendered" class="chip b">已渲染</span>
            <span v-if="polishMap[c.dir]?.cover" class="chip p">有封面</span>
          </div>
          <div v-if="eventMap[c.name]" class="row mt-8">
            <span class="muted small mono">{{ fmt(eventMap[c.name].start_sec) }} → {{ fmt(eventMap[c.name].end_sec) }}</span>
            <span class="chip b">{{ Math.round((eventMap[c.name].popularity || 0) * 100) }}%</span>
          </div>
          <div v-if="eventMap[c.name]?.summary" class="muted small mt-8">{{ eventMap[c.name].summary }}</div>
          <div class="foot">
            <button class="mini-btn" @click.stop="editEvent(eventMap[c.name])">编辑</button>
            <button class="mini-btn" @click.stop="goPolish(c)">精修 <Icon name="arrow-right" :size="11" /></button>
          </div>
        </div>
        <div v-if="!cutClips.length && !loading" class="empty">
          <div class="empty-icon"><Icon name="scissors" :size="24" /></div>
          暂无已剪切片段
        </div>
        <div v-for="i in 3" v-show="loading" :key="'sk' + i" class="skeleton"></div>
      </div>

      <div class="drawer card" :class="{ collapsed: !drawerOpen }">
        <template v-if="drawerOpen">
          <div class="drawer-head">
            <h2>未剪切事件</h2>
            <button class="mini-btn" title="收起" @click="drawerOpen = false">
              <Icon name="arrow-right" :size="13" style="transform:rotate(180deg)" />
            </button>
          </div>
          <div v-if="!uncutEvents.length" class="empty small">暂无未剪切事件</div>
          <div v-for="ev in uncutEvents" :key="ev.rank" class="uncut-item">
            <div class="row spread">
              <b>{{ ev.label }}</b>
              <span class="chip p">{{ typeZh[ev.event_type] || ev.event_type }}</span>
            </div>
            <div class="muted small mono mt-4">{{ fmt(ev.start_sec) }} → {{ fmt(ev.end_sec) }}</div>
            <div v-if="ev.summary" class="muted small mt-4">{{ ev.summary }}</div>
            <div class="row mt-6">
              <button class="btn btn-primary" @click="cutEvent(ev)">剪切</button>
              <button class="mini-btn" @click="editEvent(ev)">编辑</button>
            </div>
          </div>
        </template>
        <button v-else class="drawer-expand" @click="drawerOpen = true">
          未剪切事件{{ uncutEvents.length ? ' · ' + uncutEvents.length : '' }}
        </button>
      </div>
    </div>

    <AppModal :show="showAdd" title="添加片段" @close="showAdd = false">
      <label>标题</label>
      <input v-model="newLabel" placeholder="片段标题">
      <div class="row mt-8">
        <div class="flex-1"><label>开始时间（秒）</label><input v-model.number="newStart" type="number" step="0.1" min="0"></div>
        <div class="flex-1"><label>结束时间（秒）</label><input v-model.number="newEnd" type="number" step="0.1" min="0"></div>
        <div class="flex-1"><label>时长（秒）</label><input :value="newDuration" type="number" readonly></div>
      </div>
      <label class="mt-8">类型</label>
      <select v-model="newType">
        <option v-for="t in types" :key="t" :value="t">{{ typeZh[t] || t }}</option>
      </select>
      <label class="mt-8">详情</label>
      <textarea v-model="newSummary" rows="3"></textarea>
      <template #footer>
        <button class="btn btn-primary" @click="addClip" :disabled="!canAddClip">保存</button>
        <button class="btn" @click="showAdd = false">取消</button>
      </template>
    </AppModal>

    <AppModal :show="!!editing" title="编辑事件" @close="editing = null">
      <label>标题</label>
      <input v-model="editLabel" placeholder="事件标题">
      <div class="row mt-8">
        <div class="flex-1"><label>开始时间（秒）</label><input v-model.number="editStart" type="number" step="0.1" min="0"></div>
        <div class="flex-1"><label>结束时间（秒）</label><input v-model.number="editEnd" type="number" step="0.1" min="0"></div>
      </div>
      <label class="mt-8">类型</label>
      <select v-model="editType">
        <option v-for="t in types" :key="t" :value="t">{{ typeZh[t] || t }}</option>
      </select>
      <label class="mt-8">详情</label>
      <textarea v-model="editSummary" rows="3"></textarea>
      <template #footer>
        <button class="btn btn-primary" @click="saveEdit">保存</button>
        <button class="btn" @click="editing = null">取消</button>
      </template>
    </AppModal>

    <AppModal :show="!!playing" wide :title="playing?.name || ''" @close="playing = null">
      <video controls autoplay class="video-lg" :src="playing?.url"></video>
    </AppModal>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { useRouter } from 'vue-router'
import { api } from '../api/client'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import Icon from './Icon.vue'
import AppModal from './AppModal.vue'
import { EVENT_TYPES, TYPE_ZH, fmtClock, mediaUrl } from '../utils'

const props = defineProps({
  project: { type: Object, required: true },
  projectData: { type: Object, default: null }
})
const router = useRouter()
const projectQ = () => encodeURIComponent(props.project.config_file)

const events = ref([])
const clips = ref([])
const polishMap = ref({})
const loading = ref(false)
const loadError = ref('')
const showAdd = ref(false)
const drawerOpen = ref(true)
const playing = ref(null)
const newLabel = ref('')
const newStart = ref(0)
const newEnd = ref(10)
const newType = ref('other')
const newSummary = ref('')
const editing = ref(null)
const editLabel = ref('')
const editStart = ref(0)
const editEnd = ref(10)
const editType = ref('other')
const editSummary = ref('')
const types = EVENT_TYPES
const typeZh = TYPE_ZH
const fmt = fmtClock

const newDuration = computed(() => Math.max(0, Math.round((newEnd.value - newStart.value) * 10) / 10))
const canAddClip = computed(() => {
  return !!(newLabel.value.trim() && newSummary.value.trim() && newEnd.value > newStart.value)
})

const eventMap = computed(() => {
  const m = {}
  events.value.forEach(e => { m['clip-' + String(e.rank).padStart(2, '0')] = e })
  return m
})
const cutClips = computed(() => clips.value.filter(c => c.video_exists !== false))
const uncutEvents = computed(() => events.value.filter(ev => {
  const c = clips.value.find(x => x.name === 'clip-' + String(ev.rank).padStart(2, '0'))
  return !c || !c.video_exists
}))

function hasThumb(c) {
  return c && c.video_exists
}

function thumbSrc(c) {
  const rank = String(c.name || '').replace('clip-', '')
  return `/api/project-data/clips/${encodeURIComponent(rank)}/thumb?config=${projectQ()}`
}

function secToTs(sec) {
  sec = Math.max(0, Number(sec) || 0)
  const h = Math.floor(sec / 3600)
  const m = Math.floor((sec % 3600) / 60)
  const s = Math.floor(sec % 60)
  const ms = Math.round((sec - Math.floor(sec)) * 1000)
  return { h, m, s, ms }
}

function previewClip(c) {
  playing.value = { name: c.name, url: mediaUrl(c.dir) }
}

async function load() {
  if (props.projectData) {
    events.value = (props.projectData.events || []).slice().sort((a, b) => a.start_sec - b.start_sec)
    clips.value = props.projectData.clips || []
    polishMap.value = props.projectData.polishMap || {}
    drawerOpen.value = uncutEvents.value.length > 0
    return
  }
  loading.value = true
  loadError.value = ''
  try {
    const q = projectQ()
    const ev = await api(`/api/project-data/events?config=${q}`).catch(() => ({ events: [] }))
    const cl = await api(`/api/project-data/clips?config=${q}`)
    const pl = await api(`/api/project-data/polish?config=${q}`).catch(() => [])
    events.value = (ev.events || []).sort((a, b) => a.start_sec - b.start_sec)
    clips.value = cl || []
    const m = {}
    ;(pl || []).forEach(p => { m[p.dir] = p })
    polishMap.value = m
    drawerOpen.value = uncutEvents.value.length > 0
  } catch (e) {
    loadError.value = '片段数据加载失败：' + e.message
  } finally {
    loading.value = false
  }
}

async function run(kind, c) {
  if (kind === 'init') {
    const ok = await confirmDialog('将解析字幕/弹幕并创建 polish.toml', { title: '初始化该项目？' })
    if (!ok) return
  }
  if (kind === 'llm') {
    const ok = await confirmDialog('执行 LLM 生成后会自动 process 并刷新预览', { title: '执行 LLM 生成？' })
    if (!ok) return
  }
  try {
    await api('/api/polish/' + kind, { method: 'POST', body: JSON.stringify({ dir: c.dir }) })
    toast('已启动 ' + kind)
    if (kind === 'llm') {
      setTimeout(async () => {
        try {
          await api('/api/polish/process', { method: 'POST', body: JSON.stringify({ dir: c.dir }) })
          toast('LLM 完成，process 已执行')
          load()
        } catch (e) { toast('后处理失败: ' + e.message, 'err') }
      }, 1500)
    }
    load()
  } catch (e) { toast(e.message, 'err') }
}

async function render(c) {
  const out = await promptDialog('导出渲染视频的文件路径（可含文件夹，留空使用项目配置 output）', {
    title: '渲染输出',
    value: c.dir + '/export.mp4'
  })
  if (out === null) return
  try {
    await api('/api/polish/render', { method: 'POST', body: JSON.stringify({ dir: c.dir, output_file: out }) })
    toast('渲染已启动')
  } catch (e) { toast(e.message, 'err') }
}

async function preview(c) {
  try {
    const r = await api('/api/polish/preview', { method: 'POST', body: JSON.stringify({ dir: c.dir }) })
    const url = r.video_url || r.cover_url
    if (url) window.open(url, '_blank')
    else toast('预览返回空')
  } catch (e) { toast(e.message, 'err') }
}

async function redetect() {
  try {
    await api(`/api/project-data/pipeline?config=${projectQ()}`, { method: 'POST', body: JSON.stringify({ config: props.project.config_file, stage: 'detect', resume: 'auto' }) })
    toast('重新检测已启动')
  } catch (e) { toast(e.message, 'err') }
}

async function cutEvent(ev) {
  const ok = await confirmDialog(`将剪切「${ev.label}」（${fmt(ev.start_sec)} → ${fmt(ev.end_sec)}）`, { title: '剪切该事件？' })
  if (!ok) return
  try {
    await api(`/api/project-data/events/${ev.rank}/cut?config=${projectQ()}`, { method: 'POST' })
    toast('剪切任务已启动')
    load()
  } catch (e) { toast(e.message, 'err') }
}

function editEvent(ev) {
  if (!ev) return
  editing.value = ev
  editLabel.value = ev.label || ''
  editStart.value = ev.start_sec || 0
  editEnd.value = ev.end_sec || 0
  editType.value = ev.event_type || 'other'
  editSummary.value = ev.summary || ''
}

async function saveEdit() {
  if (!editing.value) return
  if (editEnd.value <= editStart.value) { toast('结束必须大于开始'); return }
  const ev = editing.value
  ev.label = editLabel.value
  ev.event_type = editType.value
  ev.summary = editSummary.value
  ev.start_sec = editStart.value
  ev.end_sec = editEnd.value
  ev.start = secToTs(editStart.value)
  ev.end = secToTs(editEnd.value)
  try {
    await api(`/api/project-data/events?config=${projectQ()}`, { method: 'PUT', body: JSON.stringify({ events: events.value }) })
    editing.value = null
    toast('已保存')
    load()
  } catch (e) { toast(e.message, 'err') }
}

async function goPolish(c) {
  try {
    const list = await api(`/api/project-data/polish?config=${projectQ()}`).catch(() => [])
    const found = (list || []).find(p => p.dir === c.dir)
    if (!found || !found.polish_ready) {
      const ok = await confirmDialog('是否自动初始化并跳转精修？', { title: '该片段尚未 init' })
      if (!ok) return
      await api('/api/polish/init', { method: 'POST', body: JSON.stringify({ dir: c.dir }) })
      toast('已自动 init')
    }
    router.push(`/project/${encodeURIComponent(props.project.config_file)}/slices/${encodeURIComponent(c.name)}`)
  } catch (e) { toast(e.message, 'err') }
}

async function selectVersion(c, version) {
  try {
    await api('/api/polish/select-version', { method: 'POST', body: JSON.stringify({ dir: c.dir, version }) })
    const pl = await api(`/api/project-data/polish?config=${projectQ()}`).catch(() => [])
    const found = (pl || []).find(p => p.dir === c.dir)
    if (found) polishMap.value[c.dir] = found
    toast('已切换为 ' + (version === 'silent' ? '静音版' : '原版'))
  } catch (e) { toast(e.message, 'err') }
}

async function silence(c) {
  try {
    const rank = String(c.name || '').replace('clip-', '')
    await api(`/api/project-data/clips/${rank}/silence?config=${projectQ()}`, { method: 'POST' })
    toast('静音压缩已启动')
    load()
  } catch (e) { toast(e.message, 'err') }
}

async function addClip() {
  if (!newLabel.value) { toast('请填写标题'); return }
  if (newEnd.value <= newStart.value) { toast('结束必须大于开始'); return }
  try {
    const created = await api(`/api/project-data/events?config=${projectQ()}`, { method: 'POST', body: JSON.stringify({
      label: newLabel.value,
      start_sec: newStart.value,
      end_sec: newEnd.value,
      event_type: newType.value,
      summary: newSummary.value
    }) })
    const rank = created && created.event && created.event.rank
    if (rank) await api(`/api/project-data/events/${rank}/cut?config=${projectQ()}`, { method: 'POST' })
    showAdd.value = false
    toast('片段已创建，正在剪切')
    load()
  } catch (e) { toast(e.message, 'err') }
}

function openAdd() {
  showAdd.value = true
}

defineExpose({ redetect, openAdd })

watch(() => props.projectData, load, { immediate: true })
watch(() => props.project.config_file, load)
</script>

<style scoped>
.layout { display:grid; grid-template-columns:minmax(0,1fr) 340px; gap:16px; align-items:start; }
.layout.drawer-closed { grid-template-columns:minmax(0,1fr) 40px; }
/* 抽屉展开时压缩卡片最小宽，避免半窗下只剩一张巨卡 */
.layout .cols-3 { grid-template-columns:repeat(auto-fill,minmax(210px,1fr)); }
.drawer { position:sticky; top:8px; max-height:calc(70vh - 20px); overflow:auto; }
.drawer.collapsed { padding:8px 3px; display:flex; flex-direction:column; align-items:center; gap:10px; min-height:140px; }
.drawer-head { display:flex; align-items:center; justify-content:space-between; margin-bottom:8px; }
.drawer-head h2 { margin:0; }
.drawer-expand { writing-mode:vertical-rl; letter-spacing:2px; border:1.5px dashed var(--line); border-radius:var(--r-full); background:var(--bg-soft); color:var(--ink-dim); cursor:pointer; font-size:12px; padding:12px 5px; transition:all .15s; }
.drawer-expand:hover { color:var(--main-deep); border-color:var(--main); }
.uncut-item { padding:10px; border:1px solid var(--line); border-radius:12px; margin-bottom:8px; background:var(--card); }
.thumb { width:100%; aspect-ratio:16/9; object-fit:cover; border-radius:var(--r-sm); margin-top:10px; border:1px solid var(--line); }
.pcard .ribbon { opacity:1; top:8px; right:-18px; padding:2px 30px; background:rgba(253,227,236,.5); color:rgba(216,102,155,.8); }

/* 窄屏：抽屉堆叠到片段下方，横向折叠条 */
@media (max-width: 900px) {
  .layout, .layout.drawer-closed { grid-template-columns:1fr; }
  .drawer { position:static; max-height:none; }
  .drawer.collapsed { flex-direction:row; min-height:auto; padding:6px; }
  .drawer-expand { writing-mode:horizontal-tb; letter-spacing:0; padding:4px 14px; }
}
</style>
