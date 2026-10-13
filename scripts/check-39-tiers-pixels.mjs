#!/usr/bin/env node
/**
 * **39 档真机像素证据**（每一项硬约束：每档都要真机可见证据）。
 *
 * # 为什么必须有这一条
 *
 * `check-39-tiers.mjs` 查的是**结构与求值**（档名 / 下拉 / 非恒等 / 键名对不对），
 * 它**一个像素都没看**。于是下面这件事它能全绿通过：
 *
 * > 某一档的参数写对了、键名也认、预览也算得出非恒等 —— 但底座**画不出来**。
 *
 * 实测见过这个形状（本会话）：档位参数完全合法，而底座因为字体/描边/几何
 * 的某个原因**吐出一团糊**或**整帧没有墨**，`lines_drawn` 还是个正数。
 * ⇒ 结构与求值全绿，画面是错的。
 *
 * 本判据补的就是这一层：**真的起底座、真的出 PNG、真的数像素**。
 *
 * # 判据（逐档）
 *
 *   ① **有墨**：这一帧画出了东西（不是"看着成功、其实一个字都没有"）
 *   ② **动了**：动画窗口内至少两帧的像素**显著不同**
 *      （量的是"**帧间差**"，不是"有没有墨" —— 静止的字也有墨）
 *   ③ **各档不同**：任意两档在同一帧上的像素**不相同**
 *      （防"39 个档位名映射到同一套参数"这种全绿而全错的情况）
 *
 * # ⚠️ 已知的环境依赖（不满足就**拒绝跑**，不静默跳过）
 *
 *   * 需要 `cargo build -p dhampir-worker --bin dhampir` 的产物
 *   * 需要中文字体：`<DHAMPIR>/.workbuddy-tmp/p4fonts/Default.ttf`
 *   * 需要 `ffmpeg` 在 PATH 上（底座的栅格化走它）
 *
 * ⚠️ **不用 `--font-file`**：底座的 `frame` 只认 `--font-dir` + 工程里的
 *    `font_family`（按**文件名主干**匹配，不读字体内部 name 表）。
 */

import { execFileSync } from 'node:child_process';
import {
  existsSync, mkdtempSync, mkdirSync, writeFileSync, rmSync, statSync, readdirSync, readFileSync,
} from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, '..');
const DHAMPIR = process.env.DHAMPIR_DIR ?? 'F:/para/Code/Dhampir';
const BIN = join(DHAMPIR, 'target', 'debug', 'dhampir.exe');
const FONT_DIR = join(DHAMPIR, '.workbuddy-tmp', 'p4fonts');

let checks = 0;
const problems = [];
const ok = (cond, msg) => {
  checks += 1;
  if (!cond) problems.push(msg);
  return cond;
};

// ---- 前置条件：不满足就拒绝跑（**不跳过**：跳过的判据 = 没有判据）----
if (!existsSync(BIN)) {
  console.error(`✗ 找不到底座二进制：${BIN}\n  先 cargo build -p dhampir-worker --bin dhampir`);
  process.exit(2);
}
const FONT = join(FONT_DIR, 'Default.ttf');
if (!existsSync(FONT)) {
  console.error(`✗ 找不到中文字体：${FONT}`);
  console.error('  底座的 frame 走 --font-dir + 工程里的 font_family（按文件名主干匹配）。');
  process.exit(2);
}
try {
  execFileSync('ffmpeg', ['-version'], { stdio: 'ignore' });
} catch {
  console.error('✗ PATH 上没有 ffmpeg —— 底座的栅格化走它。');
  process.exit(2);
}

// ⚠️ 旧二进制会给出**改了之前的**画面 ⇒ 拒绝跑（与另外两条判据同一条纪律）。
{
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
  for (const c of ['dhampir-timeline', 'dhampir-core', 'dhampir-worker']) {
    walk(join(DHAMPIR, 'crates', c, 'src'));
  }
  if (newest > binTime) {
    console.error(`✗ 底座二进制比源码旧（${file}）—— 拒绝在旧二进制上判定。`);
    console.error('  先重建：cd F:/para/Code/Dhampir && cargo build -p dhampir-worker --bin dhampir');
    process.exit(2);
  }
}

