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
/// v2（2026-10-19）：`llm.toml` 把「请求参数面」与「能力」分开 ——
/// `[llm] temperature/max_tokens/thinking` → `[llm.params]`（`thinking` 改名 `reasoning`），
/// 能力覆盖进 `[llm.models."<id>"]`。**搬家**那一步在 [`migrate_llm_v2`]（`retired_in` 只会删）。
pub const CONFIG_VERSION: u32 = 2;

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
    doc.insert(VERSION_KEY, toml_edit::value(CONFIG_VERSION as i64));

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
    doc.insert(VERSION_KEY, toml_edit::value(CONFIG_VERSION as i64));
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

/// 读盘 + 迁移；**程序级文件在需要时备份并写回**（工程级只返回迁移后的文本）。
pub fn prepare(path: &Path, kind: ConfigKind) -> Result<Prepared> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
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
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("读不出 {}", path.display()))?;
    let kind = ConfigKind::of_path(path);
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
        let future = "config_version = 3\n\n[asr]\nmode = \"single\"\n";
        let p = migrate_text(ConfigKind::Global, future).unwrap();
        assert_eq!(p.from, 3);
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
        assert!(text.starts_with(&format!("config_version = {CONFIG_VERSION}\n")), "{text}");
        assert!(text.contains("[llm.params]"), "没有搬出参数表：\n{text}");
        assert!(text.contains("温度参数（这句注释必须活着）"), "键上方注释被吃掉了：\n{text}");
        assert!(text.contains("reasoning = \"high\""), "enabled 该变成 high：\n{text}");
        assert!(text.contains("reasoning = \"off\""), "链路覆盖的 disabled 该变成 off：\n{text}");
        assert!(!text.contains("thinking ="), "老键不许还留在文件里：\n{text}");
        assert!(text.contains("extra_params = { seed = 42 }"), "逃逸口原位不动：\n{text}");
        assert!(text.contains("max_tokens = 8192"), "链路覆盖的值要保留：\n{text}");

        #[derive(serde::Deserialize)]
        struct File {
            llm: super::super::types::LlmConfig,
        }
        let f: File = toml::from_str(text).expect("迁移后必须能解析成新结构");
        assert_eq!(f.llm.params.temperature, Some(0.0));
        assert_eq!(f.llm.params.max_tokens, Some(16384));
        assert_eq!(f.llm.params.reasoning.as_deref(), Some("high"));
        assert_eq!(f.llm.overrides.detect.reasoning.as_deref(), Some("off"));
        assert_eq!(f.llm.overrides.detect.max_tokens, Some(8192));

        // 缺席的那两个键要**落成 v1 的生效值**（新形状里"缺席"= 不发送，不落就等于悄悄改了请求体）
        let bare = migrate_text(ConfigKind::Llm, "[llm]\nmodel = \"m\"\n").unwrap().text;
        assert!(bare.contains("temperature = 0.1"), "缺温度要补 v1 默认值：\n{bare}");
        assert!(bare.contains("max_tokens = 65536"), "缺上限要补 v1 默认值：\n{bare}");

        // 幂等：已经是 v2 的文本再迁一次，一个字节都不动
        let again = migrate_text(ConfigKind::Llm, text).unwrap();
        assert!(!again.changed);
        assert_eq!(&again.text, text);
    }

    /// ⚠️ **验收项：v1 的 llm.toml 迁移到 v2 之后，请求体逐字节不变。**
    ///
    /// 下面那 12 行 JSON 是**迁移前的老代码真跑出来的**（同一条 wire::apply、同样三个文件），
    /// 所以这条判据不是"我写了个期望值"，而是"升级前后打出去的字节一模一样"。
    /// 三个文件各钉一类：认识的模型（走能力表）/ 表外的模型（保守默认）/ **省掉两个键**的那种
    /// （v1 有代码级默认值 ⇒ 必须由迁移落成显式键）。
    #[test]
    fn migrated_v1_llm_toml_produces_byte_identical_bodies() {
        #[derive(serde::Deserialize)]
        struct File {
            llm: super::super::types::LlmConfig,
        }
        fn bodies(text: &str) -> Vec<(String, String)> {
            let f: File = toml::from_str(text).expect("老形状必须能解析");
            super::super::types::LlmLink::ALL
                .iter()
                .map(|link| {
                    let linked = f.llm.for_link(*link);
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
base_url = "http://192.168.0.107:7863/v1"
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
            let want = golden[tag].as_array().expect("每组 4 条（顺序 = LlmLink::ALL）");
            let got = bodies(&migrated);
            assert_eq!(got.len(), want.len(), "{tag} 条数不对");
            for (i, (link, body)) in got.iter().enumerate() {
                let want_body = serde_json::to_string(&want[i]).expect("金色数据要能序列化");
                assert_eq!(body, &want_body, "{tag}/{link} 的请求体变了");
            }
        }
    }

    /// 坏 TOML 要**报错**（让调用方走各自原有的"坏文件"处理），不能悄悄当成空文档。
    #[test]
    fn broken_toml_is_an_error_not_an_empty_doc() {
        assert!(migrate_text(ConfigKind::Global, "这不是 TOML [[[").is_err());
    }
}
