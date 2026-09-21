# P4「B站链接通道」下载器选型

> 核实日期：**2026-09-19**。所有仓库数据来自 GitHub REST API 实测（`/repos/{owner}/{repo}`、`/releases`），
> 不是二手博客。反爬/接口变动频繁，本表**需要按季度重新核一遍**。
>
> 结论：**默认通道改为 `yt-dlp`**，推翻设计稿 §4.5 里 `downloader = "bbdown"|"yt-dlp"` 的隐含默认。

## 0. 一句话结论

| 项 | 决定 |
|---|---|
| 默认下载器 | **yt-dlp**（sidecar 单文件 exe，sha256 钉死） |
| 弹幕（本项目最关键） | **不由下载器产出**，V-Trim 自己按 `cid` 拉 `comment.bilibili.com/{cid}.xml` |
| BBDown | 降级为 `downloader = "<绝对路径>"` 的可选逃生舱，**不随 release 分发** |
| 元数据 / resolve 卡 | 自研，数据源 = `yt-dlp -J`（不手写 WBI 签名） |

## 1. 为什么先否掉设计稿的默认项（BBDown）

`nilaoda/BBDown` 的 API 返回：

```
archived:        true          ← 仓库已归档
default_branch:  "DELETED"     ← 默认分支已被删除
has_issues:      false         ← issue 区已关闭
pushed_at:       2026-05-14
最新 release:     1.6.3  (published_at 2024-08-14)
```

也就是说：**最后一个正式发布是 2024-08-14，距今约 2 年**。B站侧这两年一直在加 WBI 签名与风控，
一个两年没有发布、issue 区都关掉的下载器，作为"平台核心通道"等于上线即待修。

同类现象不是个例 —— 整条 B站专用下载器赛道在 2026 年大面积停更：

| 项目 | stars | 归档 | 最后 push | 说明 |
|---|---|---|---|---|
| `btjawa/BiliTools` | 5,112 | **✅** | 2026-07-07 | 仓库描述直接写「本项目已停止维护。」 |

结论：**不要把平台通道押在单一"B站专用"项目上**，要么选活跃的通用型，要么自己留一层适配。

## 2. 候选对比（2026-09-19 实测）

| 项目 | 语言 | stars | 归档 | 最后 push | 许可 | B站关键能力 | 判 |
|---|---|---|---|---|---|---|---|
| `yt-dlp/yt-dlp` | Python | **192,024** | ❌ | **2026-09-16** | **Unlicense** | 弹幕(字幕轨 `danmaku`)、分P、cookie、WBI 已实现 | ✅ **默认** |
| `nilaoda/BBDown` | C# | 13,881 | **✅** | 2026-05-14 | MIT | 8K/HDR/杜比、TV/APP 无水印源、弹幕 ASS | ⚠️ 逃生舱 |
| `ScottSloan/Bili23-Downloader` | Python | 7,522 | ❌ | **2026-09-18** | **GPL-3.0** | 分P、弹幕元数据、多线程 | ⚠️ 备选 |
| `iawia002/lux` | Go | 31,688 | ❌ | 2026-03-29 | MIT | **无弹幕**、无大会员质量 | ❌ |
| `leiurayer/downkyi` | C# | 24,411 | ❌ | 2026-07-06 | GPL-3.0 | `default_branch="Deprecated"`，WPF GUI | ❌ |
| `HFrost0/bilix` | Python | 1,784 | ❌ | **2024-09-20** | Apache-2.0 | 快，但停更 2 年 | ❌ |
| `sailist/BBDown-go` | Go | 43 | ❌ | 2026-05-08 | MIT | 有 HTTP serve + JSON 日志，但无社区验证 | ❌ |

淘汰理由简述：

- **lux**（31.7k stars，看着最体面）：B站支持是本项目最弱的一环 —— **拿不到弹幕**，也拿不到大会员清晰度。
  stars 高不代表能用，它是"多站点通用"的宽度路线。
