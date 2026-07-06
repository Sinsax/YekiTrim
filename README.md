# V-Trim

直播录像自动切片工具。输入录播视频 + 弹幕文件，输出按事件分割的片段 + 字幕 + `publish_config.json`。

## 前置条件

- **ffmpeg** + **ffprobe** — 需在 PATH 中，用于音频提取和视频剪切
- **Rust** 工具链 (构建用)

## 项目结构

```
.
├── crates/               # 源码
│   ├── vtrim-core/       #   管线编排 + 业务逻辑
│   ├── vtrim-cli/        #   CLI 入口 (clap)
│   └── vtrim-polish/     #   GSAP 精修引擎
├── docs/archived/        #   历史设计文档
├── AGENTS.md
├── Cargo.toml / Cargo.lock
└── vtrim.toml            # 项目配置 (gitignored)
```

## 快速开始

```bash
# 构建
cargo build

# 创建项目 (--danmaku 可选，自动搜索同目录 .ass)
vtrim init input.mp4

# 下载 ASR 模型 (本地转写需要)
vtrim download-models

# 全自动管线
vtrim process
```

## CLI 命令

全局选项：

| 选项 | 作用 |
|------|------|
| `-c, --config <path>` | 指定配置文件路径 (默认 `./vtrim.toml`)，对所有子命令生效 |

### `vtrim init <video>`

创建 `vtrim.toml` 项目配置文件。

| 参数 | 作用 |
|------|------|
| `<video>` | 输入视频文件路径 **(必填)** |
| `--danmaku <file>` | 弹幕文件 (ASS 格式)。不传则自动寻找 `<video>.ass` |
| `-o, --output <dir>` | 默认 `./output` |
| `--config <path>` | `vtrim.toml` 输出路径 (默认 `./vtrim.toml`) |

自动发现：`--danmaku` 不传时，自动检查视频同名 `.ass` 文件，存在则写入配置。

### `vtrim process`

全自动管线处理。无视 `current_stage` 状态机，从头跑到尾，完成后设 `current_stage=4`。

| 参数 | 作用 |
|------|------|
| `-i, --input <file>` | 输入视频 (默认 `vtrim.toml` 中 `input_file`) |
| `-o, --output <dir>` | 输出目录 (默认 `vtrim.toml` 中 `output_dir`) |
| `--dry-run` | 仅分析不剪切，跑到 EventDetector 为止 |
| `--stages <stages>` | 逗号分隔指定阶段，默认 `audio,transcribe,detect,cut` |

流程：
1. 音频提取 (如未完成)
2. 语音转写 (如未完成，自动恢复已有 `transcript.json` / `video.srt`)
3. LLM 订正 (如 `asr.llm_correct=true` 且配置了 LLM key，且事件尚未检测)
4. 事件检测 (如未完成)
5. 视频剪切 (如未完成，`--dry-run` 跳过)

### `vtrim extract-audio`

Stage 1 — 从视频提取 16kHz mono WAV。

| 参数 | 作用 |
|------|------|
| `-i, --input <file>` | 输入视频 (默认 `vtrim.toml` 中 `input_file`) |
| `-o, --output <dir>` | 输出目录 (默认 `vtrim.toml` 中 `output_dir`) |

输出：`<output_dir>/video.wav`，自动写入 `vtrim.toml` 的 `audio_file`。

**状态机约束**：`current_stage >= 0` 即可运行。

### `vtrim transcribe`

Stage 2 — 语音转文字 (local sherpa-onnx SenseVoice)。

| 参数 | 作用 |
|------|------|
| `-i, --input <file>` | 输入音视频 (默认 `vtrim.toml` 中 `input_file`，自动回退 `audio_file`) |
| `-o, --output <dir>` | 输出目录 (默认 `vtrim.toml` 中 `output_dir`) |

输出：
- `<output_dir>/video.srt` — 字幕 (UTF-8 BOM)
- `<output_dir>/transcript.json` — 结构化 JSON

自动写入 `vtrim.toml` 的 `transcript_file`。

并行策略：CPU 核数 / 4 (上限 4)，每 chunk 的 ONNX intra-op threads=2。

配置参数 (`.vtrim/config.toml`)：

| 参数 | 默认 | 说明 |
|------|------|------|
| `num_workers` | `0` | 0=auto (CPU/4, 上限4)，手动值不超过 CPU 核数 |
| `chunk_duration` | `60.0` | 每块秒数 |
| `chunk_overlap` | `10.0` | 块间重叠秒数 |
| `segment_gap` | `1.0` | 段间静音阈值(秒)，超过才切为两条字幕 |
| `language` | `"zh"` | ASR 语言 |

**状态机约束**：需先完成 `extract-audio` (current >= 1)。

### `vtrim correct`

LLM 订正转录文本。

| 参数 | 作用 |
|------|------|
| `-t, --transcript <file>` | 转录 JSON/SRT 文件 |

