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
                <!-- ⚠️ 第二个徽标位（2026-10-16）：字体页的「无粗体」与「默认」可能同时成立，
                     挤进同一个 `it.badge` 会丢掉一条信息。解释一律进 `title`（行内 ≤20 字）。 -->
                <span v-if="it.badge2" class="ml-badge warn" :title="it.badge2Hint">{{ it.badge2 }}</span>
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

      <!-- **右侧预览栏**（用户口径 2026-10-10 最终版）：
           · 视频 —— 点左边在这里**直接播放**，并显示文件名/时长/大小；
           · 贴纸 —— 在这里**列表显示**整包；
           · 字体 —— **没有右栏**（样例文字输入框在左侧），字体用网格里现场渲染的样例看效果。
           ⚠️ 显示条件是 `kind !== 'font'`（两个类型共用一条右栏，靠 `kind` 切内容）。
           ⚠️ 用户中途试过"视频改弹窗、右栏只留贴纸"，又改回来了 —— **别再自动改回去**。 -->
      <aside v-if="kind !== 'font'" class="ml-side">
        <template v-if="kind === 'video'">
          <div v-if="!preview" class="ml-side-empty muted small">
            点左边的视频，在这里预览播放
          </div>
          <template v-else>
            <!-- **外围框 + 里面按比例的视频**（用户口径：「加外围框，其实不冲突吧」—— 确实不冲突）：
                 框负责 `min-height` / `max-height`，视频在框内按自身比例撑满，
                 互不干涉 ⇒ 既能限制高度，又不会因为"框比视频高"而留下黑边。 -->
            <div class="ml-video-frame" :style="videoFrameInline">
              <!-- ⚠️ 比例**由视频自己报**（`loadedmetadata` 时读 `videoWidth/videoHeight`），
                   不写死 —— 以前写死 `16/9`，而库里的 `op.mp4` 是 320×240(4:3) ⇒ 框就是错的。
                   `@loadedmetadata` 而不是 `mounted`：那时才有真实分辨率。 -->
              <video
                :src="preview.url"
                controls
                class="ml-side-video"
                @loadedmetadata="onVideoMeta($event)"
              ></video>
            </div>
            <!-- 用户口径：「信息作为两排排版」（原来 9 行一路竖着排，又把栏撑高、又不好扫）。
                 **两列**：左列是"这个文件是什么"（名称/文件/时长/大小），
                 右列是"它有什么技术规格"（分辨率/编码/帧率/码率/音轨）—— 分栏本身就是分组。
                 ⚠️ 每一项都可能是 `—`：探不出来时**如实显示 —**，不留空、更不编一个数。 -->
            <div class="ml-side-info">
              <div class="ml-info-col">
                <div class="ml-info-row"><span class="muted">名称</span><b>{{ preview.name }}</b></div>
                <div class="ml-info-row"><span class="muted">文件</span><span class="mono ml-ell" :title="preview.file">{{ preview.file }}</span></div>
                <div class="ml-info-row"><span class="muted">时长</span><b>{{ preview.durationText }}</b></div>
                <div class="ml-info-row"><span class="muted">大小</span><b>{{ preview.sizeText }}</b></div>
              </div>
              <div class="ml-info-col">
                <div class="ml-info-row"><span class="muted">分辨率</span><b>{{ preview.dimText }}</b></div>
                <div class="ml-info-row"><span class="muted">编码</span><b>{{ preview.codecText }}</b></div>
                <div class="ml-info-row"><span class="muted">帧率</span><b>{{ preview.fpsText }}</b></div>
                <div class="ml-info-row"><span class="muted">码率</span><b>{{ preview.bitrateText }}</b></div>
                <div class="ml-info-row"><span class="muted">音轨</span><b>{{ preview.audioText }}</b></div>
              </div>
            </div>
          </template>
        </template>

        <template v-else>
        <div v-if="!preview" class="ml-side-empty muted small">
          点左边的贴纸包，在这里列出包内所有图
        </div>
        <template v-else>
          <div class="row spread mb-8" style="flex-wrap:nowrap">
            <b class="m-0">{{ preview.name }}</b>
            <span class="muted small">
              <!-- 筛选中时把"看到的 / 总共"都说出来 —— 只显示一个数会让人以为图少了 -->
              {{ preview.activeTag ? `${shownFiles.length} / ${preview.files.length} 张` : `${preview.files.length} 张` }}
            </span>
          </div>

          <!-- **标签筛选**：点一个标签只看那一类，再点一次取消。
               标签是**包内文件自己的** tags 聚合出来的（见 `selectItem`），
               所以"有哪些标签"永远和"图实际带了什么标签"一致。 -->
          <div v-if="preview.tagList.length" class="ml-tagbar">
            <button
              class="ml-tag"
              :class="{ on: !preview.activeTag }"
              @click="preview.activeTag = null"
            >全部 <span class="ml-tag-n">{{ preview.files.length }}</span></button>
            <button
              v-for="t in preview.tagList"
              :key="t"
              class="ml-tag"
              :class="{ on: preview.activeTag === t }"
              :title="`只看带「${t}」的图`"
              @click="preview.activeTag = preview.activeTag === t ? null : t"
            >{{ t }} <span class="ml-tag-n">{{ tagCount(t) }}</span></button>
          </div>

          <p v-if="!preview.files.length" class="muted small m-0">这个包里没有图片。</p>
          <p v-else-if="!shownFiles.length" class="muted small m-0">
            这个包里没有带「{{ preview.activeTag }}」标签的图。
          </p>
          <!-- ⚠️ `loading="lazy"`：一个包可能有 200+ 张（ysyk 就是 210），
               不懒加载会一次性全解码，整页卡住。 -->
          <div v-else class="ml-side-grid">
            <img
              v-for="f in shownFiles"
              :key="f.name"
              :src="f.url"
              :alt="f.name"
              :title="f.tags.length ? `${f.name} · ${f.tags.join(' / ')}` : f.name"
              loading="lazy"
              @click="openStickerImage(f)"
            >
          </div>
        </template>
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
import { computed, onMounted, reactive, ref, watch } from 'vue'
import Icon from './Icon.vue'
import AppModal from './AppModal.vue'
import { api } from '../api/client'
// ⚠️ 内建表情的 CDN 地址**只有一份**（engine.js::emojiGifUrl，与出片模板那句逐字同形）——
//    别在这里再拼一遍 'https://fonts.gstatic.com/...'。
import { emojiGifUrl } from '../polish/engine'
import {
  assetUrl,
  deleteFont,
  FONT_UPLOAD_MAX_BYTES,
  setDefaultFont,
  uploadFont,
  videoThumbUrl,
} from '../api/polish'
import { stickersApi, UPLOAD_MAX_BYTES } from '../api/stickers'
// ⚠️ **`hasBoldFace` 必须一起引**（2026-10-18 真 bug）：字体分支的「无粗体」徽标用它算判据，
//    而这里以前只引了 `ensureFontFace` ⇒ 切到「字体」页时 `items` 抛 ReferenceError ⇒
//    **Vue 把整块子树卸掉，素材库直接消失**（用户报的就是这个）。
//    ⚠️ 静态门禁抓不到这一类：esbuild 不为未定义全局变量报错，而
//    `check-layout-invariants.mjs` 那条「模板标识符必须有定义」只扫**模板**，这里是 `<script setup>` 里。
//    兜它的是 headless 的 `webui/scripts/check-polish-page-loads.mjs`（含素材库三个页签）。
import { ensureFontFace, hasBoldFace } from '../polish/fonts'
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
/** 内建表情目录：`[{tag, codepoints:[…]}]` —— 881 条按 **85 个 tag** 分组，
 *  而**同一个码点会挂在多个 tag 下**（那是分类视图，不是互斥分区）。 */
