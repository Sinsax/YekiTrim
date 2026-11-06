<template>
  <div>
    <!-- 项目信息 + 危险操作（原先在首页项目表里，首页改版后挪到这里） -->
    <div class="card mb-12">
      <div class="row spread mb-8">
        <h2 class="m-0">项目信息</h2>
        <span v-if="project" class="chip" :class="project.missing ? 'r' : 'g'">
          {{ project.missing ? '目录缺失' : '正常' }}
        </span>
      </div>
      <div v-if="project" class="info-grid">
        <div><span class="muted small">名称</span><div>{{ project.name }}</div></div>
        <div><span class="muted small">来源</span><div>{{ sourceLabel(project) }}</div></div>
        <div><span class="muted small">阶段</span><div>{{ stageText }}</div></div>
        <div><span class="muted small">占用</span><div>{{ humanBytes(project.bytes) }}</div></div>
        <div><span class="muted small">源录播</span><div>{{ humanBytes(project.source_bytes) }}</div></div>
        <div><span class="muted small">产物</span><div>{{ humanBytes(project.output_bytes) }}</div></div>
      </div>
      <!-- 读到了列表但里面没有这个 id = 已被移除（或别人的项目）：给个明确结论，
           别一直停在"读取项目信息…"（以前这里只有读取中的文案，看起来像卡住了） -->
      <div v-else-if="listLoaded" class="muted small" title="目录与文件仍在原处，可从首页「服务器文件」重新导入">
        项目不存在或已被移除 · <router-link to="/workbench">回首页</router-link>
      </div>
      <div v-else class="muted small">读取项目信息…</div>

      <div class="row mt-12" style="gap:8px;flex-wrap:wrap">
        <router-link class="mini-btn" :to="`/p/${pid}/prepare`">去识别</router-link>
        <router-link class="mini-btn" :to="`/p/${pid}/slices`">去挑片段</router-link>
        <router-link class="mini-btn" :to="`/p/${pid}/polish`">去出片</router-link>
        <router-link class="mini-btn" to="/workbench">回首页</router-link>
        <span class="flex-1"></span>
        <button
          class="mini-btn danger"
          :disabled="busy || !project || !project.source_bytes"
          :title="project && project.source_bytes ? '删除原始录像，保留切片/字幕/事件/封面' : '没有可释放的源录播'"
          @click="removeSource"
        >删除源录播</button>
        <button class="mini-btn" :disabled="busy || !project" @click="removeProject">移除项目</button>
      </div>
      <p class="muted small mt-8" title="移除项目只注销登记：目录、yekitrim.toml、媒体与产物一个文件都不动，随时可从首页「服务器文件」重新导入。服务端永不自动删除任何数据。">
        <b>移除只注销登记，不动任何文件</b>
      </p>
    </div>

    <details class="form-adv">
      <summary>直接编辑 yekitrim.toml<span class="muted small">高级</span></summary>
      <div class="card mt-8">
        <p class="muted small">保存后阶段推导会立即刷新。</p>
        <textarea v-model="raw" rows="18" class="mono"></textarea>
        <div class="row" style="margin-top:10px">
          <button class="btn btn-primary" @click="save" :disabled="!loaded">保存</button>
          <button class="btn" @click="load">重新加载</button>
        </div>
      </div>
    </details>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { projectData } from '../api/projectData'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import { humanBytes, sourceLabel } from '../stores/project'
import { loadProjects, projectById, removeProject as apiRemoveProject, removeProjectSource } from '../stores/workbench'

const route = useRoute()
const router = useRouter()
const pid = route.params.pid || ''
const raw = ref('')
// 只有成功读到 yekitrim.toml 原文才允许保存：否则会把空文件写回去
const loaded = ref(false)
const busy = ref(false)
/** 项目列表已经取过一次（用来把"还在读"与"读到了但没有这个项目"分开）。 */
const listLoaded = ref(false)

const project = computed(() => projectById(pid))
const stageText = computed(() => {
  const s = Number(project.value?.stage || 0)
  return ['未开始', '已提取音频', '已识别声音', '已找到亮点', '已剪出片段'][Math.min(s, 4)]
})

async function load() {
  try {
    const d = await projectData.config(pid, '读取项目配置')
    raw.value = d.yekitrim_raw || ''
    loaded.value = true
  } catch (e) {
    loaded.value = false
    toast(e.message, 'err')
  }
}

async function save() {
  if (!loaded.value) { toast('配置尚未加载成功，已禁止保存', 'err'); return }
  if (!raw.value.trim()) { toast('配置内容为空，已禁止保存', 'err'); return }
  try {
    await projectData.putConfigRaw(pid, raw.value)
    toast('项目配置已保存')
  } catch (e) {
    toast(e.message, 'err')
  }
}

async function removeSource() {
  const ok = await confirmDialog(
    '删除原始录像（含全部原始分片），保留切片、字幕、事件与封面。不可恢复。',
    { title: `删除「${project.value?.name}」的源录播？`, danger: true }
  )
  if (!ok) return
  busy.value = true
  try {
    const r = await removeProjectSource(pid)
    toast(`已释放 ${humanBytes(r?.freed_bytes || 0)}`)
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    busy.value = false
  }
}

async function removeProject() {
  // 不是破坏性操作（目录里一个文件都不动），所以用普通样式 —— 红色会让人以为文件没了。
  // 但仍然二次确认：误点一下项目就从列表里消失，找回来要重新导入。
  const ok = await confirmDialog(
    '只把登记从服务器列表里去掉，不会动目录里的任何文件（yekitrim.toml、媒体、产物全部保留），随时可以重新导入。',
    { title: `移除项目「${project.value?.name}」？` }
  )
  if (!ok) return
  busy.value = true
  try {
    await apiRemoveProject(pid) // 内部已清前端本地痕迹（forgetProject）+ 刷新列表
    toast('已从列表移除（目录与文件保留在原处）')
    router.push('/workbench')
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    busy.value = false
  }
}

onMounted(async () => {
  // 项目列表失败会由 loadProjects 记在 store 的 error 里（模板里给出"项目不存在或已被移除"）
  if (!project.value) await loadProjects().catch(() => {})
  listLoaded.value = true
  load()
})
</script>

<style scoped>
button:disabled { opacity:.45; cursor:not-allowed; }
.info-grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(140px,1fr)); gap:10px; }
.info-grid > div > div { font-size:13px; }
</style>
