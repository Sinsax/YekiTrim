# sherpa-onnx TTS-off 替换：完成汇报与切换指南

> 对应计划：[sherpa-onnx-prebuilt-no-tts.md](sherpa-onnx-prebuilt-no-tts.md)（§10 任务清单已全部完成）
> 产物仓库：<https://github.com/Sinsax/sherpa-onnx-prebuilt-no-tts>
> 本文是给 YekiTrim 侧的参考与操作指导。

---

## 一、结论先行

> **状态更新（2026-09-18）**：本节描述的"切换"已落地，且用户已裁定以 **Apache-2.0** 发布
> （原为 GPL-3.0-or-later）。实现方式见 §12，许可变更的逐项清单见 §9.5。

上游官方预编译静态包**无条件链接 espeak-ng（GPL-3.0-or-later）**，而 YekiTrim 只做 ASR、
从不调用 TTS —— 这就是本项目**当时**必须以 GPL-3.0-or-later 发布的**唯一原因**
（见 [THIRD-PARTY-NOTICES.md](../../THIRD-PARTY-NOTICES.md) 第一节）。

替换用的 TTS-off 归档**已经构建、验证并发布**，文件名与官方**完全一致**，属于 drop-in
顶替，`sherpa-onnx-sys` 一行代码都不用改。**切换之后，把 YekiTrim 拉进 GPL 的那个动因消失。**

| | 官方归档 | 本项目归档 |
|---|---|---|
| espeak-ng | 静态链入 | **不编译、不链接** |
| 消费方二进制里的 espeak-ng 特有标记 | **60 处** | **0 处** |
| 许可动因 | GPL-3.0-or-later | 无 |

---

## 二、可用产物

| 项 | 值 |
|---|---|
| Release | <https://github.com/Sinsax/sherpa-onnx-prebuilt-no-tts/releases/tag/v1.13.3> |
| 归档名 | `sherpa-onnx-v1.13.3-win-x64-static-MT-Release-lib.tar.bz2` |
| sha256 | `94d733a9cda2d959b1f924d75f771154a5acc4c8241b4393967afdb725e23b1f` |
| 大小 | 111,446,628 字节（106.3 MiB） |
| 顶层目录 | `sherpa-onnx-v1.13.3-win-x64-static-MT-Release-lib/`（= 归档名去 `.tar.bz2`） |
| 内容 | `lib/` 下 14 个 `.lib` |
| 上游 | sherpa-onnx v1.13.3，commit `330609dab49be6ee8b30702918ca7abbbad1286a` |
| 工具链 | MSVC，静态 CRT `/MT`，Release，`BUILD_SHARED_LIBS=OFF` |

归档内含 `espeak-ng.lib` / `piper_phonemize.lib` / `ucd.lib` 三个文件，但它们是
**896 字节、零符号的占位空库** —— 因为 `sherpa-onnx-sys` 的链接清单里写死了这三个名字
（不是 whole-archive，逐项 `rustc-link-lib=static=`，文件不存在就链接失败）。
**它们不含任何 espeak-ng 代码。**

---

## 三、切换步骤

> ⚠️ **本节描述的是 2026-09-18 一次性手工切换的做法，现已被 §12 取代。** 现在归档由 vendored 的
> `sherpa-onnx-sys/build.rs` 自动取回并校验 sha256，**裸 `cargo build` 就是安全的**，不需要复制
> 归档、也不需要设任何环境变量。本节保留，因为 §3.3 那个「缓存短路」的坑与 §3.4 的版本钉死
> 约束**依然成立**，离线构建时也用得上。

### 3.1 获取归档

放到任意目录，例如 `<项目根>/.sherpa-libs/`。**不要提交进 git** —— 106 MiB 超过 GitHub
单文件 100 MiB 的硬限制，`git push` 会被直接拒收。

```powershell
$dir = "$PSScriptRoot\..\.sherpa-libs"      # 按实际调整
New-Item -ItemType Directory -Force $dir | Out-Null
$name = "sherpa-onnx-v1.13.3-win-x64-static-MT-Release-lib.tar.bz2"
Invoke-WebRequest -Uri "https://github.com/Sinsax/sherpa-onnx-prebuilt-no-tts/releases/download/v1.13.3/$name" -OutFile "$dir\$name"

# 必须校验：确认拿到的是我们门禁通过的那一份
$sha = (Get-FileHash -Algorithm SHA256 "$dir\$name").Hash.ToLower()
if ($sha -ne "94d733a9cda2d959b1f924d75f771154a5acc4c8241b4393967afdb725e23b1f") {
  throw "sha256 不匹配，实际 $sha"
}
```

### 3.2 设置环境变量

```powershell
$env:SHERPA_ONNX_ARCHIVE_DIR = "<项目根>\.sherpa-libs"
cargo build --release
```

### 3.3 ⚠️ 最大的坑：crate 的缓存短路

`sherpa-onnx-sys/build.rs` 解析库目录的**优先级顺序**是这样（已核对源码）：

```
1. SHERPA_ONNX_LIB_DIR          ← 最高，绕过一切
2. target/sherpa-onnx-prebuilt/<归档名去扩展名>/lib/   ← 缓存，存在就直接返回
3. SHERPA_ONNX_ARCHIVE_DIR      ← 排在缓存之后！
4. 从 GitHub 下载官方归档
```

