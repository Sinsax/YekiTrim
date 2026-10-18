# YekiTrim 模型能力表（`data/llm-models.json`）

**这份文件不是本仓的原创数据**：它由 `scripts/gen-llm-models.mjs` 从
[`@earendil-works/pi-ai`](https://github.com/earendil-works/pi) 的模型目录**抽取**而来，只取数据、不引其运行时。
**不许手改** `llm-models.json`（改了下一次 `--check` 就会红）；要改内容就改本文件里记的筛子，
再重跑生成脚本。

## 归属与来源

| 项 | 值 |
|---|---|
| 上游包 | `@earendil-works/pi-ai` |
| 上游版本 | `0.82.1`（`package.json` 的 `version`，写进产物 `generated_from.version`） |
| 入库副本 | **`vendor/pi-ai-models/`** —— 生成器的默认源（那边的 README 有来源/许可/每文件 sha256/更新步骤） |
| 上游许可 | **MIT**（`package.json` 的 `license`；产物 `generated_from.license` 同值） |
| 上游仓库 | <https://github.com/earendil-works/pi>（`packages/ai`） |
| 作者 | Mario Zechner |
| 抽取的源文件 | `package.json` + 上表 17 家对应的 `dist/providers/data/<provider>.json`（共 18 个文件） |
| 完整性戳 | 上述源文件**按文件名升序**拼接（每份前面写文件名 + `\n`、后面补 `\n`）后的 SHA-256，落在产物 `source_sha256` |

MIT 允许再分发与派生，条件是保留版权与许可声明。**pi-ai 的完整许可正文随 npm 包分发**，本仓不再抄一份：
需要时看安装目录下的 `node_modules/@earendil-works/pi-ai/package.json`（`license: "MIT"`）与上游仓库的 `LICENSE`。
本仓对这份数据的加工（筛选、字段改名、档位裁剪）同样按 MIT 分发；`llm-models.json` **不含** pi-ai 的任何代码。

## 怎么重新生成（源数据已入库，离线可跑）

**源数据在仓库里**：`vendor/pi-ai-models/`（上游 `package.json` + 本表用到的 17 个 `<id>.json`，逐字节原样；
那边的 README 记着来源 / 许可 / 每文件 sha256 / 更新步骤）。所以生成器**不需要外部包目录**，`--pi-ai` 只是可选覆盖：

```bash
node scripts/gen-llm-models.mjs                  # 重写产物（默认源 = vendor/pi-ai-models/）
node scripts/gen-llm-models.mjs --print-analysis  # 同上，额外打印源侧统计（distinct thinkingFormat / 逐模型 wire / 可疑点）
node scripts/gen-llm-models.mjs --check           # 只比对不写（门禁走这条；离线）
node scripts/check-llm-models.mjs                 # 门禁：结构校验 + spawn 上面那条 --check，退出码照搬
node scripts/gen-llm-models.mjs --pi-ai <外部包目录>   # 可选覆盖（本机 npm 缓存等）
```

退出码：`0` 成功/一致，`1` `--check` 不一致，`2` 源目录不可用（会打印「要准备什么」）。
产物**逐字节稳定**：2 空格缩进、LF、末尾一个换行；`providers` 按 `id` 升序、每家 `models` 按 `id` 升序。
两种源目录形态（npm 包的 `dist/providers/data/`，或 vendor 的平铺 `<id>.json`）喂给同一个 sha256 ⇒ 产物一致。

**过期口径**：这份表**不带时间戳字段**（我们不知道数据是什么时候生成的，加 `generated_at` 只会破坏逐字节稳定、
每次重跑留假 diff）。真正的时钟在 git 里 —— **`vendor/pi-ai-models/` 超过 90 天没更新就算过期**
（`git log -1 --format=%cs vendor/pi-ai-models/`），排期照上一节重跑一次；上游发新版或主用模型换代时提前做。

## 选哪些 provider（17 家 / 181 个模型）

上游 `0.82.1` 的 `dist/providers/data` 下有 **37 个** `*.json`。本表只收下面这 17 家
（`scripts/gen-llm-models.mjs` 里的 `PROVIDERS` 白名单，**多一个不抽**）：

| 保留 | 模型数 | 为什么 |
|---|---|---|
| `deepseek` | 2 | 主用模型（字幕订正/事件检测） |
| `openai` | 19 | 通用备选；丢掉了 `o1/o3/o4`、`gpt-4o`/`gpt-4-turbo`、`codex` 系、`realtime`、`chat-latest`、`deep-research` |
| `anthropic` | 11 | 通用备选；丢掉了 `-YYYYMMDD` 日期快照别名（同一个模型的重复条目） |
| `google` | 18 | Gemini 系；丢掉了 `deep-research`/`computer-use`/`live-preview`/`robotics` 与图像专用档 |
| `qwen-token-plan` / `qwen-token-plan-cn` | 15 + 15 | 通义 token plan（国际/国内两个入口） |
| `moonshotai` / `moonshotai-cn` | 10 + 10 | Kimi 系（国际/国内） |
| `zai` / `zai-coding-cn` | 6 + 6 | GLM 系（`api.z.ai` / `open.bigmodel.cn`） |
| `minimax` / `minimax-cn` | 3 + 3 | MiniMax 系（国际/国内） |
| `xiaomi` | 5 | MiMo 系；丢掉 `-ultraspeed` 变体 |
| `groq` | 4 | 便宜快速档；丢掉 Llama 系（与切片任务无关） |
| `cerebras` | 2 | 同上；丢掉 `gemma` |
| `together` | 11 | 一把聚合；丢掉 7B 级小模型与已被取代的型号 |
| `openrouter` | 41 | **只挑常用模型**（见下） |

**丢掉的 20 家与理由**（`amazon-bedrock` 114、`azure-openai-responses` 38、`google-vertex` 12、
`cloudflare-ai-gateway` 42、`cloudflare-workers-ai` 13、`github-copilot` 29、`huggingface` 50、`nvidia` 18、
`vercel-ai-gateway` 192、`opencode` 58、`opencode-go` 16、`fireworks` 16、`mistral` 30、`xai` 3、
`xiaomi-token-plan-ams/cn/sgp` 各 3、`kimi-coding` 4、`openai-codex` 7、`ant-ling` 3）：

1. **云厂商托管面**（bedrock / azure / vertex / cloudflare / nvidia / huggingface / vercel-ai-gateway）——
   用户要走它们得先有云账号与部署，和「填 base_url + api_key 就能用」的产品面无关；
2. **订阅/代理通道**（github-copilot / opencode* / kimi-coding / xiaomi-token-plan-* / openai-codex）——
   是 IDE 编程套餐的转发口，不是给服务端调用的 API；
3. **另有专属 SDK 协议**（mistral 的 `mistral-conversations`、xai 的 `openai-responses`）——
   wire 形状与 OpenAI 兼容口不同，混进这张表会污染 `thinking_wire` 的含义；
4. `ant-ling` 是上游新增的小众家，与本产品无关（真要加，加一行白名单即可）。

`openrouter`（源里 276 个）只留 **41 个常用模型**：规则写在 `gen-llm-models.mjs` 的 `keep` 正则里
（只收 `openai/gpt-5*`、`anthropic/claude-{opus,sonnet,haiku}-4/5`、`google/gemini-3.x-{pro,flash}`、
`deepseek/deepseek-{r1,v3.2,v4-pro}`、`qwen/qwen3.x-{max,plus,flash}`、`moonshotai/kimi-k2|k3`、
`z-ai/glm-{4.7,5}`、`minimax/minimax-m3`、`x-ai/grok-4.5`），
丢掉了 `~latest` 别名、`:free` 免费层、代码/图像/语音专用档与已被取代的老型号。

## 字段怎么来的

- `thinking_wires` / `max_output_wires`：本表的 wire 词表（产物里的白名单，**不是** pi-ai 的字段名）。
  `thinking_wires = [openai_effort, thinking_type, qwen_enable_thinking, openrouter_effort, none]`，
  `max_output_wires = [max_tokens, max_completion_tokens]`。`none` 当前没有 provider 使用（留作兜底）。
- **wire 是逐模型的，认 `models[].thinking_wire` / `models[].max_output_wire`**（单值字符串，取值同词表）。
  依据：pi-ai 的 `compat` 本来就是**模型级**字段 —— `moonshotai` 的 9 个 Kimi 走 deepseek 语义、
  `kimi-k3` 走 OpenAI 语义，`qwen-token-plan(-cn)` 里 `deepseek-v4-*` 两个型号也是 deepseek 语义。
  只给 provider 级并集的话，消费侧无法知道**哪支模型该用哪个**，只能整体降级成「不发推理参数」⇒
  那些模型的能力全丢。
- `providers[].thinking_wire` / `max_output_wire` 仍保留，是**该家模型级取值的升序并集**（数组，单值时长度 1），
  当摘要/自检用：脚本会断言每个模型的取值都落在这个并集里。合成 `"mixed:xxx"` 这种字符串不在词表里、
  消费侧还得再解析，所以用数组并列。**消费侧的正确读法是先看模型级**；只有拿不到模型级（老版本表）时，
  才退回并集 + 取最保守的那个。
- `providers[].temperature`：源里唯一明确的信号是 `compat.supportsTemperature`（`false` ⇒ `unsupported`）。
  它是**模型级**的（anthropic 只有 `claude-opus-4-7/4-8/5` 写了 `false`），本表按"**任一个模型声明不支持就记
  `unsupported`**"的保守口径汇总 —— 也就是 `anthropic: unsupported` 而它家别的模型其实能收温度。
  源里没有信号时一律 `supported`：**宁可多发一次让它自己拒，也别漏发**。
- `models[].thinking_levels`：**逐模型**照抄上游 `thinkingLevelMap`（值就是该档要在 wire 上写的拼写；
  `null` = 该档支持但不发送）。模型不推理、或没有 `thinkingLevelMap` ⇒ `null`（例如 google 的
  `google-generative-ai` 协议根本不读 `thinkingLevelMap`，它的档位是代码里硬编码的 `LOW/MINIMAL/HIGH`）。
  档位只保留 `off|minimal|low|medium|high|xhigh` —— **上游还有 `max`，本表把它裁掉了**（我们的 UI 只有六档；
  源里 `max` 常与 `xhigh` 同时存在，裁掉不会让任何模型失去"能调档"）。
- `hosts`：该家任一模型的 `baseUrl` 的 host，去重升序；`base_url` 取该家首个模型的值
  （同家内 `baseUrl` 不一致会在 `--print-analysis` 里暴露成多个 host）。

## 已知的源侧不一致 / 可疑处（照实收，不悄悄改）

1. **源里没有 `deepseek-chat`**：`deepseek.json` 只有 `deepseek-v4-flash` / `deepseek-v4-pro`，
   `contextWindow = 1000000`、`maxTokens = 384000`。公开的 DeepSeek Chat 是 131072 / 8192 —— 这两个数
   （以及同源的 mm 系 262144/262144）**明显偏大**，产品里当"上限提示"用，别当计费/限流依据。
2. **同一家内 `thinkingFormat` 不一致**（因此**必须**按模型级 wire 发请求）：`moonshotai`/`moonshotai-cn`
   （`deepseek` ×9 + `openai` ×1 = `kimi-k3`）、`qwen-token-plan`/`-cn`（`qwen` ×13 + `deepseek` ×2，
   即 `deepseek-v4-flash` 与 `deepseek-v4-pro`）。
3. **同一个模型名在不同 provider 下能力不同**：`kimi-k2.6` 走 moonshot 官方口是 `max_tokens = 262144`（等于
   上下文窗口），走 together 是 131000、走 qwen-token-plan 是 262144。**别跨 provider 复用一套数字。**
4. **`maxTokens == contextWindow` 的条目**：moonshot 系 9 个 Kimi（262144/262144）、
   `together/openai/gpt-oss-120b` 与 `gpt-oss-20b`（131072/131072）、`openrouter/openai/gpt-oss-120b`。
   实测上"输出上限 = 上下文窗口"几乎不可能是真的（还要放输入），**发请求前自己夹一层**。
5. **明显偏小**：`openrouter/x-ai/grok-4.5` 的 `maxTokens = 4096`（同代别的都 128000 级），疑似源里抄错。
6. **`forceAdaptiveThinking` 只影响流程、不在本表里体现**：anthropic 的
   `claude-fable-5 / opus-4-6/4-7/4-8/5 / sonnet-4-6/5` 带这个 flag（上游用它决定"走 effort 还是走 budget"）。
   本表统一记成 `thinking_wire = ["openai_effort"]`：我们只需要"把哪个档写成哪个拼写"，
   **要是以后真接 anthropic 原生协议，得按这个 flag 分叉**。
7. `google` 家的 `thinkingLevelMap` 值是 **大写**（`"LOW"`/`"HIGH"`/`"MINIMAL"`），anthropic 家是
   `"xhigh"`/`"max"` 这种小写 —— 照抄即可，别自己 lowercase（wire 上就是那个拼写）。
