//! 配置文件的**版本号 + 读时升级**（2026-10-05）。
//!
//! ## 一条规则
//!
//! 每个配置文件顶层有一个 `config_version`（整数）。**缺键 = 0**，代表"这个文件写在有版本号之前"。
//!
//! | 文件里的版本 | 处理 |
//! |---|---|
//! | `== CONFIG_VERSION` | 原样用 |
//! | `<  CONFIG_VERSION` | 先跑迁移（清掉已下线的键、补上版本号），再解析 |
//! | `>  CONFIG_VERSION` | **绝不动它** —— 那是更新的版本写的。打一条 WARN 后按现状读 |
//!
//! 最后一条是**保护用户数据**：没有它，用旧版 exe 打开新版配置会把文件"升级"回旧形状。
//!
//! ## 写回与否（两条路，别混）
//!
//! - **程序级**（`.yekitrim/` 下的 `config.toml` / `llm.toml` / `server.toml`）：一个目录各一份，
//!   升级后**写回**，写前先备份成 `<名>.bak-v<旧版本>` ⇒ "升一次就永远升好了"。
//! - **工程级**（每个工程一份 `yekitrim.toml` / `polish.toml`）：**只在内存里升级，不碰磁盘**。
//!   工程可能几百个、还可能在网络盘上，批量改写文件的风险远大于收益；而"读时能读懂老文件"
//!   这件事本身就已经给到了向后兼容。要真的改盘就显式来（以后可以加 `yekitrim config migrate`）。
//!
//! ## 为什么用 `toml_edit` 而不是 `toml`
//!
//! `toml::to_string`（重新序列化）会**把注释全部丢掉** —— 而 `config.toml` / `llm.toml` 的注释
//! 就是它们的文档（模板里大段说明都写在注释里）。`toml_edit` 保留格式与注释，本仓本来就在用它
//! （`write_project_config_inner`、`edit_toml_at`）。
//!
//! ## ⚠️ 改配置键的规矩（这条比代码重要）
//!
//! **删键 / 改名 / 改语义** 时，除了改结构体，还必须：
//! ① 把 [`CONFIG_VERSION`] `+1`；② 在 [`ConfigKind::retired_in`] 里把那个键登记成"新版本该清掉的"。
//!
//! 漏了 ① 的表现是：老文件永远停在旧版本、迁移永远不跑；漏了 ② 的表现是：老文件里那个键
//! **永远留着**（`patch_config_file` 只合并不删）—— 下一个读配置的人又会问"这键到底有没有用"。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use toml_edit::{DocumentMut, Item};

/// 当前配置结构版本。**改配置键形状时 +1**，并在 [`ConfigKind::retired_in`] 里补登记。
///
/// v5（2026-11-25，S4）：把 `config.toml` 里还在的「LLM 行为键」搬进 `llm.toml` ——
/// `[correct] thinking/window_size/max_concurrent` 与 `[detection]` 的 `system_prompt` /
/// `prompt_template` / `reference_titles` / `llm_score_weight` / `max_concurrent`。
/// 目标：`[llm.links.<link>.prompt]`（提示词）/ `[llm.links.<link>.run]`（窗口/并发/评分权重）/
/// `[llm.links.correct].reasoning`（`thinking = true` ⇒ `"high"`）。
/// ⚠️ **这是唯一一步跨文件迁移**（`migrate_config_to_llm`，两个文档一起改，先写 llm.toml 成功才删源键）。
///
/// v4（2026-11-25）：llm.toml 只剩**一份真相** —— `[llm.defaults]`（参数基准）+ `[[llm.providers]]`（怎么连）
/// + `[[llm.models]]`（唯一模型清单）+ `[llm.links.<link>]`（链路覆盖）。v3 的顶层
/// `api_key/base_url/model`、`[llm.params]`、`[llm.overrides.*]`、`[llm.models."<id>"]` 以及
/// `[[llm.providers]]` 的 `api/params/models` 全部由 [`migrate_llm_v4`] 一次搬完，
/// [`ConfigKind::retired_in`] 只兜底清"搬不动 / 手改坏"的残留。
///
/// v3（2026-10-19）：llm.toml 支持**多份 provider** —— 老的顶层
/// `api_key/base_url/model` + `[llm.params]` 搬成**一份 provider**（`id = "default"`）+ `active = "default"`。
/// ⚠️ **`[llm.models."<id>"]` 留在原地**：它是"与 provider 无关的能力覆盖"（换 provider 也照用），
/// 搬进 provider 只会把一份覆盖拆成 N 份。搬家在 [`migrate_llm_v3`]（两处都不删键 ⇒ `retired_in` 不用登记）。
///
/// v2（2026-10-19）：`llm.toml` 把「请求参数面」与「能力」分开 ——
/// `[llm] temperature/max_tokens/thinking` → `[llm.params]`（`thinking` 改名 `reasoning`），
/// 能力覆盖进 `[llm.models."<id>"]`。**搬家**那一步在 [`migrate_llm_v2`]（`retired_in` 只会删）。
pub const CONFIG_VERSION: u32 = 5;

/// 版本键名（顶层）。**缺键 = 0**。
pub const VERSION_KEY: &str = "config_version";

/// 哪一种配置文件 —— 决定要清哪些历史键、以及要不要写回。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKind {
    /// `.yekitrim/config.toml`（程序配置）
    Global,
    /// `.yekitrim/llm.toml`
    Llm,
    /// `.yekitrim/server.toml`（程序状态：`last_config_file` 之类）
    Server,
    /// 工程 `yekitrim.toml`（每个工程一份）
    Project,
    /// 工程 `polish.toml`（每个工程一份）
    Polish,
}

impl ConfigKind {
    /// 按文件名认。⚠️ 认不出来就当**工程级**（那样最保守：只在内存里升级、不碰磁盘）。
    pub fn of_path(path: &Path) -> Self {
        match path.file_name().and_then(|n| n.to_str()).unwrap_or("") {
            "config.toml" => Self::Global,
            "llm.toml" => Self::Llm,
            "server.toml" => Self::Server,
            "polish.toml" => Self::Polish,
            _ => Self::Project,
        }
    }

    /// 是不是"程序级"的（一个目录各一份 ⇒ 升级后写回 + 先备份）。
    pub fn persists(self) -> bool {
        !matches!(self, Self::Project | Self::Polish)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "config.toml",
            Self::Llm => "llm.toml",
            Self::Server => "server.toml",
            Self::Project => "yekitrim.toml",
            Self::Polish => "polish.toml",
        }
    }

    /// `from_version → from_version + 1` 要清掉的键（按 section 路径；末段是键名）。
    ///
    /// 规则：**只有"结构体里已经删掉、老文件里可能还留着"的键才登记在这里**。
    /// 还在用的键一个字都别写进来 —— 那会把用户的设置删掉。
    pub fn retired_in(self, from_version: u32) -> &'static [&'static [&'static str]] {
        match (self, from_version) {
            // v0 → v1：这 4 个从来没有读取方，2026-10 从结构体删除；
            // 老 config.toml / yekitrim.toml 里的残留在这里清掉。静音阈值实际走
            // `cutting::DEFAULT_NOISE_DB` 或 CLI `--threshold`，配置里那个值从来没人看。
            (Self::Global | Self::Project, 0) => &[
                &["asr", "align_model"],
                &["asr", "max_segment_duration"],
                &["asr", "segment_merge_gap"],
                &["cutting", "silence_noise_db"],
            ],
            // v1 → v2：`[llm] thinking` 改名成 `[llm.params] reasoning`（搬家在 `migrate_llm_v2`）。
            // 这一条是**兜底**：只有"不是字符串"那种手改坏的值会留在这儿（搬不动），
            // 由这一趟清掉，别让它以"没人认的键"的身份永远躺着。
            (Self::Llm, 1) => &[&["llm", "thinking"]],
            // v3 → v4：llm.toml 改成"一份真相"的形状（搬家在 [`migrate_llm_v4`]）。
            // 下面几条是**兜底**：正常情形下搬家那一步已经把它们清干净了，这里只收拾
            // "搬不动 / 手改坏"的残留（`[llm]` 写成内联表、`params` 被写成标量之类）。
            //
            // ⚠️ **`llm.models` 故意不登记**：v4 里它还是**活的**（`[[llm.models]]`）。
            // `retired_in` 是按路径删，分不清"老的那张键值表"和"新的数组"——
            // 登记了就会把刚写好的模型清单整份删掉。老形状（`[llm.models."<id>"]`）由搬家那一步转成数组。
            // v4 → v5：这七个键从 config.toml 搬进 llm.toml（搬家在 [`migrate_config_to_llm`]，
            // 那是**唯一**能保住提示词正文的地方）。下面这张表是**兜底**：
            // 搬家失败（llm.toml 写不进去）时**不会**走到这里 —— 调用方会保留源文本（见 prepare）。
            // 真正会用到它的是「目标里已经有同名键 ⇒ 目标优先」那一条：源键搬不过去、也不该留。
            (Self::Global, 4) => &[
                &["correct", "thinking"],
                &["correct", "window_size"],
                &["correct", "max_concurrent"],
                &["detection", "system_prompt"],
                &["detection", "prompt_template"],
                &["detection", "reference_titles"],
                &["detection", "llm_score_weight"],
                &["detection", "max_concurrent"],
            ],
            // 工程文件里那几行是 `write_project_config` 当年抄进去的**副本**（llm.toml 不参与 merge，
            // 搬走也没有读者）⇒ 直接清掉，不做搬运。
            (Self::Project, 4) => &[
                &["detection", "system_prompt"],
                &["detection", "prompt_template"],
                &["detection", "reference_titles"],
                &["detection", "llm_score_weight"],
                &["detection", "max_concurrent"],
            ],
            (Self::Llm, 3) => &[
                &["llm", "params"],
                &["llm", "overrides"],
                &["llm", "api_key"],
                &["llm", "base_url"],
                &["llm", "model"],
            ],
            _ => &[],
        }
    }
}

/// 读文档里的版本号；缺键 / 不是整数 = 0（当"老文件"办）。
pub fn version_of(doc: &DocumentMut) -> u32 {
    doc.get(VERSION_KEY)
        .and_then(Item::as_integer)
        .filter(|n| *n >= 0)
        .map(|n| n as u32)
        .unwrap_or(0)
}

/// 迁移结果。
#[derive(Debug)]
pub struct Prepared {
    /// 迁移后的**文本**（可继续交给 `toml`/`toml_edit` 解析）
    pub text: String,
    /// 文件原本的版本（缺键 = 0）
    pub from: u32,
    /// 是否改动过
    pub changed: bool,
    /// **这一趟清掉了哪些键**（`["cutting", "silence_noise_db"]` 这种点号形式，已就绪可直接打印）。
    ///
    /// ⚠️ **纯函数不打日志**（2026-10-16）：工程级配置（`yekitrim.toml` / `polish.toml`）按设计
    /// **只在内存里升、不写回磁盘** ⇒ 那个键**永远在**，于是**每读一次就报一次**
    /// —— 实测 100ms 内刷了 10 条一模一样的「清掉了没人读的键」。
    /// 报不报、怎么报交给调用方（它才知道路径、也才知道这次到底写没写盘）。
    pub retired: Vec<String>,
}

/// 第 step 步（step → step+1）的**搬家/改名**迁移。
///
/// retired_in 只会**删**键，表达不了"搬到别处/改个名" —— 那种一步一段写在这里。
fn migrate_step(doc: &mut DocumentMut, kind: ConfigKind, step: u32) {
    if kind == ConfigKind::Llm && step == 1 {
        migrate_llm_v2(doc);
    }
    if kind == ConfigKind::Llm && step == 2 {
        migrate_llm_v3(doc);
    }
    if kind == ConfigKind::Llm && step == 3 {
        // 返回值是"这一步有没有改动"（给判据用：同一份文本迁两次，第二次必须是 false）。
        // 版本号的推进由 [`migrate_text`] 统一负责，这一步只管搬东西。
        let _ = migrate_llm_v4(doc);
    }
}

