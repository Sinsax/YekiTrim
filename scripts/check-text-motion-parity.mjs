#!/usr/bin/env node
// 门禁：**文字元素动效（§5 E4a T1/T2/T4/T6）四处同口径**。
//
// # 为什么需要它
//
// 一个文字动效的数值要穿过 **四份实现**才落到屏幕上：
//
//   ① `crates/yekitrim-polish/src/render/elements.rs`   ← 配置形状（本仓）
//   ② Dhampir `crates/dhampir-timeline/src/text_motion.rs` ← **出片的逐帧求值**
//   ③ `tools/polish-to-dhampir.mjs`                     ← 把 ① 的字段名照抄给 ②
//   ④ `webui/src/polish/engine.js`                      ← **预览的逐帧求值**
//
// ② 与 ④ 是**两条独立的逐帧实现**（一个 Rust 一个 JS），而它们必须逐值相等 ——
// 否则症状是「预览里弹得挺好、成片里不弹」（或反过来），**两边各自看都正常**。
// 这类漂移在编译期、在 `cargo test`、在 `vite build` 里都**不报错**，只有对着看才发现。
//
// # 判据
//
// **不 import 任何东西**（本仓的纪律：判据零依赖、从源码文本里抽函数），而是：
//   * 从 Dhampir 的 `text_motion.rs` 里抽出 `back_out3` / `text_shake_px` 的**常数与式子**；
//   * 从 `engine.js` 里抽出 `backOut3` / `textShakePx` 的**常数与式子**；
//   * 拿**同一组采样点**分别算，要求逐值相等（1e-9）。
//
// ⚠️ 它**刻意不跑 Rust**：跑 Rust 要 GPU/编译，会让这条判据进不了常规门禁。
//    代价是它只能证明"式子在源码层面同构"，不能证明"Rust 真的这么算"——那由
//    Dhampir 自己那批单测管（`弹入窗口内先小后过冲再回落` 等）。
//    两层合起来才完整：**本判据防"改了一处忘了另一处"**，Dhampir 那批防"式子本身写错"。
//
// # 跑法
//
//     node scripts/check-text-motion-parity.mjs
//     node scripts/check-text-motion-parity.mjs --self-test   # 自证判据非空转
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const read = (rel) => readFileSync(join(repo, rel), 'utf8');

/// Dhampir 仓与本仓是**平级目录**（都在 `F:/para/Code/` 下）。
/// ⚠️ 找不到时**跳过并说明**，不报红 —— 那是"没 checkout 另一个仓"，
///    不是"代码漂了"。CI 里两个仓都在，这条就真的会跑。
const DHAMPIR = resolve(repo, '..', 'Dhampir');
function readDhampir(rel) {
  try {
    return readFileSync(join(DHAMPIR, rel), 'utf8');
  } catch {
    return null;
  }
}

let checks = 0;
const failures = [];
const check = (what, got, want) => {
  checks += 1;
  if (got !== want) failures.push(`${what}: 期望 ${want}，实得 ${got}`);
};

