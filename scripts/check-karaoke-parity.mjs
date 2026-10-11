#!/usr/bin/env node
// 门禁：**T5 真卡拉OK —— 预览腿与出片腿逐值对拍**。
//
// # 为什么需要它
//
// 逐字卡拉OK的"每个字落在哪一刻"有**两份独立实现**：
//
//   ① `tools/polish-to-dhampir.mjs::splitKaraoke`  —— **出片腿**
//      它把一条字幕切成 N 条 cue（底座没有逐字通道，只能这样表达）
//   ② `webui/src/polish/engine.js::karaokeSplitCount` —— **预览腿**
//      它每帧算一个"该显示到第几个字"的长度
//
// 两边**产物形态不同**（cue 列表 vs 截断长度），但**每个字的时刻必须逐值相等**。
// 漂了的症状是「预览里字在长、成片里是整条」（或反过来）——
// **两边各自看都正常**，编译期、`cargo test`、`vite build` 里**都不报错**。
//
// # 判据
//
// 零依赖，把两边的**守卫与算式**抽出来逐条对：
//   * 三道守卫（字数对不上 / 非单调 / 严格递增）必须两边都有；
//   * `at[0] = 起点`、`at[i] >= at[i-1] + 1ms`、`顶出末尾 ⇒ 整条退回` 三条算式必须同口径；
//   * 拿同一组输入跑两边的**纯算法**，逐值比对。
//
//     node scripts/check-karaoke-parity.mjs
//     node scripts/check-karaoke-parity.mjs --self-test
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

const tr = read('tools/polish-to-dhampir.mjs');
const js = read('webui/src/polish/engine.js');

// ── ① 出片腿：`splitKaraoke` 的存在与三道守卫 ─────────────────────────────
{
  const fn = tr.match(/function splitKaraoke\([\s\S]*?\n  \}/);
  check('出片腿有 splitKaraoke', !!fn);
  const b = fn ? fn[0] : '';
  check('出片腿守卫①：字数 ≤1 不切', has(b, /chars\.length\s*<=\s*1\)\s*return null/));
  check('出片腿守卫①b：times 长度必须等于字数', has(b, /times\.length\s*!==\s*chars\.length\)\s*return null/));
  check('出片腿守卫②：非单调不切', has(b, /t\s*<\s*times\[i\s*-\s*1\]\)\s*return null/));
  check('出片腿守卫③：每字**严格递增**（prev + 1）',
    has(b, /Math\.max\(ms,\s*prev\s*\+\s*1\)/));
  check('出片腿：第一个字从片段起点开始（at[0] = startMs）', has(b, /at\[0\]\s*=\s*startMs/));
  check('出片腿：顶出末尾 ⇒ 整条退回', has(b, /at\[at\.length\s*-\s*1\]\s*>\s*endMs\)\s*return null/));
}

// ── ② 预览腿：同样的守卫 ─────────────────────────────────────────────────
{
  const fn = js.match(/export function karaokeSplitCount\([\s\S]*?\n\}/);
  check('预览腿有 karaokeSplitCount', !!fn);
  const b = fn ? fn[0] : '';
  check('预览腿守卫①：字数 ≤1 不切', has(b, /chars\.length\s*<=\s*1\)\s*return null/));
  check('预览腿守卫①b：times 长度必须等于字数', has(b, /times\.length\s*!==\s*chars\.length\)\s*return null/));
  check('预览腿守卫②：非单调不切', has(b, /ts\s*<\s*times\[i\s*-\s*1\]\)\s*return null/));
  check('预览腿守卫③：每字**严格递增**', has(b, /Math\.max\(ms,\s*prev\s*\+/));
  // ⚠️⚠️ **量纲必须与出片腿相同（都是 1 毫秒）** —— 这条是本判据**第一次跑就抓出来的真分叉**：
  //    预览原先写 `prev + 1e-6`（1 微秒）。**逐值对拍看不出差别**（两边都递增、常规样本全过），
  //    但"多字同一时刻 + 可用区间窄"时：出片腿因"顶出末尾"整条退回，预览腿却塞得下
  //    ⇒ **预览里逐字在长、成片里是整条**。所以必须**按源码文本**钉死这个常数，
  //    不能只靠下面那段"拿同输入跑两遍" —— 那段跑的是本文件里手抄的一份，改不到源码。
  check('预览腿严格递增的量纲 = 1 毫秒（与出片腿的 prev+1 同量纲）',
    has(b, /Math\.max\(ms,\s*prev\s*\+\s*0\.001\)/));
  // ⚠️ 负向判据必须**先剥掉注释**再匹配 —— 上面那段解释里就写着 `prev + 1e-6`（作为反例），
  //    不剥的话这条会**恒红**。这正是"grep 会双向骗人"的那种坑：
  //    我第一版就是这么写的，结果被自己的注释判红（而代码是对的）。
  const bodyNoComment = b.replace(/\/\/[^\n]*/g, '');
  check('预览腿**不许**用 1 微秒那种"看着也递增"的量纲', !has(bodyNoComment, /prev\s*\+\s*1e-6/));
  check('预览腿：第一个字从起点开始（at[0] = subStart）', has(b, /at\[0\]\s*=\s*subStart/));
  check('预览腿：顶出末尾 ⇒ 整条退回', has(b, /at\[at\.length\s*-\s*1\]\s*>\s*subEnd\)\s*return null/));
  // 截断长度那条也要在
  check('预览腿有 karaokeShownChars', has(js, /export function karaokeShownChars\(/));
}

