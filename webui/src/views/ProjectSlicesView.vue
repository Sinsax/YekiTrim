<template>
  <div>
    <div v-if="loadError" class="error-banner"><Icon name="alert" :size="15" /> {{ loadError }}</div>

    <!-- 真·空页（没事件也没片段）：一句人话 + 两条路。
         卡片墙用 `v-show` 藏着而**不卸载** —— 手动添加的弹窗住在 `ProjectClipsSection` 里，
         卸载了 `clipsRef` 就变 null，那颗「手动添加」就点不动了。 -->
    <AppEmptyState
      v-if="showEmptyAll"
      icon="scissors"
      title="还没有可剪的片段"
      description="先在「识别」页查找事件，再回来剪片段"
      primary="去识别"
      :primary-to="`/p/${pid}/prepare`"
      secondary="手动添加"
      @secondary="openAddClip"
    />

    <div v-show="!showEmptyAll">
      <!-- 操作条：一个主行动 + 一个「⋯ 更多」。
           ⚠️ `slices.redetect` / `slices.add-clip` 是**高级项**，必须留在 `<details>` 里
           （`webui/scripts/check-simplify.mjs` 校验 advancedOnly 的锚点位置），别搬回一级。 -->
      <div class="row slices-bar">
        <button
          v-if="uncutCount > 0"
          class="btn btn-primary"
          data-cap="slices.cut-all"
          :title="`把 ${uncutCount} 个还没剪出来的事件一次剪成片段`"
          @click="clipsRef?.cutAll()"
        >一键剪出全部（{{ uncutCount }}）</button>
        <details ref="moreRef" class="more-menu" @keydown.esc="closeMore">
          <summary class="mini-btn" title="更多片段操作">⋯ 更多</summary>
          <div class="more-pop">
            <button class="mini-btn" data-cap="slices.redetect" :disabled="!clipsRef" @click="closeMore(); clipsRef?.redetect()">重新检测</button>
            <button class="mini-btn" data-cap="slices.add-clip" :disabled="!clipsRef" @click="closeMore(); clipsRef?.openAdd()">添加片段</button>
          </div>
        </details>
      </div>

      <!-- 全部剪完了：下一步只由这条 NextStepBar 承担（操作条不再重复一颗「去出片」，
           一页一个主按钮）。data-cap 挂在这里，契约里仍是 slices.go-polish。 -->
      <NextStepBar
        v-if="clipCount > 0 && uncutCount === 0"
        data-cap="slices.go-polish"
        :title="`已剪出 ${clipCount} 个片段`"
        desc="做智能剪辑、运镜与封面"
        cta="去出片"
        :to="`/p/${pid}/polish`"
      />

      <UiSkeleton v-if="loading" variant="grid" :count="6" />
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
import AppEmptyState from '../components/AppEmptyState.vue'
import NextStepBar from '../components/NextStepBar.vue'
import ProjectClipsSection from '../components/ProjectClipsSection.vue'
import UiSkeleton from '../components/ui/UiSkeleton.vue'

const route = useRoute()
const pid = route.params.pid || ''
const project = ref(null)
const projectDataRef = ref({ events: [], clips: [], polishMap: {} })
const clipsRef = ref(null)
/** 「⋯ 更多」弹层：原生 `<details>` 不会自己关 —— Esc 与点击外部都要收（否则它一直浮在卡片墙上） */
const moreRef = ref(null)
function closeMore() {
  if (moreRef.value) moreRef.value.open = false
}
function onDocPointerDown(e) {
  if (moreRef.value?.open && !moreRef.value.contains(e.target)) closeMore()
}
const loading = ref(true)
const loadError = ref('')
// 原片 URL：未剪切事件的「预览」要播**整场录播**（那片还没剪出来），
// 与预处理页同一个来源（`status.input_file` → `mediaUrl(pid, rel)`）。
// 服务端只放行"本人项目根内"的文件 —— 项目外的原片会拿不到，那时按钮按禁用处理。
const sourceUrl = ref('')
/** `load()` 的单飞状态（见 `load` 的注释）。 */
let inflight = null
let queuedReload = false

