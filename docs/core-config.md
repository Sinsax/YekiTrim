# `crates/yekitrim-core/src/config` 卡外迁正文（样例全文 / 死字段审计 / 版本迁移 / 设置页出口）

> 配套模块卡：`crates/yekitrim-core/src/config/AGENTS.md`（DSH 在触到本目录时自动注入）。
> 这份文档**不占指令预算**：卡里只留"改配置代码前必须知道、否则会写错"的规矩句与 ⚠️ 行；
> 三份配置的逐字样例、死字段审计表、迁移沿革与前端出口的实测全部搬到这里。
> 判断标准：**改代码前必须知道、否则会写错的留卡；想追根问底时才看的进这里。**
> 相邻文档：`docs/plan/single-user-config.md`（程序配置单点收敛的决策正文）、
> `docs/plan/font-library.md`（字体库）、`docs/file-layout.md`（数据目录与账号树）。

分节：

- §1 两层配置的逐字样例（`yekitrim.toml` / `config.toml` / `llm.toml`）
- §2 死字段审计（2026-10 已删的 4 个字段与复现口径）
- §3 `config_version` 与读时迁移（v0→v1 / v1→v2 / v2→v3）
- §4 程序配置单点收敛（2026-11-13，方案 A）的事故背景
- §5 设置页数值项的实测与几何（配置字段的前端出口）

---

## §1 两层配置的逐字样例

> 卡里保留 `yekitrim.toml` 的精简样例（够看清结构与素材清单形态）；三份的**逐字全文**在这里。
> 权威模板是 `config/mod.rs` 的 `GLOBAL_CONFIG_TOML` / `LLM_CONFIG_TOML` 常量 —— 文档与模板不一致时以模板为准。

### 1.1 `yekitrim.toml` — 项目配置（逐字）

```toml
config_version = 3           # 配置结构版本，别手改（见「版本号与读时迁移」）
input_file = ""
danmaku_file = ""            # 唯一生效值（清单里「使用中」那条）
transcript_file = ""
audio_file = ""
events_file = ""

# 素材清单（预处理页的列表；生效值仍是上面的 *_file，这里只是"能切回来"的候选）
# source = "asr" 的是 ASR 链路产物，mode 记它属于哪个模式 → 那个模式的转写名额；用户自己选的为空
[project.materials]
transcript = [{ path = "output/transcript.hybrid.json", source = "asr", mode = "hybrid" }]
danmaku = []

[detection]
danmaku_window_secs = 10

[cutting]
output_dir = "./output"
overwrite = false
```

### 1.2 `.yekitrim/config.toml` — 全局程序配置（逐字）

```toml
config_version = 3           # 配置结构版本，别手改

[asr]
mode = "hybrid"              # 默认 hybrid（预处理页有下拉；也能改这里/项目 yekitrim.toml）
model = "sensevoice"         # ASR 模型: sensevoice(快) | qwen3-asr(最准,慢)（hybrid 填词固定 qwen3-asr）
num_workers = 0              # 0=auto(CPU/4,上限4), N=指定并行数(不超过CPU核数)
chunk_duration = 60.0        # 每块秒数
chunk_overlap = 10.0         # 块间重叠秒数
segment_gap = 1.0            # 强停顿阈值(秒)：相邻字停顿超过它就直接切一条（与标点无关）
breath_gap = 0.25            # 气口阈值(秒)：标点切点吸附到这层停顿上（只定位、不单独切分）
soft_cut_secs = 6.0          # 句中标点（，、；：）软切的时间阈值(秒)
max_segment_chars = 30       # 单条字幕字数硬上限（只在没标点/气口可用时兜底）
snap_window = 0.3            # 标点边界吸附到气口的半径(秒)
llm_correct = false          # LLM 订正转录文本（需配置 [llm]）
vad = true                   # 仅 single 模式：先 VAD 只解码语音段（快 2.3×）；hybrid 不受影响
vad_threshold = 0.5          # silero 概率阈值：调高 = 更保守（多解码、少省时间）
vad_rescue_db = 16.0         # 动态电平兜底(dB)：只多解码不少解码，0=关

[correct]
system_prompt = "..."       # 订正 prompt（覆盖内建默认）

[detection]
# 主播名称 (非空时自动追加到切片标题末尾)
vtuber_name = ""
# 弹幕密度统计窗口(秒)
danmaku_window_secs = 10
# 标题+摘要 LLM 传播潜力评分权重（0=关闭；最终热度 = (1-w)*弹幕/字幕热度 + w*LLM评分）
llm_score_weight = 0.3
# 检测 system_prompt（修改此处覆盖内建默认）
system_prompt = "..."
# 检测 user prompt_template（{segments} 会被替换；留空 = 内建默认）
prompt_template = "..."

[cutting]
output_dir = "./output"
overwrite = false

[sfx]
# 音效屏蔽列表 (yekitrim polish 使用): 精确 id 屏蔽单个音效, "tag:xxx" 屏蔽整个标签族
blacklist = []
```

