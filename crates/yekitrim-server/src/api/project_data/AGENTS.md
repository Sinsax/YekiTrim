# yekitrim-server/src/api/project_data — 项目数据面（7 个文件）

> 范围：`crates/yekitrim-server/src/api/project_data`/** ｜ 上游：`api/routes.rs`（全部端点）、`api/test_support.rs`（`data_transcript_put_impl`）｜ 验证：`cargo test -p yekitrim-server`

## 职责

`/api/project-data/*` 与 `/api/fs/list`、`/api/media/*` 的全部 handler。这个目录原来是 3,570 行的
`project_data.rs`（31 个 handler，整改 P4 T18 拆开）：

| 文件 | 语义 |
|---|---|
| `status` | 状态 / 事件·片段列表入口 / 弹幕与搜索文本 / 程序与工程配置 |
| `events` | events.json 的增删改、逐条与一键剪切、管线入口 |
| `clips` | clips/ 目录：列表、缩略图、发布标题、删除、屏蔽轨道 |
| `assets` | 素材清单 `[project.materials]` 与转写稿读写 |
| `fs` | `POST /api/fs/list`（服务器文件浏览，**仅 admin**）|
| `media` | `/api/media/*path` 文件出口 |
| `mod` | 共享底座（跨文件 helper）+ 给路由表的显式再导出 |

## 入口

- **对外面只有 `mod.rs`**：`pub(crate) use` 出来的是路由表用的 handler；域内 helper 一律 `pub(super)`
  （**结构体字段同理**），跨子模块要用的也是 `pub(super)` + 显式 `use super::<域>::<名字>`。
- `data_transcript_put_impl` 是 `pub(crate)`（唯一的域外调用方是 `api/test_support.rs` 的夹具，
  拆分后必须显式说明，别当成疏忽）。
- 端点清单见 `api/AGENTS.md` 与 `docs/server-api.md` §①；任务档位/资源键见 `docs/job-queue.md`。

## 边界

- ⚠️ **模块之间只许显式 `use`，不许 glob**（`use super::{…::*}` / `pub use x::*`）：拆分前本目录与
  `polish.rs` 互相 glob 引用形成模块环。现在唯一允许的跨域边是 `use crate::api::polish::polish_projects_impl`
  （本目录 → polish，**单向**）；别反向引用它。
- ⚠️ **同步 IO 一律下 blocking 池**：读 `events.json` / `clip.json` / 字幕（网络上可能几百毫秒）都不许留
  在 worker 上；需要「async 判定 + 同步 IO」混用（删片段先问 `busy_with`）用 `blocking(move || block_on_here(…))`。
- ⚠️ **写 events.json / publish_config.json 一律走 `yekitrim_core::filelock::lock_path`**：Cutter 与 API 都会写；
  无锁的 read-modify-write 会静默丢事件（`data_events_*_impl` 与 `clip_title_put_impl` 都是这条）。
- ⚠️ **片段目录名 = 身份**：路径参数过 `resolve_clip_dir`（内部 `is_safe_component` 挡 `..`/分隔符），
  删除/改标题都只认 `clips/` 下的单个目录名 —— 别接受任意路径。
- ⚠️ **`/api/media/*` 走 `jail_existing` + `serve_file_from_headers`**（文件出口只有一份实现）；
  `/api/fs/list` 只读、仅 admin，**永不接受上传**。
- ⚠️ 子模块里的 `super::` 是 `crate::api::project_data`，不是 crate 根：跨层的写 `crate::auth` /
  `crate::media` / `crate::model`，或绝对路径 `crate::api::…`。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-server
```

拆分时 39 条用例一条没删，跟着被测函数落在各文件的 `#[cfg(test)] mod tests` 里
（删段目录 + 索引摘除、命中率/同步 IO、事件并段、素材清单、媒体 Range/ETag、字幕读写）。
