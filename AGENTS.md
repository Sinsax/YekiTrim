# YekiTrim

直播录像自动切片工具。输入录播视频 + 弹幕文件，输出按事件分割的片段 + 字幕 + `publish_config.json`。

> 范围：`.`（整个仓库）｜ 上游：无（这是根指令；模块级规矩在各模块目录的 `AGENTS.md` 里，清单见「入口」）｜ 验证：`python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py`

## 职责

**这个文件是路由页，不是手册。按模块的规矩在各自目录的 `AGENTS.md` 里。**

- DSH 在 read/write/edit **触到某个目录**时自动注入那一份 `AGENTS.md`：**动手前先读该目录的卡**（表在
  「入口」）。这里只放：全局不变式、模块索引、改代码协议、验证命令。
- ⚠️ 根文件有 **20 KB 硬预算**（`scripts/check_module_map.py` 会卡）。DSH 的工作区指令预算总共 65536 B，
  超预算时它**先整份丢掉较宽的文件** —— 根文件一旦膨胀，全局规矩会连模块索引一起消失（拆卡之前根文件
  237 KB，被静默截到 65 KB，底下的安全规矩全没进上下文）。**别再往这里加模块级内容**。

### 七个 crate 与依赖方向（单向，硬规矩）

```
yekitrim-render-core ←── yekitrim-polish ──┐
                                     ├──→ yekitrim-service ──→ yekitrim-server ──→ yekitrim-cli
        yekitrim-core ──────────────────┘
```

| crate | 干什么 |
|---|---|
| `crates/yekitrim-core` | 管线编排 + ASR / 事件检测 / 剪切 + 配置 / 弹幕 |
| `crates/yekitrim-llm` | LLM 传输层（叶子 crate） |
| `crates/yekitrim-polish` | GSAP 精修引擎（从 vtedit 合并）：字幕/贴纸/音效/特效 → 出片与封面 |
| `crates/yekitrim-render-core` | 事件/时间线**类型**与时间线数学（被 polish 用）；**不含任何 GPU 管线** |
| `crates/yekitrim-service` | CLI 与 WebUI 共用的领域逻辑（账本 / 项目 / 录播姬 / B站 / 用量） |
| `crates/yekitrim-server` | axum HTTP API + WebUI 宿主 + 任务队列，`api/` 按领域分模块 |
| `crates/yekitrim-cli` | 只剩入口（clap 命令 + tracing 装配），二进制名 `yekitrim` |
| `crates/xtask` | 构建/打包/产物门禁（`xtask scan|package`） |

硬规矩（`scripts/check_module_boundaries.py` 逐条机器检查，**19 条**：17 行级（新增 llm_is_a_leaf / only_llm_knows_wire）+ 1 成对判据 + 1 卫生）：

- `yekitrim-service` **绝不能** `use yekitrim_server::*`（它能被抽出来的全部理由）；
- `yekitrim-server` 只依赖 `yekitrim-service`，不碰 `yekitrim_cli`；
- `core` / `polish` / `render-core` 不许认识上层；
- CLI 侧要用**服务端能力**，只能经 `yekitrim-server` 暴露的 `run()` / `ServerArgs` 进入；
- 前端 `components/` 不许 import `views/`；作用域查询串只许在 `webui/src/api/` 里拼。

展开版见 README「架构」；分层决策见 `docs/plan/service-direction-review.md`。

### 管线：Stage 编号与运行模式

`AudioExtractor(1) → Transcriber(2) → EventDetector(3) → Cutter(4)`，**连续无跳过**。
所有 stage 用 `#[async_trait]` 实现 `Stage` trait（`crates/yekitrim-core/src/pipeline/stage.rs`）。

`PipelineRunner` 三种模式（`pipeline/runner.rs`）：`run()`（按文件存在性，单 stage 用）、
`run_all()`（从头跑，`process` 用）、`run_until(max)`（`--dry-run` 用）。

`current_stage` **不持久化**，从输出文件存在性推导（`config/types.rs::current_stage`）：

| 文件存在 | 推导阶段 |
|---|---|
| `output/video.wav` 或 `audio_file` 非空 | Stage 1 |
| `output/transcript.json` 或 `video.srt` 或 `transcript_file` 非空 | Stage 2 |
| `output/events.json` 或 `events_file` 非空 | Stage 3 |
| `output/clips/clips_order.txt` 存在 | Stage 4 |

