<template>
  <!-- 首页「切片总览」：**不按项目分组**，全部项目的切片卡片混排成一份总排序 ——
       卡片下方一行彩色项目名区分归属（每个项目的颜色由 id 稳定散列，永不串色）。
       ① **一个卡片墙、一套筛选 tag**（用户 2026-10-06：「未导出和已导出改成类似切片的那种筛选
          tag，所有切片都显示在一个区域内」）—— 以前是左右两栏各自折叠/各自滚动，
          想在两边之间找一条片段得在两个滚动区里分别翻。
       ② 顶部搜索框：关键词可命中 **标题 / 字幕 / 弹幕**（关键词落点可勾选）。
          ⚠️ 标题零请求（就在缓存里）；字幕与弹幕**按需拉**（勾上才拉对应项目，拉过就缓存）——
          12 个项目实测每个 transcript.json 170~226 KB、弹幕 XML/ASS 4~72 KB，
          进首页就全量拉等于凭空搬 2~3 MB。
       卡片墙固定三行高、超出在墙内滚动（滚轮直翻，不截断）。
       排序：按**最后一次改动**从新到旧（`clips[].mtime` = 片段目录里最新的文件时间，
       刚剪完 / 刚精修过的排最前），同秒并列时回落项目 id 降序 + rank 升序。
       数据全部来自 workbench store 的 per-project 缓存（loadProjectData，单飞 + 失效矩阵）。 -->
  <div class="card card-blk mb-12">
    <div class="row spread mb-8">
      <b class="t m-0"><Icon name="film" :size="15" /> 切片总览</b>
      <span class="muted small">共 {{ allClips.length }} 个片段 · {{ wb.projects?.length || 0 }} 个项目</span>
    </div>

    <div v-if="wb.error" class="error-banner"><Icon name="alert" :size="15" /> {{ wb.error }}</div>

    <!-- ─── 筛选 tag（对齐切片页那套：`mini-btn` + `.selected`，选中态持久化） ─── -->
    <div class="sort-bar">
      <span class="muted small">筛选</span>
      <button
        v-for="f in OV_FILTERS"
        :key="f.key"
        class="mini-btn"
        :class="{ selected: ovFilter === f.key }"
        :title="f.hint"
        @click="pickOvFilter(f)"
      >{{ f.label }}</button>
      <span class="muted small sort-count">
        {{ shownClips.length }}<template v-if="ovFilter !== 'all'">/{{ allClips.length }}</template> 个片段
      </span>
      <span class="v-sep"></span>
      <span class="muted small">排序</span>
      <button
        class="mini-btn"
        :class="{ selected: ovSort === 'mtime' }"
        :title="ovSort === 'mtime' ? '按最后一次改动（刚剪完/刚精修过的在最前）' : '按热度名次（rank 小的在前）'"
        @click="pickOvSort('mtime')"
      >最近改动</button>
      <button
        class="mini-btn"
        :class="{ selected: ovSort === 'rank' }"
        :title="ovSort === 'rank' ? '按热度名次（rank 小的在前）' : '按最后一次改动'"
        @click="pickOvSort('rank')"
      >热度名次</button>
    </div>

    <!-- ─── 搜索：关键词 + 落点勾选（标题常驻零请求；字幕/弹幕按需拉） ─── -->
    <div class="ov-search">
      <input
        v-model="query"
        class="fld-input ov-q"
        type="search"
        placeholder="搜索标题 / 字幕 / 弹幕"
        title="空格分词，几个词都命中才算；Esc 清空"
        @keyup.esc="query = ''"
      >
      <button
        v-for="s in SEARCH_IN"
        :key="s.key"
        class="mini-btn"
        :class="{ selected: searchIn[s.key] }"
        :title="s.hint"
        :disabled="s.key !== 'title' && !s.available"
        @click="toggleSearchIn(s)"
      >{{ s.label }}</button>
      <span class="muted small ov-qhint">{{ queryHint }}</span>
      <button v-if="query" class="mini-btn" title="清空搜索（Esc）" @click="query = ''">×</button>
    </div>

    <!-- ─── 一个卡片墙：所有命中的片段（不分栏） ─── -->
    <div v-if="!shownClips.length && !loading" class="muted small ov-empty">
      {{ emptyText }}
    </div>
    <div v-if="shownClips.length" ref="wall" class="ov-wall" :class="{ noscroll: noScroll }">
      <button
        v-for="it in shownClips"
        :key="it.key"
        class="ov-card"
        :class="{ initing: initing[it.key] }"
        :title="cardTitle(it)"
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
          <span
            v-if="it.segments > 1"
            class="ov-seg"
            :title="rangesHint(it.ranges)"
          >{{ it.segments }} 段</span>
          <!-- 已导出：以前靠"在哪一栏"表达，现在同一面墙，得在卡上标出来 -->
          <span v-if="it.exported" class="ov-fin" title="已导出（出过片或出过封面）">
            <Icon name="check" :size="10" />已导出
          </span>
          <!-- 命中来源：叠在缩略图**下缘**，**不进流式布局** —— 卡片是定高 flex 列
               （媒体 78 + 标题 + 项目名 = 123px），把这行塞进流里会把标题挤到裁切
               （实测 `titleClipped: true`）。叠在图上既不抢标题的行高，也更像"这条为什么在"。 -->
          <span v-if="it.hitText" class="ov-hit" :title="it.hitLabel + '：' + it.hitText">
            <b class="ov-hit-from">{{ it.hitLabel }}</b><span v-html="hl(it.hitText)"></span>
          </span>
          <span v-if="initing[it.key]" class="ov-init"><span class="spinner"></span>初始化…</span>
        </span>
        <span class="ov-card-title" v-html="hl(it.title)"></span>
        <span class="ov-card-proj" :style="{ color: it.color }">
          <i class="ov-dot" :style="{ background: it.color }"></i>{{ it.proj.name }}
        </span>
      </button>
    </div>
    <div v-if="shownClips.length > RENDER_CAP" class="muted small" style="margin-top:6px">
      卡片较多，只渲染前 {{ RENDER_CAP }} 张（共 {{ shownClips.length }} 个）。
    </div>

    <div v-if="noClipProjects.length" class="muted small ov-none">
      尚无切片：{{ noClipProjects.map((p) => p.name).join('、') }}
    </div>

    <div v-if="loading && !allClips.length" class="row muted small">
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
import { dmPlain, hashIndex, rangesHint } from '../utils'

