<template>
  <FileBrowserModal
    :show="show && !picked"
    :initial-path="''"
    video-only
    include-toml
    @select="onPick"
    @close="close"
  />
  <AppModal :show="show && !!picked" title="从服务器文件新建项目" @close="backToBrowse">
    <label>服务器文件（绝对路径）</label>
    <input :value="picked" readonly>
    <template v-if="!isToml">
      <label class="mt-8">弹幕文件（可选，同目录相对路径）</label>
      <input v-model="danmaku" placeholder="留空则自动查找同名弹幕">
      <label class="mt-8">输出目录</label>
      <input v-model="output" placeholder="./output">
      <p class="muted small mt-8">
        只使用**服务器上已存在**的视频（管理员来源），**不接受上传**。
        若所选目录已有 <span class="mono">vtrim.toml</span>，则只登记/打开，不会覆盖。
      </p>
    </template>
    <div v-if="isToml" class="muted small mt-8">已选择 vtrim.toml：直接登记并打开该项目，不重新 init。</div>
    <template #footer>
      <button class="btn" @click="backToBrowse">上一步</button>
      <button class="btn" @click="close">取消</button>
      <button class="btn btn-primary" :disabled="!picked || creating" @click="create">
        <span v-if="creating" class="spinner"></span>{{ creating ? '处理中…' : (isToml ? '打开项目' : '新建项目') }}
      </button>
    </template>
  </AppModal>
</template>

<script setup>
import { ref, computed } from 'vue'
import { createLocalProject } from '../stores/project'
import { toast } from '../stores/toast'
import AppModal from './AppModal.vue'
import FileBrowserModal from './FileBrowserModal.vue'

defineProps({
  show: { type: Boolean, default: false },
  workspace: { type: String, default: '' }
})
const emit = defineEmits(['update:show', 'created'])

const picked = ref('')
const pickedDir = ref('')
const danmaku = ref('')
const output = ref('./output')
const creating = ref(false)
const isToml = computed(() => picked.value.toLowerCase().endsWith('vtrim.toml'))

/** 选中项的目录（服务端回传的绝对目录）与文件名。 */
function splitPick(p) {
  const norm = String(p || '').split('\\').join('/')
  const i = norm.lastIndexOf('/')
  return i < 0 ? { dir: '', file: norm } : { dir: norm.slice(0, i), file: norm.slice(i + 1) }
}

function close() {
  picked.value = ''
  pickedDir.value = ''
  emit('update:show', false)
}

function backToBrowse() {
  picked.value = ''
}

function onPick(payload) {
  // 兼容两种 emit：字符串（旧）与 {path, dir}
  const p = typeof payload === 'string' ? payload : payload?.path || ''
  pickedDir.value = typeof payload === 'string' ? '' : payload?.dir || ''
  picked.value = p
  danmaku.value = ''
  output.value = './output'
  if (isToml.value) create()
}

async function create() {
  if (!picked.value) { toast('请选择文件'); return }
  const { dir, file } = splitPick(picked.value)
  creating.value = true
  try {
    const r = await createLocalProject({
      dir: pickedDir.value || dir,
      input: isToml.value ? '' : file,
      danmaku: isToml.value ? '' : (danmaku.value || ''),
      output_dir: isToml.value ? '' : (output.value || '')
    })
    toast(isToml.value ? '项目已打开' : '项目已创建')
    close()
    emit('created', r)
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    creating.value = false
  }
}
</script>
