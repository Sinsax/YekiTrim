<template>
  <div class="card card-blk mb-12">
    <div class="row spread mb-8">
      <b class="t" style="margin:0"><Icon name="image" :size="15" /> 表情包</b>
      <span class="muted small">
        {{ packs.length }} 个包 · 已构建 {{ builtCount }} 个
        <template v-if="!packs.length">· 上传一个压缩包即可用</template>
      </span>
      <button class="mini-btn primary" :disabled="busy" @click="openAdd">
        <Icon name="plus" :size="13" /> 添加表情包
      </button>
    </div>

    <div v-if="root" class="hint">
      贴纸库：<span class="mono">{{ root }}</span> —— 一个子文件夹 = 一个包；
      「LLM 压缩构建」读文件名产出 10~20 个语义 tag（<span class="mono">&lt;包名&gt;.toml</span>），
      精修时 LLM 就按这些 tag 选贴纸。<span v-if="!packs.length">直接把文件夹丢进这个目录也可以，刷新就能看到。</span>
    </div>

    <div v-if="error" class="error-banner mt-8"><Icon name="alert" :size="15" /> {{ error }}</div>

    <div v-if="!packs.length && !loading" class="empty small mt-8">
      还没有表情包。点右上角「添加表情包」，上传一个 zip 压缩包（里面的 gif / png / jpg / jpeg / webp 会解压成一个包）。
    </div>

    <!-- 一个包 = 一条可折叠行；展开才拉包内文件（167 张也不拖累首屏），
         包内以卡片墙展示（大图 + 文件名 + 该文件命中的 tag），点卡片看大图 -->
    <details
      v-for="p in packs"
      :key="p.name"
      class="details-card mt-8"
      @toggle="onToggle(p, $event)"
    >
      <summary>
        <span class="mono pack-name">{{ p.name }}</span>
        <span class="chip n">{{ p.count }} 张</span>
        <span class="muted small">{{ fmtBytes(p.bytes) }}</span>
        <span v-if="p.built" class="chip g">已构建 {{ p.tags.length }} tags</span>
        <span v-else class="chip a">未构建</span>
        <span class="row pack-acts" @click.stop>
          <button class="mini-btn" :disabled="busy || jobBusy" @click="build(p)">
            <span v-if="building === p.name" class="spinner"></span>
            {{ p.built ? '重新构建' : 'LLM 压缩构建' }}
          </button>
          <button class="mini-btn danger" :disabled="busy || jobBusy" @click="remove(p)">删除</button>
        </span>
      </summary>

      <div class="mt-8">
        <div v-if="p.built && p.tags.length" class="chips mb-8">
          <span v-for="t in p.tags" :key="t.tag" class="chip">{{ t.tag }}({{ t.count }})</span>
        </div>
        <div v-else class="hint mb-8">
          未构建：图片能用，但每个文件还没有语义 tag（LLM 生成事件时读不到这个包）。
        </div>

        <div class="pack-files">
          <div v-if="(detail[p.name] || []).length" class="stk-grid">
            <div
              v-for="f in detail[p.name]"
              :key="f.name"
              class="stk-card"
              :title="`${f.name} · ${fmtBytes(f.size)}${f.tags.length ? ' · ' + f.tags.join(' ') : ''}（点开看大图）`"
              @click="openPreview(p, f)"
            >
              <div class="stk-box">
                <Icon class="stk-ph" name="image" :size="22" />
                <img
                  :src="stickersApi.fileUrl(p.name, f.name)"
                  loading="lazy"
                  alt=""
                  @error="$event.target.style.display = 'none'"
                >
              </div>
              <div class="stk-meta">
                <span class="mono stk-name">{{ f.name }}</span>
                <span class="stk-size">{{ fmtBytes(f.size) }}</span>
              </div>
              <div class="chips stk-tags">
                <span v-if="!f.tags.length" class="muted stk-none">未打 tag</span>
                <span v-for="t in f.tags" :key="t" class="chip n">{{ t }}</span>
              </div>
            </div>
          </div>
          <div v-else-if="detailLoading === p.name" class="row muted small stk-loading">
            <span class="spinner"></span> 读取中…
          </div>
          <div v-else class="row muted small stk-loading">
            这个包里没有图片（gif / png / jpg / jpeg / webp）。
          </div>
        </div>
      </div>
    </details>

    <AppModal v-if="showAdd" :show="true" title="添加表情包" @close="showAdd = false">
      <div class="row">
        <div class="flex-1 minw-240">
          <label>包名（精修里选贴纸时显示的名字）</label>
          <input
            v-model.trim="addName"
            placeholder="默认取压缩包文件名，例如 ysyk"
            autocomplete="off"
            @input="nameTouched = true"
          >
        </div>
        <label class="row mt-20" style="gap:6px;cursor:pointer">
          <input v-model="addBuild" type="checkbox" style="width:auto"> 上传后立刻用 LLM 构建 tags
        </label>
      </div>
      <div v-if="addName && !nameOk" class="hint mt-4">
        包名只能用字母 / 数字 / 下划线 / 短横线 / 汉字，1~32 个字符。
      </div>

      <!-- 上传区：整块可点（走隐藏的 file input），也接拖拽。没有进度条（fetch 拿不到上传进度） -->
      <input ref="zipInput" type="file" accept=".zip,application/zip" class="hidden-file" @change="onZipPick">
      <div
        class="up-zone mt-12"
        :class="{ on: dragOver, bad: !!zipError }"
        @click="pickZip"
        @dragover.prevent="dragOver = true"
        @dragleave.prevent="dragOver = false"
        @drop.prevent="onDrop"
      >
        <div v-if="zipFile" class="row" style="gap:8px;justify-content:center">
          <Icon name="file" :size="15" />
          <span class="mono">{{ zipFile.name }}</span>
          <span class="muted small">{{ fmtBytes(zipFile.size) }}</span>
          <button class="mini-btn" @click.stop="clearZip">换一个</button>
        </div>
        <div v-else class="up-tip">
          <Icon name="arrow-up" :size="18" />
          <div>把 <b>.zip</b> 拖到这里，或点此选择文件</div>
          <div class="muted small">上限 {{ fmtBytes(UPLOAD_MAX_BYTES) }}</div>
        </div>
      </div>
      <div v-if="zipError" class="error-banner mt-8"><Icon name="alert" :size="15" /> {{ zipError }}</div>
      <div v-if="importing" class="row muted small mt-6"><span class="spinner"></span> 上传中…（大包要等一会儿，没有进度条）</div>

      <div class="hint mt-6">
        压缩包里的 gif / png / jpg / jpeg / webp 会解压进
        <span class="mono">stickers/&lt;包名&gt;/</span>；子文件夹会摊平、重名只留第一个，
        其它条目跳过并计数。已存在的包不会被合并（换个包名或先删掉那一个）。
      </div>

      <template #footer>
        <button class="btn" @click="showAdd = false">取消</button>
        <button
          class="btn btn-primary"
          :disabled="importing || !zipFile || !addName || !nameOk || !!zipError"
          @click="doUpload"
        >
          <span v-if="importing" class="spinner"></span> 上传
        </button>
      </template>
    </AppModal>

    <!-- 点卡片看大图：卡片里的图只有一百来像素，动图要放大才看得出效果 -->
    <AppModal
      v-if="previewFile"
      :show="true"
      :title="`${preview.pack} / ${previewFile.name}`"
      wide
      @close="preview = null"
    >
      <div class="stk-preview">
        <img :src="stickersApi.fileUrl(preview.pack, previewFile.name)" alt="">
      </div>
      <div class="row mt-8">
        <span class="muted small">{{ fmtBytes(previewFile.size) }}</span>
        <span v-if="previewFile.tags.length" class="chips">
          <span v-for="t in previewFile.tags" :key="t" class="chip n">{{ t }}</span>
        </span>
        <span v-else class="muted small">这个文件还没有 tag（未构建，或构建时没给它打上）。</span>
      </div>
      <template #footer>
        <button class="btn" @click="preview = null">关闭</button>
      </template>
    </AppModal>
  </div>
