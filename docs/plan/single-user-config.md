# 单人优先：程序配置收敛到「一个 owner 一个位置」

> 状态：P0/P1 **已实施并验证**（2026-11-13）｜ 决策：方案 A（顶层 `config.toml` 退役）｜ 多人开关：`config.toml` 的 `[auth] multi_user`
> 触发：用户报「LLM 已配置，切片页仍显示『未配置 AI』、智能剪辑置灰」（已单独修掉 `health.llm`，见 §1.3）

## 1. 问题

### 1.1 症状

一处配置填好、另一处说没配：

| 界面 / 能力 | 表现 |
|---|---|
| 切片页「智能剪辑 / 重新分段」 | 「未配置 AI」置灰（`/api/health.llm === false`） |
| 事件检测 | 不调 LLM（没有 key），退化成"一个整段" |
| 设置 → 处理（参考标题等） | 保存成功，但**管线不读它** |
| 设置 → 接入（webhook / 录播姬 / biliLive-tools） | 能用，但读的是另一份文件 |

### 1.2 根因：配置有两个位置，各自只被一半消费者读到

```
<数据>/.yekitrim/{config,llm}.toml          ← 顶层（单人时代）
<数据>/users/<uid>/.yekitrim/{config,llm}.toml  ← 账号树（2026-10-10 起设置页写这里）
```

两份由**同一个模板**生成 ⇒ **都带全部 section** ⇒ 一半是真值、一半是默认值，而且没有任何报错。

读取方分裂（2026-11-13 实测）：

| 读取方 | 走哪份 | 结果 |
|---|---|---|
| `ensure_global_config` / `PUT /api/admin/ingest` / `[ingest] [storage] [publish] [auth]` 的全部消费者 | 顶层 | 接入选顶层 ⇒ 能用 |
| `PUT /config/global`（设置页「处理 / 处理数值」） | 账号树 | 写账号树 |
| `load_config_at()`（**管线**：转写/检测/剪切/订正） | 顶层 `config.toml` + 顶层 `llm.toml` | **处理页设置不生效；LLM 无 key ⇒ 检测不调模型** |
| `/api/health.llm`（修前） | 顶层，且**不加载 `llm.toml`** | 恒 `false` |
| `/polish/llm*`、封面标题 | 账号树（已按 owner 修过） | 能用 |

用户机器上的实证（顶层 vs 账号那份）：

- `detection.reference_titles`：顶层 `[]`、账号那份是用户配的 6 条 ⇒ **检测一直没用用户标题**；
- `ingest.recorder.*` / `token_hash` / `proxy`：只在顶层 ⇒ 接入能用；
- `llm.toml`：顶层 key 空、账号那份有 key。

### 1.3 已修的那一条

`/api/health.llm` 改为 `optional_owner(会话) + load_llm_config_for(owner) + has_api_key()`
（`crates/yekitrim-server/src/api/{mod.rs,auth.rs}`，判据 `auth::tests::optional_owner_reads_the_session_account`）。
它只是这一族的**第一个症状**，管线侧未修。

## 2. 决策（2026-11-13 用户拍板）

1. **方案 A**：程序配置**只有一个位置** = `owner::state_dir(Owner::Local)` = 当前管理员的 `users/<uid>/.yekitrim/`；
   顶层 `<数据>/.yekitrim/{config,llm}.toml` **退役**（一次性迁移后改名留档，不再生成）。
2. webhook / 采集 = **软件设置**，不做单独调用：接入页写的文件与运行时读的文件**必须是同一份**。
3. **单人优先**：多人相关 UI（邀请码 / 账号配额 / 注册入口 / 任务指派对象）默认隐藏；代码保留，`[auth] multi_user = true` 时恢复。
4. 多人开关放 **`config.toml` 的 `[auth]`**（不是 `server.toml`），前端经 `/api/health.multi_user` 读。
5. 「字幕订正」保持隐藏（`LlmLink::ui_visible()` 不含 `correct`），本次不动它的开关。

## 3. 目标形态

```
<数据>/
├── .yekitrim/          账本 + server.toml + bin/models/logs（账号无关；不再有 config.toml / llm.toml）
├── global/resources/   公用素材库（账号无关）
└── users/<uid>/.yekitrim/
    ├── config.toml     ← 唯一程序配置（含 [ingest] [storage] [publish] [auth]）
    └── llm.toml        ← 唯一 LLM 配置
```

- 单人：`Owner::Local` → 首个管理员（server 启动 / CLI 启动都可解析）；
- 多人：`Owner::User(uid)` → 该账号；**平台级设置 = 管理员那份**（与 `PUT /api/admin/ingest` 的 admin-only 一致）。

## 4. 改动清单

### 4.1 P0（后端，本次）

