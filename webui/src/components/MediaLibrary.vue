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
          @click="kind = t.key"
        >
          <Icon :name="t.icon" :size="15" class="ml-ico" />
          <span class="ml-lab">{{ t.label }}</span>
          <span class="ml-cnt">{{ counts[t.key] }}</span>
        </button>
      </nav>

      <section class="ml-main">
        <div class="row spread mb-8">
          <span class="muted small">{{ hint }}</span>
          <div class="row" style="flex-wrap:nowrap">
            <input
              v-if="kind === 'font'"
              v-model="testText"
              class="ml-sample"
              placeholder="样例文字（留空 = 永 Aa 123）"
            >
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
          <div v-for="it in items" :key="it.key" class="ml-cell-wrap">
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
    const first = (p.files || [])[0]
    return {
      key: p.name,
      name: p.name,
      sub: `${(p.files || []).length} 张${p.built ? '' : ' · 未构建'}`,
      title: p.description || p.name,
      badge: p.built ? null : '未构建',
      thumb: first ? stickersApi.fileUrl(p.name, first) : '',
      url: first ? stickersApi.fileUrl(p.name, first) : '',
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

function open(it) {
  if (it.type === 'font') {
    zoom.value = { type: 'font', title: it.name, stack: it.stack, sample: it.sample, meta: it.meta, detail: it.detail }
    return
  }
  if (!it.url) {
    // 贴纸包里一张图都没有：给一句人话，别弹一个空白框
    toast('这个贴纸包里没有图片')
    return
  }
  zoom.value = { type: it.type, title: it.name, url: it.url, detail: it.detail }
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
  if (!(await confirmDialog(`删除贴纸包「${p.name}」？包内 ${(p.files || []).length} 张图会一起删掉。`, { danger: true })))
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

const fmtBytes = (n) => (n > 1024 * 1024 ? (n / 1024 / 1024).toFixed(1) + ' MB' : Math.round(n / 1024) + ' KB')

onMounted(() => load())
</script>

<style scoped>
/* 左导航 + 右内容：导航定宽、内容自适应。窄屏堆成上下两段（见下 media query）。 */
.ml-body { display:flex; gap:12px; align-items:flex-start; }
.ml-nav { display:flex; flex-direction:column; gap:4px; flex:none; width:128px; }
.ml-main { flex:1 1 auto; min-width:0; }

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
  .ml-nav { flex-direction:row; width:auto; }
}
</style>
