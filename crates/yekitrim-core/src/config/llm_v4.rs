//! llm.toml v4 形状与 v3 → v4 迁移（整改「LLM 配置重构」L1）。
//!
//! v3 的 LlmConfig 是两份真相并存（顶层 api_key/base_url/model + [llm.params] / [llm.models] 一套，
//! [[llm.providers]] + [llm.overrides] 另一套，靠 active 回落链切换）。v4 只有一份：
//!
//!     [llm]
//!     active = "main"        # 当前用哪一份接口（永远有值）
//!     max_concurrent = 3
//!     [llm.defaults]         # 请求参数的唯一基准（逐键）
//!     [[llm.providers]]      # 「怎么连」：id/name/api/base_url/api_key/api_key_env/model
//!     [[llm.models]]         # 「有哪些模型、属谁、能力如何」（唯一清单）
//!     [llm.links.detect]     # 「这条链路怎么发」：provider?/model?/参数覆盖
//!
//! 本文件现在是 L1 的第一片：只放新形状与纯迁移函数（from_v3），不动运行时读取路径。
//! 等 LlmConfig 换成这套字段之后，from_v3 就是 version.rs 里 v3→v4 那一步的实现。
//!
//! ⚠️ 迁移的两条硬要求（都有用例钉住）：api_key 逐字节不变；已迁过的形状再迁一次不变（幂等）。
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::types::{LlmConfig, LlmLink, LlmModel, LlmOverride, LlmParamsToml, ModelOverrideToml};

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
}

impl LlmLinkSetting {
    /// 这一行是不是空的（既没选接口/模型，也没有任何参数覆盖）—— 空行不写回文件。
    pub fn is_empty_setting(&self) -> bool {
        self.provider.trim().is_empty() && self.model.trim().is_empty() && self.params.is_empty()
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

impl Default for LlmLinkSetting {
    fn default() -> Self {
        Self {
            provider: String::new(),
            model: String::new(),
            params: LlmParamsToml::default(),
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

pub fn from_v3(c: &LlmConfig) -> Migrated {
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

    fn legacy() -> LlmConfig {
        let mut c = LlmConfig::default();
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
        let mut c = LlmConfig::default();
        c.active = Some("a".into());
        let mut a = super::super::types::LlmProvider::default();
        a.id = "a".into();
        a.base_url = "https://a.example.com".into();
        a.model = "m-a".into();
        let mut b = super::super::types::LlmProvider::default();
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
