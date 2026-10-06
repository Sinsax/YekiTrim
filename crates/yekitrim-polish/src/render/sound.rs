//! polish::render::sound：音效事件的收集与文件存在性检查。
//!
//! 入口：`SoundEvent`、`collect_sound_events`、`sound_file_exists`。
//! ⚠️ 消费者是 `cmd_render`（`pipeline.rs`）：底座那条腿不带页面，SFX 事件由这里在
//! Rust 侧直收集、混音/合流阶段混入（"画面归底座、声音归本仓"）。字段与序列化曾与
//! 出片模板的 `__SOUND_DATA` 对齐，改字段仍要两边一起看 —— 见 `crates/yekitrim-polish/src/render/AGENTS.md`

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
/// - 事件跨度不足 1s 时按 1s 兜底；
/// - ⚠️ **音效轨的眼睛关着 → 一条都不收**（`[stage.lanes] sfx`；缺字段时 = 竖画布下默认关）。
pub fn collect_sound_events(config: &ProjectFile) -> Vec<SoundEvent> {
    // **音效轨的显示开关**（2026-10-06：从 `[stage] portrait_sfx` 硬门改成轨道眼睛）。
    // 语义 = **按整类事件屏蔽**：只影响混音，`polish.toml` 里的事件**一个字都不动**
    // （与「屏蔽只做标记」同一条纪律：界面把眼睛点亮即完全恢复）。
    // ⚠️ 判据只有一处（`ProjectFile::lane_enabled`）：缺字段时音效**只跟画布方向**
    //    （竖画布默认关、横画布默认开），与素材方向无关 —— 别在这里另写一套条件。
    if !config.lane_enabled(crate::project::Lane::Sfx, None) {
        return Vec::new();
    }
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
        Event::Vignette { .. } => "vignette".into(),
        Event::HueShift { .. } => "hue_shift".into(),
        Event::Noise { .. } => "noise".into(),
        Event::Pulse { .. } => "pulse".into(),
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

    /// **音效轨的眼睛**（2026-10-06）：缺字段时竖画布默认关 ⇒ 一条都不收；
    /// 眼睛点亮（`[stage.lanes] sfx = true`）或显式关掉都照办；横画布不受画布方向影响。
    #[test]
    fn sfx_lane_switch_gates_audio() {
        let mut config = base_config();
        config.segments[0].events = vec![Event::Sfx {
            time: 1.0,
            name: "pop-notice".into(),
            volume: None,
            end: Some(2.0),
        }];
        assert!(config.stage.lanes.is_auto(), "缺省 = 自动");
        assert_eq!(collect_sound_events(&config).len(), 1, "横画布默认开");
        config.canvas.orientation = "portrait".into();
        assert!(collect_sound_events(&config).is_empty(), "竖画布缺字段 = 默认关");
        config.stage.lanes.sfx = Some(true);
        assert_eq!(collect_sound_events(&config).len(), 1, "眼睛点亮即恢复");
        config.canvas.orientation = "landscape".into();
        config.stage.lanes.sfx = Some(false);
        assert!(collect_sound_events(&config).is_empty(), "显式关 ⇒ 横画布也不出声");
    }
}
