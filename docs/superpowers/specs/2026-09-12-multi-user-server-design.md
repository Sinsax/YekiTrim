# V-Trim 多用户服务（远期目标，暂不排期）

| 项 | 值 |
|---|---|
| 状态 | **远期目标 / 未开工**——先把基础功能（当前单用户 `vtrim server` + CLI 工作流）做完再回到这里 |
| 性质 | 设计稿（改造方案 + 决策记录 + 里程碑），不是排期承诺 |
| 日期 | 2026-09-12 |
| 目标部署 | 单机自托管，≤10 人，一个 `vtrim server` 进程 |
| 相关 | `docs/superpowers/specs/2026-09-03-vtrim-webui-design.md`、`AGENTS.md`（WebUI / Server 一节） |

> 阅读方式：§0 是已经拍板的决策与仍待拍板的默认值；§1–§12 是完整方案；
> §13 说明它与当前已实现加固的衔接。开工前只需把 §0 的 6 个默认过一遍。

---

## 0. 决策记录

### 0.1 已确认（用户）
| 项 | 决策 |
|---|---|
| 部署/形态 | 单机自托管；**直接改造 `vtrim server`**（本机模式＝只有 1 个账号的部署），不新增并行的服务实现 |
| 算力/密钥 | 平台统一：共享 LLM key、共享 ASR 模型缓存、全局并发上限；用户只管用 |
| 认证 | 账号 + 密码；**项目必须按账户隔离** |
| 部署体验 | **部署即生成管理员账号与初始密码**（首次启动输出到日志 + `<data_dir>/initial-admin-password.txt`，首次登录后删除）；**单人/多人共用同一套认证逻辑，无免登录**；对外监听要求已改初始密码 |
| 输入模型 | **不接收任何用户上传/本地文件**；服务只处理"服务器上已存在或自己下载的"视频 |
| "订阅录播服务" | **对接录播姬（BililiveRecorder）这类本地录制工具**：录制完成即推送 → 自动变项目 |
| 入口补充 | 选择 B站视频 → 转项目（服务器侧下载，次要通道） |
| 源录播生命周期 | **切片完成后不释放**；只靠"最大硬盘空间"约束，不做自动删源 |

### 0.2 仍待拍板的默认（不表态则按此执行）
1. **前提假设**：录播姬与 `vtrim server` 同机（或录播姬输出目录位于服务器可读的盘/共享盘）——因为其 webhook 只给 `RelativePath`，不推送文件字节。
2. **多分片会话默认合并**为单个 `source.mp4`（`ffmpeg -f concat -c copy`，失败回退重编码），原始分片保留；因管线 `input_path` 只支持单文件。
3. **默认移入托管项目目录**（同盘 `rename` 瞬时；跨盘才 `copy+校验+删源`，先做水位预检）；`projects_root` 默认与 `recorder_root` **同盘**。保留录播姬文件管理功能可设 `move_into_managed=false`。
4. **弹幕由服务端解析录播姬 XML**（仓库现在只吃 ASS/SRT），部署方零额外配置，不依赖 DanmakuFactory。
5. **封面背景图导入**是"不接受本地文件"的**唯一例外**（≤32MB 图片，落项目目录）。
6. **未映射房间**进入"待分配池"（仅 admin 可见并指派），不静默丢数据。
7. **单人/本机部署是否保留"服务器端已有文件"作为项目来源**：工作台 v5 本期的新建向导只有「本地文件浏览」（浏览的是**服务器**目录），而本稿 §5.2 把该入口整体删掉了 → 结果是 P2 之后单人用户无法通过 UI 把"已经躺在服务器上的录像"建成项目（只能用 CLI 或等录播姬）。
   建议：保留但收得很紧——`source.kind="local"`（**admin-only**）+ `/api/fs/list` 囚笼到该账号的项目根/租户根，仍**永不接受上传**。这样既守住"不收用户文件"的合规边界，也不丢单机工作流。**若你选"不保留"**，则单人用户添加已有录像的唯一途径是 CLI，前端新建向导只留「入库记录 / B站链接」。

---

## 1. 目标与成功标准

1. **零文件搬运**：用户只做"配一次 webhook + 填房间映射"，之后每场直播录完自动出现项目、自动跑管线。
2. **严格租户隔离**：A 无法读写 B 的项目/产物/任务/事件/配置；用户只提供 id 与项目内相对路径。
3. **容量可控**：全局 + 每用户磁盘上限、入库/下载前水位预检、超限明确拒绝；不自动删用户数据。
4. **一次登录、长期可用**：部署时即生成管理员与初始密码，登录后会话长期有效（7 天滑动 / 可记住 30 天）；CLI（`vtrim extract/process/polish ...`）行为不变。
5. **平台密钥不外泄**、**任意路径面归零**。
6. 验收可执行：§9 清单全部可跑（webhook / 下载器均用 fixture + mock，不打真实网络）。

## 2. 现状冲突点与改造