状态机规则：`stage > current + 1` → 拒绝；`stage <= current` → 可重跑；`process` 无视检查全部重跑。

### CLI（命令表面）

**命令面归 `crates/yekitrim-cli/AGENTS.md`** —— 那张表曾在这里重复、占用最坏指令链预算，已挪回它该在的卡。
**别再往这里抄第二份。**

## 入口

### 模块索引（唯一权威是 `docs/module-map.json`，本表由门禁校验一致）

<!-- module-index:begin -->
| id | 目录 | 卡 | 什么时候读 |
|---|---|---|---|
| `root` | `.` | `AGENTS.md` | 每次会话都会加载（本文件） |
| `scripts` | `scripts` | `scripts/AGENTS.md` | 改门禁/加规则时 |
| `tools` | `tools` | `tools/AGENTS.md` | 动转译器之前 |
| `core` | `crates/yekitrim-core` | `crates/yekitrim-core/AGENTS.md` | 动该目录之前 |
| `core-config` | `crates/yekitrim-core/src/config` | `crates/yekitrim-core/src/config/AGENTS.md` | 改配置之前 |
| `core-pipeline` | `crates/yekitrim-core/src/pipeline` | `crates/yekitrim-core/src/pipeline/AGENTS.md` | 改管线之前 |
| `core-asr` | `crates/yekitrim-core/src/asr` | `crates/yekitrim-core/src/asr/AGENTS.md` | 改 ASR 之前 |
| `core-detection` | `crates/yekitrim-core/src/detection` | `crates/yekitrim-core/src/detection/AGENTS.md` | 改事件检测之前 |
| `core-danmaku` | `crates/yekitrim-core/src/danmaku` | `crates/yekitrim-core/src/danmaku/AGENTS.md` | 改弹幕之前 |
| `core-cutting` | `crates/yekitrim-core/src/cutting` | `crates/yekitrim-core/src/cutting/AGENTS.md` | 改剪切之前 |
| `llm` | `crates/yekitrim-llm` | `crates/yekitrim-llm/AGENTS.md` | 改 LLM 协议之前 |
| `render-core` | `crates/yekitrim-render-core` | `crates/yekitrim-render-core/AGENTS.md` | 动绘制原语之前 |
| `polish` | `crates/yekitrim-polish` | `crates/yekitrim-polish/AGENTS.md` | 动该目录之前 |
| `polish-process` | `crates/yekitrim-polish/src/process` | `crates/yekitrim-polish/src/process/AGENTS.md` | 动后处理之前 |
| `polish-render` | `crates/yekitrim-polish/src/render` | `crates/yekitrim-polish/src/render/AGENTS.md` | 动渲染导出/封面之前 |
| `polish-templates` | `crates/yekitrim-polish/templates` | `crates/yekitrim-polish/templates/AGENTS.md` | 改出片模板之前 |
| `polish-project` | `crates/yekitrim-polish/src/project` | `crates/yekitrim-polish/src/project/AGENTS.md` | 动精修配置/去静音之前 |
| `polish-assets` | `crates/yekitrim-polish/src/assets` | `crates/yekitrim-polish/src/assets/AGENTS.md` | 动贴纸库/音效库之前 |
| `polish-llm` | `crates/yekitrim-polish/src/llm` | `crates/yekitrim-polish/src/llm/AGENTS.md` | 动精修侧 LLM 之前 |
| `service` | `crates/yekitrim-service` | `crates/yekitrim-service/AGENTS.md` | 动领域逻辑之前 |
| `service-ingest` | `crates/yekitrim-service/src/ingest` | `crates/yekitrim-service/src/ingest/AGENTS.md` | 动采集入库之前 |
| `service-downloader` | `crates/yekitrim-service/src/downloader` | `crates/yekitrim-service/src/downloader/AGENTS.md` | 动下载器端口/适配器之前 |
| `service-bilibili` | `crates/yekitrim-service/src/bilibili` | `crates/yekitrim-service/src/bilibili/AGENTS.md` | 动 B站链接通道之前 |
| `service-net` | `crates/yekitrim-service/src/net` | `crates/yekitrim-service/src/net/AGENTS.md` | 动 HTTP 小工具之前 |
| `server` | `crates/yekitrim-server` | `crates/yekitrim-server/AGENTS.md` | 动 HTTP/任务队列/宿主之前 |
| `server-api` | `crates/yekitrim-server/src/api` | `crates/yekitrim-server/src/api/AGENTS.md` | 加接口/改路由之前 |
| `cli` | `crates/yekitrim-cli` | `crates/yekitrim-cli/AGENTS.md` | 加命令/改入口之前 |
| `xtask` | `crates/xtask` | `crates/xtask/AGENTS.md` | 动构建/打包/产物门禁之前 |
| `webui` | `webui` | `webui/AGENTS.md` | 改构建/dist/契约脚本时 |
| `webui-src` | `webui/src` | `webui/src/AGENTS.md` | 动前端源码之前 |
| `webui-api` | `webui/src/api` | `webui/src/api/AGENTS.md` | 动接口层之前 |
| `webui-views` | `webui/src/views` | `webui/src/views/AGENTS.md` | 改页面之前 |
| `webui-components` | `webui/src/components` | `webui/src/components/AGENTS.md` | 改组件之前 |
| `webui-styles` | `webui/src/styles` | `webui/src/styles/AGENTS.md` | 改样式之前 |
| `webui-polish` | `webui/src/polish` | `webui/src/polish/AGENTS.md` | 动精修预览共享模块之前 |
<!-- module-index:end -->

