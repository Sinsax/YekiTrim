<template>
  <div>
    <div class="page-head">
      <div><h1>精修</h1><div class="sub">polish 项目列表</div></div>
    </div>
    <div class="grid cols-3">
      <div v-for="p in projects" :key="p.dir" class="pcard">
        <div class="head">
          <div class="sticker pink">✨</div>
          <div>
            <h3>{{ p.title || p.dir }}</h3>
            <div class="path">{{ p.dir }}</div>
          </div>
        </div>
        <div class="chips">
          <span class="chip" :class="p.processed ? 'g' : 'n'">{{ p.processed ? '已 process' : '未 process' }}</span>
          <span class="chip" :class="p.rendered ? 'g' : 'n'">{{ p.rendered ? '已渲染' : '未渲染' }}</span>
          <span class="chip" :class="p.cover ? 'g' : 'n'">{{ p.cover ? '有封面' : '无封面' }}</span>
        </div>
        <div class="foot">
          <button class="mini-btn" @click="run('init', p)">init</button>
          <button class="mini-btn" @click="preview(p)">精修预览</button>
          <button class="mini-btn" @click="run('llm', p)">LLM</button>
          <button class="mini-btn" @click="run('render', p)">渲染</button>
          <button class="mini-btn" @click="run('cover', p)">封面</button>
          <button class="mini-btn" @click="openEditor(p)">编辑器</button>
        </div>
      </div>
      <div v-if="!projects.length" class="card muted">暂无精修项目</div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { api } from '../api/client'
import { toast } from '../stores/toast'

const projects = ref([])
const router = useRouter()

async function load() {
  try {
    projects.value = await api('/api/polish/projects')
  } catch (e) {
    projects.value = []
  }
}

async function run(kind, p) {
  try {
    await api('/api/polish/' + kind, { method: 'POST', body: JSON.stringify({ dir: p.dir }) })
    toast('已启动 ' + kind)
    load()
  } catch (e) { toast(e.message) }
}

async function preview(p) {
  try {
    const r = await api('/api/polish/preview', { method: 'POST', body: JSON.stringify({ dir: p.dir }) })
    const url = r.video_url || r.cover_url
    if (url) window.open(url, '_blank')
    else toast('预览返回空')
  } catch (e) { toast(e.message) }
}

async function openEditor(p) {
  let cfg = ''
  try {
    const st = await api('/api/project/status')
    cfg = st.config_file || ''
  } catch (e) {}
  router.push(`/project/${encodeURIComponent(cfg)}/polish?dir=${encodeURIComponent(p.dir)}`)
}

onMounted(load)
</script>