**第 2 条排在 `SHERPA_ONNX_ARCHIVE_DIR` 之前。** 也就是说：任何**曾经用官方归档构建过**
的机器上，`target/sherpa-onnx-prebuilt/sherpa-onnx-v1.13.3-win-x64-static-MT-Release-lib/lib/`
已经存在，此时你就算设了 `SHERPA_ONNX_ARCHIVE_DIR`，**它也会被静默忽略**，继续用旧的
（带 espeak-ng 的）官方库，而且**不会有任何报错**。

所以切换时必须二选一：

```powershell
# 方案 A：删缓存后再设 ARCHIVE_DIR（推荐）
Remove-Item -Recurse -Force target\sherpa-onnx-prebuilt
$env:SHERPA_ONNX_ARCHIVE_DIR = "<项目根>\.sherpa-libs"
cargo build --release

# 方案 B：直接指 lib 目录（优先级最高，天然绕过缓存）
#   需要先把归档解压出来
tar -xjf .sherpa-libs\sherpa-onnx-...-lib.tar.bz2 -C .sherpa-libs
$env:SHERPA_ONNX_LIB_DIR = "<项目根>\.sherpa-libs\sherpa-onnx-v1.13.3-win-x64-static-MT-Release-lib\lib"
cargo build --release
```

方案 B 的代价是**不校验归档的文件名/结构契约**（它跳过归档直接读 lib 目录）。想同时验证契约
就用方案 A。

### 3.4 版本必须钉死在 1.13.3

归档名里嵌的是 **`sherpa-onnx-sys` 的版本号**，不是 `sherpa-onnx` 的。而 `sherpa-onnx 1.13.3`
对 sys 用的是插入符依赖 `"1.13.3"`：

- **当前没问题** —— `Cargo.lock` 里 `sherpa-onnx-sys` 正是 `1.13.3`；
- **但要注意**：一旦跑 `cargo update`，sys 会被解析到更新的 1.13.x（实测是 `1.13.6`），
  归档名就对不上，报错是：

  ```
  SHERPA_ONNX_ARCHIVE_DIR does not contain expected archive:
    .../sherpa-onnx-v1.13.6-win-x64-static-MT-Release-lib.tar.bz2
  ```

  这个报错很清晰、不会静默用错库。届时的处置：要么在
  `crates/yekitrim-core/Cargo.toml` 里显式钉 `sherpa-onnx-sys = "=1.13.3"`，
  要么等本项目发布对应 sys 版本的归档。

---

## 四、验证（必做，不可省）

### 4.1 用现成扫描器扫产物

本项目自带扫描器（`crates/xtask/`，经 `.cargo/config.toml` 的 cargo alias 调用），
直接对 YekiTrim 的产物跑 —— **不再需要任何脚本文件**：

```bash
cargo xtask scan                 # 默认扫 target/release
cargo xtask scan target/debug    # 指定目录
# 退出码 0 = 干净；1 = 检出 espeak-ng；3 = 没扫到任何东西（不算通过）
```

`cargo xtask package --smoke-test` 在打包前会自动跑这道门禁，所以发布路径无需手动调它。

### 4.2 ⚠️ 不要用裸子串 `espeak` 做判据

这条很重要，**否则你会得到假失败**：sherpa-onnx 的说话人分离代码里有个类叫
`OfflineSpeaker*`，而 `OfflineSpeaker` 恰好含子串 `eSpeaker`，大小写不敏感搜 `espeak`
会命中它。

实测对照（同一份消费方代码，只换归档）：

| 归档 | 二进制里裸 `espeak` | espeak-ng 特有标记 |
|---|---|---|
| 官方（带 TTS） | 65 | `espeak-ng`=7, `espeak ng`=2, `espeak_`=51 |
| 本项目（TTS-off） | **0** | **0** |

判据要用**特有标记**：`espeak-ng`、`espeak ng`、`espeak_`、`libespeak`、`piper_phonemize`。

> 顺带说明：计划 §5.4 原本写的 `findstr /i /c:"espeak"` 就是这个假失败源，本项目已修正。

---

## 五、许可影响（切换后 YekiTrim 侧要改什么）

### 5.1 动因消失

espeak-ng 不再被链接后，**把 YekiTrim 拉进 GPL-3.0-or-later 的那个组件就不存在了**。
理论上可以把 `LICENSE` 与各 crate 的 `Cargo.toml` 换成宽松许可。

### 5.2 但有三件事必须先处理

1. **已发布的 GPL 版本收不回来。** GPL 授予接收者的权利是永久、不可撤销的；旧版本对已获得
   它的人仍然有效。换许可只对**新版本**生效。
   → **本项目不适用**：YekiTrim 从未向第三方分发过二进制（`v0.1.0` 仅为内部测试版），
   GPL 义务从未触发，所以本次是一次性彻底切换。
2. **有外部贡献者的话需要全体版权人同意。** 纯自己作品则无此问题。
3. **先做一次完整依赖审计，别只信手写的 THIRD-PARTY-NOTICES。** 跑
   `cargo deny check licenses`（或 `cargo license`）把整棵依赖树过一遍，确认 espeak-ng
   之外没有第二个 GPL/LGPL 来源（包括 build 依赖）。当前那份清单是手工维护的，且已经**过时**。

### 5.3 `THIRD-PARTY-NOTICES.md` 需要改的地方