- **downkyi / Bili23-Downloader**：都是 **GUI 优先**（WPF / PySide6）。V-Trim 是 `vtrim server` 无人值守场景，
  需要的是稳定 CLI 契约与可解析错误，不是窗口。Bili23 是这条赛道里唯一还在日更的（2026-09-18），
  留作"某个视频 yt-dlp 失败"时的第二引擎候选，但 GPL-3.0 的分发义务比 Unlicense 麻烦。
- **bilix**：2024-09 之后零推送，B站风控一变即失效。
- **BBDown-go**：43 stars、作者自己写的"部分高级功能未完全接入"，没有足够用户量帮我们发现坏掉的地方。

## 3. 两个决定性发现

> ⚠️ **本节 2026-09-19 已被实测修正，先读 `p4-prerequisite-verification.md`。**
> 修正摘要（详情见该报告 §0 与附表）：
> - **3.2 的「用 `yt-dlp -J` 拿 `cid`」不成立** —— 普通 `-J` 里 `cid` 出现 **0 次**、`subtitles` 为 `{}`，
>   因为 `extract_subtitles` 有 `writesubtitles || listsubtitles` 门控。→ 改用 B站 **view API**（裸 GET、免 WBI）。
> - **3.2 的「`--sub-langs danmaku` 静默不落盘」未复现** —— 实测落盘正常（2583B 合法 XML）。
> - **3.1 的「同源同构、零改动」只对一半** —— 正则能命中、时间/模式/颜色一致，
>   但真实 XML 是 **9 段**且 `[7]` 是**弹幕 dmid**（非行号），而录播姬该段**恒为 0**；
>   现有 `parts[7] → y` 会把全部弹幕 **clamp 到 y=1500**（实测 3600/3600）。**解析器必须改。**
> - 另外实测解除了「几十 MB XML」与「412 频次」两个担忧，但新增 4 条下载命令约束。

### 3.1 弹幕同构 —— 现有解析器可直接复用，零新代码

读 `yt-dlp` 的 `yt_dlp/extractor/bilibili.py`，`BilibiliBaseIE._get_subtitles` 里：

```python
subtitles = {
    'danmaku': [{
        'ext': 'xml',
        'url': f'https://comment.bilibili.com/{cid}.xml',
    }],
}
```

即 **yt-dlp 把弹幕当成一个字幕轨道**，语言键 `danmaku`，扩展名 `xml`，直连 B站官方 XML 接口。

再看本项目 `crates/vtrim-core/src/danmaku/mod.rs`（`parse_bililive_xml` 的文档注释）：

```
/// 结构：`<i><d p="时间,模式,字号,颜色,时间戳,弹幕池,用户,行号">文本</d>…</i>`
```

**两边是同一套格式** —— `comment.bilibili.com/{cid}.xml` 与录播姬写出的 XML 同源。
`load_danmaku_auto` 的判定 `content.contains("<d p=")` 会直接命中，`popularity` 计算不用改一行。

> 这条同时否掉了设计稿 §4.5 的「弹幕优先让下载器直接出 ASS」：
> ASS 要额外写解析器，而且**丢掉 mode / color / 池 的语义**（BBDown 走的就是 ASS 路线）。
> 拿 XML 反而更好。

### 3.2 有实测报告的坑：不要依赖 yt-dlp 帮我们把弹幕落盘

社区实测反馈：`--sub-langs danmaku --write-subs` 在**部分视频上日志显示匹配了 danmaku 却不写文件**
（字幕轨存在但下载器静默跳过）。所以别把弹幕绑在 yt-dlp 的写文件行为上：

**推荐做法**：用 `yt-dlp -J` 拿到 `cid`，然后 V-Trim 用已有的 `reqwest` 自己 GET
`https://comment.bilibili.com/{cid}.xml` 落成 `<项目>/danmaku.xml`。

好处：确定性高（纯 GET、无签名、无风控头）、不依赖 yt-dlp 的字幕写出、不给进程加 GPL 插件依赖
（`yt-dlp-danmaku` 那类插件是 MIT 但会让部署面变复杂）。

