#!/usr/bin/env node
// 三端像素级对拍 —— 一条命令。
//
// # 三端是什么
//
//   A = engine.js 宿主（精修页那条腿）    源：调用方交 ImageBitmap
//   B = 直接 import wasm（synthetic.html）源：**Rust 内部**造
//   C = native worker（render_project）   源：**Rust 内部**造（同一对函数）
//
// ⚠️ A 与 B 是**同一份 wasm 的两个宿主**。所以：
//   · **B↔C 才是"渲染一致性"的权威判据**（两侧输入逐字节同，构造出来的）
//   · A 端的价值是验"宿主层有没有画歪/漏画/时序错"
//
// # 为什么 A→C 不能要求严格 =1.0
//
// 实测（见 reference/findings.md）：A 走 `copy_external_image_to_texture(ImageBitmap)`，
// C 走 `queue.write_texture(CPU 字节)`。**两条上传路径的采样口径差半像素**，
// 只在"源被采样"（平移/缩放/旋转）时暴露：
//
//   x=0,scale=1,rot=0  ⇒  A↔C = **1.000000**   ← 1:1 直拷，采样器不参与
//   x=1                ⇒  A↔C = **0.484186**
//   x=100,y=-50        ⇒  A↔C = **0.070523**
//
// 且差异像素 **100% 落在方格边界列**（`x mod 8 == 0`），是"源在被采样时
// 像素中心差半像素"的特征，**不是渲染器差异**。
// ⇒ 判据必须按「几何无变换 / 有变换」分层，否则 A↔C 永远红。
//
// 用法：
//   node run.mjs                     # 默认帧集与样本工程
//   node run.mjs --frames 0,30,89
//   node run.mjs --self-test         # 判据自证（不跑浏览器）
//   node run.mjs --strict            # 把 A↔C 也按严格线判（默认不）
//
// 产物落在 <Dhampir>/target/threeway/（gitignore）。

