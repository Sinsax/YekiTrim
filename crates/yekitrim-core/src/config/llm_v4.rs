//! llm.toml v4 形状与 v3 → v4 迁移（整改「LLM 配置重构」L1）。
//!
//! v3 的 LlmConfig 是两份真相并存（顶层 api_key/base_url/model + [llm.params] / [llm.models] 一套，
//! [[llm.providers]] + [llm.overrides] 另一套，靠 active 回落链切换）。v4 只有一份：
//!
//! ```text
//!     [llm]
//!     active = "main"        # 当前用哪一份接口（永远有值）
//!     max_concurrent = 3
//!     [llm.defaults]         # 请求参数的唯一基准（逐键）
//!     [[llm.providers]]      # 「怎么连」：id/name/api/base_url/api_key/api_key_env/model
//!     [[llm.models]]         # 「有哪些模型、属谁、能力如何」（唯一清单）
//!     [llm.links.detect]     # 「这条链路怎么发」：provider?/model?/参数覆盖
//! ```
//!
//! 本文件现在是 L1 的第一片：只放新形状与纯迁移函数（from_v3），不动运行时读取路径。
//! 等 LlmConfig 换成这套字段之后，from_v3 就是 version.rs 里 v3→v4 那一步的实现。
//!
//! ⚠️ 迁移的两条硬要求（都有用例钉住）：api_key 逐字节不变；已迁过的形状再迁一次不变（幂等）。
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::types::{LlmLink, LlmModel, LlmOverride, LlmParamsToml, ModelOverrideToml};

/// 模板里那个占位 key（等于没配）。
pub const PLACEHOLDER_API_KEY: &str = "sk-your-api-key-here";

/// v4 的链路清单（顺序 = UI 表格的行序；correct 隐藏但键仍在）。
pub const LINK_KEYS: [&str; 5] = ["detect", "polish", "correct", "title", "utility"];

/// 一份接口（[[llm.providers]]）：只描述「怎么连」。
///
/// ⚠️ 与 v3 的 LlmProvider 的区别：没有 params（决策 A：provider.params 是「整体遮蔽」的根源，砍掉），
/// 没有 models（模型清单在顶层 [[llm.models]]）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmProviderV4 {
    /// 稳定 ASCII id（引用/删除时认人用；大小写敏感）。
    #[serde(default)]
    pub id: String,
    /// 显示名（可中文）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// 协议 id。空 = 默认那条；写错由 problems 报。v3 里这个键叫 api，v4 改名 protocol
    /// （serde alias 让老键也读得动）。
    #[serde(default, alias = "api", skip_serializing_if = "String::is_empty")]
    pub protocol: String,
    #[serde(default)]
    pub base_url: String,
    /// 明文 key（永不出接口）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    /// 从环境变量取 key（api_key 为空时才用它）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key_env: String,
    /// 这份接口的默认模型（[llm.links.*].model 没写时用它）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
}

/// 模型清单里的一条（[[llm.models]]）：这份清单是「有哪些模型」的唯一真相。
///
/// 两个 Option 是迁移期的并集：v3 的能力覆盖有两套形状（provider.models 的 LlmModel 与
/// [llm.models."<id>"] 的 ModelOverrideToml），这里原样保留；等 S3 把合并层重写成
/// 「catalog 只做建议 + 清单说了算」时再摊平。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmModelEntry {
    /// 模型 id（发给 API 的那个；清单主键 (接口, id) 唯一）。
    #[serde(default)]
    pub id: String,
    /// 属于哪份接口（[[llm.providers]].id）。
    #[serde(default)]
    pub provider: String,
    /// 显示名（可空 ⇒ 用 id）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// 来自 [[llm.providers]].models（目录形状）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<LlmModel>,
    /// 来自 [llm.models."<id>"]（表外手填覆盖形状）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<ModelOverrideToml>,
}

impl LlmModelEntry {
    /// 显示的模型名：label 优先，其次 id。
    pub fn display_name(&self) -> &str {
        if self.label.trim().is_empty() {
            self.id.as_str()
        } else {
            self.label.as_str()
        }
    }
}

/// 一条链路的设置（[llm.links.<link>]）：接口 + 模型 + 参数覆盖。
///
/// provider / model 留空 = 跟随 [llm] active 与那份接口的默认模型 —— 这样「只有一条链路要换模型」时
/// 只写一行，其余四行可以完全不存在（文件里不出现即「跟随」）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmLinkSetting {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// 请求参数覆盖（逐键；None = 跟随 [llm.defaults]）。落在子键上，不写就不出现。
    #[serde(flatten, default)]
    pub params: LlmParamsToml,
    /// 这条链路的**提示词**（`[llm.links.<link>.prompt]`）—— **不是请求参数**，不进请求体。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<LlmPromptCfg>,
    /// 这条链路的**本地行为参数**（`[llm.links.<link>.run]`）—— 窗口 / 并发 / 评分权重，不进请求体。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<LlmRunCfg>,
}

impl LlmLinkSetting {
    /// 这一行是不是空的（既没选接口/模型，也没有任何参数/提示词/行为覆盖）—— 空行不写回文件。
    pub fn is_empty_setting(&self) -> bool {
        self.provider.trim().is_empty()
            && self.model.trim().is_empty()
            && self.params.is_empty()
            && self.prompt.as_ref().map(LlmPromptCfg::is_empty).unwrap_or(true)
            && self.run.is_none()
    }
}

/// 一条链路的**提示词**（`[llm.links.<link>.prompt]`）：喂给 LLM 的文本，**不是请求参数**。
///
/// ⚠️ **为什么单独一张子表**而不是平铺进 `[llm.links.<link>]`：那一层平铺的是「这一趟怎么发」
/// （temperature / max_tokens / reasoning …），它们会被 `LlmParamsToml::merged_over` 与
/// `From<&LinkedLlm> for LlmParams` 消费；提示词与 request body 无关，混进去会让
/// "请求参数"这个词同时指两件不相干的事（S4 之前它还有过把 `out.links.clear()` 连带清空的坑）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmPromptCfg {
    /// 检测 system_prompt（空 = 内建默认）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system_prompt: String,
    /// 检测 user prompt 模板（空 = 内建默认；`{segments}` / `{reference_titles}` 会被替换）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt_template: String,
    /// 参考标题（空 = 内建示例；也喂给封面标题生成）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_titles: Vec<String>,
}