| 模块 | 现状（已核对） | 改造 |
|---|---|---|
| 认证/会话/DB | 全仓库无相关依赖（无 sqlx/rusqlite/jwt/argon2/oauth） | 新增 SQLite（users/sessions/projects/ingest_*/audit）+ argon2 |
| `AppState.workspace` | 进程内唯一"当前工程" | 删除；按 `project_id` 每请求解析 |
| `JobManager` | 全局单队列 + 有活动任务即 409；无归属 | 任务加 `owner_id`；**入库/下载槽**与**重活槽**分离；每用户串行 + 全局并发 + FIFO 排队 |
| `/api/stream`、`/api/jobs` | 全局广播/列表 | 按 owner 过滤；他人资源一律 404 |
| 工程列表 | `.vtrim/server.toml` 全局 `projects[]` | 迁入 `projects` 表；旧文件只读导入 admin |
| 任意路径入口 | `?config=<路径>`、`dir`、`input`、`config_path`、`output_dir`、`output_file`、`source`、`/api/fs/list` | **整套删除**，改为 `?project=<id>` + 项目内相对路径（§5.2） |
| 媒体流出 | `/api/media/*` 白名单来自全局工程列表（跨租户） | 只服务"本人项目的产物 + 该项目源录播" |
| 弹幕解析 | 只支持 **ASS**（`parse_ass`）与 **SRT**（`parse_srt_danmaku`），`load_danmaku_auto` 按扩展名分派；**无 XML** | 新增录播姬 XML 弹幕解析（§4.4） |
| 输入抽象 | `PipelineContext.input_path: PathBuf`（单文件） | 不变；多分片会话先合并（§4.3） |

## 3. 身份与授权

- 表：`users(id, username UNIQUE, password_hash argon2id NOT NULL, must_change_password, role admin|user, quota_bytes, disabled, created_at, last_login_at)`；`sessions(id 32B, user_id, csrf, expires_at, user_agent)`。
- **账号始终有密码**（`NOT NULL`）：不存在"无密码账号"，也不存在任何形式的免登录。
- Cookie `vtrim_sid`：`HttpOnly; SameSite=Lax; Path=/`（TLS 下 `Secure`），TTL 7 天滑动续期，登出删行；可选"记住此设备"（30 天）；支持"登出全部设备"。
- CSRF：非 HttpOnly 的 `vtrim_csrf` cookie + 所有写操作必带 `X-CSRF-Token`（前端 `api()` 统一注入一处）。
- 中间件 `require_user` / `require_admin`；白名单仅 `/api/auth/login`、`/api/health`、`/api/ingest/bililive/<token>`（自带 token 鉴权）、静态资源。**没有"本机例外"分支**。
- 登录限流（username+IP 滑窗 5 次/15 分钟）+ 可选弱密码拦截；校验失败一律拒绝，绝不接受空密码/未设置密码。
- CLI 管理入口：`vtrim user add|list|passwd|disable`、`vtrim user reset-admin-password`（救援，见 §3.5.5）。

## 3.5 部署即生成管理员账号：单人/多人同一套认证逻辑

> 设计不变式：**认证路径与"单人还是多人"完全无关**——所有请求（含 `127.0.0.1`）都必须带会话。
> 单人部署与多人部署的差别只剩"账号数量 / 是否显示用户管理 / 配额与并发默认值"，认证逻辑一行都不分叉。

### 3.5.1 为什么不再有免登录（决策记录）
- 曾被考虑过的方案是"loopback 请求视为管理员、免登录"，它的代价是 **本机任意进程/任意用户都等于管理员**：对"这台机器只有我"成立，对"多人共用一台服务器（多人 SSH）"就是越权通道。
- 同时它需要判定来源地址，带来一串经典坑：反代后 `X-Forwarded-For` 不可信、`127.0.0.1` / `::1` / `localhost` / IPv6 映射写法差异、容器网络下的来源语义。
- **结论：彻底不做免登录与来源地址判定**——部署时就把管理员和初始密码生成好（§3.5.2），所有人（含本机浏览器）走同一条登录路径。少一个分支，就少一整类风险。

### 3.5.2 首次启动：生成管理员与初始密码
- 首次启动（库中无账号）即创建 `admin`（角色 admin），密码用 **CSPRNG 生成**：
  - 长度 ≥16、排除易混字符（`0/O/1/l`）、分组可读（形如 `ABCD-EFGH-JKMN-PQRS`）便于人工输入；
  - 只存 argon2id 哈希；明文仅在内存中出现一次，输出到下面两处；
  - `must_change_password = true`。
- 明文的两条出口（都要）：
  1. **stdout/日志**一次性打印：`首次启动管理员账号: admin ｜ 初始密码: ABCD-EFGH-JKMN-PQRS ｜ 已同时写入 <data_dir>/initial-admin-password.txt，首次登录成功后该文件自动删除`
  2. **文件** `<data_dir>/initial-admin-password.txt`（Windows 上尽力设置仅当前用户可读的 ACL）——解决"控制台滚动丢失 / 无控制台（服务、nohup、容器）"场景。
- **重启不重新生成**：密码只在"无账号"时生成一次；重启沿用同一密码，避免"每次重启密码都变"的惊吓。
- **显式指定初始密码**（无人值守，推荐）：`--admin-password-file <path>`（不进进程列表）；`VTRIM_ADMIN_PASSWORD`（会出现在 `docker inspect` / `/proc/<pid>/environ`，日志给出该风险提示）；`echo 'pw' | vtrim user passwd admin --stdin`。指定后不再生成随机密码。
- 文件生命周期：**首次成功登录后删除**（删除失败只记 warning 不阻塞）；`vtrim user passwd` 改密后也删除。
- 因为部署时密码已就绪，**不需要** `setup` 初始化页、不需要 `setup token`、不需要"未初始化"状态。

