<template>
  <div class="wrap">
    <header class="hd">
      <h2>字体库</h2>
      <p class="sub">
        程序级字体库 —— 精修页的字幕与封面字体都从这里选。
        库里<b>程序自带</b>的那支不可删除。
      </p>
    </header>

    <!-- 读失败：可见 + 保留上一次列表 + 重试（不伪装成"库是空的"） -->
    <div v-if="error" class="err">
      <span>{{ error }}</span>
      <button class="mini-btn" @click="load">重试</button>
    </div>

    <section class="import">
      <label>导入字体（从服务器上挑一个文件）</label>
      <div class="row">
        <button class="mini-btn" @click="showPicker = !showPicker">{{ showPicker ? '收起' : '浏览…' }}</button>
        <span class="picked">{{ pickedDir ? pickedDir + ' / ' + pickedName : '还没选文件' }}</span>
        <input v-model="importName" class="nm" placeholder="显示名（可留空）" @keyup.enter="doImport" />
        <button class="mini-btn" :disabled="busy || !pickedName" @click="doImport">导入</button>
      </div>
      <!-- ⚠️ 选文件用 ServerDirPicker（与「表情包 → 添加」「选择素材」同一份）：
           路径**由服务端拼**，前端只回传 {dir, name} -->
      <ServerDirPicker
        v-if="showPicker"
        mode="file"
        :exts="['ttf', 'otf']"
        @update:dir="(d) => (pickedDir = d)"
        @pick-file="(f) => (pickedName = f)"
      />
      <p class="hint">
        只收 <code>.ttf</code> / <code>.otf</code>；<code>.ttc</code> 字体集合浏览器认不出，会被拒绝。
        <span v-if="root">库位置：{{ root }}</span>
      </p>
    </section>

    <p v-if="loading" class="hint">载入中…</p>
    <p v-else-if="!fonts.length" class="hint">字体库是空的 —— 导入一支，或把字体文件放进上面的库目录。</p>

    <!-- 自带字体没下载时给一条「下载」入口（清单在服务端，前端不自己维护） -->
    <div v-if="missing.length" class="dl-bar">
      <div>
        <strong>自带字体还没下载</strong>
        <span class="hint">（{{ missing.map((b) => b.display_name).join('、') }}，OFL 许可可商用，约 38 MB）</span>
      </div>
      <button class="mini-btn" :disabled="busy || downloading" @click="doDownloadAll">
        {{ downloading ? '下载中…' : '下载自带字体' }}
      </button>
    </div>

    <div class="grid">
      <article v-for="f in fonts" :key="f.id" class="card" :class="{ off: f.installed === false }">
        <div class="preview" :style="{ fontFamily: previewStack(f) }">永 Aa 123</div>
        <div class="meta">
          <div class="name" :title="f.id">{{ f.display_name || f.id }}</div>
          <!-- ⚠️ 内部族名与显示名**分开显示**：两者不一致时正是"预览与实际对不上"的现场 -->
          <div class="fam" :title="'字体内部族名：' + (f.family || '(读不出)')">
            内部族名：{{ f.family || '(读不出)' }}
          </div>
          <div class="tags">
            <span v-if="f.builtin" class="tag builtin">程序自带</span>
            <span v-if="f.is_default" class="tag def">默认</span>
            <span v-if="f.license" class="tag" :title="f.license">{{ f.license }}</span>
            <span v-if="f.emoji" class="tag">emoji</span>
            <span v-if="f.installed === false" class="tag warn">未下载</span>
            <span v-else-if="!f.ok" class="tag warn" title="浏览器不认这个字体格式，预览回退到了系统字体">预览不可用</span>
          </div>
        </div>
        <div class="acts">
          <!-- 没下载的自带字体：只能下载（设为默认/删除都无从谈起） -->
          <button v-if="f.installed === false" class="mini-btn" :disabled="busy || downloading" @click="doDownload(f)">
            {{ downloading ? '下载中…' : '下载' }}
          </button>
          <template v-else>
            <button class="mini-btn" :disabled="busy || f.is_default" @click="doDefault(f)">
              {{ f.is_default ? '已是默认' : '设为默认' }}
            </button>
            <button
              class="mini-btn"
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
import { fetchFonts, importFont, setDefaultFont, deleteFont, downloadBuiltinFonts } from '../api/polish.js'
import { confirmDialog } from '../stores/dialog.js'
import ServerDirPicker from '../components/ServerDirPicker.vue'

