<template>
  <aside class="side" :class="{ mini: collapsed }" aria-label="主导航">
    <div class="side-brand">
      <button class="brand-home" title="回到工作台" aria-label="YekiTrim 首页" @click="go('/workbench')">
        <span class="brand-logo">
          <!-- 品牌标：与登录页同一份正式 logo（src/assets/logo.png）。不要再改回内联 SVG。 -->
          <img :src="logoUrl" alt="" />
        </span>
        <span v-if="!collapsed" class="brand-copy">
          <span class="brand-name">YekiTrim</span>
          <span class="brand-sub">切片工作台</span>
        </span>
      </button>
      <button class="side-toggle" :title="collapsed ? '展开侧栏' : '折叠侧栏'"
              :aria-label="collapsed ? '展开侧栏' : '折叠侧栏'" @click.stop="toggleSide">
        <Icon :name="collapsed ? 'chevron-right' : 'chevron-left'" :size="14" />
      </button>
      <button class="mobile-close" aria-label="关闭导航" title="关闭导航" @click="emit('close-mobile')">
        <Icon name="x" :size="16" />
      </button>
    </div>

    <template v-if="!collapsed">
      <div class="side-filter">
        <Icon name="search" :size="13" />
        <input
          v-model.trim="wb.filter"
          aria-label="过滤项目"
          placeholder="过滤项目…"
          autocomplete="off"
          @keydown.esc="wb.filter = ''"
        />
        <button v-if="wb.filter" class="filter-clear" aria-label="清空过滤" title="清空过滤" @click="wb.filter = ''">
          <Icon name="x" :size="11" />
        </button>
      </div>

      <div class="side-list">
        <div class="side-cap">项目（{{ filteredProjects.length }}）</div>
        <div v-if="wb.loading && !wb.projects.length" class="hint">加载中…</div>
        <div v-else-if="!filteredProjects.length && wb.projects.length" class="hint">无匹配项目</div>
        <!-- 空态**做成按钮**（2026-11-14 用户口径）：虚线框 + 加号、整条可点。
             原来是一行带下划线的文字，看着不像能点。 -->
        <router-link v-else-if="!filteredProjects.length" class="empty-add" to="/workbench">
          <Icon name="plus" :size="12" />
          <span>去添加第一个项目</span>
        </router-link>
        <!-- 分组：项目多了以后"最近 / 更早"比一整条长列表好扫。
             过滤时不分组（用户已经在收窄了，再加标题只是噪声）。 -->
        <template v-for="g in groups" :key="g.key">
          <div v-if="g.label" class="side-subcap">{{ g.label }}<span class="side-subcap-n">{{ g.items.length }}</span></div>
          <ProjectNode
            v-for="p in g.items"
            :key="p.id"
            :p="p"
            :expanded="isExpanded(p.id)"
            :current="String(p.id) === String(route.params.pid)"
            :viewing="viewingStep"
            :running="runningOf(p.id)"
            @toggle="toggleExpand"
            @open="(e) => emit('open', e)"
            @remove="(p) => emit('remove', p)"
          />
        </template>
      </div>
    </template>

    <!-- 折叠态：项目标识图标轨（过滤数字/符号，多项目去重；悬停看全名，点击直达当前阶段） -->
    <div v-else class="rail-list">
      <button
        v-if="wb.filter"
        class="rail-item rail-filter"
        :title="`正在过滤：${wb.filter}（展开侧栏可修改）`"
        @click="toggleSide"
      >
        <Icon name="search" :size="13" />
        <span>{{ filteredProjects.length }}</span>
      </button>
      <button v-for="p in filteredProjects" :key="p.id" class="rail-proj"
              :class="{ on: String(p.id) === String(route.params.pid) }"
              :title="`${p.name} · ${runningOf(p.id) ? '运行中' : stepLabelOf(p)}`"
              @click="openProject(p)">
        <span class="ini">{{ projectLabelOf(p) }}</span>
        <span v-if="runningOf(p.id)" class="rdot"></span>
      </button>
      <div v-if="!filteredProjects.length" class="hint rail-hint"
           :title="wb.projects.length ? '过滤条件把项目都藏起来了' : '还没有项目'">·</div>
    </div>

    <div class="side-foot">
      <!-- 用户信息只做展示（点它不再登出，用户要求）；登出是右边那个按钮，且要二次确认。
           ⚠️ **本地模式（`--local-no-auth`）下整块换掉**：那里没有"某个人"可显示 —— 那个 `admin`
           只是内部账号，用户也根本没"登录"进来；会话也不存在，所以没有可退的东西。
           用户口径：本地模式**不显示 admin、不显示图标**，只留一句状态。 -->
      <div v-if="!collapsed" class="user-chip" :class="{ local: localNoAuth }">
        <span v-if="localNoAuth" class="local-mark" title="本地模式：免登录，仅本机可访问">本地模式</span>
        <template v-else>
          <span class="av">{{ initialOf(authState.user?.username || '?') }}</span>
          <span class="nm">
            <b>{{ authState.user?.username }}</b>
            <span class="muted small">{{ isAdmin ? '管理员' : '用户' }}</span>
          </span>
          <button class="mini-btn side-logout" title="退出登录（会再确认一次）" @click="emit('logout')">
            <Icon name="logout" :size="12" /> 退出
          </button>
        </template>
      </div>
      <button v-else class="rail-item rail-user"
              :title="localNoAuth
                ? '本地模式（免登录，仅本机可访问）'
                : `${authState.user?.username}（${isAdmin ? '管理员' : '用户'}）· 退出登录`"
              @click="!localNoAuth && emit('logout')">
        <!-- 折叠轨里同样不画头像/图标，用一个短标代替（64px 轨放得下） -->
        <span v-if="localNoAuth" class="local-mark">本地</span>
        <template v-else>
          <span class="av">{{ initialOf(authState.user?.username || '?') }}</span>
          <Icon name="logout" :size="14" class="rail-out" />
        </template>
      </button>

      <template v-if="!collapsed">
        <button
          v-for="e in entries"
          :key="e.key"
          class="foot-item"
          :class="{ on: current === e.key }"
          :aria-current="current === e.key ? 'page' : undefined"
          @click="go(e.path)"
        >
          <Icon :name="e.icon" :size="14" />
          {{ e.label }}
        </button>
        <!-- 主题切换：浅色（哥特浅色）/ 暗色（哥特暗色） -->
        <button class="foot-item theme-toggle" :title="themeTitle" @click="toggleTheme()">
          <Icon :name="ui.theme === 'dark' ? 'moon' : 'sun'" :size="13" />
          {{ ui.theme === 'dark' ? '暗色' : '浅色' }}
          <span v-if="!ui.themeIsManual" class="muted small">跟随系统</span>
        </button>
      </template>
      <template v-else>
        <button v-for="e in entries" :key="e.key" class="rail-item" :class="{ on: current === e.key }"
                :title="e.label" @click="go(e.path)">
          <Icon :name="e.icon" :size="16" />
        </button>
        <button class="rail-item" :title="themeTitle" @click="toggleTheme()">
          <Icon :name="ui.theme === 'dark' ? 'moon' : 'sun'" :size="16" />
        </button>
      </template>
    </div>
  </aside>
