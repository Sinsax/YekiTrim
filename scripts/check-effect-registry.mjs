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
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const read = (rel) => readFileSync(join(repo, rel), 'utf8');

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
      return new Set([...text.matchAll(/case '([a-z_]+)':/g)].map((m) => m[1]));
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
    case 'post_span':
      return new Set([...text.matchAll(/"([a-z_]+)"\s*=>/g)].map((m) => m[1]));
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

  console.log(`✓ 词汇表门禁自证：${checks} 项（多一个 / 少一个 / 少了底座锚点，都必须红且点名）`);
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