// ---------------------------------------------------------------------------
// ① T1 `back_out3`：两边各自的**常数**必须一致
// ---------------------------------------------------------------------------
{
  const rust = readDhampir('crates/dhampir-timeline/src/text_motion.rs');
  const js = read('webui/src/polish/engine.js');

  // Rust: `const S: f32 = 3.0;` 且 `u * u * ((S + 1.0) * u + S) + 1.0`
  const rustS = rust && rust.match(/fn back_out3[\s\S]{0,200}?const S: f32 = ([\d.]+);/);
  const rustBody = rust && rust.match(/fn back_out3[\s\S]{0,300}?\n\}/);
  check('Rust back_out3 里找得到 S', !!rustS, true);
  check('Rust back_out3 的 S', rustS ? rustS[1] : '缺失', '3.0');
  check(
    'Rust back_out3 的式子（u*u*((S+1)*u+S)+1）',
    !!(rustBody && /u\s*\*\s*u\s*\*\s*\(\s*\(S\s*\+\s*1\.0\)\s*\*\s*u\s*\+\s*S\s*\)\s*\+\s*1\.0/.test(rustBody[0])),
    true,
  );

  // JS: `return u * u * (4 * u + 3) + 1`（4 = S+1、3 = S 已折叠）
  const jsBody = js.match(/function backOut3[\s\S]{0,400}?\n\}/);
  check('JS 找得到 backOut3', !!jsBody, true);
  check(
    'JS backOut3 的式子（u*u*(4*u+3)+1 = 上面那条代入 S=3）',
    !!(jsBody && /u\s*\*\s*u\s*\*\s*\(\s*4\s*\*\s*u\s*\+\s*3\s*\)\s*\+\s*1/.test(jsBody[0])),
    true,
  );

  // ★ 逐值对拍：同一条式子取同样本点
  const rustEval = (t) => {
    const S = 3.0;
    const c = Math.min(1, Math.max(0, t));
    const u = c - 1.0;
    return u * u * ((S + 1.0) * u + S) + 1.0;
  };
  const jsEval = (t) => {
    const c = Math.min(1, Math.max(0, t));
    const u = c - 1;
    return u * u * (4 * u + 3) + 1;
  };
  for (const t of [0, 0.1, 0.25, 1 / 3, 0.5, 2 / 3, 0.75, 0.9, 1]) {
    check(`back_out3 逐值 t=${t}`, Math.abs(rustEval(t) - jsEval(t)) < 1e-12, true);
  }
  // ⚠️ 峰值必须是 1.25（`scale_in_peak` 是**端点不是峰值**，这条是那个坑的钉子）
  check('back_out3 峰值 = 1.25', Math.abs(rustEval(0.5) - 1.25) < 1e-12, true);
  check('back_out3 终点 = 1.0（⇒ 窗口外必须显式回 1.0）', Math.abs(rustEval(1) - 1) < 1e-12, true);
}

