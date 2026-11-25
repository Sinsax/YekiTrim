//! 链路/任务的规格表（S2）：一次 LLM 任务的默认窗口、默认并发、用量标签都住在这里。
//!
//! 为什么需要它：用量标签以前是 11 处手写的中文字面量（散在 5 个文件里），窗口大小只有订正那条能配，
//! 并发则在每个调用点各自算。三件事都是「同一个概念、多个来源」，漏一处就是统计里出现空 label。
//!
//! ⚠️ **文案一个字都不许改**：这些 label 是用量报表的分组键（历史数据按它聚合）。
//! ⚠️ 本表在 yekitrim-llm 里 —— 它不认识配置层，所以 link_key 用字符串（与 LlmLink::key() 对齐），
//!    由调用方自己映射；这样传输层不必反向依赖 core。

/// 一条 LLM 任务的规格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LlmTaskSpec {
    /// 稳定 ASCII key（测试与日志用）。
    pub key: &'static str,
    /// 属于哪条链路（= LlmLink::key()：detect / polish / correct / title / utility）。
    pub link_key: &'static str,
    /// 用量标签（报表分组键，中文字面量 —— 历史数据按它聚合，改文案等于断档）。
    pub label: &'static str,
    /// Agent 的默认窗口大小（None = 用 Agent 的内建默认 25）。
    pub default_window: Option<usize>,
    /// 段级并发覆盖（None = 跟随 [llm] max_concurrent）。
    pub default_concurrency: Option<usize>,
    /// 界面上要不要出现（订正那条只隐藏、仍生效）。
    pub ui_visible: bool,
}

const fn spec(
    key: &'static str,
    link_key: &'static str,
    label: &'static str,
    default_window: Option<usize>,
    ui_visible: bool,
) -> LlmTaskSpec {
    LlmTaskSpec { key, link_key, label, default_window, default_concurrency: None, ui_visible }
}

/// 全部任务（顺序 = 报表里习惯的阅读顺序）。
///
/// ⚠️ 每条任务**必须**是独立常量（而不是只在这张表里出现一次）：业务侧要用
/// `tasks::POLISH_SEGMENT.label` 这样的写法取值，**拼错 key 就是编译错误**，
/// 而不是运行时拿到一个兜底标签（那样统计里会出现莫名其妙的"LLM"分组）。
pub const TASKS: &[&LlmTaskSpec] = &[
    &DETECT_WINDOW,
    &DETECT_HOT_SCORE,
    &POLISH_SEGMENT,
    &POLISH_EVENTS,
    &POLISH_KEYWORDS,
    &POLISH_STICKER_PICK,
    &CORRECT,
    &COVER_TITLE,
    &STICKER_PACK_TAGS,
    &STICKER_SCAN_TAGS,
    &EMOJI_CLASSIFY,
    &CONNECTION_TEST,
    &LLM_TEST,
];

pub const DETECT_WINDOW: LlmTaskSpec = spec("detect_window", "detect", "事件检测窗口", None, true);
pub const DETECT_HOT_SCORE: LlmTaskSpec =
    spec("detect_hot_score", "detect", "事件热度LLM评分", None, true);
pub const POLISH_SEGMENT: LlmTaskSpec = spec("polish_segment", "polish", "精修分段", None, true);
pub const POLISH_EVENTS: LlmTaskSpec = spec("polish_events", "polish", "精修逐段事件", None, true);
pub const POLISH_KEYWORDS: LlmTaskSpec = spec("polish_keywords", "polish", "精修关键词", None, true);
pub const POLISH_STICKER_PICK: LlmTaskSpec =
    spec("polish_sticker_pick", "polish", "精修贴纸挑选", None, true);
pub const CORRECT: LlmTaskSpec = spec("correct", "correct", "字幕订正", Some(25), false);
pub const COVER_TITLE: LlmTaskSpec = spec("cover_title", "title", "封面文案", None, true);
pub const STICKER_PACK_TAGS: LlmTaskSpec =
    spec("sticker_pack_tags", "utility", "贴纸包标签", None, true);
pub const STICKER_SCAN_TAGS: LlmTaskSpec =
    spec("sticker_scan_tags", "utility", "贴纸扫描标签", None, true);
pub const EMOJI_CLASSIFY: LlmTaskSpec =
    spec("emoji_classify", "utility", "emoji 多维分类", None, true);
pub const CONNECTION_TEST: LlmTaskSpec = spec("connection_test", "utility", "测试链路", None, false);
pub const LLM_TEST: LlmTaskSpec = spec("llm_test", "utility", "LLM 测试", None, false);

/// 按 key 找一条。
pub fn task(key: &str) -> Option<&'static LlmTaskSpec> {
    let key = key.trim();
    TASKS.iter().copied().find(|t| t.key == key)
}

/// 某条链路下的全部任务。
pub fn by_link(link_key: &str) -> Vec<&'static LlmTaskSpec> {
    TASKS.iter().copied().filter(|t| t.link_key == link_key).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 五条链路一个都不能漏（新增链路忘了登记 = 它的用量在报表里没有标签）。
    #[test]
    fn every_link_has_at_least_one_task() {
        for link in ["detect", "polish", "correct", "title", "utility"] {
            assert!(!by_link(link).is_empty(), "链路 {link} 没有登记任何任务");
        }
    }

    /// key 与 label 唯一、非空（label 是报表分组键，重复会把两组数据合并）。
    #[test]
    fn keys_and_labels_are_unique_and_non_empty() {
        for t in TASKS {
            assert!(!t.key.trim().is_empty(), "有空 key");
            assert!(!t.label.trim().is_empty(), "任务 {} 没有用量标签", t.key);
            assert!(!t.link_key.trim().is_empty(), "任务 {} 没有 link", t.key);
        }
        for i in 0..TASKS.len() {
            for j in (i + 1)..TASKS.len() {
                assert_ne!(TASKS[i].key, TASKS[j].key, "key 重复：{}", TASKS[i].key);
                assert_ne!(TASKS[i].label, TASKS[j].label, "label 重复：{}", TASKS[i].label);
            }
        }
    }

    /// 历史文案不变（改这些字符串等于把用量报表劈成两段）。
    #[test]
    fn known_labels_are_frozen() {
        for (key, label) in [
            ("polish_segment", "精修分段"),
            ("polish_keywords", "精修关键词"),
            ("sticker_pack_tags", "贴纸包标签"),
            ("emoji_classify", "emoji 多维分类"),
            ("cover_title", "封面文案"),
            ("detect_hot_score", "事件热度LLM评分"),
        ] {
            assert_eq!(task(key).expect(key).label, label, "文案被改了：{key}");
        }
    }

    /// 订正那条只从界面隐藏（代码与配置照旧生效）。
    #[test]
    fn correction_task_is_hidden_but_present() {
        let t = task("correct").expect("correct");
        assert!(!t.ui_visible, "界面不显示");
        assert_eq!(t.link_key, "correct");
        assert_eq!(t.default_window, Some(25), "订正窗口的默认值 25");
    }
}
