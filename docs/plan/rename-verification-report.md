# 改名方案校验报告（对 `YekiTrim 改名影响清单 + 分步执行方案` 的只读核对）

> **校验方式**：全部结论来自对 `F:\para\Code\YekiTrim` 的只读扫描（glob/grep/read）。
> **未做任何改动**，未执行任何验证命令（原因见 §1）。
> **结论一句话**：方案的方向、顺序纪律、提交粒度都是对的，**但影响清单漏了一整类"跨进程/跨仓持久化契约"**，
> 照它执行会在第 4 步之后出**静默数据断链**。以下是逐条实测。
>
> ⚠️ **口径已更新（第二轮）：本次改名不做任何向后兼容。**
> 旧数据目录、旧环境变量、旧 Cookie、旧邀请码、旧登记路径**一律不迁移、不识别、不作废告警**。
> 这条口径**取消了方案 §2①② 两个决策点**，并把若干"🔴 高风险"降级为普通改名；
> 但也有**一项反而更危险**了（§3.5 的跨仓键名，必须与底座同批改）。
> 修订后的裁决见 **§8**，本节其余内容（§1～§7）为不带该口径的原始实测，仍然有效。

---

## 一、先修环境：`bash` / `pwsh` **全都不可用**（比方案里写的更严重）

方案第 0 步说"本机 bash/pwsh 完全起不来"。实测**确认，且范围更大**：

```
[stderr] bash (71480) ... fatal error - couldn't create signal pipe, Win32 error 5
[exit code] 3221225794
```

- `bash` 调用 → 失败（`3221225794` = `0xC0000142`）。
- **`pwsh` 调用 → 同样失败**，报的是**同一个 Git bash shim 的错**（`bash (54236) ... Win32 error 5`）。

也就是说：**所有 shell 执行都被这个 shim 挡住了，不只是 bash。**
`Win32 error 5` = `ERROR_ACCESS_DENIED`，这是**信号管道创建被拒绝**（沙箱/权限面），不是 PATH 问题。

**对方案的影响**：第 0 步"建基线"以及后面每一步的"验证"命令，现在**一条都跑不了**。
方案把"修环境"列为前置是正确的，但需要补一句：**这是硬阻塞，不是可选优化** ——
没修好之前，第 1～7 步的任何"验证"承诺都是空的（改了也不知道红没红）。

⚠️ 因为跑不了命令，本报告**只做静态核对**：凡涉及"跑一下才知道"的判断（门禁是否全绿、`cargo check` 是否 0 warning、
路由清单是否一致），本报告一律**不背书**，需要环境修好后补跑。

---

## 二、数字核对：大数是对的，但有两个重要偏差

| 方案声称 | 实测 | 判定 |
|---|---|---|
| Rust `yekitrim_*` = **905 处** | **905** | ✅ 完全吻合 |
| `.yekitrim` 路径字面量 = **108 处** | **108** | ✅ 完全吻合 |
| `YEKITRIM_DATA_DIR` = 46 处（其中 `.rs` 18 处） | `.rs` **18** 处吻合；全仓 `YEKITRIM*` 前缀远不止 46 | 🟡 数字对，**口径窄了**（见 §3） |
| 文档 `docs/**/*.md` = **1037 处** | **1392 处** | ❌ **少算约 26%** |
| Cargo.toml 7 个 / 35 处 | 7 个 crate 确认 | ✅ 数量对 |
| `yekitrim.toml` 相关 = "252 处" | `yekitrim.toml` 字面量 **146 处**（`*.rs`） | 🟡 口径不同（252 似含 `yekitrim_home`） |
| `check_module_boundaries.py` "**28 处正则**" | **15 条规则**（14 行级 + 1 成对） | ❌ 数不对，实际正则在 14 条内 |

**为什么 `docs` 少算要紧**：`.md` 是**不产生产物、但会被人照抄**的面。少算 355 处意味着按方案排期会漏一批文件，
而且 `docs/` 里混着**已经交付给底座仓的冻结文档**（见 §4.2），那批**根本不该动**。

---

## 三、🔴 方案最大的漏洞：漏掉了一整类"跨进程 / 跨仓 / 跨重启"的持久化契约

方案的影响清单是按**目录**组织的（`crates/`、`docs/`、`scripts/`、`webui/`），
于是**同一类「写进某个文件、被另一个进程或另一次重启读回」的字符串被完全漏掉**。
这些恰恰是改名最容易出事故的地方：**改错了不报错，只是"东西不见了"。**