### 1.3 `.yekitrim/llm.toml` — LLM 配置（独立文件，逐字）

```toml
config_version = 3

[llm]
base_url = ""              # OpenAI 兼容 API 地址
api_key = ""
model = ""
max_concurrent = 3
# ⚠️ 下面这行样例 2026-11-14 已与 LLM_CONFIG_TOML 模板对齐（旧文档写 disabled 且没有 response_format）
extra_params = { seed = 42, thinking = { type = "enabled" }, response_format = { type = "json_object" } }   # 逃逸口，优先级最低

# **请求参数面**（省略 = 不发送）；链路没覆盖就跟随这里
[llm.params]
temperature = 0
max_tokens = 65536
# reasoning = "high"       # off|minimal|low|medium|high|xhigh|max；省略 = 不表态
# thinking_budget = 16384  # 预算型方言用（省略 = 按档位给默认：1024/4096/8192/16384/32768/65536）
# top_p = 1.0
# json_mode = true

# 按链路覆盖：**同一套请求参数面**，缺一项 = 跟随 [llm.params]（`LlmConfig::for_link` 是唯一解析入口）
[llm.overrides.detect]
max_tokens = 8192
[llm.overrides.polish]
max_tokens = 32768
reasoning = "off"          # 精修默认关思考（链路代码默认给的就是这个）

# **多份 provider**（v3，2026-10-19）：`[[llm.providers]]`（id/name/api/base_url/api_key/model + 自己的
# params/models）配多份，`active = "<id>"` 指哪份在用；**active 空 = 走上面这些老键**（R4：老文件零改动）。
# `api` = 协议注册表的 id（`yekitrim_llm::protocol`，本轮只有 openai-chat）；写错由 `PUT /config/llm` 回 400。
# **能力面**：内置表没收录的模型在这里手填，只覆盖写了的字段
# [llm.models."cn:vendor.model"]
# thinking_wire = "thinking_type"      # 取值 = llm crate **方言注册表的 id**（llm/src/dialect.rs::DIALECTS）
# 清单别在这儿抄（会腐坏）：设置页「模型能力覆盖」的下拉由后端现算；
# 有一条**注册但不发送**（Gemini 走另一族 API，接它要先有协议适配）。
# max_output_wire = "max_tokens"       # 取值见同一个注册表的 max_output_field
# temperature = "unsupported"          # supported|unsupported
# reasoning = { off = "", high = "high" }   # 空串 = 支持但不发送
```

## §2 死字段审计（2026-10 已删的 4 个字段与复现口径）

下面 4 个字段**曾经定义了但全仓没有任何读取方**，已按「新增配置字段步骤」的反向流程从 `types.rs`
（结构体 + `Default` + 两个 `default_*` 函数）、`mod.rs`（`merge()` 条件 + 模板行）一并删除：

| 字段 | 为什么没人读 |
|---|---|
| `asr.align_model` | 打轴模型已固定为 SenseVoice |
| `asr.max_segment_duration` | 单段时长上限从未生效（最长时长交给 LLM 按话题完整性决定） |
| `asr.segment_merge_gap` | 短段合并从未接上 |
| `cutting.silence_noise_db` | 静音阈值实际走 `cutting::DEFAULT_NOISE_DB` 常量或 CLI `--threshold`（`service::silence::run_silence`），**从来不看配置里这个值** |

