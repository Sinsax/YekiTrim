# ASR 方案探索：单模型 / 双模型的分类评估

> **状态：纯调研，未改任何代码。**
> 上游：`asr-sensevoice-replacement-survey.md`（能否替代 SenseVoice）、
> `asr-moss-transcribe-diarize-survey.md`（MOSS 评估）。日期：2026-07

## 0. 两个模式的成本结构**根本不同**（这是全部结论的基础）

项目实测（`local.rs:214`、`:458`、`:575`，184 分钟 / 3 小时素材）：

| | **single**（快、相对准） | **hybrid**（高质量） |
|---|---|---|
| 组成 | SenseVoice 一遍 | SenseVoice 打轴 **+** qwen3 填词 |
| 实测墙钟 | **83.7 s**（含 VAD） | **≈ 15 min**（12.4 min qwen3 + 2.5 min 打轴） |
| 瓶颈 | **解码音频量**（VAD 已砍掉一半） | **qwen3 自回归串行解码**（占 8 成） |
| 并行度 | 4 路并行（`num_workers` 上限 4） | qwen3 **全局 Mutex 串行**（`QWEN3_RECOGNIZER`） |
| 优化杠杆 | 少喂音频（VAD） | **提高单流解码速度**（`decode_threads` = min(4,核数)） |

**关键推论**：两个模式**不能套用同一套优化**。single 已经吃满"少解码"这条路（2.34×）；
hybrid 的 8 成时间在 qwen3 的**串行自回归解码**上，那是一个**单流、内存带宽受限**的瓶颈 ——
**加并行度没用，只能换更快的解码器**。

## 1. 结论速览：有，但**分别在不同的模式里**

| 维度 | 谁显著优越 | 幅度 | 方向 |
|---|---|---|---|
| **速度（single）** | **SenseVoice 保持**（已吃满 VAD） | — | 无更优者 |
| **速度（hybrid）** | ⚠️ **qwen3 → 更小的自回归解码器** | 潜在 **数倍** | **唯一有显著空间的维度** |
| **质量** | **双模型（hybrid）** 本身即答案 | CER 7.81% vs 10.18% | 打轴层已是最优，**别动** |
| **占用（磁盘/内存）** | **single**（SenseVoice 165 MB vs qwen3 879 MB） | **~5×** | 想省就**别装 qwen3** |

**一句话**：**没有一个模型在三个维度全面碾压**；
唯一"显著优越"的机会在 **hybrid 的 qwen3 那一层（速度）**，
和 **single 的"不装 qwen3"（占用）**——而质量维度**已经到顶，没有替代品**。

## 2. 分维度展开

### 2.1 速度 —— 唯一有显著空间的地方是 qwen3

**single 侧已经到顶。** VAD 把解码量砍到 6515s（2.03× 更少）、墙钟 195.6→83.7 s。
剩下的 83.7 s 里 VAD 自身就占 22.0 s（`vad-single-mode.md:13`）。
**SenseVoice 本身就是 CPU 上最快的非自回归选择**（见姊妹篇），换模型只会更慢。

**hybrid 侧有数倍空间**，因为瓶颈是 qwen3 的串行自回归解码：

- qwen3 是 **0.6B decoder-only 自回归**，逐 token 生成；**串行**（全局 Mutex）。
- 现有优化只有 `decode_threads() = min(4, 核数)` —— 把单流解码喂满 4 核。
- ⚠️ **12.4 min / 1182 个窗口 ≈ 0.63 s/窗口**，而每个窗口只有 30 字左右。
  **每个窗口的解码代价与"要生成多少 token"成正比**。

**可行的加速方向（按性价比排序）：**

1. ⭐ **降 `max_new_tokens`（256 → 128）**：**零成本、立刻见效**，但要实测截断率。
   上游 sherpa-onnx 对 qwen3-asr 的默认**就是 128**，本项目取 256 是"稳定折中"
   （`config/types.rs:517-525`）。30 字窗口实测正常输出仅 20~40 token，
   **256 有一多半余量是白留的**。
   ⚠️ 代价：音乐/噪声段的失控输出会更早截断（这其实**是好事**），
   但长窗口可能被截断（C++ 会打 `Result is truncated` 到 stderr）。
2. **换更小的填词模型**：qwen3-asr 0.6B 是"最准最慢"档。
   注意 `Fun-ASR-Nano` 也是 Qwen3-0.6B 解码器，**换汤不换药**；
   真要小得多的自回归 ASR，生态里目前**没有** CPU-可用的成熟替代。
