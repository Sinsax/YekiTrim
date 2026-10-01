<template>
  <div class="polish-preview">
    <div ref="stageRef" class="stage" :style="stageStyle">
      <!-- **Dhampir 实时预览**：渲染跑在**这一侧**（Dhampir 的 wasm + WebGPU），
           后端只发 wasm 资源与工程 JSON。盖在 DOM 预览之上，下面那一套一行没动 ——
           切回"DOM"时它只是隐藏，video 元素 / rAF / 贴纸 / 字幕都没被碰过。
           用 `v-show` 而不是 `v-if`：进模式的那一帧就要 attach 到 canvas 上，
           `v-if` 会让 canvas 晚一拍才存在（ref 拿到 null）。 -->
      <canvas v-show="dhampirMode" id="dhampir-canvas" ref="dhampirCanvasRef" class="dhampir-canvas"></canvas>
      <!-- 每个**视频类**素材一个 <video>，由 wasm 侧自己 seek。用 1px + 透明而不是
           `display:none`：隐藏的 video 在部分浏览器里会停止解码，而 seek 的代价正是解码。 -->
      <div v-show="dhampirMode" ref="dhampirHostRef" class="dhampir-hosts"></div>
      <video v-if="bgOn && videoSrc" ref="bgRef" class="bg-video" :src="videoSrc" muted playsinline preload="auto"></video>
      <div class="video-wrap" :class="stageClass">
        <div class="fit-box">
          <!-- **A/B 双缓冲**（DOM 预览，2026-09-30）：两块 `<video>` 同一个 `src`，一块在台上、一块当备用。
               跳手动屏蔽区间时**不再对"正在显示的那块"改 `currentTime`** —— 备用那块在进入区间前
               `PREROLL_SECS` 就把区间终点 seek 好了（那一帧已经在解码器里），到点**换元素**即可：
               没有等待、没有黑、也没有"画面停住"。
               备用那块是 `width/height:1px` + 仍在渲染树里 —— 与 `.dhampir-hosts` 同一条经验：
               `display:none` 的 video 会停止解码，1px 不会。
               `videoRef` 是 **computed**（指向台上那块），所以 `tick`/`seekTo`/`applyAudio`/定格快照
               那些读 `videoRef.value` 的地方一个字都不用改、自动跟着换。
               Dhampir 模式的画面是引擎画的（它有自己的源），换 DOM 元素救不了它 —— 那条腿仍走定格遮罩。 -->
          <video ref="stage0Ref" class="source-video" :class="{ 'source-video-standby': activeStage !== 0 }"
                 :src="videoSrc" playsinline preload="auto" :style="videoStyle"
                 @loadedmetadata="onMeta" @seeked="onVideoSeeked" @play="onStagePlay" @pause="onStagePause"
                 @ended="onStagePause"></video>
          <video ref="stage1Ref" class="source-video" :class="{ 'source-video-standby': activeStage !== 1 }"
                 :src="videoSrc" playsinline preload="metadata" :style="videoStyle" muted
                 @loadedmetadata="onMeta" @seeked="onVideoSeeked" @play="onStagePlay" @pause="onStagePause"
                 @ended="onStagePause"></video>
        </div>
      </div>
      <div class="corner-glow"><div class="gb"></div></div>
      <div class="fx fx-vignette" :style="vignetteStyle"></div>
      <div class="fx fx-noise" :style="{ opacity: noiseOp }"></div>
      <div class="fx fx-flash" :style="flashStyle"></div>
      <div class="fx fx-hue" :style="hueStyle"></div>
      <div class="fx fx-color" :style="colorOverlayStyle"></div>

      <div class="overlay-space" :style="{ transform: 'translate(-50%,-50%) scale(' + overlayScale + ')' }">
        <div class="sticker-layer">
          <div v-for="(st, i) in stickers" :key="'s' + i" class="sticker-el"
               :style="{ left: st.x + 'px', top: st.y + 'px', opacity: st.opacity, transform: 'rotate(' + st.rot + 'deg) scale(' + st.scale + ')' }">
            <img v-if="st.src" :src="st.src" alt="" @error="onStickerError(st)">
            <span v-else class="sticker-emoji">{{ st.emoji }}</span>
          </div>
        </div>
        <div class="danmaku-layer" :style="dmLayerStyle">
          <div v-for="(d, i) in dmRows" :key="'d' + i" class="dm" :style="{ left: d.x + 'px', top: d.y + 'px', opacity: d.opacity }">{{ d.text }}</div>
        </div>
        <div class="captions" :style="capLayerStyle">
          <div v-for="(c, i) in capRowsWrapped" :key="'c' + i" class="cap"
               :style="{ opacity: c.opacity, transform: 'translateY(' + c.yOff + 'px) scale(' + c.scale + ')',
                         fontSize: (subStyle.size || 72) * c.fontScale + 'px',
                         WebkitTextStrokeWidth: capStrokeW * c.fontScale + 'px' }">
            <span v-for="(line, li) in c.lines" :key="li" class="cap-line">
              <span v-for="(p, j) in line" :key="j" :class="{ hl: p.hl }">{{ p.text }}</span>
            </span>
          </div>
        </div>
      </div>
      <!-- 跳手动屏蔽区间时的**定格遮罩**：跳的办法是给 video 改 `currentTime`，而 seek 期间
           解码器没有可呈现的帧 —— 元素底下 `app.css` 全局 `video{background:#000}` 的黑就露出来了
           （用户报的「到屏蔽区黑屏一下」），"极短的卡顿"是同一件事：画面在等解码。
           这里盖一张 seek **之前**拍的画面快照，等新帧真的上屏再撤 —— 观感从"黑一下"变成一次硬切。
           z-index 由模式决定（见 `freezeZ`）：DOM 模式只冻**视频像素**（让上层的字幕/弹幕/贴纸照常
           按新时刻画，那正是剪辑点的样子），Dhampir 模式的画面是那块 canvas（连字一起画），
           所以必须盖在它上面。 -->
      <canvas v-show="freezeOn" ref="freezeRef" class="freeze-frame" :style="{ zIndex: freezeZ }"></canvas>
      <!-- 框选布局：在此模式下视频运镜暂停（缩放复位），直接在整帧上拖出精确矩形 -->
      <div v-if="selectMode" class="sel-layer" @pointerdown.prevent="startSel('new', $event)">
        <div class="sel-box" :style="selBoxStyle" @pointerdown.prevent.stop="startSel('move', $event)">
          <span class="sel-tag">{{ selectLabel || '框选' }}</span>
          <span v-for="h in SEL_HANDLES" :key="h" class="sel-h" :class="'h-' + h"
                @pointerdown.prevent.stop="startSel(h, $event)"></span>
        </div>
      </div>
      <!-- 点选落点（贴纸「固定」模式）：与框选同一套交互，只是落一个点 —— 点哪儿落哪儿 -->
      <div v-else-if="pointMode" class="point-layer" @pointerdown.prevent="pickPoint($event)">
        <span class="point-tag">{{ pointLabel || '点选落点' }}</span>
        <!-- 画出**贴纸真实占位**（280×短边倍率的盒子，**以锚点为中心**，并做出片同样的出框 clamp），
             不是画一个点 —— 固定落点的含义是"贴纸中心落在这一点"（config.rs 的 position 口径） -->
        <span class="point-box" :style="pointBoxStyle"><span class="point-dot"></span></span>
      </div>
    </div>

    <div v-if="dhampirMode || dhampirError" class="dhampir-bar">
      <span class="muted small">
        {{ dhampirError ? ('Dhampir 预览不可用：' + dhampirError) : (dhampirStatus || 'Dhampir 预览') }}
        <!-- ⚠️ 兜底（DOM/GSAP）预览是**近似**：它是第二条渲染链路，默认出片走的是底座那条。
             这一行就是"看到的和成片可能不一样"的告知 —— 收敛之后新效果只在底座那条腿上实现。 -->
        <span v-if="dhampirError" class="err">（当前是近似预览）</span>
      </span>
      <span v-if="dhampirMode" class="muted small mono">
        {{ dhampirMeasuredFps }}/{{ dhampirFps }}fps · {{ dhampirLatencyMs }}ms · 丢 {{ dhampirDroppedShown }}
        <button class="mini-btn" style="padding:0 4px" @click="dhampirDetailOpen = !dhampirDetailOpen"
                :title="dhampirDetailOpen ? '收起进阶读数' : '展开进阶读数（debug）'">{{ dhampirDetailOpen ? '▾' : '▸' }}</button>
      </span>
      <span v-if="dhampirMode && dhampirDetailOpen" class="muted small mono" style="display:block">
        #{{ dhampirFrame }} · prep {{ dhampirPrepMs }}ms / draw {{ dhampirDrawMs }}ms · {{ dhampirAdvanced }}
        <template v-if="dhampirSlowSeeks">· 慢 seek {{ dhampirSlowSeeks }}</template>
        <template v-if="dhampirSourceMode">· {{ dhampirSourceMode }}</template>
        <template v-if="dhampirStickerInfo">· {{ dhampirStickerInfo }}</template>
      </span>
    </div>
    <div class="preview-controls">
      <button class="mini-btn" @click="togglePlay">
        <Icon :name="playing ? 'pause' : 'play'" :size="12" /> {{ playing ? '暂停' : '播放' }}
      </button>
      <input class="flex-1" type="range" :min="0" :max="1000" :value="progress" @input="onSeekInput">
      <span class="muted small mono time-label">{{ fmt(current) }} / {{ fmt(duration) }}</span>
      <button class="mini-btn" :class="{ selected: silenced }" @click="toggleMute" :title="silenced ? '取消静音' : '静音'">
        <Icon :name="silenced ? 'volume-x' : 'volume'" :size="13" />
      </button>
      <input class="vol" type="range" min="0" max="1" step="0.05" :value="volume" @input="onVolume" title="音量（视频 + 音效）">
      <button class="mini-btn" :class="{ selected: dhampirMode }" :disabled="!!dhampirError" @click="toggleDhampir"
              :title="dhampirError
                ? ('Dhampir 预览不可用：' + dhampirError)
                : '用 Dhampir 底座（wasm + WebGPU）渲染 —— 与上面的 DOM 预览并列，不替代它'">
        Dhampir
      </button>
      <button class="mini-btn" @click="fullscreen" title="全屏"><Icon name="maximize" :size="13" /></button>
    </div>
  </div>
</template>

<script setup>
// 瘦播放器：数据由父级（PolishView）一次性下发，特效状态全部来自 polish/engine.js
// （与 templates/index.html Canvas 出片路径一致），rAF 只读 video.currentTime 作为主时钟。
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
import Icon from './Icon.vue'
import { toast } from '../stores/toast'
import { addClient } from '../stores/log'
import { clipMediaUrl, mediaUrl } from '../utils'
import { api } from '../api/client'
import { buildIndex, videoFxAt, handheldSway, getFlash, getHueShiftFlash, getNoiseOpacity, getVignette, getOverlayState, gradientCss, subsAt, danmakuAt, stickersAt, wrapCaptionParts, capMeasure } from '../polish/engine'
import { canvasOf, stickerScale } from '../polish/canvas.js'
import { createSfxPlayer } from '../polish/audio'
import { ensureFontFace } from '../polish/fonts'

const props = defineProps({
  pid: { type: [String, Number], required: true },
  dir: { type: String, required: true },
  config: { type: Object, default: null },
  subtitles: { type: Array, default: () => [] },
  danmaku: { type: Array, default: () => [] },
  metaDuration: { type: Number, default: 0 },
  // 框选布局模式：叠加选框 + 暂停运镜（缩放复位），选框坐标为整帧百分比
  selectMode: { type: Boolean, default: false },
  selectRect: { type: Object, default: null },
  selectLabel: { type: String, default: '' },
  // 点选落点模式（贴纸「固定」）：点哪儿落哪儿，坐标为整帧百分比。
  // 与 selectMode 互斥（框选优先）—— 两者都是"在预览上取坐标"，没必要同时开。
  pointMode: { type: Boolean, default: false },
  point: { type: Object, default: null },
  pointLabel: { type: String, default: '' },
  // 动态兜底字体栈（页面侧从 `/api/polish/fonts` 的 `fallback_stack` 透传进来）：
  // 首项随字体库里"设为默认"的那支走 —— 与成片（`server.rs::fallback_stack`）同一份计算。
  // 缺省 = 老的字面量栈（库里没有可用中文时服务端算出来的回落态就是它，不回归）。
  fontFallback: { type: String, default: '"LXGW WenKai","Microsoft YaHei","Noto Sans SC",sans-serif' }
})
const emit = defineEmits(['time', 'update:selectRect', 'update:point', 'playing'])

// ===================== Dhampir 浏览器端预览（实时） =====================
//
// **渲染跑在这一侧**（Dhampir 的 wasm + WebGPU），后端只发两样东西：wasm 资源 +
// 转译好的工程 JSON。像素不过边界 —— 所以没有"传输"这个瓶颈。
// 实测口径（1920×1080、无头 Chrome、BrowserWebGpu）：顺序播放 wall 中位 **7.0 ms/帧**
// （seek 0.7 / draw 6.2）→ 142.9 fps；原地重画 6.9 ms。
//
// 与上面那套 DOM/CSS 复刻**并列、不替代**：同一个工程、**同一个时钟**
// （都读 `videoRef.currentTime`），所以"两边对不上"本身就是有用的信息。
const dhampirMode = ref(false)
const dhampirStatus = ref('')
const dhampirError = ref('')
const dhampirFrame = ref(0)
// **上面那个 ref 是给模板看的**（`#{{ dhampirFrame }}`）⇒ 每写一次就重渲染整棵模板 ✗。
// 所以"这一帧画过没有"的判断改用这个**非响应式**变量 —— 它每帧更新也不触发渲染 ✓。
// 两个角色必须分开：以前共用一个 ref，后果是**每帧一次整模板重渲染**（几十毫秒级）。
let dhampirFrameShown = -1
const dhampirFps = ref(30)
const dhampirCanvasRef = ref(null)
const dhampirHostRef = ref(null)
/** **实测**帧率：不是猜的，也不是适配器型号 —— 浏览器侧本来问不出型号
    （Dhampir 的 `preview.rs` 里写着：`GPUAdapterInfo` 的 name/description 是空串）。
    实时预览该不该用，唯一诚实的判据就是"它现在跑多少帧"。 */
const dhampirMeasuredFps = ref(0)
const dhampirPrepMs = ref(0)
const dhampirBreakdown = ref('')
const dhampirDrawMs = ref(0)
const dhampirSlowSeeks = ref(0)
// **工具条主行三组数字**（用户要求：实际fps/60fps · 总帧延迟 · 丢失帧）✓
const dhampirLatencyMs = ref(0)      // 总帧延迟 = "决定画这一帧"到"画完"的墙钟
const dhampirDroppedShown = ref(0)   // 每秒归零显示的丢失帧
const dhampirDropped = ref(0)        // 本秒累计
let dhampirLastFrameDrawn = -1
// **折叠区**：默认收起，展开后是完整进阶文本（debug 用）✓
const dhampirDetailOpen = ref(false)
const dhampirAdvanced = ref('')
const dhampirSourceMode = ref('')
// 贴纸那条路的读数（见 dhampirStickerStat 的说明）—— 4Hz 刷，与其它读数同一处。
const dhampirStickerInfo = ref('')

let dhampirEngine = null
let dhampirRaf = 0
let dhampirBusy = false // 同一时刻只让一帧在飞：`seek` 是异步的，堆起来只会越拖越远
/**
 * **三档纠偏**（A 案）的阈值，单位**秒**。
 *
 * 为什么必须有这一档：引擎的「跟随播放」只要求源 `<video>` **自己在走**
 * （`followPlayback && paused !== true`），它**不比绝对时刻** —— 于是偏差会在
 * 「比一次就 seek」与「永不 seek」之间二选一，而两条都错：
 *   * 每帧硬 seek ⇒ 实测 **307ms/帧**（GOP 长的素材从关键帧重解），预览 ~3fps ✗
 *   * 永不 seek  ⇒ 拖动/切片段之后画面停在旧时刻，时间轴与画面对不上 ✗
 * 所以**偏差分三档**：小偏差不碰（让它自然播）、中偏差只改速率（不 seek）、
 * 大偏差（或用户拖动/切片段）才 hard seek。
 */
const DHAMPIR_ALIGN_QUIET = 0.04   // ① 40ms 以内：什么都不做
const DHAMPIR_ALIGN_HARD = 0.5     // ③ 500ms 以上：才 hard seek
/**
 * 我们期望的**底座 wasm API 版本**（底座的"形状变更"定义：返回体字段增减 ⇒ 这个数 +1）。
 *
 * 同一个数在底座仓的 `crates/dhampir-timeline/src/host_api.rs::HOST_API_VERSION`；
 * `scripts/check-dhampir-wiring.mjs` 会**跨仓对拍**这两个值 —— **底座升版而我们没跟 ⇒ 门禁红**，
 * 免得"字段少了"表现成"预览里某些效果不生效"（没堆栈、只能猜）。
 * 当前 **5** = 字幕阴影四键（`shadow_color` / `shadow_dx_px` / `shadow_dy_px` / `shadow_blur_ratio`）那次。
 */