const require_ = createRequire(import.meta.url);
const E = await import(pathToFileURL(join(ROOT, 'webui/src/polish/engine.js')).href);
const TIERS = Object.keys(E.ANIM_PRESETS);

// ⚠️⚠️ **工作目录必须放在底座自己那棵树下，不能用 `%TEMP%`**。
//
// 实测（本会话）：把产物目录指到 `C:/Users/.../Temp/xxx/o-x` 时，底座报
// `建不了目录 …：拒绝访问。 (os error 5)` —— 它在那个位置**建不了子目录**
// （`%TEMP%` 上有一层 ACL，Windows 的 PermissionDenied）。
// ⇒ 39 档**全部出图失败**，而报出来的症状是"底座没吐满帧"，
//    看着像底座坏了，其实只是产物目录选错了地方。
//
// ⚠️ 这个坑与计划书里记的那个是**同一族**：底座的文件动作对位置敏感
//    （另一个是"写字幕临时文件撞 `os error 5`，要显式给 `TMP`"）。
//    ⇒ 一律放在 `DHAMPIR/.workbuddy-tmp/` 下面（那本来就是脚手目录）。
const SCRATCH = join(DHAMPIR, '.workbuddy-tmp');
const work = mkdtempSync(join(SCRATCH, 'pix39-'));
const fonts = FONT_DIR;

// ---------------------------------------------------------------------------
// 工程夹具：一条字幕轨，样式按档位题写
// ---------------------------------------------------------------------------

/**
 * **走真转译器建工程** —— 这是本判据与 "自己拼一份样式" 的关键区别。
 *
 * ⚠️⚠️ 第一版我在这里**自己写了一份「档位 → 底座字段」的映射**，结果是
 *    **39 档画出完全相同的画面**（判据报出 178 对档位撞车）。
 *    原因：`tiltDeg` / `swayDeg` **根本不是底座的字段** ——
 *    它们是**样式级**的量，由**转译器**折成 `transform.rotation` 与
 *    `layer.keyframes` 的**关键帧**。我那份映射把它们静默丢掉了。
 *
 * ⇒ 教训：**"自己再实现一遍映射"就是在造第二份事实来源**，
 *    而两份一定会漂；漂了以后判据测的是**我的映射**，不是**出货的那条路**。
 *    正确做法是让判据**喂真 `polish.toml`、跑真 `translate()`**，
 *    这样它测的就是用户实际会走的那条路。
 */
const { parseToml, translate } = await import(pathToFileURL(join(ROOT, 'tools/polish-to-dhampir.mjs')).href);

/** 按档位造一份最小 `polish.toml` 文本，再交给真转译器。 */
function tomlFor(tier) {
  // ⚠️ 描边一律关：底座 `outline = true` 时字幕糊成一整块
  //    （`Dhampir/docs/plan/defect-outline-blob.md`，与本功能无关的既有缺陷）。
  //    **判据绕开它、不掩盖它** —— 那个缺陷有自己的档案与自己的修法。
  return `
duration = 20.0
[project]
name = "39 档像素判据 - ${tier}"
output = "export.mp4"
[source]
video = "clip.mp4"
# 不给字幕源：本判据只关心**元素轨**那 39 档的像素。
# 给了 SRT 会多出一条 sub 轨，而它**不带** font_family
# ⇒ 底座在该轨出现的帧上**拒绝出图**（subtitle_font_missing，报"没名字可找"），
# 于是判据报"底座没吐满帧"——**症状离原因很远**（看着像底座坏了）。
# 底座这个行为是**对的**（它不出一份"看着成功、其实没字幕"的片子），
# 所以这里从**夹具**侧去掉那条轨，而不是去绕过底座的检查。
subtitles = ""
[style.subtitle]
font_ratio = 0.09
stroke_ratio = 0.0
outline = false
font = "Default"
[[segments]]
start = 0.0
end = 20.0
[[segments.events]]
type = "element"
time = 0.2
end = 19.0
kind = "text"
text = "逐字散开测试"
style = "hook"
pos = "center"
anim = "${tier}"
`;
}

