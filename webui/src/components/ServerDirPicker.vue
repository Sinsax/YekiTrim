<template>
  <div>
    <div class="row">
      <div class="flex-1 minw-240">
        <label>{{ dirLabel }}</label>
        <input
          v-model.trim="dir"
          :placeholder="dirPlaceholder"
          autocomplete="off"
          @keyup.enter="browse()"
        >
      </div>
      <button class="mini-btn mt-20" @click="browse()">浏览…</button>
      <button class="mini-btn mt-20" :disabled="!parent" @click="goUp()">上一级</button>
      <button v-if="mode === 'dir'" class="mini-btn mt-20" :disabled="!dir" @click="emit('pick-dir', dir)">
        选择当前目录
      </button>
    </div>

    <div v-if="roots.length" class="row" style="gap:8px;margin-top:8px;flex-wrap:wrap">
      <span class="muted small" style="flex:none">快捷入口</span>
      <button v-for="r in roots" :key="r.path" class="mini-btn" @click="browse(r.path)">{{ r.label }}</button>
    </div>

    <div v-if="error" class="error-banner mt-8">{{ error }}</div>

    <div class="file-box mt-8">
      <div v-if="loading" class="row muted small"><span class="spinner"></span> 读取中…</div>
      <template v-else>
        <div v-for="d in dirs" :key="'d-' + d" class="file-row" @click="enter(d)">
          <span class="file-icon"><Icon name="folder" :size="15" /></span>
          <span class="mono">{{ d }}/</span>
          <!-- dir 模式：一步选中子目录（路径由服务端在 enter 之后回传，前端不拼字符串） -->
          <button v-if="mode === 'dir'" class="mini-btn" @click.stop="pickChild(d)">选择</button>
        </div>
        <div
          v-for="f in files"
          :key="'f-' + f"
          class="file-row"
          :class="{ video: isVideoFile(f), selected: selected === f, dim: !allowed(f) }"
          @click="mode === 'file' && allowed(f) && emit('pick-file', f)"
        >
          <span class="file-icon"><Icon :name="iconFor(f)" :size="15" /></span>
          <span class="mono">{{ f }}</span>
          <span v-if="!allowed(f)" class="muted small">不适用</span>
          <span v-else-if="isVideoFile(f)" class="muted small">视频</span>
        </div>
        <div v-if="!dirs.length && !files.length" class="empty small">空目录</div>
      </template>
    </div>
  </div>
</template>

<script>
/** 媒体扩展名白名单。**导出**是因为调用方要判"这个目录里还有没有可用的视频"
 *  （首页「服务器文件」建项目要据此决定「新建项目」能不能点、以及会拿哪个文件）。
 *  消费方自己再抄一份迟早会漂。 */
export const VIDEO_EXTS = ['mp4', 'mkv', 'flv', 'mov', 'm4v', 'webm', 'avi', 'ts']
export const IMAGE_EXTS = ['gif', 'png', 'jpg', 'jpeg', 'webp']
</script>

<script setup>
/**
 * 服务器目录浏览（仅 admin 的数据源）。
 *
 * 抽出这份是因为它有两个使用场景：首页「新增项目 → 服务器文件」（选一个录像文件）与
 * 首页「表情包 → 添加」（选一个图片目录）。读目录永远走 `listTenantFiles`，
 * **进入下一级用 `{dir, enter}` 由服务端拼路径** —— 前端自己拼会把服务端回传的
 * `\\?\F:\a` 拼成 `\\?\F:\a/b`（Windows ERROR_INVALID_NAME 123）。
 */
import { onMounted, ref } from 'vue'
import Icon from './Icon.vue'
import { listTenantFiles } from '../stores/project'

const props = defineProps({
  /** `file` = 选文件（高亮 + 点击选中）；`dir` = 选目录（多一颗「选择当前目录」与行内「选择」） */
  mode: { type: String, default: 'file' },
  /** 初始目录；空 = 服务端给的第一个快捷入口 */
  startDir: { type: String, default: '' },
  /** `file` 模式下高亮的文件名 */
  selected: { type: String, default: '' },
  dirLabel: { type: String, default: '目录（服务器上的任意绝对路径，可留空去托管目录）' },
  dirPlaceholder: { type: String, default: '例如 F:/media/2025-09-12' },
  /** 只允许选这些扩展名（小写、不带点）；空数组 = 不限制。
   *  不匹配的文件置灰 + 点击无效 —— 「选弹幕/字幕」的场景里点中一个 mp4 毫无意义。 */
  exts: { type: Array, default: () => [] }
})
const emit = defineEmits(['update:dir', 'pick-file', 'pick-dir', 'update:listing'])

const dir = ref('')
const parent = ref('')
const roots = ref([])
const dirs = ref([])
const files = ref([])
const loading = ref(false)
const error = ref('')

function extOf(name) {
  return String(name).split('.').pop().toLowerCase()
}
function isVideoFile(name) {
  return VIDEO_EXTS.includes(extOf(name))
}
/** 图标按类型分：录像 / 图片（导入表情包时要能一眼看出哪些是图）/ 其它 */
function iconFor(name) {
  if (isVideoFile(name)) return 'film'
  return IMAGE_EXTS.includes(extOf(name)) ? 'image' : 'file'
}
/** 扩展名白名单（`props.exts` 非空时生效） */
function allowed(name) {
  const list = props.exts || []
  return list.length === 0 || list.includes(extOf(name))
}

/** 读目录。`enter` = 要进入的子目录名（服务端拼路径）。返回响应，便于「选择」一步到位。 */
async function browse(target = dir.value, enter = '') {
  loading.value = true
  error.value = ''
  try {
    const d = await listTenantFiles(target, enter)
    dir.value = d.dir || target
    emit('update:dir', dir.value)
    parent.value = d.parent || ''
    roots.value = d.roots || []
    dirs.value = d.dirs || []
    files.value = d.files || []
    // 调用方要据此判"这个目录里还有没有可用视频"（决定建项目按钮的可用性）
    emit('update:listing', { dir: dir.value, files: files.value, dirs: dirs.value })
    return d
  } catch (e) {
    dirs.value = []
    files.value = []
    emit('update:listing', { dir: dir.value, files: [], dirs: [], failed: true })
    error.value = '目录读取失败：' + e.message
    return null
  } finally {
    loading.value = false
  }
}

function enter(name) {
  browse(dir.value, name)
}

async function pickChild(name) {
  const d = await browse(dir.value, name)
  if (d?.dir) emit('pick-dir', d.dir)
}

function goUp() {
  if (parent.value) browse(parent.value)
}

onMounted(() => browse(props.startDir))
</script>