const DHAMPIR_HOST_API = 5
const DHAMPIR_ALIGN_RATE_LO = 0.98 // ② 超前 → 放慢一点
const DHAMPIR_ALIGN_RATE_HI = 1.02 // ② 落后 → 追一点
/** `source` -> `<video>`：与 `dhampirEngine.bindSource` 同一批元素（纠偏要按 source 找元素）。 */
let dhampirSourceVideos = new Map()
/**
 * **备用那套源**（A/B，跳手动屏蔽区间用）：`assetId -> { el, id, want, ready }`。
 *
 * 为什么要它：跟播那条路上引擎**不 await seek**（`following === true` 时它直接取
 * `<video>` 正在显示的那一帧）—— 于是源还在 seeking 时合成出来的就是**整块黑**，
 * 而这块 WebGPU 画布**读不回来**（surface 只有 RENDER_ATTACHMENT，Dhampir 自己在
 * `timeline_host.rs` 里写着），定格遮罩的 `drawImage(canvas)` 拿不到像素 ⇒ 遮罩是透明的
 * ⇒ 用户看到"整块纯黑"。**唯一干净的办法就是别让引擎在源没就绪时画**：进入屏蔽区前
 * 把"下一段起点"在备用这套源上先定位好，到点 `bindSource` 换绑 —— 那一帧早就在解码器里了。
 */
let dhampirTwins = new Map()
/** "源没就绪 → 先别画"这条路已经等了多久（毫秒）。0 = 没在等。 */
let dhampirHoldSince = 0
/** 等源的上限：超了就照画（宁可露一帧黑，也不能把预览焊死）。与定格遮罩同一个量级。 */
const DHAMPIR_SOURCE_WAIT_MS = 700
/** 三档各命中多少次（显示用，与丢失帧一样每秒归零）。 */
const dhampirAlignStat = { quiet: 0, rate: 0, hard: 0 }
const dhampirAlignShown = ref('')
let dhampirBlobUrls = [] // 字幕/弹幕的 blob URL，退出时要逐个释放
let dhampirVideos = []
let dhampirStickers = []
  dhampirVideos = []
  dhampirStickers = []
let dhampirFpsWindow = { t0: 0, frames: 0 }
let dhampirReadoutAt = 0

/**
 * **工程的"段落表"**：从 `v1` 的层读出来。每层 = 一个"保留区间"。
 *
 * 为什么需要它：转译器把 `mute_ranges` 那些段**从时间线上掐掉**（不是静音）——
 * 主画面因此从"一整层"变成"每个保留区间一层"，每层写着 `source.source_in`
 * （这一层的工程起点对应哪个**源帧**）。所以工程时间轴是**压缩过的**，
 * 而源视频的时间轴不是：`源秒 × 时基` **不等于**工程帧号。
 *
 * 实测：`clip-04`（mute_ranges=1）的层是 `0..8560`、`source_in=204` ——
 * 直接乘时基会让整条预览偏 204 帧（3.4 秒），而且越过 8560 之后越界。
 */
let dhampirSegments = []
// 贴纸诊断只报一次（每次进 Dhampir 模式重置）。这条日志是为了**不再猜**。
let dhampirStickerDiag = null
/**
 * **贴纸那条路的状态**，显示在工具条上。
 *
 * 为什么放到 UI 而不是 console：这条路我改了六版，每一版都是"看代码推断"，
 * 而 console 的日志一直没被贴回来。工具条是每轮都会被读到的地方 ——
 * 它上面写着 `就绪 n/m · 本帧 idx · 已交 k`，一眼就能定位断点：
 *   * `就绪 0/4` → `ImageDecoder` 没建起来（或取字节失败）
 *   * `就绪 4/4` 且 `idx` 在变、`已交` 在涨 → 解码与提交都对，问题在引擎那边
 *   * `已交 0` → `onPrepared` 没被调到
 */
const dhampirStickerStat = { total: 0, ready: 0, idx: -1, submits: 0, decodes: 0 }

function dhampirBuildSegments(project) {
  const track = ((project.timeline && project.timeline.tracks) || []).find((t) => t.id === 'v1')
  dhampirSegments = (((track && track.layers) || []))
    .filter((l) => l.enabled !== false)
    .map((l) => ({
      frameStart: Number(l.start) || 0,
      frameEnd: Number(l.end) || 0,
      sourceIn: Number((l.source && l.source.source_in) || 0),
    }))
    .sort((a, b) => a.frameStart - b.frameStart)
}

/**
 * 秒（**源**时间）-> 工程帧号。用**工程自己的时基**（`timeline.timebase`）+ 段落表。
 *
 * 落在**被掐掉的源区间**里时返回 `null` —— 那一段在工程里不存在，
 * 不是"要跳过去"，是"没有这一帧"（调用方据此把源挪到下一段起点）。
 * 没有段落表（单段且 source_in=0）时退化成直接乘时基，与以前一致。
 */
function dhampirFrameAt(seconds) {
  const src = Math.max(0, Math.round((seconds || 0) * dhampirFps.value))
  if (!dhampirSegments.length) return src
  for (const seg of dhampirSegments) {
    const len = seg.frameEnd - seg.frameStart
    if (src >= seg.sourceIn && src < seg.sourceIn + len) {
      return seg.frameStart + (src - seg.sourceIn)
    }
  }
  return null
}

/**
 * 当前源时间落在被掐掉的区间里时，**下一段保留区间的源起点**（秒）。
 *
 * 这就是"跳过屏蔽区间"在 Dhampir 模式下的正确形态：不是按 `polish.toml` 的
 * `mute_ranges` 去跳（那是**源**时间上的说法），而是按**工程自己的段落表**去跳 ——
 * 两者在压缩工程上不是一回事，按前者跳会**跳两次**。
 */
function dhampirNextSourceStart(seconds) {
  const src = Math.max(0, Math.round((seconds || 0) * dhampirFps.value))
  const next = dhampirSegments
    .map((s) => s.sourceIn)
    .filter((s) => s > src)
    .sort((a, b) => a - b)[0]
  return next === undefined ? null : next / dhampirFps.value
}

// ---- Dhampir 那条腿的 A/B 预滚（2026-09-30）----
//
// 见 `dhampirTwins` 上面那段：跟播时引擎不 await seek，源在 seeking 时合出来就是整块黑，
// 而这块画布读不回来（没有 COPY_SRC），定格遮罩救不了它。所以：
//   · 进屏蔽区前 `PREROLL_SECS`，把"下一段起点"在**备用那套源**上 seek 好（它们是暂停的、
//     1px、仍在渲染树里 ⇒ 照旧解码）；
//   · 到点 `bindSource` 换绑 + 让它们播起来 ⇒ 引擎那一帧立刻有像素，**零等待、零黑**；
//   · 没备好（刚拖过进度条 / 区间太近 / 源挂了）才回落"定格遮罩 + 源就绪前不画"。

/**
 * 预滚提前量（秒）：进入屏蔽区之前这么久，就把"区间终点 / 下一段起点"在**备用**那一路准备好。
 * 实测 seek 340~707ms —— 0.8s 留足余量，又不至于太早（长片远跳会一直占着解码器）。
 * DOM 那条腿的 A/B 与 Dhampir 这条腿的 A/B **共用**这一个值。
 */
const PREROLL_SECS = 0.8

/**
 * 该不该现在预滚？返回"下一段保留区间的源起点（秒）"，不用预滚时 `null`。
 *
 * 判据是**往前看**：源时间 `t + PREROLL_SECS` 已经落进被掐掉的区间 = 马上就要切。
 * 用段落表算，而不是拿 `mute_ranges` 直接比 —— 后者是**源**时间上的说法，在压缩工程上会算错。
 */
function dhampirPrerollTarget(t) {
  if (!dhampirSegments.length || !dhampirTwins.size) return null
  if (dhampirFrameAt(t) === null) return null                      // 已经在被掐掉的区间里
  if (dhampirFrameAt(t + PREROLL_SECS) !== null) return null       // 还没到"马上要切"
  return dhampirNextSourceStart(t)
}

/** 备用那块 seek 到我们要的位置了就标记就绪（`want` 变了要重新等一次）。 */
function dhampirWatchTwin(assetId, el) {
  el.addEventListener('seeked', () => {
    const tw = dhampirTwins.get(assetId)
    if (!tw || tw.el !== el) return
    if (el.readyState >= 2 && Math.abs((el.currentTime || 0) - tw.want) < 0.08) tw.ready = true
  })
}

/**
 * 这一趟预滚**真正武装了哪些源**（只对它们做就绪检查与换绑）。
 *
 * 为什么要有它：工程里可能有**这一刻不活跃**的源（画中画的第二路素材），它们没有"这一帧该停在哪"，
 * 把它们也算进"全都就绪"的话，换绑永远等不齐 —— 症状是 A/B 静默失效、又退回那一帧黑。
 */
let dhampirArmedIds = []

/** 把下一段起点交给备用那套源去定位。同一目标只做一次（每帧重设会把解码器一直打断）。 */
function dhampirArmPreroll(targetSec) {
  const targetFrame = dhampirFrameAt(targetSec)
  if (targetFrame === null) return false
  let list = []
  try { list = dhampirEngine.sourcesFor(targetFrame) || [] } catch (e) { return false }
  const ids = []
  for (const e of list) {
    const id = e && e.source
    const tw = dhampirTwins.get(id)
    if (!tw || !tw.el) continue
    const sec = typeof e.seconds === 'number' ? e.seconds : e.t
    if (typeof sec !== 'number') continue
    ids.push(id)
    if (tw.want === sec) continue                                  // 已经在滚/已就绪
    tw.want = sec
    tw.ready = false
    try { tw.el.currentTime = sec } catch (err) { /* 元数据没到就下一帧再来 */ }
    if (!tw.el.paused) tw.el.pause()
  }
  if (ids.length) dhampirArmedIds = ids
  return ids.length > 0
}

/** 备用那套源全都就绪了吗（有一个没到位就整组不换 —— 半换会留下"一半新一半旧"的源表）。
 *  ⚠️ 判据不能只看 `ready` 标记：目标位置与它当时的位置几乎重合时，浏览器可能**不发 `seeked`**，
 *  而那一帧本来就在 —— 所以"没在 seek + 有可呈现的帧 + 位置对得上"同样算就绪。 */
function dhampirTwinsReady() {
  if (!dhampirArmedIds.length) return false
  for (const id of dhampirArmedIds) {
    const tw = dhampirTwins.get(id)
    if (!tw || !tw.el) return false
    if (tw.el.seeking || tw.el.readyState < 2) return false
    if (!tw.ready && Math.abs((tw.el.currentTime || 0) - tw.want) > 0.08) return false
  }
  return true
}

/**
 * **换绑**：把引擎的源指到备用那套（它们已经在下一段起点、帧也解好了）。
 * 换完旧的成为下一次的备用（角色互换），调用方负责把时钟推到同一点。
 */
function dhampirSwapSources() {
  if (!dhampirEngine || typeof dhampirEngine.bindSource !== 'function') return false
  if (!dhampirArmedIds.length) return false
  if (!dhampirTwinsReady()) return false
  // 只换**这一趟武装过**的源（不活跃的源没有目标位置，换过去反而会让它停在一个错的时刻）
  for (const assetId of dhampirArmedIds) {
    if (!dhampirSourceVideos.has(assetId) || !dhampirTwins.get(assetId)) return false
  }
  for (const assetId of dhampirArmedIds) {
    const tw = dhampirTwins.get(assetId)
    const old = dhampirSourceVideos.get(assetId)
    dhampirEngine.bindSource(assetId, tw.id)
    dhampirSourceVideos.set(assetId, tw.el)
    const i = dhampirVideos.indexOf(old)
    if (i >= 0) dhampirVideos[i] = tw.el
    else dhampirVideos.push(tw.el)
    try { tw.el.play() } catch (e) { /* 忽略：下一帧镜像循环会再试 */ }
    if (old) {
      old.pause()
      dhampirWatchTwin(assetId, old)                               // 旧的那块接下一段的预滚
    }
    dhampirTwins.set(assetId, { el: old, id: old ? old.id : '', want: 0, ready: false })
  }
  dhampirArmedIds = []
  return true
}

/**
 * 这一帧要用的源**都有像素**吗。
 *
 * 跟播时引擎不会替我们等（它直接取"正在显示的那一帧"），源在 `seeking` / `readyState < 2`
 * 时合成出来就是黑的 —— 所以由**宿主**在 `engine.seek` 之前挡一道：没就绪就这一帧不画，
 * 画布会**保留上一帧**（WebGPU 交换链的既有行为）。`DHAMPIR_SOURCE_WAIT_MS` 是上限。
 */
function dhampirSourcesReady(f) {
  if (!dhampirEngine) return true
  let list = []
  try { list = dhampirEngine.sourcesFor(f) || [] } catch (e) { return true }
  for (const e of list) {
    const v = dhampirSourceVideos.get(e && e.source)
    if (!v) continue
    if (v.seeking || v.readyState < 2) return false
  }
  return true
}

/** 进/出 Dhampir 模式。**按钮切换保留**：出去时把引擎停掉、回到 DOM 那一套。 */
async function toggleDhampir() {
  if (dhampirMode.value) {
    dhampirMode.value = false
    stopDhampir()
    return
  }
  dhampirError.value = ''
  // 两个都在播会同时出声，也看不清在比哪一个。
  const el = videoRef.value
  if (el) el.pause()
  dhampirMode.value = true
  dhampirStatus.value = '正在加载 Dhampir…'
  try {
    await nextTick() // canvas 要先真的挂上（attach 收的是 canvas id）
    await startDhampir()
    dhampirStatus.value = ''
    dhampirFrame.value = dhampirFrameAt(el ? el.currentTime : current.value) ?? 0
    scheduleDhampir()
  } catch (e) {
    // **绝不白屏**：说清原因、退回 DOM、把按钮禁掉（`dhampirError` 有值时按钮 disabled）。
    dhampirError.value = String((e && e.message) || e)
    dhampirStatus.value = ''
    dhampirMode.value = false
    stopDhampir()
  }
}

