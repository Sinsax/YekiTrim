# P4「B站链接通道」前置实测报告

> 实测日期 **2026-09-19**。环境：Windows / `yt-dlp.exe` **2026.08.19** / `ffmpeg` 在 PATH。
> 复现脚本与原始产物在 **`.workbuddy/p4-verify/`**（该目录已 gitignore，不进仓库）：
> `t1_probe.py` / `t2_danmaku.py` / `t3_download.py` + 抓下来的 XML / 下载的 mp4。
>
> 本报告只写**实测到的事实**。凡未实测的一律标「未验证」。
> 它推翻了 `p4-bilibili-downloader-selection.md`（下称「选型文档」）的 **3 条结论**，见 §0。

## 0. 结论速览

| # | 前置项 | 实测结果 | 对方案的影响 |
|---|---|---|---|
| T0 | 工具/网络 | `yt-dlp` **本机没有**；`ffmpeg/ffprobe` 有；GitHub 与 B站 直连/代理都可通 | 必须做 sidecar 分发 |
| T1 | 供应链可钉死 | release `2026.08.19` 的 `yt-dlp.exe`(17,840,399B) sha256 与官方 `SHA2-256SUMS` **一致** | ✅ 选型文档 §3.3 成立 |
| T2 | 版本探测参数 | `--version` OK / `-version` **exit 2 无输出** | ❌ **现有 `command_available_blocking` 会把 yt-dlp 判成不可用**（必须参数化） |
| T3 | `cid` 来源 | 普通 `-J` 里 **`cid` 出现 0 次**、`danmaku` 0 次、`subtitles={}` | ❌ **推翻**「用 `yt-dlp -J` 拿 cid」；改用 B站 view API |
| T4 | 弹幕 XML 可达性 | UA 非浏览器 → **HTTP 412**；响应**强制 raw deflate**；条数≈`maxlimit×1.2` | 需浏览器 UA + 手工解 raw deflate |
| T5 | `--write-subs` 落盘 | **落盘成功**（2583B 合法 XML） | ❌ **推翻**§3.2「静默不落盘」 |
| T6 | 下载契约 | `--print after_move:filepath` 回报绝对路径且命中；混流 OK | ✅ §4.3 成立，但发现 4 个新增约束 |
| T7 | 成功率 | 10 个样本：硬失败 1（legacy 视频）+ 需登录 1 | 需补 `ExtractorBroken` 错误类 |
| T8 | 风控频次 | 8 次连发 `comment.bilibili.com` 全 200，无 412 | §6.4 的阈值担忧不成立 |
| T9 | **解析器字段兼容** | 真实 XML 是 **9 段**、`[7]` 是 dmid → 现有代码 **全部 y=1500** | ❌ **推翻**「零改动」；必须改解析器 + 固件 |
| T10 | 解压依赖 | `reqwest` 只开 `json`（**无 gzip/brotli/deflate**）；`flate2 1.1.9` **已在依赖树** | 手工解压，不加 reqwest feature |

---

## T0 环境与网络

```
yt-dlp     MISSING          ← 本机没有，方案假设的 sidecar 需要自己分发
ffmpeg     C:\Users\para\AppData\Local\Microsoft\WinGet\Links\ffmpeg.EXE
ffprobe    同目录
node       22.22.2 (managed)
python     3.13.14
```

网络探测（直连 / 走代理 `127.0.0.1:14230` 都测了）：

| 目标 | 直连 | 代理 |
|---|---|---|
| `api.github.com` | 200 / 606ms | 200 / 322ms |
| `comment.bilibili.com` | 200 / 189ms | 200 / 220ms |
| `api.bilibili.com/.../view?bvid=` | 200 / 113ms | 200 / 114ms |

⚠️ **本机环境带 `HTTP_PROXY`/`HTTPS_PROXY`，子进程会继承**。实测中所有 yt-dlp 调用都**显式清掉**了这 4 个变量。
`vtrim server` 拉起 yt-dlp 时也必须这么做 —— 否则用户的代理会作用到 B站流量上（境外出口既慢又更容易触发风控）。

