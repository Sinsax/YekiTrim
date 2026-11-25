# LLM 链路：统一与规格化（审计 + 计划，2026-11-25）

> 触发：用户「再重新检查一下 llm 链路，需要对 llm 进行一个整体的统一和规格化」。
> 配套：`docs/plan/llm-config-refactor.md`（配置/UI 重构，D1–D6 已拍板，D3 选 **A**：砍掉 `provider.params`）。
> 本文只写**事实与判断**；动手按 §5 的 S/L 交替顺序。

## 0. 结论先说

链路本身**设计是对的**（传输层单点、caps/dialect 单点、取消探针三处都接了），
不统一的地方集中在**"谁来装配一次 LLM 调用"**：构造点 18 处、命名四种、`for_link` 靠自觉、
并发/窗口/用量标签各自为政，还有一处绕开 `for_link` 直接改温度。
**规格化的目标不是加抽象，而是让「忘记套链路参数」变成编译错误。**

## 1. 现在健康的四处（别动）

| 项 | 证据 |
|---|---|
| 配置 → 请求参数**只有一个换算入口** | `impl From<&LlmConfig> for LlmParams`（`core/config/types.rs:2253`） |
| 请求体组装 / 重试 / 超时 / UA 单点 | `yekitrim-llm/src/client.rs`（429/5xx 与超时各重试一次，`attempt < 2`） |
| 推理写法的方言翻译单点 | `yekitrim_llm::dialect`（`reasoning_effort` / `thinking{type}` / `enable_thinking` / `thinking_budget` / `chat_template_kwargs` …） |
| 取消探针真接上了 | `correct.rs:262` / `detection/llm.rs:900` / `polish/llm/events.rs:316` 三处 `Agent::with_cancel(probe)`，边界门禁的成对判据也在守 |

## 2. 不统一的九处（每条带定位）

### F1 构造点 18 处，`for_link` 靠自觉
`LlmClient::new(...)` / `Agent::new(...)` 在外面的调用点有 18 处，命名四种：`llm` / `llm_cfg` / `linked` / `self.llm`。
其中**先 link 再 new**的写法靠人记：
- ✅ `detection/llm.rs:896 Agent::new(llm.for_link(Detect))`、`cover.rs` 的 `linked`、`commands.rs` 的三处 `linked`；
- ⚠️ `detection/llm.rs:573 LlmClient::new(&llm)` —— 是否已 link 只能往上游读；`polish/llm/events.rs:242/310/423`、
  `polish/llm/mod.rs:83/185` 同理（变量名 `llm_cfg` 不表示它已 link）。

**后果**：忘记 `for_link` 不报错、不告警，只是**那条链路静默用全局默认参数**。这类 bug 只能靠"参数没生效"的用户反馈发现。

### F2 绕开链路直接改参数（越权覆盖）
`crates/yekitrim-server/src/api/polish/cover.rs:459-463`：`LlmClient::new(&linked)…with_temperature(temp)` ——
`for_link` 已经把温度算好了，这里又按请求覆盖一次 ⇒ 同一件事有第二个入口（界面上的温度只在一处配，但实际有两条路）。
（`detection/llm.rs:1952/1975` 的两处 `with_max_tokens` 是测试夹具，不算违规。）

### F3 并发数有两个读法
`LlmParams.max_concurrent = c.concurrency(None)`（`types.rs:2309`，**永远全局值**），
而三处调用点又各自 `config.concurrency(section_override)` 再 `Agent::with_max_concurrent(...)`
（`correct.rs:257`、`detection/llm.rs:579/879/897`、`polish/llm/events.rs:311`）。
⇒ 同一个旋钮，`LlmParams` 里那个值**永远不等于**实际用的那个。谁读错就是 bug。

### F4 用量标签是手写中文字面量（11 处）
`"精修分段"` / `"精修关键词"` / `"贴纸包标签"` / `"emoji 多维分类"` / `"贴纸扫描标签"` / `"LLM 测试"` …
散在 5 个文件；`AgentTask::usage_context()` 默认取 `name()`。⇒ 新增链路必然漏标签（用量表里出现空 label）。

### F5 窗口大小只有 correct 能配
`with_window_size(correct.window_size.max(1))`（`correct.rs:256`）只有订正一处；检测/精修的窗口写死在各自模块。
⇒ 同一个概念三种来源：配置、常量、默认值。

### F6 "省略温度 = 不发" 借用 caps 表达
`From`：`if p.temperature.is_none() { caps.temperature = Support::Unsupported; }`（注释自己承认是借道）。
⇒ 传输层无法区分"模型不支持温度"与"用户没配温度"，两者走同一个开关。

