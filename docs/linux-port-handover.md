# Linux 移植交接

> ⚠️ **2026-10-30：当前 Linux 的操作口径见 [`docs/linux.md`](linux.md)**（构建/发行包/systemd/验证/已知限制）。
> 本文保留为 2026-10-03 的移植过程记录；其中 §2 的 3 个测试失败、§3.1 的 scan 空转、§3.2 的 kill_tree、
> §3.3 的字体路径、§3.4 的 `D:/rec` 文案**都已在 2026-10-30 修掉**。

> ⚠️ **本文已有实测续篇**：**`docs/plan/linux-adaptation-test-report.md`**（2026-10-05，在真 Linux 机器上跑的）。
> 本文里 6 处 **【待实测】** 多数已被那份报告解答或改写，**读本文前先看它**，尤其这几条结论要按新报告更正：
> - §1「ASR 是最大阻塞」：**链接面已通过**（`SHERPA_ONNX_LIB_DIR` 方案可行），但**运行期崩**（`SIGABRT`）——
>   根因是**上游预编译 onnxruntime 的 libstdc++ ABI 失配**，不在本仓 `build.rs`；
> - §3「数据目录必须重新决策」：**实测风险低于预期** —— 只读安装目录下是**优雅失败 + 可指导文案**，
>   `--data-dir` / `YEKITRIM_DATA_DIR` 逃生口有效；
> - §4「打包」：除写死 `win-x64`/`.zip`/`.exe` 外，**`cargo xtask scan` 在 Linux 上会空转**（`exit 3`）；
> - §5 清单里「字体栈」「大小写敏感」「`--local-no-auth` 两道闸」等项，新报告 §3.5 / §7 有实测读数。

> 写于 2026-10-03（Windows 侧）。**本文只陈述我在 Windows 仓库里能核实的事实**（都带文件名/行号），
> 凡是"到了 Linux 才能确认"的，一律标注 **【待实测】**，不假装已经跑过。
>
> 适用范围：把 YekiTrim 从 Windows 移植到 Linux（桌面单机 + 可选的服务器部署两种形态）。

---

## 0. 一页结论

| 面 | 状态 |
|---|---|
| **WebUI** | ✅ **已经不是问题**：`webui/dist` 现在**编在 exe 里**（`crates/yekitrim-server/build.rs`），与平台无关；前端只有 `xdg-open` 这类已分支的调用 |
| **HTTP 服务 / 任务队列 / 配置 / 数据库** | ✅ 平台无关（axum + sqlite + toml），且 **DB 自带迁移**（`PRAGMA user_version` + `migrate()`） |
| **外部命令调用** | ✅ 已有分支：`open_browser` 走 `xdg-open`（`yekitrim-server/src/lib.rs:567-575`）、`kill_tree` / `space_bytes` / 进程存活检查都有 `#[cfg(not(windows))]` 实现 |
| **ASR（sherpa-onnx）** | ⛔ **最大阻塞**：vendored fork **只带 Windows x64 静态归档**，其它目标会**直接报错**（见 §1） |
| **托盘（`yekitrim-tray`）** | ⛔ 整个是 Win32；Linux 需要另做（方案在 §2） |
| **数据目录** | ⛔ **必须重新决策**：现在是"exe 旁边"，而 Linux 上 `/usr/bin`、`/opt` 通常不可写（见 §3） |
| **打包** | ⚠️ `cargo xtask package` 写死了 `win-x64` + `.zip` + `.exe`（`crates/xtask/src/main.rs:405-407`） |
| **出片底座（Dhampir）** | ⚠️ 独立项目，需要它自己的 Linux 构建；本仓只经 `find_translator` 找它 |

**建议顺序**：§3 的数据目录决策（会改到最多地方，越早定越好）→ §1 ASR → §2 托盘/服务 → §4 打包 → §5 清单。

---

## 1. ⛔ ASR：vendored `sherpa-onnx-sys` 只支持 Windows x64

**证据**（`vendor/sherpa-onnx-sys/build.rs`）：

```rust
const PINNED_ARCHIVE_NAME: &str = "sherpa-onnx-v1.13.3-win-x64-static-MT-Release-lib.tar.bz2";
...
// YekiTrim delta 2. Our release only carries the pinned Windows x64 static archive. Any other
// target or link mode would have to come from the official upstream release, which links
// GPL-3.0-or-later espeak-ng unconditionally -- so fail loudly rather than quietly ship a
// GPL binary. SHERPA_ONNX_LIB_DIR (checked first, below) remains the escape hatch.
if archive_name != PINNED_ARCHIVE_NAME {
    return Err(...)
}
```

所以到 Linux 上有三条路，**推荐 ②**：

