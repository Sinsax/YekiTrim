---
name: yekitrim-topic-scan
description: 'Use when hunting for uncut YekiTrim clips about a topic on the running local server — 蹭热点/找素材/查某个话题有没有剪过. Searches every project''s subtitles, LLM-corrected subtitles and danmaku, maps hits back to clips, and separates "真的没有" from "有但没被检测 / 没被剪出来".'
---

# YekiTrim 话题扫描（蹭热点找素材）

**回答的问题**：「这批录播里有没有聊到 X 的？有没有还没剪的？」

**核心价值不是"搜到了"，而是把三种「没有」分开** —— 只说"没搜到"会让你得出错误结论：
实测第一轮手工搜「男友粉」得出"只有 1 条"，第二轮补上弹幕和缺口分析才发现
**有个项目 46 分钟的恋爱主线从没进过检测**。

| 判定 | 含义 | 怎么发现 |
|---|---|---|
| ❌ **真没有** | 字幕 + 订正稿 + 弹幕全文都搜不到 | 三源全文检索 |
| ⚠️ **有但漏检** | 落在**事件缺口**里 —— 检测压根没跑到那段 | 缺口分析（>60s 无事件覆盖） |
| ⚠️ **有但碎了** | 命中段**无归属**：跨事件边界，或被后处理「最短 30s 吞并」吃掉 | 命中项标注「无归属」 |

## 怎么跑

```bash
SK=.dsh/skills/yekitrim-topic-scan/scripts/scan.mjs

# ① 关键词模式：给了词就只用这些词，一个不加
node $SK --kw "维阿,virtuareal,vr"

# ② 发现模式：不给词 —— 题材套件 + 数据侧热度榜
node $SK --discover

# ③ 只要某个项目 / 要机器可读结果
node $SK --kw "恋爱" --project 5,10 --json
```

报告落在 `.workbuddy/topic-scan/scan-<时间戳>.md`（已 gitignore）。stdout 另有一份摘要。

| 开关 | 作用 |
|---|---|
| `--kw <a,b,c>` | 关键词，逗号分隔。**给了就只用这些** |
| `--discover` | 不给词模式（默认）：题材套件 + 热度榜 |
| `--project <ids>` | 只扫这些项目 id |
| `--top <n>` | 热度榜条数（默认 30） |
| `--out <dir>` | 报告目录 |
| `--json` | 额外落一份机器可读 JSON |
| `--server <url>` | 服务地址，默认 `http://127.0.0.1:26320` |
| `--quiet` | 只写文件 |

## 关键词从哪来（重要）

- **用户给了词 → 原样用，不扩不改**，`--kw` 就是唯一来源。
- **用户没给词 → 你自己定词**，且 **不联网**（用户明确要求）。两步：
  1. 用 `keywords.mjs` 的 `TOPIC_SUITE` 起手（情感/粉圈/VTuber/游戏抽象，四组）；
  2. **看报告第五节的「未剪切热度榜」** —— 那是弹幕+字幕密度算出来的**数据侧真热点**，
     照着榜单上的标题补关键词，比凭印象猜准得多。

> 用户说"蹭最热话题"时，**先跑 `--discover` 看热度榜**，再决定拿哪个话题去 `--kw` 精扫。
> 热度榜是免费的（不用联网），而且它反映的是**这批录播里本来就热的东西**。

## 加词只改一个文件

加/删关键词**只改 `scripts/keywords.mjs`**，不用动 `scan.mjs`。

⚠️ 两条纪律（实跑调出来的）：
1. **别放过于通用的词** —— `父母`/`害怕`/`一个人`/`弟弟` 这种在 12 个项目上命中 600+ 处，
   榜单直接被淹没。词要"说了这句就知道在聊这个话题"。
2. `TOPIC_SUITE` 只是**起手**，不是穷举；真正的发现靠热度榜。

## 判据（照抄界面，不许自己发明）

- **未剪切** = `clipMatchesEvent(clip, ev)` 判不出任何归属事件的事件 ——
  与 `webui/src/utils.js` **逐行一致**（`clip_dir` 优先比目录名；否则比 `clip-NN` 名 + 标题 + IoU≥0.5）。
  等价于"事件的 `clip_dir` 目录下没有 `clip.mp4`"，但**别用 `cut_exists` 凑合** —— 界面用的是前者。
- **命中归位按区间重叠**：`seg.end > ev.start_sec && seg.start < ev.end_sec`。
  ⚠️ 别用"起点落在区间内" —— 会把跨边界的句子判成无主。

## 数据源（三个都要搜）

| 源 | 位置 | 为什么必须要 |
|---|---|---|
| ASR 转写 | API `/api/project-data/transcript` | 主源 |
| **LLM 订正稿** | `<root_dir>/output/video_llm.srt` | **订正稿能救回原稿漏的词** —— 「男友粉」那句只有订正稿完整 |
| **弹幕** | `<root_dir>/*.xml` `*.ass`（项目根） | 粉丝造词（男友粉/梦女）几乎只出现在弹幕里；也是热度来源 |

`<root_dir>` 从 `/api/project-data/config?project=N` 拿。**读不到就如实写进报告的"备注"列**
（服务可能在别的机器上），不要静默当成"没有弹幕"。

## 踩坑清单（每条都是真撞过的）

- ⚠️ **`web_fetch` 工具拒绝 localhost**（`non-public IP`）—— 要用 HTTP 就**必须走脚本里的 `fetch`**
  或 shell 的 `curl`，别用 `web_fetch`。
- ⚠️ **沙箱下 shell 可能整体起不来**：msys 报
  `NtCreateDirectoryObject(\BaseNamedObjects\msys-2.0S5-…): 0xC0000022`，
  连 `echo hello` 都失败。需要 full access 才能跑。
- ⚠️ **本机没有 `jq`**，`python3` 是 Microsoft Store 占位符（跑起来不输出任何东西）——
  解析一律用 **node**。
- ⚠️ **`/api/project-data/clips` 返回的是裸 JSON 数组**，不是 `{ok, clips:[…]}`。
  当成对象解会静默得到 0 条 —— 然后你会以为"所有事件都没切"。
- ⚠️ **别用 shell 的 `grep` 搜中文**：Git bash 的 `.` 不匹配多字节字符，且 JSON 里有转义，
  结果全是错的。**用 node 读**。
- ⚠️ **别只看事件的 `label` / `summary`** —— 那是 LLM 写的摘要，会丢词。必须搜原始字幕。
- ⚠️ **ASR 有噪音**：同一个词常有谐音变体，报告里**必须贴原话**让人自己判断，不要只给计数。
- ⚠️ **只有部分项目有弹幕**（本项目 12 个里 5 个）：没有弹幕的项目热度只能靠字幕密度，
  **粉丝向内容会系统性排不高** —— 报告里要把这条点出来。
- ⚠️ **短 ASCII 词要词边界匹配**：`vr` 朴素子串会命中 `vrm`/`vrchat`。`keywords.mjs` 已对
  ≤3 字符的 ASCII 词自动加边界。

## 边界（硬规矩）

- **只读**：不写项目数据、不改 `events.json`、**不触发重新检测**。
  唯一会写的是报告目录。
- ⚠️ **绝不自动重新检测**：重新检测会**按热度整体重排 `rank`**，已有片段与事件的对应会错位
  （会被标成「独立」或「序号被占用」）。只能在报告里**建议**，由人工去预处理页操作。
- **不自动剪切**：报告给 workbench 链接，剪切由用户在界面点。
- 服务器不可达 / shell 起不来时，**明说卡在哪**，不要静默返回"没有"。