</template>

<script>
// `onJobDone` 没有反注册入口（回调数组只增不减），所以**全局只注册一次**，
// 处理器指向"当前挂载的那个实例"。写在普通 <script> 里 = 模块级，
// 不会每次进首页都往 doneWatchers 里再塞一个闭包。
import { onJobDone, useJobs } from '../composables/useJobs'
import { busyOn, STICKERS_KEY, QUEUED_HINT } from '../api/jobs'

let jobHandler = null
let jobWatcherReady = false

function ensureJobWatcher() {
  if (jobWatcherReady) return
  jobWatcherReady = true
  onJobDone((j) => {
    if (jobHandler) jobHandler(j)
  })
}
</script>

<script setup>
/**
 * 首页「表情包」区块（仅管理员渲染，见 WorkbenchView）。
 *
 * 库 = `<程序目录>/stickers/`：子文件夹是包，`<包名>.toml` 是 LLM 构建出的 tag 索引，
 * `sticker.toml` 是已构建包的注册表（只有注册过的包才会被自动加进项目的贴纸来源）。
 */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import AppModal from './AppModal.vue'
import Icon from './Icon.vue'
import { stickersApi, UPLOAD_MAX_BYTES } from '../api/stickers'
import { fmtBytes } from '../stores/ingest'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'

