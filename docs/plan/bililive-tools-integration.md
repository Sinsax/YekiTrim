# biliLive-tools × YekiTrim 对接方案

> 日期：2026-10-29 ｜ 状态：**通用 Webhook + 出片回传已实现（2026-10-29）**，复制落位仍待做 ｜ 目标读者：部署/维护这套服务的人
>
> 结论：**可以对接，推荐「同机文件系统 + 事件订阅」**。biliLive-tools 继续负责
> 录制 / 录制弹幕 / B站投稿 / 网盘同步；YekiTrim 只负责自动切片、字幕、精修、出片。
> 两边的连接点只有三个：**录制完成的文件路径、同名弹幕文件、出片后的上传调用**。

---

## 0. 已实现：通用 Webhook（2026-10-29）

先落了一个**不绑定具体录制软件**的入口，biliLive-tools 只是它的第一个客户端：

```
POST /api/ingest/webhook/<token>
Content-Type: application/json

# ① biliLive-tools「事件订阅」原样（type=file_completed）
{"type":"file_completed","data":{"filePath":"/app/video/.../rec.flv","roomId":"123456","platform":"Bilibili"}}

# ② 极简规范
{"event":"file_completed","file":"/app/video/.../rec.flv","room_id":"123456","title":"可选"}

# ③ 录播姬 FileClosed 也能打进来（与 /bililive 同一套字段）
{"EventType":"FileClosed","EventData":{"RoomId":"123456","RelativePath":"Bilibili/.../rec.flv"}}
```

行为：

- 认证复用 `[ingest]` 的 `webhook_enabled` / `token_hash` / `allow_ips`（与录播姬同一个 token）；
- `[ingest.webhook]` 配置：`root`（宿主机录像根，**空 = 未启用**）、`container_root`
  （事件里是容器路径时先剥的前缀，例 `/app/video`）、`auto_pipeline`（默认 `none`）；
- 路径一律过 `tenant::ensure_within`（`..`/绝对路径逃逸 → 400）；同目录 `-后处理.mp4` 优先，
  弹幕按原文件名找 `.xml > .ass > .srt`；
- 项目**建在录像原目录**（不搬不删源文件）；`yekitrim.toml` 落在录制目录里；
- 归属 = 首个管理员；项目 `source_kind=webhook`，弹幕预载策略同为弹幕；
- 重复投递按 `yekitrim.toml` 已登记复用，不会建第二个项目；
- `record_start` / `record_end` / `file_created` / `task` 等不建项目的事件回 **200 ignored**，
  不让录制软件把"不归你管"的事件重试；
- 设置页「录播姬（采集）」卡片里已加「通用 Webhook」一栏（root / container_root /
  auto_pipeline / 两种入口地址 / token）。

当前限制（下一阶段再做）：只做**原地项目**（没有 copy 到托管盘、没有待分配池/重试/兜底扫描）。

### 0.1 出片回传（已实现）

YekiTrim 出片后，精修/切片页出现 **「发布」**（单条）与 **「发布全部」** 按钮：

- `POST /api/publish/bililive-tools?project=<id>`：**创建后台任务**并立即返回 `{id, queued, pending}`
  （2026-10-30 改；以前同步跑完才回）。任务体把选中片段的 `clip.mp4` 复制到
  `staging_dir/<project_id>/<clip-dir>.mp4`，再按 `container_staging_root` 换成容器路径，
  调 biliLive-tools 的 `/preset/video/<id>` → `/bili/validUploadParams` → `/bili/upload`；
  任务占 `p:out` + `p:idx` 资源键（与剪切/管线互斥），可在任务抽屉看进度/取消；
- `GET /api/publish/bililive-tools?project=<id>`：读 `output/publish_state.json`，
  用 biliLive-tools 的 `/task` 回填 `submitted/running/completed/error`；
- **仅管理员**：发布配置是程序级的（B站账号/预设/passkey），普通账号不显示也不可用；
- 每条切片 = **一条视频**（`videos` 只有一项）；分区/标签/版权/合集/封面/重试全部沿用
  biliLive-tools 的**上传预设**（`upload_preset_id`），标题/简介按切片覆盖；
- B站登录账号：`uid = 0` 时自动取 `/user/list` 的第一个；也可以写死 uid；
- passkey 只从 **600 权限文件**读（内容为 `BILILIVE_TOOLS_PASSKEY`），不进 `config.toml`/日志；
- 发布状态是**手动触发**；没有接"渲染完成自动发布"。

配置：

