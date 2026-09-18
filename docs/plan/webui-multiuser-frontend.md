# WebUI 多用户前端设计（配合 multi-user-server-design）

> 状态：设计稿。服务端依据：`docs/superpowers/specs/2026-09-12-multi-user-server-design.md`（§号沿用该文）。
> 前置：单用户工作台重构（`docs/plan/webui-workbench-redesign.md`，下称"工作台方案"）先行落地；本文定义其上的多用户前端增量，随服务端 P1–P5 分阶段交付，每阶段可独立上线。
> 复用原则：**壳不变**——侧栏/抽屉/toast/dialog/token/三段流水线页全部复用，多用户只加"视图 + store + api 模块"，并启用工作台方案已预留的接缝（工作台方案 §11）。
>
> **交互基准**：预览样稿 `docs/web-ui-mockups/ia-demos/multiuser.html`（与本文件同步迭代；右上角角色切换为预览辅助，正式实现由 `/api/me.role` 驱动）。基础三页（预处理/切片/精修）的最终规格见 `webui-workbench-redesign.md`（v5 起与样稿同步）。

## 0. 阶段总览（前端 M1–M5 ↔ 服务端 P1–P5）

| 阶段 | 服务端交付 | 前端交付 | 新增面 |
|---|---|---|---|
| M1 | P1 账号/会话 | 登录/会话流 | `LoginView`、auth store、路由守卫、侧栏用户区、client.js 401/CSRF 启用 |
| M2 | P2 项目托管/隔离 | 作用域切换 + 项目列表升级 | `?config=`→`?project=`（单点）、项目行来源/占用徽标、列表过滤、总览页来源区块、删除源录播 |
| M3 | P3 录播姬接入 | 录制导入 + 自动导入规则 + 待分配 | WorkbenchView 首页区块、`#/ingest` 页、ingest store/api |
| M4 | P4 B站通道 | 新建向导多来源 | 向导 source 抽象启用：入库记录/B站链接两来源、resolve 预览、配额预检 |
| M5 | P5 容量/运维 | 存储页 + 管理页 | `#/storage`、`#/admin`（tabs）、全局设置 admin 化 |

## 1. 认证与会话（M1）

### 1.1 启动引导（App 挂载第一个请求）
```
GET /api/me ──→ 200 { user: {id, username, role}, must_change_password }
             └─ 401（未登录）
  ├─ 200                 → 进入工作台（侧栏用户区显示 用户名/角色；must_change_password 时顶部横幅）
  └─ 401                 → #/login?redirect=<当前路由>
```
> **没有免登录分支**：单人部署＝只有一个 admin 账号的部署，同样必须登录（服务端 §3.5.1）。
> 首次部署的初始密码由服务端在首次启动时生成并输出（日志 + `<data_dir>/initial-admin-password.txt`，首次登录后删除；服务端 §3.5.2）——登录页不预填、不提供"从本机读取密码"。
- 路由守卫：全局 `beforeEach` 读 auth store；未登录访问受保护路由 → 登录页（记回跳）；`/api/auth/login`、`/api/health`、静态资源为白名单（服务端 §3）。
- 会话过期：任意请求 401 → auth store 清空 → toast「登录已过期」→ 跳登录（带回跳）。精修编辑器有短间隔自动保存，丢失风险低；登录后回原位。

### 1.2 LoginView（`#/login`）
- 居中卡片（sakura token）：品牌 + 用户名 + 密码 + 登录；无注册入口（账号由管理员建）。
- 错误态：密码错（含限流提示「尝试过多，15 分钟后再试」）、账号禁用、服务不可达——文案来自后端 `error` 字段直出。
- 成功 → 回跳 redirect 参数或 `#/workbench`。

### 1.3 用户区与登出
- 侧栏底部（全局入口列表最上）：用户 chip `用户名 · 角色`，点击弹层：登出（`POST /api/auth/logout` → 回登录页）。
- `must_change_password`：顶部常驻横幅「仍在使用初始密码」+ 一键去改密（`PUT /api/auth/password`）。
- 单人部署（只有 admin 一个账号）与多用户**同一套 UI**：区别只是项目/账号数量与角色可见入口，不存在"本机模式"这个态。

### 1.4 新增文件
```
stores/auth.js     { me, user, mustChangePassword, ready, loadMe(), login(u,p), logout(), changePassword(o,n) }
api/auth.js        login / logout / me
views/LoginView.vue
```
client.js：启用 CSRF 注入（读 `vtrim_csrf` cookie → `X-CSRF-Token`，仅写请求）与 401 分支（工作台方案 §5 已留位）。

## 2. 项目作用域切换（M2）

