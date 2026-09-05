<template>
  <div>
    <div class="page-head">
      <div><h1>总览</h1><div class="sub">多项目工作区</div></div>
      <div class="actions">
        <button class="btn" @click="load"><Icon name="refresh" :size="14" /> 扫描工作区</button>
        <button class="btn btn-primary" @click="showNew = true"><Icon name="plus" :size="14" /> 打开项目</button>
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
          :key="p.config_file"
          :p="p"
          removable
          @open="openProject"
          @remove="removeProject"
        />
        <div v-if="!projects.length && !loading" class="empty">
          <div class="empty-icon"><Icon name="film" :size="24" /></div>
          暂无项目，点击「打开项目」开始
        </div>
        <div v-for="i in 4" v-show="loading" :key="'sk' + i" class="skeleton"></div>
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
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
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

const visibleModels = computed(() => models.value.filter(m => m.name !== 'firered-zh'))

async function load() {
  loading.value = true
  loadError.value = ''
  try {
    const d = await api('/api/project/list')
    projects.value = d.projects || []
    workspace.value = d.workspace_root || ''
  } catch (e) {
    loadError.value = '项目列表加载失败：' + e.message
  } finally {
    loading.value = false
  }
  try { health.value = await api('/api/health') } catch {}
  try { models.value = await api('/api/models') } catch {}
}

async function checkEnv() {
  try {
    health.value = await api('/api/health')
    models.value = await api('/api/models')
    toast('已检查')
  } catch (e) {
    toast(e.message, 'err')
  }
}

function openProject(p) {
  router.push(`/project/${encodeURIComponent(p.config_file)}/preview`)
}

async function removeProject(p) {
  const ok = await confirmDialog('只删除 vtrim.toml，不会触碰媒体文件。', { title: `删除项目「${p.name}」？`, danger: true })
  if (!ok) return
  try {
    await api('/api/project/delete', { method: 'POST', body: JSON.stringify({ config_path: p.config_file }) })
    toast('项目已删除')
    load()
  } catch (e) { toast(e.message, 'err') }
}

onMounted(load)
</script>

<style scoped>
.env-block { display:flex; align-items:center; gap:12px; padding:8px 0; border-bottom:1px dashed var(--line-soft); }
.env-block:last-child { border-bottom:none; }
.env-label { flex:none; width:48px; font-size:12px; color:var(--ink-dim); font-weight:600; }
</style>
