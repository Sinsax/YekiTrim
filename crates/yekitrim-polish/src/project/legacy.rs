//! 老工程的字段升级：事件的「开始 + 持续时间」→「开始 + 结束」。
//!
//! 核心函数在渲染核心里（`yekitrim_render_core::legacy::upgrade_event_times`，与 `Event`
//! 类型同源），这里补一个 TOML 入口——`polish.toml` 是先解析成 `toml::Value` 再升级、
//! 最后走 serde 进 `ProjectFile` 的（见 `validate.rs::load_config_from`）。

use serde_json::Value;

pub use yekitrim_render_core::legacy::upgrade_event_times;

/// TOML 值进、JSON 值出：polish.toml 走这条路（`toml::Value` 实现了 Serialize）。
pub fn upgrade_toml_value(value: &toml::Value) -> anyhow::Result<Value> {
    let mut json = serde_json::to_value(value)?;
    upgrade_event_times(&mut json);
    strip_retired_keys(&mut json);
    let retired = migrate_retired_effects(&mut json);
    // 日志要**点名、给数**（用户口径：不许静默丢）。只改内存里的值 —— 下一次保存才落盘，
    // 本次不动磁盘上的 polish.toml（与 `upgrade_event_times` 同一条规矩）。
    if retired.pulse > 0 {
        tracing::info!(
            "已退役 pulse：丢弃 {} 条（读时丢弃；下一次保存才落盘，本次不动磁盘）",
            retired.pulse
        );
    }
    if retired.zoom_bounce > 0 {
        tracing::info!(
            "已退役 zoom_bounce：丢弃 {} 条（同上）",
            retired.zoom_bounce
        );
    }
    Ok(json)
}

// ---------------------------------------------------------------------------
// 已退役事件类型的读时迁移
// ---------------------------------------------------------------------------

/// **已退役事件类型的读时丢弃**计数（分类，便于日志点名与用例断言）。
///
/// 2026-10-20 用户口径：「pulse 这个效果删除」。它原本与 `zoom_bounce` 共用同一个缩放槽
/// （`Math.max(zbZoom, plZoom)` 都乘在 `curZoom` 上），所以**两条一起退役**、都不给迁移目标：
/// 1.06 的脉冲本来就看不太出来（这正是用户要删它的原因），而 `Camera` 没有 `sfx`/`sfx_volume`、
/// 又是**持续状态**，迁成运镜得成对生成推/回位事件 + 补音效事件 ⇒ 事件翻倍还会改镜头节奏。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RetiredCounts {
    pub pulse: usize,
    pub zoom_bounce: usize,
}

impl RetiredCounts {
    pub fn is_empty(&self) -> bool {
        self.pulse == 0 && self.zoom_bounce == 0
    }
    pub fn total(&self) -> usize {
        self.pulse + self.zoom_bounce
    }
}

/// 已退役的事件类型名（读时**丢弃**，不是改写）。
const RETIRED_EVENT_TYPES: [&str; 2] = ["pulse", "zoom_bounce"];

