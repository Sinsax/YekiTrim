<template>
  <FileBrowserModal
    :show="show && !pickedVideo"
    :initial-path="workspace"
    video-only
    include-toml
    @select="onPickVideo"
    @close="close"
  />
  <AppModal :show="show && !!pickedVideo" title="打开项目" @close="backToBrowse">
    <label>选择文件路径</label>
    <input :value="pickedVideo" readonly>
    <label v-if="!isToml" class="mt-8">弹幕文件（可选）</label>
    <input v-if="!isToml" v-model="danmaku" placeholder="自动查找同名 .ass">
    <label v-if="!isToml" class="mt-8">输出目录</label>
    <input v-if="!isToml" v-model="output">
    <div v-if="isToml" class="muted small mt-8">已选择 vtrim.toml，将直接打开项目，不执行 init。</div>
    <template #footer>
      <button class="btn" @click="backToBrowse">上一步</button>
      <button class="btn" @click="close">取消</button>
      <button class="btn btn-primary" :disabled="!pickedVideo || creating" @click="create">
        <span v-if="creating" class="spinner"></span>{{ creating ? '处理中…' : '打开项目' }}
      </button>
    </template>
  </AppModal>
</template>

<script setup>
import { ref, computed } from 'vue'
import { api } from '../api/client'
import { toast } from '../stores/toast'
import AppModal from './AppModal.vue'
import FileBrowserModal from './FileBrowserModal.vue'

defineProps({
  show: { type: Boolean, default: false },
  workspace: { type: String, default: '' }
})
const emit = defineEmits(['update:show', 'created'])

const pickedVideo = ref('')
const danmaku = ref('')
const output = ref('./output')
const creating = ref(false)
const isToml = computed(() => pickedVideo.value.toLowerCase().endsWith('vtrim.toml'))

function close() {
  pickedVideo.value = ''
  emit('update:show', false)
}

function backToBrowse() {
  pickedVideo.value = ''
}

function onPickVideo(path) {
  pickedVideo.value = path
  danmaku.value = ''
  output.value = './output'
  if (pickedVideo.value.toLowerCase().endsWith('vtrim.toml')) {
    create()
  }
}

async function create() {
  if (!pickedVideo.value) { toast('请选择文件'); return }
  creating.value = true
  try {
    if (isToml.value) {
      await api('/api/project/open', { method: 'POST', body: JSON.stringify({ config_path: pickedVideo.value }) })
      toast('已打开项目')
      close()
      emit('created')
      return
    }
    try {
      await api('/api/project/open', { method: 'POST', body: JSON.stringify({ input: pickedVideo.value }) })
      toast('已打开项目')
      close()
      emit('created')
      return
    } catch (openErr) {
      // 不存在项目文件时走初始化
      await api('/api/project/init', { method: 'POST', body: JSON.stringify({
        input: pickedVideo.value,
        danmaku: danmaku.value || null,
        output_dir: output.value || './output'
      }) })
      toast('项目已创建')
      close()
      emit('created')
    }
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    creating.value = false
  }
}
</script>
