# YekiTrim 改名执行方案（yekitrim → yekitrim）

> ⚠️ **本文件曾两次被失败的脚本写入截断为 0 字节**（2026-10-03 07:19 与 07:36）。
> 两次同一个原因：生成脚本里写了 `'\ud83d\udd34'` —— 那在 Python 里是一个 **UTF-16 代理对**
> （`len == 2`，由 `\ud83d` + `\udd34` 两个码位组成），**不是** 🔴 这个字符（U+1F534）。
> `pathlib.write_text()` 会**先打开并截断文件、再 encode**，于是 encode 失败时内容已经没了。
> 现按会话中的内容与实测事实**重建**；本目录另有 `.workbuddy/safe_write.py` 记录修法
> （先 `encode` 通过再 `os.replace` 原子替换）。原文从未提交 git，无法从版本库恢复。

改名目标：`YekiTrim` → **YekiTrim**（中文「耶斯剪」）。Crate `yekitrim-*` → `yekitrim-*`，
二进制 `yekitrim` → `yekitrim`，配置 `yekitrim.toml` → `yekitrim.toml`，
数据目录 `.yekitrim/` → `.yekitrim/`，环境变量 `YEKITRIM_DATA_DIR` → `YEKITRIM_DATA_DIR`。

**口径**：**不做任何向后兼容** —— 旧目录、旧环境变量、旧 cookie、旧邀请码、旧 localStorage 键
一律**不认、不迁移、不提示**。

---

## 一、执行结果（内容 ✅ 完成 ／ 目录改名 ✅ 完成）

> 分两轮：先做「用户可见面」（步 1~7c，逐个提交、逐步验收），
> 再做「全仓彻底清扫」（步 8，一次 244 文件 / 2590 处）。
> **目录改名已由用户在会话外执行完成**（2026-10-03），改名计划至此**全部收官**。

| 步 | 内容 | 提交 |
|---|---|---|
| 0 | 修环境（`bash`/`pwsh` 曾被 shim 挡死） | （环境操作，无提交） |
| 1 | 品牌显示层 | `3f805b7` |
| 2/3 | crate 重命名（216 文件） | `5cbd996` |
| 2b | 二进制改名补完 + 一处静默失效判据 | `95ee2fd` |
| 4 | 持久化契约（唯一动用户数据的一步） | `7ca52f9` |
| 4b | 🔴 P0：token 前缀两端不一致 | `3303d27` |
| 4c | 🔴 边界门禁哨兵正则漏改（静默空转） | `14a5486` |
| 4d | 7 处用户可见文案指向不存在的路径 | `2d87b97` |
| 6 | 脚本 + 环境变量/日志过滤面 + 40 处 CLI 指引 | `3f0444a` |
| 7 | 剩余品牌面 + `yekitrim_raw` API 字段 | `e4159cd` |
| 7b | 前端 4 处活串 + README/模板 + 跨端不对称 | `f2d3475` |
| 7c | clap 帮助文案 13 处（`--help` 实测可见） | `0f2bd8e` |
| — | 测试偶发三处（既有缺陷） | `39a8500` |
| — | 断言钉错契约（复核对照实验揪出） | `3b606d5` |
| — | `attempts == 2` 的理由错（复核确定性注入推翻） | `33001ed` |
| — | 包内文案/前端串/scripts 扫尾 | `8b88a07` |
| — | 根 `Cargo.toml` 三处旧名 | `b88765e` |
| **8** | **全仓彻底改名**（244 文件 / 2590 处 + 4 个文件改名） | `3c0dd5f` |
| 8b | 发行说明包名/hash 跟改 | `7eedded` |

**验收线实测**（改完后重跑，全绿）：两门禁 exit 0 · `cargo check --workspace --all-targets`
**0 warning** · `cargo test --workspace` **759 passed / 0 failed** · 路由 **124 条逐行一致** ·
`.workbuddy/gates.mjs` 全部门禁通过 · 转译器变异自证 **24/24 打红** ·
打包 `release/yekitrim-v0.3.1-win-x64.zip`（37.1 MB，
sha256 `b303601a…`，冒烟通过，**包内旧名 0**）。

