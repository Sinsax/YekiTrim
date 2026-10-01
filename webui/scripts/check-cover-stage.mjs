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
/** 帧层不能有底色：`transparent`/`none` 才算清掉全局的 `video{background:#000}` */
const frameVideoBgIsClear = (css) => /^(transparent|none)$/i.test(decl(rule(css, '.stage-video'), 'background') || '')
const frameVideoRadiusIsZero = (css) => num(decl(rule(css, '.stage-video'), 'border-radius')) === 0
const mediaWrapHasNoBg = (css) => {
  const v = decl(rule(css, '.stage-media'), 'background')
  return v === null || /^(transparent|none)$/i.test(v)
}
const frameIsContained = (css) => (decl(rule(css, '.stage-video'), 'object-fit') || '') === 'contain'
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
console.log('\n② 设计器帧层不许刷底色（竖屏素材黑边的根因）')
check('.stage-video background 是 transparent', frameVideoBgIsClear(designer), true,
  '漏了这条 → contain 的左右留白被元素自身背景填成纯黑，还盖住 .stage-bg 模糊层')
check('.stage-video border-radius 被清成 0', frameVideoRadiusIsZero(designer), true,
  '全局 video 的 --r-sm(10px) 也漏进来了：现在被 .stage 的 14px 盖住看不出，但别留这个口子')
check('.stage-media 容器没有底色', mediaWrapHasNoBg(designer), true)
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
check('[变异] .stage-video 不声明 background（= 修复前）→ 判据翻面',
  frameVideoBgIsClear(designer.replace(/;\r?\n\s*background:transparent; border-radius:0;/, ';')), false)
check('[变异] .stage-video 写成 background:#000 → 判据翻面',
  frameVideoBgIsClear(designer.replace('background:transparent; border-radius:0;', 'background:#000;')), false)
check('[变异] 去掉 border-radius:0 → 判据翻面',
  frameVideoRadiusIsZero(designer.replace('background:transparent; border-radius:0;', 'background:transparent;')), false)
check('[变异] 帧层 contain → cover（两份同时改）→ 判据翻面',
  frameIsContained(designer.replace('object-fit:contain; display:block;', 'object-fit:cover; display:block;')) === false
  && tplFrameIsContained(coverHtml.replace('object-fit:contain; display:block;', 'object-fit:cover; display:block;')) === false, true)
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
// 同一块面板还有第二种隐身：`.controls` 的高度被 JS 限到与预览等高，而文字层那一屏内容更高，
// flex 收缩按 flex-basis 比例来 —— 没有 `min-height` 下限时它会被压成 **0 高**（两者都实测过：
// 用本机 headless Chrome 真挂载组件量 `getBoundingClientRect()` + `getComputedStyle().display`）。
const usesClass = (src, name) => new RegExp('class="[^"]*\\b' + name + '\\b').test(src)
/** `.lyr-props` 的 min-height（px）——`num()` 吃不了带单位的声明，这里 parseFloat */
const paneMinHeight = (src) => {
  const n = parseFloat(String(decl(rule(src, '.lyr-props'), 'min-height') ?? ''))
  return Number.isFinite(n) ? n : null
}
console.log('\n⑥ 全局类名穿透：设计器不许复用 app.css 里 display:none 的类名')
check('app.css 仍有全局 `.pane { display:none }`（这条就是理由）',
  /display\s*:\s*none/.test(rule(appCss, '.pane') || ''), true)
check('设计器的属性面板没叫 .pane', usesClass(designer, 'pane'), false,
  '改回 .pane → 属性面板整块 display:none，页面不报错、只是面板没了')
check('属性面板 .lyr-props 带 min-height 下限', (paneMinHeight(designer) || 0) > 0, true,
  'flex 收缩（.controls 被 JS 限高）会把它压成 0 高')
check('[变异] 面板类名改回 .pane → 判据翻面',
  usesClass(designer.replace(/class="lyr-props/g, 'class="pane'), 'pane'), true)
check('[变异] 去掉 min-height → 判据翻面',
  paneMinHeight(designer.replace('min-height:120px', '')) === null, true)

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