| 位置 | 现状 | 应改为 |
|---|---|---|
| 第一节 `espeak-ng` 行 | 标注 GPL-3.0-or-later，「这是本项目采用 GPL-3.0 的原因」 | 移除，或标注「**已不再链接**（改用 TTS-off 归档）」 |
| 第一节 `piper_phonemize / … / ucd` 行 | 一并列为随包分发 | `piper_phonemize`、`ucd` 改为「不再编译」；`kaldi-*`、`kissfft`、`ssentencepiece_core` 保留 |
| 第一节警告段（约 22–23 行） | 「该预编译静态包内不含任何第三方许可文件…」 | 该描述**对新归档同样成立**（新归档也不含许可文件），但要补上：新归档的实际组件清单与声明全文见本项目 `THIRD-PARTY-NOTICES.md` |
| 新增 | — | 加一行 **Eigen（MPL-2.0）**，见 5.4 |

### 5.4 仍须履行的义务（切换后不是零义务）

新归档编译进去的组件许可是：Apache-2.0（sherpa-onnx、openfst、kaldi-*、kaldifst、
simple-sentencepiece）、MIT（ONNX Runtime、nlohmann/json）、BSD-3-Clause（kissfft、
websocketpp）、BSD-2-Clause（fastcluster）、Boost-1.0（Asio）、MPL-2.0（Eigen）。
**没有任何 GPL / LGPL / AGPL。**

需要随二进制带上声明。两个具体点：

1. **ONNX Runtime 的预编译包里没有许可文件**（只有 `include/` 和 `lib/`），它的 MIT 声明
   必须由分发方自行补上 —— 本项目 `THIRD-PARTY-NOTICES.md` 已转载，可直接引用。
2. **Eigen 是这条链上唯一带「提供源码」义务的组件（MPL-2.0）**。要点是：这项义务的对象是
   **Eigen 自己的源码**（公开可得，<https://gitlab.com/libeigen/eigen>），**不是 YekiTrim 的源码**；
   MPL-2.0 §3.3 也明确允许把它作为 Larger Work 以不同条款分发。**所以它不妨碍闭源分发**。

> 以上是基于许可证原文与实测依赖清单的分析，不构成法律意见。

---

## 六、CI / 可复现性建议

- 归档**不能进 git**（>100 MiB 会被 GitHub 拒收）。建议写一个 `scripts/fetch-sherpa-libs.ps1`：
  从 Release 下载 → 校验 sha256（硬编码上表那个值）→ 解压到 `.sherpa-libs/`；
  并在 CI 与本地构建中都先跑它。
- sha256 写死在脚本里，等于把「用的是哪一份库」固定下来；上游升版本时同步更新。
- 构建后**必须**跑 `scan-artifact.ps1` 作为发布门禁。这一条防的是「忘了设环境变量 /
  缓存短路，静默回退官方归档」—— 那种失败不会有任何报错，只有扫产物才能发现。

---

## 七、回退

出问题时退回官方归档：删掉 `SHERPA_ONNX_ARCHIVE_DIR` / `SHERPA_ONNX_LIB_DIR`，
再 `Remove-Item -Recurse -Force target\sherpa-onnx-prebuilt`，cargo 会自动重新下载官方归档。
**注意这会同时把 GPL 义务带回来**，THIRD-PARTY-NOTICES 也得跟着回退。

---

## 八、遗留

- 本项目仓的 **CI 工作流**（`workflow_dispatch`）在计划里标为可选，**尚未实现**。
  在实现之前，版本升级需要本机跑 `cargo build --release -p yekitrim-cli --locked` →
  `cargo xtask package`（原 `scripts/build.ps1` / `scripts/publish.ps1` 写法见 §12）。
- 上游升版本时的流程见本项目 README 第 6 节。

---

## 九、YekiTrim 侧执行记录（2026-09-18）

已按本文档完成切换。本节只记录**实际落盘的东西与实测数字**，供后续改文档/许可时直接引用。

### 9.1 落盘

| 路径 | 动作 | 说明 |
|---|---|---|
| `.cargo/config.toml` | 新增 | `[env] SHERPA_ONNX_ARCHIVE_DIR = { value = ".sherpa-libs", relative = true }` |
| `scripts/fetch-sherpa-libs.ps1` | 新增 | 下载 + sha256 硬校验 + 可选解压（解压器级联 + 占位库闸门） |
| `scripts/scan-artifact.ps1` | 新增 | espeak-ng 特有标记扫描；退出码 0=干净 / 1=检出 / 3=未扫到文件 |
| `scripts/package-release.ps1` | 修改 | 构建前先跑 fetch；构建后跑 scan 门禁（**无跳过开关**） |
| `.gitignore` | 修改 | 忽略 `/.sherpa-libs/`（归档）与 `/sherpa-onnx-prebuilt-no-tts/`（旧放置目录名） |
| `.sherpa-libs/` | 落位 | 归档 + 可选解压产物；未入库 |

### 9.2 与原指导的两处偏差（实测发现后才改的）

1. **不用临时环境变量。** §3.2 的 `$env:SHERPA_ONNX_ARCHIVE_DIR = ...` 只在当前 shell 生效 ——
   换个终端、或 CI 上另起一个步骤，就会静默退回官方归档（且不报错）。改为写进
   `.cargo/config.toml` 的 `[env]`，一次配置对**所有**构建生效。Cargo `[env]` 默认
   `force = false`，所以系统/命令行变量仍然优先 —— 临时切回官方库的口子保留了。
