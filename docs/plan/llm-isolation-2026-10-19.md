# LLM 隔离 + 参数归一化（2026-10-19）

> 用户口径：「llm 相关的都打包分离在一个模块内，隔离使用」「抄一下 llm-pi-ai 的实现方式」「设好 api 后
> 能直接获取推理强度和模型」。

## 一、为什么要拆

动手前 LLM 的知识散在 **5 个 crate + 前端**：HTTP/重试在 `core::llm`、配置契约在 `core::config`、
检测/订正的 prompt 在 `core::{detection,asr}`、精修五条链路在 `polish::llm`、测试连接在 `server::api`、
落库在 `service::usage`。`polish` 直接 `use yekitrim_core::llm::*` —— 业务 crate 摸到了传输层内部。
后果是**同一个旋钮在两处各写一半**（`[llm] temperature` 在 `build_body` 写死、`extra_params` 再盖一层、
`thinking` 再盖第三层），而且换一个模型要改五个文件。

## 二、成型后的形状

```
yekitrim-llm ←─ yekitrim-core ←─ yekitrim-polish ←─ yekitrim-service ←─ server ←─ cli
        ↑_____________________________|        （polish 直接用，不经 core）
```

`crates/yekitrim-llm`（叶子 crate，**不认识配置文件、不认识任何 yekitrim-\***）：

| 模块 | 职责 |
|---|---|
| `params` | `LlmParams`（唯一的参数面）+ `ReasoningLevel` / `ModelCaps` / `ThinkingWire` / `MaxOutputWire` / `Support` |
| `wire` | ★ **唯一的方言适配层**：`reasoning_effort` / `thinking:{type}` / `enable_thinking` / `reasoning:{effort}` / `max_completion_tokens` |
| `catalog` | 内置能力表 `data/llm-models.json`（17 家 / 181 模型）+ 按 `(host, model)` 查 |
| `client` | HTTP + 重试 + 请求体拼装 + **用量采集的唯一一处** |
| `agent` | 长输入切窗口并发 + checkpoint 续传 + 取消探针 |
| `usage` | 用量记录与 `UsageSink`（落库由上层实现 trait） |

配置 → 参数的换算**写在配置侧**（`impl From<&LlmConfig> for LlmParams`，`core/src/config/types.rs`），
于是依赖方向是 **core → llm**，反过来就成环。

## 三、抄了 `@earendil-works/pi-ai` 的什么（MIT）

| 它的做法 | 我们的落点 |
|---|---|
| `thinkingLevelMap: { level: string \| null }`（`null` = 支持但不发送、**缺档 = 不提供**） | `ModelCaps.levels` |
| `compat.thinkingFormat` / `maxTokensField` / `supportsReasoningEffort` | `ThinkingWire` / `MaxOutputWire` / `Support` |
| catalog 三层（内置表打底 → modelOverrides → models 整表替换） | 表打底 + 模型级覆盖（P3 之后做整表替换） |
| **能力 ≠ 请求默认**（`maxTokens` 只是上限） | `max_tokens: Option<u32>`，`caps.max_output_ceiling` 只做校验 |
| discovery **不落盘**、认识的 route **完全不联网**、只探 OpenAI 兼容 | `GET /api/llm/models`：表命中回 `source: catalog` 且零网络 |
| 未知方言安全降级 | `ThinkingWire::None` ⇒ 不发推理参数（**宁可不发，也不瞎发**） |

**没有抄的**：它的运行时（那是 cordis 插件 + 7 个 `@deepseek-ai/dsh-*` 依赖）。引进来要给 YekiTrim 加一条
Node 侧车，把并发/取消/checkpoint/记账/体积全打乱 —— 只抄数据与契约。

## 四、实录数字（可复跑）

* 能力表：**17 家 provider / 181 模型**，`reasoning=true` 168、带 `thinking_levels` 77（`gen-llm-models.mjs --check` exit 0）
* 逐模型方言分布：`openai_effort` 70 / `thinking_type` 38 / `openrouter_effort` 41 / `qwen_enable_thinking` 26
* **逐模型覆盖真的被读到**：`moonshotai/kimi-k3` = `openai_effort`，同家其余 9 个 Kimi = `thinking_type`
  （provider 级只能是并集，所以判据钉的是模型级优先 —— `catalog::tests::模型级方言优先于_provider_级并集`）
* 搬迁测试条数不变：三个文件的 `#[test]` **13 → 13**；归一化后 `cargo test -p yekitrim-llm --lib` **22 通过**
* `cargo check --workspace --all-targets` **0 warnings**；两条模块门禁 **exit 0**

## 五、分期与提交