/** 用真转译器把一档落成底座工程，返回工程路径。 */
function projectFor(tier, tag) {
  const dir = join(work, `proj-${tag}`);
  mkdirSync(dir, { recursive: true });
  const tomlPath = join(dir, 'polish.toml');
  writeFileSync(tomlPath, tomlFor(tier), 'utf8');
  // 字幕源文件：转译器要读得到它（`[source].subtitles`）。
  writeFileSync(
    join(dir, 'dhampir-subtitle.srt'),
    ['1', '00:00:00,000 --> 00:00:20,000', '逐字散开测试', ''].join('\n'),
    'utf8'
  );
  const cfg = parseToml(readFileSync(tomlPath, 'utf8'));
  const outPath = join(dir, 'dhampir-project.doc.json');
  // ⚠️⚠️ **`translate()` 只 `return`，不落盘** —— 落盘是 CLI 的 `runCli` 干的
  //    （`writeFileSync(outPath, JSON.stringify(doc))`）。
  //    第一版我直接调 `translate(...)` 就 `return outPath`，于是**工程文件根本不存在**
  //    ⇒ 底座报"读不了工程"，而判据把它报成"**底座没吐满帧**"。
  //    ⚠️ 症状离原因很远（看着像底座坏了，其实是判据没写文件）⇒ 记在这里。
  const doc = translate(cfg, dir, outPath, {});

  // ⚠️⚠️ **给每一条字幕轨补上 `font_family`** —— 包括转译器自己生成的那条 `sub` 轨。
  //
  // 为什么要补，而不是在 TOML 里配好就完事：
  //   `[source].subtitles` 那条 SRT 会被转译器落成**另一条** `sub` 轨，
  //   而它的 `font_family` 取自 `[style.subtitle].font`。我第一版只把
  //   `font` 配给了元素轨那条路（`subStyle.font` 走的是另一处解析），
  //   于是 `sub` 轨的 `font_family` 是 `undefined` ⇒ 底座在**它出现的帧**上
  //   **拒绝出图**（`subtitle_font_missing`，报"没名字可找"）。
  //
  // ⚠️ **底座这个行为是对的**（它不出一份"看着成功、其实没字幕"的片子），
  //    所以这里是从**夹具**侧补齐名字，**不是**去绕过底座的检查。
  //    ⚠️ 别把它当成"底座太严" —— 严在这里正是我们要的。
  //
  // ⚠️ 真实管线里这一步由宿主做（`--font-families` 把"库 id → 真族名"喂进来）。
  //    本判据没有字体库，直接用目录里那份 `Default.ttf` 的**文件名主干**。
  for (const track of doc.timeline.tracks) {
    if (track.subtitle && !track.subtitle.font_family) {
      track.subtitle.font_family = 'Default';
    }
  }
  writeFileSync(outPath, `${JSON.stringify(doc, null, 2)}
`, 'utf8');
  return outPath;
}
// ---------------------------------------------------------------------------
// 出图 + 数像素（纯 JS 解 PNG，零依赖：只认我们自己刚写出的真彩/带 alpha PNG）
// ---------------------------------------------------------------------------

/** 出图；返回帧文件路径（失败返回 null）。 */
function render(projPath, frame, outTag) {
  const outDir = join(work, `o-${outTag}`);
  try {
    execFileSync(BIN, ['frame', '--project', projPath, '--from', String(frame), '--to', String(frame),
      '--out', outDir, '--font-dir', fonts], { stdio: 'ignore', env: { ...process.env, TMP: work } });
  } catch (e) {
    return null;
  }
  const png = join(outDir, `frame-${String(frame).padStart(4, '0')}.png`);
  return existsSync(png) ? png : null;
}

