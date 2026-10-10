<template>
  <div class="card card-blk mb-12">
    <div class="row spread mb-8">
      <b class="t m-0"><Icon name="image" :size="15" /> 素材库</b>
      <div class="row" style="flex-wrap:nowrap">
        <span class="muted small">{{ countText }}</span>
        <button class="mini-btn" :disabled="loading" @click="load(true)">
          <span v-if="loading" class="spinner"></span> 刷新
        </button>
      </div>
    </div>

    <!-- **文件夹管理器布局**：左=类型导航（象形 + 名称 + 计数），右=该类型的素材网格。
         用户口径：「素材库ui改成文件夹管理器类似的感觉，左侧选择类型」「统一一下导入方式，
         都需要预览效果」。三块内容因此共用同一套"网格 + 点击放大 + 拖拽导入"交互 ——
         以前它们各写一套（视频只有一行文字、字体/贴纸各自成卡），同一个动作三种手感。 -->
    <div class="ml-body">
      <nav class="ml-nav">
        <button
          v-for="t in TABS"
          :key="t.key"
          class="ml-tab"
          :class="{ on: kind === t.key }"
          :title="t.hint"
          @click="switchKind(t.key)"
        >
          <Icon :name="t.icon" :size="15" class="ml-ico" />
          <span class="ml-lab">{{ t.label }}</span>
          <span class="ml-cnt">{{ counts[t.key] }}</span>
        </button>

        <!-- 字体页的「样例文字」放**左侧类型栏**（用户口径：字体不需要右侧栏，
             样例文字输入框放左侧即可）—— 它是"这一类素材的显示参数"，跟着导航走比
             挤在右上角更顺：选了字体、视线不用跨过整个网格去改样例。 -->
        <label v-if="kind === 'font'" class="ml-sample-wrap">
          <span class="muted small">样例文字</span>
          <input
            v-model="testText"
            class="ml-sample"
            placeholder="留空 = 永 Aa 123"
          >
        </label>
      </nav>

      <section class="ml-main">
        <div class="row spread mb-8">
          <span class="muted small">{{ hint }}</span>
          <div class="row" style="flex-wrap:nowrap">
            <button class="mini-btn primary" :disabled="busy" @click="fileInput?.click()">
              <Icon name="plus" :size="13" /> {{ importLabel }}
            </button>
          </div>
        </div>

        <!-- **统一导入区**：三种素材都是"拖进来 / 点选文件"。
             ⚠️ 视频**没有上传**（它可能是几个 GB）—— 拖拽只用来提示放哪个目录，
             真正的入口是"放进目录 + 刷新"，所以这里对视频显示目录而不是打开文件框。
             这个差别必须显式写出来，否则用户拖一个 2GB 视频进来会以为界面坏了。 -->
        <div
          class="ml-drop"
          :class="{ over: dragOver, off: kind === 'video' }"
          @dragover.prevent="onDragOver"
          @dragleave.prevent="onDragLeave"
          @drop.prevent="onDrop"
          @click="kind === 'video' ? null : fileInput?.click()"
        >
          <input ref="fileInput" type="file" :accept="accept" multiple hidden @change="onFilePick">
          <Icon :name="kind === 'video' ? 'folder' : 'plus'" :size="16" />
          <div class="ml-drop-t">
            <b v-if="kind === 'video'">视频直接放进这个目录，再点「刷新」</b>
            <b v-else-if="busy">{{ importLabel }}中…（{{ doneCount }}/{{ totalCount }}）</b>
            <b v-else>{{ dropText }}</b>
            <span class="hint ml-dir mono">{{ root || '读取中…' }}</span>
          </div>
        </div>

        <div v-if="error" class="error-banner mt-8"><Icon name="alert" :size="15" /> {{ error }}</div>

        <p v-if="!items.length && !loading && !error" class="empty small mt-8">
          {{ emptyText }}
        </p>

        <!-- **统一预览网格**：缩略图 + 点击放大。视频的缩略图是一张真帧（服务端截），
             所以三种素材在这里长得一样，用户不用学三种看法。
             网格上方那排小按钮 = 该类型的**管理动作**（默认 / 构建 / 删除），
             悬停或聚焦该格才出现 —— 常显会把 108px 的格子塞满，而它们是低频操作。 -->
        <div v-else class="ml-grid mt-8">
          <div
            v-for="it in items"
            :key="it.key"
            class="ml-cell-wrap"
            :class="{ sel: preview && preview.name === it.name }"
          >
            <button class="ml-cell" :title="it.title" @click="open(it)">
              <span class="ml-thumb">
                <!-- 字体：没有缩略图，直接用该字体现场渲染样例字（就是"预览效果"本身） -->
                <span
                  v-if="kind === 'font'"
                  class="ml-font"
                  :style="{ fontFamily: it.stack }"
                >{{ it.sample }}</span>
                <!-- 视频：服务端截的帧。截不出来（容器怪异/没装 ffmpeg）时退成图标，别留白块。
                     `@error` 把 thumb 清空 ⇒ 自动落到下面那个 Icon 分支，不在界面上留一个裂图。 -->
                <img v-else-if="it.thumb" :src="it.thumb" alt="" loading="lazy" @error="it.thumb = ''">
                <Icon v-else :name="kind === 'video' ? 'video' : 'image'" :size="22" />
                <span v-if="it.badge" class="ml-badge">{{ it.badge }}</span>
              </span>
              <span class="ml-name">{{ it.name }}</span>
              <span class="ml-sub muted">{{ it.sub }}</span>
            </button>
            <div v-if="it.actions && it.actions.length" class="ml-acts">
              <button
                v-for="a in it.actions"
                :key="a.label"
                class="ml-act"
                :title="a.hint"
                :disabled="busy"
                @click.stop="a.run()"
              >{{ a.label }}</button>
            </div>
          </div>
        </div>
      </section>

      <!-- **右侧预览栏** —— 只服务贴纸（用户口径 2026-10-10 修订）：
           · 贴纸 —— 在这里**列表显示**整包；
           · **视频** —— 改成**点击弹窗**播放（用户口径：「视频还是改成点击弹出吧，
             视频的右侧栏删除」）。视频要的是"大一点看清楚"，弹窗比定宽栏合适，
             而且弹窗本来就有（`zoom`），不用为视频单留一条渲染分支；
           · **字体** —— 没有右栏，样例文字输入框在左侧。
           ⚠️ 所以右栏的显示条件从 `kind !== 'font'` 收紧成 **`kind === 'sticker'`**：
           视频页整条右栏不渲染（不是渲染出来再藏起来 —— 那还会占掉 600px 的宽度）。 -->
      <aside v-if="kind === 'sticker'" class="ml-side">
        <div v-if="!preview" class="ml-side-empty muted small">
          点左边的贴纸包，在这里列出包内所有图
        </div>
        <template v-else>
          <div class="row spread mb-8" style="flex-wrap:nowrap">
            <b class="m-0">{{ preview.name }}</b>
            <span class="muted small">{{ preview.files.length }} 张</span>
          </div>
          <p v-if="!preview.files.length" class="muted small m-0">这个包里没有图片。</p>
          <!-- ⚠️ `loading="lazy"`：一个包可能有 200+ 张（ysyk 就是 210），
               不懒加载会一次性全解码，整页卡住。 -->
          <div v-else class="ml-side-grid">
            <img
              v-for="f in preview.files"
              :key="f.name"
              :src="f.url"
              :alt="f.name"
              :title="f.name"
              loading="lazy"
              @click="openStickerImage(f)"
            >
          </div>
        </template>
      </aside>
    </div>

    <!-- 点击放大：视频真播（用 `<video>` + Range，服务端那条取文件路已支持）、
         字体放大看字形、贴纸看大图。三者在同一个弹窗里，位置/关闭方式一致。 -->
    <AppModal v-if="zoom" :title="zoom.title" @close="zoom = null">
      <div class="ml-zoom">
        <video
          v-if="zoom.type === 'video'"
          :src="zoom.url"
          controls
          autoplay
          class="ml-zoom-video"
        ></video>
        <img v-else-if="zoom.type === 'sticker'" :src="zoom.url" alt="" class="ml-zoom-img">
        <div v-else class="ml-zoom-font">
          <p class="ml-zoom-line" :style="{ fontFamily: zoom.stack }">{{ zoom.sample }}</p>
          <p class="ml-zoom-meta muted small">{{ zoom.meta }}</p>
        </div>
        <p v-if="zoom.detail" class="hint m-0 mt-8">{{ zoom.detail }}</p>
      </div>
    </AppModal>
  </div>
