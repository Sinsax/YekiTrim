<template>
  <div>
    <div v-if="loadError" class="error-banner"><Icon name="alert" :size="15" /> {{ loadError }}</div>
    <div class="row slices-bar">
      <span class="muted small">片段操作</span>
      <button class="mini-btn" :disabled="!clipsRef" @click="clipsRef?.redetect()">重新检测</button>
      <button class="mini-btn" :disabled="!clipsRef" @click="clipsRef?.openAdd()">添加片段</button>
      <button
        v-if="uncutCount"
        class="mini-btn primary"
        :title="'把 ' + uncutCount + ' 个未剪切事件全部剪出来'"
        @click="clipsRef?.cutAll()"
      >一键全切（{{ uncutCount }}）</button>
    </div>
    <div v-if="loading" class="grid cols-3">
      <div v-for="i in 6" :key="'sk' + i" class="skeleton"></div>
    </div>
    <ProjectClipsSection
      v-else-if="project"
      ref="clipsRef"
      :project="project"
      :project-data="projectDataRef"
      :source-url="sourceUrl"
      @changed="load({ silent: true })"
    />
    <div v-else class="empty"><div class="empty-icon"><Icon name="film" :size="24" /></div>未找到项目</div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useRoute } from 'vue-router'
import { loadProjects, projectById } from '../stores/workbench'
import { projectData } from '../api/projectData'
import { uncutEventsOf, mediaUrl } from '../utils'
import { onJobDone } from '../composables/useJobs'
import Icon from '../components/Icon.vue'
import ProjectClipsSection from '../components/ProjectClipsSection.vue'

const route = useRoute()
const pid = route.params.pid || ''
const project = ref(null)
const projectDataRef = ref({ events: [], clips: [], polishMap: {} })
const clipsRef = ref(null)
const loading = ref(true)
const loadError = ref('')
// 原片 URL：未剪切事件的「预览」要播**整场录播**（那片还没剪出来），
// 与预处理页同一个来源（`status.input_file` → `mediaUrl(pid, rel)`）。
// 服务端只放行"本人项目根内"的文件 —— 项目外的原片会拿不到，那时按钮按禁用处理。
const sourceUrl = ref('')
/** `load()` 的单飞状态（见 `load` 的注释）。 */
let inflight = null
let queuedReload = false

/** 未剪切事件数（操作条的「一键全切」只在有东西可切时出现）。
 *  规则与卡片墙一致（`utils.uncutEventsOf`）：序号被**别的**事件的旧片段占着也算未剪切。 */
const uncutCount = computed(() => uncutEventsOf(projectDataRef.value?.events, projectDataRef.value?.clips).length)

/**
 * 取切片页的全部数据（事件 / 片段 / 精修状态 / 原片地址）。
 *
 * `silent`：子组件写完盘（删片段、改事件）请求刷新时用 —— 那时**不能**置 `loading`，
 * 否则 `v-if="loading"` 会把整面卡片墙换成骨架屏、顺带卸载子组件（过滤器、滚动位置全丢）。
 * 任务结束的刷新（`onJobDone`）同理，而且它**每个任务结束都跑一次**，更不能用非静默。
 *
 * **单飞 + 尾随重刷**：任务密集结束时会连着触发好几次刷新，而这里一次要打四个请求、
 * 网络盘上动辄 3–4 秒。并发跑同一批既浪费又会让旧响应盖掉新数据，所以合并成一次，
 * 期间来的请求记一笔，结束后补跑一次（与 `stores/workbench.js::loadProjectData` 的 `stale` 同款）。
 */
async function load(opts = {}) {
  if (inflight) {
    queuedReload = true
    return inflight
  }
  inflight = doLoad(opts)
  try {
    await inflight
  } finally {
    inflight = null
    if (queuedReload) {
      queuedReload = false
      load({ silent: true })
    }
  }
}

async function doLoad({ silent = false } = {}) {
  if (!silent) loading.value = true
  loadError.value = ''
  try {
    if (!projectById(pid)) await loadProjects()
    project.value = projectById(pid)
    if (!project.value) throw new Error('项目不存在或无权访问')
    const [ev, cl, pl, st] = await Promise.all([
      projectData.events(pid, '读取事件列表').catch(() => ({ events: [] })),
      projectData.clips(pid, '读取切片列表').catch(() => []),
      projectData.polishMap(pid, '读取精修状态').catch(() => []),
      projectData.status(pid, '读取项目状态').catch(() => null)
    ])
    const m = {}
    ;(pl || []).forEach(p => { m[p.dir] = p })
    projectDataRef.value = { events: (ev && ev.events) || [], clips: cl || [], polishMap: m }
    sourceUrl.value = mediaUrl(pid, st && st.input_file)
  } catch (e) {
    // 静默刷新也要把失败说出来（只是不换骨架屏）
    loadError.value = '切片数据加载失败：' + e.message
  } finally {
    if (!silent) loading.value = false
  }
}

onMounted(load)

// 剪切/管线任务多数在别的页面（预处理、精修）发起，任务结束时页面必须跟着刷新，
// 否则列表一直停在旧数据（如已切好的事件仍挂在「未剪切」里）。
//
// ⚠️ **必须 `silent`**：非静默 `load()` 会置 `loading`，模板顶上那个 `v-if="loading"`
// 就把整面卡片墙换成骨架屏并**卸载 `ProjectClipsSection`** —— 子组件一没，`clipsRef`
// 变 null，工具条三颗按钮（`:disabled="!clipsRef"`）全灰。而这一次刷新在**每个任务结束时**
// 都会跑，慢盘（RaiDrive/WebDAV）上一次要 3–4 秒：用户看到的就是"剪切完界面卡住、按钮变灰"。
// 首屏骨架屏由 `onMounted(load)` 提供，任务结束的刷新不该再来一次。
//
// **必须退订**：`router-view :key=pid` 让本视图在换项目/来回切页时反复挂载，
// 不退订就会攒下一串陈旧回调 —— 任务一结束它们同时各打一整套 project-data 请求。
const offJobDone = onJobDone((job) => {
  if (String(job.project) === String(pid)) load({ silent: true })
})
onUnmounted(offJobDone)
</script>

<style scoped>
.error-banner { margin-bottom:12px; }
.slices-bar { align-items:center; margin-bottom:10px; }
button:disabled { opacity:.45; cursor:not-allowed; }
</style>