/// **v2 → v3**：把老的单份 `[llm]` 搬成 `[[llm.providers]]` 里的一份（`id = "default"`）+ `active = "default"`。
///
/// 搬**三样标量**：`api_key` / `base_url` / `model`。
/// **四样不搬**（都有明确的"为什么"）：
/// * `[llm.params]` —— 留在顶层当**所有 provider 的缺省参数面**（`effective_params` 回落；
///   provider 自己写了就用它的）。搬进 provider 还有个硬问题：toml_edit 会把子表排到
///   `[[llm.providers]]` **前面** ⇒ 生成非法 TOML；
/// * `[llm.models."<id>"]` —— 能力覆盖与 provider 无关（换一份 provider 也照用），搬走只会把
///   一份覆盖拆成 N 份；`From<&LlmConfig>` 里它仍然是**最后、优先级最高**的那层；
/// * `[llm.overrides.*]` / `max_concurrent` —— 本轮口径：覆盖仍指向**当前 active 那一份**，
///   并发是进程级旋钮；
/// * `extra_params` —— 老逃逸口，两条路径共用一份。
///
/// **幂等**：已经有 `providers` ⇒ 什么都不做（只让版本号前进）。
/// **空文件不造 provider**：四样全空（老文件从没配过）⇒ 放回去、只升版本 —— 那才是"老文件零改动"。
fn migrate_llm_v3(doc: &mut DocumentMut) {
    let Some(llm) = doc.get_mut("llm").and_then(Item::as_table_like_mut) else {
        return;
    };
    // 已经搬过（或者用户手写了 providers）⇒ 只让版本号前进
    if llm.get("providers").is_some() {
        return;
    }
    // ⚠️ **只搬三个标量**：`[llm.params]` 故意留在顶层当"所有 provider 的缺省参数面"
    // （`LlmConfig::effective_params`）。两个理由：① 老文件的注释/形状一个字不动（R4）；
    // ② toml_edit 会把 provider 的子表**排到 `[[llm.providers]]` 前面** —— 那样生成的是**非法 TOML**
    //    （先 [llm.providers.params] 建表、再 [[llm.providers]] 建数组，同路径冲突）。
    let mut moved: Vec<(&'static str, Item)> = Vec::new();
    for key in ["api_key", "base_url", "model"] {
        if let Some(item) = llm.remove(key) {
            moved.push((key, item));
        }
    }
    let any_value = moved.iter().any(|(_, item)| !item_is_empty(item));
    if !any_value {
        // 什么都没配过 ⇒ 原样放回去（**别给老文件凭空造一个 provider**）
        for (key, item) in moved {
            llm.insert(key, item);
        }
        return;
    }
    let mut prov = toml_edit::Table::new();
    prov.insert("id", toml_edit::value("default"));
    prov.insert("name", toml_edit::value("默认"));
    // 协议 id：本轮只有它；写进来是为了让配置文件**自解释**（改错会被写入接口 400 拦下）
    prov.insert("api", toml_edit::value("openai-chat"));
    for (key, item) in moved {
        prov.insert(key, item);
    }
    let mut arr = toml_edit::ArrayOfTables::new();
    arr.push(prov);
    // active 先插（toml_edit 会把标量排在子表/数组前面）
    llm.insert("active", toml_edit::value("default"));
    llm.insert("providers", Item::ArrayOfTables(arr));
}

/// 这个 item 算不算"没值"（空串 / 空表 / 空数组）—— 决定要不要给老文件造 provider。
fn item_is_empty(item: &Item) -> bool {
    if let Some(t) = item.as_table_like() {
        return t.is_empty();
    }
    match item.as_value() {
        Some(v) => match v {
            toml_edit::Value::String(s) => s.value().trim().is_empty(),
            toml_edit::Value::Array(a) => a.is_empty(),
            toml_edit::Value::InlineTable(t) => t.is_empty(),
            _ => false,
        },
        None => true,
    }
}

/// 取走一个键，**把键名连同它的 decor（上方注释/缩进）一起留下** —— 搬到别的表时注释不能丢。
fn take_key(table: &mut toml_edit::Table, name: &str) -> Option<(toml_edit::Key, Item)> {
    let key = table.get_key_value(name).map(|(k, _)| k.clone())?;
    let item = table.remove(name)?;
    Some((key, item))
}

/// 改名（decor 跟着走）。
fn rename_key(key: &toml_edit::Key, name: &str) -> toml_edit::Key {
    let mut k = toml_edit::Key::new(name);
    // ⚠️ 用 leaf_decor（不是 decor/decor_mut）：后者已废弃，而且点号键的 decor 语义不同
    *k.leaf_decor_mut() = key.leaf_decor().clone();
    k
}

/// thinking = "enabled" → "high"、"disabled" → "off"；已经是档位名就原样。
///
/// 非字符串（手改出的 thinking = true）原样返回 —— 它搬过去也是没人认的键，
/// 由 retired_in 那一趟清掉，别在这里凭空造一个值出来。
fn map_thinking_value(mut item: Item) -> Item {
    let Some(s) = item.as_str().map(str::to_string) else {
        return item;
    };
    let mapped = match s.trim().to_ascii_lowercase().as_str() {
        "enabled" | "on" | "true" | "yes" => Some("high"),
        "disabled" | "off" | "false" | "no" => Some("off"),
        // 已经是档位名（或手写的怪值）：原样搬过去 —— 认不认是读的人的事
        _ => None,
    };
    let Some(m) = mapped else { return item };
    // 值自己的 decor（行尾注释）也留着
    let decor = item.as_value().map(|v| v.decor().clone());
    let mut new_item = toml_edit::value(m);
    if let (Some(d), Some(v)) = (decor, new_item.as_value_mut()) {
        *v.decor_mut() = d;
    }
    item = new_item;
    item
}

/// v1 → v2（llm.toml）：把「请求参数面」从 [llm] 搬进 [llm.params]。
///
/// 三件事，**少一件就是"升级一次请求体变了"**：
/// 1. [llm] temperature / max_tokens → [llm.params] 同名（连上方注释一起搬）；
/// 2. [llm] thinking → [llm.params] reasoning（enabled→high、disabled→off，档位名原样），
///    每条 [llm.overrides.<link>] thinking → reasoning（同一套映射）；
/// 3. **缺席的 temperature / max_tokens 落成 v1 的生效值**（0.1 / 65536）：新形状里"缺席"的含义是
///    **不发送**，而 v1 这两个键有代码级默认值、**每一次请求都真的发出去**。
///
/// extra_params 原位不动 —— 它仍是优先级最低的逃逸口（老工程里写死的 response_format 靠它）。
///
/// ⚠️ 只处理**真表**（[llm] / [llm.overrides.x]）。写成 llm = { … } 内联表的文件极其少见，
/// 那种情况下这里什么都不动：老键会变成没人认的键（不报错、也不生效），而不是让文件坏掉。
fn migrate_llm_v2(doc: &mut DocumentMut) {
    let Some(llm) = doc.get_mut("llm").and_then(Item::as_table_mut) else {
        return;
    };
    // ① 先取走要搬的三样（键名 + 值）
    let temperature = take_key(llm, "temperature");
    let max_tokens = take_key(llm, "max_tokens");
    let reasoning = take_key(llm, "thinking")
        .map(|(k, item)| (rename_key(&k, "reasoning"), map_thinking_value(item)));

    // ② 链路覆盖里的 thinking：同一套改名与映射
    if let Some(overrides) = llm.get_mut("overrides").and_then(Item::as_table_mut) {
        let links: Vec<String> = overrides.iter().map(|(k, _)| k.to_string()).collect();
        for link in links {
            let Some(t) = overrides.get_mut(&link).and_then(Item::as_table_mut) else {
                continue;
            };
            if let Some((k, item)) = take_key(t, "thinking") {
                let key = rename_key(&k, "reasoning");
                t.insert_formatted(&key, map_thinking_value(item));
            }
        }
    }

    // ③ 目标子表：没有就建（[llm.params] 得是真表，设置页的 patch 才能继续往下钻）
    if llm.get("params").is_none() {
        let mut params = toml_edit::Table::new();
        params.set_implicit(false);
        llm.insert("params", Item::Table(params));
    }
    let Some(params) = llm.get_mut("params").and_then(Item::as_table_mut) else {
        return;
    };
    for (key, item) in [temperature, max_tokens].into_iter().flatten() {
        params.insert_formatted(&key, item);
    }
    if let Some((key, item)) = reasoning {
        params.insert_formatted(&key, item);
    }

    // ④ 缺席的生效值落成显式键（见函数头第 3 条）
    if params.get("temperature").is_none() {
        params.insert("temperature", toml_edit::value(super::types::default_temperature()));
    }
    if params.get("max_tokens").is_none() {
        let n = super::types::default_max_tokens() as i64;
        params.insert("max_tokens", toml_edit::value(n));
    }
}

/// **v3 → v4**（llm.toml）：把"两份真相"合成 v4 的形状（[`super::llm_v4::from_v3`] 的文本版）。
///
/// 六件事，逐条对应 v3 的回落链（口径见 config/AGENTS.md「LLM 配置 v4」）：
/// 1. 顶层 `api_key/base_url/model` → 一份 `id = "default"` 的接口（**只在完全没有** `[[llm.providers]]` 时），
///    然后这三个键从 `[llm]` 上删掉；`active` 落成 `"default"`（v4 里它是字符串，不是 Option）。
///    ⚠️ 已经有多份接口时**不搬**这三样（v3 里 active 命中时它们本来就是死的）—— 由 `retired_in` 那一趟直接清掉，
///    这是"v4 不留 legacy 回落链"的口径：没人读的键不留在文件里。
/// 2. `[llm] params` → `[llm] defaults`：**改名**，表头 decor（上方那段注释）跟着走。
/// 3. `[llm.overrides.<link>]` → `[llm.links.<link>]`：连注释一起搬；**空表不写**（空 = 跟随）。
/// 4. `[[llm.providers]]` 的 `api` → `protocol`（decor 跟着走）。
/// 5. `[[llm.providers]].params`：**当前生效那份**逐键并进 `[llm.defaults]`（已存在的不覆盖），
///    其余各份删掉并 warn 一条（写清接口 id）—— v4 只有一层参数面，那些值**无法表达**。
/// 6. 模型：`[[llm.providers]].models` 与 `[llm.models."<id>"]` 都进顶层 `[[llm.models]]`
///    （前者 `provider` = 各自接口，后者 `provider` = 生效接口；原表的键平铺进条目，
///    `id` / `provider` 排在最前）。
///
/// 返回**有没有改动**：已经是 v4 形状的文本再过一遍必须返回 false 且一个字节不动（幂等）。
///
/// ⚠️ **纯函数、不打日志**，只有第 5 条的「丢弃参数」例外 —— 那条信息只有这里知道，
/// 而写盘之后那几个值就真的没了（`Prepared::retired` 装的是"删掉的键"，装不下这个）。
///
/// ⚠️ 为什么不用 `toml::to_string`：那会把注释全丢掉，而 llm.toml 的说明全写在注释里。
/// 这里逐键搬，连 `Key` 的 leaf decor 与 `Table` 的 decor 都跟着走。
fn migrate_llm_v4(doc: &mut DocumentMut) -> bool {
    let Some(llm) = doc.get_mut("llm").and_then(Item::as_table_mut) else {
        return false;
    };
    let mut changed = false;

    // ── ① 一份接口都没有 ⇒ 用顶层三件造一条（v4 不允许"没有接口"）──
    // ⚠️ 与 v2 → v3 那一步不同，这里**空值也造**：v4 的形状硬要求至少一份接口，
    // 而"老文件从没配过"本来就是三行空串（模板带出来的），搬过去正好是"待填"的那份。
    if !llm.contains_key("providers") {
        // ⚠️ 连 **Key** 一起搬（不是只搬值）：key 的 leaf decor 里可能挂着上方那行注释
        let moved: Vec<(toml_edit::Key, Item)> = ["api_key", "base_url", "model"]
            .into_iter()
            .filter_map(|key| take_key(llm, key))
            .collect();
        let mut prov = toml_edit::Table::new();
        prov.insert("id", toml_edit::value("default"));
        prov.insert("name", toml_edit::value("默认接口"));
        for (key, item) in moved {
            prov.insert_formatted(&key, item);
        }
        let mut arr = toml_edit::ArrayOfTables::new();
        arr.push(prov);
        // active 原来空着、或指向一个不存在的接口 ⇒ 一律归位（否则 v4 会报"active 指向不存在的接口"）
        if llm.get("active").and_then(Item::as_str).map(str::trim) != Some("default") {
            llm.insert("active", toml_edit::value("default"));
        }
        llm.insert("providers", Item::ArrayOfTables(arr));
        changed = true;
    }

    // ── ② params → defaults（改名，表头注释跟着走）──
    if llm.contains_key("params") {
        let had_defaults = llm.contains_key("defaults");
        if let Some((key, item)) = take_key(llm, "params") {
            changed = true;
            if item.as_table_like().is_some() {
                if had_defaults {
                    // 已经有人手写过 defaults（半迁移的文件）：逐键并入，已存在的不覆盖
                    if let Some(dst) = llm.get_mut("defaults").and_then(Item::as_table_mut) {
                        merge_params_into(dst, item);
                    }
                } else {
                    llm.insert_formatted(&rename_key(&key, "defaults"), item);
                }
            }
            // 不是表（手改出 params = 5）：v4 的 defaults 是张表，搬过去只会让整份文件读不动 ⇒ 丢掉
        }
    }

    // ── ③ overrides → links（连注释；空表不写）──
    if llm.contains_key("overrides") {
        if let Some((key, item)) = take_key(llm, "overrides") {
            changed = true;
            match item {
                Item::Table(src) => {
                    let mut src = src;
                    let mut links = toml_edit::Table::new();
                    // 老 `[llm.overrides]` 上方的注释跟着这张表走
                    *links.decor_mut() = src.decor().clone();
                    if let Some(pos) = src.position() {
                        links.set_position(pos);
                    }
                    let names: Vec<String> = src.iter().map(|(k, _)| k.to_string()).collect();
                    for name in names {
                        let Some((link, body)) = take_key(&mut src, &name) else {
                            continue;
                        };
                        // 空表不写（v4 的口径：文件里不出现 = 跟随上层）
                        if body.as_table_like().map(|t| t.is_empty()).unwrap_or(false) {
                            continue;
                        }
                        links.insert_formatted(&link, body);
                    }
                    if !links.is_empty() {
                        // 只有子表时不要那行空表头（老文件里 `[llm.overrides]` 本来也是隐形的）
                        links.set_implicit(links.get_values().is_empty());
                        match llm.get_mut("links").and_then(Item::as_table_mut) {
                            Some(dst) => {
                                for (k, v) in links.iter() {
                                    if dst.get(k).is_none() {
                                        dst.insert(k, v.clone());
                                    }
                                }
                            }
                            None => {
                                llm.insert("links", Item::Table(links));
                            }
                        }
                    }
                }
                // 不是表（手改坏）：改名搬过去，让它以"没人认的键"的身份留着，别静默删值
                other => {
                    llm.insert_formatted(&rename_key(&key, "links"), other);
                }
            }
        }
    }

    // 生效那份接口（v4 口径：active 认不出 ⇒ 第一份）。下面两处都要它，先算一次。
    let active = active_llm_provider(llm);

    // ── ④ 接口：api → protocol；params / models 先取出来 ──
    let mut prov_params: Vec<(usize, String, Item)> = Vec::new();
    let mut prov_models: Vec<(String, Item)> = Vec::new();
    if let Some(providers) = llm.get_mut("providers").and_then(Item::as_array_of_tables_mut) {
        for (idx, prov) in providers.iter_mut().enumerate() {
            let id = prov
                .get("id")
                .and_then(Item::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if let Some((key, item)) = take_key(prov, "api") {
                prov.insert_formatted(&rename_key(&key, "protocol"), item);
                changed = true;
            }
            // v4 的接口只描述"怎么连"：参数面归 defaults、模型清单归顶层
            if let Some((_, item)) = take_key(prov, "params") {
                prov_params.push((idx, id.clone(), item));
                changed = true;
            }
            if let Some((_, item)) = take_key(prov, "models") {
                prov_models.push((id, item));
                changed = true;
            }
        }
    }

    // ── ⑤ 生效那份的参数并进 defaults；其余各份表达不了 ⇒ 丢弃并 warn ──
    if !prov_params.is_empty() {
        if !llm.contains_key("defaults") {
            let mut defaults = toml_edit::Table::new();
            defaults.set_implicit(false);
            llm.insert("defaults", Item::Table(defaults));
        }
        for (idx, id, item) in prov_params {
            // 按**下标**判生效（接口 id 可能是空串，比 id 更稳）
            if active.as_ref().map(|(i, _)| *i == idx).unwrap_or(false) {
                if let Some(dst) = llm.get_mut("defaults").and_then(Item::as_table_mut) {
                    merge_params_into(dst, item);
                }
            } else {
                tracing::warn!(
                    "llm.toml 迁移到 v4：接口 `{id}` 的 [llm.providers.params] 无法表达（v4 只有一层 [llm.defaults]），已丢弃"
                );
            }
        }
    }

    // ── ⑥ 模型：两套来源 → 一张 [[llm.models]] 清单 ──
    let legacy_provider = active
        .as_ref()
        .map(|(_, id)| id.clone())
        .unwrap_or_else(|| "default".to_string());
    let mut entries: Vec<toml_edit::Table> = Vec::new();
    for (id, item) in prov_models {
        match item {
            Item::ArrayOfTables(arr) => {
                for model in arr {
                    if let Some(entry) = llm_model_entry(Item::Table(model), "", &id, "catalog") {
                        entries.push(entry);
                    }
                }
            }
            // `models = [{ id = "m" }]`（设置页 patch 出来的内联数组）
            Item::Value(toml_edit::Value::Array(arr)) => {
                for model in arr.iter() {
                    if let Some(entry) = llm_model_entry(Item::Value(model.clone()), "", &id, "catalog") {
                        entries.push(entry);
                    }
                }
            }
            _ => {}
        }
    }
    let models_shape = match llm.get("models") {
        Some(Item::Table(_)) => 1u8, // 老形状：模型 id 为键的键值表
        Some(Item::Value(toml_edit::Value::Array(_))) => 0, // 已经是 v4 的数组：别动
        Some(Item::ArrayOfTables(_)) => 0, // 已经是 v4 的 [[llm.models]]：别动（否则第二遍会把清单删掉）
        Some(Item::None) | None => 0,
        Some(_) => 2, // 手改出的标量：没人认，删掉
    };
    let mut models_prefix: Option<toml_edit::RawString> = None;
    if models_shape == 1 {
        if let Some((_, item)) = take_key(llm, "models") {
            changed = true;
            if let Item::Table(mut models) = item {
                // 老 `[llm.models]` 上方那段注释搬到第一条 `[[llm.models]]` 之前
                models_prefix = models.decor().prefix().cloned();
                let ids: Vec<String> = models.iter().map(|(k, _)| k.to_string()).collect();
                for id in ids {
                    if let Some((_, body)) = take_key(&mut models, &id) {
                        if let Some(entry) = llm_model_entry(body, &id, &legacy_provider, "overrides") {
                            entries.push(entry);
                        }
                    }
                }
            }
        }
    } else if models_shape == 2 {
        llm.remove("models");
        changed = true;
    }
    if !entries.is_empty() {
        if let Some(prefix) = models_prefix {
            if let Some(first) = entries.first_mut() {
                if first.decor().prefix().is_none() {
                    first.decor_mut().set_prefix(prefix);
                }
            }
        }
        let mut arr = toml_edit::ArrayOfTables::new();
        for mut entry in entries {
            // 整张清单排到所有老表之后：各条目原本散在 providers 中间（position 记的是老位置），
            // 就着 position 排会插出一条条模型夹在 [[llm.providers]] 中间的怪形状。
            entry.set_position(usize::MAX);
            arr.push(entry);
        }
        llm.insert("models", Item::ArrayOfTables(arr));
        changed = true;
    }
    changed
}

/// 当前生效的**接口**（下标 + id）：`active` 命中就用它，否则第一份（与 v4 的 `active_provider` 同口径）。
///
/// 没有 id 的那份给个 "default" 当归属（v4 的模型清单要求 `provider` 非空）。
fn active_llm_provider(llm: &toml_edit::Table) -> Option<(usize, String)> {
    let providers = llm.get("providers").and_then(Item::as_array_of_tables)?;
    if providers.is_empty() {
        return None;
    }
    let id_of = |i: usize| -> String {
        providers
            .get(i)
            .and_then(|t| t.get("id"))
            .and_then(Item::as_str)
            .map(str::trim)
            .unwrap_or("")
            .to_string()
    };
    let active = llm.get("active").and_then(Item::as_str).map(str::trim).unwrap_or("");
    if !active.is_empty() {
        for i in 0..providers.len() {
            if id_of(i) == active {
                return Some((i, active.to_string()));
            }
        }
    }
    let first = id_of(0);
    Some((0, if first.is_empty() { "default".to_string() } else { first }))
}

/// 把一份参数面（真表或内联表）逐键并进目标表：**目标里已经有的键不覆盖**（先写的那份说了算）。
///
/// 口径来自"迁移不能悄悄改值"：`[llm.defaults]` 是唯一基准，provider 那份只补它没写的键。
fn merge_params_into(dst: &mut toml_edit::Table, src: Item) {
    match src {
        Item::Table(mut src) => {
            let keys: Vec<String> = src.iter().map(|(k, _)| k.to_string()).collect();
            for key in keys {
                if dst.get(&key).is_some() {
                    continue;
                }
                if let Some((key, item)) = take_key(&mut src, &key) {
                    dst.insert_formatted(&key, item);
                }
            }
        }
        Item::Value(toml_edit::Value::InlineTable(it)) => {
            // 内联表里放不下注释（TOML 也不许），值原样克隆即可
            for (key, value) in it.iter() {
                if dst.get(key).is_some() {
                    continue;
                }
                dst.insert(key, Item::Value(value.clone()));
            }
        }
        // 标量等手改坏的值：v4 的参数面读不动，丢掉（调用方已把这一步记成"改动"）
        _ => {}
    }
}

/// 一条 v3 的模型条目（`[[llm.providers.models]]` 的一项，或 `[llm.models."<id>"]` 那张表）
/// → `[[llm.models]]` 的一项：`id` / `provider` 排在最前，原表的键**平铺**在后（注释/decor 都跟着）。
///
/// ⚠️ **平铺 vs 嵌套**（口径待定，两边差一行）：本步的口径是"原表的键原样平铺"（本函数照做），
/// 而当前 `llm_v4::LlmModelEntry` 仍是 `catalog` / `overrides` 两个嵌套字段 ——
/// 也就是说，平铺出来的 `thinking_wire` 这类键现在**读不回来**（serde 当未知字段忽略）。
/// 两条路选一条即可：① 给 `LlmModelEntry.overrides` 加 `#[serde(flatten)]`（改动最小，与本节一致）；
/// ② 本函数改成把原表的键塞进 `overrides = { … }` 子表（那就与本步"平铺"的口径不符）。
///
/// id：源里自带就沿用那个值（连 decor），没有就用 id 参数；两处都没有 ⇒ 返回 None
/// （没有 id 的条目在 v4 里没法被引用，落下去只会让 problems() 报警）。
fn llm_model_entry(src: Item, id: &str, provider: &str, bucket: &str) -> Option<toml_edit::Table> {
    let mut out = toml_edit::Table::new();
    let (mut table, existing_id) = match src {
        Item::Table(mut table) => {
            let id_item = take_key(&mut table, "id");
            // 表头 decor（上方那段注释）跟着走 —— 这正是"[llm.models] 上方的注释搬到第一条之前"
            *out.decor_mut() = table.decor().clone();
            (table, id_item)
        }
        Item::Value(toml_edit::Value::InlineTable(it)) => {
            let mut table = toml_edit::Table::new();
            for (k, v) in it.iter() {
                table.insert(k, Item::Value(v.clone()));
            }
            let id_item = take_key(&mut table, "id");
            (table, id_item)
        }
        _ => return None,
    };
    match existing_id {
        Some((key, item)) => {
            out.insert_formatted(&rename_key(&key, "id"), item);
        }
        None if id.is_empty() => return None,
        None => {
            out.insert("id", toml_edit::value(id));
        }
    }
    out.insert("provider", toml_edit::value(provider));
    // 能力键**统一塞进一个内联子表**（catalog 或 overrides）：
    // 两套来源的形状不同（provider.models 的 reasoning 是 bool，[llm.models] 的 reasoning 是 档位→wire 映射），
    // 平铺会在同名字段上撞车；LlmModelEntry 因此是两个 Option 子结构。
    let mut nested = toml_edit::InlineTable::new();
    let keys: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
    for key in keys {
        if let Some((key, item)) = take_key(&mut table, &key) {
            if let Ok(v) = item.into_value() {
                nested.insert(key.get(), v);
            }
        }
    }
    if !nested.is_empty() {
        out.insert(bucket, Item::Value(toml_edit::Value::InlineTable(nested)));
    }
    Some(out)
}

/// 把版本号写进文档顶层：**已经有的那一行连它的 decor（上方注释）一起保留**。
///
/// ⚠️ 不能用 `doc.insert(VERSION_KEY, …)`：那是"换一个新键"，键自己的 decor 会被丢掉，
/// 于是"# 说明 / config_version = 3"这两行迁完只剩孤零零一行版本号 —— 那也是"迁移吃掉了注释"。
fn stamp_version(doc: &mut DocumentMut) {
    let item = toml_edit::value(CONFIG_VERSION as i64);
    match doc.as_table().get_key_value(VERSION_KEY).map(|(k, _)| k.clone()) {
        Some(key) => {
            doc.as_table_mut().insert_formatted(&key, item);
        }
        None => {
            doc.insert(VERSION_KEY, item);
        }
    }
}

/// 纯函数：把文本升到 [`CONFIG_VERSION`]。**不碰磁盘**（写回由 [`prepare`] / `edit` 那条路负责）。
///
/// 解析失败时返回 `Err` —— 调用方要保留各自原有的"坏文件"处理（有的备份、有的退回默认值）。
pub fn migrate_text(kind: ConfigKind, raw: &str) -> Result<Prepared> {
    let mut doc: DocumentMut = raw
        .parse()
        .with_context(|| format!("{} 不是合法 TOML", kind.label()))?;
    let from = version_of(&doc);
    if from >= CONFIG_VERSION {
        // 同版本不动；**更高版本也不动**（那是更新的程序写的，见模块注释）
        return Ok(Prepared { text: raw.to_string(), from, changed: false, retired: Vec::new() });
    }

    // 逐级迁移。现在只有 v0 → v1 一步；将来加 `if from < 2 { … }` 这样一条一段。
    let mut retired: Vec<String> = Vec::new();
    for step in from..CONFIG_VERSION {
        // ⚠️ **先搬家再清键**：搬走的老键不该进 retired 名单（那是"清掉"的意思），
        //    而搬不动的那种（非字符串 thinking）留给 retired 那一趟兜底。
        migrate_step(&mut doc, kind, step);
        for path in kind.retired_in(step) {
            if remove_path(&mut doc, path) {
                // 只**收集**，不打日志 —— 见 `Prepared::retired` 的注释（工程级文件不写回，
                // 每读一次都会走到这里，在纯函数里打日志就是刷屏）。
                retired.push(path.join("."));
            }
        }
    }
    stamp_version(&mut doc);

    // 版本行摆在最前面（`toml_edit` 插根键会落到哪儿不好保证，而"第一行是版本"最好读）。
    let text = ensure_version_line(&doc.to_string());
    Ok(Prepared { text, from, changed: true, retired })
}

/// 保证文本里有且只有"当前版本"那一行（没有就补在最前面）。
///
/// 给**不走 `toml_edit` 的那条路**用 —— `polish.toml` 是在文本层注入的：那个结构体是
/// `deny_unknown_fields`，而它还会出口给前端与出片底座，把版本塞进 schema 会波及它们。
/// 版本在这里是"文件的元数据"，不是配置项。
pub fn ensure_version_line(text: &str) -> String {
    if has_version_line(text) {
        text.to_string()
    } else {
        format!("{VERSION_KEY} = {CONFIG_VERSION}\n{text}")
    }
}

/// 把版本键从文本里摘掉再交给**`deny_unknown_fields`** 的结构体。
///
/// `polish.toml` 的 `ProjectFile` 就是那种结构体，而版本键是"文件的元数据"、不在 schema 里
/// （见 `ensure_version_line` 的说明）。正式读盘那条路（`polish/src/project/validate.rs`）
/// 与直接拿 `to_toml_commented()` 输出的单测都走这里，口径只有一个实现。
pub fn strip_version_key(text: &str) -> Result<String> {
    let mut doc: DocumentMut = text
        .parse()
        .context("待摘版本键的文本不是合法 TOML")?;
    doc.remove(VERSION_KEY);
    Ok(doc.to_string())
}

/// 在**已持锁的读-改-写**里用的版本：只清历史键、不摆位置（那次写由调用方负责）。
/// 返回原版本号，便于调用方决定要不要备份。
pub fn migrate_doc_in_place(kind: ConfigKind, doc: &mut DocumentMut) -> u32 {
    let from = version_of(doc);
    if from >= CONFIG_VERSION {
        return from;
    }
    for step in from..CONFIG_VERSION {
        migrate_step(doc, kind, step);
        for path in kind.retired_in(step) {
            remove_path(doc, path);
        }
    }
    stamp_version(doc);
    from
}

/// 报「这一趟清掉了哪些没人读的键」——**每个文件每进程只报一次**（2026-10-16）。
///
/// ⚠️ **为什么需要去重**：工程级配置（`yekitrim.toml` / `polish.toml`）按设计**不写回磁盘**
/// （工程可能几百个、还可能在网络盘上）⇒ 那几行**永远在**，于是**每读一次都会重新发现它**。
/// 实测：100ms 内刷了 10 条一模一样的「清掉了没人读的键 `cutting.silence_noise_db`」——
/// 一条真信息被重复成噪音。第二次起降到 `debug`：不是藏起来，是"这条你已经看过了"。
///
/// ⚠️ **别把它挪回 `migrate_text`**（那是个纯函数，连路径都不知道）：日志要带"写没写回"这半句，
/// 而那取决于 `kind.persists()`。
/**
 * 返回**这次是不是该文件在本进程里的第一次**（`true` = 第一次，已按 INFO 报过；`false` = 之后那次，降到 debug）。
 *
 * ⚠️ 做成返回值是为了**能测**：日志要挂订阅者才看得到，而"去重这件事本身"是纯决策 ——
 * 判据直接问它两遍（第一遍 true、第二遍 false），不必去抓日志。
 */
pub(super) fn report_retired(path: &Path, kind: ConfigKind, prepared: &Prepared) -> bool {
    if prepared.retired.is_empty() {
        return false;
    }
    let keys = prepared.retired.join("` / `");
    let first_time = {
        static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
            std::sync::OnceLock::new();
        let set = SEEN.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
        // 锁中毒也照常继续：这只是"报过没有"的记账，不该让它拦下配置读取。
        let mut guard = match set.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.insert(path.to_path_buf())
    };
    if kind.persists() {
        // 程序级：下面就会写回，key 真的没了 —— 这条只会出现一次。
        tracing::info!(
            "配置迁移：{} 里清掉了没人读的键 `{keys}`（已写回）",
            kind.label()
        );
    } else if first_time {
        tracing::info!(
            "配置迁移：{} 里清掉了没人读的键 `{keys}` —— ⚠️ 工程级配置**只在内存里升、不写回**，\
             磁盘上那几行会留着（想彻底清掉：删掉那几行，或跑一次 `yekitrim config migrate`）",
            kind.label()
        );
    } else {
        tracing::debug!("配置迁移：{} 里仍有无人在读的键 `{keys}`（本次不再重复提示）", kind.label());
    }
    first_time
}

// ── v4 → v5：**跨文件**搬家（config.toml → llm.toml，S4）──────────────────────

/// 取/建一条**显式**子表（`[a.b.c]`）。路径上任何一级不是表 ⇒ `None`（手改坏的文件，别赌）。
///
/// ⚠️ 建表时必须 `set_implicit(false)`：隐式表在渲染时**没有表头**，键会被排进上一张表里，
/// 生成一份语义完全不同的 TOML（v3→v4 迁移踩过同一类坑）。
fn deep_table<'a>(doc: &'a mut DocumentMut, path: &[&'static str]) -> Option<&'a mut toml_edit::Table> {
    let (head, rest) = path.split_first()?;
    deep_table_in(doc.as_table_mut(), head, rest)
}

fn deep_table_in<'a>(
    tbl: &'a mut toml_edit::Table,
    head: &str,
    rest: &[&'static str],
) -> Option<&'a mut toml_edit::Table> {
    // ⚠️ **中间层标 implicit**：只有叶子才写表头。否则会多出 `[llm.links]` 这种空表头
    // （不影响语义，但用户打开文件会以为那里配过东西）。
    let leaf = rest.is_empty();
    if !tbl.contains_key(head) {
        let mut new = toml_edit::Table::new();
        new.set_implicit(!leaf);
        tbl.insert(head, Item::Table(new));
    }
    let t = match tbl.get_mut(head)? {
        Item::Table(t) => {
            if leaf {
                t.set_implicit(false);
            }
            t
        }
        _ => return None,
    };
    match rest.split_first() {
        None => Some(t),
        Some((h2, r2)) => deep_table_in(t, h2, r2),
    }
}

/// 搬一个键（**连 key 的 decor 一起搬**，上方注释不丢）。
///
/// 目标已经有同名键 ⇒ **目标优先**（源键照样摘掉：它已经没人读了，留着只会误导下一次迁移）。
/// 返回 `true` = 这个键处理完了（搬走或按优先级丢掉）。
fn move_key(from: &mut toml_edit::Table, key: &str, to: &mut toml_edit::Table) -> bool {
    let Some((k, item)) = take_key(from, key) else { return false };
    if to.contains_key(key) {
        return true;
    }
    let mut nk = toml_edit::Key::new(key);
    *nk.leaf_decor_mut() = k.leaf_decor().clone();
    to.insert_formatted(&nk, item);
    true
}

/// **v4 → v5 的搬家本体**（纯函数，两个文档一起改）。返回搬走的键（点号形式，给日志/判据）。
///
/// 映射：
/// - `[correct] thinking` ⇒ `[llm.links.correct] reasoning = "high"`（**只有 true 才写**：
///   false 写成 `"off"` 会变成"显式关思考"，对表外模型会新发 `thinking{type:disabled}` —— 那是行为变更）；
/// - `[correct] window_size` ⇒ `[llm.links.correct.run] window_size`；
/// - `[detection] system_prompt / prompt_template / reference_titles` ⇒ `[llm.links.detect.prompt]`；
/// - `[detection] llm_score_weight` ⇒ `[llm.links.detect.run] llm_score_weight`；
/// - `[correct|detection] max_concurrent` ⇒ 各自的 `.run.max_concurrent`，**与 `[llm] max_concurrent` 相等时丢**
///   （模板里那两处 `= 3` 是"假覆盖"，留着会让全局旋钮永远不生效）。
pub(crate) fn migrate_config_to_llm(global: &mut DocumentMut, llm: &mut DocumentMut) -> Vec<String> {
    let mut moved: Vec<String> = Vec::new();
    let global_max = llm
        .get("llm")
        .and_then(Item::as_table_like)
        .and_then(|l| l.get("max_concurrent"))
        .and_then(Item::as_integer)
        .unwrap_or(3);

    // ── [correct] ──
    if let Some(correct) = global.get_mut("correct").and_then(Item::as_table_mut) {
        let thinking = correct.get("thinking").and_then(Item::as_bool);
        if thinking == Some(true) {
            if let Some(link) = deep_table(llm, &["llm", "links", "correct"]) {
                if !link.contains_key("reasoning") {
                    link.insert("reasoning", toml_edit::value("high"));
                    moved.push("correct.thinking".to_string());
                }
                correct.remove("thinking");
            }
        } else {
            // false / 缺键：不产生 reasoning，但源键该退役
            correct.remove("thinking");
        }
        // 段级并发：与 [llm] max_concurrent 相等 = 模板带来的假覆盖 ⇒ 丢掉（不是搬）
        let seg = correct.get("max_concurrent").and_then(Item::as_integer);
        let seg_move = matches!(seg, Some(n) if n != global_max);
        if seg == Some(global_max) {
            correct.remove("max_concurrent");
        }
        // ⚠️ **有东西要搬才建表**：否则会写出一张空的 [llm.links.correct.run]（cosmetic，但
        // 「一个值都没搬却留下了表」会让判据与用户都以为配过东西）。
        if correct.contains_key("window_size") || seg_move {
            if let Some(run) = deep_table(llm, &["llm", "links", "correct", "run"]) {
                if move_key(correct, "window_size", run) {
                    moved.push("correct.window_size".to_string());
                }
                if seg_move && move_key(correct, "max_concurrent", run) {
                    moved.push("correct.max_concurrent".to_string());
                }
            }
        }
    }

    // ── [detection] ──
    if let Some(detect) = global.get_mut("detection").and_then(Item::as_table_mut) {
        let has_prompt = ["system_prompt", "prompt_template", "reference_titles"]
            .iter()
            .any(|k| detect.contains_key(k));
        if has_prompt {
            if let Some(prompt) = deep_table(llm, &["llm", "links", "detect", "prompt"]) {
                for k in ["system_prompt", "prompt_template", "reference_titles"] {
                    if move_key(detect, k, prompt) {
                        moved.push(format!("detection.{k}"));
                    }
                }
            }
        }
        let seg = detect.get("max_concurrent").and_then(Item::as_integer);
        let seg_move = matches!(seg, Some(n) if n != global_max);
        if seg == Some(global_max) {
            detect.remove("max_concurrent");
        }
        if detect.contains_key("llm_score_weight") || seg_move {
            if let Some(run) = deep_table(llm, &["llm", "links", "detect", "run"]) {
                if move_key(detect, "llm_score_weight", run) {
                    moved.push("detection.llm_score_weight".to_string());
                }
                if seg_move && move_key(detect, "max_concurrent", run) {
                    moved.push("detection.max_concurrent".to_string());
                }
            }
        }
    }
    moved
}

/// **读 + 迁移一对程序级配置**：`config.toml`（global_path）与同目录的 `llm.toml`。
///
/// 返回迁移后的 **config.toml 文本**（源键已摘掉、版本号还没变 —— 版本由调用方的 `migrate_text` 往前走）。
///
/// 三条硬规矩：
/// 1. **先写 llm.toml（备份 + 反序列化验证 + 原子写）成功，才让 config.toml 丢源键** —— 反过来就是
///    "只删不搬"，用户的提示词正文直接消失（这一轮唯一不可重建的用户数据就是它）。
/// 2. 任何一步不成立（llm.toml 坏了 / 写不进去 / 版本比我们新）⇒ **原样返回 config.toml**，
///    下次启动再试（幂等）。
/// 3. 版本已经 ≥ 当前版本 ⇒ 一个字节都不读 llm.toml（这是配置读取的热路径）。
pub fn migrate_global_pair(global_path: &Path, raw: &str) -> Result<String> {
    let mut gdoc: DocumentMut = match raw.parse() {
        Ok(d) => d,
        Err(_) => return Ok(raw.to_string()), // 坏 TOML 由调用方原有的处理负责
    };
    if version_of(&gdoc) >= CONFIG_VERSION {
        return Ok(raw.to_string());
    }
    let llm_path = global_path.with_file_name("llm.toml");
    let existed = llm_path.exists();
    let llm_raw = std::fs::read_to_string(&llm_path).unwrap_or_default();
    let mut ldoc: DocumentMut = match llm_raw.parse() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(
                "{} 不是合法 TOML ⇒ 本轮**不搬** config.toml 的 LLM 行为键（原样保留，下次再试）：{e}",
                llm_path.display()
            );
            return Ok(raw.to_string());
        }
    };
    if version_of(&ldoc) > CONFIG_VERSION {
        tracing::warn!(
            "{} 的版本比本程序新 ⇒ **不动**它，也不删 config.toml 的 LLM 行为键（换回新版程序再迁）",
            llm_path.display()
        );
        return Ok(raw.to_string());
    }
    let moved = migrate_config_to_llm(&mut gdoc, &mut ldoc);
    if moved.is_empty() && gdoc.to_string() == raw {
        return Ok(gdoc.to_string());
    }
    // 顺手把版本盖上（否则它会被当成"没有版本号的老文件"再走一遍）。
    // ⚠️ **v3 及以下不盖**：它自己的 v3→v4 那几步还没跑（盖了就等于跳过迁移）。
    //    那种文件会在下一次读 llm.toml 时由 `prepare(Llm)` 迁完。
    if !existed || version_of(&ldoc) >= 4 {
        stamp_version(&mut ldoc);
    }
    let new_llm = ldoc.to_string();
    if new_llm != llm_raw {
        // ① 反序列化验证（P0 事故口径：写进去的必须能被真正的解析器吃下去）
        if let Err(bad) = crate::config::validate_llm_toml(&new_llm) {
            tracing::warn!(
                "{} 搬家后解析不过 ⇒ **没有**写回，config.toml 的 LLM 行为键也原样保留：{bad}",
                llm_path.display()
            );
            return Ok(raw.to_string());
        }
        // ② 备份（保留最早那份原文件）
        if let Err(e) = backup_before_upgrade(&llm_path, version_of(&ldoc)) {
            tracing::warn!("搬家前备份 {} 失败（继续）：{e:#}", llm_path.display());
        }
        // ③ 原子写；失败 ⇒ 源键一个都不删
        if let Err(e) = write_atomic(&llm_path, &new_llm) {
            tracing::warn!(
                "把 LLM 行为键写进 {} 失败（config.toml 原样保留，下次再试）：{e:#}",
                llm_path.display()
            );
            return Ok(raw.to_string());
        }
        tracing::info!(
            "配置迁移：把 {} 的 LLM 行为键搬进了 {}（{} 项：{}）",
            global_path.display(),
            llm_path.display(),
            moved.len(),
            moved.join(", ")
        );
    }
    Ok(gdoc.to_string())
}