const packs = ref([])
const root = ref('')
const loading = ref(false)
const error = ref('')
const busy = ref(false)
/** 正在构建的包名（本地高亮）+ 贴纸库是否被构建任务占用 */
const building = ref('')
// 贴纸库是**全局资源**（写 `<包名>.toml` 与 `sticker.toml` 索引）：两个构建互斥（排队）。
// 状态从全局任务列表算，而不是"我点过构建"——别的标签页/别的管理页起的构建同样算数。
const { jobs: allJobs } = useJobs()
const jobBusy = computed(() => !!busyOn(allJobs.value, [STICKERS_KEY]))
/** 展开过的包的文件清单：`包名 → [{name,size,tags}]` */
const detail = ref({})
const detailLoading = ref('')

const showAdd = ref(false)
/** 点开的卡片：`{pack, name}`，null = 关着。只存名字，文件对象从 detail 里现取
    （构建完 / 重载后 tag 变了，预览要跟着新数据走，别抓一份快照） */
const preview = ref(null)
const previewFile = computed(() => {
  const pv = preview.value
  if (!pv) return null
  return (detail.value[pv.pack] || []).find((f) => f.name === pv.name) || null
})
const addName = ref('')
const nameTouched = ref(false)
const addBuild = ref(true)
const importing = ref(false)
/** 隐藏的 file input（点上传区就是在点它） */
const zipInput = ref(null)
/** 待上传的压缩包（File 对象）；null = 还没选 */
const zipFile = ref(null)
/** 选中文件时的就地校验提示（类型/大小/空文件）——服务端的 413 不是 JSON，前端必须先挡 */
const zipError = ref('')
const dragOver = ref(false)

const builtCount = computed(() => packs.value.filter((p) => p.built).length)
/** 与服务端 `sanitize_pack_name` 同规则：字母/数字/下划线/短横线/汉字，1~32 */
const nameOk = computed(() => /^[0-9A-Za-z_\-\p{L}]{1,32}$/u.test(addName.value) && !addName.value.startsWith('.'))

async function load() {
  loading.value = true
  try {
    const d = await stickersApi.list()
    packs.value = d.packs || []
    root.value = d.root || ''
    error.value = ''
    // 已展开的包重新拉一次（构建完 tag 就变了）
    for (const name of Object.keys(detail.value)) await loadDetail(name)
  } catch (e) {
    error.value = '读取表情包失败：' + e.message
  } finally {
    loading.value = false
  }
}

