# V-Trim Roadmap

待检验和排期的新功能。

## 高优先级

### A. `vtrim validate` / `vtrim doctor` 增强

验证 vtrim.toml + 运行环境：

- 检查 ffmpeg/ffprobe 在 PATH 中
- 验证模型文件完整性和版本兼容性
- 测试 LLM API 连接（复用现有 `test_llm`）
- 预检视频格式 / 弹幕文件格式 / 音频采样率
- 检查磁盘空间是否足够
- 报告缺失的可选组件（sherpa-onnx 模型等）

### B. ✅ 已解决（ASR 部分）— 断点续传 / 进度持久化

ASR 分块转写时，每完成一批写 `.vtrim/progress.json`（含音频路径/总块数校验，避免脏缓存），重启时跳过已完成块。
LLM 订正流程已重新设计：小批次（25 段）、只返回修正项（JSON 对象）、失败自动重试一次；LLM 订正批次断点续传（`--resume` 或自动检测）尚未实现。

### C. ✅ 已解决 — 静音检测参数暴露到配置

`[asr.vad]`（threshold / min_silence_duration / min_speech_duration / max_speech_duration）已在 `Config` 中定义并写入 `vtrim.toml`。
**注：2026-08 已删除**——VAD 配置无任何模块使用（死代码），连同 `VadConfig` 结构体一并移除（`config/types.rs`、`config/mod.rs`）。

### D. `vtrim process --from-stage`

当前 `process` 内部硬编码阶段路由（检测已完成的跳过）。
提供 `--from-stage 2` 让用户指定从某个阶段开始重跑，灵活调试。

### E. `vtrim detect --preview` / 事件时间线预览

对检测到的事件输出文本/JSON 时间线预览（`start - end | label | popularity`），
让用户确认后再执行剪切。可结合 `less` 或输出到文件。

## 中优先级

### F. 多 LLM Provider 支持

当前 `send_request` 写死 OpenAI 兼容接口：
- `Authorization: Bearer` header
- `/chat/completions` 路径
- OpenAI 风格的 `choices[0].message.content` 响应格式

可抽象 `LlmClient` trait，支持 Anthropic / Google / 本地 ollama 等。

### G. 事件人工标注/编辑

- `vtrim events edit`：读取 `events.json` → 打开 `$EDITOR` → 保存覆盖
- 或支持 `events.edit.json` 作为用户覆盖文件，merge 到最终输出

### H. 多语言 ASR 支持

`AsrConfig.language` 已定义并传给 sherpa-onnx，但：
- `srt.rs` 中 `write_srt` / `parse_srt` 硬编码 `language: "zh"`
- LLM prompt 目前纯中文
- 事件类型 label 和 summary 也是中文

需要抽象语言策略，英文/日文直播场景才能工作。

### I. `--json` 输出模式

`vtrim status --json`、`vtrim detect --json` 等输出 JSON 格式，
便于 CI/脚本集成。

## 素材正确率问题清单（polish LLM 生成链）

> 核心结论：LLM 理解不是瓶颈（local/emoji 贴纸已验证），问题在**素材侧不确定性**——
> 音效 tag 宽泛、irasu 搜索/获取不可靠、特效 duration 与情绪不同步。
> 原则：**素材确定性优先，LLM 只做语义判断**。按 P1→P2→P3→P4 顺序逐一解决。

### P1. ✅ 已解决 — 音效库切换（DOVA → 小森平）+ 两级分类 + 候选注入

- 音效库替换：小森平 2295 个 → 变体归组 1277 组 → 7 大分类 / 83 细分 tag
- 两阶段 LLM：Step1 选大分类+细分 tag 方向 → Step2 按 tag 配额注入候选（≤32）选具体 id
- `resolve_sfx_tag` 变体未使用优先轮换；SFX_RULES 保留作 process 兜底

### P2. 贴纸 irasu：搜索 + 获取不确定

