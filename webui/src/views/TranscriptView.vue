<template>
  <div>
    <div class="layout">
      <div class="card">
        <h2>视频预览</h2>
        <video v-if="videoSrc" ref="videoRef" controls playsinline preload="metadata" class="video" :src="videoSrc"></video>
        <div v-else class="muted">暂无输入视频</div>
      </div>
      <div class="card">
        <div class="row spread"><h2 style="margin:0">字幕</h2><span class="muted small">{{ segments.length }} 段</span></div>
        <div class="row" style="margin:8px 0"><button class="btn btn-primary" @click="save">保存字幕修改</button><button class="btn" @click="correct">LLM 订正</button></div>
        <div class="sub-list">
          <div v-if="!segments.length" class="muted">暂无字幕</div>
          <div v-for="(seg, i) in segments" :key="i" class="sub-row" @click="playAt(seg)">
            <span class="muted small mono time">{{ fmt(seg.start) }}</span>
            <textarea v-model="seg.text" rows="1" class="sub-input" @click.stop></textarea>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api/client'
import { toast } from '../stores/toast'

const route = useRoute()
const cfg = decodeURIComponent(route.params.cfg || '')
const videoSrc = ref('')
const videoRef = ref(null)
const segments = ref([])

function fmt(sec) {
  sec = Number(sec) || 0
  const h = Math.floor(sec / 3600), m = Math.floor(sec % 3600 / 60), s = Math.floor(sec % 60)
  return (h ? h + ':' + String(m).padStart(2, '0') : m) + ':' + String(s).padStart(2, '0')
}

function playAt(seg) {
  if (videoRef.value) {
    videoRef.value.currentTime = seg.start
    videoRef.value.play()
  }
}

async function load() {
  try {
    const st = await api('/api/project/status')
    if (st.input_abs) {
      const norm = String(st.input_abs).split(String.fromCharCode(92)).join('/')
      videoSrc.value = '/api/media/' + encodeURIComponent(norm).replace(/%2F/g, '/')
    }
  } catch (e) {}
  try {
    const d = await api('/api/data/transcript')
    segments.value = d.segments || []
  } catch (e) {}
}

async function correct() {
  try {
    await api('/api/correct', { method: 'POST' })
    toast('LLM 订正已启动')
  } catch (e) { toast(e.message) }
}

async function save() {
  try {
    await api('/api/data/transcript', { method: 'PUT', body: JSON.stringify({ segments: segments.value, language: 'zh' }) })
    toast('字幕已保存')
  } catch (e) { toast(e.message) }
}

onMounted(load)
</script>

<style scoped>
.layout { display:grid; grid-template-columns:minmax(0,16fr) minmax(0,9fr); gap:14px; align-items:start; }
.video { width:100%; max-height:72vh; object-fit:contain; background:#000; border-radius:10px; display:block; }
.sub-list { max-height:55vh; overflow-y:auto; border:1px solid var(--line); border-radius:10px; padding:8px; }
.sub-row { display:flex; gap:6px; align-items:flex-start; padding:5px 6px; border-bottom:1px dashed var(--line-soft); cursor:pointer; }
.time { min-width:80px; font-size:11px; line-height:2; }
.sub-input { flex:1; font-size:12px; resize:none; }
</style>
