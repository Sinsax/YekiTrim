# V-Trim

直播录像自动切片工具。输入录播视频 + 弹幕文件，输出按事件分割的片段 + 字幕 + `publish_config.json`。

## 架构

Cargo workspace, 两个 crate:
- `crates/vtrim-core` — 管线编排 + 全部业务逻辑
- `crates/vtrim-cli` — CLI 入口 (clap)

**Stage numbering跳过了 2**: AudioExtractor(1) → Transcriber(3) → EventDetector(4) → Cutter(5)。stage 2 无实现，`status` 显示为 "Unknown (可选)"。

所有 stage 用 `#[async_trait]` 实现 `Stage` trait (`crates/vtrim-core/src/pipeline/stage.rs`)。

`PipelineRunner` 提供三种运行模式 (`runner.rs`):
- `run()` — 受 `current_stage` 状态机约束，单 stage 命令用
- `run_all()` — 无视状态机从头跑到底，`process` 命令用，完成后设 `current_stage=5`
- `run_until(max)` — 跑到指定 stage 为止，`process --dry-run` 用

## CLI

```
vtrim init <video> [--danmaku <file>]    # 创建 vtrim.toml + 立即提取音频 + 设 current_stage=1
vtrim process [-i] [-o] [--dry-run]      # 全自动管线 (无视 state)
                                         # --dry-run: 只到 EventDetector, 不剪切
vtrim extract-audio [-i]                 # Stage 1
vtrim transcribe [-i] [--backend]        # Stage 3 (local/bcut/kuaishou)
vtrim detect-events [-t <transcript>]    # Stage 4 (自动发现: 参数 > config > output_dir/*.json > *.srt)
vtrim cut [-i] [-e <events>] [-o]        # Stage 5 (自动发现: 参数 > output_dir/{stem}_events.json)
vtrim silence-remove [files...]          # 对已输出 clips 移除长静音段 (重编码)
       [--duration 2.0] [-o <dir>]      # 默认扫描 output/clips/, 输出到 output/clips_cleaned/
vtrim download-models [--model]          # sherpa-onnx binary + 模型 → ~/.vtrim/
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
| `stage > current + 1` → 拒绝 | `transcribe`(3=2+1) ✅ / `detect`(4>3) ❌ |
| `stage <= current` → 可重跑 | `extract-audio`(1≤2) ✅ |
| 完成后 stage 自动推进 | `transcribe`(3) → `current_stage=3` |
| `process` 无视 state, 跑完设 `current_stage=5` | |
| 禁用阶段自动跳过 | `is_enabled()=false` 时 `current_stage+=1` 并跳过 |

实现: `config/mod.rs` 的 `save_current_stage()` 用 `toml_edit` 原地修改 `vtrim.toml` 保留注释和格式。

## 配置文件

三源合并: 默认值 → `~/.config/vtrim/config.toml` → `./vtrim.toml` → 环境变量。模板通过 `include_str!("../../../config/default.toml")` 编译嵌入。

关键字段:

```toml
input_file = ""
danmaku_file = ""
transcript_file = ""      # vtrim transcribe 自动写入
current_stage = 0

[detection]
danmaku_window_secs = 10

[detection.llm]
api_key = ""
api_base = ""             # 留空=OpenAI, DeepSeek="https://api.deepseek.com/v1"
model = "deepseek-chat"