</template>

<script setup>
import { computed, onMounted, reactive, ref } from 'vue'
import Icon from './Icon.vue'
import AppModal from './AppModal.vue'
import { api } from '../api/client'
import {
  assetUrl,
  deleteFont,
  FONT_UPLOAD_MAX_BYTES,
  setDefaultFont,
  uploadFont,
  videoThumbUrl,
} from '../api/polish'
import { stickersApi, UPLOAD_MAX_BYTES } from '../api/stickers'
import { confirmDialog } from '../stores/dialog'
import { toast } from '../stores/toast'

defineProps({ isAdmin: { type: Boolean, default: false } })

const TABS = [
  { key: 'video', label: '视频', icon: 'video', hint: '片头 / 片尾的素材来源' },
  { key: 'sticker', label: '贴纸', icon: 'image', hint: '精修时 LLM 按 tag 选的贴纸' },
  { key: 'font', label: '字体', icon: 'type', hint: '字幕与封面可选的字体' },
]
const kind = ref('video')
const tab = computed(() => TABS.find((t) => t.key === kind.value) || TABS[0])
const hint = computed(() => tab.value.hint)
const importLabel = computed(
  () => ({ video: '导入视频', sticker: '导入贴纸包', font: '导入字体' })[kind.value],
)
const dropText = computed(
  () => ({ sticker: '把 zip 压缩包拖到这里（或点选文件）', font: '把字体文件拖到这里（或点选文件）' })[kind.value] || '',
)
const emptyText = computed(
  () => ({
    video: '还没有视频素材 —— 把 mp4 / webm / mkv 放进上面那个目录，再点「刷新」',
    sticker: '还没有贴纸包 —— 点「导入贴纸包」上传一个 zip（里面是图片）',
    font: '还没有字体 —— 点「导入字体」上传 .ttf / .otf',
  })[kind.value],
)
const accept = computed(() => (kind.value === 'font' ? '.ttf,.otf' : kind.value === 'sticker' ? '.zip' : '*'))

