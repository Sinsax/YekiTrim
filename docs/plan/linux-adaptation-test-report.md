# Linux 适配测试报告

> 实测日期：2026-10-05 ｜ 机器：Arch Linux，kernel 7.2.3，x86_64
> 工具链：rustc/cargo 1.97.0 ｜ GCC 16.2.1 ｜ glibc 2.44 ｜ node ｜ ffmpeg n9.0.2 ｜ chromium
> 依据：`docs/linux-port-handover.md`、`docs/plan/linux-migration-handoff.md`
>
> | 轮次 | 被测版本 | 说明 |
> |---|---|---|
> | 第一轮 | `6a72335`（WebUI 编进二进制） | 全量首测 |
> | **第二轮** | **`a5d2cff`**（v0.4.1 + 内容指纹 + `config_version`） | **拉取新改动后复测，见 §0** |

**一页结论**：**Linux 上可编译、可启动、可出片、界面可用、ASR 已接通**。交付物是真 ELF native 二进制，
release 内嵌 WebUI，GPL 红线（无 espeak-ng）实测守住。剩下 **3 个测试失败**（全是测试侧的 Windows 硬编码，
非产品缺陷）与 **4 个移植缺口**（`xtask` 门禁在 Linux 上失效、`kill_tree` 杀不掉孙进程、
对拍脚本写死 Windows 字体路径、一处用户可见的 `D:/rec` 文案）。
**ASR 一跑推理就崩**（§4）—— 根因在上游预编译 onnxruntime，不在本仓。
> ✅ **2026-10-05 已修复**：上游仓换用 `glibc2_28` 变体后，下游 `yekitrim transcribe`
> 端到端跑通并出字幕。**§4 的「阻塞」结论已作废**，详见 §4.3/§4.5。

> **关于桌面**：本机**没有** X/Wayland 会话，但**不需要** —— 本轮所有结论（含界面核查）
> 都已在**无桌面**环境下取得（§8 用 headless Chromium + CDP 跑通了界面并定位到文案缺陷）。
> VNC **可以用**（`Xvnc` 自带 X server，适配合本机），但只在需要**真实交互**时才值得装。


---

## 0. 第二轮复测（`a5d2cff`）—— 新改动核实

拉取的 4 个提交：`824f945`（`config_version`）、`7691fba`（v0.4.1）、`225c03e`（**dist 新鲜度改内容指纹**）、
`a5d2cff`（指纹文件改名 `.built-from`）。

### 0.1 ✅ 我上一轮报的「dist 新鲜度误报」已被**正确修好**

上一轮实测：全新 clone 下 `dist` 比 `src` 旧 **9.49 ms**，编译直接 panic。新实现改成**内容指纹**
（`webui/dist/.built-from` 里的 `src_fnv1a64`，与 `build.rs::src_fingerprint` 逐字节对拍）。

**双向验证**（不只验"能过"，还验"还会红"）：

| 场景 | 结果 |
|---|---|
| 正常树 | ✅ 编译通过，21.75s |
| **伪造全新 clone**：把 `dist` 时间戳改到 **2020-01-01**（旧 mtime 判据下必红） | ✅ **照样通过** ⇒ 确实不再依赖 mtime |
| **篡改指纹**：把 `.built-from` 改成 `deadbeefdeadbeef` | ✅ **编译失败**，且报出两个指纹 `dist 记录 … vs 当前 src …` ⇒ 门禁**没有被削弱** |

> 这条修得干净：既消掉了误报，又保住了判据强度，错误信息还比原来更有用（直接给出两个指纹）。

### 0.2 ✅ 新增的 `config_version` 读时升级在 Linux 上行为正确

`CONFIG_VERSION = 1`；缺键 = 0。四条设计约定我都在 Linux 上实测了：

| 约定 | 实测 |
|---|---|
| 程序级（`.yekitrim/config.toml` / `llm.toml`）升级后**写回** + 备份 | ✅ 日志 `已从 v0 升级到 v1（原文件备份在 …config.toml.bak-v0）`；备份文件确实**不含** `config_version`（是原始形状） |
| 工程级（`yekitrim.toml`）**只在内存升级、不碰磁盘** | ✅ 跑完 `status` 后磁盘文件 `diff` **逐字节不变** |
| **保留注释**（用 `toml_edit` 而非 `toml`） | ✅ 升级后注释完整保留 |
| **`> CONFIG_VERSION` 绝不动它**（保护用户数据） | ✅ 造 `config_version = 999`：打 WARN `比本程序认识的 1 更新 —— 按现状读，不会改它`，文件**逐字节不变** |

### 0.3 ⚠️ 三个测试失败**未变化**（都在测试侧，本轮提交没碰）

`core` 由 196 → **207 passed**（新增 11 条 config 用例），但 `path::tests::state_dir_uses_the_new_name`
**依旧失败** —— 与 §2.1 完全同一处、同一行。`service` 的 2 条（§2.2 / §2.3）也原样。

**小结：765 → 776 passed / 3 failed。**

### 0.4 ⚠️ 三个移植缺口**未变化**

