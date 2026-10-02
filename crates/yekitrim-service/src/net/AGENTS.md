# net — HTTP 层小工具（解压 / 浏览器 UA，**不含任何平台知识**）

> 范围：`crates/yekitrim-service/src/net`/** ｜ 上游：`yekitrim-service::bilibili`（view API / 弹幕）｜ 验证：`cargo test -p yekitrim-service net`

## 职责

发请求时踩到的**通用坑**收在一处：HTTP 响应的解压（B站返回的是裸 deflate）与一个带浏览器 UA 的
`reqwest::Client`。这里**不认识 B站、也不认识下载器** —— 那些分别在 `bilibili/` 与 `downloader/`。

## 入口

- `deflate::inflate(bytes) -> Result<(Vec<u8>, Flavor)>`：解开一个可能被压缩的响应体，同时回**走的是哪条路**
  （`Flavor::{Plain, RawDeflate, Zlib, Gzip}`，用于日志与测试 —— 出问题时能一眼看出风味）。
- `BROWSER_UA`：浏览器 UA 常量；`browser_client() -> reqwest::Client`：带 UA + `connect_timeout` 15s +
  `timeout` 60s 的客户端。

## 边界

- ⚠️ **本目录不许知道 B站**（判据 `bvid` / `cid` / `bilibili.com`，门禁 `net/ 不许知道 B站`）：
  deflate 解码是通用工具，不该知道调用方是谁。改了本目录顺手跑 `python3 scripts/check_module_boundaries.py`。

### `deflate.rs`——为什么不是给 reqwest 加 feature

- B站弹幕接口（`comment.bilibili.com/{cid}.xml`）返回 `Content-Encoding: deflate`，但载荷是
  **raw deflate（zlib wbits = -15）** —— 没有 zlib 头、没有 adler32 校验。这**不符合 HTTP 规范**
  （规范要求 deflate = zlib 包装）。后果：① `reqwest` 的 `deflate` feature 按规范走 zlib 包装，**解不开**；
  ② 实测 `Accept-Encoding: identity` 会被服务端**忽略**，强制压缩；③ 所以只能手工解，而且要按 raw 风味优先试。
  解码器用 `flate2`（已在 `Cargo.lock`，经 png/ureq/zip 引入 —— 提升为直接依赖不引入新传递依赖）。
- `inflate` 的顺序刻意是"**先看魔数，再用 B站的实际风味兜底**"：① 明文（`<?xml` / `<i>` / 合法 UTF-8 文本）
  → 原样返回（服务端哪天不压缩了也不会炸）；② gzip（`1f 8b`）/ zlib（`78 {01,5e,9c,da}`）有固定魔数 →
  直接按它解；③ 否则按**裸 deflate** 解（B站走的就是这条）；④ 裸 deflate 也失败 → 再试 zlib
  （有些中间层会加上头）。任何一步都不 panic：垃圾字节返回 `Err`，由调用方决定是报错还是重试。
- ⚠️ **严格解码：必须读到流结束（`Status::StreamEnd`）且把输入消费干净。**
  不要用 `flate2::read::*Decoder::read_to_end` —— 它**不要求流正常结束**：实测
  `b"random ascii that is not xml or deflate"` 会被它"解出"一段非空输出并返回 `Ok`，
  于是 `inflate` 把一坨垃圾当成解压成功交出去，上层只会看到"0 条弹幕"（静默）。
  严格判定在实测两侧都成立：真实 B站载荷（`cid=122300483`，145,077 字节）→ `eof=true / unused_data=0`；
  4 组垃圾字节（全零、`ff fe fd fc`、随机 ASCII、假 zlib 魔数）→ 全部 `Err`。
- 明文的判据刻意**窄**（只认 XML 声明或 `<i>` 根开头），避免把二进制误判成明文。

### `mod.rs`——UA 与客户端

- ⚠️ **`BROWSER_UA` 不是一个可以随便改的装饰**：实测非浏览器 UA（`python-urllib/3.13`）请求
  `comment.bilibili.com` 会直接被判 **HTTP 412**，响应头带 `x-sec-request-id`（风控）。`reqwest/x.y` 同理。
  所以**凡是要打 B站的请求都必须带上它**。单测钉住"看起来像浏览器"（`Mozilla/5.0` + `Chrome/`、
  不含 `reqwest`、不含换行）。
- `browser_client()` 的**超时是必要的**：这些接口没有 SLA，卡住比失败更糟（下载槽会被占着）。
  **不设 `Accept-Encoding`**：实测 B站**强制** deflate 且忽略 `identity`，默认值反而省事 ——
  手工解压见 `deflate::inflate`。
- 客户端建不出来只可能是 TLS 后端起不来这种环境级问题：**降级为默认客户端 + warn 继续**，
  让业务层拿到"请求失败"而不是在构造点 panic（构造点没有 `catch_unwind` 兜底，panic 会连累调用它的那个任务）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-service net
```

`inflate` 的单测**离线可跑**：明文 / raw deflate / zlib / gzip 四风味、真实载荷、以及
**4 组垃圾字节必须全部 `Err`**（那条"别用 `read_to_end`"的坑就靠它钉住）；UA 的单测钉住它看起来像浏览器。
