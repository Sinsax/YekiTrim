#!/usr/bin/env node
// **clippy 预算门禁**（2026-11-25，整改 T20a 的配套）。
//
// 背景：`cargo clippy --workspace --all-targets` 现在是 129 条告警（196 → 129 清了第一批）。
// 计划 T20 说「clippy 进 gate」，但**在清零之前**塞 `-D warnings` 只有两种结局：立刻红一片，
// 或者逼人拿 baseline 把红盖住（而 baseline 只准登记环境性失败）。所以先立一个**只降不升**的预算：
// 数量超过预算就红（阻止债务增长），低于预算就提示下调 —— 清到 0 之后再换成 `-D warnings`。
//
// 口径：只数 `clippy::*` 的 warning，并按 `code|file:line` 去重 ——
// 同一份代码会被 lib / lib-test / bin 各编一遍，cargo 会重复报同一条（实测 20+ 条重复）。
//
// 用法：
//   node scripts/check-clippy-budget.mjs            # 超预算 exit 1；低于预算 exit 0 并提示下调
//   node scripts/check-clippy-budget.mjs --update   # 把当前数量写回预算文件（人工确认过再跑）
// 缺 cargo / 编译失败 ⇒ exit 3（缺前置按本仓约定是 3，不是通过）。
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = join(dirname(fileURLToPath(import.meta.url)), '..');
const budgetFile = join(repo, 'scripts', 'check-clippy-budget.json');
const update = process.argv.includes('--update');

let raw;
try {
  raw = execFileSync('cargo', ['clippy', '--workspace', '--all-targets', '--message-format=json', '--quiet'], {
    cwd: repo,
    encoding: 'utf8',
    maxBuffer: 256 * 1024 * 1024,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
} catch (e) {
  const msg = (e && (e.stdout || e.message)) || String(e);
  console.log('· SKIP clippy 预算：cargo clippy 没跑起来（装 Rust 工具链后再跑）');
  console.log('  ' + String(msg).split('\n')[0].slice(0, 160));
  process.exit(3);
}

const seen = new Set();
const items = [];
for (const line of raw.split('\n')) {
  if (!line.startsWith('{')) continue;
  let msg;
  try { msg = JSON.parse(line); } catch { continue; }
  if (msg.reason !== 'compiler-message') continue;
  const m = msg.message;
  if (!m || m.level !== 'warning') continue;
  const code = (m.code && m.code.code) || '';
  if (!code.startsWith('clippy::')) continue;
  const span = (m.spans || []).find((s) => s.is_primary) || (m.spans || [])[0];
  const where = span ? `${span.file_name}:${span.line_start}` : '?';
  const key = `${code}|${where}`;
  if (seen.has(key)) continue;
  seen.add(key);
  items.push({ code, where, text: String(m.message || '').slice(0, 120) });
}

const count = items.length;
const budget = JSON.parse(readFileSync(budgetFile, 'utf8'));
const cap = Number(budget.budget);
console.log(`clippy 告警（去重后）：${count} / 预算 ${cap}`);

if (update) {
  budget.budget = count;
  writeFileSync(budgetFile, JSON.stringify(budget, null, 2) + '\n');
  console.log(`已把预算写回 ${budgetFile}`);
  process.exit(0);
}

if (count > cap) {
  const byCode = new Map();
  for (const it of items) byCode.set(it.code, (byCode.get(it.code) || 0) + 1);
  console.log(`❌ clippy 债务涨了 ${count - cap} 条（预算只降不升）—— 按类别：`);
  for (const [code, n] of [...byCode.entries()].sort((a, b) => b[1] - a[1]).slice(0, 10)) {
    console.log(`   ${String(n).padStart(3)} × ${code}`);
  }
  console.log('   前 10 条位置：');
  for (const it of items.slice(0, 10)) console.log(`   · ${it.where}  ${it.code}`);
  console.log('   要么清掉新增的，要么（确认不可避免时）跑 --update 并说明理由。');
  process.exit(1);
}

if (count < cap) {
  console.log(`✅ 低于预算 ${cap - count} 条 —— 可以把预算下调到 ${count}（node scripts/check-clippy-budget.mjs --update）。`);
} else {
  console.log('✅ 正好等于预算。');
}
process.exit(0);