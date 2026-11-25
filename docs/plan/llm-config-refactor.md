# LLM 配置重构：建议与计划（2026-11-25）

> 状态：**提案**，未动代码。等口径拍板后按 §6 分期执行。
> 触发：用户「llm 配置需要重构一下，配置和 webui 都要处理。当前不是很直观、十分潦草」。
> 本文只写「哪里潦草（带证据）→ 想改成什么样 → 怎么分期落地」。

## 1. 现状盘点（事实，含定位）

### 1.1 数据模型：**两份真相并存**

`crates/yekitrim-core/src/config/types.rs::LlmConfig`（约 1344 行起）同时容纳两套写法：

| 旧（legacy，R4 兼容） | 新（多接口） |
|---|---|
| 顶层 `api_key` / `base_url` / `model` | `[[llm.providers]]`（`LlmProvider`：id/name/api/base_url/api_key/api_key_env/model/params/models） |
| `[llm.params]`（`LlmParamsToml`） | `provider.params`（每份接口自己的） |
| `[llm.models]`（BTreeMap 手填能力） | `provider.models: Vec<LlmModel>`（每份接口的目录） |
| `[llm.extra_params]`（原样进请求体） | 同一份（不分接口） |
| — | `llm.active: Option<String>`（用哪一份） |

**判定规则**：`active` 为空 / 指向不存在的 id ⇒ 走 legacy 顶层字段（`active_provider()`，约 1429 行）；
`effective_params()`（约 1439 行）在「active 那份的 params 非空」时用它、否则回落顶层 `[llm.params]`。

### 1.2 参数面：**三层概念叠在同一个词上**

1. `[llm.params]`（全局默认）；
2. `provider.params`（当前接口自己的，**与 1 是回落关系**，见 `effective_params`）；
3. `[llm.overrides.<link>]`（按链路覆盖，`overrides.of(link)`）—— 但 **`pub type LlmOverride = LlmParamsToml;`**（约 1218 行），
   也就是说**链路覆盖只管请求参数，管不了「用哪个接口 / 哪个模型」**。

`for_link(link)`（约 1412 行）只把参数合并；模型与接口永远来自「当前 active 那一份」。

### 1.3 WebUI：**同一页两种形状，由数据决定**

`webui/src/views/SettingsView.vue`（1,450 行）里 AI 模型子页（`section === 'llm'`，172 行起）分五块：
快速配置卡（`LlmQuickSetup`）→ 折叠的高级（接口 / 模型能力 / 请求参数矩阵 / 运行时 / 提示词）。

- **换脸判据**：`const llmProviderMode = computed(() => Array.isArray(form.llm.providers))`（633 行）。
  于是「接口」整块**只在有 providers 时渲染**，「模型能力」整块**只在没有 providers 时渲染** ——
  同一个页面两套完全不同的形状，而切换开关**不在界面上**（取决于 `llm.toml` 里有没有那个数组）。
- **文案自己解释自己**：全局默认那行的提示写着「（{{ llmProviderMode ? '当前接口的参数' : '[llm.params]' }}）」（267 行）——
  一句话里出现两个来源，靠三元表达式现场说明。
- **保存跨两个文件**：`saveLlm()`（1,274 行）先 `PUT /api/config/llm`，**再** `PUT /api/config/global` 写提示词。
  代码注释自己写着：「提示词那一组写的是 config.toml 的 [detection]，不是 llm.toml —— 只是摆在这一页」，
  所以「保存 LLM」这个按钮实际会写两个文件、其中一块字段属于另一个页面（程序配置）。
- **模型选择入口至少三处**：`LlmQuickSetup` 的「获取模型」、`LlmProviders`（658 行）的行内弹层 + 「管理模型」、
  以及 `llm.models`（legacy 能力表）。同一件事三种形态。
- **并发数是速度第一杠杆，却藏在「高级 → 运行时」最底下**（`SliderField`，`max_concurrent`）；
  而 `crates/yekitrim-llm/AGENTS.md` 自己写着它是「批处理速度的第一杠杆」。
- **extra_params 是裸 JSON textarea**（`rows=10`，无校验、无示例、优先级只靠文案）。

### 1.4 端点（现有能力够用，只是形状绑死在"整份 patch"上）

