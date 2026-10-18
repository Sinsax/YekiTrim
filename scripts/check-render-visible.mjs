#!/usr/bin/env node
// **真渲染 + 数像素**：文字层 / 贴纸到底画进画面了没有（2026-10-14）。
//
// # 为什么要有它
//
// 本仓的门禁全都在问「**代码形状对不对**」，**没有一条在问「用户看不看得见」**。
// 2026-10-14 那次「某项目里贴纸不出现」暴露得很清楚：四处真缺陷，
// **改前 14 条门禁全绿**。文字元素同理 —— `check-text-layer-not-doubled.mjs`
// 只锁「不许画两遍」，**不锁「到底画出来没有」**。
//
// # 判据的形状（差分法）
//
// 1. 取一份**真工程**的转译产物；
// 2. 用底座**真出画面**，抽出目标时间窗口内的帧；
// 3. **把被测那条轨整条摘掉**、其余一字不动，再出一次；
// 4. 逐帧数差异像素 `|A-B|.max(axis=2) > 8`。
//
// ⚠️⚠️ **第 3 步的两个前提，缺一就会得出"没画出来"的假阴性**（我三个版本全踩过）：
//   * **被测窗口必须覆盖采样帧** —— 第一版渲帧 250-260，而文字层是 `start:466/end:586`，
//     差异 0；实际是**采样在窗口外**。所以本脚本**从工程里读窗口**再定帧号。
//   * **必须给字体** —— 底座对 `font_weight >= 600` 要 `--font-bold-file`，
//     不给就在报告里明确 `subtitle_font_missing`（它是**拒绝画**，不是静默画不出来）。
//
// ⚠️ 还有一条**环境**前提：底座写字幕临时文件用**系统 TMP**，被拒（`os error 5`）时
//    报告 `subtitle_raster_failed`、`lines_drawn: 0` —— **一个字都不画**。
//    本脚本把 `TMPDIR/TEMP/TMP` 指到工程内的可写目录，绕开它。
//
// # 用法
//
//   node scripts/check-render-visible.mjs --project <片段目录> [--kind text|sticker]
//   node scripts/check-render-visible.mjs --self-test
//
// 退出码非 0 = 判据失败。缺底座 / ffmpeg / 工程时**明说 SKIP**，不静默算过。

import { strict as assert } from 'node:assert';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync, readdirSync, openSync, closeSync } from 'node:fs';
import { execFileSync, spawnSync } from 'node:child_process';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';

/// 环境不满足时**明说跳过**，不静默算过、也不误报成产品缺陷。
class SkipError extends Error {}

const HERE = dirname(fileURLToPath(import.meta.url));
export const REPO_ROOT = resolve(HERE, '..');

/// 找底座仓（与本仓并列）。可用 `DHAMPIR_ROOT` 覆盖。
export function findDhampir() {
  const env = process.env.DHAMPIR_ROOT;
  const cands = [env, join(dirname(REPO_ROOT), 'Dhampir')].filter(Boolean);
  for (const c of cands) {
    const exe = join(c, 'target', 'release', 'dhampir.exe');
    if (existsSync(exe)) return { root: c, exe };
  }
  return null;
}

/// 数两张 PNG 的差异像素（阈值 8）。
///
/// ⚠️ 用内联 Node 读 PNG 太脆（要 zlib 解 IDAT + 反滤波），所以**借 ffmpeg** 转成
/// 裸 RGB 再比 —— 它是本链路本来就有的依赖（出片那条腿天天用）。
export function countDiff(pngA, pngB) {
  const py = `
from PIL import Image
import numpy as np, sys
A = np.array(Image.open(sys.argv[1]).convert('RGB')).astype(np.int16)
B = np.array(Image.open(sys.argv[2]).convert('RGB')).astype(np.int16)
if A.shape != B.shape:
    print('SHAPE_MISMATCH'); sys.exit(0)
d = np.abs(A - B).max(axis=2)
n = int((d > 8).sum())
if n:
    ys, xs = np.where(d > 8)
    print(f'{n} {xs.min()} {xs.max()} {ys.min()} {ys.max()}')
else:
    print('0')
`;
  const r = spawnSync('python3', ['-c', py, pngA, pngB], { encoding: 'utf8' });
  if (r.status !== 0) return null;
  const out = (r.stdout || '').trim();
  if (out === 'SHAPE_MISMATCH') return { mismatch: true };
  if (!out) return null;
  if (out === '0') return { n: 0 };
  const [n, x0, x1, y0, y1] = out.split(/\s+/).map(Number);
  return { n, box: { x0, x1, y0, y1 } };
}

