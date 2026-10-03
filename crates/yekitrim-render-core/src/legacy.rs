//! 老配置的字段升级：事件的「开始 + 持续时间」→「开始 + 结束」。
//!
//! 事件的时间表达在 2026-09 统一成 `time`（开始）+ `end`（结束，绝对秒），
//! 老配置 / 老 checkpoint / LLM 偶发的输出里写的是 `time` + `duration`（相对时长）。
//! 升级策略是**在反序列化之前改 JSON 值**，而不是给每个变体配一套 serde 兼容字段：
//! 一个函数管住全部入口，`Event` 那边保持干净；落盘时只会写 `end`
//! （下一次保存即完成迁移，渲染结果 1:1 不变）。
//!
//! 入口：`yekitrim-polish` 的 polish.toml 加载与 `PUT /api/polish/project`、LLM 事件生成
//! （含老 checkpoint）。—— 原来的第四处「`yekitrim-wasm` 的预览配置」随「作废 wgpu」一起删除了。

use serde_json::{Map, Value};

/// 事件对象里「时长」的旧字段 → 新字段的换算：`end = time + duration`。
/// 已经写了 `end` 的一律不动（新字段优先），老字段随后删掉。
fn upgrade_event(event: &mut Map<String, Value>) {
    let Some(dur) = event.remove("duration") else {
        return;
    };
    if event.contains_key("end") {
        return;
    }
    let time = event.get("time").and_then(Value::as_f64).unwrap_or(0.0);
    let Some(d) = dur.as_f64() else {
        return;
    };
    event.insert("end".to_string(), Value::from(time + d));
}

/// 递归升级一个值，兼容几种形状：
/// - `{"segments": [{"events": [...]}]}`（完整配置 JSON）
/// - `[{...segment...}]`（segments 数组）
/// - `[{...event...}]`（事件数组——LLM 输出与 checkpoint）
///
/// 判据是「对象里有 `type` 且有 `time`/`duration`」= 事件；其余对象继续往里走
/// （段里找 `events`、checkpoint 里找窗口数据，都不需要显式列举键名）。
pub fn upgrade_event_times(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.contains_key("type") && (map.contains_key("time") || map.contains_key("duration")) {
                upgrade_event(map);
                return;
            }
            for (_, v) in map.iter_mut() {
                if v.is_array() || v.is_object() {
                    upgrade_event_times(v);
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                upgrade_event_times(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Event, Segment};

    #[test]
    fn legacy_duration_becomes_end() {
        let mut v = serde_json::json!({
            "segments": [{
                "start": 0.0,
                "end": 30.0,
                "events": [
                    { "type": "sticker", "time": 10.0, "duration": 2.0, "keyword": "ysyk", "x": "auto", "y": "auto" },
                    { "type": "camera", "time": 0.0, "duration": 10.0, "zoom": "中" },
                    { "type": "blur", "time": 5.0, "duration": 0.4 },
                    { "type": "sfx", "time": 3.0, "name": "pop-notice", "duration": 1.0 },
                ],
            }],
        });
        upgrade_event_times(&mut v);
        let segs: Vec<Segment> = serde_json::from_value(v["segments"].clone()).expect("升级后应可解析");
        let ends: Vec<Option<f64>> = segs[0].events.iter().map(|e| e.end()).collect();
        assert_eq!(ends, vec![Some(12.0), Some(10.0), Some(5.4), Some(4.0)]);
        assert!(matches!(&segs[0].events[0], Event::Sticker { keyword, .. } if keyword == "ysyk"));
    }

    #[test]
    fn new_end_wins_over_legacy_duration() {
        let mut v = serde_json::json!([
            { "type": "sticker", "time": 10.0, "end": 20.0, "duration": 2.0, "keyword": "k", "x": "auto", "y": "auto" },
        ]);
        upgrade_event_times(&mut v);
        let evs: Vec<Event> = serde_json::from_value(v).unwrap();
        assert_eq!(evs[0].end(), Some(20.0));
    }

    #[test]
    fn missing_fields_left_alone() {
        // 没写 duration 的（已是 end / 什么都没写）原样通过，后者由 process 补默认值
        let mut v = serde_json::json!({
            "segments": [{
                "start": 0.0,
                "end": 30.0,
                "events": [
                    { "type": "camera", "time": 0.0, "end": 5.0, "zoom": "中" },
                    { "type": "camera", "time": 6.0, "zoom": "中" },
                ],
            }],
        });
        upgrade_event_times(&mut v);
        let segs: Vec<Segment> = serde_json::from_value(v["segments"].clone()).unwrap();
        assert_eq!(segs[0].events[0].end(), Some(5.0));
        assert_eq!(segs[0].events[1].end(), None);
    }

    #[test]
    fn nested_window_array_is_upgraded() {
        // LLM checkpoint 的形状：窗口里包着事件数组
        let mut v = serde_json::json!({
            "windows": [{
                "items": [],
                "data": [
                    { "type": "shake", "time": 1.0, "duration": 0.3, "intensity": 10 },
                ],
            }],
        });
        upgrade_event_times(&mut v);
        let ev: Event = serde_json::from_value(v["windows"][0]["data"][0].clone()).unwrap();
        assert_eq!(ev.end(), Some(1.3));
    }
}