### F7 老的链路开关与新的档位双轨
`reasoning_explicit`（`params.rs:205`）存在的唯一理由：`[correct] thinking` 这类**链路老开关**只在 LLM 层没表态时生效（`agent.rs` 判 `!params.reasoning_explicit`）。
⇒ D6 不兼容后这条双轨应当**整条删掉**。

### F8 caps 三源合并 vs 即将到来的模型清单
`From`：`catalog::caps_for(base_url, model)` → `provider.models` 命中则覆盖 → 顶层 `llm.models` 再覆盖。
D1 之后模型清单变成**顶层 `[[llm.models]]` 唯一一处**，这三段要重写成「catalog 只做建议值（UI 一键填入）+ 清单里的值说了算」。

### F9 LLM 行为键散在两个配置文件里
`llm.toml` 里是接口/参数；而 `config.toml` 里还躺着 LLM 行为：`[detection].system_prompt` / `prompt_template` / `reference_titles`、
`[detection].llm_score_weight`、`[correct] thinking` / `window_size` / `max_concurrent`、`[asr].llm_correct`。
⇒ 「LLM 怎么跑」要跨两个文件找；UI 又把提示词摆在 LLM 页（D2 保留），一份配置两个家。

### F10 `protocol` 与 `dialect` 两个概念没讲清
`PROTOCOLS` 只有 `openai-chat` 一条（端点约定：路径/鉴权/模型列表端点）；`dialect` 才有六种推理写法。
`LlmProvider.protocol` 目前实际上没有第二种取值 ⇒ 字段是"为将来预留"，但文档里和 `dialect` 并列出现，读的人会混。

## 3. 目标：一条装配路径 + 四张表

```
配置（llm.toml v2）
   │  LlmConfig
   ├─ providers[]        「怎么连」：id / name / protocol / base_url / key
   ├─ models[]           「有哪些模型、属谁、能力如何」（D1 的清单，唯一）
   ├─ defaults           「默认怎么发」（逐键基准）
   └─ links.<link>       「这条链路怎么发」：provider? / model? / 参数覆盖
        │
        ▼  LlmConfig::link(link) -> LinkedLlm      ← 唯一解析点（返回接口+模型+参数+caps，全部算完）
        │
        ▼  LlmClient::for_link(...) / Agent::for_link(...)   ← 唯一构造点（不许再 directly new）
        │
        ▼  yekitrim-llm 传输层（请求体 / 重试 / 超时 / 方言 / 用量）
```

四张表（都在 `yekitrim-llm`，不放业务代码里）：
1. `LlmLinkSpec`：link → （默认窗口、默认并发上限、是否可见于 UI、用量标签）。**标签中英文/文案的唯一来源**。
2. `PROTOCOLS`：端点约定（现有）。
3. `DIALECTS`：推理/输出上限的 wire 写法（现有）。
4. `catalog`：内置能力建议值（降级为"建议"，清单说了算）。

## 4. 原则（写进卡里，防回潮）

1. **一次 LLM 调用只能有一个构造入口**：`LlmClient::for_link(&cfg, link)` / `Agent::for_link(...)`。
   删掉 `impl From<&LlmConfig> for LlmParams` 的**公开**可用性（改成 `LinkedLlm` 才有 `Into<LlmParams>`）⇒
   **忘记套链路参数 = 编译错误**，不再靠自觉。
2. **参数只能来自配置**：任何 `.with_temperature(...)` / `.with_max_tokens(...)` / `.with_thinking(...)` 在业务侧出现即视为违规
   （要按请求覆盖，就走 `links` 的一次性 override 参数并说明理由）。
3. **并发只有一个读法**：`LlmConfig::concurrency(link)`；`LlmParams` 不再带 `max_concurrent`。
4. **用量标签只从 `LlmLinkSpec` 取**，业务侧不许手写中文字面量。
5. **能力只从清单取**，catalog 只做 UI 建议。
6. **`None` 就是"不发"**：`temperature` / `max_tokens` / `top_p` 一律 `Option`，不再借 caps 表达。

## 5. 分期（与配置重构 L1–L4 交替，每期独立可验证）