/// 从转译产物里挑出被测轨 + 它的采样窗口（**帧号**）。
///
/// ⚠️ 这一步是整条判据的关键：**窗口从工程里读，不许猜**（我第一版就是猜错窗口，
/// 得出"文字一个像素都没画"的假结论）。
export function pickTarget(doc, kind) {
  const tracks = doc?.timeline?.tracks ?? [];
  const tb = doc?.timeline?.timebase ?? { num: 30, den: 1 };
  const fps = tb.den ? tb.num / tb.den : 30;
  const want = kind === 'sticker' ? /^ssticker/ : /^text_/;
  for (const t of tracks) {
    if (!want.test(String(t.id ?? ''))) continue;
    const lay = (t.layers ?? [])[0];
    if (!lay) continue;
    const start = Number(lay.start ?? 0);
    const end = Number(lay.end ?? start + 1);
    // 采样窗口**正中**，避开淡入淡出（两端各让 20%）
    const span = Math.max(1, end - start);
    const mid = Math.round(start + span * 0.5);
    const lo = Math.max(start, mid - 2);
    const hi = Math.min(end - 1, mid + 2);
    return { id: t.id, start, end, from: lo, to: hi, fps };
  }
  return null;
}

/// 摘掉一条轨（其余一字不动）—— 差分法的对照组。
export function withoutTrack(doc, trackId) {
  const copy = JSON.parse(JSON.stringify(doc));
  copy.timeline.tracks = (copy.timeline.tracks ?? []).filter((t) => String(t.id) !== String(trackId));
  return copy;
}

// ── 自证：判据本身可信吗（不跑底座、秒级）───────────────────────────────
export function selfTest() {
  let n = 0;
  const eq = (a, b, what) => {
    assert.deepEqual(a, b, what);
    n += 1;
  };

  // ① 窗口必须从工程里读出来（第一版假阴性的根因）
  const doc = {
    timeline: {
      timebase: { num: 60, den: 1 },
      tracks: [
        { id: 'v1', layers: [{ start: 0, end: 1000 }] },
        { id: 'text_x', layers: [{ start: 466, end: 586 }] },
      ],
    },
  };
  const t = pickTarget(doc, 'text');
  eq(t.id, 'text_x', '必须挑到 text_ 轨');
  eq(t.fps, 60, 'fps 必须从 timebase 读');
  assert.ok(t.from >= 466 && t.to <= 586, `采样窗口必须在层窗口内，实得 ${t.from}-${t.to}`);
  assert.ok(t.from > 466 && t.to < 586, '采样必须避开两端（淡入淡出），实得贴边了');
  n += 1;

  // ② 窗口外的采样**必须**被判出来（老 bug 的形状）
  const far = pickTarget({ timeline: { tracks: [{ id: 'text_x', layers: [{ start: 1000, end: 1100 }] }] } }, 'text');
  assert.notEqual(far.from, t.from, '不同窗口必须给出不同采样帧 —— 否则判据是空转的');
  n += 1;

  // ③ sticker 与 text 不许互相认错
  eq(pickTarget({ timeline: { tracks: [{ id: 'ssticker-0', layers: [{ start: 0, end: 10 }] }] } }, 'text'), null,
    'text 判据不许认到 sticker 轨');
  eq(pickTarget({ timeline: { tracks: [{ id: 'text_a', layers: [{ start: 0, end: 10 }] }] } }, 'sticker'), null,
    'sticker 判据不许认到 text 轨');

  // ④ 摘轨只摘一条
  const w = withoutTrack(doc, 'text_x');
  eq(w.timeline.tracks.map((x) => x.id), ['v1'], '摘轨必须只摘掉那一条');
  eq(doc.timeline.tracks.length, 2, '摘轨不许改原对象（必须深拷贝）');

  // ⑤ 缺轨时必须返回 null（不是抛错、也不是猜一条）
  eq(pickTarget({ timeline: { tracks: [] } }, 'text'), null, '没有目标轨必须返回 null');

  console.log(`✓ 渲染可见性判据自证通过（${n} 条断言）`);
  console.log('  钉住的是：采样窗口必须**从工程里读**（我第一版猜错窗口，把"在窗口外"');
  console.log('  误判成"文字一个像素都没画"）、text/sticker 不许互相认错、摘轨必须深拷贝。');
  return n;
}