## T1 供应链可钉死（选型文档 §3.3 ✅ 成立）

`releases/latest` = **2026.08.19**（非 prerelease）：

| asset | 大小 |
|---|---|
| `yt-dlp.exe` | 17,840,399 B |
| `SHA2-256SUMS` | 1,595 B |
| `yt-dlp_linux` | 40,446,224 B |
| `yt-dlp_macos` | 37,146,048 B |

```
官方清单行  66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a  yt-dlp.exe
实算 sha256 66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a
判定        MATCH ✅          下载耗时 1.8s
```

> 与 `vtrim-core/src/download.rs::MODEL_REGISTRY` 是同一个套路，机制不必新发明。
> ⚠️ 但 `yt-dlp.exe` 是 PyInstaller 打的自带解释器的 PE，`cargo xtask scan` 那套 espeak 门禁**覆盖不到它的内部依赖**，
> 许可声明要单独写（yt-dlp 本体 Unlicense，随包 Python 运行时另计）。

## T2 版本探测（**现有代码缺陷，必须修**）

```
yt-dlp.exe --version   exit=0   out=['2026.08.19']
yt-dlp.exe -version    exit=2   无有效输出
```

`crates/vtrim-server/src/api/mod.rs:143`：

```rust
pub(crate) fn command_available_blocking(cmd: &str) -> Option<String> {
    let output = std::process::Command::new(cmd).arg("-version").output().ok()?;
```

**写死的 `-version` 用在 yt-dlp 上必然返回 None** → 界面显示「未安装」而实际装了。
选型文档 §4.5 已提到这条，本次实测确认（不是理论风险）。

## T3 `cid` 从哪来（**推翻选型文档 §3.2/§8.1**）

### 事实 1：普通 `-J` 里根本没有 `cid` / `danmaku`

对 `BV13x41117TL` 跑 `yt-dlp -J --skip-download`，全 JSON（24,333B）里：

```
'top-level keys'      → 无 cid
JSON 中 'cid' 出现次数        = 0
JSON 中 'danmaku' 出现次数    = 0
JSON 中 'comment.bilibili'    = 0
subtitles                     = {}
```

### 事实 2：原因是有门控，不是提取器没写

`yt_dlp/extractor/common.py`：

```python
def extract_subtitles(self, *args, **kwargs):
    if (self.get_param('writesubtitles', False)
            or self.get_param('listsubtitles')):
        return self._get_subtitles(*args, **kwargs)
    return {}
```

`bilibili.py` 里所有弹幕轨都是 `'subtitles': self.extract_subtitles(...)` 赋值的（367/886/897/1080/1232 行），
走的正是这个包装器 → **不加 `--write-subs` / `--list-subs` 就永远拿不到弹幕轨**。
选型文档引用 `_get_subtitles` 的那段代码没错，但**漏了调用链上的门控**。

### 事实 3：加上 `--write-subs` 就出来了，且多P 全都有

```
--list-subs            →  Language Formats / danmaku  xml
-J --write-subs        →  subtitles: {"danmaku":[{"ext":"xml",
                            "url":"https://comment.bilibili.com/14694589.xml"}]}
```

多P 视频 `BV1bK411W797`（23P）：**23/23 个 entry 都带 `danmaku` 轨**，每 P 各自一个 cid：

```
P1 id=BV1bK411W797_p1  subtitles=['danmaku']  formats=12
P2 id=BV1bK411W797_p2  subtitles=['danmaku']  formats=9
...
第一个: https://comment.bilibili.com/191557669.xml
```

⚠️ 但 **`cid` 在任何层级都不是一等字段**（顶层、entries 全是 `None`），只能从字幕 URL 里抠。

### 事实 4：更好的来源 —— B站 view API（裸 GET，无需 WBI、无需 cookie）

```
GET https://api.bilibili.com/x/web-interface/view?bvid=BV13x41117TL
  Header: 只要一个 UA，无 Referer、无 Cookie、无签名
→ code=0  data.cid=14694589  data.aid=8903802  data.pages[].cid（全部）
```