2. **`tar -xjf` 在本机不可用。** Windows 自带的 bsdtar 3.5.2 对 bz2 会外调 `bzip2 -d`，
   而 PATH 里没有 bzip2 → 直接失败。而 7z(NanaZip) 的 `x` 对 `.tar.bz2` **只解外层**、
   留下一个 913 MB 的 `.tar`，**却返回退出码 0**。因此 `fetch-sherpa-libs.ps1 -Extract`
   实现为"多解压器级联（tar → 7z → python）+ **以产物判定成功**（`<stem>/lib` 存在与否）"，
   完全不看退出码。
   > `-Extract` 是**可选**的：默认构建由 `sherpa-onnx-sys/build.rs` 自己解压进
   > `target/sherpa-onnx-prebuilt/`，不依赖任何外部解压器。

### 9.3 实测数字（同机、同一份消费方代码，只换归档）

| 项 | 官方归档 | TTS-off 归档 |
|---|---|---|
| `yekitrim.exe` 里 espeak-ng 特有标记 | `espeak-ng`=7, `espeak ng`=2, `espeak_`=51（合计 **60**） | **0** |
| 裸 `espeak` 命中 | 65 | **2** |
| `espeak-ng.lib` / `piper_phonemize.lib` / `ucd.lib` | 823,674 / 800,124 / 243,106 B | **896 / 896 / 896 B（零符号占位）** |
| `sherpa-onnx-core.lib` | 66,036,138 B | 51,009,458 B |
| `sherpa-onnx-c-api.lib` | 1,699,818 B | 564,040 B |
| `onnxruntime.lib` | 816,663,584 B | 816,663,584 B（未变） |

裸 `espeak` 那 2 处正是 §4.2 说的假阳性：都落在 `discreteSpeaker`（说话人分离标识符）里。
⚠️ 注意指导举的例子是 `OfflineSpeaker`；在 YekiTrim 这份二进制里实际残留的是
**`discreteSpeaker`** —— 同类问题、判据不变，但排查时别照着 `OfflineSpeaker` 去找。

`scan-artifact.ps1` 默认排除 `build/` 与 `deps/`，这不是图省事：
`target/release/build/sherpa-onnx-sys-*/build-script-build.exe` 里嵌着该 crate 自己的
库名清单（`espeak-ng`、`piper_phonemize` 字面量各 1 处），但那是**源码字符串**、
不是链接进来的代码；`deps/` 则是 proc-macro dylib 与二进制的一份副本。
两者都不进发布包（`package-release.ps1` 只发 `target/release/yekitrim.exe`），留在门禁里
只会制造噪声。要全量扫用 `-SkipDirectory @()`。

### 9.4 验证

| 验证项 | 结果 |
|---|---|
| 归档 sha256 | 与 §2 记录一致（`94d733a9…e23b1f`，111,446,628 B） |
| 删缓存后重建 | `cargo build --release -p yekitrim-cli --locked` 成功，4m36s；缓存内三个占位库均为 896 B |
| 产物扫描（门禁） | 退出码 0，0 命中 |
| 真实 ASR | `yekitrim transcribe`（SenseVoice 单模型，30s 音频）退出码 0、2.3s，出 7 段带时间戳的中文 |
| 打包 + 烟测 | `package-release.ps1 -SkipBuild -SmokeTest` 全绿：门禁 PASS → 打包 → `/api/health ok` 且 `/` 正常返回 WebUI → zip 14.3 MB |

阳性对照也做了：切换**前**同一脚本对旧的（官方库）二进制报 `espeak-ng`=7 / `espeak ng`=2 /
`espeak_`=51、合计 60，退出码 1 —— 与 §4.2 表格逐字吻合。0 命中因此可信。

### 9.5 当时的留待项（**已于 2026-09-18 全部完成**）

> 下面这批最初是**刻意留待**、不是遗漏。用户裁定以 **Apache-2.0** 发布后已逐项处理完，
> 保留原始清单以便追溯。

| 项 | 结果 |
|---|---|
| `LICENSE` | ✅ 换为 Apache-2.0 全文（11,357 B，与 apache.org 官方文本一致） |
| 5 个 crate 的 `license`（cli / core / polish / render-core / wasm） | ✅ 全部改为 `Apache-2.0` |
| `webui/package.json` 与 `webui/src/wasm/package.json` | ✅ 改为 `Apache-2.0` |
| `crates/xtask/Cargo.toml` | ✅ **补上** `license = "Apache-2.0"`（此前完全没有 license 字段） |
| `THIRD-PARTY-NOTICES.md` §5.3 那张表 | ✅ 按该表逐项改完（含新增 Eigen 行、espeak-ng 行标为移除） |
| `README.md`「为什么是 GPL-3.0」段（549–565 行） | ✅ 改写为「为什么是 Apache-2.0（而不是 GPL）」 |
| `docs/deploy.md:129`、`AGENTS.md:1796` 与 `:1804-1805` | ✅ 已更新 |
| `cargo deny check licenses` 依赖审计 | ✅ 等价完成：用 `cargo metadata --locked` 对全树 335 包逐条统计，结论见 `THIRD-PARTY-NOTICES.md` 第一节 |

✅ **§5.2 的第 1、2 件事对本项目都不构成约束**：GPL 义务只在**分发**时触发，而 YekiTrim 从未向
第三方分发过二进制（`v0.1.0` 仅为内部测试版、未对外发布）；且 116 个 commit 全部单人作者、
无外部贡献者。因此本次是**一次性彻底切换**，无需追溯、无需他人同意。
（若将来重新许可，届时的外部贡献者才需要 CLA/DCO 前置——见 §9.5 讨论。）

