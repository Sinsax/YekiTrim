<template>
  <div>
    <div v-if="loadError" class="error-banner"><Icon name="alert" :size="15" /> {{ loadError }}</div>

    <div class="sort-bar">
      <span class="muted small">排序</span>
      <button
        v-for="o in SORT_OPTIONS"
        :key="o.key"
        class="mini-btn"
        :class="{ selected: sortKey === o.key }"
        :title="sortKey === o.key ? '再次点击切换升/降序' : o.hint"
        @click="pickSort(o)"
      >
        {{ o.label }}
        <Icon v-if="sortKey === o.key" :name="sortDir === 'asc' ? 'arrow-up' : 'arrow-down'" :size="11" />
      </button>
      <span class="v-sep"></span>
      <span class="muted small">筛选</span>
      <button
        v-for="f in CLIP_FILTERS"
        :key="f.key"
        class="mini-btn"
        :class="{ selected: filterKey === f.key }"
        :title="f.hint"
        @click="filterKey = f.key"
      >{{ f.label }}</button>
      <span class="muted small sort-count">
        {{ shownClips.length }}<template v-if="filterKey !== 'all'">/{{ cutClips.length }}</template> 个片段<template v-if="uncutEvents.length"> · 未剪切 {{ uncutEvents.length }}</template>
      </span>
    </div>

    <div class="layout" :class="{ 'drawer-closed': !drawerOpen, 'no-drawer': !uncutEvents.length }">
      <div class="grid cols-3">
        <div
          v-for="c in shownClips"
          :key="c.dir"
          :data-clipname="c.name"
          class="pcard clip-card"
        >
          <span class="ribbon">{{ typeZh[eventMap[c.name]?.event_type] || eventMap[c.name]?.event_type || 'other' }}</span>
          <div class="head">
            <div>
              <h2>{{ eventMap[c.name]?.label || c.meta?.title || c.name }}</h2>
            </div>
          </div>
          <!-- 只有点预览图才弹出视频预览，卡片其他区域不响应点击 -->
          <img
            v-if="hasThumb(c)"
            :src="thumbSrc(c)"
            class="thumb"
            alt="海报"
            title="点击预览片段"
            @click="previewClip(c)"
            @error="$event.target.style.display='none'"
          >
          <div class="chips mt-8">
            <span class="chip b">{{ Math.round(c.duration_secs) }}s</span>
            <span class="chip n">{{ Math.round(c.size_bytes / 1024 / 1024) }}MB</span>
            <span v-if="Number(c.danmaku_count)" class="chip a">弹幕 {{ c.danmaku_count }}</span>
            <span v-if="Number(c.subtitle_count)" class="chip n">字幕 {{ c.subtitle_count }}</span>
            <span v-if="c.silenced" class="chip r">已去静音 · 屏蔽 {{ c.mute_range_count || 0 }} 段</span>
            <span v-else-if="Number(c.mute_range_count)" class="chip n">屏蔽 {{ c.mute_range_count }} 段</span>
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

      <div v-if="uncutEvents.length" class="drawer card" :class="{ collapsed: !drawerOpen }">
        <template v-if="drawerOpen">
          <div class="drawer-head">
            <h2>未剪切事件</h2>
            <div class="row">
              <button class="mini-btn" :disabled="!uncutEvents.length" @click="cutAll">全切</button>
              <button class="mini-btn" :disabled="!uncutEvents.length" @click="maximized = true">放大</button>
              <button class="mini-btn" title="收起" @click="setDrawer(false)">
                <Icon name="arrow-right" :size="13" style="transform:rotate(180deg)" />
              </button>
            </div>
          </div>
          <div v-if="!uncutEvents.length" class="empty small">暂无未剪切事件</div>
          <div v-for="ev in uncutEvents" :key="ev.rank" class="uncut-item">
            <div class="row spread">
              <b>{{ ev.label }}</b>
              <span class="chip p">{{ typeZh[ev.event_type] || ev.event_type }}</span>
            </div>
            <div class="muted small mono mt-4">{{ fmt(ev.start_sec) }} → {{ fmt(ev.end_sec) }}</div>
            <div class="chips mt-4">
              <span class="chip a">弹幕 {{ ev.danmaku_count ?? 0 }}</span>
              <span class="chip n">字幕 {{ ev.subtitle_count ?? 0 }}</span>
            </div>
            <div v-if="ev.summary" class="muted small mt-4">{{ ev.summary }}</div>
            <div class="row mt-6">
              <button class="btn btn-primary" @click="cutEvent(ev)">剪切</button>
              <button class="mini-btn" @click="editEvent(ev)">编辑</button>
            </div>
          </div>
        </template>
        <button v-else class="drawer-expand" @click="setDrawer(true)">
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

    <AppModal :show="maximized" wide :title="'未剪切事件 · ' + uncutEvents.length" @close="maximized = false">
      <div v-if="!uncutEvents.length" class="empty small">暂无未剪切事件</div>
      <div v-else class="max-grid">
        <div v-for="ev in uncutEvents" :key="'max' + ev.rank" class="uncut-item max-item">
          <div class="row spread max-title-row">
            <b>{{ ev.label }}</b>
            <span class="chip p">{{ typeZh[ev.event_type] || ev.event_type }}</span>
          </div>
          <div class="muted small mono mt-4">{{ fmt(ev.start_sec) }} → {{ fmt(ev.end_sec) }}</div>
          <div class="chips mt-4">
            <span class="chip a">弹幕 {{ ev.danmaku_count ?? 0 }}</span>
            <span class="chip n">字幕 {{ ev.subtitle_count ?? 0 }}</span>
          </div>
          <div v-if="ev.summary" class="muted small mt-4 max-summary">{{ ev.summary }}</div>
          <div class="row mt-6">
            <button class="btn btn-primary" @click="cutEvent(ev)">剪切</button>
            <button class="mini-btn" @click="editEvent(ev); maximized = false">编辑</button>
          </div>
        </div>
      </div>
      <template #footer>
        <button class="btn btn-primary" :disabled="!uncutEvents.length" @click="cutAll">一键全切</button>
        <button class="btn" @click="maximized = false">关闭</button>
      </template>
    </AppModal>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { useRouter } from 'vue-router'
