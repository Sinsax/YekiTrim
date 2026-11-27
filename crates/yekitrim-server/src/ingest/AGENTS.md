# yekitrim-server/src/ingest — 采集接入（录播姬 webhook / B站链接）

> 范围：`crates/yekitrim-server/src/ingest`/** ｜ 上游：`api/routes.rs`（全部端点）、`api/rooms.rs`（`map_webhook_path` / `owned_room`）｜ 验证：`cargo test -p yekitrim-server`

## 职责

录播姬与 B站两条采集通道的 HTTP 层。**这个目录存在的理由是让两种语义分开**：录播姬是
「无身份 webhook → 待分配池 → 靠空闲窗口完结」，B站是「用户主动提交 → 有身份 → 进程退出即完结」。
它们曾经住在同一个 3,235 行的 `ingest.rs` 里（`handle_event` 与 `bilibili_submit` 相隔 2000 行），
所以「两个 worker 不许互相 import」那条禁令当时是**可以合法绕过的**。

| 文件 | 语义 |
|---|---|
| `webhook` | 录播姬 webhook 的**会话流程**：验 token → 去重 → 累积文件 → 推 `closing` |
| `webhook_event` | payload 解析与归一化（官方 PascalCase / biliLive-tools 两种方言） |
| `webhook_path` | 容器路径 → 本机路径、产物 / 伴生弹幕 / 标题、建工程 |
| `records` | 录制记录、`/rooms/:id/recordings`、单文件导入 |
| `channel_map` | 房间 → 归属映射（待分配池的指派 / 重试 / 兜底扫描） |
| `config` | 采集配置读视图、保存通道、令牌轮换 |
| `patch` | `build_ingest_patch`：PATCH「缺席 vs 空」语义的**唯一实现** |
| `bilibili` | B站链接通道（resolve / submit） |
| `downloader` | yt-dlp 安装与状态 |
| `mod` | 共享底座（`err` / `ok` / `off` / `field*` + 类型别名）与对外再导出 |

## 入口

- **对外面只有 `mod.rs`**：`pub use` 的是 `api/routes.rs` 用的 handler；
  `pub(crate) use records::owned_room` / `pub(crate) use webhook_path::map_webhook_path` 给
  `api/rooms.rs`（这两个的可见性本来就只有 crate，别改成 `pub`）。
- 端点清单：`api/AGENTS.md`「采集配置端点」+ `docs/server-api.md` §①；本条线的设计要求见
  `docs/superpowers/specs/2026-09-12-multi-user-server-design.md` §4。

## 边界

- ⚠️ **`webhook*` 与 `bilibili` 不许出现对方的领域词**（`scripts/check_module_boundaries.py` 的成对判据，
  文件级、剥注释后匹配）：webhook 侧不许有 `bilibili|bvid|yt-dlp`，bilibili 侧不许有 `recorder|room_id|待分配`。
  门禁要防的是**语义汇合**，不只是 import 汇合 —— 判据红了别改规则，改代码（把那一行挪到对的文件里）。
- ⚠️ **会话状态机只有 `closing` 会被入库**（`ingest_worker::is_finalized()` 只认它）：指派 / 导入必须把待分配
  会话推到 `closing`；`'finalized'` = 项目已建好，别当「已指派」用；扫描不能把已有归属的会话打回 `unassigned`。
- ⚠️ **房间号必须从路径里解析**（`room_id_from_dir_name` / `room_id_from_rel_path`），不能拿整个目录名当房间号
  （录播姬默认目录名是 `<房间号>-<主播名>`，而 `channel_map` 存的是 `26320007`）。
- ⚠️ **不要为 B站另开"下载暂存目录"**：那等于把 P4 去掉的「搬入」又加回来；下载器直接写进托管项目目录。
- ⚠️ **B站会话与录播姬共用 `ingest_sessions`**（靠 `kind` 区分）：别拿 `room_id` 塞 bvid（会被
  `channel_map` 按房间号查的规则命中）；`projects.room` 同理。
- ⚠️ **采集配置视图只回 `api_pass_set` 布尔**，绝不回 `api_pass` / `token_hash`；审计只记**键名**（值里可能有口令）。
- ⚠️ `build_ingest_patch` 是 `patch_config_file` 的 PATCH 语义（**键缺席 = 不改 / 空串空数组 = 清空**）的唯一实现，
  别在别处再写一份判断 —— 拆成两处就有一边会把「没写这个键」当成「清空」。
- ⚠️ **录播入库是「原地建项目」**（2026-11-27 用户口径）：项目与 `output/` 建在**录像所在目录**，
  源文件**不搬、不复制、不删**。跨机时那个目录由用户自己映射（SMB/UNC），YekiTrim 只做
  **连接 + 目录校验**。别再往这条链路里加 `copy` / `rename` / `remove_file(src)`。
- ⚠️ **`container_root` 可自动推导**（2026-11-27）：唯一实现 `ingest::derive_container_root`
  （savePath + 历史样本 + 本地磁盘，取「命中最多、同分最浅」）；「测试连接」给建议、`PUT` 保存时自动补齐。别再写第二份前缀猜测。
- ⚠️ **目录校验只有一份** `yekitrim_service::recorder::probe_recorder_root`（判「填了对方机器的 /rec」
  「盘符映射」「不可写」）。缓存走 `ingest::root_probe_cached`（TTL 60s）：
  **`GET /api/admin/ingest` 绝不现场探测**（死 UNC 的 `metadata` 能挂几十秒）；只有
  「测试连接 / 保存配置」两条显式路径会真探测（`spawn_blocking` + 8s 超时）。
- ⚠️ **子模块里的 `super::` 是 `crate::ingest`，不是 crate 根**（与 `api/` 同一条坑）：跨层的写成
  `crate::auth` / `crate::model`；**跨同级模块的项一律 `pub(super)`**（结构体的**字段**也要，
  否则「字段 private」只在另一个文件里报），被两个以上子模块用到的 helper 放 `mod.rs`。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-server
```

`ingest` 的 23 条单测都在各文件的 `#[cfg(test)] mod tests` 里（拆分时一条没删）：webhook 流程 / 解析 /
路径逃逸、`channel_map` 归属与 409、PATCH 缺席 vs 空、配置视图不泄漏、token 轮换、项目根校验。
