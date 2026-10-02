// V-Trim ↔ Dhampir 逐步对比：**一步**跑完整条回路，并按窗口出表。
//
// # 这个脚本存在的理由：它把两个已经踩过的坑堵死了
//
// 我在手工搭这条回路时犯了两个错，**两次都产出了"看起来有据"的假结论**：
//
//   甲、**把源时间当输出时间取样。**
//       V-Trim 的 `polish.toml` 里事件时间是**源时间**，而 `mute_ranges` 会把中间
//       几段**掐掉**（本样例共 17.50 秒）。所以同一个事件在成片里出现的位置是
//       `源时间 - 被掐掉的长度`。我按源时间直接去 `-ss` 取帧，
//       于是量的是**完全不相干的时刻** —— 第一批结论（"blur 很好、flash 很差"）
//       整个是反的。
//       -> 本脚本**先算映射再取样**，并把映射表打出来。
//
//   乙、**在结论写完之前就把产物清理了。**
//       我第二次追一个"21.83 的差异"，回头要复核时发现基准文件已经被自己删了。
//       -> 本脚本把两边成片留在 `--out` 目录里，**报告写完才提示可以删**；
//          不传 `--clean` 就不动它。
//
// 另外它还有一条纪律：**用户工程必须恢复**。整个流程包在 try/finally 里，
// 无论中途哪一步失败（V-Trim 崩了、渲染失败、Ctrl-C），`polish.toml` 都还原。
//
// # 用法（本脚本住在 **V-Trim 仓**：`scripts/vtrim-compare.mjs`）
//
//     node scripts/vtrim-compare.mjs \
//       --clip "E:/media/<项目>/output/clips/<片段>" \
//       --events target/fx-events.json \
//       --out target/vtrim-run
//
// `--events` 是一个 JSON 数组，描述**要插进 polish.toml 的测试事件**（源时间）：
//
//     [
//       { "type": "blur",    "time": 66.2 },
//       { "type": "noise", "time": 68.2 },
//       { "type": "shake",   "time": 70.2, "intensity": 20 }
//     ]
//
// 不传 `--events` 就是"不插任何东西，只比现有工程"（那是**基线**，
// 用来确认两边本来就一致）。
//
// # 它怎么判
//
// 对每个事件，在**输出时间轴**上取它的窗口，量三样：
//
//   * **像素差**：整帧平均绝对差。要跟**窗口外的基线**比，不是跟 0 比 ——
//     字幕的字形栅格化（Chrome/Skia vs ffmpeg/FreeType）本身就带来 5~13 的底噪。
//   * **高频比**：本仓的拉普拉斯能量 / 参照的。`1.0` = 同锐利。
//     这个量能把"糊"和"锯齿"分开：**码率不够会变低，点采样会变高**。
//   * **最佳纵向位移**：能让差异最小的那个像素位移。`+0` 之外的值
//     基本都指向"位置不对"而不是"效果不对"。
//
// # 环境（本脚本住在 V-Trim 仓，`REPO_ROOT` = V-Trim 仓根）
//
// * `ffmpeg` / `ffprobe` 在 PATH 上
// * Dhampir 的可执行（`--dhampir` / `VTEDIT_DHAMPIR`，默认找同层 `Dhampir/target/{debug,release}`）
// * `yekitrim.exe`（`--vtrim` / `VTRIM_EXE`，默认 `.workbuddy/perf/bin/`，再退 `target/`，最后 PATH）
// * 转译器（`--translator` / `VTRIM_TRANSLATOR`，默认 `tools/polish-to-dhampir.mjs`）——
//   它就在**本仓**，见下面"转译器住在哪"
//
// **底座（Dhampir）是另一个仓库**：只调二进制、不链库，契约见 `docs/dhampir/compare-loop.md`
// 与 `docs/dhampir/README.md`。

