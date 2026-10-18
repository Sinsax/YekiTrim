# 三步编排（完整命令）

> 全程**只读**底座仓；产物落在 `<Dhampir>/target/threeway/`（gitignore）。
> 唯一会往底座仓写的操作是**临时放一个探针页**（第三步），跑完必须删掉。

---

## 前置：确认底座可用

```bash
DH=/f/para/Code/Dhampir          # 底座仓根（或设 DHAMPIR_ROOT）
YK=/f/para/Code/YekiTrim         # 本仓根

# 三件资源都在吗
ls $DH/web/engine.js
ls $DH/web/synthetic.html
ls $DH/crates/dhampir-wasm/www/pkg/dhampir_wasm.js
ls $DH/fixtures/sample-project.json
```

---

## 第 1 步：起免登录服务（A 端要拿 wasm 资源）

```bash
cd $YK
./target/debug/yekitrim.exe server --local-no-auth    # 后台放着

# 验三件（MIME 必须是 application/wasm）
curl -s -o /dev/null -w "%{http_code} %{content_type}\n" \
  http://127.0.0.1:26320/api/polish/dhampir-wasm/engine.js
curl -s -o /dev/null -w "%{http_code} %{content_type}\n" \
  http://127.0.0.1:26320/api/polish/dhampir-wasm/pkg/dhampir_wasm_bg.wasm
```

⚠️ 路径**必须**带 `/api`。少了它返回 **1007 字节 HTML**（SPA 兜底），看着像 200。

⚠️ `--local-no-auth` 下**同机任何进程**都能调这套 API。测完 **关掉服务**。

---

## 第 2 步：B 与 C（这两端不需要浏览器）

**C 端** —— native worker 出帧：

```bash
cd $DH
mkdir -p target/threeway/C
cargo run --quiet -p dhampir-worker --example render_project -- \
  fixtures/sample-project.json target/threeway/C 0 30 60 89
# → {"frame":0,"layers":1,...} + frame-NNNN.png
```

**B 端** —— 直接 import wasm 的合成出口：

```bash
cd $DH
node scripts/web-check.mjs --synthetic 0,30,60,89 --frames-only
# → target/export/frames/frame-NNNN.png
```

**判 B↔C**（权威线，必须 =1.000000）：

```bash
cargo run --quiet -p dhampir-worker --example compare_project -- \
  target/export/frames target/threeway/C 0 30 60 89
```

> 更省事：底座自带编排 `node scripts/check-dual-end.mjs`（它会自己起浏览器导出 +
> 跑 worker + 调 compare_project，并判 SSIM=1.0）。**B↔C 用这条就够。**
> 它有自证：`node scripts/check-dual-end.mjs --self-test`（5 条断言）。

---

## 第 3 步：A 端（engine.js 宿主）

⚠️ **A 端没有现成出口** —— 需要临时放一个探针页。它必须住在 `web/` 下
（静态路由只服务 `web/`、`pkg/`、`fixtures/` 三处）。

探针页要点（少一条就画不出图）：

1. `import { loadEngine } from '/engine.js'` —— **走宿主**，不是直接 import wasm
2. `await loadEngine('/pkg/dhampir_wasm.js')`
3. **顺序**：`open(project)` → `await attach('preview')` → `setSourceMode('bitmap')`
   （`attach` 之前调 `resize` 会撞"宿主尚未初始化"）
4. 源位图**必须**在 `engine.seek(frame, { onPrepared })` 的缝隙里交 ——
   `prepare` 的第一件事是 `clear_bitmaps()`，早交晚交都会被清掉，
   现象是"交与不交，画面逐字节相同"
5. 取帧：`canvas.toDataURL('image/png')`

合成源位图（与 B/C 同源，逐字节相同）：

```js
function syntheticSeedForSource(s) {
  let h = 0x811c9dc5;
  for (const ch of s) { h ^= ch.charCodeAt(0); h = Math.imul(h, 0x01000193) >>> 0; }
  return h >>> 0;
}
function syntheticSeedForSourceFrame(source, frame) {
  return (syntheticSeedForSource(source) ^ Math.imul(frame >>> 0, 0x9e3779b9)) >>> 0;
}
// 8px 方格 + 水平渐变 + 垂直渐变，alpha 255/200
```

> 已与 Rust 对拍：seed 与 FNV1a64 全等（见 `findings.md` §4）。
> **改这个公式前先重新对拍** —— 它是"输入逐字节相同"的唯一依据。

---

## 判据

`scripts/run.mjs` 导出 `judge(results, opts)`（**判据只有这一份实现**）：

```js
judge(
  [{ frame, pair: 'B↔C', ssim, strictRequired: true,
     leftMissing?, rightMissing?, sizeMismatch?, leftSize?, rightSize? }],
  { frames, strict: false, requireAuthoritative: true }
)
```

| 开关 | 作用 |
|---|---|
| `strict` | 把 A↔C 也按严格线判（默认只按含变换线） |
| `requireAuthoritative` | 要求必须有 B↔C —— **完整三端对拍要开**；单测一对时别开 |

自证：`node scripts/run.mjs --self-test`（10 条断言）。

---

## 收尾（**必做**）

```bash
cd $DH
rm -f web/<探针页>.html                      # ← 必须删，别提交
git status --short                           # 应只剩你自己的改动
```

---

## 判读表

| 现象 | 含义 | 去看 |
|---|---|---|
| B↔C < 1.0 | **真不一致** | 渲染路径 / 效果实现 |
| A↔C 严格帧（无变换）< 1.0 | **宿主层画歪** | `engine.js` 的 prepare/attach 时序 |
| A↔C 含变换帧 ~0.5–0.7 | **正常**（采样口径差半像素） | 不用查，除非差异不在方格边界列 |
| A↔C 含变换帧 < 0.5 | 比半像素口径更糟 | 查源位图内容是否真的同源 |
| 一帧都没比 | **没跑**，不是一致 | 检查导出是否成功 |
| 尺寸不同 | **不可比** | 检查 `--canvas` / `width,height` 是否一致 |
