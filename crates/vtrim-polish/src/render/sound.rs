//! polish::render::sound：音效事件的收集与文件存在性检查（喂给出片模板的 `__SOUND_DATA`）。
//!
//! 入口：`SoundEvent`、`collect_sound_events`、`sound_file_exists`。
//! 边界：`SoundEvent` 的字段与序列化必须与 `templates/index.html` 的 `__SOUND_DATA` 对齐，改一边要改另一边 —— 见 `crates/vtrim-polish/src/render/AGENTS.md`

use std::path::Path;

use crate::project::{Event, ProjectFile};

/// 与 `index.html` 中 `__SOUND_DATA` 对齐的 SFX 事件。
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SoundEvent {
    pub time: f64,
    pub file: String,
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(default)]
    pub volume: Option<f64>,
    #[serde(default)]
    pub duration: Option<f64>,
}

/// 后端无关地收集整片 SFX 事件，替代 Chrome 页面里的 `window.__SOUND_DATA`。
///
/// 规则与 JS 一致：
/// - `Sfx` 事件直接输出；
/// - 带 `sfx` 引用的事件（sticker/特效）也输出；
/// - 事件跨度不足 1s 时按 1s 兜底。
pub fn collect_sound_events(config: &ProjectFile) -> Vec<SoundEvent> {
    let mut out = Vec::new();
    for seg in &config.segments {
        for ev in &seg.events {
            let (time, file, volume, duration) = match ev {
                Event::Sfx {
                    time,
                    name,
                    volume,
                    end,
                } if !name.is_empty() => {
                    let file = format!("sfx/{}.mp3", name);
                    let dur = end.map(|e| e - *time).unwrap_or(1.0).max(1.0);
                    (*time, file, volume, dur)
                }
                Event::Sfx { .. } => continue,
                Event::Sticker {
                    time,
                    sfx: Some(name),
                    sfx_volume,
                    end,
                    ..
                }
                | Event::Shake {
                    time,
                    sfx: Some(name),
                    sfx_volume,
                    end,
                    ..
                }
                | Event::Flash {
                    time,
                    sfx: Some(name),
                    sfx_volume,
                    end,
                    ..
                }
                | Event::Blur {
                    time,
                    sfx: Some(name),
                    sfx_volume,
                    end,
                    ..
                }
                | Event::ZoomBounce {
                    time,
                    sfx: Some(name),
                    sfx_volume,
                    end,
                    ..
                }
                | Event::Vignette {
                    time,
                    sfx: Some(name),
                    sfx_volume,
                    end,
                    ..
                }
                | Event::HueShift {
                    time,
                    sfx: Some(name),
                    sfx_volume,
                    end,
                    ..
                }
                | Event::Stutter {
                    time,
                    sfx: Some(name),
                    sfx_volume,
                    end,
                    ..
                }
                | Event::Split {
                    time,
                    sfx: Some(name),
                    sfx_volume,
                    end,
                    ..
                }
                | Event::Noise {
                    time,
                    sfx: Some(name),
                    sfx_volume,
                    end,
                    ..
                }
                | Event::Pulse {
                    time,
                    sfx: Some(name),
                    sfx_volume,
                    end,
                    ..
                }
                | Event::ColorShift {
                    time,
                    sfx: Some(name),
                    sfx_volume,
                    end,
                    ..
                }
                | Event::Overlay {
                    time,
                    sfx: Some(name),
                    sfx_volume,
                    end,
                    ..
                } => {
                    let file = format!("sfx/{}.mp3", name);
                    let dur = end.map(|e| e - *time).unwrap_or(1.0).max(1.0);
                    (*time, file, sfx_volume, dur)
                }
                _ => continue,
            };

            out.push(SoundEvent {
                time,
                file,
                r#type: type_name(ev),
                volume: Some(volume.unwrap_or(0.1)),
                duration: Some(duration),
            });
        }
    }
    out
}

/// 与 `fxSfx`/`scheduleSound` 中 `type` 相同的字符串（用于日志/兼容）。
fn type_name(ev: &Event) -> String {
    match ev {
        Event::Sfx { name, .. } => name.clone(),
        Event::Sticker { .. } => "sticker".into(),
        Event::Shake { .. } => "shake".into(),
        Event::Flash { .. } => "flash".into(),
        Event::Blur { .. } => "blur".into(),
        Event::ZoomBounce { .. } => "zoom_bounce".into(),
        Event::Vignette { .. } => "vignette".into(),
        Event::HueShift { .. } => "hue_shift".into(),
        Event::Stutter { .. } => "stutter".into(),
        Event::Split { .. } => "split".into(),
        Event::Noise { .. } => "noise".into(),
        Event::Pulse { .. } => "pulse".into(),
        Event::ColorShift { .. } => "color_shift".into(),
        Event::Overlay { .. } => "overlay".into(),
        Event::Camera { .. } => "camera".into(),
    }
}

/// 文件是否存在于项目目录下（渲染时过滤缺失文件）。
pub fn sound_file_exists(project_path: &Path, event: &SoundEvent) -> bool {
    let file = event.file.replace("sfx/", "");
    project_path.join("sfx").join(file).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config() -> ProjectFile {
        ProjectFile::example()
    }

    #[test]
    fn sfx_event_is_collected() {
        let mut config = base_config();
        config.segments[0].events = vec![Event::Sfx {
            time: 1.0,
            name: "pop-notice".into(),
            volume: Some(0.2),
            end: Some(2.5),
        }];
        let events = collect_sound_events(&config);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].file, "sfx/pop-notice.mp3");
        assert_eq!(events[0].volume, Some(0.2));
        assert_eq!(events[0].duration, Some(1.5));
    }

    #[test]
    fn fx_with_sfx_is_collected() {
        let mut config = base_config();
        config.segments[0].events = vec![Event::Flash {
            time: 2.0,
            color: Some("#fff".into()),
            end: Some(2.4),
            anchor: None,
            sfx: Some("boom".into()),
            sfx_volume: Some(0.5),
        }];
        let events = collect_sound_events(&config);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].file, "sfx/boom.mp3");
        assert_eq!(events[0].r#type, "flash");
        assert_eq!(events[0].volume, Some(0.5));
        // JS 中 `Math.max(transientDur, 1.0)` 最少 1s
        assert!(events[0].duration.unwrap() >= 1.0);
    }
}