impl LlmPromptCfg {
    /// 三个键都空 = 没配（`[llm.links.detect.prompt]` 只写了个空表也当没写）。
    pub fn is_empty(&self) -> bool {
        self.system_prompt.trim().is_empty()
            && self.prompt_template.trim().is_empty()
            && self.reference_titles.iter().all(|s| s.trim().is_empty())
    }
}

/// 一条链路的**本地行为参数**（`[llm.links.<link>.run]`）：不进请求体。
///
/// ⚠️ 这一层与 `params` 的分界是「会不会改变发出去的字节」：`window_size` 决定分几次调用、
/// `max_concurrent` 决定同时几个、`llm_score_weight` 决定后处理怎么用那些分数 —— 三者都
/// **只在本地生效**，写进 `params` 会被当成"厂商扩展参数"原样塞进请求体。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmRunCfg {
    /// Agent 每次处理多少条（订阅正链路）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_size: Option<usize>,
    /// 这条链路的并发上限（留空 = 跟随 `[llm] max_concurrent`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<usize>,
    /// 检测链路：LLM 标题/摘要评分在最终热度里的权重（0 = 关闭该标准、也不再发那次请求）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_score_weight: Option<f64>,
}

/// `run_for(link)` 解析出来的生效值（缺省已经填好，调用方直接用）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LlmRunResolved {
    pub window_size: usize,
    pub max_concurrent: usize,
    pub llm_score_weight: f64,
}

impl Default for LlmRunResolved {
    /// ⚠️ 窗口默认取 **`yekitrim_llm::tasks::CORRECT.default_window`**（S2 定的唯一来源），
    /// 不在 config 层再写一份 `25` —— 那正是 S4 要消灭的"第二份默认值"。
    fn default() -> Self {
        Self {
            window_size: yekitrim_llm::tasks::CORRECT.default_window.unwrap_or(25).max(1),
            max_concurrent: 3,
            llm_score_weight: 0.3,
        }
    }
}

/// llm.toml v4 的顶层形状。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmV4 {
    /// 当前用哪一份接口（[[llm.providers]].id；空 = 第一份）。
    #[serde(default)]
    pub active: String,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    /// 请求参数的唯一基准。
    #[serde(default)]
    pub defaults: LlmParamsToml,
    /// 老逃逸口：整表原样进请求体，优先级最低（D4 之后只装「未知键」）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_params: Option<toml::Table>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<LlmProviderV4>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<LlmModelEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub links: BTreeMap<String, LlmLinkSetting>,
}

#[allow(clippy::derivable_impls)] // 配置卡要求手写 Default（serde 默认值必须与 Rust 默认值一致）
impl Default for LlmProviderV4 {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            protocol: String::new(),
            base_url: String::new(),
            api_key: String::new(),
            api_key_env: String::new(),
            model: String::new(),
        }
    }
}

#[allow(clippy::derivable_impls)] // 配置卡要求手写 Default（serde 默认值必须与 Rust 默认值一致）
impl Default for LlmModelEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            provider: String::new(),
            label: String::new(),
            catalog: None,
            overrides: None,
        }
    }
}

#[allow(clippy::derivable_impls)] // 配置卡要求手写 Default（serde 默认值必须与 Rust 默认值一致）
impl Default for LlmLinkSetting {
    fn default() -> Self {
        Self {
            provider: String::new(),
            model: String::new(),
            params: LlmParamsToml::default(),
            prompt: None,
            run: None,
        }
    }
}

impl Default for LlmV4 {
    /// ⚠️ **手写**（配置卡：所有配置类型手写 Default，保证 serde 默认值与 Rust 默认值一致）。
    /// 这里最容易错的是 max_concurrent：serde 默认是 3，derive 会给 0。
    fn default() -> Self {
        Self {
            active: String::new(),
            max_concurrent: default_max_concurrent(),
            defaults: LlmParamsToml::default(),
            extra_params: None,
            providers: Vec::new(),
            models: Vec::new(),
            links: BTreeMap::new(),
        }
    }
}

fn default_max_concurrent() -> usize {
    3
}

impl LlmV4 {
    /// 按 id 找一份接口。
    pub fn provider(&self, id: &str) -> Option<&LlmProviderV4> {
        let id = id.trim();
        self.providers.iter().find(|p| p.id == id)
    }

    /// 当前生效的接口：active 命中就用它；空/认不出 ⇒ 第一份（v4 不允许「没有接口」）。
    ///
    /// ⚠️ 指向不存在的 id 时不报错（读配置要能活下来）；写通道由 problems 拦。
    pub fn active_provider(&self) -> Option<&LlmProviderV4> {
        let id = self.active.trim();
        if !id.is_empty() {
            if let Some(p) = self.provider(id) {
                return Some(p);
            }
        }
        self.providers.first()
    }

    /// 某链路要用的接口（链路里写了就用它，否则 active）。
    pub fn provider_for(&self, link: LlmLink) -> Option<&LlmProviderV4> {
        match self.links.get(link.key()) {
            Some(l) if !l.provider.trim().is_empty() => {
                self.provider(&l.provider).or_else(|| self.active_provider())
            }
            _ => self.active_provider(),
        }
    }

    /// 某链路要用的模型 id：链路指定 > 接口默认 > 该接口清单里的第一个 > 空。
    pub fn model_for(&self, link: LlmLink) -> String {
        if let Some(l) = self.links.get(link.key()) {
            if !l.model.trim().is_empty() {
                return l.model.trim().to_string();
            }
        }
        let Some(p) = self.provider_for(link) else {
            return String::new();
        };
        if !p.model.trim().is_empty() {
            return p.model.trim().to_string();
        }
        self.models
            .iter()
            .find(|m| m.provider == p.id)
            .map(|m| m.id.clone())
            .unwrap_or_default()
    }

    /// 某链路的生效参数：defaults 逐键打底，链路覆盖里写了的字段优先。
    pub fn params_for(&self, link: LlmLink) -> LlmParamsToml {
        match self.links.get(link.key()) {
            Some(l) => self.defaults.merged_over(&l.params, link),
            None => self.defaults.clone(),
        }
    }