| 缺口 | 复测结果 |
|---|---|
| `xtask scan` Linux 空转（§3.1） | ⛔ 原样：`SCAN_EXTENSIONS` 仍是 `["exe","dll"]`，`exit 3` / `nothing to scan` |
| `CREATE_NO_WINDOW` 死代码（§3.1） | ⛔ 原样：`cargo check --workspace --all-targets` 仍 **1 条真 warning** |
| `kill_tree` 传正 pid（§3.2） | ⛔ 原样：`render_proc.rs:91` 仍是 `args(["-9", &pid.to_string()])` |
| 对拍写死 `C:/Windows/Fonts/msyh.ttc`（§3.3） | ⛔ 原样（未复核改动） |
| **ASR 崩溃（§4）** | ✅ **已修复**（2026-10-05，上游换 `glibc2_28`）；原为 `free(): invalid pointer` / SIGABRT / exit 134 |

> ASR 这条**预期之内**：崩点在**上游预编译 onnxruntime**里，本轮的配置/构建改动不可能影响它。
> ✅ 后续已由上游仓修复（§4.5）。

### 0.5 第二轮其他读数（均通过）

| 项 | 结果 |
|---|---|
| `cargo build -p yekitrim-cli`（debug） | ✅ 21.75s |
| `cargo build --release` | ✅ 3m17s |
| `cargo test --workspace` | ⚠️ **776 passed / 3 failed** |
| 两条 python 门禁 | ✅ 35 模块 / 7 项；**17 条 / 391 文件 / 0 违规** |
| node 契约脚本 | ✅ 接线 89 / 词汇表 104 / 预览引擎 19 全绿 |
| webui 8 个 `check-*.mjs` | ✅ 全部 `exit=0` |
| release 起服务（内嵌） | ✅ `health=200`、`静态资源：内嵌（30 个文件）` |
| release 二进制 GPL 面 | ✅ `espeak_` 0 / `espeak-ng|piper_phonemize` 0 |

---


## 1. 结论表

| 面 | 结果 | 证据 |
|---|---|---|
| `cargo build -p yekitrim-cli`（debug） | ✅ 通过 | 28.72s，产出 `target/debug/yekitrim`（ELF 64-bit PIE） |
| `cargo build --release` | ✅ 通过 | 4m30s，`target/release/yekitrim` 76 MB |
| `cargo check --workspace --all-targets` | ⚠️ **1 warning**（要求 0） | 见 §3.1 `CREATE_NO_WINDOW` 死代码 |
| `cargo test --workspace` | ⚠️ **765 passed / 3 failed** | 与交接单预测的 765 个一致；3 个失败见 §2 |
| sherpa-onnx Linux 链接 | ✅ 通过 | 静态链 252 个符号，零 undefined；运行期真进 C++ |
| **GPL 红线（无 espeak-ng）** | ✅ 守住 | `espeak_` 符号 0 个；字符串仅 `wespeaker` 假阳性 |
| WebUI（debug 读盘） | ✅ | 日志 `静态资源：磁盘 …/webui/dist`，`/` 出 index.html |
| WebUI（release 内嵌） | ✅ | 日志 `静态资源：内嵌（30 个文件）` |
| 两条 python 门禁 | ✅ 全过 | 35 模块 / 7 项；17 条边界 / 389 文件 / 0 违规（含 CRLF 卫生条） |
| node 契约脚本 | ✅ 全过 | 接线 89 项 / 转译器 / 词汇表 104 点 / 预览引擎 19 项 / 对拍自检 24 项 |
| webui 的 8 个 `check-*.mjs` | ✅ 全过 | 全部 `exit=0` |
| `xtask scan`（GPL 门禁） | ⛔ **在 Linux 上是空转** | `nothing to scan` + `exit 3`，见 §3.1 |
| 出片管线（转译 + 底座渲染） | ✅ 通过 | 1920×1080 / 536 帧 / 8.933s，与黄金帧**时长差 0.000000s** |
| ASR（sherpa-onnx 推理） | ✅ **已修复**（原为启动即崩） | 曾 `free(): invalid pointer` / SIGABRT；现跑通出字幕，见 §4.3–4.5 |
| 数据目录 `--data-dir` / `YEKITRIM_DATA_DIR` | ✅ | 只读安装目录下失败信息可指导，逃生口有效 |

---

## 2. 测试失败（3 个，全是测试侧 Windows 硬编码）

跑法：`SHERPA_ONNX_LIB_DIR=<...> cargo test --workspace --no-fail-fast`

### 2.1 ⭐ `path::tests::state_dir_uses_the_new_name` —— 改名时机械替换把判据写反了

```
crates/yekitrim-core/src/path.rs:177:
  assertion failed: !yekitrim_home_at(exe).to_string_lossy().contains("/.yekitrim")
```

**根因**：这条判据原本是 `contains("/.vtrim")`（断言旧目录名不再被创建）。改名 `vtrim → yekitrim` 时
被**机械替换**成 `contains("/.yekitrim")`，于是从"断言**旧**名不存在"变成了"断言**新**名不存在" ——
而紧接着第 175 行刚断言过路径**恰好等于** `/install/.yekitrim`。两条自相矛盾。