```toml
[publish.bililive_tools]
base_url = "http://127.0.0.1:18010"
passkey_file = "<600 权限文件>"
upload_preset_id = "default"
staging_dir = "<宿主机上、biliLive-tools 容器也挂载的目录>"
container_staging_root = "<同一目录在容器内的路径>"
uid = 0
```

⚠️ 两个路径面必须指向**同一个目录**（只换前缀）；写错时 biliLive-tools 会报"文件不存在"。
⚠️ 跨文件系统复制（YekiTrim 项目盘 ↔ biliLive 录制盘）是预期行为：切片很小，不搬源录像。

---

## 1. 现场核实（2026-10-29）

| 事实 | 值 |
|---|---|
| 前端入口 | `http://192.168.0.107:3333/` = biliLive-tools 的静态前端（Caddy） |
| API 入口 | `http://192.168.0.107:18010/`（**不是 3333**；3333 不代理 API） |
| 版本 | biliLive-tools `3.22.2` |
| 部署 | Docker：`bililive-webui-1` + `bililive-api-1`；compose 在 `~/biliLive/docker-compose.yml` |
| 数据目录 | 宿主机 `~/biliLive/{data,video}` ↔ 容器 `/app/{data,video}` |
| API 认证 | 请求头 `Authorization: <BILILIVE_TOOLS_PASSKEY>`（compose 的环境变量；**不是** WebUI 里那个 `config.passKey`） |
| 录制目录 | `/app/video/<平台>/<主播>/[YYYY-MM-DD HH-mm-ss-SSS][标题]/` |
| 单场文件 | `<同名>.flv` 或 `.mp4`、`<同名>.xml`、`<同名>.ass`，可能还有 `-后处理.mp4` |
| 事件订阅 | 配置项 `externalWebhook`；biliLive-tools 会 POST `{id,version,time,type,data}` |
| 事件类型 | `record_start`、`record_end`、`file_created`、`file_completed`、`task` |
| YekiTrim 现有采集 | `ingest_sessions`（`kind=recorder/bilibili`）+ `ingest_worker`（完结判定/归属/配额/落位/合并/建项目/起管线） |
| YekiTrim 上传能力 | **没有** B站上传；只产出 `output/clips/*` + `publish_config.json` |
| 磁盘 | `~/biliLive/video` 32G 可用（90%）；YekiTrim 项目盘 114G 可用；**两者不同文件系统** |

### 1.1 关键路径语义

- biliLive-tools API 里的路径是**容器内路径**（`/app/video/...`）。已用 `/files/exists`
  实测：`/app/video/...` = `true`，同样的宿主机路径 = `false`。
- 事件里的 `filePath` 同样是容器路径；YekiTrim 必须做
  `容器路径 → 宿主机路径` 的映射后才能读文件。
- 投稿接口 `/bili/upload` 的 `videos[].path` 也必须落在**容器可见**的路径里。
  YekiTrim 的成片默认在另一个盘，**必须**先放进 `/app/video`（见 §4.3）。

---

## 2. 职责划分（不要重复造）

| 能力 | biliLive-tools | YekiTrim |
|---|---|---|
| 直播录制、分段、断线重连 | ✅ | ❌ |
| 录制弹幕（XML / ASS） | ✅ | 解析、切片、字幕 |
| B站登录 / 投稿 / 分P / 分区 / 封面 / 重试 | ✅ | ❌ |
| 自动事件检测、切片 | 基础 | ✅ 核心 |
| ASR、LLM 字幕订正 | 有 AI 选项 | ✅ 核心 |
| 精修、Dhampir 出片、封面 | 有限 | ✅ 核心 |
| 任务队列 / 日志 / 配额 | ✅ | ✅ |

**一句话**：biliLive-tools 是“上游录制 + 下游上传”，YekiTrim 是中间的“自动剪辑工厂”。
两边都不需要把对方的功能再实现一遍。

---

## 3. 推荐架构（方案 A：原生适配器）