---

## 二、🔴 持久化契约清单（**核心：按契约类别组织，不按目录**）

按目录扫会漏掉跨进程/跨仓/跨重启的那几类。以下六组必须**全数**改到，留一个就永久失效且无回退。

### A 组 — 磁盘文件名（🔴 高）
- `yekitrim.toml` → `yekitrim.toml`（项目配置，**146 处**，最高风险）
- `.yekitrim/` → `.yekitrim/`（程序状态目录）
- `yekitrim.db` → `yekitrim.db`（账本）
- `yekitrim.lock` → `yekitrim.lock`（单实例锁）
- `yekitrim-YYYYMMDD.log` → `yekitrim-YYYYMMDD.log`
- **规范常量**（唯一真源）：`crates/yekitrim-core/src/path.rs` 的
  `PROJECT_CONFIG_FILE` / `STATE_DIR_NAME` / `PROJECT_STATE_DIR_NAME` / `DATA_DIR_ENV`

### B 组 — 网络/认证协议（🔴 高）
- `yekitrim_sid` / `yekitrim_csrf` cookie 名（会让所有人当场掉线）
- `yekitrim_inv_` 邀请码前缀（会让存量邀请码失效）
- `yekitrim_<40>` ingest webhook token 前缀 → `yekitrim_`

### C 组 — 跨进程 / 跨仓协议（🔴 高）
- `YEKITRIM_PROG` / `YEKITRIM_PATH` / `YEKITRIM_META` 三个哨兵
- `YEKITRIM_EXE` / `YEKITRIM_TRANSLATOR` / `YEKITRIM_PYTHON` 工具链环境变量
- ⚠️ `extensions['yekitrim.polish_toml']` —— 写给**底座 Dhampir** 的文档格式键名（跨仓，需协调）

### D 组 — 日志过滤器（🟠 中高，前两轮遗漏）
- `RUST_LOG=yekitrim=debug` → `=yekitrim=debug`（**crate target 是 `yekitrim_cli::*`**，
  旧过滤器**匹配不到任何东西**，用户照提示开日志会什么都看不到且不报错）

### E 组 — 前端本地存储（🟡 中，量最大，前两轮遗漏）
- 12 个 localStorage 键 `yekitrim.*` → `yekitrim.*`

### F 组 — 跨边界字段名（🟠 中）
- `yekitrim_raw` → `yekitrim_raw`（生产端 `server/api/config.rs`、消费端
  `webui/src/views/ProjectSettingsView.vue`、**两条测试断言清单**、卡片 —— 四处同批改）

### 明确**不动**
- `ysyk` ｜ `Dhampir` ｜ `VTEDIT_*`（约 30 个变量）｜ `docs/dhampir/**`（冻结件）
- `release/yekitrim-v0.*-win-x64/**`（历史产物）
- `.workbuddy/**`（gitignore）｜ `.zcode/plans/**`
- 历史说明性文字里的旧名（`xtask/src/main.rs:540,546,553` 等，**有意保留**）

---

## 三、⚠️ 三处「半改即坏」（必须同批改，逐处核对）

1. **`yekitrim.toml`**：`path.rs` 常量 + 实际读写点 + 打包白名单，**三处同批**。
2. **`YEKITRIM_DATA_DIR`**：`path.rs` 与 `db.rs` **各有一份实现**，只改一处会让**账本错位**。
3. **二进制名**：`Cargo.toml` 里 `[package] name` 与 `[[bin]] name` **都要改**
   （`sed '^name = "…"$'` 会同时命中两者，别只改一处）。

---

## 四、⚠️ 不兼容口径带来的额外纪律

