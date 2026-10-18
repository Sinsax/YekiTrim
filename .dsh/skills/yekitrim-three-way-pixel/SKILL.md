---
name: yekitrim-three-way-pixel
description: 'Use when verifying that YekiTrim''s preview and the Dhampir base render the same pixels — 三端对拍/像素级一致/预览与成片画得一样/改了渲染怕画歪. Runs A (engine.js host), B (wasm direct), C (native worker) on one project and one frame number, then judges with SSIM. Knows that B↔C is the authoritative line, that A↔C can never be exactly 1.0 (source-upload sampling differs half a pixel), and that docs/golden is NOT a third end.'
---

# 三端像素级对拍

**回答的问题**：「预览和成片画出来的，是不是同一张图？」改完渲染/转译之后，这是唯一的最终判据。

## ★ 先记住四件事，否则结论一定是错的

1. **B↔C 才是权威判据**。两侧的源由 Rust 的同一对函数（`synthetic_source_rgba8` +
   `synthetic_seed_for_source_frame`）造出，**输入逐字节相同是"构造出来的"，不是"声明出来的"**。
2. **A↔C 永远不可能严格 =1.0**（含变换时）。A 走 `copy_external_image_to_texture(ImageBitmap)`，
   C 走 `queue.write_texture(CPU 字节)`，**两条上传路径的采样口径差半像素**。
   实测：`x=0` 时 **1.000000**，`x=1` 时 **0.484186**，`x=100` 时 **0.070523**。
   ⇒ 判据必须按「几何无变换 / 有变换」分层。
3. **`docs/golden/dhampir-parity/` 不是第三端**。它的 `golden.json` 写着
   `"frozen_from": "yekitrim polish render --renderer chrome"` —— 那是**已删除的 DOM 腿**
   冻的，**不是底座画的**。拿它判底座对错会得出错误结论。
4. **A 与 B 是同一份 wasm 的两个宿主**（`engine.js` 自己写着"这一层是底座的一部分"）。

## 三端各是什么

| 端 | 实现 | 源从哪来 | 出口 |
|---|---|---|---|
| **A** | `engine.js` 宿主（精修页那条腿） | 调用方逐帧交 `ImageBitmap` | canvas → `toDataURL` |
| **B** | 直接 `import` wasm（`web/synthetic.html`） | **Rust 内部**造 | `dhampir_sample_project_render_png` |
| **C** | native worker（`render_project` example） | **Rust 内部**造（同一对函数） | 写 PNG |

## 怎么跑

判据实现在 `scripts/run.mjs`（导出 `judge()` / `findDhampir()`，含自证）：

```bash
SK=.dsh/skills/yekitrim-three-way-pixel

# 判据自证（不跑浏览器，秒级）
node $SK/scripts/run.mjs --self-test

# 完整三步编排见 reference/runbook.md
```

## 判据分层（**这就是为什么不能只报一个数**）

| 层 | 比法 | 通过线 | 回答什么 |
|---|---|---|---|
| **L1 严格** | 几何无变换的帧 | **SSIM = 1.000000** | 渲染逐像素相同吗 |
| **L2 宽松** | 含平移/缩放/旋转的帧 | SSIM ≥ 0.55 | 没有大改 |
| **L3 权威** | **B ↔ C** | **SSIM = 1.000000** | 底座两端一致吗 |
| **L4 结构** | A ↔ B | 差异区域 ⊆ 被采样的源区域 | 宿主层画歪了吗 |

⚠️ **一帧都没比 = "没跑"，不是"一致"** —— 判据里必须分开报（`run.mjs` 已这么做）。
⚠️ **尺寸不同 ≠ 画面不同** —— 也要分开报，否则看不出该查哪里。

## 必跑的前置（顺序不能换）

```bash
# ① 免登录起服务（wasm 资源那条路由在 nest("/api") 下，要认证）
./target/debug/yekitrim.exe server --local-no-auth

# ② 验证三件资源都拿得到（MIME 必须是 application/wasm）
curl -s -o /dev/null -w "%{http_code} %{content_type}\n" \
  http://127.0.0.1:26320/api/polish/dhampir-wasm/pkg/dhampir_wasm_bg.wasm
```

⚠️ `--local-no-auth` 让**同机任何进程**都能调这套 API。**测完记得关**。

## 会踩的坑（都实测过）

| 坑 | 现象 | 正解 |
|---|---|---|
| 路径少了 `/api` | 返回 **1007 字节的 HTML**，看着像 200 | 路由在 `nest("/api")` 下；那是 **SPA 兜底**，不是 404 |
| 拿 `<video>` 真解码 vs 合成源比 | SSIM 极低 | **输入不同源，这个数没有意义** —— 先对齐输入 |
| 陈旧的 wasm pkg | 报的错与原因毫无关系 | 改 Rust 后必须重建 wasm 包 |
| Chrome 复用 user-data-dir | 页面是旧的 | 每次用全新 profile |
| `onPrepared` 之外交位图 | "交与不交，画面逐字节相同" | 静态源**必须**在 `engine.seek(frame, {onPrepared})` 的缝隙里交（`prepare` 第一件事是 `clear_bitmaps()`） |
| `dhampir.exe` 写盘失败 | `Permission denied` | C 端改用 `render_project` example（**它写盘正常**） |

## 环境结论（2026-10 实测）

| 环节 | 状态 |
|---|---|
| 免登录取 wasm（`--local-no-auth`） | ✅ 200 + `application/wasm` |
| headless Chrome + canvas 像素读回 | ✅ |
| headless WebGPU | ✅ `ADAPTER-OK` |
| A / B / C 三端都能出帧 | ✅ |
| **B↔C** | ✅ **SSIM = 1.000000**（帧 0/30/60/89） |
| **A↔C** | 帧 0/60 = 1.0；帧 30 = 0.7206；帧 89 = 0.9993 |
| A 端连跑 3 次 | ✅ 逐字节相同（`sha256=cb6171b41e624898`） |
| `dhampir.exe` 写盘 | ❌ 被拦（**与本仓改动无关**：未改动的 release 二进制同样失败） |

## 参考

- `reference/findings.md` —— 完整实测数据与根因推导（含排除法全表）
- `reference/runbook.md` —— 三步编排的详细命令
- 本仓 `docs/plan/three-way-pixel-test-report.md` —— 同一份结论的仓库内版本