对比（同一视频，两条路交叉验证一致）：

| 来源 | 调用次数 | 拿到的 cid | 额外信息 |
|---|---|---|---|
| `yt-dlp -J --write-subs` | 1 次 yt-dlp 进程 + 解析 JSON | `14694589`（藏在 URL 里） | formats |
| **B站 view API** | **1 次 GET** | `14694589`（顶层）+ **23/23 pages** | `part` 名 / 单P 时长 / `stat.danmaku` / `stat.view` |

**结论：`cid` 改用 view API。** 一次拿到全部分P 的 cid、P 名、时长、弹幕数，
正好是 resolve 预览卡要显示的东西，还省掉一次 yt-dlp 进程。**且这条不需要 WBI 签名**（实测裸 GET 就通）。

## T4 弹幕 XML：UA、压缩、规模、截断

### 4.1 UA 是硬门槛

```
UA = python-urllib/3.13   →  HTTP/1.1 412 Precondition Failed
                             x-sec-request-id: c0b38961-...   （风控标识头）
UA = Chrome/131           →  HTTP/1.1 200 OK   Content-Type: text/xml
```

### 4.2 响应**强制压缩**，而且是**裸 deflate**

```
Accept-Encoding: （不传）    →  Content-Encoding: deflate
Accept-Encoding: identity   →  Content-Encoding: deflate   ← 被忽略！
Accept-Encoding: deflate,... →  Content-Encoding: deflate
Vary: Origin,Accept-Encoding
```

也就是说**无法用 `identity` 规避**，必须自己解压。而且风味是**裸 deflate**：

```
原始 145,077B   头 2 字节 = 9cfd
zlib.decompress(b)         ❌ Error -3: incorrect header
zlib.decompress(b, -15)    ✅ 解出 361,725B 合法 XML     ← 裸 deflate
zlib.decompress(b, 31)     ❌ Error -3
```

⚠️ 这**不符合 HTTP 规范**（`deflate` 按规范应是 zlib 包装）。踩坑表现是"抓到一串乱码、正则一条也匹配不上"——
如果不做解压校验，这会是一条**静默失败**。

### 4.3 规模远小于「几十 MB」的担忧（选型文档 §6.1 解除）

| cid | `<maxlimit>` | XML `<d>` 条数 | XML 字节 | 解压后 |
|---|---|---|---|---|
| `137649199` | 1000 | **1,200** | 123,003 | — |
| `122300483` | 3000 | **3,600** | 361,725 | 361,725 |
| `109658126` | 1000 | 4 | 684 | — |

### 4.4 但确实有上限：条数 ≈ `maxlimit × 1.2`

```
cid=137649199   B站自报 stat.danmaku=149,494   XML 只给 1,200  （0.8%）
cid=122300483   B站自报 stat.danmaku=  7,344   XML 只给 3,600  （49%）
```

比例 1200/1000 = 3600/3000 = **1.2**，两例一致 → 上限由该视频的 `maxlimit` 决定，不是随机截断。

**这条要正确理解**：`maxlimit` 是**弹幕池上限**，XML 返回的是**当前弹幕池**（即 B站播放器实际显示的那批），
更早的弹幕已被挤出池子。所以它不是"丢数据"，而是"只给池"。

⚠️ **但它对 `popularity` 有真实影响**：录播姬路径录的是**全量**直播弹幕，
B站链接通道只能拿到**池量级**（≤ maxlimit×1.2，典型 1000–3000 条）。
热门录播的实际弹幕量可能远超池上限 → 两条路径算出的热度**不同口径**。
（未验证：这是否会显著改变切片排序结果，需要在真实录播上比一次。）

## T5 `--write-subs --sub-langs danmaku` 落盘（**推翻选型文档 §3.2**）