[cutting]
preserve_source = true    # Cutter 始终 -c copy，此字段未使用
max_parallel = 4          # 未使用 (Cutter 单线程)
output_dir = "./output"
```

环境变量: `VTRIM_LLM_KEY`, `VTRIM_LLM_MODEL`, `VTRIM_LLM_BASE_URL`, `VTRIM_OUTPUT_DIR`。

merge 条件: String 字段 `!is_empty()`, f64/u32 字段 != 默认值。

**配置默认值 bug**: `Config::default()` 用 `toml::from_str("")` 初始化，嵌套 struct 的 `Default` derive 给零值而非 serde 字段级默认值。所有配置类型已手动实现 `Default`，但新增字段时必须手动加否则 merge 逻辑出错。

## 各阶段 I/O

### AudioExtractor
- 输入: 视频 → ffmpeg 16kHz mono WAV + ffprobe VideoInfo
- `ffmpeg -y -vn -acodec pcm_s16le -ar 16000 -ac 1` → `output/audio_16khz.wav`

### Transcriber
- 本地 sherpa-onnx-offline (Paraformer/SenseVoice/FunASR-nano, CPU/GPU 自动降级)
- 云端 Bcut(B站) / 快手 ASR API (无 key 要求)
- VAD 模式: sherpa-onnx-vad-with-offline-asr (Silero VAD)
- 120s 定长分块 + 10s 重叠
- 输出: `{stem}.srt` + `{stem}_transcript.json`，自动写 `transcript_file` 到 vtrim.toml

### EventDetector
- 一次 LLM API 调用分析全部 segments (利用 64k+ 上下文窗口), 非分批
- 全中文 prompt: ≥2分钟片段、B 站风格标题、详尽摘要
- 弹幕: ASS 解析 → 10s 窗口归一化密度 → 每个 event 的 `popularity` 0~1
- 按 popularity 降序赋 `rank`，最终按时间排序输出
- 降级: API key 为空或调用失败时使用单个默认事件
- 输出: `{stem}_events.json`

### Cutter
- 单线程 ffmpeg `-c copy` (始终流复制)
- `ffmpeg -ss {start} -i {input} -t {duration} -c copy -avoid_negative_ts make_zero {output}`
- 命名: `{rank:03}_{label}.mp4` → `output/clips/`
- 字幕: 从原始 SRT 裁剪时间范围 + 偏移 → `{rank:03}_{label}.srt`
- 输出: `output/publish_config.json`

## Cutter 未使用的配置字段

- `cutting.preserve_source` — 始终 `-c copy`，设 false 也没用
- `cutting.max_parallel` — 单线程，忽略此值

## PipelineContext 中的未使用字段

- `vocal_path: Option<PathBuf>` — 声源分离预留，无 stage 读写

## 开发

```bash
cargo build                    # 4 dead_code + 1 unused_mut (均为预存在)
cargo test --lib danmaku       # ASS 解析 + 密度计算测试
cargo run -- status            # 查看当前进度
```

前置条件: `ffmpeg` + `ffprobe` 在 PATH 中。

### 新增配置字段步骤

1. `config/types.rs` 加字段 + serde default
2. `config/mod.rs` 的 `merge()` 加对应条件
3. 结构体手动 `Default` impl (有字段级 serde default 时必须)
4. `config/default.toml` + 注释

### 核心文件索引

| 路径 | 说明 |
|------|------|
| `vtrim-cli/src/main.rs:27` | `resolve_input()` input 回退 |
| `vtrim-cli/src/main.rs:41` | `build_pipeline_runner()` 注册 stage |
| `vtrim-cli/src/main.rs:62` | `check_stage()` 状态机 |
| `vtrim-core/src/pipeline/stage.rs` | STAGE_* 常量 + Stage trait |
| `vtrim-core/src/pipeline/runner.rs` | PipelineRunner |
| `vtrim-core/src/config/mod.rs` | 三源合并 + toml_edit |
| `vtrim-core/src/config/types.rs` | Config 类型定义 + Default |
| `vtrim-core/src/types/mod.rs` | Event, PipelineContext, Transcript |
| `vtrim-core/src/download.rs` | 模型/binary 下载 |
| `vtrim-core/src/asr/local.rs` | sherpa-onnx CLI (CPU/GPU 降级) |
| `vtrim-core/src/asr/cloud_asr.rs` | Bcut/快手 ASR |
| `vtrim-core/src/asr/vad.rs` | VAD+ASR |
| `vtrim-core/src/detection/llm.rs` | LLM prompt + API |
| `vtrim-core/src/detection/mod.rs` | EventDetector + 弹幕热度 |
| `vtrim-core/src/danmaku/mod.rs` | ASS 解析 + 密度 |
| `vtrim-core/src/cutting/mod.rs` | Cutter + remove_silence |
