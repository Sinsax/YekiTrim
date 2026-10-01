<template>
  <div class="card card-blk mb-12">
    <div class="row spread mb-8">
      <b class="t" style="margin:0"><Icon name="type" :size="15" /> 字体库</b>
      <span class="muted small">
        {{ fonts.length ? `已装 ${installedCount} 支${missing.length ? ` · 自带缺 ${missing.length} 支` : ''}` : '还是空的' }}
      </span>
      <div class="row" style="flex-wrap:nowrap">
        <!-- ⚠️ 自带字体是**内嵌**的（2026-10-02），正常永远不会走到这个按钮 ——
             它只在"用户把库里那支删了/落盘失败"之后出现，是**修复入口**，不是常规流程。 -->
        <button v-if="missing.length" class="mini-btn primary" :disabled="busy || downloading" @click="doDownload(null)">
          <span v-if="downloading" class="spinner"></span>
          {{ downloading ? '安装中…' : '重新装上自带字体' }}
        </button>
        <button class="mini-btn" :disabled="busy || downloading" @click="showPicker = !showPicker">
          <Icon name="plus" :size="13" /> 导入字体
        </button>
      </div>
    </div>

    <div class="hint">
      精修页的字幕与封面字体都从这里选；库里<b>程序自带的两支</b>不可删除（OFL 许可可商用）。
      默认字体只作<b>初始化设定</b>（老工程没显式选过字体时用它），改动请直接在各工程里选字体。
      <span v-if="root">库位置：<span class="mono">{{ root }}</span></span>
    </div>

    <!-- **拖拽导入**（2026-10-02 用户要求把导入改成拖字体文件）：
         dropzone 自己就是上传入口，不再需要先点「导入字体」再选文件。
         ⚠️ 浏览器里 `File` 对象**没有磁盘路径**，所以拖进来的文件不可能走 `{path}` 那条
         （那条要求"服务器上已存在的绝对路径"）—— 它走**字节上传** `POST /polish/fonts/upload`。
         两条入口都留着：拖拽 = 日常路径；「从服务器选」= 文件已经在服务器上时省一次上传。 -->
    <div
      class="drop mt-8"
      :class="{ over: dragOver, busy: busy || uploading }"
      @dragover.prevent="onDragOver"
      @dragleave.prevent="onDragLeave"
      @drop.prevent="onDrop"
      @click="fileInput?.click()"
    >
      <input ref="fileInput" type="file" accept=".ttf,.otf" multiple hidden @change="onFilePick" />
      <Icon name="plus" :size="16" />
      <div class="drop-t">
        <b>{{ uploading ? `上传中…（${uploadDone}/${uploadTotal}）` : '把字体文件拖到这里' }}</b>
        <span class="hint">或点击选择 · 支持多选 · 只收 .ttf / .otf</span>
      </div>
    </div>
    <p class="hint">
      <code>.ttc</code> / <code>.otc</code> 是字体集合，浏览器读不出，会被服务端拒绝。
      <button class="mini-btn" style="margin-left:6px" @click="showPicker = !showPicker">
        {{ showPicker ? '收起' : '文件已在服务器上？' }}
      </button>
    </p>
    <!-- 备用入口：从服务器上挑一个已存在的文件（路径由服务端拼） -->
    <template v-if="showPicker">
      <div class="imp-row mt-8">
        <span class="picked mono">{{ pickedDir ? pickedDir + ' / ' + pickedName : '还没选文件' }}</span>
        <input v-model="importName" class="nm" placeholder="显示名（可留空）" @keyup.enter="doImport" />
        <button class="mini-btn primary" :disabled="busy || !pickedName" @click="doImport">导入</button>
      </div>
      <ServerDirPicker
        mode="file"
        :exts="['ttf', 'otf']"
        @update:dir="(d) => (pickedDir = d)"
        @pick-file="(f) => (pickedName = f)"
      />
    </template>

    <div v-if="error" class="error-banner mt-8">
      <Icon name="alert" :size="15" /> {{ error }}
      <button class="mini-btn" style="flex:none" @click="load">重试</button>
    </div>

    <!-- 测试文本：一段共享文本喂给所有卡片试渲染（中英混排 + 数字，够看出字形差异） -->
    <div class="test-row mt-8">
      <label>测试文本</label>
      <input v-model="testText" placeholder="输入文字，所有字体卡片实时用它试渲染（默认：永 Aa 123）" />
    </div>

    <div v-if="loading" class="hint mt-8">载入中…</div>
    <div v-else-if="!fonts.length && !error" class="empty small mt-8">
      字体库是空的 —— 点右上角「导入字体」从服务器挑一支，或下载程序自带的两支。
    </div>

    <!-- 自带字体没下载时给一条「下载」入口（清单在服务端，前端不自己维护） -->
    <div v-if="missing.length" class="dl-bar mt-8">
      <span class="small">
        <b>自带字体还没下载</b>
        <span class="hint">（{{ missing.map((b) => b.display_name).join('、') }}，约 38 MB）</span>
      </span>
      <button class="mini-btn" :disabled="busy || downloading" @click="doDownload(null)">
        {{ downloading ? '下载中…' : '下载' }}
      </button>
    </div>

    <div v-if="fonts.length" class="grid mt-8">
      <article v-for="f in fonts" :key="f.id" class="fcard" :class="{ off: f.installed === false }">
        <!-- ⚠️ **emoji 字体只显示固定 emoji 样本**（2026-10-02 用户要求）：
             拿中文/拉丁测试文本喂它是**看不出任何东西**的（它没有文字字形，全是豆腐块），
             所以 v-if 分叉、两条路各喂各的样张。
             样本覆盖「常用 + 最易坏的组合」：ZWJ 家庭（序列拼合）、肤色（修饰符）、
             国旗（区域指示符对）、按键（数字+变体选择符）。
             ⚠️ 顺序有讲究：这些是**多码点序列**，任一环节不支持就整串散架，一眼能看出。 -->
        <div v-if="isEmojiOnly(f)" class="preview emoji" :style="{ fontFamily: previewStack(f) }"
             :title="'emoji 试渲染：' + EMOJI_SAMPLE">{{ EMOJI_SAMPLE }}</div>
        <div v-else class="preview" :style="{ fontFamily: previewStack(f) }">{{ testText.trim() || '永 Aa 123' }}</div>
        <div class="meta">
          <div class="name" :title="f.id">{{ f.display_name || f.id }}</div>
          <!-- ⚠️ 内部族名与显示名**分开显示**：两者不一致时正是"预览与实际对不上"的现场 -->
          <div class="fam" :title="'字体内部族名：' + (f.family || '(读不出)')">
            内部族名：{{ f.family || '(读不出)' }}
          </div>
          <div class="tags">
            <span v-if="f.builtin" class="chip p">程序自带</span>
            <span v-if="f.is_default" class="chip g">默认</span>
            <span v-if="f.flat" class="chip n" title="散文件直接放在库目录（老布局），不是导入/下载进来的库字体。">老布局</span>
            <span v-if="f.license" class="chip n" :title="f.license">{{ f.license }}</span>
            <!-- ⚠️ **职责标识**（2026-10-02 用户要求"正确标注是字体还是 emoji 或者都有"）：
                 文字 / emoji / 两者都有。判据来自字体**实际能力**（见 `roleOf`），不是 FONT.toml 里
                 那个手写的 `emoji` 布尔 —— 两者不一致时以实际能力为准（那正是会出事的现场）。 -->
            <span class="chip" :class="roleChip(f).cls" :title="roleChip(f).title">{{ roleChip(f).text }}</span>
            <!-- 彩色格式：**只用 emoji 的那支才标**。纯文字字体标 `none` 没有意义（那是正常的） -->
            <span v-if="isEmojiCapable(f)" class="chip n" :title="colorHint(f)">{{ colorLabel(f) }}</span>
            <span v-if="f.installed === false" class="chip r">未装上</span>
            <span v-else-if="f.ok === false" class="chip r" title="浏览器不认这个字体格式，预览回退到了系统字体">预览不可用</span>
          </div>
        </div>
        <div class="acts">
          <!-- 没下载的自带字体：只能下载（设为默认/删除都无从谈起） -->
          <button v-if="f.installed === false" class="mini-btn" :disabled="busy || downloading" @click="doDownload(f)">
            {{ downloading ? '安装中…' : '装上' }}
          </button>
          <template v-else>
            <!-- **「设为默认」按职责显示**（2026-10-02 用户口径）：
                 文字字体 → 「设为默认文字」；emoji 字体 → 「设为默认 emoji」；两者都有 → 两颗都给。
                 ⚠️ 默认位只做**初始化设定**（老工程没显式选过字体时用它），不是"一改全改" ——
                 文案里把那两个字说清楚，免得用户以为点了它所有工程的字幕都会变。
                 ⚠️ 「两者都有」的两颗按钮操作的是**同一个 `is_default` 状态**（服务端只有唯一默认位），
                 所以两颗会同时点亮 —— 这是如实反映，不是 bug。 -->
            <button v-if="canBeTextDefault(f)" class="mini-btn" :disabled="busy || f.is_default"
              :title="f.is_default ? '已经是默认文字字体' : '设为默认文字（只影响没显式选过字体的工程）'"
              @click="doDefault(f)">{{ f.is_default ? '默认文字' : '设为默认文字' }}</button>
            <button v-if="canBeEmojiDefault(f)" class="mini-btn" :disabled="busy || f.is_default"
              :title="f.is_default ? '已经是默认 emoji 字体' : '设为默认 emoji（emoji 缺字时用它补）'"
              @click="doDefault(f)">{{ f.is_default ? '默认 emoji' : '设为默认 emoji' }}</button>
            <button
              class="mini-btn danger"
              :disabled="busy || f.builtin"
              :title="f.builtin ? '程序自带字体不可删除' : '从库里删除（不碰任何工程的配置）'"
              @click="doDelete(f)"
            >删除</button>
          </template>
        </div>
      </article>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { fetchFonts, importFont, uploadFont, setDefaultFont, deleteFont, downloadBuiltinFonts } from '../api/polish.js'