1. **不允许「半改」** —— 不认旧值意味着每个旧字符串都必须被找到并改掉。
2. **`yekitrim` 不含 `yekitrim` 子串**（是 `yekitrim`→`ekitrim`）⇒ **「全局替换」救不了 `strip_prefix` 型**，
   必须逐个找 `contains` / `starts_with` / `strip_prefix` / `eq_ignore_ascii_case` 判据。
3. **`docs/dhampir/` 的冻结件**：明文承诺「未改一字 + SHA256 对过账」。
   **用户已裁决不碰** ⇒ 15 处旧脚本名是**有意保留的不一致**。

---

## 五、⚠️ 七类「编译过、测试绿、行为坏」的事故（实测踩出来的）

1. **`strip_prefix` 型双写字面量**（P0，`3303d27`）：生成端 `format!("yekitrim_{…}")`、
   校验端 `strip_prefix("yekitrim_")` 各一份 ⇒ 改一端后**每个 token 恒 401**，
   录播姬采集整条链静默失效（生产中断）。
   **修法是消灭第二份真源（导出共享常量 `TOKEN_PREFIX`），不是改字符串。**
2. **门禁自身被改名削弱**（`14a5486`）：`check_module_boundaries.py` 靠 `YEKITRIM_*` 字面量认哨兵，
   改名后**匹配不到任何东西、仍报 0 违规**。自证：注入 `YEKITRIM_PATH` → exit 0（漏）；
   注入 `YEKITRIM_PATH` → exit 1（抓到）。
3. **用户可见文案**：CLI 在**教用户敲不存在的命令**、编辑不存在的路径。
   （`2d87b97` 7 处路径；`3f0444a` **又扫出 40 处命令名**。）
4. **日志过滤器静默失效**：见 D 组。
5. **跨端不对称**（`f2d3475`）：后端已说新名、前端还说旧名。
   ⚠️ `PolishPreview.vue:1076` 是**跨行模板串**，**行级扫描必然漏**。
   ⇒ **扫描域必须包含 `webui/src`，判据不能只按行匹配。**
6. **`YekiTrim` 品牌面**：`doctor` 标题、`config init` **写进用户磁盘**的模板首行、
   单实例冲突提示、缺 dist 的 503 页 `<title>`。
7. **clap 帮助文案**（`0f2bd8e`）：前几轮只扫了**字符串字面量**，
   漏掉了 **`///` 文档注释** —— 而 clap derive **把 `///` 渲染成 `--help` 正文**。
   实测修前 `yekitrim --help` 印着「初始化视频工程 (创建 `yekitrim.toml`)」
   「需先运行 `yekitrim transcribe`」—— 用户照着敲**命令不存在**、建出来的配置**程序不认**。
   ⚠️ **上一轮的 grep 模式结构性地扫不到它**（只看引号里的内容）。
   ⇒ **扫「用户可见的旧名」时，判据必须覆盖三种来源**：
   ① 字符串字面量 · ② **文档注释（clap 会渲染成 `--help`）** · ③ 跨行模板串。
   **而且判据要跑真命令**（`--help` 实测），不能只 grep 源码。

---

## 六、⚠️ 测试与门禁纪律（逐条对应一次踩坑）

- ⓪ **`cargo test --workspace` 是唯一判据** —— `cargo check` 不跑测试，`cargo test -p X` 会漏 target。
- ① 改名后**跑一条真实端到端路径**，别只看测试。
- ② 猎 `contains("yekitrim")` / `starts_with` / `strip_prefix` / 前缀剥离清单。
- ③ 找出**断言了旧值**的测试。
- ④ 拆开两个 `sed`（`^name = "…"$` 同时命中 `[package]` 与 `[[bin]]`）。
- ⑤ 提防 `target/` 里的**旧产物造假日绿**。
- ⑥ 重跑 `skip()` 型「找不到就 exit 0」的门禁，确认它**真的做了检查**。
- ⑦ ⚠️ **别在他人改动工作区时跑文本解析型门禁**（实测 3 次假红，全是 `git stash` 中间态）；
  **也别把探针写进共享树** —— 用 `git worktree`。**独立复现任何红再动手。**
