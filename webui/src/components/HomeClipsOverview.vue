<template>
  <!-- 首页「切片总览」：**不按项目分组**，全部项目的切片卡片混排成一份总排序 ——
       卡片下方一行彩色项目名区分归属（每个项目的颜色由 id 稳定散列，永不串色）。
       两个区块：① 未导出（还没出过片/封面的切片）② 已完成导出（rendered||cover）。
       两个区块功能/样式统一：标题行即折叠开关（localStorage 持久化），
       卡片墙固定三行高、超出在墙内滚动（滚轮直翻，不截断）。
       排序：按**最后一次改动**从新到旧（`clips[].mtime` = 片段目录里最新的文件时间，
       刚剪完 / 刚精修过的排最前），同秒并列时回落项目 id 降序 + rank 升序。
       数据全部来自 workbench store 的 per-project 缓存（loadProjectData，单飞 + 失效矩阵）。 -->
  <div class="card card-blk mb-12">
    <div class="row spread mb-8">
      <b class="t" style="margin:0"><Icon name="film" :size="15" /> 切片总览</b>
      <span class="muted small">未导出 {{ unexported.length }} · 已完成导出 {{ exported.length }}</span>
    </div>

    <div v-if="wb.error" class="error-banner"><Icon name="alert" :size="15" /> {{ wb.error }}</div>

    <!-- ─────────────── ① 未导出 ─────────────── -->
    <div class="ov-sec">
      <button class="ov-sec-head ov-fold" :aria-expanded="!unexportFold" @click="toggleUnexportFold">
        <span class="ov-caret" :class="{ closed: unexportFold }">▾</span>
        <span class="chip a">未导出</span>
        <span class="muted small">点击卡片进入该片段的精修（没初始化过会先自动初始化）</span>
        <span class="muted small ov-fold-count">{{ unexported.length }}</span>
      </button>
      <template v-if="!unexportFold">
        <div v-if="!unexported.length && !loading" class="muted small ov-empty">没有未导出的切片。</div>
        <div v-if="unexported.length" ref="uWall" class="ov-wall" :class="{ noscroll: uNoScroll }">
          <button
            v-for="it in shownUnexported"
            :key="'u' + it.key"
            class="ov-card"
            :class="{ initing: initing[it.key] }"
            :title="it.title + '（' + it.proj.name + '）'"
            @click="openClip(it)"
          >
            <span class="ov-media">
              <img
                v-if="!brokenThumbs[it.key]"
                :src="it.src"
                alt=""
                loading="lazy"
                @error="markBroken(it.key)"
              >
              <span v-else class="ov-fallback"><Icon name="film" :size="16" /></span>
              <span class="ov-dur">{{ it.dur }}</span>
              <span v-if="initing[it.key]" class="ov-init"><span class="spinner"></span>初始化…</span>
            </span>
            <span class="ov-card-title">{{ it.title }}</span>
            <span class="ov-card-proj" :style="{ color: it.color }">
              <i class="ov-dot" :style="{ background: it.color }"></i>{{ it.proj.name }}
            </span>
          </button>
        </div>
        <div v-if="unexported.length > RENDER_CAP" class="muted small" style="margin-top:6px">
          卡片较多，只渲染前 {{ RENDER_CAP }} 张（共 {{ unexported.length }} 个）。
        </div>
        <div v-if="noClipProjects.length" class="muted small ov-none">
          尚无切片：{{ noClipProjects.map((p) => p.name).join('、') }}
        </div>
      </template>
    </div>

    <!-- ─────────────── ② 已完成导出（与未导出同一套折叠/三行/展开功能） ─────────────── -->
    <div class="ov-sec ov-sec-done">
      <button class="ov-sec-head ov-fold" :aria-expanded="!exportedFold" @click="toggleExportedFold">
        <span class="ov-caret" :class="{ closed: exportedFold }">▾</span>
        <span class="chip g">已完成导出</span>
        <span class="muted small">出过片或封面的切片，点击直接进入精修编辑</span>
        <span class="muted small ov-fold-count">{{ exported.length }}</span>
      </button>
      <template v-if="!exportedFold">
        <div v-if="!exported.length && !loading" class="muted small ov-empty">还没有已导出的切片。</div>
        <div v-if="exported.length" ref="eWall" class="ov-wall" :class="{ noscroll: eNoScroll }">
          <button
            v-for="it in shownExported"
            :key="'e' + it.key"
            class="ov-card"
            :class="{ initing: initing[it.key] }"
            :title="it.title + '（' + it.proj.name + '）'"
            @click="openClip(it)"
          >
            <span class="ov-media">
              <img
                v-if="!brokenThumbs[it.key]"
                :src="it.src"
                alt=""
                loading="lazy"
                @error="markBroken(it.key)"
              >
              <span v-else class="ov-fallback"><Icon name="film" :size="16" /></span>
              <span class="ov-dur">{{ it.dur }}</span>
              <span v-if="initing[it.key]" class="ov-init"><span class="spinner"></span>初始化…</span>
            </span>
            <span class="ov-card-title">{{ it.title }}</span>
            <span class="ov-card-proj" :style="{ color: it.color }">
              <i class="ov-dot" :style="{ background: it.color }"></i>{{ it.proj.name }}
            </span>
          </button>
        </div>
        <div v-if="exported.length > RENDER_CAP" class="muted small" style="margin-top:6px">
          卡片较多，只渲染前 {{ RENDER_CAP }} 张（共 {{ exported.length }} 个）。
        </div>
      </template>
    </div>

    <div v-if="loading && !allClips.length" class="row muted small" style="gap:8px">
      <span class="spinner"></span> 正在读取各项目的切片数据…
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, onUpdated, reactive, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import Icon from './Icon.vue'
import { cacheOf, invalidate, loadProjectData, loadProjects, wb } from '../stores/workbench'
import { onJobDone } from '../composables/useJobs'
import { projectData } from '../api/projectData'
import { initPolish } from '../api/polish'
import { watchJob } from '../api/jobs'
import { toast } from '../stores/toast'
import { hashIndex } from '../utils'