    /// 某链路的**提示词**（`[llm.links.<link>.prompt]`）：没写 / 三项全空 ⇒ `None` = 用内建默认。
    ///
    /// ⚠️ 这是提示词的**唯一解析入口**（与 `for_link` 之于请求参数同理）：别再在调用点自己
    /// `links.get(...)` —— 那会让"哪一层优先"多出第二份实现。
    pub fn prompt_for(&self, link: LlmLink) -> Option<&LlmPromptCfg> {
        self.links
            .get(link.key())
            .and_then(|l| l.prompt.as_ref())
            .filter(|p| !p.is_empty())
    }

    /// 某链路的**本地行为参数**（窗口 / 并发 / 评分权重），缺省已经填好。
    ///
    /// ⚠️ 必须在 `materialize()` **之前**取：物化会把 `links` 清空（避免下游重算两遍参数），
    ///    所以 `LinkedLlm` 把这一份一起带走（见 `LinkedLlm::run`）。
    pub fn run_for(&self, link: LlmLink) -> LlmRunResolved {
        let d = LlmRunResolved::default();
        let run = self.links.get(link.key()).and_then(|l| l.run.as_ref());
        LlmRunResolved {
            // 0/负数不表示"没配"，但那两个值会被夹成 1（与 [llm] 并发同一条口径）
            window_size: run.and_then(|r| r.window_size).map(|n| n.max(1)).unwrap_or(d.window_size),
            max_concurrent: run
                .and_then(|r| r.max_concurrent)
                .map(|n| n.max(1))
                .unwrap_or_else(|| self.concurrency(None)),
            llm_score_weight: run
                .and_then(|r| r.llm_score_weight)
                .filter(|w| w.is_finite())
                .map(|w| w.clamp(0.0, 1.0))
                .unwrap_or(d.llm_score_weight),
        }
    }

    /// 某链路的并发上限（`run.max_concurrent` 优先，否则 `[llm] max_concurrent`，至少 1）。
    ///
    /// ⚠️ 这是**唯一**读法：三处调用点（检测 / 订正 / 精修）都走它，别再各自
    /// `concurrency(section_override)`（S1 之后 `[correct] max_concurrent` 这类段级覆盖已经搬进 llm.toml）。
    pub fn concurrency_for(&self, link: LlmLink) -> usize {
        self.run_for(link).max_concurrent
    }

    /// 清单里查某个模型的条目（按 接口 + id，id 大小写不敏感）。
    pub fn model_entry(&self, provider: &str, model: &str) -> Option<&LlmModelEntry> {
        let model = model.trim();
        self.models
            .iter()
            .find(|m| m.provider == provider && m.id.trim().eq_ignore_ascii_case(model))
    }

    /// 写配置时要拦下来的问题（空 = 没问题）。比 v3 多两条：清单 (provider, id) 重复、
    /// 清单里的 provider 认不出。
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        for p in &self.providers {
            if p.id.trim().is_empty() {
                out.push(format!("有一份接口没有 id（name = {:?}）", p.name));
            } else if seen.contains(&p.id.as_str()) {
                out.push(format!("接口 id 重复：{}", p.id));
            } else {
                seen.push(&p.id);
            }
            let proto = p.protocol.trim();
            if !proto.is_empty() && yekitrim_llm::protocol::protocol(proto).is_none() {
                out.push(format!(
                    "接口 {} 的协议认不出：{proto}（可选：{}）",
                    if p.id.is_empty() { "?" } else { p.id.as_str() },
                    yekitrim_llm::protocol::PROTOCOLS
                        .iter()
                        .map(|x| x.id)
                        .collect::<Vec<_>>()
                        .join(" / ")
                ));
            }
        }
        if self.providers.is_empty() {
            out.push("没有任何接口（至少要有 [[llm.providers]] 一条）".to_string());
        }
        let active = self.active.trim();
        if !active.is_empty() && self.provider(active).is_none() {
            out.push(format!("active 指向不存在的接口：{active}"));
        }
        let mut seen_models: Vec<(String, String)> = Vec::new();
        for m in &self.models {
            let key = (m.provider.clone(), m.id.trim().to_lowercase());
            if m.id.trim().is_empty() {
                out.push(format!("模型清单里有一条没有 id（provider = {:?}）", m.provider));
            } else if seen_models.contains(&key) {
                out.push(format!("模型清单里 (接口, id) 重复：{} / {}", m.provider, m.id));
            } else {
                seen_models.push(key);
            }
            if !m.provider.trim().is_empty() && self.provider(&m.provider).is_none() {
                out.push(format!("模型 {} 的 provider 认不出：{}", m.id, m.provider));
            }
        }
        // 链路行为参数：0 不表示"没配"，会被夹成 1 —— 照实说一句（不拦保存，只提示）
        for (key, link) in &self.links {
            let Some(run) = &link.run else { continue };
            if run.max_concurrent == Some(0) {
                out.push(format!("[llm.links.{key}.run] max_concurrent = 0 会被当成 1（想跟随全局就删掉这一行）"));
            }
            if run.window_size == Some(0) {
                out.push(format!("[llm.links.{key}.run] window_size = 0 会被当成 1"));
            }
        }
        out
    }
}

/// v3 → v4 的纯迁移（不碰磁盘；调用方负责备份与写回）。
///
/// 映射（逐条对应 v3 的回落链，保持「迁移前后生效值不变」）：
/// - 顶层 api_key/base_url/model → 一份 id = "default" 的接口（原本没有 [[llm.providers]] 时）；
/// - [llm.params] → [llm.defaults]；若 active 那份接口自己写了 params，整份取它的
///   （v3 的 effective_params 是「整体遮蔽」而不是逐键合并 —— 迁移照搬，行为不变）；
/// - [llm.overrides.<link>] → [llm.links.<link>].*（非空才写）；
/// - [llm.models."<id>"] → [[llm.models]]（provider = 当前生效接口）；
/// - [[llm.providers]].models → [[llm.models]]（provider = 各自接口）；
/// - [[llm.providers]].params → 只对当前生效那份进 defaults（其余各份表达不了，进 dropped）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Migrated {
    pub config: LlmV4,
    /// 迁移过程中没能表达出来的东西（调用方必须打 warn，别静默丢）。
    pub dropped: Vec<String>,
}

