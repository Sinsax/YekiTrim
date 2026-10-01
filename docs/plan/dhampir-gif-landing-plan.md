# 落地计划：贴纸动图直通底座（GIF/WebP 原生解码）

> 上游提案：`docs/plan/dhampir-gif-native-decode-proposal.md`（讨论稿，111 行）
> 底座定稿：`../Dhampir/plan/dhampir-gif-native-decode-proposal.md` §8–§10（383 行）
> 状态：**V-Trim 侧已接线（2026-10-02）** —— 版本对齐 + 宿主接线 + 门禁都落地了，
> 只剩浏览器真素材验证。执行记录见 §7。

---

## 0. 一句话现状

提案是 V-Trim 提的；**底座已经做完了，而且比提案要求的多**（GIF + 动画 WebP）。
本仓还停在 v5：`DHAMPIR_HOST_API = 5`，预览仍走逐帧 HTTP + base64 PNG。
所以现在的任务不是"推动底座"，而是**把 V-Trim 这一侧接上去**。

---

## 1. 底座交付了什么（已核实，非提案推断）

### 1.1 契约：v6，三条新导出

| 导出 | 返回体 | 语义 |
|---|---|---|
| `dhampir_asset_load_animation(asset_id, bytes)` | `AnimLoadResult` | 整段字节进引擎，引擎解码 + 上传显存 |
| `dhampir_asset_animation_info(asset_id)` | `AnimQueryResult` | 只查不加载（诊断/幂等） |
| （版本号） | `dhampir_host_api_version()` | 已导出 = **6** |

三个形状（`AnimInfoView` / `AnimLoadResult` / `AnimQueryResult`）都在
`../Dhampir/crates/dhampir-timeline/src/host_api.rs`：240（常量）、527–600（形状）。

**与提案的偏差（都对我们有利）**：

- 提案写 `dhampir_asset_load_gif` → 实际叫 `..._load_animation`：**按 magic 嗅探**分派 GIF / 动画 WebP。
- 提案写 `GifInfo{frame_count,width,height}` → 实际 `AnimInfoView` 多给 `format` / `loop_count` /
  `total_ms` / `frame_delays_ms` / `bytes`。
- **形状一个键都没动**（老工程逐字节不变）—— 变的是**导出面**，与 v3→v4 同类。

### 1.2 关键语义（与我们这侧的写法直接相关）

1. **帧号由求值层直出，宿主不换算**。`sources_for(f)` 给的 `SourceView.source_frame` **已经是最终帧号**
   （`layer.rs:831` 的 `source_frame_at_delays` + `:901` 的 `source_frame_looped`）。
   ⚠️ 提案原文写"由底座从秒映射"是**错的**，底座在 §8.2-1 已更正：`seconds` 只用于给 `<video>` seek，
   宿主**不该**拿它算任何东西。我们现有的 `dhampirStickersFrom` 里那套秒→帧号换算要整段删掉，
   不是"改成让底座算"。
2. **时间真值以解码器为准**。`AnimInfoView.frame_delays_ms` 是解码事实，
   工程 JSON 里的 `timebase`/`frame_delays_ms` **退为诊断**，冲突时以解码为准（§8.3-2）。
   → 我们该把返回的 `info` 回写资产表（底座侧有 `insert_with_delays` 对应路径）。
3. **动图缓存排在 `texture_for` 最前**，优先于位图/视频
   （底座注释：让位图抢先，贴纸会永远钉在宿主给的那一帧，表现成"动图不动了"）。
   → 我们**必须停止**对动图资产调 `setSourceBitmap`，否则两边抢。
4. **失败形态分两层**：加载期走返回体的 `error`（不是 issues 数组）；
   渲染期 native 出问题清单、**wasm 预览静默缺层**（`resolver.texture_for` 返回 `None` 就跳过，`render/timeline.rs:1005-1007`）。
   → 预览侧不能指望 `draw` 报错，错误必须在 load 时接住并提示。
5. `bytes` 走 `Uint8Array`；`asset_id` 必须是已登记且 `kind = "image_sequence"` 的资产；
   重复 load 幂等替换；缓存生命周期挂 `open()`。

### 1.3 底座实测数据（clip-18 真实素材）

| 文件 | 文件字节 | 画布 | 帧数 | 总时长 | 显存 |
|---|---|---|---|---|---|
| 叹号.gif | 2.6 MiB | 500x500 | 38 | 1140 ms | 36.2 MiB |
| 思考(认真地).gif | 2.2 MiB | 500x500 | 32 | 960 ms | 30.5 MiB |
| 电风扇2.gif | 1.8 MiB | 500x500 | 32 | 960 ms | 30.5 MiB |
| 笑.gif | 1.9 MiB | 500x500 | 32 | 960 ms | 30.5 MiB |

