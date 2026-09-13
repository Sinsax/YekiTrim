<template>
  <div>
    <div class="page-head">
      <div><h1>总览</h1><div class="sub">多项目工作区</div></div>
      <div class="actions">
        <button class="btn" @click="load"><Icon name="refresh" :size="14" /> 刷新</button>
        <button v-if="isAdmin" class="btn btn-primary" @click="showNew = true">
          <Icon name="plus" :size="14" /> 从服务器文件新建
        </button>
      </div>
    </div>

    <div v-if="loadError" class="error-banner"><Icon name="alert" :size="15" /> {{ loadError }}</div>

    <div class="block-frame">
      <div class="section-title row spread">
        <span>环境与模型</span>
        <button class="btn" @click="checkEnv">环境检查</button>
      </div>
      <div class="card">
        <div class="env-block">
          <div class="env-label">环境</div>
          <div class="chips">
            <span class="chip" :class="health.ffmpeg ? 'g' : 'r'">ffmpeg {{ health.ffmpeg ? '✓' : '✗' }}</span>
            <span class="chip" :class="health.llm ? 'g' : 'n'">LLM {{ health.llm ? '✓' : '未配置' }}</span>
            <span class="chip" :class="health.chrome ? 'g' : 'r'">Chrome {{ health.chrome ? '✓' : '✗' }}</span>
          </div>
        </div>
        <div class="env-block">
          <div class="env-label">模型</div>
          <div class="chips">
            <span v-for="m in visibleModels" :key="m.name" class="chip" :class="m.installed ? 'g' : 'n'">
              {{ m.name }} {{ m.installed ? '已配置' : '未配置' }}
            </span>
          </div>
        </div>
      </div>
    </div>

    <div class="block-frame">
      <div class="section-title">项目</div>
      <div class="grid cols-4">
        <ProjectCard
          v-for="p in projects"
          :key="p.id"
          :p="p"
          removable
          @open="openProject"
          @remove="removeProject"
          @remove-source="removeSource"
        />
        <div v-if="!projects.length && !loading" class="empty">
          <div class="empty-icon"><Icon name="film" :size="24" /></div>
          暂无项目<template v-if="isAdmin">，点击「从服务器文件新建」开始</template>
        </div>
        <div v-for="i in 4" v-show="loading" :key="'sk' + i" class="skeleton"></div>
      </div>
    </div>

    <div class="block-frame">
      <div class="section-title row spread">
        <span>录制导入（最近）</span>
        <router-link class="mini-btn" to="/ingest">全部记录</router-link>
      </div>
      <div v-if="ingestState.ready === false" class="empty">
        <div class="empty-icon"><Icon name="alert" :size="22" /></div>
        录播姬尚未接入：本机执行 <span class="mono">vtrim ingest token</span> 生成 token，
        并把录制目录填进 <span class="mono">[ingest.recorder] root</span>。
      </div>
      <div v-else class="card">
        <div v-if="!ingestState.records.length" class="muted small">暂无入库记录</div>
        <div v-else>
          <div v-for="r in ingestState.records.slice(0, 5)" :key="r.session_id" class="row spread ingest-row">
            <div>
              <b>房间 {{ r.room_id || '-' }}</b>
              <span class="chip ml-6" :class="stateClass(r.state)">{{ r.state_label }}</span>
              <span class="muted small ml-6">{{ ingestFmtTime(r.started_at) }} · {{ r.file_count }} 文件 · {{ ingestFmtBytes(r.bytes) }}</span>
            </div>
            <router-link v-if="r.project_id" class="mini-btn" :to="`/project/${r.project_id}/slices`">打开项目</router-link>
            <button v-else-if="r.state === 'failed'" class="mini-btn" @click="retryIngest(r)">重试</button>
          </div>
        </div>
      </div>
    </div>

    <div class="block-frame">
      <div class="section-title">运行任务</div>
      <div class="card"><JobList /></div>
    </div>

    <NewProjectModal v-model:show="showNew" :workspace="workspace" @created="load" />
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { api } from '../api/client'
import { authState } from '../stores/auth'
import { deleteProject, deleteProjectSource, loadProjects } from '../stores/project'
import { ingestApi } from '../api/ingest'
import {
  fmtBytes as ingestFmtBytes,
  fmtTime as ingestFmtTime,
  ingestState,
  loadRecords,
  setReady,
  stateClass
} from '../stores/ingest'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import { projectEntryPath } from '../utils'
import Icon from '../components/Icon.vue'
import ProjectCard from '../components/ProjectCard.vue'
import JobList from '../components/JobList.vue'
import NewProjectModal from '../components/NewProjectModal.vue'

const router = useRouter()
const health = ref({})
const models = ref([])
const projects = ref([])
const workspace = ref('')
const showNew = ref(false)
const loading = ref(false)
const loadError = ref('')

const isAdmin = computed(() => authState.user?.role === 'admin')
const visibleModels = computed(() => models.value.filter(m => m.name !== 'firered-zh'))

async function load() {
  loading.value = true
  loadError.value = ''
  try {
    // M2：只列**本人**项目（归属由服务端 DB 决定，前端不再传路径）
    projects.value = await loadProjects()
  } catch (e) {
    loadError.value = '项目列表加载失败：' + e.message
  } finally {
    loading.value = false
  }
  try { health.value = await api('/api/health') } catch {}
  setReady(health.value?.ingest_ready !== false)
  try { await loadRecords('mine') } catch {}
  if (isAdmin.value) {
    try { models.value = await api('/api/admin/models') } catch {}
  }
}

async function retryIngest(r) {
  try {
    await ingestApi.retry(r.session_id)
    toast('已重新排队')
    await loadRecords('mine')
  } catch (e) { toast(e.message, 'err') }
}

async function checkEnv() {
  try {
    health.value = await api('/api/health')
    if (isAdmin.value) models.value = await api('/api/admin/models')
    toast('已检查')
  } catch (e) {
    toast(e.message, 'err')
  }
}

function openProject(p) {
  // 预处理（事件检测）已完成的项目直接进切片页，否则先进预览页
  router.push(projectEntryPath(p.id, p.stage))
}

async function removeProject(p) {
  const ok = await confirmDialog('只注销项目（删 vtrim.toml），不会触碰媒体文件。', { title: `删除项目「${p.name}」？`, danger: true })
  if (!ok) return
  try {
    await deleteProject(p.id)
    toast('项目已删除')
  } catch (e) { toast(e.message, 'err') }
}

async function removeSource(p) {
  const ok = await confirmDialog(
    '删除源录播（原始视频/分片），保留切片、字幕、事件与封面。此操作不可恢复。',
    { title: `删除「${p.name}」的源录播？`, danger: true }
  )
  if (!ok) return
  try {
    const r = await deleteProjectSource(p.id)
    toast(`已释放 ${(r?.freed_bytes / 1024 / 1024 || 0).toFixed(1)} MB`)
  } catch (e) { toast(e.message, 'err') }
}

onMounted(load)
</script>

<style scoped>
.env-block { display:flex; align-items:center; gap:12px; padding:8px 0; border-bottom:1px dashed var(--line-soft); }
.env-block:last-child { border-bottom:none; }
.env-label { flex:none; width:48px; font-size:12px; color:var(--ink-dim); font-weight:600; }
</style>
