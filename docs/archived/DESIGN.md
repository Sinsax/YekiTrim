# V-Trim 设计文档

直播录像自动切片工具。输入录播视频，输出按事件分割的片段 + 字幕 + 发布配置。

---

## 一、架构总览

```
输入: 直播录像.mp4
        │
        ▼
┌───────────────────────────────────────┐
│ Stage 1: AudioExtractor               │  ffmpeg CLI (本地)
│ output: video.wav                     │
└─────────────┬─────────────────────────┘
               │
               ▼
┌───────────────────────────────────────┐
│ Stage 2: Transcriber                  │  ASR (本地/云端) + 可选 Spleeter
│ output: transcript.json + video.srt   │
└─────────────┬─────────────────────────┘
               │
               ▼
┌───────────────────────────────────────┐
│ Stage 3: EventDetector                │
│   LLM (云端)                           │
│ output: events.json                   │
└─────────────┬─────────────────────────┘
               │
               ▼
┌───────────────────────────────────────┐
│ Stage 4: Cutter + Publisher           │  ffmpeg CLI (本地)
│ output: clips/*.mp4 + .srt            │
│         publish_config.json           │
└───────────────────────────────────────┘
```

### 设计原则

| 原则 | 说明 |
|------|------|
| 零本地 ML 推理 | 所有 ML 相关（ASR、源分离、LLM）都走云端 API |
| 管线化 | 5 个 Stage 顺序执行，每阶段输出中间文件，支持断点续跑 |
| 可选阶段 | 音源分离默认关闭，可直接跳过 |
| 配置驱动 | TOML 配置 + 环境变量覆盖 |
| 边缘设备优先 | Linux x64 为目标平台，依赖精简 |

---

## 二、目录结构

```
vtrim/
├── Cargo.toml
├── config/
│   └── default.toml
├── crates/
│   ├── vtrim-core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config/
│   │       │   ├── mod.rs
│   │       │   └── types.rs
│   │       ├── types/
│   │       │   └── mod.rs
│   │       ├── pipeline/
│   │       │   ├── mod.rs
│   │       │   ├── runner.rs
│   │       │   └── stage.rs
│   │       ├── audio/
│   │       │   ├── mod.rs
│   │       │   └── extract.rs
│   │       ├── separation/
│   │       │   ├── mod.rs
│   │       │   └── cloud.rs
│   │       ├── asr/
│   │       │   ├── mod.rs
│   │       │   └── openai.rs
│   │       ├── detection/
│   │       │   ├── mod.rs
│   │       │   ├── rules.rs
│   │       │   ├── batching.rs
│   │       │   ├── llm_refiner.rs
│   │       │   ├── merging.rs
│   │       │   └── validator.rs
│   │       ├── cutting/
│   │       │   ├── mod.rs
│   │       │   ├── ffmpeg.rs
│   │       │   └── publisher.rs
│   │       └── progress/
│   │           └── mod.rs
│   └── vtrim-cli/
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── cli.rs
│           └── progress.rs
```

---

## 三、核心数据流

### PipelineContext

贯穿所有 Stage 的上下文对象：

```rust
struct PipelineContext {
    /// 输入文件路径
    input_path: PathBuf,
    /// 文件名（不含扩展名），用于输出命名
    input_stem: String,
    /// 输出目录
    output_dir: PathBuf,

    // ── 中间文件 ──
    audio_path: Option<PathBuf>,      // Stage1 产出
    vocal_path: Option<PathBuf>,      // Stage2 产出（可选）
    transcript: Option<Transcript>,   // Stage3 产出
    events: Vec<Event>,               // Stage4 产出

    // ── 最终产物 ──
    segments: Vec<Segment>,           // Stage5 产出

    // ── 元数据 ──
    video_info: Option<VideoInfo>,
    started_at: DateTime<Utc>,
}
```

### 核心类型

```rust
// 时间戳
struct Timestamp { h: u32, m: u32, s: u32, ms: u32 }
impl Timestamp {
    fn to_seconds(&self) -> f64;
    fn from_seconds(f64) -> Self;
    fn to_srt_format(&self) -> String;  // "HH:MM:SS,mmm"
}

// 视频信息
struct VideoInfo {
    duration_secs: f64, width: u32, height: u32,
    video_codec: String, audio_codec: String, format: String,
}

// ASR 输出
struct Transcript {
    segments: Vec<TranscriptSegment>, language: String,
}
struct TranscriptSegment {
    start: f64, end: f64, text: String, confidence: f64,
}

// 规则引擎输出（候选段）
struct RawSegment {
    index: usize, start: Timestamp, end: Timestamp,
    text: String, silence_before: f64, speech_density: f64,
}

// LLM 返回的原始事件
#[derive(Deserialize)]
struct LLMEvent {
    start: String, end: String, label: String,
    summary: String, event_type: EventType,
}

// 最终事件
struct Event {
    start: Timestamp, end: Timestamp,
    label: String, summary: String,
    event_type: EventType, confidence: f64,
}
enum EventType { Intro, Gameplay, Interaction, QnA, Break, Outro, Other }

// 剪切输出段
struct Segment {
    index: usize, event: Event,
    output_path: PathBuf, subtitle_path: PathBuf,
}
```