const fonts = ref([])
const root = ref('')
const loading = ref(false)
const busy = ref(false)
const error = ref('')
const importName = ref('')
const showPicker = ref(false)
const pickedDir = ref('')
const pickedName = ref('')
const downloading = ref(false)

/** 还没下载的自带字体（清单来自服务端；界面据此给「下载」入口）。 */
const missing = computed(() => fonts.value.filter((f) => f.installed === false))

/** 预览用：把库里字体现场注册进浏览器（**同一支字节**，与出片注入的 @font-face 同源）。 */
const previewStack = (f) => `"${f.family || f.id}", system-ui, sans-serif`

async function registerAll(list) {
  for (const f of list) {
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
    const r = await fetchFonts('字体库')
    const list = (r && r.fonts) || []
    // 每支一个可直接拉的字节 URL（预览与出片取的是同一份）
    for (const f of list) f.url = '/api/polish/font?name=' + encodeURIComponent(f.id)
    fonts.value = list
    root.value = (r && r.root) || ''
    error.value = ''
    registerAll(list)
  } catch (e) {
    error.value = '字体库读取失败：' + (e && e.message ? e.message : e)
  } finally {
    loading.value = false
  }
}

async function act(fn) {
  busy.value = true
  try { await fn() } finally { busy.value = false }
}

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

const doDefault = (f) => act(async () => {
  try { await setDefaultFont(f.id); await load() }
  catch (e) { error.value = '设置失败：' + (e && e.message ? e.message : e) }
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

/** 下载自带字体（`id` 缺省 = 下全部还缺的）。幂等 —— 已装好的服务端会跳过。 */
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
const doDownloadAll = () => doDownload(null)

onMounted(load)
</script>

<style scoped>
.wrap { padding: 18px 22px 40px; }
.hd h2 { margin: 0 0 4px; font-size: 18px; }
.sub { margin: 0 0 14px; color: var(--muted, #7a6f6a); font-size: 12px; }
.err {
  display: flex; align-items: center; gap: 10px; margin-bottom: 12px; padding: 8px 12px;
  border: 1px solid #e8b4b4; background: #fdf2f2; border-radius: 8px; font-size: 12px;
}
.import { margin-bottom: 16px; padding: 12px 14px; background: var(--bg-soft, #faf6f4); border-radius: 10px; }
.import label { display: block; margin-bottom: 6px; font-size: 12px; color: var(--muted, #7a6f6a); }
.import .row { display: flex; gap: 8px; }
.import input {
  flex: 1 1 auto; min-width: 0; padding: 6px 9px; border: 1px solid var(--line, #e4dcd8);
  border-radius: 6px; font-size: 12px;
}
.import input.nm { flex: 0 0 160px; }
.import .picked {
  flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  color: var(--muted, #7a6f6a); font-size: 11px; align-self: center;
}
.hint { margin: 6px 0 0; color: var(--muted, #7a6f6a); font-size: 11px; }
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(230px, 1fr)); gap: 12px; }
.card {
  display: flex; flex-direction: column; gap: 8px; padding: 12px;
  border: 1px solid var(--line, #e4dcd8); border-radius: 10px; background: #fff;
}
.preview { padding: 8px 0; font-size: 24px; text-align: center; background: var(--bg-soft, #faf6f4); border-radius: 8px; }
.meta { min-height: 62px; }
.name { font-size: 13px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.fam {
  margin-top: 2px; font-size: 11px; color: var(--muted, #7a6f6a);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.tags { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 6px; }
.tag { padding: 1px 6px; border-radius: 999px; background: var(--bg-soft, #f0ebe8); font-size: 10px; }
.tag.builtin { background: var(--main-soft, #fde3ec); color: var(--pink-text, #b3476d); }
.tag.def { background: #e6f4ea; color: #2f7d4a; }
.tag.warn { background: #fdf2f2; color: #b04a4a; }
.acts { display: flex; gap: 6px; }
.acts .mini-btn { flex: 1 1 auto; }
.mini-btn:disabled { opacity: 0.45; cursor: not-allowed; }
/* 未下载的自带字体：整张卡压暗，一眼看出"这支还不能用" */
.card.off { opacity: 0.72; }
.dl-bar {
  display: flex; align-items: center; justify-content: space-between; gap: 12px;
  margin-bottom: 14px; padding: 10px 14px; border-radius: 10px;
  background: var(--main-soft, #fde3ec); font-size: 12px;
}
.dl-bar strong { margin-right: 6px; }
.dl-bar .mini-btn { flex: 0 0 auto; }
</style>