### 3.1 `yekitrim.toml` —— 项目配置文件名（146 处，🔴 最高风险）

方案把它和 `.yekitrim/` 一起归进第 4 步、当成"252 处引用中的一个子集"。**这个归类是错的**，
因为 `yekitrim.toml` 和 `.yekitrim/` 的**风险性质完全不同**：

- `.yekitrim/` 是**程序目录下的一个目录**，位置由 `path.rs` 唯一算出 → 迁移面收敛在一处；
- `yekitrim.toml` 是**每个工程目录里的一个文件**，而且它的**绝对路径被持久化存起来**：
  - `crates/yekitrim-server/src/persist.rs:15,18` —— `server.toml` 里存
    `last_config_file` 与 `projects[]`（**已登记工程的 `yekitrim.toml` 绝对路径列表**）；
  - `crates/yekitrim-service/src/db.rs:151` —— SQLite `projects` 表里存 `yekitrim.toml` 路径，
    「目录/yekitrim.toml 已不存在」会显示成 **"缺失"而不是消失**（这正是断链的**症状描述**）；
  - `crates/yekitrim-server/src/api/projects.rs:428` —— 有一段
    `n.eq_ignore_ascii_case("yekitrim.toml")` 的**文件名白名单校验**（"只允许操作 yekitrim.toml"）。

**结论**：改这个文件名 = **同时要迁移 `server.toml` 与 `projects` 表里所有已登记路径**，
外加那条白名单。方案第 4 步完全没有覆盖这一层 —— 只写了"252 处引用"，
会让执行者以为这是个**查找替换**任务。**这是全案风险最高的一处，比 `.yekitrim/` 更高。**

### 3.2 `yekitrim_sid` / `yekitrim_csrf` —— 会话 Cookie 名（🔴 会让所有人当场掉线）

方案**一个字都没提**。实测在 `crates/yekitrim-service/src/auth.rs:30-31` 定义为常量：

```rust
pub const COOKIE_SID: &str = "yekitrim_sid";
pub const COOKIE_CSRF: &str = "yekitrim_csrf";
```

它们是**浏览器里正在生效的 cookie 名**。改名影响：
- 已登录用户**全部静默登出**（旧 cookie 不再被认）；
- CSRF 校验面同步变化 —— 若只改一半（比如改了 `COOKIE_SID` 忘改前端读 `yekitrim_csrf`），
  会变成**写请求全 403**，而页面看起来是"登录着但点不动"。

### 3.3 `yekitrim_inv_` —— 邀请码前缀（🔴 会让存量邀请码失效）

`crates/yekitrim-service/src/auth.rs:51`：`INVITE_PREFIX = "yekitrim_inv_"`，
`new_invite_code()` 生成 `yekitrim_inv_<43 字符>`。方案同样**未提**。

⚠️ 而且这个仓有一条**明确的明文落库**纪律（`yekitrim-service/AGENTS.md`：
*"邀请码明文直接落库（用户要求：管理员要能重复查看、复制再发）"*）——
意味着**库里存着一批 `yekitrim_inv_...` 明文码**。改前缀 = 存量码全部作废，
且**管理员已经在界面上看到、可能已经发出去**。

### 3.4 `YEKITRIM_EXE` / `YEKITRIM_TRANSLATOR` / `YEKITRIM_PYTHON` —— 工具链环境变量（🔴 方案完全遗漏）

方案的 §2② 只讨论 `YEKITRIM_DATA_DIR`，§2③ 只讨论 `VTEDIT_*`。实测**还有第三族**，
住在 `scripts/` 里，且是**文档明确承诺给用户的接口**：

| 变量 | 位置 | 说明 |
|---|---|---|
| `YEKITRIM_EXE` | `scripts/yekitrim-compare.mjs:872` | 指定 `yekitrim.exe`（文档 `docs/dhampir/README.md:62` 写明） |
| `YEKITRIM_TRANSLATOR` | `scripts/yekitrim-compare.mjs:851` | 指定转译器路径（文档承诺） |
| `YEKITRIM_PYTHON` | `scripts/yekitrim-compare.mjs:580` | 指定 python 解释器（本机 `python` 不在 PATH，**这个变量是真有用的**） |

