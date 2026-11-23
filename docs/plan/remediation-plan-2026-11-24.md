# YekiTrim 整改计划（2026-11-24）

> 依据：`docs/plan/architecture-review-2026-11-24.md`（问题清单与证据）。
> 状态：**计划书，未动一行代码。**
> 口径（用户 2026-11-24 拍板）：
> 1. **全量**覆盖 —— 结构层 2 项 + 次级 5 项 + 快赢 7 条 + 10-15 遗留 6 项。
> 2. **先本地 `gate.mjs`，再谈 CI** —— 本计划**不含** CI 工作流落地，只在 P5 里写前置条件。
> 3. **DOM 近似预览允许语义接近**（不要求与成片逐像素一致）—— 这条决定了 P3 的收敛方向。

**每个任务都有四件事**：改哪里 / 怎么算做完 / 怎么验证 / 破了什么。没有"验证"的一律不算任务。

**三条硬约束（做完 P0 之前不要开始别的）**：
1. **T3（卡瘦身）必须先于任何"新增目录"** —— 最坏指令链只剩 6 字节，`root+server+api` 只剩 22 字节。
2. **T5（gate）必须先于"大规模重构"** —— 否则 P3/P4 的搬运没有回归网。
3. **T10（401 兜底）是安全改动，单独一个提交、单独一次发版**，不和别的混。

---

## 0. 总览与顺序

| 阶段 | 任务 | 体量 | 相互依赖 |
|---|---|---|---|
| **P0 止血** | T1、T2、T4、T6（+ 先做 T3） | 每条 ≤ 1 小时（T3 半天） | T3 是 P1/P4 的前置 |
| **P1 门禁执行化** | T5、T7、T8 | 2–3 天 | 需 T3 先腾出预算 |
| **P2 契约与正确性** | T9–T12 | 2–4 天 | T9 需 T7 的路由清单 |
| **P3 重复真相收敛** | T13–T15 | 3–5 天 | 用户口径已给（语义接近） |
| **P4 结构拆分** | T16–T19 | 5–8 天 | T16 必须在 T3 之后；T17/T18 无依赖 |
| **P5 收尾** | T20–T23 | 1–2 天 | — |

**贯穿纪律**（每次提交都要跑）：
```bash
python3 scripts/check_module_map.py
python3 scripts/check_module_boundaries.py
```
P0 完成后追加 `node scripts/gate.mjs`（T5 建成后）。

---

## P0 止血（当天，未做会继续失血）

### T1 删除 `crates/yekitrim-server/%SystemDrive%/`

- **改哪里**：删整个目录（932 KB，3 个 Windows 字体缓存 `.db`，2026-10-02 创建）。
- **为什么**：某次带未展开 `%SystemDrive%` 的路径操作留下的垃圾。不在 git 里（被 `*.db` 忽略），
  但会被任何 zip/备份/`cargo xtask package` 的目录遍历带上，也会让 IDE 索引莫名变慢。
- **怎么算做完**：`ls crates/yekitrim-server/` 只剩 AGENTS.md / Cargo.toml / build.rs / src。
- **怎么验证**：`git status --porcelain -uall | grep -c SystemDrive` = 0；`cargo check -p yekitrim-server` 仍 0 warning。
- **破了什么**：不破任何东西。**若担心**是某个构建脚本的产物，删完后全量跑一次
  `cargo check --workspace --all-targets` 确认没有东西往那里写。

### T2 `.gitignore` 补 `/video/`

- **改哪里**：根 `.gitignore`，在「运行时数据」那一段加：
  ```
  # 宣传/实验素材（2026-11-24）：video/ 是 2693 个未跟踪文件 / 85 MB 的抽帧与截图，
  # *.mp4 已挡，但 *.jpg/*.png 没挡 —— 一次 `git add -A` 就会全卷进来（2026-10-11 出过同类事故）。
  /video/
  ```
  如果 `video/assets/real/*.jpg` 里有**要入库**的样例，把它们先移到 `docs/plan/screenshots/` 或
  `experiments/`（后者已 gitignore）再忽略整个 `video/`。
- **怎么算做完**：`git status --porcelain -uall | wc -l`（当前 **2694**，其中 **2689** 在 `video/`）降到个位数。
- **怎么验证**：`git status --porcelain -uall | grep -c video/` = 0；`git check-ignore -v video/assets/real/compare.jpg` 命中。
- **破了什么**：不破代码。**副作用**：`video/` 里的文件从此不进仓库 —— 若要发布宣传视频，走 release 附件。

### T3 AGENTS 卡片瘦身（腾出指令链预算）

**这是唯一一条"不做就会卡住 P4"的任务。**

- **现状**：门禁输出
  ```
  最坏链 57338/57344 B（root + webui + webui-src + webui-components）
  root+server+api 链 = 57322 B
  ```
  两条链都只剩 **个位数到 22 字节**。而 `docs/module-map.json` 规定「新增目录 = 新增卡」，
  于是 P4 拆 `api/` 的第一步就会被门禁拒绝。
- **改哪里**：
  1. 主目标 `webui/src/components/AGENTS.md`（22,277 B，25 条 ⚠️）—— 把**已成代码的历史叙述**
     （"以前是 X、2026-09-30 改成 Y、实测数据 …"）整段挪去 `docs/webui-components.md`
     （该文件已存在，141 KB，就是干这个的），卡里只留**当前规矩 + 一句指针**。
  2. 次目标 `crates/yekitrim-server/src/api/AGENTS.md`（22,828 B）—— 同样的手法，
     挪去 `docs/plan/` 或新建 `docs/server-api.md`。
  3. 参考标尺：`webui/src/api/AGENTS.md` 只有 7,362 B，仍然说清了"唯一出口 + 边界"—— 目标是这个密度。