- ⑧ ⚠️ **跑测试的「绿」要先确认它真的编译了**：有一轮「连续 5 次绿」其实 `cargo check` 是红的，
  跑的是**旧二进制**。
- ⑨ ⚠️ **「最像的原因」别急着收工**：`cargo test --workspace` 偶发红，先判成环境变量竞态（补锁）
  —— **仍偶发**；继续挖才找到真因是**三个独立缺陷叠加**（serve 退出 / script 太短 / pid 复用继承残留）。
  每修掉一个、错误就换一个。**读错误消息比读源码更能定位。**
- ⑩ ⚠️ **改全局环境变量的测试必须持锁**（`set_var` 是进程级的，而 `cargo test` 并行）。
  仓库早有禁令（`yekitrim-service/src/user.rs`），`config/mod.rs` 那条违反了它。
- ⑪ ⚠️ **判据必须与源码状态对齐（提交号/时间戳）**：一个「30 轮」任务是在**修之前**启动的，
  它报的红**不能**当作「修后仍红」。
- ⑫ ⚠️ **断言不该钉错契约**：`LlmClient` 对 `is_connect()/is_timeout()` **会重试**，
  所以 `attempts == 1` 钉的是**传输层行为**，不是「硬失败只记一行」的契约 ⇒ 放宽为 `(1..=3)`。
- ⑬ ⚠️ **「摘掉 X 会红 ⇒ X 必要」是不完整的因果推断**，必须再做「把 X 还原」的**对照组**。
  复核正是靠这个对照实验（摘锁红 C、锁还原**仍红** D）推翻了「那把锁是 `attempts` 偶发解药」的结论。
- ⑭ 🔴 **「跑 N 轮全绿」永远不足以判「清零」**（复核给的方法论，已采纳）：
  它只能给出「概率 < 1/N」的上界。我拿「25 轮 + 60 轮全绿」当清零证据，
  复核反手就用**确定性注入**证明第四条路径仍可达。
  ✅ **正确判据：枚举所有会走 `Err(e)` 重试分支的用例，逐个做确定性注入验证**。
  ① `grep attempts` 找出所有断言 → ② 逐个问「传输层失败时它会怎样」→
  ③ 用注入（如 attempt0 指向死端口）验证它不红。**这比跑 1000 轮可靠得多。**
- ⑮ 🔴 **「它由脚本决定」不等于「它精确」**：我曾为 `send_retries_then_records_attempts`
  的 `attempts == 2` 辩护说「它的重试次数由 script 决定、是确定性的」。**错。**
  重试次数是 **script 与传输层叠加**的：script 只有 2 份、而 loop 最多 3 次尝试 ⇒
  首连失败时会变成 3。自证：在真实 `.post(&url)` 路径注入「attempt0 指向死端口」，
  用旧断言跑出 **`left: 3 / right: 2`**；换新断言同一注入下绿（且打印出 `attempts = 3`）。
  修法：script 备足 3 份 + 断言改 `(2..=3)`。**下界 2 仍钉住「它确实重试了」。**
- ⑯ 🔴 **写文件的脚本必须「先编码、再原子替换」**：
  `pathlib.write_text()` **先打开并截断、再 encode** ⇒ encode 失败时内容已丢。
  本计划文件因此**丢了两次**（同一个 `'\ud83d\udd34'` 代理对；`len==2`）。
  修法见 `.workbuddy/safe_write.py`：`data = text.encode(); tmp.write_bytes(data); os.replace(tmp, p)`。
  **emoji 要用 `chr(0x1F534)` 构造，不要用 `\udXXX` 转义。**

---

## 七、最终状态（2026-10-03 全量清扫 + 目录改名后）

### 全仓旧名字面量 = **0**（改名完成）

本计划书成文时记的「6 行 / 2 文件」负断言，**已从源码中全部移除**，
全仓（排除 `.git` / `target` / `release`）已无任何 `vtrim` / `V-Trim` 字面量。

