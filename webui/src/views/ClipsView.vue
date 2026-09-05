<template>
  <div>
    <div class="page-head">
      <div><h1>片段</h1><div class="sub">已剪切片段 · 右侧为未剪切事件</div></div>
      <div class="actions">
        <button class="btn" @click="redetect"><Icon name="refresh" :size="14" /> 重新检测</button>
        <button class="btn btn-primary" @click="showAdd = true"><Icon name="plus" :size="14" /> 添加片段</button>
      </div>
    </div>

    <div v-if="loadError" class="error-banner"><Icon name="alert" :size="15" /> {{ loadError }}</div>

    <div class="layout">
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
            <div class="sticker pink">🎬</div>
            <div>
              <h2>{{ eventMap[c.name]?.label || c.meta?.title || c.name }}</h2>
            </div>
          </div>
          <img v-if="hasThumb(c)" :src="thumbUrl(c.dir)" class="thumb" alt="海报" @error="$event.target.style.display='none'">
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
            <button class="mini-btn" :class="{ selected: polishMap[c.dir]?.version !== 'silent' }" @click.stop="selectVersion(c, 'original')">原版</button>
            <button v-if="c.silenced" class="mini-btn" :class="{ selected: polishMap[c.dir]?.version === 'silent' }" @click.stop="selectVersion(c, 'silent')">静音版</button>
            <button v-if="c.video_exists && !c.silenced" class="mini-btn" @click.stop="silence(c)">静音压缩处理</button>
          </div>
        </div>
        <div v-if="!cutClips.length && !loading" class="empty">
          <div class="empty-icon"><Icon name="scissors" :size="24" /></div>
          暂无已剪切片段
        </div>
        <div v-for="i in 3" v-show="loading" :key="'sk' + i" class="skeleton"></div>
      </div>

      <div class="drawer card">
        <h2>未剪切事件</h2>
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
        <button class="btn btn-primary" @click="addClip">保存</button>
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
import { ref, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../api/client'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import Icon from '../components/Icon.vue'
import AppModal from '../components/AppModal.vue'
import { EVENT_TYPES, TYPE_ZH, fmtClock, mediaUrl, thumbUrl } from '../utils'

const route = useRoute()
const router = useRouter()
const cfg = decodeURIComponent(route.params.cfg || '')
const events = ref([])
const clips = ref([])
const polishMap = ref({})
const loading = ref(false)
const loadError = ref('')
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
const types = EVENT_TYPES
const typeZh = TYPE_ZH
const fmt = fmtClock

const newDuration = computed(() => Math.max(0, Math.round((newEnd.value - newStart.value) * 10) / 10))

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

function hasThumb(c) {
  return c && c.video_exists
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
  loading.value = true
  loadError.value = ''
  try {
    const ev = await api('/api/data/events').catch(() => ({ events: [] }))
    const cl = await api('/api/data/clips')
    events.value = (ev.events || []).sort((a, b) => a.start_sec - b.start_sec)
    clips.value = cl || []
  } catch (e) {
    loadError.value = '片段数据加载失败：' + e.message
  } finally {
    loading.value = false
  }
  try {
    const pl = await api('/api/polish/projects').catch(() => [])
    const m = {}
    ;(pl || []).forEach(p => { m[p.dir] = p })
    polishMap.value = m
  } catch {}
}

async function redetect() {
  try {
    await api('/api/pipeline/stage', { method: 'POST', body: JSON.stringify({ stage: 'detect', resume: 'auto' }) })
    toast('重新检测已启动')
  } catch (e) { toast(e.message, 'err') }
}

async function cutEvent(ev) {
  const ok = await confirmDialog(`将剪切「${ev.label}」（${fmt(ev.start_sec)} → ${fmt(ev.end_sec)}）`, { title: '剪切该事件？' })
  if (!ok) return
  try {
    await api('/api/data/events/' + ev.rank + '/cut', { method: 'POST' })
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
    await api('/api/data/events', { method: 'PUT', body: JSON.stringify({ events: events.value }) })
    editing.value = null
    toast('已保存')
    load()
  } catch (e) { toast(e.message, 'err') }
}

async function goPolish(c) {
  try {
    const list = await api('/api/polish/projects').catch(() => [])
    const found = (list || []).find(p => p.dir === c.dir)
    if (!found || !found.polish_ready) {
      const ok = await confirmDialog('是否自动初始化并跳转精修？', { title: '该片段尚未 init' })
      if (!ok) return
      await api('/api/polish/init', { method: 'POST', body: JSON.stringify({ dir: c.dir }) })
      toast('已自动 init')
    }
    router.push(`/project/${encodeURIComponent(cfg)}/polish?dir=${encodeURIComponent(c.dir)}`)
  } catch (e) { toast(e.message, 'err') }
}

async function selectVersion(c, version) {
  try {
    await api('/api/polish/select-version', { method: 'POST', body: JSON.stringify({ dir: c.dir, version }) })
    const pl = await api('/api/polish/projects').catch(() => [])
    const found = (pl || []).find(p => p.dir === c.dir)
    if (found) polishMap.value[c.dir] = found
    toast('已切换为 ' + (version === 'silent' ? '静音版' : '原版'))
  } catch (e) { toast(e.message, 'err') }
}

async function silence(c) {
  try {
    const rank = String(c.name || '').replace('clip-', '')
    await api('/api/data/clips/' + rank + '/silence', { method: 'POST' })
    toast('静音压缩已启动')
    load()
  } catch (e) { toast(e.message, 'err') }
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
  } catch (e) { toast(e.message, 'err') }
}

function focusClip() {
  const name = route.query.focus
  if (!name) return
  const el = document.querySelector(`[data-clipname="${name}"]`)
  if (el) {
    el.scrollIntoView({ behavior: 'smooth', block: 'center' })
    el.style.boxShadow = '0 0 0 4px var(--main-soft)'
    setTimeout(() => { el.style.boxShadow = '' }, 2600)
  }
}

onMounted(() => {
  load()
  focusClip()
})
watch(() => route.query.focus, focusClip)
</script>

<style scoped>
.layout { display:grid; grid-template-columns:minmax(0,1fr) 340px; gap:16px; align-items:start; }
.drawer { position:sticky; top:90px; max-height:calc(100vh - 120px); overflow:auto; }
.uncut-item { padding:10px; border:1px solid var(--line); border-radius:12px; margin-bottom:8px; background:var(--card); }
.thumb { width:100%; aspect-ratio:16/9; object-fit:cover; border-radius:var(--r-sm); margin-top:10px; border:1px solid var(--line); }
</style>
