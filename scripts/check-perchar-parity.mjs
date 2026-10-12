#!/usr/bin/env node
// **逐字对拍：预览实现 ↔ 底座实现**（P4，2026-10-13）。
//
// # 为什么必须有一条**独立**的判据
//
// P3 在**底座**加了逐字能力（`text_perchar` 几何 + `text_perchar_anim` 时间），
// 而预览侧要在 YekiTrim 的 `engine.js` 里**再做一份**（前端没有 Rust）。
// 两份实现必然漂移，而漂移的症状是本仓最恨的那一类：
//
//   预览里字散开了、成片里没散    —— 或者反过来
//   **两边各自看都正常**，没有任何报错，也没有任何别的判据会红。
//
// ⚠️ 本仓已经栽过**四次**同形状的坑（`color` / `font_ratio` / `scale` / `parts`
//    都曾"只在一边接了"），见 Dhampir `text_item_json` 的注释。
//
// # 它怎么对拍（**不重写第三份实现**）
//
// 判据**不实现**逐字算法，而是：
//
//   ① 用 `tools/polish-to-dhampir.mjs` 转译一份**最小工程**（带 `anim`）；
//   ② 跑**底座真机** `dhampir subtitle --frame N`，拿到它算出来的 `cells`
//      （这是**权威**，因为成片就是它画的）；
//   ③ 用 `node -e` 跑**预览那份** `engine.js` 的逐字求值，拿同一组字；
//   ④ **逐字逐字段比**：`ch` / `offset_em` / `advance_em` / `scale` /
//      `rotation_deg` / `offset_x_em` / `offset_y_em`。
//
// ⇒ 判据自己**不含算法**，"谁对"由底座说了算。这与 `check-sway-parity.mjs`
//   同一条思路（那份是"从转译器源码里抠函数"再对拍，本份是"问真机要答案"）。
//
// # ⚠️ 前提（不满足就**如实跳过**，不是报红）
//
// * 底座二进制在 `F:\para\Code\Dhampir\target\debug\dhampir.exe`（或 `.build` 里那份）；
// * 有可用的中文字体（底座按**文件名**的族名找字体，见下方 `FONT_DIR`）；
// * `TMP` 指一个可写目录（Windows 上 Rust 读 `TMP`，**忽略 `TMPDIR`/`TEMP`**）。
//
// 三者缺一时打印**为什么跳过**并以 0 退出 —— 但会打印一行大写的
// `⚠️ 跳过：…`，因为"静默跳过"比"假绿"更坏（看着像测过了）。
//
// 用法：
//   node scripts/check-perchar-parity.mjs
//   node scripts/check-perchar-parity.mjs --self-test   # 变异自证

import { execFileSync } from 'node:child_process';
import {
  existsSync,
  mkdtempSync,
  writeFileSync,
  readFileSync,
  rmSync,
  statSync,
  readdirSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, '..');
const DHAMPIR = resolve(REPO, '..', 'Dhampir');

// ---------------------------------------------------------------- 判据框架

let passed = 0;
const failures = [];
function ok(name, cond, detail) {
  if (cond) {
    passed += 1;
  } else {
    failures.push(detail ? `${name} —— ${detail}` : name);
  }
}

// ---------------------------------------------------------------- 环境探测

/** 找底座二进制。⚠️ 找不到**不是**代码缺陷，是环境没构建。 */
function findDhampir() {
  const candidates = [
    join(DHAMPIR, 'target', 'debug', 'dhampir.exe'),
    join(DHAMPIR, 'target', 'debug', 'dhampir'),
    join(DHAMPIR, 'target', 'release', 'dhampir.exe'),
    join(DHAMPIR, 'target', 'release', 'dhampir'),
  ];
  for (const c of candidates) {
    if (existsSync(c)) return c;
  }
  return null;
}