/** 样例文字：空 = 各卡片用默认样张。中英数字混排才看得出字形/字重差异。 */
const testText = ref('')
const SAMPLE = '永 Aa 123'
const sampleOf = () => testText.value.trim() || SAMPLE

/** 三种素材各自的列表状态（切类型不互清 —— 切回来是即时的）。 */
const videos = ref([])
const packs = ref([])
/** 包封面：`{包名: 图片URL}`。**列表接口不带文件名**（只回 `count`），所以封面要现拉 detail。
 *
 * ⚠️ 缓存在这里、**每个包只拉一次**：`items` 是 computed，每次重算都会调 `coverOf`，
 * 不缓存就是"每渲染一次发一轮请求"（素材库一多就是几十连发）。
 * ⚠️ 拉失败**留空**（渲染成占位），不要拿别人的图顶上 —— 张冠李戴比没有图更难发现。 */
const covers = reactive({})
function coverOf(pack) {
  if (covers[pack] !== undefined) return covers[pack]
  covers[pack] = '' // 占位，同时充当"已在拉"的标记
  stickersApi
    .detail(pack)
    .then((d) => {
      const first = (d?.files || [])[0]?.name
      covers[pack] = first ? stickersApi.fileUrl(pack, first) : ''
    })
    .catch(() => {
      covers[pack] = ''
    })
  return ''
}
/* ---------------- 右侧预览栏 ---------------- */

/** 当前选中的素材（视频对象 / 贴纸包对象）。`null` = 还没选。 */
const preview = ref(null)

/** 切类型：**顺手清掉选中**。不清的话，从"贴纸"切到"视频"时右栏还挂着上一个贴纸包
 * （右栏是按 `kind` 渲染的，数据却是上一个类型的）—— 会显示成"视频页里列着一堆图"。 */
function switchKind(k) {
  if (kind.value === k) return
  kind.value = k
  // 右栏只服务贴纸；切走就把选中清掉（右栏那时整条不渲染，留着旧数据没意义）。
  preview.value = null
}

