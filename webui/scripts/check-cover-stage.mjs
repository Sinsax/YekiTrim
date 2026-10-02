// 封面设计器舞台（CoverDesigner.vue 的 `.stage`）⇄ 出片模板 `templates/cover.html` 的
// **图层 / 比例 / 数字契约**检查。
//
// ⚠️ 最要紧的是一条**回归钉子**（用户报过"封面设计器素材左右黑边，出片却正常"）：
//    `webui/src/styles/app.css` 有一条**全局元素选择器**
//        video { max-width:100%; max-height:170px; background:#000; border-radius:var(--r-sm); margin-top:6px }
//    （给剪辑卡片那类视频缩略图用的）。类选择器只覆盖**自己声明过**的属性，
//    于是 `background:#000` 与 `border-radius:var(--r-sm)` **穿透**到设计器的 `.stage-video`：
//    `object-fit:contain` 让出来的左右留白区由**元素自身背景**填充 ⇒ 竖屏素材在设计器里
//    就是两条**纯黑边**；而它（`.stage-media` z1）压在 `.stage-bg`（模糊背景层 z0）之上，
//    把背景层整个盖住 —— 看起来"模糊背景没运行"。出片走 `<img>` + 独立模板（不带 app.css），
//    所以这个 bug **只在设计器里出现**，两边一比就是"预览与成片不一致"。
//
// 顺带钉住几条"两份实现必须同口径"的契约（出片 `cover.html` + `cover.rs` 是事实来源）：
//   · 帧层：两边都 `object-fit:contain`（素材保持原比例，**用户明确要求**，别改回 cover）；
//   · 背景层：两边都 `object-fit:cover` + `blur(28px×倍率)` + `scale(1+px/150)`；
//   · 画布：**恒 1920×1080 横版**（不跟 `[canvas] orientation`）；
//   · 背景图文件名：blur 读关键帧 `_cover_frame.jpg`、image 读 `_cover_bg.jpg`（别共用一个文件）；
//   · **全局类名穿透**：`app.css` 的 `.pane { display:none }` 不许被设计器复用（⑥，2026-09 图层化
//     改造时踩到 —— 与上面那条 `video { background:#000 }` 同一个机制）。
//
// 运行： node webui/scripts/check-cover-stage.mjs
//
// 零依赖、不 import 任何模块（`webui/package.json` 没有 `"type":"module"`；engine.js 那套
// data-URL import 也已因 `./canvas.js` 失效），一律**从源码文本里抠**。CSS 只做"选择器 → 声明块"
// 的文本解析，不引 CSS 解析器：这里要钉的是"某条声明在不在"，不是层叠后的计算值。

import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..', '..')

const read = async (...parts) => readFile(join(root, ...parts), 'utf8')
const appCss = await read('webui', 'src', 'styles', 'app.css')
// ⚠️ 通配规则在 app.css 与 tokens.css **各有一份**（历史遗留），两边都要查 —— 只查一处会漏。
const tokensCss = await read('webui', 'src', 'styles', 'tokens.css')
const designer = await read('webui', 'src', 'components', 'CoverDesigner.vue')
const coverHtml = await read('crates', 'vtrim-polish', 'templates', 'cover.html')
const coverRs = await read('crates', 'vtrim-polish', 'src', 'cover.rs')

// ── 断言脚手架 ────────────────────────────────────────────────────────────
let failed = 0

function check(label, ok, want = true, hint = '') {
  if (ok === want) {
    console.log(`  ok   ${label}`)
  } else {
    failed++
    console.log(`  FAIL ${label}`)
    console.log(`         实得 ${ok}，期望 ${want}${hint ? '：' + hint : ''}`)
  }
}

const num = (s) => (s === null || s === undefined ? null : Number(String(s).trim()))

/** 取 `selector` 的声明块文本。⚠️ `.stage-bg` 也会命中 `.stage-bg-img`，所以要求选择器后面
 *  **紧跟** `{`（`.stage-bg { … }` 里 `.stage-bg` 后是空格再 `{`；`.stage-bg-img {` 里是 `-`，不匹配）。 */
function rule(css, selector) {
  const re = new RegExp(selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '\\s*\\{')
  const m = re.exec(css)
  if (!m) return null
  const open = m.index + m[0].length - 1
  const close = css.indexOf('}', open)
  return close < 0 ? null : css.slice(open + 1, close)
}

/** 声明块的某条属性值（`null` = 没写这条声明 ⇒ 全局规则/继承的会生效） */
function decl(block, prop) {
  if (!block) return null
  const m = new RegExp('(?:^|;)\\s*' + prop + '\\s*:\\s*([^;]+)', 'i').exec(block)
  return m ? m[1].trim() : null
}

