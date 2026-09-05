<template>
  <div>
    <div class="page-head">
      <div><h1>切片</h1></div>
    </div>

    <div v-if="loadError" class="error-banner"><Icon name="alert" :size="15" /> {{ loadError }}</div>

    <div v-if="loading" class="grid cols-3">
      <div v-for="i in 6" :key="'sk' + i" class="skeleton"></div>
    </div>

    <div v-else-if="!projects.length" class="empty">
      <div class="empty-icon"><Icon name="sparkles" :size="24" /></div>
      暂无项目
    </div>

    <div v-for="(proj, gi) in projects" :key="proj.config_file" class="project-group">
      <div class="group-header" @click="toggleGroup(gi)">
        <span class="group-name">{{ proj.name || proj.dir }}</span>
        <span class="muted small mono">{{ proj.dir }}</span>
        <span class="muted small">{{ projectDataFor(proj).clips.length }} 个片段</span>
        <span class="row">
          <button class="mini-btn">{{ isExpanded(gi) ? '收起' : '展开' }}</button>
        </span>
      </div>

      <div v-show="isExpanded(gi)" class="project-clips-scroll">
        <ProjectClipsSection :ref="el => setSectionRef(gi, el)" :project="proj" :project-data="projectDataFor(proj)" />
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { api } from '../api/client'
import Icon from '../components/Icon.vue'
import ProjectClipsSection from '../components/ProjectClipsSection.vue'

const projects = ref([])
const projectData = ref({})
const emptyData = { events: [], clips: [], polishMap: {} }
const loading = ref(false)
const loadError = ref('')
const expanded = ref({})
const sectionRefs = ref({})

function setSectionRef(gi, el) {
  if (el) sectionRefs.value[gi] = el
}

function projectDataFor(proj) {
  return projectData.value[proj.config_file] || emptyData
}

function triggerRedetect(gi) {
  sectionRefs.value[gi]?.redetect()
}

function triggerAdd(gi) {
  sectionRefs.value[gi]?.openAdd()
}

function isExpanded(gi) {
  return expanded.value[gi] !== false
}

function toggleGroup(gi) {
  expanded.value[gi] = !isExpanded(gi)
}

async function load() {
  loading.value = true
  loadError.value = ''
  projects.value = []
  projectData.value = {}
  expanded.value = {}
  try {
    const d = await api('/api/project/list')
    const list = (d.projects || []).slice().sort((a, b) => (b.mtime_ms || 0) - (a.mtime_ms || 0))
    projects.value = list
    for (const proj of list) {
      try {
        const cfgQ = encodeURIComponent(proj.config_file)
        const ev = await api(`/api/project-data/events?config=${cfgQ}`).catch(() => ({ events: [] }))
        const cl = await api(`/api/project-data/clips?config=${cfgQ}`).catch(() => [])
        const pl = await api(`/api/project-data/polish?config=${cfgQ}`).catch(() => [])
        const m = {}
        ;(pl || []).forEach(p => { m[p.dir] = p })
        projectData.value[proj.config_file] = {
          events: (ev && ev.events) || [],
          clips: cl || [],
          polishMap: m
        }
      } catch (e) {
        loadError.value = `项目 ${proj.name || proj.id} 数据加载失败: ${e.message}`
      }
    }
  } catch (e) {
    loadError.value = '项目加载失败：' + e.message
  } finally {
    loading.value = false
  }
}

onMounted(load)
</script>

<style scoped>
.project-group { border:1px solid var(--line); border-radius:var(--r); margin-bottom:20px; background:var(--card); overflow:hidden; }
.group-header { display:flex; align-items:center; gap:12px; padding:12px 16px; cursor:pointer; background:var(--bg-soft); border-bottom:1px solid var(--line); }
.group-name { font-weight:700; font-size:15px; }
.project-clips-scroll { max-height:70vh; overflow-y:auto; padding:14px; }
</style>
