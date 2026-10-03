# 录播姬录制：从「管理员手工指派」改成「用户填直播间」

> **实施状态**：第 1–2 步已落地（`[ingest.recorder] api_base/api_user/api_pass`、
> `service/recorder.rs` 客户端、`GET/POST /api/rooms`、`DELETE /api/rooms/:room_id`、
> `POST /api/admin/recorder/test`、`channel_map` 加 `title/source`（账本 v6）、
> 首页「录播姬录制」块的用户视图）；第 3 步的**设置页「录播姬」卡**与第 4 步
> （`#/ingest` 降级 + 入口搬迁）待做 —— 目前 `api_base` 等键走设置页的
> 「源码模式」或直接改 `config.toml`。
>
> 现状（P3）是"录播姬把录像推给 YekiTrim → 管理员在待分配池里指派给某个用户"。
> 目标是把这条链路反过来：**用户自己填目标直播间，录完自动进他自己的项目**；
> 管理员只做一次"对接录播姬"的部署与配置。本文是这一版的规格（尚未实现）。

## 1. 角色分工（一句话版）

| 角色 | 做什么 | 频率 |
|---|---|---|
| 管理员 | 部署录播姬 + 在 YekiTrim 填「录播姬地址 / 账号密码 / webhook token」→ 之后不用管 | 一次 |
| 用户 | 在首页「录播姬录制」里粘贴直播间（房间号或链接）→ 等录像自动出现在项目里 | 每次想录时 |
| 系统 | 往录播姬加房间 → 收 webhook → 按房间归属自动入库 + 建项目 + 跑管线 | 自动 |

## 2. 为什么是录播姬（BililiveRecorder），怎么部署最省事

**选它**：活跃维护（最近提交 2026-09）、有 Windows 桌面版和 Docker 镜像、**自带 REST API**
（WebUI 用的就是它）、**webhook**（YekiTrim 现在解析的就是它的 payload）、弹幕一并录制（YekiTrim 的检测/切片要用弹幕）。
备选 blrec **已于 2025-10 归档停维护**，不建议新部署。自己用 ffmpeg/streamlink 拉流则要重造
"断线重连 + 弹幕抓取 + 分片修复"，不划算。

**Docker 部署（推荐，一条命令）**：

```bash
docker run -d --name bililive-recorder \
  -p 2356:2356 \
  -v /srv/recorder:/rec \
  -e BREC_HTTP_BASIC_USER=admin \
  -e BREC_HTTP_BASIC_PASS='<换成一个强密码>' \
  -e PUID=1000 -e PGID=1000 -e UMASK=022 \
  --restart unless-stopped \
  bililive/recorder:latest        # 或 ghcr.io/bililiverecorder/bililiverecorder:latest
```

要点：

- 配置与录像都在容器内的 **`/rec`**（所以只挂这一个卷）；`-p 2356:2356` 第一个端口可改，第二个别改。
- **`/rec` 必须让 YekiTrim 也读得到**：YekiTrim 的 `[ingest.recorder] root` 指向**宿主机**那个目录
  （同机部署 = 同一个路径；YekiTrim 也跑容器就用同一个卷）。
- 设了 `BREC_HTTP_BASIC_USER/PASS` 后管理页面走 HTTP Basic，**REST API 也在同一个 HTTP 服务下**
  （即 YekiTrim 调 API 要带同一组 Basic 凭据）——集成时以 `/swagger/` 为准核一次。
- 2.11.0 起**没设密码时会拒绝看起来不是局域网来源的请求**；不要把 2356 直接开到公网。
- 录播姬里把 webhook 指向 YekiTrim：`WebHookUrlsV2 = http://<YekiTrim>:26320/api/ingest/bililive/<token>`
  （token 用 `yekitrim ingest token` 生成，明文只显示一次）。
- 只想单机用、懒得装 Docker：用**桌面版**（Windows 双击即用），API/端口/webhook 配置项一致。

## 3. 录播姬的 REST API（来自其源码 `BililiveRecorder.Web/Api/RoomController.cs`）

| 方法 | 路径 | 用途 |
|---|---|---|
| GET | `/api/room` | 列出所有房间（含 `streaming` / `recording` / `autoRecord`） |
| **POST** | **`/api/room`** | **加房间**：body `{"roomId": 123, "autoRecord": true}` ← 用户填直播间就是打这个 |
| DELETE | `/api/room/{roomId}` | 删房间（不删已录文件） |
| GET | `/api/room/{roomId}` | 单房间状态（开播/录制/弹幕连接） |
| POST | `/api/room/{roomId}/start` / `/stop` | 手动起停录制 |
| GET/POST | `/api/room/{roomId}/config` | 房间级配置 |
| GET | `/swagger/` · `/swagger/brec/swagger.json` | 自带 OpenAPI，集成时按它校准 |

