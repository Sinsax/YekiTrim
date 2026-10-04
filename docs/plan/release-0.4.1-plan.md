# v0.4.1 发行计划（✅ 已发布）

| 项 | 值 |
|---|---|
| 目标版本 | **v0.4.1**（补丁版；上一版 v0.4.0，2026-10-03） |
| 状态 | ✅ **已发布**（2026-10-05）—— Gitea Release `v0.4.1` 已建，附件 zip + `.sha256.txt` 已传，**详情与 Linux 侧复核见下方「发布结果」** |
| 发行说明 | `docs/releases/v0.4.1.md`（`package` 会拷进包内成 `RELEASE-NOTES.md`） |
| 适配测试 | `docs/plan/linux-adaptation-test-report.md`（Linux 侧实测报告） |
| 主题 | 当初只登记了「WebUI 换肤」；实际发布时 `v0.4.0..HEAD` 已累计 **51 个提交** |
| 平台 | Windows x64 |

## ⚠️ 实际范围远大于"补丁"（2026-10-05 补记）

这份计划只登记了换肤那几条，但换肤之后又落了 40 多个提交（托盘薄启动器、设置页重排、
配置版本号与读时迁移、WebUI 内嵌进 exe、三类新门禁……）。其中**两处会动到用户手上的东西**：

1. **配置文件第一次读取时会被自动迁移一次**（补 `config_version` + 清 4 个死键；
   程序级文件先备份 `.bak-v0`，工程级文件不写回）；
2. **发行包里不再有 `webui/dist/`** —— 升级从此只需换 exe。

两处都已写进 [发行说明](../releases/v0.4.1.md) 的开头。若要把版本号改成 **v0.5.0**，
只需改 `Cargo.toml` 的 `[workspace.package] version` 再重打一次包。

## 为什么是补丁版号而不是 0.4.0 重打包

用户拍板（2026-10-03）：**不重新打 v0.4.0 的包**，换肤作为 0.4.1 发布。

- `release/yekitrim-v0.4.0-win-x64.zip`（已发到 Gitea）里是**换肤前**的粉色界面，**保持原样不动**。
  发行包一旦发布就不再改动 —— 改了 hash 就对不上已公布的 `.sha256.txt`。
- 换肤**不改任何对外契约**（接口/配置/数据格式/CLI 参数全没动，CSS 变量**名**也没改），
  所以补丁版号在语义上说得通。

> ⚠️ 历史记录订正：`a6b7d86` 当初写成 `feat(webui)!:`，那个 `!` **是错的**，
> 已由 `d3b6462` 空提交订正。该提交已推送，没做 force-push 改写共享历史。

## 本版包含的改动（`v0.4.0..HEAD`）

| 提交 | 内容 |
|---|---|
| `ba5cda8` | 文档：Gitea 发布纪律 + 两处遗留的解决结果 |
| `a6b7d86` | 换肤①：粉 → 砖红 + 沙/驼 |
| `d3b6462` | 文档：订正换肤提交的类型标记 |
| `d5399aa` | 设计：哥特双方案预览 + 核验记录 |
| `85209f2` | **换肤②：哥特橙瞳 + 暗色模式（本版主体）** |
| `261259d` | 修 `followSystemTheme` 被 setTheme 写回 localStorage 的锁死缺陷 |

> ⚠️ **本版换肤做了两次**：先换砖红（`a6b7d86`），后按立绘特征色换成**哥特橙瞳并为暗色模式**
> （`85209f2`）。两次都在 0.4.0 之后、都未发布，所以**本版的最终外观是哥特橙瞳那一版** ——
> 写发行说明时以 `85209f2` 为准，别再描述砖红。

## 换肤要点（发布说明素材）

**三个参考色**（用户给定）：`#cab5a0` 沙 / `#cba98e` 驼 / `#d86353` 砖红。
**角色分配**：砖红当主色，沙/驼当底色与辅助。

⚠️ **为什么不能反过来**：实测沙色/驼色**压白字对比度只有 1.98 / 2.19**（WCAG 图形需 3.0），
做按钮底色会让白字糊掉；只有砖红能到 **3.60**。

