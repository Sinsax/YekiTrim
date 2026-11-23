#!/usr/bin/env node
// 前端 request 层（`webui/src/api/*.js`）调用的 /api/... ←→ 服务端 `routes.rs` 注册的 (method, path)
// —— **双向契约差集**。
//
// # 为什么需要它
//
// 前端调了一条服务端没注册的路由，**前端不报错、编译不报错、服务端 146 条用例全绿**：
// 浏览器命中 SPA 回退拿到 200 HTML，`client.js` 的「只认 JSON」判据把它变成一句
// 「接口不存在或返回格式错误」，或者干脆把 null 当成功（`apiOr` 那类兜值）。
// 症状是"点了没反应"，从现象看不出是路由问题 —— 与 `check-dhampir-wiring.mjs` 头部写的是同一类缝。
//
// `scripts/route_manifest.py` 已经把路由表 dump 成稳定清单，但它 **exit 恒 0**
// （"比较是调用方的事"）。这个脚本就是那个调用方：拿清单、扫前端、做差集。
// ⚠️ 本脚本**不碰 route_manifest.py 的语义**（它仍是纯 dump + exit 0）。
//
// # 双向
//
//   前向（**红**）：前端调了、后端没注册 ⇒ exit 1，逐条给出 文件:行 / 字面量 / 最接近的已注册路由
//   后向（**只提示**）：后端注册了、前端从不调 ⇒ 不红。CLI / 运维 / SSE / 视图直连都会用到；
//                       红它就是逼下一个人往白名单里塞东西（那才是把判据废掉的第一步）
//
// # 它**抓不到**什么（写在这里，免得下次有人以为它证明了更多）
//
//   · **字段名**：46 个 handler 收 `Json<serde_json::Value>`，请求体字段写错这条看不见（那是 T9 的事）；
//   · **method 只做提示**：前端把 method 写在 opts 对象里（`{ method: 'PUT' }`），还有一批
//     "只造 URL、不自己发请求"的 helper（`paths.status(pid)`）—— method 推定不可靠，
//     所以 method 不符只打提示、**不参与红**（axum 会回 405，那是另一档更明显的错误）；
//   · **handler 体 / 中间件 / .route_layer 语义** —— 归 `cargo test -p yekitrim-server`。
//
// # 归一化（本任务唯一有技术含量的部分，别退化成前缀匹配）
//
// 前端那几十条模板串 / 拼接串必须先变成**段可比**的形态：
//
//   1. **先剥查询串**：`/api/polish/project?${qs(…)}` → `/api/polish/project`
//      （? / # 之后的一切都不属于路径；模板串插值常在 ? 后面，一起被剥掉）
//   2. **插值段 → :x**：`/api/admin/users/${encodeURIComponent(u)}/quota`
//      → `/api/admin/users/:x/quota`，与路由里的 :name 段同形
//   3. **:x 只吞一段**（不跨 /）：`/api/jobs/:x` 与 `/api/jobs/:id/cancel` **不相交**
//   4. **粘连插值 = 查询串 / 后缀，不是段**：
//      `/api/ingest/records${q}${sep}limit=${limit}`（q 是 '' 或 '?all=1'，前面没有 /）
//      → `/api/ingest/records`。判据是"插值前面那个字符是不是 /" —— 段内插值在 JS 里一律写作 /${…}。
//   5. **`+` 拼接按同一套规则续喂**：`'/api/jobs/' + encodeURIComponent(id) + '/cancel'`
//      → `/api/jobs/:x/cancel`。（只从以 /api 开头的那个字面量起头，链尾的 `'/cancel'` 不算新路径。）
//
// 比对用**对称相交**判据（`pathsIntersect`）：static 必须逐段相等，:x / :param 各自只吞一段，
// *path 吞剩余（至少一段）。前向与后向用**同一个函数** —— 两个方向问的都是"这两条路径有没有
// 公共的具体路径"，各写一份必然漂移。
//
// # 白名单
//
// 每条都在下面的 WHITELIST 表里写理由。白名单的语义是**只降级、不扩大**：它只在"这条前端路径
// 没命中任何已注册路由"时把红降成提示，**不许**吞掉真正的差集 —— 判据是段级相交、不是前缀，
// 所以 `/api/healthz` 不会被 `/api/health` 吞掉（自证 ⑦ 钉住了这条）。
//
// 用法：
//   node scripts/check-api-contract.mjs              # 主检查（红 = 有前端调用没有对应路由）
//   node scripts/check-api-contract.mjs --self-test  # 内置样本自证「会红也会绿」
//   node scripts/check-api-contract.mjs --mutate     # 变异自证：差集判据翻面后自证必须失败
//   node scripts/check-api-contract.mjs -v           # 打印完整后向清单（默认只打 12 条 + 计数）
//
// 零依赖（只用 node 内置），几毫秒跑完；python3 只用来 dump 路由清单。

