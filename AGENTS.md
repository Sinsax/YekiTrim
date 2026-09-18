# V-Trim

直播录像自动切片工具。输入录播视频 + 弹幕文件，输出按事件分割的片段 + 字幕 + `publish_config.json`。

## 架构

Cargo workspace, 三个 crate:
- `crates/vtrim-core` — 管线编排 + 全部业务逻辑
- `crates/vtrim-cli` — CLI 入口 (clap)
- `crates/vtrim-polish` — GSAP 精修引擎（从 vtedit 合并）

**Stage 编号**: AudioExtractor(1) → Transcriber(2) → EventDetector(3) → Cutter(4)，连续无跳过。

所有 stage 用 `#[async_trait]` 实现 `Stage` trait (`crates/vtrim-core/src/pipeline/stage.rs`)。

`PipelineRunner` 提供三种运行模式 (`runner.rs`):
- `run()` — 受文件存在性约束（而非 `current_stage`），单 stage 命令用
- `run_all()` — 无视文件检查从头跑到底，`process` 命令用
- `run_until(max)` — 跑到指定 stage 为止，`process --dry-run` 用

`current_stage` 不再持久化到 `vtrim.toml`，改为从输出文件存在性自动推导：

| 文件存在 | 推导阶段 |
|---------|---------|
| `output/video.wav` 或 `audio_file` 非空 | Stage 1 |
| `output/transcript.json` 或 `video.srt` 或 `transcript_file` 非空 | Stage 2 |
| `output/events.json` 或 `events_file` 非空 | Stage 3 |
| `output/clips/clips_order.txt` 存在 | Stage 4 |

## CLI

```
vtrim init <video> [--danmaku <file>]    # 创建 vtrim.toml
vtrim extract [-i] [-o]                  # 提取音频 (Stage 1)
vtrim process [-i] [-o] [--dry-run]      # 全自动管线
vtrim transcribe [-i]                    # Stage 2 (local)
vtrim detect [-t <transcript>]           # Stage 3 (自动发现: 参数 > config > output_dir/*.json > *.srt)
vtrim cut [-i] [-e <events>] [-o]        # Stage 4 (自动发现: 参数 > config/output_dir/events.json)
vtrim silence [files...]                 # 「去静音」: 长停顿映射为屏蔽轨道 (写 polish.toml, 不改视频文件)
       [--duration 3.5] [--threshold]    # 默认扫描 output/clips/, 未初始化则先 polish init
vtrim download                           # 下载全部 ASR 模型 (sensevoice/qwen3-asr) → <程序>/.vtrim/models/
vtrim status                             # 查看当前进度
vtrim usage [--days N]                   # LLM 用量（每次调用的 token/缓存命中/耗时，账本 llm_usage）
vtrim doctor [--url <base>]              # 安全与运行状况摘要（账号/初始密码/锁/磁盘/服务探测）
vtrim ingest token [--disable]           # 录播姬: 生成 webhook token / 关闭 webhook
vtrim ingest status                      # 录播姬: 采集配置概览
vtrim user invite [--note 给谁] [--days N] [--count N] [--disable]
                                         # 注册邀请码: 生成(明文落库,可重复查看)并开启注册 / --disable 关闭
vtrim user invites                       # 邀请码列表(明文 + 未使用/已使用·绑定谁) + 注册开关状态
vtrim user invite-revoke <id|完整邀请码>  # 撤销邀请码
vtrim config init [--path]               # 生成全局配置文件
vtrim config show                        # 显示当前配置 (含 LLM)
vtrim correct [-t <transcript>]          # LLM 订正转录文本
vtrim polish init [<dir>]                # 精修: 初始化项目 + 解析字幕弹幕
vtrim polish llm [<dir>]                 # 精修: LLM 生成 segments（使用 .vtrim/llm.toml）
vtrim polish process [<dir>]             # 精修: 后处理 + 下载资源 + 计算位置
vtrim polish render [<dir>]              # 精修: 渲染视频（自动先跑 process）
vtrim polish cover [<dir>] [--template]  # 精修: 生成封面
vtrim polish preview [<dir>] [mode]      # 精修: 浏览器预览（video/cover/双端口）
vtrim polish doctor [<dir>]              # 精修: 系统检查
vtrim polish test [<dir>]                # 精修: 测试模式
vtrim polish stickers list               # 本地贴纸: 列出索引（组名+成员数+tags）
vtrim polish stickers test <kw...>       # 本地贴纸: 测试 keyword→贴纸匹配（诊断用）
vtrim polish stickers scan [--llm]       # 本地贴纸: 生成/更新 stickers.json 清单（--llm 批量生成英文 tags）
```

所有 stage 命令的 `-i` 可选，默认从 `vtrim.toml` 的 `input_file` 读取。

## 状态机

`current_stage` 不再持久化到 `vtrim.toml`，改为从输出文件存在性自动推导。

| 规则 | 示例 |
|------|------|
| `stage > current + 1` → 拒绝 | `detect`(3) 无转写文件 → ❌ |
| `stage <= current` → 可重跑 | `transcribe`(2) 存在转写文件 → ✅ |
| 完成后 stage 自动推进 | `transcribe`(2) 成功后文件已存在 |
| `process` 无视文件检查, 全部重跑 | |
| stage 无跳过 | 缺失前置文件则拒绝 |

实现: `config/types.rs` 的 `current_stage()` 方法通过文件存在性推导。

## 配置文件

两层合并: `.vtrim/config.toml` (程序) → `vtrim.toml` (项目, 后加载覆盖前者)。无环境变量。
实现见 `vtrim-core/src/config/mod.rs::load_config_at`（先 global 后 project）。

`vtrim init` 生成 `vtrim.toml`（项目字段：input/danmaku/转写/剪切参数）。
`vtrim config init` 生成 `.vtrim/config.toml`（`[asr]` + `[detection]` + `[correct]` + `[sfx]`）和 `.vtrim/llm.toml`（`[llm]`，覆盖写）。
`vtrim server` 启动时若程序配置缺失会自动补齐（`service::ensure_global_config`）：只创建缺失文件；
已存在但 TOML 语法错误的会备份为 `*.toml.broken` 后重建（坏文件会让整份配置被忽略）。
设置页表单只覆盖常用项，保存走 `config::patch_config_file`（toml_edit 合并，保留其他 section 与注释），
表单数据源用 `/api/config` 的 `global_config`（global-only），不能用 merged 的 `config`。

⚠️ **`patch_config_file` 支持两层以上（递归深合并）**：值是对象时进子表继续合并 ——
`{"ingest": {"recorder": {"api_base": …}}}` 只改命中的键，`[ingest.recorder]` 里没提到的兄弟键
（`root`/`api_user`/`api_pass`…）与**段内注释**都保留。**别再退回"一层 section → key"**：
那时 `recorder` 会被整项换成 inline table，把整张子表静默丢掉（文件仍是合法 TOML，不报错，
下次读配置才发现值没了）。回归钉子：`config::patch_config_tests`（3 条，含"兄弟键与注释不丢"）。
新增需要写嵌套 section 的表单时直接用这个通道，不必再开专用端点。

**`vtrim.toml`** — 项目配置:
```toml
input_file = ""
danmaku_file = ""
transcript_file = ""
audio_file = ""
events_file = ""

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
mode = "hybrid"              # 默认 hybrid（准备页不给切换，只能改这里/项目 vtrim.toml）
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
# ⚠️ align_model / max_segment_duration / segment_merge_gap / segment_padding 已废弃（无人读取）

[correct]
system_prompt = "..."       # 订正 prompt（覆盖内建默认）

[detection]
# 主播名称 (非空时自动追加到切片标题末尾)
vtuber_name = ""
# 弹幕密度统计窗口(秒)
danmaku_window_secs = 10
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
max_tokens = 65536
extra_params = { seed = 42, thinking = { type = "disabled" } }
```

merge 条件: String 字段 `!is_empty()`, f64/u32 字段 != 默认值, Vec 字段非空时追加合并。

**配置默认值注意事项**: 所有配置类型必须手动实现 `Default`（而非 `#[derive(Default)]`）以确保 serde 默认值与 Rust 默认值一致。`Config::default()` 用 `toml::from_str("")` 初始化，依赖各字段的 `#[serde(default)]` 来调用正确的 `Default` impl。新增字段时必须同时加 `Default` impl + `merge()` 条件。

### LLM 并发与加速（2026-09）

- **一个旋钮**：`.vtrim/llm.toml` 的 `[llm] max_concurrent`（默认 3），**事件检测 / 精修事件生成 /
  字幕订正共用**，解析统一走 `LlmConfig::concurrency(section_override)`（`config/types.rs`）。
  `[detection] max_concurrent` / `[correct] max_concurrent` 是**可选覆盖**（`Option<usize>`，
  留空即跟随全局）——别再在代码里写死 `3`，也别让某条链路偷偷用自己的。设置页「LLM 配置」块有输入框。
  **实测中转站允许并发且接近线性**（3 路并发墙钟 ≈ 单发 1.3 倍），所以它是批处理速度的第一杠杆。
- **前缀缓存**：`DEFAULT_PROMPT` 里 `{segments}` 必须留在**最后**，每场固定的弹幕爆发块走
  `{hot_windows}` 排在它之前（公共前缀 86% → 98%）。provider 按最长公共前缀命中上下文缓存，
  把变化部分放中间等于把后面的固定内容全浪费掉。老模板（用户 config.toml 里的副本）没有
  `{hot_windows}`，`build_prompt` 会把它拼进 `{segments}` 的替换值里——位置等价、不丢内容。
- **checkpoint 按提示词指纹失效**：窗口 id 折入 `AgentTask::prompt_fingerprint()`（在
  `agent.rs::run_windows` 里改写），否则「字幕没变但 prompt 改了」会命中旧窗口、拿回上一版结果。
  改了 prompt / 贴纸来源 / 音效屏蔽后重跑，日志会显示「实际跳过 0 个窗口（内容匹配）」。
- `response_format = { type = "json_object" }` 放在 `[llm] extra_params` 里
  （两份 llm.toml 实例与 `config init` 模板都已带上）。中转支持就保证不写解释性前言；不支持就删掉。
- 排查偶发 `error sending request`：那是 reqwest 的**建连失败**（不是超时、不是 HTTP 状态码）。
  本机若挂着 Clash/Mihomo 的 TUN（DNS 返回 fake-ip `198.18.0.x`）就是代理那一侧的事。
  `LlmClient` 的 HTTP 选项集中在 `llm/mod.rs::build_http_client`（connect_timeout 10s /
  pool_idle_timeout 30s / tcp_keepalive 30s / 浏览器 UA），**所有选项只在那里设**——
  `with_timeout` 会重建客户端，散着写就会像 UA 那样被静默丢掉。

### LLM 用量记账（token / 缓存命中 / 耗时）

- **采集点只有一个**：`LlmClient::send`（`vtrim-core/src/llm/mod.rs`）。成功与**最终失败**都记一条
  `UsageRecord`（`llm/usage.rs`），重试算同一次调用（`attempts` 记实际尝试次数，**不按尝试拆行**）。
  要加字段就改这一处 + `usage.rs`，别去 9 个调用点各写一遍。
- **归因**：`label`（链路名）+ `project`（项目/切片目录）。Agent 系任务走
  `AgentTask::usage_context()`（默认取 `name()`，事件检测/订正/精修段事件各自补 project）；
  非 Agent 的构造点（精修分段/关键词、贴纸挑选、贴纸包标签、封面文案、LLM 测试）显式
  `.with_usage_context(...)`。**新增链路记得挂 context**，否则统计里是空 label。
- **落地**：core 只 `emit`，写库由 CLI/server 侧装的 sink 负责——`service::usage::DbSink`
  （进程级默认 sink，`usage::set_default_sink`；`LlmClient::with_usage_sink` 是测试用的私有注入）。
  安装点两处：`server::run`（`AuthService::open` 之后）与 CLI 的 `process/detect/correct/polish` 分支。
  写库失败只 warn，**绝不影响调用结果**。
- **表**：账本 v7 `llm_usage`（ts/label/project/model/host/prompt_tokens/completion_tokens/
  cached_tokens/reasoning_tokens/latency_ms/attempts/ok/status/error）。`cached_tokens` 认三种写法：
  DeepSeek `prompt_cache_hit_tokens`、OpenAI `prompt_tokens_details.cached_tokens`、缺省 0
  ——**前缀缓存省了多少，看这一列**。`latency_ms` 是从第一次尝试到拿到结果的总墙钟（含退避等待）。
- **读出来**：`GET /api/admin/usage?days=N`（**仅 admin**，记录含他人项目路径，与 `/api/admin/logs`
  同一个理由）→ 总量 + 按天/链路/项目/模型分组；设置页「LLM 用量」卡（`components/LlmUsage.vue`）；
  CLI `vtrim usage [--days N]`。聚合全走 SQL `GROUP BY`，不在 Rust 里扫表。
  **缓存率 = Σcached ÷ Σprompt**（只算输入侧、先求和再相除，不是每次调用比率的平均；
  分母不掺 completion —— 缓存只作用于输入）。`prompt_tokens == 0` 显示 `—`（老中转没报 usage /
  失败行 token 全 0，不是"没命中"）。单发链路（封面文案等）天然 0%、每次任务的第一个窗口必然 0%，
  所以看比率要按「按链路」那张表。实现只此一份：`service::usage::cache_rate`（CLI），
  前端 `LlmUsage.vue` 的 `cacheRate` 是镜像。
- 每次调用还会打一行结构化日志 `LLM 调用 label=… prompt_tokens=… cached_tokens=… latency_ms=…`
  （logbus 渲染成 `k=v` 进设置页「运行日志」与文件日志；失败是 `LLM 调用失败` + status）。

## 各阶段 I/O

### AudioExtractor (Stage 1, `vtrim extract`)
- 输入: 视频 → ffmpeg 16kHz mono WAV + ffprobe VideoInfo
- `ffmpeg -y -vn -acodec pcm_s16le -ar 16000 -ac 1` → `output/video.wav`

### Transcriber
- sherpa-onnx 库 (CPU only)，模型按 `[asr] model` 选择: `sensevoice` (默认) | `qwen3-asr` (Qwen3-ASR 0.6B int8, 最准最慢)；hybrid 模式固定 = sensevoice 打轴 + qwen3-asr 填词。**firered-zh 已下线**（CTC 时间戳在 BGM 段造幽灵时间戳，见 ROADMAP）：注册表、下载清单、界面都不再有它
- **断句 = 「SenseVoice 出时间、文本出切点」**（`asr/align.rs`，2026-09 按实测重做）：
  - **实测结论**（`VT_ASR_WAV=<wav> cargo test -p vtrim-core --lib diagnose_sensevoice_tokens -- --ignored --nocapture`）
    —— **SenseVoice 完全不吐标点**（真实一块 60s 全文无一个 `。！？`），token 是单字的（89/89）。
    所以"句末标点强制切"在真实数据上**永远没有信号**；唯一可用的边界信号是**气口**：
    实测问句边界是 `+0.42s` 的停顿，普通字间只有 `0.12~0.24s`，而 `segment_gap`(1.0s) 把这层信号整个跳过。
  - **分工**：**标点（来自最终文本，hybrid 下是 qwen3 的）+ 强停顿（`segment_gap`）决定切成几条**，
    **气口（`breath_gap`）决定切在哪一刻**，**字数（`max_segment_chars`）只做硬上限**。
    ⚠️ 气口**只用来定位切点，不是独立切分信号** —— 实测词中间也有 0.30s 停顿，单靠它会切碎。
  - `CharTimeline`（逐字时间戳，`from_tokens` 把 token 摊成单字共享时间戳）是**唯一时间权威**；
    qwen3 没有时间戳，`align_to_timeline` 用贪心+小范围重新同步把它的文本对到时间轴上
    （吃错字/漏字/多字），`split_text_on_timeline` 据此切分。首尾用**对齐后的字时间**定，
    不能照抄窗口起止（qwen3 常漏掉窗口开头几个字，照抄会让字幕提前好几秒）。
  - ⚠️ **别把"短段并回邻居"加回 `cleanup_segments`**：那正是把切好的短问句重新粘成一条的元凶。
  - ⚠️ **别再 `trim_end_matches` 删成片字幕的尾标点**（那是断句证据）。
  - **四条踩过的坑（都在这条链路上，改之前先看）**：
    ① **切点不做任何"最小时长/字数份额"的抬升**。曾按字数份额把切点往后推来避免 0.05s 的短条，
       结果窗口的 `(start, end)` 不再覆盖它自己那些字，hybrid 按 `slice_by_time` 取字取到空集
       —— 实测 1186 个窗口里 **589 个**退回"一个窗口一条"（`align.json` 少一半，细分成片丢失）。
       短条改用 `merge_too_short` **合并**解决（只动文本归属，不动时间与分区），
       且**并完超过 `max_chars` 就不并**（否则把 `怎么？你们都玩过吗？` 又粘回一条）。
    ② **`f64::clamp` 会 panic**（`min > max`），而这里的边界全是算出来的 ——
       实测在 3 小时素材上崩掉整个转写进程。一律用 `clamp_ordered`（`hi < lo` 时以 `lo` 为准）。
    ③ **切点去重分两类**：标点切点是**文本层的事实**，时间算得再近也要留
       （一律按时间判会把对齐差窗口里的三条问句并成两条）；强停顿/字数上限按**时间距**判
       （按字距判会误并「≥1s 停顿里夹着一个孤字」的两处边界，实测少留约 150 个真实停顿）。
    ④ **块间重叠必须按"下一块起点"分区**（`token_belongs_to_chunk`，半开区间 `[offset, offset+step)`）。
       老口径按块中点（`offset + dur - overlap/2`）会让 `[50s, 55s)` **两块都留下** ——
       旧代码靠 `dedup_overlaps` 在段一级兜住，而时间轴是逐字的、没有那层兜底，
       于是 5 秒的字写两遍、切出两个覆盖同一段音频的窗口（qwen3 白跑、字幕重复）。
  - 中间产物落 `output/asr/`（`raw/chunk-NNN.json` 原始 token / `timeline.json` 时间轴+气口 /
    `align.json` 每个窗口切成几条+切点来源 / `segments.raw.json` 清理前）。
    这几份记的是**切分时看到的原文**，即 `asr::text::normalize` **之前** —— 与落盘字幕有细微差别是有意的。
    `artifacts::reset` 必须先清目录：`raw/` 按块编号命名，换过 `chunk_duration` 后残留会冒充本次结果。
  - **字幕文本后处理**（`asr/text.rs::normalize`，断句之后、写盘之前，两种模式共用一处；用户定的排版规则）：
    ① **行尾只留带情绪的符号** —— `？！!?` 保留，`。.` 与 `，、；：,;:` 从行尾去掉
    （`…` `～` 保留：是语气/拖长音，不是中性标点）；
    ② **转述句式去引号** —— `xx说：“xxxx”` → `xx说，xxxx`（引号前的冒号变逗号，
    删引号后露出来的 `。”，` 叠写收敛成 `。`）。
    ⚠️ 只动**行尾与引号**，正文里的标点一个不许碰（切分是按原文标点做的）；
    空/纯符号输入原样返回，**绝不产出空字幕**；ASCII 单引号不删（`don't`）。
    注意它改变 `transcript.json` 的正文，而检测/订正/切片都吃这份文件 —— 行尾句号少了，
    检测 prompt 里的句末信号会少一点（一条 ≈ 一句已经由切分保证，损失有限）。
    诊断：`VT_TRANSCRIPT=<transcript.json> cargo test -p vtrim-core --lib diagnose_text_normalize -- --ignored --nocapture`
    （真实转写上看 before→after 与条数，**不用重跑 ASR**；实测 1096 条里 42 条走转述改写）
  - 回归钉子：`diagnose_hybrid_three_questions`（真实双模型，三个问句必须切成三条、**首条起点 = 窗口起点**，不写盘）。
- 可配置定长分块 (默认 60s + 10s 重叠)
- 并行转写: 默认 CPU/4 路并行 (上限 4)，每 chunk intra-op threads=2
- **qwen3 单流解码的线程数 = `min(4, 核数)`**（`local::decode_threads`）：这个 session 是**串行**用的
  （全局 `QWEN3_RECOGNIZER` Mutex），所以多给线程纯赚。实测 qwen3 占整条转写约八成时间
  （3 小时素材 12.4 分钟 / 1182 个窗口 vs SenseVoice 2.5 分钟）——**性能只动这里**。
  ⚠️ 若将来改多实例并行，这里要回到 1（否则 N 实例 × 4 线程互相抢核）。
- **首条字幕的起点 = 窗口起点**（`align.rs` 的 `next_start = span.0`）。
  ⚠️ **别改成 `time_of(0)`**：它对没对齐上的字返回的是**后一个锚点**的时间，
  于是"qwen3 句首与 SenseVoice 字轴对不上"时起点会往后跳 —— 那正是"句子开头几个字
  看起来没识别到"的一种成因（字幕出现时那几个字已经说过去了）。
- **生成标记必须截掉**（`text::strip_generation_markers`，在**切分之前**）：qwen3 喂到音乐/噪声段会吐
  `<|endoftext|>` 然后接着胡说（实测 `我都孤。<|endoftext|>Human Rights Watch（人权观察）…`，
  71 字/0.3 秒）。不截会自成一条字幕，还会漏进 `clip.srt`（`clip-29`/`clip-35` 踩过）。
- **`segment_padding`（切给识别器的音频余量）默认 0 = 关**。机制在 `mod.rs::decode_span`：
  前后各留一点，**前置余量只吃停顿、不碰上一个字的语音**（下限取上一个字起始 + 0.35s）。
  ⚠️ 实测 0.3s 的 A/B 不划算（`diagnose_head_padding`，3 分钟真实音频、17 个窗口）：
  11 个窗口文本变了、平均对齐命中率 **0.473 → 0.430**（更差），只换来 2 个"整窗从空到有内容"，
  所以**默认关**。想试就设 0.1~0.3 并在**自己素材**上复测。同一旋钮也控制 chunk 解码的左余量。
- **`max_new_tokens` 默认 512**（曾 2048）：上游 sherpa-onnx 对 qwen3-asr 的默认只有 **128**，
  而本项目窗口 ≤30 字（≈≤15 秒）——512 已有充足余量。放太大只会让失控解码多说废话
  （那才是 `<|endoftext|>` 那类长条目的来源）。`max_total_len`（**输入**上限，同样写 2048）**不要动**：
  60 秒窗口确实需要它，历史上"踩过的坑"多半是它。
- ⚠️ **`[asr]` 里 `align_model` / `max_segment_duration` / `segment_merge_gap`
  是死旋钮**（全仓库没有任何读取方，只有配置 merge 逻辑）：改它们不会有效果，
  字段只为老 toml 不炸而保留，别再往界面加。（`segment_padding` 已从这份名单里**移出** —— 它现在真的在用。）
- SRT 使用 UTF-8 BOM 写入 (Windows 字幕兼容)
- 输出: `video.srt` + `transcript.json`，自动写 `transcript_file` 到 vtrim.toml

### EventDetector
- **分批窗口调用**：每窗口 30 条字幕段、窗口间重叠 5 条（`DETECT_WINDOW_SIZE`/`DETECT_WINDOW_OVERLAP`），
  并发取 `.vtrim/llm.toml` 的 `[llm] max_concurrent`（默认 3，见上「LLM 并发与加速」）；
  每窗口完成即写 `events.checkpoint.json`（断点续传）、
  累计结果写 `events.partial.json`。单窗口失败重试 2 次，仍失败且该窗 > 30 段则拆半递归。
- 弹幕: ASS/SRT 解析 → `danmaku_window_secs`（默认 10s）窗口归一化密度 → 每个 event 的 `popularity` 0~1；
  密度 ≥ 0.5 的窗口按密度降序取 top 20、相邻合并成「弹幕爆发时刻」区间**注入 prompt**（名场面位置）。
- ⚠️ **`danmaku_file` 是相对工程目录存的，读它必须解析**：`vtrim init` 用 `path_to_rel` 写成相对名，
  读取侧一律 `Workspace::resolve`（卡片上的弹幕量、订正用的弹幕都是；Web 侧统一入口是
  `Workspace::danmaku_path`）。管线侧入口是 `PipelineRunner::inject_config`（`pipeline/runner.rs`），
  它按**工程根（`output_dir` 的父目录）→ 视频父目录**的顺序拼绝对路径（两个基准都试一遍，
  否则"视频在 `videos/`、弹幕在工程根"的布局会静默丢掉弹幕）。
  **用户可以在预处理页随时换弹幕**（`POST /api/project-data/assets`，见 WebUI 一节），
  换完要**重新检测**才生效。
  曾经这里只做 `PathBuf::from` → 相对进程 CWD → 弹幕**永远读不到**（日志只有 `弹幕文件不存在` +
  `无弹幕数据，使用字幕密度评分`），于是热度退化成纯字幕密度（归一化到最大值 ⇒ 最密的那条恒为 100%），
  切片也不生成 `clip-NN/clip.dm.srt`。改这里之后要重新检测才会用上弹幕热度。
  Cutter 取弹幕也走同一条解析（`load_danmaku_auto`，ASS / 录播姬 XML / SRT 都认）——
  以前它单独用 `parse_ass`，XML 弹幕会只有热力图、没有 `clip.dm.srt`。
  只读诊断：`VT_PROJECT_DIR=<项目目录> cargo test -p vtrim-core --lib diagnose_danmaku_resolution -- --ignored --nocapture`