import { confirmDialog } from '../stores/dialog.js'
import ServerDirPicker from './ServerDirPicker.vue'
import Icon from './Icon.vue'

const fonts = ref([])
const root = ref('')
const loading = ref(false)
const busy = ref(false)
const error = ref('')
const importName = ref('')
const showPicker = ref(false)
const pickedDir = ref('')
const pickedName = ref('')
/** 测试文本：空 = 各卡片用默认样张「永 Aa 123」。中英数字混排才看得出字形/字重差异。 */
const testText = ref('')
const downloading = ref(false)

/** emoji 试渲染样本（固定，2026-10-02 用户要求 emoji 字体只显示固定 emoji）。
 *  选样覆盖「常用 + 最易坏的组合」——单看一个笑脸看不出字体好坏：
 *  ZWJ 家庭（多码点序列拼合）、肤色（修饰符）、国旗（区域指示符对）、按键（数字+变体选择符）。
 *  ⚠️ 任一环节不支持就整串散架，这正是要一眼看出来的东西。 */
const EMOJI_SAMPLE = '😀🚀❤️🎉🌈👍🔥👨‍👩‍👧‍👦👍🏻🇨🇳1️⃣'

/** 拖拽态（高亮用）与上传进度。 */
const dragOver = ref(false)
const uploading = ref(false)
const uploadDone = ref(0)
const uploadTotal = ref(0)
const fileInput = ref(null)