pub fn from_v3(c: &LlmV3) -> Migrated {
    let mut dropped = Vec::new();
    let active_id = c
        .active
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // ① 接口
    let mut providers: Vec<LlmProviderV4> = c
        .providers
        .iter()
        .map(|p| LlmProviderV4 {
            id: p.id.clone(),
            name: p.name.clone(),
            protocol: p.api.clone(),
            base_url: p.base_url.clone(),
            api_key: p.api_key.clone(),
            api_key_env: p.api_key_env.clone(),
            model: p.model.clone(),
        })
        .collect();
    if providers.is_empty() {
        providers.push(LlmProviderV4 {
            id: "default".to_string(),
            name: "默认接口".to_string(),
            protocol: String::new(),
            base_url: c.base_url.clone(),
            api_key: c.api_key.clone(),
            api_key_env: String::new(),
            model: c.model.clone(),
        });
    }
    let effective_id = active_id
        .clone()
        .filter(|id| providers.iter().any(|p| &p.id == id))
        .unwrap_or_else(|| providers[0].id.clone());

    // ② 参数面（v3 的整体遮蔽照搬）
    let active_params = c
        .providers
        .iter()
        .find(|p| p.id == effective_id)
        .map(|p| p.params.clone())
        .filter(|p| !p.is_empty());
    let defaults = match &active_params {
        Some(p) => p.clone(),
        None => c.params.clone(),
    };
    for p in &c.providers {
        if p.id != effective_id && !p.params.is_empty() {
            dropped.push(format!(
                "接口 {} 的 [llm.providers.params] 无法表达（v4 只有一层 [llm.defaults]），已丢弃",
                p.id
            ));
        }
    }

    // ③ 模型清单
    let mut models: Vec<LlmModelEntry> = Vec::new();
    for p in &c.providers {
        for m in &p.models {
            if models
                .iter()
                .any(|e| e.provider == p.id && e.id.trim().eq_ignore_ascii_case(m.id.trim()))
            {
                continue;
            }
            models.push(LlmModelEntry {
                id: m.id.clone(),
                provider: p.id.clone(),
                label: m.name.clone(),
                catalog: Some(m.clone()),
                overrides: None,
            });
        }
    }
    for (id, ov) in &c.models {
        match models
            .iter_mut()
            .find(|e| e.provider == effective_id && e.id.trim().eq_ignore_ascii_case(id.trim()))
        {
            Some(entry) => entry.overrides = Some(ov.clone()),
            None => models.push(LlmModelEntry {
                id: id.clone(),
                provider: effective_id.clone(),
                label: String::new(),
                catalog: None,
                overrides: Some(ov.clone()),
            }),
        }
    }

    // ④ 链路覆盖：只写非空的
    let mut links: BTreeMap<String, LlmLinkSetting> = BTreeMap::new();
    for link in [
        LlmLink::Detect,
        LlmLink::Polish,
        LlmLink::Correct,
        LlmLink::Title,
        LlmLink::Utility,
    ] {
        let ov: &LlmOverride = c.overrides.of(link);
        if ov.is_empty() {
            continue;
        }
        links.insert(
            link.key().to_string(),
            LlmLinkSetting {
                provider: String::new(),
                model: String::new(),
                params: ov.clone(),
                // v3 没有"链路提示词/行为"这两层（它们还在 config.toml 里，S4 的跨文件迁移负责搬）
                prompt: None,
                run: None,
            },
        );
    }

    Migrated {
        config: LlmV4 {
            active: effective_id,
            max_concurrent: c.max_concurrent.max(1),
            defaults,
            extra_params: c.extra_params.clone(),
            providers,
            models,
            links,
        },
        dropped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy() -> LlmV3 {
        let mut c = LlmV3::default();
        c.api_key = "sk-legacy-0123456789".to_string();
        c.base_url = "https://api.example.com/v1".to_string();
        c.model = "example-chat".to_string();
        c.params.temperature = Some(0.7);
        c.params.max_tokens = Some(4096);
        c.max_concurrent = 5;
        c
    }

    /// 老文件（只有顶层三件 + [llm.params]）迁成一份接口 + 一层 defaults。
    #[test]
    fn legacy_top_level_becomes_one_provider_and_defaults() {
        let m = from_v3(&legacy());
        assert_eq!(m.config.providers.len(), 1, "没有 providers ⇒ 造一份");
        assert_eq!(m.config.active, "default");
        let p = &m.config.providers[0];
        assert_eq!(p.base_url, "https://api.example.com/v1");
        assert_eq!(p.model, "example-chat");
        assert_eq!(m.config.defaults.temperature, Some(0.7));
        assert_eq!(m.config.defaults.max_tokens, Some(4096));
        assert_eq!(m.config.max_concurrent, 5);
        assert!(m.dropped.is_empty());
    }

    /// api_key 逐字节不变 —— 迁移唯一不可重建的东西。
    #[test]
    fn api_key_survives_byte_for_byte() {
        let m = from_v3(&legacy());
        assert_eq!(m.config.providers[0].api_key, "sk-legacy-0123456789");
    }

    /// 链路覆盖：[llm.overrides.detect] → [llm.links.detect]，空的链路不写。
    #[test]
    fn overrides_become_links_and_empty_links_are_skipped() {
        let mut c = legacy();
        c.overrides.detect.max_tokens = Some(1234);
        let m = from_v3(&c);
        assert_eq!(m.config.links.len(), 1, "只有 detect 写了");
        let d = m.config.links.get("detect").expect("detect 存在");
        assert_eq!(d.params.max_tokens, Some(1234));
        assert!(d.provider.is_empty() && d.model.is_empty(), "跟随默认");
        assert!(!m.config.links.contains_key("polish"));
    }

    /// 能力两套来源都进清单；表外覆盖挂到生效接口上。
    #[test]
    fn models_from_both_legacy_shapes_land_in_one_list() {
        let mut c = legacy();
        c.models.insert(
            "example-chat".to_string(),
            ModelOverrideToml {
                thinking_wire: Some("thinking_type".into()),
                ..Default::default()
            },
        );
        let m = from_v3(&c);
        assert_eq!(m.config.models.len(), 1);
        let e = &m.config.models[0];
        assert_eq!(e.id, "example-chat");
        assert_eq!(e.provider, "default");
        assert!(e.overrides.is_some(), "手填覆盖要带过来");
        assert_eq!(m.config.model_for(LlmLink::Detect), "example-chat");
        assert_eq!(m.config.params_for(LlmLink::Detect).max_tokens, Some(4096));
    }

    /// 幂等：迁出来的 v4 序列化→反序列化之后逐字段不变。
    #[test]
    fn migration_is_idempotent_over_a_round_trip() {
        let mut c = legacy();
        c.overrides.polish.max_tokens = Some(8192);
        let once = from_v3(&c).config;
        let text = toml::to_string(&once).expect("序列化");
        let back: LlmV4 = toml::from_str(&text).expect("反序列化");
        assert_eq!(back, once, "round-trip 必须逐字段相等:\n{text}");
    }

    /// 多接口：非生效那份的 params 表达不了 ⇒ 必须进 dropped（别静默丢）。
    #[test]
    fn inactive_provider_params_are_reported_not_silently_dropped() {
        let mut c = LlmV3::default();
        c.active = Some("a".into());
        let mut a = LlmProviderV3::default();
        a.id = "a".into();
        a.base_url = "https://a.example.com".into();
        a.model = "m-a".into();
        let mut b = LlmProviderV3::default();
        b.id = "b".into();
        b.base_url = "https://b.example.com".into();
        b.model = "m-b".into();
        b.params.temperature = Some(0.1);
        c.providers = vec![a, b];
        let m = from_v3(&c);
        assert_eq!(m.config.active, "a");
        assert!(
            m.dropped.iter().any(|s| s.contains('b') && s.contains("params")),
            "非生效接口的 params 必须报出来: {:?}",
            m.dropped
        );
    }
}

// ---------------------------------------------------------------------------
// v3 形状（只给迁移用）：字段与 v3 的 LlmConfig 逐个相同，读的是老文件。
// ---------------------------------------------------------------------------

use super::types::ModelOverrideToml as _MO;

/// v3 的链路覆盖表（**只给迁移用**：v4 里它是 `[llm.links.<link>]`）。
///
/// ⚠️ 这个类型是 S4/L4 之后 `types::LlmOverrides` 的唯一残影 —— 它只服务 `from_v3` 的读，
/// 不参与任何运行时；别再让别处用它（那会把"两套真相"又捡回来）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LlmOverridesV3 {
    #[serde(default)]
    pub detect: LlmParamsToml,
    #[serde(default)]
    pub polish: LlmParamsToml,
    #[serde(default)]
    pub correct: LlmParamsToml,
    #[serde(default)]
    pub title: LlmParamsToml,
    #[serde(default)]
    pub utility: LlmParamsToml,
}