- 综合评分: 有弹幕 `popularity = (0.6×弹幕峰值 + 0.4×弹幕均值 + 字幕密度) / 2`；无弹幕时只用字幕密度
  （两个分量都是**相对量**：弹幕按全片最热的窗口归一，字幕密度按全部事件的最大值归一）。
  按 popularity 降序赋 `rank`，最终按时间排序输出 —— `rank` 就是热度名次，也决定 `clips/clip-NN` 目录名。
- 后处理: 丢非法 → 排序 → 区间合并 → **边界吸附到 ASR 段边界** → 合并兜底 → 最短 30s（不足吞并相邻）
  → 重名标题加 `（i/n）`。**不做最长时长切分**（曾按 240s 把长事件等分成 `xxx（1/2）`，用户明确反对：
  一次连续事件被拦腰截断；时长交给 LLM 按话题完整性决定）。prompt 也明令禁止等分。
- 降级: API key 为空时使用单个默认事件（调用失败则报错）
- 输出: `events.json`
- **收拾历史等分产物**：`POST /api/project-data/events/merge-splits` →
  `vtrim_core::detection::merge_split_events`（纯函数，含单测）：把「基名 `（i/n）` + 编号 1..=n 齐全
  + 条数吻合 + 时间首尾相接（1s 容差）」的一组合回一个事件 —— 时间取并集、标题去后缀、摘要取最长、
  热度/置信度取最高、**rank 沿用组内最热的那条**（已剪片段里至少一个目录还对得上）。
  同名但**时间不相邻**的事件不合并（那是给两个不同话题加的区分后缀）。
  ⚠️ **接口保留、UI 已删**（用户要求：prompt 修好之后新检测不再产出 `（i/n）`，
  切片页那颗按钮撤掉了）—— 留着是给**已有**的老 events.json 兜底，用 curl/后续再说：
  合并后事件序号会变，旧片段要用户自己删掉重切。
  只读诊断：`VT_EVENTS_JSON=<项目>/output/events.json cargo test -p vtrim-core --lib diagnose_merge_splits -- --ignored --nocapture`
- ⚠️ **改内建 prompt 要同步用户配置**：`.vtrim/config.toml` 的 `[detection] system_prompt` /
  `prompt_template` 是**写死的副本**（`config init` 时把内建文本填进去了），代码里改默认值对它无效；
  要么在设置页改，要么把两个字段留空（留空才走内建默认）。
  **每个程序目录各有一份**（`target/debug/.vtrim/`、`release/<包名>/.vtrim/` 互不相干）——
  改 prompt 时别只改一个；`'''…'''` 是三引号**字面量**（免转义），正文里没有单引号时可用。

### Cutter
- GPU 加速编码 (NVENC/QSV/AMF 自动检测, 降级 libx264)
- `ffmpeg -ss {start} -i {input} -t {duration} -c:v {auto_encoder} -preset {auto_preset} {qparam} 23 -pix_fmt yuv420p -c:a aac -movflags +faststart {output}`
- 命名: `output/clips/clip-{rank:02}/clip.mp4`（**标题不参与命名**，只写进同目录 `clip.json` 与 `publish_config.json` / `clips_order.txt`）
- `clip.json` = 剪出时冻结的元数据（`title` / **`summary`** / `event_type` / `popularity` / `rank` /
  `original_start` / `original_end` / `duration`）—— 切片页拿它当"这条片段自己的身份"，
  重新检测换掉 events.json 后照样能显示（见 WebUI 一节「切片与事件的对应关系」）
- **不覆盖别人的片段**：目标目录已有 `clip.json` 且与本次事件标题不同、区间 IoU < 0.5 →
  该条剪切报错跳过（`foreign_clip_conflict`，含单测），不让重排后的 rank 撞掉已剪好的成片
- 字幕: 从原始 SRT 裁剪时间范围 + 偏移 → `clip-NN/clip.srt`（弹幕 → `clip-NN/clip.dm.srt`，元数据 → `clip.json`）
- 输出: `output/publish_config.json` + `output/clips/clips_order.txt` (按弹幕热度降序)

## WebUI / Server

- ⚠️ **同步 IO 绝不能留在 async 线程上（剪切时"整个服务器连不上"的根因，2026-09）**：
  项目目录经常在网络盘上（RaiDrive/WebDAV 挂载的录播目录），剪切/渲染把盘占满时，
  一次 `exists()`、一次读 `vtrim.toml` 就要几百毫秒到几秒。这类调用留在 tokio worker 上做，
  十几个并发（前端一次页面加载就会打出一串）就能把 worker 全占住 —— runtime 连 accept
  都排不上，浏览器表现就是"服务器无法建立连接"，而日志里一行都没有（请求根本没处理完）。
  实测样本：`GET /api/projects` 有 2288ms 的记录，日志里 1 秒内 52 次连发。
  - **handler 里的同步文件访问**一律走 `api.rs::blocking(f)`（`spawn_blocking`，512 线程的
    blocking 池），worker 留给 accept 与轻量接口。已覆盖：`projects_list` / `project_list` /
    `project_data_{status,events,clips,polish,transcript,config}` / `clips/:rank/thumb` /
    `clips/:rank`(删除) / `clips/:rank/silence` / `events/:rank/cut` / `events/cut-all` /
    `polish_project_get` / `polish_subtitles_get`。**新增读项目目录的接口照着做**。
  - 需要「async 判定 + 同步 IO」混在一起的实现（如删除片段要先问 `busy_with`）用
    `blocking(move || block_on_here(xxx_impl(...)))` —— 整段搬到 blocking 线程，自带一个
    current-thread runtime。
  - **管线任务体**一律用 `api.rs::spawn_job(jobs, job_id, body)`（专用线程 + current-thread
    runtime）。管线（转写/检测/剪切/精修 process）是整段同步代码，跑在 worker 上就是白占一个到
    它结束（实测 59s 的剪切占 37s，转写是分钟级）。`cut_all` 内部的逐片段剪切走 `spawn_blocking`
    （3 路真并行），进度在 await 侧统一汇报。**入库自动管线 / `models_download` /
    `stickers::spawn_build` 也走 `spawn_job`**。
  - `spawn_job` 带 **panic 兜底**（`catch_unwind` + 替任务调 `fail()`）：任务体的
    `complete/fail` 在它自己末尾，中间 panic 会让 Job 永远 `running`、资源键永不释放 ——
    同项目/同片段的后续任务会全部排队饿死。新增任务入口时把 `(jobs, job_id)` 传进去。
  - 其余同类修复（一并照此维护）：`serve_file` 的元数据、`/api/media` 的归属校验 + `jail_existing`
    （播放会打出一串 Range 请求）、`polish_silence`/`polish_project_put`/`config_*_put`
    （`with_path_lock` 的闭包跑在当前线程，改成 `lock_path().await` + `blocking(...)`）、
    `polish_cover_frame`/`preview` 的截帧 ffmpeg、入库的扫描与跨盘搬运、`POST /api/admin/ingest/scan`。
  - **写 `polish.toml` 一律走路径锁**：编辑器保存、去静音（`clip_silence` / `polish_silence`）、
    封面配置、`cmd_parse` 全在改同一份文件 —— 无锁的 read-modify-write 会静默丢改动。
  - **前端取数要单飞**：`stores/workbench.js::loadProjects()`（`inflight` 去重）与
    `composables/useJobs.js::load()`（在飞合并 + SSE 250ms 合帧）。视图注册 `onJobDone(fn)`
    **必须用返回的退订函数在 `onUnmounted` 里退订** —— 不退订会随反复挂载攒出一串陈旧回调，
    任务结束时同时各打一整套 project-data 请求（就是那次 1 秒 52 连发的放大源）。
    `loadProjectData` 的 `stale` 标记也别删：任务结束撞上在飞请求时，旧响应会盖掉新数据。
- `vtrim server` 只服务 `webui/dist`（Vue3 构建产物），**不再内嵌单文件页面**：
  找不到 `webui/dist/index.html` 时 `/` 返回 **503 + 构建指引**，启动日志也会告警。
  构建：`cd webui && npm install && npm run build`。`webui/dist` **已入库**
  （`webui/.gitignore` 不再忽略它）：改了 `webui/src` 必须重建并一起提交，否则端出旧界面。
- **样式纪律（前端改样式前必读）**：唯一设计标准是
  `docs/web-ui-mockups/sakura-cream.html`（配色/圆角/阴影/字体，v0.2 §4 全量取自它），
  v5 壳与数据型组件的布局数值取自交互基准 `docs/web-ui-mockups/ia-demos/multiuser.html`。
  - `webui/src/styles/tokens.css` = 色板/圆角/阴影/字体变量（**不得另取色**）；
    `webui/src/styles/app.css` = 两份样稿组件类的**并集**（`.shell/.side/.proj/.snode/.foot-item/
    .user-chip/.card/.card-blk/.btn/.mini-btn/.chip/.state-badge/.src-badge/.tb/.tabs/.wiz-src/
    .player/.lane/.grid9/.meter/.login-*/.empty/...`）。
  - **组件内不再重复定义这些基准类**（scoped 样式只写样稿里没有的东西，如行内删除按钮、
    页面专属布局）；新增组件先在样稿里找同名类，找到就照抄数值。
  - 关键数值：侧栏 268px、卡片 `22px 22px 18px`（`.card-lg`）/`16px`（`.card-blk`）、
    页标题 26px、chip `2px 10px`、进度条 6px、贴纸 52px/`rotate(-5deg)`、
    `.cols-main` = `1fr 300px`、主区 padding `18px 22px 60px`。