| 方案 | 做法 | 代价 |
|---|---|---|
| ① 删掉根 `Cargo.toml` 的 `[patch.crates-io] sherpa-onnx-sys` | 回到上游 crate | ⛔ **会把 GPL-3.0 的 espeak-ng 链进来** —— 会让整个发行物变成 GPL 组合作品。**没有明确许可决策就别走这条** |
| ② `SHERPA_ONNX_LIB_DIR=<自己编的 lib 目录>` | 在 Linux 上编一份**关掉 TTS/espeak** 的 sherpa-onnx，把 lib 目录指过去 | 要自己维护一份 Linux 构建；**必须继续保证不带 espeak-ng** |
| ③ 给 vendor fork 加 Linux 归档 | 在 `build.rs` 里加 `linux-x64` 的归档名与校验，把归档放进 vendor | 要改 fork（它有自己的 `FORK-NOTES.md`）/ 加体积 |

⚠️ **许可红线不变**（根卡 + `THIRD-PARTY-NOTICES.md`）：**不许把 espeak-ng 或任何 GPL 组件弄进二进制**。
两道防线都要在 Linux 上继续生效：`vendor/sherpa-onnx-sys/build.rs` 的占位断言 +
`cargo xtask scan`（它是 Rust 二进制，本身平台无关）。

**【待实测】** Linux 上编译出的 sherpa-onnx 静态库与当前 `sherpa-onnx` crate 版本的 API 是否完全兼容
（现在这份 pin 是 `v1.13.3`）。

---

## 2. 托盘与"双击就用"：整块 Win32，需要另做

`crates/yekitrim-cli/src/tray.rs`（约 570 行）**全部是 Windows**：`windows-sys`、`Shell_NotifyIconW`、
`ShellExecuteW`、**Job Object**（`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`，保证"托盘被杀 ⇒ 服务一起走"）、
`CREATE_NO_WINDOW`、`include_bytes!` 的 ico。非 Windows 下它只打印一行提示就退出
（`tray.rs:46` 的 `#[cfg(not(windows))]`）。

**Linux 的等价物建议用系统设施，而不是再写一个 GUI 托盘**：

| Windows 里托盘做的事 | Linux 对应 |
|---|---|
| 双击启动 `yekitrim.exe server --local-no-auth --open` | `yekitrim.service`（`systemd --user`）或一个 `.desktop` 项 |
| 托盘图标 / 右键菜单 | `systemd --user` 的 `systemctl --user start|stop yekitrim`（或用 `yekitrim status`） |
| **"父死子死"（Job Object）** | systemd 默认 **`KillMode=control-group`** —— 停 unit 会连它拉起的 ffmpeg/底座一起收 |
| 退出 = `POST /api/shutdown` + 令牌 | **同一套契约直接可用**：令牌走 `YEKITRIM_SHUTDOWN_TOKEN`、端点是 `/api/shutdown`（见 `yekitrim-server/src/api/AGENTS.md`）。一个 shell 脚本 `curl -X POST -H "x-yekitrim-shutdown-token: $TOK" ...` 就够 —— **当初把这两条做成"进程边界上的契约"而不是进程内调用，正是为了这种场景** |
| 抓 `WebUI: http://…` 从 stdout 拿端口 | Linux 脚本**不需要抓**：直接 `http://127.0.0.1:26320`，或用 `--port`（默认端口唯一来源是 `yekitrim-service/src/doctor.rs::DEFAULT_PORT`） |

⚠️ 若真要写 Linux 托盘（GTK/AppIndicator），注意它**不能**链接 `yekitrim-core`（那条依赖会把 ASR 拉回来，
Windows 侧就是因此把托盘做成 0.5 MB 的"薄启动器"；见 `crates/yekitrim-cli/AGENTS.md`）。
shell 脚本或 systemd unit **天然满足这条**。

**【待实测】** `--local-no-auth` 在 Linux 桌面上的两道闸（`Host` 必须 loopback、写请求必须同源）
在 `xdg-open` 打开的浏览器里是否同样成立（理论上与平台无关）。

---

## 3. ⛔ 数据目录：**这是必须重新决策的架构点**

现状（根卡「边界」第一条，也是 `docs/file-layout.md` 的正文）：

> `path::data_dir()` = `YEKITRIM_DATA_DIR` ?? **`exe_dir()`** —— 配置/账本/模型/字体/工程全在它下面。

Windows 上这很自然（解压即用、绿色安装）。**Linux 上通常不成立**：装到 `/usr/bin`、`/opt/yekitrim`、
`/usr/local/bin` 时，exe 所在目录**普通用户不可写**，于是：

- 首次启动写 `.yekitrim/config.toml` 就失败；
- 模型（几个 GB）没地方放；
- 日志写不出去。

**三个候选（需要用户拍板）**：

