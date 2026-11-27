# 目录结构（程序放在哪、数据放在哪）

> 口径（2026-11-27 定稿）：**安装目录只有两个根** —— `app/`（只读程序件）与 `data/`
> （全部可写状态）。没有 `.yekitrim/`、没有 `cloud/`、没有 `global/resources/`、
> 没有顶层 `resources/`。改路径解析之前先读这份。

## 两个根

```
<程序目录>/                      # exe 所在目录（debug = target/debug/）
├── yekitrim.exe
├── yekitrim-tray.exe
├── yekitrim.ico
├── LICENSE
├── THIRD-PARTY-NOTICES.md
├── RELEASE-NOTES.md
├── app/                        # 只读；随版本发布，升级整目录替换
│   ├── runtime/
│   │   ├── polish-to-dhampir.mjs
│   │   └── effects.json
│   └── dhampir/
│       ├── bin/dhampir.exe
│       ├── preview/{engine.js,pkg/*}
│       └── VERSION
└── data/                       # 全部可写；备份/换盘只搬它
    ├── layout.json             # {"version":2} 迁移标记，别删
    ├── server.toml             # 当前工程 + 端口
    ├── ledger/                 # 账本与运行锁
    │   ├── yekitrim.db{,-wal,-shm}
    │   ├── yekitrim.lock
    │   ├── initial-admin-password.txt
    │   └── render-procs/
    ├── logs/                   # 唯一日志根
    │   ├── tray.log
    │   ├── yekitrim-YYYYMMDD.log
    │   ├── render-<job>.log
    │   └── llm/                # 原 .yekitrim/llm_debug
    ├── models/{sensevoice,qwen3-asr,silero-vad}
    ├── bin/yt-dlp.exe          # 内置下载器
    ├── projects/               # 托管入库根（按需创建，可整体覆盖）
    │   └── users/<uid>/<room>/<stamp>-<pid>/
    ├── shared/{fonts,stickers,videos}   # 公用素材库
    └── users/<uid>/
        ├── config.toml
        ├── llm.toml
        ├── fonts/
        ├── stickers/
        └── videos/
```

**唯一解析入口**（`crates/yekitrim-core/src/path.rs` 与 `owner.rs`）：

| 资产 | 解析函数 | 落点 |
|---|---|---|
| 程序目录 | `path::program_dir()` | exe 同级 |
| 只读程序件 | `path::app_dir()` | `<程序>/app/` |
| 数据根 | `path::data_dir()` | `YEKITRIM_DATA_DIR` ?? `<程序>/data/` |
| 账本 / 锁 | `path::ledger_dir()` | `<data>/ledger/` |
| 日志 | `path::logs_dir()` | `<data>/logs/` |
| 模型 | `path::models_dir()` | `<data>/models/` |
| yt-dlp | `path::bin_dir()` | `<data>/bin/` |
| 托管项目（默认） | `path::projects_dir()` | `<data>/projects/`（可被 `[storage] projects_root` 覆盖） |
| 公用素材 | `owner::shared_assets_dir()` | `<data>/shared/` |
| 账号根 | `owner::owner_root(owner)` | `<data>/users/<uid>/` |
| 账号配置 | `owner::config_path` / `llm_config_path` | `<data>/users/<uid>/{config,llm}.toml` |
| 账号素材 | `owner::asset_dir(owner, kind)` | `<data>/users/<uid>/{fonts,stickers,videos}/` |
| 转译器 / 词汇表 | `path::runtime_dir()` | `<程序>/app/runtime/` |
| 出片底座 | `path::dhampir_dir()` | `<程序>/app/dhampir/` |

⚠️ **`YEKITRIM_DATA_DIR` 是唯一的口子**：`target/debug` 与 `target/release`（以及换
`CARGO_TARGET_DIR`）各是一份程序目录；想共用 1 GB 级模型就把它指到同一处。它表示
**数据根**，不是账本目录 —— 账本永远在 `<data>/ledger/`。

## 归属：只有一种布局（多用户）

`Owner::Local`（托盘 / CLI / `--local-no-auth`）**就是当前管理员**，与
`Owner::User(admin_uid)` 逐字节相同。业务代码不许自己拼 `<data>/users/...`：

- 用户素材：`<data>/users/<uid>/{fonts,stickers,videos}/`（**写**只有这一处）
- 公用素材：`<data>/shared/{fonts,stickers,videos}/`（**只读**补缺；自带字体落这里）
- 配置：`<data>/users/<uid>/{config,llm}.toml`（`llm.toml` 有明文 key，按账号分）

