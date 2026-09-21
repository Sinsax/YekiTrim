# V-Trim Roadmap

待检验和排期的新功能。

## 进行中：多用户服务（账号隔离 + 录播姬接入）

⚠️ **不是"远期目标"** —— 本节 P1 / P2 / P3 + 邀请码自助注册**已全部落地**，
**P4（B站链接通道）也已落地**（2026-09-19；选型/实测/计划见 `docs/plan/p4-*.md`，
硬规矩见 `crates/vtrim-service/src/bilibili/AGENTS.md`、`.../downloader/AGENTS.md` 与
`crates/vtrim-server/AGENTS.md`（`download_worker`））。
只剩 **P5（容量与运维）**。
2026-09-18 的结构前置（`server/` → `vtrim-service` + `vtrim-server` 两个 crate，
`api.rs` 按领域拆成 `api/` 模块树）已完成，见 `docs/plan/service-direction-review.md`。

把 `vtrim server` 从"单机单用户"改造为"单机多账号"（≤10 人）：**部署即生成管理员账号与初始密码**，
**单人/多人共用同一套认证逻辑（无免登录，本机也要登录）**，唯一门槛是"对外监听要求已改初始密码"；
项目按账户隔离、平台统一 LLM key/模型与全局并发上限、**对接录播姬**（录制完成 webhook 自动
入库并跑管线，多分片会话自动聚合、XML 弹幕直解）、B站链接转项目（粘贴链接 →
解析预览 → 勾选分P → yt-dlp 下载 → 弹幕旁路自取 → 自动建项目，与录播姬产物同构）、
只限制最大磁盘空间（**源录播不自动释放**，仅在项目页手动删源）。

- 完整设计（决策记录、现状冲突点、里程碑 P1–P5、测试与验收、风险与回滚）：
  `docs/superpowers/specs/2026-09-12-multi-user-server-design.md`
- 地基已完成（2026-09-12 提交 `ba1dafb`、`5e9d4c0`）：JSON 404 兜底、单活动任务与协作式取消、
  SSE 历史回放、Range/路径校验、`filelock` 串行化、`webui/dist` 入库——见设计稿 §13。
  （「单活动任务」后来先放宽成"资源域互斥 + 单一并发上限 3"，再细化为**三档闸门 + 前后台优先级**
  ——轻活不排队、略重/重活各有名额，见 `crates/vtrim-server/AGENTS.md`（任务队列）与 `vtrim-server/src/jobs.rs`。）

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

### P3. ✅ 已重构 — 特效 duration 按情景分类 clamp

- 根因：LLM 写 0.3~1s duration，渲染侧直接使用，导致 blur 等瞬时特效长时间驻留
- 重构后：
  1. `process/fx.rs` 新增 `normalize_fx_durations`：瞬时特效（shake/flash/blur/zoom_bounce/
     vignette/hue_shift/stutter/split/noise/pulse/color_shift）统一 clamp 0.1~0.6s，
     blur 上限 0.4s，缺省自动填默认值
  2. Step2 prompt 改为“瞬时特效 duration 由系统接管；blur 只用于瞬间失焦/眩晕/闪回”
  3. 模板 `templates/index.html` 用 `transientDur(ev, def)` 兜底，blur 改为快速脉冲无 hold
- 涉及：`process/fx.rs`、`process/mod.rs`、`llm/segment.rs`、`templates/index.html`

### P4. ✅ 已重构 — LLM 流程辅助项

- 字幕 `take(100)` 截断 → Step1 改为预算感知采样（首尾 + 中部均匀采样到 200 条）
- 解析失败无反馈 → Step1/Step2 解析失败错误回传 LLM 重试 1 次
- Step2 已迁移到 core `Agent`：每段一个窗口，并发 ≤3 + checkpoint 续传 + 错误容忍
- 无匹配率报告：仍待补（后续）
- 涉及：`llm/segment.rs`、`llm/events.rs`、`llm/mod.rs`、`process/mod.rs`

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

### 断句规则（v5 —— ✅ 已按实测结论重做，见下）

原计划：句末标点强制切 / 逗号软切（>6s 或 >20 字）/ 单段 ≤8s / 短段 <1.5s 并入前段。

**实测推翻了这个前提**（`cargo test -p vtrim-core --lib diagnose_sensevoice_tokens -- --ignored`，
真实音频一块 60s）：**SenseVoice 根本不吐标点**（全文一个 `。！？` 都没有），token 还是单字的（89/89）。
所以"句末标点强制切"在真实数据上**永远没有信号可用** —— 不是判断写得不对，是没有标点可判。
同时"短段并入前段"正好把切好的短问句重新粘起来。

改为（`crates/vtrim-core/src/asr/align.rs`）：

| 作用 | 信号 | 来源 |
|------|------|------|
| 切成几条 | 句末/句中标点、强停顿（`segment_gap`） | 最终文本 + 时间轴 |
| 切在哪一刻 | **气口**（相邻字停顿 ≥ `breath_gap`，实测问句边界 +0.42s、普通字间 +0.12~0.24s） | 时间轴（SenseVoice，唯一有时间戳的一侧） |
| 不许超标 | 字数（`max_segment_chars`） | 最终文本 |

气口**只用来定位切点**，不单独作为切分信号 —— 实测词中间也有 0.30s 停顿，单靠它会切碎。
`cleanup_segments` 不再合并短段。中间产物落 `output/asr/`（原始 token / 时间轴 / 切点来源）。

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
