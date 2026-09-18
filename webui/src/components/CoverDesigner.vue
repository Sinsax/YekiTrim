<template>
  <div class="card" id="cover-design">
    <div class="row spread">
      <h2 class="card-h2 m-0"><Icon name="image" :size="16" /> 封面设计器</h2>
      <button class="btn btn-primary" :disabled="generating || !!clipBusyJob"
              :title="clipBusyJob ? '该片段正在' + kindText(clipBusyJob.kind) : ''"
              @click="generateCover">{{ generating ? '生成中…' : '生成封面' }}</button>
    </div>
    <!-- 配置没读回来 → `configLoaded` 这道写入门闸继续关着，这个面板之后**不会**再保存任何改动。
         必须摆到最显眼处并给重试：否则就是"改了十分钟、关掉再打开全没了"，还没有一句解释。 -->
    <div v-if="configLoadError" class="error-banner">
      <Icon name="alert" :size="15" />
      <span>封面配置读取失败，<b>改动不会被保存</b>：{{ configLoadError }}</span>
      <button class="mini-btn" style="margin-left:auto" @click="reloadCoverConfig">重试</button>
    </div>
    <div class="designer-grid">
      <div class="stage-wrap" ref="stageWrapRef">
        <div ref="stageRef" class="stage">
          <!-- 背景层（在帧**之下**）：纯色，或当前帧模糊（用出片同一张 `_cover_frame.jpg`）。
               帧缩小/平移时才会露出来 —— 背景是加的一层，不替换帧（用户要求）。 -->
          <div class="stage-bg" :style="{ background: bgColor }">
            <img v-if="(bgMode === 'blur' || bgMode === 'image') && bgImageOk" class="stage-bg-img"
                 :class="{ 'bg-blur': bgMode === 'blur' }" :src="bgImageSrc"
                 :style="bgMode === 'blur' ? bgBlurStyle : null" alt="" @error="onBgImageError">
          </div>
          <!-- 帧层（当前帧清晰图）：拖动平移 / 滚轮缩放，出片里就是 #frame img 的 transform -->
          <div class="stage-media">
            <video ref="videoRef" playsinline preload="metadata" class="stage-video" :src="videoSrc" :style="mediaStyle" @timeupdate="onVideoTime" @loadedmetadata="onMeta" @play="playing = true" @pause="playing = false"></video>
          </div>
          <div class="fx-layer">
            <div v-for="(fx, i) in fxList" :key="i" class="fx-item" :class="'fx-' + fx.type" :style="fxStyle(fx)">{{ fx.type === 'emoji' ? (fx.text || '🙂') : '' }}</div>
          </div>
          <div class="gradient-layer" :style="gradientStyle"></div>
          <div class="vignette-layer" :style="{ opacity: vignette }"></div>
          <div class="title-wrap" :style="{ left: textX + '%', top: textY + '%', width: textWidth + '%' }">
            <div ref="titleTextRef" class="title-text" :style="titleStyle" v-html="titleHtml"></div>
          </div>
          <div class="stage-gesture-layer" title="拖动 = 移动当前帧 · 滚轮 = 缩放当前帧"
               @pointerdown="onStagePointerDown" @wheel.prevent="onStageWheel"></div>
          <!-- 安全区线框（纯辅助线，只画在设计器里，**不在 cover.html 出片模板里**）：
               16:9 框 = 四周等比内缩 5%（1824×1026），4:3 框 = 满高居中（1440×1080）。
               压在手势层之上（z7）才看得出标题有没有越线；pointer-events:none 所以不挡拖动。 -->
          <div v-if="showGuide169 || showGuide43" class="guide-layer">
            <div v-if="showGuide169" class="guide-box guide-169"><span class="guide-tag">16:9 安全区</span></div>
            <div v-if="showGuide43" class="guide-box guide-43"><span class="guide-tag">4:3 裁切区</span></div>
          </div>
        </div>
        <div class="guide-row mt-4">
          <span class="mini-label">安全区</span>
          <button class="mini-btn" :class="{ selected: showGuide169 }" title="16:9 安全区：四周内缩 5%，平台 UI / 圆角吃掉的部分"
                  @click="toggleGuide('169')">16:9</button>
          <button class="mini-btn" :class="{ selected: showGuide43 }" title="4:3 裁切区：满高居中，被裁成 4:3 时还看得见的部分"
                  @click="toggleGuide('43')">4:3</button>
        </div>
        <div class="seek-row mt-4">
          <button class="mini-btn" @click="togglePlay">{{ playing ? '暂停' : '播放' }}</button>
          <input class="flex-1 seek-range" type="range" min="0" :max="duration || 1" step="0.1" :value="time" @input="onSeekInput">
          <span class="muted small mono seek-time">{{ fmtTime(time) }} / {{ fmtTime(duration) }}</span>
        </div>
      </div>
      <div class="controls" ref="controlsRef">
        <details open class="grp">
          <summary>预设</summary>
          <div class="preset-row">
            <button v-for="p in presets" :key="p.key" class="mini-btn" :class="{ selected: preset === p.key }" @click="applyPreset(p.key)">{{ p.label }}</button>
          </div>
        </details>

        <details open class="grp">
          <summary>标题与高亮</summary>
          <!-- 标题文本框：固定 5 行高、不可拖拽缩放（内容超出在框内滚动）；字段样式用设计基准 .fld-input -->
          <textarea v-model="titleLinesText" class="fld-input title-lines" rows="5" placeholder="封面标题，每行一段"></textarea>
          <!-- 首次拿到标题时自动排一次版并放进 4:3 安全区；放不下就在这儿说出来（不静默） -->
          <div v-if="titleFitHint" class="muted small mt-4 fit-hint">{{ titleFitHint }}</div>
          <div class="mini-row mt-4">
            <button class="mini-btn" :disabled="generatingTitle" title="只用 clip.srt + polish.toml 逐个事件生成" @click="generateTitleByLlm">{{ generatingTitle ? '生成中…' : 'LLM 生成标题' }}</button>
            <span v-if="saveError" class="muted small save-err">保存失败：{{ saveError }}</span>
            <span v-else-if="savingConfig" class="muted small">保存中…</span>
          </div>
          <div class="mini-grid mt-4">
            <div class="mini-field"><label>高亮色</label><input v-model="hlColor" type="color"></div>
            <label class="mini-field check"><input v-model="textShadow" type="checkbox"><span>文字阴影 {{ textShadow ? '开' : '关' }}</span></label>
          </div>
          <div class="mini-row mt-4">
            <input v-model="highlightInput" class="kw-input" placeholder="高亮关键词，回车添加" @keyup.enter="addHighlightKeyword">
            <button class="mini-btn" @click="addHighlightKeyword"><Icon name="plus" :size="11" /> 添加</button>
          </div>
          <div v-if="highlightKeywords.length" class="hl-chips mt-4">
            <span v-for="(kw, i) in highlightKeywords" :key="i" class="hl-chip">
              {{ kw }}
              <button class="hl-x" title="删除" @click="removeHighlightKeyword(i)">×</button>
            </span>
          </div>
        </details>

        <details open class="grp">
          <summary>排版</summary>
          <div class="mini-grid">
            <div class="mini-field"><label>对齐</label>
              <select v-model="align">
                <option value="left">左</option>
                <option value="center">中</option>
                <option value="right">右</option>
              </select>
            </div>
            <div class="mini-field"><label>宽度</label><input type="range" min="20" max="98" step="1" v-model.number="textWidth"><span class="rv">{{ textWidth }}%</span></div>
          </div>
          <div class="mini-grid mt-4">
            <div class="mini-field"><label>X</label><input type="range" min="0" max="100" step="1" v-model.number="textX"><span class="rv">{{ textX }}%</span></div>
            <div class="mini-field"><label>Y</label><input type="range" min="0" max="100" step="1" v-model.number="textY"><span class="rv">{{ textY }}%</span></div>
          </div>
          <div class="mini-grid mt-4">
            <div class="mini-field"><label>字号</label><input type="range" min="24" max="240" step="2" v-model.number="fontSize"><span class="rv">{{ fontSize }}px</span></div>
            <div class="mini-field"><label>描边</label><input type="range" min="0" max="60" step="1" v-model.number="stroke"><span class="rv">{{ stroke }}px</span></div>
          </div>
          <!-- 封面字体：字体库（程序目录 resources/fonts/）里的字体。空 = 系统字体栈。
               库是空的就把目录写出来 —— 不然用户没处知道该往哪儿丢字体文件。 -->
          <div class="mini-field mt-4">
            <label>字体</label>
            <select :value="coverFont" :disabled="!configLoaded" @change="onCoverFontChange">
              <option v-for="o in fontOptions" :key="o.value" :value="o.value">{{ o.label }}</option>
            </select>
          </div>
          <div v-if="!fonts.length" class="muted small mt-4">
            字体库为空：把 .ttf / .otf / .ttc 丢进 <span class="mono">{{ fontsRoot || '程序目录 resources/fonts/' }}</span>
          </div>
          <div class="mini-grid mt-4">
            <div class="mini-field"><label>行距</label><input type="range" min="0.8" max="2.6" step="0.05" v-model.number="lineHeight"><span class="rv">{{ Number(lineHeight).toFixed(2) }}</span></div>
            <div class="mini-field"><label>字距</label><input type="range" min="-10" max="60" step="1" v-model.number="letterSpacing"><span class="rv">{{ letterSpacing }}px</span></div>
          </div>
        </details>

        <!-- 关键帧层（封面三层 = 文字 / 关键帧 / 背景，用户要求）：当前帧清晰图，在背景之上、文字之下 -->
        <details open class="grp">
          <summary>关键帧</summary>
          <div class="mini-row">
            <span class="mini-label">时间</span>
            <input v-model.number="time" type="number" step="0.1" class="w-time">
            <button class="mini-btn" @click="takeCurrent">取当前帧</button>
            <button class="mini-btn" @click="exportFrame">导出关键帧</button>
          </div>
          <div class="mini-grid mt-4">
            <div class="mini-field"><label>帧缩放</label><input type="range" min="0.5" max="3" step="0.05" v-model.number="coverZoom"><span class="rv">{{ Math.round(coverZoom * 100) }}%</span></div>
            <div class="mini-field"><button class="mini-btn" style="width:100%;justify-content:center" @click="resetFrame">复位</button></div>
          </div>
          <div class="mini-grid mt-4">
            <div class="mini-field"><label>偏移</label><span class="rv">X {{ Math.round(coverOffsetX) }} · Y {{ Math.round(coverOffsetY) }}</span></div>
            <div class="drag-hint" title="拖动预览 = 移动当前帧 · 滚轮 = 缩放当前帧">拖预览移动 · 滚轮缩放</div>
          </div>
          <div class="muted small mt-4">关键帧在背景之上、文字之下；「导出关键帧」把它固化成出片用的那张图（出片不再删除它）。</div>
        </details>

        <details open class="grp">
          <summary>背景与阴影</summary>
          <!-- 背景在**关键帧之下**：纯色 · 当前帧模糊（关键帧的模糊副本）· 自定义图片（导入的图） -->
          <div class="mini-row">
            <span class="mini-label">背景</span>
            <button v-for="m in BG_MODES" :key="m.key" class="mini-btn seg-btn"
                    :class="{ selected: bgMode === m.key }" :title="m.hint"
                    @click="bgMode = m.key">{{ m.label }}</button>
          </div>
          <div v-if="bgMode === 'color'" class="mini-grid mt-4">
            <div class="mini-field"><label>颜色</label><input v-model="bgColor" type="color">
              <input v-model="bgColorText" class="hex-input" maxlength="7" spellcheck="false" placeholder="#000000"
                     title="直接填 16 进制码（#1a2b3c 或 #abc），回车/失焦生效"
                     @change="commitBgColorText" @keyup.enter="commitBgColorText" @blur="commitBgColorText">
            </div>
          </div>
          <div v-else-if="bgMode === 'blur'" class="mini-grid mt-4">
            <div class="mini-field"><label>模糊倍率</label><input type="range" min="0.2" max="4" step="0.1" v-model.number="blurScale"><span class="rv">×{{ Number(blurScale).toFixed(1) }}</span></div>
          </div>
          <div v-else class="mini-row mt-4">
            <button class="mini-btn" @click="fileInput.click()">导入背景图片</button>
            <span class="muted small">{{ bgImageOk ? '已导入 _cover_bg.jpg' : '还没导入（先点左边选一张）' }}</span>
          </div>
          <input ref="fileInput" type="file" accept="image/*" class="hidden-file" @change="importBg">
          <div v-if="bgMode === 'blur' && bgFrameTried && !bgImageOk" class="muted small mt-4">
            背景截帧没生成出来（出片会自己截，这里先用手动「导出关键帧」补一张）
          </div>
          <div class="muted small mt-4">背景在「关键帧」之下 —— 把帧缩小或拖开就能看到</div>
          <div class="mini-grid mt-4">
            <div class="mini-field"><label>渐变</label>
              <select v-model="gradientDir">
                <option value="left">左</option>
                <option value="right">右</option>
                <option value="top">上</option>
                <option value="bottom">下</option>
                <option value="radial">径向</option>
                <option value="none">无</option>
              </select>
            </div>
            <div class="mini-field"><label>暗角</label><input type="range" min="0" max="1" step="0.02" v-model.number="vignette"><span class="rv">{{ Number(vignette).toFixed(2) }}</span></div>
          </div>
        </details>

        <details open class="grp">
          <summary>封面特效（可叠加）</summary>
          <div v-for="(fx, i) in fxList" :key="i" class="fx-card">
            <div class="fx-head">
              <span class="chip p">{{ fxTypeName(fx.type) }}</span>
              <button class="mini-btn danger" @click="removeFx(i)">删除</button>
            </div>
            <!-- 表情：先挑字符（可直接粘贴 emoji），默认贴主播身上 -->
            <template v-if="fx.type === 'emoji'">
              <div class="mini-row mt-4">
                <input v-model="fx.text" class="hex-input grow" maxlength="12" placeholder="🙂" title="要贴的表情，可直接粘贴 emoji（最多 12 个字符）">
              </div>
              <div class="emoji-picks mt-4">
                <button v-for="e in EMOJI_PICKS" :key="e" class="emoji-pick" :title="'用 ' + e" @click="fx.text = e">{{ e }}</button>
              </div>
            </template>
            <div class="mini-grid mt-4">
              <div class="mini-field"><label>{{ fx.type === 'emoji' ? '大小' : '强度' }}</label><input type="range" min="0" max="1" step="0.05" v-model.number="fx.amount"><span class="rv">{{ Number(fx.amount).toFixed(2) }}</span></div>
              <div class="mini-field"><label>X</label><input type="range" min="0" max="100" step="1" v-model.number="fx.x"><span class="rv">{{ fx.x }}%</span></div>
            </div>
            <div class="mini-grid mt-4">
              <div class="mini-field"><label>Y</label><input type="range" min="0" max="100" step="1" v-model.number="fx.y"><span class="rv">{{ fx.y }}%</span></div>
              <div v-if="fx.type === 'overlay'" class="mini-field"><label>颜色</label><input type="color" v-model="fx.color"></div>
            </div>
          </div>
          <div class="mini-row mt-4">
            <select v-model="addType" class="mini-select grow">
              <option value="glow">光晕</option>
              <option value="blur">模糊</option>
              <option value="vignette">暗角点</option>
              <option value="overlay">颜色覆盖</option>
              <option value="scanline">扫描线</option>
              <option value="emoji">表情 emoji</option>
            </select>
            <button class="mini-btn" @click="addFx"><Icon name="plus" :size="11" /> 添加</button>
          </div>
        </details>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { api } from '../api/client'
