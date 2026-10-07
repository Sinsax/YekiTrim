# yekitrim-core/src/detection — 事件检测（LLM 分批窗口 + 弹幕热度 + 后处理）

> 范围：`crates/yekitrim-core/src/detection`/** ｜ 上游：`pipeline::PipelineRunner`（Stage 3）、`yekitrim-service::core_pipeline`、`/api/project-data/events/merge-splits` ｜ 验证：`cargo test -p yekitrim-core --lib detection`

## 职责

读 `transcript.json`（+ 弹幕），让 LLM 把转写切成以话题为单位的事件，并用「弹幕/字幕密度 + LLM 传播
潜力评分」给每条算热度，输出 `events.json`。检测器（`mod.rs`）负责窗口、并发、断点续传、评分合成与
后处理；`llm.rs` 存 prompt 与 API 调用。

## 入口

- `detection::EventDetector`（Stage 3）。
- `detection::load_events(path)`、`detection::merge_split_events(Vec<Event>) -> (Vec<Event>, usize)`
  （纯函数，含单测 —— 收拾历史「（i/n）」等分产物）。
- `detection::llm`：`score_events_with_llm`、`call_with_progress`、`test_llm`、`create_default_event`、
  `parse_json`、`DEFAULT_PROMPT` / `DEFAULT_SYSTEM_PROMPT` / `TITLE_RULES` / `DEFAULT_REFERENCE_TITLES`。
- 弹幕热度有关的口径常量：`DETECT_WINDOW_SIZE`（每窗 30 条字幕段）/ `DETECT_WINDOW_OVERLAP`（重叠 5 条）。
- 产物：`output/events.json`（另有 `events.checkpoint.json` / `events.partial.json` 中间态）。

## 边界

- **分批窗口调用**：每窗口 30 条字幕段、窗口间重叠 5 条（`DETECT_WINDOW_SIZE`/`DETECT_WINDOW_OVERLAP`），
  并发取 `.yekitrim/llm.toml` 的 `[llm] max_concurrent`（默认 3，见上「LLM 并发与加速」）；
  每窗口完成即写 `events.checkpoint.json`（断点续传）、
  累计结果写 `events.partial.json`。单窗口失败重试 2 次；**被 max_tokens 截断（`finish_reason=length`）
  时不重试**（参数不变必然再截断，白烧一次调用），直接走拆半递归：`≥ 8 段`且层数 `< 2` 才拆，
  两半各自成败、**一半成功也保留那一半**（曾经的判据是 `len > 30`，而窗口大小就是 30，这条兜底
  从未触发过，被截断的窗口整段丢失 —— 实测事件覆盖率掉到 46%）。截断前已完整的 JSON 元素由
  `llm::parse_json` 抢救回来；解析失败的现场转储在**程序目录** `.yekitrim/llm_debug/`（别再写进程 CWD）。
  ⚠️ **取消**：`EventDetector::run` 把 `ctx.progress` 包成 `CancelProbe` 交给
  `call_with_progress` → `Agent::with_cancel`（窗口边界生效）；检测跑完后、**热度评分之前**再查一次，
  用户在评分阶段点停止就不会再发那批调用。
  ⚠️ **输出上限按链路取**：窗口与热度评分都走 `LlmConfig::for_link(LlmLink::Detect)`
  （即 `[llm.overrides.detect]`）—— 「检测 8k / 精修 32k」不再互相迁就；它只改**请求上限**，
  不进 checkpoint 指纹（提示词没变，续传照常命中）。
- 弹幕: ASS/SRT 解析 → `danmaku_window_secs`（默认 10s）窗口归一化密度 → 每个 event 的 `popularity` 0~1；
  密度 ≥ 0.5 的窗口按密度降序取 top 20、相邻合并成「弹幕爆发时刻」区间**注入 prompt**（名场面位置）。
- ⚠️ **`danmaku_file` 是相对工程目录存的，读它必须解析**：`yekitrim init` 用 `path_to_rel` 写成相对名，
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
  只读诊断：`VT_PROJECT_DIR=<项目目录> cargo test -p yekitrim-core --lib diagnose_danmaku_resolution -- --ignored --nocapture`
- 综合评分: 基础热度有弹幕 `base = (0.6×弹幕峰值 + 0.4×弹幕均值 + 字幕密度) / 2`，无弹幕时只用字幕密度；
  再由 LLM 仅根据 `label + summary` 按预测点击率/有趣程度/爆点强度给 0~100 分，
  `popularity = (1-w)×base + w×(LLM分/100)`，`w = detection.llm_score_weight`（默认 0.3，0=关闭）。
  评分失败/漏评的条目保留 `base`，不阻断检测。弹幕与字幕密度都是**相对量**：弹幕按全片最热的窗口归一，
  字幕密度按全部事件的最大值归一。
  按 popularity 降序赋 `rank`，最终按时间排序输出 —— `rank` 就是热度名次，也决定 `clips/clip-NN` 目录名。
- 后处理: 丢非法 → 排序 → 区间合并 → **边界吸附到 ASR 段边界** → 合并兜底 → 最短 30s（不足吞并相邻）
  → 重名标题加 `（i/n）`。**不做最长时长切分**（曾按 240s 把长事件等分成 `xxx（1/2）`，用户明确反对：
  一次连续事件被拦腰截断；时长交给 LLM 按话题完整性决定）。prompt 也明令禁止等分。
- 降级: API key 为空时使用单个默认事件（调用失败则报错）
- 输出: `events.json`
- **收拾历史等分产物**：`POST /api/project-data/events/merge-splits` →
  `yekitrim_core::detection::merge_split_events`（纯函数，含单测）：把「基名 `（i/n）` + 编号 1..=n 齐全
  + 条数吻合 + 时间首尾相接（1s 容差）」的一组合回一个事件 —— 时间取并集、标题去后缀、摘要取最长、
  热度/置信度取最高、**rank 沿用组内最热的那条**（已剪片段里至少一个目录还对得上）。
  同名但**时间不相邻**的事件不合并（那是给两个不同话题加的区分后缀）。
  ⚠️ **接口保留、UI 已删**（用户要求：prompt 修好之后新检测不再产出 `（i/n）`，
  切片页那颗按钮撤掉了）—— 留着是给**已有**的老 events.json 兜底，用 curl/后续再说：
  合并后事件序号会变，旧片段要用户自己删掉重切。
  只读诊断：`VT_EVENTS_JSON=<项目>/output/events.json cargo test -p yekitrim-core --lib diagnose_merge_splits -- --ignored --nocapture`
- **分散事件第二趟**（`scattered.rs` + `run()` 里那段，`[detection.scattered]` 默认**关**）：主通道只收
  "完整话题"（后处理那条最短 30s 会吞并小块），所以同一个话题被别的话题打断成几小段时不报。第二趟用
  自己的 prompt + 自己的 `topic` 归组键 + 自己的断点文件再跑一遍**低标准**检测，按话题拼成一条
  **多段 `ranges`** 事件；仍不达标的丢。它**不走** `postprocess_events`（那条 30s 吞并会吃光小块），
  跨窗去重门槛按趟取（第二趟传 0）。
  ⚠️ **第二趟只在"主检测没盖住的地方"才可能贡献**：`absorb_scattered` 把被主事件盖住（重叠 ≥50%）
  或紧贴（间隔 ≤1s）的一律丢掉 —— 主事件是权威。实测（项目 12，300 段 / 28.7 分钟窗口、主检测覆盖
  76.9%）：第二趟吐 81 个候选块 → 过门留下 18 条（含 2 条多段），**全部**因"覆盖 ≥49% 或间隔 0.0s"
  被丢 ⇒ **新增 0 条**；把 `merge_min_secs/merge_min_subs` 从 20s/3 条放到 8s/0 条也只多活 1 条。
  ⇒ **密排项目上"没有复合片段"不是阈值高，是主检测不留空隙**（想要自动复合片段得让**主检测**就带
  topic、再把同话题的多条主事件并成一条多段 —— 那是另一件设计，尚未做）。
  只读诊断（两条，都会真发 LLM 调用、只写临时目录）：
  `VT_PROJECT_DIR=<项目目录> [VT_SCATTER_FROM=0] [VT_SCATTER_MAX_SEGS=300] cargo test -p yekitrim-core`
  `--lib diagnose_scattered_stage`（跑通整条 Stage 3，看最终落盘）/
  `--lib diagnose_scattered_funnel`（**漏斗**：一次调用拿候选块，再用同一批块重放多组阈值 +
  逐条打印"为什么没进主列表"—— 校准阈值看这条）。
- ⚠️ **改内建 prompt 要同步用户配置**：`.yekitrim/config.toml` 的 `[detection] system_prompt` /
  `prompt_template` 是**写死的副本**（`config init` 时把内建文本填进去了），代码里改默认值对它无效；
  要么在设置页改，要么把两个字段留空（留空才走内建默认）。
  **每个程序目录各有一份**（`target/debug/.yekitrim/`、`release/<包名>/.yekitrim/` 互不相干）——
  改 prompt 时别只改一个；`'''…'''` 是三引号**字面量**（免转义），正文里没有单引号时可用。
- **前缀缓存**：`DEFAULT_PROMPT` 里 `{segments}` 必须留在**最后**（`{hot_windows}` 排在它之前），
  改 prompt 时别把变化的部分塞到中间 —— 细节见 `src/llm/AGENTS.md` 与 `src/config/AGENTS.md`。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-core --lib detection
```

覆盖 `detection::tests`（后处理/评分/`merge_split_events`）与 `llm::tests`（prompt 拼装/JSON 解析）。
真实数据诊断见上文的 `VT_PROJECT_DIR=` / `VT_EVENTS_JSON=` 两条 `--ignored` 命令。
