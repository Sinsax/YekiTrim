# ingest — 采集入库：容量预检 / 项目落地 / 录播姬 CLI

> 范围：`crates/vtrim-service/src/ingest`/** ｜ 上游：`vtrim-server` 的 `ingest_worker` 与 `download_worker`、`vtrim ingest` CLI ｜ 验证：`cargo test -p vtrim-service ingest`

## 职责

录播姬采集（P3）与 B站下载（P4）**共用的入库前半段**：容量预检与"把一份素材落成一个项目"。
这两件历史上长在 `server::ingest_worker` 里，但它们**与录播姬无关**，所以下沉到这里
（见 `docs/plan/p4-implementation-plan.md` §2）。`mod.rs` 还是 `vtrim ingest token` / `ingest status`
两条 CLI 的落点。

## 入口

- `capacity::{reserve_bytes, watermark_rejects, check_capacity}`（**纯函数优先**：前两个可用确定性的
  `(avail, total, pct, incoming)` 复现线上场景，不受跑测试那台机器当下剩余空间影响）。
- `landing::{ProjectLanding, LandedProject, land_project}`——`ProjectLanding` 字段刻意保持"纯数据"
  （没有 `AppState`、没有 config 引用），任何来源都能复用、也能离线测。
- `mod::{generate_token, cmd_token, cmd_status}`；`ingest::patch_ingest`（私有，走
  `vtrim_core::config::patch_config_file` 深合并写 `[ingest]`）。

## 边界

- ⚠️ 水位保留量**必须钳在 512 MiB ~ 8 GiB**（`reserve_bytes` 的 `RESERVE_MIN_BYTES` /
  `RESERVE_MAX_BYTES`）—— 纯比例在大容量数据盘上会把几 MB 的入库判死。
- ⚠️ 水位判定必须**带上本次占用**：这一档曾经只看"可用 / 整盘比例"、与 `incoming` 完全无关，
  于是在 931.5 GB 的盘上要求保留 93.2 GB，"可用 80.7 GB"连一个 **8.8 MB** 的入库都拒，
  而提示还让用户"清理磁盘"—— 那条线永远清不到。现在两处修正：保留量有绝对封顶，
  且判定是"**写完这一份之后**剩余空间跌破保留底线才拒绝"（`watermark_rejects`）。
- 纪律：**不足一律报错，绝不自动删任何用户数据**。
- `check_capacity` 三档依次判定、**都通过才放行**：① 每用户配额（`users.quota_bytes` 优先，
  其次 `[storage] default_quota_bytes`）② 全平台磁盘上限 `[storage] max_disk_bytes`（0 = 不限）
  ③ 目标磁盘写完这一份之后的剩余空间。阈值全部来自传入的 `cfg`，**不在这里重新读全局配置**
  （调用方拿的是同一份快照，测试也能构造确定性阈值）。`pct == 0` = **关闭**水位检查（历史约定）。
  `incoming` 是**预计占用**：取不到准确值时宁可给大一点 —— 误拒的代价是"用户重试一次"，
  放过的代价是把磁盘写满。
- `landing` 的职责边界**刻意划得很窄**，它**只做登记**，不做：
  ❌ 搬文件 / 合并分片（录播姬的多分片合并是它自己的事）❌ 容量预检（那是 `capacity`）
  ❌ 起管线（起管线要 `jobs` + `run_stages`，在 `vtrim_server::pipeline_boot`）。
  顺序上调用方应当是：**容量预检 → 素材就位 → `land_project` → 起管线**。
- ⚠️ **幂等**：同一个 `vtrim.toml` 重复落位**不会建出第二个项目**（先按 `config_file` 查，命中就复用）。
  这一条是"重试"能安全使用的前提：下载重试 / 录播姬重投都靠它。
- ⚠️ `ProjectLanding.source_kind` 同时是**预载策略的键**（`vtrim_service::assets::preload_policy`），
  写错不会报错 —— 只会让弹幕悄悄不预载、热度排序变差。
- ⚠️ `ProjectLanding.room` 写进 `projects.room`（**直播间号**），B站来源**必须留 `None`**：
  这一列会被原样暴露给前端（`summary_json` 的 `"room"`、预处理页的 `source.room`）当"房间"展示，
  塞 bvid 进去是误导。B站的来源标识走 `ingest_sessions.source_ref` + 项目名/标签。
- ⚠️ **两个 worker 之间不许互相调用**：共享的东西一律下沉到本模块，否则录播姬那套
  「待分配池 / `is_finalized` 空闲窗口」的语义会顺着调用链渗进 B站通道。
  本模块的消费者就是 `vtrim-server` 的两个 worker（`ingest_worker` / `download_worker`）。
- `land_project` 用 `vtrim_core::config::write_project_config_exact` 而不是会自动发现同名 `.ass` 的那个：
  这里两侧的来源**都已经自己探测过弹幕**了（录播姬 `find_danmaku` / B站下载），
  再让配置层去猜一遍只会多一层看不见的副作用。

### 会话账本（`ingest_sessions`，与 B站共用）

- B站会话与录播姬**共用 `ingest_sessions`**，靠 v8 新增的 `kind` 列区分（`recorder` | `bilibili`）；
  `source_ref` 放来源侧标识（bvid，多P 形如 `BV1xx-p3`）。
  ⚠️ **别拿 `room_id` 塞 bvid** —— `room_id` 会被 `channel_map` 按房间号查的规则命中，
  把 B站会话误判成某个直播间。⚠️ `projects.room` 也**不能**塞 bvid。
- ⚠️ **会话状态机只有 `closing` 会被入库**（`ingest_worker::is_finalized()` 只认它）：
  - **指派/导入必须把待分配会话推到 `closing`**（`Db::assign_ingest_session` 里的
    `CASE WHEN state='unassigned' THEN 'closing'`）。曾经写成 `'finalized'` → 「指派并入库」
    只改归属、**永远不进库**（实测停在 finalized、`project_id` 恒空，得再点「重试」）。
  - `'finalized'` = **项目已建好**（`Db::set_ingest_project` 写），别当"已指派"用。
  - 扫描（`register_scan_session`）每两轮就会重走一遍，**不能把已有归属的会话打回 `unassigned`**
    （那会和 assign 来回震荡，手动指派过的会话被反复踢回待分配池）→ 改状态前先看 `owner_id`。

### `vtrim ingest` CLI

- `cmd_token`：生成不可猜测的 token（`vtrim_` + 40 位 base64url），**明文只显示这一次**，
  配置里只存 scrypt 哈希（`[ingest] token_hash`）并开启 `webhook_enabled`；
  `--disable` 只关开关、**保留 token**。打印的 webhook 地址用 `crate::doctor::DEFAULT_PORT`
  （**别写端口字面量**）。
- `cmd_status`：只读概览，**不打印任何秘密**。
- ⚠️ 「明文只见一次」（本命令）与「邀请码明文落库、可重复查看」（`invite`）是**两条相反的规矩**，
  别互相抄；见 `crates/vtrim-service/AGENTS.md`。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p vtrim-service ingest
```

容量预检与落位的单测（含"水位必须带上 `incoming`"与钳制边界）。改了 `[storage]` 阈值语义或
`ingest_sessions` 状态机，再跑 `python3 scripts/check_module_boundaries.py`。
