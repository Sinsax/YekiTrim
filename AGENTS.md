# V-Trim

直播录像自动切片工具。输入录播视频 + 弹幕文件，输出按事件分割的片段 + 字幕 + `publish_config.json`。

## 架构

Cargo workspace, 三个 crate:
- `crates/vtrim-core` — 管线编排 + 全部业务逻辑
- `crates/vtrim-cli` — CLI 入口 (clap)
- `crates/vtrim-polish` — GSAP 精修引擎（从 vtedit 合并）

**Stage 编号**: AudioExtractor(1) → Transcriber(2) → EventDetector(3) → Cutter(4)，连续无跳过。

所有 stage 用 `#[async_trait]` 实现 `Stage` trait (`crates/vtrim-core/src/pipeline/stage.rs`)。

`PipelineRunner` 提供三种运行模式 (`runner.rs`):
- `run()` — 受文件存在性约束（而非 `current_stage`），单 stage 命令用
- `run_all()` — 无视文件检查从头跑到底，`process` 命令用
- `run_until(max)` — 跑到指定 stage 为止，`process --dry-run` 用

`current_stage` 不再持久化到 `vtrim.toml`，改为从输出文件存在性自动推导：

| 文件存在 | 推导阶段 |
|---------|---------|
| `output/video.wav` 或 `audio_file` 非空 | Stage 1 |
| `output/transcript.json` 或 `video.srt` 或 `transcript_file` 非空 | Stage 2 |
| `output/events.json` 或 `events_file` 非空 | Stage 3 |
| `output/clips/clips_order.txt` 存在 | Stage 4 |

## CLI

```
vtrim init <video> [--danmaku <file>]    # 创建 vtrim.toml
vtrim extract [-i] [-o]                  # 提取音频 (Stage 1)
vtrim process [-i] [-o] [--dry-run]      # 全自动管线
vtrim transcribe [-i]                    # Stage 2 (local)
vtrim detect [-t <transcript>]           # Stage 3 (自动发现: 参数 > config > output_dir/*.json > *.srt)
vtrim cut [-i] [-e <events>] [-o]        # Stage 4 (自动发现: 参数 > config/output_dir/events.json)
vtrim silence [files...]                 # 对已输出 clips 移除长静音段 (重编码)
       [--duration 2.0] [--threshold]    # 默认扫描 output/clips/, 输出到输入文件同目录
vtrim download                           # 下载全部 ASR 模型 (sensevoice/firered-zh/qwen3-asr) → .vtrim/models/
vtrim status                             # 查看当前进度
vtrim config init [--path]               # 生成全局配置文件
vtrim config show                        # 显示当前配置 (含 LLM)
vtrim correct [-t <transcript>]          # LLM 订正转录文本
vtrim polish init [<dir>]                # 精修: 初始化项目 + 解析字幕弹幕
vtrim polish llm [<dir>]                 # 精修: LLM 生成 segments（使用 .vtrim/llm.toml）
vtrim polish process [<dir>]             # 精修: 后处理 + 下载资源 + 计算位置
vtrim polish render [<dir>]              # 精修: 渲染视频（自动先跑 process）
vtrim polish cover [<dir>] [--template]  # 精修: 生成封面
vtrim polish preview [<dir>] [mode]      # 精修: 浏览器预览（video/cover/双端口）
vtrim polish doctor [<dir>]              # 精修: 系统检查
vtrim polish test [<dir>]                # 精修: 测试模式
vtrim polish stickers list               # 本地贴纸: 列出索引（组名+成员数+tags）
vtrim polish stickers test <kw...>       # 本地贴纸: 测试 keyword→贴纸匹配（诊断用）
vtrim polish stickers scan [--llm]       # 本地贴纸: 生成/更新 stickers.json 清单（--llm 批量生成英文 tags）
```

所有 stage 命令的 `-i` 可选，默认从 `vtrim.toml` 的 `input_file` 读取。

## 状态机

`current_stage` 不再持久化到 `vtrim.toml`，改为从输出文件存在性自动推导。

| 规则 | 示例 |
|------|------|
| `stage > current + 1` → 拒绝 | `detect`(3) 无转写文件 → ❌ |
| `stage <= current` → 可重跑 | `transcribe`(2) 存在转写文件 → ✅ |
| 完成后 stage 自动推进 | `transcribe`(2) 成功后文件已存在 |
| `process` 无视文件检查, 全部重跑 | |
| stage 无跳过 | 缺失前置文件则拒绝 |

