<script setup>
/**
 * 贴纸选择器（属性面板里的那一块）—— **行递进三段式 + 整块固定高度**（2026-10-15 定稿）。
 *
 * # 交互（用户口径：「以行形式递进」「固定高度」）
 *
 * 一条竖列，答一级就往路径区叠一行：
 *   [贴纸包] ysyk ✎  →  [分类] 害怕 ✎  →  [贴纸] 正在选… ●
 *
 * * **路径区恒 3 行（78px）**：已选值（带 ✎，点一下回那一级重选）/ 正在选…（实心，当前这一级）/
 *   未选（虚线，还没轮到）。三行都占位 ⇒ 第 1/2/3 页、空库、读失败**高度完全一样**（整块 330px）。
 *   ⚠️ 不占位的话三页会各差 24/27px ——「切一级面板跳一下」正是要修的东西之一。
 * * **正文区恒 196px**（超出滚动）：列表 8 行 / 网格 2 行多。空态在这块里**居中**，别让它塌成矮盒子。
 * * **点 ✎ 行只换页、字段一个都不动**；**选到新值才清更深的选择**，重选同一个值保留
 *   （老实现是点包就无条件清 name/source ⇒ 点回去看一眼，具体图就没了）。
 * * 「全部（不限）」= 不限分类、sticker_tag 清空 —— 它与「没选 tag」落盘是同一个状态，
 *   所以行上**照实写「全部（不限）」**，不假装选了某个分类。
 * * emoji 是第 1 页的一行（老实现里它基本选不到：表情墙只在事件本来就没设来源时才露出）。
 *
 * # 数据
 *
 * 全部来自 GET /api/polish/sticker-library（packs[] 的 name / built / description /
 * tags[{tag,files}] / files，files = 包内全部图）与 GET /api/polish/emoji-catalog。
 * **本组件不做任何写盘**：选中即 emit('pick', patch)，由父组件写事件（TimelineEditor::onStickerPick）。
 *
 * patch 的键语义（undefined = 不动、null = 清）：
 *   keyword（贴纸态元素落 text）/ sticker_tag / name（去扩展名；emoji 是码点）/ source。
 *
 * ⚠️ 两个有意为之：
 *   ① emoji 的「分类」只用于**分组浏览**，落盘的 sticker_tag 后端对 emoji 一律忽略
 *      （load_package_meta("emoji") 不存在 ⇒ Step2b 直接跳过）—— 写它只为让这一行照实显示；
 *   ② 未构建包（没有 <包>.toml）没有分类可列 ⇒ 第 2 页只剩「全部（不限）」+ 一句提示，
 *      「去构建」的入口留给素材库（本组件不导航）。
 */
import { computed, onMounted, ref, watch } from 'vue'
import Icon from '../Icon.vue'
import { stickerFileUrl, fetchEmojiCatalog } from '../../api/polish'
// ⚠️ safeEmoji 从 engine.js 来 —— 预览画的就是它算出来的字符，
//    本面板自己再写一份解码就会「面板显示 A、画面画 B」（这正是用户报过的那个缺陷的形状）。
import { safeEmoji, emojiGifUrl } from '../../polish/engine'

const props = defineProps({
  lib: { type: Array, default: () => [] }, // [{name, built, description, tags:[{tag,files}], files}]
  libError: { type: String, default: '' }, // 读失败原文（非空 ⇒ 错误条，**不许**退化成「库是空的」）
  pid: { type: [String, Number], default: '' },
  dir: { type: String, default: '' },
  pack: { type: String, default: '' },     // 事件里的来源/包名（贴纸态元素是 text）
  tag: { type: String, default: '' },      // sticker_tag
  name: { type: String, default: '' },     // 去扩展名的文件名；emoji 是码点
  source: { type: String, default: '' },   // source（emoji / 包名）—— 只读回显用
})
const emit = defineEmits(['pick', 'retry-lib'])

const EMOJI = 'emoji'