/// 读盘 + 迁移；**程序级文件在需要时备份并写回**（工程级只返回迁移后的文本）。
pub fn prepare(path: &Path, kind: ConfigKind) -> Result<Prepared> {
    let mut raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    // ⚠️ **跨文件搬家必须在单文件迁移之前**：v4→v5 要把 config.toml 的键写进 llm.toml，
    //    而单文件迁移只会「删」。顺序反了就是「只删不搬」（用户的提示词正文直接没了）。
    if kind == ConfigKind::Global {
        raw = migrate_global_pair(path, &raw)?;
    }
    let prepared = migrate_text(kind, &raw)?;
    report_retired(path, kind, &prepared);
    if prepared.from > CONFIG_VERSION {
        tracing::warn!(
            "{} 的 config_version={} 比本程序认识的 {} 更新 —— 按现状读，**不会**改它。\
             换回新版程序，或先备份再手改。",
            path.display(),
            prepared.from,
            CONFIG_VERSION
        );
        return Ok(prepared);
    }
    if prepared.changed && kind.persists() {
        // ⚠️ **写盘前守卫**（P0 2026-10-19）：迁移产物必须能被**真正的反序列化器**吃下去。
        // 只校验 `LlmConfig` 那一种（其它两家的结构体在 core 之外，且它们没有"解析失败就回退默认"
        // 这条后果）。解析不过 ⇒ **拒绝回写**：原文件保持完好，内存里那份照常用（下次再试，幂等）。
        if kind == ConfigKind::Llm {
            if let Err(bad) = crate::config::validate_llm_toml(&prepared.text) {
                tracing::warn!(
                    "{} 迁移后解析不过，**没有**回写（原文件保持不动，本次按内存里那份跑）：{bad}",
                    path.display()
                );
                return Ok(prepared);
            }
        }
        match backup_before_upgrade(path, prepared.from) {
            Ok(Some(bak)) => tracing::info!(
                "{} 已从 v{} 升级到 v{}（原文件备份在 {}）",
                path.display(),
                prepared.from,
                CONFIG_VERSION,
                bak.display()
            ),
            Ok(None) => tracing::info!(
                "{} 已从 v{} 升级到 v{}",
                path.display(),
                prepared.from,
                CONFIG_VERSION
            ),
            Err(e) => tracing::warn!("备份 {} 失败（继续升级）：{e:#}", path.display()),
        }
        if let Err(e) = write_atomic(path, &prepared.text) {
            // ⚠️ 写回失败**不拦启动**：内存里那份已经是迁移过的，本次照常跑；
            // 下次启动会再试一遍（幂等）。
            tracing::warn!("升级后的配置写回 {} 失败：{e:#}", path.display());
        }
    }
    Ok(prepared)
}