| 期 | 内容 | 依赖 | 验收 |
|---|---|---|---|
| **S1** | 构造入口唯一化：`LinkedLlm` + `LlmClient::for_link` / `Agent::for_link`；删 `From<&LlmConfig>` 公开路径；收掉 `cover.rs` 的 `with_temperature`；并发解析唯一化（`concurrency(link)`，`LlmParams` 去掉 `max_concurrent`） | 可与 L1 并行 | 编译期：业务侧再无 `LlmClient::new(&config)`；`cargo test --workspace` 全绿；「两条链路并发值不同」用例 |
| **S2** | `LlmLinkSpec` + 用量标签/窗口/并发默认值收敛；删 `reasoning_explicit` 双轨（F7） | S1 | 用量表 label 非空、与 spec 一致（用例枚举 5 条链路）；删掉 `[correct] thinking` 之后行为不变的单测 |
| **S3** | caps 合并层重写：catalog 只做建议，`[[llm.models]]` 说了算（F8） | L1 | 「表里没有、清单里写了」与「表里写了、清单里改了」两条用例；UI 的"获取模型"只填建议不改生效值 |
| **S4** | 配置边界：把 `[correct] thinking/window_size/max_concurrent`、`[detection]` 的提示词与 `llm_score_weight` 搬进 `llm.toml`（D2 保留 UI 位置）；`protocol`/`dialect` 文案归一（F9/F10） | L1 | 迁移后 `config.toml` 里不再有 LLM 行为键；`llm.toml` 能独立解释"LLM 怎么跑"；老文件迁移幂等 |
| **L2** | 端点：`/effective`、草稿测试、v2 写入校验 | L1 | 契约差集 0；草稿能测通 |
| **L3** | WebUI 五块 IA + 模型列表 + 提示词独立折叠 + extra_params 拆分交互 | L1/S2 | 手工回归清单（见 §6） |
| **L4** | 删 `llmProviderMode`/`LlmQuickSetup` 重复弹层 + 文档卡同步 | L3 | `gate --all` 0 非基线失败 |

**顺序建议**：`L1 → S1 → S3 → L2 → S2 → L3 → S4 → L4`。
（L1 定形状，S1 让形状不可能被绕过，S3 让能力只有一个来源，然后才动 UI。）

## 6. 验收清单（整轮做完时跑）

```bash
cargo test --workspace --no-fail-fast          # 全绿
cargo check --workspace --all-targets          # 0 warning
node scripts/check-clippy-budget.mjs           # 不涨过预算
node scripts/gate.mjs --all                    # 0 条非基线失败
python3 scripts/check_module_boundaries.py     # 21 条
python3 scripts/check_module_map.py            # 卡与预算
npm --prefix webui run build                   # UI 期必跑，dist 与 src 同批提交
```

**手工回归**（UI 期）：① 填 key→测试（不保存）→ 保存 → 刷新，值不丢；② 检测链用模型 A、精修链用模型 B，
两边的任务日志里 `model=` 各不相同；③ 把 `extra_params` 里写 `temperature` → 保存被拒并指路；
④ 老 `llm.toml`（v3）打开一次→保存→再打开，key 仍在、结构已是 v2。

## 7. 不在本次范围

- 新增第二家 `protocol`（现在只有 `openai-chat`）；
- 重试策略/超时数值调整（`client.rs` 现有 1 次重试、10s 连接超时）；
- 多模态输入（`input` 能力位已有，链路侧没有消费方）。
## 8. 进度与重新排期（2026-11-25 晚，执行中）

| 期 | 状态 | 提交 |
|---|---|---|
| L1 配置 v4 + 迁移 + server 通道 | ✅ | `c14235c9` `f61aff2f` `7724ed8c` `f0cfaa7a` `4247a3f6` |
| S1 构造入口唯一化 | ✅ | `850ea110` `620e512f` `998478fe` |
| S3 caps 单源（后端判据） | ✅ | `0ace3189`（UI 那半并入 L3） |
| L2 端点（`link_snapshot` + `/effective`） | ✅ 2/2 | `c33a66f2` `c298b227` |
| S2 F2 温度第二入口 | ✅ 1/3 | `be222d4e` |
| S2 F4/F5 `LlmTaskSpec` + 用量标签 | 待做 | — |
| S2 F7 删 `reasoning_explicit` 双轨 | **并入 S4** | — |
| L3 WebUI 五块 IA | 待做 | — |
| S4 配置边界搬键 | 待做 | — |
| L4 删旧分支 | 待做 | — |

### 为什么 F7 并入 S4

`reasoning_explicit` 不只是一条**老开关的双轨**，它同时是 `LlmClient::with_thinking()` 与 `wire.rs` 的
语义锚点（「LLM 层有没有表过态」决定发不发推理参数）。真正要退场的是 `[correct] thinking` 这个**配置键**，
而删配置键按本仓规矩必须走「`CONFIG_VERSION` +1 + `retired_in` 登记 + 模板注释」那三步 ——
那正是 S4「把 `[correct] thinking` 搬进 `llm.toml`」要动的东西。分两处改会把同一次迁移拆成两次版本升级。

### L2 剩余一项也并入 L3

`POST /api/llm/test` 的**草稿测试**（不落盘就能试一个新 key）在语义上依赖「按链路测哪个连接」，
而 L3 的链路表格才是它的入口（每行一个「测试这条链路」）。后端参数（`base_url`/`api_key`/`model` 草稿）
已经由 `resolve_llm_conn` 的 draft 参数支持，L3 时把它接到 `llm_test` 的 body 上即可。