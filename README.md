# YekiTrim

直播录像自动切片工具：输入录播视频 + 弹幕，自动**转写 → 检测高光事件 → 按事件切出片段与字幕**，
再进精修编辑器加特效字幕 / 贴纸 / 音效 / 封面并出片。

- 一条命令跑完整条管线（`yekitrim process`），也可以在 WebUI 里点。
- 事件自带热度排序（弹幕/字幕密度 + LLM 传播潜力评分），切完输出 `publish_config.json`。

## 功能一览

| 能力 | 说明 |
|---|---|
| 全自动切片 | ffmpeg 抽音 → sherpa-onnx 本地 ASR → LLM 事件检测 → 按事件剪切 + 裁剪字幕/弹幕 |
| 事件排序 | 弹幕/字幕热度与 LLM 评分加权，输出 `popularity`（0~1）与 `rank` |
| 精修出片 | GSAP 时间线引擎：特效字幕、弹幕渲染、贴纸、音效、封面；出片走 Dhampir 底座 |
| WebUI 多用户 | 账号 / 会话 / 项目 / 任务队列 / LLM 用量，浏览器里从建项目到出片 |
| 录播姬采集 | 录播结束 webhook 自动入库，并按策略跑管线 |
| 字体库 | 两支 OFL 字体内嵌，首次启动自动装好 |

## 前置条件

| 依赖 | 用途 |
|---|---|
| **ffmpeg** + **ffprobe** | 抽音 / 剪切，需在 `PATH` |
| **Rust** 工具链 | 构建 |
| **Node.js 18+** | **出片必需**：`polish.toml` → 底座工程由 `tools/polish-to-dhampir.mjs` 完成 |
| **Dhampir 底座** | 唯一的出片通道（本仓不带 GPU 渲染） |

⚠️ 出片只有底座这一条腿（逐帧 Chrome 兜底已整条删除），缺 Node / 底座时是**明确报错**，不会静默降级。
`yekitrim polish doctor` 的 `[dhampir]` 块会说明缺什么；底座二进制放同层
`Dhampir/target/{debug,release}/` 或 `<程序目录>/dhampir/bin/`，也可用 `VTEDIT_DHAMPIR` /
`VTEDIT_DHAMPIR_CLI` 指定。

## 快速开始

```bash
cargo build

yekitrim init input.mp4      # 生成 yekitrim.toml；--danmaku 可选（默认找同名 .ass）
yekitrim download            # 下载 ASR 模型（sensevoice ~165MB / qwen3-asr ~879MB）
yekitrim process             # 跑完 4 个 stage，产物在 output/

yekitrim server --open       # 想要界面就起 WebUI（默认 127.0.0.1:26320）
```

## 架构

依赖方向**单向**，由 `scripts/check_module_boundaries.py` 逐条机器检查：

```
yekitrim-core ───────────┐
                         ├──→ yekitrim-service ──→ yekitrim-server ──→ yekitrim-cli
yekitrim-polish ─────────┘
      ↑
yekitrim-render-core
```

| crate | 职责 |
|---|---|
| `crates/yekitrim-core` | 管线编排 + ASR / 事件检测 / 剪切 + 配置 / 弹幕 |
| `crates/yekitrim-llm` | LLM 传输层（叶子 crate） |
| `crates/yekitrim-render-core` | 事件/时间线类型与时间线数学（不含任何 GPU 管线） |
| `crates/yekitrim-polish` | 精修引擎：字幕 / 贴纸 / 音效 / 特效 → 出片与封面 |
| `crates/yekitrim-service` | CLI 与 WebUI 共用的领域逻辑（账本 / 项目 / 录播姬 / B站 / 用量） |
| `crates/yekitrim-server` | axum HTTP API + WebUI 宿主 + 任务队列 |
| `crates/yekitrim-cli` | 入口（clap + tracing），二进制名 `yekitrim` |
| `crates/xtask` | 构建 / 打包 / 产物门禁（`cargo xtask scan\|package`） |

- `yekitrim-service` 绝不 `use yekitrim_server::*`；`server` 只依赖 `service`，不碰 `cli`。
- `core` / `polish` / `render-core` 不认识上层；**GPU 出片整条不在本仓**，归独立的 Dhampir 底座。
- 前端 `components/` 不许 import `views/`；作用域查询串只在 `webui/src/api/` 里拼。