</template>

<script setup>
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import logoUrl from '../../assets/logo.png'
import ProjectNode from './ProjectNode.vue'
import Icon from '../Icon.vue'
import { authState } from '../../stores/auth'
import { useJobs } from '../../composables/useJobs'
import { ui, toggleSide } from '../../stores/ui'
import { toggleTheme } from '../../stores/theme'
import { STEPS, stageInfo, hashIndex } from '../../utils'
import { filteredProjects, isExpanded, toggleExpand, wb } from '../../stores/workbench'

const props = defineProps({
  projectCount: { type: Number, default: 0 },
  /** 移动抽屉打开时临时使用完整导航，不改写用户的桌面折叠偏好。 */
  mobileOpen: { type: Boolean, default: false },
  /** 本地免登录（`/api/health.local_no_auth`）：没有会话可退，收起「退出」入口 */
  localNoAuth: { type: Boolean, default: false }
})
const emit = defineEmits(['open', 'remove', 'logout', 'open-jobs', 'close-mobile'])

const route = useRoute()
const router = useRouter()
const { jobs } = useJobs()

/** 折叠态：只留品牌 / 项目标识 / 全局入口图标（状态在 stores/ui.js，localStorage 持久化）
 *  ⚠️ 移动抽屉打开时**强制展开**：抽屉只有 304px 宽，图标轨那 64px 在那里没有意义；
 *     这里只影响渲染，**不回写** `yekitrim.ui.sideCollapsed`（桌面偏好不许被手机端改掉）。 */
const collapsed = computed(() => ui.sideCollapsed && !props.mobileOpen)

/** 主题切换按钮的提示：明说当前档位与"是否跟随系统" */
const themeTitle = computed(() => {
  const cur = ui.theme === 'dark' ? '暗色' : '浅色'
  const tail = ui.themeIsManual ? '点这里切换' : '当前跟随系统，点这里固定'
  return `主题：${cur}（${tail}）`
})