⚠️ `docs/dhampir/compare-loop.md` 与 `docs/dhampir/README.md` 都把
`YEKITRIM_TRANSLATOR` / `YEKITRIM_EXE` 写成**对外承诺的用法**。改它们 = 改对外契约。

### 3.5 `extensions['yekitrim.polish_toml']` —— 写给底座的**文档格式键名**（🔴 跨仓协议）

**这是方案 §2④ 隐约感觉到、但从没点名的那件事。**
`tools/polish-to-dhampir.mjs:3812-3814` 在产物文档里**字面写入**：

```js
extensions: {
  'yekitrim.polish_toml': { ... }
}
```

而 `scripts/check-yekitrim-translator.mjs:804` 又**读回这个键**做行为判据：

```js
return (doc.extensions || {})['yekitrim.polish_toml'] || {};
```

**性质**：这是**交给底座 Dhampir 消费的文档里的一级键名**。它不是"YekiTrim 内部标识"——
方案 §2④ 说"保留 `yekitrim-*` 作为历史名，只改内部标识"，**但没指出"内部标识"里就包含这个跨仓键**。
若把它当"内部"改掉，底座侧读不到扩展元数据 → **静默丢特效/音效清单**（不报错）。

---

## 四、方案中需要修正的**事实性断言**

### 4.1 `ysyk` "不改"的**理由**是错的（结论对，依据错）

方案 §2⑤ 说 `ysyk` 是"`assets/stickers/ysyk_0.9.0/` 的目录名与**观众可见的品牌词**"。

实测：`ysyk` 在代码里是 **46 处**，绝大多数是**测试夹具值与包名常量**
（`sticker_index.rs` 里 30+ 处是 `create_dir_all("ysyk")` 这类测试），
`cli.rs:381` 的注释明说"贴纸**包名**（如 ysyk）"—— 它是**用户可自建的贴纸包名**，不是品牌词。

**为什么要较真**：结论"别改"是对的，但真正的理由是
**它是磁盘上已存在的目录名 + 已写进用户 `polish.toml` 的 `sticker_source` 值**
（`project/config.rs:1148`）。按方案的理由（"品牌词"）去评估，下次可能会有人觉得
"品牌换了，这个该一起换" —— 那就砸了存量工程。**理由要写对，不然守不住。**

### 4.2 `docs/dhampir/` 里有**明确声明"不依赖本仓"的冻结交付物**（不应纳入改名）

方案把 `docs/**/*.md` 整体当成 1037 处待改。实测其中一批**性质相反**：

- `docs/dhampir/base-handoff.md:3` —— *"**这份文档是自包含的**：不需要 YekiTrim 仓"*；
- `docs/dhampir/README.md:7` —— *"逐字节对账过 SHA256，迁移时未改内容"*；
- `docs/dhampir/gap-plan.md:4` —— *"「YekiTrim」= 本仓。读数与命令原样保留，**未改一字**"*。

**这批是交付给底座仓的冻结件**：里面有"迁来时未改一字""SHA256 对过账"的**明文承诺**。
在里面改名 = **让这份承诺变成假话**，而且底座侧可能已经引用了它的措辞。
⚠️ 方案对 `tools/` 提了"与底座同步排期"，但**对 `docs/dhampir/` 这一批只字未提**，属于同类风险漏网。

### 4.3 `YEKITRIM_DATA_DIR` "18 处代码"的**性质**被夸大

方案 §2② 说"18 处代码"要不要保留兼容读。实测 18 处 `.rs` 命中里，
**真正的运行时读取只有 2 处**：

- `crates/yekitrim-core/src/path.rs:30` —— `std::env::var_os("YEKITRIM_DATA_DIR")`（**唯一权威**）；
- `crates/yekitrim-service/src/db.rs:33` —— `std::env::var("YEKITRIM_DATA_DIR")`（**第二处读取！**）。

其余 16 处是注释、文档串、以及**测试里的 `set_var`**。

⚠️ **但这里有个方案没发现的东西**：`path.rs` 的注释写"**唯一的口子**"，
而 `db.rs:33` **自己也读了一遍**环境变量。也就是说这个仓已经有一处
"注释说唯一、实际两处"的漂移。**改名时必须两处一起改**，
否则会出现"配置/模型读了新变量、账本读旧变量" → **账本跑到另一个目录去**
（这正是 `path.rs:26-28` 记录的那类"两个可能位置"事故的翻版）。
**方案只点了 `path.rs`，没点 `db.rs`。**