/** 单区块最多渲染多少张卡片进 DOM（墙本身只显示三行，超出纯 DOM 负担） */
const RENDER_CAP = 120

const router = useRouter()
const loading = computed(() => wb.loading)

/** 缩略图 404 / 解码失败的集合（key = pid:dir）：显示胶片占位，别反复重试同一张 */
const brokenThumbs = reactive({})
function markBroken(key) {
  brokenThumbs[key] = true
}

/** 折叠开关工厂：状态记 localStorage（key 持久化），toggle 时回写。 */
function useFold(key) {
  const v = ref((() => {
    try { return localStorage.getItem(key) === '1' } catch { return false }
  })())
  function toggle() {
    v.value = !v.value
    try { localStorage.setItem(key, v.value ? '1' : '0') } catch { /* 隐私模式忽略 */ }
  }
  return [v, toggle]
}
const [unexportFold, toggleUnexportFold] = useFold('yekitrim.overview.unexportFold')
const [exportedFold, toggleExportedFold] = useFold('yekitrim.overview.exportedFold')

/** 墙内滚动：内容不足三行时没有滚动条，把顶部/底部渐隐 mask 撤掉（否则观感像缺了一块） */
const uWall = ref(null)
const eWall = ref(null)
const uNoScroll = ref(false)
const eNoScroll = ref(false)

function checkOverflow() {
  const pairs = [[uWall.value, uNoScroll], [eWall.value, eNoScroll]]
  for (const [el, flag] of pairs) {
    flag.value = !!el && el.scrollHeight <= el.clientHeight + 2
  }
}
// 每次重渲染后实测一次（数据到达 / 折叠切换 / 展开切换都会走到）
onUpdated(checkOverflow)

/**
 * 项目专属颜色：id 稳定散列进调色板 —— 同一项目永远同一个颜色，新增项目也不打乱已有分配。
 * 色板挑的是在浅色底上可读的中等饱和度色，避开主题粉（--main）与错误红。
 */
const PROJECT_COLORS = [
  '#2563eb', '#7c3aed', '#0891b2', '#d97706', '#0d9488', '#9333ea',
  '#ea580c', '#059669', '#6d28d9', '#c026d3', '#4d7c0f', '#ca8a04',
  '#1d4ed8', '#b45309'
]
function colorOf(pid) {
  return PROJECT_COLORS[hashIndex(String(pid), PROJECT_COLORS.length)]
}

/**
 * 全部项目的全部切片摊平。每条：
 * - key 唯一（pid:dir），卡片 :key 与坏图集合共用；
 * - exported 与切片页「已导出」chip 同口径（rendered || cover）；
 * - color 是所属项目的专属色（卡片底部项目名那一行用）。
 */
const allClips = computed(() => {
  const out = []
  for (const p of wb.projects || []) {
    const c = cacheOf(p.id)
    const color = colorOf(p.id)
    for (const x of c.clips || []) {
      if (!x || x.video_exists === false) continue
      const m = c.polishMap?.[x.dir] || {}
      const meta = x.meta || {}
      out.push({
        key: p.id + ':' + x.dir,
        pid: p.id,
        dir: x.dir,
        name: x.name,
        proj: p,
        color,
        title: String(meta.title || x.name),
        dur: Math.round(Number(x.duration_secs) || 0) + 's',
        rank: Number(meta.rank) || 0,
        /** 「最后一次改动」的 unix 秒（服务端 `clips[].mtime`）：排序主键，见 sortedClips */
        mtime: Number(x.mtime) || 0,
        exported: !!(m.rendered || m.cover),
        src: projectData.thumbUrl(p.id, x.name)
      })
    }
  }
  return out
})