import { toast } from '../stores/toast'
import { addClient } from '../stores/log'
import Icon from './Icon.vue'
import { mediaUrl, fmtTime } from '../utils'
import { watchJob, busyOn, clipKey, kindText } from '../api/jobs'
import { useJobs } from '../composables/useJobs'
import { fetchFonts } from '../api/polish'
import { ensureFontFace } from '../polish/fonts'

const props = defineProps({
  dir: { type: String, required: true },
  /** 项目 id（拼资源键用，与后端 server/scope_key.rs 一致）；缺省时不做作用域门控 */
  pid: { type: [String, Number], default: '' }
})
const emit = defineEmits(['changed'])
const generating = ref(false)
// 封面与渲染/LLM/后处理写同一批文件（polish.toml / cover.jpg / _cover_frame.jpg），
// 只在该片段被占用时禁用「生成封面」；改参数是同步保存，随时可存。
const { jobs: allJobs } = useJobs()
const clipBusyJob = computed(() => {
  // 反斜杠用 charCode 拼，别在字符串里写转义（切片页 sameDir 同款写法）
  const name = String(props.dir || '').split(String.fromCharCode(92)).join('/').split('/').filter(Boolean).pop() || ''
  const pid = props.pid
  if (!name || !pid) return null
  return busyOn(allJobs.value, [clipKey(pid, name)])
})
const generatingTitle = ref(false)
/**
 * 封面配置是否已成功读回。**它同时是一道写入门闸**（读失败时用默认值写回 = 把磁盘上的
 * 配置清成默认），所以不能随便放开 —— 但正因为是门闸，读失败就必须喊出来：以前 `onMounted`
 * 用裸 `catch {}` 收尾，configLoaded 永远停在 false，于是这个面板里**之后所有改动都不再保存**，
 * 界面上一个字都不提示（用户花十分钟调封面，关掉再开全没了）。现在配合下面的 error banner。
 */
const configLoaded = ref(false)
const configLoadError = ref('')
const saveError = ref('')
const savingConfig = ref(false)
const stageRef = ref(null)
const stageWrapRef = ref(null)
const controlsRef = ref(null)
const stageScale = ref(1)
let resizeRo = null
const videoSrc = ref('')
const videoRef = ref(null)
const fileInput = ref(null)
const titleLines = ref([])
/** `.title-text` 元素本身（自动排版要量它的 rect；量 `.title-wrap` 会量到那个更宽的容器） */
const titleTextRef = ref(null)
/** 自动排版放不下时给用户的一句提示（放不下就说出来，别静默） */
const titleFitHint = ref('')
const time = ref(0)
const textX = ref(35)
const textY = ref(50)
const textWidth = ref(62)
const preset = ref('left-text')
const fontSize = ref(120)
const stroke = ref(20)
const hlColor = ref('#ffcc00')
const highlightKeywords = ref([])