- **怎么算做完**：最坏链 `root + webui + webui-src + webui-components` ≤ **50,000 B**（留 7 KB 余量）；
  `root + server + api` ≤ 50,000 B。
- **怎么验证**：`python3 scripts/check_module_map.py` 打印的"最坏链"两项都 ≤ 50000。
  ⚠️ 这道门禁还有一条 **`warning_floor: 117`**（⚠️ 行数下限，只许多不许少）——
  实测当前是 **602 行**，余量很大，所以搬走历史叙述时只要别顺手删 ⚠️ 行就不会红；
  但"搬"与"删"要分清：**只搬不带 ⚠️ 的叙述**。
- **破了什么**：无代码。风险是"搬走的内容以后没人看" —— 所以每段搬走时**必须在卡里留一行指针**
  （`> 历史与实测数据见 docs/…`），且 `docs/` 那份要有标题层级，能被 grep 到。

### T4 同步 IO 三处补 `blocking`

- **改哪里**（三处都已定位到行）：
  1. `crates/yekitrim-server/src/api/project_data.rs:1206` `clip_delete_impl` ——
     `std::fs::remove_dir_all(&clip_dir)`（1228）+ `prune_clip_from_index`（1230，内部读两个索引文件）。
     把这两步整体包进 `blocking(move || …)`；注意 `busy_with` 的检查必须**留在外面**（它是 async）。
  2. `crates/yekitrim-server/src/api/projects.rs:372` `projects_source_delete` ——
     递归 `read_dir` 累计大小（396/423），整段进 `blocking`。
  3. `crates/yekitrim-server/src/api/projects.rs:162` `create_local_project` ——
     `create_dir_all`（173）+ `read_dir` 扫候选（312），进 `blocking`。
- **为什么**：根卡的第一条全局不变式。这三处在网络盘/慢盘上会直接占住 tokio worker
  （`clip_delete_impl` 删的是含成片的整个片段目录，最重）。
- **怎么算做完**：三处的同步 FS 调用都在 `blocking(…)` 闭包内。
- **怎么验证**：① `cargo test -p yekitrim-server`；
  ② 机械复查：对这三个函数名 grep，函数体内 `std::fs::` 的每一行都在 `blocking(move || …)` 的缩进块里；
  ③ **新增**一条单测（参照 `api/mod.rs::blocking_helper_keeps_workers_free` 的写法）：
  用一个单 worker 的 runtime 证明 `clip_delete_impl` 不会阻塞并发请求。
- **破了什么**：`blocking` 闭包要求 `'static`，`ws: &Workspace` / `&AppState` 需要先 clone 出
  所需字段（`output_dir` 的 `PathBuf`、`clip_dir` 的 `PathBuf`）。改完检查借用是否能过。

### T5 `scripts/gate.mjs`（统一执行入口，**不含 CI**）

- **新建**：`scripts/gate.mjs`，零依赖（node 内置模块）。
- **它要做什么**：
  1. **按改动面选脚本**：读 `git diff --name-only <base>`，映射到要跑的脚本
     （`webui/src/**` → `webui/scripts/check-*.mjs` + `npm --prefix webui run build` 的指纹校验；
      `crates/yekitrim-polish/**` → `scripts/check-*parity*.mjs` + `cargo test -p yekitrim-polish`；…）。
     映射表**写在 gate.mjs 里并加注释说明理由**，不要另开配置文件。
  2. **`--all`**：全量跑（给发版前用）。
  3. **任何"没跑"都必须显式打印** —— 这是本任务唯一不可妥协的判据：
     `SKIP check-xxx: 缺 fixture <path>（用 --require-sample 让它变红）`。
     理由是本仓已有的教训：`yekitrim-compare.mjs` 曾因参数白名单漏了 `--golden` 而**静默换参照**，
     "数字还都在阈值内"。静默跳过比不跑更糟。
  4. **守卫自检一起跑**：32 个脚本支持 `--self-test` / `--mutate`，gate 必须把它们跑上
     （`check-effect-registry` 烂了很久就是因为"主检查进了 gates、自检没进"）。
- **怎么算做完**：`node scripts/gate.mjs --all` 在当前工作树**能跑完并给出汇总表**
  （通过 N / 跳过 M / 失败 K，每行带脚本名与耗时）。首次全量跑**允许有红** ——
  红的先登记进 `scripts/gate-baseline.json`（见下），不要求当天修完。
- **怎么验证**：① 故意改坏一个前端文件，`gate.mjs` 必须选中对应脚本并变红；
  ② 把某脚本的 fixture 删掉，输出必须是 `SKIP` 而**不是**"通过"；
  ③ `node scripts/gate.mjs --all` 的退出码 = 是否有**非基线**失败。
- **破了什么**：不破。**已知风险**：`check-layout-invariants` 与 `check-polish-page-loads`
  带 CDP 探针的注释/可选路径（静态部分不需要浏览器，真几何靠 `.workbuddy/probe-*.mjs`，
  那脚本已 gitignore），所以它们在 gate 里**应能直接跑**；真正需要浏览器的只有
  `check-polish-page-loads` 的页面加载分支。无论哪种，**跑不了就按第 3 条显式 SKIP**，别静默算过。
- **基线文件**：`scripts/gate-baseline.json`，形如 `{"check-dhampir-wiring.mjs": "本机底座 host_api 低一版（已知红）"}`。
  **基线里的每一条都必须带理由字符串**，不允许空值 —— 否则它就是第二个"静默跳过"。

### T6 保留期与每日清理

