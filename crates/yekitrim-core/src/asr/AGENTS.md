# yekitrim-core/src/asr — 转写（SenseVoice 打轴 / qwen3 填词 / VAD / 断句 / 文本后处理）

> 范围：`crates/yekitrim-core/src/asr`/** ｜ 上游：`pipeline::PipelineRunner`（Stage 2）、`yekitrim-service::core_pipeline` ｜ 验证：`cargo test -p yekitrim-core --lib asr`

## 职责

把 `output/video.wav` 转成 `video.srt` + `transcript.json`：sherpa-onnx 本地识别（CPU only）→
逐字时间轴 `CharTimeline` → 断句切分（标点/强停顿/气口/字数）→ 文本后处理 `text::normalize`。
两种模式：`single`（纯 SenseVoice，配 VAD）/ `hybrid`（sensevoice 打轴 + qwen3-asr 填词）。
⚠️ 2026-11-14 review 订正：**没有第三种「qwen3-asr 单模型」模式** —— 代码只有
`if mode == "single" else hybrid` 两岔；`[asr] model` 只影响**下载清单**
（且 hybrid/未知 mode 一律连带 sensevoice，见 `download.rs::required_models`），运行时不按它选模型。

## 入口

- `asr::Transcriber`（Stage 2）。
- `asr::align`：`CharTimeline`（`from_tokens`/`slice_by_time`）、`align_to_timeline`、
  `Alignment::time_of`、`split_text_on_timeline`。
- `asr::local`：`create_recognizer`、`create_qwen3_recognizer`、`transcribe_sensevoice`、
  `ensure_ready`、`decode_threads`、`plan_vad_windows`。
- `asr::vad`：`detect_speech`、`merge_regions`、`rescue_by_level`。
- `asr::text::{normalize, strip_generation_markers}`；`asr::artifacts::{reset, write_all}`；
  `asr::correct::correct_transcript[_with_checkpoint]`。
- 产物：`output/video.srt`（UTF-8 BOM）+ `output/transcript.json`，另存 `output/transcript.<mode>.json`。

## 边界

- sherpa-onnx 库 (CPU only)。**运行时的模型选择是写死的两条链**：single = SenseVoice；
  hybrid = sensevoice 打轴 + qwen3-asr（0.6B int8）填词 —— `[asr] model` **不参与运行时选择**（见上）。
  **firered-zh 已下线**（CTC 时间戳在 BGM 段造幽灵时间戳，见 ROADMAP）：注册表、下载清单、界面都不再有它
- **断句 = 「SenseVoice 出时间、文本出切点」**（`asr/align.rs`）：
  - **分工**：**标点（来自最终文本，hybrid 下是 qwen3 的）+ 强停顿（`segment_gap`）决定切成几条**，
    **气口（`breath_gap`）决定切在哪一刻**，**字数（`max_segment_chars`）只做硬上限**。
    ⚠️ 气口**只用来定位切点，不是独立切分信号** —— 实测词中间也有 0.30s 停顿，单靠它会切碎。
  - `CharTimeline`（逐字时间戳，`from_tokens` 把 token 摊成单字共享时间戳）是**唯一时间权威**；
    qwen3 没有时间戳，`align_to_timeline` 用贪心+小范围重新同步把它的文本对到时间轴上
    （吃错字/漏字/多字）。首尾用**对齐后的字时间**定，不能照抄窗口起止（qwen3 常漏掉窗口开头几个字，
    照抄会让字幕提前好几秒）。
  - ⚠️ **别把"短段并回邻居"加回 `cleanup_segments`**：那正是把切好的短问句重新粘成一条的元凶。
  - ⚠️ **别再 `trim_end_matches` 删成片字幕的尾标点**（那是断句证据）。
  - **四条踩过的坑（都在这条链路上，改之前先看）**：
    ① **切点不做任何"最小时长/字数份额"的抬升**：窗口的 `(start, end)` 会不再覆盖它自己那些字，
       hybrid 按 `slice_by_time` 取字取到空集（实测 1186 个窗口里 589 个退回"一条一窗"）。短条改用 `merge_too_short` **合并**解决（只动文本归属，不动时间与分区），
       且**并完超过 `max_chars` 就不并**（否则把 `怎么？你们都玩过吗？` 又粘回一条）。
    ② **`f64::clamp` 会 panic**（`min > max`），而这里的边界全是算出来的 ——
       实测在 3 小时素材上崩掉整个转写进程。一律用 `clamp_ordered`（`hi < lo` 时以 `lo` 为准）。
    ③ **切点去重分两类**：标点切点是**文本层的事实**，时间算得再近也要留
       （一律按时间判会把对齐差窗口里的三条问句并成两条）；强停顿/字数上限按**时间距**判
       （按字距判会误并「≥1s 停顿里夹着一个孤字」的两处边界）。
    ④ **块间重叠必须按"下一块起点"分区**（`token_belongs_to_chunk`，半开区间 `[offset, offset+step)`）：
       按块中点会让重叠区的字写两遍、切出两个覆盖同一段音频的窗口（qwen3 白跑、字幕重复）。
  - 中间产物落 `output/asr/`（`raw/chunk-NNN.json` / `timeline.json` / `align.json` /
    `segments.raw.json`），记的是 `normalize` **之前**的原文（与落盘字幕的细微差别是有意的）。
    `artifacts::reset` 必须先清目录：`raw/` 按块编号命名，换过 `chunk_duration` 后残留会冒充本次结果。
  - **字幕文本后处理**（`asr/text.rs::normalize`，断句之后、写盘之前，两种模式共用一处；用户定的排版规则）：
    ① **行尾只留带情绪的符号** —— `？！!?` 保留，`。.` 与 `，、；：,;:` 从行尾去掉
    （`…` `～` 保留：是语气/拖长音，不是中性标点）；
    ② **转述句式去引号** —— `xx说：“xxxx”` → `xx说，xxxx`（引号前的冒号变逗号，
    删引号后露出来的 `。”，` 叠写收敛成 `。`）。
    ⚠️ 只动**行尾与引号**，正文里的标点一个不许碰（切分是按原文标点做的）；
    空/纯符号输入原样返回，**绝不产出空字幕**；ASCII 单引号不删（`don't`）。
    它改的是 `transcript.json` 正文（检测/订正/切片都吃它）。
    诊断：`VT_TRANSCRIPT=<transcript.json> cargo test -p yekitrim-core --lib diagnose_text_normalize -- --ignored --nocapture`
  - 回归钉子：`diagnose_hybrid_three_questions`（三个问句必须切成三条、**首条起点 = 窗口起点**）。
- 可配置定长分块 (默认 60s + 10s 重叠)
- **single 模式先用 VAD 只解码语音段**（`asr/vad.rs` + `local.rs::plan_vad_windows`；
  完整分析与实测见 `docs/plan/vad-single-mode.md`）。**184 分钟真实录播 195.6s → 83.7s（2.34×）**。
  - **只对 single 生效**（`use_vad`）：hybrid 的时间 8 成在 qwen3 上，VAD 只省打轴那一小段，
    而且语音切得更碎会让 qwen3 窗口变多 —— 所以 hybrid 仍旧固定分块，**它的输出一个字都不变**。
  - 流程：silero → 合并 ≤0.8s 近邻 → **两侧各 0.2s 护住句尾** → **动态电平兜底** → 区间转解码窗口（`decode` 含
    余量、`claim` 半开不重叠；超 `chunk_duration` 的连续语音按**与固定分块同一套步进**切开）。
    断句/时间轴/`RawChunk` 全没动。
  - **动态电平兜底**（按这一场**自己的**电平，不用固定 dB 门限）：逐帧 RMS（20ms）→ 语音电平取 VAD
    区间内帧的 75 分位 → `floor = 语音电平 − vad_rescue_db`（默认 **16dB**）→ 连响 ≥0.5s 且高于 floor
    的空档**捞回来解码**；**只做加法**（失效方向是"退化成固定分块"，不是"字幕少一条"）。
  - ⚠️ **喂 VAD 的块必须是一次窗口（512 采样）**，别图省事喂一大块：上游
    `voice-activity-detector.cc` 先 push 完整批窗口、再按**这批结束时**的 `buffer_.Tail()` 反推段边界，
    喂 4 秒一块会把段边界量化到 4 秒（实测合成音频真语音 20.75~25.59s 被报成 23.69~31.50s，
    **换模型换阈值都不变**）。上游那句 `NOTE(fangjun): Please don't use a very large n.` 就是这个。
  - ⚠️ **VAD 按块并行（块数 = worker 数）时时间戳必须加回块偏移**（`detect_speech` 报的是相对
    喂入起点的时间）；忘了加，第 2 块之后的段全落在 `[0, feed_len]` 里与第 1 块重叠，一合并整片消失
    （实测 1410 段塌成 79 段）。块首要留 1s 余量把 LSTM 状态喂热，接缝重叠交给 `merge_regions` 收口
    —— **宁可重叠也不能留缝**（留缝就是丢字幕）。
  - VAD 失败一律**回退固定分块**（模型没下、判定异常），绝不把转写打死；模型随 `yekitrim download`
    下到 `<模型根>/silero-vad/`，转写前 `ensure_models` 也会确保它在位（失败只告警）。
  - 开关：`[asr] vad`（`Option<bool>`，**显式写了才覆盖** —— bool 会掉进"≠默认才覆盖"的坑，
    "全局关、单工程开"就表达不出来）、`vad_threshold`（silero 概率阈值）、`vad_rescue_db`（兜底 dB，0=关）。
    设置页「程序配置」的 ASR 块有这三项（复选框 + 两个滑块）。
  - 诊断：`VT_ASR_WAV=<wav> cargo test -p yekitrim-core --lib diagnose_vad_regions -- --ignored --nocapture`
- 并行转写: 默认 CPU/4 路并行 (上限 4)，每 chunk intra-op threads=2
- **qwen3 单流解码的线程数 = `min(4, 核数)`**（`local::decode_threads`）：这个 session 是**串行**用的
  （全局 `QWEN3_RECOGNIZER` Mutex）——**性能只动这里**。
  ⚠️ 若将来改多实例并行，这里要回到 1（否则 N 实例 × 4 线程互相抢核）。
- **首条字幕的起点 = 窗口起点**（`align.rs` 的 `next_start = span.0`）。
  ⚠️ **别改成 `time_of(0)`**：它对没对齐上的字返回的是**后一个锚点**的时间，
  于是"qwen3 句首与 SenseVoice 字轴对不上"时起点会往后跳 —— 那正是"句子开头几个字
  看起来没识别到"的一种成因（字幕出现时那几个字已经说过去了）。
- **生成标记必须截掉**（`text::strip_generation_markers`，在**切分之前**）：qwen3 喂到音乐/噪声段会吐
  `<|endoftext|>` 然后接着胡说。不截会自成一条字幕，还会漏进 `clip.srt`。
- **`segment_padding`（切给识别器的音频余量）默认 0 = 关**。机制在 `mod.rs::decode_span`：
  前后各留一点，**前置余量只吃停顿、不碰上一个字的语音**（下限取上一个字起始 + 0.35s）。
  ⚠️ 实测 0.3s 的 A/B 不划算（`diagnose_head_padding`，3 分钟真实音频、17 个窗口）：
  11 个窗口文本变了、平均对齐命中率 **0.473 → 0.430**（更差），只换来 2 个"整窗从空到有内容"，
  所以**默认关**。想试就设 0.1~0.3 并在**自己素材**上复测。同一旋钮也控制 chunk 解码的左余量。
- **`max_new_tokens` 默认 256**：固定全局值，不做 per-stream 动态覆盖。
  上游 sherpa-onnx 对 qwen3-asr 的默认是 **128**，本项目取 256 作为稳定折中。
  如果以后调大 `max_segment_chars`，需要同步调大 `max_new_tokens`。
  ⚠️ 截断时 C++ 会打 `Result is truncated. max_new_tokens ... too small` 到 stderr。
  `max_total_len`（**输入**上限，写 2048）**不要动**：60 秒窗口确实需要它，历史上"踩过的坑"多半是它。
- ⚠️ **`[asr]` 的 `align_model` / `max_segment_duration` / `segment_merge_gap` 已删除**
  （全仓从来没有读取方）—— 别再往界面或结构体加回来。审计口径与兼容性钉子见
  `crates/yekitrim-core/src/config/AGENTS.md` 的「死字段」一节。
- SRT 使用 UTF-8 BOM 写入 (Windows 字幕兼容)
- 输出: `video.srt` + `transcript.json`，并把生效值写进 `transcript_file`（yekitrim.toml）
- ⚠️ **固定名不改、按模式另存一份**（2026-09）：`output/video.srt` 与 `output/transcript.json` 这两个
  名字被 `status.rs` / `ProjectConfig::current_stage()` / 老 CLI 认死，**别改成按模式命名**；
  另存的那份是 `output/transcript.<mode>.json`（`asr/mod.rs::mode_slug`：小写、≤32、只允许 ASCII
  字母数字与 `-`/`_`）。`AsrMetadata.mode` 记是哪条链路出的（`#[serde(default)]`，老 json 不会炸）。
- ⚠️ **产物落盘 = 一次绑定**：登记清单与生效值这件事**只有一条路** ——
  `yekitrim_service` 侧的 `assets::bind_asr_artifact`（管线经 `core_pipeline::save_transcript_after_asr`
  调它，LLM 订正那份走 `bind_derived_material`：订正稿是**普通条目**，不占 ASR 槽）。
  绑不上（产物没落盘等）才退回裸写 `transcript_file`。别再在别处手写 `transcript_file` ——
  漏登记 = 那个模式的槽明明占了却看不见，用户就少一次转写机会。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-core --lib asr
```

覆盖切分/去重/`clamp_ordered`、`normalize`、VAD。