import { api } from '../api/client'
import { projectData } from '../api/projectData'
import { initPolish, polishPost, runRender } from '../api/polish'
import { watchJob } from '../api/jobs'
import { toast } from '../stores/toast'
import { confirmDialog, promptDialog } from '../stores/dialog'
import Icon from './Icon.vue'
import AppModal from './AppModal.vue'
import { EVENT_TYPES, TYPE_ZH, fmtClock, mediaUrl } from '../utils'

const props = defineProps({
  project: { type: Object, required: true },
  projectData: { type: Object, default: null }
})
const router = useRouter()

const events = ref([])
const clips = ref([])
const polishMap = ref({})
const loading = ref(false)
const loadError = ref('')
const showAdd = ref(false)
// 用户手动收起/展开后，数据刷新（任务完成、缓存失效）不得再把抽屉弹回去。
// 偏好放 localStorage：组件会被 loading 骨架卸载重建，刷新页面也不丢。
const DRAWER_STORE = 'vtrim.slices.drawer'
let drawerUserPref = null   // null = 未表态，跟随「有无未剪切事件」
try { drawerUserPref = localStorage.getItem(DRAWER_STORE) } catch {}
const drawerOpen = ref(drawerUserPref != null ? drawerUserPref === 'open' : true)
function setDrawer(open) {
  drawerUserPref = open ? 'open' : 'closed'
  try { localStorage.setItem(DRAWER_STORE, drawerUserPref) } catch {}
  drawerOpen.value = open
}
const maximized = ref(false)
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

// ── 排序切换：热度 rank / 开始时间 / 弹幕量 / 字幕量 ──────────────
const SORT_OPTIONS = [
  { key: 'rank', label: '热度', dir: 'asc', hint: '按 rank（热度）排序，1 最热' },
  { key: 'start', label: '开始时间', dir: 'asc', hint: '按原片开始时间排序' },
  { key: 'danmaku', label: '弹幕量', dir: 'desc', hint: '按片段弹幕条数排序' },
  { key: 'subtitle', label: '字幕量', dir: 'desc', hint: '按片段字幕条数排序' }
]
const SORT_STORE = 'vtrim.slices.sort'