/** 正在选哪一级：pack / tag / sticker。**本组件唯一的本地「位置」状态**，其余全从 props 派生。 */
const edit = ref('pack')
/** UI-only：「全部（不限）」也算在分类那一级做过选择（落盘仍是 sticker_tag 缺省）。 */
const tagPicked = ref(false)
const qPack = ref('')
const qTag = ref('')
const qSticker = ref('')
const emojiGroups = ref([])
const emojiError = ref('')
const emojiPlain = ref('')
const imgFailed = ref({})   // 「包/文件名」→ true（缩略图 404 画占位图标，title 仍是文件名）
// 码点 → true：内建表情的 **CDN 动图**拿不到（离线 / 被墙 / 该码点 CDN 没有）
// ⇒ 退回静态字符。字符是**兜底**，不是主路径 —— 主路径必须是出片会画的那个 GIF。
const emojiBroken = ref({})
function markEmojiBroken(cp) { emojiBroken.value = { ...emojiBroken.value, [cp]: true } }
/** 刚发出去的那份 props 快照 —— 父组件写回来时**不算「外部换了事件」**，别把 edit 重置掉。 */
let lastEmit = null

const keyOf = (o) => [o.pack || '', o.tag || '', o.name || ''].join('\u0001')
const selfKey = computed(() => keyOf(props))

/* ---------------- 库数据 ---------------- */
const packs = computed(() => (props.lib || []).map((p) => {
  const tags = Array.isArray(p.tags) ? p.tags : []
  const files = Array.isArray(p.files) ? p.files : []
  const seen = new Set()
  let tagged = 0
  for (const t of tags) for (const f of (t.files || [])) if (!seen.has(f)) { seen.add(f); tagged++ }
  return { name: p.name, built: !!p.built, description: p.description || '', tags, files, tagged }
}))
const isEmoji = computed(() => props.pack === EMOJI)
const activePack = computed(() => packs.value.find((p) => p.name === props.pack) || null)

function packMeta(p) {
  if (!p) return ''
  if (!p.built) return p.files.length + ' 张 · 未构建'
  return p.tags.length + ' 分类 · ' + p.files.length + ' 张'
}

onMounted(async () => {
  try {
    const r = await fetchEmojiCatalog('读取表情目录')
    emojiGroups.value = (r && r.groups) || []
  } catch (e) {
    // ⚠️ 目录读不到**不许**伪装成「没有表情」：报出来，手填那条路照样能用。
    emojiError.value = e.message || String(e)
  }
})

/* ---------------- 路径区（恒 3 行） ---------------- */
const rows = computed(() => {
  const raw = [
    { lv: 'pack', chip: '贴纸包', val: props.pack, meta: props.pack ? (isEmoji.value ? '表情 · 内建' : packMeta(activePack.value)) : '' },
    { lv: 'tag', chip: '分类', val: tagPicked.value ? (props.tag || '全部（不限）') : '', meta: '' },
    { lv: 'sticker', chip: '贴纸', val: props.name ? (isEmoji.value ? (safeEmoji(props.name) + ' ' + props.name) : props.name) : '', meta: '' },
  ]
  return raw.map((r) => {
    const cur = edit.value === r.lv
    return {
      ...r, cur, todo: !r.val && !cur, ph: !r.val,
      text: r.val || (cur ? '正在选…' : '未选'),
      title: cur ? '正在选这一级' : (r.todo ? '还没轮到（先选上一级）' : '点一下回到这一级重选（字段不动）'),
    }
  })
})

/* ---------------- 各级选项 ---------------- */
const shownPacks = computed(() => {
  const kw = qPack.value.trim().toLowerCase()
  return packs.value.filter((p) => !kw || p.name.toLowerCase().includes(kw))
})
/** 分类那一级的行。⚠️ **两套来源**：普通包读 <包>.toml 的 tags；emoji 读内建表情目录的分组 ——
 *  少了 emoji 这一支，表情来源的「分类」页就只剩「全部（不限）」（headless 几何判据当场抓到过）。 */