impl LlmOverridesV3 {
    pub fn of(&self, link: LlmLink) -> &LlmParamsToml {
        match link {
            LlmLink::Detect => &self.detect,
            LlmLink::Polish => &self.polish,
            LlmLink::Correct => &self.correct,
            LlmLink::Title => &self.title,
            LlmLink::Utility => &self.utility,
        }
    }
}

/// v3 的一份接口（[[llm.providers]]，带 params / models）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmProviderV3 {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub api: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub api_key_env: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub params: LlmParamsToml,
    #[serde(default)]
    pub models: Vec<LlmModel>,
}

/// v3 的顶层形状（只有迁移读它）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmV3 {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub max_concurrent: usize,
    #[serde(default)]
    pub params: LlmParamsToml,
    #[serde(default)]
    pub models: BTreeMap<String, _MO>,
    #[serde(default)]
    pub extra_params: Option<toml::Table>,
    #[serde(default)]
    pub overrides: LlmOverridesV3,
    #[serde(default)]
    pub active: Option<String>,
    #[serde(default)]
    pub providers: Vec<LlmProviderV3>,
}

// ---------------------------------------------------------------------------
// LlmV4 的运行时语义（接线之后 LlmConfig 就是它）
// ---------------------------------------------------------------------------

impl LlmV4 {
    /// 并发数的唯一解析入口：段自己的覆盖优先，否则 [llm] max_concurrent，至少 1。
    pub fn concurrency(&self, section_override: Option<usize>) -> usize {
        section_override.unwrap_or(self.max_concurrent).max(1)
    }

    /// 把某链路落成一份可直接执行的配置：接口 = 该链路选中的那份、defaults = 生效参数、
    /// 那份接口的 model = 该链路的模型。
    ///
    /// 为什么这么绕：下游 From<&LlmV4> for LlmParams 只认「当前接口 + 一层参数面」。
    /// 与其在每个调用点各算一遍（那正是「忘记套链路参数」的来源），不如在这里一次解析干净。
    pub fn for_link(&self, link: LlmLink) -> LinkedLlm {
        // ⚠️ run 必须在 materialize **之前**取（它会清空 links）
        LinkedLlm { cfg: self.materialize(link), run: self.run_for(link) }
    }

    /// 链路物化（内部实现）：接口/模型/参数一次解析干净。别直接调它 —— 用 for_link。
    fn materialize(&self, link: LlmLink) -> LlmV4 {
        let mut out = self.clone();
        let provider_id = self.provider_for(link).map(|p| p.id.clone()).unwrap_or_default();
        let model = self.model_for(link);
        out.active = provider_id.clone();
        if let Some(p) = out.providers.iter_mut().find(|p| p.id == provider_id) {
            if !model.trim().is_empty() {
                p.model = model;
            }
        }
        out.defaults = self.params_for(link);
        // 链路覆盖已经并进 defaults，别再让下游重算一遍（否则会叠两遍）。
        out.links.clear();
        out
    }

    /// 当前接口的明文 key（空串 = 没配）。server 的 draft/测试通道与精修命令的"配没配"判定读它。
    pub fn effective_api_key(&self) -> String {
        self.active_provider().map(|p| p.key()).unwrap_or_default()
    }

    /// 当前接口的 base_url（空串 = 没配）。
    pub fn effective_base_url(&self) -> String {
        self.active_provider().map(|p| p.base_url.clone()).unwrap_or_default()
    }

    /// 当前接口的默认模型。
    pub fn effective_model(&self) -> String {
        self.active_provider().map(|p| p.model.clone()).unwrap_or_default()
    }