### 4.4 `check_module_boundaries.py` 的正则数

方案说"`28 处正则`"。实测该文件是 **15 条规则**（`RULES` 14 条 + `PAIRED_RULES` 1 条），
分层相关的正则集中在 6 条里。**方案对风险的判断是对的**
（`yekitrim_service|yekitrim_server|...` 硬编码在 `:94,100,106,112,118,124`，
改名后会**失配 → 假绿**，这比报红危险），只是数字需要订正。

---

## 五、对方案「五个阻塞点」的裁决

| 方案决策点 | 校验意见 |
|---|---|
| ① `.yekitrim/` 迁移策略 A/B/C | **方向对，但排期错**：`yekitrim.toml` 的持久化路径迁移（§3.1）应**并列为第 4 步的第二个阻塞点**，且风险**高于** `.yekitrim/` |
| ② `YEKITRIM_DATA_DIR` 兼容读 | **支持"要"**，但需补：**必须同时改 `db.rs:33`**（方案漏了）；且老变量兼容期要覆盖 `YEKITRIM_EXE`/`YEKITRIM_TRANSLATOR`/`YEKITRIM_PYTHON` |
| ③ `VTEDIT_*` 本轮不动 | ✅ **同意，且实测支持**：约 30 个 `VTEDIT_*` 变量中铁证有 `VTEDIT_DHAMPIR_CLI`（`dhampir.rs:142`）、`VTEDIT_TRANSLATOR`（`:239`）、`VTEDIT_RENDERER`（`pipeline.rs:224`）是**运行时契约**，动它们会直接砸出片 |
| ④ 转译器链保留历史名 | ✅ **同意**，但需补 §3.5：**`extensions['yekitrim.polish_toml']` 这个键必须单独对待**，它是文档协议的一部分，不是"内部标识" |
| ⑤ `ysyk` 不动 | ✅ **同意**，理由需订正为"磁盘目录名 + 用户工程里的值"（§4.1） |

---

## 六、方案**完全遗漏**的项（建议补进影响清单）

| # | 项 | 位置 | 风险 |
|---|---|---|---|
| 1 | **`yekitrim.toml` 已登记路径迁移** | `persist.rs` / `db.rs:151` / `projects.rs:428` | 🔴 断链 |
| 2 | **`yekitrim_sid` / `yekitrim_csrf`** | `yekitrim-service/src/auth.rs:30-31` | 🔴 全员登出 / 写请求 403 |
| 3 | **`yekitrim_inv_` 邀请码前缀** | `auth.rs:51`（**明文已落库**） | 🔴 存量码作废 |
| 4 | **`YEKITRIM_EXE` / `YEKITRIM_TRANSLATOR` / `YEKITRIM_PYTHON`** | `scripts/yekitrim-compare.mjs`（**文档承诺**） | 🟠 破坏对外用法 |
| 5 | **`extensions['yekitrim.polish_toml']`** | `tools/polish-to-dhampir.mjs:3814`（**底座消费**） | 🔴 静默丢特效 |
| 6 | **`db.rs:33` 的第二处 `YEKITRIM_DATA_DIR` 读取** | `yekitrim-service/src/db.rs:33` | 🔴 账本错位 |
| 7 | **`docs/dhampir/` 冻结交付物** | `base-handoff.md` / `gap-plan.md` / `README.md` | 🟠 承诺失真 |
| 8 | **`webui/package-lock.json`** | `name: yekitrim-webui` ×2 | 🟡 改 `package.json` 必须同步（方案只点了 package.json） |
| 9 | **`webui/src` 的 `localStorage` 键** | `yekitrim.ui.coverGuides`（`docs/webui-components.md:446,535`） | 🟡 用户偏好静默丢失 |
| 10 | **`yekitrim.lock` 单实例锁文件名** | `docs/deploy.md:119` | 🟠 改名后可能双实例并存 |

> 第 6 项与第 10 项是同一类：**都是"名字写在注释/文档里、但代码里另有出处"，需要逐个核对**。

---

## 七、结论与建议

**方案可以执行，但不能按现状执行。** 三点修正：

1. **第 0 步不是"修环境 + 建基线"，而是"修环境"** —— `bash` 与 `pwsh` **同时**被同一个 shim 挡死，
   基线命令一条都跑不了。环境修好之前，后续所有"验证"节都是纸面承诺。
   （本报告的静态结论不受影响，但凡是"跑一下才知道"的，需要补跑。）

