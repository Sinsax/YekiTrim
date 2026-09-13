<template>
  <div>
    <div class="page-head">
      <div>
        <h1>项目（{{ wb.projects.length }}）</h1>
        <div class="sub">预处理 → 切片 → 精修，侧栏任意项目×阶段一击直达</div>
      </div>
      <div class="actions">
        <button class="btn" @click="refresh"><Icon name="refresh" :size="14" /> 刷新</button>
        <button v-if="isAdmin" class="btn btn-primary" @click="emit('new-project')">
          <Icon name="plus" :size="14" /> 新建项目
        </button>
      </div>
    </div>

    <div v-if="wb.error" class="error-banner"><Icon name="alert" :size="15" /> {{ wb.error }}</div>

    <div class="card card-blk mb-12">
      <table class="tb">
        <thead>
          <tr><th>项目</th><th style="width:210px">进度</th><th style="width:96px">占用</th><th style="width:250px">操作</th></tr>
        </thead>
        <tbody>
          <tr v-for="p in wb.projects" :key="p.id" :class="{ missing: p.missing }">
            <td>
              <b class="small">{{ p.name }}</b>
              <div class="hint">{{ sourceLabel(p) }}<template v-if="p.missing"> · ⚠ 目录缺失</template></div>
            </td>
            <td>
              <div class="row">
                <div class="progress" style="width:80px"><div :style="{ width: (nextOf(p).progress / 3 * 100) + '%' }"></div></div>
                <button class="mini-btn" @click="open(p, nextOf(p).step)">{{ nextOf(p).label }}</button>
              </div>
            </td>
            <td class="num small">{{ humanBytes(p.bytes) }}</td>
            <td>
              <span class="row" style="gap:6px">
                <button class="mini-btn" @click="preview(p)">预览</button>
                <button class="mini-btn" @click="open(p, 'prepare')">打开</button>
                <button
                  v-if="isAdmin && p.source_bytes > 0"
                  class="mini-btn danger"
                  @click="removeSource(p)"
                >删除源录播</button>
              </span>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-if="!wb.projects.length && !wb.loading" class="empty mt-12">
        <div class="empty-icon"><Icon name="film" :size="24" /></div>
        还没有项目<template v-if="isAdmin">：点「新建项目」从服务器文件建一个，或配置录播姬自动入库</template>
      </div>
      <div v-else-if="wb.loading && !wb.projects.length" class="skeleton mt-12"></div>
    </div>

    <div class="card card-blk">
      <div class="row spread mb-8">
        <b class="t" style="margin:0">录制导入（最近）</b>
        <router-link class="mini-btn" to="/ingest">全部记录与导入规则</router-link>
      </div>
      <div v-if="ingestState.ready === false" class="empty">
        <div class="empty-icon"><Icon name="alert" :size="22" /></div>
        录播姬尚未接入：本机执行 <span class="mono">vtrim ingest token</span> 生成 token，
        并把录制目录填进 <span class="mono">[ingest.recorder] root</span>。
      </div>
      <template v-else>
        <SessionRow v-for="r in recent" :key="r.session_id" :r="r" @changed="reloadIngest" />
        <div v-if="!recent.length" class="hint">暂无导入记录</div>
      </template>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import Icon from '../components/Icon.vue'
import SessionRow from '../components/ingest/SessionRow.vue'
import { authState } from '../stores/auth'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import { humanBytes, sourceLabel } from '../stores/project'
import { ingestState, loadRecords, setReady } from '../stores/ingest'
import { loadProjectData, loadProjects, removeProjectSource as apiRemoveSource, uncutEventsOf, polishedCountOf, wb } from '../stores/workbench'
import { nextStep, stageInfo } from '../utils'
import { api } from '../api/client'

const emit = defineEmits(['new-project', 'preview'])
const router = useRouter()
const isAdmin = computed(() => authState.user?.role === 'admin')
const recent = computed(() => (ingestState.records || []).slice(0, 4))

/** 项目状态推导（与侧栏共用 `utils.stageInfo`；未加载明细时按 stage/has_clips 粗判）。 */
function infoOf(p) {
  const c = wb.cache[String(p.id)]
  const detail = c?.status
    ? { clips: c.clips, uncutEvents: uncutEventsOf(p.id), polishedCount: polishedCountOf(p.id) }
    : null
  return stageInfo(p, detail)
}

function nextOf(p) {
  const info = infoOf(p)
  const { label, step } = nextStep(p, null)
  return { label, step, progress: info.progress }
}

async function refresh() {
  await loadProjects()
  await reloadIngest()
}

async function reloadIngest() {
  await loadRecords('mine')
}

function open(p, step) {
  router.push(`/p/${encodeURIComponent(p.id)}/${step}`)
}

function preview(p) {
  emit('preview', p)
}

async function removeSource(p) {
  const ok = await confirmDialog(
    '删除源录播（原始视频/分片），保留切片、字幕、事件与封面。此操作不可恢复。',
    { title: `删除「${p.name}」的源录播？`, danger: true }
  )
  if (!ok) return
  try {
    const r = await apiRemoveSource(p.id)
    toast(`已释放 ${humanBytes(r?.freed_bytes || 0)}`)
  } catch (e) {
    toast(e.message, 'err')
  }
}

onMounted(async () => {
  await loadProjects()
  try {
    const h = await api('/api/health')
    setReady(h?.ingest_ready !== false)
  } catch {}
  await reloadIngest()
  // 预热前几个项目的精确计数（侧栏徽标用）
  for (const p of wb.projects.slice(0, 8)) {
    loadProjectData(p.id).catch(() => {})
  }
})
</script>

<style scoped>
.tb { width:100%; border-collapse:collapse; }
.tb th { text-align:left; font-size:11px; color:var(--ink-faint); font-weight:600; padding:4px 8px; }
.tb td { padding:8px; border-top:1px dashed var(--line-soft); vertical-align:middle; }
.tb tr.missing { opacity:.6; }
.num { text-align:right; }
.hint { font-size:11px; color:var(--ink-faint); }
</style>