/** 单区块最多渲染多少张卡片进 DOM（墙本身只显示三行，超出纯 DOM 负担） */
const RENDER_CAP = 120

const router = useRouter()
const loading = computed(() => wb.loading)

/** 缩略图 404 / 解码失败的集合（key = pid:dir）：显示胶片占位，别反复重试同一张 */
const brokenThumbs = reactive({})
function markBroken(key) {
  brokenThumbs[key] = true
}

// ───────────────────────── 筛选 tag（对齐切片页那套口径） ─────────────────────────
/**
 * 一级筛选：全部 / 未导出 / 已导出。
 *
 * 「已导出」的判据与切片页二级、卡片上那枚 chip **同一套**（`rendered || cover`）——
 * 别用 `polish_ready`：剪切收尾就会 `polish init`，那个字段对所有已剪片段都为真，筛不出东西。
 * 默认「全部」：以前两栏并排时"一共多少"要靠两边的数相加，现在一眼就能看全。
 */
const OV_FILTERS = [
  { key: 'all', label: '全部', hint: '未导出 + 已导出，全都显示' },
  { key: 'unexported', label: '未导出', hint: '还没出过片或封面的切片' },
  { key: 'exported', label: '已导出', hint: '已经出过片或出过封面（与卡片上的标记同口径）' }
]
const OV_FILTER_STORE = 'yekitrim.overview.filter'
const ovFilter = ref((() => {
  try {
    const v = localStorage.getItem(OV_FILTER_STORE) || ''
    if (OV_FILTERS.some((f) => f.key === v)) return v
  } catch { /* 隐私模式忽略 */ }
  return 'all'
})())
function pickOvFilter(f) {
  ovFilter.value = f.key
  try { localStorage.setItem(OV_FILTER_STORE, f.key) } catch { /* 隐私模式忽略 */ }
}

