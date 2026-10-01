# ASR 方案探索：模型选择（不受运行时约束）

> **状态：纯调研，未改任何代码。**
> 上游：`asr-sensevoice-replacement-survey.md`（运行时内视角）、`asr-per-mode-tradeoff-survey.md`（单/双模型取舍）。
> 本篇**不受 sherpa-onnx 约束**，纯看模型本身。日期：2026-07

## 0. 判据（沿用并收紧）

| # | 硬约束 | 说明 |
|---|---|---|
| **C1** | **逐字时间戳** | `align.rs` 的 `gap_at`/`breath_points` 靠**相邻字**时间差找气口 |
| **C2** | **中文单字粒度** | 多字 token 共用一个时间戳 → 气口在该 token 内失效（Dolphin 死因） |
| **C3** | **BGM/音乐段不造幽灵戳** | `firered-zh`(v1) 的死因 |
| **C4** | **CPU 可用、代价与音频近似线性** | 3 小时录播要能在可接受时间内跑完 |
| **C5** | **许可宽松** | 项目硬规矩：二进制里不许进 GPL |

## 1. 结论：**有一个真正的挑战者 —— FireRedASR2-CTC**（不是 AED）

**在 sherpa-onnx 里带逐字时间戳的是 CTC 变体，不是 AED 变体**（§4 有源码依据）。
FireRedASR2-CTC 是**唯一**同时满足 C1~C5 五条硬约束的候选：

| 约束 | FireRedASR2-CTC | 依据 |
|---|---|---|
| C1 逐字时间戳 | ✅ | 官方 fixture：**19 token / 19 时间戳**，中文 token 是单字 |
| C2 单字粒度 | ✅ | `from_tokens` 已能处理多字符英文子词（`align.rs:77-91`） |
| C3 抗幽灵戳 | ⚠️ **未验证** | 纯 CTC，与 SenseVoice 同类机制 —— **正是 v1 翻车的那一类** |
| C4 CPU 线性代价 | ✅ | 272s 长音频 **RTF 0.058**（3 线程 + VAD，与 V-Trim 同架构） |
| C5 许可 | ✅ | **Apache-2.0** |
| 额外 | ✅ | **非自回归、代价线性**；20+ 中文方言；唱歌 CER 1.17% |

⚠️ **但它不是"已被证明的替代品"**：决定性未知量（C3）**无法从公开资料判定**，
只能在 V-Trim 自己的素材上实测。且 740 MB vs SenseVoice 的 165 MB ≈ **4.5×**。

## 2. 中文精度对照（各来源自报 CER，越低越好）

| 模型 | 参数量 | aishell1 | aishell2 | ws_net | ws_meeting | 平均 | 来源 |
|---|---|---|---|---|---|---|---|
| **FireRedASR2-LLM** | 8B+ | 0.64 | 2.15 | 4.44 | 4.32 | **2.89** | FireRedASR2S 论文 |
| **FireRedASR2-AED** | **1B+** | **0.57** | 2.51 | 4.57 | 4.53 | **3.05** | FireRedASR2S 论文 |
| Doubao-ASR（商业 API） | — | 1.52 | 2.77 | 5.73 | 4.74 | 3.69 | 同上 |
| Qwen3-ASR | — | 1.48 | 2.71 | 4.97 | 5.88 | 3.76 | 同上 |
| Fun-ASR（商业 API） | — | 1.64 | 2.38 | 6.85 | 5.78 | 4.16 | 同上 |
| Whisper-Large-v3 | 1.6B | 5.14 | 4.96 | 10.48 | 18.87 | 9.86 | FireRedASR 论文 |
| **SenseVoice-L** | 1.6B | 2.09 | 3.04 | 6.01 | 6.73 | **4.47** | FireRedASR 论文 |
| Paraformer-Large | 0.2B | 1.68 | 2.85 | 6.74 | 6.97 | 4.56 | 同上 |
| FireRedASR-AED（v1） | 1.1B | 0.55 | 2.52 | 4.88 | 4.76 | 3.18 | 同上 |

**要点**：FireRedASR2-AED（1B+）平均 CER **3.05%**，**显著优于 SenseVoice-L 的 4.47%**（相对降 32%），
而且**同时优于 Doubao-ASR 与 Qwen3-ASR 这些商业/大模型基线**。

