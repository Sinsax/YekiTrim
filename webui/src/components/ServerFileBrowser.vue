<template>
  <!-- `has-project` = 当前目录已有工程标志文件 → 目录区加绿框（与内联那套同一个信号） -->
  <div class="sfb" :class="{ 'has-project': hasProject }">
    <!-- 顶：面包屑（每一段可点回退）+ 快捷入口 -->
    <div class="sfb-top">
      <div class="sfb-crumbs">
        <template v-for="(c, i) in crumbs" :key="c.path">
          <Icon v-if="i" name="chevron-right" :size="12" class="sep" />
          <button class="crumb" :class="{ on: i === crumbs.length - 1 }" :title="c.path"
                  @click="go(c.path)">{{ c.label }}</button>
        </template>
      </div>
      <div v-if="roots.length && !hideRoots" class="sfb-roots">
        <span class="muted small flex-none">快捷入口</span>
        <button v-for="r in roots" :key="r.path" class="mini-btn" @click="go(r.path)">{{ r.label }}</button>
      </div>
    </div>

    <div v-if="error" class="error-banner">
      <Icon name="alert" :size="15" /> {{ error }}
      <button class="mini-btn flex-none" @click="go(current || startDirOrRoot())">重试</button>
    </div>

    <!-- 主体：左目录树 / 右文件列表 -->
    <div class="sfb-body">
      <!-- 左：目录树（懒加载 —— 展开某节点时才去列它的子目录） -->
      <div class="sfb-tree" role="tree">
        <div v-if="!flatTree.length && !treeLoading" class="muted small sfb-pad">（没有可展开的目录）</div>
        <div v-for="n in flatTree" :key="n.key" class="tnode" :class="{ on: n.path && n.path === current }"
             :style="{ paddingLeft: (6 + n.depth * 14) + 'px' }" @click="go(n)">
          <button class="tgl" :title="expanded.has(n.key) ? '收起' : '展开'" @click.stop="toggle(n)">
            <Icon :name="expanded.has(n.key) ? 'arrow-down' : 'arrow-right'" :size="11" />
          </button>
          <Icon name="folder" :size="13" class="tico" />
          <span class="tname" :title="n.path || n.name">{{ n.name }}</span>
          <span v-if="loadingKeys.has(n.key)" class="spinner"></span>
        </div>
      </div>

      <!-- 右：当前目录的内容（子目录可进，文件可选） -->
      <div class="sfb-files">
        <div class="sfb-path mono small" :title="current">{{ current || '—' }}</div>
        <div class="sfb-list" :class="{ busy: loading }">
          <div v-if="loading" class="muted small sfb-pad"><span class="spinner"></span> 读取中…</div>
          <template v-else>
            <div v-for="d in dirs" :key="'d-' + d" class="frow dir" @click="enter(d)">
              <Icon name="folder" :size="14" />
              <span class="mono fname">{{ d }}</span>
              <Icon name="chevron-right" :size="12" class="fgo" />
            </div>
            <div v-for="f in files" :key="'f-' + f"
                 class="frow" :class="{ on: selected === f, dim: !allowed(f), video: isVideo(f), project: isProjectFile(f) }"
                 @click="allowed(f) && emit('pick-file', f)">
              <Icon :name="iconFor(f)" :size="14" />
              <span class="mono fname">{{ f }}</span>
              <!-- 三种标签：工程配置（可选，点它 = 添加这个工程）/ 视频 / 不可选 -->
              <span v-if="isProjectFile(f)" class="tag-project">工程配置</span>
              <span v-else-if="!allowed(f)" class="muted small flex-none">不可选</span>
              <span v-else-if="isVideo(f)" class="muted small flex-none">视频</span>
            </div>
            <div v-if="!dirs.length && !files.length" class="muted small sfb-pad">空目录</div>
          </template>
        </div>
      </div>
    </div>
  </div>
</template>

<script>
/** 媒体扩展名白名单与 ServerDirPicker 同源（那个是这两张表的唯一真源，别各写一份）。 */
import { VIDEO_EXTS, IMAGE_EXTS } from './ServerDirPicker.vue'
export { VIDEO_EXTS, IMAGE_EXTS }
</script>