/** 排序：最近改动（默认，`clips[].mtime`）/ 热度名次（`rank` 小在前）。选中态持久化。 */
const OV_SORT_STORE = 'yekitrim.overview.sort'
const ovSort = ref((() => {
  try { return localStorage.getItem(OV_SORT_STORE) === 'rank' ? 'rank' : 'mtime' } catch { return 'mtime' }
})())
function pickOvSort(k) {
  ovSort.value = k
  try { localStorage.setItem(OV_SORT_STORE, k) } catch { /* 隐私模式忽略 */ }
}

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
        /** **多段片段**的段数（`clip.json` 的 `ranges`；>1 才在卡上打标记） */
        segments: (Array.isArray(meta.ranges) ? meta.ranges.length : 0) > 1 ? meta.ranges.length : 0,
        /** 各段源区间（按时间）：悬停 title 用 `rangesHint` 一次列全（只有多段非空） */
        ranges: (Array.isArray(meta.ranges) ? meta.ranges.length : 0) > 1 ? meta.ranges : [],
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

/** 总排序：默认按**最后一次改动**（`mtime`）从新到旧 —— 刚剪完 / 刚精修过的浮到最前面。
 *  同秒并列（网络盘的 mtime 精度可能就是秒）时回落项目 id 降序 + rank 升序，让顺序稳定可预期。
 *  切到「热度名次」时 `rank` 小的在前（`rank` 是检测排的，重新检测会整体重排）。 */
const sortedClips = computed(() =>
  allClips.value.slice().sort((a, b) => {
    if (ovSort.value === 'rank') {
      if (a.rank !== b.rank) return a.rank - b.rank
      return b.mtime - a.mtime
    }
    if (a.mtime !== b.mtime) return b.mtime - a.mtime
    if (Number(a.pid) !== Number(b.pid)) return Number(b.pid) - Number(a.pid)
    return a.rank - b.rank
  })
)

const unexported = computed(() => sortedClips.value.filter((x) => !x.exported))
const exported = computed(() => sortedClips.value.filter((x) => x.exported))

/** 还没切出任何片段的项目（预处理没跑完 / 检测完还没切）：单独一行带过，不让它们从总览里消失 */
const noClipProjects = computed(() =>
  (wb.projects || []).filter((p) => !(cacheOf(p.id).clips || []).some((x) => x && x.video_exists !== false))
)

// ───────────────────────── 搜索：标题 / 字幕 / 弹幕 ─────────────────────────
/**
 * 关键词落点。**勾了才拉数据**：
 * - `title` 常驻：标题就在 `clips[].meta.title` 里，零请求（所以它不可取消）。
 * - `subtitle` = 各项目 `output/transcript.json`（实测 170~226 KB/项目）。
 * - `danmaku` = 各项目弹幕文件（录播姬 XML / ASS，实测 4~72 KB/项目）。
 *
 * ⚠️ 勾上之后**异步拉取**（`ensureText(pid)`），拉的过程中卡片照旧显示（标题那一部分先出结果），
 * 拉完自动重筛。12 个项目全勾 ≈ 2~3 MB / 24 个请求 —— 所以**默认不勾**，也绝不进首页就拉。
 */
