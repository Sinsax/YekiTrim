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
        <div class="fit-box" ref="fitBoxRef" :style="fitBoxStyle">
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
      <!-- **Dhampir 首次加载的门帘**（2026-10-02）：引擎 / 贴纸动图 / 字幕都是**异步**到位的，
           半途播出来的是「缺效果」甚至空白的画面 —— 而它长得就像「这一版就是这样」。所以加载期间：
           ① 掀这块门帘（说清在等什么）② 播放按钮置灰 ③ `togglePlay` 里再挡一道（父级/快捷键也走它）。
           只在**首次加载**这一趟挂（`dhampirLoading`）；热重载不挂 —— 那时画面仍是上一版、引擎还在画。 -->
      <!-- ⚠️ 还要 `&& dhampirMode`：引擎加载卡住时用户仍能再点一次「Dhampir」退出模式，
           那时门帘必须跟着消失 —— 只认 `dhampirLoading` 的话它会一直留在台上。 -->
      <div v-if="dhampirLoading && dhampirMode" class="dhampir-loading">
        <span class="dl-spin"></span>
        <span class="dl-text">{{ dhampirStatus || '正在加载 Dhampir 预览…' }}</span>
        <span class="muted small" title="特效/贴纸还没就绪">加载完成前不可播放</span>
      </div>
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
      <button class="mini-btn" :disabled="dhampirLoading" @click="togglePlay"
              :title="dhampirLoading ? 'Dhampir 正在加载，加载完成前不能播放' : '播放 / 暂停'">
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
// §4「转译报告可见」：DROP/CLAMP 的播报只有 `api/polish.js` 那一份实现，三处调用点共用。
import { logTranslateReport } from '../api/polish'
import { buildIndex, videoFxAt, handheldSway, getFlash, getHueShiftFlash, getNoiseOpacity, getVignette, getOverlayState, gradientCss, subsAt, danmakuAt, stickersAt, wrapCaptionParts, capMeasure } from '../polish/engine'
import { canvasOf, stickerScale } from '../polish/canvas.js'
import { createSfxPlayer } from '../polish/audio'
import { ensureFontFace } from '../polish/fonts'