- **现状三处都没有保留期**：`purge_expired_sessions` 只在启动时调一次（`auth.rs:209`），
  注释写的是"启动 + 每日一次的后台任务用" —— **那个每日任务不存在**；`audit` 表无清理；
  `llm_usage` 表无清理；`<data_dir>/logs/yekitrim-YYYYMMDD.log` 每天一份、**永不删除**
  （`render_proc::prune_logs` 只匹配 `render-*.log`，保留 20 份）。
- **改哪里**：
  1. `crates/yekitrim-server/src/jobs.rs` 的 `spawn_housekeeping`（已有 5s ticker）**不要挂在这里** ——
     它是 5s 粒度，语义不符。在 `lib.rs` 里另起一个 `spawn_daily_maintenance`（`tokio::time::interval(24h)`，
     首次延迟到启动后 10 分钟），内容是：`auth.purge_expired()` + `db.purge_old_audit(N 天)` +
     `db.purge_old_llm_usage(N 天)` + `logbus::prune_old_logs(N 天)`。
  2. 三个新函数都放 `yekitrim-service`（它才碰 DB），server 只编排。
     **保留天数进配置**（`[storage] audit_keep_days / usage_keep_days / log_keep_days`，
     默认 90/365/30），按 cfg 五步协议加（字段 + `merge()` + 手写 `Default` + 模板 + 版本登记）。
     **第 0 步是查重（已查）**：`config/types.rs:652` 的 `StorageConfig` 现在只有
     `projects_root` / `max_disk_bytes` / `disk_watermark_pct` / `default_quota_bytes` 四项，
     **没有**任何保留期字段 —— 所以三个 `*_keep_days` 是新增，不会撞名。
     注意这个结构体的每个字段都带 `#[serde(default…)]`，新字段必须同样带
     （否则老配置文件会读不出来）。
- **怎么算做完**：启动 10 分钟后日志里出现一条 `daily maintenance: sessions=… audit=… usage=… logs=…`。
- **怎么验证**：① 单元测试：三个 `purge_old_*` 各造 2 条新旧数据，断言只删旧的（纯 SQL，好写）；
  ② **定时器首跳不用睡 10 分钟**：把间隔做成参数（`spawn_daily_maintenance(state, first_delay, period)`），
  生产传 `(10min, 24h)`，测试传 `(10ms, 20ms)` 并用 `tokio::time::pause()` 证明跳数与顺序。
  别直接断言"第一跳在 10 分钟" —— 那要真的等，测试会被人删掉。
  ③ 日志目录造 40 个假 `yekitrim-*.log`，跑一次，剩 30 个。
- **破了什么**：**删数据**。所以三个 purge 都必须"只删超过保留期的"，并且**第一次上线时打 info 说明删了多少**。
  审计表是安全审计用途 —— 删它之前先确认 `docs/deploy.md` 里没有"审计需长期保留"的承诺（我没找到，但请你确认）。

---

## P1 门禁执行化

### T7 `route_manifest` 接上"契约差集"

- **改哪里**：`scripts/route_manifest.py` 目前只 dump（`exit 0` 恒真，输出是
  `# sources:` 注释 + `## routes (sorted)` + 每行 `METHOD  path  -> handler`，共 168 行）。**不要改它的语义** ——
  新建 `scripts/check-api-contract.mjs`（node 侧更好写 JS 正则），它：
  1. 调 `python3 scripts/route_manifest.py --out <tmp>` 拿 `(method, path)` 清单（现成能力）；
  2. 扫 `webui/src/api/*.js` 的 `/api/...` 字面量（现状：**57 条单引号 + 53 条模板串**）。
     模板串要**先剥查询串再归一化**：`` `/api/polish/project?${qs(…)}` `` → `/api/polish/project`，
     `` `/api/admin/users/${encodeURIComponent(username)}/quota` `` → `/api/admin/users/:x/quota`
     （`:x` 匹配路由里的任意 `:param` 段）。这一步是本任务唯一有技术含量的部分，别用"前缀匹配"糊过去。
  3. **双向差集**：前端调了不存在的路由 = **红**；后端注册了前端从不调 = **提示**（不红）。
- **白名单**：SSE（`/api/stream`、`/api/jobs/:id/events`、`/api/admin/logs/events`）、
  `/api/media/*`、`/api/health`、静态资源必须白名单，**每条都要注释写理由**。
- **怎么算做完**：`node scripts/check-api-contract.mjs` 在当前树**通过或只剩已登记的白名单项**。
- **怎么验证**：① 在前端 api 层手写一条 `/api/does-not-exist` → 必须红；
  ② 在 `routes.rs` 注释掉一条 route → 必须红；③ 变异自证：把差集判断 `!==` 改成 `===`，必须翻面。
- **破了什么**：无。**已知噪音**：46 个 handler 收 `Json<serde_json::Value>`，字段名错**抓不到** ——
  这条只查路径，不查字段（字段靠 T9）。

### T8 `cargo xtask gate` 还是独立脚本？

**结论：保持独立 `scripts/gate.mjs`，不进 xtask。**
理由（`crates/xtask/AGENTS.md` 明写）：xtask 刻意只依赖 std + sha2 + zip，不调 python/node。

- **本条任务内容**：在 `scripts/gate.mjs` 里加一个 `--check-structure` 子动作，
  把两条 python 门禁包进去，让"一次改动跑完全部相关守卫"真的成立（现在根卡的「验证」节只列了两条）。
- **怎么验证**：`node scripts/gate.mjs --changed` 在只改前端时必须**不跑** Rust 测试，
  且输出里明确写 `SKIP cargo test -p yekitrim-server（未触及 crates/）`。

---

## P2 契约与正确性

### T9 请求体改 `Deserialize` 结构（46 处，分批）