3. **多实例并行**：⚠️ **当前架构明确禁止**。`local.rs:212-215` 写着
   "若改成多实例并行，这里要回到 1（否则 N 实例 × 4 线程会互相抢核）"。
   要做需要把全局 Mutex 换成实例池，属**结构性改造**，风险高。

> **单条最划算的动作：把 `max_new_tokens` 从 256 降到 128 试一遍。** 这是唯一"改一个配置值、可能省两成时间"的旋钮。

### 2.2 质量 —— 已经是当前最优，没有替代品

- **hybrid 的文本质量 = qwen3 的文本质量**（打轴只提供时间，不提供字）。
- 姊妹篇已证明：**打轴层没有比 SenseVoice 更好的**（Paraformer CER 10.18% 更差；
  Dolphin 粒度不达标；Fun-ASR-Nano 官方承认无可靠时间戳）。
- **要提质量只能动 qwen3 那一层**，而 qwen3 已经是 sherpa-onnx 里"最准"的档。
- ⭐**不要动打轴模型** —— 这是 hybrid 架构的核心价值（"SenseVoice 出时间、文本出切点"）。

### 2.3 占用 —— single 显著优越（~5×）

| | SenseVoice | qwen3-asr |
|---|---|---|
| 磁盘 | **165 MB** | **879 MB** |
| 常驻 | 小（非自回归单遍） | 大（0.6B 权重，自回归 KV cache） |

- **只要不跑 hybrid，就完全不需要 qwen3 的 879 MB。**
  对"部署到边缘/小内存机器"的场景，**single 在占用上碾压 hybrid**（用户口径的"部署面"）。
- ⚠️ 但当前 `download::ensure_models` 会**按 mode 自动拉 qwen3**
  （`download.rs:339-344` 的 `required_models`）—— 装了 hybrid 就一定吃这 879 MB。

## 3. 三个维度能否同时改善？

**不能。** 这是**同一个三角**：

```
        质量（qweb3 填词）
           /\
          /  \
   速度 —— —— 占用
```

- **要质量 → 必须 hybrid → 必然付 qwen3 的 879 MB + 12.4 min。**
- **要速度/占用 → 只能 single → 接受 SenseVoice 的文本质量（CER 7.81%，其实不差）。**
- 唯一"帕累托改进"的机会是 **§2.1 第 1 条**：让 hybrid 的 qwen3 解码**更快**，
  **不牺牲质量、不增加占用**。

## 4. 建议

1. ⭐ **先试 `max_new_tokens = 128`**（`[asr]`，hybrid 生效）。
   零风险、可回退、可能省 qwen3 时间的可观比例。**这是唯一"一个旋钮三面都不吃亏"的改动。**
   建议先跑一场真实素材，看 stderr 有没有 `Result is truncated` 与字幕是否变短。
2. **打轴层一个字都别动**（SenseVoice 是最优；结论见姊妹篇）。
3. **让 single 真正"轻部署"**：如果目标是边缘/小内存，确保**不装 qwen3 那 879 MB**
   （当前 `ensure_models` 按 mode 拉，single 模式本来就不拉 —— 这条已经是好的，别改坏）。
4. **hybrid 多实例并行 = 一次性大改造**，不建议在没有明确收益证据前做
   （`local.rs:212-215` 已明确警告）。

## 5. 附：引用

- qwen3 占 hybrid 八成时间（12.4 min vs 打轴 2.5 min）：`crates/vtrim-core/src/asr/local.rs:458`、`:214`
- VAD 实测（184 分钟 195.6→83.7 s；VAD 自身 22.0 s）：`docs/plan/vad-single-mode.md:11-13`
- `max_new_tokens` 默认 256 及"上游默认 128"的说明：`crates/vtrim-core/src/config/types.rs:517-525`
- qwen3 串行 Mutex 与 `decode_threads` 的耦合警告：`crates/vtrim-core/src/asr/local.rs:196-216`
- 模型体积 165 MB / 879 MB：`crates/vtrim-server/src/api/config.rs:37-38`
- 按 mode 决定要下哪些模型：`crates/vtrim-core/src/download.rs:339-344`
- 打轴层无可替代（CER/Paraformer/Whisper/Dolphin 对照）：`asr-sensevoice-replacement-survey.md`
- MOSS 评估：`asr-moss-transcribe-diarize-survey.md`