- 根因：Blogger API `q=keyword` 搜日文站博文，中英文 keyword 命中差；
  下载依赖网络，失败写**紫色占位图**；LLM 无法预知哪些图能拿到
- 状态：**已屏蔽（运行时过滤）** — `default_sticker_source` 改为 `["emoji","local"]`；
  `load_config_from` 读取时 `strip_blocked_sticker_sources`（配置写了也不读取）；
  irasu 代码全部保留，修复后可恢复（改默认值 + 删 strip）
- 方向（修复后）：
  1. **本地预下载模式**：`sticker fetch` 命令预先下载一批图（带索引），
     prompt 只注入已下载的 irasu 清单（确定性 = local）
  2. **删除占位图**：下载失败 = 该 keyword 不可用，宁缺毋滥，
     process 自动降级 emoji/local（轮询逻辑已有）
  3. keyword → 日文同义词表（laugh→笑い、shock→ショック）增强 Blogger 搜索命中
- 涉及：`project/validate.rs`、`project/config.rs`、`assets/irasu_search.rs`、`assets/downloader.rs`

### P3. 特效 duration 与情绪释放不匹配

- 根因：LLM 写 0.3~1s duration，渲染侧 `fxDur = max(ev.duration, 0.1)` 直接用；
  一句话没结束特效就消失，突兀、情绪无释放
- 方向：
  1. process 统一归一 duration = 特效时刻到**下一句字幕开始**的时长（clamp 0.8~2.5s，
     强情绪点下限 1.5s）
  2. 渲染侧补齐退场缓动（shake/flash 等 fade-out）
- 涉及：`process/mod.rs`、`templates/index.html`

### P4. LLM 流程辅助项

- 字幕 `take(100)` 截断：长视频后半段 LLM 不可见 → 按 token 预算分批
- 解析失败无反馈：直接 fallback 默认段 → 错误信息回传 LLM 重试 1 次
- 无匹配率报告：sticker/sfx 命中率不可见 → 输出匹配率报告
- 涉及：`llm/segment.rs`、`llm/mod.rs`、`process/mod.rs`

## 低优先级

### J. ASR 并行引擎切换

当前 `std::thread::scope` 每批重新创建 OS 线程。
可切换为 `rayon` 或固定 `threadpool`，减少小块时的创建开销。
对大文件（数小时录像）效果更明显。

### K. 自动 GPU 检测报告

`vtrim doctor` 输出 GPU 编码器可用性：
- NVENC / QSV / AMF / libx264
- 推荐最优编码参数

### L. 弹幕密度热力图输出

`vtrim detect --heatmap` 生成 JSON 格式的弹幕密度热力图数据，
可导入外部可视化工具。

### M. 配置文件 Schema 导出

`vtrim config schema` 输出 JSON Schema，便于编辑器补全。

## ASR 时间戳对齐（qwen3 + sensevoice，进行中）

### 背景

- 5 模型对比测试（2026-08，uv 隔离 Python 原型）结论：qwen3-asr 准确率最优
  （CER 平均 16.2%，稳定性 7/7），推荐为默认模型；funasr-nano 因官方包
  KV=512 硬限制（>~28s 音频截断/退化）被排除；firered-AED 太慢无时间戳
- **sherpa-onnx 的 qwen3-asr 无 token 时间戳**（C++ 源码确认，1.13.x 均无，
  仅输出 text+tokens）
- firered-zh (CTC) 有帧级时间戳，但 BGM 段制造"幽灵时间戳"（把音乐误识别
  字符摊满整段 → 对齐时间全错）→ 方案 A（CTC 对齐）否决
- sensevoice 有逐 token 时间戳（RTF 0.01，几乎免费），BGM 段安静不出 token
  → 选定为时间源

### 方案（Python 原型已验证，时间戳 OK）

qwen3 文本 + sensevoice 时间轴，双模型同音频解码：