import { execFileSync } from 'node:child_process';
import {
  copyFileSync,
  existsSync,
  linkSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO_ROOT = resolve(new URL('..', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1'));

// ---------------------------------------------------------------------------
// 参数
// ---------------------------------------------------------------------------

function parseArgs(argv) {
  const out = { events: null, clean: false, sampleStep: 0.05, fixture: null, allFixtures: false };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === '--clip') out.clip = argv[++i];
    else if (a === '--events') out.events = argv[++i];
    else if (a === '--out') out.out = argv[++i];
    else if (a === '--dhampir') out.dhampir = argv[++i];
    else if (a === '--vtrim') out.vtrim = argv[++i];
    else if (a === '--translator') out.translator = argv[++i];
    else if (a === '--font-file') out.fontFile = argv[++i];
    else if (a === '--fixture') out.fixture = argv[++i];
    else if (a === '--all-fixtures') out.allFixtures = true;
    // 阈值覆盖：调参用（先跑 `--baseline-pixel-max 999` 读基线，再把它写进常量）。
    // 它**也是"这条断言真的会红"的自证手段** —— 传一个不可能达到的值（`--baseline-pixel-max 1`），
    // 整条回路必须 `exit 1`。
    else if (a === '--baseline-pixel-max') out.baselinePixelMax = Number(argv[++i]);
    else if (a === '--no-mask-danmaku') out.noMaskDanmaku = true;
    // 黄金帧（2026-10-01）：把"参照"从"现场渲染兜底腿"改成"冻结的小图"。
    // 删掉第二条链路（兜底腿）的前置条件④要求"删掉它不影响基线"，而现在的参照就是它自己 ——
    // 所以先冻结一份，之后没有兜底腿也能验回归。
    else if (a === '--capture-golden') out.captureGolden = argv[++i];
    else if (a === '--golden') out.golden = argv[++i];
    else if (a === '--clean') out.clean = true;
    else if (a === '--self-test') out.selfTest = true;
    else if (a === '--help' || a === '-h') out.help = true;
    else throw new Error(`不认识的参数：${a}`);
  }
  return out;
}

const USAGE = `用法：node scripts/vtrim-compare.mjs --clip <clip 目录> [选项]
       node scripts/vtrim-compare.mjs --fixture <名字> | --all-fixtures

  --clip <目录>         V-Trim 的片段目录（里面有 polish.toml / clip.mp4）
  --fixture <名字>      用固定样本（demo / portrait / real）——**会先复制到运行目录**，
                        绝不动样本本体；缺样本时明确跳过并说清要准备什么
  --all-fixtures        依次跑全部固定样本
  --events <json>       要插进去的测试事件（JSON 数组，源时间）。不给 = 只比现有工程
  --out <目录>          运行目录（默认 target/vtrim-compare）
  --translator <路径>   转译器（默认 tools/polish-to-dhampir.mjs；
                        也可用环境变量 VTRIM_TRANSLATOR）
  --dhampir <路径>      Dhampir 可执行（默认找同层 Dhampir/target/{debug,release}；
                        也可用环境变量 VTEDIT_DHAMPIR）
  --vtrim <路径>        vtrim 可执行（默认自动找）
  --font-file <路径>    字幕字体（默认 C:/Windows/Fonts/msyh.ttc）
  --baseline-pixel-max <n>  覆盖"基线像素差"阈值（调参 / 自证断言会红）
  --no-mask-danmaku     不屏蔽弹幕带（默认屏蔽左带 + 上带，理由见文件头"弹幕按设计"）
  --capture-golden <目录>  跑常规回路并**冻结**基线采样帧（<目录>/<样本>/ref-<t>.jpg + golden.json）；
                          **删掉兜底腿之前跑一次**，之后就不用再渲染它了
  --golden <目录>          参照只读冻结帧：**完全不渲染兜底腿**（只做基线对照，不许配 --events）
  --clean               跑完删掉运行目录（**默认保留**，见文件头"坑乙"）
  --self-test           只跑自检（映射 / 插事件 / 屏蔽带 / 阈值判定，不碰工程）
`;

// ---------------------------------------------------------------------------
// 固定样本（M1）：**只在这里列**，跑之前一律复制到运行目录
// ---------------------------------------------------------------------------
//
// 为什么必须复制：这条工具会**改工程的 `polish.toml`**（插测试事件）并往片段目录写产物。
// 拿仓库里的样本直接跑 = 把 `docs/` 里的演示工程改脏（虽然它 `finally` 会还原，
// 但"样本本体永不被写"这条不变量比"相信 finally"更值钱）。
//
// `mute` 那两段是**故意**加的：屏蔽区间是出过两次事故的地方（剪两次 / 没剪），
// 每个样本都带上它，回归网才盖得住。
/// 给样本注入的两段屏蔽区间（源时间）。
///
/// ⚠️ **必须写成 `[[mute_ranges]]` 数组表**，不能写成行内表
/// （`mute_ranges = [{ start = 2.0, … }]`）：V-Trim 自己序列化出来就是数组表，
/// 而转译器的 TOML 解析器**不支持行内表**（它先按逗号切数组元素 → 拿到 `{ start = 2.0` 就抛
/// "不认识的 TOML 值"）。第一版就是行内表：工具先报 `mute_ranges（无）`、随后转译直接崩。
const FIXTURE_MUTE_PAIRS = [
  [2.0, 3.0],
  [7.0, 9.0],
];
const FIXTURE_MUTES = FIXTURE_MUTE_PAIRS.map(
  ([start, end]) => `[[mute_ranges]]\nstart = ${start.toFixed(1)}\nend = ${end.toFixed(1)}\n`,
).join('\n');
const FIXTURES = {
  demo: {
    source: 'docs/wgpu-effect-reference/demo',
    note: '仓库自带的 12s 演示工程（最快；+2 段屏蔽区间）',
    media: ['clip.mp4'],
    patch: (toml) => withMutes(toml),
  },
  portrait: {
    source: 'docs/wgpu-effect-reference/demo',
    note: '同演示工程，但画布改成竖屏（+2 段屏蔽区间）——竖素材那条运镜门就在这里量',
    media: ['clip.mp4'],
    patch: (toml) => setTomlKey(withMutes(toml), 'canvas', 'orientation', '"portrait"'),
  },
  real: {
    source: '.workbuddy/perf/realproj/output/clips/clip-08',
    note: '真实工程（长片；只有本机有这个目录 —— 缺了会明确跳过）',
    media: ['clip.mp4'],
    patch: (toml) => toml,
  },
};

/// 在某个 `[table]` 下设一个键（有就换、没有就插；整张表都没有就在**文件末尾**追加）。
///
/// ⚠️ 为什么不能只写 `.replace(/^orientation\s*=.*$/m, …)`：**原始工程里常常没有这个键**
/// （演示工程就是这样 —— `[canvas]` 是 `process` 后处理时补上的），于是替换静默不命中，
/// 竖屏样本跑出来还是横屏、数字与横屏一模一样（第一次跑就是这么被骗过的）。
function setTomlKey(toml, table, key, valueLiteral) {
  const lines = toml.split(/\r?\n/);
  const head = `[${table}]`;
  const at = lines.findIndex((l) => l.trim() === head);
  if (at < 0) {
    return `${toml.replace(/\s*$/, '')}\n\n${head}\n${key} = ${valueLiteral}\n`;
  }
  for (let i = at + 1; i < lines.length; i += 1) {
    if (lines[i].trimStart().startsWith('[')) break; // 出了这张表
    if (new RegExp(`^\\s*${key}\\s*=`).test(lines[i])) {
      lines[i] = `${key} = ${valueLiteral}`;
      return lines.join('\n');
    }
  }
  lines.splice(at + 1, 0, `${key} = ${valueLiteral}`);
  return lines.join('\n');
}

/// 而 `ProjectFile.mute_ranges` 是顶层字段 —— 这一条踩过，写错了表现为"屏蔽区间根本没生效"）。
function withMutes(toml) {
  const lines = toml.split(/\r?\n/);
  // 已有的（无论 `mute_ranges = …` 还是 `[[mute_ranges]]` 块）整块删掉，避免两份。
  const kept = [];
  for (let i = 0; i < lines.length; i += 1) {
    if (/^mute_ranges\s*=/.test(lines[i])) continue;
    if (/^\[\[mute_ranges\]\]\s*$/.test(lines[i])) {
      // 连带它的键值行一起吞掉（到下一个 `[` 或空行块结束）
      let j = i + 1;
      while (j < lines.length && !/^\s*\[/.test(lines[j]) && !/^\s*$/.test(lines[j])) j += 1;
      i = j - 1;
      continue;
    }
    kept.push(lines[i]);
  }
  const firstSection = kept.findIndex((l) => l.trimStart().startsWith('['));
  const at = firstSection < 0 ? kept.length : firstSection;
  kept.splice(at, 0, FIXTURE_MUTES);
  return kept.join('\n');
}

/// 准备一个样本的副本。`media` 里的文件用**硬链接**（同一卷才成，失败就退回复制）——
/// 真实工程的 `clip.mp4` 有几百 MB，复制一遍纯属浪费。
function prepareFixture(fx, destDir) {
  const src = resolve(fx.source);
  if (!existsSync(join(src, 'polish.toml'))) {
    throw new Error(
      `样本缺源：${fx.source}\n  要准备什么：${fx.note}\n  （演示样本随仓库；真实样本只在本机）`,
    );
  }
  rmSync(destDir, { recursive: true, force: true });
  mkdirSync(destDir, { recursive: true });
  for (const name of readdirSync(src)) {
    const from = join(src, name);
    const to = join(destDir, name);
    if (fx.media.includes(name)) {
      try {
        linkSync(from, to);
      } catch {
        copyFileSync(from, to);
      }
      continue;
    }
    // `data/` 是后处理缓存（`polish render` 会先跑 process 重建），不必带 —— 省时间也省空间。
    // ⚠️ 只跳 `data/`：`sfx/` 可能被 `polish.toml` 引用（跳了就是"音效静默消失"）。
    if (name === 'data') continue;
    if (name.endsWith('.mp4') || name.endsWith('.mkv') || name.endsWith('.flv')) continue;
    const st = statSync(from);
    if (st.isDirectory()) copyDir(from, to);
    else copyFileSync(from, to);
  }
  const tomlPath = join(destDir, 'polish.toml');
  writeFileSync(tomlPath, fx.patch(readFileSync(tomlPath, 'utf8')), 'utf8');
  return destDir;
}

function copyDir(from, to) {
  mkdirSync(to, { recursive: true });
  for (const name of readdirSync(from)) {
    const f = join(from, name);
    const t = join(to, name);
    if (statSync(f).isDirectory()) copyDir(f, t);
    else copyFileSync(f, t);
  }
}


const log = (msg) => console.log(msg);
const step = (msg) => console.log(`\n=== ${msg} ===`);

// ---------------------------------------------------------------------------
// 判据：阈值 + 弹幕带屏蔽（2026-09-30 M1）
// ---------------------------------------------------------------------------
//
// # 为什么要有"弹幕带"
//
// **弹幕排布按底座当前的设计**（用户 2026-09-30 拍板：底座画成头像 + 用户名列表，
// 不再往参照的滚动弹幕上对）。所以那一块**按定义就是不一样的**：底座占画面左侧一条竖列
// （实测 x < ~0.20W、y 到 ~0.55H），参照则是横穿顶部的滚动弹幕。
// 拿它进阈值 = 每次都被"按设计的差异"判红，阈值就废了。
//
// 所以默认**屏蔽左带 + 上带**（L 形），只比中间与下半幅（画面、字幕、特效都在那儿）。
// 不屏蔽时（`--no-mask-danmaku`）照常报数，只是不进判据 ——
// **弹幕区的差异仍然打印出来**（`pixelFull`），不许因为"屏蔽了"就看不见。
const MASK = {
  danmakuLeftRatio: 0.22, // 左带宽度（底座的弹幕列）
  danmakuTopRatio: 0.25, // 上带高度（参照的滚动弹幕主要在这条带里）
};
/// 屏蔽矩形（像素）—— 现在是两条带，写成数组是为了让 `--no-mask-danmaku` 与自检都只看这一处。
function maskRectsFor(width, height, enabled = true) {
  if (!enabled) return [];
  return [
    { x: 0, y: 0, w: Math.round(width * MASK.danmakuLeftRatio), h: height },
    { x: 0, y: 0, w: width, h: Math.round(height * MASK.danmakuTopRatio) },
  ];
}

/// 判据（纯函数，**自检直接测它** —— "一条不会红的断言等于没断言"）。
///
/// 输入：基线（窗口外）的像素差 / 位移、以及各特效窗口的汇总；输出：失败原因列表。
/// 三条硬判据 + 一条信息项：
///   1. **时长必须一致**（两条腿的成片时长差 ≤ `durationDeltaSec`）——
///      这条是拿真事故换来的：转译器掐一遍 `mute_ranges`、出片尾部再剪一遍 ⇒ 成片短一大截，
///      而当时所有"像素/位移"判据都还在正常范围内（因为两边都是"自己的成片"）。
///   2. 基线像素差 ≤ 阈值（说明两条腿在"什么都没发生"的时候是一致的）。
///   3. 基线位移 ≤ `maxDy`（非 0 基本就是几何差，不是效果差）。
///   4. 每个特效窗口：中位像素差不得高于邻居中位差 × `windowFactor` + `windowAbs`（老判据，保留）。
const THRESHOLDS = {
  // 标定（2026-09-30，演示样本 + 屏蔽弹幕带）：
  //   基线中位像素差 **15.07**（逐点 3.0 / 6.5 / 15.1 / 60.0 / 77.6 —— 大数是**剪切帧**，
  //   演示工程是快切特效片；同一个 `dy=0` 说明那不是几何差），位移 **0px**，时长差 **0.000s**。
  //   阈值取"中位 × 1.3 左右"：**够松到不误报剪帧、够紧到基线真退化时会红**
  //   （退化到 30+ 必红 —— 而 30 正是"采样点落在剪点上"那种系统性错误的量级）。
  // ⚠️ **基线阈值要以 `real` 样本为准**（真实长片剪切少、基线应该在 7 上下，见文件头"坑"）：
  //   演示样本的 15 是"快切 + 中位数"的产物，不是两条腿的真实底噪。
  baselinePixel: 20,
  maxDy: 4,
  windowFactor: 1.5,
  windowAbs: 2.0,
  durationDeltaSec: 0.2,
};

function judge(measured) {
  const failures = [];
  const t = { ...THRESHOLDS, ...(measured.overrides ?? {}) };
  if (Math.abs(measured.refDuration - measured.mineDuration) > t.durationDeltaSec) {
    failures.push(
      `时长不一致：参照 ${measured.refDuration.toFixed(3)}s vs 本仓 ${measured.mineDuration.toFixed(3)}s` +
        `（差 ${Math.abs(measured.refDuration - measured.mineDuration).toFixed(3)}s > ${t.durationDeltaSec}s）——` +
        ` 先查「屏蔽区间是不是剪了两次/没剪」（--keep-muted 那条腿契约）`,
    );
  }
  if (measured.basePixel > t.baselinePixel) {
    failures.push(
      `基线像素差 ${measured.basePixel.toFixed(2)} > ${t.baselinePixel}` +
        `（窗口外的底噪本身变大了：字形/几何/弹幕带屏蔽口径都可能）`,
    );
  }
  if (Math.abs(measured.baseDy) > t.maxDy) {
    failures.push(`基线位移 ${measured.baseDy}px > ${t.maxDy}px（几何差，不是效果差）`);
  }
  for (const w of measured.windows ?? []) {
    if (w.pixel > w.neighbour * t.windowFactor + t.windowAbs) {
      failures.push(
        `${w.group}：窗口像素差 ${w.pixel.toFixed(2)} 高于邻居 ${w.neighbour.toFixed(2)}` +
          `（判据 ${w.neighbour.toFixed(2)} × ${t.windowFactor} + ${t.windowAbs}）`,
      );
    }
  }
  return failures;
}


function run(cmd, args, opts = {}) {
  return execFileSync(cmd, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], ...opts });
}

/// ffprobe 的 `-of default=nw=1` 是 key=value —— **不要用 `csv=p=0`**：
/// 它是位置式的，字段一多一少就静默串位（本项目已因它错过三次）。
function probeVideo(path) {
  const text = run('ffprobe', [
    '-v', 'error', '-select_streams', 'v',
    '-show_entries', 'stream=nb_frames,r_frame_rate,duration,width,height',
    '-of', 'default=nw=1', path,
  ]);
  const fields = {};
  for (const line of text.split('\n')) {
    const at = line.indexOf('=');
    if (at > 0) fields[line.slice(0, at)] = line.slice(at + 1).trim();
  }
  const [num, den] = (fields.r_frame_rate || '0/1').split('/').map(Number);
  return {
    frames: Number(fields.nb_frames),
    fps: den ? num / den : 0,
    duration: Number(fields.duration),
    width: Number(fields.width),
    height: Number(fields.height),
  };
}

