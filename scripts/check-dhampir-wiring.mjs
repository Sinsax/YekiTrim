#!/usr/bin/env node
// 守「WebUI 调的东西」与「服务端 / 引擎提供的东西」**对得上**。
//
// # 为什么需要它
//
// 浏览器端实时预览把三处**隔着边界**的名字串在一起：
//
//   1. WebUI 调的 **HTTP 路径**  ←→  `routes.rs` 注册的路由
//   2. WebUI 引用的 **`/api` 前缀** ←→ 生产里 `nest("/api", …)` 那一层
//   3. WebUI 调的 **`engine.*` 方法** ←→ `engine.js` 里 `Engine` 真的有这个方法
//
// 三处都不会在编译期报错，而**错了的症状都是"点了没反应"** —— 最难查的那一类。
// 我自己在这条缝里死过一次：handler 返回给前端的 URL 少了 `/api`，服务端 146 条用例全绿、
// 只有真浏览器会 404。所以把它们变成一条**几毫秒跑完的静态检查**。
//
// 运行：node scripts/check-dhampir-wiring.mjs

import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { join, relative, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '..');

const ROUTES = join(repo, 'crates/vtrim-server/src/api/routes.rs');
const WEBUI_SRC = join(repo, 'webui/src');
// 与 Rust 侧 `wasm_dirs()` 同一套"同层目录"规则：Dhampir 与 V-Trim 并排。
const DHAMPIR = resolve(repo, '..', 'Dhampir');
const ENGINE_JS = join(DHAMPIR, 'web/engine.js');
// ---- 底座 wasm API 版本（形状变更的哨兵；2026-10-01 底座回执 §1 点名要断言）----
const HOST_API_RS = join(DHAMPIR, 'crates/dhampir-timeline/src/host_api.rs');
const PREVIEW_PKG_JS = join(DHAMPIR, 'crates/dhampir-wasm/www/pkg/dhampir_wasm.js');
const HOST_API_DOC = join(DHAMPIR, 'docs/host-api.md');
const PREVIEW_VUE = join(WEBUI_SRC, 'components/PolishPreview.vue');

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (/\.(vue|js|mjs|ts)$/.test(name)) out.push(p);
  }
  return out;
}

const failures = [];
let checks = 0;
function check(ok, label, detail) {
  checks += 1;
  if (!ok) failures.push(detail ? `${label}\n    ${detail}` : label);
}

