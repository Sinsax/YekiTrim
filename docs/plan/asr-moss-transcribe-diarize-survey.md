# ASR 方案探索：MOSS-Transcribe-Diarize 0.9B

> **状态：纯调研，未改任何代码。** 结论先行，证据在后。
> 调研对象：<https://github.com/OpenMOSS/MOSS-Transcribe-Diarize>
> 日期：2026-07 ｜ 对照基线：本项目当前 `vtrim-core/src/asr`（sherpa-onnx，CPU only）
> 姊妹篇（SenseVoice 可替代性）：`asr-sensevoice-replacement-survey.md`

## 一句话结论

**不建议把它接进主链路**；真正值得抄的是它的**「说话人分离（diarization）+ 声学事件」能力**，
而那件事在本项目里有一条**成本低得多**的路（sherpa-onnx 自带的 `OfflineSpeakerDiarization`）。
MOSS 的硬伤不是质量（它质量确实 SOTA），而是**性能模型与「3 小时录播、CPU-only、多租户 server」这个场景正面冲突**。

## 1. 它是什么

| 项 | 值 |
|---|---|
| 参数量 | 0.9B |
| 架构 | Whisper-Medium encoder（24 层，80 mel，30s chunk）+ VQAdaptor（4× 时间合并 + MLP）+ Qwen3-0.6B 风格因果解码器（28 层，GQA，QK-norm，NEOX RoPE） |
| 音频前端 | `WhisperFeatureExtractor`，16 kHz，80 mel，**30 秒分块** |
| 输出 | 单条自回归流：`[起始秒][Sxx]文本[结束秒]`，说话人标签 `[S01]`/`[S02]`… |
| 语言 | 50+ |
| 许可 | 权重 **Apache-2.0**；C++ 移植 `moss-transcribe.cpp` 为 **MIT** |

关键点：**这是一个 audio-LLM，不是传统 ASR**。它把「转写 + 说话人分离 + 时间戳 + 声学事件」合成**一次自回归解码**产出，
所以没有独立的 VAD / 声学模型 / 说话人聚类三个阶段 —— 这既是它的卖点，也是它**代价与音频长度超线性相关**的根因。