- **前端信息架构（v5 工作台，`docs/plan/webui-workbench-redesign.md`）**：
  - 壳：左侧栏常驻（品牌 / 项目过滤 / 项目树：名称+来源徽标+进度徽标+可展开的
    预处理·切片·精修节点+占用 / 底部全局入口：首页·入库·设置(admin) + 用户 chip）；
    面包屑 + 内容区；任务抽屉、改密弹窗、Toast/Dialog 挂在壳上；登录页不套壳。
    **侧栏不再有「＋新建项目」与「当前项目」入口**（建项目在首页，导航靠项目树/面包屑）。
    **当前项目整行高亮**（用户要求；判据 `String(p.id) === String(route.params.pid)`，与折叠轨同一个）：
    `.proj.on .proj-head` = `--main-soft` 底色 + 左侧 3px `--main` 强调条 + 名字 `--pink-text`
    （只给底色认不出来 —— `--main-soft`(#FDE3EC) 与悬停底 `--bg-soft`(#FFEFE7) 太接近）。
    ⚠ 选择器**必须比 `.proj-head:hover` 更特指**，否则鼠标一放上去高亮就被悬停底色盖掉；
    这条已用 `CSS.forcePseudoState` 强制 `:hover` 验过。样式写在 `ProjectNode.vue` 的 scoped 里
    （样稿没有这个态），别塞回 app.css。
    **来源徽标只显示类型**（用户要求）：`sourceBadge(p)` 本地来源回 **`本机`**，**不要拼路径** ——
    服务端的 `source.label` 是 `本机 · <父目录>/<目录名>`（`api.rs`，非租户根）或
    `本机文件 · <相对路径>`，而项目名本身就是那个目录名，拼进去等于把名称显示两遍还把徽标撑长。
    完整标签（含路径）挂在徽标的 `title` 上（`sourceLabel(p)`），悬停仍能看到；设置页的「来源」
    仍用完整 `sourceLabel`，两处分工别混。
    ⚠ **`.src-badge` 必须是 `flex:none`**（app.css，样稿本来就是这么写的）：它曾经为了让长目录名
    标签能截断而写成 `flex:0 1 auto; min-width:0; max-width:150px` + 省略号，源标签改成只放类型
    之后这个前提没了，而它仍会被压缩 —— 实测项目名很长时盒宽被挤到 **18px** 而「本机」需要
    **30px**，`.side-list` 又是 `overflow-x:hidden`，于是硬裁掉大半个字（用户报「本机显示了一半」）。
    要让位就让项目名（`.nm`，`flex:1 1 auto` + `min-width:0`，本来就有省略号）去截断。
    同理 `.proj-head .del` 也钉了 `flex:none`（`.mini-btn` 默认可收缩）。**别再改回可收缩。**
    **阶段节点只标两件事**（用户要求）：**当前位置**（`.snode.viewing` = `current && viewing === s.key`）
    与**已完成阶段的打勾**（`.snode.done` 绿底 ✓）。
    ⚠ **打勾只有「预处理」拿得到**（`nodeDone` 里 `key === 'prepare' && …`）：转写 + 事件检测是一次性
    里程碑（`stage >= 3`，转写与 events.json 都落盘），做完就是做完了。**切片与精修没有真正意义上的
    "完成"**（用户原话）—— 重新检测随时冒出未剪切事件、随时能补切一条、精修可以反复重新导出，
    而 `stageInfo().done` 里那两档（`slices` = 全切完且无未剪切、`polish` = 全部出片）只是**瞬时状态**，
    随时翻回去，拿来当勾选语义就是在宣称一个不存在的终态（实测 `stage >= 4` 时旧口径会给「切片」打勾）。
    这两条的状态交给进度 chip（已切 N · 待切 M / 精修 N/M）与运行中的百分比表达。
    ⚠ 原来还有一档 `cur`（`progress === i + 1`，走到的最远阶段）的粉底高亮，**已按用户要求删除**
    （app.css 里那条和 `.snode.cur .ic` 一起删了）。样稿 `ia-demos/multiuser.html` 里有 `.snode.cur`，
    那是**有意弃用**的 —— 别因为"样稿里有"就捡回来。圆点现在**只表完成度**（不再有粉色渐变点），
    "我在哪"由底色表达，两个信号不打架。
    ⚠ `viewing` **必须再 `&& current`**：`SideNav` 对**每个** ProjectNode 都传同一个 `viewingStep`
    （它只从 `route.name` 来，不带项目），不门控的话多个项目同时展开时同名阶段会**一起高亮**
    （实测修复前 `viewing` 命中 2 处）。位置高亮 = 选中的项目 + 该项目的阶段，两个条件缺一不可。
    `.snode.viewing` 的底色用 `--main-soft` + `--pink-text`（与当前项目行同款），scoped 规则带
    `[data-v]`（(0,3,0)）天然压过 app.css 的 `.snode:hover`（(0,2,0)），悬停盖不掉。
    **侧栏可向左折叠**（用户要求）：品牌行右侧的折叠按钮 → `.side.mini`（app.css，
    `width:64px` + `transition:width .18s`）。折叠态 = 品牌 V + 折叠按钮（竖排）+
    项目首字圆头像（`title` 名称·当前阶段，点击直达该项目当前阶段，运行中红点）+
    底部三图标入口 + 用户头像；展开态保留过滤框与完整项目树。状态在 `stores/ui.js`
    的 `ui.sideCollapsed` / `toggleSide()`，localStorage `vtrim.ui.sideCollapsed`，
    **模块初始化时同步读取**（放 computed 里读会在首帧闪一下展开态）。
    折叠轨的类（`.rail-*`/`.side-toggle`/`.rdot`）是样稿没有的新态，写在 SideNav 的 scoped 样式里。
  - 侧栏底部：**用户块只做展示**（点它不再登出，用户要求），登出是块内的「退出」按钮
    （折叠轨 = 头像 + logout 图标上下两行）；`doLogout()` 里先 `confirmDialog` 二次确认。
    **全局入口只剩 首页 / 设置**（「入库」入口已按用户要求删除；`#/ingest` 仍可从首页
    「录播姬录制」块的「全部记录与导入规则」进）。
  - 首页 `#/workbench` = **「新增项目」区块**（先选来源，再切换成对应的导入方式）+
    「录制导入（最近）」；**没有项目列表**（原先的项目表已移除）：
    - 服务器文件（**仅 admin；非 admin 整条不显示** —— 不给一个点不动、只写着"仅管理员"
      的入口，用户要求）：首页内嵌目录浏览器（任意绝对路径 + 快捷入口 + 上一级），
      选一个视频 → 新建项目。`sources` 按角色过滤，`source` 用 watch 保证始终落在"看得见且
      可用"的那一项（非 admin 默认「录播姬录制」）；B 站链接的不可用文案也按角色分流
      （admin 提示可用「服务器文件」，非 admin 提示等管理员指派/配录播姬）；
    - 录播姬录制：**用户只看到「配置直播间」**（一个输入框 + 我自己登记的直播间列表：
      `录制中/在播/未开播/未在录播姬` chip + 删除），**看不到**「全部记录与导入规则」「录播姬尚未接入
      …」「扫描录制目录」与待分配池 —— 那些是管理员向的，全在 `<template v-if="isAdmin">` 里。
      录播姬不可用（未配置 `api_base` 或连不上）时整块**置灰**且输入禁用，只给一句"录播姬未接入，
      暂时不能添加"，不给管理配置指引（用户要求）；`roomsAvailable` 初值 `null` = 还没问到，
      避免先闪一下"未接入"。管理员额外看到连接状态/测试连接/扫描/待分配池。
      实现：`GET/POST /api/rooms` + `DELETE /api/rooms/:room_id` → `service::recorder::RecorderClient`
      （录播姬 `POST /api/room {roomId, autoRecord:true}`，Basic 认证，见
      `docs/plan/recorder-room-flow.md`）→ 写 `channel_map`（`source='user'`）；录完 webhook 按
      `channel_map` 归属自动入库成他的项目。配置键 `[ingest.recorder] api_base/api_user/api_pass`
      （空 = 未对接）；admin 自检 `POST /api/admin/recorder/test`；
    - B 站链接：P4 未实现，如实标注不可用。
  - 首页「新增项目」下面的 **「表情包」区块**（`components/StickerPacks.vue`，
    **仅 admin；非 admin 整块不渲染** —— 与「服务器文件」来源同规矩）：
    - 库固定在 `vtrim_core::path::sticker_dir()` = **`<程序目录>/stickers/`**（不是项目里那份
      拷贝）：一个子文件夹 = 一个包，`<包名>.toml` = LLM 压出来的 tag 索引，
      `sticker.toml` = 已构建包的注册表。区块里显示实际路径 ——
      **debug 与 release 实例各有一份库，互不相干**（改包别只改一个）。
    - 列表**以磁盘为准**（`scan_packages`）：手工把文件夹丢进 `stickers/` 也会出现。
      「未构建」= 没有 `<包名>.toml`：图能看，但 LLM 生成事件时读不到这个包
      （`local_sticker_candidates_text` 只列已构建的，`fill_default_local_packages`
      也只认 `sticker.toml` 里注册过的）。
    - 「＋ 添加表情包」= **上传 zip 压缩包**（用户要求：比让管理员去服务器上找目录方便）：
      弹窗里一个可点可拖的上传区 + 包名（默认取 zip 文件名、非法字符换成 `_`，可改）→
      `POST /api/admin/stickers/upload?name=<包名>&build=<0|1>`，**请求体就是压缩包的原始字节**
      （`application/octet-stream`，不是 multipart —— 单文件上传用 multipart 只是多一层边界解析，
      而 `client.js` 的 `api()` 本来就能直接传 `File`，只要求调用方把 `Content-Type` 覆盖掉）。
      ⚠️ **这是全站第二处有意接收用户上传的入口**（第一处是封面背景图 base64）；
      「服务器文件建项目」那条仍然**永不接受上传**。别按旧规矩（"全站没有 multipart、不接受上传"）
      把这条删掉 —— 它是有意做的，闸门在下面。
    - 上传的处理在 `vtrim_polish::assets::sticker_zip`（贴纸库逻辑与 `import_package` 同处）：
      **只取条目 basename**（`../../x.gif`、`/abs/x.gif`、`..\x.gif` 因此都只会落到
      `stickers/<包名>/x.gif`，zip-slip 不需要额外的 containment 校验）；跳过目录条目、
      `__MACOSX/` 与 `.` 开头的条目（苹果伴随文件叫 `._x.gif`，**它有 .gif 后缀**，只按扩展名过滤会收进库里）、
      非图片（判据是 `sticker_index::is_image_file_name`，**唯一一份**，别在各处再抄一遍扩展名清单）；
      子目录**摊平**，摊平后重名**只留第一个**、其余计入 `skipped`。
      **先解到暂存目录 `stickers/.upload-<pid>-<rand>/`，成功后 `fs::rename` 成 `<包名>/`**
      —— 任何失败路径都 `remove_dir_all(暂存)`，所以**不存在半个包**；暂存名以 `.` 开头，
      而 `scan_packages` 只认非 `.` 开头的目录、`sanitize_pack_name` 也拒绝 `.` 开头，
      所以它既不出现在列表里也访问不到。
      闸门：请求体 64 MiB（路由上的 `DefaultBodyLimit`，只给这一条放开，axum 默认才 2 MB）+
      `ZipLimits` 的条目数 2000 / 单文件 32 MiB / 解压总量 256 MiB；单文件那层按**实际写入字节数**
      判（`take(limit+1)` 后比对），所以中央目录里谎报尺寸没用。
      ⚠️ 前端 `webui/src/api/stickers.js::UPLOAD_MAX_BYTES` 与 Rust `UPLOAD_MAX_BYTES`
      是同一个数（改一处要改两处），前端必须先挡：axum 超限回的是 **413 + 纯文本**，
      而 `client.js` 只认 JSON，会把"文件太大"报成"接口不存在或返回格式错误"。
      上传**没有进度条**（`fetch` 拿不到上传进度），只显示「上传中…」。
      zip 依赖钉在 `zip = "2.4.2"`（`default-features = false, features = ["deflate"]`）：
      该版本连同全部传递依赖都在本机 cargo 缓存里，**4.6.1 的 deflate 走 `flate2/zlib-rs`，那份没缓存**。
    - 目录导入**接口保留但没有 UI**：`POST /api/admin/stickers/import {dir,name,build}`
      （`import_package`：把服务器上一个目录里的图片**复制**进 `stickers/<包名>/`，来源目录原样留着）
      —— 留给「服务器上已经有一堆图」时用 curl 导入。两条入口的共同规矩：目标已存在一律拒绝、
      绝不合并；一张图都没进来（复制失败 / 压缩包里没有图片）就报 400 且不留目录。
    - 「LLM 压缩构建」= `vtrim_polish::commands::build_package_tags`（CLI `polish stickers build`
      的同一份实现）：读文件名产出 10~20 个语义 tag → 写 `<包名>.toml` → 注册进 `sticker.toml`。
      走 JobManager（kind `sticker_build`，占 `stickers` 资源键：另一个构建在跑时**排队**），
      进度经 `api::core::progress_bridge` 上报，取消在**批次边界**生效。
      上传时勾了「上传后立刻构建」就顺带起它；**导入成功与构建任务是两件事** ——
      构建起不来只回一句 `job_error`，上传照样算成功。
    - 展开一个包 = **卡片墙 + 点开看大图**（用户要求：卡片比表格行装得下更多内容）：
      `.stk-grid` = `repeat(auto-fill,minmax(112px,1fr))`，一张卡 = 方形图台（`aspect-ratio:1`、
      `object-fit:contain`、占满卡宽）+ 文件名（单行省略，完整信息在 `title`）+ 体积 +
      **该文件的 tag 全部换行显示（不裁切）**；点卡片 → `AppModal wide` 大图预览
      （`.stk-preview` `max-height:60vh`）。容器 `.pack-files` 框内滚动、上限 **420px**
      （167 张不撑长首页）；图挂了/没加载出来时露 `Icon image` 占位（垫在 `<img>` 下面，
      `@error` 只把那张图 `display:none`）。**`preview` 只存 `{pack, name}`**，文件对象由
      `previewFile` 从 `detail` 现取 —— 别存文件对象快照，构建完 / 重载后 tag 变了预览要跟着变。
      别退回「缩略图 40px | 文件名 | tags」那张表格（看不出是哪个表情）。
    - 接口（全部 admin）：`GET /api/admin/stickers`（列表）/ `GET .../:pack`（包内文件 + 每个文件的
      tags，展开时才拉）/ `POST .../upload?name=&build=`（**上传 zip**，见上）/
      `POST .../import {dir,name,build}`（服务器目录导入，无 UI） / `POST .../:pack/build` /
      `DELETE .../:pack`（目录 + 清单 + 索引项一起删）/ `GET .../:pack/files/:name`（预览图，
      与 `/api/media/*` 共用 `media::serve_file` 的 Range 实现）。
      包名过 `sanitize_pack_name`、文件名过 `is_safe_component`，再 canonicalize 要求落在库内。
  - 项目级操作在 `#/p/:pid/settings`（项目设置页也显示名称/来源/阶段/占用/源大小/产物，
    以及三个阶段的打开入口）：**「删除源录播」**（真删媒体，唯一的手动释放空间手段）与
    **「移除项目」**（只注销登记、零文件改动）。两个入口——设置页按钮与侧栏项目行的 `×`
    （`ProjectNode` → `SideNav` → `App.vue::removeProject`）——文案与行为必须一致：
    **都不再是 `danger` 红色**（移除已不是破坏性操作），但仍然二次确认（误点一下项目就从列表里没了）。
    移除后前端要调 `stores/workbench.js::forgetProject(pid)` 清本地痕迹
    （per-pid 缓存 / `vtrim.wb.expanded` / `vtrim.wb.last` / `vtrim.polish.last.<pid>`）——
    项目 id 是 `AUTOINCREMENT` 不会复用，所以那些残留不会串号，但会永远躺在本地存储里。
  - 路由：`#/workbench`（首页：新增项目 + 录制导入最近）、`#/p/:pid/{prepare,slices,polish,polish/:clip,settings}`、
    `#/ingest`、`#/settings`（仅 admin）、`#/login`；`/` 按 localStorage 回上次项目+阶段；
    **旧路径 `/project/:cfg/**`、`/polish`、`/jobs`、`/models`、`/projects` 全部 redirect**。
  - **懒加载 chunk 失效会自动刷一次**（`router.onError` + `sessionStorage` 闸门）：dist 每次构建
    换 hash 名且清空 outDir，浏览器里跑着旧 index 时点**还没加载过的**路由 → `import()` 取到
    已被删的旧 chunk → 404 → vue-router 中止导航，表现为「点哪个按钮都不跳转，必须手动刷新」。
    检测到动态 import 失败就 `location.reload()` 拿新构建；刷新后仍失败则不再刷（防死循环），
    任意一次导航成功即清闸门（下次发版还能自动刷）。**这是发版/重建 dist 后的正常现象，不是死锁。**
  - 作用域单点：`api/scope.js` 的 `scopeQ(pid)`；业务调用只在 `api/projectData.js` /
    `api/polish.js` / `api/jobs.js` 三处拼查询串，视图/组件不自己拼。
  - ⚠️ **换项目必须让视图重挂**：`App.vue` 的主 `<router-view :key="route.params.pid || route.name" />`。
    `/p/:pid/{prepare,slices,polish,settings}` 里**每个步骤是同一条 route record**，同一步骤下换项目
    只有 `:pid` 变 —— vue-router 会**复用组件实例**，`setup`/`onMounted` 不再执行，而这四个视图都是在
    那时抓一次 pid（`ProjectSlicesView`、`PolishView`（连 `routeCfg` 都是字符串常量）、`ProjectSettingsView`
    直接冻结；`PrepareView` 的 pid 虽是 computed，`load()` 也只在 `onMounted` 跑）。表现就是
    「切了项目只有标题变、页面还全是上一个项目的」（用户实测的关键线索：**跨步骤切换正常、同步骤不正常**）；
    预处理页更危险 —— 显示的是旧数据，`saveTranscript(pid.value, …)` 却用新 pid，把 A 的字幕写进 B。
    key 用 **pid** 而不是 `fullPath`：后者会让精修页换片段也整块重挂，丢掉原地切换。
  - `stores/workbench.js`：项目列表 + 侧栏展开/过滤/上次位置 + per-pid 数据缓存与
    `invalidate(...)`；`composables/useJobs.js` 的 `onJobDone` 驱动失效矩阵
    （任务结束 → 缓存失效 + 徽标刷新）。任务带 `project` 字段，侧栏显示 "预处理 42%"。
  - **预处理 = 一个按钮、两段任务**（用户要求合并转写 + 事件检测；`PrepareView`）：
    「预处理」卡只有一个主按钮，**按阶段推进** —— 没转写 → 起 `stage_transcribe`，转写成功后
    **自动接 `stage_detect`**（两次 `POST /api/project-data/pipeline`，两个 job；转写失败就不往下走，
    已完成的转写留着不用重跑）；已有转写 → 直接起检测。所以后一段失败"再点一次"即可。
    ⚠️ **按钮只受 `busy` 限制，不再"跑完就永久置灰"**（2026-09 用户改口，旧口径"没有重新预处理入口/
    跑完就完成"作废）：两段都就绪时它变成**「重新检测」**，点击走 `confirmDialog(redetectWarning())`
    二次确认，只重跑事件检测（切了弹幕/字幕后要生效就是点它）。**不做全流程重跑**。
    ⚠️ **转写一生只跑一次**（用户要求"转写应该有所绑定"）：只要工程里已经有转写 —— ASR 产物或
    用户选的外部字幕 —— 前端永远不再起转写，也没有「重新转写」入口；换字幕只有一条路：
    「字幕」那一行的「选择…」。服务端同一条不变量在 `project_data_pipeline` 里守着
    （`stage == "transcribe"` 且 `ws.transcript_path()` 有值 → **409**），
    但**只拦 HTTP 这一侧**：CLI `vtrim transcribe` 不受影响，是转写崩在中途（只留半份产物）时的逃生口。
    chip 只剩「转写 / 事件检测」两枚（转写 ✓ 以**字幕**文件为准，光有 wav 不算）；
    **ASR 模式不在这个页面选**（用户要求：固定 hybrid，只在设置里改）：没有下拉、也不读
    项目 `config`（`GET /api/project-data/config` 的 `asr` 不再被这页请求），
    代码默认 `default_asr_mode() = "hybrid"`（`config/types.rs`）与全局模板
    `GLOBAL_CONFIG_TOML` 的 `[asr] mode` 都写 hybrid；项目 `vtrim.toml` 的 `[asr] mode`
    仍由设置页（项目配置表单/源码）改，merge 时项目覆盖全局。
    **LLM 订正入口已从前端撤下**（用户要求"暂时删除"）：后端 `/api/project-data/correct` 与
    `vtrim correct` 都还在，`api/projectData.js` 里的 `correct()` 也留着，后续要恢复只需补 UI；
    设置页 `[asr] llm_correct` 勾选项仍在（它作用于 `process` 全流程，不是这个按钮）。
  - **输入素材（弹幕 / 字幕）随时可换**（2026-09，用户要求"预处理时可以选择确定字幕文件和弹幕文件…
    都可以进行随时切换"）。「预处理」卡 chip 下面就是两行素材，值存在 `vtrim.toml` 的
    `[project] danmaku_file` / `transcript_file`（**相对工程根**，工程外的绝对路径仅 admin）。
    - 行状态：`已载入` / `未设置` / `文件不存在`（配置写了但文件不在**照实报"不存在"**，
      不要静默飘到 `output/` 兜底上 —— 管线确实会退到 `output/`，但用户选的这份没了必须让他知道）；
      字幕行在生效值来自外部文件时多一枚 **「外部字幕」** chip（判据 `external`：不在 `output/` 里）。
      动作 = 「选择…」+（有值时）「清除」（二次确认）。**「清除」= 回落到自动**（弹幕无；字幕落到
      `output/` 里的转写产物），不是删文件。
    - 选择弹窗：候选来自**服务端扫的项目目录**（见下），admin 额外内嵌 `ServerDirPicker`（自由浏览
      服务器任意目录，`exts` prop 把不相干的文件置灰标「不适用」）。候选**一次扫完**（`GET` 就带回来的），
      弹窗打开不再多打一轮。
    - **改完不自动跑任务**：只把提示切成「素材已切换 —— 点下面的按钮重新检测才会生效」。
      LLM 调用有成本，**别改成保存即起任务**。
    - 接口：`GET /api/project-data/assets` → `{danmaku, transcript, source, candidates, root_dir}`（只
      stat + 扫目录，**不解析**文件内容：3 小时录播的弹幕 XML 有几十 MB）；
      `POST /api/project-data/assets` `{kind, path}` 或 `{kind, dir, name}` → 写前**解析一遍**
      （解析失败一律不落盘），成功回 `{value, count, status}`，`count` 就是界面上的「弹幕 3 条」。
      ⚠️ **`{dir, name}` 这条形态是为了不让前端拼路径**：`/api/fs/list` 回传的目录可能带 `\\?\` verbatim
      前缀，前端拼成 `\\?\F:\a/b` 在 Windows 上是 ERROR_INVALID_NAME（首页目录选择器同款教训）。
      实现全在 `service/assets.rs`（`classify` / `asset_status` / `scan_candidates` / `set_asset`，
      含单测），路由与 handler 只是薄壳（`blocking` + `lock_path(vtrim.toml)`，与设置页抢同一把锁）。
    - **候选扫描范围 = 项目根 + 一层子目录**，跳过 `output`/`clips`/`.vtrim`/`stickers`/`assets` 与点开头
      目录；扩展名分类一张表：`xml`/`ass` → 弹幕、`json` → 字幕、**`srt` 两边都列**（从名字分不出是
      弹幕还是字幕，交给用户）；排序 = 同名（与视频同 stem）优先 → mtime 倒序。
    - **按来源类型预载**（`assets::preload_policy`）：`recorder` → 弹幕（入库时 `find_danmaku` 已搬入
      并写进 `vtrim.toml`，这条早在跑）；`local`（服务器文件）→ **不预载**，只列候选让用户点
      —— 为此 `create_local_project` 改调 `write_project_config_exact`（不做"同目录同名 .ass 自动发现"，
      那条是 `vtrim init` 的语义，两条路要分开）；`bilibili` → 字幕（"有字幕就自动载入"），
      **该来源仍是 P4 未实现，这个分支目前没有调用方**，B站下载落地时照 `ingest_worker::find_danmaku`
      的同名探测填 `transcript_file` 即可。
    - ⚠️ **转写解析只有一个入口**：`Workspace::transcript_path()`（`transcript_file` → `output/transcript.json`
      → `output/video.srt`）与 `Workspace::danmaku_path()`（`danmaku_file` → 视频同目录 `danmu.ass`/
      `danmaku.ass`）。**一律相对工程根**。以前 `data_transcript_impl` 用裸 `PathBuf::from(...).exists()`
      （相对**进程 CWD**）而 `transcript_starts` 用 `ws.resolve`，靠 `output/` 兜底才没暴露 ——
      指向外部字幕时就是"选了不生效"。新增读取方走这两个方法，别再各写一份。
    - ⚠️ **detect 阶段必须认 `transcript_file`**（`core_pipeline::run_stage("detect")` 的取字幕顺序：
      `-t` 覆盖 → `transcript_path` → 报错）。只认 `output/` 的话"选了外部字幕后重新检测"读到的还是
      旧的那份。`run_process` 的 recovery 分支与它共用同一个方法。
    - ⚠️ **字幕保存写回"实际读到的那一份"**（`data_transcript_put_impl`）：目标是 `transcript_path`，
      再同步**已存在**的 `output/transcript.json` / `output/video.srt`。只写 `output/` 的话，
      选了外部字幕再编辑就是**静默丢失**（下次读的还是原文件）。`output/` 两份都不存在、
      又没有 `transcript_file` 时才 404。
    - ⚠️ **`vtrim-core` 里两个基准问题一并修了**：`ProjectConfig::current_stage` 与
      `runner::inject_config` 的 `*_file` 存在性/相对解析都改成先按 `output_dir.parent()`（= 工程根）
      —— 前者让"只上传字幕、从不跑 ASR"的工程能被判成 stage 2（否则重跑/门槛判断全跟着错），
      后者让"视频在 `videos/` 子目录、弹幕在工程根"这种布局不再静默丢掉弹幕（仍有视频父目录兜底）。
      `Path::new("").join(x) == x`，所以 CLI 的 `./output` 相对口径行为不变。
      两条都有回归测试（`current_stage_tests` / `runner::tests::danmaku_in_project_root_resolves_when_video_is_nested`）。
    - 顺带修掉：LLM 订正取弹幕用的是 ASS-only 的 `load_danmaku`，选中的 `.xml`/`.srt` 弹幕等于没有 ——
      改走 `load_danmaku_auto` + 共享的 `ws.danmaku_path`。
  - 预处理页 = 播放器 + **滚动式字幕轨道** + 切片标记行 + 预处理动作卡：
    轨道是**精修时间轴同款的像素刻度滚动视图**（左侧 sticky 标签列 + 横向滚动；缩放 0.5~120 px/s，
    进入时按视口自动选档，「－ ＋ 全览」换档且以视口中心为锚点；标尺点击/拖动跳转；播放头跟随，
    用户滚过 2 秒后恢复跟随）。**拖动/点击标尺只改时间，绝不搬窗口**（用户要求"拖动指针不移动窗口、
    只在这个范围变换，只有下方滚动条可以移动窗口"）：`seekAt()` 把像素换算出的时间**先夹进当前
    可见区间**再夹到轨道范围；`onRulerPointerDown` 的每次 `pointermove` 都刷新
    `markUserScroll()`（"用户接管视口"的 2 秒窗口），所以**整段拖动期间**自动跟随都不会抢。
    想看别处就用下方横向滚动条滚过去 —— 别去掉这两个夹取，也别把 follow 的判据改回只看滚动。
    块按 `start × scale` 绝对定位，**只渲染视口 ±2 秒内的块**
    （3 小时几千条也不卡；刻度同样只画可见区）。**「字幕属性」是常驻横排条，贴在时间轴上方**
    （用户要求：不要右侧竖面板；未选中时字段禁用 + 一句提示，位置固定不跳、不占轨道宽度，
    窄屏自动换行）—— 点块选中后就地编辑，**操作与精修预览的字幕属性同款**：字幕内容 / 开始·结束秒数 /
    「当前设为入点」「当前设为出点」/「按当前时间拆分」/「删除」，外加「＋」在当前位置新增一条；
    拖动平移、拖左右缘改起止、双击删除也保留（拖拽结束那一次 click 用一次性标志吞掉，
    不用时间窗，否则紧接着点别的块会选不中）。改动即时写 `PUT /api/project-data/transcript`
    （父组件 500ms 防抖；卡片头显示「保存中… / 已保存 HH:MM:SS / 保存失败」，不再每次 toast 刷屏）。
    `markIn/markOut` 在轨道上高亮成标记带；轨道总长 = `max(视频元数据时长, 最后一条字幕结束)`；
    **切片块**（用户要求整合：`标记入点 / 读数 / 标记出点 / 读数 / 片段标题 / 切片 / 更多设置`
    全在**右列「预处理」卡下方**那一块里，`grid cols-main` 右列 = 预处理 + 切片两张卡，
    `.side-col` 包着、`mt-12` 相接 —— 别再把它拆回左列最下面，也别在预处理卡里另放一个
    「手动添加片段」入口，那是同一个表单）：标题已填 →
    「切片」一次点击直接 `POST events` + `cutOne` 并跳切片页（标题是唯一必填，**详情可选**）；
    标题留空 → 打开表单（类型/详情）。标题是**纯人工查看的元数据**（只进 `events.json` /
    `clip.json` / `publish_config.json` 与卡片展示），**不影响目录与文件名**
    （固定 `clips/clip-NN/clip.mp4`，见 `vtrim-core/src/cutting/mod.rs`）。
    时间读数用 `.fld-input` + `.slice-time`：**`flex-basis` 必须写 0**（全局 `input{width:100%}`
    会让基线占满整行，flex-wrap 下按钮被挤到上一行、两块各占一行）；
    精修页 = **片段小卡条**（`FragmentList`：108px 小卡 = 缩略图 + 状态 chip 贴角 + 一行标题，
    默认 1 行横向滚，「展开」按钮切 2 行；`ui.fragRows` 持久化 `vtrim.ui.fragRows`）→
    ⚠️ 两个踩过的坑：①**顺序必须按数字序号**（`clip-100` 在 `clip-02` 之后）——
    服务端 `data_clips_impl` 与 `PolishView.loadClips` 都按 `rankOfName()` 排过，别退回字符串排序
    （会得到 clip-01, clip-02, clip-100, clip-101, …, clip-11，卡片上的序号也跟着错）；②`.frag-cards`
    是 `grid-auto-flow:column`，**必须写 `justify-content:start` + `grid-auto-columns:max-content`** ——
    缺省 `normal`(=stretch) 会把 auto 宽的列撑满整行，只有两张卡时它们被推到两端（用户原话
    "只有两个的时候分的也太开了"）。
    `grid cols-main edit-grid` 左列 = 预览（属性面板在左、舞台在右）、
    右栏 = 「生成与导出」卡（LLM 剪辑参数块〔区域编辑 + 贴纸来源〕→ 渲染导出）→
    **时间轴编辑器独占一整行**（v6.4，用户要求：`grid` **之后**的 `.card.tl-card.mt-16`，
    不再挤在左列里）→ 封面设计器 → 底部产物整行；切片页 = 操作条 + **一面卡片墙**
    （已剪切片段 + 未剪切事件混排，靠筛选切换；抽屉与「放大视图」已删）。操作条 =
    重新检测 / 添加片段 / **一键全切（N）**（有未剪切事件时才出现）。
    **导出产物同名不覆盖：加 `_1` / `_2`**（用户要求"如果已经存在导出文件，就加 _n，类似封面的操作"）：
    `render::unique_output_path`（`render/pipeline.rs`，**成片与封面共用**，有单测）在目标已存在时
    追加 `_1` / `_2`…（`Ffmpeg::mux` 带 `-y`，同名即覆盖 —— 别去掉这层后缀）；
    服务端 `/api/project-data/polish` 用 `latest_numbered_file` 把**编号最大**的那份回给前端
    （`rendered_file` / `cover_file`；切片页卡片的「已导出」chip 取自这里的 `rendered` / `cover`）。
    ⚠️ 而「产物」栏以前显示的名字是写死的（`renderedName`/`coverName` 两个 ref 从没被赋过值，
    恒为 `export.mp4` / `cover.jpg`），用户根本看不出产物其实叫 `vtuber-clip_1.mp4`，会以为被覆盖了 ——
    现在直接取 `clipEntry.rendered_file` / `cover_file`，**别再退回写死的名字**（harness 有 8 项断言）。
    「渲染导出」弹窗（PolishView）与切片页卡片的 `promptDialog` 也要写明"同名不覆盖、自动 `_1`"；
    服务端渲染/封面**成功**时再从子进程 stdout 里捞 `FINISHED:` / `Cover saved:` 那行
    （`subprocess_note`，含单测）记进任务日志 —— 成功分支原本把 stdout 整个丢掉，日志里看不到实际产物名。
    **切片卡片可删除**（用户要求）：`DELETE /api/project-data/clips/:rank`，前端先
    `confirmDialog(..., {danger:true})` 二次确认，按钮在卡片右下角（`.clip-del` 推 `margin-left:auto`，
    别贴着「精修」）。服务端删掉整个 `clips/<目录名>/`（精修产物/成片/封面一并没），
    并把引用从 `publish_config.json` / `clips_order.txt` 里摘掉（`prune_clip_from_index`，有单测）；
    **`events.json` 不动** —— 目录没了事件在切片页自动回到「未剪切」，可以重切。
    路径参数过 `resolve_clip_dir`：**目录名优先**（`clip-out` 这类手工命名的目录也要能删 ——
    卡片墙列的就是 `clips/` 下的真实目录名），没有同名目录时才把纯数字当老契约的 rank（`2` → `clip-02`）；
    名字先过 `is_safe_component`（`..` / 分隔符 / 盘符 → 400），拼出来只可能落在 `clips/` 内。
    缩略图接口（`clips/:rank/thumb`）走同一个解析 —— 前端一律传 `c.name`。
    ⚠️ **删除不是任务（不发 job），刷新只能由前端自己驱动**：`ProjectClipsSection` 的数据是父页面
    （`ProjectSlicesView`）传下来的**快照**，它自己的 `load()` 在 props 模式下只是照抄那份旧快照 ——
    删完不 `emit('changed')` 让父页面 `load({silent:true})` 重新取数，卡片就留在墙上，用户再点一次
    就是 404「clip-XX 不存在」（用户报过「删除会报错找不到内容」；日志现场 = 一次 200 紧接三次 404）。
    同一条规矩适用于改事件（`saveEdit`）与新建片段（`addClip`）。`silent` 必须真的静默（别置 `loading`，
    否则整面墙换骨架屏、子组件被卸载）；删除中按钮禁用（`deletingDirs`）防连点；404 当「已经不在了」
    提示并刷新，而不是报删除失败。
    切片页**「弹幕量」排序**依赖 `/api/project-data/clips` 的 `danmaku_count`：有 `clip.dm.srt`
    就用它，**没有（早于该文件的老切片）就按 `clip.json` 的 `original_start/end` 从源弹幕里数**
    （`count_in_range`，源弹幕按需解析一次）—— 否则整列 0，排序静默退化成按 rank（用户报过"没生效"）。
    **「重新检测」在已有检测结果时必须二次确认**（`utils.js` 的 `redetectWarning()`，切片页
    「重新检测」与预处理页那条检测路径（按钮 = 开始预处理 / 开始事件检测）共用同一份文案）：重跑只整份覆盖 `output/events.json`
    （手动改过的标题/时间也一起没），**`output/clips/` 一个文件都不动**；但事件按热度重新编号后
    目录名 `clip-NN`（= rank）与新事件会对不上 —— 文案里要提醒"需要时删掉重切"，别写成"会删片段"。
    **未剪切事件与已剪切片段同在一面卡片墙**（用户要求合并）：没有抽屉、没有「放大视图」，
    未剪切事件就是同款 `.pcard`（`.uncut-card` 虚线顶边 + 一枚「未剪切」chip，没有缩略图，
    动作 = 「编辑 / 剪切」）。
    **墙面是一个列表、按同一个排序键混排**（`shownItems`：`{kind:'clip'|'event'}` 合成一个数组再排）——
    「热度」是**数值**（`popularity` 0~1，见 detection 的评分公式），**不是 rank**：
    rank 只是热度排出来的名次、重新检测会整体重排，旧片段只能靠 `clip.json` 里冻结的 `popularity`
    参与排序（用户要求）。已剪片段的四个键都取"属于自己的那条事件"，认不出事件就退回 clip.json 的冻结值
    （`clipStats`）；未剪切事件用 events.json（`eventStats`）。同值时**进过精修的**优先（`polishReady`）、再按序号兜底。
    ⚠️ `readSort()` 的回落必须用 **`SORT_OPTIONS[0]`**：以前写死 `key:'rank'`，换成 popularity 之后
    旧偏好失配 → 排序栏一个都不高亮、还按不存在的键静默排序（实测踩过）。
    筛选顺序固定 **已剪切 / 未剪切 / 精修中 / 全部（最后）**；
    默认勾选见 `applyDefaultFilter()`：**有已剪切就选「已剪切」，一个都没有就选「未剪切」**，
    且默认值**不写盘**（只记用户手点过的，避免"全是未剪切"时期的默认在剪完之后过期）。
    ⚠️ 筛选里的「精修中」只管已剪片段，含义是**进过精修的都算**（`isInPolish` =
    `polish_ready || rendered || cover`），**已导出的也在内** —— 用户要求把原来那档
    「已精修＝已出片」合并进来，筛选栏不再为"已导出"单开一档（想只看已导出的看卡片 chip）。
    key 因此从 `unpolished` 换成了 **`polish`**：含义变了，沿用旧 key 会让 localStorage 里
    存过的偏好静默指向另一批片段，靠启动时的 `FILTERS.some(f => f.key === v)` 挡掉、回落默认档
    （与 `readSort()` 那条 `SORT_OPTIONS[0]` 是同一类坑）。
    **卡片上的导出状态只剩一枚 chip：`已导出`**（用户要求：「已精修」+「有封面」合并），
    判据 `isExported` = **`rendered || cover`**（出过片或出过封面都算 —— 两者都是"精修产物已落盘"，
    分成两枚会让同一张卡经常并排两枚、把小差别放大）。进过精修但还没导出 → 「精修中」；
    从没进过精修 → 不挂 chip。**全应用统一用「已导出」**：精修页 `clipRow` 的三态是
    未初始化 / 精修中 / 已导出（同一判据），别在任何地方再写「已精修」——
    「已精修 / 精修中」并列正是用户报过的歧义。排序兜底的 `polishReady` 与侧栏
    `polishedCountOf`（只数 `rendered`，不含只有封面的）是另外两个口径，别与 chip 混用。
    批量动作 `一键全切（N）` 在**页面操作条**上（`ProjectSlicesView` 调 `cutAll`，抽屉没了之后
    不能再放在列表头上）。
    **切片与事件的对应关系不能只看目录名（rank）**（用户要求：已剪好的切片要独立于新的事件）：
    `rank` 是检测按热度排的，**重新检测会整体重排** —— 只看目录名会让新事件"继承"老片段的标题、
    老片段被套上新名字。判据（前端 `utils.js` `clipMatchesEvent` = Rust `cutting::foreign_clip_conflict`）：
    目录名相同 **且（标题相同 或 原片区间 IoU ≥ 0.5）**；老 `clip.json` 缺字段时退回只按目录名。
    - 已剪卡片**先认自己的 `clip.json`**（title/summary/event_type/original_start/end/popularity
      都是剪的时候冻结的），只有"认得出事件"时才跟着事件走（`clipView()`：有 `ev` 才显示「编辑」）；
      认不出事件的片段挂一枚 **「独立」** chip，卡片时间/热度也来自 clip.json。
    - 未剪切事件 = **没有任何"属于自己的"片段**的事件（`uncutEventsOf`）；序号被别人的旧片段占着
      也算未剪切，卡片会多一枚 **「序号被占用」** chip。
    - **Cutter 会拦下覆盖**：目标目录里已有 clip.json 且标题/区间都对不上 → 这一条剪切失败并给出
      原因（`clips/clip-04 已属于另一个片段（…）…先删掉那个片段再剪`），不会覆盖别人剪好的成片。
      所以重新检测后要重切，得先删掉旧的冲突片段。
    - **未剪切事件「预览」= 播原片 + 播放头锁在事件区间**（用户要求"入出口锁定在规定范围"）：
      底栏 `预览(▶) / 编辑 / 剪切`；弹窗复用已剪片段那个 `AppModal wide`，`playing` 多带
      `start/end/heat` 时走区间分支（`isRangeMode`）。源 URL 由切片页从 `status.input_abs`
      生成（`inputMediaUrl`，与预处理页同源）——**服务端只放行项目根内的文件**，拿不到时按钮
      禁用并写清原因。锁法：`loadedmetadata` 到位才写 `currentTime = 入点` 再 `play()`
      （提前写会被忽略甚至抛，`PrepareView.seekTo` 同款教训）；`timeupdate` 到出点 →
      默认 `pause()` 停在出点前 `RANGE_EPS`，勾「循环本段」则跳回入点；`seeking/seeked`
      越出 `[start-0.2, end+0.2]` 回弹。回写 `currentTime` 期间用 `clampBusy` 防重入
      （否则自己触发的 seeked 会再进来一次）。
      **区间预览不用原生 `controls`，播放控件是自写的**（用户要求：原生那条进度条跟标记条
      不是一个坐标系，看着就是"两条进度条对不上"，要统一可控）：视频只是画面（点画面 =
      播放/暂停），下面 `.ev-controls` 一行 = 播放/暂停 · **唯一那条带标记的进度条**（点/拖跳转、
      吸附进区间）· `段内 0:08 / 0:35 · 整片 2:46:40` · 静音 · 音量滑条 · 全屏
      （`stageRef.requestFullscreen`）。键盘：空格播放/暂停、←→ 各 5 秒（输入框内不抢键）。
      音量/静音走**全应用共用**的 `vtrim_master_vol` / `vtrim_muted`，读写用 `polish/audio.js`
      导出的 `readMasterVolume/readMuted/saveMasterVolume/saveMuted`（别各写一份，更别写成
      `parseFloat(...) || 1` —— 音量 0 是合法值）。已剪片段的预览仍是原生 `controls`
      （它没有第二条进度条，不存在对不上的问题）。
      `@error` 给人话（flv/mkv/HEVC 浏览器放不了），别黑屏。精度：`timeupdate` ≈4Hz，
      溢出 ≤0.25s，够用；要帧级得换 rAF（`PolishPreview` 有先例）。
    - **播放范围要看得见**（用户要求"对播放范围进行标记"）：`.ev-controls` 里那条 `.rng-track` 画的
      **不是整片，而是一段视窗** `[viewStart, viewEnd]` —— 整片 3 小时时事件那 35 秒只占 0.35%，
      糊成一根细线根本没法看，所以默认视窗 = **事件区间前后各留点余量**
      （`pad = max(10s, 事件时长 × 0.5)`，用户要求"显示当前播放范围前后多出来一点的部分"），
      知道了 duration 才画（`loadedmetadata` → `resetView()`）。
      条上 `.rng-band` = 事件区间（左右 2px 主色描边 = 入点/出点），`.rng-head` = 播放头；
      条左右两端各一个小字 = 视窗起止时刻，旁边是 `段内 0:08 / 0:35 · 整片 2:46:40`。
      **滚轮缩放**：以光标为锚点（那儿的时间不动），步长 `max(2s, 视窗 × 1/4)`（小窗口细调、
      大窗口几下就能拉到整片），窗口夹在 `[0, duration]` 内，最小 `MIN_WINDOW = 2s`；
      **双击复位**回默认视窗。**点或拖这条轨道就是跳转** —— 像素先按**视窗**换算成秒
      （`rangeTimeAt`），再 `seekRange` **吸附进 [start, end]**（键盘 ←→ 同一条路径）。
      播放头用 rAF 跟随（弹窗关闭 / 组件卸载时 `stopRangeClock` + 摘掉 window 的拖拽与键盘监听）。
    - **预览里要显示字幕**（用户要求）：原片上叠一行当前这句，`.ev-cap` 与预处理页的 `.cap-overlay`
      **同款**（`3.75cqw`/`0.625cqw` = 72px/12px@1920，奶白 `#dcbda0` + 深色描边，
      `.ev-stage` 要写 `container-type:inline-size` 它才生效）。文字来自 `transcript`
      （`/api/project-data/transcript`，原片时间轴坐标，与视频同一套时间），按 `rangePos` 取。
      **懒加载**：只在打开预览时才拉（3 小时录播几千条，别拖累页面首屏；拉过一次就缓存，
      重新打开不再请求）。取当前句用 `captionAt()` 二分（rAF 每帧都会调，别线性扫），
      往回多看几条以兼容手动编辑造成的轻微重叠；没有转写文件（404）就不显示，下次预览再试。
    卡片**不再显示「已去静音 / 屏蔽 N 段」**（用户要求）：屏蔽轨道是精修编辑器内部状态，不在外部列表暴露。
    编辑区网格用 `align-items:start`（两栏各取自然高度，右栏不跟着预览拉长）；
    精修页页头是**紧凑一行**
    （`.compact-head`：chip + 19px 标题，没有 `.sub` 第二行）——竖高都让给预览。
  - **时间轴编辑器必须保持紧凑**（用户要求精简；它是竖高大户，且**独占一整行**：v6.4 起横向
    空间 = 整个内容宽度，多出右栏那 300px，别再把它塞回左列）：泳道 **26px（原 34）×7 条
    + 标尺 20px = 202px**；**卡片头取消** —— 标题/提示经 `<slot name="head">` 进
    `TimelineEditor` 的 `.tl-head` 一行，与「去静音 / 撤销去静音」「重新后处理」按钮同排；卡片 padding
    收到 12px（`.tl-card`）。改泳道高度时同步三处：`wavePath`（响度包络线，见下条；
    常量 `WAVE_H = 26` 与 SVG 的 `height` 都在这儿）、`.block`（`top:3px; height:20px; line-height:20px; font-size:10px`）、
    `.lane-label`/`.lane-track` 的 26px。
    - **响度显示 = 按像素列取峰值的包络线，压在不透明的字幕块之上**（用户报「音频频谱需要更加清晰、
      看不出音量电平高低差和位置」，2026-09 重做）。**两个独立缺陷叠加**，改之前先读懂这两条，
      否则很容易只修一个、看着还是没变化：
      - ⚠️ **遮挡**：曲线画在**字幕泳道**上，而那条泳道的字幕块是 `laneBlocks` 里写死的不透明
        `#3b82f6`（`top:3px; height:20px`），原来波形 `z-index:1`、块 `z-index:2` ——
        **说话时段（最该看电平的时候）整段被盖住**，只剩字幕之间的空隙和块上下各 3px。
        现在 `.wave-layer` 是 **`z-index:4`**：压过字幕块(2)与选中块(3)、仍在播放头(5)之下。
        `pointer-events:none` 不能去掉（否则挡住拖块与选中）。
      - ⚠️ **刻度**：原来是 `h = ((db+60)/60) × 22`，即窗口 [-60, 0]；而实测
        （`media/…/clip-08/data/loudness.json`，**1330 点**，0.1s 一点）语音落在
        **-12 ~ -32 LUFS**、静音 -33 ~ -56 ——
        整段语音的高度只在 11~17.6px 之间抖，26px 泳道里**动态范围只有 6.6px**，看着就是一条平线。
        现在窗口是 **[-40, -8]（`WAVE_DB_TOP`/`WAVE_DB_BOTTOM`）**，同样语音占 4.5~20.1px，
        动态范围 12.9px（约两倍），且不削顶。**这两个常量就是"看不出高低差"的旋钮。**
      - ⚠️ **形态**：原来是「一个 0.1s 采样点一根 rect、宽 `max(1, scale*0.02)`」——
        1px/s 时 10 根柱子挤进 1px 糊成一片，120px/s 时点距 12px 而柱宽 1px、中间全是缝，
        两头都看不出"位置"。现在是**按 `WAVE_COL_STEP`(2px) 像素列聚合、每列取最大 db**
        （峰值保持，DAW 常规做法，也正好是编辑器要的语义："这一刻最响到过多少"；
        取均值会把瞬时爆点抹平，而找爆点正是看这条线的目的）。一列最多一个顶点 →
        **顶点位置永远跟着缩放走**，既不糊也不出缝；放大到比采样还细时会有空列，
        直接跳过让折线连过去。
      - **双层描边**：深色 halo（`rgba(15,23,42,.55)`、3.4px）+ 亮色芯（`#fff`、1.5px）。
        这条线要同时压在不透明蓝块**和**近白背景（`--bg` #FFF7F3）上，单色必然在一边糊掉；
        双层保证两种底上都有一条清晰的边。**改颜色前先想清楚它会落在哪两种底上。**
      - 用户明确**不要**电平参考线/刻度，**不要**新增泳道（保持 7 条 202px），
        也**不要**把字幕块改半透明 —— 这三条都是当时一起评估过、被否掉的方案，别再提。
    - **时间模型 = 开始 + 结束**（v6.6，用户要求「所有开始+持续时间的模式都改成开始+结束」）：
      事件字段是 `time`（开始秒）+ `end`（结束秒），**没有 `duration` 了**；分段/字幕/屏蔽本来就是
      `start`/`end`，四类块因此共用同一套控件。老工程的 `duration` 由
      `vtrim-render-core/src/legacy.rs::upgrade_event_times` 在**反序列化前**换算（`end = time + duration`，
      渲染 1:1 不变，下次保存即完成迁移），入口四处：`polish.toml` 加载（`validate.rs`）、
      `PUT /api/polish/project` 的 segments、LLM 事件输出 + 老 checkpoint（`llm/events.rs`）、
      wasm 预览配置。缺 `end` 的由 `process` Step 0 补默认（`Event::default_end`；`sfx` 例外 =
      用文件原始长度）。三处求值器统一走各自的 `evDur()` / `evDuration()`，别再直接读 `.duration`。
    - **控件统一**（v6.6）：**体拖移动 / 左缘改开始 / 右缘改结束 / 双击删除**，字幕与屏蔽块现在也能拖
      （以前 `startPointer` 第一句 `if (block.si == null) return` 把它们挡掉了）。唯一例外：**分段没有左缘**
      （缩 start 会把事件留在段外，正确处理要连带搬事件）。块对象都带 `kind`（`seg|event|sub|mute`），
      拖拽/选中/面板都按它分发；分段体拖仍然连带搬段内事件（用快照算增量，别逐帧累加）。
    - ⚠ **字幕/屏蔽轨道必须保持按时间有序**：`splitSelected` 拆出来的后半段要
      `splice(i+1, 0, …)` 插在原块**后面**，**不能 `push` 到数组末尾** —— 追加会让列表
      （以及落盘的 `clip.srt` / `data/subtitles.json`）乱序。乱序会连带毁掉所有按顺序读它的
      地方：去静音按相邻间隔算静音（曾判出 13 段/121s 的假静音，用户原话"去静音完全对不上，
      像用了别的素材"）、预览按时间二分取当前字幕。`PUT /api/polish/subtitles` 落盘前会按
      `start` 排序（老数据再存一次即自愈），`detect_silence_from_srt` 也会先排序+合并重叠 ——
      但别因此就允许编辑器产出乱序列表。
    - **属性面板 = 标配块 + 类型专属块**：标配永远是 `开始（秒）/ 结束（秒）` + 「设为入点 / 设为出点 /
      定位 / 删除」（拆分只给字幕/屏蔽，复制只给事件）；类型专属：分段（备注 + 情绪下拉）、字幕（内容）、
      事件（类型 + 各自字段）、贴纸（keyword 两级列表）、覆盖层（类型/颜色/渐变两端色/不透明度/形状）、
      音效（音效目录列表 + 音量）、瞬时特效（intensity / color），所有视觉事件还有「配声」(`sfx` + `sfx_volume`)。
      **「保存修改」按钮已删** —— 面板容器上 `@input/@change` 事件委托 → `touch(kind)`。
    - **覆盖层（overlay）的契约 = 一条规则，四处实现**（`timeline.rs::overlay_stops` 的文档注释里有全文）：
      `overlay_type == "gradient"` 的有效色档是 —— `colors` 滤掉空串后 ≥2 档就用它 / 恰好 1 档则
      用它 + 同色零透明凑两档 / 没有 `colors` 但有 `color` 则 `[color, 同色零透明]` / 都没有就不渲染；
      `"color"` 以 `color` 为准、缺失时退回 `colors[0]`。**单色一律「该色 → 透明」**（用户定的语义）。
      ⚠ 求值器**必须保证返回 ≥2 档且无空串**：出片路径按 `addColorStop(i/(n-1))` 铺档，`n == 1`
      是 `0/0 = NaN` 抛 `IndexSizeError`、空串色档抛 `SyntaxError`，两者都会**打断整个 chunk 的渲染**。
      ⚠ 兜底出来的第二档必须是**同色零透明**（`overlay_fade_stop` / `overlayFadeStop`），
      **不能用 CSS 的 `transparent`**：canvas 渐变在**非预乘**空间插值，`#ff0000 → transparent`
      中间会插出 `rgba(127,0,0,·)` 那种暗红 —— 实测同一条红色覆盖层（opacity 0.3、同一个源帧、
      同一时刻）画面平均 R「184.12 → **178.82**」（红了反而变暗）；换成 `rgba(255,0,0,0)` 才是
      「184.12 → **193.66**」的干净淡出。而 CSS 那边两者**完全等价**（按预乘插值，逐点取样相同），
      所以写 `transparent` 恰好会让**浏览器预览与出片观感不一致**。
      四处实现：`vtrim-render-core/src/timeline.rs`、`crates/vtrim-polish/templates/index.html`（**出片权威路径**）、
      `webui/src/polish/engine.js`、`webui/src/components/PolishPreview.vue`（经 engine 的 `gradientCss`）。
      **前两处是独立副本**（HTML 里的 JS 无法 import engine），漂移检查跑
      `node webui/scripts/check-overlay.mjs`（把 index.html 的两个顶层函数与 engine.js 逐例对拍）。
      ⚠ 为什么会有这条规矩：LLM 提示词里的示例长期只写
      `{"overlay_type":"gradient","color":"#ff0000"}`，而四处求值器都只认 `colors` ——
      实测 4 个项目 **8/8 覆盖层全都没画出来**（`colors` 压根没出现过）。所以：
      ①提示词示例现在给 `colors` 数组 + 纯色两个写法（`llm/segment.rs`，另有单测锁住），
      且写了 `colors`/`color` 却漏掉 `overlay_type` 时也会被推断成 overlay；
      ②老工程靠**读时兜底**，不用重跑 LLM。诊断：
      `VT_POLISH_DIR=<绝对路径> cargo test -p vtrim-polish --lib diagnose_overlay_stops -- --ignored --nocapture`
      （cargo test 的 CWD 是 crate 根，路径必须写绝对路径）。
      ⚠ 两处 DOM/GSAP 预览的渐变 CSS **结尾那个 `)` 不能省** —— 少了它整条 `background`
      非法被浏览器丢弃，表现正是"面板里设了色档、预览里什么都没有"（`engine.js::gradientCss` 一份，
      模板里 `overlayGradientCss` 一份）。径向语义 = **中心该色、边缘淡出**（canvas 是
      `createRadialGradient(960,540,0,…,960)`）；`handlers.overlay` 里那个只有页内预览支持的
      `overlay_type:"love"`（飘心）分支**已删** —— 留着就是"预览有、出片没有"。
      不透明度一律用**数值判定**（`typeof ev.opacity === 'number'`），**别写 `ev.opacity || 1`**：
      0 是合法值，falsy 判定会把它变成全不透明（面板 `min="0"`）。面板「终色」在未设色档时
      只能显示黑（`input type=color` 表达不了透明），所以旁边必须留那句退化说明。
    - **事件类型由轨道决定，不给跨轨道选**（v6.7，用户要求「类型应该是每个轨道前添加的按钮自动分配的，
      只有轨道类型下的细分类型会显示对应的类型」）：泳道标签上的 ＋ 负责定类型 —— 运镜/贴纸/覆盖层/音效轨
      直接加该类型；**视觉特效轨的 ＋ 弹一个 11 项细分类型菜单**（`openFxMenu`，fixed 定位，点别处关闭；
      菜单项的 `click` 不能被 document 上的 capture pointerdown 提前清掉，`closeFxMenu` 里用
      `closest('.fx-menu')` 放行）。属性面板里：特效事件给一个**只列这 11 种**的下拉（换细分类型仍走
      `changeEventType`：按新类型重建对象、保留时间跨度与配音），其余事件只读展示（`.type-ro`：「类型 贴纸 sticker」）。
      想换轨道就删掉重加 —— 别再退回那个 15 种的全类型下拉。
    - **自动保存 + 编辑完成后后处理 + 实时预览**（v6.6；触发时机 v6.7 改）：
      ①任何改动立刻 `emit('preview', …)` → `PolishView.onTimelinePreview` 就地镜像进 `cfg/subs`
      （**不 bump `previewKey`、不重挂载**）→ 预览索引重算，**拖拽过程中画面就跟着动**；
      ②700ms 防抖 PUT 落盘（segments+mute 走 `/api/polish/project`，字幕走 `/api/polish/subtitles`），
      面板底部显示「保存中… / 已保存 HH:MM:SS / 保存失败」；
      ③**落盘 ≠ 编辑完成**：后处理只在 `emit('done')` 之后跑 —— 编辑器在"一次操作真的做完"时才发：
      面板控件 `change`（失焦/选定；`input` 每敲一键**不算**）/ 取消选中 / 切片段（要带**旧 dir**，
      否则会跑到刚切过去的片段上）/ 组件卸载（卸载前先 `flushSave()`，别丢最后 700ms 的改动）。
      页面侧 `onEditDone` 再 1.5s 合并防抖 → `startJob('process', …, {silent:true, keepPreview:true})`，
      完成后 `load({silent:true})` + `reload()` 把后处理改写的结果同步回块与面板；`target !== dir`
      （切走/离开页面）时**只发任务不绑 UI**。`silent | keepPreview` 决定不重挂载预览 ——
      重挂载会把播放位置打回 0，编辑循环里不能接受；头部另有「重新后处理」按钮兜底。
      ⚠️ **别再改回"落盘后 1.2s 自动跑"**：你还在面板里给新建的贴纸填 keyword 时它就会插进来，
      半成品被当成品处理（数量抽稀直接把它删掉）——用户原话：「我新建一个贴纸，处理时它直接被删除了。
      需要的是只有操作完成时才进行后处理，而不是之中或者一些干扰的」。忙时仍走 `pendingProcess`
      （现在存 **dir**），空闲补跑一次。
    - **手工放的事件（`manual`）后处理一律不改写**（用户要求"别在之中干扰"）：
      编辑器在**新建事件**（`addEventAt`）、**复制**（`duplicateSelected`）与**改时间**时都打这个标记；
      `process` 侧三处吸附（`align_stickers_to_subtitles` / Step 2c / `align_pip_shots`）跳过它，
      **`cap_events` 的数量抽稀也跳过它**（额度只约束 LLM 生成的那些；不豁免的话均匀采样会把中间那个
      漏掉，表现就是"我新建的贴纸过两秒自己没了"）。面板 chip 写明语义（`手动` = 不吸附、不抽稀 /
      `自动对齐` = 会被吸附、超额度可能被抽稀），「跟随字幕」清标记交回自动处理。
      回归：`cargo test -p vtrim-polish --lib cap_events_never_removes_manual_events`；
      真实工程上跑一遍：`VT_POLISH_DIR=<片段目录> cargo test -p vtrim-polish --lib diagnose_manual_events_survive_process -- --ignored --nocapture`（**会写回该工程的 polish.toml**，只在临时工程上用）。
    - **贴纸选择 = 两级列表**（v6.6，用户要求「keyword 作为一级筛选，选完自动装载 name 列表」）：
      一级 = 全部 keyword（`包名 · tag`），二级 = 该 keyword 下的贴纸缩略图；选中写
      `keyword/sticker_tag/name/source`（`name` 是**去扩展名的文件名**，与 `copy_sticker_from_local`
      的匹配一致），选「自动」清掉 name/source 交回后处理。数据来自**只读**接口
      `GET /api/polish/sticker-library`（磁盘为准）与缩略图 `GET /api/polish/sticker-file`
      （与 admin 版同款路径安全校验）；音效列表来自 `GET /api/polish/sfx-catalog`。
      写操作仍然只在 admin 侧，别把库管理接口开到 `/polish/*`。
    **「去静音」只在时间轴头部**（那里带状态文案与「撤销去静音」）；右侧「生成与导出」卡
    **不要再放一个去静音**（用户指出重复）。（v6.5：头部那颗**常显**，见下面「分段为空」那条。）
    **去静音 = 与当前屏蔽状态「相加」**（用户要求）：检测出的静音**并入**「屏蔽」轨道，
    重叠/相邻的合成一段（不是追加 —— 追加会在手动 10~20s 的屏蔽上再叠一块 16.5s 的红块）；
    状态文案要带上相加后的总数（`已自动屏蔽 N 段静音 · 屏蔽轨道共 M 段`），而不是只报自动那几段。
    撤销是**差集**：只减掉当初自动加进去的区间（`silence_ranges`），手动屏蔽原样留下。
    **播放头跟随 = 只在播放时跟，且越界才滚、只滚刚好**（两轮反馈合起来的要求：
    ①"只有把指针拖出显示进度外才移动视角，而不是乱飞"；②"拖动还是不主动变化，只能播放或者
    自己移动时间轴来移动"）：`watch(currentTime)` 第一句就是 `if (!props.playing) return` ——
    **不播放时跳转/拖动一个像素都不动视角**；播放时播放头若在可视区（左右各留
    `margin = min(140, max(40, 12%视口))`）内同样不动，越界才把 `scrollLeft` 调到让它刚好落在
    余量线内侧。`drag`（拖块）/`rulerDragging`（标尺划动）/手动滚动后的 2 秒（`userScrolling`）
    一律不跟。**别再退回"每帧把播放头钉在 30% 处"**（`scrollLeft = t*scale - 30%宽`）——
    预览是 rAF 驱动的，那样等于一帧一滚，画面永远在飘。
    `playing` 这条链：`PolishPreview` 在 play/pause/ended 里 `emit('playing', …)` →
    `PolishView.previewPlaying` → `TimelineEditor :playing`（别用"时间在涨"猜播放状态）。
    坐标必须用**标尺轨道的实际左边**（`rulerTrackRef` 的 rect 减 scroller 的 rect）+ `t×scale`：
    最左边是 sticky 的 96px 标签列，直接拿 `t*scale` 当可视坐标会差一个标签列宽。
    **分段为空时时间轴照常渲染**（用户报过：删掉最后一段后整块被换成一句"暂无时间轴数据，请先运行
    LLM 自动生成分段"，连字幕/屏蔽都改不了）：`.tl-no-seg` 只留一句提示，整条时间轴（含
    字幕/屏蔽泳道）照旧画，别改成 `v-if` 整块隐藏。那个位置**不再有「新建分段」按钮**
    （v6.5，用户要求：它是为上面那个 bug 加的临时入口，bug 已修；分段一律由「LLM 剪辑」生成）。
    **「去静音」在时间轴头部常显**（v6.5）：它只读 `polish.toml` 的视频/字幕（`/api/polish/silence`），
    不依赖分段，所以**别再加回 `v-if="segments.length"` 门控** —— 那会让删空分段后既没有分段入口、
    也没有去静音。事件只能挂在分段上，`addEvent()` 在无分段时给的是"去跑 LLM 剪辑"的提示。
    ⚠️ **别用裸 `<template>` 包裹要无条件渲染的内容**：Vue 会把它渲染成真正的 `<template>` 元素，
    而浏览器**不渲染 template 的内容** —— 表现是整块界面凭空消失，可 `document.querySelector`
    照样能查到（DOM 存在 ≠ 看得见，这一版就这么翻过车）。要无条件渲染就直接把内容展开。
    同理：**前端布局类改动必须做几何断言**（`getBoundingClientRect()` 宽高 > 0），
    光查 DOM 节点在不在会漏掉"渲染出来但零尺寸/被 display:none"这类问题。
    ⚠️ 写 headless harness 时：**`app.mount(host)` 会清空 `host` 的内容** —— 属性面板的
    Teleport 挂载点（`#tl-props-host`）要挂在 host 的**内层节点**上、组件 mount 到另一个内层节点，
    否则挂载点被清掉 → `teleportTo` 解析不到 → 面板整块不渲染（harness 实测踩过）。
    另外：选中一个块是 **pointerdown** 干的（`startPointer`），`block.click()` 不会选中，
    得派发 `pointerdown` + window 上的 `pointerup`。
  - **属性面板在视频左侧、与时间轴分离**（用户要求）：面板仍由 `TimelineEditor` 渲染（选中
    状态与全部编辑逻辑留在组件里），但用 `<Teleport :to="teleportTo">` 投到
    **`PolishView` 预览卡里的 `#tl-props-host`**（prop `propsTarget` 的默认值即该选择器）。
    **⚠️ 必须门控 Teleport**：Teleport 的目标在**挂载时**用 `document.querySelector` 解析，
    而子组件挂载时整棵编辑器子树还没 `hostInsert` 进 document —— 直接 `:to="propsTarget"`
    会解析到 null、**面板整块不渲染，只剩挂载点那块空白**（实测踩过）。
    正确做法：`const teleportTo = ref('')`，在 `onMounted`（post-flush 阶段，那时页面已进文档）
    里 `document.querySelector(target)` 成功后再设值（并留一次 `nextTick` 重试 + 失败 console.warn）。
    挂载点样式：父级 `position:relative; width:232px`，面板 `position:absolute; inset:0`
    （页面 scoped CSS 用 `:deep()`），长表单只在面板内滚动，不会撑高预览卡把时间轴顶出首屏。
  - **预览尺寸随视口收缩**：`PolishPreview` 的 `.stage` 宽度 =
    `min(100%, calc(clamp(170px, 100vh - 540px, 620px) * 16/9))` —— 高度按 16:9 由「剩下的
    竖高」反推。`540` 是按「其余固定部分 ≈ 611」（面包屑 66 + 内容 padding 18 + 紧凑页头 ~38 +
    片段卡条 1 行 ~105 + 编辑区上边距 16 + 预览卡头/控制条 ~93 + 行间距 16 + 时间轴卡 ~259）
    留了 ~70px 的超出：**时间轴第 7 条泳道会落在首屏外，要小滚一下**（用户明确要预览更大 →
    把 540 调到 610 即时间轴完整可见，代价是预览回到 ~350）。上限 620 防大屏过分占位，下限 170。
    **别再写死 `66vh`**（那样只有预览自己能进首屏）。
    时间轴独占整行后，编辑区**第一行**的高度 = `max(预览卡, 生成与导出栏)`：右栏比预览卡高时
    时间轴会再往下推一点（预览卡更高时位置与从前一致），所以别再往右栏无谓堆竖高。
  - **预览音量/静音 = 总音量，必须同时落到 `<video>` 元素**（`PolishPreview` 的 `applyAudio()`）：
    滑块 = 视频 `volume` + SFX 播放器 masterVol，静音 = 视频 `muted` + SFX `setMuted`，
    两者都写 localStorage（`vtrim_master_vol` / `vtrim_muted`，由 `polish/audio.js` 读写）。
    **只调 SFX 播放器就是 bug**：视频里的说话声/音乐丝毫不受影响，用户实测反馈「音量控制不好使」。
    读回音量用 `isFinite` + clamp，**不能写 `parseFloat(...) || 1`** —— 音量 0 是合法值，
    falsy 判定会把它变成 1（`templates/index.html` 的独立预览页同一处也已修）。
    音量为 0 时按钮按「没声音」显示（`silenced`），点一下把音量抬回上次非零值 ——
    别做成一按没反应的死按钮（`applyAudio` 在 `onMounted` 与 `loadedmetadata` 都要调，切片段后要重贴）。
    垂直版背景 `<video>` 始终 `muted`，否则同一段音轨响两遍。
  - **区域编辑（取代九宫格 3×3 选择器）**：在「生成与导出」卡的 **LLM 剪辑参数块**里，
    只有「人物」「弹幕区域」两个按钮（同一个再点=退出，点另一个=换区域；无区域下拉、
    无贴纸区域）→ 左侧 `PolishPreview` 进入**选框模式**（视频运镜暂停、缩放/滤镜复位；
    盒外压暗；拖框体平移、拖 8 个角/边缩放、空白拖拽重画，坐标为 1920×1080 整帧百分比），
    读数 / 保存选框 / 清除选框都在右侧同一块内，预览卡头显示「选框模式 · 运镜已暂停」chip。
    （**别把这些控件放到页头**：页头在片段列表上方、离预览很远，用户实测反馈"按钮不见了"。
    也别只放在预览卡头而让参数与动作分家 —— 用户要求"区域编辑和贴纸来源作为 LLM 剪辑的参数"。）
    **贴纸落点不在 UI 里框选，也不再由 `layout.sticker_area` 决定**：`process` 每次按
    **人物(liver) / 字幕带 / 弹幕区域(danmaku_area) + 真实弹幕带** 反选安全区，在安全区
    **中心**撒点（细节见下条「贴纸落点推导」）。
    保存写 `polish.toml` 的 `layout[区域].top_left/bottom_right`（"55.00%" 百分比字符串），
    清除即删除两个角回到原 `grid`（grid 永不被框选改写 → 旧工程零迁移）。
    **保存 / 清除选框后自动重跑一次 `process`**（`reprocessLayout()`，与时间轴保存同一条路：
    `startJob('process', …)` + `load({silent:true})`）。这两个框是 **process 的输入**、不是渲染的输入：
    Step 2b 按 `layout.liver`/`layout.danmaku_area` 反选安全区并**重写每个贴纸的 `x/y`**，
    Step 2d 把 `anchor` 解析成 `origin`/`to_origin`（**每次运行都覆盖**）。渲染侧只认这些结果坐标
    （`index.html` 的 `ce.origin || liverOrigin` 里，读 layout 的只是"事件没写 origin"时的兜底），
    所以改了框不重跑 process，出片/预览还是旧焦点。
    ⚠ **别再写回"保存不触发任务 / 人物框只影响运镜聚焦中心（渲染时读取）/ 弹幕区域仅作参考"**——
    那是加锚点解析（Step 2d）之前的旧说法：人物框现在同时喂 Step 2b 与 2d，弹幕区域也既当贴纸障碍、
    又是 `anchor="danmaku"` 在那一刻没有真实弹幕时的兜底。process 无 LLM、幂等
    （每次都按当前 layout 全量重算 x/y 与 origin），跑一次是秒级。
    读取优先级（`vtrim-render-core/src/types.rs`）：`free_rect()` →
    `region_rects()`/`region_origin()` 一律**自由选框 > grid > origin_grid/origin**；
    JS 侧同一规则在 `webui/src/polish/engine.js` 的 `regionRect/regionOrigin`（GRID_RECTS
    表须与 Rust `grid_rect` 对齐）与 `templates/index.html` 的 `liverOrigin`。
  - **运镜（camera）三个能力**：**聚焦**（`anchor` = liver/danmaku/sticker/center）、
    **位移**（再写 `anchor_to` 即从 origin 平移到 to_origin）、**画中画**（`zoom="远"`）。
    - **锚点是「意图」、坐标是「结果」**：`process` Step 2d 的 `resolve_camera_anchors`
      把 `anchor`/`anchor_to` 解析成 `origin`/`to_origin`（与贴纸 keyword→name 同一套约定），
      所以改了选框/挪了贴纸重跑 process 就生效，不用手改坐标。渲染侧四条路径都只认百分比坐标。
      排位在贴纸落点(2b)与时长自适应(2c)之后——`anchor="sticker"` 要读最终位置。
    - **倍率只有一张表**：`CameraStyle::factor`（`vtrim-render-core/src/types.rs`），
      模板 canvas / 模板 GSAP / `engine.js` / `timeline.rs`（wgpu，未启用）四处都走它。
      ⚠ 模板 canvas 路径曾硬编码 `1.8/1.5` 且完全不读 `[style.camera]`，而它才是出片路径
      （`vtrim-polish/Cargo.toml`：「chrome-render 为默认出片路径」）——别再把倍率写回去。
      同样的坑：`placement.rs` 曾为了对齐那条硬编码而复刻了一份。
    - **画中画的判据是 `|zoom-1| > 0.001`，不是 `zoom > 1.001`**（`templates/index.html` 与
      `PolishPreview.vue` 两处）。写成后者会把「远」档整档吞掉，画面完全不动。
    - **远景进出是硬切，不是缓动**（用户要求）：过渡的一端只要是「远」，「zoom」与不动点就在
      事件起点**一次性到位**，那一段跨度 (`end - time`) 不产生任何缓动、位移也不生效。
      缓动缩到画中画看着像「镜头在拉远」，而它要的是喜剧式的切换。
      三处求值器同步实现（模板 canvas 让 `p = 1.0`、GSAP 用 `tl.set` 取代 `tl.to`、`engine.js` 同 canvas）；
      贴纸避让侧走 `placement::transition_secs()` → `CameraShot.transition = 0` ——
      别写回常量 0.6，否则会按 0.6s 过渡凭空多算一个早已结束的旧状态当障碍。
    - **硬切 `cut`（切镜）= 同一条规则放开给任意景别**（用户要求「给运镜增加一个切镜的变体」）：
      `Event::Camera` 的 `cut: bool`（serde `skip_serializing_if = "is_false"`，老配置零迁移），
      编辑器属性面板第一行「方式 = 平滑运镜 / 硬切」。`cut = true` 时这一条**整档瞬间到位**：
      三处求值器的 `hardCut` 判据都加了 `ev.cut === true`，`transition_secs(label, prev, cut)` 同理。
      想「切过去再推」就写两条事件（切 + 平滑），别在求值器里搞两段式。
      泳道块标签与模板编辑器的 `labelFor` 都加「切·」前缀。
    - **三档倍率的当前值：中 1.3 / 近 1.5 / 画中画 0.55**（用户定：中 1.3、近 1.5）。
      **放大档位不存在「推出框露背景」的风险**：运镜是绕不动点做 `scale`，`s ≥ 1` 时画面四周
      一律向外扩张（DOM 靠 `overflow:hidden` 裁、canvas 靠画满 1920×1080），任何倍率都盖满整帧，
      露底只发生在画中画（`zoom_wide < 1`，刻意留黑边 `(1-factor)/2×1920 = 432px`）。
      所以再调大是安全的，别再拿「怕露出背景底色」当理由往下压。
    - **旧内建默认按值迁移**：`process/mod.rs::migrate_camera_zoom` 在 Step 0a 把
      中 `1.15 → 1.3`、近 `1.3 → 1.5`、远 `0.7 → 0.55`（**只认这几个旧默认值**，手改过的不动，
      日志逐条报）。理由是这些值只可能来自 `Default`（UI 从来没有入口，只能手改 polish.toml），
      等价于「没指定」——与下面 `origin = "50% 50%"` 的迁移同一个道理；而它们**早已被序列化写进
      老工程的 polish.toml**（实测 `media/` 下 29 份全是 1.15/1.3），只改 `Default` 对老工程无效。
      三处兜底默认值（`default_zoom_mid/close/wide()`、模板 `camFactor`、`engine.js` `camFactor`）
      必须一起改。
    - **画中画配额：一个切片只有一条**（用户要求）：`process/fx.rs::cap_pip_shots`
      —— 按时间序只保留**最早**的一条「远」，其余降级成「中」（**只改 zoom**，anchor/origin 留着）。
      （上一版是"≤2 条 + 不能连续 + 间隔 ≥60s"，配额收到 1 之后后两条被完全包含，已删除。）
      调用点两处：`cmd_process` Step 1c（在补默认运镜之后）与 LLM 落盘前（`llm/mod.rs` Step 4b，
      让编辑器里看到的就已经合规）。prompt 里也写了同样规则，但 LLM 是按段窗口生成的、
      数不出整片次数，所以守卫才是兜底。
    - **画中画要对齐字幕/事件边界**（用户要求："不能没头没尾的断开"）：
      `process/fx.rs::align_pip_shots(subtitles)` —— 起点吸到**包含它的那条字幕的开始**
      （落在空隙里就吸到下一条字幕的开始），终点吸到**那条字幕的结束**（空隙里就吸到上一条的结束）；
      结果夹在所属分段 `[start, end]` 内（分段就是"事件"边界，切镜不跨段）；
      对齐后不足 `MIN_PIP_SECS`(1.5s) 就往后借下一条字幕的结束。只动 `time`/`end`。
      两个调用点与配额守卫成对（process Step 1c / LLM Step 4b），字幕用的是**过滤掉屏蔽区间后**的那份。
    - **画中画到点收工**（用户要求：「镜头事件需要拉长无缝连接…本来是画中画的戏剧效果，被延长后
      感觉很有诡异」）：运镜是**状态机**，事件只写起点、档位保持到下一条运镜 —— 别的档位的
      别的档位的跨度只是"多久推到位"，**画中画的 `end - time` 等于"它该待多久"**。不补就一直挂着
      （实测 clip-35 一条 2.1s 的画中画挂成 16.8s：下一条运镜在 16 秒外）。
      `process/fx.rs::close_pip_shots` 在画中画终点补一条**回镜头**事件：档位/不动点取
      **时间上**前一条非画中画运镜的**落点**（`anchor_to`/`to_origin` 就是终点那一侧；没有前一条
      就照默认运镜聚焦主播），持到下一条运镜；下一条运镜在 `MIN_RETURN_SECS`(0.5s) 内就来则不补。
      补的是**真事件**不是渲染侧特例 —— 模板 canvas/GSAP、`engine.js`、编辑器、贴纸避让一致，
      回镜头本就是硬切（「远」进出硬切那条规则管）。幂等。⚠️ **前后镜头按时间算，不按文件顺序**：
      段之间时间上会重叠（LLM 按窗口生成 + 去静音切段），求值器也是先按时间排序再走的。
      调用点同配额/对齐两处（process Step 1c / LLM Step 4b）。
    - **位移用 zoom 的同一段窗口与曲线**（`min((end-time)*0.5, 0.6)`、`pow2Out`），
      不新增旋钮；不写 `anchor_to` 时两端相等，退化成「原地聚焦」。
      「主播→弹幕→回主播」= 两条事件，不是一条（一条只表达一段位移）。
    - 贴纸避让跟着位移走：`CameraShot.origin_to` + `camera_states_for` 推**两端**，
      且「上一状态」取上一段的**落点**。稳态窗口里不能再把位移起点当可见状态（会凭空多一堵墙）。
    - `process` 给每个 ≥1s 且没有运镜的段补 `zoom="中", anchor="liver"`（整段）——
      所以运镜是**全程开着**的，不是点缀。改默认档位/默认锚点就在这一处
      （`process/mod.rs` Step 1 与 `llm/mod.rs` Step 4 是两个重复实现，要一起改）。
    - **老工程迁移**：`origin = "50% 50%"` 且无锚点的事件补成 `anchor="liver"`（有日志计条数）。
      这个值从来不是用户能设的（LLM prompt 示例 + 两个生成器写死的常量），所以当成「没指定」。
      只读诊断：
      `VT_POLISH_DIR=<项目目录> cargo test -p vtrim-polish --lib diagnose_camera_anchors -- --ignored --nocapture`
      画中画配额/对齐/收工 + 去静音阈值 + 贴纸对齐在**真实工程**上的效果（含 2.0/3.0/3.5/5.0s 阈值对照）：
      `VT_POLISH_DIR=<项目目录> cargo test -p vtrim-polish --lib diagnose_pip_and_silence -- --ignored --nocapture`
      （加 `VT_CAM_JSON=1` 会多打一行 `CAM_JSON={...}`：处理后的运镜轨道，可直接喂给
      `templates/index.html` 的 `getActiveCamera` 与 `webui/src/polish/engine.js` 的 `cameraAt` 对拍）
  - **封面特效 emoji（`[[cover.effects]]` kind = `emoji`，用户要求"给主播加点小表情"）**：
    `text` 是贴的字符（默认 `🙂`，最多 `COVER_EMOJI_MAX_CHARS`(12) 个字符 —— 带 ZWJ/肤色修饰符的
    一个 emoji 就要 7~8 个码点），`x/y` 是**字符中心**（画布百分比，`translate(-50%,-50%)`），
    `amount` 当大小用（`COVER_EMOJI_MIN_PX`(48) ~ `COVER_EMOJI_MAX_PX`(288)px，公式只在
    `cover.rs::cover_fx_html` 里一份，前端 `fxStyle` 是镜像并按 `stageScale` 缩放）。
    ⚠️ **`text` 是用户输入，进模板前必须 `escape_html`**（不转义＝往封面 HTML 里注模板，有单测）。
    加特效时组件默认把 x/y 设成 `layout.liver` 的聚焦点（`liverOrigin()`）——那个点是帧层
    transform 的**不动点**，所以缩放/平移帧都不会把小表情带跑。字体栈要带
    `Segoe UI Emoji / Apple Color Emoji / Noto Color Emoji`，否则出片是豆腐块。
  - **封面设计器「标题与高亮」的标题文本框**（用户要求）：**固定 5 行高、不可拖拽缩放**
    （`CoverDesigner.vue`：`rows="5"` + `.title-lines { line-height:1.5; resize:none }` ——
    高度 = 5×行高 + padding ≈ 108px，内容超出在框内滚动）。别改回 `rows="2"`，也别加
    `resize:vertical`。字段外观用**设计基准 `.fld-input`**（app.css：bg-soft 底 + 6px/10px
    padding + 12.5px + `--r-sm` 圆角，与 multiuser 样稿封面板一致）：曾有一版更小的紧凑覆盖
    （`.grp textarea{font-size:12px;padding:5px 8px}`），用户要求**样式回归设计**，不要再加。
    出片与预览都按 `title_lines` 逐行渲染，5 行是完整支持的。
  - **封面舞台的安全区线框（`CoverDesigner.vue`，用户要求「一个是 4:3 一个是 16:9」）**：
    舞台（1920×1080 的 16:9 画布）上叠一层纯辅助线 —— `.guide-layer`（`inset:0`、
    `z-index:7`、`pointer-events:none`）。压在标题（z5）与手势层（z6）**之上**才看得出
    "标题有没有越线"，而 `pointer-events:none` 保证不挡拖帧/滚轮。里面两条框：
    - **16:9 安全区**（`.guide-169`）= `width:95%; height:95%` 居中 → 舞台坐标 **1824×1026**
      （四周等比内缩 5%：平台 UI / 圆角 / overscan 会吃掉的部分）。
      ⚠️ **别写成 `inset:5%`**：CSS 的 inset 百分比是**横向按父宽、纵向按父高**解析的，
      看着像等比其实不是（这么写会得到 1728×972，连 16:9 都不是）。等比缩放只能写 width/height。
    - **4:3 裁切区**（`.guide-43`）= `height:100%` + `aspect-ratio:4/3` 居中 → **1440×1080**
      （满高，被裁成 4:3 时还看得见的部分）。宽度交给浏览器算：不过 JS、不碰 `stageScale`。
    两条框**故意互相交叠**（16:9 更宽更矮、4:3 更窄更高，横线穿过 4:3 框）—— 这是标准的宽容区
    画法，别为了"好看"把它们改成嵌套。
    线是**白线 + 1px 深色外圈**（`box-shadow:0 0 0 1px rgba(0,0,0,.45)`）：封面素材亮暗不定，
    单色线总会在某一类画面上糊掉。线宽与字号用**预览 px**（**不乘 `stageScale`**）—— 它不属于
    「设计器 ⇄ 出片的镜像清单」（那套规矩是给会进成片的东西定的），只是 UI 辅助线，
    纯 CSS 相对尺寸跟着舞台一起缩放即可。
    ⚠️ **只画在设计器里**：`crates/vtrim-polish/templates/cover.html` 与 `cover.rs` 一个字都没改，
    所以线框**不可能**进入成片。将来若要往 CLI 那个独立预览页（`vtrim polish preview cover`）加，
    必须同时补 `html.export .guide-layer{display:none}`（与 `#toolbar` 同一机制），否则进成片。
    开关是舞台下方独立一行两个 chip（`.mini-btn.selected` 绿底，与「预设」同款），
    **各自独立**可开可关（可能只想看 4:3），默认两个都开；偏好落
    `localStorage['vtrim.ui.coverGuides']`（`"169|43"`，少一段就是关了一个），
    **在 setup 里同步读**（照 `stores/ui.js::readSideCollapsed` 的写法）—— 放 computed / onMounted
    会先画一帧展开态再切，闪一下。开关**不写 `cover.toml`、不加后端接口**：纯前端显示偏好。
  - **标题首次自动排版 = 塞进 4:3 裁切区（`CoverDesigner.vue::fitTitleIntoSafeArea`）**：
    用户要求「第一次获取到标题时需要将排版弄好然后放在安全区内，后续手动操作时不要执行」。
    - **触发只有两处**（都是"标题被**获取**"）：① `loadCoverConfig` 发现磁盘上 `title_lines`
      **为空**（新工程，靠 `project.name` 回退拆行）；② `generateTitleByLlm` 拿到新标题。
      手动操作（改文字 / 拖滑杆 / 点预设 / 拖拽）**一律不触发** —— 这是"仅限第一次"的全部含义，
      **别在 watch 里接它**，那样用户每动一下都要跟自动排版打架。
    - **已有封面一律不碰**：判据就是"磁盘上有没有 `title_lines`"。非空 = 排过/存过 → 一个字段都不改
      （用户明确要求不动 `media/` 下已有的那二十多个封面）。所以**不需要新增配置字段**，
      整件事零后端改动：只动 `font_size` / `text_x` / `text_y`（必要时一次 `text_width`），
      全都在 `/api/polish/cover-config` 的白名单里，由现有 600ms 防抖 watch 落盘。
    - **目标矩形 = 那条 `.guide-43` 线框**（4:3 裁切区）：画布 1920×1080 上
      `x ∈ [240, 1680]`（= 1080×4/3 居中，左右各留 240px）、`y ∈ [0, 1080]` 上下贴边；
      再内缩 `12 + stroke/2` 画布 px 的安全边距 —— `-webkit-text-stroke` 沿字形轮廓**向外**画，
      且设计器的字体与出片 headless Chrome 能拿到的字体未必一致，宁可保守。
    - **量的是 `.title-text` 而不是 `.title-wrap`**：后者是 62% 宽的容器，拿它当约束会把本来
      已经在框内的标题白白挪走。`textX/textY` 是 `.title-wrap` 的**中心点**（`translate(-50%,-50%)`），
      所以平移中心 = 整体平移。
    - 顺序：先按 `min(tW/bW, tH/bH)` 比例缩字号**并迭代**（字号一变换行数会跳变，不能一次算完），
      到下限 24 还因为换行过多而偏高时才放宽一档文字区（没变矮就退回去），最后按各边溢出量平移。
      **告警只看最终状态**（`relaxed = !fits(b)`）—— 中途"缩不动"不等于失败，早置位会误报。
    - ⚠️ **默认对齐是 `left`**（`default_cover_text_align()`，2026-09 由 `center` 改来）：默认预设
      就叫「左文」、默认 `text_x = "35%"`，三处以前是"预设左文、配置居中"，自相矛盾。
      已有工程的 `polish.toml` 里都显式写着 `text_align = "center"`，字段存在就不走默认，**不受影响**。
      改这个默认值**不要顺手改 `cover.rs::cover_text_align` 的 `_ => ("center","center")` 兜底** ——
      那个 `_` 同时兜着合法值 `"center"` 本身，改了会把显式居中的老封面一起翻成左对齐。
      左对齐 + 默认 `text_x=35%`/`text_width=62%` 时文字左边缘在 76.8px，**本来就在 4:3 裁切区
      （左边界 240px）之外** —— 所以上面第 3 步的平移对左对齐是第一次必然发生，不是可有可无。
    - **安全区这件事后端不知道**（`cover.rs` / `cover.html` 一个字没改），所以只能在前端算：
      好在设计器用的是与出片同一套 CSS（字体栈 / line-height / text-align / `-webkit-text-stroke`）。
      定位是"把初始排版调好"，**不是像素级契约**；实在放不下会 `console.warn` + 面板上一行琥珀色提示
      （`.fit-hint`），不静默。
  - **封面三层：文字 / 关键帧 / 背景**（用户原话：「封面分三层：文字，关键帧，背景」）：
    `#title`（文字）在最上、`#frame`（**关键帧层**：当前帧清晰图 `_cover_frame.jpg`）居中、
    `#bg`（**背景层**）在最下，再往上才是渐变/暗角/特效。**三层各有各的图源，谁也不许覆盖谁**。
    - **背景层三种模式**（`[cover] bg_mode`）：`color` 纯色（默认黑，`bg_color` + 手填 16 进制码）/
      `blur` 关键帧**模糊副本**平铺（`blur_scale` 0.2~4）/ `image` 导入的自定义图片 `_cover_bg.jpg`。
      各自读各自的文件：blur → `COVER_FRAME_IMG`，image → `COVER_BG_IMG`；
      `image` 模式**没有那个文件就退回 color**（`cover_bg_vars(config, bg_image)`，别在模板里挂死图）。
    - ⚠️ **导入背景图片写的是 `_cover_bg.jpg`**（`/api/polish/cover-background` 同时把 `bg_mode` 设成
      `image`）。**别再写回 `_cover_frame.jpg`** —— 那是"背景替换关键帧"的老 bug（用户报过
      「又把关键帧那一层给我删了」）。`cover.image` 也**不要**在导入时清空（它是关键帧的来源之一）。
    - ⚠️ **出片不再删 `_cover_frame.jpg`**（`cmd_cover` 末尾那句 `remove_file` 已删）：它就是关键帧层的
      图源，删了等于①用户导出的那一帧白导、②下次出片又按 `frame_pos` 重新截、同一份封面每次换脸。
    - **关键帧可拖动/滚轮缩放**（用户要求"按之前的逻辑"）：设计器 `.stage-gesture-layer`
      （拖动 = 平移帧、滚轮 = 缩放帧 0.5~3）→ `coverOffsetX/Y`、`coverZoom` → payload 的
      `bg_x/bg_y/bg_zoom`（服务端存 1920 画布 px，预览按 `stageScale` 换算）→ 出片写进
      `#frame img` 的 `transform`，**与背景层互不影响**。这几个字段是**活字段**（曾一度降级为遗留）。
    - 帧的 transform 只此一份：`cover.rs::cover_frame_vars`（原点取 `layout.liver` 的聚焦点，
      右半边基准 1.4 / 左半边 1.3，再乘 `bg_zoom` 夹到 0.2~3）；前端 `mediaStyle`+`GRID_CENTERS`
      是镜像，改一边要改另一边。模糊倍率的公式同理：`cover_blur_css` ↔ `coverBlurCss`。
    - 设计器分组按三层排：**「标题与高亮」（文字）· 「关键帧」（时间 / 取当前帧 / 导出关键帧 /
      帧缩放 / 复位 / 偏移读数 / 「拖预览移动 · 滚轮缩放」）· 「背景与阴影」（背景三模式 + 渐变 + 暗角）**。
      纯色那格是取色器 **+ 手填 16 进制码**（`.hex-input`，`#1a2b3c` / `#abc` 都认，非法退回上一个值）；
      「导入背景图片」按钮**在背景组里**（image 模式下才出现），不在关键帧组、也不再挂在播放条那一行。
    - ⚠️ **关键帧文件可能不存在**（新工程没导出过、也没出片过）：设计器里模糊背景会先 404 ——
      图被 v-if 摘掉、看着像"没运行"（用户报过）。`onBgImageError()` 的补法：**只对 blur 模式**
      （image 模式缺图只提示"还没导入"）**不带 time** 调 `/api/polish/cover-frame`，服务端按
      `[cover] frame_pos` 截（与出片自己要截的是同一帧），只补一次（`bgFrameTried` 防循环），
      再失败就在模式下面提示手动「导出关键帧」。换背景模式会重置这两个状态。
    - 三处必须同步：`CoverConfig`（`project/config.rs`）、`/api/polish/cover-config` 的白名单 patch
      （漏键＝"点了没反应"）、`templates/cover.html`。
    - ⚠️ **设计器预览 ⇄ 出片的镜像清单**（用户报过"设计器和实际渲染的图片不一致"；下列每一项
      都曾经各写各的，出片是事实来源，预览照着抄）：
      · **字体**：模板以前**没写 font-family**，headless Chrome 落到它自己的默认字体上，而预览用
        应用的 `--font` —— 现在两边都是 `'LXGW WenKai','Noto Sans SC','Microsoft YaHei',sans-serif`
        （与 `templates/index.html` 同一套）。
      · 文字描边色 `#2a2522`（不是半透明黑）、文字阴影 `0 5px 22px rgba(0,0,0,0.78)`（预览要按
        `stageScale` 缩放，页面里是同一个 `coverShadowCss`）。
      · **渐变底色**取自 `[style.subtitle] highlight_color`（`build_cover_html` 里那套
        `(1-c)×{30,25,50}` + min 10/10/15），预览以前写死 `rgba(20,10,40,…)`。
      · **图层序**（自下而上）：背景 < 帧 < 暗角 < 渐变 < 特效 < 标题 —— 模板里 `#fx-extra` 是
        `z-index:1`、暗角/渐变是 auto，所以特效在它们**之上**；预览的 `.vignette/.gradient/.fx`
        必须按同一顺序给 z-index（2/3/4）。
      · 特效公式：`blur` = `1 + amount×14` px、`overlay` = `rgba(hex→rgb, amount)` + `mix-blend-mode`
        （不是"色块 + 元素 opacity"）；emoji 的 x/y 是字符中心、字号 `48+amount×240`、
        `drop-shadow(0 6px 14px)`，预览一律乘 `stageScale`。
      · **`liverOrigin()` 的优先序必须与 `region_origin` 一致**：自由选框中心 > `origin_grid` >
        `origin` > `grid[0]` > `65% 50%`（预览以前漏了自由选框，画过人物框后两边聚焦点不同）。
      · `bg_zoom` 夹取范围四处统一 **0.5~3**（设计器滚轮、load、后端 patch、`cover_frame_vars`）。
      · 没有 `title_lines` 时按标点拆 `project.name` 并砍掉 `【主播名】` —— 预览也要拆（以前整串一行）。
    - 公式与净化都在 `cover.rs` 且**只此一份**：`cover_blur_css(scale)`（基准 28px、
      放大 `1+px/150` 把模糊边缘推出画面）、`sanitize_hex_color`（**颜色直接进 HTML/CSS，
      必须净化**，非法值退 `#000000`）、`cover_bg_mode`（只有 `blur` 走模糊，其余一律 `color`，
      老配置里的 `frame` 也当纯色）。前端 `CoverDesigner.vue` 的 `coverBlurCss` 是镜像，
      改一边要改另一边。
    - **出片几何**（`cmd_cover`）：模板里 `html.export` 让 `#page` 不缩放、左上角对齐视口原点，
      同时 `Emulation.setDeviceMetricsOverride(1920×1080)` 把视口**撑满画布**，
      抓图用 `clip: (0,0,1920,1080)`。**别再退回 `clip: None`**：headless 实测视口只有
      1904×929，16:9 页面被居中 → 成片变 1904×929 且左右两条黑边（老 bug，已修）；
      `#toolbar`（右下角 ↻ 预览按钮）在 `html.export` 下 `display:none`，
      否则会被连按钮一起抓进成片。
  - **贴纸也要对齐最近的字幕（Step 2b1，用户要求：「表情贴纸也需要对齐最近的字幕」）**：
    `process/fx.rs::align_stickers_to_subtitles` —— 起点吸到**最近那条字幕**的开始、终点吸到它的结束
    （离最近字幕 > `STICKER_SNAP_MAX_SECS`(2.0s) 的不动：纯视觉反应没有台词可依附）。
    **必须排在 2b 落点之前**（落点按贴纸时间窗口算相机/字幕/其它贴纸障碍，时间挪了就得重算），
    2c 的尾帧自适应还能在它之后往后接一句。`end - time` 是"停留"时长，入场/上浮/淡出
    （0.35+0.4+0.3s）另外叠加，所以对齐到一条 1s 的字幕也不会把动画截断。
    LLM 落盘前也跑一次（`llm/mod.rs` Step 4b），编辑器里看到的就已是对齐后的时间。
    ⚠️ **手工放的事件不受后处理改写**（用户报过「调整贴纸的结束时间没有成功」）：事件带
    `manual: bool`（`Sticker` / `Camera` 两个变体有，serde `skip_serializing_if = "is_false"`，老工程零迁移）
    时，`align_stickers_to_subtitles` / Step 2c 尾帧延长 / `align_pip_shots` 三处**全部跳过**
    （`Event::is_time_manual()`），**`cap_events` 的数量抽稀也跳过**（见上面「手工放的事件」那条）。
    编辑器里改过这条事件时间的动作（体拖 / 左右缘 / 面板开始·结束 / 设为入·出点）、**新建**与
    **复制**都会自动 `manual = true`；面板给一枚「跟随字幕」清标记，下一次后处理恢复吸附。
    没有这个标记的话，用户改完 1 秒后自动后处理就把值吸回去了 —— 看上去就是"改了没生效"。
  - **贴纸落点推导（`crates/vtrim-polish/src/process/placement.rs`，Step 2b）**：
    帧坐标固定 1920×1080（模板 Canvas），贴纸固定 280×280，`ev.x/ev.y` 是**盒子左上角**
    百分比（模板 DOM/canvas 与 `engine.js` 都按左上角消费，模板再钳制到 `[32,1608]×[32,768]`）。
    障碍 = ① 人物 `layout.liver`、② 弹幕区域 `layout.danmaku_area`（两者都在**源帧**坐标里
    框选，出片时视频整体被运镜缩放 → 先按该贴纸可见窗口的相机状态投到输出帧，zoom 取
    模板 Canvas 路径的 1.5/1.8 × 瞬时特效预留 1.10，窗口内的缩放过渡两端都算）、
    ③ 字幕带（Canvas `cy=960` 与 DOM `bottom:80px` 两条路径取并集）、
    ④ 真实弹幕带（只有 `data/danmaku.json` 非空时才是障碍，按模板 `y=(d.y/1920)*540` 映射）、
    ⑤ 同刻已落位的其他贴纸（互不相压）。
    **落点规则**：候选 = 16px 网格 + 各障碍边缘的「临界位」（窄缝里也能刚好放下）；
    先取 clearance（到最近障碍/帧边的像素距离）最高的一档（×0.9），在该档里按「盒心到帧心
    距离」排序取前 8 个，再用 seed（`fract(idx*3.7 + seg.start*1.3)`）伪随机挑一个 ——
    即**安全区中心而不是边缘**，且逐条有变化。帧内实在没有安全位时退化为「重叠代价最小」
    并在日志里告警；最小余量 < 20px 时也会提示「选框贴得太近」。
    **`layout.sticker_area` 已降级**：`grid` 完全不参与落点（会打日志说明）；
    手写的自由选框只作**偏好**（里面有安全位就优先用，没有就自动放弃）。
    只读诊断（扫真实工程看落点，不写文件）：
    `VT_PROBE_ROOT=<media 目录> cargo test -p vtrim-polish --lib diagnose_sticker_placement -- --ignored --nocapture`
  - **任务完成后的刷新必须静默**（别让页面看起来像浏览器刷新）：`PolishView.load({silent:true})`
    不置 `loading`、不卸载编辑器 —— 非静默时 `v-if="loading"` 会把整个编辑器换成骨架屏，
    编辑器一卸载，预览视频、`selectRegion` 与滚动位置全丢。需要重新从磁盘取数的子组件
    各自用 `:key` 计数重挂载：`TimelineEditor`=`segKey`（LLM 剪辑/初始化/去静音）、
    `CoverDesigner`=`coverKey`（渲染导出）、`PolishPreview`=`previewKey`（任何任务完成）。
  - ⚠ **「读取失败」不许伪装成正常状态，也不许在基线不可信时写盘**（2026-09-16，一批"静默丢改动"的根因）。
    同一类 bug 在精修链路上出现过四处，这几条一起守：
    - **读失败 → 可见 + 暂停写入**：`TimelineEditor::loadData` 直接调 `api()`（**不再用 `apiOr` 兜底**），
      失败只置 `loadError`、**不动现有数组也不清脏标记**；`flushSave` 开头 `if (loadError.value) return`。
      没有这道闸，读失败会被当成"空时间轴"，用户拖一下就把 `segments: []` PUT 回去 —— 磁盘上的分段/字幕
      就这么没了。横幅 + 「重试」在 `tl-head` 下面（**不能放进 Teleport 的属性面板**：挂载点解析不到
      就整块不渲染）。`saveText` 里 `loadError` 必须优先于 `saving`/`ok` —— 这时写着"已保存"是骗人的。
    - **`/api/polish/subtitles` 的 404 是空态、不是错误**：`clip.srt` 与 `data/subtitles.json` 都不存在时
      服务端就返回 404「暂无字幕文件」，没转写的片段本来就是这样。**只有非 404 才算读失败** ——
      否则每个没有字幕的片段都会挂一条"读取失败，已暂停保存"，还把保存一起锁掉。
    - **`reload()` 不许静默丢弃**：老写法 `if (drag || saving || saveTimer) return` 会在"后处理跑完要同步
      结果"时被 700ms 防抖窗口挡掉 —— 编辑器留着后处理**之前**的值，紧接着 `flushSave` 又把旧值 PUT 回去，
      把刚算出的贴纸落点/锚点坐标整个顶掉，而且没人重试。现在：忙 → 置 `reloadQueued` 排队，由 `flushSave`
      的 `finally` 里 `drainQueuedReload()` 补做；真要跑时若还有没落盘的改动，**先落盘再重载**（用户的改动
      不丢；被顶掉的那次后处理改写由渲染前的 `process` 重算回来）。重载期间再收到请求，`finally` 里再跑一轮。
    - **换片段先清空**：`watch(() => props.dir)` 里 `commitEdit` 之后必须 `resetTimelineState()` 再 `loadData()`。
      不清空的话，新片段读取失败时数组还留着**上一个片段**的分段/字幕，用户一次编辑就把它们写进新片段目录
      （跨片段串写）。清空 + `loadError` 拦写，两道一起才安全。
    - 配套的**可见性**：`PolishView` 页面级读取（`fetchProject`）失败要记 `cfgError` —— 否则 `ready=false`
      会渲染成「未初始化精修项目」+ 一颗「初始化」按钮，用户会把**已存在**的工程当成没初始化再点一次；
      但**要带 `!ready` 门控**，已经有数据时的静默刷新失败只加一条横幅、绝不卸载编辑器（见上一条）。
      切片列表（`clipsError`）与贴纸来源（`stickerSourcesError`）同理：失败保留上一次的列表 + 错误条，
      **不能退化成"还没有可精修的切片"/"贴纸库为空"**（假空态让人去查库而不是查服务）。
    - `CoverDesigner` 的 `configLoaded` 是**写入门闸**（读失败时拿默认值写回会把磁盘配置清成默认），
      所以它一停在 false，面板里所有改动就静默不保存了 —— 读失败必须出错误条 + 「重试」，保存失败用
      与「保存中…」同处的常驻文字（`.save-err`），而不是 600ms 防抖下会刷屏、一闪就没的 toast。
    - **误删要有退路**：删除是"双击即生效 → 700ms 落盘 → 随后自动后处理"，点完就不可逆。所以
      `TimelineEditor` 保留**最近一次删除**（`lastDeleted`，15 秒）给头部一枚「撤销删除」；事件按
      **分段对象本身**认位（后处理会整体改写 segments，下标会漂）。字幕/屏蔽/事件双击即删不再弹确认，
      只有**分段**保留确认（连带全部事件，破坏面大一个量级）。
    验证这套东西不能靠真后端（读失败路径没法自然触发）：把 `webui/dist` 拷一份、在 `index.html` 里
    于应用模块**之前**注入一个拦截 `fetch` 的 mock（页面/时间轴带 `preview=1`、`CoverDesigner` 不带，
    据此把两者的读失败分开），再用 `window.__MOCK.fail` 按场景把某个接口打成 500，就能逐条验
    "横幅出现 / 没有假空态 / 读失败期间一个 PUT 都不发 / 重试后恢复"。
  - **模型管理与模型路径（别再各写一套）**：模型管理页的「已安装」与运行时**同源** ——
    `vtrim_core::asr::local::resolve_model_dir`（配置 `local_model_dir` → 用户目录 `~/.vtrim/models`
    → 程序目录 `<exe>/.vtrim/models`）是唯一真源，`download::model_install()` 在其之上再校验注册表
    marker（`tokens.txt` / `tokenizer/tokenizer_config.json`，**不是** `dir.exists()`）。接口
    `/api/models` 回 `{name, installed, dir, source, source_label}`，界面显示实际路径与来源。
    ⚠️ 曾经 `/api/models` 只查程序目录 `dir.exists()`，而 qwen3-asr 放在用户目录 →
    界面显示"未下载"却一直在用它（用户报过）；列表也改成由注册表 `download::model_names()` 给出，
    别再手写模型数组。`vtrim download` / `POST /api/models/download` 仍然装到**程序目录**（用户显式指定）。
    ⚠️ **`download::ensure_one_model`（转写前的自动守卫）也必须走 `model_install`**，
    不要再拼 `vtrim_home()`：曾经它写程序目录、而运行时读用户目录，后果有两个 ——
    ① 同一份模型被下载两次（程序目录那份永远不会被读）；
    ② **同音纠错的 `lexicon.txt` + `replace.fst` 下到程序目录，而 `create_recognizer` 读的是用户目录那份
    → 同音纠错从未生效**（日志一直是"未启用"）。
    同音纠错的开关就是"这对文件在不在被读的目录里"：`lexicon.txt` 是**字→拼音表**、
    `replace.fst` 是 sherpa 官方规则集，**都不适合塞自定义热词**（要工具链编译）。
    hybrid 下它必须挂在 **qwen3** 的 recognizer 上（`apply_homophone_replacer` 两处都调）——
    只挂 SenseVoice 等于只改了字轴，成片字幕文本一点不变。
    ⚠️ **但别以为它有用**：实测（`diagnose_hr_effect`，5 分钟真实素材、30 个窗口）
    **HR 开与关的输出逐字相同** —— SenseVoice 字轴 299 字完全一致、qwen3 30 个窗口文本
    0 个变化。也就是说这套官方规则在这类直播口语上**没有命中任何一条**，
    修 A5 的真正价值是"不再重复下载模型"和"配置说启用就真的启用"，不是字准确率。
    留着它的理由只有一个：数字/单位/多音字较多的素材它可能才生效，而代价为 0。
  - 设置页：全局配置表单/源码双模式 + LLM 测试 + 模型管理（`ModelList`）+ **LLM 用量**
    （`components/LlmUsage.vue`，admin 卡：总量 + 按天/链路/项目/模型分组，数据来自
    `GET /api/admin/usage`）+ 项目设置
    （项目配置表单/源码，ASR 模式等走 `PUT /api/project-data/config {patch}`）。
- **认证（P1 已落地）**：除 `/api/health`、`/api/auth/login` 与 `/api/auth/register` 外**所有接口都要求会话**，
  含 loopback；**没有免登录分支、没有来源地址判定**。单人部署＝只有一个 admin 账号的部署：
  - 首次启动生成 `admin` + 随机初始密码，日志打印一次并写入
    `<data_dir>/initial-admin-password.txt`（首次登录成功后自动删除）；重启不重新生成。
  - 会话在 SQLite `sessions` 表，`vtrim_sid` cookie 为 **HttpOnly**，7 天滑动续期；
    写请求必须带 `X-CSRF-Token`（= `vtrim_csrf` cookie，双提交），缺失/不匹配 → **403**；
    未登录 → **401**（前端 `client.js` 统一跳登录并记回跳）。
  - 登录失败 5 次（同用户名+同 IP，取 `ConnectInfo` 对端地址，**不信任 `X-Forwarded-For`**）
    → **429**，窗口 15 分钟；账号禁用 → 403/401 文案直出。
  - 启动预检：`data_dir` 不可写 → 启动失败（身份不可缺省，绝不退化成内存模式）；
    `vtrim.lock` 单实例保护（独占句柄 + pid 存活判定，`--force` 只在确认残留时用）；
    `--host 0.0.0.0 --allow-remote` 且 `must_change_password=1` → 拒绝启动。
  - `/api/health` 只回能力位与 `accounts / must_change_password / listening / tls_terminated /
    registration_enabled`，**不含任何路径**（它无鉴权）。
  - **邀请码自助注册（已落地）**：`POST /api/auth/register {username, password, invite}`（**公开**，
    与登录同层，无 CSRF/会话要求）。三重条件：`[auth] registration_enabled` 开着 + 邀请码有效 +
    用户名/密码合规；成功后**注册即登录**（下发会话 cookie，`role` 固定 `user`，永不自助成 admin）。
    失败一律 **403**（不区分"码不对/码用过"，防止被拿来枚举），限流按 IP（key `register|<ip>`，
    与登录的限流分开），5 次锁 15 分钟。
    - 邀请码 = 一次性 token `vtrim_inv_<43 字符 base64url>`（`service::auth::new_invite_code`）。
      **明文直接落库**（用户要求：管理员要能重复查看、复制再发 —— 与 `vtrim ingest token`"只见一次"
      的规矩**不同**）：库里没有 selector/哈希那一套了，注册校验就是一次明文查表（`token` 唯一索引），
      因此**别再退回"只存哈希"**，那会让管理员看不到码。
      **一个码只能绑一个账号**：消费走 `Db::register_with_invite`（同一把锁里 `UPDATE ... WHERE
      used_at IS NULL AND revoked=0 AND 未过期` 再 INSERT），用过即写 `used_by/used_at`，
      列表里显示「已使用 · 绑定 谁 · 时间」；失败不会留下半个账号、也不会烧掉码。
    - 生成/查看/撤销：CLI `vtrim user invite [--note 给谁] [--days N] [--count N] [--disable]` /
      `vtrim user invites` / `vtrim user invite-revoke <id|selector>`；**生成时自动打开注册开关**，
      关闸用 `--disable`（已有码保留）。UI：管理员**设置页 →「注册与邀请码」**卡
      （`components/InviteManager.vue`：开关 / 生成 / **列表里明文一直可见 + 一键复制** /
      未使用·已使用 chip / 绑给了谁 / 撤销；行 `flex-wrap` 保证窄窗口换行不撑破卡片）。
    - HTTP（仅 admin）：`GET /api/admin/invites`（**回明文** + `used` / `used_by_name`）、
      `POST /api/admin/invites`（`{note?, days?, count?}` → `codes`）、
      `DELETE /api/admin/invites/:id`（撤销；已注册账号不受影响）、
      `PUT /api/admin/invites/registration {enabled}`。
    - 前端：`#/register`（`RegisterView.vue`，与登录页同壳外样式，不套 shell；`/api/health`
      的 `registration_enabled` 为 false 时直接显示"未开放"而不是给表单），登录页只在开关打开时
      露出「没有账号？用邀请码注册」入口。
    - 账本 **v5**：`invites` 表（`token` 明文唯一 / note / created_by / expires_at / used_by / used_at /
      revoked）。v4（只存 selector+哈希）的老行在 v5 迁移里重建表并标注为"旧邀请码（明文不可查看）"，
      `used_by/used_at` 历史保留。
      「消费码 + 建号」是**一次原子操作**（`Db::register_with_invite`，同一个锁里 `UPDATE ... WHERE
      used_at IS NULL AND revoked=0 AND 未过期` 再 INSERT），两个人抢同一个码只有一个人能成；
      失败不会留下半个账号。用户名查重用 `find_user_ci`（**大小写不敏感**，挡 `Admin` 冒充 `admin`）。
- **项目托管与隔离（P2 已落地）**：
  - 归属由 SQLite `projects` 表决定；`/api/projects` 只回本人项目，
    `DELETE /api/projects/:id`（**只注销登记，项目目录里零文件改动**）、
    `DELETE /api/projects/:id/source`（删源录播，保留产物，唯一的手动释放空间手段）。
  - ⚠️ **「移除项目」不碰任何文件**（用户拍板）：`vtrim.toml`、媒体、`output/` 产物全留着，
    重新导入同一个目录即可再绑回来（`create_local_project` 见到已有 `vtrim.toml`
    **只登记、不覆盖**，所以手改过的工程设置不会丢；新登记会拿到**新 id**，
    项目目录内的产物都不含服务器侧 id，换 id 安全）。
    两个入口共用 `api.rs::unregister_project`（`DELETE /api/projects/:id` 与过渡别名
    `POST /project/delete`）—— 以前是两份拷贝、各删一次 `vtrim.toml`，别再写回两份。
  - ⚠️ **`persist::forget_project` 必须同时清 `last_config_file`**，少了这一步"移除"会被静默撤销：
    `server/mod.rs` 每次启动都把"当前工程"写回 `server.toml` 的 `projects[]`，而
    `import_legacy_projects` 会把 `projects[]` 里**"配置文件还在、DB 没行"**的路径重新登记到
    admin 名下 —— 注销不删文件，所以那个配置一定还在，于是"移除当前在看的项目 → 重启一次路径
    回到列表 → 再重启一次项目自己回来"。以前没这问题纯粹是因为 `vtrim.toml` 被删了
    （`legacy_projects()` 有 `filter(|p| p.exists())`）。回归：
    `server::api::tests::unregister_keeps_every_file_and_can_rebind`
    + `server::persist::tests::forget_project_clears_the_remembered_last_project`。
  - **路径面归零**：所有 `/api/project-data/*` 用 `?project=<id>`；polish 用
    `{project, dir:"output/clips/…"}`（服务端把相对路径归一化成项目内绝对路径）。
    旧的 `?config=<路径>` 只在"该路径已登记且属于本人"时作为过渡兼容（未登记一律 404）。
  - `vtrim-core/src/tenant.rs` 是唯一拼接入口：拒绝绝对路径/UNC/`C:foo`/`..`/符号链接逃逸。
  - `/api/media/<project_id>/<项目内相对路径>`；旧写法（绝对路径）也只在本项目内放行。
  - `/api/admin/*`（config/models/doctor/llm test/**users**）仅 admin；旧路径作为过渡别名挂在同一层。
    `GET /api/admin/users` 只回展示字段（id/username/role/disabled/quota_bytes/created_at），**不含口令哈希**，
    供"待分配池指派给谁"下拉使用。
  - `/api/fs/list` 仅 admin，**可以浏览服务器任意目录**（`{dir}` 绝对路径，与单用户时代一致；
    也兼容 `{root, path}` 相对写法）。这是"服务器上已有文件"这条 admin-only 来源，**永不接受上传**；
    `[storage] local_roots` 只是前端下拉的**快捷入口**，不再是硬限制。
    **进入下一级用 `{dir, enter:"子目录名"}`，路径由服务端拼**：前端自己拼 `dir + '/' + name`
    会把 canonicalize 的 `\\?\F:\a` 拼成 `\\?\F:\a/b`（Windows ERROR_INVALID_NAME 123）。
    因此所有出 JSON 的路径都过 `vtrim_core::path::display()`（剥掉 `\\?\`/`\\?\UNC\`），
    前端拿到的永远是 `F:\a\b` 这种可回传、可拼 URL 的形式。
  - 管理员「首页 →「新增项目」→ 服务器文件」：`POST /api/projects {source:{kind:"local", dir, input, danmaku, output_dir}}`
    （`dir` 绝对路径；非管理员只能用自己的租户根，且 `/api/fs/list` 与 `kind:"local"` 一律 403）；
    **目标目录已有 `vtrim.toml` 时只登记/打开、不覆盖**（保护手改过的工程设置）。
  - 任务带 `owner_id`：`/api/jobs`、`/api/jobs/:id`、`/api/jobs/:id/events`、canc、`/api/stream`
    全部按归属过滤，他人任务 404（SSE 在服务端过滤）。
  - 前端作用域是项目 id：`/project/:pid`，`api/scope.js` 是唯一拼 `project=<id>` 的地方。
- **wgpu 真渲染：暂不使用，且明确不在考虑范围内（用户决定，2026-09）。**
  改动渲染/运镜/特效语义时**不要为它做任何同步**，也不用去修它的测试。理由与边界：
  - 默认构建不含它：`vtrim-polish` 是 `default = ["chrome-render"]`，`wgpu-render` 要显式开
    （`crates/vtrim-polish/Cargo.toml` 自己的注释：「chrome-render 为默认出片路径」）。
  - 涉及的文件：`vtrim-polish/src/render/native/`（`#[cfg(feature="wgpu-render")]`）、
    `vtrim-render-core` 的 `draw/effects/render/composite/sprite/gpu`（`#[cfg(feature="wgpu")]`）、
    `server/api.rs` 的单帧预览接口（同 feature）、前端按 `/api/health.wgpu_render` 隐藏入口。
  - **`vtrim-render-core/timeline.rs` 不在 feature 门后，但它的唯一非测试消费者就是 wgpu**
    （原生走 `render/native/mod.rs`，浏览器走 `vtrim-wasm`——它启用了 `vtrim-render-core/wgpu`，
    产物在 `webui/src/wasm/`，由 `PolishPreview.vue` 的「真渲染」按钮使用，而那个按钮同样按
    `wgpu_render` 能力位隐藏，所以默认构建下整条路都不可达）。默认构建里 `timeline.rs` 照常编译、
    但没人在用：**它是一份平行实现，不是行为的事实来源。**
  - ⚠ 因此：`timeline.rs` 里那套求值与 `webui/src/polish/engine.js` 长得像，但**只有后者代表预览**；
    出片的事实来源是 `templates/index.html` 的 canvas 路径。改语义时同步
    **`templates/index.html`（权威）+ `engine.js` + `PolishPreview.vue`** 三处即可。
    运镜的聚焦/位移/画中画为了不留半截实现，`timeline.rs` 里也写了（并配了单测），
    **但以后不必再重复这件事**。
  - `cargo test -p vtrim-render-core --lib` 只跑 `timeline.rs`+`easing.rs` 的单测；
    `draw.rs` 等要 `--features wgpu` 才编译。**别把它当成成片行为的回归网。**
  - 已知且接受的副作用：`crates/vtrim-polish/tests/` 下 4 个集成测试
    （`text_render`/`text_visual`/`text_layout`/`text_dump`）引用 `vtrim_render_core::composite`，
    而该模块只在 `wgpu` feature 下存在 → **默认构建下 `cargo check --all-targets` 必然报这 4 个错**。
    这是既有现象，不要去"修"它（除非哪天决定把 wgpu 整块删掉）。
- **录播姬采集（P3 已落地）**：
  - `vtrim ingest token [--disable]` 生成 webhook token（明文只显示一次，配置里只存 scrypt 哈希）
    并开启 `[ingest] webhook_enabled`；`vtrim ingest status` 看配置概览。
  - **WebUI 侧（设置页「录播姬（采集）」卡 = `components/RecorderConfig.vue`）**：开关
    （`webhook_enabled`）+ `root`/`api_base`/`api_user`/`api_pass`/`allow_ips`/`auto_pipeline` 表单 +
    一键生成/轮换 token + 「测试连接」。开关关闭时**只留开关与一句说明**，打开才展开表单（用户要求）。
    对应三个端点，都在 `require_admin` 层下：
    - `GET /api/admin/ingest` → `config_view_json()`：**只回 `api_pass_set` 布尔，绝不回 `api_pass`
      明文或 `token_hash`**（表单回填不需要它们；要看原文走 `GET /api/config` 的 `global_raw`，那是
      admin 专用且是源码编辑器 round-trip 的前提）。
    - `PUT /api/admin/ingest` → 校验在纯函数 `build_ingest_patch`（可单测、不碰真实全局配置）。
      语义：**键缺席 = 不改**、给空串/空数组 = 清空（`api_pass` 缺席即"不改密码"）。
      `allow_ips` 只收 IP 字面量（运行时是**精确字符串相等**，放行 `192.168.1.0/24` 只会让来源被
      静默拦掉）；`api_base` 非空必须带 scheme；`root` 不可达**不拒绝**、只回 `warnings`
      （网络盘可能只是暂时没挂上）。审计只记**键名**（值里可能有口令）。
    - `POST /api/admin/ingest/token {action}` → `generate` 明文**只在这一次响应里**回，`disable`
      只关开关并保留 token。
  - `POST /api/ingest/bililive/<token>`：**自带 token 鉴权、不要求会话**（录播姬只能填 URL）；
    只做 验 token → `EventId` 去重 → 落库，然后**立即 2xx**；`allow_ips` 白名单可选；
    `EventData` 用大小写不敏感的多别名取值，未知事件类型忽略但记事件表。
  - `ingest_sessions/ingest_files/ingest_events/channel_map` 四张表；
    `GET /api/ingest/records`（本人；admin `?all=1`/`?unassigned=1`）、`GET/PUT /api/channel-map`、
    `POST /api/ingest/retry`、`POST /api/admin/ingest/assign`、`POST /api/admin/ingest/scan`、
    `GET /api/admin/ingest`。
  - 入库 worker（串行＝入库槽 1）：完结判定（SessionEnded + 全部分片闭环或空闲窗口）→
    水位/配额预检 → 落位（同盘 rename / 跨盘 copy+校验+删源）→ 多分片
    `ffmpeg -f concat -c copy` 合并为 `source.mp4`（失败回退重编码，原始分片保留）→
    弹幕（`.xml`/`.ass`/`.srt`）一并搬入 → 建项目 → 按 `auto_pipeline` 自动跑管线；
    失败清理半成品目录并写 `state=failed` + 错误文案。
  - ⚠️ **会话状态机只有 `closing` 会被入库**（`ingest_worker::is_finalized()` 只认它）：
    - **指派/导入必须把待分配会话推到 `closing`**（`Db::assign_ingest_session` 里的
      `CASE WHEN state='unassigned' THEN 'closing'`）。曾经写成 `'finalized'` → 「指派并入库」
      只改归属、**永远不进库**（实测停在 finalized、`project_id` 恒空，得再点「重试」）。
    - `'finalized'` = **项目已建好**（`Db::set_ingest_project` 写），别当"已指派"用。
    - 扫描（`register_scan_session`）每两轮就会重走一遍，**不能把已有归属的会话打回 `unassigned`**
      （那会和 assign 来回震荡，手动指派过的会话被反复踢回待分配池）→ 改状态前先看 `owner_id`。
  - **房间号必须从路径里解析**（`service::recorder::room_id_from_dir_name` /
    `room_id_from_rel_path`），**不能拿整个目录名当房间号**：录播姬默认目录名是
    `<房间号>-<主播名>`（实测 `26320007-耶斯yeki`），而 `channel_map` 存的是 `26320007`
    —— 直接比永远不匹配，扫到的真实录像全被判成"没登记"掉进待分配池。两个用它的地方：
    兜底扫描的归属、`GET /api/rooms/:id/recordings` 的分组。
  - **首页「我登记的直播间」每行可展开看录像**（`WorkbenchView`，用户要求"列表挂在房间下面"）：
    `GET /api/rooms` 每行带 `recordings{files,bytes,latest}` 汇总（一次只读 walk + **8s TTL 缓存**
    `ingest_worker::list_recordings`，网络盘上很贵，别去掉缓存）；展开才调
    `GET /api/rooms/:id/recordings`（**只读**，不登记会话）列明细；
    `POST /api/rooms/:id/recordings/import {rel}` 导入**单个**文件 —— 会话命名与兜底扫描同一套
    （`scan-<目录名>-<文件名主干>`）所以不会被扫描重复登记，创建后显式置 `closing` +
    `notify` worker（该文件已同时记 opened/closed → 立刻可入库，**不等空闲窗口**）。
    两道校验别删：文件必须**属于该房间**（`room_id_from_rel_path(rel) == room_id`，否则用户能拿
    任意 rel 把别人的录像导进自己项目）、路径过 `resolve_recorder_path`（囚笼 + 必须存在）。
  - 兜底扫描：`watch_interval_mins`（0=关闭）或手动 `POST /api/admin/ingest/scan`，
    文件"两轮大小+mtime 不变"才登记，覆盖漏推送/录播姬重启/历史录播导入；
    未映射房间进"待分配池"（`state=unassigned`，仅 admin 指派）。
  - 前端：`#/ingest` 入库页（记录 + 房间映射 + 待分配 + 扫描）与总览页「录制导入（最近）」，
    `/api/health.ingest_ready` 决定是否显示配置引导卡。
- API 都在 `/api/*`：未注册路径返回 **404 + JSON**（`{"ok":false,"error":"未知接口: …"}`），
  不会回退成 SPA HTML（否则前端会把 200 HTML 解析成 `null` 静默失败）。
- **访问日志（`server/access_log.rs`）**：`api::router()` 最外层挂了 `from_fn` 中间件，
  **每次前端交互都留痕** —— `方法 路径 → 状态 (耗时ms) user=<谁>`；状态 ≥400 时额外读
  响应体（仅 JSON，≤64KB，读完原样拼回）取出服务端的 `error` 文案，≥500 记 error、
  ≥400 记 warn。**别退回"只打成功"或"只打错误"**：handler 的 `err(...)` 与内联
  `{"ok":false,...}` 从来不自己打日志，前端又有大量 `.catch(() => [])`，
  两边一叠加失败就彻底消失了。
  - 操作者来自 `require_user`/`require_admin` 塞进**响应扩展**的 `Actor`
    （请求扩展里的 `CurrentUser` 挂在最外层的中间件读不到）；403 也带操作者。
  - 高频接口成功路径降级为 `debug` 免得刷屏：`/api/media/*`、`/api/health`、`/api/stream`、
    `GET /api/jobs`、`/api/admin/logs/events`（出错仍 warn/error）。要逐条看：
    `RUST_LOG=vtrim=debug`。
  - **不记录请求体**（登录/注册含口令）。
- **运行日志（`server/logbus.rs` + 设置页「运行日志」卡）**：
  `LogBusLayer` 挂在 `main.rs` 的 tracing subscriber 上（`registry + EnvFilter + fmt + logbus`），
  把每条事件同时写进 ①2000 条内存环形缓冲 ②broadcast ③文件
  `<data_dir>/logs/vtrim-YYYYMMDD.log`（`server::run` 解析完 `--data-dir` 后立刻
  `enable_file_log()`，之后才有文件；`vtrim doctor` 显示路径）。
  - 接口：`GET /api/admin/logs?after=<seq>&limit=`（历史/断线补齐）与
    `GET /api/admin/logs/events?after=<seq>`（SSE 实时，`Lagged` 合成一条 warn，不报失败），
    **都在 admin 路由下（仅管理员）**：日志含其他用户的工程路径与任务输出。
  - 前端：`webui/src/stores/log.js`（服务端 SSE + 前端侧条目合成一条时间线，按 `seq`
    去重、cap 2000、**只在设置页挂载时连 SSE**）+ `components/LogConsole.vue`
    （级别档 全部/调试/信息/警告/错误、关键字、自动滚动、清空、连接状态）。
    改前端样式时注意：`.log-*` 是 scoped 新类，颜色一律走 tokens。
  - 任务日志桥接：`Job::bridge_tracing` 把 `log()/阶段起止/完成/失败/取消` 也发给 tracing
    （**`Progress` 不桥接**，1% 一条会把日志冲掉）。

### 字体库（`resources/fonts/`）—— 用户丢字体进来就能选

`vtrim_core::fonts` + `path::font_dir()`：程序级字体库固定在 **`<程序目录>/resources/fonts/`**
（与 `.vtrim/`、`stickers/` 同级；`config init` 与 `server` 启动时都会 `ensure_dir`）。
往里丢 `.ttf` / `.otf` / `.ttc` 就会出现在精修页的两个下拉里：**字幕字体**（`polish.toml` 的
`[style.subtitle] font`）与**封面字体**（`[cover] font`）。发行包**仍然不带任何字体**（授权原因，
见 README「字体」）——这个目录是用户自己填的，打包脚本只建空目录 + 一份说明。

- **id = 文件名去扩展名**（与贴纸库 `name` 同规矩）—— 配置里存的就是它；`family` 只是展示名
  （手写的最小 `name` 表解析，`ttf/otf/ttc` 都认，按 `(路径, mtime, 体积)` 缓存，别每次请求重读几十 MB）。
  **空串 = 系统字体栈**（`LXGW WenKai → Noto Sans SC → Microsoft YaHei`），老工程就是这一档。
- ⚠️ **别改成"把字体拷进项目"**（贴纸那套 `copy_sticker_from_local` 是另一回事）：出片页面由
  渲染服务器（`render/server.rs`，ServeDir 根 = 项目目录）端给 headless Chrome，所以库里字体
  必须有一条自己的路由 —— `/vt-fonts/:name`（只认库目录下的单段文件名）。
  **两条建 router 的路都要挂**（`start_file_server` 提帧/封面截图 + `start_with` 预览/CLI 出片），
  漏一条就是"预览有、成片没有"。
- **浏览器只认系统已装字体**，所以选了库内字体就**必须注入 `@font-face`**：
  `server::font_face_css(...)` 拼出来的片段进两个模板的 `<!-- __FONT_FACE__ -->` 占位
  （`index.html` 由 `build_html` 注入字幕字体、`cover.html` 由 `cover.rs` 的
  `cover_font_css` 注入封面字体，**两条 replace 链都要改**）。
  **声明的字族名 = 配置里存的那串 id**，所以 `index.html` 里
  `.cap{font-family:"<style.subtitle.font>",...}` **一行都不用改**；`cover.html` 的字体栈改成了
  `__FONT_FAMILY__` 占位（空串时替换回系统栈）。库外/解析不到的值**什么都不注入**——
  注入了就是一条 404 的 @font-face，老工程的 `font = "LXGW WenKai"` 行为必须保持原样。
- **WebUI 预览另有一份**：页面 DOM 只认注册过的 `FontFace`，所以 `polish/fonts.js::ensureFontFace`
  在预览（`PolishPreview` 字幕、`CoverDesigner` 标题）里把同一份字节注册一遍；
  wasm 真渲染的字体字节仍走 `/api/polish/font`（候选链里已加库目录）。
- **接口**：`GET /api/polish/fonts`（只读，非 admin 可读，与 `/polish/sticker-library` 同规矩）
  → `{ok, root, fonts:[{id,file,family,bytes}]}`；`root` 过 `path::display` 出 JSON。
  **没有上传入口**（字体由管理员自己丢进目录，也避开"全站不接受上传"那条规矩的例外面）。
  保存只动两个已有端点：`PUT /api/polish/project` 的 `subtitle_font`、`POST /api/polish/cover-config`
  的 `cover.font`。字体**只影响预览与渲染**（`process` 的贴纸/运镜重算不读它）——
  **保存即生效，不要顺手起 `process` 任务**。
- 回归钉子：`vtrim-core` 的 `fonts::tests`（扫描/解析/越权）、`vtrim-polish` 的
  `render::server::tests`（注入/不注入、模板占位符还在）。

### 渲染导出与封面（`polish_render` / `polish_cover`）

**这是一条子进程链路，出过两次"任务永远运行中、日志里一行都没有"的事故，改之前先读完这节。**

- **输出必须落文件，不能用管道**：`api.rs::run_polish_child` 把子进程 stdout/stderr 重定向到
  `<data_dir>/logs/render-<job_id>.log`，再用 `follow_render_output` 边跑边读。
  ⚠ 绝对不要改回 `Command::output()`/管道：Chrome 与 ffmpeg 是**孙进程**，它们会继承 stdout
  句柄 —— 只要还有一个活着，管道就永远不 EOF，`output()` 一直等下去。实测子进程早已带着
  `Render error: evDur is not defined` 退出，任务却卡了半小时、错误全被闷在缓冲区里。
- **分类规则**：`frames A..B: NN%` / `chunk#N: ...%` → `jobs.progress()`（按**最小值**聚合：
  最慢的分块决定整体）；`-- Xxx --`、`FINISHED:`、`Cover saved:`、`✂ 已导出去除`、`⏱` 等里程碑
  → `jobs.log(info)`（进运行日志面板与文件）；其余行 → `debug`（只挂在任务卡上，不刷日志）。
- **取消 = 杀进程树**：子进程是另一个进程，看不到进程内标志。`render_proc::kill_tree(pid)`
  在取消时 `taskkill /F /T`（含 Chrome/ffmpeg），父进程再清 `_chunk_*` / `_temp_*`。
- **启动清扫孤儿**：每次渲染往 `<data_dir>/render-procs/<job_id>.pid` 登记 pid，
  `server::run` 启动时 `render_proc::sweep_orphans` 结束上一次遗留的渲染（**先核对命令行**
  再杀，pid 会被系统回收）。服务器重启带不走子进程 —— 实测两个渲染各留 3 个 Chrome 空转两小时，
  把后续渲染拖到根本跑不完。
- **卡死守卫**：页面侧 `capture_and_read` 记录 `__CAPTURE_PROGRESS` 的最后变化时间，
  超过 `VTEDIT_STALL_SECS`（默认 180s）报错退出；父进程另有"子进程 60s 无输出 → warn"。
  别再退回**无超时轮询**：那是"长时间无反应 + 日志空白"的直接来源。
- **分阶段打点**：`cmd_render` 每段一条 `⏱ 阶段 {:.1}s`，`render_chunk_sync` 把每块的
  **抽帧 / 渲染 / 编码**分开报。实测 clip-35（262s 片段，debug 构建）294s 里：起服务 16.1s、
  贴纸 GIF 40.8s、分块 210.7s（每块 抽帧 ~30s + 渲染 145~171s + 编码 8s）、音效混音 9.3s、
  合流 2.7s、屏蔽重编码 9.2s、封面 1.1s。出片慢先看这几条，别再猜。
- **带磁盘缓存的预计算**（都在 `<工程>/data/`，按「文件名 + mtime + 大小」失效）：
  - `loudness.json`（`render::measure_loudness_curve_cached`，出片页与混音共用一份）——
    预览服务的 HTML 是**每个请求**重建的，不缓存等于每刷一次页面烧一遍整段 ebur128；
    实测「起服务」16.1s → 0.1s。
  - `sticker_frames.json`（`prepare_sticker_data` 的 GIF 解码结果）——6 个贴纸解 40.4s → 命中 1.67s
    （产物 95MB，断言过冷/热输出一致）。贴纸多一个就多一份解码，别把缓存去掉。
  - 诊断：`VT_POLISH_DIR=<项目> cargo test -p vtrim-polish --lib diagnose_sticker_cache -- --ignored --nocapture`
- **屏蔽区间导出走硬件编码**：`Ffmpeg::cut_ranges` 是整段重编码，用 `detect_encoder()`
  的结果（libx264 时 17s → NVENC 9s）。别写回硬编码。
- **模板里的工具函数必须放在脚本顶层**：`templates/index.html` 里 canvas 出片那一段是
  独立的 IIFE，看不到 `init()` 内部的东西。`evDur` 曾放在 `init()` 里、被 IIFE 的
  `buildIndex()` 调用 → 每次出片都抛 `evDur is not defined`（GSAP 预览路径却完全正常，
  所以只在出片时炸）。新增给两条路径共用的函数时，放顶层并留注释。
  **模板改完必须真跑一次出片验证**（预览看着正常 ≠ 出片正常）。
- **前端留痕纪律（别再让信息"静默消失"）**：
  - `api()`（`webui/src/api/client.js`）是唯一请求入口，失败统一 `console.warn` +
    写日志面板（≥500 记 error，其余 warn），成功只写 `debug`。
    可选 `{ label: '读取切片列表' }` 给失败配上业务上下文。
  - **读取类兜底用 `apiOr(path, fallback, label)`**，不要写裸 `api(...).catch(() => [])`：
    后者把"读取失败"伪装成"没有数据"，页面与日志里都看不出异常。包装器
    （`projectData.*` / `fetchSubtitles` / `fetchProject` / `fetchStickerSources`）
    都收一个 `label` 参数，视图侧传中文说明。
  - 非 API 的失败（裸 fetch、SSE、解析）用 `stores/log.js` 的 `addClient(level, text, source)`；
    高频可预期失败（弹幕文件缺失、字体回退、任务轮询重试）记 `debug`，不要刷 warn。
  - 不做 toast→日志桥（err toast 本来就可见，桥接只会和 `api()` 的记录重复）。
- 任务模型：**资源域互斥 + 三档并发闸门 + 前后台优先级**（不再是"全局同一时刻只跑一个"，
  也不是"所有任务共用一个并发数"）。**分档理由 / 三档成员清单 / 取舍 / 排查见
  `docs/job-queue.md`** —— 改档位或上限时那份文档要一起改。
  - `JobManager::create(owner, JobSpec { kind, message, resources, class, priority })` 立刻返回
    `JobHandle {id, queued, ..}`：本档名额没满 **且** 资源键与**在跑**的任务不相交 → 直接开跑；
    否则进 `Queued`，由调度器按 `(优先级, 创建顺序)` 放行。**冲突不再 409**（旧行为），
    响应带 `queued` 供前端提示「已排队」。
  - **三档（`JobClass`）按本地资源占用分，不按时长分** —— LLM 类任务墙钟很长但全程在等网络，
    不该跟渲染抢名额：

    | 档 | 名额 | 归入 |
    |---|---|---|
    | `Light` 轻活 | **不占闸门**（不限） | `polish_init` / `polish_process` / `polish_cover` |
    | `Medium` 略重 | `MEDIUM_CONCURRENT`(3) | `cut_one`、`stage_extract`、`stage_detect`、`correct`、`polish_llm`、`sticker_build`、`download_models` |
    | `Heavy` 重活 | `HEAVY_CONCURRENT`(2) | `stage_transcribe`、`stage_cut`/`process`/`pipeline`、`cut_all`、`polish_render`、`ingest_*` |

    两个上限是 `jobs.rs` 里的常量（与旧 `MAX_CONCURRENT_JOBS` 同一约定：要调就改常量），
    但集中在 `JobClass::cap()` 一处。
  - ⚠️ **档位豁免的只是闸门名额，"写同一批文件"的物理冲突照旧**：轻活撞上在跑任务的资源键
    仍然排队（有单测）。所以调度器里名额满 / 资源冲突都是 `continue` 而不是 `break` ——
    重活排满不该挡住后面的轻活/略重；同档 FIFO 由"名额满则本档后续全不满足条件"自然保持。
  - 分类规则只有两个函数，别在创建点各写一遍：`JobClass::for_pipeline(stages)`
    （含 `transcribe`/`process`/`cut` → 重活，否则略重）与 `JobClass::for_polish(kind)`
    （`render` 重活 / `llm` 略重 / 其余轻活）。
  - **优先级（`JobPriority`）只在同档内插队**：`.background()` 标在入库自动管线、模型下载、
    贴纸构建上，它们排在用户点按的任务后面；**不抢占**已经在跑的（半路掐断转写等于白跑）。
  - 每个创建点（8 处：pipeline / cut_all / cut_one / correct / download_models /
    polish_* / sticker_build / ingest_*）都必须先 `handle.wait_start()` 再干活 ——
    排队期间被取消时它返回 `Err`，**直接收工**（`jobs.complete()` 只为释放占位、唤醒后面排队的）。
  - `JobSummary` 带回 `class` / `class_cap` / `queue_position` / `priority`：任务行排队时在
    说明行写「重活任务（并行上限 2）· 同档第 2 位 · 轮到时自动开始」（`api/jobs.js::queueDetail`），
    状态 chip 的 title 用 `classTitle` 写档位/上限/来源。**细节写在说明行、不塞进 chip**：
    chip 是全应用共用样式，长文案会把窄抽屉里的行挤变形。
    `queue_position` 是**同档队列里的名次**，不是"还要等几个人"——卡在资源冲突上也照算，
    别把它呈现成剩余等待数。各页「已排队」toast 统一用 `QUEUED_HINT`，
    **别再写"前面那条任务结束后自动开始"**（排队现在也可能是本档名额满）。
  - ⚠ **前端任务列表与按钮门控纪律**（2026-09-16 一次补齐，下面每条都是"界面哑火"的来源）：
    - **请求必须有超时**：`api()` 默认兜一个松上限（10 min），`jobsApi` 自己传 **10s**。
      没有超时的 `fetch` 一旦卡住就永不 settle，而 `useJobs` 的去重 `inflight` 只由 `.finally` 清
      —— 一次卡住的 `/api/jobs` 会把任务列表与所有按作用域门控的按钮一起冻在最后一帧。
      用户症状正是**"队列加载不出来 + 剪切按钮一直灰"**，而日志里**一条 `/api/jobs` 都没有**
      （它在 `access_log.rs` 的 noisy 名单里只记 debug）。所以：新增轮询类接口自己给短超时；
      noisy 接口**慢过 `SLOW_MS`(1s) 照样记 info**，别再让慢请求静默。
    - **读失败 ≠ 没有任务**：`useJobs.load()` 失败时**保留上一份快照**并置 `lastError`，
      `JobList` 把「读取中 / 读取失败（带重试按钮）/ 真的没有任务」**分三态**显示。
      以前失败写 `jobs.value = []`，界面和"真的没有任务"长得一模一样（都是「暂无任务」）。
    - **门控要能被陈旧快照兜住**：`busyOn` 只是"提前告诉用户要等"，**不是正确性保证**
      （正确性由服务端的资源键排队兜）。`useJobs.stale`（快照超过 `SNAPSHOT_TTL_MS`=15s 没刷新）
      为真时，各视图必须把 `busyOn` 的结果降级成 `null` 放行 —— 否则一次读取故障就把页面按钮
      永久锁死。**别把门控当"不许点"**：宁可放行让服务端去排队。
    - **任务结束的刷新必须 `silent`**：`ProjectSlicesView` 的 `onJobDone` 曾漏了 `silent:true` ——
      非静默 `load()` 一置 `loading`，`v-if="loading"` 就把整面卡片墙换成骨架屏并
      **卸载 `ProjectClipsSection`** → `clipsRef` 变 null → 工具条三颗按钮（`:disabled="!clipsRef"`）
      全灰；慢盘上这一次要 3–4 秒，而它**每个任务结束都跑一次**。这就是用户报的
      "剪切完界面卡住、按钮变灰"。（同一条规矩见「任务完成后的刷新必须静默」那条。）
    - 这类刷新还要**单飞 + 尾随重刷**（`ProjectSlicesView.load` 的 `inflight`/`queuedReload`）：
      任务密集结束时会连着触发好几次，每次四个请求，慢盘上并发跑同一批既浪费又会让旧响应盖新数据。
  - **「剪切」按钮不禁用，忙时只改文案**（`ProjectClipsSection`）：`cut_one` 必持项目级的
    `p{pid}:out` + `p{pid}:idx`，按占用禁用它会让**所有**卡片的剪切一起灰、连点下一条都做不到，
    而服务端遇到占用本来就是排队。所以按钮常可点，忙时文案变「剪切（排队）」+ title 说明谁在占
    （`.uncut-card .foot` 因此加了 `flex-wrap`：多两个字在不换行的 flex 行里会把按钮挤变形）。
    `编辑` 仍然禁用（走同步接口写 events.json，撞 `p{pid}:out` 是 409，排队解决不了）；
    `预览` 只看有没有原片地址。**这条是用户拍板的，别改回禁用。**
  - **任务行的"卡住"提示**：`JobSummary.updated_at`（epoch 秒，`Job::push` 每次事件都更新）——
    运行中但 `IDLE_WARN_SECS`(120s) 没有新事件，就在行上写"可能卡在外部进程或网络盘上"。
    没有它，挂住的外部进程在界面上完全看不出来，只能干等。
  - 资源键（`server/scope_key.rs`，**相等匹配**）：`p{pid}:out`（管线/剪切/订正/入库：events.json 与
    output/*）、`p{pid}:idx`（所有 cut 共写 publish_config.json + clips_order.txt）、
    `p{pid}:clip:{clip-07}`（单条重切 + 该目录上的精修）、`models`（下载模型 vs 转写
    `ensure_models`）、`stickers`（表情包构建）、`dir:{规范化路径}`（精修目录兜底键）。
    **全切刻意不带 `clip:*`**：它只处理没有 `clip.mp4` 的目录，所以与精修并行是安全的
    （用户拍板）；单条重切与同片段的精修则必须互斥。
  - 取消：`cancel()` 置状态位 + 丢开跑信号（排队中直接不发车）。**已取消但协程还没退出的任务
    仍然占着资源**（`worker_done`），否则"取消渲染后立刻再渲染"会两个进程写一个目录。
  - **三条"任务一定会有终态"的底线**（2026-09-16，排查"剪切卡住 + 按钮一直灰"时补的）——
    违反任何一条，任务都会**永久**占着资源键与档位名额，同项目后续任务无声排队：
    ① 工作协程抛 panic → `spawn_job` 的 `catch_unwind` 替它 `fail`（**release 构建是
       `panic = "abort"`，这条兜底在 release 里其实是失效的**，别指望它）；
    ② **线程都起不来** → `spawn_job` 也要 `fail`（以前只 `tracing::error!` 就完了，
       任务停在 Running 谁也查不出来）；
    ③ **外部进程要有超时**：`cutting::cut_segment` 的 ffmpeg 走 `run_ffmpeg(cmd, timeout, what)`
       （按片长给 `max(3×时长, 180s)`，超时 `kill` 并报超时）—— 素材在网络盘上（RaiDrive/WebDAV）
       盘一抖 ffmpeg 就永远卡在 read 上，没有超时就是无限期占着 `p{pid}:out`/`idx`。
       新增外部进程调用时照这个来，别再写裸 `.output()`。
  - **调度器自愈 tick**：调度是事件驱动的（`create`/`complete`/`fail` 各跑一次 `schedule_locked`），
    只要有人永远不返回就**没有任何事件**再来触发它。`jobs::spawn_housekeeping`（`server::run` 里起）
    每 `SCHED_TICK`(5s) 重跑一次，并在有任务排队超过 `BLOCKED_WARN`(120s) 时打一条 warn
    **点名是谁挡住谁、因为什么**（`blocked_report`：先按资源冲突找，找不到才是本档名额满）。
    这条日志就是"为什么我的任务不动了"的答案，别删。
  - **淘汰只吃"已终态且不再占资源"的**（`Job::evictable`）：单看 `is_active()` 会漏掉
    "已取消但工作协程还没退出"这类 —— 把它淘汰掉，它之后的 `complete`/`fail` 会找不到这个 Job
    直接 `return`，**没人再触发调度器**，后面的任务就停在那儿等下一次 `create`。
  - 同步接口（没法排队）用 `jobs.busy_with(&keys)` → 409 + 文案说明谁在占用：
    删片段 / merge-splits（events.json）/ 改屏蔽轨道 / 预览的隐式 process。
  - 取消经 `ProgressReporter::cancelled()` 在**阶段/批次边界**生效（进行中的单次
    ffmpeg/ASR/LLM 调用不会被打断）；`/api/jobs/:id/events` 先回放历史事件再续传，
    `Lagged` 不会误报成失败。
  - **前端按钮按作用域门控**（`api/jobs.js` 的 `clipKey/outKey/idxKey/MODELS_KEY/STICKERS_KEY`
    必须与 `scope_key.rs` 一致，`busyOn(jobs, keys)` 求交集）：只在自己片段/项目被占用时才禁用，
    「别处有任务在跑」不再让按钮变灰。同步保存类操作（选框/封面参数/字幕/去静音）**不受门控**。
  - **长任务写 polish.toml 一律走 `project::update_polish_toml(dir, |cfg| …)`**（锁内重新
    load + 只覆盖自己负责的字段）。cmd_llm 只写 segments/filter、cmd_process 只写 segments/processed、
    cmd_cover 只写 cover.*。**不许再"开头 load、结尾整份写回"**：那样 LLM 跑两分钟期间用户存的
    选框/封面/屏蔽会被结尾那次覆写静默吞掉（这正是"LLM 剪辑时不能调区域"的根因）。
  - 编辑器里被推迟的后处理（时间轴自动保存 / 选框保存触发的 process）：**不进队列**，
    只记 `pendingProcess`（带片段名），该片段空闲时自动补跑一次 —— 一次编辑风暴不该塞十几条任务。
  ⚠️ 前端订阅：`watchJob()` 返回的是 **`{ close() }` 对象**，关订阅要 `w.close()` ——
  当函数调（`jobClose?.()`）会抛 `jobClose is not a function`（压缩后就是「ve is not a function」），
  而且它就在 `jobClose = watchJob(...)` 上一行：抛错后**新任务永远订阅不上，界面从此停在「运行中」**，
  自动后处理又会被 `jobRunning` 挡住（整条链路静默失效）。`PolishView` / `PrepareView` / `CoverDesigner`
  三处都踩过，已统一走 `closeJobWatcher()` / `.close?.()`。
- `/api/polish/*` 的 `dir` 必须是非空且存在的目录；`polish.toml` / `events.json` /
  `vtrim.toml` / 全局配置的读-改-写走 `vtrim_core::filelock::with_path_lock` 串行化。
  管线里那些**同步**上下文的写（`config::edit_toml_at` / `patch_config_file` /
  `normalize_project_paths_in`）走 `filelock::with_path_lock_sync`（同一路径的 std 锁，
  只包微秒级的读改写）—— 管线写 `transcript_file` 与设置页保存项目配置是同一份文件，
  无锁时后写的会把先写的键整段覆盖。
- 监听非 loopback 地址必须显式 `--allow-remote`；账号数据在
  `<data_dir>`（`--data-dir` / `VTRIM_DATA_DIR`，默认 `<exe_dir>/.vtrim/cloud`），
  由 `vtrim user *` 与 HTTP 侧共用同一个 SQLite 账本。

### 多用户服务进度（设计：`docs/superpowers/specs/2026-09-12-multi-user-server-design.md`）

| 阶段 | 状态 |
|---|---|
| P1 step-1 账号账本 + `vtrim user` CLI | ✅ 已落地 |
| P1 step-2 HTTP 会话鉴权 + 前端 M1 登录 | ✅ 已落地（`server/auth.rs`、`service/instancelock.rs`） |
| P1 step-3 `vtrim doctor` 安全摘要 | ✅ 已落地（`service/doctor.rs`、`service/disk.rs`） |
| P2 项目托管与隔离（`?project=`、囚笼、任务归属、前端 M2） | ✅ 已落地（`vtrim-core/tenant.rs`、`service/projects.rs`、`server/scope.rs`） |
| P3 录播姬接入（webhook / 入库 worker / 兜底扫描 / 前端 M3） | ✅ 已落地（`server/ingest.rs`、`server/ingest_worker.rs`、`danmaku::parse_bililive_xml`） |
| 邀请码自助注册（账本 v4 `invites` / `POST /api/auth/register` / 前端 `#/register` + 设置页管理卡） | ✅ 已落地（`service/invite.rs`、`components/InviteManager.vue`） |
| P4 B站链接通道 / P5 容量与运维 | ⏳ 待做 |

## 开发

```bash
cargo build                    # 0 warnings
cargo check -p vtrim-cli --all-targets   # 改 server 后必跑（0 warnings）
cargo test -p vtrim-cli        # server 单测（JobManager / Range / 路由 404）
cargo test --lib danmaku       # ASS 解析 + 密度计算测试
cargo run -- status            # 查看当前进度
node webui/scripts/check-overlay.mjs   # 覆盖层色档契约：templates/index.html ↔ engine.js 对拍
```

前置条件: `ffmpeg` + `ffprobe` 在 PATH 中。

### 新增配置字段步骤

1. `config/types.rs` 加字段 + serde default
2. `config/mod.rs` 的 `merge()` 加对应条件（注意 `.vtrim/config.toml` 在 `vtrim.toml` 之后 merge）
3. 结构体手动 `Default` impl (有字段级 serde default 时必须)
4. `config/default.toml` + 注释

### 核心文件索引

| 路径 | 说明 |
|------|------|
| `vtrim-cli/src/main.rs:27` | `resolve_input()` input 回退 |
| `vtrim-cli/src/main.rs:41` | `build_pipeline_runner()` 注册 stage |
| `vtrim-cli/src/main.rs:62` | `check_stage()` 状态机 |
| `vtrim-core/src/pipeline/stage.rs` | STAGE_* 常量 + Stage trait |
| `vtrim-core/src/pipeline/runner.rs` | PipelineRunner (run/run_all/run_until) |
| `vtrim-cli/src/server/jobs.rs` | JobManager：资源域互斥 + 三档闸门 + 前后台优先级（`JobClass`/`JobPriority`/`JobSpec`/调度器/取消） |
| `vtrim-cli/src/server/scope_key.rs` | 任务资源键：谁和谁不能同时跑（`p{pid}:out/idx/clip`、`models`、`stickers`、`dir:`） |
| `docs/job-queue.md` | 任务队列说明书：三档成员/理由、优先级、与资源键的关系、取舍、排查 |
| `vtrim-cli/src/server/render_proc.rs` | 渲染/封面子进程：pid 登记、取消杀进程树、启动清扫孤儿（**别退回管道**，见「渲染导出」一节） |
| `vtrim-core/src/config/mod.rs` | 两源合并 + toml_edit（`edit_toml_at` 走 `with_path_lock_sync`） |
| `vtrim-core/src/config/types.rs` | Config 类型定义 + Default impl |
| `vtrim-core/src/types/mod.rs` | Event, PipelineContext, Transcript |
| `vtrim-core/src/download.rs` | 模型/binary 下载 + 断点续传 |
| `vtrim-core/src/asr/local.rs` | sherpa-onnx (CPU only) |
| `vtrim-core/src/detection/llm.rs` | LLM prompt + API |
| `vtrim-core/src/detection/mod.rs` | EventDetector + 弹幕热度 |
| `vtrim-core/src/danmaku/mod.rs` | ASS/SRT 弹幕解析 + 密度 |
| `vtrim-core/src/danmaku/parser.rs` | ASS Dialogue 解析 |
| `vtrim-core/src/llm/mod.rs` | 统一 LLM 客户端（core + polish 共用） |
| `vtrim-core/src/cutting/mod.rs` | Cutter |
| `vtrim-core/src/cutting/silence.rs` | 静音检测（`detect_silence_ranges`）。**先按时间排序 + 合并重叠区间**再取相邻间隔（`silence_gaps_from_segments`，含单测）—— 乱序/重叠的字幕按文件顺序硬算会造出几十秒假静音 |
| `vtrim-polish/src/project/silence.rs` | 「去静音」= 静音区间 ↔ 屏蔽轨道（mute_ranges / silence_ranges）。**与当前屏蔽相加**：应用 = `轨道 ∪ 检测结果`（重叠/相邻合成一段），撤销 = `轨道 − 当初自动加进去的`（差集，手动屏蔽原样留下）；检测结果两侧各留 `SILENCE_PAD_SECS`(0.8s) **气口**（贴边剪会削掉首尾音节），且**默认阈值 `DEFAULT_MIN_GAP_SECS = 3.5s`**（字幕间隔 3.5s 以内一律不处理，用户要求；实际剪掉 = 间隔 − 1.6s）；`union_ranges`/`subtract_ranges`/`pad_ranges` 是纯函数，有单测 |
| `vtrim-polish/src/commands.rs` | 所有 cmd_* 精修命令入口 |
| `vtrim-polish/src/process/mod.rs` | cmd_process 主流程（字幕/段处理/预算） |
| `vtrim-polish/src/process/sfx.rs` | SFX 规则引擎 + 音效解析 |
| `vtrim-polish/src/process/fx.rs` | 缺 end 补默认 + 瞬时特效跨度 clamp |
| `vtrim-polish/src/process/download.rs` | 贴纸解析 + 资产下载命令 |
| `vtrim-polish/src/process/placement.rs` | 贴纸落点：从 人物/字幕/弹幕 反选安全区 + 中心撒点（纯几何，含单测）|
| `vtrim-polish/src/project/config.rs` | vtedit 风格项目配置 + Event/Segment 类型 |
| `vtrim-polish/src/render/chrome.rs` | Chrome headless 渲染 |
| `vtrim-polish/src/render/ffmpeg.rs` | FFmpeg 合成/编码/SFX 混音 |
| `vtrim-polish/src/render/server.rs` | 预览 HTTP server（含字体库路由 `/vt-fonts/:name` 与 `@font-face` 注入，见「字体库」一节） |
| `vtrim-core/src/fonts.rs` | 程序级字体库 `<程序目录>/resources/fonts/`：磁盘扫描 + 字族名解析（`name` 表，无外部依赖） + id→文件解析 |
| `vtrim-polish/src/llm/mod.rs` | LLM 段分割 + 关键词 |
| `vtrim-polish/src/llm/events.rs` | Step2 逐段事件生成（core Agent + checkpoint） |
| `vtrim-polish/src/assets/downloader.rs` | emoji/irasu/SFX 下载 |
| `vtrim-polish/src/render/pipeline.rs:11` | cmd_render（自动 process → render → cover） |
| `vtrim-polish/src/commands.rs:296` | cmd_preview（自动 process → 双端口预览） |
| `vtrim-polish/src/commands.rs:200` | cmd_llm（使用 .vtrim/llm.toml） |
| `vtrim-cli/src/server/scope.rs` | `/api/polish/*` 的作用域守卫：`/api/polish/*` 的**路径面收口**（`EXTRA_PATH_FIELDS` + `resolve_file_in_project`） |
| `vtrim-cli/src/server/ratelimit.rs` | 全站请求限流（按 IP 固定窗口）+ 入库 webhook 专用额度 |
| `vtrim-cli/src/server/ingest.rs` | 录播姬接入：webhook 接收 + **采集配置读写**（`config_view_json` / `build_ingest_patch` / `rotate_token`） |
| `webui/src/components/RecorderConfig.vue` | 设置页「录播姬（采集）」卡：开关 + 连接信息表单 + token 生成 + 测试连接 |
| `vtrim-cli/src/service/doctor.rs` | 安全与运行状况摘要（直接读本机账本，不依赖服务端接口） |
| `LICENSE` / `THIRD-PARTY-NOTICES.md` | 许可（GPL-3.0-or-later）与第三方组件/素材出处 |

## 开源与公网部署

本节记的是**对外发布 / 公网多租户**才需要、但改代码时极容易踩掉的东西。单机自用不受影响。

### 许可

- 本项目整体 **GPL-3.0-or-later**（`LICENSE`），根因是发行二进制经 `sherpa-onnx` **静态链接了
  GPL-3.0-or-later 的 espeak-ng**（其预编译静态包里打包的第三方，且是无条件链接）。
  改依赖（sherpa-onnx / 出片二进制）前先看 `THIRD-PARTY-NOTICES.md`，别把许可改回 MIT。
- **不内置任何字体**：原先随包的兜底字体「乐米波波体」已移除 —— 它是"免费商用但**不包括
  嵌入式用途**"，随软件分发/内嵌必须另行授权。字体候选链到系统字体（`C:/Windows/Fonts/msyh.ttc`）
  为止；想用圆润风格让**用户自行安装**（备忘写在 `api.rs::polish_font` 与
  `native/mod.rs` 的候选链旁，别把 ttf 塞回仓库或发布包）。
- 素材类（`sfx_catalog.json` 音效目录、いらすとや 插画、EmoteLab 贴纸）的出处与条款见
  `THIRD-PARTY-NOTICES.md` 第五节。

### 路径面（多租户的头号风险，改动前先读）

**服务端能读写的文件范围 = 请求里出现的每一个路径字段，而不是"项目目录"这个概念。**
曾经有三处漏了归属校验，普通登录用户就能把服务器上任意文件（`<数据目录>/.vtrim/llm.toml`
里的 **LLM API key**、账号账本 `cloud/vtrim.db` 里的明文邀请码与全部真实路径）复制/读进
自己的项目，再用 `/api/media` 取回。三道防线，缺一不可：

| 防线 | 位置 | 规矩 |
|---|---|---|
| ① polish body 字段收口 | `server/scope.rs::enforce_scope` | body 里每个"会被 handler 拿去开文件"的字段都要在 `EXTRA_PATH_FIELDS` 名单里（`dir` / `output_file` / `source` / `cover.image`）。**新增此类字段必须同步加进名单**，漏一个就是一条任意文件读取。 |
| ② 项目配置路径键 | `server/api.rs::PROJECT_PATH_KEYS` + `check_project_paths` | `PUT /api/project-data/config` 的 `raw` 与 `patch` 两条分支都要过这张表（`input_file` 等 5 个 + `cutting.output_dir`）：非 admin 必须能 `jail_join` 进自己项目。`Workspace::resolve` 对**绝对路径原样放行**，所以校验只能在这里做。admin 豁免（用「服务器文件」建项目时 `input_file` 本来就是项目外的路径）。 |
| ③ 消费侧纵深防御 | `vtrim-polish/src/cover.rs` | 渲染封面时对 `cover.image` 再做一次 `tenant::ensure_within`。handler 层的校验只覆盖"走 API 的那些入口"，别删这道。 |

⚠️ 相关的两个既有约定也别松：`every /api/media` 走 `jail_existing`；`is_safe_component`
挡路径拼接（删除片段/贴纸文件名）。

### 认证与授权

- `must_change_password` 是**服务端强制**（`auth::require_password_changed`）：仍用初始密码时
  除 `/me`、`/auth/password`、`/auth/logout` 外一律 403。**挂载顺序不能反** —— `require_user`
  必须在外层（先跑并写入 `CurrentUser`），这个门在内层才读得到用户；axum 里"后加的 layer 在外层"，
  所以 `api.rs` 里那两行是倒着写的，有注释。
- `--trust-proxy` 是 `X-Forwarded-*` 的**唯一开关**（`auth::trust_proxy()`）：
  关了（默认）就只认 socket 对端，`X-Forwarded-For` 一律不采信 —— 否则限流能被"每个请求换个
  假 IP"绕过，`X-Forwarded-Proto: https` 还能把 cookie 的 `Secure` 判定交给请求方。
  打开时客户 IP 取 `X-Forwarded-For` 的**最后一段**（自己那台反代追加的那段）。
- `/api/health` 是**未认证**接口：不含账号数量、不含"是否仍用初始密码"
  （那等于告诉远端"这台机器还在用初始密码"）。要看这些走 `/api/admin/users`（admin）、
  `/api/me`（已登录）或 `vtrim doctor`（直接读本机账本）。

### 限流与配额（公网部署的护栏）

单机自用时它们永远碰不到；`docs/job-queue.md` §4 那句"前台任务都是人点出来的，频率有天然上限"
在多租户下**不成立**（脚本可以每秒几千次）。三处：

| 位置 | 内容 |
|---|---|
| `server/ratelimit.rs` | 全站按客户端 IP 的通用额度（装得下一次页面加载的突发）+ 入库 webhook 专用额度（未认证入口，每次失败都要跑 scrypt）。挂载在 `api::router`**最外层、访问日志之外**：既要覆盖未认证入口，也不让被限流的请求各写一行日志。 |
| `server/ingest.rs::token_shape_ok` | webhook token 的形状检查，在 scrypt **之前**快速否掉垃圾请求（别为不可能成立的输入付 32MiB 的 KDF）。形状不过**不写 audit**（否则变成"用垃圾请求放大 DB 写"）。 |
| `server/jobs.rs` | `MAX_CREATE_PER_MINUTE` / `MAX_ACTIVE_PER_OWNER` / `MAX_ACTIVE_TOTAL`：这套队列里**唯一的"拒绝"路径**（闸门与资源键只排队）。超限 = 任务记失败 + 丢掉开跑信号（`wait_start()` 报 Err，与"排队中被取消"同一条路径）。**别给轻活加闸门名额**来替代它 —— 那会把编辑器每次改动都要重跑的 `polish_process` 一起挡住。 |

另外两处小坑：登录失败计数表（`auth.rs::rate_fail`）必须按 `MAX_LOGIN_KEYS` 清理（用户名来自
请求且不要求存在，不设上限就能把内存撑爆）；访问日志必须 `redact_path` 掉 webhook 的 token 段
（它是**路径段**，原样记下来等于把凭证抄进日志文件与「运行日志」面板）。

### 默认端口（26320）

**唯一一份定义**在 `service/doctor.rs::DEFAULT_PORT`，`ServerArgs.port`（clap 默认值）、
`DEFAULT_URL`（`vtrim doctor --url` 默认 + `vtrim ingest token` 打印的 webhook 地址）都引用它；
`main.rs` 里 `polish preview` 回退起服务的那份 `ServerArgs` 也是。
**别在任何地方再写端口字面量**（前端 webhook 地址用 `location.origin`，不用管）。

数值上是这么挑的：老默认 **7788 在部分 Windows 上属于 Hyper-V/WSL 的保留端口段**
（实测段 7635–7834），表现是 `10013 WSAEACCES` 绑不上、而 `netstat` 里**一个监听者都没有**
（它不是"被占用"，去 netstat 只会白找）。26320 落在跨平台安全带 **15001–32767** 内
（躲开 Windows 动态段与保留段，也在 Linux 动态段 32768–60999 之外）。
⚠️ 两个段**都会动态重排**，所以别把"26320 一定能绑"当保证：
`mod.rs::first_bindable_port` 会在绑失败时**实测试绑**一个再来报（候选按"先安全带内"排序），
`bind_conflict_hint` 按 `10013`（系统拒绝，别去 netstat）/ `10048`（真被占用，给 netstat + tasklist）
/ 未知（两种成因都列）分流，并带上本次实例的 exe 目录与数据目录 —— 同机多份解包目录时靠这个区分。
`service::doctor` 有单测锁住 `DEFAULT_URL` 与 `DEFAULT_PORT` 一致（改一个忘另一个 =
`vtrim doctor` 探测不到自己）。

### 公网部署检查清单

1. 反代（Caddy/nginx）+ HTTPS；服务本身**没有 TLS**，裸 HTTP 上线等于明文口令与 cookie。
2. 显式加 `--allow-remote`（默认只监听 loopback）与**强口令**（仍用初始密码会被拒绝启动）。
3. 反代后面加 `--trust-proxy`，否则所有请求都被当成来自反代本机（限流退化成全局），
   也不会下发 `Secure` cookie。
4. `<数据目录>/.vtrim/`（含 `vtrim.db`、`llm.toml`、`logs/`）**不要**放进镜像层、
   不要公开、不要打进任何分享包；备份它即可。
5. 分发前检查 `release/` 里那份**解包目录**（不是 zip）：跑过的实例会在里面留下
   `.vtrim/`（`server.toml` 含真实绝对路径）。`scripts/package-release.ps1 -SmokeTest` 会清掉它。