const tagRows = computed(() => {
  if (isEmoji.value) {
    return emojiGroups.value.map((g) => ({ tag: g.tag, count: (g.codepoints || []).length, unit: '个' }))
  }
  const p = activePack.value
  if (!p) return []
  const kw = qTag.value.trim().toLowerCase()
  return p.tags
    .filter((t) => !kw || String(t.tag).toLowerCase().includes(kw))
    .map((t) => ({ tag: t.tag, count: (t.files || []).length, unit: '张' }))
})
/** 「全部（不限）」那一行的计数。 */
const allTagMeta = computed(() => {
  if (isEmoji.value) {
    let n = 0
    for (const g of emojiGroups.value) n += (g.codepoints || []).length
    return n + ' 个'
  }
  return ((activePack.value && activePack.value.files.length) || 0) + ' 张'
})
/** 当前分类下的文件（「全部（不限）」= 包内全部图）。 */
const currentFiles = computed(() => {
  if (!props.pack) return []
  if (isEmoji.value) {
    const out = []
    for (const g of emojiGroups.value) {
      if (props.tag && g.tag !== props.tag) continue
      for (const cp of (g.codepoints || [])) out.push({ cp, stem: cp, file: cp, emoji: true })
    }
    return out
  }
  const p = activePack.value
  if (!p) return []
  const t = props.tag ? p.tags.find((x) => x.tag === props.tag) : null
  const src = t ? (t.files || []) : p.files
  return src.map((f) => ({ file: f, stem: String(f).replace(/\.[^.]+$/, ''), emoji: false }))
})
const shownFiles = computed(() => {
  const kw = qSticker.value.trim().toLowerCase()
  if (!kw) return currentFiles.value
  return currentFiles.value.filter((x) => x.stem.toLowerCase().includes(kw) || (x.emoji && x.cp === kw))
})
const secCount = computed(() => {
  if (edit.value === 'pack') return packs.value.length + ' 个'
  if (edit.value === 'tag') return props.pack ? props.pack : '先选一个包'
  return shownFiles.value.length + ' ' + (isEmoji.value ? '个' : '张')
})
const srcFor = (file) => stickerFileUrl(props.pid, props.dir, props.pack, file)
const failKey = (stem) => props.pack + '/' + stem
const isBroken = (stem) => !!imgFailed.value[failKey(stem)]
function markFailed(stem) { imgFailed.value = { ...imgFailed.value, [failKey(stem)]: true } }

/* ---------------- 写回（emit） ---------------- */
/** 按父组件 onStickerPick 的规则算「这份 patch 写完之后 props 该长什么样」，用于认自己的回声。 */
function afterPatch(p) {
  return {
    pack: p.keyword !== undefined ? (p.keyword || '') : props.pack,
    tag: p.sticker_tag !== undefined ? (p.sticker_tag || '') : props.tag,
    name: p.name !== undefined ? (p.name || '') : props.name,
  }
}
function pick(patch) {
  lastEmit = keyOf(afterPatch(patch))
  emit('pick', patch)
}
/** 外部换了事件（不是我们刚写的那份）⇒ 停到**第一个还没答的**那一级。 */
function resetLevel() {
  qPack.value = ''
  qTag.value = ''
  qSticker.value = ''
  tagPicked.value = !!props.tag
  edit.value = !props.pack ? 'pack' : !props.tag ? 'tag' : 'sticker'
}
watch(selfKey, (now) => {
  if (lastEmit && now === lastEmit) { lastEmit = null; return } // 自己刚写的那份
  lastEmit = null
  resetLevel()
}, { immediate: true })