`GET /api/config`（含 `llm_config` + `llm_raw`）· `PUT /api/config/llm`（整份 patch）·
`PUT /api/config/global`（提示词在 config.toml）· `GET /api/llm/models` · `GET /api/config/llm/{catalog,dialects,protocols}` ·
`POST /api/llm/test`（**存整份之后才测**，见 §2-⑨）。
按账号分文件：`owner::state_dir(owner)/llm.toml`（`api/config.rs::config_llm_put`，390 行）；
「AI 模型」子页**不是 admin-only**（`utils.js::SETTINGS_SECTIONS` 里只有接入/系统带 `adminOnly`）—— 这点是对的，别改。

## 2. 问题清单（按"会不会让人配错"排序）

| # | 问题 | 证据 |
|---|---|---|
| ① | **两套真相**并存，且写回形状不固定（老文件旧形状、新文件新形状） | §1.1；`active_provider()` / `effective_params()` 的回落链 |
| ② | **不能用不同模型跑不同链路**（检测/精修/标题只能共用一个 model） | `pub type LlmOverride = LlmParamsToml`（1218）+ `for_link` 只合并参数 |
| ③ | **页面形状由数据决定**，用户既看不出自己在哪一档、也没有切换入口 | `llmProviderMode = Array.isArray(...)`（633） |
| ④ | **三个参数面**同名不同源，需要现场三元表达式解释 | 267 行文案；`effective_params()` |
| ⑤ | **一个保存按钮写两个文件**，其中提示词属于另一个页面 | `saveLlm()` 1274–1295 |
| ⑥ | **能力/目录三源**（内置表、探针、手填），UI 里分两种形态 | `catalog::caps_json_merged`；legacy `llm.models` vs `provider.models` |
| ⑦ | **extra_params 裸 JSON**，无校验无示例 | 模板 `rows=10` textarea |
| ⑧ | **模型选择三处入口**，各自一套弹层 | `LlmQuickSetup` / `LlmProviders` / `llm.models` |
| ⑨ | **测试连接必须先把整份存上**（想试一个 key 得先落盘） | 存完才能 `POST /llm/test`；provider 行内的 `testOne` 也读表单值 |
| ⑩ | **术语四套**：接口 / provider / 配置 / 链路 / 能力 / 参数 / 目录，同页混用 | 见全套文案 |

## 3. 目标设计（建议）

**一句话原则**：*一份真相、两级参数、链路能选模型；默认路径三个字段能跑通。*

### 3.1 数据模型 v2（`llm.toml`，config_version 3 → 4）

```toml
[llm]
active = "main"          # 唯一真相：当前用哪一份接口（永远有值）
max_concurrent = 3       # 提到顶层显眼处（速度第一杠杆）

[llm.defaults]           # 原 [llm.params]：所有链路的默认请求参数（**两级里的第一级**）
temperature = 0.7
max_tokens = 4096
json_mode = true

[llm.links.detect]       # 每条链路可覆盖：接口 + 模型 + 参数（**两级里的第二级**）
provider = "cheap"       # 空 = 跟随 llm.active
model = "deepseek-chat"  # 空 = 跟随该接口的默认模型
temperature = 0.3

[[llm.providers]]
id = "main"
name = "DeepSeek"
protocol = "openai-chat" # 原 api（改名，与 /config/llm/protocols 的 id 一致）
base_url = "https://api.deepseek.com"
api_key_env = "DEEPSEEK_KEY"   # 或 api_key（明文，现状保留）
model = "deepseek-reasoner"

[[llm.providers.models]]       # 能力/目录**只留这一处**
id = "deepseek-chat"
input = ["text"]
max_output_tokens = 8192
```

关键取舍：
- **砍掉 `provider.params`**（当前那层回落是"三层"的根源）→ 只留 **默认 + 链路覆盖** 两级；要按接口区分就写成两条链路覆盖。
- **`[llm.overrides.*]` → `[llm.links.*]`**，并新增 `provider` / `model` 两个字段（**这是新能力，要你拍板**）。
- **`llm.models`（legacy 顶层能力表）折叠进 `providers[].models`**；迁移时挂到当时那份接口下。
- `extra_params` 保留在 `[llm]`，语义不变（优先级最低），但 UI 降级为"高级 JSON"并有校验。

### 3.2 迁移（R4 不变式：老文件**读得动、跑得对**）