2. **影响清单要按"契约类别"重排，而不是按目录重排。** 目录视角漏掉了
   §3 那五类**持久化/跨进程/跨仓契约**，其中 `yekitrim.toml`（§3.1）与
   `extensions['yekitrim.polish_toml']`（§3.5）**风险高于方案当前认定的最高风险项 `.yekitrim/`**。

3. **`YEKITRIM_DATA_DIR` 有第二处读取（`db.rs:33`）**，与 `path.rs` 注释宣称的"唯一的口子"矛盾 ——
   这是**改名时最容易踩的静默错位**，方案漏了。

**建议的最小修订**：在方案 §2 增加第 ⑥ 个阻塞点**"持久化契约清单"**，
把 §3 的五类 + §6 的十项登记进去，并在第 4 步里**先做 `yekitrim.toml` 路径迁移、再做 `.yekitrim/` 迁移**。
（⚠️ 该建议写在"要兼容"口径下；**不兼容口径下的修订版见 §8**，第 4 步的顺序结论被推翻。）

---

## 八、🔻 按「不做任何向后兼容」重排后的裁决

> 前置口径（用户拍板）：**旧数据目录、旧环境变量、旧 Cookie、旧邀请码、旧登记路径一律不认、不迁移、不告警。**

### 8.1 风险等级变化（这是本轮最大的结构变化）

| 项 | 要兼容时 | **不兼容后** | 变化 |
|---|---|---|---|
| `.yekitrim/` → `.yekitrim/`（§2①） | 🔴 三选一 + 写迁移代码 + 幂等测试 | **普通改名，零迁移** | ⬇️ **整节作废** |
| `YEKITRIM_DATA_DIR` → `YEKITRIM_DATA_DIR`（§2②） | 🟠 双变量回退 + deprecation | **直接换名** | ⬇️ **整节作废** |
| `yekitrim_inv_` 存量邀请码（§3.3） | 🔴 存量明文码全废、管理员已发出的码失效 | **不是问题**：旧码本就不该再被认 | ⬇️ **降为普通改名** |
| `yekitrim_sid` / `yekitrim_csrf`（§3.2） | 🔴 全员静默登出 + 半改则写请求全 403 | **不是问题**：掉线即预期行为 | ⬇️ **降为普通改名** |
| `YEKITRIM_EXE` / `YEKITRIM_TRANSLATOR` / `YEKITRIM_PYTHON`（§3.4） | 🟠 需保留旧名以不打断脚本 | **直接改**（文档承诺需同步更新） | ⬇️ **降为普通改名 + 文档同步** |
| **`yekitrim.toml` 登记路径（§3.1）** | 🔴 需迁移 `server.toml` + `projects` 表 | **仍是 🔴 最高风险**，但理由变了（见 8.2） | ➡️ **不变** |
| **`extensions['yekitrim.polish_toml']`（§3.5）** | 🟠 可留旧键名，底座照常读 | **🔴 升级为最高风险**（见 8.3） | ⬆️ **上调** |

### 8.2 `yekitrim.toml`：风险**不因放弃兼容而消失**，只是理由换了

放弃兼容**不等于**这一项可以省事。理由从"要不要迁移旧数据"变成
**"新代码读不到自己刚写的值"** —— 三处写入/读取点必须**同时**改：

| 位置 | 作用 | 漏改的症状 |
|---|---|---|
| `crates/yekitrim-server/src/persist.rs:15,18` | `server.toml` 存 `last_config_file` / `projects[]` | 重启后已登记项目消失 |
| `crates/yekitrim-service/src/db.rs:151` | `projects` 表存路径 | 列表显示"缺失" |
| `crates/yekitrim-server/src/api/projects.rs:428` | `eq_ignore_ascii_case("yekitrim.toml")` 白名单 | **导入报 400「只允许操作 yekitrim.toml」，而文件名已叫 `yekitrim.toml`** |

⚠️ **"不兼容"的口径在此处的实际含义**：旧登记记录直接作废、**用户重新导入一次** ——
这是可接受的,但**必须是有意识的选择**,且 `projects.rs:428` 那半改故障**不会**因放弃兼容而免。

### 8.3 ⬆️ `extensions['yekitrim.polish_toml']`：**放弃兼容后反而更危险**

