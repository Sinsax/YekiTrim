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
// ⚠️ `legacy: true` 的老档（`perchar_big` / `perchar_small`）**有意不进下拉**：它们只为
//    "认得出老工程"而留着（删了会让 `effect_anim` 变成认不出的档、一保存就丢）。
//    ②「界面能选」对它们不成立是**设计**，不是断链 —— 从判据里排除，别再把设计报成缺陷。
const legacyDropdownKeys = [...vueSrc.matchAll(/\{ value: '([a-z_0-9]+)', label: '[^']+', legacy: true \}/g)]
  .map((m) => m[1]);

console.log('39 档 × 四标准验收表');
console.log('='.repeat(72));
console.log(`  档位来源：引擎 ${engineKeys.length} · 转译器 ${translatorKeys.length} · 下拉 ${dropdownKeys.length}`);

// ⚠⚠ 2026-10-14（三轴收敛）：档位从 **39 收到 9**（两边各 9 档）。
//   硬编码 39 在收敛后**永远红** —— 而它又正好在**每次都要跑**的路上，
//   结果就是整张验收表被当成“本来就红”而被无视——它真正报的那 9 条反而没人看。
//   ⇒ 改成**下限**：九档是收敛后的定义，每改一档都要有意图（而不是增到 40 就绿）。
ok(engineKeys.length >= 9, `引擎档数不应少于 9（三轴收敛后的定义），实得 ${engineKeys.length}`);
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
  // ⚠️ **2026-10-18 底座新增的三条原语**（本判据 2026-11-24 才补进来）：
  //    `per_char_out_frames`（出场逐字窗口，窗口锚在出点）
  //    `scale_curve` + `scale_cycle_ms`（周期缩放，心跳那一档）
  //    底座侧证据：`Dhampir/crates/dhampir-timeline/src/layer.rs:971`（字段声明）、
  //    `crates/dhampir-core/src/overlay.rs:626`（读 `style.per_char_out_frames`）、
  //    `overlay.rs:565-571`（`text_motion::scale_curve_params` 求值）、
  //    `schema/doc-v1.schema.json` 的 `SubtitleStyle.properties`（v0.5.0）。
  //    ⇒ 缺这三条时**转译器与底座都是对的**、只有本表报“写出底座不认的键”（假红）。
  'per_char_out_frames', 'scale_curve', 'scale_cycle_ms',
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

// ===========================================================================
// ④ 的取数工具（2026-11-24 修：按花括号配对切档，不再按行）
// ===========================================================================
//
// ⚠️⚠️ 上一版是「从 `    <tier>: {` 那一行起、到下一个 `    },` 止」，两个坏处：
//
//   ① **单行档会吃进后面档位的键**。档位表里 `pop` / `fade` / `shake` 都是**单行**
//      （`fade: { sort: 'transition' },`）—— 它们的 `},` 就在**同一行**，而循环
//      只找「下一条以 4 空格 `},` 开头的行」⇒ 一路吃到 `perchar` 那个多行档的结尾。
//      实测症状：`vtext` / `pop` / `fade` / `shake` **四档全被报出 `per_char_out_frames`**
//      ——而那是 `perchar` 那一档的键。**假红**，而且指向一个不存在的缺陷。
//   ② **驼峰键根本进不了样本**：取键的正则写的是 `^ {6}([a-z_0-9]+):`，
//      `tiltDeg` / `fontWeight` / `arcToDefault` 这些**首字母后就有大写**的键
//      永远匹配不上 ⇒ 它们从未被核过（那一半判据是**空转**的）。
//
// ⇒ 改成**按花括号配对**切出这一档的对象体，再用一个极小的扫描器取 depth-1 的键
//    （跳过注释与字符串）。单行 / 多行 / 嵌套三种写法一视同仁。

/** 从 `ANIM_PRESETS` 源码里切出 `<tier>: { … }` 的**对象体**（不含外层花括号）。 */
function slicePresetBody(text, tier) {
  const re = new RegExp('(?:^|\\n) {4}' + tier + ': \\{');
  const m = re.exec(text);
  if (!m) return null;
  const open = m.index + m[0].length - 1; // 指向 `{`
  const from = open + 1;
  let depth = 1;
  for (let i = from; i < text.length; i += 1) {
    const c = text[i];
    if (c === '/' && text[i + 1] === '/') { while (i < text.length && text[i] !== '\n') i += 1; continue; }
    if (c === '"' || c === "'" || c === '`') {
      const q = c; i += 1;
      while (i < text.length && text[i] !== q) { if (text[i] === '\\') i += 1; i += 1; }
      i += 1; continue;
    }
    if (c === '{') depth += 1;
    else if (c === '}') { depth -= 1; if (depth === 0) return text.slice(from, i); }
  }
  return null;
}

/** 对象体里 **depth 1** 的键名（嵌套对象/数组里的不算）；跳过注释与字符串。 */
function topLevelKeys(body) {
  const keys = [];
  let depth = 0;
  let i = 0;
  while (i < body.length) {
    const c = body[i];
    if (c === '/' && body[i + 1] === '/') { while (i < body.length && body[i] !== '\n') i += 1; continue; }
    if (c === '"' || c === "'" || c === '`') {
      const q = c; i += 1;
      while (i < body.length && body[i] !== q) { if (body[i] === '\\') i += 1; i += 1; }
      i += 1; continue;
    }
    if (c === '{' || c === '[') { depth += 1; i += 1; continue; }
    if (c === '}' || c === ']') { depth -= 1; i += 1; continue; }
    if (depth === 0 && /[A-Za-z_]/.test(c)) {
      let j = i;
      while (j < body.length && /[A-Za-z_0-9]/.test(body[j])) j += 1;
      let k = j;
      while (k < body.length && /\\s/.test(body[k])) k += 1;
      if (body[k] === ':') keys.push(body.slice(i, j));
      i = j; continue;
    }
    i += 1;
  }
  return keys;
}

/** `key: { … }` / `key: [ … ]` 的**内层**原文；不是对象/数组就返回 null。 */
function subBody(body, key) {
  const re = new RegExp('(?:^|[{,\\s])' + key + '\\s*:');
  const m = re.exec(body);
  if (!m) return null;
  let i = m.index + m[0].length;
  while (i < body.length && /\\s/.test(body[i])) i += 1;
  const open = body[i];
  if (open !== '{' && open !== '[') return null;
  const close = open === '{' ? '}' : ']';
  const from = i + 1;
  let depth = 0;
  for (; i < body.length; i += 1) {
    const c = body[i];
    if (c === '/' && body[i + 1] === '/') { while (i < body.length && body[i] !== '\n') i += 1; continue; }
    if (c === '"' || c === "'" || c === '`') {
      const q = c; i += 1;
      while (i < body.length && body[i] !== q) { if (body[i] === '\\') i += 1; i += 1; }
      continue;
    }
    if (c === open) depth += 1;
    else if (c === close) { depth -= 1; if (depth === 0) return body.slice(from, i); }
  }
  return null;
}

/** 扁平对象列表（如 `scale_curve: [{ ms, scale }, …]`）里出现的**全部**键名。 */
function flatKeys(text) {
  return [...text.matchAll(/[{,]\\s*([A-Za-z_][A-Za-z_0-9]*)\\s*:/g)].map((m) => m[1]);
}

// ===========================================================================
// ④ 的**键集权威**：底座的 `doc-v1.schema.json`
// ===========================================================================
//
// ⚠️ 为什么不再只用手抄白名单：白名单是**手抄的底座键集**，底座加了新原语它**不会
//    自己知道**。2026-10-18 底座加了 `per_char_out_frames` / `scale_curve` /
//    `scale_cycle_ms`，本表没跟 ⇒ 报「写出底座不认的键」，而**转译器与底座都正确**。
//    手抄表的老化是**单向**的（只会漏、不会多），症状恰好是「判据全红、代码全对」。
//
// ⇒ 读得到底座 schema 就以它为准（机器判据），读不到再退回手抄白名单并**明说没跑**。
const DHAMPIR_ROOT = process.env.DHAMPIR_ROOT || join(ROOT, '..', 'Dhampir');
const BASE_SCHEMA = join(DHAMPIR_ROOT, 'schema', 'doc-v1.schema.json');
function loadBaseKeySets() {
  if (!existsSync(BASE_SCHEMA)) return null;
  let doc;
  try { doc = JSON.parse(readFileSync(BASE_SCHEMA, 'utf8')); } catch { return null; }
  const defs = doc.$defs || doc.definitions || {};
  const props = (n) => new Set(Object.keys(defs?.[n]?.properties ?? {}));
  const style = props('SubtitleStyle');
  if (style.size === 0) return null;
  return { style, perChar: props('PerCharSpec'), scalePoint: props('ScalePoint') };
}
const BASE_KEYS = loadBaseKeySets();
const STYLE_KEYS = BASE_KEYS ? BASE_KEYS.style : TRANSLATOR_KEYS;
const PERCHAR_ALLOWED = BASE_KEYS ? BASE_KEYS.perChar : PERCHAR_KEYS;
const SCALE_POINT_ALLOWED = BASE_KEYS && BASE_KEYS.scalePoint.size > 0
  ? BASE_KEYS.scalePoint
  : new Set(['ms', 'scale']);
console.log('  ④ 键集权威：' + (BASE_KEYS
  ? (BASE_SCHEMA + '（底座 SubtitleStyle / PerCharSpec / ScalePoint）')
  : '底座 schema 不在 -> 退回手抄白名单（· SKIP 交叉核对）'));

const rows = [];
for (const tier of engineKeys) {
  const spec = E.ANIM_PRESETS[tier];
  const row = { tier, c1: false, c2: false, c3: false, c4: false, note: '' };

  // ---- ① 配置能表达：转译器认得这个名字 ----
  row.c1 = translatorSet.has(tier);
  ok(row.c1, `① ${tier}：转译器不认得这个名字（用户写了会报"认不出的档"）`);

  // ---- ② 界面能选：下拉里有（`legacy: true` 的老档例外，见上面的说明） ----
  const isLegacy = legacyDropdownKeys.includes(tier);
  row.c2 = dropdownSet.has(tier) || isLegacy;
  if (isLegacy) row.note = '老档别名：只为识别老工程，有意不进下拉';
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
    // ⚠ ⚠ 2026-10-14：三轴收敛后档名从 `fade_only` 改成了 **`fade`**。
    //   旧名字留在这里 ⇒ 豁免**永远不生效**，`fade` 被当成“预览不动”报红
    //   （而它本来就是**有意为之的空档**：淡入淡出不在这几个数上）。
    // ⚠ 心跳也要算进去：它**不走** `scaleInFrom/Peak`（双峰周期函数，
    //   见 `heartbeatScaleAt`），只看旧字段会把它误判成恒等。
    const numeric = ['scaleInFrom', 'scaleInPeak', 'shakePx', 'shakeHz', 'squashY',
      'tiltDeg', 'swayDeg']
      .map((k) => spec[k])
      .filter((v) => v !== undefined);
    const hbActive = typeof spec.heartbeatAmp === 'number' && spec.heartbeatAmp > 1;
    const flatByDesign = tier === 'fade';
    previewActive = numeric.some((v) => Math.abs(v) > 1e-6) || hbActive || flatByDesign;
    if (flatByDesign) row.note = '只淡入淡出：无缩放/抖动（设计如此）';
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
  // ⚠️⚠️ **按花括号配对切档**（2026-11-24 修）。第一版写成「从 `    <tier>: {` 那一行起、
  //    到下一个 `^    },` 止」，对**单行档**（`fade: { sort: 'transition' },`）会一路
  //    吃到**后面**那个多行档的结尾 ⇒ `vtext` / `pop` / `fade` / `shake` 四档都拿到了
  //    `perchar` 的 `per_char_out_frames`，被报成「写出底座不认的键」（**假红**）。
  //    而且取键正则 `^ {6}([a-z_0-9]+):` 看不见**驼峰键**（`tiltDeg` / `fontWeight` …）
  //    ⇒ 那一半判据一直在**空转**。两处一起改：切档按 `{}` 配对、取键按 depth 1 扫。
  const body = slicePresetBody(tBlock[1], tier);
  const topKeys = body === null ? [] : topLevelKeys(body);
  // ⚠️ 排除 `sort` —— 它是**表内部的三轴归属标签**，从不被写出去（2026-10-14 记）。
  // ⚠️ **驼峰键是表内部字段**（`tiltDeg` / `squashY` / `fontWeight` / `arcToDefault` /
  //    `heartbeatAmp` …）：`translate` 拿它们算 `transform` 关键帧，或经
  //    `put('font_weight', …)` 转成蛇形再写 —— **从不原样落到底座**。所以 ④ 只核蛇形键。
  const TABLE_ONLY = new Set(['sort']);
  const emitted = topKeys.filter((k) => /^[a-z][a-z_0-9]*$/.test(k) && !TABLE_ONLY.has(k));
  const badKeys = emitted.filter((k) => !STYLE_KEYS.has(k));
  // `per_char` 的子键（底座 `PerCharSpec`）
  const perCharKeys = body === null ? [] : topLevelKeys(subBody(body, 'perChar') ?? '');
  const badPerChar = perCharKeys.filter((k) => !PERCHAR_ALLOWED.has(k));
  // `scale_curve` 的点（底座 `ScalePoint`：`ms` / `scale`）
  const scaleKeys = body === null ? [] : flatKeys(subBody(body, 'scale_curve') ?? '');
  const badScale = [...new Set(scaleKeys)].filter((k) => !SCALE_POINT_ALLOWED.has(k));
  row.c4 = body !== null && badKeys.length === 0
    && badPerChar.length === 0 && badScale.length === 0;
  ok(body !== null, `④ ${tier}：转译器表里找不到这一档（结构没对上）`);
  // ⚠️ **反空转**：每档至少得提出一个键来（连 `fade` 都有 `sort`）。提不出来 = 切档/取键
  //    那一步又坏了 —— 那时 `badKeys` 恒空，判据会**假装通过**。
  ok(topKeys.length > 0,
    `④ ${tier}：一个键都没取到 —— 判据空转（切档/取键的锚点漂了）`);
  ok(badKeys.length === 0, `④ ${tier}：转译器写出了底座**不认**的键 ${JSON.stringify(badKeys)}`
    + ` —— 底座对认不出的键**静默忽略** ⇒ 成片里这一档什么都没发生`
    + `（键集来源：${BASE_KEYS ? BASE_SCHEMA : '手抄白名单'}）`);
  ok(badPerChar.length === 0, `④ ${tier}：per_char 里有底座**不认**的子键 ${JSON.stringify(badPerChar)}`
    + ' —— 同上，静默忽略');
  ok(badScale.length === 0, `④ ${tier}：scale_curve 里有底座**不认**的字段 ${JSON.stringify(badScale)}`
    + ' —— 同样是静默忽略（心跳那一档会整个退化成恒等）');

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
