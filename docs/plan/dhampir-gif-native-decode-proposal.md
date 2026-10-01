# 提案：底座原生解码 GIF（贴纸动图直通）

> 来源：V-Trim 仓库（精修预览 + 出片宿主）。目的：把贴纸动图从「宿主逐帧喂位图」改成
> 「底座引擎自己解码、自己按时间取帧」，消灭逐帧 HTTP 与宿主侧整条取帧管线。
> 状态：**讨论稿** —— API 命名与内部实现由底座定，本文只钉契约语义与验收判据。

---

## 1. 现状（V-Trim 侧，2026-10 实测数字）

贴纸是全链路**唯一**还在逐帧走网络的资产：视频是宿主侧 `<video>` 直供，字幕/弹幕在 wasm 里解析。
贴纸现状：

```
进页面     V-Trim 服务端预热：整段解码 GIF → PNG 帧表（进程内缓存，键 = mtime+len）
           冷解码单张 3~9s（66 帧 GIF，debug 构建）
播放中     宿主按帧调 POST /api/polish/sticker-frame → 服务端缓存取帧 → base64 PNG（~390KB/帧）
           回程 14~25ms/帧；宿主 createImageBitmap → 常驻 canvas → setSourceBitmap
```

宿主侧整条管线（`webui/src/components/PolishPreview.vue`，约 100 行）：
按素材时基把「工程帧 → 秒 → GIF 帧号」换算、相位锚点、在飞去重、30ms 超时、PNG 解码中转。

痛点：① 逐帧 HTTP（seek 后首帧晚 ~20ms、偶发一帧抖动）；② base64 膨胀 1.33×；
③ 宿主自己换算帧号 —— 相位/延迟语义两处实现（服务端出片 vs 前端预览），一致性靠纪律守。

## 2. V-Trim 已经提供给底座的数据（工程 JSON 里，无需新增）

转译器（`tools/polish-to-dhampir.mjs`）为每个贴纸资产写的字段，**底座可以直接消费**：

| 字段 | 语义 | 备注 |
|---|---|---|
| `kind: "image_sequence"` | 动图资产 | 语义不变；底座可继续按 kind 分派 |
| `uri` | **相对 asset-root（片段目录）** 的 GIF 原文件路径 | 如 `assets/stickers/生气.gif` |
| `frame_count` | GIF 帧数（探测所得） | |
| `timebase` | GIF **实测平均帧率**（如 100/3 ≈ 33.3fps） | ⚠️ 曾写死 10fps → 动图慢 3.33 倍，别回退 |
| `frame_delays_ms` | **逐帧延迟表，仅非匀速 GIF 才写**（匀速缺省 = 用 timebase） | JSON 里 `undefined` 会被丢弃，即「缺省 = 匀速」 |
| `width` / `height` | GIF 原生尺寸 | |

**时间的唯一权威是「秒」**：引擎现在对每个源算 `seconds`（含 `source.source_in` 偏移），
GIF 帧号应当由底座从秒映射：**匀速** `frame = floor(sec × fps)`；**非匀速**按
`frame_delays_ms` 前缀和查表。宿主不再自己算帧号。

## 3. 提议的契约变更（HOST_API_VERSION 5 → 6）

新增（命名示意，底座定稿）：

```rust
// 宿主把 GIF 原始字节一次性交给底座（由宿主负责取文件：
// 浏览器 = fetch 媒体 URL；原生 = 按 asset-root 读盘 —— 与视频源同一分工）。
// 返回 {frame_count, width, height}，底座内部建延迟表与帧缓存。
dhampir_asset_load_gif(asset_id: &str, bytes: &[u8]) -> Result<GifInfo, Issues>

// 可选：宿主查询（诊断/进度用），不参与逐帧渲染。
dhampir_asset_gif_info(asset_id: &str) -> Option<GifInfo>
```

删除/免掉的宿主职责：逐帧 `set_bitmap`、帧号换算、相位锚点、取帧 HTTP。
**不变**：`sources_for(f)`、`seek → prepare → draw` 单趟流水线、`clear_bitmaps` 时机、
`textToken` 单飞语义（底座内部解码后，宿主那次 30ms 赛跑与在飞去重整个消失）。