这是全案**唯一一处本仓改不了的依赖**：

- 写入：`tools/polish-to-dhampir.mjs:3812-3814`
- 读回：`scripts/check-yekitrim-translator.mjs:804`
- **消费方在底座 Dhampir 仓**（不在本仓测试覆盖内）

- 要兼容时：可以**留旧键名不动**，底座照常работа,零风险。
- **不兼容后**：键名一改,若底座侧**未同批改**,则**读不到扩展元数据 → 静默丢特效/音效清单,且不报错**。

**唯一要求**：这一项**必须与底座侧同一批提交**,或**先确认底座不读这个键**再改。
⚠️ 上轮报告的 §2④"保留历史名"建议,在**不兼容口径下已不适用** —— 改为**"与底座同步改"。**

### 8.4 不兼容口径下**反而新增**的两条纪律

1. **不允许"半改"**：既然不认旧值,那**每一个旧字符串都必须被找到并改掉** ——
   留一个 = 该处永久失效且**无回退**。这抬高了**扫描完备性**的要求（`docs/` 实测 1392 处而非 1037,少算的那 355 处现在更致命）。
2. **`docs/dhampir/` 的冻结件矛盾加剧**（§4.2）：这批文档承诺"**未改一字**""**SHA256 对过账**"。
   不兼容口径下若不改它们,则文档与代码不一致;改它们,则**已交付的承诺变成假话**。
   👉 **建议**：把这批**移出改名范围并显式标注"冻结于改名前的历史交接件"**,而不是改内容。

### 8.5 修订后的第 4 步顺序（**推翻** §7 的建议）

```
ⅰ  yekitrim.toml → yekitrim.toml   （先做这个：三处登记点 + 白名单同批改，无迁移代码）
ⅱ  .yekitrim/ → .yekitrim/         （普通改名；无迁移代码、无兼容读）
ⅲ  YEKITRIM_DATA_DIR → YEKITRIM_DATA_DIR
        ⚠️ 必须同时改 path.rs:30 与 db.rs:33 两处，否则账本错位
ⅳ  yekitrim_sid / yekitrim_csrf / yekitrim_inv_   （普通改名，无需过渡）
```

**§7 结论仍成立的两条**：
1. **第 0 步"修环境"是硬阻塞** —— `bash` 与 `pwsh` 同时被同一 shim 挡死,基线命令一条都跑不了。
2. **影响清单要按"契约类别"重排,而不是按目录重排** —— 目录视角漏掉了 §3 那五类。
   （其中 §3.3 邀请码与 §3.2 Cookie 在**不兼容口径下已不再是风险**,但**仍必须被扫描到并改掉**,
    否则就是"半改"。）

---

## 九、执行完成后的复核报告（**最终**）

> 本附录是**执行后**的实测收口；上文（§1–§8）是**执行前**的分析稿，保留原样不动。

### 9.1 提交序列（11 个，逐个可单独 revert）

| 提交 | 内容 |
|---|---|
| `3f805b7` | 第 1 步 品牌显示层 |
| `5cbd996` | 第 2/3 步 crate 重命名（216 文件） |
| `95ee2fd` | 第 2b 步 二进制改名补完 + 一处静默失效的进程判据 |
| `7ca52f9` | 第 4 步 持久化契约（唯一动用户数据的一步） |
| `3303d27` | 🔴 P0：token 前缀两端不一致（录播姬整条链 401） |
| `14a5486` | 🔴 边界门禁哨兵正则漏改（门禁静默空转） |
| `2d87b97` | 第 4d 步 7 处用户可见文案指向不存在的路径 |
| `3f0444a` | 第 6 步 脚本 + 环境变量/日志过滤面 + 40 处 CLI 指引 |
| `e4159cd` | 第 7 步 剩余品牌面 + `yekitrim_raw` API 字段 |
| `f2d3475` | 前端 4 处活串 + README/模板 + 跨端不对称 |
| `39a8500` | `cargo test --workspace` 的三处偶发（既有缺陷） |
| `0f2bd8e` | clap `--help` 文案 13 处仍教用户用旧名 |
| `33001ed` | `attempts == 2` 的辩护理由错（复核确定性注入推翻） |

### 9.2 验收线逐条实测