// ---------------------------------------------------------------------------
// 底座 wasm API 版本（2026-10-01 回执 §1：wasm 返回体加了 4 个阴影键 ⇒ 4 → 5，属**形状变更**）
//
// 为什么必须跨仓断言：这个数对不上时**不会编译错、不会 500**，只会让"返回体字段少了/多了"
// 表现成"预览里某些效果不生效"（没有堆栈、只能靠猜）。底座那边是"字段增减就 +1"的纪律，
// 我们这边就必须有一个跟着它动的哨兵。
// ---------------------------------------------------------------------------
{
  const vue = readFileSync(PREVIEW_VUE, 'utf8');
  const ours = Number((vue.match(/const DHAMPIR_HOST_API\s*=\s*(\d+)/) ?? [])[1]);
  check(Number.isInteger(ours), '前端必须声明 DHAMPIR_HOST_API（形状变更的哨兵）', String(ours));
  const rs = readFileSync(HOST_API_RS, 'utf8');
  const theirs = Number((rs.match(/pub const HOST_API_VERSION:\s*u32\s*=\s*(\d+)/) ?? [])[1]);
  check(Number.isInteger(theirs), '底座 host_api.rs 里能读到 HOST_API_VERSION', String(theirs));
  check(
    ours === theirs,
    '底座 wasm API 版本与我们对齐（我们 ' + ours + ' / 底座 ' + theirs + '）',
    '形状变更（返回体字段增减）要两边一起动：改前端的 DHAMPIR_HOST_API，或在底座确认是兼容变更',
  );
  const pkg = existsSync(PREVIEW_PKG_JS) ? readFileSync(PREVIEW_PKG_JS, 'utf8') : '';
  check(pkg.includes('dhampir_host_api_version'), '底座 wasm glue 导出 dhampir_host_api_version', PREVIEW_PKG_JS);
  const doc = existsSync(HOST_API_DOC) ? readFileSync(HOST_API_DOC, 'utf8') : '';
  for (const key of ['shadow_color', 'shadow_dx_px', 'shadow_dy_px', 'shadow_blur_ratio']) {
    check(doc.includes(key), '底座契约文档写了 ' + key, HOST_API_DOC);
  }
  // ⚠️ 版本断言必须问**模块命名空间**，不能问 `loadEngine` 的返回值：底座那个函数返回的是
  // `new Engine(mod)` **包装对象**，`dhampir_host_api_version` 不在它身上。
  // 2026-10-01 我写成 `dhampirEngine.dhampir_host_api_version()` ⇒ 永远报"没有这个导出"，
  // 预览被**我们自己的断言**挡掉（用户只看到「当前是近似预览」）—— 静态判据抓不住这类"问错对象"，
  // 所以在这里钉两条：**不许**问包装对象、**必须**自己 import pkg 拿命名空间。
  // ⚠️ 判据必须走**去注释后的源码**：上面那段解释里就写着反面例子
  // （`dhampirEngine.dhampir_host_api_version()`），裸匹配会把注释自己顶翻。
  // ⚠️ 反过来，"必须自己 import"那条要测**原文** —— 去注释会把 `/* @vite-ignore */` 一起吃掉，
  // 于是正则永远匹配不上（第一版就是这么红的 ✗）。
  const vueCode = vue
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .split('\n')
    .map((l) => l.replace(/(^|[^:])\/\/.*$/, '$1'))
    .join('\n');
  check(
    !/dhampirEngine\.dhampir_host_api_version/.test(vueCode),
    '版本断言不许问 loadEngine 的返回值（那是 Engine 包装对象，没有这个导出）',
    PREVIEW_VUE,
  );
  check(
    /await import\(\/\* @vite-ignore \*\/ PKG_URL\)/.test(vue),
    '版本断言要自己 `await import(PKG_URL)` 拿模块命名空间（ESM 缓存 ⇒ 不会重复下载）',
    PREVIEW_VUE,
  );
}

const routesSrc = readFileSync(ROUTES, 'utf8');
const frontend = walk(WEBUI_SRC).map((p) => ({
  path: relative(repo, p),
  text: readFileSync(p, 'utf8'),
}));