判据必然是恒假的（新名就是 `.yekitrim`），`git show 7ca52f9` 里能直接看到替换前后两行。

**为什么 Windows 上没被发现**：Windows 路径用反斜杠（`C:\install\.yekitrim`），字面量 `"/.yekitrim"`
（正斜杠）永远匹配不上 ⇒ **断言靠巧合通过**。Linux 用 `/` 做分隔符 ⇒ 立刻命中。

> 实测复算：`posixpath.join("/install",".yekitrim")` = `/install/.yekitrim` → 含 `/.yekitrim` → **断言失败**；
> `ntpath.join("C:\\install",".yekitrim")` = `C:\install\.yekitrim` → 不含 → **通过**。

**这是本报告里最有价值的一条**：它不是"Linux 才有的 bug"，而是**一条一直在空转的判据**，
Linux 只是把它暴露出来了。修法见 §5。

### 2.2 `workspace::tests::resolve_joins_root_for_relative` —— 测试用了 Windows 绝对路径

```
left:  "C:/projects/demo/C:/other/video.mp4"
right: "C:/other/video.mp4"
```

**根因**：`Workspace::resolve` 用 `p.is_absolute()` 判断（`workspace.rs:49-56`），实现**正确且平台无关**。
但测试把 `C:/other/video.mp4` 当绝对路径 —— 这在 Linux 上**不是**绝对路径（不以 `/` 开头），
于是走了 `join` 分支。**产品逻辑没问题，是测试的输入假设绑死了 Windows。**

### 2.3 `instancelock::tests::acquire_writes_pid_and_blocks_second_instance` —— 断言只认 Windows 那条错误分支

```
instancelock.rs:244: 错误文案要能指导用户
```

**根因**：`InstanceLock::acquire` 有两条拒绝路径：
- Windows：`open_exclusive` 用 `share_mode(0)` 独占 ⇒ 第二次 open 失败 ⇒ 走 `instancelock.rs:129` 的
  「**已有 yekitrim server 在使用同一个数据目录**」；
- 非 Windows：句柄不互斥 ⇒ 第二次 open 成功 ⇒ 靠锁文件里的 pid 判断 ⇒ 走 `instancelock.rs:149` 的
  「**锁文件 … 记录的进程仍在运行：pid …（启动于 …）**」。

测试只断言前者的文案。两条文案**都是合格的用户指引**（实测打印了后者，说明了 pid、启动时间与排障建议），
所以这是**测试断言过窄**，不是文案回归。

> 实测抓到的真实文案：
> `锁文件 /tmp/.tmpXXXX/yekitrim.lock 记录的进程仍在运行：pid 586779（启动于 2026-10-05 03:12:24）。`
> `若确认那是无关进程（例如锁文件是从别的部署复制过来的），用 --force 接管。`

---

## 3. 移植缺口

### 3.1 ⛔ `cargo xtask scan` 在 Linux 上**空转**（GPL 门禁失效）

```
$ cargo xtask scan
xtask scan: nothing to scan at /home/toki/project/YekiTrim/target/release
That is itself a gate failure: it usually means the build never ran.
exit=3
```

**根因**：`crates/xtask/src/main.rs:44`

```rust
const SCAN_EXTENSIONS: &[&str] = &["exe", "dll"];
```

Linux 的产物叫 `target/release/yekitrim`（**没有扩展名**），`path.extension()` 返回 `None`
⇒ 过滤器一条都不匹配 ⇒ `artifacts` 为空 ⇒ 报 `nothing to scan`。

**影响**：`xtask scan` 是根卡写明的**许可红线第二道防线**。在 Linux 上它**扫不到任何文件**，
等于这道防线不存在。严重度被"它失败时 exit 3（fail-closed）"限制住了 —— 不会静默放行，
但**Linux 的打包流程永远过不去**（`cargo xtask package` 依赖它）。

**同一 crate 的相邻问题**：`main.rs:46` 的 `CREATE_NO_WINDOW` 只在 `#[cfg(windows)]` 块里用
（`:771`），在 Linux 上是死代码 ⇒ **`cargo check --workspace --all-targets` 有 1 条 warning**，
而本仓硬要求是 **0 warning**。

> 旁证：我**手工**对 release 二进制复核了 GPL 面 —— `espeak_` 符号 0 个、
> `espeak-ng|piper_phonemize` 字符串 0 个。**产物是干净的**，失效的只是门禁本身。

### 3.2 ⛔ `kill_tree` 在 Linux 上杀不掉孙进程

`crates/yekitrim-server/src/render_proc.rs:87-95`：

```rust
// 负号 = 进程组；拿不到进程组时退回杀单个（孙进程由子进程退出时自行收尾）
let _ = yekitrim_core::process::cmd("kill")
    .args(["-9", &pid.to_string()])   // ← 传的是正数 pid，不是进程组
```