// ── 判据（每条都写成函数：变异自证段要对同一批判据复跑） ──────────────────────
const appVideoBgIsBlack = (css) => /^#000(000)?$/i.test(decl(rule(css, 'video'), 'background') || '')
/** 帧层**显示的那张图**（`_cover_frame.jpg`，与出片模板 `#frame img` 同源）不能有底色。
 *  2026-10 起帧层从 `<video>` 换成 `<img>` —— `img` 不是全局 `video{}` 规则的目标，所以这里
 *  钉的是「帧层仍然 contain 且铺满」；取帧的 video 元素在舞台外（.scrub-row 监视器，见下）。 */
const frameImgIsContained = (css) => (decl(rule(css, '.stage-frame-img'), 'object-fit') || '') === 'contain'
const frameImgFillsStage = (css) => {
  const b = rule(css, '.stage-frame-img')
  return /width\s*:\s*100%/.test(b || '') && /height\s*:\s*100%/.test(b || '')
}
/** 取帧播放器（2026-10 起 = 舞台**外面** .scrub-row 的可见小监视器；用户报「没法播视频选关键帧」，
 *  隐身播放器让拖进度条成了盲选）：① `<video>` 必须还在（删了取帧/时长就全失灵）；
 *  ② 它必须在 .scrub-row 里、**舞台（.stage）内不许再出现任何 <video>** —— 舞台内的帧源
 *  只有 `_cover_frame.jpg` 一份（出片只有一个 `#frame img`，两帧图源就是那次不一致的成因）；
 *  ③ 监视器是声明了尺寸的真盒子（不是隐身的 1px），显式清掉全局 video 规则的
 *  `max-height:170px`/`margin-top`「卡片缩略图」口径。 */
/** 取帧播放器（2026-10 **第三轮：口径修正**）。出图链路是「ffmpeg 按 frame_pos×时长 截帧 →
 *  写 `_cover_frame.jpg` → 塞进 `cover.html` → 出图程序渲染」——**前端从来不是帧源**，
 *  它只负责让用户**选时间点**。所以舞台里**常驻**一个播放器是对的，**不需要**也没有理由
 *  把它按"当前选中哪个层"门控（那正是两个真 bug 的来源：选中别的层就没法拖进度条；
 *  刚选好的帧一切层就跳回静态图）。
 *  ① 舞台帧层必须有 `.stage-video-live` 的 <video>（**不得**挂 frameSelected 之类门控）；
 *  ② `_cover_frame.jpg` 的静态 <img> 保留为**取不到视频时的兜底**（与出片同源，别删）；
 *  ③ 播放器清掉全局 `video{}` 的黑底/圆角/170px 口径，盒子与静态帧一致。 */
const hasStageLiveVideo = (src) => {
  const clean = src.replace(/<!--[\s\S]*?-->/g, '')
  return /<video[^>]*class="stage-video-live"/.test(clean)
}
/** ⚠️ 播放器**不许**按层选中态门控（口径修正的钉子：再挂回 v-if/frameSelected 就是回退）。 */
const liveVideoIsNotGated = (src) => {
  const clean = src.replace(/<!--[\s\S]*?-->/g, '')
  const m = /<video([^>]*)class="stage-video-live"/.exec(clean)
  return !!m && !/v-if|v-show/.test(m[1])
}
const hasStaticFrameFallback = (src) => /class="stage-frame-img"/.test(src)
const stageVideoClearsGlobal = (css) => {
  const b = rule(css, '.stage-video-live')
  const v = /^(transparent|none)$/i.test(decl(b, 'background') || '') && num(decl(b, 'border-radius')) === 0
  return v && /max-height\s*:\s*none/.test(b || '') && /margin\s*:\s*0/.test(b || '')
    && /object-fit\s*:\s*contain/.test(b || '') && !/opacity\s*:\s*0/.test(b || '')
}
/** ⚠️（2026-10 第三轮）**"帧层不许放 video"是错的契约，已删除** —— 出图链路是 ffmpeg 截帧写
 *  `_cover_frame.jpg` → 注入 `cover.html` → 出图程序渲染，**前端从来不是帧源**，它只是让用户**选
 *  时间点**。当年那次"设计器第 0 秒 vs 成片 frame_pos"的真根因是**没把播放头对到 frame_pos**，
 *  守门人是下面的「打开设计器会按 frame_pos × 时长 seek」那条，而不是"禁止播放"。
 *  代码里若再出现 `v-if="frameSelected"` 之类的层选中门控，`liveVideoIsNotGated` 会当场报红。 */
/** 「生成封面」必须**无条件**按当前进度刷新关键帧：0 秒是合法取值，不能拿 `time > 0` 当门控
 *  （那会让"没拖过进度条"的用户选的那一帧永远进不了产物）。 */