/** 起引擎：工程 JSON -> 每个视频素材一个 <video> -> loadEngine -> open/attach/resize/bind */
async function startDhampir() {
  if (dhampirEngine) return
  if (!navigator.gpu) throw new Error('这个浏览器没有 WebGPU（Dhampir 实时预览需要它）')
  const host = dhampirHostRef.value
  const canvas = dhampirCanvasRef.value
  if (!host || !canvas) throw new Error('画布还没准备好')

  // ① 工程 JSON（后端跑转译器）
  const res = await api('/api/polish/dhampir-project', {
    method: 'POST',
    label: 'Dhampir 工程',
    body: JSON.stringify({ dir: props.dir }),
  })
  const project = res.project
  if (!project || !project.timeline) throw new Error('后端没有返回可用的工程')
  const tb = project.timeline.timebase
  if (tb && tb.num > 0 && tb.den > 0) dhampirFps.value = tb.num / tb.den
  // 段落表要和时基一起先算好：**帧号映射全靠它**（见 dhampirFrameAt 的说明）。
  dhampirBuildSegments(project)
  dhampirStickerDiag = null
  Object.assign(dhampirStickerStat, { total: 0, ready: 0, idx: -1, submits: 0, decodes: 0 })
  const hints = project.render_hints || {}
  if (hints.width > 0 && hints.height > 0) {
    // ① **按窗口尺寸渲染**（只作用于预览 UI；随时可切回满分辨率）。
    //
    // 目标尺寸是引擎里**一切尺寸的源头**：合成面积、宿主栅格化（文字/贴纸位图）、
    // 上传纹理都随它变小 —— 所以这一处能同时吃掉 draw 与上传那两段。
    //
    // **只缩不放**（scale ≤ 1），且**保持工程宽高比**：目标尺寸一改，wasm 会按新尺寸
    // 重新排版；宽高比变了画面框架就跟着变 —— 那不是"变快"，那是"画错了"。
    //
    // 切回满分辨率：URL 加 `?nofit=1`（与引擎的 `?src=` 同一套口径），
    // 或运行时设 `window.__dhampirFit = false`。
    // 读数：canvas 元素自身的 width/height 就是最终目标尺寸（驱动一直在打这个字段）。
    const full = { w: hints.width, h: hints.height }
    let w = full.w
    let h = full.h
    // ⚠️ **默认关掉**（= 走满分辨率）。实测它**会把画面裁切/放大** ✗：
        //    同一帧、同窗口、只切这个开关 ⇒ 两次画布 PNG 差 **61.82%**，
        //    而**字幕层完全一致** ✓ ⇒ 文字是按目标尺寸等比重排的 ✓，**视频层不是** ✗
        //    （并排图见 out/cmp-fit-nofit.png；结论写进 docs/vtrim-integration.md §11）
        // **取景正确 > 帧率** ⇒ 默认满分辨率；想再试就地开：地址加 `?fit=1`，
        //    或运行时 `window.__dhampirFit = true`。
        const forceFull = new URLSearchParams(location.search).get('fit') !== '1'
          && window.__dhampirFit !== true
    if (!forceFull) {
      const box = canvas.getBoundingClientRect()
      const dpr = window.devicePixelRatio || 1
      if (box.width > 0 && box.height > 0) {
        const scale = Math.min((box.width * dpr) / full.w, (box.height * dpr) / full.h, 1)
        if (scale < 1) {
          // 下限兜住一个能看的尺寸：极小窗口下压成几十像素的话，字和贴纸都没法看了。
          w = Math.max(320, Math.round(full.w * scale))
          h = Math.max(180, Math.round(full.h * scale))
        }
      }
    }
    canvas.width = w
    canvas.height = h
  }

  // ② 每个**视频类**素材一个 <video>。贴纸（image_sequence）与音频走别的路，
  //    是下一步的事 —— 这一步只接视频层，先让画面动起来。
  const videos = (project.assets || []).filter((a) => a.kind === 'video' && a.uri)
  // **贴纸 / 图片序列**：这些素材**没有 <video>**，引擎不会替它们生成位图
  // （它的 prepare 只处理 this.videos 里的元素）—— 契约要求**宿主自己交**：
  // `dhampir_project_set_bitmap(source, bitmap)`。不交的症状是**静默少画几层**。
  const stickerImages = (project.assets || [])
    .filter((a) => (a.kind === 'image_sequence' || a.kind === 'image') && a.uri)
  host.innerHTML = ''
  const bindings = []
  videos.forEach((a, i) => {
    const v = document.createElement('video')
    v.id = 'dhampir-src-' + i
    v.muted = true
    v.playsInline = true
    v.preload = 'auto'
    // ⚠️ **不能用 `mediaUrl`**：转译出来的 `asset.uri` 是相对**片段目录**的
    // （Dhampir 用 `--asset-root <片段目录>` 解析），而 `/api/media/<pid>/<rel>`
    // 是相对**工程根**解析的 —— 直接拼会 404：视频没数据 → `createImageBitmap`
    // 报 "does not have enough data" → **画面全黑**，而字幕/弹幕照样画得出来
    // （它们不需要视频），于是看起来像"只有字"。
    // `clipMediaUrl` 就是为这条约定写的（把 dir 归一成片段名 + 拼 `output/clips/…`）。
    v.src = clipMediaUrl(props.pid, props.dir, a.uri)
    host.appendChild(v)
    bindings.push([a.id, v.id, v])
  })
  // 等能解码；**某一个源挂了不拖死整块**（限时后照常往下走，画面缺那一层而已）
  await Promise.all(
    bindings.map(([, , v]) =>
      new Promise((done) => {
        const fin = () => done()
        v.addEventListener('loadeddata', fin, { once: true })
        v.addEventListener('error', fin, { once: true })
        setTimeout(fin, 3000)
      }),
    ),
  )
  bindings.forEach(([, , v]) => v.pause())

  // ③ 引擎。`engine.js` 是自包含的（没有 import），pkg 的 URL 由这里给。
  // ⚠️ **URL 必须先放进变量**：写成字面量时，即使带 `/* @vite-ignore */`，
  // Rollup 仍会把它当模块去解析并报 `handleInvalidResolvedId`（构建直接失败）。
  // 放进变量，打包器就无法静态分析，才会原样留给运行期。
  const ENGINE_URL = '/api/polish/dhampir-wasm/engine.js'
  const PKG_URL = '/api/polish/dhampir-wasm/pkg/dhampir_wasm.js'
  // **带重试地载入引擎模块** ✓
  // 为什么：实测用户那边这一步会失败，而且**请求根本没到服务端**
  // （服务端日志里连一条都没有 ✗）—— 典型的"连接被瞬时占住/被拦"✓。
  // 旧代码只报"模组载入失败"，看不出原因 ✗。现在：重试 3 次（间隔递增）+ 把真实原因报出来 ✓。
  const loadEngineWithRetry = async (url, attempts = 3) => {
    let last = null
    for (let i = 0; i < attempts; i += 1) {
      try {
        return await import(/* @vite-ignore */ url)
      } catch (e) {
        last = e
        console.error("[dhampir] engine.js 载入失败（第 " + (i + 1) + "/" + attempts + " 次）：" + ((e && e.message) || e), e)
        if (i < attempts - 1) await new Promise((d) => setTimeout(d, 300 * (i + 1)))
      }
    }
    const why = String((last && last.message) || last)
    throw new Error("载入 Dhampir 引擎失败（" + attempts + " 次都失败）：" + why
      + " —— 请把这条原文连同控制台一起反馈（服务端日志里没有这条请求就说明是浏览器侧拦的）")
  }
  const { loadEngine } = await loadEngineWithRetry(ENGINE_URL)
  dhampirEngine = await loadEngine(PKG_URL)
  // ---------------------------------------------------------------------------
  // **形状断言：底座 wasm API 版本**（底座回执 2026-10-01 点名要我们加的）
  //
  // 底座把"返回体字段增减"定义成形状变更 ⇒ `HOST_API_VERSION` +1（现在 **5**：字幕阴影那四键
  // 就是这次加的）。版本对不上时**当场说清**，别让"字段少了/多了"表现成
  // "预览里某些效果不生效"（那种症状没有堆栈、只能靠猜）。
  // 数字的来源：底座 `crates/dhampir-timeline/src/host_api.rs::HOST_API_VERSION`；
  // `scripts/check-dhampir-wiring.mjs` 会**跨仓**把这两个数钉在一起（它变了我们这门禁就红）。
  //
  // ⚠️ **不能问 `dhampirEngine`**：底座的 `loadEngine(pkgUrl)` 返回的是 **`new Engine(mod)` 包装对象**，
  // 不是 wasm 模块命名空间 ⇒ 那样写永远报"没有这个导出"，**预览会被我们自己的断言挡掉**
  // （用户看到的就是「当前是近似预览」）。正确做法：自己 import 同一个 URL 拿命名空间
  // （ESM 缓存 ⇒ 与 `loadEngine` 内部同一份、不重复下载；它已经 `await mod.default()` 初始化过了 ⇒ 安全）。
  // ---------------------------------------------------------------------------
  const pkgNs = await import(/* @vite-ignore */ PKG_URL)
  const hostApi =
    typeof pkgNs.dhampir_host_api_version === 'function' ? pkgNs.dhampir_host_api_version() : null
  if (hostApi !== DHAMPIR_HOST_API) {
    throw new Error(
      "底座 wasm API 版本不匹配：期望 " + DHAMPIR_HOST_API + "，实得 " +
        (hostApi === null ? "(没有 dhampir_host_api_version 这个导出)" : hostApi) +
        " —— 这是**形状变更**（字段增减），请换一个匹配的底座产物（`vtrim polish doctor` 会打印当前用的那份）",
    )
  }
  // **打开底座的细粒度计时**：`prepareMs` 含「seek 视频 + 文字栅格化」，
  // `drawMs` 是 GPU 提交。底座说得很对 ——「只知道一帧要 200ms，该改哪里只能靠猜」。
  dhampirEngine.timing = true
  // ⚠️ **不要再 JSON.parse 一次**：`Engine.open()` 内部已经
  // `JSON.parse(mod.dhampir_project_open(json))` 了，返回的是**对象**。
  // 再 parse 会把它转成 "[object Object]" —— 浏览器报的正是
  // "unexpected character at line 1 column 2"（那个 `o` 被当成了数组元素）。
  // 另外 `open()` 收字符串或对象都行，所以这里直接给对象、不必 stringify。
  const opened = dhampirEngine.open(project)
  if (opened.ok !== true) {
    throw new Error('工程没通过 Dhampir 校验：' + JSON.stringify(opened.issues || []).slice(0, 200))
  }
  await dhampirEngine.attach(canvas.id)
  // 源模式**交给引擎的探针** —— 实测引擎判 bitmap 是对的：
  // WebGPU 的 copyExternalImageToTexture **不接受 <video>**（只认
  // ImageBitmap / HTMLImageElement / HTMLCanvasElement / OffscreenCanvas），
  // 所以 `video` 那条零拷贝路在浏览器里走不通，硬指定只会 panic 一次。
  dhampirSourceMode.value = dhampirEngine.sourceMode || ''
  
  // **预览用「跟随播放」**：视频自己在走时不逐帧 seek，直接取它正在显示的帧。
  //
  // 为什么非开不可：源素材 GOP 可能 > 4 秒（实测一份 1080p60 素材前 250 帧里
  // 0 个关键帧），逐帧 seek 每次都从关键帧重解 —— 实测 212ms/帧，预览 5fps。
  // 看片不需要帧精确；出片（原生 CLI）与双端比对走的是**默认关**的那条路，
  // 所以帧精确一点没丢。
  dhampirEngine.followPlayback = true
  dhampirSourceMode.value = dhampirEngine.sourceMode || ''

  dhampirEngine.resize(canvas.width, canvas.height)
  for (const [assetId, videoId] of bindings) dhampirEngine.bindSource(assetId, videoId)
  // **让源视频播起来** —— followPlayback 只在视频自己正在走时才跳过 seek。
  // 静音：预览不带声音（口径如此）；它们只是「帧的来源」。
  // `dhampirSourceVideos` 与 `dhampirVideos` 是**同一批元素**：前者的键是引擎的
  // `source`（= 素材 id，`sources_for` 给的就是它），纠偏按 source 找元素要用它。
  dhampirSourceVideos = new Map()
  for (const [assetId, , v] of bindings) dhampirSourceVideos.set(assetId, v)
  dhampirVideos = [...dhampirSourceVideos.values()]

  // **备用那套源**（A/B）：每个源再多一块同 `src` 的 `<video>`，1px、暂停、静音，
  // 平时什么都不做；进屏蔽区前把"下一段起点"seek 好，到点 `bindSource` 换绑即成。
  // 放在同一个 1px 宿主里（`stopDhampir` 一句 `innerHTML=''` 就一起收掉）。
  dhampirTwins = new Map()
  for (const [assetId, , v] of bindings) {
    const twin = document.createElement('video')
    twin.id = v.id + '-b'
    twin.muted = true
    twin.playsInline = true
    // `metadata` 够了：预滚那一次 `currentTime` 会按需去取它要的那段（同一个 URL，
    // 与台上那块共用浏览器缓存），不必让它从片头开始把整片都预读一遍。
    twin.preload = 'metadata'
    twin.src = v.src
    twin.pause()
    host.appendChild(twin)
    dhampirWatchTwin(assetId, twin)
    dhampirTwins.set(assetId, { el: twin, id: twin.id, want: 0, ready: false })
  }

  // 贴纸：**自己解码 GIF，按工程要的帧号取帧**（见下面 `ImageDecoder` 那段）。
  dhampirStickers = stickerImages.map((a, k) => {
    // 素材时基（贴纸 GIF 是 100/3 ≈ 33.33fps）—— 用来把"这一帧第几秒"换成第几帧。
    //
    // **GIF 的解码放到服务端**（`/polish/sticker-frame` 按帧给 PNG）。浏览器这条路
    // 两条都实测过、都不可用：`<img>` 的动画时钟会被节流（不可见 / 被 canvas 遮住 /
    // 1×1 盖在最上层——三种可见性写法都不推进帧），而 `ImageDecoder`（WebCodecs）
    // 在目标浏览器里根本不存在（`typeof ImageDecoder !== 'function'` → 帧数 0）。
    // 服务端用的是**出片那同一个解码器**，于是预览与成片的贴纸逐帧一致。
    const tb = a.timebase || {}
    const fps = (tb.num > 0 && tb.den > 0) ? (tb.num / tb.den) : 30
    // **这个素材被用在哪些"层区间"里**（工程帧）。两个用处：
    //
    //   1. **活跃判据**：只有当前帧落在某个区间里才值得交位图 —— 引擎的
    //      `sources_for` 只看这一帧的合成，不活跃的层它根本不会去取。
    //      每帧白交 6 张 500×500（`drawImage` + `createImageBitmap` + 上传）是纯浪费，
    //      而这段正是"帧率不够"的一块。
    //   2. **GIF 的相位锚点**：`<img>` 的动画时钟从**页面加载**那一刻开始跑，
    //      与"工程第几帧"毫无关系 —— 不重启的话，贴纸显示的是**随机相位**的 GIF 帧
    //      （看起来就是"GIF 没有正常播放"）。素材 timebase 是 100/3 ≈ 33.33fps，
    //      所以相位必须锚在**层的起点**上。
    const spans = []
    for (const track of (project.timeline && project.timeline.tracks) || []) {
      for (const layer of track.layers || []) {
        if (layer.enabled === false) continue
        if (layer.source && layer.source.asset_id === a.id) {
          spans.push({
            start: Number(layer.start) || 0,
            end: Number(layer.end) || 0,
            // **这个区间起点对应的素材帧**（层里的 `source.source_in`）。
            // 有了它，"这一帧该放 GIF 的第几帧"可以自己算出来，不必依赖
            // `sourcesFor` 返回的字段名 —— 那条路我一开始就赌错了字段名的话，
            // 症状恰好是"贴纸一直是同一帧"（`sec` 拿不到 → `lastFrame` 永远 -1 → 不交）。
            sourceIn: Number((layer.source && layer.source.source_in) || 0),
          })
        }
      }
    }
    spans.sort((x, y) => x.start - y.start)
    // **`uri` 与 `assetId` 是两样东西**：工程里的 `asset.uri` 才是"工程内相对路径"
    // （`assets/stickers/问号.gif`），而 `assetId`（`sticker/问号.gif`）只是素材表的键。
    // 服务端是拿 uri 往**片段目录**上 join 的 —— 传 id 的后果是 404「贴纸不存在」，
    // 而日志里那句话看起来像"文件没了"，其实是参数给错了（我踩过）。
    return { assetId: a.id, uri: a.uri, fps, spans, activeKey: null, bytes: null, bytesIdx: -1, count: 0, lastFrame: -1 }
    // 「产品没建」与「驱动的查询看不到」这两种可能。
  })
  // **预热贴纸帧表**（2026-10-01）：服务端第一次被要某张贴纸的某一帧时，得**整段解码**那张
  // GIF（debug 构建实测单张 3~9 秒、`clip-04` 四张合计 20.2 秒）。而逐帧接口那头
  // `Promise.race([…, 30ms])` 超时就"这一帧不交" ⇒ 症状是**播放走到贴纸先空一段、然后突然出现**。
  //
  // 这里在**页面加载时**就把这件事做掉：解码挪到"用户还没播到那里"的时候，真正播到时全部命中
  // 缓存（实测 10~14ms）。**fire-and-forget**：不 await、不挡任何东西，失败了也不影响播放
  // （真有问题会在播到那一帧时由 `/polish/sticker-frame` 照常报错，不会静默）。
  //
  // ⚠️ 服务端**逐张串行**解（它是 `blocking` 池里的一个循环），所以这里**不要**按贴纸并发发多个
  // 请求 —— 那只会让同一张 CPU 被几路抢，反而更慢。
  callPrewarmStickers()
  for (const v of dhampirVideos) { v.muted = true; v.play().catch(() => {}) }

  // ④ **字幕 / 弹幕**。
  //
  // 工程里那两条资产的 `uri` 是**绝对路径**（指向转译输出目录），浏览器取不到 ——
  // 所以文本由后端在 `/dhampir-project` 里一并带回来（它在 cleanup 之前读）。
  // 这里转成 blob URL 喂给引擎；**解析在 wasm 里**（parse_srt / parse_ass）。
  //
  // 不做这一步的症状是最难查的那一类：**预览看起来正常、只是没有字幕**，还不报错。
  // **把工程声明的字体族装进页面。**
  //
  // canvas 栅格化用的是**页面里已有的字体**：字体没加载就**静默回退**，
  // 而症状正是"字体不对"（字号/位置/颜色都对）。DOM 预览那条路已经在用
  // `ensureFontFace`，这里复用同一个入口 —— 它取的是 `/api/polish/font`，
  // 与出片侧注入的 @font-face 是同一份字节。
  const families = new Set()
  for (const track of (project.timeline && project.timeline.tracks) || []) {
    const f = (track.subtitle && track.subtitle.font_family) || (track.danmaku && track.danmaku.font_family)
    if (f) families.add(f)
  }
  for (const family of families) {
    try {
      ensureFontFace(null, props.dir, family, { onDebug: (m) => addClient('debug', m, 'dhampir') })
    } catch (e) {
      dhampirStatus.value = '字体 ' + family + ' 没装上（预览会用回退字体）'
    }
  }
  // 字体是异步到位的；不等它的话第一屏会是回退字形。
  try { if (document.fonts && document.fonts.ready) await document.fonts.ready } catch (e) { /* 等不到就算了 */ }

  const subTexts = res.subtitles || {}
  const entries = []
  for (const a of project.assets || []) {
    if (a.kind !== 'subtitle') continue
    const name = String(a.uri || '').split(/[\\/]/).pop()
    const text = subTexts[name]
    if (!text) continue
    const ext = (name.split('.').pop() || '').toLowerCase()
    const url = URL.createObjectURL(new Blob([text], { type: 'text/plain;charset=utf-8' }))
    dhampirBlobUrls.push(url)
    entries.push({ assetId: a.id, url, format: ext === 'ass' || ext === 'ssa' ? 'ass' : 'srt' })
  }
  if (entries.length > 0) {
    const loaded = await dhampirEngine.loadSubtitles(entries)
    const failed = (loaded && loaded.failed) || []
    // **失败要看得见**：登记不上的那一路，画面里就是"没有字幕/弹幕"，不报错。
    if (failed.length > 0) {
      dhampirStatus.value = '（字幕 ' + failed.length + ' 路没登记上）'
    }
  }
}