/** 未剪切事件数（操作条的「一键剪出全部」只在有东西可剪时出现）。
 *  规则与卡片墙一致（`utils.uncutEventsOf`）：序号被**别的**事件的旧片段占着也算未剪切。 */
const uncutCount = computed(() => uncutEventsOf(projectDataRef.value?.events, projectDataRef.value?.clips).length)
/** 已剪出的片段数（「去出片（N）」与「已剪出 N 个片段」都用它）。 */
const clipCount = computed(() => (projectDataRef.value?.clips || []).length)
/** 检测到的事件数：一个都没有 = 这一页还没有"可剪的片段"。 */
const eventCount = computed(() => (projectDataRef.value?.events || []).length)
/** 真·空页：没事件也没片段。读失败时**不出空态**（让上面的错误条说话，别伪装成"没东西可剪"）。 */
const showEmptyAll = computed(() =>
  !loading.value && !loadError.value && !!project.value && !clipCount.value && !eventCount.value
)

/** 空态里的「手动添加」：弹窗住在卡片墙组件里，借它的 `openAdd()` 打开。 */
function openAddClip() {
  clipsRef.value?.openAdd()
}

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
    // ⚠️ **事件 / 切片 / 精修状态读失败 ⇒ 整页走错误条，不许兜成空数组**（2026-10-30 修）：
    //    这里原来各有 `.catch(() => 空)`，于是接口一挂，卡片墙就显示「还没有片段 / 全部未剪切」——
    //    和"真的没剪过"长得一模一样，用户会以为切片丢了、或者再去剪一遍。
    //    现在让 Promise.all 直接 reject ⇒ 下面 catch 置 `loadError`，**上一次的数据原样留着** + 错误条。
    //    ⚠️ 状态（原片地址）是附带的：它挂了只影响「预览整场录播」那颗按钮，
    //    不值得把整面墙判成失败；而且 `api()` 自己已经把失败写进控制台与运行日志了。
    // ⚠️ 用 allSettled 而不是 all（2026-11-15）：四个接口里任何一个失败都会让
    // Promise.all 整体 reject，**连已经读到的数据也一起丢掉** —— 实测症状是
    // 「切片数据加载失败：暂无 clips」，而事件其实读到了、只是被同一个 catch 吞了。
    // 现在：**事件与片段是主数据**（任一失败才报错），精修状态与项目状态是辅助（失败降级）。
    const [evR, clR, plR, stR] = await Promise.allSettled([
      projectData.events(pid, '读取事件列表'),
      projectData.clips(pid, '读取切片列表'),
      projectData.polishMap(pid, '读取精修状态'),
      projectData.status(pid, '读取项目状态')
    ])
    const primaryFail = [evR, clR].find((r) => r.status === 'rejected')
    if (primaryFail) throw primaryFail.reason
    const ev = evR.value
    const cl = clR.value
    const pl = plR.status === 'fulfilled' ? plR.value : []
    const st = stR.status === 'fulfilled' ? stR.value : null
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
onMounted(() => document.addEventListener('pointerdown', onDocPointerDown))

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
onUnmounted(() => document.removeEventListener('pointerdown', onDocPointerDown))
</script>

<style scoped>
.error-banner { margin-bottom:12px; }
.slices-bar { align-items:center; gap:8px; margin-bottom:10px; flex-wrap:wrap; }
button:disabled { opacity:.45; cursor:not-allowed; }
/* 「⋯ 更多」= 二级操作披露：贴右边，弹层浮在上层、不挤动操作条与卡片墙 */
.more-menu { position:relative; margin-left:auto; }
.more-menu > summary { list-style:none; cursor:pointer; user-select:none; }
.more-menu > summary::-webkit-details-marker { display:none; }
.more-menu[open] > summary { color:var(--pink-text); border-color:var(--main); }
.more-pop {
  position:absolute; right:0; top:calc(100% + 6px); z-index:20;
  display:flex; flex-direction:column; gap:6px; min-width:104px; padding:8px;
  background:var(--card); border:1px solid var(--line); border-radius:var(--r-sm);
  box-shadow:0 8px 22px rgba(0,0,0,.14);
}
.more-pop .mini-btn { justify-content:flex-start; white-space:nowrap; }
</style>