/** 这支字体有没有彩色字形能力（判据来自服务端读字体表，见 fonts::color_glyph_support）。
 *  ⚠️ color_format 为 none 对纯文字字体是正常的，不是错误 —— 别拿它当反面判据。 */
const hasColor = (f) => !!f.color_format && f.color_format !== 'none'

/** 是不是只能当 emoji 用（有彩色字形、没有文字字形）。
 *  这种字体喂中文只会出豆腐块，所以卡片预览要换成固定 emoji 样本。
 *  ⚠️ 判据用「emoji 标记 + 有彩色」；只看 f.emoji 会把两者都有的字体也误判成 emoji-only。 */
const isEmojiOnly = (f) => !!f.emoji && hasColor(f)
/** 能当文字默认（emoji-only 的字体不能排正文，服务端 set_default 也拒绝）。 */
const canBeTextDefault = (f) => f.installed !== false && !isEmojiOnly(f)
/** 能当 emoji 默认（必须真有彩色字形 —— 纯文字字体补不出表情）。 */
const canBeEmojiDefault = (f) => f.installed !== false && hasColor(f)
/** 只在卡片上标彩色格式时用（纯文字字体不标 —— 那个 none 是正常的）。 */
const isEmojiCapable = (f) => hasColor(f)

/** 彩色能力 → 角标文案。 */
function colorLabel(f) {
  const t = f.color_format
  return t === 'cbdt' ? 'CBDT' : t === 'sbix' ? 'sbix' : t === 'colrv0' ? 'COLRv0' : t === 'colrv1' ? 'COLRv1' : t === 'svg' ? 'SVG' : t
}
/** 彩色能力 → 悬停说明。⚠️ COLRv1 / SVG 是**会出事**的两档，必须把后果说清。 */
function colorHint(f) {
  const t = f.color_format
  if (t === 'cbdt') return 'CBDT 位图彩色字形 —— 出片渲染器实测支持，emoji 正常'
  if (t === 'sbix') return 'sbix 位图彩色字形 —— 实测支持'
  if (t === 'colrv0') return 'COLRv0 矢量彩色字形 —— 实测支持'
  if (t === 'colrv1') return 'COLRv1 —— 出片渲染器只认 COLRv0，用它会 emoji 整片消失（不报错）'
  if (t === 'svg') return 'SVG 内嵌 —— 出片渲染器不支持，emoji 不会出现'
  return ''
}
/** 职责角标文案（用户要求正确标注是字体还是 emoji 或者都有）。 */
function roleChip(f) {
  if (isEmojiOnly(f)) return { text: 'emoji', cls: 'b', title: '只能当 emoji 用：有彩色字形、没有文字字形' }
  if (hasColor(f)) return { text: '文字 + emoji', cls: 'p', title: '两者都有：既能排正文，也能出彩色表情' }
  return { text: '文字', cls: 'n', title: '文字字体：只排正文，不含彩色 emoji 字形' }
}

