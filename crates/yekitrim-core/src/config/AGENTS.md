# yekitrim-core/src/config — 配置类型、两层合并与安全写入

> 范围：`crates/yekitrim-core/src/config`/** ｜ 上游：core 各 stage、`yekitrim-service`、`yekitrim-server`（`/api/config`）、`yekitrim-cli` ｜ 验证：`cargo test -p yekitrim-core --lib config`

## 职责

定义全站配置类型（`types.rs`）与读写合并（`mod.rs`）。两层合并: `<data>/users/<uid>/config.toml` (程序) → `yekitrim.toml` (项目, 后加载覆盖前者)。无环境变量。
实现见 `yekitrim-core/src/config/mod.rs::load_config_at`（先 global 后 project）。
此外提供 `patch_config_file`（设置页表单保存通道）、`write_project_config` 与几个 `save_*_path_in`（stage 落盘后回写生效值）。

> 历史与实测数据见 **`docs/core-config.md`**（§1 三份配置的逐字样例 / §2 死字段审计表 / §3 版本迁移沿革 / §4 程序配置单点收敛 / §5 设置页数值项的实测与几何）。

## 入口

- `config::load_config()` / `load_config_at(project_file)` / `load_global_config()` / `load_llm_config()`。
- `config::patch_config_file(path, &serde_json::Value)` —— 设置页表单保存的唯一通道。
- `config::write_project_config` / `write_project_config_exact` / `normalize_project_paths_in`；`save_events_path[_in]` / `save_audio_path[_in]` / `save_transcript_path[_in]`。
- `config::resolve_reference_titles(Option<&LlmPromptCfg>) -> Vec<String>`（S4 起提示词住 llm.toml）。
- `types::{Config, ProjectConfig, Materials, MaterialEntry, AuthConfig, IngestConfig, RecorderConfig, StorageConfig, AudioConfig, AsrConfig, DetectionConfig, LlmConfigV4, CuttingConfig, SfxConfig, ChromeConfig, CorrectConfig}`（⚠️ v3 的 `LlmConfig` / `LlmProvider` / `LlmOverrides` 已在 L4 删除，只剩迁移用的 `llm_v4::LlmV3`）；`ProjectConfig::current_stage(&output_dir)`；`MaterialEntry::{asr, user, is_asr, slot}` / `Materials::{of, of_mut}`；`LlmConfigV4::{for_link, concurrency, concurrency_for, prompt_for, run_for}`（按链路的四件套，`LinkedLlm::run()` 带本地行为参数）。

## 边界

**merge 条件**: String 字段 `!is_empty()`, f64/u32 字段 != 默认值, Vec 字段非空时追加合并。

**配置默认值注意事项**: 所有配置类型必须手动实现 `Default`（而非 `#[derive(Default)]`）以确保 serde 默认值与 Rust 默认值一致。`Config::default()` 用 `toml::from_str("")` 初始化，依赖各字段的 `#[serde(default)]` 来调用正确的 `Default` impl。新增字段时必须同时加 `Default` impl + `merge()` 条件。

### 新增配置字段步骤

1. `config/types.rs` 加字段 + serde default
2. `config/mod.rs` 的 `merge()` 加对应条件（注意 `<data>/users/<uid>/config.toml` 在 `yekitrim.toml` 之后 merge）
   —— ⚠️ **`[llm]` 是例外：它不参与 merge**，`load_config_at` 最后一句 `config.llm = load_llm_config()` 无条件覆盖，所以 LLM 相关字段只改 types/mod 的模板，不用碰 `merge()`
3. 结构体手动 `Default` impl (有字段级 serde default 时必须)
4. `config/mod.rs` 的模板常量 + 注释（`GLOBAL_CONFIG_TOML` = config.toml / `LLM_CONFIG_TOML` = llm.toml；仓库里**没有** `config/default.toml` 这个文件，旧文档里的写法是错的）

### 两层样例（形状；三份的逐字全文见 `docs/core-config.md` §1）

**`yekitrim.toml`** — 项目配置（只留形状，逐字全文见 §1.1）:
```toml
config_version = 3           # 配置结构版本，别手改（见「版本号与读时迁移」）
input_file = ""              # 唯一生效值；danmaku_file / transcript_file / audio_file / events_file 同理

[project.materials]          # 素材清单（生效值仍是上面的 *_file，这里只是"能切回来"的候选）
transcript = [{ path = "output/transcript.hybrid.json", source = "asr", mode = "hybrid" }]
danmaku = []

[detection]
danmaku_window_secs = 10

[cutting]
output_dir = "./output"
overwrite = false
```

⚠️ `llm.toml` 的 `extra_params` 样例 2026-11-26 已**取消全局那份**：逃逸口只按模型写在 `[[llm.models]].extra_params`。文档与模板不一致时**以模板为准**。

**`<data>/users/<uid>/config.toml` / `llm.toml`** — 程序配置与 LLM 配置：**字段与注释的权威是 `config/mod.rs` 的两个模板常量**，逐字样例见 §1.2 / §1.3（`[asr]` 的 14 个键、`[correct]`、`[detection]`、`[cutting]`、`[sfx]`；llm 的 `[llm]` / `[llm.defaults]` / `[[llm.providers]]` / `[[llm.models]]` / `[llm.links.<link>]` + 它的 `.prompt` / `.run` 子表）。⚠️ S4 起 **config.toml 里没有任何 LLM 行为键**（提示词 / 窗口 / 并发 / 评分权重都在 llm.toml）。

### 死字段（2026-10：**已删除**，别再捡回来）

`asr.align_model` / `asr.max_segment_duration` / `asr.segment_merge_gap` / `cutting.silence_noise_db` 这 4 个字段**曾经定义了但全仓没有任何读取方**，已按「新增配置字段步骤」的反向流程从 `types.rs`（结构体 + `Default` + 两个 `default_*` 函数）与 `mod.rs`（`merge()` 条件 + 模板行）一并删除。逐个"为什么没人读"与**复现审计口径**（找 `.<字段名>` 读取点时必须排除 `config/` 自身，否则会把死的伪装成在用的）见 §2。

⚠️ **老 toml 里还留着这几行不会炸**：`Config` / `AsrConfig` / `CuttingConfig` **都没有** `deny_unknown_fields`（只有 polish 的 `polish.toml` 有）。这条由 `config::removed_fields_tests::legacy_toml_with_removed_keys_still_loads` 钉住 —— 给这三处**任何一处**加 `deny_unknown_fields` 都会让判据翻面（变异自证跑过）。

⚠️ 它们**已经从设置页表单里摘掉**（表单不再写回，字段本身留在文件里、仍可用「编辑源文件」改）。真正的清理（从 `Config` 结构体里删）要走反向流程，且要确认旧文件里的多余键不会让反序列化失败 —— **本次没做，留给你拍板**。

### ⚠️ 程序配置只有一个位置（2026-11-13，方案 A）

**唯一路径** = `config::global_config_path()` = `<数据>/users/<uid>/config.toml`。读写两侧都用它：`load_global_config` / `load_config_at` / `load_llm_config` / `ensure_global_config` / 设置页与接入页的写通道。**别再新增"顶层 config.toml"候选位置。**

启动迁移 `migrate_top_level_config_to`：`[ingest] [storage] [publish] [auth]` 顶层赢，其余段与 `llm.toml` 账号赢（只补缺失），旧文件改名 `*.migrated-<日期>.bak`；幂等、fail-soft。判据 `config::config_migration_tests`；正文 `docs/plan/single-user-config.md`，事故背景见 §4。

`[auth] multi_user`（默认 false = 单人优先）：前端藏邀请码/配额/注册入口/入库指派下拉，经 `/api/health.multi_user` 读；`merge()` **只认 true**。

### 版本号与读时迁移（2026-10-05）

**每个配置文件顶层都有一个 `config_version`**（`config/version.rs` 是唯一实现）：

| 文件 | 位置 | 升级后写回？ |
|---|---|---|
| `config.toml` / `llm.toml` / `server.toml` | `<data>/` | ✅ 写回（先备份 `.bak-v<旧>`），一个目录各一份 |
| `yekitrim.toml` / `polish.toml` | **每个工程一份** | ❌ **只在内存里升，不碰磁盘**（工程可能几百个、还可能在网络盘上） |

读的时候三种情况：**版本相同 ⇒ 原样用** · **版本更低 ⇒ 先迁移再解析** · **版本更高 ⇒ 一个字节都不动**（那是更新的程序写的；没有这一条，用旧版 exe 打开新配置会把文件"升"回去 —— 直接损坏用户数据）。缺键 = **0**。

迁移是**纯函数 + 保注释**：用 `toml_edit`（`toml::to_string` 重序列化会**把注释全丢掉**，而模板里大段说明都写在注释里）。已发生的迁移（`v0 → v1` 清死键 / `v1 → v2` 把 `temperature`/`max_tokens`/`thinking` 搬进 `[llm.params]` / `v2 → v3` 搬成 `[[llm.providers]]` / `v3 → v4` 收成"一份真相" / **`v4 → v5` 把 LLM 行为键从 config.toml 搬进 llm.toml**）逐条见 §3，**改迁移前先看那里的判据**。
⚠️ `v2 → v3` 时 `[llm.models."<id>"]` **留在原地**（能力覆盖与 provider 无关，换 provider 也照用；搬走会把一份拆成 N 份），`overrides` / `max_concurrent` / `extra_params` 也都不搬。

⚠️ **改配置键的规矩（第 5 步，比代码重要）** —— 删键 / 改名 / 改语义时必须做三件事：
① `config/version.rs` 的 `CONFIG_VERSION` **+1**；② 在 `ConfigKind::retired_in` 里把那个键按 `["section", "key"]` 登记成"新版本该清掉的"；③ 到 `config/mod.rs` 的 `GLOBAL_CONFIG_TOML` / `LLM_CONFIG_TOML` 模板里同步注释。**漏 ①** ⇒ 迁移永远不跑；**漏 ②** ⇒ 老文件里那个键**永远留着**（`patch_config_file` 只合并不删）。
⚠️ **"搬家/改名"再加一件**：`retired_in` 只会**删**，表达不了"搬到别处" —— 那种要在 `migrate_step()` 里写一段一步一段的迁移（`migrate_llm_v2` 是样板：**连键的 decor 一起搬**，上方注释不丢；搬完再让 `retired_in` 兜底清掉搬不动的残留）。
⚠️ **跨文件搬家**（`v4 → v5` 是唯一一例）：`migrate_config_to_llm` + `migrate_global_pair` —— **先写 llm.toml（备份 + `validate_llm_toml` + 原子写）成功，才让 config.toml 丢源键**；失败就两个文件都原样留着、下次再试。三条读路径（`prepare` / `edit_toml_at` / `clean_retired_keys_in`）都要接，**漏 `edit_toml_at` 就是"只删不搬"**（用户的提示词正文直接没了）。

⚠️ **`polish.toml` 的版本键在文本层**（不在 `ProjectFile` 里）：那个结构体是 `deny_unknown_fields`，而且还会出口给前端与出片底座 —— 把版本塞进 schema 会波及它们。所以写侧 `to_toml_commented()` 在文本里补一行、读侧 `validate.rs` 用 `version::strip_version_key()` 摘掉再看结构体。

⚠️ **工程级那份"只在内存里升"的代价**：磁盘上那几行**永远在** ⇒ 每次读都会重新发现它。所以：① 报要**去重**（`report_retired`：每文件每进程只报一次，实测 100ms 曾刷 10 条）；② 真清走 **`clean_retired_keys_in`**（CLI `yekitrim config migrate <工程目录|toml>`，清前留 `.bak-v<旧>`；**返回空 = 不动磁盘**）。`migrate_text` 是**纯函数、不打日志**（日志在调用方 —— 只有调用方知道路径与"写没写回"）。

判据：`config::version::tests`（29 条，含 v3→v4 / **v4→v5 跨文件搬家**；含**文件版本更高时不动**、幂等、注释不丢、程序级写回+备份 / 工程级不碰磁盘 / `clean_retired_keys_in` 真写回且幂等 / **同一文件只报一次**）。**变异自证**跑过：`CONFIG_VERSION` 改回 0、或把 4 个死键从 `retired_in` 里去掉，都会让判据翻面（不是空转）。

### 写入纪律

`yekitrim init` 生成 `yekitrim.toml`（项目字段：input/danmaku/转写/剪切参数）。

⚠️ **工程配置只允许带「按工程变化」的段**（`write_project_config` 的唯一口径）：`[asr]` / `[detection]` / `[cutting]` / `[sfx]`。`[llm]` / `[correct]` / `[auth]` / `[publish]` / `[storage]` / `[ingest]`（含子表）**一律不许抄进工程文件** —— 工程目录会被共享、搬到 NAS、发给别人（webhook 模式下甚至直接建在录像目录里）：把 `token_hash` / `api_pass` / `passkey_file` 抄进去就是泄漏；而且 `merge()` 把项目段当覆盖值 ⇒ 管理员改了全局口令、老工程拿旧副本顶回去。实现在 `config::project_config_body`，判据 `config::project_template_tests`。
⚠️ **存量工程不自动剥离**（R4：已有文件不无提示重写）：老工程里已经抄进去的段会留着；要清就用户自己删（或重建工程）。别在 `patch_config_file` / `merge` 的读路径上顺手剥离 —— 那会静默改用户文件，也分不清「我们抄的」与「用户自己写的工程级覆盖」。

`yekitrim config init` 生成 `<data>/users/<uid>/config.toml`（`[asr]` + `[detection]` + `[correct]` + `[sfx]`）和 `<data>/users/<uid>/llm.toml`（`[llm]`，覆盖写）。`yekitrim server` 启动时若程序配置缺失会自动补齐（`yekitrim_service::ensure_global_config`）：只创建缺失文件；已存在但 TOML 语法错误的会备份为 `*.toml.broken` 后重建（坏文件会让整份配置被忽略）。设置页表单只覆盖常用项，保存走 `config::patch_config_file`（toml_edit 合并，保留其他 section 与注释），表单数据源用 `/api/config` 的 `global_config`（global-only），不能用 merged 的 `config`。

⚠️ **`patch_config_file` 支持两层以上（递归深合并）**：值是对象时进子表继续合并 —— `{"ingest": {"recorder": {"api_base": …}}}` 只改命中的键，`[ingest.recorder]` 里没提到的兄弟键（`root`/`api_user`/`api_pass`…）与**段内注释**都保留。**别再退回"一层 section → key"**：那时 `recorder` 会被整项换成 inline table，把整张子表静默丢掉（文件仍是合法 TOML，不报错，下次读配置才发现值没了）。回归钉子：`config::patch_config_tests`。新增需要写嵌套 section 的表单时直接用这个通道，不必再开专用端点。

⚠️ **`move_into_managed` 已于 v7 退场**（2026-11-27）：录播入库恒为「原地建项目」，不再搬源文件。
删配置键按五步走 —— `CONFIG_VERSION` 6→7 + `retired_in(6)` 登记 `ingest.recorder.move_into_managed`
+ 字段/`Default`/`merge()`/模板一起删；判据 `config::version::v7_retires_move_into_managed`。

⚠️ **`[ingest.recorder]` 的新字段（`kind` / `api_key` / `container_root`）必须同步进 `merge()`**（2026-11）：`merge` 是逐字段手写的，漏一条的症状是**文件里写了、`load_global_config()` 永远读默认值**（设置页保存成功、行为完全没变，还没有任何报错）。`kind` 的默认是 `"bililive"`，判断要写成"**≠ bililive 才覆盖**" —— 写成"非空才覆盖"会让默认值在合并时把已选的值打回去。回归钉子：`config::recorder_backend_merge_tests`。

⚠️ **写 `llm.toml` 之前必须"反序列化验一遍"**（P0 事故 2026-10-19）：保存路径曾把 `json_mode = ""`（字符串）写进布尔字段 —— 文件**是合法 TOML**，只有反序列化到 `LlmConfig` 才炸，而炸的后果是**整份 LLM 配置回退默认值**。守卫 = `config::validate_llm_toml`（server 写通道：先写同目录临时文件 → 验 → 才 `rename`；失败回 400 且**原文件一个字节不动**）。迁移回写（`version::prepare`）也吃同一道闸。
⚠️ **日志里的错误文案必须先脱敏**：TOML 报错会带出错那一行的原文 ⇒ 一律走 `config::redact_secrets`（`api_key` 的值抹成空串，键名/行号保留）。**只此一份实现**（server 的 `redact_api_key` 转调它）。

⚠️ **`null` = 删键**（`merge_into_table` 的第一条分支）：patch 是合并语义，这是唯一能表达"取消该键 / 回到跟随全局"的写法 —— 设置页把「按链路覆盖」那条的勾**取消**走的就是它。**别再退回"null → 空串"**：`max_tokens = ""` 是合法 TOML 但读不出 u32，`load_llm_config` 解析失败会把**整份 LLM 配置（含 api_key）打回默认值**。回归钉子：`null_patch_removes_key_and_keeps_siblings` 与 `empty_subtable_patch_changes_nothing`（`{}` 不删表、不吞注释）。

### LLM 配置 v5（整改「LLM 配置重构」L1–L3 / S4 已收口）

- config/llm_v4.rs：**v4 的唯一形状**（LlmV4 / LlmProviderV4 / LlmModelEntry / LlmLinkSetting）
  + **纯迁移函数** from_v3(&LlmConfig) -> Migrated（不碰磁盘；dropped 里装「表达不了」的东西，调用方必须打 warn）。
- 形状：[llm] active/max_concurrent + [llm.defaults]（唯一参数基准）+ [[llm.providers]]（只描述怎么连）
  + [[llm.models]]（唯一模型清单，(provider, id) 唯一）+ [llm.links.<link>]（接口/模型/参数覆盖）。
- ⚠️ **决策 A**：provider.params 已废弃 —— v3 的 effective_params() 是「整体遮蔽」（接口写了一个键就整份顶掉顶层），
  那是「改一个字段悄悄丢 max_tokens」的根源；v4 只有 defaults + 链路覆盖两层。迁移照搬 v3 的生效值（整体取用），
  非生效接口的 params 无法表达 ⇒ 进 dropped。
- ⚠️ 迁移两条硬要求（有用例）：**api_key 逐字节不变**、**round-trip 幂等**。
- **v5 增量（S4）**：`[llm.links.<link>.prompt]`（检测的 system_prompt / prompt_template / reference_titles）
  与 `[llm.links.<link>.run]`（window_size / max_concurrent / llm_score_weight）—— 两者都**不进请求体**；
  解析入口只有 `prompt_for()` / `run_for()`（`LinkedLlm::run()` 把 run 一起带出来：`materialize()` 会清空 links）。
- **2026-11-26 增量（config_version 5 → 6）**：`[[llm.models]].extra_params` = **每支模型自己的逃逸口**；
  **全局 `[llm].extra_params` 已取消**。老文件那一行由 `migrate_llm_extra_params_to_models` 搬到
  「active 接口 + 它的默认模型」的清单条目上（没有清单就补一条）—— 请求体逐字节不变；搬不动的由
  `retired_in(5)` 兜底删。解析唯一入口仍是 `LlmV4::to_params()`（只看模型级）。
  判据：`config::llm_v4::caps_source_tests::model_extra_params_go_into_request` 与
  `config::version::tests::v6_moves_global_extra_params_onto_the_active_model`。
- 当前状态：**接线完成（L1→L3 + S4）** —— `Config.llm` 就是 `LlmV4`；v3 的顶层三件 / `[llm.params]` / `[llm.overrides]` /
  `[llm.models."<id>"]` 与 v3 的类型（`LlmConfig` / `LlmProvider` / `LlmOverrides`）**已删除**，只剩迁移用的
  `llm_v4::LlmV3`；`[[llm.models]]` 的能力键是**嵌套内联表**（`catalog` / `overrides`）—— 两套来源形状不同，平铺会撞名。

### LLM 按链路覆盖（2026-09 / v4 起是 `[llm.links.<link>]`）

- ⚠️ **链路模型归属**（2026-11-27，M2）：`[llm.links.<l>].model` 指向的模型**在清单里、却属于另一家接口**时，
  `LlmV4::problems()` 会报一条（"…属于接口 A，而这条链路用的是 B"）。以前这条规则只活在前端链路表里
  （`modelMismatch` 标黄），CLI 与手改 toml 的人拿不到提示。⚠️ **清单里没有这个 id 不报** —— 手打 / 自建网关是合法的。
  生效面：源文件模式（`check_provider_raw`）**硬拦 400**；普通 patch 的探测体只带 providers，**零回归**。
- **一个入口**：`LlmV4::for_link(LlmLink)` —— 两级参数（`[llm.defaults]` 打底 + `[llm.links.<link>]` 逐键覆盖）的**唯一实现**（逐字段见 `LlmParamsToml::merged_over`），产物是 `LinkedLlm`（编译期拦住"忘记套链路参数"）。
  这里只记配置形态：链路那一层平铺的是**请求参数**（`temperature` / `max_tokens` / `reasoning` / `top_p` / `json_mode`，未知键忽略），
  另有 `provider` / `model`（引用接口与模型清单）与两张**不进请求体**的子表 `.prompt` / `.run`。
  ⚠️ **参数面与能力面分开**：能力面在 `[[llm.models]].overrides`（`ModelOverrideToml` → `caps`，模型 id 匹配大小写不敏感）。
- 新增链路 = `LlmLink` 加一个变体 + `ALL`/`key`/`label_zh`/`ui_visible` 四处（设置页那张表按后端给的 `override_links` 出行，前端不用改）。
  ⚠️ **界面行 ≠ 全部链路**：`LlmLink::ui_visible()` 是界面那几行（2026-10-19 起 = `detect/polish/title/utility`，**不含 `correct`** —— 用户口径"字幕订正删除"，但代码与 `[llm.links.correct]` 照旧生效，`asr/correct.rs` 还在用）；`LlmLink::LEGACY_V1` 是 v1 那四条，**金标准请求体按它遍历**（用 `ALL` 会去找第 5 条，而那份数据一个字都不许动）。
- ⚠️ **`max_concurrent` 现在也在链路表里**（`[llm.links.<link>.run].max_concurrent`，留空 = 跟随 `[llm]`）——
  S4 之前它是 `[detection]`/`[correct]` 的段级 `Option` 覆盖，那两处**已删除**。读法只有 `concurrency_for(link)`。

### LLM 并发与加速（2026-09）

- **一个旋钮**：`<data>/users/<uid>/llm.toml` 的 `[llm] max_concurrent`（默认 3），**事件检测 / 精修事件生成 / 字幕订正共用**；
  某条链路要单独压就写 `[llm.links.<link>.run] max_concurrent`。读法只有两个入口：`concurrency(None)`（全局）与
  `concurrency_for(link)` / `run_for(link).max_concurrent`（链路）。**别再在代码里写死 `3`，也别让某条链路偷偷用自己的。**
  设置页「LLM 配置 → 链路配置」的「默认参数」块是全局旋钮的输入框。

### ⚠️ 设置页数值项（配置字段的前端出口）

⚠️ **设置页的数值项走 `components/SliderField.vue`**（滑块 + 标签行右端可直填的数字框，16 处）：手填的值只夹 `[min, max]`、**不吸附 step**（step 是滑块的语义，不是配置的语义 —— 想设 `chunk_duration = 63` 不该只能眼看着它跳 60/65），且**清空/非数字不写模型**（否则退格清空那一瞬就把 60 写成 0）。
⚠️ **滑块宽度 = 所在格子的 1/3**（`.slf-range` 的 `flex:0 0 33.333%`，别改回 100% —— `width:100%` 会被 flex-basis 盖掉）—— 三样东西因此能在**一行**里读完。窄格（LLM 那几列，格子 ≤220px）由 `@container` 把标签挤到独占一行，**别改成媒体查询**（量的是格子宽而不是视口宽）。改完要重建 `webui/dist`（否则 server 端出的还是旧界面）。几何实测见 §5。
⚠️ **ASR 的 `Mode` 与它的说明挤成一行**（`.mode-field`，「标签 · 下拉（`width:auto`）· 说明」）：其他页（PolishView / CoverDesigner / DownloaderConfig）仍是旧写法，要用就自己换。
⚠️ **「按链路覆盖」在 L3 之后是表格**（行 = 链路，列 = 接口/模型/温度/输出上限/推理/状态），不再是 SliderField 网格：
未覆盖的格子显示 `/api/config/llm/effective` 的灰字实值，**点一下才可编辑**，「跟随」按钮发 `null` = 删键。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-core --lib config
```

覆盖 `patch_config_tests`（深合并/兄弟键与注释不丢/**null 删键**）、`llm_v4::*`（for_link 两级参数 / run_for / link_snapshot / 迁移幂等）、`version::s4_tests`（v4→v5 跨文件搬家）、`vad_merge_tests`（`Option<bool>` 显式覆盖口径）、`reference_titles_tests` —— 动了 `merge()`、`Default` impl、`for_link`、迁移或模板就必须看到它们全绿。
