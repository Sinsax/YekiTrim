# V-Trim

直播录像自动切片工具。输入录播视频 + 弹幕文件，输出按事件分割的片段 + 字幕 + `publish_config.json`。

## 架构

Cargo workspace, 两个 crate:
- `crates/vtrim-core` — 管线编排 + 全部业务逻辑
- `crates/vtrim-cli` — CLI 入口 (clap)

**Stage 编号**: AudioExtractor(1) → Transcriber(2) → EventDetector(3) → Cutter(4)，连续无跳过。

所有 stage 用 `#[async_trait]` 实现 `Stage` trait (`crates/vtrim-core/src/pipeline/stage.rs`)。

`PipelineRunner` 提供三种运行模式 (`runner.rs`):
- `run()` — 受 `current_stage` 状态机约束，单 stage 命令用
- `run_all()` — 无视状态机从头跑到底，`process` 命令用，完成后设 `current_stage=4`
- `run_until(max)` — 跑到指定 stage 为止，`process --dry-run` 用

## CLI

```
vtrim init <video> [--danmaku <file>]    # 创建 vtrim.toml
vtrim extract-audio [-i] [-o]            # 提取音频 (Stage 1)
vtrim process [-i] [-o] [--dry-run]      # 全自动管线 (无视 state)
                                         # --dry-run: 只到 EventDetector, 不剪切
vtrim transcribe [-i] [--backend]        # Stage 2 (local/bcut/kuaishou)
vtrim detect-events [-t <transcript>]    # Stage 3 (自动发现: 参数 > config > output_dir/*.json > *.srt)
vtrim cut [-i] [-e <events>] [-o]        # Stage 4 (自动发现: 参数 > config/output_dir/events.json)
vtrim silence-remove [files...]          # 对已输出 clips 移除长静音段 (重编码)
       [--duration 2.0] [-o <dir>]      # 默认扫描 output/clips/, 输出到输入文件同目录
vtrim download-models                    # sherpa-onnx binary + SenseVoice 模型 → .vtrim/
vtrim setup-spleeter                     # 安装 Spleeter 人声分离到 .vtrim/venv/ (需 uv)
vtrim separate [input] [-o <dir>]       # 人声分离 (默认使用 config.audio_file)
vtrim status
vtrim config init [--path]
vtrim config show
vtrim version
```

所有 stage 命令的 `-i` 可选，默认从 `vtrim.toml` 的 `input_file` 读取。

## 状态机

`vtrim.toml` 中的 `current_stage` 控制单 stage 命令:

| 规则 | 示例 (current=2) |
|------|-----------------|
| `stage > current + 1` → 拒绝 | `detect`(3=2+1) ✅ / `cut`(4>3) ❌ |
| `stage <= current` → 可重跑 | `transcribe`(2≤2) ✅ |
| 完成后 stage 自动推进 | `transcribe`(2) → `current_stage=2` |
| `process` 无视 state, 跑完设 `current_stage=4` | |
| stage 无跳过 | 所有 stage 均为连续编号 |

实现: `config/mod.rs` 的 `save_current_stage()` 用 `toml_edit` 原地修改 `vtrim.toml` 保留注释和格式。

## 配置文件

两层合并: `vtrim.toml` (项目) → `.vtrim/config.toml` (程序, 后加载覆盖前者)。无环境变量。

`vtrim init` 生成 `vtrim.toml`（项目字段：input/danmaku/backend/VAD/检测参数/剪切参数）。
`vtrim config init` 生成 `.vtrim/config.toml`（`[asr]` + `[detection.llm]` 跨工程共享）。

**`vtrim.toml`** — 项目配置:
```toml
input_file = ""
danmaku_file = ""
transcript_file = ""
audio_file = ""
events_file = ""
vocal_file = ""            # vtrim separate 自动写入
current_stage = 0

[asr]
backend = "local"

[asr.vad]
threshold = 0.5
min_silence_duration = 0.5
min_speech_duration = 0.25
max_speech_duration = 20.0

[detection]
danmaku_window_secs = 10
silence_threshold_secs = 3.0
min_segment_secs = 120
max_segment_secs = 900

[cutting]
preserve_source = true
max_parallel = 4
output_dir = "./output"
overwrite = false
```

