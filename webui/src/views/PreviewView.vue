<template>
  <div>
    <div class="card">
      <h2>视频预览</h2>
      <video v-if="src" ref="videoRef" controls playsinline preload="metadata" class="preview-video" :src="src"></video>
      <div v-else class="muted">暂无输入视频</div>
    </div>
    <div class="card" style="margin-top:16px">
      <h2>＋ 添加片段</h2>
      <div class="row">
        <div style="flex:1;min-width:120px"><label>开始时间（秒）</label><input v-model.number="start" type="number" step="0.1" min="0"></div>
        <div style="flex:1;min-width:120px"><label>结束时间（秒）</label><input v-model.number="end" type="number" step="0.1" min="0"></div>
        <button class="mini-btn" @click="takeStart">取当前为开始</button>
        <button class="mini-btn" @click="takeEnd">取当前为结束</button>
      </div>
      <div class="row" style="margin-top:8px">
        <div style="flex:2;min-width:180px"><label>标题</label><input v-model="label" placeholder="片段标题"></div>
        <div style="flex:1;min-width:120px"><label>类型</label>
          <select v-model="eventType">
            <option v-for="t in types" :key="t" :value="t">{{ t }}</option>
          </select>
        </div>
      </div>
      <label style="margin-top:8px">详情</label>
      <textarea v-model="summary" rows="2"></textarea>
      <div class="row" style="margin-top:8px">
        <button class="btn btn-primary" @click="saveClip">生成片段</button>
        <span class="muted small">保存后自动剪切并出现在片段页</span>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../api/client'
import { toast } from '../stores/toast'

const route = useRoute()
const router = useRouter()
const cfg = decodeURIComponent(route.params.cfg || '')
const src = ref('')
const videoRef = ref(null)
const start = ref(0)
const end = ref(10)
const label = ref('')
const summary = ref('')
const eventType = ref('other')
const types = ['gameplay','interaction','qna','reaction','storytelling','music','food','rant','break','outro','chat','eating','irl','ranking','discussion','intro','other']

async function load() {
  try {
    const st = await api('/api/project/status')
    if (st.input_abs) {
      const norm = String(st.input_abs).split(String.fromCharCode(92)).join('/')
      src.value = '/api/media/' + encodeURIComponent(norm).replace(/%2F/g, '/')
    }
  } catch (e) {}
}

function takeStart() {
  if (videoRef.value) start.value = Math.round((videoRef.value.currentTime || 0) * 10) / 10
}
function takeEnd() {
  if (videoRef.value) end.value = Math.round((videoRef.value.currentTime || 0) * 10) / 10
}

async function saveClip() {
  if (!label.value) { toast('请填写标题'); return }
  if (end.value <= start.value) { toast('结束必须大于开始'); return }
  try {
    const created = await api('/api/data/events', { method: 'POST', body: JSON.stringify({
      label: label.value,
      start_sec: start.value,
      end_sec: end.value,
      event_type: eventType.value,
      summary: summary.value
    }) })
    const rank = created && created.event && created.event.rank
    if (rank) await api('/api/data/events/' + rank + '/cut', { method: 'POST' })
    toast('片段已生成，正在剪切')
    router.push(`/project/${encodeURIComponent(cfg)}/clips`)
  } catch (e) {
    toast(e.message)
  }
}

onMounted(load)
</script>

<style scoped>
.preview-video { width:100%; max-height:72vh; object-fit:contain; background:#000; border-radius:10px; display:block; margin:0 auto; }
</style>