- **读**：v3 老文件原样可用（现有兼容代码不动）；`providers` 为空时按 legacy 解析。
- **写**：只写 v2 形状 —— 首次保存（或显式 `yekitrim config migrate`）时：
  `api_key/base_url/model` → `providers[0]`（id = `main`，name = `默认接口`）；`[llm.params]` → `[llm.defaults]`；
  `[llm.overrides.*]` → `[llm.links.*]`；`llm.models` → `providers[0].models`；旧键登记 `retired_in`（`version.rs`）。
- **写盘前留备份**：`llm.toml.bak-v3`（配置卡既有规矩：改键必须登记迁移 + 备份）。
- ⚠️ **迁移不许丢 `api_key` 明文**（它是唯一不可重建的东西）——迁移单测必须钉住「迁移前后 key 逐字节相同」。

### 3.3 WebUI 信息架构（一页四块，从上到下 = 从常用到罕见）

1. **当前接口**（常显，占首屏）：接口下拉（= `active`）+ 名称 + `base_url` + 密钥（密文框 + 「已设置」徽标）+
   模型选择（一处！带 catalog/探针提示）+ 一行能力 chip + **「测试这个接口」**（不落盘就能测）。
   右上角：`管理接口…`（抽屉里增删/改名/复制）。
2. **默认参数**（常显）：温度 / 最大输出 / JSON 模式 / 推理档位（按 caps 显示）+ **并发数**（从"运行时"提上来）。
3. **按链路覆盖**（表格，默认全"跟随默认"）：行 = 检测 / 精修 / 标题 / 工具（订正隐藏但保留键）；
   列 = **接口** / **模型** / 温度 / 最大输出 / 推理 / 状态；未覆盖的格子显示灰字实值（来自新的 `/effective` 端点），
   点一下才变成可编辑 —— 一眼能看出「哪条链路跟谁、谁被改过」。
4. **提示词**（折叠，且**只在它属于自己的页面出现**）：建议搬回「程序配置」页（见 §5 问题 2）；
   若保留在本页，必须给它**独立的保存按钮**与"写 config.toml"标注，绝不与"保存接口"共用一个按钮。

删除：`LlmQuickSetup` 与 `LlmProviders` 里重复的模型弹层、`llmProviderMode` 分支、legacy 那一支的独立形状。

### 3.4 端点增量（都不破坏现有契约）

- `GET /api/config` 增加 `llm_schema_version`（3/4）与 `llm_effective`（合并后的生效值：每链路的 provider/model/params）。
- `GET /api/config/llm/effective?link=detect`：给表格填灰字实值（**前端不再自己算**）。
- `POST /api/llm/test` 接受**未保存的草稿**（body 里带 base_url/key/model，或 `{provider_id}` 读已存那份）：
  现在"想试 key 必须先保存"是第 ⑨ 条问题的根。
- `PUT /api/config/llm` 接受 v2；**收到 legacy 顶层键 → 400 + 指路**（`配置已升级到 v2，请用 providers/defaults/links`）。

## 4. 为什么这样改（对应的病根）

| 病根 | 改法 |
|---|---|
| 两套真相 | providers 唯一存储 + 迁移 + 只写 v2 |
| 三层参数 | 砍成两级（默认 + 链路） |
| 页面靠数据换脸 | 只有一种形状：接口下拉 + 参数 + 链路表 |
| 一处保存写两文件 | 提示词搬走或独立保存 |
| 能力/目录多源 | 只留 `providers[].models`，catalog 只做**建议值**（"表里有这条，一键填入"） |
| 模型选择三处 | 统一 `LlmModelPicker`，只出现在"当前接口"卡 |

## 5. 需要你拍板的六件事

1. **要不要支持「按链路选不同模型/接口」？**（本文建议：要。否则"检测用便宜模型、精修用贵的"永远做不到 —— 这是当前最大的功能缺口）
2. **提示词留在 AI 模型页，还是搬回「程序配置」页？**（建议搬回；它与 LLM 接口配置没有归属关系，只是恰好都喂给 LLM）
3. **要不要砍掉 `provider.params` 这一层**（只留默认 + 链路覆盖）？（建议砍：三层是 ④ 号问题的根源）
4. **`extra_params` 保留吗？**（建议保留但降级：高级 JSON + 校验 + 「优先级最低」说明）
5. **密钥怎么存**：继续明文 `llm.toml`（现状，简单但落盘明文），还是默认 `api_key_env` + 密文框只显示「已设置」？（安全性 vs 易用性；`api_key_env` 已经在结构里了）
6. **兼容策略**：老文件「只读兼容、首次保存写新形状」可以吗？（还是要求一次性 `yekitrim config migrate`，之后**禁止**写 v3 形状）