const SEARCH_IN = [
  { key: 'title', label: '标题', hint: '片段标题（就在缓存里，零请求）', available: true },
  { key: 'subtitle', label: '字幕', hint: '转写文本；勾上后按需读取各项目的字幕（每个 170~226 KB）', available: () => true },
  { key: 'danmaku', label: '弹幕', hint: '弹幕文本；勾上后按需读取各项目的弹幕文件（每个 4~72 KB）', available: () => true }
]
/** 勾选状态（`title` 恒真；另外两个持久化，省得每次进来重勾）。 */
const searchIn = reactive({ title: true, subtitle: false, danmaku: false })
try {
  searchIn.subtitle = localStorage.getItem('yekitrim.overview.search.subtitle') === '1'
  searchIn.danmaku = localStorage.getItem('yekitrim.overview.search.danmaku') === '1'
} catch { /* 隐私模式忽略 */ }
function toggleSearchIn(s) {
  if (s.key === 'title') return
  searchIn[s.key] = !searchIn[s.key]
  try { localStorage.setItem('yekitrim.overview.search.' + s.key, searchIn[s.key] ? '1' : '0') } catch { /* 忽略 */ }
  if (searchIn[s.key]) ensureText()
}

const query = ref('')
/** 空格分词：**几个词都命中**才算（句子里的词序与空格不必照抄）。 */
const terms = computed(() => query.value.trim().toLowerCase().split(/\s+/).filter(Boolean))

/**
 * 项目文本缓存：`pid -> { subs: string[]|null, dm: string[]|null }`。
 * `null` = 还没拉（或正在拉）；`[]` = 拉过了但没有内容。**拉过就留着**（切来切去不重复请求）。
 */
const textCache = reactive({})
/** 正在拉的 pid 集合（单飞：同一个项目不并发拉两次） */
const fetching = reactive({})

function bucket(pid) {
  if (!textCache[pid]) textCache[pid] = { subs: null, dm: null }
  return textCache[pid]
}

/**
 * 把字幕/弹幕拉进缓存。**一次请求拿两桶**（`search-text`），只在"勾了对应落点且那桶还空着"
 * 时拉 —— 两桶都齐了就一个请求都不发。
 *
 * 单飞：`fetching[pid]` 一支，同一个项目不并发拉两次（切来切去 / 勾来勾去都会走到这）。
 */
async function ensureText(only) {
  for (const pid of (wb.projects || []).map((p) => p.id)) {
    if (only != null && String(pid) !== String(only)) continue
    const b = bucket(pid)
    const wantSub = searchIn.subtitle && b.subs === null
    const wantDm = searchIn.danmaku && b.dm === null
    if ((!wantSub && !wantDm) || fetching[pid]) continue
    fetching[pid] = true
    projectData.searchText(pid, '搜索：读取字幕与弹幕')
      .then((r) => {
        if (wantSub) b.subs = subTexts(r)
        if (wantDm) b.dm = dmTexts(r)
      })
      .catch(() => {
        // 读失败按"这一桶是空的"收尾：别卡在 null 上反复重试（下次任务结束会重拉）
        if (wantSub) b.subs = []
        if (wantDm) b.dm = []
      })
      .finally(() => { fetching[pid] = false })
  }
}

/** 字幕桶的形状不定（`{subs:[…]}` / `segments[]` / 裸数组）：都认，取文本字段。 */
function subTexts(r) {
  const a = Array.isArray(r) ? r : (r?.subs || r?.segments || [])
  return a.map((s) => (typeof s === 'string' ? s : String(s?.text ?? s?.content ?? ''))).filter(Boolean)
}
/** 弹幕桶（`{dm:[…]}` / `{lines:[…]}`）：都认，取文本字段。 */
function dmTexts(r) {
  const a = Array.isArray(r) ? r : (r?.dm || r?.lines || r?.danmaku || [])
  // ⚠️ **过一遍 `dmPlain`**：弹幕文件里每条都带 `{\move(...)}`/`{\pos(...)}` 这类覆写标签
  //    （解析层有意保留，渲染要按它摆位），不摘掉的话卡片上显示的是标签串，
  //    而且搜 `move`/`pos` 会命中每一条弹幕 —— 详见 `utils.js::dmPlain`。
  return a
    .map((x) => (typeof x === 'string' ? x : String(x?.text ?? '')))
    .map(dmPlain)
    .filter(Boolean)
}

