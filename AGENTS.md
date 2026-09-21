# V-Trim

直播录像自动切片工具。输入录播视频 + 弹幕文件，输出按事件分割的片段 + 字幕 + `publish_config.json`。

> 范围：`.`（整个仓库）｜ 上游：无（这是根指令；模块级规矩在各模块目录的 `AGENTS.md` 里，清单见「入口」）｜ 验证：`python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py`

## 职责

**这个文件是路由页，不是手册。按模块的规矩在各自目录的 `AGENTS.md` 里。**

- DSH 会在 read/write/edit **触到某个目录**时自动注入那一份 `AGENTS.md`：**动手前先读该目录的卡**
  （表在下面「入口」）。卡里写的是"这个模块是什么、入口在哪、不许碰什么、改完跑什么"。
- 这里只放：全局不变式、模块索引、改代码协议、验证命令。
- ⚠️ 根文件有 **20 KB 硬预算**（`scripts/check_module_map.py` 会卡）。DSH 的工作区指令预算总共
  65536 B，而超预算时它**先整份丢掉较宽的文件** —— 根文件一旦膨胀，全局规矩会连模块索引一起消失
  （拆卡之前根文件 237 KB，被静默截到 65 KB，底下的安全规矩全没进上下文）。**别再往这里加模块级内容**。

### 六个 crate 与依赖方向（单向，硬规矩）

```
vtrim-render-core ←── vtrim-polish ──┐
                                     ├──→ vtrim-service ──→ vtrim-server ──→ vtrim-cli
        vtrim-core ──────────────────┘
```

| crate | 干什么 |
|---|---|
| `crates/vtrim-core` | 管线编排 + ASR / 事件检测 / 剪切 + 配置 / LLM 客户端 / 弹幕 |
| `crates/vtrim-polish` | GSAP 精修引擎（从 vtedit 合并）：字幕/贴纸/音效/特效 → 出片与封面 |
| `crates/vtrim-render-core` | GPU 绘制原语（被 polish / wasm 用），不认识业务 |
| `crates/vtrim-service` | CLI 与 WebUI 共用的领域逻辑（账号账本 / 项目 / 录播姬 / B站 / 用量） |
| `crates/vtrim-server` | axum HTTP API + WebUI 宿主 + 任务队列，`api/` 按领域分模块 |
| `crates/vtrim-cli` | 只剩入口（clap 命令 + tracing 装配），二进制名 `vtrim` |
| `crates/vtrim-wasm` | **不在 workspace members 里**（独立 target），只依赖 `vtrim-render-core` |
| `crates/xtask` | 构建/打包/产物门禁（`cargo xtask scan|package`） |

硬规矩（`scripts/check_module_boundaries.py` 逐条机器检查，14 条规则）：

- `vtrim-service` **绝不能** `use vtrim_server::*`（它能被抽出来的全部理由）；
- `vtrim-server` 只依赖 `vtrim-service`，不碰 `vtrim_cli`；
- `core` / `polish` / `render-core` 不许认识上层；
- CLI 侧要用**服务端能力**，只能经 `vtrim-server` 暴露的 `run()` / `ServerArgs` 进入；
- 前端 `components/` 不许 import `views/`；作用域查询串只许在 `webui/src/api/` 里拼。

README 的「架构」一节有同一张图的展开版；分层决策的来龙去脉见 `docs/plan/service-direction-review.md`。

### 管线：Stage 编号与运行模式

`AudioExtractor(1) → Transcriber(2) → EventDetector(3) → Cutter(4)`，**连续无跳过**。
所有 stage 用 `#[async_trait]` 实现 `Stage` trait（`crates/vtrim-core/src/pipeline/stage.rs`）。

`PipelineRunner` 三种模式（`pipeline/runner.rs`）：`run()`（受文件存在性约束，单 stage 命令用）、
`run_all()`（无视文件检查从头跑，`process` 用）、`run_until(max)`（`process --dry-run` 用）。

`current_stage` **不持久化**，从输出文件存在性推导（`config/types.rs::current_stage`）：

| 文件存在 | 推导阶段 |
|---|---|
| `output/video.wav` 或 `audio_file` 非空 | Stage 1 |
| `output/transcript.json` 或 `video.srt` 或 `transcript_file` 非空 | Stage 2 |
| `output/events.json` 或 `events_file` 非空 | Stage 3 |
| `output/clips/clips_order.txt` 存在 | Stage 4 |

状态机规则：`stage > current + 1` → 拒绝；`stage <= current` → 可重跑；`process` 无视检查全部重跑。

### CLI（命令表面）

