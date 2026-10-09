# 底层身份维度 + local 模式（计划书 v3 · 实施版）

> 状态：**已定稿待实施**。拍板口径（用户 2026-10-10）：
> 「先做底层设计方便后续能将多用户数据分离进行处理，然后 tray 也就是 local 模式，
>  单独设计而且复用」→ 追问后明确：**多用户补齐到能用**。
>
> v1（拆除多用户）已废；v2（只留通道）已升级为本稿。

## 一、目标与成功判据

**目标**：建一条**唯一的**身份→数据通道，并让 `Owner::Local`（托盘/CLI）与
`Owner::User(uid)`（多用户）**都真正可用**、共用同一套业务。

**成功判据（可验证）**：

| # | 判据 |
|---|---|
| 1 | 单人下目录形状**一个字节不变**，现有 **12 个项目照常打开** |
| 2 | 从身份到数据**只有一条通道**（业务不许自己拼路径，机器判据钉住） |
| 3 | **多用户真能用**：能建第二个账号，两个账号的**素材库/配置/项目互不干扰** |
| 4 | 托盘仍是 **0.7 MB**（不链接任何业务 crate） |
| 5 | 两条门禁 + 全量测试通过；`cargo check` 0 警告 |

**明确不做**：删 auth/invite/scope、Tauri/Electron/IPC 壳、托盘链接业务 crate。

---

## 二、摸底结论（均已核实，非推测）

| 事实 | 证据 |
|---|---|
| 身份→路径**只有一条通道** | `tenant_root(uid)`，server 侧仅 **4 个调用点** |
| 配置**无 uid 维度** | `load_global_config()` 只读 `<data>/.yekitrim/config.toml` |
| 项目**已有** owner 维度 | `projects.owner_id` + `idx_projects_owner` 索引 |
| 托盘**已在用** local 模式 | `tray.rs:349` 传 `--local-no-auth` |
| 托盘**必须薄** | 0.7 MB vs 103.8 MB（**148 倍**），链接业务会被 ASR 拖成上百 MB |
| `quota_bytes` **只存不查** | 全仓无"超磁盘配额就拒绝"的代码；`quota_exceeded()` 是**任务并发**限制 |
| 素材库**有口头矛盾** | `stickers.rs`「用户级」vs `api/AGENTS.md`「程序级」 |

### 2.1 三类数据能不能按用户分（这是"补齐到能用"的核心裁决）

| 数据 | 能否按用户分 | 理由 |
|---|---|---|
| **账本**（`users`/`sessions`/`invites`） | ❌ **绝对不能** | 它是**身份的来源**。分用户 = 登录前得先知道你是谁 = 循环依赖 |
| **项目登记**（`projects` 表） | ✅ 已用 `owner_id` 字段（**不按文件分**） | 一个账本、一张表、按行归属。这是**正确**形状，保持 |
| **素材库**（`resources/*`） | ✅ 可按目录分 | 纯文件、无交叉引用、无 DB 外键 |
| **配置**（`config.toml`/`llm.toml`） | ✅ 可按文件分 | 无外键；`llm.toml` 含 key，分用户是**安全净改善** |
| 模型 / `runtime/` / 日志 | ❌ 全局 | 共享算力 / 只读交付物 / 运维面 |

**⇒ 关键设计结论：账本永远全局，项目按行归属，只有素材库与配置按目录/文件分。**

这条裁决消掉了上一轮卡住的"配额""迁移"大半问题（见 §六）。

---

## 三、设计

### 3.1 身份抽象（新增，放在 `yekitrim-service`）

```rust
/// 数据归属。单人部署下恒为 `Local`。
pub enum Owner {
    /// 本机唯一使用者（托盘 / CLI / --local-no-auth）
    Local,
    /// 多用户模式下的某个账号
    User(i64),
}
```

**`Owner` 只回答"是谁"，不回答"在哪"。**

### 3.2 路径只有一处算

```rust
pub fn owner_root(owner: &Owner) -> PathBuf;
pub fn asset_dir(owner: &Owner, kind: AssetKind) -> PathBuf;  // fonts/stickers/videos
pub fn config_path(owner: &Owner) -> PathBuf;
```

| | `Owner::Local` | `Owner::User(uid)` |
|---|---|---|
| `owner_root` | `<data>/`（**现状不变**） | `<data>/users/<uid>/` |
| `asset_dir` | `<data>/resources/<kind>/`（**现状不变**） | `<data>/users/<uid>/resources/<kind>/` |
| `config_path` | `<data>/.yekitrim/config.toml`（**现状不变**） | `<data>/users/<uid>/.yekitrim/config.toml` |

**`Owner::Local` 的每一行都等于今天的实际位置 ⇒ 单人零迁移。**

### 3.3 身份从哪来（唯一分叉点）

| 调用者 | `Owner` |
|---|---|
| CLI | `Local` |
| server `--local-no-auth`（托盘） | `Local` |
| server 正常模式 | `User(CurrentUser.id)` |