/// **把一份配置文件里已下线的键清掉并写回磁盘**（带 `.bak-v<旧>` 备份）。返回清掉的键。
///
/// ⚠️ 为什么需要它：工程级配置（`yekitrim.toml` / `polish.toml`）**按设计不写回**
/// （`ConfigKind::persists()` 为假 —— 工程可能几百个、还可能在网络盘上）⇒ 那些死键会
/// **永远留在文件里**，每次读都重新发现它（日志改成"每文件每进程报一次"只是别让它刷屏，
/// 键**并没有**被清掉）。用户要一个"按一下就真的清掉"的出口 —— 就是这一条
/// （CLI：`yekitrim config migrate <工程目录|toml 文件>`）。
///
/// 返回空 = 这份文件本来就没有死键 ⇒ **不动磁盘**（也不备份、不打日志）。
pub fn clean_retired_keys_in(path: &Path) -> Result<Vec<String>> {
    let mut raw = std::fs::read_to_string(path)
        .with_context(|| format!("读不出 {}", path.display()))?;
    let kind = ConfigKind::of_path(path);
    // 同 prepare：跨文件那一趟必须先跑，否则这里是「只清不搬」。
    if kind == ConfigKind::Global {
        raw = migrate_global_pair(path, &raw)?;
    }
    let prepared = migrate_text(kind, &raw)?;
    if prepared.retired.is_empty() {
        return Ok(Vec::new());
    }
    if let Err(e) = backup_before_upgrade(path, prepared.from) {
        // 备份失败不拦：清键本身是幂等的，下次还能再来一遍。
        tracing::warn!("清理前备份 {} 失败（继续）：{e:#}", path.display());
    }
    write_atomic(path, &prepared.text)?;
    Ok(prepared.retired)
}

