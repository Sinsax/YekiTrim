# yekitrim-llm — LLM 传输层（统一客户端 + Agent 分批窗口 + 用量记账）

> 范围：`crates/yekitrim-llm`/** ｜ 上游：`yekitrim-core`（检测/订正）、`yekitrim-polish`（段分割/事件/关键词/贴纸/封面）、`yekitrim-server`（测试连接）｜ 验证：`cargo test -p yekitrim-llm --lib`

## 职责

**LLM 传输层**（独立 crate，不认识配置文件、也不认识任何 yekitrim-* crate）：`LlmClient` 负责 HTTP + 重试 + 用量采集，`Agent` 把长输入切成
窗口并发跑并支持 checkpoint 续传（事件检测、订正、精修段事件都走它），`usage` 定义记账记录与 sink。
上层（CLI/server）只负责装 sink，core 只 `emit`。

## 入口

- `client::LlmClient`（`client.rs`）：`new(LlmParams)`（泛型：接受任何 `Into<LlmParams>`）/ `raw(api_key, base_url, model)` / `send(messages)` / `chat(system, user)`
  / `with_temperature` / `with_max_tokens` / `with_timeout` / `with_thinking` / `with_usage_context(ctx)`。
- `agent`：`AgentTask`（`name()`、`usage_context()`、`prompt_fingerprint()`、窗口/提示词构造）、
  `Agent`（`new(LlmParams)` / `with_window_size` / `with_max_concurrent` / `with_cancel(probe)` /
  `build_windows` / `run` / `run_with_checkpoint` / `run_windows`）、`CancelProbe`、
  `AgentItem` / `AgentOutput` / `AgentWindow` / `AgentProgress`、
  `fingerprint(parts)`、`extract_json[_array|_object]`、`items_to_json`、`windows_from_checkpoint`。
- `usage`：`UsageRecord`、`UsageContext`（`new(label)` / `project(...)`）、`UsageSink`（trait）、
  `set_default_sink` / `clear_default_sink` / `default_sink`、`emit` / `emit_with`、`parse_usage`、
  `host_of`、`truncate_error`、`now_secs`。

## 边界

### 模型能力（2026-10-20：**探针 > 内置表 > 保守默认**）

- **优先级只在一处实现**：`catalog::caps_json_merged` = 先 `caps_for`（内置表 + 保守默认）再
  `caps::apply_partial`（探针自描述）。**reader 说了的字段覆盖表，没说的一字不动**。
  实测起因：服务器自述 `max_output_tokens = 393216`，台面上却显示 8192/384000 ——
  用户质疑「有思考的连正常输出都办不到」。
- ⚠️ **匹配必须严格**（`catalog::find_model`）：整串精确（大小写不敏感）；host 命中某一家就
  **只在那家里找**；host 认不出时只有"全表唯一"才认，**有歧义 ⇒ 未收录**（宁可空着也不猜）。
  **别按后缀/裸名模糊命中**：自建网关的 `cn:deepseek-v4.1-flash` 与公开的 `deepseek-v4-flash`
  是两支模型，借它的数字就是"上限只有 8192"这类事故。判据：
  `probe_wins_over_catalog_for_the_same_model` / `prefixed_custom_ids_never_match_bare_table_names` /
  `ambiguous_bare_id_on_an_unknown_host_is_not_guessed` / `unknown_id_gets_no_guessed_limits`。

### 并发与加速（2026-09）

- **一个旋钮**：`.yekitrim/llm.toml` 的 `[llm] max_concurrent`（默认 3），**事件检测 / 精修事件生成 /
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
- ⚠️ **取消必须把探针交给 Agent**（`Agent::with_cancel(CancelProbe)`）：`run_windows` 在**派发每个窗口前**
  与**每收一个窗口后**查它，命中就 `abort_all()`（在飞的 HTTP 请求随 future 一起丢弃）并返回
  「任务已取消」。只在阶段边界查一次 = 用户点了停止、后台还把几十个窗口跑完 —— 检测那条链路实测
  18 个窗口、十几分钟，取消按钮形同虚设。读探针的源头是 `ProgressReporter::cancelled()`
  （server 侧 `JobManager::cancel_token` 的 `AtomicBool`）。回归钉子：
  `agent::tests::run_windows_stops_when_cancel_probe_turns_true`。
- `response_format = { type = "json_object" }` 放在 `[llm] extra_params` 里
  （两份 llm.toml 实例与 `config init` 模板都已带上）。中转支持就保证不写解释性前言；不支持就删掉。
- 排查偶发 `error sending request`：那是 reqwest 的**建连失败**（不是超时、不是 HTTP 状态码）。
  本机若挂着 Clash/Mihomo 的 TUN（DNS 返回 fake-ip `198.18.0.x`）就是代理那一侧的事。
  `LlmClient` 的 HTTP 选项集中在 `client.rs::build_http_client`（connect_timeout 10s /
  pool_idle_timeout 30s / tcp_keepalive 30s / 浏览器 UA），**所有选项只在那里设**——
  `with_timeout` 会重建客户端，散着写就会像 UA 那样被静默丢掉。

### 按链路参数（2026-09）

- ⚠️ **所有构造 `LlmClient` / `Agent` 的地方都必须走 `LlmConfig::for_link(LlmLink)`**：它解出该链路那一套**请求参数**（`temperature` / `max_tokens` / `reasoning` / `top_p` / `json_mode`），优先级 = `[llm.overrides.<link>]` > `[llm]` > 链路代码默认；
  配置形态的正文在 `crates/yekitrim-core/src/config/AGENTS.md`。**在 `for_link` 之外再写
  `with_max_tokens` / `with_temperature` / `with_thinking`，就是把同一个旋钮掰成两半** ——
  从前精修那两处写死的 `with_thinking(Some("disabled"))` 就是这么来的（已收进 `default_thinking`）。
- 四条链路：`Detect`（检测窗口 + 热度评分）、`Polish`（分段 / 逐段事件 / 贴纸精挑 / 关键词）、
  `Correct`（订正）、`Utility`（贴纸包与扫描标签、封面文案、连通测试）。
  检测窗口请求体真带上覆盖值的回归钉子：`detection::llm::tests::detect_window_request_carries_link_max_tokens`。
- ⚠️ **`[correct] thinking`（`AgentTask::thinking`，订正那条老开关）只在 LLM 层没表态时生效**
  （`agent.rs` 默认 `process_window` 里判 `!params.reasoning_explicit`）—— 不这么写，
  它会把 `for_link` 解出来的 thinking 顶掉。

### 用量记账（token / 缓存命中 / 耗时）

- **采集点只有一个**：`LlmClient::send`（`crates/yekitrim-llm/src/client.rs`）。成功与**最终失败**都记一条
  `UsageRecord`（`usage.rs`），重试算同一次调用（`attempts` 记实际尝试次数，**不按尝试拆行**）。
  要加字段就改这一处 + `usage.rs`，别去 9 个调用点各写一遍。
- **归因**：`label`（链路名）+ `project`（项目/切片目录）。Agent 系任务走
  `AgentTask::usage_context()`（默认取 `name()`，事件检测/订正/精修段事件各自补 project）；
  非 Agent 的构造点（精修分段/关键词、贴纸挑选、贴纸包标签、封面文案、LLM 测试）显式
  `.with_usage_context(...)`。**新增链路记得挂 context**，否则统计里是空 label。
- **落地**：core 只 `emit`，写库由 CLI/server 侧装的 sink 负责——`yekitrim_service::usage::DbSink`
  （进程级默认 sink，`usage::set_default_sink`；`LlmClient::with_usage_sink` 是测试用的私有注入）。
  安装点两处：`server::run`（`AuthService::open` 之后）与 CLI 的 `process/detect/correct/polish` 分支。
  写库失败只 warn，**绝不影响调用结果**。
- **表**：账本 v7 `llm_usage`（ts/label/project/model/host/prompt_tokens/completion_tokens/
  cached_tokens/reasoning_tokens/latency_ms/attempts/ok/status/error）。`cached_tokens` 认三种写法：
  DeepSeek `prompt_cache_hit_tokens`、OpenAI `prompt_tokens_details.cached_tokens`、缺省 0
  ——**前缀缓存省了多少，看这一列**。`latency_ms` 是从第一次尝试到拿到结果的总墙钟（含退避等待）。
- **读出来**：`GET /api/admin/usage?days=N`（**仅 admin**，记录含他人项目路径，与 `/api/admin/logs`
  同一个理由）→ 总量 + 按天/链路/项目/模型分组；设置页「LLM 用量」卡（`components/LlmUsage.vue`）；
  CLI `yekitrim usage [--days N]`。聚合全走 SQL `GROUP BY`，不在 Rust 里扫表。
  **缓存率 = Σcached ÷ Σprompt**（只算输入侧、先求和再相除，不是每次调用比率的平均；
  分母不掺 completion —— 缓存只作用于输入）。`prompt_tokens == 0` 显示 `—`（老中转没报 usage /
  失败行 token 全 0，不是"没命中"）。单发链路（封面文案等）天然 0%、每次任务的第一个窗口必然 0%，
  所以看比率要按「按链路」那张表。实现只此一份：`yekitrim_service::usage::cache_rate`（CLI），
  前端 `LlmUsage.vue` 的 `cacheRate` 是镜像。
- 每次调用还会打一行结构化日志 `LLM 调用 label=… prompt_tokens=… cached_tokens=… latency_ms=…`
  （logbus 渲染成 `k=v` 进设置页「运行日志」与文件日志；失败是 `LLM 调用失败` + status）。

- ⚠️ **本 crate 有两条专属门禁**（`scripts/check_module_boundaries.py`）：
  `llm_is_a_leaf`（不许出现 `yekitrim_core|yekitrim_polish|yekitrim_service|toml::`）与
  `only_llm_knows_wire`（wire 字面量只许出现在这里）。两条都带变异自证 ——
  它们守的正是「隔离」这件事本身，别为了让代码通过去放宽它们。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-llm --lib
```

`client::tests` / `agent::tests`（窗口、指纹、checkpoint、JSON 抽取）/ `usage::tests`（三种
`cached_tokens` 写法、`latency_ms`、失败也记一行）都要绿。