**注释说"负号 = 进程组"，但代码传的是正 pid** —— 只杀单个进程。而本仓 AGENTS 卡明确写着
取消必须杀掉**孙进程**（ffmpeg 与 Dhampir 底座拉起的进程），Windows 侧靠 `taskkill /F /T` 实现。

**实测**（`start_new_session` 起父进程 → 父进程再起孙进程）：

| 做法 | 孙进程 |
|---|---|
| `kill -9 <正 pid>`（**现行实现**） | **仍然存活** ✗ |
| `kill -9 -<pgid>`（负号 = 进程组） | **被回收** ✓ |

**影响**：Linux 上取消渲染任务后，ffmpeg / 底座孙进程会**继续烧 CPU 并占着文件**，
与 Windows 侧那条踩过的"文件被占用"事故同形。

> 注：`instancelock::process_alive` 的 Linux 分支（`/proc` + `kill -0`）**实测正常**，
> 本次失败的是它的**测试断言**（§2.3），不是实现。

### 3.3 ⚠️ 对拍脚本写死 Windows 字体路径

`scripts/yekitrim-compare.mjs:864`

```js
const fontFile = args.fontFile || 'C:/Windows/Fonts/msyh.ttc';
```

Linux 上默认值必然不存在 ⇒ 底座直接报
`--font-file 指不到一个文件：C:/Windows/Fonts/msyh.ttc` 并失败。

**逃生口有效**：显式传 Linux 中文字体即可（我用的是本仓自带的
`<data_dir>/resources/fonts/NotoSansSC-Regular/font.ttf`，与产品路径同源）。
交接单 §6 已预告"字体要换 Noto/思源"，这条属于**已预告但未落地**。

### 3.4 ⚠️ 前端 Windows 文案：已定位到**唯一一处用户可见的**

交接单提到用户可见的 `D:/rec`、`\\NAS\rec` 盘符文案。第二轮**逐条核过**（用 headless Chromium
真跑了界面，见 §8），全仓**只有一处**真的会显示给用户：

```
webui/src/components/RecorderConfig.vue:37
  <input v-model.trim="form.root" placeholder="例如 D:/rec 或 \\NAS\rec">
```

**已确认它进了发行产物**：`webui/dist/assets/SettingsView-DjUC5Fl-.js` 里含字面量 `D:/rec`。

**修法**（一行）：换成平台无关或 Linux 友好的例子，例如 `例如 /mnt/nas/rec 或 ./rec`；
若要保留 Windows 用户，可考虑按平台分支（`navigator.userAgent`）或直接两边都列。

> 其余命中都是**防御性代码或注释**，**不该改**：
> - `crates/yekitrim-server/src/api/config.rs` 的 `C:/`、`D:/` 是**路径面守卫的测试输入**
>   （验证盘符路径不会绕过作用域，见 `api/AGENTS.md` 的「路径面三防线」）；
> - `webui/src/components/ServerFileBrowser.vue` 的「盘符」是**读服务端 `volumes` 的通用渲染**
>   （Linux 上返回空数组，自然不显示）；
> - `crates/yekitrim-server/src/api/polish/:1888` 同为守卫用例。

### 3.5 ✅ 字体栈：**Linux 上实测正常，不是缺陷**

`polish.rs:1578-1581` 的字体候选链里确实有 `C:/Windows/Fonts/...`，一度像是 Linux 上的断链。
**实测结论：它是链尾兜底，Linux 上根本走不到，内置字体先行命中。**

| 探针 | 结果 |
|---|---|
| `GET /api/polish/fonts` | ✅ 返回内置 `NotoSansSC-Regular` / `NotoColorEmoji-CBDT`，二者 `installed: true` |
| `GET /api/polish/font?name=NotoSansSC-Regular` | ✅ **200 / 10,540,644 字节**，`file` 认作 TrueType Font |
| 与磁盘内置文件逐字节比 | ✅ **完全相同** |

⇒ 预览取字体这条链在 Linux 上是通的。**`C:/Windows/Fonts` 那条留着无害**（Windows 用户要用系统雅黑），
**别当 Linux 缺陷去删**。

> 前端 CSS 里的 `--font: 'Noto Sans SC','Microsoft YaHei',sans-serif`（`styles/tokens.css:95`）
> 是**浏览器侧**字体栈，靠 fallback 生效；`Microsoft YaHei` 在 Linux 上落空后由 `Noto Sans SC` 接住
> —— 而后者正是上面实测能拿到的内置字体，所以**预览与出片同源**，交接单 §5 担心的
> "预览与出片不一致"**未被证实**。

---

## 4. ✅ ASR：曾「链接通过、运行即崩」——**已修复**（2026-10-05）

> 本章保留完整诊断过程；**结论已由 §4.5 的修复取代**。

这是本次最重的发现。**链接面完全正常**（见 §4.1），但**一跑推理就 SIGABRT**。

### 4.1 链接面：✅ 正常

用 `SHERPA_ONNX_LIB_DIR=/home/toki/project/sherpa-onnx-prebuilt-no-tts/build/cmake-build/install/lib`
（交接单推荐的方案 ②）编译成功：

