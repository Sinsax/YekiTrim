#!/usr/bin/env node
// **成片文字质量**：渲染是无损的，成片要过一遍 H.264 —— 这条判据量「编码到底毁了多少」。
//
// # 为什么要有它
//
// 2026-10-18 用户报「成片文字锯齿感强烈」。查下来渲染侧**已经是 1px 抗锯齿**
// （描边外缘扫描线 3~4px 平滑过渡），差距全在**编码**：同一帧 lossless PNG ↔ 成片解码
// 裁剪区 MAE 3.83 / max 51，损伤全在字形边缘。当时靠手工对比定位，做完就没了 —— 这条把它变成可复跑的。
//
// # 判据（三项）
//
//   MAE（0..255） · max · 「差值 > 40 的像素占比」
//
// 阈值见 THRESHOLDS。**调编码参数前后各跑一次**，数字必须下降而不是「看着还行」。
//
// # 用法
//
//   node scripts/check-export-text-quality.mjs --project <片段目录> [--frame 700] [--box x0,y0,x1,y1]
//   node scripts/check-export-text-quality.mjs --self-test
//
// ⚠️ 需要：底座 exe（target/debug/dhampir/bin/dhampir.exe）+ ffmpeg + ffprobe。
//    缺任何一个**明说 SKIP**（退出码 0），绝不静默算过。
// ⚠️ --box 是「只看文字那一块」：整帧的 MAE 会被背景主导（实测字形区 4.40 / 整帧 3.83），
//    而这条判据关心的是**文字边缘**。
// ⚠️ 这是**验收工具**，不进 CI（要真出片、要底座）。

