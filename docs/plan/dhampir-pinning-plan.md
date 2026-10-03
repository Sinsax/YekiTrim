# YekiTrim 侧：Dhampir 依赖钉固 —— 实施计划

> 前置已就绪：底座已开源于 <https://github.com/Sinsax/Dhampir>，首个 Release **v0.1.0** 已发。
> 本文是**执行清单**（可逐条勾），背景与取舍见 [dhampir-dependency-pinning.md](dhampir-dependency-pinning.md)。

## 0. 已经确定的两个值（后面所有步骤都用它们）

```
url    = https://github.com/Sinsax/Dhampir/releases/download/v0.1.0/dhampir-0.1.0-win32-x64.zip
sha256 = 81e4f603c2df9cd26b1095dfaee873a9123b2481b35cf4a97357fa3c59cbe855
```

sha256 已由**下载回来重算**验证（与侧车、与本地构建三方逐字相同）。

---

## 1. 为什么要做（一句话）

YekiTrim 现在**指着同层 `../Dhampir` 的源码树**拿底座。要改成「从 Release 拿定版产物」，
必须先有一个**可判定的版本契约** —— 否则错版底座不会在启动/打包时被拦，
只会在出片时由底座自己报 `unsupported_project_schema`：**报得清楚，但报得晚**。

---

## 2. 改动清单（按依赖顺序，4 个提交）

### 提交 1：`dhampir.lock.toml` + 校验（`crates/xtask`）

新建 **`crates/xtask/dhampir.lock.toml`** —— 底座依赖的**唯一真相**，也是下载地址的唯一来源：

```
# 底座依赖钉固。改这个文件 = 换底座版本。
# ⚠️ 换版本必须重跑出片对拍（docs/ 里的黄金帧口径），因为它决定成片画面。
[dhampir]
version = "0.1.0"
url     = "https://github.com/Sinsax/Dhampir/releases/download/v0.1.0/dhampir-0.1.0-win32-x64.zip"
sha256  = "81e4f603c2df9cd26b1095dfaee873a9123b2481b35cf4a97357fa3c59cbe855"
# 本仓调用的**契约下限**（不是期望值 —— 见 §4）
min_project_schema = 1
min_host_api       = 6
```

解析器**手写 20 行**，不引 TOML 依赖 —— 与本 crate「只用 std + sha2 + zip」的纪律一致
（`crates/xtask/AGENTS.md` 明确写了「刻意不依赖 python / PowerShell」，同理不加解析库）。
认不出的行 / 缺键 → **报错**，不猜。

`PackageOptions` 加三个字段（`crates/xtask/src/main.rs:303`）：

| 新字段 | CLI | 含义 |
|---|---|---|
| `dhampir_from` | `--dhampir-from <url|zip>` | 从 Release 拉（或本地 zip）|
| `dhampir_sha256` | `--dhampir-sha256 <hex>` | 覆盖期望 sha256（**变异自证用**）|
| `dhampir_lock` | `--dhampir-lock <path>` | 换一份钉固（测试用）|

**关键**：`--dhampir-from` 解包后**复用现有的 `--dhampir-artifact` 路径**（`main.rs:494-535`），
因为包内形状已经**逐文件验过完全对齐**（`VERSION` / `bin/<exe>` / `preview/{engine.js,pkg/*}`）。
下游一行不用改 —— 只多一层「拿到 zip → 解包 → 校验」。

### 提交 2：`yekitrim-polish` 侧的契约下界

`crates/yekitrim-polish/src/render/dhampir.rs:59-62` 现在**只读不比对** `project_schema`，
理由是「在 YekiTrim 里再抄一份期望值 = 给同一条规矩留第二处实现」—— **这个理由成立，不动它**。

要补的是**下界**（`!=` 换成 `<`）：

```
/// 本仓调用的底座契约**下限**。**不是**期望值 —— 底座可以更新，但不能更旧。
/// 比 `!=` 宽松是刻意的：底座加端点/加字段属兼容变更，不该被旧的宿主拦下。
pub const MIN_PROJECT_SCHEMA: u32 = 1;
pub const MIN_HOST_API: u32 = 6;
```

读 `VERSION` 时：`schema < MIN` → **进 `problems`**（现在只是 `tracing_like_log` 一行）。
这样它会出现在 `/api/health.dhampir_ready`、`yekitrim polish doctor`、前端提示**同一处**
（`readiness()` 是唯一判断 —— 卡里强调过「别各写一份」）。

