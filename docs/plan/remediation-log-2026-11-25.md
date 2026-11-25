# 整改执行记录（remediation-log，2026-11-25）

> 对应计划：`docs/plan/remediation-plan-2026-11-24.md`（T1–T23）；依据：`docs/plan/architecture-review-2026-11-24.md`。
> 纪律：每个任务一个提交、提交说明带任务号；T10 安全改动单独提交（将来单独发版）；不做 CI；
> 未放行前不改版本号、不打包。本文只记**做成了什么、用什么判据证明**，论证留给计划书。

## 状态总表

| 任务 | 状态 | 提交 | 关键判据 |
|---|---|---|---|
| T1 `%SystemDrive%` 垃圾目录 | ✅ 无需做（经查已不存在） | `d2782244` | `git status -uall` 0 条未跟踪 |
| T2 `.gitignore` 补 `/video/` | ✅ | `d2782244` | `git check-ignore -v video/...` 命中；untracked 2694 → 0 |
| T3 卡瘦身（components / api） | ✅ | `5b55f8e9` | 两条链 57338→49610 / 57322→47149 B |
| T3b 其余链条瘦身 | ✅（计划外派生） | `f2a25dd9` | 全链 →50503 B |
| T3c scripts 卡瘦身 | ✅（派生） | `823a645a` | 24572→17929 B，⚠️ 行 28→32 |
| T3d polish 卡再瘦 | ✅（派生） | `66b5a6cb` | 最坏链 52464→51376 B |
| T4 三处裸同步 IO 补 blocking | ✅ | `68b75b7a` | 新用例 `clip_delete_runs_sync_io_off_the_worker`，变异自证红/绿 |
| T4a 已覆盖清单 + source_delete 回归 | ✅ | `8bf024c8` `1b02a84f` | 新增回归覆盖此前零覆盖的 handler |
| T5 `scripts/gate.mjs` 统一入口 | ✅ | `7d7705d6` `9c87094c` | 117 项；SKIP 显式；退出码=非基线失败；反自证两条 |
| T5a 首轮 6 条真红定性 | ✅ | `1f12c166` `0cd7e872` | **6 条全 A 类**（判据自身坏），baseline 一条没加；gate 复跑 EXIT=0 |
| T6 保留期 + 每日维护 | ✅ | `848000ad` | 真机：`daily maintenance: sessions=1 audit=2 usage=2 logs=10`，40 份日志→30 |
| T7 前后端路由契约差集 | ✅ | `b85a51b0` | 前向差集 0；顺手补「注释掉的 `.route(` 仍算已注册」的假绿 |
| T8 gate 里包结构门禁 | ✅（并入 T5） | `7d7705d6` | `--check-structure` |
| T9 请求体结构体化（第一批） | ✅ 9/10 | `a2ddaa6a` | 字段名拼错→400 点名；5 条老契约用例仍绿；变异自证 |
| T10 路由 401 兜底枚举式 | ✅（安全改动，单独提交） | `38de6254` | 探针路由未挂认证层仍 401；白名单 6 条各带理由 |
| T10a 修 `api::config` flaky | ✅（派生） | `2d95c085` | 连跑 10 次全绿 |
| T11 `projects_list` N+1 | ✅ | `a7e97c24` | owner 名一次查表；变异→整列 null 即红 |
| T12 `cover-takumi` 归属 | ✅ | `781c07f4` | `--features cover-takumi` 绿；`xtask package` 缺 feature 拒绝打包 |
| T13 parity 判据边界（D-1） | ✅ | `8f95f736` | 11 保留 / 2 降级；校正「index.html 是出片路径」的旧口径 |
| T14 删模板腿（三步） | 🟡 第 1、2 步已落；第 3 步待跨版本 | `0d5da2f8` + `de0c3db3` | 第 1 步清死代码（`preview()` / `coverPreview`，前端 0 调用）、第 2 步 `polish preview` 改指引退出 + 删 `cmd_preview_cover`；第 3 步要删 `templates/index.html` 整条腿，按计划等一个发行版本 |
| T15 `engine.js` 定位写明近似 | ✅ | `6df30f37` | 文件头改成「近似预览腿，不是出片路径」+ 允许/不允许的边界；polish 卡顶部同步 D-1 定位 |
| T16 拆 `api/polish/` | ✅ | `570ff8ff` + `037f819c` | 6,235 行 → `api/polish/` 8 个文件（产线最大 775）；glob 归零；`///` 707 / `#[test]` 48 守恒 |
| T17 拆 `ingest.rs` + 领域词判据 | ✅ | `2c25df7d` | 10 个文件；21 条规则；三处变异自证 |
| T18 拆 `api/project_data/` | ✅ | `e02f9df4` | 3,570 行 → `api/project_data/` 7 个文件（产线最大 711）；`#[test]` 39 守恒 |
| T19 路径判据收敛 | ✅ | `fdc8b7b1` | 一份谓词 + 显式严格度；变异（摘 `:` 规则）⇒ 两条用例同时红 |
| T20 edition 统一 + rust-version + 锁文件 | ✅ | `d060162a` | 8 crate → 2024；`cargo test --workspace` 全绿 |
| T21 `VTEDIT_*` 环境变量 → 参数 | ✅ | `12e8aa2f` | `RenderOpts`；新用例证明两份参数互不污染 |
| T22 进程级全局可见化 | ✅ | `730d0278` | 真跑 server 日志出现 `usage sink: installed` / `local admin uid = 1` |
| T23 决策登记 D-1..D-5 | ✅ | `b87cb9e3` | ROADMAP 一节 |
| T4b `create_local_project` 异步化 | ✅ | `d4a2c29a` | 整段落盘一个闭包；新用例 `create_project_runs_sync_io_off_the_worker`；把 `api::blocking` 改成同步跑 ⇒ 该用例与 clip_delete 那条**同时红** |

