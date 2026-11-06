#!/usr/bin/env node
// 门禁：**效果词汇表只有一个权威**（`spec/effects.json`），凡是"手抄了一份类型清单"的地方都必须与它一致。
//
// # 为什么需要它
//
// 加/删一个精修效果，历史上要人肉同步 **11+ 处**（见 `docs/plan/effect-pipeline-unification.md` 的清点）：
// 类型真源 / 后处理时长表 / LLM 提示词 / 模板 canvas / `engine.js` / 前端三张表 / 转译器 / 守卫 / 文档。
// 没有一处是权威 ⇒ **漏一处不报错**，症状是"某个通道少画一个效果"或"LLM 永远不生成新类型"。
// 这条门禁把"漏了哪一处"变成一句话，把"增删效果"从"记得改 11 处"变成"跑到绿为止"。
//
// # 判据（两向都查）
//
//   ① `spec/effects.json` 里的每个 id，该消费者必须**认得**；
//   ② 该消费者认得的 id，必须**在词汇表里**（防止删了词汇表、消费点还留着）。
// 覆盖不到的地方就是"待派生"清单：前端三张表与转译器白名单应当**直接读这份 JSON**（下一批），
// 那样这两向判断会天然成立（本门禁退化为只守"读没读"）。
//
// # 跑法
//
//     node scripts/check-effect-registry.mjs
//     node scripts/check-effect-registry.mjs --self-test   # 自证：多一个/少一个都必须红（不碰真文件）
import { strict as assert } from 'node:assert';
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const read = (rel) => readFileSync(join(repo, rel), 'utf8');

/** 递归读一棵目录下所有源码文件（用于"锚点有没有消费者"那条判据）。
 *  ⚠️ 只收源码后缀，跳过 `node_modules` / `dist` / `target` —— 那些是生成物，
 *     拿它们当"消费者"会让判据恒真（dist 是 src 的产物，src 删了 dist 还在）。
 *  ⚠️ **定义文件自己不在消费范围内**（调用方传的是别的目录），否则"定义了就算有人用"。 */
