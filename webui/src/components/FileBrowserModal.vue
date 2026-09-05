<template>
  <AppModal :show="show" title="选择文件" @close="close">
    <div class="row">
      <div class="flex-1 minw-180">
        <label>当前目录</label>
        <input v-model.trim="path" placeholder="目录路径" @keyup.enter="browse">
      </div>
      <button class="mini-btn mt-20" @click="browse">打开</button>
    </div>
    <div v-if="loadError" class="error-banner mt-8">{{ loadError }}</div>
    <div class="file-box mt-8">
      <div v-if="loading" class="row muted small"><span class="spinner"></span> 加载中…</div>
      <div v-else-if="!dirs.length && !filteredFiles.length" class="empty small">空目录</div>
      <div v-for="d in dirs" :key="'d-' + d" class="file-row" @click="enterDir(d)">
        <span class="file-icon"><Icon name="folder" :size="15" /></span><span class="mono">{{ d }}/</span>
      </div>
      <div v-for="f in filteredFiles" :key="'f-' + f" class="file-row video" :class="{ selected: isSelected(f) }" @click="pick(f)">
        <span class="file-icon"><Icon :name="isVideo(f) ? 'film' : 'file'" :size="15" /></span><span class="mono">{{ f }}</span>
      </div>
    </div>
    <div class="row mt-12">
      <span class="spacer"></span>
      <button class="btn" @click="close">取消</button>
      <button class="btn btn-primary" :disabled="!selected" @click="confirm">{{ confirmLabel }}</button>
    </div>
  </AppModal>
</template>

<script setup>
import { ref, watch, computed } from 'vue'
import { api } from '../api/client'
import AppModal from './AppModal.vue'
import Icon from './Icon.vue'

const props = defineProps({
  show: { type: Boolean, default: false },
  initialPath: { type: String, default: '' },
  videoOnly: { type: Boolean, default: true },
  includeToml: { type: Boolean, default: false }
})
const emit = defineEmits(['select', 'close'])

const path = ref('')
const dirs = ref([])
const files = ref([])
const loading = ref(false)
const loadError = ref('')
const selected = ref('')

const VIDEO_EXTS = ['mp4', 'mkv', 'flv', 'mov', 'm4v', 'webm', 'avi', 'ts']

function isVideo(name) {
  const ext = String(name).split('.').pop().toLowerCase()
  return VIDEO_EXTS.includes(ext)
}

const filteredFiles = computed(() => {
  if (!props.videoOnly) return files.value
  return files.value.filter(f => isVideo(f) || (props.includeToml && f.toLowerCase() === 'vtrim.toml'))
})

function fullPath(file) {
  return path.value ? path.value.replace(/[\/]+$/, '') + '/' + file : file
}

function isSelected(file) {
  return selected.value === fullPath(file)
}

const confirmLabel = computed(() => selected.value.toLowerCase().endsWith('vtrim.toml') ? '打开项目' : '新建项目')

async function browse() {
  loading.value = true
  loadError.value = ''
  try {
    const d = await api('/api/fs/list', { method: 'POST', body: JSON.stringify({ dir: path.value }) })
    path.value = d.dir || path.value
    dirs.value = d.dirs || []
    files.value = d.files || []
  } catch (e) {
    dirs.value = []
    files.value = []
    loadError.value = '目录读取失败：' + e.message
  } finally {
    loading.value = false
  }
}

function enterDir(name) {
  path.value = path.value ? path.value.replace(/[\/]+$/, '') + '/' + name : name
  browse()
}

function pick(file) {
  selected.value = fullPath(file)
}

function confirm() {
  if (!selected.value) return
  emit('select', selected.value)
  close()
}

function close() {
  selected.value = ''
  emit('close')
}

watch(() => props.show, (v) => {
  selected.value = ''
  if (v) {
    path.value = props.initialPath || ''
    browse()
  }
})
</script>

<style scoped>
.file-box { max-height:280px; overflow:auto; border:1px solid var(--line); border-radius:var(--r-sm); padding:8px; background:var(--card); }
.file-row { display:flex; align-items:center; gap:8px; padding:6px 8px; border-radius:8px; cursor:pointer; color:var(--ink-dim); }
.file-row:hover { background:var(--bg-soft); }
.file-row.selected { background:var(--main-soft); color:var(--main-deep); border:1px solid var(--main); }
.file-row.video { color:var(--ink); }
.file-icon { flex:none; display:flex; }
.file-row .mono { font-size:12px; word-break:break-all; }
.spacer { flex:1; }
</style>
