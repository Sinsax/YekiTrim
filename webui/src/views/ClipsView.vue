<template>
  <div>
    <div class="page-head">
      <div><h1>片段</h1><div class="sub">已剪切片段 · 右侧抽屉为未剪切事件</div></div>
      <div class="actions"><button class="btn" @click="redetect">重新检测</button><button class="btn btn-primary" @click="showAdd = true">＋ 添加片段</button></div>
    </div>
    <div class="layout">
      <div class="grid cols-3">
        <div v-for="c in cutClips" :key="c.dir" class="pcard" @click="previewClip(c)">
          <span class="ribbon">{{ typeZh[eventMap[c.name]?.event_type] || eventMap[c.name]?.event_type || 'other' }}</span>
          <div class="head">
            <div class="sticker pink">🎬</div>
            <div>
              <h2>{{ eventMap[c.name]?.label || c.meta?.title || c.name }}</h2>
            </div>
          </div>
          <div class="chips" style="margin-top:8px">
            <span class="chip b">{{ Math.round(c.duration_secs) }}s</span>
            <span class="chip n">{{ Math.round(c.size_bytes / 1024 / 1024) }}MB</span>
            <span v-if="c.silenced" class="chip p">已静音压缩</span>
          </div>
          <div v-if="eventMap[c.name]" class="row" style="margin-top:8px">
            <span class="muted small mono">{{ fmt(eventMap[c.name].start_sec) }} → {{ fmt(eventMap[c.name].end_sec) }}</span>
          </div>
          <div v-if="eventMap[c.name]?.summary" class="muted small" style="margin-top:8px">{{ eventMap[c.name].summary }}</div>
          <div class="foot">
            <button class="mini-btn" @click.stop="editEvent(eventMap[c.name])">编辑</button>
            <button class="mini-btn" @click.stop="goPolish(c)">精修 ▸</button>
            <button class="mini-btn" :class="{ selected: !c.silent_selected }" @click.stop="selectVersion(c, 'original')">原版</button>
            <button v-if="c.silenced" class="mini-btn" :class="{ selected: c.silent_selected }" @click.stop="selectVersion(c, 'silent')">静音版</button>
            <button v-if="c.video_exists && !c.silenced" class="mini-btn" @click.stop="silence(c)">静音压缩处理</button>
          </div>
        </div>
        <div v-if="!cutClips.length" class="card muted">暂无已剪切片段</div>
      </div>
      <div class="drawer card">
        <h2>未剪切事件</h2>
        <div v-if="!uncutEvents.length" class="muted">暂无未剪切事件</div>
        <div v-for="ev in uncutEvents" :key="ev.rank" class="uncut-item">
          <div class="row spread">
            <b>{{ ev.label }}</b>
            <span class="chip p">{{ typeZh[ev.event_type] || ev.event_type }}</span>
          </div>
          <div class="muted small mono" style="margin-top:4px">{{ fmt(ev.start_sec) }} → {{ fmt(ev.end_sec) }}</div>
          <div class="row" style="margin-top:6px">
            <button class="btn btn-primary" @click="cutEvent(ev)">剪切</button>
            <button class="mini-btn" @click="editEvent(ev)">编辑</button>
          </div>
        </div>
      </div>
    </div>

    <div v-if="showAdd" class="modal-backdrop" @click.self="showAdd = false">
      <div class="modal">
        <h2>添加片段</h2>
        <label>标题</label><input v-model="newLabel" placeholder="片段标题">
        <div class="row">
          <div style="flex:1"><label>开始时间（秒）</label><input v-model.number="newStart" type="number" step="0.1" min="0"></div>
          <div style="flex:1"><label>结束时间（秒）</label><input v-model.number="newEnd" type="number" step="0.1" min="0"></div>
        </div>
        <label>类型</label>
        <select v-model="newType">
          <option v-for="t in types" :key="t" :value="t">{{ typeZh[t] || t }}</option>
        </select>
        <label>详情</label><textarea v-model="newSummary" rows="3"></textarea>
        <div class="row" style="margin-top:12px">
          <button class="btn btn-primary" @click="addClip">保存</button>
          <button class="btn" @click="showAdd = false">取消</button>
        </div>
      </div>
    </div>

    <div v-if="editing" class="modal-backdrop" @click.self="editing = null">
      <div class="modal">
        <h2>编辑事件</h2>
        <label>标题</label><input v-model="editLabel" placeholder="事件标题">
        <div class="row">
          <div style="flex:1"><label>开始时间（秒）</label><input v-model.number="editStart" type="number" step="0.1" min="0"></div>
          <div style="flex:1"><label>结束时间（秒）</label><input v-model.number="editEnd" type="number" step="0.1" min="0"></div>
        </div>
        <label>类型</label>
        <select v-model="editType">
          <option v-for="t in types" :key="t" :value="t">{{ typeZh[t] || t }}</option>
        </select>
        <label>详情</label><textarea v-model="editSummary" rows="3"></textarea>
        <div class="row" style="margin-top:12px">
          <button class="btn btn-primary" @click="saveEdit">保存</button>
          <button class="btn" @click="editing = null">取消</button>
        </div>
      </div>
    </div>

    <div v-if="playing" class="modal-backdrop" @click.self="playing = null">
      <div class="modal" style="width:min(94vw,900px)">
        <div class="row spread"><b>{{ playing?.name }}</b><button class="btn" @click="playing = null">关闭</button></div>
        <video controls autoplay style="width:100%;max-height:70vh;background:#000;border-radius:12px" :src="playing?.url"></video>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../api/client'