// ---- 封面字体（字体库 = 程序目录 resources/fonts/）----
// 值存 polish.toml 的 `[cover] font`：空 = 系统字体栈（老工程就是这个状态），
// 非空 = 库里的字体 id。出片由 cover.rs 注入 @font-face，这里负责**让设计器预览也认得它**。
const fonts = ref([])
const fontsRoot = ref('')
const coverFont = ref('')
const fontOptions = computed(() => {
  const opts = [{ value: '', label: '（系统默认）' }]
  for (const f of fonts.value) {
    opts.push({ value: f.id, label: f.family && f.family !== f.id ? `${f.family}（${f.id}）` : f.id })
  }
  const cur = coverFont.value
  if (cur && !fonts.value.some((f) => f.id === cur)) opts.push({ value: cur, label: `${cur}（不在字体库）` })
  return opts
})
/** 选中的即浏览器里要用的字族名；未选时交给全站 `--font`（不写 fontFamily，保持继承） */
const coverFontFamily = computed(() => (coverFont.value ? `"${coverFont.value}"` : ''))

async function loadFonts() {
  try {
    const r = await fetchFonts('读取字体库')
    fonts.value = r.fonts || []
    fontsRoot.value = r.root || ''
  } catch (e) {
    addClient('warn', '字体库读取失败，封面字体只能手填：' + (e.message || e), 'cover')
  }
}

/** 换封面字体：改的是 ref，落盘交给下面那条防抖保存（与其它封面参数同一条路） */
function onCoverFontChange(e) {
  const next = e.target.value
  coverFont.value = next
  // 先让浏览器认得它，预览才会立刻换字形（出片侧的 @font-face 由 cover.rs 注入）
  if (next) ensureFontFace(props.pid, props.dir, next, { onDebug: (m) => addClient('debug', m, 'cover') })
}
const highlightInput = ref('')
const letterSpacing = ref(8)
const lineHeight = ref(1.7)
const textShadow = ref(true)
const gradientDir = ref('left')
const vignette = ref(0.08)
const align = ref('left')
/** 字幕高亮色（`[style.subtitle] highlight_color`）—— 出片的渐变底色由它推导，预览要跟着它 */
const subtitleHlColor = ref('')
const layout = ref({})
const duration = ref(0)
const playing = ref(false)
/** 背景模式：color（16 进制纯色，默认黑）/ blur（当前帧模糊平铺）—— 只有这两种。
 *  背景是**帧之下的一层**：帧缩小/平移时露出，不替换帧（用户要求）。 */
const bgMode = ref('color')
const bgColor = ref('#000000')
/** 手填的 16 进制码（与取色器双向同步；只认 #rgb / #rrggbb） */
const bgColorText = ref('#000000')
const blurScale = ref(1)
/** 模糊背景用的是出片同一张 `_cover_frame.jpg`（预览=出片同一份素材，不是另一条时间轴） */
const bgImageOk = ref(true)
/** 那张截帧不在时自动补一次（出片结束会删掉它，所以平时确实没有这个文件） */
const bgFrameTried = ref(false)
/** 导出关键帧后自增，给 URL 去缓存 */
const frameStamp = ref(0)
const BG_MODES = [
  { key: 'color', label: '纯色', hint: '16 进制颜色铺满背景（默认黑）；帧缩小/平移时才露出来' },
  { key: 'blur', label: '当前帧模糊', hint: '关键帧（当前帧）模糊后平铺铺满，可调模糊倍率' },
  { key: 'image', label: '自定义图片', hint: '导入的图片铺满背景（写在 _cover_bg.jpg，不覆盖关键帧）' }
]
/** 导入背景图后自增，给 `_cover_bg.jpg` 的 URL 去缓存 */
const bgStamp = ref(0)
/** 帧层（当前帧清晰图）的位移/缩放：拖动预览 = 平移，滚轮 = 缩放（用户要求：按之前的逻辑） */
const coverZoom = ref(1)
const coverOffsetX = ref(0)
const coverOffsetY = ref(0)
const stageDrag = ref(null)
const fxList = ref([])
const addType = ref('glow')
/** 安全区线框的开关（纯设计器辅助线，不进出片模板、不进 cover.toml）。
 *  偏好**在 setup 里同步读** —— 放进 computed 或 onMounted 会先画一帧再切，闪一下
 *  （与 `stores/ui.js::readSideCollapsed` 同一个教训）。两个开关各自独立，可以只看 4:3。 */
const LS_GUIDES = 'vtrim.ui.coverGuides'
const showGuide169 = ref(readGuides().includes('169'))
const showGuide43 = ref(readGuides().includes('43'))

function readGuides() {
  try {
    return String(localStorage.getItem(LS_GUIDES) ?? '169|43')
  } catch {
    return '169|43'
  }
}

function toggleGuide(key) {
  const flag = key === '169' ? showGuide169 : showGuide43
  flag.value = !flag.value
  const on = []
  if (showGuide169.value) on.push('169')
  if (showGuide43.value) on.push('43')
  try { localStorage.setItem(LS_GUIDES, on.join('|')) } catch {}
}
let saveConfigTimer = null
let saveConfigPending = false
const titleLinesText = computed({
  get: () => titleLines.value.join('\n'),
  set: (v) => { titleLines.value = String(v || '').split('\n') }
})

function escapeHtml(s) {
  return String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]))
}

/** 没有 `title_lines` 时的回退：与 `cover.rs` 一致 —— 砍掉 `【主播名】` 前缀，再按中英标点拆行。 */
function splitTitleFallback(raw) {
  const clean = String(raw || '').split('【')[0]
  return clean
    .split(/[，。！？、,.!]/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0)
}

function highlightTitleLine(line, kws) {
  let html = escapeHtml(line)
  for (const kw of kws) {
    const target = escapeHtml(kw).toLowerCase()
    if (!target) continue
    let out = ''
    let rest = html
    let idx = rest.toLowerCase().indexOf(target)
    while (idx >= 0) {
      out += rest.slice(0, idx) + '<span class="hl">' + rest.slice(idx, idx + target.length) + '</span>'
      rest = rest.slice(idx + target.length)
      idx = rest.toLowerCase().indexOf(target)
    }
    html = out + rest
  }
  return html
}

const titleHtml = computed(() => {
  const lines = titleLines.value.length ? titleLines.value : ['']
  const kws = highlightKeywords.value.filter(Boolean)
  return lines.map(line => highlightTitleLine(line, kws)).join('\n')
})

const presets = [
  { key: 'left-text', label: '左文', x: 35, y: 50, width: 62, align: 'left', textShadow: true, gradientDir: 'left', vignette: 0.08 },
  { key: 'center-text', label: '居中', x: 50, y: 50, width: 70, align: 'center', textShadow: true, gradientDir: 'radial', vignette: 0.05 },
  { key: 'right-text', label: '右文', x: 65, y: 50, width: 62, align: 'right', textShadow: true, gradientDir: 'right', vignette: 0.08 },
  { key: 'minimal', label: '极简', x: 50, y: 50, width: 72, align: 'center', textShadow: false, gradientDir: 'none', vignette: 0.15 }
]

const fxNames = { glow: '光晕', blur: '模糊', vignette: '暗角点', overlay: '颜色覆盖', scanline: '扫描线', emoji: '表情' }
function fxTypeName(t) { return fxNames[t] || t }
/** 常用表情：点一下换字符（也能直接往输入框里粘任意 emoji） */
const EMOJI_PICKS = ['😆', '😂', '😭', '🥺', '😮', '❤️', '🔥', '✨']
/** emoji 的尺寸区间（px，1920×1080 画布）——与 Rust `cover.rs` 的 COVER_EMOJI_* 同源 */
const COVER_EMOJI_MIN_PX = 48
const COVER_EMOJI_MAX_PX = 288
/** `#rrggbb` → `r,g,b`（特效颜色要拼进 rgba()，与 Rust `hex_to_rgb` 同一套）。 */
function fxHexToRgb(hex) {
  const h = String(hex || '').trim().replace('#', '')
  const full = h.length === 3 ? h.split('').map((c) => c + c).join('') : h
  if (!/^[0-9a-fA-F]{6}$/.test(full)) return '0,0,0'
  return [full.slice(0, 2), full.slice(2, 4), full.slice(4, 6)]
    .map((c) => parseInt(c, 16))
    .join(',')
}

