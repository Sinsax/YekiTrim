# 实测数据与根因推导（2026-10）

> 全部数字来自本机实跑。**本文是结论的出处**，SKILL.md 只摘要点。

---

## 1. 基线矩阵（320×180，`fixtures/sample-project.json`，帧 0/30/60/89）

| 对比 | 最差 SSIM | 逐帧 |
|---|---|---|
| **B ↔ C** | **1.000000** | 四帧全 1.0 |
| **A ↔ C** | 0.720583 | 0=1.0 / 30=0.7206 / 60=1.0 / 89=0.999282 |

### 1.1 变异自证（证明判据非空转）

把 C 侧一帧换成同样 640×360 的**合法 RGBA PNG、像素不同**：

```
SSIM 0.747646   PSNR 5.345 dB   MAE 119.9723
```

换回原图 ⇒ 恢复 1.000000。篡改成非法 PNG ⇒ 解码报错拒收（**不是静默算成一样**）。

---

## 2. A↔C 差异的根因（排除法全表）

样本：`sample-project`，d 层（v2 track）逐一改几何。

| 实验 | SSIM | 排除 / 指向 |
|---|---|---|
| 几何归零（x=0,y=0,scale=1,rot=0） | **1.000000** | 差异**只在有变换时**出现 |
| 去 `gaussian_blur` | 0.570 | 不是模糊 |
| 单独 `scale=0.5` | **0.999977** | 不是缩放 |
| 单独 `rotation=15°` | **0.999851** | 不是旋转 |
| 单独 `x=100,y=-50` | **0.070523** | **平移是大头** |
| `x=1` | **0.484186** | 1px 就崩 |
| `x=0` | **1.000000** | 无平移就完美 |
| `createImageBitmap` 关色彩转换 | 0.572 | 不是色彩空间 |
| **B 端跑同一份 `x=1`** | **1.000000** | **只有源位图来源不同** |

### 2.1 差异的像素级形态（无偏测量，`x=1`）

```
差值 == 0        : 50400 像素 (87.7%)
差值 ≈ 118 / 150 :  12.3%   ← 二值化，不是模糊
差异像素 x mod 8 分布 : {0: 7112}   ← **100% 落在方格边界列**
```

**为什么是边界列**：合成源是 8px 方格（`(x/8 + y/8 + seed) % 2`）。平移 1px 后
输出 x 取源的 x−1 ⇒ 方格内部像素仍属同一格（同色），**只有每格第一列翻相**。
这正是"1 像素位移 + 高频方格"的签名。

### 2.2 结论

| 端 | 上传方式 |
|---|---|
| A | `copy_external_image_to_texture(ImageBitmap)` |
| C | `queue.write_texture(CPU 字节)` |

- **位图内容逐字节相同** —— `x=0` 时 A↔C = 1.000000 就是证据
  （JS 复刻的 `synthetic_source_rgba8` 已与 Rust 对拍：seed + FNV1a64 全等）
- **只在发生采样时暴露** —— `x=0,scale=1,rot=0` 是 1:1 直拷，采样器不参与
- ⇒ **根因是两条源上传路径的采样口径差半像素，不是渲染器差异**
  （A/B 与 C 共用同一份 wasm / 同一份 WGSL）

⚠️ 真实精修页同样走 `setSourceBitmap`（`ImageBitmap`），
所以**只要精修页里发生"源被平移"，就会带上同一口径**。

---

## 3. 稳定性

A 端连跑 3 次，输出逐字节相同：

```
第 1 次: 76425 字节  sha256=cb6171b41e624898
第 2 次: 76425 字节  sha256=cb6171b41e624898
第 3 次: 76425 字节  sha256=cb6171b41e624898
```

B 端帧间产物大小差异明显（5659 / 33111 / 641 / 34504）⇒ 内容确实随帧变化，不是同一张图重复。

---

## 4. JS 与 Rust 的合成源公式对拍

```
a.mp4 f=0   seed 2606505093   fnv1a64 a907b5f4cfed4fbd   ✓
b.mp4 f=10  seed 1386617490   fnv1a64 db010f0582543bfd   ✓
d.mp4 f=20  seed 2772672814   fnv1a64 db010f0582543bfd   ✓
```

**注意**：`b.mp4 f=10` 与 `d.mp4 f=20` **摘要相同但 seed 不同** ——
这是公式的真实性质（checker 只由 `+seed` 的**奇偶**决定），不是移植错误。
⇒ **别拿"不同源摘要相同"当 bug 报**。

---

## 5. 环境能力

| 项 | 结果 |
|---|---|
| `yekitrim server --local-no-auth` | ✅ 三道闸（只许 loopback / 与 `--allow-remote` 互斥 / Host 须 loopback 主机名 / 写请求须同源） |
| `/api/polish/dhampir-wasm/engine.js` | ✅ 200 / 75309 B / `text/javascript` |
| `…/pkg/dhampir_wasm.js` | ✅ 200 / 111556 B / `text/javascript` |
| `…/pkg/dhampir_wasm_bg.wasm` | ✅ 200 / 1539731 B / **`application/wasm`** |
| headless Chrome canvas 像素读回 | ✅ 写入 `(12,34,56)` → 读出 `PX:12,34,56,255` |
| headless WebGPU adapter | ✅ `ADAPTER-OK`（`--headless=new --enable-unsafe-webgpu`） |
| `dhampir.exe render` | ❌ `Permission denied`（写 mp4） |
| `dhampir.exe frame` | ❌ `PermissionDenied` code 5（写 PNG） |
| `dhampir.exe probe` | ✅（只读） |
| `render_project` example | ✅ 写盘正常 |

**`dhampir.exe` 写盘失败的归因**（三条证据 ⇒ 与本仓改动无关）：
未改动的 release 二进制（2026-10-12 构建，早于全部改动）同样失败；
绕开脚本直接调它同样失败；输出到 `%TEMP%` 同样失败。
⇒ C 端**不要用 `dhampir.exe`**，用 `render_project` example。

---

## 6. 两个必须写进结论的纠正

| 曾经的说法 | 实测事实 |
|---|---|
| `render/mod.rs` 只剩 `pub mod dhampir` | **11 个模块**。真正的证据是 `render/backend.rs::RenderBackendKind` 只剩 `Dhampir` 一个变体 |
| golden 可能是底座冻的 | **不是**。`golden.json` 写 `"frozen_from": "yekitrim polish render --renderer chrome"`，而 **Chrome 腿 2026-10-01 整条删除**（`backend.rs` 顶部注释） |
