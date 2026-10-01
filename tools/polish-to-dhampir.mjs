#!/usr/bin/env node
// 把本仓（V-Trim）的 `polish.toml` 转译成 Dhampir 的工程文件（`*.doc.json`）。
//
// # 这份转译器是本仓的**出口**，不是产品代码
//
// 它活在 `tools/` 下（不在 `vtrim` 二进制的编译/运行路径上），用来回答一个具体问题：
// **「V-Trim 决定画什么、底座决定怎么画」这个分工，落到一份真工程上到底缺什么。**
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
//   「参照」= **本仓自己的出片模板** `crates/vtrim-polish/templates/index.html`
//             （canvas 那条腿：`renderFrame` / `getActiveXxx`）—— 它是本转译的**复刻对象**，
//             所以"参照侧的不一致"就是**本仓可以修的不一致**
//             正文里的 `index.html:NNN` / `templates/index.html` 都指这个文件（行号对着本仓的它）
//
// 规格（行为对照表 / DROP·CLAMP·NOTE 三级记录 / 怎么加一个新事件类型）见
// `docs/vtrim-integration.md`；曲线那部分的守卫在 `scripts/check-vtrim-translator.mjs`。

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
const EFFECT_VOCAB = (() => {
  const empty = { known: new Set(), labels: new Map(), loaded: false };
  try {
    const here = dirname(fileURLToPath(import.meta.url));
    const raw = readFileSync(join(here, '..', 'spec', 'effects.json'), 'utf8');
    const spec = JSON.parse(raw);
    return {
      known: new Set(spec.effects.map((e) => e.id)),
      labels: new Map(spec.effects.map((e) => [e.id, e.label])),
      loaded: true,
    };
  } catch {
    return empty;
  }
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
          node[key] = [];
          node = node[key][0] = {};
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
/// —— V-Trim 的配置里**四种都出现过**（CSS 本来就认这四种）。
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
// **V-Trim 的 `sticker_tag` 是一个"意图词"，不是文件名。**
//
// 流程是两段的：LLM 先按语义挑一个 tag（`搞笑` / `震惊` / `看热闹`……），
// 再由 `Step2b` 拿这个 tag 去**贴纸包**里查出候选文件、挑一个、写回
// `event.name` + `event.source`（见 V-Trim `llm/mod.rs:280`）。
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
/// 而 V-Trim 这边给的是 **`clip.dm.srt`** —— 一个"SRT 外壳 + 里面塞了
/// ASS 覆盖标签"（`{\move(...)}`、`{\pos(...)}`、`{\c&H...}`）的混合体。
/// 格式不是底座要的那个，所以要转一道。
///
/// **覆盖标签原样留着**：底座解析时本来就会把它们剥掉
/// （`ass_text_to_plain`），而且 `\move` 的坐标**故意不参与布局** ——
/// 落点由 `DanmakuSpec` 与目标尺寸统一算，两端拿同一份。留着的好处是
/// "这份 ASS 就是 V-Trim 那份的直译"，下一个人对着两边能看懂。
/// 把 `\move(x1,y1,x2,y2)` 补成 **6 参数** `\move(x1,y1,x2,y2,0,travelMs)`。
///
/// # 为什么要补
///
/// ASS 的 `\move` 有两种写法，含义**不同**：
///
/// * 4 个参数 —— 跨的是 **cue 窗口**（起点到终点由 cue 的 `Start`/`End` 定）
/// * 6 个参数 —— **显式**给出移动的起止时刻（毫秒，相对 cue 起点）
///
/// "滚动时长"是后者那个 `t2 - t1`。V-Trim 的 `progress = (el - fadeIn) / travel`
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
/// （`crates/vtrim-polish/templates/index.html`，同一份也在 `webui/src/polish/engine.js`）。
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
/// 录播姬 XML 给的窗口是**固定 8 秒**（`vtrim-core/src/danmaku/mod.rs::parse_bililive_xml`），
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
/// ⚠️ 与 `vtrim_core::danmaku::clamp_display_windows` 是同一条规则的两份实现
/// （Rust 那份管 V-Trim 自己的 `data/danmaku.json`）；改一边要改另一边。
export function danmakuExitMs(startMs, endMs, travelMs) {
  const travel = Number.isFinite(travelMs) && travelMs > 0 ? travelMs : 0;
  const need = startMs + DANMAKU_FADE_IN_MS + travel;
  return endMs > need ? endMs : need;
}

function cuesToAss(cues, { font = 'Sans', fontSize = 42 } = {}) {
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
      `Dialogue: 0,${assTime(cue.startMs)},${assTime(danmakuExitMs(cue.startMs, cue.endMs, cue.travelMs))},Default,,0,0,0,,${withMoveTravel(cue.text, cue.travelMs)}`,
    );
  }
  out.push('');
  return out.join('\n');
}

/// V-Trim 的贴纸盒子边长基准，**短边 1080 时为 280px**。
///
/// 抄自 V-Trim `process/placement.rs:36`：`STICKER_SIZE = 280.0`，
/// 而 `sticker_size() = STICKER_SIZE * min(W,H)/1080`。
/// 落点 `x`/`y` 是**这个盒子的左上角**（同文件 276 行的注释）。
///
/// 我第一版猜 220px，实测 V-Trim 成片里量出来约 290px —— 猜的和真的是两回事。
const STICKER_BOX_BASE = 280.0;