`roomId` 就是直播间号（`https://live.bilibili.com/123` → `123`），长短号都接受（录播姬自己会解析）。

## 4. 数据与接口

**不新增表**：`channel_map(room_id, user_id, auto_pipeline, enabled)` 本来就是"房间 → 归属人"，
用户加直播间 = 往它写一行 + 调录播姬加房间。只加两列：

- `title`（缓存的房间标题，列表里好看）
- `source`（`user` / `admin`，区分"用户自己加的"和"管理员兜底指派的"）

**配置**（`[ingest.recorder]` 增补，`yekitrim config`/设置页可改）：

```toml
[ingest.recorder]
root = ""                    # 录播姬录像目录（已有）
api_base = ""                # 例：http://127.0.0.1:2356（空 = 未对接，用户端加房间走"待分配"兜底）
api_user = ""                # HTTP Basic 用户名（录播姬 BREC_HTTP_BASIC_USER）
api_pass = ""
```

**HTTP（均为本人作用域，admin 可加 `?all=1`）**：

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/rooms` | 我的直播间 + 实时状态（联录播姬查 `/api/room`，失败时降级为"状态不可用"） |
| POST | `/api/rooms` | `{url 或 room_id}` → 解析房间号 → 录播姬 `POST /api/room` → 写 `channel_map` → 返回该行 |
| DELETE | `/api/rooms/:room_id` | 删 `channel_map` 行 + 录播姬 `DELETE /api/room/{id}`（录像文件保留） |
| PUT | `/api/rooms/:room_id` | `{enabled, auto_pipeline}`（暂停/恢复录制 → 同步录播姬的 `autoRecord`） |
| POST | `/api/admin/recorder/test` | 连通性 + 版本：`GET {api_base}/api/version`、`GET /api/room` 各一次 |
| POST | `/api/admin/recorder/sync` | 把录播姬里存在但没登记的房间列出来（进"待分配池"兜底） |

**webhook 链路不变**（已实现）：验 token → 去重 → 会话聚合 → 完结判定 → 入库 →
`channel_map` 查归属 → 建项目 → 按 `auto_pipeline` 跑管线。唯一变化是**归属几乎总是有值**
（用户自己加的），`unassigned` 只在"录播姬里手动加了房间"时出现。

## 5. 前端

**首页 →「录播姬录制」来源**（重写这块，取代现在的待分配池 + 指派表格）：

```
┌ 录播姬录制 ────────────────────────────────┐
│ [ 粘贴直播间号或链接            ] [ 添加 ]  │   ← 用户唯一要做的
│ 123456 · 某主播            ● 在播 · 录制中  │   ← 状态来自录播姬
│  最近：2026-09-14 20:00 入库 · 3 个片段  删除│
│ 789012 · 另一个            ○ 未开播        │
│ [管理页] [扫描录制目录]                      │   ← admin 才有的两个按钮
└────────────────────────────────────────────┘
```

- 用户：只有输入框 + 自己的房间列表（状态 / 最近入库 / 删除）。**没有**"指派给谁"。
- 管理员：多半行「录播姬：已连接 v2.x / 未配置（去设置）」（`/api/admin/recorder/test` 的结果）
  + 「打开录播姬管理页」（`api_base` 新窗口）+ 「扫描录制目录」（历史/漏推送兜底）。
- 未对接录播姬（`api_base` 为空）时：输入框变成只读提示"管理员还没对接录播姬"，
  并把当前页面降级成"待分配池"的只读列表，别给一个填了报错的框。

**设置页 → 新增「录播姬」卡（admin）**：`api_base` / `api_user` / `api_pass`（口令不进表单回显，
留空 = 不改）+「测试连接」+ webhook token 一键生成/查看指引 + 录像目录 `root`。

**侧栏「入库」入口已按你的要求删掉**；`#/ingest` 页降级为 admin 诊断页（记录 / 房间映射 / 扫描），
入口挪到设置页的「录播姬」卡里，普通用户看不到。

## 6. 迁移与兼容

- 老数据（`channel_map` 里 admin 指派的房间）原样保留，`source='admin'` 回填。
- `unassigned` 兜底池、`/api/admin/ingest/assign`、扫描目录**都不删**：它们是"录播姬里先有房间、
  YekiTrim 后知道"的补录路径，也是录播姬挂掉/换机时的恢复手段。
- `[ingest.recorder] api_base` 为空时，功能整体退回现在的行为（不会把界面弄成半坏状态）。

## 7. 实施顺序（每步都能单独用）

1. 配置 + `RecorderClient`（reqwest + Basic）+ `POST /api/admin/recorder/test` ← 先把"能不能连上"跑通。
2. `GET/POST/DELETE /api/rooms` + `channel_map` 两列（迁移 v5）。
3. 首页「录播姬录制」块重写 + 设置页「录播姬」卡。
4. `#/ingest` 降级为 admin 诊断 + 入口搬迁。