1. qwen3（准确文本，主输出）+ sensevoice（tokens + timestamps，仅取时间轴）
2. **LCS 锚点**：两边文本公共子串（≥3 字）直接钉死时间（100% 帧级）
3. **锚点间局部 DP**：±2s 时间窗 + 局部比例先验，就近匹配（不用全局比例）
4. 未锚定字符线性插值；强制单调；句 end = 句尾字时间 + 0.3s
5. 实测（68.8s 直播片段，含 BGM/拖长音段）：时间戳边界误差 <0.5s，
   静音/音乐段正确留空，用户确认时间戳无问题

性能：sensevoice RTF 0.01 → 双模型总耗时 ≈ qwen3 单独 × 1.01~1.05，
内存 +160MB（模型一次加载全程复用；**估算值，release 实测峰值 RSS 后修正**——qwen3-0.6B int8 + sensevoice int8 双 session 驻留，建议按 +300~400MB 预算）。

### 断句规则（v5，待用户验证）

当前原型断句问题：句号处未强制切段（"…是真的假的？我很早之前…"合并一条）。
v5 规则：

- 句末标点（。！？）**强制切段**
- 逗号（，；、）软切：段 >6s 或 >20 字时切
- 单段 ≤8s；超长无标点句按 ≤16 字比例切
- 短段 <1.5s 并入前段

### Rust 实施计划（✅ 主体已实现，待对照 68.8s 验收）

1. ✅ 新建 `crates/vtrim-core/src/asr/align.rs`：
   - `align_text_to_timeline(qtext, schars, stimes) -> AlignedText`（LCS 锚点 +
     锚点间局部 DP + 插值 + 单调强制）
   - `build_srt_segments()`（v5 断句规则）
   - 单测：重复字符 / 文本差异 / 空时间轴 / 句末强制切 / 短段合并 / 无标点长段
2. ✅ `asr/local.rs`：`model = "qwen3-asr"` 时创建双 recognizer
   （qwen3 + sensevoice；sensevoice 模型缺失自动降级现有比例法）
   - 每 chunk 顺序双解码（sensevoice 时间被 qwen3 推理掩盖）
   - CTC 幽灵时间戳教训：**禁用 firered-zh 作时间源**
3. ✅ 配置：`[asr]` 新增 `timestamp_alignment` 开关（默认开，**仅 qwen3-asr 生效**——funasr-nano 已因 KV 硬限制排除，代码 MODEL_REGISTRY 亦无 funasr）
4. ✅ 验证：`cargo test` 通过；已用 68.8s `clip_full.wav` 跑通 Rust 双模型对齐，
   时间轴与 `clip_aligned.srt` 基本一致（见 `scripts/asrtest/rust_validation.md`）

### 相关资产（✅ 已移入仓库）

- 原型脚本：`scripts/asrtest/align_srt.py`
  （`uv run --with sherpa-onnx==1.13.3 --with numpy python align_srt.py <wav> <out.srt>`）
- 模型：`~/.vtrim/models/`（sensevoice/firered-zh/qwen3-asr）
- 测试音频：`scripts/asrtest/wavs/`（含用户片段 clip_full.wav / clip30.wav）
- 对比数据：`scripts/asrtest/results.json`（5 模型 × 7 音频 CER/RTF）作为回归基准

---

## 评审结论与工程化补充（2026-08 代码评审）

> 目标：**识别效果够好 + 句子级时间戳**。方案技术路线可靠（双模型时间戳对齐是成熟思路，68.8s 原型已验证），
> 但原型覆盖只有单个短片段，以下为评审后的修正项、集成约束与后续优化方向，实施前必须纳入计划。

### 1. 计划修正

- 配置开关生效范围：仅 qwen3-asr（已改，见上）
- 内存声明：+160MB 为估算，release 实测后更新（已标注）
- 原型资产入库：align_srt.py / wavs / results.json 移入 `scripts/asrtest/`（Temp 目录随时被系统清理）

### 2. 集成约束（实施前必须界定，否则出现"两套后处理叠加"）