const isAdmin = computed(() => authState.user?.role === 'admin')
const current = computed(() => {
  if (route.name === 'workbench') return 'home'
  if (route.name === 'ingest') return 'ingest'
  // 设置页拆成了子页（settings-config / settings-llm…）：认 meta 标记，别写死路由名
  if (route.meta?.settings) return 'settings'
  return 'project'
})
const viewingStep = computed(() => (route.name?.startsWith('project-') ? String(route.name).split('-')[1] : ''))

/** 全局入口列表（role 过滤；存储/管理属于 P5，未实现前不显示假入口）。icon 供折叠轨用。 */
const entries = computed(() => {
  const list = [
    { key: 'home', label: '首页', path: '/workbench', icon: 'home' },
    { key: 'settings', label: '设置', path: '/settings', icon: 'settings', admin: true }
  ]
  return list.filter((e) => !e.admin || isAdmin.value)
})

/**
 * 项目 → 正在跑的任务（任务带 project 标签）。
 *
 * 并行之后一个项目可能同时有好几条（比如 clip-01 在渲染、clip-02 在跑 LLM）：
 * 取第一条显示进度，另外用 `extra` 标注"还有 N 条"，别让侧栏看起来只有一个任务。
 */
function runningOf(pid) {
  const list = (jobs.value || []).filter(
    (x) => (x.status === 'running' || x.status === 'queued') && String(x.project) === String(pid)
  )
  if (!list.length) return null
  const j = list[0]
  return { kind: j.kind, current: j.current || 0, total: j.total || 0, extra: list.length - 1 }
}

/** 项目首字（emoji/中文都按字符取，不用 charAt 以免切开代理对）。 */
function initialOf(s) {
  const ch = Array.from(String(s || '').trim())[0] || '·'
  return ch.toUpperCase()
}

/**
 * 折叠轨的项目标识：显示两个字，只取“有意义”的字符，过滤掉 `[` `]` `-` `:` 数字这些。
 *
 * 录播姬项目名通常是 `[2026-08-13 20-54-35-962][好困]`：
 * 过滤后只剩 `好困`，取末尾相邻两个字（标签通常在末尾），所以折叠后显示 `好困`。
 *
 * 多项目去重按项目 id 固定顺序分配，先到先得；同一项目永远拿同一个标识。
 */
const PROJECT_FALLBACK_CHARS = [
  '春', '夏', '秋', '冬', '东', '南', '西', '北',
  '风', '花', '雪', '月', '星', '海', '山', '云',
  '林', '川', '光', '影', '木', '水', '火', '土'
]
/** Unicode 字母（含 CJK）才算有意义；数字/标点/符号/emoji 都过滤掉。 */
const MEANINGFUL_CHAR_RE = /[\p{L}\p{Nl}]/u

function projectNameChars(name) {
  return Array.from(String(name || '')).filter((ch) => MEANINGFUL_CHAR_RE.test(ch))
}

/** 标题里的候选两字标识：末尾相邻两字优先，再往前滑。 */
function projectLabelCandidates(p) {
  const chars = projectNameChars(p.name)
  if (chars.length < 2) return []
  const out = []
  for (let i = chars.length - 2; i >= 0; i--) {
    out.push(chars[i] + chars[i + 1])
  }
  return out
}

/** 标题没有足够可用字或候选全被占时，从固定字池里按 id 稳定挑一个两字组合。 */
function fallbackProjectLabel(seed, used) {
  const n = PROJECT_FALLBACK_CHARS.length
  const total = n * n
  const start = hashIndex(seed, total)
  for (let k = 0; k < total; k++) {
    const idx = (start + k) % total
    const label = PROJECT_FALLBACK_CHARS[Math.floor(idx / n)] + PROJECT_FALLBACK_CHARS[idx % n]
    if (!used.has(label)) return label
  }
  return '项目'
}

const projectLabels = computed(() => {
  const map = {}
  const used = new Set()
  // 按 id 排序而不是按当前过滤/排序结果：列表顺序变化时每个项目的标识不能跟着变。
  const list = [...(wb.projects || [])].sort((a, b) => Number(a.id) - Number(b.id))
  for (const p of list) {
    let label = ''
    for (const c of projectLabelCandidates(p)) {
      if (!used.has(c)) {
        label = c
        break
      }
    }
    if (!label) label = fallbackProjectLabel(String(p.id ?? p.name ?? ''), used)
    used.add(label)
    map[String(p.id)] = label
  }
  return map
})

function projectLabelOf(p) {
  return projectLabels.value[String(p.id)] || '项目'
}