- 静态链进 252 个 `SherpaOnnx*` 符号，**零 undefined / 零 duplicate**；
- 库顺序经独立复算**是对的**（c-api → core；**反过来会真报 undefined reference**，
  说明顺序是承重的，而现行 `SHERPA_ONNX_STATIC_LIBS` 顺序正确）；
- `libstdc++ / m / pthread / dl` 声明齐全；
- 独立探针二进制**真进了 C++ 代码**：`SherpaOnnxGetVersionStr()` 返回 `"1.13.3"`，
  `SherpaOnnxCreateOfflineRecognizer` 走到底层校验并报 `Errors in config`（预期行为）。

### 4.2 运行面：⛔ 崩溃

用**真实语音**（`zh.wav`，5.6s）跑 `yekitrim transcribe`：

```
INFO  yekitrim_core::asr::local: ASR 模型: SenseVoice (int8)
INFO  yekitrim_core::asr::local: SenseVoice 同音纠错已启用 (lexicon.txt + replace.fst)
free(): invalid pointer
timeout: the monitored command dumped core      ← raw exit=134 (SIGABRT)
```

**稳定复现**，`gdb` 回栈：

```
#2  abort ()
#6  std::__detail::_Compiler<std::regex_traits<char> >::_Compiler(char const*, char const*, std::locale const&, ...)
#7  onnxruntime::DeviceDiscovery::DiscoverDevicesForPlatform()
#8  onnxruntime::DeviceDiscovery::GetDevices()::{lambda()#1}::operator()()
#9  onnxruntime::DeviceDiscovery::GetDevices()
#10 onnxruntime::Environment::EpInfo::Create(...)
#11 onnxruntime::Environment::RegisterExecutionProviderLibrary(...)
```

崩点在 **onnxruntime 环境初始化**（还没碰模型）里的 `std::regex` 构造。

### 4.3 根因（**§4.3 于 2026-10-05 订正**）

> ⚠️ **本节初版把根因写成「上游 glibc2_17 预编译包的旧 libstdc++ 与宿主 GCC 16 失配」。**
> **该归因不完整、且方向偏了 —— 已订正。**
>
> **否证实验**：把**同一份归档**放进 **`sherpa-onnx-prebuilt-no-tts` 自己的
> `ubuntu:22.04` 构建容器**（GCC 11.4 / libstdc++ 6.0.30，与归档**同龄**）里跑真实模型——
> **照样崩**。既然是"同龄"环境也崩，就**不是"宿主太新"**。
>
> **订正后的根因**：问题出在**归档内部混了两批不同编译器产的目标文件**：
>
> | 组件 | 编译器（`.comment` 实测） | 来源 |
> |---|---|---|
> | `libonnxruntime.a` | **GCC 11.2.1 (Red Hat)** | 上游预编译包 `...-glibc2_17.zip` |
> | `libsherpa-onnx-*.a` | **GCC 11.4.0 (Ubuntu 22.04)** | 本仓在 ubuntu:22.04 容器里编 |
>
> `_Compiler<std::regex_traits<char>>` 在 `libonnxruntime.a` 里有 **6 处重复弱定义**，
> 而共享 libstdc++ **并不提供**这些模板符号（纯头文件模板）⇒ 链接器**静默合并**弱符号
> （零错误、零警告）⇒ 运行期跨边界堆操作错乱 ⇒ `free(): invalid pointer`。
>
> ⚠️ 排查中途还曾认定判据是「**厂商族不同**（Red Hat vs Ubuntu）」，**该假设同样被推翻**：
> 新旧两个 onnxruntime 变体**都是 Red Hat**（11.2.1 vs 14.2.1），一个崩一个不崩。
> **有效信号是上面那个弱符号重复数（6 → 0），不是厂商、也不是主版本差。**

**已排除的替代解释**（这些仍然成立）：

- ❌ 不是宿主 `std::regex` 坏 —— 原生编译同一段 regex 代码运行正常（`regex ok: 1 -> CPU`）；
- ❌ 不是 ffprobe 缺失 —— `ffprobe` 在 Linux 上实测正常（对真实文件返回 `320,240`）；
- ❌ 不是库顺序/重复符号 —— 链接零错误；
- ❌ 不是环境变量能绕过的 —— `ORT_DISABLE_ALL_PROVIDERS=1` 照样崩；
- ❌ **不是"宿主太新"** —— 见上方否证实验（同龄容器内同样崩）。

### 4.5 ✅ 修法（**已于 2026-10-05 由上游仓修复并验证**）

**实际采用的修法**：`sherpa-onnx-prebuilt-no-tts` 把 onnxruntime 从 `glibc2_17`
换成上游的 **`glibc2_28` 变体（GCC 14.2.1）** —— 该弱符号重复数 **6 → 0**，
只替换一个 `.a`，未重编 sherpa。

**验证结果（下游侧实测）**：

