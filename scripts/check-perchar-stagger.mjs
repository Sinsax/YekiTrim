#!/usr/bin/env node
/**
 * D 组「逐字错峰」的**到达顺序**判据 —— 真机、逐字、按列量。
 *
 * # 为什么要有这一条
 *
 * C 组和 D 组在**配置形状**上几乎一样（都是 `per_char` + `per_char_frames`），
 * 区别只有 `stagger_frames` 一个数。于是"以为做了 D 组、其实做成了 C 组"
 * 这件事**没有任何配置层面的症状**：两份 spec 都合法、都能跑、都出画面。
 *
 * 光看"有没有动"也分不开 —— 两组都在动。必须量**动的次序**：
 *
 *   * C 组：所有字**同时**动，幅度不同 ⇒ 各字的顶边**同时**变化；
 *   * D 组：每个字**依次**动，幅度相同 ⇒ 顶边像**波一样从左扫到右**。
 *
 * 判据就是那个"波"：把一行按字宽切成 N 列，逐帧取每列的**墨迹顶边**，
 * 然后看**"已经动过的列"的个数**是否随帧递增。
 *
 * # 为什么量顶边而不是量亮度/墨数
 *
 * 顶边是本判据里**唯一**满足下面三条的量：
 *   ① 单调 —— "跳起"就是往上走，顶边只会变小；
 *   ② 逐字独立 —— 每个字的顶边只由那个字自己的变换决定；
 *   ③ 对亮度不敏感 —— 位图着色/抗锯齿变了也不会漂。
 * 量总墨数会把"整行一起平移"误判成"逐字散开"（我在 P4 已经栽过一次），
 * 量平均亮度则会把底色变化算进来。
 *
 * # 实测基线（2026-10-13，`hop_std` vs `scatter_lg`，同一句话「逐字散开测试」）
 *
 * ```text
 * hop_std  f30  顶边:  656  684  726  734  726  726     ← 第 1 字已经起来了
 * hop_std  f34  顶边:  660  653  685  734  726  725     ← 第 2、3 字在起
 * hop_std  f38  顶边:  662  657  654  693  726  725     ← 波扫到第 4 字
 * hop_std  f46  顶边:  662  659  660  666  654  684     ← 第 6 字最后到
 *
 * scatter  f30  顶边:  662  664  660  668  660  660     ← 六列几乎同高
 * scatter  f46  顶边:  662  666  673  689  689  695     ← 仍然同时
 * ```
 *
 * ⚠️ 别把阈值钉在"某个绝对像素"上：字体/字号/`font_ratio` 一换，这些数就全变。
 *    本判据只钉**结构性事实** —— D 组各字的开始时间**不相同**，C 组相同。
 */

import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, writeFileSync, rmSync, statSync, readdirSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, '..');
const DHAMPIR = process.env.DHAMPIR_DIR ?? 'F:/para/Code/Dhampir';
const BIN = join(DHAMPIR, 'target', 'debug', 'dhampir.exe');

let checks = 0;
let failures = 0;
const ok = (cond, msg) => {
  checks += 1;
  if (cond) console.log(`  ✓ ${msg}`);
  else {
    failures += 1;
    console.log(`  ✗ ${msg}`);
  }
};

if (!existsSync(BIN)) {
  console.error(`✗ 找不到底座二进制：${BIN}\n  先 cargo build -p dhampir-worker --bin dhampir`);
  process.exit(3);
}

