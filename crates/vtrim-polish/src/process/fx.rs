use crate::project::{Event, Segment};

/// 瞬时特效的 duration 上限（秒）。
/// 原则：瞬时特效只能“爆发一下”，不允许长时间驻留。
pub fn transient_duration_cap(event_type: &str) -> f64 {
    match event_type {
        "shake" => 0.5,
        "flash" => 0.3,
        "blur" => 0.4,
        "zoom_bounce" => 0.6,
        "stutter" => 0.35,
        "split" => 0.35,
        "noise" => 0.4,
        "pulse" => 0.5,
        "hue_shift" => 0.5,
        "color_shift" => 0.5,
        "vignette" => 0.6,
        _ => f64::INFINITY,
    }
}

/// 瞬时特效的默认 duration（LLM 未写时使用）。
pub fn transient_default_duration(event_type: &str) -> f64 {
    match event_type {
        "shake" => 0.3,
        "flash" => 0.25,
        "blur" => 0.3,
        "zoom_bounce" => 0.4,
        "stutter" => 0.25,
        "split" => 0.25,
        "noise" => 0.3,
        "pulse" => 0.4,
        "hue_shift" => 0.35,
        "color_shift" => 0.35,
        "vignette" => 0.4,
        _ => 0.5,
    }
}

/// 事件对应的字符串类型名。
fn event_type_name(event: &Event) -> &'static str {
    match event {
        Event::Shake { .. } => "shake",
        Event::Flash { .. } => "flash",
        Event::Blur { .. } => "blur",
        Event::ZoomBounce { .. } => "zoom_bounce",
        Event::Stutter { .. } => "stutter",
        Event::Split { .. } => "split",
        Event::Noise { .. } => "noise",
        Event::Pulse { .. } => "pulse",
        Event::HueShift { .. } => "hue_shift",
        Event::ColorShift { .. } => "color_shift",
        Event::Vignette { .. } => "vignette",
        _ => "",
    }
}

/// 归一化所有瞬时特效的 duration：
/// - 缺省填默认值
/// - 超上限 clamp 到上限
/// - 低于 0.1 提到 0.1
pub fn normalize_fx_durations(segments: &mut [Segment]) {
    for seg in segments {
        for event in &mut seg.events {
            normalize_event_duration(event);
        }
    }
}

fn normalize_event_duration(event: &mut Event) {
    let type_name = event_type_name(event);
    if type_name.is_empty() {
        return;
    }
    let cap = transient_duration_cap(type_name);
    let default = transient_default_duration(type_name);

    match event {
        Event::Shake { duration, .. }
        | Event::Flash { duration, .. }
        | Event::Blur { duration, .. }
        | Event::ZoomBounce { duration, .. }
        | Event::Stutter { duration, .. }
        | Event::Split { duration, .. }
        | Event::Noise { duration, .. }
        | Event::Pulse { duration, .. }
        | Event::HueShift { duration, .. }
        | Event::ColorShift { duration, .. }
        | Event::Vignette { duration, .. } => {
            let d = duration.unwrap_or(default).clamp(0.1, cap);
            *duration = Some(d);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blur(duration: Option<f64>) -> Event {
        Event::Blur {
            time: 0.0,
            duration,
            anchor: None,
            sfx: None,
            sfx_volume: None,
        }
    }

    #[test]
    fn blur_is_capped_to_transient() {
        let mut segs = vec![Segment {
            start: 0.0,
            end: 10.0,
            fx: "high".into(),
            note: String::new(),
            events: vec![blur(Some(2.5))],
            extra: Default::default(),
        }];
        normalize_fx_durations(&mut segs);
        let Event::Blur { duration, .. } = &segs[0].events[0] else {
            panic!("expected blur");
        };
        assert_eq!(duration.unwrap(), 0.4);
    }

    #[test]
    fn blur_default_is_applied() {
        let mut segs = vec![Segment {
            start: 0.0,
            end: 10.0,
            fx: "high".into(),
            note: String::new(),
            events: vec![blur(None)],
            extra: Default::default(),
        }];
        normalize_fx_durations(&mut segs);
        let Event::Blur { duration, .. } = &segs[0].events[0] else {
            panic!("expected blur");
        };
        assert_eq!(duration.unwrap(), 0.3);
    }

    #[test]
    fn sticker_and_camera_untouched() {
        let mut segs = vec![Segment {
            start: 0.0,
            end: 10.0,
            fx: "high".into(),
            note: String::new(),
            events: vec![
                Event::Sticker {
                    time: 1.0,
                    duration: 5.0,
                    keyword: "laugh".into(),
                    sticker_tag: None,
                    name: None,
                    source: None,
                    x: "auto".into(),
                    y: "auto".into(),
                    sfx: None,
                    sfx_volume: None,
                },
                Event::Camera {
                    time: 0.0,
                    duration: 10.0,
                    zoom: "中".into(),
                    origin: "50% 50%".into(),
                },
            ],
            extra: Default::default(),
        }];
        normalize_fx_durations(&mut segs);
        match &segs[0].events[0] {
            Event::Sticker { duration, .. } => assert_eq!(*duration, 5.0),
            _ => panic!("expected sticker"),
        }
    }
}