## 6. 分期计划（每期独立可发版、独立验收）

| 期 | 内容 | 体量 | 验收 |
|---|---|---|---|
| **L0** | 本文（口径确认：§5 六问） | — | 六问有答案 |
| **L1** | 后端数据模型 v2 + 迁移 + 兼容读（**不碰 UI**） | 2–3 天 | 老 llm.toml **逐字节**仍可读可跑；迁移后 key 不丢；`config_version` 3→4 有单测；`cargo test --workspace` 全绿 |
| **L2** | 端点增量（`/effective`、草稿测试、v2 写入校验） | 1–2 天 | 未保存草稿能测通；legacy 键写入 400 并指路；`check-api-contract` 前向 0 差集 |
| **L3** | WebUI 新 IA（前三块 + 链路表格），旧表单暂留可切 | 3–5 天 | `npm run build`；手工：填 key→测试→保存→刷新不丢；两条链路不同模型，日志里 `model=` 各不相同 |
| **L4** | 删旧分支（`llmProviderMode`、`LlmQuickSetup` 重复模型选择、legacy 形状）+ 文档与卡更新 | 1–2 天 | `check-simplify` / `check-layout-invariants` 绿；`docs/webui-capabilities.json` 的 `data-cap` 同步；`gate --all` 0 条非基线失败 |

**依赖**：L1 必须先于 L3（UI 改的是新形状）；L2 可与 L1 并行；L4 要等 L3 稳定一版。

**风险与对策**：
- `llm.toml` 含**明文 API key** ⇒ 迁移前备份 `.bak-v3`，单测钉「key 不丢」；
- 按链路模型要在 `for_link()` 处解析并落到 `LlmClient`（`LlmConfig::for_link` 需返回"接口+模型+参数"三件），
  这是本轮唯一**有行为风险**的改动 —— 用「两条链路不同 model，日志逐条断言」的用例钉住；
- UI 大改要保 `data-cap` 契约不破（`check-simplify.mjs` 会逐条查）。

## 7. 不在本提案范围

- 换协议/加新 provider 协议（`protocol` 已有注册表 `PROTOCOLS`，不在这次重构里动）；
- LLM 用量统计（`LlmUsage.vue` / `llm_usage` 表）的表单化改造；
- 检测/精修页面里的"用哪个模型"入口（等 L1 定了模型归属再谈，可能收敛到一处）。
## 8. 修订：2026-11-25 用户口径（D1–D6）与由此产生的设计增量

> 本节覆盖前文的对应条目；前文保留作背景与证据。**仍未定的是「provider.params 砍不砍」（见 §8.5）**。

### 8.0 拍板结论

| 编号 | 结论 | 对设计的影响 |
|---|---|---|
| **D1** | **要**按链路选不同模型/接口；并且**必须新增一个「模型列表」**，否则无从分配 | §8.1 新增顶层 `[[llm.models]]`；`providers` 不再挂 models |
| **D2** | 提示词**留在 AI 模型页**，但要**显示在「高级」之外**，作为**独立的默认折叠块** | §8.2 IA 调整为五块（提示词与「高级配置」平级） |
| **D3** | （待定）provider.params 冲突的详细解释见 §8.5 | 影响 §3.1 的参数层数 |
| **D4** | `extra_params` **保留但要拆分**成结构化设置（如 `reasoning_effort` / `thinking.type`），不再靠手写 JSON | §8.3 说明「哪些已经结构化、哪些要补、extra_params 退化成什么」 |
| **D5** | 密钥存储**按当前做法**（明文 `llm.toml` + 可选 `api_key_env`） | §3.4 的"密文只显示已设置"改为**不做** |
| **D6** | **不需要兼容**老形状 | §8.4 迁移策略：v2 为唯一写形状，一次性迁移，不做"老文件一个字节不改"（R4 取消） |

### 8.1 增量一：**模型列表**（D1 的前置）

现在的模型散在三处（legacy `[llm.models]` 表、`provider.models` 目录、各处的"获取模型"弹层），**没有一张能用来"分配"的清单**。新增顶层：