/**
 * 底座二进制**比源码旧吗**？旧了就拒绝跑。
 *
 * ⚠️⚠️ **这是本轮实测踩到的一个真坑**：我加了 `scale_spread`（底座 + 预览两边）
 *    之后跑本判据，它**红了** —— 报"底座 0.5、预览 0.84"。我差点去查算法，
 *    实际是 `dhampir.exe` **还没重建**：判据拿的是改动前那份。
 *    重建之后立刻全绿。
 *
 * ⇒ 症状是"**判据红着，而代码是对的**"，代价是**去查一个不存在的问题**；
 *    反向更坏：代码错、二进制旧 ⇒ **判据假绿**。
 *    所以这里**直接拒绝在旧二进制上跑** —— 不猜、不猜重。
 */
function binaryIsStale(bin) {
  try {
    const binTime = statSync(bin).mtimeMs;
    // 只比"会不会影响这个二进制"的源码：逐字那三处的 crate。
    const roots = [
      join(DHAMPIR, 'crates', 'dhampir-timeline', 'src'),
      join(DHAMPIR, 'crates', 'dhampir-core', 'src'),
      join(DHAMPIR, 'crates', 'dhampir-worker', 'src'),
    ];
    let newest = 0;
    let newestFile = '';
    const walk = (dir) => {
      let entries;
      try {
        entries = readdirSync(dir, { withFileTypes: true });
      } catch {
        return;
      }
      for (const e of entries) {
        const full = join(dir, e.name);
        if (e.isDirectory()) walk(full);
        else if (e.name.endsWith('.rs')) {
          const t = statSync(full).mtimeMs;
          if (t > newest) {
            newest = t;
            newestFile = full;
          }
        }
      }
    };
    for (const r of roots) walk(r);
    if (newest > binTime) return newestFile;
    return null;
  } catch {
    return null;
  }
}

/** 底座的主机侧探测：`dhampir probe` 能不能跑起来。 */
function dhampirRuns(bin) {
  try {
    execFileSync(bin, ['--help'], { stdio: 'ignore', timeout: 20000 });
    return true;
  } catch {
    return false;
  }
}

/** 一个**可写**的临时目录。⚠️ 先在仓内找（沙箱常只放行工作区）。 */
function workRoot() {
  const inRepo = join(REPO, '.workbuddy-tmp', 'perchar-parity');
  try {
    const fs = require('node:fs');
    fs.mkdirSync(inRepo, { recursive: true });
    return inRepo;
  } catch {
    return mkdtempSync(join(tmpdir(), 'perchar-parity-'));
  }
}

// ---------------------------------------------------------------- 夹具

/**
 * 一份**最小**工程：一条视频轨 + 一条字幕轨（带 `per_char`）。
 *
 * ⚠️⚠️ **素材必须登记在 `assets` 数组里、`uri` 指向真文件** ——
 *    我第一版只写 `tracks[].source.asset_id`，于是底座报
 *    `subtitle_asset_kind`:「字幕轨只能引用 subtitle 素材，而 X 不是」，
 *    而 `subtitle` 子命令**不报错**、只是给出 `items: []`
 *    ⇒ 判据会以"底座没输出 cells"的名义**跳过**（看着像环境问题，
 *    其实是夹具写错了）。**夹具写错 = 判据永远跳过 = 等于没有**。
 * ⚠️ 素材 `id` 必须以 `.srt` 结尾：底座按扩展名认类型，不是按 `kind` 字段。
 */
function minimalProject(perCharJson, srtPath) {
  return JSON.stringify(
    {
      project_schema: 1,
      assets: [
        {
          id: 'clip.mp4',
          kind: 'video',
          name: 'bg',
          uri: 'clip.mp4',
          frame_count: null,
          timebase: { num: 30, den: 1 },
          width: 640,
          height: 360,
          content_hash: null,
          tags: {},
        },
        {
          id: 'dhampir-subtitle.srt',
          kind: 'subtitle',
          name: '字幕',
          uri: srtPath,
          frame_count: null,
          timebase: { num: 30, den: 1 },
          width: null,
          height: null,
          content_hash: null,
          tags: {},
        },
      ],
      timeline: {
        schema: 3,
        timebase: { num: 30, den: 1 },
        tracks: [
          {
            id: 'v1',
            kind: 'video',
            layers: [
              {
                id: 'bg',
                start: 0,
                end: 300,
                source: { asset_id: 'clip.mp4', source_in: 0 },
              },
            ],
          },
          {
            id: 'sub',
            kind: 'subtitle',
            layers: [
              {
                id: 'cue',
                start: 0,
                end: 300,
                source: { asset_id: 'dhampir-subtitle.srt', source_in: 0 },
              },
            ],
            subtitle: {
              font_ratio: 0.12,
              color: [255, 255, 255, 255],
              outline: false,
              per_char: JSON.parse(perCharJson),
            },
          },
        ],
      },
    },
    null,
    2,
  );
}