### 3.5.3 单人 / 多人只差这三件事
| 维度 | 单人 | 多人 |
|---|---|---|
| 账号数 | 1（admin） | N（admin 建号） |
| UI | 不显示用户管理/审计入口 | admin 显示 |
| 默认值 | 配额不限、并发按单机（见 §6/§7） | 管理员按用户配 `quota_bytes`，全局 `max_concurrent_jobs` 封顶 |
| **认证** | **必须登录（同多人）** | **必须登录** |

所有项目/任务/入库记录从一开始就带 `owner_id`（单人也一样），所以**不存在"单人数据无主、多人化时要迁移"的问题**，也没有单人特例代码分支。

### 3.5.4 唯一的暴露面硬门槛：不得仍用初始密码
- 本机（loopback）监听：初始密码可登录；UI 顶部常驻横幅 + 每次启动日志 warning 提示尽快改密。
- **对外监听（非 loopback）时必须 `must_change_password = false`**（即已改密，或在部署时用 `--admin-password-file` 显式设定），否则**拒绝启动**并给出修复命令（`vtrim user passwd admin`）。理由：初始密码会出现在日志与初始密码文件里，等同于"把密码写在服务器上"，不应对外。
- 其余保持不变：对外强制登录 + 建议 TLS 反代（文档给 Caddy/nginx 示例）+ 启动告警。

### 3.5.5 忘记密码：确定性救援路径
- `vtrim user passwd <name> [--stdin]` / `vtrim user reset-admin-password`：**能在这台机器上执行 CLI 的人本就等价于拥有主机权限**，因此直接允许，不引入任何后门；写审计。
- 无 CLI 的场景（容器/托管）：可选增强——启动时若存在 `<data_dir>/.admin-reset`（0600），则把 admin 密码重置为该文件内容并立即删除，日志记审计。
- 明确不做：邮箱找回、安全问题、"密保"。

### 3.5.6 代价与缓解（诚实记录）
- 代价：连"打开本机浏览器"都需要登录一次（这是去掉免登录的直接结果）。
- 缓解：会话 7 天滑动续期 + 可选"记住此设备 30 天"（本机浏览器基本只登录一次）；`vtrim server --open` 启动日志直接给出用户名与初始密码文件路径。
- 明确不做：把初始密码预填到登录页、把密码放进 URL/二维码（都会造成泄露面）。

### 3.5.7 身份数据不可缺省（与现有容错的差异）
- 现有策略：程序配置目录不可写时，`vtrim server` 用内建默认值继续跑（只读安装位置仍能用）。
- 身份/账号数据**不能这样**：`data_dir` 或 `vtrim.db` 不可写 → **启动失败**并给出修复指引（`--data-dir <可写路径>`），绝不做"静默内存模式"——否则会出现"以为改了密码/建了号，重启后全没了"。
- 一句话原则：**配置可缺省，身份不可缺省。**

### 3.5.8 单实例保护与 schema 演进
- `<data_dir>/vtrim.lock`（pid + 启动时间，进程退出释放）：第二个实例指向同一 `data_dir` 时明确报错（避免两个 server 抢同一批项目/端口/SQLite 写）。仅当确认是崩溃残留（pid 不存在）才允许 `--force`。
- `PRAGMA user_version` + 启动自动迁移；迁移前自动备份 `vtrim.db.bak-<old_ver>`；迁移失败则拒绝启动（不半迁移）。

### 3.5.9 可观测性
- `/api/health`：`version`、能力位（`ffmpeg` / `chrome` / `llm` / `ingest_ready` / `wgpu_render`）、`accounts`（账号数）、`must_change_password`、`listening: loopback|remote`、`tls_terminated`（由可信代理头判定，仅作提示）。**不暴露路径，也没有 `mode/auto_login/setup_required` 这些分支字段。**
  注：`ingest_ready` 同时覆盖"录播姬 webhook 已启用 + B站下载器可用"，前端据此决定首页/入库区块显示引导卡还是内容。
- `vtrim doctor` 增"安全摘要"：监听地址与可达性、账号数、是否仍在使用初始密码、初始密码文件是否仍存在、TLS 终止探测、`data_dir` 可写性与剩余磁盘、备份建议。
- 前端：`must_change_password` 时顶部横幅（一键去改密）；对外且未 TLS 时一次性提示。