/// 解析 V-Trim 的描边串，形如 `"12px #403c3b"` —— **一个字段里两个值**。
///
/// 解析不出就退回给的那对兜底值，并**不静默**（调用方看不到返回值里的这一层，
/// 所以这里只负责"尽快给出一个合理的默认"，准确性由转译器的 DROP/CLAMP 清单负责）。
function parseStroke(raw, fallbackWidthPx, fallbackColor) {
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
  const colorMatch = raw.match(/#[0-9a-fA-F]{6}/);
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
/// 这个字段（见 `docs/vtrim-integration.md` 第六节第 6 条），这里就是喂它的地方。
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
/// V-Trim 的 `release/` 里也没带（它只带 `ysyk_0.9.0/` 目录）。
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
/// if(label==='近'){v=c.zoom_close;d=1.5;}
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
///   - **V-Trim / CSS**：度，`0` = **向上**、顺时针增大（`linear-gradient(0deg, …)` 是从下往上）。
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
  const defaults = { 近: 1.5, 远: 0.55, 中: 1.3 };
  const keys = { 近: 'zoom_close', 远: 'zoom_wide', 中: 'zoom_mid' };
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
// 纯函数：V-Trim 的三条曲线（**从参照逐行抄的**）
// ---------------------------------------------------------------------------
//
// 这些提到模块顶层并导出，是为了**能被测试直接调用**。
// 它们都只依赖参数、不依赖某一份工程配置。

/// `back_out(t, s)` —— V-Trim 的 `E.back_out(p, 3)`。
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

/// 本仓出片模板的贴纸逐帧状态（`crates/vtrim-polish/templates/index.html:1517 getActiveSticker`）。
///
/// # 窗口 = **事件时长本身**（口径见 `docs/vtrim-integration.md` §14：出点 = 最后一个显示帧）
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
///   * `crates/vtrim-polish/templates/index.html:1527`（`getActiveSticker` 的守卫）；
///   * `crates/vtrim-render-core/src/timeline.rs:530` 与
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

/// 本仓出片模板的手持波形（`crates/vtrim-polish/templates/index.html:593 handheldSway`）。
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

/// 本仓出片模板那份 `shake`（`crates/vtrim-polish/templates/index.html:1575 getShake`）：**固定 0.2s 的四级阶梯**。
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

// **导出给守卫**（`scripts/check-vtrim-translator.mjs` 的 `checkMappings`）：它拿一份**合成工程**
// 过一遍 `translate`，断言"某一档真的转出来了"。只有 `isEntryPoint` 那道门保证直接跑仍然是 CLI。
export function translate(config, clipDir, outPath, options = {}) {
  /// **出片那条腿传 `true`**：时间线保持整条（屏蔽区间留给出片尾部的整段重编码去剪，
  /// 与 chrome 那条腿同一条口径）。预览必须留 `false`（客户端要靠压缩时间线跳屏蔽区间）——
  /// 详见下面 `keepRanges` 那段注释。
  const keepMuted = options.keepMuted === true;
  const timebase = { num: 60, den: 1 }; // 素材实测 60fps
  const fps = timebase.num / timebase.den;
  const frameAt = (seconds) => Math.round(seconds * 60);

  const assets = [];
  const videoAssetId = 'clip.mp4';
  assets.push({
    id: videoAssetId,
    kind: 'video',
    name: config.project?.name ?? 'clip',
    uri: config.source?.video ?? 'clip.mp4',
    frame_count: null,
    timebase,
    width: 1920,
    height: 1080,
    content_hash: null,
    tags: {},
    note: '来源：polish.toml [source].video',
  });
  note('assets', 'frame_count 留空：转译器不去探测媒体（要让 probe 子命令回填）');

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
  // ⚠️ **口径以出片模板为准**（`crates/vtrim-polish/templates/index.html:1413-1414`，权威是它）：
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
  // 而 V-Trim 的运镜事件是**首尾相接**地铺满整段的，正好一条轨放得下；
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
  //     V-Trim 成片       96.900s      <- 短了 17.49s
  //     三段 mute_ranges  5.30 + 2.36 + 9.83 = 17.49s   <- 正好对上
  //
  // 也就是说 `mute_ranges` 里那些段**在成片里根本不存在**，
  // 后面所有内容的时间**整体前移**。按"只静音、不裁剪"转出来的东西
  // 从第一刀（56.14s）之后**全部错位**，最大偏 17.49 秒。
  //
  // 实测对账（按"输出时间 = 源时间 - 之前掐掉的总长"换算之后）：
  //
  //     V-Trim 65s  <-> 源 70.30s   逐像素差 9.17（错位时是 28.18）
  //     V-Trim 79s  <-> 源 84.30s   逐像素差 6.72（错位时是 **93.74**）
  //     V-Trim 90s  <-> 源 97.66s   逐像素差 8.39
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
  // 而**出片不能** —— 出片尾部还有一道 `Ffmpeg::cut_ranges`（与 chrome 那条腿共用的整段重编码），
  // 它按**源时间**剪。这里先掐一遍、尾部再剪一遍 = **剪两次**：实测成片比 chrome 短一大截，
  // 而且音轨是从**整条**源视频抽的、没跟着压缩，第一刀之后全错位。
  // ⇒ 出片给"整条时间线"，屏蔽区间交给尾部那一剪（两边口径就统一了）。
  const keepRanges = [];
  if (keepMuted) {
    keepRanges.push({ start: 0, end: totalFrames });
  } else {
    let cursor = 0;
    for (const mute of muteFrames) {
      if (mute.start > cursor) keepRanges.push({ start: cursor, end: mute.start });
      cursor = Math.max(cursor, mute.end);
    }
    if (cursor < totalFrames) keepRanges.push({ start: cursor, end: totalFrames });
  }
  // 每个保留区间的**输出起点**（把前面掐掉的长度减掉）。
  let outputCursor = 0;
  for (const range of keepRanges) {
    range.outStart = outputCursor;
    outputCursor += range.end - range.start;
  }
  const durationFrames = outputCursor; // 裁剪后的总长
  const cutFrames = totalFrames - durationFrames;

  /// 源帧号 -> 输出帧号。落在**被掐掉的段里**返回 null（那一段不存在）。
  const remapFrame = (sourceFrame) => {
    for (const range of keepRanges) {
      if (sourceFrame >= range.start && sourceFrame < range.end) {
        return range.outStart + (sourceFrame - range.start);
      }
    }
    return null;
  };
  /// 源秒 -> 输出帧号（便捷）。
  const remapSeconds = (seconds) => remapFrame(frameAt(seconds));

  /// 源毫秒区间 -> 输出毫秒区间，**可能一段变多段**。
  ///
  /// `mute_ranges` 是把那些段从时间线上**删掉**，所以一条字幕/弹幕如果跨了
  /// 剪辑点，它在成片里就该**断成两截**（前半在剪辑点结束、后半从下一段续上）。
  /// 只按起点重映射会让它**整条错位** —— 那是"字幕对不上话"最直接的来源。
  const remapCueMs = (startMs, endMs) => {
    const sf = Math.round((startMs / 1000) * fps);
    const ef = Math.round((endMs / 1000) * fps);
    const out = [];
    for (const range of keepRanges) {
      const from = Math.max(sf, range.start);
      const to = Math.min(ef, range.end);
      if (to <= from) continue;
      out.push({
        startMs: Math.round(((range.outStart + (from - range.start)) / fps) * 1000),
        endMs: Math.round(((range.outStart + (to - range.start)) / fps) * 1000),
      });
    }
    return out;
  };

  // 主画面：**每个保留区间一层**，首尾相接（不重叠）。
  const mainLayers = keepRanges.map((range, index) => ({
    id: `main-${index}`,
    start: range.outStart,
    end: range.outStart + (range.end - range.start),
    source: { asset_id: videoAssetId, source_in: range.start },
    transform: { x: 0, y: 0, scale: 1.0, rotation: 0 },
    opacity: 1.0,
    blend: 'normal',
    enabled: true,
    effects: [],
    keyframes: [],
    note: `主画面第 ${index + 1}/${keepRanges.length} 段（源第 ${range.start}~${range.end} 帧）`,
  }));

  /// 把一个关键帧放到**它所在的那一层**上，帧号换成该层内的相对帧号。
  ///
  /// 关键帧的帧号是**相对图层起点**的（`schema.rs:732`），而裁剪之后
  /// 源时间被切成了好几层 —— 所以不能再用绝对帧号。
  const placeKeyframe = (sourceFrame, target, value, easing) => {
    for (let index = 0; index < keepRanges.length; index += 1) {
      const range = keepRanges[index];
      if (sourceFrame >= range.start && sourceFrame <= range.end) {
        const layer = mainLayers[index];
        const local = range.outStart + (sourceFrame - range.start) - layer.start;
        layer.keyframes.push({ frame: Math.max(0, local), target, value, easing });
        return true;
      }
    }
    return false; // 落在被掐掉的段里
  };

  /// 把一个**特效**挂到它所在的那一层上（整层特效，帧号是轨道级的，不用转）。
  ///
  /// 特效的 `window` 是按**该层内的 local frame** 求值的，所以挂到裁剪后的
  /// 某一层上时，`fade_in`/`transient` 那些数字仍然是"这一段内部的秒数" ——
  /// 与 V-Trim 的语义一致（它的 shake 也是相对事件起点算的）。
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
    for (let index = 0; index < keepRanges.length; index += 1) {
      const range = keepRanges[index];
      if (sourceFrame >= range.start && sourceFrame < range.end) {
        mainLayers[index].effects.push(effect);
        return true;
      }
    }
    return false;
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
  /// V-Trim 的 `flash` / `hue_shift` / `noise` 都是**先升到峰值再线性落回**的
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
    const frames = Math.max(1, Math.round(seconds * fps));
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
    note(
      'mute_ranges',
      keepMuted
        ? `${muteFrames.length} 段屏蔽区间（共 ${(muteFrames.reduce((s, m) => s + (m.end - m.start), 0) / fps).toFixed(2)} 秒）**保留在时间线里**` +
          `（--keep-muted：出片尾部按源时间剪一次，与 chrome 那条腿同一条口径；` +
          `预览那条腿不能这么转）`
        : `${muteFrames.length} 段被**掐掉**（不是静音）：共 ${(cutFrames / fps).toFixed(2)} 秒，` +
          `成片从 ${(totalFrames / fps).toFixed(2)}s 变成 ${(durationFrames / fps).toFixed(2)}s；` +
          `主画面因此拆成 ${keepRanges.length} 段，所有事件时间已重映射`,
    );
  }

  // ---------------------------------------------------------------------
  // **源是不是竖素材**（`CFG.vertical`）—— 它整条关掉运镜
  // ---------------------------------------------------------------------
  //
  // 参照的判据在 `crates/vtrim-polish/src/project/config.rs`：
  //
  //     源是竖素材（`h > w`，沿用既有判据）
  //
  // 而 `templates/index.html:1413` 里：
  //
  //     if (CFG.vertical) return { zoom: 1.0, ox: ox * CW / 100, oy: oy * CH / 100, handheld: 0.0 };
  //
  // 也就是**整个运镜状态机被跳过**（`zoom=1`、`handheld=0`、不动点仍按 layout 给）。
  //
  // 不处理它的症状是"**竖素材进横画布导出还在缩放**" —— 这正是 V-Trim 修过的
  // 那一类失败（`docs/plan/polish-vertical-canvas.md`）。
  //
  // # 为什么这里要探媒体（与"不探测媒体"那条纪律的关系）
  //
  // 转译器的纪律是"**不探测媒体**：`frame_count` 留空，由底座的
  // `dhampir probe --write` 回填" —— 为的是"在一台没有 ffmpeg 的机器上也能跑完
  // 并给出清单"。**贴纸 GIF 已经是例外**（要它的帧数与尺寸算缩放）。
  //
  // 这里沿用同一条例外纪律：**探得到就照做，探不到就进 CLAMP 明说**
  // —— 不给默认值假装成功。用 [`probeSize`]（只问宽高，不带 `-count_frames`），
  // 对一条长视频是毫秒级的，不会把转译拖成"整条解码"。
  const sourceVideoPath = join(clipDir, config.source?.video ?? 'clip.mp4');
  const sourceSize = existsSync(sourceVideoPath) ? probeSize(sourceVideoPath) : null;
  /// `true` = 竖素材（关运镜）；`null` = 判不了（探不到）。
  const sourceIsVertical = sourceSize ? sourceSize.height > sourceSize.width : null;
  if (sourceIsVertical === true) {
    note(
      'source.video',
      `源素材是**竖的**（${sourceSize.width}x${sourceSize.height}）—— 参照整条关掉运镜` +
        '（`index.html:1413`：`zoom=1`、`handheld=0`），本仓同样关掉**全部 camera 事件**',
    );
  }

  // ---- 逐事件转译 ----
  const sfxClips = [];
  // 覆盖层事件数（进 `extensions.vtrim.polish_toml` 的元数据 + CLI 摘要）。
  //
  // ⚠️ 以前这里是个 `overlayLayers` **数组**：只有 `gradient` 那档往它里面推，
  // 于是"四档里接了几档"从计数上根本看不出来（M2 补齐 `color` / `fade_in|fade_out` 时，
  // 那个数组就彻底没人写了 —— 计数会永远显示 0，是**比不计数更糟**的那种元数据）。
  // 现在四档统一走 `placeEnvelope`，这里就改成一个诚实的计数器。
  let overlayEventCount = 0;
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

      // 事件的**源帧号**（V-Trim 的时间）与**输出帧号**（裁剪后的时间线）。
      // 落在被掐掉的段里的输出帧号是 null —— 那一段在成片里不存在。
      const srcStart = frameAt(at);
      const srcEnd = frameAt(typeof event.end === 'number' ? event.end : at);

      switch (event.type) {
        case 'camera': {
          // **竖素材：整条运镜不生效**（见上面 `sourceIsVertical` 的说明）。
          // 不是"丢掉"—— 是参照本来就不画，跳过才是**对齐**。
          if (sourceIsVertical === true) {
            break;
          }
          if (sourceIsVertical === null) {
            // 探不到源尺寸 -> 判不了是不是竖素材。**明说，不猜。**
            clamp(
              where,
              '探不到源素材尺寸（ffprobe 不可用？），**判不了"源是不是竖素材"** —— ' +
                '参照对竖素材会整条关掉运镜（`index.html:1413`），而这里只能照出运镜。' +
                '若这份工程的源是竖的，运镜会与参照不同。',
            );
            break;
          }
          // 运镜 = Transform 上的关键帧。**这是底座已有的能力**（T8），
          // 所以这一条是"转过去"而不是"丢掉"。
          const factor = zoomFactor(event.zoom, config.style?.camera, where);

          // 起点的**不动点**（focus）。V-Trim 的运镜是「以不动点为中心缩放」，
          // **不是**「把不动点搬到画面中心」—— 这两个完全不一样，见下面的推导。
          //
          // **优先用事件自己的 `origin`**：V-Trim 每条 camera 都带这个字段，
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

          // # 偏移公式（**从 V-Trim 源码推出来的**，不是猜的）
          //
          // V-Trim `render-core/draw.rs:274 camera_transform()` 里写死了：
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
          // 而那正是 V-Trim 的行为（source pass 清屏为黑，见它的
          // `wide_zoom_shrinks_around_origin`，z=0.55 时四下各 432px 黑边）。
          // **只收集，不落关键帧**：过渡要按 V-Trim 的状态机算
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
          });
          cameraCount += 1;
          break;
        }

        case 'shake': {
          // V-Trim 的 `shake` 是一个**短促的画面抖动**（爆点那一刻），
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
          // **`intensity` 是像素。** V-Trim 的 `shake()` 把它**直接**当 CSS 位移用：
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
          // V-Trim 有**两份** `shake`，形状完全不同：
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
            `shake intensity=${intensity}（**像素**，V-Trim 直接当 CSS 平移用）-> ` +
              '按出片那份 `getShake` 的四级阶梯逐帧写进**视频层**的 x/y',
          );
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

          // **优先用 `name`** —— V-Trim 的 `Step2b` 跑过之后，
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
            how = `V-Trim 选定的 name（source=${event.source ?? '?'}）`;
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
          // 落点：`x`/`y` 是**贴纸盒子的左上角**（V-Trim `process/mod.rs:276`），
          // 而底座 `Transform` 的 origin 是图层中心 —— 所以要挪半个盒子。
          //
          // 盒子边长按 V-Trim 的规则算：`280 * min(W,H)/1080`。
          // 素材是 500×500 的原图，要塞进这个盒子，所以 `scale = 盒子/素材`。
          const px = ratio(event.x);
          const py = ratio(event.y);
          const hasPos = px !== null && py !== null;
          if (!hasPos) {
            // ⚠️ 这句要给**下一步怎么做**（2026-09-30 M2 改）：落点是 `process` 的产物
            // （`vtrim-polish/src/process/mod.rs:276` 按 人物/字幕/弹幕 推导安全区后写死百分比），
            // 所以"没有落点"的正确处理是**先跑一次后处理**，而不是在这里猜一个位置。
            // 底座确实没有安全区推导 —— 这句话要留着，但要说清"不是底座的缺陷，是这一步没跑"。
            drop(
              where,
              `贴纸落点 "${event.x ?? '(无)'}","${event.y ?? '(无)'}" 解析不了 —— ` +
                '落点是**后处理的产物**（`process` 按人物/字幕/弹幕推导安全区后写死百分比）：' +
                '先跑一次 `vtrim polish process`（编辑器里随便改一下自动跑、或点「渲染」时也会先跑），' +
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
          const baseX = px * width - width / 2 + box / 2;
          const baseY = py * height - height / 2 + box / 2;

          // ---- 贴纸动效：逐帧采样 `stickerStateAt`（不再是手写关键帧）----
          //
          // 见 `vtrim-polish/templates/index.html:1517 getActiveSticker()`。
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
              '以及 `vtrim-render-core/src/timeline.rs` 与 `webui/src/polish/engine.js`' +
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
                  // V-Trim 的 `shape = "linear"` 对应底座 OverlayShape::Linear = 1。
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
          // **参照出片里的 `noise` 是一层纯白薄纱**，不是噪声：
          //
          //     getNoiseOpacity -> 0.08（0.08s 升到峰值，0.72s 落回）
          //     ctx.fillStyle = '#ffffff'; ctx.fillRect(0, 0, CW, CH)
          //
          // 所以这里映射成 **白色 `flash`**，而不是底座的 `noise` 特效 ——
          // 画面以参照成片为准。这一条同时是**本仓出片模板的可疑处**：名字叫 noise、
          // 出片画的却是白纱（真正的噪声只在 DOM 预览的 `handlers.noise` 里）。
          placeEnvelope(
            startFrame,
            0.80,
            { kind: 'flash', params: { amount: 0.08, r: 1, g: 1, b: 1 } },
            (dt) => (dt < 0.08 ? dt / 0.08 : Math.max(0, 1 - (dt - 0.08) / 0.72)),
            where,
          );
          note(
            where,
            '参照出片的 `noise` 是**纯白薄纱**（0.08 峰值、0.08s 升、0.72s 落），不是噪声 —— ' +
              '按白纱复现。**这是本仓出片模板的一处"名不副实"，值得本仓自己核。**',
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
  // 运镜：照抄 V-Trim 的**状态机**，不是"每条事件画两个端点"
  // ---------------------------------------------------------------------
  //
  // 权威实现在 `vtrim-polish/templates/index.html:1398 getActiveCamera()`：
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
  //    中间值，而 V-Trim 是稳稳的 1.3（那 0.6s 的过渡早就结束了）。
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
  // **这不是近似**：V-Trim 的 `pow2_out` 与底座的 `EaseOut` **逐值相同**
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
// 配方已实现（`bg1` 同源层 + `gaussian_blur`、`bg2` 调整层），但被 `BACKDROP_ENABLED = false` **关着** ——
// 底座分段路径的中间纹理逐帧累积，长片必 OOM（实测阈值在 1100~1500 帧之间）。
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
  const fitSrc = probeSize(join(clipDir, config.source?.video ?? 'clip.mp4'));
  const srcW = fitSrc ? fitSrc.width : width;
  const srcH = fitSrc ? fitSrc.height : height;
  const srcPortrait = srcH > srcW;
  const stagePositionRaw = config.stage?.position ?? 'auto';
  const axis = (() => {
    if (stagePositionRaw !== 'auto') {
      // 显式位置：按比例定轴（交叉相乘，与参照同一条判据）
      if (!fitSrc) return 'fill';
      const lhs = BigInt(srcH) * BigInt(width);
      const rhs = BigInt(srcW) * BigInt(height);
      if (lhs > rhs) return 'x';
      if (lhs < rhs) return 'y';
      return 'fill';
    }
    return srcPortrait ? 'x' : 'fill';
  })();
  // 方向不跨用：横画布写 top、竖画布写 left 一律当 center（与参照的净化一致）
  const stagePosition = (() => {
    if (stagePositionRaw === 'auto') return 'center';
    const landscape = config.canvas?.orientation !== 'portrait';
    const ok = landscape
      ? ['left', 'center', 'right'].includes(stagePositionRaw)
      : ['top', 'center', 'bottom'].includes(stagePositionRaw);
    return ok ? stagePositionRaw : 'center';
  })();
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
      x = stagePosition === 'left' ? bx : stagePosition === 'right' ? bx + bwid - w : bx + (bwid - w) / 2;
      y = by + (bhgt - h) / 2;
    } else if (axis === 'y') {
      w = bwid;
      h = (bh * w) / bw;
      if (h > bhgt) {
        h = bhgt;
        w = (bw * h) / bh;
      }
      y = stagePosition === 'top' ? by : stagePosition === 'bottom' ? by + bhgt - h : by + (bhgt - h) / 2;
      x = bx + (bwid - w) / 2;
    } else {
      const s = Math.max(bwid / bw, bhgt / bh);
      w = bw * s;
      h = bh * s;
      x = bx + (bwid - w) / 2;
      y = by + (bhgt - h) / 2;
    }
    if (Math.abs(stageScale - 1) > 1e-9) {
      const mx = x + w / 2;
      const my = y + h / 2;
      w *= stageScale;
      h *= stageScale;
      x = mx - w / 2;
      y = my - h / 2;
    }
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
  if (!fitSrc) {
    clamp('stage', '探不到源素材尺寸（ffprobe 不可用？），按画布尺寸当源 —— 比例不一致时画面会错位');
  }
  if (Math.abs(FIT - 1) > 1e-6 || SR.x !== 0 || SR.y !== 0 || STAGE_BG !== 'none') {
    note(
      'stage',
      `源 ${srcW}x${srcH} -> 画布 ${width}x${height}（axis=${axis} position=${stagePosition} ` +
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
    // `td = min(duration * 0.5, 0.6)`；硬切时整段不缓动。
    const durationSec = Math.max(1 / fps, (shot.srcEnd - shot.srcStart) / fps);
    const tdFrames = hardCut ? 0 : Math.max(1, Math.round(Math.min(durationSec * 0.5, 0.6) * fps));

    // 过渡的**起点**：zoom 还是上一个状态，但不动点已经是**本事件自己的** origin
    // （V-Trim 里 `ox = lerp(ax, bx, p)`，p=0 时就是 ax）。
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
      note(shot.where, `远景（${shot.zoom}）进出按 V-Trim 的规则**硬切**，没有过渡`);
    }
    if (shot.z < 1) {
      clamp(
        shot.where,
        `zoom=${shot.zoom}（${shot.z}）是**画中画档**：四周会留黑边 —— ` +
          '这与 V-Trim 一致（它也是黑底），但 V-Trim 的 `stage.bg_mode` 还能铺模糊底，底座没有那一层',
      );
    }
    // `handheld` 不在这里处理 —— 它是**视频层的纯位移/旋转/呼吸缩放**，
    // 与独立 `shake` 一起在循环之后按帧采样（见 `cameraStateAt` 那一段）。
    if (shot.zoom === '近' && !shot.handheld) {
      note(shot.where, '近景未加即时抖 —— V-Trim 的近景自带轻微推拉，底座只做缩放');
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

  /// 逐帧重放 V-Trim 的 `getActiveCamera`（`index.html:1398`）。
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
    }))
    .filter((e) => e.outStart !== null)
    .sort((a, b) => a.outStart - b.outStart);

  const cameraStateAt = (outFrame) => {
    const t = outFrame / fps;
    let zoom = 1.0;
    let handheld = 0.0;
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
      const ax = e.origin.x * 100;
      const ay = e.origin.y * 100;
      const bx = e.originTo.x * 100;
      const by = e.originTo.y * 100;
      ox = ax + (bx - ax) * p;
      oy = ay + (by - ay) * p;
    }
    return { zoom, handheld, ox: (ox * width) / 100, oy: (oy * height) / 100 };
  };

  /// 输出帧 -> 源帧（`placeKeyframe` 要的是源帧号）。
  const sourceFrameForOutput = (outFrame) => {
    for (const range of keepRanges) {
      const length = range.end - range.start;
      if (outFrame >= range.outStart && outFrame < range.outStart + length) {
        return range.start + (outFrame - range.outStart);
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
    // 拿 V-Trim 的成片对拍时，参照两次渲染**完全相同**（差 0.00），
    // 只有本仓这边变了，所以不是渲染不确定，也不是编码器。
    //
    // 修法：密集键**从上一个稀疏键的下一帧开始写**，一路写到窗口末尾。
    // 因为 `cameraStateAt` 已经能逐帧重放运镜曲线，那一段写进去的值就是
    // **基准曲线本身** —— 于是段结构虽然变了，曲线没变。
    //
    // 代价：每个窗口多几十到几百个键（实测这一份工程 12 个窗口共多约 2 千个）。
    // 换来的是"写密集键这件事**不再有副作用**"。
    const sparseSorted = [...sparseOutFrames].sort((a, b) => a - b);
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
    /// 每个窗口的密集区间：`[上一个稀疏键 + 1, 窗口末 + 1]`。
    ///
    /// **没有"上一个稀疏键"时就别往前铺。**
    ///
    /// `prevSparse` 返回 -1 有两种情形：这一个通道上**一个稀疏键都没有**
    /// （竖素材整条关掉运镜就是这种），或者稀疏键都在窗口**之后**。
    /// 两种情形下窗口**之前**都没有段结构可保护 —— 往前铺到第 0 帧只是白写。
    /// 实测：竖素材那份工程因此写了 **16716** 个关键帧（每帧一个），
    /// 而它们全是常量 `x=0,y=0,scale=1`；只从窗口起点开始写就够。
    const denseRanges = spans.map(([a, b]) => {
      const prev = prevSparse(a);
      return [prev === -1 ? a : prev + 1, b + 1];
    });
    const inDenseRange = (out) => denseRanges.some(([a, b]) => out >= a && out <= b);

    let handheldFrames = 0;
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
      const moving = st.handheld > 1e-6 || inDenseRange(out);
      if (!moving) continue;

      const sourceFrame = sourceFrameForOutput(out);
      if (sourceFrame === null) continue;
      const base = cameraOffset(camScale, { x: st.ox / width, y: st.oy / height });
      placeKeyframe(sourceFrame, 'x', base.x + sway.dx + shx + stx, 'linear');
      placeKeyframe(sourceFrame, 'y', base.y + sway.dy + shy + sty, 'linear');
      placeKeyframe(sourceFrame, 'scale', camScale * FIT, 'linear');
      if (st.handheld > 1e-6) {
        placeKeyframe(sourceFrame, 'rotation', sway.rot, 'linear');
        handheldFrames += 1;
      }
      movedFrames += 1;
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

  // **不再报「pow2_out 只能近似」** —— V-Trim 的 `pow2_out(t) = 1-(1-t)^2`
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
    const layers = keepRanges.map((range, index) => ({
      id: `voice-${index}`,
      start: range.outStart,
      end: range.outStart + (range.end - range.start),
      // **原声从素材的对应时刻取**，不是从头。
      // `source_in` 是素材内偏移，掐掉前面的部分之后必须跟着走。
      source: { asset_id: videoAssetId, source_in: range.start },
      transform: { x: 0, y: 0, scale: 1.0, rotation: 0 },
      opacity: 1.0,
      blend: 'normal',
      enabled: true,
      effects: [],
      keyframes: [],
      note: '原声',
    }));
    if (layers.length > 0) {
      audioTracks.push({ id: 'a0', kind: 'audio', layers });
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
      // **音量**：V-Trim 的三处调用点都是
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
  //   那为什么还关着：**还没做产品路径的验收**（走 `vtrim polish render --renderer dhampir`
  //     完整链路 + 黄金帧对拍重标定）。打开就一行：把下面这个 `if (false && ...)` 的开关改掉。
  //   另有一条**改不掉**的：底座 `gaussian_blur.radius` 上限 16（σ8），参照模糊底要 σ28 ⇒ 只能一半。
  //
  // 原注释保留：**暂时不发背景层** —— 它会撞上底座的**分段路径泄漏**，长片必 OOM。
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
  //     `vtrim polish render --renderer dhampir` 也成功（12s / 1080x1920 / 3.2MB）。
  //   · **但"看得见"没验成**：拿一个竖画布 + `bg_mode=blur` 的工程开/关各渲一次，成片**逐像素相同** ——
  //     因为那个工程的 `axis` 走 **cover**（默认档），主画面铺满画布，**底被完全盖住** ⇒ 开关对画面无影响。
  //   · **下一步（一行夹具就能定论）**：造一个 `axis = "x"` 或 `"y"`（**contain**）+ 竖画布 + `bg_mode=blur`
  //     的工程 ⇒ 主画面留边，那条边里应当是**模糊底**（不是纯黑）。验成了再把这个开关打开。
  //
  // ⚠️ 另有一条**改不掉**的限制：底座 `gaussian_blur.radius` 上限 16 ⇒ σ8，而参照模糊底要 σ28
  // ⇒ 即便打开也**只能做到一半**，按约定如实记 CLAMP（不是静默近似）。
  const BACKDROP_ENABLED = false;
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
        // 症状在 `vtrim-compare` 里表现为 `*rawvideo* muxer: Invalid argument`，
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
    if (false && blurPx > 8) {
      clamp(
        'stage',
        `模糊底的 blur(${blurPx}px)：底座 gaussian_blur 的 radius 上限 16（= σ8），` +
          `而参照要 σ${blurPx} —— 且多个模糊**不叠加**（取最大值），所以模糊底比参照**淡**`,
      );
    }
  }
  if (STAGE_BG !== 'none' && !BACKDROP_ENABLED) {
    clamp(
      'stage',
      `背景层（bg=${STAGE_BG}）：**暂时不发** —— 它会加一个常驻的"有素材图层"，` +
        `从而触发底座的**分段路径逐帧泄漏**（实测：2 层时 1500 帧 OOM，1 层时全片通过；` +
        `去掉模糊也一样）。底座修好前，主画面之外的边是黑的` +
        (STAGE_BG === 'color' ? '（底座也没有纯色背景层）' : ''),
    );
  } else if (STAGE_BG === 'color') {
    note('stage', 'bg_mode=color：底座没有"纯色背景层"，那部分仍是黑底（与 bg_mode=black 同）');
  }

  // ---- 组装 ----
  const tracks = [
    // 背景在最前：铺满时被主画面整块盖住，有位移才露出来（与参照同序）。
    ...backdropTracks,
    // 主画面：裁剪之后是**每个保留区间一层**（首尾相接，不重叠）。
    { id: 'v1', kind: 'video', layers: mainLayers },
  ];
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
  // 为什么紧跟主画面：**V-Trim 的出片顺序是**
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

  // 字幕与弹幕轨 —— 用底座自己的子系统，把 V-Trim 的素材接进来。
  const subtitleTracks = [];  if (config.source?.subtitles && existsSync(join(clipDir, config.source.subtitles))) {
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
      timebase: { num: 60, den: 1 },
      width: null,
      height: null,
      content_hash: null,
      tags: {},
      note: `由 ${id} 重映射到裁剪后的时间线`,
    });
    // 字幕样式：**从 V-Trim 的模板抄实际值**，不能用缺省。
    //
    // V-Trim 的 CSS（`templates/index.html:747`）：
    //   font-size: 72px            -> font_ratio = 72/1080 = 0.0667
    //   color: #dcbda0             -> [220,189,160,255]
    //   -webkit-text-stroke: 12px #403c3b  -> 底座只有 `outline: bool`
    // 落点：画布绘制用 `cy = CH - 120*CKH`（同文件 1966 行）
    //   -> bottom_margin = 120/1080 = 0.1111
    const subStyle = config.style?.subtitle ?? {};
    const subSizePx = typeof subStyle.size === 'number' ? subStyle.size : 72;
    const subColor = hexColor(subStyle.color) ?? { r: 1, g: 1, b: 1 };
    // 描边 V-Trim 写的是 `"12px #403c3b"` —— 一个字符串里两个值。
    const subStroke = parseStroke(subStyle.stroke, 12, '#403c3b');
    // 行高：V-Trim 的字幕 CSS 是 `line-height:1.5`。
    const subLineHeight = 1.5;
    // **落点要换算，不能直接搬 120。**
    //
    // V-Trim 画的时候用的是 `cy = CH - 120` —— 那是**文字的中心**；
    // 而底座的 `bottom_margin` 是**行盒底边**到底边的距离。两者差半个行盒。
    //
    // 直接填 `120/1080` 会让字幕整体**高约 54px**（行盒 = 72*1.5 = 108，一半是 54）。
    // 实测 55s 那一帧的字幕带差从 34.16 降到 26.42，整体 16.07 -> 14.42。
    const subLineBoxPx = subSizePx * subLineHeight;
    const subBottomMargin = (120 - subLineBoxPx / 2) / height;
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
        // V-Trim 写 `12px`，那是 CSS `-webkit-text-stroke` 的值 —— 判据在字的
        // 轮廓上、**里外各一半**；而 ffmpeg 的 `borderw` 是全在外侧。
        // 直接填 12 会让成片的描边粗一倍（实测 55s 那一帧差 +0.98，
        // 而 4px 跟 6px 几乎一样好，说明 6px 附近就是对的）。
        stroke_ratio: subStroke.widthPx / 2 / height,
        stroke_color: subStroke.color,
        // **淡入淡出**（V-Trim `getActiveSubs`：fadeIn 0.35 / fadeOut 0.18、
        // 入场从下方 +20px 浮上来、退场向上 -8px）。
        fade_in_ms: Math.round(0.35 * 1000),
        fade_out_ms: Math.round(0.18 * 1000),
        rise_in_px: 20,
        rise_out_px: 8,
        // **字重与行高**：V-Trim 的 CSS 写死了
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
        // **换行安全宽 87.5%**：V-Trim 写死 `wrapCaptionParts(parts, CW * 0.875, …)`。
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
        font_family: subStyle.font || null,
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
  // V-Trim 的弹幕是 `clip.dm.srt`（SRT 外壳 + ASS 覆盖标签）。
  // 底座要的是**真 ASS**，所以转一道写成 `dhampir-danmaku.ass`。
  //
  // 转出来的文件放在**工程文件旁边**，不是 clip 目录里 ——
  // 它是转译的产物，不该污染源素材目录。
  const danmakuTracks = [];
  let danmakuCueCount = 0;
  const danmakuFile = config.source?.danmaku;
  if (danmakuFile) {
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
        // `vtrim-polish/src/parse/text.rs:74`：
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

        const cues = [];
        let straddled = 0;
        for (let index = 0; index < rawCues.length; index += 1) {
          const cue = rawCues[index];
          const pieces = remapCueMs(cue.startMs, cue.endMs);
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

        // **去掉逐条的 `\c` 颜色标签 —— V-Trim 不认它。**
        //
        // 素材里确实有：`{\pos(960,1040)}{\c&HFF3FE3}还能续约吗`。但 V-Trim 的
        // 渲染那一侧**只读轨道级颜色**（`templates/index.html:2006`）：
        //
        //     var color_dm = dmS.color || '#ffffff';      // 一份，给所有弹幕用
        //     dms_a.forEach(function(dm) { ... ctx.fillText(dm.text, dm.x, dm.y) });
        //
        // `getActiveDms` 的返回值只有 `{text, opacity, x, y}` —— **没有颜色**。
        // `\pos` 与 `\c` 在这个渲染器里都是残留（前者我早就记下了）。
        //
        // 我先前把 `\c` 当成了"V-Trim 需要逐条颜色"的证据（那份 `danmaku.json`
        // 里确实有 `color` 字段），于是**接了个看着对、其实与参照相反的东西**：
        // 出片一对比 —— 48.8s 那一帧 V-Trim 是白的、底座是品红的。
        //
        // 契约里的逐条颜色是**对的通用原语**（ASS 的 `\c` 就是这个意思），
        // 所以保留；**转译器不写它**才是"复现 V-Trim 的行为"。
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
            `逐条颜色 \\c：素材里 ${stripped} 条带颜色标签，V-Trim 的渲染器**不读它**` +
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
          timebase: { num: 60, den: 1 },
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
            // V-Trim 的弹幕：`font-size: 42px`（模板 760 行）
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
            // **颜色**：V-Trim 弹幕是 `#ffffff`。以前底座没有这个字段，
            // 只能与字幕**共用**一份 —— 而字幕是暖色 `#dcbda0`，必然错一个。
            color: rgbaBytes(hexColor(dmStyle.color) ?? { r: 1, g: 1, b: 1 }),
            // **基础不透明度 0.9**（V-Trim 模板 1503 行 `var opacity = 0.9`）。
            opacity: 0.9,
            // **淡入淡出**（同模板 1495 行：fadeIn 0.3 / fadeOut 0.2）。
            fade_in_ms: DANMAKU_FADE_IN_MS,
            fade_out_ms: Math.round(0.2 * 1000),
            outline: true,
            // 同字幕：CSS 的居中描边折半成"外侧宽度"。
            stroke_ratio: parseStroke(dmStyle.stroke, 2, '#000000').widthPx / 2 / height,
            stroke_color: parseStroke(dmStyle.stroke, 2, '#000000').color,
            // **泳道带的几何**。V-Trim 的 `assign_danmaku_tracks` 用
            // `TRACK_YS = [300, 500, 700, …]`，那是 **1920 宽的日志坐标**；
            // 模板里 `y = (d.y/1920) * (540*CK)` 再除以画布高，归一化之后
            // 就是 `d.y / 3840` —— 与画布尺寸无关，所以直接写这两个数。
            //
            // 底座老规则是"0 号泳道贴顶、间距取行盒高"（0 起、0.048），
            // 而 V-Trim 是 0.0781 起、间距 0.0521 —— 差值是肉眼可见的一整条。
            lane_top_ratio: 300 / 3840,
            lane_spacing_ratio: 200 / 3840,
            // V-Trim 的弹幕 CSS：`font-weight:600`。
            font_weight: 600,
            line_height: 1.5,
            font_family: dmStyle.font || null,
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
  // 对齐 V-Trim **出片**那条腿（`renderFrame`）：
  //
  //     v1(主画面) -> 调整图层 -> 字幕 -> 弹幕 -> 贴纸
  //
  // **贴纸排在最后**是这一轮核出来的：`renderFrame` 里 `getActiveSticker` 在
  // 字幕（L1948）与弹幕（L2003）**之后**（L2029）。我先前按 DOM 层级把它排在
  // 字幕之前 —— 两者只有在"贴纸与字幕在时间上重叠"时才看得出区别。
  //
  // 音频不参与绘制顺序，放最后只为读起来清楚。
  for (const track of stickerTracks) tracks.push(track);
  for (const track of audioTracks) tracks.push(track);
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
  if (config.stage?.bg_mode && config.stage.bg_mode !== 'auto') {
    // ⚠️ 措辞更正（2026-10-01）：**底座有原语**（[同源 cover 层] → [调整层 `gaussian_blur`] → [主层]），
    // 配方也已经写在这份转译器里 —— 不发是因为 `BACKDROP_ENABLED` **关着**（还没做产品路径验收；
    // 旧读数里那条"长片必 OOM"在新底座 `c7f29b9` 上复现不出来）。以前这里写"底座没有背景模式这一层"，
    // 是**说错了**：会让人去底座加一个它根本不需要的概念。
    drop(
      'stage',
      `bg_mode="${config.stage.bg_mode}"：**底座有原语、我们暂时不发** —— 配方（同源层 → 模糊调整层 → 主层）已在转译器里，` +
        '被 `BACKDROP_ENABLED` 关着（未做产品路径验收）；且底座 `gaussian_blur.radius` 上限 16 ⇒ σ8，参照要 σ28（只能一半）',
    );
  }
  if (config.style?.danmaku && danmakuCueCount > 0) {
    // 字号/颜色/描边/不透明度/淡入淡出**都已接**；逐条滚动时长**也已接**（写进 ASS 的 6 参数 `\move`，
    // 轨道级 `duration_ms` 只当回退值）。
    //
    // ⚠️ 2026-10-01（Phase ③）修正：这条以前写"这三项没接：高亮色（素材里的 `\c` 标签**底座会剥掉**）、
    // 显示区域、逐条滚动时长（**已接**）" —— 一句话里三处毛病：
    //   ① 说"三项没接"，第三项自己又写着"已接"；
    //   ② 把摘 `\c` 甩给底座 —— **是本仓自己摘的**（V-Trim 的 canvas 渲染器只读轨道色），
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
    timeline: { schema: 4, timebase: { num: 60, den: 1 }, markers: [], tracks },
    view: { playhead: 0, selection: null, zoom: 1 },
    render_hints: { width, height, format: 'mp4' },
    extensions: {
      // **转译的来龙去脉留在这里**：工程文件本身要说清它是从哪来的。
      'vtrim.polish_toml': {
        translated_at_frame_rate: timebase.num / timebase.den,
        camera_events: cameraCount,
        shake_events: shakeCount,
        sticker_events: stickerCount,
        sticker_layers: stickerLayers.length,
        sfx_clips: sfxClips.length,
        overlay_events: overlayEventCount,
        mute_ranges: muteFrames.length,
        cut_seconds: cutFrames / fps,
        kept_ranges: keepRanges.length,
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
// 上面的纯函数是要被 `scripts/check-vtrim-translator.mjs` import 来测的 ——
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
        '  --keep-muted  出片用：时间线保持整条，屏蔽区间交给尾部那一剪（预览**不能**用它）',
    );
    process.exit(2);
  }
  const clipDir = resolve(args[0]);
  const outIndex = args.indexOf('--out');
  const outPath = outIndex >= 0 ? resolve(args[outIndex + 1]) : join(clipDir, 'dhampir-project.doc.json');

  const tomlText = readFileSync(join(clipDir, 'polish.toml'), 'utf8');
  const config = parseToml(tomlText);
  const doc = translate(config, clipDir, outPath, { keepMuted: args.includes('--keep-muted') });
  mkdirSync(dirname(outPath), { recursive: true });
  writeFileSync(outPath, JSON.stringify(doc, null, 2) + '\n', 'utf8');

  const counts = { DROP: 0, CLAMP: 0, NOTE: 0 };
  for (const issue of issues) counts[issue.level] += 1;

  console.log(`转译完成 -> ${outPath}`);
  console.log(`  轨道 ${doc.timeline.tracks.length} 条 / 素材 ${doc.assets.length} 个`);
  const stats = doc.extensions['vtrim.polish_toml'];
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