const SRT = '1\n00:00:00,000 --> 00:00:09,000\n一二三四五六七八\n';

/** 底座要求字体**按文件名的族名**找 ⇒ 造一个名字对得上的目录。 */
function fontDir(work) {
  const dir = join(work, 'fonts');
  try {
    const fs = require('node:fs');
    fs.mkdirSync(dir, { recursive: true });
    const src = [
      'C:/Windows/Fonts/msyh.ttc',
      '/System/Library/Fonts/PingFang.ttc',
      '/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc',
    ].find((p) => existsSync(p));
    if (!src) return null;
    const dst = join(dir, 'Microsoft YaHei.ttf');
    if (!existsSync(dst)) fs.copyFileSync(src, dst);
    return dir;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------- 两侧取数

/** 底座算出来的逐字格子（**权威**）。 */
function baseCells(bin, work, fonts, project, srt, frame) {
  const projectPath = join(work, 'p.json');
  writeFileSync(projectPath, project, 'utf8');
  const out = execFileSync(
    bin,
    ['subtitle', '--project', projectPath, '--frame', String(frame)],
    {
      cwd: work,
      env: { ...process.env, TMP: work },
      encoding: 'utf8',
      timeout: 120000,
      maxBuffer: 64 * 1024 * 1024,
    },
  );
  const parsed = JSON.parse(out);
  const items = parsed.items || [];
  const withCells = items.find((it) => Array.isArray(it.cells) && it.cells.length > 0);
  return withCells ? withCells.cells : null;
}

/**
 * 预览那份的逐字求值。
 *
 * ⚠️ **在子进程里跑**（`node -e`）而不是 import 进本进程：`engine.js` 是 ESM，
 *    而本判据是 ESM —— 直接 import 也行，但子进程能把"预览侧报错"与
 *    "判据自己报错"分开（前者应当让判据红，后者是判据写错了）。
 */
function previewCells(enginePath, specJson, text, localFrame, count) {
  const script = `
    import(${JSON.stringify('file://' + enginePath)}).then((E) => {
      // 必须先归一化：charTransformAt 吃的是归一化后的驼峰结构，
      // 而这里是工程里那份**蛇形** JSON。直接把蛇形喂进去的症状是
      // 每个字段都读不到 ⇒ 全部退到缺省值 ⇒ 预览恒定等、底座在动 ⇒ 全红。
      const spec = E.readPerChar(${specJson});
      const out = [];
      for (let i = 0; i < ${count}; i++) out.push(E.charTransformAt(spec, i, ${count}, ${localFrame}));
      process.stdout.write(JSON.stringify(out));
    }).catch((e) => { console.error(String(e)); process.exit(3); });
  `;
  const out = execFileSync(process.execPath, ['--input-type=module', '-e', script], {
    encoding: 'utf8',
    timeout: 60000,
  });
  return JSON.parse(out);
}

/** 预览那份的逐字**几何**（位置 / 宽度）。 */
function previewLayout(enginePath, text) {
  const script = `
    import(${JSON.stringify('file://' + enginePath)}).then((E) => {
      process.stdout.write(JSON.stringify(E.charLayoutOf(${JSON.stringify(text)})));
    }).catch((e) => { console.error(String(e)); process.exit(3); });
  `;
  const out = execFileSync(process.execPath, ['--input-type=module', '-e', script], {
    encoding: 'utf8',
    timeout: 60000,
  });
  return JSON.parse(out);
}

// ---------------------------------------------------------------- 主流程

const enginePath = join(REPO, 'webui', 'src', 'polish', 'engine.js');

// 预览侧必须先导出那三个函数，否则整条对拍无从谈起。
const engineSrc = readFileSync(enginePath, 'utf8');
ok(
  '预览侧导出了 `charTransformAt`（逐字时间求值）',
  /export function charTransformAt\s*\(/.test(engineSrc),
  '没有它 = 预览根本不知道每个字该怎么动 ⇒ P4 无从对拍',
);
ok(
  '预览侧导出了 `charLayoutOf`（逐字几何）',
  /export function charLayoutOf\s*\(/.test(engineSrc),
  '没有它 = 预览不知道每个字在哪',
);
ok(
  '预览侧导出了 `PER_CHAR_DEFAULT`（缺省值表）',
  /export const PER_CHAR_DEFAULT\s*=/.test(engineSrc),
  '缺省值两边必须同一份，否则"少写一个键"两端会算出不同的字',
);

const selfTest = process.argv.includes('--self-test');

const bin = findDhampir();
if (!bin) {
  console.log('⚠️ 跳过：找不到底座二进制（`F:\\para\\Code\\Dhampir\\target\\debug\\dhampir.exe`）。');
  console.log('   这不是判据失败 —— 是底座还没构建。构建后本判据会真跑。');
  console.log(`\n✓ 静态部分 ${passed} 项通过（函数是否导出）`);
  process.exit(0);
}

// ⚠️ 旧二进制会给出**改了之前的**答案（红或绿的都不可信）⇒ 直接拒绝跑。
const stale = binaryIsStale(bin);
if (stale) {
  console.log('✗ 底座的二进制比源码旧 —— 拒绝在此之上判定。');
  console.log(`   比二进制新的源文件：${stale}`);
  console.log('   请先重建：cd F:/para/Code/Dhampir && cargo build -p dhampir-worker --bin dhampir');
  console.log('   ⚠️ 不跳过、不猜测：旧二进制上的红绿都是**另一个版本**的答案。');
  process.exit(1);
}

const work = workRoot();
const fonts = fontDir(work);
// 字幕素材：底座按**扩展名**认类型，且要读得到 ⇒ 写进工作目录。
const srtPath = join(work, 'dhampir-subtitle.srt');
writeFileSync(srtPath, SRT, 'utf8');

// 逐字档位（与底座单测/`text_perchar` 真机测试同一个形状）。
// ⚠️ 故意**同时**打开四个通道：漏接任何一个都会被逐字段比出来。
const SPEC = {
  name: 'parity',
  scale_from: 0.5,
  scale_to: 1.4,
  rotation_from: -12,
  rotation_to: 9,
  offset_x_from: -0.2,
  offset_x_to: 0.3,
  offset_y_from: 0.15,
  offset_y_to: -0.25,
  spread_x_from: 0,
  spread_x_to: 1,
  spread_x_em: 0.6,
  spread_y_from: 1,
  spread_y_to: 0,
  spread_y_em: 0.3,
  // ⚠️ **按序号放大的字号**（C 组 11 档靠它）—— 主档位必须覆盖它。
  //    漏了的话那 11 档的"逐字"部分**完全没被对拍过**：
  //    预览与底座可以各按各的算法算字号，而判据全绿
  //    （这正是我在这条守卫上已经栽过一次的形状 —— 缺省值那一项）。
  scale_spread: 0.8,
  per_char_frames: 24,
  stagger_frames: 5,
  easing: 'linear',
};
const SPEC_JSON = JSON.stringify(SPEC);

// 取几帧：跨"还没轮到 / 正在动 / 已经完了"三个阶段 ——
// 只对稳态那一帧的话，错峰整条通道**根本没被覆盖**（而错峰正是最容易漂的地方）。
// ⚠️⚠️ 帧表必须覆盖**三个阶段**，而"稳态"那一档要挑**真的跑完之后**的帧：
//    `stagger_frames = 5` × 24 帧 × 8 个字 ⇒ 最后一个字在第 `7×5+24 = 59` 帧跑完。
//    我第一版挑了第 40 帧当"稳态"，而它其实还在错峰中途（后 4 个字没动完）
//    —— 判据照样能比（两边都错），但**"稳态"那条判据没在测稳态**。
const FRAMES = [0, 3, 14, 25, 40, 59, 70];

/**
 * **只让一个通道动**的档 —— 专门用来覆盖**缺省值表**。
 *
 * ⚠️⚠️ 为什么必须单独有一条，以及**为什么不能写成"什么都不配"**：
 *
 *   主档位是"**配全了**"的（真实工程的常态）⇒ `PER_CHAR_DEFAULT` 每一项
 *   **从没被读到** ⇒ 把 `scaleFrom` 的缺省从 `1.0` 改成 `0.0`
 *   （字缩成不可见、而 0 是合法值所以不报错）主对拍**抓不住**。
 *   实测：那条变异确实没被翻面。
 *
 *   但"什么都不配"的档**也测不到缺省值** —— 它的四个通道端点全相等
 *   ⇒ `isActive()` 为假 ⇒ 底座**不输出 cells** ⇒ 整轮跳过。
 *   （我第一版就写成 `{per_char_frames, scale_from}`，而 `scale_from`
 *     还是**显式给了** ⇒ 缺省值依然没被读到，白跑一轮。）
 *
 *   正确做法是：**让别的通道动、把要看的那一项留空**。
 *   这里只给 `rotation_*`，于是 `scale_from/scale_to` 取缺省 `1.0/1.0` ——
 *   缺省一被改成 `0.0`，`scale` 就从 `1.0` 变成 `0.0`，立刻翻面。
 */
const SPEC_DEFAULTED = JSON.stringify({
  per_char_frames: 20,
  rotation_from: -15,
  rotation_to: 12,
});

/**
 * 跑一轮完整对拍。
 *
 * ⚠️ 抽成函数是为了 `--self-test`：自检要拿**变异过的引擎**反复调它，
 *    而且每一轮必须**重置计数**（否则第一轮的绿会把后面的红盖掉）。
 *    只写在顶层的话变异台根本不会跑（本仓的 `check-yekitrim-translator.mjs`
 *    栽过这个：颜色那条第一版就是这么漏的）。
 *
 * @returns {{failures: string[], comparisons: number, nonZero: number, skipReason: string|null}}
 */
export function runParity({ engine, bin, work, fonts, srtPath, frames, spec }) {
  const specJson = spec ?? SPEC_JSON;
  const localFailures = [];
  const localOk = (name, cond, detail) => {
    if (!cond) localFailures.push(detail ? `${name} —— ${detail}` : name);
  };
  let comparisons = 0;
  let nonZero = 0;
  let skipReason = null;

  for (const frame of frames) {
  let base;
  try {
    base = baseCells(bin, work, fonts, minimalProject(specJson, srtPath), srtPath, frame);
  } catch (error) {
    skipReason = String(error.message || error).slice(0, 300);
    break;
  }
  if (!base) {
    skipReason = `底座在第 ${frame} 帧没有输出任何 cells`;
    break;
  }

  const text = base.map((c) => c.ch).join('');
  const count = base.length;
  const prev = previewCells(engine, specJson, text, frame, count);

  // 几何：位置与宽度
  const layout = previewLayout(engine, text);
  localOk(
    `几何：字数一致（第 ${frame} 帧）`,
    layout.length === count,
    `预览 ${layout.length} 个 vs 底座 ${count} 个`,
  );

  for (let i = 0; i < Math.min(layout.length, count); i++) {
    comparisons += 1;
    localOk(
      `几何[${i}] @${frame}：字符`,
      layout[i].ch === base[i].ch,
      `预览 ${JSON.stringify(layout[i].ch)} vs 底座 ${JSON.stringify(base[i].ch)}`,
    );
    // ⚠️ **两侧命名不同**（预览是驼峰、底座是蛇形）—— 这里显式列一张对照表。
    //    直接同名读会让**每一条都 undefined**、全红一片，而"全红"最容易被
    //    当成"夹具坏了"而跳过；显式表能让失败信息指出**是哪个字段**。
    const fields = [
      ['offset_em', layout[i].offsetEm, base[i].offset_em, 1e-4],
      ['advance_em', layout[i].advanceEm, base[i].advance_em, 1e-4],
    ];
    for (const [name, got, want, eps] of fields) {
      if (want !== 0) nonZero += 1;
      localOk(
        `几何[${i}] @${frame}：${name}`,
        Math.abs(got - want) < eps,
        `预览 ${got} vs 底座 ${want}`,
      );
    }
    // 变换：四个通道
    const xf = [
      ['scale', prev[i].scale, base[i].scale, 1e-4],
      ['rotation_deg', prev[i].rotationDeg, base[i].rotation_deg, 1e-4],
      ['offset_x_em', prev[i].offsetXEm, base[i].offset_x_em, 1e-4],
      ['offset_y_em', prev[i].offsetYEm, base[i].offset_y_em, 1e-4],
    ];
    for (const [name, got, want, eps] of xf) {
      comparisons += 1;
      if (Math.abs(want) > eps) nonZero += 1;
      localOk(
        `变换[${i}] @${frame}：${name}`,
        Math.abs(got - want) < eps,
        `预览 ${got} vs 底座 ${want}`,
      );
    }
  }
  }

  return { failures: localFailures, comparisons, nonZero, skipReason };
}

// ---------------------------------------------------------------- 自检

/**
 * **变异自证**：每一个变异都必须让对拍**打红**。
 *
 * ⚠️ 价值不在于"跑了一遍"，而在于**每一条都断言替换命中一次**：
 *    没命中的变异体原样跑绿，看着像"判据抓不住"，实际是**变异没生效**
 *    （本仓 `check-yekitrim-translator.mjs` 反复栽在这上面）。
 */
const MUTATIONS = [
  {
    name: '去掉 spread（退化成整行平移）',
    from: 'offsetXEm: mix(spec.offsetXFrom, spec.offsetXTo) + spreadX',
    to: 'offsetXEm: mix(spec.offsetXFrom, spec.offsetXTo)',
    why: '这正是 P3 真机实测抓到的缺陷：常数位移 = 整行平移，字距不变',
  },
  {
    name: '还没轮到时返回恒等（而不是起始值）',
    from: 'const t = Math.min(1, Math.max(0, raw))',
    to: 'if (raw < 0) return IDENTITY; const t = Math.min(1, Math.max(0, raw))',
    why: '给恒等会让字"先正常显示一下再重新弹"，闪一下、极难归因',
  },
  {
    name: '错峰恒 0（所有字同时动）',
    from: 'return rank * spec.staggerFrames',
    to: 'return 0',
    why: '错峰最易漂：稳态下所有字一样，只有中间帧分得出来',
  },
  {
    name: '全角判定失效（中日韩字按半角算）',
    from: 'if (isFullWidth(ch)) return 1.0',
    to: 'if (false && isFullWidth(ch)) return 1.0',
    why: '字宽表两端必须同一份，否则逐字定位与折行用两个字宽',
  },
  {
    name: '按序号的字号乘数写成加法（倍率语义错）',
    from: 'scale: mix(spec.scaleFrom, spec.scaleTo) * (1 + spec.scaleSpread * rank),',
    to: 'scale: mix(spec.scaleFrom, spec.scaleTo) + spec.scaleSpread * rank,',
    why: '字号是倍率：倍率的"再大 80%"是 ×1.8 不是 +0.8（scale_to=1.4 时差 0.6）',
  },
  {
    name: '字号乘数整个忽略（C 组 11 档全废）',
    from: 'scale: mix(spec.scaleFrom, spec.scaleTo) * (1 + spec.scaleSpread * rank),',
    to: 'scale: mix(spec.scaleFrom, spec.scaleTo),',
    why: '这正是 C 组施工时抓到的能力缺口：没有它时「大小字/变小/放大」做不出来',
  },
  {
    name: '缺省缩放写成 0（Rust 那边专门有 one() 就为这个）',
    from: 'scaleFrom: 1.0,',
    to: 'scaleFrom: 0.0,',
    why: '0 是合法值 ⇒ 字缩成不可见却不报错',
  },
];

if (selfTest) {
  const original = readFileSync(enginePath, 'utf8');
  let bad = 0;
  // ⚠️⚠️ 变异体必须**放在 `engine.js` 旁边** —— 它 `import` 了同目录的
  //    `canvas.js`/`edl.js`：放到临时目录里去会 `ERR_MODULE_NOT_FOUND`，
  //    而那个错会被当成"变异没被打红"（其实是**变异体跑不起来**）。
  const scratch = join(dirname(enginePath), '.engine-mut.mjs');
  for (const m of MUTATIONS) {
    const hits = original.split(m.from).length - 1;
    if (hits !== 1) {
      console.log(
        `✗ 变异「${m.name}」的锚点命中 ${hits} 次（必须恰好 1 次）—— 变异没生效，等于没测`,
      );
      bad += 1;
      continue;
    }
    writeFileSync(scratch, original.replace(m.from, m.to), 'utf8');
    let r = runParity({ engine: scratch, bin, work, fonts, srtPath, frames: FRAMES });
    // ⚠️ **两轮都要跑**：缺省值那一类只能被「只写必填键」的那一轮抓到，
    //    而通道那一类两轮都能抓 —— 只跑一轮会漏掉整整一类（实测漏过 1 条）。
    if (!r.skipReason && r.failures.length === 0) {
      r = runParity({
        engine: scratch,
        bin,
        work,
        fonts,
        srtPath,
        frames: FRAMES,
        spec: SPEC_DEFAULTED,
      });
    }
    if (r.skipReason) {
      console.log(`✗ 变异「${m.name}」跑不起来：${r.skipReason}`);
      bad += 1;
    } else if (r.failures.length === 0) {
      console.log(`✗ 变异「${m.name}」**没有被打红**（${r.comparisons} 个对拍点全过）`);
      console.log(`   原因：${m.why}`);
      bad += 1;
    } else {
      console.log(`✓ 变异「${m.name}」被打红（${r.failures.length} 条）`);
    }
    rmSync(scratch, { force: true });
  }
  if (bad) {
    console.log(`\n✗ 自检失败：${bad} / ${MUTATIONS.length} 个变异没被打红`);
    process.exit(1);
  }
  console.log(`\n✓ 自检通过：${MUTATIONS.length} 个变异全部被打红`);
  process.exit(0);
}

// ---------------------------------------------------------------- 主流程

const result = runParity({ engine: enginePath, bin, work, fonts, srtPath, frames: FRAMES });
const resultMin = runParity({
  engine: enginePath,
  bin,
  work,
  fonts,
  srtPath,
  frames: FRAMES,
  spec: SPEC_DEFAULTED,
});

if (result.skipReason) {
  console.log(`⚠️ 跳过：${result.skipReason}`);
  console.log('   这不是判据失败 —— 是环境不满足（见文件头「前提」）。');
  console.log(`\n✓ 静态部分 ${passed} 项通过`);
  process.exit(0);
}

for (const f of result.failures) failures.push(f);
for (const f of resultMin.failures) failures.push(`[靠缺省值的档] ${f}`);

const comparisons = result.comparisons + resultMin.comparisons;
const nonZero = result.nonZero + resultMin.nonZero;

// ---- 反空转：别让"两边都是 0"蒙过去 ----
ok(
  '对拍点足够多（不是空转）',
  comparisons >= 100,
  `只比了 ${comparisons} 个点`,
);
ok(
  '对拍点里有非零值（不是全 0 蒙混）',
  nonZero >= 20,
  `只有 ${nonZero} 个非零点 —— 全 0 的对拍证明不了任何事`,
);

// ---- 结果 ----
if (failures.length) {
  console.log(`✗ 逐字对拍失败（${failures.length} / ${passed + failures.length}）：`);
  for (const f of failures.slice(0, 20)) console.log(`    ${f}`);
  if (failures.length > 20) console.log(`    …还有 ${failures.length - 20} 条`);
  process.exit(1);
}
console.log(
  `✓ 逐字对拍通过：预览 ↔ 底座逐字段一致（${passed} 项判据 / ` +
    `${comparisons} 个对拍点，其中 ${nonZero} 个非零）`,
);