// ── ③ 接线：预览真的用它（不是写了两个没人调的函数）──────────────────────
{
  check('buildIndex 收 karaokeCues', has(js, /buildIndex\(\{[^}]*karaokeCues/));
  // ⚠️ 只在开关真的开着时才吃 —— R4 的保证点
  check('只在 elements.karaoke === true 时建索引',
    has(js, /cfg\.elements\.karaoke\s*===\s*true\s*&&\s*Array\.isArray\(karaokeCues\)/));
  check('按**起点毫秒**索引（不是数组下标）', has(js, /Math\.round\(Number\(c\.start\)\s*\*\s*1000\)/));
  check('subsAt 里真的调了 karaokeSplitCount', has(js, /karaokeSplitCount\(s,\s*times,\s*s\.start,\s*s\.end\)/));
  // 出片腿同样按起点毫秒索引
  check('出片腿也按起点毫秒索引', has(tr, /byStart\.set\(Math\.round\(c\.start\s*\*\s*1000\)/));
}

// ── ④ ★ 拿同一组输入跑两边的纯算法，逐值比对 ────────────────────────────
{
  // 把两边的算式**按源码原文重写**（不 import：本仓判据零依赖的纪律）。
  // 出片腿：字幕起点 500ms、终点 3000ms；times 与它自己的口径一致（秒）
  const splitDhampir = (chars, times, subStartMs, subEndMs, pieceStartMs, pieceEndMs) => {
    const startMs = pieceStartMs, endMs = pieceEndMs;
    if (chars.length <= 1) return null;
    if (times.length !== chars.length) return null;
    if (!(endMs > startMs)) return null;
    const spanMs = subEndMs - subStartMs;
    if (!(spanMs > 0)) return null;
    const at = []; let prev = startMs;
    for (let i = 0; i < chars.length; i += 1) {
      const t = times[i];
      if (!Number.isFinite(t)) return null;
      if (i > 0 && t < times[i - 1]) return null;
      const rel = (t * 1000 - subStartMs) / spanMs;
      let ms = Math.round(startMs + rel * (endMs - startMs));
      ms = Math.max(ms, prev + 1);
      at.push(ms); prev = ms;
    }
    at[0] = startMs;
    if (at[at.length - 1] > endMs) return null;
    return at;
  };
  // 预览腿：单位统一用**秒**（预览本来就以秒为时间轴）
  const splitPreview = (chars, times, subStart, subEnd) => {
    if (chars.length <= 1) return null;
    if (times.length !== chars.length) return null;
    const span = subEnd - subStart;
    if (!(span > 0)) return null;
    const at = []; let prev = subStart;
    for (let i = 0; i < chars.length; i += 1) {
      const ts = times[i];
      if (!Number.isFinite(ts)) return null;
      if (i > 0 && ts < times[i - 1]) return null;
      const rel = (ts - subStart) / span;
      let ms = subStart + rel * span;
      ms = Math.max(ms, prev + 0.001);
      at.push(ms); prev = ms;
    }
    at[0] = subStart;
    if (at[at.length - 1] > subEnd) return null;
    return at;
  };

  // 样本：字幕 0.5~3.0s，6 个字每 0.3s 一个（与真机 T5 探针同形）
  const chars = Array.from('今天天气真好');
  const times = [0.5, 0.8, 1.1, 1.4, 1.7, 2.0];

  // ① 整条字幕 = 整段（片段不裁切）：两边必须**逐值相等**（换到毫秒/秒同一尺度）
  const viaDhampir = splitDhampir(chars, times, 500, 3000, 500, 3000);
  const viaPreview = splitPreview(chars, times, 0.5, 3.0);
  check('两边都算出了时刻表', !!viaDhampir && !!viaPreview);
  if (viaDhampir && viaPreview) {
    check('时刻表长度相等', viaDhampir.length === viaPreview.length);
    for (let i = 0; i < Math.min(viaDhampir.length, viaPreview.length); i += 1) {
      const d = viaDhampir[i], p = viaPreview[i] * 1000;
      check(`第 ${i} 个字时刻一致（出片 ${d}ms vs 预览 ${Math.round(p)}ms）`, Math.abs(d - p) < 1.5);
    }
    // ⚠️ 第一个字必须落在字幕起点（不是 times[0]）—— 两边同一条
    check('两边都把第一个字拉到起点', viaDhampir[0] === 500 && Math.abs(viaPreview[0] - 0.5) < 1e-9);
  }

  // ② 守卫对拍：非单调 ⇒ 两边都退回
  const bad = [0.5, 0.8, 0.6, 1.4, 1.7, 2.0];
  check('非单调：出片腿退回', splitDhampir(chars, bad, 500, 3000, 500, 3000) === null);
  check('非单调：预览腿退回', splitPreview(chars, bad, 0.5, 3.0) === null);

  // ③ 守卫对拍：字数对不上 ⇒ 两边都退回
  check('长度不符：出片腿退回', splitDhampir(chars, [0.5, 0.8], 500, 3000, 500, 3000) === null);
  check('长度不符：预览腿退回', splitPreview(chars, [0.5, 0.8], 0.5, 3.0) === null);

  // ④ 守卫对拍：**6 个字挤在同一毫秒** ⇒ 严格递增兜底之后顶出末尾 ⇒ 两边都退回。
  //    ⚠️ 这条正是"`prev + 1` 与 `Math.max(ms, prev)` 之差"的判据：
  //    若写成 `Math.max(ms, prev)`，6 个字会全挤在 0.5s，**两边都算得出**（不该退回）。
  const same = [0.5, 0.5, 0.5, 0.5, 0.5, 0.5];
  // 可用区间只有 2ms ⇒ 严格递增 6 个字需要 ≥5ms ⇒ 顶出
  const tightD = splitDhampir(chars, same, 500, 3000, 500, 502);
  const tightP = splitPreview(chars, same, 0.5, 0.502);
  check('六字同刻+窄区间：出片腿退回（严格递增兜底生效）', tightD === null);
  check('六字同刻+窄区间：预览腿退回（严格递增兜底生效）', tightP === null);
}

if (process.argv.includes('--self-test')) {
  const { strict: assert } = await import('node:assert');
  let self = 0;
  // ① 把预览腿的量纲从 1 毫秒退化成 1 微秒 ⇒ 判据必须红。
  //    ⚠️ 刻意**不是**"退化成不递增"那种一眼就错的变异 —— 抓它没有价值。
  //    真正会溜过去的是"看着也递增、但门槛与出片腿不同"这种，本题第一次跑就是它。
  const weakened = js.replace(/Math\.max\(ms,\s*prev\s*\+\s*0\.001\)/, 'Math.max(ms, prev + 1e-6)');
  assert.notEqual(weakened, js, '自证：变异必须命中');
  const fn = weakened.match(/export function karaokeSplitCount\([\s\S]*?\n\}/);
  assert.equal(
    /Math\.max\(ms,\s*prev\s*\+\s*0\.001\)/.test(fn[0]),
    false,
    '自证①：量纲退化成 1 微秒后必须红',
  );
  assert.equal(/prev\s*\+\s*1e-6/.test(fn[0]), true, '自证①b：变异确实落在量纲上');
  self += 1;
  // ② 拿掉预览腿的"顶出末尾 ⇒ 退回"
  const noGuard = js.replace(/if \(at\[at\.length - 1\] > subEnd\) return null/, '');
  assert.notEqual(noGuard, js, '自证：变异必须命中');
  assert.equal(/at\[at\.length\s*-\s*1\]\s*>\s*subEnd\)\s*return null/.test(noGuard), false, '自证②：拿掉守卫后必须红');
  self += 1;
  // ③ 把"只在开关开着时建索引"改成无条件 ⇒ 判据必须红（这是 R4 的保证点）
  const always = js.replace(/cfg\.elements\.karaoke\s*===\s*true\s*&&\s*Array\.isArray\(karaokeCues\)/, 'true');
  assert.notEqual(always, js, '自证：变异必须命中');
  assert.equal(/cfg\.elements\.karaoke\s*===\s*true\s*&&\s*Array\.isArray\(karaokeCues\)/.test(always), false, '自证③：无条件索引后必须红');
  self += 1;
  console.log(`✓ 卡拉OK两腿对拍自证：${self} 项（弱化递增 / 拿掉守卫 / 无条件索引，都必须红）`);
  process.exit(0);
}

if (failures.length) {
  console.error(`✗ T5 卡拉OK 两腿不一致：${failures.length} 项（共 ${checks} 项）\n`);
  for (const f of failures) console.error('  - ' + f);
  console.error('\n  ⇒ 出片腿切 cue、预览腿算截断长度 —— 形态不同但**每字的时刻必须相等**。');
  console.error('    漂了的症状是「预览里字在长、成片里是整条」或反过来，**两边各自看都正常**。');
  process.exit(1);
}
console.log(`✓ T5 卡拉OK 两腿逐值对拍：${checks} 项（守卫 / 算式 / 接线 / 同输入比对）`);