- 唯一改动点（工作台方案已预留）：`api/projectData.js` 的 `scopeQ(pid)` → `project=${pid}`；`api/polish.js` 的 `dir` 参数 → `{project, dir}` 项目内相对路径；`workbench.loadProjects()` 端点 → `GET /api/projects`。路由/组件/store 零改动（`:pid` 本就是不透明标识）。
- `POST /api/projects` 仅接受 `source`（不再有本地路径 init）——新建向导见 §4。
- **项目列表升级**（`/api/projects` 含来源/占用/状态）：侧栏项目行徽标增加来源文字（`录制 23058` / `B站`）与占用（`12.4 GB`）；项目数可能增长 → 列表顶部加**过滤输入框**（按名称/房间前缀匹配，纯前端）。
- **首页项目列表**（WorkbenchView 主区块，表格化，可在此一并处理与预览每个项目）：
  - 列：项目（名称 + 来源：`录制 · 房间 23058` / `B站下载`）｜进度（进度条 + 下一步按钮：打开预处理/去切片/去精修）｜占用｜操作（预览[最近成片] · 打开 · 删除源录播）
  - 「删除源录播」（危险，confirmDialog 说明"保留切片/字幕/事件/封面，仅删源录播且不可恢复"）→ `DELETE /api/projects/:id/source` → project store invalidate + storage 刷新
  - 「预览」：有切片 → 媒体弹层播最近成片；无切片 → 提示先完成预处理
  - 项目详情（来源场次/UP主、文件产物清单）在预处理页的项目文件区与存储页明细中呈现
- 渲染导出路径分支删除（默认固定 `export.mp4`）。

## 3. 入库（M3）

### 3.1 WorkbenchView 升级为真正的首页
未选项目态的首页 = **项目列表**（主区块，见 §2）+ **录制导入（最近）**。已选项目态照旧直进工作区。

**录制导入区块**（首页；完整列表在入库页）：最近 `ingest_sessions`（`GET /api/ingest/records`）：
- 行：`房间 23058 · 09-12 20:00` + 状态徽标（`录制中` / `等待收尾` / `已建项目` / `失败` / `未分配`）+ 文件数/大小 + 去向（项目链接或 重试/指派）
- 刷新：入库/下载任务（本身是 job）完成事件触发 + 30s 兜底轮询
- failed 行内「重试」；admin 对未映射行内「指派」

**自动导入规则**（入库页内；语义 = 哪个房间录完后归谁、自动执行到哪一步）：
- 房间 → 我的绑定列表 + `自动策略` 档位下拉（`仅入库 none` / `转写 transcribe` / `检测 detect`（默认）/ `切片 cut`）+ 启用开关 → `GET/PUT /api/channel-map`
- 文案说明策略含义（cut 会自动占盘切片，detect 为推荐默认）

### 3.2 `#/ingest` 入库页
首页区块的完整版，三个区块：**录制导入**（全量，筛选：全部/失败/未分配）+ **自动导入规则**（房间/归属/完成后自动执行/启用，表格化）+ admin 的**待分配**（未配置规则的已完结会话，行内指派：选用户 + 策略 → 走该用户配额预检入库）。

### 3.3 状态与提示
- 缺弹幕的入库项目：总览页提示「未找到弹幕文件，热度排序将不可用」（服务端 §4.4）
- 入库失败原因直出（`error` 字段：磁盘不足文案含"预计需要/可用/建议删除"→ 按钮「去存储页」）
- 录播姬未配置/未启用时（`/api/health.ingest_ready=false`）：区块显示引导卡（仅 admin 可跳管理页配置）

## 4. B站链接通道（M4）

新建向导按 source kind 分步（工作台方案 §2.2 的抽象在此启用）：

```
步骤1 选择来源 → [录制导入] [B站链接]        （本地文件浏览来源移除）
步骤2a 入库记录 → 已完结未建项目会话列表 → 选中确认
步骤2b B站链接 → URL 输入 → POST /api/ingest/resolve
              → 预览卡：封面/标题/UP主/时长/分P多选/质量下拉/预计占用
              → 配额预检不过 → 禁用确认 + 原因
              → POST /api/projects {source:{kind:"bilibili",…}} → 下载任务（下载槽）
```

错误文案映射（`resolve`/下载 job 的 `error` 分类 → 用户可读 + 指引）：

| 分类 | 文案方向 |
|---|---|
| UnsupportedUrl | 仅支持 B站视频链接 |
| NeedLogin / MemberOnly | 该视频需要登录/大会员，请联系管理员配置下载 cookie |
| RateLimited | 触发风控限流，请稍后重试 |
| ToolMissing | 服务器未安装下载器，请联系管理员 |
| DiskFull | 磁盘空间不足（预计/可用）→「去存储页」 |

分 P 默认每 P 一个项目；`already_exists` 时预览卡标记已存在并禁用该 P。

## 5. 存储页（M5，`#/storage`，全员）

- 顶部计量条：我的占用 / `quota_bytes`（0=不限则只显示已用）；admin 额外见全局 `max_disk_bytes` 与水位
- 按项目明细表（`GET /api/storage`）：项目、来源、源录播大小、产物大小、合计、最近活动
- 行操作「删除源录播」：同总览页动作（同一 confirm 组件）
- 超限引导：任何 409 磁盘拒绝的 toast 带「去存储页」按钮

## 6. 管理页（M5，`#/admin`，仅 admin）

Tab 结构（懒加载各 tab 数据）：