- **改哪里**：`crates/yekitrim-server/src/api/*.rs` 里 `Json(body): Json<serde_json::Value>` 的 46 个 handler。
  **不要一次全改**。分批：
  1. 第一批（10 个）：写操作且字段少的 —— `polish/init`、`polish/process`、`polish/render`、
     `polish/cover`、`project-data/events:post`、`events/:rank/cut`、`clips/:rank/title`、
     `projects:post`、`rooms:post`、`ingest/retry`。
  2. 第二批：读接口与带 `Option` 字段的。
  3. **`polish_project_put`（`api/polish.rs:2118`，268 行）留到最后**，它是 EDL 深合并，
     结构体化会改动合并语义 —— 单独一个任务评估。
- **关键约束**：字段名一旦结构体化就**成了契约**，前端拼错的字段会从"静默 null"变成 **400**。
  这正是目的，但必须**同批**改前端（否则用户看到 400）。所以每批的验收都包含"前端对应调用点已核对"。
- **怎么算做完**：第一批 10 处完成；`cargo test -p yekitrim-server` 全绿；前端对应页面手工点一遍。
- **怎么验证**：① 单测：给每个新结构体一条"缺字段 / 字段类型错 → 400 且 error 说明哪个字段"；
  ② 变异自证：把某字段名改错一个字母，测试必须红。
- **破了什么**：**这是行为改动**。宽松解析（多余字段）要保持（`#[serde(default)]` + 不 `deny_unknown_fields`），
  否则老前端会突然 400。

### T10 路由 401 覆盖改枚举式

- **现状**：`routes.rs:913` 的 `unauthenticated_requests_are_401_except_whitelist` 手写了 **33 条**路径，
  而注册了 **127 条**。新加一条路由忘了挂 `protected`，**没有任何测试会红**。
- **改哪里（推荐方案 B，因为它让"漏配 = 401"而不是"漏配 = 公开"）**：
  在 `routes.rs` 里 `public.merge(admin).merge(protected)` **之后**加一层兜底中间件
  `require_authenticated_or_whitelist`：
  ```
  若 req.extensions() 已有 CurrentUser → 放行（admin / protected 两层已经写过了）
  否则若 (method, path) 在 PUBLIC 表里 → 放行
  否则 → 401 JSON（与 authenticate 的错误体同形）
  ```
  `PUBLIC` 表集中一处（`/health`、`/auth/login`、`/auth/register`、
  `/ingest/bililive/:token`、`/ingest/webhook/:token`、`/shutdown`），每条**写明理由**。
  ⚠️ 该层必须挂在 `compress_json` **之前**、`access_log` **之后**
  （否则被 401 的请求在日志里看不到 body，而 access_log 对 ≥400 会读 body）。
  ⚠️ 它**不替代** `require_user` / `require_admin` —— 那两层仍负责 CSRF 与角色；
  这一层只兜"完全没挂任何认证层"的漏网路由。
- **不推荐方案 A**（`include_str!("routes.rs")` 在 Rust 里重写正则）：两份正则必然漂。
- **怎么算做完**：删掉手写的 33 条清单（白名单表取代它）；新增一条故意不带 `require_user`
  的测试路由，未登录访问必须 401、带会话必须 200。
- **怎么验证**：① 新增路由的正向/反向测试各一条；② 现有 159 个 api 测试仍绿
  （尤其 `write_requests_require_matching_csrf` 与 `unauthenticated_requests_are_401_except_whitelist`
  要改写成"扫白名单表"）；③ **变异自证**：把兜底层去掉，新路由的测试必须红。
- **破了什么**：**这是安全相关的行为改动**。漏一个白名单 = 登录页打不开，所以白名单表
  **必须有测试逐条访问一遍**（未登录 → 200/非 401），而不是只靠人看。

### T11 `projects_list` 的 N+1（`owner_name` 每项目一次全表扫）

- **改哪里**：`api/projects.rs:86` 的 `owner_name(db, p.owner_id)` 在闭包里逐项目调用，
  而 `owner_name` 内部是 `db.list_users()`（全表）。**改成**：进闭包前取一次 `list_users()`，
  建成 `HashMap<i64, String>` 传进去。
- **怎么算做完**：`/api/projects` 的 DB 查询次数从 1+N 降到 2。
- **怎么验证**：① 单测：造 10 个项目、3 个用户，断言返回的 owner 名正确（功能不变）；
  ② 计次：`Db` 上加一个 `#[cfg(test)]` 的计数器，或直接在测试里 decode SQL 日志 —— **推荐前者**，
  加 `pub(crate) fn query_count()` 只在 test 编译。
- **破了什么**：无。

### T12 `cover-takumi` feature 归属（10-15 的 S3）

- **现状**：`yekitrim-polish` 的 `default = []`，`cover-takumi` 只在 `yekitrim-cli` 的
  `default = ["cover-takumi"]` 里转发；`yekitrim-server` / `yekitrim-service` 对 polish 是
  **无 feature 的路径依赖**，且 `server/Cargo.toml` 的 `[features]` 块是**空的**。
  ⇒ `render/takumi.rs`（1,092 行）与 `tests/takumi_font_id.rs` 在 `cargo test -p yekitrim-server`
  与 `cargo check --workspace --all-targets` 下**整份不参与编译**，而它正是 WebUI「生成封面」的后端
  ⇒ `/api/health.cover_static.compiled` 在 server 语境**恒 false**。
- **改哪里（推荐选项 2）**：
  ```toml
  # crates/yekitrim-server/Cargo.toml
  [features]
  cover-takumi = ["yekitrim-polish/cover-takumi"]
  ```
  并在 server 侧新增一个 `--features cover-takumi` 的 **check-only** 岗位（见验收），
  让"server 语境能不能编到 takumi"这件事有判据。