/** 粗粒度当前阶段（不看缓存，够图标轨用）。 */
function stepOf(p) {
  const pr = stageInfo(p).progress
  return pr >= 3 ? 'polish' : pr === 2 ? 'slices' : 'prepare'
}

function stepLabelOf(p) {
  return STEPS.find((s) => s.key === stepOf(p))?.name || '预处理'
}

/** 侧栏分组：按"加入时间"分最近 / 更早。30 天是拍的 —— 再细分只会让标签比项目还多。 */
const GROUPS_RECENT_DAYS = 30
const groups = computed(() => {
  const list = filteredProjects.value
  if (wb.filter.trim() || list.length <= 6) return [{ key: 'all', label: '', items: list }]
  const now = Date.now() / 1000
  const recent = []
  const older = []
  for (const p of list) {
    const t = Number(p.created_at) || 0
    // 没有 created_at 的老项目归"更早"：宁可沉底，也不要让它冒充"最近"
    if (t && now - t < GROUPS_RECENT_DAYS * 86400) recent.push(p)
    else older.push(p)
  }
  return [
    { key: 'recent', label: '最近', items: recent },
    { key: 'older', label: '更早', items: older }
  ].filter((g) => g.items.length)
})

function openProject(p) {
  emit('open', { pid: p.id, step: stepOf(p) })
}

async function go(path) {
  await router.push(path)
  // 移动抽屉：选完入口就收起（路由 watcher 也兜一层，这里让「点同一个地址」也能关）
  emit('close-mobile')
}

defineExpose({ openJobs: () => emit('open-jobs') })
</script>

<style scoped>
/* 折叠轨（样稿无此态，只写样稿里没有的东西）：宽度由 app.css 的 `.side.mini` 给 */
.side-toggle { margin-left:auto; width:26px; height:26px; flex:none; border-radius:var(--r-full);
  border:1px solid var(--line); background:var(--card); color:var(--ink-dim);
  display:flex; align-items:center; justify-content:center; }
.side-toggle:hover { border-color:var(--main); color:var(--pink-text); background:var(--main-soft); }
.side.mini .side-brand { flex-direction:column; gap:6px; padding:12px 0 6px; }
.side.mini .side-toggle { margin-left:0; }
/* 品牌按钮：原来是整块 div 可点，键盘 Tab 根本到不了。拆成「按钮 + 折叠开关」两个控件，
   `flex:1 1 auto` 让文字区吃掉余量、`min-width:0` 让长副标题能截断。 */
.brand-home { display:flex; align-items:center; gap:10px; min-width:0; flex:1 1 auto;
  padding:0; background:none; border:none; border-radius:var(--r-sm);
  text-align:left; color:inherit; }
.brand-home:hover .brand-name { color:var(--pink-text); }
.brand-copy { display:flex; flex-direction:column; min-width:0; }
.side.mini .brand-home { flex:none; }

/* 过滤框：图标在左、清空按钮在右，输入框两边留出内边距（否则文字压在图标上）。
   `.side-filter` 的外边距仍走 app.css 的基准值，这里只补内部排版。 */
.side-filter { display:flex; align-items:center; }
.side-filter > svg { position:absolute; left:9px; color:var(--ink-faint); pointer-events:none; }
.side-filter input { padding-left:27px; padding-right:26px; }
/* 键盘 Tab 到过滤框时必须有可见焦点（用户走查报「看不出焦点在哪」）。
   用 :focus-visible：鼠标点进来不画环，只有键盘操作才画 —— 与「Ctrl+K 快速跳转」同属键盘可达性。 */
.side-filter input:focus-visible { outline:2px solid var(--main); outline-offset:1px; border-radius:var(--r-sm); }
.filter-clear { position:absolute; right:3px; width:20px; height:20px; flex:none;
  border-radius:50%; display:flex; align-items:center; justify-content:center;
  color:var(--ink-faint); cursor:pointer; }
.filter-clear:hover { background:var(--bg-soft); color:var(--ink); }
/* 折叠轨里的过滤状态：不显示的话，被过滤掉的项目看起来就是"消失"了 */
.rail-filter { gap:3px; height:28px; font-size:10px; font-weight:600;
  background:var(--main-soft); color:var(--main-deep); }
.rail-filter:hover { background:var(--main-soft); color:var(--main-deep); }

/* 移动抽屉的关闭按钮：只在抽屉里出现（桌面 268px 侧栏有折叠开关，不需要它） */
.mobile-close { display:none; }

/* 分组小标题：比「项目（N）」更弱一档，只是分隔，不是标题 */
.side-subcap { display:flex; align-items:center; gap:6px; padding:10px 8px 3px;
  font-size:10px; letter-spacing:.12em; color:var(--ink-faint); }