## 阻塞

- **T14 / T15**：都要改前端并重建 `webui/dist`，而工作区里有一份**不是我方产生**的未提交改动
  （`webui/src/components/ProjectClipsSection.vue`，card/list 两档重构 + dist 已重建）。
  一重建 dist 就会把那份未提交源码的效果固化进入库产物，所以这两条停在这里等放行。

## 本轮之前暴露、值得单独记的两条实测

1. **T5a 的 #1 推翻了任务书的前提**：计划写「转译器写出底座不认的键」，实测本机底座 v0.5.0 **认**那三个键
   （`dhampir-timeline/src/layer.rs` 声明、`overlay.rs` 读取、`schema/doc-v1.schema.json` 收录）。
   真因是**判据的手抄键表单向老化**（只漏不多 ⇒ 判据全红、代码全对）。修法：键集权威改为读底座 schema。
2. **T19 的判据说反了**：计划写「把判据改松，两条 polish handler 用例都必须红」，实测**不会** ——
   第②③道（`jail_*` / `ensure_within` 的解析后归属校验）在后面兜住了。这是纵深防御生效的证据，
   但也说明：字符串判据的变异敏感性只能由**谓词/中间件层单测**保证，不能指望 handler 级用例。

## 验收口径（整轮做完时逐条打勾）

```bash
python3 scripts/check_module_map.py          # 模块/预算/骨架/索引/⚠️ 下限
python3 scripts/check_module_boundaries.py   # 21 条（含 2 条领域词隔离）
node scripts/gate.mjs --all                  # 非基线失败 = 0
node scripts/check-api-contract.mjs          # 前端路由字面量零差集
cargo check --workspace --all-targets        # 0 warning
cargo test --workspace --no-fail-fast
cargo check -p yekitrim-server --features cover-takumi
npm --prefix webui run build                 # 只有改前端时
```
## 阶段小结（2026-11-25，第三~五轮）

- **P4 结构拆分已完成**：`ingest.rs` 3,235 行 → 10 个文件；`api/polish/` 6,235 行 → 8 个文件；
  `api/project_data/` 3,570 行 → 7 个文件。`api/` 下现有三张卡（api / api-polish / api-project-data），
  模块 36 → 38，最坏链 54,998 / 57,344 B，`glob 重导出` 0 命中，`route_manifest` 全程 142 条逐行不变。
- **T16 拆成三阶段**（与计划不同，理由成立）：先 `T16a` 断开 glob 环（`validated_project_dir` 下沉到
  `api/mod.rs`），再分别拆两个文件 —— 计划要求同批次是因为当时两者互相 glob 引用、只拆一半会留环；
  环先断掉之后，两次拆分可各自独立编译与验证。
- **两个执行者中途失败**（一次在 T16/T18 开工前、一次在 T4b 中途留下 `E0425`），由 Lead 接手：
  T4b 的收尾 = 修编译错 + 补 `create_project_runs_sync_io_off_the_worker` 用例 + 机制级变异自证。

## 仅剩的阻塞

T14 / T15 仍等 `webui/src/components/ProjectClipsSection.vue` 那份**未提交**改动放行（见上节）。
## 第六轮补记：拆分后的**门禁漂移**（这一节值得单独读）

拆文件之后 `cargo check` / `cargo test` 全绿，但有一类门禁是**按文件路径读源码**的，编译期完全看不见：
`check-dhampir-wiring` / `check-emotion-track` / `check-element-color` / `check-camera-zoom-parity` /
`check-layout-invariants` 都写死了 `crates/yekitrim-server/src/api/polish.rs` 这类路径。
拆成目录后它们读不到，症状是「判据静默恒假」或「整条脚本报错」，而编译器一个字都不会说。

处置：5 个脚本改成**目录感知读取**（目录就按文件名顺序拼接全部 `.rs`），另有两条判据跟着代码走：
`polish_emotion_get` 的可见性 `pub(super)`→`pub(crate)`、⑳ 的 `body.get("ranges")`→强类型字段 `ranges: Vec<EventRangeBody>`。
顺带把 docs 里 23 个 `.md` 的旧路径指向新目录。
验证：`node scripts/gate.mjs --all` → 通过 194 / 0 条非基线失败 · EXIT=0。

**教训**：拆文件的验收不能只看 cargo —— 必须跑一次 `gate --all`。

## 第七轮补记：baseline 清账 + clippy 债务（如实记）

- **gate-baseline.json 4 条 → 1 条**：check-39-tiers-pixels / check-perchar-stagger / check-render-visible 这三个脚本
  后来自己改成了「缺前置就 exit 2 并打印要准备什么」，gate 已经把它们记成 SKIP，基线里那三条永远不会再命中
  —— 留着就是「第二个静默跳过」（gate 自己每次都在提示可删）。已删；剩下的只有 check-polish-page-loads#self-test。
- **clippy 没做，记成债务**：计划 T20 的第三小项标着「可选」，实测 cargo clippy --workspace --all-targets 有 **196 条告警**
  （23 field-reassign-with-default / 15 doc-list-item-indentation / 11 neg-cmp-op-on-partial-ord / 10 type-complexity /
  7 empty-line-after-doc-comment / 7 unnecessary-sort-by / 6 needless-borrow / 5 too-many-arguments …）。
  没有把 -D warnings 塞进 gate —— 在清零之前塞进去，要么立刻红一片，要么逼着人拿 baseline 把红盖住
  （而 baseline 只准登记环境性失败）。顺序 = 先分批清零，最后一类清完再加 gate 项；任务记为 T20a。