    /// 造一份「一条接口 + 一个模型」的最小配置（测试夹具与迁移默认值共用）。
    pub fn single(base_url: &str, model: &str, api_key: &str) -> LlmV4 {
        let mut c = LlmV4 { active: "default".to_string(), ..Default::default() };
        c.providers.push(LlmProviderV4 {
            id: "default".to_string(),
            name: "默认接口".to_string(),
            protocol: String::new(),
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
            api_key_env: String::new(),
            model: model.to_string(),
        });
        c
    }

    /// 当前接口配没配 key（显式 api_key 或 api_key_env 指向的变量非空）。
    ///
    /// ⚠️ 多接口下没有「顶层 api_key」这种东西了：判定必须走 active 那份 ——
    /// 读顶层会把每个用户都判成「没配」而静默退化成默认事件（2026-10-20 那一族 bug）。
    pub fn has_api_key(&self) -> bool {
        self.active_provider().map(|p| !p.key().is_empty()).unwrap_or(false)
    }

    /// 旧名字（server 的两处调用沿用它）：转调 problems()。
    pub fn provider_problems(&self) -> Vec<String> {
        self.problems()
    }

    /// 该接口 + 该模型的能力面：内置表打底 → 清单里的目录形状 → 清单里的手填覆盖。
    pub fn effective_caps(&self, provider: &str, model: &str) -> yekitrim_llm::ModelCaps {
        let base_url = self.provider(provider).map(|p| p.base_url.clone()).unwrap_or_default();
        let mut caps = yekitrim_llm::catalog::caps_for(&base_url, model);
        if let Some(entry) = self.model_entry(provider, model) {
            if let Some(cat) = &entry.catalog {
                super::types::apply_provider_model(&mut caps, cat);
            }
            if let Some(ov) = &entry.overrides {
                super::types::apply_model_override(&mut caps, ov);
            }
        }
        caps
    }
}

impl LlmV4 {
    /// 配置 -> 传输层参数（内部实现；业务侧只经 LinkedLlm 拿到它）。
    pub(crate) fn to_params(&self) -> yekitrim_llm::LlmParams {
        use yekitrim_llm::params::Support;
        let provider = self.active_provider();
        let (api_key, base_url, model) = match provider {
            Some(p) => (p.key(), p.base_url.clone(), p.model.clone()),
            None => (String::new(), String::new(), String::new()),
        };
        let mut caps = self.effective_caps(
            provider.map(|p| p.id.as_str()).unwrap_or_default(),
            &model,
        );
        let p = &self.defaults;
        let reasoning = p
            .reasoning
            .as_deref()
            .and_then(yekitrim_llm::ReasoningLevel::from_thinking_flag);
        // ⚠️ 「省略 temperature = 不发」只能借 caps 这一个出口表达（LlmParams.temperature 不是 Option）。
        // S2/F6 会把这条改成 Option，届时这里整段删掉。
        if p.temperature.is_none() {
            caps.temperature = Support::Unsupported;
        }
        yekitrim_llm::LlmParams {
            api_key,
            base_url,
            model,
            temperature: p.temperature.filter(|t| t.is_finite()).unwrap_or_default(),
            max_tokens: p.max_tokens,
            reasoning,
            thinking_budget: p.thinking_budget.filter(|n| *n > 0),
            top_p: p.top_p.filter(|v| v.is_finite()),
            json_mode: p.json_mode.unwrap_or(false),
            extra_params: self.extra_params.as_ref().and_then(|t| serde_json::to_value(t).ok()),
            caps,
        }
    }
}

/// 一条链路物化后的配置：LlmConfig::for_link(link) 的产物。
///
/// 存在的唯一理由：让「忘记套链路参数」**编译不过** —— 客户端构造只接受它
/// （LlmClient::new / Agent::new 吃 Into<LlmParams>，这里只为它实现转换）。
/// 想改「这一趟发什么」就改 [llm.links.<link>] 或 [llm.defaults]，别在调用点手改参数。
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedLlm {
    /// 物化后的配置（接口 / 模型 / 参数都已解析干净，links 已清空）。
    cfg: LlmV4,
    /// 这条链路的**本地行为参数**（窗口 / 并发 / 评分权重）。
    ///
    /// ⚠️ 为什么放在这里而不是"再查一次 links"：`materialize()` 会 `out.links.clear()`
    /// （参数已经并进 defaults，不能让下游重算两遍）。若调用点想再取窗口/并发，就只能
    /// 拿着物化后的配置去查 —— 那会**永远查不到**，然后静默用默认值。
    run: LlmRunResolved,
}

impl LinkedLlm {
    /// 物化后的配置本体（读连接信息 / 打日志用）。
    pub fn config(&self) -> &LlmV4 {
        &self.cfg
    }
    /// 这条链路的本地行为参数（窗口 / 并发 / 评分权重）。
    pub fn run(&self) -> LlmRunResolved {
        self.run
    }
}

impl std::ops::Deref for LinkedLlm {
    type Target = LlmV4;
    fn deref(&self) -> &LlmV4 {
        &self.cfg
    }
}

impl From<LinkedLlm> for yekitrim_llm::LlmParams {
    fn from(c: LinkedLlm) -> Self {
        c.cfg.to_params()
    }
}

impl From<&LinkedLlm> for yekitrim_llm::LlmParams {
    fn from(c: &LinkedLlm) -> Self {
        c.cfg.to_params()
    }
}