```
$ YEKITRIM_DATA_DIR=/tmp/e2e/.yekitrim yekitrim transcribe -c /tmp/e2e/yekitrim.toml
INFO  字幕已生成: output/video.srt
INFO  转录 JSON 已保存: output/transcript.json
exit=0
```

`video.srt` 实际内容（真实时间轴 + 文本）：

```
1
00:00:00,853 --> 00:00:03,873
呢几个字都表达唔到我想讲嘅意思
```

⇒ **Linux 上 ASR 已可用**，本报告 §1 的"⛔ 阻塞"结论**作废**。

**代价（下游需要知道的新下限）**：`glibc2_28` 引用 `__cxa_call_terminate`
（属 **CXXABI_1.3.15**）⇒ 需 **libstdc++ ≥ GCC 12**。
`ubuntu:22.04`（libstdc++ 6.0.30 / CXXABI_1.3.13）**链不了**该变体；
Arch / Ubuntu 24.04+ / Debian 13+ 可以。

**上游仓同时补了两条门禁**（都对旧归档翻面验证过）：

- `scripts/check-toolchain-consistency.sh` —— 静态、秒级、无需模型；
- `scripts/smoke-real-model.sh` —— 运行期，⚠️ **必须真实模型**
  （实测"不存在的模型路径"会被 `Validate` 拦下**不崩**，假模型覆盖不了）。

详见 `sherpa-onnx-prebuilt-no-tts/linux-adaptation-plan.md` 与
`asr-runtime-crash-linux-x64.md`。

### 4.6 原建议（留档，已被上面的实际修法取代）

1. ~~首选：用同一套 glibc/libstdc++ 基线重建 onnxruntime~~ —— 实际用了更省的
   "换上游另一变体"，未重编；
2. 或把 onnxruntime 换成动态链接 libstdc++ 的构建（避免弱符号内嵌）—— 未采用；
3. 在 README 的 Linux 小节**补上这条 ABI 注意事项**（目前只写了 glibc 符号版本）；
4. 短期：`doctor` / 启动时**探测 ASR 可用性**，失败给出可操作提示，而不是留一个一跑就 SIGABRT 的入口。

---

## 5. 建议的修复清单（按性价比排序）

| # | 位置 | 改动 | 风险 |
|---|---|---|---|
| 1 | `crates/yekitrim-core/src/path.rs:177` | 判据改成断言**旧名** `.vtrim` 不存在（恢复原意）：`!…contains("/.vtrim")` | 极低；纯测试 |
| 2 | `crates/xtask/src/main.rs:44` | `SCAN_EXTENSIONS` 增加"**无扩展名**"分支（Linux ELF），并补 `#[cfg]` 让 `CREATE_NO_WINDOW` 只在 windows 下存在 | 低；门禁修复 |
| 3 | `crates/yekitrim-server/src/render_proc.rs:87` | `kill -9 -<pgid>`（负号 = 进程组），与注释一致；取不到 pgid 时退回杀单个 | 中；**建议配一条真起孙进程的回归用例** |
| 4 | `crates/yekitrim-service/src/workspace.rs:176` | 测试改用平台无关的绝对路径（或 `#[cfg]` 两支） | 极低 |
| 5 | `crates/yekitrim-service/src/instancelock.rs:244` | 断言放宽为"两条文案其一即可指导用户" | 极低 |
| 6 | `scripts/yekitrim-compare.mjs:864` | 按平台给默认字体（Linux 用本仓自带 `NotoSansSC-Regular/font.ttf`） | 低 |
| 7 | `webui/src/components/RecorderConfig.vue:37` | 占位符 `D:/rec 或 \\NAS\rec` 换成平台友好写法（**唯一用户可见的一处**，见 §3.4）；改完要 `npm --prefix webui run build` 重建 dist | 极低 |
| 8 | ASR | ✅ **已解决**：上游仓换 `glibc2_28` 版 onnxruntime（见 §4.5） | 已闭合 |

> 1–7 都是**小改动、无行为风险**（含 3 处测试断言、2 处门禁/工具、1 处文案），可以一轮做完；
> ~~8 需要用户就「ASR 基线」拍板。~~ ✅ 已按「换上游另一变体」落地，无需再拍板。

> ✅ **明确不要动的**：`crates/yekitrim-server/src/api/polish/:1578-1581` 的 `C:/Windows/Fonts` 兜底
> —— Linux 上实测走不到内置字体之前就命中了（§3.5），留着对 Windows 用户有用。

---

## 6. 复现命令（Linux）