function go(lv) { edit.value = lv }
function pickPack(name) {
  if (name === props.pack) { edit.value = 'tag'; return } // 重选同一个包：更深的选择保留
  pick({ keyword: name, sticker_tag: null, name: null, source: null })
  tagPicked.value = false
  edit.value = 'tag'
}
function pickTag(t) {
  const next = t || ''
  if (next !== props.tag) pick({ sticker_tag: next || null, name: null, source: null })
  tagPicked.value = true
  edit.value = 'sticker'
}
function pickFile(stem) {
  pick({ keyword: props.pack, sticker_tag: props.tag || null, name: stem, source: props.pack })
}
function pickCodepoint(cp) {
  // emoji：keyword 固定 emoji（LLM 与 process 同一口径），source=emoji，name=码点。
  pick({ keyword: EMOJI, sticker_tag: props.tag || null, name: cp, source: EMOJI })
}
function autoPick() {
  pick({ keyword: props.pack, sticker_tag: props.tag || null, name: null, source: null })
}
/** 手填/粘贴：宽进 —— 判合法只有一份（engine.js::safeEmoji / isEmojiName），这里不判。 */
function applyPlain() {
  const v = String(emojiPlain.value || '').trim()
  if (!v) return
  pick({ keyword: EMOJI, sticker_tag: props.tag || null, name: v, source: EMOJI })
  emojiPlain.value = ''
}
</script>