```
biliLive-tools 录制完成
        │  externalWebhook: {type:"file_completed", data:{filePath, roomId, platform}}
        ▼
YekiTrim  POST /api/ingest/bililive-tools/<token>
        │  ① 校验 token + allow_ips
        │  ② 容器路径 → 宿主机路径，jail 在配置根内
        │  ③ 去重：roomId + 会话目录 + 文件名
        │  ④ 落 ingest_sessions(kind="bililive_tools", state="closing")
        ▼
bililive_tools_worker（串行/小并发）
        │  ① 等同名 .xml/.ass 稳定（finalize_idle_mins）
        │  ② 选片：优先 -后处理.mp4，其次 .flv/.mp4
        │  ③ 复制（不是移动）到 <projects_root>/users/<uid>/<room>/<session>/
        │  ④ capacity 预检 → landing::land_project（写 yekitrim.toml + 登记）
        │  ⑤ pipeline_boot 起 auto_pipeline（none/transcribe/detect/cut）
        ▼
YekiTrim 项目（视频 + 弹幕）→ 自动切片 → 精修 → 出片
        │  可选「发布」按钮
        ▼
暂存目录 <biliLive 根>/_yekitrim_publish/<project>/<clip>.mp4
        │  POST /bili/upload（Authorization: passkey）
        ▼
biliLive-tools 投稿（B站/网盘）
```

### 3.1 必须钉死的几条设计

1. **只复制，不移动、不删除源文件**。
   - `~/biliLive/video` 只剩 32G（90%），且 biliLive-tools 的录制历史 / 自动投稿还引用原路径；
   - YekiTrim 盘 114G，项目落那边更安全；
   - 跨文件系统（btrfs ↔ ext），**硬链接不可用**，走 `copy + 校验`。
2. **选片顺序**：`-后处理.mp4` > 同名 `.mp4` > 同名 `.flv`。
   两者并存时只入库一次；事件给的 `filePath` 可能是原始 `.flv`（转换前的路径），
   以“目录里最新的最终文件”为准。
3. **弹幕优先 `.xml`**（录制原始 XML 信息最全），没有再退 `.ass`。
   YekiTrim 的 `load_danmaku_auto` 两种都支持。
4. **事件 + 扫描双保险**：事件是加速器，不是依赖。漏推 / 重启 / YekiTrim 离线时，
   按 `scan_interval_mins` 扫配置根补登记。
5. **路径映射 + jail**：只接受映射后位于 `[ingest.bililive_tools].root` 内的路径；
   `..`、绝对路径逃逸、符号链接逃逸一律 400。
6. **房间映射**：事件里的 `roomId`（如 `123456`）+ `platform`（`Bilibili`）是主键；
   复用现有 `channel_map`（房间 → 账号）与待分配池。不要把 bvid 塞进 `room_id`。
7. **投稿路径必须在容器内**：YekiTrim 成片先暂存到 `<biliLive 根>/_yekitrim_publish/`
   （容器里就是 `/app/video/_yekitrim_publish/`），再调 `/bili/upload`。

---

## 4. 数据契约

### 4.1 biliLive-tools → YekiTrim（事件订阅）

POST（无认证头，靠 URL token + 来源 IP）：

```json
{
  "id": "uuid",
  "version": 1,
  "time": "2026-10-29T03:00:00.000Z",
  "type": "file_completed",
  "data": {
    "filePath": "/app/video/Bilibili/<主播>/[2026-10-27 20-09-21-930][奋斗]/[...].flv",
    "roomId": "123456",
    "platform": "Bilibili"
  }
}
```

可用的 `type`：

| type | data（关键字段） | YekiTrim 用途 |
|---|---|---|
| `record_start` | roomId/platform/title/username/time | 日志 / 可选“开始录制”提示 |
| `record_end` | 同上 | 兜底结束信号 |
| `file_created` | filePath/roomId/platform | 忽略（文件还没写完） |
| `file_completed` | filePath/roomId/platform | **主触发**：登记入库 |
| `task` | event/taskId/taskType/status/error/durationMs/extra | 上传状态回写 |

> `task` 的 `taskType` 里投稿是 `biliUpload`；`status` 有
> `pending/running/paused/completed/error/cancelled`。
>
> ⚠️ 这些事件由 **biliLive-tools 自己的录制器** 触发；通过它的 webhook 收进来的
> 录播姬/其他录制器文件不一定发 `file_completed` —— 扫描兜底是必需的，不是可选项。

### 4.2 YekiTrim 配置（新增）

```toml
[ingest.bililive_tools]
# 连接
base_url = "http://127.0.0.1:18010"
passkey_file = "<600 权限文件，内容为 BILILIVE_TOOLS_PASSKEY>"
# 也可：passkey_env = "BILILIVE_TOOLS_PASSKEY"（仅当 YekiTrim 进程环境里也有这个变量）
# 路径
root = "<biliLive 宿主机根>/video"       # 宿主机根
container_root = "/app/video"             # 容器内对应根
# 事件
webhook_token_hash = ""                   # 新增 `yekitrim ingest bililive-tools token` 生成
allow_ips = []                            # 空 = 不限；Docker 部署时来源是桥接网段，不能只填 127.0.0.1
# 入库
auto_pipeline = "detect"                  # none | transcribe | detect | cut
finalize_idle_mins = 3
scan_interval_mins = 10
landing = "copy"                          # copy（推荐）| in_place
prefer_postprocessed = true
# 投稿
upload_preset_id = "default"
publish_staging = "<biliLive 宿主机根>/_yekitrim_publish"
```