const props = defineProps({
  pid: { type: [String, Number], required: true },
  dir: { type: String, required: true },
  config: { type: Object, default: null },
  /**
   * 「磁盘上那份配置」——与 `config` 的区别就是**它已经落盘了**。
   *
   * `config` 是页面手里的**镜像**：编辑器拖动时会 60ms 节流地把它换成内存里的中间态
   * （`onTimelinePreview`，**不落盘**）。而 Dhampir 那条腿的工程是从**磁盘 `polish.toml`
   * 转译**出来的 —— 拿镜像的变化去触发重载，只会读到还没写下去的内容，
   * 表现是"拖完松手，画面先闪回改动前的样子"。
   *
   * 所以页面在**落盘完成**（以及任务跑完 `load()`）时更新它；热重载只认它。
   */
  persistedConfig: { type: Object, default: null },
  subtitles: { type: Array, default: () => [] },
  /**
   * 「磁盘上那份字幕」——**只改字幕时唯一会变的信号**：字幕不在 `polish.toml` 里
   * （住 `data/subtitles.json`），所以下面那个重载指纹里的配置子树一个字都不会动。
   * 没有它，改完字幕预览不刷新（用户报过）。
   */
  persistedSubtitles: { type: Array, default: null },
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
  fontFallback: { type: String, default: '"Noto Sans SC","Microsoft YaHei",sans-serif' },
  // ⚠️ **emoji 补充槽位**（2026-10-02 字体职责分离）：库里那支负责表情的字体（服务端自动选、
  // 至多一支），空串 = 库里没有 ⇒ 不追加（浏览器自带系统 emoji）。
  // 它是**独立 prop**、不并进 fontFallback —— 服务端就是这么分的（见 api/polish.js 的说明）。
  emojiFamily: { type: String, default: '' }
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
/**
 * **首次加载 Dhampir 的门帘**（2026-10-02）——`startDhampir` 跑着的那段就是 `true`。
 *
 * 为什么要有它：引擎起来、贴纸动图 `dhampirLoadAnimations`、字幕 `dhampirApplySubtitles`
 * 三件事都是**异步**的，而 `dhampirMode` 在 `await` **之前**就翻了真 —— 于是加载期间用户能按播放，
 * 看到的是「缺效果 / 空白」的画面；加载**失败**时更容易被当成「这一版就是这样」（实际只是近似预览）。
 * 所以：加载期间**不许播**（模板按钮置灰 + `togglePlay` 里挡一道），并盖一层门帘说清在等什么。
 *
 * ⚠️ **只盖首次加载**，不盖 `reloadDhampirProject`（热重载）：那时引擎已经起来、画面仍是上一版，
 * 挂着门帘反而会**每次编辑都闪一下**（热重载是 300ms 防抖后自动跑的）。
 */
const dhampirLoading = ref(false)
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
/**
 * **底座 wasm 的模块命名空间**（与 `dhampirEngine` 并列，`startDhampir` 里装好）。
 *
 * 为什么要单独留一份：`loadEngine(pkgUrl)` 返回的是 `new Engine(mod)` **包装对象**，
 * 而动图那两条导出（`dhampir_asset_load_animation` / `dhampir_asset_animation_info`）
 * **不在包装对象上** —— 底座 `web/engine.js` 还没包它们。所以直接拿命名空间调
 * （ESM 缓存 ⇒ 与 `loadEngine` 内部同一份，不重复下载）。
 * ⚠️ `stopDhampir` 里要一起清掉（引擎没了，命名空间留着只会让"已停"的实例继续被调到）。
 */
let dhampirPkgNs = null
let dhampirRaf = 0
let dhampirBusy = false // 同一时刻只让一帧在飞：`seek` 是异步的，堆起来只会越拖越远
/** 正在热重载工程（见 `reloadDhampirProject`）：期间不重入、也不让 `tick()` 抢 seek */
let dhampirReloading = false
/** 重载请求撞上"有帧在飞"被拦下 → 那一帧收尾后补跑一次（别把这次改动丢了） */
let dhampirReloadPending = false
/** 热重载的合并防抖定时器（连续改动只重载最后一次）；`stopDhampir` 也要能取消它 */
let dhampirReloadTimer = 0
/** 上一次喂给重载判据的配置指纹（见下面那个 `watch`） */
let dhampirCfgKey = ''
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
 * 当前 **6** = 动图原生解码（`dhampir_asset_load_animation` / `dhampir_asset_animation_info`
 * 两条导出 + `AnimInfoView`）那次 —— 与 v3→v4 同类：**形状没变、导出面变了**。
 * 那两条导出让引擎自己解码 GIF/动画 WebP，宿主不再逐帧喂位图（见本文件「动图直通」一节）。
 */
const DHAMPIR_HOST_API = 6
const DHAMPIR_ALIGN_RATE_LO = 0.98 // ② 超前 → 放慢一点
const DHAMPIR_ALIGN_RATE_HI = 1.02 // ② 落后 → 追一点
/**
 * **出声时**纠偏速率的收窄幅度（见 `dhampirSourceAudible`）。
 *
 * 为什么要有这一对：纠偏靠 `playbackRate ±2%`，而**速率一动音高就跟着动**。
 * 无声时无所谓（那些元素只是帧的来源），一旦出声就是"预览跑调"。
 * 所以出声时把幅度收到 ±0.5%（约 1/12 个半音，听不出来），
 * 偏差自然会大一点 —— 超过 `DHAMPIR_ALIGN_HARD` 仍旧走 hard seek，
 * 周期性的源切换也会把它归零，不会一路漂下去。
 *
 * ⚠ 别把这两条删掉改回直接用 ±2%：±2% 是 ±35 音分，人耳对持续音的
 * 十几音分就能听出来（浏览器只有 `preservesPitch` 时才会替我们盖住，
 * 而它不是所有实现都有 —— 下面 `dhampirPreservePitch` 就是给它兜底的）。
 */
const DHAMPIR_ALIGN_RATE_AUDIBLE_LO = 0.995
const DHAMPIR_ALIGN_RATE_AUDIBLE_HI = 1.005
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

/**
 * **引擎代**：每换一个"引擎手上的动图缓存"，这个数就 +1。
 *
 * 换的地方只有两处，都在**引擎侧**：
 *   * `attach()` —— 新建 `AnimationTextures`（空的）；
 *   * `open()` 成功 —— 底座 `dhampir_project_open` 内部会 `clear_animations()`
 *     （`crates/dhampir-wasm/src/timeline_host.rs`，注释原文"换一份工程就是换一批素材"）。
 *
 * ⚠️ 为什么必须有它：动图（GIF/动画 WebP）是**引擎自己解码**的，宿主只在加载时交一次字节；
 * 而宿主那两项状态（`animLoaded` / `animError`）描述的是**引擎的手上状态**。
 * 不绑代际就会出现这个组合：热重载 → 引擎被清空 → 宿主还拿着 `animLoaded=true`
 * 不再交字节 ⇒ **贴纸在预览里消失，只有整页重载（F5）才回来**（用户报过，实测复现）。
 * 而每次编辑都会走一次热重载（`dhampirCfgKey` watch），所以这条会把"偶发"变成"必然"。
 */
let dhampirAnimGen = 0
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

/**
 * **源素材自己的帧率**（来自工程里那条视频素材的时基）。
 *
 * ⚠️ 它与 `dhampirFps`（**工程网格** = `timeline.timebase`）**不是一回事**（2026-10-06）：
 * 电台那种"视频流 5fps 静止帧 + 主音频"的素材，工程网格会被抬到 30fps（让叠加层一秒动 30 次），
 * 而素材仍只有 5fps。段落表里的 `source_in` 是**素材帧号**（底座 `source_frame_looped` 按时间换算），
 * 所以"源秒 → 源帧"要用**这个**帧率；用网格帧率去比就是 6 倍错位（屏蔽区间的跳转点全错）。
 */
let dhampirSourceFps = 30
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
const dhampirStickerStat = { total: 0, ready: 0, idx: -1, submits: 0, decodes: 0, delayConflict: 0 }

function dhampirBuildSegments(project) {
  // 源素材那份时基（= `source_in` 的单位）。取不到就当作与网格一致（老行为）。
  const videoAsset = ((project.assets || []).find((a) => a.kind === 'video')) || {}
  const stb = videoAsset.timebase || {}
  dhampirSourceFps = stb.num > 0 && stb.den > 0 ? stb.num / stb.den : dhampirFps.value
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
  const seqFps = dhampirFps.value || 30
  const srcFps = dhampirSourceFps || seqFps
  // 无段落表（单段、source_in=0）⇒ 源时间与序列时间同一时刻，直接乘网格帧率（老行为）
  if (!dhampirSegments.length) return Math.max(0, Math.round((seconds || 0) * seqFps))
  // `source_in` 是**素材帧号** ⇒ 源位置用素材帧率换算；段落长度是序列帧 ⇒ 换算回素材帧再比。
  // ⚠️ **位置不取整**（见下面那段说明）：取整会把段内换算也钉在素材帧上。
  const srcPos = Math.max(0, (seconds || 0) * srcFps)
  for (const seg of dhampirSegments) {
    const lenSeq = seg.frameEnd - seg.frameStart
    const lenSrc = Math.max(1, Math.round((lenSeq * srcFps) / seqFps))
    if (srcPos >= seg.sourceIn && srcPos < seg.sourceIn + lenSrc) {
      // ⚠️ **段内换算走秒，不走素材帧**（2026-10-06 修「5/30fps · 丢 30」）：
      // 用素材帧算的话，5fps 素材的工程帧只能落在 `round(src)*seqFps/srcFps` = **6 的倍数**上
      // ⇒ 一秒只有 5 个不同的帧号 ⇒ `tick()` 里 `f === dhampirFrameShown` 把另外 5/6 帧全挡掉
      // ⇒ 读数 `5/30fps · 丢 30`，而**叠加层（字幕/弹幕/贴纸）也只动 5 次/秒** ——
      // 那正是"30fps 网格下限"要解决的事，成片按 30fps 画、预览却只画 5 帧/秒 = 预览≠成片。
      // 段落**边界**仍旧按素材帧判（`source_in` 与 `lenSrc` 同单位），所以"落在被掐掉的区间
      // 里 ⇒ null"这条语义一个字都没变。
      const localSec = (seconds || 0) - seg.sourceIn / srcFps
      return seg.frameStart + Math.round(localSec * seqFps)
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
  const srcFps = dhampirSourceFps || dhampirFps.value || 30
  // 同 `dhampirFrameAt`：位置**不取整**（边界是素材帧、位置是秒，取整只会让判定抖半帧）
  const src = Math.max(0, (seconds || 0) * srcFps)
  const next = dhampirSegments
    .map((s) => s.sourceIn)
    .filter((s) => s > src)
    .sort((a, b) => a - b)[0]
  return next === undefined ? null : next / srcFps
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
    // ⚠️ 换绑 = **声源也跟着换**（2026-10-02）。
    // 新上台的那块必须立刻拿到用户的音量/静音状态 + 音高保持，否则换台之后
    // 这一路要么没声音、要么带着上一轮预滚留下的旧速率（跑调）。
    // 旧的那块随后变"备用"，由下面的 `applyAudio` 一并压回静音。
    dhampirPreservePitch(tw.el)
    try { tw.el.play() } catch (e) { /* 忽略：下一帧镜像循环会再试 */ }
    if (old) {
      old.pause()
      dhampirWatchTwin(assetId, old)                               // 旧的那块接下一段的预滚
    }
    dhampirTwins.set(assetId, { el: old, id: old ? old.id : '', want: 0, ready: false })
  }
  dhampirArmedIds = []
  // 换完整体重铺一次：台上的拿到音量、旧的（已变备用）压回静音，
  // 免得"两路同时出声"或"换过去那路没声音"——只在这里做一次，别在循环里逐元素加。
  applyAudio()
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

/**
 * **把磁盘上的工程热重载进引擎**（改效果 / 挪事件时间后不用刷新页面）。
 *
 * ## 为什么非有不可
 *
 * 画面是**引擎按它自己那份工程**画的（`open()` 在 `startDhampir` 里只调用**一次**），
 * 而工程 JSON 是**从磁盘 `polish.toml` 转译**来的（`POST /polish/dhampir-project`）。
 * `props.config` 一变，DOM 那条腿会重算（`buildIndex`），**但引擎收不到任何通知** ——
 * 于是"改一个效果 / 把一个事件挪个时间"必须刷新页面（= 重挂载 → `startDhampir` 重跑
 * → 重新读盘）才看得见。这里补上的就是这条缝。
 *
 * ## 只换工程，不重建别的
 *
 * `open()` 内部会 `bumpPlanGeneration()` 作废帧标识缓存（底座为这种场景准备的），
 * 而 `<video>` 源 / 解码器 / 贴纸位图 / 字体 / rAF **全都留着** —— 所以播放位置不丢、
 * 画面不黑。**不要**改成 `stopDhampir() + startDhampir()`：那会重建全部源与解码器，
 * 播放位置打回 0、画面闪一下（只有在热重载真的失败时才值得退回那条路）。
 *
 * ## 失败不许静默
 *
 * `open()` 失败时**底座保留上一份可用工程**（Rust 侧口径），所以画面不会崩 ——
 * 但那正是最容易变成"改了没反应、也不报错"的地方，所以失败要留痕。
 */
async function reloadDhampirProject() {
  if (!dhampirMode.value || !dhampirEngine || dhampirReloading) return
  // 有帧在飞时别换工程：`tick()` 正在对旧工程 seek，换掉会让那一帧对不上。
  if (dhampirBusy) { dhampirReloadPending = true; return }
  dhampirReloading = true
  try {
  const res = await api('/api/polish/dhampir-project', {
      method: 'POST',
      label: 'Dhampir 工程（重载）',
      body: JSON.stringify({ dir: props.dir }),
    })
    // 期间可能已经退出 Dhampir 模式（`stopDhampir` 把引擎置空了）—— 那就别往回写。
    if (!dhampirMode.value || !dhampirEngine) return
    // §4：转译报告先播报（它解释"为什么某个效果在成片里不见了"）
    logTranslateReport(res, 'dhampir')
    const project = res.project
    if (!project || !project.timeline) throw new Error('后端没有返回可用的工程')

    // ⚠️ **字体必须在 `open()` 之前装好**（2026-10-02 修）：canvas 栅格化用的是页面里
    // 已有的字体，没装好就静默回退 —— 换字体不生效、且字号/位置/颜色全对，最难查的那一类。
    // 这段原先只在 `startDhampir` 里跑，热重载从不执行 ⇒ "换字体不热重载"。
    await ensureProjectFonts(project)
    const tb = project.timeline.timebase
    if (tb && tb.num > 0 && tb.den > 0) dhampirFps.value = tb.num / tb.den
    // 段落表**必须跟着换**：时间线被压缩（`keep_muted = false`）时段落边界会变，
    // 不换的话 `dhampirFrameAt` 的帧号映射整体错位（画面与时间轴对不上）。
    dhampirBuildSegments(project)

    // 画布尺寸跟着工程走（渲染提示可能变了；只缩不放、保宽高比，口径与首启一致）。
    {
      const canvas = dhampirCanvasRef.value
      const size = canvas ? dhampirCanvasSizeFor(project, canvas) : null
      if (size && (canvas.width !== size.w || canvas.height !== size.h)) {
        canvas.width = size.w
        canvas.height = size.h
        if (dhampirEngine) dhampirEngine.resize(size.w, size.h)
      }
    }

    // **贴纸层描述必须重建**：新增/删除/挪时间的贴纸要新的 spans 才看得见。
    // 动图加载状态按 uri 继承（见 dhampirStickersFrom），但**有效性绑引擎代**（`dhampirAnimGen`）。
    dhampirStickers = dhampirStickersFrom(project)
    dhampirStickerDiag = null
    Object.assign(dhampirStickerStat, { total: 0, ready: 0, idx: -1, submits: 0, decodes: 0 })
  const opened = dhampirEngine.open(project)
    if (opened && opened.ok !== true) {
      // 旧工程还在（底座口径）⇒ 画面不会崩，但要说清这次改动没生效。
      console.warn('Dhampir 热重载被拒（保留上一份工程）：' + JSON.stringify((opened && opened.issues) || []).slice(0, 200))
      dhampirStatus.value = '工程重载被拒（画面仍是上一版）'
      setTimeout(() => { if (dhampirMode.value) dhampirStatus.value = '' }, 2500)
      return
    }
    // ⚠️ `open()` **成功**就代表底座刚 `clear_animations()` 过（失败时它保留上一份工程、
    // 也不清动图）⇒ 代际 +1，让 `dhampirLoadAnimations` 必须重交一遍字节。
    dhampirAnimGen += 1

    // 换绑源：新工程的 asset 区间可能变了，按当前的 `sourcesFor` 重新核对一遍。
    // （元素没变，`bindSource` 只是把 source→video 的映射重写一次，代价极小。）
    for (const [assetId, v] of dhampirSourceVideos) {
      if (v && v.id) { try { dhampirEngine.bindSource(assetId, v.id) } catch (e) { /* 源可能已不在工程里 */ } }
    }
    // **动图重新交给引擎**（在 open() 之后：`open()` 会清掉引擎里已加载的动图，
    // 所以顺序不能反 —— 反了就是"热重载后贴纸不动了"）。
    // 上面刚 `dhampirAnimGen += 1` ⇒ 这次**一定会重交**（同代内才跳过）。
  await dhampirLoadAnimations(project)
    // 静态图的预热保留（它们仍走宿主位图那条路）。
    callPrewarmStickers()
    // **字幕/弹幕必须重喂**：loadSubtitles 是替换语义（resetTextMemos + 逐条 set），
    // 新工程改过的字幕内容只有重喂才进引擎 —— 原先漏了这步，改字幕热重载不生效。
  await dhampirApplySubtitles(project, res.subtitles)

    // **强制下一帧重画**：帧号没变的话 `tick()` 会以为"还是这一帧"直接 return
    // （那是 30fps 内容在 60Hz rAF 上的正常优化）。播放位置**不动** ✓
    dhampirFrameShown = -1
    dhampirLastFrameDrawn = -1
    dhampirStatus.value = ''
    scheduleDhampir()
  } catch (e) {
    console.warn('Dhampir 热重载失败：' + ((e && e.message) || e))
    dhampirStatus.value = '工程重载失败：' + ((e && e.message) || e)
    setTimeout(() => { if (dhampirMode.value) dhampirStatus.value = '' }, 4000)
  } finally {
    dhampirReloading = false
    // 重载期间被拦下的那次（有帧在飞）：现在补一次，别把改动丢了。
    if (dhampirReloadPending) { dhampirReloadPending = false; scheduleDhampir(); reloadDhampirProject() }
  }
}

/** 进/出 Dhampir 模式。**按钮切换保留**：出去时把引擎停掉、回到 DOM 那一套。 */
async function toggleDhampir() {
  if (dhampirMode.value) {
    dhampirMode.value = false
    stopDhampir()
    // 退出 Dhampir：`dhampirVideos` 已经清空，声音交回 DOM 那条腿
    // （`stopDhampir` 把宿主 `innerHTML=''` 清掉、源元素随之销毁）。
    // ⚠️ 必须重铺一次，否则回来时台上那块还带着上一次进 Dhampir 之前的状态。
    applyAudio()
    return
  }
  dhampirError.value = ''
  // 两个都在播会同时出声，也看不清在比哪一个。
  const el = videoRef.value
  if (el) el.pause()
  // ⚠️ 门帘要在**翻 `dhampirMode` 之前**掀：加载期间不许播（见 `dhampirLoading` 的说明）。
  dhampirLoading.value = true
  dhampirMode.value = true
  dhampirStatus.value = '正在加载 Dhampir…'
  try {
    await nextTick() // canvas 要先真的挂上（attach 收的是 canvas id）
    await startDhampir()
    dhampirStatus.value = ''
    dhampirFrame.value = dhampirFrameAt(el ? el.currentTime : current.value) ?? 0
    scheduleDhampir()
    // 进 Dhampir：出来的那条腿（DOM 那块 video）让位，声音归引擎那批源。
    // `startDhampir` 里已经铺过一次，这里再铺一次是为了盖住"加载期间用户动了滑块"那一段。
    applyAudio()
  } catch (e) {
    // **绝不白屏**：说清原因、退回 DOM、把按钮禁掉（`dhampirError` 有值时按钮 disabled）。
    dhampirError.value = String((e && e.message) || e)
    dhampirStatus.value = ''
    // ⚠️ **失败也要把"卡在哪一步"打出来**：分步计时那套本来就有（`dhampirMark`），
    // 只在成功末尾 `dhampirMarkReport()` 的话，一次失败只剩下一个孤零零的报错信息
    // 与一堆 minify 后的变量名（实测「F i not iterable」就是这么来的）。
    const lastStep = dhampirLoadMarks.length ? dhampirLoadMarks[dhampirLoadMarks.length - 1].label : '(还没开始)'
    console.error('[dhampir] 起引擎失败（最后一步：' + lastStep + '）：' + ((e && e.stack) || e))
    dhampirMark('失败')
    dhampirMarkReport()
    dhampirMode.value = false
    stopDhampir()
  } finally {
    // 成功与失败都要放下门帘（失败时 `dhampirMode` 已翻回 false，门帘更不该留着）。
    dhampirLoading.value = false
  }
}

/** 起引擎：工程 JSON -> 每个视频素材一个 <video> -> loadEngine -> open/attach/resize/bind */
/**
 * 把工程声明的字体族装进页面（canvas 栅格化用的是**页面里已有的字体**）。
 *
 * ⚠️ **热重载也必须走这一步**（2026-10-02 修）。这段原先写死在 `startDhampir` 里，
 * 而 `reloadDhampirProject` 不跑它 —— 于是**换字体不生效**：新字体族从没注册进页面，
 * canvas 静默回退成默认字形，而字号/位置/颜色全对，看起来就像"字体那项坏了"。
 *
 * 字体是异步到位的，不等 `document.fonts.ready` 的话重载后第一屏仍是回退字形。
 * 已经装过的族由 `ensureFontFace` 自己幂等，重复调用不会重复取字节。
 */
async function ensureProjectFonts(project) {
  const families = new Set()
  for (const track of (project.timeline && project.timeline.tracks) || []) {
    const f = (track.subtitle && track.subtitle.font_family) || (track.danmaku && track.danmaku.font_family)
    if (f) families.add(f)
  }
  for (const family of families) {
    try {
      await ensureFontFace(null, props.dir, family, { onDebug: (m) => addClient('debug', m, 'dhampir') })
    } catch (e) {
      dhampirStatus.value = '字体 ' + family + ' 没装上（预览会用回退字体）'
    }
  }
  // 字体是异步到位的；不等它的话第一屏会是回退字形。
  try { if (document.fonts && document.fonts.ready) await document.fonts.ready } catch (e) { /* 等不到就算了 */ }
}

/**
 * 按工程给的渲染提示（render_hints）与窗口尺寸决定画布目标分辨率（只缩不放、保宽高比）。
 * **startDhampir 与热重载共用** —— 尺寸口径两处必须一致，所以抽成一份。
 * 默认满分辨率（fit 实测会裁切/放大视频层，见 startDhampir 里的对拍记录）。
 */
function dhampirCanvasSizeFor(project, canvas) {
  const hints = project.render_hints || {}
  if (!(hints.width > 0 && hints.height > 0)) return null
  const full = { w: hints.width, h: hints.height }
  let w = full.w
  let h = full.h
  const forceFull = new URLSearchParams(location.search).get('fit') !== '1'
    && window.__dhampirFit !== true
  if (!forceFull) {
    const box = canvas.getBoundingClientRect()
    const dpr = window.devicePixelRatio || 1
    if (box.width > 0 && box.height > 0) {
      const scale = Math.min((box.width * dpr) / full.w, (box.height * dpr) / full.h, 1)
      if (scale < 1) {
        w = Math.max(320, Math.round(full.w * scale))
        h = Math.max(180, Math.round(full.h * scale))
      }
    }
  }
  return { w, h }
}

/**
 * 从新工程重建贴纸层描述（活跃区间 / 素材时基 / 相位锚点）。
 * **热重载也必须重建**（2026-10-02 补）：贴纸的新增/删除/时间挪动都要新的 spans 才看得见，
 * 而这段原先只在 `startDhampir` 里跑 —— 热重载后 `dhampirStickers` 还是旧的，
 * 症状是“改了贴纸时间，Dhampir 预览里不生效；DOM 预览却是对的”。
 * 解码缓存（bytes/bytesIdx）**继承**：同 uri 同 fps 的缓存仍然有效，不重复解码。
 */
/**
 * **动图直通：把 GIF / 动画 WebP 的原始字节整段交给引擎，由引擎自己解码、自己按帧号取帧。**
 *
 * # 为什么改成这样（2026-10，底座 host_api v6）
 *
 * 以前是本文件逐帧喂位图：按素材时基把「工程帧 → 秒 → GIF 帧号」算出来，
 * 每换一帧就 `POST /api/polish/sticker-frame` 取一张 base64 PNG（~390KB/帧），
 * 再 `createImageBitmap` 画进常驻 canvas、最后 `setSourceBitmap`。
 * 那条路的问题是结构性的：
 *   * 逐帧 HTTP（seek 后首帧晚 ~20ms、偶发一帧抖动）；
 *   * base64 膨胀 1.33×（实测整包 46.2 MiB vs GIF 原文件 8.4 MiB，4.1×）；
 *   * **帧号换算有两份实现**（服务端出片一份、这里一份），一致性只能靠纪律守。
 *
 * 现在引擎按 `(source, source_frame)` 自己取帧 —— 而 `source_frame` 是**求值层直出的最终帧号**
 * （底座 `layer.rs::source_frame_at_delays`，逐帧延迟累加 + 循环取余 + `source_in` 为圈入口）。
 * ⚠️ **所以我们不再自己算帧号**，也不该拿 `seconds` 去换算任何东西 ——
 * `seconds` 只是给 `<video>` seek 用的驱动字段（底座 §8.2-1 明确更正过这一点）。
 *
 * # 一次性 vs 每帧
 *
 * 每个动图资产**只 fetch + load 一次**（`st.animLoaded` 记账），播放期零请求、零宿主操作。
 * 素材文件换了（`uri` 变了）要重新 load —— 引擎按 id 替换是幂等的。
 *
 * # 失败形态
 *
 * 加载失败**不改路由**：引擎照旧走宿主位图那条路（也就是这一层画不出来）。
 * ⚠️ 必须在这里接住错误：**渲染期没有 issue 通道** —— `resolver.texture_for` 返回 `None`
 * 的层是被**静默跳过**的（底座 `render/timeline.rs`），指望 `draw` 报错是等不到的。
 */
async function dhampirLoadAnimations(project) {
  if (!dhampirPkgNs || typeof dhampirPkgNs.dhampir_asset_load_animation !== 'function') {
    // 底座太老（没有 v6 那两条导出）⇒ 说清楚，别让贴纸"静静地不动"
    if (dhampirStickers.length > 0) {
      dhampirStickerInfo.value = '底座没有动图导出（要 host_api v6）'
    }
    return
  }
  const byId = new Map((project.assets || []).map((a) => [a.id, a]))
  // 打印一次引擎身份：把这个应用的适配器与我基准里的对比（13x 差距的第一嫌疑）。
  if (!window.__dhampirAnimLogged) {
    window.__dhampirAnimLogged = true
    console.log('[dhampir] 动图加载环境 ' + JSON.stringify({
      canvas: dhampirCanvasRef.value ? dhampirCanvasRef.value.width + 'x' + dhampirCanvasRef.value.height : '?',
      dpr: window.devicePixelRatio,
      stickers: dhampirStickers.length,
    }))
  }
  for (const st of dhampirStickers) {
    const a = byId.get(st.assetId)
    if (!a || !a.uri) continue
    // 只有动图走这条路；静态图（kind === 'image'）仍走宿主位图那条路。
    if (a.kind !== 'image_sequence') continue
    // 幂等：**同一代引擎内**同素材不重复传。代际不同 = 引擎手上已经没有了（见 `dhampirAnimGen`）
    if (st.animLoaded && st.animUri === a.uri && st.animGen === dhampirAnimGen) continue
    // 已确定失败的（真解不开/取不到）不反复重试 —— 否则日志会被刷成墙。
    // 但 "还没 attach" 那条**要重试**：它不是失败，是时机不对。
    // 失败也**按代记**：上一代引擎里失败的那张，换过引擎要再试一次
    // （不记代的话，一次瞬时失败会让那张贴纸直到整页重载都不再出现）
    if (st.animError && !st.animPendingAttach && st.animGen === dhampirAnimGen) continue
    try {
      // 地址口径与视频源一致（`clipMediaUrl` 把 dir 归一成片段名 + 拼 output/clips/…）
      //
      // ⚠️ **计时区间必须把 `fetch` 包进来**（2026-10-02 修）：原来 `tFetched` 打在
      // `fetch` **之前**，于是"取字节"量的只是 `clipMediaUrl()` 的字符串拼接（恒为 0ms），
      // 而 `await fetch` + `arrayBuffer()` 被算进了"解码"里 —— 一份 5 MB 的 GIF
      // 在本地也要几百毫秒，"解码很慢"这个结论就是被这条错计时喂出来的。
      // 三段分开：**取字节**（网络/盘） / **解码**（wasm） / **上传**（已在 wasm 内部）。
      const url = clipMediaUrl(props.pid, props.dir, a.uri)
      const tFetch0 = performance.now()
      const resp = await fetch(url)
      if (!resp.ok) throw new Error('HTTP ' + resp.status)
      const bytes = new Uint8Array(await resp.arrayBuffer())
      const tFetched = performance.now()
      const raw = dhampirPkgNs.dhampir_asset_load_animation(st.assetId, bytes)
      const tDecoded = performance.now()
      const res = JSON.parse(raw)
      const mib = (bytes.byteLength / 1048576).toFixed(1)
      console.log('[dhampir] 动图 ' + st.assetId + ' 取字节 ' + Math.round(tFetched - tFetch0)
        + 'ms (' + mib + ' MB) / 解码 ' + Math.round(tDecoded - tFetched) + 'ms')
      if (res && res.ok === true) {
        st.animLoaded = true
        st.animUri = a.uri
        st.animGen = dhampirAnimGen
        st.animInfo = res.info || null
        st.animPendingAttach = false
        dhampirStickerStat.ready += 1
        // **时间真值归一**（底座 §8.3-2）：解码器读出的 `frame_delays_ms` 是事实，
        // 工程 JSON 里那份退为诊断。不一致时报出来 —— 否则「映射用一张表、像素用另一张表」
        // 会把漂移请回来（症状是动图越播越偏）。
        if (res.info && Array.isArray(res.info.frame_delays_ms)) {
          st.delays = res.info.frame_delays_ms
          // ⚠️ **冲突要报**（T3）：判据是"**解码器赢**"，所以只可能是一边静默漂 ——
          // 工程 JSON 那份是转译器用 ffprobe 探的，解码器是底座自己读的，两者
          // 只在"文件被换过 / 探测器与解码器口径不同"时才会不一致。
          // 报的目的是**别让漂移静默发生**（用户看到的是"贴纸越播越偏"，最难归因的那种）。
          const declared = Array.isArray(a.frame_delays_ms) ? a.frame_delays_ms : null
          if (declared && declared.length === st.delays.length) {
            const diff = declared.findIndex((v, i) => v !== st.delays[i])
            if (diff >= 0) {
              const msg = 'dhampir: 动图 ' + st.assetId + ' 的帧延迟表与工程 JSON 不一致，'
                + '按解码器为准（第 ' + diff + ' 帧：工程 ' + declared[diff] + 'ms / 解码器 ' + st.delays[diff] + 'ms）'
              console.warn(msg)
              dhampirStickerStat.delayConflict += 1
              st.animDelayConflict = msg
            }
          } else if (declared && declared.length !== st.delays.length) {
            // 帧数都对不上是更严重的漂移（工程表会被当权威用错地方）
            const msg = 'dhampir: 动图 ' + st.assetId + ' 的帧延迟表长度不一致（工程 '
              + declared.length + ' / 解码器 ' + st.delays.length + '），按解码器为准'
            console.warn(msg)
            dhampirStickerStat.delayConflict += 1
            st.animDelayConflict = msg
          }
        }
      } else {
        // parsed:true / ok:false = 认得出格式但解不开；parsed:false = 连格式都不认识
        const why = (res && res.error) || '未通过校验'
        // ⚠️ **「还没 attach」不是失败，是时机不对**（2026-10-02 实测）：
        // 底座把纹理上传挂在 attach 之后 —— host 还没接上时它只**记账时间真值**并返回 ok:false。
        // 这条必须与"真解不开"分开：混在一起会让动图**永远不重试**（下次进 Dhampir 也不重传，
        // 因为 `animLoaded` 没置上、但 `animError` 会让人以为这条路坏了）。
        // 判据用错误原文里的 "attach" —— 底座那条文案是 '还没 attach 上 canvas：…'。
        if (/attach/.test(why)) {
          // 留给 attach 之后那次重试（startDhampir 在 attach 完会再调一遍）。
          // 用 continue 而不是 return：每一张都各自标 pending，
          // 免得将来某张资产在 attach 之前就独立失败时被这一条整个跳过。
          st.animPendingAttach = true
          continue
        }
        console.warn('dhampir: 动图没解开 ' + st.assetId + '（' + a.uri + '）：' + why)
        st.animError = why
        st.animGen = dhampirAnimGen
      }
    } catch (e) {
      const why = (e && e.message) || String(e)
      console.warn('dhampir: 动图加载失败 ' + st.assetId + '（' + a.uri + '）：' + why)
      st.animError = why
      st.animGen = dhampirAnimGen
    }
  }
}

function dhampirStickersFrom(project) {
  const stickerImages = (project.assets || [])
    .filter((a) => (a.kind === 'image_sequence' || a.kind === 'image') && a.uri)
  const prevByUri = new Map((dhampirStickers || []).map((x) => [x.uri, x]))
  return stickerImages.map((a) => {
    const tb = a.timebase || {}
    const fps = (tb.num > 0 && tb.den > 0) ? (tb.num / tb.den) : 30
    const spans = []
    for (const track of (project.timeline && project.timeline.tracks) || []) {
      for (const layer of track.layers || []) {
        if (layer.enabled === false) continue
        if (layer.source && layer.source.asset_id === a.id) {
          spans.push({
            start: Number(layer.start) || 0,
            end: Number(layer.end) || 0,
            sourceIn: Number((layer.source && layer.source.source_in) || 0),
          })
        }
      }
    }
    spans.sort((x, y) => x.start - y.start)
    // 动图（image_sequence）的加载状态**按 uri 继承**（连同 `animGen`）：同一代引擎内素材没换
    // 就不必重传字节（几十 MB 再传一遍是白花）。⚠️ **跨代不继承有效性** ——
    // 判据在 `dhampirLoadAnimations` 里带 `st.animGen === dhampirAnimGen`（见 `dhampirAnimGen`）。
    // assetId 取**新工程**的（理论上不变，但别让旧工程残留值漏进来）。
    const prev = prevByUri.get(a.uri)
    if (prev) return { ...prev, assetId: a.id, spans }
    // ⚠️ 帧号/位图那几项（bytes / bytesIdx / canvas / count / lastFrame）**已经不需要了**：
    // 动图由引擎自己解码并按 `source_frame` 取帧，宿主不再逐帧换算、不再喂位图。
    return {
      assetId: a.id, uri: a.uri, fps, spans,
      // 动图状态（见 dhampirLoadAnimations）
      animLoaded: false, animUri: '', animInfo: null, animError: '', animPendingAttach: false,
      animDelayConflict: '',
    }
  })
}

/**
 * 把随工程带回来的字幕/弹幕旁写内容转成 blob URL 并喂给引擎。
 * **热重载也必须重跑**（2026-10-02 衑）：`loadSubtitles` 的语义是**替换**（它先 `resetTextMemos`
 * 再逐条 `dhampir_project_set_subtitles`），所以新工程改了字幕内容/新增弹幕后必须重喂；
 * 原先只在 `startDhampir` 里跑 ⇒ 热重载后改的字幕不进引擎。
 * 旧的 blob URL 在这里一并释放（entries 是全量替换）。
 */
async function dhampirApplySubtitles(project, subtitles) {
  // **旧的 blob URL 先释放**：entries 是全量替换，旧 URL 留着就是泄漏
  // （首启时这个数组本来就是空的 —— stopDhampir 已清；重载时这里才是唯一的回收点）。
  for (const old of dhampirBlobUrls) URL.revokeObjectURL(old)
  dhampirBlobUrls = []
  const subTexts = subtitles || {}
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
    if (failed.length > 0) {
      dhampirStatus.value = '（字幕 ' + failed.length + ' 路没登记上）'
    }
  }
}

// ---------------------------------------------------------------------------
// 建源 / 拆源：**首次进 Dhampir 与换片段共用一份**
// ---------------------------------------------------------------------------
//
// ⚠️ **源元素的 id 必须带代际**（`dhampir-src-<gen>-<i>`）：换片段时新的源是**先建后拆**的
// （先建 = 引擎那一帧立刻有新像素可用，不留黑窗），同 id 的话 `getElementById` 会拿到**旧**那批
// —— 引擎就会一直画上一个片段的帧（不报错、也不黑，最难查的那种）。
let dhampirSrcGen = 0
/** 正在换片段（防重入）/ 换片段待办（引擎还没起来 / 有帧在飞）。 */
let dhampirSwitching = false
let dhampirSwitchPending = false

/**
 * 为一份工程建源：每个视频类素材一块 `<video>`（台上那块）+ 一块备用 twin（A/B 预滚）。
 * 返回 `[assetId, videoId, el]` 绑定表；`dhampirSourceVideos` / `dhampirVideos` / `dhampirTwins`
 * 也在这里铺好（引擎要按素材 id 找元素）。
 */
async function buildDhampirSources(project) {
  const host = dhampirHostRef.value
  if (!host) throw new Error('源宿主还没准备好')
  const gen = (dhampirSrcGen += 1)
  const videos = (project.assets || []).filter((a) => a.kind === 'video' && a.uri)
  const bindings = []
  const created = []
  videos.forEach((a, i) => {
    const v = document.createElement('video')
    v.id = 'dhampir-src-' + gen + '-' + i
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
    created.push(v)
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

  // `dhampirSourceVideos` 与 `dhampirVideos` 是**同一批元素**：前者的键是引擎的 `source`
  // （= 素材 id，`sources_for` 给的就是它），纠偏按 source 找元素要用它。
  dhampirSourceVideos = new Map()
  for (const [assetId, , v] of bindings) dhampirSourceVideos.set(assetId, v)
  dhampirVideos = [...dhampirSourceVideos.values()]

  // **备用那套源**（A/B）：每个源再多一块同 `src` 的 `<video>`，1px、暂停、静音，
  // 平时什么都不做；进屏蔽区前把"下一段起点"seek 好，到点 `bindSource` 换绑即成。
  // 放在同一个 1px 宿主里（`stopDhampir` 一句 `innerHTML=''` 就一起收掉）。
  dhampirTwins = new Map()
  dhampirArmedIds = []
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
    created.push(twin)
    dhampirWatchTwin(assetId, twin)
    dhampirTwins.set(assetId, { el: twin, id: twin.id, want: 0, ready: false })
  }
  // 备用那套**永远不出声**（换台瞬间两套同源若都在轨，音量会叠成两倍）。
  // 建完立刻压一次：`applyAudio` 也会认它们（按 `dhampirIsTwin`），这里是入队前的双保险。
  for (const tw of dhampirTwins.values()) {
    try { tw.el.muted = true } catch (e) { /* 忽略 */ }
  }
  return { bindings, created }
}

/** 把一批源元素**真正放掉**（`remove()` 不保证释放解码器：`src` 留着就还在缓存里）。 */
function dropDhampirElements(els) {
  for (const el of els || []) {
    try { el.pause() } catch (e) { /* 忽略 */ }
    try { el.removeAttribute('src'); el.load() } catch (e) { /* 忽略 */ }
    try { el.remove() } catch (e) { /* 忽略 */ }
  }
}

/**
 * **换片段**：同一个页面、同一块 canvas，把 Dhampir 那条腿整条换到新片段上。
 *
 * ## 为什么必须走这条路（而不是重挂组件）
 *
 * 底座的 `<canvas>` **一页只 attach 一次、没有 detach**：任何重挂都让新 canvas 从没被
 * `present` 过 ⇒ **画面全黑，只能 F5**（用户 2026-10-06 报的「在精修列表内切换精修视频，
 * 不会重载 Dhampir 导致黑屏」）。所以父级切片段改成**静默加载**（不卸载编辑器），
 * 这里负责把工程/源/字幕/贴纸/字体整条换过来。
 *
 * ## 顺序（每一条都是踩出来的）
 *
 * ① **先建新源、再拆旧源**：段中间不能没有可用元素，否则正好落在那几帧上就是黑的；
 * ② `open()` 在 `bindSource` 之后：先让引擎手里有元素，再换工程；
 * ③ `open()` 成功之后再 `dhampirAnimGen += 1` + 重交动图（`open()` 会清引擎里的动图缓存）；
 * ④ 字幕**必须重喂**（`loadSubtitles` 是替换语义）；
 * ⑤ 最后把"这一帧画过了"的标记清掉，否则 `tick()` 会以为还是那一帧、直接 return。
 */
async function switchDhampirClip() {
  // 没进 Dhampir：DOM 那条腿跟着 `props.dir` 自己换，这里什么都不用做。
  if (!dhampirMode.value) return
  if (!dhampirEngine) {
    // 引擎还没起来（首屏那 1~2 秒）：记下待办，`startDhampir` 收尾时会来补 ——
    // 否则首屏那次 open 用的是**换片段之前**取的工程，画面会停在上一个片段。
    console.log('[dhampir] 换片段：引擎还没起来，记为待办')
    dhampirSwitchPending = true
    return
  }
  if (dhampirSwitching || dhampirBusy) {
    console.log('[dhampir] 换片段：' + (dhampirSwitching ? '正在换' : '有帧在飞') + '，记为待办')
    dhampirSwitchPending = true
    return
  }
  console.log('[dhampir] 换片段开始：' + props.dir)
  dhampirSwitching = true
  try {
    const res = await api('/api/polish/dhampir-project', {
      method: 'POST', label: 'Dhampir 工程（换片段）', body: JSON.stringify({ dir: props.dir }),
    })
    if (!dhampirMode.value || !dhampirEngine) return
    // §4：转译报告先播报（换片段也是新一次转译）
    logTranslateReport(res, 'dhampir')
    const project = res.project
    if (!project || !project.timeline) throw new Error('后端没有返回可用的工程')

    // 播放头回 0：旧时刻对新片段没有意义（`current` 是这一侧自己的播放头）。
    current.value = 0
    playing.value = false
    const tb = project.timeline.timebase
    if (tb && tb.num > 0 && tb.den > 0) dhampirFps.value = tb.num / tb.den
    dhampirBuildSegments(project)
    dhampirStickerDiag = null
    Object.assign(dhampirStickerStat, { total: 0, ready: 0, idx: -1, submits: 0, decodes: 0 })

    // ⚠️ **改尺寸要放到最后那一下**（见下面那段）：`canvas.width = …` 会把交换链整个换掉、
    // 旧画面立刻变黑（实测：切片段后 1.5s 量到平均亮度 **0.18**，而 7.5s 是 219）。
    // 所以顺序是「**先建新源**（这一段的 await 期间旧画面照常画）→ 绑源 → 改尺寸 → open → 排帧」。

    // ⚠️ **换源/换工程必须落在两帧之间**：宿主那侧 `seek()` 是同步进栈的（`dhampirBusy`
    // 就是"这一帧在飞"的闸）。撞上了就交回 tick 补跑 —— 别在这一刻把引擎正在用的元素拆掉。
    if (dhampirBusy) { dhampirSwitching = false; dhampirSwitchPending = true; return }
    const previous = [...dhampirVideos, ...[...dhampirTwins.values()].map((t) => t.el)]
    const { bindings, created } = await buildDhampirSources(project)
    if (!dhampirMode.value || !dhampirEngine) { dropDhampirElements(created); return }
    if (dhampirBusy) {
      // 建源这一趟（等 loadeddata，最长 3s）里引擎又开始画了：把刚建的先撤掉，重来一次
      // （不做的话就得在帧中间换绑 + `open()`，那是"RefCell already borrowed"那类崩溃的形状）。
      // 撤掉之后 `dhampirSourceVideos` 还指着它们 —— 下一次 `buildDhampirSources` 会重新铺。
      dropDhampirElements(created)
      dhampirSourceVideos = new Map()
      dhampirVideos = []
      dhampirTwins = new Map()
      dhampirSwitching = false
      dhampirSwitchPending = true
      return
    }

    // ↓↓↓ **同步块**：绑源 → 改尺寸 → `open()` → 重置"这一帧画过了" → 排帧。
    //      中间不给浏览器任何机会插进一帧，黑窗最多一帧（16ms）。
    for (const [assetId, videoId] of bindings) {
      try { dhampirEngine.bindSource(assetId, videoId) } catch (e) { /* 该素材不在工程里 */ }
    }
    const canvas = dhampirCanvasRef.value
    const size = canvas ? dhampirCanvasSizeFor(project, canvas) : null
    if (canvas && size && (canvas.width !== size.w || canvas.height !== size.h)) {
      canvas.width = size.w
      canvas.height = size.h
      dhampirEngine.resize(size.w, size.h)
    }
    const opened = dhampirEngine.open(project)
    if (opened && opened.ok !== true) {
      console.warn('Dhampir 换片段被拒（保留上一份工程）：' + JSON.stringify((opened && opened.issues) || []).slice(0, 200))
      dhampirStatus.value = '换片段被拒（画面仍是上一版）'
      setTimeout(() => { if (dhampirMode.value) dhampirStatus.value = '' }, 3000)
      return
    }
    // 新工程 = 引擎刚 `clear_animations()` 过 ⇒ 代际 +1（动图必须重交一遍字节）
    dhampirAnimGen += 1
    // 旧源现在可以放掉了（新源已经绑上并被 `open()` 认下）。
    dropDhampirElements(previous)
    dhampirFrameShown = -1
    dhampirLastFrameDrawn = -1
    dhampirHoldSince = 0
    scheduleDhampir()   // 立刻安排重画：新工程的第一帧，别再等下一个自然的帧

    // ↓↓↓ 慢活（动图字节 / 字幕 / 字体）：**放在重画之后** —— 它们要几百毫秒到几秒，
    //      摆在前面就是把黑窗撑到那么长。
    dhampirStickers = dhampirStickersFrom(project)
    dhampirStickerDiag = null
    Object.assign(dhampirStickerStat, { total: 0, ready: 0, idx: -1, submits: 0, decodes: 0 })
    stickerFailed.value = {}
    await dhampirLoadAnimations(project)
    callPrewarmStickers()
    for (const v of dhampirVideos) {
      dhampirPreservePitch(v)
      v.play().catch(() => {})
    }
    applyAudio()
    await dhampirApplySubtitles(project, res.subtitles)
    await ensureProjectFonts(project)
    dhampirStatus.value = ''
    console.log('[dhampir] 换片段完成：' + props.dir)
  } catch (e) {
    console.warn('Dhampir 换片段失败：' + ((e && e.message) || e))
    dhampirStatus.value = '换片段失败：' + ((e && e.message) || e)
    setTimeout(() => { if (dhampirMode.value) dhampirStatus.value = '' }, 4000)
  } finally {
    dhampirSwitching = false
    if (dhampirSwitchPending) { dhampirSwitchPending = false; switchDhampirClip() }
  }
}

async function startDhampir() {
  dhampirMark('开始')
  if (dhampirEngine) return
  if (!navigator.gpu) throw new Error('这个浏览器没有 WebGPU（Dhampir 实时预览需要它）')
  const host = dhampirHostRef.value
  const canvas = dhampirCanvasRef.value
  if (!host || !canvas) throw new Error('画布还没准备好')

  // ⚠️ **底座一页只 attach 一次 canvas，而且没有 detach**：本页已经 attach 过**别的** canvas 时，
  // 再挂一块新的结局是"引擎一切正常、日志里首屏照样跑完，而画面**永远黑**"
  // （实测：块被换成新节点后舞台平均亮度 0.18，只能 F5）。这条路上只剩刷新 ——
  // 路由里已经带着当前片段，刷新后落到同一处、只 attach 一次。
  // ⚠️ 用 sessionStorage 挡一道（同一次会话只自动刷一次），否则成了刷新环；
  // attach 成功后会把这个闸清掉，让"下一次合理的重挂"还能再自救一次。
  const RELOAD_KEY = 'yekitrim.dhampir.attachReload'
  if (window.__dhampirAttachedCanvas && window.__dhampirAttachedCanvas !== canvas) {
    console.warn('[dhampir] 本页已经 attach 过一块 canvas（底座没有 detach）—— 现在这块是新的，'
      + '引擎画不到它上面。自动刷新一次（这正是"要 F5 才回来"的那件事）')
    if (!sessionStorage.getItem(RELOAD_KEY)) {
      sessionStorage.setItem(RELOAD_KEY, '1')
      location.reload()
      throw new Error('正在刷新页面以重新接入 Dhampir')
    }
    throw new Error('底座一页只 attach 一次 canvas：请刷新页面（这次会话已经自动刷过一次了）')
  }

  // ① 工程 JSON（后端跑转译器）
  const res = await api('/api/polish/dhampir-project', {
    method: 'POST',
    label: 'Dhampir 工程',
    body: JSON.stringify({ dir: props.dir }),
  })
  const project = res.project
  if (!project || !project.timeline) throw new Error('后端没有返回可用的工程')
  // §4：首启那次转译的报告也要播报（这是用户最常走的那条路）
  logTranslateReport(res, 'dhampir')
  const tb = project.timeline.timebase
  if (tb && tb.num > 0 && tb.den > 0) dhampirFps.value = tb.num / tb.den
  // 段落表要和时基一起先算好：**帧号映射全靠它**（见 dhampirFrameAt 的说明）。
  dhampirBuildSegments(project)
  dhampirStickerDiag = null
  Object.assign(dhampirStickerStat, { total: 0, ready: 0, idx: -1, submits: 0, decodes: 0 })
  // 画布目标尺寸：与热重载共用同一份口径（dhampirCanvasSizeFor）。
  {
    const size = dhampirCanvasSizeFor(project, canvas)
    if (size) { canvas.width = size.w; canvas.height = size.h }
  }

  // ② 建源（每个**视频类**素材一个 `<video>` + 一个备用 twin）—— 与换片段共用一份实现。
  const { bindings } = await buildDhampirSources(project)
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
  dhampirMark('视频源就绪')
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
  dhampirMark('engine.js + wasm 载入')
  const pkgNs = await import(/* @vite-ignore */ PKG_URL)
  dhampirPkgNs = pkgNs          // 动图加载要用它（包装对象上没有那两条导出）
  const hostApi =
    typeof pkgNs.dhampir_host_api_version === 'function' ? pkgNs.dhampir_host_api_version() : null
  if (hostApi !== DHAMPIR_HOST_API) {
    throw new Error(
      "底座 wasm API 版本不匹配：期望 " + DHAMPIR_HOST_API + "，实得 " +
        (hostApi === null ? "(没有 dhampir_host_api_version 这个导出)" : hostApi) +
        " —— 这是**形状变更**（字段增减），请换一个匹配的底座产物（`yekitrim polish doctor` 会打印当前用的那份）",
    )
  }
  // **打开底座的细粒度计时**：`prepareMs` 含「seek 视频 + 文字栅格化」，
  // `drawMs` 是 GPU 提交。底座说得很对 ——「只知道一帧要 200ms，该改哪里只能靠猜」。
  // **自报"这份 wasm 是哪一次构建"**。
  //
  // 为什么必须有它：`HOST_API_VERSION` 只在**导出面/形状**变化时才 +1，
  // 一次纯实现修复（例如 2026-10-02 把动图上传从逐帧改成一张数组纹理）不改版本号 ——
  // 于是"浏览器里跑的是修好的那份还是旧的"从版本号上**看不出来**。
  // 那次排查就卡在这：宿主侧改了 `crates/dhampir-wasm/www/pkg`，而预览其实是服务端
  // 从 `<程序目录>/dhampir/preview/pkg` 发的，**是两份不同的文件**，两者 api 都是 6。
  //
  // 打在控制台而不是日志面板：这是排查用的判据，要能直接与另一份产物对比。
  // 拿不到就不打（老产物没这条导出）——**不要**因此报错，它不是契约的一部分。
  const buildId = typeof pkgNs.dhampir_build_id === 'function' ? pkgNs.dhampir_build_id() : '(老产物没有 dhampir_build_id)'
  console.log('[dhampir] 预览 wasm ' + buildId + '  host_api=' + hostApi + '  ' + PKG_URL)
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
  dhampirMark('工程 open + pkg 命名空间')
  await dhampirEngine.attach(canvas.id)
  // 新引擎的动图缓存是空的 ⇒ 代际 +1（见 `dhampirAnimGen` 的说明）
  dhampirAnimGen += 1
  // ⚠️ 这两行**必须紧跟在代际 +1 那条语句之后**：判据 ⑪ 会检查"每个 attach 之后 420 字符内
  // 有 +1"，插在中间会把那条判据挤到窗口外（那个窗口是故意的 —— 它要的是"紧跟着"）。
  // 记下"这一页已经接过哪块 canvas"（见本函数开头那道自救刷新的判据），并清掉自动刷新的闸。
  window.__dhampirAttachedCanvas = canvas
  try { sessionStorage.removeItem('yekitrim.dhampir.attachReload') } catch (e) { /* 忽略 */ }
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

  dhampirMark('attach canvas')
  dhampirEngine.resize(canvas.width, canvas.height)
  for (const [assetId, videoId] of bindings) dhampirEngine.bindSource(assetId, videoId)
  dhampirMark('尺寸/绑源')
  // **让源视频播起来** —— followPlayback 只在视频自己正在走时才跳过 seek。
  // **声音：源就是预览的声源。**（2026-10-02 改）
  //
  // 原先这里写死 `v.muted = true`，口径是「预览不带声音；它们只是帧的来源」——
  // 那时 Dhampir 是**可选**模式，与 DOM 预览并列，用户按需切过去看画面，不出声还能接受。
  // 后来 `onMounted` 改成**默认进 Dhampir**（见文件尾那段），于是：
  //   · 台上那块 `<video>` 被静音 ⇒ 说话声/音乐**一点都没有**；
  //   · `tick()` 的 Dhampir 分支在 `player.sync(...)` **之前就 return** ⇒ 音效也没有。
  // 两条一起断的结果就是"预览完全没有声音"，而音量/静音控件还照常显示（它只写台上那块
  // DOM video `applyAudio`，写的是一个已经被静音的元素）—— 症状正是"音量控制不好使"。
  //
  // 现在源按**用户声明的那一份**状态出声，且这两件事是分开的：
  //   · 底座的画面**不看** `muted`（它只取"正在显示的那一帧"，跟播条件只有 `paused`）
  //     ⇒ 出声不影响画面；
  //   · 出片那条腿（`yekitrim polish render` → 转译器）**根本不经过页面**，
  //     音轨由本仓 ffmpeg 混入（`render/sound.rs`）⇒ 这里怎么改都动不到成片。
  // 贴纸：**动图整段交给引擎**（引擎自己解码、按 `source_frame` 取帧），
  // 静态图仍走宿主位图那条路。层描述（活跃区间）从新工程重建 ——
  // startDhampir 与热重载共用 dhampirStickersFrom。
  dhampirStickers = dhampirStickersFrom(project)
  dhampirStickerDiag = null
  Object.assign(dhampirStickerStat, { total: 0, ready: 0, idx: -1, submits: 0, decodes: 0 })

  // **动图交给引擎**（每个资产一次 fetch + 一次 load，之后播放期零请求）。
  // ⚠️ 也必须在 `attach()` **之后**：底座把纹理上传挂在 attach 上，
  // host 还没接上时它只记账时间真值并返回 ok:false（实测错误文案 = "还没 attach 上 canvas"）。
  // 本函数在 startDhampir 里正好位于 attach 之后 —— 顺序别动。
  await dhampirLoadAnimations(project)
  dhampirMark('动图解码')

  // **静态图的预热保留**（2026-10-01 那套）：它们仍走服务端逐帧 PNG，
  // 第一次被要时得整段解码，预热把这件事挪到"用户还没播到那里"的时候。
  // fire-and-forget：不 await、不挡任何东西。
  // ⚠️ 服务端逐张串行解，所以这里**不要**按贴纸并发发多个请求。
  callPrewarmStickers()
  // 同上：**出声**由用户声明决定（`applyAudio` 才是唯一权威，这里只是建源时的初值）。
  // 音高保持也在这里装一次：纠偏会改 `playbackRate`，装晚了会先跑调一小段。
  for (const v of dhampirVideos) {
    dhampirPreservePitch(v)
    v.play().catch(() => {})
  }
  applyAudio()

  // ④ **字幕 / 弹幕**。
  //
  // 工程里那两条资产的 `uri` 是**绝对路径**（指向转译输出目录），浏览器取不到 ——
  // 所以文本由后端在 `/dhampir-project` 里一并带回来（它在 cleanup 之前读）。
  // 这里转成 blob URL 喂给引擎；**解析在 wasm 里**（parse_srt / parse_ass）。
  //
  // 不做这一步的症状是最难查的那一类：**预览看起来正常、只是没有字幕**，还不报错。
  await dhampirApplySubtitles(project, res.subtitles)

  // **把工程声明的字体族装进页面。**
  //
  // canvas 栅格化用的是**页面里已有的字体**：字体没加载就**静默回退**，
  // 而症状正是"字体不对"（字号/位置/颜色都对）。DOM 预览那条路已经在用
  // `ensureFontFace`，这里复用同一个入口 —— 它取的是 `/api/polish/font`，
  // 与出片侧注入的 @font-face 是同一份字节。
  await ensureProjectFonts(project)
  dhampirMark('字体')
  dhampirMarkReport()
  // 首屏期间用户换了片段（`switchDhampirClip` 那时引擎还没起来，只记了待办）：
  // 现在引擎就绪了，补做那次换片段 —— 否则首屏开的是**旧片段**的工程，画面停在上一个片段。
  if (dhampirSwitchPending) { dhampirSwitchPending = false; switchDhampirClip() }
}

function stopDhampir() {
  if (dhampirRaf) {
    cancelAnimationFrame(dhampirRaf)
    dhampirRaf = 0
  }
  const host = dhampirHostRef.value
  if (host) host.innerHTML = ''
  dhampirEngine = null // wasm 侧是单例宿主；下次 attach 会重新接上
  // 动图那条路用的是**命名空间**（见 dhampirPkgNs 的说明），引擎没了它也要一起清 ——
  // 留着的话"已停"的实例还能被调到，而调用会打在一个没有 open 过工程的宿主上。
  dhampirPkgNs = null
  dhampirSegments = []
  dhampirBusy = false
  // 收摊时把热重载的状态一起清掉：留着的话下一次进 Dhampir 会被一个陈旧的 pending 触发。
  dhampirReloading = false
  dhampirReloadPending = false
  // 换片段那套状态也要清：留着的话下次进 Dhampir 会被一个陈旧的待办触发一次无谓的换片段。
  dhampirSwitching = false
  dhampirSwitchPending = false
  if (dhampirReloadTimer) { clearTimeout(dhampirReloadTimer); dhampirReloadTimer = 0 }
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
 * Dhampir 首屏的**分步计时**。
 *
 * 为什么要有它：加载慢的时候只能靠猜（"是不是 wasm 太大""是不是转译慢"）。
 * 2026-10-02 实测过一次 —— wasm 走 304 之后只要 **1 ms**，而 `dhampir-project` 那条
 * 要 **1290 ms**：**大头根本不在 wasm 上**。没有这份数据就只能凭感觉优化。
 *
 * 口径：`mark()` 每调一次就往浏览器控制台打一行；`report()` 收尾给一条汇总 + `console.table`。
 * ⚠️ **走控制台，不走日志面板**：面板默认级别是「信息」，而 debug 级会被滤掉（等于没装）。
 * ⚠️ **不要用 `Date.now()` 之间的差值堆**：这里只用 `performance.now()` 的单调时钟。
 */
const dhampirLoadMarks = []
let dhampirLoadT0 = 0
function dhampirMark(label) {
  if (!dhampirLoadT0) { dhampirLoadT0 = performance.now(); dhampirLoadMarks.length = 0 }
  const at = performance.now() - dhampirLoadT0
  const prev = dhampirLoadMarks.length ? dhampirLoadMarks[dhampirLoadMarks.length - 1].at : 0
  dhampirLoadMarks.push({ label, at })
  // 每一步**当场**打一行：长的那一步你会在控制台里"看着它卡住"，
  // 比等整条链跑完再回头算差值直观得多。
  console.log('[dhampir] ' + label + '  ' + Math.round(at) + 'ms  (+' + Math.round(at - prev) + 'ms)')
  // 顺带把当前那一步写进门帘文案：加载久的时候用户至少知道卡在哪一段
  dhampirStatus.value = '正在加载 Dhampir…（' + label + '）'
}
function dhampirMarkReport() {
  if (!dhampirLoadMarks.length) return
  const total = Math.round(dhampirLoadMarks[dhampirLoadMarks.length - 1].at)
  // ⚠️ **打浏览器控制台，不打日志面板**：面板默认级别是「信息」而这条是 debug ⇒
  // 看不见（等于没装）。控制台不需要切任何过滤，F12 就有。
  const rows = dhampirLoadMarks.map((m, i) => {
    const prev = i > 0 ? dhampirLoadMarks[i - 1].at : 0
    return {
      步骤: m.label,
      累计ms: Math.round(m.at),
      本步ms: Math.round(m.at - prev),
    }
  })
  // 单行摘要（一眼看总账 + 最慢那步），表格（逐步拆开）
  let worst = rows[0]
  for (const r of rows) if (r.本步ms > worst.本步ms) worst = r
  console.log(
    '[dhampir] 首屏 ' + total + 'ms，最慢一步：' + worst.步骤 + ' +' + worst.本步ms + 'ms',
  )
  console.table(rows)
  dhampirLoadMarks.length = 0
  dhampirLoadT0 = 0
}

/**
 * 这一批源现在**是不是在出声**。
 *
 * 判据只有两条（与 `applyAudio` 落到元素上的那两个属性同源）：
 * 没被静音、且音量大于 0。`startDhampir` 建源时把 `muted` 设成**用户声明的那一份**，
 * 之后 `applyAudio` 持续维持，所以这里读到的就是"用户按了静音没有"。
 */
function dhampirSourceAudible() {
  if (muted.value) return false
  const n = Number(volume.value)
  return Number.isFinite(n) && n > 0
}

/**
 * **音高保持**：`playbackRate` 非 1 时让浏览器替我们保住音高。
 *
 * 浏览器实现的时间关系是「以渲染管线为准」的 —— 也就是说 `preservesPitch` 打开后
 * 音轨会被重采样回自然音高，纠偏的 ±% 不再跑调。缺失该属性的实现（老 Safari / 部分
 * WebView）拿不到这个保护，那就只能靠 `DHAMPIR_ALIGN_RATE_AUDIBLE_*` 收窄幅度兜底
 * —— **两道防线都要有**，别只留一道。
 *
 * 返回是否拿到了这个属性（诊断用；拿不到不算错）。
 */
function dhampirPreservePitch(v) {
  let ok = false
  for (const key of ['preservesPitch', 'mozPreservesPitch', 'webkitPreservesPitch']) {
    if (key in v) {
      try { v[key] = true; ok = true } catch (e) { /* 只读实现：忽略 */ }
    }
  }
  return ok
}

/**
 * **纠偏里能用的速率区间**：出声时收窄，无声时用满 ±2%。
 *
 * 为什么不能一刀切用 ±0.5%：无声时源就是"帧的来源"，追得越紧画面越跟手，
 * 而 ±2% 能在不走 hard seek 的前提下把偏差压回去（实测硬 seek 是 307ms/帧）。
 * 出声之后速率连着音高，才需要让位给音质。
 *
 * ⚠ 这个函数与 `dhampirAlignSources` 里的用法必须成对出现 —— 那里是唯一的调用点。
 */
function dhampirAlignRateRange() {
  if (!dhampirSourceAudible()) return { lo: DHAMPIR_ALIGN_RATE_LO, hi: DHAMPIR_ALIGN_RATE_HI }
  if (dhampirSourcesPitchLocked()) return { lo: DHAMPIR_ALIGN_RATE_LO, hi: DHAMPIR_ALIGN_RATE_HI }
  return { lo: DHAMPIR_ALIGN_RATE_AUDIBLE_LO, hi: DHAMPIR_ALIGN_RATE_AUDIBLE_HI }
}

/**
 * 这一批源是不是**都**拿到了 `preservesPitch`。
 *
 * 全都拿到 ⇒ 速率随便动都不跑调，用满 ±2%；
 * 有一个没拿到 ⇒ 整批收窄（宁可漂一点，也不要一路跑调）。
 */
function dhampirSourcesPitchLocked() {
  if (!dhampirVideos.length) return false
  for (const v of dhampirVideos) {
    if (!(('preservesPitch' in v) || ('mozPreservesPitch' in v) || ('webkitPreservesPitch' in v))) return false
  }
  return true
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
  // ⚠️ **遍历 `dhampirAllSourceVideos()` 而不是 `dhampirVideos`**（用户报的"拖动之后没声音"）。
  //
  // `dhampirVideos` 只是"台上那批"的**数组视图**，而 `dhampirSwapSources` 换绑时会把旧元素
  // 从它里面换出去、同时记成新的备用（`dhampirTwins`）—— 也就是**换台那一刻旧元素掉出这个数组**，
  // 而它**还在播、还是声源**。`applyAudio` 早就为同一个原因改用了并集视图（那里有一段长注释），
  // 但**这里漏了**：于是拖动时这一份落不到那些元素上，它们停在旧时刻继续出声
  // ⇒ 听起来就是"拖动之后没声音了"（其实是位置没跟上，或被后续的 seek 弄停）。
  //
  // 并集视图（当前绑定 + 备用 + 数组兜底，Set 去重）正是"所有可能出声的元素"的定义。
  const seen = new Set()
  for (const v of dhampirAllSourceVideos()) {
    if (!v || seen.has(v)) continue
    seen.add(v)
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
 *   ② |err| < 500ms → **只改 `playbackRate`**（超前 `base×lo` / 落后 `base×hi`，
 *      由 `dhampirAlignRateRange()` 取：**无声时** ±2%（追得紧、画面跟手），
 *      **出声时**收窄到 ±0.5%、且源都拿到 `preservesPitch` 时仍可用满 ±2% —— 详见那里的说明），
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
  // ⚠️ 速率区间**按"出不出声"取**（2026-10-02）：出声时 ±2% 会跑调，见
  // `dhampirAlignRateRange` 的说明。整批一次取定，不在循环里逐元素问。
  const { lo, hi } = dhampirAlignRateRange()
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
    const rate = err > 0 ? base * lo : base * hi
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
    // **换片段待办**：首屏还没起来时点的、或撞上"有帧在飞"被拦下的那次 —— 每帧开头补跑
    // （与下面 `dhampirReloadPending` 同一条纪律：**待办必须有人消费**，否则那次换片段
    //  静默丢掉，画面停在上一个片段而用户以为已经切过去了）。
    if (dhampirSwitchPending && !dhampirSwitching) {
      dhampirSwitchPending = false
      switchDhampirClip()
    }
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
      // **动图不再逐帧交位图**（2026-10，host_api v6）：GIF/动画 WebP 的字节在建源时
      // 已整段交给引擎（`dhampirLoadAnimations`），此后由引擎自己解码、自己按
      // `source_frame` 取帧 —— 播放期**零请求、零宿主操作**。
      //
      // ⚠️ **但静态图仍要走位图那条路**：引擎的动图这条路只收动图，
      // 单帧图（PNG/JPEG/静态 WebP）它明确拒收（`AnimError::NotAnimation`）。
      // 所以这一段只处理 `kind !== 'image_sequence'` 的那几个。
      //
      // ⚠️ 也别再对动图调 `setSourceBitmap`：底座把动图缓存排在 `texture_for` **最前**，
      // 宿主再交位图就是两边抢 —— 症状是贴纸**永远钉在宿主给的那一帧**（看起来只是"动图不动了"）。
      onPrepared: async () => {
        dhampirStickerStat.total = dhampirStickers.length
        dhampirStickerStat.ready = dhampirStickers.filter((x) => x.animLoaded || x.bytes).length
        for (const st of dhampirStickers) {
          if (st.animLoaded) continue          // 动图：引擎供帧，这里什么都不做
          // 剩下的只有静态图（走老路：按秒算不出帧，它本来就只有一帧）
          let sec = 0
          const span = st.spans.find((sp) => f >= sp.start && f < sp.end)
          if (!span) { st.lastFrame = -1; continue }           // 这一帧不活跃
          sec = (span.sourceIn + (f - span.start) * (st.fps / dhampirFps.value)) / st.fps
          const idx = Math.max(0, Math.floor(sec * st.fps + 1e-6))
          try {
            if (idx !== st.bytesIdx) {
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
              dhampirStickerStat.decodes += 1
            }
            if (!st.bytes) continue
            // ⚠️ **每帧都必须交**：引擎每帧 `prepare` 开头都会 `clear_bitmaps()`，
            // 不交的那一帧这个源没有位图 → `require_bitmap` 直接不画那一层。
            // 而且位图**每帧要是新对象**：wasm 换掉旧位图时会 `close()` 它，
            // 复用会拿到"已关闭的位图"（不报错、只是不显示）。
            if (st.canvasIdx !== idx || !st.canvas) {
              const bmp = await createImageBitmap(new Blob([st.bytes], { type: 'image/png' }))
              if (!st.canvas) st.canvas = document.createElement('canvas')
              st.canvas.width = bmp.width
              st.canvas.height = bmp.height
              st.canvas.getContext('2d').drawImage(bmp, 0, 0)
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
            // 动图与静态图现在是**两条路**，读数也要分开说 ——
            // 混在一起念（"就绪 4/4 · 已交 0"）会被读成"贴纸没交上去"，而实际是引擎在供帧。
            const anim = dhampirStickers.filter((x) => x.animLoaded).length
            const animErr = dhampirStickers.filter((x) => x.animError).length
            const bits = ['贴纸 ' + dhampirStickerStat.ready + '/' + dhampirStickerStat.total]
            if (anim > 0) bits.push('引擎解码 ' + anim)          // GIF/WebP：交给引擎了
            if (animErr > 0) bits.push('失败 ' + animErr)         // 加载期就报，别等渲染
            // ⚠️ **延迟表冲突必须露出来**：它不会让贴纸"不动"，只会让它**越播越偏** ——
            // 静默的话就变成最难归因的那类问题（细节见 docs/webui-components.md ②.4）。
            if (dhampirStickerStat.delayConflict > 0) bits.push('延迟表冲突 ' + dhampirStickerStat.delayConflict)
            if (dhampirStickerStat.submits > 0 || anim < dhampirStickerStat.total) {
              // 只有静态图才走位图那条路；全动图时这一栏本就该是 0
              bits.push('本帧 idx ' + dhampirStickerStat.idx)
              bits.push('取帧 ' + dhampirStickerStat.decodes)
              bits.push('已交 ' + dhampirStickerStat.submits)
            }
            dhampirStickerInfo.value = bits.join(' · ')
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
        // 重载请求撞上"有帧在飞"被拦下（见 reloadDhampirProject）：这一帧收尾了就补跑，
        // 否则那次改动会**静默丢掉**（用户看到的就是"改了没反应"）。
        if (dhampirReloadPending && !dhampirReloading) {
          dhampirReloadPending = false
          reloadDhampirProject()
        }
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
  resolveUrl: (name) => clipMediaUrl(props.pid, props.dir, 'sfx/' + name + '.mp3'),
  // 音效加载失败（文件缺失 / 格式不对）以前是纯静默 —— "预览没有音效"连个可查的痕迹都没有。
  // 每个 name 只报一次，走 debug（可预期失败不刷 warn，见前端留痕纪律）。
  onError: (name) => addClient('debug', `音效 sfx/${name}.mp3 加载失败 —— 预览里这条不会响（出片侧同样会跳过它）`, 'preview'),
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
  axis: 'fill', object_fit: 'cover', position: 'center',
  position_x_pct: 50, position_y_pct: 50, free_x: 0, free_y: 0,
  bg: 'none', bg_color: '#000000', blur_px: 28, blur_img_scale: 1.187, scale: 1, padding: 0
}
/** 老接口只有 `vertical_position`（三档枚举）→ 回落成 X 轴百分比（`auto`/居中 → 50）。 */
const legacyPositionPct = (v) => (v === 'left' || v === 'top' ? 0 : v === 'right' || v === 'bottom' ? 100 : 50)
const stageFit = computed(() => {
  const s = props.config?._canvas?.stage
  if (s) return s
  return vertical.value
    ? { ...FALLBACK_STAGE, axis: 'x', object_fit: 'contain', bg: 'blur',
        position_x_pct: legacyPositionPct(props.config?.vertical_position) }
    : FALLBACK_STAGE
})
/** 背景层（当前帧模糊）只在 bg=blur 时需要；black / color 走舞台底色 */
const bgOn = computed(() => stageFit.value.bg === 'blur')
/** 基线**恒为居中**：连续位置（两轴）由 `measureStageShift()` 按「实测余量 × (pct/100 − 0.5)」补，
 *  与出片模板同一条式子（那边也只加 `stage-center`）。 */
const stageClass = computed(() => ['axis-' + (stageFit.value.axis || 'fill'), 'stage-center'])
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

// ---- 连续位置的 DOM 修正（`[stage] position_x/y_pct`，2026-10-05 两轴）----
// `#video-wrap` 的基线**恒为居中**，而契约里的位置是**两轴各 0–100**（五档吸附）。
// 这里按**实测**余量把 `.fit-box` 挪到那一档：居中 ⇒ 只补 `free × (pct/100 − 0.5)`，两端天然精确。
// ⚠️ 与出片那条腿（模板 `stageDrawRect` 的 `bx+(bwid-w)*px`）是**同一条线性插值**、**同一条顺序**
//   （先定画面大小 → 绕盒心缩放 → 再插值）：所以余量必须量**缩放后**的画面（`clientWidth * scale`）
//   —— `transform` 不改布局尺寸，不乘就会在 `scale≠1` 时整体漂移。
// ⚠️ 减掉 `getComputedStyle` 的 padding：`--stage-pad` 是百分比、浏览器已解析成 px，
//   而 `clientWidth` 含 padding（不减掉会整体偏一个 padding）。
const fitBoxRef = ref(null)
const stageShift = ref({ dx: 0, dy: 0 })
function measureStageShift() {
  const box = fitBoxRef.value
  const wrap = box ? box.parentElement : null
  if (!box || !wrap) return
  const s = stageFit.value
  const sc = Number(s.scale) || 1
  const pad = parseFloat(getComputedStyle(wrap).paddingLeft) || 0
  const freeX = wrap.clientWidth - 2 * pad - box.clientWidth * sc
  const freeY = wrap.clientHeight - 2 * pad - box.clientHeight * sc
  const px = typeof s.position_x_pct === 'number' ? s.position_x_pct : 50
  const py = typeof s.position_y_pct === 'number' ? s.position_y_pct : 50
  stageShift.value = { dx: freeX * (px / 100 - 0.5), dy: freeY * (py / 100 - 0.5) }
}
const fitBoxStyle = computed(() => ({
  transform: 'translate(' + stageShift.value.dx + 'px,' + stageShift.value.dy + 'px) scale('
    + (Number(stageFit.value.scale) || 1) + ')'
}))
// 测量时机：契约任一影响几何的字段变化后（下一帧再量，等布局落定）
watch(() => {
  const s = stageFit.value
  return [s.axis, s.position_x_pct, s.position_y_pct, s.padding, s.scale,
    props.config?._canvas?.width, props.config?._canvas?.src?.width].join('|')
}, async () => { await nextTick(); measureStageShift() })

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
/** 兜底字体栈：页面透传的 props.fontFallback（服务端动态算、与成片同源）。
 *  ⚠️ **空串必须回落到字面量**：Vue 的 prop 默认值只在 undefined 时生效，而页面侧在
 *  旧 server（`/api/polish/fonts` 还没有 `fallback_stack` 字段）下传来的是空串 ——
 *  直接拼进 font 简写会得到 `'700 72px '` 这种**非法值**，canvas 的 font 赋值被静默忽略，
 *  量宽/折行全乱（症状是预览字幕排版崩坏）。 */
const capFallback = computed(() => {
  const base = (props.fontFallback || '').trim()
    || '"Noto Sans SC","Microsoft YaHei",sans-serif'
  // ⚠️ **emoji 追加在末尾**（职责分离）：正文在前、emoji 只兜底别人都没有的字形。
  // 排在末尾而不是开头，是因为排在前面会**抢走**数字/字母这类它也有的字形（观感突变）。
  // 库里没有 emoji 字体时不追加 —— 浏览器还有系统 emoji 那层兜底，别把手写死。
  const em = (props.emojiFamily || '').trim()
  return em ? `${base},"${em}",sans-serif` : base
})
/** 量宽用的字体简写。⚠️ 兜底栈走 capFallback（服务端动态算、与成片同源）——
 *  这里曾写死 `"LXGW WenKai",…`，"设为默认"对预览的量宽/折行完全无效。 */
const capFontSpec = computed(() => '700 ' + (subStyle.value.size || 72) + 'px '
  + (subStyle.value.font ? '"' + subStyle.value.font + '",' : '') + capFallback.value)
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
  // **字幕垂直落点**：`position_pct` 存在时按百分比，否则保持历史上那条 CSS 的 80px。
  //
  // ⚠️⚠️ **容器底边 ≠ 文字带底边，要补两项**（2026-10 复核发现，缺了会跟成片恒差、且不报错）：
  //   ① `.cap` 自己 `padding:14px 0`（见下面 `.cap` 的样式）⇒ 文字带底边比容器底边高 14px；
  //   ② `.cap` 是 `line-height:1.5`，最后一行 inline box 的**底边**在字形底边之下
  //      `(lh - size)/2 = 0.25 × size` ⇒ 再多 N*0.25×size…… 只有**最后一行**那一份（`0.25 × size`）。
  //   合计：`14 + 0.25 × size`。
  //
  // ⚠️ **这条"缺省时看不出、一用新控件就恒差"**：缺省 `--cap-bottom` 是 `80px`，
  // 而 `80 - (14 + 0.25×72) = 48`…… 老路径之所以没问题，是因为**缺省走的是 `.settings` 里那条
  // `80 * v.kh`**、并且出片侧 `CH-120` 那套是"视觉对齐"的历史口径 —— 两边本来就是照着
  // "看起来一致"调出来的。**所以缺省路径一个字节都不许动**（下面 `posPct === null` 分支）。
  //
  // ⚠️ 偏差**不是常数**：size 48→120 时它从 26px 涨到 44px。也就是说"字越大错位越大"，
  // 而「字幕位置」恰恰是视觉最敏感的那一项 —— 所以必须按 size 补，不能写死 14。
  const CAP_PAD_Y = 14 // 与 `.cap { padding:14px 0 }` 同值 —— 改一边必须改另一边
  const capSize = s.size || 72
  const capAnchorFix = CAP_PAD_Y + 0.25 * capSize
  const posPct = typeof s.position_pct === 'number' && Number.isFinite(s.position_pct)
    ? Math.min(50, Math.max(0, s.position_pct))
    : null
  return {
    // ⚠️ 兜底栈 = capFallback（服务端动态算、与成片同源，空串回落字面量）；选了库内字体时排最前。
    // 这里曾写死 `"${s.font || 'LXGW WenKai'}","Microsoft YaHei",…` —— "设为默认"对预览无效。
    fontFamily: s.font ? `"${s.font}",${capFallback.value}` : capFallback.value,
    fontSize: capSize + 'px',
    color: s.color || '#dcbda0',
    WebkitTextStroke: s.stroke || '12px #403c3b',
    '--hl-color': s.highlight_color || '#f56b41',
    ...(posPct === null ? {} : { '--cap-bottom': `calc(${posPct}% + ${capAnchorFix}px)` })
  }
})
const dmLayerStyle = computed(() => {
  const s = dmStyleCfg.value
  return {
    // `[style.danmaku] font` 是**字体库 id**（与字幕同一套取法）：选了就排在最前，
    // 没选就跟随字幕那条动态兜底栈（`capFallback`，服务端同源）—— 别写死字体名。
    fontFamily: s.font ? `"${s.font}",${capFallback.value}` : capFallback.value,
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
// 只发**一次**请求，把当前工程里**还需要服务端解码的那些**贴纸 uri 一起交上去；
// 服务端在 `blocking` 池里逐张解。
//
// ⚠️ **动图不再交上去**（2026-10-02，host_api v6）：它们的字节已经整段给了引擎，
// 服务端再解一遍纯属白烧 CPU（实测单张冷解码 3~9 秒）。只留静态图 ——
// 那几个仍走 `/polish/sticker-frame`，预热对它们还是有效的。
// 失败按 `debug` 记 —— 它是纯优化，失败不该在日志面板里刷 warn 让人以为出了错。
function callPrewarmStickers() {
  const assets = (dhampirStickers || [])
    .filter((s) => !s.animLoaded)
    .map((s) => s.uri)
    .filter(Boolean)
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
          // ⚠️ **换台会一次跳过整段，换完必须补一次 sync**（2026-10-02 实测：
          // "有些音效播了、有些没有"）。这条路径上 `v.currentTime` 从旧位置**直接跳到
          // 目标位置**，中间那一段的音效没有任何一帧能覆盖 —— 少了这一句就永久丢失。
          player.sync(idx.value.soundEvents, v.currentTime)
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
    // ⚠️ **音效不能跟着"让位"一起让掉**（2026-10-02 修）。
    // 上面"让出重活"让的是 DOM/CSS 复刻那套**渲染**（字幕/弹幕/贴纸被 canvas 盖住，
    // 继续跑只是白烧 CPU）；而 `player.sync` 是**出声**，与画面无关。
    // 原先它在下面那行（非 Dhampir 路径），Dhampir 分支提前 return ⇒ 音效整条不触发，
    // 症状是"预览一点声音都没有"（连音效都没有）。
    player.sync(idx.value.soundEvents, v.currentTime)
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
  // ⚠️ **Dhampir 加载期间不许播**（第二道闸；第一道是模板上那颗按钮置灰）：
  // `togglePlay` 还经 `defineExpose` 给父级（PolishView 的快捷键 / 时间轴的 toggle）用 ——
  // 只禁按钮挡不住它们。半途播出来的是缺效果/空白的画面，而加载失败时它会被当成「这一版就这样」。
  if (dhampirLoading.value) return
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
  // ⚠️ **拖动之后把引擎源叫醒**（用户报的"拖动的时候直接没声音了"）。
  //
  // 为什么必须有：`syncDhampirTime` 只**设位置**，不负责让元素继续播。而拖动一次就是
  // 在一条 6 Mbps 的流上 seek —— 浏览器会**丢掉旧的解码缓冲、等待新数据**，表现上等同于
  // 源被暂停。时钟（`videoRef`）是另一块元素、且在 Dhampir 下 `muted = true`，
  // 它照旧在走 ⇒ 没有任何东西会把源重新 `play()` 起来：
  //   · `dhampirAlignSources` 末尾那条兜底要求 `clockRunning`，而拖动期间时钟可能被判为
  //     暂停（`el.paused`）⇒ 它直接 return，兜底**不生效**；
  //   · tick 里的镜像循环只在「`v.paused !== el.paused`」时才动 —— 源因 seek 停下、
  //     时钟照旧在走，两者**状态看起来一致**时会漏掉。
  // 于是源停在拖动后的位置不出声，而时间轴还在走。这里在 seek 之后显式续播一次。
  //
  // ⚠️ 只在时钟**本来就在走**时续播：时钟是暂停态（用户在拖一条暂停的预览）时把源
  // play 起来就成了"画面停着、声音在播"。
  // ⚠️ 用 `dhampirAllSourceVideos()`（并集视图）而不是 `dhampirVideos` —— 理由与
  // `syncDhampirTime` 里那段相同：换台中间态里旧元素会掉出数组视图。
  if (!v.paused && !v.ended && dhampirMode.value) {
    // `dhampirAllSourceVideos()` 的返回值先落到局部变量再遍历 —— 写法与 `applyAudio` 里
    // 那段**故意不同**：`scripts/check-preview-audio.mjs` 的变异台要求
    // `for (const dv of dhampirAllSourceVideos()) {` 这个片段在 SFC 里**恰好命中一次**
    // （它给 `applyAudio` 那条断言做变异自证），这里再抄一遍会让变异台自己报错。
    const wake = dhampirAllSourceVideos()
    for (const dv of wake) {
      // 备用那套是预滚用的、**永远不该出声**（见 `applyAudio`），别把它叫醒。
      if (!dv || dv.muted || dhampirIsTwin(dv)) continue
      if (dv.paused) dv.play().catch(() => {})
    }
  }
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
 * 音量/静音同时作用在**所有真正在响的 <video>** 和 SFX 层。
 * 之前只调 SFX 播放器，视频里的说话声/音乐丝毫不受影响 —— 用户反馈的「音量控制不好使」就是这个：
 * 点静音、拉滑块都对真正在响的那一路没作用。
 *
 * ⚠️ **Dhampir 模式下"台上那块"不是唯一在响的元素**（2026-10-02 修）：
 * `dhampirVideos` 是**另一批** `<video>`（引擎的帧来源，同时就是声源），
 * `videoRef` 指向的那块只是时钟/DOM 预览那一路。只写 `videoRef` 时，
 * 用户拖滑块 → Dhampir 那批源音量纹丝不动 ⇒ 控件的表现就是"不好使"。
 * 所以这里两批一起写：`videoRef`（DOM 预览 / 时钟）+ `dhampirVideos`（引擎那一路的声）。
 *
 * 备用/预滚那批（`dhampirTwins`）**刻意不出声**（它们永远 `muted`）——
 * 换绑瞬间两套源同时在轨，不压住会叠成两倍音量。
 */
function applyAudio() {
  const n = Number(volume.value)
  const vol = Number.isFinite(n) ? Math.min(1, Math.max(0, n)) : 0
  const m = !!muted.value
  const v = videoRef.value
  if (v) {
    v.volume = vol
    // ⚠️ **Dhampir 模式下台上那块必须闭麦**（2026-10-02 实测：声音播了两遍）。
    //
    // DOM 那条腿与引擎那条腿**指向同一段素材**：
    //   · DOM 腿  = `stage0Ref`/`stage1Ref` 这对 A/B `<video>`（模板里**没有** `muted` 属性，
    //                它就是自绘预览的声源）
    //   · 引擎腿 = `dhampirVideos` 那批（同时是引擎的帧来源与声源）
    // 两条腿都出声 = 同一句话听两遍（还带一点相位差，听着像"回声/糊"）。
    //
    // 进 Dhampir 时 `toggleDhampir` 只 `pause()` 了台上那块**一次**，但
    // `scheduleDhampir` 的镜像循环每帧把引擎源对齐到台上那块的状态
    // （`v.paused !== el.paused` ⇒ play/pause）—— 只要台上那块被任何路径重新 play 起来
    // （快捷键 / 时间轴 toggle / 预滚换台），它就又出声了。**光靠 pause 挡不住，必须闭麦。**
    v.muted = m || dhampirMode.value
  }
  // 引擎那一路：**台上那批 + 备用那批，一个都不能漏**。
  //
  // ⚠️ 不能只遍历 `dhampirVideos`（2026-10-02 实测踩到，由 `audiotest` 钉住）：
  // `dhampirSwapSources` 换绑时把旧元素**从 `dhampirVideos` 里换出去**
  // （`dhampirVideos[i] = tw.el`），同时把它记成新的备用（`dhampirTwins.set(assetId, {el: old})`）。
  // 于是那一刻旧元素**不在** `dhampirVideos` 里 —— 只遍历台上一批的话，
  // 它既不会被压回静音、也不会被写音量，而它**还在播**（`pause()` 之后下一帧镜像循环
  // 又会按时钟把它 play 起来）⇒ 换台之后**两路同源同时在轨、声音叠成两倍**。
  // 所以这里按 `dhampirSourceVideos`（引擎当前认的那批）+ `dhampirTwins`（备用那批）
  // 合并遍历，用 **Set 去重**（`dhampirVideos` 是它们的并集视图，不再单独遍历）。
  // 音效播放器（SFX）**也是出声的一条腿**，必须一起铺。
  //
  // ⚠️ 以前它只由 `toggleMute` / `onVolume` 两处直接调 `setMuted`/`setMasterVolume` 管，
  // 而 `applyAudio()` 完全不碰它 —— 于是**任何不经过那两个控件的路径**（进/出 Dhampir、
  // 建源、换绑、恢复播放）都不会把音量铺到它身上：音量滑块看着有效，实际只对
  // 两条视频腿生效，音效仍按上一次的值出声。
  player.setMasterVolume(vol)
  player.setMuted(m)
  const seen = new Set()
  for (const dv of dhampirAllSourceVideos()) {
    if (!dv || seen.has(dv)) continue
    seen.add(dv)
    dv.volume = vol
    // ⚠️ **备用那套不给它解除静音** —— 换台前后两套源指向同一段素材，
    //    备用那块若出声就会与台上那块叠成两声。判据用"是不是 twin"，不用下标。
    dv.muted = m || dhampirIsTwin(dv)
  }
}

/**
 * 引擎侧**全部** `<video>` 源：当前绑定那批 + 备用（预滚）那批。
 *
 * `dhampirVideos` 只是"台上那批"的数组视图，换绑的**中间态**里旧元素会掉出它
 * —— 而那个瞬间它仍然在播（见 `applyAudio` 里那段）。要覆盖"所有可能出声的元素"，
 * 就得从这两个 Map 取并集。
 */
function dhampirAllSourceVideos() {
  const out = []
  for (const v of dhampirSourceVideos.values()) out.push(v)
  for (const tw of dhampirTwins.values()) { if (tw && tw.el) out.push(tw.el) }
  // 兜底：换绑/初始化的中间态里，数组视图可能已经拿到而 Map 还没跟上
  for (const v of dhampirVideos) out.push(v)
  return out
}

/** 这块元素是不是 A/B 里**备用那套**（预滚用，永远不出声）。 */
function dhampirIsTwin(el) {
  for (const tw of dhampirTwins.values()) {
    if (tw && tw.el === el) return true
  }
  return false
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

// ---- 区域选框的坐标系（2026-10-05，④ 定案 W1）----------------------------------
// `layout.<区域>.top_left/bottom_right` 的百分比基准是 **主画面矩形 `sr`**（源帧经画面构成摆到
// 画布上的那一块），**不是整块画布** —— 模板（canvas 出片路径）、`engine.js`、转译器与 Rust
// `process::placement` 都按 `sr` 读。而**拖拽是在舞台里进行的**（整块画布 = 100%×100%）。
// 编辑端过去把舞台百分比**直接**写进 `layout`：只要 `sr ≠ 整块画布`（源比例≠画布 / `scale≠1` /
// `padding≠0`），存下来的区域与预览/出片所指的位置就不是同一块 —— 这就是"区域与落点对不上"的
// 根因之一。所以这里只做**两个换算**：存盘（`emit`）走 `stage→sr`，显示（`selBoxStyle`）走
// `sr→stage`。
// ⚠️ **点选落点不在此列**：`[sticker] position` 是另一个键、另一套基准（盒子中心百分比），
// `pctAt` 仍然给舞台坐标 —— 别把这两个换算合并成一个（那会把落点一起改错）。
// `sr` 直接量台上那块 `<video>` 的盒子：`.fit-box` 带 `--src-ar` 的 `aspect-ratio` 且
// `object-fit:cover`，所以元素框 **就是**画面上画出来的那块；`axis=fill` 时它等于整块舞台，
// 两个换算退化成恒等（横屏存量零变化）。
const stageRectPx = () => {
  const el = stageRef.value
  if (!el) return null
  const b = el.getBoundingClientRect()
  return b.width && b.height ? b : null
}
/** `sr` 在舞台百分比坐标里的矩形；量不到（还没挂载/尺寸为 0）时返回 null，两个换算退化为恒等。 */
const srInStage = () => {
  const st = stageRectPx()
  const v = videoRef.value
  const vb = v && v.getBoundingClientRect ? v.getBoundingClientRect() : null
  if (!st || !vb || !vb.width || !vb.height) return null
  return {
    x: ((vb.left - st.left) / st.width) * 100,
    y: ((vb.top - st.top) / st.height) * 100,
    w: (vb.width / st.width) * 100,
    h: (vb.height / st.height) * 100
  }
}
const clamp100 = (v) => Math.min(100, Math.max(0, v))
/** 舞台百分比 → `sr` 百分比（**写盘**用）：`sr` 之外的部分夹回边界（区域语义定义在 `sr` 内）。 */
function stageToSrRect(r) {
  const s = srInStage()
  if (!s || s.w <= 0 || s.h <= 0) return r
  const a = { x: clamp100(((r.x0 - s.x) / s.w) * 100), y: clamp100(((r.y0 - s.y) / s.h) * 100) }
  const b = { x: clamp100(((r.x1 - s.x) / s.w) * 100), y: clamp100(((r.y1 - s.y) / s.h) * 100) }
  return {
    x0: Math.min(a.x, b.x), y0: Math.min(a.y, b.y),
    x1: Math.max(a.x, b.x), y1: Math.max(a.y, b.y)
  }
}
/** `sr` 百分比 → 舞台百分比（**显示**用）：让用户看到自己画的那一块。 */
function srToStageRect(r) {
  const s = srInStage()
  if (!s || s.w <= 0 || s.h <= 0) return r
  const a = { x: s.x + (r.x0 / 100) * s.w, y: s.y + (r.y0 / 100) * s.h }
  const b = { x: s.x + (r.x1 / 100) * s.w, y: s.y + (r.y1 / 100) * s.h }
  return {
    x0: Math.min(a.x, b.x), y0: Math.min(a.y, b.y),
    x1: Math.max(a.x, b.x), y1: Math.max(a.y, b.y)
  }
}

const selBoxStyle = computed(() => {
  const r = selNorm(srToStageRect(props.selectRect))
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
  const r0 = handle === 'new' ? { x0: p.x, y0: p.y, x1: p.x, y1: p.y } : selNorm(srToStageRect(props.selectRect))
  selDrag = { handle, sx: p.x, sy: p.y, r0 }
  if (handle === 'new') emitRect(r0)
  window.addEventListener('pointermove', onSelMove)
  window.addEventListener('pointerup', endSel)
  window.addEventListener('pointercancel', endSel)
}

function emitRect(r) {
  // 对外（父级 → `layout`）一律是 `sr` 百分比；舞台坐标只活在本组件内部（见上方换算注释）。
  const sr = stageToSrRect(r)
  emit('update:selectRect', { x0: r2(sr.x0), y0: r2(sr.y0), x1: r2(sr.x1), y1: r2(sr.y1) })
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
//      浏览器 WebGPU 里跑 `yekitrim-render-core` 的合成管线，逐帧 `render_frame`）；
//   ② 回退的 server 单帧（`POST /api/polish/render-frame`，桌面 wgpu 后端渲染一帧）。
// 两条都随「作废 wgpu」一起删除（用户拍板：GPU 那侧交给独立的 Dhampir 底座）。
// 预览的事实来源始终是 `polish/engine.js` 的自绘 DOM/GSAP 层 —— 与出片模板
// `templates/index.html` 的 canvas 路径同口径。**别再把这套接回来**：要真渲染预览就走底座那条链路。
watch(idx, (v) => {
  player.setEvents(v.soundEvents)
  player.setLoudnessCurve(props.config?.loudness_curve || [])
}, { immediate: true })

// ---- Dhampir 工程热重载：**磁盘上的 polish.toml 变了**就把新工程推进引擎 ----
//
// ⚠️ **认的是 `persistedConfig`，不是 `config`**（2026-10-02 修）。
//
// 这条缝的第一版挂在 `props.config` 上，而那是**页面手里的镜像**：编辑器拖动时会
// 60ms 节流地把它换成内存中间态（`onTimelinePreview`，**不落盘**）。于是每次拖拽都会
// 去磁盘要一份**还没写下去**的改动 —— 拿回旧工程、`open()` 覆盖掉画面，
// 表现是"拖完松手，画面先闪回改动前的样子，等后处理跑完才追上"。
//
// 现在两个信号各有明确含义，**触发两次、各管一段**：
//   ① `persistedConfig` 变（落盘完成）→ 反映**你手改的值**（快，松手就变）
//   ② 任务跑完 `load()` 之后再更新它 → 反映**后处理的校准结果**（慢，但与出片一致）
//
// 深比较：父级每次都造一个新对象，浅比较 `!==` 会误判成"变了"；而配置本来就小。
// 防抖 300ms：连续改动（拖滑块 / 连点）只重载最后一次。
watch(() => props.persistedConfig, (cfg) => {
  // 只认"能进 Dhampir 工程"的那几棵子树（= 转译器真正读的那些，见
  // `tools/polish-to-dhampir.mjs`）：别让无关字段触发重载，也别漏掉真正相关的。
  // 字段名对着 `ProjectFile`（`yekitrim-polish/src/project/config.rs`）抄，别自己编。
  const key = JSON.stringify([
    cfg && cfg.segments, cfg && cfg.mute_ranges, cfg && cfg.silence_ranges,
    cfg && cfg.layout, cfg && cfg.canvas, cfg && cfg.stage, cfg && cfg.sticker,
    cfg && cfg.style, cfg && cfg.filter, cfg && cfg.sfx,
    cfg && cfg.sticker_source, cfg && cfg.duration,
    // 字幕**不在 polish.toml 里**（它住 `data/subtitles.json`）⇒ 只改字幕时上面这些子树
    // 一个字节都不变。所以要单独把"磁盘上那份字幕"算进指纹，否则那次改动永远不触发重载。
    props.persistedSubtitles,
  ])
  if (key === dhampirCfgKey) return
  dhampirCfgKey = key
  if (!dhampirMode.value || !dhampirEngine) return
  // ⚠️ **换片段期间由 `switchDhampirClip` 负责那一次工程替换**（它本身就是"取新片段工程"）：
  // 这里再排一次热重载就是把同一份工程开两遍（还有一次会把**换到一半**的状态顶掉）。
  if (dhampirSwitching || dhampirSwitchPending) return
  // 工程重载防抖：**150ms**（原 300ms）。它是"落盘完成 → 画面跟上"这段路上除了落盘防抖之外
  // 唯一的等待，而落盘是离散事件（一次编辑一次 PUT），所以只需盖住"保存紧接着又一次保存"
  // 的合并；两个防抖加起来从 ~1s 收到 ~0.4s。别再往上抬，也别拿掉（拿掉会把
  // 「手改的值」与「后处理校准结果」两次重载挤成同一帧，反而看不到中间态）。
  if (dhampirReloadTimer) clearTimeout(dhampirReloadTimer)
  dhampirReloadTimer = setTimeout(() => { dhampirReloadTimer = 0; reloadDhampirProject() }, 150)
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
    measureStageShift()
  })
  if (stageRef.value) ro.observe(stageRef.value)
  measureStageShift()
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

/** 换片段（`props.dir` 变了）：**原地**把 Dhampir 那条腿换到新片段上。
 *
 * ⚠️ 父级切片段是**静默加载**（见 `PolishView` 的 `watch(clipName)`）—— 就是为了不卸载本组件、
 * 不换掉那块 canvas（底座一页只 attach 一次、没有 detach ⇒ 重挂必黑屏，只能 F5）。
 * 所以"跟着换"这件事归这里：工程/源/字幕/贴纸/字体整条换，播放头回 0。
 * ⚠️ 别在这里 `stopDhampir() + startDhampir()`：那会重建引擎与全部源、画面闪一下，
 * 而且**第二次 attach 在底座上是空转**（新 canvas 从没被 present 过）。
 */
watch(() => props.dir, (dir, old) => {
  if (!dir || dir === old) return
  switchDhampirClip()
})
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
/* Dhampir 加载门帘：铺满舞台、压在**定格遮罩（60）之上**。
   ⚠️ `pointer-events:none` —— 门帘只负责「说清在等什么」，「不许播」由按钮置灰 + `togglePlay` 那道闸负责
   （否则框选 / 点选那两层会被它挡掉）。
   ⚠️ 颜色写死在这里是**有意的**：它盖在视频上，必须与主题无关（浅色主题下也不能变成白底黑字）——
   与 `.dhampir-canvas{background:#000}` / `.stage{background:#000}` 同一条口径。 */
.dhampir-loading {
  position: absolute; inset: 0; z-index: 70;
  display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px;
  background: rgba(0, 0, 0, .62); color: #fff; font-size: 13px; text-align: center;
  pointer-events: none;
}
.dhampir-loading .dl-text { font-weight: 600; }
.dhampir-loading .dl-spin {
  width: 22px; height: 22px; border-radius: 50%;
  border: 2px solid rgba(255, 255, 255, .28); border-top-color: #fff;
  animation: dl-spin .8s linear infinite;
}
@keyframes dl-spin { to { transform: rotate(360deg); } }
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
.dm { position:absolute; white-space:nowrap; font-weight:600; -webkit-text-stroke:2px #000; paint-order:stroke fill;
      /* ⚠️ 必须 inherit：`app.css` 的 `*{font-family:var(--font)}` 是**直接命中**本元素的声明，
         **压过**父层 `.danmaku-layer` 的内联 font-family（继承值永远输给本元素的任何声明）
         ⇒ 不写 inherit 时「弹幕字体」这一项**一点效果都没有**、也不报错。
         量宽（`measureDanmakuWidths`）与这里必须同一个族，否则弹幕没滚到边就消失。 */
      font-family:inherit; }
.captions { position:absolute; bottom:var(--cap-bottom, 80px); left:var(--cap-left, 120px); width:var(--cap-w, 1680px); }
.cap { position:relative; text-align:center; padding:14px 0; font-weight:700; line-height:1.5;
       -webkit-text-stroke:12px #403c3b; paint-order:stroke fill;
       /* 同 `.dm`：`*{font-family:var(--font)}` 会盖掉父层 `.captions` 的字体栈
          （那个栈由 `fetchFonts().fallback_stack` 透传，是"预览与成片同源"的**唯一通道**）。 */
       font-family:inherit; }
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