impl LlmProviderV4 {
    /// 明文 key 优先，其次环境变量；都没有 ⇒ 空串（构造客户端时才会报错）。
    ///
    /// ⚠️ 模板里那个占位 key 等于没配（与 v3 的 PLACEHOLDER_API_KEY 同口径）——
    /// 否则「照模板填一半」会被判成已配置，出片时才炸在 401。
    pub fn key(&self) -> String {
        let explicit = self.api_key.trim();
        if !explicit.is_empty() && explicit != PLACEHOLDER_API_KEY {
            return explicit.to_string();
        }
        let var = self.api_key_env.trim();
        if var.is_empty() {
            return String::new();
        }
        std::env::var(var).unwrap_or_default()
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;

    fn two_providers() -> LlmV4 {
        let mut c = LlmV4::default();
        c.max_concurrent = 3;
        c.providers.push(LlmProviderV4 {
            id: "main".into(),
            name: "主力".into(),
            protocol: String::new(),
            base_url: "https://api.example.com/v1".into(),
            api_key: "sk-main".into(),
            api_key_env: String::new(),
            model: "big-model".into(),
        });
        c.providers.push(LlmProviderV4 {
            id: "cheap".into(),
            name: "便宜".into(),
            protocol: String::new(),
            base_url: "https://api.example.com/v1".into(),
            api_key: "sk-cheap".into(),
            api_key_env: String::new(),
            model: "small-model".into(),
        });
        c.active = "main".into();
        c
    }

    /// 链路写了 provider + model ⇒ for_link 把它落成"当前接口就是它、模型就是那个"。
    #[test]
    fn for_link_materializes_provider_model_and_params() {
        let mut c = two_providers();
        c.defaults.max_tokens = Some(4096);
        c.links.insert(
            "detect".into(),
            LlmLinkSetting {
                provider: "cheap".into(),
                model: "small-model-v2".into(),
                prompt: None,
                run: None,
                params: LlmParamsToml { max_tokens: Some(512), ..Default::default() },
            },
        );
        let d = c.for_link(LlmLink::Detect);
        assert_eq!(d.active, "cheap", "接口按链路选");
        let p = d.active_provider().expect("有接口");
        assert_eq!(p.model, "small-model-v2", "模型按链路选");
        assert_eq!(d.defaults.max_tokens, Some(512), "链路覆盖并进 defaults");
        assert!(d.links.is_empty(), "已经并进 defaults，不许留着让下游重算");
        // 没写覆盖的链路跟随 active
        let pol = c.for_link(LlmLink::Polish);
        assert_eq!(pol.active, "main");
        assert_eq!(pol.active_provider().unwrap().model, "big-model");
        assert_eq!(pol.defaults.max_tokens, Some(4096), "跟随 defaults");
    }

    /// 并发：段覆盖 > 全局，且至少 1。
    #[test]
    fn concurrency_prefers_section_then_global_with_floor_one() {
        let mut c = two_providers();
        assert_eq!(c.concurrency(None), 3);
        assert_eq!(c.concurrency(Some(7)), 7);
        assert_eq!(c.concurrency(Some(0)), 1, "0 不许放行（会变成不限并发）");
        c.max_concurrent = 0;
        assert_eq!(c.concurrency(None), 1);
    }

    /// From<&LlmV4>：key/地址/模型来自接口，参数来自 defaults，能力来自清单。
    #[test]
    fn llm_params_are_built_from_provider_defaults_and_model_list() {
        let mut c = LlmV4::default();
        c.providers.push(LlmProviderV4 {
            id: "main".into(),
            name: String::new(),
            protocol: String::new(),
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: String::new(),
            api_key_env: String::new(),
            model: "deepseek-v4-pro".into(),
        });
        c.providers[0].api_key = "sk-from-provider".into();
        c.active = "main".into();
        c.defaults.temperature = Some(0.2);
        c.defaults.max_tokens = Some(2048);
        c.defaults.json_mode = Some(true);
        // 能力：表里认识 deepseek ⇒ thinking_type；清单里再把它换成 openai_effort
        let before: yekitrim_llm::LlmParams = c.to_params();
        assert_eq!(before.api_key, "sk-from-provider");
        assert_eq!(before.model, "deepseek-v4-pro");
        assert!((before.temperature - 0.2).abs() < 1e-9);
        assert_eq!(before.max_tokens, Some(2048));
        assert!(before.json_mode);
        assert_eq!(before.caps.dialect.id, "thinking_type");
        c.models.push(LlmModelEntry {
            id: "deepseek-v4-pro".into(),
            provider: "main".into(),
            label: String::new(),
            catalog: None,
            overrides: Some(ModelOverrideToml {
                thinking_wire: Some("openai_effort".into()),
                ..Default::default()
            }),
        });
        let after: yekitrim_llm::LlmParams = c.to_params();
        assert_eq!(after.caps.dialect.id, "openai_effort", "清单里的覆盖必须真的进 caps");
    }

    /// 「省略温度 = 不发」仍借 caps 表达（S2/F6 之前的老口径，先钉住，改的时候会看到它翻面）。
    #[test]
    fn missing_temperature_still_means_do_not_send() {
        let mut c = two_providers();
        c.defaults.temperature = None;
        let p: yekitrim_llm::LlmParams = c.to_params();
        assert_eq!(p.caps.temperature, yekitrim_llm::params::Support::Unsupported);
    }

    /// problems：清单 (provider,id) 重复、provider 认不出、没有接口 —— 都要能报出来。
    #[test]
    fn problems_flag_duplicate_models_and_unknown_provider() {
        let mut c = two_providers();
        assert!(c.problems().is_empty(), "干净的配置不该有问题");
        c.models.push(LlmModelEntry { id: "m1".into(), provider: "main".into(), ..Default::default() });
        c.models.push(LlmModelEntry { id: "M1".into(), provider: "main".into(), ..Default::default() });
        c.models.push(LlmModelEntry { id: "m2".into(), provider: "ghost".into(), ..Default::default() });
        let p = c.problems();
        assert!(p.iter().any(|s| s.contains("重复")), "大小写不同的同 id 也算重复: {p:?}");
        assert!(p.iter().any(|s| s.contains("ghost")), "认不出的 provider 要报: {p:?}");
        let empty = LlmV4::default();
        assert!(empty.problems().iter().any(|s| s.contains("没有任何接口")));
    }
}

#[cfg(test)]
mod caps_source_tests {
    use super::*;

    /// 造一份「一支内置表认识的模型」的配置（deepseek 的方言是 thinking_type）。
    fn with_model_entry(entry: Option<LlmModelEntry>) -> LlmV4 {
        let mut c = LlmV4::single("https://api.deepseek.com/v1", "deepseek-v4-pro", "k");
        if let Some(mut e) = entry {
            e.id = "deepseek-v4-pro".into();
            e.provider = "default".into();
            c.models.push(e);
        }
        c
    }

    /// **表里写了、清单里也写了 ⇒ 清单说了算**（内置表只做建议值）。
    #[test]
    fn model_list_wins_over_the_builtin_table() {
        let only_table = with_model_entry(None);
        let caps = only_table.effective_caps("default", "deepseek-v4-pro");
        assert_eq!(caps.dialect.id, "thinking_type", "内置表给的是 thinking_type");

        let with_list = with_model_entry(Some(LlmModelEntry {
            overrides: Some(ModelOverrideToml {
                thinking_wire: Some("openai_effort".into()),
                ..Default::default()
            }),
            ..Default::default()
        }));
        let caps2 = with_list.effective_caps("default", "deepseek-v4-pro");
        assert_eq!(caps2.dialect.id, "openai_effort", "清单里写了的必须盖过内置表");
    }