<script setup>
/**
 * 服务器文件浏览器（双栏：左目录树 / 右文件列表）。
 *
 * 与 `ServerDirPicker` 的分工 —— **两者不是彼此的复制品**：
 *   · `ServerDirPicker` = 就地小选择器（一个路径框 + 一层列表），适合嵌在页面里。
 *   · 本组件 = 大浏览器（目录树 + 面包屑 + 右侧文件），适合放进弹窗。
 * 底线相同：**只读服务器侧**（都走 `listTenantFiles` → `/api/fs/list`），
 * 这里**不出现 `<input type="file">`**。
 *
 * ⚠️ **路径一律由服务端拼**（与 ServerDirPicker 同一条铁律）：前端只把「父目录路径 + 子目录名」
 * 交给 `listTenantFiles(dir, enter)`，拿回服务端 canonicalize 过的 `dir`。自己写
 * `parent + '/' + name` 会把 `\?\F:` 拼成 `\?\F:/b` → Windows ERROR_INVALID_NAME 123。
 *
 * ⚠️ **树根不能用 `roots` 直接当**：服务端会把**当前目录**追加进 `roots`（本意是给下拉框显示
 * "当前位置"），而管理员能浏览任意绝对路径、配置根之外的地方（实测默认只有「我的托管目录」
 * 一个配置根，而用户要去的 `F:/media/...` 根本不在它下面）——只挂配置根的话，树里永远到不了
 * 用户真正在看的目录。所以树根 = **当前路径的祖先链顶端**（如 `F:`）+ 配置根。
 *
 * emits：`update:dir`（当前目录）/ `update:listing`（`{dir,files,dirs}`，供判有无可用视频）/ `pick-file`。
 */
import { computed, onMounted, ref, watch } from 'vue'
import Icon from './Icon.vue'
import { listTenantFiles } from '../stores/project'

const props = defineProps({
  /** 初始定位到的目录；空 = 服务端给的第一个快捷入口 */
  startDir: { type: String, default: '' },
  /** 当前已选文件名（高亮用） */
  selected: { type: String, default: '' },
  /** 只允许选这些扩展名（小写、不带点）；空 = 不限制 */
  exts: { type: Array, default: () => [] },
  /**
   * "工程标志文件名"（`yekitrim.toml`）。**只有新增/添加项目那条路传它**（默认空 = 不启用）。
   * 传了之后，当前目录存在该文件时给目录区加一层绿框 + 「已有工程」标记 —— 与内联那套同口径。
   * ⚠️ 别给它默认值：别的选择器（字体/弹幕/字幕）里这个文件名没有含义。
   */
  projectFile: { type: String, default: '' },
  /**
   * 隐藏组件**内部**那排快捷入口。
   * 调用方如果把 roots 提到外面做成「常用位置」条（首页「服务器文件」就是），
   * 这里再画一遍就是同一个东西出现两次 —— 传 true 关掉。
   * 默认 false：弹窗等其它用法仍然需要内置的快捷入口。
   */
  hideRoots: { type: Boolean, default: false }
})
const emit = defineEmits(['pick-file', 'update:dir', 'update:listing', 'update:roots'])

const current = ref('')
const crumbs = ref([])
/** **稳定的配置根**（只在探测时取一次）。位置由面包屑表达，别让根列表跟着当前位置变。 */
const roots = ref([])
const dirs = ref([])
const files = ref([])
const loading = ref(false)
const treeLoading = ref(false)
const error = ref('')

const treeRoots = ref([])
/**
 * 盘符/卷节点（服务端 `fs/list` 的 `volumes`）。
 * ⚠️ 没有它的话只能从"当前路径所在那个盘"往上展开（`parent` 到盘根就断了）—— **换不了盘**。
 * 这些是**展示候选**，不是访问限制（`/api/fs/list` 本来就允许 admin 浏览任意绝对路径）。
 */
const volumeNodes = ref([])
const childMap = ref({})
const expanded = ref(new Set())
const loadingKeys = ref(new Set())
const pathOf = ref({})