- 解码器读出的延迟表与 V-Trim 探测的那份**逐值相同**（全 30ms）——"时间真值归一"有实测依据。
- 传输量 **46.2 MiB → 8.4 MiB**（4.1x 膨胀消失）。
- 四张贴纸解码+上传 218ms（Vulkan 真机）。

### 1.3b ⚠️ 实测出来的调用顺序契约（**做的时候才发现，提案里没有**）

拿真 GIF 直接打底座 wasm（node + `dhampir_wasm_bg.wasm`）量出来的：

```
load 在 attach 之前  →  {parsed:true, ok:false, error:"还没 attach 上 canvas：动图现在只能先记账"}
load 在 attach 之后  →  {parsed:true, ok:true, info:{format, width, height, frame_count, ...}}
```

- **纹理上传挂在 `attach()` 上**：host 还没接上时底座**只记账时间真值**、不上传纹理，
  并返回 `ok:false`。也就是说字节白传了、**贴纸一张都不出**。
- ⚠️ **两端都不会报错**：它是个正常的 `ok:false` 返回体，没有异常、没有堆栈 ——
  症状就是"动图不动"，与"格式不支持"长得一模一样。
- ⚠️ `info`（`frame_count` / `frame_delays_ms` / …）**只在 `ok:true` 时给出**，
  所以 attach 之前也拿不到延迟表（"时间真值归一"那一步同样做不了）。

**所以顺序必须是 `open()` → `attach()` → `load_animation()`**，
而且是**两次**都要对：首启（`startDhampir`）与热重载（`reloadDhampirProject`）。
`open()` 会 `clear_animations()`（换工程就是换一批素材），所以 load 也不能排在 open 之前。

本仓的钉子：`scripts/check-dhampir-wiring.mjs` 里断言 `startDhampir` 里
`open` < `attach` < `dhampirLoadAnimations` 的**下标顺序**（含三处缺失的报错分支）。

### 1.4 ⚠️ 一个必须先决策的问题：显存预算

**四张贴纸 = 127.8 MiB，预算是 256 MiB —— 只剩 2 倍余量。**
引用 8 张以上贴纸的工程就会撞上限，而超预算的行为是**明确拒绝**（不是降级）。

底座给了三个对策（**均未实施**）：

1. 按**目标显示尺寸**解码（贴纸通常显示得比 500 小，可省数倍）；
2. 提高预算（如 512 MiB）并让宿主按机器能力传参；
3. LRU 驱逐（⚠️ GIF 无关键帧，驱逐后回放要重播整段，会牺牲 seek 延迟承诺 —— 与提案 §6.3 的
   "seek 首帧 <1 帧间隔"直接冲突，**不建议**）。

底座当初选"按原生尺寸上传"的理由是逐帧一致性（两端像素逐字节相同）。
这是 §3 里我们要一起定的第一件事。

---

## 2. V-Trim 侧改动清单

### T1 — 版本门禁（**第一个做，也是最简单的一步**）

- `webui/src/components/PolishPreview.vue`：`const DHAMPIR_HOST_API = 5` → **6**（第 253 行）。
- `scripts/check-dhampir-wiring.mjs`：跨仓钉子自己会转绿（它读底座 `host_api.rs` 的常量比对）。
- ⚠️ **只改版本号不算完成**：版本号是"拦住两端错配"的闸，改了它但不接字段 = 静默忽略新能力。
  所以这一步要和 T2 一起提交（或在 T2 之后），别单独提交一个"版本对齐"。

### T2 — 宿主接线（**核心**）

在 `PolishPreview.vue` 的帧循环里：

```
现状：POST /api/polish/sticker-frame → base64 PNG → createImageBitmap → canvas → setSourceBitmap

改为：每个贴纸 fetch 一次 GIF（clipMediaUrl，带缓存）
      → dhampir_asset_load_animation(assetId, bytes)
      → 完事（播放期零贴纸请求、零宿主操作）
```

要删的（约 100 行，集中在 `dhampirStickersFrom` 与帧循环）：

- 秒→GIF 帧号换算、相位锚点（底座从"时间线帧 + 延迟表"直出，见 §1.2-1）；
- 30ms 超时赛跑（`textToken` 单飞语义在底座内部解码后整个消失，提案 §3 已确认）；
- 在飞去重；
- 逐帧 `setSourceBitmap`（⚠️ **必须删**，否则与引擎缓存抢，见 §1.2-3）。