- **为什么不选"把 takumi 设成 polish 默认"**：`cli/Cargo.toml` 记着不这么做的三条理由
  （拉长构建、Windows 上 `libwebp-sys` 的 `LNK4098`、发行产物许可面要重核）。
- **怎么算做完**：`cargo check -p yekitrim-server --features cover-takumi` 通过；
  `cargo test -p yekitrim-polish --features cover-takumi --test takumi_font_id` 能跑。
- **怎么验证**：① 上面两条命令；② `/api/health` 的 `cover_static.compiled` 在带 feature 的构建里为 true
  （加一条只在 `#[cfg(feature = "cover-takumi")]` 下编译的测试）；③ **判据守卫**：
  在 `gate.mjs` 里加一行"发布构建必须带 cover-takumi，否则红"（对应 `xtask package`）。
- **破了什么**：构建时间与链接告警（只在带 feature 时）。**默认构建仍不带**，所以不影响日常开发。

---

## P3 重复真相收敛（用户已给口径：**允许 DOM 近似，只保语义**）

### T13 定边界：哪些 parity 判据保留、哪些降级

**这是 P3 唯一需要拍板的任务，文档化在 `crates/yekitrim-polish/AGENTS.md`（P3 里唯一改卡的任务）。**

我已按脚本内容分好类（每条脚本引用了哪些实现，是 grep 出来的）：

| 脚本 | 守着的一对 | 新口径下的处置 |
|---|---|---|
| `check-camera-zoom-parity` | 5 处：`engine.js` / 转译器 / `index.html` / `types.rs` | **保留**（数学表；与 DOM 无关） |
| `check-anim-presets-parity` | UI 清单 ↔ 转译器清单 | **保留**（清单一致性，不是像素） |
| `check-karaoke-parity` | `engine.js` ↔ 转译器 | **保留**（逐字时刻是语义） |
| `check-perchar-parity` | `engine.js` ↔ 转译器 | **保留** |
| `check-text-motion-parity` | 四处 | **保留**（时间/位移数值） |
| `check-sway-parity` | `engine.js` ↔ 转译器 | **保留** |
| `check-sfx-fade-parity` | 两腿 | **保留**（淡入淡出是听感，不是像素） |
| `check-stroke-outward-parity` | `PolishPreview.vue` ↔ 转译器 | **保留**（描边宽度定义，语义） |
| `check-grid-parity` | `engine.js` / Rust / 模板 / 转译器 | **保留**（九宫格是落点语义） |
| `check-element-pos-parity` | `PolishPreview.vue` ↔ `engine.js` | **保留**（同一个 bug 修了四轮） |
| `check-dom-camera-parity` | DOM 腿 ↔ 底座求值器 | **降级为"容差 ±1px"**（唯一的纯像素项） |
| `check-noise-parity` | `PolishPreview.vue` / `engine.js` / `index.html` | **降级**：只验"开启/强度值透传"，不验噪点图案一致 |
| `check-color-parity`（webui） | `index.html` / `types.rs` | **保留** |

**注意**：13 个里只有 **2 个**真的该降级。所以"重复真相税"不是靠删脚本解决的 ——
**它的真实收益在 T14**（删掉第三套渲染实现本身），而不是放宽判据。
这一点必须先说清楚，否则会误以为"口径一松就省一半"。

**另一个必须先算清的事实**：`cmd_render`（真实出片）走的是 `polish-to-dhampir.mjs`，
**根本不读 `templates/index.html`**（`pipeline.rs` 里对它的两处提及都是**注释**，不是读文件）。
只有两处 `include_str!("../../templates/index.html")`：
`render/server.rs:47`（预览服务）与 `project/config.rs:4267`（一条"模板必须消费 `_canvas` 契约"的测试）。
也就是说 `index.html` 今天**只服务预览 + 一条契约测试**，不是"出片权威路径" ——
`timeline.rs:721` 那句"出片权威路径"是旧口径，T15 要一并改掉。
这决定了 T14 第 3 步的风险比看上去低得多：**删它不影响任何成片**。

### T14 删掉 `templates/index.html` 的旧出片/预览腿（分三步，**每一步可独立回滚**）

**前置事实（已核实）**：
- `RenderServer`（`render/server.rs:544` 用 `include_str!(index.html)`）现在只服务
  `polish preview` CLI 的兜底 + 服务端 `/polish/preview`、`/polish/cover-preview`。
- `/polish/preview` 的前端调用点 `ProjectClipsSection.vue:1446 preview()` **没有任何模板/ expose 引用**
  —— 是死代码（`defineExpose` 只暴露了 `redetect/openAdd/cutAll`）。
- `/polish/cover-preview` 的 JS 封装 `coverPreview` 在 `webui/src/api/polish.js:205` **无调用方**。
- `polish preview` CLI 在**找不到 `yekitrim.toml`** 时才回退到旧 `cmd_preview`（`cli/main.rs:159`）。

- **第 1 步（清死代码，零风险）**：删 `ProjectClipsSection.vue` 的 `preview()` 与
  `api/polish.js` 的 `coverPreview`（连同 `webui/src/api/AGENTS.md` 里的那一行）；
  服务端两条路由**保留**（CLI `polish preview` 还要走），但加注释"仅 CLI 兜底"。
- **第 2 步（收敛 CLI 兜底）**：`cli/main.rs::run_polish_preview` 找不到 `yekitrim.toml` 时，
  改成**打印指引并退出**（"先 `yekitrim init` 或在 WebUI 里导入"），不再 `cmd_preview`。
  这会让 `cmd_preview` 只剩 **`commands::cmd_test`（`polish test` 自检的最后一步）** 一个调用方 ——
  `cmd_preview_cover` 则**已经无调用方**（grep 只有定义与文档行）→ 直接删。
  `cmd_preview` 保留（给 `polish test` 用），但它的文档要写清"只剩自检用"。
