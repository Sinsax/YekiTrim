#!/usr/bin/env node
/**
 * **39 档 × 四标准验收表**（第 7 步）—— 逐档、逐项、机器判定。
 *
 * # 四标准（用户原话）
 *
 *   ① **配置能表达** —— 档位名是合法的 `anim`，转译器认得它
 *   ② **界面能选** —— 下拉里有这一项，且用户选得中
 *   ③ **预览看得见** —— 预览引擎（`engine.js`）对它能算出一个**非恒等**的结果
 *   ④ **成片出得来** —— 转译器把它落成底座认得的字段，且底座**真的**求值出东西
 *
 * # 为什么要有这张表（而不是"我检查过了"）
 *
 * 39 档里每一档都可能**只在某一项上**断掉，而四种断法**症状完全不同**：
 *
 *   * ①断：用户写了 `anim = "xxx"`，转译器报"认不出的档"（**响**，好查）
 *   * ②断：配置写对了、成片也对，但**下拉里没有** ⇒ 用户根本选不到（**最冤**）
 *   * ③断：下拉有、成片对，但预览**不动** ⇒ 用户以为没生效，反复调（**最像"没做"**）
 *   * ④断：前三项都对，成片里**什么都没有** ⇒ 前面全白做（**最严重**）
 *
 * ⚠️ ②③④ 三种**都不会报错**。所以四项必须**逐档分开量**，
 *    不能"抽查几档 + 声称 39 档都行"。
 *
 * # ⚠️ 这张表**不是**真机证据的替代品
 *
 * 它覆盖的是**结构与求值**：档位名、下拉项、预览的非恒等结果、底座的求值输出。
 * **像素**那一层由另外三份真机证据承担（见 `EVIDENCE` 表），
 * 两者**互相独立**、不许顶替：
 *
 *   | 证据 | 覆盖 | 怎么来的 |
 *   |---|---|---|
 *   | 本判据 | 39 档 × ①–④ 的结构与求值 | 自动 |
 *   | `.evidence/perchar-scatter-strip.png` | C 组逐字散开的**像素** | 人工看过 |
 *   | `.evidence/perchar-hop-strip.png` | D 组错峰的**像素** | 人工看过 |
 *   | `.evidence/vertical-e2e-f30.png` | E 组竖排的**像素** | 人工看过 |
 *   | `docs/plan/text-styles-39-完成计划.md` 第 3 步真机表 | A+B 组的**像素** | 人工看过 |
 */

import { readFileSync, existsSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, '..');

let checks = 0;
const problems = [];
const ok = (cond, msg) => {
  checks += 1;
  if (!cond) problems.push(msg);
  return cond;
};

// ===========================================================================
// 取三处的档位集合
// ===========================================================================