async function loadDetail(name) {
  detailLoading.value = name
  try {
    const d = await stickersApi.detail(name)
    detail.value = { ...detail.value, [name]: d.files || [] }
  } catch {
    // 失败已由 api() 记日志；这里只是没有预览
    detail.value = { ...detail.value, [name]: [] }
  } finally {
    if (detailLoading.value === name) detailLoading.value = ''
  }
}

function onToggle(p, e) {
  if (e.target.open && !detail.value[p.name]) loadDetail(p.name)
}

function openPreview(p, f) {
  preview.value = { pack: p.name, name: f.name }
}

function openAdd() {
  showAdd.value = true
  addName.value = ''
  nameTouched.value = false
  addBuild.value = true
  clearZip()
}

function pickZip() {
  if (zipInput.value) zipInput.value.click()
}

function clearZip() {
  zipFile.value = null
  zipError.value = ''
  dragOver.value = false
}

/** zip 文件名 → 候选包名：去掉 `.zip`，其余非法字符换成 `_`（与服务端 sanitize_pack_name 同规则）。
    自动换掉非法字符只是省一次手改，用户仍可自己编辑。 */
function tidyName(fileName) {
  return String(fileName || '')
    .replace(/\.zip$/i, '')
    .replace(/[^0-9A-Za-z_\-\p{L}]/gu, '_')
    .replace(/^_+|_+$/g, '')
    .slice(0, 32)
}

/** 收下一个文件：类型/大小/空文件就地校验，过了才允许提交。 */
function acceptZip(file) {
  zipFile.value = null
  zipError.value = ''
  if (!file) return
  if (!/\.zip$/i.test(file.name || '')) {
    zipError.value = `只收 .zip 压缩包（选的是「${file.name || '未知文件'}」）`
    return
  }
  if (!file.size) {
    zipError.value = '这个文件是空的'
    return
  }
  if (file.size > UPLOAD_MAX_BYTES) {
    zipError.value = `压缩包太大（${fmtBytes(file.size)}，上限 ${fmtBytes(UPLOAD_MAX_BYTES)}）`
    return
  }
  zipFile.value = file
  if (!nameTouched.value) addName.value = tidyName(file.name)
}

function onZipPick(e) {
  acceptZip(e.target.files && e.target.files[0])
  // 清空 value：同一个文件改完再选一次也要能触发 change
  e.target.value = ''
}

function onDrop(e) {
  dragOver.value = false
  const dt = e.dataTransfer
  acceptZip(dt && dt.files && dt.files[0])
}

async function doUpload() {
  if (!zipFile.value) return
  importing.value = true
  try {
    const r = await stickersApi.uploadPack(zipFile.value, addName.value, addBuild.value)
    const skipped = r.skipped ? `，跳过 ${r.skipped} 个` : ''
    toast(`已上传「${r.name}」${r.copied} 张${skipped}`)
    if (r.job_error) toast('标签构建没起来：' + r.job_error, 'err')
    showAdd.value = false
    await load()
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    importing.value = false
  }
}

async function build(p) {
  busy.value = true
  try {
    const r = await stickersApi.build(p.name)
    if (r?.job_error) { toast(r.job_error, 'err'); return }
    if (!r?.id) { toast('构建任务未启动', 'err'); return }
    building.value = p.name
    toast(r.queued
      ? QUEUED_HINT
      : `已开始构建「${p.name}」的标签（任务 ${r.id}）`)
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    busy.value = false
  }
}

async function remove(p) {
  const ok = await confirmDialog(
    `删除表情包「${p.name}」？会删掉 ${p.count} 张图片与它的 tag 清单，删了就找不回来。\n` +
      '已经复制进各项目的贴纸不受影响；如果某个项目的贴纸来源里还选着这个包，去精修页把它去掉。',
    { title: '删除表情包？', danger: true }
  )
  if (!ok) return
  busy.value = true
  try {
    await stickersApi.remove(p.name)
    const d = { ...detail.value }
    delete d[p.name]
    detail.value = d
    if (preview.value && preview.value.pack === p.name) preview.value = null
    toast('已删除')
    await load()
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    busy.value = false
  }
}