### 改代码协议（每一条都对应一次踩坑）

1. **先读卡**：改哪个目录先 `read` 它的 `AGENTS.md`（读一次就会触发注入）。
2. **只改卡声明的范围**：跨模块改动 = 先读对方卡，**并在提交说明里写明理由**。
3. **跑该卡的「验证」命令**，再跑两条门禁：
   `python3 scripts/check_module_map.py`（结构/覆盖率/预算）与
   `python3 scripts/check_module_boundaries.py`（分层与前端边界）。
4. **新增目录 = 新增卡**：新建模块目录后要开 `AGENTS.md`、登记进 `docs/module-map.json`、
   加进本文件的索引表 —— 否则 `check_module_map.py` 会报"无主文件"。
5. **别把模块级内容塞进根文件**：根文件超标会让全局规矩整份消失（见「职责」的 ⚠️）。

## 边界

以下是**全局**不变式（模块内的规矩在各自卡里）：

- ⚠️ **程序资产只挂一个根**（2026-10-02 口径：*不要用户目录*）：`path::data_dir()` = `YEKITRIM_DATA_DIR` ??
  `exe_dir()`，配置/账本/模型/字体/工程全在它下面。**别再挂 `~/.yekitrim/`** —— 两个可能的位置就必然有地方
  只查一处（实测 CLI 与 WebUI 各看一份字体库）。见 **`docs/file-layout.md`**。
- ⚠️ **同步 IO 绝不能留在 async 线程上**（剪切时"整个服务器连不上"的根因）：handler 里的
  同步文件访问走 `api::blocking(f)`，任务体走 `api::spawn_job`。完整纪律（已覆盖清单、panic 兜底、
  路径锁）在 `crates/yekitrim-server/src/api/AGENTS.md`。
- ⚠️ **「取消」必须在批次/窗口边界真正生效**：判据只有 `ProgressReporter::cancelled()`（server 侧由
  `JobManager::cancel_token` 的 `AtomicBool` 供给）。阶段边界由 `PipelineRunner` 查；**批量 LLM 调用
  要自己把探针交给 `llm::Agent::with_cancel(probe)`**（派发每个窗口前 + 每收一个窗口后查，命中即
  `abort_all` 并报「任务已取消」）。只在阶段边界查过一次的批量循环 = 用户点了停止、后台仍把十几个
  窗口跑完（实测 18 窗、十几分钟）；新增批量循环照这里来。
- ⚠️ **产物落盘 = 一次绑定**：`transcript_file` 的登记只有一条路 ——
  `yekitrim_service::assets::bind_asr_artifact`（订正稿走 `bind_derived_material`）。
  别再在别处手写 `transcript_file`，见 `crates/yekitrim-service/AGENTS.md`。
- ⚠️ **配置改动的五步**（字段 + `merge()` 条件 + 手动 `Default` impl + 模板/注释 + **改键时升 `config_version` 并登记迁移**）见
  `crates/yekitrim-core/src/config/AGENTS.md`；`patch_config_file` 是递归深合并，别退回"一层 section"。
- ⚠️ **许可：别把 espeak-ng 或任何 GPL 组件弄回二进制** —— 那会让整个产物重新变成 GPL 组合作品。
  两道防线：`vendor/sherpa-onnx-sys/build.rs` 的占位断言与 `cargo xtask scan`。
  改依赖前先看 `THIRD-PARTY-NOTICES.md`。部署相关的正文（路径面三防线、认证授权、限流配额、
  端口选择、公网清单）在 **`docs/deploy.md`**，改多租户面之前先读它。