- **第 3 步（删模板）**：上面两步稳定一个版本后，删 `templates/index.html`、
  `render/server.rs` 的 `INDEX_HTML` 与只服务它的代码路径，以及 `project/config.rs:4267`
  那条**拿它当输入**的契约测试（`template_stage_hooks_match_contract`）——
  把它改成断言 `engine.js` + `timeline.rs` 两个消费面（这才是以后的真相）。
  再删 T13 表里被标"降级"的两条中引用 `index.html` 的部分。
  **`templates/cover.html` 保留**（封面是另一条链路）。
- **怎么验证（每步）**：① `cargo check --workspace --all-targets` 0 warning；
  ② `cargo test -p yekitrim-polish` + `cargo test -p yekitrim-server` 全绿；
  ③ `node scripts/gate.mjs --all` 与基线比**不新增红**；
  ④ 手工：`yekitrim polish preview <clip>` 在**有** `yekitrim.toml` 时仍正常打开 WebUI 精修页。
- **破了什么**：**用户可见的行为**（`polish preview` 在"独立 clip 目录、没有工程"时不再起服务）。
  这就是为什么必须分三步、且第 2 步要单独出一个版本。

### T15 `engine.js` 的定位写清楚（不删，但降级为"近似"）

- **改哪里**：`webui/src/polish/engine.js` 文件头（现在写的是"忠实移植"）+ `webui/src/polish/AGENTS.md`。
  改成明确口径：**DOM 腿是近似预览，用于无 WebGPU 的机器；数值以转译器/timeline 为准，
  允许视觉上的细节差异（噪点、部分特效强度）**。
- **怎么算做完**：文件头 + 卡里都写着这句；`PolishPreview.vue` 里 DOM 腿的提示文案
  （现在只有 Dhampir 失败时才有 `近似预览 · …`）在**没开 Dhampir** 时也给一个弱提示
  （例如 hover 才显示），避免用户拿它当成片。
- **怎么验证**：`npm --prefix webui run build` 成功；`node webui/scripts/check-*.mjs` 不新增红。
- **破了什么**：文案。**注意**：`check-preview-ab.mjs` 有一条"在 catch 头一段找 `dhampirMode.value = false`"
  的脆弱判据（判据窗口 1200 字符）—— 动这附近的文案会被它误伤，改完必须跑它。

---

## P4 结构拆分

### T16 拆 `api/polish.rs`（2,787 产线行 / 39 个 handler）

**必须 T3 先做完**（否则新目录要开卡，指令链超预算）。

- **切法（已按函数逐个量过，直接给分组）**：
  ```
  src/api/polish/
    mod.rs        共享底座（DEFAULT_RENDER_STEM / render_stems / numbered_family /
                  artifact_files_of / scope_pid；`validated_project_dir` **不是**这里的，
                  它定义在 `api/project_data.rs:1436` —— 拆完要么下沉到 `api/mod.rs`，
                  要么由 project_data 显式 `pub(crate)` 导出，二选一并写注释）
    program.rs    polish_spawn / polish_init / polish_llm / polish_llm_segment /
                  polish_process / polish_render        （6 个，~230 L）
    project.rs    polish_project_get(_impl) / polish_project_put(_locked) /
                  polish_subtitles_*(_impl) / polish_emotion_get(_impl)
                  + read_data_json / loudness_curve_cached   （~700 L）
    cover.rs      cover_title_* 那 15 个纯函数 + polish_cover_title / polish_cover_config /
                  polish_cover_background / polish_cover_frame / polish_silence /
                  polish_select_version / set_bg_image_mode   （~800 L）
    fonts.rs      polish_fonts* (6 个) / polish_font / font_import_allowed /
                  font_import_roots / font_content_type       （~400 L）
    assets.rs     polish_sticker_* / polish_emoji_catalog / polish_video_library /
                  polish_sfx_catalog / polish_asset_file / asset_kind_root /
                  polish_video_thumb / make_video_thumb        （~450 L）
    preview.rs    preview_port_for / polish_preview / polish_cover_preview （~120 L）
  ```
  然后把 Dhampir 那一段（`polish_dhampir_*` + 转译缓存，~700 L）单独成 `dhampir.rs`。
- **同时做**：`api/polish.rs:21` 与 `api/project_data.rs:21` 的 `use super::{…::*}` **全部改显式**。
  （现在两个文件互相 glob 引用，Rust 允许模块环、门禁抓不到 —— 这是"按领域一文件"失效的根因。）
- **怎么算做完**：`wc -l src/api/polish/*.rs` 每个文件 ≤ 900 产线行；`api/polish.rs` 与
  `api/project_data.rs` 之间**没有** `use …::*`；`routes.rs` 只改 `use super::polish::*` 这一行。
- **怎么验证**：① `cargo check --workspace --all-targets`；② `cargo test -p yekitrim-server` 全绿
  （api 有 159 个测试，这就是回归网）；③ `python3 scripts/check_module_map.py` 通过 ——
  **前提是 T3 已经把预算腾出来了**，且 `docs/module-map.json` 里新增 7 个 `kind=module` 条目、
  根索引表同步加 7 行（门禁会逐条核对，漏一条就红）。
  ⚠️ 新增 7 张卡会让"最坏链"从 `root + server + api` 变成 `root + server + api + api-polish`，
  所以 T3 的目标是 **≤ 50,000 B**（留 7 KB 给这 7 张卡 —— 每张按 300~600 B 的空卡算）。
