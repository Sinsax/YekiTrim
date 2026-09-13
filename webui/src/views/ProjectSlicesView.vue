<template>
  <div>
    <div v-if="loadError" class="error-banner"><Icon name="alert" :size="15" /> {{ loadError }}</div>
    <div class="row slices-bar">
      <span class="muted small">片段操作</span>
      <button class="mini-btn" :disabled="!clipsRef" @click="clipsRef?.redetect()">重新检测</button>
      <button class="mini-btn" :disabled="!clipsRef" @click="clipsRef?.openAdd()">添加片段</button>
    </div>
    <div v-if="loading" class="grid cols-3">
      <div v-for="i in 6" :key="'sk' + i" class="skeleton"></div>
    </div>
    <ProjectClipsSection v-else-if="project" ref="clipsRef" :project="project" :project-data="projectDataRef" />
    <div v-else class="empty"><div class="empty-icon"><Icon name="film" :size="24" /></div>未找到项目</div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api/client'
import { scopeQ } from '../api/scope'
import { loadProjects, projectById } from '../stores/workbench'
import { projectData } from '../api/projectData'
import Icon from '../components/Icon.vue'
import ProjectClipsSection from '../components/ProjectClipsSection.vue'

const route = useRoute()
const pid = route.params.pid || ''
const project = ref(null)
const projectDataRef = ref({ events: [], clips: [], polishMap: {} })
const clipsRef = ref(null)
const loading = ref(true)
const loadError = ref('')

async function load() {
  loading.value = true
  loadError.value = ''
  try {
    if (!projectById(pid)) await loadProjects()
    project.value = projectById(pid)
    if (!project.value) throw new Error('项目不存在或无权访问')
    const [ev, cl, pl] = await Promise.all([
      projectData.events(pid).catch(() => ({ events: [] })),
      projectData.clips(pid).catch(() => []),
      projectData.polishMap(pid).catch(() => [])
    ])
    const m = {}
    ;(pl || []).forEach(p => { m[p.dir] = p })
    projectDataRef.value = { events: (ev && ev.events) || [], clips: cl || [], polishMap: m }
  } catch (e) {
    loadError.value = '切片数据加载失败：' + e.message
  } finally {
    loading.value = false
  }
}

onMounted(load)
</script>

<style scoped>
.error-banner { margin-bottom:12px; }
.slices-bar { align-items:center; margin-bottom:10px; }
button:disabled { opacity:.45; cursor:not-allowed; }
</style>