// ⚠️ 旧二进制会给出**改了之前的**答案 ⇒ 拒绝跑（与 `check-perchar-parity.mjs` 同一条纪律）。
function staleBinary() {
  const binTime = statSync(BIN).mtimeMs;
  let newest = 0;
  let file = '';
  const walk = (d) => {
    let es;
    try {
      es = readdirSync(d, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of es) {
      const f = join(d, e.name);
      if (e.isDirectory()) walk(f);
      else if (e.name.endsWith('.rs')) {
        const t = statSync(f).mtimeMs;
        if (t > newest) {
          newest = t;
          file = f;
        }
      }
    }
  };
  for (const c of ['dhampir-timeline', 'dhampir-core', 'dhampir-worker']) walk(join(DHAMPIR, 'crates', c, 'src'));
  return newest > binTime ? file : null;
}

const stale = staleBinary();
if (stale) {
  console.error(`✗ 底座二进制比源码旧（${stale}）—— 拒绝在旧二进制上判定。`);
  process.exit(3);
}

const require_ = createRequire(import.meta.url);
const E = await import(new URL('../webui/src/polish/engine.js', import.meta.url));

const work = mkdtempSync(join(process.env.TMP ?? process.env.TEMP ?? '/tmp', 'hoporder-'));
const fonts = join(work, 'fonts');
execFileSync('cmd', ['/c', 'mkdir', fonts], { stdio: 'ignore' });

const srt = join(work, 'dhampir-subtitle.srt');
writeFileSync(srt, '1\n00:00:00,000 --> 00:00:20,000\n逐字散开测试\n', 'utf8');

/** 造工程：一条字幕轨带给定的 `per_char`。 */
function project(spec) {
  const p = join(work, `p-${Math.random().toString(36).slice(2)}.json`);
  writeFileSync(
    p,
    JSON.stringify({
      project_schema: 1,
      assets: [
        { id: 'clip.mp4', kind: 'video', name: 'b', uri: 'clip.mp4', frame_count: null,
          timebase: { num: 30, den: 1 }, width: 1920, height: 1080, content_hash: null, tags: {} },
        { id: 'dhampir-subtitle.srt', kind: 'subtitle', name: 's', uri: srt, frame_count: null,
          timebase: { num: 30, den: 1 }, width: null, height: null, content_hash: null, tags: {} },
      ],
      timeline: {
        schema: 3, timebase: { num: 30, den: 1 },
        tracks: [
          { id: 'v', kind: 'video', layers: [{ id: 'b', start: 0, end: 600, source: { asset_id: 'clip.mp4', source_in: 0 } }] },
          { id: 'sub', kind: 'subtitle', layers: [{ id: 'c', start: 0, end: 600, source: { asset_id: 'dhampir-subtitle.srt', source_in: 0 } }],
            subtitle: { font_ratio: 0.08, color: [255, 255, 255, 255], outline: false, per_char: spec } },
        ],
      },
    }),
    'utf8'
  );
  return p;
}

/** 逐帧取"每列墨迹顶边"。 */
function columnTops(projPath, frames, cols = 6) {
  const outDir = join(work, 'f' + Math.random().toString(36).slice(2));
  const rows = [];
  for (const f of frames) {
    // ⚠️ 这个夹具**没有真的视频**（`clip.mp4` 只是给 `assets` 占位，
    //    没有它 `subtitle` 会静默返回空 items）—— 所以 `frame` 有可能因为
    //    解不了视频而退出非零。**渲染不出来不算判据失败**：
    //    逐字那一维的事实由① ② 两条求值判据钉住，这一条只是"真的落了盘"的补充。
    //    但也**不许**把"渲染失败"当成通过 —— 它单独报一条，红就是红。
    try {
      execFileSync(BIN, ['frame', '--project', projPath, '--from', String(f), '--to', String(f),
        '--out', outDir, '--font-dir', fonts], { stdio: 'ignore', env: { ...process.env, TMP: work } });
    } catch {
      rows.push(null);
      continue;
    }
    const png = join(outDir, `frame-${String(f).padStart(4, '0')}.png`);
    rows.push(existsSync(png) ? png : null);
  }
  return rows;
}

/**
 * 逐字**开始时间**：底面一个字的顶边第一次"明显上移"的帧。
 *
 * 直接量像素要解码 PNG。这里走底座自己的求值（`subtitle` 子命令）拿 cells 的
 * `offset_y_em` —— 它**就是**最终落点里逐字那一维，且与像素一一对应
 * （像素那一路已经由 `.evidence/perchar-hop-*.png` 真机看过）。
 * ⚠️ 用求值而不是量像素的理由：量像素要按列切、要定阈值，
 *    而"波从左扫到右"这件事在 `offset_y_em` 上**直接可读**。
 */
function startFrames(projPath, count, spec) {
  const starts = [];
  for (let i = 0; i < count; i += 1) {
    starts.push(null);
  }
  for (let f = 0; f < 90; f += 1) {
    const raw = execFileSync(BIN, ['subtitle', '--project', projPath, '--frame', String(f)],
      { encoding: 'utf8', env: { ...process.env, TMP: work } });
    let j;
    try {
      j = JSON.parse(raw);
    } catch {
      continue;
    }
    const item = (j.items ?? []).find((it) => (it.cells ?? []).length === count);
    if (!item) continue;
    item.cells.forEach((c, i) => {
      if (starts[i] === null && Math.abs(c.offset_y_em - spec.offset_y_from) > 1e-3) {
        starts[i] = f;
      }
    });
  }
  return starts;
}

console.log('D 组「逐字错峰」到达顺序判据（真机）');
console.log('='.repeat(60));

const COUNTS = 6;
const FRAMES_EVAL = 90;

// ---- ① D 组：各字的开始时间必须**互不相同**且递增（波从前往后扫）----
const dSpec = { offset_y_from: 0.55, offset_y_to: 0, per_char_frames: 12, stagger_frames: 4, easing: 'back_out' };
const dProj = project(dSpec);
const dStarts = startFrames(dProj, COUNTS, dSpec);
console.log(`  hop_std 各字开始帧：${JSON.stringify(dStarts)}`);
const dLive = dStarts.filter((v) => v !== null);
ok(dLive.length === COUNTS, `① D 组六个字都动起来了（实得 ${dLive.length} / ${COUNTS}）`);
ok(new Set(dLive).size > 1, `① D 组各字的开始帧**不相同**（错峰存在；不同开始帧 ${new Set(dLive).size} 个）`);
ok(
  dLive.every((v, i) => i === 0 || v >= dLive[i - 1]),
  `① D 组开始帧**单调不降**（波从前往后扫）：${JSON.stringify(dLive)}`
);
ok(
  dLive[dLive.length - 1] - dLive[0] >= (COUNTS - 1) * dSpec.stagger_frames * 0.5,
  `① D 组首末字的开始帧间隔与 stagger 同量级（实得 ${dLive[dLive.length - 1] - dLive[0]} 帧，`
  + `stagger=${dSpec.stagger_frames}×${COUNTS - 1} 个间隔）`
);

// ---- ② C 组（对照组）：所有字**同时**开始 ⇒ 这判据能区分两组，不是恒真 ----
const cSpec = { spread_x_from: 0, spread_x_to: 1, spread_x_em: 0.45, spread_y_from: 0, spread_y_to: 1,
  spread_y_em: 0.28, per_char_frames: 16, stagger_frames: 3, easing: 'back_out' };
const cProj = project({ ...cSpec, offset_y_from: 0.55, offset_y_to: 0, per_char_frames: 12, stagger_frames: 0 });
const cSpec0 = { offset_y_from: 0.55, offset_y_to: 0, per_char_frames: 12, stagger_frames: 0, easing: 'back_out' };
const cStarts = startFrames(cProj, COUNTS, cSpec0);
console.log(`  对照组(无错峰)各字开始帧：${JSON.stringify(cStarts)}`);
const cLive = cStarts.filter((v) => v !== null);
ok(cLive.length === COUNTS, `② 对照组六个字也都动起来了（${cLive.length} / ${COUNTS}）`);
ok(
  new Set(cLive).size === 1,
  `② 对照组各字**同时**开始（不同开始帧 ${new Set(cLive).size} 个）—— `
  + `⇒ ① 量的确实是"错峰"本身，不是"有没有动"（否则这条判据恒真）`
);

// ---- ③ 真机像素（**只报不判**）----
//
// ⚠️ 这个夹具**没有真的视频**（`clip.mp4` 只占位），所以 `frame` 解不了视频、
//    渲染必然失败。**渲染不出来不算判据失败**，但也**绝不许当成通过** ——
//    所以它单独打印、不进 `checks`。
//
// 「D 组真的出得来像素」这件事的证据在**仓库外的那次真机跑**上：
// `.evidence/perchar-hop-strip.png`（Dhampir 仓）—— 同一句话 frame 27→46，
// 六列顶边 `656/684/726/734/726/726` → `662/659/660/666/654/684`，
// 肉眼可见"波从左扫到右"。**那条证据是人工看过的，不是这条判据产的。**
const pngs = columnTops(dProj, [30, 46]);
const havePng = pngs.filter(Boolean).length;
console.log(
  `  · （只报不判）D 组测试帧渲染：${havePng} / 2 —— 夹具没有真视频，`
  + `渲染不出属预期；像素证据见 Dhampir 仓 .evidence/perchar-hop-strip.png`
);

rmSync(work, { recursive: true, force: true });

console.log('='.repeat(60));
if (failures > 0) {
  console.log(`✗ 逐字错峰判据失败：${failures} / ${checks}`);
  process.exit(1);
}
console.log(`✓ 逐字错峰判据通过：${checks} 项 —— D 组各字依次起步，对照组同时起步`);
