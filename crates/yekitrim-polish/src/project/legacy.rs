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
    Ok(json)
}

/// 把**已经下线的配置键**在读的时候吞掉（2026-10-01，Phase ③）。
///
/// ⚠️ **为什么不能直接删字段**：`SubtitleStyle` 上有 `#[serde(deny_unknown_fields)]`
/// （`yekitrim-render-core/src/types.rs`）—— 老工程里留着 `entrance = "cycle-5"` 的话，
/// 删掉字段就等于让那些工程**直接读不出来**。所以先在这里吞掉，schema 才敢删。
///
/// ⚠️ **为什么该下线**：`entrance` 是 vtedit 时代的"字幕入场动画"，默认值就是 `"cycle-5"`，
/// 而 `polish init` 的模板还替**每个新工程**写上 `entrance = "fade-up"` —— 但全仓**没有任何读者**：
/// 本仓的 canvas 出片路径（`templates/index.html`）与参照都没有这类动画。留着的代价是
/// "让人以为能配"（交接文档的 E6 条目点过这件事）。要真做就得**发明**一套动画规格 —— 没有参照，
/// 所以选择"从契约里移除"。
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
    /// 会让**所有老工程直接读不出来**（这正是不能"直接删字段"的原因）。
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
}