function stopDhampir() {
  if (dhampirRaf) {
    cancelAnimationFrame(dhampirRaf)
    dhampirRaf = 0
  }
  const host = dhampirHostRef.value
  if (host) host.innerHTML = ''
  dhampirEngine = null // wasm 侧是单例宿主；下次 attach 会重新接上
  dhampirSegments = []
  dhampirBusy = false
  dhampirSourceVideos = new Map()
  dhampirVideos = []
  dhampirTwins = new Map()
  dhampirArmedIds = []
  dhampirHoldSince = 0
  dhampirAlignStat.quiet = 0
  dhampirAlignStat.rate = 0
  dhampirAlignStat.hard = 0
  dhampirAlignShown.value = ''
  dhampirMeasuredFps.value = 0
  dhampirFpsWindow = { t0: 0, frames: 0 }
  for (const url of dhampirBlobUrls) URL.revokeObjectURL(url)
  dhampirBlobUrls = []
}

/**
 * **把"当前时间"落到 Dhampir 用的那些视频上。**
 *
 * 为什么必须有：时钟读的是 `videoRef.currentTime`，而**画面来自 `dhampirVideos`**
 * （`bindSource` 绑的是它们）。只动 `videoRef` 的话，拖进度条、点时间轴、或者
 * **骑过手动屏蔽区间**之后，时间轴停在 X、画布还画在 Y —— 用户看到的就是
 * 「时间轴对不上」。以前 `seekTo` / `onSeeked` / 屏蔽跳转都只碰 `videoRef` 与背景视频。
 */
/**
 * **时钟自己的速率** = 源的"自然速率"。
 *
 * 纠偏要跟的是 DOM 时钟，所以"自然"不是一个写死的 1×：今天时钟恒为 1×，
 * 但纠偏的 ±2% 必须是**在基准上叠加**的（`base ± 2%`），否则将来加了倍速播放，
 * 纠偏每帧都会把源拽回 1× —— 症状是"倍速下画面与时间轴越走越远"。
 */
function dhampirClockRate() {
  const el = videoRef.value
  const r = el && Number(el.playbackRate)
  return Number.isFinite(r) && r > 0 ? r : 1
}

function syncDhampirTime(t) {
  if (!dhampirVideos.length) return
  const base = dhampirClockRate()
  for (const v of dhampirVideos) {
    // **拖动/切片段是硬对齐**：先把纠偏留下的速率清掉（不然拖动之后源会一直
    // 按 base±2% 走），再按老口径（>20ms 才赋值）把位置直接落到位。
    if (Math.abs(v.playbackRate - base) > 1e-9) {
      try { v.playbackRate = base } catch (e) { /* 不支持就算了，下一帧纠偏会再试 */ }
    }
    if (Math.abs((v.currentTime || 0) - t) > 0.02) {
      try { v.currentTime = t } catch (e) { /* 元数据还没到时赋值会被忽略，下一帧再来 */ }
    }
  }
}

/**
 * **三档纠偏**：每一帧、在 `engine.seek(f)` **之前**把源 `<video>` 与「这一帧该停在哪」
 * 的偏差收拾掉。见 `DHAMPIR_ALIGN_QUIET` 上面那段说明（为什么不能只比一次）。
 *
 * 「这一帧该停在哪」**不问引擎要第二份实现** —— 用的就是 `sourcesFor(f)`（`prepare`
 * 内部同一个调用），它的 `seconds` 是权威值：多路素材各有各的时基，自己再算一遍
 * 就会与引擎漂开（症状是"某一层总是慢半拍"）。
 *
 * 三档（`err = 实际 - 目标`，>0 = 超前；`base` = 时钟速率，见 `dhampirClockRate`）：
 *   ① |err| < 40ms  → 什么都不做（速率收回 `base`，让它自然播 ✓ 这一档是绝大多数帧）
 *   ② |err| < 500ms → **只改 `playbackRate`**（超前 `base×0.98` / 落后 `base×1.02`），
 *      **不 seek** ✓ —— 这正是"躲开每帧 hard seek"的那一档
 *   ③ |err| ≥ 500ms → 速率复位 + 硬 seek（时钟在走时由这里做；暂停时交给引擎 ——
 *      它那边的 `seekVideo` 会**等 `seeked` 事件**，比这里"设了就走"更靠谱，
 *      而且它按 `currentTime === seconds` 短路，两边都设会变成一次多余的 seek ✗）
 *
 * 末尾那句"时钟在走 ⇒ 源也得在走"是**跟随播放的前提**：引擎只认 `paused`，
 * 源被镜像暂停（DOM 时钟暂停）之后恢复播放时，少了它就会退回每帧 hard seek。
 */
function dhampirAlignSources(f) {
  if (!dhampirEngine || dhampirSourceVideos.size === 0) return
  let entries = []
  try {
    entries = dhampirEngine.sourcesFor(f) || []
  } catch (e) {
    return // 引擎还没就绪/这一帧无源：纠偏不是必须的，绝不能因此打断出帧
  }
  const el = videoRef.value
  const clockRunning = !!(el && !el.paused && !el.ended)
  const base = dhampirClockRate()
  for (const e of entries) {
    const v = dhampirSourceVideos.get(e && e.source)
    if (!v) continue
    const want = typeof (e && e.seconds) === 'number' ? e.seconds : (e && typeof e.t === 'number' ? e.t : null)
    if (want === null) continue
    const err = (v.currentTime || 0) - want // >0 = 超前
    const abs = Math.abs(err)
    if (abs >= DHAMPIR_ALIGN_HARD) {
      // ③ 大偏差：**才** hard seek
      if (Math.abs(v.playbackRate - base) > 1e-9) {
        try { v.playbackRate = base } catch (e2) { /* 忽略 */ }
      }
      if (clockRunning) {
        try { v.currentTime = want } catch (e2) { /* 元数据没到就下一帧再来 */ }
      }
      dhampirAlignStat.hard += 1
      continue
    }
    if (abs < DHAMPIR_ALIGN_QUIET || !clockRunning) {
      // ① 小偏差：不碰（把上一步可能留下的速率收回来）
      if (Math.abs(v.playbackRate - base) > 1e-9) {
        try { v.playbackRate = base } catch (e2) { /* 忽略 */ }
      }
      dhampirAlignStat.quiet += 1
      continue
    }
    // ② 中偏差：**只改速率，不 seek**
    const rate = err > 0 ? base * DHAMPIR_ALIGN_RATE_LO : base * DHAMPIR_ALIGN_RATE_HI
    if (Math.abs(v.playbackRate - rate) > 1e-9) {
      try { v.playbackRate = rate } catch (e2) { /* 忽略 */ }
    }
    dhampirAlignStat.rate += 1
  }
  if (!clockRunning) return
  for (const v of dhampirSourceVideos.values()) {
    if (v.paused) v.play().catch(() => {})
  }
}

/** 每帧：把"现在第几秒"换成帧号交给 `engine.seek()`（它内部做完 sources_for + seek + draw）。 */
function scheduleDhampir() {
  if (dhampirRaf) return
  const tick = () => {
    dhampirRaf = requestAnimationFrame(tick)
    if (!dhampirMode.value || !dhampirEngine || dhampirBusy) return
    const el = videoRef.value
    // **镜像 DOM 时钟的暂停状态**：引擎的「跟随播放」只看 `paused`（它不比绝对时刻，
    // 见 engine.js 里那段说明）—— 时钟暂停时源也得暂停（暂停态由引擎**按帧** seek 到
    // 精确位置 ✓），时钟在走时源必须在走（否则引擎退回每帧 hard seek ⇒ 实测 307ms/帧）。
    // 速率上的小偏差不在这里管，交给 `dhampirAlignSources` 的三档纠偏 ✓
    for (const v of dhampirVideos) {
      if (el && v.paused !== el.paused) { if (el.paused) v.pause(); else v.play().catch(() => {}) }
    }
    const f = dhampirFrameAt(el ? el.currentTime : current.value)
    // `null` = 当前源位置落在**被掐掉的区间**里（工程里没有这一段）：不画，
    // 把源视频挪到下一段的源起点（与 tick 里同一处说明）。
    if (f === null) {
      const target = dhampirNextSourceStart(el ? el.currentTime : current.value)
      if (target !== null) syncDhampirTime(target)
      return
    }
    if (f === dhampirFrameShown) return // 帧号没变就别重画（30fps 内容在 60Hz rAF 上会画两次）
    // ⚠️ **源没就绪就别进引擎**：跟播时引擎**不 await seek**（`following === true` 时它直接取
    // `<video>`"正在显示"的那一帧），源在 seeking / 没像素时合成出来就是**整块黑**。
    // 而这块 WebGPU 画布**读不回来**（surface 只有 RENDER_ATTACHMENT，Dhampir 的
    // `timeline_host.rs` 自己写着）⇒ 定格遮罩的 `drawImage(canvas)` 拿不到像素、遮罩是透明的
    // ⇒ 用户看到的就是"整块纯黑"。**不画**，画布会保留上一帧（交换链既有行为）⇒ 只是"停一下"。
    // 这一段本该由 A/B 换绑消掉（`dhampirSwapSources`），这里是它没赶上的兜底。
    if (!dhampirSourcesReady(f)) {
      if (dhampirHoldSince && performance.now() - dhampirHoldSince > DHAMPIR_SOURCE_WAIT_MS) {
        dhampirHoldSince = 0 // 等超了：照画（宁可露一帧黑，也不能把预览焊死）
      } else {
        if (!dhampirHoldSince) dhampirHoldSince = performance.now()
        return
      }
    } else {
      dhampirHoldSince = 0
    }
    // **三档纠偏**（A 案）：把源与"这一帧该停在哪"的偏差收拾掉再进引擎 ——
    // 大偏差它才会去 hard seek（小偏差不动、中偏差只改速率）。放在 `dhampirBusy = true`
    // 之前：纠偏是同步的、失败也只是不纠偏，绝不能让它把"这一帧在飞"的闸卡住。
    dhampirAlignSources(f)
    // **整圈起点**：从"决定画这一帧"到"这一帧画完"。它与 `engine.lastSeekCost` 的差，
    // 就是**发生在 `engine.seek` 之外**的时间 —— 那正是工具条分段解释不了的那部分。
    const loopT0 = performance.now()
    dhampirBusy = true
    Promise.resolve(dhampirEngine.seek(f, {
      // **贴纸位图必须在这里交**（prepare 之后、draw 之前）。交在 seek 之后会被
      // 下一次 prepare 的 clear_bitmaps() 清掉 —— 症状是"交与不交画面完全一样"。
      onPrepared: async () => {
        // 引擎按工程算好了**每个源这一帧停在第几秒**（各自经素材时间基换算）——
        // 贴纸的 GIF 帧号就从它来，不再看 `<img>` 当前显示的是哪一帧。
        const wanted = new Map()
        for (const e of dhampirEngine.sourcesFor(f) || []) {
          const src = (e && e.source) || null
          const sec = (e && (typeof e.seconds === 'number' ? e.seconds : e.t)) || 0
          if (src) wanted.set(src, sec)
        }
        dhampirStickerStat.total = dhampirStickers.length
        dhampirStickerStat.ready = dhampirStickers.filter((x) => !!x.bytes).length
        for (const st of dhampirStickers) {
          // **秒数**：优先用引擎给的；给不出就自己按时间基算 ——
          // 素材帧 = source_in + (工程帧 - 层起点) × (素材fps / 工程fps)，再除回秒。
          //（工程时基 60 与素材时基 100/3 不是一回事，直接相乘会差 1.8 倍。）
          let sec = wanted.get(st.assetId)
          if (sec === undefined) {
            const span = st.spans.find((sp) => f >= sp.start && f < sp.end)
            if (!span) { st.lastFrame = -1; continue }           // 这一帧不活跃
            sec = (span.sourceIn + (f - span.start) * (st.fps / dhampirFps.value)) / st.fps
          }
          const idx = Math.max(0, Math.floor(sec * st.fps + 1e-6))
          try {
            // **帧号变了才去取**（取回来的是 PNG 字节，缓存住；同一帧反复用的是同一份）。
            if (idx !== st.bytesIdx) {
              // ⚠️ **同一个贴纸同一时刻只允许一个在飞**（2026-10-01 修性能问题）。
              //
              // 以前是每帧都 `Promise.race([api(…), 30ms])` —— 30ms 放弃但**不中止请求** ✗，
              // 而下一帧（33ms 后）帧号已经变了 ⇒ 不是"同一个请求"（去重也拦不住）⇒ 服务端
              // 冷解码一张 GIF 期间，**每帧都再发一个**：实测 200ms 内 30+ 个请求、
              // 其中 4 个各 4.1 秒（= 同一张图被并发整段解码 4 次 ✗）。
              //
              // ⚠️ **冷解码的真实代价比那个 4.1s 还大一倍**（2026-10-01 实测
              // `decode_sticker_frames_png`，`clip-04/assets/stickers/`，debug 构建）：
              // `冒泡_2.gif` 77 帧 **8.8s** / `哭_1.gif` 66 帧 **6.5s** / `跳舞_1.gif` 66 帧 **6.8s**
              // （解出来 15–20MB PNG 字节）。所以"每帧发一个"的后果是**同一张图被并发解 N 次**，
              // 每次都把 CPU 占满 6–9 秒 —— 这也是"播放走到贴纸就卡死"的主因，不只是请求数多。
              // 换素材（更大的 GIF）会让这几个数继续涨，别把 30ms 那个超时当成安全阀。
              //
              // 现在：这个贴纸**有在飞的就不发新的**（这一帧先不交它，语义与超时一致 ✓）；
              // 落地后按自己的帧号采用，下一次需要新帧时再发。服务端解码一次之后，
              // 单帧取回只要几毫秒 ⇒ 超时几乎不再触发。
              let res = null
              if (!st.pending) {
                const p = api('/api/polish/sticker-frame', {
                  method: 'POST',
                  label: '贴纸帧',
                  body: JSON.stringify({ dir: props.dir, asset: st.uri, frame: idx }),
                })
                  .catch(() => null)
                  .finally(() => { st.pending = null })
                st.pending = p
                res = await Promise.race([p, new Promise((r) => setTimeout(() => r(null), 30))])
              }
              if (!res) continue   // **在飞/超时 ⇒ 这一帧不交该贴纸** ✓ 永不卡帧 ✓
              if (res.ok !== true || !res.png_base64) {
                throw new Error('接口没给 png_base64' + (res && res.count ? '（共 ' + res.count + ' 帧）' : ''))
              }
              const bin = atob(res.png_base64)
              const bytes = new Uint8Array(bin.length)
              for (let i = 0; i < bin.length; i += 1) bytes[i] = bin.charCodeAt(i)
              st.bytes = bytes
              st.bytesIdx = idx
              st.count = res.count || 0
              dhampirStickerStat.decodes += 1                        // 计的是"取了几次"
            }
            if (!st.bytes) continue
            // ⚠️ **每帧都必须交**：引擎每帧 `prepare` 开头都会 `clear_bitmaps()`，
            // 不交的那一帧这个源没有位图 → `require_bitmap` 直接不画那一层。
            // 而且位图**每帧要是新对象**：wasm 换掉旧位图时会 `close()` 它，
            // 复用会拿到"已关闭的位图"（不报错、只是不显示）。
            //
            // **PNG 解码只在帧号变了时做一次**：以前是每个显示帧都把同一张 PNG
            // 重新解一遍（`createImageBitmap(new Blob([st.bytes]))`），贴纸活跃时
            // 那就是**每帧一次完整 PNG 解码** —— 这是"帧率不够"里的一块。
            // 现在解出来的位图画进一张**常驻 canvas**，之后每帧只做一次便宜的
            // `createImageBitmap(canvas)`：仍然每帧新建（契约要求 ✓），但不再重复解码。
            if (st.canvasIdx !== idx || !st.canvas) {
              const bmp = await createImageBitmap(new Blob([st.bytes], { type: 'image/png' }))
              if (!st.canvas) st.canvas = document.createElement('canvas')
              st.canvas.width = bmp.width
              st.canvas.height = bmp.height
              st.canvas.getContext('2d').drawImage(bmp, 0, 0)
              // 中转用的位图**自己关掉**：留着就是白占一块内存。
              bmp.close()
              st.canvasIdx = idx
            }
            dhampirStickerStat.idx = idx
            dhampirEngine.setSourceBitmap(st.assetId, await createImageBitmap(st.canvas))
            dhampirStickerStat.submits += 1
          } catch (e) {
            // **不吞**：吞掉只会让"贴纸不动"变成查不出的谜（这个坑我踩过）
            console.warn('dhampir: 贴纸帧取不到 ' + st.assetId + '（第 ' + idx + ' 帧）：' + ((e && e.message) || e))
          }
        }
      },
    }))
      .then(async () => {
        // 每帧只更新**非响应式**那个（跳过判断用 ✓）；
        // 模板用的 ref 跟其它读数一起限频到 ~4Hz —— 它本来就只是给人看的 ✓。
        if (dhampirLastFrameDrawn >= 0 && f > dhampirLastFrameDrawn + 1) {
          dhampirDropped.value += (f - dhampirLastFrameDrawn - 1)   // **丢失帧** ✓
        }
        dhampirLastFrameDrawn = f
        dhampirFrameShown = f
        // **读数限频到 ~4Hz**：这些 ref 每帧都写的话，工具条每帧重渲染一次 ——
        // 那是纯开销，而且只为了让人眼看得清（人眼也读不了每秒 60 次的数字）。
        // 画面本体的更新不受影响（帧号 `dhampirFrame` 仍然每帧写，它便宜）。
        if (performance.now() - dhampirReadoutAt > 250) {
          dhampirReadoutAt = performance.now()
          dhampirFrame.value = f      // 模板那一栏（人眼看 4Hz 足够 ✓）
          if (dhampirStickerStat.total > 0) {
            dhampirStickerInfo.value = '贴纸 就绪 ' + dhampirStickerStat.ready + '/' + dhampirStickerStat.total +
              ' · 本帧 idx ' + dhampirStickerStat.idx +
              ' · 解码 ' + dhampirStickerStat.decodes + ' · 已交 ' + dhampirStickerStat.submits
          }
          const cost = dhampirEngine.lastSeekCost
          if (cost) {
            dhampirPrepMs.value = Math.round(cost.prepareMs)
            dhampirDrawMs.value = Math.round(cost.drawMs)
          const bd = dhampirEngine.lastPrepareBreakdown
          if (bd) {
            // 引擎早就记了完整分段（`createBitmapMs` + `setBitmapMs` 是 bitmap 模式的核心代价，
            // 后者含 GPU 上传）—— 之前只露了总数，"慢在哪"就还是得猜。
            dhampirLatencyMs.value = Math.round(performance.now() - loopT0)
            dhampirAdvanced.value = '整圈 ' + Math.round(performance.now() - loopT0)
              + 'ms / seek ' + Math.round(bd.seekMs)
              + ' / bitmap ' + Math.round(bd.createBitmapMs)
              + ' / set ' + Math.round(bd.setBitmapMs)
              + ' / text ' + Math.round(bd.textMs)
              + '（命中 ' + bd.shortCircuitHits + ' 未命中 ' + bd.shortCircuitMisses + '）'
              // **三档纠偏的命中数**（每秒归零）：`硬 0` 就说明这一秒没有一次硬 seek ✓
              + (dhampirAlignShown.value ? ' · 对齐 ' + dhampirAlignShown.value : '')
          }
          }
          if (Array.isArray(dhampirEngine.slowSeeks)) {
            dhampirSlowSeeks.value = dhampirEngine.slowSeeks.length
          }
        }
        dhampirError.value = ''
        // 实测帧率：1 秒一个窗口。**这是给用户看的读数**，也是降级判断的依据。
        const now = performance.now()
        if (!dhampirFpsWindow.t0) dhampirFpsWindow = { t0: now, frames: 0 }
        dhampirFpsWindow.frames += 1
        const span = now - dhampirFpsWindow.t0
        if (span >= 1000) {
          dhampirMeasuredFps.value = Math.round((dhampirFpsWindow.frames * 1000) / span)
          dhampirFpsWindow = { t0: now, frames: 0 }
          dhampirDroppedShown.value = dhampirDropped.value   // 每秒归零显示 ✓
          dhampirDropped.value = 0
          // 三档纠偏的读数同样每秒归零（逐帧累加的话一眼看不出"这一秒还 seek 不 seek"）
          dhampirAlignShown.value = '静 ' + dhampirAlignStat.quiet + ' 快 ' + dhampirAlignStat.rate + ' 硬 ' + dhampirAlignStat.hard
          dhampirAlignStat.quiet = 0
          dhampirAlignStat.rate = 0
          dhampirAlignStat.hard = 0
        }
      })
      .catch((e) => {
        dhampirError.value = String((e && e.message) || e)
      })
      .finally(() => {
        dhampirBusy = false
        // **帧尾排帧**（B 案）：一帧画完**立刻续跑**，不等一个显示刷新周期。
        //
        // 为什么：一帧的活（prep + draw ≈ 20ms）本来就长于一个刷新间隔 ⇒ 帧尾若再
        // 排一个 rAF，每帧净多等 ~16.7ms（实测读数就是"整圈 20ms 却只有 24fps"，
        // 数学正好对得上：20 + 16.7 ≈ 37ms ⇒ 27fps）。
        //
        // 两步：先把 tick **开头**排的那个 rAF 撤掉（它对同一帧的 `tick` 身份本来也会
        // 被 rAF 去重，撤掉只是省一次空跑），再**直接**调 `tick()` 续跑。
        //
        // ⚠️ **续跑点只有这一处 + 开头那处保活**，且这一处放在 `finally` 里 ⇒
        // 成功 / 打不开 / 抛错 / 跳过**任何出口都会续跑**（死循环的前车之鉴：
        // 帧尾用 `setTimeout` 排帧 ⇒ 待办回调逐帧累积 ⇒ 某一帧几十个回调一起冲进
        // wasm ⇒ `RefCell already borrowed` panic ✗）。这里任一时刻**恰好一个待办**：
        // 撤掉旧的、立刻跑新的、新的开头再排一个 ⇒ 不累积 ✓
        if (dhampirRaf) {
          cancelAnimationFrame(dhampirRaf)
          dhampirRaf = 0
        }
        // 只有还在 Dhampir 模式里才续跑：`stopDhampir()` 之后不能自己把自己转起来。
        if (dhampirMode.value && dhampirEngine) tick()
      })
  }
  // **开头这处 rAF 是唯一的保活**：帧尾那处是"加速"，它自己也会在 tick 开头重排一个
  // ⇒ 万一帧尾那条路断了（模式退出、引擎空、抛错），起搏仍然在 ✓。
  // 只排**一个**待办回调（`requestAnimationFrame` 按回调身份去重 ✓）——
  // "帧尾用 setTimeout 排帧"那版就是这么把 wasm 撞进 `RefCell already borrowed` 的 ✗。
  dhampirRaf = requestAnimationFrame(tick)
}