- ⚠️ **路径面三防线**（普通用户能读服务器任意文件的历史事故，三处缺一不可）：
  ① `yekitrim-server/src/scope.rs::enforce_scope` 的 `EXTRA_PATH_FIELDS`；
  ② `api/config.rs::PROJECT_PATH_KEYS` + `check_project_paths` / `check_material_paths`；
  ③ 消费侧纵深防御（`yekitrim-polish/src/cover.rs` 的 `tenant::ensure_within`）。
  细节与"新增路径字段要做什么"在 `crates/yekitrim-server/src/api/AGENTS.md`。
- ⚠️ **WebUI 在 release 里是内嵌的**（`yekitrim-server/build.rs` 把 `webui/dist` 编进 exe）：**升级只需换
  exe**，"新 exe + 旧界面"这种错配在物理上不存在。debug 走读盘（改前端不必重链接），`--webui-dir` /
  `YEKITRIM_WEBUI_DIR` 可显式覆盖。⚠️ **只有 release 才内嵌/查新鲜度**：debug 下改前端**不**牵动后端编译。
  ⚠️ `webui/dist` **仍要入库**、改了 `src` 仍要重建（release 的编译期输入，缺失/过期即编译失败）。
- ⚠️ **本仓不再自带任何 GPU 渲染**：wgpu 管线 / `native/` / `yekitrim-wasm` / `render-frame` 接口**整条已
  删除**，GPU 那侧归独立的 **Dhampir 底座**。⚠️ **出片默认就是底座**（`dhampir`），画面归底座、音轨归本仓。
  **别再往本仓加回 GPU 管线** —— 细节与删除清单见 `polish-render` 卡。
- ⚠️ **默认端口 26320 只有一份定义**：`yekitrim-service/src/doctor.rs::DEFAULT_PORT`。
  `ServerArgs.port`、`DEFAULT_URL`、`polish preview` 回退都引用它，**别处不许再写端口字面量**。
- ⚠️ **只监听 loopback**；`--allow-remote` 才对外，且**仍用初始密码时拒绝启动**。
  `--trust-proxy` 是 `X-Forwarded-*` 的唯一开关（决定限流键与 cookie 的 `Secure`）。
- ⚠️ **改内建 prompt 要同步用户配置**：`.yekitrim/config.toml` 里的检测 prompt 是 `config init`
  填进去的**副本**，改代码默认值对它无效；**每个程序目录各有一份**。
- ⚠️ **不许手改生成物**：`webui/dist/`（vite 构建产物，**已入库**）—— 改了 `webui/src` 必须
  `npm --prefix webui run build` 重建，再一起提交；别直接编辑 dist 里的 js/css。


## 验证

```bash
# 结构门禁（本文件与模块卡的预算/覆盖率/骨架/索引一致/⚠️ 下限）
python3 scripts/check_module_map.py
# 边界门禁（19 条：分层方向 + P4 隔离 + 前端 + LLM 隔离 + 成对判据 + 工作区卫生）
python3 scripts/check_module_boundaries.py

# 改任何 crate 之后
cargo check --workspace --all-targets          # 0 warnings
cargo test -p yekitrim-server                   # HTTP 侧（JobManager / Range / 路由 / 配置 patch）
cargo test -p yekitrim-service                  # 领域逻辑（db / 项目 / 容量 / 下载器 / B站）
cargo test -p yekitrim-core --lib danmaku       # ASS + XML 解析 + 密度

# 改了前端（webui/src）之后：必须重建 dist
npm --prefix webui run build
node webui/scripts/check-overlay.mjs           # 覆盖层色档（另有 8 个 check-*.mjs，见 webui/AGENTS.md）

# 改了转译器（tools/polish-to-dhampir.mjs）之后：曲线守卫 + 变异自证（见 tools/AGENTS.md）
node scripts/check-yekitrim-translator.mjs
node scripts/check-yekitrim-translator.mjs --mutate

cargo run -- status                            # 查看当前进度
```

⚠️ **用 `python3`**：本机 `python` 不在 PATH（文档里的旧写法 `python scripts/...` 跑不起来）。
⚠️ 需要真实网络/B站行为核实时，样本与脚本留在 `.workbuddy/`（已 gitignore）。