```toml
# 模型清单：一份真相。链路/接口都只引用它的 id。
[[llm.models]]
id = "deepseek-chat"        # 发给 API 的模型 id（也是清单主键）
provider = "main"           # 归属哪份接口（`[[llm.providers]].id`）
label = "DeepSeek V3"       # 显示名（可空，空则用 id）
# 能力面：catalog 提案 + 探针结果 + 手改，三者合并后**落在这里**
input = ["text"]
max_output_tokens = 8192
thinking_wire = "thinking_type"   # 与 yekitrim_llm::dialect 的 DIALECTS 同集合
```

- `[[llm.providers]]` **不再有 `models` 字段**（避免两份清单）——它只描述"怎么连"（id/name/protocol/base_url/key）。
- `[llm.links.<link>]` 用 `model = "<id>"` 引用；`provider` 可省略（由模型的 `provider` 反查）。
- **来源**：`GET /api/llm/models`（按接口列）只用来"导入候选"；`GET /api/config/llm/catalog` 提供能力建议值；
  用户点"加入清单"才落盘 —— 清单**只装用户真的打算用的模型**（当前是每点一次就写进文件，越攒越多）。
- UI 里它是一张**表**（id / 显示名 / 接口 / 能力摘要 / 用量或引用它的链路），链路表格的"模型"列从这里选。

### 8.2 增量二：提示词的位置（D2）

页面改为**五块**，从上到下：

1. **当前接口**（常显）
2. **默认参数**（常显，含并发数）
3. **按链路覆盖**（表格）
4. **模型列表**（折叠，默认收起；但**在「高级配置」之外**，与它平级）
5. **提示词**（**折叠、默认收起、独立块**）：参考标题 / `system_prompt` / `prompt_template`，
   标题行必须写明"写入 `config.toml` 的 `[detection]`"，并有**自己的保存按钮**（不再与"保存接口"共用一个）。

「高级配置」里只剩：协议/端点细节、`extra_params`（高级 JSON）、按接口的能力手改。

### 8.3 增量三：`extra_params` 拆分（D4）

**先说清现状**（避免重复劳动）：**推理那两个例子已经结构化了**，只是 UI 没讲明白：

- `[llm.params].reasoning`（档位 `off|minimal|low|medium|high|xhigh|max`）由 `yekitrim_llm::dialect` **按模型**翻译成 wire：
  `reasoning_effort`（OpenAI o 系）/ `thinking: {type: enabled}`（DeepSeek、Anthropic 兼容层）/ `enable_thinking`（Qwen）/
  `thinking_budget`（Anthropic、Gemini 变体）/ `chat_template_kwargs`（vLLM/SGLang）…（`dialect.rs` 的 DIALECTS 表）；
- `[llm.params].thinking_budget` 是预算型方言的 token 数（不写就按档位映射表给默认，`budget_for`）。

所以 D4 的真实工作 = **把"还有哪些只能手写 JSON"的补成结构化 + 把 extra_params 降级**：

**新增结构化字段**（`LlmParamsToml` 里目前没有、但常用）：
`stop: Vec<String>` · `seed: Option<i64>` · `presence_penalty: Option<f64>` · `frequency_penalty: Option<f64>` ·
`top_k: Option<u32>`（方言相关，不做就留 extra_params）· `service_tier`（OpenAI，可选）。
其中 `response_format` 已经有等价物 `json_mode`，不再重复加。

**`extra_params` 退化后的定位**：
- 只用于"上面没有的、provider 私有的"键（如 `chat_template_kwargs`、`repetition_penalty`、厂商扩展）；
- 优先级**最低**（现状不变：结构化项之后）；
- UI：折叠 + `JSON` 校验（现在就一个裸 textarea）+ **冲突检查**：如果里面出现已知键（`temperature` / `max_tokens` /
  `reasoning_effort` / `thinking` / `response_format`），保存时**拒绝并指路**"请用上面的结构化设置" ——
  否则"手写 thinking"与"档位"会互相打架（现在只有一句注释说"省略时 extra_params.thinking 说了算"）。

### 8.4 增量四：迁移策略（D6：不需要兼容）

- **v2 是唯一写形状**；老文件在**第一次加载/保存**时做一次性迁移（也可 `yekitrim config migrate` 显式触发）：
  顶层 `api_key/base_url/model` → `providers[0]`；`[llm.params]` → `[llm.defaults]`；`[llm.overrides.*]` → `[llm.links.*]`；
  `[llm.models]`（含 provider.models）→ 顶层 `[[llm.models]]`；`extra_params` 原样保留。