const stageRef = ref(null)
/** **台上那块** `<video>` 的 ref（A/B 双缓冲里 0/1 两块之一）。 */
const stage0Ref = ref(null)
const stage1Ref = ref(null)
/** 哪一块在台上（0/1）。**换元素 = 改这一个数**，`videoRef` 跟着变。 */
const activeStage = ref(0)
/**
 * 台上那块 `<video>`。
 *
 * ⚠️ 它是 **computed**，不是模板 ref —— A/B 双缓冲要"换元素"，而组件里几十处都在读
 * `videoRef.value`（时钟、`seekTo`、音量、定格快照、Dhampir 纠偏…）。做成 computed 之后
 * 那些地方**一个字都不用改**，换台就是换 `activeStage` 这一个数。
 */
const videoRef = computed(() => (activeStage.value === 0 ? stage0Ref.value : stage1Ref.value))
/** 台下的备用那块（预滚用）。 */
const standbyRef = computed(() => (activeStage.value === 0 ? stage1Ref.value : stage0Ref.value))
const bgRef = ref(null)
const current = ref(0)
const duration = ref(0)
const playing = ref(false)
const muted = ref(false)
const volume = ref(1)
/** 非零音量的记忆值：从 0 恢复声音时用它（localStorage 里存的就是 masterVol） */
let lastVol = 1
/** 按钮/图标状态：静音或音量为 0 都算「没声音」 */
const silenced = computed(() => muted.value || volume.value <= 0)
const overlayScale = ref(1)
const stickerFailed = ref({})
let rafId = 0
let ro = null

const player = createSfxPlayer({
  resolveUrl: (name) => clipMediaUrl(props.pid, props.dir, 'sfx/' + name + '.mp3')
})

const videoSrc = computed(() => clipMediaUrl(props.pid, props.dir, props.config?.source?.video || 'clip.mp4'))
const muteRanges = computed(() => (props.config?.mute_ranges || [])
  .map(r => ({ start: Number(r.start) || 0, end: Number(r.end) || 0 }))
  .filter(r => r.end - r.start > 0.001)
  .sort((a, b) => a.start - b.start))
/** `t` 落在哪一段屏蔽区间里。`pad` = 提前量（见 `SKIP_LOOKAHEAD`）：跳转要提前一帧认定，
 *  否则区间里那一帧（本不该出现的内容）会先被显示出来。 */
function muteRangeAt(t, pad = 0) {
  return muteRanges.value.find(r => t >= r.start - pad && t < r.end) || null
}
const vertical = computed(() => !!props.config?.vertical)

// ---- 输出画布（`_canvas` 契约，SSOT 在 ./polish/canvas.js）----
// 横屏 1920×1080 → k=kh=1，一切几何与改造前逐像素相同；竖屏 1080×1920 时
// 舞台比例 / 虚拟 overlay / 弹幕带 / 字幕带 / 贴纸盒子全部随 k/kh 缩放。
const cv = computed(() => canvasOf(props.config))

// ---- 画面构成（位置 / 背景）----
// 取值来自 `_canvas.stage` —— Rust `ProjectFile::resolve_stage` 唯一推导，前端不重算（两边不可能不一致）。
// 拿不到 `_canvas`（老接口 / 非 preview 请求）时退回改造前的竖屏规则，行为与以前一致。
const FALLBACK_STAGE = {
  axis: 'fill', object_fit: 'cover', position: 'center', bg: 'none',
  bg_color: '#000000', blur_px: 28, blur_img_scale: 1.187, scale: 1, padding: 0
}
const stageFit = computed(() => {
  const s = props.config?._canvas?.stage
  if (s) return s
  return vertical.value
    ? { ...FALLBACK_STAGE, axis: 'x', object_fit: 'contain', bg: 'blur',
        position: props.config?.vertical_position || 'left' }
    : FALLBACK_STAGE
})
/** 背景层（当前帧模糊）只在 bg=blur 时需要；black / color 走舞台底色 */
const bgOn = computed(() => stageFit.value.bg === 'blur')
const stageClass = computed(() => {
  const s = stageFit.value
  const axis = s.axis || 'fill'
  return axis === 'fill' ? ['axis-fill'] : ['axis-' + axis, 'stage-' + (s.position || 'center')]
})
/** fit 作用域的 CSS 变量：放在 `.stage` 上，`.video-wrap` / `.fit-box` / `.bg-video` 全部继承 */
const stageStyle = computed(() => {
  const c = props.config?._canvas || {}
  const s = stageFit.value
  const src = c.src || {}
  const v = cv.value
  return {
    background: s.bg === 'color' ? (s.bg_color || '#000000') : '#000',
    '--src-ar': (src.width > 0 && src.height > 0) ? (src.width + ' / ' + src.height) : '16 / 9',
    '--stage-pad': ((Number(s.padding) || 0) / (c.width || 1920) * 100) + '%',
    '--stage-scale': String(s.scale || 1),
    '--bg-blur': (s.blur_px || 28) + 'px',
    '--bg-img-scale': String(s.blur_img_scale || 1.187),
    // 画布驱动的舞台比例 + 虚拟 overlay / 弹幕带 / 字幕带 / 贴纸盒子尺寸
    '--stage-ar': v.W + ' / ' + v.H,
    // 宽 = 高度预算(clamp) × W/H。⚠️ 必须是 **W/H**（横屏 1.7778，与 P3 前的硬编码 *16/9 一致）：
    // 曾经错写成 H/W，横屏舞台被缩到 1/3 高（用户报"横屏被改错"）。竖屏 W/H=0.5625 →
    // 宽 = clamp×0.5625，高 = 宽÷(9/16) = clamp —— **竖屏与横屏同高**（用户要求）。
    '--stage-ratio': String(v.W / v.H),
    '--ov-w': v.W + 'px',
    '--ov-h': v.H + 'px',
    '--dm-h': (540 * v.k) + 'px',
    '--sticker-s': (280 * stickerScale(v)) + 'px',
    '--sticker-emoji': (200 * stickerScale(v)) + 'px',
    '--cap-bottom': (80 * v.kh) + 'px',
    '--cap-left': (120 * v.k) + 'px',
    '--cap-w': (v.W - 240 * v.k) + 'px'
  }
})

const stickerMap = computed(() => props.config?._assets?.sticker_map || {})
function stickerSrc(name) {
  if (stickerFailed.value[name]) return ''
  const p = stickerMap.value[name]
  return p ? clipMediaUrl(props.pid, props.dir, p) : ''
}

const effDuration = computed(() => duration.value || props.metaDuration || 0)

/**
 * 喂给索引的配置：点选编辑态里把**草稿落点**临时顶上去，让贴纸本身跟着动
 * （所见即所得；不写盘，保存由父级负责）。
 *
 * 只覆盖 `_canvas.sticker.position`、不动 `position_mode` —— draft 只在「固定」模式里存在
 * （父级 `startPointEdit` 保证了这点），所以这不改变任何语义，只是把"已保存的那一版"
 * 换成"草稿那一版"。旧对象原样返回，避免无谓重建索引。
 */
const indexConfig = computed(() => {
  if (!props.pointMode || !props.point || !props.config) return props.config
  const nx = Number(props.point.x), ny = Number(props.point.y)
  if (!Number.isFinite(nx) || !Number.isFinite(ny)) return props.config
  const cl = (v) => Math.round(Math.min(100, Math.max(0, v)))
  const cv = props.config._canvas || {}
  const st = cv.sticker || {}
  const next = cl(nx) + '% ' + cl(ny) + '%'
  if (String(st.position || '') === next) return props.config
  return { ...props.config, _canvas: { ...cv, sticker: { ...st, position: next } } }
})

const idx = computed(() => buildIndex({
  config: indexConfig.value,
  subtitles: props.subtitles,
  danmaku: props.danmaku,
  duration: effDuration.value,
  stickerSrc
}))

// ---- 逐帧状态（rAF 更新 current 触发重算）----
const vfx = computed(() => videoFxAt(idx.value, current.value))
const flash = computed(() => getFlash(idx.value, current.value))
const hueFlash = computed(() => getHueShiftFlash(idx.value, current.value))
const noiseOp = computed(() => getNoiseOpacity(idx.value, current.value))
const vignette = computed(() => getVignette(idx.value, current.value))
const colorOverlay = computed(() => getOverlayState(idx.value, current.value))
const capRows = computed(() => subsAt(idx.value, current.value))

// A5 字幕折行：与出片（templates/index.html renderFrame）同一套 wrapCaptionParts
// —— 安全宽 = 画布宽 * 0.875、最多 3 行、装不下缩字号（下限 0.7）。
// 行内 white-space:nowrap 禁掉浏览器自动换行，预览与出片断行位置一致。
/** 量宽用的字体简写。⚠️ 兜底栈走 props.fontFallback（服务端动态算、与成片同源）——
 *  这里曾写死 `"LXGW WenKai",…`，"设为默认"对预览的量宽/折行完全无效。 */
const capFontSpec = computed(() => '700 ' + (subStyle.value.size || 72) + 'px '
  + (subStyle.value.font ? '"' + subStyle.value.font + '",' : '') + props.fontFallback)
const capStrokeW = computed(() => parseFloat(subStyle.value.stroke) || 12)
const capRowsWrapped = computed(() => capRows.value.map((c) => {
  const w = wrapCaptionParts(c.parts, cv.value.W * 0.875, (tx) => capMeasure(tx, capFontSpec.value))
  return { ...c, lines: w.lines, fontScale: w.scale }
}))
const dmRows = computed(() => danmakuAt(idx.value, current.value))
const stickers = computed(() => stickersAt(idx.value, current.value))
const progress = computed(() => duration.value ? Math.round(current.value / duration.value * 1000) : 0)
const fmt = (s) => {
  const n = Number(s) || 0
  const m = Math.floor(n / 60)
  const sec = n - m * 60
  const secStr = sec.toFixed(2).padStart(5, '0')
  return String(m).padStart(2, '0') + ':' + secStr
}