自动发现优先级：`--transcript` 参数 > `vtrim.toml` 中 `transcript_file` > `<output_dir>/transcript.json`。

输出：`<output_dir>/video_llm.srt`，自动更新 `transcript_file` 指向此文件。

需要 `.vtrim/llm.toml` 已配置 API key。无状态机约束 (可独立运行)。

### `vtrim detect-events`

Stage 3 — LLM 分析转写结果，检测高价值事件。

| 参数 | 作用 |
|------|------|
| `-t, --transcript <file>` | 转录 JSON/SRT 文件 |

自动发现优先级：`--transcript` 参数 > `vtrim.toml` 中 `transcript_file` > `<output_dir>/transcript.json` > `<output_dir>/video.srt`。

输出：`<output_dir>/events.json` (EventStore 格式)。

自动写入 `vtrim.toml` 的 `events_file`。

检测逻辑：
- LLM API key 非空 → 调用 LLM 分析全部转录段 (单次调用，64k+ 上下文)
- LLM API key 为空 → 降级为单个默认事件 (覆盖整段视频)
- 弹幕文件存在 → 计算 10s 窗口密度，输出 `popularity` (0~1) + `rank`
- 主播名称 (`detection.vtuber_name`) 非空 → 自动追加到标题末尾

**状态机约束**：需先完成 `transcribe` (current >= 2)。

### `vtrim cut`

Stage 4 — 按事件片段剪切视频 + 生成字幕。

| 参数 | 作用 |
|------|------|
| `-i, --input <file>` | 输入视频 (默认 `vtrim.toml` 中 `input_file`) |
| `-e, --events <file>` | 事件 JSON 文件 |
| `-o, --output <dir>` | 输出目录 (默认 `vtrim.toml` 中 `output_dir`) |

自动发现优先级：`--events` 参数 > `vtrim.toml` 中 `events_file` > `<output_dir>/events.json`。

剪切输出结构：

```
<output_dir>/
└── clips/
    ├── clip-01/
    │   ├── clip.mp4           # 视频片段
    │   ├── clip.srt           # 裁剪后字幕
    │   ├── clip.dm.srt        # 弹幕字幕 (有弹幕时)
    │   └── clip.json          # 元数据 (标题、热度、原始时间范围)
    ├── clip-02/
    │   └── ...
    ├── clips_order.txt        # 按弹幕热度降序排列
    └── publish_config.json    # 发布配置
```

编码器自动检测顺序：`h264_nvenc` → `h264_qsv` → `h264_amf` → `libx264`。
字幕从原始 SRT 裁剪时间范围并偏移到 clip 起始时间。

**状态机约束**：需先完成 `detect-events` (current >= 3)。

### `vtrim silence-remove`

对已输出的 clip 视频移除长静音段 (重编码)。

| 参数 | 作用 |
|------|------|
| `[files...]` | 要处理的视频文件。不传则扫描 `<cwd>/output/clips/*/clip.mp4` |
| `--duration <sec>` | 静音阈值(秒)，默认 `2.0`。超过此长度的连续静音将被移除 |
| `-o, --output <dir>` | 输出目录 (默认输入文件同目录) |

输出：`{input_stem}_cleaned.mp4`，同步调整 `{input}.srt` 和 `{input}.dm.srt` 字幕时间轴。

静音检测策略：如有同名 `.srt` 文件，优先分析 SRT 字幕间隔；否则用 ffmpeg silencedetect filter。

### `vtrim download-models`

下载 sherpa-onnx binary + SenseVoice 模型到 `.vtrim/models/sensevoice/`。

### `vtrim llm-test`

测试 LLM 连接。

| 参数 | 作用 |
|------|------|
| `-p, --prompt <text>` | 测试提示词 (默认 "你好，请用一句话介绍你自己。") |
| `-o, --output <file>` | 输出文件 (默认 `llm_test_response.json`) |

返回 LLM 完整 API response body。需要 `.vtrim/llm.toml` 已配置。

### `vtrim status`

显示当前处理进度。

```
视频工程: input.mp4
当前进度: Stage #2

  [✓] AudioExtractor
  [→] Transcriber
  [·] EventDetector
  [·] Cutter
```

### `vtrim config init`

生成全局配置文件。

| 参数 | 作用 |
|------|------|
| `--path <path>` | 输出路径 (默认 `.vtrim/config.toml`) |

生成文件：
- `.vtrim/config.toml` — ASR、检测等全局参数，含内建 prompt 模板
- `.vtrim/llm.toml` — LLM 独立配置 (API key、model 等)

### `vtrim config show`

显示合并后的完整配置 (vtrim.toml + .vtrim/config.toml)。

### `vtrim version`

显示版本号。

---

### 视频精修 (`vtrim polish`)

精修引擎，基于 GSAP 为已剪切 clip 添加特效字幕、弹幕渲染、封面生成、预览。