.side-subcap-n { flex:none; padding:0 5px; border-radius:var(--r-full);
  background:var(--surface-2); font-size:9.5px; letter-spacing:0; }

.rail-list { flex:1; min-height:0; overflow-y:auto; overflow-x:hidden;
  display:flex; flex-direction:column; align-items:center; gap:6px; padding:0 0 8px; }
.rail-proj { position:relative; width:40px; height:40px; flex:none; border-radius:var(--r-full);
  background:var(--bg-soft); color:var(--ink-dim); font-size:13px; font-weight:700;
  display:flex; align-items:center; justify-content:center; transition:.15s; }
.rail-proj:hover { background:var(--main-soft); color:var(--pink-text); }
.rail-proj.on { background:linear-gradient(135deg,var(--action-bg),var(--action-bg-deep)); color:var(--on-action); box-shadow:var(--shadow-pink); }
.rail-hint { padding:4px 0; }
/* 空项目提示 = 去首页添加第一个项目的入口（不再是一句点不动的死文字） */
/* 空态 = 一条**按钮**（虚线框 + 加号，整条可点、hover 变主色） */
.empty-add { display:flex; align-items:center; justify-content:center; gap:6px;
  width:100%; box-sizing:border-box; margin:6px 0 2px; padding:12px 10px;
  border:1px dashed var(--line); border-radius:10px; background:var(--bg-soft);
  color:var(--pink-text); text-decoration:none; font-size:12px; font-weight:var(--fw-medium);
  transition:border-color var(--dur-fast) var(--ease), background-color var(--dur-fast) var(--ease),
    color var(--dur-fast) var(--ease); }
.empty-add:hover { border-color:var(--main); background:var(--main-soft); color:var(--main-deep); }
.empty-add:focus-visible { outline:2px solid var(--main-glow); outline-offset:2px; }

.side.mini .side-foot { align-items:center; padding:8px 0; }
/* 用户块只做展示（登出改成了右侧按钮 + 二次确认），光标别留成可点的手型 */
.user-chip { cursor:default; }
.side-logout { margin-left:auto; flex:none; padding:3px 8px; font-size:11.5px; }
/* 本地模式（免登录）：那块用户信息里**没有"某个人"也没有可退的会话** ——
   不画头像、不写用户名/角色，只留这句状态标（用户口径）。 */
.local-mark { flex:1; font-size:12px; font-weight:600; color:var(--green-deep); }
/* 折叠轨（64px）里的短标 */
.rail-user .local-mark { flex:none; font-size:10px; line-height:1.1; }
/* 折叠轨的用户块：头像 + 退出图标上下两行（原来只有头像）。
   选择器必须比 `.rail-item`（同为 scoped (0,2,0) 且写在后面）更特指，否则 height:34px 会赢 */
.side-foot .rail-user { flex-direction:column; height:auto; padding:4px 0; gap:2px; }
.rail-user .rail-out { color:var(--ink-faint); }
.rail-user:hover .rail-out { color:var(--red-deep); }
.rail-item { position:relative; width:40px; height:34px; flex:none; border-radius:var(--r-sm);
  display:flex; align-items:center; justify-content:center; color:var(--ink-dim); }
.rail-item:hover { background:var(--bg-soft); color:var(--ink); }
.rail-item.on { background:var(--main-soft); color:var(--pink-text); }
.rail-user .av { width:26px; height:26px; border-radius:50%; font-size:12px; color:var(--on-main);
  background:linear-gradient(135deg,var(--main),var(--tan));
  display:flex; align-items:center; justify-content:center; }
.rdot { position:absolute; top:2px; right:2px; width:8px; height:8px; border-radius:50%;
  background:var(--red); border:1.5px solid var(--card); }

/* ── 移动抽屉（≤760px）：抽屉本身就是展开态，折叠开关让位给「关闭」 ──
   抽屉的定位/宽度/位移在 app.css 的壳层规则里（`.side` 是全局壳类），这里只管内部控件。 */
@media (max-width: 760px) {
  .side-toggle { display:none; }
  .mobile-close { display:flex; margin-left:auto; width:30px; height:30px; flex:none;
    align-items:center; justify-content:center; border-radius:var(--r-full);
    border:1px solid var(--line); background:var(--card); color:var(--ink-dim); cursor:pointer; }
  .mobile-close:hover { border-color:var(--main); color:var(--pink-text); background:var(--main-soft); }
  /* 抽屉里项目名可以多显示一些（遮罩外没有内容要抢宽度） */
  .side-brand { padding-top:14px; }
}
</style>