/** 点一个贴纸包 → 拉整包图，填进右栏。
 *
 * ⚠️ **只服务贴纸**：视频已改回弹窗（用户口径），不在这里处理 —— 别再把视频分支加回来，
 * 那会让"右栏"同时承担两种形态。
 * ⚠️ **贴纸要现拉 detail**（列表只有 `count`）：整包的图只在这一刻拉一次。
 * ⚠️ 落结果前核对 `preview.value?.name`，否则点得快时慢响应会覆盖新选中。 */
async function selectStickerPack(it) {
  preview.value = { type: 'sticker', name: it.name, files: [] }
  try {
    const d = await stickersApi.detail(it.name)
    if (preview.value?.name !== it.name) return // 期间又点了别的包 ⇒ 丢弃这次结果
    preview.value.files = (d?.files || []).map((f) => ({
      name: f.name,
      url: stickersApi.fileUrl(it.name, f.name),
    }))
  } catch (e) {
    if (preview.value?.name === it.name) {
      preview.value.files = []
      toast('读取包内图片失败：' + e.message, 'err')
    }
  }
}

/** [`coverOf`] 的 await 版（点开时用：要拿到**确定**结果，不能拿占位空串）。 */
async function coverOfAsync(pack) {
  if (covers[pack]) return covers[pack]
  try {
    const d = await stickersApi.detail(pack)
    const first = (d?.files || [])[0]?.name
    covers[pack] = first ? stickersApi.fileUrl(pack, first) : ''
  } catch {
    covers[pack] = ''
  }
  return covers[pack]
}
const fonts = ref([])
// ⚠️ **每种素材各有各的根**，别只留一个 —— 视频/字体/贴纸的库是三棵不同的树
// （`<数据>/resources/{videos,fonts,stickers}`，多用户下再套一层 `users/<uid>/`）。
// 以前这里只有一份 `root`、且**永远取 video-library 那个**，于是切到"字体"页时
// 提示语照旧显示视频目录（"把字体文件拖到这里"下面写着 `…\resources\videos`）——
// **只是显示错了，写入是对的**（后端各按各的 kind 落盘）。这类"看着不对但不报错"
// 最容易让人以为传错地方，所以三个根分开存。
const roots = reactive({ video: '', font: '', sticker: '' })
const root = computed(() => roots[kind.value] || '')
const loading = ref(false)
const error = ref('')
const counts = computed(() => ({
  video: videos.value.length,
  sticker: packs.value.length,
  font: installedFontCount.value,
}))
const countText = computed(() => `${counts.value.video} 视频 · ${counts.value.sticker} 贴纸包 · ${counts.value.font} 字体`)

/** 已装字体数（`installed === false` 是"清单里有但没落盘"的自带字体，不该算可用）。 */
const installedFontCount = computed(() => fonts.value.filter((f) => f.installed !== false).length)

/**
 * 当前类型的网格数据。**三种素材被折成同一个形状**（`key/name/sub/title/thumb…`），
 * 所以模板里那一个网格能画三种东西 —— 加第四种素材时只改这里。
 */