const coverFrameIsUnconditional = (src) =>
  !/> 0\)\s*\{\s*\n\s*await coverFrame\(props\.pid, props\.dir, time\.value\)/.test(src)
  && /const frameAt = Number\(time\.value\) \|\| 0[\s\S]{0,200}?await coverFrame\(props\.pid, props\.dir, frameAt\)/.test(src)
const mediaWrapHasNoBg = (css) => {
  const v = decl(rule(css, '.stage-media'), 'background')
  return v === null || /^(transparent|none)$/i.test(v)
}
const frameIsContained = (css) => (decl(rule(css, '.stage-frame-img'), 'object-fit') || '') === 'contain'
const tplFrameIsContained = (html) => {
  const b = rule(html, '#frame img')
  return (decl(b, 'object-fit') || '') === 'contain' && /width:100%/.test(b || '') && /height:100%/.test(b || '')
}
const tplBgImgIsCovered = (html) => {
  const b = rule(html, '#bg.bg-blur img, #bg.bg-image img')
  return /display:block/.test(b || '') && (decl(b, 'object-fit') || '') === 'cover'
}
const tplBgImgHiddenByDefault = (html) => /display:none/.test(rule(html, '#bg img') || '')
const tplBlurHasFilterAndScale = (html) => {
  const b = rule(html, '#bg.bg-blur img')
  return /filter\s*:\s*blur\(var\(--bg-blur\)\)/.test(b || '') && /transform\s*:\s*scale\(var\(--bg-blur-scale\)\)/.test(b || '')
}
/** 图层序：**默认栈**里背景层在帧层之下。
 *  2026-09 起 z-index 由 `[cover] layers` 决定（行内样式），所以判据从"读 CSS 的 z-index"
 *  改成两条：① 设计器的默认序里 bg 在 frame 之前；② 内容层的 z-index 不再写死在 CSS 里。 */
const defaultStackHasBgUnderFrame = (src) => /['"]bg['"][\s\S]{0,60}?['"]frame['"]/.test(src)
const contentZIndexOf = (css, sel) => num(decl(rule(css, sel), 'z-index'))
const contentZIndexesGone = (css) =>
  ['.stage-bg', '.stage-media', '.vignette-layer', '.gradient-layer', '.title-wrap'].every(
    (sel) => contentZIndexOf(css, sel) === null
  )

/** Rust 常量（类型不写死：`f64`/`u32` 都有） */
const rsNum = (name, src = coverRs) => {
  const m = new RegExp(name + '\\s*:\\s*[a-z0-9]+\\s*=\\s*([0-9.]+)').exec(src)
  return m ? Number(m[1]) : null
}
const rsStr = (name) => {
  const m = new RegExp(name + '\\s*:\\s*&str\\s*=\\s*"([^"]+)"').exec(coverRs)
  return m ? m[1] : null
}
const designerConst = (name) => {
  const m = new RegExp('const ' + name + '\\s*=\\s*([0-9.]+)').exec(designer)
  return m ? Number(m[1]) : null
}

// ── ① 全局 video 规则：`.stage-video` 那两行覆盖的**存在理由** ────────────────
console.log('① 全局 video 规则（说明 .stage-video 为什么必须显式清底色）')
check('app.css 的 `video {}` 仍在', rule(appCss, 'video') !== null, true)
check('app.css 的 `video {}` 带 background:#000', appVideoBgIsBlack(appCss), true,
  '全局黑底规则没了 → 可以删掉 .stage-video 的 background:transparent，并同步本脚本与 AGENTS.md')

// ── ② 设计器帧层：不许有黑底 / 圆角（回归钉子） ─────────────────────────────
console.log('\n② 设计器帧层与出片同构（舞台常驻取帧播放器 + `_cover_frame.jpg` 兜底图）')
check('帧层 img 保持原比例（object-fit:contain，与 #frame img 同规则）', frameImgIsContained(designer), true,
  '竖屏素材会被裁掉 —— 那是用户明确否掉的（要求完整显示、左右留白露出背景层）')
check('帧层 img 铺满舞台（width/height 100%）', frameImgFillsStage(designer), true,
  '不铺满 → 帧层缩在某一角，与出片 #frame img 的 100%/100% 不同口径')
check('舞台帧层有常驻取帧播放器（stage-video-live）', hasStageLiveVideo(designer), true,
  '没有它 = 用户只能看静态图 + 拖进度条盲选（2026-10 用户口径：图层里的视频预览就该能播）')
check('播放器**不**按选中层门控（全局取景 UI，切层不影响找帧）', liveVideoIsNotGated(designer), true,
  '挂回 v-if/frameSelected = 回退到两个真 bug：选中别的层没法拖进度条、选好的帧一切层就跳回去')
check('未选中时回落静态图（_cover_frame.jpg，与出片同源）', hasStaticFrameFallback(designer), true,
  '帧层契约：静态图必须还是 _cover_frame.jpg（11.09/255 那次同帧修复的根子）')
check('舞台播放器清掉全局 video 口径（黑底/圆角/170px 缩略图）', stageVideoClearsGlobal(designer), true,
  '全局 video{} 穿透：黑边盖住下层模糊背景（用户报过「设计器黑边、出片正常」）')
check('.stage-media 容器没有底色', mediaWrapHasNoBg(designer), true)
// ⚠️ **根因钉子（2026-10）**：帧层必须是 `_cover_frame.jpg` 的图 —— 与出片模板 `#frame img` 同源。
// 以前这里放的是实时 `<video>`（打开时恒为第 0 秒），出片却读 `_cover_frame.jpg`（按 frame_pos 截），
// ⇒「设计器里是这一帧、成片里是另一帧」，两边都不报错，只能比对产物才发现。
check('帧层静态兜底图与出片同源（_cover_frame.jpg 的 <img>）', hasStaticFrameFallback(designer), true,
  '取不到视频就黑屏 —— 这张图就是兜底；图源必须仍是 _cover_frame.jpg（与出片同一个文件）')
check('帧层的 img 指向 `_cover_frame.jpg`（与出片同一个文件）',
  /frameImageSrc\s*=\s*computed[\s\S]{0,400}?'_cover_frame\.jpg'/.test(designer), true,
  '换成别的图源 → 两边又不是同一张图了')
check('打开设计器会按 frame_pos × 时长 seek（不再是恒 0 秒）',
  /function seekToFramePos\(\)[\s\S]{0,300}?framePos\.value \* duration\.value/.test(designer)
  && /framePos\.value = Math\.min\(1, Math\.max\(0, Number\(cfg\?\.cover\?\.frame_pos/.test(designer), true,
  '反推不出 frame_pos → 打开时视频停在 0 秒，与出片截的那一帧不同')
check('「生成封面」不再用 `time > 0` 当门控（0 秒是合法取值）', coverFrameIsUnconditional(designer), true,
  '留着这个门控 → 用户没拖过进度条时，他选的那一帧永远进不了产物')
check('图层序：默认栈里 bg 在 frame 之前（z-index 由栈给）', defaultStackHasBgUnderFrame(designer), true,
  '把默认序改成 frame 在前 → 帧层会被背景层盖住')
check('内容层的 z-index 不再写死在 CSS 里（只允许行内/栈给）', contentZIndexesGone(designer), true,
  'CSS 里写死图层序 = 列表里这么排、成片里那么画（不报错的那类 bug）')

// ── ③ 出片模板：同一套图层契约 ──────────────────────────────────────────────
console.log('\n③ 出片模板 cover.html 同口径')
check('#frame img 保持原比例（object-fit:contain）', tplFrameIsContained(coverHtml), true,
  'contain 是用户要求（素材原比例完整显示），别改回 cover')
check('#bg.bg-blur/.bg-image 的 img 铺满（display:block + cover）', tplBgImgIsCovered(coverHtml), true)
check('#bg img 默认 display:none（color 模式不显示图）', tplBgImgHiddenByDefault(coverHtml), true)
check('blur 层带 filter:blur(--bg-blur) 与 transform:scale(--bg-blur-scale)', tplBlurHasFilterAndScale(coverHtml), true)
check('#page 用注入的画布尺寸（__COVER_W__px / __COVER_H__px）',
  /width:__COVER_W__px/.test(rule(coverHtml, '#page') || '') && /height:__COVER_H__px/.test(rule(coverHtml, '#page') || ''), true)
check('#page 没写死 1920px（必须由 cover.rs 注入）', /1920px/.test(rule(coverHtml, '#page') || ''), false)

// ── ④ 数字同源（出片是事实来源，预览是镜像） ────────────────────────────────
console.log('\n④ 数字同源：cover.rs ⇄ CoverDesigner.vue ⇄ cover.html')
check('模糊基准半径 = 28（Rust）', rsNum('COVER_BLUR_BASE_PX'), 28)
check('模糊基准半径 = 28（设计器）', designerConst('COVER_BLUR_BASE_PX'), 28)
check('模糊放大公式两边都是 1 + px/150',
  /let img_scale = 1\.0 \+ blur_px \/ 150\.0;/.test(coverRs) && /imgScale: Math\.round\(\(1 \+ px \/ 150\)/.test(designer), true)
check('模糊倍率夹取两边都是 0.2~4',
  rsNum('COVER_BLUR_SCALE_MIN') === 0.2 && rsNum('COVER_BLUR_SCALE_MAX') === 4
  && /Math\.min\(4, Math\.max\(0\.2, Number\(scale\) \|\| 1\)\)/.test(designer), true)
check('画布恒 1920×1080（Rust COVER_CAPTURE_*）', rsNum('COVER_CAPTURE_W') === 1920 && rsNum('COVER_CAPTURE_H') === 1080, true)
check('画布恒 1920×1080（设计器 COVER_*）', designerConst('COVER_W') === 1920 && designerConst('COVER_H') === 1080, true)
check('关键帧文件名两边一致', rsStr('COVER_FRAME_IMG') === '_cover_frame.jpg' && designer.includes("'_cover_frame.jpg'"), true)
check('自定义背景文件名两边一致', rsStr('COVER_BG_IMG') === '_cover_bg.jpg' && designer.includes("'_cover_bg.jpg'"), true)
check('设计器没把两个文件弄混（blur→关键帧、image→背景图）',
  /bgMode\.value === 'image'\) return clipMediaUrl\(props\.pid, props\.dir, '_cover_bg\.jpg'\)/.test(designer)
  && /return clipMediaUrl\(props\.pid, props\.dir, '_cover_frame\.jpg'\)/.test(designer), true)

// ── ⑤ 自证有效：把源码改回修复前，上面那批判据必须翻面 ────────────────────────
console.log('\n⑤ 自证有效：变异后上面的断言必须失败（否则本脚本是空转的）')
check('[变异] 舞台播放器删掉（退回纯静态图）→ 判据翻面',
  hasStageLiveVideo(designer.replace('class="stage-video-live"', 'class="stage-video-x"')) === false, true)
check('[变异] 播放器挂回选中层门控（= 两个真 bug 回退）→ 判据翻面',
  liveVideoIsNotGated(designer.replace('<video ref="videoRef" class="stage-video-live"', '<video v-if="frameSelected" ref="videoRef" class="stage-video-live"')) === false, true)
check('[变异] 静态兜底图删掉（取不到视频就黑屏）→ 判据翻面',
  hasStaticFrameFallback(designer.replace('class="stage-frame-img"', 'class="stage-frame-x"')) === false, true)
check('[变异] 舞台播放器改回隐身（opacity:0）→ 判据翻面',
  stageVideoClearsGlobal(designer.replace('object-fit:contain;\n  background:transparent', 'object-fit:contain; opacity:0;\n  background:transparent')) === false, true)
check('[变异] 帧层 contain → cover（两份同时改）→ 判据翻面',
  frameIsContained(designer.replace('.stage-frame-img { position:absolute; inset:0; width:100%; height:100%; object-fit:contain; display:block; }', '.stage-frame-img { position:absolute; inset:0; width:100%; height:100%; object-fit:cover; display:block; }')) === false
  && tplFrameIsContained(coverHtml.replace('object-fit:contain; display:block;', 'object-fit:cover; display:block;')) === false, true)
check('[变异] 打开时不 seek 到 frame_pos（= 真正的 2026-10 根因）→ 判据翻面',
  !/function seekToFramePos\(\)/.test(designer.replace('function seekToFramePos()', 'function seekToFramePosX()')), true,
  '去掉 seekToFramePos = 打开恒第 0 秒、成片却是 frame_pos 那帧（当年只能比对产物才发现）')
check('[变异] 背景层 cover → contain → 判据翻面',
  tplBgImgIsCovered(coverHtml.replace('display:block; width:100%; height:100%; object-fit:cover;', 'display:block; width:100%; height:100%; object-fit:contain;')), false)
// 只改一边的数字同源：`rsNum` 吃的是被替换过的源码文本，必须立刻读到新值（否则这项是空转的）
const rsMismatch = coverRs.replace(/(COVER_BLUR_BASE_PX\s*:\s*[a-z0-9]+\s*=\s*)[0-9.]+/, '$140.0')
check('[变异] 只把 Rust 的基准改成 40 → 与设计器的 28 不再相等（本脚本能抓到）',
  rsNum('COVER_BLUR_BASE_PX', rsMismatch) === 40 && designerConst('COVER_BLUR_BASE_PX') === 28, true)

// ── ⑥ 全局类名穿透（`.pane`）与属性面板高度 ──────────────────────────────────
// `styles/app.css` 有一条**全局** `.pane { display:none }`（设置页那几个 tab 面板靠 `.on` 才显示）。
// 封面设计器 2026-09 图层化时新加的"属性面板"如果也叫 `.pane`，整块面板会**直接隐身** ——
// 与 `.stage-video` 那两条黑边同一个机制（全局规则穿透组件里没显式覆盖的属性），而且**不报错**。
// 同一块面板还有第二种隐身：面板高度被 JS 限到与预览等高（`syncControlsBound`），两栏**内部滚动**。
// ⚠️ 2026-10 起面板内部改成「左设置 / 右列表」左右排布（⑥c），滚动前提从"给 min-height 下限"
// 变成了 **`min-height:0`** —— flex/grid 子项默认 `min-height:auto`（= 内容高度），不给 0
// 就**永远出不了滚动条**，而是把两栏连同外层一起撑高、溢出被 `.controls{overflow:hidden}` 裁掉
// （表现：最底下那排控件点不到，且**不报错**）。两种都实测过：用本机 headless Chrome 真挂载，
// 量 `getBoundingClientRect()` + `getComputedStyle().display`。
const usesClass = (src, name) => new RegExp('class="[^"]*\\b' + name + '\\b').test(src)
/** `.lyr-props` 的 min-height（px）——`num()` 吃不了带单位的声明，这里 parseFloat */
const paneMinHeight = (src) => {
  const n = parseFloat(String(decl(rule(src, '.lyr-props'), 'min-height') ?? ''))
  return Number.isFinite(n) ? n : null
}
/** 两栏都必须能滚：`min-height:0` 是 flex/grid 子项出滚动条的**前提**（见上面那段注释） */
const paneCanScroll = (src) => paneMinHeight(src) === 0
console.log('\n⑥ 全局类名穿透：设计器不许复用 app.css 里 display:none 的类名')
check('app.css 仍有全局 `.pane { display:none }`（这条就是理由）',
  /display\s*:\s*none/.test(rule(appCss, '.pane') || ''), true)
check('设计器的属性面板没叫 .pane', usesClass(designer, 'pane'), false,
  '改回 .pane → 属性面板整块 display:none，页面不报错、只是面板没了')
check('属性面板 .lyr-props 的 min-height 是 0（能真正滚起来）', paneCanScroll(designer), true,
  'min-height 不是 0 → 子项被内容顶开、滚动条永远不出现，溢出被 .controls 裁掉')
check('[变异] 面板类名改回 .pane → 判据翻面',
  usesClass(designer.replace(/class="lyr-props/g, 'class="pane'), 'pane'), true)
// ⚠️ 变异必须只打 `.lyr-props` 那条：`min-height:0` 在文件里出现多处（`.lyr-body`/`.lyr-col` 也有），
// 裸的 `replace` 只换第一处，变异就白做了。这里先切出 `.lyr-props` 的规则体再改。
check('[变异] .lyr-props 的 min-height 改成 120px → 判据翻面',
  paneCanScroll(designer.replace(/(\.lyr-props \{[^}]*?)min-height:0/, '$1min-height:120px')) === false, true)

// ⚠️ 高亮词是**经 `v-html` 注入**的（`highlightTitleLine()` 拼 <span class="hl">），而 Vue 的 scoped 编译
// **不给 v-html 内容打 data-v 属性**。所以 `:deep()` 不能省 —— 写成 `.title-text .hl` 会编译成
// `.title-text .hl[data-v-xxx]`、永远匹配不上，症状是「字体照旧会变（fontFamily 在父级、span 继承）、
// 只有高亮色静默失效」（用户 2026-10 报的原文）。出片侧不受影响：`cover.rs` 把那串 HTML 注入到
// `cover.html` 的**全局** <style> 里，没有 scope 这回事。
console.log('\n⑥b 高亮词：v-html 注入的 .hl 必须能命中（scoped → 非 :deep() 就静默失效）')
const hlRuleIsDeep = (src) => {
  // 只认 `.title-text :deep(.hl)`；写成 `.title-text .hl` 就是那个 bug 的回归
  return /\.title-text\s*:deep\(\.hl\)/.test(src)
}
check('设计器里有 `.title-text :deep(.hl)` 高亮规则', hlRuleIsDeep(designer), true,
  '少了 :deep() → v-html 的 span 没有 data-v 属性、规则匹配不上 → 高亮色不生效（字体却照常继承，极易误判）')
check('该规则确实给高亮色（color:var(--hlc…)，与出片模板 .hl 同口径）',
  /:deep\(\.hl\)\s*\{[^}]*color\s*:\s*var\(--hlc/.test(designer), true,
  '高亮色必须走同一个 --hlc 变量（titleStyle 行内注入），否则预览与出片两个颜色')
check('titleStyle 仍然注入 --hlc（变量的源头）', /'--hlc'\s*:/.test(designer), true,
  '变量没了 → :deep(.hl) 退到 fallback #ffcc00，用户改的高亮色不生效')
check('[变异] :deep() 去掉（退回 scoped 直配 .hl）→ 判据翻面',
  hlRuleIsDeep(designer.replace('.title-text :deep(.hl)', '.title-text .hl')) === false, true)
check('[变异] 高亮色不接 --hlc（写死颜色）→ 判据翻面',
  /:deep\(\.hl\)\s*\{[^}]*color\s*:\s*var\(--hlc/.test(designer.replace('color:var(--hlc, #ffcc00)', 'color:#ffcc00')) === false, true)
// 高亮字号（2026-10 用户要求「给高亮区域一个字号设置」）：**两端必须读同一个变量** ——
// 设计器把 `--hl-fs` 写进行内 style、出片模板从 `__HL_FS__` 填进 `:root`（静态渲染器不跑 JS，
// 变量只能从那儿来）。任一端写死 px/em，预览与成片就会各按各的字号排。
check('设计器 .hl 规则的字号走 --hl-fs 变量',
  /:deep\(\.hl\)\s*\{[^}]*font-size\s*:\s*var\(--hl-fs/.test(designer), true,
  '写死字号 → 用户调「高亮字号」时预览不动（或出片不动）')
check('titleStyle 注入 --hl-fs（设计器那侧的源头）', /'--hl-fs'\s*:/.test(designer), true,
  '变量没了 → :deep(.hl) 退到 fallback 1em，高亮字号设置不生效')
check('出片模板的 .hl 读同一个 --hl-fs（与设计器同口径）',
  /\.hl\s*\{[^}]*font-size\s*:\s*var\(--hl-fs/.test(coverHtml), true,
  '模板 .hl 不读变量 → 成片恒为 1em，"预览调了、成片没变"')
check('出片模板 :root 填了 --hl-fs 占位符（静态渲染路径的变量源）',
/--hl-fs\s*:\s*__HL_FS__/.test(coverHtml), true,
  ':root 少了它 → 静态渲染器把整条 var(--hl-fs) 声明丢掉（Takumi 不跑 JS，只能从这儿取值）')
check('cover.rs 替换了 __HL_FS__', /__HL_FS__/.test(coverRs), true,
  '占位符没被替换 → 成片里出现字面量 __HL_FS__、字号整条失效')
check('[变异] 设计器字号写死（不接 --hl-fs）→ 判据翻面',
  /:deep\(\.hl\)\s*\{[^}]*font-size\s*:\s*var\(--hl-fs/.test(designer.replace('font-size:var(--hl-fs, 1em)', 'font-size:1em')) === false, true)
check('[变异] 模板 .hl 去掉 font-size（出片不认字号）→ 判据翻面',
  /\.hl\s*\{[^}]*font-size\s*:\s*var\(--hl-fs/.test(coverHtml.replace('font-size:var(--hl-fs, 1em); ', '')) === false, true)
// ⚠️ **高亮词的字体必须与标题同源**（用户 2026-10 报「高亮词和标题字体不一样，后端没问题、前端有问题」）。
// 根因不是 `.hl` 自己设了字体，而是`styles/app.css` + `styles/tokens.css` 里那条**通配规则**
// `* { font-family:var(--font) }` **直接命中**了 v-html 注入的 `<span class="hl">` —— 直接命中的声明
// 压过**继承**来的值，于是库字体在 span 上被换回全站 `--font`：标题是库字体、高亮词是系统字体。
// 出片侧没这问题：`cover.html` 的 `*` 只设 margin/padding/box-sizing，**不设 font-family`**。
// 修法 = 在高亮规则里显式写 `font-family:inherit`（用本机 headless 实测过 A/B：
// 没有它 computed font-family 退成 `"Noto Sans SC", …`，加上它就是库字体）。
const appCssHasUniversalFont = () =>
  /\*\s*\{[^}]*font-family\s*:\s*var\(--font\)/.test(appCss) ||
  /\*\s*\{[^}]*font-family\s*:\s*var\(--font\)/.test(tokensCss)
const hlInheritsFont = (src) =>
  /:deep\(\.hl\)\s*\{[^}]*font-family\s*:\s*inherit/.test(src)
check('通配规则 *{font-family:var(--font)} 仍在（这就是必须写 inherit 的理由）',
  appCssHasUniversalFont(), true)
check('设计器 .hl 规则显式 font-family:inherit（压过通配符，与标题同字）',
  hlInheritsFont(designer), true,
  '少了它 → 通配符直接命中 v-html 的 span，库字体被换回全站 --font，高亮词变成另一种字体')
check('[变异] .hl 去掉 font-family:inherit → 判据翻面',
  hlInheritsFont(designer.replace('font-family:inherit; ', '')) === false, true)


// ── ⑥c 右栏内部：**左设置 / 右列表** 左右排布 + 右栏 40% ─────────────────
// 用户 2026-10 要求：图层列表与图层设置从"上下结构"改成左右。当时顺手把右栏减半到
// minmax(220px,20%)（每栏 ~107px）；**2026-10-02 用户改口**：两块面板各宽一倍 ——
// 每栏 107px 塞不下「标签+滑块+数字框」（实测 mini-field 只剩 39px） ⇒ 右栏回 minmax(440px,40%)。
// ⚠️ 两栏用 **grid** 而不是两个 flex 子项：flex-basis 会被"文字那一屏 ~800px 内容高度"顶开，
// 两栏宽度就不按比例了（这正是旧注释里记的那次踩坑）。
// 几何实证（本机 headless，外层 1146px）：229px 时每栏 107px、mini-field 39px；
// 458px 时每栏 ~222px、mini-field ~135px，「对齐/行距」两列一行放得下。
const layerBodyIsSideBySide = (src) => {
  const r = rule(src, '.lyr-body') || ''
  return /display\s*:\s*grid/.test(r) && /grid-template-columns\s*:\s*minmax\(0,1fr\)\s+minmax\(0,1fr\)/.test(r)
}
/** `.lyr-body` 里两栏的**顺序**：设置在前 = 左、列表在后 = 右（DOM 顺序即视觉顺序） */
const propsBeforeList = (src) => {
  const iProps = src.indexOf('class="lyr-props"'), iList = src.indexOf('class="lyr-list"')
  return iProps > 0 && iList > 0 && iProps < iList
}
/** 右栏宽度：`minmax(440px, 40%)`（2026-10-02 用户改口：每栏 107px 塞不下表单行，各宽一倍回 40%） */
const gridRightColumn = (src) => (rule(src, '.designer-grid') || '').match(/minmax\([^)]*\)\s*$/)
check('右栏内部两栏并排（.lyr-body 是两列 grid）', layerBodyIsSideBySide(designer), true,
  '退回上下结构 → 用户要的左右排布没了')
check('设置栏在左、列表在右（DOM 顺序）', propsBeforeList(designer), true,
  '顺序反了就是"左列表 / 右设置"，与用户要求相反')
check('右栏宽度 40%（minmax(440px, 40%)）',
  /minmax\(440px,\s*40%\)/.test(rule(designer, '.designer-grid') || ''), true,
  '改回 220px/20% → 每栏 ~107px，「标签+滑块+数字框」塞不下（用户 2026-10-02 报排版混乱）')
check('两栏各有 min-width:0（长设置项不会把 grid 撑破）',
  /min-width\s*:\s*0/.test(rule(designer, '.lyr-props') || '') || /min-width\s*:\s*0/.test(rule(designer, '.lyr-col') || ''), true)
// ⚠️ 变异必须只打 `.lyr-body` 那条：那个 minmax 串在源码里出现**两次**（`.txt-top` 也有一份），
// 裸的 `replace` 只换第一处（= `.txt-top`），变异就白做了 —— 这里先切出 `.lyr-body` 那一段再改。
check('[变异] .lyr-body 退回单列 → 判据翻面',
  layerBodyIsSideBySide(designer.replace(/\.lyr-body \{[^}]*\}/, '.lyr-body { display:flex; }')) === false, true)
check('[变异] 右栏宽度改回 220px/20% → 判据翻面',
  // ⚠️ 只打**规则行**：注释里也写着 220px/20%（历史记录），全文件 replace 会连注释一起换，
  // 变异后主断言照样通过 → 判据不翻面（本仓变异纪律：from 串必须恰好命中目标）。
  (() => { const m = rule(designer.replace(/\.designer-grid \{[^}]*\}/, '.designer-grid { display:grid; grid-template-columns:minmax(0,1fr) minmax(220px, 20%); gap:16px; align-items:start; }'), '.designer-grid') || ''; return !/minmax\(440px,\s*40%\)/.test(m); })(), true)

// ── ⑦ 图层栈在两端都不许被"两处实现"拆开 ─────────────────────────────────────
// 图层栈的 z-index 注入必须**只有一条替换链**（`build_cover_page`）：以前预览与出片各写一份，
// 漏改一边就是"预览有层序、出片没有"；`#fx-extra` 一旦有 z-index 就会把特效锁进同一个 stacking
// context（拖动排序看着生效、出片不动）。
console.log('\n⑦ 图层栈：一条替换链 + #fx-extra 不许有 z-index')
check('`COVER_HTML` 只在 `build_cover_page` 里替换一次（没有第二份构建器）',
  (coverRs.match(/COVER_HTML\s*\n?\s*\.replace/g) || []).length === 1, true,
  '出现两次 → 又分叉了；预览与出片必须同走 build_cover_page')
check('cover.rs 调了 `cover_layer_css` 与 `cover_fx_html`',
  /cover_layer_css\(&plan, legacy\)/.test(coverRs) && /cover_fx_html\(cc, &plan, legacy\)/.test(coverRs), true)
/** `#fx-extra` 既不能在样式表里、也不能在内联里带 z-index */
const fxExtraHasNoZIndex = (html) =>
  !/z-index/.test(rule(html, '#fx-extra') || '') && !/id="fx-extra"[^>]*z-index/.test(html)
check('模板 `#fx-extra` 没有 z-index（否则特效被锁死在一层）', fxExtraHasNoZIndex(coverHtml), true,
  '内联 z-index 会让所有特效进同一个 stacking context，拖动排序就白做了')
check('模板有 `__LAYER_CSS__` 占位（图层栈注入点）', /<style>__LAYER_CSS__<\/style>/.test(coverHtml), true)
check('[变异] 给 #fx-extra 加回内联 z-index → 判据翻面',
  fxExtraHasNoZIndex(coverHtml.replace('id="fx-extra" style="position:absolute;inset:0;pointer-events:none;"', 'id="fx-extra" style="position:absolute;inset:0;pointer-events:none;z-index:1;"')), false)

console.log(failed ? `\n${failed} 项失败` : '\n全部通过')
process.exit(failed ? 1 : 0)