- **不再保留"老形状读得动、一个字节不改"那套 R4 兼容**（D6）。但**迁移必须幂等 + 不丢 key**，写盘前 `.bak-v3`。
- 读取时若仍看到 legacy 键（迁移没跑到）⇒ **warn + 自动迁移**，不再静默按回落链解析；
  `config_version` 3 → 4，旧键登记 `retired_in`（`config/version.rs`）。

### 8.5 待定：`provider.params` 到底砍不砍（D3 的详细解释）

**现在的规则**（`config/types.rs::effective_params`，约 1439 行）：
> `active` 那份 provider 的 `params` **非空**就用它，**否则整体回落**顶层 `[llm.params]`。

注意关键词是**整体**：它**不是逐键深合并**，而是一把开关。于是有四种会让人配错的现场：

| 现场 | 结果 | 为什么危险 |
|---|---|---|
| 老文件有 `[llm.params] temperature=0.7, max_tokens=4096`；新加一份 provider 只填了 `temperature=0.3` | 生效值 = **只** temperature=0.3，`max_tokens` **变成没配** | 一个正常编辑动作就**悄悄丢了**输出上限；不再发 `max_tokens` 后由 provider 决定上限，费用/延迟/截断行为都变，而界面不会说 |
| 两份 provider 各写一份 params | 切 `active` = 整体换一套 | 用户以为是"同一套参数、按接口微调"，实际是**两套独立配置**，某一套漏了 `json_mode` ⇒ 检测链路突然不走 JSON ⇒ 解析失败率上升 |
| 链路覆盖只写了 `temperature` | 其余键来自 `effective_params`（可能已经是"缺 max_tokens"的那份） | 故障点与真正的原因隔了两层，日志里只看到"没发 max_tokens" |
| 界面文案 | 「全局默认（当前接口的参数）」 | 一句三元表达式：同一格在不同模式下写进**两个不同的 TOML 位置**（provider 模式写 `[[llm.providers]].params`，legacy 写 `[llm.params]`），用户无法知道自己改的是哪份 |

**三个选项**：

| 选项 | 做法 | 优点 | 缺点 |
|---|---|---|---|
| **A（建议）** | **砍掉 `provider.params`**：请求参数只有两级 —— `[llm.defaults]`（逐键基准）+ `[llm.links.*]`（逐键覆盖） | 概念最少；"每接口微调"用链路覆盖表达（一个链路本来就绑一个接口+模型）；不会出现"整体遮蔽丢键" | 如果将来要"同一接口在不同链路用不同参数"，得靠链路覆盖（本来也是这么写） |
| B | 保留但改成**逐键合并**（provider.params 的 `Some` 才覆盖 defaults） | 保住"接口级默认"；不会丢键 | 仍然是两层同名概念；UI 要解释"这格子来自哪层"；与链路覆盖叠成三层优先级 |
| C | 保持现状（整体遮蔽），只在 UI 加警告 | 零改动 | "不直观"的根源之一原样保留 |

**我的建议是 A**。理由：请求参数描述的是"这次调用怎么发"，与"连哪一家"无关；而链路覆盖已经能表达"这条链路想怎么发"，
再在接口上放一份参数，就只是给"同一件事"提供第二个入口。

### 8.6 分期（按 D1–D6 修订）

| 期 | 内容 |
|---|---|
| **L1** | v2 模型：`providers` / `llm.defaults` / `llm.links.*`（含 `provider`+`model`）/ **顶层 `[[llm.models]]`** / `extra_params` 保留；`config_version` 4 + 一次性迁移（幂等、`.bak-v3`、key 不丢）；**D6：删掉 legacy 回落链** |
| **L2** | 端点：`/api/config/llm/effective`（每链路生效值）、`/api/llm/test` 支持草稿、`PUT /api/config/llm` 只收 v2（旧键 400 指路）、`GET /api/config` 带 `llm_schema_version` |
| **L3** | WebUI 五块 IA（接口 / 默认参数 / 链路表格 / **模型列表** / **提示词独立折叠**）；模型选择只保留一处（从模型列表选）；extra_params 折叠 + JSON 校验 + 已知键冲突拒绝 |
| **L4** | 删 `llmProviderMode` 分支与 `LlmQuickSetup` 重复弹层；文档/卡/data-cap 同步 |

L1 里**唯一有行为风险**的是「链路绑 provider+model」—— 要在 `for_link()` 处解析并让 `LlmClient` 真的用那条模型，
用「两条链路不同 model，日志里 `model=` 各不相同」的用例钉住。