**审计口径（复现用）**：对每个字段在 `crates/` 里找 `.<字段名>` 的读取点，**必须排除 `config/` 自身**
—— 那里只有定义 / `merge` / 模板文本，会把"死的"伪装成"在用的"（实测踩过）。

**⚠️ 老 toml 里还留着这几行不会炸**：`Config` / `AsrConfig` / `CuttingConfig` **都没有**
`deny_unknown_fields`（只有 polish 的 `polish.toml` 有）。这条由
`config::removed_fields_tests::legacy_toml_with_removed_keys_still_loads` 钉住 —— 给这三处**任何一处**
加 `deny_unknown_fields` 都会让判据翻面（变异自证跑过）。

它们**已经从设置页表单里摘掉**（表单不再写回，字段本身留在文件里、仍可用「编辑源文件」改）。
真正的清理（从 `Config` 结构体里删）要走「新增配置字段步骤」的反向流程，且要确认旧文件里的多余键
不会让反序列化失败（serde 默认忽略未知字段）—— **本次没做，留给你拍板**。

## §3 `config_version` 与读时迁移

**每个配置文件顶层都有一个 `config_version`**（`config/version.rs` 是唯一实现）。

| 文件 | 位置 | 升级后写回？ |
|---|---|---|
| `config.toml` / `llm.toml` / `server.toml` | `<数据目录>/.yekitrim/` | ✅ 写回（先备份 `.bak-v<旧>`），一个目录各一份 |
| `yekitrim.toml` / `polish.toml` | **每个工程一份** | ❌ **只在内存里升，不碰磁盘**（工程可能几百个、还可能在网络盘上） |

读的时候三种情况：**版本相同 ⇒ 原样用** · **版本更低 ⇒ 先迁移再解析** · **版本更高 ⇒ 一个字节都不动**
（那是更新的程序写的；没有这一条，用旧版 exe 打开新配置会把文件"升"回去 —— 直接损坏用户数据）。
缺键 = **0**（加版本号之前的文件）。

迁移是**纯函数 + 保注释**：用 `toml_edit`（`toml::to_string` 重序列化会**把注释全丢掉**，而模板里大段说明
都写在注释里）。`v0 → v1` 清掉的就是 §2 那 4 个从来没读取方的键。

### 3.1 `v2 → v3`（2026-10-19，llm.toml）

顶层 `api_key/base_url/model` + `[llm.params]` 搬成**一份 provider**（`[[llm.providers]]`，`id = "default"` /
`name = "默认"` / `api = "openai-chat"`）+ `active = "default"`。
⚠️ `[llm.models."<id>"]` **留在原地**（能力覆盖与 provider 无关，换 provider 也照用；搬走会把一份拆成 N 份），
`overrides` / `max_concurrent` / `extra_params` 也都不搬。判据仍是那 12 条金色请求体。

### 3.2 `v1 → v2`（2026-10-19，llm.toml）

`[llm] temperature/max_tokens/thinking` 搬进 `[llm.params]`（`thinking` 改名 `reasoning`，
`enabled`→`high`、`disabled`→`off`），链路覆盖里的 `thinking` 同理；**两个键缺席时要落成 v1 的生效值**
（`0.1` / `65536`）—— 新形状里"缺席"= **不发送**，而 v1 那两个键有代码级默认值、每次都真的发出去。
判据是"迁移前后**请求体逐字节相同**"：`version::tests::migrated_v1_llm_toml_produces_byte_identical_bodies`
（12 行金色 JSON，含"省掉两个键"的那个文件）。**改这段迁移前先看那条用例**。

### 3.3 「搬家/改名」类迁移的写法

`retired_in` 只会**删**，表达不了"搬到别处" —— 那种要在 `migrate_step()` 里写一段一步一段的迁移
（`migrate_llm_v2` 是样板：**连键的 decor 一起搬**，上方注释不丢；搬完再让 `retired_in` 兜底清掉
搬不动的残留）。

### 3.4 工程级"只在内存里升"的代价