| 期 | 内容 | 状态 |
|---|---|---|
| P0 | 落盘工作区 WIP、记基线 | ✅ `bcfdc12` |
| P1 | 抽 `yekitrim-llm`（纯搬迁，逐字节） | ✅ `57b01d9` |
| P2 | 参数归一化 + 能力表 + 方言适配层 | ✅ `6ebce37` |
| P3 | `GET /api/llm/models`（表命中不联网）+ `GET /api/config/llm/catalog` + `api_key` 不回明文 + **2 条边界门禁**（17→19） | ✅ |
| P4 | 前端：模型下拉 + 能力驱动控件（`caps.levels` 驱动档位下拉） | ✅ |
| P5 | 卡片口径统一 + 本文件 | — |

## 六、没做的 / 下次再说的

* **`llm.toml` 的 TOML 实体（`[llm.params]` / `[llm.compat]` / `[llm.models]`）与 `config_version` 迁移**：
  原计划在 P2，实际推迟到 UI 能驱动它的时候一次做完 —— 现在**存量 `llm.toml` 一个字节都不用改**
  （`[llm] temperature/max_tokens/thinking/extra_params` 照旧，换算在 `From<&LlmConfig>` 里）。
  好处是「隔离」与「参数面」可以分别验收；代价是文档里那张 v2 形状暂时只是设计。
* **模态（`input: [text, image]`）**：表里只记 `text`。pi-ai 的口径是"模态是声明不是探测，且宁少不多"，
  我们暂时用不到多模态。
* **`supportsReasoningEffort` / `forceAdaptiveThinking`**：源里有、本表没体现；接原生 anthropic 协议时要按它分叉。
* **已知的源侧不一致**（照实收，未悄悄改，见 `crates/yekitrim-llm/data/NOTICE.md`）：
  `deepseek-chat` 在源里不存在（只有 `v4-flash/v4-pro`）、`openrouter/x-ai/grok-4.5` 的 max_output 疑似抄错、
  `anthropic` 只有 3 个型号写了 `supportsTemperature:false`。

## 七、补记（本文件写完之后又落的四笔）

第六节那条「TOML v2 推迟」**已经做掉了**，下面按提交补全：

| commit | 内容 |
|---|---|
| `bdf0048` | 能力表 **vendor 进仓库**（19 文件 / 201 KB）+ 离线门禁 `check-llm-models.mjs`（`--self-test` 14 项）。**为什么必须做**：原来 `--check` 要外部 npx 缓存里的 pi-ai 包，判据跑不了 CI ⇒ 表会不会腐没人知道。provenance 可复核：17 个数据文件的 sha256 与上游自带的 `.manifest.json` 逐条一致；不带 `--pi-ai` 重跑产物 SHA-256 **逐字节相同** |
| `0e5998a` | **配置 v2**：`[llm.params]` = 请求参数面、`[llm.models."<id>"]` = 能力面（表外模型可手填）、`[llm.overrides.<link>]` 同一套面；`CONFIG_VERSION` 1→2。**迁移对拍**：3 个老文件 × 4 条链路 = **12 条金色请求体**（全部由迁移前的老代码真跑出来）逐字节相同 |
| `f029fd4` | 「获取模型」带**草稿 key**（只走这一次请求、不落盘）。补的是最常见路径：刚填新 key 还没保存就点 —— 以前会拿旧 key 换一个 401，界面上写着「API key 不对」 |
| `798659f` | 新接口进**接线门禁**（`api/llm.js` 的每个 `/api/...` 必须在 `routes.rs` 有对应 `.route(...)`）。97 → **103 项** |
| `385954a` | 平铺字体目录的**真判据**。`stage_font_dir` 的文档里写着「判据：`scripts/check-font-parity.mjs`」，**那个脚本从来不存在** ⇒ 判据是空的。改成 Rust 判据（造 v2 布局假库 → 平铺 → 断言无子目录 + 逐字节相同 + **自证假库确实带一层目录**），变异自证：让平铺结果带一层 ⇒ FAIL，复原 ⇒ 通过 |

### 仍然没做的（与第六节合并看）

* 界面上一处小滞后：`GET /api/config/llm/catalog` 是**纯查表**（故意的，它不读盘），所以刚保存的 `[llm.models]` 手填能力不会立刻反映在「能力已收录」那行字上 —— **出站请求体是对的** |
* 模态只记 text；`supportsReasoningEffort` / `forceAdaptiveThinking` 未体现 |
* 底座 `v0.4.1` 的 tag 未 push（推了才触发 CI 发布）；底座仓那批 web-parity WIP 仍未提交 |
* 字形比 drawtext 时代小 8%（`ass_font_size` 的 1.25 常数）—— 等用户拍板 |