| # | 改动 | 落点 |
|---|---|---|
| 1 | 新增 `config::global_config_path()` = `owner::config_path(Owner::from_env())` | `crates/yekitrim-core/src/config/mod.rs` |
| 2 | `load_global_config()` 改走 `load_config_for(Owner::from_env())`（不再读顶层） | 同上 |
| 3 | `load_config_at()` 的全局层与 `config.llm` 改走 owner（**管线因此读到处理页设置与账号 llm.toml**） | 同上 |
| 4 | `load_config()` 的 CLI 语义不变，但依赖 `Local` 已解析 | 同上 |
| 5 | `ensure_global_config` / `init_global_config` / `global_config_path` 写到 owner 路径；不再生成顶层 | `crates/yekitrim-service/src/init_config.rs` |
| 6 | 接入页写入改 `config::global_config_path()` | `crates/yekitrim-server/src/ingest.rs` |
| 7 | `[auth] multi_user`（默认 false）+ 模板 + `merge()` 条件 | `crates/yekitrim-core/src/config/{types.rs,mod.rs}` |
| 8 | `/api/health.multi_user` | `crates/yekitrim-server/src/api/mod.rs` |
| 9 | 启动迁移（幂等、备份、fail-soft） | `crates/yekitrim-core/src/config/mod.rs::migrate_top_level_config_to` + `crates/yekitrim-server/src/lib.rs` |
| 10 | CLI/服务侧补 `set_local_admin`（账本存在时） | `crates/yekitrim-cli/src/main.rs` |

**迁移胜负表**（顶层 → `users/<admin>/`；幂等，跑两遍是 no-op）：

| section | 谁赢 | 理由 |
|---|---|---|
| `[ingest]` `[storage]` `[publish]` `[auth]` | **顶层赢**（整段覆盖） | 运行时一直读顶层：webhook_enabled / token_hash / 代理 / passkey 都在那儿 |
| `[asr]` `[detection]` `[cutting]` `[correct]` `[audio]` `[project]` `[sfx]` | **账号赢**；只补账号没有的键 | 设置页一直写账号那份（`reference_titles` 6 条在这边） |
| `llm.toml` | **账号赢**；只补缺失键 | 账号那份有真 key |
| 顶层文件 | 迁移后 rename 成 `config.toml.migrated-<日期>.bak` / `llm.toml.migrated-<日期>.bak` | 可回滚、不删用户数据 |

约束：迁移**不得**阻止启动（只读安装 / 跨盘 / 坏文件 ⇒ warn 并保持原状）；注释用 `toml_edit` 保留。

### 4.2 P1（前端，单人优先）

| # | 改动 | 落点 |
|---|---|---|
| 1 | `health.multi_user` 缓存与 `multiUser()` / `multiUserKnown()` | `webui/src/stores/health.js` |
| 2 | 设置「系统」页的 `InviteManager` / `UserQuotaManager` 仅在多人时渲染 | `webui/src/views/SettingsView.vue` |
| 3 | 登录页「用邀请码注册」仅在多人时露出 | `webui/src/views/LoginView.vue` |
| 4 | 子页标记 `multiOnly`，导航过滤（与 `localHidden` 同一套机制） | `webui/src/utils.js` |
| 5 | 重建 `webui/dist` + 跑 `webui/scripts/check-*.mjs` | `npm --prefix webui run build` |

**隐藏 ≠ 删除**：接口（`/api/admin/users`、`/api/admin/invites`…）与 CLI（`yekitrim user invite`）全部保留。

### 4.3 P2（开多人时再做，本次不做）

- 把仍在用平台 loader 读**账号级**字段的 4 处改成按会话 owner：`doctor` 的 `asr`、`sfx.blacklist`、
  `detection.danmaku_window_secs`、`cover-title` 的参考标题（后者已改）。
- 接入页在多人下的 owner 语义（谁能改 webhook）。
- 管线跑在 server 进程 ⇒ 多人时读的是**管理员**的处理配置（已知限制，由「多人默认隐藏」挡住）。

## 5. 验收

P0：

1. 修改「接入」后 `yekitrim_core::config::load_global_config().ingest` 立刻反映新值；
2. 修改「处理 → 参考标题」后管线日志里出现该标题（`resolve_reference_titles` 取到 6 条）；
3. 重启 server 后 `<数据>/.yekitrim/` **不再长出** `config.toml`（只有 `.migrated-*.bak`）；
4. 迁移跑两遍是 no-op（单测钉住）；坏文件/只读目录只 warn；
5. `cargo test -p yekitrim-core --lib config` / `-p yekitrim-service` / `-p yekitrim-server` 全绿；
6. `python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py` 全绿。

P1：

7. 单人安装：设置「系统」页看不到邀请码/配额；登录页没有注册入口；
8. `[auth] multi_user = true` 后两者原样回来；
9. `npm --prefix webui run build` 成功，`webui/dist` 一并更新。

## 6. 回滚

1. 迁移是 rename 而非删除：`config.toml.migrated-<日期>.bak` 改回 `config.toml` 即回到旧形态；
2. 代码侧回滚点：`load_global_config()` / `load_config_at()` / `global_config_path()` 三个函数各自一行；
3. `[auth] multi_user` 默认 false，不影响旧行为。