实现: `config/types.rs` 的 `current_stage()` 方法通过文件存在性推导。

## 配置文件

两层合并: `vtrim.toml` (项目) → `.vtrim/config.toml` (程序, 后加载覆盖前者)。无环境变量。

`vtrim init` 生成 `vtrim.toml`（项目字段：input/danmaku/转写/剪切参数）。
`vtrim config init` 生成 `.vtrim/config.toml`（`[asr]` + `[detection]` + `[correct]` + `[sfx]`）和 `.vtrim/llm.toml`（`[llm]`）。

**`vtrim.toml`** — 项目配置:
```toml
input_file = ""
danmaku_file = ""
transcript_file = ""
audio_file = ""
events_file = ""

[detection]
danmaku_window_secs = 10

[cutting]
output_dir = "./output"
overwrite = false
silence_noise_db = -30
```

**`.vtrim/config.toml`** — 全局程序配置:
```toml
[asr]
model = "sensevoice"         # ASR 模型: sensevoice(快) | firered-zh(CTC,较准) | qwen3-asr(最准,慢)
num_workers = 0              # 0=auto(CPU/4,上限4), N=指定并行数(不超过CPU核数)
chunk_duration = 60.0        # 每块秒数
chunk_overlap = 10.0         # 块间重叠秒数
segment_gap = 1.0            # 段间静音阈值(秒)，超过此间隔才切为两条字幕
llm_correct = false          # LLM 订正转录文本（需配置 [llm]）

[correct]
system_prompt = "..."       # 订正 prompt（覆盖内建默认）

[detection]
# 主播名称 (非空时自动追加到切片标题末尾)
vtuber_name = ""
# 弹幕密度统计窗口(秒)
danmaku_window_secs = 10
# 检测 system_prompt（修改此处覆盖内建默认）
system_prompt = "..."
# 检测 user prompt_template（{segments} 会被替换；留空 = 内建默认）
prompt_template = "..."

[cutting]
output_dir = "./output"
overwrite = false
silence_noise_db = -30

[sfx]
# 音效屏蔽列表 (vtrim polish 使用): 精确 id 屏蔽单个音效, "tag:xxx" 屏蔽整个标签族
blacklist = []
```

**`.vtrim/llm.toml`** — LLM 配置（独立文件）:
```toml
[llm]
base_url = ""              # OpenAI 兼容 API 地址
api_key = ""
model = ""
temperature = 0
max_tokens = 65536
extra_params = { seed = 42, thinking = { type = "disabled" } }
```

merge 条件: String 字段 `!is_empty()`, f64/u32 字段 != 默认值, Vec 字段非空时追加合并。

**配置默认值注意事项**: 所有配置类型必须手动实现 `Default`（而非 `#[derive(Default)]`）以确保 serde 默认值与 Rust 默认值一致。`Config::default()` 用 `toml::from_str("")` 初始化，依赖各字段的 `#[serde(default)]` 来调用正确的 `Default` impl。新增字段时必须同时加 `Default` impl + `merge()` 条件。

## 各阶段 I/O

### AudioExtractor (Stage 1, `vtrim extract`)
- 输入: 视频 → ffmpeg 16kHz mono WAV + ffprobe VideoInfo
- `ffmpeg -y -vn -acodec pcm_s16le -ar 16000 -ac 1` → `output/video.wav`

### Transcriber
- sherpa-onnx 库 (CPU only)，模型按 `[asr] model` 选择: `sensevoice` (默认) | `firered-zh` (FireRedASR v2 CTC int8) | `qwen3-asr` (Qwen3-ASR 0.6B int8, 最准最慢)
- firered-zh / qwen3-asr 无 token 级时间戳: 兜底按标点+字数占比分配块内时间 (split_text_by_ratio)
- 可配置定长分块 (默认 60s + 10s 重叠)
- 并行转写: 默认 CPU/4 路并行 (上限 4)，每 chunk intra-op threads=2
- SRT 使用 UTF-8 BOM 写入 (Windows 字幕兼容)
- 输出: `video.srt` + `transcript.json`，自动写 `transcript_file` 到 vtrim.toml

### EventDetector
- 一次 LLM API 调用分析全部 segments (利用 64k+ 上下文窗口), 非分批
- 全中文 prompt: ≥1分钟片段、B 站风格标题(含 emoji)、详尽摘要
- 弹幕: ASS 解析 → 10s 窗口归一化密度 → 每个 event 的 `popularity` 0~1
- 按 popularity 降序赋 `rank`，最终按时间排序输出
- 降级: API key 为空时使用单个默认事件（调用失败则报错）
- 输出: `events.json`