所有子命令接受一个可选的 `<dir>` 参数，为 clip 目录 (默认当前目录)。精修项目需要 `config.toml` 和 `llm.toml` 位于该目录下。

| 子命令 | 作用 |
|--------|------|
| `init [<dir>]` | 初始化精修项目，创建 `config.toml` + `llm.toml` + `data/` + `sfx/`，自动解析素材 |
| `llm [<dir>]` | LLM 分析生成 segments 和关键词 (使用 `.vtrim/llm.toml`) |
| `process [<dir>]` | 后处理：解析 emoji/irasu keyword、下载资源、计算位置 |
| `render [<dir>]` | 渲染视频，自动先跑 process |
| `cover [<dir>] [--template <t>]` | 生成封面，可选模板: `left-text` / `center-text` / `right-text` / `minimal` |
| `preview [<dir>] [mode]` | 浏览器预览，自动先跑 process。mode: `video` / `cover` / 留空(双端口) |
| `doctor [<dir>]` | 系统检查：ffmpeg、Chrome、LLM 配置 |
| `test [<dir>]` | 测试模式，生成测试素材 + 预览 |

## 管线流程

| 阶段 | Stage # | 输入 | 输出 |
|------|---------|------|------|
| AudioExtractor | 1 | 视频 (mp4) | WAV (16kHz mono) |
| Transcriber | 2 | WAV | SRT + JSON |
| EventDetector | 3 | 转录 JSON + 弹幕 ASS | events.json |
| Cutter | 4 | 视频 + events.json | clips + 字幕 + publish_config |

Stage 编号连续无跳过。所有 stage 通过 `#[async_trait]` 实现 `Stage` trait。

### 状态机规则

`vtrim.toml` 中 `current_stage` 控制单 stage 命令：

| 规则 | 示例 (current=2) |
|------|-----------------|
| `stage > current + 1` → 拒绝 | `detect`(3=2+1) ✅ / `cut`(4>3) ❌ |
| `stage <= current` → 可重跑 | `transcribe`(2≤2) ✅ |
| 完成后 stage 自动推进 | `transcribe`(2) → `current_stage=2` |
| `process` 无视 state, 跑完设 `current_stage=4` | |

### 单个 Stage 命令运行模式

- `run()` — 受 `current_stage` 状态机约束，不能跳过
- `run_all()` — 无视状态机从头跑到底 (`process` 命令)
- `run_until(max)` — 跑到指定 stage 为止 (`process --dry-run`)

## 配置

双层配置合并：`vtrim.toml` (项目) → `.vtrim/config.toml` (全局，后加载覆盖前者)。无环境变量。

合并规则：String 字段 `!is_empty()` 时覆盖，数值字段 != 默认值时覆盖。

### `vtrim.toml` — 项目配置

```toml
[project]
input_file = ""              # 输入视频
danmaku_file = ""            # 弹幕文件
transcript_file = ""         # 转写文件 (由 transcribe 写入)
audio_file = ""              # 音频文件 (由 extract-audio 写入)
events_file = ""             # 事件文件 (由 detect-events 写入)
current_stage = 0            # 当前进度 (0=未开始, 1~4=各阶段)

[asr]
backend = "local"            # ASR 后端

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
output_dir = "./output"
overwrite = false
```

### `.vtrim/config.toml` — 全局配置

```toml
[asr]
num_workers = 0              # 0=auto(CPU/4,上限4)
chunk_duration = 60.0        # 每块秒数
chunk_overlap = 10.0         # 块间重叠
segment_gap = 1.0            # 段间静音阈值(秒)
language = "zh"              # ASR 语言

[detection]
vtuber_name = ""             # 主播名称 (追加到标题末尾)
danmaku_window_secs = 10

[cutting]
output_dir = "./output"
overwrite = false
```

### `.vtrim/llm.toml` — LLM 独立配置

```toml
base_url = ""                # 留空=OpenAI
api_key = ""
model = ""                   # 留空=使用各模块默认模型
temperature = 0
max_tokens = 65536
extra_params = { seed = 42, thinking = { type = "disabled" } }
```

## 输出

```
<output_dir>/
├── video.wav                # 音频
├── video.srt                # 字幕 (UTF-8 BOM)
├── video_llm.srt            # LLM 订正字幕 (如有)
├── transcript.json          # 结构化转写
├── events.json              # 检测的事件列表
├── publish_config.json      # 发布配置
└── clips/
    ├── clips_order.txt      # 按弹幕热度降序
    ├── clip-01/
    │   ├── clip.mp4
    │   ├── clip.srt         # 裁剪后字幕
    │   ├── clip.dm.srt      # 弹幕字幕
    │   └── clip.json        # 元数据
    └── clip-02/...
```

## 开发

```bash
cargo build                    # 0 warnings
cargo test --lib danmaku       # ASS 解析 + 密度计算测试
cargo run -- status            # 查看当前进度
```

## 许可

MIT