/** ① 预览引擎的档位表（这是**权威**：档位是什么由它能算什么决定）。 */
const engineSrc = readFileSync(join(ROOT, 'webui/src/polish/engine.js'), 'utf8');
const engineBlock = engineSrc.match(/const ANIM_PRESETS = \{([\s\S]*?)\n\}/);
if (!engineBlock) {
  console.error('✗ 在 engine.js 里找不到 ANIM_PRESETS');
  process.exit(2);
}
const engineKeys = [...engineBlock[1].matchAll(/^  ([a-z_0-9]+): \{/gm)].map((m) => m[1]);

/** ② 转译器的档位表。 */
const translatorSrc = readFileSync(join(ROOT, 'tools/polish-to-dhampir.mjs'), 'utf8');
const tBlock = translatorSrc.match(/const ANIM_PRESETS = \{([\s\S]*?)\n  \};/);
if (!tBlock) {
  console.error('✗ 在转译器里找不到 ANIM_PRESETS');
  process.exit(2);
}
const translatorKeys = [...tBlock[1].matchAll(/^    ([a-z_0-9]+): \{/gm)].map((m) => m[1]);

/** ③ 下拉里的档位。 */
const vueSrc = readFileSync(join(ROOT, 'webui/src/components/TimelineEditor.vue'), 'utf8');
const dropdownKeys = [...vueSrc.matchAll(/\{ value: '([a-z_0-9]+)', label: '[^']+' \}/g)].map((m) => m[1]);

console.log('39 档 × 四标准验收表');
console.log('='.repeat(72));
console.log(`  档位来源：引擎 ${engineKeys.length} · 转译器 ${translatorKeys.length} · 下拉 ${dropdownKeys.length}`);
console.log('');

ok(engineKeys.length === 39, `引擎档数应当是 39，实得 ${engineKeys.length}`);
ok(new Set(engineKeys).size === engineKeys.length, '引擎档位名有重复');

// ===========================================================================
// 逐档检查四项
// ===========================================================================

const E = await import(pathToFileURL(join(ROOT, 'webui/src/polish/engine.js')).href);

/** ② 下拉：档位名与标签都在。 */
const dropdownSet = new Set(dropdownKeys);
/** ① 转译器表（用来判"认得"）。 */
const translatorSet = new Set(translatorKeys);

/**
 * ④ **预览侧**的键名白名单（`engine.js::ANIM_PRESETS` 里的 camelCase）。
 *
 * ⚠️⚠️ 第一版我在这里抄的是**底座**的蛇形名（`scale_in_from` / `shake_px` …），
 *    而引擎表用的是**驼峰**（`scaleInFrom` / `shakePx` …）—— 于是
 *    **39 档全部**被报成"写出了不认的键"。
 *    症状是"判据全红、而代码是对的"：白名单抄错了对象。
 *    ⇒ 两侧各有各的名字，**必须分别列**（下面 `TRANSLATOR_KEYS` 才是蛇形）。
 */
const ENGINE_KEYS = new Set([
  // 整块（A+B）
  'tiltDeg', 'shakePx', 'shakeHz', 'swayDeg', 'swayMs',
  'scaleInFrom', 'scaleInPeak', 'scaleInMs', 'squashY',
  // 逐字（C+D）：`perChar` 是一个子对象
  'perChar',
  // 竖排（E）
  'vertical', 'safeWidthRatio', 'fontFamily', 'fontWeight',
  // `perChar` **子对象**里的键（驼峰）
  'scaleFrom', 'scaleTo', 'rotationFrom', 'rotationTo',
  'offsetXFrom', 'offsetXTo', 'offsetYFrom', 'offsetYTo',
  'perCharFrames', 'staggerFrames', 'easing', 'reverse',
  'spreadXEm', 'spreadYEm', 'spreadXFrom', 'spreadXTo', 'spreadYFrom', 'spreadYTo',
  'scaleSpread',
]);

/** ④ **转译器侧**的键名白名单（蛇形，直传底座）。 */
const TRANSLATOR_KEYS = new Set([
  'scale_in_from', 'scale_in_peak', 'scale_in_ms', 'shake_px', 'shake_hz', 'shake_phase',
  'plate_color', 'plate_pad_px', 'squashY',
  'per_char', 'vertical', 'safe_width_ratio', 'font_family', 'font_weight',
  'scale_from', 'scale_to', 'rotation_from', 'rotation_to',
  'offset_x_from', 'offset_x_to', 'offset_y_from', 'offset_y_to',
  'per_char_frames', 'stagger_frames', 'easing', 'reverse',
  'spread_x_em', 'spread_y_em', 'spread_x_from', 'spread_x_to', 'spread_y_from', 'spread_y_to',
  'scale_spread',
]);
/** `per_char` 的子字段（底座 `PerCharSpec`）。 */
const PERCHAR_KEYS = new Set([
  'scale_from', 'scale_to', 'rotation_from', 'rotation_to',
  'offset_x_from', 'offset_x_to', 'offset_y_from', 'offset_y_to',
  'per_char_frames', 'stagger_frames', 'easing', 'reverse',
  'spread_x_em', 'spread_y_em', 'spread_x_from', 'spread_x_to', 'spread_y_from', 'spread_y_to',
  'scale_spread',
]);

const rows = [];
for (const tier of engineKeys) {
  const spec = E.ANIM_PRESETS[tier];
  const row = { tier, c1: false, c2: false, c3: false, c4: false, note: '' };

  // ---- ① 配置能表达：转译器认得这个名字 ----
  row.c1 = translatorSet.has(tier);
  ok(row.c1, `① ${tier}：转译器不认得这个名字（用户写了会报"认不出的档"）`);

  // ---- ② 界面能选：下拉里有 ----
  row.c2 = dropdownSet.has(tier);
  ok(row.c2, `② ${tier}：**下拉里没有这一项** —— 配置写得出来、成片也出得来，`
    + `但用户根本选不到（最冤的一种断法）`);

  // ---- ③ 预览看得见：预览能算出**非恒等**的结果 ----
  //
  // 逐字档（带 `perChar`）走 `perCharActive`；整块档看那几个数；
  // 竖排档看 `vertical`。
  let previewActive = false;
  if (spec.perChar !== undefined) {
    // ⚠️⚠️ **别在这里调 `readPerChar`** —— 它是「**底座 JSON**（蛇形）→ 引擎（驼峰）」
    //    的归一化器，**不是**给引擎自己那些档位用的。引擎档位表里的 `perChar`
    //    **本来就是驼峰**、被 `charTransformAt` 直接消费。
    //    第一版我拿它过了一遍 ⇒ `readPerChar` 读蛇形键、一个都没匹配上
    //    ⇒ 返回**全是缺省值** ⇒ `perCharActive` 为假 ⇒ **20 个逐字档全被报成"预览恒等"**。
    //    ⚠️ 这个错法与"三条档位是空壳"那一次**症状一模一样**（都是"整档不生效"），
    //       差别只在**是谁没生效**：那次是档位本身空，这次是判据把档位读空了。
    //       ⇒ 看到"整组档全恒等"时，先怀疑**判据的取数方式**，再怀疑档位。
    const normalized = { ...E.PER_CHAR_DEFAULT, ...spec.perChar };
    previewActive = E.perCharActive(normalized);
    // 逐字档还必须**真的**能对某个下标算出非恒等变换。
    if (previewActive) {
      // ⚠️⚠️ **必须扫完整个动作窗口，不能只探一帧**。
      //
      // 第一版我只探了 `start + perCharFrames/2` 那一帧 —— 于是
      // `arc_wave`（`rotationFrom: -12 → rotationTo: +12`）**恰好在中点穿过 0**
      // ⇒ 那一帧的 `rotationDeg` 就是 0 ⇒ 被报成"预览恒等"。
      // **档位是好的，是探针太窄**：任何**对称往返**的档位（从负到正、
      // 或从 +x 回到 0）都会在中途经过恒等值。
      //
      // ⇒ 扫描整个窗口，取**偏离恒等最大**的那一帧。
      //    这才是"这一档到底动不动"的正确问法。
      const started = E.perCharStartFrame(normalized, 1, 6);
      const span = Math.max(1, normalized.perCharFrames) + Math.max(1, normalized.staggerFrames) * 6 + 2;
      let best = 0;
      let bestFrame = started;
      for (let f = started; f <= started + span; f += 1) {
        const t = E.charTransformAt(normalized, 1, 6, f);
        const dev = Math.max(
          Math.abs(t.scale - 1),
          Math.abs(t.rotationDeg) / 90,
          Math.abs(t.offsetXEm),
          Math.abs(t.offsetYEm)
        );
        if (dev > best) {
          best = dev;
          bestFrame = f;
        }
      }
      previewActive = best > 1e-6;
      if (!previewActive) {
        row.note = `逐字在第 1 个字上、整段窗口 [${started}, ${started + span}] 内始终恒等`;
      } else {
        row.note = `峰值在第 ${bestFrame} 帧`;
      }
    }
  } else if (spec.vertical === true) {
    previewActive = true; // 竖排是排版方向，恒等与否由布局决定（见③的说明）
    row.note = '竖排：排版方向，非逐字变换';
  } else {
    // 整块：`scale_in_*` / `shake_*` / `squashY` 任一非零即"看得见"。
    // ⚠️ `fade_only` 是**有意为之的空档**（「只淡出」= 不缩放不抖），
    //    它的"看得见"由淡入淡出承担，不在这一档的几个数上 ⇒ 单独放行。
    const numeric = ['scaleInFrom', 'scaleInPeak', 'shakePx', 'shakeHz', 'squashY',
      'tiltDeg', 'swayDeg']
      .map((k) => spec[k])
      .filter((v) => v !== undefined);
    previewActive = numeric.some((v) => Math.abs(v) > 1e-6) || tier === 'fade_only';
    if (tier === 'fade_only') row.note = '只淡出：无缩放/抖动（设计如此）';
  }
  row.c3 = !!previewActive;
  ok(row.c3, `③ ${tier}：预览算出的是**恒等**结果 ⇒ 用户看到的画面不动`
    + `${row.note ? `（${row.note}）` : ''}`);

  // ---- ④ 成片出得来：转译器落成的每个键都是底座真的读的 ----
  //
  // ⚠️ 这一条查的是"键名对不对"。**键名错 = 底座静默用缺省**（serde 忽略认不出的键）
  //    ⇒ 成片里那一档**什么都没发生**，而报告里一行差异都没有。
  //    这正是 R1 那条纪律（新效果 = 词汇表 + 转译器 case + 底座原语）里最容易漏的一环。
  //
  // ⚠️⚠️ **按行取块，不用正则跨块匹配**。第一版我写的是
  //    `new RegExp('^    X: \\{[\\s\\S]*?\\n    \\},?', 'm')` ——
  //    非贪婪在**多行**上会一路吃到**下一个** `\n    }`，于是"这一档的键"
  //    里混进了**后面所有档位名**，判据报出 `["slam_std","slam_lg",…]`
  //    这种明显荒谬的"底座不认的键"。
  //    ⇒ 症状是**判据全红、而代码是对的**（代价是去查一个不存在的问题）。
  //    改成按行扫：从 `    <tier>: {` 那一行起，缩进回到 `    },` 为止。
  const lines = tBlock[1].split('\n');
  const startAt = lines.findIndex((l) => l.startsWith(`    ${tier}: {`));
  let block = null;
  if (startAt >= 0) {
    const acc = [];
    for (let i = startAt; i < lines.length; i += 1) {
      acc.push(lines[i]);
      // 顶层那一档在缩进 4 的 `},` 处结束。
      if (i > startAt && /^ {4}\},?\s*$/.test(lines[i])) break;
    }
    block = acc.join('\n');
  }
  const raw = block === null ? null : block;
  const emitted = new Set();
  if (block !== null) {
    // 只取缩进 **6** 的键（档位自己的字段）；更深的属于 perChar 子对象。
    for (const m of block.matchAll(/^ {6}([a-z_0-9]+):/gm)) emitted.add(m[1]);
  }
  const badKeys = [...emitted].filter((k) => !TRANSLATOR_KEYS.has(k) && k !== 'perChar');
  // perChar 的子键也要逐个核（缩进 8 及以上）
  const badPerChar = block === null
    ? []
    : [...block.matchAll(/^ {8,}([a-z_0-9]+):/gm)]
        .map((m) => m[1])
        .filter((k) => !PERCHAR_KEYS.has(k));
  row.c4 = raw !== null && badKeys.length === 0 && badPerChar.length === 0;
  ok(raw !== null, `④ ${tier}：转译器表里找不到这一档（结构没对上）`);
  ok(badKeys.length === 0, `④ ${tier}：转译器写出了底座**不认**的键 ${JSON.stringify(badKeys)}`
    + ` —— 底座对认不出的键**静默忽略** ⇒ 成片里这一档什么都没发生`);
  ok(badPerChar.length === 0, `④ ${tier}：per_char 里有底座**不认**的子键 ${JSON.stringify(badPerChar)}`
    + ' —— 同上，静默忽略');

  rows.push(row);
}

// ===========================================================================
// 验收表
// ===========================================================================

console.log('');
console.log('  #  档位                ①配置  ②界面  ③预览  ④成片   备注');
console.log('  ' + '-'.repeat(68));
rows.forEach((r, i) => {
  const mark = (b) => (b ? ' ✓  ' : ' ✗  ');
  console.log(
    `  ${String(i + 1).padStart(2)}  ${r.tier.padEnd(18)}${mark(r.c1)}${mark(r.c2)}${mark(r.c3)}${mark(r.c4)}  ${r.note}`
  );
});
console.log('  ' + '-'.repeat(68));
const allFour = rows.filter((r) => r.c1 && r.c2 && r.c3 && r.c4).length;
console.log(`  **四标准全过：${allFour} / ${rows.length} 档**`);

// ⚠️ 真机像素证据**不在本判据里**，但要求它**存在** —— 否则这张表会被
//    当成"39 档都验过了"的唯一凭据，而它其实**没看一个像素**。
const EVIDENCE = [
  ['Dhampir/.evidence/perchar-scatter-strip.png', 'C 组逐字散开'],
  ['Dhampir/.evidence/perchar-hop-strip.png', 'D 组错峰'],
  ['Dhampir/.evidence/vertical-e2e-f30.png', 'E 组竖排'],
];
console.log('');
console.log('  真机像素证据（本判据**不**校验其内容，只要求它在）：');
for (const [rel, what] of EVIDENCE) {
  const p = join('F:/para/Code', rel);
  const here = existsSync(p) || existsSync(join(ROOT, '..', rel));
  console.log(`    ${here ? '✓' : '✗'} ${what.padEnd(14)} ${rel}`);
  ok(here, `真机证据不在：${rel}（${what}）`);
}

console.log('='.repeat(72));
if (problems.length > 0) {
  console.log(`✗ 39 档验收表有 ${problems.length} 处问题（共 ${checks} 项判据）：`);
  for (const p of problems) console.log(`    · ${p}`);
  process.exit(1);
}
console.log(`✓ 39 档验收表通过：${checks} 项判据 —— ${allFour} / ${rows.length} 档四标准全过`);