其余目录：`webui/`（Vue 3 + Vite，`dist` 已入库、release 内嵌进 exe）、`docs/`（部署 / 设计 / 发行说明）、
`tools/`（转译器等脚本）。完整模块地图见 [docs/module-map.json](docs/module-map.json)，改代码前先读对应目录的
`AGENTS.md`（根卡 [AGENTS.md](AGENTS.md) 是路由页）。

## 命令总览

**参数、默认值与注意点见 [docs/cli-reference.md](docs/cli-reference.md)。**

管线：

| 命令 | 作用 |
|---|---|
| `init <video>` | 建 `yekitrim.toml`；已存在默认拒绝，`--force` 才覆盖（先备份） |
| `process` | 补齐未完成的阶段（已有产物跳过；`--dry-run` 只跑到 detect） |
| `extract` / `transcribe` / `correct` / `detect` / `cut` | Stage 1~4 单级执行，`correct` 是 LLM 订正字幕 |
| `silence [files…]` | 把长停顿并入屏蔽轨道（只写 `polish.toml`，不剪视频） |
| `status` / `doctor` | 当前进度 / 安全与运行状况摘要 |

环境与配置：

| 命令 | 作用 |
|---|---|
| `download` | 下载全部 ASR 模型到 `<数据目录>/.yekitrim/models/` |
| `fonts list` / `fonts install [--id]` | 查看 / 重装自带字体 |
| `config init` / `config show` / `config migrate` | 程序级配置生成 / 显示 / 清掉已下线的键 |
| `usage [--days N]` | LLM 用量（token / 缓存命中 / 耗时） |

服务与账号：

| 命令 | 作用 |
|---|---|
| `server` | 起 WebUI + HTTP API（见下节） |
| `user list\|info\|add\|passwd\|check\|disable\|init-admin\|reset-admin-password` | 账号与密码 |
| `user invite\|invites\|invite-revoke` | 注册邀请码与注册开关 |
| `ingest token\|status` | 录播姬 webhook token / 采集配置概览 |

精修：

| 命令 | 作用 |
|---|---|
| `polish init\|llm\|process\|render\|cover\|preview\|doctor\|test` | 特效精修全流程 |
| `polish stickers list\|test\|scan\|build` | 本地贴纸库 |

## 管线与状态机

| Stage | # | 输入 | 输出 |
|---|---|---|---|
| AudioExtractor | 1 | 视频 | 16kHz mono WAV |
| Transcriber | 2 | WAV | `video.srt` + `transcript.json` |
| EventDetector | 3 | 转录 JSON + 弹幕 | `events.json`（含 `popularity` / `rank`） |
| Cutter | 4 | 视频 + `events.json` | `clips/` + 字幕 + `publish_config.json` |

- Stage 编号**连续无跳过**；`current_stage` **不落盘**，由 `output/` 里的产物存在性推导。
- `stage > current + 1` → 拒绝；`stage <= current` → 可重跑；跑完自动推进。
- `process` 无门槛，但**已有产物的阶段会跳过**（想重跑用单级命令）。
- 三种运行模式：`run()`（受状态机约束）、`run_all()`（`process`）、`run_until(max)`（`--dry-run`）。

## 配置

两层合并：项目 `yekitrim.toml` → 程序级 `config.toml`（后加载覆盖前者）。程序配置**按账号各一份**，默认在
`<数据目录>/users/<uid>/.yekitrim/config.toml`（单人是当前管理员那份，数据目录默认就是程序目录）；
LLM 凭据单独放同目录的 `llm.toml`。合并规则：String 非空才覆盖，数值只有不等于默认值时才覆盖。

```toml
# yekitrim.toml（yekitrim init 生成）
[project]
input_file = ""
danmaku_file = ""
transcript_file = ""      # transcribe / correct 写入
audio_file = ""           # extract 写入
events_file = ""          # detect 写入

[asr]
backend = "local"

[detection]
llm_score_weight = 0.3    # 标题+摘要 LLM 评分权重，0=关闭
silence_threshold_secs = 3.0
min_segment_secs = 120
max_segment_secs = 900

[cutting]
output_dir = "./output"
overwrite = false
```

```toml
# <数据目录>/users/<uid>/.yekitrim/config.toml（yekitrim config init 生成，含内建 prompt 副本）
[asr]
model = "sensevoice"      # sensevoice(快) | qwen3-asr(最准,慢)
num_workers = 0           # 0=auto(CPU/4，上限 4)
chunk_duration = 60.0     # 每块秒数
chunk_overlap = 10.0      # 块间重叠
segment_gap = 1.0         # 段间静音阈值(秒)
language = "zh"

[detection]
vtuber_name = ""          # 非空则追加到标题末尾
danmaku_window_secs = 10

[cutting]
output_dir = "./output"
overwrite = false
```