/**
 * PNG → 灰度采样。用 `ffmpeg` 转 rawvideo 而不是自己解 PNG：
 * 本判据只关心"有没有墨 / 帧间差 / 档间差"，用现成的解码器最省事也最不会错。
 *
 * ⚠️ 采样成 **64×36** 的小图：像素证据要的是**结构**（哪里亮、动没动），
 *    不需要全分辨率 —— 而且逐档比 39 张 1920×1080 会慢到跑不动。
 */
function gray(png) {
  const raw = execFileSync('ffmpeg', ['-v', 'error', '-i', png, '-vf', `scale=${W}:${H}`,
    '-pix_fmt', 'gray', '-f', 'rawvideo', '-'], { maxBuffer: 1 << 24 });
  return raw;
}

// ⚠️⚠️ **采样分辨率决定"能不能分辨两档"**。
//
// 第一版用 **64×36** 比"各档不同"，而 39 档里有大量**细微差别**的档
// （`arc_up` 转 −14° vs `scatter_lg` 平移；`size_last` 扩散 0.75 vs `grow_asc` 0.8）
// —— 缩到 64×36 之后这些差别**落到同一个灰阶格子里**，于是判据报出
// `arc_up=scatter_lg`、`shake_std=sway_slow` 这种**明显不该相同**的成对。
// ⇒ 那不是"档位重复"，是**判据的分辨率不够**。
//
// 提到 **192×108**（16 倍采样点）：仍远小于原图 1920×1080，跑得动，
// 但足以把上面那些差别分开。⚠️ 别为了"跑得快"把它降回去 —— 降回去
// 这条判据就会开始**误报撞车**（而且报得像真的一样）。
const W = 192;
const H = 108;

/** 墨量（灰度 > 阈值 的采样点数）。 */
function ink(g) {
  let n = 0;
  for (let i = 0; i < g.length; i += 1) if (g[i] > 40) n += 1;
  return n;
}

/**
 * 两张图的差异：**逐采样点归一化差**（0..1），对整幅亮度做归一化。
 *
 * ⚠️⚠️ **不能用"有多少点的灰度差 > 24"那种硬阈值**。
 *
 * 第一版就是这么写的，于是它**看不见小幅度的真实差别**：
 * 实测 `tilt_rev`（静态 5°）与 `sway_lg`（±12° 往复）在 192×108 上是
 * **同一个画面** —— 而原图上两者的**墨迹质心相差 5 像素**
 * （`tilt_rev` 稳定在 x≈960，`sway_lg` 从 963 漂到 956）。
 * 5 / 1920 ≈ 0.26%，缩到 192 宽之后不足一个采样点 ⇒ 硬阈值判它"相同"。
 *
 * ⇒ 改用**归一化均方差**：任何幅度的差别都会反映到数值上，
 *    而不是被"够不够 24 阶"这个门槛吃掉。
 *    ⚠️ 判据的分辨率/度量方式决定了它**能不能看见差别** ——
 *       选错了它就会**误报"两档一样"**，而且报得像真的。
 */
