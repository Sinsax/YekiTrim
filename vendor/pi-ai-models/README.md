# vendor/pi-ai-models —— 能力表的**源数据**（第三方，逐字节原样）

这里放的是 [`@earendil-works/pi-ai`](https://github.com/earendil-works/pi) 的模型目录**原始 JSON**，
给 `scripts/gen-llm-models.mjs` 当输入。**入库的理由**：`--check` 要能进 CI 常驻门禁，
不能依赖开发者本机的 npm 缓存（`D:/Sinsa/link/...` 那种路径不在仓库里，换台机器判据就跑了）。
**只放数据，不放任何代码**：不引 pi-ai 的运行时，也不给它加依赖。

## 来源与许可

| 项 | 值 |
|---|---|
| 上游包 | `@earendil-works/pi-ai` |
| 上游版本 | **0.82.1**（与本目录 `package.json` 的 `version` 同值） |
| 许可 | **MIT**（`package.json` 的 `license`） |
| 上游仓库 | <https://github.com/earendil-works/pi>（`packages/ai`） |
| 作者 | Mario Zechner |
| 取文件路径 | `dist/providers/data/<id>.json` 与包根 `package.json` |
| 入库日期 | 2026-10-19 |

MIT 允许再分发与派生，条件是保留版权与许可声明 —— 完整许可正文在上游仓库与 npm 包里，本目录不重复抄；
本仓对这份数据的加工（筛选、字段改名、档位裁剪）与出处说明见 `crates/yekitrim-llm/data/NOTICE.md`。

## 文件清单（sha256 逐文件）

| 文件 | 字节 | sha256 | 备注 |
|---|---|---|---|
| `package.json` | 2643 | `955aa1caab4c875fc7755abe7cacbf9002d9875c471e8d5af245e495ae1d4596` | 取 version/license 用（不是运行时依赖） |
| `anthropic.json` | 5903 | `fea1226a62396c8d4a3d9c3b7f65d4dc527ac025d05719880418d3a2b0665a69` |  |
| `cerebras.json` | 1426 | `3fd3e19b83ab4be07d27817e25e5be0ffe4158b1394f0311d6a22d9f03847728` |  |
| `deepseek.json` | 1094 | `0dcc807a4e5827b488c6ceac87884ff6e735e01cf4f2ddfec9dd812e6fde041b` |  |
| `google.json` | 9182 | `bcf8a8d59ccbb75bc6fb4a261a37ac4b7c806f40ee7313b4411178964be5fa37` |  |
| `groq.json` | 2683 | `868c90a897f75f866e09a585f6aee5cb991e40a9c3281497b5842a23de7d9add` |  |
| `minimax.json` | 948 | `253edd4c910a41c8bb252400ba099f3a4de0a8900ce154319288ab091b372987` |  |
| `minimax-cn.json` | 963 | `8ccd71ff838f4b78bb88809babb0dc32a4fcfaec2bc4f4083841000428df4f77` |  |
| `moonshotai.json` | 5117 | `d8d5209873058ddccd37c1f026833e2ea3f9476c3dc9628b4a940d407acc0b2e` |  |
| `moonshotai-cn.json` | 5147 | `1e6146ff3477883636448c0f44222f0a52b3c4562ef8f4eecf7e941cdd59d12e` |  |
| `openai.json` | 17105 | `8e3852ef3567b23d2ab03b0f52f1caeb77e6e776f248e6111f851ccbe0fca8b6` |  |
| `openrouter.json` | 110577 | `bc3e1ea3de6cb962c92ec3bd45f4e73c85ba2ad62ba824253498b41f8acc2b1e` | 只用了其中 41 个常用模型，但**整份**入库（哈希吃整文件） |
| `qwen-token-plan.json` | 6651 | `597fae649f549a8ffb0b6a28426ea6761b7ece41a78b6fdef49ca970ac029970` |  |
| `qwen-token-plan-cn.json` | 6636 | `a1495ccc21835c51d438207652f948aa9ecf4a3d784e81deaf313e484bdab1ec` |  |
| `together.json` | 9377 | `1d46a57e99da9be6f7e78d8f98f5610adbb266df3de7b0a13652d6fa175086e7` |  |
| `xiaomi.json` | 2381 | `810368d3828c8bb9498012a0d48639030c0c216b8c13e79da02daa515aa23448` |  |
| `zai.json` | 2601 | `474670a0cf1109f4c296ea93d2c5c6eb8f53541dc9c0bddbb0e6cb054b971ebe` |  |
| `zai-coding-cn.json` | 2709 | `28fcb04ce65b4c6612a6b866902e0c832e18b685e5374578c4e2872569131d39` |  |

合计 **193143 B**。这 18 个文件的 sha256 **与上游自带的 `dist/providers/data/.manifest.json` 逐条一致**
（上游 `schemaVersion: 3`，`generatedAt: 2026-07-25T12:44:19.521Z`）—— 也就是说这些文件是**原样**搬进来的，
模型条目里的**输入模态 `input`**（`["text"]` / `["text","image"]`）同样**来自这些源文件** —— 产物
`crates/yekitrim-llm/data/llm-models.json` 的 `models[].input` 就是从这里原样搬的（只去重 + 固定顺序）。
没有重新格式化、没有裁剪字段。任何人可以用一行命令复核：

```bash
# 与上游 manifest 对拍（把 <pi-ai> 换成任意一份 0.82.1 的安装目录）
node -e "const f=require('fs'),p=process.argv[1];const m=JSON.parse(f.readFileSync(p+'/dist/providers/data/.manifest.json','utf8')).files;const c=require('crypto');let bad=0;for(const[n,h]of Object.entries(m)){const t=f.readFileSync('vendor/pi-ai-models/'+n);if(c.createHash('sha256').update(t).digest('hex')!==h){console.log('MISMATCH',n);bad++}}console.log(bad?'FAIL':'OK all match')" <pi-ai>
```

## 谁在用它

```bash
node scripts/gen-llm-models.mjs                 # 默认源就是本目录 → 重写 crates/yekitrim-llm/data/llm-models.json
node scripts/gen-llm-models.mjs --check          # 离线比对：0 一致 / 1 不一致 / 2 源不可用
node scripts/check-llm-models.mjs                # 门禁：spawn 上面那条 --check 并照搬退出码
node scripts/gen-llm-models.mjs --pi-ai <外部包目录>   # 可选覆盖（本机 npm 缓存等）
```

两种源目录形态喂给**同一个 sha256**（哈希只吃文件名 + 文件内容，不吃路径）⇒ 用本目录或用外部 npm 包
生成出来的产物**逐字节一致**。这一点实测过：换源重跑后产物 SHA-256 不变。

## 怎么更新（换到更新的 pi-ai 版本）

1. 拿一份新版本（三选一）：
   * 本机缓存：`node scripts/gen-llm-models.mjs --pi-ai <npm-cache>/node_modules/@earendil-works/pi-ai`
   * 现场装：`npm i --prefix <tmp> @earendil-works/pi-ai`
   * 源码包：<https://github.com/earendil-works/pi> 的 `packages/ai`
2. 覆盖本目录：`package.json` + **下面这 17 个** `<id>.json`：
   `anthropic` `cerebras` `deepseek` `google` `groq` `minimax` `minimax-cn` `moonshotai`
   `moonshotai-cn` `openai` `openrouter` `qwen-token-plan` `qwen-token-plan-cn` `together`
   `xiaomi` `zai` `zai-coding-cn`（清单的权威在 `scripts/gen-llm-models.mjs` 的 `PROVIDERS` 白名单）。
3. 重跑生成器，**逐条看 diff**：
   ```bash
   node scripts/gen-llm-models.mjs --print-analysis   # 会打印 distinct thinkingFormat / 逐模型 wire / 可疑点
   ```
   重点看：家数与模型数有没有异常跳变、`thinking_wire` 有没有出现新值（新值要在 `THINKING_WIRES` 词表里登记）、
   `data/NOTICE.md` 里记的"源侧可疑处"是不是被上游修掉了。
4. 更新本文件：版本号、日期、上面那张 sha256 表。
5. `node scripts/check-llm-models.mjs` 必须过（它会连产物 `generated_from.version` 一起校验）。

## 过期口径（没有时间戳字段，这是故意的）

**判定：本目录超过 90 天没更新（看 `git log -1 --format=%cs vendor/pi-ai-models/`）就算过期**，
排期重跑一次上面的更新流程；上游发新版本、或某个模型明显换代了（比如表里主用模型已被取代）时提前做。

为什么**不**在产物里加 `generated_at` 之类的时间戳字段：

1. **我们不知道生成时间**。能拿到的时间只有"这次跑脚本的时刻"，那是**生成时刻**不是**数据时刻**；
   把它写进产物会让同一份源数据每次生成都得到不同文件（破坏逐字节稳定 = `--check` 失去意义、
   每次重跑都留一个假 diff、多分支必冲突）。
2. **真正的时钟在 git 里**：`vendor/` 与产物的提交时间就是"这份数据什么时候进来的"，
   而且是**唯一没有歧义**的那个时间（谁都能查、不用信任产物自称）。
3. 过期是**排期问题**，不是**数据字段问题** —— 判据放在这条 README 里（90 天 + 版本漂移），
   由人和 CI 的节奏管，不塞进机器判据去假装精确。

## 为什么只有这 17 家

上游 0.82.1 的 `dist/providers/data/` 有 **37 个** `<id>.json`。这里只 vendor 我们**真的会读**的 17 家；
剩下的 20 家（`amazon-bedrock` 46 KB、`vercel-ai-gateway` 64 KB 这类大块头也在其中）**故意不收**：
云厂商托管面 / 聚合网关 / 编程订阅通道，与"填 base_url + api_key 就能用"的产品面无关。
取舍逐条理由（含每家多少模型）写在 `crates/yekitrim-llm/data/NOTICE.md`；
要新增一家 = 先改 `gen-llm-models.mjs` 的 `PROVIDERS` 白名单，再把它的 `<id>.json` vendor 进来。