| 角色 | v0.4.0 | **v0.4.1** |
|---|---|---|
| `--main` | `#F682B8` | **`#d86353`** |
| `--main-deep` | `#EE6FA6` | **`#B15144`**（暗化 0.82，文字色专用；压暖白 **4.81 过 AA**） |
| `--main-soft` | `#FDE3EC` | `#FAECEA` |
| `--main-glow` | `#FFD3E1` | `#F5D6D2` |
| `--bg` | `#FFF7F3` | `#F7F3F0` |
| `--bg-soft` | `#FFEFE7` | `#F0EAE4` |
| `--line` / `--line-soft` | `#F2E1D9` / `#FAEEE7` | `#E7DED4` / `#EFE9E2` |
| `--ink` | `#4A3F45` | `#574A40` |
| `--ink-dim` | `#9C8B92` | **`#77685B`**（原值压暖白仅 2.6，**不达标**，顺手修） |
| `--ink-faint` | `#C9B8BE` | `#A79483` |
| 新增 | — | `--tan` `#cba98e` / `--tan-soft` `#F6EFE8` / `--tan-deep` `#8C6F57` |

⚠️ **三个变量名保留但语义已变**（改名会牵动 27+ 处引用，风险大于收益）：
`--pink-light`（现=砖红浅调）/ `--pink-text`（现=砖红文字 = `--main-deep`）/
`--shadow-pink`（现=砖红投影）。

**品牌标**（新增，此前只有字母「Y」）：暖白圆角方 + 砖红 Y + 沙色下竖（切片意象）。
favicon 与登录页/侧栏用同一套 SVG 几何；圆形容器渐变 `--main → --pink-light` 改为 `--main → --tan`。

**顺带清掉的 9 处硬编码**（这类值绕过 `tokens.css`，是换肤必漏点）：

- 旧粉三种写法：`rgba(238,111,166,·)` / `rgba(246,130,184,·)` / `#f682b84d`
- 旧**冷褐**中性色：`rgba(90,60,70,·)` / `rgba(74,63,69,·)` / `#4a3f4552`
- `rgba(255,247,243,.85)`（`App.vue` 吸顶面包屑底）、`#3b2f36`/`#6b4a58`（`.player` 渐变）

## 发布时要做的事（检查清单）

1. ✅ `Cargo.toml` 的 `[workspace.package] version` → `0.4.1`（7 个 crate 靠 `version.workspace = true` 继承）
   —— **已完成**（2026-10-05）。三处版本消费点已实测一致：CLI `--version` / `/api/health.version` /
   打包名（xtask 从根 Cargo.toml 读）**全部为 0.4.1**。
2. ✅ 写 `docs/releases/v0.4.1.md`（素材见上）—— 已完成
3. `cargo check --workspace --all-targets` + `cargo test --workspace`
   ⚠️ **基线按平台不同**：Windows 上调基线是 **759 passed**；**Linux 上实测 776 passed / 3 failed** ——
   差的 3 条是**测试侧的 Windows 硬编码**（`path.rs` 一条被改名写反的判据、
   `workspace.rs` 用 `C:/` 当绝对路径、`instancelock.rs` 只认 Windows 那条错误文案），
   **非产品缺陷**，详见 Linux 报告的 §2。在 Linux 上跑第 3 步要按"776/3、失败项就是那 3 条"判读。
   ⚠️ **Linux 上 `cargo check` 有 1 条 warning**（`xtask` 的 `CREATE_NO_WINDOW` 只在 `#[cfg(windows)]` 用），
   本仓要求 0 warning ⇒ 在 Windows 上跑第 3 步才是"0 warning"的判定环境。
4. ✅ 两条门禁 + `npm --prefix webui run build`（确认 dist 是最新）+ 8 个前端契约脚本 —— Linux 上全过
5. ✅ `cargo xtask package --smoke-test` —— **已在 Windows 侧执行**（`eb66d1a`）
   （Linux 侧无法代劳：`package` 写死 `.exe`/`win-x64`/`.zip`，`scan` 在 Linux 上因扫不到 ELF 而 exit 3，
   且本机没有 Windows target / mingw / wine）
6. ⚠️ **核对包内界面确实是新皮肤**（v0.4.0 的教训：包是换肤前打的）—— Windows 侧已核对
7. ✅ `git tag -a v0.4.1` → push tag → Gitea 建 Release 并传 zip + `.sha256.txt` —— **已完成**

### ✅ 发布结果（2026-10-05，已在 Linux 侧独立复核）

