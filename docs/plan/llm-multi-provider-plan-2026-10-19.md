# LLM 多配置 + 可折叠（参考 DSH 的编排，2026-10-19）

> 用户口径：「参考一下 dsh 的配置方式，**可以配置多个，可折叠**。每个模块的配置也要**精简紧凑**一点」
> 参考实拍：DSH 的 provider 列表（`command` / `step_plan` / `wb2api`，各带 `自定义` 徽章 + 绿点 + 编辑/删除），
> 展开后是：API 密钥 / 自定义设置（折叠）/ 显示名称 / API 地址 / **API 协议**（下拉）/ 模型目录（恢复默认 · 获取可用模型）/
> 模型行（id + 显示名 + 折叠 + 删除）/ `+ 添加模型`；模型里再有：上下文窗口 / 最大输出 / **输入类型（文本☑ 图片☑）** /
> **推理等级**（开关 + 档位行：`档位` 下拉 + `API 参数值` 输入 + 删除）/ `+ 添加档位`。

## 一、为什么这是数据模型问题，不是样式问题

我们现在是**一个全局 `[llm]`**（`api_key` / `base_url` / `model` + `[llm.params]` + `[llm.models."<id>"]`）。
DSH 那套是**多份命名 provider**，每份带自己的模型目录与逐模型能力。要"能配置多个"，TOML 必须变成列表。

## 二、目标形状（`llm.toml`）

```toml
[llm]
active = "wb2api"            # 哪一份在用；**空 = 走 legacy 顶层字段**（R4，老文件一个字节不用改）
max_concurrent = 3           # 仍是全局（并发是进程级旋钮，不分 provider）

[[llm.providers]]
id      = "wb2api"           # 稳定 id（目录名/引用用；ASCII）
name    = "wb2api"           # 显示名（可中文）
api     = "openai-chat"      # **协议**（见第三节；这是"接入更多标准"的入口）
base_url = "http://192.168.0.107:7863/v1"
api_key  = "sk-…"            # 或 api_key_env = "…"
model    = "cn:deepseek-v4.1-flash"   # 当前选中的模型 id（必须在该 provider 的模型目录里）

  [llm.providers.params]     # 该 provider 的请求参数面（与 [llm.params] 同形状）
  reasoning   = "max"
  temperature = 0.3
  max_tokens  = 16384

  [[llm.providers.models]]   # 该 provider 的模型目录（表里没有的模型也能手填能力）
  id = "cn:deepseek-v4.1-flash"
  name = "Deepseek-V4.1-Flash"
  context_window = 1000000
  max_output = 393216
  input = ["text", "image"]
  reasoning = true

    [[llm.providers.models.levels]]    # 档位 → API 参数值（把 DSH 的"推理等级"表搬成数据）
    level      = "max"
    wire_value = "max"

[llm.overrides.<link>]       # 四条链路的覆盖，**形状不变**
```

**R4 铁律**：`active` 为空（或整份文件没有 `[[llm.providers]]`）⇒ 走原来的顶层字段与 `[llm.models."<id>"]`，
行为与现在**逐字节相同**。12 条金色请求体判据必须继续绿。

## 三、`api` 字段就是我们的"协议注册表"入口

DSH 那个下拉（`OpenAI Chat Completions`）正是我们 `dialect`/协议该待的位置。**别把它做成硬编码枚举**：
```rust
pub struct Protocol { pub id: &'static str, pub label: &'static str,
                      pub chat_path: &'static str,   // "/chat/completions"
                      pub models_path: &'static str, // "/models"
                      pub auth: AuthStyle }          // Bearer 头 / x-api-key 头 …
pub const PROTOCOLS: &[Protocol] = &[ … ];   // 现在只 openai-chat，加一家 = 加一条
```
* 现有 `dialect`（推理参数怎么写）**保持不动** —— `api` 是"请求怎么发"，`dialect` 是"参数怎么拼"，两层分开；
* `auth` 现在就两条：`Bearer`（`Authorization: Bearer`）与 `x-api-key`（Anthropic 那族），足够覆盖主流；
* ⚠️ **本轮只做 `openai-chat`**，但把注册表与字段立起来 —— 以后接 Gemini/Anthropic 原生时是"加一条 + 一份请求体映射"，
  不是"改配置结构"。

## 四、前端结构（照 DSH 的层级，别自创）

```
LLM 配置
├── [provider 列表]  ← 每行：名称 + 「自定义/内置」徽章 + 状态点 + 编辑 / 删除
│     • 点「编辑」展开那一份（**同一时间只展开一份**，可再点折叠）
│     • 底部 `+ 添加配置`
└── 展开后（紧凑，一节一行）
      ├── API 密钥（留空 = 不修改，title 说明）
      ├── ▾ 自定义设置（**默认折叠**：显示名称 / API 地址 / API 协议下拉）
      ├── 模型目录  [恢复默认] [获取可用模型]
      │     └── 每个模型一行：id + 显示名 + ▾ 展开 + 🗑
      │           └── 展开后（紧凑两列）：上下文窗口 / 最大输出 / 输入类型（文本☑ 图片☑）/
      │                 ▾ 推理等级（开关 + 档位行：档位下拉 + API 参数值 + 🗑）+ `+ 添加档位`
      └── `+ 添加模型`
```

**紧凑化的具体口径**（用户原话"精简紧凑一点"）：
* 一节一行，标签左对齐定宽（不再一行一个全宽 label 加大片空白）；
* 次要项（显示名称/协议/上下文窗口等）默认收进「自定义设置」折叠里；
* 同一个解释**只说一次**（上一轮已把 notes 24→10，这次要按 provider 维度再收）；
* 模型/档位行用**紧凑行**（高度 ≤ 32px，操作按钮图标化 + `title`）。

## 五、分期

| 期 | 内容 | 验收 |
|---|---|---|
| **P1 数据模型** | `[[llm.providers]]` + `active` + `Protocol` 注册表 + 迁移（`config_version` +1：把现有顶层字段**搬成一份 provider**，`active` 指向它）+ `From<&LlmConfig>` 走新模型 | 老文件读出来**行为不变**（12 条金色请求体）；新旧两种写法各一条迁移用例；`cargo test --workspace` 全绿 |
| **P2 接口** | `GET /config` 回 `llm_config.providers`；`PUT /config/llm` 支持整份 provider 列表（深合并数组=替换，要钉一条用例） | 数组替换语义有单测；`api_key` 仍不回明文、空串=不修改 |
| **P3 前端** | provider 列表 + 折叠 + 模型目录 + 每模型能力/档位（照第四节），紧凑化 | 16 条 webui 判据 + 契约探针全绿；**折叠状态机**有几何断言（展开/折叠各一次）；900px 不溢出 |

## 六、明确不做（本轮）
* 只实现 `openai-chat` 协议（注册表立起来，别的先不接）；
* 不做"多 provider 同时用于不同链路"（`overrides` 仍指向当前 `active` 那一份的参数面）；
* 不做 provider 级别的连通测试矩阵（`测试连接` 仍测当前 active）。