### 9.6 顺带发现：`AGENTS.md` 的 `yekitrim.toml` 示例是错的

`AGENTS.md` 把 `input_file` / `danmaku_file` / `audio_file` 等写成**顶层键**，但
`Config.project` 没有 `#[serde(flatten)]`，真实布局是 **`[project]` 段**
（`yekitrim init` 生成的也是 `[project]`）。照 AGENTS.md 的示例手写会**静默丢配置**，
现象是 `Error: 未初始化工程 input_file` —— 而错误信息完全不指向配置格式。
这条留到"文档"那一步一起改。

---

## 十、归档获取接线（2026-09-18，消费方侧）

> ⚠️ **本节与 §11 描述的 PowerShell 方案已被 §12 取代**（同日）：4 个 `.ps1` 已删除，改为
> vendored 的 `sherpa-onnx-sys` + `cargo xtask`。本节保留作为实测记录 —— 里面的坑与数字仍然有效。

归档不入户（106 MiB > GitHub 单文件硬限、`git push` 会被拒收），所以"**每换一台机器都要手动
把归档复制过去**"这一步必须消掉。本节记录它现在怎么被自动取回。

### 10.1 发布源

| 项 | 值 |
|---|---|
| 仓库 | `https://github.com/Sinsax/sherpa-onnx-prebuilt-no-tts` |
| Release | `v1.13.3`（2026-09-18 发布） |
| 资产 | `sherpa-onnx-v1.13.3-win-x64-static-MT-Release-lib.tar.bz2` |
| 大小 | 111,446,628 B |
| sha256 | `94d733a9cda2d959b1f924d75f771154a5acc4c8241b4393967afdb725e23b1f` |

归档名与 sha256 都硬编码在 `scripts/fetch-sherpa-libs.ps1` 顶部（三个常量一起改：`ArchiveName`
/ `ArchiveSha256` / `ArchiveSize`）。

### 10.2 一条命令

新克隆、或换一台机器：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build.ps1
```

= 取归档（sha256 硬校验）→ `cargo build` → 断言产物无 espeak-ng。

只想补归档、不构建：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\fetch-sherpa-libs.ps1
```

`fetch` 是**幂等**的：本地已有且哈希一致就复用，只花一次 sha256（106 MiB，亚秒级），不会重复
下载。所以把它放在每次构建前跑的成本可以忽略 —— 这也是 `build.ps1` 不给"跳过 fetch"开关的原因。

### 10.3 实测

| 验证项 | 结果 |
|---|---|
| GitHub 下载分支（真实拉取） | 106.3 MB 下载成功，sha256 与 §2 记录一致，退出码 0 |
| Release 资产的 digest | GitHub 记录的 `sha256:94d733a9…` 与本地归档一致 → 同一份字节 |
| `scripts/build.ps1` 全流程 | 退出码 0，门禁 `PASS` |
| 缓存预检 + 自愈（见 10.4） | 伪造官方库缓存（三个占位库改成 823,674 / 800,124 / 243,106 B）→ 被检出 → 清缓存 → `cargo clean -p sherpa-onnx-sys --profile release` → build.rs 重跑并重新解压：三个库全部回到 **896 B**，`sherpa-onnx-core.lib` = 51,009,458 B（TTS-off 尺寸，非官方的 66,036,138 B） |

⚠️ 注意：这段**下载分支在本节之前从未被执行过**。本地归档一直在，脚本直接走"已存在"短路，
所以"这个 URL 能下载"是本次才真正跑出来的结论，不是推断的。

### 10.4 `build.ps1` 比 fetch 多做的两件事

1. **构建前的缓存预检（新增的安全网）**。若 `target/sherpa-onnx-prebuilt/**/espeak-ng.lib`
   存在且 **> 4096 B**，说明该缓存来自**官方归档**（真 `espeak-ng.lib` 是 823,674 B）。由于缓存
   优先级高于 `SHERPA_ONNX_ARCHIVE_DIR`，它会**静默覆盖**我们刚校验过的归档。此时脚本打印警告、
   删除该缓存，再 `cargo clean -p sherpa-onnx-sys --profile <profile>` 逼 build.rs 重跑，
   从 `.sherpa-libs` 重新解压。

   ⚠️ 这里有两个**只有实测才暴露出来**的坑，都已写进脚本注释：

   - **只删目录不够。** cargo 的指纹**不跟踪** build.rs 解压出来的那些库：缓存删掉后，
     `cargo build` 报 `Finished `release` profile in 0.41s` —— 既不重新链接，也不重建缓存。
     结果就是**用官方库链接出来的二进制毫发无伤地活过这次"修复"**。必须 `cargo clean -p`
     才能让 build.rs 重跑。（实测：删缓存后 exe 的 mtime 纹丝不动。）
   - **`cargo clean -p` 不带 `--profile` 会清错 profile。** 实测它删掉 108 个文件 / 2.7 GiB ——
     那是 **debug** 产物，release 一个没动，紧接着的 `--release` 构建 0.38s 空跑。
     看着像修好了，其实什么都没发生。带上 `--profile release` 才真正命中（22 个文件 / 886 MiB）。

   **代价**：一次 `sherpa-onnx-sys` 加依赖链的重建（实测约 2 分钟到链接阶段）。只在"曾经用官方
   归档构建过"的机器上触发一次。正因为代价真实存在，下面的扫描门禁**不能省** —— 万一重建被跳过，
   门禁仍然会拒绝把这份产物当成好构建。