// ---- 1/2：前端调的 dhampir 路径 <-> 服务端注册的路由 ----
const declared = new Set(
  [...routesSrc.matchAll(/\.route\("\/polish\/(dhampir-[a-z-]+)/g)].map((m) => m[1]),
);
const called = new Set();
for (const f of frontend) {
  for (const m of f.text.matchAll(/['"`]\/api\/polish\/(dhampir-[a-z-]+)/g)) called.add(m[1]);
}

check(declared.size > 0, 'routes.rs 里应当有 dhampir 路由', '一条都没找到 —— 是不是改名字了？');
for (const name of called) {
  check(
    declared.has(name),
    `前端调了 /api/polish/${name}，服务端必须有对应的路由`,
    'routes.rs 里没有 .route("/polish/' +
      name +
      '", …) —— 浏览器会落到 fallback（它返回 200 "未知接口"，最容易被当成"成功了"）',
  );
}
for (const name of declared) {
  check(
    called.has(name),
    `服务端注册了 /polish/${name}，前端应当在用它`,
    '没有任何 webui/src 下的引用它 —— 死路由，或者前端漏了接线',
  );
}

// ---- 3：前端引用的 dhampir 路径必须带 `/api`（生产是 nest("/api", …)）----
const barePolish = [];
for (const f of frontend) {
  for (const m of f.text.matchAll(/['"`](\/polish\/dhampir-)/g)) {
    barePolish.push(`${f.path}: ${m[1]}`);
  }
}
check(
  barePolish.length === 0,
  '前端引用的 dhampir 路径必须带 `/api` 前缀',
  `少了前缀（生产会 404）：\n    ${barePolish.join('\n    ')}`,
);

// ---- 4：前端调的 `engine.*` 方法，`Engine` 上必须真的有 ----
if (existsSync(ENGINE_JS)) {
  const engineSrc = readFileSync(ENGINE_JS, 'utf8');
  const methods = new Set(
    [...engineSrc.matchAll(/^\s{2}(?:async\s+)?([A-Za-z_]\w*)\s*\(/gm)].map((m) => m[1]),
  );
  const used = new Set();
  for (const f of frontend) {
    for (const m of f.text.matchAll(/\b(?:dhampirEngine|engine)\s*\.\s*([A-Za-z_]\w*)\s*\(/g)) {
      used.add(m[1]);
    }
  }
  // 只查与 Dhampir 引擎有关的那几个：`engine.` 这个名字在别处也可能被用。
  const DHAMPIR_METHODS = ['open', 'attach', 'resize', 'bindSource', 'seek', 'endFrame', 'firstFrame'];
  for (const name of used) {
    if (!DHAMPIR_METHODS.includes(name)) continue;
    check(
      methods.has(name),
      `前端调了 engine.${name}()，但 engine.js 的 Engine 上没有这个方法`,
      '拼错方法名只会在运行时报，症状是"点了没反应"',
    );
  }
  // ---- 5：`engine.js` 调的 **wasm 导出名**，`timeline_host.rs` 里必须真的有 ----
  //
  // 这是比"Engine 的方法存在"更下一层的不变量：Dhampir 那边把某个导出改名之后，
  // `engine.js` **不会编译报错**（它是 JS），只会在运行到那一条时才炸 ——
  // 而症状是"画面卡住/没反应"，从现象看不出是导出名的问题。
  //
  // 反过来也查：`timeline_host.rs` 里导出的 `dhampir_project_*` 若**没人调**，
  // 要么是引擎里漏接了一条，要么是 Dhampir 那边新加的还没接 —— 两种都该看见。
  const hostRs = join(DHAMPIR, 'crates/dhampir-wasm/src/timeline_host.rs');
  if (existsSync(hostRs)) {
    const hostSrc = readFileSync(hostRs, 'utf8');
    const exported = new Set(
      [...hostSrc.matchAll(/pub\s+(?:async\s+)?fn\s+(dhampir_\w+)/g)].map((m) => m[1]),
    );
    const calledInEngine = new Set(
      [...engineSrc.matchAll(/mod\.(dhampir_\w+)/g)].map((m) => m[1]),
    );
    for (const name of calledInEngine) {
      check(
        exported.has(name),
        `engine.js 调了 ${name}()，但 timeline_host.rs 里没有这个导出`,
        'wasm 导出改名后 JS 不会编译报错，只会在运行到那一条时才炸 —— 症状是"没反应"',
      );
    }
    check(
      calledInEngine.size > 0,
      'engine.js 应当调用 wasm 导出',
      '一个都没调到 —— 是不是 engine.js 换了调用方式？',
    );
  } else {
    console.log(`（提示：没找到 ${hostRs} —— 跳过"wasm 导出名"那一段）`);
  }
} else {
  console.log(`（提示：没找到 ${ENGINE_JS} —— 跳过"引擎方法/导出名"那两段）`);
}

// ---- 汇总 ----
// ---- 定版产物（见 docs/vtrim-integration.md §8）----
//
// 设了 `VTEDIT_DHAMPIR_PREVIEW` / `VTEDIT_DHAMPIR_CLI` 就按**产物**的标准查。
//
// 为什么这两条必须能红：产物缺一件的症状是"页面白屏 / 控制台 unreachable executed /
// 出片时才发现"—— 看起来全像渲染坏了，其实是**部署坏了**。
// **一条不会红的检查等于没检查**，所以这里对着**实际配置的那个目录**查：
// 把产物里任何一件删掉，这段就应当变红。
//
// 只用已有的 `readFileSync`（try/catch 当 exists），不新增 import ——
// 这个脚本刻意零依赖，并且跨平台路径用 `/` 拼（Node 在 Windows 上照样认）。
const artifactPreview = process.env.VTEDIT_DHAMPIR_PREVIEW || '';
if (artifactPreview) {
  // ⚠️ **`VERSION` 在产物根目录，不在 `preview/` 里** —— 打包脚本写的是 `<out>/VERSION`，
  // Rust 那侧也是 `preview.parent().join("VERSION")`。第一版我把 `VERSION` 列在这里，
  // 于是**任何**配置都会报"缺 VERSION" —— 那种"永远红"的检查看着像很严，其实是错的。
  const need = ['engine.js', 'pkg/dhampir_wasm.js', 'pkg/dhampir_wasm_bg.wasm', '../VERSION'];
  const missing = need.filter((rel) => {
    try {
      readFileSync(artifactPreview.replace(/[/\\]+$/, '') + '/' + rel);
      return false;
    } catch (e) {
      return true;
    }
  });
  check(
    missing.length === 0,
    'VTEDIT_DHAMPIR_PREVIEW 指的产物目录要齐全（engine.js / pkg 两件 / VERSION）',
    '缺：' + missing.join('、') + '（在 Dhampir 仓跑 `node scripts/package.mjs`）',
  );
}

const artifactCli = process.env.VTEDIT_DHAMPIR_CLI || '';
if (artifactCli) {
  let ok = true;
  try {
    readFileSync(artifactCli);
  } catch (e) {
    ok = false;
  }
  check(ok, 'VTEDIT_DHAMPIR_CLI 指的可执行文件要在', '找不到：' + artifactCli);
}

// ---- 两条腿的时间线契约：**屏蔽区间只许剪一次**（2026-09-30）----
//
// 这条缝不看名字看**语义**，因为它的失效方式很安静：转译器有一支 `--keep-muted`
// （时间线整条保留，屏蔽区间交给出片尾部那道 `cut_ranges` 按源时间剪），
// 预览那条腿**不能**用它（客户端靠"落在被掐掉的段里 = 没有对应帧"跳过屏蔽区间）。
//
// 两边都剪的后果实测过：12s 的片子带 3s 屏蔽，出片只有 **6.0s**（应 9.0s），
// 而且音轨是从整条源视频抽的、没跟着压缩 —— 第一刀之后全错位。
// 两边都不剪的后果更安静：屏蔽区间**照常播出来**、报告里一句都不说。
const DHAMPIR_RS = join(repo, 'crates/vtrim-polish/src/render/dhampir.rs');
const POLISH_API = join(repo, 'crates/vtrim-server/src/api/polish.rs');
const dhampirSrc = readFileSync(DHAMPIR_RS, 'utf8');
const polishApiSrc = readFileSync(POLISH_API, 'utf8');

// 出片腿：`render_video_only` 里那次 `prepare_project[_reporting](clip_dir, project_path, true)`
// （`_reporting` 那个变体额外把转译报告带回来给 `--strict` 用 —— 参数口径**必须一样**）
check(
  /prepare_project(?:_reporting)?\(\s*clip_dir\s*,\s*project_path\s*,\s*true\s*\)/.test(dhampirSrc),
  '出片腿必须传 `keep_muted = true`（整条时间线，屏蔽交给尾部那一剪）',
  'render/dhampir.rs 里找不到 `prepare_project[_reporting](clip_dir, project_path, true)` —— ' +
    '传 false 就是"转译时先掐一遍、尾部再剪一遍"（成片短一大截 + 音轨错位）',
);
// 预览腿：`polish_dhampir_project` 里那次必须 `false`
check(
  /prepare_project\(\s*&clip\s*,\s*&scratch\s*,\s*false\s*\)/.test(polishApiSrc),
  '预览腿必须传 `keep_muted = false`（压缩时间线，客户端要跳屏蔽区间）',
  'api/polish.rs 里找不到 `prepare_project(&clip, &scratch, false)` —— ' +
    '传 true 的话客户端拿不到"断点"，会一路播过屏蔽区间',
);

if (failures.length > 0) {
  console.error('✗ Dhampir 接线检查未通过：\n');
  for (const f of failures) console.error('  - ' + f);
  console.error(`\n  （共 ${checks} 项，失败 ${failures.length} 项）`);
  process.exit(1);
}
console.log(
  `✓ Dhampir 接线：${checks} 项全绿` +
    `（前端调 ${called.size} 条路径 / 服务端注册 ${declared.size} 条 / 前缀与引擎方法都对）`,
);
