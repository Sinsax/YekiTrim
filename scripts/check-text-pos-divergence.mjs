#!/usr/bin/env node
// 门禁：**文字纵向档位（`pos`）必须真的接到成片**，且那条"只在预览生效"的旧警示不许回来。
//
// # 这个落差是什么（以及它曾经有多难发现）
//
// `elements.text` 有用户可见的下拉：纵向位置 = 上 / 中 / 下（`pos`）。
// `process` **真的**按它选落点，**预览**也真的按那对 `x`/`y` 摆 —— 三档在预览里看得出差别。
// **但成片曾经一律贴底**：转译器把 ASS 对齐码硬编码成 `2`（底部居中）。
//
// 根因不是"忘了接线"，是**底座缺字段**：`text_layout::layout` 拿 `bottom_margin`
// **同时**当"左右安全宽"的输入：
//
//     let margin = style.bottom_margin.clamp(0.0, 0.5);
//     let max_width_px = sequence_width * (1.0 - 2.0 * margin);   // ← 左右安全宽
//     let block_top    = 1.0 - margin - keep * line_height_norm;  // ← 纵向位置
//
// 安全宽 0.875 要它 = 0.0625；把 `top` 抬上去要它 ≈ 0.6 —— 抬上去 `max_width` 变负数
// ⇒ **一个字都排不出来**（真实事故：误填 0.5，"每一条文字元素都静默消失"，
// 报告里**一行差异都没有**，因为布局只是返回了空）。
//
// # 2026-10-12：修好了（底座 v7）
//
// 底座加了与安全宽解耦的 `SubtitleStyle::anchor_bottom`（文字块**底边**的归一化 y），
// `safe_width_ratio` 同时被 `layout()` 真正读起来。转译器把 `pos` 映射过去。
// 真机实测三档在成片里的文字纵向重心：**0.394 / 0.663 / 0.903**（严格单调，跨度 549px）。
//
// # 这条判据盯什么（★ 已经从"盯落差被写清"翻成"盯接线别断"）
//
//   ① 转译器必须把 `pos` 真的写进 `anchor_bottom`，且**按 (样式 × 档位) 两维分轨**
//      —— `anchor_bottom` 是轨级常量、`pos` 是逐事件的，只按样式分轨会**互相覆盖**
//      （实测真实工程里 `hook` 同时有 `top` 与 `center`）；
//   ② 界面上的**旧警示不许回来**（回来就是谎报：用户会以为选了「上」没用）；
//   ③ 规格文档必须记着这条已接通（别让后来的人照旧结论重新提一遍需求）；
//   ④ **判据的前提**：ASS 对齐码仍是 `2` —— 那一列至今不表达纵向，
//      纵向全靠 `anchor_bottom`。它若变了，本判据要重审。
//
//     node scripts/check-text-pos-divergence.mjs
//     node scripts/check-text-pos-divergence.mjs --self-test
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const read = (rel) => readFileSync(join(repo, rel), 'utf8');

let checks = 0;
const failures = [];
const check = (name, ok) => {
  checks += 1;
  if (!ok) failures.push(name);
};

const TL = read('webui/src/components/TimelineEditor.vue');
const SPEC = read('docs/plan/elements-spec.md');
const REQ = read('docs/plan/dhampir-elements-request.md');
const TRANS = read('tools/polish-to-dhampir.mjs');