const items = computed(() => {
  if (kind.value === 'video') {
    return videos.value.map((v) => ({
      key: v.name,
      name: v.name,
      sub: v.duration == null ? '读不出时长' : `${v.duration.toFixed(1)}s`,
      // ⚠️ 探不出时长的**照列不隐藏**（用户放进去了却看不到，比"能用但标出来"更难查），
      // 只是把后果说清楚。
      title: v.duration == null ? `${v.file}（ffprobe 读不出时长，片头/片尾里不能选它）` : v.file,
      badge: v.duration == null ? '不可用' : null,
      thumb: videoThumbUrl(v.file),
      url: assetUrl('video', v.file),
      // 右栏要用：文件名（显示 + 探大小）与时长（`m:ss`）
      file: v.file,
      duration: v.duration,
      type: 'video',
      detail: v.duration == null ? 'ffprobe 读不出时长，片头/片尾里选不了它。' : null,
    }))
  }
  if (kind.value === 'font') {
    return fonts.value.map((f) => ({
      key: f.id,
      name: f.display_name || f.id,
      sub: f.installed === false ? '未安装' : f.flat ? '老布局' : f.file,
      title: f.id,
      badge: f.is_default ? '默认' : null,
      stack: `"${f.family || f.id}", system-ui, sans-serif`,
      sample: sampleOf(),
      meta: `族名 ${f.family || f.id} · ${f.installed === false ? '还没落盘' : (f.bytes ? fmtBytes(f.bytes) : '')}`,
      type: 'font',
      // ⚠️ 自带字体**不可删**（服务端回 403）—— 前端就别给那颗按钮，而不是让用户点了再报错。
      // 判据是 `builtin`，与 `fonts::remove_font` 同源。
      actions: [
        f.is_default || f.installed === false
          ? null
          : { label: '默认', hint: '设为默认字体（只影响新工程的推荐位）', run: () => doSetDefault(f) },
        f.builtin ? null : { label: '删除', hint: '从字体库删除', run: () => doDeleteFont(f) },
      ].filter(Boolean),
    }))
  }
  // 贴纸：一行一个包（不是一张图一个格）—— 包内可能几百张，铺成格子会卡；
  // 这里用包里第一张当封面，点开看整包。
  return packs.value.map((p) => {
    return {
      key: p.name,
      name: p.name,
      // ⚠️ **列表接口只回 `count`，没有 `files`**（`files` 只有 `detail` 才回）。
      // 这里以前读 `(p.files || []).length` ⇒ **永远是 0**，界面显示"0 张"，
      // 而磁盘上其实有几百张（实测 ysyk = 210 张）。点开还会误报"这个贴纸包里没有图片"。
      sub: `${p.count || 0} 张${p.built ? '' : ' · 未构建'}`,
      title: p.description || p.name,
      badge: p.built ? null : '未构建',
      // 封面图要**现取**（列表不带文件名）：`coverOf` 只拉一次 detail 并缓存。
      thumb: coverOf(p.name),
      url: coverOf(p.name),
      type: 'sticker',
      detail: p.built ? null : '未构建：图能看，但 LLM 生成事件时读不到这个包。',
      actions: [
        p.built ? null : { label: '构建', hint: '让 LLM 读文件名生成语义 tag（要花时间与额度）', run: () => doBuildPack(p) },
        { label: '删除', hint: '删掉整个包（目录 + 清单 + 索引项）', run: () => doDeletePack(p) },
      ].filter(Boolean),
    }
  })
})

async function load(force = false) {
  loading.value = true
  error.value = ''
  try {
    // ⚠️ 素材库是**全局**资产（不在任何项目里），所以这三条请求**都不带 `{project, dir}` 作用域**
    // —— 别给 `api()` 硬塞一个作用域：那会去查一个跟素材库无关的项目。
    const [v, s, f] = await Promise.all([
      api('/api/polish/video-library', { label: '读取视频素材库' }),
      stickersApi.list(),
      api('/api/polish/fonts', { label: '读取字体库' }),
    ])
    videos.value = v?.videos || []
    packs.value = s?.packs || []
    fonts.value = f?.fonts || []
    // 三个根各取各的（都只对 admin 非空 —— 服务端刻意不给非 admin 回服务器路径）
    roots.video = v?.root || ''
    roots.sticker = s?.root || ''
    roots.font = f?.root || ''
    if (force) toast('素材库已刷新')
  } catch (e) {
    // ⚠️ 读失败**不伪装成空库**（否则用户会以为素材丢了、反复重放文件）。
    error.value = '素材库读取失败：' + e.message
  } finally {
    loading.value = false
  }
}

async function open(it) {
  // **视频：点击弹窗播放**（用户口径 2026-10-10：「视频还是改成点击弹出吧，视频的右侧栏删除」）。
  // 弹窗比定宽右栏大得多，看画面本来就更合适；那条 `<video>` 分支本来就在弹窗里，直接复用。
  if (it.type === 'video') {
    zoom.value = { type: 'video', title: it.name, url: it.url, detail: it.detail }
    return
  }
  // **字体：也走弹窗**（没有右栏，看大字形）。
  if (it.type === 'font') {
    zoom.value = { type: 'font', title: it.name, stack: it.stack, sample: it.sample, meta: it.meta, detail: it.detail }
    return
  }
  // **贴纸：进右侧预览栏**（在那儿列整包）。
  await selectStickerPack(it)
}