function diff(a, b) {
  if (a.length !== b.length) return 1;
  // ⚠️⚠️ **先做亮度归一化，再比形状** —— 否则量到的是"淡入"，不是"档位动作"。
  //
  // 背景：底座给**每一条**字幕都加了入场淡入（`fade_in_ms`，约 350ms），
  // 而**档位自己的动作**（`scale_in_*` / `per_char`）恰好与它**重叠**。
  // 直接比两帧的灰度 ⇒ 主要量到的是**亮度在涨** ⇒ 连完全静态的档
  //（`tilt_std`，只改 `tiltDeg`）也会被判成"动得厉害" ⇒ ② 空转。
  //
  // ⇒ 每帧先除以自己的**峰值亮度**，把"亮了多少"这个自由度消掉，
  //    剩下的差别才来自**形状/位置**（= 档位真正改的东西）。
  let maxA = 0;
  let maxB = 0;
  for (let i = 0; i < a.length; i += 1) {
    if (a[i] > maxA) maxA = a[i];
    if (b[i] > maxB) maxB = b[i];
  }
  if (maxA === 0 && maxB === 0) return 0;
  const ka = maxA === 0 ? 0 : 255 / maxA;
  const kb = maxB === 0 ? 0 : 255 / maxB;
  let se = 0;
  for (let i = 0; i < a.length; i += 1) se += (a[i] * ka - b[i] * kb) ** 2;
  const rms = Math.sqrt(se / a.length);
  // 分母固定 255（归一化之后两幅图的量程都是 0..255）⇒ 差值是"形状差异占满幅的比例"。
  return Math.min(1, rms / 255);
}

// ---------------------------------------------------------------------------
// 逐档跑
// ---------------------------------------------------------------------------

console.log('39 档真机像素证据（真的起底座、真的数像素）');
console.log('='.repeat(74));
console.log(`  底座 ${BIN}`);
console.log(`  字体 ${FONT}`);
console.log(`  文本 「逐字散开测试」（6 字）`);
console.log('');

// ⚠️⚠️⚠️ **帧集必须落在"这一档的内容真的在画"的区间里**。
//
// 第一版我用了 `[0, 6, 14, 24]` —— 而事件 `time = 0.2s`、30fps ⇒ 第 6 帧才开始，
// 底座要到**第 12 帧**才吐 items（实测逐帧查过 `slam_std`：f0..f10 全 0、f12 起 1）。
// 于是抽样点**大部分在内容出现之前**，量到的"墨"其实是**另一条轨**的，
// 而判据照样报"39/39 有墨且动了" —— **一次假绿**（这条判据自己的）。
//
// ⇒ 逐帧扫过一遍（实测 `tilt_std`：f0..f12 **墨=0**，**f14 起才有 74747 个墨点**）。
//    ⚠️ 所以起点必须是 **14**，不是 12 —— 取 12 会掺进一帧**全黑**，
//       于是"帧间差"量的是"黑 → 有字"，**每一档都被算成动得很厉害**（全部饱和到 1.0），
//       ② 就变成了永远绿 = **空转**。这正是它上一版的病。
// ⚠️ 教训与"判据必须量它声称量的那个东西"是同一类：抽样点选错，
//    整条判据就在测**别的东西**，而它全绿。
//
// ⚠️ 也别只看"动完了没有"：`slam_*`（180ms ≈ 5 帧）到 f20 就落定了，
//    只取后半段会让 `slam_std` / `slam_lg` **本来就该一样**（都是静止的终态）。
// ⚠️⚠️⚠️ **帧集必须同时满足三件事** —— 这是本判据最难选对的一个数。
//
// 实测校准（`slam_std`，时间线 60fps，事件 `time = 0.2s` ⇒ 层 `start = 12`）：
//
//   | 帧 | 灰度峰值 | 墨点 | 发生了什么 |
//   |---|---|---|---|
//   | 12 | 0 | 0 | 还没出现 |
//   | 14 | 46 | **118572** | 出现了，**2.4 倍大**（`scale_in_from`），且刚开始淡入 |
//   | 18 | 125 | 34430 | 缩到正常大小 —— **砸入在这一段** |
//   | 28 | 240 | 77531 | 大小稳定，只剩淡入 |
//   | 33 | 255 | 77633 | 淡入结束 |
//
// ⇒ 三条约束：
//   ① **起点 ≥ 14**：13 之前没内容（否则量的是"黑 → 有字"）；
//   ② **终点 ≤ ~30**：33 之后所有档位的入场动画都结束了
//      （`slam` 180ms、`pop` 350ms ⇒ 最迟 21 帧后收），再往后只有淡入；
//   ③ **窗口要盖住各档的动作**：`scale_in_ms` 最大 500ms = 30 帧，
//      所以起点不能太晚，否则会漏掉慢档的前半段。
//
// ⚠️ 在这个窗口里，**淡入与档位动作是重叠的** —— 所以 ② 不能直接比"两帧像不像"
//    （那样量到的主要是淡入的亮度变化）。见下面 `diff()` 的**亮度归一化**。
//
// ⚠️ 别把窗口往后挪到"淡入结束之后"（我试过 34..58）：那时
//    `slam` / `pop` / `shrink_in` / `size_*` / `hop_fast` **都已经落定**，
//    于是它们**本来就该长得一样** ⇒ 判据报出 45 对"撞车"，全是假警报。
const FRAMES = [14, 16, 18, 20, 22, 25, 28];