```
vtrim init <video> [--danmaku <file>]    # 创建 vtrim.toml
vtrim extract [-i] [-o]                  # 提取音频 (Stage 1)
vtrim process [-i] [-o] [--dry-run]      # 全自动管线
vtrim transcribe [-i]                    # Stage 2 (local)
vtrim detect [-t <transcript>]           # Stage 3 (自动发现: 参数 > config > output_dir/*.json > *.srt)
vtrim cut [-i] [-e <events>] [-o]        # Stage 4 (自动发现: 参数 > config/output_dir/events.json)
vtrim silence [files...] [--duration 3.5] [--threshold]   # 「去静音」: 写 polish.toml，不改视频
vtrim download                           # 下载全部 ASR 模型 → <程序>/.vtrim/models/
vtrim status                             # 查看当前进度
vtrim usage [--days N]                   # LLM 用量（token/缓存命中/耗时，账本 llm_usage）
vtrim doctor [--url <base>]              # 安全与运行状况摘要
vtrim ingest token [--disable] | ingest status            # 录播姬: webhook / 采集配置概览
vtrim user invite|invites|invite-revoke  # 邀请码（明文落库，可重复查看）+ 注册开关
vtrim user list|info|add|passwd|check|disable|init-admin|reset-admin-password
vtrim config init [--path] | config show # 全局配置生成 / 显示
vtrim correct [-t <transcript>]          # LLM 订正转录文本
vtrim polish init|llm|process|render|cover|preview|doctor|test [<dir>]
vtrim polish stickers list|test|scan     # 本地贴纸: 列出 / 测 keyword 匹配 / 生成清单
```

所有 stage 命令的 `-i` 可选，默认从 `vtrim.toml` 的 `input_file` 读取。

## 入口

### 模块索引（唯一权威是 `docs/module-map.json`，本表由门禁校验一致）

<!-- module-index:begin -->
| id | 目录 | 卡 | 什么时候读 |
|---|---|---|---|
| `root` | `.` | `AGENTS.md` | 每次会话都会加载（本文件） |
| `scripts` | `scripts` | `scripts/AGENTS.md` | 改门禁/加规则时 |
| `core` | `crates/vtrim-core` | `crates/vtrim-core/AGENTS.md` | 动 `crates/vtrim-core/` 下的东西之前 |
| `core-config` | `crates/vtrim-core/src/config` | `crates/vtrim-core/src/config/AGENTS.md` | 动 `crates/vtrim-core/src/config/` 下的东西之前 |
| `core-pipeline` | `crates/vtrim-core/src/pipeline` | `crates/vtrim-core/src/pipeline/AGENTS.md` | 动 `crates/vtrim-core/src/pipeline/` 下的东西之前 |
| `core-asr` | `crates/vtrim-core/src/asr` | `crates/vtrim-core/src/asr/AGENTS.md` | 动 `crates/vtrim-core/src/asr/` 下的东西之前 |
| `core-detection` | `crates/vtrim-core/src/detection` | `crates/vtrim-core/src/detection/AGENTS.md` | 动 `crates/vtrim-core/src/detection/` 下的东西之前 |
| `core-danmaku` | `crates/vtrim-core/src/danmaku` | `crates/vtrim-core/src/danmaku/AGENTS.md` | 动 `crates/vtrim-core/src/danmaku/` 下的东西之前 |
| `core-cutting` | `crates/vtrim-core/src/cutting` | `crates/vtrim-core/src/cutting/AGENTS.md` | 动 `crates/vtrim-core/src/cutting/` 下的东西之前 |
| `core-llm` | `crates/vtrim-core/src/llm` | `crates/vtrim-core/src/llm/AGENTS.md` | 动 `crates/vtrim-core/src/llm/` 下的东西之前 |
| `render-core` | `crates/vtrim-render-core` | `crates/vtrim-render-core/AGENTS.md` | 动绘制原语之前 |
| `polish` | `crates/vtrim-polish` | `crates/vtrim-polish/AGENTS.md` | 动 `crates/vtrim-polish/` 下的东西之前 |
| `polish-process` | `crates/vtrim-polish/src/process` | `crates/vtrim-polish/src/process/AGENTS.md` | 动后处理（字幕/段/贴纸/音效）之前 |
| `polish-render` | `crates/vtrim-polish/src/render` | `crates/vtrim-polish/src/render/AGENTS.md` | 动渲染导出/封面之前（子进程链路，出过事故） |
| `polish-templates` | `crates/vtrim-polish/templates` | `crates/vtrim-polish/templates/AGENTS.md` | 改出片模板之前 |
| `polish-project` | `crates/vtrim-polish/src/project` | `crates/vtrim-polish/src/project/AGENTS.md` | 动精修项目配置/去静音之前 |
| `polish-assets` | `crates/vtrim-polish/src/assets` | `crates/vtrim-polish/src/assets/AGENTS.md` | 动贴纸库/音效目录/资产下载之前 |
| `polish-llm` | `crates/vtrim-polish/src/llm` | `crates/vtrim-polish/src/llm/AGENTS.md` | 动精修侧 LLM 之前 |
| `service` | `crates/vtrim-service` | `crates/vtrim-service/AGENTS.md` | 动领域逻辑（账本/项目/账号/用量）之前 |
| `service-ingest` | `crates/vtrim-service/src/ingest` | `crates/vtrim-service/src/ingest/AGENTS.md` | 动采集入库之前 |
| `service-downloader` | `crates/vtrim-service/src/downloader` | `crates/vtrim-service/src/downloader/AGENTS.md` | 动下载器端口/适配器之前 |
| `service-bilibili` | `crates/vtrim-service/src/bilibili` | `crates/vtrim-service/src/bilibili/AGENTS.md` | 动 B站链接通道之前 |
| `service-net` | `crates/vtrim-service/src/net` | `crates/vtrim-service/src/net/AGENTS.md` | 动 HTTP 小工具之前 |
| `server` | `crates/vtrim-server` | `crates/vtrim-server/AGENTS.md` | 动 HTTP/任务队列/宿主之前 |
| `server-api` | `crates/vtrim-server/src/api` | `crates/vtrim-server/src/api/AGENTS.md` | 加接口/改路由之前（含同步 IO 纪律） |
| `cli` | `crates/vtrim-cli` | `crates/vtrim-cli/AGENTS.md` | 加命令/改入口之前 |
| `xtask` | `crates/xtask` | `crates/xtask/AGENTS.md` | 动构建/打包/产物门禁之前 |
| `wasm` | `crates/vtrim-wasm` | `crates/vtrim-wasm/AGENTS.md` | 动 wasm 之前（生成物不许手改） |
| `webui` | `webui` | `webui/AGENTS.md` | 改构建、`dist`、契约脚本时 |
| `webui-src` | `webui/src` | `webui/src/AGENTS.md` | 动前端源码之前（路由/重挂/缓存纪律） |
| `webui-api` | `webui/src/api` | `webui/src/api/AGENTS.md` | 动接口层/作用域之前 |
| `webui-views` | `webui/src/views` | `webui/src/views/AGENTS.md` | 改页面（预处理/切片/精修/设置）之前 |
| `webui-components` | `webui/src/components` | `webui/src/components/AGENTS.md` | 改组件（时间轴/预览/封面设计器）之前 |
| `webui-styles` | `webui/src/styles` | `webui/src/styles/AGENTS.md` | 改样式之前（唯一设计标准） |
| `webui-polish` | `webui/src/polish` | `webui/src/polish/AGENTS.md` | 动精修预览共享模块之前 |
<!-- module-index:end -->

