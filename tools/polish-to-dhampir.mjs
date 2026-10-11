#!/usr/bin/env node
// 把本仓（YekiTrim）的 `polish.toml` 转译成 Dhampir 的工程文件（`*.doc.json`）。
//
// # 这份转译器是本仓的**出口**，不是产品代码
//
// 它活在 `tools/` 下（不在 `yekitrim` 二进制的编译/运行路径上），用来回答一个具体问题：
// **「YekiTrim 决定画什么、底座决定怎么画」这个分工，落到一份真工程上到底缺什么。**
// 所以它宁可**报出转不了的东西**，也不猜 —— 猜出来的部分看着像成功了，
// 而那正是最难归因的一类问题。
//
// 用法：
//
//     node tools/polish-to-dhampir.mjs <clip 目录> [--out 工程文件路径]
//
// 转不了 / 有歧义的地方一律打进报告（打到 stdout；`DHAMPIR_QUIET=1` 只留开头那几行统计），
// 并按严重程度分三档：
//
//   DROP   底座没有对应概念，**这一条会丢**
//   CLAMP  有对应概念但表达力不够，**转出来的与原意不同**
//   NOTE   转过去了，但有一处需要注意的语义差
//
// # 词表（正文里通篇出现这两个词；这份文件写于底座侧，搬进本仓时换过一次视角）
//
//   「底座」= Dhampir —— wgpu 预览 + 渲染底座，也就是这份转译器的**下游**
//   「参照」= **本仓自己的出片模板** `crates/yekitrim-polish/templates/index.html`
//             （canvas 那条腿：`renderFrame` / `getActiveXxx`）—— 它是本转译的**复刻对象**，
//             所以"参照侧的不一致"就是**本仓可以修的不一致**
//             正文里的 `index.html:NNN` / `templates/index.html` 都指这个文件（行号对着本仓的它）
//
// 规格（行为对照表 / DROP·CLAMP·NOTE 三级记录 / 怎么加一个新事件类型）见
// `docs/yekitrim-integration.md`；曲线那部分的守卫在 `scripts/check-yekitrim-translator.mjs`。

import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

// ---------------------------------------------------------------------------
// 效果词汇表（**唯一权威**：`spec/effects.json`）
// ---------------------------------------------------------------------------
//
// 加/删一个效果时，类型清单不许在转译器里再抄一份 —— 这份表由
// `scripts/check-effect-registry.mjs` 与本文件（以及前端、模板、engine.js、Rust）互相钉住。
// 这里只读**两个集合**：所有类型 id、以及"id → 中文名"，用途是**把报告里的措辞说准**
// （"词汇表里有但转译器没接" vs "连词汇表都没有"）。
//
// ⚠️ **读不到不算错**（打包形态可能没带上它）：那时退化成"不知道词汇表"，
// 报告退回原来那句"不认识的事件类型"，**不让缺一个 JSON 把出片搞挂**。
//
// ⚠️ **2026-10-09：发布包里 `effects.json` 从 `<程序目录>/spec/` 挪到 `<程序目录>/runtime/`**
//（与转译器合并成一个目录）。所以这里要**试两个位置**，而不是写死 `../spec`：
//   ① 同目录 `./effects.json`  —— 新布局（两个文件都在 `runtime/` 里）
//   ② `../spec/effects.json`  —— 老布局 + **开发树**（`tools/` 与 `spec/` 是仓库根下并列的两个目录）
// 顺序无所谓（两者不会同时存在），但两个都得试：只试新的 ⇒ 开发树里守卫全空转；
// 只试老的 ⇒ 发布包里退化成"不知道词汇表"（而它**不报错**，只是报告文案变粗）。
const EFFECT_VOCAB = (() => {
  const empty = { known: new Set(), labels: new Map(), loaded: false };
  const here = dirname(fileURLToPath(import.meta.url));
  for (const rel of [['effects.json'], ['..', 'spec', 'effects.json']]) {
    try {
      const raw = readFileSync(join(here, ...rel), 'utf8');
      const spec = JSON.parse(raw);
      return {
        known: new Set(spec.effects.map((e) => e.id)),
        labels: new Map(spec.effects.map((e) => [e.id, e.label])),
        loaded: true,
      };
    } catch {
      // 试下一个位置
    }
  }
  return empty;
})();

// ---------------------------------------------------------------------------
// 极其精简的 TOML 解析
// ---------------------------------------------------------------------------
//
// **只支持 polish.toml 用到的那一小撮语法**：表头、字符串、数字、布尔、
// 数组、行注释、`[[数组表]]`。不支持内联表、多行字符串、日期。
//
// 为什么不用现成的解析器：这份脚本要能在仓库里**直接跑**，
// 不引入依赖（底座的 scripts/ 一律零依赖）。语法不够用时**当场报错**，
// 不静默解析成错的东西。

// **导出是为了守卫能用**：`[[cover.effects]]` 那种"普通表里的数组表"曾让解析器崩
// （无条件把父级改成数组 → `[cover]` 整张表被丢掉 → 读 undefined 直接 TypeError），
// 症状是**整个工程转不出来**。守卫里那条用例就是钉它的。
export function parseToml(text) {
  const root = {};
  let current = root;

  const unquote = (raw) => {
    const value = raw.trim();
    if (value.startsWith('"') && value.endsWith('"') && value.length >= 2) {
      return value.slice(1, -1).replace(/\\"/g, '"').replace(/\\\\/g, '\\');
    }
    if (value.startsWith("'") && value.endsWith("'") && value.length >= 2) {
      return value.slice(1, -1);
    }
    return null; // 不是字符串
  };

  /// 去掉行内注释：**逐个字符走，记住自己在不在引号里**。
  ///
  /// 不能用正则图省事 —— `["#ffaa00", "#000000"]` 里的 `#` 是颜色，
  /// 而 `danmaku = ["aespa", "吉赛尔", ]` 里的 `#` 才是注释。
  /// 正则分不出这两者，会把颜色截成空串。
  const stripComment = (text) => {
    let quote = null;
    for (let i = 0; i < text.length; i += 1) {
      const ch = text[i];
      if (quote !== null) {
        if (ch === quote) quote = null;
        continue;
      }
      if (ch === '"' || ch === "'") {
        quote = ch;
        continue;
      }
      if (ch === '#') return text.slice(0, i);
    }
    return text;
  };

  const parseValue = (raw) => {
    const clean = stripComment(raw).trim();
    if (clean === 'true') return true;
    if (clean === 'false') return false;
    const str = unquote(clean);
    if (str !== null) return str;
    // 数组：只支持一层的字面量数组。
    if (clean.startsWith('[')) {
      if (!clean.endsWith(']')) throw new Error(`数组没闭合：${clean}`);
      const inner = clean.slice(1, -1).trim();
      if (inner === '') return [];
      // **尾逗号是合法的 TOML**（polish.toml 里就写了），
      // 所以要滤掉空项 —— 否则会去解析一个空串然后炸掉。
      return inner
        .split(',')
        .map((item) => item.trim())
        .filter((item) => item !== '')
        .map((item) => parseValue(item));
    }
    const num = Number(clean);
    if (!Number.isNaN(num) && clean !== '') return num;
    throw new Error(`不认识的 TOML 值：${clean}`);
  };

  // **先把多行数组拼成一行。** `polish.toml` 里的 `title_lines` / `danmaku`
  // 都是这么写的，逐行解析会在 `[` 那一行直接断掉。
  const lines = [];
  let pending = null;
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (pending !== null) {
      // 数组还没闭合：继续吃，但把行尾注释去掉。
      const body = line.replace(/(^|[^"'])--.*$/, '$1').trim();
      pending += ' ' + body;
      if (pending.includes(']')) {
        lines.push(pending.slice(0, pending.indexOf(']') + 1));
        pending = null;
      }
      continue;
    }
    if (line === '' || line.startsWith('#')) continue;
    // 行内注释：只在**不在引号里**的时候才算注释。
    const assignment = line.match(/^([A-Za-z0-9_.-]+)\s*=\s*(.*)$/);
    if (assignment && assignment[2].trim().startsWith('[') && !assignment[2].includes(']')) {
      pending = `${assignment[1]} = ${assignment[2].trim()}`;
      continue;
    }
    lines.push(line);
  }
  if (pending !== null) throw new Error(`数组一直没闭合：${pending}`);

  for (const rawLine of lines) {
    const line = rawLine;

    // `[[segments]]` / `[[segments.events]]`
    const arrayTable = line.match(/^\[\[(.+)\]\]$/);
    if (arrayTable) {
      const path = arrayTable[1].trim().split('.');
      const last = path.pop();
      let node = root;
      for (const key of path) {
        // 数组表的父级**可能只是一张普通表**（`[cover]` 之后再出现 `[[cover.effects]]`，
        // 这是合法 TOML）。以前这里无条件把父级改成数组 —— 那会把已经解析好的那张表
        // 整个丢掉，紧接着读 `undefined['effects']` 直接崩（clip-04 就是这么崩的）。
        if (Array.isArray(node[key])) {
          node = node[key][node[key].length - 1];
        } else if (node[key] === undefined) {
          // ⚠️ **中间那一段是「表」，不是「数组」**（2026-10 修，第 4 批实测）。
          //
          // `[[a.b]]` 的语义是"`a` 这张表里，`b` 是一个数组" —— 中间那一段 `a`
          // **永远是表**。以前这里写成 `node[key] = []`（照 `[[segments]]` 那种
          // "最后一段是数组"的形状抄的），于是 `[[edl.clips]]` **前面没有 `[edl]` 表头**时
          // 解析成 `{edl: [{clips: [...]}]}`：`config.edl.clips` 是 `undefined`
          // ⇒ 转译器把整条 EDL **当没有**，而 Rust（真 toml 解析器）读得出来。
          //
          // 症状是**消费点②与①分叉且不报错**（画面按整条源出片、音轨按 EDL 拼），
          // 正是规格 §3 R7 "预览 ≠ 成片" 那一类。
          // 现实里的触发面：Rust / 服务端落盘一律经 `toml::to_string_pretty`，
          // 它会写出 `[edl]` 表头，所以**自家写的工程不受影响** ——
          // 但**手改过的 / 第三方生成的** `polish.toml` 会。而 Rust 侧的读盘单测
          // （`legacy_project_has_empty_edl_and_writes_nothing` 用例 ④）**明确接受**
          // 只有 `[[edl.clips]]` 的文本 ⇒ 两端对同一份文件的读法必须一致。
          node[key] = {};
          node = node[key];
        } else {
          node = node[key]; // 普通表：直接进去，**不要动它的类型**
        }
      }
      if (!Array.isArray(node[last])) node[last] = [];
      const entry = {};
      node[last].push(entry);
      current = entry;
      continue;
    }

    // `[layout.liver.top_left]`
    const table = line.match(/^\[(.+)\]$/);
    if (table) {
      const path = table[1].trim().split('.');
      let node = root;
      for (const key of path) {
        if (node[key] === undefined) node[key] = {};
        node = node[key];
      }
      current = node;
      continue;
    }

    const assignment = line.match(/^([A-Za-z0-9_-]+)\s*=\s*(.+)$/);
    if (!assignment) throw new Error(`不认识的行：${line}`);
    current[assignment[1]] = parseValue(assignment[2]);
  }
  return root;
}

// ---------------------------------------------------------------------------
// 转译
// ---------------------------------------------------------------------------

const issues = [];
const drop = (where, what) => issues.push({ level: 'DROP', where, what });
const clamp = (where, what) => issues.push({ level: 'CLAMP', where, what });
const note = (where, what) => issues.push({ level: 'NOTE', where, what });

// ---------------------------------------------------------------------------
// 字体：库 id → 真族名
// ---------------------------------------------------------------------------

/// 库 id → **字体文件里的真族名**。由宿主（Rust）经 `--font-families <json>` 传进来。
///
/// 为什么必须有这一步：底座的 libass **按族名找字体**，而 `polish.toml` 里存的是**库 id**。
/// 两者在库里**确实会不一样**（实测：id `GenJyuuGothic-Bold` vs 族名 `Gen Jyuu Gothic Bold`），
/// 而名字对不上时 libass **不报错** —— 静默落 `ArialMT`，且 `lines_failed: 0`、`issues: []`。
/// 实测（同字体、同 `fontsdir`，只换名字）：
///
///     fontselect: (GenJyuuGothic-Bold, 400, 0)    -> ArialMT            ← 传 id
///     fontselect: (Gen Jyuu Gothic Bold, 400, 0)  -> GenJyuuGothic-Bold ← 传真族名
///
/// ⚠️ **库外的值原样返回**，一个字节都不动：老工程写 `font = "LXGW WenKai"` 那种
/// "系统字体族名"本来就不在库里，它也**不该**被改写（那时宿主走系统字体栈兜底）。
/// 这条与 `yekitrim_core::fonts::family_for` 是同一个口径（那边是 Rust 侧的权威）。
///
/// ⚠️ 映射由宿主给、本文件**不去读 `FONT.toml`**：转译器的纪律是零依赖、不探媒体、
/// 只读本仓的工程文件。字体库的解析规则（两种布局、`family` 字段、emoji 过滤）在 Rust 侧
/// 已经有一份，抄过来就是第二份会漂的实现。
let fontFamilies = {};

/// 给映射表赋值（CLI 解析 `--font-families` 时调；守卫可以直接调这个做逐值测）。
export function setFontFamilies(map) {
  fontFamilies = map && typeof map === 'object' ? map : {};
}

/// 见上。`raw` 空 / 不在映射里 → **原样返回**（`''` → `null`，契约里空就是"没指定"）。
export function fontFamilyFor(raw) {
  const id = typeof raw === 'string' ? raw.trim() : '';
  if (!id) return null;
  const mapped = fontFamilies[id];
  return typeof mapped === 'string' && mapped.trim() ? mapped.trim() : id;
}

/// 把 `"44.6%"` 或 `44.6` 变成比例。转不了就返回 null。
function ratio(raw) {
  if (typeof raw === 'number') return raw > 1 ? raw / 100 : raw;
  if (typeof raw !== 'string') return null;
  const text = raw.trim();
  if (text === 'auto') return null;
  const percent = text.match(/^(-?[\d.]+)%$/);
  if (percent) return Number(percent[1]) / 100;
  // `"86.47% 61.94%"` 这种两段式的先在上层拆开，到这儿只剩单个数。
  const plain = Number(text);
  if (!Number.isNaN(plain)) return plain > 1 ? plain / 100 : plain;
  return null;
}

/// `"86.47% 61.94%"` -> `{x, y}`（比例）。
function point(raw) {
  if (typeof raw !== 'string') return null;
  const parts = raw.trim().split(/\s+/);
  if (parts.length !== 2) return null;
  const x = ratio(parts[0]);
  const y = ratio(parts[1]);
  if (x === null || y === null) return null;
  return { x, y };
}

/// `"#ffaa00"` -> `{r, g, b, a}`（0..1）。四种写法都收：`#RGB` / `#RGBA` / `#RRGGBB` / `#RRGGBBAA`
/// —— YekiTrim 的配置里**四种都出现过**（CSS 本来就认这四种）。
///
/// ⚠️ 以前只认 6 位，`#140A28CC` 这种直接返回 `null`。后果**不是"少了个透明度"**：
/// 调用点是 `hexColor(...) ?? 兜底`，而 overlay 那处是 `if (!first) { drop(...) }` ——
/// **整条覆盖层事件被丢掉**（实测：演示工程三条 overlay 全丢，成片少了整块渐变色罩）。
export function hexColor(raw) {
  if (typeof raw !== 'string') return null;
  const match = raw.trim().match(/^#?([0-9a-fA-F]{3,8})$/);
  if (!match) return null;
  let hex = match[1];
  if (hex.length === 3 || hex.length === 4) {
    hex = [...hex].map((c) => c + c).join('');
  }
  if (hex.length !== 6 && hex.length !== 8) return null;
  const value = parseInt(hex.slice(0, 6), 16);
  return {
    r: ((value >> 16) & 0xff) / 255,
    g: ((value >> 8) & 0xff) / 255,
    b: (value & 0xff) / 255,
    a: hex.length === 8 ? parseInt(hex.slice(6), 16) / 255 : 1,
  };
}

// ---------------------------------------------------------------------------
// 贴纸包
// ---------------------------------------------------------------------------
//
// **YekiTrim 的 `sticker_tag` 是一个"意图词"，不是文件名。**
//
// 流程是两段的：LLM 先按语义挑一个 tag（`搞笑` / `震惊` / `看热闹`……），
// 再由 `Step2b` 拿这个 tag 去**贴纸包**里查出候选文件、挑一个、写回
// `event.name` + `event.source`（见 YekiTrim `llm/mod.rs:280`）。
//
// 而这份切片的 `polish.toml` 里 **`name`/`source` 都是空的** ——
// 就是说 **Step2b 从没跑过**（checkpoint 里也是 `name: None, source: None`）。
// 那时 `sticker_tag` 还是 LLM 的原始措辞（`搞笑`/`震惊`/`看热闹`），
// 而贴纸包里的 tag 是另一套词（`笑`/`惊讶`/`害怕`）。
//
// 所以这里**不能**拿 `sticker_tag` 直接当文件名去找 —— 一个都找不到。
// 要么按同义词映射，要么按关键字兜底。

/// 找一张贴纸的**真实文件**。返回绝对路径或 null。
///
/// 贴纸目录有三种布局，实测都遇到过：
///
/// 1. **平铺**：`assets/stickers/生气.gif`
///    （`最鱿鱼之人` 那次录制就是这样，`name` 也填好了）；
/// 2. **带包目录**：`assets/stickers/ysyk_0.9.0/生气.gif`
///    （`海市蜃楼之馆` 那次）；
/// 3. 任意一层子目录。
///
/// 所以按"平铺 → 已知包名 → 任意子目录"依次找，**找到第一个存在的**。
/// 不做模糊匹配 —— 名字对不上就报出来，不猜。
function findStickerFile(clipDir, fileName) {
  const base = join(clipDir, 'assets', 'stickers');
  const flat = join(base, fileName);
  if (existsSync(flat)) return { path: flat, uri: `assets/stickers/${fileName}`, dir: base };

  let entries;
  try {
    entries = readdirSync(base, { withFileTypes: true });
  } catch {
    return null;
  }
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const nested = join(base, entry.name, fileName);
    if (existsSync(nested)) {
      return {
        path: nested,
        uri: `assets/stickers/${entry.name}/${fileName}`,
        dir: join(base, entry.name),
      };
    }
  }
  return null;
}

/// 按**任意允许的图片扩展名**找一个贴纸（`gif/png/jpg/jpeg/webp`）。
///
/// ⚠️ 为什么要它：`event.name` 是**不带扩展名**的，而贴纸库里允许的扩展名不止 `.gif`
/// —— 只试 `.gif` 会让 PNG 贴纸"找不到"（本仓的渲染侧踩过同一个坑：写死 `.gif` 时
/// PNG 贴纸进得了项目、预览显示得出来，**成片里一张都没有**）。
function findStickerAnyExt(clipDir, stem) {
  if (!stem) return null;
  for (const ext of ['gif', 'png', 'jpg', 'jpeg', 'webp']) {
    const found = findStickerFile(clipDir, `${stem}.${ext}`);
    if (found) return found;
  }
  return null;
}

/// 把 SRT 解析成 `{startMs, endMs, text}`。
///
/// 只认最基本的 SRT 块：序号 / 时间行 / 文本行（可多行）。**认不出就报错**，
/// 不静默跳过 —— 一条弹幕悄悄消失是查不出来的。
function parseSrt(text) {
  const cues = [];
  // 统一换行，然后按空行切块。
  const blocks = text.replace(/\r\n?/g, '\n').split(/\n{2,}/);
  for (const block of blocks) {
    const lines = block.split('\n').map((line) => line.trim()).filter((line) => line !== '');
    if (lines.length < 2) continue;
    // 第一行是序号（可能缺），找时间行。
    const timeIndex = lines.findIndex((line) => line.includes('-->'));
    if (timeIndex < 0) continue;
    const match = lines[timeIndex].match(
      /(\d+):(\d+):(\d+)[,.](\d+)\s*-->\s*(\d+):(\d+):(\d+)[,.](\d+)/,
    );
    if (!match) continue;
    const ms = (h, m, s, frac) =>
      Number(h) * 3600000 + Number(m) * 60000 + Number(s) * 1000 + Number(frac.padEnd(3, '0').slice(0, 3));
    const body = lines.slice(timeIndex + 1).join('\\N');
    if (body === '') continue;
    cues.push({
      startMs: ms(match[1], match[2], match[3], match[4]),
      endMs: ms(match[5], match[6], match[7], match[8]),
      text: body,
    });
  }
  return cues;
}

/// 毫秒 -> SRT 时间 `HH:MM:SS,mmm`。
function srtTime(ms) {
  const total = Math.max(0, Math.round(ms));
  const msec = total % 1000;
  const totalSec = Math.floor(total / 1000);
  const s = totalSec % 60;
  const m = Math.floor(totalSec / 60) % 60;
  const h = Math.floor(totalSec / 3600);
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')},${String(msec).padStart(3, '0')}`;
}

/// 六条轨道**没设过显式开关**时的默认值 —— 纯函数，**与 Rust
/// `ProjectFile::lane_default` 逐条一致**（那张表的单测在 `project/config.rs`；
/// 这一份的对拍在 `scripts/check-yekitrim-translator.mjs::checkLaneSwitches`，含变异自证）。
///
/// 用户 2026-10-06 口径：竖屏素材不再**硬**关掉运镜/特效，改成"默认关、眼睛一点就能开"。
///   * 运镜 / 视觉特效：**素材竖 或 画布竖** ⇒ 默认关；
///   * 音效：**画布竖** ⇒ 默认关（与素材方向无关）；
///   * 字幕 / 弹幕 / 贴纸：恒默认开。
///
/// ⚠️ `portraitSrc` 传 `null`（探不到源尺寸）时**不算竖**（与 Rust 注入的 `vertical` 同口径）。
export function laneDefaults(portraitSrc, portraitCanvas) {
  const portrait = portraitSrc === true;
  const canvas = portraitCanvas === true;
  return {
    subtitles: true,
    danmaku: true,
    camera: !(portrait || canvas),
    sticker: true,
    effects: !(portrait || canvas),
    sfx: !canvas,
  };
}

/// 视频素材的扩展名（**必须与 Rust `sticker_index::is_video_file_name` 同集合**）。
///
/// ⚠️ 这是"一份判据两处实现"里不得不重复的那一处：判据的真源在 Rust
/// （`crates/yekitrim-polish/src/assets/sticker_index.rs`，它决定**什么能进素材库**），
/// 而转译器是零依赖的 node 脚本、不 import 本仓的任何东西（`tools/AGENTS.md` 的硬规矩），
/// 所以只能照抄一份。**改一边必须改另一边** —— 集合漂移的症状是"库里能看到这个素材、
/// 出片时转译器说项目里没有它"，而用户看不出是判断标准不一致。
const VIDEO_EXTS = ['.mp4', '.mov', '.mkv', '.webm', '.avi'];

/// **叠化的重叠时长（秒）** —— 相邻两段在成片时间轴上重叠这么久，用来做交叉淡化。
///
/// # 为什么是 0.5s（规格 §11.3 说"典型 0.3~1.0s"）
///
/// 取**区间中位**：0.3s 在 60fps 下只有 18 帧，淡化窗口短到肉眼只看到"糊一下"，
/// 而叠化的判据恰恰是"中点那一帧两层同时可见"—— 窗口太短时中点离两端都近，
/// 与硬切的区分度下降。1.0s 又太长，会让"两段各让出半秒"变得明显。
/// `.workbuddy/edl-transition-probe/` 的探针用的就是 30 帧 @60fps = **0.5s**，
/// 所以这个值是**实测过的**（最大帧间 YAVG 跳变 2.22 vs 硬切 26.97）。
///
/// ⚠️ **它是一个常量、不是每段可调的落盘字段**：落盘就有两个真源（工程里一个、
/// 这里一个），改一处会让另一处静默漂 —— 与 `out_start` 不落盘同一条纪律
/// （规格 §2.2 规则 1）。将来真要做成可调，**必须**先把它写成 30 处消费点同源的那一份。
export const DISSOLVE_OVERLAP_SEC = 0.5;

/// 叠化的重叠**帧数**（`round(秒 × fps)`，与 `edl_clip_frames` 的量化口径同一条）。
///
/// ⚠️ **必须 ≥1 且 < 该段自己的长度**：重叠为 0 时"叠化"退化成硬切（静默），
/// 而重叠 ≥ 段长会把前一段整段吃掉（那不是叠化，是把前一段删了）。
/// 调用点据此报 DROP，**不在这里悄悄夹**（夹了就是"改用户的时间线而不说"）。
export function dissolveOverlapFrames(fps) {
  if (!Number.isFinite(fps) || fps <= 0) return 0;
  return Math.max(0, Math.round(DISSOLVE_OVERLAP_SEC * fps));
}

/// 一个 EDL 段指向的**视频素材 id**（`null` = 本条片段）。
///
/// # 这是"素材库来源"判据的**唯一一份**（三处消费点的 JS 那一侧）
///
/// 另两处：Rust `EdlClip::is_asset` / `asset_id_ref`（`yekitrim-render-core/src/types.rs`，
/// 出片腿与音轨组装读它）、前端 `webui/src/polish/edl.js::edlSourceKind`（成片轨上色 + 角标）。
/// 三处分歧的症状是"**预览里那段是素材、成片里是本条**"（或反过来），而**不报错** ——
/// 所以口径按逐字写在下面，改一处必须三处一起看。
///
/// 口径（与 Rust 那两条逐字相同）：
///  · `source_kind` 必须是**小写** `'asset'`（`'Asset'` / `'ASSET'` 不认 —— 别开始宽容，
///    工程文件是机器写的，宽容只会让"拼错"从"看得出来"变成"看不出来"）；
///  · `asset_id` 必须**非空、非全空白**（空串会让这一段静默退化成"素材库里没有这个文件"，
///    而那个失败在出片里表现为"这一段用回本条片段"，用户看不出区别）；
///  · 其余一切（字段缺席 / `source_kind = 'clip'` / 认不出的档位）⇒ `null` = 本条片段。
///
/// ⚠️ **`asset_id` 是素材名（去扩展名），不是路径**（`assets/AGENTS.md` 那条纪律）：
/// 项目里那一份在 `<项目>/assets/videos/<文件名>`，而"哪一个文件"由**库里的名字**查出来。
export function edlClipAssetId(clip) {
  if (!clip || typeof clip !== 'object') return null;
  if (clip.source_kind !== 'asset') return null;
  const id = typeof clip.asset_id === 'string' ? clip.asset_id.trim() : '';
  return id === '' ? null : id;
}

/// `EdlClip::transition_in` 的取值 —— **`cut` 是缺省**（字段缺席 = 硬切）。
export const EDL_TRANSITION_CUT = 'cut';
/// 叠化（跨轨重叠 + 上层 `opacity` 0→1 线性关键帧）。
export const EDL_TRANSITION_DISSOLVE = 'dissolve';

/// 一段的**入场转场**（`null` = 硬切）。
///
/// # 三处消费点同源的那一份（JS 侧）
///
/// 另两处：Rust `EdlClip::transition_kind`（`yekitrim-render-core/src/types.rs` —— 音轨
/// 交叉淡化读它）、前端 `webui/src/polish/edl.js::edlTransition`（成片轨角标）。
/// 分歧的症状是"**预览里某段带转场、成片里是硬切**"（或反过来），而**不报错**。
///
/// 口径（与 Rust 那条逐字相同）：
///  · `transition_in` 必须是**小写** `'dissolve'` —— `'Dissolve'` / `'DISSOLVE'` 不认
///    （工程文件是机器写的，宽容只会让"拼错"从"看得出来"变成"看不出来"）；
///  · 其余一切（字段缺席 / `'cut'` / 认不出的档位）⇒ `null` = 硬切。
///
/// ⚠️ **认不出 = 硬切，但必须在报告里点名**（调用点会打一条 NOTE）——
/// 静默当硬切会让用户以为"我选了叠化"（与 §11.6 那句"宁可没有，不要一个看着能点、
/// 出片没效果的选项"同一条纪律）。
export function edlClipTransition(clip) {
  if (!clip || typeof clip !== 'object') return null;
  return clip.transition_in === EDL_TRANSITION_DISSOLVE ? EDL_TRANSITION_DISSOLVE : null;
}

/// 算出**保留区间**（源时间，帧）并给每段标上**输出起点** `outStart`。
///
/// 这是"源时间 → 成片时间"那张映射表的构造处，画面层 / 音轨 / 字幕弹幕音效
/// **三处都必须消费它的产物**（`keepRanges` / `remapFrame` / `remapCueMs`），
/// 否则音画会各自错位（而**不会报错**）。
///
/// # 三个分支
///
/// 1. **`clips` 非空**（`config.edl`，规格 §2.2 规则 3 的另一半）—— 成片时间线
///    **显式给出来了**：数组顺序就是成片播放顺序，区间可重复、可乱序。`keepMuted`
///    在这条路上**不再短路**（§5：EDL 非空时"唯一那一刀"就是 EDL 本身，尾部那道
///    `cut_ranges` 与 `mute_ranges` 一起被忽略）。`source_start` / `source_end` 取整成
///    源帧号后，若它已被用户改小（`end = start`），我们**只改端点、只许变大**地吃掉
///    这段与他自己的意图冲突的屏蔽段 —— 见下面 `clips.length > 0` 那一段的注释。
/// 2. `keepMuted = false`：**压缩时间线**（`mute_ranges` 的补集）—— 预览那条腿。
///    客户端靠"落在被掐掉的段里 = 没有对应帧"去跳屏蔽区间。
/// 3. `keepMuted = true`：**整条一条** —— 出片那条腿（`2026-09-30` 起）。
///    出片尾部还有一道 `Ffmpeg::cut_ranges`（Rust 侧的整段重编码）**按源时间**剪；
///    这里若先掐一遍、尾部再剪一遍 = **剪两次**：实测成片短一大截，
///    而且音轨是从**整条**源视频抽的、没跟着压缩，第一刀之后全错位。
///    ⇒ 出片给"整条时间线"，屏蔽区间交给尾部那一剪（两边口径就统一了）。
///    ⚠️ 这条只服务**空 EDL 的老工程**（存量零回归 R4）；EDL 非空时走第 1 条。
///
/// ⚠️ **`outStart` 是累加出来的**，所以它必须在这里算、不能各消费点各算一遍 ——
/// 三处各推一遍的症状是"块位置差一点点"且**不报错**（同 `check-grid-parity.mjs` 那条教训）。
///
/// ⚠️⚠️ **EDL 分支必须与 `Edl::resolve_clips` 逐值同构**（消费点②，规格 §3）：
/// 它**不看 `mute_ranges`、也不看 `totalFrames`** —— 有 EDL 时屏蔽整个被忽略
/// （§2.3；`ProjectFile::resolve_edl` 的注释同一句）。**这里多写一条规则就是第 2 份推导**，
/// 而两份推导的分歧只会表现为"成片短一段"，不报错。实测踩过：多求了一次与
/// `mute_ranges` 的交集 ⇒ 画面比派生值少 180 帧，靠合流前那条时长断言才发现。
///
/// @param {number} totalFrames 源总帧数
/// @param {Array<{start:number,end:number}>} muteFrames 屏蔽区间（**已排序、已过滤空段**）
/// @param {boolean} keepMuted 见上（**只在 `clips` 为空时生效**）
/// @param {Array<{source_start:number,source_end:number,speed?:number,source_kind?:string,asset_id?:string}>} [clips] `config.edl.clips`（秒域）
/// @param {number} [fps] 源帧率（`clips` 秒 → 帧的量纲；缺省不转 EDL）
/// @returns {Array<{start:number,end:number,outStart:number,speed:number,assetId:(string|null)}>}
export function buildKeepRanges(totalFrames, muteFrames, keepMuted, clips = [], fps = 0) {
  const keepRanges = [];
  if (Array.isArray(clips) && clips.length > 0 && fps > 0) {
    // ---- 成片时间线（EDL）显式给出 ----
    //
    // ⚠️⚠️ **这段必须与 `Edl::resolve_clips` **逐值同构**，一个改动都不许有。**
    // 它是"三处消费点同源"（规格 §3 / §9 R7）里的**消费点②**，另外两处是
    // Rust 的出片腿（`resolve_edl`）与前端（批 3）。**两边不一致就是画面与音轨
    // 各自剪法不同，而完全没有报错** —— 实测过一次：这里多求了一次与
    // `mute_ranges` 的交集，而 Rust 侧不求，于是画面那条腿少出 180 帧（15.0s vs 派生 18.0s），
    // 是合流前那条时长断言（`Ffmpeg::mux_checked`）把它点出来的。
    //
    // ⚠️ **`mute_ranges` 在有 EDL 时整个被忽略**（规格 §2.3 明写；`ProjectFile::resolve_edl`
    // 的文档注释也写着同一句）。这里**不许**自己再求一次交 —— 那会造出第二份推导，
    // 而两份推导的分歧只会表现为"成片短一段"，不会报错。
    //
    // 口径（与 `edl_clip_frames` 逐式）：
    //   `sf = round(source_start*fps)`、`ef = round(source_end*fps)`、
    //   `dur = round((ef - sf) / speed)`。
    // ⚠️ **不是 `round((source_end-source_start)*fps/speed)`** —— 非帧对齐的秒数上两者差 1 帧，
    //    而 `out_start` 是累加值 ⇒ 那 1 帧会推着后面**所有**段一起偏。
    for (const clip of clips) {
      const speed = typeof clip.speed === 'number' && Number.isFinite(clip.speed) && clip.speed > 0
        ? clip.speed
        : 1.0;
      const sf = Math.round((clip.source_start ?? 0) * fps);
      const ef = Math.round((clip.source_end ?? 0) * fps);
      if (!(ef > sf)) continue; // 非法段跳过（与 `Edl::resolve_clips` 同口径）
      keepRanges.push({
        start: sf,
        end: ef,
        // 该段在成片里占的**帧数**（`Edl::resolve_clips` 逐式）：源帧数 / speed，四舍五入。
        durFrames: Math.max(1, Math.round((ef - sf) / speed)),
        speed,
        // ⚠️ **这一段画面从哪个 asset 来**（规格 §11.4 / 第 6 批）。
        //
        // 判据与 Rust `EdlClip::is_asset` / 前端 `edl.js::edlSourceKind` **同一份口径**：
        // `source_kind === 'asset'` 且 `asset_id` 非空才算素材段，其余一律 `null`
        // （= 本条片段）。三处分歧的症状是"预览里那段是素材、成片里是本条"，而**不报错**。
        //
        // ⚠️ `asset_id` 里存的是**素材名（去扩展名）**，不是路径 —— 与库里的
        // `VideoAsset::name`、以及项目里那份拷贝的定位口径一致（`assets/AGENTS.md`）。
        assetId: edlClipAssetId(clip),
        // ⚠️ **第 7 批（task-10）**：这一段的**入场转场**（`null` = 硬切）。
        //
        // ⚠️ **它是"派生"还是"落盘"的分界线**：`transition_in` 落盘（第 6 批定形），
        // 而**重叠帧数不落盘** —— 它就是下面 `DISSOLVE_OVERLAP_SEC` 这一个常量乘 fps。
        // 不落盘的理由与 `out_start` 同一条（规格 §2.2 规则 1）：两个真源必然漂。
        transition: edlClipTransition(clip),
      });
    }
    // ⚠️ `totalFrames` 在这里**不参与裁剪**：EDL 的段可以引用到"探到的时长"之外
    // （时长探短、或素材时间基与容器标称不符），而 `resolve_clips` 不看总时长 ——
    // 这里裁一刀就会比派生值**短**，然后 `-shortest` 静默截断。
  } else if (keepMuted) {
    // 老工程那两条路**没有 EDL**，画面一律来自本条片段（`source_kind` 那时还不存在）。
    keepRanges.push({ start: 0, end: totalFrames, assetId: null });
  } else {
    let cursor = 0;
    for (const mute of muteFrames) {
      if (mute.start > cursor) keepRanges.push({ start: cursor, end: mute.start, assetId: null });
      cursor = Math.max(cursor, mute.end);
    }
    if (cursor < totalFrames) keepRanges.push({ start: cursor, end: totalFrames, assetId: null });
  }
  // 每个保留区间的**输出起点**（把前面掐掉的长度减掉）。
  //
  // ⚠️ **帧号域累加，不是秒域**（规格 §9 R8）：`outStart = Σ 前面每段的时长`是**累加**，
  // 在秒域累加时单段一次 f64 舍入会推着后面**所有**段一起偏（段越多、变速越碎越明显）。
  // EDL 段还可以带 `speed` —— 一变速这条就更不能写在秒域。
  let outputCursor = 0;
  for (const range of keepRanges) {
    range.outStart = outputCursor;
    outputCursor += range.durFrames ?? (range.end - range.start);
  }
  return keepRanges;
}

/// 把一串 cue 写成 SRT。用于**重映射之后的字幕**（裁剪过的时间线）。
function cuesToSrt(cues) {
  const out = [];
  cues.forEach((cue, index) => {
    out.push(String(index + 1));
    out.push(`${srtTime(cue.startMs)} --> ${srtTime(cue.endMs)}`);
    // 解析时把多行折成了 `\N`，写回去要还原成真换行。
    out.push(cue.text.replace(/\\N/g, '\n'));
    out.push('');
  });
  return out.join('\n');
}

/// 毫秒 -> ASS 时间 `H:MM:SS.cc`（**厘秒**，两位）。
function assTime(ms) {
  const totalCs = Math.round(ms / 10);
  const cs = totalCs % 100;
  const totalSeconds = Math.floor(totalCs / 100);
  const s = totalSeconds % 60;
  const totalMinutes = Math.floor(totalSeconds / 60);
  const m = totalMinutes % 60;
  const h = Math.floor(totalMinutes / 60);
  return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}.${String(cs).padStart(2, '0')}`;
}

/// 把一串 cue 写成**底座弹幕子系统认的 ASS**。
///
/// # 为什么必须转
///
/// 底座的弹幕轨吃的是 `AssetKind::Subtitle` + 内容是 **ASS** 的文件
/// （一条 `Dialogue` 一行弹幕，见 `crate::danmaku::parse_ass_danmaku`）。
/// 而 YekiTrim 这边给的是 **`clip.dm.srt`** —— 一个"SRT 外壳 + 里面塞了
/// ASS 覆盖标签"（`{\move(...)}`、`{\pos(...)}`、`{\c&H...}`）的混合体。
/// 格式不是底座要的那个，所以要转一道。
///
/// **覆盖标签原样留着**：底座解析时本来就会把它们剥掉
/// （`ass_text_to_plain`），而且 `\move` 的坐标**故意不参与布局** ——
/// 落点由 `DanmakuSpec` 与目标尺寸统一算，两端拿同一份。留着的好处是
/// "这份 ASS 就是 YekiTrim 那份的直译"，下一个人对着两边能看懂。
/// 把 `\move(x1,y1,x2,y2)` 补成 **6 参数** `\move(x1,y1,x2,y2,0,travelMs)`。
///
/// # 为什么要补
///
/// ASS 的 `\move` 有两种写法，含义**不同**：
///
/// * 4 个参数 —— 跨的是 **cue 窗口**（起点到终点由 cue 的 `Start`/`End` 定）
/// * 6 个参数 —— **显式**给出移动的起止时刻（毫秒，相对 cue 起点）
///
/// "滚动时长"是后者那个 `t2 - t1`。YekiTrim 的 `progress = (el - fadeIn) / travel`
/// 用的就是**逐条的** travel（它按文本字节数算，四条实测 15.52/10.40/12.11/9.76 秒），
/// 而底座契约里的 `DanmakuSpec::duration_ms` 是**轨道级**的一个值 —— 只能取平均。
///
/// 不补的话底座读不到逐条时长，就退回那个平均值：**长句滚得太快、短句滚得太慢**，
/// 症状是"同一时刻参照的弹幕已经在左边、底座的还在右边"。
///
/// 只认 4 参数那一版才改写：已经是 6 参数的（手写的素材）**原样不动**。
///
/// ⚠️ **静态 cue 也补**（2026-10-01，底座回执 B4 的配方）：只有 `\pos(x,y)`、没有 `\move` 的那一类
/// 也要给逐条时长，否则底座读不到 `t2 - t1` 就退回**轨道级** `duration_ms`（= 平均值 ⇒ 长句短句都不对）。
/// 底座的说明是"**只读 `t2 - t1`，坐标忽略**"，所以补一个零位移的 `\move(0,0,0,0,0,travel)` 即可。
/// 补在 `\pos` **同一个覆盖块里**（就追加在那个标签后面），不改动坐标、也不新增块。
///
/// 我们这条链**目前不会产生 `\pos`**（全仓 grep `\pos(` 为 0；素材只有 `\move`）——
/// 这条是给**外部工具写的 SRT/ASS** 兜底的，所以判据里专门钉住"有 `\pos` ⇒ 必须补"和
/// "没有任何定位标签 ⇒ 一个字都不许动"（后者是纪律：不替参照发明位置）。
export function withMoveTravel(text, travelMs) {
  if (!Number.isFinite(travelMs) || travelMs <= 0) return text;
  const moved = String(text).replace(
    /\\move\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*,\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)/g,
    (whole, x1, y1, x2, y2) => `\\move(${x1},${y1},${x2},${y2},0,${Math.round(travelMs)})`,
  );
  if (moved !== String(text)) return moved;
  // 静态 cue：把零位移 `\move` 追加在 `\pos(...)` 之后（同一个覆盖块内）
  const POS = /\\pos\(\s*[-\d.]+\s*,\s*[-\d.]+\s*\)/;
  if (!POS.test(moved)) return moved;
  return moved.replace(POS, (tag) => `${tag}\\move(0,0,0,0,0,${Math.round(travelMs)})`);
}

/// 弹幕淡入时长（毫秒）—— 与参照 `getActiveDms` 的 `fadeIn = 0.3` 逐值一致
/// （`crates/yekitrim-polish/templates/index.html`，同一份也在 `webui/src/polish/engine.js`）。
export const DANMAKU_FADE_IN_MS = 300;

/// 弹幕在屏窗口的**终点**（毫秒）：至少要让这条滚到左边出画，才允许它消失。
///
/// # 为什么必须抬
///
/// 参照是**两口独立的钟**：滚动由 `travel` 决定（`progress = (el - fadeIn) / travel`、
/// `min(progress, 1)`），而**渐隐钉在 cue 自己的 `End` 上**
/// （`if (t < d.start || t >= d.end) continue` + `d.end - 0.2` 起渐隐）—— 两者之间本来
/// 没有任何约束。底座照抄了同一套语义（`dhampir-timeline/src/danmaku.rs::layout`
/// 的 `exit = cue.end_ms`），于是两边都会"**还没走到左边缘就渐隐**"，
/// 唯一的"已出屏"判据（`x < -textW - 50`）永远轮不到执行。
///
/// 录播姬 XML 给的窗口是**固定 8 秒**（`yekitrim-core/src/danmaku/mod.rs::parse_bililive_xml`），
/// 而中文弹幕的 travel 普遍 8.5~16 秒（`(1080 + 字节数×32)/150`）—— 实测
/// 「还能续约吗」（15 字节、travel 10.40s）在距左边缘 **337px** 处开始渐隐，
/// 13 个汉字那条（39 字节、travel 15.52s）在**屏幕中段 728px** 处就没了。
///
/// 参照那边 `assign_danmaku_tracks` 占轨道用的就是 `start + travel`（分配器早就认为
/// 一条弹幕在屏 `travel` 秒），只有 `end` 这个字段没跟上 —— 这里把它对齐：
/// `max(素材 End, start + fadeIn + travel)`，渐隐整段落在字**已经出画之后**。
///
/// **只抬不压**：素材显式写了更长窗口的（滚到左边停一会儿再消失）保持原样。
/// `travelMs` 读不出来（手写素材没写 6 参数 `\move`）时按 0 处理 —— 那时底座会回退到
/// 轨道级的 `duration_ms`，这里不猜它的值。
///
/// ⚠️ 与 `yekitrim_core::danmaku::clamp_display_windows` 是同一条规则的两份实现
/// （Rust 那份管 YekiTrim 自己的 `data/danmaku.json`）；改一边要改另一边。
export function danmakuExitMs(startMs, endMs, travelMs) {
  const travel = Number.isFinite(travelMs) && travelMs > 0 ? travelMs : 0;
  const need = startMs + DANMAKU_FADE_IN_MS + travel;
  return endMs > need ? endMs : need;
}

/// ⚠️⚠️ `kind: 'text'` 那条路**必须**传 `danmaku: false`。
///
/// `cuesToAss` 是给**弹幕**写的，而弹幕的出点规则是"至少滚到左边缘出画"
/// （`danmakuExitMs`：`max(endMs, start + 300 + travel)`）。文字元素**不滚**，
/// 却被同一条规则处理 ⇒ 每条 cue 的出点都被抬到 `start + 300ms`。
///
/// ★ **这个缺陷在打字机之前看不出来**：一条文字元素只有**一条** cue，
///   被抬到 300ms 也好、抬到 10 秒也好，只要原时长 ≥300ms，
///   `max()` 取的就是原值 ⇒ **行为完全正确**。而打字机把它拆成十几条短 cue
///   （每条 60~70ms）之后，`max()` 一律取 300ms ⇒
///   **所有 cue 互相重叠、每条都比自己该在的位置长 4 倍** ——
///   表现为"打字机完全不生效，文字一次全冒出来"（正是我们想修的那个观感）。
///
/// ⚠️ 所以这里加一个**显式开关**而不是改 `danmakuExitMs`：
///   那条规则对弹幕是**对的**（有实测支撑，改它会让弹幕提前消失）。
function cuesToAss(cues, { font = 'Sans', fontSize = 42, danmaku = true } = {}) {
  const out = [];
  out.push('[Script Info]');
  out.push('ScriptType: v4.00+');
  out.push('WrapStyle: 2');
  out.push('ScaledBorderAndShadow: yes');
  out.push('Title: 由 polish.toml 的 danmaku 素材转译');
  out.push('');
  out.push('[V4+ Styles]');
  out.push(
    'Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, ' +
      'Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, ' +
      'Shadow, Alignment, MarginL, MarginR, MarginV, Encoding',
  );
  out.push(
    `Style: Default,${font},${fontSize},&H00FFFFFF,&H000000FF,&H00000000,&H80000000,` +
      '0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1',
  );
  out.push('');
  out.push('[Events]');
  out.push('Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text');
  for (const cue of cues) {
    out.push(
      `Dialogue: 0,${assTime(cue.startMs)},${assTime(
        // ⚠️ 文字元素**原样用 `endMs`**（它不滚，"抬起出点"那条规则不适用）。
        danmaku ? danmakuExitMs(cue.startMs, cue.endMs, cue.travelMs) : cue.endMs,
      )},Default,,0,0,0,,${withMoveTravel(cue.text, cue.travelMs)}`,
    );
  }
  out.push('');
  return out.join('\n');
}

/// YekiTrim 的贴纸盒子边长基准，**短边 1080 时为 280px**。
///
/// 抄自 YekiTrim `process/placement.rs:36`：`STICKER_SIZE = 280.0`，
/// 而 `sticker_size() = STICKER_SIZE * min(W,H)/1080`。
/// 落点 `x`/`y` 是**这个盒子的左上角**（同文件 276 行的注释）。
///
/// 我第一版猜 220px，实测 YekiTrim 成片里量出来约 290px —— 猜的和真的是两回事。
const STICKER_BOX_BASE = 280.0;

/// 解析 YekiTrim 的描边串，形如 `"12px #403c3b"` —— **一个字段里两个值**。
///
/// 解析不出就退回给的那对兜底值，并**不静默**（调用方看不到返回值里的这一层，
/// 所以这里只负责"尽快给出一个合理的默认"，准确性由转译器的 DROP/CLAMP 清单负责）。
/// ⚠️ **export 出来是为了守卫**（`scripts/check-yekitrim-translator.mjs` 逐值钉它）：
/// 它有一条**静默**的失败模式 —— 颜色正则收窄成只认 6 位时 `#fff` 会**悄悄**回落到兜底深灰：
/// 用户选白色描边、成片是深灰，而**报告里一行差异都没有**。`isEntryPoint` 那道门保证直接跑仍是 CLI。
export function parseStroke(raw, fallbackWidthPx, fallbackColor) {
  const color = hexColor(fallbackColor) ?? { r: 0, g: 0, b: 0 };
  const result = {
    widthPx: fallbackWidthPx,
    color: [
      Math.round(color.r * 255),
      Math.round(color.g * 255),
      Math.round(color.b * 255),
      255,
    ],
  };
  if (typeof raw !== 'string') return result;
  const widthMatch = raw.match(/([\d.]+)\s*px/);
  if (widthMatch) result.widthPx = Number(widthMatch[1]);
  // ⚠️ **`{3,8}` 而不是 `{6}`**（2026-10 修）：`hexColor` 本来就能展开 `#RGB` / `#RGBA` /
  //    `#RRGGBB` / `#RRGGBBAA`（本仓 `polish.toml` 里四种都出现过），但**外层这个正则只认 6 位**
  //    ⇒ `#fff` / `#abc` 匹配不上、`hexColor` 根本走不到，于是**静默回落到兜底深灰**。
  //    症状极具迷惑性：用户在编辑器里选"白色描边"、成片是深灰，**报告里一行差异都没有**。
  //    ⚠️ 用 `{3,8}` 之后 `#RRGGBBAA` 会被吃成 8 位——`hexColor` 认它（忽略 alpha，与
  //    `outline` 只有开/关一致），这比"整条回落"好得多。
  const colorMatch = raw.match(/#[0-9a-fA-F]{3,8}/);
  if (colorMatch) {
    const parsed = hexColor(colorMatch[0]);
    if (parsed) {
      result.color = [
        Math.round(parsed.r * 255),
        Math.round(parsed.g * 255),
        Math.round(parsed.b * 255),
        255,
      ];
    }
  }
  return result;
}

/// 把 `{r,g,b}`（0..1）转成 RGBA 字节数组。
function rgbaBytes(color, alpha = 255) {
  return [
    Math.round(color.r * 255),
    Math.round(color.g * 255),
    Math.round(color.b * 255),
    alpha,
  ];
}

/// 探一个图片/动图的**帧数与像素尺寸**。探不到的部分是 null。
///
/// 用 `ffprobe`（底座原生侧本来就依赖 ffmpeg）。**不给默认值假装成功** ——
/// **只问宽高**的探针 —— 比 [`probeImage`] 快得多。
///
/// `probeImage` 带 `-count_frames`，对一条长视频要**整条解码**才能数出帧数。
/// 而"源是不是竖素材"只要宽高，用不着数帧 —— 这里就不带那个开关。
///
/// 探不到（ffprobe 不在 PATH / 文件读不了）返回 `null`，**由调用方决定怎么办**，
/// 与 `probeImage` 同一条纪律：不给默认值假装成功。
function probeSize(path) {
  try {
    const out = execFileSync(
      'ffprobe',
      ['-v', 'error', '-select_streams', 'v:0',
       '-show_entries', 'stream=width,height', '-of', 'default=nw=1', path],
      { encoding: 'utf8', timeout: 20000, stdio: ['ignore', 'pipe', 'ignore'] },
    );
    const fields = new Map();
    for (const line of out.split(/\r?\n/)) {
      const at = line.indexOf('=');
      if (at > 0) fields.set(line.slice(0, at).trim(), line.slice(at + 1).trim());
    }
    const width = parseInt(fields.get('width') ?? '', 10);
    const height = parseInt(fields.get('height') ?? '', 10);
    if (!(width > 0 && height > 0)) return null;
    return { width, height };
  } catch {
    return null;
  }
}

/// 探一段**视频**的真实时基与尺寸：`{ fps: {num,den}, width, height }`。
///
/// ⚠️ **为什么必须是真探测**（2026-10-05 修）：这里原来写死 `{num:60, den:1}` +
/// `width:1920, height:1080`（注释"素材实测 60fps"），而**时基决定帧号映射** ——
/// 一份 30fps 的素材会被声明成 60fps，帧号整体错 2×（画面放到一半就播完 / 定位全错）；
/// 分辨率同理：预览改用 720p 代理后若仍声明 1920×1080，层的 `scale`/`FIT` 会按错的源
/// 尺寸算 ⇒ 缩放/取景全错。
///
/// `r_frame_rate` 是"最大帧率"（VFR 素材会骗人），所以**优先 `avg_frame_rate`** ——
/// 它才反映"平均每帧多久"。两者都拿不到就返回 null，由调用方决定（当前回落 60fps）。
///
/// 探不到（ffprobe 不在 PATH / 文件读不了）返回 `null` —— 与 `probeSize`/`probeImage`
/// 同一条纪律：**不给默认值假装成功**。
function probeMedia(path) {
  try {
    const out = execFileSync(
      'ffprobe',
      ['-v', 'error', '-select_streams', 'v:0',
       '-show_entries', 'stream=width,height,r_frame_rate,avg_frame_rate:format=duration',
       '-of', 'default=nw=1', path],
      { encoding: 'utf8', timeout: 20000, stdio: ['ignore', 'pipe', 'ignore'] },
    );
    const fields = new Map();
    for (const line of out.split(/\r?\n/)) {
      const at = line.indexOf('=');
      if (at > 0) fields.set(line.slice(0, at).trim(), line.slice(at + 1).trim());
    }
    const width = parseInt(fields.get('width') ?? '', 10);
    const height = parseInt(fields.get('height') ?? '', 10);
    // 帧率是**分数**（`30000/1001` 这种必须原样保留：化成小数再乘回去会漂）
    const parseRate = (raw) => {
      const m = /^(\d+)\/(\d+)$/.exec(String(raw ?? '').trim());
      if (!m) return null;
      const num = parseInt(m[1], 10), den = parseInt(m[2], 10);
      if (!(num > 0 && den > 0)) return null;
      const gcd = (a, b) => (b ? gcd(b, a % b) : a);
      const k = gcd(num, den);
      return { num: num / k, den: den / k };
    };
    // **标准帧率吸附**：`avg_frame_rate` 是实测值，长片会因为容器时长取整而给出
    // `671400/11191`（≈59.9946）这种**不可用**的时间基 —— 拿它当 timebase，帧号会与
    // "素材是 60fps"这个事实整体漂移，而且每帧号都是大整数。
    // 所以：**先看实测值能否吸附到标准帧率**（含 NTSC 的 29.97/59.94），能就吸附；
    // 不能再用容器声明的 `r_frame_rate`（标称值）；都没有才退回实测原始分数。
    const STANDARD = [[24000, 1001], [24, 1], [25, 1], [30000, 1001], [30, 1], [50, 1], [60000, 1001], [60, 1]];
    const nominal = parseRate(fields.get('r_frame_rate'));
    const average = parseRate(fields.get('avg_frame_rate'));
    const snapped = (() => {
      if (!average) return null;
      const f = average.num / average.den;
      // **取最近的那一个**，不是"第一个落在容差里的"：59.9946 到 60/1 差 0.009%、
      // 到 60000/1001 差 0.091% —— 按顺序命中会把 60fps 素材吸成 NTSC 的 59.94
      // （实测踩过：timebase 变成 60000/1001，帧号整体漂）。
      let best = null;
      for (const [n, d] of STANDARD) {
        const err = Math.abs(f - n / d) / (n / d);
        if (err < 0.002 && (!best || err < best.err)) best = { num: n, den: d, err };
      }
      return best ? { num: best.num, den: best.den } : null;
    })();
    const fps = snapped ?? nominal ?? average;
    if (!(width > 0 && height > 0) && fps === null) return null;
    // ⚠️ **`duration` 是加出来的**（2026-10-09，给片头/片尾的「播到结尾」用）：
    // 老消费点只读 `fps`/`width`/`height`，多一个键对它们**完全无影响**。
    // 探不到就当 `null`（调用方据此报 DROP），**不给默认值假装成功**。
    const duration = parseFloat(fields.get('duration') ?? '');
    return {
      fps,
      width: width > 0 ? width : null,
      height: height > 0 ? height : null,
      duration: Number.isFinite(duration) && duration > 0 ? duration : null,
    };
  } catch {
    return null;
  }
}

/// 探一张动图的**逐帧时长**（毫秒）。
///
/// 用 `frame=duration_time`（**不是** `pkt_duration_time` —— 那个对 GIF 是空的，
/// 实测 0 个值；`duration_time` 给 43 个 `0.030000`）。
///
/// 探不到（或帧数对不上）返回 `null`，**由调用方决定怎么办** —— 与 `probeImage` 同一条纪律。
///
/// # 为什么要它
///
/// 底座的循环素材帧号是按**素材单一时间基**匀速换算的。GIF 允许逐帧不同的 delay，
/// 那种动图在匀速换算下**误差随时间线性长出来**。底座已经加了 `frame_delays_ms`
/// 这个字段（见 `docs/yekitrim-integration.md` 第六节第 6 条），这里就是喂它的地方。
function probeFrameDelays(path) {
  try {
    const out = execFileSync(
      'ffprobe',
      ['-v', 'error', '-select_streams', 'v:0', '-show_frames',
       '-show_entries', 'frame=duration_time', '-of', 'default=nw=1', path],
      { encoding: 'utf8', timeout: 30000, stdio: ['ignore', 'pipe', 'ignore'] },
    );
    const delays = [];
    for (const line of out.split(/\r?\n/)) {
      const at = line.indexOf('=');
      if (at <= 0 || !line.startsWith('duration_time')) continue;
      const seconds = Number.parseFloat(line.slice(at + 1));
      if (!Number.isFinite(seconds) || seconds < 0) return null;
      // 毫秒取整。0 会被契约判为"总时长 0"而报错，所以下限给 1ms。
      delays.push(Math.max(1, Math.round(seconds * 1000)));
    }
    return delays.length > 0 ? delays : null;
  } catch {
    return null;
  }
}

/// 探不到就返回 null，让调用方决定怎么办。
function probeImage(path) {
  const result = { frames: null, width: null, height: null, fps: null };
  try {
    // **按字段名解析，不按位置。**
    //
    // `-of csv=p=0` 的字段顺序是 **ffprobe 决定的**，而且**加一个字段就会重排**：
    // 请求 `nb_read_frames,width,height` 时吐的是 `width,height,nb_read_frames`（3 个），
    // 再加一个 `r_frame_rate` 就变成 `width,height,r_frame_rate,nb_read_frames`。
    // 我按位置解过两次、错两次（第一次把宽当帧数，第二次把帧率当帧数），
    // 而两次的结果**看起来都还挺合理**。`default=nw=1` 给的是 `键=值`，不会错。
    const out = execFileSync(
      'ffprobe',
      ['-v', 'error', '-count_frames', '-select_streams', 'v:0',
       '-show_entries', 'stream=nb_read_frames,width,height,r_frame_rate',
       '-of', 'default=nw=1', path],
      { encoding: 'utf8', timeout: 20000, stdio: ['ignore', 'pipe', 'ignore'] },
    );
    const fields = new Map();
    for (const line of out.split(/\r?\n/)) {
      const at = line.indexOf('=');
      if (at > 0) fields.set(line.slice(0, at).trim(), line.slice(at + 1).trim());
    }
    const int = (key) => {
      const value = parseInt(fields.get(key) ?? '', 10);
      return Number.isFinite(value) && value > 0 ? value : null;
    };
    result.frames = int('nb_read_frames');
    result.width = int('width');
    result.height = int('height');
    const rate = fields.get('r_frame_rate') ?? '';
    const rateMatch = rate.match(/^(\d+)\/(\d+)$/);
    if (rateMatch) {
      const num = Number(rateMatch[1]);
      const den = Number(rateMatch[2]);
      if (num > 0 && den > 0) result.fps = { num, den };
    }
  } catch {
    // 探不到就都是 null。
  }
  return result;
}

/// 贴纸包的 tag 表（`ysyk_0.9.0`，34 个 tag）。
///
/// **这份切片目录里没有 `ysyk.toml`** —— 只有 210 个 GIF。
/// YekiTrim 的 `release/` 里也没带（它只带 `ysyk_0.9.0/` 目录）。
/// 而**别的切片目录里有**（`E:\media\...\clip-12\assets\stickers\ysyk.toml`），
/// 实测那份引用 163 个文件，本切片目录里 161 个都在 —— 是同一套贴纸包。
///
/// 所以查找顺序是：
///   1. 本切片目录下的 `ysyk.toml`
///   2. `--sticker-package` 显式指定的路径
///   3. **同卷其它切片目录里的同名 toml**（就近找一个，实测能对上）
///
/// 都找不到就退回"按文件名直接找"（`pickSticker` 会返回 null 并报 DROP）——
/// **不会瞎猜**。
function loadStickerPackage(clipDir, explicitPath) {
  const candidates = [];
  if (explicitPath) candidates.push(explicitPath);
  candidates.push(
    join(clipDir, 'assets', 'stickers', 'ysyk.toml'),
    join(clipDir, 'assets', 'stickers', 'stickers', 'ysyk.toml'),
  );
  for (const path of candidates) {
    const parsed = tryReadPackage(path);
    if (parsed) return { ...parsed, source: path };
  }
  // 兜底：在"兄弟切片目录"里找一份。只在同卷上找，且只认 `ysyk.toml`。
  const borrowed = findSiblingPackage(clipDir);
  if (borrowed) return borrowed;
  return null;
}

function tryReadPackage(path) {
  if (!existsSync(path)) return null;
  try {
    const text = readFileSync(path, 'utf8');
    const tags = new Map();
    let current = null;
    for (const line of text.split(/\r?\n/)) {
      const trimmed = line.trim();
      const tagLine = trimmed.match(/^tag\s*=\s*"(.+)"$/);
      if (tagLine) {
        current = tagLine[1];
        tags.set(current, []);
        continue;
      }
      const fileLine = trimmed.match(/^"(.+)",?$/);
      if (fileLine && current !== null) tags.get(current).push(fileLine[1]);
    }
    if (tags.size === 0) return null;
    return { tags, dir: dirname(path) };
  } catch {
    return null;
  }
}

/// 在**别的切片目录**里借一份 `ysyk.toml`。
///
/// **借之前要核对它能对上本地的文件** —— 借错包比不借更糟。
/// 判据：引用的文件里至少一半在本地存在，就用它。
///
/// 搜索范围要**逐级往上放大**：同一次录制的兄弟切片 → 同一 `output/` 下
/// 所有录制 → 整个 `E:\media`。实测本切片所在的录制**七个切片全都没有 toml**，
/// 而另一次录制（`2026-09-02_20-53-08-367_最鱿鱼之人`）里有，
/// 且那份引用 163 个文件、本切片目录里 161 个都在 —— 是同一套贴纸包。
/// 只找同级的兄弟会导致**一个都借不到**（第一版就是这个错）。
function findSiblingPackage(clipDir) {
  const localDir = join(clipDir, 'assets', 'stickers', 'ysyk_0.9.0');
  let localFiles = null;
  try {
    localFiles = new Set(readdirSync(localDir));
  } catch {
    return null;
  }

  // 逐级放大搜索根。每一级都扫它底下所有 `assets/stickers/ysyk.toml`。
  const roots = [];
  // clips/ -> output/ -> 录制目录/ -> E:\media
  let cursor = clipDir;
  for (let i = 0; i < 4; i += 1) {
    cursor = dirname(cursor);
    roots.push(cursor);
  }

  for (const root of roots) {
    const found = scanForPackage(root, clipDir, localFiles, 3);
    if (found) return found;
  }
  return null;
}

/// 在 `root` 下找 `assets/stickers/ysyk.toml`，深度受限（别扫整个盘）。
function scanForPackage(root, self, localFiles, depth) {
  if (depth < 0 || !existsSync(root)) return null;
  let entries;
  try {
    entries = readdirSync(root, { withFileTypes: true });
  } catch {
    return null;
  }
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const dir = join(root, entry.name);
    if (dir === self) continue;
    const candidate = join(dir, 'assets', 'stickers', 'ysyk.toml');
    const parsed = tryReadPackage(candidate);
    if (parsed && packageMatchesLocal(parsed, localFiles)) {
      // 借来的包，`dir` 用**本地**的贴纸目录 —— 文件在本切片里。
      return { ...parsed, dir: join(self, 'assets', 'stickers', 'ysyk_0.9.0'), source: candidate, borrowed: true };
    }
    const deeper = scanForPackage(dir, self, localFiles, depth - 1);
    if (deeper) return deeper;
  }
  return null;
}

/// 借来的包对不对得上本地文件：引用的文件至少一半在本地存在。
function packageMatchesLocal(parsed, localFiles) {
  const referenced = new Set();
  for (const files of parsed.tags.values()) for (const f of files) referenced.add(f);
  if (referenced.size === 0) return false;
  let hit = 0;
  for (const f of referenced) if (localFiles.has(f)) hit += 1;
  return hit * 2 >= referenced.size;
}

/// 「意图词 → 贴纸包 tag」的同义映射。
///
/// 这份表是**看着数据写的**，不是猜的：左边是这份 `polish.toml` 里
/// 真正出现过的词（加几个明显会出现的），右边是贴纸包里真实存在的 tag。
const STICKER_SYNONYMS = {
  搞笑: '笑',
  爆笑: '笑',
  笑: '笑',
  好笑: '笑',
  震惊: '惊讶',
  惊讶: '惊讶',
  吃惊: '惊讶',
  疑问: '问号',
  问号: '问号',
  困惑: '问号',
  害怕: '害怕',
  恐惧: '害怕',
  委屈: '哭',
  哭: '哭',
  难过: '哭',
  开心: '开心',
  高兴: '开心',
  看热闹: '吃',
  围观: '吃',
  吃瓜: '吃',
};

/// 给一个意图词，挑出贴纸包里可用的文件。
///
/// 三级：同义表 → tag 里包含关系 → 包里的 `笑`。**全都失败就返回空**，
/// 由调用方报 DROP —— 猜一个不相干的贴纸比不贴更糟。
function pickSticker(pkg, tag) {
  if (!pkg) return null;
  const mapped = STICKER_SYNONYMS[tag];
  if (mapped && pkg.tags.has(mapped) && pkg.tags.get(mapped).length > 0) {
    return { tag: mapped, files: pkg.tags.get(mapped), how: `同义映射 ${tag}→${mapped}` };
  }
  if (pkg.tags.has(tag) && pkg.tags.get(tag).length > 0) {
    return { tag, files: pkg.tags.get(tag), how: 'tag 同名' };
  }
  for (const [name, files] of pkg.tags) {
    if (files.length > 0 && (name.includes(tag) || tag.includes(name))) {
      return { tag: name, files, how: `近似匹配 ${tag}→${name}` };
    }
  }
  return null;
}

/// 运镜的 `zoom` 名 -> 倍率。**逐字照抄参照的 `camFactor`**（`templates/index.html:578-585`）：
///
/// ```js
/// if(label==='特写'){v=c.zoom_closeup;d=1.8;}
/// else if(label==='近'){v=c.zoom_close;d=1.5;}
/// else if(label==='远'){v=c.zoom_wide;d=0.55;}
/// else{v=c.zoom_mid;d=1.3;}
/// return (typeof v==='number'&&isFinite(v))?v:d;
/// ```
///
/// ⚠️ 两处曾经不一致（2026-09-30 修，M2）：
///   1. **配置里没有这一档时，兜底不是 1.0**，而是**按档位**取默认（远 = **0.55**）。
///      旧实现统一退回 1.0 ⇒ 一个"远景（画中画拉远）"的事件在成片里**根本不拉远**，
///      而报告里只有一行 CLAMP（两个样本都命中过）。参照的默认值就是它的行为，照抄。
///   2. 认不出的档位：模板走 `else` = **中景**（不是 1.0，也不是"事件自己的值"）。
///   3. `isFinite` 也要一起看 —— `NaN` 过 `typeof === 'number'` 但模板会退回默认值。
/// `overlay` 线性渐变的方向：**CSS 角度 → 底座角度**（D5）。
///
/// 两边约定**不同**，这是本函数的全部理由：
///   - **YekiTrim / CSS**：度，`0` = **向上**、顺时针增大（`linear-gradient(0deg, …)` 是从下往上）。
///     不写 = 参照的缺省方向 `linear-gradient(to bottom right, …)` = **135°**。
///   - **底座**：度，在 shader 里当方向向量用 —— `dir = vec2(cos φ, sin φ)`（屏幕 y 向下），
///     所以 `0` = 向右、`90` = 向下。
/// 令两者方向向量相等：`(cos φ, sin φ) = (sin θ, −cos θ)` ⇒ **φ = θ − 90**（再归一到 [0, 360)）。
/// 校验：CSS `to bottom right`(135°) ⇒ φ = 45 ⇒ 方向 (0.707, 0.707) = 右下 ✓。
///
/// ⚠️ 以前这里**写死 0**（= 正右方），与参照的"右下"不一致 —— 一个被"底座不出色罩"（底座侧 D1）
/// 掩盖了的偏差；D5 一并修掉。`undefined`/非数/`NaN` 都按缺省 135° 处理。
export function overlayBaseAngle(cssAngle) {
  const css = typeof cssAngle === 'number' && Number.isFinite(cssAngle) ? cssAngle : 135;
  return ((css - 90) % 360 + 360) % 360;
}

export function zoomFactor(name, camera, where) {
  // ⚠️ 这张表与 Rust `CameraStyle`、模板 `camFactor`、`engine.js::camFactor`
  //    **四处逐值一致**（门禁 `check-camera-zoom-parity.mjs` 钉住）。
  const defaults = { 特写: 1.8, 近: 1.5, 远: 0.55, 中: 1.3 };
  const keys = { 特写: 'zoom_closeup', 近: 'zoom_close', 远: 'zoom_wide', 中: 'zoom_mid' };
  // 认不出的档位按**中景**（与模板的 `else` 分支同口径）。
  const key = keys[name] ?? 'zoom_mid';
  const fallback = defaults[name] ?? 1.3;
  const value = camera?.[key];
  if (typeof value === 'number' && Number.isFinite(value)) {
    return value;
  }
  if (where) {
    note(
      where,
      Number.isFinite(value)
        ? `zoom="${name}" 按模板默认倍率 ${fallback}（[style.camera].${key} 没配）`
        : `zoom="${name}" 的值不是有限数（${value}），按模板默认倍率 ${fallback}`,
    );
  }
  return fallback;
}

// ---------------------------------------------------------------------------
// 纯函数：YekiTrim 的三条曲线（**从参照逐行抄的**）
// ---------------------------------------------------------------------------
//
// 这些提到模块顶层并导出，是为了**能被测试直接调用**。
// 它们都只依赖参数、不依赖某一份工程配置。

/// `back_out(t, s)` —— YekiTrim 的 `E.back_out(p, 3)`。
///
///     function(t, s) { s = s || 1.70158; return --t * t * ((s + 1) * t + s) + 1; }
///
/// **贴纸弹入用的是 `s = 3`**，比底座 `Easing::BackOut` 的经典 1.70158 猛得多
/// （峰值约 1.25 vs 1.10）。契约的 `Easing` 是不带参数的枚举、装不下 `s`，
/// 所以转译器**按帧采样**这条曲线 —— 与手持摇摆同一个办法。
export const backOut3 = (t) => {
  const s = 3;
  const u = t - 1;
  return u * u * ((s + 1) * u + s) + 1;
};

export const pow2Out = (p) => 1 - (1 - p) * (1 - p);

/// **运镜速度档位表**（规格 `camera-and-noise-spec.md` §1.2）：`速度 → 过渡秒数`。
///
/// ⚠️ **`med` 不在表里** —— 它的算法是**老式子** `min(dur × 0.5, 0.6)`，
/// 与"没写 `speed`"走**同一条**分支。把它写成一个固定秒数（比如 0.6）就**破坏了 R4**：
/// 存量工程里那些短段的过渡会从 `dur × 0.5` 变成 0.6，**每一段都变**。
export const CAMERA_SPEED_SECS = {
  snap: 0.10,
  fast: 0.25,
  slow: 1.5,
};

/// 运镜过渡应该占多少**帧**（规格 §1）。
///
/// - `hardCut` ⇒ 0（硬切不用缓动，`cut` 与速度互斥、以 `cut` 为准）；
/// - `speed` 认不出 / 没写 / `med` ⇒ **老式子** `max(1, round(min(dur × 0.5, 0.6) × fps))`；
/// - `snap` / `fast` ⇒ 固定秒数；
/// - `slow` ⇒ `min(1.5, dur × 0.8)` —— ⚠️ **必须夹**：不夹的话"过渡比镜头本身还长"，
///   会**推不完就切走**（规格 §1.2 那条硬约束）。
///
/// ⚠️ 结果**至少 1 帧**（除硬切）：0 帧会让这一段退化成硬切，
/// 而用户写 `slow` 想要的恰恰是"看得见的缓动"。
///
/// 抽成导出函数是为了**能逐值测**（守卫的 `checkCameraSpeed`）——
/// 它埋在 `translate` 的大循环里，行为判据只能靠"造工程出产物"间接看，太钝。
export function cameraTransitionFrames({ speed, hardCut, durationSec, fps }) {
  if (hardCut) return 0;
  const norm = typeof speed === 'string' ? speed.trim().toLowerCase() : '';
  let secs;
  if (Object.prototype.hasOwnProperty.call(CAMERA_SPEED_SECS, norm)) {
    secs = CAMERA_SPEED_SECS[norm];
    // `slow` 夹到整段的 0.8（`snap`/`fast` 本来就比任何段短，夹了也不变）
    if (norm === 'slow') secs = Math.min(secs, durationSec * 0.8);
  } else {
    // 没写 / `med` / 认不出的值 —— **老式子，一个字节不改**
    secs = Math.min(durationSec * 0.5, 0.6);
  }
  return Math.max(1, Math.round(secs * fps));
}

/// 四段相位的**标称**段长（窗口装得下时用的值，见 `stickerStateAt`）。
export const STICKER_POP_IN_SEC = 0.35;
export const STICKER_FLOAT_UP_SEC = 0.4;
export const STICKER_FADE_OUT_SEC = 0.3;

/// 贴纸可见窗口**超出事件时长**的秒数：`0` ⇒ 窗口上界就是事件的 `dur`，**不溢出**。
///
/// 上一刀这里是 `0.35 + 0.4 + 0.3`，即窗口 = `dur + 1.05` —— 贴纸比事件多显示 0.4 秒 ✗
/// （用户实测：出点晚了）。现在的口径是「淡出在窗口**内部**消化、窗口以事件时长为准」。
export const STICKER_WINDOW_SEC = 0;

/// 贴纸图层要**逐帧采样**的帧数：窗口 `[0, durSec + STICKER_WINDOW_SEC]`，**含上界那一帧**
/// （上界那一帧正好是淡出终点，opacity = 0）。
export const stickerWindowFrames = (durSec, fps) =>
  Math.max(1, Math.round((durSec + STICKER_WINDOW_SEC) * fps));

/// 本仓出片模板的贴纸逐帧状态（`crates/yekitrim-polish/templates/index.html:1517 getActiveSticker`）。
///
/// # 窗口 = **事件时长本身**（口径见 `docs/yekitrim-integration.md` §14：出点 = 最后一个显示帧）
///
/// ```js
/// if (t < et || t > et + dur) continue;   // 窗口不溢出到事件之外 ✓
/// popIn 0.35 → floatUp 0.4 → hold → fadeOut 0.3   // 淡出**贴在窗口末尾**
/// ```
///
/// 四段相位在窗口**内部**消化：
///
///     0        → 0.35          弹入（back_out(3)）
///     0.35     → 0.75          上浮（线性）
///     0.75     → dur - 0.3     保持
///     dur - 0.3 → dur          渐隐出场（线性到 0）⇒ **最后一帧正好落在出点** ✓
///
/// 所以淡出起点是 `dur - fadeOut`，**不再是** `popIn + floatUp + dur` —— 后者是上一刀的形状：
/// 窗口被放宽到 `dur + 1.05`（四段之和），贴纸比事件多显示 0.4 秒 ✗（用户实测）。
///
/// **边界情形**（`dur` 装不下 `popIn + floatUp + fadeOut`，即 `dur < 1.05`）：**优先保淡出** ——
/// 前两段按可用时长等比压缩（淡出起点不得早于 0），`dur <= 0.3` 时退化为"全程淡出"。
/// 任何时长下都不会出现负段长 / NaN / 最后一帧不透明。
///
/// 四处同形（窗口上界 = `dur`，淡出起点 = `dur - fadeOut`）：
///   * 本文件的 `STICKER_WINDOW_SEC = 0` + `stickerWindowFrames`；
///   * `crates/yekitrim-polish/templates/index.html:1527`（`getActiveSticker` 的守卫）；
///   * `crates/yekitrim-render-core/src/timeline.rs:530` 与
///     `webui/src/polish/engine.js:705`（两处同形判据 + 同形相位）。
export const stickerStateAt = (localSec, durSec, sk) => {
  // 退化时长（<= 0）没有可见窗口 —— 直接返回不可见，**不留除零**。
  if (!(durSec > 0)) return { opacity: 0, scale: 1, rot: 0, yOff: 0 };
  // 淡出**贴在窗口末尾**：起点 = `dur - fadeOut`；窗口比 0.3 还短时淡出就占满整个窗口。
  const fadeOut = Math.min(STICKER_FADE_OUT_SEC, durSec);
  // 前两段可用的总时长；装不下标称的 0.35 + 0.4 就等比压缩（**优先保淡出**）。
  const k = Math.min(1, Math.max(0, durSec - fadeOut) / (STICKER_POP_IN_SEC + STICKER_FLOAT_UP_SEC));
  const popIn = STICKER_POP_IN_SEC * k;
  const floatUp = STICKER_FLOAT_UP_SEC * k;
  const fadeStart = durSec - fadeOut;
  let opacity = 0;
  let scale = 1;
  let rot = 0;
  let yOff = 0;
  if (localSec < popIn) {
    const p = localSec / popIn;
    const bp = backOut3(p);
    scale = 0.3 + 0.7 * bp;
    rot = -15 + 15 * bp;
    opacity = pow2Out(p);
  } else if (localSec < popIn + floatUp) {
    // 上浮是**线性**的（`yOff = -18*SK*p`），不是缓动。
    const p = (localSec - popIn) / floatUp;
    opacity = 1;
    yOff = -18 * sk * p;
  } else if (localSec < fadeStart) {
    // 保持段：右端从 `popIn + floatUp + durSec` 缩短到 `dur - fadeOut`。
    opacity = 1;
    yOff = -18 * sk;
  } else {
    // 第四段（淡出）。**这一支永远够得到** —— 它就贴在窗口末尾，见上面的说明。
    const p = (localSec - fadeStart) / fadeOut;
    // `Math.max(0, …)` 只吃掉上界那一帧的浮点尾数（`p` 可能比 1 大 1 ulp）：
    // 出点那一帧必须是**恰好 0**，不是 `-2e-16`。
    opacity = Math.max(0, 1 - p);
    scale = 1 - 0.6 * p;
    yOff = -18 * sk;
  }
  return { opacity, scale, rot, yOff };
};

/// 本仓出片模板的手持波形（`crates/yekitrim-polish/templates/index.html:593 handheldSway`）。
///
///     单位：dx/dy 是 **1920x1080 输出像素**，rot 是**度**，scale 是无量纲乘数
///
/// `amp` 是状态机上的 0~1 幅度（见 `cameraStateAt`），不是布尔。
export const handheldSway = (t, amp) => {
  const a = amp > 0 ? amp : 0;
  if (a === 0) return { dx: 0, dy: 0, rot: 0, scale: 1 };
  const TAU = Math.PI * 2;
  return {
    dx: (6.5 * Math.sin(TAU * 0.47 * t) + 3.0 * Math.sin(TAU * 0.83 * t + 1.7)) * a,
    dy: (5.0 * Math.sin(TAU * 0.59 * t + 0.9) + 2.4 * Math.sin(TAU * 1.13 * t + 2.4)) * a,
    rot: (0.32 * Math.sin(TAU * 0.37 * t + 0.4) + 0.16 * Math.sin(TAU * 0.91 * t + 2.9)) * a,
    scale: 1.0 + 0.007 * Math.sin(TAU * 0.53 * t + 1.1) * a,
  };
};

/// 本仓出片模板那份 `shake`（`crates/yekitrim-polish/templates/index.html:1575 getShake`）：**固定 0.2s 的四级阶梯**。
///
/// 与事件自己的 `duration` **无关**，也与预览那份 GSAP `shake()` 无关。
export const shakeOffsetAt = (localSec, off) => {
  if (localSec < 0.04) return { x: off, y: -off };
  if (localSec < 0.08) return { x: -off, y: off };
  if (localSec < 0.12) return { x: off / 2, y: -off / 2 };
  if (localSec < 0.16) return { x: -off / 2, y: off / 2 };
  return { x: 0, y: 0 };
};

/// 出片那份 `getShake` 的窗口长度是**写死的 0.2 秒**。
export const SHAKE_SECONDS = 0.2;

// **导出给守卫**（`scripts/check-yekitrim-translator.mjs` 的 `checkMappings`）：它拿一份**合成工程**
// 过一遍 `translate`，断言"某一档真的转出来了"。只有 `isEntryPoint` 那道门保证直接跑仍然是 CLI。
/**
 * 每个密集窗口的采样区间：`[上一个稀疏键 + 1, 下一个稀疏键 - 1]`。
 *
 * **没有"上一个稀疏键"时就别往前铺。**
 *
 * `prevSparse` 返回 -1 有两种情形：这一个通道上**一个稀疏键都没有**
 * （竖素材整条关掉运镜就是这种），或者稀疏键都在窗口**之后**。
 * 两种情形下窗口**之前**都没有段结构可保护 —— 往前铺到第 0 帧只是白写。
 * 实测：竖素材那份工程因此写了 **16716** 个关键帧（每帧一个），
 * 而它们全是常量 `x=0,y=0,scale=1`；只从窗口起点开始写就够。
 *
 * ⚠️ **右端不许封在窗口末尾**（2026-10-05 ③R3 修）：封在 `b + 1` 时，窗口之后到**下一个稀疏键**
 * 那一段里只有两个稀疏键 ⇒ 曲线退化成**两点直线弦**，`pow2_out` 被拉直 —— 实测过渡后段
 * zoom 偏 ~0.04（≈3%、画面边缘 ~29px）。触发条件正是用户报的"shake 与运镜同时存在"
 * （shake 窗口压在运镜过渡上）。所以右端铺到**下一个稀疏键的前一帧**；
 * 没有下一个稀疏键就铺到最后一个输出帧。左端保持不变（上面那条"16716 键"的教训仍在）。
 *
 * 判据：`node scripts/check-yekitrim-translator.mjs` 的 `checkDenseRanges`
 * （逐值钉右端落点 + 两个边界），并且在 `--mutate` 里也调 —— 变异条目
 * 「密集键右端退回窗口末尾」必须把它打红。
 */
export function denseRangesFor(spans, sparseSorted, lastOut) {
  /// 严格小于 `out` 的最后一个稀疏键（没有就给 -1）。
  const prevSparse = (out) => {
    let lo = 0;
    let hi = sparseSorted.length - 1;
    let best = -1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (sparseSorted[mid] < out) {
        best = sparseSorted[mid];
        lo = mid + 1;
      } else {
        hi = mid - 1;
      }
    }
    return best;
  };
  /// 严格大于 `out` 的第一个稀疏键（没有就给 `Infinity`）。
  const nextSparse = (out) => {
    let lo = 0;
    let hi = sparseSorted.length - 1;
    let best = Infinity;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (sparseSorted[mid] > out) {
        best = sparseSorted[mid];
        hi = mid - 1;
      } else {
        lo = mid + 1;
      }
    }
    return best;
  };
  return spans.map(([a, b]) => {
    const prev = prevSparse(a);
    const next = nextSparse(b + 1);
    const right = Number.isFinite(next) ? next - 1 : lastOut;
    // `max` 只为兜住"下一个稀疏键恰好是 `b + 1`"（那时 `next - 1 = b`，不许把区间收短）
    return [prev === -1 ? a : prev + 1, Math.max(b + 1, right)];
  });
}

export function translate(config, clipDir, outPath, options = {}) {
  /// **出片那条腿传 `true`**：时间线保持整条（屏蔽区间留给出片尾部的整段重编码去剪，
  /// 与成片那条口径一致）。预览必须留 `false`（客户端要靠压缩时间线跳屏蔽区间）——
  /// 详见下面 `keepRanges` 那段注释。
  const keepMuted = options.keepMuted === true;
  // **实际会被用到的那份媒体**：预览那条腿可以传 `--video _proxy_720.mp4`，于是时基与
  // 分辨率都跟着**代理**走（层的 scale/FIT 也按它算），而不是源。渲染那条腿不传 ⇒ 源。
  const videoRel = options.video ?? config.source?.video ?? 'clip.mp4';
  const videoAbs = join(clipDir, videoRel);
  const media = existsSync(videoAbs) ? probeMedia(videoAbs) : null;
  // 探不到就维持改造前的值（60fps / 1920x1080）并**明确报一条 NOTE** —— 不静默假装探到了。
  //
  // ⚠️ **素材自己那份时基（`assetTimebase`，**不设下限**）与工程网格（`timebase`，**下限 30fps**）是两件事**：
  //   · 源帧号靠**素材时基**换算（`asset_timebases` → `source_frame = seconds × asset.fps`）；
  //     把下限也套到素材上，5fps 的片子会被当成 30fps ⇒ 源帧号跑 6 倍 ⇒ 画面在 1/6 时长内播完。
  //   · 工程网格是**字幕/弹幕/贴纸/关键帧的更新频率**。电台直播那种"视频流 5fps 静止帧、主要是音频"的素材，
  //     网格也降到 5fps 会让所有叠加层一秒只动 5 次（很糙）；给一个 **30fps 下限**，视频层照旧是静止帧、
  //     叠加层按 30fps 走 —— 这正是用户要的（2026-10-06）。
  const assetTimebase = media?.fps ?? { num: 60, den: 1 };
  const mediaFps = assetTimebase.num / assetTimebase.den;
  const FPS_FLOOR = 30;
  const timebase = mediaFps > 0 && mediaFps < FPS_FLOOR ? { num: FPS_FLOOR, den: 1 } : assetTimebase;
  const fps = timebase.num / timebase.den;
  if (media?.fps && mediaFps < FPS_FLOOR) {
    note(
      'assets',
      `源素材只有 ${mediaFps}fps（电台那种"5fps 静止帧 + 音频"的流）⇒ **工程网格抬到 ${FPS_FLOOR}fps**：` +
        '叠加层（字幕/弹幕/贴纸）按 30fps 更新，视频层仍是那几张静止帧（源帧号按素材自己的 ' +
        `${mediaFps}fps 换算，不受影响）`,
    );
  }
  const mediaW = media?.width ?? null;
  const mediaH = media?.height ?? null;
  if (media?.fps == null) {
    note(
      'assets',
      '探不到源素材时基（ffprobe 不可用 / 文件读不了），按 60fps 处理 —— ' +
        '素材若不是 60fps，帧号会整体错位（见 probeMedia 的说明）',
    );
  }
  const frameAt = (seconds) => Math.round(seconds * fps);

  const assets = [];
  const videoAssetId = 'clip.mp4';
  assets.push({
    id: videoAssetId,
    kind: 'video',
    name: config.project?.name ?? 'clip',
    uri: videoRel,
    frame_count: null,
    // ⚠️ **素材自己的时基，不设下限**（源帧号靠它换算；套上限会让 5fps 素材的源帧号跑 6 倍）
    timebase: assetTimebase,
    width: mediaW ?? 1920,
    height: mediaH ?? 1080,
    content_hash: null,
    tags: {},
    note: '来源：polish.toml [source].video',
  });
  note('assets', 'frame_count 留空：转译器不去探测媒体（要让 probe 子命令回填）');

  // ---- 视频素材（第 6 批 / 规格 §11.2）----
  //
  // EDL 里 `source_kind = "asset"` 的段指向**另一个 mp4**，底座把它当**第二个 `asset`**
  // （探针已验：`assets[]` 是数组、`opened_streams: 2`、两段各自的像素都真出现了）。
  //
  // ⚠️⚠️ **每个素材必须按它自己的 `timebase` 换算 `source_in`** —— 这是本批**最容易埋雷**
  // 的一条（规格 §11.2 第 3 条）：
  //
  //   `source_in` 的单位是**该素材自己的帧**，而 `layer.start/end` 是**序列帧**。底座
  //   （`EvalContext::source_frame_looped`）拿「序列帧 → 秒（用时间线时基）→ 素材帧（用该
  //   asset 自己的 `timebase`）」再 `+ source_in`。所以**素材帧率 ≠ 时间线帧率**时，
  //   写死的那个 `assetTimebase` 会算出**别的素材的帧号** ——
  //
  //   症状（那条注释记的实测）：**画面跳到源里 6 倍远的位置**，而且**不报错**。
  //   （5fps 素材 + 30fps 网格 ⇒ 本来该第 20 帧、写成序列帧就是 120。）
  //
  //   ⇒ 这里给**每个 asset 一份自己的 `assetTimebase`**，`mainLayers` 按 `range.assetId`
  //     取那一份来换算（见下面的 `sourceFramesFor`）。
  //
  // ⚠️ **素材必须已经在项目里**（`assets/videos/<文件名>`）：出片与预览只读项目目录，
  //   引用库里的绝对路径 = 换机器/库改名就废（`assets/AGENTS.md`）。所以这里查的是
  //   **项目目录**，缺了当场 DROP（而不是"猜一个路径"或"静默用回本条片段"）。
  //
  // ⚠️ **时基靠 `probeMedia` 真探**，不重用源那一份 —— 那会造出**第二个 fps 真源**，
  //   而第 2 批刚为"硬编码 30fps"打过一架（`render/pipeline.rs`）。探不到就**明确报**、
  //   并按时间线时基兜底（那会让该素材的帧号错，所以必须让人看见）。
  const videoAssets = new Map(); // asset_id（素材名）→ { id, timebase, width, height } | null
  /// 登记一个视频素材（返回它的 asset_id；登记不上返回 `null` 并报 DROP）。
  const registerVideoAsset = (assetName) => {
    if (!assetName) return null;
    if (videoAssets.has(assetName)) return videoAssets.get(assetName)?.id ?? null;
    videoAssets.set(assetName, null); // 先占位，防止同一名字递归重入
    // 素材在**项目里**那一份：`<项目>/assets/videos/<文件名>`。扩展名由库里那份决定，
    // 而库里那一份是 Rust `AssetDownloader::download_videos` 复制过来的 —— 这里按
    // **同名不同扩展名**逐个试（与贴纸那条"要试全部图片扩展名"是同一个坑）。
    const videoDir = join(clipDir, 'assets', 'videos');
    let found = null;
    try {
      const entries = readdirSync(videoDir);
      found = entries
        .filter((f) => {
          const stem = f.replace(/\.[^.]+$/, '');
          return stem === assetName && VIDEO_EXTS.some((e) => f.toLowerCase().endsWith(e));
        })
        .sort()[0] ?? null;
    } catch {
      found = null;
    }
    if (!found) {
      drop(
        'assets',
        `EDL 引用视频素材「${assetName}」，但项目里没有它的拷贝（找的是 assets/videos/` +
          `${assetName}.{${VIDEO_EXTS.map((e) => e.replace('.', '')).join('/')}}）—— ` +
          '这一段会缺画面。素材必须先复制进项目（跑一次后处理），**不许引用素材库里的绝对路径**：' +
          '那样换机器/库改名之后这个工程就出不了片了',
      );
      return null;
    }
    const rel = `assets/videos/${found}`;
    const abs = join(clipDir, rel);
    // ⚠️ **按素材自己的媒体探时基**（`probeMedia` 吸附到最近标准帧率）。
    const m = probeMedia(abs);
    if (!m || !m.fps) {
      note(
        'assets',
        `视频素材「${assetName}」：探不到它自己的时基（ffprobe 不可用 / 文件读不了），` +
          `按工程时基 ${assetTimebase.num}/${assetTimebase.den} 处理 —— 素材若不是这个帧率，` +
          '这一段的 `source_in` 帧号会整体错位（画面跳到素材里的别的位置）',
      );
    }
    const tb = m?.fps ?? assetTimebase;
    const id = rel;
    assets.push({
      id,
      kind: 'video',
      name: assetName,
      uri: rel,
      frame_count: null,
      timebase: tb,
      width: m?.width ?? mediaW ?? 1920,
      height: m?.height ?? mediaH ?? 1080,
      content_hash: null,
      tags: {},
      note: `来源：素材库视频「${assetName}」（已复制进项目）`,
    });
    const rec = { id, timebase: tb, width: m?.width ?? mediaW ?? 1920, height: m?.height ?? mediaH ?? 1080 };
    videoAssets.set(assetName, rec);
    if (m?.fps && (tb.num / tb.den) !== (assetTimebase.num / assetTimebase.den)) {
      note(
        'assets',
        `视频素材「${assetName}」的时基 ${tb.num}/${tb.den} 与源 ` +
          `${assetTimebase.num}/${assetTimebase.den} **不同** —— 这一段的 \`source_in\` 按` +
          '**它自己的**时基换算（规格 §11.2 第 3 条；混用就是"画面跳到源里 6 倍远的位置"）',
      );
    }
    return id;
  };

  // 登记 EDL 里引用到的全部素材（在 keepRanges 之前，因为下面按 `range.assetId` 取时基）。
  //
  // ⚠️ **片头/片尾的素材也要登记**（2026-10-09）：它们由下面 `intro_outro` 那段拼进时间线，
  //    而这里若不登记，`assetIdForRange` 会对它返回 `videoAssetId`（= 主素材）⇒
  //    **片头放的是正片开头那几秒**，且 `DROP` 里一行都没有（注册表里查不到就当"源"）。
  const edlClipsRaw = Array.isArray(config.edl?.clips) ? config.edl.clips : [];
  const ioRaw = config.intro_outro ?? {};
  const ioAssetNames = [ioRaw.intro, ioRaw.outro]
    .map((x) => (typeof x?.asset_id === 'string' ? x.asset_id.trim() : ''))
    .filter(Boolean);
  const referencedAssetNames = [
    ...new Set([...edlClipsRaw.map((c) => edlClipAssetId(c)), ...ioAssetNames].filter(Boolean)),
  ];
  for (const name of referencedAssetNames) registerVideoAsset(name);

  // ---- 音效素材 ----
  //
  // **必须核对文件真的在。** `polish.toml` 里写着 `poka01` / `poka02`，
  // 而这个切片的 `sfx/` 下只有 `poka.mp3` —— 直接按名字生成引用会得到
  // 一条**指向不存在文件的音轨**，而它在工程校验里是合法的
  // （校验只看"有没有登记"，不看"文件在不在"），要到渲染时才炸。
  // 那属于"看着成功、其实没验"。所以这里当场查盘，缺了就报出来。
  const sfxAssets = new Map();
  const registerSfx = (name) => {
    if (!name || sfxAssets.has(name)) return sfxAssets.get(name);
    const fileName = `${name}.mp3`;
    if (!existsSync(join(clipDir, 'sfx', fileName))) {
      drop('sfx', `音效文件 sfx/${fileName} 不存在 —— 这一条音效转不出来（同目录里没有同名文件）`);
      sfxAssets.set(name, null);
      return null;
    }
    const id = `sfx/${fileName}`;
    sfxAssets.set(name, id);
    assets.push({
      id,
      kind: 'audio',
      name,
      uri: `sfx/${fileName}`,
      frame_count: null,
      timebase: { num: 48000, den: 1 },
      width: null,
      height: null,
      content_hash: null,
      tags: {},
      note: '来源：polish.toml 的 sfx 事件',
    });
    return id;
  };

  // ---- 画布 ----
  const orientation = config.canvas?.orientation ?? 'landscape';
  const width = orientation === 'portrait' ? 1080 : 1920;
  const height = orientation === 'portrait' ? 1920 : 1080;
  // 参照模板 305 行：`SK = min(CW,CH)/1080` —— 贴纸的盒子/边距/上浮都按**短边**缩放。
  // 定义在画布尺寸旁边，因为贴纸那段（事件循环里）要用它（`const` 不提升）。
  const SK = Math.min(width, height) / 1080;

  // ---- 布局：liver 的聚焦点（= 运镜的**不动点**）----
  //
  // ⚠️ **口径以出片模板为准**（`crates/yekitrim-polish/templates/index.html:1413-1414`，权威是它）：
  //
  //     var defOrigin = liver.origin_grid ? gridOrigin(liver.origin_grid)
  //                                       : (liver.origin || '65% 50%');
  //     var dp = defOrigin.split(' '); ox = parseFloat(dp[0]) || 65; oy = parseFloat(dp[1]) || 50;
  //
  // 两条容易被"顺手修正"的地方，**都照抄**：
  //   1. **兜底是写死的 `65% 50%`**，不是画布中心 —— 而且 `origin = "center"` 这种
  //      **九宫格名字**在模板里 `parseFloat` 会 NaN，于是**也落到 65% / 50%**（演示工程正是这种：
  //      `grid = ["center"] / origin = "center"`）。按九宫格解成 50% 会与参照差一眼看得出的位移。
  //   2. 模板读的是 `origin_grid`（旧名），配置里写的是 `grid` —— 所以模板**根本没看那个字段**，
  //      这里也照它的实际行为来（先旧名，再 `origin` 串，最后 65% / 50%）。
  const liver = config.layout?.liver;
  const gridCenters = {
    'top-left': [1 / 6, 1 / 6], top: [1 / 2, 1 / 6], 'top-right': [5 / 6, 1 / 6],
    'mid-left': [1 / 6, 1 / 2], center: [1 / 2, 1 / 2], 'mid-right': [5 / 6, 1 / 2],
    'bot-left': [1 / 6, 5 / 6], bottom: [1 / 2, 5 / 6], 'bot-right': [5 / 6, 5 / 6],
  };
  const TEMPLATE_DEFAULT_ORIGIN = { x: 0.65, y: 0.5 };
  /// 模板那条 `defOrigin` 的等价物：`origin_grid` → `origin` 串 → **65% / 50%**。
  function templateDefaultOrigin() {
    if (liver?.origin_grid) {
      const cell = gridCenters[liver.origin_grid] ?? [1 / 2, 1 / 2]; // 模板 gridOrigin 认不出 → 50%
      return { x: cell[0], y: cell[1], source: `九宫格 ${liver.origin_grid}` };
    }
    const raw = typeof liver?.origin === 'string' ? liver.origin.trim() : '';
    const dp = raw.split(' ');
    const px = parseFloat(dp[0]);
    const py = parseFloat(dp[1]);
    if (raw && !Number.isNaN(px) && !Number.isNaN(py)) {
      return { x: px / 100, y: py / 100, source: `layout.liver.origin=${raw}` };
    }
    return { ...TEMPLATE_DEFAULT_ORIGIN, source: raw ? `模板兜底（origin="${raw}" 解析不出来）` : '模板兜底 65% 50%' };
  }
  const focus = templateDefaultOrigin();
  // 自由选框是**额外的一条路**：`process` 会把框选结果写进 `liver.origin`，所以正常工程上面就已经拿到了；
  // 只有"框了但没跑过后处理"的工程才会走到这里（这时框中心比 65% / 50% 更贴近意图）。
  if (liver?.top_left && liver?.bottom_right) {
    const tl = point(liver.top_left.x + ' ' + liver.top_left.y);
    const br = point(liver.bottom_right.x + ' ' + liver.bottom_right.y);
    if (tl && br && !/origin=/.test(focus.source)) {
      const mid = { x: (tl.x + br.x) / 2, y: (tl.y + br.y) / 2, source: '自由选框（未写回 origin）' };
      note('layout.liver', `自由选框中心 ${(mid.x * 100).toFixed(2)}% ${(mid.y * 100).toFixed(2)}% 覆盖模板兜底`);
      Object.assign(focus, mid);
    }
  }
  note('layout.liver', `聚焦点 ${(focus.x * 100).toFixed(2)}% ${(focus.y * 100).toFixed(2)}%（取自${focus.source}）`);

  // ---- 轨道 ----
  //
  // 一个**必须讲清楚的约束**：同一轨的图层不许重叠（`layer_overlap`）。
  // 而 YekiTrim 的运镜事件是**首尾相接**地铺满整段的，正好一条轨放得下；
  // 运镜要驱动 Transform，所以它就是**视频层自己的关键帧**，
  // 不需要另开一条轨 —— 这也是设计文档 §4.3 说的"运镜就是 Transform 上的关键帧"。
  const videoLayers = [];

  // ---------------------------------------------------------------------
  // `mute_ranges` = **把那些段从时间线上掐掉**，不是"静音"
  // ---------------------------------------------------------------------
  //
  // 这一条害我找了好几轮。实测证据：
  //
  //     源 clip.mp4      114.383s
  //     YekiTrim 成片       96.900s      <- 短了 17.49s
  //     三段 mute_ranges  5.30 + 2.36 + 9.83 = 17.49s   <- 正好对上
  //
  // 也就是说 `mute_ranges` 里那些段**在成片里根本不存在**，
  // 后面所有内容的时间**整体前移**。按"只静音、不裁剪"转出来的东西
  // 从第一刀（56.14s）之后**全部错位**，最大偏 17.49 秒。
  //
  // 实测对账（按"输出时间 = 源时间 - 之前掐掉的总长"换算之后）：
  //
  //     YekiTrim 65s  <-> 源 70.30s   逐像素差 9.17（错位时是 28.18）
  //     YekiTrim 79s  <-> 源 84.30s   逐像素差 6.72（错位时是 **93.74**）
  //     YekiTrim 90s  <-> 源 97.66s   逐像素差 8.39
  //
  // 所以这里做三件事：
  //   1. 算出**保留区间**（mute 的补集）；
  //   2. 主画面从"一整层"变成**每个保留区间一层**，`source_in` 指向该区间的源起点；
  //   3. 所有事件/字幕/弹幕的时间都**重映射**到裁剪后的时间线。
  const totalFrames = frameAt(config.duration ?? 0);
  const muteFrames = (config.mute_ranges ?? [])
    .map((m) => ({ start: frameAt(m.start ?? 0), end: frameAt(m.end ?? 0) }))
    .filter((m) => m.end > m.start)
    .sort((a, b) => a.start - b.start);

  // 保留区间（源时间，帧）。**掐掉的段真的不在里面。**
  //
  // ⚠️ **`--keep-muted`（出片那条腿）下这里退化成"整条一条保留区间"**（2026-09-30）：
  // 预览必须用压缩时间线（客户端靠"落在被掐掉的段里 = 没有对应帧"去跳屏蔽区间），
  // 而**出片不能** —— 出片尾部还有一道 `Ffmpeg::cut_ranges`（本仓那侧的整段重编码），
  // 它按**源时间**剪。这里先掐一遍、尾部再剪一遍 = **剪两次**：实测成片短一大截，
  // 而且音轨是从**整条**源视频抽的、没跟着压缩，第一刀之后全错位。
  // ⇒ 出片给"整条时间线"，屏蔽区间交给尾部那一剪（两边口径就统一了）。
  // ---------------------------------------------------------------------
  // **成片时间线（EDL）** —— 规格 `docs/plan/edl-retiming-spec.md` §2.2 / §3.1
  // ---------------------------------------------------------------------
  //
  // `config.edl.clips` 非空 ⇒ 成片时间线**显式给出来了**：数组顺序 = 成片播放顺序，
  // 区间可重复（重放）、可乱序（冷开场）。此时它**取代** `mute_ranges` 的补集
  // （§2.2 规则 3：「不引入"两者相加"的语义」）——屏蔽不再被删，而是把那几段的
  // **端点**夹开（用户口径原文：「屏蔽不是删除，是更改端点」，见 `buildKeepRanges`）。
  //
  // ⚠️ **`--keep-muted` 在 EDL 非空时不再短路**（§5）：出片尾部那道 `cut_ranges` 是与
  //    `mute_ranges` 绑在一起的，所以**调用点必须把两者一起跳过** —— 只让转译器不短路、
  //    尾部照剪，就是"剪两次"（实测 12s 带 3s 屏蔽出成 6.0s）。Rust 侧的对应改动在
  //    `render/pipeline.rs::cmd_render`（`has_edl` 那一段），两处**必须同时**。
  //
  // ⚠️ `totalFrames` 在"时长未知"时是 0（`frameAt(config.duration ?? 0)`）⇒ 那时 EDL
  //    段全会被 `clippedEnd > totalFrames` 夹没。这不是新问题（老分支同样退化成空），
  //    但要记住**它是静默的**：所以下面 `keepRanges` 为空时会打一条 NOTE（§4.2）。
  const edlClips = Array.isArray(config.edl?.clips) ? config.edl.clips : [];
  // ⚠️ **片头 / 片尾拼在 EDL 的首尾**（2026-10-09）。它与 Rust 的
  //    `ProjectFile::edl_clips_with_intro_outro` 是**同一条推导的两份实现**，必须逐值一致 ——
  //    不一致的症状是"画面有片头、音轨没有"或时长对不上，而 `mux_checked` 只在**样本数**上兜底。
  //
  //    为什么拼在这里：片头不过是"素材库里某段视频的一个区间"，而 `EdlClip` 本来就支持
  //    `source_kind = "asset"` ⇒ 拼进去就自动继承了 `assetId`/`source_in`/时基那一整套。
  //
  //    `out_sec = 0` 表示**播到素材结尾**，所以先拿素材时长：用**项目里那份拷贝**
  //    （`assets/videos/`）探，与 `registerVideoAsset` 同一份基准。
  //    ⚠️ 探不到 ⇒ **跳过这一侧并报 DROP**（静默少一段片头是最坏形态）。
  const introOutroClips = [];
  {
    const io = config.intro_outro ?? {};
    const ioVideoDir = join(clipDir, 'assets', 'videos');
    const findIoAsset = (name) => {
      try {
        return (
          readdirSync(ioVideoDir)
            .filter((f) => {
              const stem = f.replace(/\.[^.]+$/, '');
              return stem === name && VIDEO_EXTS.some((e) => f.toLowerCase().endsWith(e));
            })
            .sort()[0] ?? null
        );
      } catch {
        return null;
      }
    };
    for (const [side, which] of [
      ['intro', '片头'],
      ['outro', '片尾'],
    ]) {
      const item = io[side];
      const name = typeof item?.asset_id === 'string' ? item.asset_id.trim() : '';
      if (!name) continue;
      const found = findIoAsset(name);
      if (!found) {
        drop(
          'intro_outro',
          `${which}素材「${name}」在项目里没有拷贝（找的是 assets/videos/${name}.*）—— ` +
            '这一段不会出现在成片里。先跑一次后处理把素材复制进项目',
        );
        continue;
      }
      // ⚠️ **时长按素材自己的媒体探**（不是工程网格）——「播到结尾」要的是真时长。
      const m = probeMedia(join(ioVideoDir, found));
      const durSec = m?.duration ?? null;
      if (!(durSec > 0)) {
        drop(
          'intro_outro',
          `${which}素材「${name}」读不出时长，「播到结尾」是多久就定不下来 —— 这一段落不进去`,
        );
        continue;
      }
      const inSec = Math.max(0, Number(item.in_sec) || 0);
      // 与 Rust `IntroOutroClip::trim` 同口径：出点 <= 0 或 <= 入点 ⇒ 播到素材结尾
      const rawOut = Number(item.out_sec) || 0;
      const outSec = rawOut <= 0 || rawOut <= inSec ? durSec : Math.min(rawOut, durSec);
      const start = Math.min(inSec, durSec);
      if (!(outSec > start)) {
        drop('intro_outro', `${which}素材「${name}」的入出点算出来是空区间，这一段落不进去`);
        continue;
      }
      introOutroClips.push({
        side,
        which,
        clip: {
          source_start: start,
          source_end: outSec,
          speed: 1.0,
          kind: side,
          label: `${which}·${name}`,
          source_kind: 'asset',
          asset_id: name,
          // ⚠️ 片头/片尾都是**硬切**：片头前面没有可叠的东西，片尾的叠化会与
          // 用户自己排在末尾那段的 `transition_in` 抢"前一段"的语义。
          transition_in: null,
        },
      });
    }
  }
  // ⚠️ 没有 EDL 时不在这里造基片（那是 `buildKeepRanges` 的 `mute_ranges` 补集分支的事）——
  //    只有**已经给了 EDL** 的工程才在两端拼接，避免造出第二份"补集"推导。
  const composedEdlClips = introOutroClips.length && edlClips.length
    ? [
        ...introOutroClips.filter((x) => x.side === 'intro').map((x) => x.clip),
        ...edlClips,
        ...introOutroClips.filter((x) => x.side === 'outro').map((x) => x.clip),
      ]
    : edlClips;
  if (introOutroClips.length) {
    if (!edlClips.length) {
      drop(
        'intro_outro',
        '工程开了片头/片尾但没有成片时间线（`[edl.clips]` 为空）—— 本工具这条腿不替它造正片时间线，' +
          '这一段被跳过；请在编辑器里存过一次时间线，或直接跑 `yekitrim polish render`',
      );
    } else {
      note(
        'intro_outro',
        `片头/片尾拼进成片时间线：${introOutroClips.map((x) => x.which).join('、')}`,
      );
    }
  }
  const keepRanges = buildKeepRanges(totalFrames, muteFrames, keepMuted, composedEdlClips, fps);
  const durationFrames = keepRanges.length
    ? keepRanges[keepRanges.length - 1].outStart + (keepRanges[keepRanges.length - 1].durFrames ?? (keepRanges[keepRanges.length - 1].end - keepRanges[keepRanges.length - 1].start))
    : 0; // 裁剪后的总长
  const cutFrames = totalFrames - durationFrames;

  if (edlClips.length > 0) {
    // ⚠️ **别只对 `keepRanges` 的长度做判断** —— 它可能因为时长未知而整条为空，
    // 那时音轨（Rust 侧逐段组装）会拼出 **0 秒**，而成片画面还在。
    if (keepRanges.length === 0) {
      drop(
        'edl',
        `config.edl 有 ${edlClips.length} 段，但**一段都没能落地**（源总帧数=${totalFrames}，` +
          `fps=${fps}）—— 时长探不到时 EDL 推不出来。成片会是空的，请先确认 ` +
          '`[project] duration` / `data/clip-meta.json` 有真值',
      );
    } else {
      note(
        'edl',
        `成片时间线：${edlClips.length} 段 -> ${keepRanges.length} 段落地，` +
          `成片 ${(durationFrames / fps).toFixed(2)}s（源 ${(totalFrames / fps).toFixed(2)}s）；` +
          '**`mute_ranges` 整个被忽略**（规格 §2.3：有 EDL 时成片播什么只由 EDL 决定），' +
          '出片尾部那道 `cut_ranges` 也**不执行** —— 唯一那一刀就是 EDL 本身（规格 §5）',
      );
      // ⚠️ **重放（或任何重复源区间）必须点名**：它让同一条字幕/弹幕/音效出现在**多个
      //    不相邻的位置**，而"只按起点重映射一遍"的写法会让后来那几次整条错位、且不报错。
      //    这一行是给"用户说重放没生效"时第一眼看的证据。
      const seen = new Map();
      let dupes = 0;
      for (const range of keepRanges) {
        const key = `${range.start}-${range.end}`;
        if (seen.has(key)) dupes += 1;
        seen.set(key, true);
      }
      if (dupes > 0) {
        note(
          'edl',
          `其中有 ${dupes} 段与前面的段同源（重放/回忆回溯）：主画面与**原声**各出现两次` +
            `（各自一条 layer），音效与字幕/弹幕按**一对多**重映射（一条 cue 落在多个位置）`,
        );
      }
      // ---- 变速（第 4 批）----
      //
      // ⚠️ 第 2/3 批这里打的是 **CLAMP**：「画面按 speed 换算、音轨按原速拼」⇒
      // 变速段音画不同步，所以报告里明确警告"别拿它出片"。
      // 第 4 批把音轨那条腿实现掉了（`Ffmpeg::assemble_audio_from_edl` 逐段挂
      // `atempo`，见 `crates/yekitrim-polish/src/render/ffmpeg.rs::atempo_filter`），
      // 所以那条 CLAMP **整条作废** —— 留着它会把"正确行为"报成差异，
      // 而 CLAMP 是要用户逐条挂台账的东西（`docs/yekitrim-integration.md` 第五节）。
      //
      // 现在它降级成一条 **NOTE**：变速段要显式点名，因为它是"同一份画面两处换算"
      // 最容易分叉的地方（转译器缩段长 / Rust 缩音轨段），而**分叉时不报错**。
      // 这条 NOTE 是"用户说某段快放没生效"时第一眼要看的证据。
      const speeds = [...new Set(keepRanges.map((r) => r.speed ?? 1.0))].filter((s) => s !== 1.0);
      if (speeds.length > 0) {
        const sped = keepRanges.filter((r) => Math.abs((r.speed ?? 1.0) - 1.0) > 1e-9);
        note(
          'edl',
          `其中有 ${sped.length} 段变速（速率 ${speeds.join(' / ')}）：段长按 ` +
            '`round((ef-sf)/speed)` 缩（与 Rust `Edl::resolve_clips` 逐值一致），' +
            '**画面**挂 `source.source_rate`（底座 0.3.0 / D24 起；`rate` = 每输出 1 帧' +
            '消耗几个源帧，所以 `rate = speed`），' +
            '**音轨**逐段挂 `atempo`（时域伸缩，**基频不变**；用 `asetpts` 缩放会变调，规格 R6）。' +
            '两边吃的是同一批 clips，所以成片时长 == 派生值',
        );
      }
    }
  }

  /// 源帧号 -> 输出帧号。**可能落进多个段**（重放让同一段源出现两次）。
  ///
  /// ⚠️ 返回值**永远是数组**。`keepRanges` 在 EDL 下可以重复、可以乱序（重放 / 冷开场），
  /// 所以"源帧 → 输出帧"不再是一条单射的函数，而是一对多的关系。只取第一个命中的段
  /// （老写法）会让**后来那几次重放整条错位**，而且不报错 —— 这正是 §9 R7 那一类。
  ///
  /// 每段的输出帧按**它自己的**长度与位移算：段在成片里占 `durFrames` 帧（含 speed），
  /// 展开成 `[起点, 终点)` 闭区间求交。speed=1 且不重复时退化成老口径
  /// （`outStart + (sourceFrame - start)`）。
  const remapFrames = (sourceFrame) => {
    const out = [];
    for (const range of keepRanges) {
      if (sourceFrame < range.start || sourceFrame >= range.end) continue;
      const length = range.durFrames ?? (range.end - range.start);
      const speed = range.speed ?? 1.0;
      // 段内偏移换算到成片帧号（speed=1 时逐值等于 `sourceFrame - start`）。
      const local = Math.min(length - 1, Math.max(0, Math.floor((sourceFrame - range.start) / speed)));
      out.push(range.outStart + local);
    }
    return out;
  };
  /// 兼容旧写法：**只关心"第一个"命中的段**的那些调用点用它。
  /// ⚠️ 落在被掐掉的段里返回 `null`（那一段不存在）—— 老语义一字不改。
  const remapFrame = (sourceFrame) => {
    const all = remapFrames(sourceFrame);
    return all.length > 0 ? all[0] : null;
  };
  /// 源秒 -> 输出帧号（便捷）。
  const remapSeconds = (seconds) => remapFrame(frameAt(seconds));

  /// 源毫秒区间 -> 输出毫秒区间，**可能一段变多段**。
  ///
  /// `mute_ranges` 是把那些段从时间线上**删掉**，所以一条字幕/弹幕如果跨了
  /// 剪辑点，它在成片里就该**断成两截**（前半在剪辑点结束、后半从下一段续上）。
  /// 只按起点重映射会让它**整条错位** —— 那是"字幕对不上话"最直接的来源。
  ///
  /// ⚠️ **EDL 把"一段变多段"扩成"一对多"**（重放 → 同一条 cue 出现在多个**不相邻**位置）：
  /// 这里遍历的是**输入区间**（不是段），所以同一个输入区间与两个同源段各求一次交、
  /// 各自输出一条 —— 结果按**成片时间**排序（不是数组顺序！EDL 允许乱序）。
  const remapCueMs = (startMs, endMs) => {
    const sf = Math.round((startMs / 1000) * fps);
    const ef = Math.round((endMs / 1000) * fps);
    const out = [];
    for (const range of keepRanges) {
      const from = Math.max(sf, range.start);
      const to = Math.min(ef, range.end);
      if (to <= from) continue;
      const length = range.durFrames ?? (range.end - range.start);
      const speed = range.speed ?? 1.0;
      const offset = (sourceFrame) =>
        Math.min(length, Math.max(0, Math.round((sourceFrame - range.start) / speed)));
      const startFrame = range.outStart + offset(from);
      // 右端**向上取整**：区间被裁到半帧时不能塌成 0 时长（老口径也是这么处理的）。
      const endFrame = Math.max(startFrame + 1, range.outStart + offset(to));
      out.push({
        startMs: Math.round((startFrame / fps) * 1000),
        endMs: Math.round((endFrame / fps) * 1000),
      });
    }
    out.sort((a, b) => a.startMs - b.startMs || a.endMs - b.endMs);
    return out;
  };

  // ⚠️ **`source_in` 是"素材自己的帧号"，不是序列帧号**（2026-10-06 修）：
  // 底座 `EvalContext::source_frame` 是 `source_frame_looped(source_in, local_frame, 时间线时基, 素材时基, …)`
  // —— 它把**序列帧**按时间换算成素材帧，再把 `source_in` 加上去。所以两者**单位不同**：
  //   · 图层 `start`/`end` = 序列帧（工程网格，可能被抬到 30fps）；
  //   · `source_in` = 素材帧（源自己的 5fps / 24fps / 60fps）。
  // 单位混了会怎样（实测症状：**经过屏蔽区间之后声音和画面全乱**）：电台那种 5fps 素材 + 30fps 网格，
  // 第二段本该从素材第 20 帧起，写成序列帧就是 120 ⇒ 画面跳到源里 6 倍远的位置，而声音（宿主按
  // `source_in / 工程帧率` 换算的**秒**）是对的 ⇒ 两边从第一个屏蔽段起就分叉。
  // 素材帧率与网格一致时（老工程、60fps 素材）两者数值相同 ⇒ 老产物逐字节不变。
  //
  // ⚠️⚠️ **第 6 批：这里从"一份 `assetTimebase`"变成"每个 asset 各算一遍"**（规格 §11.2 第 3 条）。
  //
  // 改之前它是 `(seqFrame) => round(seqFrame / fps * (assetTimebase.num / assetTimebase.den))`
  // —— 写死的是**源**那一份时基。EDL 一旦指向**第二个素材**（帧率可能 24 / 30 / 60 各不同），
  // 那个式子算出来的是**别的素材的帧号**，症状是"**画面跳到源里 6 倍远的位置**"，且**不报错**。
  //
  // 现在按 asset 各取自己那一份（`assetTimebaseFor`），**序列帧的那一半（`/ fps`）不变** ——
  // 变的只有"序列秒 → 该素材帧"那一步的帧率。**源素材仍走原来那一份**，所以
  // "EDL 段全指源"的老产物**逐字节不变**（R4；两个 sha256 基线钉着）。
  const assetTimebaseFor = (assetId) => {
    if (!assetId) return assetTimebase; // 无素材 id ⇒ 源那一份（老路径逐值不变）
    const rec = videoAssets.get(assetId);
    // 认不出（登记时已 DROP 过）⇒ 退回源那一份，**不再重复报错**
    return rec?.timebase ?? assetTimebase;
  };
  /// 序列帧 → **指定素材**自己的帧号。
  const toSourceFramesFor = (seqFrame, assetId) => {
    const tb = assetTimebaseFor(assetId);
    return Math.round((seqFrame / fps) * (tb.num / tb.den));
  };
  /// 序列帧 → **源**素材帧号（老路径的唯一口径；`toSourceFramesFor(x, null)` 的别名）。
  const toSourceFrames = (seqFrame) => toSourceFramesFor(seqFrame, null);
  /// 一个保留区间的画面来自哪个 asset（`range.assetId` 为空 ⇒ 源）。
  const assetIdForRange = (range) =>
    (range.assetId && videoAssets.get(range.assetId)?.id) || videoAssetId;

  // =====================================================================
  // **叠化（第 7 批 / task-10）** —— 跨轨重叠 + 上层 `opacity` 关键帧
  // =====================================================================
  //
  // 规格 §11.3 的**方案 A**，可行性由 `.workbuddy/edl-transition-probe/` 实测：
  //   · **同轨**重叠 ⇒ 底座 exit 2 `layer_overlap`（这条腿走不通）；
  //   · **跨轨**重叠（后一条 track 在上）⇒ exit 0；
  //   · 跨轨 + 上层 `opacity` 0→1 线性关键帧 ⇒ exit 0、`issues: []`、**真叠化**
  //     （最大帧间 YAVG 跳变 **2.22** vs 硬切 **26.97**）。
  //
  // # 几何：谁动、动多少
  //
  // `transition_in = "dissolve"` 的段（下称 B）与它**前一段**（A）在成片时间轴上
  // **重叠 `overlapFrames` 帧**：B 的起点往前挪 `overlapFrames`，于是
  //   `[A.outStart, B.outStart+overlap)` 这一段两层同时可见。
  //
  // ⚠️ **挪的是"画面上那一层"的起点、不是 `keepRanges` 的 `outStart`**。
  // 这两件事必须分开：`outStart` 是"成片时间轴"的唯一推导（三处消费点逐值同构，
  // `check-edl-invariants.mjs` 钉着），改它等于改整条下游（字幕/弹幕/音效重映射、
  // 总时长、合流断言）。所以 **`outStart` 一个字节都不动**，只把主画面那层
  // 的 `start` 提前 —— 底座的 `source_in` 是按层自己的 `start` 求值的，
  // 提前 `start` 就是"提前开演"，而**总时长不变**（B 的 `end` 也跟着提前）。
  //
  // ⇒ 代价（如实记下来）：重叠窗口里 B 播的是**它自己的开头**、A 播的是**它自己的结尾**
  //    —— 这正是"交叉淡化"的定义（两段各让出一部分同时演），不是 bug。
  //
  // # 为什么切到第二条 track
  //
  // 底座同轨不许重叠（`layer_overlap`）。所以带叠化的段要落到 `v2`。
  // `v2` 在 `v1` **之后** ⇒ 它画在上面（探针 Q2 实测的 z 序），
  // 于是"B 淡入盖上 A"成立。
  //
  // ⚠️ **`v2` 内部同样不许重叠**：两段**都**带叠化时（B 与 C），
  // B 在 `v2`、C 也在 `v2` 就会重叠。所以这里做的是**交替分轨**
  // （A→v1、B→v2、C→v1…，按"这一段有没有叠化 + 上一段在哪条轨"决定），
  // 而不是"所有叠化段都塞 v2"。
  //
  // ⚠️ **只支持单层**：v2 上是 B、v1 上是 A —— 若叠化段的前一段**自己也在 v2**，
  // 那就是 v2 内部重叠 ⇒ 底座 exit 2。交替分轨正是为了避免它。
  const overlapFramesOf = () => dissolveOverlapFrames(fps);

  // 每段的**画面层几何**：`{ track, start, end, fade }`。
  //
  // `track` = 0 落到 `v1`、1 落到 `v2`（`v2` 在后 ⇒ 在上）。
  // `fade` = 该层自己的 `opacity` 0→1 关键帧（**图层相对帧号**，见下面那段注释）。
  const mainGeom = [];
  let trackCursor = 0; // 当前这一段落在哪条轨（0/1 交替）
  for (let index = 0; index < keepRanges.length; index += 1) {
    const range = keepRanges[index];
    const durFrames = range.durFrames ?? (range.end - range.start);
    const wantsDissolve = range.transition === EDL_TRANSITION_DISSOLVE;
    const overlap = wantsDissolve ? overlapFramesOf() : 0;
    // 前一段的**实际结束帧**（可能因为拖过 `start` 而提前）。
    const prev = index > 0 ? mainGeom[index - 1] : null;
    // 叠化的**成立条件**（三条都要满足，缺一条就退化成硬切并报 DROP）：
    //   ① 有前一段（成片的第一段没有"前一段"可叠）；
    //   ② 重叠帧数 ≥ 1（fps 探得出来）；
    //   ③ 重叠 < 前一段与这一段各自的长度 —— 否则前一段会被整段吃掉。
    //      ⚠️ 判据用**前一段挪过之后的实际长度**，不是它的名义 `durFrames`。
    const prevLen = prev ? prev.end - prev.start : 0;
    const canDissolve =
      wantsDissolve && prev !== null && overlap >= 1 && overlap < durFrames && overlap < prevLen;
    let start = range.outStart;
    if (wantsDissolve && !canDissolve) {
      drop(
        'edl.transition',
        `第 ${index + 1} 段声明了 transition_in = "dissolve"，但**叠化条件不成立**，` +
          `已按硬切出片：前一段存在=${prev !== null}、重叠=${overlap} 帧、` +
          `本段长=${durFrames} 帧、前一段（挪后）长=${prevLen} 帧。` +
          `要求 1 ≤ 重叠 < min(本段长, 前一段长)。` +
          `（第一段没有"前一段"可叠；段太短时叠化会把它整段吃掉 —— 那不该静默发生）`,
      );
    } else if (canDissolve) {
      start = range.outStart - overlap;
    }
    // 交替分轨：本段要叠化就得与**前一段所在的轨**不同（跨轨才合法）；
    // 不要叠化时回到 v1（v1 是"底"那条，主画面绝大多数段都在它上面）。
    trackCursor = canDissolve ? (prev?.track === 0 ? 1 : 0) : 0;
    mainGeom.push({
      track: trackCursor,
      start,
      end: start + durFrames,
      // 重叠窗口的**帧数**（0 = 没有叠化）。淡入关键帧的终点就是它。
      overlap: canDissolve ? overlap : 0,
    });
  }

  // 主画面：**每个保留区间一层**。
  //
  // ⚠️ **层序 = `keepRanges` 的顺序 = 成片播放顺序**（不是源顺序）。冷开场/乱序剪辑下
  // "图层顺序 ≠ 源顺序"是**故意的**，也正是这些手法能成立的原因（§9 R3）。
  // `source_in` 仍按**素材帧**给（下一行那句话不能动）。
  //
  // ⚠️ 第 6 批起：`asset_id` 与 `source_in` **都按该段自己的素材**取 —— 素材段指向
  // 第二个 asset，而它的 `source_in` 按**那个素材自己的时基**换算（见上面的
  // `toSourceFramesFor`）。两件事一起改，只改一件就是"指向了新素材、帧号还是老的"。
  //
  // ⚠️ 第 7 批起：`start`/`end` 走上面的 `mainGeom`（带叠化的段会提前），
  // 并按 `track` **拆成 v1 / v2 两条轨**（`mainLayers` 只留 v1 那一半）。
  const mainLayers = keepRanges.map((range, index) => {
    const assetId = assetIdForRange(range);
    const isSource = assetId === videoAssetId;
    const geom = mainGeom[index];
    // 叠化的**上层**B：`opacity` 0→1 线性关键帧，窗口 = 重叠区。
    //
    // ⚠️⚠️ **关键帧是图层相对帧号**（`schema.rs:732`「相对片段起点，0..=M-1」）——
    // 这里 `geom.start` 被提前了 `overlap` 帧，所以**层内的 0..overlap** 正好就是
    // 重叠窗口。写成**绝对**帧号会"一个键都不落在区间里、且不报错"（探针 REPORT 第 3 条）。
    const keyframes =
      geom.overlap > 0
        ? [
            { frame: 0, target: 'opacity', value: 0, easing: 'linear' },
            { frame: geom.overlap, target: 'opacity', value: 1, easing: 'linear' },
          ]
        : [];
    return {
      id: `main-${index}`,
      start: geom.start,
      // ⚠️ **用 `durFrames`，不是 `end - start`**：EDL 段带 speed 时两者不等
      // （`durFrames = round((end-start)/speed)`，与 Rust `resolve_clips` 逐值一致）。
      end: geom.end,
      // ⚠️ **`source_rate` 是画面变速的唯一表达**（底座 0.3.0 / D24，2026-10-09）。
      //
      // 在那之前底座**没有**任何表达播放速率的字段：`end - start` 同时当"占多长输出时间"
      // 和"要读多少源帧"用，两者被绑死。于是这里只能靠 `durFrames` 缩短段长 ——
      // **时长对了，但内容被截断**：`speed=2.0` 的段只播了源的前一半
      // （Lead 用亮度斜坡源从成片像素反推实测：计划 src 60→120，实际 59→89）。
      //
      // 现在语义分开了：`end - start` = **输出**帧数，`source_rate` = 每输出 1 帧消耗
      // **几个**源帧。所以 `speed=2.0` ⇒ `source_rate=2.0`，段长仍是 `durFrames`，
      // 而底座会去读 `durFrames × rate` 个源帧 —— 两头一凑正好是这一段该播的源区间。
      //
      // ⚠️ **不是 `1/speed`**：方向容易记反。`rate` 是"消耗源帧的速率"，
      // 2 倍速 ⇒ 每输出一帧要吃掉 2 个源帧 ⇒ `rate = speed`。
      //
      // ⚠️ **缺省 1.0 时不要写这个键**（`serde(default)`）—— 写了虽然语义相同，
      // 但会让"原速工程"与老产物的 JSON 不再逐字节相同（R4 那条基线正是逐字节比对）。
      source: {
        asset_id: assetId,
        source_in: toSourceFramesFor(range.start, isSource ? null : range.assetId),
        ...(Math.abs((range.speed ?? 1.0) - 1.0) > 1e-9
          ? { source_rate: range.speed ?? 1.0 }
          : {}),
      },
      transform: { x: 0, y: 0, scale: 1.0, rotation: 0 },
      // ⚠️ **基准 `opacity` 必须是 0**（不是 1）：关键帧第一条就是 0，而底座的
      // "末键之后取 last.value" 语义在这里用不上（0 键层立刻被覆盖）。写 1 也对
      // （首帧就落在 0 键上），但**写 0 更不容易被误读成"这层不透明"**。
      opacity: geom.overlap > 0 ? 0.0 : 1.0,
      blend: 'normal',
      enabled: true,
      effects: [],
      keyframes,
      note:
        (isSource
          ? `主画面第 ${index + 1}/${keepRanges.length} 段（源第 ${range.start}~${range.end} 帧）`
          : `主画面第 ${index + 1}/${keepRanges.length} 段（**素材 ${range.assetId}** 第 ` +
            `${toSourceFramesFor(range.start, range.assetId)}~${toSourceFramesFor(range.end, range.assetId)} 帧）`) +
        (geom.overlap > 0
          ? `；**叠化**：${geom.overlap} 帧淡入（第 ${geom.start}~${geom.start + geom.overlap} 帧与上一段重叠）`
          : ''),
    };
  });

  /// 把一个关键帧放到**它所在的那一层**上，帧号换成该层内的相对帧号。
  ///
  /// 关键帧的帧号是**相对图层起点**的（`schema.rs:732`），而裁剪之后
  /// 源时间被切成了好几层 —— 所以不能再用绝对帧号。
  const placeKeyframe = (sourceFrame, target, value, easing) => {
    // ⚠️ **一对多**：EDL 下同一段源可以出现在成片里两次（重放），运镜/手持/抖动
    // 那套逐帧变换**两边都要写**。只写第一个命中的段 ⇒ 重放那一次的运镜与抖动整条丢失，
    // 而且不报错（第 2/3… 次重放看起来"就是没有运镜"）。
    // ⚠️ `range.end` 那一帧**也**要能落键（老写法是 `<=`）—— 段的最后一帧是合法的关键帧位置。
    let placed = false;
    for (let index = 0; index < keepRanges.length; index += 1) {
      const range = keepRanges[index];
      if (sourceFrame >= range.start && sourceFrame <= range.end) {
        const layer = mainLayers[index];
        const speed = range.speed ?? 1.0;
        const local = range.outStart + Math.round((sourceFrame - range.start) / speed) - layer.start;
        layer.keyframes.push({ frame: Math.max(0, local), target, value, easing });
        placed = true;
      }
    }
    return placed; // 一次都没落上 = 落在被掐掉的段里
  };

  /// 把一个**特效**挂到它所在的那一层上（整层特效，帧号是轨道级的，不用转）。
  ///
  /// 特效的 `window` 是按**该层内的 local frame** 求值的，所以挂到裁剪后的
  /// 某一层上时，`fade_in`/`transient` 那些数字仍然是"这一段内部的秒数" ——
  /// 与 YekiTrim 的语义一致（它的 shake 也是相对事件起点算的）。
  /// **把缓动从"段落起点"挪到"段落终点"。**
  ///
  /// 契约的求值用的是相邻对 `[a, b]` 里 **`b` 的缓动** —— 也就是"怎么**到达** b"
  /// （`curve.rs::channel_from`：`b.easing.apply(t)`）。而人写关键帧时的直觉是
  /// "从这个键开始用这条曲线"（CSS 的 `animation-timing-function` 就是这种）。
  ///
  /// **两者差一个位置，而差出来的效果非常像"曲线参数没调好"**：
  /// 实测 19.9s 那一次运镜（`中->近`、`td=0.6s`）两端精确、中段偏小 2~3% ——
  /// 因为 `ease_out` 写在起点键上，而这一段实际取的是终点键的 `linear`，
  /// **整整一段运镜变成了直线插值**。
  ///
  /// 逐通道独立处理（不同 `target` 是各自一条曲线）。
  const shiftEasingToArrival = (keyframes) => {
    const byTarget = new Map();
    for (const key of keyframes) {
      if (!byTarget.has(key.target)) byTarget.set(key.target, []);
      byTarget.get(key.target).push(key);
    }
    for (const keys of byTarget.values()) {
      keys.sort((a, b) => a.frame - b.frame);
      for (let index = keys.length - 1; index >= 1; index -= 1) {
        keys[index].easing = keys[index - 1].easing;
      }
      // 第一个键的缓动只服务"它之前的那一段"，而它之前没有键（用 fallback）。
      if (keys.length > 0) keys[0].easing = 'linear';
    }
    return keyframes;
  };

  const placeEffect = (sourceFrame, effect) => {
    // ⚠️ **一对多**（同 `placeKeyframe`）：重放/重复源区间下，同一个特效**每一份都要挂上**。
    // 只挂第一份 ⇒ 第二次重放的画面没有那个特效，而报告里一行差异都没有。
    let placed = false;
    for (let index = 0; index < keepRanges.length; index += 1) {
      const range = keepRanges[index];
      if (sourceFrame >= range.start && sourceFrame < range.end) {
        mainLayers[index].effects.push(effect);
        placed = true;
      }
    }
    return placed;
  };

  /// **发一条调整图层**，而不是把特效挂在实拍片段上。
  ///
  /// # 为什么必须这样（实测踩出来的）
  ///
  /// 渲染器把图层切成两种步骤（`core::render::plan_steps`）：
  /// **有素材的**进 `Draw`，**没素材、有特效的**（调整图层）才进 `Adjust`。
  /// 而 `Draw` 只认一条特效 —— 跑在**源纹理**上的 `SeparableBlur`（`gaussian_blur`）。
  ///
  /// 于是「把 shake 挂在 main-0 上」的结果是：通过校验、出现在 JSON 里、
  /// 被求值层算出来、**然后被整条丢掉**。实测确认：
  /// `shake` / `zoom_bounce` / `pulse` 挂在普通图层上时，把 `amount` 从 0 改成
  /// 0.2（1080p 下 216 像素）出帧**逐像素相同**。
  ///
  /// 这是底座第 5 次「写好了没接上」，也是最大的一次（一整条管线级）。
  /// 契约层现在会为此报 `effect_would_be_ignored`；这里是从源头做对。
  ///
  /// # 顺带解决"窗口基准"
  ///
  /// `Effect::strength(local, …)` 的 `local` 是**图层局部帧**。挂在 main-0（整片一层）上时，
  /// 想表达"第 19.9 秒抖一下"只能拿 `transient` 窗口去凑 —— 而那是错的。
  /// 图层一独立，**图层边界就是事件窗口**，`local == 0` 天然就是事件起点。
  const placeAdjustment = (srcStart, srcEnd, effect, where) => {
    const startFrame = remapFrame(srcStart);
    if (startFrame === null) {
      note(where, '这一条整段落在被掐掉的区间里，跳过（成片里没有那一段）');
      return false;
    }
    const endFrame = remapFrame(srcEnd) ?? startFrame + 1;
    const id = `adj${adjustmentEffects.length}`;
    adjustmentEffects.push({
      id,
      start: startFrame,
      // 区间左闭右开，至少一帧。
      end: Math.max(endFrame, startFrame + 1),
      effect,
      opacityKeyframes: [],
    });
    return true;
  };

  /// 挂一个**逐帧控制强度**的调整图层。
  ///
  /// `shape(localSec)` 返回 0~1 的**包络**（相对峰值）。峰值本身写在效果的
  /// `amount` 参数上，于是 `效果强度 × 图层不透明度` 就是参照那条逐帧曲线。
  ///
  /// # 为什么非要逐帧
  ///
  /// YekiTrim 的 `flash` / `hue_shift` / `noise` 都是**先升到峰值再线性落回**的
  /// 短包络（0.04~0.15 秒的攻击段）。用契约里现成的 `window`（attack/hold/release
  /// 的帧数）去凑是凑不出来的：那几条曲线**各有各的分段点**，而 attack 段
  /// 只有 2~9 帧。所以与手持/贴纸同一个办法 —— **按帧采样**。
  ///
  /// # 前提
  ///
  /// 调整图层的 `opacity` **必须真的被渲染器读到**。这一条先前是缺的
  /// （`Step::Adjust` 只带 `effects`），一起补上了 —— 见
  /// **底座仓**的 `crates/dhampir-core/src/render/timeline.rs` 里 `Step::Adjust` 的文档。
  /// 与 [`placeAdjustment`] 同，但**直接给输出帧号**（不经过源帧映射）。
  ///
  /// 给"按帧拆层"用：层层都是**输出时间轴**上的一帧宽，
  /// 而 `placeAdjustment` 的入参是源帧 —— 逐帧拆时两边容易搞混。
  const placeAdjustmentOut = (outStart, outEnd, effect, where) => {
    adjustmentEffects.push({
      id: `adj${adjustmentEffects.length}`,
      start: outStart,
      end: Math.max(outEnd, outStart + 1),
      effect,
      opacityKeyframes: [],
    });
    return true;
  };

  const placeEnvelope = (outStart, seconds, effect, shape, where) => {
    if (outStart === null) {
      note(where, '这一条整段落在被掐掉的区间里，跳过（成片里没有那一段）');
      return false;
    }
    let frames = Math.max(1, Math.round(seconds * fps));
    // ⚠️⚠️ **窗口不许长过它所在的那一段**（乱序 EDL 下实测踩到的真 bug）。
    //
    // `seconds` 是**源时间**上的事件时长，而 `outStart` 是重映射之后的成片帧号。
    // 顺序 EDL 下两者恰好吻合（段是紧接着的），**乱序/重放**下就不吻合了：
    // 实测一条 `source_start=6.4s` 的 flash（源里 7 秒的窗口）落在成片第 384 帧 ——
    // 而那一段只到 420 帧（成片尾），于是窗口溢到 **433 帧**，
    // 底座按"所有层 end 的最大值"出片 ⇒ 成片 **433 帧 vs 派生 420 帧**，
    // 被合流前那条断言拦下（`mux_checked`，正是为这类静默错误加的）。
    //
    // 夹到**所在 keepRange 的右端**：多出来的部分本来就不在成片里，
    // 画上去只会把成片拉长。
    const owner = keepRanges.find((r) => outStart >= r.outStart && outStart < r.outStart + (r.durFrames ?? (r.end - r.start)));
    if (owner) {
      const ownerEnd = owner.outStart + (owner.durFrames ?? (owner.end - owner.start));
      const room = ownerEnd - outStart - 1; // 末键必须严格小于层时长
      if (room < frames) frames = Math.max(1, room);
    }
    const keyframes = [];
    for (let i = 0; i <= frames; i += 1) {
      keyframes.push({ frame: i, target: 'opacity', value: shape(i / fps), easing: 'linear' });
    }
    adjustmentEffects.push({
      id: `adj${adjustmentEffects.length}`,
      start: outStart,
      // 最后一个关键帧必须**严格小于**图层时长。
      end: outStart + frames + 1,
      effect,
      opacityKeyframes: keyframes,
    });
    return true;
  };

  if (muteFrames.length > 0) {
    // ⚠️ **EDL 非空时这段注释的措辞必须换**（否则是**误导**）：那时屏蔽没有被"掐掉"，
    // 而是把那几段的**端点**夹开了（用户口径原文：「屏蔽不是删除，是更改端点」）。
    // 同一份报告里如果既写"掐掉 3 秒"又写"EDL 3 段落地"，看报告的人会拿这两个数去对账。
    const muteSeconds = (muteFrames.reduce((s, m) => s + (m.end - m.start), 0) / fps).toFixed(2);
    note(
      'mute_ranges',
      edlClips.length > 0
        ? `${muteFrames.length} 段屏蔽区间（共 ${muteSeconds} 秒）**在有 EDL 时整个不生效** —— ` +
          '成片播什么**只由 EDL 决定**（规格 §2.3）：屏蔽段本来就在源时间轴上，' +
          '用户新拖出来的段覆盖到它是**故意**的（"这段我就是要"）。' +
          '轨道上的红块照旧显示、也照旧可编辑，它只是不再是出片的**判据**。' +
          '⚠️ **别在这里再叠一次求交** —— 那是第 2 份推导，而两份推导的分歧只表现为"成片短一段"、不报错'
        : keepMuted
          ? `${muteFrames.length} 段屏蔽区间（共 ${muteSeconds} 秒）**保留在时间线里**` +
            `（--keep-muted：出片尾部按源时间剪一次（本仓那侧）；` +
            `预览那条腿不能这么转）`
          : `${muteFrames.length} 段被**掐掉**（不是静音）：共 ${(cutFrames / fps).toFixed(2)} 秒，` +
            `成片从 ${(totalFrames / fps).toFixed(2)}s 变成 ${(durationFrames / fps).toFixed(2)}s；` +
            `主画面因此拆成 ${keepRanges.length} 段，所有事件时间已重映射`,
    );
  }

  // ---------------------------------------------------------------------
  // **源是不是竖素材**（`vertical`）—— 六条轨道开关里"素材竖"那一档的输入
  // ---------------------------------------------------------------------
  //
  // 判据 = `h > w`（沿用既有口径，Rust 侧 `lane_default` 读同一件事）。
  // 它**不再**单独关掉运镜：0.6.0 起「素材竖 ⇒ 运镜/视觉特效默认关」是
  // `[stage.lanes]` 缺字段时的**默认值**，眼睛（或 toml 里写 true）一点就开 ——
  // 参照侧（`templates/index.html::getActiveCamera` / `engine.js::cameraAt`）与这里
  // 读的是**同一份** `lanes`，三端不可能各关各的。
  //
  // # 为什么这里要探媒体（与"不探测媒体"那条纪律的关系）
  //
  // 转译器的纪律是"**不探测媒体**：`frame_count` 留空，由底座的
  // `dhampir probe --write` 回填" —— 为的是"在一台没有 ffmpeg 的机器上也能跑完
  // 并给出清单"。**贴纸 GIF 已经是例外**（要它的帧数与尺寸算缩放）。
  //
  // 这里沿用同一条例外纪律：**探得到就照做，探不到就进 NOTE 明说**（明说不装成功）。
  // 用 [`probeSize`]（只问宽高，不带 `-count_frames`），
  // 对一条长视频是毫秒级的，不会把转译拖成"整条解码"。
  const sourceVideoPath = join(clipDir, config.source?.video ?? 'clip.mp4');
  const sourceSize = existsSync(sourceVideoPath) ? probeSize(sourceVideoPath) : null;
  /// `true` = 竖素材；`false` = 横素材；`null` = 判不了（探不到）。
  const sourceIsVertical = sourceSize ? sourceSize.height > sourceSize.width : null;

  // ---- 六条轨道的**显示开关**（时间轴每条轨道前那枚眼睛，2026-10-06 用户口径）----
  // 语义 = **按整类屏蔽**：只在转译侧跳过（成片与底座预览里都没有这一类），
  // **事件本身一个字都不动** —— 与「屏蔽只做标记」同一条纪律，界面点亮眼睛即完全恢复。
  //
  // 判据 = `[stage.lanes] <lane>`；**缺字段 = 自动**，默认值与 Rust
  // `ProjectFile::lane_default` **同口径**（两边必须一模一样，判据见
  // `scripts/check-yekitrim-translator.mjs::checkLaneSwitches`）：
  //
  //   * 运镜 / 视觉特效：**素材竖 或 画布竖** ⇒ 默认关；
  //   * 音效：**画布竖** ⇒ 默认关（与素材方向无关）；
  //   * 字幕 / 弹幕 / 贴纸：恒默认开。
  //
  // ⚠️ 「素材竖」这一档**取代了** 0.5.0 的 `portraitNoCamera` 硬门（源竖 + 画布非竖 ⇒
  //    运镜整条关掉、**界面改不了**）。用户 2026-10-06 报的「竖屏素材不应该默认不启用视觉特效
  //    轨道的内容」就是它 —— 现在它只是**默认值**，眼睛一点就能开，参照侧读同一份注入值。
  const CANVAS_PORTRAIT = (config.canvas?.orientation ?? 'landscape') === 'portrait';
  const PORTRAIT_SRC = sourceIsVertical === true;
  /** 缺字段时的默认（**与 Rust `ProjectFile::lane_default` 逐条一致**）。 */
  const LANE_DEFAULT = laneDefaults(PORTRAIT_SRC, CANVAS_PORTRAIT);
  /** `true`/`false` = 用户显式设过；`null` = 自动。 */
  const laneExplicit = (lane) => {
    const v = config.stage?.lanes?.[lane];
    return typeof v === 'boolean' ? v : null;
  };
  const laneOn = (lane) => laneExplicit(lane) ?? LANE_DEFAULT[lane] === true;
  /// 事件类型 → 轨道（不在表里 = 不受门控）。
  /// 「视觉特效」口径 = 视频层那 8 类 + `overlay`（含片头 `fade_in` 那条覆盖层）。
  /// ⚠️ 与 Rust `Lane::of_event`、`webui/src/polish/engine.js::LANE_OF_TYPE` **同集合**。
  const LANE_OF_TYPE = {
    camera: 'camera',
    // 元素（§5）沿用 sticker 轨（规格 §7 座位表）—— 与 engine.js / 模板 / Rust 同集合
    element: 'sticker',
    sticker: 'sticker',
    sfx: 'sfx',
    shake: 'effects',
    blur: 'effects',
    flash: 'effects',
    hue_shift: 'effects',
    noise: 'effects',
    vignette: 'effects',
    overlay: 'effects',
    pulse: 'effects',
  };
  const laneAllows = (type) => {
    const lane = LANE_OF_TYPE[type];
    return !lane || laneOn(lane);
  };
  if (sourceIsVertical === null) {
    note(
      'source.video',
      '探不到源素材尺寸（ffprobe 不可用？）⇒ **判不了"素材竖"**，运镜/视觉特效按"素材不竖"取默认' +
        '（横画布下 = 开）。参照侧读的是 Rust 注入的同一档，两边一致',
    );
  } else if (PORTRAIT_SRC && !CANVAS_PORTRAIT) {
    note(
      'source.video',
      `源素材是**竖的**（${sourceSize.width}x${sourceSize.height}）而画布不是 ⇒ 运镜/视觉特效**默认关**` +
        '（0.6.0 起是"眼睛默认灭"，`[stage.lanes]` 里写 true 即开）',
    );
  } else if (PORTRAIT_SRC) {
    note(
      'source.video',
      `源素材是**竖的**、画布**也是竖的** ⇒ 运镜/视觉特效**默认关**；` +
        '`[stage.lanes] camera/effects = true` 可点亮',
    );
  }
  for (const lane of ['subtitles', 'danmaku', 'camera', 'sticker', 'effects', 'sfx']) {
    if (laneExplicit(lane) === false) {
      note(
        'stage.lanes',
        `\`${lane}\` 轨的眼睛**关着** ⇒ 这一整类不转译（事件/素材一个字都没动，点亮即恢复）`,
      );
    }
  }

  // ---- 逐事件转译 ----
  const sfxClips = [];
  // 覆盖层事件数（进 `extensions.yekitrim.polish_toml` 的元数据 + CLI 摘要）。
  //
  // ⚠️ 以前这里是个 `overlayLayers` **数组**：只有 `gradient` 那档往它里面推，
  // 于是"四档里接了几档"从计数上根本看不出来（M2 补齐 `color` / `fade_in|fade_out` 时，
  // 那个数组就彻底没人写了 —— 计数会永远显示 0，是**比不计数更糟**的那种元数据）。
  // 现在四档统一走 `placeEnvelope`，这里就改成一个诚实的计数器。
  let overlayEventCount = 0;
  // 文字元素（§5 E3）：**每条独立一层**，不复用字幕轨。
  //
  // ⚠️ 为什么不共用 `subtitleTracks`：那条轨的 `SubtitleStyle` 是**字幕的**
  //    （`font_ratio` 0.0667、底部、`max_lines` 3）—— 大字混进去会被字幕的
  //    字号与位置规则吃掉。四档预设的字号差近 3 倍，必须各排各的。
  const textCues = [];
  // 文字样式的**兜底**（§5 E3）。
  //
  // ⚠️ **正常路径不读这里** —— 样式由 `process` 的 Step 1d3 解析后写进
  //    `event.source`（格式见 `process/mod.rs::layout_text_elements`），
  //    转译器只**解析**它。这里这份只在"工程没过 `process`"时兜底
  //    （直接手写 polish.toml 跑转译器的那种）。
  //
  // ⚠️ 为什么不在这里维护第二份真表：两边各存一份数值，分叉的症状是
  //    "算出来的盒子与画出来的字对不上"（文字溢出自家的底衬），
  //    而**两边单测各自都过** —— 这类"两份同一个事实"的坑本仓踩过多次。
  const TEXT_STYLE_FALLBACK = {
    hook: { fontPx: 120, strokePx: 8, color: [255, 255, 255, 255], strokeColor: [26, 26, 26, 255], weight: 900, backdrop: false },
    chapter: { fontPx: 84, strokePx: 5, color: [255, 255, 255, 255], strokeColor: [26, 26, 26, 255], weight: 700, backdrop: true },
    comment: { fontPx: 44, strokePx: 2, color: [255, 226, 122, 255], strokeColor: [26, 26, 26, 255], weight: 700, backdrop: true },
    count: { fontPx: 64, strokePx: 4, color: [255, 255, 255, 255], strokeColor: [26, 26, 26, 255], weight: 800, backdrop: false },
  };

  /// 解析 `process` 写回的样式串：`style|font_px|stroke_px|r,g,b,a|weight|backdrop`。
  ///
  /// ⚠️ **认不出就退回兜底并按 `style` 名取那一档** —— 不抛错：
  /// 这个串是**本仓自己写的**，它坏了属于"内部不一致"，
  /// 而"因为一个解析失败整条出片跑不出来"的代价远大于"这条字用了默认样式"。
  /// ⚠️ 退回时**必须能被看见**：调用方据返回值里的 `parsed` 打一行 note。
  function parseTextStyle(source, styleName) {
    const fallback = TEXT_STYLE_FALLBACK[styleName] ?? TEXT_STYLE_FALLBACK.comment;
    if (typeof source !== 'string' || source.length === 0) {
      return { ...fallback, name: styleName || 'comment', backdrop: !!fallback.backdrop, parsed: false };
    }
    const parts = source.split('|');
    if (parts.length !== 7) {
      return { ...fallback, name: styleName || 'comment', backdrop: !!fallback.backdrop, parsed: false };
    }
    const [name, fontPx, strokePx, colorCsv, strokeCsv, weight, backdrop] = parts;
    const parseRgba = (csv) => csv.split(',').map((v) => Number.parseInt(v, 10));
    const rgba = parseRgba(colorCsv);
    const strokeRgba = parseRgba(strokeCsv);
    const nums = [fontPx, strokePx, weight, backdrop].map((v) => Number.parseFloat(v));
    if (
      rgba.length !== 4 ||
      strokeRgba.length !== 4 ||
      rgba.some((v) => !Number.isFinite(v)) ||
      strokeRgba.some((v) => !Number.isFinite(v)) ||
      nums.some((v) => !Number.isFinite(v))
    ) {
      return { ...fallback, name: styleName || 'comment', backdrop: !!fallback.backdrop, parsed: false };
    }
    return {
      name,
      fontPx: nums[0],
      strokePx: nums[1],
      color: rgba,
      strokeColor: strokeRgba,
      weight: nums[2],
      // `backdrop` 写的是 `0`/`1`（Rust 的 `bool as u8`）。
      backdrop: nums[3] !== 0,
      parsed: true,
    };
  }
  // ---------------------------------------------------------------------
  // §5 E4a —— **T2 打字机**（文字元素动效里唯一本仓自足的那一档）。
  //
  // ⚠️ 与 Rust 侧 `render/elements.rs` 是**同一套口径的两份实现**：
  //    Rust 那份负责"能不能开 / 参数落盘"，这份负责"真的把 cue 拆开"。
  //    ⚠️ 两边必须逐值一致（既有 `checkElementsParity` 钉着，它拿同一组输入
  //    跑 Rust 的常数与这里的常数对拍）。
  //
  // ⚠️ **不新增字段、也不改 `Event`** —— 配置读的是 `[elements]` 段
  //    （与 §11 BGM 同一套做法：新能力一律"默认关 + 显式开"）。
  // ---------------------------------------------------------------------

  /// 读 `[elements]` 段，**带兜底**：认不出的值退回默认，绝不抛错。
  ///
  /// ⚠️ 这个段是**用户手写的** ⇒ 类型写错（`typewriter = "yes"`）时
  /// `=== true` 自然为 false（= 关着），那是**安全方向**；
  /// 而速度/尾巴是数字，脏值必须在这里就挡掉（下面除法会炸）。
  const readElementsCfg = (config) => {
    const raw = config?.elements ?? {};
    const num = (v, fallback) =>
      typeof v === 'number' && Number.isFinite(v) ? v : fallback;
    /// ⚠️ **可选数字**：`undefined` / 脏值 ⇒ `undefined`（= 不写这个键 = 底座默认）。
    ///
    /// ⚠️ 与 `num` 的差别是**故意的**：`num` 是"兜底成默认值再往下用"，
    /// 这里是"没写就**不写进底座工程**"。两者混用会让"用户没配"与
    /// "用户配了等于默认的数"变成同一件事 —— 那正是 R4 要防的
    /// （底座那边 6 个键都是 `Option`，"不写"与"写默认值"是**两种**状态）。
    const optNum = (v) => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);
    const speed = num(raw.chars_per_sec, 14.0);
    const hold = num(raw.hold_secs, 0.6);
    // ⚠️ **T6 底衬颜色**：四元组、每项 0..255 且是整数。缺一项 / 越界 / 非数
    //    ⇒ 整个当"没配"（底座那边是 `[u8;4]`，写半截进去会让**整份工程**反序列化失败）。
    const optColor = (v) => {
      if (!Array.isArray(v) || v.length !== 4) return undefined;
      if (!v.every((c) => Number.isInteger(c) && c >= 0 && c <= 255)) return undefined;
      return v;
    };
    return {
      // ⚠️ 只有**真的**是 `true` 才算开（`"yes"` / `1` 一律当关 —— 保守）。
      typewriter: raw.typewriter === true,
      // 与 Rust `safe_chars_per_sec()` 同一套夹取：非正数一律退默认。
      chars_per_sec: speed > 0 ? speed : 14.0,
      hold_secs: hold >= 0 ? hold : 0.6,
      // ── T1 弹入 / T4 抖动 / T6 底衬（2026-10-11）──
      // 口径与字段名见底座 `docs/text-motion-integration.md` 与
      // `docs/plan/t6-plate-interface.md`。**全部 `undefined` = 不写键**。
      scale_in_from: optNum(raw.scale_in_from),
      scale_in_peak: optNum(raw.scale_in_peak),
      // ⚠️ 毫秒是整数。`u64` 收到小数会让**整份工程**反序列化失败（serde 不隐式取整）。
      scale_in_ms: Number.isInteger(raw.scale_in_ms) && raw.scale_in_ms >= 0
        ? raw.scale_in_ms
        : undefined,
      shake_px: optNum(raw.shake_px),
      shake_hz: optNum(raw.shake_hz),
      plate_color: optColor(raw.plate_color),
      plate_pad_px: optNum(raw.plate_pad_px),
    };
  };

  /// 把 T1/T4/T6 那几个**配了的**字段挂到字幕轨的 `subtitle` 上。
  ///
  /// ⚠️ **不配就不写键**（不是"写默认值"）—— 底座那边六个键全是 `Option`，
  /// 「没写」与「写了等于默认的数」是**两种**状态，混起来会让老工程多出键
  /// （R4 红线：默认全关时底座工程**逐字节相同**）。
  ///
  /// ⚠️ **`undefined` 一律不写**：`{ a: undefined }` 经 `JSON.stringify` 本来就
  /// 会消失，但这里显式判一次 —— 免得将来有人换成别的序列化器时**静默多出 `null`**。
  /// （底座那边 `shake_px: null` 与"没写"**都是**关，但 `plate_color: null` 是
  /// "显式不铺"，语义不同 ⇒ 不能靠 `undefined` 悄悄变 `null`。）
  ///
  /// `phase` 是**这一轨**的抖动相位（见 `SHAKE_PHASE_BY_STYLE`）。
  const motionFields = (cfg, phase) => {
    const out = {};
    const put = (key, v) => {
      if (v !== undefined) out[key] = v;
    };
    put('scale_in_from', cfg.scale_in_from);
    put('scale_in_peak', cfg.scale_in_peak);
    put('scale_in_ms', cfg.scale_in_ms);
    put('shake_px', cfg.shake_px);
    put('shake_hz', cfg.shake_hz);
    // ⚠️ **相位按档错开，但 0 那一档不写**（0 就是底座默认值 ⇒ 写了是 R4 违规：
    //    "没写"与"写了默认值"在老工程上必须是**同一份字节**）。
    //    这一条与上面五个不同：它的值**取决于轨身份**，不是纯配置。
    if (phase !== 0) put('shake_phase', phase);
    put('plate_color', cfg.plate_color);
    put('plate_pad_px', cfg.plate_pad_px);
    return out;
  };

  /// 抖动相位**按 style 档错开**（§5 E4a T4，2026-10-11）。
  ///
  /// # ⚠️⚠️ 这是**轨级**相位，不是逐条 —— 底座的契约边界就是这样
  ///
  /// 底座的 `shake_phase` 住在 `SubtitleStyle` 上（**轨级**），而
  /// `overlay.rs` 的 cue 循环里读的是那个 `style` ⇒ **同一轨的所有 cue 相位必然相同**。
  /// 底座在 `overlay.rs:359` 自己写着：
  /// > "相位默认给 0；'同屏多条要错开'由宿主显式写 `shake_phase` 决定
  /// >  （**契约不给自动错开，那要靠条目身份，而契约层没有那个身份**）"
  ///
  /// ⇒ 底座是**有意**把这件事交回宿主的，宿主的能力边界就是轨级。
  ///
  /// ★ **所以这条局限必须记住：同一条轨里有多条 cue 时，它们同拍。**
  ///   本仓按 style 分 4 轨、文字元素**通常**一档一条，但**不保证**；
  ///   同一档出现两次（两段 hook）就同拍。这不是 bug，是契约边界。
  ///   要真正逐条错开，得回底座加 `Cue.shake_phase` ——
  ///   ⚠️ 那件事与 T3（`Cue.scale`）**是同一个形状**，应该**一起做**，别单独为 T4 做。
  ///
  /// 间隔取 1.9：实测"把相位挪 1.9 之后抖动轨迹明显不同"的那个量级。
  /// 0 那一档**不写键**（0 就是底座默认值，写出来是 R4 违规）。
  const SHAKE_PHASE_BY_STYLE = { hook: 0, chapter: 1.9, comment: 3.8, count: 5.7 };

  /// 把一条 cue 拆成"逐字出现"的多条（T2）。
  ///
  /// ⚠️ **纯函数、无累积状态** —— 规格 §4.1 的硬约束：`seek` 到第 N 帧
  /// 与"播到第 N 帧"必须长得一样，否则预览与导出分叉。
  /// ⚠️ 判据与 Rust 那份逐条对齐（见 `elements.rs` 的同名函数）：
  ///   关着/一个字/脏区间/脏帧率 ⇒ 原样一条；**不许产出 0 帧的 cue**
  ///   （0 帧在底座那边一帧都不显示 ⇒ 那个字永远不会出现）。
  /// ⚠️ **这里不再判 `cfg.typewriter`** —— 调用方（`if (tw)`）已经判过了。
  /// 两处都判会让"关着开关"这件事有**两个**执行点：
  /// 变异台改外层、内层照样短接 ⇒ 变异**改不动行为** ⇒ 那条变异永远"存活"
  /// （实测：`const tw = true` 与 `if (false) return [cue]` 两处都抓不住）。
  /// ★ 与 §9 那次同一个形状：**一道门只留一个执行点**，否则测的是另一个。
  function splitTypewriter(cue, fps, cfg) {
    const chars = Array.from(cue.text);
    if (chars.length <= 1 || cue.end <= cue.start || !(fps > 0) || !Number.isFinite(fps)) {
      return [cue];
    }
    const span = cue.end - cue.start;
    const perCharSecs = 1 / cfg.chars_per_sec;
    // 每字至少 1 帧：帧不够时**宁可少打几个字**，也不产出 0 帧 cue。
    const maxSteps = Math.max(1, Math.floor(span));
    const wanted = Math.floor(span / (perCharSecs * fps));
    const typingFrames = Math.min(Math.max(1, wanted), maxSteps);

    const out = [];
    let shown = 0;
    let frame = cue.start;
    const holdFrames = Math.round(cfg.hold_secs * fps);

    for (let i = 0; i < typingFrames; i += 1) {
      const target = Math.min(
        chars.length,
        Math.max(1, Math.ceil(((i + 1) / typingFrames) * chars.length)),
      );
      let next = cue.start + Math.round(((i + 1) * span) / typingFrames);
      next = Math.min(cue.end, Math.max(next, frame + 1));
      out.push({ start: frame, end: next, text: chars.slice(0, target).join('') });
      shown = target;
      frame = next;
      if (frame >= cue.end) break;
    }

    if (shown < chars.length || frame < cue.end) {
      const tailStart = Math.max(0, cue.end - holdFrames);
      if (tailStart > frame) {
        out.push({
          start: frame,
          end: tailStart,
          text: chars.slice(0, Math.max(1, shown)).join(''),
        });
        frame = tailStart;
      }
      if (frame < cue.end) {
        const full = chars.join('');
        const last = out[out.length - 1];
        // 与最后一条文本相同时**合并**（不产出内容等价的两条 cue）。
        if (last && last.text === full && last.end === frame) {
          last.end = cue.end;
        } else {
          out.push({ start: frame, end: cue.end, text: full });
        }
      }
    }
    return out.length > 0 ? out : [cue];
  }

  const stickerLayers = [];
  /// **调整图层**：走「调整级」管线的特效（Warp / ColorAdjust / ColorMask）
  /// 必须挂在**没有素材**的图层上，否则渲染器整条丢掉（见下面的 `placeAdjustment`）。
  const adjustmentEffects = [];
  const stickerAssets = new Map();
  /// 贴纸素材的尺寸（算缩放用）。assetId -> {frames,width,height}
  const stickerSizes = new Map();
  const stickerPkg = loadStickerPackage(clipDir);
  let stickerCount = 0;
  let stickerAnimCount = 0;
  let cameraCount = 0;
  let shakeCount = 0;
  /// 运镜事件先收集起来，循环之后再统一处理（理由见循环之后那段）。
  const cameraEvents = [];
  /// 独立 `shake` 事件也先收集（与手持摇摆同一趟按帧采样，见循环之后那段）。
  const shakeWindows = [];
  /// **运镜那一趟写了稀疏关键帧的输出帧号。**
  ///
  /// 逐帧采样的那一趟要用它算"往前铺到哪" —— 见那里 `denseRanges` 的说明。
  const sparseOutFrames = new Set();
  /// 同样收进"视频层逐帧"那一趟的：`stutter`（位移）与 `zoom_bounce`/`pulse`（缩放乘数）。
  const stutterWindows = [];
  const zoomBounceWindows = [];
  const pulseWindows = [];

  if (stickerPkg) {
    note(
      'assets/stickers',
      `贴纸包读到 ${stickerPkg.tags.size} 个 tag（${stickerPkg.source}）` +
        (stickerPkg.borrowed ? '—— **借自兄弟切片目录**，本切片里没有 toml' : ''),
    );
  } else {
    note('assets/stickers', '没找到贴纸包 TOML，贴纸只能按文件名直接找');
  }

  for (const [segmentIndex, segment] of (config.segments ?? []).entries()) {
    const offset = segment.start ?? 0;
    for (const event of segment.events ?? []) {
      const where = `segments[${segmentIndex}].events(${event.type})`;
      const at = typeof event.time === 'number' ? event.time : offset;

      // 事件的**源帧号**（YekiTrim 的时间）与**输出帧号**（裁剪后的时间线）。
      // 落在被掐掉的段里的输出帧号是 null —— 那一段在成片里不存在。
      const srcStart = frameAt(at);
      const srcEnd = frameAt(typeof event.end === 'number' ? event.end : at);

      // **轨道眼睛**（见上面 `laneAllows`）：只跳过这一类事件的转译，
      // 事件本身保留（点亮眼睛即恢复）。⚠️ `continue` —— `break` 会跳出整个事件循环。
      if (!laneAllows(event.type)) {
        note(
          where,
          `轨道 \`${LANE_OF_TYPE[event.type]}\` 的眼睛关着（\`[stage.lanes]\`）：这一类整条不渲染` +
            '（事件保留，界面点亮即恢复）',
        );
        continue;
      }

      // §5：**元素（`element`）在进 switch 之前先归一化**。
      //
      // ⚠️ 为什么要在这里做，而不是在 `case 'element'` 里做：
      //    贴纸路径（`case 'sticker'`）里有 200 多行 —— 素材解析（`name` → 文件 →
      //    探测帧数与尺寸 → 登记资产 → 算盒子与关键帧）与**三条已修过的历史坑**
      //    （先判 `tag` 会整条丢、帧数必须真的探、逐帧延迟表只在非匀速时写）。
      //    在 `case` 里"再走一遍 switch"要绕一圈、还会踩到 `break`/`continue` 语义；
      //    在这里把形状换掉**最省事也最不容易错** —— 下面那条 case 看到的
      //    就是一条普普通通的 sticker 事件。
      //
      // ⚠️ 只对 `kind === "sticker"` 做归一化：`kind === "text"` 要留给下面的 case 报
      //    "还没有排版通道"（**必须报到报告里**，不许静默跳过 —— R1）。
      if (event.type === 'element' && String(event.kind ?? '').trim().toLowerCase() === 'sticker') {
        // 关键词在 `text` 里（§5 E1 的字段口径），搬到 `keyword`；
        // 元素没有 `sfx` 字段（规格 §2 的结构里没有）⇒ 别让残留值漏过去。
        event.type = 'sticker';
        event.keyword = event.text ?? '';
        delete event.sfx;
      }

      switch (event.type) {
        case 'camera': {
          // 运镜整条被眼睛关掉时**根本走不到这里**（上面 `laneAllows` 就 `continue` 了）——
          // 不是"丢掉"，是参照侧读同一份 `lanes` 也不画，跳过才是**对齐**。
          // 运镜 = Transform 上的关键帧。**这是底座已有的能力**（T8），
          // 所以这一条是"转过去"而不是"丢掉"。
          const factor = zoomFactor(event.zoom, config.style?.camera, where);

          // 起点的**不动点**（focus）。YekiTrim 的运镜是「以不动点为中心缩放」，
          // **不是**「把不动点搬到画面中心」—— 这两个完全不一样，见下面的推导。
          //
          // **优先用事件自己的 `origin`**：YekiTrim 每条 camera 都带这个字段，
          // 而且它不一定等于 `layout.liver` 的中心 —— 实测样本 B 里就有一条
          // `origin = "50.00% 50.00%" anchor="center"`（画中画拉远那一档），
          // 用 liver 中心会算错。只用"事件没给 origin"时才退回 liver 中心。
          //
          // ⚠️ **不再"算不出来就把整条运镜丢掉"**（2026-09-30）：模板的式子是
          // `oa = (c.ev.origin || defOrigin)`、`ob = (c.ev.to_origin || c.ev.origin || defOrigin)` ——
          // **它永远有一个兜底**。以前这里在 `anchor_to="danmaku"` 但没写 `to_origin` 时整条 DROP，
          // 于是"参照在缩放、成片不动"（那正是"画面不对"最常见的一种）。
          const ownOrigin = point(event.origin ?? '');
          const anchorPoint = event.anchor_to === 'danmaku'
            ? (point(event.to_origin ?? '') ?? ownOrigin ?? focus)
            : (ownOrigin ?? focus);
          if (!anchorPoint) {
            // 走到这里只剩一种可能：`focus` 也没了（不可能 —— 它现在是**恒非空**的兜底）。
            drop(where, `锚点算不出来（anchor=${event.anchor ?? '?'}），这一条运镜跳过`);
            break;
          }
          if (ownOrigin && focus && (Math.abs(ownOrigin.x - focus.x) > 1e-6 || Math.abs(ownOrigin.y - focus.y) > 1e-6)) {
            note(
              where,
              `这一条用了事件自己的 origin（${event.origin}），` +
                `与 layout.liver 中心（${(focus.x * 100).toFixed(2)}% ${(focus.y * 100).toFixed(2)}%）不同`,
            );
          }

          // # 偏移公式（**从 YekiTrim 源码推出来的**，不是猜的）
          //
          // YekiTrim `render-core/draw.rs:274 camera_transform()` 里写死了：
          //
          //     cam_off_x = s * (1.0 - z) * camera.ox
          //     offx      = ... + cam_off_x
          //     l00       = s * z
          //
          // 换成像素空间就是：**`p_out = z * (p_in - origin) + origin`**
          // —— 不动点在 `origin`，缩放围绕它进行。它自己的测试
          // `wide_zoom_shrinks_around_origin` 说的就是"以不动点为中心缩小，
          // 四周留黑边"。
          //
          // 底座渲染器的几何（`render/compose.rs:169`）：
          //
          //     p_out = (p_src - c_src) * scale + c_out + offset
          //
          // 源尺寸 == 画布尺寸时 `c_src == c_out == 中心`，于是
          //
          //     p_out = z * p_src + (1 - z) * 中心 + offset
          //
          // 两式对照（`z*1` 项相同），令 `p_src` 项之后的常数相等：
          //
          //     offset = (1 - z) * (origin - 中心)
          //
          // **我前两版都错了**：
          //   v1：`(0.5 - focus) * W * z`      —— 多乘一个 z，且符号反了
          //   v2：夹住偏移让它"不出空带"      —— 掩掉了症状，画面仍然不对
          //
          // 错在这里的表现是**运镜整体偏移、且幅度不对**：
          // `65.02%` 处、`z=1.3` 时正确偏移是 `-86.5px`，
          // 而 v1 给的是 `-374.9px`（差了 4.33 倍 = `z/(1-z)`）。
          //
          // **不需要夹。** `z > 1` 时以任何内点为中心放大，画面都仍然铺满
          // （四边各向外扩 `(z-1)` 倍）；`z < 1` 时四周留黑边 ——
          // 而那正是 YekiTrim 的行为（source pass 清屏为黑，见它的
          // `wide_zoom_shrinks_around_origin`，z=0.55 时四下各 432px 黑边）。
          // **只收集，不落关键帧**：过渡要按 YekiTrim 的状态机算
          // （`td = min(duration*0.5, 0.6)`、远景硬切），见循环之后那一段。
          cameraEvents.push({
            srcStart,
            srcEnd,
            z: factor,
            origin: anchorPoint,
            originTo: point(event.to_origin ?? '') ?? anchorPoint,
            cut: event.cut === true,
            zoom: event.zoom,
            handheld: event.handheld === true,
            where,
            // 运镜速度（规格 `camera-and-noise-spec.md` §1）：`snap|fast|med|slow`。
            // 缺省（没写）= 老式子，见 `cameraTransitionFrames`。
            speed: event.speed,
            // 荷兰角（规格 §4）：整帧绕**聚焦中心**旋转的度数，正 = 顺时针。
            // ⚠️ 这里是**持续状态**，与 `zoom` 同一套状态机（跟着到下一条运镜）——
            //    做成"瞬时特效"的话"到点就回正"会变成**闪一下**（规格 §4.5 原话）。
            // ⚠️ `process::cap_dutch_angle` 已经夹过 ±6°、限过配额、并把**与画中画同段**的
            //    剔除掉了（几何原因：`远` 档最大安全角是 0.00°）。这里不再重复判。
            tilt: typeof event.tilt === 'number' ? event.tilt : 0,
          });
          cameraCount += 1;
          break;
        }

        case 'shake': {
          // YekiTrim 的 `shake` 是一个**短促的画面抖动**（爆点那一刻），
          // 形状是 `{ time, end, intensity, sfx? }` —— 注意
          // **跟 `camera` 里的 `handheld=true` 不是一回事**：
          //   - `handheld` 是"整段镜头轻微晃"（持续、幅度小）
          //   - 独立 `shake` 事件是"这一下猛地一晃"（短、幅度大）
          // 底座两者都落在 `shake` 特效上，区别只在**时间窗**与**幅度**。
          const startFrame = remapFrame(srcStart);
          if (startFrame === null) {
            note(where, '这一条整段落在被掐掉的区间里，跳过（成片里没有那一段）');
            break;
          }
          const endFrame = remapFrame(srcEnd) ?? startFrame + 1;
          const length = Math.max(1, endFrame - startFrame);
          // **`intensity` 是像素。** YekiTrim 的 `shake()` 把它**直接**当 CSS 位移用：
          //
          //     tl.to('#video-wrap', { x: off, y: -off, duration: 0.025 }, t);
          //     tl.to('#video-wrap', { x: -off, y:  off, duration: 0.025 }, t + 0.025);
          //
          // 所以 0.4 就是 **0.4 像素**。而底座 `shake.amount` 是**归一化位移比例**
          // （`warp.wgsl`：`amount 是位移幅度（归一化）`），所以换算就是除以目标高。
          //
          // 我先前写的是 `intensity * 0.05` —— 在 1080p 下是 **21.6 像素，
          // 比参照大 54 倍**。参数本身是泛用的（归一化幅度对任何目标尺寸都成立），
          // 错的是这条换算。
          const intensity = typeof event.intensity === 'number' ? event.intensity : 14;
          // **收集起来，等运镜循环跑完再一起按帧采样**（与手持摇摆同一趟）。
          //
          // # 我先前抄错了哪一份
          //
          // YekiTrim 有**两份** `shake`，形状完全不同：
          //
          //   `index.html:532`  `shake(tl,…)`        —— **GSAP DOM 预览**用的：
          //       `n = dur/0.05` 个 0.025s 的交替偏移，幅度渐变
          //   `index.html:1575` `getShake(t, items)` —— **canvas 出片**用的：
          //       **固定 0.2s 的四级阶梯**，与事件 duration 无关
          //
          // 我先前抄了预览那一份、做成了连续波形。**出片要的是下面这条阶梯**：
          //
          //     dt < 0.04  -> ( off, -off)
          //     dt < 0.08  -> (-off,  off)
          //     dt < 0.12  -> ( off/2, -off/2)
          //     dt < 0.16  -> (-off/2,  off/2)
          //     否则        -> (0, 0)
          //
          // 而且它作用在**视频层**上（`renderFrame` 里 `ctx.translate(shake.x, shake.y)`
          // 在 video 那段 `ctx.save()` 之内）—— 字幕/弹幕/贴纸**不跟着抖**。
          // 所以这里不再是"调整图层"，而是视频层的 x/y 位移。
          shakeWindows.push({ outStart: startFrame, intensity, where });
          note(
            where,
            `shake intensity=${intensity}（**像素**，YekiTrim 直接当 CSS 平移用）-> ` +
              '按出片那份 `getShake` 的四级阶梯逐帧写进**视频层**的 x/y',
          );
          // # 亚像素告警（2026-10-05 加，R1 的"让存量工程可见"那一半）
          //
          // `intensity` 的单位是**像素**（上面三段论证 + 本文件 `|| 14` 的兜底都是按像素），
          // 但 **LLM 提示词的唯一示例写的是 `0.5`**（`crates/yekitrim-polish/src/llm/segment.rs:96`，
          // 全篇没有单位），编辑器默认又是 `10`（`TimelineEditor.vue:1288`）
          // ⇒ 存量工程里 LLM 生成的 shake 常落在 `0.3~0.9`：**亚像素、预览与成片都看不出来**，
          // 而报告照样写 `shake_events: 1`（用户报的正是"shake 没正常触发"）。
          //
          // 这**不是**"转丢了"（那该报 DROP），所以只报一条 NOTE 让人自己去改数值：
          // 转换本身没错、也没法在这里猜"他本来想要几个像素"。
          // ⚠️ 判据用**开区间** `(0, 2)`：`0` 是合法值（"这一下不抖"）别报；
          // 负数是坏数据，留给校验层，不在这儿替它编语义。
          if (intensity > 0 && intensity < 2) {
            note(
              where,
              `shake intensity=${intensity} 落在 (0, 2) —— **疑似单位错误**：` +
                '这个字段是**像素**（默认 14），`0.3~0.9` 是照 LLM 提示词那个 `0.5` 的示例填出来的' +
                '**亚像素**值，预览与成片都看不出来（见 `docs/plan/polish-preview-bugs-2026-10-05.md` R1）',
            );
          }
          shakeCount += 1;
          if (event.sfx) {
            note(where, `shake 自带的音效「${event.sfx}」：底座没有"音效驱动画面"，两个子系统之间没有连线`);
          }
          break;
        }

        case 'sticker': {
          // 贴纸 = **图片素材 + 落点**。素材是贴纸包里的图（底座 T12 的
          // `AssetKind::ImageSequence`），落点是 `polish.toml` 里的百分比。
          stickerCount += 1;
          const tag = event.sticker_tag ?? '';

          // **优先用 `name`** —— YekiTrim 的 `Step2b` 跑过之后，
          // `name`/`source` 就已经是**选定好的文件名**了，那是最准的信息。
          // 只有它是空的（`Step2b` 没跑）才退回按 tag 猜。
          //
          // ⚠️ 2026-09-30（M2）修：以前**先**判 `sticker_tag` 空就整条 DROP ——
          // 而后处理（`Step2b`）跑过的工程里 `sticker_tag` 常常是空的、`name` 却是齐的，
          // 于是"选好的贴纸"整条不进成片，报告里只有一行"没有 sticker_tag，挑不出文件"。
          // 现在的顺序与注释一致：先看 `name`，再退 tag，两者都没有才报。
          let fileName = null;
          let how = '';
          if (event.name) {
            fileName = `${event.name}.gif`;
            how = `YekiTrim 选定的 name（source=${event.source ?? '?'}）`;
          } else if (tag) {
            const picked = pickSticker(stickerPkg, tag);
            if (picked) {
              fileName = picked.files[0];
              how = picked.how;
            }
          }
          if (!fileName) {
            drop(
              where,
              `贴纸：既没有 name、也没有能对应的 sticker_tag（tag=${tag || '(空)'} keyword=${event.keyword ?? '?'}）`,
            );
            break;
          }

          // **按三种目录布局找真文件**（平铺 / 带包目录 / 任意子目录）。
          // ⚠️ 扩展名：贴纸库里允许 `gif/png/jpg/jpeg/webp`（见 render 卡），
          // 而 `name` 是不带扩展名的 —— 只试 `.gif` 会让 PNG 贴纸"找不到"（本仓那边也踩过同一个坑）。
          const found = findStickerFile(clipDir, fileName) ?? findStickerAnyExt(clipDir, event.name ?? fileName);
          if (!found) {
            drop(where, `贴纸「${tag || event.name}」-> ${fileName}：assets/stickers/ 下（含子目录）找不到`);
            break;
          }
          fileName = found.uri.split('/').pop();

          // 素材登记（同一文件只登一次）。
          const assetId = `sticker/${fileName}`;
          if (!stickerAssets.has(assetId)) {
            stickerAssets.set(assetId, true);
            // **帧数必须是真的。** 先写 1 当占位，被校验当场拦下
            // （`source_range_exceeded`：素材只有 1 帧、却要取 224 帧）。
            // 所以这里探一次，探不到就按 1（并用 note 说清）。
            const probed = probeImage(found.path);
            const frames = probed.frames;
            // **逐帧延迟表：只在非匀速时写。**
            //
            // 匀速（本样例四张贴纸全是 `[30,30,…]`）时不写 —— 老行为逐字节不变，
            // 工程也不会白白多一张表。非匀速才写，并如实记一条 note。
            const delays = probeFrameDelays(found.path);
            const uniform =
              delays === null || delays.every((value) => value === delays[0]);
            const delaysForAsset = uniform ? undefined : delays;
            // 记下尺寸，下面算贴纸缩放要用。
            stickerSizes.set(assetId, probed);
            assets.push({
              id: assetId,
              kind: 'image_sequence',
              name: fileName.replace(/\.gif$/i, ''),
              uri: found.uri,
              frame_count: frames ?? 1,
              // GIF 的帧间隔不均匀，底座的 `timebase` 只能给一个标称值。
              // 这是**动图在原生侧还没打通**的直接原因（见设计文档 §11.1），
              // 这里按 10fps 标称登记。
              //
              // **用 GIF 自己的帧率，不是写死的 10fps。**
              // 实测这些贴纸全是 `100/3`（33.3fps），我先前写死 10fps
              // —— 于是动图**慢 3.33 倍**，而"贴纸在动"这件事看起来是正常的，
              // 只有逐像素比才看得出来（实测 4.5s 差 -1.09、48.8s 差 -1.37）。
              timebase: probed.fps ?? { num: 10, den: 1 },
              // 非匀速动图才带（`undefined` 会被 JSON.stringify 丢掉，等于老行为）。
              frame_delays_ms: delaysForAsset,
              width: probed.width,
              height: probed.height,
              content_hash: null,
              tags: {},
              note: `贴纸（${how}）`,
            });
            if (frames === null) {
              clamp(where, `${fileName} 的帧数探不到，按 1 帧算（会当成静态图）`);
            }
            if (delaysForAsset) {
              note(
                where,
                `贴纸「${tag}」(${fileName}) 的帧间隔**不均匀**（${delaysForAsset.slice(0, 6).join('/')}… ms，` +
                  `共 ${delaysForAsset.length} 帧）—— 已写进 frame_delays_ms，底座按延迟累加落帧`,
              );
            } else if (delays === null) {
              note(
                where,
                `贴纸「${tag}」(${fileName}) 的逐帧时长探不到，按匀速（素材帧率）处理 —— ` +
                  '若它其实是非匀速动图，动图会随时间漂',
              );
            }
            if (probed.width === null || probed.height === null) {
              clamp(where, `${fileName} 的像素尺寸探不到，贴纸按原始大小画（盒子对不齐）`);
            }
          }

          const startFrame = remapFrame(srcStart);
          if (startFrame === null) {
            note(where, '这一条整段落在被掐掉的区间里，跳过（成片里没有那一段）');
            break;
          }
          const endFrame = remapFrame(srcEnd) ?? startFrame + 1;
          // 落点：两种模式，**基准不同、不许"统一"**
          // （与 `templates/index.html::stickerAnchor`、`engine.js::stickerAnchor` 逐字同口径）：
          //
          //   `auto`   = 用事件自带的 `x`/`y`（`process/placement.rs` 安全区算法算好写进 segments），
          //                含义是**贴纸盒子的左上角**（YekiTrim `process/mod.rs:276`）
          //   `manual` = 全部固定到 `[sticker] position` 那一点，**忽略事件的 `x`/`y`**，
          //                含义是**盒子中心**（`config.rs` 的 `StickerConfig.position` / 设置页口径）
          //
          // ⚠️ **2026-10-02 修**：以前这里**只读 `event.x/y`**，整个 `case` 里没有 `manual` 分支 ——
          // 于是"设计器里把落点改成固定、点保存"**在那条腿上完全丢失**：改的是
          // `[sticker] position`，而这里读的是 auto 落点算出来的 `ev.x/y`。
          // 症状极具迷惑性：**预览（DOM 那条腿）当场就对**（engine.js 有这个分支），
          // 只有底座出的片子里贴纸还在老位置，报告里一行差异都没有。
          // 而 `process` 每次都把 `ev.x/y` 重算一遍，所以那个旧值还会一直"看起来很新"。
          //
          // 盒子边长按 YekiTrim 的规则算：`280 * min(W,H)/1080`。
          // 素材是 500×500 的原图，要塞进这个盒子，所以 `scale = 盒子/素材`。
          const stickerMode = config.sticker?.position_mode === 'manual' ? 'manual' : 'auto';
          let anchor; // { x, y, center } —— center = 这个点是盒子中心（manual）还是左上角（auto）
          if (stickerMode === 'manual') {
            // `[sticker] position` 形如 `"35% 50%"`（也接受单值 = 两轴同值），缺省 `"50% 50%"`。
            // 解析口径照抄那两份参照：按空白拆、`parseFloat`、非有限数回落 50。
            const parts = String(config.sticker?.position || '50% 50%').trim().split(/\s+/);
            const mx = parseFloat(parts[0]);
            const my = parseFloat(parts.length > 1 ? parts[1] : parts[0]);
            anchor = {
              x: Number.isFinite(mx) ? mx : 50,
              y: Number.isFinite(my) ? my : 50,
              center: true,
            };
          } else {
            const ex = parseFloat(String(event.x ?? '').replace(/%$/, ''));
            const ey = parseFloat(String(event.y ?? '').replace(/%$/, ''));
            anchor = {
              x: Number.isFinite(ex) ? ex : 50,
              y: Number.isFinite(ey) ? ey : 50,
              center: false,
            };
          }
          // manual 模式**永远有落点**（`[sticker] position` 自带缺省值）⇒ 不走下面那条 DROP。
          // 只有 auto 才需要"事件里有没有 x/y"这一关 —— 而且判据必须是 `ratio()` 的结果，
          // **不能写 `event.x !== undefined`**：没跑过后处理的工程里那个键是**存在**的，
          // 值恰好是字符串 `"auto"` ⇒ 旧判据会当"有落点"，然后拿 `NaN` 去算位置
          // （贴纸安静地贴到错的地方，报告里一行差异都没有）。
          const hasPos = stickerMode === 'manual' || (ratio(event.x) !== null && ratio(event.y) !== null);
          if (!hasPos) {
            // ⚠️ 这句要给**下一步怎么做**（2026-09-30 M2 改）：落点是 `process` 的产物
            // （`yekitrim-polish/src/process/mod.rs:276` 按 人物/字幕/弹幕 推导安全区后写死百分比），
            // 所以"没有落点"的正确处理是**先跑一次后处理**，而不是在这里猜一个位置。
            // 底座确实没有安全区推导 —— 这句话要留着，但要说清"不是底座的缺陷，是这一步没跑"。
            drop(
              where,
              `贴纸落点 "${event.x ?? '(无)'}","${event.y ?? '(无)'}" 解析不了 —— ` +
                '落点是**后处理的产物**（`process` 按人物/字幕/弹幕推导安全区后写死百分比）：' +
                '先跑一次 `yekitrim polish process`（编辑器里随便改一下自动跑、或点「渲染」时也会先跑），' +
                '再转译。底座没有安全区推导，这里**不猜**一个位置。',
            );
            break;
          }

          const box = STICKER_BOX_BASE * (Math.min(width, height) / 1080);
          const size = stickerSizes.get(assetId) ?? {};
          const sourceW = size.width ?? box;
          const sourceH = size.height ?? box;
          // 按"装进正方形盒子"算缩放：取长边，保证不溢出盒子。
          const fit = box / Math.max(sourceW, sourceH);
          // 锚点百分比 → **盒子左上角**（像素），再折算成底座 `Transform` 的中心基准
          // （底座 origin 是图层中心，所以要挪半个盒子）。
          //   manual：锚点**本来就是中心** ⇒ 不能再挪那半个盒子
          //   auto  ：锚点是左上角 ⇒ 要挪
          const anchorPxX = (anchor.x / 100) * width;
          const anchorPxY = (anchor.y / 100) * height;
          // ⚠️ **钳进安全区**（2026-10-05 修）：参照 `index.html:976-983 stickerBox` 与
          // `engine.js::clampSticker` 都会把盒子左上角钳进 `[M, 边长-box-M]`（`M = 32*SK`），
          // 而这条腿原先**没有这一道** ⇒ manual 落点写在边缘时（`"98% 50%"`）两条预览都显示钳过的
          // 位置、成片却把贴纸甩到框外（实测 1920 宽：参照 1608 vs 这里 1741.6，出框 101.6px）
          // —— 正是"预览与出片不一致"那一类，而且不报错。
          const M = 32 * (Math.min(width, height) / 1080); // 与参照 `32*SK` 同一个量
          const leftTopX = Math.max(M, Math.min(width - box - M,
            anchor.center ? anchorPxX - box / 2 : anchorPxX));
          const leftTopY = Math.max(M, Math.min(height - box - M,
            anchor.center ? anchorPxY - box / 2 : anchorPxY));
          const baseX = leftTopX - width / 2 + box / 2;
          const baseY = leftTopY - height / 2 + box / 2;

          // ---- 贴纸动效：逐帧采样 `stickerStateAt`（不再是手写关键帧）----
          //
          // 见 `yekitrim-polish/templates/index.html:1517 getActiveSticker()`。
          // 相位（含第四段淡出）都写在 `stickerStateAt` 的文档里。
          //
          // `SK = min(CW,CH)/1080`（参照模板 305 行）—— 贴纸的盒子、上浮量
          // 都按**短边**缩放。1080p 下是 1。

          // # 关键帧的帧号是**相对图层起点**的，不是绝对帧号
          //
          // 契约校验写得很清楚（`schema.rs:732`）："关键帧在第 N 帧，
          // 而片段只有 M 帧（**相对片段起点**，0..=M-1）"。
          //
          // 我第一版直接写了绝对帧号（`startFrame`、`startFrame+21`），
          // 于是贴纸层的 `keyframes[0].frame = 2809` 远大于它的 duration，
          // **一个关键帧都不落在区间里** → 动画完全不生效。
          // 表现很隐蔽：工程校验**不报错**（越界检查只在 v1 的 `validate_project`
          // 里，v2 的 `validate_timeline_v2` 不查这个），渲染也不报错，
          // 只是"贴纸不动"。
          //
          // 抽帧看不出来的原因：`opacity` 的 fallback 是 `layer.opacity = 1.0`，
          // 所以**没有生效的关键帧等于"一直满不透明"** —— 从静止帧看是对的。
          // 只有对比"有贴纸 vs 无贴纸"才看得出来。
          // # 贴纸动效：**按帧采样参照的四段相位**
          //
          // 我先前是"手写几条关键帧"去逼近，而那必然漏掉口径（`back_out` 的 `s=3`、
          // 上浮是线性的、四段的相位值都是常数）。这里改成**逐帧采样**
          // `stickerStateAt`，把参照的四个相位原样搬过来 ——
          // 与手持摇摆、运镜过渡同一个办法：凡是"`t` 的纯函数"，就按帧采，
          // 别用几条关键帧去猜它。
          //
          // 窗口 = 参照的 `continue` 判据：**就是事件时长本身** `[et, et + dur]` ——
          // 淡出在窗口**内部**消化（`dur - 0.3 -> dur` 渐隐）⇒ 最后一帧正好落在出点。
          //
          // 上一刀的形状是"窗口 = 四段之和 = `dur + 1.05`、淡出接在事件之后"✗ ——
          // 于是贴纸比事件**多显示 0.4 秒**（用户实测：出点晚了）。现在不溢出一帧。
          //
          // 紧跟着的 `note(...)` 记的是**同一个事实**（窗口 = dur、出点即淡出终点），
          // 它进转译报告，所以措辞也要与上面这几行一致。
          note(
            where,
            '贴纸的淡出**移进了窗口内部**：窗口 = 事件时长 `dur`（不再溢出到事件之外）——' +
              '相位 `0→0.35` 弹入、`0.35→0.75` 上浮、`0.75→dur-0.3` 保持、`dur-0.3→dur` 渐隐' +
              ' ⇒ **出点即淡出终点**（最后一帧 opacity 已趋 0，既不硬切、也不再晚 0.4 秒）。' +
              '四处同口径：本文件的 `STICKER_WINDOW_SEC = 0`、`templates/index.html` 的 `getActiveSticker`、' +
              '以及 `yekitrim-render-core/src/timeline.rs` 与 `webui/src/polish/engine.js`' +
              '（窗口上界 = `dur`，淡出起点 = `dur - fadeOut`）。',
          );
          const durSec = Math.max(1 / fps, (srcEnd - srcStart) / fps);
          // **窗口以事件时长为准** ✓：`STICKER_WINDOW_SEC = 0`（见 `stickerStateAt` 的文档）。
          //
          // 上一刀这里写的是四段之和（= 1.05）✗ —— 逐帧采样循环于是多采 0.4 秒的关键帧，
          // 贴纸的可见区间比事件长出 0.4 秒（用户实测的出点偏晚）。
          // 现在采样的上界就是 `dur`：最后一帧（第 `windowFrames` 帧，`t = dur`）opacity = 0，
          // **出点即淡出终点** ✓（不需要另写关键帧 ✓）。
          const windowFrames = stickerWindowFrames(durSec, fps);
          const keyframes = [];
          for (let local = 0; local <= windowFrames; local += 1) {
            const st = stickerStateAt(local / fps, durSec, SK);
            keyframes.push({ frame: local, target: 'scale', value: fit * st.scale, easing: 'linear' });
            keyframes.push({ frame: local, target: 'rotation', value: st.rot, easing: 'linear' });
            keyframes.push({ frame: local, target: 'opacity', value: st.opacity, easing: 'linear' });
            keyframes.push({ frame: local, target: 'y', value: baseY + st.yOff, easing: 'linear' });
          }
          // 关键帧必须**严格小于**图层时长（校验判据是 `>= duration` 报错），
          // 所以图层比最后一个关键帧多一帧。
          const duration = windowFrames + 1;


          stickerAnimCount += 1;
          stickerLayers.push({
            id: `sticker-${stickerLayers.length}`,
            start: startFrame,
            // **比 cue 多一帧**（只为让最后那个关键帧严格小于图层时长）：
            // 淡出在 `end` **内部**走完 —— 窗口 = `[time, end]`，见模板 1527 行的判据。
            end: startFrame + duration,
            source: { asset_id: assetId, source_in: 0 },
            // **循环。** 贴纸是一张短动图铺一段区间（实测 12 帧的 GIF 铺 224 帧），
            // 不循环就过不了 `source_range_exceeded`。而谎报 `frame_count`
            // 是"校验过、渲染读不存在的帧" —— 那是错的形状。
            loop_source: true,
            // 左上角 -> 中心偏移。`+ box/2` 把"左上角对齐"换成"中心对齐"。
            transform: { x: baseX, y: baseY, scale: fit, rotation: 0 },
            keyframes,
            note: `贴纸「${tag}」(${how}) 落点 ${event.x},${event.y}（盒子 ${box.toFixed(0)}px，素材 ${sourceW}×${sourceH}，缩放 ${fit.toFixed(3)}）+ 弹出/上浮/淡出关键帧`,
          });
          if (event.sfx) note(where, `贴纸自带音效「${event.sfx}」已单独转成音轨`);
          break;
        }

        case 'element': {
          // 统一视觉元素（§5）。走到这里的**只有 `kind === "text"`**（以及认不出的 kind）——
          // `kind === "sticker"` 已在 switch 之前归一化成 `type = "sticker"` 了。
          //
          // ⚠️ 这个 `case` 存在的意义是**让"文字元素没有出图通道"这件事被明确报出来**，
          //    而不是落进 `default` 的"不认识的事件类型"（那会把"本仓的待办"
          //    说成"工程的问题"，而两者该由不同的人去处理）。
          // ⚠️ `kind === "sticker"` 的元素**根本走不到这里** —— 上面已经归一化成
          //    `type = "sticker"` 并入 `case 'sticker'` 了（复用那条验过的路径）。
          const kind = (event.kind ?? '').trim().toLowerCase();
          if (kind === 'text') {
            // 文字元素 = **本仓排版 + 底座的字幕样式原语**（§5 E3）。
            //
            // ⚠️ 这一条以前是 `drop()`（"还没有排版通道"）。现在排好了 ——
            //    `process` 的 Step 1d3 已经把盒子算出来并写回 `x`/`y`（百分比），
            //    这里把它落成一层文字。
            //
            // ⚠️ **底座没有"自由文本图层"**（`Layer` 上没有 text 字段）——
            //    文字在底座里一律走**字幕资产 + `SubtitleStyle`**。
            //    所以这条的实现是：给每一条文字元素建**一个自己的 ASS 资产 +
            //    一个单层字幕轨**，样式从四档预设推。
            //    ⚠️ 不塞进主字幕轨：那条轨的样式是字幕的（font_ratio 0.0667 / 底部），
            //    混进去会让大字被字幕的样式与位置规则吃掉。
            const rawStyle = (event.style ?? '').trim().toLowerCase();
            const st = parseTextStyle(event.source, rawStyle);
            if (!st.parsed) {
              note(
                where,
                `文字元素「${event.text ?? ''}」的样式参数没拿到（event.source 为空或坏）` +
                  ` —— 按 ${st.name} 的**兜底值**画；正常路径下这不该发生（process 会写回）`,
              );
            }
            if (rawStyle && !TEXT_STYLE_FALLBACK[rawStyle]) {
              note(where, `文字元素「${event.text ?? ''}」的 style = "${rawStyle}" 认不出，按 comment 处理`);
            }
            const textBody = String(event.text ?? '');
            if (!textBody.trim()) {
              drop(where, '文字元素没有 text（空文本没什么可画的）');
              break;
            }
            const startFrame = remapFrame(srcStart);
            if (startFrame === null) {
              note(where, '这一条整段落在被掐掉的区间里，跳过（成片里没有那一段）');
              break;
            }
            const endFrame = Math.max(startFrame + 1, remapFrame(srcEnd) ?? startFrame + 1);
            textCues.push({
              start: startFrame,
              end: endFrame,
              text: textBody,
              style: st,
              // 纵向档位由排版那一步算进了 `y` —— 这里只带过来给报告用。
              pos: (event.pos ?? '').trim().toLowerCase() || 'bottom',
            });
            note(
              where,
              `文字元素「${textBody}」-> 独立文字层（style=${rawStyle || 'comment'}，` +
                `pos=${(event.pos ?? '').trim().toLowerCase() || 'bottom'}，` +
                `占 ${endFrame - startFrame} 帧）`,
            );
            break;
          }
          drop(where, `元素 \`kind = "${event.kind ?? ''}"\` 认不出（只认 text / sticker）`);
          break;
        }

        case 'sfx': {
          const name = event.name;
          if (!name) {
            drop(where, 'sfx 事件没有 name');
            break;
          }
          const startFrame = remapFrame(srcStart);
          if (startFrame === null) {
            note(where, '这一条整段落在被掐掉的区间里，跳过（成片里没有那一段）');
            break;
          }
          const endFrame = remapFrame(srcEnd) ?? startFrame + 1;
          sfxClips.push({
            name,
            start: startFrame,
            end: Math.max(startFrame + 1, endFrame),
            volume: typeof event.volume === 'number' ? event.volume : null,
          });
          registerSfx(name);
          break;
        }

        case 'overlay': {
          // 参照的 `getOverlayState`（`templates/index.html:1642-1669`）有**四档**：
          //
          //   gradient  色档 + **0.4s 进出包络**
          //   color     单色   + **同一包络**（没写颜色 → 参照 `continue` 跳过这一条）
          //   fade_in   整窗**线性斜坡**（起点全遮 → 末端全透），**不叠包络**；颜色缺省黑
          //   fade_out  反过来
          //
          // ⚠️ 以前只实现了 `gradient`，另外两档（`color` / `fade_in|fade_out`）整条 DROP
          // —— 而 `fade_in` 正是**片头渐入**那一档（`process::ensure_intro_fade_in` 补的就是它），
          // 也就是说"开头渐入没生效"这类症状里，曾经有一条是**转译侧直接丢掉的**（2026-09-30 M2 修）。
          //
          // 底座那边四档都是同一个 ColorMask 的 `overlay`：`shape` = Solid(0)/Linear(1)/Radial(2)，
          // 峰值写在 `amount`、逐帧曲线走**图层不透明度关键帧**（与 `flash`/`noise` 同一条 `placeEnvelope`）。
          const ot = event.overlay_type || '';
          const colors = event.colors ?? [];
          const startFrame = remapFrame(srcStart);
          if (startFrame === null) {
            note(where, '这一条整段落在被掐掉的区间里，跳过（成片里没有那一段）');
            break;
          }
          const durSec = Math.max(1 / fps, (srcEnd - srcStart) / fps);
          // 模板：`var opacity = typeof ev.opacity === 'number' ? ev.opacity : 1;`
          const baseOpacity = typeof event.opacity === 'number' ? event.opacity : 1;
          // 模板：`fadeIn = 0.4, fadeOut = 0.4`，且是 **else-if**（窗口比 0.8s 还短时只有进端）
          const envelope = (dt) => {
            let o = 1;
            if (dt < 0.4) o *= dt / 0.4;
            else if (durSec - dt < 0.4) o *= (durSec - dt) / 0.4;
            return Math.max(0, Math.min(1, o));
          };
          /// 单色（Solid）覆盖层的效果参数。`col` 是 `hexColor` 的结果。
          const solidEffect = (col) => ({
            kind: 'overlay',
            params: {
              amount: baseOpacity * col.a,
              r: col.r, g: col.g, b: col.b,
              r2: col.r, g2: col.g, b2: col.b,
              shape: 0, // OverlayShape::Solid
              angle: 0,
            },
          });

          if (ot === 'fade_in' || ot === 'fade_out') {
            // 颜色：`ev.color` → 色档第一个 → **缺省黑**（模板 `index.html:1656`）
            const raw =
              (typeof event.color === 'string' && event.color.trim()) ||
              String(colors[0] ?? '') ||
              '#000000';
            const col = hexColor(raw);
            if (!col) {
              drop(where, `${ot}：颜色解析不了（${raw}）`);
              break;
            }
            const ramp = (dt) => (ot === 'fade_in' ? 1 - dt / durSec : dt / durSec);
            if (placeEnvelope(startFrame, durSec, solidEffect(col), ramp, where)) {
              overlayEventCount += 1;
            }
            note(where, `${ot}：整窗线性斜坡（**不叠** 0.4s 包络），颜色 ${raw}`);
            break;
          }

          if (ot === 'color') {
            const raw =
              (typeof event.color === 'string' && event.color.trim()) || String(colors[0] ?? '');
            if (!raw) {
              // 参照也是跳过（`continue`，不是画一个默认色）—— 照实报，别自己编个颜色。
              drop(where, 'overlay_type="color" 但没写颜色（参照同一条也跳过：`index.html:1663`）');
              break;
            }
            const col = hexColor(raw);
            if (!col) {
              drop(where, `overlay_type="color"：颜色解析不了（${raw}）`);
              break;
            }
            if (placeEnvelope(startFrame, durSec, solidEffect(col), envelope, where)) {
              overlayEventCount += 1;
            }
            note(where, `单色覆盖层 ${raw}，峰值不透明度 ${baseOpacity}（含 0.4s 进出包络）`);
            break;
          }

          if (ot !== 'gradient') {
            drop(where, `overlay_type="${event.overlay_type}" 没有对应实现`);
            break;
          }
          const first = hexColor(colors[0]);
          const second = hexColor(colors[1] ?? colors[0]);
          if (!first || !second) {
            drop(where, `颜色解析不了：${JSON.stringify(colors)}`);
            break;
          }
          // 色档可以各带不透明度（`#RRGGBBAA`）。底座**整层只有一个 `amount`**：
          // 两档相同 → 精确（乘进去）；不同 → 那是它表达不了的形状，按 CLAMP 报出来并取平均
          // （不静默只取一档：取哪一档都会让另一档的颜色看起来"淡错了"）。
          const stopAlpha = (first.a + second.a) / 2;
          if (Math.abs(first.a - second.a) > 1e-6) {
            clamp(
              where,
              `两个色档的不透明度不同（${first.a.toFixed(2)} / ${second.a.toFixed(2)}）：` +
                `底座整层只有一个 amount，取平均 ${stopAlpha.toFixed(2)}`,
            );
          }
          // 渐变也走 `placeEnvelope`（而不是"整窗恒定 amount"）：参照对渐变同样叠 0.4s 进出包络，
          // 恒定强度会让渐变的头尾**硬起硬落**（与 `color` 是同一处包络）。
          if (
            placeEnvelope(
              startFrame,
              durSec,
              {
                kind: 'overlay',
                params: {
                  amount: baseOpacity * stopAlpha,
                  r: first.r, g: first.g, b: first.b,
                  r2: second.r, g2: second.g, b2: second.b,
                  // YekiTrim 的 `shape = "linear"` 对应底座 OverlayShape::Linear = 1。
                  shape: event.shape === 'radial' ? 2 : 1,
                  // 角度换算见 `overlayBaseAngle` 的注释（两边的 0° 方向不一样）。
                  angle: overlayBaseAngle(event.angle),
                },
              },
              envelope,
              where,
            )
          ) {
            overlayEventCount += 1;
          }
          note(
            where,
            `overlay 渐变：${event.shape === 'radial' ? '径向' : '线性'}，方向 ` +
              `${(typeof event.angle === 'number' ? event.angle : 135).toFixed(0)}°（参照/CSS 约定）` +
              ` ⇒ 底座 ${overlayBaseAngle(event.angle).toFixed(0)}°`,
          );
          break;
        }


        case 'pulse': {
          const startFrame = remapFrame(srcStart);
          if (startFrame === null) {
            note(where, '这一条整段落在被掐掉的区间里，跳过（成片里没有那一段）');
            break;
          }
          // `getPulse`：dt<0.12 -> ×1.06；dt<0.28 -> ×0.97；之后回 1。窗口 0.48s。
          pulseWindows.push({ outStart: startFrame, where });
          note(where, 'pulse = 视频层缩放 ×1.06 / ×0.97（两档），窗口写死 0.48s');
          break;
        }


        case 'vignette': {
          // **D4（2026-10-01）：从 DROP 改成映射。**
          //
          // 以前这里是"参照出片路径里没有 vignette（只有 DOM 的 `handlers.vignette`，
          // `renderFrame` 里没有 `getXxx`）⇒ 不替参照补画"。那一刀切在**参照侧**：
          // 本仓 canvas 现在补上了暗角（`templates/index.html::getVignette` + 出片里那段
          // 归一化空间的径向渐变），两端一致了 ⇒ 转译器映射底座的 `vignette` 特效。
          //
          // 参照的形（DOM 预览）：
          //     radial-gradient(ellipse at center, transparent 55%, rgba(0,0,0,0.5) 100%)
          //     × getVignette(t) 包络（前 dur×0.4 恒 1，之后在 min(0.3, dur×0.6) 秒内线性归零）
          // 底座那张牌：`kind:"vignette"`，`amount` = **边缘压暗量**（0.5 ⇔ rgba(…,0.5)），
          // `radius`/`softness` 取它的默认（0.7 / 0.35，与 55% 档一致）。
          // 逐帧曲线照旧走 `placeEnvelope`（图层不透明度关键帧 × amount）—— 与 flash/hue 同一套。
          const startFrame = remapFrame(srcStart);
          if (startFrame === null) {
            note(where, '这一条整段落在被掐掉的区间里，跳过（成片里没有那一段）');
            break;
          }
          // `transientDur(ev, 0.4)`：cap 0.6（同参照的 `TRANSIENT_CAP.vignette`）。
          // ⚠️ **没写 `end` 时用默认 0.4**，不是"事件跨度"（跨度此时是 0）—— 参照的
          // `evDuration(ev, def)` 就是这个语义：有 end 用 `end - time`，否则用 def。
          // （第一版写成了跨度 ⇒ 没写 end 的工程里暗角只有 0.1s，几乎看不见。）
          const hasEnd = typeof event.end === 'number' && event.end > at;
          const rawSpan = hasEnd ? Math.max(0.05, event.end - at) : 0.4;
          const vigDur = Math.max(0.1, Math.min(rawSpan, 0.6));
          placeEnvelope(
            startFrame,
            vigDur,
            { kind: 'vignette', params: { amount: 0.5, radius: 0.7, softness: 0.35 } },
            (dt) => {
              if (dt < vigDur * 0.4) return 1;
              return Math.max(0, 1 - (dt - vigDur * 0.4) / Math.min(0.3, vigDur * 0.6));
            },
            where,
          );
          note(
            where,
            `暗角：边缘压暗 0.5（参照 = radial-gradient transparent 55% → rgba(0,0,0,.5) 100%），` +
              `包络 ${vigDur.toFixed(2)}s（前 40% 恒 1，之后 ${Math.min(0.3, vigDur * 0.6).toFixed(2)}s 线性归零）`,
          );
          break;
        }



        case 'flash': {
          const startFrame = remapFrame(srcStart);
          const rgb = hexColor(event.color) ?? { r: 1, g: 1, b: 1 };
          const dur = typeof event.duration === 'number' ? event.duration : 0.35;
          // `getFlash`：0.04s 升到 **0.20**，然后 `dur` 秒线性落回 0。
          placeEnvelope(
            startFrame,
            0.04 + dur,
            { kind: 'flash', params: { amount: 0.20, r: rgb.r, g: rgb.g, b: rgb.b } },
            (dt) => (dt < 0.04 ? dt / 0.04 : Math.max(0, 1 - (dt - 0.04) / dur)),
            where,
          );
          note(where, `flash 色 ${event.color || '#ffffff'}，峰值 0.20、上升 0.04s、下降 ${dur}s`);
          break;
        }

        case 'hue_shift': {
          const startFrame = remapFrame(srcStart);
          const rgb = hexColor(event.color) ?? { r: 1, g: 0.0, b: 0.25 };
          // `getHueShiftFlash`：0.15s 升到 **0.25**，然后 0.80s 线性落回 0（窗口 0.95s）。
          placeEnvelope(
            startFrame,
            0.95,
            { kind: 'flash', params: { amount: 0.25, r: rgb.r, g: rgb.g, b: rgb.b } },
            (dt) => (dt < 0.15 ? dt / 0.15 : Math.max(0, 1 - (dt - 0.15) / 0.80)),
            where,
          );
          note(where, `hue_shift 色 ${event.color || '#ff0040'}，峰值 0.25、上升 0.15s、下降 0.80s`);
          break;
        }

        case 'noise': {
          const startFrame = remapFrame(srcStart);
          // §20：`noise` 发**底座的 `noise` 原语**（真黑白噪点），不再发白色 `flash`。
          //
          // ⚠️ **这条以前映射成白色 `flash`**，注释自陈"参照出片里的 noise 是一层纯白薄纱"。
          // 用户拍板「noise 需要改成真黑白噪点」⇒ 坐标系换了：**以真噪声为准**，
          // 本仓出片模板的 `handlers.noise` 同步改成真噪声（那边改完要重建二进制）。
          //
          // ⚠️ 强度**从 0.08 降下来到 0.15 是错的、0.08 保持不变也是错的** ——
          //    这两个数不可直接比较：白纱 0.08 是"整屏盖一层 8% 的白"（低频、糊），
          //    真噪点 0.08 是"逐像素 ±10/255 的颗粒"（高频、刺）。
          //    底座侧的量纲 bug 修掉之后（`NOISE_FULL_SCALE`），**0.08 就是 ±10 的颗粒** ——
          //    那是一个明显可见但不糊的强度，正合"爆点闪一下"的用法。
          //    ⚠️ 以前那个 bug 让 0.08 变成 ±0.04（看不见），所以本节的两个数**不可比**。
          //
          // 参数名与底座登记表对齐：`amount` + `seed`（见 `dhampir-core/src/effects.rs` 的 `NOISE`）。
          // `seed` 取一个固定值：噪声必须是**帧号的函数**，同一帧两次渲染要逐字节相同
          //（用随机会让预览与成片对不上）。
          placeEnvelope(
            startFrame,
            0.80,
            { kind: 'noise', params: { amount: 0.08, seed: 7 } },
            (dt) => (dt < 0.08 ? dt / 0.08 : Math.max(0, 1 - (dt - 0.08) / 0.72)),
            where,
          );
          note(
            where,
            '`noise` 发底座的**真黑白噪点**（§20）：amount 0.08 + 固定 seed，' +
              '包络沿用 0.08s 升 / 0.72s 落。⚠️ 以前这里发的是白色 `flash`（参照出片的"白纱"），' +
              '现在以真噪声为准 —— 出片模板的 `handlers.noise` 已同步改成真噪声。',
          );
          break;
        }

        case 'blur': {
          const startFrame = remapFrame(srcStart);
          if (startFrame === null) {
            note(where, '这一条整段落在被掐掉的区间里，跳过（成片里没有那一段）');
            break;
          }
          // `getBlur`：0.80s 窗口 —— 前 0.48s **恒定 8px**，后 0.32s 线性降到 0。
          //
          // 半径是**参数**、不是不透明度，所以不能用包络去混（那会变成
          // "糊与不糊的叠加"而不是"半径变小"）。这里按帧发 1 帧一层的
          // 调整图层，半径逐帧给 —— 0.32s 是 19 层。
          // # 半径口径：CSS 的 `blur(Npx)` 里 **N 就是标准差 σ**
          //
          // 底座 `gaussian_blur` 的 `radius` 是**核半径**，而它内部取 **σ = radius / 2**
          // （`render/blur.rs`：`sigma 取 radius / 2 —— 核覆盖 ±2σ`）。
          //
          // 所以参照的 `blur(8px)`（σ=8）对应底座 **radius = 16**。
          // 我第一版直接写 `radius: 8`，那只有 σ=4 —— **糊得只有一半**，
          // 实测该窗口与参照差 27.6（窗口外基线 1.7）。
          const CSS_BLUR_PX = 8;
          const peakRadius = Math.min(2 * CSS_BLUR_PX, 16); // 16 是登记表上限
          const constFrames = Math.round(0.48 * fps);
          const rampFrames = Math.round(0.32 * fps);
          for (let i = 0; i < constFrames; i += 1) {
            placeAdjustmentOut(startFrame + i, startFrame + i + 1,
              { kind: 'gaussian_blur', params: { radius: peakRadius } }, where);
          }
          for (let i = 0; i <= rampFrames; i += 1) {
            const radius = peakRadius * (1 - i / rampFrames);
            if (radius <= 0.01) break;
            placeAdjustmentOut(startFrame + constFrames + i, startFrame + constFrames + i + 1,
              { kind: 'gaussian_blur', params: { radius } }, where);
          }
          note(
            where,
            `blur css(${CSS_BLUR_PX}px) = σ${CSS_BLUR_PX} -> 底座 radius=${peakRadius}（σ=radius/2）；` +
              `恒定 0.48s + 线性降 0.32s（按帧拆成 ${constFrames + rampFrames + 1} 层）`,
          );
          break;
        }
        default:
          // **按词汇表分类报**（2026-10-01，S3 派生）：`spec/effects.json` 是效果类型的唯一权威。
          // 三种情况说清是哪一种 —— 以前只有一句"不认识的类型"，分不出"这个效果不该存在"与"转译器漏接了"。
          if (EFFECT_VOCAB.known.has(event.type)) {
            drop(
              where,
              `事件类型「${event.type}」（${EFFECT_VOCAB.labels.get(event.type) ?? '?'}）在词汇表里，` +
                `但**转译器还没有对应的 case** —— 这是本仓的漏洞（加效果时漏了转译器），不是工程的问题`,
            );
          } else {
            drop(where, `不认识的事件类型「${event.type}」—— 它也不在效果词汇表（spec/effects.json）里`);
          }
      }
    }
  }

  // ---------------------------------------------------------------------
  // 运镜：照抄 YekiTrim 的**状态机**，不是"每条事件画两个端点"
  // ---------------------------------------------------------------------
  //
  // 权威实现在 `yekitrim-polish/templates/index.html:1398 getActiveCamera()`：
  //
  //     var zoom = 1.0                        // 初值，不是第一个事件的值
  //     for each camera 事件 c:
  //         tgt = camFactor(c.zoom)
  //         td  = min(c.duration * 0.5, 0.6)   // 过渡窗口，最多 0.6 秒
  //         ce  = c.time + td
  //         hardCut = c.cut || tgt == 远 || zoom == 远
  //         p   = (t >= ce || hardCut) ? 1 : pow2_out((t - c.time) / td)
  //         zoom = (t >= ce || hardCut) ? tgt : lerp(zoom, tgt, p)
  //
  // 三条要点，我先前**每一条都做错了**：
  //
  // 1. **过渡发生在事件的「开头」，只占 `td`（≤0.6s）**，不是横跨整条事件。
  //    我先前在事件起点写"目标值"、在终点再写一次同值，于是相邻两条之间
  //    是「从上一段一直插值到下一段」—— 实测 45s 处 zoom 变成 1.3→1.5 的
  //    中间值，而 YekiTrim 是稳稳的 1.3（那 0.6s 的过渡早就结束了）。
  //    差 42.46/255。
  //
  // 2. **远景（`远`）进出是硬切**（`td` 当 0），因为"缓动缩过去看着像镜头在拉远"。
  //
  // 3. **初值是 1.0**，第一个事件也是从 1.0 过渡过去。
  //
  // 还有个便宜的巧合让关键帧能精确表达它：`offset = (1-z)*(origin-中心)`，
  // `origin` 在过渡期内**不变**（`to_origin` 缺省即 origin），而 `z` 按
  // `pow2_out` 插值 —— 于是 `offset` 也是同样一条缓动曲线上的线性量，
  // 用同一个 easing 给 `x`/`y`/`scale` 写关键帧就是**逐值等价**的。
  //
  // **这不是近似**：YekiTrim 的 `pow2_out` 与底座的 `EaseOut` **逐值相同**
  // （`pow2_in` 同理对上 `EaseIn`，`schema.rs` 有用例钉着）—— 我先前把它报成
  // "同族不同参"并进 CLAMP，那是错的；见下面 `逐帧重放 getActiveCamera` 那一趟的说明。
  const WIDE_ZOOM = zoomFactor('远', config.style?.camera, 'style.camera');
  cameraEvents.sort((a, b) => a.srcStart - b.srcStart);

  // ---------------------------------------------------------------------
  // **画布 fit 因子** —— 源怎么铺进画布
  // ---------------------------------------------------------------------
  //
  // # 底座画源的默认几何
  //
  // `inverse_affine`（`compose.rs:174`）在 `scale = 1`、`offset = 0` 时给出的是
  // **"中心对齐 + 1:1"**：`p_src = p_out - c_out + c_src`。
  // **既不是 contain 也不是 cover** —— 源按原始像素尺寸画在画布中心，
  // 装不下的部分在画布外（看不见），装不满的地方是透明（出片即黑）。
  //
  // 而参照的 canvas 那条腿是 `object-fit`：
  //
  //   * `axis = "fill"`  -> **cover**（裁切铺满），`f = max(cw/sw, ch/sh)`
  //   * `axis = "x"/"y"` -> **contain**（完整可见 + 留边），`f = min(cw/sw, ch/sh)`
  //
  // # 为什么只乘 `scale`、`cameraOffset` 不动
  //
  // 参照：`p_out = z * (p_in - origin) + origin`，而 `p_in = f*(p_src - c_src) + c_out`。
  // 代入展开：
  //
  //     p_out = z*f*(p_src - c_src) + z*(c_out - origin) + origin
  //
  // 底座：`p_out = scale*(p_src - c_src) + c_out + offset`。两式对照：
  //
  //     scale  = z * f                          <- **只改这里**
  //     offset = (1 - z) * (origin - c_out)     <- 就是现有的 `cameraOffset`
  //
  // 所以 `cameraOffset` 一个字都不用动（它用的 `z` 是**相机 zoom**，不含 fit）。
  // 比例一致时 `f = 1`，既有工程**逐字节不变**。
  //
  // ⚠️ **`contain` 也已经做了**（2026-10-01 更正：`stageDrawRect` 把 `axis = x/y` 的**摆位**、
// `position`（left/right/top/bottom/center）、`padding`、`scale` 全实现了，`FIT` 就是它算出来的 ——
// 这条注释以前写着"只做了 fill 这一半"，是**过期**的）。真正还差的是**背景层**：
// 配方已实现（`bg1` 同源层 + `gaussian_blur`、`bg2` 调整层），**2026-10-06 起默认打开**
// （原先关着是因为"中间纹理逐帧累积、长片 OOM"；现在真跑 1800/3600 帧 WorkingSet 全程平，见开关那儿）。
// 原话保留：**只做了 `axis = "fill"`（cover）这一半。**
  // `contain` 还差**摆位**（`position`）与**背景层**（`bg_mode` 的模糊/纯色/黑）——
  // 见转译完成时的 CLAMP。
  // ---------------------------------------------------------------------
  // **画布 fit：`sr`（主画面的绘制矩形）+ 相机绕它转**
  // ---------------------------------------------------------------------
  //
  // 这段照搬参照 `index.html:1793` 的 `stageDrawRect` —— **那是权威实现**，
  // 不是我的推导。它同时管 `axis`（fill/x/y）、`position`、`padding`、`scale`。
  //
  // 关键点：**相机是绕 `sr` 转的，不是绕画布**：
  //
  //     var anchorX = sr.x + cam.ox / CW * sr.w;
  //     p_out = anchor + camScale * (p_sr - anchor)
  //
  // 而底座是 `p_out = scale*(p_src - c_src) + c_out + offset`。两式对照
  // （其中 `p_sr = sr + (p_src/size)*sr.wh`）：
  //
  //     scale  = camScale * (sr.w / sw)                    <- sr 等比，一个数就够
  //     offset = anchor*(1-z) + z*sr + scale*c_src - c_out
  //
  // # 与"只乘一个 fit 因子"的关系
  //
  // 先前那版只乘了 `f = max(cw/sw, ch/sh)`，那**只在 cover 且 sr 铺满该轴时**成立：
  // 那时 `sr.h = CH`，上面通式正好化简成 `offset = (1-z)*(origin - c_out)`，
  // 也就是原本的 `cameraOffset`。换成通式之后 **cover 行为不变，contain 也对得上**。
  // 与上面**同一份**探测结果（别在这儿再探一次源：预览用代理时两边会打架）。
  // 探不到时的回落与改造前逐字一致：按画布尺寸当源（CLAMP 文案见下面 stage 那段）。
  const srcW = mediaW ?? width;
  const srcH = mediaH ?? height;
  // **位置 = 两轴各自的百分比**（2026-10-05 x/y 改版）：`position_x_pct` / `position_y_pct`
  // 优先；**缺哪条轴**就回落到单值 `position_pct`（2026-10-02 的字段），再缺按老枚举换算
  // （`auto` → 50 居中、`left`/`top` → 0、`right`/`bottom` → 100；方向不跨用 ⇒ 居中）。
  // 与 `ProjectFile::resolve_stage` 逐值同义（含 `[0,100]` 夹取）。
  // ⚠️ 老单值 ⇒ 两轴同值：贴满的那条轴余量为 0，代入什么值都不动 ⇒ **与老行为逐像素等价**。
  const stagePctLegacy = (() => {
    const raw = config.stage?.position_pct;
    if (typeof raw === 'number' && Number.isFinite(raw)) return Math.max(0, Math.min(100, raw));
    const p = String(config.stage?.position ?? 'auto').trim().toLowerCase();
    const landscape = config.canvas?.orientation !== 'portrait';
    if (p === 'left' && landscape) return 0;
    if (p === 'right' && landscape) return 100;
    if (p === 'top' && !landscape) return 0;
    if (p === 'bottom' && !landscape) return 100;
    return 50; // center / auto / 方向不跨用 / 认不出
  })();
  const clampPct = (raw) =>
    typeof raw === 'number' && Number.isFinite(raw) ? Math.max(0, Math.min(100, raw)) : stagePctLegacy;
  const stagePctX = clampPct(config.stage?.position_x_pct);
  const stagePctY = clampPct(config.stage?.position_y_pct);
  // 轴**只看比例**（2026-10-02 起没有 auto 特例：不匹配的源一律**适配留边**）——
  // 与参照 `stageDrawRect` 的调用方同一条判据（交叉相乘，避免浮点除法）。
  const axis = (() => {
    // 源尺寸探不到 ⇒ 不猜轴（与改造前 `!fitSrc` 同一条口径）
    if (mediaW == null || mediaH == null) return 'fill';
    const lhs = BigInt(srcH) * BigInt(width);
    const rhs = BigInt(srcW) * BigInt(height);
    if (lhs > rhs) return 'x';
    if (lhs < rhs) return 'y';
    return 'fill';
  })();
  // 枚举只是**过渡期兼容输出**（报告那行摘要用）；几何一律走两轴 pct。
  // 与 `ProjectFile::resolve_stage` 同口径：按**有留边的那条轴**报（`axis=fill` 一律 center）。
  const stageMainPct = axis === 'x' ? stagePctX : axis === 'y' ? stagePctY : 50;
  const stagePosition = stageMainPct <= 0
    ? (axis === 'y' ? 'top' : 'left')
    : stageMainPct >= 100
      ? (axis === 'y' ? 'bottom' : 'right')
      : 'center';
  const stagePadding = Number(config.stage?.padding ?? 0) || 0;
  const stageScale = Number(config.stage?.scale ?? 1) || 1;

  /// 与参照的 `stageDrawRect` 逐值同义。
  const stageDrawRect = (bw, bh, cw, ch) => {
    if (!(bw > 0) || !(bh > 0)) return { x: 0, y: 0, w: cw, h: ch };
    let pad = stagePadding;
    const cap = Math.min(cw, ch) / 2 - 1;
    if (pad > cap) pad = cap > 0 ? cap : 0;
    const bx = pad;
    const by = pad;
    const bwid = cw - pad * 2;
    const bhgt = ch - pad * 2;
    let x;
    let y;
    let w;
    let h;
    if (axis === 'x') {
      h = bhgt;
      w = (bw * h) / bh;
      if (w > bwid) {
        w = bwid;
        h = (bh * w) / bw;
      }
    } else if (axis === 'y') {
      w = bwid;
      h = (bh * w) / bw;
      if (h > bhgt) {
        h = bhgt;
        w = (bw * h) / bh;
      }
    } else {
      const s = Math.max(bwid / bw, bhgt / bh);
      w = bw * s;
      h = bh * s;
    }
    // ② 绕**布局盒中心**乘大小（先定画面大小，再按余量摆位）
    w *= stageScale;
    h *= stageScale;
    // ③ 两轴各自在「盒 − 画面」的余量上按 `pct/100` 线性插值（= CSS `background-position` 语义；
    //    余量为负时同一条插值就是**裁切取景**）—— 与参照 `stageDrawRect`（模板）逐值同义。
    x = bx + (bwid - w) * (stagePctX / 100);
    y = by + (bhgt - h) * (stagePctY / 100);
    return { x, y, w, h };
  };
  const SR = stageDrawRect(srcW, srcH, width, height);
  /// `sr` 带来的等比缩放（= 先前那个 `FIT`，但现在含 `padding` / `scale`）。
  const FIT = SR.w / srcW;
  /// 背景模式：判据照抄参照 `resolve_stage`（横画布**且铺满** -> none，否则 black/color/blur）。
  const STAGE_BG = (() => {
    const fills = axis === 'fill' && config.canvas?.orientation !== 'portrait';
    if (fills) return 'none';
    const mode = config.stage?.bg_mode ?? 'auto';
    if (mode === 'black') return 'black';
    if (mode === 'color') return 'color';
    return 'blur';
  })();
  if (mediaW == null || mediaH == null) {
    clamp('stage', '探不到源素材尺寸（ffprobe 不可用？），按画布尺寸当源 —— 比例不一致时画面会错位');
  }
  if (Math.abs(FIT - 1) > 1e-6 || SR.x !== 0 || SR.y !== 0 || STAGE_BG !== 'none') {
    note(
      'stage',
      `源 ${srcW}x${srcH} -> 画布 ${width}x${height}（axis=${axis} position_x=${stagePctX} ` +
        `position_y=${stagePctY} position=${stagePosition} ` +
        `padding=${stagePadding} scale=${stageScale}）：绘制矩形 ` +
        `${SR.x.toFixed(1)},${SR.y.toFixed(1)} ${SR.w.toFixed(1)}x${SR.h.toFixed(1)}` +
        `（fit ${FIT.toFixed(4)}），背景 bg=${STAGE_BG}`,
    );
  }

  /// 给定 zoom 与不动点，算底座 `Transform.x/y`（文档像素）。
  ///
  /// **相机绕 `sr` 转，不是绕画布** —— 照搬参照 `renderFrame`：
  ///
  ///     var anchorX = sr.x + cam.ox / CW * sr.w;
  ///     p_out = anchor + camScale * (p_sr - anchor)
  ///
  /// 而底座是 `p_out = scale*(p_src - c_src) + c_out + offset`。两式对照：
  ///
  ///     scale  = camScale * (sr.w / sw)          <- sr 等比，一个数就够
  ///     offset = anchor*(1-z) + z*sr + scale*c_src - c_out
  ///
  /// # ⚠️ 这里踩过一个坑，值得留着
  ///
  /// 我先前用的是"只对 cover 成立"的简化式 `offset = (1-z)*(origin - c_out)`，
  /// 还**误以为两者等价** —— 因为我只验了 `origin = 0.5` 那个点，而**两式在那一点
  /// 恰好都归零**。换个 origin 立刻分开：`ox=0.4, z=0.55` 时旧式 −48.66、通式 −153.80
  /// （两份工程实测差 764 个关键帧、`x` 差 105~175 像素）。
  ///
  /// 实测读数从 44.20 变成 48.94 —— **但那不能作为反证**：竖屏那份的读数被
  /// "本仓手写双线性放大比浏览器 `drawImage` 软 2.5 倍"主导，4.7 是二阶效应。
  /// **以推导为准，不以那个读数为准。** 而且不换通式，`contain` 的摆位无从谈起。
  const cameraOffset = (zoom, origin) => {
    const scale = zoom * FIT;
    const ax = SR.x + origin.x * SR.w;
    const ay = SR.y + origin.y * SR.h;
    return {
      x: ax * (1 - zoom) + zoom * SR.x + scale * (srcW / 2) - width / 2,
      y: ay * (1 - zoom) + zoom * SR.y + scale * (srcH / 2) - height / 2,
    };
  };

  let prevZoom = 1.0;
  // **上一个状态的位移也要跟着记。** 只记 zoom 是不够的：硬切的"钉住"那一帧
  // 要写的是**上一个状态**的 offset，而 offset 同时依赖 zoom 与不动点。
  // 我先前拿"本事件自己的 origin"重算了一遍，于是 65s 那一帧的 x 从 -86.52
  // 一路插值到 0.00（差 44.75/255）—— 缩放对、位置不对。
  // `focus` 现在**恒非空**（= 模板那条 `defOrigin`，见 `templateDefaultOrigin`）——
  // 这里不再自己编一个 0.5/0.5 的兜底：那会让"锚点"与"位移起点"用两个不同的不动点。
  let prevOffset = cameraOffset(1.0, focus);
  for (let index = 0; index < cameraEvents.length; index += 1) {
    const shot = cameraEvents[index];
    const z = shot.z;
    const hardCut = shot.cut
      || Math.abs(z - WIDE_ZOOM) < 1e-6
      || Math.abs(prevZoom - WIDE_ZOOM) < 1e-6;
    // 过渡窗口（规格 §1）：`speed` 决定，缺省走**老式子**（存量逐字节零回归）。
    const durationSec = Math.max(1 / fps, (shot.srcEnd - shot.srcStart) / fps);
    const tdFrames = cameraTransitionFrames({ speed: shot.speed, hardCut, durationSec, fps });

    // 过渡的**起点**：zoom 还是上一个状态，但不动点已经是**本事件自己的** origin
    // （YekiTrim 里 `ox = lerp(ax, bx, p)`，p=0 时就是 ax）。
    const from = shot.origin ?? focus ?? { x: 0.5, y: 0.5 };
    const to = shot.originTo ?? from;
    const off1 = cameraOffset(z, to);

    if (tdFrames === 0) {
      // **硬切**：在**前一帧**把旧状态钉住，这一帧才是瞬时跳变。
      //
      // 不钉的话，两个关键帧落在同一帧上会互相覆盖（后面的赢），
      // 曲线就从**上一个关键帧**一路插值过来 —— 实测那会把整段「远景」
      // 拖成一条 10 秒的缓动，第 65 秒那一帧差 **141.48/255**
      // （画面在慢慢拉远，而它本该就是一个硬切）。
      if (shot.srcStart - 1 >= 0) {
        placeKeyframe(shot.srcStart - 1, 'scale', prevZoom * FIT, 'linear');
        placeKeyframe(shot.srcStart - 1, 'x', prevOffset.x, 'linear');
        placeKeyframe(shot.srcStart - 1, 'y', prevOffset.y, 'linear');
        const prevOut = remapFrame(shot.srcStart - 1);
        if (prevOut !== null) sparseOutFrames.add(prevOut);
      }
      placeKeyframe(shot.srcStart, 'scale', z * FIT, 'linear');
      placeKeyframe(shot.srcStart, 'x', off1.x, 'linear');
      placeKeyframe(shot.srcStart, 'y', off1.y, 'linear');
      const cutOut = remapFrame(shot.srcStart);
      if (cutOut !== null) sparseOutFrames.add(cutOut);
    } else {
      // 过渡：起点用旧 zoom、终点用新 zoom，窗口就是 `td`。
      const off0 = cameraOffset(prevZoom, from);
      // **缓动写在"段落终点"那个关键帧上。**
      //
      // 求值用的是 `b.easing`（`curve.rs::channel_from`：相邻对 [a,b] 取 **b** 的缓动），
      // 也就是「怎么**到达**这个键」。我先前把 `ease_out` 写在**起点**键上 ——
      // 于是这一段用的是终点键的 `linear`，**整整一段运镜都变成了直线插值**。
      //
      // 实测（19.9s 中->近，td=0.6s）：
      //   时刻     参照 zoom    我的 zoom
      //   19.9      1.300       1.297   <- 端点对
      //   20.15     1.432       1.398   <- 中段差 2.4%
      //   20.30     1.478       1.449
      //   20.50     1.500       1.502   <- 端点对
      // 两端精确、中段偏小 —— 这条形状就是"直线 vs pow2_out"。
      placeKeyframe(shot.srcStart, 'scale', prevZoom * FIT, 'linear');
      placeKeyframe(shot.srcStart, 'x', off0.x, 'linear');
      placeKeyframe(shot.srcStart, 'y', off0.y, 'linear');
      placeKeyframe(shot.srcStart + tdFrames, 'scale', z * FIT, 'ease_out');
      placeKeyframe(shot.srcStart + tdFrames, 'x', off1.x, 'ease_out');
      placeKeyframe(shot.srcStart + tdFrames, 'y', off1.y, 'ease_out');
      const tr0 = remapFrame(shot.srcStart);
      const tr1 = remapFrame(shot.srcStart + tdFrames);
      if (tr0 !== null) sparseOutFrames.add(tr0);
      if (tr1 !== null) sparseOutFrames.add(tr1);
    }

    if (hardCut && !shot.cut && tdFrames === 0) {
      note(shot.where, `远景（${shot.zoom}）进出按 YekiTrim 的规则**硬切**，没有过渡`);
    }
    if (shot.z < 1) {
      clamp(
        shot.where,
        `zoom=${shot.zoom}（${shot.z}）是**画中画档**：四周会留黑边 —— ` +
          '这与 YekiTrim 一致（它也是黑底），但 YekiTrim 的 `stage.bg_mode` 还能铺模糊底，底座没有那一层',
      );
    }
    // `handheld` 不在这里处理 —— 它是**视频层的纯位移/旋转/呼吸缩放**，
    // 与独立 `shake` 一起在循环之后按帧采样（见 `cameraStateAt` 那一段）。
    if (shot.zoom === '近' && !shot.handheld) {
      note(shot.where, '近景未加即时抖 —— YekiTrim 的近景自带轻微推拉，底座只做缩放');
    }
    prevZoom = z;
    prevOffset = off1;
  }
  // ---------------------------------------------------------------------
  // 手持摇摆 + 独立 shake：**视频层的纯位移**，按帧采样
  // ---------------------------------------------------------------------
  //
  // 这两条都是 `t` 的纯函数，且都作用在**视频层**上（`renderFrame` 里它们都在
  // video 那段 `ctx.save()` 之内）—— 字幕/弹幕/贴纸**不跟着抖**。
  //
  // 所以表达方式是：**把逐帧算出来的位移写进主画面层自己的 `x`/`y` 关键帧**
  // （手持还要写 `scale`/`rotation`）。不用调整图层 —— 那会影响底下所有东西。
  //
  // # 为什么必须逐帧采，不能手写几条关键帧
  //
  // 手持是 4 个通道 × 每条 2 个正弦分量（6 个频率/相位参数）；
  // shake 是 0.04s 一档的四级阶梯 —— 60fps 下一档只有 2~3 帧，
  // **手写关键帧根本落不到那些帧上**。

  /// 逐帧重放 YekiTrim 的 `getActiveCamera`（`index.html:1398`）。
  ///
  /// 运镜那段循环写的是**稀疏关键帧**（每条事件起点/过渡终点），
  /// 而 `handheld` 与它共用同一套状态机、同一个 `td`、同一条缓动 ——
  /// 所以这里按帧重放一遍，供"需要逐帧"的那两条用。
  ///
  /// **口径必须与那段循环逐值一致**，否则手持窗口的边界会出现台阶。
  const camEventsOut = cameraEvents
    .map((shot) => ({
      outStart: remapFrame(shot.srcStart),
      durationSec: Math.max(1 / fps, (shot.srcEnd - shot.srcStart) / fps),
      z: shot.z,
      origin: shot.origin ?? focus ?? { x: 0.5, y: 0.5 },
      originTo: shot.originTo ?? shot.origin ?? focus ?? { x: 0.5, y: 0.5 },
      cut: shot.cut,
      handheld: shot.handheld,
      tilt: shot.tilt ?? 0,
    }))
    .filter((e) => e.outStart !== null)
    .sort((a, b) => a.outStart - b.outStart);

  const cameraStateAt = (outFrame) => {
    const t = outFrame / fps;
    let zoom = 1.0;
    let handheld = 0.0;
    // 荷兰角也走"持续状态 + 同一段过渡"：与 zoom 逐式同形（同 `td`、同 `hardCut`、同 `p`）。
    // ⚠️ 起始值**必须是 0**（不倾斜）—— 写成第一条事件的 tilt 会让**片头一开始就歪着**，
    //    而用户从任何界面都看不出那个初值是从哪来的。
    let tilt = 0.0;
    const def = focus ?? { x: 0.5, y: 0.5 };
    let ox = def.x * 100;
    let oy = def.y * 100;
    for (const e of camEventsOut) {
      const startT = e.outStart / fps;
      if (t < startT) break;
      const td = Math.min(e.durationSec * 0.5, 0.6);
      const ce = startT + td;
      const hardCut = e.cut
        || Math.abs(e.z - WIDE_ZOOM) < 1e-6
        || Math.abs(zoom - WIDE_ZOOM) < 1e-6;
      const p = (t >= ce || hardCut) ? 1.0 : pow2Out((t - startT) / td);
      zoom = (t >= ce || hardCut) ? e.z : zoom + (e.z - zoom) * p;
      handheld += ((e.handheld ? 1.0 : 0.0) - handheld) * p;
      tilt += ((e.tilt ?? 0) - tilt) * p;
      const ax = e.origin.x * 100;
      const ay = e.origin.y * 100;
      const bx = e.originTo.x * 100;
      const by = e.originTo.y * 100;
      ox = ax + (bx - ax) * p;
      oy = ay + (by - ay) * p;
    }
    return { zoom, handheld, tilt, ox: (ox * width) / 100, oy: (oy * height) / 100 };
  };

  /// 输出帧 -> 源帧（`placeKeyframe` 要的是源帧号）。
  ///
  /// ⚠️ **只返回第一个命中的段** —— 它给"只需要一个代表帧"的调用点用。
  /// 逐帧写关键帧那条路**不要**用它：EDL 允许重复源区间（重放），那时一个输出帧
  /// 对应多个段，必须逐段各写一份（见下面那个 `for (const range of keepRanges)`）。
  const sourceFrameForOutput = (outFrame) => {
    for (const range of keepRanges) {
      const length = range.durFrames ?? (range.end - range.start);
      if (outFrame >= range.outStart && outFrame < range.outStart + length) {
        const speed = range.speed ?? 1.0;
        return range.start + Math.round((outFrame - range.outStart) * speed);
      }
    }
    return null;
  };

  {
    // **`durationFrames` 才是输出总帧数。** 我先前写成
    // `remapFrame(segments.at(-1).end)` —— 那是**秒**，而 `remapFrame` 要的是**源帧号**，
    // 于是循环只跑到第 114 帧，19.9s 那次 shake（第 ~1194 帧）根本没进过这个循环。
    // 症状是"代码写了、NOTE 也打了，但关键帧一个都没落上"。
    const lastOut = Math.max(0, durationFrames - 1);

    // ---------------------------------------------------------------------
    // 这一趟管五件事，它们**都作用在视频层上**（`renderFrame` 里全在 video 那段
    // `ctx.save()` 之内）：
    //
    //     shake       0.2s 四级阶梯位移          （基础级，global:1）
    //     stutter     0.12s 两档位移            （基础级，global:1）
    //     zoom_bounce ×1.10（0.08s）/ 0.30s 窗口 （乘在运镜 zoom 上）
    //     pulse       ×1.06 / ×0.97 / 0.48s 窗口 （乘在运镜 zoom 上）
    //     handheld    2×4 正弦，amp 走运镜状态机
    //
    // 三者之外的**运镜本身**仍然用稀疏关键帧（那段逻辑已验证过）。
    // ---------------------------------------------------------------------
    const shakeFrames = Math.max(1, Math.round(SHAKE_SECONDS * fps));
    const shakeSpans = shakeWindows.map((w) => ({
      from: w.outStart, to: w.outStart + shakeFrames, intensity: w.intensity,
    }));
    // `getStutterVideo`：0.12s，0.06s 处换档
    const stutterSpans = stutterWindows.map((w) => ({
      from: w.outStart, to: w.outStart + Math.max(1, Math.round(0.12 * fps)), intensity: w.intensity,
    }));
    const bounceSpans = zoomBounceWindows.map((w) => ({
      from: w.outStart, to: w.outStart + Math.max(1, Math.round(0.30 * fps)),
    }));
    const pulseSpans = pulseWindows.map((w) => ({
      from: w.outStart, to: w.outStart + Math.max(1, Math.round(0.48 * fps)),
    }));

    /// `getZoomBounce`：`dt < 0.08 ? curZoom*1.10 : curZoom`（窗口 0.30s）
    const bounceFactor = (dt) => (dt < 0.08 ? 1.10 : 1.0);
    /// `getPulse`：`dt < 0.12 -> 1.06`；`dt < 0.28 -> 0.97`；之后 1.0（窗口 0.48s）
    const pulseFactor = (dt) => (dt < 0.12 ? 1.06 : dt < 0.28 ? 0.97 : 1.0);
    /// `getStutterVideo`：`dt < 0.06 -> (-off, +off)`，否则 `(off, -off)`（窗口 0.12s）
    const stutterOffset = (dt, off) =>
      dt < 0.06 ? { x: -off, y: off } : { x: off, y: -off };

    // ---------------------------------------------------------------------
    // **窗口内每一帧都要写，哪怕这一帧的值恰好等于基准。**
    //
    // 这一条是踩出来的：`zoom_bounce` 的系数在 `dt >= 0.08` 之后回到 **1.0**，
    // 于是"这一帧要不要写"的判据变假、**不再写关键帧** —— 而**已经写下的
    // `scale = zoom × 1.1` 会一直留到下一个稀疏关键帧**（实测 62.7s 留到 66s，
    // 视频整整放大 1.1 倍，顶部那条边带被推出画面）。
    //
    // 所以判据从"这一帧动没动"改成"这一帧在不在某个窗口的跨度里"，
    // 并且每个跨度**多盖一帧**用来收尾（把值钉回基准）。
    // ---------------------------------------------------------------------
    const spans = [
      ...shakeSpans.map((s) => [s.from, s.to]),
      ...stutterSpans.map((s) => [s.from, s.to]),
      ...bounceSpans.map((s) => [s.from, s.to]),
      ...pulseSpans.map((s) => [s.from, s.to]),
    ];
    /// 这一帧在不在（任一个）窗口跨度里（含收尾那一帧）。
    const inAnySpan = (out) => spans.some(([a, b]) => out >= a && out <= b + 1);

    // -------------------------------------------------------------------
    // **往前铺**：每个窗口的密集键要从**上一个稀疏键之后**就开始写
    // -------------------------------------------------------------------
    //
    // # 为什么（这是实测出来的一个真 bug）
    //
    // 契约的插值取相邻对 `[a, b]` 里 **b 的值与缓动**（`curve.rs::channel_from`）。
    // 所以**在某个通道上插一个新键，会把它之前的那一段也改掉** ——
    // 原来 `[706, 无]` 是"恒为 -86.5"，插了 940 之后变成"从 -86.5 线性插到 -116.5"。
    //
    // 实测症状：在 stutter 的窗口（输出帧 940 处，layer-local）插了密集键之后，
    // **它之前 234 帧**的画面被平移（x 差 4.6px、y 差 3.8px），
    // 整帧平均差 14.35、30% 的像素超阈值 —— 而那个区间**没有任何特效**。
    // 拿 YekiTrim 的成片对拍时，参照两次渲染**完全相同**（差 0.00），
    // 只有本仓这边变了，所以不是渲染不确定，也不是编码器。
    //
    // 修法：密集键**从上一个稀疏键的下一帧开始写**，一路写到窗口末尾。
    // 因为 `cameraStateAt` 已经能逐帧重放运镜曲线，那一段写进去的值就是
    // **基准曲线本身** —— 于是段结构虽然变了，曲线没变。
    //
    // 代价：每个窗口多几十到几百个键（实测这一份工程 12 个窗口共多约 2 千个）。
    // 换来的是"写密集键这件事**不再有副作用**"。
    const sparseSorted = [...sparseOutFrames].sort((a, b) => a - b);
    // 密集键往前铺的区间由**导出的纯函数**算 —— 见 `denseRangesFor` 顶层的长注释。
    // 抽出去不是为了好看：`scripts/check-yekitrim-translator.mjs` 的 `checkDenseRanges`
    // 要逐值钉"右端落在哪一帧"，而它是主流程里的内联计算时，任何守卫都看不见它。
    const denseRanges = denseRangesFor(spans, sparseSorted, lastOut);
    const inDenseRange = (out) => denseRanges.some(([a, b]) => out >= a && out <= b);

    let handheldFrames = 0;
    // §19：荷兰角逐帧计数（与 handheld 分开报，这样报告里能看出到底哪个生效了）
    let dutchFrames = 0;
    let movedFrames = 0;
    for (let out = 0; out <= lastOut; out += 1) {
      const st = cameraStateAt(out);
      const sway = handheldSway(out / fps, st.handheld);
      const shSpan = shakeSpans.find((s) => out >= s.from && out <= s.to);
      const sh = shSpan ? shakeOffsetAt((out - shSpan.from) / fps, shSpan.intensity) : null;
      const stSpan = stutterSpans.find((s) => out >= s.from && out <= s.to);
      const stu = stSpan ? stutterOffset((out - stSpan.from) / fps, stSpan.intensity) : null;
      const boSpan = bounceSpans.find((s) => out >= s.from && out <= s.to);
      const bo = boSpan ? bounceFactor((out - boSpan.from) / fps) : 1.0;
      const puSpan = pulseSpans.find((s) => out >= s.from && out <= s.to);
      const pu = puSpan ? pulseFactor((out - puSpan.from) / fps) : 1.0;

      const shx = sh ? sh.x : 0;
      const shy = sh ? sh.y : 0;
      const stx = stu ? stu.x : 0;
      const sty = stu ? stu.y : 0;
      // `finalZoom = max(zbZoom, plZoom)`（参照原式），两者都是 `curZoom × 系数`，
      // 所以等价于 `curZoom × max(系数)`。
      const zoomFactor = Math.max(bo, pu);
      // **`camScale` 同时进 `scale` 与 `cameraOffset`。**
      //
      // 参照是"绕不动点按 `camScale` 缩放"，而底座把"绕中心缩放 + 位移"折成
      // `offset = (1-z)*(origin-中心)` —— 那个推导里的 `z` **必须与 `transform.scale`
      // 是同一个值**，否则缩放与位移对不上，表现是"放大时画面往一边飘"。
      // 我先前 `scale = z*sway.scale` 而 `offset` 用的是 `z`，两者不一致。
      // ⚠️ **这里不要预乘 `FIT`**：`cameraOffset` 是通式，内部已经按 `sr` 乘过一次。
      // 预乘会让逐帧那趟变成 `FIT²`，而稀疏那趟只有一次 —— 两条路不一致。
      // 显式写进关键帧的 `transform.scale` 仍然要乘 FIT（见下面的 placeKeyframe）。
      const camScale = st.zoom * zoomFactor * sway.scale;
      // 手持与上面那些窗口不同：它是**连续**的（amp 平滑进出），
      // 所以它的判据仍然是"amp 还没回到 0"。
      // 手持是连续量（amp 平滑进出），判据仍是"amp 还没回到 0"；
      // 其余那些窗口用**往前铺过的**密集区间（见 `denseRanges`）。
      // ⚠️ **`tilt` 也必须进这个判据**（§19）。只写 `handheld || inDenseRange(out)` 的话，
      //    一段"只有荷兰角、没有别的动效"的运镜**整段被跳过** —— 转译器算出了 tilt、
      //    note 里也没有它（因为那一帧根本没走到），表现是"配了荷兰角、成片一动不动"，
      //    而且它**不报错**。荷兰角是**持续状态**，判据与 handheld 同类（连续量）。
      const moving = st.handheld > 1e-6 || Math.abs(st.tilt ?? 0) > 1e-6 || inDenseRange(out);
      if (!moving) continue;

      // ⚠️ **一对多**（重放 / 重复源区间）：同一个成片帧可能落在**多个**段上，
      // 那时每一段都要写它**自己**那份逐帧变换。只写第一段 ⇒ 第二次重放
      // 完全没有运镜/手持/抖动，而报告里一行差异都没有。
      //
      // 每段的值**可能不同**（段的起点不同 ⇒ `sourceFrameForOutput` 不同 ⇒
      // `cameraStateAt` 的状态不同），所以这里逐段重算，不能只算一次再复制。
      let wrote = false;
      for (const range of keepRanges) {
        const length = range.durFrames ?? (range.end - range.start);
        if (!(out >= range.outStart && out < range.outStart + length)) continue;
        const speed = range.speed ?? 1.0;
        const sourceFrame = range.start + Math.round((out - range.outStart) * speed);
        const base = cameraOffset(camScale, { x: st.ox / width, y: st.oy / height });
        placeKeyframe(sourceFrame, 'x', base.x + sway.dx + shx + stx, 'linear');
        placeKeyframe(sourceFrame, 'y', base.y + sway.dy + shy + sty, 'linear');
        placeKeyframe(sourceFrame, 'scale', camScale * FIT, 'linear');
        // 旋转通道 = **荷兰角（§19）+ 手持摇摆的 rot**，两者**相加**。
        //
        // ⚠️ 它们是**不同的东西、必须叠加**而不是二选一：
        //    荷兰角是"静止的持续倾斜"（幅度大、要停住），手持 rot 是"微幅抖动"
        //    （幅度小、一直在变）。写成 `if (tilt) ... else if (handheld) ...` 的话，
        //    同时开时**会丢掉一个** —— 而"丢的是哪个"取决于判断顺序，
        //    表现是"开了手持就没有荷兰角"或反过来，从界面上完全看不出来。
        //
        // ⚠️ 只要**任何一个**非零就得写：以前这里只在 handheld 时写，
        //    改成只看 handheld 会让荷兰角**永远写不进去**（转译器算对了、成片没歪）。
        const rot = (st.tilt ?? 0) + (st.handheld > 1e-6 ? sway.rot : 0);
        if (Math.abs(rot) > 1e-6 || st.handheld > 1e-6) {
          placeKeyframe(sourceFrame, 'rotation', rot, 'linear');
        }
        wrote = true;
      }
      if (!wrote) continue;
      if (st.handheld > 1e-6) {
        handheldFrames += 1;
      }
      if (Math.abs(st.tilt ?? 0) > 1e-6) {
        dutchFrames += 1;
      }
      movedFrames += 1;
    }
    if (dutchFrames > 0) {
      note(
        'camera.tilt',
        `荷兰角（§19）逐帧写进视频层的 rotation（${dutchFrames} 帧）—— ` +
          '与手持摇摆的 rot **相加**（两者是不同尺度上的倾斜，同时开时都在）',
      );
    }
    if (handheldFrames > 0) {
      note(
        'camera.handheld',
        `手持摇摆逐帧写进视频层的 x/y/scale/rotation（${handheldFrames} 帧）—— ` +
          '波形是 2 条正弦 × 4 个通道，`amp` 走与 zoom 同一套状态机',
      );
    }
    if (movedFrames > 0) {
      note(
        'events.fx',
        `密集键往前铺到上一个稀疏键（${denseRanges.length} 个窗口）—— 否则插新键会改掉它之前那一段的插值`,
      );
      note(
        'events.fx',
        `视频层的逐帧变换共 ${movedFrames} 帧（shake 四级阶梯 / stutter 两档 / ` +
          'zoom_bounce ×1.10 / pulse ×1.06·×0.97 / 手持）；这些**只作用于视频**，' +
          '字幕弹幕贴纸不跟着动，与参照 `renderFrame` 的顺序一致',
      );
    }
  }

  // **不再报「pow2_out 只能近似」** —— YekiTrim 的 `pow2_out(t) = 1-(1-t)^2`
  // 与底座的 `EaseOut` **逐值相同**（`pow2_in` 同理对上 `EaseIn`）。
  // 我先前把它报成"同族不同参"，那是错的；`schema.rs` 有逐值用例钉着。

  // ---- 原声轨 ----
  //
  // **这是第一版漏掉的最要命的一块。** 我先前只把音效转成音轨，
  // 于是成片里**原声整个没了** —— 实测源 -24.2dB、我的产物 -91.0dB（数字静音），
  // 而本仓自己的成片是 -24.3dB。
  //
  // 原因是底座的音轨是**显式**的：不写一条引用视频素材的音频层，
  // 就真的没有原声。而本仓这边原声是**默认就有的底**。
  //
  // **复用上面那套 `keepRanges`**：原声与主画面走**同一批区间**，
  // 于是两者在时间线上必然对齐（这是"音画不同步"这类错的防线）。
  // 区间在源时间与输出时间上都记着，`outStart` 直接就是输出时间轴上的位置。
  const audioTracks = [];
  {
    // ⚠️ **原声与主画面走同一套 `mainGeom`**（第 7 批 / task-10）：叠化的段
    // 在画面上提前 `overlap` 帧开演，**原声必须跟着提前** —— 不跟就是
    // "画面已经淡入 0.5 秒、声音还在上一段"，而这**不报错**（与 §11.7 ① 那条
    // "画面与音轨按两张不同的时间线剪、全程零报错"是同一种事故）。
    //
    // ⚠️ **同一轨内不许重叠**（底座 `layer_overlap`，音轨一样管）：两段音轨
    // 各提前 `overlap` 帧、上一段又没缩短 ⇒ 一定重叠 ⇒ 底座 **exit 2**。
    // 所以这里做的是**拆轨**：带叠化的那段原声落到 `a2`，与画面 `v2` 同步。
    // 代价照实说：`a0` 里那一段被"挖走"之后，**重叠窗口里两段原声同时在响**，
    // 而那正是底座会**相加**的地方（不是淡化）—— 出片那条腿不看这些音轨层
    // （`--no-audio`，声音由 `Ffmpeg::assemble_audio_from_edl_fades` 重拼、
    // 在那里做真淡化），**预览那条腿**听到的就是这个相加的结果。
    // 预览的听感与成片会**略有出入**（预览重叠区偏响）—— 这是底座的合成语义，
    // 本仓改不动；成片是准的，预览"两层都在响"至少不撒谎。
    const layers = keepRanges.map((range, index) => ({
      id: `voice-${index}`,
      // ⚠️ 用 `mainGeom[index].start`（已带叠化偏移），**不是** `range.outStart`。
      start: mainGeom[index] ? mainGeom[index].start : range.outStart,
      // ⚠️ 同主画面：EDL 段带 speed 时 `durFrames` ≠ `end - start`，必须用它，
      // 否则原声比画面短/长一截（而音画不同步这一类**不报错**）。
      end:
        (mainGeom[index] ? mainGeom[index].start : range.outStart) +
        (range.durFrames ?? (range.end - range.start)),
      // **原声从素材的对应时刻取**，不是从头。
      // `source_in` 是素材内偏移，掐掉前面的部分之后必须跟着走。
      //
      // ⚠️ 第 6 批：**素材段连原声一起指向那个素材**。只改画面不改音轨 = 那一段
      // "画面是素材、声音还是本条片段" —— 而这一类**不报错**（与规格 §11.7 ① 那条
      // "画面与音轨按两张不同的时间线剪、全程零报错"是同一种事故）。
      // ⚠️ 素材自己的音频流**帧号**同样按它自己的时基换算（下面的 `toSourceFramesFor`）。
      source: {
        asset_id: assetIdForRange(range),
        source_in: toSourceFramesFor(
          range.start,
          assetIdForRange(range) === videoAssetId ? null : range.assetId,
        ),
      },
      transform: { x: 0, y: 0, scale: 1.0, rotation: 0 },
      opacity: 1.0,
      blend: 'normal',
      enabled: true,
      effects: [],
      keyframes: [],
      note: '原声',
    }));
    // 拆轨：与画面 `v1`/`v2` **同一份 `mainGeom.track`**。
    const a1Layers = layers.filter((_, index) => !mainGeom[index] || mainGeom[index].track === 0);
    const a2Layers = layers.filter((_, index) => mainGeom[index] && mainGeom[index].track === 1);
    if (a1Layers.length > 0) {
      audioTracks.push({ id: 'a0', kind: 'audio', layers: a1Layers });
    }
    // 叠化那条原声轨：**只在真的有段落上去时才写**（无转场工程逐字节不变）。
    if (a2Layers.length > 0) {
      audioTracks.push({ id: 'a2', kind: 'audio', layers: a2Layers });
    }
    // `mute_ranges` 的 NOTE 已经在上面（算 keepRanges 的地方）报过了 ——
    // 同一件事报两遍会让清单看起来有两处要改，而那是一处。
  }

  // ---- 音效轨 ----
  if (sfxClips.length > 0) {
    // **每条音效一段，同一轨上首尾相接的才放得下。** 实测这些音效确实不重叠，
    // 所以一条轨够用；真重叠了就得分轨（同轨不许重叠）。
    const layers = [];
    let index = 0;
    for (const clip of sfxClips) {
      const assetId = sfxAssets.get(clip.name);
      if (!assetId) {
        // 文件不存在时 `registerSfx` 已经报过 DROP —— 这里**不能**再生成图层，
        // 否则会得到一条指向不存在素材的音轨（校验放行、渲染才炸）。
        continue;
      }
      const previous = layers[layers.length - 1];
      // **音量**：YekiTrim 的三处调用点都是
      //     `var vol = ev.volume || CFG.sfx.volume || 0.1;`
      // （模板 920 / 1108 / 1167 行）—— 是**覆盖**不是相乘：
      // 事件自己的 `volume` 优先，没有才用 `[sfx] volume` 那个总线值。
      // 底座 `Layer.gain` 就是它（混音器真的会乘）。
      const gain = clip.volume ?? config.sfx?.volume ?? 0.1;
      layers.push({
        id: `sfx-${index}-${clip.name}`,
        start: clip.start,
        end: clip.end,
        source: { asset_id: assetId, source_in: 0 },
        transform: { x: 0, y: 0, scale: 1.0, rotation: 0 },
        opacity: 1.0,
        // **以前这一条是丢的**：契约没有增益字段，混音器写死 1.0。
        gain,
        blend: 'normal',
        enabled: true,
        effects: [],
        keyframes: [],
        note: `音效 ${clip.name}（音量 ${gain}${clip.volume === null ? '，取自 [sfx] volume' : '，事件自带'}）`,
      });
      index += 1;
      if (previous && clip.start < previous.end) {
        clamp(
          `sfx(${clip.name})`,
          `与上一条音效重叠（第 ${clip.start} 帧 < ${previous.end}）—— 同轨不许重叠，需要分轨`,
        );
      }
    }
    if (layers.length > 0) {
      audioTracks.push({ id: 'a1', kind: 'audio', layers });
    }
  }

  // ---------------------------------------------------------------------
  // **背景层**（`drawStageBackdrop` 的复刻）
  // ---------------------------------------------------------------------
  //
  // 参照（`index.html:1828`）：
  //
  //     if (STAGE_BG === 'none') return;
  //     if (black|color) { fillRect(0,0,CW,CH); return; }
  //     var r = stageCoverRect(bmp, CW, CH);          // cover
  //     var bs = STAGE.blur_img_scale;
  //     ctx.filter = 'blur(' + STAGE.blur_px + 'px) brightness(0.5) saturate(1.15)';
  //     ctx.translate(CW/2,CH/2); ctx.scale(bs,bs); ctx.translate(-CW/2,-CH/2);
  //     ctx.drawImage(bmp, r.x, r.y, r.w, r.h);
  //
  // # 为什么要拆成两层
  //
  // 底座的**有素材图层只执行 `SeparableBlur`** —— 把 `brightness`/`saturation`
  // 挂在同一个图层上会被**整条丢掉**（现在是校验错误 `effect_would_be_ignored`）。
  // 所以：视频层只带 `gaussian_blur`，色彩用**紧跟其后的调整层** ——
  // 那时画面上只有背景，调整层作用的就是它自己，不会碰到主画面。
  //
  // # blur 的数字
  //
  // CSS 的 `blur(Npx)` 里 **N 就是标准差 σ**；底座的 `radius` 是核半径、**σ = radius/2**。
  // 所以 `radius = 2 * blur_px`（`blur_scale=1.0` 时 `blur_px=28` -> radius 56）。
  //
  // # brightness(0.5) 怎么表达
  //
  // 底座没有"乘法亮度"：`brightness` 是**加性**的，`contrast` 是"绕 0.5 乘"。
  // 于是 `0.5x` = `contrast(0.5)` 再 `brightness(-0.25)`：
  //     (x-0.5)*0.5 + 0.5 - 0.25 = 0.5x   —— 逐值相等。
  const clampScale = (v) => (Number.isFinite(v) ? Math.min(Math.max(v, 0.1), 3.0) : 1.0);
  const blurPx = Math.round(28 * clampScale(Number(config.stage?.blur_scale ?? 1)));
  const blurImgScale = Math.round((1 + blurPx / 150) * 1000) / 1000;
  const bgCover = Math.max(width / srcW, height / srcH);
  const backdropTracks = [];
  // ⚠️ **仍然暂时不发背景层，但理由变了**（2026-10-01 在新底座 `c7f29b9` 上重测）：
  //
  //   旧读数（**旧版本**）：注入一个常驻第二层 ⇒ 60/200/600/1100 帧过、**1500 帧 OOM**
  //     （去掉模糊也一样；只去掉调整层仍 OOM）⇒ 当时判断是分段路径的中间纹理逐帧累积。
  //   新读数（`c7f29b9`，同一份工程 + 竖屏 + `bg_mode=blur`，doc 里**确有** `bg1/bg2` 两条背景层
  //     与 `gaussian_blur(radius=16)`）：**200 / 1100 / 1500 帧全部渲完，没有 OOM**。
  //     ⇒ 那条泄漏在**新版上复现不出来**（交接单 §0 已按这个读数下调）。
  //
  //   **2026-10-06：产品路径验收做完了，开关打开**（下面 `BACKDROP_ENABLED = true`）：
  //     · 出片那一趟真跑 1800 帧（19.3s，峰值 WS 981MB）与 3600 帧（32.3s，峰值 977MB）**全程平**，
  //       对照"不画衬底"的 1800 帧（17.4s / 973.7MB）没有差别 ⇒ 那条泄漏确实不在新版上；
  //     · 两条像素探针都过：`portrait_backdrop`（四角最暗 85.9，拆闸后 0.0）、
  //       `portrait_stage_fit`（框外衬底亮度 99.3、纯黑 0.00%，B③ 拆闸后 99.6% 纯黑）。
  //   仍有一条**改不掉**的：底座 `gaussian_blur.radius` 上限 16（σ8），参照模糊底要 σ28 ⇒ 只能一半
  //     （打开时**如实记一条 CLAMP**，不静默近似）。
  //
  // # 实测（同一份工程、同一个范围，`--from 0`）
  //
  //   1 个常驻"有素材图层"（原工程）      -> 全片 5812 帧 **通过**
  //   2 个常驻"有素材图层"（注入一个）    -> 60/200/600/1100 帧通过，**1500 帧 OOM**
  //                                        而且**去掉模糊也一样**（不是模糊的问题）
  //
  // 阈值落在 1100~1500 帧之间 -> **逐帧增长**（每帧 ~16-33 MB，1500 帧就是 25-50 GB）。
  //
  // # 为什么是"多一层"触发的
  //
  // 单层走**非分段路径**（不分配中间纹理）；一旦有第二层就走 `render_segmented`
  // （`timeline.rs:696`，每个 stage 都 `alloc_texture` 中间纹理）—— 而那批中间纹理
  // **每帧都在涨**。所以「背景层」这条设计（常驻 + 多一层）正好踩中它。
  //
  // **底座修好之前不发**：发出去等于"某些工程能渲、长片必崩"，而且崩在别人机器上。
  // 代码留着、注释写清；修好之后把下面这个 `if (false && ...)` 的开关打开即可。
  // ⚠️ **为什么还关着**（2026-10-01 试开过一次，又关回来）：
  //
  //   · **能跑**：新底座 `c7f29b9` 上带 `bg1/bg2` 渲 200/1100/1500 帧全部通过、**无 OOM**（旧版本那条
  //     "常驻第二层长片必 OOM"复现不出来，见交接单 §0）；产品路径
  //     `yekitrim polish render --renderer dhampir` 也成功（12s / 1080x1920 / 3.2MB）。
  //   · **但"看得见"没验成**：拿一个竖画布 + `bg_mode=blur` 的工程开/关各渲一次，成片**逐像素相同** ——
  //     因为那个工程的 `axis` 走 **cover**（默认档），主画面铺满画布，**底被完全盖住** ⇒ 开关对画面无影响。
  //   · **下一步（一行夹具就能定论）**：造一个 `axis = "x"` 或 `"y"`（**contain**）+ 竖画布 + `bg_mode=blur`
  //     的工程 ⇒ 主画面留边，那条边里应当是**模糊底**（不是纯黑）。验成了再把这个开关打开。
  //
  // ⚠️ 另有一条**改不掉**的限制：底座 `gaussian_blur.radius` 上限 16 ⇒ σ8，而参照模糊底要 σ28
  // ⇒ 即便打开也**只能做到一半**，按约定如实记 CLAMP（不是静默近似）。
  // ✅ **2026-10-06 打开**：原先写 false 的理由是"底座侧模糊中间纹理逐帧累积、长片必 OOM
  // （实测 1100~1500 帧）"。现在那份泄漏**已经不在**了 —— 真跑实测（1080p、720×1280 竖素材）：
  //   · 衬底关（1 路视频层）：1800 帧 17.4s，峰值 WorkingSet **973.7MB**（全程平）；
  //   · 衬底开（+模糊配对层）：1800 帧 19.3s 峰值 **981MB**、**3600 帧（120s）32.3s 峰值 977MB** —— 同样平，
  //     没有任何逐帧增长。
  // 像素 A/B（同一帧）：衬底关时留边处 luma **0.0（黑）**；衬底开时 **123.3（模糊底）**，画面内两档都是 84.3。
  const BACKDROP_ENABLED = true;
  if (STAGE_BG === 'blur' && !BACKDROP_ENABLED) {
    clamp(
      'stage',
      'bg_mode="blur"：底座有原语、我们暂时不发 —— 配方（同源层 → 模糊调整层 → 主层）已在转译器里，' +
        '被 `BACKDROP_ENABLED` 关着（未做产品路径验收）；且底座 `gaussian_blur.radius` 上限 16 ⇒ σ8，' +
        '参照要 σ28（只能一半）。**纯色/黑不受这条影响**：它们走画布底色（`render_hints.background`）',
    );
  }
  if (BACKDROP_ENABLED && STAGE_BG === 'blur') {
    backdropTracks.push({
      id: 'bg1',
      kind: 'video',
      layers: mainLayers.map((layer, index) => ({
        ...layer,
        id: `bg-video-${index}`,
        transform: { x: 0, y: 0, scale: bgCover * blurImgScale, rotation: 0 },
        // ⚠️⚠️ **这一层会让长片渲染 OOM（底座的问题，我实测过、还没修）**。
        //
        // 它是"整条时间线常驻的、有素材的模糊图层"。实测（同一份工程、同一个范围）：
        //     · 渲 60 帧        -> 三种情况都 ok
        //     · 渲全片 5818 帧 -> 保留这一层 **OOM**（`wgpu error: Out of Memory`），
        //                          去掉它 -> 通过；只去掉调整层（留这一层）-> 仍 OOM
        // 所以触发者是**这一层**，而且**只在长跑时**出现 —— 是逐帧累积。
        // 症状在 `yekitrim-compare` 里表现为 `*rawvideo* muxer: Invalid argument`，
        // 那是本仓 panic 之后管道断掉的下游症状，**不是参照的问题**（参照渲得好好的）。
        //
        // 底座侧要查的是模糊中间纹理有没有复用。**在那之前，带背景层的长片渲不出来。**
        //
        // ⚠️ 底座的 `radius` 上限是 **16**（= σ8），而参照的模糊底要 σ=28。
        // 而且 `blur_radius()` 取的是**多个模糊里的最大值**，**不叠加** ——
        // 所以"多叠几趟凑出 σ28"这条路走不通。这里按上限发，并如实记 CLAMP。
        effects: [{ kind: 'gaussian_blur', params: { radius: Math.min(2 * blurPx, 16) } }],
        note: `背景（模糊底）第 ${index + 1} 段`,
      })),
    });
    backdropTracks.push({
      id: 'bg2',
      kind: 'video',
      layers: [
        {
          id: 'bg-adjust',
          start: 0,
          end: Math.max(...mainLayers.map((l) => l.end)),
          transform: { x: 0, y: 0, scale: 1.0, rotation: 0 },
          opacity: 1.0,
          blend: 'normal',
          enabled: true,
          effects: [
            { kind: 'contrast', params: { amount: 0.5 } },
            { kind: 'brightness', params: { amount: -0.25 } },
            { kind: 'saturation', params: { amount: 1.15 } },
          ],
          keyframes: [],
          note: '背景的色彩（复刻 CSS 的 brightness(0.5) saturate(1.15)）',
        },
      ],
    });
    if (blurPx > 8) {
      clamp(
        'stage',
        `模糊底的 blur(${blurPx}px)：底座 gaussian_blur 的 radius 上限 16（= σ8），` +
          `而参照要 σ${blurPx} —— 且多个模糊**不叠加**（取最大值），所以模糊底比参照**淡**`,
      );
    }
  }
  // **纯色/黑背景 = 底座的清屏色**（唯一不引入图层、也不碰泄漏的落地方式）。
  const stageBackground =
    STAGE_BG === 'black'
      ? '#000000'
      : STAGE_BG === 'color'
        ? (String(config.stage?.bg_color ?? '').trim() || '#000000')
        : '';
  if (stageBackground) {
    note(
      'stage',
      '背景 bg=' + STAGE_BG + '：按**画布底色**（render_hints.background = ' + stageBackground + '）发给底座 —— ' +
        `留边处就是它，不引入任何图层`,
    );
  }
  // ⚠️ 这条只对 **blur** 成立：纯色/黑走的是 `render_hints.background`（清屏色，零图层），
  // 已经落地了 —— 拿"会加图层"去吓它，就是一条过期的说明（2026-10-06 修）。
  if (STAGE_BG === 'blur' && !BACKDROP_ENABLED) {
    clamp(
      'stage',
      `背景层（bg=${STAGE_BG}）：**暂时不发** —— 它会加一个常驻的"有素材图层"，` +
        `从而触发底座的**分段路径逐帧泄漏**（实测：2 层时 1500 帧 OOM，1 层时全片通过；` +
        `去掉模糊也一样）。底座修好前，主画面之外的边是黑的` +
        (STAGE_BG === 'blur' ? '（模糊那条腿要图层，纯色那条腿走画布底色、已经能落地）' : ''),
    );
  } else if (STAGE_BG === 'color') {
    note('stage', 'bg_mode=color：走画布底色（`render_hints.background`），留边处就是它');
  }

  // ---- 组装 ----
  //
  // ⚠️ **主画面拆成 v1 / v2 两条轨（第 7 批 / task-10）**：`transition_in = "dissolve"`
  // 的段与它前一段在成片时间轴上**重叠**，而底座**同轨不许重叠**（`layer_overlap`，
  // 探针实测 exit 2）。上面 `mainGeom` 给每段算了 `track`，这里按它分组。
  //
  // ⚠️ **z 序 = track 顺序**（靠后的轨在上）：`v2` 排在 `v1` 之后 ⇒ 带叠化的那一段
  // 画在上面、`opacity` 0→1 就是"淡入盖上前一段"（探针 Q2/Q3 实测）。
  // **别把 v2 排到 v1 前面** —— 那样淡入的是底层，观感完全不同。
  // ⚠️ **没有叠化时 `v2` 是空的、直接不写这条轨**（老工程/无转场工程逐字节不变，R4）。
  const v1Layers = mainLayers.filter((_, index) => mainGeom[index].track === 0);
  const v2Layers = mainLayers.filter((_, index) => mainGeom[index].track === 1);
  const tracks = [
    // 背景在最前：铺满时被主画面整块盖住，有位移才露出来（与参照同序）。
    ...backdropTracks,
    // 主画面：裁剪之后是**每个保留区间一层**（除叠化处外首尾相接，不重叠）。
    { id: 'v1', kind: 'video', layers: v1Layers },
  ];
  // 叠化那条轨：**只在真的有段落上去时才写**。
  if (v2Layers.length > 0) {
    tracks.push({ id: 'v2', kind: 'video', layers: v2Layers });
  }
  // （覆盖层不再单独组轨：四档现在与 `flash`/`noise` 同一族，走 `adjustmentEffects` 的
  //   逐帧强度包络 —— 见上面 `case 'overlay'`。`overlayEventCount` 只用于元数据与摘要。）
  // 贴纸：**一张一张各占一条轨**。
  //
  // 底座同轨图层不许重叠（`layer_overlap`），而这些贴纸的时间窗
  // 是各自独立的（21.3-25.03、27.68-29.75……），塞一条轨要手工分段，
  // 且一改就错。每条一轨最简单、最不会错 —— 代价是轨数多，
  // 而轨数不影响渲染正确性。
  // **贴纸最后才画** —— 见下面"轨道顺序"那一整段。
  const stickerTracks = [];
  for (const sticker of stickerLayers) {
    stickerTracks.push({
      id: `s${sticker.id}`,
      kind: 'video',
      layers: [{
        id: sticker.id,
        start: sticker.start,
        end: sticker.end,
        transform: sticker.transform,
        opacity: 1.0,
        blend: 'normal',
        enabled: true,
        source: sticker.source,
        // **这一行漏过。** 上面构造 stickerLayers 时写了 `loop_source: true`，
        // 但组装轨道时是**重新拼一个对象**，没带过来 ——
        // 表现是校验照旧报 `source_range_exceeded`，而看代码"明明写了"。
        // 这种"构造时写了、组装时丢了"是纯手工拼对象最容易出的错。
        loop_source: sticker.loop_source === true,
        effects: [],
        // 贴纸的动效（弹出/上浮/淡出）走关键帧 —— 同样别在这里丢了。
        keyframes: sticker.keyframes ?? [],
        note: sticker.note,
      }],
    });
  }

  // 调整图层：**一条特效一条轨**，紧跟在主画面之后。
  //
  // 调整图层作用于「**已经画上去的东西**」（`plan_steps` 里它会把累计的
  // `Draw` 冲掉再 `Adjust`），所以**位置就是它的作用域**。
  //
  // 为什么紧跟主画面：**YekiTrim 的出片顺序是**
  //
  //     视频 -> noise -> flash -> hue_shift -> overlay -> 字幕 -> 弹幕 -> 贴纸
  //
  // （`templates/index.html` `renderFrame`，逐行核过）——
  // 于是这些调整**只作用在视频上**，贴纸与字幕弹幕在它们之后画，不受影响。
  //
  // 我先前按 **DOM** 的层级推断，把调整图层放到最后（"贴纸就在 #video-wrap 里"）——
  // 那是预览的结构，**不是出片的顺序**。出片认的是 canvas 那一条。
  //
  // 为什么一条一轨：同轨图层不许重叠（`layer_overlap`），而各特效的窗口是
  // 独立的 —— 塞一条轨要手工分段，一改就错。与贴纸同一条理由。
  for (const adjustment of adjustmentEffects) {
    tracks.push({
      id: `adj-${adjustment.id}`,
      kind: 'video',
      layers: [{
        id: adjustment.id,
        start: adjustment.start,
        end: adjustment.end,
        transform: { x: 0, y: 0, scale: 1.0, rotation: 0 },
        opacity: 1.0,
        blend: 'normal',
        enabled: true,
        // **没有 `source`** —— 这正是"调整图层"的定义（`Layer::is_adjustment`）。
        effects: [adjustment.effect],
        // 逐帧强度包络（大多数调整图层是空数组 = 恒为 1.0）。
        keyframes: adjustment.opacityKeyframes,
        note: `调整图层：${adjustment.effect.kind}（作用于它下面的一切）`,
      }],
    });
  }

  // 字幕与弹幕轨 —— 用底座自己的子系统，把 YekiTrim 的素材接进来。
  const subtitleTracks = [];
  // 字幕轨的眼睛关着 ⇒ **整条不转**（素材与 `data/subtitles.json` 一个字都没动）。
  if (laneOn('subtitles') && config.source?.subtitles && existsSync(join(clipDir, config.source.subtitles))) {
    const id = config.source.subtitles;
    // **剪过的字幕要另写一份。**
    //
    // 源 `clip.srt` 的时间是**源时间轴**的；主画面裁剪之后时间轴变了，
    // 直接引用它会让字幕**从第一刀起整体错位**（最大 17.49 秒）。
    // 所以解析出来、重映射、写成 `dhampir-subtitle.srt` 放在工程文件旁边。
    const rawCues = parseSrt(readFileSync(join(clipDir, id), 'utf8'));
    const remapped = [];
    let straddled = 0;
    for (const cue of rawCues) {
      const pieces = remapCueMs(cue.startMs, cue.endMs);
      if (pieces.length === 0) continue;
      if (pieces.length > 1) straddled += 1;
      for (const piece of pieces) remapped.push({ ...piece, text: cue.text });
    }
    const subName = 'dhampir-subtitle.srt';
    const subPath = join(dirname(outPath ?? clipDir), subName);
    mkdirSync(dirname(subPath), { recursive: true });
    writeFileSync(subPath, cuesToSrt(remapped), 'utf8');
    note(
      'source.subtitles',
      `${rawCues.length} 条字幕 -> 重映射成 ${remapped.length} 条（${subName}）；` +
        (straddled > 0 ? `其中 ${straddled} 条跨了剪辑点、被切成两段` : '没有跨剪辑点的'),
    );
    assets.push({
      id: subName,
      kind: 'subtitle',
      name: '字幕',
      // 绝对路径（与弹幕的 ASS 同理：文件在工程文件旁边，不在 clip 目录里）。
      uri: subPath,
      frame_count: null,
      timebase,
      width: null,
      height: null,
      content_hash: null,
      tags: {},
      note: `由 ${id} 重映射到裁剪后的时间线`,
    });
    // 字幕样式：**从 YekiTrim 的模板抄实际值**，不能用缺省。
    //
    // YekiTrim 的 CSS（`templates/index.html:747`）：
    //   font-size: 72px            -> font_ratio = 72/1080 = 0.0667
    //   color: #dcbda0             -> [220,189,160,255]
    //   -webkit-text-stroke: 12px #403c3b  -> 底座只有 `outline: bool`
    // 落点：画布绘制用 `cy = CH - 120*CKH`（同文件 1966 行）
    //   -> bottom_margin = 120/1080 = 0.1111
    const subStyle = config.style?.subtitle ?? {};
    const subSizePx = typeof subStyle.size === 'number' ? subStyle.size : 72;
    const subColor = hexColor(subStyle.color) ?? { r: 1, g: 1, b: 1 };
    // 描边 YekiTrim 写的是 `"12px #403c3b"` —— 一个字符串里两个值。
    const subStroke = parseStroke(subStyle.stroke, 12, '#403c3b');
    // 行高：YekiTrim 的字幕 CSS 是 `line-height:1.5`。
    const subLineHeight = 1.5;
    // **落点要换算，不能直接搬 120。**
    //
    // YekiTrim 画的时候用的是 `cy = CH - 120` —— 那是**文字的中心**；
    // 而底座的 `bottom_margin` 是**行盒底边**到底边的距离。两者差半个行盒。
    //
    // 直接填 `120/1080` 会让字幕整体**高约 54px**（行盒 = 72*1.5 = 108，一半是 54）。
    // 实测 55s 那一帧的字幕带差从 34.16 降到 26.42，整体 16.07 -> 14.42。
    const subLineBoxPx = subSizePx * subLineHeight;
    // **垂直落点**：`position_pct` 存在时按百分比，否则走上面那条历史公式。
    //
    // ⚠️ **区分 `undefined` 与"写了 12"**：`SubtitleStyle.position_pct` 是 `Option<f64>`，
    // **缺省 = 老工程没写过** ⇒ 必须走历史公式、逐字节不变（存量零回归 R4）。
    // `120/1080 = 11.11%`，与用户可见的 "12%" 并不相等 —— 拿 % 去表达"等于今天"
    // 必然留一点偏差，所以这里**只在显式写了才改道**。
    //
    // 两个口径的换算：设置页/目标口径是**文字带底边**距画布底（与 `bottom_margin` 同义），
    // 而 `bottom_margin` 要的是**行盒底边** ⇒ 直接用 `pct/100`，**不要再减半个行盒**。
    const subPosPct = typeof subStyle.position_pct === 'number' && Number.isFinite(subStyle.position_pct)
      ? Math.min(50, Math.max(0, subStyle.position_pct))
      : null;
    const subBottomMargin = subPosPct === null
      ? (120 - subLineBoxPx / 2) / height
      : subPosPct / 100;
    subtitleTracks.push({
      id: 'sub',
      kind: 'subtitle',
      subtitle: {
        font_ratio: subSizePx / height,
        bottom_margin: subBottomMargin,
        // **3**：参照的 `wrapCaptionParts` 里 `MAX_LINES = 3`。
        // ⚠️ 那个 3 **不是截断阈值** —— 参照从不丢行，它只出现在缩字公式里
        // （"缩到恰好三行装得下"）。这里写 3 是配合下面的缩字，
        // 让"缩完装得下"成立；真正的"不丢行"由 `keep_all_lines` 保证。
        max_lines: 3,
        color: [
          Math.round(subColor.r * 255),
          Math.round(subColor.g * 255),
          Math.round(subColor.b * 255),
          255,
        ],
        outline: true,
        // **描边口径：CSS 居中 -> 底座的外侧宽度，要折半。**
        //
        // YekiTrim 写 `12px`，那是 CSS `-webkit-text-stroke` 的值 —— 判据在字的
        // 轮廓上、**里外各一半**；而 ffmpeg 的 `borderw` 是全在外侧。
        // 直接填 12 会让成片的描边粗一倍（实测 55s 那一帧差 +0.98，
        // 而 4px 跟 6px 几乎一样好，说明 6px 附近就是对的）。
        stroke_ratio: subStroke.widthPx / 2 / height,
        stroke_color: subStroke.color,
        // **淡入淡出**（YekiTrim `getActiveSubs`：fadeIn 0.35 / fadeOut 0.18、
        // 入场从下方 +20px 浮上来、退场向上 -8px）。
        fade_in_ms: Math.round(0.35 * 1000),
        fade_out_ms: Math.round(0.18 * 1000),
        rise_in_px: 20,
        rise_out_px: 8,
        // **字重与行高**：YekiTrim 的 CSS 写死了
        //   `.cap { font-weight:700; line-height:1.5; }`
        // 底座先前两样都不设（400 / 1.2）—— 笔画比它细一圈，两行字幕的行距也窄。
        font_weight: 700,
        line_height: subLineHeight,
        // **文字阴影（B5，2026-10-01 接入）**：参照是 `.cap` 的
        //   `text-shadow: 0 2px 12px rgba(0,0,0,0.4)`（`templates/index.html` 的样式块）。
        //
        // ⚠️ 三处口径要一起看，免得以后有人"优化"掉其中一处：
        //   ① **模糊半径按文档高折算**（`shadow_blur_ratio`，底座求值层乘目标高 —— 与
        //      `stroke_ratio` → `stroke_px` 同一处换算），所以这里除以 `height`（**文档高**，不是宽）；
        //   ② **dx/dy 是文档像素、不再二次换算**（底座说明也写了）；
        //   ③ 底座"给不给颜色"才是判据：`None` 与"全透明"都走老路 ⇒ 我们要画就必须给颜色。
        // 12px ÷ 1080 = 0.011111（底座回执里的示例值 0.0111 就是这一档）。
        // ⚠️ **硬阴影要显式写 0**（不许省略）：省略与 0 观感相同但**形状不同**，会让下游判据分叉。
        shadow_color: [0, 0, 0, Math.round(0.4 * 255)],
        shadow_dx_px: 0,
        shadow_dy_px: 2,
        shadow_blur_ratio: 12 / height,
        // **换行安全宽 87.5%**：YekiTrim 写死 `wrapCaptionParts(parts, CW * 0.875, …)`。
        // 底座先前的默认是"复用 bottom_margin"，凑出来 88% —— 碰巧接近，
        // 于是在刚好卡边界的行上断点会不同。
        safe_width_ratio: 0.875,
        // **装不下就整体缩字号**，下限 0.7 —— 参照：
        //
        //     if (tw > maxLineW + EPS) scale = Math.max(0.7, Math.min(1, maxLineW * 3 / tw));
        //
        // 不写这条的话，长字幕是**直接丢行**（底座的默认行为是"超出就丢"），
        // 而参照是"缩小了全画出来" —— 两者一眼就能看出区别。
        shrink_min_scale: 0.7,
        // **不截断行数。** 参照的闭式只保证 `总宽 × scale == 可用宽 × 3`，
        // 而贪心折行每行会浪费一点，**实际会折出 4 行**；参照那边不丢，
        // 是因为它根本不截断（`wrapCaptionParts` 把折出来的全画了）。
        // 实测：140 个全宽字、`max_lines = 3`、`shrink_min_scale = 0.7` 时
        // 折出 4 行、丢 1 行；置 true 才与参照一致（丢 0）。
        keep_all_lines: true,
        // **高亮词的颜色**：参照 `var hlColor = subS.highlight_color || '#f56b41'`。
        //
        // 只在字幕文本里真的有 `<span class="hl">` 时才起作用（参照也是 ——
        // 它那个变量一直是定义着的，只是没标记就没有段会用到它）。
        // 所以这里**无条件写**：写了但没有标记 = 什么都不发生。
        highlight_color: [245, 107, 65, 255], // #f56b41
        // **字体族**：`polish.toml` 里写的是 `font = "LXGW WenKai"`，而
        // **那台机器上没装** —— 它走到 CSS 的字体栈兜底
        // （`'LXGW WenKai','Noto Sans SC','Microsoft YaHei',sans-serif`），
        // 实际生效的是**微软雅黑**。名字如实带进契约，宿主找不到时会报出来。
        //
        // ⚠️ 但**库里那支要换成真族名**（`fontFamilyFor`）：底座的 libass 按族名找字体，
        // 而 `polish.toml` 存的是**库 id**。实测同一份字体、同一个 `fontsdir`：
        // 传 id `GenJyuuGothic-Bold` → `fontselect` 落 **ArialMT**；传真族名
        // `Gen Jyuu Gothic Bold` → 命中。落 ArialMT 时 `lines_failed: 0`、`issues: []`
        // —— 比 `.notdef` 还隐蔽，所以这一步不能省。
        font_family: fontFamilyFor(subStyle.font),
      },
      layers: [{
        id: 'cue',
        start: 0,
        end: durationFrames,
        source: { asset_id: subName, source_in: 0 },
        transform: { x: 0, y: 0, scale: 1.0, rotation: 0 },
        opacity: 1.0,
        blend: 'normal',
        enabled: true,
        effects: [],
        keyframes: [],
        note: '整条时间线的字幕层（时间是裁剪后的）',
      }],
    });
  }

  // ---- 弹幕轨 ----
  //
  // YekiTrim 的弹幕是 `clip.dm.srt`（SRT 外壳 + ASS 覆盖标签）。
  // 底座要的是**真 ASS**，所以转一道写成 `dhampir-danmaku.ass`。
  //
  // 转出来的文件放在**工程文件旁边**，不是 clip 目录里 ——
  // 它是转译的产物，不该污染源素材目录。
  const danmakuTracks = [];
  let danmakuCueCount = 0;
  const danmakuFile = config.source?.danmaku;
  // 弹幕轨的眼睛关着 ⇒ **整条不转**（素材一个字都没动）。
  if (laneOn('danmaku') && danmakuFile) {
    const sourcePath = join(clipDir, danmakuFile);
    if (!existsSync(sourcePath)) {
      drop('source.danmaku', `弹幕素材 ${danmakuFile} 不存在`);
    } else {
      const rawCues = parseSrt(readFileSync(sourcePath, 'utf8'));
      if (rawCues.length === 0) {
        drop('source.danmaku', `${danmakuFile} 里一条 cue 都解析不出来`);
      } else {
        // **时间要重映射** —— 与字幕同理：主画面裁剪之后，源时间轴上的
        // 弹幕时刻整体错位（最大 17.49 秒），不重映射就"弹幕对不上话"。
        // ---- 逐条的滚动时长（**先算，后面要挂到每条 cue 上**）----
        //
        // `yekitrim-polish/src/parse/text.rs:74`：
        //
        //     let travel = (1080.0 + entry.text.len() as f64 * 32.0) / 150.0;
        //
        // 单位是**秒**，`text.len()` 是 **UTF-8 字节数**（不是字符数 ——
        // 一个汉字 3 字节，这个坑值得记）。
        //
        // **必须先剥掉 ASS 覆盖标签再数字节。**
        //
        // 这是个真踩到的坑：`rawCues` 的 `cue.text` **带着** `{\move(...)}`
        // 那些标签（`parseSrt` 只切时间与正文，不剥标签），于是
        // `Buffer.byteLength` 把标签字符也算进"文本长度"了 ——
        // 39 字节的句子被数成 61 字节，travel 从 15.52s 变成 20.21s。
        //
        // 症状很隐蔽：**四个值一起偏大**，看起来像"公式选错了"，
        // 而不像"多算了几个字符"。而它还会把轨道级那个平均值一起带偏
        // （契约里那个 `duration_ms` 就是这么来的）。
        //
        // **与素材里的 `data/danmaku.json` 逐条对过，剥标签后完全吻合**：
        //   小雨现在跟很多人关系不错的  39 字节 -> 15.52s（数据里 15.52）
        //   还能续约吗                 15 字节 -> 10.40s（数据里 10.4）
        //   我是小雨直接3d了           23 字节 -> 12.11s（数据里 12.1067）
        //   困在雨季                   12 字节 ->  9.76s（数据里 9.76）
        const travelText = (raw) => String(raw).replace(/\{[^}]*\}/g, '').trim();
        const travels = rawCues.map(
          (cue) => (1080.0 + Buffer.byteLength(travelText(cue.text), 'utf8') * 32.0) / 150.0,
        );
        const meanTravel = travels.reduce((sum, value) => sum + value, 0) / travels.length;
        const dmDurationMs = Math.round(meanTravel * 1000);

        // **弹幕时间偏移**（`[style.danmaku] offset_secs`，默认 -3 = 整体提前 3s）。
        //
        // 录播姬的弹幕时间戳普遍比画面晚几秒（用户在直播回放里发弹幕的时机
        // 滞后于说话），精修时把弹幕整体提前几秒更贴合内容。偏移落在**源
        // 时间轴**上 —— 先平移、再走下面的 `remapCueMs` 裁剪重映射，被裁掉
        // 的部分语义与手改弹幕文件一致。与预览两条腿（出片模板的 DM 注入点、
        // `engine.js::buildIndex`）同口径：三处各自在入口平移一次，sign 一致
        // （负 = 提前），`isFinite` 兜住坏值回默认。
        const dmOffsetRaw = config.style?.danmaku?.offset_secs;
        const dmOffsetSecs = typeof dmOffsetRaw === 'number' && Number.isFinite(dmOffsetRaw)
          ? dmOffsetRaw
          : -3.0;
        const cues = [];
        let straddled = 0;
        for (let index = 0; index < rawCues.length; index += 1) {
          const cue = rawCues[index];
          const pieces = remapCueMs(cue.startMs + dmOffsetSecs * 1000, cue.endMs + dmOffsetSecs * 1000);
          if (pieces.length === 0) continue;
          if (pieces.length > 1) straddled += 1;
          // **travel 跟着 cue 走**（一条被裁成两段时，两段用同一个 travel ——
          // 它就是"这条弹幕滚多快"，与裁不裁无关）。
          for (const piece of pieces) {
            cues.push({ ...piece, text: cue.text, travelMs: Math.round(travels[index] * 1000) });
          }
        }
        danmakuCueCount = cues.length;
        const assName = 'dhampir-danmaku.ass';
        const assPath = join(dirname(outPath ?? clipDir), assName);
        const dmStyle = config.style?.danmaku ?? {};
        const dmSizePx = typeof dmStyle.font_size === 'number' ? dmStyle.font_size : 42;

        // **去掉逐条的 `\c` 颜色标签 —— YekiTrim 不认它。**
        //
        // 素材里确实有：`{\pos(960,1040)}{\c&HFF3FE3}还能续约吗`。但 YekiTrim 的
        // 渲染那一侧**只读轨道级颜色**（`templates/index.html:2006`）：
        //
        //     var color_dm = dmS.color || '#ffffff';      // 一份，给所有弹幕用
        //     dms_a.forEach(function(dm) { ... ctx.fillText(dm.text, dm.x, dm.y) });
        //
        // `getActiveDms` 的返回值只有 `{text, opacity, x, y}` —— **没有颜色**。
        // `\pos` 与 `\c` 在这个渲染器里都是残留（前者我早就记下了）。
        //
        // 我先前把 `\c` 当成了"YekiTrim 需要逐条颜色"的证据（那份 `danmaku.json`
        // 里确实有 `color` 字段），于是**接了个看着对、其实与参照相反的东西**：
        // 出片一对比 —— 48.8s 那一帧 YekiTrim 是白的、底座是品红的。
        //
        // 契约里的逐条颜色是**对的通用原语**（ASS 的 `\c` 就是这个意思），
        // 所以保留；**转译器不写它**才是"复现 YekiTrim 的行为"。
        // ⚠️ **2026-10-01 补充**：底座现在**能**显示逐条颜色了（回执 §5：wasm 的条目清单补齐了
        // `color`/`font_ratio`/`scale`/`parts`，预览侧也收得到）——**更不许顺手打开**：
        // 我们这条链的参照是**本仓 canvas**，它不读逐条颜色（`getActiveDms` 的返回值只有
        // `{text, opacity, x, y}`）；开了就是"底座比参照更花"，上面那次 48.8s 的品红/白就是证据。
        //
        // 只摘 `\c&HBBGGRR&` 这一条指令，**不整块删** ——
        // 标签块里常常还有别的（`{\pos(...)\c&H...}` 这种）。
        // 摘空了的块（`{}`）顺手去掉，免得留一堆空标签给下游解析。
        const stripColorTags = (text) => text
          .replace(/\\[0-9]?c&H[0-9A-Fa-f]{6}&?/gi, '')
          .replace(/\{\s*\}/g, '');
        const stripped = cues.filter((cue) => stripColorTags(cue.text) !== cue.text).length;
        if (stripped > 0) {
          note(
            'style.danmaku',
            `逐条颜色 \\c：素材里 ${stripped} 条带颜色标签，YekiTrim 的渲染器**不读它**` +
              '（只读轨道级 color）—— 已去掉，与参照保持一致',
          );
          for (const cue of cues) cue.text = stripColorTags(cue.text);
        }

        const ass = cuesToAss(cues, {
          font: dmStyle.font ?? 'Sans',
          fontSize: dmSizePx,
        });
        mkdirSync(dirname(assPath), { recursive: true });
        writeFileSync(assPath, ass, 'utf8');

        assets.push({
          id: assName,
          kind: 'subtitle',
          name: '弹幕',
          // **绝对路径。** 这个 ASS 是转译的产物、住在工程文件旁边，
          // 而 `--asset-root` 通常指向 clip 目录 —— 用相对 uri 会去 clip 目录里找，
          // 找不到（实测报 `os error 2`）。宿主对绝对 uri 是直接用的
          // （`build_sources`：`raw.is_absolute()` 就不挂 asset_root）。
          uri: assPath,
          frame_count: null,
          timebase,
          width: null,
          height: null,
          content_hash: null,
          tags: {},
          note: `由 ${danmakuFile} 转成 ASS（${cues.length} 条）`,
        });
        danmakuTracks.push({
          id: 'dm',
          kind: 'danmaku',
          danmaku: {
            asset_id: assName,
            // YekiTrim 的弹幕：`font-size: 42px`（模板 760 行）
            //   -> font_ratio = 42/1080 = 0.0389
            //
            // `duration_ms` 是**每条自己算**的（见下面 travels 的推导），
            // 而底座这一项在**轨道级**、一份素材一个值 —— 所以取平均并报出范围。
            // 分成多条轨是**不行的**：泳道分配是**每轨各自从 0 开始**
            // （`overlay.rs:203` 在 track 循环里调 `layout_danmaku`），
            // 拆开会把每条都塞进第 0 泳道、叠成一坨。
            lanes: 8,
            duration_ms: dmDurationMs,
            font_ratio: dmSizePx / height,
            // **颜色**：YekiTrim 弹幕是 `#ffffff`。以前底座没有这个字段，
            // 只能与字幕**共用**一份 —— 而字幕是暖色 `#dcbda0`，必然错一个。
            color: rgbaBytes(hexColor(dmStyle.color) ?? { r: 1, g: 1, b: 1 }),
            // **基础不透明度 0.9**（YekiTrim 模板 1503 行 `var opacity = 0.9`）。
            opacity: 0.9,
            // **淡入淡出**（同模板 1495 行：fadeIn 0.3 / fadeOut 0.2）。
            fade_in_ms: DANMAKU_FADE_IN_MS,
            fade_out_ms: Math.round(0.2 * 1000),
            outline: true,
            // 同字幕：CSS 的居中描边折半成"外侧宽度"。
            stroke_ratio: parseStroke(dmStyle.stroke, 2, '#000000').widthPx / 2 / height,
            stroke_color: parseStroke(dmStyle.stroke, 2, '#000000').color,
            // **泳道带的几何**。YekiTrim 的 `assign_danmaku_tracks` 用
            // `TRACK_YS = [300, 500, 700, …]`，那是 **1920 宽的日志坐标**；
            // 模板里 `y = (d.y/1920) * (540*CK)` 再除以画布高，归一化之后
            // 就是 `d.y / 3840` —— 与画布尺寸无关，所以直接写这两个数。
            //
            // 底座老规则是"0 号泳道贴顶、间距取行盒高"（0 起、0.048），
            // 而 YekiTrim 是 0.0781 起、间距 0.0521 —— 差值是肉眼可见的一整条。
            lane_top_ratio: 300 / 3840,
            lane_spacing_ratio: 200 / 3840,
            // YekiTrim 的弹幕 CSS：`font-weight:600`。
            font_weight: 600,
            line_height: 1.5,
            // 同字幕那条：库里那支换成真族名（弹幕空值 = 跟随字幕，转译器已经在
            // 上面把空值填成字幕那支了，所以这里拿到的多半非空）。
            font_family: fontFamilyFor(dmStyle.font),
          },
          layers: [{
            id: 'shots',
            start: 0,
            end: durationFrames,
            source: { asset_id: assName, source_in: 0 },
            transform: { x: 0, y: 0, scale: 1.0, rotation: 0 },
            opacity: 1.0,
            blend: 'normal',
            enabled: true,
            effects: [],
            keyframes: [],
            note: '整条时间线的弹幕层',
          }],
        });
        note(
          'source.danmaku',
          `${cues.length} 条弹幕从 ${danmakuFile} 转成 ${assName}（写到工程文件旁边）；` +
            `在屏窗口末尾抬到 max(素材 End, start + ${DANMAKU_FADE_IN_MS}ms + travel)：` +
            '素材 End 短于"滚完时刻"时，参照与底座都会**没到左边缘就渐隐**（录播姬 XML 的 8 秒' +
            '窗口对中文弹幕普遍不够）—— 抬到滚完为止，明细见 danmakuExitMs',
        );
        if (config.style?.danmaku?.speed) {
          // ⚠️ 这条先前写的是"逐条时长**要动契约**" —— **过时了**。
          // 后来发现 ASS 的 `\move` 支持 6 参数版（显式起止时刻），
          // 于是逐条时长**没动契约**就接上了：`\move(x1,y1,x2,y2,0,travelMs)`。
          // 留这一条是因为**轨道级那个平均值仍然写着**（它是回退值，读不到素材时才用）。
          note(
            'style.danmaku',
            `滚动时长逐条算出来是 ${travels.map((t) => t.toFixed(2)).join('/')} 秒（按文本字节数变）；` +
              `底座的轨道级 duration_ms 取了平均 ${meanTravel.toFixed(2)}s（${dmDurationMs}ms）—— ` +
              '它现在是**回退值**：每条 cue 的逐条时长写进了 ASS 的 6 参数 `\\move`，底座优先读那个',
          );
        }
        note(
          'style.danmaku',
          `highlight_color=${config.style?.danmaku?.highlight_color ?? '?'}：` +
            '底座弹幕只渲染成统一颜色（素材里的 `\\c` 标签会被剥掉），高亮色这一项没接',
        );
      }
    }
  }

  // ---------------------------------------------------------------------
  // 轨道顺序 = 绘制顺序（`plan_steps` 按数组顺序走）
  // ---------------------------------------------------------------------
  //
  // 对齐 YekiTrim **出片**那条腿（`renderFrame`）：
  //
  //     v1(主画面) -> 调整图层 -> 字幕 -> 弹幕 -> 贴纸
  //
  // **贴纸排在最后**是这一轮核出来的：`renderFrame` 里 `getActiveSticker` 在
  // 字幕（L1948）与弹幕（L2003）**之后**（L2029）。我先前按 DOM 层级把它排在
  // 字幕之前 —— 两者只有在"贴纸与字幕在时间上重叠"时才看得出区别。
  //
  // 音频不参与绘制顺序，放最后只为读起来清楚。
  // 文字元素 -> 每档预设一条轨（§5 E3）。
  //
  // ⚠️ **按 `style` 分组**而不是"一条元素一条轨"：同一档的样式完全相同，
  //    分成多条轨会产生一堆 id 不同、内容等价的轨道（底座每多一条轨就多一趟合成）。
  //    ⚠️ 但**不同档绝不能合并** —— 那是"大招与吐槽用同一个字号"。
  const textTracks = [];
  {
    const byStyle = new Map();
    // ⚠️⚠️ **T2 打字机（§5 E4a）在这里拆 cue** —— 拆完再分组，
    //    否则下面"按 style 取 cues[0].style"那条会拿到拆之前的那一条。
    //
    // ⚠️ **为什么是拆 cue、不是 ASS 的 `\k` 覆盖标签**：底座解析字幕时会把
    //    `{...}` 整块剥掉（`subtitle.rs::ass_text_to_plain`，只留颜色与 `\move`）
    //    ⇒ `\k`（libass 原生卡拉OK）**根本到不了 libass**，那条路是死的。
    //    拆 cue 是唯一可行的：不改底座一行，且**成片与预览读的是同一份 cue 表**，
    //    不会分叉（规格 §4.1 那条硬约束）。
    const elementsCfg = readElementsCfg(config);
    const tw = elementsCfg.typewriter === true;
    if (tw) {
      const before = textCues.length;
      const split = [];
      for (const cue of textCues) {
        for (const piece of splitTypewriter(
          { start: cue.start, end: cue.end, text: cue.text },
          fps,
          elementsCfg,
        )) {
          split.push({ ...cue, start: piece.start, end: piece.end, text: piece.text });
        }
      }
      textCues.length = 0;
      textCues.push(...split);
      note(
        'elements.typewriter',
        `打字机（T2）：${before} 条文字元素拆成 ${split.length} 条逐字 cue` +
          `（${elementsCfg.chars_per_sec} 字/秒 + ${elementsCfg.hold_secs}s 静止尾巴）。` +
          '⚠️ 逐帧计算、无累积状态 ⇒ seek 到第 N 帧与播到第 N 帧同形',
      );
    }

    for (const cue of textCues) {
      const key = cue.style.name;
      if (!byStyle.has(key)) byStyle.set(key, []);
      byStyle.get(key).push(cue);
    }
    // ⚠️ 遍历**固定顺序**（不是 Map 的插入序）：同一份工程跑两次要逐字节相同。
    for (const styleName of ['hook', 'chapter', 'comment', 'count']) {
      const cues = byStyle.get(styleName);
      if (!cues || cues.length === 0) continue;
      const st = cues[0].style;
      const id = `text_${styleName}`;
      // ⚠️ **复用弹幕那条 `cuesToAss`**，不自己造一个 ASS 生成器：
      //    自带生成器要重新处理 `[Script Info]` / `Dialogue` 格式 / 转义 /
      //    时间格式四件事，而它们都已经在那条里被验过了（有单测 + 真出片）。
      //    重复实现 = 两套转义规则，迟早分叉。
      const ass = cuesToAss(
        cues.map((c) => ({
          startMs: (c.start / fps) * 1000,
          endMs: (c.end / fps) * 1000,
          // ⚠️ ASS 里 `\n` 是硬换行 —— 排版那一步已经折好行了
          //    （`TextLayout::solve` 的 `lines`），但它的结果**没有传下来**
          //    （底座重排一次，用 `safe_width_ratio` 与 `shrink_min_scale` 约束）。
          //    这里把原始文本原样发出，让底座按同一套常量排 —— 两边口径一致
          //    （`safe_width_ratio` 0.875 就是 `CAPTION_SAFE_WIDTH_RATIO`）。
          text: c.text,
          travelMs: 0,
        })),
        // ⚠️⚠️ **`danmaku: false` 不能省** —— 文字元素不滚，
        //    弹幕那条"至少滚到出画"的出点规则会把每条 cue 抬到 300ms，
        //    打字机拆出来的短 cue 于是**全部重叠**（详见 `cuesToAss` 的文档）。
        { font: 'Sans', fontSize: st.fontPx, danmaku: false },
      );
      const assPath = join(clipDir, `dhampir-${id}.ass`);
      writeFileSync(assPath, ass, 'utf8');
      assets.push({
        id,
        kind: 'subtitle',
        name: `文字-${styleName}`,
        uri: assPath,
        frame_count: null,
        timebase,
        width: null,
        height: null,
        content_hash: null,
        tags: {},
        note: `§5 E3 文字元素（${styleName}），共 ${cues.length} 条`,
      });
      textTracks.push({
        id,
        kind: 'subtitle',
        subtitle: {
          font_ratio: st.fontPx / height,          // ⚠️⚠️ **`bottom_margin` 不是"文字的位置"，它是底座算「左右安全宽」的输入。**
          //
          // 这里原先写 0.5（注释说"居中当中立值"）—— 那让**每一条文字元素都静默消失**。
          // 底座的 `text_layout::layout`：
          //
          //     let margin = style.bottom_margin.clamp(0.0, 0.5);
          //     let max_width_px = sequence_width * (1.0 - 2.0 * margin);
          //
          // 0.5 ⇒ `max_width_px = 宽度 × 0 = 0` ⇒ 函数**提前返回空布局**，
          // 一个字都不排。★ 而报告里**一行差异都没有**（转译成功、轨道也在、
          // `probe` 零 error/warning），只有真去数像素才发现成片里没有那几个字。
          // 实测：`--versions` 那个工程改成 0.5 之后，文字位置的黄色像素数 **0**
          // （字幕自己的 `hl_color` 有 27k 像素，形状还长得像，极易误判成"画出来了"）。
          //
          // ⇒ 用**字幕同一条口径**：底座那个 `safe_width_ratio` 字段的文档自己写着
          //   "本仓先前拿 `bottom_margin` 当左右边距用…转译器按参照显式写 0.875"，
          //   但 `layout()` **今天仍然只读 `bottom_margin`**（`safe_width_ratio`
          //   只在缩字那条路上被读到）。所以这里**必须**给 `bottom_margin` 一个
          //   能让安全宽成立的值：`margin = (1 - 0.875) / 2 = 0.0625`。
          //   ⚠️ 纵向位置**不受它影响** —— 那是 `pos` + ASS 对齐码的事（见 `cuesToAss`）。
          bottom_margin: (1 - 0.875) / 2,
          max_lines: 3,
          color: st.color,
          outline: true,
          // ⚠️ 描边口径与字幕**同一条**：CSS 居中 -> 底座外侧宽度要折半。
          stroke_ratio: st.strokePx / 2 / height,
          stroke_color: st.strokeColor,
          fade_in_ms: 0,
          fade_out_ms: 0,
          rise_in_px: 0,
          rise_out_px: 0,
          font_weight: st.weight,
          // ⚠️ **不缩字号**：文字的字号是四档预设**定死**的（那是它的语义），
          //    像字幕那样"装不下就缩"会让 `hook` 缩成 `comment`。
          shrink_min_scale: 1.0,
          // ⚠️ **也不截断**：排版那一步（Rust `TextLayout::solve`）已经按
          //    `CAPTION_MAX_LINES` 折好并截过，这里再截一次是重复裁剪。
          keep_all_lines: true,
          safe_width_ratio: 0.875,
          // ── T1 弹入 / T4 抖动 / T6 底衬（2026-10-11，§5 E4a）──
          //
          // ⚠️ **只写配了的键**（`motionFields` 里 `undefined` 一律不写）：
          //    不配 ⇒ 底座工程**一个键都不多** ⇒ 存量工程产物逐字节不变（R4）。
          //
          // ⚠️ 字段名**逐字照底座接口文档**，别自己起名：
          //    T1/T4 → 底座 `docs/text-motion-integration.md`
          //    T6    → `docs/plan/t6-plate-interface.md`
          // ⚠️ `shake_phase` 是**轨级**（按 style 档错开，见 `SHAKE_PHASE_BY_STYLE`）——
          //    同轨多条 cue 仍同拍，那是底座契约的能力边界。
          ...motionFields(elementsCfg, SHAKE_PHASE_BY_STYLE[styleName]),
        },
        layers: cues.map((c, i) => ({
          id: `${id}_${i}`,
          start: c.start,
          end: c.end,
          source: { asset_id: id, source_in: 0 },
          opacity: 1.0,
        })),
      });
    }
  }

  for (const track of stickerTracks) tracks.push(track);
  for (const track of audioTracks) tracks.push(track);
  for (const track of textTracks) tracks.push(track);
  for (const track of subtitleTracks) tracks.push(track);
  for (const track of danmakuTracks) tracks.push(track);

  // 关键帧必须按帧号有序（契约层假定"用户会按顺序写"）。
  //
  // # 还要**去重**
  //
  // 运镜事件是**首尾相接**的（0.9→5.0、5.0→10.7……），于是前一条的终点
  // 与后一条的起点落在**同一帧**上。两条都往同一帧写 `scale`/`x`/`y`，
  // 就出现了重复的 target@frame —— 而那条曲线在 `channel_from` 里是
  // "取后一个的值"，表现是**这一段运镜被吃掉一半**。
  //
  // 后写的那条赢（它是新一段的起点，语义上更该说了算），
  // 与前一条的终点值相同或不同都无所谓 —— 衔接处本来就在同一帧上。
  // **每一层各自去重+排序**（裁剪之后主画面是好几层，每层有自己的曲线）。
  for (const layer of mainLayers) {
    const seen = new Map();
    for (const key of layer.keyframes) {
      seen.set(`${key.frame}|${key.target}`, key);
    }
    layer.keyframes = [...seen.values()];
    layer.keyframes.sort((a, b) => (a.frame - b.frame) || a.target.localeCompare(b.target));
    // 关键帧必须 **严格小于** 本层时长（校验判据 `>= duration` 就报错）。
    const limit = layer.end - layer.start;
    layer.keyframes = layer.keyframes.filter((key) => key.frame < limit);

    // ⚠️ **一个键都没有的层，也要把画面构成的几何落到基准 transform 上**（2026-10-02 修）。
    //
    // `SR`/`FIT` 是画面构成（`axis`/`position_x+y_pct`/`scale`/`padding`）的唯一几何，
    // 而它以前**只**通过相机关键帧写进去（`placeKeyframe(..., 'scale', camScale * FIT)`）。
    // 运镜整条被眼睛关掉时（见上面的 `laneAllows`）⇒ 这一层一个键都没有 ⇒ 主层停在
    // `scale=1 / offset=0`，
    // 底座便按「源 1:1 居中」画：**竖素材进竖画布 = 画面缩在中间、四周一圈黑**（用户最常见的组合）。
    // 基准值取「中性相机」（`zoom = 1`、不动点=中心）算出的那一组 —— 与有键时的同一式子。
    // ⚠️ **第一个实参必须是 `1.0`，不是 `FIT`**：`cameraOffset` 内部已经 `scale = zoom * FIT`
    // （它的注释专门写着「不要预乘 FIT，否则变成 FIT²」），传 `FIT` 会让位移按 `FIT²` 算、
    // 与这里写下的 `scale: FIT` 不一致 —— 居中时那个点恰好归零（所以只看中心看不出来）。
    // ⚠️ **只对 0 键的层做**：有键的层逐字节不动（存量零回归靠这条）。
    if (layer.keyframes.length === 0) {
      const zero = cameraOffset(1.0, { x: 0.5, y: 0.5 });
      layer.transform = { x: zero.x, y: zero.y, scale: FIT, rotation: 0 };
    }
  }

  // ---- 封面 / 弹幕：明确说明没转 ----
  if (config.cover) {
    const titleLines = config.cover.title_lines ?? [];
    drop(
      'cover',
      `封面的 ${titleLines.length} 行标题 + 字体/描边/暗角等 ${Object.keys(config.cover).length} 个参数：` +
        '封面是**静态图产出**，与底座的"逐帧合成"不是同一条路（本仓自己也是另开一条）',
    );
  }
  // 弹幕：**这一条要说准，别冤枉它。**
  //
  // 画面里那些弹幕面板有两种来源，要分清：
  //   - 录制时就烧进像素的（`海市蜃楼之馆` 那次，`danmaku.json` 是 `[]`）；
  //   - 真的有一份独立弹幕数据（`最鱿鱼之人` 那次，`danmaku.json` 有 4 条）。
  //
  // 所以这里**读一下 `data/danmaku.json` 再下结论**，不写死。
  const danmakuFilterWords = config.filter?.danmaku?.length ?? 0;
  if (danmakuFilterWords > 0) {
    let rawCount = null;
    try {
      const rawPath = join(clipDir, 'data', 'danmaku.json');
      if (existsSync(rawPath)) {
        const parsed = JSON.parse(readFileSync(rawPath, 'utf8'));
        rawCount = Array.isArray(parsed) ? parsed.length : null;
      }
    } catch {
      rawCount = null;
    }
    const verdict =
      rawCount === null
        ? 'data/danmaku.json 读不出来，无法判断有没有独立弹幕数据'
        : rawCount === 0
          ? 'data/danmaku.json 是空的 —— 画面里的弹幕是源像素自带的'
          : `data/danmaku.json 有 ${rawCount} 条独立弹幕数据`;
    note('filter.danmaku', `${danmakuFilterWords} 个过滤词保留在 extensions 里；${verdict}`);
  }
  if (config.sfx) {
    note('sfx', `[sfx] volume=${config.sfx.volume}：这是**混音总线音量**，底座没有总线增益`);
    if (config.sfx.shake_enabled) {
      note('sfx', 'shake_enabled=true：本仓会按音效强弱触发画面抖动，底座没有"音效驱动画面"这条链路');
    }
  }
  // ⚠️ 这条**只在图层那条腿真的没发**时才成立（2026-10-06）：开关已经打开（见 `BACKDROP_ENABLED`），
  // 而**纯色/黑**走的是画布底色（`render_hints.background`）—— 拿"暂时不发"去报它们，
  // 就是一条会把人引去查错方向的说明（实测：`bg_mode=color` 也报"暂时不发"，而它明明发出去了）。
  if (!BACKDROP_ENABLED && config.stage?.bg_mode === 'blur') {
    drop(
      'stage',
      'bg_mode="blur"：**底座有原语、我们暂时不发** —— 配方（同源层 → 模糊调整层 → 主层）已在转译器里，' +
        '被 `BACKDROP_ENABLED` 关着；且底座 `gaussian_blur.radius` 上限 16 ⇒ σ8，参照要 σ28（只能一半）。' +
        '**纯色/黑不受这条影响**：它们走画布底色（`render_hints.background`）',
    );
  }
  if (config.style?.danmaku && danmakuCueCount > 0) {
    // 字号/颜色/描边/不透明度/淡入淡出**都已接**；逐条滚动时长**也已接**（写进 ASS 的 6 参数 `\move`，
    // 轨道级 `duration_ms` 只当回退值）。
    //
    // ⚠️ 2026-10-01（Phase ③）修正：这条以前写"这三项没接：高亮色（素材里的 `\c` 标签**底座会剥掉**）、
    // 显示区域、逐条滚动时长（**已接**）" —— 一句话里三处毛病：
    //   ① 说"三项没接"，第三项自己又写着"已接"；
    //   ② 把摘 `\c` 甩给底座 —— **是本仓自己摘的**（YekiTrim 的 canvas 渲染器只读轨道色），
    //      而底座其实**支持**逐条颜色（`dhampir-timeline/src/subtitle.rs` 把行内 `\c` 抽进 `CueStyle.color`，
    //      还带 BGR→RGB 单测）⇒ 发了反而会让成片**比参照更花**，所以故意不发（见上面 `stripColorTags`）；
    //   ③ `highlight_color` 不是"没接"：**本仓渲染器也不消费它**（`highlight_color` 只用于字幕的 `.hl`，
    //      见 `templates/index.html` 的 755 / 1946 行）⇒ 这一项对弹幕本来就没有效果。
    drop(
      'style.danmaku',
      '弹幕的**显示区域**对不上（底座泳道由 `lanes` 决定，参照按 `layout.danmaku_area` 框选）。' +
        '另外两项**都不是缺口**：逐条颜色 `\\c` 是**本仓主动摘掉的**（渲染器只读轨道色；底座其实支持，' +
        '发了会比参照更花）；`highlight_color` **本仓渲染器也不消费**（它只服务字幕高亮）',
    );
  }
  if (config.sticker?.position_mode) {
    note('sticker', `贴纸落点模式 "${config.sticker.position_mode}"：底座没有安全区推导，落点直接取 polish.toml 里的百分比`);
  }

  const doc = {
    project_schema: 1,
    generator: { app: 'dhampir', version: '0.0.1' },
    meta: {
      title: config.project?.name ?? 'translated',
      created_at: null,
      modified_at: null,
    },
    assets,
    timeline: { schema: 4, timebase, markers: [], tracks },
    view: { playhead: 0, selection: null, zoom: 1 },
    // **画布底色**（零图层）：`bg_mode = color|black` 时把颜色交给底座当**清屏色**。
    // ⚠️ 不要为它加图层：加一层"有素材的图层"会触发底座分段路径的逐帧泄漏（长片 OOM，实测 2 层 1500 帧）。
    // `blur` 那条腿是**图层级**的（同源层 + 模糊），与这里的清屏色无关（它还关着，见 STAGE_BG 那两段）。
    render_hints: {
      width,
      height,
      format: 'mp4',
      ...(stageBackground ? { background: stageBackground } : {}),
    },
    extensions: {
      // **转译的来龙去脉留在这里**：工程文件本身要说清它是从哪来的。
      'yekitrim.polish_toml': {
        translated_at_frame_rate: timebase.num / timebase.den,
        // 素材自己那份（没抬下限的那个）也记下来：两者不等时，看这张表就能分辨
        // "工程网格被抬到 30" 与 "源真的 30fps"（排查时最容易混的一对）。
        source_frame_rate: assetTimebase.num / assetTimebase.den,
        camera_events: cameraCount,
        shake_events: shakeCount,
        sticker_events: stickerCount,
        sticker_layers: stickerLayers.length,
        sfx_clips: sfxClips.length,
        overlay_events: overlayEventCount,
        mute_ranges: muteFrames.length,
        cut_seconds: cutFrames / fps,
        kept_ranges: keepRanges.length,
        // **成片时间线的实况** —— ⚠️ **只在真的用了 EDL 时才写这几个键**。
        //
        // 这不是"好看"的问题：`target/edl-baseline/*.json` 的 sha256 是**存量零回归（R4）**的
        // 判据，而老工程 `edl` 段缺失 ⇒ 多出四个键 = 逐字节不同 ⇒ 那条判据当场翻面。
        // 所以带 EDL 才有这几个键（`edl_active` 自己就是"有没有 EDL"，老工程没有它也不丢信息）。
        //
        // `out_frames` = 成片总帧数（= 主画面所有层 `end` 的最大值，与 Rust
        // `Edl::total_frames` / `dhampir::sequence_frames` 同一口径）。
        ...(edlClips.length > 0
          ? {
              edl_clips: edlClips.length,
              edl_active: true,
              out_frames: durationFrames,
              out_seconds: durationFrames / fps,
            }
          : {}),
        voice_ranges: audioTracks.find((t) => t.id === 'a0')?.layers.length ?? 0,
        subtitles: subtitleTracks.length > 0,
        danmaku: danmakuCueCount,
        danmaku_words: config.filter?.danmaku ?? [],
        danmaku_highlight: config.filter?.highlight ?? [],
        cover: config.cover ? { title_lines: config.cover.title_lines ?? [], frame_pos: config.cover.frame_pos } : null,
      },
    },
  };

  return doc;
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------
//
// **只有被当成入口跑时才执行这一段。**
//
// 上面的纯函数是要被 `scripts/check-yekitrim-translator.mjs` import 来测的 ——
// 不设这个门的话，一 import 就会 attempt 去读 `process.argv[2]` 的工程目录、
// 然后退 2。这是"可测"与"可跑"之间的最小代价。

const isEntryPoint = (() => {
  const entry = process.argv[1];
  if (!entry) return false;
  try {
    return import.meta.url === pathToFileURL(resolve(entry)).href;
  } catch {
    return false;
  }
})();

if (isEntryPoint) runCli();

function runCli() {
  const args = process.argv.slice(2);
  if (args.length === 0) {
    console.error(
      '用法：node tools/polish-to-dhampir.mjs <clip 目录> [--out 工程文件] [--keep-muted]\n' +
        '  --keep-muted  出片用：时间线保持整条，屏蔽区间交给尾部那一剪（预览**不能**用它）\n' +
        '  --video       预览用：把"实际媒体"换成代理（时基/分辨率随之；**出片不许传**）\n' +
        '  --font-families <json>  宿主给的「库 id → 真族名」映射（底座按族名找字体，\n' +
        '                          传 id 会静默落 ArialMT；不给就按 id 原样写）',
    );
    process.exit(2);
  }
  const clipDir = resolve(args[0]);
  const outIndex = args.indexOf('--out');
  const outPath = outIndex >= 0 ? resolve(args[outIndex + 1]) : join(clipDir, 'dhampir-project.doc.json');
  // 预览那条腿用它把"实际媒体"换成 720p 代理（时基/分辨率随之，见 probeMedia）。
  // **渲染那条腿不许传** —— 出片必须用源。
  const videoIndex = args.indexOf('--video');
  const videoRel = videoIndex >= 0 ? args[videoIndex + 1] : undefined;
  // 宿主传来的「库 id → 真族名」映射（见 `fontFamilyFor`）。给了就装在全局，
  // 没给就空表 —— 那时库里那支会按 id 原样写进契约（老行为，会静默落 ArialMT）。
  const famIndex = args.indexOf('--font-families');
  if (famIndex >= 0) {
    const path = args[famIndex + 1];
    try {
      setFontFamilies(JSON.parse(readFileSync(path, 'utf8')));
    } catch (e) {
      console.error(`✗ 读不了 --font-families ${path}：${e.message}`);
      process.exit(2);
    }
  }

  const tomlText = readFileSync(join(clipDir, 'polish.toml'), 'utf8');
  const config = parseToml(tomlText);
  const doc = translate(config, clipDir, outPath, { keepMuted: args.includes('--keep-muted'), video: videoRel });
  mkdirSync(dirname(outPath), { recursive: true });
  writeFileSync(outPath, JSON.stringify(doc, null, 2) + '\n', 'utf8');

  const counts = { DROP: 0, CLAMP: 0, NOTE: 0 };
  for (const issue of issues) counts[issue.level] += 1;

  console.log(`转译完成 -> ${outPath}`);
  console.log(`  轨道 ${doc.timeline.tracks.length} 条 / 素材 ${doc.assets.length} 个`);
  const stats = doc.extensions['yekitrim.polish_toml'];
  console.log(
    `  运镜 ${stats.camera_events} 条、贴纸 ${stats.sticker_layers} 张、` +
      `音效 ${stats.sfx_clips} 条、overlay ${stats.overlay_events} 条、` +
      `原声 ${stats.voice_ranges} 段${stats.subtitles ? '、字幕 ✓' : ''}` +
      `${stats.danmaku ? `、弹幕 ${stats.danmaku} 条 ✓` : ''}`,
  );
  console.log(`  丢失 ${counts.DROP} / 降级 ${counts.CLAMP} / 注意 ${counts.NOTE}`);

  if (process.env.DHAMPIR_QUIET !== '1') {
    for (const level of ['DROP', 'CLAMP', 'NOTE']) {
      const group = issues.filter((i) => i.level === level);
      if (group.length === 0) continue;
      console.log(`\n---- ${level} (${group.length}) ----`);
      for (const issue of group) console.log(`  [${issue.where}] ${issue.what}`);
    }
  }
}
