<template>
  <div>
    <div class="layout">
      <div class="card">
        <h2>视频预览</h2>
        <video v-if="videoSrc" ref="videoRef" controls playsinline preload="metadata" class="video" :src="videoSrc"></video>
        <div v-else class="empty small">暂无输入视频</div>
      </div>
      <div class="card">
        <div class="row spread"><h2 class="m-0">字幕</h2><span class="muted small">{{ segments.length }} 段</span></div>
        <div class="row my-8">
          <button class="btn btn-primary" @click="save"><Icon name="save" :size="13" /> 保存字幕修改</button>
          <button class="btn" @click="correct">LLM 订正</button>
        </div>
        <SubtitlePanel :segments="segments" @play="playAt" />
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api/client'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import Icon from '../components/Icon.vue'
import SubtitlePanel from '../components/SubtitlePanel.vue'
import { inputMediaUrl } from '../utils'

const route = useRoute()
const videoSrc = ref('')
const videoRef = ref(null)
const segments = ref([])

function playAt(seg) {
  if (videoRef.value) {
    videoRef.value.currentTime = seg.start
    videoRef.value.play()
  }
}

async function load() {
  try {
    const st = await api('/api/project/status')
    if (st.input_abs) videoSrc.value = inputMediaUrl(st.input_abs)
  } catch {}
  try {
    const d = await api('/api/data/transcript')
    segments.value = d.segments || []
  } catch {}
}

async function correct() {
  const ok = await confirmDialog('将对全部字幕启动 LLM 订正', { title: '启动 LLM 订正？' })
  if (!ok) return
  try {
    await api('/api/correct', { method: 'POST' })
    toast('LLM 订正已启动')
  } catch (e) { toast(e.message, 'err') }
}

async function save() {
  try {
    await api('/api/data/transcript', { method: 'PUT', body: JSON.stringify({ segments: segments.value, language: 'zh' }) })
    toast('字幕已保存')
  } catch (e) { toast(e.message, 'err') }
}

onMounted(load)
</script>

<style scoped>
.layout { display:grid; grid-template-columns:minmax(0,16fr) minmax(0,9fr); gap:14px; align-items:start; }
.video { width:100%; max-height:72vh; object-fit:contain; background:#000; border-radius:var(--r-sm); display:block; }
</style>
