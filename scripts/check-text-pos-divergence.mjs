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
// # ⚠️⚠️ 2026-10-20 口径更新：**文字层整条退役，界面上的 `pos` 下拉与纵向档位一起删了**
//
// 用户口径：「算了，删除文字层吧」。⇒ 老工程里的 `element{kind:"text"}` 在
// **读时丢弃并计数**（`project/legacy.rs::RetiredCounts.text_layer`），编辑器不再有
// 文字元素面板（纵向位置下拉/三轴动效/角度滑杆全删）。于是本判据旧的 ②b 段
// （Rust `TextAnchor::Auto` 三档、兜底不是贴底）**永久红且指错地方** —— 那些代码是
// **按新口径删掉的**，不是坏了。已退役，改成「退役守卫」（排版实现不许回来 + 老数据丢弃计数）。
//
// **转译器那半仍然有效**：转译器直接读 `polish.toml`（不经过 Rust 的读时丢弃），
// 所以**未保存过的老工程**走底座那条腿时仍会产出文字轨 —— `pos → anchor_bottom`
// 与分轨/轨 id 的不变式必须继续成立，否则老工程在底座里会「位置错/互相覆盖」且不报错。
//
// # 这条判据盯什么（2026-10-20 之后）
//
//   ① 转译器必须把 `pos` 真的写进 `anchor_bottom`，且分轨键覆盖全部**轨级常量**
//      —— `anchor_bottom` / `scale_in_*` / 三轴档 / 颜色 / **落点** 都是轨级参数，
//      少一维就两档共用一个常量（后写覆盖先写，报告里一行差异都没有）；
//   ② 界面上的**旧警示不许回来**（回来就是谎报）；**纵向位置下拉也不许回来**
//      （文字层面板已删；它若单独回来 = 用户能设、成片里没有）；
//   ③ 规格文档必须记着这条已接通（别让后来的人照旧结论重新提一遍需求）；
//   ④ **判据的前提**：ASS 对齐码仍是 `2` —— 那一列至今不表达纵向，
//      纵向全靠 `anchor_bottom`。它若变了，本判据要重审。
//   ⑤ **退役守卫**：Rust 的 `TextAnchor` / `TextLayout` 定义不许回来；
//      老数据必须读时丢弃并计数（日志点名）。
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
//
//    ⚠️⚠️ **2026-10-13 两级化（`df5bf46`）后又变成四维**：
//    动效那一维**拆成了两条轴**（`styleAnim` / `effectAnim`）——
//    「竖排 + 抖动」与「纯竖排」是两种不同的轨级参数组合，必须分轨。
//    ⇒ 判据跟着改。**第一版漏改了这条**，仓里因此红了一次
//      （守的是"纵向档不能丢"，而纵向档其实还在 —— **判据过期，不是代码坏**）。
//    ⚠️ **改转译器的分轨键/轨 id 时，这条判据的锚点必须一起改** ——
//       它匹配的是**源码字面量**，不是行为。
//    ⚠️⚠️ **2026-10-13 颜色接入后又变成五维**（同一理由的**第四次**出现）：
//    底座的 `SubtitleStyle.color` 是 `[u8;4]` **轨级**字段，而颜色是**逐事件**的
//    ⇒ 同轨两种颜色只能活一种（后写覆盖先写）。
//    ⚠️ **实测症状**：7 条不同颜色的文字元素渲染出来**全是白的** ——
//       而 ASS 文件、转译报告、任务日志**三者都完全正常**。
//    ⇒ 判据跟着改。**改转译器的分轨键/轨 id 时，这条判据的锚点必须一起改** ——
//       它匹配的是**源码字面量**，不是行为（这一条已经绊过两次了）。
// ⚠️ 用**三重引号拆开写**：这段源码里既有反引号又有单引号，
//    直接写成 `'...${cue.anim || ''}...'` 会让 JS 在 `''` 处提前闭合字符串
//    （实测把整份判据写成语法错误、**跑都跑不起来**）。
check(
  '★ 分轨键含「样式 × 纵向档 × 样式动效 × 动效动效 × **过渡动效** × 颜色 × **落点**」' +
    '（都是轨级参数，少一维就互相覆盖）',
  // ⚠ 2026-10-14（三轴）：键里又多了 `transitionAnim` 一维；
  //   2026-10-15 **落点（`originKey`）** 又进来一维（`origin` 是轨级字段，两个不同落点
  //   必须分轨，否则后写的覆盖先写的 —— 与本文件里颜色那处同一个理由）。
  //   ⇒ 现在键是 **7 维**：style|pos|styleAnim|effectAnim|transitionAnim|color|origin。
  //   这条判据的锚点是**源码字面量**，改分轨键就必须一起改（已经绊过三次）。
  // ⚠ 用**正则**而不是字面量：这段源码里同时有反引号与单引号，
  //   写成字面量会在 `''` 处提前闭合（已踩三次）。
  /`\$\{cue\.style\.name\}\|\$\{cue\.pos\}\|\$\{cue\.styleAnim[\s\S]{0,200}?\$\{cue\.transitionAnim[\s\S]{0,120}?\$\{colorKey\}\|\$\{originKey\}`/.test(TRANS),
);
check(
  '遍历顺序里含三个档位（top/center/bottom 都要分到轨）',
  /for \(const posName of \['top', 'center', 'bottom'\]\)/.test(TRANS),
);
// ⚠️ 2026-10-13（39 档）：id 尾部再加**动效档**（为空则不加 ⇒ 老工程 id 不变、R4）。
//    不带的话「同一样式、同一档位、两个不同动效」会生成**两条同 id 的轨**，
//    底座按 id 认层 ⇒ 后一条把前一条整个盖掉，而报告里一行差异都没有。
//    ⚠️ 两级化后尾部那个档名是 `animTag`（两轴拼出来的，`fade_only`/`pop_std`
//    那种单轴档与老 `anim` **逐字符相同** ⇒ R4 不变）。
check(
  '轨 id 带档位后缀 + 动效档后缀 + 落点 + 颜色（两条不同档的轨不能撞同一个 id）',
  // ⚠ 2026-10-15 起 id 还带 `posTag`（落点）、2026-10-30 起带 `colorTag`（颜色）——
  //   三者与分轨键**同一个理由**：轨级参数不同就必须不同 id，否则底座按 id 认层、
  //   后一条把前一条整个盖掉，而报告里一行差异都没有。
  /const id = `text_\$\{styleName\}_\$\{posName\}\$\{posTag\}\$\{animTag \? '_' \+ animTag : ''\}`[\s\S]{0,80}?\$\{colorTag \? '_' \+ colorTag : ''\}/.test(TRANS),
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
// ⚠️ **与贴纸"同逻辑"要有可核对的形状**。
//
// ⚠️⚠️ **2026-10-13 改口径：那句位置说明已经从面板上删掉了**（用户原话：
//     「位置 x/y 由后处理算（每次重跑覆盖）。这些文案，没有必要」）。
//     原判据要求这句话在文字支与贴纸支**各出现一次**（`>= 2`）——
//     它守的其实是**同一件事被说了两遍**。现在两边都不说这句了。
//
// ⚠️ **但"同逻辑"本身一个字都没变**，而且它仍然必须有判据 ——
//    真不变式是：**两条支路都"不给选位置"**（位置由后处理算），
//    而不是"两条支路都写着同一句话"。后者只是前者的**曾经的一种实现**。
//    只删断言会让"文字支/贴纸支各自长出一个位置下拉"无人看守。
{
  const STICKER_HINT = '位置 x/y 由后处理算（每次重跑覆盖）。'
  // ① 那句被点名的废话**不许回来**（回来了就是又把同一件事说了两遍）。
  check(
    '★ 那句「位置 x/y 由后处理算」已被删除（用户明确说没有必要）',
    !TL.includes(STICKER_HINT),
    `面板上又出现了「${STICKER_HINT}」—— 用户已明确要求删掉这类文案`,
  )
  // ② 真不变式：两条支路**都没有**位置输入控件（下拉或数字框）。
  //    ⚠️ 盯的是"有没有给出选位置的入口"，不是文案 —— 而且
  //       **必须先剥注释再匹配**：那句"别再退回纵向位置下拉"的告警**本身**
  //       就写在注释里、且含「纵向位置」四个字 ⇒ 不剥注释会**把告警当成违规**。
  //       （本仓踩过同类假阳性：`check_module_boundaries.py` 的上一版手写 grep。）
  const TL_MARKUP = TL.replace(/<!--[\s\S]*?-->/g, '')
  const cut = TL_MARKUP.indexOf('selItem.kind')
  const textSeg = TL_MARKUP.slice(cut, TL_MARKUP.indexOf('StickerPicker', cut))
  for (const [name, seg] of [['文字支', textSeg], ['贴纸支', TL_MARKUP.slice(TL_MARKUP.indexOf('StickerPicker', cut))]]) {
    check(
      `★ ${name}**没有**位置下拉（位置一律由后处理算）`,
      !/<select[^>]*selItem\.pos/.test(seg) && !/<label>\s*(纵向|横向)位置/.test(seg),
      `${name}又出现了选位置的入口 —— 用户要的是"和贴纸同逻辑：不给选位置"`,
    )
  }
}

// ---------------------------------------------------------------------------
// ②b Rust 那一侧：**没写 `pos` 就是"让后处理算"**（`TextAnchor::Auto`）
//
// ⚠️ 这条是"界面撤了下拉"的**后端对应面**：界面不再写 `pos`，
//    所以"字段缺失"是**常态**。若兜底仍是 `Bottom`（老口径），
//    所有没带 `pos` 的文字都会贴底 —— 与"自动算"正好相反，而且不报错。
// ---------------------------------------------------------------------------
//
// ⚠️⚠️ **2026-10-20 口径更新：文字层退役。**
//   旧断言（已退役）：`placement.rs` 里必须有 `TextAnchor::Auto` 档、
//   `TextAnchor::parse` 兜底是 `Auto`、`Auto` 走中线占位。
//   为什么退役：文字层整条退役（用户口径「算了，删除文字层吧」）——
//   `TextAnchor` / `TextLayout` / `wrap_text` 随文字排版一起删了（见 placement.rs:548 那段注释），
//   老工程里的 `element{kind:"text"}` 在 **project/legacy.rs 读时丢弃并计数**。
//   ⇒ 旧断言在"产品按新口径删干净了"之后永久红，而且红的时候指错地方。
//   现在锁的是**退役本身**：排版实现不许偷偷回来、老数据必须被丢弃并计数（不静默保留）。
const PLACEMENT = read('crates/yekitrim-polish/src/process/placement.rs');
const LEGACY = read('crates/yekitrim-polish/src/project/legacy.rs');
check(
  '★ 文字层已退役：Rust 里 `TextAnchor` / `TextLayout` 的定义必须不存在（排版实现不许回来）',
  !/\benum\s+TextAnchor\b/.test(PLACEMENT) && !/\bstruct\s+TextLayout\b/.test(PLACEMENT),
);
check(
  '★ 老工程里的文字层必须**读时丢弃并计数**（不是静默保留、也不是静默删磁盘）',
  /text_layer/.test(LEGACY) && /retired\.text_layer > 0/.test(LEGACY),
);
check(
  '退役计数必须在日志里点名（别静默丢用户数据）',
  /已退役文字层/.test(LEGACY),
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
// ⚠ 2026-10-14：`Outline` / `Shadow` 从字面量改成了**模板插值**
//   （元素描边走自己的四档，问题 ③）。对齐码那一列位置不变，
//   判据改成钉"对齐码仍为 2 且不受描边插值影响"。
const ASS_TAIL = '${outlinePx ?? 2},${shadowPx ?? 1},2,10,10,10,1';
check(
  'ASS 对齐码仍是 2（底部居中）—— 纵向全靠 anchor_bottom；变了必须重审',
  TRANS.includes(ASS_TAIL),
);

if (process.argv.includes('--self-test')) {
  const { strict: assert } = await import('node:assert');
  let self = 0;
  // ① 分轨键少一维 ⇒ 必须红。⚠️ **从真源码现场抽**（不手抄）——
  //   分轨键改过五次（两维 → … → 七维），手抄的锚点每改一次就断一次。
  const KEY_LINES = TRANS.split('\n').filter((l) =>
    l.includes('${cue.style.name}|${cue.pos}') ||
    l.includes('${colorKey}|${originKey}'));
  assert.equal(KEY_LINES.length, 2, '自证：抽不到分轨键那两行');
  const FULL_KEY = KEY_LINES[0].slice(KEY_LINES[0].indexOf('`')) +
    '\n' + KEY_LINES[1].slice(0, -1);
  // ①a 退到两维（只留 style|pos）⇒ 必须红
  const twoDim = TRANS.replace(FULL_KEY, '`${cue.style.name}|${cue.pos}`');
  assert.notEqual(twoDim, TRANS, '自证①a：变异必须命中');
  assert.equal(twoDim.includes('${cue.styleAnim'), false, '自证①a：退到两维必须红');
  // ①b 砍掉**落点**那一维（origin）⇒ 必须红（2026-10-15 新加的那一维）
  const noOrigin = TRANS.replace(FULL_KEY, FULL_KEY.replace('|${originKey}', ''));
  assert.notEqual(noOrigin, TRANS, '自证①b：变异必须命中');
  assert.equal(noOrigin.includes('${originKey}'), false, '自证①b：砍掉落点维必须红');
  self += 1;
  // ② 算出了锚却不写进轨 ⇒ 必须红
  const notWritten = TRANS.replace(/anchor_bottom:\s*anchorBottomFor\(/, 'anchor_bottom_unused: anchorBottomFor(');
  assert.notEqual(notWritten, TRANS, '自证②：变异必须命中');
  assert.equal(/anchor_bottom:\s*anchorBottomFor\(/.test(notWritten), false, '自证②：不写进轨必须红');
  self += 1;
  // ③ 旧警示被加回来 ⇒ 必须红（判据是双向的：既防没接线，也防谎报）
  //   ⚠️ 锚点必须用**模板里真实存在**的一行 —— 文字支已删，旧的 `<label>排版</label>` 不在了。
  const warnBack = TL.replace(
    '<label class="pos-lbl">位置</label>',
    '<div class="hint warn-hint">纵向档位目前只在预览生效，成片会贴底。</div><label class="pos-lbl">位置</label>',
  );
  assert.notEqual(warnBack, TL, '自证③：变异必须命中（模板锚点变了就要改自证）');
  assert.equal(/纵向档位目前只在预览生效/.test(warnBack), true, '自证③：警示回来必须被检出');
  self += 1;
  // ④ `safe_width_ratio` 被拿掉 ⇒ 必须红（那等于退回两用途打架的老路）
  const noSafeW = TRANS.replaceAll('safe_width_ratio: 0.875,', '');
  assert.notEqual(noSafeW, TRANS, '自证④：变异必须命中');
  assert.equal(/safe_width_ratio: 0\.875,/.test(noSafeW), false, '自证④：拿掉安全宽必须红');
  self += 1;
  // ⑤ **「纵向位置」下拉被加回来** ⇒ 必须红（用户要的是贴纸同逻辑：不给选位置）。
  const posBack = TL.replace(
    '<label class="pos-lbl">位置</label>',
    '<label>纵向位置</label><select :value="selItem.pos"></select><label class="pos-lbl">位置</label>',
  );
  assert.notEqual(posBack, TL, '自证⑤：变异必须命中');
  assert.equal(/<label>纵向位置<\/label>/.test(posBack), true, '自证⑤：下拉回来必须被检出');
  self += 1;
  // ⑥ Rust 文字排版被加回来（`enum TextAnchor`）⇒ 必须红。
  //   旧版自证是"兜底退回 Bottom"—— 那段随文字层删了；现在锁的是"排版实现不许回来"。
  const back = PLACEMENT.replace('// ⚠️ 文字层退役', 'enum TextAnchor { Auto }\n// ⚠️ 文字层退役');
  assert.notEqual(back, PLACEMENT, '自证⑥：变异必须命中（锚点变了就要改自证）');
  assert.equal(/\benum\s+TextAnchor\b/.test(back), true, '自证⑥：把 TextAnchor 加回来必须被检出');
  self += 1;
  console.log('✓ 文字纵向档位接线自证：' + self + ' 项（七维分轨砍维 / 不写进轨 / 警示回潮 / 拿掉安全宽 / 下拉回潮 / 排版实现回潮，都必须红）');
  process.exit(0);
}

if (failures.length) {
  console.error(`✗ 文字纵向档位的接线断了：${failures.length} 项红（共 ${checks} 项）\n`);
  for (const f of failures) console.error('  - ' + f);
  console.error('\n  ⇒ 转译器那半守的是**老工程进底座时的位置正确性**：`pos` → `anchor_bottom`，');
  console.error('     且分轨键必须覆盖全部轨级参数（样式/档位/三轴动效/颜色/落点）。');
  console.error('     ⚠️ 文字层面板与 Rust 排版都已在 2026-10-20 退役 —— 那几条判据只锁"不许回来"，');
  console.error('        别再按"面板上选上、成片在上"的旧口径理解它们。');
  process.exit(1);
}
console.log(
  `✓ 文字纵向档位接线守卫：${checks} 项（转译器七维分轨 / 界面无旧下拉与旧警示 / 文档已更新 / ASS 前提成立 / 文字层退役守卫）`,
);