### 3.5.10 验收清单（"单人即开即用 + 多人原生"如何算成立）
1. 全新解压 → `vtrim.exe server --open` → 日志打印用户名/初始密码/文件路径；`<data_dir>/initial-admin-password.txt` 存在。
2. 浏览器打开 → **登录页** → 用初始密码登录成功 → 进入项目页；未登录访问任意接口（含 loopback）一律 401。
3. 首次登录成功后 `initial-admin-password.txt` **被自动删除**；横幅提示"仍在使用初始密码"。
4. 重启服务 → **密码不变**（不重新生成）；未过期的会话仍然有效。
5. 改密后：旧密码失效、新密码可用、`must_change_password=false`、横幅消失。
6. 建项目并跑 extract/transcribe/detect → 项目 `owner_id` = admin（`/api/projects`、`/api/storage` 正常）。
7. 仍在使用初始密码时 `--host 0.0.0.0` → **拒绝启动** + 一条修复命令；改密后同一命令可正常启动并远程登录。
8. `--admin-password-file pw.txt` 全新部署 → 不生成随机密码，直接用该密码登录。
9. `vtrim user add alice` + 各自登录 → 逐接口验证互不可见（项目/任务/SSE/media 全 404）。
10. 忘记密码 → `vtrim user reset-admin-password`（或 `passwd admin --stdin`）本机恢复，写审计。
11. `data_dir` 只读 → 启动失败 + `--data-dir` 指引（不会表面可用实际不保存）；两个实例指向同一 `data_dir` → 第二个明确报错。



## 4. 采集：录播姬接入（主力通道）