### 3.4 三条硬规矩（违反即判负）

1. **只有一条通道** —— 不许再出现 `data_dir().join("resources")` 这类自己拼的路径
2. **不许"候选位置 + 逐级回落"** —— `file-layout.md` 那次事故正是这个形状
   （CLI 看一份字体库、WebUI 看另一份，**界面看不出异常**）
3. **不许业务分支** —— 一旦出现 `if local { A } else { B }`，两个形态就漂移

### 3.5 ⚠️ 保留项（最容易误删，单列）

**越权模型变化 ≠ 输入校验可以取消。** 这三样防的是**运行在本机浏览器里的恶意网页**
（如提交 `../../.yekitrim/llm.toml` 读走 API key），**不是"别的用户"**，两种形态下**都需要**：

- `scope.rs::is_safe_component`
- `cover.rs::tenant::ensure_within`
- `guard_local_request`（Host 必须 loopback + 写请求同源）

计划补一条"恶意文件名仍被拒"的单测钉住。

---

## 四、实施批次（每批可独立验证，绿了才进下一批）

### 批 1：身份地基

**写作用域**：`crates/yekitrim-service/src/`（新增模块 + `AGENTS.md` 登记）

- 加 `Owner` + `owner_root` / `asset_dir` / `config_path`
- `Owner::Local` 解析到**现有位置**

**验证**：`cargo test -p yekitrim-service` + 两条门禁
**判据单测**：① 同一 `Owner` 只解析出**一个**位置，无候选列表
② `Owner::Local` 结果与今天的 `data_dir()/resources/*` **逐字节相等**

### 批 2：收口调用点 + 标注身份

**写作用域**：`crates/yekitrim-server/src/`

- 4 处 `tenant_root(uid)` 改走新通道
- `--local-no-auth` 合成身份标成 `Owner::Local`
- **非** local 模式下把 `CurrentUser.id` 传成 `Owner::User`

**验证**：`cargo test -p yekitrim-server`（188 条基线）
**判据**：现有测试**全绿且不改断言**（改断言 = 行为变了，要单独说明）

### 批 3：素材库与配置真分用户

**写作用域**：`crates/yekitrim-server/src/`、`crates/yekitrim-polish/src/assets/`

- 素材库三个库的解析点改走 `asset_dir(owner, kind)`
- 配置读写改走 `config_path(owner)`
- 收口 `stickers.rs` 与 `api/AGENTS.md` 的口头矛盾：统一成「**归属 `Owner`，单人下即全局**」
- 删掉 `stickers.rs` 里"别再单方面加回 admin 门控"（描述的是已不成立的中间态）

**验证**：两条门禁 + 全量测试

### 批 4：多用户可用性验证（判据 3 的核心）

- 建第二个账号 → 验证两个账号的素材库/配置/项目**互不干扰**
- 验证越权仍被拒（甲改不了乙的素材）

**验证**：真机跑服务 + 手工核对

### 批 5：文档 + 真机回归

- `file-layout.md` / `deploy.md` / 相关 `AGENTS.md`
- 托盘起 → 界面可用 → 素材库/项目列表/精修全通 → **12 个项目能打开**

---

## 五、流程

```
批1 身份地基 ─→ 批2 收口调用点 ─→ 批3 素材/配置分用户 ─→ 批4 多用户验证 ─→ 批5 文档+回归
    service 测      server 188        两条门禁+全量          真机双账号        托盘+12 项目
      └──────────────┴─────────────────┴─────────────────────┴──────────────────┘
                          每批各自跑全量,绿了才进下一批;每批一个提交
```

---

## 六、风险与已消解的问题

| 风险 | 应对 |
|---|---|
| 误删输入校验（§3.5） | 单列 + 补单测 |
| 门禁连锁（17 条边界规则） | 每批跑 |
| 指令链预算（57344 B 已很紧） | 边改边跑 `check_module_map.py` |
| 托盘被"顺手消重" | 保留 `tray.rs:255` 刻意的第二实现（代价换 148 倍体积） |
| 用户级素材**不计配额** | `quota_bytes` 今天**只存不查**（已核实）⇒ 本次**只记录、不新增配额机制**；用户级素材成为"可写不计量"的磁盘，需**单独决策**（见 §七） |

**上一轮卡住、本稿已消解的问题**：
- "老库迁移" → `Owner::Local` 解析到现状，**单人零迁移**
- "回落语义 × 用户维度组合爆炸" → 硬规矩 2：**不许回落**
- "三重身份来源" → 账本必须全局（§2.1），只有素材/配置分

---

## 七、需要用户拍板的遗留决策

1. **用户级素材是否计入配额** —— 今天 `quota_bytes` 只存不查。不计量则单个用户可写满磁盘。
2. **`Owner` 命名** —— 也可叫 `DataScope` / `Tenant` / `Profile`。
3. **托盘要不要有界面级配置**（如"切换数据目录"）—— 若要，它是托盘**唯一**该碰路径的地方，需单独设计。