### 3.3 供应链可钉死 —— 复用已有的"预编译归档"做法

`yt-dlp` 官方 release（当前 `2026.08.19`）的 assets 同时提供：

```
yt-dlp.exe          ← Windows 单文件，内置 Python，无需运行时
SHA2-256SUMS        ← 官方哈希清单，可直接钉 sha256
yt-dlp_linux / yt-dlp_macos / yt-dlp_arm64.exe …
```

与 `vtrim-core/src/download.rs` 的 `MODEL_REGISTRY`（模型归档 + marker 判定）是同一个套路，
`release/*.zip.sha256.txt` 也已经有了 —— **照抄即可，不必新发明机制**。

**不要**从第三方镜像站（gitcode / 各类"软件站"）取 BBDown 之类的二进制：主仓已归档，
镜像出的 exe 没有任何可信来源。

## 4. 推荐方案

### 4.1 通道分工

```
B站 URL
  │
  ├─ resolve 卡（标题/封面/UP主/时长/分P/预计占用）
  │     └── yt-dlp -J --skip-download   ← 元数据齐全，无需自写 WBI 签名
  │
  ├─ 视频流
  │     └── yt-dlp --cookies --limit-rate -f ... --merge-output-format mp4
  │
  └─ 弹幕  ← 不经过下载器
        └── GET https://comment.bilibili.com/{cid}.xml
              └── 写入 <项目>/danmaku.xml + vtrim.toml 的 danmaku_file
                    └── parse_bililive_xml（现有，零改动）
```

### 4.2 配置（沿用设计稿 §4.5 的键名，只改默认值）

```toml
[ingest]
downloader = "yt-dlp"        # 改：原设计稿隐含默认 bbdown；也支持 "bbdown" | "<绝对路径>"
max_quality = "1080p"
rate_limit_kbps = 0          # 0 = 不限；>0 映射到 --limit-rate <n>K
max_concurrent_downloads = 1 # 已有「下载槽」，勿放大：B站 412 风控与 IP 频次相关
cookie_file = ""             # 仅 admin 可写；无 cookie 时清晰度封顶 480/720p
```

`cookie_file` 是刚需而非可选项：B站未登录只给低清流。界面上对非 admin 隐藏该项（沿用 §5.3）。

### 4.3 yt-dlp 命令行契约

| 目的 | 参数 |
|---|---|
| 元数据 / resolve | `-J --skip-download --no-warnings`（多分P 会返回 playlist entries，每条含 `cid`） |
| 只要第 N P | `--playlist-items N`（配 `-o` 固定模板） |
| 最高清晰度 | `-f "bv*[height<=<maxq>]+ba/b"` |
| 混流 | `--merge-output-format mp4` |
| 限速 | `--limit-rate <k>K` |
| 封面 | `--write-thumbnail --convert-thumbnails jpg` |
| 取最终路径 | **`--print after_move:filepath`** |

⚠️ **`--print after_move:filepath` 请务必用**：不要靠文件名模板去猜产物路径。
B站标题里的 `/ \ : * ? " < >` 会被 yt-dlp 消毒，中文与 emoji 也可能变样，
猜路径就是"下载成功但项目指向一个不存在的文件"。让 yt-dlp 自己回报最终路径最稳。
（本项目已有同类教训：不让前端拼路径，见 AGENTS.md 的 `{dir, name}` 那条。）

### 4.4 错误分类映射（设计稿要求的 6 类）

| 分类 | 判据（yt-dlp stderr 关键字） |
|---|---|
| `UnsupportedUrl` | `Unsupported URL` |
| `NeedLogin` | `Sign in to confirm` / `only available for registered users` |
| `MemberOnly` | `Premium` / `大会员` / `This video is only available to VIP` |
| `RateLimited` | `HTTP Error 412` / `HTTP Error 429`（412 = B站风控，最常见） |
| `ToolMissing` | 进程 spawn 失败（`NotFound`）/ `Access is denied` |
| `DiskFull` | `No space left on device` / `os error 112` |