function readTree(rootRel, lang, selfFile) {
  const exts = { js: ['.js', '.mjs', '.vue'], html: ['.html'], rs: ['.rs'] }[lang] ?? ['.js'];
  const skip = new Set(['node_modules', 'dist', 'target', '.git', 'assets']);
  const out = [];
  const walk = (rel) => {
    let entries;
    try {
      entries = readdirSync(join(repo, rel), { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      const next = `${rel}/${e.name}`;
      if (e.isDirectory()) {
        if (!skip.has(e.name)) walk(next);
      } else if (exts.some((x) => e.name.endsWith(x))) {
        // ⚠️ 定义文件自己不算消费者（理由见调用点的长注释）
        if (selfFile && next === selfFile) continue;
        try { out.push(read(next)); } catch { /* 读不了就跳过，别让判据因此报红 */ }
      }
    }
  };
  walk(rootRel);
  return out.join('\n');
}

/** 去掉注释后再抽取（注释里也写着 `handlers.xxx` 这种示例，裸匹配会误命中）。 */
const stripComments = (text, lang) => {
  if (lang === 'rs') {
    return text
      .split(/\r?\n/)
      .filter((l) => !/^\s*\/\//.test(l))
      .join('\n');
  }
  // js / vue / html：整行 `//` 与 `/* */` 块
  return text
    .split(/\r?\n/)
    .filter((l) => !/^\s*(\/\/|\*|\/\*)/.test(l))
    .join('\n');
};

const camelToSnake = (s) => s.replace(/([a-z0-9])([A-Z])/g, '$1_$2').replace(/([A-Z]+)([A-Z][a-z])/g, '$1_$2').toLowerCase();

/** 从源码里抽出一组 id（每个消费者一个适配器；**适配器是门禁的一部分，要跟着消费点的形态改**）。 */
function extract(consumer, registry) {
  const text = stripComments(read(consumer.file), consumer.lang);
  switch (consumer.kind) {
    case 'rust-enum': {
      // `pub enum Event { Shake { … }, … }` —— 变体名 camel→snake
      const at = text.indexOf(consumer.anchor);
      assert.ok(at >= 0, `${consumer.file} 里找不到 ${consumer.anchor}（锚点过时了）`);
      const body = text.slice(at, text.indexOf('\n}', at));
      return new Set([...body.matchAll(/^\s{4}([A-Z]\w*)\s*[{,]/gm)].map((m) => camelToSnake(m[1])));
    }
    case 'rust-match-strings': {
      // `"shake" => 0.5,`
      return new Set([...text.matchAll(/"([a-z_]+)"\s*=>/g)].map((m) => m[1]));
    }
    case 'js-switch-cases': {
      // ⚠️ **P2（2026-10）：手写 `case` 之外，还要算上"声明式那条路"。**
      //    `spec/effects.json` 里有 `dhampir_rule` 的效果**故意不再**留 `case`
      //    （两处都算会让"改了词汇表没生效"，而症状是成片节奏与声明不符、报告零差异）。
      //    ⇒ 这里把「有 `dhampir_rule`」的效果并进来，否则它们会被误判成"转译器缺这一档"。
      const hand = new Set([...text.matchAll(/case '([a-z_]+)':/g)].map((m) => m[1]));
      for (const e of registry?.effects ?? []) if (e.dhampir_rule) hand.add(e.id);
      return hand;
    }
    case 'js-handlers': {
      // `handlers.shake = function(ev, et) {`
      return new Set([...text.matchAll(/handlers\.([a-z_]+)\s*=\s*function/g)].map((m) => m[1]));
    }
    case 'js-array': {
      // `const FX_TYPES = ['shake', …]`
      const at = text.indexOf(consumer.anchor);
      assert.ok(at >= 0, `${consumer.file} 里找不到 ${consumer.anchor}`);
      const body = text.slice(at, text.indexOf(']', at));
      return new Set([...body.matchAll(/'([a-z_]+)'/g)].map((m) => m[1]));
    }
    case 'js-object-keys': {
      // `const DEFAULT_SPAN = { camera: 1.0, … }`
      const at = text.indexOf(consumer.anchor);
      assert.ok(at >= 0, `${consumer.file} 里找不到 ${consumer.anchor}`);
      const body = text.slice(at, text.indexOf('}', at));
      return new Set([...body.matchAll(/(^|[{,]\s*)([a-z_]+)\s*:/g)].map((m) => m[2]));
    }
    case 'js-functions': {
      // `function getShake(idx, t)` / `export function getOverlayState(…)` —— 名字 → id 用别名表
      const names = [...text.matchAll(/function (get[A-Za-z0-9_]+)\(/g)].map((m) => m[1]);
      const ids = new Set();
      for (const n of names) {
        ids.add(consumer.aliases[n] ?? camelToSnake(n.replace(/^get/, '')));
      }
      return ids;
    }
    case 'derives': {
      // **派生检查**（S3）：这些表不许再手写，必须是"读词汇表算出来的"。
      // 判据 = 文件里同时出现"导入词汇表"与每个派生表达式；少了任何一个就说明有人把它抄回来了。
      const text = read(consumer.file);
      const missing = consumer.need.filter((n) => !text.includes(n));
      return { derives: true, missing };
    }
    case 'mentions': {
      // 没有清单形态的消费点（LLM 提示词、文档）：逐个 id 断言"出现过"
      const missing = registry.effects.filter((e) => !text.includes(e.id)).map((e) => e.id);
      return { mentions: true, missing };
    }
    default:
      throw new Error(`未知的消费者类型：${consumer.kind}`);
  }
}

function loadRegistry() {
  return JSON.parse(read('spec/effects.json'));
}

const CONSUMERS = [
  {
    name: '类型真源（yekitrim-render-core::Event 枚举）',
    file: 'crates/yekitrim-render-core/src/types.rs',
    lang: 'rs',
    kind: 'rust-enum',
    anchor: 'pub enum Event',
  },
  {
    name: '前端四张表 + 两个分组（**必须派生自词汇表**）',
    file: 'webui/src/components/TimelineEditor.vue',
    lang: 'vue',
    kind: 'derives',
    // 加/删效果时这里**不该有人动**：这几个表达式就是"读词汇表"的证据。
    need: [
      '../../../spec/effects.json',
      "EFFECTS.filter((e) => (e.lane ?? 'fx') === 'fx')",
      'Object.fromEntries(EFFECTS.map((e) => [e.id, e.label]))',
      'e.sfx_capable === true',
      "uiGroup('intensity')",
      "uiGroup('color')",
      'Object.fromEntries(EFFECTS.map((e) => [e.id, e.default_span]))',
      'const LANE_TYPES = (lane) =>',
      "types: LANE_TYPES('fx')",
    ],
  },
  {
    name: '转译器 switch（底座通道）',
    file: 'tools/polish-to-dhampir.mjs',
    lang: 'js',
    kind: 'js-switch-cases',
  },
  {
    name: 'LLM 提示词（**必须派生自词汇表**）',
    file: 'crates/yekitrim-polish/src/llm/segment.rs',
    lang: 'rs',
    kind: 'derives',
    // 提示词里两份类型清单（system 与 user）以前是手写的；现在都从词汇表算。
    need: [
      'include_str!("../../../../spec/effects.json")',
      'fn fx_type_list(',
      'fn durable_type_list(',
      'fx_list, durable_list',
    ],
  },
  {
    name: '事件参考文档',
    file: 'docs/polish-events-reference.md',
    lang: 'md',
    kind: 'mentions',
  },
];

const allIds = (registry) => registry.effects.map((e) => e.id);

// ---------------------------------------------------------------------------
// 锚点检查：① 登记的锚点必须真的存在；② 那些文件里不许有"没人认领"的实现
// ---------------------------------------------------------------------------
const ANCHOR_SITES = {
  canvas: { file: 'crates/yekitrim-polish/templates/index.html', lang: 'html' },
  dom_preview: { file: 'webui/src/polish/engine.js', lang: 'js' },
  dhampir_case: { file: 'tools/polish-to-dhampir.mjs', lang: 'js' },
  post_span: { file: 'crates/yekitrim-polish/src/process/fx.rs', lang: 'rs' },
};

/** 某个文件里"看起来像效果实现"的名字（用于孤儿检测）。 */
function orphanNames(site, text) {
  switch (site) {
    case 'canvas':
      return new Set([...text.matchAll(/handlers\.([a-z_]+)\s*=\s*function/g)].map((m) => `handlers.${m[1]}`));
    case 'dom_preview':
      // 效果求值器一律 `get*`（`cameraAt`/`stickersAt`/`sfxVolume` 由锚点显式登记 ⇒ 见下）
      return new Set([...text.matchAll(/function (get[A-Za-z0-9_]+)\(/g)].map((m) => m[1]));
    case 'dhampir_case':
      return new Set([...text.matchAll(/case '([a-z_]+)':/g)].map((m) => m[1]));
    case 'post_span': {
      // ⚠️⚠️ **只扫两张登记表**（`transient_duration_cap` / `transient_default_duration`）。
      //
      // 旧写法扫的是**整份 fx.rs** 的 `"名字" =>` —— 2026-10-20「补位三类型轮换」
      // （`b720aa6` / `79d9d3d`）给自动补位加了候选别名 `"overlay_hue" => Some("overlay")`
      // 与 `"overlay_hue" => Event::Overlay{..}`：那是**候选别名**（造出来的是 overlay），
      // 不是独立效果实现 ⇒ 旧写法把它当孤儿（**假红**）。
      //
      // 两张表是"效果 id → 数值"的权威登记：表里出现没人认领的 id 才是真的孤儿
      // （加了效果却忘了写进词汇表 / 删了效果却没删表项）。
      const bodies = ['transient_duration_cap', 'transient_default_duration']
        .map((fn) => {
          const at = text.indexOf(`fn ${fn}`);
          if (at < 0) return '';
          const end = text.indexOf('\n}', at);
          return end < 0 ? text.slice(at) : text.slice(at, end);
        })
        .join('\n');
      return new Set([...bodies.matchAll(/"([a-z_]+)"\s*=>/g)].map((m) => m[1]));
    }
    default:
      return new Set();
  }
}

// ---------------------------------------------------------------------------
// **不变量（S4 条件③，2026-10-01 程序化）：每个效果都必须有"底座那条腿"的实现锚点**
//
// 背景：删掉第二条渲染链路（canvas 出片 + DOM 预览）的前置条件之一是
// 「没有任何效果只能靠兜底通道出片」。以前这条靠人肉统计（每加一个效果都要重新数一遍），
// 现在把它变成**红了就说明不变量被破坏**：词汇表里某个效果没有 `anchors.dhampir_case`
// ⇒ 它在底座出片里根本不存在，只能靠兜底腿画 —— 那正是"删不得"的状态。
//
// ⚠️ 反过来**不**要求 `canvas` / `dom_preview` 一定存在：兜底腿被删掉之后那两个锚点本就该消失
// （存在时仍由 `checkAnchors` 校验"锚点真的在那个文件里"）。
/// 形状级判据：`ui` **必须是数组**（或缺省）。
///
/// 2026-10-01 事故：`overlay.ui` 被写成对象 `{}` ⇒ 编辑器里 `(e.ui ?? []).includes(name)`
/// 抛 `(intermediate value).includes is not a function` ⇒ 整个编辑器 setup 失败、整块界面消失，
/// 而**服务端日志全是 200**。这类"形状不对"用派生表达式查不出来（`uiGroup` 的实现本身没错），
/// 所以单独钉一条；预览引擎守卫（`check-preview-engine.mjs`）里还有一条同样判据兜底。
function checkUiShape(registry) {
  const failures = [];
  let checks = 0;
  for (const effect of registry.effects) {
    checks += 1;
    if (effect.ui !== undefined && !Array.isArray(effect.ui)) {
      failures.push(
        `效果 \`${effect.id}\` 的 \`ui\` 不是数组（实得 ${JSON.stringify(effect.ui)}）—— ` +
          '前端按数组用（`.includes`），形状错了会让整块界面消失',
      );
    }
  }
  return { checks, failures };
}

function checkDhampirCoverage(registry) {
  const failures = [];
  let checks = 0;
  for (const effect of registry.effects) {
    checks += 1;
    const anchor = registry.anchors.dhampir_case?.[effect.id];
    if (!anchor) {
      failures.push(
        `效果 \`${effect.id}\` 没有 anchors.dhampir_case —— 它只能靠兜底通道出片（S4 条件③ 被破坏）：` +
          '补上底座映射，或把这个效果从词汇表里删掉',
      );
    }
  }
  return { checks, failures };
}

/// **可达性判据：锚点光"在那个文件里存在"不算数，必须**真的被消费者调用到**。
///
/// ⚠️ 这条补的是一个**真实的维度漏洞**（2026-10-12，由 §5 E4a 撞出来）。
///
/// 原判据只有 `text.includes(anchor)` —— 在**定义文件**里做子串匹配。
/// 它回答的是"这个名字出现过吗"，**从来不问"有没有人用"**。
/// ⇒ 死代码照样全绿。真实案例：`getElements` 在 `engine.js` 里定义得好好的、
///    词汇表也认领了它，但**整个 `webui/src` 里零个调用点** ——
///    结果是 `element` 事件在预览里一个字都不画，而门禁是绿的。
///
/// 判据：锚点名必须出现在**定义文件之外**的某个消费者里（`import` 或调用）。
/// ⚠️ **刻意做粗**（"有没有别的文件用它"），不做数据流分析：
///    目的是拦住"导出了但没人用"这种一眼可见的洞，不是证明运行时一定执行到。
///    真正的"画得对不对"由各自的运行期判据管（`check-preview-engine.mjs` 等）。
function checkAnchorsAreReached(site, meta, wanted, failures, registry) {
  let checks = 0;
  // 消费者范围：前端预览锚点找 `webui/src`，转译器/后处理锚点找它们各自的调用方。
  const CONSUMER_GLOBS = {
    dom_preview: ['webui/src'],
    dhampir_case: ['tools', 'crates/yekitrim-polish/src'],
    post_span: ['crates/yekitrim-polish/src'],
    canvas: ['crates/yekitrim-polish/templates'],
  };
  // ⚠️ `canvas` 是**自包含**的：`handlers.xxx` 的定义与派发都在 `index.html` 里，
  //    **没有跨文件消费者** ⇒ 对它做"别的文件提没提到"是错的。
  //    它的可达性判据是「**同文件里有没有按类型取用 handler**」。
  //    ⚠️ 实际写法是**两行**（`var fn=handlers[ev.type];` 然后 `fn(ev,et);`），
  //       不是 `handlers[...](...)` 一行 —— 第一版按一行写，于是把整个 canvas 判成死代码（假红）。
  const SELF_CONTAINED = { canvas: /handlers\s*\[[^\]]+\]/ };
  const roots = CONSUMER_GLOBS[site] ?? [];
  // ⚠️⚠️ **必须把"定义文件自己"从消费语料里剔掉** —— 否则判据恒真。
  //    第一版就是漏了这一步：`dom_preview` 的消费范围写的是 `webui/src`，
  //    而 `engine.js` **就在 `webui/src` 里** ⇒ 定义行本身就算"有人引用" ⇒
  //    我把 `getVignette` 的**唯一消费者删掉**之后门禁**照样全绿**（实测确认过）。
  //    ⇒ **纪律：写"有没有人用它"这类判据时，第一件事是把"它自己"排除掉。**
  const corpus = roots
    .map((r) => readTree(r, meta.lang, meta.file))
    .join('\n');
  for (const [id, anchor] of Object.entries(wanted)) {
    if (anchor === null) continue;
    checks += 1;
    // 自包含 site：判据是"定义文件里有没有派发点"（不是别的文件提没提到）。
    if (SELF_CONTAINED[site]) {
      const self = stripComments(read(meta.file), meta.lang);
      if (!SELF_CONTAINED[site].test(self)) {
        failures.push(
          `锚点**不可达**：\`${site}\` 是自包含的，但 ${meta.file} 里**没有按类型派发**` +
            `（期望 \`${SELF_CONTAINED[site]}\`）—— 定义了却没人调，效果不会出画`,
        );
      }
      continue;
    }
    if (!corpus) continue; // 找不到消费目录时**不报红**（那是判据配置问题，不是实现问题）
    // `handlers.flash` 这种按 `handlers[type]` 派发，取最后一段即可。
    const bare = anchor.includes('.') ? anchor.split('.').pop() : anchor;
    const esc = bare.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const re = new RegExp(`(?<![\\w.])${esc}(?![\\w])`);
    // ⚠️⚠️ **"被别的文件引用"只是入口的一种**。锚点也可能是**同文件内的间接调用**
    //    （`getShake`/`getBlur`/`sfxVolume`/`cameraAt` 全是被 `videoFxAt`/`buildIndex` 调的，
    //     而 `videoFxAt` 才是预览直接消费的那个）。
    //    ⇒ 判据分两级：① 定义文件之外有人提到它；② 否则看它在**自己文件里有没有调用位**。
    const outsideUsed = re.test(stripComments(corpus, meta.lang));
    let insideUsed = false;
    if (!outsideUsed) {
      const self = stripComments(read(meta.file), meta.lang);
      // ⚠️ 定义文件里的"使用形态"按 site 不同：
      //   * `dhampir_case` / `post_span` 是 **switch/match 派发**（`case 'flash':` / `"flash" =>`），
      //     它**不是调用**、后面也不跟 `(` ⇒ 光找 `name(` 会把整个转译器判成死代码（实测假红）。
      //   * 其余的（`dom_preview`）是普通函数 ⇒ 找调用位 `name(`，且**不算它自己的定义行**。
      if (site === 'dhampir_case') {
        // ⚠️ **P2（2026-10）：两条路都算"有调用位"** ——
        //   ① 手写 `case 'flash':`（声明式表达不了的：发关键帧/图层、按帧循环、多档分支）；
        //   ② `spec/effects.json` 里有 `dhampir_rule`（声明式那条路，转译器按形状求值）。
        // 只认 ① 的话，P2 把四条 case 搬进词汇表之后会**假红**（"锚点没有消费者"）——
        // 而它们其实由**更可靠**的一条路驱动（声明 + 门禁双向钉着）。
        insideUsed = new RegExp(`case\\s+'${esc}'\\s*:`).test(self);
        if (!insideUsed) {
          const rule = (registry.effects ?? []).find((e) => e.id === id)?.dhampir_rule;
          if (rule) {
            // 声明式：转译器里必须有那个形状的派发器。
            // ⚠️ 认**两种写法**：`===`（正向支持表）与 `!==`（"只实现了这一种形状"的守卫）。
            //    转译器用的是后者 —— 它把"认得但没实现"也变成**当场抛错**，比清单更不容易漏。
            //    只认 `===` 会**假红**（实测就是这么撞上的）。
            const shapeLit = rule.shape.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
            insideUsed = new RegExp(
              `shape\\s*(?:===|!==)\\s*'${shapeLit}'`,
            ).test(self);
            if (!insideUsed) {
              failures.push(
                `\`${id}\` 声明了 \`dhampir_rule.shape="${rule.shape}"\`，但 ${meta.file} 里` +
                  `**没有形状为 "${rule.shape}" 的派发器** —— 声明了却没人执行，等于没接（且不报错）`,
              );
            }
          }
        }
      } else if (site === 'post_span') {
        insideUsed = new RegExp(`"${esc}"\\s*=>`).test(self);
      } else {
        const callRe = new RegExp(`${esc}\\s*\\(`, 'g');
        const defRe = new RegExp(
          `(?:function\\s+${esc}\\s*\\(|(?:const|let|var)\\s+${esc}\\s*=\\s*(?:async\\s*)?(?:\\(|function))`,
        );
        insideUsed = [...self.matchAll(callRe)].length > (defRe.test(self) ? 1 : 0);
      }
    }
    if (!outsideUsed && !insideUsed) {
      failures.push(
        `锚点**没有消费者**：\`${anchor}\`（词汇表 ${id} → ${site}）既没被 ${roots.join(' / ')} 引用、` +
          `在 ${meta.file} 里也没有调用位 —— 它就是死代码：门禁以前只在定义文件里查"名字在不在"，这种洞照样绿`,
      );
    }
  }
  return checks;
}

function checkAnchors(registry) {
  const failures = [];
  let checks = 0;
  for (const [site, meta] of Object.entries(ANCHOR_SITES)) {
    const text = stripComments(read(meta.file), meta.lang);
    const wanted = registry.anchors[site];
    const claimed = new Set();
    for (const [id, anchor] of Object.entries(wanted)) {
      if (anchor === null) continue; // 显式"不支持"
      claimed.add(anchor);
      checks += 1;
      if (!text.includes(anchor)) {
        failures.push(`锚点对不上：${meta.file} 里找不到 \`${anchor}\`（词汇表 ${id} → ${site}）`);
      }
    }
    checks += checkAnchorsAreReached(site, meta, wanted, failures, registry);
    // 孤儿：文件里有、词汇表没认领
    const orph = [...orphanNames(site, text)].filter((n) => !claimed.has(n));
    // `dom_preview` 的 `*At`/`sfxVolume` 形态不在 `get*` 正则里 ⇒ 天然不会误报；
    // `handlers.xxx` 这种示例行由 stripComments 挡掉（注释行）。
    for (const name of orph) {
      failures.push(`没人认领的实现：${meta.file} 的 \`${name}\` —— 要么加进 spec/effects.json 的 anchors.${site}，要么删掉`);
    }
    checks += 1;
  }
  return { checks, failures };
}

/** 值级一致性：**同一组数字散在三处**（fx.rs 两张表 + 前端 DEFAULT_SPAN）—— 这里把三处钉在一起。 */
function checkValues(registry) {
  const failures = [];
  let checks = 0;
  const fx = read('crates/yekitrim-polish/src/process/fx.rs');
  const timeline = read('webui/src/components/TimelineEditor.vue');

  /** 从 `fn(…) { "id" => v, … }` 里取一张表（⚠️ fx.rs 有**两张**表，必须按函数名分锚，不能一把抓）。 */
  const table = (src, fnName) => {
    const at = src.indexOf(`fn ${fnName}`);
    assert.ok(at >= 0, `fx.rs 里找不到 fn ${fnName}（锚点过时了）`);
    const body = src.slice(at, src.indexOf('\n}', at));
    return new Map([...body.matchAll(/"([a-z_]+)"\s*=>\s*([0-9.]+),/g)].map((m) => [m[1], Number(m[2])]));
  };
  const caps = table(fx, 'transient_duration_cap');
  const spans = table(fx, 'transient_default_duration');

  const spanAt = timeline.indexOf('const DEFAULT_SPAN = {');
  const uiSpans = new Map(
    [...timeline.slice(spanAt, timeline.indexOf('}', spanAt)).matchAll(/([a-z_]+)\s*:\s*([0-9.]+)/g)].map((m) => [m[1], Number(m[2])]),
  );
  void uiSpans; // 已派生（见上面 `derives` 那条），不再有字面量可对

  // **Rust 侧的默认时长**（`Event::default_end` 的 match 表）：这是原先第三份拷贝。
  const rustTypes = read('crates/yekitrim-render-core/src/types.rs');
  const rustAt = rustTypes.indexOf('pub fn default_end');
  assert.ok(rustAt >= 0, 'types.rs 里找不到 default_end');
  const rustBody = rustTypes.slice(rustAt, rustTypes.indexOf('\n    }', rustAt));
  const rustDefaults = new Map(
    [...rustBody.matchAll(/Event::([A-Z]\w*)\s*\{\s*\.\.\s*\}\s*=>\s*([0-9.]+)/g)].map((m) => [
      camelToSnake(m[1]),
      Number(m[2]),
    ]),
  );
  console.log(
    `  （值级：fx.rs 上限 ${caps.size} / fx.rs 默认 ${spans.size} / Rust default_end ${rustDefaults.size}）`,
  );
  for (const e of registry.effects) {
    checks += 3;
    // fx.rs 的两张表**只覆盖瞬时特效**（durable 的时长由事件自己的 end 决定）⇒ 只对 fx 判。
    if (e.kind === 'fx') {
      if (e.duration_cap === null) {
        if (caps.has(e.id)) failures.push(`fx.rs 的 transient_duration_cap 里给 \`${e.id}\` 写了上限，词汇表说它没有`);
      } else if (caps.get(e.id) !== e.duration_cap) {
        failures.push(`时长**上限**不一致：\`${e.id}\` 词汇表 ${e.duration_cap} / fx.rs ${caps.get(e.id)}`);
      }
      if (spans.get(e.id) !== e.default_span) {
        failures.push(`默认**时长**不一致：\`${e.id}\` 词汇表 ${e.default_span} / fx.rs ${spans.get(e.id)}`);
      }
    } else if (caps.has(e.id) || spans.has(e.id)) {
      failures.push(`fx.rs 的两张表里出现了非瞬时效果 \`${e.id}\`（那是 durable 类型，时长由事件自己给）`);
    }
    // `sfx` 在 Rust 里是 0.0（"用文件原始长度"的合法语义）⇒ 特殊对待。
    const rustWant = e.id === 'sfx' ? 0 : e.default_span;
    if (rustDefaults.get(e.id) !== rustWant) {
      failures.push(`Rust \`Event::default_end\` 不一致：\`${e.id}\` 词汇表 ${rustWant} / types.rs ${rustDefaults.get(e.id)}`);
    }
  }
  return { checks, failures };
}

function compare(consumer, registry, consumerIds) {
  const problems = [];
  const want = registry.effects.filter(consumer.only ?? (() => true));
  if (consumerIds.derives) {
    for (const n of consumerIds.missing) problems.push(`这里缺"读词汇表"的那一句：\`${n}\``);
    return problems;
  }
  if (consumerIds.mentions) {
    for (const id of consumerIds.missing) problems.push(`词汇表里有 \`${id}\`，这里**一个字都没提**`);
    return problems;
  }
  for (const e of want) {
    if (!consumerIds.has(e.id)) problems.push(`词汇表里有 \`${e.id}\`（${e.label}），这里缺`);
  }
  for (const id of consumerIds) {
    if (!allIds(registry).includes(id)) problems.push(`这里认得 \`${id}\`，但词汇表里没有（删之前先删它）`);
    else if (consumer.only && !want.some((e) => e.id === id)) problems.push(`\`${id}\` 不该出现在这里（按词汇表的分类过滤掉了）`);
  }
  return problems;
}

// ---------------------------------------------------------------------------
// **字段级判据（P0，2026-10）：`fields` 必须与 Rust 事件变体逐字段一致**
//
// # 为什么需要它
//
// 词汇表原本只管"事件**类型**"，不管"事件**字段**" —— 给一个效果加字段要人肉查 6 处
// （`types.rs` / `legacy.rs` / `engine.js` / 模板 / 转译器 / 面板控件 + `config.rs`），
// 而且**零门禁覆盖**：漏一处不报错，症状是"预览里有、成片里没有"。
//
// 现在每个效果用 `fields` 声明它的**私有**字段，本判据把它与 Rust `pub enum Event` 的
// 对应变体逐字段对拍：**Rust 有而没声明**、**声明了而 Rust 没有**，两种都点名报红。
//
// ⚠️ **公共头**（`field_common`：time/end/anchor/sfx/sfx_volume/manual）两边都要排除 ——
//    它们出现在多数变体上，不属于任何单个效果。
// ⚠️ 判据的**方向**：Rust 是这一轮的实现权威（`fields` 是描述，不是目标）。
//    所以"声明了但 Rust 没有"也是错 —— 那说明描述漂了（本次 P0 就靠它抓出 5 个假字段）。
function checkFields(registry) {
  const failures = [];
  let checks = 0;
  const common = new Set((registry.field_common || []).map((f) => f.name));

  // 读 Rust 的 Event 枚举，抽出每个变体的字段名
  let variants;
  try {
    const src = read('crates/yekitrim-render-core/src/types.rs');
    const body = src.match(/pub enum Event \{([\s\S]*?)\n\}/);
    if (!body) throw new Error('找不到 `pub enum Event {`');
    variants = new Map();
    for (const m of body[1].matchAll(/^\s{4}([A-Z][A-Za-z]*)\s*\{([\s\S]*?)^\s{4}\}/gm)) {
      // 变体名 SnakeCase：HueShift → hue_shift（与词汇表 id 同口径）
      const snake = m[1].replace(/([a-z0-9])([A-Z])/g, '$1_$2').toLowerCase();
      variants.set(
        snake,
        [...m[2].matchAll(/^\s+([a-z_]+)\s*:/gm)].map((x) => x[1]),
      );
    }
  } catch (e) {
    return { checks: 1, failures: [`字段级判据：读 Rust 类型真源失败（${e.message}）`] };
  }

  for (const effect of registry.effects) {
    checks += 1;
    const rust = variants.get(effect.id);
    if (!rust) {
      failures.push(
        `效果 \`${effect.id}\`：Rust \`pub enum Event\` 里找不到对应变体 —— ` +
          '要么 id 拼错，要么这个效果只活在词汇表里（后者等于"声明了但没人实现"）',
      );
      continue;
    }
    // ⚠️ 先做**形状**判据再做内容比 —— 形状错时下面那句 `.map` 会抛，
    //    而抛出的表现是"门禁崩了"而不是"点名哪里错"（自证第 ③ 条抓到的就是这个）。
    checks += 1;
    if (effect.fields !== undefined && !Array.isArray(effect.fields)) {
      failures.push(
        `效果 \`${effect.id}\` 的 \`fields\` 不是数组（实得 ${JSON.stringify(effect.fields)}）—— ` +
          '前端/门禁都会按数组用，形状错了会让整块界面消失（与 `ui` 那条同源事故）',
      );
      continue;
    }
    const fieldArr = effect.fields || [];
    for (const f of fieldArr) {
      checks += 1;
      if (!f || typeof f.name !== 'string' || !f.name) {
        failures.push(`效果 \`${effect.id}\` 的 \`fields\` 里有一项没有 \`name\`：${JSON.stringify(f)}`);
      }
    }
    const declared = new Set(fieldArr.map((f) => f.name));
    const rustPrivate = rust.filter((n) => !common.has(n));
    const missing = rustPrivate.filter((n) => !declared.has(n));
    const extra = [...declared].filter((n) => !rust.includes(n) && !common.has(n));
    if (missing.length || extra.length) {
      const parts = [];
      if (missing.length) parts.push(`Rust 有但没声明：[${missing.join(', ')}]`);
      if (extra.length) parts.push(`声明了但 Rust 没有：[${extra.join(', ')}]`);
      failures.push(
        `效果 \`${effect.id}\` 的 \`fields\` 与 Rust 变体不一致 —— ${parts.join('；')}\n` +
          '      改法：以 `crates/yekitrim-render-core/src/types.rs` 的实现为准修 `fields`；' +
          '若确实是新加的字段，先改 Rust 再改词汇表（或反过来，但两边必须同一个改动里一起动）',
      );
    }

    // —— `dhampir_rule.collect.fields` 的判据（P4，2026-10）——
    //
    // 声明式收集段（`camera_rule`）靠 `collect.fields` 指名要搬哪些字段。
    // 名单写错**不会报错**：搬不到就是 `undefined`，而下游 `?? 兜底` 会把它变成
    // 一个"看着正常"的值（`tilt` 变 0、`speed` 变 undefined）。
    // ⇒ 判据两条：① 名单里的每个名字都必须是**已声明字段**；② 不许漏掉 Rust 私有字段
    //    （漏了 = 那个字段静默不进成片，正是 P0 抓出的 `camera.tilt` 那类缺陷）。
    const collect = effect.dhampir_rule?.collect?.fields;
    if (collect !== undefined) {
      checks += 1;
      if (!Array.isArray(collect)) {
        failures.push(
          `效果 \`${effect.id}\` 的 \`dhampir_rule.collect.fields\` 不是数组` +
            `（实得 ${JSON.stringify(collect)}）—— 转译器会按数组遍历，形状错了整条收集段空转`,
        );
      } else {
        const unknown = collect.filter((n) => !declared.has(n) && !common.has(n));
        if (unknown.length) {
          failures.push(
            `效果 \`${effect.id}\` 的 \`dhampir_rule.collect.fields\` 里有**没声明的字段**：` +
              `[${unknown.join(', ')}] —— 转译器搬不到它（静默 undefined），` +
              '下游的 `??` 兜底会把它变成"看着正常"的值',
          );
        }
        // ⚠️ 只查"没声明"不够：**漏搬**才是 P0 那个 `camera.tilt` 的形状
        //    （字段在 Rust/词汇表里都有，就是没人读 ⇒ 进不了成片）。
        const notCarried = [...declared].filter((n) => !collect.includes(n));
        if (notCarried.length) {
          failures.push(
            `效果 \`${effect.id}\` 有字段**没进 \`collect.fields\`**：[${notCarried.join(', ')}] —— ` +
              '它们不会被搬进收集记录 ⇒ **静默不进成片**（报告里一行差异都没有）。' +
              '若某个字段是**派生值**（如 `origin` 被解析成点），也要在名单里、由转译器显式跳过',
          );
        }
      }
    }
  }
  return { checks, failures };
}

function run(registry, { quiet = false } = {}) {
  let checks = 0;
  const failures = [];
  const anchors = checkAnchors(registry);
  checks += anchors.checks;
  failures.push(...anchors.failures);
  const coverage = checkDhampirCoverage(registry);
  checks += coverage.checks;
  failures.push(...coverage.failures);
  const uiShape = checkUiShape(registry);
  checks += uiShape.checks;
  failures.push(...uiShape.failures);
  const fields = checkFields(registry);
  checks += fields.checks;
  failures.push(...fields.failures);
  const values = checkValues(registry);
  checks += values.checks;
  failures.push(...values.failures);
  for (const consumer of CONSUMERS) {
    let ids;
    try {
      ids = extract(consumer, registry);
    } catch (e) {
      failures.push(`${consumer.name}：抽取失败（${e.message}）`);
      continue;
    }
    const problems = compare(consumer, registry, ids);
    checks += 1;
    if (problems.length === 0) {
      if (!quiet) console.log(`  ✓ ${consumer.name}`);
      continue;
    }
    failures.push(`${consumer.name}（${consumer.file}）：\n      - ${problems.join('\n      - ')}`);
  }
  return { checks, failures };
}

function selfTest() {
  let checks = 0;
  const eq = (a, b, what) => {
    checks += 1;
    assert.equal(a, b, what);
  };
  const registry = loadRegistry();
  // ⚠️ **不要在这里写死类型个数**：它每加/删一个效果都要手工同步，而这条断言就是这么烂掉的
  // （2026-10-01 发现：词汇表早已 15 → 11，这里还写 15 ⇒ 自检长期是红的；又因为它没进 gates，
  //   红了半年也没人知道）。改成"下限 + 打出来"，并把自检本身接进 gates（见 .workbuddy/gates.mjs）。
  checks += 1;
  assert.ok(
    registry.effects.length >= 8,
    `词汇表类型太少（${registry.effects.length} 个）—— 防误删；若真的收敛到这个数量以下，改这个下限`,
  );
  console.log(`  · 词汇表当前 ${registry.effects.length} 个类型：${registry.effects.map((e) => e.id).join(', ')}`);
  eq(new Set(allIds(registry)).size, registry.effects.length, 'id 不许重复');

  // 多一个：所有消费点都必须报"缺"
  const plus = { ...registry, effects: [...registry.effects, { ...registry.effects[0], id: 'brand_new', label: '新效果' }] };
  const r1 = run(plus, { quiet: true });
  eq(r1.failures.length > 0, true, '词汇表多一个 → 必须报红');
  eq(
    r1.failures.some((f) => f.includes('brand_new')),
    true,
    '报红要**点名**新 id（而不是只说"不一致"）',
  );

  // 少一个：认得它的消费点必须报"多"
  const minus = { ...registry, effects: registry.effects.filter((e) => e.id !== 'noise') };
  const r2 = run(minus, { quiet: true });
  eq(r2.failures.length > 0, true, '词汇表少一个 → 必须报红');
  eq(
    r2.failures.some((f) => f.includes('noise')),
    true,
    '报红要点名那个还留着 `noise` 的消费点',
  );
  // ③ 不变量：把某个效果的 `dhampir_case` 摘成 null ⇒ 必须红且点名
  const noBase = {
    ...registry,
    anchors: { ...registry.anchors, dhampir_case: { ...registry.anchors.dhampir_case, shake: null } },
  };
  const r3 = run(noBase, { quiet: true });
  eq(r3.failures.length > 0, true, '摘掉某个效果的 dhampir_case → 必须红（S4 条件③ 不变量）');
  eq(
    r3.failures.some((f) => f.includes('shake') && f.includes('dhampir_case')),
    true,
    '报红要点名那个效果与缺失的锚点',
  );

  // ⑤ **声明式那条路自己也要能翻面**（P2，2026-10）。
  //
  // ⚠️ 由来：P2 把四条 `case` 搬进 `spec/effects.json` 的 `dhampir_rule` 之后，
  //    可达性判据**假红**（"锚点没有消费者"），而原因只是它只认手写 `case`。
  //    修法是"两条路都算"，但那样一来**判据本身**就有了新洞：
  //    某个效果只要在词汇表里挂个 `dhampir_rule`，就**再也不需要**转译器里有任何东西。
  //
  // ⇒ 所以这里补一条反向自检：**声明了形状、但转译器里没有那个形状的派发器 ⇒ 必须红**。
  //    这正是"声明了却没人执行"那类静默失效（症状：报告里一行差异都没有、特效一个都不转）。
  {
    // ⚠️ 拿 `flash` 当探针，因为它**没有手写 case**（P2 已搬进声明式）——
    //    于是"可达性"只能靠 `dhampir_rule.shape` 那条路满足。
    //    用 `shake` 试是**测不到东西**的：它有 `case 'shake':`，把 `dhampir_rule` 换成
    //    一个不存在的形状也照样绿（`case` 那条路先命中了）—— 第一版就是这么写的，当场假绿。
    const ghost = {
      ...registry,
      effects: registry.effects.map((e) =>
        e.id === 'flash' ? { ...e, dhampir_rule: { shape: 'no_such_shape_xyz', kind: 'flash', params: {} } } : e,
      ),
    };
    const fails = [];
    checkAnchorsAreReached('dhampir_case', ANCHOR_SITES.dhampir_case, ghost.anchors.dhampir_case, fails, ghost);
    eq(
      fails.some((f) => f.includes('no_such_shape_xyz')),
      true,
      '声明了不存在的形状 ⇒ 必须红并点名那个形状（"声明了却没人执行"）',
    );
  }

  // ⑥ **`collect.fields` 判据自己也要能翻面**（P4，2026-10）。
  //
  // ⚠️ 由来：`camera_rule.collect.fields` 有两个方向的错，而**两个都不报错**：
  //    ① 写了个**没声明的字段** ⇒ 搬不到 ⇒ `undefined` ⇒ 下游 `??` 兜底成"看着正常"的值；
  //    ② **漏掉一个字段** ⇒ 它静默不进成片（这正是 P0 抓出的 `camera.tilt` 的形状）。
  //    两个方向都要能翻面，否则这条判据等于装饰。
  {
    const withCollect = registry.effects.find((e) => e.dhampir_rule?.collect?.fields);
    eq(!!withCollect, true, '词汇表里至少要有一个 `collect.fields` 的效果（否则下面两条自检空转）');
    if (withCollect) {
      const id = withCollect.id;
      // ① 造假字段名
      const ghostField = {
        ...registry,
        effects: registry.effects.map((e) =>
          e.id === id
            ? { ...e, dhampir_rule: { ...e.dhampir_rule, collect: { fields: [...e.dhampir_rule.collect.fields, 'no_such_field_xyz'] } } }
            : e,
        ),
      };
      const f1 = checkFields(ghostField).failures;
      eq(
        f1.some((f) => f.includes('no_such_field_xyz')),
        true,
        '`collect.fields` 里有没声明的字段 ⇒ 必须红并点名',
      );
      // ② 漏掉一个字段
      const dropped = withCollect.dhampir_rule.collect.fields[0];
      const ghostDrop = {
        ...registry,
        effects: registry.effects.map((e) =>
          e.id === id
            ? { ...e, dhampir_rule: { ...e.dhampir_rule, collect: { fields: e.dhampir_rule.collect.fields.slice(1) } } }
            : e,
        ),
      };
      const f2 = checkFields(ghostDrop).failures;
      eq(
        f2.some((f) => f.includes(dropped)),
        true,
        '`collect.fields` 漏掉一个已声明字段 ⇒ 必须红并点名（否则它静默不进成片）',
      );
    }
  }

  // ④ **可达性判据本身不许空转**（2026-10-12 补）。
  //
  // ⚠️ 这条自检的由来：可达性判据第一版**漏了把定义文件自己从消费语料里剔掉** ——
  //    `dom_preview` 的消费范围是 `webui/src`，而 `engine.js` **就在里面**
  //    ⇒ 定义行自己就算"有人引用" ⇒ 我把 `getVignette` 的**唯一消费者删掉**之后
  //    门禁**照样全绿**（实测确认）。判据写错的表现是"看起来全过"，比报错难发现。
  //
  // 做法：喂一个**不可能被任何地方引用**的锚点名，要求判据必须翻红并点名。
  {
    const probe = { ...registry.anchors.dom_preview, __probe__: 'definitelyNotReferencedAnywhere12345' };
    const fails = [];
    checkAnchorsAreReached('dom_preview', ANCHOR_SITES.dom_preview, probe, fails, registry);
    eq(fails.length > 0, true, '可达性判据对"没人引用的锚点"必须报红');
    eq(
      fails.some((f) => f.includes('definitelyNotReferencedAnywhere12345')),
      true,
      '可达性判据报红时要**点名**那个锚点',
    );
  }

  // ⑤ **字段级判据本身不许空转**（P0，2026-10）。
  //
  // ⚠️ 为什么要自证：这条判据读的是 `types.rs` 的**正则抽取**（`^\s{4}Variant {` + `^\s+field:`）。
  //    抽取写错的表现是"永远抓不到"——两边都空 ⇒ 全部 `✓`，看着像"字段都对上了"。
  //    所以两个方向各喂一次假数据，要求**必须红且点名那个字段**。
  {
    // 方向一：词汇表声明一个 Rust 没有的字段
    const withBogus = {
      ...registry,
      effects: registry.effects.map((e) =>
        e.id === 'shake' ? { ...e, fields: [...(e.fields || []), { name: 'bogus_probe', type: 'f64' }] } : e,
      ),
    };
    const rb = checkFields(withBogus);
    eq(rb.failures.length > 0, true, '声明了 Rust 没有的字段 → 必须报红');
    eq(
      rb.failures.some((f) => f.includes('bogus_probe')),
      true,
      '字段级判据报红要**点名**那个字段（而不是只说"不一致"）',
    );

    // 方向二：词汇表**漏声明**一个 Rust 确实有的私有字段
    //         （做法：把 shake 的 fields 清空 ⇒ `intensity` 就成了"Rust 有但没声明"）
    const withMissing = {
      ...registry,
      effects: registry.effects.map((e) => (e.id === 'shake' ? { ...e, fields: [] } : e)),
    };
    const rm = checkFields(withMissing);
    eq(rm.failures.length > 0, true, '漏声明 Rust 有的字段 → 必须报红');
    eq(
      rm.failures.some((f) => f.includes('intensity')),
      true,
      '漏声明时报红要点名 `intensity`',
    );

    // 方向三：形状错（`fields` 写成对象）—— 与 `ui` 那条同源
    const withBadShape = {
      ...registry,
      effects: registry.effects.map((e) => (e.id === 'shake' ? { ...e, fields: { intensity: 1 } } : e)),
    };
    eq(checkFields(withBadShape).failures.length > 0, true, '`fields` 形状错（非数组）→ 必须报红');
  }

  // ⑦ **fx.rs 孤儿扫描的口径**（2026-10-30 修假红）。
  //
  // 旧写法扫整份 fx.rs 的 `"名字" =>`，于是自动补位的候选别名
  // `"overlay_hue" => Some("overlay")`（它造出来的是 overlay，不是独立效果）
  // 被当成"没人认领的实现" ⇒ 判据假红。
  // 现在只扫两张登记表 ⇒ 两个方向都要钉住：
  //   ① 表里出现没人认领的 id **必须**被检出（真孤儿）；
  //   ② 表外的候选别名**不许**被检出（假红不许回来）。
  {
    const fakeFx = `
pub fn transient_duration_cap(event_type: &str) -> f64 {
    match event_type {
        "shake" => 0.5,
        "ghost_effect_xyz" => 0.5,
        _ => f64::INFINITY,
    }
}
pub fn transient_default_duration(event_type: &str) -> f64 {
    match event_type {
        "shake" => 0.3,
        _ => 0.5,
    }
}
fn auto_fill_category(kind: &str) -> Option<&'static str> {
    match kind {
        "blur" => Some("blur"),
        "overlay_hue" => Some("overlay"),
        _ => None,
    }
}
fn make_auto_fill(kind: &str) -> u8 {
    match kind {
        "overlay_hue" => 1,
        _ => 0,
    }
}`;
    const orph = orphanNames('post_span', fakeFx);
    eq(orph.has('ghost_effect_xyz'), true, 'fx.rs 登记表里的真孤儿必须被检出');
    eq(orph.has('overlay_hue'), false, 'fx.rs 补位候选别名（overlay_hue）不许被当成孤儿（假红）');
  }

  console.log(
    `✓ 词汇表门禁自证：${checks} 项（多一个 / 少一个 / 少了底座锚点 / 锚点不可达 / ` +
      '字段多声明 / 字段漏声明 / 字段形状错 / fx.rs 孤儿扫描口径，都必须红且点名）',
  );
}

if (process.argv.includes('--self-test')) {
  selfTest();
  process.exit(0);
}

const registry = loadRegistry();
console.log(`效果词汇表权威：spec/effects.json（${allIds(registry).length} 个：${allIds(registry).join(', ')}）`);
const retiring = registry.effects.filter((e) => e.status === 'retiring').map((e) => e.id);
if (retiring.length) console.log(`  ⚠ 已定稿待删（docs/plan/fx-type-consolidation.md）：${retiring.join(', ')}`);
const { checks, failures } = run(registry);
if (failures.length) {
  console.error(`\n✗ ${failures.length} 处与词汇表不一致：\n  - ${failures.join('\n  - ')}`);
  console.error('\n  （加/删效果的正确顺序：先改 spec/effects.json，再跑到这条门禁变绿）');
  process.exit(1);
}
console.log(`\n✓ 全部 ${checks} 个消费点与词汇表一致`);
