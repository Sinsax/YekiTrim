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
    let migrated = migrate_retired_effects(&mut json);
    if migrated > 0 {
        tracing::info!(
            "已把 {migrated} 条已退役的 `zoom_bounce` 事件读时迁移成 `pulse`\
             （只改内存里的值；下一次保存才落盘，本次不动磁盘上的 polish.toml）"
        );
    }
    Ok(json)
}

// ---------------------------------------------------------------------------
// 已退役事件类型的读时迁移
// ---------------------------------------------------------------------------

/// 退役的瞬时运镜类型 `zoom_bounce`。
///
/// 它曾与 `pulse` **并存**，而两者在出片模板里是**同一个槽**：
/// `finalZoom = Math.max(getZoomBounce(...), getPulse(...))`，都乘在 `curZoom` 上
/// （见 2026-10-01 之前 `templates/index.html` 的 `getZoomBounce`）。
const RETIRED_ZOOM_BOUNCE: &str = "zoom_bounce";

/// 迁移目标：现役里语义**最近**的那一个（同为「瞬时运镜放大」，乘在运镜 zoom 上）。
const MIGRATED_ZOOM_BOUNCE_TO: &str = "pulse";

/// **已退役事件类型 `zoom_bounce` → `pulse` 的读时迁移**（返回迁移条数）。
///
/// # 为什么是 `pulse`（而不是继续拒载）
///
/// 两者是**同一条通道**上的同一件事，只是曲线不同：
///
/// | | `zoom_bounce`（已退役） | `pulse`（现役） |
/// |---|---|---|
/// | 乘在哪 | `curZoom`（运镜 zoom） | `curZoom`（运镜 zoom） |
/// | 窗口 | 0.30s | 0.48s |
/// | 曲线 | `dt < 0.08 → ×1.10`，之后回 1.0 | `dt < 0.12 → ×1.06`；`dt < 0.28 → ×0.97`；之后 1.0 |
/// | `time` / `end` / `sfx` | 同一组字段 | 同一组字段（另有 `anchor` / `sfx_volume`） |
///
/// 退役前那一版模板里两者是 `Math.max(zbZoom, plZoom)` —— **同一个缩放槽的两个来源**，
/// 所以把 `zoom_bounce` 折进 `pulse` 不换通道、不挪层级。
///
/// # 映射时**丢了什么**（如实写，别以为是无损迁移）
///
/// 1. **峰值 1.10 → 1.06**：迁移后那一下「弹」会**弱一点**（1.10 / 1.06 ≈ 1.04 倍差距）。
/// 2. **波形不同**：老的是一段 80ms 平顶再直接回 1.0；新的是 120ms ×1.06 → 160ms **×0.97（下沉）**
///    → 1.0。**这个下沉是老的没有的**，总窗口也从 0.30s 拉到 0.48s。
/// 3. `end` 两条曲线都**不读**（都按 `time` + 固定窗口算），所以用户写的 `end`
///    （实测 `end - time` = 0.4s / 0.6s）在迁移前后同样只起「时长登记」作用 —— 这一项没损失。
///
/// # 为什么只改 `type`、其余键原样带过去
///
/// `Pulse` 的字段是 `zoom_bounce` 的**超集**（`time` / `end` / `sfx` 一一对应，
/// 另有 `anchor` / `sfx_volume`），而 `Event` 是 `#[serde(tag = "type")]` 且**没有**
/// `deny_unknown_fields` ⇒ 老工程里若还留着别的键，反序列化会**忽略**而不是报错。
/// 所以**不删键**：删了就成了「迁移时替用户丢数据」，而这是读时迁移最不该做的事。
///
/// # 范围
///
/// 只迁移 `zoom_bounce` 一条。`split` / `stutter` / `color_shift` **没有现役等价物**
/// （它们不在缩放通道上），仍由 `validate.rs::retired_effect_message` 拒载并给人话 ——
/// 别把这条迁移扩成「什么都往最近邻塞」。
fn migrate_retired_effects(value: &mut Value) -> usize {
    fn walk(value: &mut Value, migrated: &mut usize) {
        match value {
            Value::Object(map) => {
                // 判据与 `upgrade_event_times` 同源：**有 `type` 且是字符串**才算事件。
                // 只认 `type`，别顺带改 `kind`（`[[edl.clips]]` 与封面特效用的也是 kind）。
                let hit = matches!(
                    map.get("type"),
                    Some(Value::String(kind)) if kind == RETIRED_ZOOM_BOUNCE
                );
                if hit {
                    map.insert(
                        "type".to_string(),
                        Value::String(MIGRATED_ZOOM_BOUNCE_TO.to_string()),
                    );
                    *migrated += 1;
                }
                for (_, v) in map.iter_mut() {
                    if v.is_array() || v.is_object() {
                        walk(v, migrated);
                    }
                }
            }
            Value::Array(items) => {
                for item in items.iter_mut() {
                    walk(item, migrated);
                }
            }
            _ => {}
        }
    }
    let mut migrated = 0usize;
    walk(value, &mut migrated);
    migrated
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
    // `zoom_bounce` → `pulse` 的读时迁移（2026-10-19）
    // -----------------------------------------------------------------------

    /// 一份**含 `zoom_bounce`** 的工程 TOML：老工程里实际长这样（取自用户工程的实测形状：
    /// 只有 `type` / `time` / `end` / `sfx` 四个键）。
    const LEGACY_ZOOM_BOUNCE_TOML: &str = r##"
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
type = "flash"
time = 20.0
end = 20.4
color = "#ffffff"
"##;

    /// ★ 核心判据：含 `zoom_bounce` 的工程 ⇒ 解析出来是 `Pulse`，且**时间与音效都在**。
    #[test]
    fn zoom_bounce_migrates_to_pulse() {
        let toml_value: toml::Value = toml::from_str(LEGACY_ZOOM_BOUNCE_TOML).unwrap();
        let json = upgrade_toml_value(&toml_value).unwrap();
        let segs: Vec<crate::project::Segment> =
            serde_json::from_value(json["segments"].clone()).expect("迁移之后必须读得出来");
        let events = &segs[0].events;
        assert_eq!(events.len(), 3, "迁移不许增删事件");
        assert!(
            matches!(
                &events[1],
                crate::project::Event::Pulse { .. }
            ),
            "zoom_bounce 必须变成 Pulse，实际：{:?}",
            events[1]
        );
        // 事件级字段逐个对上（这是「内容不丢」的判据，不只是类型名对不对）
        let (time, end, sfx) = match &events[1] {
            crate::project::Event::Pulse { time, end, sfx, .. } => (*time, *end, sfx.clone()),
            other => panic!("应当是 Pulse：{other:?}"),
        };
        assert_eq!(time, 9.9);
        assert_eq!(end, Some(10.5));
        assert_eq!(sfx.as_deref(), Some("poka"));
        // 落盘形态也必须是 `pulse`（不能只在内存里对）
        let serialized = serde_json::to_string(&events[1]).unwrap();
        assert!(serialized.contains("\"type\":\"pulse\""), "序列化必须是 pulse：{serialized}");
        assert!(!serialized.contains("zoom_bounce"), "序列化不许残留退役类型：{serialized}");
    }

    /// 同一份工程里**别的事件一个字段都没动**（迁移不是整份重写）。
    #[test]
    fn migration_leaves_other_events_untouched() {
        let toml_value: toml::Value = toml::from_str(LEGACY_ZOOM_BOUNCE_TOML).unwrap();
        // 升级前的原样（`upgrade_toml_value` 之前的形状）
        let before = serde_json::to_value(&toml_value).unwrap();
        let after = upgrade_toml_value(&toml_value).unwrap();
        let events_of = |v: &Value| v["segments"][0]["events"].clone();
        let (b, a) = (events_of(&before), events_of(&after));
        for i in [0usize, 2] {
            assert_eq!(
                b[i], a[i],
                "第 {i} 条事件（非 zoom_bounce）必须逐字段不变"
            );
        }
        // 反向：中间那条**必须**变了（否则上面两条断言可能空转）
        assert_ne!(b[1], a[1], "zoom_bounce 那一条必须被改写");
        assert_eq!(a[1]["type"], Value::String("pulse".into()));
        assert_eq!(b[1]["type"], Value::String("zoom_bounce".into()));
    }

    /// ★ R4：**不含** `zoom_bounce` 的工程 ⇒ 与「什么都不做」逐字节相同。
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
type = "pulse"
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

    /// 老工程**同时**写 `duration` 与 `zoom_bounce`：两处升级必须叠加（先算 end、再改 type）。
    #[test]
    fn zoom_bounce_with_legacy_duration_gets_both_upgrades() {
        let raw = r#"
[[segments]]
start = 0.0
end = 30.0

[[segments.events]]
type = "zoom_bounce"
time = 9.9
duration = 0.6
"#;
        let toml_value: toml::Value = toml::from_str(raw).unwrap();
        let json = upgrade_toml_value(&toml_value).unwrap();
        let segs: Vec<crate::project::Segment> =
            serde_json::from_value(json["segments"].clone()).unwrap();
        match &segs[0].events[0] {
            crate::project::Event::Pulse { time, end, .. } => {
                assert_eq!(*time, 9.9);
                assert_eq!(*end, Some(10.5), "duration 要先换算成 end 再迁移类型");
            }
            other => panic!("应当是 Pulse：{other:?}"),
        }
    }
}