| 方案 | 说明 | 影响面 |
|---|---|---|
| A. 保持 exe 旁边（要求"绿色解压到 `~/YekiTrim`"） | 文档里规定安装方式；systemd unit 用 `WorkingDirectory` 指过去 | 改动最小，但对"系统级安装"不友好 |
| B. 默认 `$XDG_DATA_HOME/yekitrim`（退 `~/.local/share/yekitrim`） | 符合 Linux 习惯；`YEKITRIM_DATA_DIR` 仍可覆盖 | ⚠️ 与根卡那句"**别再挂 `~/.yekitrim/`**"口径**相反** —— 那条是为 Windows 单目录安装定的，**必须显式改口径**（否则下一个人会以为违背了不变式） |
| C. 按安装形态分流：exe 目录可写就用它，否则用 XDG | 兼容两种装法 | 逻辑最复杂；"两个可能的位置就必然有地方只查一处"—— 根卡已经吃过这个亏（CLI 与 WebUI 各看一份字体库） |

⚠️ 不管选哪个，**都要同步改**：`crates/yekitrim-core/src/path.rs`、`docs/file-layout.md`、
根卡那条不变式、以及**已经装在用户机器上的数据迁移**（如果之前有 Windows 之外的用户，这里是空的，
所以现在是**改这个的最佳时机**）。

---

## 4. 打包与发布

`cargo xtask package` 现在是 Windows 专用形态（`crates/xtask/src/main.rs`）：

- 包名写死 `yekitrim-v{version}-win-x64`（:405）、产物 `.zip`（:407）、必需清单里有 `yekitrim.exe`；
- 冒烟测试（`--smoke-test`）会启动**解包后的 exe** 并检查 `/api/health` 与 `/`（`xtask` 里那段
  `checking /api/health and the WebUI`）—— 这条**与平台无关**，Linux 上照样能跑；
- 它还会清掉解包目录里的 `.yekitrim/` 与 `stickers/`（跑冒烟会生成）—— 逻辑同样可复用。

Linux 需要：目标名（`linux-x64` / `aarch64`）、`.tar.gz`（保留可执行位）、去掉 `.exe` 后缀、
`.desktop` 项 + systemd unit 随包（或单独给）、以及 `resources/fonts/` 那个空目录的处理照旧。

⚠️ **字体不需要往包里放**：它们早就 `include_bytes!` 编在 `yekitrim.exe` 里了，
`xtask package` 甚至**专门删掉**包里重复的那份（理由写着"纯重复"）。WebUI 现在也是同样套路。

---

## 5. 移植检查清单

**构建 / 依赖**
- [ ] `vendor/sherpa-onnx-sys` 的 Linux 归档（或 `SHERPA_ONNX_LIB_DIR`）就位，且**不含 espeak-ng**
- [ ] `cargo xtask scan` 在 Linux 上通过（GPL 门禁）
- [ ] `cargo check --workspace --all-targets` **0 warning**（本仓硬要求）
- [ ] `cargo test --workspace`：**765 个测试**里有没有 Windows-only 的（【待实测】）
- [ ] `vite build` 与 9 个契约脚本（`node webui/scripts/check-*.mjs`、`node scripts/check-preview-audio.mjs`）
- [ ] 两条门禁：`python3 scripts/check_module_map.py`、`python3 scripts/check_module_boundaries.py`
      （后者第 17 条是"工作副本不许有 CRLF"，Linux 上天然满足 ✓）

**运行**
- [ ] `path::data_dir()` 的决策落地（§3），并确认**首次启动**能写出配置/日志/模型目录
- [ ] `instancelock`：Linux 分支用 `/proc/<pid>` + `kill -0`（`yekitrim-service/src/instancelock.rs:80-88`）
      —— 在容器里 `/proc` 可能不同，【待实测】
- [ ] `disk.rs` 的 Linux 分支用 `df -k`（`yekitrim-service/src/disk.rs:36-48`）—— 在**网络盘/overlayfs** 上
      的输出格式要实测
- [ ] `render_proc::kill_tree` 的 Linux 分支（`yekitrim-server/src/render_proc.rs:87,134`）确认能杀掉
      **孙进程**（ffmpeg 与底座拉起的进程）
- [ ] `open_browser` → `xdg-open`（`yekitrim-server/src/lib.rs:574`）在无桌面/SSH 场景下失败**不能拦启动**
- [ ] `ffmpeg` / `ffprobe` 在 `PATH`（发行说明已要求自备）
- [ ] **Dhampir 底座**的 Linux 构建 + `find_translator` 能找到它

**界面 / 路径**
- [ ] Windows 路径假设：前端占位符与提示里的 `D:/rec`、`\\NAS\rec`、盘符概念
      （`crates/yekitrim-server/src/api/*.rs` 里多处"盘符/UNC"是**防御性**判断，留着无害；
      但**用户看得见的文案**要改成 `/mnt/nas/rec` 这类）