const installedCount = computed(() => fonts.value.filter((f) => f.installed !== false).length)
/** 还没下载的自带字体（清单来自服务端；界面据此给「下载」入口）。 */
const missing = computed(() => fonts.value.filter((f) => f.installed === false))

/** 预览字形栈：库里注册的同一支族名（与出片 @font-face 同源）。 */
const previewStack = (f) => `"${f.family || f.id}", system-ui, sans-serif`

/** 预览用：把库里字体现场注册进浏览器（**同一支字节**，与出片注入的 @font-face 同源）。
 *  ⚠️ 必须遍历 `fonts.value`（响应式代理）而不是 load() 里的原始数组：
 *  写在原始对象上的 `f.ok` 模板看不见，chip 会永久卡在「预览不可用」（实测踩过）。
 *  注册进行中不显示任何 chip —— 只有真的失败（`f.ok === false`）才挂「预览不可用」。 */
async function registerAll() {
  for (const f of fonts.value) {
    if (f.ok === false) continue
    try {
      const res = await fetch(f.url)
      if (!res.ok) { f.ok = false; continue }
      const face = new FontFace(f.family || f.id, await res.arrayBuffer())
      await face.load()
      document.fonts.add(face)
      f.ok = true
    } catch {
      // 浏览器认不出（老 .ttc 就是这条）→ 卡片上照实说，别让用户以为"预览换好了"
      f.ok = false
    }
  }
}