/** 总排序：按**最后一次改动**（`mtime`）从新到旧 —— 刚剪完 / 刚精修过的片段浮到最前面。
 *  同秒并列（网络盘的 mtime 精度可能就是秒）时回落项目 id 降序 + rank 升序，让顺序稳定可预期。 */
const sortedClips = computed(() =>
  allClips.value.slice().sort((a, b) => {
    if (a.mtime !== b.mtime) return b.mtime - a.mtime
    if (Number(a.pid) !== Number(b.pid)) return Number(b.pid) - Number(a.pid)
    return a.rank - b.rank
  })
)

const unexported = computed(() => sortedClips.value.filter((x) => !x.exported))
const exported = computed(() => sortedClips.value.filter((x) => x.exported))
/** DOM 渲染上限（RENDER_CAP 张），三行截断由 CSS max-height 负责 */
const shownUnexported = computed(() => unexported.value.slice(0, RENDER_CAP))
const shownExported = computed(() => exported.value.slice(0, RENDER_CAP))

/** 还没切出任何片段的项目（预处理没跑完 / 检测完还没切）：单独一行带过，不让它们从总览里消失 */
const noClipProjects = computed(() =>
  (wb.projects || []).filter((p) => !(cacheOf(p.id).clips || []).some((x) => x && x.video_exists !== false))
)

/** 正在初始化精修的卡片（key = pid:dir）：卡片盖 spinner，双击/重复点击直接忽略 */
const initing = reactive({})
/** 等初始化任务期间开着的 watchJob 订阅，组件卸载时统一关掉 */
const jobClosers = []

/** 把一次 watchJob 订阅包成 promise：终态 succeeded 才 resolve，其余按错误抛。 */
function waitJobDone(id) {
  return new Promise((resolve, reject) => {
    let settled = false
    const w = watchJob(id, {
      onDone(s) {
        if (settled) return
        settled = true
        if (s?.status === 'succeeded') resolve(s)
        else reject(new Error(s?.message || s?.error || '任务' + (s?.status || '异常结束')))
      }
    })
    jobClosers.push(() => w.close())
  })
}

function openClip(it) {
  // 统一去精修页：已初始化（polish_ready）直接进；
  // 没初始化的先跑 polish init（异步任务），成功后自动跳转。
  //
  // ⚠️ 路由参数用**目录名**（`it.name`），不是绝对路径（`it.dir`）：
  //    PolishView 拿 `route.params.clip` 去匹配 `clips[].name`（`clips` 接口里
  //    `name` = 目录名、`dir` = 绝对路径，见 `api/project_data.rs::data_clips_impl`），
  //    传绝对路径永远匹配不上 → 报「片段 … 不存在」并停在空态。
  //    用命名路由而非拼字符串：目录名可能带空格/中文/`#`，交给 router 编码。
  const toPolish = { name: 'project-polish-clip', params: { pid: it.pid, clip: it.name } }
  // `polishMap` 按**绝对目录**索引（`api/polish.rs::polish_projects_impl` 回的 `dir`），
  // 所以判就绪仍用 `it.dir`。
  const ready = !!(cacheOf(it.pid).polishMap?.[it.dir]?.polish_ready)
  if (ready) {
    router.push(toPolish)
    return
  }
  if (initing[it.key]) return
  initing[it.key] = true
  toast('正在初始化精修工程，完成后自动进入…')
  ;(async () => {
    try {
      const r = await initPolish(it.pid, it.dir)
      if (r?.queued) toast('初始化已排队：轮到时自动开始')
      await waitJobDone(r?.id)
      // 任务成功后 App.vue 的失效矩阵会刷 polishMap，这里只负责跳转
      toast('精修工程已就绪')
      router.push(toPolish)
    } catch (e) {
      toast('精修初始化失败：' + (e?.message || e), 'err')
    } finally {
      initing[it.key] = false
    }
  })()
}

// ---- 数据装载：进页面拉一次项目列表 + 每个项目的数据 ----
async function ensureAll() {
  await loadProjects()
  await Promise.allSettled((wb.projects || []).map((p) => loadProjectData(p.id)))
}

onMounted(ensureAll)

// 任务结束：App.vue 的失效矩阵已清掉对应缓存，这里重拉一次（loadProjectData 见 status=null 会再取）
const offJobDone = onJobDone((job) => {
  const pid = job?.project
  if (pid) loadProjectData(pid, { force: false }).catch(() => {})
  else ensureAll()
})

// 项目列表变化（新建/移除）时补拉新项目的数据
watch(() => (wb.projects || []).map((p) => p.id).join(','), () => {
  Promise.allSettled((wb.projects || []).map((p) => loadProjectData(p.id)))
})