/** 从右栏的贴纸列表里点开一张 —— 大图仍走弹窗（列表格子小，看不清细节）。 */
function openStickerImage(f) {
  zoom.value = { type: 'sticker', title: f.name, url: f.url, detail: preview.value?.name }
}

const zoom = ref(null)

/* ---------------- 统一导入 ---------------- */

const dragOver = ref(false)
const busy = ref(false)
const doneCount = ref(0)
const totalCount = ref(0)
const fileInput = ref(null)

function onDragOver() {
  if (kind.value === 'video') return
  dragOver.value = true
}
function onDragLeave() {
  dragOver.value = false
}

async function onDrop(e) {
  dragOver.value = false
  if (kind.value === 'video') return
  const files = [...(e.dataTransfer?.files || [])]
  if (files.length) await importFiles(files)
}
async function onFilePick(e) {
  const files = [...(e.target?.files || [])]
  e.target.value = ''
  if (files.length) await importFiles(files)
}

/** 按当前类型分派导入。⚠️ 三条路的前端预检**必须在这里**（超限时服务端回 413 + 纯文本，
 *  而 `client.js` 只认 JSON，会把"文件太大"报成"接口不存在或返回格式错误"）。 */
async function importFiles(files) {
  if (kind.value === 'font') return importFonts(files)
  return importStickerZip(files[0])
}

async function importFonts(files) {
  const good = files.filter((f) => /\.(ttf|otf)$/i.test(f.name))
  const bad = files.filter((f) => !/\.(ttf|otf)$/i.test(f.name))
  if (bad.length) {
    toast(`已跳过 ${bad.length} 个非字体文件（只收 .ttf / .otf）`, 'warn')
  }
  if (!good.length) return
  const tooBig = good.find((f) => f.size > FONT_UPLOAD_MAX_BYTES)
  if (tooBig) {
    toast(`${tooBig.name} 超过 ${Math.round(FONT_UPLOAD_MAX_BYTES / 1024 / 1024)} MB 上限`, 'err')
    return
  }
  busy.value = true
  totalCount.value = good.length
  doneCount.value = 0
  const failed = []
  try {
    for (const f of good) {
      try {
        await uploadFont(f)
      } catch (e) {
        failed.push(`${f.name}：${e.message}`)
      }
      doneCount.value += 1
    }
  } finally {
    busy.value = false
  }
  await load()
  report(failed, `已导入 ${good.length - failed.length} 支字体`)
}

async function importStickerZip(file) {
  if (!file) return
  if (!/\.zip$/i.test(file.name)) {
    toast('贴纸包要传 zip 压缩包', 'err')
    return
  }
  if (file.size > UPLOAD_MAX_BYTES) {
    toast(`${file.name} 超过 ${Math.round(UPLOAD_MAX_BYTES / 1024 / 1024)} MB 上限`, 'err')
    return
  }
  const name = file.name.replace(/\.zip$/i, '')
  busy.value = true
  try {
    // 一律**不**顺带构建：构建是 LLM 任务、要花时间与额度，让用户在列表里显式点。
    // （上传成功与构建任务是两件事 —— 构建起不来不该让上传显示成失败。）
    await stickersApi.uploadPack(file, name, false)
  } catch (e) {
    toast('导入失败：' + e.message, 'err')
    return
  } finally {
    busy.value = false
  }
  await load()
  toast(`已导入贴纸包「${name}」，可在列表里点「构建」让 LLM 生成 tag`)
}

/** 统一汇报：成功的走 toast，失败的**逐条列出来**（只报"部分失败"等于没说）。 */
function report(failed, okText) {
  if (!failed.length) return toast(okText)
  toast(`${okText}，${failed.length} 个失败：${failed.join('；')}`, 'err')
}

/* ---------------- 管理动作 ---------------- */