<template>
  <div class="sp">
    <!-- 路径区：恒 3 行（78px）—— 已选 ✎ / 正在选… ● / 未选 -->
    <div class="stack">
      <div v-for="r in rows" :key="r.lv" class="srow"
           :class="[r.cur ? 'cur' : r.todo ? 'todo' : 'can', r.ph ? 'ph' : '']"
           :title="r.title" @click="r.cur || r.todo ? null : go(r.lv)">
        <span class="chip" :class="r.cur ? 'p' : 'n'">{{ r.chip }}</span>
        <span class="val">{{ r.text }}</span>
        <span v-if="r.meta" class="meta">{{ r.meta }}</span>
        <span v-if="r.cur" class="dot" />
        <span v-else-if="!r.todo" class="pen">✎</span>
      </div>
    </div>

    <div class="opts">
      <div class="sec-h">
        <span v-if="edit === 'pack'">选择贴纸包</span>
        <span v-else-if="edit === 'tag'">选择分类</span>
        <span v-else>选择贴纸</span>
        <span class="cnt">{{ secCount }}</span>
        <span v-if="edit === 'sticker' && !libError" class="sp-r">
          <!-- ⚠️ 「不选 = 自动挑一张」这句**删了**（2026-10-15）：面板内容宽只有 210px，
               它 + 「自动」按钮会把标题行挤成三行并压到下面的内容上（headless 截图里一眼看到）。
               同一句话按钮的 title 里已经有，没必要再占一行。 -->
          <span v-if="name" class="chip p">已选</span>
          <button class="mini-btn" :class="{ on: !name }"
                  title="不指定具体图：由后处理按包/分类挑一张" @click="autoPick">自动</button>
        </span>
      </div>

      <!-- 读失败：照实报 + 重试（**不许**退化成「库是空的」） -->
      <div v-if="libError" class="body center">
        <div>
          <div class="err"><Icon name="alert" :size="12" /> 贴纸库读取失败：{{ libError }}</div>
          <div style="margin-top:6px"><button class="mini-btn" @click="emit('retry-lib')">重试</button></div>
        </div>
      </div>
      <!-- 空库：真的一张贴纸都没有 -->
      <div v-else-if="edit === 'pack' && !packs.length" class="body center">
        <div class="muted small">贴纸库为空：往程序目录 <span class="mono">resources/stickers/</span> 放包</div>
      </div>

      <template v-else>
        <input v-if="edit === 'pack'" v-model="qPack" class="q" placeholder="筛选包名…"
               title="只筛这一级" @input.stop @change.stop>
        <input v-else-if="edit === 'tag'" v-model="qTag" class="q" placeholder="筛选分类…"
               title="只筛这一级" @input.stop @change.stop>
        <!-- ⚠️ 表情那一页**不挂顶部筛选**：它下面那条「粘一个表情，或填码点」才是真入口
             （两条 placeholder 一样会让用户以为要填两次）。 -->
        <input v-else-if="!isEmoji" v-model="qSticker" class="q" placeholder="筛选文件名…"
               title="只筛这一级" @input.stop @change.stop>

        <div class="body">
          <!-- ① 包 -->
          <template v-if="edit === 'pack'">
            <div class="list">
              <div v-for="p in shownPacks" :key="p.name" class="row-item" :class="{ on: pack === p.name }"
                   :title="p.name + (p.description ? ' · ' + p.description : '') + '（' + packMeta(p) + '）'"
                   @click="pickPack(p.name)">
                <span class="nm mono">{{ p.name }}</span>
                <span v-if="!p.built" class="chip a" title="没有 &lt;包&gt;.toml：LLM 不认识这个包，只能手动选图">未构建</span>
                <span class="meta">{{ packMeta(p) }}</span><span class="go">›</span>
              </div>
              <div class="row-item" :class="{ on: isEmoji }" title="内建表情目录（编译期内嵌）" @click="pickPack('emoji')">
                <span class="nm mono">emoji</span><span class="meta">表情 · 内建</span><span class="go">›</span>
              </div>
              <div v-if="!shownPacks.length" class="pad muted small">没有匹配的包</div>
            </div>
          </template>

          <!-- ② 分类 -->
          <template v-else-if="edit === 'tag'">
            <div class="list">
              <div class="row-item" :class="{ on: tagPicked && !tag }"
                   title="不限分类：包内全部图（含没有分类的）" @click="pickTag('')">
                <span class="nm">全部（不限）</span>
                <span class="meta">{{ allTagMeta }}</span>
                <span class="go">›</span>
              </div>
              <div v-for="t in tagRows" :key="t.tag" class="row-item" :class="{ on: tag === t.tag }"
                   :title="t.tag + ' · ' + t.count + t.unit" @click="pickTag(t.tag)">
                <span class="nm">{{ t.tag }}</span>
                <span class="meta">{{ t.count }} {{ t.unit }}</span><span class="go">›</span>
              </div>
            </div>
            <div v-if="!isEmoji && activePack && !activePack.tags.length" class="warn">
              未构建：LLM 不认识这个包，只能手动选图（在素材库里构建后这里会出分类）
            </div>
            <div v-else-if="!tagRows.length" class="hint mt-4">没有匹配的分类</div>
            <div v-if="isEmoji && emojiError" class="hint mt-4">表情目录读取失败：{{ emojiError }}</div>
            <div v-if="!isEmoji && activePack && activePack.files.length > activePack.tagged" class="hint mt-4">
              其中 {{ activePack.files.length - activePack.tagged }} 张没有分类（只在「全部（不限）」里）
            </div>
          </template>

          <!-- ③ 贴纸 -->
          <template v-else>
            <div v-if="isEmoji" class="em-tags">
              <template v-for="g in emojiGroups" :key="g.tag">
                <template v-if="!tag || g.tag === tag">
                  <div class="em-tag">{{ g.tag }}</div>
                  <div class="em-grid">
                    <button v-for="cp in (g.codepoints || []).filter((c) => !qSticker.trim() || c === qSticker.trim().toLowerCase())"
                            :key="cp" class="em-cell" :class="{ on: name === cp }"
                            :title="cp" @click="pickCodepoint(cp)">
                      <!-- ⚠️ **面板画的必须是出片会画的那一个**（2026-10-17）。以前这里直接写
                           `{{ safeEmoji(cp) }}` —— 一个**静态字符**，而渲染侧（出片模板 /
                           底座）对同一码点拿的是 **Noto 动态 GIF**（`index.html` 的 CDN 回退）。
                           用户口径：「polish编辑器内有，但是是静态的」。
                           CDN 拿不到才退回字符（`loading="lazy"` 是必须的：不选分类时这里
                           会同时渲染 881 个格子）。 -->
                      <img v-if="!emojiBroken[cp]" :src="emojiGifUrl(cp)" alt="" loading="lazy"
                           @error="markEmojiBroken(cp)">
                      <span v-else>{{ safeEmoji(cp) }}</span>
                    </button>
                  </div>
                </template>
              </template>
              <div v-if="emojiError" class="hint mt-4">表情目录读取失败：{{ emojiError }}</div>
              <div v-else-if="!emojiGroups.length" class="pad muted small">表情目录为空。</div>
              <div class="row em-in">
                <input v-model="emojiPlain" class="em-plain" placeholder="粘一个表情，或填码点如 1f62d"
                       @input.stop @change.stop @keydown.enter.stop="applyPlain">
                <button class="mini-btn" :disabled="!emojiPlain.trim()" @click="applyPlain">用</button>
              </div>
            </div>
            <div v-else class="grid3">
              <button v-for="f in shownFiles" :key="f.file" class="cell" :class="{ on: name === f.stem }"
                      :title="f.file" @click="pickFile(f.stem)">
                <img v-if="!isBroken(f.stem)" :src="srcFor(f.file)" alt="" loading="lazy" @error="markFailed(f.stem)">
                <Icon v-else name="image" :size="16" />
              </button>
              <div v-if="!shownFiles.length" class="pad muted small" style="grid-column:1/-1">
                这个分类下没有图
              </div>
            </div>
          </template>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