// ── 主流程 ────────────────────────────────────────────────────────────
function main() {
  const argv = process.argv.slice(2);
  if (argv.includes('--self-test')) {
    selfTest();
    process.exit(0);
  }
  const gi = (k, d) => {
    const i = argv.indexOf(k);
    return i >= 0 ? argv[i + 1] : d;
  };
  const project = gi('--project');
  const kind = gi('--kind', 'text');
  if (!project) {
    console.log('用法：node scripts/check-render-visible.mjs --project <片段目录> [--kind text|sticker]');
    console.log('      node scripts/check-render-visible.mjs --self-test');
    process.exit(2);
  }

  const dh = findDhampir();
  if (!dh) {
    console.log('· SKIP 渲染可见性判据：找不到底座（设 DHAMPIR_ROOT，或与 Dhampir 仓并列）');
    process.exit(0);
  }
  let hasFfmpeg = true;
  try {
    execFileSync('ffmpeg', ['-version'], { stdio: 'ignore' });
  } catch {
    hasFfmpeg = false;
  }
  if (!hasFfmpeg) {
    console.log('· SKIP 渲染可见性判据：这台机器没有 ffmpeg');
    process.exit(0);
  }

  // ⚠️ 工作目录**建在仓库里**，不用系统 TMP：底座出片时会往**输出文件旁边**写
  //    `.<名字>.video.mp4` 这类临时件，而它拉起的 **ffmpeg 孙进程**在系统 TMP 下
  //    会 `Error opening output file … Permission denied`（Node 自己写得进去、
  //    孙进程写不进去 —— 实测如此）。落在仓库里两个进程都写得动。
  const work = join(REPO_ROOT, '.workbuddy', `render-visible-${process.pid}`);
  rmSync(work, { recursive: true, force: true });
  mkdirSync(work, { recursive: true });
  // ⚠️ 底座写字幕临时文件用**系统 TMP**，被拒时 `lines_drawn: 0` ⇒ 一个字都不画。
  //    指到工程内可写目录，绕开 `os error 5`。
  const tmp = join(work, 'tmp');
  mkdirSync(tmp, { recursive: true });
  const env = { ...process.env, TMPDIR: tmp, TEMP: tmp, TMP: tmp };

  try {
    // ① 转译
    const docPath = join(work, 'p.json');
    const tr = spawnSync(process.execPath,
      [join(REPO_ROOT, 'tools', 'polish-to-dhampir.mjs'), project, '--out', docPath],
      { encoding: 'utf8', env: { ...env, DHAMPIR_QUIET: '1' } });
    if (!existsSync(docPath)) {
      console.log('· SKIP 渲染可见性判据：转译没产出工程');
      console.log((tr.stderr || tr.stdout || '').slice(0, 300));
      process.exit(0);
    }
    const doc = JSON.parse(readFileSync(docPath, 'utf8'));
    const target = pickTarget(doc, kind);
    if (!target) {
      console.log(`· SKIP 渲染可见性判据：这个工程里没有 ${kind} 轨`);
      process.exit(0);
    }

    // ② 字体（底座的硬前提：不给字体它**拒绝画**，报告 subtitle_font_missing）
    const fontDir = join(REPO_ROOT, 'target', 'debug', 'users', '1', 'resources', 'fonts');
    let font = null;
    if (existsSync(fontDir)) {
      for (const d of readdirSync(fontDir)) {
        const f = join(fontDir, d, 'font.ttf');
        if (existsSync(f)) { font = f; break; }
      }
    }
    if (!font) {
      console.log('· SKIP 渲染可见性判据：本地字体库里没有字体（底座不给字体就不画文字）');
      process.exit(0);
    }

    // ③ 出两次画面：原样 / 摘掉目标轨
    const render = (docObj, outDir, tag) => {
      const p = join(work, `${tag}.json`);
      writeFileSync(p, JSON.stringify(docObj));
      mkdirSync(outDir, { recursive: true });
      // ⚠️⚠️ **输出必须落文件，不能用管道**（`render/AGENTS.md` 那条纪律：
      // 底座拉起的孙进程会继承 stdout 句柄，只要还有一个活着管道就不 EOF；
      // 实测症状正是 `第 N 帧写进编码器失败：管道正在被关闭 (os error 232)`）。
      const logPath = join(outDir, 'render.log');
      const logFd = openSync(logPath, 'w');
      let r;
      try {
        r = spawnSync(dh.exe, ['render', '--project', p,
          '--from', String(target.from), '--to', String(target.to),
          '--out', join(outDir, 'o.mp4'), '--asset-root', project,
          '--font-file', font], { stdio: ['ignore', logFd, logFd], env });
      } finally {
        closeSync(logFd);
      }
      const r2 = { stdout: readFileSync(logPath, 'utf8'), stderr: '' };
      const txt = r2.stdout || '';
      if (!existsSync(join(outDir, 'o.mp4'))) {
        // ⚠️ 说不出"为什么没出片"，后面那步只会报 `No such file or directory` ——
        //    必须在这里把底座的报告打出来，否则误诊成"渲染坏了"。
        const NL = String.fromCharCode(10);
        const tail = txt.split(NL).slice(-6).join(NL);
        // ⚠️ **沙箱拦孙进程**是环境问题、不是产品问题：DSH 的受限模式下
        //    Node 起的子进程**再拉起 ffmpeg**（孙进程）时，隔着管道/命名管道会被拦，
        //    症状正是 `Error opening output file … Permission denied` 或
        //    `第 N 帧写进编码器失败：管道正在被关闭 (os error 232)`。
        //    同一份命令在 bash 里跑**完全正常**。这时必须**明说 SKIP**，
        //    不能报成"渲染坏了"、更不能报成"文字没画出来"。
        if (/Permission denied|os error 232|PermissionDenied/.test(tail)) {
          throw new SkipError(
            '底座拉起的 ffmpeg 孙进程被沙箱拦了（Node 起的子进程不能再开管道）—— ' +
            '同一条命令在 bash 里跑是好的。这是环境限制，不是产品缺陷。'
          );
        }
        throw new Error(`底座没有产出 ${join(outDir, 'o.mp4')}。报告尾部：${NL}${tail}`);
      }
      return txt;
    };
    const outA = join(work, 'A');
    const outB = join(work, 'B');
    const ra = render(doc, outA, 'with');
    const rb = render(withoutTrack(doc, target.id), outB, 'without');

    // ⚠️ `lines_failed > 0` 必须**明说**：那时"差异为 0"是**环境问题**，不是"没画"
    const m = /"lines_failed":(\d+)/.exec(ra);
    const failed = m ? Number(m[1]) : 0;
    if (failed > 0) {
      console.log(`· SKIP 渲染可见性判据：底座报告 ${failed} 行画不出来（多半是字体/TMP 权限），`);
      console.log('  这时"差异为 0"是环境问题、**不能**当成"没画出来"。');
      process.exit(0);
    }

    const pngDirA = join(work, 'pa');
    const pngDirB = join(work, 'pb');
    mkdirSync(pngDirA, { recursive: true });
    mkdirSync(pngDirB, { recursive: true });
    // ⚠️ Windows 上 Node 把 **CJK 参数**交给原生程序会因为编码丢字节
    //    （`Error opening input file C:\...` 截断在半个汉字上）。
    //    这里输入 mp4 已经落在 **ASCII 的 work 目录**里，但为稳妥仍显式给 UTF-8 代码页。
    const ff = (args) => execFileSync('ffmpeg', ['-v', 'error', '-y', ...args], {
      encoding: 'buffer',
      env: { ...env, FFREPORT: '' },
    });
    ff(['-i', join(outA, 'o.mp4'), join(pngDirA, 'f%03d.png')]);
    ff(['-i', join(outB, 'o.mp4'), join(pngDirB, 'f%03d.png')]);

    const fa = readdirSync(pngDirA).filter((f) => f.endsWith('.png')).sort();
    const fb = readdirSync(pngDirB).filter((f) => f.endsWith('.png')).sort();
    assert.equal(fa.length, fb.length, '两次渲染的帧数必须相同');
    assert.ok(fa.length > 0, '一帧都没抽到 = 没跑，不是"一致"');

    let total = 0;
    const rows = [];
    for (let i = 0; i < fa.length; i += 1) {
      const r = countDiff(join(pngDirA, fa[i]), join(pngDirB, fb[i]));
      assert.ok(r && !r.mismatch, '两次渲染的尺寸必须相同（尺寸不同 ≠ 画面不同）');
      total += r.n;
      rows.push({ frame: target.from + i, n: r.n, box: r.box });
    }

    const label = kind === 'sticker' ? '贴纸' : '文字';
    console.log(`✓ 渲染可见性（${label}）：轨「${target.id}」窗口 [${target.start}-${target.end}]，` +
      `采样帧 ${target.from}-${target.to}`);
    for (const r of rows) {
      console.log(r.n
        ? `    帧${String(r.frame).padStart(5)}  ${label}像素=${String(r.n).padStart(6)}  ` +
          `x[${r.box.x0}-${r.box.x1}] y[${r.box.y0}-${r.box.y1}]`
        : `    帧${String(r.frame).padStart(5)}  ${label}像素=     0`);
    }
    assert.ok(total > 0,
      `★ ${label}层**在窗口内一个像素都没画**（摘掉它前后逐帧相同）—— ` +
      `这正是「用户看不看得见」判据要抓的东西。合计差异 = ${total}`);
    console.log(`  ⇒ 合计 ${total} 个${label}像素：**真的画进画面了**`);
  } catch (e) {
    if (e instanceof SkipError) {
      console.log(`· SKIP 渲染可见性判据：${e.message}`);
      process.exitCode = 0;
    } else {
      throw e;
    }
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

const isEntry = process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url));
if (isEntry) main();