- [ ] 字体栈：tokens / 预览里的 `Microsoft YaHei` 等回退名在 Linux 上不存在（【待实测】会不会导致
      预览与出片不一致 —— 出片走底座 + 内嵌 Noto，预览走浏览器）
- [ ] 文件名**大小写敏感**：Linux 上 `Clip.mp4` 与 `clip.mp4` 是两个文件。所有按扩展名/文件名匹配的地方
      要实测（素材清单、ASR 产物发现、贴纸/音效库）
- [ ] 中文/空格/emoji 路径（本仓此前踩过 GBK 的坑，Linux 是 UTF-8，理论上更简单）

**部署形态**
- [ ] 单机桌面：`.desktop` + 自动开浏览器 + 免登录（§2）
- [ ] 服务器：`systemd` unit（`Restart=on-failure`、`KillMode=control-group`）+ 反代 + HTTPS
      （见 `docs/deploy.md` C 节，那部分与平台无关）
- [ ] `--allow-remote` / `--trust-proxy` 的安全口径照旧（根卡）

---

## 6. 建议的验证流程（到 Linux 上按序跑）

```bash
# ① 先只验"能编译、能起、界面出得来"（先绕过 ASR）
SHERPA_ONNX_LIB_DIR=<你的 linux sherpa lib 目录> cargo build --release -p yekitrim-cli --bins
YEKITRIM_DATA_DIR=/tmp/yt cargo run --release --bin yekitrim -- server --local-no-auth --port 26320
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:26320/api/health   # 期望 200
curl -s http://127.0.0.1:26320/ | head -c 80                                # 期望出 index.html（内嵌那份）
# 启动日志里应有一行：静态资源：内嵌（30 个文件）

# ② 再验深一点的
cargo test --workspace
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
for s in webui/scripts/check-*.mjs scripts/check-preview-audio.mjs; do node "$s" || echo "FAIL $s"; done

# ③ 最后才是真正的管线（要模型 + ffmpeg）
cargo run --release --bin yekitrim -- download     # 下模型
cargo run --release --bin yekitrim -- doctor       # 安全与运行状况摘要
```

⚠️ 到 Linux 上**先不要动** `[patch.crates-io] sherpa-onnx-sys`：删了就会静默链上 GPL 的 espeak-ng
（`vendor/sherpa-onnx-sys/build.rs` 的注释就是这么写的 —— "fail loudly rather than quietly ship a
GPL binary"）。用 `SHERPA_ONNX_LIB_DIR` 那条逃生口。

---

## 7. 与 Windows 侧共享的、不要重复发明的部分

移植时**照用**，别在 Linux 侧另写一套：

- **配置两层合并**：默认 → `.yekitrim/config.toml` → 工程的 `yekitrim.toml`（`yekitrim-core/src/config`）
- **`{patch:{section:{key:value}}}` 写入语义**：保留注释、只改指定键
- **DB 迁移**：`PRAGMA user_version` + `migrate()`（`yekitrim-service/src/db.rs`）
- **跨进程关停契约**：`YEKITRIM_SHUTDOWN_TOKEN` + `POST /api/shutdown`（托盘用的那两条）
- **路径面三防线**（防普通用户读服务器任意文件）：`scope.rs` / `api/config.rs` / 消费侧
- **任务队列的三档与资源键**：`docs/job-queue.md`
- **发布形态**：WebUI 内嵌 + 字体内嵌 ⇒ **包 = exe + 许可文件**（Linux 上同样成立）

---

## 附：本文的事实来源（便于复核）

| 结论 | 出处 |
|---|---|
| WebUI 内嵌 | `crates/yekitrim-server/build.rs`、`src/web.rs`（`choose()` + 四个判据） |
| 托盘整块 Win32 | `crates/yekitrim-cli/src/tray.rs:46` 的非 Windows 分支 |
| Job Object → systemd | `tray.rs` 的 `make_kill_on_close_job`；映射到 systemd `KillMode` |
| `xdg-open` | `crates/yekitrim-server/src/lib.rs:567-575` |
| `/proc` + `kill -0` | `crates/yekitrim-service/src/instancelock.rs:80-88` |
| `df -k` | `crates/yekitrim-service/src/disk.rs:36-48` |
| 杀进程树的 Unix 分支 | `crates/yekitrim-server/src/render_proc.rs:87,134` |
| 进程存活/控制台判断 | `crates/yekitrim-core/src/process.rs:22-56` |
| ASR 只有 win-x64 归档 | `vendor/sherpa-onnx-sys/build.rs:30,145-160` |
| 打包写死 win-x64/.zip | `crates/xtask/src/main.rs:405-407` |
| 数据目录 = exe 旁边 | 根卡「边界」第一条、`docs/file-layout.md` |