/**
 * ② 「动了」的阈值。
 *
 * ⚠️ 这个数**不是拍出来的**，是在实测分布里**取在空档上**（39 档跑完后按值排序）：
 *
 *   | 档位数 | ② 值 | 是什么 |
 *   |---|---|---|
 *   | **4** | **0.0047 ~ 0.0076** | `fade_only`（**设计上就没有动作**）、`vtext_thin/bold`（纯排版，`fontWeight` 在夹具里解析到同一份字体）、`vtext_shrink`（`scaleSpread` 是**稳态**，淡入结束时早已落定） |
 *   | ↑ 空档 ↑ | **0.008 ~ 0.036** | ←── 阈值取在这里 |
 *   | 35 | **0.0366 ~ 0.2263** | 真的在动的档 |
 *
 * ⚠️ `fade_only` 那 0.0047 **不是动作**，是 192×108 重采样的**噪声**：
 *    实测归一化之后两帧最大只差 **4/255**、且"差>30 的点"**为 0 个**。
 *    ⇒ 阈值压在噪声之上、真动作之下，取的 0.02（比噪声高一个数量级、
 *       比最小真动作低 1.8 倍）。
 *
 * ⚠️ 别把它调低到 0.005 —— 那样 `fade_only`（空壳档）也会"通过 ②"，
 *    而它**本来就没有动作**，判据就成了空转。
 */
const MOTION_MIN = 0.02;

const perTier = new Map();
const sigs = new Map();