function readSort() {
  try {
    const raw = JSON.parse(localStorage.getItem(SORT_STORE) || '{}')
    const opt = SORT_OPTIONS.find(o => o.key === raw.key)
    if (opt) return { key: opt.key, dir: raw.dir === 'asc' || raw.dir === 'desc' ? raw.dir : opt.dir }
  } catch {}
  return { key: 'rank', dir: 'asc' }
}

const initialSort = readSort()
const sortKey = ref(initialSort.key)
const sortDir = ref(initialSort.dir)

function pickSort(opt) {
  if (sortKey.value === opt.key) sortDir.value = sortDir.value === 'asc' ? 'desc' : 'asc'
  else { sortKey.value = opt.key; sortDir.value = opt.dir }
}

watch([sortKey, sortDir], () => {
  try { localStorage.setItem(SORT_STORE, JSON.stringify({ key: sortKey.value, dir: sortDir.value })) } catch {}
})

function rankOf(name) {
  return Number(String(name || '').replace(/^clip-/, '')) || 0
}

function clipStats(c) {
  const ev = eventMap.value[c.name]
  return {
    rank: rankOf(c.name) || Number(c.meta?.rank) || 0,
    start: ev ? Number(ev.start_sec) || 0 : Number(c.meta?.original_start) || 0,
    danmaku: Number(c.danmaku_count) || 0,
    subtitle: Number(c.subtitle_count) || 0
  }
}

function eventStats(ev) {
  return {
    rank: Number(ev.rank) || 0,
    start: Number(ev.start_sec) || 0,
    danmaku: Number(ev.danmaku_count) || 0,
    subtitle: Number(ev.subtitle_count) || 0
  }
}

function makeCmp(statsOf) {
  return (a, b) => {
    const sa = statsOf(a)
    const sb = statsOf(b)
    const va = sa[sortKey.value]
    const vb = sb[sortKey.value]
    if (va !== vb) return (va - vb) * (sortDir.value === 'asc' ? 1 : -1)
    // 排序值相同时：已精修优先，再按 rank
    const pa = polishMap.value[a.dir]?.polish_ready ? 1 : 0
    const pb = polishMap.value[b.dir]?.polish_ready ? 1 : 0
    if (pa !== pb) return pb - pa
    return sa.rank - sb.rank
  }
}

// ── 精修状态筛选：全部 / 已精修（进过精修流程）/ 未精修 ─────────
const CLIP_FILTERS = [
  { key: 'all', label: '全部', hint: '显示全部已剪切片段' },
  { key: 'polished', label: '已精修', hint: '已进入精修流程（初始化过 polish）' },
  { key: 'unpolished', label: '未精修', hint: '还没进精修流程的片段' }
]
const FILTER_STORE = 'vtrim.slices.filter'
const filterKey = ref((() => {
  try { return localStorage.getItem(FILTER_STORE) || 'all' } catch { return 'all' }
})())
watch(filterKey, (v) => { try { localStorage.setItem(FILTER_STORE, v) } catch {} })

const cutClips = computed(() => clips.value
  .filter(c => c.video_exists !== false)
  .slice()
  .sort(makeCmp(clipStats)))
const shownClips = computed(() => {
  if (filterKey.value === 'all') return cutClips.value
  const polished = (c) => !!polishMap.value[c.dir]?.polish_ready
  return cutClips.value.filter((c) => filterKey.value === 'polished' ? polished(c) : !polished(c))
})
const uncutEvents = computed(() => events.value
  .filter(ev => {
    const c = clips.value.find(x => x.name === 'clip-' + String(ev.rank).padStart(2, '0'))
    return !c || !c.video_exists
  })
  .slice()
  .sort(makeCmp(eventStats)))

function hasThumb(c) {
  return c && c.video_exists
}