> ⚠️ **可比性声明（重要）**：上表**所有行都来自 FireRedASR 系列论文的同一套评测**，所以行间可比。
> 但网络上常被引用的 **SenseVoice 7.81%**（FunASR 选型指南）与 **Paraformer 10.18%**
> **来自另一个来源、另一套评测集**，**绝不能与本表的 2.89/3.05 放在同一列比较**。
> 任何"精度提升 2.5 倍"的说法都是**不成立的**。
> 诚实的表述是：*"FireRedASR2 在 4 个公开普通话基准上报告平均 CER 2.89~3.05%；
> SenseVoice 常被引用的 7.81% 来自不同的评测集，两者不可直接比较。"*
> **在同口径（FireRedASR 论文）下，SenseVoice-L 是 4.47% vs FireRedASR2-AED 的 3.05%。**

**方言/口音**（19 个测试集）：FireRedASR2-AED **11.67%**，SenseVoice 系无对应公开数据；
对比 Doubao-ASR 15.39% / Qwen3-ASR 11.85% / Fun-ASR 12.76% —— **领先**。覆盖 20+ 种中文方言。

**唱歌**（opencpop）：FireRedASR2-AED **1.17%**。
⭐ 这一条对 V-Trim **特别有价值**：本项目实测 VAD 版本"多出来的内容是唱段/歌词"
（`vad-single-mode.md:22-24`），而 SenseVoice 在唱段上的表现没有公开数据。

## 3. ⭐ 关键：时间戳机制变了（这是它和"已下线的 firered-zh"的根本区别）

