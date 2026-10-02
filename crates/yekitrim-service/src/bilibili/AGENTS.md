# bilibili — B站专属知识（bvid / view API / 弹幕落盘，**不认识下载器**）

> 范围：`crates/yekitrim-service/src/bilibili`/** ｜ 上游：`yekitrim-server::download_worker`（装配）｜ 验证：`cargo test -p yekitrim-service bilibili`

## 职责

B站链接通道（P4）的**平台侧**：从用户粘的任意形态里抠出 `bvid`、用 view API 拉元数据（含全部分P 的 `cid`）、
把弹幕 XML 落到项目根。引擎与 B站**只在一处装配**（`yekitrim-server/download_worker.rs`），本目录**不知道 yt-dlp 的存在**，
也不开子进程。落地依据：`docs/plan/p4-implementation-plan.md` §2。

**为什么元数据走 view API 而不是 `yt-dlp -J`**（实测 `p4-prerequisite-verification.md` T3）：普通 `yt-dlp -J` 里
**`cid` 出现 0 次**、`subtitles` 是 `{}`（`extract_subtitles` 有 `writesubtitles || listsubtitles` 门控）。
而 B站的 view API **裸 GET 就能用**（不需要 WBI 签名、不需要 cookie、不需要 Referer），一次调用给出全部分P 的
`cid` + P 名 + 时长 + 弹幕总数 + 封面 —— 分P 视频也不需要逐 P 再请求，比 `-J` 快一个数量级，而 resolve 卡是
用户每次换链接都会点的。

## 入口

- `mod.rs`：`BiliError`（`BadInput` / `NotFound` / `Forbidden` / `NotAvailable` / `RateLimited` / `Api` /
  `Transport` / `Malformed`）+ `user_message` / `kind_str` / `user_facing`；`extract_bvid(input)`；
  `session_ref(bvid, page)` / `parse_session_ref(raw)`；并 re-export `fetch_view` / `parse_view` / `BvInfo` /
  `BvPage` / `fetch_danmaku` / `DanmakuFetch`。
- `api.rs`：`VIEW_API`（`https://api.bilibili.com/x/web-interface/view`）；`BvInfo { bvid, aid, title, uploader,
  duration_secs, cover, pages, danmaku_total, view_total }` + `is_multi_page()` / `total_duration_of(indices)`；
  `BvPage { index, cid, part, duration_secs }`；`parse_view(raw)`（**离线可测**）/ `fetch_view(bvid)`。
- `danmaku.rs`：`DANMAKU_API`（`https://comment.bilibili.com`）/ `DANMAKU_FILE`（`danmaku.xml`）/
  `EMPTY_XML`；`DanmakuFetch { xml, entries, declared_encoding }` + `is_empty()`；
  `parse_danmaku_response(cid, body, declared_encoding)`（**离线可测**）/ `fetch_danmaku(cid)`；
  `save_danmaku(project_dir, fetch)`（返回可直接塞进 `write_project_config` 的相对文件名）/ `danmaku_path`。

## 边界

- ⚠️ **本目录不许出现 `yt-dlp` / `VTRIM_PROG` / `VTRIM_PATH` / `VTRIM_META` / `process::Command` / `.spawn(`**
  （门禁 `bilibili 不许知道下载器`）：弹幕走旁路（自己 GET），与引擎完全无关；混在一起就没法单测。
  改了本目录就顺手跑 `python3 scripts/check_module_boundaries.py`。
- ⚠️ **B站侧与下载器的错误类刻意分开**（`BiliError` vs `DownloadErrorKind` 那一套）：这两条通道是独立的
  （view API / 弹幕 vs 视频流），失败原因也不同（一个可能是"稿件审核中"，另一个可能是"格式不可用"）。
  混成一个枚举会逼着两边都处理对方的分支。
- ⚠️ **`extract_bvid` 只把 `BV` 前缀规整成大写，后 10 位原样保留** —— bvid 的后半段是**大小写敏感**的
  （B站用了一套自造的 base58 变体，`BV1GJ411x7h7` 与 `BV1GJ411X7H7` 是两个不同的视频）。
  早期版本整串 `to_ascii_uppercase()`，会让所有小写字母的视频**稳定查不到**（表现为"视频不存在"，
  很像是 B站的问题，极难归因）。
- ⚠️ **不支持也不猜番剧**（`/bangumi/play/ep123`）与课程（`/cheese/`）：它们没有 bvid，而且本项目没有对应的
  信息架构（见落地计划 §1 的"不做"清单）。认不出来就明确报错，**不要**当成"某个视频"去试。
- ⚠️ bvid 固定 12 字符且**大小写敏感**，所以 `session_ref` 的分隔符只能用不在 bvid 字符集里的字符：
  `/` 是安全的（`as_bvid` 只接受字母数字）。**别再换回 `-` 或 `_`** —— 那些字符看起来更"路径安全"，
  但一旦将来 bvid 字符集变化就会产生歧义。
- `session_ref(bvid, page)`：第 1 P **不写后缀**（单P 与"多P 的第一 P"用同一种形态，resolve 预览里也看不出差别），
  多P 形如 `BV1xx/p3`。`parse_session_ref` 认不出来返回 `None`（而不是给一个"大概对"的默认值）：
  调用方据此把会话标 failed，否则会拿一个错视频去下载。`p0` 视为非法。
- 单P 也有一条 `BvInfo.pages`（B站的 `pages` 对单P 也返回一项），所以**不必为单P/多P 写两套分支** ——
  `pages.len() == 1` 就是单P。`total_duration_of` 在有时长缺失时回 `None`（**别硬凑一个数出来**）。
  `parse_view` 里标题为空 → `Malformed`（不是"标题为空的项目"）。

### 弹幕（`danmaku.rs`，两个实测出来的坑）

- ⚠️ **UA 是硬门槛**：非浏览器 UA（`python-urllib`、`reqwest/x.y`）→ **HTTP 412**，响应头带 `x-sec-request-id`。
  所以走 `crate::net::browser_client()`（见 `net/AGENTS.md`）；`Referer` 不是必须的，但带上更接近真实浏览器
  （实测带上更稳）。
- ⚠️ **响应恒为裸 deflate，且 `Accept-Encoding: identity` 被服务端忽略** → 必须手工解压
  （`crate::net::deflate::inflate`）。不解压会拿到乱码，正则一条都匹配不上，**静默产出 0 条弹幕**。
- 校验分两层、**都不能省**：① 解压后必须**像 XML**（有 `<i>` 根，允许 BOM），否则报错 ——
  不解压就传给解析器会静默得到 0 条（可能是风控页/404 页）；② 条数由**真解析器**数出来
  （`yekitrim_core::danmaku::parse_bililive_xml`，不是 `grep` 计数），这样"格式变了"会体现在数字上。
- 404 是**合法但空**的特例：弹幕池可能被关闭/清空 → 记一条 warn，回 `EMPTY_XML`（结构完整、只是没有 `<d>`），
  让上层走"项目照样建、但没有 popularity"那条路，而不是整体失败。`BiliError::Malformed` 与 `RateLimited`
  则必须报错，不能当"空弹幕"糊过去。
- **弹幕走旁路（不走下载器）的三个理由**：① 弹幕是 popularity 排序的唯一输入，**失败要能单独重试**
  （不用重下整个视频）；② 弹幕只需要 `cid`，而 `cid` 在 resolve 阶段就拿到了 → 预览卡能显示"实际能收到多少条"；
  ③ 下载器的 `--write-subs` 行为是它的实现细节（哪天改了、或换个引擎，我们不跟着碎）。
- 实测量级：弹幕响应 ≈ `maxlimit×1.2`（实测最大 361KB，**不是全量**）——**别按"几十 MB 流式解析"去设计**；
  连发 8 次未见风控（所以限速不必按 412 阈值设计）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-service bilibili
```

B站侧单测**全部离线可跑**：不打真实网络、不 spawn 进程 —— `extract_bvid` 的形态矩阵（含大小写敏感的
后 10 位）、`session_ref` 编解码往返、`parse_view` 用**实测响应的裁剪版**、`parse_danmaku_response` 的
解压/结构校验/404 空弹幕。
