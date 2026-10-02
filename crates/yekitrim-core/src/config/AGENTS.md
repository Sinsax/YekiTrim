# yekitrim-core/src/config — 配置类型、两层合并与安全写入

> 范围：`crates/yekitrim-core/src/config`/** ｜ 上游：core 各 stage、`yekitrim-service`、`yekitrim-server`（`/api/config`）、`yekitrim-cli` ｜ 验证：`cargo test -p yekitrim-core --lib config`

## 职责

定义全站配置类型（`types.rs`）与读写合并（`mod.rs`）。两层合并:
`.vtrim/config.toml` (程序) → `vtrim.toml` (项目, 后加载覆盖前者)。无环境变量。
实现见 `yekitrim-core/src/config/mod.rs::load_config_at`（先 global 后 project）。
此外提供 `patch_config_file`（设置页表单保存通道）、`write_project_config` 与几个
`save_*_path_in`（stage 落盘后回写生效值）。

## 入口

- `config::load_config()` / `load_config_at(project_file)` / `load_global_config()` / `load_llm_config()`。
- `config::patch_config_file(path, &serde_json::Value)` —— 设置页表单保存的唯一通道。
- `config::write_project_config` / `write_project_config_exact` / `normalize_project_paths_in`；
  `save_events_path[_in]` / `save_audio_path[_in]` / `save_transcript_path[_in]`。
- `config::resolve_reference_titles(&DetectionConfig) -> Vec<String>`。
- `types::{Config, ProjectConfig, Materials, MaterialEntry, AuthConfig, IngestConfig, RecorderConfig,
  StorageConfig, AudioConfig, AsrConfig, DetectionConfig, LlmConfig, CuttingConfig, SfxConfig,
  ChromeConfig, CorrectConfig}`；`ProjectConfig::current_stage(&output_dir)`；
  `MaterialEntry::{asr, user, is_asr, slot}` / `Materials::{of, of_mut}`；
  `LlmConfig::concurrency(section_override)`（并发）、`LlmConfig::for_link(LlmLink)`（按链路的
  调用参数，`LlmOverride` / `LlmOverrides` 是它的配置形态）。

## 边界

**merge 条件**: String 字段 `!is_empty()`, f64/u32 字段 != 默认值, Vec 字段非空时追加合并。

**配置默认值注意事项**: 所有配置类型必须手动实现 `Default`（而非 `#[derive(Default)]`）以确保 serde 默认值与 Rust 默认值一致。`Config::default()` 用 `toml::from_str("")` 初始化，依赖各字段的 `#[serde(default)]` 来调用正确的 `Default` impl。新增字段时必须同时加 `Default` impl + `merge()` 条件。

### 新增配置字段步骤

1. `config/types.rs` 加字段 + serde default
2. `config/mod.rs` 的 `merge()` 加对应条件（注意 `.vtrim/config.toml` 在 `vtrim.toml` 之后 merge）
   —— ⚠️ **`[llm]` 是例外：它不参与 merge**，`load_config_at` 最后一句 `config.llm = load_llm_config()`
   无条件覆盖，所以 LLM 相关字段只改 types/mod 的模板，不用碰 `merge()`
3. 结构体手动 `Default` impl (有字段级 serde default 时必须)
4. `config/mod.rs` 的模板常量 + 注释（`GLOBAL_CONFIG_TOML` = config.toml / `LLM_CONFIG_TOML` = llm.toml；
   仓库里**没有** `config/default.toml` 这个文件，旧文档里的写法是错的）

### 两层样例（逐字保留）

**`vtrim.toml`** — 项目配置:
```toml
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
silence_noise_db = -30
```

**`.vtrim/config.toml`** — 全局程序配置:
```toml
[asr]
mode = "hybrid"              # 默认 hybrid（预处理页有下拉；也能改这里/项目 vtrim.toml）
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
# ⚠️ align_model / max_segment_duration / segment_merge_gap 已废弃（无人读取）

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
silence_noise_db = -30

[sfx]
# 音效屏蔽列表 (vtrim polish 使用): 精确 id 屏蔽单个音效, "tag:xxx" 屏蔽整个标签族
blacklist = []
```

**`.vtrim/llm.toml`** — LLM 配置（独立文件）:
```toml
[llm]
base_url = ""              # OpenAI 兼容 API 地址
api_key = ""
model = ""
temperature = 0
max_tokens = 65536         # 没单独配覆盖的链路用它
extra_params = { seed = 42, thinking = { type = "disabled" } }

# 按链路覆盖：缺一项 = 跟随 [llm] 的同名项（`LlmConfig::for_link` 是唯一解析入口）
[llm.overrides.detect]
max_tokens = 8192
[llm.overrides.polish]
max_tokens = 32768
```

### 写入纪律

`vtrim init` 生成 `vtrim.toml`（项目字段：input/danmaku/转写/剪切参数）。
`vtrim config init` 生成 `.vtrim/config.toml`（`[asr]` + `[detection]` + `[correct]` + `[sfx]`）和 `.vtrim/llm.toml`（`[llm]`，覆盖写）。
`vtrim server` 启动时若程序配置缺失会自动补齐（`yekitrim_service::ensure_global_config`）：只创建缺失文件；
已存在但 TOML 语法错误的会备份为 `*.toml.broken` 后重建（坏文件会让整份配置被忽略）。
设置页表单只覆盖常用项，保存走 `config::patch_config_file`（toml_edit 合并，保留其他 section 与注释），
表单数据源用 `/api/config` 的 `global_config`（global-only），不能用 merged 的 `config`。

⚠️ **`patch_config_file` 支持两层以上（递归深合并）**：值是对象时进子表继续合并 ——
`{"ingest": {"recorder": {"api_base": …}}}` 只改命中的键，`[ingest.recorder]` 里没提到的兄弟键
（`root`/`api_user`/`api_pass`…）与**段内注释**都保留。**别再退回"一层 section → key"**：
那时 `recorder` 会被整项换成 inline table，把整张子表静默丢掉（文件仍是合法 TOML，不报错，
下次读配置才发现值没了）。回归钉子：`config::patch_config_tests`（含"兄弟键与注释不丢"）。
新增需要写嵌套 section 的表单时直接用这个通道，不必再开专用端点。

⚠️ **`null` = 删键**（`merge_into_table` 的第一条分支）：patch 是合并语义，这是唯一能表达
"取消该键 / 回到跟随全局"的写法 —— 设置页把「按链路覆盖」那条的勾**取消**走的就是它。
**别再退回"null → 空串"**：`max_tokens = ""` 是合法 TOML 但读不出 u32，`load_llm_config`
解析失败会把**整份 LLM 配置（含 api_key）打回默认值**。回归钉子：`null_patch_removes_key_and_keeps_siblings`
与 `empty_subtable_patch_changes_nothing`（`{}` 不删表、不吞注释）。

### LLM 按链路覆盖（2026-09）

- **一个入口**：`LlmConfig::for_link(LlmLink)`（`config/types.rs`）—— `max_tokens` / `temperature` /
  `thinking` 三层优先级的**唯一实现**（链路覆盖 > `[llm]` > 链路代码默认）。判据与理由见
  `crates/yekitrim-core/src/llm/AGENTS.md`，这里只记配置形态：`[llm.overrides.{detect,polish,correct,utility}]`，
  未知键忽略（拼错不该让整份配置打回默认）。
- 新增链路 = `LlmLink` 加一个变体 + `ALL`/`key`/`label_zh`/`default_thinking` 四处 + `LlmOverrides` 加字段
  （设置页那张表按后端序列化出来的键自动出行，前端不用改）。
- ⚠️ **`max_concurrent` 不在这张表里**：它仍是 `[llm] max_concurrent` + `[detection]`/`[correct]` 的
  `Option` 段覆盖（上面「LLM 并发与加速」那套），两套语义并存、各管各的，别混。

### LLM 并发与加速（2026-09）

- **一个旋钮**：`.vtrim/llm.toml` 的 `[llm] max_concurrent`（默认 3），**事件检测 / 精修事件生成 /
  字幕订正共用**，解析统一走 `LlmConfig::concurrency(section_override)`（`config/types.rs`）。
  `[detection] max_concurrent` / `[correct] max_concurrent` 是**可选覆盖**（`Option<usize>`，
  留空即跟随全局）——别再在代码里写死 `3`，也别让某条链路偷偷用自己的。设置页「LLM 配置」块有输入框。
  **实测中转站允许并发且接近线性**（3 路并发墙钟 ≈ 单发 1.3 倍），所以它是批处理速度的第一杠杆。

### 死旋钮（别再加回界面）

⚠️ **`[asr]` 里 `align_model` / `max_segment_duration` / `segment_merge_gap`
是死旋钮**（全仓库没有任何读取方，只有配置 merge 逻辑）：改它们不会有效果，
字段只为老 toml 不炸而保留，别再往界面加。（`segment_padding` 已从这份名单里**移出** —— 它现在真的在用。）

### ⚠️ 设置页数值项（配置字段的前端出口）

⚠️ **设置页的数值项走 `components/SliderField.vue`**（滑块 + 标签行右端可直填的数字框，16 处）。
滑块有 `step` 栅格，**栅格外的值拖不出来**（想设 `chunk_duration = 63` 只能眼看着它跳 60/65）——
所以手填的值只夹 `[min, max]`、**不吸附 step**（step 是滑块的语义，不是配置的语义），
且**清空/非数字不写模型**（否则退格清空那一瞬就把 60 写成 0）。
数字框位置沿用 `.lbl-row` / `.rv` 的**标签行右端**约定（app.css），别挪到滑块右边（会被读成下一格的读数）。
⚠️ **滑块宽度 = 所在格子的 1/3**（`.slf-range` 的 `flex:0 0 33.333%`，以前吃满整格；`input[type=range]` 的
`width:100%` 会被 flex-basis 盖掉，别再改回 100%）——三样东西因此能在**一行**里读完，行高从「标签行 + 整宽滑块」
两行压成一行。窄格（LLM 那几列，格子 ≤220px）由 `@container` 把标签挤到独占一行，**别改成媒体查询**
（量的是格子宽而不是视口宽）。实测（1600 宽 + 真实 Chrome）：格宽 400 → 滑块 133，数字框右缘与格子右缘齐平（spill 0），
820/980/1100/1280 各档都没有横向溢出。改完要么重建 `webui/dist`，否则 `vtrim server` 端出的还是旧界面。
⚠️ **ASR 的 `Mode` 与它的说明挤成一行**（`.mode-field`）：说明原本另起一行，把那一格撑到 **81px**，而同一行的
数值行只有 23px —— 那一行右侧空出 58px 竖向留白（实测）。改成「标签 · 下拉（`width:auto`）· 说明」后那一行 31px。
其他页（PolishView / CoverDesigner / DownloaderConfig）仍是旧写法，要用就自己换。
⚠️ **LLM 配置的「按链路覆盖」同样是 SliderField**（一行一条链路：勾选框 + 两个滑块 + thinking 下拉）。
"这一项没有值"用**勾选框**表达、不是空输入框：不勾 = 整条链路发 `null`（= 删键、跟随全局）；
勾上 = 把该条的 max_tokens/temperature 写实（thinking 只有选具体值才写）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-core --lib config
```

覆盖 `patch_config_tests`（深合并/兄弟键与注释不丢/**null 删键**）、`types::llm_link_tests`
（链路覆盖三级优先级 / 坏值兜底 / 模板可解析）、`vad_merge_tests`（`Option<bool>` 显式覆盖口径）、
`reference_titles_tests` —— 动了 `merge()`、`Default` impl、`for_link` 或模板就必须看到它们全绿。