错误分类要**落库**（`ingest_sessions.state=failed` + 原因），文案直接给用户 —— 沿用 §3.3 的做法。

### 4.5 工具发现与安装

照 `vtrim-server/src/api/mod.rs` 里的 `chrome_available_blocking` 现成模式：
配置路径优先 → 候选列表探测 → 30s 缓存。

- 探测命令：`yt-dlp --version`（注意：**不是** `-version`，`command_available_blocking` 现在写死了 `-version`，通用化时要参数化）
- 安装位置：`<程序目录>/.vtrim/bin/yt-dlp.exe`
- 版本 + sha256 写死在注册表里（照 `MODEL_REGISTRY`），`vtrim doctor` 的 `downloader` 就绪位顺带报出来
- `/api/health` 只回就绪位，不回路径（§5.3 已定）

### 4.6 许可与合规

- `yt-dlp` = **Unlicense**（公有领域）：随 release 打包**无附加义务**，比 GPL-3.0 省事得多。
- BBDown 若用户自行放置：不随包分发，仅作 `<绝对路径>` 逃生舱，不在 notices 里声明为分发包一部分。

⚠️ **这一条与 2026-09-19 的「0.2.0 二进制分发许可合规复核」直接冲突，必须一起看**：
那次复核的结论是「发行物里 **0 个 GPL/LGPL/AGPL**，只有 Apache-2.0 的署名义务」，
所以 v0.2.0 二进制**不背**开源传染义务。

如果这里把 `Bili23-Downloader`（GPL-3.0）或 `downkyi`（GPL-3.0）**打进 release** 做 sidecar，
那份复核结论立刻失效 —— 分发侧要新增「附源码或书面要约」的义务，
`cargo xtask scan` 那套门禁也只覆盖 Rust 闭包与 exe，管不到外挂的 py/exe。

**所以默认通道选 Unlicense 的 yt-dlp 不是"顺手"，而是维持既有合规姿态的必要条件。**
真要引入 GPL 引擎，走 `<绝对路径>`（用户自备、不随包分发），别进 `release/`。

## 5. 需要改的既有文档/代码（不能只加不改）

| 位置 | 现在写的 | 要改成 |
|---|---|---|
| `docs/superpowers/specs/2026-09-12-multi-user-server-design.md` §4.5 | `downloader = "bbdown"\|"yt-dlp"`（顺序暗示 bbdown 优先） | 默认 `yt-dlp`，并注明 BBDown 已归档 |
| `AGENTS.md:523` | 「B 站链接：P4 未实现，如实标注不可用」 | 落地后改为实际契约 |
| `AGENTS.md:663-665` | `bilibili` → 预载字幕，「该来源仍是 P4 未实现，这个分支目前没有调用方」 | 补调用方：弹幕预载 `danmaku.xml` + 字幕 |
| `ROADMAP.md:9` | 只剩 P4 / P5 待做 | 更新 P4 状态 |

⚠️ 注意 AGENTS.md:663 说 bilibili 来源"预载字幕"——但按本方案，**弹幕才是主要产物**（字幕很多 UP 主根本没传）。
预载策略应改成「弹幕优先，字幕有就带」，否则 B站项目会稳定丢掉 popularity 排序。

## 6. 待实测验证（落地前必须做掉，别默认成立）

> ✅ **本节的 6 项已于 2026-09-19 实测完毕，见 `p4-prerequisite-verification.md`。逐项状态：**
>
> | # | 项 | 状态 |
> |---|---|---|
> | 1 | 弹幕 XML 规模 / 条数上限 | ✅ 有上限且已量化：条数 ≈ `maxlimit×1.2`；实测最大 **361KB**，「几十 MB」担忧不成立 |
> | 2 | `p` 属性 8 段是否一致 | ❌ **不一致** —— 主站是 **9 段**、`[7]` 是 dmid；录播姬 8 段、`[7]` 恒为 0 → 解析器必须改 |
> | 3 | `--sub-langs danmaku` 不落盘 | ❌ **未复现**（落盘正常）；自取 XML 的理由需重写 |
> | 4 | 412 风控阈值 | ✅ 8 次连发全 200；**UA 才是硬门槛**（非浏览器 UA 直接 412） |
> | 5 | 分P 每 P 一项目 vs P5 容量 | ⏳ 仍成立，属设计决策 → 已在落地计划里定为「默认不勾全选」 |
> | 6 | 无水印源 | ⏳ **未验证**；另发现默认会选到 HEVC，需 `-S vcodec:h264` |