/** 当前是不是"只搜标题"（决定要不要提示"勾字幕/弹幕"）。 */
const titleOnly = computed(() => !searchIn.subtitle && !searchIn.danmaku)

/**
 * 搜索状态提示。空关键词时**只说搜索范围**（`只搜标题` / 空着不占地方）——
 * 以前这里直接渲染落点名"标题"，紧挨着那排同名按钮，看起来像凭空多出一个按钮。
 */
const queryHint = computed(() => {
  if (!terms.value.length) return titleOnly.value ? '只搜标题' : ''
  if (Object.keys(fetching).some((k) => fetching[k])) return '读取中…'
  return ''
})

/**
 * 一条片段对关键词的命中情况。
 * - `title` 命中 ⇒ 直接算（`hitText` 留空，标题本来就在卡上）。
 * - 字幕 / 弹幕命中 ⇒ 返回 `hitText`（卡片上那行小字）+ `hitFrom`（字幕/弹幕）+ `hitCount`。
 *
 * ⚠️ **`hitCount` 不是装饰**：弹幕天生高度重复（实测一个词命中 19 个片段、每个片段都是同一句
 * 弹幕），只显示"首个命中行"的话整面墙看起来一模一样、分不出哪条更值得点。
 * 带上"命中 N 条"至少让"这句被刷屏了"和"只提了一句"这两件事能分辨。
 * 弹幕/字幕条数多，命中数封顶显示（见 `hitLabel`），避免一行里塞三位数。
 */
function matchOf(it) {
  const ts = terms.value
  if (!ts.length) return { hit: true, hitText: '', hitFrom: '', hitCount: 0 }
  const hay = (s) => String(s || '').toLowerCase()
  if (!ts.every((t) => hay(it.title).includes(t))) {
    // 标题不全中：再看字幕 / 弹幕（逐行扫，记条数、留第一条当样张）
    const b = textCache[it.pid] || {}
    for (const [on, arr, from] of [[searchIn.subtitle, b.subs, '字幕'], [searchIn.danmaku, b.dm, '弹幕']]) {
      if (!on || !arr) continue
      let first = ''
      let n = 0
      for (const line of arr) {
        const l = hay(line)
        if (ts.every((t) => l.includes(t))) {
          if (!first) first = line
          n += 1
        }
      }
      if (n) return { hit: true, hitText: first, hitFrom: from, hitCount: n }
    }
    return { hit: false, hitText: '', hitFrom: '', hitCount: 0 }
  }
  return { hit: true, hitText: '', hitFrom: '', hitCount: 0 }
}

/** 命中行的前缀读数：`弹幕 ×19`（单条就不写 ×N，别制造噪声）。 */
function hitLabel(m) {
  if (!m.hitFrom) return ''
  return m.hitCount > 1 ? `${m.hitFrom} ×${m.hitCount}` : m.hitFrom
}

/** 命中关键词的片段（按一级筛选收窄后，再按搜索收窄）。 */
const filtered = computed(() => {
  const k = ovFilter.value
  let list = k === 'unexported' ? unexported.value : k === 'exported' ? exported.value : sortedClips.value
  if (terms.value.length) list = list.filter((x) => matchOf(x).hit)
  return list
})

/** 卡片要用的命中信息（`matchOf` 每张卡只算一次，模板里别重复调）。
 *  ⚠️ **搜索时把"命中多"的排前面**：同一句弹幕被刷屏的片段（命中 19 条）与只提一次的片段，
 *  含义完全不同；按命中数降序能把前者顶上来，再回落到常规排序（最近改动 / 热度名次）。
 *  没在搜索时不动排序 —— 别让"命中数"这条键在空关键词下也参与。 */