```
exit=0
落盘: BV13x41117TL.danmaku.xml   2,583B
头部: <?xml version="1.0" encoding="UTF-8"?><i><chatserver>chat.bilibili.com</chatserver>
      <chatid>14694589</chatid><mission>0</mission><maxlimit>1000</maxlimit><state>0</state>
      <real_name>0</real_name><source>k-v</source><d p="1...
<d> 条数: 26   （与 stat.danmaku=26 一致）
```

**"静默不落盘"这条社区传闻没有复现**（至少在这个视频与这个版本上）。内容与自取的 XML 同构（同一接口）。

所以**自取 XML 的理由要重写**：不再是"下载器不可靠"，而是——
① 弹幕要在 resolve 阶段就能显示数量/预览；② 下载失败时弹幕仍可留档；③ 与下载解耦、可独立重试。

## T6 下载契约（选型文档 §4.3 基本成立，但有 4 个新增约束）

### 6.1 `--print after_move:filepath` ✅ 成立

```
stdout 回报：F:\para\Code\V-Trim\.workbuddy\p4-verify\dl\BV12N4y1M7rh.mp4
fs 命中    ：存在=True   大小=19,447,680B
耗时 6.8s，附产物封面 BV12N4y1M7rh.jpg (204,833B)
ffprobe    ：mov,mp4, duration=313.557  stream: hevc 640x360 / aac
```

**回报的是绝对路径**，可以直接写进 `vtrim.toml`，不需要拼路径。

### 6.2 ⚠️ 约束 1：默认会选到 **HEVC（H.265）**

上面那次拿到的是 `hevc 640x360`。HEVC-in-mp4 在浏览器里的支持面很窄，
**本项目的 WebUI 预览会播不了**。加 `-S vcodec:h264` 后：

```
--print "%(resolution)s|%(vcodec)s|%(acodec)s"
→  1920x1080|avc1.640032|mp4a.40.2     ✅ H.264
```

### 6.3 ⚠️ 约束 2：格式串不健壮会**直接 exit 1，不自动回退**

```
-f "b[height<=360]/b"
→ ERROR: [BiliBili] BV12N4y1M7rh: Requested format is not available...  exit=1
```

该视频没有 360p 的**合流**格式（只有分离流）。所以格式串必须始终以 `bv*+ba/b` 兜底：

```
-f "bv*[height<=N]+ba/b[height<=N]/bv*+ba/b"
```

### 6.4 ⚠️ 约束 3：非 TTY 下**默认没有进度输出**

一次性抓到 **0 行**进度。加 `--progress` 之后才有 —— 而且**出现在 stdout**（不是 stderr）：

```
stdout:
  dl:  0.0%| 553.48KiB/s|02:06|1024/NA
  ...
  dl:100.0%|  48.29MiB/s|00:00|71803629/NA
  F:\...\dl\prog2.mp4              ← --print 的输出，和进度混在同一流
```

→ 必须用**哨兵前缀**分流（`--progress-template "VTRIM_PROG:..."` 与 `--print "VTRIM_PATH:..."`），
按行前缀解析，别按"最后一行是路径"这种脆弱约定。

### 6.5 ⚠️ 约束 4：`--cookies` 只认 Netscape 格式

Netscape 格式（不是浏览器扩展默认的 JSON）。JSON 传进去**静默失效**，不报错。

## T7 成功率与错误分类

10 个样本（yt-dlp 官方 `_TESTS` 里的 B站 URL + 常见热门）：

```
成功 8 / 失败 2
```

| 现象 | 数量 | 判读 |
|---|---|---|
| `_type=video` 正常 | 6 | 常态 |
| `_type=playlist`（多P：23P / 4P） | 2 | **不是失败** —— 格式在 `entries[]` 里，需按 P 展开 |
| `Unable to extract initial state` | 1 | **真失败**（见下） |
| `This video is only available for registered users` | 1 | 需 cookie，属预期 |

### `Unable to extract initial state` 的根因（已查到上游）

yt-dlp issue **#14793**：legacy 老视频（av 号时代）**页面里没有 `__INITIAL_STATE__`**，
而提取器依赖它。维护者给的绕过方式是**重定向到 bvid 形式**。