import { readFileSync, readdirSync, statSync, writeFileSync, unlinkSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join, relative, resolve, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const BT = String.fromCharCode(96); // 反引号（本文件里用常量代替，方便与自证样本共存）
const D = '$'; // 自证样本里要拼出插值标记，直接用常量拼，别在源码里写成一团


// ---------------------------------------------------------------------------
// 白名单：**只降级、不扩大**，每条都写清"为什么它不该红"。
// ---------------------------------------------------------------------------
// ⚠️ 加条目之前先问一句「它真的调不到吗」：能靠补一条 api/ 里的封装解决的，就别加白名单。
// 白名单越长，这个脚本能抓到的真差集越少 —— 它自己就是那条"门禁不可绕过"的滑坡。
const WHITELIST = [
  {
    path: '/api/stream',
    why:
      'SSE 主通道（global_stream）：前端在 composables/useJobs.js 用 new EventSource 直连，' +
      '不经过 api()（api() 只认 JSON，而它回 text/event-stream）。EventSource 不走 request 层是设计，不是漏接线。',
  },
  {
    path: '/api/jobs/:id/events',
    why:
      'SSE 单任务事件流（job_events）：同上，api/jobs.js::watchJob 里 new EventSource 直连。' +
      '它**确实**在 api/ 层被拼出来（归一化后也能命中路由），列进来是为了「SSE 三兄弟同口径」：' +
      '有人把它删掉时按"通道类型"处理，而不是红。',
  },
  {
    path: '/api/admin/logs/events',
    why:
      'SSE 日志流（admin_logs_stream）：stores/log.js 里 EventSource 直连（带 ?after=<seq> 续传）。' +
      '它不住 api/ 层，后向清单里必须认得它，否则每次都被报成"死路由"的假阳性。',
  },
  {
    path: '/api/media/*',
    why:
      '取媒体文件（media_get）：URL 由**服务端**拼好塞进响应（预览 / 缩略图 / 代理视频），前端只把' +
      '字符串贴到 <video src>；它发 Range 请求、回字节流，永远不该走 api()。',
  },
  {
    path: '/api/health',
    why:
      '健康探针：stores/health.js 的唯一缓存点确实在调它（api() 那条唯一通路），但同一个端点也是' +
      '启动探活 / 托盘 / yekitrim doctor 的公共入口 —— 它的存在理由不依赖前端那一条调用线。',
  },
  {
    path: '/api/polish/dhampir-wasm/*',
    why:
      '静态资源（polish_dhampir_wasm）：底座的 engine.js / pkg/dhampir_wasm.js / .wasm，前端用动态 ' +
      'import(PKG_URL) 与 <script> 直连 —— 静态资源不是 JSON 接口，走不了 api()。',
  },
];

// ---------------------------------------------------------------------------
// 一、把 JS 源码读成「字面量清单」：**先剥注释**。
// ---------------------------------------------------------------------------
// 注释里写着 /api/xxx 的地方太多了（本仓注释风格如此），不剥就会把"文档里的路径"当成"真调用"。
// 剥注释用**等长空格**替换 ⇒ 行号完全不变（报错要给出 文件:行）。
function stripComments(src) {
  let out = '';
  let i = 0;
  const literals = [];
  const n = src.length;
  const blank = (s) => s.replace(/[^\n]/g, ' ');
  while (i < n) {
    const ch = src[i];
    const next = src[i + 1];
    if (ch === '/' && next === '/') {
      const end = src.indexOf('\n', i);
      const stop = end < 0 ? n : end;
      out += blank(src.slice(i, stop));
      i = stop;
      continue;
    }
    if (ch === '/' && next === '*') {
      const end = src.indexOf('*/', i + 2);
      const stop = end < 0 ? n : end + 2;
      out += blank(src.slice(i, stop));
      i = stop;
      continue;
    }
    if (ch === "'" || ch === '"' || ch === BT) {
      const lit = readLiteral(src, i);
      out += src.slice(i, lit.end);
      literals.push({ parts: lit.parts, end: lit.end, start: i });
      i = lit.end;
      continue;
    }
    out += ch;
    i += 1;
  }
  return { code: out, literals };
}

/** 读一个字符串 / 模板串 → **片段序列**：{t:'lit',v} 与 {t:'expr',v}（插值源码）。 */
function readLiteral(src, i) {
  const quote = src[i];
  if (quote === BT) return readTemplate(src, i);
  let cur = '';
  let j = i + 1;
  while (j < src.length) {
    const ch = src[j];
    if (ch === '\\') {
      cur += src[j + 1] ?? '';
      j += 2;
      continue;
    }
    if (ch === quote) return { parts: [{ t: 'lit', v: cur }], end: j + 1 };
    if (ch === '\n') return { parts: [{ t: 'lit', v: cur }], end: j }; // 未闭合：别把整份文件吞掉
    cur += ch;
    j += 1;
  }
  return { parts: [{ t: 'lit', v: cur }], end: j };
}

function readTemplate(src, i) {
  const parts = [];
  let cur = '';
  let j = i + 1;
  while (j < src.length) {
    const ch = src[j];
    if (ch === '\\') {
      cur += src[j + 1] ?? '';
      j += 2;
      continue;
    }
    if (ch === BT) {
      parts.push({ t: 'lit', v: cur });
      return { parts, end: j + 1 };
    }
    if (ch === '$' && src[j + 1] === '{') {
      parts.push({ t: 'lit', v: cur });
      cur = '';
      const closed = readBraceExpr(src, j + 2);
      parts.push({ t: 'expr', v: closed.expr });
      j = closed.end + 1;
      continue;
    }
    cur += ch;
    j += 1;
  }
  parts.push({ t: 'lit', v: cur });
  return { parts, end: j };
}

/** 花括号配平（里面可能还有字符串 / 模板串 / 嵌套对象）。 */
function readBraceExpr(src, i) {
  let depth = 1;
  let j = i;
  while (j < src.length) {
    const ch = src[j];
    if (ch === "'" || ch === '"' || ch === BT) {
      j = readLiteral(src, j).end;
      continue;
    }
    if (ch === '{') depth += 1;
    else if (ch === '}') {
      depth -= 1;
      if (depth === 0) return { expr: src.slice(i, j), end: j };
    }
    j += 1;
  }
  return { expr: src.slice(i), end: src.length };
}

// ---------------------------------------------------------------------------
// 二、把一段「路径表达式」归一化（规则见文件头 1–5）
// ---------------------------------------------------------------------------
const SEG = ':x'; // 插值段：与路由里的 :param 同形（只吞一段）

const isQuote = (ch) => ch === "'" || ch === '"' || ch === BT;

function skipSpace(code, i) {
  while (i < code.length && /\s/.test(code[i])) i += 1;
  return i;
}

/** 查询串起点（? 或 #）；没有则 -1。 */
function queryCut(v) {
  const q = v.indexOf('?');
  const h = v.indexOf('#');
  if (q < 0) return h;
  if (h < 0) return q;
  return Math.min(q, h);
}

function feedParts(state, parts) {
  for (const p of parts) {
    if (state.stopped) return;
    if (p.t === 'lit') {
      const cut = queryCut(p.v);
      if (cut >= 0) {
        state.out += p.v.slice(0, cut);
        state.stopped = true;
        return;
      }
      state.out += p.v;
      continue;
    }
    // 插值：前面是 / ⇒ 段内插值；否则**粘连**（查询串 / 计数后缀）⇒ 停
    if (state.out.endsWith('/')) state.out += SEG;
    else state.stopped = true;
  }
}

/**
 * 表达式操作数到哪儿结束：顶层（括号配平后）的 , ) ; } 换行 就是边界。
 * ⚠️ 顶层 `+` 也是边界：少了它，`encodeURIComponent(id) + '/cancel'` 会被整体吞掉 ⇒
 * 归一化成 /api/jobs/:x（把后面那段路径吃掉）—— 实测踩过，症状是"少一条路径差集"。
 */
function operandEnd(code, i) {
  let depth = 0;
  let j = i;
  while (j < code.length) {
    const ch = code[j];
    if (isQuote(ch)) {
      j = readLiteral(code, j).end;
      continue;
    }
    if (ch === '(' || ch === '[' || ch === '{') depth += 1;
    else if (ch === ')' || ch === ']' || ch === '}') {
      if (depth === 0) return j;
      depth -= 1;
    } else if (ch === '\n' && depth === 0) return j;
    else if (depth === 0 && (ch === ',' || ch === ';' || ch === '+')) return j;
    j += 1;
  }
  return j;
}

/** 从以 /api 开头的那个字面量起，把整条 `+` 拼接链归一化成一个路径模式。 */
function normalizePathExpr(code, lit) {
  const state = { out: '', stopped: false };
  feedParts(state, lit.parts);
  let pos = lit.end;
  while (!state.stopped) {
    const plus = skipSpace(code, pos);
    if (code[plus] !== '+') break;
    const opStart = skipSpace(code, plus + 1);
    if (isQuote(code[opStart])) {
      const next = readLiteral(code, opStart);
      pos = next.end;
      feedParts(state, next.parts);
      continue;
    }
    const end = operandEnd(code, opStart);
    if (state.out.endsWith('/')) {
      state.out += SEG; // 段内拼接：'/api/jobs/' + id
      pos = end;
    } else {
      state.stopped = true; // 粘连拼接：'/api/jobs' + (all ? '?all=1' : '')
    }
  }
  let path = state.out;
  while (path.endsWith('/') && path.length > 1) path = path.slice(0, -1);
  return path;
}

// ---------------------------------------------------------------------------
// 三、路径 → 段；段级**相交**判据
// ---------------------------------------------------------------------------
function classifySegment(seg) {
  if (seg.startsWith('*')) return { kind: 'wild', raw: seg };
  if (seg.startsWith(':')) return { kind: 'param', raw: seg };
  return { kind: 'static', value: seg, raw: seg };
}

function segmentsOf(path) {
  const trimmed = path.length > 1 && path.endsWith('/') ? path.slice(0, -1) : path;
  return trimmed
    .split('/')
    .filter((s, idx) => !(idx === 0 && s === ''))
    .map(classifySegment);
}

function segmentCompatible(a, b) {
  if (a.kind === 'static' && b.kind === 'static') return a.value === b.value;
  return true; // param / :x 都能吞任意一段（但只吞**一段**）
}

/** 两条路径模式是否可能指向**同一个**具体路径（对称：前向与后向共用）。 */
function pathsIntersect(a, b) {
  let i = 0;
  let j = 0;
  for (;;) {
    const x = a[i];
    const y = b[j];
    if (!x && !y) return true;
    // *path 吞掉剩余；axum 的 *path 至少吃一段 ⇒ 对面必须还有段
    if (x && x.kind === 'wild') return b.length - j >= 1;
    if (y && y.kind === 'wild') return a.length - i >= 1;
    if (!x || !y) return false; // 段数不等且没有通配兜底 ⇒ 不相交（:x 不跨 /）
    if (!segmentCompatible(x, y)) return false;
    i += 1;
    j += 1;
  }
}

/** 最接近的已注册路由（按段的编辑距离）—— 报错时给"是不是拼错了"一条线索。 */
function closestRoute(segs, routes) {
  let best = null;
  for (const r of routes) {
    const d = segDistance(segs, r.segments);
    if (!best || d < best.distance) best = { route: r, distance: d };
  }
  return best;
}

function segDistance(a, b) {
  const rows = a.length + 1;
  const cols = b.length + 1;
  const dp = Array.from({ length: rows }, (_, i) =>
    Array.from({ length: cols }, (_, j) => (i === 0 ? j : j === 0 ? i : 0)),
  );
  for (let i = 1; i < rows; i += 1) {
    for (let j = 1; j < cols; j += 1) {
      const same = a[i - 1].raw === b[j - 1].raw ? 0 : 1;
      dp[i][j] = Math.min(dp[i - 1][j] + 1, dp[i][j - 1] + 1, dp[i - 1][j - 1] + same);
    }
  }
  return dp[rows - 1][cols - 1];
}

// ---------------------------------------------------------------------------
// 四、两边各自的模型
// ---------------------------------------------------------------------------
/** route_manifest.py 的 dump → [{method, path, segments, handler}]；路径补上 nest("/api", …) 那一层。 */
function parseManifest(text) {
  const routes = [];
  for (const line of text.split('\n')) {
    const m = /^([A-Z?]+)\s+(\S+)\s+->\s+(\S+)$/.exec(line.trim());
    if (!m) continue;
    const path = '/api' + m[2];
    routes.push({ method: m[1], path, segments: segmentsOf(path), handler: m[3] });
  }
  return routes;
}

/** 扫一份源码 → 前端调用 [{file, line, literal, path, segments, method, callee}]。 */
function extractCalls(file, src) {
  const { code, literals } = stripComments(src);
  const calls = [];
  for (const lit of literals) {
    const head = lit.parts[0];
    if (!head || head.t !== 'lit' || !/^\/api(\/|$)/.test(head.v)) continue;
    const path = normalizePathExpr(code, lit);
    if (!path || path === '/api' || path === '/api/') continue;
    const callee = calleeOf(code, lit.start);
    calls.push({
      file,
      line: code.slice(0, lit.start).split('\n').length,
      literal: rawLiteralOf(src, lit),
      path,
      segments: segmentsOf(path),
      method: methodOf(code, lit.end, callee),
      callee,
    });
  }
  return calls;
}

function rawLiteralOf(src, lit) {
  const raw = src.slice(lit.start, lit.end).replace(/\s+/g, ' ');
  return raw.length > 72 ? raw.slice(0, 69) + '…' : raw;
}

/** 字面量左边紧挨着的调用名（api( / startJob( / new EventSource( ）。 */
function calleeOf(code, litStart) {
  let i = litStart - 1;
  while (i >= 0 && /\s/.test(code[i])) i -= 1;
  if (code[i] !== '(') return null;
  i -= 1;
  while (i >= 0 && /\s/.test(code[i])) i -= 1;
  const end = i + 1;
  while (i >= 0 && /[\w$]/.test(code[i])) i -= 1;
  return code.slice(i + 1, end) || null;
}

/**
 * method **只做提示**（不参与红）。推定顺序：
 * startJob/polishPost 恒 POST → EventSource 恒 GET → 实参里的 method: 'X' → api/apiOr 缺省 GET →
 * 其余（只造 URL 的 helper）认不出，返回 null。
 */
function methodOf(code, litEnd, callee) {
  if (callee === 'startJob' || callee === 'polishPost') return 'POST';
  if (callee === 'EventSource') return 'GET';
  if (callee !== 'api' && callee !== 'apiOr') return null;
  let depth = 0;
  let j = litEnd;
  while (j < code.length) {
    const ch = code[j];
    if (isQuote(ch)) {
      j = readLiteral(code, j).end;
      continue;
    }
    if (ch === '(') depth += 1;
    else if (ch === ')') {
      if (depth === 0) break;
      depth -= 1;
    }
    j += 1;
  }
  const m = /method\s*:\s*['"]([A-Za-z]+)['"]/.exec(code.slice(litEnd, j));
  return m ? m[1].toUpperCase() : 'GET';
}

// ---------------------------------------------------------------------------
// 五、差集（判据集中在这一个函数里，--mutate 翻的就是下面那一行）
// ---------------------------------------------------------------------------
function diffContract({ calls, routes, whitelist }) {
  const missing = [];
  const whitelisted = [];
  const hints = [];

  for (const call of calls) {
    const hits = routes.filter((r) => pathsIntersect(call.segments, r.segments));
    const routeHit = hits[0];
    const matched = routeHit !== undefined;
    if (!matched) {
      const wl = whitelist.find((w) => pathsIntersect(call.segments, segmentsOf(w.path)));
      if (wl) whitelisted.push({ call, whitelist: wl });
      else missing.push({ call, closest: closestRoute(call.segments, routes) });
      continue;
    }
    // method 只提示：路径命中了，但没有任何一条相交路由接受这个 method（ANY / ? 算通吃）
    if (call.method) {
      const ok = hits.some((r) => r.method === call.method || r.method === 'ANY' || r.method === '?');
      if (!ok) hints.push({ call, methods: [...new Set(hits.map((r) => r.method))] });
    }
  }

  // 后向：注册了、前端从不调 —— **只提示**
  const unused = routes.filter((route) => !calls.some((c) => pathsIntersect(c.segments, route.segments)));
  return { missing, whitelisted, hints, unused };
}

// ---------------------------------------------------------------------------
// 六、自证：内置小样本，证明判据**会红也会绿**（一条不会红的检查等于没检查）
// ---------------------------------------------------------------------------
const FIXTURE_MANIFEST = [
  '# sources: fixture (self-test only)',
  '',
  '## routes (sorted)',
  'GET     /health -> health',
  'GET     /jobs -> jobs_list',
  'GET     /jobs/:id -> job_get',
  'POST    /jobs/:id/cancel -> job_cancel',
  'GET     /jobs/:id/events -> job_events',
  'GET     /media/*path -> media_get',
  'GET     /polish/project -> polish_project_get',
  'PUT     /polish/project -> polish_project_put',
  'GET     /polish/stickers/:pack -> pack_detail',
  'GET     /polish/stickers/limits -> pack_limits',
  'POST    /polish/dhampir-status -> polish_dhampir_status',
  'GET     /stream -> global_stream',
  'PUT     /admin/users/:name/quota -> admin_set_user_quota',
  'POST    /ingest/records -> ingest_records_post',
  '',
].join('\n');

// 自证用假前端：**故意写成真源码的形态**（注释 / 模板串 / 拼接 / SSE），让归一化真的被走一遍。
const FIXTURE_FRONTEND = [
  '// 自证用假前端：注释里的 /api/never-listed 不许被当成真调用',
  "import { api } from './client'",
  '',
  '// 单引号（简单）',
  "export const jobs = () => api('/api/jobs')",
  '// 模板串 + 查询串：必须剥成 /api/polish/project',
  'export const proj = (pid, dir) => api(' + BT + '/api/polish/project?' + D + '{qs(pid, dir)}' + BT + ')',
  '// 插值段：必须归一化成 /api/admin/users/:x/quota',
  'export const quota = (u) =>',
  '  api(' + BT + '/api/admin/users/' + D + '{encodeURIComponent(u)}/quota' + BT + ", { method: 'PUT' })",
  '// 三段拼接：必须归一化成 /api/jobs/:x/cancel',
  "export const cancel = (id) => api('/api/jobs/' + encodeURIComponent(id) + '/cancel', { method: 'POST' })",
  '// 粘连插值（前面不是 /）= 查询串，不是段',
  'export const records = (s, l) => api(' + BT + '/api/ingest/records' + D + '{s}' + D + '{l}' + BT + ", { method: 'POST' })",
  '// 粘连拼接（三元）：也必须停在 /api/jobs',
  "export const list = (all) => api('/api/jobs' + (all ? '?all=1' : ''))",
  '// SSE：归一化后命中 /api/jobs/:id/events',
  "export const events = (id) => new EventSource('/api/jobs/' + encodeURIComponent(id) + '/events')",
  '',
].join('\n');

const FIXTURE_WHITELIST = [
  { path: '/api/stream', why: '自证：SSE' },
  { path: '/api/health', why: '自证：探针' },
  { path: '/api/media/*', why: '自证：媒体字节流' },
];

let selfTestChecks = 0;
function expect(ok, label, detail) {
  selfTestChecks += 1;
  if (ok) {
    console.log('  ✓ ' + label);
    return true;
  }
  console.error('  ✗ ' + label + (detail ? '\n      ' + detail : ''));
  return false;
}

function runSelfTest() {
  console.log('· 自证：内置样本（假路由表 + 假前端清单），证明判据**会红也会绿**');
  let bad = 0;
  const tally = (ok, label, detail) => {
    if (!expect(ok, label, detail)) bad += 1;
  };
  const routes = parseManifest(FIXTURE_MANIFEST);
  const calls = extractCalls('fixture/api.mjs', FIXTURE_FRONTEND);
  const routeOf = (p) => routes.find((r) => r.path === p);
  const callOf = (p) => calls.find((c) => c.path === p);

  tally(routes.length === 14, '假路由表解析出 14 条 (method, path)', '实际 ' + routes.length);
  tally(
    calls.length === 7 && calls.every((c) => c.path.startsWith('/api/')),
    '扫出 7 条调用且全部归一化成 /api/… 形态',
    calls.map((c) => c.path).join(' '),
  );

  // ① 模板串：先剥查询串
  tally(callOf('/api/polish/project') !== undefined, '模板串剥掉查询串 → /api/polish/project', calls.map((c) => c.path).join(' '));
  tally(
    pathsIntersect(callOf('/api/polish/project').segments, routeOf('/api/polish/project').segments),
    '剥掉查询串之后能命中 GET /polish/project',
  );

  // ② 插值段 → :x，且能与路由里的 :param 对上
  tally(
    callOf('/api/admin/users/:x/quota') !== undefined,
    '插值段归一化成 :x（/api/admin/users/:x/quota）',
    calls.map((c) => c.path).join(' '),
  );
  tally(
    pathsIntersect(callOf('/api/admin/users/:x/quota').segments, routeOf('/api/admin/users/:name/quota').segments),
    ':x 能匹配路由里的 :param（/admin/users/:name/quota）',
  );

  // ③ `+` 拼接
  tally(callOf('/api/jobs/:x/cancel') !== undefined, '+ 拼接三段归一化成 /api/jobs/:x/cancel', calls.map((c) => c.path).join(' '));
  tally(
    pathsIntersect(callOf('/api/jobs/:x/cancel').segments, routeOf('/api/jobs/:id/cancel').segments),
    '拼接串能命中 POST /jobs/:id/cancel',
  );
  tally(callOf('/api/jobs/:x/cancel').method === 'POST', '拼接串的 method 提示读到 POST', callOf('/api/jobs/:x/cancel').method);

  // ④ 粘连插值 = 查询串 / 后缀，不是段
  tally(callOf('/api/ingest/records') !== undefined, '粘连插值不许被当成段（/api/ingest/records）', calls.map((c) => c.path).join(' '));
  tally(callOf('/api/jobs') !== undefined && callOf('/api/jobs').method === 'GET', '粘连 + 三元停在 /api/jobs 且 method 推定 GET');

  // ⑤ :x 只吞一段（不跨 /），通配至少吃一段
  tally(!pathsIntersect(segmentsOf('/api/jobs/:x'), segmentsOf('/api/jobs/:id/cancel')), ':x 不跨 /（/api/jobs/:x ≠ /api/jobs/:id/cancel）');
  tally(pathsIntersect(segmentsOf('/api/jobs/:x'), segmentsOf('/api/jobs/:id')), ':x 只吞一段也能命中 /api/jobs/:id');
  tally(pathsIntersect(segmentsOf('/api/media/clip.mp4'), segmentsOf('/api/media/*path')), '通配 *path 能吞剩余段');
  tally(!pathsIntersect(segmentsOf('/api/media'), segmentsOf('/api/media/*path')), '通配 *path 至少吃一段（/api/media 本身不算命中）');

  // ⑥ 缺路由必须**红**
  const ghost = diffContract({
    calls: extractCalls('fixture/ghost.mjs', "api('/api/does-not-exist')\n"),
    routes,
    whitelist: FIXTURE_WHITELIST,
  });
  tally(
    ghost.missing.length === 1 && ghost.missing[0].call.path === '/api/does-not-exist',
    '缺路由：必须进 missing（红）',
    JSON.stringify(ghost.missing.map((m) => m.call.path)),
  );
  tally(!!(ghost.missing[0] && ghost.missing[0].closest && ghost.missing[0].closest.route), '缺路由：要给出最接近的已注册路由');

  // ⑦ 白名单：只降级；且**不许吞掉真差集**（段级相交，不是前缀）
  // ⚠️ 这一格必须用一张**没有** GET /health 的路由表 —— 白名单只在"没命中已注册路由"时才生效，
  // 拿一张注册了 /health 的表来测，测的其实是"命中"，白名单永远为空（第一版就是这么空转的）。
  const wlRoutes = routes.filter((r) => r.path !== '/api/health');
  const wl = diffContract({
    calls: extractCalls('fixture/wl.mjs', [
      "api('/api/health')",
      "api('/api/healthz')",
      "api('/api/stream/extra')",
      '',
    ].join('\n')),
    routes: wlRoutes,
    whitelist: FIXTURE_WHITELIST,
  });
  tally(wl.whitelisted.length === 1 && wl.whitelisted[0].call.path === '/api/health', '白名单命中 ⇒ 不红（/api/health）', JSON.stringify(wl.whitelisted.map((w) => w.call.path)));
  tally(
    wl.missing.length === 2 && wl.missing.map((m) => m.call.path).join(',') === '/api/healthz,/api/stream/extra',
    '白名单**不许**吞掉真差集（/api/healthz、/api/stream/extra 必须红）',
    JSON.stringify(wl.missing.map((m) => m.call.path)),
  );

  // ⑧ 全命中样本必须绿（判据不是永远红）
  const clean = diffContract({ calls, routes, whitelist: FIXTURE_WHITELIST });
  tally(clean.missing.length === 0, '全命中样本：missing 为空（判据会绿）', JSON.stringify(clean.missing.map((m) => m.call.path)));
  tally(clean.unused.length > 0, '后向：注册了没人调的必须出现在 unused 里（只提示）');

  // ⑨ method 只提示、不红
  const mres = diffContract({
    calls: extractCalls('fixture/method.mjs', "api('/api/polish/dhampir-status')\n"),
    routes,
    whitelist: FIXTURE_WHITELIST,
  });
  tally(
    mres.missing.length === 0 && mres.hints.length === 1,
    'method 不符：只进 hints、不进 missing（不红）',
    JSON.stringify({ missing: mres.missing.length, hints: mres.hints.length }),
  );

  // ⑩ 注释里的 /api/… 不算调用
  tally(!calls.some((c) => c.path.includes('never-listed')), '注释里的 /api/… 不算调用');

  if (bad > 0) {
    console.error('\n✗ 自证失败：' + bad + ' 条判据抓不住它该抓的东西');
    return 1;
  }
  console.log('✓ 自证通过：' + selfTestChecks + ' 项（缺路由会红 / 模板串归一化后命中 / 白名单不影响真差集）');
  return 0;
}

// ---------------------------------------------------------------------------
// 七、变异自证：把差集判据翻面，自证必须失败
// ---------------------------------------------------------------------------
// 为什么要有它：判据写反（或写成恒真）的表现是"看起来全过"，比报错难发现得多。
// 这里**真改源码**（拷到临时文件里只替换那一行），再跑同一份自证。
// ⚠️ 先断言"锚点恰好命中一次"：少了这条，没替换上的变异体会原样跑绿，看着像"判据抓不住"。
// ⚠️ 锚点必须**拼**出来：整段写成字面量的话，它在这个表里自己也算一次命中
// （实测 `.split(anchor).length - 1 === 2`，"命中一次"的断言当场把变异台顶翻 —— 那是好事，
//  但它顶翻的原因是锚点重复、不是判据有问题，所以从根上消掉）。
const MUTATION = {
  from: 'const matched = routeHit ' + '!==' + ' undefined;',
  to: 'const matched = routeHit ' + '===' + ' undefined;',
  label: '差集判据 !== → ===（翻面）',
};

function runMutation() {
  console.log('· 变异自证：' + MUTATION.label);
  const src = readFileSync(join(here, 'check-api-contract.mjs'), 'utf8');
  const hits = src.split(MUTATION.from).length - 1;
  if (hits !== 1) {
    console.error('  ✗ 变异锚点在源码里出现 ' + hits + ' 次（必须恰好 1 次）—— 变异台自己错了');
    return 1;
  }
  const mutated = src.replace(MUTATION.from, MUTATION.to);
  if (mutated === src) {
    console.error('  ✗ 替换之后文本没变（变异台自己错了）');
    return 1;
  }
  const tmp = join(tmpdir(), 'check-api-contract.mutant.mjs');
  writeFileSync(tmp, mutated, 'utf8');
  let code = 0;
  let out = '';
  try {
    out = execFileSync(process.execPath, [tmp, '--self-test'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
  } catch (e) {
    code = e.status ?? 1;
    out = String(e.stdout ?? '') + String(e.stderr ?? '');
  } finally {
    try {
      unlinkSync(tmp);
    } catch {}
  }
  if (code === 0) {
    console.error('  ✗ 判据**没有**翻面：变异体跑自证照样通过 ⇒ 这条判据是空转的');
    console.error(out.trim().split('\n').slice(-6).join('\n'));
    return 1;
  }
  console.log('  ✓ 变异体 exit ' + code + '（原版 exit 0）⇒ 判据真的在判断');
  for (const line of out.split('\n').filter((l) => l.includes('✗')).slice(0, 4)) {
    console.log('    ' + line.trim());
  }
  console.log('✓ 变异自证通过（' + MUTATION.label + '）');
  return 0;
}

// ---------------------------------------------------------------------------
// 八、主检查
// ---------------------------------------------------------------------------
function argValue(argv, name, fallback) {
  const i = argv.indexOf(name);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback;
}

function collectFiles(dir, out = []) {
  for (const name of readdirSync(dir).sort()) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) collectFiles(p, out);
    else if (/\.(js|mjs|vue)$/.test(name)) out.push(p);
  }
  return out;
}

function main() {
  const argv = process.argv.slice(2);
  const root = resolve(argValue(argv, '--root', resolve(here, '..')));
  const verbose = argv.includes('-v') || argv.includes('--verbose');

  if (argv.includes('--mutate')) return runMutation();
  if (argv.includes('--self-test')) return runSelfTest();

  // ---- 1) 路由清单：**不改 route_manifest.py 的语义**（它 exit 恒 0，比较是这里的事）----
  const manifestPath = join(tmpdir(), 'yekitrim-routes.txt');
  execFileSync('python3', ['scripts/route_manifest.py', '--root', '.', '--out', manifestPath], {
    cwd: root,
    stdio: ['ignore', 'ignore', 'pipe'],
  });
  const routes = parseManifest(readFileSync(manifestPath, 'utf8'));
  if (routes.length === 0) {
    console.error('✗ 路由清单是空的 —— route_manifest.py 没抽到任何 .route(...)，先修那边');
    return 1;
  }

  // ⚠️ 交叉验证（2026-11-24 补，**不改 route_manifest.py 的语义**）：它是**源码级**正则抽取器、
  // 不剥注释 ⇒ 一行被 `//` 注释掉的 `.route(...)` 在清单里仍然算"已注册"，而真正 build 出来的
  // router 里没有它 ⇒ 前向判据会**假绿**（前端调、生产 404、门禁却绿）。
  // 我这里只把"注释掉的注册"单独判红：要么删掉这行，要么改回真注册。实测这条是必需的
  // —— 反自证②第一次就是"注释掉 routes.rs 的 /open-path，门禁照旧绿"（清单把它当成了已注册）。
  const apiRsDir = join(root, 'crates/yekitrim-server/src/api');
  const zombies = [];
  for (const name of readdirSync(apiRsDir).filter((n) => n.endsWith('.rs')).sort()) {
    readFileSync(join(apiRsDir, name), 'utf8')
      .split('\n')
      .forEach((line, idx) => {
        const m = /^\s*\/\/.*\.route\s*\(\s*"(\/[^"]*)"/.exec(line);
        if (m) zombies.push({ file: 'crates/yekitrim-server/src/api/' + name, line: idx + 1, path: '/api' + m[1] });
      });
  }

  // ---- 2) 前端 request 层 ----
  const apiDir = join(root, 'webui/src/api');
  const calls = readdirSync(apiDir)
    .filter((n) => n.endsWith('.js'))
    .sort()
    .flatMap((n) => extractCalls('webui/src/api/' + n, readFileSync(join(apiDir, n), 'utf8')));
  if (calls.length === 0) {
    console.error('✗ webui/src/api/*.js 里一条 /api 字面量都没扫到 —— 判据失效的表现就是"全绿"');
    return 1;
  }

  // 视图 / stores 里直连的 /api 字面量：**只用来给后向清单做注解**（不参与红）。
  // 后向那栏问的是"前端从不调"，而前端不等于 api/ 一层：把 views/stores 的引用算进来，
  // 否则 /api/projects、/api/health 这些会被当成死路由，清单立刻变成没人看的噪音。
  const elsewhere = collectFiles(join(root, 'webui/src'))
    .filter((p) => !p.startsWith(apiDir))
    .flatMap((p) => extractCalls(relative(root, p).replace(/\\/g, '/'), readFileSync(p, 'utf8')));

  const result = diffContract({ calls, routes, whitelist: WHITELIST });
  const singleQuoted = calls.filter((c) => !c.literal.startsWith(BT)).length;
  const templateQuoted = calls.length - singleQuoted;

  // ---- 3) 报告 ----
  console.log('· 契约：前端 webui/src/api/*.js ←→ 服务端 api/routes.rs');
  console.log(
    '  服务端注册 ' + routes.length + ' 条 (method, path) / ' + new Set(routes.map((r) => r.path)).size + ' 条路径' +
      '；前端 api 层 ' + calls.length + ' 条字面量（单引号 ' + singleQuoted + ' / 模板串与拼接 ' + templateQuoted + '）' +
      ' → ' + new Set(calls.map((c) => c.path)).size + ' 条路径',
  );

  if (verbose) {
    console.log('');
    console.log('· 前端 api 层逐条（归一化之后；-v 才打）：');
    for (const c of calls) {
      console.log('    ' + (c.file + ':' + c.line).padEnd(32) + (c.method ?? '??').padEnd(7) + c.path);
    }
  }

  if (result.whitelisted.length > 0) {
    console.log('');
    console.log('· 白名单命中（不红；理由见脚本里的 WHITELIST 表）：');
    for (const w of result.whitelisted) {
      console.log('    ' + w.call.file + ':' + w.call.line + '  ' + w.call.path + '  ← ' + w.whitelist.path);
    }
  }

  if (result.hints.length > 0) {
    console.log('');
    console.log('· method 提示（路径命中、但相交路由都不接受这个 method —— **不红**，axum 会回 405）：');
    for (const h of result.hints) {
      console.log('    ' + h.call.file + ':' + h.call.line + '  ' + h.call.method + ' ' + h.call.path + '（已注册：' + h.methods.join('/') + '）');
    }
  }

  const unused = [];
  for (const route of result.unused) {
    if (WHITELIST.some((w) => pathsIntersect(route.segments, segmentsOf(w.path)))) continue;
    if (elsewhere.some((c) => pathsIntersect(c.segments, route.segments))) continue;
    const prev = unused.find((u) => u.path === route.path);
    if (prev) prev.methods.push(route.method);
    else unused.push({ path: route.path, methods: [route.method], handler: route.handler });
  }
  console.log('');
  console.log(
    '· 后向（**只提示**，不红）：' + unused.length + ' 条已注册路径前端一处都没调' +
      '（api 层 + views/stores 都算过；CLI / 运维 / 托盘会用它们）',
  );
  const shown = verbose ? unused : unused.slice(0, 12);
  for (const u of shown) console.log('    ' + u.methods.join(',').padEnd(12) + u.path + '  -> ' + u.handler);
  if (shown.length < unused.length) {
    console.log('    … 还有 ' + (unused.length - shown.length) + ' 条（node scripts/check-api-contract.mjs -v 看全部）');
  }

  // ---- 4) 判据：只认"前端调了、后端没有"（外加"注册被注释掉"这条交叉验证）----
  if (zombies.length > 0) {
    console.error('');
    console.error('✗ routes.rs 里有 ' + zombies.length + ' 条被注释掉的 .route(...)：');
    for (const z of zombies) {
      console.error('  · ' + z.file + ':' + z.line + '  ' + z.path);
    }
    console.error('  route_manifest.py 不剥注释 ⇒ 清单会把它当成"已注册"（假绿）。删掉这行，或改回真注册。');
    return 1;
  }

  if (result.missing.length > 0) {
    console.error('');
    console.error('✗ 契约断裂：' + result.missing.length + ' 条前端调用的路由服务端**没有注册**');
    console.error('  （浏览器会命中 SPA 回退拿到 200 HTML，症状是"点了没反应"）');
    for (const m of result.missing) {
      console.error('');
      console.error('  · ' + m.call.file + ':' + m.call.line + '  ' + m.call.path);
      console.error('      字面量：' + m.call.literal);
      console.error(
        m.closest
          ? '      最接近的已注册路由：' + m.closest.route.method + ' ' + m.closest.route.path + '（段距离 ' + m.closest.distance + '）'
          : '      没有任何已注册路由可与它比 —— 路由表是不是空了？',
      );
    }
    console.error('');
    console.error('  修法：① 后端 routes.rs 补一条 .route(...)；② 前端改回已注册的路径；');
    console.error('        ③ 确实是"不走 api() 的通道"（SSE / 字节流 / 静态资源）⇒ 加进 WHITELIST **并写理由**。');
    return 1;
  }

  console.log('');
  console.log(
    '✓ 契约：前端 api 层 ' + calls.length + ' 条字面量全部命中已注册路由' +
      '（其中白名单 ' + result.whitelisted.length + ' 条）；后向 ' + unused.length + ' 条只提示',
  );
  return 0;
}

process.exit(main());