const emojiGroups = ref([])
/** 包封面：`{包名: 图片URL}`。**列表接口不带文件名**（只回 `count`），所以封面要现拉 detail。
 *
 * ⚠️ 缓存在这里、**每个包只拉一次**：`items` 是 computed，每次重算都会调 `coverOf`，
 * 不缓存就是"每渲染一次发一轮请求"（素材库一多就是几十连发）。
 * ⚠️ 拉失败**留空**（渲染成占位），不要拿别人的图顶上 —— 张冠李戴比没有图更难发现。 */
const covers = reactive({})
function coverOf(pack) {
  // 内建包：封面就是第一张动图（没有 detail 可拉，也别去拉 —— 那是一次注定 404 的请求）
  if (pack === EMOJI_PACK) return emojiCover.value
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
  // 右栏服务视频与贴纸；切到字体时整条不渲染，而且两类数据形状不同 ——
  // 不清的话从贴纸切到视频，右栏会拿着 `{files:[…]}` 去渲染 `<video>`。
  preview.value = null
}

/** 秒 → `m:ss`（视频时长）。探不出来时给 `—`，**不猜**。 */
function fmtDuration(sec) {
  if (!sec || sec <= 0) return '—'
  const t = Math.round(sec)
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`
}

/** 当前视频的宽高比（`null` = 还没拿到，用 16:9 兜底免得框塌成 0 高）。 */
const videoRatio = ref(null)

/** 框的内联样式：把视频比例写给**框**。
 *
 * ⚠️ **比例必须落在框上，不能只靠视频自己撑**：`.ml-side` 是 column flex，
 * 框没有确定高度时只写 `min/max-height` 会让它缩到 `min-height`（实测被钉在 200px，
 * 加宽栏完全没效果）。给框一个 `aspect-ratio` 之后高度就有依据了，
 * 再由 `min/max-height` 夹住上下限。
 *
 * ⚠️ 换视频时要**重置**，否则新视频先按上一个的比例闪一下（`null` ⇒ 16:9 兜底）。 */
const videoFrameInline = computed(() => {
  const r = videoRatio.value
  if (!r) return {}
  const style = { aspectRatio: String(r) }
  // ⚠️ **只有"不窄于 1:1"的视频才给宽度下限**：竖屏需要的是"窄而高"的框
  // （宽度由 `max-height` 反推），强行给 `min-width` 会让框比视频宽 ⇒ 两侧黑边（实测 24px）。
  if (r >= 1) style.minWidth = 'min(100%, 380px)'
  return style
})

/** `<video>` 报到真实分辨率 ⇒ 记下比例。拿不到就保持 `null`（兜底 16:9）。 */
function onVideoMeta(e) {
  const v = e?.target
  if (v?.videoWidth > 0 && v?.videoHeight > 0) {
    videoRatio.value = v.videoWidth / v.videoHeight
  }
}

/** 帧率 → `30 fps` / `29.97 fps`。**整数不显示小数**（`30 fps` 比 `30.00 fps` 清爽），
 * 探不出来给 `—`，**不猜**（`0 fps` 是编的）。 */
function fmtFps(v) {
  if (!v || v <= 0) return '—'
  // 取整后再比：`29.9999` 这种浮点误差不该显示成 `29.9999 fps`
  return `${Math.abs(v - Math.round(v)) < 0.005 ? Math.round(v) : v.toFixed(2)} fps`
}

/** 点一个贴纸包 → 拉整包图，填进右栏。
 *
 * ⚠️ **只服务贴纸**：视频已改回弹窗（用户口径），不在这里处理 —— 别再把视频分支加回来，
 * 那会让"右栏"同时承担两种形态。
 * ⚠️ **贴纸要现拉 detail**（列表只有 `count`）：整包的图只在这一刻拉一次。
 * ⚠️ 落结果前核对 `preview.value?.name`，否则点得快时慢响应会覆盖新选中。 */
async function selectItem(it) {
  if (it.type === 'video') {
    videoRatio.value = null // 换视频先清比例，等 loadedmetadata 再报（避免按上一个的比例闪一下）
    // ⚠️ **别拿 `it.key` 去 `videos` 里反查** —— `videos.value` 里放的是**接口原始对象**
    // （只有 `{name, file, duration}`，**没有 `key`**），`it.key` 是 `items` 计算时才造出来的。
    // 曾经写成 `videos.value.find(x => x.key === it.key)` ⇒ 永远 `undefined` ⇒
    // **点了视频右栏什么都不出现**。「有字段名就读」之前先 `Object.keys()` 看一眼。
    // `it` 自己就带齐了要用的字段，直接用。
    if (typeof it.file !== 'string' || !it.file) return
    preview.value = {
      type: 'video',
      name: it.name,
      file: it.file,
      url: it.url,
      durationText: fmtDuration(it.duration),
      sizeText: '…',
      // 技术信息直接来自列表接口（服务端已一次 ffprobe 探全），点选无延迟。
      // ⚠️ 每一项都可能缺失 ⇒ 显示侧逐项兜底成 `—`，**不要在这里编默认值**。
      dimText: it.width && it.height ? `${it.width}×${it.height}` : '—',
      codecText: it.codec ? String(it.codec).toUpperCase() : '—',
      fpsText: fmtFps(it.fps),
      bitrateText: it.bitrate ? `${(it.bitrate / 1e6).toFixed(1)} Mbps` : '—',
      audioText: it.has_audio === true ? '有' : it.has_audio === false ? '无（静音）' : '—',
    }
    // 大小是"锦上添花"：拿不到就显示 —，**不能因为探不到就让整条选中失败**。
    // ⚠️ 落结果前核对还是不是同一条，防止点得快时慢响应覆盖新选中。
    try {
      const res = await fetch(it.url, { method: 'HEAD' })
      const len = Number(res.headers.get('content-length') || 0)
      if (preview.value?.file === it.file) preview.value.sizeText = fmtBytes(len)
    } catch {
      if (preview.value?.file === it.file) preview.value.sizeText = '—'
    }
    return
  }
  // ⚠️ **内建 `emoji` 包没有 detail 接口可拉**（磁盘上没有这个包）—— 包内文件现从目录造，
  //    形状与下面那条**完全一致**（`{name, url, tags}`）⇒ 右栏那份渲染一个字都不用改，
  //    标签筛选栏也白拿（你那 85 个分类各自命中多少张，都是现算的）。
  //    ⚠️ 别在这里也走一次 `stickersApi.detail('emoji')` —— 那是一次注定 404 的请求。
  if (it.emoji) {
    preview.value = { type: 'sticker', name: EMOJI_PACK, files: [], tagList: [], activeTag: null, builtin: true }
    // **必须是 CDN 动图**（与出片模板的 CDN 回退、`engine.js::emojiGifUrl` 同一个地址）——
    // 静态字符那一版正是用户报的「polish编辑器内有，但是是静态的」。
    preview.value.files = emojiAll.value.map((e) => ({ name: e.cp, url: emojiGifUrl(e.cp), tags: e.tags }))
    const n = {}
    for (const f of preview.value.files) for (const t of f.tags) n[t] = (n[t] || 0) + 1
    preview.value.tagList = Object.keys(n).sort((a, b) => n[b] - n[a] || a.localeCompare(b, 'zh'))
    return
  }
  preview.value = { type: 'sticker', name: it.name, files: [], tagList: [], activeTag: null }
  try {
    const d = await stickersApi.detail(it.name)
    if (preview.value?.name !== it.name) return // 期间又点了别的包 ⇒ 丢弃这次结果
    // ⚠️ **把 `tags` 一起带上**（detail 接口本来就回了，以前丢掉了）——
    // 右栏的标签筛选全靠它。`tags` 缺失时给 `[]`，那张图就只在"全部"里出现。
    preview.value.files = (d?.files || []).map((f) => ({
      name: f.name,
      url: stickersApi.fileUrl(it.name, f.name),
      tags: Array.isArray(f.tags) ? f.tags : [],
    }))
    // 标签清单**从包内文件现算**，不用列表接口那份 —— 两份数据来源不同会打架
    // （列表的 `tags[].count` 是"命中文件数"，这里要的是"有哪些标签"）。
    // 顺序按命中数降序、同数按字面，保证同一份数据每次渲染顺序一致。
    const n = {}
    for (const f of preview.value.files) for (const t of f.tags) n[t] = (n[t] || 0) + 1
    preview.value.tagList = Object.keys(n).sort((a, b) => n[b] - n[a] || a.localeCompare(b, 'zh'))
  } catch (e) {
    if (preview.value?.name === it.name) {
      preview.value.files = []
      toast('读取包内图片失败：' + e.message, 'err')
    }
  }
}

/** 右栏当前该显示的贴纸（按标签筛过）。`activeTag` 为空 ⇒ 整包。 */
const shownFiles = computed(() => {
  const p = preview.value
  if (!p || p.type !== 'sticker') return []
  if (!p.activeTag) return p.files
  return p.files.filter((f) => f.tags.includes(p.activeTag))
})

/** 某个标签在这个包里命中多少张（标签栏上那个小数字）。 */
function tagCount(t) {
  const p = preview.value
  if (!p || p.type !== 'sticker') return 0
  return p.files.filter((f) => f.tags.includes(t)).length
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
/** **内建 emoji 包**：不是磁盘上的目录，而是编译期内嵌的 `assets/emoji_catalog.json`
 *  （881 条，走 `GET /api/polish/emoji-catalog`）；`emojiAll` 是它**去重后**的全量
 *  （码点 → 它挂着的那些 tag）。
 *
 *  ⚠️⚠️ **它是「贴纸页里的一个包」，不是第五种素材类型**（2026-10-17 用户订正：
 *  「emoji 这个表情是作为贴纸包存在的，你怎么给我单开一页？」）。编辑器那边它就是这么存在的 ——
 *  `StickerPicker` 的包列表里有一行固定的 `emoji`（「表情 · 内建」）。所以素材库照同一个模型：
 *  **贴纸页多一行 `emoji` 包**，点开在右栏列整包（与 ysyk 那条完全同一条路），
 *  只是包内文件来自目录、不是磁盘。**别再给它单开一个类型页**。
 *
 *  ⚠️ 汇总要去重：同一个码点会挂在多个 tag 下（那是分类视图，不是互斥分区），
 *  直接拼各组会重复（目录 881 条而各组之和 > 881）。 */
const EMOJI_PACK = 'emoji'
const emojiAll = computed(() => {
  const byCp = new Map()
  for (const g of emojiGroups.value) {
    for (const cp of (g.codepoints || [])) {
      let e = byCp.get(cp)
      if (!e) { e = { cp, tags: [] }; byCp.set(cp, e) }
      if (g.tag && !e.tags.includes(g.tag)) e.tags.push(g.tag)
    }
  }
  return [...byCp.values()]
})
/** 内建包的那张封面（第一张动图）—— 它没有 `stickersApi.detail` 可拉。 */
const emojiCover = computed(() => (emojiAll.value.length ? emojiGifUrl(emojiAll.value[0].cp) : ''))
const counts = computed(() => ({
  video: videos.value.length,
  // ⚠️ **内建包也算一个包**（网格里确实多一行）—— 不算的话「列表 2 行、计数 1」自相矛盾。
  sticker: packs.value.length + (emojiAll.value.length ? 1 : 0),
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
      // 右栏要用：文件名（显示 + 探大小）、时长（`m:ss`）与技术信息（服务端已探好）
      file: v.file,
      duration: v.duration,
      width: v.width,
      height: v.height,
      codec: v.codec,
      fps: v.fps,
      bitrate: v.bitrate,
      has_audio: v.has_audio,
      type: 'video',
      detail: v.duration == null ? 'ffprobe 读不出时长，片头/片尾里选不了它。' : null,
    }))
  }
  if (kind.value === 'font') {
    return fonts.value.map((f) => ({
      key: f.id,
      name: f.display_name || f.id,
      // ⚠️ **副标题以前是 `f.file`**（2026-10-16 用户报「字体列表有点紊乱」）：v2 布局下每一条的
      //    `file` 都是 `font.ttf` ⇒ 三张卡片副标题**一模一样**，看不出区别（看着还像坏了）。
      //    改成有信息量的来源：自带 / 导入（老平铺布局与未安装照旧单独标）。
      sub: f.installed === false ? '未安装' : f.flat ? '老布局' : f.builtin ? '自带' : '导入',
      title: f.id,
      badge: f.is_default ? '默认' : null,
      // ⚠️ **「无粗体」要说出来**（2026-10-16）：库里没有 `<族名> Bold` 时，字幕/元素的
      //    700·600 字重会被底座**退回常规体**（它不合成粗体）—— 而预览这一侧也显式关掉了
      //    合成（`.cap/.dm/.text-el { font-synthesis:none }`）⇒ 两边一致地细。
      //    以前这件事**只有出片日志里那句 warn 知道**：用户在界面上选 900，看着没变化，
      //    没有任何提示。判据与 Rust `find_font` 同源（`hasBoldFace`）。
      // ⚠️ **emoji 字体与未安装的不挂这枚徽标**（2026-10-16）：
      //    · emoji 字体不参与正文的字重（它只补表情的字形），标「无粗体」纯属噪音；
      //    · 还没落盘的条目谈"有没有粗体"没有意义（同族粗体可能就在下载列表里）。
      badge2: f.emoji || f.installed === false || hasBoldFace(fonts.value, f) ? null : '无粗体',
      badge2Hint: '库里没有「' + (f.family || f.id) + ' Bold」这一支 ⇒ 字幕与文字的 700·600 字重会退回常规体' +
        '（预览与成片一致地细）。要真粗体：导入一支同族名的 Bold 字体。',
      stack: `"${f.family || f.id}", system-ui, sans-serif`,
      sample: sampleOf(),
      meta: `族名 ${f.family || f.id} · ${f.installed === false ? '还没落盘' : (f.bytes ? fmtBytes(f.bytes) : '')}`,
      type: 'font',
      // ⚠️ 自带字体**不可删**（服务端回 403）—— 前端就别给那颗按钮，而不是让用户点了再报错。
      // 判据是 `builtin`，与 `fonts::remove_font` 同源。
      // ⚠️ **emoji 字体不给「设为默认」**（用户报「设为默认失败…NotoColorEmoji-CBDT」）：
      // emoji 是**独立的补充槽位**，不参与"正文默认位"的竞争 —— `scan_all` 出口就把它
      // 强制成 `is_default = false`（`fonts.rs` 的 A4 第三道拦截）。给它按钮 =
      // 点下去必然失败，而且会走到 `dir_for_id` 那条**误导性**的报错
      // （"不在目录布局里…先重新导入一次"，把这支好好的自带字体说成坏了）。
      // 判据 `f.emoji` 由接口直接给出，与 `set_default` 里 `meta.emoji` 同源。
      actions: [
        f.is_default || f.installed === false || f.emoji
          ? null
          : { label: '默认', hint: '设为默认字体（只影响新工程的推荐位）', run: () => doSetDefault(f) },
        f.builtin ? null : { label: '删除', hint: '从字体库删除', run: () => doDeleteFont(f) },
      ].filter(Boolean),
    }))
  }
  // 贴纸：一行一个包（不是一张图一个格）—— 包内可能几百张，铺成格子会卡；
  // 这里用包里第一张当封面，点开看整包。
  const rows = packs.value.map((p) => {
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
  // ⚠️ **末尾多一行内建 `emoji` 包**（2026-10-17，用户订正：emoji 是**作为贴纸包**存在的）。
  //    位置与编辑器 `StickerPicker` 的包列表一致（真实包在前、`emoji` 在最后）。
  //    它没有 `<包名>.toml`、也不在磁盘上 ⇒ `sub`/`title` 现算，`actions` 为空
  //    （「构建」「删除」对一条不存在于磁盘的包都不适用 —— 别给点了必然失败的按钮）。
  if (emojiAll.value.length) {
    rows.push({
      key: EMOJI_PACK,
      name: EMOJI_PACK,
      sub: emojiAll.value.length + ' 张 · 内建',
      title: '内建 Noto 动态表情：编译期内嵌、不联网',
      badge: null,
      thumb: emojiCover.value,
      url: emojiCover.value,
      type: 'sticker',
      emoji: true,
      detail: '内建：编译期内嵌、不联网 —— 精修里选「表情」来源时用的就是这一套。',
      actions: [],
    })
  }
  return rows
})

/**
 * 给字体库里每一支注册 FontFace —— **样例字要用它**。
 *
 * ⚠️ 幂等：`ensureFontFace` 自己按「名字|dir」缓存，重复调用不会重复取字节。
 * ⚠️ 只跳过**没落盘**的条目（`installed === false`）：`.ttc` 也试着注册 —— 认不出时浏览器
 *    会自己退回 `system-ui`（与下拉里"不进列表"那条判据不冲突：这里只是**展示**）。
 * ⚠️ 库接口是**程序级**的（与工程无关）⇒ `pid` 传 `null`、`dir` 传空串。
 */
async function registerLibraryFonts() {
  for (const f of fonts.value) {
    if (f.installed === false) continue
    const name = f.family || f.id
    await ensureFontFace(null, '', name, { onDebug: (m) => console.debug('[字体库]', m) })
  }
}

/** 切到「字体」页时才注册（视频/贴纸页不必为几 MB 字体买单）。 */
watch(kind, (k) => { if (k === 'font') registerLibraryFonts() })

async function load(force = false) {
  loading.value = true
  error.value = ''
  try {
    // ⚠️ 素材库是**全局**资产（不在任何项目里），所以这三条请求**都不带 `{project, dir}` 作用域**
    // —— 别给 `api()` 硬塞一个作用域：那会去查一个跟素材库无关的项目。
    const [v, s, f, em] = await Promise.all([
      api('/api/polish/video-library', { label: '读取视频素材库' }),
      stickersApi.list(),
      api('/api/polish/fonts', { label: '读取字体库' }),
      // ⚠️ **内建 `emoji` 包的内容**（贴纸页里那一行）：编译期内嵌、不属于任何工程 ⇒
      //    这条**不需要作用域**，别给它塞 {project, dir}（那会去查一个跟素材库无关的工程）。
      //    形状 {groups:[{tag, codepoints}]}。
      api('/api/polish/emoji-catalog', { label: '读取内建表情包' }),
    ])
    videos.value = v?.videos || []
    packs.value = s?.packs || []
    fonts.value = f?.fonts || []
    emojiGroups.value = em?.groups || []
    // ⚠️ **字体页的样例字要真的用那支字体渲染**（2026-10-16 修，用户报「字体列表有点紊乱」）：
    //    卡片上的 `it.stack` 写的是**族名**，而这一页**从未注册过 FontFace** ⇒ 浏览器根本不认识
    //    那些族名 ⇒ **每张卡片都用系统字体画样例**（三张卡看着一模一样，换样例文字也没区别）。
    //    注册名必须与 `stack` 里那个名字**逐字相同**（都是内部族名）—— 注册 id 而 CSS 用族名
    //    等于没注册（同一个坑在字幕预览那边也踩过）。
    if (kind.value === 'font') await registerLibraryFonts()
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
  // **字体没有右栏**（用户口径）：直接弹窗看大字形。
  if (it.type === 'font') {
    zoom.value = { type: 'font', title: it.name, stack: it.stack, sample: it.sample, meta: it.meta, detail: it.detail }
    return
  }
  // **视频与贴纸都进右侧预览栏**（视频在那儿直接播、贴纸在那儿列整包）。
  // ⚠️ 视频**不弹窗** —— 用户试过弹窗那条路又改回来了，别再自动改回去。
  await selectItem(it)
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
  /* 宽度：300 → 600（用户：「宽度翻倍」）→ **自适应**（用户：「播放器的高度还是没变」）。
     ⚠️ **不能写死像素**：视频高度 = 栏宽 ÷ 宽高比，所以想让画面更高就得让栏更宽 ——
     但写死 860px 在窄窗口里**只会横向溢出**，右侧被切掉、还得拖滚动条
     （实测：窗口 744px 时文档宽 1012px，就是这个毛病，用户截图里播放器被截断正是它）。
     改成"占窗口的 62%、至少 380px、至多 860px"：
       · 窗口够宽 ⇒ 拿满 860px（画面最高，这是用户要的）；
       · 窗口窄 ⇒ 跟着缩，**永远不溢出**。 */
  flex:none; width:clamp(380px, 62%, 860px);
  display:flex; flex-direction:column; gap:8px;
  border-left:1px solid var(--line); padding-left:12px;
  min-height:220px;
}
.ml-side-empty {
  display:flex; align-items:center; justify-content:center;
  min-height:160px; text-align:center; padding:0 12px;
  border:1px dashed var(--line); border-radius:8px;
}
/* 视频（右栏里的那只）。
   ⚠️ **尺寸要由视频自己决定，不能用 CSS 猜比例**（用户口径：「播放器要显示正确的尺寸」）：
     · `width:100%` —— 铺满栏宽；
     · `height:auto` —— 高度按视频**内在宽高比**算；
     · **不要写 `aspect-ratio`** —— 那是猜。我上一版写死 `16/9`，而库里的 `op.mp4`
       是 320×240(4:3) ⇒ 画面框比例就是错的（这条正是用户报的"尺寸不对"）。
     · **不要写 `object-fit`** —— 不写时元素本身就被视频撑成正确比例，
       黑边根本不会出现；写 `contain` 反而会在比例算错时留边。
     · `flex:none` —— `.ml-side` 是 column flex 容器，不加会被压扁（默认 `flex-shrink:1`）。
     · ⚠️ **必须显式解除 `max-height`** —— 这是"播放器高度怎么都改不了"的**真正原因**：
       `styles/app.css:241` 有一条**全局** `video { max-height:170px }`（给工程里那些小预览用的），
       它**依旧命中**这个元素 —— 我给自己的类设了 `width/height` 却**没管 `max-height`**，
       于是无论栏宽 300 / 600 / 860，播放器**永远只有 170px 高**。
       实测（宽度 860 与 460 各放一只）：两者都是 `848x170` / `448x170` —— 高度被钉死在 170。
       这也解释了为什么"加宽栏却没变高"：宽度确实变了，只是被 max-height 截回来了。
   实测：op.mp4(320×240) 在 860 栏下 → 860×636；probe.mp4(1920×1080) → 860×484。 */

/* 视频的**外围框**：负责"能有多大 / 至少多大"，不参与比例计算。
   · `max-height:70vh` —— 用户口径的上限。竖屏长视频（720×1280）在 860 栏下会到 1529px，
     不收住会把信息区顶到几屏之外、得滚动才看得见。
   · `min-height:200px` —— 用户口径的下限。窄窗口下 4:3 只算出 ~150px 时，
     不至于缩成一条缝（这条正是"起码能正常显示"）。
   · `display:flex` + `justify-content:center` —— 视频按比例缩进框里时**居中**，
     而不是贴着左边、右边空一块。 */
.ml-video-frame {
  flex:none; width:100%;
  /* 比例由 JS 写在行内 `aspect-ratio`（见 `videoFrameStyle`）—— 框**就是视频的形状**，
     所以不会有黑边。
     · `max-height:70vh`（用户口径的上限）：竖屏长视频在 860 栏下会到 1529px，
       不收住会把信息区顶出几屏。收住后框变矮、**宽度也跟着按比例收**（`aspect-ratio` 的效果），
       于是仍然没有黑边 —— 这正是"上限"和"无黑边"能同时成立的原因。
     · `min-height:200px`（用户口径的下限）：窄窗口下 4:3 只算出 ~150px 时不至于缩成一条缝。
       ⚠️ 上限与下限冲突时（如竖屏在很矮的窗口里）浏览器取 min 优先，框会略高于 70vh，
       这是有意的 —— "起码能正常显示"优先于严格上限。 */
  /* ⚠️ **`width:100%` + `aspect-ratio` 在"被 max-height 夹住"时会破功**：
     竖屏 720×1280 想要的框是 634×1127，但 `max-height:70vh` 把它压到 634 高 ——
     而 `width:100%` 是**硬性**的 ⇒ 宽度不跟着缩 ⇒ 视频只能缩到框中间，
     两侧留下 504px 黑边（实测就是这个数）。
     改成 `width:auto` + `height:100%` 也不行（框又会塌）。
     正解：**宽度交给 `aspect-ratio` 推**（`width:auto` + `height:auto` 由 max-height 决定，
     但宽度没有约束源）。所以给框加 `max-width:100%` 并把宽度放开为 `auto`，
     由 `aspect-ratio` + `max-height` 共同算出宽度。 */
  /* ⚠️ 宽度要 `auto` —— 只有 `auto` 才能让 `aspect-ratio` 真正接管宽度
     （竖屏那支靠 `max-height` 反推出一个**较窄**的框，`width:100%` 会挡住它 ⇒ 504px 黑边）。
     ⚠️ 但纯 `auto` 会让**小视频**的框塌到原生尺寸（320×240 的 `op.mp4` 框只有 320 宽，
     本该铺满栏宽）。所以给一个宽度下限 —— **只对"宽高比 ≥ 1"的视频加**
     （见模板上的 `style`：竖屏不能有这个下限，否则框比视频宽 ⇒ 24px 黑边，实测过）。
     `align-self:center`：宽框居中，不贴左边。 */
  width:auto;
  min-height:200px; max-height:70vh; max-width:100%;
  align-self:center;
  display:flex; align-items:center; justify-content:center;
  background:#000; border-radius:8px; overflow:hidden;
}
/* 框里的视频：**宽高都跟着框，并保持自身比例**。
   ⚠️ 关键在 `max-width:100%` + `max-height:100%` + `object-fit:contain` 这一组：
     两者同时成立时浏览器**按比例缩放**（不会拉伸），于是
       · 框比视频"方" ⇒ 视频按高度贴满、左右留极窄的边（框本来就贴合，几乎看不见）；
       · 框比视频"扁" ⇒ 视频按宽度贴满。
     这就是"完全覆盖视频、不要黑边"与"高度有上下限"能同时成立的原因：
     **框管尺寸、视频管比例**，两件事分开。
   ⚠️ `max-height:none` 仍然必要：全局 `styles/app.css:241` 的 `video{max-height:170px}`
     会命中这里 —— 那正是"播放器高度怎么都改不了"的真正原因（实测 860 栏下也只有 170px 高）。
     它由本规则里的 `max-height:100%` 覆盖（scoped 选择器特异性更高），这里再显式写一次防止
     将来有人调优先级时又漏掉。 */
.ml-side-video {
  display:block; margin:0;
  /* 框的形状已经等于视频比例了，视频**填满框**即可 —— `object-fit:contain` 是保险：
     万一比例还没报上来（兜底 16:9）时视频也不会被拉变形，只会暂时留一点边。 */
  width:100%; height:100%;
  object-fit:contain;
  border-radius:8px; background:transparent;
  /* ⚠️ **必写**：全局 `styles/app.css:241` 有 `video{max-height:170px}`，
     它正是"播放器高度怎么都改不了"的真正原因（实测 860 栏下也只有 170px 高）。 */
  max-height:none;
}
/* 信息**两列**（用户口径：「信息作为两排排版」）。
   ⚠️ `align-items:flex-start`：两列行数不同（左 4 行、右 5 行），不写会被拉成等高、
   左列多出一截空白。`gap:20px` 是列间距，比行间距大，让人一眼看出是**两栏**不是一串。 */
.ml-side-info { flex:none; display:flex; gap:20px; align-items:flex-start; }
.ml-info-col { flex:1 1 0; min-width:0; display:flex; flex-direction:column; gap:5px; }
.ml-info-row { display:flex; gap:8px; align-items:baseline; font-size:12px; }
.ml-info-row > .muted { flex:none; width:42px; }
.ml-info-row > b, .ml-info-row > span:not(.muted) { min-width:0; overflow-wrap:anywhere; }
.ml-ell { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }

/* 标签筛选栏。
   ⚠️ `flex-wrap:wrap` + **不给 max-height**：ysyk 有 34 个标签，写死高度会
   把标签藏起来（用户根本不知道还有别的标签）。让它自然换行、整栏跟着长。
   标签多时这一块会比较高，所以**排在图片列表上面**，图片列表自己滚（见 `.ml-side-grid`）。 */
.ml-tagbar { display:flex; flex-wrap:wrap; gap:4px; flex:none; }
.ml-tag {
  display:inline-flex; align-items:center; gap:4px;
  padding:2px 7px; border-radius:999px; cursor:pointer;
  font-size:11px; line-height:1.6;
  border:1px solid var(--line); background:transparent; color:inherit;
}
.ml-tag:hover { border-color:var(--main); }
/* 选中态：**底色 + 边框一起变**，只变边框在小标签上几乎看不出来。 */
.ml-tag.on { border-color:var(--main); background:var(--main-soft); color:var(--main); }
.ml-tag-n { opacity:.6; font-size:10px; }

/* 包内图片列表：多列小图，可滚。一个包 200+ 张（ysyk = 210），
   不给最大高度会把整页撑到几屏长、左栏的网格被挤出视野。
   8 列是跟着 860px 的栏宽定的：列数少了每张图被拉得很大、一屏反而看得更少。 */
.ml-side-grid {
  display:grid; grid-template-columns:repeat(8, 1fr); gap:4px;
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
/* 样张**整条都要看得见**（2026-10-19，用户报「字体列表显示的有问题」）：以前是 white-space:nowrap
   + 容器 overflow:hidden，97px 的格子放 186px 的样张 ⇒ 右边**半个字被硬裁、连省略号都没有**
   （截图 .workbuddy/fontlist/crop-three-cards.png）。现在允许折行、居中收在 72px 高度内
   —— 宁可换一行，不切字形。 */
.ml-font { font-size:22px; line-height:1.15; text-align:center; white-space:normal; overflow-wrap:anywhere; max-height:100%; overflow:hidden; }
.ml-badge {
  position:absolute; right:3px; bottom:3px;
  font-size:10px; padding:0 4px; border-radius:4px;
  background:var(--main); color:#fff;
}
/* ⚠️ 「无粗体」那枚挪到**左下角**（`.ml-badge` 是右下角绝对定位的，两枚同角会叠成一枚 ——
   第一版就是这样，白写一条信息）。配色走仓里既有的警示口径（`--amber-soft` / `--amber-deep`，
   两个主题都有定义；`App.vue` 的 `.pwd-banner` 与 `app.css` 的 `.chip.a` 同一套）。 */
.ml-badge.warn { right:auto; left:3px; background:var(--amber-soft); color:var(--amber-deep); }
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
    /* 竖排是**上下**两段，此时"栏宽"= 整行宽：`clamp(…, 62%, …)` 在这宽度下会算出
       一个比可视区窄的怪值（62% 是相对**父容器**，竖排后父容器是整个内容区）——
       直接放开成 100%。 */
    width:100%;
    border-left:none; border-top:1px solid var(--line);
    padding-left:0; padding-top:12px;
  }
  .ml-side-grid { grid-template-columns:repeat(auto-fill, minmax(64px, 1fr)); max-height:320px; }
  /* 竖排后栏宽 = 整行，但**窄窗口**下两列挤成一团（每列只剩一百多像素）⇒ 掉回一列。 */
  .ml-side-info { flex-direction:column; gap:5px; }
}

/* 中等宽度：栏还没到 860px，两列各自只剩 ~200px，`文件` 那种长值会被压得很窄。
   这时**并成一列**更好读 —— 用容器查询做不到（父容器是 flex 项，宽度由 clamp 决定），
   所以按窗口宽断一次。 */
@media (max-width: 1100px) {
  .ml-side-info { flex-direction:column; gap:5px; }
}
</style>