for (const tier of TIERS) {
  const proj = projectFor(tier, tier);
  const gs = [];
  for (const f of FRAMES) {
    const png = render(proj, f, `${tier}-${f}`);
    gs.push(png ? gray(png) : null);
  }
  const got = gs.filter(Boolean).length;
  ok(got === FRAMES.length, `出图 ${tier}：${got} / ${FRAMES.length} 帧（底座没吐满帧）`);
  if (got < FRAMES.length) {
    perTier.set(tier, { ink: 0, motion: 0 });
    sigs.set(tier, null);
    continue;
  }
  const inks = gs.map(ink);
  const maxInk = Math.max(...inks);
  // ① 有墨（不是"看着成功、其实一个字都没有"）
  ok(maxInk >= 3, `① 有墨 ${tier}：动画窗口内最多只有 ${maxInk} 个墨点 —— 画不出来`);
  // ② 动了：窗口内至少一对帧显著不同
  let maxDiff = 0;
  for (let i = 0; i < gs.length; i += 1) {
    for (let j = i + 1; j < gs.length; j += 1) maxDiff = Math.max(maxDiff, diff(gs[i], gs[j]));
  }
  // ⚠️ 阈值按**归一化 RMS** 的量纲定：实测"某一档完全静止"时是 0.000000，
  //    而"有字在动"的档都在 0.02 以上（见下面的表）⇒ 0.002 有一到两个数量级的余量。
  // ⚠️⚠️ **② 只对"应当是动态"的档作数** —— 39 档里有 7 档**本来就不是动态的**。
  //
  // 实测把这 7 档单列出来之后，判据才对得上"这一档到底是什么"：
  //
  //   * `tilt_std/sm/rev` —— 是**静态倾角**（`tiltDeg` 一个常数），
  //     底座把它落成 `transform.rotation` 的**一个定值**，全程不变。
  //     ⚠️ 它是**样式**，不是动效 —— 硬要求它"动"就是**判据搞错了对象**。
  //   * `fade_only` —— 「只淡出」，**设计上就没有位移/缩放**；
  //     它的"看得见"由淡入淡出承担（那是**全档共有**的包络，不是这一档的特征）。
  //   * `vtext_thin/bold` —— 纯排版方向（竖排），静态；
  //     `fontWeight` 在夹具里解析到同一份字体，两者像素必然相同。
  //   * `vtext_shrink` —— `scaleSpread` 是**稳态**（各字大小不同但不变），
  //     淡入结束时早已落定。
  //
  // ⚠️ **这 7 档仍然要过 ①（有墨）与 ③（各档不同）** —— 只是不要求 ②。
  //    把它们写成"豁免"而不是"把阈值调低到 0.005"，是因为后者会让
  //    **真正的空壳档**（比如参数被清零的 `hop_std`）也混过去。
  // ⚠⚠ 2026-10-14（三轴收敛）：名单里原来是**已经被删掉的 39 档旧名**
  //   （`tilt_std/sm/rev` / `fade_only` / `vtext_thin/bold/shrink`）⇒ 豁免**一档也没命中**，
  //   而新名的四档（`tilt` / `vtext` / `fade` / `perchar_small`）反而被当成“该动而没动”报红。
  //   ⇒ 改成**收敛后的名字**（它们就是同一批设计）。
  const STATIC_BY_DESIGN = new Set([
    'tilt',            // 静态倾角（一个常数）—— 是**样式**不是动效
    'fade',            // 「只淡入淡出」，**设计上就没有位移/缩放**
    'vtext',           // 纯排版方向（竖排）
    'perchar_small',   // `scaleSpread` 是**稳态**（各字大小不同但不变）
  ]);
  if (STATIC_BY_DESIGN.has(tier)) {
    // 「设计上静态」的档：只要求它在窗口里**别乱变**（否则说明有意外动作）。
    ok(maxDiff < MOTION_MIN, `② 静止 ${tier}：它**设计上**不该有位移/缩放，`
      + `实测帧间差却有 ${maxDiff.toFixed(6)}（阈值 ${MOTION_MIN}）`
      + ` —— 有意外动作，或者它其实不是静态档`);
    perTier.set(tier, { ink: maxInk, motion: maxDiff, static: true });
    sigs.set(tier, Buffer.concat(gs));
    continue;
  }
  ok(maxDiff >= MOTION_MIN, `② 动了 ${tier}：窗口内最大帧间差只有 ${maxDiff.toFixed(6)}`
    + ` —— 静止的（阈值 ${MOTION_MIN}）`);
  perTier.set(tier, { ink: maxInk, motion: maxDiff });
  // 档签名：用第 24 帧（大多数档此时已经动完，形状最能代表这一档）
  sigs.set(tier, gs[3]);
}

// ③ 各档不同：两两比签名
let samePairs = [];
for (let i = 0; i < TIERS.length; i += 1) {
  for (let j = i + 1; j < TIERS.length; j += 1) {
    const a = sigs.get(TIERS[i]);
    const b = sigs.get(TIERS[j]);
    if (!a || !b) continue;
    if (diff(a, b) < 0.002) samePairs.push([TIERS[i], TIERS[j]]);
  }
}
/**
 * ⚠️ **允许撞车的成对**（必须逐条给出理由，不许用通配掩盖）。
 *
 * 这些是**结构上必然相同**的：底座的字体解析只按**文件名主干**找文件，
 * 而夹具字体目录里只有 `Default.ttf` ⇒ `fontWeight: 700` 与 `400`
 * **解析到同一个文件**，画面必然一模一样。
 * ⚠️ 真实环境里两者不同（字体库里有 Bold 那一份）—— 这是**夹具的已知收窄**，
 *    不是档位重复。**点名写在这里**，而不是放宽判据。
 */