### 4.1 Webhook 接收（按官方 v2 契约）
参考：[录播姬 Webhook 文档](https://github.com/BililiveRecorder/website/blob/main/src/content/docs/reference/webhook.md)（外部文档，实现时以其当前版本为准）

- 路由：`POST /api/ingest/bililive/<token>`（录播姬只能填 URL，无法加自定义头 → token 放路径；token 存哈希、可在 admin 页生成/轮换/禁用；可选来源 IP 白名单，同机部署填 `127.0.0.1`）。
- **必须尽快返回 2xx**（非 2xx 会被最多重试 3 次、payload 不变）→ handler 只做：验 token → 以 `EventId` 去重（表 `ingest_events(event_id PK, received_at, kind, session_id)`）→ 写入/更新 `ingest_sessions` → **立即 200**，重活交给后台任务。
- 事件处理（**不依赖顺序**，官方明确不保证发送/接收顺序）：
  - `FileOpening`：记录该 session 的待关闭文件（`RelativePath`、`FileOpenTime`）。
  - `FileClosed`：记录已完成文件（`RelativePath`、`FileSize`、`Duration`、`FileCloseTime`）；**不立即建项目**（一次录制可能输出多个文件）。
  - `SessionEnded`：标记 session 已结束。
  - `StreamStarted` / `StreamEnded`：仅记审计/通知，不参与入库。
  - 未知事件类型：忽略但记审计（官方声明以后会新增类型）。
- **完成判定**：`SessionEnded` 已到 **且** 此后 `[ingest.recorder] finalize_idle_mins`（默认 5）内无新 `FileClosed`（应对"下一个 `FileOpening` 早于上一个 `FileClosed`"的乱序）；或 `SessionEnded` 已到且该 session 的 `FileOpening` 全部有对应 `FileClosed`。
- 落库：`ingest_sessions(session_id PK, room_id, short_id, owner_id NULL, state open|closing|finalized|failed, opened_files json, closed_files json, first_event_at, last_event_at, project_id NULL, error NULL)`；服务重启后可继续未完成会话。
- 兜底扫描：`[ingest.recorder] watch_interval_mins`（默认 10）扫描 `recorder_root` 下白名单扩展名（flv/mkv/mp4/ts），以（路径 + 大小 + mtime 稳定 ≥2 轮）判定"新完整文件"，与 `ingest_files` 比对后补登记 —— 覆盖漏推送、录播姬重启、历史录播导入。

### 4.2 房间 → 账号映射与自动策略
- 配置：`[ingest.recorder] root = "D:\\BililiveRecorder"`、`webhook_enabled`、`channel_map = { "23058" = "alice" }`（键可用 `RoomId` 或 `ShortId`；也可在页面维护，落 `channel_map(room_id PK, user_id, auto_pipeline, enabled)`）。
- `auto_pipeline ∈ {none(仅入库), transcribe, detect, cut}`，**默认 `detect`**（下完 + 转写 + 检测，不自动切片以免白占盘）。
- 未映射的房间/文件 → **待分配池**（仅 admin 可见并指派）；指派后按该用户配额与策略处理。
- 用户侧"订阅"体验：用户看到的是"我的房间映射 + 最近入库记录 + 自动策略开关"，无需轮询任何外部站点。

### 4.3 多文件会话 → 单项目（`input_path` 是单文件）
- 同 `SessionId` 的多个文件（录播姬按大小/时长切分）→ 默认**合并**为一个 `source.mp4`：`ffmpeg -f concat` + `-c copy`（同编码无损、秒级），失败回退重编码；原始分片**保留**（源不删策略）。
- 可选 `concat_files=false`：只把第一个文件作为项目源，其余登记为附加片段（前端只读提示）。
- 项目目录：`<projects_root>/users/<uid>/<room>/<session 起始时间>-<pid>/`，`vtrim.toml` 的 `input_file`/`danmaku_file` 均为项目内相对路径。
- **落位策略**：`move_into_managed=true`（默认）——同盘 `fs::rename`（瞬时）；跨盘则 `copy + 校验 + 删源`，开始前做**水位预检**。`projects_root` 默认与 `recorder_root` 同盘（如 `D:\vtrim-projects`）。若要保留录播姬的文件管理/播放功能，设 `false`（项目目录＝录制目录，代价：同一目录只能有一个项目）。

### 4.4 弹幕（本项目价值最高的输入）
- 录播姬会在录制目录写同场次弹幕文件（通常 `*.xml`）。仓库当前**只解析 ASS/SRT** → 决策：**服务端新增 `danmaku::parse_bililive_xml`**，把 XML 直接映射为 `DanmakuEntry{start, text, color, y}`（语义一一对应，约 100 行 + 单测），`load_danmaku_auto` 增加 `.xml` 分支 —— 部署方零额外配置。
- 关联规则：与视频**同 session / 同目录同前缀**的 `.xml`/`.ass`；写进项目 `vtrim.toml` 的 `danmaku_file`；缺失时项目仍可跑（只是失去 popularity 排序，前端明确提示）。

### 4.5 次要通道：B站链接转项目
- `POST /api/ingest/resolve {url}` → `{video_id, title, duration, owner, parts[], cover, estimated_bytes, already_exists}`；确认后 `POST /api/projects {source:{kind:"bilibili", url|video_id, part_ids?, quality?}}`。
- 下载器适配层：`[ingest] downloader = "bbdown"|"yt-dlp"|"<绝对路径>"`、`max_quality`、`rate_limit_kbps`、`max_concurrent_downloads=1`、`cookie_file`（仅 admin）；错误分类可读（`UnsupportedUrl/NeedLogin/MemberOnly/RateLimited/ToolMissing/DiskFull`）。
- 分P 默认每 P 一个项目；弹幕优先让下载器直接出 ASS，否则走 §4.4 的解析路径。

## 5. 存储与隔离

### 5.1 目录布局
```
exe_dir/.vtrim/            # 平台资源：config.toml / llm.toml / models / stickers（仅 admin）
  cloud/vtrim.db           # users / sessions / projects / ingest_* / channel_map / audit
<projects_root>/users/<uid>/<room>/<session>-<pid>/
  source.mp4 | 源分片       # 源录播（不自动释放）
  danmaku.xml / danmaku.ass
  vtrim.toml
  output/                  # video.wav video.srt transcript.json events.json clips/ publish_config.json
D:\BililiveRecorder\       # 录播姬工作目录（配置项，不归 vtrim 所有）
```
- **归属由 DB 决定**：用户可见范围 = 他拥有的 `projects` 行；囚笼以**项目根**为单位（不再有"每用户一个 root"）。
- CLI 兼容：`vtrim polish render <项目目录>`、`vtrim extract -i <目录内视频>` 仍可手工执行。

### 5.2 路径面归零
- 所有 `/api/project-data/*`、`/api/polish/*` 统一 `?project=<id>` + **项目内相对路径**（如 `{project:7, dir:"output/clips/clip-01"}`）；服务端查 `projects` 得项目根 → `canonicalize` → 必须 `starts_with(project_root)` → 拒绝绝对路径 / `..` / 符号链接逃逸。
- 删除：`/api/fs/list`、`project_init`/`project_open` 的 `input|config_path|output_dir`、`polish render` 的任意 `output_file`（改项目内固定 `export.mp4`）、`cover-background` 的任意 `source`。
- **webhook 路径囚笼**（新增，必须）：`EventData.RelativePath` 与 `[ingest.recorder] root` 拼接后必须仍在 `root` 内；否则持有 token 者可让服务把任意文件登记为项目、再经 `/api/media/*` 读出。
- 纵深防御：`polish render/preview` 前，对 `resolve_paths` 解析出的 `video/subtitles/danmaku` 再校验"必须在该项目目录内"。
- 新增 `vtrim-core/src/tenant.rs`：`jail_join(root, rel)` / `jail_existing(root, rel)`；单测覆盖 `..`、绝对路径、UNC、`C:foo`、符号链接、空串、末尾分隔符。

### 5.3 仅管理员可访问
`/api/admin/{config,models,doctor,llm/test,users,jobs,ingest}`（含录播姬 token、`recorder_root`、channel_map、下载器与 cookie）。`/api/health` 只回版本 + ffmpeg/chrome/llm/downloader 就绪位 + `wgpu_render` + 是否需登录，不回任何路径。

## 6. 任务与容量

- 分槽：**入库槽**（`max_concurrent_ingest=1`：合并/移动/ffprobe/弹幕解析）、**重活槽**（`max_concurrent_jobs=2`：ASR/剪切/渲染）、**下载槽**（`max_concurrent_downloads=1`）。入库与下载不占算力。
- 每用户在跑 1 个重活（同项目绝不并发写），其余排队；每用户排队上限 `per_user_queued_jobs=3` → 超出 429。
- 调度 FIFO + 同用户串行 + 全局信号量；`JobEvent` 带 `job_id`；`/api/stream` 只推本人事件；`/api/jobs/:id/cancel` 校验归属；取消沿用**协作式取消令牌**（阶段/批次边界生效，排队中的直接出队）。
- 入库任务可回滚：移动/拷贝中途失败 → 清理半成品、`ingest_sessions.state=failed` 并记原因（供"待分配/失败"列表重试）。

## 7. 磁盘上限（只限制空间，不自动删）

- 配置：`[server] max_disk_bytes`（全局，0=不限）、`users.quota_bytes`（0=不限）、`disk_watermark_pct=10`、`[storage] projects_root`。
- 计量：`storage(user_id, bytes, updated_at)` 在入库/切片/渲染/删除后更新（避免每次 `du`）；`GET /api/storage` 返回我的占用、上限、按项目明细。
- 超限：入库前（webhook 触发时也要）与下载/渲染前统一预检 → 不足则 `ingest_sessions.state=failed` 或 HTTP `409`，文案含"预计需要/可用/建议删除哪些项目的源录播"，**绝不自动删**。
- 源录播按决策**保留**；项目页提供手动"删除源录播（保留切片/字幕/事件/封面）"动作 `DELETE /api/projects/:id/source`（唯一的手动释放手段）。
- 只清理**临时态**：`.partial/`、`*_thumb.jpg`、`*.partial.json`、`*.checkpoint.json` 超 7 天。
- 备份＝拷贝 `.vtrim/cloud/vtrim.db` + `projects_root`（WAL，`PRAGMA foreign_keys=ON`）。

## 8. API 与前端变更

| 现在 | 改造后 |
|---|---|
| — | `POST /api/auth/login` `POST /api/auth/logout` `GET /api/me` |
| `GET /api/health`（含 workspace 路径） | 无路径；能力位 + `accounts / must_change_password / listening / tls_terminated` |
| `GET /api/project/list` | `GET /api/projects`（仅本人；含来源/占用/状态） |
| `POST /api/project/init|open|delete` | `POST /api/projects`（仅 `source`）/`DELETE /api/projects/:id`/`DELETE /api/projects/:id/source` |
| — | `POST /api/ingest/bililive/<token>`、`GET /api/ingest/records`、`GET/PUT /api/channel-map`、`POST /api/ingest/resolve`、`GET /api/storage` |
| `/api/project-data/*?config=<路径>` | 同语义，`?project=<id>` |
| `POST /api/fs/list`、任意 `dir`/`source`/`output_file` | **删除**；`{project, path}` 项目内相对路径 |
| `GET /api/config`、`/api/models`、`/api/doctor`、`/api/llm/test` | `/api/admin/*` |

前端：
1. `api/client.js`：注入 `X-CSRF-Token`；`401`→跳登录（记回跳）、`403` 越权、`409` 显示磁盘/队列原因、`429` 排队已满。
2. `LoginView`：**始终启用、无免登录分支**；启动先 `GET /api/me`，401 即跳登录并记住回跳；`must_change_password` 时顶部常驻横幅（一键去改密）。
3. 首页新增**入库动态**与**我的房间映射**（开关 + 自动策略档位）；删除 `FileBrowserModal` 的"挑选服务器文件"用途。
4. `StorageView`（占用/上限/按项目明细/删除源录播）、`AdminView`（账号/配额/录播姬 token 与 `recorder_root`/下载器与 cookie/全平台任务/待分配池指派）。
5. 项目作用域 `?config=<enc path>` → `?project=<id>`（`router.js` 守卫、`ProjectLayout`、`PreviewView`、`ProjectSlicesView`、`ProjectClipsSection`、`PolishView`、`PolishProjectsView`、`ProjectSettingsView`）。
6. 重建并提交 `webui/dist`。

### 8.1 前端契约（与 `docs/plan/webui-multiuser-frontend.md` / 工作台 v5 对齐）

> 前端阶段 M1–M5 与服务端 P1–P5 一一对应（见该文 §0）。**前置**：工作台重构
> `docs/plan/webui-workbench-redesign.md`（v5）先落地，它已预留不透明 `:pid`、`scopeQ(pid)` 单点、
> client.js 错误分类与 CSRF 注入点、侧栏入口列表化——服务端 P1 不依赖它，可并行；M2 依赖其单点改造。

**认证（M1）**
- `GET /api/me`：已登录 → `{ user: {id, username, role}, must_change_password }`；未登录 → **401**。
  **没有 `auto_login` / `requires_login` 字段，也没有免登录分支**（单人部署＝只有一个 admin 账号的部署）。
- `POST /api/auth/login {username, password}` → `{ user, must_change_password }`（`Set-Cookie: vtrim_sid`；同时下发 `vtrim_csrf`）。
  失败：401（密码错/账号禁用文案直出）、429（限流，前端文案「尝试过多，15 分钟后再试」）。
- `POST /api/auth/logout` → 200（删会话行）。
- `PUT /api/auth/password {old_password, new_password}` → 改自己的密码；成功后 `must_change_password=false`（横幅消失、初始密码文件已删）。
- 前端启动：`GET /api/me` → 200 进工作台 / 401 → `#/login?redirect=<当前路由>`；任意请求 401 → 清 auth store + toast「登录已过期」+ 回登录（带回跳）。
- 用户区：`用户名 · 角色` + 登出弹层；`must_change_password` 时顶部常驻横幅。
- 侧栏可见性由 `role` 决定（**无"本机模式"特例**）：单人部署唯一账号是 admin，故可见 `[任务, 入库, 存储, 管理]`。

**任务（M3–M5）**
- `/api/jobs`、`/api/jobs/:id` 每项：`id, kind, status(queued|running|succeeded|failed|cancelled), message, slot(ingest|download|heavy), current, total, created_at, error?`；
  `queued` 时附 `queued_position`（同用户队列位次，1 起）——前端显示「排队第 N 位」与槽位徽标。
- 每用户已在跑 1 个重活时，新任务进入 `queued` 而**不是** 409（409 只用于磁盘/水位等必须用户先处理的原因）；排队上限超出 → **429**（「排队已满，请等待任务完成」）。
- `/api/stream` 只推本人事件（服务端过滤，前端无感知）。

**项目与存储（M2 / M5）**
- `GET /api/projects` 每项：`{id, name, source:{kind:"recorder"|"bilibili"|"local", label}, stage, progress?, bytes, source_bytes, has_clips, missing?}`。
- `DELETE /api/projects/:id/source` → 删源录播（保留产物），返回最新占用。
- `GET /api/storage`：`{used_bytes, quota_bytes, global_max_bytes, watermark_pct, disk_free_bytes, projects:[{id, name, source_label, source_bytes, output_bytes, total_bytes, last_active_at}]}`。
- 磁盘/配额不足统一 **409**，`error` 文案含「预计需要 / 可用 / 建议删除哪些项目的源录播」（前端据此给「去存储页」按钮）。

**入库（M3）**
- `GET /api/ingest/records`：`[{session_id, room_id, room_label, started_at, state(open|closing|finalized|failed|unassigned), files, bytes, project_id?, error?}]`。
- `GET/PUT /api/channel-map`：`[{room_id, user_id, auto_pipeline(none|transcribe|detect|cut), enabled}]`（PUT 整体替换该用户规则表）。
- `POST /api/admin/ingest/assign {session_id, user_id, auto_pipeline}`（待分配指派，admin）。
- `POST /api/ingest/retry {session_id}`（失败重试）。
- `/api/health.ingest_ready=false` → 前端显示配置引导卡（仅 admin 可跳管理页）。

**封面背景图（唯一的本地文件例外）**
- `POST /api/polish/cover-background {project, image_data}`（base64，≤32MB，落项目目录）；不接受视频或其它文件类型。

## 9. 测试与验收

单测/集成（fixture + mock，离线）：
- **Webhook**：官方样例的 `SessionStarted/FileOpening/FileClosed/SessionEnded`；`EventId` 重复投递只处理一次；乱序（`SessionEnded` 先于 `FileClosed`、下个 `FileOpening` 先到）仍正确 finalize；未知事件忽略；token 错误 401；`RelativePath` 越出 `recorder_root` 被拒。
- **会话聚合**：3 个分片 `FileClosed` + `SessionEnded` → 一个项目、按序 concat、`ffprobe` 校验失败则 `state=failed`；`concat_files=false` 时只取首片。
- **兜底扫描**：注入未覆盖文件 → 扫描后补登记；大小仍在变化的文件不登记。
- **弹幕**：录播姬 XML 样本 → `DanmakuEntry` 数量/时间/颜色/位置正确；缺弹幕时项目仍可跑。
- **囚笼**：`..`、绝对路径、UNC、`C:foo`、符号链接、空串 → 全 `Err`；`?project=<他人>`、他人 job/SSE/media → 404。
- **配额/水位**：`max_disk_bytes`、`quota_bytes` 预检拒绝（文案含预计/可用）；跨盘移动时水位跌破 → 中止且清理半成品；删除源后占用回落。
- **调度**：2 用户 × 3 重活 + 1 入库 + 1 下载 → 重活 ≤2、入库 ≤1、下载 ≤1、同用户串行、FIFO；取消排队即出队；在跑任务在阶段边界停止。
- **认证**：错误密码/禁用/过期会话/缺 CSRF/普通用户访问 admin 接口 403。

手工验收：全新 `.vtrim` + `vtrim server`（loopback）→ 日志打印初始密码与密码文件路径 → 登录页用初始密码进入 → 首次登录成功后密码文件被删除、横幅提示改密；用 `curl` 模拟录播姬推 `FileClosed+SessionEnded`（指向测试 flv）→ 项目自动出现 → 自动转写/检测 → 切片 → 精修预览/渲染；第二账号验证完全隔离；把 `max_disk_bytes` 压到接近已用 → 新入库被明确拒绝；项目页删除源录播 → 占用回落。

命令：`cargo check -p vtrim-cli --all-targets`（0 warning）、`cargo test -p vtrim-cli`、`cargo test -p vtrim-core --lib`、`npm --prefix webui run build`，加上面的手工脚本。

## 10. 实施顺序（每阶段可独立交付/回滚）

- **P1 账号与账本**：SQLite + users/sessions/audit + argon2 + `/api/auth/*` + `/api/me` + 中间件 + `vtrim user *` + **首次启动生成 admin 与初始密码**（日志 + `<data_dir>/initial-admin-password.txt` + `must_change_password`）+ **全部接口统一要求会话（含 loopback，无免登录分支）**。此阶段仍只有 1 个账号，行为上就是"要登录的单人版"。
- **P2 项目托管与隔离**：`projects` 表 + 项目目录布局 + `?config=`→`?project=` + `tenant.rs` 囚笼 + 删除全部本地路径入口 + admin-only 接口迁移 + 前端作用域改造。
- **P3 录播姬接入**：webhook（`EventId` 去重 + 乱序容错 + 立即 2xx）+ `ingest_sessions/ingest_files` + 兜底扫描 + channel_map + 多文件聚合/合并 + XML 弹幕解析 + 水位预检 + 入库槽任务 + 首页入库动态。
- **P4 B站链接通道**：`resolve` + 下载器适配层 + 下载槽 + 新建项目弹窗（次要入口）。
- **P5 容量与运维**：`storage` 计量与上限、管理员页、审计、临时清理、反代/TLS 文档、`vtrim doctor` 多用户自检、备份/恢复说明、`AGENTS.md`/`README.md` 更新。

## 11. 威胁模型 / 合规边界 / 非目标

- **覆盖**：跨租户读写（项目囚笼 + 归属校验）、webhook 伪路径（`recorder_root` 囚笼）、密钥/cookie 泄露（admin-only、不回传）、暴力登录（限流）、CSRF、越权查看/取消他人任务、磁盘写爆（预检 + 水位）、token 泄露（可轮换/禁用 + 可选 IP 白名单）。
- **合规**：不接收用户上传的视频文件；服务仅自托管内部使用（需登录、≤10 人）；不提供"整片打包导出"接口（`/api/media/*` 只为预览/精修单文件服务）；录播姬录制与 B站 下载的来源授权由部署者自行确认；文档写明该边界。
- **非目标**：用户上传/本地目录挑选/本机 agent；多节点与对象存储；每用户自带 LLM key；计费；OAuth；每用户独立模型副本；录播姬之外的录制工具适配（接口留扩展位，payload 适配器分离）；不做"自动删源/自动清理用户数据"。

## 12. 风险与回滚

1. **动到刚稳定的单机 server** → P1 只加"身份 + 登录门槛"（账号仅 admin，功能面与今天一致，但浏览器开始要求登录一次）；P2 才切换数据选择器（`?config=` → `?project=`），切换期保留 `?config=` 只读兼容一版并标 deprecated。每阶段可独立回滚。
2. **新增 SQLite/argon2 构建风险**（bundled sqlite 需 cc；仓库已有 `bzip2` 同类构建）→ 退路是文件账本（`users.toml`+`sessions.json`+复用已实现的 `vtrim_core::filelock`），隔离/调度设计不变。
3. **录播姬 webhook 契约变化**（v1 弃用、未来新增事件类型、不保证顺序）→ 只依赖 `SessionId/RelativePath/FileSize/RoomId` 这些稳定字段；未知事件忽略并记审计；兜底扫描保证"webhook 全挂也能入库"。
4. **跨盘移动慢/写满** → `projects_root` 默认与 `recorder_root` 同盘（rename 瞬时）；跨盘必带水位预检与半成品清理；可切 `move_into_managed=false`。
5. **源文件不释放 ⇒ 磁盘单调增长** → 靠全局/每用户上限 + 水位预检 + 手动删源 + 占用明细提示；这是既定策略，不做自动删除。
6. **`panic = "abort"`** → 多用户下放大为"一个人搞挂所有人"：P5 增加 `catch_unwind` 兜底中间件并审计 handler 内 `unwrap/expect`；不想要兜底则明确接受该风险。

---

## 13. 与当前已实现加固的衔接（地基已在，勿重复做）

2026-09-12 的两笔提交已把多用户改造直接依赖的地基打好（提交 `ba1dafb`、`5e9d4c0`）：

| 已实现 | 对本计划的意义 |
|---|---|
| `/api/*` 未注册路径返回 **JSON 404**（不再回退 SPA HTML） | P1/P2 改路由时行为可预期，前端不再静默拿到 `null` |
| `JobManager` **单活动任务 + 409**、`JobStatus/JobEvent` 状态机 | P3/P4 直接把它扩展成"每用户串行 + 全局并发 + 排队"，取消链路已可用 |
| **协作式取消令牌**（`ProgressReporter::cancelled()`，阶段/批次边界） | 多用户取消语义直接复用；排队任务的取消＝出队 |
| SSE **先回放历史**、`Lagged` 不再误报失败 | 用户级 `/api/stream` 过滤时不会出现"任务假失败" |
| `/api/media/*` Range **416/206** 修复、文件不存在 404 | 多用户播放源/产物的出入参校验已可靠 |
| `dir` 校验（非空 + 存在 + 是目录）、`project_delete` 只允许 `vtrim.toml`、字体名/`rank` 收敛 | P2 的"路径面归零 + 囚笼"可以在这些校验上直接收紧 |
| `vtrim_core::filelock`（按路径串行读改写）+ `persist` 原子读改写 | P1 若走"文件账本"退路可直接复用；即使走 SQLite，`polish.toml`/`events.json` 并发写也已被保护 |
| `vtrim_home()` 去 `panic`、环境探测缓存/`spawn_blocking` | 多用户下 handler 不 panic、不阻塞 worker 的前提 |
| `webui/dist` 入库 + 非 loopback 需 `--allow-remote` | P1 之后 `--allow-remote` 语义改为"强制登录"，dist 已有版本管理流程 |