import { existsSync, mkdtempSync, readdirSync, statSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

/** 阈值：文字区的 MAE / max / Δ>40 占比。**改编码参数后必须重新标定**
 *  2026-10-18 标定（同一片段 frame 700、同一 --box，本脚本自己的灰度口径）：
 *    `slow/crf17`（现在）  MAE 2.59 / max 51 / 0.160%
 *    `veryfast/crf20`（旧） MAE 3.72 / max 52 / 0.125%
 *  ⇒ 取 **mae 3.0**：新参数过、旧参数**不过**（这是**变异自证**跑出来的数，不是拍的）。
 *  ⚠️ 阈值是**按这一条片段标的**：换片段/换帧要重跑看数字（不同内容的绝对 MAE 不可比），
 *     必要时用 --max-mae 收紧。
 *  ⚠️ max / over40 这两项对 crf 变化不敏感（实测几乎不动），留着是防**灾难性**回归
 *     （比如编码器参数被写成 yuv444→420 之外的东西、或帧格式错）。 */
const THRESHOLDS = { mae: 3.0, max: 64, over40: 0.005 };

const ROOT = resolve(fileURLToPath(new URL('.', import.meta.url)), '..');
const EXE = join(ROOT, 'target', 'debug', 'dhampir', 'bin', 'dhampir.exe');

function arg(name, def = '') {
  const i = process.argv.indexOf('--' + name);
  return i >= 0 && i + 1 < process.argv.length ? process.argv[i + 1] : def;
}
const has = (name) => process.argv.includes('--' + name);

function skip(why) {
  console.log('SKIP  ' + why);
  process.exit(0);
}

/** 灰度 rawvideo 里的一块：**ffmpeg 负责解码**（本仓脚本零依赖，不引 PNG 解码库）。 */
function grayFrame(src, w, box) {
  const args = ['-v', 'error', '-i', src, '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'gray', '-'];
  const out = spawnSync('ffmpeg', args, { maxBuffer: 1 << 30 });
  if (out.status !== 0) throw new Error('ffmpeg 解码失败：' + String(out.stderr).slice(0, 200));
  const raw = out.stdout;
  const [x0, y0, x1, y1] = box;
  const out2 = [];
  for (let y = y0; y < y1; y += 1) {
    for (let x = x0; x < x1; x += 1) out2.push(raw[y * w + x]);
  }
  return Buffer.from(out2);
}

function metrics(a, b) {
  let sum = 0;
  let max = 0;
  let over = 0;
  for (let i = 0; i < a.length; i += 1) {
    const d = Math.abs(a[i] - b[i]);
    sum += d;
    if (d > max) max = d;
    if (d > 40) over += 1;
  }
  return { mae: sum / a.length, max, over40: over / a.length };
}

// ---- self-test：判据自己得能「抓得住」 -------------------------------------
if (has('self-test')) {
  const same = Buffer.alloc(64, 128);
  const near = Buffer.alloc(64, 130);
  const far = Buffer.alloc(64, 0);
  const m0 = metrics(same, same);
  const m1 = metrics(same, near);
  const m2 = metrics(same, far);
  const ok0 = m0.mae === 0 && m0.max === 0 && m0.over40 === 0;
  const ok1 = Math.abs(m1.mae - 2) < 1e-9 && m1.over40 === 0;
  const ok2 = Math.abs(m2.mae - 128) < 1e-9 && m2.over40 === 1;
  if (ok0 && ok1 && ok2) {
    console.log('✓ self-test：同一张 = 0；差 2 级 = MAE 2 且不越 40；差 128 = 全部越 40');
    process.exit(0);
  }
  console.error('✗ self-test 失败：', { m0, m1, m2 });
  process.exit(1);
}

// ---- 真跑 ---------------------------------------------------------------
const project = arg('project');
if (!project) skip('缺 --project（这是验收工具：要一个真片段）');
if (!existsSync(EXE)) skip('缺底座 exe：' + EXE);
if (spawnSync('ffmpeg', ['-version']).status !== 0) skip('PATH 上没有 ffmpeg');

const frame = Number(arg('frame', '700'));
const boxArg = arg('box', '');
const work = mkdtempSync(join(tmpdir(), 'yt-encq-'));
const pngDir = join(work, 'png');
const doc = join(work, 'q.doc.json');

// 1) 转译（与出片同一条腿）
const tr = spawnSync('node', [join(ROOT, 'tools', 'polish-to-dhampir.mjs'), project, '--out', doc, '--keep-muted'], {
  encoding: 'utf8',
  env: { ...process.env, TMPDIR: work, TEMP: work, TMP: work },
});
if (tr.status !== 0) skip('转译失败：' + String(tr.stderr).slice(0, 300));

// 2) 字体：从库里挑一支（底座不猜系统字体）
const fontDir = process.env.VT_FONT_DIR || join(ROOT, 'target', 'debug', 'users', '1', 'resources', 'fonts');
const pickFont = (dir) => {
  if (!existsSync(dir)) return null;
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    let st;
    try { st = statSync(p); } catch { continue; }
    if (st.isDirectory()) {
      const inner = pickFont(p);
      if (inner) return inner;
    } else if (/\.(ttf|otf)$/i.test(name)) {
      return p;
    }
  }
  return null;
};
const font = pickFont(fontDir);
if (!font) skip('字体库里一支字体都没有（' + fontDir + '）—— 底座不猜系统字体');

const common = ['--project', doc, '--asset-root', resolve(project), '--font-file', font, '--font-dir', fontDir];

// 3) lossless PNG
const fr = spawnSync(EXE, ['frame', ...common, '--frame', String(frame), '--out', pngDir], { encoding: 'utf8' });
if (fr.status !== 0) skip('底座 frame 失败：' + String(fr.stderr || fr.stdout).slice(0, 300));
const png = join(pngDir, 'frame-' + String(frame).padStart(4, '0') + '.png');
if (!existsSync(png)) skip('底座没写出 PNG：' + png);

// 4) 出片一小段并解码同一帧
const mp4 = join(work, 'q.mp4');
const rd = spawnSync(EXE, ['render', ...common, '--from', String(frame), '--to', String(frame + 1), '--out', mp4, '--no-audio'], {
  encoding: 'utf8',
});
if (rd.status !== 0) skip('底座 render 失败：' + String(rd.stderr || rd.stdout).slice(0, 300));

const sizeOut = spawnSync('ffprobe', ['-v', 'error', '-select_streams', 'v:0', '-show_entries', 'stream=width,height', '-of', 'csv=p=0', mp4], { encoding: 'utf8' });
const [w, h] = String(sizeOut.stdout).trim().split(',').map(Number);
if (!(w > 0 && h > 0)) skip('ffprobe 读不出尺寸');

const region = boxArg ? boxArg.split(',').map(Number) : [0, 0, w, h];
if (region.some((v) => !Number.isFinite(v)) || region[2] > w || region[3] > h) skip('--box 非法或超出画面：' + boxArg);

const ref = grayFrame(png, w, region);
const enc = grayFrame(mp4, w, region);
const m = metrics(ref, enc);

const maxMae = Number(arg('max-mae', String(THRESHOLDS.mae)));
const pass = m.mae <= maxMae && m.max <= THRESHOLDS.max && m.over40 <= THRESHOLDS.over40;
console.log('成片文字质量（frame ' + frame + '，区域 ' + region.join(',') + '）：');
console.log('  MAE = ' + m.mae.toFixed(2) + '（阈值 ≤ ' + THRESHOLDS.mae + '）');
console.log('  max = ' + m.max + '（阈值 ≤ ' + THRESHOLDS.max + '）');
console.log('  Δ>40 占比 = ' + (m.over40 * 100).toFixed(3) + '%（阈值 ≤ ' + (THRESHOLDS.over40 * 100).toFixed(2) + '%）');
if (pass) {
  console.log('✓ 通过');
  process.exit(0);
}
console.error('✗ 不通过：编码把文字边缘毁得太多 —— 调 encoder_args（preset/crf）或 cut_ranges');
process.exit(1);
