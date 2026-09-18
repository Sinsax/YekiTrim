//! 老工程的字段升级：事件的「开始 + 持续时间」→「开始 + 结束」。
//!
//! 核心函数在渲染核心里（`vtrim_render_core::legacy::upgrade_event_times`，与 `Event`
//! 类型同源），这里补一个 TOML 入口——`polish.toml` 是先解析成 `toml::Value` 再升级、
//! 最后走 serde 进 `ProjectFile` 的（见 `validate.rs::load_config_from`）。

use serde_json::Value;

pub use vtrim_render_core::legacy::upgrade_event_times;

/// TOML 值进、JSON 值出：polish.toml 走这条路（`toml::Value` 实现了 Serialize）。
pub fn upgrade_toml_value(value: &toml::Value) -> anyhow::Result<Value> {
    let mut json = serde_json::to_value(value)?;
    upgrade_event_times(&mut json);
    Ok(json)
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
}