onUnmounted(() => {
  offJobDone()
  // 初始化还挂在半路时离开首页：关掉订阅（任务本身在服务端继续跑完，不丢）
  while (jobClosers.length) {
    try { jobClosers.pop()() } catch { /* 已关过就忽略 */ }
  }
})
</script>

<style scoped>
/* 两个分区之间用虚线隔开 */
.ov-sec { margin-top:6px; }
.ov-sec-done { margin-top:14px; padding-top:12px; border-top:1px dashed var(--line-soft); }
.ov-sec-head { display:flex; align-items:center; gap:8px; margin-bottom:8px; }
.ov-empty { padding:2px 0 4px; }

/* 折叠开关：整行就是按钮，hover 高亮提示可点；折叠时计数还在标题上（内容藏起来但规模可见） */
.ov-fold {
  border:none; background:transparent; padding:2px 4px; margin-left:-4px;
  width:100%; font:inherit; text-align:left; border-radius:var(--r-sm);
}
.ov-fold:hover { background:var(--bg-soft); }
.ov-fold .muted { text-align:left; }
.ov-caret { display:inline-block; color:var(--ink-dim); font-size:11px;
  transition:transform .15s; flex:none; }
.ov-caret.closed { transform:rotate(-90deg); }
.ov-fold-count { margin-left:auto; flex:none; }

/* 卡片墙：高度固定三行，超出**墙内滚动**（滚轮往下翻就能看到全部，不截断、不跳页）。
   卡片尺寸写死（媒体 78 + 标题/项目名各一行定高）→ 高度可精确参与三行计算。 */
.ov-wall {
  --ov-card-h: 123px;
  --ov-gap: 10px;
  max-height: calc(3 * var(--ov-card-h) + 2 * var(--ov-gap));
  overflow-y:auto; overflow-x:hidden;
  scrollbar-width:thin;
  /* 顶部/底部渐隐提示"还有内容"（滚动后上缘遮罩自然消失观感） */
  mask-image:linear-gradient(to bottom, transparent 0, #000 14px, #000 calc(100% - 6px), transparent 100%);
  -webkit-mask-image:linear-gradient(to bottom, transparent 0, #000 14px, #000 calc(100% - 6px), transparent 100%);
}
/* 内容不足三行时 mask 的渐隐反而显得"缺了"：没滚动条就把 mask 撤掉 */
.ov-wall.noscroll {
  mask-image:none; -webkit-mask-image:none;
}

/* 卡片墙：不按项目分组，统一换行的等宽格子 */
.ov-wall {
  display:flex; flex-wrap:wrap; gap:var(--ov-gap);
}
.ov-card {
  width:140px; height:var(--ov-card-h); box-sizing:border-box;
  border:1px solid var(--line); border-radius:var(--r-sm);
  overflow:hidden; background:var(--card); padding:0; cursor:pointer; text-align:left;
  display:flex; flex-direction:column; flex:none;
}
.ov-card:hover { border-color:var(--main); box-shadow:var(--shadow); }
/* 正在自动初始化的卡片：压暗 + 盖一层 spinner，重复点击被 openClip 里的 initing 门挡掉 */
.ov-card.initing { opacity:.75; pointer-events:none; }
.ov-init {
  position:absolute; inset:0; display:flex; flex-direction:column; gap:6px;
  align-items:center; justify-content:center;
  /* ⚠️ 别写死白纱：暗色下白纱 + --ink-dim(浅灰) = 对比度 1.25。走 --veil（两套主题成对）。 */
  background:var(--veil); color:var(--ink-dim); font-size:11px; font-weight:600;
}
.ov-media { position:relative; display:block; flex:none; }
.ov-card img { display:block; width:100%; height:78px; object-fit:cover; background:var(--bg-soft); }
.ov-fallback {
  display:flex; align-items:center; justify-content:center; height:78px;
  color:var(--ink-faint); background:var(--bg-soft);
}
.ov-dur {
  position:absolute; right:3px; top:3px; padding:1px 5px; border-radius:var(--r-full);
  background:rgba(0,0,0,.55); color:#fff; font-size:10px; line-height:1.4;
}
.ov-card-title {
  display:block; padding:5px 7px 2px; font-size:11.5px; line-height:16px; color:var(--ink);
  overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-weight:600;
}
/* 项目名行：颜色随项目 —— 文字 + 前面一颗同色小圆点，扫一眼就知道归属 */
.ov-card-proj {
  display:flex; align-items:center; gap:5px; padding:0 7px 6px;
  font-size:10.5px; line-height:14px;
  overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-weight:600;
}
.ov-dot { flex:none; width:6px; height:6px; border-radius:50%; }
.ov-none { padding:6px 0 2px; }
</style>