const subStyle = computed(() => idx.value.subStyle || {})
const dmStyleCfg = computed(() => idx.value.dmStyle || {})

const videoStyle = computed(() => {
  // 内容层的 object-fit 与出片一致（axis=fill → cover；留边 → contain），由 `_canvas.stage` 决定
  const objectFit = stageFit.value.object_fit || 'cover'
  // 框选布局模式：运镜/缩放/抖动/滤镜全部复位，让选框直接对应整帧真实位置
  if (props.selectMode) return { transform: 'none', transformOrigin: 'center', filter: 'none', objectFit }
  const f = vfx.value
  const t = []
  if (f.shake.x || f.shake.y) t.push(`translate(${f.shake.x}px, ${f.shake.y}px)`)
  // 手持摇摆（运镜的附加变种）：位移在屏幕空间，旋转/呼吸缩放绕同一个不动点，
  // 所以必须排在 scale 之前 —— transform 列表从右往左作用，而相机块（含运镜缩放）
  // 在出片模板里是最内层（最后 apply 到画面上的那一段）。
  const sway = handheldSway(current.value, f.camera.handheld)
  if (sway.dx || sway.dy) t.push(`translate(${sway.dx}px, ${sway.dy}px)`)
  if (sway.rot) t.push(`rotate(${sway.rot}deg)`)
  // 判据是「缩放不等于 1」而不是「大于 1」：画中画档位（zoom<1）也是合法运镜
  // 摇摆的呼吸缩放与运镜缩放都是绕不动点的缩放，合成一个 scale（amp=0 时 sway.scale 恒为 1）
  const camScale = f.zoom * sway.scale
  if (Math.abs(camScale - 1) > 0.001) t.push(`scale(${camScale})`)
  const filter = f.blur > 0 ? `blur(${f.blur}px)` : ''
  return {
    transform: t.join(' ') || 'none',
    transformOrigin: `${f.camera.ox / cv.value.W * 100}% ${f.camera.oy / cv.value.H * 100}%`,
    filter: filter || 'none',
    objectFit
  }
})
const flashStyle = computed(() => ({ opacity: flash.value.opacity, background: flash.value.color }))
const hueStyle = computed(() => hueFlash.value ? { opacity: hueFlash.value.opacity, background: hueFlash.value.color } : { opacity: 0 })
const vignetteStyle = computed(() => ({
  opacity: vignette.value,
  background: 'radial-gradient(ellipse at center,transparent 55%,rgba(0,0,0,0.5) 100%)'
}))
const colorOverlayStyle = computed(() => {
  const ov = colorOverlay.value
  if (!ov) return { opacity: 0 }
  if (ov.type === 'color') return { opacity: ov.opacity, background: ov.color }
  // 手拼字符串曾漏掉收尾 ')' —— 整条 background 非法会被浏览器丢弃，
  // 表现是「设了色档但预览里什么都没有」。统一走 engine 的 gradientCss。
  return { opacity: ov.opacity, background: gradientCss(ov.colors, ov.shape, ov.angle) }
})

const capLayerStyle = computed(() => {
  const s = subStyle.value
  return {
    // ⚠️ 兜底栈 = props.fontFallback（服务端动态算、与成片同源）；选了库内字体时排最前。
    // 这里曾写死 `"${s.font || 'LXGW WenKai'}","Microsoft YaHei",…` —— "设为默认"对预览无效。
    fontFamily: s.font ? `"${s.font}",${props.fontFallback}` : props.fontFallback,
    fontSize: (s.size || 72) + 'px',
    color: s.color || '#dcbda0',
    WebkitTextStroke: s.stroke || '12px #403c3b',
    '--hl-color': s.highlight_color || '#f56b41'
  }
})
const dmLayerStyle = computed(() => {
  const s = dmStyleCfg.value
  return {
    fontSize: (s.font_size || 42) + 'px',
    color: s.color || '#ffffff',
    WebkitTextStroke: s.stroke || '2px #000000'
  }
})

function onStickerError(st) {
  stickerFailed.value = { ...stickerFailed.value, [st.name]: true }
}

// 贴纸帧表预热（fire-and-forget，见 `dhampirStickers` 构建处那段说明）。
//
// 只发**一次**请求，把当前工程里所有贴纸的 uri 一起交上去；服务端在 `blocking` 池里逐张解。
// 失败按 `debug` 记 —— 它是纯优化，失败不该在日志面板里刷 warn 让人以为出了错。
function callPrewarmStickers() {
  const assets = (dhampirStickers || []).map((s) => s.uri).filter(Boolean)
  if (!assets.length) return
  api('/api/polish/sticker-prewarm', {
    method: 'POST',
    label: '贴纸预热',
    body: JSON.stringify({ dir: props.dir, assets }),
  }).catch(() => null)
}

// ---- 跳手动屏蔽区间：定格遮罩（2026-09）----
//
// 跳的办法是给 video 改 `currentTime`。**实测**（`.workbuddy/mute-skip/run.mjs`：CDP 录**合成器输出**
// + 页内逐帧轨迹；软件解码/GPU 解码 × headless/真窗口 四种组合都跑过）：
//
//   · seek 期间元素的 `readyState` 掉到 **1**（HAVE_METADATA）——**整段没有可呈现的帧**
//     （实测 seeking 101/102 帧），时长取决于缓冲与解码：未缓冲的远跳 **707ms**、缓冲好的 **190ms**；
//   · 本机合成器会**保持上一帧**（所以四种组合都没拍到黑帧），但"元素没有帧"这件事换一条解码
//     路径/换个平台露出来的就是它自己的黑底（`app.css` 全局 `video{background:#000}`）——
//     用户报的"黑屏一下"是这一档，**遮罩让它不再取决于平台**；
//   · 与平台无关、必然发生的是**错位**：画面停住等解码，而改造前时钟/字幕/弹幕是立刻跳到剪切后的
//     —— 那 100~700ms 里字已经换了、画面还是剪切前那一张，看起来正是"卡一下"。
//
// 所以：**seek 之前**把当前画面快照进一张盖住整块舞台的画布，等**新帧真的上屏**再撤；
// **定格期间时钟也冻住**（撤遮罩的那一帧才推，见 `endSkipFreeze`）—— 画面与字一起走，
// 观感从"黑一下 / 字先跳"变成一次硬切。
const freezeRef = ref(null)
const freezeOn = ref(false)
/** 撤遮罩的兜底：seek 万一不回来（解码器卡住 / 源被换掉），不能把画面永久冻住 */
const SKIP_FREEZE_MAX_MS = 700
/** 最短保留时长：`requestVideoFrameCallback` 可能在 seek 生效**之前**就回调（那一帧还是旧的），
 *  撤早了就又露出黑的那一帧 —— 给一道下限。 */
const SKIP_FREEZE_MIN_MS = 30
/** 提前这么多秒就认定"要进屏蔽区"。等到 `currentTime` 真的进区间，那一帧（16.7ms）已经把
 *  该掐掉的画面显示出来了；提前一帧触发，剪辑点才落在区间起点上。 */
const SKIP_LOOKAHEAD = 0.03
let freezeTimer = 0
let freezeStartedAt = 0

/** 遮罩的层级：DOM 模式（5）= 只冻**视频像素**，上层那套 DOM 覆盖层（字幕/弹幕/贴纸，z-index 10）
 *  留在上面、且它们的时钟也被冻住 ⇒ 整帧合成图一起定格；Dhampir 模式的画面是那块 canvas
 *  （连字一起画，z-index 40），遮罩必须盖在它上面。 */
const freezeZ = computed(() => (dhampirMode.value ? 60 : 5))

/** 舞台底色（与 `stageStyle` 同一条口径）；`blur` 那一档由 bg 视频自己画上去 */
function stageBackdropColor() {
  const s = stageFit.value
  return s.bg === 'color' ? (s.bg_color || '#000000') : '#000000'
}

/**
 * 把"此刻屏幕上那块画面"快照进遮罩画布。返回是否拍到了东西。
 *
 * ⚠️ **几何照元素的 `getBoundingClientRect()` 走**，不自己算 `axis/position/scale/padding`：
 * 那套变换（含运镜的 translate/scale）CSS 已经应用过一次，照它的包围盒画一遍即可。
 * 手持摇摆里那零点几度的 rotate 不会被复现 —— 一帧的误差看不出来。
 */
function snapshotStageFreeze() {
  const stage = stageRef.value
  const c = freezeRef.value
  if (!stage || !c) return false
  const box = stage.getBoundingClientRect()
  const w = Math.round(box.width)
  const h = Math.round(box.height)
  if (w <= 0 || h <= 0) return false
  if (c.width !== w || c.height !== h) { c.width = w; c.height = h }
  const ctx = c.getContext('2d')
  if (!ctx) return false
  ctx.setTransform(1, 0, 0, 1, 0, 0)
  ctx.filter = 'none'
  ctx.clearRect(0, 0, w, h)
  // Dhampir 模式：画面就是那块 canvas（本来就铺满舞台），照抄一张最省事也最准
  const dc = dhampirCanvasRef.value
  if (dhampirMode.value && dc && dc.width > 0) {
    try { ctx.drawImage(dc, 0, 0, w, h); return true } catch (e) { /* 画不出来就退到下面那条 */ }
  }
  ctx.fillStyle = stageBackdropColor()
  ctx.fillRect(0, 0, w, h)
  const paint = (v, blurPx) => {
    if (!v || !v.videoWidth) return
    const r = v.getBoundingClientRect()
    if (r.width <= 0 || r.height <= 0) return
    try {
      if (blurPx > 0) ctx.filter = 'blur(' + blurPx + 'px)'
      ctx.drawImage(v, r.left - box.left, r.top - box.top, r.width, r.height)
    } catch (e) { /* 没有可画的帧（seek 中）就只留底色 */ }
    ctx.filter = 'none'
  }
  if (bgOn.value) paint(bgRef.value, Number(stageFit.value.blur_px) || 28)
  paint(videoRef.value, 0)
  return true
}

/** 盖住画面。**必须在改 `currentTime` 之前调** —— 那时画面还是好的。 */
function beginSkipFreeze() {
  freezeStartedAt = performance.now()
  // 连续两段屏蔽（或上一张还没撤）：留着上一张，别拿正在 seek 的画面重拍
  if (!freezeOn.value && snapshotStageFreeze()) freezeOn.value = true
  window.clearTimeout(freezeTimer)
  freezeTimer = window.setTimeout(endSkipFreeze, SKIP_FREEZE_MAX_MS)
}

/** 撤遮罩（新帧上屏 / `seeked` 兜底 / 超时，谁先到算谁）。**这一刻才是剪辑点**：遮罩撤掉的
 *  同时把时钟推到视频的位置 —— 定格期间时钟是冻住的，否则字幕/弹幕会先跳到剪切后，
 *  而画面还定格在剪切前（"字先动、画后到"）。 */
function endSkipFreeze() {
  if (!freezeOn.value) return
  const held = performance.now() - freezeStartedAt
  if (held < SKIP_FREEZE_MIN_MS) {
    window.clearTimeout(freezeTimer)
    freezeTimer = window.setTimeout(endSkipFreeze, SKIP_FREEZE_MIN_MS - held)
    return
  }
  window.clearTimeout(freezeTimer)
  freezeTimer = 0
  freezeOn.value = false
  const v = videoRef.value
  if (!v) return
  current.value = v.currentTime
  emit('time', v.currentTime)
}

/**
 * 登记"新帧真的呈现了"这件事。
 *
 * `requestVideoFrameCallback` 的回调发生在**新帧交给合成器之后**，所以在这里撤遮罩一定接得上；
 * `seeked` 早一点（那时新帧只是解码好了、还没上屏），由它撤会露出黑的那一帧 —— 所以它只作兜底
 * （见 `onSeeked`）。回调时 `seeking` 还挂着 = 这一帧是 seek 之前的，重新登记一次。
 *
 * ⚠️ **Dhampir 模式还要多等一件事**（`wantFrame`）：那块画布是**引擎**画的，用的是**它自己**那套
 * `<video>` 源（`dhampirSourceVideos`）—— 与"DOM 时钟那一帧上屏"是**两套解码器、两个时刻**。
 * 只等 DOM 时钟的话，遮罩会在引擎的源就绪**之前**撤掉，而引擎这时先画的那一张是没有像素的
 * （画布露黑）—— 症状正是"Dhampir 预览跳屏蔽区还是黑一下"。
 * 判据用**引擎已经画到目标帧**（`dhampirFrameShown`，它在 `engine.seek()` 的 `.then()` 里更新），
 * 比"源都 readyState≥2"更直接：它证明的是"画布上已经是新位置那一帧"。
 * `wantFrame = null`（DOM 模式，或目标帧算不出来）时退回旧行为；`SKIP_FREEZE_MAX_MS` 仍是兜底。
 */
function armFreezeRelease(wantFrame = null) {
  const v = videoRef.value
  if (v && typeof v.requestVideoFrameCallback === 'function') {
    v.requestVideoFrameCallback(() => {
      // 遮罩已经被撤（`SKIP_FREEZE_MAX_MS` 超时 / `seeked` 兜底）且 seek 也停了：**别再登记** ——
      // 不复位的话这条 rvfc 链会跟着每一次上屏一直续下去（一个永不结束的回调）。
      if (!freezeOn.value && !v.seeking) return
      if (v.seeking || !freezeTargetDrawn(wantFrame)) { armFreezeRelease(wantFrame); return }
      endSkipFreeze()
    })
  }
}

/** 遮罩可以撤了吗：DOM 模式恒真（画面就是 `videoRef` 自己）；Dhampir 模式要引擎画到目标帧。 */
function freezeTargetDrawn(wantFrame) {
  if (wantFrame === null || !dhampirMode.value) return true
  return dhampirFrameShown >= wantFrame
}

// ---- DOM 那条腿的 A/B 预滚（2026-09-30）----
//
// 与 Dhampir 那条腿同一个思路（见 `dhampirTwins`）：**不要在"正在显示的那块"上喊 seek**。
// 备用那块（`standbyRef`，1px、仍在渲染树里 ⇒ 照旧解码）在进入屏蔽区前 `PREROLL_SECS`
// 就把区间终点 seek 好；到点**换台**（`activeStage` 翻一下，`videoRef` 是 computed 自动跟着走）
// —— 那一帧已经在解码器里，零等待、零黑帧，连定格遮罩都不需要。
let prerollArmedFor = null
let prerollTarget = 0
const prerollReady = ref(false)

/** 到点的时刻。与 `tick` 里那次跳**同一个算式**（预滚与跳错开一点点就会白备一趟）。 */
function skipCutTime(r) {
  const v = videoRef.value
  return Math.max(Math.min(v?.duration || (r.end + 0.01), r.end + 0.01), r.end)
}

/** 播放头之后最近的一段屏蔽区间。 */
function nextMuteRange(t) {
  return muteRanges.value.find((r) => r.start > t) || null
}

/** 把区间终点交给备用那块去 seek。同一段只做一次（每帧重设会把解码器一直打断）。 */
function armPreroll(r) {
  const standby = standbyRef.value
  if (!standby || !r) return
  const target = skipCutTime(r)
  if (prerollArmedFor === r && prerollTarget === target) return
  prerollArmedFor = r
  prerollTarget = target
  prerollReady.value = false
  try {
    standby.muted = true // 备用那块永远不出声（否则换台那一瞬间两路声音叠加）
    standby.currentTime = target
  } catch (e) { /* 元数据没到就下一帧再来 */ }
  if (!standby.paused) standby.pause() // 停在区间终点等换台；暂停态也会解码这一帧
}

function prerollReset() {
  prerollArmedFor = null
  prerollTarget = 0
  prerollReady.value = false
}

/** 备用那块 seek 完了 = 预滚就绪（只有当它停在我们要的那一刻才算）。 */
function onStandbySeeked() {
  const standby = standbyRef.value
  if (!standby || !prerollTarget) return
  if (standby.readyState >= 2 && Math.abs((standby.currentTime || 0) - prerollTarget) < 0.05) {
    prerollReady.value = true
  }
}

/** 两块 `<video>` 共用的 `seeked`：台上那块走原来的 `onSeeked`，台下那块只标记"预滚就绪"。 */
function onVideoSeeked(e) {
  const el = e && e.target
  if (el && el !== videoRef.value) { onStandbySeeked(); return }
  onSeeked()
}

/** 两块共用的 `play`/`pause`/`ended`：**只有台上那块**能改 UI 的播放态 ——
 *  台下的备用块被换台逻辑暂停时，不该让按钮/时间轴跟着抖。 */
