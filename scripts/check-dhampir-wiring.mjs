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

const ROUTES = join(repo, 'crates/yekitrim-server/src/api/routes.rs');
const WEBUI_SRC = join(repo, 'webui/src');
// 与 Rust 侧 `wasm_dirs()` 同一套"同层目录"规则：Dhampir 与 YekiTrim 并排。
const DHAMPIR = resolve(repo, '..', 'Dhampir');
const ENGINE_JS = join(DHAMPIR, 'web/engine.js');
// ---- 底座 wasm API 版本（形状变更的哨兵；2026-10-01 底座回执 §1 点名要断言）----
const HOST_API_RS = join(DHAMPIR, 'crates/dhampir-timeline/src/host_api.rs');
const PREVIEW_PKG_JS = join(DHAMPIR, 'crates/dhampir-wasm/www/pkg/dhampir_wasm.js');
const HOST_API_DOC = join(DHAMPIR, 'docs/host-api.md');
const PREVIEW_VUE = join(WEBUI_SRC, 'components/PolishPreview.vue');
const TIMELINE_EDITOR = join(WEBUI_SRC, 'components/TimelineEditor.vue');
const POLISH_VIEW = join(WEBUI_SRC, 'views/PolishView.vue');
/// 精修编辑器的服务端 handler —— **读旁写字幕**那条链住在这里（见 4e-bis）。
const POLISH_RS = join(repo, 'crates/yekitrim-server/src/api/polish');

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

