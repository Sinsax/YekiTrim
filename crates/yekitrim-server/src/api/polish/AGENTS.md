# yekitrim-server/src/api/polish — 精修编辑器后端（8 个文件）

> 范围：`crates/yekitrim-server/src/api/polish`/** ｜ 上游：`api/routes.rs`（全部端点）、`api/project_data.rs`（`polish_projects_impl` 快照）｜ 验证：`cargo test -p yekitrim-server`

## 职责

`/api/polish/*` 的全部 handler。这个目录原来是一个 6,235 行的 `polish.rs`（39 个 handler，整改 P4 T16 拆开）：

| 文件 | 语义 |
|---|---|
| `program` | 起任务：init / llm / llm-segment / process / render / cover（都是「校验 → `polish_spawn`」）|
| `project` | 精修工程本体（`polish.toml`）、字幕、情绪轨道 |
| `cover` | 封面：标题生成的一族纯函数 + 配置 / 背景 / 帧 |
| `fonts` | 字体库（按 `Owner` 分库）|
| `assets` | 素材：贴纸 / 视频库 / 音效（目录 / 试听缓存 / 屏蔽）/ emoji / 贴纸帧与预热 |
| `preview` | 预览端口 LRU + 两条预览入口 |
| `dhampir` | 底座出片通道：就绪位 / 工程转译缓存 / wasm 转发 |
| `mod` | 共享底座（成片产物清单 / `polish_spawn` / `scope_pid`）+ 给路由表的显式再导出 |

## 入口

- **对外面只有 `mod.rs`**：`pub(crate) use` 出来的是 `api/routes.rs` 用的 handler 与 `FONT_UPLOAD_MAX_BYTES`；
  域内 helper 一律 `pub(super)`（**结构体字段同理**），跨子模块用不到的不要外露。
- 端点清单与业务口径见 `api/AGENTS.md` 与 `docs/server-api.md` §①；任务档位/资源键见 `docs/job-queue.md`。

## 边界

- ⚠️ **模块之间只许显式 `use`，不许 glob**（`use super::{…::*}` / `pub use x::*`）：拆分前 `polish.rs` 与
  `project_data.rs` 互相 glob 引用形成模块环（Rust 允许、编译不报错、门禁抓不到），「一文件一模块」就此失效。
  现在唯一的跨域边是 `project_data → polish`（`polish_projects_impl`），我这边**不许反向**引用它。
- ⚠️ **同步 IO 一律下 blocking 池**；任务体走 `api::spawn_job`（见 `api/AGENTS.md`「同步 IO 纪律」）。
- ⚠️ **写 `polish.toml` 只走 `yekitrim_polish::project::update_polish_toml`**（路径锁 + 白名单）；别开
  「开头 load、结尾整份写回」的第二条路 —— 那会静默吞掉用户在两分钟 LLM 期间存的选框/封面/屏蔽。
- ⚠️ **`dir` 参数一律过 `crate::api::validated_project_dir`**（空串会在 server 进程 CWD 里写出 polish.toml）。
- ⚠️ **`renderer` 白名单只有 `dhampir`**（`/polish/render`）与 `takumi`（`/polish/cover`，口径在 `cover.rs`）：
  认不出的值回 **400**，不静默当默认；本目录不抄第二份白名单。
- ⚠️ **音效两件（`sfx-preview` / `sfx-blacklist`）**：试听抓源站落 `<data>/cache/sfx/` 再按 Range 供出
  （站方要求不转载音效 URL，**别改成把 URL 直接回给浏览器**）；黑名单**按 `Owner` 读写**、
  走 `patch_config_file` 整份覆盖，别用"读整份 Config 再写回"（会抹掉用户 config.toml 的注释）。
- ⚠️ **子模块里的 `super::` 是 `crate::api::polish`，不是 crate 根**：跨层的写 `crate::auth` / `crate::model`
  或绝对路径 `crate::api::…`（本目录是 `api/polish/`，比 `api/*.rs` 深一层）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-server
```

拆分时 polish 的 38 条用例**一条没删**，跟着被测函数落在各文件的 `#[cfg(test)] mod tests` 里
（工程往返 / edit 白名单 / 封面配置 / 字体两层库 / 贴纸帧与预热 / dhampir 转译与 wasm）。