function onStagePlay(e) {
  if (e && e.target === videoRef.value) setPlaying(true)
}
function onStagePause(e) {
  if (e && e.target === videoRef.value) setPlaying(false)
}

/**
 * 到点了：**换台**。备用那块已经在区间终点、帧也解出来了 —— 这一次跳没有任何等待。
 * 返回是否换成了（没备好就退回落定格的旧路）。
 */
function tryStageSwap(r) {
  if (dhampirMode.value) return false              // 画面是引擎画的，换 DOM 元素没用
  const standby = standbyRef.value
  const prev = videoRef.value
  if (!standby || !prev || standby === prev) return false
  const target = skipCutTime(r)
  if (standby.seeking || standby.readyState < 2) return false
  // 就绪的判据：`seeked` 标记过，或**位置本来就在那儿**（目标与它当时的位置重合时浏览器
  // 可能不发 `seeked`，而那一帧本来就在 —— 只看标记会让 A/B 白白不生效）。
  const nearTarget = Math.abs((standby.currentTime || 0) - target) <= 0.08
  if (!prerollReady.value && !nearTarget) return false
  try {
    standby.volume = prev.volume
    standby.muted = !!muted.value
  } catch (e) { /* 忽略 */ }
  // **先换台、再暂停旧那块**：换台之后 `videoRef` 已经是新的了，旧那块随后的 `pause` 事件
  // 不会被 `onStagePause` 当成"用户按了暂停"（否则按钮会闪一下"播放"、时间轴也跟着抖）。
  activeStage.value = 1 - activeStage.value        // ★ `videoRef` 是 computed，读它的地方全体跟随
  prev.pause()
  standby.play().catch((e) => {
    // 自动播放被拒（本页已有用户手势，罕见）：退回静音播放 —— 宁可没声音，也别整段不动还没提示
    try { standby.muted = true } catch (e2) { /* 忽略 */ }
    standby.play().catch(() => {})
    console.warn('preview: A/B 换台后 play() 被拒，已退回静音：' + ((e && e.message) || e))
  })
  if (bgOn.value && bgRef.value) bgRef.value.currentTime = standby.currentTime
  current.value = standby.currentTime
  player.seek(current.value)
  emit('time', standby.currentTime)
  prerollReset()                                    // 下一帧会给"新的备用那块"备下一段
  return true
}

// ---- 播放控制 ----
function tick() {
  const v = videoRef.value
  if (!v) return
  // **Dhampir 模式：让出"重活"，但时钟照发。**
  //
  // 让出的是那套 DOM/CSS 复刻（字幕/弹幕/贴纸 + 每帧写样式、触发整棵树重渲染）——
  // 它在这时候**整块被 canvas 盖住**，继续跑只是白烧 CPU，而且它和 Dhampir 的画布
  // 循环都是 60Hz，两边抢起来实测把 draw 从 0.9ms 抬到 **9ms**、整体掉到 **15fps**。
  //
  // ⚠️ **但 `current` 与 `emit('time')` 不能一起停掉** —— 这是踩过的坑：
  // 时间轴的"跟随播放头"、工具条上的时间读数、编辑器的播放头**全靠这一个事件**，
  // 停了它们的症状是"播放时时间轴不动、控件也不动"，而画面本身是好的（所以很难归因）。
  // 下面这两行是那个 bug 的修复：让位只让渲染，不让时钟。
  if (dhampirMode.value) {
    // **屏蔽区间照样要跳。** 它就是"预览跳过手动屏蔽的时间段"这件事本身；
    // 少了它，Dhampir 模式会一路播过本该跳掉的区间 —— 时间轴与画面对不上，
    // 而且屏蔽段的内容本来就不该出现在预览里。
    // **这里不能再用 `muteRangeAt`。** 它读的是 `polish.toml` 的 `mute_ranges`
    // （**源**时间上的说法），而工程的时间轴已经把那些段**掐掉了** ——
    // 在压缩工程上按它再跳一次，等于跳两次（我上一版就是这么错的）。
    // 正确的说法是"按段落表算帧号"：落在被掐掉的源区间里就没有对应帧。
    if (!v.paused && !v.ended) {
      // **A/B 预滚**：往前看一段时间，马上要切就把"下一段起点"在备用那套源上先定位好
      const ahead = dhampirPrerollTarget(v.currentTime)
      if (ahead !== null) dhampirArmPreroll(ahead)
    }
    if (!v.paused && !v.ended && dhampirFrameAt(v.currentTime) === null) {
      const target = dhampirNextSourceStart(v.currentTime)
      if (target !== null) {
        // ① **A/B 命中**：备用那套源已经在下一段起点、帧也解好了 ⇒ 换绑，引擎那一帧立刻有像素。
        //    画布这边不需要遮罩（下一帧就会画出正确的画面），DOM 时钟这次 seek 在画布后面看不见。
        if (dhampirSwapSources()) {
          v.currentTime = target
          syncDhampirTime(target)
          current.value = v.currentTime
          emit('time', v.currentTime)
          if (bgOn.value && bgRef.value) bgRef.value.currentTime = target
          rafId = requestAnimationFrame(tick)
          return
        }
        // ② 没备好（刚拖过进度条 / 区间太近 / 源挂了）：回落定格遮罩 + 引擎"源不就绪不画"
        //    （两条一起才不露黑：遮罩挡住 DOM 那条腿，画布自己保留上一帧）
        beginSkipFreeze()
        v.currentTime = target
        syncDhampirTime(target)
        // 撤遮罩要等**引擎画到目标帧**，不只是 DOM 时钟上屏（两套解码器，见 `armFreezeRelease`）
        armFreezeRelease(dhampirFrameAt(target))
        if (bgOn.value && bgRef.value) bgRef.value.currentTime = target
        // **时钟不在这里推**：定格期间画面还停在剪切前，字幕/弹幕先跳到剪切后就是"字先动、画后到"。
        // 推时钟的时机 = `endSkipFreeze()`（新帧上屏的那一刻）。
        rafId = requestAnimationFrame(tick)
        return
      }
    }
    // 定格期间：画面与时钟一起停（`endSkipFreeze` 里推）
    if (freezeOn.value) { rafId = requestAnimationFrame(tick); return }
    current.value = v.currentTime
    emit('time', v.currentTime)
    if (!v.paused && !v.ended) rafId = requestAnimationFrame(tick)
    return
  }
  // 预览时跳过手动屏蔽区间
  if (!v.paused && !v.ended) {
    // **A/B 预滚**：还没进区间就先让备用那块把区间终点准备好（进入前 PREROLL_SECS 之内）
    const upcoming = nextMuteRange(v.currentTime)
    if (upcoming && upcoming.start - v.currentTime <= PREROLL_SECS) armPreroll(upcoming)
    // `SKIP_LOOKAHEAD`：提前一帧认定，剪辑点才落在区间起点上（见常量处的说明）
    const r = muteRangeAt(v.currentTime, SKIP_LOOKAHEAD)
    if (r) {
      // ① 预滚命中 → **换台**：那一帧已经在解码器里，零等待、零黑帧（遮罩都不用上）
      if (tryStageSwap(r)) {
        rafId = requestAnimationFrame(tick)
        return
      }
      // ② 没备好（刚拖过进度条 / 区间太近）→ 回落定格的旧路
      const target = Math.min(v.duration || (r.end + 0.01), r.end + 0.01)
      // 定格遮罩：**先拍再跳**（seek 之后拍到的就是黑的那一帧了）
      beginSkipFreeze()
      v.currentTime = Math.max(target, r.end)
      syncDhampirTime(v.currentTime)
      armFreezeRelease()
      if (bgOn.value && bgRef.value) bgRef.value.currentTime = v.currentTime
      // 时钟同 Dhampir 那条：不在这里推，交给 `endSkipFreeze()`
      rafId = requestAnimationFrame(tick)
      return
    }
  }
  // 定格期间：画面与时钟一起停（`endSkipFreeze` 里推）
  if (freezeOn.value) { rafId = requestAnimationFrame(tick); return }
  current.value = v.currentTime
  player.sync(idx.value.soundEvents, v.currentTime)
  emit('time', v.currentTime)
  if (bgOn.value && bgRef.value && bgRef.value.currentTime &&
      Math.abs(bgRef.value.currentTime - v.currentTime) > 0.05) {
    bgRef.value.currentTime = v.currentTime
  }
  if (!v.paused && !v.ended) rafId = requestAnimationFrame(tick)
}

function togglePlay() {
  const v = videoRef.value
  if (!v) return
  if (v.paused) {
    if (bgOn.value && bgRef.value) bgRef.value.play().catch(() => {})
    v.play().catch(() => {})
    setPlaying(true)
    cancelAnimationFrame(rafId)
    rafId = requestAnimationFrame(tick)
  } else {
    v.pause()
    if (bgOn.value && bgRef.value) bgRef.value.pause()
    setPlaying(false)
    cancelAnimationFrame(rafId)
  }
}

/** 播放状态往外发一份：时间轴的"跟随播放头"只在**正在播放**时生效
 *  （用户要求：拖动/跳转不该自己动视角，只有播放时才跟）。 */
function setPlaying(v) {
  const on = !!v
  if (playing.value === on) return
  playing.value = on
  emit('playing', on)
}

function seekTo(t) {
  const v = videoRef.value
  if (!v) return
  v.currentTime = Number(t) || 0
  syncDhampirTime(v.currentTime)
  current.value = v.currentTime
  player.seek(current.value)
  prerollReset()          // 手动拖动之后那趟预滚的目标就作废了（下一帧会按新位置重新armed）
  if (bgOn.value && bgRef.value) bgRef.value.currentTime = v.currentTime
  if (duration.value) emit('time', v.currentTime)
}

function onSeekInput(e) {
  const p = Number(e.target.value) || 0
  if (videoRef.value && duration.value > 0) seekTo(duration.value * p / 1000)
}

function onMeta() {
  const v = videoRef.value
  if (!v) return
  duration.value = v.duration || 0
  prerollReset() // 换源/换片段之后那一趟预滚的目标就作废了（两块元素都会各触发一次，无副作用）
  // 切片段/换源后把音量与静音重新落到视频元素上（元素属性虽会保留，这里保证与控件状态一致）
  applyAudio()
  // seek 触发首帧解码，避免打开页面时舞台全黑
  if (v.currentTime === 0) v.currentTime = 0.001
  if (bgOn.value && bgRef.value) {
    if (bgRef.value.currentTime === 0) bgRef.value.currentTime = 0.001
    if (Math.abs((bgRef.value.currentTime || 0) - v.currentTime) > 0.05) {
      bgRef.value.currentTime = v.currentTime
    }
  }
}

// 任何来源的 seek（进度条/代码/原生控制条）都同步覆盖层时钟
function onSeeked() {
  const v = videoRef.value
  if (!v) return
  syncDhampirTime(v.currentTime)
  // 定格期间**不推时钟**：画面还停在剪切前，推了就是"字先动、画后到"（`endSkipFreeze` 里推）
  if (!freezeOn.value) current.value = v.currentTime
  player.seek(v.currentTime)
  if (bgOn.value && bgRef.value) bgRef.value.currentTime = v.currentTime
  // 跳屏蔽区间的定格遮罩：没有 `requestVideoFrameCallback` 的浏览器退到"seeked 之后再等两帧"
  // （seeked 早于上屏，直接撤会露出黑的那一帧）；有它的那条路在 `armFreezeRelease` 里。
  if (freezeOn.value && typeof v.requestVideoFrameCallback !== 'function') {
    requestAnimationFrame(() => requestAnimationFrame(endSkipFreeze))
  }
}

/**
 * 音量/静音同时作用在**视频本身**和 SFX 层。
 * 之前只调 SFX 播放器，视频里的说话声/音乐丝毫不受影响 —— 用户反馈的「音量控制不好使」就是这个：
 * 点静音、拉滑块都对真正在响的那一路没作用。
 */
function applyAudio() {
  const v = videoRef.value
  if (!v) return
  const n = Number(volume.value)
  v.volume = Number.isFinite(n) ? Math.min(1, Math.max(0, n)) : 0
  v.muted = !!muted.value
}

function toggleMute() {
  if (silenced.value) {
    // 当前没声音 → 恢复（音量被拉到 0 时抬回上次的非零值，否则按钮看着是「恢复了」却依旧静音）
    muted.value = false
    player.setMuted(false)
    if (volume.value <= 0) {
      volume.value = lastVol
      player.setMasterVolume(lastVol)
    }
  } else {
    muted.value = true
    player.setMuted(true)
  }
  applyAudio()
}

function onVolume(e) {
  const v = Number(e.target.value)
  volume.value = Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0
  if (volume.value > 0) {
    lastVol = volume.value
    if (muted.value) { muted.value = false; player.setMuted(false) }
  }
  player.setMasterVolume(volume.value)
  applyAudio()
}
function fullscreen() {
  if (stageRef.value) stageRef.value.requestFullscreen?.()
}

// ---- 框选布局：整帧百分比矩形（拖框体 = 平移，拖 8 个角/边 = 缩放）----
const SEL_HANDLES = ['nw', 'n', 'ne', 'w', 'e', 'sw', 's', 'se']
const SEL_MIN = 2 // 最小边长（百分比），防止拖成一条线
let selDrag = null

const r2 = (v) => Math.round(v * 100) / 100

/** 选框兜底：越界裁剪 + 顺序化 + 最小尺寸（父级数据异常时不至于画出负宽盒子）。 */
function selNorm(r) {
  const n = (v, d) => { const x = parseFloat(v); return Number.isFinite(x) ? Math.min(100, Math.max(0, x)) : d }
  let x0 = n(r?.x0, 0), y0 = n(r?.y0, 0), x1 = n(r?.x1, 100), y1 = n(r?.y1, 100)
  if (x1 < x0) [x0, x1] = [x1, x0]
  if (y1 < y0) [y0, y1] = [y1, y0]
  if (x1 - x0 < 0.5) x1 = Math.min(100, x0 + 0.5)
  if (y1 - y0 < 0.5) y1 = Math.min(100, y0 + 0.5)
  return { x0, y0, x1, y1 }
}

const selBoxStyle = computed(() => {
  const r = selNorm(props.selectRect)
  return {
    left: r.x0 + '%',
    top: r.y0 + '%',
    width: (r.x1 - r.x0) + '%',
    height: (r.y1 - r.y0) + '%'
  }
})

/** 指针位置 → 舞台（= 画布整帧）百分比坐标。 */
function pctAt(e) {
  const el = stageRef.value
  if (!el) return null
  const b = el.getBoundingClientRect()
  if (!b.width || !b.height) return null
  return {
    x: Math.min(100, Math.max(0, ((e.clientX - b.left) / b.width) * 100)),
    y: Math.min(100, Math.max(0, ((e.clientY - b.top) / b.height) * 100))
  }
}

function startSel(handle, e) {
  const p = pctAt(e)
  if (!p) return
  const r0 = handle === 'new' ? { x0: p.x, y0: p.y, x1: p.x, y1: p.y } : selNorm(props.selectRect)
  selDrag = { handle, sx: p.x, sy: p.y, r0 }
  if (handle === 'new') emitRect(r0)
  window.addEventListener('pointermove', onSelMove)
  window.addEventListener('pointerup', endSel)
  window.addEventListener('pointercancel', endSel)
}

function emitRect(r) {
  emit('update:selectRect', { x0: r2(r.x0), y0: r2(r.y0), x1: r2(r.x1), y1: r2(r.y1) })
}

function onSelMove(e) {
  if (!selDrag) return
  // 指针在窗口外松开（收不到 pointerup）时用 buttons=0 兜底收尾，避免选框跟着鼠标跑
  if (e.buttons === 0) { endSel(); return }
  const p = pctAt(e)
  if (!p) return
  const { handle, sx, sy, r0 } = selDrag
  const dx = p.x - sx
  const dy = p.y - sy
  let { x0, y0, x1, y1 } = r0

  if (handle === 'new') {
    x0 = Math.min(sx, p.x); x1 = Math.max(sx, p.x)
    y0 = Math.min(sy, p.y); y1 = Math.max(sy, p.y)
    if (x1 - x0 < SEL_MIN) x1 = Math.min(100, x0 + SEL_MIN)
    if (y1 - y0 < SEL_MIN) y1 = Math.min(100, y0 + SEL_MIN)
  } else if (handle === 'move') {
    const w = x1 - x0, h = y1 - y0
    x0 = Math.min(Math.max(0, x0 + dx), 100 - w)
    y0 = Math.min(Math.max(0, y0 + dy), 100 - h)
    x1 = x0 + w; y1 = y0 + h
  } else {
    if (handle.includes('w')) x0 = Math.min(Math.max(0, r0.x0 + dx), x1 - SEL_MIN)
    if (handle.includes('e')) x1 = Math.max(Math.min(100, r0.x1 + dx), x0 + SEL_MIN)
    if (handle.includes('n')) y0 = Math.min(Math.max(0, r0.y0 + dy), y1 - SEL_MIN)
    if (handle.includes('s')) y1 = Math.max(Math.min(100, r0.y1 + dy), y0 + SEL_MIN)
  }
  emitRect({ x0, y0, x1, y1 })
}