**`.vtrim/config.toml`** — 全局程序配置:
```toml
[asr]
num_threads = 0              # 0=CPU/4取整上限4, N=指定并行数
chunk_duration = 60.0        # 每块秒数
chunk_overlap = 10.0         # 块间重叠秒数
separation = "none"          # 人声分离: none / spleeter (需先 vtrim setup-spleeter)

[detection.llm]
api_key = ""
api_base = ""                # 留空=OpenAI
model = "gpt-4o-mini"
temperature = 0.1
max_tokens = 16384
batch_size = 25
batch_overlap = 3
```

merge 条件: String 字段 `!is_empty()`, f64/u32 字段 != 默认值。

**配置默认值 bug**: `Config::default()` 用 `toml::from_str("")` 初始化，嵌套 struct 的 `Default` derive 给零值而非 serde 字段级默认值。所有配置类型已手动实现 `Default`，但新增字段时必须手动加否则 merge 逻辑出错。

## 各阶段 I/O

### AudioExtractor (Stage 1, `vtrim extract-audio`)
- 输入: 视频 → ffmpeg 16kHz mono WAV + ffprobe VideoInfo
- `ffmpeg -y -vn -acodec pcm_s16le -ar 16000 -ac 1` → `output/video.wav`

### Transcriber
- 本地 sherpa-onnx-offline (SenseVoice, CPU only)
- 云端 Bcut(B站) / 快手 ASR API (无 key 要求)
- 可选 BGM 分离: `separation = "spleeter"` 时调用 Spleeter 提取人声 → ASR
- 可配置定长分块 (默认 60s + 10s 重叠)
- 并行转写: 默认 CPU/4 路并行 (上限 4)，每 chunk intra-op threads=2
- SRT 使用 UTF-8 BOM 写入 (Windows 字幕兼容)
- 输出: `video.srt` + `transcript.json`，自动写 `transcript_file` 到 vtrim.toml

### EventDetector
- 一次 LLM API 调用分析全部 segments (利用 64k+ 上下文窗口), 非分批
- 全中文 prompt: ≥1分钟片段、B 站风格标题(含 emoji)、详尽摘要
- 弹幕: ASS 解析 → 10s 窗口归一化密度 → 每个 event 的 `popularity` 0~1
- 按 popularity 降序赋 `rank`，最终按时间排序输出
- 降级: API key 为空或调用失败时使用单个默认事件
- 输出: `events.json`

### Cutter
- GPU 加速编码 (NVENC/QSV/AMF 自动检测, 降级 libx264)
- `ffmpeg -ss {start} -i {input} -t {duration} -c:v {auto_encoder} -preset {auto_preset} {qparam} 23 -pix_fmt yuv420p -c:a aac -movflags +faststart {output}`
- 命名: `{label}.mp4` → `output/clips/` (无编号前缀)
- 字幕: 从原始 SRT 裁剪时间范围 + 偏移 → `{label}.srt`
- 输出: `output/publish_config.json` + `output/clips/clips_order.txt` (按弹幕热度降序)

## PipelinContext 中的未使用字段

- `segments: Vec<Segment>` — Cutter 在完成时填充，event 驱动阶段不读写

## 开发

```bash
cargo build                    # 0 warnings
cargo test --lib danmaku       # ASS 解析 + 密度计算测试
cargo run -- status            # 查看当前进度
```

前置条件: `ffmpeg` + `ffprobe` 在 PATH 中。

## 未使用的配置字段

- `cutting.preserve_source` — Cutter 用硬件编码重编码，此字段无效
- `cutting.max_parallel` — Cutter 单线程，忽略此值

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
| `vtrim-core/src/download.rs` | 模型/binary 下载 + 断点续传 + setup_spleeter |
| `vtrim-core/src/asr/local.rs` | sherpa-onnx CLI (CPU only) |
| `vtrim-core/src/asr/cloud_asr.rs` | Bcut/快手 ASR |
| `vtrim-core/src/asr/separate.rs` | Spleeter 人声分离 |
| `vtrim-core/src/detection/llm.rs` | LLM prompt + API |
| `vtrim-core/src/detection/mod.rs` | EventDetector + 弹幕热度 |
| `vtrim-core/src/danmaku/mod.rs` | ASS 解析 + 密度 |
| `vtrim-core/src/cutting/mod.rs` | Cutter + remove_silence |