```bash
cd /home/toki/project/YekiTrim
export SHERPA_ONNX_LIB_DIR=/home/toki/project/sherpa-onnx-prebuilt-no-tts/build/cmake-build/install/lib

# ① 编译
cargo build -p yekitrim-cli
cargo check --workspace --all-targets          # 期望 0 warning，实测 1（§3.1）

# ② 测试
cargo test --workspace --no-fail-fast          # 实测 765 passed / 3 failed（§2）

# ③ 门禁
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
for s in webui/scripts/check-*.mjs; do node "$s" || echo "FAIL $s"; done
cargo xtask scan                               # 实测 exit 3 空转（§3.1）

# ④ 起服务（debug 读盘）
rm -rf /tmp/yt && mkdir -p /tmp/yt
YEKITRIM_DATA_DIR=/tmp/yt ./target/debug/yekitrim server --local-no-auth --port 26320
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:26320/api/health   # 期望 200

# ⑤ 起服务（release 内嵌）
cargo build --release -p yekitrim-cli
YEKITRIM_DATA_DIR=/tmp/yt-rel ./target/release/yekitrim server --local-no-auth --port 26400
# 启动日志应出现：静态资源：内嵌（30 个文件）

# ⑥ ASR（曾会崩，§4；上游换 glibc2_28 后已修复 —— 应产出 output/video.srt）
YEKITRIM_DATA_DIR=/tmp/yt ./target/debug/yekitrim transcribe -c <工程>/yekitrim.toml
```

---

## 7. 已核实无误的移植面（避免下一轮重做）

- **`path::data_dir()` = `YEKITRIM_DATA_DIR` ?? exe 目录** —— 只读安装目录下**优雅失败**且文案可指导：
  `无法创建账号数据目录 …（Permission denied）… 请用 --data-dir <可写路径>`；`--data-dir` 逃生口实测有效。
  ⇒ 交接单 §3 的"数据目录必须重新决策"**实测风险低于预期**（有逃生口，不是硬阻塞）。
- **`instancelock::process_alive`** Linux 分支（`/proc/<pid>` + `kill -0`）实测正常。
- **`disk::space_bytes`** 用 `df -k` 解析第 2 行实测正常（tmpfs 上也对）。
- **`open_browser` → `xdg-open`** 失败不拦启动（无桌面场景安全）。
- **托盘** `yekitrim-tray` 在 Linux 上打一行提示 + `exit 1`，不崩、不误导。
- **WebUI 两种来源**都实测正确：debug 读盘、release 内嵌（30 文件），且有启动日志一行可辨。
- **`--local-no-auth` 两道闸**实测生效：非 loopback `Host` 的**写请求**被 403；
  跨源 `Origin` 的写请求被 403（只读 `/api/health` 放行，符合设计）。
- **CRLF 卫生门禁**（边界第 17 条）在 Linux 检出上天然通过。
- **黄金帧对拍不需要 Chrome**，Linux 上可跑（本次实跑到渲染完成）。

---

## 8. 关于「桌面问题用 VNC 够不够」

### 8.1 结论：**够，而且 VNC 是这台机器上唯一可行的桌面方案；但"桌面"本身能测的面很窄**

**本机现状（实测）**：

| 项 | 值 |
|---|---|
| 有图形会话吗 | **没有**。`loginctl` 里 4 个会话，其中 seat0 那个是 **`Type=tty`**（不是 `x11`/`wayland`） |
| 有合成器吗 | **没有**。`gnome-shell` / `kwin` / `sway` / `Hyprland` / `Xorg` / `plasmashell` / `mutter` **一个都没在跑** |
| `DISPLAY` / `WAYLAND_DISPLAY` / `XDG_SESSION_TYPE` | **三者全未设置** |
| `/tmp/.X11-unix/` | **不存在**（没有任何 X server 在听） |

也就是说：**当前没有任何 X/Wayland 会话**。所以问题不是"用 VNC 还是用别的远程桌面" ——
而是**得先有一个 X server**。VNC 正好能在无显示器的机器上**凭空起一个**（`Xvnc` 自带 X server），
这一点比 `x11vnc`（只能**附加到已存在的真实 X 显示**）更适合本机。

**可用性实测**：两个包本机仓库里都有（`extra/tigervnc 1.16.2-5`、`extra/x11vnc 1:0.9.17-1`），但**都未安装**；
当前用户属于 `wheel`，装包可行。**我本轮没有安装**（涉及系统级改动，等你点头）。

### 8.2 VNC 能测什么、不能测什么

**⚠️ 先说不能测的 —— 本轮所有 Linux 适配结论都不依赖桌面**：

| 已实测项 | 需要桌面吗 |
|---|---|
| 编译 / 测试 / 两条门禁 / node 契约脚本 | ❌ 不需要（已全跑） |
| HTTP 服务、`/api/health`、内嵌 WebUI 返回 | ❌ 不需要（已全跑，`curl` 即可） |
| **出片管线（转译 + 底座渲染）** | ❌ **不需要**（已实测到 1920×1080 / 536 帧 / 时长差 0.000000s） |
| 黄金帧对拍 | ❌ **不需要 Chrome**（交接单 §2② 已写明） |
| ASR | ❌ 不需要（崩在上游 onnxruntime，与桌面无关） |
| `webui/dist` 新鲜度 | ❌ 不需要（**新版已改成内容指纹**，见 §0.1） |

**VNC 真正能补的那一块**：`webui/dist` 的界面**在真实浏览器里**渲染对不对 ——
即"界面/路径"清单里那几条**只能在真浏览器里看出来**的项（交接单 §5 的「界面/路径」组）：