| Tab | 内容 | 端点（服务端 §5.3） |
|---|---|---|
| 账号 | 用户表（用户名/角色/配额编辑/禁用开关/最后登录）、新建用户、重置密码 | `/api/admin/users` |
| 录播姬 | webhook URL 展示+复制、token 生成/轮换/禁用、`recorder_root`、来源 IP 白名单 | `/api/admin/ingest` |
| 房间映射 | 全局 room→用户 表（同用户侧但可编辑 user 列） | `/api/channel-map` |
| 下载器 | downloader 选择 / max_quality / rate_limit / cookie 状态 / 并发数 | `/api/admin/config` |
| 任务 | 全平台任务（含 owner 列、取消） | `/api/admin/jobs` |
| 待分配池 | 未映射会话指派（用户+策略） | `/api/admin/ingest` |
| 平台配置 | 原"全局设置"内容（config.toml/llm.toml/模型管理/doctor/LLM 测试）整体迁入 | `/api/admin/{config,models,doctor,llm/test}` |

**IA 变化**：多用户下「全局设置」从侧栏消失（平台密钥 admin-only，服务端 §5.3），普通用户侧栏 = `[任务, 入库, 存储]`；admin = `[任务, 入库, 存储, 管理]`。单人部署＝该唯一账号就是 admin，因此它能看到 `[任务, 入库, 存储, 管理]`（含原「全局设置」内容），**与多人部署用同一套可见性规则**，没有单独的本机分支。侧栏入口列表由 role 过滤——工作台方案的列表化配置在此生效。

## 7. 任务中心升级（贯穿 M3–M5）

- 任务行新增：**排队中**状态（queued：显示「排队第 N 位」，取消=立即出队）、档位徽标（`轻活`/`略重`/`重活`）
  —— 已落地（2026-09）：档位/名次由 `JobSummary.class` / `class_cap` / `queue_position` 给，
  排队行在说明行写「重活任务（并行上限 2）· 同档第 2 位」（`api/jobs.js::queueDetail`）。
  设计里的槽位词是 `入库`/`下载`/`重活`，实现改成**按本地资源占用分三档**（LLM/网络类落「略重」
  而不是单独一槽），入库自动管线靠 `JobPriority::Background` 体现。
- 每用户串行语义对用户透明：第二个任务提交后显示排队中而非报错（已落地）。
  **`429`（排队超 3）未实现且已放弃**：队列现在按档位各有名额、超了就一直排队，不设上限也不拒绝。
- admin 的全平台任务见管理页；`/api/stream` 服务端已按 owner 过滤，前端无感知

## 8. 封面背景图上传（唯一的本地上传例外）

服务端 §0.2.5：不接受本地视频，但封面背景图例外（≤32MB，落项目目录）。CoverDesigner 的背景选择增加「上传图片」：`<input type=file>` 校验类型/大小 → POST 上传（`{project}` 作用域）→ 回填背景路径。FileBrowserModal 在多用户期仅保留此用途（若上传走标准 file input 则一并删除）。

## 9. store / api / 路由增量清单

```
stores/   auth.js（M1）· ingest.js（M3：records/channelMap/assign）· admin 状态就地（M5）
api/      auth.js（M1）· ingest.js（M3：records/channel-map/resolve）· admin.js（M5）· storage.js（M5）
views/    LoginView.vue（M1）· IngestView.vue（M3）· StorageView.vue（M5）· AdminView.vue（M5）
组件      wizard/SourcePicker · wizard/BilibiliStep（M4）；common/StorageMeter（M5）
路由      #/login（M1）· #/ingest（M3）· #/storage、#/admin（M5）；守卫接 auth store
```

侧栏角色矩阵（**无"本机模式"列**：单人部署＝唯一账号是 admin）：

| 入口 | 多用户 user | admin（含单人部署） |
|---|---|---|
| 新建项目（置顶） | ✓（入库记录 / B站链接） | ✓（同 + 待分配快捷） |
| 任务 / 入库 / 存储 | ✓✓✓ | ✓✓✓ |
| 设置（平台） | ✗ | ✗（内容移入 管理） |
| 管理 | ✗ | ✓ |

## 10. 验收清单（按阶段）

- [x] M1：未登录访问任意页 → 登录页带回跳；初始密码（服务端首次启动生成）可登录成功且首次登录后密码文件被删；错误密码/禁用/限流文案正确；改密后 `must_change_password` 横幅消失；登出干净
- [x] M2：A 账号看不到 B 的项目/任务/SSE/媒体（404）；项目行来源/占用徽标正确；删除源录播后占用回落
- [x] M3：模拟 webhook 完结 → 首页动态出现→项目自动出现→按策略自动跑管线；失败行可重试；admin 可指派待分配
- [ ] M4：B站链接 resolve 预览/分P/质量/配额预检；五类错误文案正确；下载任务出现在下载槽
- [ ] M5：存储计量与上限拒绝文案；管理页各 tab 可用；普通用户侧栏无管理/设置入口
- [ ] 全程：`npm run build` 零错误；工作台方案验收清单不回退