| 项 | 值 |
|---|---|
| 提交 | `eb66d1a`（Windows 产物补打）；tag `v0.4.1` 指向它 |
| Release | Gitea `v0.4.1`「YekiTrim v0.4.1」，非 draft / 非 prerelease |
| 附件 | `yekitrim-v0.4.1-win-x64.zip`（39,861,473 B）+ `.sha256.txt` |
| sha256 | `5c3ea7d0…a9d39138` —— **下载实物复算一致**（与 sidecar、与提交声明三方吻合） |
| 包内 | 27 项；`yekitrim.exe` 71.9 MB / `yekitrim-tray.exe` 520 KB / `dhampir/` 全套 / 许可三件齐 |

**Linux 侧下载实物独立复核**（不只信提交说明）：

- ✅ **sha256 复算一致**（下载 zip 后 `sha256sum` 与 sidecar 逐字符相同）；
- ✅ **GPL 红线**：拿 `xtask scan` 自己的 5 个 MARKERS 扫包内
  `yekitrim.exe` / `yekitrim-tray.exe` / `dhampir.exe` —— **全部 0 命中**
  （这正是 `cargo xtask scan` 在 Linux 上空转时**查不到**的那一步，此处补上了）；
- ✅ **烟测产物未进包**：无 `.yekitrim/`、无 `stickers/`（配置/账号库/初始密码/真实绝对路径都没漏）；
- ✅ **`webui/dist` 已从包中消失**（前端编进 exe，符合本版第 2 条变更）；
- ✅ **不随包带字体**（仅 `resources/fonts/放这里.txt`，授权原因）；
- ✅ `LICENSE` / `THIRD-PARTY-NOTICES.md` / `RELEASE-NOTES.md` 三件齐。

> ⚠️ 包内 `RELEASE-NOTES.md`（sha256 `4f258107…`）与仓库 `docs/releases/v0.4.1.md`
> **在 Windows 补打时同步更新过**（状态行、提交数 50→51、包内容清单改为 27 项）—— 属正常，
> 包内那份才是随包交付的最终版。

### 📌 Linux 侧在这轮做了什么（供回溯）

Linux 侧推了 `c0f7c20`（版本号提到 0.4.1 + `docs/plan/linux-adaptation-test-report.md`），
并**刻意没有建 tag / Release** —— 因为产物在 Linux 上造不出来，而 tag 时间戳不可逆
（见下方纪律）：先建空 Release 再补附件的话，tag 时间已经定死、只能 force-push 重写。
Windows 侧随后按这个顺序补打，结果如上表。

> ✅ **发行说明同源**：`package` 会把 `docs/releases/v<版本>.md` 自动拷成包内
> `RELEASE-NOTES.md`（`xtask/src/main.rs:619-624`）。版本链条已核对完整：
> 根 `Cargo.toml` = `0.4.1` ⇒ 查到 `docs/releases/v0.4.1.md`（文件自身也声明 v0.4.1）。

> ⚠️ **Gitea 发布纪律**（见 `rename-execution-plan.md`「Gitea 发布纪律」）：
> Release 列表按 **tag 的 tagger 时间**排序；`PATCH /releases/{id}` **改不动时间**，
> 唯一有效修法是重建 tag（`GIT_COMMITTER_DATE`）再重建 Release。
> 这正是不建议"先建空 Release 再补附件"的原因 —— 补的时候 tag 时间已经定死了。


---

## 附：暗色模式（2026-10-03 新增）

亮色（哥特浅色）默认，暗色（哥特暗色）在 `:root[data-theme="dark"]`；入口在侧栏底部。

**⚠️ 暗色不是简单反色，有三处结构性差异（发行说明要点）：**

| 项 | 亮色 | 暗色 |
|---|---|---|
| 主按钮文字 | 白字（压橙 3.25，大字级） | **深字** `#202020`（压橙 **5.02**，正文级） |
| 主色渐变 | `--main → --main-deep` 压深 | **提亮**（`#F88858`，像发光） |
| `--main-deep` | `#904028` 压深（当文字色） | `#F5B89E` **提亮**（方向相反） |
| `--main-soft` | 实色 `#FCEDE7` | **透明叠加** `rgba(232,104,56,.16)` |
| 阴影 | 投影 | **描边 + 内发光** |

**验证口径（发布时按这个跑）**：双主题变量必须 **42:42 逐变量对等** —— 漏一个会静默继承
亮色的浅色值，暗底上直接看不见，而**构建与契约脚本都不会报错**（这是最阴的坑）。

**已知未做**：`followSystemTheme`（交还系统偏好）虽已修好逻辑，但**界面上还没有入口**
（当前只有 toggle 浅/暗）。若要暴露，加一个下拉三选（浅/暗/跟随系统）即可。