2. **给 fetch 传 `-NoHints`**。fetch 结尾原本会打印 "next: 删缓存 → cargo build → 扫产物"，
   在 wrapper 里读起来像"什么都没做成"。

### 10.5 ⚠️ 仍未入库（这条不补上，"自动取归档"是半截的）

`.cargo/config.toml` 目前**未跟踪**（`git status` 显示 `?? .cargo/`，且它不在 `.gitignore` 里）。
后果：新克隆能拿到 `scripts/build.ps1`，却拿不到 `[env] SHERPA_ONNX_ARCHIVE_DIR` —— 于是
`build.rs` 的解析链在缓存之后**直接去 GitHub 下载官方归档**，全程无任何提示，产物重新变成
GPL 合组合作品。

```powershell
git add .cargo scripts/build.ps1 scripts/fetch-sherpa-libs.ps1 scripts/scan-artifact.ps1
```

同批入库的还有 `scripts/package-release.ps1` 与 `.gitignore` 的改动（已修改、未提交）。

> 措辞修正（见 §11.1 的源码级确认）：`.cargo/config.toml` 一旦入库，"静默去下载官方归档"这条
> 路径就**够不着了** —— `SHERPA_ONNX_ARCHIVE_DIR` 只要被设上，本地归档缺失时 build.rs 是**直接
> 报错**，不是联网下载。所以残留风险不是"静默变 GPL"，而是更窄的一条：
> **曾经用官方归档构建过**的机器上 `target/` 缓存优先级更高、build.rs 根本不会走到判断 ——
> 那条由 `build.ps1` 的预检兜住，**裸 `cargo build` 与 IDE 里点构建都兜不住**。

---

## 十一、按需取归档的边界与占用（2026-09-18 续，消费方侧）

本节回答三件事：**是谁在构建 sherpa-onnx**、**缺归档时会怎样**、**它把项目撑大了多少**。

### 11.1 本仓不构建 sherpa-onnx，也不含它的源码

`crates/yekitrim-core/Cargo.toml`：

```toml
sherpa-onnx = "1.13.3"      # crates.io 依赖
```

全仓 `Cargo.toml` 里与 sherpa 相关的**只有这一行** —— 没有 `[patch]` / `[replace]`，也没有指向
本地源码的 `path =`。所以**"构建 sherpa-onnx" 这件事只发生在隔离仓库
`Sinsax/sherpa-onnx-prebuilt-no-tts` 里**；本仓在构建期只做一件事：**按需下载那份归档**
（106 MiB，sha256 钉死，见 §10.1）。

### 11.2 缺归档时：响亮报错，**不会**去下载官方归档

`sherpa-onnx-sys-1.13.3/build.rs` 的解析链（读源码，非推断）：

| 顺序 | 条件 | 行为 |
|---|---|---|
| 1 | `SHERPA_ONNX_LIB_DIR` 有设 | 只检查**是不是目录**。不是 → `Err`；是 → 直接用（**最高优先级，绕过缓存**） |
| 2 | 缓存 `target/sherpa-onnx-prebuilt/<归档名>/lib` 存在 | **直接 return**，3、4 一个都不走 |
| 3 | `SHERPA_ONNX_ARCHIVE_DIR` 有设、但目录里没有那份归档 | `Err`：`SHERPA_ONNX_ARCHIVE_DIR does not contain expected archive: <完整路径>`。**不联网** |
| 4 | `SHERPA_ONNX_ARCHIVE_DIR` **未设** | 用 `ureq` 从 GitHub **下载官方归档**（带 espeak-ng 的那份） |

**实测**（模拟全新克隆：把 `.sherpa-libs` 移走、用独立 `CARGO_TARGET_DIR`、跑裸 `cargo build`）：

```
thread 'main' panicked at sherpa-onnx-sys-1.13.3\build.rs:40:9:
SHERPA_ONNX_ARCHIVE_DIR does not contain expected archive:
  F:\para\Code\YekiTrim\.sherpa-libs\sherpa-onnx-v1.13.3-win-x64-static-MT-Release-lib.tar.bz2
```

**9 秒**失败、退出码 101、**没有发起任何官方归档下载**。这正是想要的行为：**宁可响亮失败，
也不要静默去拉 100+ MB 的官方 GPL 归档**。（btw，失败前 cargo 会先编译掉 build.rs 的依赖树
—— `ureq` / `rustls` / `ring` / `tar` / `bzip2`，那正是"联网下载"能力所在，也说明第 4 条确实
存在、只是够不着。）验证后 `.sherpa-libs` 已还原，sha256 逐字节一致。

> 附带发现：build.rs 在判断之前先 `create_dir_all("target/<profile>/sherpa-onnx-prebuilt")`，
> 所以失败后会留下一个**空目录**。无害（下次归档到位时 `lib_dir.is_dir()` 仍为假，会正常解包），
> 只是别把它当成"缓存已就绪"。