// ---------------------------------------------------------------------------
// ② T4 `text_shake_px`：**两个方向的频率必须不同**（相同就退化成对角直线）
// ---------------------------------------------------------------------------
{
  const rust = readDhampir('crates/dhampir-timeline/src/text_motion.rs');
  const js = read('webui/src/polish/engine.js');
  const jsBody = js.match(/function textShakePx[\s\S]{0,900}?\n\}/);
  check('JS 找得到 textShakePx', !!jsBody, true);
  const body = jsBody ? jsBody[0] : '';

  // 六个数：dx 的 0.62/2.7、dy 的 0.62/1.3/3.1、以及 0.38
  const nums = {
    '0.62（主项系数）': /0\.62/,
    '0.38（次项系数）': /0\.38/,
    'dx 次频 2.7×': /2\.7/,
    'dy 主频 1.3×': /1\.3/,
    'dy 次频 3.1×': /3\.1/,
    'dx 相位偏移 ×1.9': /ph\s*\*\s*1\.9|\*\s*1\.9/,
    'dy 相位偏移 ×0.6': /0\.6/,
    'dy 常数 1.1': /1\.1/,
    'dy 常数 2.3': /2\.3/,
  };
  for (const [what, re] of Object.entries(nums)) check(`JS shake 常数 ${what}`, re.test(body), true);

  // 与 Rust 对照同一组数
  for (const [what, re] of Object.entries({
    'Rust 0.62': /0\.62/,
    'Rust 0.38': /0\.38/,
    'Rust 2.7': /2\.7/,
    'Rust 1.3': /1\.3/,
    'Rust 3.1': /3\.1/,
  }))
    check(`text_motion.rs ${what}`, !!(rust && re.test(rust)), true);

  // ★ 三条"关"的分支都要显式挡（README 里专门写了为什么）
  check('JS shake 挡 px<=0', /!\s*\(shakePx\s*>\s*0\)/.test(body), true);
  check('JS shake 挡 hz<=0（0Hz 会变成"错位"不是"不抖"）', /!\s*\(shakeHz\s*>\s*0\)/.test(body), true);
  check('JS shake 挡 t 非有限', /Number\.isFinite\(tSec\)/.test(body), true);

  // ★ 逐值对拍（同一条式子两份实现）
  const ev = (t, px, hz, ph) => {
    const w = 2 * Math.PI * hz;
    const dx = px * (0.62 * Math.sin(w * t + ph) + 0.38 * Math.sin(w * 2.7 * t + ph * 1.9));
    const dy = px * (0.62 * Math.sin(w * 1.3 * t + ph + 1.1) + 0.38 * Math.sin(w * 3.1 * t + ph * 0.6 + 2.3));
    return [dx, dy];
  };
  // 幅度线性于 px（判据之一，写在 Rust 文档里）
  for (const t of [0, 0.01, 0.037, 0.1, 0.5, 1.234]) {
    const a = ev(t, 6, 18, 0), b = ev(t, 12, 18, 0);
    check(`shake 线性于 px（t=${t}）`, Math.abs(b[0] - 2 * a[0]) < 1e-9 && Math.abs(b[1] - 2 * a[1]) < 1e-9, true);
  }
  // 两个方向的频率不同 ⇒ dx 与 dy 的轨迹不重合（相同就退化成一条直线）
  const s = ev(0.03, 6, 18, 0);
  check('shake 两方向确实不同（非对角直线）', Math.abs(s[0] - s[1]) > 1e-6, true);

  // 是否 import 了本仓这份函数（判据要真的跑在被测对象上）
  const hasCall = /export function getElements[\s\S]*?textShakePx\(/.test(js) || /textShakePx\(\s*local/.test(js);
  check('getElements 里真的调了 textShakePx', hasCall, true);
}

// ---------------------------------------------------------------------------
// ③ T1/T6 的 **None 落点**必须与 Rust 逐值一致（缺省值写错 = 老工程观感变了）
// ---------------------------------------------------------------------------
{
  const rust = readDhampir('crates/dhampir-timeline/src/text_motion.rs');
  const js = read('webui/src/polish/engine.js');
  // ⚠ 窗口宽度跟着**注释长度**走 —— 它 2026-10-14 又一次撞上下限：
  //    往 textMotionCfg 里加了心跳三个键（带一段说明）后，这条拿不到函数体了
  //    ⇒ 报的是「找不到 textMotionCfg」，而真因只是“注释变长了”。同类坑已踩两次。
  const cfg = js.match(/function textMotionCfg[\s\S]{0,4000}?\n\}/);
  const body = cfg ? cfg[0] : '';
  check('JS 找得到 textMotionCfg', !!cfg, true);
  // ⚠️ scaleInMs 缺省 0（= 关）；shakePx 缺省 0；shakeHz 缺省 18；platePadPx 缺省 12
  check('textMotionCfg: scale_in_ms 缺省 0', /scaleInMs:\s*num\(e\.scale_in_ms,\s*0\)/.test(body), true);
  check('textMotionCfg: shake_px 缺省 0', /shakePx:\s*num\(e\.shake_px,\s*0\)/.test(body), true);
  check('textMotionCfg: shake_hz 缺省 18', /shakeHz:\s*num\(e\.shake_hz,\s*18(\.0)?\)/.test(body), true);
  check('textMotionCfg: plate_pad_px 缺省 12', /platePadPx:\s*num\(e\.plate_pad_px,\s*12(\.0)?\)/.test(body), true);
  check('Rust DEFAULT_SHAKE_HZ = 18.0', !!(rust && /DEFAULT_SHAKE_HZ[^=]*=\s*18\.0/.test(rust)), true);
  // ⚠️ T1 三个字段里**只有 scale_in_ms 决定开不开**
  check(
    'JS: 只有 scaleInMs>0 才弹（from/peak 只是端点）',
    /if\s*\(mo\s*&&\s*mo\.scaleInMs\s*>\s*0/.test(js),
    true,
  );
  // ⚠️ 窗口外必须显式回 1.0，不是停在 peak
  //
  // ⚠ 2026-10-14：上限从 300 放到 1400 —— 中间插了**心跳**那一支
  //   （它是独立的双峰函数，不走 `local < win` 那个窗口）。
  //   ⚠ 这条判据钉的是"**T1 那一支**窗口外回 1.0"，
  //     放宽上限不会让它失效（它仍然要求那个 `if` 真的存在）。
  check(
    'JS: 窗口外 scale 回 1.0（不是停在 peak）',
    /let motionScale = 1[\s\S]{0,1400}?if\s*\(local\s*<\s*win\)/.test(js),
    true,
  );
  // ★ 心跳那一支（双峰）也必须在——否则上面那条只要求 T1 在，
  //   "心跳被删了但 T1 还在"会**照样绿**。
  check(
    'JS: 心跳走独立的双峰函数（不是拿 T1 单峰冒充）',
    /heartbeatScaleAt\s*\(/.test(js) && /export function heartbeatScaleAt\s*\(/.test(js),
    true,
  );
}

// ---------------------------------------------------------------------------
// ④ T2 打字机：预览**不拆 cue**（求值期截断），出片**必须拆 cue** —— 两边口径同一句
// ---------------------------------------------------------------------------
{
  const js = read('webui/src/polish/engine.js');
  check('JS 里有 typewriter 分支', /mo\.typewriter/.test(js), true);
  check('JS: 按字数截断（slice）', /slice\(0,\s*Math\.max\(0,\s*n\)\)/.test(js), true);
  const tr = read('tools/polish-to-dhampir.mjs');
  check('转译器照抄了 typewriter 字段', /put\('typewriter'/.test(tr) || /typewriter/.test(tr), true);
}

if (process.argv.includes('--self-test')) {
  // 自证：判据本身不许空转 —— 故意改坏一份，要求它翻红
  const { strict: assert } = await import('node:assert');
  let self = 0;
  const js = read('webui/src/polish/engine.js');
  const broken = js.replace('u * u * (4 * u + 3) + 1', 'u * u * (5 * u + 3) + 1');
  assert.notEqual(broken, js, '自证：变异必须真的命中');
  self += 1;
  const body = broken.match(/function backOut3[\s\S]{0,400}?\n\}/);
  assert.equal(
    /u\s*\*\s*u\s*\*\s*\(\s*4\s*\*\s*u\s*\+\s*3\s*\)\s*\+\s*1/.test(body[0]),
    false,
    '自证：把 4 改成 5 之后，T1 判据必须翻红',
  );
  self += 1;
  const noCall = js.replace(/textShakePx\(\s*local/, 'textShakePxXX(local');
  assert.equal(noCall !== js, true, '自证：变异命中');
  assert.equal(/textShakePx\(\s*local/.test(noCall), false, '自证：拿掉调用之后判据必须翻红');
  self += 1;
  console.log(`✓ 文字动效对拍自证：${self} 项（改式子 / 拿掉调用，都必须红）`);
  process.exit(0);
}

if (failures.length) {
  console.error(`✗ 文字元素动效对拍：${failures.length} 项不一致（共 ${checks} 项）\n`);
  for (const f of failures) console.error('  - ' + f);
  console.error('\n  ⇒ 四份实现（elements.rs / text_motion.rs / 转译器 / engine.js）必须同口径，');
  console.error('    改一处就要改其余几处 —— 否则「预览弹得好好的、成片不弹」且不报错。');
  process.exit(1);
}
console.log(`✓ 文字元素动效四处同口径：${checks} 项（T1 back_out3 / T4 抖动 / 缺省落点 / T2 口径）`);
if (!readDhampir('crates/dhampir-timeline/src/text_motion.rs'))
  console.log('  ⚠ 未找到 ../Dhampir 检出 —— Rust 侧常数未对拍（只查了本仓一侧）');