const ALLOWED_SAME = [
  ['vtext_thin', 'vtext_bold'],
  ['vtext_thin', 'vtext_shrink'],
  ['vtext_bold', 'vtext_shrink'],
];
const allowed = new Set(ALLOWED_SAME.flatMap(([a, b]) => [`${a}=${b}`, `${b}=${a}`]));
const hard = samePairs.filter(([a, b]) => !allowed.has(`${a}=${b}`));
ok(hard.length === 0,
  `③ 各档不同：有 ${hard.length} 对档位画出**同一个画面**（档位名映射到了同一套参数）：`
  + hard.map(([a, b]) => `${a}=${b}`).join(', '));
if (samePairs.length > hard.length) {
  console.log(`  ⚠️ 另有 ${samePairs.length - hard.length} 对是**夹具的已知收窄**`
    + `（字体目录只有 Default.ttf ⇒ 400/700 解析到同一份）：`
    + ALLOWED_SAME.map(([a, b]) => `${a}=${b}`).join(', '));
}

// ---------------------------------------------------------------------------
// 报告
// ---------------------------------------------------------------------------

console.log('  #  档位                墨点  帧间差   判定');
console.log('  ' + '-'.repeat(56));
TIERS.forEach((t, i) => {
  const r = perTier.get(t) ?? { ink: 0, motion: 0 };
  const good = r.ink >= 3 && (r.static === true || r.motion >= MOTION_MIN);
  console.log(
    `  ${String(i + 1).padStart(2)}  ${t.padEnd(18)}${String(r.ink).padStart(4)}  `
    + `${r.motion.toFixed(4).padStart(7)}   ${good ? '✓' : '✗'}`
  );
});
console.log('  ' + '-'.repeat(56));
// ⚠️ 报出**档间最小距离**：证明 ③ 不是"勉强挤过阈值"。
//    若某两档距离贴着阈值，说明判据分不清它们 —— 那要**提分辨率**，不是放宽阈值。
{
  let minD = Infinity;
  let minPair = '';
  for (let i = 0; i < TIERS.length; i += 1) {
    for (let j = i + 1; j < TIERS.length; j += 1) {
      const a = sigs.get(TIERS[i]);
      const b = sigs.get(TIERS[j]);
      if (!a || !b) continue;
      const d = diff(a, b);
      if (d < minD) {
        minD = d;
        minPair = `${TIERS[i]} vs ${TIERS[j]}`;
      }
    }
  }
  console.log(`  档间最小距离 ${minD.toFixed(6)}（${minPair}）· 判定阈值 0.002`
    + ` ⇒ 余量 ${(minD / 0.002).toFixed(1)}×`);
}
const goodCount = TIERS.filter((t) => {
  const r = perTier.get(t) ?? { ink: 0, motion: 0 };
  if (r.ink < 3) return false;
  // 「设计上静态」的档：② 是"别乱动"，已经由断言保证；这里只查 ①。
  return r.static === true || r.motion >= MOTION_MIN;
}).length;
console.log(`  **有墨且可见地动了：${goodCount} / ${TIERS.length} 档**`);

rmSync(work, { recursive: true, force: true });

console.log('='.repeat(74));
if (problems.length > 0) {
  console.log(`✗ 39 档像素证据有 ${problems.length} 处问题（共 ${checks} 项判据）：`);
  for (const p of problems) console.log(`    · ${p}`);
  process.exit(1);
}
console.log(`✓ 39 档像素证据通过：${checks} 项判据 —— ${goodCount} / ${TIERS.length} 档有墨、可见地动、且互不相同`);