// 分隔符反斜杠用字符码取：在源码里写反斜杠字面量，很容易在生成文件那一层被吃掉一层，
// 只剩 / 当分隔符 —— 那样 Windows 路径永远切不开：节点名会显示成整条路径，树也展不下去。
const BS = String.fromCharCode(92)
const LAST = (p) => {
  let s = String(p)
  // 先去掉尾部单位分隔符：盘根 "F:\" 去掉后是 "F:"，否则"取最后一段"会得到空串，
  // 回退成整条路径 —— 节点名会变成一整条路径，链上按名字匹配子节点也就永远匹配不上。
  while (s.length > 1 && (s.endsWith("/") || s.endsWith(BS))) s = s.slice(0, -1)
  // POSIX 根 `/` 没有"最后一段"，但不能取成空串 —— 否则 Linux 上面包屑/树根是空白按钮。
  if (s === "/" || s === BS) return s
  const sep = Math.max(s.lastIndexOf("/"), s.lastIndexOf(BS))
  return sep >= 0 ? s.slice(sep + 1) : s
}
const extOf = (n) => String(n).split('.').pop().toLowerCase()
/** 是不是"工程标志文件"（没传 `projectFile` 时恒 false） */
const isProjectFile = (n) => !!props.projectFile && String(n).toLowerCase() === props.projectFile.toLowerCase()
/** 当前列出的目录里有没有工程标志文件 = 这是一个**已有工程**目录 */
const hasProject = computed(() => files.value.some(isProjectFile))
const isVideo = (n) => VIDEO_EXTS.includes(extOf(n))
const iconFor = (n) => (isVideo(n) ? 'film' : IMAGE_EXTS.includes(extOf(n)) ? 'image' : 'file')
const allowed = (n) => {
  const list = props.exts || []
  return list.length === 0 || list.includes(extOf(n))
}
const keyOf = (parent, name, path) => path || parent + '#' + name

const flatTree = computed(() => {
  const out = []
  const walk = (nodes, depth) => {
    for (const n of nodes) {
      out.push({ ...n, depth })
      if (expanded.value.has(n.key)) walk(childMap.value[n.key] || [], depth + 1)
    }
  }
  walk(treeRoots.value, 0)
  return out
})

async function read(target, enter = '') {
  loading.value = true
  error.value = ''
  try {
    return await listTenantFiles(target, enter)
  } catch (e) {
    error.value = '目录读取失败：' + e.message
    return null
  } finally {
    loading.value = false
  }
}
async function readQuiet(target) {
  try {
    return await listTenantFiles(target)
  } catch {
    return null
  }
}
/** 「父路径 + 名字」→ 服务端换回真实路径 */
async function resolvePath(parent, name) {
  const d = await read(parent, name)
  return d?.dir || ''
}

function allNodes() {
  const out = [...treeRoots.value]
  for (const arr of Object.values(childMap.value)) out.push(...arr)
  return out
}
function findNodeByPath(path) {
  if (!path) return null
  return allNodes().find((n) => (n.path || pathOf.value[n.key]) === path) || null
}
/** 节点拿到真实路径后写回（免得重复解析，也让高亮能匹配） */
function rememberPath(key, path) {
  pathOf.value = { ...pathOf.value, [key]: path }
  const next = {}
  for (const [k, arr] of Object.entries(childMap.value)) {
    next[k] = arr.map((c) => (c.key === key ? { ...c, path } : c))
  }
  childMap.value = next
}

/** 懒加载：展开某节点时才去列它的子目录 */
async function loadChildren(node) {
  if (childMap.value[node.key]) return
  loadingKeys.value = new Set(loadingKeys.value).add(node.key)
  try {
    let path = node.path || pathOf.value[node.key] || ''
    if (!path) {
      path = await resolvePath(node.parent, node.name)
      if (!path) return
      rememberPath(node.key, path)
    }
    const d = await read(path)
    if (!d) return
    childMap.value = {
      ...childMap.value,
      [node.key]: (d.dirs || []).map((nm) => ({ name: nm, parent: path, path: '', key: keyOf(path, nm, '') }))
    }
  } finally {
    const s = new Set(loadingKeys.value)
    s.delete(node.key)
    loadingKeys.value = s
  }
}