async function doSetDefault(f) {
  busy.value = true
  try {
    await setDefaultFont(f.id)
  } catch (e) {
    return toast('设为默认失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
  await load()
  toast(`已把「${f.display_name || f.id}」设为默认（只影响新工程）`)
}

async function doDeleteFont(f) {
  // ⚠️ 二次确认：字体是"导入后不易得"的东西（几十 MB、还要去下载），误点一次就得重来。
  if (!(await confirmDialog(`删除字体「${f.display_name || f.id}」？`, { danger: true })))
    return
  busy.value = true
  try {
    await deleteFont(f.id)
  } catch (e) {
    return toast('删除失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
  await load()
  toast('字体已删除')
}

async function doBuildPack(p) {
  busy.value = true
  try {
    // 走任务队列（kind `sticker_build`），所以这里只是"派出去"，不等待完成。
    await stickersApi.build(p.name)
  } catch (e) {
    return toast('构建失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
  toast(`已派发「${p.name}」的构建任务，可在任务列表里看进度`)
}

async function doDeletePack(p) {
  if (!(await confirmDialog(`删除贴纸包「${p.name}」？包内 ${p.count || 0} 张图会一起删掉。`, { danger: true })))
    return
  busy.value = true
  try {
    await stickersApi.remove(p.name)
  } catch (e) {
    return toast('删除失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
  await load()
  toast('贴纸包已删除')
}

// 字节数 → 人话。⚠️ **全组件只有这一份**（以前我另加过一份，撞了 `Identifier already declared`）——
// 要改单位就改这里，别在别处再写一个。
const fmtBytes = (n) =>
  !n || n < 0
    ? '—' // 探不到（HEAD 失败 / 元数据读不出）就**照实说没拿到**，不显示 "0 KB" 那种假数
    : n > 1024 * 1024
      ? (n / 1024 / 1024).toFixed(1) + ' MB'
      : Math.round(n / 1024) + ' KB'

onMounted(() => load())
</script>

<style scoped>
/* 左导航 + 右内容：导航定宽、内容自适应。窄屏堆成上下两段（见下 media query）。 */
.ml-body { display:flex; gap:12px; align-items:flex-start; }
.ml-nav { display:flex; flex-direction:column; gap:4px; flex:none; width:128px; }
.ml-main { flex:1 1 auto; min-width:0; }

/* ---------- 右侧预览栏 ----------
   三栏：类型导航（128px）| 素材网格（自适应）| 预览栏（定宽）。
   ⚠️ 预览栏**定宽**而不是自适应：视频播放器需要一个稳定的宽高比参考，
   宽度一变、同一段视频每换一次选中就跳一次尺寸。 */
.ml-side {
  flex:none; width:600px;                 /* 用户口径：宽度翻倍（300 → 600） */
  display:flex; flex-direction:column; gap:8px;
  border-left:1px solid var(--line); padding-left:12px;
  min-height:220px;
}
.ml-side-empty {
  display:flex; align-items:center; justify-content:center;
  min-height:160px; text-align:center; padding:0 12px;
  border:1px dashed var(--line); border-radius:8px;
}
/* 包内图片列表：多列小图，可滚。一个包 200+ 张（ysyk = 210），
   不给最大高度会把整页撑到几屏长、左栏的网格被挤出视野。
   6 列是跟着 600px 的栏宽定的：列数少了每张图被拉得很大、一屏反而看得更少。 */
.ml-side-grid {
  display:grid; grid-template-columns:repeat(6, 1fr); gap:4px;
  max-height:460px; overflow-y:auto; padding-right:2px;
}
.ml-side-grid img {
  width:100%; aspect-ratio:1; object-fit:contain; cursor:zoom-in;
  border-radius:6px; background:var(--bg-soft); display:block;
}
.ml-side-grid img:hover { outline:1px solid var(--main); }

/* 选中态：左网格里当前在右栏显示的那一条。
   ⚠️ 必须**比 `:hover` 更特指**，否则鼠标一移开就看不见选的是谁。 */
.ml-cell-wrap.sel .ml-cell { border-color:var(--main); background:var(--main-soft); }

/* 左栏里的「样例文字」：跟类型导航同一列，不上浮到主区右上角。 */
.ml-sample-wrap { display:flex; flex-direction:column; gap:4px; margin-top:8px; padding:0 2px; }
.ml-sample-wrap .ml-sample { width:100%; }

.ml-tab {
  display:flex; align-items:center; gap:8px;
  padding:7px 10px; border-radius:7px; cursor:pointer;
  border:1px solid transparent; background:transparent;
  color:var(--text); font-size:13px; text-align:left;
}
.ml-tab:hover { background:var(--bg-soft); }
.ml-tab.on { background:var(--main-soft); border-color:var(--main); color:var(--pink-text); font-weight:600; }
.ml-ico { flex:none; }
.ml-lab { flex:1 1 auto; min-width:0; }
/* ⚠️ 计数必须 `flex:none`：写成可收缩时名字一长就被挤成一条竖线（同 `.src-badge` 那条教训）。 */
.ml-cnt { flex:none; font-size:11px; opacity:.65; font-variant-numeric:tabular-nums; }

/* 导入区：与网格同宽，虚线框。视频那档 `off` = 不响应拖拽（它不走上传）。 */
.ml-drop {
  display:flex; align-items:center; gap:10px;
  padding:10px 12px; border-radius:8px;
  border:1px dashed var(--line); background:var(--bg-soft);
  cursor:pointer;
}
.ml-drop.over { border-color:var(--main); background:var(--main-soft); }
.ml-drop.off { cursor:default; }
.ml-drop-t { display:flex; flex-direction:column; gap:2px; min-width:0; }
.ml-dir { overflow-wrap:anywhere; }

/* 网格：自适应列宽（`minmax` 让窄屏自动减列，不用媒体查询）。 */
.ml-grid { display:grid; grid-template-columns:repeat(auto-fill, minmax(108px, 1fr)); gap:8px; }
.ml-cell {
  display:flex; flex-direction:column; gap:4px; padding:6px;
  border:1px solid var(--line); border-radius:8px; background:var(--card);
  cursor:pointer; text-align:left; overflow:hidden;
}
.ml-cell-wrap { position:relative; display:flex; }
.ml-cell-wrap .ml-cell { flex:1 1 auto; min-width:0; }
.ml-cell:hover { border-color:var(--main); }
/* 管理动作条：默认隐身、悬停/键盘聚焦才出来（常显会把 108px 的格子塞满）。 */
.ml-acts { position:absolute; top:4px; right:4px; display:flex; gap:3px; opacity:0; transition:opacity .12s; }
.ml-cell-wrap:hover .ml-acts,
.ml-cell-wrap:focus-within .ml-acts { opacity:1; }
.ml-act {
  font-size:10px; padding:1px 5px; border-radius:4px; cursor:pointer;
  border:1px solid var(--line); background:var(--card); color:var(--ink-dim);
}
.ml-act:hover { border-color:var(--main); color:var(--pink-text); }
.ml-thumb {
  position:relative; display:flex; align-items:center; justify-content:center;
  height:72px; border-radius:6px; background:var(--bg-soft); overflow:hidden;
}
.ml-thumb img { max-width:100%; max-height:100%; object-fit:contain; }
.ml-font { font-size:22px; line-height:1; white-space:nowrap; }
.ml-badge {
  position:absolute; right:3px; bottom:3px;
  font-size:10px; padding:0 4px; border-radius:4px;
  background:var(--main); color:#fff;
}
.ml-name { font-size:12px; font-weight:600; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.ml-sub { font-size:11px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }

.ml-sample { width:150px; }
.ml-zoom { display:flex; flex-direction:column; align-items:center; gap:6px; }
.ml-zoom-video { max-width:100%; max-height:60vh; background:#000; border-radius:6px; }
.ml-zoom-img { max-width:100%; max-height:60vh; object-fit:contain; }
.ml-zoom-line { margin:0; font-size:34px; line-height:1.3; }
.ml-zoom-meta { margin:0; }

/* 窄屏：导航转成横排一行（128px 定宽在手机上是纯浪费）。 */
@media (max-width: 720px) {
  .ml-body { flex-direction:column; }
  .ml-nav { flex-direction:row; width:auto; flex-wrap:wrap; }
  /* 预览栏转成整行、并去掉那条竖分隔线（横排时它变成一条突兀的顶边） */
  .ml-side {
    width:100%; border-left:none; border-top:1px solid var(--line);
    padding-left:0; padding-top:12px;
  }
  .ml-side-grid { grid-template-columns:repeat(auto-fill, minmax(64px, 1fr)); max-height:320px; }
}
</style>