/// **已退役事件类型的读时丢弃**：老工程里的 `pulse` / `zoom_bounce` 事件在读的时候就摘掉，
/// 并**按类型计数**（调用方据此打日志，见 `upgrade_toml_value`）。
///
/// # 为什么是"丢弃"而不是迁移
///
/// - `pulse`：×1.06 / ×0.97 / 0.48s 的整帧缩放脉冲。迁成运镜 `zoom="微"` 的话：
///   ① `Camera` **没有** `sfx` / `sfx_volume`（要保音效得再生成一条 sfx 事件）；
///   ② camera 是**持续状态**、pulse 是**瞬时包络** ⇒ 还得成对生成"推 + 回位"。
///   两条加起来 = 事件数翻倍 + 改掉镜头节奏，而 1.06 本来就看不太出来 ⇒ **退役、不给目标**。
/// - `zoom_bounce`：它原本就是"同槽的更弱版本"，目标没了，跟着一起丢。
///
/// ⚠️ **枚举变体留着**（`Event::Pulse` / 老类型名不进 serde 的 `deny_unknown_fields` 面）——
/// 删变体会让老文件**直接打不开**，而这是 R4 的红线。读时丢掉是唯一安全形态。
///
/// ⚠️ 只认 `type`（事件的 tag），**别顺带改 `kind`**：`[[edl.clips]]` 与封面特效用的也是 kind。
fn migrate_retired_effects(value: &mut Value) -> RetiredCounts {
    fn walk(value: &mut Value, counts: &mut RetiredCounts) {
        match value {
            Value::Array(items) => {
                items.retain_mut(|item| {
                    if let Some(kind) = item.get("type").and_then(|t| t.as_str()) {
                        if RETIRED_EVENT_TYPES.contains(&kind) {
                            if kind == "pulse" {
                                counts.pulse += 1;
                            } else {
                                counts.zoom_bounce += 1;
                            }
                            return false;
                        }
                    }
                    walk(item, counts);
                    true
                });
            }
            Value::Object(map) => {
                for (_, v) in map.iter_mut() {
                    if v.is_array() || v.is_object() {
                        walk(v, counts);
                    }
                }
            }
            _ => {}
        }
    }
    let mut counts = RetiredCounts::default();
    walk(value, &mut counts);
    counts
}
/// 把**已经下线的配置键**在读的时候吞掉（2026-10-01，Phase ③）。
///
/// ⚠️ **为什么不能直接删字段**：`SubtitleStyle` 上有 `#[serde(deny_unknown_fields)]`
/// （`yekitrim-render-core/src/types.rs`）—— 老工程里留着 `entrance = "cycle-5"` 的话，
/// 删掉字段就等于让那些工程**直接读不出来**。所以先在这里吞掉，schema 才敢删。
///
/// ⚠️ **为什么该下线**：`entrance` 是 vtedit 时代的「字幕入场动画」，默认值就是 `"cycle-5"`，
/// 而 `polish init` 的模板还替**每个新工程**写上 `entrance = "fade-up"` —— 但全仓**没有任何读者**：
/// 本仓的 canvas 出片路径（`templates/index.html`）与参照都没有这类动画。留着的代价是
/// 「让人以为能配」（交接文档的 E6 条目点过这件事）。要真做就得**发明**一套动画规格 —— 没有参照，
/// 所以选择「从契约里移除」。
fn strip_retired_keys(json: &mut Value) {
    if let Some(sub) = json
        .get_mut("style")
        .and_then(|s| s.get_mut("subtitle"))
        .and_then(|v| v.as_object_mut())
    {
        sub.remove("entrance");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_value_is_upgraded() {
        let raw = r#"
[[segments]]
start = 0.0
end = 20.0

[[segments.events]]
type = "camera"
time = 1.0
duration = 4.0
zoom = "中"
"#;
        let toml_value: toml::Value = toml::from_str(raw).unwrap();
        let json = upgrade_toml_value(&toml_value).unwrap();
        let segs: Vec<crate::project::Segment> = serde_json::from_value(json["segments"].clone()).unwrap();
        assert_eq!(segs[0].events[0].end(), Some(5.0));
    }

    #[test]
    fn toml_without_duration_is_untouched() {
        let raw = r#"
[[segments]]
start = 0.0
end = 20.0

[[segments.events]]
type = "camera"
time = 1.0
end = 5.0
zoom = "中"
"#;
        let toml_value: toml::Value = toml::from_str(raw).unwrap();
        let json = upgrade_toml_value(&toml_value).unwrap();
        let segs: Vec<crate::project::Segment> = serde_json::from_value(json["segments"].clone()).unwrap();
        assert_eq!(segs[0].events[0].end(), Some(5.0));
    }

    /// 已下线的 `entrance` 要被**读时吞掉** —— 否则 `SubtitleStyle` 的 `deny_unknown_fields`
    /// 会让**所有老工程直接读不出来**（这正是不能「直接删字段」的原因）。
    #[test]
    fn retired_entrance_key_is_swallowed() {
        let raw = r#"
[style.subtitle]
font = "LXGW WenKai"
entrance = "cycle-5"
"#;
        let toml_value: toml::Value = toml::from_str(raw).unwrap();
        let json = upgrade_toml_value(&toml_value).unwrap();
        assert!(
            json["style"]["subtitle"].get("entrance").is_none(),
            "老工程的 `entrance` 必须在读时被吞掉"
        );
        // 更强的断言：吞掉之后真的能反序列化进 `SubtitleStyle`（它带 `deny_unknown_fields`）
        let style: yekitrim_render_core::types::SubtitleStyle = serde_json::from_value(json["style"]["subtitle"].clone())
            .expect("吞掉已下线的键之后必须读得出来");
        assert_eq!(style.font, "LXGW WenKai");
    }

    // -----------------------------------------------------------------------
    // 已退役事件类型的读时丢弃：`pulse` / `zoom_bounce`（2026-10-20）
    // -----------------------------------------------------------------------

    /// 一份**含两种退役类型**的老工程 TOML（真实工程的形状：只有 type/time/end/sfx）。
    const LEGACY_RETIRED_TOML: &str = r##"
[[segments]]
start = 0.0
end = 30.0

[[segments.events]]
type = "camera"
time = 1.0
end = 5.0
zoom = "中"

[[segments.events]]
type = "zoom_bounce"
time = 9.9
end = 10.5
sfx = "poka"

[[segments.events]]
type = "pulse"
time = 12.0
end = 12.4

[[segments.events]]
type = "flash"
time = 20.0
end = 20.4
color = "#ffffff"
"##;

    /// ★ 核心判据：**含 `pulse` / `zoom_bounce` 的老文件能打开**，那两条被**丢弃**、并**分类计数**。
    ///
    /// 变异自证：把 `migrate_retired_effects` 里的丢弃分支去掉 ⇒ 这里会解析失败/条数不对 ⇒ 红。
    #[test]
    fn retired_events_are_dropped_and_counted() {
        let toml_value: toml::Value = toml::from_str(LEGACY_RETIRED_TOML).unwrap();
        let json = upgrade_toml_value(&toml_value).unwrap();
        let segs: Vec<crate::project::Segment> =
            serde_json::from_value(json["segments"].clone()).expect("丢弃之后必须读得出来");
        let events = &segs[0].events;
        assert_eq!(events.len(), 2, "两条退役的都要丢掉，其余保留");
        assert!(matches!(events[0], crate::project::Event::Camera { .. }));
        assert!(matches!(events[1], crate::project::Event::Flash { .. }));
        // 落盘形态也不许残留退役类型名
        let serialized = serde_json::to_string(events).unwrap();
        assert!(!serialized.contains("pulse"), "序列化不许残留 pulse：{serialized}");
        assert!(!serialized.contains("zoom_bounce"), "序列化不许残留 zoom_bounce：{serialized}");
    }

    /// **计数**要对：调用方靠它打「已退役 pulse：丢弃 N 条」（用户口径：不许静默丢）。
    #[test]
    fn retired_counts_are_classified() {
        let toml_value: toml::Value = toml::from_str(LEGACY_RETIRED_TOML).unwrap();
        let mut json = serde_json::to_value(&toml_value).unwrap();
        upgrade_event_times(&mut json);
        let counts = migrate_retired_effects(&mut json);
        assert_eq!(counts.pulse, 1);
        assert_eq!(counts.zoom_bounce, 1);
        assert_eq!(counts.total(), 2);
        assert!(!counts.is_empty());
        // 幂等：再跑一次什么都不剩
        let again = migrate_retired_effects(&mut json);
        assert!(again.is_empty(), "第二次必须 0（迁移幂等）");
    }

    /// 别的事件**一个字段都没动**（丢弃不是整份重写）。
    #[test]
    fn dropping_retired_events_leaves_others_untouched() {
        let toml_value: toml::Value = toml::from_str(LEGACY_RETIRED_TOML).unwrap();
        let before = serde_json::to_value(&toml_value).unwrap();
        let after = upgrade_toml_value(&toml_value).unwrap();
        let (b, a) = (before["segments"][0]["events"].clone(), after["segments"][0]["events"].clone());
        assert_eq!(b[0], a[0], "camera 那条逐字段不变");
        assert_eq!(b[3], a[1], "flash 那条逐字段不变（丢掉两条之后它挪到 index 1）");
        assert_eq!(a.as_array().unwrap().len(), 2, "只少了退役的两条");
    }

    /// ★ R4：**不含**退役类型的工程 ⇒ 与「什么都不做」逐字节相同。
    ///
    /// 比较的是升级后 JSON 的**序列化文本**（比 Value 相等更严：键集合/值/嵌套一个都不许动）。
    #[test]
    fn r4_project_without_retired_types_is_byte_identical() {
        let raw = r#"
[[segments]]
start = 0.0
end = 30.0

[[segments.events]]
type = "camera"
time = 1.0
end = 5.0
zoom = "中"

[[segments.events]]
type = "blur"
time = 9.9
end = 10.5
sfx = "poka"

[style.subtitle]
font = "LXGW WenKai"
"#;
        let toml_value: toml::Value = toml::from_str(raw).unwrap();
        let plain = serde_json::to_string(&serde_json::to_value(&toml_value).unwrap()).unwrap();
        let upgraded =
            serde_json::to_string(&upgrade_toml_value(&toml_value).unwrap()).unwrap();
        assert_eq!(plain, upgraded, "不含退役类型的工程必须逐字节不变（R4）");
    }

    /// 老工程**同时**写 `duration` 与退役类型：先算 end、再丢事件，别把别的字段搞坏。
    #[test]
    fn retired_event_with_legacy_duration_is_still_dropped() {
        let raw = r#"
[[segments]]
start = 0.0
end = 30.0

[[segments.events]]
type = "camera"
time = 1.0
duration = 4.0

[[segments.events]]
type = "zoom_bounce"
time = 9.9
duration = 0.6
"#;
        let toml_value: toml::Value = toml::from_str(raw).unwrap();
        let json = upgrade_toml_value(&toml_value).unwrap();
        let segs: Vec<crate::project::Segment> =
            serde_json::from_value(json["segments"].clone()).unwrap();
        assert_eq!(segs[0].events.len(), 1, "退役那条要丢掉");
        match &segs[0].events[0] {
            crate::project::Event::Camera { time, end, .. } => {
                assert_eq!(*time, 1.0);
                assert_eq!(*end, Some(5.0), "duration 要先换算成 end（这半边照旧生效）");
            }
            other => panic!("应当是 Camera：{other:?}"),
        }
    }
}