function fxStyle(fx) {
  const s = stageScale.value || 1
  const amount = Number(fx.amount) || 0.3
  const x = Number(fx.x ?? 50)
  const y = Number(fx.y ?? 50)
  if (fx.type === 'emoji') {
    // x/y 是字符中心（定位/居中靠 .fx-emoji 的 CSS），字号按舞台缩放；
    // 阴影同样按舞台缩放（出片是 1920 画布上的 px，模板写的是 0 6px 14px）
    return {
      left: `${x}%`,
      top: `${y}%`,
      fontSize: `${Math.round((COVER_EMOJI_MIN_PX + amount * (COVER_EMOJI_MAX_PX - COVER_EMOJI_MIN_PX)) * s)}px`,
      filter: `drop-shadow(0 ${(6 * s).toFixed(1)}px ${(14 * s).toFixed(1)}px rgba(0,0,0,0.45))`
    }
  }
  if (fx.type === 'glow') return {
    background: `radial-gradient(circle at ${x}% ${y}%, rgba(255,220,180,${Math.min(0.9, amount)}) 0%, transparent 60%)`
  }
  if (fx.type === 'blur') return {
    // 与 Rust `cover_fx_html` 的 blur 分支同公式：`1 + amount×14` px（画布 px → 按舞台缩放）
    backdropFilter: `blur(${((1 + amount * 14) * s).toFixed(1)}px)`,
    WebkitMaskImage: `radial-gradient(circle at ${x}% ${y}%, black 0%, transparent 60%)`,
    maskImage: `radial-gradient(circle at ${x}% ${y}%, black 0%, transparent 60%)`
  }
  if (fx.type === 'vignette') return {
    background: `radial-gradient(circle at ${x}% ${y}%, transparent 40%, rgba(0,0,0,${Math.min(1, amount)}) 100%)`
  }
  if (fx.type === 'overlay') {
    // 与 Rust 同一实现：颜色直接编成 rgba（不透明度 = amount，无额外夹取）+ overlay 混合，
    // 不是"铺个色块再调元素 opacity"
    return {
      background: `rgba(${fxHexToRgb(fx.color)},${amount})`,
      mixBlendMode: 'overlay'
    }
  }
  if (fx.type === 'scanline') return {
    background: `repeating-linear-gradient(0deg, rgba(255,255,255,${Math.min(0.5, amount * 0.45)}) 0 1px, transparent 1px 4px)`
  }
  return {}
}


/** 文字阴影：数值与出片模板 `cover_shadow_css` 一致（`0 5px 22px rgba(0,0,0,0.78)`），
 *  再按舞台缩放 —— 出片是 1920 画布上的 px，预览舞台是缩小的，不缩放就会看着更糊。 */
function coverShadowCss(s) {
  if (!textShadow.value) return 'none'
  return `0 ${(5 * s).toFixed(1)}px ${(22 * s).toFixed(1)}px rgba(0,0,0,0.78)`
}

const titleStyle = computed(() => {
  const s = stageScale.value || 1
  const ls = Number(letterSpacing.value) || 0
  return {
    fontSize: Math.max(12, Math.round((fontSize.value || 120) * s)) + 'px',
    // 选中的封面字体（空 = 不写，继续继承全站 `--font`，与出片模板的默认栈一致）
    fontFamily: coverFontFamily.value || undefined,
    letterSpacing: (ls * s).toFixed(1) + 'px',
    lineHeight: Number(lineHeight.value) || 1.7,
    // 描边色与模板一致（`#2a2522`，不是半透明黑）
    WebkitTextStroke: ((stroke.value || 20) * s).toFixed(1) + 'px #2a2522',
    textShadow: coverShadowCss(s),
    '--hlc': hlColor.value,
    textAlign: align.value || 'left'
  }
})

// ===== 标题安全区自动排版 =====
//
// 目标矩形 = 设计器里那条 **4:3 裁切区** 线框（`.guide-43`）。线框是 `height:100%` +
// `aspect-ratio:4/3` 居中，所以在 1920×1080 画布上它是 1440 宽、左右各留 240px、上下贴边。
// 标题放进去的含义：封面被平台裁成 4:3 时一个字都不会掉。
// 安全区目前**只存在于设计器**（`cover.rs` / `cover.html` 完全不知道它），所以这件事只能在
// 前端做 —— 好在设计器用的是与出片同一套 CSS（字体栈、line-height、text-align、
// `-webkit-text-stroke`），能量到真实排版。
const SAFE_X0 = 240
const SAFE_X1 = 1680
/** 字号下限，与「字号」滑杆的 min 一致 */
const TITLE_FONT_MIN = 24

/** 内缩一圈安全边距（画布 px）：`-webkit-text-stroke` 是沿字形轮廓**向外**画的，
 *  所以要留半个描边宽度；再留 12px 吸收「设计器用的字体」与「出片 headless Chrome
 *  能拿到的字体」不一致带来的排版差 —— 这个功能的定位是"把初始排版调好"，
 *  不是像素级契约，所以宁可保守。 */
function titleSafeMargin() {
  return 12 + (Number(stroke.value) || 0) / 2
}

/**
 * 首次拿到标题时把排版调好并放进安全区（只动 **字号 / 位置**，以及在确实换行过多时
 * 放宽一次文字区宽度）。对齐 / 行距 / 字距 / 描边 / 颜色 / 高亮词一律不碰。
 *
 * ⚠️ **只在「标题被获取」时调用**：`loadCoverConfig` 发现磁盘上没有 `title_lines`
 * （新工程，靠 `project.name` 回退拆行）、以及「LLM 生成标题」拿到新标题之后。
 * 手动操作（改文字 / 拖滑杆 / 点预设 / 拖拽）**一律不调** —— 否则用户每动一下都要跟
 * 自动排版打架，这正是"仅限第一次"要防的。
 */
async function fitTitleIntoSafeArea() {
  const stage = stageRef.value
  const el = titleTextRef.value
  // 没有任何非空行就不用排（空标题量出来是 0 高，排了也没意义）
  if (!stage || !el || !titleLines.value.some((l) => String(l ?? '').trim())) return
  // 字体没就绪时量到的是回退字体的宽度，排完照样会偏
  try { await document.fonts?.ready } catch {}
  await nextTick()

  const s = stageScale.value || 1
  const stageW = stage.clientWidth
  const stageH = stage.clientHeight
  if (!(stageW > 0) || !(stageH > 0)) return

  const m = titleSafeMargin()
  const left = (SAFE_X0 + m) * s
  const right = (SAFE_X1 - m) * s
  const top = m * s
  const bottom = (1080 - m) * s
  const maxW = Math.max(1, right - left)
  const maxH = Math.max(1, bottom - top)

  // 量的是**文字本身**的范围（用 Range 盖住文本节点），不是 `.title-text` 那个块级盒子的宽度。
  // ⚠️ 差别很大：设计器里 `.title-text` 是块级元素、宽度 = `.title-wrap` 的 62%；而出片模板
  // `cover.html` 的 `#title-wrap` 是 flex 列、`#title` 会收缩到内容宽。两者**视觉一致**
  // （左/中/右三种对齐下文字落点相同），但盒宽差好几百 px。
  // 直接量盒子会把"居中/右对齐的短标题"误判成溢出，再硬把 `text_x` 拽到 50% 附近 ——
  // 那是把本来没问题的排版改坏。量文字就没这个问题。
  const measure = () => {
    const sb = stage.getBoundingClientRect()
    let bb = null
    try {
      const range = document.createRange()
      range.selectNodeContents(el)
      const r = range.getBoundingClientRect()
      if (r.width > 0 && r.height > 0) bb = r
    } catch { /* 取不到就退回元素盒子 */ }
    if (!bb) bb = el.getBoundingClientRect()
    return { w: bb.width, h: bb.height, l: bb.left - sb.left, t: bb.top - sb.top }
  }
  const fits = (b) => b.w <= maxW + 0.5 && b.h <= maxH + 0.5

  // ① 字号：超出就按「两个方向里更紧的那个」比例缩。要迭代 —— 字号一变换行数会跳变，
  //    所以每轮都必须重新量，不能一次算完。
  for (let i = 0; i < 6; i++) {
    const b = measure()
    if (fits(b)) break
    const cur = Number(fontSize.value) || 120
    if (cur <= TITLE_FONT_MIN) break          // 到头了，剩下的交给 ③ 与下面的告警
    const k = Math.min(maxW / Math.max(b.w, 1), maxH / Math.max(b.h, 1)) * 0.98
    const next = Math.max(TITLE_FONT_MIN, Math.floor((cur * k) / 2) * 2)
    if (next >= cur) break                    // 比例算下来缩不动（k ≈ 1），没必要空转
    fontSize.value = next
    await nextTick()
  }

  // ② 最后手段：横向有余、却**因为换行太多而偏高**时，把文字区放宽一档（放宽 → 每行装更多
  //    → 行数变少 → 变矮）。只在确实变矮时才留下，否则原样退回去。
  let b = measure()
  if (!fits(b) && b.h > maxH && b.w <= maxW) {
    const before = textWidth.value
    const wider = Math.min(96, Math.max(before + 6, Math.ceil((maxW / s / 1920) * 100)))
    if (wider > before) {
      textWidth.value = wider
      await nextTick()
      b = measure()
      if (!fits(b)) {
        textWidth.value = before
        await nextTick()
        b = measure()
      }
    }
  }

  // ③ 位置：把整块挪进安全区。`textX/textY` 是 `.title-wrap` 的**中心点**
  //    （`translate(-50%,-50%)`），所以平移中心即整体平移；量的是文字本身（`.title-text`），
  //    不会因为外层那个 62% 宽的容器把本来在框内的标题白白挪走。
  let dx = 0
  let dy = 0
  if (b.w > maxW) dx = (left + right) / 2 - (b.l + b.w / 2)   // 比框还宽：只能居中
  else if (b.l < left) dx = left - b.l
  else if (b.l + b.w > right) dx = right - (b.l + b.w)
  if (b.h > maxH) dy = (top + bottom) / 2 - (b.t + b.h / 2)
  else if (b.t < top) dy = top - b.t
  else if (b.t + b.h > bottom) dy = bottom - (b.t + b.h)
  if (dx || dy) {
    const pct = (v) => Math.round(Math.min(100, Math.max(0, v)) * 10) / 10
    textX.value = pct(Number(textX.value) + (dx / stageW) * 100)
    textY.value = pct(Number(textY.value) + (dy / stageH) * 100)
    await nextTick()
    b = measure()
  }

  // 告警只看**最终状态**：中途缩不动不等于失败（可能其实已经放得下了）。
  // 判定 = 「到最小字号仍然溢出」，也就是现在这一刻还没进框。
  const relaxed = !fits(b)
  titleFitHint.value = relaxed
    ? '标题按最小字号仍放不进 4:3 安全区，已按最小溢出摆放 —— 建议缩短标题或调窄文字区'
    : ''
  if (relaxed) console.warn('[cover] 标题按最小字号仍放不进 4:3 安全区', b, { maxW, maxH })
}