async function toggle(node) {
  const s = new Set(expanded.value)
  if (s.has(node.key)) s.delete(node.key)
  else s.add(node.key)
  expanded.value = s
  if (s.has(node.key)) await loadChildren(node)
}

/** 沿服务端给的 parent 往上回溯，得到祖先链（真实路径，前端不拼） */
async function buildCrumbs(dir) {
  const chain = [{ label: LAST(dir), path: dir }]
  let cur = dir
  for (let i = 0; i < 12; i++) {
    const d = await readQuiet(cur)
    const p = d?.parent || ''
    if (!p || p === cur) break
    chain.unshift({ label: LAST(p), path: p })
    cur = p
  }
  crumbs.value = chain
}

/**
 * 把祖先链铺进左树：链顶作为根，逐级确保节点存在 + 补真实路径 + 展开。
 * 这样"右栏进到哪，左树就展开到哪"，而不是永远只显示配置根。
 */
async function ensureChain(chain) {
  for (let i = 0; i < chain.length; i++) {
    const path = chain[i]
    if (i === 0) {
      if (!treeRoots.value.some((n) => n.path === path)) {
        treeRoots.value = [{ name: LAST(path), parent: '', path, key: path }, ...treeRoots.value]
      }
    }
    const node = findNodeByPath(path)
    if (!node) continue
    if (i + 1 < chain.length) {
      if (!expanded.value.has(node.key)) expanded.value = new Set(expanded.value).add(node.key)
      await loadChildren(node)
      const kid = (childMap.value[node.key] || []).find((c) => c.name === LAST(chain[i + 1]))
      if (kid) rememberPath(kid.key, chain[i + 1])
    }
  }
}

/** 切到某个目录：右列表 + 面包屑 + 左树跟着展开 + 通知父组件 */
async function go(nodeOrPath) {
  let target = ''
  if (typeof nodeOrPath === 'string') {
    target = nodeOrPath
  } else {
    target = nodeOrPath.path || pathOf.value[nodeOrPath.key] || ''
    if (!target) {
      target = await resolvePath(nodeOrPath.parent, nodeOrPath.name)
      if (!target) return
      rememberPath(nodeOrPath.key, target)
    }
  }
  const d = await read(target)
  if (!d) return
  current.value = d.dir || target
  dirs.value = d.dirs || []
  files.value = d.files || []
  emit('update:dir', current.value)
  // ⚠️ 文件清单必须发出去：父组件据它判"这个目录有没有可用视频"。
  //    不发的话"不点文件、让后端自动挑第一个视频"那条路会被误判成不可用。
  emit('update:listing', { dir: current.value, files: files.value, dirs: dirs.value })
  await buildCrumbs(current.value)
  await ensureChain(crumbs.value.map((c) => c.path))
}

/** 右侧列表里进子目录：走服务端 enter 契约 */
async function enter(name) {
  const d = await read(current.value, name)
  if (!d) return
  await go(d.dir || current.value)
}

/**
 * 外部改 `startDir`（常用位置 chip / 上层切换）要跟着跳。
 * ⚠️ `v !== current.value` 挡回环：自己浏览时 emit 的 `update:dir` 会让父组件把 startDir 设成同一个值。
 */
watch(() => props.startDir, (v) => {
  if (v && v !== current.value) go(v)
})

onMounted(async () => {
  treeLoading.value = true
  try {
    // ⚠️ 探测必须在**空目录**上做：`dir=''` 时服务端把 cur 解析成默认根（= roots[0] 本身），
    //    "当前目录不在根列表里就追加"那条分支不会触发 ⇒ 拿到的是**纯配置根**。
    const base = await listTenantFiles('')
    roots.value = base?.roots || []
    // 快捷入口（如「我的托管目录」）并进调用方的「常用位置」，作为**不可移除的预设点**
    emit('update:roots', roots.value)
    volumeNodes.value = (base?.volumes || []).map((v) => ({
      name: v.label,
      parent: '',
      path: v.path,
      key: v.path
    }))
    // 盘符先铺在树顶；祖先链顶端若正好是某个盘（`F:\`）会自动复用，不会出现两个 F:
    treeRoots.value = [...volumeNodes.value]
    await go(props.startDir || base?.dir || '')
    treeRoots.value = [
      ...treeRoots.value,
      ...roots.value
        .filter((r) => !treeRoots.value.some((n) => n.path === r.path))
        .map((r) => ({ name: r.label, parent: '', path: r.path, key: r.path }))
    ]
  } finally {
    treeLoading.value = false
  }
})
</script>