⚠️ **残留风险只有一条**：第 2 条（缓存）的优先级高于第 3 条（归档）。在**曾经用官方归档构建过**
的机器上，缓存已经存在，build.rs 第一枪就 return，上面那个报错**根本不会发生** —— 产物静默恢复
GPL。这条由 `build.ps1` 的构建前预检兜住（§10.4）；**裸 `cargo build` 与 IDE 里点构建都兜不住**。
日常构建请走 `scripts/build.ps1`。

### 11.3 占用基线

| 位置 | 大小 | 性质 |
|---|---|---|
| `.sherpa-libs/<归档>.tar.bz2` | **106.3 MB** | 仓库内、gitignored。**项目目录唯一的固定新增占用** |
| `.sherpa-libs/<stem>/`（`-Extract` 的产物） | **约 871 MB** | **正常构建完全不读**；只为 §3.3 option B 的 `SHERPA_ONNX_LIB_DIR` 而存在 |
| `target/sherpa-onnx-prebuilt/` | **约 977 MB** | build.rs 自己解包出的构建缓存，落在 `target/` 内，属常规构建产物 |

代价集中在单个成员身上：`onnxruntime.lib` = **816,663,584 B**。

**本次清理**：上一轮验证 `-Extract` 留下的那份解包没被删，`.sherpa-libs/` 一度是 **977.1 MB**
（比归档本身体积大 9 倍）—— 正是本文档要避免的那种"项目里的隐性臃肿"。已删除该解包目录
（**870.8 MB**），`.sherpa-libs/` 回到 **106.3 MB**，归档 sha256 复校一致。

**并加了防回潮的告警**（`scripts/fetch-sherpa-libs.ps1`）：

- 非 `-Extract` 路径下若发现 `.sherpa-libs/<stem>/` 还在 → 打印它的实际体积 + 删除命令
- `-Extract` 路径 → 明确提示这 ~871 MB 会留在盘上、正常构建不读它

实测：伪造 5 MB 残留 → 告警如期触发，退出码仍为 0。

### 11.4 一句话

归档**只下载、不入库、不本地编译**。新机器一条命令（§10.2）即可；项目目录的固定代价是
**106 MB**，其余占用都在 `target/` 里。

> ⚠️ 本节（含 §10）描述的 **PowerShell wrapper 方案已于同日被 §12 取代**：`build.ps1`、
> `fetch-sherpa-libs.ps1`、`scan-artifact.ps1` 已删除，`.sherpa-libs/` 已清空，
> 项目目录的固定代价从 106 MB 变成 **0**。本节保留，因为其中两个实测坑（§10.4 的"只删缓存不够"、
> "`cargo clean -p` 不带 `--profile` 会清错 profile"）与 §11.2 的解析链表格仍然有效。

---

## 十二、改用 vendored fork：让 `cargo build` 本身就安全（2026-09-18 续）

§10/§11 的方案有一个结构性缺陷：**保护写在 wrapper 里，而 wrapper 是可以被绕过的。**
`build.ps1` 会做缓存预检、会逼 build.rs 重跑、会扫产物 —— 但裸 `cargo build` 和 IDE 里点构建
一个都不做，而"曾经用官方归档构建过"的机器上缓存优先级最高（§11.2），于是产物会静默恢复 GPL。
正确性挂在"人记得用哪个命令"上，这在同一个问题上已经失效过两次（§10.4 那两条）。

结论：**断言必须放进每个人都要走的路径 —— 上游 crate 的 `build.rs` 本身。**

### 12.1 落盘

| 路径 | 动作 | 说明 |
|---|---|---|
| `vendor/sherpa-onnx-sys/` | **新增** | `sherpa-onnx-sys 1.13.3` 的快照，**只有 `build.rs` 被改**；`src/**` 与 `LICENSE` 逐字节上游。62.9 KB 纯文本，无二进制 |
| `Cargo.toml`（根） | 修改 | `[patch.crates-io] sherpa-onnx-sys = { path = "vendor/sherpa-onnx-sys" }`；members 加 `crates/xtask` |
| `crates/xtask/` | **新增** | 打包与产物门禁的 Rust 实现：`cargo xtask scan` / `cargo xtask package [--smoke-test]` |
| `.cargo/config.toml` | 改写 | **只剩 `[alias] xtask`**，不再注入 `SHERPA_ONNX_ARCHIVE_DIR` |
| `scripts/build.ps1`、`fetch-sherpa-libs.ps1`、`scan-artifact.ps1`、`package-release.ps1` | **删除** | 仓库内已无自建 `.ps1` |
| `.sherpa-libs/` | **删除** | 106.3 MB 的本地归档副本不再需要 |
| `Cargo.lock` | 修改 | `sherpa-onnx-sys` 的 `source` 行消失（registry → path）；新增 `sha2`、`zip`、`xtask` |
| `vendor/sherpa-onnx-sys/FORK-NOTES.md` | **新增** | 改了哪几处 + 上游升版本时怎么同步 |

### 12.2 fork 的五处改动

1. `RELEASE_BASE_URL` → `https://github.com/Sinsax/sherpa-onnx-prebuilt-no-tts/releases/download`
2. 常量 `PINNED_ARCHIVE_NAME` / `PINNED_ARCHIVE_SHA256` + 一道守卫：**非** Windows x64 static
   的目标（会落到官方归档）直接 `Err`，附两条逃生口（`SHERPA_ONNX_LIB_DIR`／摘掉 `[patch]`）
3. **sha256 校验**（新增 `sha2` build-dep）：下载后与 `SHERPA_ONNX_ARCHIVE_DIR` 本地副本都校验；
   不匹配的文件**当场删掉**，绝不留在缓存里让下一次构建信任它。上游**一行校验都没有**
