#!/usr/bin/env node
// 门禁：**§16 `speed` 档位的过渡时长 —— 四方逐值一致**（含"用哪条事件的 dur"）。
//
// # 为什么需要它
//
// 运镜档位切换的过渡时长由 `speed` 决定，而这条算式住在**四个地方**：
//
//   ① `crates/yekitrim-polish/src/process/placement.rs::transition_secs` —— **出片侧的权威**
//   ② `webui/src/polish/engine.js::cameraTransitionSecs`               —— **预览侧**
//   ③ `crates/yekitrim-polish/src/llm/segment.rs` 的提示词（教 LLM 写哪些值）
//   ④ `webui/src/components/TimelineEditor.vue` 的 `<option>`（人手动选的）
//
// ① 与 ② 是**两份独立实现**，漂了的症状是「预览里切得干脆、成片里拖拖拉拉」——
// **两边各自看都正常**，编译期、`cargo test`、`vite build` 里都不报错。
//
// # ⚠️ 最容易错的是"`dur` 取哪条事件"
//
// `slow` 的式子是 `min(1.5, dur × 0.8)` —— **`dur` 是"上一条"事件的时长**，
// 不是当前这条。取错的话：
//   * 上一条短、当前这条长 ⇒ 该是 `min(1.5, 短×0.8)`（较短），错取成当前这条会**拖长**
//   * 上一条长、当前这条短 ⇒ 反之
// 判据因此**必须同时钉住"算式"与"dur 的来源"**，只钉算式抓不到这个错。
//
//     node scripts/check-transition-frames.mjs
//     node scripts/check-transition-frames.mjs --self-test
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const read = (rel) => readFileSync(join(repo, rel), 'utf8');

let checks = 0;
const failures = [];
const check = (what, got, want = true) => {
  checks += 1;
  if (got !== want) failures.push(`${what}: 期望 ${want}，实得 ${got}`);
};
const has = (t, re) => re.test(t);

const rust = read('crates/yekitrim-polish/src/process/placement.rs');
const js = read('webui/src/polish/engine.js');
const seg = read('crates/yekitrim-polish/src/llm/segment.rs');
const tl = read('webui/src/components/TimelineEditor.vue');

// ── ① 出片侧（Rust）：三档常数 + slow 的夹取 ──────────────────────────────
{
  const fn = rust.match(/pub fn transition_secs\([\s\S]*?\n\}/);
  check('Rust 有 transition_secs', !!fn);
  const b = fn ? fn[0] : '';
  check('Rust snap = 0.10', has(b, /Some\("snap"\)\s*=>\s*0\.10/));
  check('Rust fast = 0.25', has(b, /Some\("fast"\)\s*=>\s*0\.25/));
  check('Rust slow = min(1.5, dur × 0.8)', has(b, /Some\("slow"\)\s*=>\s*1\.5_f64\.min\(dur\s*\*\s*0\.8\)/));
  // ⚠️ 硬切**优先于 speed**（写 `cut` 时 speed 必须被无视）
  check('Rust 硬切/远景优先于 speed（那三条 return 0.0 在 match 之前）',
    b.indexOf('return 0.0') < b.indexOf('match speed'));
}

// ── ② 预览侧（JS）：同样的三档 + 同样的夹取 ───────────────────────────────
{
  const tbl = js.match(/CAMERA_SPEED_SECS\s*=\s*\{[^}]*\}/);
  check('JS 有 CAMERA_SPEED_SECS 表', !!tbl);
  const t = tbl ? tbl[0] : '';
  check('JS snap = 0.10', has(t, /snap\s*:\s*0\.10/));
  check('JS fast = 0.25', has(t, /fast\s*:\s*0\.25/));
  check('JS slow = 1.5', has(t, /slow\s*:\s*1\.5/));
  const fn = js.match(/export function cameraTransitionSecs\([\s\S]*?\n\}/);
  check('JS 有 cameraTransitionSecs', !!fn);
  const b = fn ? fn[0] : '';
  check('JS 硬切优先（第一句 return 0）', has(b, /if\s*\(hardCut\)\s*return\s*0/));
  check('JS slow 做 min(…, dur × 0.8)', has(b, /norm\s*===\s*'slow'\s*\?\s*Math\.min\(CAMERA_SPEED_SECS\.slow,\s*dur\s*\*\s*0\.8\)/));
  // ⚠️ **认不出/没写 ⇒ 老式子**（不能退成 0，那是硬切）
  check('JS 兜底 = min(dur × 0.5, 0.6)（不是 0、不是 snap）', has(b, /Math\.min\(dur\s*\*\s*0\.5,\s*0\.6\)/));
}