---

## 7. 实施记录（2026-11-13）

### 7.1 落地清单

| 文件 | 改动 |
|---|---|
| `crates/yekitrim-core/src/config/mod.rs` | 新增 `global_config_path()`；`load_global_config()` / `load_config_at()` / `load_llm_config()` 改 owner；新增 `merge_top_level_into`（纯函数）+ `migrate_top_level_config_to` + `migration_archive_path`；`config_migration_tests` 6 条 |
| `crates/yekitrim-core/src/config/types.rs` | `AuthConfig::multi_user`（默认 false）+ `Default` |
| `crates/yekitrim-core/src/config/mod.rs`（模板） | 模板 `[auth]` 加 `multi_user = false` 注释行；`merge()` **只认 true** |
| `crates/yekitrim-service/src/init_config.rs` | `global_config_path()` 转调 core；注释改写 |
| `crates/yekitrim-service/src/{ingest/mod.rs,invite.rs}` | 两处私有 `global_config_path` 转调 core |
| `crates/yekitrim-server/src/ingest.rs` | `write_global_config` 写 owner 路径 |
| `crates/yekitrim-server/src/lib.rs` | `install_local_admin` 回 `Option<uid>`；启动调迁移（在 `ensure_global_config` 之前） |
| `crates/yekitrim-server/src/api/mod.rs` | `/api/health` 加 `multi_user`；`llm` 位上一轮已按账号 |
| `crates/yekitrim-cli/src/main.rs` | 启动 `install_local_admin_from_db()`；`config migrate` 目标改 owner 路径 |
| `webui/src/{stores/health.js,router.js,utils.js}` + `views/{SettingsView,LoginView,IngestView}.vue` | `multiUser()` 缓存、首帧守卫、单人隐藏邀请码/配额/注册/入库指派（含直敲 `#/register` 拦截） |
| `webui/dist/**` | 重建（指纹 `ab11d40c415563a5`） |
| 卡与文档 | `crates/yekitrim-core/AGENTS.md`、`core/src/config/AGENTS.md`、`crates/yekitrim-service/AGENTS.md`、`crates/yekitrim-server/AGENTS.md`、`webui/src/AGENTS.md`、`docs/file-layout.md` |

### 7.2 验证结果

- `cargo test -p yekitrim-core --lib config`：**57 通过**（含迁移 6 条 + owner 读取判据）。
- `cargo test -p yekitrim-core --lib`：**292 通过**（全量，连跑 **5 次**均绿）。
  ⚠️ 期间抓到并修掉一条**我自己引入的**偶发红：两个改 `YEKITRIM_DATA_DIR` 的用例各声明了一把
  `static LOCK` —— 两个不同的 Mutex 等于没锁，并行跑时互相把环境变量指到对方目录。
  现已收口成文件级唯一一把 `config::DATA_DIR_ENV_LOCK`（第三次红、五次绿）。
- `cargo test -p yekitrim-service`：**138 通过**；`cargo test -p yekitrim-server`：**250 通过**。
- `cargo check --workspace --all-targets`：0 warning。
- `python3 scripts/check_module_map.py` / `check_module_boundaries.py`（19 条）：全过。
- `npm --prefix webui run build`：成功；`check-overlay` 等 16 个脚本通过（2 个因缺外部依赖 SKIP）。
- **真实数据端到端**（复制本机 `yekitrim-lan` 的两份配置到临时数据目录）：
  - 首次启动日志：`程序配置已收敛到 users/1/：2 份顶层旧文件已合并并留档`；
  - 合并结果：`webhook_enabled=true` / `recorder.kind=bililive-tools` / `token_hash` / `proxy` / `publish.*`
    全部从顶层保留；`detection.reference_titles` = **账号那份的 6 条**；`llm.toml` 的真 key 未被空串覆盖；
  - 第二次启动：**无迁移日志**（幂等），顶层不再长出 `config.toml`；
  - `/api/health`：`llm=true`、`multi_user=false`、`ingest_ready=true`（顶层文件已消失、值仍在）。
- 前端实测（headless Chromium）：单人下登录页无注册入口、系统页无邀请码/配额（用量与日志仍在）；
  `[auth] multi_user = true` 重启后全部恢复；直敲 `#/register` 在单人下被守卫挡回登录页。

### 7.3 遗留（P2，开多人时再做）

- `/api/auth/register` 服务端**没有**加 `multi_user` 门（现在只靠 UI 隐藏 + `registration_enabled`
  默认 false）。真要单人彻底关门，在注册 handler 里加一条 `!multi_user → 403` 即可。
- 仍在用平台 loader 读**账号级**字段的 4 处（`doctor.asr` / `sfx.blacklist` / `danmaku_window_secs` /
  精修侧读配置），多人开启前需要改按会话 owner。
- 管线跑在 server 进程 ⇒ 多人时读管理员那份处理配置（已知限制）。