// ---------------------------------------------------------------------------
// ① 转译器：`pos` → `anchor_bottom`，且分轨是两维的
// ---------------------------------------------------------------------------
check(
  '转译器把纵向档映射成 anchor_bottom（写出了锚）',
  /anchor_bottom:\s*anchorBottomFor\(/.test(TRANS),
);
check(
  '映射函数存在（档位 → 块底归一化 y）',
  /function anchorBottomFor\(/.test(TRANS),
);
// ⚠️ 2026-10-13（39 档）：分轨键从**两维**扩到**三维** `样式|纵向档|动效档`。
//    第三维与第二维是同一个理由的**第二次出现** —— `scale_in_*` / `shake_*`
//    那些动效字段**也是轨级的**（住在底座 `SubtitleStyle` 上），
//    所以"同档位一条砸入一条摇摆"必须分两条轨，否则同样互相覆盖。
//    ⇒ 判据跟着改成三维；**但原来的语义一个字都没丢**：
//      纵向档必须仍在键里（否则三档共用一个 `anchor_bottom`）。
// ⚠️ 用**三重引号拆开写**：这段源码里既有反引号又有单引号，
//    直接写成 `'...${cue.anim || ''}...'` 会让 JS 在 `''` 处提前闭合字符串
//    （实测把整份判据写成语法错误、**跑都跑不起来**）。
check(
  '★ 分轨键含「样式 × 纵向档」（否则三档共用一个轨级常量、互相覆盖）',
  TRANS.includes('`${cue.style.name}|${cue.pos}|${cue.anim || ' + "''" + '}`'),
);
check(
  '遍历顺序里含三个档位（top/center/bottom 都要分到轨）',
  /for \(const posName of \['top', 'center', 'bottom'\]\)/.test(TRANS),
);
// ⚠️ 2026-10-13（39 档）：id 尾部再加**动效档**（为空则不加 ⇒ 老工程 id 不变、R4）。
//    不带的话「同一样式、同一档位、两个不同动效」会生成**两条同 id 的轨**，
//    底座按 id 认层 ⇒ 后一条把前一条整个盖掉，而报告里一行差异都没有。
check(
  '轨 id 带档位后缀 + 动效档后缀（两条不同档的轨不能撞同一个 id）',
  /const id = `text_\$\{styleName\}_\$\{posName\}\$\{animName \? '_' \+ animName : ''\}`/.test(TRANS),
);
// ⚠️ `bottom` 档**故意不写**：它要减 `caption_clearance`（字幕带高度，只有底座知道），
//    写死会与字幕重叠。所以这里判的是"它被显式地排除"，而不是"它也有值"。
check(
  'bottom 档显式排除（留给底座扣字幕带，不许写死）',
  /if \(posKey === 'bottom'\) return undefined;/.test(TRANS),
);
// ⚠️ `safe_width_ratio` 必须**显式**写：不写（默认 0）底座会退回"复用 bottom_margin
//    当安全宽"的老路，而那个字段现在还要表达纵向 ⇒ 两个用途再次打架。
check(
  '文字轨显式写了 safe_width_ratio（否则退回复用老路、与纵向锚打架）',
  /safe_width_ratio: 0\.875,/.test(TRANS),
);

// ---------------------------------------------------------------------------
// ② 界面：**下拉已经撤掉了**，而且旧警示不许回来
//
// ⚠️ 2026-10-12 用户口径：「精简为和贴纸同逻辑的」——
//    文字元素面板现在只有「内容 + 样式 + 一句位置说明」，与贴纸同形。
//    `pos` 不再从界面写入 ⇒ 走 `TextAnchor::Auto`（后处理按安全落点算）。
//    这条判据**跟着翻面**：以前守"下拉在不在 + 警示撤没撤"，
//    现在守"下拉**不许回来** + 位置归后处理"。
// ---------------------------------------------------------------------------
check(
  '★ 「纵向位置」下拉**不许回来**（用户要的是贴纸同逻辑：不给选位置）',
  // ⚠️ 判**模板里的 label**，不是"整份文件不含这四个字" ——
  //    注释里正当地提到了这个下拉（"别再退回…"），按全文判会把它自己顶翻。
  //    判据要认的是**用户看得见的那一行**。
  !/<label>纵向位置<\/label>/.test(TL) && !/selItem\.pos\s*=/.test(TL),
);
check(
  '★ 那条"只在预览生效、成片贴底"的旧警示也不许回来（回来就是谎报）',
  !/纵向档位目前只在预览生效/.test(TL),
);
check(
  '旧警示的样式也没留在样式表里（防"定义了没用"）',
  !/\.warn-hint\s*\{/.test(TL),
);
// ⚠️ **与贴纸"同逻辑"要有可核对的形状**：两边那句位置说明必须同字。
//    只断言"文字支有句 hint"太弱 —— 写成"横向位置由…"（只提横向）也照样过，
//    而那正是"看起来文字和贴纸不是一回事"的来源。
const STICKER_HINT = '位置 x/y 由后处理算（每次重跑覆盖）。';
check(
  '★ 文字与贴纸有**同一句**位置说明（"同逻辑"的可核对形状）',
  TL.split(STICKER_HINT).length - 1 >= 2,
);

// ---------------------------------------------------------------------------
// ②b Rust 那一侧：**没写 `pos` 就是"让后处理算"**（`TextAnchor::Auto`）
//
// ⚠️ 这条是"界面撤了下拉"的**后端对应面**：界面不再写 `pos`，
//    所以"字段缺失"是**常态**。若兜底仍是 `Bottom`（老口径），
//    所有没带 `pos` 的文字都会贴底 —— 与"自动算"正好相反，而且不报错。
// ---------------------------------------------------------------------------
const PLACEMENT = read('crates/yekitrim-polish/src/process/placement.rs');
check(
  'Rust 侧有 `Auto` 档（与贴纸同口径：不给纵向带、在整块安全区里挑）',
  /^    Auto,$/m.test(PLACEMENT),
);
check(
  '★ `TextAnchor::parse` 认不出的兜底是 `Auto`（不是贴底）',
  /_ => Self::Auto,/.test(PLACEMENT),
);
check(
  '`Auto` 走"没偏好"的中线占位（真正落点由 2b2 求解器定）',
  /TextAnchor::Center \| TextAnchor::Auto =>/.test(PLACEMENT),
);

// ---------------------------------------------------------------------------
// ③ 文档：记着"已接通"（别让后来的人照旧结论重提需求）
// ---------------------------------------------------------------------------
check('elements-spec 有 `pos` 那一节（界面撤了、字段仍在，必须写下为什么）',
  /`pos`：界面已撤，字段仍在/.test(SPEC));
check('spec 提到了新的纵向锚字段', /anchor_bottom/.test(SPEC));
check(
  '★ spec 写的是"已接通"（别让后来的人照旧结论重提一遍需求）',
  /已接通成片|成片侧也已经接通/.test(SPEC) && !/只有预览生效，成片一律贴底/.test(SPEC),
);
check(
  '★ spec 记下了"和贴纸同逻辑"这条定稿（面板已不给选位置）',
  /和贴纸同逻辑/.test(SPEC) && /TextAnchor::Auto/.test(SPEC),
);
check('spec 记下了真机实测的三档重心（可复核）', /0\.394/.test(SPEC) && /0\.903/.test(SPEC));
check('需求文档仍有 T7 纵向锚一节（供底座侧追溯）', /T7.*纵向锚|纵向锚.*T7/.test(REQ));

// ---------------------------------------------------------------------------
// ④ 判据的前提：ASS 对齐码那一列**至今不表达纵向**
//
// ⚠️ Style 行在源码里是**跨行拼接**的（模板字面量 + `+` 续一行），
//    所以不能用"匹配整行"的写法（第一版就是那么写、恒不命中）。
//    这里钉**后半段那个字面量** —— 含 `Alignment` 那一列。
//
// 字段序见 `cuesToAss` 的 Format 行：
//   Bold,Italic,Underline,StrikeOut,ScaleX,ScaleY,Spacing,Angle,BorderStyle,Outline,Shadow,
//   **Alignment**,MarginL,MarginR,MarginV,Encoding
// 实际串 `0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1`：
//   0,0,0,0(Bold..StrikeOut) 100,100(ScaleX,ScaleY) 0(Spacing) 0(Angle)
//   1(BorderStyle) 2(Outline) 1(Shadow) **2(Alignment)** 10,10,10(Margins) 1(Encoding)
//
// ⇒ 纵向**一律**由 `anchor_bottom` 表达。哪天真去改对齐码，这条会红、必须重审。
const ASS_TAIL = "'0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1'";
check(
  'ASS 对齐码仍是 2（底部居中）—— 纵向全靠 anchor_bottom；变了必须重审',
  TRANS.includes(ASS_TAIL),
);

if (process.argv.includes('--self-test')) {
  const { strict: assert } = await import('node:assert');
  let self = 0;
  // ① 退回只按样式分轨 ⇒ 必须红（这是最要命的那个回归）
  //
  // ⚠️ 2026-10-13（39 档）：键从两维变成三维 ⇒ 变异体的锚点也要跟着改，
  //    否则 `.replace` **不命中**、`oneDim === TRANS`，自证①在"变异必须命中"那句就炸
  //    （实测如此 —— 那说明变异没生效，看着像"抓不住"）。
  const THREE_DIM_KEY = '`${cue.style.name}|${cue.pos}|${cue.anim || ' + "''" + '}`';
  const oneDim = TRANS.replace(THREE_DIM_KEY, 'cue.style.name');
  assert.notEqual(oneDim, TRANS, '自证：变异必须命中');
  assert.equal(oneDim.includes(THREE_DIM_KEY), false, '自证①：一维分轨必须红');
  // ⚠️ 再补一条：**只砍掉第三维**（退到两维）也必须红 —— 那是"同档位两个动效
  //    互相覆盖"的回归，比①更隐蔽（纵向档还在键里，肉眼看着"没改坏"）。
  const twoDim = TRANS.replace(THREE_DIM_KEY, '`${cue.style.name}|${cue.pos}`');
  assert.notEqual(twoDim, TRANS, '自证①b：变异必须命中');
  assert.equal(twoDim.includes('${cue.anim'), false, '自证①b：退到两维必须红');
  self += 1;
  // ② 算出了锚却不写进轨 ⇒ 必须红
  const notWritten = TRANS.replace(/anchor_bottom:\s*anchorBottomFor\(/, 'anchor_bottom_unused: anchorBottomFor(');
  assert.notEqual(notWritten, TRANS, '自证②：变异必须命中');
  assert.equal(/anchor_bottom:\s*anchorBottomFor\(/.test(notWritten), false, '自证②：不写进轨必须红');
  self += 1;
  // ③ 旧警示被加回来 ⇒ 必须红（判据是双向的：既防没接线，也防谎报）
  const warnBack = TL.replace(
    '<label>样式</label>',
    '<div class="hint warn-hint">纵向档位目前只在预览生效，成片会贴底。</div><label>样式</label>',
  );
  assert.notEqual(warnBack, TL, '自证③：变异必须命中');
  assert.equal(/纵向档位目前只在预览生效/.test(warnBack), true, '自证③：警示回来必须被检出');
  self += 1;
  // ④ `safe_width_ratio` 被拿掉 ⇒ 必须红（那等于退回两用途打架的老路）
  // ⚠️ 这个字面量在源码里出现**多次**（字幕轨一处、文字轨一处）—— 只 δÌ滻Îļþ 一次
  //    是**不翻面**的（那是变异没生效，看着像抓不住）。要全替换。
  const noSafeW = TRANS.replaceAll('safe_width_ratio: 0.875,', '');
  assert.notEqual(noSafeW, TRANS, '自证④：变异必须命中');
  assert.equal(/safe_width_ratio: 0\.875,/.test(noSafeW), false, '自证④：拿掉安全宽必须红');
  self += 1;
  // ⑤ **「纵向位置」下拉被加回来** ⇒ 必须红（用户要的是贴纸同逻辑：不给选位置）。
  //    ⚠️ 连 `selItem.pos =` 那半也要覆盖 —— 只藏 label 而逻辑还在写 `pos`
  //    是"看着撤了、实际还写"，比不撤更坏（界面与落点对不上）。
  const posBack = TL.replace('<label>样式</label>', '<label>纵向位置</label><select :value="selItem.pos"></select><label>样式</label>');
  assert.notEqual(posBack, TL, '自证⑤：变异必须命中');
  assert.equal(/<label>纵向位置<\/label>/.test(posBack), true, '自证⑤：下拉回来必须被检出');
  self += 1;
  // ⑥ Rust 兜底退回 `Bottom` ⇒ 必须红（界面已经不写 `pos` 了，
  //    退回贴底就是"所有没带 pos 的文字统统贴底"，与"自动算"相反且不报错）。
  const bottomFallback = PLACEMENT.replace('_ => Self::Auto,', '_ => Self::Bottom,');
  assert.notEqual(bottomFallback, PLACEMENT, '自证⑥：变异必须命中');
  assert.equal(/_ => Self::Auto,/.test(bottomFallback), false, '自证⑥：退回贴底必须红');
  self += 1;
  console.log(`✓ 文字纵向档位接线自证：${self} 项（一维分轨 / 不写进轨 / 警示回潮 / 拿掉安全宽 / 下拉回潮 / 兜底退回贴底，都必须红）`);
  process.exit(0);
}

if (failures.length) {
  console.error(`✗ 文字纵向档位的接线断了：${failures.length} 项红（共 ${checks} 项）\n`);
  for (const f of failures) console.error('  - ' + f);
  console.error('\n  ⇒ 这条判据守的是**用户看得见的那件事**：他在面板上选「上」，');
  console.error('     成片里就该在上。整条链是 `pos` → 转译器 `anchor_bottom` → 底座 `text_layout`。');
  console.error('     ⚠️ 分轨必须是**两维**的：`anchor_bottom` 是轨级常量、`pos` 是逐事件的，');
  console.error('        只按样式分轨时同一样式的两个档位会共用一个常量（后写覆盖先写，');
  console.error('        而报告里一行差异都没有）。接口见 docs/plan/dhampir-elements-request.md。');
  process.exit(1);
}
console.log(
  `✓ 文字纵向档位已接通成片：${checks} 项（转译器两维分轨 / 界面无旧警示 / 文档已更新 / ASS 前提成立）`,
);
