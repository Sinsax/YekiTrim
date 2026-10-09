//! polish::render::sound：音效事件的收集、重映射与文件存在性检查。
//!
//! 入口：`SoundEvent`、`collect_sound_events`、`remap_sound_events`、`sound_file_exists`。
//! ⚠️ 消费者是 `cmd_render`（`pipeline.rs`）：底座那条腿不带页面，SFX 事件由这里在
//! Rust 侧直收集、混音/合流阶段混入（"画面归底座、声音归本仓"）。字段与序列化曾与
//! 出片模板的 `__SOUND_DATA` 对齐，改字段仍要两边一起看 —— 见 `crates/yekitrim-polish/src/render/AGENTS.md`
//!
//! ⚠️ **收了之后必须重映射**（`remap_sound_events`）：`collect_sound_events` 给的是
//! **源时间**，而 `mix_sfx` 按**成片时间**摆。EDL 非空时两者不等，
//! 不重映射就是"每个音效都错位、且零报错"。

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

/// 把一批 SFX 事件的**源时间**重映射到**成片时间**（一对多）。
///
/// # 为什么这个函数必须存在（这是 R2：**静默错位**）
///
/// `collect_sound_events` 把 `event.time` **原样透传**，而 `Ffmpeg::mix_sfx` 对
/// "成片时间线长什么样"**一无所知** —— 它只是把每个音效 `adelay` 到 `time` 那一秒。
/// 于是只要成片不是"源时间轴原样"（有 EDL 重排、有屏蔽段、有变速），
/// **每一个音效都错位**，而且：
///
/// - 不会有任何报错（`ffmpeg` 照常成功）；
/// - 报告里也不会有点名（转译器那边只管画面）；
/// - 症状是"爆点那一下的音效晚了两秒"，会被人当成"音效文件选错了"。
///
/// 音效**没有"被裁成两段"这回事**（它是个短促的点事件，不能播一半再续），所以口径是：
/// **起点落在哪个成片段就整体归到那一段**；起点落在成片里不存在的源时刻（被屏蔽掉的地方）
/// ⇒ **整条丢弃**并留一条 warn（丢一条比错位一条好 —— 错位会把后面的观众认知也带偏）。
///
/// ⚠️ **一对多**：重放（同一段源在成片里出现两次）会让同一个音效要**响两次**，
/// 所以返回的是"事件列表"而不是"事件"。只改第一条 = 第二次重放是哑的。
///
/// # 参数
///
/// - `events`：`collect_sound_events` 的产物（源时间）。
/// - `clips`：`ProjectFile::resolve_edl(total_duration, fps)` 的结果（成片时间轴）。
///   **空数组 ⇒ 原样返回**（时长未知 / 老工程没推导出来时不该把音效全丢了）。
/// - `fps`：源素材帧率。映射在**帧号域**做，与 `Edl::resolve_clips` 同一口径
///   （帧域累加是不会漂的，秒域会）。
pub fn remap_sound_events(
    events: &[SoundEvent],
    clips: &[yekitrim_render_core::types::ResolvedClip],
    fps: f64,
) -> Vec<SoundEvent> {
    // 空 EDL ⇒ **原样返回**。调用侧已经在别处判过一次"空 resolve 结果"，
    // 这里再兜一层：这条路上"返回空"等于把成片的音效**全删了**，是灾难性的静默失败。
    if clips.is_empty() || !(fps > 0.0) {
        return events.to_vec();
    }
    let mut out = Vec::with_capacity(events.len());
    for ev in events {
        let mut placed = 0usize;
        for clip in clips {
            // 段在**源时间轴**上的闭开区间与成片起点，都用帧号（`resolve_clips` 已给整数帧）。
            // ⚠️ 段长用 `round((ef - sf) / speed)` —— 与 `resolve_clips` 的帧数口径**逐式相同**
            //    （写成 `(out_end_frame - out_start_frame)` 在 speed=1 时也一样，但变速时会差）。
            let sf = (clip.source_start * fps).round().max(0.0) as u64;
            let ef = (clip.source_end * fps).round().max(0.0) as u64;
            if ef <= sf {
                continue;
            }
            let ev_frame = (ev.time * fps).round().max(0.0) as u64;
            if ev_frame < sf || ev_frame >= ef {
                continue; // 这一条不在本段（或被屏蔽掉）
            }
            // 段内偏移 → 成片帧号。`speed = 1.0` 时逐值等于 `本地帧`。
            let speed = if clip.speed.is_finite() && clip.speed > 0.0 {
                clip.speed
            } else {
                1.0
            };
            let local = ((ev_frame - sf) as f64 / speed).round().max(0.0) as u64;
            let out_frame = clip.out_start_frame + local;
            let mut mapped = ev.clone();
            mapped.time = out_frame as f64 / fps;
            out.push(mapped);
            placed += 1;
        }
        if placed == 0 {
            // **被屏蔽掉的音效**：起点落在成片里不存在的源时刻。
            // ⚠️ 必须留痕 —— 静默丢弃正是本函数存在的理由（R2）。
            tracing::warn!(
                "  ⚠ 音效 {} （源 {:.3}s）落在成片里不存在的时间上（屏蔽区间内），已跳过",
                ev.file,
                ev.time
            );
        }
    }
    // **不乱序**：`mix_sfx` 按 `adelay` 各自定位，顺序不影响结果，
    // 但排一下能让日志与"成片里第几个响"对得上（排查时省一轮）。
    out.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
    out
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

    /// 源时间 → 成片时间：**一条 cue 落在多个段上就响多次**（重放）。
    ///
    /// 这是 R2 的核心判据。老行为是 `collect_sound_events` 原样交出源时间、
    /// `mix_sfx` 瞎子一样按那个数摆 —— EDL 一重排就全体错位且零报错。
    ///
    /// ⚠️ 这一份 EDL 里 `[3,6)` 出现了两次、而整条 `[0,12)` 里**也**含 4.0s，
    /// 所以 4.0s 那一下要响**三次**。判据刻意钉住"命中几次就是几次"，
    /// 不做去重（音效是"这一下要响"，不是"同一件事只播一遍"的语义）。
    #[test]
    fn remap_sound_events_expands_replays() {
        use yekitrim_render_core::types::{Edl, EdlClip};
        // 60fps、12 秒源。EDL：重放 [3,6) 两次 + 整条 [0,12)。
        let edl = Edl {
            clips: vec![
                EdlClip { source_start: 3.0, source_end: 6.0, speed: 1.0, kind: Some("replay".into()), label: None, source_kind: None, asset_id: None, transition_in: None},
                EdlClip { source_start: 0.0, source_end: 12.0, speed: 1.0, kind: None, label: None, source_kind: None, asset_id: None, transition_in: None},
                EdlClip { source_start: 3.0, source_end: 6.0, speed: 1.0, kind: Some("replay".into()), label: None, source_kind: None, asset_id: None, transition_in: None},
            ],
        };
        let clips = edl.resolve_clips(60.0);
        let events = vec![SoundEvent {
            time: 4.0, // **三段都含它** ⇒ 要响三次
            file: "sfx/boom.mp3".into(),
            r#type: "sfx".into(),
            volume: Some(0.5),
            duration: Some(1.0),
        }];
        let out = remap_sound_events(&events, &clips, 60.0);
        // ⚠️ 命中的段数**不是**"去重后的段数"：音效是"这一下要响"，
        // 4.0s 在 [3,6)（重放）、[0,12)（原序）、[3,6)（重放）里各出现一次 ⇒ 三次。
        assert_eq!(out.len(), 3, "三段都含 4.0s，要响三次，实得 {out:?}");
        // 段 0（[3,6)，out_start_frame = 0）：偏移 (4-3)*60 = 60 帧 ⇒ 1.0s
        assert!((out[0].time - 1.0).abs() < 1e-9, "实得 {}", out[0].time);
        // 段 1（[0,12)，out_start_frame = 180 = 3.0s）：偏移 4*60 = 240 ⇒ 420 帧 ⇒ 7.0s
        assert!((out[1].time - 7.0).abs() < 1e-9, "实得 {}", out[1].time);
        // 段 2（[3,6)，out_start_frame = 180+720 = 900）：偏移 60 ⇒ 960 帧 ⇒ 16.0s
        assert!((out[2].time - 16.0).abs() < 1e-9, "实得 {}", out[2].time);
    }

    /// **被屏蔽掉的源时刻上的音效必须整条丢弃**（而不是错位保留）。
    ///
    /// 音效是点事件，不能"播一半再续" —— 起点落在成片里不存在的时刻上时，
    /// 留着它只会把这一下挪到别的画面上去。丢弃要留 warn（那是它唯一的存在证明）。
    #[test]
    fn remap_sound_events_drops_muted_positions() {
        use yekitrim_render_core::types::{Edl, EdlClip};
        // 源 12 秒，屏蔽掉 [7,9) ⇒ 成片是 [0,7) + [9,12)
        let edl = Edl {
            clips: vec![
                EdlClip { source_start: 0.0, source_end: 7.0, speed: 1.0, kind: None, label: None, source_kind: None, asset_id: None, transition_in: None},
                EdlClip { source_start: 9.0, source_end: 12.0, speed: 1.0, kind: None, label: None, source_kind: None, asset_id: None, transition_in: None},
            ],
        };
        let clips = edl.resolve_clips(60.0);
        let events = vec![
            SoundEvent { time: 1.0, file: "sfx/keep.mp3".into(), r#type: "sfx".into(), volume: None, duration: None },
            SoundEvent { time: 8.0, file: "sfx/gone.mp3".into(), r#type: "sfx".into(), volume: None, duration: None },
        ];
        let out = remap_sound_events(&events, &clips, 60.0);
        assert_eq!(out.len(), 1, "被屏蔽那条要丢掉，实得 {out:?}");
        assert_eq!(out[0].file, "sfx/keep.mp3");
        assert!((out[0].time - 1.0).abs() < 1e-9, "第一段起点是 0：实得 {}", out[0].time);
    }

    /// **空 EDL ⇒ 原样返回**，绝不返回空表。
    ///
    /// `resolve_edl` 在 `total_duration <= 0` 时给空数组。这时"按映射丢掉一切"
    /// 会让成片完全没有音效，而报告里一个字都没有。宁可原样（等于老行为）。
    #[test]
    fn remap_sound_events_empty_clips_passthrough() {
        let events = vec![SoundEvent {
            time: 2.5, file: "sfx/boom.mp3".into(), r#type: "sfx".into(), volume: None, duration: None,
        }];
        let out = remap_sound_events(&events, &[], 60.0);
        assert_eq!(out.len(), 1);
        assert!((out[0].time - 2.5).abs() < 1e-9);
    }

    /// 上一段保留后落到下一段：声字号 shifted by the dropped gap，**不是** `time - 0`。
    #[test]
    fn remap_sound_events_shifts_across_gap() {
        use yekitrim_render_core::types::{Edl, EdlClip};
        let edl = Edl {
            clips: vec![
                EdlClip { source_start: 0.0, source_end: 3.0, speed: 1.0, kind: None, label: None, source_kind: None, asset_id: None, transition_in: None},
                EdlClip { source_start: 6.0, source_end: 9.0, speed: 1.0, kind: None, label: None, source_kind: None, asset_id: None, transition_in: None},
            ],
        };
        let clips = edl.resolve_clips(60.0);
        // 源 7.0s 在第二段里，段内偏移 1.0s；第二段成片起点 = 3.0s ⇒ 成片 4.0s
        let events = vec![SoundEvent {
            time: 7.0, file: "sfx/boom.mp3".into(), r#type: "sfx".into(), volume: None, duration: None,
        }];
        let out = remap_sound_events(&events, &clips, 60.0);
        assert_eq!(out.len(), 1);
        assert!((out[0].time - 4.0).abs() < 1e-9, "实得 {}", out[0].time);
    }
}