对 V-Trim 的意义：**直播录像都是新视频，这类失败基本不会遇到**。
但要作为独立错误类映射出去（不是用户的错，重试也没用）。

### 实测到的 stderr 关键字（喂给 §错误分类表）

| 场景 | 实测输出 |
|---|---|
| 非法 URL | `ERROR: Unsupported URL: https://...` |
| 不存在的 BV | `ERROR: [BiliBili] 1zzzzzzzzzz: Unable to download webpage: HTTP Error 404: Not Found` |
| legacy 视频 | `ERROR: [BiliBili] 1wP4y1P72h: Unable to extract initial state; please report this issue` |
| 需登录 | `ERROR: [BiliBili] 1Ks411f7aQ: This video is only available for registered users` |
| 格式不可用 | `ERROR: [BiliBili] ...: Requested format is not available. Use --list-formats ...` |
| 非 B站域名 | `ERROR: [generic] Got HTTP Error 403 caused by Cloudflare anti-bot challenge` |

⚠️ 选型文档 §4.4 表里的 `Sign in to confirm` / `Premium` / `大会员` **本次一次都没出现**，
真实文案是 `only available for registered users`。该表需按实测改写。

## T8 风控频次（选型文档 §6.4 基本解除）

连续 8 次请求 `comment.bilibili.com/122300483.xml`（间隔 0.3s，浏览器 UA）：

```
#1 200 145,077B 232ms   #2 200 145,077B 231ms   #3 200 145,077B 209ms
#4 200 145,077B 150ms   #5 200 145,077B 148ms   #6 200 145,077B 154ms
#7 200 145,077B 244ms   #8 200 145,077B 3249ms
```

**全部 200，无 412**。所以「单并发 + 适度限速」的保守设定不是必需品，但也没必要放开
（真正会 412 的是**UA**，不是频次）。

## T9 解析器字段兼容（**新发现，必须修**）

### 真实 XML 的 `p` 是 **9 段**

```
p="0.60100,4,25,15138834,1604742895,0,9e5adeaa,40685126698926083,10"

[0] 时间     0.60100
[1] 模式     4
[2] 字号     25
[3] 颜色     15138834
[4] 时间戳   1604742895
[5] 弹幕池   0
[6] 用户hash 9e5adeaa
[7] 40685126698926083     ← 17-19 位的弹幕 dmid（不是行号！）
[8] 10                    ← 第 9 段（取值 4..10）

段数分布（3600/3600 条）: {9: 3600}
```

### 录播姬官方文档确认了差异

> 参数兼容 mikufans 主站弹幕 XML 文件，按顺序含义分别为：… **7. 发送者 UID（主站弹幕 XML 为发送用户 ID 的 CRC32）** …
> **8. 固定为 0（主站弹幕 XML 为弹幕的数据库 ID）**
> —— rec.danmuji.org/user/danmaku

即：**录播姬写 8 段、`[7]` 恒为 `0`**；主站写 `[7]` = 数据库 ID（dmid）。

### 后果：现有代码在主站 XML 上**全部 y=1500**

`crates/vtrim-core/src/danmaku/mod.rs:164-168`：

```rust
let row: f64 = parts.get(7).and_then(parse).filter(|v: &f64| *v > 0.0)
    .unwrap_or(default_y_for_mode(mode));
...
y: row.clamp(0.0, 1500.0),
```

- **录播姬 XML**：`[7]` = `0` → `.filter(>0.0)` 挡掉 → 走 `default_y_for_mode` ✅
- **主站 XML**：`[7]` = `2.2772981386706944e16` → 过滤通过 → `.clamp(0,1500)` → **1500.0**

实测：`全部条目 y 是否都是 1500.0 → True`（3600/3600）。

**也就是说 `parts[7] → y` 这条分支在现有录播姬路径上从未生效过，一到主站 XML 就会被激活并产生错误值。**

### 影响面（不要夸大）