### 改代码协议（每一条都对应一次踩坑）

1. **先读卡**：改哪个目录，先 `read` 那个目录的 `AGENTS.md`（读一次就会触发注入）。
2. **只改卡声明的范围**：卡里写着"改动只允许落在本目录"。跨模块改动 = 先读对方卡，
   并**在提交说明里写明理由**（"顺带改了 X" 是这类项目最容易出事的地方）。
3. **跑该卡的「验证」命令**，再跑两条门禁：
   `python3 scripts/check_module_map.py`（结构/覆盖率/预算）与
   `python3 scripts/check_module_boundaries.py`（分层与前端边界）。
4. **新增目录 = 新增卡**：新建模块目录后要开 `AGENTS.md`、登记进 `docs/module-map.json`、
   加进本文件的索引表 —— 否则 `check_module_map.py` 会报"无主文件"。
5. **别把模块级内容塞进根文件**：根文件超标会让全局规矩整份消失（见「职责」的 ⚠️）。

## 边界

以下是**全局**不变式（模块内的规矩在各自卡里）：

- ⚠️ **同步 IO 绝不能留在 async 线程上**（剪切时"整个服务器连不上"的根因）：handler 里的
  同步文件访问走 `api::blocking(f)`，任务体走 `api::spawn_job`。完整纪律（已覆盖清单、panic 兜底、
  路径锁）在 `crates/vtrim-server/src/api/AGENTS.md`。
- ⚠️ **「取消」必须在批次/窗口边界真正生效**：判据只有 `ProgressReporter::cancelled()`（server 侧由
  `JobManager::cancel_token` 的 `AtomicBool` 供给）。阶段边界由 `PipelineRunner` 查；**批量 LLM 调用
  要自己把探针交给 `llm::Agent::with_cancel(probe)`**（派发每个窗口前 + 每收一个窗口后查，命中即
  `abort_all` 并报「任务已取消」）。只在阶段边界查过一次的批量循环 = 用户点了停止、后台仍把十几个
  窗口跑完（实测 18 窗、十几分钟）；新增批量循环照这里来。
