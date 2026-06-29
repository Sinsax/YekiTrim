# V-Trim

直播录像自动切片工具。输入录播视频 + 弹幕文件，输出按事件分割的片段 + 字幕 + `publish_config.json`。

## 前置条件

- **ffmpeg** + **ffprobe** — 需在 PATH 中，用于音频提取和视频剪切
- **uv** (可选) — 如需 Spleeter 人声分离，安装 [uv](https://docs.astral.sh/uv/#installation)
- **Rust** 工具链 (构建用)

## 快速开始

```bash
# 构建
cargo build

# 初始化工程 (提取音频 + 创建 vtrim.toml)
vtrim init input.mp4 --danmaku danmaku.ass

# 下载 ASR 模型 (本地转写需要)
vtrim download-models

# (可选) 安装人声分离
vtrim setup-spleeter

# 全自动管线 (转写 → 事件检测 → 剪切)
vtrim process
```

## 管线流程

| 阶段 | 说明 |
|------|------|
| AudioExtractor | 从视频中提取 16kHz mono WAV |
| Transcriber | 语音转文字 (SRT + JSON)，可选 Spleeter 人声分离 |
| EventDetector | LLM 分析转写，检测高价值事件 |
| Cutter | 按事件片段剪切视频 + 生成字幕 |

### 运行模式

- **单阶段命令**: `vtrim extract-audio` / `vtrim transcribe` / `vtrim detect-events` / `vtrim cut` — 受 `current_stage` 约束，必须按顺序执行
- **全自动**: `vtrim process` — 无视状态机从头跑到底，完成后设 `current_stage=4`
- **仅分析**: `vtrim process --dry-run` — 跑到 EventDetector 为止，不剪切

### 状态机规则

| 规则 | 示例 (current=2) |
|------|-----------------|
| `stage > current + 1` → 拒绝 | `detect`(3=2+1) ✅ / `cut`(4>3) ❌ |
| `stage <= current` → 可重跑 | `transcribe`(2≤2) ✅ |
| 完成后 stage 自动推进 | `transcribe` → `current_stage=2` |
| `process` 无视 state，跑完设 `4` | |

## CLI 命令

所有 stage 命令的 `-i` 可选，默认从 `vtrim.toml` 的 `input_file` 读取。

| 命令 | 作用 |
|------|------|
| `init <video> [--danmaku <file>]` | 创建 `vtrim.toml` |
| `extract-audio [-i] [-o]` | 从视频提取音频 (Stage 1) |
| `process [-i] [-o] [--dry-run]` | 全自动管线无视状态机 |
| `transcribe [-i] [--backend <backend>]` | 语音转文字 (local/bcut/kuaishou，默认 local) |
| `detect-events [-t <transcript>]` | LLM 事件检测 (自动发现: 参数 > config > output_dir/*.json > *.srt) |
| `cut [-i] [-e <events>] [-o]` | 剪切片段 (自动发现: 参数 > output_dir/events.json) |
| `silence-remove [files...] [--duration 2.0] [-o <dir>]` | 移除 clips 中的长静音段 (默认扫 output/clips/，输出到输入文件同目录) |
| `download-models` | 下载 sherpa-onnx binary + SenseVoice 模型 → `.vtrim/` |
| `setup-spleeter` | 安装 Spleeter 人声分离到 `.vtrim/venv/` + 下载预训练模型 |
| `separate [input] [-o <dir>]` | 人声分离 (默认使用 config.audio_file) |
| `status` | 查看当前进度 |
| `config init [--path <path>]` | 生成全局配置文件 `.vtrim/config.toml` |
| `config show` | 显示当前配置 (合并后) |

## 配置

双层配置合并：`vtrim.toml` (项目) → `.vtrim/config.toml` (全局，后加载覆盖前者)。无环境变量。

String 字段 `!is_empty()` 时覆盖，数值字段 != 默认值时覆盖。

### `vtrim.toml` — 项目配置

```toml
input_file = ""
danmaku_file = ""
transcript_file = ""
audio_file = ""
events_file = ""
vocal_file = ""
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
min_segment_secs = 30
max_segment_secs = 900

[cutting]
preserve_source = true      # 无效字段 (Cutter 总是重编码)
max_parallel = 4            # 无效字段 (Cutter 单线程)
output_dir = "./output"
overwrite = false
```

### `.vtrim/config.toml` — 全局配置 (程序目录)

```toml
[asr]
num_threads = 0              # 0=CPU/4 取整，上限 4
chunk_duration = 60.0        # 转写分块长度 (秒)
chunk_overlap = 10.0         # 块间重叠
separation = "none"          # 人声分离: none / spleeter

[detection.llm]
api_key = ""
api_base = ""                # 留空=OpenAI
model = "gpt-4o-mini"
temperature = 0.1
max_tokens = 16384
batch_size = 25
batch_overlap = 3
```

`vtrim config init` 生成 `.vtrim/config.toml`，所有项目共享同一份 LLM 和 ASR 参数。

## ASR 后端

| 后端 | 说明 | 要求 |
|------|------|------|
| `local` | sherpa-onnx-offline (SenseVoice, CPU) | `vtrim download-models` |
| `bcut` | 哔哩哔哩云端 ASR | 无 |
| `kuaishou` | 快手云端 ASR | 无 |

本地 ASR 支持分块并行转写：音频按 `chunk_duration` 切块，`num_threads` 路并行转写，每块用 `--num-threads=2` 的 ONNX intra-op 并行。

## 人声分离 (Spleeter)

可选启用 BGM 分离，分离后仅对人声做 ASR，提升转写准确率：

```bash
vtrim setup-spleeter    # 安装 Spleeter + 下载 2stems 模型 (首次 2-10 分钟)
vtrim separate          # 手动分离 (默认使用 config.audio_file)
```

启用自动分离：`.vtrim/config.toml` 设置 `separation = "spleeter"`。

spleeter 通过 `MODEL_PATH` 环境变量定位预训练模型，所有模型集中缓存在 `.vtrim/pretrained_models/`。`vtrim separate` 在运行前自动检测并修复旧版双层嵌套结构。

## 事件检测

- 使用 LLM (OpenAI / DeepSeek 兼容 API) 分析完整转写文本
- 全中文 prompt，输出 B 站风格标题（含 emoji）+ 详尽摘要
- 弹幕密度归一化为 `popularity` (0~1)，按热度排序赋 `rank`，最终按时间排序输出
- 降级策略：API key 为空或调用失败时使用单个默认事件

## 视频剪切

- 硬件编码自动检测：NVENC → QSV → AMF → libx264
- 命名：`{event_label}.mp4` → `output/clips/` (无编号前缀)
- 字幕：从原始 SRT 裁剪时间范围 + 偏移 → `{label}.srt`
- 输出：`output/publish_config.json` + `output/clips/clips_order.txt` (按弹幕热度降序)

## 输出结构

```
output/
├── video.wav
├── video.srt
├── transcript.json
├── events.json
├── publish_config.json
├── vocals.wav              # 人声分离结果
├── accompaniment.wav
└── clips/
    ├── clips_order.txt      # 按弹幕热度降序排列
    ├── 标题1.mp4
    ├── 标题1.srt
    ├── 标题2.mp4
    └── 标题2.srt
```

## 开发

```bash
cargo build                    # 0 warnings
cargo test --lib danmaku       # ASS 解析 + 密度计算测试
cargo run -- status            # 查看当前进度
```

## 许可

MIT