```toml
# <数据目录>/users/<uid>/.yekitrim/llm.toml
base_url = ""             # 留空 = OpenAI
api_key = ""
model = ""                # 留空 = 各模块默认模型
temperature = 0
max_tokens = 65536
extra_params = { seed = 42, thinking = { type = "disabled" } }
```

⚠️ 程序级配置里的检测 prompt 是 `config init` 填进去的**副本**，改代码里的默认值对它无效，
**每个程序目录各有一份**。`config init` 是覆盖写；多用户形态下每个账号各一份（见
[docs/file-layout.md](docs/file-layout.md)）。

## 输出

```
<output_dir>/
├── video.wav                # 音频
├── video.srt                # 字幕（UTF-8 BOM）
├── video_llm.srt            # LLM 订正字幕（如有）
├── transcript.json          # 结构化转写
├── events.json              # 检测出的事件
├── publish_config.json      # 发布配置
└── clips/
    ├── clips_order.txt      # 按热度降序
    ├── clip-01/
    │   ├── clip.mp4
    │   ├── clip.srt         # 裁剪并偏移后的字幕
    │   ├── clip.dm.srt      # 弹幕字幕
    │   └── clip.json        # 元数据（标题 / 热度 / 原始时间范围）
    └── clip-02/…
```

## WebUI（`yekitrim server`）

```bash
yekitrim server --open                          # 默认 127.0.0.1:26320
yekitrim server --host 0.0.0.0 --allow-remote   # 对外监听（先改初始密码）
```

- **只监听 loopback**；`--allow-remote` 才允许对外，**仍在用初始密码时拒绝启动**。
- **认证不可关闭**：除 `/api/health`、`/api/auth/login` 外所有接口都要会话（本机访问也一样）。
  首次启动打印一次 admin 初始密码，并存到 `<data_dir>/initial-admin-password.txt`（首次登录后删除）。
- **Windows 双击形态**：发行包里的 `yekitrim-tray.exe` 等价 `server --local-no-auth --open`
  （仅 loopback、本机免登录，带 `Host` 与同源两道闸）。这是**唯一**免登录路径，别拿它对外。
- 项目：管理员可在首页「新增项目 → 服务器文件」填**任意绝对路径**；普通用户只能用自己的
  `<projects_root>/users/<uid>`；**服务不接受上传**。B站下载 / 录播姬入库 / 本机导入落同一个根。
- `--data-dir` 默认 `<exe_dir>/.yekitrim/cloud`；反代 / HTTPS / 上线清单见 [docs/deploy.md](docs/deploy.md)。
- release 里 WebUI 是**内嵌**的，升级只需换 exe；`--webui-dir` / `YEKITRIM_WEBUI_DIR` 可覆盖。
- 全部参数与首次启动行为：[docs/cli-reference.md](docs/cli-reference.md)。

## 视频精修（`yekitrim polish`）

给已切好的 clip 加特效字幕 / 弹幕 / 贴纸 / 音效，生成封面并出片。`<dir>` 是 clip 目录（默认当前目录），
精修项目需要该目录下有 `config.toml` 与 `llm.toml`。

| 子命令 | 作用 |
|---|---|
| `init` | 初始化精修项目（`config.toml` + `llm.toml` + `data/` + `sfx/`，解析素材） |
| `llm` | LLM 分析生成 segments 与关键词 |
| `process` | 后处理：解析 emoji / keyword、下载资源、计算位置 |
| `render` | 出片（自动先跑 process；默认走 Dhampir 底座） |
| `cover [--template left-text\|center-text\|right-text\|minimal]` | 生成封面 |
| `preview [video\|cover]` | 浏览器预览 |
| `doctor` | 检查 ffmpeg / 底座 / LLM 配置 |
| `test` | 生成测试素材 + 预览 |
| `stickers list\|test\|scan\|build` | 本地贴纸库（列出 / 测 keyword 匹配 / 生成清单） |

WebUI 的精修页与这些命令等价；细节见 [docs/cli-reference.md](docs/cli-reference.md)。

## 开发与验证