| 消费方 | 是否受影响 |
|---|---|
| `popularity` 计算（选这条路的主要理由） | **不受影响**（只用 `start`/`text`） |
| 精修链路 `assign_danmaku_tracks` | **被掩盖** —— 该函数只写不读，直接覆盖 `entry.y = TRACK_YS[track]` |
| `process/placement.rs` 弹幕带 `y = (d.y/1920)*540` | **受影响**（若在轨道分配之前读到 y，弹幕带会退化成一条线） |

→ 严重度取决于调用顺序，**未完全验证**；但修它成本极低（见计划 §3.5）。

### 项目固件本身也是错的

`crates/vtrim-core/src/danmaku/mod.rs:414` 的测试固件注释写着「录播姬写出的 B站标准弹幕 XML（官方样例结构）」，
但内容是**手编的 8 段、第 8 段是小行号**：

```
<d p="1.5,1,25,16777215,1700000000,0,abc,120">第一条 &lt;滚动&gt; 弹幕</d>
<d p="12.25,4,25,16711935,1700000010,0,abc,1000">底端弹幕</d>
```

**它既不是主站格式，也不是录播姬格式（录播姬那段恒为 0，不会是 120/1000）。**
所以这个测试**从来没验证过真实数据**。→ 计划里要换成两套真实固件（主站 9 段 + 录播姬 8 段）。

## T10 解压依赖

```toml
# 三个 crate 都是这样
reqwest = { version = "0.12", features = ["json"] }     # 没有 gzip / brotli / deflate
```

`Cargo.lock`：`reqwest 0.12.28`、**`flate2 1.1.9` 已在依赖树里**（经由 `png` / `ureq` / `zip`）、
`async-compression` 与 `brotli` **不在**。

→ 给 reqwest 加 `deflate` feature 有风险（它按规范走 zlib 包装，而 B站发的是裸 deflate），
**更稳的做法是自己用 `flate2::read::DeflateDecoder`（裸）解，并把 zlib 包装作为兜底**。
`flate2` 已存在，加为直接依赖**不引入任何新传递依赖**。

---

## 附：被本次实测推翻/修正的原结论

| 选型文档位置 | 原来写的 | 实测后 |
|---|---|---|
| §3.2 / §8.1 | 用 `yt-dlp -J` 拿 `cid` | 普通 `-J` **没有 cid**；改用 B站 view API（裸 GET 免签名） |
| §3.2 / §6.3 | `--sub-langs danmaku` 静默不落盘 | **落盘正常**；自取的理由改为职责分离 |
| §3.1 / §6.2 | 与录播姬 XML「同源同构、零改动」 | **只对了一半**：正则能命中、时间/模式/颜色一致；但 `p` 是 9 段、`[7]` 语义不同 → y 全是 1500 |
| §4.4 | 错误关键字 `Sign in to confirm` / `Premium` | 实测文案是 `only available for registered users`；需补 `Unable to extract initial state` |
| §6.1 | 担心「几十 MB XML」 | **不成立**：上限 ≈ maxlimit×1.2，实测最大 361KB |
| §6.4 | 担心 412 频次阈值 | 频次不敏感；**UA 才是硬门槛** |
| §4.3 | 只差 `--print after_move:filepath` | 另需：`-S vcodec:h264`、格式串兜底、`--progress`+哨兵前缀、cookie 须 Netscape |

## 仍未验证（不当作结论）

1. **池量级弹幕对 `popularity` 的实际影响**：录播姬（全量）vs B站（池）两条路径的热度排序差异有多大。
2. **`assign_danmaku_tracks` 与 `placement` 的调用顺序**：决定 y=1500 是"真影响弹幕带"还是"被完全掩盖"。
3. **长录播的大文件下载稳定性**：本次只下了 5 分钟 / 19MB；数小时 GB 级未测。
4. **cookie 的实际解锁效果**：本机无 B站账号 cookie，未测 1080P+/大会员内容。
5. **`Unable to extract initial state` 在"新视频"上的复现率**：本机样本偏旧，需用近期录播再抽样。
6. **第 9 段（`[8]`，取值 4–10）的语义**：只观察到分布，未确定含义（不影响本方案）。