`asr/AGENTS.md:25` 记录 firered-zh 下线原因是 **"CTC 时间戳在 BGM 段造幽灵时间戳"**。
FireRedASR2-AED **不是**那个机制。论文 §3.1.2 原文（[arXiv:2603.10420](https://arxiv.org/html/2603.10420v1)）：

> "A key update in FireRedASR2-AED is the support of timestamps via an additional **CTC branch**
> attached to the encoder. After the base AED model ... is fully trained, we add a lightweight
> CTC projection head ... and train it **post-hoc by freezing the encoder and decoder**...
>
> During inference, we first decode the token sequence using the **AED decoder (beam search)**.
> We **then** compute frame-level CTC logits from encoder states and perform
> **CTC forced alignment between the CTC logits and the AED-decoded token sequence**."

**机制差异（为什么幽灵戳风险大幅降低）：**

| | firered-zh（v1，已下线） | **FireRedASR2-AED** |
|---|---|---|
| 时间戳来源 | CTC **自由解码**输出（CTC 自己决定说什么、何时说） | **CTC 强制对齐**去贴**已经由 AED 解码好的** token 序列 |
| 文本权威 | CTC | **AED**（beam search，精度高） |
| BGM 段行为 | CTC 自由解码 → 在无语音处也能吐密集伪 token | 对齐目标已固定；无对应 token 就没有可对齐的槽位 |

**这篇论文的表 1 把它列为 v1→v2 的三条关键更新之一**：
`Timestamps (AED): Not Supported → Supported`。
论文明确"对中文，每个字符 token 即一个 word 单元"（*"For Chinese, we treat each character token as a word unit"*），
所以**中文是逐字（character-level）**。

⚠️ ⚠️ **但这里有一个决定性但书，必须在决策前说清楚**：

**这套"AED + 强制对齐"时间戳只存在于 FireRedASR2S 的上游 Python 实现里。**
sherpa-onnx 里收录的 **AED 转换版不产出时间戳**（`Convert()` 没填 `r.timestamps`，
实测 `"timestamps": []`）；**sherpa-onnx 里带逐字时间戳的是 CTC 变体**，
而 CTC 变体的时间戳来自 **CTC 自由解码**，**不是**论文那套"对齐到 AED 序列"的机制。

sherpa-onnx 官方文档也把这件事写明了 —— 那段 CTC 转换的原文是：

> "This model is converted from FireRedASR2-AED.
> Note that **only the CTC branch is converted. The attention decoder branch is excluded.**"

⚠️ **换言之：sherpa-onnx 里的 v2-CTC 是一个"只有 CTC 分支的独立模型"，
它既不是 AED、也吃不到 AED 的高精度文本** —— 它的文本来自 CTC 自己解码。
这与"用 AED 的文本 + CTC 对齐的时间戳"是**两件不同的事**。

因此：

| 想要的 | 走哪条路 | 代价 |
|---|---|---|
| **AED 的对齐时间戳**（更安全，论文机制） | FireRedASR2S **上游 Python**（`return_timestamp=True`） | 引入 **Python/PyTorch**，与部署面冲突 |
| **逐字时间戳**（在 sherpa-onnx 里） | **v2 CTC 变体** | 时间戳机制**与当年翻车的 firered-zh 同类**（CTC 自由解码） |

⚠️ **另外必须诚实标注**：论文**没有**提供"BGM/音乐段时间戳鲁棒性"的定量评测。
**"强制对齐比自由解码更稳"是机制推理，不是实测证据。**
这两条都**必须在 V-Trim 自己的素材上复测**（项目已有现成手段：
`VT_ASR_WAV=... diagnose_vad_regions` 与 `vtrim.toml` 的 A/B 流程）。

## 4. 部署现实（不受运行时约束，但仍值得看）

sherpa-onnx **已经收录 v2**（[官方文档](https://k2-fsa.github.io/sherpa/onnx/FireRedAsr/pretrained.html)），
提供两个转换版本 —— 这大幅降低了验证成本：

| 版本 | 文件 | 体积 | CPU RTF（官方实测） | **sherpa-onnx 里实际能拿到的时间戳** |
|---|---|---|---|---|
| **v2 CTC** | `model.int8.onnx` | **740 MB** | **0.173**（10s 单片）/ **0.058**（272s + VAD，3 线程） | ✅ **逐字**（实测输出逐 token 时间戳） |
| v2 AED | encoder+decoder int8 | 398+779 = **1.18 GB** | **0.182**（272s + VAD，3 线程） | ⛔ **实测 `"timestamps": []`（空）** |
| v1 AED（旧） | encoder+decoder int8 | 1.7 GB | **1.945**（10s 单片） | ❌ 无 |

⚠️ **三个重要观察（第 2 条是本次调研的关键更正）**：

1. **v2 比 v1 快得多**（文档原话：*"The AED model in v2 is also much faster than the v1 AED model"*），
   v1 的 RTF 1.945 是**慢于实时**的 —— 这就是当年它在本项目"最准最慢"定位的来源。
2. ⛔ **在 sherpa-onnx 里，"AED 的强制对齐时间戳"目前拿不到。**
   我把这一点查到了源码层：`offline-recognizer-fire-red-asr-impl.h` 的 `Convert()` 只填
   `r.tokens` / `r.text`，**从头到尾没有给 `r.timestamps` 赋过值**；
   而 AED 官方输出的实测也正是 `"timestamps": []`。
   → **sherpa-onnx 里唯一真正带逐字时间戳的是 v2 CTC 变体**（`fire_red_asr_ctc` 那条路径）。
   ⚠️ 这意味着一件重要的事：**要走这条路，就得用 CTC 分支 —— 而 CTC 分支正是与
   "当年 firered-zh 幽灵时间戳"同类的机制**（§3 里我们担心的那一类）。
   AED 的"强制对齐"更安全，但**它在 sherpa-onnx 中目前不产出时间戳**；
   想用 AED 的对齐时间戳，只能走 **FireRedASR2S 上游的 Python 实现**（`return_timestamp=True`），
   代价是引入 Python/PyTorch —— 与本项目的部署面冲突。
3. **"CTC 强制对齐"与"CTC 自由解码"在 sherpa-onnx 里其实都通过 CTC 头产出时间戳**：
   论文的强制对齐发生在**上游 Python 实现**里（AED 解码 + CTC 对齐）。
   sherpa-onnx 的 v2-CTC 是**独立 CTC 分支模型的自由解码**，不是论文那套对齐。
   ⚠️ **这两者被我此前的文档混为一谈了，此处更正。**

**许可**：FireRedASR2-AED 是 **Apache-2.0**（[HF 模型卡](https://huggingface.co/FireRedTeam/FireRedASR2-AED)可查），
不触发 GPL 三防线。**这不是 GPL 问题**。

## 5. 与 SenseVoice 的诚实取舍

| 维度 | SenseVoice | **FireRedASR2-AED** | 谁赢 |
|---|---|---|---|
| 中文 CER（平均） | 4.47 | **3.05** | **FireRed**（-32%） |
| 逐字时间戳 | ✅ | ✅ | 平 |
| 时间戳机制风险 | ✅ 已验证（项目在用） | ⚠️ **未在本项目验证** | **SenseVoice** |
| 模型体积 | **165 MB** | **1.18 GB**（AED）/ 740MB（CTC） | **SenseVoice（~7×）** |
| CPU 速度 | 更快 | 更慢（RTF 0.058~0.18，官方 3 线程） | **SenseVoice** |
| 方言覆盖 | 5 种语言 | **20+ 中文方言** | **FireRed** |
| 唱歌 | 无公开数据 | **1.17% CER** | **FireRed** |
| 成熟度 | 项目已深度集成、踩坑已知 | 全新（2026-02 发布） | **SenseVoice** |

**结论：如果"质量"是首要目标且能接受 7× 体积与更慢的速度，FireRedASR2-AED 是当前唯一值得认真评估的替代品。
但它不是"白捡的升级"——换来的精度要先跨过"时间戳在 BGM 段是否可靠"这一关。**

## 6. 建议（已按 §4 的源码级发现修正）

1. ⭐ **先做一次"零风险验证"**：用 sherpa-onnx 的 **v2 CTC** 转换版，
   在**本项目自己的素材**（尤其是含 BGM/唱歌的录播）上跑一遍，
   **只对比逐字时间戳**（`VT_ASR_WAV` 诊断 + `timeline.json`），
   重点看 **BGM/音乐段有没有幽灵时间戳**（C3）。
   这一步**不需要改任何生产代码**。
2. **验证的判据**：
   - 若 **BGM 段时间戳干净** → CTC 路可行，且它同时出文本+时间戳，
     **可能连 qwen3 那层都省掉**（架构级简化），值得继续。
   - 若 **BGM 段出现幽灵戳** → 说明 CTC 自由解码的老问题在 v2 上依旧，
     **维持 SenseVoice，本案结束**（因为 sherpa-onnx 的 AED 路拿不到时间戳）。
3. ⛔ **不要按我上一版的说法去"验证 AED 的强制对齐"** ——
   **sherpa-onnx 的 AED 转换版不产出时间戳**（§4 已给出源码依据）。
   想用 AED 的对齐时间戳必须上 **上游 Python 实现**，那要引入 Python/PyTorch，
   与"CPU、零外部运行时"的部署面冲突，**不建议**。
4. **CTC 版 740 MB vs SenseVoice 165 MB（4.5×）**，且 RTF 更慢 ——
   即使时间戳过关，也要把它当成"用体积和速度换精度与方言覆盖"的**权衡**，不是白捡。

## 7. 已排除的候选（附理由）

| 模型 | 排除理由 |
|---|---|
| **Dolphin** | 多字 token + 每 token 单时间戳（`"说的是"` 共享一个）→ **C2 不满足** |
| **Fun-ASR-Nano** | 官方承认 checkpoint **无可靠时间戳** → C1 不满足 |
| **Paraformer** | ⛔ **在 sherpa-onnx 里 `"timestamps": []`（空）**，官方文档明写"does not support timestamps" → **C1 在运行时内不满足**（它的逐字时间戳只存在于 FunASR/PyTorch 实现里）。且 CER 也更差 |
| **Whisper 系** | BPE 非单字；自回归慢；中文 CER ~9.86%；音乐段幻觉 → C1/C2/C3/C4 全不满足 |
| **MOSS-Transcribe-Diarize** | 段级时间戳；CPU 长音频掉出实时 → C1/C4 不满足 |
| **FireRedASR v1** | CPU RTF 1.945（慢于实时）、时间戳是 CTC 自由解码 → C3/C4 |

## 8. 附：引用

- FireRedASR2S 论文（时间戳机制、CER 全表、VAD/LID/Punc）：<https://arxiv.org/html/2603.10420v1>
- FireRedASR2S 仓库：<https://github.com/FireRedTeam/FireRedASR2S>
- FireRedASR2-AED 模型卡（Apache-2.0）：<https://huggingface.co/FireRedTeam/FireRedASR2-AED>
- sherpa-onnx 的 FireRedAsr v2 转换（CTC/AED 体积与 RTF 实测、"only the CTC branch is converted"）：<https://k2-fsa.github.io/sherpa/onnx/FireRedAsr/pretrained.html>
- **sherpa-onnx 源码**（AED 路径不填时间戳的直接依据）：[`offline-recognizer-fire-red-asr-impl.h`](https://github.com/k2-fsa/sherpa-onnx/blob/master/sherpa-onnx/csrc/offline-recognizer-fire-red-asr-impl.h) 的 `Convert()` 只设 `r.tokens`/`r.text`
- Fun-ASR-Nano 无可靠时间戳（官方）：<https://www.funasr.com/en/blog/fun-asr-nano-guide.html>
- FunASR 2026 选型指南（SenseVoice 仍是 CPU 首选）：<https://www.funasr.com/en/blog/which-funasr-model.html>
- Qwen3-ForcedAligner-0.6B（独立的对齐模型，非 ASR 自带）：<https://github.com/QwenLM/Qwen3-ASR>
- FireRedASR v1 论文（v1 与 SenseVoice/Whisper/Paraformer 的 CER 对照）：<https://arxiv.org/pdf/2501.14350>
- 本项目 firered-zh 下线原因（CTC 幽灵时间戳）：`crates/vtrim-core/src/asr/AGENTS.md:25`
- 本项目唱段/歌词现象：`docs/plan/vad-single-mode.md:22-24`
- 打轴链路的硬约束：`crates/vtrim-core/src/asr/align.rs:107-123`