- 用户可见的 Windows 文案（`D:/rec`、`\\NAS\rec`）是不是真的显示出来了；
- 中文字体回退在浏览器里是否正常（预览走浏览器、出片走内嵌 Noto，**两条腿本来就可能不一致**）；
- 布局 / 色档在 Linux 上的实际观感。

### 8.3 实际做法：**headless Chromium 已经够用 —— 本轮已用它跑通界面**

本机**已装 `chromium`**（`/usr/bin/chromium`），**无需装任何东西、无需桌面**。

**⚠️ 一个实测踩到的坑，记下来免得下一个人重复**：直接把 `--screenshot` 指向 YekiTrim 的页面
**会一直卡住、出不了图**（`data:text/html` 这种简单页却正常）。原因不在渲染 ——
用 `--dump-dom` 能看到 DOM 已完整构建、服务端日志里 `/api/projects` 与 `/api/me` 都 **200**，
即**页面本身渲染正常**；卡住的是 `--screenshot` 在等"网络静默"，而 WebUI 有一条**长期挂着的 SSE**
（`/api/stream`，`yekitrim-server` 的 AGENTS 卡专门写过它会导致"永远等不到返回"）。
`--virtual-time-budget` 加到 15s 也没用。

**可行的两步法**（本轮用的，零依赖）：

```bash
# ① 先确认页面真的渲染了（不受 SSE 影响）
chromium --headless=new --no-sandbox --user-data-dir=/tmp/cp --dump-dom http://127.0.0.1:26400/ | head -c 300

# ② 要图就走 CDP —— Page.captureScreenshot 不等待网络静默，SSE 挡不住
node /tmp/shot.mjs http://127.0.0.1:26400/ /tmp/app.png     # 见下方脚本
```

CDP 脚本（`shot.mjs`，约 30 行，只用 node 内置的 `fetch` + `WebSocket`）：
起 chromium 带 `--remote-debugging-port` → 从 `/json/list` 拿页面 target →
`Page.navigate` → 睡 7 秒让 Vue 挂载 → `Page.captureScreenshot` → 落盘。
实测输出 **133,429 字节 PNG**。

**本轮用它真读到了界面文本**（证明 Linux 上界面可用）：

```
YekiTrim / 切片工作台 / 项目（0） / 本地模式 / 首页 / 设置 / 浅色 / 跟随系统
把录像变成项目 / 切片总览 / 未导出 0 · 已导出 0 / 新增项目
选一个来源…服务器文件 / 录播姬录制 / B 站链接 / 浏览… / 新建项目
```

⇒ 中文界面、路由、主题、空状态全部正常，**没有任何渲染失败**。

**代价对比**：

| 方案 | 成本 | 能覆盖 |
|---|---|---|
| **headless chromium（`--dump-dom` + CDP 截图）** | 极低，**无需装任何东西、无需桌面**；已将 SSE 卡住的坑与解法记在上面 | 页面能否渲染、**用户可见文案**、字体回退、布局 —— 本轮已靠它定位到 §3.4 那处 `D:/rec` |
| VNC + 真桌面 | 装包 + 起会话 + 连进去；**且要额外装桌面/浏览器** | 同上 + **真实交互**（点击、拖拽、真键盘输入） |
| 都不做 | 0 | 界面那几条**留在未核实状态** |


### 8.4 建议

1. **先用 headless chromium 截图**核界面（零安装、零风险，能覆盖大部分）；
2. **只有当需要真实交互**（点按钮、拖滑块、验拖拽）时，再装 VNC：
   ```bash
   sudo pacman -S tigervnc          # 含 Xvnc（自带 X server ⇒ 适配无显示器机器）
   vncserver :1 -geometry 1600x1000 -depth 24
   # 然后从你的机器 SSH 端口转发：ssh -L 5901:127.0.0.1:5901 toki@<host>
   ```
   ⚠️ VNC 只监听 loopback 最安全（默认 `-localhost`），配合 SSH 转发用。
3. 若走 VNC 路线，**建议顺手确认 `--local-no-auth` 在 `xdg-open` 起的浏览器里两道闸仍成立**
   —— 这是交接单 §2 唯一还标着【待实测】的桌面项（不过我在 §7 已用 `curl` 验证了闸本身的逻辑，
   桌面只影响"浏览器怎么发起请求"，不影响闸的判定）。

> **一句话**：VNC **可以**，但它解决的是一个**当前没有被卡住**的问题 —— 编译、测试、出片、
> 门禁全都已经在无桌面环境下跑通了。真有界面要核，**headless chromium 是更划算的第一步**。

---

## 9. 交付物

| 文件 | 说明 |
|---|---|
| `target/debug/yekitrim` | 157 MB（含 debuginfo），Linux ELF native |
| `target/release/yekitrim` | 76 MB，内嵌 WebUI（30 文件）+ 内嵌字体，**零 espeak-ng** |

> 打包（`cargo xtask package`）**未在 Linux 上跑通** —— 被 §3.1 的 `scan` 空转挡住，
> 且包名/后缀仍写死 `win-x64` / `.zip` / `.exe`（交接单 §4 已预告）。