`owner::asset_dirs(owner, kind)` 固定返回 `[用户库, 公用库]`：**用户库优先、公用补缺**。
写永远只写第一个；读才会看两层。`Owner::Local` 由上层 `set_local_admin(uid)` 注入
（core 不认识账本），默认 1。

## 托管项目（`data/projects/`）

它是默认**托管入库根**，不是配置目录。每个项目目录：

```
<projects_root>/users/<uid>/<room>/<YYYYMMDD-HHMMSS>-<pid>/
├── yekitrim.toml
├── 源录播 + 弹幕
├── output/            # video.wav / transcript.json / events.json / clips/ / 字幕
└── .yekitrim/         # 工程内 checkpoint（与程序数据目录无关）
```

哪些入口会用到它：

| 入口 | 是否落到 `<projects_root>/users/<uid>` |
|---|---|
| B 站链接下载（yt-dlp） | **总是** |
| 录播姬入库 | **否** —— 项目与 `output/` 就建在**录像所在目录**（2026-11-27 起不搬源文件） |
| WebUI「从服务器本机文件新建」 | 否，用用户选的目录；仅管理员 |
| CLI `yekitrim init` | 否，用 `--output` / `./output` |

- `[storage] projects_root` 可整体指到数据盘（如 `E:/yekitrim-projects`）；改配置只影响新项目。
- 非管理员的可访问边界是 `<projects_root>/users/<自己的 uid>`。
- **按需创建**：没有 B 站下载时，`data/projects` 可以不存在（录播入库根本不占它）。
- ⚠️ 录播姬入库计划重做，本次只保证迁移后形状不变。

## 旧位置与迁移（0.5 → v2）

启动时 `yekitrim_core::layout::ensure_layout()` 幂等地把旧树搬成新树（只 `rename`，
目标已存在则跳过 + 告警，绝不覆盖；未知条目进 `data/legacy/`）：

| 0.5 | v2 |
|---|---|
| `.yekitrim/server.toml` | `data/server.toml` |
| `.yekitrim/cloud/` | `data/ledger/` |
| `.yekitrim/models/` `bin/` `projects/` | `data/models/` `bin/` `projects/` |
| `.yekitrim/llm_debug/` | `data/logs/llm/` |
| `.yekitrim/cloud/logs/*` | `data/logs/` |
| `.yekitrim/{config,llm}.toml`（≤0.4） | `data/legacy-config/`（启动后并入 admin） |
| `logs/` | `data/logs/` |
| `global/resources/<kind>/` | `data/shared/<kind>/` |
| `users/<uid>/.yekitrim/*.toml` | `data/users/<uid>/*.toml` |
| `users/<uid>/resources/<kind>/` | `data/users/<uid>/<kind>/` |
| `resources/`（空壳） | 删除 |
| `runtime/` | `app/runtime/` |
| `dhampir/` | `app/dhampir/` |

- 迁移写 `data/layout.json`、`layout-migration-journal-<ts>.json` 与根目录说明文件。
- `yekitrim migrate-layout --root <dir>` 默认 dry-run；`--apply` 执行；`--rollback <journal>` 回滚。
- ⚠️ **迁移后旧 0.5 exe 不可再运行**：它会找不到 `.yekitrim` 并新建一份空状态，看起来像数据丢了。
- 裸换 exe（只替换二进制、旧目录不动）仍可用：`app/runtime`、`app/dhampir` 缺失时分别回落
  `<程序>/runtime`、`<程序>/dhampir`，更老的 `tools/` + `spec/` 也仍被读取。

## 不在这里的东西（有意）

- **工程素材**（录播视频）：默认在 `[storage] projects_root`；用户也可放在任意盘（本机导入）。
- **发行包**：`release/`（gitignored）—— 构建产物，不是运行态。
- **cargo 构建中间物**：`target/debug/{deps,build,incremental,.fingerprint}` —— 与程序资产无关。

## 历史（别重蹈）

1. **用户目录优先**（2026-10 之前）：字体/模型挂 `~/.yekitrim/`，于是同一个东西有两个可能位置，
   实测 CLI 与 WebUI 各看一份字体库、界面上完全看不出来。已整条删除。
2. **`.yekitrim/` + `cloud/` + `global/resources/` + `users/<uid>/resources/`**（2026-10 ~ 2026-11）：
   概念要解释、层级要记忆，且顶层 `resources/` 是纯空壳。2026-11-27 收敛成 `app/` + 扁平 `data/`。
3. ⚠️ **别再退回第二份位置**：`YEKITRIM_DATA_DIR` 已经正面解决"构建目录漂移"，不需要第二个资产根。
