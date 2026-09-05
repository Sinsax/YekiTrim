<template>
  <div>
    <div class="page-head">
      <div><h1>{{ name }}</h1><div class="sub">{{ cfg }}</div></div>
      <div class="actions">
        <router-link class="tab-btn" :to="`/project/${cfgEnc}/overview`">概览</router-link>
        <router-link class="tab-btn" :to="`/project/${cfgEnc}/preview`">预览</router-link>
        <router-link class="tab-btn" :to="`/project/${cfgEnc}/transcript`">字幕</router-link>
        <router-link class="tab-btn" :to="`/project/${cfgEnc}/clips`">片段</router-link>
      </div>
    </div>
    <router-view />
  </div>
</template>

<script setup>
import { computed, ref, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api/client'

const route = useRoute()
const cfg = computed(() => decodeURIComponent(route.params.cfg || ''))
const cfgEnc = computed(() => encodeURIComponent(cfg.value))
const name = ref('工程')

onMounted(async () => {
  try {
    await api('/api/project/switch', { method: 'POST', body: JSON.stringify({ config_path: cfg.value }) })
    const st = await api('/api/project/status')
    name.value = st.name || st.input_file || '工程'
  } catch (e) {
    name.value = '工程'
  }
})
</script>

<style scoped>
.tab-btn { padding:7px 16px; border-radius:999px; border:1px solid var(--line); background:var(--card); color:var(--ink-dim); font-size:13px; cursor:pointer; text-decoration:none; }
.tab-btn.router-link-active { background:linear-gradient(135deg,var(--main),var(--main-deep)); color:#fff; border:none; font-weight:600; }
</style>
