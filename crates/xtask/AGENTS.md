# xtask — 构建 / 打包 / 产物门禁（`cargo xtask scan|package`）

> 范围：`crates/xtask`/** ｜ 上游：发布流程（`cargo xtask package`）与本地验证（`cargo xtask scan`）｜ 验证：`cargo check -p xtask`

## 职责

仓库任务二进制，两个子命令：**产物门禁**（扫已链接的二进制里有没有 espeak-ng 标记）与**发布打包**
（构建 → 门禁 → 组装 → zip，可选 smoke test）。它取代了历史上的 `scripts/package-release.ps1` 与
`scripts/scan-artifact.ps1`，让整条构建与发布流程跑在**一套工具链**上：不要 PowerShell、不要 tar/7z/bzip2/python，
连带它们那些 ASCII-only / BOM / `$LASTEXITCODE` 的坑一起消掉。

**准备 TTS-off 的 sherpa-onnx 归档不在本 crate** —— 那在 vendor 的
`sherpa-onnx-sys/build.rs`（见 `vendor/sherpa-onnx-sys/FORK-NOTES.md`），是构建**无法绕开**的唯一一处。
本二进制只对它的产物做门禁与打包。

## 入口

- 通过 `.cargo/config.toml` 的别名：`xtask = "run -p xtask --"` —— 所以是
  `cargo xtask scan [path] [--show-bare-espeak]` 与
  `cargo xtask package [--skip-build] [--smoke-test] [--version V] [--out-dir DIR]`。
- `scan`：`path` 默认 `target/release`；递归收 `SCAN_EXTENSIONS`（`exe` / `dll`），
  跳过 `SKIP_DIRS`（`build` / `deps`）与**本工具自己的二进制**（见「边界」），逐文件计数 `MARKERS`（5 个：
  `espeak-ng` / `espeak ng` / `espeak_` / `libespeak` / `piper_phonemize`，**不区分大小写**）。
  显式传单个文件时按原样扫、不做自我过滤（`package` 的门禁走的正是这条）。
- `package`：版本 → `cargo build --release -p yekitrim-cli --locked` → `yekitrim.exe` → 调 `scan` 门禁 →
  查 `webui/dist` → 组装 → 逐项断言必需文件 →（可选）`smoke_test` → `zip_dir` → 打印 size 与 sha256 →
  **写 `<zip>.sha256.txt` 校验文件**（2026-10-01 起自动生成，见「边界」最后一条）。
- 打包内容：`yekitrim.exe`、`webui/dist/`（整个）、`resources/fonts/放这里.txt`、
  `LICENSE`、`THIRD-PARTY-NOTICES.md`，以及 `docs/releases/v<版本>.md`（存在才拷）；
  **外加默认出片通道的随身件（2026-10-01）**：`tools/polish-to-dhampir.mjs`（转译器，它按**同层**
  `../spec/effects.json` 读效果词汇表）、`spec/effects.json`，以及底座三样
  `dhampir/bin/<exe>` + `dhampir/preview/{engine.js,pkg/*}` + `dhampir/VERSION`
  （包内形状由 `yekitrim_polish::render::dhampir::{find_binary,wasm_dirs,preview_dir}` 定义；
  ⚠️ `VERSION` 在 **`dhampir/` 下**、不在 `preview/` 里 —— 摆错报错只说"缺 VERSION"）。
  底座来源 = 同层 `../Dhampir`（产物默认 `../Dhampir/out`、二进制默认 `../Dhampir/target/release/dhampir.exe`）；
  可用 `--dhampir-artifact <dir>` / `--dhampir-exe <path>` 覆盖。**找不到就拒绝打包** ——
  确实要打"没有底座"的包得显式 `--no-dhampir`（那台机器上默认通道会不可用）。
  输出到 `<repo>/release/`（gitignored），包名 `yekitrim-v<版本>-win-x64`，zip 用 Deflated level 9。
- `smoke_test`：起打包好的 exe（端口 `18000 + 纳秒%1000`，Windows 上 `CREATE_NO_WINDOW`），
  最多 20 次 × 500ms 轮询 `/api/health`（含 `"ok"` + `true`）与 `/`（` 200 ` + `<div id="app">`），
  然后 kill 并**清掉 `package_dir` 里的 `.yekitrim` 与 `stickers`**（首次运行会生成它们，运行态不许进发布包）。

## 边界

- ⚠️ **门禁不可跳过**：`package` 里的 `scan` 是 mandatory and unskippable，失败直接拒绝打包 ——
  linking espeak-ng turns the shipped binary into a GPL combined work, and the failure prints nothing at all.
  （vendor 的 build script 已经拒绝链接非钉死的 TTS-off lib 集；这一步检查的是**真正出来的产物**。）
  改依赖（sherpa-onnx / 出片二进制）前先看 `THIRD-PARTY-NOTICES.md`。
- ⚠️ **`scan` 的标记刻意不是裸 `espeak`**：sherpa-onnx 的说话人分离里有个类叫 `OfflineSpeaker`
  （含 "eSpeaker"）与 `discreteSpeaker`，不区分大小写搜 "espeak" 会命中它们 ——
  而那正是真实二进制里保留的东西。要看那个误导性计数只能显式加 `--show-bare-espeak`，
  **它永远不影响 exit code**。
- `scan` 的 **exit code 语义**：`0` = clean，`1` = 找到标记（并逐个文件列出命中数），
  `3` = **什么都没扫到**（路径不存在 / 读不了 / 目录里没有 exe/dll）。最后一条本身也是门禁失败 ——
  "nothing to scan" 通常意味着构建根本没跑过。未知子命令与未知 flag 是 `2`。
- `SKIP_DIRS = ["build", "deps"]`：它们不是 release payload，而且带着 cargo 自己的 string tables
  （`sherpa-onnx-sys` 的 build-script 二进制把 lib 名清单逐字嵌进去），扫它们会产生假阳性。
  同理**必须跳过本工具自己的二进制**（`is_self_artifact`：文件名 `xtask`）—— 它把 5 个 `MARKERS`
  逐字嵌在自己的字符串表里，扫 `target/release/xtask.exe` 必然假阳性（实测 11 命中）；
  而它是 dev 工具、不是 payload。跳过的文件会打一行 `skip: <路径>`，不会被静默吞掉。
  计数用**不重叠**出现次数（与它取代的 PowerShell 扫描器同语义）。
- ⚠️ **版本同源**：版本只从**一处**读 —— 根 `Cargo.toml` 的 `[workspace.package] version`；
  找不到才回退 `crates/yekitrim-cli/Cargo.toml` 的 `[package]`（保住老 checkout）。
  为什么 root-first：各 crate 现在写 `version.workspace = true`，旧的逐 crate 扫描找不到任何东西，
  打包会以 "no version found" 失败。这个版本同时决定发行包名 `yekitrim-v<版本>-win-x64` 与二进制的
  `CARGO_PKG_VERSION`，**必须同源**。
  `table_version` 刻意**不认** `version.workspace = true`：继承来的不是字面量，
  静默把 `"true"` 当版本号比继续往下找更糟。
- ⚠️ **`webui/dist` 缺失即拒绝打包**（`webui/dist/index.html` 不是文件 → Err "run 'npm --prefix webui run build' first"）：
  `yekitrim server` 只服务 `webui/dist`，陈旧的 dist 会端出陈旧界面。`webui/src` 比 `dist` 新**只 warn**
  （时间戳不可靠，不拿它当门禁）。
- ⚠️ **`LICENSE` 与 `THIRD-PARTY-NOTICES.md` 是硬要求，不是 best-effort**：缺任一个直接 Err。
  二进制静态链接的第三方代码几乎全是 MIT / Apache-2.0 / BSD / ISC / Zlib，每一种都要求版权声明与许可文本
  随二进制分发，Apache-2.0 还额外要求许可文本与 NOTICE 文件 —— "什么都不带"是唯一确定错误的选项。
- ⚠️ **发行包仍然不带任何字体**：只建空的 `resources/fonts/` + 一份 `放这里.txt` 说明。
  原先随包的兜底字体授权不允许嵌入式用途（见 README「字体」），**别把 ttf 塞回发布包**。
- ⚠️ **刻意不依赖 python / PowerShell**：整个 crate 只用 std + `sha2` + `zip`。
  所以**仓库任务不许往 xtask 里塞脚本调用**（那会把刚去掉的那层依赖又加回来）。
- `zip = { version = "2", default-features = false, features = ["deflate", "time"] }`：
  默认 feature 集还会拉 zstd / bzip2 / lzma / aes，等于为了写一个 zip 去编译四个 vendored C 工具链 ——
  而"不依赖外部工具"正是这个 helper 存在的全部理由。`time` 保留真实 mtime。
- `repo_root()` 靠 `CARGO_MANIFEST_DIR` 往上两级（`crates/xtask` 必须在仓库根下两层），
  **挪动这个 crate 的深度会直接 panic**。`cargo()` 优先用 `CARGO` 环境变量。
- ⚠️ 分发前检查 `release/` 里那份**解包目录**（不是 zip）：跑过的实例会在里面留下 `.yekitrim/`
  （`server.toml` 含真实绝对路径）。`cargo xtask package --smoke-test` 会清掉它 —— 这是这条开关的主要用途之一。
- ⚠️ **验证发行行为要拿 `release/` 里的 exe，别拿 `cargo build --release`（工作区级）的那份**：
  `package` 建的是 `-p yekitrim-cli`（feature 统一域更窄），两者**字节不同**（实测 sha256 不一样），
  拿错等于验证了一个不发的二进制。`--skip-build` 复跑只重组装与 zip，不会换掉这一份。
- ⚠️ **`<zip>.sha256.txt` 由 `package` 每次重新生成**（2026-10-01 修）：以前它只手工写，
  于是 `yekitrim-v0.2.1-win-x64.zip.sha256.txt` 从 2026-09-22 起就没更新过、而 zip 换过好几版 ——
  **一个撒谎的校验和比没有更糟**（用户照它校验会以为下载坏了）。现在 `write_checksum_sidecar`
  跟着 zip 一起写（`built` 那行用 `fmt_utc_date` 打 UTC 日期：本 crate 刻意不带 chrono，
  用 Howard Hinnant 的 `civil_from_days`）。⚠️ 手工改了 zip 就必须重跑 `package`（或删掉侧车文件）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo check -p xtask
```

改完跑编译。真验证门禁/打包要跑 `cargo xtask scan`（0 = clean）与 `cargo xtask package --smoke-test`
（会重建 release、跑门禁、组装、起一次打包好的 exe 验 `/api/health` 与 WebUI）。
