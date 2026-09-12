<template>
  <div>
    <div class="page-head">
      <div>
        <h1>{{ name }}</h1>
        <div class="sub">{{ cfg }}</div>
      </div>
      <div class="actions subnav">
        <router-link class="tab-btn" :to="`/project/${cfgEnc}/preview`">预览</router-link>
        <router-link class="tab-btn" :to="`/project/${cfgEnc}/slices`">切片</router-link>
        <router-link class="tab-btn" :to="`/project/${cfgEnc}/settings`">设置</router-link>
      </div>
    </div>
    <div v-if="!ready" class="muted small" style="padding:20px">切换项目中…</div>
    <router-view v-else />
  </div>
</template>

<script setup>
import { computed, ref, watch, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api/client'

const route = useRoute()
const cfg = computed(() => route.params.cfg || '')
const cfgEnc = computed(() => encodeURIComponent(cfg.value))
const isPolishRoute = computed(() => route.path.includes('/polish') || /\/slices\/[^/]+$/.test(route.path))
const name = ref('工程')
const ready = ref(false)

async function load() {
  if (!cfg.value) return
  ready.value = false
  name.value = '工程'
  try {
    const st = await api(`/api/project-data/status?config=${encodeURIComponent(cfg.value)}`)
    name.value = st.name || st.input_file || '工程'
  } catch (e) {
    name.value = '工程'
  } finally {
    ready.value = true
  }
}

onMounted(load)
watch(() => cfg.value, load)
</script>