⚠️ **订正：原文把这两条并称为「旧名必须被拒」的承重负断言，这个归类是错的。**
翻提交史后，两者来由完全不同 —— 其中一条根本不是"旧名必须被拒"，另一条也不是安全防线：

| 位置 | 引入提交 | 它**真正**在防什么 | 改写损失 |
|---|---|---|---|
| `render_proc.rs::is_our_render` | `95ee2fd` | 判据**退化成认不出程序**（不是"旧名被拒"） | **无** |
| `ingest.rs::token_shape_ok` | `3303d27` | 无（见下：是"不兼容口径"的一次性声明） | 仅声明本身 |

**① `render_proc` —— 原文理解反了。** `95ee2fd` 引入时判据是
**`contains("vtrim") || contains("yekitrim")`**，注释写着「**两个名字都要认**、不能只 `contains("vtrim")`」，
因为旧 exe 在改名期间可能还在跑。踩的坑是：`"yekitrim"` **不含** `"vtrim"` 子串（中间是 `ekitrim`），
只判旧名 ⇒ 改名后判据恒 false ⇒ `sweep_orphans` 认不出遗留渲染进程、
ffmpeg / 底座孙进程**永不被回收**（文件被占用且无任何报错）。
所以它测的是**「判据不能认不出自己的程序」**。`3c0dd5f` 全仓清扫删掉旧名半边时旧 exe 早已不存在，
是合理收尾。⇒ **当前改写为「缺 polish / 缺 render·cover」两条，语义无损、覆盖反而更全。**

**② `ingest` —— 真正的防线从来不是这一条。** `3303d27` 修的 P0 是生成端/校验端前缀不一致
（`"yekitrim_…".strip_prefix("vtrim_")` → `None` ⇒ 每个 token 恒 401 ⇒ 录播姬采集整条链不可用）。
修法是「**消灭第二个真源**」，因此真正的防线是这三条，且**全部完好**：
（a）两端共用 `TOKEN_PREFIX`（`service/src/ingest/mod.rs:39`）；
（b）断言 `generate_token()` 产物以 `TOKEN_PREFIX` 开头（`ingest.rs:1724-1730`）；
（c）测试**一律用常量拼**、不写字面量前缀（老测试写死旧值 ⇒ 它"保护的就是那个坏行为"）。
至于「旧前缀 `vtrim_` 必须被拒」那一条，按当时的 commit 说明只是**顺带记录"不兼容口径"**，
**不是安全防线**。

### 用户裁决：不保留任何"旧名被拒"的钉子（2026-10-03）

> 「不需要兼容过去和服务端在运行的考虑，当前是开发环境，总之不需要考虑任何过去内容的兼容。」

⇒ **那两条断言无需恢复**，且**不应**恢复：
- 旧 exe 不存在 ⇒ `is_our_render` 无理由再认旧名（认了反而放宽判据）；
- 旧 prefix 不会再出现 ⇒ 钉住它是**过期的声明**，不是防线。

**反证实验（坐实②的"非防线"判断）**：把 `service/src/ingest/mod.rs:39` 的 `TOKEN_PREFIX`
**临时改回 `"vtrim_"`**，重跑 `cargo test -p yekitrim-server --lib token_shape` ⇒ **仍然 ok**。
即该断言改前钉的也只是那个具体旧值，与 P0 的真实防线（a/b/c）无关。已还原，本 crate `git diff` 为空。

**反向对照（同批实测，证明防线仍然有效）**：把校验端 `token_shape_ok` 的
`strip_prefix(TOKEN_PREFIX)` **临时退回 `strip_prefix("vtrim_")`**（= 复现 P0 那个原始 bug），
重跑同一用例 ⇒ **FAILED，panicked at `ingest.rs:1724`**（正是防线 (b) 那条钉子）。
两实验对照说明：**P0 仍被牢牢钉住，被删掉的只是那条过期的"旧前缀声明"。** 已还原。