1. **弹幕 XML 规模**：3 小时录播的 `comment.bilibili.com/{cid}.xml` 有多大？是否全量返回、有无条数上限？
   本项目已有"几十 MB XML"的前车之鉴，读取要确认内存策略。
2. **`p` 属性 8 段是否完全一致**：抽一个真实 B站 XML 与录播姬 XML 逐字段对比（尤其"行号"段语义）。
3. **`--sub-langs danmaku` 不落盘的复现率**：确认 §3.2 的坑是否普遍 —— 这是我们自己拉 XML 的理由，
   如果复现不了，也只是少一层保险。
4. **412 风控触发阈值**：单并发 + 适度限速是否够；长录播（>2h）分片下载会不会中途被掐。
5. **分P 每个 P 一个项目**：这是设计稿定的，但会成倍放大下载量与磁盘占用，
   与 P5（容量）冲突，落地前确认是否要给用户"选 P 下载"而非"全 P 展开"。
6. **无水印源**：yt-dlp 没有 BBDown 的 `-tv/-app` TV/APP 接口，拿到的是带水印片源。
   切片产物是否受影响需要人工看一眼（水印通常在角标，可能被裁切吃掉）。

## 7. 落地清单（按依赖顺序）

1. `vtrim-service/src/downloader/`（新）：`DownloaderKind` + `resolve()` / `download()` trait
2. `yt-dlp` 注册表 + sha256 钉死 + `ensure_ytdlp()`（照 `download.rs::ensure_one_model`）
3. 弹幕抓取：`cid` → `comment.bilibili.com/{cid}.xml` → `<项目>/danmaku.xml`
4. `POST /api/ingest/resolve`（薄壳）+ 错误分类枚举
5. `POST /api/projects {source:{kind:"bilibili", ...}}` + 下载槽接线
6. WebUI 新建向导「B站链接」分支（§4 的 M4）
7. 文档同步（本文件 §5 那张表）

## 8. 运行时流程（yt-dlp 通道）

### 8.1 两条通道，不是一个命令

```
用户粘贴 B站 URL
   │
   ├──── 视频轨道（yt-dlp）──────────────────────────────┐
   │  ① resolve   yt-dlp -J --skip-download → 预览卡      │
   │  ② 落库排队  配额预检 + 下载槽(=1)                    │
   │  ③ 下载      yt-dlp -f … --cookies --print …         │
   │                                                      ▼
   └──── 弹幕轨道（自研 GET）──────────────────────→ 建项目 + 自动管线
       ④ cid → comment.bilibili.com/{cid}.xml → danmaku.xml
       ⑤ parse_bililive_xml（现有，零改动）
```

**只有 ① 和 ③ 需要 shell 出 yt-dlp**，其余全是项目已有的骨架。

### 8.2 逐阶段落点