### Stage trait

```rust
#[async_trait]
trait Stage: Send + Sync {
    fn name(&self) -> &'static str;
    fn is_enabled(&self, config: &Config) -> bool;
    async fn run(&self, ctx: &mut PipelineContext) -> Result<()>;
}
```

---

## 四、各 Stage 详细设计

### Stage1 — AudioExtractor

**功能**：从视频中提取 16kHz mono WAV 音频。

**方式**：`ffmpeg` CLI 子进程。

```
ffmpeg -i "输入.mp4" -vn -acodec pcm_s16le -ar 16000 -ac 1 "video.wav"
```

同时用 `ffprobe` 获取视频编码信息存到 `VideoInfo`。

**产出**：`video.wav` + `PipelineContext.video_info`

---

### Stage2 — SourceSeparator（可选，默认关闭）

**功能**：将混合音频分离为人声和背景音乐，提升 ASR 准确率。

**配置**：

```toml
[separation]
enabled = false      # 默认关闭
provider = "openai"  # 预留
```

**架构**：

```rust
#[async_trait]
trait SeparationBackend: Send + Sync {
    async fn separate(&self, input: &Path) -> Result<SeparateOutput>;
}
struct SeparateOutput {
    vocal: PathBuf,
    bgm: Option<PathBuf>,
}
```

- `enabled=false` → Stage 跳过，ASR 直接处理 `video.wav`
- `enabled=true` → 调用云端分离 API，ASR 处理 `vocal.wav`（人声更纯净）

---

### Stage3 — Transcriber（仅云端）

**功能**：将音频转为带时间戳的文字。

**配置**：

```toml
[asr]
provider = "openai"
api_key = "${VTRIM_ASR_KEY}"
model = "whisper-1"
language = "zh"
prompt = "直播,游戏,解说"
```

**实现**：

```rust
#[async_trait]
trait AsrBackend: Send + Sync {
    async fn transcribe(&self, audio: &Path) -> Result<Transcript>;
}

struct OpenAiAsr {
    api_key: String,
    model: String,
    language: Option<String>,
    prompt: Option<String>,
}
```

**大文件处理**：OpenAI 限制 25MB。超过时自动用 ffmpeg 分段（每段 20min），逐段上传后合并结果。

**产出**：
- `transcript.json` — 完整的结构化转录
- `transcript.srt` — SRT 格式字幕文件

---

### Stage4 — EventDetector

分为 4 个子阶段：

```
SRT / transcript.json
        │
        ▼
┌───────────────┐
│   RulesEngine  │  静音检测 + 语速密度 + 时长约束
│   (本地)       │  → Vec<RawSegment>
└───────┬───────┘
        │
┌───────▼───────┐
│    Batcher    │  每 25 段一批，批间重叠 3 段
└───────┬───────┘
        │
┌───────▼───────┐
│  LLMRefiner   │  送 LLM 合并同类段、拆分长段、动态命名
│   (云端)       │  → Vec<LLMEvent>
└───────┬───────┘
        │
┌───────▼───────┐
│   Validator   │  校验时间戳、连续性、时长
│   + Merging   │  合并批间结果 / 异常降级
└───────────────┘
        │
        ▼
   Vec<Event>
```

#### RulesEngine 算法

```python
# 静音检测
cuts = []
for each adjacent segment pair:
    gap = seg[i+1].start - seg[i].end
    if gap > silence_threshold_secs:
        cuts.push(seg[i].end)

# 语速密度分析
window_size = 5
density = []  # 滑动窗口计算字/秒
for sliding_window:
    avg = sum(word_count / duration) / window_size
    density.push(avg)  # 骤降处标记为候选切点

# 时长约束
for each segment:
    if duration < min:  → 合并到相邻段
    if duration > max:  → 在密度最低点切分
```

#### LLM Prompt

```
系统指令:
你是一个直播录像事件分割器。分析以下语音分段，做两件事：
1. 合并：属于同一事件（同一主题/环节）的相邻段合并
2. 拆分：某段内主题变化则拆分

输出 JSON 数组，每个元素：
{
  "start": "HH:MM:SS,mmm",
  "end": "HH:MM:SS,mmm",
  "label": "4-8字动态事件名",
  "summary": "一句话摘要",
  "event_type": "intro|gameplay|interaction|qna|break|outro|other"
}

用户消息:
[段 001] 00:00:00,000 → 00:00:15,000
大家好欢迎来到今天的直播间

[段 002] 00:00:15,000 → 00:00:45,000
今天我们来玩一个新游戏叫做荒野大冒险
...
```

