#!/usr/bin/env node
// 守卫：**转译报告的 DROP / CLAMP 清单**（M2，2026-09-30）。
//
// # 为什么它比"再写几条断言"更值
//
// `tools/AGENTS.md` 把这份清单定义成**接口产物**（"CI 断言 `DROP == 0` / 把 `CLAMP` 逐条挂台账"）。
// 于是"清单变了"这件事本身就该有人看一眼：
//   * **多了**一条 → 某次改动把一档事件转丢了（M2 一直在修的就是这类）；
//   * **少了一条** → 修好了（好事）或**悄悄不报了**（坏事：报告被削弱，比转丢了更难发现）。
// 两种都要人来确认，所以这里把清单钉成**基线**：变了就红，改基线要顺手写清"为什么它该长这样"。
//
// 判据刻意只覆盖 **DROP + CLAMP**：`NOTE` 是解释性的，措辞天天变（把它也钉住会让守卫天天红）。
//
// # 跑法
//
//     node scripts/check-dhampir-drop-list.mjs
//
// 零外部依赖（node + 本仓的转译器）；样本是**仓库自带**的演示工程，所以它在常规门禁里跑得动
// （不需要底座二进制、ffmpeg、Chrome、GPU —— 那些是对拍工具的要求，不是这一条）。
import { strict as assert } from 'node:assert';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const CLIP = join(repo, 'docs', 'wgpu-effect-reference', 'demo');
const TRANSLATOR = join(repo, 'tools', 'polish-to-dhampir.mjs');

// 样本的 `clip.mp4` 被 `.gitignore` 排除（`*.mp4`）⇒ 干净检出上它不存在。
// 缺它时转译器探不到源尺寸，会多报一条 CLAMP `stage` —— 那是样本缺失，不是防线变化。
// 按仓库惯例：**缺依赖就 SKIP 并明说**，不写成通过、也不拿它去改基线。
if (!existsSync(join(CLIP, 'clip.mp4'))) {
  console.log('· SKIP：缺 docs/wgpu-effect-reference/demo/clip.mp4（.gitignore 排除的媒体样本）',
    '—— 本判据要在带样本的机器上跑');
  process.exit(0);
}

// ---------------------------------------------------------------------------
// 基线：**每一条都要写清"为什么它还在"**。改了清单就改这里，并在提交说明里说清为什么。
// ---------------------------------------------------------------------------
const BASELINE = {
  DROP: [
    // ⚠️ `vignette` 这条**已删除**（2026-10-01 D4）：以前它在这里的理由是"参照出片路径里没有暗角、
    // 不替参照补画"。那一刀切在参照侧 ⇒ 现在本仓 canvas 补上了暗角绘制、转译器也映射了底座
    // `vignette`，两端一致，报告里不再有这一条。**别把它加回来**（加回来就是"又变成预览有、出片没有"）。
    { where: 'cover', why: '封面是静态图产出，本仓自己另开一条（见 docs/plan/cover-dhampir-tradeoff.md）' },
  ],
  // CLAMP 现在是**空的**：唯二的两条（`split` 无 shear、`color_shift` 无 sepia）随着这两个类型
  // 在 2026-10-01 的类型收敛里被删掉而消失（见 docs/plan/fx-type-consolidation.md）。
  // ⚠️ 空不等于"以后都不用管"：这条门禁对**新增**的 CLAMP 更敏感（多一条就红）。
  CLAMP: [],
};

