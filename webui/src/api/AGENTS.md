# webui/src/api — 前端唯一的请求层（作用域与 HTTP 都只在这里拼）

> 范围：`webui/src/api`/** ｜ 上游：整个前端（`views/`、`components/`、`stores/`、`composables/`、`polish/`）｜ 验证：`npm --prefix webui run build`

## 职责

前端所有 HTTP 调用的唯一出口：作用域查询串、CSRF、超时、留痕、401 回跳都在这层收口，
视图/组件只传不透明的 `pid` 与项目内相对 `dir`。服务端契约：所有 `/api/project-data/*` 与
`/api/polish/*` 都要求 `project=<id>`（或 `{project, dir}`），**不再接受任意路径**。

## 入口

- `scope.js`：`scopeState`（`pid`/`project`）、`setScope(pid, project)`、`scopeQ(pid)`（`project=12`，
  **不含前导 `?`**）、`scopeQuery(pid)`（带 `?`）。
- `client.js`：`api(path, opts)`（唯一入口）、`apiOr(path, fallback, label)`、`readCookie(name)`、
  `csrfHeaders()`、`setUnauthorizedHandler(fn)`、`currentRoute()`、`redirectToLogin()`。
- `projectData.js`：`projectData.*`（status / events / clips / polishMap / config / transcript / assets /
  pipeline / cut / cutAll / deleteClip / silence / thumbUrl / correct / saveTranscript / saveEvents /
  saveConfig）。
- `polish.js`：内部 `startJob(url, pid, dir, extra)`（=`polishPost`）+ `initPolish` / `runLlm` /
  `runLlmSegment` / `runProcess` / `runSilence` / `runRender` / `runCover` / `coverTitle` / `coverFrame` /
  `coverConfig` / `coverBackground` / `coverPreview` / `fetchProject` / `fetchSubtitles` / `saveSubtitles` /
  `saveProjectPatch` / `fetchStickerSources` / `fetchStickerLibrary` / `stickerFileUrl` / `fetchSfxCatalog` /
  `fontUrl` / `fetchFonts`；`watchJob` 从 `jobs.js` 再导出。
- `jobs.js`：`jobsApi.{list,detail,cancel}`、`watchJob(id, {onEvent,onSummary,onDone})`（SSE 优先、
  轮询兜底）、`clipKey` / `outKey` / `idxKey` / `busyOn` / `snapshotStale` / `KIND_ZH` / `CLASS_ZH`。
- `auth.js`：`authApi`（登录 / 登出 / 当前用户 / 改密）；`ingest.js`：`ingestApi` + `PIPELINES`；
  `stickers.js`：`stickersApi` + `UPLOAD_MAX_BYTES`（全局贴纸库，接口全在 `/api/admin/*`，仅 admin）。

## 边界

- ⚠ **作用域单点**：`api/scope.js` 的 `scopeQ(pid)`；业务调用只在 `api/projectData.js` /
  `api/polish.js` / `api/jobs.js` 三处拼查询串，**视图/组件不自己拼**。门禁：`views/`、
  `components/`、`stores/`、`composables/` 里**不许出现 `[?&]pid=`**
  （`python3 scripts/check_module_boundaries.py` 会卡这条）。
- `client.js::api()` 的约定：
  - **只认 JSON**：`Content-Type` 不含 `application/json` 就报「接口不存在或返回格式错误」——
    后端只服务 `webui/dist`，缺失的 `/api` 路由会命中 SPA 回退并返回 **200 HTML**，不显式报错
    调用方会拿到 `null` 当成功。
  - **可传 `File` 覆盖 `Content-Type`**（`api(path, {method:'POST', body: file, headers:{'Content-Type':'application/octet-stream'}})`）——
    上传 zip 压缩包走的就是这条（不是 multipart）。
  - **CSRF**：写请求自动注入 `X-CSRF-Token`（= `yekitrim_csrf` cookie，双提交），调用方不关心。
  - **留痕**：失败（网络错误 / 非 2xx / `ok:false`）→ `console.warn` + 设置页「运行日志」；
    成功只写 `console.debug`；高频只读接口（`/api/media`、`/api/health`、`/api/jobs`、`/api/stream`、
    `/api/admin/logs`）成功时连面板都不写（否则 3 秒一次的轮询会冲干净 2000 条缓冲）。
  - **超时**：默认 10 分钟这个**很松**的上限（`/api/polish/silence`、`cover-title`、`ingest/scan`
    都是同步长接口，卡紧了就是把"慢"改成"失败"）；按接口自给 —— `jobsApi` 传 10s（它才是冻住的
    源头），确实要更久的传 `{ timeout: 0 }`。请求**必须一定会 settle**，否则 `useJobs` 的去重
    `inflight`（只由 `.finally` 清）会被永久占住。
  - **401**：默认统一跳登录并记回跳；启动时的 `GET /api/me` 与登录请求传 `silent401: true` 自己处理。
- ⚠ **`apiOr` 只用来兜值，不许把"读取失败"伪装成"没有数据"**：`api(...).catch(() => [])` 是同类
  反模式。精修时间轴的读路径（`TimelineEditor::loadData`）**必须直接调 `api()`**，失败置
  `loadError` 并**暂停写入**（详见 `webui/src/AGENTS.md` 的读失败纪律）。
- ⚠ **上传上限是双份常量**：`stickers.js::UPLOAD_MAX_BYTES` 与 Rust
  `crates/yekitrim-server/src/stickers.rs::UPLOAD_MAX_BYTES` 是**同一个数**（**改一处要改两处**，
  Rust 侧有单测把它钉在 64 MiB）。**前端必须先挡**：axum 超限回的是 **413 + 纯文本**，而
  `client.js` 只认 JSON，会把"文件太大"报成"接口不存在或返回格式错误"。
- **`jobs.js` 的资源键必须与后端 `yekitrim-server/src/server/scope_key.rs` 一致**（`clipKey`/`outKey`/
  `idxKey`）：前端用同一套键判断"我这个片段/项目忙不忙"，只在自己作用域被占用时才禁用按钮。
  `busyOn` 只是提前提示、**不是正确性保证**（正确性由服务端排队兜），所以 `snapshotStale` 超时后
  各视图要把它的结果降级成 `null`，宁可放行也不要把按钮永远灰着。
- ⚠️ **`watchJob` 拿到 404 必须收订阅**：任务表在内存里（`crates/yekitrim-server/src/jobs.rs`），
  服务重启会清空全部历史，而浏览器标签页可以开一整夜 —— 旧口径只写一条 debug 就接着 2 秒一次地
  轮询，服务端日志被 `GET /api/jobs/<旧 id> → 404` 刷满（实测一晚几百条），页面还永远停在「运行中」。
  现在 404 = 终态 `lost`（`STATUS_ZH.lost`「记录已失效」）+ 一条 warn + 收订阅；
  **其余失败（网络/超时）仍照旧重试**，别一起收。
- ⚠️ **出片通道（`polish.js::runRender(pid, dir, outputFile, renderer)`）留空 = 跟随默认**：
  默认通道只有一处定义（CLI 的 `--renderer`，现在是底座 `dhampir`），前端**不许写回落**
  （写 `|| 'dhampir'` 就成了第二处默认值，两边一漂就变成"看着选了底座、其实在跑别的"）。
  服务端对认不出的值回 **400**（`/polish/render` 的白名单）。
- **`polish.js` 的写请求统一带 `{project, dir}`**，`dir` 是**项目内相对路径**（如
  `output/clips/clip-01`，由 `clipRelDir(clipName)` 造）；**路由参数 `:clip` 用目录名、`polishMap`
  用绝对 `dir` 索引**，两者别混（详见 `webui/src/AGENTS.md`）。
- 各文件只服务一个领域，新增接口放**对应文件**、不要新建"杂项 api"；需要新作用域形态时先改
  `scope.js`，别在调用点手拼。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
npm --prefix webui run build
python3 scripts/check_module_boundaries.py
```

`npm --prefix webui run build` 是语法/导入门禁（本卡没有单测运行器）；边界门禁会检查
**作用域查询串只许在 `webui/src/api/` 里拼**（`views/`、`components/`、`stores/`、`composables/`
里出现 `[?&]pid=` 即失败）。