function endSel() {
  selDrag = null
  window.removeEventListener('pointermove', onSelMove)
  window.removeEventListener('pointerup', endSel)
  window.removeEventListener('pointercancel', endSel)
}

// 退出框选模式时收尾（父级可能在拖拽中被卸载/切换）
watch(() => props.selectMode, (on) => { if (!on) endSel() })

// ---- 点选落点（贴纸「固定」模式）----
// 与框选共用 `pctAt`（同一套"指针 → 整帧百分比"换算），只是取一个点而不是拖矩形。
//
// ⚠️ `[sticker] position`（manual）= **贴纸盒子的中心**（`config.rs` 的 `StickerConfig.position`
// 注释 / 设置页口径）；auto 的事件 x/y 才是盒子左上角 —— 两个键、两种基准，不要"统一"。
// 所以这里画的是**以锚点为中心的贴纸真实占位**（边长 280×短边倍率、边距 32×短边倍率），
// 并做出片同样的出框 clamp；上浮量不在这里画（那是动画，且 clamp 之后才叠加）。
const pointBoxStyle = computed(() => {
  const c = cv.value
  const size = 280 * stickerScale(c)
  const margin = 32 * stickerScale(c)
  const p = props.point
  const nx = Number(p?.x), ny = Number(p?.y)
  const px = (Number.isFinite(nx) ? Math.min(100, Math.max(0, nx)) : 50) / 100 * c.W
  const py = (Number.isFinite(ny) ? Math.min(100, Math.max(0, ny)) : 50) / 100 * c.H
  // 与出片同一个 clamp（index.html::stickerBox / engine.js::clampSticker）
  const x = Math.max(margin, Math.min(c.W - size - margin, px - size / 2))
  const y = Math.max(margin, Math.min(c.H - size - margin, py - size / 2))
  // 舞台是按 CSS 缩放的，换算成舞台内的像素；overlayScale = 舞台宽 / 画布宽
  const s = overlayScale.value || 1
  return { left: x * s + 'px', top: y * s + 'px', width: size * s + 'px', height: size * s + 'px' }
})

function pickPoint(e) {
  const p = pctAt(e)
  if (!p) return
  emit('update:point', { x: r2(p.x), y: r2(p.y) })
}

// ---- 真渲染预览：**整条已删除** ----
// 这里曾经有两条真渲染通道：
//   ① wasm 实时模式（`webui/src/wasm/` 的 wasm-bindgen 产物 + `init_renderer()`，
//      浏览器 WebGPU 里跑 `vtrim-render-core` 的合成管线，逐帧 `render_frame`）；
//   ② 回退的 server 单帧（`POST /api/polish/render-frame`，桌面 wgpu 后端渲染一帧）。
// 两条都随「作废 wgpu」一起删除（用户拍板：GPU 那侧交给独立的 Dhampir 底座）。
// 预览的事实来源始终是 `polish/engine.js` 的自绘 DOM/GSAP 层 —— 与出片模板
// `templates/index.html` 的 canvas 路径同口径。**别再把这套接回来**：要真渲染预览就走底座那条链路。
watch(idx, (v) => {
  player.setEvents(v.soundEvents)
  player.setLoudnessCurve(props.config?.loudness_curve || [])
}, { immediate: true })

// ===== 默认进 Dhampir（用户要求：默认为 dhampir；旧预览暂时禁用）=====
// 另起一个 onMounted（Vue 允许多个 ✓）：挂载后自动切一次，失败由 toggleDhampir 的 catch 接住 ✓
onMounted(() => {
  setTimeout(() => {
    if (!dhampirMode.value && !dhampirError.value) toggleDhampir()
  }, 300)
})

onMounted(() => {
  volume.value = player.getMasterVolume()
  muted.value = player.isMuted()
  lastVol = volume.value > 0 ? volume.value : 1
  applyAudio()
  ro = new ResizeObserver(() => {
    const w = stageRef.value ? stageRef.value.clientWidth : 0
    if (w) overlayScale.value = w / cv.value.W
  })
  if (stageRef.value) ro.observe(stageRef.value)
})
onUnmounted(() => {
  cancelAnimationFrame(rafId)
  window.clearTimeout(freezeTimer)
  freezeOn.value = false
  if (ro) ro.disconnect()
  endSel()
  // Dhampir 预览：停 rAF、摘掉那些 <video>（解码器与网络请求都要放掉）。
  stopDhampir()
})

// 字幕字体：选了程序级字体库里的字体时，浏览器得先认得它（@font-face）才会用 ——
// 出片那条路是 render/server.rs 注入的，这里是**同一套装在 DOM 预览上**，
// 少了它预览还是系统字体、出片却换了字体。字体加载完就自然重排，不用手动重挂。
// `dir` 在精修页是绝对路径（服务端按它反查归属），所以这里不用再传 project。
watch(() => [props.dir, subStyle.value.font], ([dir, font]) => {
  if (font) ensureFontFace(null, dir, font, { onDebug: (m) => addClient('debug', m, 'preview') })
}, { immediate: true })

defineExpose({ seekTo, togglePlay, pause })

/** 供父级（框选布局等）定格画面用：正在播放则暂停。 */
function pause() {
  const v = videoRef.value
  if (v && !v.paused) togglePlay()
}
</script>

<style scoped>
/* Dhampir 实时预览：画布盖在 DOM 预览之上（z-index 高于它、低于框选/点选那两层）。 */
.dhampir-canvas {
  position: absolute; inset: 0; width: 100%; height: 100%;
  object-fit: contain; background: #000; z-index: 40;
}
/* 素材宿主。
 *
 * # `<video>` 这一半
 *
 * **不用 `display:none`** —— 隐藏的 video 在部分浏览器里会停止解码，
 * 而 seek 的代价正是解码。1px + `overflow:hidden` 就够（仍在渲染树里）。
 *
 * # `<img>`（贴纸 GIF）这一半 —— 这里踩过两次
 *
 * 浏览器**不保证**给"不可见"的图推进 GIF 动画帧，而宿主每帧 `drawImage(img)`
 * 取的就是"它当前显示的那一帧"。两次都错在"怎么才算可见"上：
 *
 *   1. `opacity: 0` + 1px + 裁掉 → **不推进**：症状是贴纸入场动效正常（那是引擎按
 *      keyframes 算的）但 GIF 永远是同一帧；
 *   2. 铺满 + 压在 canvas 下面 → **还是不推进**：图被 canvas **完全遮住**，
 *      浏览器做**遮挡剔除**（occluded），照样不喂它动画帧。
 *
 * 正确的组合是：**被真的绘制、又不被任何东西遮住**。
 * 好在 `drawImage` 取的是**图像本身的像素**，与元素的显示尺寸无关 ——
 * 所以下面把贴纸图做成 **1×1、盖在 canvas 之上**（看不见，但确实在画），
 * 拿到的仍是 500×500 的完整帧。宿主自身的 z-index 必须高于 canvas（40），
 * 否则定位成 `fixed` 的子元素仍会被关在宿主的层叠上下文里、重新被遮住。
 */
.dhampir-hosts {
  position: absolute; left: 0; top: 0; width: 1px; height: 1px;
  opacity: 1; pointer-events: none; overflow: hidden; z-index: 50;
}
.dhampir-hosts img {
  position: fixed; right: 0; bottom: 0; width: 1px; height: 1px;
  opacity: 1; pointer-events: none;
}
.dhampir-bar {
  display: flex; align-items: center; gap: 8px; margin-top: 10px; flex-wrap: wrap;
}
.polish-preview { width:100%; }
/* 预览尺寸：宽度由「可用高度」反推（16:9），并随视口收缩 —— 目标是最大化窗口下
   预览尽量大、时间轴基本同屏。540px ≈ 其余固定部分之和（面包屑 66 + 内容 padding 18 +
   紧凑页头 ~38 + 片段卡条 1 行 ~105 + 编辑区上边距 16 + 预览卡头/控制条 ~93 + 左列间距 16
   + 时间轴卡 ~259 ≈ 611），所以底部会留 ~70px 在首屏外（时间轴第 7 条泳道要小滚一下）；
   想让它完整可见就把 540 调回 610。上限 620 免得大屏上预览过分占位，下限 170。
   —— 用户要求：预览再放大一点。 */
/* 预览舞台比例跟随 `_canvas`（--stage-ar = W/H；--stage-ratio = W/H 数值，是"高度预算→宽度"的乘数：
   clamp 是高度预算，横屏 ×16/9、竖屏 ×9/16 —— 两个方向的舞台**同高**）。 */
.stage { position:relative; width:min(100%, calc(clamp(170px, 100vh - 540px, 620px) * var(--stage-ratio, 1.7778)));
  aspect-ratio:var(--stage-ar, 16/9); margin:0 auto; background:#000; border-radius:14px; overflow:hidden;
  box-shadow:0 6px 18px rgba(0,0,0,.10); }
.stage:fullscreen { width:100%; border-radius:0; }
.bg-video { position:absolute; inset:0; width:100%; height:100%; object-fit:cover;
  filter:blur(var(--bg-blur, 28px)) brightness(0.5) saturate(1.15);
  transform:scale(var(--bg-img-scale, 1.187)); max-height:none; margin:0; border-radius:0; }
/* 画面构成（`_canvas.stage` 驱动）：主画面按比例放进 .fit-box，留边由背景层/舞台底色补 */
.video-wrap { position:absolute; inset:0; display:flex; align-items:center; justify-content:center;
  overflow:hidden; padding:var(--stage-pad, 0%); }
.fit-box { position:relative; transform-origin:center center; transform:scale(var(--stage-scale, 1)); }
.video-wrap.axis-fill .fit-box { width:100%; height:100%; }
.video-wrap.axis-x .fit-box { height:100%; max-width:100%; aspect-ratio:var(--src-ar, 16 / 9); }
.video-wrap.axis-y .fit-box { width:100%; max-height:100%; aspect-ratio:var(--src-ar, 16 / 9); }
.video-wrap.stage-center { justify-content:center; align-items:center; }
.video-wrap.stage-left { justify-content:flex-start; }
.video-wrap.stage-right { justify-content:flex-end; }
.video-wrap.stage-top { align-items:flex-start; }
.video-wrap.stage-bottom { align-items:flex-end; }
.source-video { width:100%; height:100%; max-height:none; margin:0; border-radius:0; object-fit:cover; will-change:transform,filter; }
/* A/B 双缓冲里**台下那块**（备用）：1px 仍留在渲染树里 —— `display:none` 的 video 会停止解码，
   而"预滚"要的正是解码。见模板里那段说明。 */
.source-video-standby { position:absolute; left:0; top:0; width:1px; height:1px; opacity:1; pointer-events:none; }
.corner-glow { position:absolute; inset:0; z-index:1; pointer-events:none; opacity:0; transition:opacity 1s; }
.corner-glow .gb { position:absolute; bottom:-60px; right:-60px; width:260px; height:260px; border-radius:50%; background:radial-gradient(circle,rgba(0,212,255,0.12),transparent 70%); filter:blur(50px); }
.stage:hover .corner-glow { opacity:1; }
/* ⚠️ `z-index:6` = **压在定格遮罩（5）之上、覆盖层（10）之下**。
   遮罩拍的是"视频像素"（快照里没有这几层 CSS 特效），不抬到 6 的话，跳屏蔽区间那 200~700ms 里
   flash / color / hue / vignette / noise 会**整块消失再回来** —— 又是一次眨眼。
   它们的值都由被冻住的 `current` 算出来（定格期间不变），所以实时画在上面与拍进快照等价。
   正常播放时没有任何视觉变化：仍然在视频之上、字幕/弹幕/贴纸之下。 */
.fx { position:absolute; inset:0; pointer-events:none; z-index:6; }
.fx-noise { background:#fff; }
.fx-color { opacity:0; }

/* 虚拟 overlay（默认 1920×1080，随 `_canvas` 由 --ov-w/--ov-h 代入）：top:0 left:0 后由 scale 定位中心 */
.overlay-space { position:absolute; top:50%; left:50%; width:var(--ov-w, 1920px); height:var(--ov-h, 1080px); transform-origin:center center; pointer-events:none; z-index:10; }
/* 定格遮罩：画布 1 像素 = 1 CSS 像素（坐标照 `getBoundingClientRect` 的相对位置画，不乘 DPR）。
   `inset:0` 铺满舞台 —— 连模糊底与黑边一起冻住，seek 期间舞台不会有第二处闪黑。 */
.freeze-frame { position:absolute; inset:0; width:100%; height:100%; pointer-events:none; }
.sticker-layer { position:absolute; inset:0; overflow:hidden; }
.sticker-el { position:absolute; width:var(--sticker-s, 280px); height:var(--sticker-s, 280px); display:flex; align-items:center; justify-content:center; }
.sticker-el img { width:100%; height:100%; object-fit:contain; }
.sticker-emoji { font-size:var(--sticker-emoji, 200px); line-height:var(--sticker-s, 280px); text-align:center; font-family:'Noto Color Emoji','Apple Color Emoji','Segoe UI Emoji',sans-serif; }
.danmaku-layer { position:absolute; top:0; left:0; width:var(--ov-w, 1920px); height:var(--dm-h, 540px); overflow:hidden; }
.dm { position:absolute; white-space:nowrap; font-weight:600; -webkit-text-stroke:2px #000; paint-order:stroke fill; }
.captions { position:absolute; bottom:var(--cap-bottom, 80px); left:var(--cap-left, 120px); width:var(--cap-w, 1680px); }
.cap { position:relative; text-align:center; padding:14px 0; font-weight:700; line-height:1.5;
       -webkit-text-stroke:12px #403c3b; paint-order:stroke fill; }
/* A5 折行：每行一个块级 nowrap 行 —— 断行由 wrapCaptionParts 决定（与出片一致） */
.cap-line { display:block; white-space:nowrap; }
.cap .hl { color:var(--hl-color, #f56b41); }

/* ---- 框选布局覆盖层：盒子外用超大 box-shadow 压暗，突出选框 ---- */
.sel-layer { position:absolute; inset:0; z-index:70; cursor:crosshair; touch-action:none; }
.sel-box { position:absolute; border:1.5px solid var(--main-deep); cursor:move;
  box-shadow:0 0 0 9999px rgba(0,0,0,.42); }
.sel-tag { position:absolute; top:4px; left:6px; padding:2px 10px; border-radius:var(--r-full);
  background:var(--main-soft); color:var(--pink-text); font-size:11px; white-space:nowrap; }
.sel-h { position:absolute; width:12px; height:12px; border-radius:3px; background:#fff;
  border:1.5px solid var(--main-deep); }
.h-nw { left:-6px; top:-6px; cursor:nwse-resize; }
.h-n { left:50%; top:-6px; margin-left:-6px; cursor:ns-resize; }
.h-ne { right:-6px; top:-6px; cursor:nesw-resize; }
.h-w { left:-6px; top:50%; margin-top:-6px; cursor:ew-resize; }
.h-e { right:-6px; top:50%; margin-top:-6px; cursor:ew-resize; }
.h-sw { left:-6px; bottom:-6px; cursor:nesw-resize; }
.h-s { left:50%; bottom:-6px; margin-left:-6px; cursor:ns-resize; }
.h-se { right:-6px; bottom:-6px; cursor:nwse-resize; }

/* ---- 点选落点覆盖层（贴纸「固定」）：**不压暗**（要能看清画面挑落点），
   只给一个十字落点 + 中心提示；光标变十字表示可点 ---- */
.point-layer { position:absolute; inset:0; z-index:70; cursor:crosshair; touch-action:none; }
.point-tag { position:absolute; top:6px; left:8px; padding:2px 10px; border-radius:var(--r-full);
  background:var(--main-soft); color:var(--pink-text); font-size:11px; white-space:nowrap; }
/* 贴纸真实占位框（边长 280×短边倍率，**以锚点为中心**）：虚线框 + 中心一个点，便于对齐 */
.point-box { position:absolute; border:1.5px dashed var(--main-deep); background:rgba(255,255,255,.12);
  border-radius:4px; pointer-events:none; box-sizing:border-box; }
.point-dot { position:absolute; left:50%; top:50%; width:12px; height:12px; margin:-6px 0 0 -6px;
  border-radius:50%; background:#fff; border:2px solid var(--main-deep); }

.preview-controls { display:flex; align-items:center; gap:8px; margin-top:10px; }
.time-label { min-width:96px; text-align:right; }
.vol { width:80px; flex:none; padding:0; border:none; background:transparent; }
/* 音量的滑块外观统一由 app.css 的 input[type=range] 提供（这里不再覆盖，避免两套外观） */
button:disabled { opacity:.45; cursor:not-allowed; }
</style>