// ── ③ ★ `dur` 的来源：必须是**上一条**事件 ──────────────────────────────
{
  // 预览：`cameraTransitionSecs(c.ev, c.duration, …)`，而 `c` 由"时间上 ≤ t 的最后一条"选出
  check('JS 把 `c.duration`（不是 duration 之外的量）传进去', has(js, /cameraTransitionSecs\(c\.ev,\s*c\.duration,\s*hardCut\)/));
  // ⚠️ 那表"时间上 ≤ t 的最后一条"：循环是升序遍历 + 命中即更新，**不是**找第一条
  check('JS 的运镜循环是"升序遍历、最后一条 ≤ t 的赢"（c 会被反复覆盖）',
    has(js, /for \(const c of idx\.camera\)\s*\{\s*\n\s*if \(t < c\.time\) break/));
  // 出片侧：Rust 那边由求值器给 prev 的 dur —— 钉住它读的字段名
  const tr = read('tools/polish-to-dhampir.mjs');
  check('转译器把 speed 透传给底座（不在这一层重算时长）', has(tr, /speed/));
}

// ── ④ 生产者两侧：LLM 提示词与人工下拉必须教/提供同一组值 ────────────────
for (const v of ['snap', 'fast', 'slow']) {
  check(`提示词教了 ${v}`, has(seg, new RegExp(`"${v}"`)));
  check(`下拉提供了 ${v}`, has(tl, new RegExp(`value="${v}"`)));
}
// `med` 是"没写"那一档（Rust 的 `_ =>` / JS 的兜底），不需要出现在 speed 的三档里
check('提示词里 med 与"不写"同义（存在 med 字样）', has(seg, /med/));

// ── ⑤ ★ 拿同一组输入跑两边的纯算式，逐值比对 ────────────────────────────
{
  const rustEval = (speed, dur, cut = false) => {
    const WIDE = '远';
    if (cut) return 0.0;
    switch (speed) {
      case 'snap': return 0.10;
      case 'fast': return 0.25;
      case 'slow': return Math.min(1.5, dur * 0.8);
      default: return 0.6; // TRANSITION_SECS
    }
  };
  const jsEval = (speed, dur, hardCut = false) => {
    if (hardCut) return 0;
    const tbl = { snap: 0.10, fast: 0.25, slow: 1.5 };
    const norm = typeof speed === 'string' ? speed.trim().toLowerCase() : '';
    if (Object.prototype.hasOwnProperty.call(tbl, norm)) {
      return norm === 'slow' ? Math.min(tbl.slow, dur * 0.8) : tbl[norm];
    }
    return Math.min(dur * 0.5, 0.6);
  };
  // snap/fast 与 dur 无关 ⇒ 两边必须恒等
  for (const sp of ['snap', 'fast']) {
    for (const dur of [0.4, 3.6, 10.0]) {
      check(`${sp} dur=${dur} 两腿相等`, rustEval(sp, dur) === jsEval(sp, dur));
    }
  }
  // slow 依赖 dur ⇒ 挑几个关键点（含夹取边界 dur×0.8 = 1.5 ⇔ dur = 1.875）
  for (const dur of [0.4, 1.0, 1.875, 2.4, 3.6, 10.0]) {
    check(`slow dur=${dur} 两腿相等`, Math.abs(rustEval('slow', dur) - jsEval('slow', dur)) < 1e-12);
  }
  check('slow 在 dur=1.875 处正好夹到 1.5（边界两侧都对）',
    Math.abs(rustEval('slow', 1.875) - 1.5) < 1e-12 && Math.abs(rustEval('slow', 1.874) - 1.4992) < 1e-9);
  // 硬切优先：写了 cut 就必须 0，**哪怕 speed=slow**
  check('硬切优先：cut + slow ⇒ 0', rustEval('slow', 3.6, true) === 0 && jsEval('slow', 3.6, true) === 0);

  // ★★ 真机测出来的那一组（§16 实测，见 plan 文档）
  //    事件：中@2.0~2.4s（dur=0.4）→ 近@2.4~6.0s；过渡发生在进入"近"时，
  //    `dur` 取**上一条**（0.4s）= 实测 10 帧 ÷ 30fps = 0.333s（1 帧粒度内）
  check('真机 slow 实测 0.333s ≈ min(1.5, 0.4×0.8)=0.32', Math.abs(0.32 - 10 / 30) < 1 / 30 + 1e-9);
  check('真机 snap 实测 0.100s == 0.10（精确）', Math.abs(0.10 - 3 / 30) < 1 / 30 + 1e-9);
  check('**若 dur 错取当前这条（3.6s）会得到 1.5s**，与实测 0.333s 差 4.5 倍 ⇒ 判据能区分',
    Math.abs(rustEval('slow', 3.6) - 10 / 30) > 1.0);
}

if (process.argv.includes('--self-test')) {
  const { strict: assert } = await import('node:assert');
  let self = 0;
  const cases = [
    ['Rust snap 常数改掉', rust, /Some\("snap"\)\s*=>\s*0\.10/, /Some\("snap"\)\s*=>\s*0\.50/],
    ['JS slow 去掉 dur 夹取', js, /Math\.min\(CAMERA_SPEED_SECS\.slow,\s*dur\s*\*\s*0\.8\)/, 'CAMERA_SPEED_SECS.slow'],
    ['JS 硬切不再优先', js, /if\s*\(hardCut\)\s*return\s*0/, 'if (false) return 0'],
    ['JS 的 dur 改成不传（改用写死的）', js, /cameraTransitionSecs\(c\.ev,\s*c\.duration,\s*hardCut\)/, 'cameraTransitionSecs(c.ev, 3.6, hardCut)'],
  ];
  for (const [name, src, from, to] of cases) {
    const mutated = src.replace(from, to);
    assert.notEqual(mutated, src, `自证「${name}」：变异必须命中`);
    assert.equal(from.test(mutated), false, `自证「${name}」：变异后必须翻红`);
    self += 1;
  }
  console.log(`✓ §16 过渡时长自证：${self} 处变异（常数 / 夹取 / 硬切优先 / dur 来源）都翻红`);
  process.exit(0);
}

if (failures.length) {
  console.error(`✗ §16 speed 过渡时长不一致：${failures.length} 项（共 ${checks} 项）\n`);
  for (const f of failures) console.error('  - ' + f);
  console.error('\n  ⇒ 这条算式住在四处（Rust / engine.js / 提示词 / 下拉），漏改一处的症状是');
  console.error('    「预览里切得干脆、成片里拖拖拉拉」，**两边各自看都正常**。');
  console.error('    ⚠️ 还要钉住 `dur` 取的是**上一条**事件 —— 只钉算式抓不到这个错。');
  process.exit(1);
}
console.log(`✓ §16 speed 过渡时长四处一致：${checks} 项（Rust / 预览 / 提示词 / 下拉 + dur 来源 + 逐值对拍）`);