/// 抓转译器的三方清单（解析 stdout；与 `.workbuddy/dhampir-report.mjs` 同一套规则）。
function collectIssues() {
  const dir = mkdtempSync(join(tmpdir(), 'yekitrim-drop-'));
  try {
    const out = execFileSync(
      process.execPath,
      [TRANSLATOR, CLIP, '--out', join(dir, 'demo.doc.json'), '--keep-muted'],
      { encoding: 'utf8' },
    );
    const got = { DROP: [], CLAMP: [], NOTE: [] };
    let cur = null;
    for (const raw of out.split('\n')) {
      const line = raw.replace(/\s+$/, '');
      const head = line.match(/^---- (DROP|CLAMP|NOTE) \((\d+)\) ----$/);
      if (head) {
        cur = head[1];
        continue;
      }
      if (/^(===|转译完成|----)/.test(line)) {
        cur = null;
        continue;
      }
      if (!cur) continue;
      const item = line.match(/^\s{2}\[(.+?)\]\s(.*)$/);
      if (item) got[cur].push({ where: item[1], what: item[2] });
    }
    const counts = out.match(/丢失 (\d+) \/ 降级 (\d+) \/ 注意 (\d+)/);
    return { got, counts: counts ? { DROP: +counts[1], CLAMP: +counts[2], NOTE: +counts[3] } : null };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

/// 比对（纯函数：**自证直接测它** —— 一条不会红的门禁等于没有）。
function compare(got, baseline) {
  const failures = [];
  let checks = 0;
  for (const level of ['DROP', 'CLAMP']) {
    const want = baseline[level].map((x) => x.where);
    const have = got[level].map((x) => x.where);
    checks += 2;
    for (const w of have) if (!want.includes(w)) failures.push(`${level} 多了一条：${w}`);
    for (const w of want) if (!have.includes(w)) failures.push(`${level} 少了一条：${w}`);
  }
  return { failures, checks };
}

function selfTest() {
  let checks = 0;
  const eq = (a, b, what) => {
    checks += 1;
    assert.equal(a, b, what);
  };
  const base = { DROP: [{ where: 'a' }, { where: 'b' }], CLAMP: [{ where: 'c' }] };
  eq(compare(base, base).failures.length, 0, '清单一致时不该报错');
  const extra = { DROP: [{ where: 'a' }, { where: 'b' }, { where: '新丢的' }], CLAMP: [{ where: 'c' }] };
  eq(
    compare(extra, base).failures.some((f) => f.includes('多了一条：新丢的')),
    true,
    '多一条必须报「多了一条」（这是"某次改动把一档转丢了"的形状）',
  );
  const fewer = { DROP: [{ where: 'a' }], CLAMP: [{ where: 'c' }] };
  eq(
    compare(fewer, base).failures.some((f) => f.includes('少了一条：b')),
    true,
    '少一条也必须报（修好了要改基线；**报告被削弱**了更要查）',
  );
  console.log(`✓ 清单比对自证：${checks} 项（多一条 / 少一条都必须红）`);
}

if (process.argv.includes('--self-test')) {
  selfTest();
  process.exit(0);
}

const { got, counts } = collectIssues();
let checks = 0;
const failures = [];

for (const level of ['DROP', 'CLAMP']) {
  const want = BASELINE[level].map((x) => x.where);
  const have = got[level].map((x) => x.where);
  checks += 1;
  for (const w of have) {
    if (!want.includes(w)) {
      failures.push(`${level} 多了一条：${w}\n      内容：${got[level].find((x) => x.where === w).what.slice(0, 160)}`);
    }
  }
  checks += 1;
  for (const w of want) {
    if (!have.includes(w)) {
      failures.push(
        `${level} 少了一条：${w}\n` +
          `      基线记的理由：${BASELINE[level].find((x) => x.where === w).why}\n` +
          '      —— 要么是修好了（好事，把基线改掉），要么是**报告被削弱**（坏事，查为什么不再报）',
      );
    }
  }
  // 计数也要对得上：清单的"条数"与打印的统计同源，两个都变才说明真变了。
  checks += 1;
  if (counts && counts[level] !== got[level].length) {
    failures.push(`${level} 的统计(${counts[level]})与清单条数(${got[level].length})不一致 —— 报告自己矛盾了`);
  }
}

if (failures.length) {
  console.error('✗ 转译报告的 DROP/CLAMP 清单与基线不同：\n');
  for (const f of failures) console.error(`  - ${f}`);
  console.error(
    `\n  （现行清单：${got.DROP.length} DROP / ${got.CLAMP.length} CLAMP；` +
      `基线在 ${'scripts/check-dhampir-drop-list.mjs'} 顶部 ` +
      '—— 改它之前先确认"是新丢了一档，还是修好了一条"）',
  );
  process.exit(1);
}

console.log(
  `✓ DROP/CLAMP 清单与基线一致（${checks} 项）：` +
    `${got.DROP.length} DROP / ${got.CLAMP.length} CLAMP（各条理由写在脚本顶部）`,
);
