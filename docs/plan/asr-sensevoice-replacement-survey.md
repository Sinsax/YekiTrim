# ASR 方案探索：SenseVoice 的可替代性评估

> **状态：纯调研，未改任何代码。**
> 上游问题：MOSS-Transcribe-Diarize 调研（见 `asr-moss-transcribe-diarize-survey.md`）引出
> 「有没有能替代 SenseVoice 的打轴模型」。日期：2026-07

## 0. 先确定"替代"的判据

SenseVoice 在 YekiTrim 里**不是"转写器"**，是**打轴器**。它是 `CharTimeline` 的唯一数据源，
而整条断句链路都建在这条**逐字时间轴**上（见 `crates/yekitrim-core/src/asr/AGENTS.md`）。
所以替代品必须同时满足**五条硬约束**，缺一条就不是"替代"：

| # | 硬约束 | 依据 |
|---|---|---|
| **C1** | **逐字时间戳**（不是词级/段级） | `align.rs:107-123` 的 `gap_at`/`breath_points` 靠**相邻字**的时间差找气口；`split_text_on_timeline` 的切点全落在字上 |
| **C2** | **单字 token**（或可摊成单字） | `CharTimeline::from_tokens`（`align.rs:77-91`）把 token 摊成单字、**共享该 token 的时间戳** —— 多字 token 会让整段字共用一个时间，气口粒度直接损失 |
| **C3** | **BGM/音乐段不造幽灵时间戳** | `firered-zh` 就是死在这条（`asr/AGENTS.md:25`），CTC 在非语音段输出密集伪时间戳 |
| **C4** | **CPU、非自回归、代价与音频线性** | 184 分钟素材要能在分钟级跑完；这是 MOSS 被否掉的同一把尺子 |
| **C5** | **sherpa-onnx 已有推理入口** | 换运行时 = 重走 `vendor/sherpa-onnx-sys` 那套编译/许可/打包（GPL 三防线） |

## 1. 结论

**在 C1/C2 这两条上，目前没有比 SenseVoice 更好的替代品；在 CPU 场景下它仍是 FunASR 官方推荐的第一选择。**