onMounted(async () => {
  ensureJobWatcher()
  jobHandler = (j) => {
    if (j.kind !== 'sticker_build') return
    building.value = ''
    if (j.status === 'succeeded') toast('标签已构建完成')
    else if (j.status === 'cancelled') toast('构建已取消')
    else toast('构建失败：' + (j.error || '未知错误'), 'err')
    load()
  }
  await load()
})

onUnmounted(() => {
  jobHandler = null
})
</script>

<style scoped>
/* 折叠行：数值全部来自 app.css 的 .details-card / .chip / .hint，这里只补
   「包名占位」「右侧动作组」「包内卡片墙」「大图预览」「上传区」五件样稿没有的事。 */
.pack-name { flex:none; font-size:13px; }
.pack-acts { margin-left:auto; gap:6px; }
/* 167 张也不能把首页撑长：固定高度内滚动（卡片比表格行高，所以比表格版高一点）。
   底衬用 bg-soft、卡片用 card，一张张卡才立得起来（同色会糊成一片） */
.pack-files { max-height:420px; overflow:auto; border:1px solid var(--line-soft); border-radius:var(--r-sm); background:var(--bg-soft); }
.stk-loading { padding:10px; }

/* 一个文件 = 一张卡：图占满卡宽（方形底衬），下面文件名 + 体积 + 它命中的 tag。
   卡片比表格行装得下更多东西（大图 + 换行的 tag），一屏也还是能扫到十几张。 */
.stk-grid { display:grid; grid-template-columns:repeat(auto-fill,minmax(112px,1fr)); gap:8px; padding:8px; }
.stk-card {
  display:flex; flex-direction:column; gap:4px; padding:6px;
  background:var(--card); border:1px solid var(--line-soft); border-radius:10px;
  cursor:zoom-in; transition:all .15s;
}
.stk-card:hover { border-color:var(--main); box-shadow:var(--shadow-pink); transform:translateY(-2px); }
.stk-box {
  position:relative; aspect-ratio:1/1; display:flex; align-items:center; justify-content:center;
  background:var(--bg-soft); border-radius:8px; overflow:hidden;
}
/* 图挂了/还在加载时露出来的占位图标（放图下面，图盖在上面） */
.stk-ph { position:absolute; color:var(--ink-faint); opacity:.5; }
.stk-box img { position:relative; max-width:100%; max-height:100%; object-fit:contain; display:block; }
.stk-meta { display:flex; align-items:baseline; gap:6px; min-width:0; }
.stk-name { flex:1 1 auto; min-width:0; font-size:10.5px; color:var(--ink-dim); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.stk-size { flex:none; font-size:9.5px; color:var(--ink-faint); }
/* tag 全部显示（不裁切、不滚动）：卡片是竖排的，一行的卡片按最高的那张对齐 */
.stk-tags { gap:3px; }
.stk-tags .chip { font-size:9.5px; padding:1px 6px; }
.stk-none { font-size:9.5px; }

.stk-preview { display:flex; align-items:center; justify-content:center; min-height:220px; padding:10px; background:var(--bg-soft); border-radius:var(--r-sm); }
.stk-preview img { max-width:100%; max-height:60vh; object-fit:contain; }

/* 上传区（样稿里没有这个态）：虚线框 + 奶白底，拖进来/悬停时亮成主色。
   数值全部取自 tokens（颜色只认 --line / --bg-soft / --main / --main-soft / --r-sm）。 */
.hidden-file { display:none; }
.up-zone {
  display:flex; align-items:center; justify-content:center; min-height:92px;
  padding:14px 16px; cursor:pointer; text-align:center;
  border:1.5px dashed var(--line); border-radius:var(--r-sm); background:var(--bg-soft);
  transition:border-color .15s, background .15s;
}
.up-zone:hover, .up-zone.on { border-color:var(--main); background:var(--main-soft); }
.up-zone.bad { border-color:var(--red); }
.up-tip { display:flex; flex-direction:column; align-items:center; gap:4px; color:var(--ink-dim); font-size:12.5px; }
.up-zone .mono { font-size:11.5px; word-break:break-all; }
</style>