function thumbSrc(c) {
  const rank = String(c.name || '').replace('clip-', '')
  return projectData.thumbUrl(props.project?.id, rank)
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
    drawerOpen.value = drawerUserPref != null ? drawerUserPref === 'open' : (uncutEvents.value.length > 0)
    return
  }
  loading.value = true
  loadError.value = ''
  try {
    const [ev, cl, pl] = await Promise.all([
      projectData.events(props.project.id).catch(() => ({ events: [] })),
      projectData.clips(props.project.id).catch(() => []),
      projectData.polishMap(props.project.id).catch(() => [])
    ])
    events.value = (ev.events || []).sort((a, b) => a.start_sec - b.start_sec)
    clips.value = cl || []
    const m = {}
    ;(pl || []).forEach(p => { m[p.dir] = p })
    polishMap.value = m
    drawerOpen.value = drawerUserPref != null ? drawerUserPref === 'open' : (uncutEvents.value.length > 0)
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
    await polishPost('/api/polish/' + kind, props.project.id, c.dir)
    toast('已启动 ' + kind)
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
    await runRender(props.project.id, c.dir, out || '')
    toast('渲染已启动')
  } catch (e) { toast(e.message, 'err') }
}

async function preview(c) {
  try {
    const r = await polishPost('/api/polish/preview', props.project.id, c.dir)
    const url = r.video_url || r.cover_url
    if (url) window.open(url, '_blank')
    else toast('预览返回空')
  } catch (e) { toast(e.message, 'err') }
}

async function redetect() {
  try {
    await projectData.pipeline(props.project.id, { stage: 'detect', resume: 'auto' })
    toast('重新检测已启动')
  } catch (e) { toast(e.message, 'err') }
}

// 剪切任务是异步的：发完请求就结束的话，列表永远停在「未剪切」，
// 这里盯住任务 id，终态（成功/失败/取消）时刷新列表。
function watchCutJob(started, label) {
  const r = started
  if (!r?.id) return
  watchJob(r.id, {
    onDone(s) {
      if (s?.status === 'failed') toast(label + '失败：' + (s.message || '未知错误'), 'err')
      load()
    }
  })
}

async function cutAll() {
  const list = uncutEvents.value.slice()
  if (!list.length) return
  const ok = await confirmDialog(`将对 ${list.length} 个未剪切事件执行一键全切，是否继续？`, { title: '一键全切', danger: true })
  if (!ok) return
  try {
    const r = await projectData.cutAll(props.project.id, { ranks: list.map(ev => Number(ev.rank)) })
    toast(`已启动一键全切（${list.length} 个事件），可在“运行任务”查看整体进度`)
    watchCutJob(r, '一键全切')
  } catch (e) {
    toast('启动全切失败: ' + e.message, 'err')
    return
  }
  maximized.value = false
  load()
}

async function cutEvent(ev) {
  const ok = await confirmDialog(`将剪切「${ev.label}」（${fmt(ev.start_sec)} → ${fmt(ev.end_sec)}）`, { title: '剪切该事件？' })
  if (!ok) return
  try {
    const r = await projectData.cutOne(props.project.id, ev.rank)
    toast('剪切任务已启动')
    watchCutJob(r, '剪切')
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
    await projectData.saveEvents(props.project.id, events.value)
    editing.value = null
    toast('已保存')
    load()
  } catch (e) { toast(e.message, 'err') }
}

async function goPolish(c) {
  try {
    const list = await projectData.polishMap(props.project.id).catch(() => [])
    const found = (list || []).find(p => sameDir(p.dir, c.dir))
    // 传**绝对目录**（服务端 scope 守卫会校验它确实落在本人项目内）：
    // 相对路径要自己算 output_dir 的前缀，容易错（旧写法就退化成 'output'，静音/渲染会打错目录）。
    if (!found || !found.polish_ready) {
      await initPolish(props.project.id, c.dir)
      toast('已自动 init')
    }
    // v5 路由：`/p/:pid/polish/:clip`（旧 `/project/:cfg/slices/:clip` 已不存在，会被 catch-all 送回首页）
    router.push({ name: 'project-polish-clip', params: { pid: props.project.id, clip: c.name } })
  } catch (e) { toast(e.message, 'err') }
}