> 配置改动遵守 `crates/yekitrim-core/src/config/AGENTS.md` 的五步：
> 字段 + `merge()` + 手写 `Default` + 模板/注释 + 必要时的 `config_version` 迁移。

### 4.3 YekiTrim → biliLive-tools（投稿）

```http
POST http://127.0.0.1:18010/bili/upload
Authorization: <BILILIVE_TOOLS_PASSKEY>
Content-Type: application/json

{
  "uid": 1234567890,
  "videos": [
    { "path": "/app/video/_yekitrim_publish/<project>/clip-01.mp4", "title": "切片标题" }
  ],
  "config": { "...": "从 /preset/video/<id> 取到的上传预设，覆盖 title/desc/source 等" },
  "options": { "removeOriginAfterUploadCheck": false }
}
```

- 投稿前先 `POST /bili/validUploadParams` 预检；
- `uid` 从 `GET /user/list` 取当前登录账号；
- 上传预设（分区/标签/版权/合集/封面规则）在 biliLive-tools UI 里维护，
  YekiTrim 只按 `upload_preset_id` 读取并覆盖每条切片的标题/简介/封面；
- 返回 `taskId` 落库；订阅 `task` 事件回写状态（成功/失败/审核）。

---

## 5. 实施方案

### Phase 0：连通性（0.5 天，已可验证）

1. `GET /common/version`、`GET /recorder/list`、`GET /recorder/:id/recent-record-folder`
   用 `Authorization: $BILILIVE_TOOLS_PASSKEY` 打通；
2. 用 `/files/exists` 确认容器路径语义；
3. 在 biliLive-tools 设置 →「事件订阅」填 YekiTrim 的测试 echo URL，
   确认 `file_completed` 能到；
4. 结论：不需要改 biliLive-tools，不需要开放新的公网端口。

**验收**：能从 YekiTrim 侧读到“最新一场录像 + XML/ASS 路径 + 房间号”。

### Phase 1：录制完成 → 自动入库（1.5–2 天）

后端：

- 新模块 `crates/yekitrim-server/src/ingest_bililive_tools.rs`
  （webhook + token/allow_ips + 路径映射 + 去重 + 落库）；
- 新 worker `crates/yekitrim-server/src/bililive_tools_worker.rs`
  （与 `ingest_worker` / `download_worker` 并列，**不互相 import**；
  复用 `service::ingest::{capacity,landing}` 与 `pipeline_boot`）；
- `ingest_sessions.kind = "bililive_tools"`；`projects.source_kind =
  "bililive_tools"`，并在 `assets::preload_policy` 映射到 `Preload::Danmaku`；
- 落位策略 `copy`：复制视频 + 弹幕 + 校验大小，**源文件保持不动**；
- 配额/水位复用 `capacity::check_capacity`；管理员按设计豁免；
- 事件去重：`session_id = hash(roomId + 会话目录名 + 文件名)`，重复投递回 200。

前端（设置页）：

- 新增「biliLive-tools（采集）」卡片：base URL、密钥（env/file 二选一）、
  root/container_root 映射、事件 URL + 生成 token、允许 IP、扫描间隔、
  「测试连接」「扫描历史」「立即扫描」；
- 事件 URL 只显示一次明文 token；库里存哈希（与录播姬同一套）。

**验收**：

- 模拟 `file_completed` → YekiTrim 出现项目、`danmaku_file` 指向 XML、
  `source_kind=bililive_tools`、自动任务到 detect/cut；
- 重复投递不建第二个项目；
- 重启后扫描不重复；
- 路径逃逸 / 错 token → 400/401。

### Phase 2：出片 → B站投稿（已实现，2026-10-30 改为后台任务）

- `POST /api/publish/bililive-tools`：入参 `project_id`、`clip_ranks`、
  `upload_preset_id`、每条切片的 `title/desc/cover` 覆盖；
- 先把成片复制/硬链接到 `publish_staging`（容器内可见），
  再 `validUploadParams` + `/bili/upload`；
- 新增 `publish_tasks` 表（或 JSON 侧车）记录 `taskId/状态/错误`；
- `task` 事件回写；前端在精修页/切片页出「发布」按钮（单条 / 全选）；
- 投稿失败时保留暂存文件，给出 biliLive-tools 的 `whyUploadFailed` 文案。