```bash
# 结构 / 边界门禁（改任何东西都跑）
python3 scripts/check_module_map.py
python3 scripts/check_module_boundaries.py

# 改 Rust 之后
cargo check --workspace --all-targets        # 0 warnings
cargo test -p yekitrim-server                # HTTP：JobManager / Range / 路由 / 配置 patch
cargo test -p yekitrim-service               # 领域：db / 项目 / 容量 / 下载器 / B站
cargo test -p yekitrim-core --lib danmaku    # ASS + XML 解析 + 密度

# 改前端（webui/src）之后：dist 必须重建（已入库）
npm --prefix webui run build
for f in webui/scripts/check-*.mjs scripts/check-preview-audio.mjs; do node "$f" || break; done

# 改转译器（tools/polish-to-dhampir.mjs）之后
node scripts/check-yekitrim-translator.mjs
node scripts/check-yekitrim-translator.mjs --mutate

# 打包
cargo xtask package --smoke-test                    # Windows x64
cargo xtask package --target linux --smoke-test     # Linux x64（构建基线见 docs/linux.md）
```

⚠️ 用 `python3`（本机 `python` 不在 PATH）。`webui/dist` 是**入库产物**，改了 `webui/src` 必须重建后
一起提交；debug 下改前端不牵动后端编译，release 才会查新鲜度。

## 许可

**Apache-2.0**（[LICENSE](LICENSE)）：允许商用、修改、**闭源再分发**，义务只有保留声明这一类。

- 二进制里**不再有 GPL 组件**：本项目只做 ASR，`espeak-ng` 已换成 896 字节零符号占位库；
  随二进制分发的 312 个包审计后无 GPL / LGPL / AGPL，逐条见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。
- 分发发行包时附上 `LICENSE` 与 `THIRD-PARTY-NOTICES.md`（`cargo xtask package` 已强制打入）。
- 为什么从 GPL-3.0-or-later 切换、为什么选 Apache-2.0 而非 MIT、各种分发场景要做什么：
  [docs/licensing.md](docs/licensing.md)。`ffmpeg` / `ffprobe` 以外部命令调用，不随包分发也不链接。

## 字体

两支**自带字体**都是 SIL OFL-1.1（可商用 / 可嵌入），字节内嵌在 `yekitrim.exe` 里，首次启动自动落盘
（不需要联网，OFL 全文随字体一起落盘）：

| 字体 | 用途 | 体积 |
|---|---|---|
| **Noto Sans SC** | 中文正文（字幕 / 封面） | 10.0 MB |
| **Noto Color Emoji** | 彩色 emoji（CBDT 位图） | 9.9 MB |

```bash
yekitrim fonts list                          # 看库里有什么
yekitrim fonts install                       # 补装缺失的自带字体
yekitrim fonts install --id NotoColorEmoji   # 只装某一支
```

- 想用别的字体：在 WebUI「字体库」页导入，或放进字体库目录（`yekitrim fonts list` 会打印实际路径：
  单人是 `<数据目录>/users/<uid>/resources/fonts/`，自带的在公用库 `<数据目录>/global/resources/fonts/`）；
  精修页「字幕字体」与封面设计器「字体」下拉选完**保存即生效**（预览与成片一致）。
- ⚠️ `.ttc` / `.otc` 用不了（浏览器 `@font-face` 不认字体集合），导入会被直接拒绝，请先拆包。
- 兜底栈、换字体历史、字体库目录形状与第三方字体授权：[docs/fonts.md](docs/fonts.md)。

## 文档

| 文档 | 看什么 |
|---|---|
| [docs/cli-reference.md](docs/cli-reference.md) | 每条命令的参数 / 默认值 / 注意点 |
| [docs/deploy.md](docs/deploy.md) | 三种部署形态、反代 + HTTPS、上线检查清单 |
| [docs/linux.md](docs/linux.md) | Linux 构建基线 / 运行 / systemd / 验证 |
| [docs/file-layout.md](docs/file-layout.md) | 程序资产与配置落在哪（只有一个根） |
| [docs/job-queue.md](docs/job-queue.md) | 任务队列、取消与 SSE |
| [docs/licensing.md](docs/licensing.md) / [docs/fonts.md](docs/fonts.md) | 许可取舍 / 字体细节 |
| [AGENTS.md](AGENTS.md) + [docs/module-map.json](docs/module-map.json) | 工程约定（路由页）与模块索引 |
| [ROADMAP.md](ROADMAP.md) / [docs/releases](docs/releases) | 待排期功能 / 发行说明 |