/** 比较两条绝对目录（忽略大小写与分隔符差异）。 */
function sameDir(a, b) {
  const norm = (s) => String(s || '').split(String.fromCharCode(92)).join('/').toLowerCase()
  return norm(a) === norm(b)
}

async function addClip() {
  if (!newLabel.value) { toast('请填写标题'); return }
  if (newEnd.value <= newStart.value) { toast('结束必须大于开始'); return }
  try {
    const created = await projectData.createEvent(props.project.id, {
      label: newLabel.value,
      start_sec: newStart.value,
      end_sec: newEnd.value,
      event_type: newType.value,
      summary: newSummary.value
    })
    const rank = created && created.event && created.event.rank
    if (rank) await projectData.cutOne(props.project.id, rank)
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
watch(() => props.project?.id, load)
</script>

<style scoped>
.sort-bar { display:flex; align-items:center; gap:6px; flex-wrap:wrap; margin-bottom:10px; }
.sort-bar .sort-count { margin-left:auto; }
.v-sep { width:1px; height:14px; background:var(--line); margin:0 4px; flex:none; }
.layout { display:grid; grid-template-columns:minmax(0,1fr) 340px; gap:16px; align-items:start;
  transition:grid-template-columns .25s ease; }
.layout.drawer-closed { grid-template-columns:minmax(0,1fr) 40px; }
.layout.no-drawer { grid-template-columns:minmax(0,1fr); }
/* 抽屉展开时压缩卡片最小宽，避免半窗下只剩一张巨卡 */
.layout .cols-3 { grid-template-columns:repeat(auto-fill,minmax(210px,1fr)); }
.drawer { position:sticky; top:8px; max-height:calc(70vh - 20px); overflow:auto; }
.drawer.collapsed { padding:8px 2px; display:flex; flex-direction:column; align-items:center; gap:10px;
  background:transparent; border:none; box-shadow:none; }
.drawer-head { display:flex; align-items:center; justify-content:space-between; margin-bottom:8px; }
.drawer-head h2 { margin:0; }
.drawer-expand { writing-mode:vertical-rl; letter-spacing:2px; border:1.5px dashed var(--line); border-radius:var(--r-full); background:var(--bg-soft); color:var(--ink-dim); cursor:pointer; font-size:12px; padding:12px 5px; transition:all .15s; }
.drawer-expand:hover { color:var(--main-deep); border-color:var(--main); }
 .uncut-item { padding:10px; border:1px solid var(--line); border-radius:12px; margin-bottom:8px; background:var(--card); }
.max-grid { display:grid; grid-template-columns:repeat(auto-fill,minmax(300px,1fr)); gap:12px; max-height:70vh; overflow:auto; padding-right:4px; }
.max-item { margin:0; display:flex; flex-direction:column; }
.max-title-row { align-items:flex-start; }
.max-title-row b { flex:1; min-width:0; }
.max-summary { display:-webkit-box; -webkit-line-clamp:4; -webkit-box-orient:vertical; overflow:hidden; }
.thumb { width:100%; aspect-ratio:16/9; object-fit:cover; border-radius:var(--r-sm); margin-top:10px; border:1px solid var(--line); cursor:pointer; transition:filter .15s, border-color .15s; }
.thumb:hover { filter:brightness(1.06); border-color:var(--main); }
/* 卡片本身不再响应点击（只有预览图会打开视频窗口），去掉手型光标 */
.pcard.clip-card { cursor:default; }

/* 窄屏：抽屉堆叠到片段下方，横向折叠条 */
@media (max-width: 900px) {
  .layout, .layout.drawer-closed { grid-template-columns:1fr; }
  .drawer { position:static; max-height:none; }
  .drawer.collapsed { flex-direction:row; min-height:auto; padding:6px; }
  .drawer-expand { writing-mode:horizontal-tb; letter-spacing:0; padding:4px 14px; }
}
</style>