要保留的：

- 层区间重建（活跃区间），供诊断条使用；
- `sources_for(f)` 的调用（动图条目照旧出现，底座当前**不过滤**，留作诊断）；
- 失败提示路径 —— 但改为**在 load 时**接住 `error`（§1.2-4：预览期没法报 issue）。

### T3 — 资产表的回写

把 `AnimInfoView.frame_delays_ms` 回写资产表（§1.2-2）。底座侧已有对应的 `insert_with_delays`。
不一致时报 warning 并以解码为准 —— 这条要有断言，否则"映射用一张表、像素用另一张表"的漂移会回来。

### T4 — 服务端

⚠️ **服务端解码器保留**。出片腿在本仓（未切底座）时它仍是权威。
只删**预览消费**：`/polish/sticker-frame` 与 `/polish/sticker-prewarm` 的预览侧调用点。
接口本身先不删（出片腿还在用），标记为"仅出片"。

### T5 — 门禁更新

- `check-dhampir-wiring`：**位图探针换成 GIF 探针**（现在探 `setSourceBitmap`，接线后它不该再出现
  在动图路径上）。
- parity 对拍 `scripts/vtrim-compare.mjs --golden` 重跑三样本。
- 新增：断言动图路径上不再有 `sticker-frame` 调用（防止回退）。

---

## 3. 需要和底座一起定的三件事

底座 §10.9 自己列了未收口项，其中两条直接卡在我们这侧：

| # | 问题 | 为什么卡我们 | 建议 |
|---|---|---|---|
| 1 | `sources_for` 是否过滤动图层 | 当前**不过滤**，动图层照旧出现在清单里 ⇒ JS 侧不能"清单里有的源都去 seek" | **保持不过滤**。我们改 JS 侧的消费端（只对 video 条目 seek），不动契约形状 |
| 2 | 动图层要不要参与预渲染缓存（`begin_frame` 内容标识） | 若 GIF 层不进内容标识，预渲染缓存可能在"GIF 帧已变、视频源未变"时**错误命中** | 把 `(asset_id, 引擎算出的帧号)` 纳入内容标识。这条**必须底座改**（标识在引擎内部） |
| 3 | 显存预算（§1.4） | 8 张贴纸就撞 256 MiB | **按目标显示尺寸解码** —— 两端仍共用同一份实现，逐帧一致性不受影响（差异只在采样器上），能省数倍显存 |

第 2 条是**正确性问题**（会出错误画面），优先级高于第 3 条的容量问题。

---

## 4. 验收判据（V-Trim 侧要跑的）

底座侧已给的（§8.5 + §10）：

1. 非匀速与匀速两条路径各出一条黄金帧；
2. 同一 GIF 多层/多区间引用 → 只解码一份（可断言解码计数）；
3. 损坏 GIF → 加载期报错 + 该层不渲染 + 两端不 panic；
4. `begin_frame` 内容标识含 GIF 帧（换帧不被缓存去重盖掉）；
5. 延迟表真值冲突（解码 vs 工程 JSON）报 warning，以解码为准。

V-Trim 侧新增的：

6. **播放期零贴纸请求**（网络面板断言，这是本提案最大的收益点）；
7. **seek 到任意时刻贴纸首帧即时**（<1 帧间隔）；
8. 页面加载每贴纸**一次**传输（GIF 原文件）；
9. 逐帧一致：同一工程同一帧号，预览贴纸像素与出片一致。
   ⚠️ 过渡期口径是 ≤20/255（服务端解码器仍在），**终态应另设收紧线（如 ≤2/255）**
   —— 否则"同解码器"的收益被宽阈值掩盖（底座 §8.4 提醒）。

---

## 5. 建议的落地次序

```
① 与底座定 §3 的三条（尤其第 2 条内容标识）      ← 先定,否则返工
② T1 版本号 + T2 宿主接线（一起提交,浏览器手测 clip-18）
③ T3 资产表回写 + T5 门禁（位图探针换 GIF 探针）
④ T4 服务端预览消费下线（出片腿保留）
⑤ parity 对拍 + 黄金帧重跑,过渡期 20/255 → 终态收紧线
⑥ 全量出片跑一次（底座 §10.9 说"用这个项目跑完整出片"还没做）
```

⚠️ **②之前先在浏览器里跑真素材**：底座 §10.9 明确"浏览器预览侧未跑" ——
wasm 接线只编译过、没喂过真 GIF。这一步的风险最高。