官方榜单（[README 评测表](https://github.com/OpenMOSS/MOSS-Transcribe-Diarize)）：

| 模型 | AISHELL-4 CER↓ | Alimeeting CER↓ | Podcast CER↓ | Movies CER↓ |
|---|---|---|---|---|
| Gemini 3 Pro | 22.75 | 26.75 | — | 8.62 |
| **MOSS 0.9B** | **14.84** | **24.86** | **5.97** | **6.36** |
| MOSS Pro（闭源 API） | 13.78 | 18.22 | 4.46 | 5.86 |

中文会议场景 **明显强于 Gemini 3 Pro**，且 Δcp（说话人感知差异）表现最好。**质量不是问题。**

## 2. 致命问题：性能模型与录播切片场景冲突

### 2.1 CPU RTF 随音频长度**上升**，长音频直接掉出实时

来自 [`moss-transcribe.cpp` 基准](https://github.com/mudler/moss-transcribe.cpp/blob/master/benchmarks/BENCHMARK.md)，
20 核 x86、8 线程、F32：

| 音频长度 | moss-transcribe.cpp | PyTorch (torch CPU) | 速度比 |
|---|---|---|---|
| 11 s | 6.5 s（**RTF 0.59**） | 10.5 s（RTF 0.96） | 1.62× |
| 44 s | 24.2 s（**RTF 0.55**） | 43.0 s（RTF 0.98） | 1.78× |
| 132 s | 102.8 s（**RTF 0.78**） | 162.1 s（**RTF 1.23**） | 1.58× |

RTF **随长度单调上升**（解码是自回归的，上下文随音频变长；注意力代价随序列增长）。
上游自己写的原话：

> **"for hour-long audio on CPU you are looking at a long run for either engine, the reference model targets GPU."**
> **"neither is suited to hour-long audio on CPU."**

外推一下（工程估算，不是实测）：132 s 时 RTF 已 0.78。
**3 小时录播 ≈ 10800 s × RTF ≈ 2.3 小时**（按 RTF 0.78 平算，实际会更差）。
本项目当前基线是 **184 分钟真实录播 83.7 s 跑完（≈ 1.3 分钟，RTF ≈ 0.0076）**
（见 `docs/plan/vad-single-mode.md`）。

**差距三个数量级。** 这不是"慢一点"，是"从分钟级变成小时级"。

### 2.2 量化救不了量级

[量化阶梯](https://huggingface.co/mudler/moss-transcribe.cpp-gguf)（11 s 片、8 线程）：

| dtype | 体积 | 耗时 | vs f32 | 转写一致性 |
|---|---|---|---|---|
| f32 | 3.4 GB | 7.83 s | 1.0× | 字节一致（对齐基线） |
| q8_0 | 942 MB | 3.97 s | 2.0× | 字节一致 |
| q5_k | 619 MB | 4.47 s | 1.8× | 字节一致 |
| q4_k | 511 MB | 3.81 s | 2.1× | 词级一致 |

最好的情况（q4_0，2.2×）也只能把上面那个估算从 2.3 小时压到约 1 小时。
**对"上传录播 → 等切片"的产品形态，仍然是不可接受的等待时间。**

### 2.3 与本项目架构的三处正面冲突

1. **多租户 server**：`vtrim-server` 是任务队列 + 多用户。MOSS 单请求要吃掉一个核群几十分钟，
   而当前 sherpa-onnx 是 CPU/4 路并行（`asr/local.rs` 的 `num_workers`，上限 4）+ 每 chunk intra-op 2 线程。
   一个 MOSS 请求会把整个 CPU 池占死，**队列里其他人的任务全部饿死**。
2. **GPU 也不是解药**：官方 GPU 数据（Blackwell）132 s → ~46 s（RTF 0.35），**只比 CPU 快一倍多**，
   且"on par with PyTorch"，不是碾压。为一个 0.9B 模型引入 CUDA 工具链 + 3.4 GB 权重，
   换来的 RTF 仍远差于现在的纯 CPU sherpa。
3. **本项目已经有的 VAD 优化会被完全废掉**：`single` 模式用 silero 只解码语音段（实测 **2.34×**，
   因为真实录播"真有字"的时间只占 33~43%）。MOSS 是端到端模型，**跳过静音段这件事它自己做不了**，
   你没法只喂语音段 —— 它自己的分块是固定 30 s。也就是说这项优化对它**结构性失效**。

## 3. 但它有一个真本事：Diarization（说话人分离）

这是 MOSS 唯一对本项目有**结构性**价值的地方。
V-Trim 现在**完全没有说话人概念** —— `TranscriptSegment` 只有 `start`/`end`/`text`。
多人联动（几个主播 / 连麦 / 嘉宾）的录播，现在只能切成一堆无主的字幕，
**事件检测和字幕排版都拿不到"谁在说"**。

参考 [`transcript_parser.cpp`](https://github.com/mudler/moss-transcribe.cpp/blob/master/src/transcript_parser.cpp)：
它的输出里每段自带 `speaker`（`[S01]`、`[S02]`，按出现顺序编号），
解析器把 `[start][Sxx]text[end]` 拆成 `{start, end, speaker, text}` —— 契约非常干净。

### 3.1 低成本替代路线：sherpa-onnx 自带 diarization

**本项目已经在用 sherpa-onnx 了**（`vendor/sherpa-onnx-sys`），而 sherpa-onnx **自带离线说话人分离**：

- Rust API：`OfflineSpeakerDiarization` / `OfflineSpeakerSegmentationModelConfig` /
  `FastClusteringConfig`（见 [sherpa-onnx Rust 示例](https://github.com/k2-fsa/sherpa-onnx/blob/master/rust-api-examples/examples/offline_speaker_diarization.rs)）
- 模型：pyannote segmentation + 3D-Speaker 声纹嵌入（**都是 ONNX，CPU 可跑**）

也就是说：**不需要引入任何新运行时、新语言、新权重格式**，
在现有 `vendor/sherpa-onnx-sys` 上就能拿到说话人时间轴，
再把它**按时间轴叠加到现有 `CharTimeline` 上**（`asr/align.rs` 的 `slice_by_time` 已经是按时间取字的地方）。

这需要评估：
- 两个额外 ONNX 模型的体积/下载链路（接进 `download.rs` 的 `MODEL_REGISTRY`）；
- 额外的推理耗时（segmentation + 声纹嵌入，**非自回归，代价与音频长度线性**，和 MOSS 完全不是一个量级）；
- `TranscriptSegment` 加 `speaker: Option<String>` 字段（**要动 `types.rs`，属于跨模块改动**）；
- 下游消费方：字幕排版（`vtrim-polish`）、事件检测 prompt（`vtrim-core/src/detection`）是否要用 `[S01]`。

### 3.2 顺带一提：声学事件（acoustic event）

MOSS 还能标注声学事件（音乐/笑声之类）。本项目的 `firered-zh` 当年正是因为
**"CTC 时间戳在 BGM 段造幽灵时间戳"** 下线的（见 `asr/AGENTS.md`）。
MOSS 是自回归生成时间戳，**理论上不会有 CTC 那种幽灵戳**，这一点值得记一笔；
但因为它解决的是"我们已经有方案"的问题，优先级低于 diarization。

## 4. 如果真要接，接入形态对比

| 方案 | 做法 | 评价 |
|---|---|---|
| **A. 子进程调 CLI** | 编译 `moss-transcribe`，`Command` 调 `transcribe model.gguf audio.wav --format json` | **最贴合现有形态**。项目已有从零编译 C 库的先例（`vendor/sherpa-onnx-sys/build.rs`），CLI 的 `--format json` 直接吐 `{start,end,speaker,text}`，正好对上 `TranscriptSegment`。但引入 CMake/ggml 编译链 + 619 MB~3.4 GB 权重 |
| B. 链接 C API | `moss-transcribe.cpp` 提供 `MT_SHARED=ON` 的 `libmoss-transcribe.so` | 目前 C API 极薄（`include/moss_transcribe.h` **只有一个 `version()`**），Flat C-API 还在 roadmap，**现在接不了** |
| C. HTTP 服务 | vLLM / SGLang Omni 起 `/v1/audio/transcriptions` | 需要 Python + CUDA 环境，**与"本地 CPU、零外部依赖"的部署面冲突**；但适合"用户自己有 GPU 机器"的场景，可作为**可选远程后端** |
| D. Python 直接调 | transformers + torch | 引入 Python 运行时，**与整个项目的 Rust/sherpa 方向背道而驰**，最快出结果、最不适合入库 |

**注意许可面**：MOSS 是 Apache-2.0，`moss-transcribe.cpp` 是 MIT —— 都不触发 GPL。
`vendor/sherpa-onnx-sys/build.rs:337-366` 的占位断言只检查 sherpa-onnx 自己的 lib 目录里
三个 stub（`espeak-ng.lib` / `piper_phonemize.lib` / `ucd.lib` 各 ≤4096 B），
新增一个**独立**的 GGUF 引擎（或 HTTP 客户端、子进程）**根本不会进这条代码路径**。
真正会触发它的只有两件事：改成非 `windows-x86_64+static` 目标（`:153-163`），
或**删掉 workspace 根 `Cargo.toml` 的 `[patch.crates-io]` sherpa-onnx-sys 条目**（`:159-161` 明写了这会退回上游的 GPL espeak-ng 链接）。第二道闸是 `cargo xtask scan`。
⚠️ 但有一个**新的许可问题**：MOSS 的音频编码器是 **Whisper-Medium 架构**。
架构本身是 MIT（OpenAI 代码），但**从 Whisper 权重蒸馏/初始化的权重**会带上 MIT/权重条款 ——
本项目的 `THIRD-PARTY-NOTICES.md` 目前只覆盖 sherpa-onnx 系权重，
**引入 MOSS 前必须补一条**（`AGENTS.md` 明确要求"改依赖前先看 THIRD-PARTY-NOTICES.md"）。
这条要按**法律口径**核实，不是工程判断。

## 4.1 ⚠️ 顺带发现的既有隐患（与 MOSS 无关，但接任何第三个后端都会踩）

做完代码面清点后发现三处**现存**结构问题。它们今天不出事只是因为**恰好只有两个模式**，
一旦加第三个后端（不论是不是 MOSS）就会变成真 bug：

1. **Hybrid 是 `else` 兜底分支**（`crates/vtrim-core/src/asr/mod.rs:149`）：
   `mode` 是**自由字符串、没有任何校验**（`config/types.rs:530`，默认 `"hybrid"`），
   而分派是 `if mode == "single" { … } else { …qwen3 hybrid… }`。
   **拼错或新增的第三个模式会静默跑 qwen3 hybrid 路径**，不报错、不提示 ——
   用户以为在用新后端，实际跑的是旧链路。
2. **`ASR_MODES` 是定长 `[&str; 2]`**（`config/types.rs:82`，注释自己写着"加模式要同步改这里"），
   它驱动 `vtrim-service/src/assets.rs:276-291` 的**素材槽位**计算
   （用户口径的"输出一次绑定一次、只能两次"里的那个 2）。
   加模式必须同步改长度，否则第三个模式的产物**绑不上槽**。
3. **`mode_slug` 在两处重复实现**（`asr/mod.rs:401-413` 与 `vtrim-service/src/assets.rs:356-368`
   的 `asr_artifact_name`），后者注释明确要求两者保持一致 —— 改一处忘另一处，
   产物文件名对不上，就是"槽明明占了却看不见"。

另外几处静默兜底（`asr/local.rs:112-115` 的 `find_model`、
`download.rs:287-298` 对未知 `[asr] model` **静默替换成 sensevoice**、
`vtrim-server/src/api/config.rs:37-38` 的第三个按模型名 `match`、未知模型显示成"879 MB / 最准最慢"）
都指向同一个结论：**新增 ASR 后端在这个仓库不是加一个分支，而是横跨
core / service / server / webui 四层的 39 处改动**（清单见附录 A）。

> 这三条**不是本次要修的**（本次只调研）。但它们是"要不要加第三种 ASR 后端"这个决策里的**真实成本项**。

## 5. 建议

1. **不把 MOSS 接进主链路**（Stage 2）。性能模型（RTF 随长度上升、CPU 长音频掉出实时、GPU 也仅约 1 倍提升）
   与"3 小时录播 / CPU-only / 多租户队列"正面冲突，且会**废掉已有的 VAD 2.34× 优化**。
2. **把 MOSS 记为「可选远程后端」候选**：如果将来支持"用户自己配一台 GPU 机器跑 ASR"，
   它的 OpenAI 兼容 `/v1/audio/transcriptions`（vLLM/SGLang）是**干净的接入点**，
   和 `[llm] base_url` 的形态一致，不需要碰编译链。
3. **优先做的替代项：用 sherpa-onnx 自带的 `OfflineSpeakerDiarization` 补说话人维度**。
   同运行时、同权重格式、线性代价，直接补上本项目当前**完全没有**的能力。
   落地前需要一次独立评估（额外模型体积 / 耗时 / `TranscriptSegment.speaker` 的跨模块影响 / 下游是否消费）。
4. **不修改任何代码**的情况下，本调研已给出 spec；真要动手建议**先做第 3 项**（收益/风险比最好）。

## 6. 附：已核实的引用

- 模型仓库：<https://github.com/OpenMOSS/MOSS-Transcribe-Diarize>
- 权重（Apache-2.0）：<https://huggingface.co/OpenMOSS-Team/MOSS-Transcribe-Diarize>
- C++/ggml 移植（MIT）：<https://github.com/mudler/moss-transcribe.cpp>
- GGUF 权重（Apache-2.0，q4_k~f16）：<https://huggingface.co/mudler/moss-transcribe.cpp-gguf>
- CPU 基准与"不适合小时级音频"的原话：<https://github.com/mudler/moss-transcribe.cpp/blob/master/benchmarks/BENCHMARK.md>
- 转写流解析器（`[start][Sxx]text[end]` → 结构化段）：[`src/transcript_parser.cpp`](https://github.com/mudler/moss-transcribe.cpp/blob/master/src/transcript_parser.cpp)
- CLI（`--format json|srt|ass`）：[`src/cli.cpp`](https://github.com/mudler/moss-transcribe.cpp/blob/master/src/cli.cpp)
- ONNX INT8 版（CPU，第三方导出，Apache-2.0）：<https://huggingface.co/soniqo/MOSS-Transcribe-Diarize-0.9B-ONNX-INT8-ENC>
- sherpa-onnx 自带离线说话人分离（替代路线）：<https://k2-fsa.github.io/sherpa/onnx/speaker-diarization/index.html>
- 本项目当前 RTF 基线：`docs/plan/vad-single-mode.md`（184 分钟 → 83.7 s）
- 本项目 ASR 现状与踩坑：`crates/vtrim-core/src/asr/AGENTS.md`

## 附录 A：新增第三个 ASR 后端的改动清单（代码面清点结果）

> 只列**已核实**的接入点，全部带 `file:line`。这份清单是"成本项"的证据，不是施工单。

**A. 模式字符串本身**（`mode` 是自由 `String`，无枚举、无校验）
- `config/types.rs:526-530`（字段 + 只写了两个值的文档注释）、`:626-630`（`default_asr_mode`）、
  `:649`（手动 `Default` impl）、`:78-82`（`ASR_MODES` 定长数组）、`:96`（文档）
- `config/mod.rs:521-522`（merge 条件）+ `GLOBAL_CONFIG_TOML` 模板

**B. Stage 2 分派**（`crates/vtrim-core/src/asr/mod.rs`，813 行）
- `:131`（`mode` 局部量）、`:149`（`if single … else hybrid` 兜底）
- `:142-147`（**两个模式共用**的无条件 SenseVoice 打轴 + `ensure_ready`；无 SenseVoice 的后端要绕开它）
- `:325-332` / `local.rs:412-419`（`AsrMetadata` 硬编码 model/mode）、`:364-368`（`language: "zh"` 硬编码）
- `:386-390` + `:399-413`（`mode_slug` 文件名后缀）、`:132`（`SplitPolicy::from_config`）
- `:336-346` + `artifacts.rs:91-102/137-143`（中间产物要求 `&[RawChunk]` + `CharTimeline`
  —— 外部 HTTP 后端没有逐块 token，要么造要么绕）

**C. 模型解析 / 下载**（`download.rs` 479 行）
- `:22-35`（`MODEL_REGISTRY` 四元组）、`:339-344`（`required_models`，测试在 `:462-478` 钉住 `hybrid`）
- `:287-298`（未知 `[asr] model` **静默退回 sensevoice**）、`:86-115`（裸 ONNX 走另一条下载路）
- `local.rs:112-115`（`find_model` 的模型名 `match`）、`:982-992`（`ensure_ready` 只认 SenseVoice）
- `vtrim-server/src/api/config.rs:37-38`（第三个按模型名 `match`）、`:350`（测试钉死 `["sensevoice","qwen3-asr"]`）

**D. 产物与绑定**（`vtrim-service`）
- `assets.rs:276-291`（`ASR_MODES` 槽位循环）、`:351-368`（**与 `mode_slug` 重复的** `asr_artifact_name`）、
  `:392-398`（`bind_asr_artifact`，唯一登记通道）、`:703-718`（`slot_bound`）
- `core_pipeline.rs:40-46`（`save_transcript_after_asr`）、`:279-282` / `:368-370` / `:381-383`

**E. 输出契约**（后端必须满足，与模式无关）
- `types/mod.rs:57-63`（`Transcript`）、`:87-96`（`TranscriptSegment`）、`:66-85`（`AsrMetadata`）
- 不变量：段**单调、不重叠**（`asr/mod.rs:322-323`、`:810`、`align.rs:697`）、无空文本
- 固定名 `video.srt` / `transcript.json` 被 `config/types.rs:174-178`（`current_stage`）、
  `cutting/mod.rs:495-496` 认死

**F. 前端**（两个 `<select>` 各自硬编码两个选项，且**默认值不一致**）
- `webui/src/views/PrepareView.vue:75-76`（选项）、`:348`（默认 `'hybrid'`）、`:361-376`、`:410/424/434`、
  `:482`、`:650-651`、`:828-834`
- `webui/src/views/SettingsView.vue:52-56`、`:297`、`:304-320`、`:377-392`、
  `:455-472`（这里的默认是 `'single'`，与 PrepareView 相反）
- `webui/src/api/projectData.js:41`；改完**必须重建 `webui/dist`**（已入库）
- 断言"只有两个模式"的文档共 7 处（含 `webui/src/AGENTS.md:100-108`、`views/AGENTS.md:110-112`）

**G. 依赖 / 许可面**
- `vendor/sherpa-onnx-sys/build.rs:337-366`（stub 断言，**不约束**新的非 sherpa 依赖）、
  `:153-163`（目标三元组守卫）、`:159-161`（删 `[patch.crates-io]` 的警告）
- 第二道闸 `cargo xtask scan` + `THIRD-PARTY-NOTICES.md`
- ⚠️ 分层：`crates/vtrim-core/AGENTS.md` 禁止 core `use` 上层；
  外部引擎的编排若需要 service 级设施，**只能注入（sink/回调/参数）或放在 `vtrim-service`**，
  由 `scripts/check_module_boundaries.py` 机器检查
