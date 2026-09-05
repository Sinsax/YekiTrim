<template>
  <div>
    <div v-if="loadError" class="error-banner"><Icon name="alert" :size="15" /> {{ loadError }}</div>
    <div v-if="loading" class="grid cols-3">
      <div v-for="i in 6" :key="'sk' + i" class="skeleton"></div>
    </div>
    <ProjectClipsSection v-else-if="project" :project="project" :project-data="projectData" />
    <div v-else class="empty"><div class="empty-icon"><Icon name="film" :size="24" /></div>未找到项目</div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api/client'
import Icon from '../components/Icon.vue'
import ProjectClipsSection from '../components/ProjectClipsSection.vue'

const route = useRoute()
const cfg = decodeURIComponent(route.params.cfg || '')
const cfgQ = encodeURIComponent(cfg)
const project = ref(null)
const projectData = ref({ events: [], clips: [], polishMap: {} })
const loading = ref(true)
const loadError = ref('')

async function load() {
  loading.value = true
  loadError.value = ''
  try {
    const list = await api('/api/project/list')
    project.value = (list.projects || []).find(p => p.config_file === cfg) || {
      config_file: cfg,
      dir: cfg.replace(/[\\/]vtrim\.toml$/i, ''),
      name: cfg.replace(/[\\/]vtrim\.toml$/i, '').split(/[\\/]/).pop() || cfg
    }
    const ev = await api(`/api/project-data/events?config=${cfgQ}`).catch(() => ({ events: [] }))
    const cl = await api(`/api/project-data/clips?config=${cfgQ}`).catch(() => [])
    const pl = await api(`/api/project-data/polish?config=${cfgQ}`).catch(() => [])
    const m = {}
    ;(pl || []).forEach(p => { m[p.dir] = p })
    projectData.value = { events: (ev && ev.events) || [], clips: cl || [], polishMap: m }
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
</style>