#### Validator 规则

| 检查项 | 规则 | 处理 |
|--------|------|------|
| 时间戳匹配 | 必须在 SRT 中存在 | 自动对齐到最近 SRT 时间点 |
| 连续性 | end_n ≈ start_{n+1}（±1s） | 自动填补微小间隙 |
| 无重叠 | start_{n+1} >= end_n | 裁剪重叠部分 |
| 时长 | 2min ~ 15min | 超限自动截断/合并 |
| 严重问题 | 整批乱码/空数组 | 丢弃 LLM 结果，降级到规则结果 |

---

### Stage5 — Cutter + Publisher

#### 剪切

```
ffmpeg -i "输入.mp4" \
  -ss {keyframe_aligned_start} \
  -to {end} \
  -c copy -avoid_negative_ts make_zero \
  "{input_stem}_{序号:03d}.mp4"
```

**关键帧对齐**：`-ss` 向前对齐到最近的关键帧（避免花屏）。关键帧间隔通过 `ffprobe` 估算。

**字幕偏移**：读取 `transcript.srt`，过滤本段范围条目，时间戳减去 `start` 偏移，写入 `{input_stem}_{序号:03d}.srt`。

**并行**：`tokio::task::spawn_blocking` 最多同时 4 个 ffmpeg 进程。

#### Publisher — publish_config.json

```json
{
  "source_video": "2024-01-15_直播录像.mp4",
  "processed_at": "2024-01-16T10:30:00Z",
  "duration_secs": 7200.0,
  "clips": [
    {
      "index": 1,
      "filename": "2024-01-15_直播录像_001.mp4",
      "title": "开场介绍",
      "duration_secs": 45.0,
      "start_time": "00:00:00,000",
      "end_time": "00:00:45,000",
      "tags": ["intro", "开场"],
      "description": "主播介绍今天直播内容和主题",
      "subtitle_file": "2024-01-15_直播录像_001.srt",
      "cover_image": null,
      "custom_metadata": {}
    }
  ]
}
```

**输出目录结构**：
```
output/
├── 2024-01-15_直播录像_001.mp4
├── 2024-01-15_直播录像_001.srt
├── 2024-01-15_直播录像_002.mp4
├── 2024-01-15_直播录像_002.srt
├── ...
└── publish_config.json
```

---

## 五、配置系统

### 查找顺序

1. `./vtrim.toml`（项目级）
2. `~/.config/vtrim/config.toml`（用户级）
3. 环境变量覆盖（`VTRIM_ASR_KEY`、`VTRIM_LLM_KEY` 等）

### 完整配置

```toml
[audio]
sample_rate = 16000
channels = 1

[separation]
enabled = false
provider = "openai"

[asr]
provider = "openai"
api_key = ""
model = "whisper-1"
language = "zh"
prompt = "直播,游戏,解说"

[detection]
silence_threshold_secs = 3.0
min_segment_secs = 120
max_segment_secs = 900

[detection.llm]
provider = "openai"
api_key = ""
model = "gpt-4o-mini"
temperature = 0.1
max_tokens = 4096
batch_size = 25
batch_overlap = 3

[cutting]
preserve_source = true
max_parallel = 4
output_dir = "./output"
overwrite = false
```

---

## 六、CLI 接口

```
vtrim process <input> [--config <path>] [--output <dir>]
                     [--stages audio,separate,transcribe,detect,cut]
                     [--separation]
                     [--dry-run]

vtrim transcribe <input> [--language <lang>] [--prompt <text>]
vtrim detect-events <transcript.json>
vtrim cut <input> <events.json> [--output <dir>]
vtrim config init
vtrim config show
vtrim version
```

---

## 七、依赖

```toml
# 工作区
[workspace]
members = ["crates/vtrim-core", "crates/vtrim-cli"]

# vtrim-core
tokio = { version = "1", features = ["full"] }
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
tracing = "0.1"
tracing-subscriber = "0.3"
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }
symphonia = "0.6"
reqwest = { version = "0.12", features = ["json", "multipart"] }

# vtrim-cli
clap = { version = "4", features = ["derive", "env"] }
indicatif = "0.18"
```

---

## 八、路线图

| 阶段 | 内容 |
|------|------|
| M1 | workspace + types + config + pipeline 骨架 |
| M2 | Stage1 AudioExtractor + ffprobe |
| M3 | Stage3 ASR（OpenAI API，含大文件分段） |
| M4 | Stage4 规则引擎（静音+密度+时长） |
| M5 | Stage4 LLM 精调（prompt + 分批 + 合并 + 校验） |
| M6 | Stage2 可选云端分离（trait + 骨架） |
| M7 | Stage5 Cutter + Publisher（命名 + publish_config） |
| M8 | CLI 整合 + 进度 + 错误处理 + 测试 |