// ---- 3b：LLM 新接口的接线（**同一类缝**：前端路径 <-> routes.rs 注册的路由）----
// 为什么并到这里而不是新开一个脚本：这张卡的判据本来就是「隔着边界的名字」，HTTP 路径 ←→ 路由
// 正是其中一处；而 `scripts/AGENTS.md` 的卡片预算只剩 76 B，新脚本需要新卡片行。
const LLM_API = join(repo, 'webui/src/api/llm.js');
const llmJs = existsSync(LLM_API) ? readFileSync(LLM_API, 'utf8') : '';
check(
  llmJs.length > 0,
  'webui/src/api/llm.js 应当存在（设置页的 LLM 面全走它）',
  '没找到这个文件 —— 是改路径了，还是接口层被绕过了？',
);
const llmPaths = new Set();
for (const m of llmJs.matchAll(/['"`](\/api\/[^'"`]*)/g)) {
  const p = m[1].split('?')[0].split('${')[0].replace(/\/$/, '');
  if (p) llmPaths.add(p);
}
check(llmPaths.size > 0, 'api/llm.js 应当至少调一个 /api 路径', '一条都没找到');
for (const p of llmPaths) {
  const suffix = p.slice('/api'.length);
  check(
    routesSrc.includes('.route("' + suffix + '"'),
    `api/llm.js 调了 ${p}，routes.rs 必须有 .route("${suffix}", …)`,
    '路由名对不上 —— 浏览器会落到 fallback（它返回 200 "未知接口"，最容易被当成"成功了"）',
  );
}

// ---- 热重载的四条判据（主检查与 --self-test 共用，避免两处各写一份而漂移）----
// ---- 4：前端调的 `engine.*` 方法，`Engine` 上必须真的有 ----
// 4f/4g 的探针（2026-10-05）：与主检查共用一份，别在两处各写一个正则而漂移
const PROBE_SUBS_SIGNAL = /props\.persistedSubtitles/
// 形状探针：健康版里 `toast(...)` 后**紧跟**那条注释（中间只有空白）；
// 变异版会在这中间塞一句 `previewKey.value++` ⇒ 探针不再命中（这才叫翻面）。
const PROBE_PREVIEW_REMOUNT = /toast\(jobLabel\(kind\) \+ '完成'\)\s*\/\/ ⚠️ \*\*任务完成后绝不重挂预览\*\*/
const PROBE_RELOAD_DEF = /async function reloadDhampirProject\s*\(\)\s*\{/
const PROBE_RELOAD_WATCH = /watch\(\(\)\s*=>\s*props\.persistedConfig,/
const PROBE_RELOAD_OPEN = /dhampirEngine\.open\(/
const PROBE_ENGINE_REOPEN = /open\s*\(project\)\s*\{/
// 字体：热重载这条腿必须**在 open() 之前**把工程声明的字体族装进页面。
// 这段原先写死在 startDhampir 里，于是换字体不生效（canvas 静默回退系统字体，
// 而字号/位置/颜色全对 —— 最难查的那一类）。
const PROBE_RELOAD_FONT = /await\s+ensureProjectFonts\(project\)/
const PROBE_FONT_DEF = /async function ensureProjectFonts\s*\(project\)\s*\{/

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
  // ---- 4b：**工程热重载这条缝必须在**（改效果/挪事件时间不用刷新页面）----
  //
  // 画面是引擎按**它自己那份工程**画的，而那份工程是从磁盘 `polish.toml` 转译来的。
  // 曾经 `open()` 只在 `startDhampir()` 里调用一次 ⇒ 改完效果**必须刷新页面**
  // （= 重挂载 → 重新读盘）才看得见，而 DOM 那条腿（`buildIndex` 重算）当场就变 ——
  // 两条腿表现不一致，且**不报任何错**。
  //
  // 判据盯三件事，因为它们各自都能让这条链静默断掉：
  //   ① 还**有人调** `open()`（改写时把它挪回"只调一次"，热重载就没了）；
  //   ② 存在**重载入口**（拉工程 → `open`）；
  //   ③ 有个 `watch` 挂在 `props.persistedConfig` 上（= **磁盘上那份**：落盘完成 / 任务跑完
  //      `load()` 之后才更新）。⚠️ **不能是 `props.config`** —— 那是页面手里的镜像，
  //      拖动时会被 60ms 节流地换成内存中间态（不落盘），挂它上面每次拖拽都白读一次磁盘。
  const vueSrc = readFileSync(PREVIEW_VUE, 'utf8');
  check(
    PROBE_RELOAD_DEF.test(vueSrc),
    'PolishPreview.vue 里应当有 Dhampir 工程的热重载入口（reloadDhampirProject）',
    '少了它，改效果 / 挪事件时间就必须刷新页面才看得见（且不报错）',
  );
  check(
    PROBE_RELOAD_WATCH.test(vueSrc),
    'PolishPreview.vue 应当有个 watch 挂在 props.config 上（父级 load 换掉它 = 改动已落盘）',
    '热重载没有触发点 = 有入口但永远不跑，表现与没有它完全一样',
  );
  // 判据必须只看**重载函数体内部**：`startDhampir` 里也有一处 `engine.open()`，
  // 全局 match 会被它满足 —— 那正是自证抓出来的空转。
  const reloadBody = fnBodyOf(vueSrc, 'reloadDhampirProject')
  check(
    PROBE_RELOAD_OPEN.test(reloadBody),
    '热重载函数体里必须真的调 engine.open() 把新工程推进引擎',
    '只重新取 JSON 而不 open()，引擎里那份工程一个字节都没变 —— 画面照旧不动（别被 startDhampir 里那处 open() 满足）',
  );
  // 引擎那份必须真的接受“再开一次”（不是只给首次载入用）。
  check(
    /open\s*\(project\)\s*\{/.test(engineSrc) && /bumpPlanGeneration\(\)/.test(engineSrc),
    'engine.js 的 open() 应当能被重复调用并作废帧标识缓存（bumpPlanGeneration）',
    '底座若把 open() 改成“只能开一次”，热重载会静默失效 —— 这条钉住那个契约',
  );

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
// ---- 定版产物（见 docs/yekitrim-integration.md §8）----
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
const DHAMPIR_RS = join(repo, 'crates/yekitrim-polish/src/render/dhampir.rs');
const POLISH_API = join(repo, 'crates/yekitrim-server/src/api/polish');
const dhampirSrc = readFileSync(DHAMPIR_RS, 'utf8');
// 2026-11-25 整改后这两个常量指向**目录**（api/polish/*.rs）：目录就拼接全部 .rs。
const readAny = (abs) => {
  if (!statSync(abs).isDirectory()) return readFileSync(abs, 'utf8');
  return readdirSync(abs)
    .filter((n) => n.endsWith('.rs'))
    .sort()
    .map((n) => readFileSync(join(abs, n), 'utf8'))
    .join('\n');
};
const polishApiSrc = readAny(POLISH_API);

// 出片腿：`render_video_only` 里那次 `prepare_project[_reporting](clip_dir, project_path, true)`
// （`_reporting` 那个变体额外把转译报告带回来给 `--strict` 用 —— 参数口径**必须一样**）
check(
  // ⚠️ 第 4 个参数（`video`：预览代理）2026-10-05 加进来之后，出片腿写的是 `..., true, None`
  // —— 判据必须跟着放宽一位，否则它报的是"找不到"，而语义（整条时间线）一直没变。
  /prepare_project(?:_reporting)?\(\s*clip_dir\s*,\s*project_path\s*,\s*true\s*(?:,\s*None\s*)?\)/.test(dhampirSrc),
  '出片腿必须传 `keep_muted = true`（整条时间线，屏蔽交给尾部那一剪）',
  'render/dhampir.rs 里找不到 `prepare_project[_reporting](clip_dir, project_path, true[, None])` —— ' +
    '传 false 就是"转译时先掐一遍、尾部再剪一遍"（成片短一大截 + 音轨错位）',
);
// 预览腿：`polish_dhampir_project` 里那次必须 `false`
check(
  // 同出片腿：第 4 个参数是预览代理（`preview_video.as_deref()`），`keep_muted` 仍是 false。
  // ⚠️ `_reporting` 变体（§4 转译报告可见）与出片腿同一个理由要认 —— 两条腿的参数口径必须一样；
  // 判据一边认一边不认的话，改了预览腿这条就报"找不到"，而 keep_muted 语义其实没动过。
  /prepare_project(?:_reporting)?\(\s*&clip\s*,\s*&scratch\s*,\s*false\s*(?:,\s*preview_video\.as_deref\(\),?\s*)?\)/.test(polishApiSrc),
  '预览腿必须传 `keep_muted = false`（压缩时间线，客户端要跳屏蔽区间）',
  'api/polish/（拆分后的目录）里找不到 `prepare_project[_reporting](&clip, &scratch, false[, preview_video.as_deref()])` —— ' +
    '传 true 的话客户端拿不到"断点"，会一路播过屏蔽区间',
);

/** 从源码里切出某个函数（`function NAME` / `async function NAME`）的函数体（花括号配平）。
 *
 * ⚠️ **不能用 `slice(indexOf(name))`**：那会一路切到文件末尾，把后面 `startDhampir` 里的
 * 同名调用一起圈进来 —— 自证就是被这一条骗过去的（重载函数体里的 `open()` 改掉了，
 * 但切片尾巴上还有一处，判据照样绿）。 */
/** `commitEdit` 的形状判据：`done` 必须被 `onceSaved(() => …)` 包住，不许裸 emit。
 *
 * 抽成函数是为了**主检查与自证共用同一份定义** —— 各写一份必然漂移，
 * 那种情况下自证证明的只是"自证自己写对了"（这个脚本在这条缝上已经栽过一次）。 */
function doneWrappedOk(body) {
  if (!/onceSaved\(/.test(body)) return false
  // 把 `onceSaved(...)` 那一段整体抠掉，剩下的正文里不许再出现 `emit('done'`
  const stripped = body.replace(/onceSaved\([\s\S]*?\)\s*$/m, '')
  return !/emit\('done'/.test(stripped)
}

function fnBodyOf(src, name) {
  let start = src.indexOf('async function ' + name)
  if (start < 0) start = src.indexOf('function ' + name)
  if (start < 0) return ''
  const open = src.indexOf('{', start)
  if (open < 0) return ''
  let depth = 0
  for (let i = open; i < src.length; i += 1) {
    if (src[i] === '{') depth += 1
    else if (src[i] === '}') {
      depth -= 1
      if (depth === 0) return src.slice(open + 1, i)
    }
  }
  return ''
}

  // ---- 4c：**「落盘完成」要在 `done` 之前**（2026-10-02 修）----
  //
  // `commitEdit` 必须把 `done` 挂到 `onceSaved` 上，**不能当场 emit**：落盘是 `touch()`
  // 排的 `SAVE_DEBOUNCE_MS`（现 250ms）定时器，当场发就等于告诉页面"磁盘上已经有这次改动了"，而其实还没有。
  // 以前侥幸没出事纯粹是时序巧合（`process` 前压着 1.5s > 250ms）——
  // 一旦有消费者不等那 1.5s（Dhampir 热重载就是），它立刻读到旧内容。
{
  const editorSrc = readFileSync(TIMELINE_EDITOR, 'utf8')
  const viewSrc = readFileSync(POLISH_VIEW, 'utf8')
  const commitBody = fnBodyOf(editorSrc, 'commitEdit')
  // 字体：热重载与首启**两条路都要装字体**（单独读这份，别借上面那个 block 的 `vueSrc` —— 它在那里面）
  const previewSrc = readFileSync(PREVIEW_VUE, 'utf8')
  const reloadBody = fnBodyOf(previewSrc, 'reloadDhampirProject')
  check(
    PROBE_FONT_DEF.test(previewSrc),
    'PolishPreview.vue 应当有个 ensureProjectFonts(project)（把工程声明的字体族装进页面）',
    'canvas 栅格化只用页面里已有的字体：没装就静默回退，症状是"字体不对"而字号/位置/颜色全对',
  )
  check(
    PROBE_RELOAD_FONT.test(reloadBody),
    'reloadDhampirProject 里必须 await ensureProjectFonts(project)（热重载也要装字体）',
    '只在 startDhampir 里装 = 换字体不热重载（这段原先就写死在那里面，实际踩过）',
  )
  check(
    PROBE_RELOAD_FONT.test(fnBodyOf(previewSrc, 'startDhampir')),
    'startDhampir 里也必须装字体（首启路径）',
    '热重载装了、首启没装 = 进页面时字体就是错的，之后怎么改都不对',
  )
  check(
    /onceSaved\(/.test(commitBody),
    'TimelineEditor 的 commitEdit 必须把 done 挂在 onceSaved 上（落盘成功之后才通知页面）',
    '当场 emit done = 抢在 PUT 前面报"已完成"，任何"收到 done 就去读磁盘"的消费者都会读到旧内容',
  );
  // `done` 只许出现在 `onceSaved(() => …)` 的**回调里**：把回调整体抠掉之后，
  // 函数体里不该再剩任何 `emit('done')`（剩一个就是一条抢跑路径）。
  check(
    doneWrappedOk(commitBody),
    "commitEdit 里不许有任何**未被 onceSaved 包住**的 emit('done')",
    '漏一处就是一条抢跑路径：页面收到 done 就会去读磁盘，而那次 PUT 还没发',
  );
  // 页面必须把「落盘完成」转成 persistedConfig —— 否则信号发出来了没人接。
  check(
    /@saved="onEditSaved"/.test(viewSrc) && /persistedCfg\.value = cfg\.value/.test(viewSrc),
    'PolishView 必须接住 TimelineEditor 的 @saved 并更新 persistedCfg',
    '信号发了没人接 = Dhampir 那条腿永远等不到"落盘完成"，又退回"只有后处理跑完才重载"',
  );
  check(
    /persistedCfg\.value = cfg\.value/.test(viewSrc),
    'load() 从盘上读回配置后也要同步 persistedCfg（任务完成含 process 的校准结果）',
    '少了这一条，后处理改写的事件就永远进不了引擎',
  );

  // ---- 4e：热重载的**派生状态重建**（2026-10-02 补）----
  //
  // `open()` 换的只是"工程"，而有一批**从工程推导出来的宿主侧状态**原来只在
  // `startDhampir` 里建 —— 热重载不重建它们，用户改了对应的东西就是"没反应"：
  //   段落表（时间线压缩后帧号映射错位）/ 画布尺寸 / 贴纸 spans / 字幕旁写。
  // 三份共享实现（dhampirCanvasSizeFor / dhampirStickersFrom / dhampirApplySubtitles）
  // 必须存在，且**重载路径必须真的调它们** —— 只在首启调 = 半次修复。
  const startBody = fnBodyOf(previewSrc, 'startDhampir')
  for (const [fn, what, sym] of [
    ['dhampirCanvasSizeFor', '画布目标尺寸（render_hints → 只缩不放）', 'canvas'],
    ['dhampirStickersFrom', '贴纸层描述（活跃区间/相位锚点；解码缓存按 uri+fps 继承）', 'stickers'],
    ['dhampirApplySubtitles', '字幕/弹幕旁写（loadSubtitles 是替换语义，必须重喂）', 'subs'],
    ['dhampirLoadAnimations', '动图直通（整段字节交给引擎，引擎自己解码取帧）', 'animations'],
  ]) {
    const re = new RegExp('function\\s+' + fn + '\\b')
    const call = fn + '('
    check(
      re.test(previewSrc),
      'PolishPreview.vue 应当定义 ' + fn + '()（' + what + '，startDhampir 与热重载共用）',
      '两处各写一份必然漂移 —— 抽成一份是这条判据的前提',
    )
    check(
      reloadBody.includes(call),
      'reloadDhampirProject 里必须调 ' + call + ')（热重载要重建' + sym + '派生状态）',
      '热重载只 open() 不重建派生状态 = 用户改了对应的东西就是"没反应"（贴纸/字幕各踩过一次）',
    )
    check(
      startBody.includes(call),
      'startDhampir 里也必须调 ' + call + ')（首启路径）',
      '热重载调了、首启没调 = 进页面时状态就是缺的',
    )
  }
  // ---- 4f：**只改字幕也要触发重载**（2026-10-05 修「改完字幕预览不刷新」）----
  //
  // 字幕**不在 `polish.toml` 里**（它住 `data/subtitles.json`）⇒ 重载指纹里的配置子树
  // 一个字节都不变，那次改动于是永远不触发重载。信号必须由父级在**落盘后**递进来
  // （与 `persistedConfig` 同款：拖动中的镜像不算数，磁盘还没写下去）。
  check(
    PROBE_SUBS_SIGNAL.test(previewSrc),
    '热重载指纹必须把「磁盘上那份字幕」算进去（props.persistedSubtitles）',
    '缺了它：改完字幕预览不刷新 —— 字幕不住 polish.toml，配置子树不会变',
  )
  check(
    /:persisted-subtitles="persistedSubs"/.test(viewSrc),
    'PolishView 必须把 persistedSubs 递给预览',
    '接了一半 = 信号永远是 null，判据看着在、实际不生效',
  )
  check(
    /persistedSubs\.value = subs\.value/.test(fnBodyOf(viewSrc, 'onEditSaved')),
    '「落盘完成」时才更新 persistedSubs（不能拿未落盘的镜像当磁盘版本）',
    '镜像不算数：那时磁盘上还是旧字幕，重载只会把旧内容再喂一遍',
  )

  // ---- 4g：**任务完成后绝不重挂预览**（2026-10-05 修「LLM 剪辑后自动刷新黑屏」）----
  //
  // 底座 `dhampir_project_attach()` 是**一页只接一次**的语义
  //（`dhampir-wasm/src/timeline_host.rs`：`PROJECT_HOST` 已存在就直接返回 `{"already":true}`，
  // 且**没有 detach 导出**），而 wasm 实例是页面级单例（`mod.default()` 是幂等 init）。
  // ⇒ 只要 `#dhampir-canvas` **元素**被换掉（`:key` 重挂），新 canvas 永远接不上，
  // 引擎继续把帧画在**已卸载的旧 canvas** 上 ⇒ 整块黑（fps/帧号照涨、控制台无报错），
  // 刷整页才有新的 wasm 实例 ⇒ 这就是用户看到的"自动刷新后黑屏、手动刷新才正常"。
  // 所以任务完成那条路**不许** bump `previewKey`：刷新交给就地热重载
  //（`after` 里的 `load({silent:true})` → `persistedCfg` → `reloadDhampirProject`）。
  check(
    !PROBE_PREVIEW_REMOUNT.test(fnBodyOf(viewSrc, 'startJob')),
    'startJob 里不许 bump previewKey（任务完成重挂预览 = 画布被换 ⇒ 底座单例宿主接不上 ⇒ 黑屏）',
    '底座没有 detach、wasm 实例是页面级单例；重挂预览必黑，只有整页刷新才恢复',
  )

  // ---- 4e-2：**动图直通真的接上了没有**（2026-10-02，host_api v6）----
  //
  // 版本号改成 6 只是"拦住两端错配"的闸，**不证明新能力被用上** ——
  // 改了数字却没接那两条导出 = 贴纸静默不动（与"字段少了"同一类症状）。
  // 所以这里断言：宿主确实调了 `dhampir_asset_load_animation`，且**不再**对动图
  // 逐帧调 `/polish/sticker-frame`。
  check(
    /dhampir_asset_load_animation/.test(previewSrc),
    'PolishPreview.vue 应当调底座的 dhampir_asset_load_animation（host_api v6）',
    '只把 DHAMPIR_HOST_API 改成 6 而不接这条导出 = 动图贴纸静默不动',
  )
  // 动图那条路**不许**再走逐帧位图：`dhampirLoadAnimations` 成功的资产在帧循环里
  // 必须是 `continue`（由引擎供帧），否则宿主与引擎缓存两边抢 ——
  // 底座把动图排在 `texture_for` 最前，宿主再交位图就会把贴纸**钉死在**它给的那一帧。
  check(
    /if\s*\(st\.animLoaded\)\s*continue/.test(previewSrc),
    '帧循环里动图必须 continue（引擎供帧，宿主不再 setSourceBitmap）',
    '不跳过 = 与引擎的动图缓存抢，症状是"动图不动了"（钉在宿主给的那一帧）',
  )

  // ⚠️ **loadAnimations 必须排在 attach 之后**（2026-10-02 实测）：
  // 底座把纹理上传挂在 attach 上 —— host 没接上时 `dhampir_asset_load_animation`
  // 返回 `parsed:true, ok:false, error:"还没 attach 上 canvas：动图现在只能先记账"`，
  // 也就是说**字节白传了、贴纸一张都不出**。这条顺序错了两端都不会报错，
  // 只会表现成"动图不动"，所以在这里钉住。
  {
    const iOpen = startBody.indexOf('dhampirEngine.open(')
    const iAttach = startBody.indexOf('dhampirEngine.attach(')
    const iAnim = startBody.indexOf('dhampirLoadAnimations(')
    check(
      iOpen >= 0 && iAttach >= 0 && iAnim >= 0,
      'startDhampir 里应当有 open / attach / dhampirLoadAnimations 三处调用',
      '找到的位置：open=' + iOpen + ' attach=' + iAttach + ' anim=' + iAnim,
    )
    check(
      iAnim > iAttach && iAttach > iOpen,
      'startDhampir 里的顺序必须是 open → attach → dhampirLoadAnimations',
      '实测顺序 = open(' + iOpen + ') attach(' + iAttach + ') anim(' + iAnim + ')；' +
        'load 排在 attach 之前时底座只记账、不上传纹理（ok:false，"还没 attach 上 canvas"）⇒ 贴纸全不出且不报错',
    )
  }

  // ⚠️⚠️ **播放期零请求**（这次改造最大的收益，也是最容易被顺手改回去的一条）：
  // 帧循环里那个 `/polish/sticker-frame` 调用**必须在** `if (st.animLoaded) continue` **之后**。
  // 不然动图会重新走回逐帧 HTTP 取 PNG（~390KB/帧），而且与引擎自己的解码两边冲突。
  {
    const iSkip = previewSrc.indexOf('if (st.animLoaded) continue')
    const iFetch = previewSrc.indexOf("'/api/polish/sticker-frame'")
    check(
      iSkip >= 0,
      '帧循环里应当有 if (st.animLoaded) continue 跳过动图',
      '没有这一行 = 动图又回到了宿主位图那条路（引擎供帧与宿主交图两边抢）',
    )
    check(
      iSkip >= 0 && iFetch >= 0 && iFetch > iSkip,
      '动图跳过必须发生在取帧之前（这样播放期才是零请求）',
      '下标 = skip(' + iSkip + ') fetch(' + iFetch + ')；反过来就是每帧发一次 HTTP',
    )
  }

  // ⚠️ **延迟表冲突必须报**（T3）：解码器读的那份是事实、工程 JSON 那份退为诊断，
  // 但两者不一致时**不能静默** —— 症状是"动图越播越偏"，属于最难归因的那类。
  {
    check(
      /frame_delays_ms/.test(previewSrc) && /diff >= 0/.test(previewSrc),
      '动图加载后应当把解码器的帧延迟表与工程 JSON 对一下并报冲突',
      '只看 frame_delays_ms 不看差异 = 存下来但从不核对，漂移会静默发生',
    )
    check(
      /declared\.length !== st\.delays\.length/.test(previewSrc),
      '帧延迟表长度不一致也要报（比逐值不同更严重的漂移）',
      '工程表长度对不上时，那份表会被当权威用在别处',
    )
    check(
      /延迟表冲突/.test(previewSrc),
      '冲突读数要露在工具条上（不能只 console.warn）',
      '只写控制台 = 用户看不到，等于没报',
    )
  }

  // 字幕重喂还依赖 response 的旁写内容字段名 —— 两端各一份，钉一起。
  check(
    /res\.subtitles/.test(previewSrc),
    'PolishPreview.vue 消费 /dhampir-project 响应的 res.subtitles 字段',
    '服务端返回的键是 subtitles（旁写文件名 → 内容）；改错键名 = 字幕静默消失',
  )

  // ---- 4e-bis：**服务端必须按 `doc.assets` 找旁写字幕，不许按写死的名字**（2026-10-13）----
  //
  // 这条是本仓最贵的一次"隔着边界的名字对不上"：
  //
  //   服务端原先只读两个**写死的**名字（`dhampir-subtitle.srt` / `dhampir-danmaku.ass`），
  //   在 `--out` 旁边找。而**文字元素轨的 ASS 写在另一个目录**
  //   （当时是片段根目录；2026-10-18 起改成 `--out` 同目录的 `<工程文件名>.assets/`）
  //   （`dhampir-text_<样式>_<档>[_<动效>][_<颜色>].ass`）——
  //   一条都没读过 ⇒ `dhampir_project_set_subtitles` 从没被调用
  //   ⇒ **wasm 预览里文字元素一个字都不显示**。
  //
  //   ⚠️ 而成片那条腿（`dhampir.exe`）**完全正常** —— 它按 `asset.uri`（绝对路径）
  //      自己找文件。所以症状是"成片里好好的、预览里没有，而报告/日志/`doc.json` 全绿"，
  //      上面那条"消费了 res.subtitles 字段"**照样通过**（键确实在，只是少了一类）。
  //
  // ⇒ 判据压在**形状**上：读旁写字幕要**遍历 doc 的 assets**，不是"在固定名字上循环"。
  const polishRs = readAny(POLISH_RS);
  check(
    /fn read_side_subtitles/.test(polishRs),
    '服务端有一个专门的「读旁写字幕」函数（read_side_subtitles）',
    '散在 handler 里的内联循环没法被这条判据压住 —— 抽成函数是为了可检',
  );
  // ⚠️ **必须把这条判据限定在 `read_side_subtitles` 的函数体里**：
  //    `Some("subtitle")` 在文件里出现 4 次（**单测里也有**），
  //    裸匹配的话「把过滤条件改成别的东西」这条变异**打不红** ——
  //    实测就是这么发现的（第一版判据绿着，而生产代码已经被改坏）。
  //    这与 `check-element-color.mjs` 那条"正则误命中无关处"是同一类坑。
  const sideSubsBody = (() => {
    const start = polishRs.indexOf('fn read_side_subtitles(');
    if (start < 0) return '';
    // 函数体到下一个顶层 `}` 为止（本仓这条函数后面紧跟另一个 `fn`）。
    const rest = polishRs.slice(start);
    const end = rest.indexOf('\n}\n');
    return end < 0 ? rest : rest.slice(0, end);
  })();
  check(
    /get\("assets"\)/.test(sideSubsBody) && /== Some\("subtitle"\)/.test(sideSubsBody),
    '读旁写字幕要**遍历 doc 的 assets 并只认 kind == "subtitle"**（而不是一个写死的名字列表）',
    '按写死的名字读 = 文字元素轨（名字带档位与颜色）永远读不到：' +
      '预览里那一类文字不显示，而成片正常、报告全绿',
  );
  check(
    /const LEGACY_NAMES: \[&str; 2\] = \["dhampir-subtitle\.srt", "dhampir-danmaku\.ass"\]/.test(polishRs),
    '那两个老名字只能作为**兜底常量**存在（`LEGACY_NAMES`），不许再散在读取逻辑里',
    '散着写 = 下一个人加一类文字时会照抄这个形状，而它正是本次缺陷的形状',
  );

  // ---- 4f：**提取残留不许有**（2026-10-02 实际踩过）----
  //
  // 把公共块抽成 helper 时，若函数体里留下一处**消费被抽走局部量**的残块，
  // 生产构建照样过（vite 不做未定义标识符检查），运行时才炸 —— 表现正是
  // "Dhampir 预览不可用：xxx is not defined（当前是近似预览）"。
  // 判据：这两条路径的函数体里不许再出现被抽走的那几个局部量。
  for (const [body, name] of [[startBody, 'startDhampir'], [reloadBody, 'reloadDhampirProject']]) {
    // 按标识符的**任何出现**查，不只查声明：当年那次事故是"声明被抽进 helper、
    // 使用块残留" —— 残块里只有 `entries.length`，查声明永远抓不住它。
    for (const leaked of ['entries', 'stickerImages', 'subTexts']) {
      const re = new RegExp('\\b' + leaked + '\\b')
      check(
        !re.test(body),
        name + ' 里不许再出现被抽进 helper 的局部量（' + leaked + '）',
        '提取残留（哪怕只是一处使用）= 运行时 ReferenceError，整个 Dhampir 预览退回近似预览（vite 构建抓不住它）',
      )
    }
  }
}

// ---- 自证：把几处关键字符串真改一遍，上面的判据必须打红 ----
//
// **一条不会红的检查等于没检查**。这条规矩在这个脚本上有过教训：新加的四条判据第一版
// 用 `/reloadDhampirProject\s*\(/` 去匹配 —— 而 `reloadDhampirProject` 在注释里、在调用点
// 一共出现 5 次，把函数定义整个删掉、判据**照样绿**。所以判据必须绑在**定义**（`async function
// reloadDhampirProject() {`）与**注册**（`watch(() => props.config,`）上，并由这里钉住。
const SELF_TEST = process.argv.includes('--self-test');
// 4f/4g 的探针（与主检查共用一份，避免两处各写一份而漂移）
if (SELF_TEST) {
  const vueText = readFileSync(PREVIEW_VUE, 'utf8');
  const cases = [
    ['[变异] 删掉热重载入口（定义改名）', 'async function reloadDhampirProject() {', 'async function _gone() {', PROBE_RELOAD_DEF],
    ['[变异] 拆掉 persistedConfig 触发点', 'watch(() => props.persistedConfig, (cfg) => {', 'watch(() => props.persistedConfig?.duration, (cfg) => {', PROBE_RELOAD_WATCH],
    ['[变异] 重载不再调 engine.open()（只改重载函数体内那处）', 'dhampirEngine.open(project)', 'dhampirEngine.doc()', PROBE_RELOAD_OPEN],
    // 4c：把 done 改回"当场发"（抢在落盘前面）—— 判据必须点名"未被 onceSaved 包住"
    ['[变异] done 退回当场 emit（抢在落盘之前）', 'onceSaved(() => emit(\'done\', payload))', "emit('done', payload)"],
    // 字体：热重载不再装字体（退回"只在 startDhampir 里装"那版）—— 判据必须点名
    ['[变异] 热重载不装字体', '    await ensureProjectFonts(project)' + "\n" + '    const tb = project.timeline.timebase', '    const tb = project.timeline.timebase', PROBE_RELOAD_FONT],
    // 4f：指纹里去掉字幕信号（退回"改字幕不刷新"那版）
    ['[变异] 指纹里去掉字幕信号', 'props.persistedSubtitles,', '', PROBE_SUBS_SIGNAL],
    // 4g：任务完成又去重挂预览（退回"LLM 剪辑后黑屏、F5 才好"那版）
    ['[变异] 任务完成重挂预览', "toast(jobLabel(kind) + '完成')", "toast(jobLabel(kind) + '完成'); previewKey.value++", PROBE_PREVIEW_REMOUNT],
  ]
  const editorText = readFileSync(TIMELINE_EDITOR, 'utf8')
  let bad = 0
  for (const [label, from, to, probe] of cases) {
    // ⚠️ 变异必须落在**判据真正看的那段文本**里：`dhampirEngine.open(project)` 在
    // `startDhampir` 里也有一处，直接对全文 replace 会改到那一处 —— 判据看的是重载
    // 函数体，于是「变异没被抓住」。这里按变异类型挑它该落的那份源文本。
    const target = label.includes('重挂预览')
      ? readFileSync(POLISH_VIEW, 'utf8') // 4g 那条要改的是父组件
      : label.includes('engine.open') || label.includes('热重载不装字体')
        ? fnBodyOf(vueText, 'reloadDhampirProject')
        : (label.includes('done') ? fnBodyOf(editorText, 'commitEdit') : vueText)
    if (target.split(from).length - 1 < 1) {
      console.error('  ✗ ' + label + '：源文本里找不到要改的片段（' + from + '）')
      bad += 1
      continue
    }
    const mutated = target.replace(from, to)
    if (mutated === target) {
      console.error('  ✗ ' + label + '：替换后文本没变（变异台自己错了）')
      bad += 1
      continue
    }
    // 有 probe 的走正则翻面；没有 probe 的是"源码形状"判据，按它自己的谓词翻面。
    const flipped = probe
      ? (!probe.test(mutated) && probe.test(target))
      : doneWrappedOk(mutated) === false && doneWrappedOk(target) === true
    if (flipped) console.log('  ✓ ' + label + ' → 判据翻面')
    else { console.error('  ✗ ' + label + ' → 判据**没有**翻面（这条判据是空转的）'); bad += 1 }
  }
  if (bad > 0) {
    console.error('');
    console.error('✗ 自证失败：有判据抓不住它该抓的回归');
    process.exit(1);
  }
  console.log('✓ 自证：' + cases.length + ' 处变异全部被判据抓住');
}
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