<style scoped>
/* 双栏文件浏览器 —— 样稿里没有这个东西，样式按纪律写在组件 scoped 里（不塞 app.css）。 */
.sfb { display:flex; flex-direction:column; gap:10px; }

.sfb-top { display:flex; flex-direction:column; gap:6px; }
.sfb-crumbs { display:flex; align-items:center; flex-wrap:wrap; gap:2px; min-height:22px; }
.sfb-crumbs .sep { color:var(--ink-faint); flex:none; }
.crumb { background:none; border:none; padding:2px 5px; border-radius:6px; cursor:pointer;
  color:var(--ink-dim); font-size:12px; max-width:230px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.crumb:hover { background:var(--bg-soft); color:var(--ink); }
.crumb.on { color:var(--main-deep); font-weight:600; }
.sfb-roots { display:flex; align-items:center; gap:6px; flex-wrap:wrap; }

/* 高度跟弹窗走：给足空间，列表自己滚 —— 别再钉死成一个矮条 */
.sfb-body { display:grid; grid-template-columns: minmax(190px, 26%) 1fr; gap:10px;
  height:min(54vh, 460px); min-height:230px; }
.sfb-tree { border:1px solid var(--line); border-radius:var(--r-sm); padding:6px 4px;
  overflow:auto; background:var(--bg); }
.sfb-files { display:flex; flex-direction:column; min-width:0;
  border:1px solid var(--line); border-radius:var(--r-sm); background:var(--card); }
.sfb-path { padding:7px 10px; border-bottom:1px solid var(--line-soft); color:var(--ink-dim);
  overflow:hidden; text-overflow:ellipsis; white-space:nowrap; flex:none; }
.sfb-list { flex:1; overflow:auto; padding:6px; }

.tnode { display:flex; align-items:center; gap:5px; padding:4px 6px; border-radius:7px;
  cursor:pointer; color:var(--ink-dim); font-size:12.5px; }
.tnode:hover { background:var(--bg-soft); }
.tnode.on { background:var(--main-soft); color:var(--main-deep); font-weight:600; }
.tnode .tname { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.tnode .tico { flex:none; color:var(--tan-deep); }
.tgl { flex:none; width:16px; height:16px; display:flex; align-items:center; justify-content:center;
  background:none; border:none; cursor:pointer; color:var(--ink-faint); padding:0; border-radius:4px; }
.tgl:hover { background:var(--line-soft); color:var(--ink); }

.frow { display:flex; align-items:center; gap:8px; padding:6px 9px; border-radius:8px;
  cursor:pointer; color:var(--ink-dim); }
.frow:hover { background:var(--bg-soft); }
.frow .fname { flex:1; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.frow.video, .frow.dir { color:var(--ink); }
/* `yekitrim.toml` 那行：可选，点它 = "添加这个已有工程"（不是当输入视频） */
.frow.project { color:var(--green-deep); }
.tag-project { flex:none; font-size:11px; padding:1px 6px; border-radius:999px;
  color:var(--green-deep); background:var(--green-soft); border:1px solid var(--green); }
/* ⚠️ 已有工程目录的绿框（与内联那套同一个信号，别挪到别的元素上） */
.sfb.has-project { border:1px solid var(--green); border-radius:var(--r-sm);
  padding:10px 12px; background:var(--green-soft); }
.sfb.has-project::before { content:"已有工程 · 将直接添加（不覆盖现有配置）";
  display:block; font-size:11.5px; color:var(--green-deep); margin-bottom:2px; }
.frow.dir .fgo { color:var(--ink-faint); flex:none; }
.frow.on { background:var(--main-soft); color:var(--main-deep); font-weight:600; }
.frow.dim { opacity:.5; cursor:not-allowed; }
.frow.dim:hover { background:none; }
.sfb-pad { padding:8px 6px; }
.sfb-list.busy { opacity:.6; }
</style>
