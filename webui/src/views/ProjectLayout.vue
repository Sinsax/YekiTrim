<template>
  <div>
    <div class="page-head">
      <div>
        <h1>{{ name }}</h1>
        <div class="sub">
          <span v-if="project?.source?.label">{{ project.source.label }}</span>
          <span v-if="project" class="chip n ml-6">{{ humanBytes(project.bytes) }}</span>
          <span v-else class="mono">项目 #{{ pid }}</span>
        </div>
      </div>
      <div class="actions subnav">
        <router-link class="tab-btn" :to="`/project/${pid}/preview`">预览</router-link>
        <router-link class="tab-btn" :to="`/project/${pid}/slices`">切片</router-link>
        <router-link class="tab-btn" :to="`/project/${pid}/settings`">设置</router-link>
      </div>
    </div>
    <div v-if="!ready" class="muted small" style="padding:20px">切换项目中…</div>
    <div v-else-if="missing" class="empty">
      <span class="empty-icon">⚠</span>
      项目目录或 vtrim.toml 已不存在（可能被移动/删除）。在总览页移除它，或把文件放回原位置。
    </div>
    <router-view v-else />
  </div>
</template>

<script setup>
import { computed, ref, watch, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api/client'
import { scopeQ, setScope } from '../api/scope'
import { humanBytes, loadProjects, projectById } from '../stores/project'

const route = useRoute()
const pid = computed(() => route.params.pid || '')
const project = computed(() => projectById(pid.value))
const name = ref('工程')
const ready = ref(false)
const missing = ref(false)

async function load() {
  if (!pid.value) return
  ready.value = false
  missing.value = false
  name.value = '工程'
  setScope(pid.value, projectById(pid.value))
  try {
    const st = await api(`/api/project-data/status?${scopeQ(pid.value)}`)
    name.value = st.name || st.input_file || '工程'
  } catch (e) {
    missing.value = true
  } finally {
    ready.value = true
  }
}

onMounted(async () => {
  if (!projectById(pid.value)) await loadProjects()
  setScope(pid.value, projectById(pid.value))
  await load()
})
watch(pid, load)
</script>

<style scoped>
.ml-6 { margin-left:6px; }
</style>