import { toast } from '../stores/toast'

const route = useRoute()
const router = useRouter()
const cfg = decodeURIComponent(route.params.cfg || '')
const events = ref([])
const clips = ref([])
const showAdd = ref(false)
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
const types = ['gameplay','interaction','qna','reaction','storytelling','music','food','rant','break','outro','chat','eating','irl','ranking','discussion','intro','other']
const typeZh = { gameplay:'游戏', interaction:'互动', qna:'问答', reaction:'反应', storytelling:'故事', music:'音乐', food:'美食', rant:'吐槽', break:'休息', outro:'结尾', chat:'聊天', eating:'吃东西', irl:'日常', ranking:'排行', discussion:'讨论', intro:'开场', other:'其他' }

const eventMap = computed(() => {
  const m = {}
  events.value.forEach(e => { m['clip-' + String(e.rank).padStart(2, '0')] = e })
  return m
})
const cutClips = computed(() => clips.value.filter(c => c.video_exists))
const uncutEvents = computed(() => events.value.filter(ev => {
  const c = clips.value.find(x => x.name === 'clip-' + String(ev.rank).padStart(2, '0'))
  return !c || !c.video_exists
}))

function fmt(sec) {
  sec = Number(sec) || 0
  const h = Math.floor(sec / 3600), m = Math.floor(sec % 3600 / 60), s = Math.floor(sec % 60)
  return (h ? h + ':' + String(m).padStart(2, '0') : m) + ':' + String(s).padStart(2, '0')
}

function mediaUrl(dir, file = 'clip.mp4') {
  const sep = String.fromCharCode(92)
  const norm = String(dir).split(sep).join('/')
  return '/api/media/' + encodeURIComponent(norm + '/' + file).replace(/%2F/g, '/')
}

function previewClip(c) {
  playing.value = { name: c.name, url: mediaUrl(c.dir) }
}

async function redetect() {
  try {
    await api('/api/pipeline/stage', { method: 'POST', body: JSON.stringify({ stage: 'detect', resume: 'auto' }) })
    toast('重新检测已启动')
  } catch (e) { toast(e.message, 'err') }
}

async function load() {
  try {
    const ev = await api('/api/data/events').catch(() => ({ events: [] }))
    const cl = await api('/api/data/clips').catch(() => [])
    events.value = (ev.events || []).sort((a, b) => a.start_sec - b.start_sec)
    clips.value = cl || []
  } catch (e) {}
}

async function cutEvent(ev) {
  if (!confirm('剪切该事件？')) return
  try {
    await api('/api/data/events/' + ev.rank + '/cut', { method: 'POST' })
    toast('剪切任务已启动')
    load()
  } catch (e) { toast(e.message) }
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
  ev.start_sec = editStart.value
  ev.end_sec = editEnd.value
  ev.event_type = editType.value
  ev.summary = editSummary.value
  try {
    await api('/api/data/events', { method: 'PUT', body: JSON.stringify({ events: events.value }) })
    editing.value = null
    toast('已保存')
    load()
  } catch (e) { toast(e.message) }
}

function goPolish(c) {
  router.push(`/project/${encodeURIComponent(cfg)}/polish?dir=${encodeURIComponent(c.dir)}`)
}

async function selectVersion(c, version) {
  try {
    await api('/api/polish/select-version', { method: 'POST', body: JSON.stringify({ dir: c.dir, version }) })
    c.silent_selected = version === 'silent'
    toast('已切换为 ' + (version === 'silent' ? '静音版' : '原版'))
  } catch (e) { toast(e.message) }
}

async function silence(c) {
  try {
    const rank = String(c.name || '').replace('clip-', '')
    await api('/api/data/clips/' + rank + '/silence', { method: 'POST' })
    toast('静音压缩已启动')
    load()
  } catch (e) { toast(e.message) }
}

async function addClip() {
  if (!newLabel.value) { toast('请填写标题'); return }
  if (newEnd.value <= newStart.value) { toast('结束必须大于开始'); return }
  try {
    const created = await api('/api/data/events', { method: 'POST', body: JSON.stringify({
      label: newLabel.value,
      start_sec: newStart.value,
      end_sec: newEnd.value,
      event_type: newType.value,
      summary: newSummary.value
    }) })
    const rank = created && created.event && created.event.rank
    if (rank) await api('/api/data/events/' + rank + '/cut', { method: 'POST' })
    showAdd.value = false
    toast('片段已创建，正在剪切')
    load()
  } catch (e) { toast(e.message) }
}

onMounted(load)
</script>

<style scoped>
.layout { display:grid; grid-template-columns:minmax(0,1fr) 340px; gap:16px; align-items:start; }
.drawer { position:sticky; top:90px; max-height:calc(100vh - 120px); overflow:auto; }
.uncut-item { padding:10px; border:1px solid var(--line); border-radius:12px; margin-bottom:8px; background:var(--card); }
.pcard h2 { font-size:15px; margin:0; word-break:break-word; }
</style>