- **破了什么**：纯搬运。**唯一风险**是 `pub(super)` 的可见性 —— 拆成子模块后
  `api::polish::fonts` 要访问 `api::polish::mod` 里的 helper，`pub(super)` 语义会变
  （`super` 从 `api` 变成 `api::polish`）。逐个改 `pub(crate)` 或 `pub(in crate::api::polish)`。

### T17 拆 `ingest.rs`（3,235 行，两条 worker 语义汇合 —— 10-15 的 W1）

- **现状**：`ingest.rs` 引 `crate::ingest_worker::` 11 次、`crate::download_worker::` 4 次。
  门禁规则只管"两个 worker 不许互相 import"，于是它成了**合法绕过通道**：
  录播姬的待分配池语义与 B站的"提交即下载"语义住在同一个文件里（`handle_event:163` 与
  `bilibili_submit:2210` 相隔 2000 行）。**禁令想防的事已经发生了。**
- **切法（按现有函数清单）**：
  ```
  src/ingest/
    mod.rs            err/ok/field* 共享解析 + 再导出
    webhook.rs        handle_event / authorize_webhook / webhook / custom_webhook /
                      parse_webhook_event / resolve_webhook_completion / map_webhook_path /
                      prepare_webhook_project / normalize_* / push_event_sep / sanitize_title
    records.rs        records / recording_state / owned_room / room_recordings / import_recording /
                      session_json / state_label
    channel_map.rs    channel_map_get/put / assign / retry / scan_now / first_admin
    config.rs         config_view* / build_ingest_patch / put_config / config_patch /
                      write_global_config* / audit_keys / rotate_token / check_projects_root /
                      projects_root_gate / local_lan_ip
    bilibili.rs       bili_status / bili_err / check_quality / bilibili_resolve / bilibili_submit
    downloader.rs     downloader_install / downloader_status
  ```
- **同时做（比拆文件更重要）**：在 `check_module_boundaries.py` 的 `PAIRED_RULES` 里加一条
  **成对判据**：`ingest/webhook.rs` 与 `ingest/bilibili.rs` **不许出现对方的领域词**
  （webhook.rs 里不许有 `bilibili|bvid|yt-dlp`；bilibili.rs 里不许有 `recorder|room_id|待分配`）。
  理由：门禁要防的是"语义汇合"，不只是"import 汇合"。
- **怎么算做完**：7 个文件每个 ≤ 600 行；新增的成对判据 0 违规。
- **怎么验证**：① `cargo test -p yekitrim-server` 全绿（`ingest` 相关测试必须全部保留）；
  ② `python3 scripts/check_module_boundaries.py` 通过（**含新判据**）；
  ③ **变异自证**：在 `webhook.rs` 里写一句含 `bvid` 的注释外的代码，判据必须红。
- **破了什么**：纯搬运 + 一条新门禁。**风险已排除**：`build_ingest_patch`（335 行，`:1565`）
  的**唯一生产调用方**是 `put_config`（`:1941`），其余 20 多处全在 `#[cfg(test)]` 里 ——
  搬进 `ingest/config.rs` 时把它的测试一起搬即可，不用跨文件引用。

### T18 拆 `api/project_data.rs`（1,922 产线行 / 31 handler）

- **切法**：`project_data/{status,events,clips,assets,fs,media}.rs`，
  `fs_list`（153 行）与 `media_get`（47 行）单独成 `fs.rs` / `media.rs`。
- **⚠️ 顺序约束**：`project_data` 与 `polish` 现在**互相 glob import**（各有一行
  `use super::{…::*}`）。所以 **T16 与 T18 必须同一个批次做**，中间不能只做一半 ——
  只拆一边，另一边的 `use super::polish::*` 会指向新目录的 `mod.rs`，而 `mod.rs` 里
  如果只再做一层 `pub use`，环还在、只是换了个位置。
- **怎么算做完 / 验证**：同 T16，外加硬判据：`grep -c '::\*}' src/api/*.rs` = 0。
- **破了什么**：同 T16 的 `pub(super)` 可见性。

### T19 路径判据三份 → 一份（10-15 的 W4）

- **现状三份**：`crates/yekitrim-core/src/tenant.rs::ensure_within`、
  `crates/yekitrim-server/src/scope.rs::is_safe_project_relative`、
  `crates/yekitrim-server/src/api/polish.rs` 的 handler 侧兜底（注释写明"两层缺一不可"）。
- ⚠️ **本条我建议不做，或只做"登记"**。理由：根卡明确写了"路径面三防线，三处缺一不可"，
  而且 `/etc/passwd` 那条洞就是"只有一份实现、藏在 handler 里没有第二道"留下的。
  **这些是纵深防御，不是重复。**
- **折中方案（推荐）**：把三处的**判据函数**收敛到 `core::tenant` 一个实现，但**保留三个调用点**。
  这样"判据只有一份"（不会漂）而"防线仍是三道"（不会一次绕过全破）。
- **怎么验证**：① 现有 `polish_path_fields_cannot_read_outside_files` + `polish_dir_must_be_inside_own_project`
  两条测试必须绿；② 加一条变异测试：把判据函数改松，两条测试都必须红。

---

## P5 收尾

### T20 构建与依赖卫生

- **改哪里**：
  1. `webui/` 两份锁（`package-lock.json` + `pnpm-lock.yaml`）**删一份**。
     推荐**留 `package-lock.json`**（根卡所有命令都是 `npm --prefix webui …`，pnpm 没有任何调用方）。
  2. workspace 加 `rust-version`（按你实际用的稳定版写，比如 `"1.82"`），
     并统一 edition（现在 6×2021 + 2×2024）。
     ⚠️ **edition 2024 会强制 `std::env::set_var` 为 unsafe** —— 而 `cli/main.rs:538/541`
     正是裸 `set_var`。所以**统一 edition 之前先做 T21**。
  3. 加 `clippy.toml`（可选）+ `cargo clippy --workspace --all-targets -- -D warnings` 进 `gate.mjs`。