| 验收线 | 结果 |
|---|---|
| `check_module_map.py` 0 违规 | ✅ exit 0（35 模块 / 7 crate） |
| `check_module_boundaries.py` 0 违规 | ✅ exit 0（15 条规则 / 243 文件） |
| `cargo check --workspace --all-targets` 0 warning | ✅ exit 0 / 0 warning |
| **`cargo test --workspace` 全绿** | ✅ **连续 25 轮 EXIT=0**（17 行 `test result: ok`） |
| **路由清单逐行一致** | ✅ **124 条**，只差第 1 行 `# sources:` |
| 全部 node 门禁 | ✅ `.workbuddy/gates.mjs` **全部门禁通过** |
| 打包 | ✅ `release/yekitrim-v0.3.1-win-x64.zip` 37.1 MB，sha256 `d4e5e028…499dd2` |
| 包内二进制 | ✅ `yekitrim.exe --version` → `yekitrim 0.3.1` |
| `webui/dist` | ✅ 重建；`YekiTrim` 5 文件、`V-TRIM` 0、旧串 0 残留 |

### 9.3 本仓原始分析稿**没有预见**、执行中实测新增的六类

1. **`strip_prefix` 型双写字面量**（P0，`3303d27`）：生成端与校验端各写一份字面量，
   改一端 ⇒ **验证永久为假且不报错**。这是本仓最致命的一类 —— `yekitrim` **不含** `yekitrim` 子串，
   所以“全局替换”救不了它。**修法是消灭第二份真源（导出共享常量），不是改字符串。**
2. **门禁自身被改名削弱**（`14a5486`）：规则靠旧字面量认哨兵，改名后**匹配不到任何东西、仍报 0 违规**。
   自证方法：注入新名哨兵 → exit 0（漏）；注入旧名 → exit 1（抓到）。
3. **用户可见文案**（`2d87b97` / `3f0444a` / `e4159cd`）：CLI 在**教用户敲不存在的命令**、
   编辑不存在的路径。4d 只扫了路径、没扫命令名 ⇒ 又扫出 **40 处**。
4. **日志过滤器静默失效**：`RUST_LOG=yekitrim=debug` —— crate target 已是 `yekitrim_*`，
   旧过滤器**匹配不到任何东西**，用户照 LogConsole 提示开日志会**什么都看不到、且不报错**。
5. **跨端不对称**（`f2d3475`）：后端已说新名、前端还说旧名。**扫描域必须包含 `webui/src`**，
   且判据不能只按行匹配（`PolishPreview.vue:1076` 是**跨行模板串**，行级扫描必漏）。
6. **`YekiTrim` 品牌面**：`doctor` 标题、`config init` **写进用户磁盘**的模板首行、单实例提示、503 页 title。

### 9.4 三条**方法论**教训（比代码改动更值得记）

1. ⚠️ **`cargo test` 的“绿”要连跑多轮、且要确认它真的编译了**：
   本轮有一轮“连续 5 次绿”其实 `cargo check` 是红的 —— 跑的是**旧二进制**。
2. ⚠️ **判据必须与源码状态对齐（提交号/时间戳）**：一个“30 轮”任务是在**修之前**启动的，
   它报的红**不能**当作“修后仍红”的证据。
3. ⚠️ **“最像的原因”别急着收工**：`cargo test --workspace` 偶发红，我先判成环境变量竞态（补了锁）
   —— **仍偶发**；继续挖才找到真因是**三个独立缺陷叠加**（serve 退出示警 / script 太短 / pid 复用继承残留）。
   每修掉一个、错误就换一个 —— **读错误消息比读源码更能定位**。

### 9.5 已知且有意的残留

- ⚠️ **`docs/dhampir/**` 的 15 处旧脚本名**：该目录是**冻结的历史交接件**，
  明文承诺“未改一字 + SHA256 对过账”。**用户已裁决不碰** ⇒ 这是**有意保留的不一致**。
- ⚠️ **`release/yekitrim-v0.{1,2.1,3.0,3.1}-win-x64/**`**：历史产物，按冻结口径保留。
- ⚠️ **注释与历史说明里的旧名**：一律保留（它们记的正是改名史）。
- ⚠️ **已登记未修**：`server/api/polish.rs:876` 的 `.yekitrim-preview-{kind}` 锁文件名；
  `server/access_log.rs:46` 的 `/api/ingest/bililive/` 路由前缀双写（与 P0 同型，但属路由非品牌）。