    /// **清单没写的字段仍然由内置表补齐**（建议值的价值就在这：用户只改他关心的那一项）。
    #[test]
    fn builtin_table_still_fills_the_gaps_the_list_is_silent_about() {
        let c = with_model_entry(Some(LlmModelEntry {
            overrides: Some(ModelOverrideToml {
                input: Some(vec!["text".into(), "image".into()]),
                ..Default::default()
            }),
            ..Default::default()
        }));
        let caps = c.effective_caps("default", "deepseek-v4-pro");
        assert!(caps.input.image, "清单里写了多模态 ⇒ 生效");
        assert_eq!(
            caps.dialect.id, "thinking_type",
            "清单没提推理写法 ⇒ 照样是内置表给的那条（不是退回 none）"
        );
    }

    /// **表里没有、清单里写了 ⇒ 清单直接生效**（自建网关 / 表外模型的那条路）。
    #[test]
    fn unknown_model_gets_its_caps_only_from_the_list() {
        let mut c = LlmV4::single("https://my-gateway.example.com/v1", "my-model", "k");
        let before = c.effective_caps("default", "my-model");
        assert_eq!(before.dialect.id, "none", "表外模型保守默认");
        assert!(!before.input.image, "没有被猜成多模态");

        c.models.push(LlmModelEntry {
            id: "my-model".into(),
            provider: "default".into(),
            overrides: Some(ModelOverrideToml {
                thinking_wire: Some("openai_effort".into()),
                ..Default::default()
            }),
            ..Default::default()
        });
        let after = c.effective_caps("default", "my-model");
        assert_eq!(after.dialect.id, "openai_effort", "清单是表外模型的唯一来源");
    }
}

impl LlmV4 {
    /// 一条链路的**生效快照**（给 /api/config/llm/effective 与 L3 的链路表格填灰字实值）。
    ///
    /// 为什么放在配置层：生效值是 for_link 解析出来的那三件事（接口 / 模型 / 参数），
    /// 前端自己算一遍就是把同一个规则抄第二份 —— 那正是 L1 之前「界面显示的与真发的不一致」的来源。
    pub fn link_snapshot(&self, link: LlmLink) -> serde_json::Value {
        let resolved = self.for_link(link);
        let p = resolved.active_provider();
        let params = &resolved.defaults;
        serde_json::json!({
            "link": link.key(),
            "label": link.label_zh(),
            "overridden": {
                "provider": self.links.get(link.key()).map(|l| !l.provider.trim().is_empty()).unwrap_or(false),
                "model": self.links.get(link.key()).map(|l| !l.model.trim().is_empty()).unwrap_or(false),
                "params": self.links.get(link.key()).map(|l| !l.params.is_empty()).unwrap_or(false),
            },
            "provider": {
                "id": p.map(|x| x.id.clone()).unwrap_or_default(),
                "name": p.map(|x| x.name.clone()).unwrap_or_default(),
                "base_url": p.map(|x| x.base_url.clone()).unwrap_or_default(),
                "api_key_set": p.map(|x| !x.key().is_empty()).unwrap_or(false),
            },
            "model": resolved.effective_model(),
            // 本地行为参数（窗口 / 并发 / 评分权重）：不进请求体，但界面与日志要能解释"这一趟怎么跑"
            "run": {
                "window_size": self.run_for(link).window_size,
                "max_concurrent": self.run_for(link).max_concurrent,
                "llm_score_weight": self.run_for(link).llm_score_weight,
            },
            // 提示词覆盖与"参数覆盖"分开报：前者不改请求字节（D2 / L3 的灰字要用）
            "prompt_overridden": self.prompt_for(link).is_some(),
            "params": {
                "temperature": params.temperature,
                "max_tokens": params.max_tokens,
                "reasoning": params.reasoning,
                "thinking_budget": params.thinking_budget,
                "top_p": params.top_p,
                "json_mode": params.json_mode,
            },
            "max_concurrent": self.concurrency(None),
        })
    }
}

#[cfg(test)]
mod link_snapshot_tests {
    use super::*;

    /// 快照 = for_link 的解析结果（前端不再自己算）。
    #[test]
    fn snapshot_reports_the_effective_values_and_override_flags() {
        let mut c = LlmV4::single("https://api.example.com/v1", "big-model", "k");
        c.defaults.max_tokens = Some(4096);
        // detect 覆盖模型与上限；polish 什么都不写（跟随默认）
        c.links.insert(
            "detect".into(),
            LlmLinkSetting {
                model: "small-model".into(),
                params: LlmParamsToml { max_tokens: Some(512), ..Default::default() },
                ..Default::default()
            },
        );
        let d = c.link_snapshot(LlmLink::Detect);
        assert_eq!(d["model"], "small-model", "链路选了模型 ⇒ 快照报它");
        assert_eq!(d["params"]["max_tokens"], 512);
        assert_eq!(d["overridden"]["model"], true);
        assert_eq!(d["overridden"]["provider"], false, "没写接口 ⇒ 跟随 active");
        assert_eq!(d["provider"]["id"], "default");
        assert_eq!(d["provider"]["api_key_set"], true, "key 只报有没有，不报值");
        assert!(d.get("api_key").is_none(), "快照里不许出现明文 key");

        let p = c.link_snapshot(LlmLink::Polish);
        assert_eq!(p["model"], "big-model", "没写 ⇒ 跟随接口默认模型");
        assert_eq!(p["params"]["max_tokens"], 4096, "没写 ⇒ 跟随 defaults");
        assert_eq!(p["overridden"]["model"], false);
    }

    /// 并发与链路无关（全局一个旋钮），但快照要如实报出来。
    #[test]
    fn snapshot_carries_the_global_concurrency() {
        let mut c = LlmV4::single("https://api.example.com/v1", "m", "k");
        c.max_concurrent = 7;
        assert_eq!(c.link_snapshot(LlmLink::Utility)["max_concurrent"], 7);
    }
}
