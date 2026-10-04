<script setup>
// 贴纸选择器（时间轴属性面板里用）：
//   keyword（贴纸包 · tag）作为**一级筛选**，选中后二级列表自动装载该 keyword 下的贴纸缩略图。
// 数据来自 `GET /api/polish/sticker-library`（只读）；选中即写回
// `keyword`（包名）/ `sticker_tag`（tag）/ `name`（文件名，不含扩展名）/ `source`（包名）。
// `name` 必须是**去扩展名的文件名** —— 后处理与出片都是按 stem 匹配包内文件的。
import { computed, ref } from 'vue'
import Icon from '../Icon.vue'
import { stickerFileUrl } from '../../api/polish'

const props = defineProps({
  lib: { type: Array, default: () => [] }, // [{name, built, tags:[{tag,files}], files}]
  pid: { type: [String, Number], default: '' },
  dir: { type: String, default: '' },
  pack: { type: String, default: '' },     // ev.keyword
  tag: { type: String, default: '' },      // ev.sticker_tag
  name: { type: String, default: '' },     // ev.name（文件名 stem）
  source: { type: String, default: '' },   // ev.source
})
const emit = defineEmits(['pick'])

const filter = ref('')
const imgFailed = ref({})

/** 一级列表：一个 (包, tag) 一行；未构建的包用一行「未构建」承载它的全部文件。 */
const keywords = computed(() => {
  const kw = String(filter.value || '').trim().toLowerCase()
  const out = []
  for (const p of props.lib || []) {
    const tags = p.tags && p.tags.length
      ? p.tags.map((t) => ({ pack: p.name, tag: t.tag, files: t.files || [] }))
      : [{ pack: p.name, tag: '', files: p.files || [] }]
    for (const t of tags) {
      if (kw && !(`${t.pack} ${t.tag}`.toLowerCase().includes(kw))) continue
      out.push(t)
    }
  }
  return out
})

/** 当前选中的一级条目（按 ev.keyword + ev.sticker_tag 反查）。 */
const active = computed(() => {
  const want = props.pack || ''
  if (!want) return null
  return keywords.value.find((k) => k.pack === want && (k.tag || '') === (props.tag || ''))
    || { pack: want, tag: props.tag || '', files: [] }
})

/** 二级列表：选中的 keyword 下**已构建**的文件名（去掉扩展名 = name）。 */
const files = computed(() => {
  const a = active.value
  if (!a) return []
  return (a.files || []).map((f) => ({ file: f, stem: String(f).replace(/\.[^.]+$/, '') }))
})

const isEmojiSource = computed(() => !props.pack || props.pack === 'emoji')

function pickKeyword(k) {
  // 换 keyword：清掉旧的 name/source，让后处理重新解析（或等用户点具体贴纸）
  emit('pick', { keyword: k.pack, sticker_tag: k.tag, name: null, source: null })
}
function pickFile(stem) {
  const a = active.value
  if (!a) return
  emit('pick', { keyword: a.pack, sticker_tag: a.tag, name: stem, source: a.pack })
}
function pickAuto() {
  const a = active.value
  emit('pick', { keyword: a ? a.pack : props.pack, sticker_tag: a ? a.tag : props.tag, name: null, source: null })
}
function markFailed(stem) { imgFailed.value = { ...imgFailed.value, [stem]: true } }
function srcFor(file) { return stickerFileUrl(props.pid, props.dir, props.pack, file) }
</script>

<template>
  <div class="sp">
    <label>keyword（包 · tag）</label>
    <input v-model="filter" class="sp-filter" placeholder="筛选关键词…" @input.stop @change.stop>
    <div class="sp-list">
      <div v-for="k in keywords" :key="k.pack + '/' + k.tag" class="sp-kw"
           :class="{ on: active && k.pack === active.pack && (k.tag || '') === (active.tag || '') }"
           :title="k.pack + (k.tag ? ' · ' + k.tag : '（未构建：没有 tag）')"
           @click="pickKeyword(k)">
        <b>{{ k.pack }}</b><span v-if="k.tag"> · {{ k.tag }}</span><i>({{ k.files.length }})</i>
      </div>
      <div v-if="!keywords.length" class="muted small pad">贴纸库为空（程序目录 stickers/ 下放包）</div>
    </div>

    <template v-if="active">
      <div class="row spread sp-head">
        <span class="mono">{{ active.pack }}<template v-if="active.tag"> · {{ active.tag }}</template></span>
        <button class="mini-btn" title="不指定具体文件：由后处理按 keyword 自动挑一个" @click="pickAuto">自动</button>
      </div>
      <div class="sp-grid">
        <button v-for="f in files" :key="f.file" class="sp-cell" :class="{ on: f.stem === name }"
                :title="f.stem" @click="pickFile(f.stem)">
          <img v-if="!imgFailed[f.stem]" :src="srcFor(f.file)" alt="" loading="lazy" @error="markFailed(f.stem)">
          <Icon v-else name="image" :size="16" />
        </button>
        <div v-if="!files.length" class="muted small pad">
          这个 keyword 下没有清单，用「自动」
        </div>
      </div>
    </template>

    <template v-else-if="isEmojiSource">
      <label>emoji / 自定义 name</label>
      <input :value="name" placeholder="emoji 来源填码点，如 1f62d" @input.stop="emit('pick', { name: $event.target.value, source: null })">
      <div class="hint">emoji 来源没有文件清单：name 是 Unicode 码点的十六进制。</div>
    </template>
    <div v-else class="muted small pad">点上面一个 keyword，下面会列出它的贴纸。</div>
  </div>
</template>

<style scoped>
.sp { margin-top: 6px; }
.sp-filter { margin-top: 2px; }
.sp-list { max-height: 132px; overflow: auto; border: 1px solid var(--line); border-radius: 8px; margin-top: 4px; }
.sp-kw { padding: 3px 8px; font-size: 11.5px; cursor: pointer; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.sp-kw i { color: var(--ink-faint); font-style: normal; margin-left: 4px; }
.sp-kw:hover { background: var(--bg-soft); }
.sp-kw.on { background: var(--main-soft, rgba(229,140,150,.15)); color: var(--main-deep); font-weight: 700; }
.sp-head { font-size: 11.5px; margin-top: 6px; align-items: center; gap: 6px; }
.sp-grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 4px; max-height: 210px; overflow: auto; margin-top: 4px; }
.sp-cell { padding: 2px; border: 1px solid var(--line); border-radius: 6px; background: var(--card); cursor: pointer;
  aspect-ratio: 1; display: flex; align-items: center; justify-content: center; overflow: hidden; }
.sp-cell img { max-width: 100%; max-height: 100%; object-fit: contain; }
.sp-cell:hover { border-color: var(--main); }
.sp-cell.on { outline: 2px solid var(--main); }
.pad { padding: 6px 8px; }
</style>