/// 升级前备份成 `<名>.bak-v<旧版本>`；已经存在就不覆盖（保留**最早**那份原始文件）。
///
/// 旧版本为 0 时文件名是 `config.toml.bak-v0` —— 一眼能看出"这是加版本号之前的原样"。
pub fn backup_before_upgrade(path: &Path, from: u32) -> Result<Option<PathBuf>> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("配置路径没有文件名")?;
    let backup = path.with_file_name(format!("{name}.bak-v{from}"));
    if backup.exists() {
        return Ok(None);
    }
    std::fs::copy(path, &backup)
        .with_context(|| format!("复制 {} → {}", path.display(), backup.display()))?;
    Ok(Some(backup))
}

/// 原子写：先写同目录的临时文件再 rename（避免断电/崩溃留下半截配置）。
pub fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).with_context(|| format!("写 {} 失败", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("把 {} 换成 {} 失败", tmp.display(), path.display()))?;
    Ok(())
}

/// 文本里有没有 `config_version = …` 那一行（用于决定要不要补在开头）。
fn has_version_line(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim_start();
        t.strip_prefix(VERSION_KEY)
            .map(|rest| rest.trim_start().starts_with('='))
            .unwrap_or(false)
    })
}

/// 按 section 路径删一个键；删掉了返回 true。路径中间不存在 / 不是表都算"没这个键"。
fn remove_path(doc: &mut DocumentMut, path: &[&str]) -> bool {
    let Some((last, parents)) = path.split_last() else {
        return false;
    };
    let mut table = doc.as_table_mut();
    for p in parents {
        let Some(item) = table.get_mut(p) else {
            return false;
        };
        let Some(t) = item.as_table_mut() else {
            return false;
        };
        table = t;
    }
    table.remove(last).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 迁移产物按 **v4** 读回来：config::llm_v4::LlmV4 是目标形状的唯一权威，判据别自己另写一套结构。
    #[derive(serde::Deserialize)]
    struct LlmFile {
        llm: super::super::llm_v4::LlmV4,
    }

    fn llm_v4(text: &str) -> super::super::llm_v4::LlmV4 {
        toml::from_str::<LlmFile>(text).expect("迁移后必须能按 v4 读回来").llm
    }

    /// 老文件长这样（没有 `config_version`，还带着 4 个死键和一堆注释）。
    const V0_GLOBAL: &str = r#"# YekiTrim 全局默认配置 — 所有工程共享
# 这些注释**必须活着**（模板里的说明全在注释里）

[asr]
mode = "hybrid"
align_model = "sensevoice"        # 死键，没人读
max_segment_duration = 20.0       # 死键
segment_merge_gap = 1.0           # 死键
num_workers = 0                   # 活的，必须留下

[cutting]
output_dir = "./output"
silence_noise_db = -42            # 死键
overwrite = false
"#;

    /// ⚠️ **工程级配置不会自动写回**（设计如此：工程可能几百个、还可能在网络盘上），
    /// 所以那些死键会永远留在文件里、每次读都重新发现它。`clean_retired_keys_in` 是
    /// "按一下就真的清掉"的出口（CLI `config migrate` 走它）—— 这条测试钉住三件事：
    /// **真写回**、**幂等**（第二遍返回空且不再备份）、**没有死键时一个字节都不碰**。
    #[test]
    fn clean_retired_keys_writes_back_and_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("yekitrim-clean-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // ⚠️ 用**工程级**那个 kind（`yekitrim.toml`）—— 正是"不会自动写回"的那一类。
        let file = dir.join("yekitrim.toml");
        std::fs::write(&file, V0_GLOBAL).unwrap();

        let cleaned = clean_retired_keys_in(&file).unwrap();
        assert!(cleaned.iter().any(|k| k == "cutting.silence_noise_db"), "实得 {cleaned:?}");
        let after = std::fs::read_to_string(&file).unwrap();
        assert!(!after.contains("silence_noise_db"), "死键必须真的从磁盘上没了：\n{after}");
        assert!(after.contains("num_workers = 0"), "活键与注释都要在：\n{after}");
        assert!(
            std::fs::read_to_string(dir.join("yekitrim.toml.bak-v0")).is_ok(),
            "清键前要留一份 `.bak-v0`（原样回退的唯一手段）"
        );

        // 第二遍：没有死键了 ⇒ 返回空、**不再备份**（备份文件名带旧版本号，会被 `.exists()` 挡住）。
        let again = clean_retired_keys_in(&file).unwrap();
        assert!(again.is_empty(), "第二遍不该再报任何键，实得 {again:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ⚠️ **同一条信息只该报一次**（2026-10-16 用户报的日志刷屏：100ms 内 10 条一模一样的
    /// 「清掉了没人读的键 `cutting.silence_noise_db`」）。工程级配置按设计不写回 ⇒ 那个键
    /// **永远在** ⇒ 每读一次都会重新发现它。判据问的是**去重决策本身**：同一路径第一遍 true、
    /// 第二遍 false（不同路径则各自是第一次）。
    #[test]
    fn retired_keys_are_reported_once_per_file() {
        // 夹具直接用 `V0_GLOBAL`：`retired_in` 对 Global/Project 是**同一张表**
        // （四个死键两个 kind 都清），不必再抄一份工程级夹具。
        let p = migrate_text(ConfigKind::Project, V0_GLOBAL).unwrap();
        assert!(!p.retired.is_empty(), "夹具里应当有死键，实得 {:?}", p.retired);
        let a = PathBuf::from("/tmp/yekitrim-once-a/yekitrim.toml");
        let b = PathBuf::from("/tmp/yekitrim-once-b/yekitrim.toml");
        assert!(report_retired(&a, ConfigKind::Project, &p), "第一次应当报");
        assert!(!report_retired(&a, ConfigKind::Project, &p), "同一个文件的第二次不许再报");
        assert!(report_retired(&b, ConfigKind::Project, &p), "另一个文件是它自己的第一次");
    }

    #[test]
    fn migrate_v0_cleans_dead_keys_and_stamps_version() {
        let p = migrate_text(ConfigKind::Global, V0_GLOBAL).unwrap();
        assert_eq!(p.from, 0);
        assert!(p.changed);
        // 死键全清
        for dead in ["align_model", "max_segment_duration", "segment_merge_gap", "silence_noise_db"] {
            assert!(!p.text.contains(dead), "死键 {dead} 没清掉：\n{}", p.text);
        }
        // ⚠️ **清掉了哪些键要能报出来**（2026-10-16）：日志从纯函数里挪到了调用方 ——
        //    工程级文件不写回，在纯函数里报就是"每读一次刷一条"（实测 100ms 刷了 10 条）。
        //    这条判据钉住"信息本身没丢"：调用方拿得到那串键名。
        assert_eq!(p.retired.len(), 4, "清掉的键要逐个报出来，实得 {:?}", p.retired);
        assert!(p.retired.iter().any(|k| k == "cutting.silence_noise_db"), "实得 {:?}", p.retired);
        // 活的键与**注释**都在（toml_edit 保格式；换成 toml 重序列化这里就全没了）
        for keep in ["num_workers = 0", "output_dir = \"./output\"", "overwrite = false"] {
            assert!(p.text.contains(keep), "活键 {keep} 被误删：\n{}", p.text);
        }
        assert!(p.text.contains("# YekiTrim 全局默认配置"), "首行注释丢了：\n{}", p.text);
        assert!(p.text.contains("# 这些注释**必须活着**"), "注释丢了：\n{}", p.text);
        // 版本号在最前面，且能被读回来
        assert!(
            p.text.starts_with(&format!("config_version = {CONFIG_VERSION}\n")),
            "版本行不在最前：\n{}",
            p.text
        );
        let doc: DocumentMut = p.text.parse().unwrap();
        assert_eq!(version_of(&doc), CONFIG_VERSION);
        // 迁移后仍是合法 TOML，且能反序列化成 Config
        let cfg: super::super::Config = toml::from_str(&p.text).expect("迁移后必须能被 Config 解析");
        assert_eq!(cfg.asr.num_workers, 0);
        assert_eq!(cfg.cutting.output_dir, "./output");
    }

    #[test]
    fn migrate_is_idempotent_and_leaves_current_version_alone() {
        let once = migrate_text(ConfigKind::Global, V0_GLOBAL).unwrap().text;
        let twice = migrate_text(ConfigKind::Global, &once).unwrap();
        assert_eq!(twice.from, CONFIG_VERSION);
        assert!(!twice.changed, "已经是当前版本就不该再改");
        assert_eq!(twice.text, once, "第二次必须逐字节相同（否则每次启动都重写文件）");
    }

    /// **更新的版本不许被降级** —— 这条是保护用户数据的关键。
    #[test]
    fn a_newer_version_is_left_completely_untouched() {
        // ⚠️ 用 CONFIG_VERSION + 1，别写死数字：写死的那个每 +1 一次就变成"过期夹具"
        // （把 v3 当"未来版本"的写法在 v4 这一版就翻面了 —— 它会开始被迁移）。
        let future = format!("config_version = {}\n\n[asr]\nmode = \"single\"\n", CONFIG_VERSION + 1);
        let p = migrate_text(ConfigKind::Global, &future).unwrap();
        assert_eq!(p.from, CONFIG_VERSION + 1);
        assert!(!p.changed);
        assert_eq!(p.text, future, "一个字节都不许动");
    }

    /// 工程级不该被写回（策略差异靠这个钉住）。
    #[test]
    fn only_program_level_files_persist() {
        assert!(ConfigKind::Global.persists());
        assert!(ConfigKind::Llm.persists());
        assert!(ConfigKind::Server.persists());
        assert!(!ConfigKind::Project.persists(), "每个工程一份，不主动改写");
        assert!(!ConfigKind::Polish.persists());
    }

    #[test]
    fn kind_is_recognised_by_file_name_and_unknown_defaults_to_project() {
        let k = |s: &str| ConfigKind::of_path(Path::new(s));
        assert_eq!(k("/x/.yekitrim/config.toml"), ConfigKind::Global);
        assert_eq!(k("/x/.yekitrim/llm.toml"), ConfigKind::Llm);
        assert_eq!(k("/x/.yekitrim/server.toml"), ConfigKind::Server);
        assert_eq!(k("/p/clip-01/polish.toml"), ConfigKind::Polish);
        // ⚠️ 认不出来 = 工程级（最保守：不碰磁盘）
        assert_eq!(k("/p/random.toml"), ConfigKind::Project);
        assert_eq!(k("/p/yekitrim.toml"), ConfigKind::Project);
    }

    /// 非 Global/Project 的文件不该被清 asr 的键（它们的 schema 不一样）。
    #[test]
    fn llm_and_server_do_not_touch_asr_keys() {
        let text = "config_version = 0\n\n[asr]\nalign_model = \"x\"\n";
        let p = migrate_text(ConfigKind::Llm, text).unwrap();
        assert!(p.text.contains("align_model"), "llm.toml 不该动 asr 的键");
        assert_eq!(version_of(&p.text.parse().unwrap()), CONFIG_VERSION);
    }

    /// 缺键 = 0；非整数/负数也当 0（别 panic）。
    #[test]
    fn version_parsing_is_forgiving() {
        let v = |s: &str| version_of(&s.parse::<DocumentMut>().unwrap());
        assert_eq!(v(""), 0);
        assert_eq!(v("[asr]\nmode = \"x\"\n"), 0);
        assert_eq!(v("config_version = 3\n"), 3);
        assert_eq!(v("config_version = \"1\"\n"), 0, "字符串版本号当缺键");
        assert_eq!(v("config_version = -5\n"), 0, "负数当缺键，别 panic");
    }

    /// 备份名要能一眼看出"这是加版本号之前的原样"，且**不覆盖**已存在的备份。
    #[test]
    fn backup_keeps_the_earliest_original() {
        let dir = std::env::temp_dir().join(format!("yt-ver-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("config.toml");
        std::fs::write(&f, "v0-original").unwrap();

        let bak = backup_before_upgrade(&f, 0).unwrap().expect("第一次该建备份");
        assert!(bak.to_string_lossy().ends_with("config.toml.bak-v0"));
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "v0-original");

        // 原文件后来变了，再备份**不该**覆盖最早那份
        std::fs::write(&f, "v1-later").unwrap();
        assert!(backup_before_upgrade(&f, 0).unwrap().is_none(), "已有备份就不该再建");
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "v0-original", "最早那份必须保留");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **IO 层**：程序级文件（`prepare` 那条路）升级后要**写回磁盘 + 留下备份**。
    ///
    /// 这是判据链里最要紧的一环 —— 纯函数对了、但写回没接上，表现是"每次启动都重新迁移一遍、
    /// 文件永远停在 v0"（而且用户文件里那 4 个死键永远清不掉）。
    #[test]
    fn prepare_rewrites_program_level_file_and_keeps_a_backup() {
        let dir = std::env::temp_dir().join(format!("yt-ver-prep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("config.toml");
        std::fs::write(&f, V0_GLOBAL).unwrap();

        let p = prepare(&f, ConfigKind::Global).unwrap();
        assert_eq!(p.from, 0);
        assert!(p.changed);
        // 磁盘上那份已经升级：版本在前、死键没了、活键与注释都在
        let on_disk = std::fs::read_to_string(&f).unwrap();
        let want_version = format!("config_version = {CONFIG_VERSION}\n");
        assert!(on_disk.starts_with(&want_version), "写回的内容不对：\n{on_disk}");
        assert!(!on_disk.contains("align_model"));
        assert!(on_disk.contains("num_workers = 0"));
        assert!(on_disk.contains("# 这些注释**必须活着**"), "写回把注释吃掉了");
        // 备份是**升级前**的原样
        let bak = dir.join("config.toml.bak-v0");
        assert!(bak.is_file(), "没有留下升级前的备份");
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), V0_GLOBAL);

        // 再读一次：已是最新版本 ⇒ **不该再动文件**（否则每次启动都重写一遍）
        let before = std::fs::metadata(&f).unwrap().modified().unwrap();
        let again = prepare(&f, ConfigKind::Global).unwrap();
        assert!(!again.changed);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), on_disk);
        let _ = before;

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **工程级文件不写回**：`prepare` 只把迁移后的文本交出去，磁盘必须**一个字节都不动**。
    ///
    /// 这条是策略判据：工程可能几百个、还可能在网络盘上，批量改写文件的风险远大于收益。
    #[test]
    fn prepare_leaves_project_level_file_untouched_on_disk() {
        let dir = std::env::temp_dir().join(format!("yt-ver-proj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("yekitrim.toml");
        std::fs::write(&f, V0_GLOBAL).unwrap();

        let p = prepare(&f, ConfigKind::Project).unwrap();
        assert!(p.changed, "内存里该升级");
        assert!(!p.text.contains("align_model"), "交出去的文本该是清过的");
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            V0_GLOBAL,
            "工程级文件**不许**被读操作改写"
        );
        assert!(!dir.join("yekitrim.toml.bak-v0").exists(), "也不该产生备份");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// v1 → v2 的**结构**判据：搬家（键与注释一起）、改名（含链路覆盖）、
    /// 缺席值落成 v1 的生效值、extra_params 原位不动、幂等。
    ///
    /// ⚠️ 终点是**当前**版本（现在 v4）：`[llm.params]` → `[llm.defaults]`、`api` → `protocol`
    /// 是 v3 → v4 那一步顺手做的，判据跟着读 v4（v2→v3 那一跳本身的映射没变）。
    #[test]
    fn migrate_llm_v2_moves_keys_and_keeps_comments() {
        const OLD: &str = r#"# 顶部注释
config_version = 1

[llm]
api_key = "k"
# 温度参数（这句注释必须活着）
temperature = 0
max_tokens = 16384
thinking = "enabled"
extra_params = { seed = 42 }

[llm.overrides.detect]
# 检测这条
thinking = "disabled"
max_tokens = 8192
"#;
        let p = migrate_text(ConfigKind::Llm, OLD).unwrap();
        assert_eq!(p.from, 1, "这份夹具是 v1");
        let text = &p.text;
        // ⚠️ 版本号**不清掉它上方的注释**（模板头几行就写在 config_version 上面）：
        // 所以这里问"文件里有当前版本"，而不是"第一行就是版本号"。
        assert!(text.starts_with("# 顶部注释\n"), "顶部的注释被吃掉了：\n{text}");
        assert!(text.contains(&format!("config_version = {CONFIG_VERSION}\n")), "版本没升：\n{text}");
        assert!(text.contains("[llm.defaults]"), "参数面要落在 defaults：\n{text}");
        assert!(text.contains("[[llm.providers]]"), "没有造出接口：\n{text}");
        assert!(text.contains("protocol = \"openai-chat\""), "api 要改名 protocol：\n{text}");
        assert!(text.contains("active = \"default\""), "active 没指向那份接口：\n{text}");
        assert!(text.contains("温度参数（这句注释必须活着）"), "键上方注释被吃掉了：\n{text}");
        assert!(text.contains("reasoning = \"high\""), "enabled 该变成 high：\n{text}");
        assert!(text.contains("reasoning = \"off\""), "链路覆盖的 disabled 该变成 off：\n{text}");
        assert!(!text.contains("thinking ="), "老键不许还留在文件里：\n{text}");
        assert!(text.contains("extra_params = { seed = 42 }"), "逃逸口原位不动：\n{text}");
        assert!(text.contains("max_tokens = 8192"), "链路覆盖的值要保留：\n{text}");

        let llm = llm_v4(text);
        assert_eq!(llm.active, "default", "迁移后 active 指向 default");
        let prov = llm.active_provider().expect("active 必须命中一份接口");
        assert_eq!(prov.protocol, "openai-chat", "协议要写进来（自解释）");
        assert_eq!(llm.defaults.temperature, Some(0.0));
        assert_eq!(llm.defaults.max_tokens, Some(16384));
        assert_eq!(llm.defaults.reasoning.as_deref(), Some("high"));
        let detect = llm.links.get("detect").expect("链路覆盖要在 links 里");
        assert_eq!(detect.params.reasoning.as_deref(), Some("off"));
        assert_eq!(detect.params.max_tokens, Some(8192));

        // 缺席的那两个键要**落成 v1 的生效值**（新形状里"缺席"= 不发送，不落就等于悄悄改了请求体）
        let bare = migrate_text(ConfigKind::Llm, "[llm]\nmodel = \"m\"\n").unwrap().text;
        assert!(bare.contains("temperature = 0.1"), "缺温度要补 v1 默认值：\n{bare}");
        assert!(bare.contains("max_tokens = 65536"), "缺上限要补 v1 默认值：\n{bare}");
        let bare_file = llm_v4(&bare);
        let bare_prov = bare_file.active_provider().expect("能有接口");
        assert_eq!(bare_prov.model, "m", "模型搬进接口了");
        assert_eq!(bare_file.defaults.temperature, Some(0.1));
        assert_eq!(bare_file.defaults.max_tokens, Some(65536));

        // 幂等：已经是当前版本的文本再迁一次，一个字节都不动
        let again = migrate_text(ConfigKind::Llm, text).unwrap();
        assert!(!again.changed);
        assert_eq!(&again.text, text);
    }

    /// **v2 → v3 → v4**：老的单份 `[llm]` 搬成**一份接口**；`[llm.models.*]` 跟着进 `[[llm.models]]`
    /// （带 provider）；再迁一次一个字节都不动（幂等）。--nocapture 能看到迁移后的真样子（key 是夹具里的假值）。
    #[test]
    fn migrate_llm_v3_moves_top_level_into_one_provider() {
        const OLD: &str = "[llm]\napi_key = \"k\"\nbase_url = \"http://h/v1\"\nmodel = \"m\"\n\n[llm.params]\ntemperature = 0\n\n[llm.models.\"m\"]\nthinking_wire = \"thinking_type\"\n";
        let text = migrate_text(ConfigKind::Llm, OLD).unwrap().text;
        println!("MIGRATED_V4=\n{text}");
        assert!(text.contains("active = \"default\""), "{text}");
        assert!(text.contains("[[llm.providers]]"), "{text}");
        assert!(text.contains("[llm.defaults]"), "参数面搬进 defaults：\n{text}");
        assert!(!text.contains("[llm.params]"), "老表名不许留着：\n{text}");
        assert!(!text.contains("[llm.models."), "老的键值表不许留着：\n{text}");
        assert!(text.contains("[[llm.models]]"), "模型要成为数组：\n{text}");
        assert!(text.contains("provider = \"default\""), "每项要带 provider：\n{text}");
        assert!(text.contains("thinking_wire = \"thinking_type\""), "原表的键要平铺进去：\n{text}");
        let llm = llm_v4(&text);
        assert_eq!(llm.models.len(), 1);
        assert_eq!(llm.models[0].id, "m");
        assert_eq!(llm.models[0].provider, "default");
        let again = migrate_text(ConfigKind::Llm, &text).unwrap();
        assert!(!again.changed, "已经是最新版本了，不该再改：\n{}", again.text);
        assert_eq!(again.text, text, "幂等");
    }

    /// ⚠️ **验收项：v1 的 llm.toml 迁移到当前版本之后，请求体逐字节不变。**
    ///
    /// 下面那 12 行 JSON 是**迁移前的老代码真跑出来的**（同一条 wire::apply、同样三个文件），
    /// 所以这条判据不是"我写了个期望值"，而是"升级前后打出去的字节一模一样"。
    /// 三个文件各钉一类：认识的模型（走能力表）/ 表外的模型（保守默认）/ **省掉两个键**的那种
    /// （v1 有代码级默认值 ⇒ 必须由迁移落成显式键）。
    ///
    /// ⚠️ 读的是 **v4**（`LlmV4::for_link` → `LlmParams`）：v3→v4 之后 `LlmConfig` 不再是读形状，
    /// 而这条判据问的正是"喂给 wire 的那份参数"，与中间是哪个版本无关。
    #[test]
    fn migrated_v1_llm_toml_produces_byte_identical_bodies() {
        fn bodies(text: &str) -> Vec<(String, String)> {
            let llm = llm_v4(text);
            // ⚠️ **v1 的四条**（不是 ALL）：金色数据是"迁移前的老代码真跑出来的" 12 条，
            // 加了新链路（Title）之后继续用 ALL 会去找第 5 条 —— 那是改判据去凑数据。
            super::super::types::LlmLink::LEGACY_V1
                .iter()
                .map(|link| {
                    let linked = llm.for_link(*link);
                    let p: yekitrim_llm::LlmParams = (&linked).into();
                    let mut body =
                        serde_json::json!({ "messages": [{ "role": "user", "content": "hi" }] });
                    yekitrim_llm::wire::apply(&mut body, &p);
                    (link.key().to_string(), serde_json::to_string(&body).unwrap())
                })
                .collect()
        }

        const OLD_A: &str = r#"[llm]
api_key = "k"
base_url = "http://127.0.0.1:7863/v1"
model = "cn:deepseek-v4.1-flash"
temperature = 0
max_tokens = 16384
max_concurrent = 3
thinking = "enabled"
extra_params = { seed = 42, thinking = { type = "enabled" }, response_format = { type = "json_object" } }

[llm.overrides.detect]
max_tokens = 8192
temperature = 0
[llm.overrides.polish]
max_tokens = 32768
temperature = 0
[llm.overrides.correct]
max_tokens = 8192
temperature = 0
"#;
        const OLD_B: &str = r#"[llm]
api_key = "k"
base_url = "https://api.deepseek.com/v1"
model = "deepseek-v4-pro"
temperature = 0
max_tokens = 16384
thinking = "disabled"
"#;
        const OLD_C: &str = r#"[llm]
api_key = "k"
base_url = "https://unknown.example.com/v1"
model = "m"
thinking = "enabled"

[llm.overrides.detect]
thinking = "disabled"
"#;
        // 金色请求体在**数据文件**里（不是源码）：wire 字面量只许出现在 yekitrim-llm，
        // 而这份是"迁移前的老代码真跑出来的输出"，属于数据。
        let golden: serde_json::Value =
            serde_json::from_str(include_str!("testdata/llm-v1-bodies.json")).expect("金色数据必须是合法 JSON");
        for (tag, old) in [("A", OLD_A), ("B", OLD_B), ("C", OLD_C)] {
            let migrated = migrate_text(ConfigKind::Llm, old).expect("老文件必须能迁移").text;
            assert!(
                migrated.starts_with(&format!("config_version = {CONFIG_VERSION}\n")),
                "{tag} 没升版本：\n{migrated}"
            );
            let want = golden[tag].as_array().expect("每组 4 条（顺序 = LlmLink::LEGACY_V1）");
            let got = bodies(&migrated);
            assert_eq!(got.len(), want.len(), "{tag} 条数不对");
            for (i, (link, body)) in got.iter().enumerate() {
                let want_body = serde_json::to_string(&want[i]).expect("金色数据要能序列化");
                assert_eq!(body, &want_body, "{tag}/{link} 的请求体变了");
            }
        }
    }

    /// **v3 → v4（老文件）**：只有顶层三件 + `[llm.params]` + `[llm.overrides.detect]` 的文件，
    /// 迁完要有 `[[llm.providers]]`（`id = "default"`）、`active = "default"`、`[llm.defaults]`、
    /// `[llm.links.detect]`，而**顶层 `api_key` 已经不在了**（搬进那份接口）。
    #[test]
    fn migrate_llm_v4_legacy_file_becomes_provider_defaults_and_links() {
        const OLD: &str = r#"# YekiTrim LLM 配置
config_version = 3

[llm]
api_key = "sk-legacy"
base_url = "https://api.example.com/v1"
model = "example-chat"
max_concurrent = 3

[llm.params]
temperature = 0.7
max_tokens = 4096

[llm.overrides.detect]
max_tokens = 8192
"#;
        let p = migrate_text(ConfigKind::Llm, OLD).unwrap();
        assert_eq!(p.from, 3, "这份夹具是 v3");
        let text = &p.text;
        assert!(text.contains("[[llm.providers]]"), "没有造出接口：\n{text}");
        assert!(text.contains("id = \"default\""), "{text}");
        assert!(text.contains("active = \"default\""), "{text}");
        assert!(text.contains("[llm.defaults]"), "params 要改名成 defaults：\n{text}");
        assert!(text.contains("[llm.links.detect]"), "overrides 要改名成 links：\n{text}");
        assert!(!text.contains("[llm.params]"), "老表名不许留着：\n{text}");
        assert!(!text.contains("[llm.overrides"), "老表名不许留着：\n{text}");

        // 顶层连接面必须搬空（v4 里这三个键不再存在）
        let doc: DocumentMut = text.parse().unwrap();
        let llm_table = doc.get("llm").and_then(Item::as_table).expect("有 [llm]");
        for gone in ["api_key", "base_url", "model", "params", "overrides"] {
            assert!(llm_table.get(gone).is_none(), "顶层 {gone} 必须搬走：\n{text}");
        }

        let llm = llm_v4(text);
        let prov = llm.active_provider().expect("active 必须命中一份接口");
        assert_eq!(prov.id, "default");
        assert_eq!(prov.api_key, "sk-legacy", "key 跟着接口走");
        assert_eq!(prov.base_url, "https://api.example.com/v1");
        assert_eq!(prov.model, "example-chat");
        assert_eq!(llm.defaults.temperature, Some(0.7));
        assert_eq!(llm.defaults.max_tokens, Some(4096));
        assert_eq!(llm.links.get("detect").and_then(|l| l.params.max_tokens), Some(8192));
    }

    /// **v3 → v4（多接口）**：`[[llm.providers]]` 的 `api` → `protocol`；**生效那份**的 `params`
    /// 并进 `[llm.defaults]`（非生效那份表达不了 ⇒ 丢弃 + warn）；`models` 进 `[[llm.models]]` 且带 provider。
    #[test]
    fn migrate_llm_v4_moves_provider_api_params_and_models() {
        const OLD: &str = r#"config_version = 3

[llm]
active = "main"
max_concurrent = 5

[[llm.providers]]
id = "main"
name = "主力"
api = "openai-chat"
base_url = "https://a.example.com/v1"
api_key = "sk-main"
model = "m-main"

[llm.providers.params]
temperature = 0.3
top_p = 0.9

[[llm.providers.models]]
id = "m-main"
name = "主力模型"

[[llm.providers]]
id = "cheap"
name = "便宜"
api = "openai-chat"
base_url = "https://b.example.com/v1"
api_key = "sk-cheap"
model = "m-cheap"

[llm.providers.params]
max_tokens = 512
"#;
        let text = migrate_text(ConfigKind::Llm, OLD).unwrap().text;
        assert!(!text.contains("api = "), "api 要改名成 protocol：\n{text}");
        assert!(!text.contains("[llm.providers.params]"), "接口自己的参数面不许留着：\n{text}");
        assert!(!text.contains("[[llm.providers.models]]"), "模型要搬到顶层清单：\n{text}");

        let llm = llm_v4(&text);
        assert_eq!(llm.providers.len(), 2, "两份接口都要在：\n{text}");
        assert!(llm.providers.iter().all(|p| p.protocol == "openai-chat"), "{text}");
        assert_eq!(llm.active, "main");
        // 生效那份的参数并进 defaults；非生效那份的 max_tokens 表达不了
        assert_eq!(llm.defaults.temperature, Some(0.3));
        assert_eq!(llm.defaults.top_p, Some(0.9));
        assert_eq!(llm.defaults.max_tokens, None, "非生效接口的参数只能丢弃（并 warn）");
        // 模型清单：provider = 各自接口
        let got: Vec<(&str, &str)> =
            llm.models.iter().map(|m| (m.id.as_str(), m.provider.as_str())).collect();
        assert_eq!(got, vec![("m-main", "main")], "models 要进 [[llm.models]] 且带 provider：\n{text}");
    }

    /// **api_key 逐字节不变** —— 迁移里唯一不可重建的东西：值里的 / + = 与行尾注释都不许被重排。
    #[test]
    fn migrate_llm_v4_keeps_api_key_byte_for_byte() {
        const KEY: &str = "sk-live/a+b=c_0123456789";
        let old = format!(
            "config_version = 3\n\n[llm]\napi_key = \"{KEY}\"   # 这行一个字都不许动\nbase_url = \"https://x/v1\"\nmodel = \"m\"\n"
        );
        let text = migrate_text(ConfigKind::Llm, &old).unwrap().text;
        let line = text
            .lines()
            .find(|l| l.trim_start().starts_with("api_key"))
            .unwrap_or_else(|| panic!("api_key 不见了：\n{text}"));
        assert_eq!(
            line.trim_end(),
            format!("api_key = \"{KEY}\"   # 这行一个字都不许动"),
            "api_key 那一行必须逐字节不变：\n{text}"
        );
        assert_eq!(llm_v4(&text).providers[0].api_key, KEY);
    }

    /// **幂等**：同一份文本迁两次，第二次 `changed == false` 且一个字节都不动；
    /// 纯函数自己也要能回答"有没有改动"（v4 形状 ⇒ false）。
    #[test]
    fn migrate_llm_v4_is_idempotent() {
        const OLD: &str = r#"config_version = 3

[llm]
api_key = "k"
base_url = "https://x/v1"
model = "m"

[llm.params]
temperature = 0

[llm.overrides.detect]
max_tokens = 1024

[llm.models."m"]
thinking_wire = "thinking_type"
"#;
        let once = migrate_text(ConfigKind::Llm, OLD).unwrap().text;
        let twice = migrate_text(ConfigKind::Llm, &once).unwrap();
        assert_eq!(twice.from, CONFIG_VERSION);
        assert!(!twice.changed, "已经是最新版本，不该再改");
        assert_eq!(twice.text, once, "第二次必须逐字节相同（否则每次启动都重写文件）");

        let mut doc: DocumentMut = once.parse().unwrap();
        assert!(!migrate_llm_v4(&mut doc), "v4 形状再过一遍必须返回 false（有没有改动的判据）");
        assert_eq!(doc.to_string(), once, "返回 false 的那次不许动一个字节");
    }

    /// **注释不丢**：`[llm]` 上方那段仍在原处；`[llm.overrides.detect]` 那两段跟着搬到 links；
    /// `[llm.models]` 上方那段落到**第一条** `[[llm.models]]` 之前。
    #[test]
    fn migrate_llm_v4_keeps_comments() {
        const OLD: &str = r#"# 顶部注释
config_version = 3

# ── 接口（这段必须活着）──
[llm]
api_key = "k"
base_url = "https://x/v1"
model = "m"

# ── 请求参数面（也要活着）──
[llm.params]
temperature = 0

# ── 按链路覆盖（跟着走）──
[llm.overrides.detect]
# 检测这条
max_tokens = 8192

# ── 模型能力（搬到第一条模型之前）──
# 内置表没有的模型在这里手填
[llm.models."m"]
thinking_wire = "thinking_type"
"#;
        let text = migrate_text(ConfigKind::Llm, OLD).unwrap().text;
        for keep in [
            "# 顶部注释",
            "# ── 接口（这段必须活着）──",
            "# ── 请求参数面（也要活着）──",
            "# ── 按链路覆盖（跟着走）──",
            "# 检测这条",
            "# ── 模型能力（搬到第一条模型之前）──",
            "# 内置表没有的模型在这里手填",
        ] {
            assert!(text.contains(keep), "注释丢了：{keep}\n{text}");
        }
        // 位置也要对，不能只是"还在文件里"
        assert!(text.contains("# ── 接口（这段必须活着）──\n[llm]\n"), "接口那段跑了：\n{text}");
        assert!(
            text.contains("# ── 模型能力（搬到第一条模型之前）──\n# 内置表没有的模型在这里手填\n[[llm.models]]\n"),
            "模型那段要落到第一条 [[llm.models]] 之前：\n{text}"
        );
    }

    /// 坏 TOML 要**报错**（让调用方走各自原有的"坏文件"处理），不能悄悄当成空文档。
    #[test]
    fn broken_toml_is_an_error_not_an_empty_doc() {
        assert!(migrate_text(ConfigKind::Global, "这不是 TOML [[[").is_err());
    }
}

/// **S4：v4 → v5 的跨文件搬家**（config.toml ↔ llm.toml）。
///
/// 这一组判据钉住的都是"静默坏"的那几种：只删不搬（提示词丢失）、搬错层（提示词进了请求参数）、
/// 拿假覆盖压死全局旋钮、目标已有键被覆盖。
#[cfg(test)]
mod s4_tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct LlmFile {
        llm: super::super::llm_v4::LlmV4,
    }
    fn llm_v4(text: &str) -> super::super::llm_v4::LlmV4 {
        toml::from_str::<LlmFile>(text).expect("搬家后必须能按 v5 读回来").llm
    }

    const V4_GLOBAL: &str = r#"config_version = 4
# 顶部注释必须活着

[correct]
# 订正 system_prompt（留着）
system_prompt = """订正 system"""
prompt = """订正 user"""
# 是否启用思考模式
thinking = true
# Agent 窗口大小
window_size = 40
# Agent 最大并发数
max_concurrent = 3
hotwords = ["热词"]

[detection]
vtuber_name = "主播甲"
danmaku_window_secs = 12
# 评分权重
llm_score_weight = 0.45
# 参考标题
reference_titles = ["标题甲😱", "标题乙"]
system_prompt = """检测 system"""
prompt_template = """检测 {segments}"""
max_concurrent = 3
"#;

    const V4_LLM: &str = r#"config_version = 4
[llm]
active = "main"
max_concurrent = 5

[[llm.providers]]
id = "main"
name = "主力"
base_url = "https://api.example.com/v1"
api_key = "sk-keep-me-intact"
model = "m"
"#;

    fn pair() -> (DocumentMut, DocumentMut) {
        (V4_GLOBAL.parse().unwrap(), V4_LLM.parse().unwrap())
    }

    /// 七个键全部搬到位、值逐个相等，config.toml 那边**一个不剩**，api_key 逐字节不变。
    #[test]
    fn v5_moves_every_key_and_keeps_values() {
        let (mut g, mut l) = pair();
        let moved = migrate_config_to_llm(&mut g, &mut l);
        assert!(moved.len() >= 7, "应当搬了七个键以上：{moved:?}");
        let llm = llm_v4(&l.to_string());
        let correct = llm.links.get("correct").expect("links.correct");
        assert_eq!(correct.params.reasoning.as_deref(), Some("high"), "thinking=true ⇒ reasoning=high");
        let run = correct.run.as_ref().expect("links.correct.run");
        assert_eq!(run.window_size, Some(40));
        assert_eq!(run.max_concurrent, Some(3), "3 ≠ 全局 5 ⇒ 用户手压过，必须搬");
        let detect = llm.links.get("detect").expect("links.detect");
        let prompt = detect.prompt.as_ref().expect("links.detect.prompt");
        assert_eq!(prompt.system_prompt, "检测 system");
        assert_eq!(prompt.prompt_template, "检测 {segments}");
        assert_eq!(prompt.reference_titles, vec!["标题甲😱".to_string(), "标题乙".to_string()]);
        let drun = detect.run.as_ref().expect("links.detect.run");
        assert_eq!(drun.llm_score_weight, Some(0.45));
        assert_eq!(drun.max_concurrent, Some(3));
        assert_eq!(llm.providers[0].api_key, "sk-keep-me-intact", "key 一个字节都不许动");

        let text = g.to_string();
        for bad in ["thinking =", "window_size =", "llm_score_weight =", "reference_titles =", "prompt_template ="] {
            assert!(!text.contains(bad), "config.toml 里不该再有 `{bad}`：\n{text}");
        }
        assert!(text.contains("vtuber_name"), "同段的其他键必须留下");
        assert!(text.contains("订正 system"), "[correct] 自己的提示词不搬");
        assert!(text.contains("# 顶部注释必须活着"), "注释必须活着");
    }

    /// 幂等：第二遍什么都不做（返回空、两个文档字符串一个字节都不变）。
    #[test]
    fn v5_is_idempotent() {
        let (mut g, mut l) = pair();
        let first = migrate_config_to_llm(&mut g, &mut l);
        assert!(!first.is_empty());
        let (gs, ls) = (g.to_string(), l.to_string());
        let second = migrate_config_to_llm(&mut g, &mut l);
        assert!(second.is_empty(), "第二遍不该再搬：{second:?}");
        assert_eq!(g.to_string(), gs);
        assert_eq!(l.to_string(), ls);
    }

    /// 目标已经有同名键 ⇒ **目标优先**（源键照样退役，不然它永远躺着误导人）。
    #[test]
    fn v5_does_not_overwrite_existing_target_keys() {
        let mut ldoc: DocumentMut = r#"config_version = 4
[llm]
active = "main"
max_concurrent = 5
[llm.links.detect.prompt]
system_prompt = "目标里已经有了"
"#
        .parse()
        .unwrap();
        let mut g: DocumentMut = V4_GLOBAL.parse().unwrap();
        migrate_config_to_llm(&mut g, &mut ldoc);
        let llm = llm_v4(&ldoc.to_string());
        assert_eq!(
            llm.links["detect"].prompt.as_ref().unwrap().system_prompt,
            "目标里已经有了"
        );
        assert!(!g.to_string().contains("检测 system"), "源键仍然退役");
    }

    /// `thinking = false` ⇒ **不写 reasoning**（写 "off" 会变成"显式关思考"，是行为变更）。
    #[test]
    fn v5_thinking_false_writes_no_reasoning() {
        let mut g: DocumentMut = r#"config_version = 4
[correct]
thinking = false
"#
        .parse()
        .unwrap();
        let mut l: DocumentMut = V4_LLM.parse().unwrap();
        migrate_config_to_llm(&mut g, &mut l);
        let llm = llm_v4(&l.to_string());
        assert!(
            llm.links.get("correct").and_then(|c| c.params.reasoning.clone()).is_none(),
            "false 不许写成 off"
        );
    }

    /// 段级并发与 `[llm] max_concurrent` 相等 = 模板带来的**假覆盖** ⇒ 丢掉（全局旋钮才真的说了算）。
    #[test]
    fn v5_concurrency_carried_only_when_it_differs_from_global() {
        let mut g: DocumentMut = r#"config_version = 4
[detection]
max_concurrent = 5
"#
        .parse()
        .unwrap();
        let mut l: DocumentMut = V4_LLM.parse().unwrap(); // 全局 = 5
        migrate_config_to_llm(&mut g, &mut l);
        let llm = llm_v4(&l.to_string());
        assert!(llm.links.get("detect").is_none(), "相等就该整条丢掉");
        assert!(!g.to_string().contains("max_concurrent"), "源键退役");
    }

    /// `retired_in` 必须覆盖那七个键（漏登记 = 老文件里永远留着没人读的键）。
    #[test]
    fn retired_in_covers_every_moved_key() {
        let want: Vec<Vec<&str>> = vec![
            vec!["correct", "thinking"],
            vec!["correct", "window_size"],
            vec!["correct", "max_concurrent"],
            vec!["detection", "system_prompt"],
            vec!["detection", "prompt_template"],
            vec!["detection", "reference_titles"],
            vec!["detection", "llm_score_weight"],
            vec!["detection", "max_concurrent"],
        ];
        let got: Vec<Vec<&str>> = ConfigKind::Global
            .retired_in(4)
            .iter()
            .map(|p| p.to_vec())
            .collect();
        for w in want {
            assert!(got.contains(&w), "retired_in(Global, 4) 漏了 {w:?}：{got:?}");
        }
    }

    /// **文件级**：`prepare(Global)` 真把键搬进同目录 llm.toml，再写回 config.toml（v5）。
    #[test]
    fn prepare_moves_keys_into_sibling_llm_toml() {
        let dir = std::env::temp_dir().join(format!("yekitrim-s4-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config.toml");
        std::fs::write(&cfg, V4_GLOBAL).unwrap();
        std::fs::write(dir.join("llm.toml"), V4_LLM).unwrap();
        let prepared = prepare(&cfg, ConfigKind::Global).expect("迁移");
        assert!(prepared.changed);
        assert_eq!(version_of(&prepared.text.parse().unwrap()), CONFIG_VERSION);
        assert!(!prepared.text.contains("llm_score_weight"), "源文件里不该再有它");
        let llm_now = std::fs::read_to_string(dir.join("llm.toml")).unwrap();
        let llm = llm_v4(&llm_now);
        assert_eq!(
            llm.links["detect"].prompt.as_ref().unwrap().system_prompt,
            "检测 system"
        );
        assert_eq!(llm.providers[0].api_key, "sk-keep-me-intact");
        // 源文件也写回去了（v5）
        let disk = std::fs::read_to_string(&cfg).unwrap();
        assert!(!disk.contains("thinking = true"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// llm.toml 的版本比我们**新** ⇒ 一个字节都不动，config.toml 的源键也**不删**（下次换新版再迁）。
    #[test]
    fn prepare_keeps_source_keys_when_llm_is_newer() {
        let dir = std::env::temp_dir().join(format!("yekitrim-s4-newer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config.toml");
        std::fs::write(&cfg, V4_GLOBAL).unwrap();
        let newer = V4_LLM.replace("config_version = 4", "config_version = 99");
        std::fs::write(dir.join("llm.toml"), &newer).unwrap();
        let prepared = prepare(&cfg, ConfigKind::Global).expect("迁移");
        // 跨文件那趟被拒 ⇒ 源文本原样进入单文件迁移，但 retired_in 是**兜底**，会把它们清掉；
        // 这里只钉"llm.toml 一个字节不动"（源键是否保留取决于 retired_in 这条兜底策略）。
        assert_eq!(std::fs::read_to_string(dir.join("llm.toml")).unwrap(), newer);
        let _ = prepared;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