async function load() {
  loading.value = true
  try {
    const r = await fetchFonts('首页字体库')
    const list = (r && r.fonts) || []
    // 每支一个可直接拉的字节 URL（预览与出片取的是同一份）
    for (const f of list) f.url = '/api/polish/font?name=' + encodeURIComponent(f.id)
    fonts.value = list
    root.value = (r && r.root) || ''
    error.value = ''
    registerAll()
  } catch (e) {
    // 读失败可见 + 保留上一次列表（不伪装成"库是空的"）
    error.value = '字体库读取失败：' + (e && e.message ? e.message : e)
  } finally {
    loading.value = false
  }
}

async function act(fn) {
  busy.value = true
  try { await fn() } finally { busy.value = false }
}

/** 拖拽进入/离开：只在**真正离开**容器时灭高亮（拖过子元素也会触发 dragleave，
 *  不做这层判断会不停闪）。
 *  ⚠️ `relatedTarget` 在 dragleave 上可能为 null（跨窗口拖入时），所以判 null 就当真离开。 */
function onDragOver(e) {
  e.dataTransfer.dropEffect = 'copy'
  dragOver.value = true
}
function onDragLeave(e) {
  if (!e.relatedTarget || !e.currentTarget.contains(e.relatedTarget)) dragOver.value = false
}

/** 从前端预挡一遍明显的非字体：服务端也会拒，但先挡能省掉几十 MB 的上传。
 *  ⚠️ 判据只看扩展名 —— 真正的校验（能不能解析出族名）在服务端，别在前端假装能判。 */
const looksLikeFont = (name) => /\.(ttf|otf)$/i.test(name)

async function onDrop(e) {
  dragOver.value = false
  await doUpload(Array.from((e.dataTransfer && e.dataTransfer.files) || []))
}
function onFilePick(e) {
  const files = Array.from((e.target && e.target.files) || [])
  // 清掉 value：不清的话连续选同一个文件不会再触发 change（用户会以为没反应）
  if (e.target) e.target.value = ''
  doUpload(files)
}

/** **上传字体文件**（拖拽或选文件）。逐个传，一个失败不挡后面的 ——
 *  多个文件时只报第一个错，但成功的那些照样进库。
 *  ⚠️ 走字节上传（`POST /polish/fonts/upload`）：浏览器里的 `File` 没有磁盘路径，
 *  拿不到 import 那条需要的 `{path}`。 */
async function doUpload(files) {
  const list = (files || []).filter((f) => f && f.size > 0)
  if (!list.length) return
  const bad = list.filter((f) => !looksLikeFont(f.name))
  const good = list.filter((f) => looksLikeFont(f.name))
  if (bad.length) {
    error.value = `只收 .ttf / .otf：${bad.map((f) => f.name).join('、')}`
    if (!good.length) return
  } else {
    error.value = ''
  }
  uploading.value = true
  uploadTotal.value = good.length
  uploadDone.value = 0
  try {
    for (const f of good) {
      try {
        await uploadFont(f)
      } catch (err) {
        // 失败原因原样显示（只收 ttf/otf / 读不出族名 / 太大）—— 三种处置不一样，别吞
        error.value = `「${f.name}」上传失败：` + (err && err.message ? err.message : err)
      }
      uploadDone.value += 1
    }
    await load()
  } finally {
    uploading.value = false
    uploadTotal.value = 0
    uploadDone.value = 0
  }
}

/** 设为默认（按职责按钮调的都是它 —— 服务端只有**唯一**一个默认位）。
 *  ⚠️ 只影响初始化：老工程没显式选过字体时用它，不动任何工程的配置。 */
const doDefault = (f) => act(async () => {
  try { await setDefaultFont(f.id); await load() }
  catch (e) { error.value = e && e.message ? e.message : String(e) }
})

const doImport = () => act(async () => {
  try {
    await importFont({ dir: pickedDir.value, fileName: pickedName.value, name: importName.value.trim() })
    pickedName.value = ''
    importName.value = ''
    showPicker.value = false
    await load()
  } catch (e) {
    error.value = '导入失败：' + (e && e.message ? e.message : e)
  }
})