- **怎么验证**：`cargo clippy --workspace --all-targets -- -D warnings` 通过（首次可能要清一批）。

### T21 `VTEDIT_RENDERER` / `VTEDIT_STRICT` 环境变量 → 参数

- **改哪里**：`cli/main.rs:538/541` 写、`polish/render/{backend,pipeline}.rs:372/456` 读。
  改成 `cmd_render(input, RenderOpts { renderer, strict })` 显式传参，
  环境变量**只保留为 CLI 入口的默认值来源**（向后兼容老脚本）。
- **怎么验证**：① 现有渲染测试；② 新增一条"两个并发 `RenderOpts` 不同 renderer 互不影响"的单测
  （现在做不到，因为读的是进程级 env —— 这正是要修的点）。
- **破了什么**：CLI 的 `--renderer` 语义不变（仍从参数来）；直接设环境变量调库的行为会保留，
  但**不再是唯一路径**。

### T22 进程级全局的可见化

- **改哪里**：
  1. `yekitrim-llm/src/usage.rs` 的 `DEFAULT_SINK` 与 `core/owner.rs` 的 `LOCAL_ADMIN_UID`：
     **不改架构**（改动面太大），只在 server 启动与 CLI 入口各加一条**启动日志**：
     `usage sink: installed` / `local admin uid = N`。
  2. 顺带在 `yekitrim doctor` 里也打这两项。
- **为什么**：这两处的失败模式是"静默降级"（漏装 = 全部 token 不记账且不报错）。
  让它在启动日志里可见，就够挡住 90% 的踩坑。
- **怎么验证**：启动一次 server，日志里必须有这两行；`yekitrim doctor` 输出里也必须有。

### T23 决策登记（不写代码）

把本轮**未做但已定口径**的项写进 `ROADMAP.md`（不要散在 docs/plan）：

- **D-1**：DOM 近似预览**允许语义接近**（用户 2026-11-24 拍板）→ T13/T15 的依据。
- **D-2**：`cover-takumi` 默认仍在 CLI 侧（T12 只加 server 的 feature 转发）。
- **D-3**：CI **暂不做**（用户 2026-11-24 拍板：先本地 gate）。前置条件写在 `ROADMAP.md`：
  `vendor/sherpa-onnx-sys` 的离线归档 + `webui/dist` 入库已具备（✅），
  缺的是"能不能在没有 Windows 的 runner 上跑通 ASR 测试"—— 那是下一步再评估的事。
- **D-4**：`core/config/config.rs::merge()`（130 行手写）**暂不改**（10-15 的 C5）——
  它现在是对的，只是容易漏改；已有五步协议 + 门禁兜底。

---

## 附：第一周执行顺序（建议）

| 天 | 做什么 | 产出 |
|---|---|---|
| 第 1 天上午 | T1 + T2 + T3 | 仓库干净、`git status` 只剩个位数、两张卡瘦身后门禁打印最坏链 ≤ 50 KB |
| 第 1 天下午 | T4 + T6 的 DB 三个 purge 函数（不带定时器） | 3 处 blocking + 3 条 purge 单测 |
| 第 2 天 | T6 的定时器 + 配置项（走 cfg 五步） | 启动日志出现 daily maintenance 一行 |
| 第 3–4 天 | T5 `gate.mjs` + `gate-baseline.json` | `node scripts/gate.mjs --all` 能跑完并给出汇总 |
| 第 5 天 | T7 `check-api-contract.mjs` | 差集为 0 或只剩登记项 |

第 2 周起进入 P2（T9 第一批 10 个 handler 是整轮里最划算的一块：一次改动就堵住
"字段名拼错变 null"这一整类缺陷）。

**每个任务一个提交**，提交说明里带任务号（`T4: …`），方便回滚到任意一项。

---

## 附：验收清单（整轮做完时逐条打勾）

```bash
# 结构
python3 scripts/check_module_map.py          # 35+ 模块，最坏链 ≤ 50000
python3 scripts/check_module_boundaries.py   # 19+1 条（含 ingest 语义成对判据）
node scripts/gate.mjs --all                  # 非基线失败 = 0
node scripts/check-api-contract.mjs           # 0 未解释差异

# Rust
cargo check --workspace --all-targets         # 0 warning
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo check -p yekitrim-server --features cover-takumi

# 前端
npm --prefix webui run build                  # dist 指纹刷新
node webui/scripts/check-*.mjs                # 17 条

# 行为（手工，每轮发版前）
yekitrim polish preview <有 yekitrim.toml 的 clip>     # 仍能打开精修页
登录 → 建项目 → 转写 → 检测 → 切 → 出片 → 封面           # 主流程
```

## 附：本计划**没有**包含的项（如实）

- **CI 落地**（用户口径：先本地）。
- **`core/detection → core/cutting` 的横边**（10-15 的 C1/C2）：它现在只 2 处调用、且是"可选副作用"
  （失败只 warn）。修它要动 `EventDetector::run` 的职责边界，收益低于 P0–P4，**建议下一轮**。
- **`config::merge()` 重写**（C5）、**`detection/mod.rs::run()` 的 283 行拆分**（C6）：
  前者现在是对的、后者职责顺序有硬约束。**都建议先取证再动**，不在本轮。
- **`server` 直接 `use yekitrim_core::` 4 处**：core 是共享底座，不是违规，**不动**。
- **前端迁 TS**：收益主要在"类型单一来源 + 自动跑检查"，而后者正是 T5/T7/T9 在做的。
  先把契约与门禁做出来，再评估 TS —— 顺序反了会花大钱买小收益。