FunASR 团队 2026-07 的官方选型指南【[来源](https://www.funasr.com/en/blog/which-funasr-model.html)】原话：

> **"on CPU use SenseVoice, or Paraformer for Chinese timestamps/hotwords."**
>
> | 场景 | 选择 | 理由 |
> |---|---|---|
> | CPU / edge / 要最快 | **SenseVoice** | 非自回归、极快、CPU 实时，多语言+情绪，CER **7.81%** |
> | CPU + 纯中文 + 要时间戳/热词 | Paraformer | 最轻、**字符级时间戳**、热词，CER **10.18%** |

注意 CER：**SenseVoice 7.81% 优于 Paraformer 10.18%**，而 Paraformer 唯一"赢"的地方（字符级时间戳+热词）**YekiTrim 已经用 SenseVoice 拿到了**。

## 2. 逐个候选的评估

### 2.1 Paraformer（sherpa-onnx 已支持）

- ✅ **C1/C2 满足**：FunASR 文档明确 "character-level timestamps"，输出 `[[880,1120],[1120,1360],…]` **每字起止**。
- ❌ **C4 不划算**：CER 10.18% vs SenseVoice 7.81%，**更差**。
- ⛔ **结论**：**唯一在粒度上真正合格的替代品，但精度更差，没有替换动机**。
  只有在"SenseVoice 挂了要降级"时才值得作为**兜底备份**。

### 2.2 Dolphin（DataoceanAI，sherpa-onnx 已支持，40 种东亚语言 + 22 种中文方言）

看 sherpa-onnx 官方示例的真实输出【[来源](https://k2-fsa.github.io/sherpa/onnx/Dolphin/pretrained.html)】：

```json
{"text": " 对我做了介绍哈那么我想说的是呢大家如果对我的研究感兴趣呢。",
 "timestamps": [0.04,0.28,0.60,0.84,1.32,1.76,2.20,2.36,2.72,3.24,3.48,3.72,4.12,4.40,4.76,5.52],
 "tokens":[" ", "对我", "做了", "介绍", "哈", "那么", "我想", "说的是", "呢", "大家", "如果", "对我的", "研究", "感兴趣", "呢", "。"]}
```

- ⛔ **C2 明确不满足**：token 是**多字的**（`"对我"`、`"说的是"`、`"对我的"`、`"感兴趣"`），
  而时间戳**每个 token 只有一个**。16 个 token 覆盖 5.6 秒音频 ——
  **`"说的是"` 三个字共用一个时间戳**。
- 后果：`from_tokens` 摊开后这三个字的 `gap_at` **全是 0**，
  **气口检测在该 token 内部完全失效**（`breath_gap` 找不到切点），
  切点只能落在 token 边界上 —— 中文里就是**每 2~3 个字才有一次切分机会**。
  这正是 `asr/AGENTS.md` 里"气口只用来定位切点"那套机制被削弱的情形。
- ❌ **C4 存疑**：官方示例 RTF **0.094~0.256（num_threads=1，int8）**，
  比 SenseVoice 明显重（`model.int8.onnx` 99MB，但 small 版 int8 就 239MB、fp32 783MB）。
- ⛔ **结论**：**粒度不达标**，不能替代打轴。

### 2.3 Whisper 系（sherpa-onnx 已支持）

- ⛔ **C2 不满足**：BPE 子词 token，**不是单字**；中文一个 token 常覆盖多字。
- ❌ **C4 不满足**：自回归解码，CPU 上比 SenseVoice 慢一个量级。
- ⛔ **C3 存疑**：Whisper 在音乐/静音段有著名的幻觉（hallucination）问题，
  与 firered-zh 死因同类。
- ⛔ **中文精度差**：FunASR 明确写 "all three beat Whisper on Chinese (**~20%**)" vs SenseVoice 7.81%。
- **结论**：三条全不满足。

### 2.4 Fun-ASR-Nano / MLT-Nano（sherpa-onnx 已支持）

- ⭐ 质量最强（中文 CER **8.06%**，SenseVoice encoder + Qwen3-0.6B 解码器）。
- ❌ **C4 不满足**：官方明确 "**with a GPU**, use flagship Fun-ASR-Nano"，是 LLM-ASR。
- ⛔ **C1 关键缺陷**：官方文档直言 ——
  > "The released `model.pt` checkpoint **lacks trained CTC alignment weights**,
  > so it does **not provide reliable** [timestamps]"
  【[来源](https://www.funasr.com/en/blog/fun-asr-nano-guide.html)】
- ⛔ **结论**：**没有可靠时间戳** —— 直接排除，连讨论的必要都没有（与 MOSS 同一类问题）。

### 2.5 MOSS-Transcribe-Diarize

见 `asr-moss-transcribe-diarize-survey.md`：只出**段级** `[起][止]`，无逐字时间戳（C1/C2 双不满足），
且 CPU 长音频掉出实时（C4 不满足）。

## 3. 汇总表

| 候选 | C1 逐字 | C2 单字 token | C3 抗幽灵戳 | C4 CPU 线性 | C5 已有入口 | 判定 |
|---|:---:|:---:|:---:|:---:|:---:|---|
| **SenseVoice（现状）** | ✅ | ✅ | ✅ | ✅ | ✅ | **基线** |
| Paraformer | ✅ | ✅ | ⚠️ CTC 同类风险 | ✅ | ✅ | 精度更差，仅作兜底 |
| Dolphin | ❌ | ❌ 多字 token | ⚠️ CTC | ⚠️ RTF 0.09~0.26 | ✅ | **粒度不达标** |
| Whisper | ❌ | ❌ BPE | ❌ 幻觉 | ❌ 自回归 | ✅ | 全不满足 |
| Fun-ASR-Nano | ❌ 官方承认无可靠戳 | — | — | ❌ 需 GPU | ✅ | 直接排除 |
| MOSS | ❌ 段级 | ❌ | ✅ 自回归生成 | ❌ 掉出实时 | ❌ | 直接排除 |

## 4. 真正的建议

1. **SenseVoice 保持不动。** CPU/中文/逐字时间戳这三个条件同时成立时，它**就是当前最优解**，
   连 FunASR 官方选型指南都这么写。**没有替换动机。**
2. **如果目标是"更准"，不要换打轴模型 —— 换填词模型。**
   YekiTrim 的架构已经把这两件事分开了（hybrid = SenseVoice 打轴 + qwen3 填词）。
   要提升文本质量，动的是 qwen3 那一层（`local.rs::create_qwen3_recognizer`），
   打轴层不该动 —— 这正是现有设计的价值。
3. **Paraformer 可作为可选兜底**（同一套 sherpa-onnx 入口，改 `MODEL_REGISTRY` 一行 + 一个模式分支）。
   但它 CER 更差，**只在"想要热词可调"或"想有个不依赖 SenseVoice 的备胎"时才有意义**。
4. **Dolphin 值得单独记一笔**（40 种东亚语言 + 22 种中文方言，覆盖面远超 SenseVoice 的 5 种语言）。
   如果将来要做**方言/小语种**录播，它是候选 —— 但**要接受打轴粒度降到 2~3 字**，
   意味着 `breath_gap` 那套气口机制基本失效，**断句质量会退化**。这是个 trade-off，不是升级。

## 5. 附：引用

- FunASR 官方选型指南（2026-07，"on CPU use SenseVoice"，CER 对照）：<https://www.funasr.com/en/blog/which-funasr-model.html>
- Fun-ASR-Nano 无可靠时间戳的官方说明：<https://www.funasr.com/en/blog/fun-asr-nano-guide.html>
- Dolphin sherpa-onnx 预训练模型与真实输出示例（多字 token + 逐 token 时间戳）：<https://k2-fsa.github.io/sherpa/onnx/Dolphin/pretrained.html>
- SenseVoice sherpa-onnx Python API（**逐字时间戳**示例：14 字 14 个时间戳）：<https://k2-fsa.github.io/sherpa/onnx/sense-voice/python-api.html>
- sherpa-onnx 全部离线模型清单（含 Dolphin / FunASR Nano / Paraformer / FireRedAsr）：<https://k2-fsa.github.io/sherpa/onnx/pretrained_models/offline-transducer/index.html>
- 本项目打轴链路与踩坑：`crates/yekitrim-core/src/asr/AGENTS.md`、`crates/yekitrim-core/src/asr/align.rs`
- 本项目 CPU 基线（184 分钟 → 83.7 s）：`docs/plan/vad-single-mode.md`
- 同批 MOSS 调研：`docs/plan/asr-moss-transcribe-diarize-survey.md`
