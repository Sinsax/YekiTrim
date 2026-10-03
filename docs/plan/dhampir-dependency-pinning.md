# Dhampir 依赖钉固方案（v0.3.2 起）

> 状态：**待 GitHub 建仓后落地**。本文是执行清单，不是已完成的事实。
> 前置：`F:\para\Code\dhampir` 已上传 GitHub 并发布首个带产物的 Release。

## 0. 为什么不是「拉最新」

底座现在是 `0.0.1`，产物版本串 `dhampir-1+<sha>` 里**只有 git sha，没有语义化版本**。
YekiTrim 侧**只读** `project_schema` 不比对（`render/dhampir.rs:59-62` 明确写了「不抄第二份期望值」——
这个决定本身是对的，避免双实现）。两条合起来的效果是：

**错版底座不会在启动/打包时被拦，只会在出片时由底座自己报 `unsupported_project_schema`。**
报得清楚，但报得晚 —— 用户已经点了「导出」才发现。

所以「从 Release 拉」这条路要成立，必须先有一个**可判定的版本契约**。

## 1. 底座侧（做一次）

### 1.1 产物版本串补两个字段

`scripts/package.mjs` 写出的 `VERSION` 现在有：

```
project_schema=1
host_api=6
license=Apache-2.0
git=2bd78d5
built_at=2026-10-02T18-15-12
platform=win32-x64
```

补两条：

| 字段 | 值 | 用途 |
|---|---|---|
| `version` | `0.4.0`（语义化） | 人读 + Release tag 对齐 |
| `zip_sha256` | 产物 zip 自己的 sha256 | **让「拉取」可被验证**，而不是只靠 HTTPS |

⚠️ `zip_sha256` 只能**先打包再回填**（自指），所以 `package.mjs` 要写成：生成 zip → 算 sha →
把 sha 写进 `VERSION` → 重新生成 zip。或者干脆**把 sha 放在 release 页面与侧车文件里**
（`dhampir-<ver>.zip.sha256.txt`），`VERSION` 只留 `version` —— 后者更简单，也跟 YekiTrim 自己的
`<zip>.sha256.txt` 约定一致。**推荐后者。**

### 1.2 打 tag 并发 Release

```bash
git tag -a v0.4.0 -m 'dhampir v0.4.0'
git push origin main --tags
# 然后挂 dhampir-0.4.0-win-x64.zip + .sha256.txt
```

⚠️ **改名**：现在是 `dhampir-1+<sha>`（`1` 是 schema 版本，不是产品版本）。
Release 资产建议叫 `dhampir-<semver>-<platform>.zip`，与 YekiTrim 的 `yekitrim-v<semver>-win-x64.zip` 对齐。

## 2. YekiTrim 侧（三处改动）

### 2.1 `xtask`：新增 `--dhampir-from <url|zip>`

当前 `crates/xtask/src/main.rs:491-537` 的逻辑是：

```
--dhampir-artifact <dir>            ← 已有（解包好的目录）
  ?? 同层 ../Dhampir/dist/dhampir-*  ← 已有（开发机）
  ?? 同层 ../Dhampir/out             ← 已有（老布局）
  ?? Err(no sibling Dhampir checkout)
```

**加一条**：`--dhampir-from <zip>` = 解包到 `<temp>` 后走同一条 `--dhampir-artifact` 路径。
因为形状已经逐文件对齐（`VERSION` / `bin/<exe>` / `preview/{engine.js,pkg/*}`），
**这一步只多一个解包，不动下游任何代码**。

必带校验（缺一不可）：
1. `zip` 的 sha256 == 期望值（来自侧车或 `--dhampir-sha256 <hex>` 参数）；
2. 解包后 `verify_artifact_dir` 那四项必需文件齐（已有逻辑，复用）；
3. `VERSION` 里的 `project_schema` **在 YekiTrim 支持范围内**（见 2.2）。

### 2.2 把「不比对」改成「声明下界」

现在 YekiTrim 完全不比对 `project_schema`。**这个不改，但补一条下界声明**：

`crates/yekitrim-polish/src/render/dhampir.rs` 加一个常量，语义是「**至少**要这么新」：

```rust
/// 本仓调用的底座契约下限。**不是**期望值 —— 底座可以更新，但不能更旧。
/// 比 `!=` 宽松是刻意的：底座加端点/加字段属兼容变更，不该被旧的宿主拦下。
const MIN_PROJECT_SCHEMA: u32 = 1;
```

读 `VERSION` 时：`schema < MIN_PROJECT_SCHEMA` → **报错并给出升级指引**（现在只是 `info` 一行）。
`host_api` 同理给一个下界。

⚠️ **仍然不引入「期望值」** —— 那会变成第二份实现（原注释的顾虑成立）。
下界与期望值是两回事：下界是「我依赖哪些能力」，期望值是「我猜你会是什么版本」。

### 2.3 钉固写在哪

新增 `crates/yekitrim-polish/src/render/AGENTS.md` 之外的一处**唯一真相**，例如
`crates/xtask/dhampir.lock.toml`：

```toml
# 底座依赖钉固 —— 唯一真相。改它 = 换底座版本，必须重跑出片对拍。
version       = "0.4.0"
url           = "https://github.com/<你>/dhampir/releases/download/v0.4.0/dhampir-0.4.0-win-x64.zip"
sha256        = "<64 hex>"
project_schema = 1
host_api       = 6
```

它同时是**下载地址的唯一来源** —— 你要的「下载链接确定」就落在这一行。
`xtask` 默认读它，`--dhampir-from` 可临时覆盖（覆盖时必须显式给 sha）。

## 3. 验收（落地后必须全绿）

```bash
# ① 拉取 + 校验 + 打包（不依赖同层 ../Dhampir）
cargo xtask package --skip-build --dhampir-from <url> --smoke-test

# ② 故意给错 sha → 必须拒绝（变异自证，证明校验不是装饰）
cargo xtask package --skip-build --dhampir-from <url> --dhampir-sha256 0000...

# ③ 故意给低 schema → 必须拒绝

# ④ 收口：包内 dhampir/ 与底座 zip 的业务文件逐一对上
```

⚠️ **②③ 必须真的红**。守卫「永远绿」是这套东西最危险的失效模式（底座 CI 注释里就写着这句）。

## 4. 还没做的两件事（不在本次范围）

1. **wasm 陈旧守卫**：`check-web-invariants.mjs` / `check-dual-end.mjs` 现在红，
   原因是 `crates/dhampir-wasm/www/pkg` 比源码旧，需要 `wasm-pack build ... --dev` 重建一次。
   这是**发布前一次性动作**，不是代码问题（`wasm-pack 0.15.0` 已装）。
2. **回填 `Cargo.toml` 的 `repository`**：五个 crate 都继承 `[workspace.package]` 那一处。

## 5. 为什么不用 git submodule

考虑过，不选：

- 底座产物里有 **wasm 与 exe**，都要 `wasm-pack` + Rust release 构建；submodule 只会拿到源码，
  **拿不到产物** —— 每个用 YekiTrim 的人都要装整套 Rust wasm 工具链才能出片，与「解压即用」冲突。
- `dist.zip` 是**已验收的定版产物**（`VERSION` 带 git sha），submodule 拿到的是移动的 `main`。
- 钉 sha256 + 语义化 tag 的 Release，比 submodule 的「某个 commit」**更可验证**。