兼容策略建议：版本号硬门禁（现有机制：宿主 `DHAMPIR_HOST_API` 与底座
`dhampir_host_api_version()` 不等就当场报错），**两边同一次提交里一起 bump**。
V-Trim 侧有跨仓门禁 `scripts/check-dhampir-wiring.mjs` 把这个数钉住（现在 = 5）。

## 4. 底座实现要点（已知坑，全部实测过）

1. **GIF 解码器**：纯 Rust `gif` crate（MIT/Apache，无 GPL 传染）即可；
   disposal method（restore-to-bg / restore-to-previous）、局部调色板、交错存储都要走对 ——
   V-Trim 服务端解码器（`decode_sticker_frames_png`）是出片权威，两边语义对齐。
2. **内存预算（最要紧）**：解码 RGBA 帧很大 —— 500×500×4B ≈ 1MB/帧，66 帧 ≈ **66MB/张**，
   四张贴纸就是 264MB，不能全量常驻。建议二选一：
   - **按目标尺寸解码**：渲染目标尺寸 = 工程缩放后的盒子（`280 × min(W,H)/1080` 那套），
     解码时直接缩到目标 → 每帧 ~0.3MB，66 帧 ~20MB/张，可全量常驻；
   - 或 **LRU**（保留活跃 span ±N 帧），上限 ~16 帧/张。
3. **非匀速延迟**：`frame_delays_ms` 缺省 = 匀速（timebase）；给了就按前缀和查表，
   **不要**把首帧延迟算进相位（第 0 帧从层起点起播）。
4. **多实例**：同一 GIF 可能被多层/多区间引用（asset_id 相同）—— 解码缓存按 asset_id 建
   一份，层只持引用。
5. **失败形态**：GIF 损坏/超限（如 >64MB）时**保留该层但报 issue**（走 `open()` 的 issues 通道），
   不要 panic —— 出片逐帧路径里任何 panic 都是整块 chunk 失败。
6. **原生/浏览器同源**：出片（native）与预览（wasm）必须用**同一个解码实现** ——
   这是本提案的核心收益：贴纸帧的一致性从「两份实现对齐」变成「同一份实现」。

## 5. V-Trim 侧将配套做的事（底座就绪后一次落地）

- `DHAMPIR_HOST_API` 5 → 6（含 `scripts/check-dhampir-wiring.mjs` 的跨仓钉子）。
- 宿主删整条逐帧管线：`/polish/sticker-frame`、`/polish/sticker-prewarm` 的**预览消费**、
  `dhampirStickersFrom` 的帧号/相位/在飞去重逻辑（保留层区间重建，供诊断条）。
  服务端解码器保留 —— 出片腿（V-Trim 服务端 PNG 帧）在底座出片也切 GIF 前仍是权威。
- 帧循环里改为：每贴纸 fetch 一次 GIF（`clipMediaUrl`，带缓存）→
  `dhampir_asset_load_gif(assetId, bytes)` → 完事。
- 门禁更新：`check-dhampir-wiring` 的位图探针换成 GIF 探针；parity 对拍
  （`scripts/vtrim-compare.mjs --golden`）重跑三样本。

## 6. 验收判据（两边共用）

1. **逐帧一致**：同一工程、同一帧号，预览贴纸像素与出片一致
   （V-Trim 侧黄金帧对拍：基线中位像素差 ≤ 现行阈值 20/255，贴纸窗口重点抽查）。
2. **相位正确**：GIF 从层起点起播、`source_in` 偏移生效；非匀速表按延迟走
   （构造一个 [30,30,…,200,…] 的测试 GIF 钉住）。
3. **性能**：页面加载每贴纸一次传输（GIF 原文件，通常 0.1~2MB）；播放期**零**贴纸请求；
   seek 到任意时刻贴纸首帧即时（<1 帧间隔）；内存 ≤ §4.2 预算。
4. **版本门禁**：版本不匹配当场报错（不是静默回退近似预览）。

## 7. 不建议的替代路线（为何不是它们）

- **前端 JS 解码 GIF**：目标浏览器无 `ImageDecoder`（实测）；且换解码器 = 预览/出片
  逐帧一致性失保（disposal/透明处理各家有微差）。
- **服务端批量预取 PNG**：内存峰值高（~100MB 级）、解码总量不变，只是把请求藏起来。
- **维持逐帧 + 小批量预取**：能解 seek 首帧延迟，但「两份帧号实现」的结构性成本一直在。