/** 渐变底色：与 `cover.rs::build_cover_html` **同一套推导**（从字幕高亮色反推一个暗色调）。
 *  以前这里写死 `rgba(20,10,40,…)`，而出片是按 `[style.subtitle] highlight_color` 算的 ——
 *  改一个字幕高亮色，出片的渐变色调就变了，设计器却纹丝不动（用户报"设计器和出片不一致"）。 */
function coverGradientRgb() {
  const hex = String(subtitleHlColor.value || '').trim()
  if (!/^#[0-9a-fA-F]{3,6}$/.test(hex)) return [20, 10, 40]
  const h = hex.replace('#', '')
  const full = h.length === 3 ? h.split('').map((c) => c + c).join('') : h
  const r = (parseInt(full.slice(0, 2), 16) || 0) / 255
  const g = (parseInt(full.slice(2, 4), 16) || 0) / 255
  const b = (parseInt(full.slice(4, 6), 16) || 0) / 255
  return [
    Math.max(10, Math.floor((1 - r) * 30)),
    Math.max(10, Math.floor((1 - g) * 25)),
    Math.max(15, Math.floor((1 - b) * 50))
  ]
}

const gradientStyle = computed(() => {
  const dir = gradientDir.value
  if (dir === 'none') return { display: 'none' }
  const [r, g, b] = coverGradientRgb()
  const from = `rgba(${r},${g},${b},0.55)`
  const to = `rgba(${r},${g},${b},0.15)`
  if (dir === 'radial') return { background: `radial-gradient(ellipse at 30% 50%, ${from} 0%, ${to} 60%, transparent 100%)` }
  if (dir === 'right') return { background: `linear-gradient(270deg, ${from} 0%, ${to} 50%, transparent 100%)` }
  if (dir === 'top') return { background: `linear-gradient(180deg, ${from} 0%, ${to} 50%, transparent 100%)` }
  if (dir === 'bottom') return { background: `linear-gradient(0deg, ${from} 0%, ${to} 50%, transparent 100%)` }
  return { background: `linear-gradient(90deg, ${from} 0%, ${to} 50%, transparent 100%)` }
})


// 背景只有两种模式，这里的公式必须与 Rust `cover.rs` 保持一致
/** 模糊平铺的基准半径（px，1920×1080 画布）——与 Rust `cover.rs::COVER_BLUR_BASE_PX` 同源，
    改一边必须改另一边。 */
const COVER_BLUR_BASE_PX = 28
/** 模糊倍率 → { px, imgScale }：与 Rust `cover_blur_css` 同一套公式（含 0.2~4 夹取）。
    imgScale 用于把模糊溢出的边缘推出画面（blur 的扩散约等于半径的 3 倍）。 */
function coverBlurCss(scale) {
  const s = Math.min(4, Math.max(0.2, Number(scale) || 1))
  const px = COVER_BLUR_BASE_PX * s
  return { px, imgScale: Math.round((1 + px / 150) * 1000) / 1000 }
}

/** 背景模糊层（出片同一张 `_cover_frame.jpg`）的样式；帧层在它之上，互不影响。 */
const bgBlurStyle = computed(() => {
  const { px, imgScale } = coverBlurCss(blurScale.value)
  return {
    filter: `blur(${(px * (stageScale.value || 1)).toFixed(1)}px)`,
    transform: `scale(${imgScale})`,
    transformOrigin: 'center center'
  }
})
// 背景层的图源：blur 用关键帧 `_cover_frame.jpg`，image 用导入的 `_cover_bg.jpg`
// —— 两个文件分开，导入背景不会盖掉「关键帧层」（与 Rust 的 COVER_FRAME_IMG / COVER_BG_IMG 同源）
const bgImageSrc = computed(() => {
  if (!props.dir) return ''
  if (bgMode.value === 'image') return mediaUrl(props.dir, '_cover_bg.jpg') + '?t=' + bgStamp.value
  return mediaUrl(props.dir, '_cover_frame.jpg') + '?t=' + frameStamp.value
})

/**
 * 背景截帧（`_cover_frame.jpg`）**平时不存在**：出片写完 cover.jpg 就把它删了，
 * 所以「当前帧模糊」一进设计器基本必然 404 → 图被 v-if 摘掉，看起来"没运行"。
 * 这里补一次：不带 time 调截帧接口，服务端按 `[cover] frame_pos` 截 —— 与出片自己要截的
 * 是**同一帧**，不会偷偷改渲染用的时间点。补过还失败就不再重试（避免死循环），给一句提示。
 */
async function onBgImageError() {
  bgImageOk.value = false
  // image 模式缺图 = 还没导入，别去补关键帧（那是另一层的东西）
  if (bgMode.value === 'image' || bgFrameTried.value) return
  bgFrameTried.value = true
  try {
    await api('/api/polish/cover-frame', { method: 'POST', body: JSON.stringify({ dir: props.dir }) })
    frameStamp.value++
    bgImageOk.value = true
  } catch (e) {
    console.warn('生成背景截帧失败', e)
  }
}

// 换背景模式时重试一次（上一次的失败状态不该黏住）
watch(bgMode, () => {
  bgImageOk.value = true
  bgFrameTried.value = false
})

/** 手填 16 进制码：认 #rgb / #rrggbb（不带 # 也认），展开成小写 #rrggbb；非法就退回原值。 */
function commitBgColorText() {
  const raw = String(bgColorText.value || '').trim()
  const hex = raw.startsWith('#') ? raw.slice(1) : raw
  if (!/^[0-9a-fA-F]+$/.test(hex) || (hex.length !== 3 && hex.length !== 6)) {
    toast('颜色要写 16 进制码，如 #1a2b3c 或 #abc', 'err')
    bgColorText.value = bgColor.value
    return
  }
  const full = hex.length === 3 ? hex.split('').map((c) => c + c).join('') : hex
  const next = ('#' + full).toLowerCase()
  bgColorText.value = next
  bgColor.value = next
}

// 取色器改颜色时同步到文本框（文本框 → 取色器走 commitBgColorText）
watch(bgColor, (v) => { bgColorText.value = v }, { immediate: true })

// 帧层（当前帧）的位移/缩放：与后端 cover.rs::cover_frame_vars 同一套公式 ——
// 原点取 layout.liver 的聚焦点，基准 1.4（liver 在右半边）/ 1.3，再乘「帧缩放」。
const GRID_CENTERS = {
  'top-left': '16.67% 16.67%', top: '50% 16.67%', 'top-right': '83.34% 16.67%',
  'mid-left': '16.67% 50%', center: '50% 50%', 'mid-right': '83.34% 50%',
  'bot-left': '16.67% 83.33%', bottom: '50% 83.33%', 'bot-right': '83.34% 83.33%'
}

/**
 * 主播（`layout.liver`）在画面里的百分比中心。优先序与 `vtrim-render-core::region_origin`、
 * `webui/src/polish/engine.js::regionOrigin` **完全一致**：
 * **自由选框（画过框 → 框中心）> origin_grid > origin > grid[0]**，都没有 → `65% 50%`。
 * 帧层拿它当聚焦原点；**emoji 默认就贴在这个点上**（那个点是帧层变换的不动点，缩放帧也不跑偏）。
 * ⚠️ 以前这里漏了自由选框：画过人物框之后，出片按框中心聚焦，预览还按九宫格 —— 又一处不一致。
 */
function liverOrigin() {
  const reg = layout.value?.liver || {}
  const tl = reg.top_left
  const br = reg.bottom_right
  if (tl && br) {
    const x0 = parseFloat(String(tl.x ?? ''))
    const y0 = parseFloat(String(tl.y ?? ''))
    const x1 = parseFloat(String(br.x ?? ''))
    const y1 = parseFloat(String(br.y ?? ''))
    // 与 Rust `free_rect` 同样的有效性判断（框太小 = 没框）
    if ([x0, y0, x1, y1].every(Number.isFinite) && Math.abs(x1 - x0) >= 0.5 && Math.abs(y1 - y0) >= 0.5) {
      return { x: (x0 + x1) / 2, y: (y0 + y1) / 2 }
    }
  }
  let origin = ''
  if (reg.origin_grid && GRID_CENTERS[reg.origin_grid]) {
    origin = GRID_CENTERS[reg.origin_grid]
  } else if (reg.origin) {
    origin = reg.origin
  } else if (reg.grid && reg.grid.length && GRID_CENTERS[reg.grid[0]]) {
    origin = GRID_CENTERS[reg.grid[0]]
  }
  const parts = String(origin).split(/\s+/)
  const x = parseFloat(parts[0])
  const y = parseFloat(parts[1])
  return { x: Number.isFinite(x) ? x : 65, y: Number.isFinite(y) ? y : 50 }
}

const mediaStyle = computed(() => {
  const { x: baseX, y: baseY } = liverOrigin()
  const baseZoom = baseX >= 50 ? 1.4 : 1.3
  const zoom = (baseZoom || 1) * (coverZoom.value || 1)
  return {
    transformOrigin: baseX + '% ' + baseY + '%',
    transform: `translate(${coverOffsetX.value || 0}px, ${coverOffsetY.value || 0}px) scale(${zoom})`
  }
})

/** 帧复位：缩放回 100%、偏移归零（拖飞了能一键回来）。 */
function resetFrame() {
  coverZoom.value = 1
  coverOffsetX.value = 0
  coverOffsetY.value = 0
}

function applyPreset(key) {
  const p = presets.find(x => x.key === key)
  if (!p) return
  preset.value = key
  textX.value = p.x
  textY.value = p.y
  if (p.width) textWidth.value = p.width
  align.value = p.align || 'center'
  textShadow.value = p.textShadow !== false
  if (p.gradientDir) gradientDir.value = p.gradientDir
  if (p.vignette != null) vignette.value = p.vignette
}

function takeCurrent() {
  const v = videoRef.value
  if (v) time.value = Math.round((v.currentTime || 0) * 10) / 10
}

function addFx() {
  const type = addType.value
  // emoji 默认贴主播身上（帧层的聚焦原点 = 变换不动点，缩放帧也不会把它带跑）
  const at = type === 'emoji' ? liverOrigin() : { x: 50, y: 50 }
  fxList.value.push({
    type,
    amount: type === 'emoji' ? 0.5 : 0.3,
    x: Math.round(at.x),
    y: Math.round(at.y),
    color: '#000000',
    text: type === 'emoji' ? '😆' : ''
  })
}
function removeFx(i) { fxList.value.splice(i, 1) }

async function exportFrame() {
  try {
    await api('/api/polish/cover-frame', { method: 'POST', body: JSON.stringify({ dir: props.dir, time: time.value || 0 }) })
    // 出片的模糊背景就是这张文件：去掉缓存让预览立刻换成新截的那一帧
    frameStamp.value++
    bgImageOk.value = true
    toast('关键帧已导出')
  } catch (e) { toast(e.message) }
}

async function importBg(e) {
  const file = e.target.files[0]
  if (!file) return
  const reader = new FileReader()
  reader.onload = async () => {
    try {
      const base64 = String(reader.result).split(',')[1]
      await api('/api/polish/cover-background', { method: 'POST', body: JSON.stringify({ dir: props.dir, image_data: base64 }) })
      // 服务端写的是 `_cover_bg.jpg` + bg_mode=image：本地同步切过去并刷新图
      bgMode.value = 'image'
      bgStamp.value++
      bgImageOk.value = true
      toast('背景已导入（关键帧层不受影响）')
    } catch (err) { toast(err.message) }
  }
  reader.readAsDataURL(file)
}

async function generateTitleByLlm() {
  if (generatingTitle.value) return
  generatingTitle.value = true
  try {
    const r = await api('/api/polish/cover-title', { method: 'POST', body: JSON.stringify({ dir: props.dir }) })
    const lines = r?.title_lines || []
    if (lines.length) {
      titleLines.value = lines
      // 标题内容整体换了一批 → 重新排一次并放回安全区（行数/字数变了，旧字号不再适用）。
      // 这是"标题被获取"的第二种情形；手动改文字/拖滑杆**不走这里**。
      await fitTitleIntoSafeArea()
    } else {
      toast('LLM 未返回标题')
    }
  } catch (e) {
    toast('LLM 生成标题失败：' + (e.message || e), 'err')
  } finally {
    generatingTitle.value = false
  }
}

function addHighlightKeyword() {
  const kw = highlightInput.value.trim()
  if (!kw) return
  if (!highlightKeywords.value.includes(kw)) {
    highlightKeywords.value = [...highlightKeywords.value, kw]
  }
  highlightInput.value = ''
}

function removeHighlightKeyword(i) {
  highlightKeywords.value.splice(i, 1)
}

function coverConfigPayload() {
  return {
    title_lines: titleLines.value,
    text_width: textWidth.value / 100,
    text_x: textX.value + '%',
    text_y: textY.value + '%',
    font_size: fontSize.value,
    // 封面字体：字体库里的 id，空串 = 系统字体栈（cover.rs 就是这么解释的）
    font: coverFont.value,
    stroke: stroke.value,
    hl_color: hlColor.value,
    highlight_keywords: highlightKeywords.value,
    letter_spacing: letterSpacing.value + 'px',
    line_height: lineHeight.value,
    text_shadow: textShadow.value,
    shadow_dir: textShadow.value ? 'center' : 'none',
    shadow_intensity: textShadow.value ? 1 : 0,
    gradient_dir: gradientDir.value,
    vignette: vignette.value,
    text_align: align.value,
    effects: fxList.value.map(f => ({
      kind: f.type,
      amount: Number(f.amount) || 0.3,
      x: Number(f.x) || 50,
      y: Number(f.y) || 50,
      color: f.color || '#000000',
      // 只有 emoji 用；服务端渲染前还会再转义一次
      text: f.type === 'emoji' ? String(f.text || '').slice(0, 12) : ''
    })),
    bg_mode: bgMode.value,
    // 颜色只接受 #rrggbb（服务端还会再净化一次）：它是直接进 HTML 模板的字符串
    bg_color: /^#[0-9a-fA-F]{6}$/.test(bgColor.value) ? bgColor.value.toLowerCase() : '#000000',
    blur_scale: Math.min(4, Math.max(0.2, Number(blurScale.value) || 1)),
    // 帧层（当前帧）的位移/缩放：出片写进 #frame img 的 transform，与背景层无关
    bg_x: Math.round((coverOffsetX.value || 0) / (stageScale.value || 1) * 10) / 10,
    bg_y: Math.round((coverOffsetY.value || 0) / (stageScale.value || 1) * 10) / 10,
    bg_zoom: coverZoom.value || 1
  }
}

async function persistCoverConfig() {
  if (!configLoaded.value) return
  if (savingConfig.value) { saveConfigPending = true; return }
  savingConfig.value = true
  try {
    await api('/api/polish/cover-config', {
      method: 'POST',
      body: JSON.stringify({ dir: props.dir, cover: coverConfigPayload() })
    })
    saveError.value = ''
  } catch (e) {
    // 以前只 `console.warn` + 一次 toast：600ms 防抖下连着改会刷屏，而且 toast 一闪就没了。
    // 改成和「保存中…」同一处的常驻文字（成功即清），失败就一直是它。
    saveError.value = e.message || String(e)
    console.warn('保存封面设置失败', e)
  } finally {
    savingConfig.value = false
    if (saveConfigPending) {
      saveConfigPending = false
      persistCoverConfig()
    }
  }
}

function scheduleCoverConfigSave() {
  if (!configLoaded.value) return
  if (saveConfigTimer) clearTimeout(saveConfigTimer)
  saveConfigTimer = setTimeout(() => {
    saveConfigTimer = null
    persistCoverConfig()
  }, 600)
}

watch(
  () => [
    JSON.stringify(titleLines.value),
    textX.value, textY.value, textWidth.value,
    fontSize.value, stroke.value,
    hlColor.value, JSON.stringify(highlightKeywords.value),
    // 封面字体（`[cover] font`）也走这条防抖保存 —— 换了字体不落盘，就只是设计器里看着变了
    coverFont.value,
    letterSpacing.value, lineHeight.value,
    textShadow.value,
    gradientDir.value, vignette.value,
    align.value,
    JSON.stringify(fxList.value),
    bgMode.value, bgColor.value, blurScale.value,
    coverOffsetX.value, coverOffsetY.value, coverZoom.value
  ],
  scheduleCoverConfigSave
)

async function generateCover() {
  if (generating.value) return
  if (clipBusyJob.value) { toast('该片段正在' + kindText(clipBusyJob.value.kind) + '，请等它结束', 'err'); return }
  generating.value = true
  try {
    // 手动设定过时间时，先生成/刷新关键帧，避免生成时仍用默认 frame_pos
    if (Number(time.value) > 0) {
      await api('/api/polish/cover-frame', { method: 'POST', body: JSON.stringify({ dir: props.dir, time: time.value }) })
    }
    await api('/api/polish/cover-config', { method: 'POST', body: JSON.stringify({ dir: props.dir, cover: coverConfigPayload() }) })
    const coverRes = await api('/api/polish/cover', { method: 'POST', body: JSON.stringify({ dir: props.dir }) })
    if (coverRes && coverRes.id) {
      await new Promise((resolve, reject) => {
        // `watchJob()` 返回 `{ close() }` 对象（不是函数），别直接当函数调
        const watcher = watchJob(coverRes.id, {
          onDone(summary) {
            watcher.close()
            if (summary.status === 'succeeded') resolve()
            else reject(new Error(summary.error || summary.message || '封面生成失败'))
          }
        })
      })
    }
    emit('changed')
    toast('封面已生成')
  } catch (e) {
    console.error('生成封面失败', e)
    toast('生成封面失败：' + (e.message || e), 'err')
  } finally {
    generating.value = false
  }
}


function onVideoTime() {
  const v = videoRef.value
  if (v) time.value = Math.round((v.currentTime || 0) * 10) / 10
}
function onMeta() {
  const v = videoRef.value
  if (v) duration.value = v.duration || 0
}
function togglePlay() {
  const v = videoRef.value
  if (!v) return
  if (v.paused) v.play().catch(() => {})
  else v.pause()
}
function onSeekInput(e) {
  const v = videoRef.value
  if (!v || !duration.value) return
  const t = Number(e.target.value) || 0
  v.currentTime = t
  time.value = Math.round(t * 10) / 10
}
function updateStageScale() {
  const w = stageRef.value ? stageRef.value.clientWidth : 0
  stageScale.value = w > 0 ? w / 1920 : 1
}

/** 拖动预览 = 平移「当前帧」（背景不动）；松手让自动保存链把 bg_x/bg_y 写进 polish.toml。 */
function onStagePointerDown(e) {
  if (e.button !== 0) return
  e.preventDefault()
  const startX = e.clientX
  const startY = e.clientY
  const startOx = coverOffsetX.value
  const startOy = coverOffsetY.value
  const onMove = (ev) => {
    coverOffsetX.value = startOx + (ev.clientX - startX)
    coverOffsetY.value = startOy + (ev.clientY - startY)
  }
  const onUp = () => {
    window.removeEventListener('pointermove', onMove)
    window.removeEventListener('pointerup', onUp)
    window.removeEventListener('pointercancel', onUp)
    stageDrag.value = null
  }
  stageDrag.value = { onMove, onUp }
  window.addEventListener('pointermove', onMove)
  window.addEventListener('pointerup', onUp)
  window.addEventListener('pointercancel', onUp)
}

/** 滚轮 = 缩放「当前帧」（0.5~3 倍，与接口写入侧的夹取一致）。 */
function onStageWheel(e) {
  const factor = e.deltaY > 0 ? 0.9 : 1.1
  coverZoom.value = Math.min(3, Math.max(0.5, coverZoom.value * factor))
}

// 右侧设置面板与左侧预览（含播放条）等高：超出部分面板内部滚动，
// 不再让设置列把整张卡片撑得远高于编辑器；窄屏单列时解除限制。
function syncControlsBound() {
  const ctl = controlsRef.value
  if (!ctl) return
  if (!stageWrapRef.value || window.matchMedia('(max-width: 900px)').matches) {
    ctl.style.maxHeight = ''
    return
  }
  ctl.style.maxHeight = stageWrapRef.value.offsetHeight + 'px'
}

/**
 * 读一份封面配置。**失败必须可见**：`configLoaded` 是写入门闸（读失败时拿默认值写回会把
 * 磁盘配置清成默认），它一停在 false，面板里所有改动就静默不保存了 —— 所以失败时摆出
 * 错误条 + 「重试」，而不是只写一行 console。
 *
 * 返回值 = **这次是不是「首次拿到标题」**（磁盘上没有 `title_lines`，只能靠 `project.name`
 * 回退拆行）—— 调用方据此决定要不要跑一次自动排版。
 */
async function loadCoverConfig(retrying = false) {
  try {
    const cfg = await api('/api/polish/project?dir=' + encodeURIComponent(props.dir), { label: '读取封面配置' })
    layout.value = cfg?.layout || {}
    videoSrc.value = mediaUrl(props.dir, cfg?.source?.video || 'clip.mp4')
    // 标题行：与 `cover.rs::build_cover_html` 同一套回退规则 —— 没有 title_lines 时按标点
    // 把 `project.name` 拆行，并砍掉 `【主播名】` 后缀（以前这里直接用整串当一行，
    // 于是"设计器里一行、出片四行"）
    // ⚠️ 这个判据同时是「已有封面」的判据：磁盘上有 title_lines 就说明排过/存过，
    // 自动排版**一个字段都不碰**（用户要求：不动已有封面）。
    const hadSavedTitle = !!(cfg?.cover?.title_lines && cfg.cover.title_lines.length)
    titleLines.value = hadSavedTitle
      ? [...cfg.cover.title_lines]
      : splitTitleFallback(cfg?.project?.name || '')
    const parsePct = (v, fallback) => {
      const n = parseFloat(String(v ?? '').replace('%', ''))
      return Number.isFinite(n) ? n : fallback
    }
    textX.value = parsePct(cfg?.cover?.text_x, 35)
    textY.value = parsePct(cfg?.cover?.text_y, 50)
    textWidth.value = Math.round((cfg?.cover?.text_width ?? 0.62) * 100)
    fontSize.value = cfg?.cover?.font_size || 120
    coverFont.value = cfg?.cover?.font || ''
    if (coverFont.value) ensureFontFace(props.pid, props.dir, coverFont.value, { onDebug: (m) => addClient('debug', m, 'cover') })
    stroke.value = cfg?.cover?.stroke || 20
    hlColor.value = cfg?.cover?.hl_color || '#ffcc00'
    highlightKeywords.value = Array.isArray(cfg?.cover?.highlight_keywords) ? [...cfg.cover.highlight_keywords] : []
    letterSpacing.value = parseFloat(String(cfg?.cover?.letter_spacing ?? '8').replace('px', '')) || 8
    lineHeight.value = cfg?.cover?.line_height ?? 1.7
    textShadow.value = cfg?.cover?.text_shadow ?? !(cfg?.cover?.shadow_dir === 'none' || (cfg?.cover?.shadow_intensity ?? 1) <= 0)
    gradientDir.value = cfg?.cover?.gradient_dir || 'left'
    vignette.value = cfg?.cover?.vignette ?? 0.08
    // 默认左对齐 —— 与后端 `default_cover_text_align()`、默认预设「左文」保持一致
    align.value = cfg?.cover?.text_align || 'left'
    // 渐变底色的色相来自**字幕**的高亮色（`cover.rs::build_cover_html` 就是这么取的），
    // 不是封面自己的 hl_color —— 预览必须跟它同源，否则出片的渐变跟设计器不是一个色调
    subtitleHlColor.value = cfg?.style?.subtitle?.highlight_color || ''
    // 背景模式：只有 color / blur，其它值（含老配置的 frame）一律纯色，与 Rust 侧同一套规则
    const mode = String(cfg?.cover?.bg_mode || '').toLowerCase()
    bgMode.value = mode === 'blur' ? 'blur' : (mode === 'image' ? 'image' : 'color')
    bgColor.value = /^#[0-9a-fA-F]{6}$/.test(cfg?.cover?.bg_color || '') ? cfg.cover.bg_color.toLowerCase() : '#000000'
    blurScale.value = Math.min(4, Math.max(0.2, Number(cfg?.cover?.blur_scale) || 1))
    // 帧层（当前帧）的位移/缩放：服务端存的是 1920 画布上的 px，预览按舞台缩放到显示尺寸
    coverOffsetX.value = (Number(cfg?.cover?.bg_x) || 0) * stageScale.value
    coverOffsetY.value = (Number(cfg?.cover?.bg_y) || 0) * stageScale.value
    coverZoom.value = Math.min(3, Math.max(0.5, Number(cfg?.cover?.bg_zoom) || 1))
    if (Array.isArray(cfg?.cover?.effects)) {
      fxList.value = cfg.cover.effects.map(e => ({
        type: e.kind || e.type,
        amount: e.amount ?? 0.3,
        x: e.x ?? 50,
        y: e.y ?? 50,
        color: e.color || '#000000',
        text: e.text || ''
      }))
    }
    configLoaded.value = true
    configLoadError.value = ''
    return !hadSavedTitle
  } catch (e) {
    // 读不到 → 保持未加载（写入门闸继续关着），但**必须说出来**：不说的话用户以为改动存上了，
    // 实际上这个面板从此不再往磁盘写任何东西。
    configLoaded.value = false
    configLoadError.value = e.message || String(e)
    addClient('warn', '封面配置读取失败，改动不会被保存：' + configLoadError.value, 'cover')
    if (retrying) toast('封面配置仍然读取失败：' + configLoadError.value, 'err')
    return false
  }
}

/** 错误条上的「重试」：读成功且这次是首次拿到标题时，照样排一次（失败那轮什么都没写盘） */
async function reloadCoverConfig() {
  if (await loadCoverConfig(true)) await fitTitleIntoSafeArea()
}

onMounted(async () => {
  updateStageScale()
  syncControlsBound()
  resizeRo = new ResizeObserver(() => { updateStageScale(); syncControlsBound() })
  if (stageWrapRef.value) resizeRo.observe(stageWrapRef.value)
  videoSrc.value = mediaUrl(props.dir, 'clip.mp4')
  loadFonts()
  // 首次拿到标题（磁盘上没有 title_lines）→ 把排版调好并放进 4:3 安全区。
  // 排完的改动由下面那个 600ms 防抖 watch 自动落盘，所以只跑这一次。
  if (await loadCoverConfig()) await fitTitleIntoSafeArea()
})

onUnmounted(() => {
  if (saveConfigTimer) {
    clearTimeout(saveConfigTimer)
    saveConfigTimer = null
  }
  if (configLoaded.value) persistCoverConfig()
  if (resizeRo) {
    resizeRo.disconnect()
    resizeRo = null
  }
  // 拖到一半被卸载（切片段/关页面）时把 window 上的监听摘掉，别留着往空组件写
  if (stageDrag.value) {
    window.removeEventListener('pointermove', stageDrag.value.onMove)
    window.removeEventListener('pointerup', stageDrag.value.onUp)
    window.removeEventListener('pointercancel', stageDrag.value.onUp)
    stageDrag.value = null
  }
})
</script>

<style scoped>
/* 保存失败的常驻文字（与「保存中…」同一处）：600ms 防抖下用 toast 会刷屏，而且一闪就没了 */
.save-err { color:var(--red-deep); }
/* 自动排版放不下时的常驻提示（琥珀色 = 警告，不是错误：结果仍然可用） */
.fit-hint { color:var(--amber-deep); }
.designer-grid { display:grid; grid-template-columns:minmax(0,1fr) 340px; gap:16px; align-items:start; }
/* 右侧设置列：紧凑分组 + 双列微网格；高度被 JS 限制为与左侧预览等高，超出内部滚动 */
.controls { display:flex; flex-direction:column; gap:8px; min-width:0; overflow-y:auto; padding-right:2px; scrollbar-width:thin; }
.controls::-webkit-scrollbar { width:6px; }
.controls::-webkit-scrollbar-thumb { background:var(--line); border-radius:3px; }
.grp { border:1px solid var(--line); border-radius:10px; background:var(--card); padding:1px 10px 8px; }
.grp summary { list-style:none; display:flex; align-items:center; gap:6px; font-size:12px; font-weight:700; color:var(--ink); padding:6px 0 3px; cursor:pointer; user-select:none; }
.grp summary::-webkit-details-marker { display:none; }
.grp summary::after { content:'▾'; margin-left:auto; font-size:10px; color:var(--ink-faint); transition:transform .15s; }
.grp:not([open]) summary::after { transform:rotate(-90deg); }
/* 标题文本框：5 行固定高度由 `rows="5"` + 这里的 line-height 决定（不随内容长高），
   resize:none 去掉拖拽手柄。别再往这里加 font-size/padding 覆盖 ——
   字段外观用设计基准 .fld-input（app.css：bg-soft + 6px/10px + 12.5px）。 */
.title-lines { line-height:1.5; resize:none; }
.mini-row { display:flex; align-items:center; gap:6px; min-width:0; }
/* 分段切换（背景模式）：三个按钮等宽铺满一行，别用 .mini-field（那是一条「标签+控件」的窄行） */
.mini-label { flex:none; font-size:11px; color:var(--ink-dim); white-space:nowrap; }
.seg-btn { flex:1; justify-content:center; padding:3px 0; }
.mini-grid { display:grid; grid-template-columns:1fr 1fr; gap:4px 10px; }
.mini-field { display:flex; align-items:center; gap:5px; min-width:0; }
.mini-field > label { flex:none; margin:0; font-size:11px; color:var(--ink-dim); white-space:nowrap; }
.mini-field > input[type=range] { flex:1; min-width:0; padding:0; height:18px; background:transparent; border:none; box-shadow:none; accent-color:var(--main); cursor:pointer; }
.mini-field > select { flex:1; min-width:0; width:auto; padding:2px 6px; font-size:11.5px; border-radius:6px; }
.mini-field > input[type=color] { flex:1; min-width:0; width:auto; height:22px; padding:1px 2px; border-radius:6px; }
/* 手填 16 进制码：与取色器并排，窄列也能看清 6 位 */
.hex-input { flex:1; min-width:0; padding:2px 6px; font-size:11.5px; font-family:monospace; }
.mini-field.check { gap:4px; font-size:11.5px; color:var(--ink-dim); cursor:pointer; white-space:nowrap; }
.mini-field.check input[type=checkbox] { width:auto; margin:0; flex:none; }
.kw-input { flex:1; min-width:0; padding:4px 8px; font-size:12px; }
.grow { flex:1; min-width:0; }
.rv { flex:none; min-width:32px; text-align:right; font-size:10.5px; color:var(--ink-faint); font-family:monospace; }
.stage { position:relative; width:100%; aspect-ratio:16/9; background:#000; border-radius:14px; overflow:hidden; }
/* 背景层在最底下（纯色 / 当前帧模糊帧），帧层压在它上面 —— 帧缩小/平移时才露出背景 */
.stage-bg { position:absolute; inset:0; z-index:0; overflow:hidden; }
.stage-bg-img { display:block; width:100%; height:100%; object-fit:cover; transform-origin:center center; }
.stage-media { position:absolute; inset:0; z-index:1; overflow:hidden; }
.stage-video { position:absolute; inset:0; width:100%; height:100%; max-height:none; margin:0; object-fit:cover; display:block; }
.fx-layer { position:absolute; inset:0; pointer-events:none; z-index:4; }
.fx-item { position:absolute; inset:0; }
/* 表情特效：不是铺满的图层，而是一个点 —— 位置/字号/阴影走行内样式，居中在这。
   字体栈与出片模板（cover.rs 的 EMOJI_FONT）保持一致，别落到中文字体上。 */
.fx-item.fx-emoji { inset:auto; transform:translate(-50%,-50%); line-height:1;
  font-family:'Segoe UI Emoji','Apple Color Emoji','Noto Color Emoji',sans-serif; }
.emoji-picks { display:flex; flex-wrap:wrap; gap:4px; }
.emoji-pick { background:var(--bg-soft); border:1px solid var(--line); border-radius:6px;
  font-size:15px; line-height:1; padding:3px 6px; cursor:pointer; }
.emoji-pick:hover { border-color:var(--main); }
/* 图层顺序必须与出片模板一致（自下而上）：背景 < 帧 < 暗角 < 渐变 < 特效 < 标题。
   模板里 `#fx-extra` 是 z-index:1、暗角/渐变没有 z-index（自动）→ 特效在它们**之上**；
   预览以前三块都在 z-index:2 且 DOM 顺序把特效放最前，于是特效被渐变/暗角压暗（用户看到的
   "设计器和出片不一样"之一）。 */
.vignette-layer { position:absolute; inset:0; pointer-events:none; z-index:2; background:radial-gradient(ellipse at 55% 45%, transparent 50%, rgba(0,0,0,1) 100%); }
.gradient-layer { position:absolute; inset:0; pointer-events:none; z-index:3; }
.title-wrap { position:absolute; z-index:5; width:62%; transform:translate(-50%,-50%); text-align:center; pointer-events:none; }
/* 拖拽/滚轮层压在最上面（标题是 pointer-events:none，不挡操作） */
.stage-gesture-layer { position:absolute; inset:0; z-index:6; cursor:grab; }
.stage-gesture-layer:active { cursor:grabbing; }
/* 安全区线框：只是辅助线，不参与"设计器 ⇄ 出片镜像"那一套 —— 所以线宽与字号用**预览 px**
   （不乘 stageScale），也不做任何 JS 换算，纯 CSS 相对尺寸跟着舞台一起缩放。
   白线 + 1px 深色外圈（box-shadow 扩张）：封面素材亮暗不定，单色线总会在某一类画面上糊掉。 */
.guide-layer { position:absolute; inset:0; pointer-events:none; z-index:7; }
.guide-box { position:absolute; left:50%; top:50%; transform:translate(-50%,-50%);
  border:1px solid rgba(255,255,255,.85); box-shadow:0 0 0 1px rgba(0,0,0,.45); }
/* 16:9 安全区 = 等比缩到 95%（1920×1080 → 1824×1026）。别写成 `inset:5%` ——
   CSS 的 inset 百分比横向按父宽、纵向按父高解析，看着像等比其实不是。 */
.guide-169 { width:95%; height:95%; }
/* 4:3 裁切区 = 满高居中（1080×4/3 = 1440 宽）。宽度交给 aspect-ratio 算，不用 JS。 */
.guide-43 { height:100%; aspect-ratio:4/3; }
/* 标签放框**内侧**左上角：4:3 框是满高，框外没有空间 */
.guide-tag { position:absolute; left:4px; top:4px; font-size:10px; line-height:1.5; padding:0 5px;
  border-radius:999px; background:rgba(0,0,0,.55); color:#fff; white-space:nowrap; }
.guide-row { display:flex; align-items:center; gap:6px; }
.drag-hint { font-size:10.5px; color:var(--ink-faint); white-space:nowrap; cursor:default; }
.title-text { color:#f0e6d3; font-weight:900; line-height:1.7; paint-order:stroke fill; white-space:pre-line; overflow-wrap:anywhere; }
.title-text .hl { color:var(--hlc, #ffcc00); -webkit-text-stroke:inherit; paint-order:stroke fill; }
.seek-row { display:flex; align-items:center; gap:8px; }.seek-range { flex:1; min-width:0; padding:0; }
.seek-time { flex:none; min-width:120px; text-align:right; }
.w-time { width:90px; flex:none; }
.hidden-file { display:none; }
.preset-row { display:flex; gap:6px; }
.preset-row .mini-btn { flex:1; justify-content:center; padding:3px 0; }
.hl-chips { display:flex; flex-wrap:wrap; gap:6px; }
.hl-chip { display:inline-flex; align-items:center; gap:4px; background:var(--bg-soft); border:1px solid var(--line); border-radius:999px; padding:3px 8px; font-size:12px; color:var(--ink-dim); }
.hl-x { background:none; border:none; color:var(--ink-faint); cursor:pointer; font-size:14px; line-height:1; padding:0; }
.hl-x:hover { color:var(--red); }
.fx-card { border:1px solid var(--line); border-radius:8px; padding:6px 8px; background:var(--bg-soft); }
.fx-head { display:flex; align-items:center; justify-content:space-between; gap:6px; }
@media (max-width: 900px) {
  .designer-grid { grid-template-columns:1fr; }
  .controls { max-height:none !important; overflow:visible; }
}
</style>