**验收**：

- 一条测试切片能从 YekiTrim 点到 biliLive-tools 的投稿任务；
- 任务 `task-end` 后 YekiTrim 显示“已投稿”；
- `task-error` 显示原因，不丢文件。

### Phase 3：历史回填与运维（0.5–1 天）

- 「扫描历史」默认只列清单、**不自动跑 ASR**（228G / 59 场，成本很高）；
  用户勾选后再入库；
- 入库/投稿失败重试；`ingest_sessions` 记录页显示来源 `biliLive-tools`；
- 磁盘水位告警（biliLive 盘 90% 已用；YekiTrim 项目盘 114G 可用）；
- 备份：`~/biliLive/data/app.db` + `video/`；YekiTrim 数据目录。

---

## 6. 备选方案（按推荐度）

| 方案 | 做法 | 优点 | 缺点 |
|---|---|---|---|
| **A. 原生适配器**（推荐） | 本方案 §5 | 长期最稳、复用现有入库/配额/归属/UI | 要改 YekiTrim 核心 + 配置 + 测试 |
| B. 外部桥 | 独立 Python/Node 守护进程：事件 → `POST /api/projects`（local 源）+ `POST /api/project-data/pipeline`；出片 → `/bili/upload` | 不改 YekiTrim，1 天可 PoC | 多一个进程；要自己维护 admin 会话/重试/幂等；错误分散 |
| C. 纯 API 拉取 | 轮询 `/record-history/*`、`/files/list`，用 `/assets/download` 下载 | 跨机可用 | 8G 文件走 HTTP，慢且占带宽；只建议跨机 |
| D. 伪装录播姬 | 把 `file_completed` 转成 `FileClosed` 投给 `/api/ingest/bililive/<token>` | 复用现有 webhook | 目录模板/房间号/字段语义不匹配，容易踩坑；不推荐 |

> 若只想先验证，B 方案最快；但生产建议直接做 A —— 两者可以共用同一套路径映射和
> “复制不移动”的落位规则。

---

## 7. 风险与对策

| 风险 | 对策 |
|---|---|
| 容器路径 vs 宿主机路径 | 配置 `root` / `container_root` 显式映射；所有读文件前过 jail |
| Docker 里打 webhook 的来源 IP 不是 127.0.0.1 | `allow_ips` 留空（靠 URL token）或加 Docker 桥接网段；别只填 127.0.0.1 |
| biliLive-tools 升级改 API/事件字段 | 钉版本 3.22.2；适配器只依赖本文列出的字段；升级前跑契约测试 |
| 原始 `.flv` 与 `-后处理.mp4` 双份 | 选片顺序固定；同会话只登记一次 |
| 事件丢失 / YekiTrim 重启 | 周期扫描兜底；`file_completed` 只是加速器 |
| 磁盘（32G / 90%） | 只复制不移动；入库前容量预检；历史回填默认手动勾选 |
| 上传路径在容器外 | 成片先暂存到 `/app/video/_yekitrim_publish/` |
| passkey 泄漏 | 只从 env / 600 权限文件读；不进 `config.toml`、不进日志 |
| 重复入库 | `(roomId, 会话目录, 文件名)` 幂等 + `land_project` 的 config_file 去重 |
| 录播姬/ biliLive-tools 同时录同一房间 | 只选一个来源；`channel_map` 只保留一处映射 |

---

## 8. 待确认

1. 是否保留录播姬作为第二来源，还是 biliLive-tools 完全取代它？
2. 新录像默认归属哪个账号（admin 兜底，还是按 `channel_map` 自动指派）？
3. 投稿形态：当前实现是**每片一条视频**；要「一条视频多 P」需要再改 `videos[]` + 分P标题（未做）。
4. 历史 228G：只做“可选清单”，还是先回填最近 N 场？
5. 是否接受把 YekiTrim 项目复制到 114G 的项目盘（推荐），
   还是必须原地 `in_place`（32G 盘，写 `output/` 到录制目录）？

---

## 9. 参考

- biliLive-tools 文档：<https://docs.irenmu.com/>
- 本仓采集/入库：`crates/yekitrim-server/src/ingest.rs`、`ingest_worker.rs`、
  `crates/yekitrim-service/src/ingest/landing.rs`
- 配置规矩：`crates/yekitrim-core/src/config/AGENTS.md`
- 路径面三防线：`crates/yekitrim-server/src/api/AGENTS.md`