const cards = computed(() => {
  const list = filtered.value.map((x) => {
    const m = terms.value.length ? matchOf(x) : { hitText: '', hitFrom: '', hitCount: 0 }
    return { ...x, hitText: m.hitText, hitLabel: hitLabel(m), hitCount: m.hitCount }
  })
  if (terms.value.length) list.sort((a, b) => b.hitCount - a.hitCount)
  return list
})

/** DOM 渲染上限（RENDER_CAP 张），三行截断由 CSS max-height 负责 */
const shownClips = computed(() => cards.value.slice(0, RENDER_CAP))

const emptyText = computed(() => {
  if (terms.value.length) {
    return titleOnly.value
      ? '没有命中标题的片段（勾上「字幕」「弹幕」可以搜转写与弹幕）'
      : '没有命中的片段'
  }
  if (ovFilter.value === 'unexported') return '没有未导出的切片。'
  if (ovFilter.value === 'exported') return '还没有已导出的切片。'
  return '还没有切片。'
})

/**
 * 关键词高亮（`v-html`）：**HTML 转义在前**，再把命中的词包 `<mark>`。
 * ⚠️ 先转义再拼标签，别反过来 —— 字幕/弹幕文本来自用户文件，里面可能有 `<`。
 * 命中素材要高亮在哪：搜到的字幕行原样显示在卡上，把命中的词点亮才看得出"为什么是它"。
 */
function hl(s) {
  const raw = String(s ?? '')
  const esc = raw.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]))
  const ts = terms.value.slice().sort((a, b) => b.length - a.length)
  if (!ts.length) return esc
  let out = esc
  for (const t of ts) {
    if (!t) continue
    const re = new RegExp('(' + t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + ')', 'gi')
    out = out.replace(re, '<mark>$1</mark>')
  }
  return out
}

/** 卡片悬停 title：片段名（项目）+（多段时的各段区间由 `.ov-seg` 自己带）。 */
function cardTitle(it) {
  return it.title + '（' + it.proj.name + '）'
}

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
  // 上次勾过字幕/弹幕就直接补拉（偏好是持久的，搜索结果也该是）
  ensureText()
}

onMounted(ensureAll)

// 任务结束：App.vue 的失效矩阵已清掉对应缓存，这里重拉一次（loadProjectData 见 status=null 会再取）
const offJobDone = onJobDone((job) => {
  const pid = job?.project
  if (pid) loadProjectData(pid, { force: false }).catch(() => {})
  else ensureAll()
  // ⚠️ **文本缓存要跟着失效**：字幕/弹幕刚改过（重新转写 / 换了弹幕文件），
  //    留着老文本会继续按旧内容命中 —— 只丢那一个项目的两个桶，别整份清掉。
  if (pid) {
    const b = bucket(pid)
    b.subs = null
    b.dm = null
    ensureText(pid)
  }
})

// 项目列表变化（新建/移除）时补拉新项目的数据
watch(() => (wb.projects || []).map((p) => p.id).join(','), () => {
  Promise.allSettled((wb.projects || []).map((p) => loadProjectData(p.id)))
  ensureText()
})

// 勾选变化（另一个入口在 toggleSearchIn 里）→ 补拉
watch(() => [searchIn.subtitle, searchIn.danmaku].join(','), () => {
  if (!titleOnly.value) ensureText()
})

/** 墙内滚动：内容不足三行时没有滚动条，把顶部/底部渐隐 mask 撤掉（否则观感像缺了一块） */
const wall = ref(null)
const noScroll = ref(false)

function checkOverflow() {
  const el = wall.value
  noScroll.value = !!el && el.scrollHeight <= el.clientHeight + 2
}
// 每次重渲染后实测一次（数据到达 / 筛选切换 / 搜索收窄都会走到）
onUpdated(checkOverflow)

onUnmounted(() => {
  offJobDone()
  // 初始化还挂在半路时离开首页：关掉订阅（任务本身在服务端继续跑完，不丢）
  while (jobClosers.length) {
    try { jobClosers.pop()() } catch { /* 已关过就忽略 */ }
  }
})
</script>