/* ⚠️ 高度全写死（整块 330 = 路径 78 + 标题/筛选 50 + 正文 192 + 间隔 10）——
   切页面 / 切状态都不许跳；判据 scripts/check-sticker-picker-flow.mjs。 */
.sp { margin-top: 6px; height: 330px; display: flex; flex-direction: column; min-width: 0; }
.stack { flex: none; height: 78px; display: flex; flex-direction: column; gap: 3px; }
.srow { height: 24px; display: flex; align-items: center; gap: 6px; padding: 0 8px; border-radius: 8px;
  font-size: 11.5px; background: var(--bg-soft); border: 1px solid transparent; min-width: 0; }
.srow.can { cursor: pointer; }
.srow.can:hover { border-color: var(--main); }
.srow.can:hover .pen { color: var(--pink-text); }
.srow .val { font-weight: 700; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.srow .meta { margin-left: auto; color: var(--ink-faint); font-size: 10.5px; flex: none; font-weight: 400; }
.srow .pen { color: var(--ink-faint); flex: none; font-size: 11px; }
.srow.ph { border-style: dashed; border-color: var(--line); }
.srow.ph .val { font-weight: 400; color: var(--ink-faint); }
.srow.todo { background: transparent; border-style: dashed; border-color: var(--line-soft); }
.srow.todo .chip { color: var(--ink-faint); }
.srow.cur { background: var(--main-soft); box-shadow: inset 3px 0 0 var(--main); }
.srow.cur .val { color: var(--pink-text); }
.dot { width: 6px; height: 6px; border-radius: 50%; background: var(--main); flex: none; margin-left: auto; }
.chip { display: inline-flex; align-items: center; gap: 4px; padding: 0 9px; height: 17px;
  border-radius: var(--r-full); font-size: 10.5px; background: var(--bg-soft); color: var(--ink-dim);
  flex: none; white-space: nowrap; }
.chip.p { background: var(--main-soft); color: var(--pink-text); }
.chip.a { background: var(--amber-soft); color: var(--amber-deep); }
.chip.n { background: var(--card); border: 1px solid var(--line); color: var(--ink-faint); }
.opts { margin-top: 6px; display: flex; flex-direction: column; min-height: 0; }
/* ⚠️ 标题行**不许换行**（210px 内容宽很挤）：换行会让盒子装不下、压到正文上。
   ⚠️ 高度 **22px**：它右边那颗「自动」是标准 .mini-btn（22px 高）—— 写 18px 时按钮被 overflow 裁掉
   上下各 2px（headless 判据按"内容高 ≤ 盒高"当场抓到）。计数位可省略号，右侧那组不许被压缩。 */
.sec-h { flex: none; height: 22px; display: flex; align-items: center; gap: 6px; font-size: 11.5px;
  font-weight: 600; white-space: nowrap; overflow: hidden; }
.sec-h > span:first-child { flex: none; }
.sec-h .cnt { color: var(--ink-faint); font-weight: 400; font-size: 10.5px; min-width: 0;
  overflow: hidden; text-overflow: ellipsis; }
.sec-h .sp-r { margin-left: auto; display: flex; gap: 6px; align-items: center; flex: none; }
.q { flex: none; height: 24px; width: 100%; margin-top: 4px; font-size: 11.5px; padding: 0 8px;
  border: 1px solid var(--line); border-radius: 8px; background: var(--card); color: var(--ink); }
.q::placeholder { color: var(--ink-faint); }
.body { flex: none; height: 192px; margin-top: 4px; overflow: auto; min-height: 0; }
.body.center { display: flex; align-items: center; justify-content: center; text-align: center; }
.list { border: 1px solid var(--line); border-radius: 8px; background: var(--card); overflow: hidden; }
.row-item { height: 24px; display: flex; align-items: center; gap: 6px; padding: 0 8px; font-size: 11.5px;
  cursor: pointer; white-space: nowrap; overflow: hidden; }
.row-item + .row-item { border-top: 1px solid var(--line-soft); }
.row-item:hover { background: var(--bg-soft); }
.row-item.on { background: var(--main-soft); color: var(--pink-text); font-weight: 700; }
.row-item .nm { overflow: hidden; text-overflow: ellipsis; }
.row-item .meta { margin-left: auto; color: var(--ink-faint); font-weight: 400; font-size: 10.5px; flex: none; }
.row-item .go { color: var(--ink-faint); flex: none; }
.grid3 { display: grid; grid-template-columns: repeat(3, 1fr); gap: 4px; align-content: start; }
.cell { padding: 2px; border: 1px solid var(--line); border-radius: 6px; background: var(--card);
  cursor: pointer; aspect-ratio: 1; display: flex; align-items: center; justify-content: center;
  overflow: hidden; color: var(--ink-faint); }
.cell:hover { border-color: var(--main); }
.cell.on { outline: 2px solid var(--main); }
.cell img { max-width: 100%; max-height: 100%; object-fit: contain; }
.em-tag { font-size: 10.5px; color: var(--ink-faint); letter-spacing: .04em; margin: 4px 0 2px; }
.em-grid { display: grid; grid-template-columns: repeat(8, 1fr); gap: 2px; }
.em-cell { padding: 0; border: 1px solid transparent; border-radius: 5px; background: var(--card);
  font-size: 16px; line-height: 1.5; cursor: pointer; aspect-ratio: 1; display: flex;
  align-items: center; justify-content: center; }
.em-cell:hover { background: var(--bg-soft); border-color: var(--main); }
.em-cell img { max-width: 100%; max-height: 100%; object-fit: contain; }
.em-cell.on { outline: 2px solid var(--main); }
.em-in { margin-top: 6px; gap: 4px; flex-wrap: nowrap; }
.em-plain { flex: 1 1 auto; min-width: 0; font-size: 12px; height: 24px; padding: 0 8px;
  border: 1px solid var(--line); border-radius: 8px; background: var(--card); color: var(--ink); }
.pad { padding: 6px 8px; }
.warn { margin-top: 6px; font-size: 11px; color: var(--amber-deep); background: var(--amber-soft);
  border-radius: 8px; padding: 4px 8px; }
.err { font-size: 11px; color: var(--red-deep); background: var(--red-soft); border-radius: 8px;
  padding: 4px 8px; display: inline-flex; gap: 6px; align-items: center; }
.mini-btn { display: inline-flex; align-items: center; gap: 4px; height: 22px; padding: 0 10px;
  font-size: 11.5px; border-radius: var(--r-full); border: 1px solid var(--line); background: var(--card);
  color: var(--ink-dim); cursor: pointer; white-space: nowrap; flex: none; }
.mini-btn:hover { border-color: var(--main); color: var(--pink-text); }
.mini-btn.on { background: var(--green-soft); color: var(--green-deep); border-color: var(--green); font-weight: 600; }
.mini-btn:disabled { opacity: .45; cursor: not-allowed; }
.mt-4 { margin-top: 4px; }
</style>