// ---------------------------------------------------------------------------
// polish.toml：极简读 + 单点写
// ---------------------------------------------------------------------------

/// 读出 `[[mute_ranges]]` 的区间（**源秒**）。这些段是**掐掉**而不是静音。
function readMuteRanges(toml) {
  const ranges = [];
  const re = /\[\[mute_ranges\]\]([\s\S]*?)(?=\n\[|$)/g;
  let m;
  while ((m = re.exec(toml)) !== null) {
    const start = /start\s*=\s*([\d.]+)/.exec(m[1]);
    const end = /end\s*=\s*([\d.]+)/.exec(m[1]);
    if (start && end) ranges.push([Number(start[1]), Number(end[1])]);
  }
  return ranges.sort((a, b) => a[0] - b[0]);
}

/// **源秒 -> 输出秒。** 减去在它之前被掐掉的总长度。
///
/// 这就是"坑甲"的那个函数。手工搭回路时我漏了它，于是量错了时刻。
function makeRemap(muteRanges) {
  return (sourceSec) => {
    let cut = 0;
    for (const [a, b] of muteRanges) {
      if (sourceSec <= a) break;
      cut += Math.min(sourceSec, b) - a;
    }
    return sourceSec - cut;
  };
}

/// 把测试事件插进**覆盖它那个源时间**的 segment 里。
///
/// 返回 `{ text, insertedAt }`。找不到就抛 —— **不静默丢**。
function insertEvents(toml, events) {
  const lines = toml.split('\n');
  // 每个 [[segments]] 的起止行与 start/end
  const segments = [];
  for (let i = 0; i < lines.length; i += 1) {
    if (lines[i].trim() !== '[[segments]]') continue;
    let start = null;
    let end = null;
    let stop = lines.length;
    for (let j = i + 1; j < lines.length; j += 1) {
      if (lines[j].trim() === '[[segments]]') { stop = j; break; }
      const s = lines[j].trim();
      const ms = /^start\s*=\s*([\d.]+)/.exec(s);
      const me = /^end\s*=\s*([\d.]+)/.exec(s);
      if (ms) start = Number(ms[1]);
      if (me) end = Number(me[1]);
    }
    segments.push({ head: i, stop, start, end });
  }
  // 从后往前插，行号才不会位移
  const sorted = [...events].sort((a, b) => b.time - a.time);
  const at = [];
  for (const ev of sorted) {
    const seg = segments.find((s) => s.start !== null && s.end !== null && ev.time >= s.start && ev.time < s.end);
    if (!seg) throw new Error(`事件 ${ev.type}@${ev.time}s 不落在任何 segment 里（源时间 ${segments.map((s) => `${s.start}-${s.end}`).join(', ')}）`);
    const body = [`[[segments.events]]`, `type = "${ev.type}"`, `time = ${ev.time}`];
    for (const [k, v] of Object.entries(ev)) {
      if (k === 'type' || k === 'time') continue;
      body.push(typeof v === 'string' ? `${k} = "${v}"` : `${k} = ${v}`);
    }
    body.push('');
    lines.splice(seg.stop, 0, ...body);
    at.push({ type: ev.type, sourceSec: ev.time, segment: `${seg.start}-${seg.end}` });
    // 插完之后各 segment 的行号都变了 -> 重新解析（事件不多，代价可忽略）
    return { text: lines.join('\n'), insertedAt: at };
  }
  return { text: toml, insertedAt: at };
}

// 逐条插（每条都重新解析一次行号，避免"插完一次后行号全错"）
function insertEventsOneByOne(toml, events) {
  let text = toml;
  const at = [];
  for (const ev of events) {
    const r = insertEvents(text, [ev]);
    text = r.text;
    at.push(...r.insertedAt);
  }
  return { text, insertedAt: at };
}

// ---------------------------------------------------------------------------
// 取帧与度量
// ---------------------------------------------------------------------------

/// 取一帧。**取不到就抛** —— 不要留一个"文件不存在"给下游去猜。
///
/// ffmpeg 在 `-ss` 超出片长时可能**退 0 但不写文件**，于是下游会拿到一个
/// 莫名其妙的 `FileNotFoundError`（我第一次真跑就是这样，报的是 Python 读不到 PNG，
/// 而真正的原因是"本仓只渲了 48 秒"）。
function grab(video, seconds, outPath) {
  try {
    run('ffmpeg', ['-v', 'error', '-ss', String(seconds), '-i', video, '-frames:v', '1', '-y', outPath]);
  } catch (error) {
    throw new Error(`取帧失败 ${video} @${seconds}s：${String(error.stderr || error.message).slice(0, 200)}`);
  }
  if (!existsSync(outPath)) {
    const info = probeVideo(video);
    throw new Error(
      `取帧没产出文件：${video} @${seconds}s（该视频只有 ${info.duration.toFixed(2)}s / ${info.frames} 帧）`,
    );
  }
  return outPath;
}

/// 度量交给一个 Python 片段（numpy 读 PNG 最省事；本仓的 scripts 一律零 npm 依赖）。
///
/// **每次调用只跑一次 Python、把整张表算完** —— 逐帧起进程会让整个回路慢一个数量级。
function measure(plan) {
  const py = `
import json, sys
import numpy as np
from PIL import Image
from numpy.lib.stride_tricks import sliding_window_view
plan = json.load(open(sys.argv[1], encoding='utf-8'))
rows = plan['rows'] if isinstance(plan, dict) else plan
rects = (plan.get('maskRects') if isinstance(plan, dict) else None) or []
def L(p):
    return np.asarray(Image.open(p).convert('RGB')).astype(float)
def mask(a):
    # 屏蔽带（弹幕区）置 False —— **两边的图都用同一张掩码**，否则比的是"掩码差"。
    m = np.ones(a.shape[:2], bool)
    for r in rects:
        m[r['y']:r['y']+r['h'], r['x']:r['x']+r['w']] = False
    return m
def lap(a, m):
    # 高频只看**中段**（上下各留 200 行避开字幕与黑边），x 也避开弹幕带。
    g = a[200:900].mean(axis=2)
    mm = m[200:900]
    k = np.array([[0,1,0],[1,-4,1],[0,1,0]], float)
    v = (sliding_window_view(g,(3,3))*k).sum(axis=(2,3))
    # 掩码里被屏蔽的点必须从方差里剔掉 —— 直接 np.var 会把"掩码边缘"当成高频。
    return float(v[mm[1:-1,1:-1]].var()) if v.any() else 0.0
def best_dy(a, r, m):
    # 返回 (最佳残差, 最佳位移, **位移 0 的残差**)。
    # ⚠️ 第三个值是 2026-10-01 加的：**目标函数平坦时"最佳位移"是噪声** ——
    # 竖屏样本上实测：最佳位移与 0 位移的残差只差 0.00~0.05dB（PSNR 口径），
    # 却让判据报出"基线位移 -10px（几何差，不是效果差）"。够不成识别的位移不许当几何结论，
    # 所以把 dy=0 的残差一起带回来，由 JS 侧的 identifiableDy 决定算不算数。
    A, R, M = a[200:900], r[200:900], m[200:900]
    best = None
    zero = None
    for dy in range(-24, 25):
        d = np.abs(np.roll(A, dy, axis=0)[24:-24] - R[24:-24])
        mm = M[24:-24]
        sc = float(d[mm].mean()) if mm.any() else float(d.mean())
        if dy == 0: zero = sc
        if best is None or sc < best[0]: best = (sc, dy)
    return best[0], best[1], zero
def score(a, r, m):
    return float(np.abs(a-r)[m].mean()) if m.any() else float(np.abs(a-r).mean())
out = []
for row in rows:
    a = L(row['mine']); r = L(row['ref'])
    m = mask(a)
    # **基线允许 ±1 帧的时间对齐**（row['refCandidates']）：两条腿剪过之后 PTS 网格可能差一帧，
    # 那不是"口径差"。选了哪个偏移要带回去（kT），不许悄悄换一帧。
    kT = 0
    if row.get('refCandidates'):
        best = None
        for cand in row['refCandidates']:
            rr = L(cand['path'])
            sc = score(a, rr, m)
            if best is None or sc < best[0]:
                best = (sc, cand['k'], rr)
        if best is not None:
            r = best[2]
            kT = best[1]
    sc, dy, sc0 = best_dy(a, r, m)
    out.append({
        'pixel': score(a, r, m),
        # 不屏蔽的那个数也要带回去：屏蔽是**判据**的需要，不是"看不见"的许可。
        'pixelFull': float(np.abs(a-r).mean()),
        'ratio': lap(a, m)/max(lap(r, m), 1e-9), 'dy': dy, 'residual': sc,
        'residual0': float(sc0), 'kT': kT,
    })
print(json.dumps(out))
`;
  const tmpPy = join(plan.dir, '_measure.py');
  const tmpJson = join(plan.dir, '_plan.json');
  writeFileSync(tmpPy, py, 'utf8');
  // ⚠️ 这里写的是**对象** `{ rows, maskRects }`，Python 那边读 `plan['rows']`
  // （第一版我把 maskRects 和 rows 平铺成对象、Python 却还在 `for row in plan:` 迭代 →
  //  拿到的是键名字符串，报 `TypeError: string indices must be integers` ✗）。
  writeFileSync(tmpJson, JSON.stringify({ rows: plan.rows, maskRects: plan.maskRects ?? [] }), 'utf8');
  // ⚠️ **别写死 `python`**（2026-09-30 修）：本机（以及不少 Windows 环境）`python` 不在 PATH ——
  // 仓库自己的规矩就是"跑法用 `python3`"（根卡「验证」那节写着）。第一版写死 `python`，
  // 于是两边都出完片了、在最后一步度量上报 `Command failed: python …`（看着像渲染失败，其实是解释器没找到）。
  // 依次试 `python3` → `python`，全都不行就把"要 numpy 的 python3"说清楚。
  const pyCandidates = [process.env.VTRIM_PYTHON, 'python3', 'python'].filter(Boolean);
  let text = null;
  const pyErrors = [];
  for (const bin of pyCandidates) {
    try {
      text = run(bin, [tmpPy, tmpJson]);
      break;
    } catch (error) {
      pyErrors.push(`${bin}: ${String(error.message || error).split('\n')[0]}`);
    }
  }
  if (text === null) {
    throw new Error(
      `度量脚本跑不起来（它要一个带 numpy 的 python）——\n  ${pyErrors.join('\n  ')}\n` +
        '  （本机没有 `python` 只有 `python3`，或用 VTRIM_PYTHON=<解释器路径> 指定）',
    );
  }
  return JSON.parse(text.trim().split('\n').pop());
}

/// 位移**可识别**吗？（2026-10-01）
///
/// `residual0` = 位移 0 时的残差、`residualBest` = 最佳位移时的残差。
/// 只有当"移位真的让画面更像"时才算识别出位移；否则 dy 只是噪声里的最小值 ——
/// **在噪声上宣布"几何差"比漏报更糟**（它会让人去查一个不存在的问题）。
///
/// 门槛：相对改善 ≥ 1% **且**绝对改善 ≥ 0.02（残差是 0…255 的平均绝对差）。
/// 竖屏样本实测反例：最佳位移 −10px 只把残差从 9.869 降到 9.868（0.01%）⇒ 不算识别。
function identifiableDy(residual0, residualBest) {
  if (!Number.isFinite(residual0) || !Number.isFinite(residualBest)) return false;
  const abs = residual0 - residualBest;
  return abs >= 0.02 && abs >= 0.01 * residual0;
}

// ---------------------------------------------------------------------------
// 自检：**验证那两个坑真的被堵住了**
// ---------------------------------------------------------------------------
//
// 一个只在"跑得通"时好看的脚本没有意义 —— 这一轮我犯的两个错都发生在
// "脚本看起来在正常工作"的时候。所以这里按**逐值**钉：
//
//   1. `remap` 对多段 `mute_ranges` 的映射（坑甲）
//   2. `insertEventsOneByOne` 在**多个事件落到不同 segment** 时行号不串（插一次
//      会让后面所有 segment 的行号位移，一次性从后往前插才对，逐条插则必须重解析）

function selfTest() {
  let checks = 0;
  const eq = (actual, expected, what) => {
    checks += 1;
    const a = typeof actual === 'number' ? Number(actual.toFixed(6)) : actual;
    const b = typeof expected === 'number' ? Number(expected.toFixed(6)) : expected;
    if (a !== b) throw new Error(`${what}：期望 ${b}，实得 ${a}`);
  };

  // ---- 0. 位移可识别性（2026-10-01）----
  //
  // 这三个数是**实测**口径：竖屏样本上最佳位移 −10px 只把残差从 9.869 降到 9.868
  // ⇒ 必须判"不可识别"（否则报出假的"几何差"）；而真的差 10px 的几何问题
  // 一定会让残差明显下降（演示样本上 blur 那次是 −21px，改善远大于 1%）。
  eq(identifiableDy(9.869, 9.868), false, '改善 0.01% ⇒ 不可识别（竖屏样本那个假几何差）');
  eq(identifiableDy(9.869, 9.869), false, '零改善 ⇒ 不可识别');
  eq(identifiableDy(20, 8), true, '改善 60% ⇒ 可识别（真的位置差）');
  eq(identifiableDy(10, 9.5), true, '改善 5%（绝对 0.5）⇒ 可识别');
  eq(identifiableDy(10, 9.99), false, '绝对改善 0.01 < 0.02 ⇒ 不算（防"用极小改善下结论"）');

  // ---- 1. 源 -> 输出映射 ----
  //
  // 掐掉 [10,12) 与 [20,23)：共 5 秒。
  //   源 5   （在第一段之前）      -> 5
  //   源 11  （在第一段里面）      -> 11 - 1 = 10
  //   源 12  （正好在掐掉段之后）  -> 12 - 2 = 10
  //   源 15                        -> 13
  //   源 21  （在第二段里面）      -> 21 - 2 - 1 = 18
  //   源 25                        -> 20
  const r = makeRemap([[10, 12], [20, 23]]);
  eq(r(5), 5, 'remap 在掐掉段之前');
  eq(r(11), 10, 'remap 在掐掉段里面');
  eq(r(12), 10, 'remap 正好在掐掉段之后');
  eq(r(15), 13, 'remap 在两段之间');
  eq(r(21), 18, 'remap 在第二个掐掉段里面');
  eq(r(25), 20, 'remap 在全部掐掉之后');
  // **核心**：把"减去掐掉长度"这一步去掉会得到什么 —— 必须与上面不同，
  // 否则这个自检没有分辨力。
  const identity = (s) => s;
  if (identity(25) === r(25)) throw new Error('自检失效：恒等映射与 remap 同值，说明用例选错了点');

  // 读 mute_ranges
  const tomlSample = [
    '[project]', 'name = "x"', '',
    '[[mute_ranges]]', 'start = 10.0', 'end = 12.0', '',
    '[[mute_ranges]]', 'start = 20.0', 'end = 23.0', '',
    '[stage]', 'blur_px = 28', '',
  ].join('\n');
  const ranges = readMuteRanges(tomlSample);
  eq(ranges.length, 2, 'mute_ranges 条数');
  eq(ranges[0][0], 10, 'mute_ranges[0].start');
  eq(ranges[1][1], 23, 'mute_ranges[1].end');
  // 不能把 `[stage] blur_px` 之类误当成区间
  eq(readMuteRanges('[stage]\nblur_px = 28\n').length, 0, '没有 mute_ranges 时应当是 0 条');

  // ---- 2. 插事件：多个事件落到不同 segment，行号不能串 ----
  const toml2 = [
    '[[segments]]', 'start = 0.0', 'end = 10.0', 'note = "a"', '',
    '[[segments]]', 'start = 10.0', 'end = 20.0', 'note = "b"', '',
    '[[segments]]', 'start = 20.0', 'end = 30.0', 'note = "c"', '',
  ].join('\n');
  const inserted = insertEventsOneByOne(toml2, [
    { type: 'blur', time: 2.0 },        // -> segment 0
    { type: 'noise', time: 12.0 }, // -> segment 1
    { type: 'shake', time: 25.0, intensity: 20 },   // -> segment 2
  ]);
  eq(inserted.insertedAt.length, 3, '三个事件都要插进去');
  eq(inserted.insertedAt.filter((x) => x.segment === '0-10').length, 1, 'segment 0 收到 1 条');
  eq(inserted.insertedAt.filter((x) => x.segment === '10-20').length, 1, 'segment 1 收到 1 条');
  eq(inserted.insertedAt.filter((x) => x.segment === '20-30').length, 1, 'segment 2 收到 1 条');
  // 每个事件块必须落在**它自己那个** segment 里（行号串了就会串到别的段）
  const blocks = inserted.text.split('[[segments]]');
  const inSeg = (i, ty) => blocks[i].includes(`type = "${ty}"`);
  if (!inSeg(1, 'blur')) throw new Error('blur 没落在第 1 个 segment 里（行号串了）');
  if (!inSeg(2, 'noise')) throw new Error('noise 没落在第 2 个 segment 里（行号串了）');
  if (!inSeg(3, 'shake')) throw new Error('shake 没落在第 3 个 segment 里（行号串了）');
  checks += 3;
  // 而且**段内顺序**不能把别段的 header 吃掉
  eq(blocks.length, 4, 'segment 头数量不变');

  // 事件落不进任何 segment 时必须**抛**，不能静默丢
  checks += 1;
  let threw = false;
  try {
    insertEventsOneByOne(toml2, [{ type: 'blur', time: 99.0 }]);
  } catch {
    threw = true;
  }
  if (!threw) throw new Error('事件落在所有 segment 之外时应当抛，实得静默通过');

  console.log(`✓ vtrim-compare 自检：${checks} 项全绿`);
  console.log('  钉住的是「源->输出映射」与「多事件插入不串行号」——这一轮我犯的两个错');

  // ---- 3. 屏蔽带（弹幕区）与阈值判定 ----
  //
  // ⚠️ 这两条是**判据自己的自证**：判据写错了的表现是"永远绿"，
  // 比报错难发现得多。所以既测"屏蔽带形状对"，也测"超阈值必须报错"。
  const full = maskRectsFor(1920, 1080, true);
  eq(full.length, 2, '屏蔽带两条（左带 + 上带）');
  eq(full[0].w, 422, '左带宽度 = 1920 × 0.22');
  eq(full[1].h, 270, '上带高度 = 1080 × 0.25');
  eq(maskRectsFor(1920, 1080, false).length, 0, '--no-mask-danmaku 时没有屏蔽带');

  const ok = judge({
    refDuration: 9.0, mineDuration: 9.0, basePixel: 12, baseDy: 0, windows: [],
  });
  eq(ok.length, 0, '时长一致 + 基线在阈值内 => 无失败');
  const bad1 = judge({ refDuration: 9.0, mineDuration: 6.0, basePixel: 12, baseDy: 0, windows: [] });
  if (!bad1.some((f) => f.includes('时长不一致'))) {
    throw new Error(`时长差 3s 必须报「时长不一致」，实得：${JSON.stringify(bad1)}`);
  }
  checks += 1;
  const bad2 = judge({ refDuration: 9.0, mineDuration: 9.0, basePixel: 40, baseDy: 12, windows: [] });
  if (bad2.length !== 2) throw new Error(`基线像素差与位移都要报，实得 ${JSON.stringify(bad2)}`);
  checks += 1;
  const bad3 = judge({
    refDuration: 9.0, mineDuration: 9.0, basePixel: 12, baseDy: 0,
    windows: [{ group: 'blur@1.0', pixel: 60, neighbour: 12 }],
  });
  if (!bad3.some((f) => f.includes('blur@1.0'))) throw new Error(`高于邻居的窗口必须报，实得 ${JSON.stringify(bad3)}`);
  checks += 1;
  // 阈值覆盖（`--baseline-pixel-max`）也必须真起作用 —— 它就是"让断言红"的那个开关。
  const override = judge({ refDuration: 9.0, mineDuration: 9.0, basePixel: 12, baseDy: 0, windows: [], overrides: { baselinePixel: 5 } });
  if (!override.some((f) => f.includes('基线像素差'))) throw new Error('阈值覆盖没生效');
  checks += 1;
  console.log(`✓ 判据自证：屏蔽带 4 项 + 阈值判定 5 项（含"超阈值必须报"的三条）`);
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.selfTest) {
    selfTest();
    return;
  }
  if (args.help || (!args.clip && !args.fixture && !args.allFixtures)) {
    console.log(USAGE);
    process.exit(args.help ? 0 : 2);
  }

  // ---- 固定样本：**逐个再起一次自己**（不重构下面那条 400 行主流程）----
  //
  // 每个样本一个独立运行目录；缺样本时：显式点名要它 -> 报错（并说清要准备什么），
  // `--all-fixtures` -> **跳过并打印**（不然干净检出上连演示样本都跑不了）。
  if (args.allFixtures) {
    const passThrough = [];
    // ⚠️ **白名单**：`--all-fixtures` 是"再起一个自己"，只转这些参数。
    // 忘了加进来的表现**特别隐蔽**：命令照样跑、只是**换了口径** ——
    // 例如漏了 `--golden` 就会静默用"现渲染兜底腿"那条活参照（数字与黄金帧不同，
    // 而两边都在阈值内 ⇒ 不会报红，只会让人拿错基线比）。
    // 2026-10-01：`golden` / `captureGolden` 就是这么漏的（跑了 `all --golden`，
    // portrait 报 9.87 = 活参照口径，而黄金帧是 9.84）。
    for (const key of [
      'vtrim', 'dhampir', 'fontFile', 'events', 'translator', 'baselinePixelMax', 'golden', 'captureGolden',
    ]) {
      if (args[key] !== undefined && args[key] !== null && args[key] !== false) {
        const flag =
          key === 'fontFile' ? 'font-file'
          : key === 'baselinePixelMax' ? 'baseline-pixel-max'
          : key === 'captureGolden' ? 'capture-golden'
          : key;
        passThrough.push(`--${flag}`, String(args[key]));
      }
    }
    if (args.noMaskDanmaku) passThrough.push('--no-mask-danmaku');
    let bad = 0;
    let skipped = 0;
    for (const name of Object.keys(FIXTURES)) {
      const out = join(resolve(args.out || join(REPO_ROOT, 'target', 'vtrim-compare')), name);
      step(`样本 ${name} —— ${FIXTURES[name].note}`);
      if (!existsSync(join(resolve(FIXTURES[name].source), 'polish.toml'))) {
        skipped += 1;
        log(`  ⚠ 跳过：本机没有 ${FIXTURES[name].source}（${FIXTURES[name].note}）`);
        continue;
      }
      try {
        execFileSync(process.execPath, [fileURLToPath(import.meta.url), '--fixture', name, '--out', out, ...passThrough], {
          stdio: 'inherit',
        });
        log(`  ✓ 样本 ${name}：判据全过`);
      } catch {
        bad += 1;
        console.error(`  ✗ 样本 ${name}：判据没过（报告见 ${join(out, 'report.json')}）`);
      }
    }
    step(`固定样本汇总：${Object.keys(FIXTURES).length} 个（跳过 ${skipped} / 失败 ${bad}）`);
    process.exit(bad ? 1 : 0);
  }

  if (args.fixture) {
    const fx = FIXTURES[args.fixture];
    if (!fx) throw new Error(`不认识的样本 ${args.fixture}（可用：${Object.keys(FIXTURES).join(' / ')}）`);
    const outRoot = resolve(args.out || join(REPO_ROOT, 'target', 'vtrim-compare'));
    args.clip = prepareFixture(fx, join(outRoot, `${args.fixture}-clip`));
    log(`  样本 ${args.fixture}：${fx.note}\n    -> ${args.clip}`);
  }

  if (args.golden && args.captureGolden) throw new Error('--golden 与 --capture-golden 只能用一种');
  if (args.golden && args.events) {
    throw new Error(
      '黄金帧模式只做**基线对照**：不许配 --events（冻结的是"不插事件"那一轮的采样帧；\n' +
        '  要量事件窗口就还得有兜底腿，或一个能出参照的旧二进制）',
    );
  }

  const clip = resolve(args.clip);
  // 黄金帧的样本名：优先 --fixture，否则用片段目录名。
  const goldenName = args.fixture ?? basename(clip);
  const goldenDir = args.golden ? join(resolve(args.golden), goldenName) : null;
  const captureDir = args.captureGolden ? join(resolve(args.captureGolden), goldenName) : null;
  const tomlPath = join(clip, 'polish.toml');
  if (!existsSync(tomlPath)) throw new Error(`找不到 ${tomlPath}`);

  const runDir = resolve(args.out || join(REPO_ROOT, 'target', 'vtrim-compare'));
  // 可执行后缀：主要在 Windows 上用，但路径拼接不写死 `.exe`。
  const EXE = process.platform === 'win32' ? '.exe' : '';
  // 转译器**就在本仓**（`tools/polish-to-dhampir.mjs`），见 docs/dhampir/compare-loop.md。
  //
  // 与 `vtrim` 同一个办法：候选列表 + 环境变量，全找不到就**大声报错并列出找过哪里**。
  // 「指一个相对路径」在这里是错的 —— 底座是**另一个仓库**，
  // 它可能在同层，也可能被 `VTEDIT_DHAMPIR` 指到别处。
  const translatorCandidates = [
    args.translator,
    process.env.VTRIM_TRANSLATOR,
    join(REPO_ROOT, 'tools', 'polish-to-dhampir.mjs'),
  ].filter(Boolean).map((p) => resolve(p));
  const translator = translatorCandidates.find((p) => existsSync(p));
  // 底座可执行：`--dhampir` / `VTEDIT_DHAMPIR` / 同层 `Dhampir/target/{debug,release}`。
  // **不猜版本目录** —— 找不到就报错并列出找过的地方（与 `vtrim` 同一套纪律）。
  const dhampirCandidates = [
    args.dhampir,
    process.env.VTEDIT_DHAMPIR,
    join(REPO_ROOT, '..', 'Dhampir', 'target', 'debug', `dhampir${EXE}`),
    join(REPO_ROOT, '..', 'Dhampir', 'target', 'release', `dhampir${EXE}`),
  ].filter(Boolean).map((p) => resolve(p));
  const dhampir = dhampirCandidates.find((p) => existsSync(p));
  const fontFile = args.fontFile || 'C:/Windows/Fonts/msyh.ttc';
  // vtrim 可执行：按**候选列表**找，而不是猜一个相对路径。
  //
  // 我第一版按 `clip/../../../../..` 往上找五层 —— 那依赖片段在磁盘上的深度，
  // 换个项目就找不到（第一次真跑就是这么失败的）。候选列表把常见位置与环境变量
  // 都列出来，全找不到才退到 PATH，并在报错里**说清找过哪些地方**。
  const vtrimCandidates = [
    args.vtrim,
    process.env.VTRIM_EXE,
    join(REPO_ROOT, '.workbuddy', 'perf', 'bin', `vtrim${EXE}`),
    join(REPO_ROOT, 'target', 'release', `vtrim${EXE}`),
    join(REPO_ROOT, 'target', 'debug', `vtrim${EXE}`),
  ].filter(Boolean).map((p) => resolve(p));
  const vtrim = vtrimCandidates.find((p) => existsSync(p)) || 'vtrim';

  if (!translator) {
    throw new Error(
      '找不到转译器（`polish-to-dhampir.mjs`）。它在本仓 `tools/` 下，\n' +
      '  用 `--translator <路径>` 或环境变量 `VTRIM_TRANSLATOR` 显式指一个。找过：\n' +
      translatorCandidates.map((p) => `    ${p}`).join('\n'),
    );
  }
  if (!dhampir) {
    throw new Error(
      '找不到 Dhampir 的可执行文件。用 `--dhampir <路径>` 或环境变量 `VTEDIT_DHAMPIR` 指一个，\n' +
      '  或把它放在与本仓同层的 `Dhampir/target/{debug,release}/`。找过：\n' +
      dhampirCandidates.map((p) => `    ${p}`).join('\n'),
    );
  }
  log(`转译器：${translator}`);
  // `vtrim` 落回 PATH 时这里不报错（让它自己 spawn 失败），但**列清楚找过什么**。
  log(`vtrim：${existsSync(vtrim) ? vtrim : `${vtrim}（不在候选里，试 PATH）`}`);
  if (!existsSync(vtrim)) {
    log(`  找过：\n${vtrimCandidates.map((p) => `    ${p}`).join('\n')}`);
  }

  mkdirSync(runDir, { recursive: true });
  const originalToml = readFileSync(tomlPath, 'utf8');
  const backupPath = join(runDir, 'polish.toml.orig');
  const workingPath = join(runDir, 'polish.toml.test');

  const events = args.events ? JSON.parse(readFileSync(resolve(args.events), 'utf8')) : [];
  const muteRanges = readMuteRanges(originalToml);
  const remap = makeRemap(muteRanges);

  log(`片段：${clip}`);
  log(`运行目录：${runDir}`);
  log(`要插的测试事件：${events.length} 个`);

  // ---- 映射表：**先打出来**（坑甲）----
  step('源时间 -> 输出时间映射');
  log(`  mute_ranges（掐掉，不是静音）：${muteRanges.map(([a, b]) => `${a}-${b}`).join('  ') || '（无）'}`);
  const totalCut = muteRanges.reduce((s, [a, b]) => s + (b - a), 0);
  log(`  共掐掉 ${totalCut.toFixed(2)} 秒`);
  if (events.length) {
    log('  测试事件的落点：');
    for (const ev of events) {
      log(`     ${String(ev.type).padEnd(12)} 源 ${String(ev.time).padStart(7)}  ->  输出 ${remap(ev.time).toFixed(2)}`);
    }
  }

  let report = [];
  let failed = null;
  let assertFailures = [];
  try {
    // ---- 准备 toml ----
    writeFileSync(backupPath, originalToml, 'utf8');
    let testToml = originalToml;
    if (events.length) {
      const r = insertEventsOneByOne(originalToml, events);
      testToml = r.text;
      for (const a of r.insertedAt) log(`  已插入 ${a.type}@${a.sourceSec}s 到 segment ${a.segment}`);
    }
    writeFileSync(workingPath, testToml, 'utf8');
    copyFileSync(workingPath, tomlPath);

    // ---- 转译 ----
    step('转译');
    const projectPath = join(runDir, 'project.json');
    const translateLog = run('node', [translator, clip, '--out', projectPath]);
    for (const line of translateLog.trim().split('\n').slice(0, 5)) log(`  ${line}`);

    // ---- 两边渲染 ----
    const info = probeVideo(join(clip, 'clip.mp4'));
    step(`渲染（源 ${info.width}x${info.height} ${info.fps}fps）`);
    // 产物名**从工程的 `polish.toml` 读**，别写死 `vtuber-clip_1.mp4`（2026-09-30 修）：
    // 真实片段里 `output = "vtuber-clip.mp4"`、上一版成片还在 ⇒ `unique_output_path` 加 `_1`；
    // 但演示工程的 `output = "export.mp4"` —— 写死就会在下面那句 `却产出` 上失败
    // （第一次拿演示工程跑这个工具，就是停在这里）。
    const outputName =
      (readFileSync(join(clip, 'polish.toml'), 'utf8').match(/^\s*output\s*=\s*"([^"]+)"/m) ?? [])[1] ||
      'vtuber-clip.mp4';
    const stem = outputName.replace(/\.mp4$/i, '');
    const outPlain = join(clip, outputName);
    const outSuffixed = join(clip, `${stem}_1.mp4`);
    // 只删**带后缀那一份**（与旧行为一致）：`outPlain` 是用户自己的既有成片，不许动它。
    // 删掉 `_1` 之后本次渲染的名字才是确定的：`outPlain` 存在 ⇒ 写 `_1`；不存在 ⇒ 写 `outPlain`。
    if (existsSync(outSuffixed)) rmSync(outSuffixed);
    let refPath;
    let refInfo;
    if (args.golden) {
      // -----------------------------------------------------------------
      // **黄金帧模式**：参照是冻结的小图，**一个字节都不渲染兜底腿**。
      //
      // 但工程仍然要先跑一次 `process`（贴纸落点/运镜锚点这些是后处理的产物）——
      // 那是**产品命令**，不是兜底腿的一部分；兜底腿指的是 `render --renderer chrome`。
      // 兜底腿当时是"渲染时隐式跑 process"，这里显式跑一次。
      // -----------------------------------------------------------------
      const metaPath = join(goldenDir, 'golden.json');
      if (!existsSync(metaPath)) {
        throw new Error(
          `黄金帧模式缺 ${metaPath} —— 先在**还有兜底腿**的时候跑一次：\n` +
            `  node scripts/vtrim-compare.mjs --fixture ${goldenName} --capture-golden <目录>`,
        );
      }
      // ⚠️ **process 是"尽力而为"** —— 与出片那条路同口径：`cmd_render` 里是
      // `if let Err(e) = process::cmd_process(..) { ⚠ Process before render: .. }`，失败照样出片。
      // 对拍样本是**刚复制到运行目录**的，缺 `data/subtitles.json` 时 process 必然失败
      // （"Run 'vtrim polish init' first"）—— 那不影响出片，别把它变成硬失败（我第一版就写成硬失败，
      // 于是黄金帧模式在样本上根本跑不起来）。
      try {
        if (!existsSync(join(clip, 'data', 'subtitles.json'))) {
          run(vtrim, ['polish', 'init', clip]);
        }
        run(vtrim, ['polish', 'process', clip]);
      } catch (e) {
        log(`    ⚠ process 没跑成（与出片同口径：尽力而为）—— ${String(e.message).split('\n')[0].slice(0, 110)}`);
      }
      refInfo = JSON.parse(readFileSync(metaPath, 'utf8'));
      refPath = join(goldenDir, 'ref.mp4'); // 只作报告里的标识（这个文件不存在，也不需要存在）
      log(
        `    参照（**冻结帧**，不碰兜底腿）${refInfo.frames} 帧 / ${refInfo.duration.toFixed(3)}s / ${refInfo.fps}fps` +
          `  -> ${goldenDir}`,
      );
    } else {
      // ⚠️ **活参照那条路已随兜底腿一起作废**（2026-10-01）。
      //
      // 这里原本是 `vtrim polish render --renderer chrome`：拿本仓那条 headless Chrome 逐帧腿
      // 当**活的参照**。那条腿已被用户拍板整条删除，所以这条路现在**必然失败** ——
      // 而且失败的样子很难看：`render --renderer chrome` 会报"认不出的出片通道"，
      // 与"参照渲染坏了"看起来一模一样，很容易被当成工具坏了去查。
      //
      // 所以这里**当场说清楚**：要参照就用**冻结帧**（`--golden`），别去修这条已经不存在的路。
      // 冻结帧是 2026-10-01 在删除前用那条腿跑出来冻上的（`--capture-golden`）。
      throw new Error(
        '活参照模式已作废：它渲染的是本仓那条 headless Chrome 兜底腿，而那条腿 2026-10-01 已整条删除。' +
          '改用冻结参照帧：\n' +
          `    node scripts/vtrim-compare.mjs --fixture ${goldenName} --golden docs/golden/dhampir-parity\n` +
          '  （冻结帧在 docs/golden/dhampir-parity/<样本>/，2026-10-01 删除前用那条腿冻的；\n' +
          '   `--capture-golden` 一并作废 —— 它同样要跑那条腿，不再重冻。）',
      );
      // eslint-disable-next-line no-unreachable
      log('  V-Trim（chrome 后端，会自动先跑 process）…');
      run(vtrim, ['polish', 'render', clip, '--renderer', 'chrome']);
      const vtrimOut = [outSuffixed, outPlain].find((p) => existsSync(p));
      if (!vtrimOut) throw new Error(`V-Trim 跑完了却没产出 ${outPlain}（也没产出 ${stem}_1.mp4）`);
      refPath = join(runDir, 'ref.mp4');
      copyFileSync(vtrimOut, refPath);
      rmSync(vtrimOut); // 工程目录立刻恢复干净
      refInfo = probeVideo(refPath);
      log(`    参照 ${refInfo.frames} 帧 / ${refInfo.duration.toFixed(3)}s / ${refInfo.fps}fps  -> ${refPath}`);
    }

    const minePath = join(runDir, 'mine.mp4');
    // **`--to` 要按本仓自己的帧率算，不能用参照的帧数。**
    //
    // 我第一版写的是 `refInfo.frames - 1` —— 参照是 **30fps**、本仓是 **60fps**，
    // 于是只渲了一半（到手 2907 帧 / 48.45s，而它该是 5812 帧 / 96.87s）。
    // 这是"两个宿主帧率不同"最基础的一处，而我恰恰写错了。
    const timeline = JSON.parse(readFileSync(projectPath, 'utf8')).timeline;
    const tb = timeline.timebase;
    const mineFps = tb.num / tb.den;
    // 工程的**总帧数**没有单列字段，由各层 `start + end` 的最大值决定。
    // 拿它跟"参照时长 × 本仓帧率"取小 —— 否则会比工程多出两三帧
    // （实测 5814 vs 工程的 5812），虽然不影响窗口内的取样，但会让报告里的
    // 帧数看着对不上，而"看着对不上"正是这份工具要消灭的东西。
    let projectFrames = 0;
    for (const track of timeline.tracks) {
      for (const layer of track.layers ?? []) {
        // `end` 是**绝对帧号**（不是长度）—— 我第一版写成 `start + end`，
        // 于是算出 11320 = 2 x 5660，钳位完全失效。
        projectFrames = Math.max(projectFrames, Number(layer.end));
      }
    }
    const byDuration = Math.round(refInfo.duration * mineFps);
    const lastFrame = Math.max(1, (projectFrames > 0 ? Math.min(byDuration, projectFrames) : byDuration) - 1);
    log(`  工程共 ${projectFrames} 帧，参照时长折合 ${byDuration} 帧 -> 出 0..${lastFrame}`);
    log(`  本仓时间基 ${tb.num}/${tb.den} = ${mineFps}fps -> 出 0..${lastFrame} 帧（参照 ${refInfo.frames} 帧 @${refInfo.fps}fps）`);
    run(dhampir, [
      'render', '--project', projectPath, '--from', '0', '--to', String(lastFrame),
      '--out', minePath, '--asset-root', clip, '--font-file', fontFile,
    ]);
    const mineInfo = probeVideo(minePath);
    log(`    本仓 ${mineInfo.frames} 帧 / ${mineInfo.duration.toFixed(3)}s / ${mineInfo.fps}fps  -> ${minePath}`);

    // ---- 采样 ----
    //
    // **一律用输出时间**（坑甲）。基线取窗口之外的几处。
    step('逐窗口度量');
    const frameDir = join(runDir, 'frames');
    mkdirSync(frameDir, { recursive: true });
    const rows = [];
    const sample = (tag, t, opts = {}) => {
      const m = join(frameDir, `${tag}-m-${t.toFixed(3)}.png`);
      const r = join(frameDir, `${tag}-r-${t.toFixed(3)}${args.golden ? '.jpg' : '.png'}`);
      grab(minePath, t, m);
      if (args.golden) {
        // 冻结帧直接拷贝（PIL 按内容认格式，扩展名只为了让人看得懂）。
        const src = join(goldenDir, `ref-${t.toFixed(3)}.jpg`);
        if (!existsSync(src)) {
          throw new Error(`黄金帧缺 ${src} —— 冻结时用的采样点与这次不一致（改了 mute_ranges 或样本？）`);
        }
        copyFileSync(src, r);
      } else {
        grab(refPath, t, r);
      }
      // -------------------------------------------------------------------
      // **时间对齐只给基线开**（2026-09-30 M1 诊断出来的）
      //
      // 两条腿按屏蔽区间剪过之后，PTS 网格可能差**半帧到一帧**（参照 30fps ⇒ 33ms），
      // 于是同一个墙钟时刻在两边的画面本来就差一点。诊断实测（演示样本 t=5.0s）：
      //   参照往前一帧 ⇒ 整帧差 **5.90**；同一时刻 ⇒ 6.19；往前两帧 ⇒ 9.73。
      // 即：**内容对齐的真实底噪 ~6**，而工具原来的基线采样点又正好落在**剪点上**
      // （输出 2.0s 处 ⇒ 实测 30.62，见 report.json 的 baseline）。
      //
      // ⚠️ **特效窗口不许开它**：那里量的就是"同一时刻像不像"，
      // 给它 ±1 帧的自由度会把"特效慢了一帧"放过去 —— 而那正是它要抓的东西。
      const refCandidates = [{ k: 0, path: r }];
      // ⚠️ **黄金帧模式不做 ±1 帧时间对齐**：冻结的就是"那一刻"那一帧，没有邻帧可取
      // （硬要取就得连邻帧一起冻结，收益很小 —— 这条对齐本来就是为"两条腿 PTS 网格差半帧"服务的，
      //   而 PTS 网格是**出片那条腿**的属性，不随参照来源改变）。
      if (opts.alignTemporal && !args.golden) {
        const refFrame = 1 / (refInfo.fps || 30);
        for (const k of [-1, 1]) {
          const tt = t + k * refFrame;
          if (tt <= 0 || tt >= refInfo.duration - 1e-3) continue;
          const p = join(frameDir, `${tag}-r-${t.toFixed(3)}-k${k}.png`);
          grab(refPath, tt, p);
          refCandidates.push({ k, path: p });
        }
      }
      return { mine: m, ref: r, refCandidates };
    };

    // 基线：在事件窗口之外、且离得够远的时刻
    const winOf = (ev) => {
      const widths = { blur: 0.80, shake: 0.20, flash: 0.44, hue_shift: 0.95,
        noise: 0.80, pulse: 0.48, };
      const outStart = remap(ev.time);
      const w = widths[ev.type] ?? 0.5;
      return { outStart, outEnd: outStart + w, width: w };
    };
    const busy = events.map((ev) => winOf(ev));
    // ---------------------------------------------------------------------
    // 基线采样点：**避开剪点**（2026-09-30 M1 修的）
    //
    // 原来是从 t=2.0 起每 7.3s 取一个 —— 对 8.9s 的样本只取到**一个点**，
    // 而那个点正好落在屏蔽区间的剪点上（输出 2.0s = 剪点），于是基线量出 30.62，
    // 而内容对齐的真实底噪只有 ~6（诊断见 `sample()` 的注释）。
    // 现在：从 1.2s 起每 1.4s 取一个、最多 5 个，且**距剪点 ±0.4s 内不要**
    // （剪点处两条腿的 PTS 边界本来就可能差一帧，那不是"口径差"）。
    const cutOutTimes = muteRanges.map(([s]) => remap(s)).filter((x) => x > 0);
    const nearCut = (t) => cutOutTimes.some((c) => Math.abs(t - c) < 0.4);
    const baselineTimes = [];
    for (let t = 1.2; t < refInfo.duration - 1.2 && baselineTimes.length < 5; t += 1.4) {
      if (nearCut(t)) continue;
      if (busy.some((b) => t > b.outStart - 1.0 && t < b.outEnd + 1.0)) continue;
      baselineTimes.push(Number(t.toFixed(2)));
    }
    for (const [i, t] of baselineTimes.entries()) {
      rows.push({ ...sample(`base${i}`, t, { alignTemporal: true }), group: '基线', t });
    }

    // ---------------------------------------------------------------------
    // **冻结黄金帧**（--capture-golden）：把刚抓下来的基线参照帧存成小图 + 一份 meta。
    //
    // 为什么要在还有兜底腿的时候做：删掉它之后，这条工具的参照就没了。
    // 已经渲染过的那一版**就是**当时的"可信参照"，冻结下来即可（q=2 的 JPEG：肉眼无损，
    // 但会带一点点编码噪声 ⇒ 黄金帧模式的基线会比现在略高，判据 20 的余量足够）。
    // ---------------------------------------------------------------------
    if (captureDir) {
      mkdirSync(captureDir, { recursive: true });
      const frozen = [];
      for (const [i, t] of baselineTimes.entries()) {
        const src = join(frameDir, `base${i}-r-${t.toFixed(3)}.png`);
        const dst = join(captureDir, `ref-${t.toFixed(3)}.jpg`);
        run('ffmpeg', ['-v', 'error', '-i', src, '-q:v', '2', '-y', dst]);
        frozen.push({ t, file: basename(dst) });
      }
      writeFileSync(
        join(captureDir, 'golden.json'),
        JSON.stringify(
          {
            fixture: goldenName,
            captured_at: new Date().toISOString(),
            frozen_from: 'vtrim polish render --renderer chrome（兜底腿最后一次渲染；那条腿 2026-10-01 已删除）',
            frames: refInfo.frames,
            duration: refInfo.duration,
            fps: refInfo.fps,
            width: refInfo.width,
            height: refInfo.height,
            times: baselineTimes,
            frames_meta: frozen,
          },
          null,
          2,
        ),
        'utf8',
      );
      log(`  ✔ 冻结 ${frozen.length} 帧黄金帧 -> ${captureDir}（之后可用 --golden 复跑，不再需要兜底腿）`);
    }

    for (const [i, ev] of events.entries()) {
      const w = winOf(ev);
      // -----------------------------------------------------------------
      // **取样点必须落在"两个片子都会取到同一帧"的时刻上。**（坑丙）
      //
      // 本仓与参照的帧率**不一样**（实测这一对是 60 vs 30）。同一条时间
      // `-ss t` 在 60fps 上落在第 `round(t*60)` 帧，在 30fps 上落在
      // `round(t*30)` 帧 —— **是两个不同的瞬间**。
      //
      // 对长窗口这无所谓；对**短效果**是致命的：stutter 的台阶只有 0.06 秒，
      // 半帧错位就把整条量歪。实测症状是"stutter 18.24、邻居 5.42"，
      // 而按参照帧率对齐再取，每一个点的最佳位移都是 (0,0)、整帧差 2.4~4.8 ——
      // **效果本来就是对的**。
      //
      // 修法：取样点取**参照帧长**的整数倍（粗的那个）。这样两个片子都必然
      // 落在自己那一格的开头，时间是一致的。
      //
      // 具体做法：**取参照每一帧的中点**。那一瞬间在两个片子里都落在
      // "参照的第 f 帧"之内 —— 60fps 那边会取到覆盖这一瞬间的那一帧，
      // 于是两边看的是**同一个瞬间**。
      //
      // ⚠️ 我第一版写成 `round(t/refFrame)*refFrame + refFrame*0.5` ——
      // 那个 `+0.5` 把点推到参照帧的**中点**，而 60fps 那边正好落到**下一帧**，
      // **自己制造了半帧错位**（stutter 从 18.24 只降到 12.43）。
      // 中点要加在**帧号**上，不是加在时间上。
      const refFrame = 1 / (refInfo.fps || 30);
      const ts = [];
      const firstF = Math.ceil(w.outStart / refFrame - 1e-6);
      const lastF = Math.floor(w.outEnd / refFrame + 1e-6);
      for (let f = firstF; f <= lastF && ts.length < 8; f += 1) {
        // **取帧边界，不取中点。**
        //
        // 反直觉，但实测如此：ffmpeg 的 `-ss t` 会输出**PTS >= t 的第一帧**。
        // 取中点 `(f+0.5)/fps` 时，那个值在浮点上可能落到 f+1 的边界上，
        // 于是参照取到 f+1、本仓取到"含 f+1 的那一帧" —— **两边又错开一帧**。
        // 实测：取中点 stutter 量出 14.30，取边界量出 (0,0)。
        ts.push(Number((f * refFrame).toFixed(4)));
      }
      // 窗口比参照一帧还短：只能取一个点（取窗口中点）。
      if (!ts.length) ts.push(Number((w.outStart + w.width / 2).toFixed(4)));
      for (const [k, t] of ts.entries()) rows.push({ ...sample(`ev${i}-${k}`, t), group: `${ev.type}@${ev.time}`, t });

      // -----------------------------------------------------------------
      // **局部基线**：紧邻窗口的几点（前后各两点）。
      //
      // 这一条是查 blur 时现出来的：全局基线 7.02 看着很干净，
      // 但 blur 的窗口（输出 60.90）**正好落在一条弹幕的滚动区间里**
      // （`56.13-67.65`），而那条弹幕本来就有一条已知差异
      // （逐条滚动时长 vs 轨道级平均，见 DROP/CLAMP 清单）。
      // 于是量到 26.17 全部记在 blur 头上 —— **而归因是错的**。
      //
      // 局部基线让"底噪本身就在变"这件事显形：判据是**跟邻居比**，
      // 而不是跟"全片的平均值"比。
      // -----------------------------------------------------------------
      const local = [w.outStart - 1.2, w.outStart - 0.6, w.outEnd + 0.6, w.outEnd + 1.2]
        .filter((x) => x > 0.5 && x < refInfo.duration - 0.5);
      for (const [k, x] of local.entries()) {
        rows.push({ ...sample(`loc${i}-${k}`, Number(x.toFixed(3))), group: `${ev.type}@${ev.time}#邻居`, t: Number(x.toFixed(3)) });
      }
    }

    const maskRects = maskRectsFor(refInfo.width, refInfo.height, !args.noMaskDanmaku);
    if (maskRects.length) {
      log(
        `  屏蔽弹幕带（按设计，不进判据）：${maskRects
          .map((r) => `x<${r.x + r.w}∩y<${r.y + r.h}`)
          .join(' / ')}` +
          `（左 ${(MASK.danmakuLeftRatio * 100).toFixed(0)}% + 上 ${(MASK.danmakuTopRatio * 100).toFixed(0)}%，` +
          `--no-mask-danmaku 可关）`,
      );
    }
    const measured = measure({ dir: runDir, rows, maskRects });

    // ---------------------------------------------------------------------
    // **位移先过"可识别性"闸门**（2026-10-01）
    //
    // `best_dy` 给的是"残差最小的那个位移"，**没有问"这个位移真的让画面更像吗"**。
    // 竖屏样本上就撞到了：两腿的差异主要来自色调（参照是更早的历史产物），
    // 残差曲线几乎是平的 —— 最佳位移与 0 位移只差 0.01%，却报出
    // 「基线位移 -10px（几何差，不是效果差）」⇒ **在噪声上宣布几何问题**，
    // 比漏报更糟（会让人去查一个不存在的摆放 bug）。
    //
    // 所以：改善够不上门槛的位移一律记 0，并把"抑制了几处"打出来（不许静默）。
    // 原始值留在 `dyRaw` 里，报告照样带着 —— 口径变了不等于把数据藏起来。
    let suppressedDy = 0;
    for (const m of measured) {
      if (m.dy === 0) continue;
      m.dyRaw = m.dy;
      if (!identifiableDy(m.residual0, m.residual)) {
        m.dy = 0;
        suppressedDy += 1;
      }
    }
    if (suppressedDy) {
      log(
        `  ⚠ 位移不可识别（残差曲线平坦）⇒ 判 0 的有 **${suppressedDy}** 处：` +
          '那不是几何差，别去查不存在的摆放问题（原始值在报告的 `groups[].dyRaw` 里）',
      );
    }

    const byGroup = new Map();
    rows.forEach((row, i) => {
      if (!byGroup.has(row.group)) byGroup.set(row.group, []);
      byGroup.get(row.group).push({ ...row, ...measured[i] });
    });

    const avg = (list, key) => list.reduce((s, x) => s + x[key], 0) / Math.max(list.length, 1);
    /// **中位数**。判据要用它，不能用平均数 —— 见下面 `median` 那一段的理由。
    const median = (list, key) => {
      if (!list.length) return 0;
      const v = list.map((x) => x[key]).sort((a, b) => a - b);
      const mid = v.length >> 1;
      return v.length % 2 ? v[mid] : (v[mid - 1] + v[mid]) / 2;
    };
    const base = byGroup.get('基线') || [];
    // ---------------------------------------------------------------------
    // 基线取**中位数**，不取平均（2026-09-30 M1）
    //
    // 实测（演示样本，5 个窗口外时刻）：`[77.61, 15.07, 6.47, 2.97, 60.01]` ——
    // 平均 32.42、中位 15.07。那两个大数是**剪切帧**：演示工程是"快切特效片"，
    // 采样点一旦落在硬切上，两条腿的 PTS 网格差一帧就足以让整帧差几十，
    // 那既不是口径差、也不是效果差（同组 `dy` 反而是 0）。
    // 平均会被它们主导 —— 与窗口判据用中位数是同一个理由（见下面"坑丁"）。
    //
    // 分布**照实打全**（mean/median/worst/samples）：中位数是判据口径，不是"看不见那两个数"。
    const baseSamples = base.map((x) => x.pixel).sort((a, b) => a - b);
    const basePixel = median(base, 'pixel');
    const basePixelMean = avg(base, 'pixel');
    const basePixelWorst = baseSamples.length ? baseSamples[baseSamples.length - 1] : 0;
    const basePixelFull = avg(base, 'pixelFull');
    const baseRatio = avg(base, 'ratio');

    step('结果');
    // **基线也要打 dy** —— 它是 dy 那一列的参照点。
    // 没有它，"blur 位移 -21px" 这句话就没法判：那是**本来就有的位置差**，
    // 还是**这个特效引入的**？（我第一版只打了像素差与高频比，于是拿到
    // -21 这个数时不知道该往哪查。）
    // 位移判据只用**可识别**的点（带 `dyRaw` 的那些是被闸门判 0 的噪声点，见上面那段）。
    // 一个都没有 ⇒ 判 0：拿噪声下"几何差"的结论，比如实说"这批点识别不出位移"更糟。
    const baseDyOk = base.filter((x) => x.dyRaw === undefined);
    const baseDy = baseDyOk.length ? Math.round(avg(baseDyOk, 'dy')) : 0;
    log(
      `  基线（窗口外 ${base.length} 处）：**中位**像素差 ${basePixel.toFixed(2)}` +
        `（平均 ${basePixelMean.toFixed(2)} / 最差 ${basePixelWorst.toFixed(2)}；不屏蔽中位口径另计 ` +
        `${basePixelFull.toFixed(2)}）   高频比 ${baseRatio.toFixed(2)}   位移 ${baseDy > 0 ? '+' : ''}${baseDy}px`,
    );
    log(`    逐点：${baseSamples.map((x) => x.toFixed(1)).join(' / ')}（大数是剪切帧，天然不可比）`);
    log('');
    log('  ' + '特效'.padEnd(20) + '输出窗口'.padEnd(16) + '中位差'.padStart(8) + '邻居'.padStart(9) + '高频比'.padStart(8) + '最差点'.padStart(7) + '位移'.padStart(6) + '  判定');
    for (const ev of events) {
      const key = `${ev.type}@${ev.time}`;
      const g = byGroup.get(key);
      if (!g) continue;
      const w = winOf(ev);
      // -----------------------------------------------------------------
      // **用中位数，不用平均数。**（坑丁）
      //
      // 一个窗口只取 5~8 个点，而只要**有一个点落在镜头切换/硬切那一帧上**，
      // 它的差就是几十上百，足以主导整条平均。
      // 实测：blur 窗口 8 个点是 `1.13 1.14 1.13 1.18 1.18 **157.35** 2.75 2.69`
      // —— 平均 21.08、中位 1.18。**那个 157 是参照刚切到全幅、本仓还在画中画，
      // 是"切换差一帧"，不是 blur 的问题。**
      const pixel = median(g, 'pixel');
      const ratio = median(g, 'ratio');
      const dy = Math.round(median(g, 'dy'));
      // **判据跟邻居比，不跟全片平均比**（见上面"局部基线"的理由）。
      const neighbour = byGroup.get(`${key}#邻居`) || [];
      const ref = neighbour.length >= 2 ? median(neighbour, 'pixel') : basePixel;
      const bad = pixel > ref * 1.5 + 2;
      // **邻居自己也脏的时候，判据没力气** —— 要明说。
      //
      // 不说的话，"✓"会被读成"这个特效是对的"，而它实际只说明
      // "这个窗口跟它周围一样差"。两者的区别在**换一批测试时刻**时就显形了。
      const noisy = ref > basePixel * 2;
      // 窗口比参照的 2 帧还短时，取样点会**重复落在同一帧**上 —— 判据照样给，
      // 但要说清"这条窗口只能取到 N 个不同的帧"。
      const refFramesInWindow = Math.max(1, Math.round(w.width * (refInfo.fps || 30)));
      const thin = refFramesInWindow < 2;
      const verdict = bad ? '✗ 高于邻居' : noisy ? '⚠ 底噪太脏，判据无效' : thin ? '✓ 窗口仅 ' + refFramesInWindow + ' 参照帧' : '✓';
      const sharp = ratio > 1.25 ? ' 锯齿' : ratio < 0.8 ? ' 偏糊' : '';
      const shift = dy !== 0 ? ` 位移${dy > 0 ? '+' : ''}${dy}px` : '';
      const worst = Math.max(...g.map((x) => x.pixel));
      log('  ' + key.padEnd(20) + `${w.outStart.toFixed(2)}-${w.outEnd.toFixed(2)}`.padEnd(16)
        + pixel.toFixed(2).padStart(8) + ref.toFixed(2).padStart(9) + ratio.toFixed(2).padStart(8)
        + worst.toFixed(0).padStart(7) + String(dy).padStart(6)
        + `  ${verdict}${sharp}${shift}`);
    }

    report = [...byGroup.entries()].map(([group, list]) => {
      // 位移只统计**可识别**的点；`dyN` 是参与点数（复核时一眼看出这组有几个点算数）。
      const ok = list.filter((x) => x.dyRaw === undefined);
      return {
        group, n: list.length, pixel: avg(list, 'pixel'), ratio: avg(list, 'ratio'),
        dy: ok.length ? Math.round(avg(ok, 'dy')) : 0,
        dyN: ok.length,
        // 被「不可识别」闸门判 0 的那几处，原始最佳位移仍留在这里（口径变了 ≠ 把数据藏起来）
        dyRaw: Math.round(list.reduce((s, x) => s + (x.dyRaw ?? x.dy), 0) / Math.max(list.length, 1)),
      };
    });

    // ---- 判据（阈值断言）----
    //
    // 上面那张表是"给人看"的；这一段是"给 CI 看"的。两者**都要**：
    // 只看表 = 每次都要人来判；只判据 = 红了不知道为什么。
    const windowsForJudge = [];
    for (const ev of events) {
      const key = `${ev.type}@${ev.time}`;
      const g = byGroup.get(key);
      if (!g) continue;
      const neighbour = byGroup.get(`${key}#邻居`) || [];
      windowsForJudge.push({
        group: key,
        pixel: median(g, 'pixel'),
        neighbour: neighbour.length >= 2 ? median(neighbour, 'pixel') : basePixel,
      });
    }
    const overrides = {};
    if (Number.isFinite(args.baselinePixelMax)) overrides.baselinePixel = args.baselinePixelMax;
    assertFailures = judge({
      refDuration: refInfo.duration,
      mineDuration: mineInfo.duration,
      basePixel,
      baseDy,
      windows: windowsForJudge,
      overrides,
    });
    writeFileSync(join(runDir, 'report.json'), JSON.stringify({
      clip, muteRanges, totalCut, events: events.map((e) => ({ ...e, outTime: remap(e.time) })),
      baseline: {
        pixel: basePixel, // **中位**（判据用这个）
        pixelMean: basePixelMean,
        pixelWorst: basePixelWorst,
        pixelFull: basePixelFull,
        ratio: baseRatio,
        dy: baseDy,
        samples: baseSamples.map((x) => Number(x.toFixed(3))),
      },
      thresholds: { ...THRESHOLDS, ...overrides },
      maskRects, failures: assertFailures, groups: report,
      duration: { ref: refInfo.duration, mine: mineInfo.duration },
      ref: refPath, mine: minePath, project: projectPath,
    }, null, 2) + '\n', 'utf8');
    log(`\n  报告：${join(runDir, 'report.json')}`);
    if (assertFailures.length) {
      console.error(`\n✗ 判据没过（${assertFailures.length} 条）：`);
      for (const f of assertFailures) console.error(`  - ${f}`);
      console.error('  （阈值常量在脚本头部 THRESHOLDS；标定方法见文件尾注释）');
    } else {
      log(
        `  ✓ 判据全过：时长差 ${Math.abs(refInfo.duration - mineInfo.duration).toFixed(3)}s` +
          ` / 基线像素差 ${basePixel.toFixed(2)}≤${overrides.baselinePixel ?? THRESHOLDS.baselinePixel}` +
          ` / 基线位移 ${baseDy}px  窗口 ${windowsForJudge.length} 个`,
      );
    }
  } catch (error) {
    failed = error;
  } finally {
    // **无论成败都恢复用户的 polish.toml。**
    try {
      writeFileSync(tomlPath, originalToml, 'utf8');
      log('\n  polish.toml 已恢复');
    } catch (restoreError) {
      console.error(`\n  ✗✗ polish.toml 恢复失败：${restoreError.message}`);
      console.error(`  原始内容在：${backupPath}`);
    }
    // 本仓自己的产物不要留在用户的片段目录里
    for (const stray of ['vtuber-clip_1.mp4']) {
      const p = join(clip, stray);
      if (existsSync(p)) { rmSync(p); log(`  已删 ${stray}（V-Trim 的产物，工程目录要干净）`); }
    }
  }

  if (failed) {
    console.error(`\n✗ 失败：${failed.message}`);
    process.exit(1);
  }
  if (assertFailures.length) {
    // 判据没过 —— 报告与产物**都留着**（删了就复核不了，见坑乙）。
    process.exit(1);
  }
  if (args.clean) {
    rmSync(runDir, { recursive: true, force: true });
    log(`  已删运行目录 ${runDir}（--clean）`);
  } else {
    log('  **运行目录保留**（坑乙）：结论要复核时还得看它。确认无误后再删。');
  }
}

main();