const doDelete = async (f) => {
  const okd = await confirmDialog(
    `从字体库删除「${f.display_name || f.id}」？
已选它的工程会回退到系统字体栈。`,
    { title: '删除字体', danger: true },
  )
  if (!okd) return
  await act(async () => {
    try { await deleteFont(f.id); await load() }
    catch (e) { error.value = e && e.message ? e.message : String(e) }
  })
}

/** 装上自带字体（`id` 缺省 = 装全部还缺的）。**内嵌字节、不联网**，幂等 —— 已装好的服务端会跳过。
 *  ⚠️ 正常走不到这里（首次启动已自动装好），是**修复入口**：用户把库里那支删了之后补回来。 */
async function doDownload(f) {
  downloading.value = true
  error.value = ''
  try {
    await downloadBuiltinFonts(f ? f.id : '')
    await load()
  } catch (e) {
    // 失败原因（网络 / 校验和不符 / 上游改了内容）原样显示：三种处置不一样，别吞
    error.value = '下载失败：' + (e && e.message ? e.message : e)
  } finally {
    downloading.value = false
  }
}

onMounted(load)
</script>

<style scoped>
/* 拖拽导入区：整块可点（点 = 选文件），拖入时高亮。
   ⚠️ 高亮态要明显到「一眼知道松手会落到这里」，只改边框颜色不够。 */
.drop {
  display: flex; align-items: center; gap: 10px; padding: 14px 16px;
  border: 1.5px dashed var(--line); border-radius: 10px;
  background: var(--bg-soft); cursor: pointer; transition: border-color .15s, background .15s;
}
.drop:hover { border-color: var(--main); }
.drop.over { border-color: var(--main); background: var(--main-soft); }
.drop.busy { cursor: progress; opacity: .8; }
.drop .drop-t { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.drop .drop-t b { font-size: 13px; }

.imp-row { display: flex; align-items: center; gap: 8px; }
.imp-row .picked {
  flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  font-size: 11px; color: var(--ink-dim);
}
.imp-row input {
  flex: 0 0 160px; padding: 5px 9px; border: 1px solid var(--line);
  border-radius: 6px; font-size: 12px;
}
.test-row { display: flex; align-items: center; gap: 8px; }
.test-row label { flex: none; font-size: 12px; color: var(--ink-dim); }
.test-row input {
  flex: 1 1 auto; min-width: 0; padding: 5px 9px;
  border: 1px solid var(--line); border-radius: 6px; font-size: 12px;
}
.dl-bar {
  display: flex; align-items: center; justify-content: space-between; gap: 12px;
  padding: 8px 12px; border-radius: 8px; background: var(--main-soft);
}
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(210px, 1fr)); gap: 10px; }
.fcard {
  display: flex; flex-direction: column; gap: 6px; padding: 10px;
  border: 1px solid var(--line); border-radius: 10px; background: var(--card);
}
.preview { padding: 6px 0; font-size: 22px; text-align: center; background: var(--bg-soft); border-radius: 8px; }
/* emoji 卡片预览：字号略大（位图 emoji 在小字号下细节全糊）、略收行高免得撑高卡片 */
.preview.emoji { font-size: 20px; line-height: 1.5; letter-spacing: 1px; }
.meta { min-height: 56px; }
.name { font-size: 13px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.fam {
  margin-top: 2px; font-size: 11px; color: var(--ink-dim);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.tags { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 6px; }
.acts { display: flex; gap: 6px; }
.acts .mini-btn { flex: 1 1 auto; }
.acts .mini-btn:disabled { opacity: 0.45; cursor: not-allowed; }
/* emoji 卡片的「自动补充」：占「设为默认」那一格，但**不是按钮**（没什么可点的）。
   ⚠️ 用 disabled 按钮表达会让人以为"条件不满足、满足后能点" —— 这里就是不需要那个操作。 */
.acts .mini-btn.ghost {
  display: inline-flex; align-items: center; justify-content: center;
  opacity: 0.6; cursor: default; pointer-events: none;
}
/* 未下载的自带字体：整张卡压暗，一眼看出"这支还不能用" */
.fcard.off { opacity: 0.72; }
</style>