4. `assert_placeholders_are_placeholders()`：`espeak-ng.lib` / `piper_phonemize.lib` / `ucd.lib`
   必须存在且 **≤ 4096 B**；**解压后**与**缓存命中时**两处都跑 —— 后者才是关键，它把 §11.2
   "唯一残留风险"从"靠 wrapper 兜"变成"裸 `cargo build` 与 IDE 构建都兜得住"
5. **下载超时**（`timeout_connect` 30s / `timeout_read` 60s）+ 传输失败时清掉 `.part`

### 12.3 实测

| 验证项 | 结果 |
|---|---|
| `cargo build --release -p yekitrim-cli --locked` | **EXIT=0**，9m32s（含 106 MB 下载 + 全量重链） |
| 归档来源 | 缓存内 `…lib.tar.bz2` = **111,446,628 B**，sha256 `94d733a9…e23b1f` 与 §10.1 钉死值一致；**无 `.part` 残留** |
| 为什么能断定是"下载"来的 | 构建前后 `SHERPA_ONNX_ARCHIVE_DIR` 与 `SHERPA_ONNX_LIB_DIR` **均为空**，项目 `.cargo/config.toml` **已无 `[env]`**，用户级 `~/.cargo/config.toml` **不存在**，`.sherpa-libs/` 当时仍在但已无人引用 → 缓存里那份归档只可能来自 fork 的下载 |
| 解压出的占位库 | `espeak-ng.lib` = **896**、`piper_phonemize.lib` = **896**、`ucd.lib` = **896**；`sherpa-onnx-core.lib` = **51,009,458**（TTS-off 尺寸，非官方的 66,036,138）；`onnxruntime.lib` = **816,663,584** |
| 产物门禁 `cargo xtask scan` | 1 个文件 / 33 MB，5 个标记全 0 → `PASS` |
| 打包 `cargo xtask package --smoke-test` | 门禁 PASS → 组装 → 烟测（随机端口启 exe，`/api/health` + `/` 返回 WebUI）→ zip **14.3 MB**，sha256 `c63b2f8e…` |
| 包内不含运行时状态 | `.yekitrim/`、`stickers/`、`*.db`、`initial-admin-password.txt` **均不存在**（烟测生成的被 xtask 自动清掉） |

**第一次尝试是失败的，值得记下来**：下载在 20 分钟里只爬到 **66.6 MB / 111.4 MB**，随后 **20 秒零增长**
且**没有任何报错** —— 上游 ureq 没设任何超时，于是"卡住"表现为"构建永远不结束"。
同一时刻用 curl 量同一条 URL：直连 20 MB 用 **1.65 s（12.7 MB/s）**、走 FlClash 代理
（`HTTP_PROXY=127.0.0.1:5441`）用 **1.25 s（16.7 MB/s）** —— 网络是好的，是那一条连接死了。
加上超时后重跑：**230 秒内 29 MB → 107 MB，全程无停顿**。

> 顺带结论：`cargo` **默认不回显 build script 的输出**（成功时），所以
> `eprintln!("Downloading …")` 不会出现在构建日志里。要看到它得加 `-vv`。
> 判断"归档到底从哪来"因此不能靠日志，要靠**排除法**（见上表第 3 行）。

### 12.4 `cargo xtask` 为什么在 xtask 里而不在 build.rs 里

cargo 内建只负责 build，**打包（组装 `webui/dist` + zip + 烟测）它做不了**，所以这部分是独立
Rust 二进制：`cargo xtask scan` / `cargo xtask package [--smoke-test] [--skip-build] [--version V] [--out-dir D]`，
别名写在 `.cargo/config.toml` 的 `[alias]`。它等值复刻了被删掉的 `package-release.ps1`
（含"只发 `target/release/yekitrim.exe`"、"构建后门禁不可跳过"、"烟测后清 `.yekitrim`/`stickers`"）。

两个实现细节：

- **扫描排除 `build/` 与 `deps/`**（同 §9.3 的理由），退出码 0=干净 / 1=检出 / 3=没扫到文件。
- `zip` crate 收紧了默认特性：`default-features = false, features = ["deflate", "time"]`。
  默认那套会把 `zstd` / `bzip2-sys` / `lzma-sys` / `aes` 一起拖进来 —— 四个 C 工具链由 `cc`
  现编，只为了写一个 zip，跟"不依赖外部工具"的初衷正好相反。

### 12.5 代价

- **仓库多 63 KB 文本**（`vendor/`）＋ 一个 `crates/xtask`。换来的是项目目录 106 MB → **0**、
  4 个 ps1 → **0**，以及"裸 `cargo build` 也安全"。
- **上游升版本要手动同步 fork**：`build.rs` 只有 13 KB、改动集中在常量块与 `download_prebuilt_libs()`，
  diff 很小；步骤见 `vendor/sherpa-onnx-sys/FORK-NOTES.md`。
- `Cargo.lock` 里该包的 `source` 从 registry 变成 path，需要提交一次；之后 `--locked` 正常。
- **离线构建**：归档仍可在本地留一份（`.gitignore` 已为它保留 `/.sherpa-libs/` 规则），
  设 `SHERPA_ONNX_ARCHIVE_DIR` 指向它即可 —— 但**同样要过 sha256 校验**（§12.2 第 3 条）。