⚠️ **下界值必须与 `dhampir.lock.toml` 同源**，否则又是一份会漂的手抄。
做法：`yekitrim-polish` 是库、**不能读 `crates/xtask/` 的运行时文件**（依赖方向），
所以常量写在 `dhampir.rs`，由 **`xtask` 侧的守卫反向核对 lock 文件**（见提交 4）。

### 提交 3：`VERSION` 加 `version=` 的消费

底座 v0.1.0 起 `VERSION` 第一行是 `version=0.1.0`。YekiTrim 侧：

- `readiness()` 的 `contract_version` 字段**保持原语义**（`project_schema`），另加一个
  `dhampir_version` 显示用；
- `dhampir-status` / `doctor` 打印它 —— 排查「浏览器里跑的是不是修好的那份」时这是第一手信息
  （底座那条腿为此专门加了 `DHAMPIR_GIT_SHA` 注入，同一个目的）。

### 提交 4：守卫

`scripts/check-dhampir-wiring.mjs` 现在读 `../Dhampir/crates/dhampir-wasm/www/pkg/dhampir_wasm.js`
（**源码树**）。钉固之后应当改成读**钉固的那份**，并在找不到同层 Dhampir 时**跳过**
（有 `dhampir.lock.toml` 就不该再要求有源码树）。

新增三条守卫（都在 `scripts/`，都可 `--self-test`）：

1. **lock ↔ 常量一致性**：`dhampir.lock.toml` 的 `min_*` 必须等于 `dhampir.rs` 的两个常量
   —— 防「改了 lock 忘了改代码」。
2. **lock 的 url 与 version 自洽**：URL 里必须含 `v<version>` 与 `dhampir-<version>-`。
   —— 防手抄 URL 时版本号对不上；这类错要到下载时才炸。
3. **sha256 形状**：64 位小写 hex。

---

## 3. 验收（**这段是重点，不可省**）

```bash
# ① 正常路径：不依赖同层 ../Dhampir，从 Release 拉并打包
cargo xtask package --skip-build --dhampir-from <url> --smoke-test

# ② ★变异自证：给错 sha256 —— 必须拒绝
cargo xtask package --skip-build --dhampir-from <url> \
  --dhampir-sha256 0000000000000000000000000000000000000000000000000000000000000000

# ③ ★变异自证：把 lock 的 min_project_schema 改成 99 —— 必须拒绝

# ④ 收口：包内 dhampir/ 与 Release zip 的业务文件逐一对上
```

⚠️ **②③ 必须真的红，不许「永远绿」**。这是整套东西最危险的失效模式 ——
守卫恒真比没有守卫更坏（它会让人以为验过了）。底座 CI 的注释里也写着这句。

做法：先故意把 sha256 改一个字符、**真跑一次**、看到它拒绝，再改回来。

---

## 4. 为什么是「下界」而不是「期望值」（最容易改错的一条）

| | 语义 | 后果 |
|---|---|---|
| **期望值**（`!=`）| 「我猜你**就是** X」 | 底座每次兼容变更都要宿主跟着改 → 两条线被绑死 |
| **下界**（`<`）| 「我**至少**要 X」 | 底座加东西宿主自动接受；底座**删/改**才失败 |

产物名里那个 `0.1.0` 是**产品版本**（发布节奏），`project_schema` / `host_api` 是**兼容性承诺**。
钉固钉的是 **zip 的字节**，所以哪怕只有实现修复（契约不动）产品版本也要动 ——
但宿主**不该**因此失败，这就是下界的意义。

---

## 5. 明确不做

1. **不引新依赖**：`zip`（deflate only）与 `sha2` 已在 `crates/xtask/Cargo.toml`。
2. **不用 git submodule**：底座产物含 wasm + exe，submodule 只给源码、**拿不到产物**，
   那每个人都要装整套 Rust wasm 工具链才能出片，与「解压即用」冲突。
3. **不把 sha256 校验做成可选开关**：能跳过校验的钉固等于没有钉固。
   （`--dhampir-artifact <本地目录>` 是**开发路径**，语义本就不同，不在此列。）
4. **不动 `find_binary()` 的本地回退链**（`dhampir.rs:125-210`）：发布包那份优先，
   开发树的 `../Dhampir/target/{release,debug}` 继续保留 —— 开发时不该强制拉 Release。

---

## 6. 风险与回退

| 风险 | 应对 |
|---|---|
| GitHub 不可达 / Release 被删 | `--dhampir-artifact <dir>` 仍可用；CI 可缓存 zip |
| 底座发新版破坏出片 | 钉固 = **不自动升级**；换版本是一个显式提交，必须重跑对拍 |
| 将来要支持多平台 | lock 改成 `[dhampir.win32-x64]` 多节；现在只有一个平台，**不提前抽象** |