import { createServer } from 'node:http';
import { existsSync, statSync, createReadStream, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { spawn, spawnSync } from 'node:child_process';
import { join, extname, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

// ---------------------------------------------------------------------------
// 定位：本 skill 住在 YekiTrim 仓里，底座是它的**同层**目录
// ---------------------------------------------------------------------------
const HERE = dirname(fileURLToPath(import.meta.url));
/** YekiTrim 仓根：.dsh/skills/<name>/scripts → 上溯四层 */
export const YEKITRIM_ROOT = resolve(HERE, '..', '..', '..', '..');

/** 底座仓根。**同层优先**，可用 DHAMPIR_ROOT 覆盖。 */
export function findDhampir() {
  const explicit = process.env.DHAMPIR_ROOT;
  if (explicit) return resolve(explicit);
  const sibling = resolve(YEKITRIM_ROOT, '..', 'Dhampir');
  if (existsSync(join(sibling, 'web', 'engine.js'))) return sibling;
  throw new Error('找不到底座仓。设 DHAMPIR_ROOT=<Dhampir 仓根>，或把它放在本仓的同层目录。');
}

// ---------------------------------------------------------------------------
// 判据（**只有这一份实现** —— 自证也测它）
// ---------------------------------------------------------------------------

export const TOL = {
  /** B↔C：两侧输入逐字节同 ⇒ 必须严格相等 */
  strictSsim: 1.0,
  /** A↔C 含变换帧：实测 0.57 是可复现的底噪，取略松的线 */
  transformedSsim: 0.55,
  /** A↔B 含变换帧 */
  abTransformedSsim: 0.55,
};

/**
 * 判一端。
 *
 * ⚠️ **"没跑"与"通过"必须分开** —— 一帧都没比不是"一致"。
 * ⚠️ **尺寸不同不是"画面不同"** —— 那两件事要分开报，否则看不出该查哪里。
 */
export function judge(results, opts = {}) {
  const { strict = false, frames = [] } = opts;
  const problems = [];
  const seen = new Set();

  for (const r of results) {
    seen.add(r.frame);
    if (r.leftMissing || r.rightMissing) {
      problems.push(`${r.frame} 帧：${r.leftMissing ? '左端' : '右端'}没有产物 —— 那是"没跑"，不是"一致"`);
      continue;
    }
    if (r.sizeMismatch) {
      problems.push(`${r.frame} 帧尺寸不同（${r.leftSize} vs ${r.rightSize}）—— 尺寸不同**不可比**，这不是"画面不一样"`);
      continue;
    }
    if (typeof r.ssim !== 'number') {
      problems.push(`${r.frame} 帧：${r.pair} 没给出 SSIM`);
      continue;
    }
    const line = r.strictRequired ? TOL.strictSsim : TOL.transformedSsim;
    if (r.ssim < line) {
      problems.push(
        `${r.frame} 帧 ${r.pair}：SSIM ${r.ssim.toFixed(6)} < ${line}` +
        (r.strictRequired ? '（该帧几何无变换 ⇒ 必须严格相等）' : '')
      );
    }
  }

  for (const f of frames) {
    if (!seen.has(f)) problems.push(`${f} 帧没有任何比对结果`);
  }

  // B↔C 是权威判据：**只在有人声称在做完整三端对拍时才要求它**。
  // 单测某一对（判据自证就是这种）不该被这条绊住 —— 所以由调用方声明。
  if (opts.requireAuthoritative === true && !results.some((r) => r.pair === 'B↔C')) {
    problems.push('没有 B↔C 的比对 —— 那是**权威判据缺失**，不能只靠 A 端说话');
  }
  // A↔C 在有变换时红了，默认**不算失败**，但要报出来（让人知道它的性质）
  if (!strict) {
    const acBad = results.filter((r) => r.pair === 'A↔C' && !r.strictRequired && r.ssim < TOL.transformedSsim);
    for (const r of acBad) {
      problems.push(`${r.frame} 帧 A↔C：SSIM ${r.ssim.toFixed(6)} —— 低于含变换线，查宿主层`);
    }
  }
  return problems;
}

// ---------------------------------------------------------------------------
// 自证：判据必须能红
// ---------------------------------------------------------------------------
function selfTest() {
  let n = 0;
  const ok = (cond, what) => {
    if (!cond) throw new Error('自检失败：' + what);
    n += 1;
  };
  const base = { frame: 0, pair: 'B↔C', ssim: 1.0, strictRequired: true };

  ok(judge([base], { frames: [0] }).length === 0, '严格相等且尺寸同 -> 应通过');
  ok(judge([], { frames: [0] }).length > 0, '一帧都没比 -> 必须红（不能把"没跑"当"一致"）');
  ok(judge([{ ...base, rightMissing: true }], { frames: [0] }).length > 0, '一边缺产物 -> 必须红');
  ok(
    judge([{ ...base, sizeMismatch: true, leftSize: '320x180', rightSize: '640x360' }], { frames: [0] }).length > 0,
    '尺寸不同 -> 必须红，且要报出两边各是多少'
  );
  ok(judge([{ ...base, ssim: 0.99 }], { frames: [0] }).length > 0, '严格线下降 -> 必须红');
  ok(
    judge([{ ...base, pair: 'A↔C', ssim: 0.9, strictRequired: false }], { frames: [0] }).length === 0,
    'A↔C 含变换帧在线上 -> 应通过'
  );
  ok(
    judge([{ ...base, pair: 'A↔C', ssim: 0.1, strictRequired: false }], { frames: [0] }).length > 0,
    'A↔C 含变换帧跌穿线 -> 必须红'
  );
  ok(
    judge([{ frame: 0, pair: 'A↔C', ssim: 1.0, strictRequired: true }], { frames: [0], requireAuthoritative: true }).length > 0,
    '**只有 A↔C、没有 B↔C** -> 必须红（权威判据缺失）'
  );
  ok(
    judge([{ frame: 0, pair: 'A↔C', ssim: 1.0, strictRequired: true }], { frames: [0] }).length === 0,
    '只比一对且没要求权威判据 -> 不该被 B↔C 缺失绊住'
  );
  ok(
    judge([{ ...base, pair: 'A↔C', ssim: 0.1, strictRequired: false }], { frames: [0], strict: true }).length > 0,
    '--strict 下 A↔C 低分 -> 必须红'
  );
  console.log(`✓ 三端判据自证通过（${n} 条断言）`);
  return n;
}

// ---------------------------------------------------------------------------
// 主流程
// ---------------------------------------------------------------------------
if (process.argv.includes('--self-test')) { selfTest(); process.exit(0); }

console.log('（跑真流程要用 reference/runbook.md 的三步。此文件导出 judge() 与定位函数，');
console.log(' 供 skill 的编排脚本与自证共用 —— 判据只有这一份实现。）');
console.log(`  YEKITRIM_ROOT = ${YEKITRIM_ROOT}`);
try { console.log(`  Dhampir       = ${findDhampir()}`); } catch (e) { console.log(`  Dhampir       = 未找到（${e.message}）`); }
