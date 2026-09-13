<template>
  <AppModal :show="show" title="选择服务器文件" @close="close">
    <div class="row mb-6">
      <div class="flex-1 minw-240">
        <label>目录（可填任意绝对路径；管理员可浏览整台机器）</label>
        <input v-model.trim="dir" placeholder="例如 F:/media/2025-09-12" @keyup.enter="browse">
      </div>
      <button class="mini-btn mt-20" @click="browse">打开</button>
      <button class="mini-btn mt-20" :disabled="!parent" @click="goUp">上一级</button>
    </div>
    <div v-if="roots.length" class="row mb-6">
      <div class="flex-1">
        <label>快捷入口（可在 [storage] local_roots 里增删）</label>
        <div class="row quick">
          <button v-for="r in roots" :key="r.path" class="mini-btn" :class="{ on: r.path === dir }" @click="jump(r.path)">
            {{ r.label }}
          </button>
        </div>
      </div>
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
import { listTenantFiles } from '../stores/project'
import AppModal from './AppModal.vue'
import Icon from './Icon.vue'

const props = defineProps({
  show: { type: Boolean, default: false },
  initialPath: { type: String, default: '' },
  videoOnly: { type: Boolean, default: true },
  includeToml: { type: Boolean, default: false }
})
const emit = defineEmits(['select', 'close'])

// 路径一律**相对所选根**（服务端囚笼）；根 = 租户根 + [storage] local_roots（仅 admin）
// 目录一律用**绝对路径**（管理员可去任意位置）；服务端只在 admin 下接受绝对路径。
const dir = ref('')
const parent = ref('')
const roots = ref([])
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
  return dir.value ? dir.value.replace(/[\/\\]+$/, '') + '/' + file : file
}

function isSelected(file) {
  return selected.value === fullPath(file)
}

const confirmLabel = computed(() => selected.value.toLowerCase().endsWith('vtrim.toml') ? '打开项目' : '新建项目')

async function browse() {
  loading.value = true
  loadError.value = ''
  try {
    const d = await listTenantFiles(dir.value)
    dir.value = d.dir || dir.value
    roots.value = d.roots || []
    parent.value = d.parent || ''
    dirs.value = d.dirs || []
    files.value = d.files || []
    selected.value = ''
  } catch (e) {
    dirs.value = []
    files.value = []
    loadError.value = '目录读取失败：' + e.message
  } finally {
    loading.value = false
  }
}

/** 快捷入口：直接跳到某个根（租户根 / [storage] local_roots） */
function jump(p) {
  dir.value = p
  browse()
}

function enterDir(name) {
  dir.value = dir.value ? dir.value.replace(/[\/\\]+$/, '') + '/' + name : name
  browse()
}

function goUp() {
  dir.value = parent.value || ''
  browse()
}

function pick(file) {
  selected.value = fullPath(file)
}

function confirm() {
  if (!selected.value) return
  // 不在这里 close：由父组件根据 select 结果切换/关闭弹窗
  emit('select', { path: selected.value, dir: dir.value })
}

function close() {
  selected.value = ''
  emit('close')
}

watch(() => props.show, (v) => {
  selected.value = ''
  if (v) {
    dir.value = props.initialPath || ''
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
.quick { flex-wrap:wrap; gap:6px; }
.quick .on { background:var(--main-soft); color:var(--main-deep); border-color:var(--main); }
</style>