<style scoped>
/* 筛选/搜索那两行：与切片页 `.sort-bar` 同一套观感（mini-btn + .selected） */
.sort-bar { display:flex; align-items:center; flex-wrap:wrap; gap:6px; margin-bottom:8px; }
.sort-count { margin-left:auto; flex:none; }
.v-sep { width:1px; height:16px; background:var(--line-soft); margin:0 4px; flex:none; }

/* 搜索行：输入框 + 落点勾选 + 就绪提示，一条紧凑横排（别做成两行占高度） */
.ov-search { display:flex; align-items:center; gap:6px; margin-bottom:10px; flex-wrap:wrap; }
.ov-q { max-width:320px; }
.ov-qhint { flex:none; }
/* 命中高亮：`<mark>` 走主题粉，别用浏览器默认黄（暗色下刺眼且与主题无关）。
   ⚠️ 命中行是**深色叠层**上的白字，`mark` 要单独给一套（粉底 + 浅字），
   否则深底 + 深字或浅底 + 白字会读不出来。 */
.ov-card-title :deep(mark) {
  background:var(--main-soft); color:var(--pink-text); border-radius:2px; padding:0 1px;
}
.ov-hit :deep(mark) {
  background:var(--pink-light); color:#2a2522; border-radius:2px; padding:0 1px;
}

/* 卡片墙：高度固定三行，超出**墙内滚动**（滚轮往下翻就能看到全部，不截断、不跳页）。
   卡片尺寸写死（媒体 78 + 标题/项目名各一行定高）→ 高度可精确参与三行计算。 */
.ov-wall {
  --ov-card-h: 123px;
  --ov-gap: 10px;
  max-height: calc(3 * var(--ov-card-h) + 2 * var(--ov-gap));
  overflow-y:auto; overflow-x:hidden;
  scrollbar-width:thin;
  display:flex; flex-wrap:wrap; gap:var(--ov-gap);
  /* 顶部/底部渐隐提示"还有内容"（滚动后上缘遮罩自然消失观感） */
  mask-image:linear-gradient(to bottom, transparent 0, #000 14px, #000 calc(100% - 6px), transparent 100%);
  -webkit-mask-image:linear-gradient(to bottom, transparent 0, #000 14px, #000 calc(100% - 6px), transparent 100%);
}
/* 内容不足三行时 mask 的渐隐反而显得"缺了"：没滚动条就把 mask 撤掉 */
.ov-wall.noscroll {
  mask-image:none; -webkit-mask-image:none;
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
/* 「N 段」标记（多段片段）：叠在时长下方 */
.ov-seg {
  position:absolute; right:3px; top:20px; padding:0 6px; border-radius:var(--r-full);
  background:var(--main-soft); color:var(--pink-text); font-size:10px; line-height:16px;
}
/* 「已导出」标记：现在所有片段在同一面墙上，得靠这枚标区分（以前靠"在哪一栏"） */
.ov-fin {
  position:absolute; left:3px; top:3px; display:inline-flex; align-items:center; gap:3px;
  padding:1px 6px; border-radius:var(--r-full);
  background:var(--green-soft); color:var(--green-deep); font-size:10px; line-height:1.4;
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
/* 命中素材那一行（搜字幕/弹幕时才有）：**叠在缩略图下缘**，不进流式布局 ——
   卡片定高 123px（媒体 78 + 标题 16 + 项目名 14），塞进流里会把标题挤到裁切
   （实测 `titleClipped: true`，几何探针钉着这条）。 */
.ov-hit {
  position:absolute; left:0; right:0; bottom:0; box-sizing:border-box;
  padding:2px 5px; font-size:10px; line-height:14px;
  color:#fff; background:rgba(0,0,0,.62);
  overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
}
.ov-hit-from { color:var(--pink-light); margin-right:4px; font-weight:600; }
.ov-none { padding:6px 0 2px; }
.ov-empty { padding:2px 0 4px; }
</style>