---

## 6. 相关文件与位置索引

**本仓**

| 位置 | 内容 |
|---|---|
| `webui/src/components/PolishPreview.vue:253` | `DHAMPIR_HOST_API = 5` → 6 |
| `webui/src/components/PolishPreview.vue:682` | `dhampirStickersFrom`（帧号/相位/在飞去重，要删） |
| `webui/src/components/PolishPreview.vue:1179` | `/api/polish/sticker-frame` 调用点 |
| `webui/src/components/PolishPreview.vue:1223` | `setSourceBitmap`（要删） |
| `webui/src/components/PolishPreview.vue:1598` | `/api/polish/sticker-prewarm` 调用点 |
| `scripts/check-dhampir-wiring.mjs` | 跨仓版本钉子 + 位图探针 |
| `tools/polish-to-dhampir.mjs:1780` | `kind: "image_sequence"` 的字段写出 |

**底座（`../Dhampir`）**

| 位置 | 内容 |
|---|---|
| `crates/dhampir-timeline/src/host_api.rs:240` | `HOST_API_VERSION = 6` |
| `crates/dhampir-timeline/src/host_api.rs:527-600` | 三个 v6 形状 |
| `crates/dhampir-core/src/animation.rs` | 解码模块（449 行）+ 三道闸 |
| `crates/dhampir-core/src/render/animation.rs` | 纹理缓存 + 显存预算闸 |
| `docs/host-api.md` | v5→v6 一节（契约正文） |

---

## 7. 执行记录（V-Trim 侧，2026-10-02）

### 已完成

| 项 | 落点 | 说明 |
|---|---|---|
| T1 版本对齐 | `PolishPreview.vue` | `DHAMPIR_HOST_API` 5 → **6** |
| T2 宿主接线 | `PolishPreview.vue` | 新增 `dhampirLoadAnimations()`；删掉动图的逐帧取帧/位图提交 |
| T3 时间真值 | 同上 | 把 `info.frame_delays_ms` 存进 `st.delays` |
| T4 服务端标注 | `api/polish.rs` | `sticker-frame` 注释标明"只剩出片腿 + 静态图用" |
| T5 门禁 | `check-dhampir-wiring.mjs` | 动图探针 + **顺序探针** |
| — 预热收窄 | `PolishPreview.vue` | `callPrewarmStickers` 不再把动图交上去（白烧 CPU） |
| — 读数分栏 | `PolishPreview.vue` | 工具条把"引擎解码 / 取帧"分开念 |

### 关键实现点

1. **`pkgNs` 提到模块作用域**（`dhampirPkgNs`）：底座的 `loadEngine()` 返回的是
   `new Engine(mod)` **包装对象**，那两条 v6 导出**不在它身上**（底座 `web/engine.js` 还没包）。
   所以只能拿模块命名空间直调 —— ESM 缓存保证与 `loadEngine` 内部同一份，不重复下载。
2. **`open → attach → loadAnimations`**：见 §1.3b（本轮实测才发现，已加门禁钉子）。
3. **静态图仍走老路**：底座的动图这条路**只收动图**（单帧图明确报 `NotAnimation`），
   所以帧循环里 `if (st.animLoaded) continue` 只跳过动图，静态图照旧逐帧交位图。
4. **动图绝不调 `setSourceBitmap`**：底座把动图缓存排在 `texture_for` 最前，
   宿主再交位图就是两边抢 —— 症状是贴纸**永远钉在宿主给的那一帧**。

### 验证

- `node scripts/check-dhampir-wiring.mjs` —— **84 项全绿**（含跨仓版本 6 == 6）。
- 7 个 `webui/scripts/check-*.mjs` + `check-font-inputs` + `check-vtrim-translator`
  + 两条模块门禁 —— 全绿。
- `npm --prefix webui run build` —— 通过，`dist` 已重建。
- **直打底座 wasm 量出调用顺序契约**（node + 真 GIF 非问号.gif，2.6MB）—— 见 §1.3b。

### 未做（诚实的边界）

- **浏览器真素材验证**：本机没有 `clip-18` 那批素材（`media/` 是空的），
  只用手头的 `问号.gif` 验到了"load 契约与顺序"这一层。
  真正的"动图在预览里动起来"需要在有素材的机器上开真浏览器跑一遍。
- T3 只做了"存下来"，还没把冲突（解码表 vs 工程 JSON）报成 warning。
- 显存预算问题（§1.4）按用户要求**本轮不处理**。
