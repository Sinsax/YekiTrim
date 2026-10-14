// 精修预览引擎（`webui/src/polish/engine.js`）的**运行期**守卫。
//
// # 为什么要有它（2026-10-01 的真实事故）
//
// 上一轮删四个效果类型时，同一个手法在三处留下连带损伤，而**门禁一条都没红**：
//   ① `buildIndex` 的桶清单丢了 `vignette` / `sfx`（模板那份是全的）⇒ 工程里**恰好没有该类型事件**时
//      桶不会按需创建 ⇒ `getVignette(t, idx.vignette)` 里 `for (const v of undefined)` 抛
//      `can't access property Symbol.iterator, d.vignette is undefined`；
//   ② `videoFxAt` 的返回对象被削成 `{shake, blur}` ⇒ 预览读 `f.camera.handheld` 抛
//      `can't access property "handheld", i.camera is undefined`；
//   ③ 词汇表里 `overlay.ui` 写成对象 `{}` ⇒ 编辑器 `(e.ui ?? []).includes(...)` 抛
//      `(intermediate value).includes is not a function`。
// 三者都会让 **Vue 把整块子树 unmount** —— 用户看到的是"预览区的视频和控件一起消失、还不报错"，
// 而服务端日志干干净净（全是 200）。**静态读源码抓不住**，所以这里真跑一遍。
//
// engine.js 是**纯 ESM、零依赖**（浏览器预览的移植副本），因此可以在 node 里直接 import：
// 用一份"**一个事件都没有**"的最小工程建索引（正是 ① 的触发条件），再把预览会调的那一族求值器
// 逐个跑一遍。判据写错会"永远绿"，所以自带 `--self-test`：故意删一个桶，必须能抛出来。
import { readFileSync } from 'node:fs';
import { copyFileSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ENGINE = 'webui/src/polish/engine.js';
const TEMPLATE = 'crates/yekitrim-polish/templates/index.html';
/// 变异自证要 `import()` 绝对路径的**未变异**那份（做反向控制）。
const ENGINE_ABS = resolve(ENGINE);
/// `engine.js` 的相对依赖（变异体放到临时目录时要一起复制，否则 `ERR_MODULE_NOT_FOUND`）。
const ENGINE_DEPS = ['canvas.js'];

/** 预览真正会调的那一族（签名都是 `(idx, t)`）。 */
const IDX_T_FAMILY = [
  'videoFxAt', 'cameraAt', 'getFlash', 'getHueShiftFlash', 'getNoiseOpacity', 'getVignette',
  'getOverlayState', 'subsAt', 'danmakuAt', 'stickersAt',
];

let checks = 0;
const fail = (msg) => {
  throw new Error(msg);
};

/** 模板 `buildIndex` 里预建的桶集合（出片权威路径那份）。 */
function templateBuckets(text) {
  const at = text.indexOf('function buildIndex()');
  if (at < 0) fail(`模板里找不到 buildIndex（${TEMPLATE}）`);
  const seg = text.slice(at, at + 4000);
  const names = new Set();
  // 形如 `camera: [], sticker: [],`（**一行多**也要全抓到 —— 我第一版带 `^` 锚点，只抓到每行第一个，
  // 于是把"两边不一致"误报成真：模板那份其实是全的）
  for (const m of seg.matchAll(/([a-z_]+):\s*\[\]/g)) names.add(m[1]);
  return names;
}

// ---------------------------------------------------------------------------
// 「登记了」≠「画得出来」—— 每个桶都必须有一条**能产出可见输出**的绘制路径
// ---------------------------------------------------------------------------
//
// # 为什么需要它（2026-10 的维度漏洞）
//
// 上面 ①②③ 证明的都是「桶**登记了**」：成员集合两边一致、空工程下是数组、求值器不抛。
// 但「登记了」**完全不蕴含**「预览真的会把它画出来」——
// 这两件事差着一个维度，而**少的那一维恰好是错的所在**（与旧总表"只查消费者不查生产者"同一族）。
//
// 实证的洞：`element` 桶登记得好好的、求值器也不抛，
// 但 `engine.js:890` 的 `if (kind !== 'sticker') continue` 让它**恒返回空数组** ⇒
// **预览一片空白、成片画得出，而没有任何门禁会红**。
//
// # 判据怎么咬住"绘制路径存在"（而不是"文件里出现过某个词"）
//
// **不看源码文本**，而是给每个桶喂一条**该类型的真事件**，跑**预览真正会调的那个求值器**，
// 要求它的输出与「空工程基线」**可观测地不同**。
//   · `shapecount`：输出是数组 ⇒ 比长度；输出是数/串 ⇒ 比值
//   · `probe`：拿该求值器产出的**子值**（如 `videoFxAt(...).camera.zoom`）比
// 判据的"红"是真的红：改坏绘制路径 ⇒ 输出退回基线 ⇒ `different` 为假 ⇒ fail。
//
// ⚠️ **`null` 也算一种输出**（`getOverlayState` 无事件时回 `null`）——
//    所以基线比较用 `JSON.stringify` 逐值比，不用真值判断（`0` 与 `null` 都是合法基线）。
//
// ⚠️ **每条探针的事件形状必须真的能触发该路径**（时间落在窗口内、必需字段给全）——
//    给错了会**假红**。三条踩过的：
//      · `shake` 不给 `intensity` 时 `getShake` 回 `{0,0}`（与基线相同 ⇒ 误判成"没画"）；
//      · `pulse` 的可视窗口只有 **0.48s**，`t` 取 1.0 时早已出窗（同样误判）；
//      · `hue_shift` 在**空**索引上 `getHueShiftFlash` 会抛（`idx.hue_shift` 未按需创建）——
//        所以基线那一侧要用 `buildIndex` 的正常产物，别拿 `{}`。
const DRAW_PROBES = {
  // 桶名 → { ev: 触发事件, at: 采样时刻, read: (mod, idx) => 该桶的可见输出 }
  camera: {
    ev: { type: 'camera', time: 0.5, end: 3, zoom: '近' },
    at: 0.55,
    read: (m, idx) => m.videoFxAt(idx, 0.55).camera.zoom,
  },
  sticker: {
    ev: { type: 'sticker', time: 0.5, duration: 3, name: '生气' },
    at: 0.55,
    read: (m, idx) => m.stickersAt(idx, 0.55).length,
  },
  shake: {
    // ⚠️ `intensity` 必给：不给时 `getShake` 与基线同为 `{0,0}`
    ev: { type: 'shake', time: 0.5, duration: 3, end: 3, intensity: 14 },
    at: 0.55,
    read: (m, idx) => JSON.stringify(m.videoFxAt(idx, 0.55).shake),
  },
  flash: {
    ev: { type: 'flash', time: 0.5, duration: 3, end: 3 },
    at: 0.55,
    read: (m, idx) => m.getFlash(idx, 0.55).opacity,
  },
  blur: {
    ev: { type: 'blur', time: 0.5, duration: 3, end: 3 },
    at: 0.55,
    read: (m, idx) => m.videoFxAt(idx, 0.55).blur,
  },
  overlay: {
    // ⚠️ `overlay_type` 必给：没有它 `getOverlayState` 回 `null`（与基线同）
    ev: { type: 'overlay', time: 0.5, end: 3, overlay_type: 'color', color: '#ff0000' },
    at: 0.55,
    read: (m, idx) => JSON.stringify(m.getOverlayState(idx, 0.55)),
  },
  vignette: {
    ev: { type: 'vignette', time: 0.5, duration: 3, end: 3 },
    at: 0.55,
    read: (m, idx) => m.getVignette(idx, 0.55),
  },
  hue_shift: {
    ev: { type: 'hue_shift', time: 0.5, duration: 3, end: 3 },
    at: 0.55,
    read: (m, idx) => m.getHueShiftFlash(idx, 0.55).opacity,
  },
  noise: {
    ev: { type: 'noise', time: 0.5, duration: 3, end: 3 },
    at: 0.55,
    read: (m, idx) => m.getNoiseOpacity(idx, 0.55),
  },
  pulse: {
    // ⚠️ 可视窗口只有 0.48s（`getPulse` 里写死），`at` 必须落在窗内
    ev: { type: 'pulse', time: 0.5, duration: 3, end: 3 },
    at: 0.55,
    read: (m, idx) => m.videoFxAt(idx, 0.55).zoom,
  },
  sfx: {
    ev: { type: 'sfx', time: 0.5, duration: 1, name: 'pop-notice' },
    at: 0.55,
    // 音效的"画出来"= 索引里那条 soundEvents（`audio.js::sync` 消费它）
    read: (m, idx) => idx.soundEvents.length,
  },
  element: {
    // ⚠️⚠️ **2026-10-14 分离之后，这个探针必须用 `kind: 'text'`**。
    //
    //    原先它用的是 `kind: 'sticker'`（那时贴纸条目确实住在 `idx.element` 里）。
    //    分家之后那些条目在**索引期**就归一到 `idx.sticker` 了，
    //    `idx.element` **只剩文字** ⇒ 再拿贴纸去探这个桶，恒为 0。
    //
    //    ⚠️ 探针失败时**会被当成"桶是死的"**（判据报"登记了但画不出来"）——
    //      那是**误报**：桶没死，是**探针的输入形状过时了**。
    //      所以探测事件要跟着**桶里装什么**走，不能只跟着桶名走。
    ev: {
      type: 'element', kind: 'text', text: '生气',
      time: 0.5, end: 3.5, x: '50%', y: '47%', pos: 'bottom',
      source: 'hook|80|6|255,255,255,255|26,26,26,255|700|0',
    },
    at: 0.55,
    read: (m, idx) => m.getElements(idx, 0.55).length,
  },
};

/** 用一条事件建一份最小工程的索引（`duration` 给足，事件落在 0.5s）。 */
function indexWith(mod, ev) {
  return mod.buildIndex({
    config: { segments: [{ start: 0, end: 10, events: [ev] }], duration: 10 },
    subtitles: [], danmaku: [], duration: 10,
  });
}

/** 空工程基线索引（**正常 `buildIndex` 产物**，不是 `{}` —— 见 `hue_shift` 那条注）。 */
function emptyIndex(mod) {
  return mod.buildIndex({
    config: { segments: [{ start: 0, end: 10, events: [] }], duration: 10 },
    subtitles: [], danmaku: [], duration: 10,
  });
}

/// 逐桶跑一遍绘制探针，返回 `{ bucket: { withEv, base, different } }`。
/// **抽成函数**是为了变异自证能拿变异后的模块跑**同一份**判据。
function drawPathReport(mod, buckets) {
  const base = emptyIndex(mod);
  const out = {};
  for (const name of buckets) {
    const probe = DRAW_PROBES[name];
    if (!probe) {
      out[name] = { withEv: null, base: null, different: false, noProbe: true };
      continue;
    }
    let withEv, baseVal;
    try { withEv = JSON.stringify(probe.read(mod, indexWith(mod, probe.ev))); }
    catch (e) { withEv = `THREW:${e && e.message}`; }
    try { baseVal = JSON.stringify(probe.read(mod, base)); }
    catch (e) { baseVal = `THREW:${e && e.message}`; }
    out[name] = { withEv, base: baseVal, different: withEv !== baseVal };
  }
  return out;
}

// ---------------------------------------------------------------------------
// 已知死桶基线 —— **每一条都必须写清"为什么它现在画不出来"以及"谁在修"**
// ---------------------------------------------------------------------------
//
// 本仓的房规（见 `check-dhampir-drop-list.mjs`）：**"清单变了"这件事本身就该有人看一眼**，
// 所以把已知缺陷钉成**基线**：多了 / 少了都红，改基线要顺手写清理由。
//
// ⚠️ 基线的**方向性**很重要 —— 这条钉的不是"允许画不出来"，而是：
//   · 一个**不在**基线里的桶画不出来 ⇒ **红**（新缺陷，必须立刻看见）；
//   · 一个**在**基线里的桶**画得出来了** ⇒ **红**（修好了 ⇒ 把它从基线删掉，
//     否则这条基线会永远替一个已经修好的洞打掩护）。
//
// 这条基线存在的理由：修 `engine.js` 是 **task-26** 的写范围
// （它已登记 `webui/src/polish/engine.js` + `webui/dist`，且要求"真机可见"的同帧对比证据）——
// 本任务（task-25）只修**判据**，不动产品行为。⇒ 这条基线是**task-25 → task-26 之间的交接凭据**：
// task-26 改完 `engine.js` 后，本门禁会因"`element` 已能画出来"而**主动报红提醒删基线**。
// ── 2026-10-12：`element` **已从基线删除**（task-26 修好了 sticker 支）─────────────
//
// 删除依据：本门禁"在基线里 + 能画出来" ⇒ 主动报红要求删（上面那条方向性判据）。
// ✅ 删除时 `element` 的 sticker 支**确实能画了**（真跑 `getElements`：`src` 非空、
//    `opacity/scale/rot` 走出 backOut 四阶段动画）。
//
// ⚠️ **但"能画的只是 sticker 支"** —— `kind === "text"` 仍然**有意留空**，那是 §5 E3
//    （`engine.js::getElements` 上方的注释：文字要动态盒算落点、**故意不近似**，
//    且"转译器会在报告里明说这条，所以这里静默是对的"）。
//    ⇒ **别把这条删除理解成"element 全好了"**：文字支**从来不是"被掐断"，是"有意不做"**。
//    下一个接手 text 的人要自己把 `buildIndex` 的 element 分支与 `getElements` 的文字支接起来。
//
// ★ 修的时候发现**四处**成因，不是两处（下面这份记录保留全部四处，供后来人对照）：
//    (a) `if (kind !== "sticker") continue` —— 恒真，整条通道出局；
//    (b) **字段路径错位**：读 `e.kind` / `e.text`，而 `buildIndex` 只把载荷放在 `e.ev.*`；
//        ⚠️ 同函数的 `e.time` / `e.duration` 是**对的**（那两个确实在 entry 上，
//        `stickersAt` 同口径）⇒ **绝不能全局替换 `e.` → `e.ev.`**，那会把贴纸弄坏。
//    (c) ★ **一个恒真的兜底**：`(idx.stickerSrc || (() => ''))(...)` ——
//        但 `idx` 上**从来没有** `stickerSrc`（`buildIndex` 只把结果落到 `entry.src`）
//        ⇒ 兜底恒生效、`src` **恒为空串**。**它让 (a)(b) 的修复看起来无效**。
//    (d) ★★ **`buildIndex` 的 element 分支从来不设 `entry.src`**（只有 `sticker` 分支设）
//        ⇒ 就算 (a)(b)(c) 都修好，图源仍然没有。**这一处才是"贴纸画不出来"的最后一道**。
//    ⇒ 教训：`guard` 交来的"(a)(b) 两处"是**从静态阅读推出的**；(c)(d) 只有**真跑一遍**才会露出来
//      （我是靠"修完 (a)(b) 之后 `src` 仍然空"这一步才挖下去的）。
const KNOWN_DEAD_BUCKETS = {
  // 2026-10-12：`element` 已修复并删除 —— 见上方说明。
};

async function main() {
  const mod = await import('../webui/src/polish/engine.js');
  const buckets = mod.INDEXED_TYPES;
  if (!Array.isArray(buckets) || buckets.length === 0) fail('engine.js 没有导出 INDEXED_TYPES（桶清单只许有一份）');
  checks += 1;

  // ---- ① 桶清单：engine.js ↔ 模板 必须同一个集合 ----
  const tpl = templateBuckets(readFileSync(TEMPLATE, 'utf8'));
  const eng = new Set(buckets);
  const onlyTpl = [...tpl].filter((n) => !eng.has(n));
  const onlyEng = [...eng].filter((n) => !tpl.has(n));
  if (onlyTpl.length || onlyEng.length) {
    fail(
      `桶清单两边不一致（两份实现必须同一个集合）：\n` +
        `  只有模板有：${onlyTpl.join(', ') || '(无)'}\n` +
        `  只有 engine.js 有：${onlyEng.join(', ') || '(无)'}`,
    );
  }
  checks += 1;
  console.log(`  ✓ 桶清单两边一致（${buckets.length} 个：${buckets.join(', ')}）`);

  // ---- ② 空工程建索引：每个桶都必须是数组（不是 undefined）----
  const empty = mod.buildIndex({ config: { segments: [] }, subtitles: [], danmaku: [], duration: 10 });
  const missing = buckets.filter((t) => !Array.isArray(empty[t]));
  if (missing.length) {
    fail(
      `空工程下这些桶不是数组：${missing.join(', ')}\n` +
        `  ⇒ 工程里没有该类型事件时它就是 undefined，求值器一迭代就抛（预览整块消失）`,
    );
  }
  checks += 1;
  console.log(`  ✓ 空工程的每一个桶都是数组（这正是事故的触发条件）`);

  // ---- ②b ★ 「登记了」必须伴随「画得出来」（维度漏洞的补丁）----
  //
  // 上面 ①②③ 只证明"桶登记了"；这条证明"每个桶都有一条**能画出东西**的路径"。
  // 判据 = 喂一条真事件后，预览求值器的输出必须与空工程基线**可观测地不同**。
  const report = drawPathReport(mod, buckets);

  // 探针覆盖：**每个桶都必须有一条探针**，否则新加的桶会从缝里溜过去
  const noProbe = Object.entries(report).filter(([, r]) => r.noProbe).map(([n]) => n);
  if (noProbe.length) {
    fail(
      `这些桶在 INDEXED_TYPES 里，但**没有绘制探针**：${noProbe.join(', ')}\n` +
        `  这条判据必须覆盖每一个桶，否则新加的桶会从缝里溜过去。\n` +
        `  请在 scripts/check-preview-engine.mjs 的 DRAW_PROBES 里补一条（并说明预览在哪画它）。`,
    );
  }
  checks += 1;
  console.log(`  ✓ ${buckets.length} 个桶都有绘制探针（判据的覆盖面是全的）`);

  // ⚠️ 判据的**两个方向**（见 KNOWN_DEAD_BUCKETS 的注释）
  const dead = Object.entries(report).filter(([, r]) => !r.different).map(([n]) => n);
  const unexpectedDead = dead.filter((n) => !KNOWN_DEAD_BUCKETS[n]);
  const unexpectedlyAlive = Object.keys(KNOWN_DEAD_BUCKETS).filter((n) => report[n] && report[n].different);

  if (unexpectedDead.length) {
    fail(
      `这些桶「登记了」但**画不出来**，且**不在**已知基线里：\n` +
        unexpectedDead
          .map((n) => `  · ${n}: 有事件=${report[n].withEv}  基线=${report[n].base}`)
          .join('\n') +
        `\n  ⇒ 症状 = 预览里看不到、成片里画得出，且过去**没有任何门禁会红**。\n` +
        `  ⇒ 要么补上绘制路径，要么（确属已知缺陷时）登记进 KNOWN_DEAD_BUCKETS 并写清 who/why。`,
    );
  }
  if (unexpectedlyAlive.length) {
    fail(
      `这些桶**已经能画出来了**，但仍挂在 KNOWN_DEAD_BUCKETS 里：${unexpectedlyAlive.join(', ')}\n` +
        `  ⇒ 缺陷修好了就该把它从基线里删掉 —— 留着会让这条基线**永远替一个已经修好的洞打掩护**。`,
    );
  }
  checks += 1;
  const alive = buckets.length - dead.length;
  console.log(
    `  ✓ ${alive}/${buckets.length} 个桶**能画出来**（喂真事件 ⇒ 输出与空基线不同）` +
      (dead.length ? `；${dead.length} 个在已知基线里（${dead.join(', ')} —— 见脚本内 who/why）` : ''),
  );

  // ---- ②c 词汇表的 `dom_preview` 必须与上面那条**实测**结论一致 ----
  //
  // ⚠️ 这条是 ②b 的"出口检查"：`dom_preview: true` 是**词汇表对外的承诺**，
  //    而 ②b 是**跑出来的事实**。两者不一致时，登记表就在骗下一个人
  //    （§20 的 noise 就是这么骗过去的：写着 `dom_preview: true`，实际画的是白纱）。
  //
  // 判据：`dom_preview === true` ⇒ 该桶**必须**能画出来；画不出来就红。
  // 反向（`dom_preview: false` 但能画出来）**只报告不判红** —— 见下面那段说明。
  const spec2 = JSON.parse(readFileSync('spec/effects.json', 'utf8'));
  const lying = [];
  const understated = [];
  for (const e of spec2.effects) {
    const r = report[e.id];
    if (!r || r.noProbe) continue; // 桶与效果不是一一对应时跳过（由 ②b 的覆盖面判据负责）
    if (e.dom_preview === true && !r.different) lying.push(e.id);
    if (e.dom_preview === false && r.different) understated.push(e.id);
  }
  if (lying.length) {
    fail(
      `这些效果在 spec/effects.json 里写着 \`dom_preview: true\`，但**预览画不出来**：${lying.join(', ')}\n` +
        `  ⇒ 登记表承诺了预览支持，实际没有 —— 下一个人会照它行事，这正是 §20 那条能一路绿灯的原因。\n` +
        `  ⇒ 要么补上绘制路径，要么把 \`dom_preview\` 改成 false 并写清原因。`,
    );
  }
  checks += 1;
  console.log(`  ✓ 词汇表的 \`dom_preview: true\` 都对得上实测（没有"承诺了却没实现"的条目）`);
  if (understated.length) {
    // ⚠️ **只报告，不判红**（刻意）：`dom_preview: false` 但实际能画出来，属于"登记得比事实保守"，
    //    不会误导任何人。而且现状**确实有一条**（`sfx`：它经 `idx.soundEvents` → `audio.js::sync`
    //    在预览里真的会响，却登记成 false）。把它判红会**在本任务范围外**制造一条红门禁
    //    —— 那属于 task-25 之外的整改，留给拥有 `spec/effects.json` 的那一批。
    console.log(
      `  ⚠️ 另有 ${understated.length} 项登记得比事实保守（\`dom_preview: false\` 但能画出来）：${understated.join(', ')}` +
        ` —— 不判红（不会误导人），但建议对齐。`,
    );
  }

  // ---- ③ 预览那一族求值器：逐个真跑，不许抛 ----
  for (const name of IDX_T_FAMILY) {
    const fn = mod[name];
    if (typeof fn !== 'function') fail(`engine.js 没导出 ${name}（预览会 import 它）`);
    checks += 1;
    try {
      fn(empty, 1.0);
    } catch (e) {
      fail(`${name}(idx, 1.0) 抛了：${e && e.message}`);
    }
  }
  console.log(`  ✓ ${IDX_T_FAMILY.length} 个求值器在空工程上都能跑（不抛）`);

  // ---- ④ `videoFxAt` 的返回字段就是预览的契约 ----
  const vfx = mod.videoFxAt(empty, 1.0);
  for (const field of ['shake', 'blur', 'camera', 'zoom']) {
    checks += 1;
    if (vfx[field] === undefined) {
      fail(`videoFxAt(...).${field} 是 undefined —— 预览读它（如 \`f.camera.handheld\`）会抛，整块预览消失`);
    }
  }
  if (vfx.camera === null || typeof vfx.camera !== 'object') fail('videoFxAt(...).camera 必须是个对象（预览读 .handheld/.ox/.oy）');
  checks += 1;
  console.log(`  ✓ videoFxAt 返回 ${Object.keys(vfx).join(', ')}（camera.handheld=${vfx.camera.handheld}）`);

  // ---- ⑤ 词汇表的 `ui` 必须是数组（编辑器 `.includes` 的前提）----
  const spec = JSON.parse(readFileSync('spec/effects.json', 'utf8'));
  const badUi = spec.effects.filter((e) => e.ui !== undefined && !Array.isArray(e.ui));
  if (badUi.length) {
    fail(`这些效果的 ui 不是数组：${badUi.map((e) => `${e.id}=${JSON.stringify(e.ui)}`).join(', ')} —— 编辑器会抛 .includes is not a function`);
  }
  checks += 1;
  console.log(`  ✓ 词汇表 ${spec.effects.length} 个效果的 ui 都是数组（或缺省）`);
}

// ---------------------------------------------------------------------------
// 自证：往空索引里删一个桶，那一族求值器**必须**抛 —— 否则上面那些断言是空转的
// ---------------------------------------------------------------------------
async function selfTest() {
  const mod = await import('../webui/src/polish/engine.js');
  const idx = mod.buildIndex({ config: { segments: [] }, subtitles: [], danmaku: [], duration: 10 });
  delete idx.vignette;
  let threw = false;
  try {
    mod.getVignette(idx, 1.0);
  } catch {
    threw = true;
  }
  if (!threw) fail('自证失败：删掉 `vignette` 桶之后 getVignette 竟然没抛 —— 说明判据是空转的');
  delete idx.camera;
  let threw2 = false;
  try {
    mod.videoFxAt(idx, 1.0).camera.handheld;
  } catch {
    threw2 = true;
  }
  if (!threw2) fail('自证失败：删掉 `camera` 桶之后 videoFxAt(...).camera 竟然没抛');
  console.log('✓ 预览引擎守卫自证：删桶之后那两族求值器都会抛（判据不是空转的）');
}

// ---------------------------------------------------------------------------
// 变异自证：**把绘制路径改坏 ⇒ ②b 必须红**
// ---------------------------------------------------------------------------
//
// # 为什么要真改源码、真加载变异体
//
// 本仓已经**四次**出现"变异该红没红"，每次抓到的都是**判据的洞**。
// 所以这里不比对文本，而是：
//   1. 把 `webui/src/polish/engine.js` 的源码**真改**一处（写在系统临时目录，**不动仓库文件**）；
//   2. 断言替换**确实命中 1 次**（少了这条，"变异体"可能根本没变，看着像"抓不住"）；
//   3. 用 `import()` **真加载**变异体（每次都是新 URL ⇒ 模块缓存不串）；
//   4. 拿它跑**同一份** `drawPathReport`，要求目标桶**从"能画"翻成"画不出来"**。
//
// ⚠️ 第 4 步必须与 `main()` 里那条判据是**同一段代码** —— 否则自证证明的是另一件事。
async function mutate() {
  const src = readFileSync(ENGINE, 'utf8');
  let n = 0;

  // 一处变异 = `{ label, from, to, bucket }`：
  // 把 `from` 换成 `to` 后，`bucket` 那条探针**必须**从 different:true 翻成 false。
  const MUTATIONS = [
    {
      label: '§element 的洞本身：`getElements` 里 `kind !== "sticker"` 提前 continue（**原始缺陷 a**）',
      from: "    if (kind !== 'sticker') continue\n",
      to: "    continue\n",
      bucket: 'element',
    },
    {
      label: '§element 的洞之二：字段路径错位 `e.kind` →（索引里其实是 `e.ev.kind`）',
      from: "    const kind = String(e.kind ?? '').trim().toLowerCase()\n",
      to: "    const kind = String(e.kind ?? '').trim().toLowerCase()\n    if (String(e.kind ?? '').trim() === '') continue\n",
      bucket: 'element',
    },
    {
      label: '`getElements` 干脆不往 out 里推（画不出来但也不抛 —— 静默空白）',
      from: '    out.push({ src: (idx.stickerSrc || (() => \'\'))(e.text || \'\'), opacity, scale, rot, yOff })\n',
      to: '    void 0\n',
      bucket: 'element',
    },
    {
      // ⚠️ 这条锚点必须**比一行长**：`if (t < et || t > et + dur) continue` 在 engine.js 里
      // 出现**两次**（`getElements` 与 `stickersAt` 各一次）—— 只写那一行会命中 2 次，
      // 被上面的"命中 1 次"断言当场拦下（这是它该有的行为，我第一版就是这么写错的）。
      label: '`stickersAt` 的窗口条件写反（贴纸永远出不来）',
      from: '  for (const e of idx.sticker) {\n    const et = e.time, dur = e.duration || 2.5\n    if (t < et || t > et + dur) continue\n',
      to: '  for (const e of idx.sticker) {\n    const et = e.time, dur = e.duration || 2.5\n    if (true) continue\n',
      bucket: 'sticker',
    },
    {
      label: '`getNoiseOpacity` 恒返回 0（噪点桶登记了、画不出来）',
      from: 'export function getNoiseOpacity(idx, t) {\n',
      to: 'export function getNoiseOpacity(idx, t) {\n  return 0\n',
      bucket: 'noise',
    },
    {
      label: '`getFlash` 恒返回 0 不透明度',
      from: 'export function getFlash(idx, t) {\n',
      to: 'export function getFlash(idx, t) {\n  return { opacity: 0, color: \'#ffffff\' }\n',
      bucket: 'flash',
    },
  ];

  for (const mut of MUTATIONS) {
    const hits = src.split(mut.from).length - 1;
    if (hits !== 1) {
      fail(
        `变异自证失败：「${mut.label}」的锚点在 engine.js 里命中 ${hits} 次（应为 1）。\n` +
          `  ⇒ 变异台写错了（锚点漂了），这次自证是**空转**的 —— 修锚点，别放过。`,
      );
    }
    const mutated = src.replace(mut.from, mut.to);
    if (mutated === src) fail(`变异自证失败：「${mut.label}」替换后文本没变`);

    const dir = mkdtempSync(join(tmpdir(), 'preview-engine-mut-'));
    let report;
    try {
      const file = join(dir, 'engine-mutated.mjs');
      writeFileSync(file, mutated, 'utf8');
      // ⚠️ `engine.js` 有 `import ... from './canvas.js'` —— 变异体放在临时目录里，
      // 相对依赖也得**一起放**，否则 `ERR_MODULE_NOT_FOUND`（我第一版就栽在这）。
      // 依赖是**原样复制**的（只变异目标文件），所以这是干净的对照。
      for (const dep of ENGINE_DEPS) {
        copyFileSync(resolve('webui/src/polish', dep), join(dir, dep));
      }
      const mutMod = await import(pathToFileURL(file).href);
      report = drawPathReport(mutMod, mutMod.INDEXED_TYPES);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }

    const r = report[mut.bucket];
    if (!r) fail(`变异自证失败：变异后找不到桶 \`${mut.bucket}\``);
    if (r.different) {
      fail(
        `变异自证失败：「${mut.label}」\n` +
          `  ⇒ 桶 \`${mut.bucket}\` 的绘制探针**仍然认为画得出来**（有事件=${r.withEv} 基线=${r.base}）——\n` +
          `     说明这条判据**抓不住**这类回归，它是空转的。`,
      );
    }
    n += 1;
    console.log(`  ✓ 变异 #${n} 打红：${mut.label}\n      ↳ 桶 \`${mut.bucket}\` 翻成"画不出来"（有事件=${r.withEv} vs 基线=${r.base}）`);
  }

  // 反向自证：**没变异**的源码必须仍然全绿 —— 否则上面的"翻面"可能只是判据恒假
  const clean = drawPathReport(await import(pathToFileURL(ENGINE_ABS).href), (await import(pathToFileURL(ENGINE_ABS).href)).INDEXED_TYPES);
  const deadUnexpected = Object.keys(clean).filter((k) => !clean[k].different && !KNOWN_DEAD_BUCKETS[k]);
  if (deadUnexpected.length) {
    fail(
      `变异自证的反向控制失败：**未变异**的 engine.js 上这些桶也画不出来：${deadUnexpected.join(', ')}\n` +
        `  ⇒ 说明"画不出来"不是因为变异，判据本身有问题。`,
    );
  }
  console.log(`  ✓ 反向控制：未变异的 engine.js 上除已知基线（${Object.keys(KNOWN_DEAD_BUCKETS).join(', ')}）外全部能画出来`);

  console.log(`\n✓ 变异自证：${n}/${MUTATIONS.length} 处变异全部打红（判据不是空转的）`);
}

if (process.argv.includes('--self-test')) {
  await selfTest();
} else if (process.argv.includes('--mutate')) {
  await mutate();
} else {
  await main();
  console.log(`✓ 预览引擎守卫：${checks} 项全绿（空工程不炸 / 求值器不抛 / 视频层字段齐 / 两份桶清单一致 / **每个桶都有能画出它的路径**）`);
}