- ⚠️ **产物落盘 = 一次绑定**：`transcript_file` 的登记只有一条路 ——
  `vtrim_service::assets::bind_asr_artifact`（订正稿走 `bind_derived_material`）。
  别再在别处手写 `transcript_file`，见 `crates/vtrim-service/AGENTS.md`。
- ⚠️ **配置改动的四步**（字段 + `merge()` 条件 + 手动 `Default` impl + 模板/注释）见
  `crates/vtrim-core/src/config/AGENTS.md`；`patch_config_file` 是递归深合并，别退回"一层 section"。
- ⚠️ **许可：别把 espeak-ng 或任何 GPL 组件弄回二进制** —— 那会让整个产物重新变成 GPL 组合作品。
  两道防线：`vendor/sherpa-onnx-sys/build.rs` 的占位断言与 `cargo xtask scan`。
  改依赖前先看 `THIRD-PARTY-NOTICES.md`。部署相关的正文（路径面三防线、认证授权、限流配额、
  端口选择、公网清单）在 **`docs/deploy.md`**，改多租户面之前先读它。
- ⚠️ **路径面三防线**（普通用户能读服务器任意文件的历史事故，三处缺一不可）：
  ① `vtrim-server/src/scope.rs::enforce_scope` 的 `EXTRA_PATH_FIELDS`；
  ② `api/config.rs::PROJECT_PATH_KEYS` + `check_project_paths` / `check_material_paths`；
  ③ 消费侧纵深防御（`vtrim-polish/src/cover.rs` 的 `tenant::ensure_within`）。
  细节与"新增路径字段要做什么"在 `crates/vtrim-server/src/api/AGENTS.md`。
- ⚠️ **`webui/dist` 的位置靠 `CARGO_MANIFEST_DIR/../..`**（`vtrim-server/src/web.rs`），按 crate 深度算：
  别把 `vtrim-server` 挪到更深的目录，否则 WebUI 静默 404。`webui/dist` **已入库**，改了
  `webui/src` 必须重建并一起提交。
- ⚠️ **`wgpu-render` feature 的转发链**：使用点全在 `vtrim-server`（`api/` ×3、`model.rs` ×5），
  `vtrim-cli` 与 `vtrim-server` 都只是转发。漏掉转发不报错，只会让 `/api/health` 的
  `wgpu_render` 报 false、GPU 后端静默不编译进去 —— 动构建特性要实测这一条。
- ⚠️ **默认端口 26320 只有一份定义**：`vtrim-service/src/doctor.rs::DEFAULT_PORT`。
  `ServerArgs.port`、`DEFAULT_URL`、`polish preview` 回退都引用它，**别处不许再写端口字面量**。
- ⚠️ **只监听 loopback**；`--allow-remote` 才对外，且**仍用初始密码时拒绝启动**。
  `--trust-proxy` 是 `X-Forwarded-*` 的唯一开关（决定限流键与 cookie 的 `Secure`）。
- ⚠️ **改内建 prompt 要同步用户配置**：`.vtrim/config.toml` 里的检测 prompt 是 `config init`
  填进去的**副本**，改代码默认值对它无效；**每个程序目录各有一份**（debug / release 互不相干）。
- ⚠️ **不许手改生成物**：`webui/src/wasm/`、`crates/vtrim-wasm/src/wasm/`（wasm-bindgen 输出）。

## 验证

```bash
# 结构门禁（本文件与模块卡的预算/覆盖率/骨架/索引一致/⚠️ 下限）
python3 scripts/check_module_map.py
# 边界门禁（14 条：分层方向 + P4 隔离 + 前端）
python3 scripts/check_module_boundaries.py

# 改任何 crate 之后
cargo check --workspace --all-targets          # 0 warnings
cargo test -p vtrim-server                     # HTTP 侧（JobManager / Range / 路由 / 配置 patch）
cargo test -p vtrim-service                    # 领域逻辑（db / 项目 / 容量 / 下载器 / B站）
cargo test -p vtrim-core --lib danmaku         # ASS + XML 解析 + 密度

# 改了前端（webui/src）之后：必须重建 dist
npm --prefix webui run build
node webui/scripts/check-overlay.mjs           # 覆盖层色档（另有 5 个 check-*.mjs，见 webui/AGENTS.md）

cargo run -- status                            # 查看当前进度
```

⚠️ **用 `python3`**：本机 `python` 不在 PATH（文档里的旧写法 `python scripts/...` 跑不起来）。
⚠️ 需要真实网络/B站行为核实时，样本与脚本留在 `.workbuddy/`（已 gitignore）。