> **口径一致性**：这与 `path.rs:170` 的既有表述同调 ——
> 「状态目录就一个名字：`.yekitrim`。**旧名一律不再被创建**（无兼容期）」。无兼容期 ⇒ 无需为旧值留念。


### 全仓计数（`--exclude-dir=.git --exclude-dir=target`）

| 范围 | 旧名字面量 |
|---|---|
| `crates/` | 0 |
| `AGENTS.md` 各卡 | 0 |
| `docs/`（含历史档案，**本计划书自身除外**） | 0 |
| `scripts/` `tools/` `webui/src` `webui/scripts` | 0 |
| `.gitignore` + `webui/.gitignore` | 0 |
| `release/`（产物与包名） | 0 |
| **文件名/目录名带旧名（全仓）** | **0** |
| `.workbuddy/`（gitignored） | 仅**二进制**残留（Chrome 缓存 / 旧 exe / db-wal） |

本计划书自身仍保留旧名字面量（引述用），这是**有意**的 —— 它是历史记录，不是源码。

### 本轮额外清掉的（都是可再生的垃圾，不是源码）

- `.workbuddy/target-dhampir/**` —— 1.8 GB 陈旧 cargo 缓存（每个 `.d` 记着旧 crate 路径）。
- `.workbuddy/anim-verify/chrome-profile{,-2}` —— 391 MB 一次性 Chrome 配置缓存。
- `.workbuddy/**` 里 197 个文本草稿 + 51 个带旧名的 fixture 路径（已改名）。
- `release/vtrim-v0.{1.0,2.0,2.1,3.0,3.1}-win-x64*` —— 14 个历史产物（~430 MB）。

### 冻结项：两条都按用户裁决**改掉了**

- `extensions['yekitrim.polish_toml']`（跨仓协议键，`tools/polish-to-dhampir.mjs` 4 处）—— 用户拍板「也改」。
  ⚠️ **底座 Dhampir 若按老键读会接不上**，这条要在跨仓对接时同步。
- `docs/vtrim-integration.md` → `docs/yekitrim-integration.md`（文件已 `git mv`，全仓引用已跟改）。

### ✅ 已完成：仓库**目录名**（原「唯一未完成项」）

`F:\para\Code\V-Trim` → `F:\para\Code\YekiTrim` **已由用户在会话外执行成功**。

原阻塞原因（存档）：Windows 报 `另一个程序正在使用此文件，进程无法访问`——
持有者是**当时的 DSH 会话本身**（工作区根 = 会话 CWD，进程活着就占着），**在会话内部无法释放**，
所以只能等会话退出后在外部执行。

**改名后核查（本轮实测）**：
- 全仓**无任何硬编码 `Code/V-Trim` 绝对路径**（`grep` 排除 `.git`/`target` 后为空；
  仅 `.workbuddy/` 下 gitignored 的二进制残留命中）。✅
- `git remote` 是服务端地址（`…/sinsa/V-Trim.git`），**不受本地目录名影响**，无需改。✅
- 两条门禁 exit 0；`cargo test -p yekitrim-server --lib render_proc` 5 passed / 0 failed。✅


### 🟡 登记但有意未改（与品牌无关，各自单独一批）

- `server/api/polish.rs` 的 `.yekitrim-preview-{kind}` 锁文件名（改它=改锁路径）。
- `server/access_log.rs` 的 `/api/ingest/bililive/` 路由前缀双写（与 P0 同型，但属**路由**非品牌）。
- `check_module_map.py -v` 在 GBK 控制台崩 `UnicodeEncodeError`（既有缺陷）。

## 八、⚠️ 不兼容口径下**用户可见的代价**（必须让用户知道）

**旧 `.yekitrim/` 的逐项目检查点日志全部失效** ⇒ 存量工程要**从头重跑 `polish llm`**。
在长片子上这是**真金白银的 LLM 花费**。这是「不做兼容」的**预期代价**，不是 bug。