| 约束 | 说明 |
|------|------|
| 后处理职责 | v5 断句**替换** `split_text_by_ratio` / `build_segments_from_timestamps`（`asr/local.rs`）；`transcribe_chunked` 尾部的短段合并/噪声清理由 v5 自包含，仅保留 chunk 重叠过滤 |
| transcript.json 联动 | Transcriber 同时写 `video.srt` + `transcript.json`（`asr/mod.rs:86-89`），detect/correct/cut 全部消费后者——**对齐结果必须同步写 json**，否则事件边界仍用比例法时间轴 |
| chunk 边界一致性 | 每 chunk 独立对齐，合并后做**全局单调强制**（现有 sort+dedup 不够，重叠区时间戳可能倒退/不连续） |
| 模型联动 | `ensure_models`（`download.rs`）在 model=qwen3-asr 时顺带确保 sensevoice 模型存在；sensevoice 加载失败 → 降级不报错 |
| 配置流程 | 新增字段走 AGENTS.md 四步：types.rs 字段 + merge 条件 + Default impl + 模板 |

### 3. 降级链（三级，每级在日志记录触发原因）

1. 双模型对齐（qwen3 文本 + sensevoice 时间轴）
2. 锚点覆盖率 <60% 或 sensevoice 缺失 → sensevoice 时间戳直出（现有 `build_segments_from_timestamps` 路径）
3. 上述不可用 → 比例法（现状）

### 4. 质量指标（✅ 已实现 `AsrMetadata`，写入 transcript.json，`#[serde(default)]` 向后兼容）

锚点数 / 锚点覆盖率 / 最大插值跨度 / 降级原因 / 模型版本。
`vtrim status` 可展示，用户据此调 segment_gap 与 v5 阈值。

### 5. 验收基准（量化"句子级"）

- 边界误差：68.8s 对照 + 新增回归集（10~20 个 60s 片段，人工标注句边界），**±0.3s 内命中率 ≥90%**
- 新增用例：连续音乐 >60s（验证 BGM 不出假锚点）、锚点极低输入（验证降级链）
- 句子完整性：句末标点结尾占比、无重叠/无空洞

### 6. 后续优化方向（排期后做，按性价比排序）

1. **统一时间轴接口**（架构级）：所有模型产出 (chars, times) 统一结构 + 单一断句器（v5），
   sensevoice 直出 / qwen3 对齐 / firered 比例法三路合一，避免再长第四套分支
2. **n-gram 锚点优先**：先做 2~4 字精确 n-gram 锚点，间隙再做局部 DP（字符级 LCS 在长文本上碎片化，n-gram 更稳）
3. **同音字容错锚点**：qwen3 与 sensevoice 错字高度同音（法环/发还），匹配引入拼音相似度（pinyin crate），显著提升锚点密度
4. **hotwords**：`local.rs` 的 qwen3/sensevoice 已支持 hotwords 字段（当前传 None）——注入主播名/游戏名/专有名词直降 CER；`vtuber_name` 自动加入
5. **订正后重映射**：`vtrim correct` 改文本不改时间戳，字数变化导致句级时间戳漂移——订正后按 (新文本, 原时间轴) 重对齐或比例重映射
6. **VAD 辅助边界**（sensevoice 缺失时兜底增强）：sherpa SileroVAD 静音边界作为句子边界候选，优于纯比例法
7. **展示层换行解耦**：句子级时间戳保持，超长句在 SRT 展示层拆行（≤2 行/条），不影响事件边界

### 7. v5 断句规则优先级（明确化，原计划未定义）

`句末标点（。！？）强制切` > `停顿 gap>segment_gap 强制切` > `逗号软切（段>6s 或 >20 字）` > `单段 ≤8s` > `无标点按 16 字比例切`；
短段 <1.5s 并入前段。segment_gap 配置保留并作为第二优先级（对齐后的字符级静音间隔是可靠切段信号）。