| # | 阶段 | 动作 | 落点（复用 / 新增） |
|---|---|---|---|
| 0 | 工具发现 | 配置路径 → `<程序目录>/.vtrim/bin/yt-dlp.exe` → PATH，30s 缓存 | **复用** `api::probe_cache` / `probe_mode`；⚠️ `command_available_blocking` 写死了 `-version`，yt-dlp 要 `--version`，**必须参数化** |
| 1 | resolve | `yt-dlp -J --skip-download --no-warnings <url>` → 标题/UP主/时长/封面/分P `entries[].cid` | **新增** `POST /api/ingest/resolve`（薄壳，无状态、不落库） |
| 2 | 排队 | 建会话（`state=downloading`）+ 配额/水位预检 | **复用** `ingest_worker::check_capacity`；**新增** 会话的 `kind` 判别（见 §8.4） |
| 3 | 下载 | `yt-dlp -f "bv*[height<=N]+ba/b" --cookies --merge-output-format mp4 --limit-rate --print after_move:filepath -o <项目>/source.%(ext)s` | **新增** `vtrim-service/src/downloader/`；目录用 **复用** `projects::new_project_dir` |
| 4 | 弹幕 | `cid` → GET `comment.bilibili.com/{cid}.xml` → `<项目>/danmaku.xml` | **新增**（纯 reqwest，十来行） |
| 5 | 建项目 | 写 `vtrim.toml` + 登记项目 + 计量 + 起管线 | **复用** `write_project_config` / `db.create_project` / `projects::refresh_bytes` / `ingest_worker::spawn_pipeline` |

进度上报：读 yt-dlp **stderr** 的 `[download] 12.3% of …`，转发到已有的 `/api/stream`（SSE）。

### 8.3 与录播姬入库流程的差异（哪些能照搬、哪些不能）

| 环节 | 录播姬（已实现） | B站 yt-dlp | 影响 |
|---|---|---|---|
| 触发者 | 外部 webhook，**无身份** | 登录用户在 WebUI 主动提交 | 下载通道**不需要**"待分配池 + admin 指派"那一整套 |
| 归属 | `unassigned` → admin 指派 | 直接归当前用户 | 省掉 `assign` 状态迁移 |
| 完结判定 | 等 `SessionEnded` + 文件大小连续两轮不变 | **进程退出即完结** | 确定性高一个档，`is_finalized` 那套时间窗口完全不需要 |
| 文件来源 | 从录播姬目录**搬入**（rename/copy + 校验） | 下载器**直接写进**托管目录 | 省掉 `move_file` / `concat_fragments`（yt-dlp 自己混流多分片） |
| 进度 | 无 | stderr 百分比 → SSE | 更好的 UX，也让"卡住"可判 |
| 弹幕 | 同目录同名文件（`find_danmaku`） | 按 `cid` 单独 GET | 见 §8.1 弹幕轨道 |

**结论：复用面比看起来大得多** —— 状态机反而更简单（没有"等录制结束"这种不确定性）。

### 8.4 落地前必须先定的三个决策

1. **`ingest_sessions` 复用还是新建表？**
   现有 schema（`db.rs:319`）**没有来源判别列**，把 bvid 塞进 `room_id` 是 hack（会让 `channel_map` 的按房间号查规则误命中）。
   建议：加一列 `kind TEXT NOT NULL DEFAULT 'recorder'`（`recorder` | `bilibili`），复用整张表 ——
   重试（`/api/ingest/retry`）、记录页（`/api/ingest/records`）、容量预检、SSE 全挂在它上面，新建表等于把这些再抄一遍。
2. **`assets::preload_policy("bilibili")` 现在是 `Subtitle`，要改成 `Danmaku`。**
   见 `crates/vtrim-service/src/assets.rs:386` 与本文档 §5 的警告：不改的话 B站项目会稳定丢掉 popularity 排序。
3. **分P 展开策略**（本文档 §6.5）：设计稿定"每个 P 一个项目"，会成倍放大磁盘占用，与 P5（容量）冲突。
   倾向改为"resolve 预览里让用户勾选要哪几 P"，而不是无条件全展开。

### 8.5 三个不是决策、是硬性约束的点

- **`--print after_move:filepath` 必须用**（§4.3）：不要猜产物路径，B站标题里的 `/ \ : * ? " < >` 会被消毒。
- **下载槽固定 1**：B站 412 风控与 IP 频次相关，别放大并发（§4.2 已定）。
- **`cookie_file` 是刚需**：无 cookie 时清晰度被封顶到 480/720p，且部分视频直接 `NeedLogin`。