### Cutter
- GPU 加速编码 (NVENC/QSV/AMF 自动检测, 降级 libx264)
- `ffmpeg -ss {start} -i {input} -t {duration} -c:v {auto_encoder} -preset {auto_preset} {qparam} 23 -pix_fmt yuv420p -c:a aac -movflags +faststart {output}`
- 命名: `{label}.mp4` → `output/clips/` (无编号前缀)
- 字幕: 从原始 SRT 裁剪时间范围 + 偏移 → `{label}.srt`
- 输出: `output/publish_config.json` + `output/clips/clips_order.txt` (按弹幕热度降序)

## 开发

```bash
cargo build                    # 0 warnings
cargo test --lib danmaku       # ASS 解析 + 密度计算测试
cargo run -- status            # 查看当前进度
```

前置条件: `ffmpeg` + `ffprobe` 在 PATH 中。

### 新增配置字段步骤

1. `config/types.rs` 加字段 + serde default
2. `config/mod.rs` 的 `merge()` 加对应条件（注意 `.vtrim/config.toml` 在 `vtrim.toml` 之后 merge）
3. 结构体手动 `Default` impl (有字段级 serde default 时必须)
4. `config/default.toml` + 注释

### 核心文件索引

| 路径 | 说明 |
|------|------|
| `vtrim-cli/src/main.rs:27` | `resolve_input()` input 回退 |
| `vtrim-cli/src/main.rs:41` | `build_pipeline_runner()` 注册 stage |
| `vtrim-cli/src/main.rs:62` | `check_stage()` 状态机 |
| `vtrim-core/src/pipeline/stage.rs` | STAGE_* 常量 + Stage trait |
| `vtrim-core/src/pipeline/runner.rs` | PipelineRunner (run/run_all/run_until) |
| `vtrim-core/src/config/mod.rs` | 两源合并 + toml_edit |
| `vtrim-core/src/config/types.rs` | Config 类型定义 + Default impl |
| `vtrim-core/src/types/mod.rs` | Event, PipelineContext, Transcript |
| `vtrim-core/src/download.rs` | 模型/binary 下载 + 断点续传 |
| `vtrim-core/src/asr/local.rs` | sherpa-onnx (CPU only) |
| `vtrim-core/src/detection/llm.rs` | LLM prompt + API |
| `vtrim-core/src/detection/mod.rs` | EventDetector + 弹幕热度 |
| `vtrim-core/src/danmaku/mod.rs` | ASS/SRT 弹幕解析 + 密度 |
| `vtrim-core/src/danmaku/parser.rs` | ASS Dialogue 解析 |
| `vtrim-core/src/llm/mod.rs` | 统一 LLM 客户端（core + polish 共用） |
| `vtrim-core/src/cutting/mod.rs` | Cutter |
| `vtrim-core/src/cutting/silence.rs` | 静音检测 + remove_silence |
| `vtrim-polish/src/commands.rs` | 所有 cmd_* 精修命令入口 |
| `vtrim-polish/src/process/mod.rs` | cmd_process 主流程（字幕/段处理/预算） |
| `vtrim-polish/src/process/sfx.rs` | SFX 规则引擎 + 音效解析 |
| `vtrim-polish/src/process/fx.rs` | 瞬时特效 duration 分类与 clamp |
| `vtrim-polish/src/process/download.rs` | 贴纸解析 + 资产下载命令 |
| `vtrim-polish/src/project/config.rs` | vtedit 风格项目配置 + Event/Segment 类型 |
| `vtrim-polish/src/render/chrome.rs` | Chrome headless 渲染 |
| `vtrim-polish/src/render/ffmpeg.rs` | FFmpeg 合成/编码/SFX 混音 |
| `vtrim-polish/src/render/server.rs` | 预览 HTTP server |
| `vtrim-polish/src/llm/mod.rs` | LLM 段分割 + 关键词 |
| `vtrim-polish/src/llm/events.rs` | Step2 逐段事件生成（core Agent + checkpoint） |
| `vtrim-polish/src/assets/downloader.rs` | emoji/irasu/SFX 下载 |
| `vtrim-polish/src/render/pipeline.rs:11` | cmd_render（自动 process → render → cover） |
| `vtrim-polish/src/commands.rs:296` | cmd_preview（自动 process → 双端口预览） |
| `vtrim-polish/src/commands.rs:200` | cmd_llm（使用 .vtrim/llm.toml） |