磁盘上那几行**永远在** ⇒ 每次读都会重新发现它。所以：① 报要**去重**（`report_retired`：每文件每进程只报一次，
实测 100ms 曾刷 10 条）；② 真清走 **`clean_retired_keys_in`**（CLI `yekitrim config migrate <工程目录|toml>`，
清前留 `.bak-v<旧>`；**返回空 = 不动磁盘**）。`migrate_text` 是**纯函数、不打日志**，它只把清掉的键放进
`Prepared::retired`（日志在调用方 —— 只有调用方知道路径与"写没写回"）。

### 3.5 判据

`config::version::tests`（13 条，含**文件版本更高时不动**、幂等、注释不丢、程序级写回+备份 / 工程级不碰磁盘 /
`clean_retired_keys_in` 真写回且幂等 / **同一文件只报一次**）。**变异自证**跑过：`CONFIG_VERSION` 改回 0、
或把 4 个死键从 `retired_in` 里去掉，都会让判据翻面（不是空转）。

### 3.6 `polish.toml` 的版本键为什么在文本层

那个结构体是 `deny_unknown_fields`，而且还会出口给前端与出片底座 —— 把版本塞进 schema 会波及它们。
所以写侧 `to_toml_commented()` 在文本里补一行、读侧 `validate.rs` 用 `version::strip_version_key()`
摘掉再看结构体。

## §4 程序配置单点收敛（2026-11-13，方案 A）的事故背景

**唯一路径** = `config::global_config_path()` = `<数据>/users/<uid>/.yekitrim/config.toml`。
读写两侧都用它：`load_global_config` / `load_config_at` / `load_llm_config` / `ensure_global_config` /
设置页与接入页的写通道。

**事故形状**：设置页写账号树，而启动初始化与接入页还写**顶层** —— 两份同模板、各被一半消费者读
（实测症状：参考标题不生效、检测静默不调模型）。

**启动迁移 `migrate_top_level_config_to`**：`[ingest] [storage] [publish] [auth]` 顶层赢，其余段与 `llm.toml`
账号赢（只补缺失），旧文件改名 `*.migrated-<日期>.bak`；幂等、fail-soft。判据 `config::config_migration_tests`；
正文 `docs/plan/single-user-config.md`。

## §5 设置页数值项的实测与几何（配置字段的前端出口）

设置页的数值项走 `components/SliderField.vue`：滑块 + 标签行右端可直填的数字框，共 **16 处**。

**为什么手填值不吸附 step**：滑块有 `step` 栅格，**栅格外的值拖不出来**（想设 `chunk_duration = 63` 只能
眼看着它跳 60/65）—— 所以手填的值只夹 `[min, max]`、**不吸附 step**（step 是滑块的语义，不是配置的语义），
且**清空/非数字不写模型**（否则退格清空那一瞬就把 60 写成 0）。

**几何**：数字框沿用 `.lbl-row` / `.rv` 的**标签行右端**约定（app.css），别挪到滑块右边（会被读成下一格的读数）。
**滑块宽度 = 所在格子的 1/3**（`.slf-range` 的 `flex:0 0 33.333%`，以前吃满整格；`input[type=range]` 的
`width:100%` 会被 flex-basis 盖掉，别再改回 100%）—— 三样东西因此能在**一行**里读完，行高从「标签行 +
整宽滑块」两行压成一行。窄格（LLM 那几列，格子 ≤220px）由 `@container` 把标签挤到独占一行，**别改成媒体查询**
（量的是格子宽而不是视口宽）。实测 820/1280/1600 各档均无横向溢出。改完要重建 `webui/dist`
（否则 server 端出的还是旧界面）。

**ASR 的 `Mode` 与它的说明挤成一行**（`.mode-field`，「标签 · 下拉（`width:auto`）· 说明」，实测 31px）：
其他页（PolishView / CoverDesigner / DownloaderConfig）仍是旧写法，要用就自己换。

**LLM 配置的「按链路覆盖」同样是 SliderField**（一行一条链路：勾选框 + 两个滑块 + reasoning 下拉）。
"这一项没有值"用**勾选框**表达、不是空输入框：不勾 = 整条链路发 `null`（= 删键、跟随全局）；
勾上 = 把该条的 max_tokens/temperature 写实（reasoning 只有选具体值才写）。
