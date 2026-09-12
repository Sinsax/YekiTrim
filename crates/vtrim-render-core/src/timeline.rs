//! Canvas 批量渲染器 `renderFrame` 的时间线求值翻译。
//!
//! 这些是纯数学函数，输入项目数据、输出某一帧的视觉状态。
//! 当前先落地与 JS 公式逐条对应的核心函数，绘制层 P2/P3/P4 接入。

use crate::types::{Event, Segment};

/// 与 JS `getActiveCamera` 对齐的相机状态。
#[derive(Debug, Clone, PartialEq)]
pub struct CameraState {
    pub zoom: f64,
    pub ox: f64,
    pub oy: f64,
}

/// 与 JS `getShake` 对齐的震动位移。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShakeState {
    pub x: f64,
    pub y: f64,
}

/// 与 JS `getFlash` 对齐的闪白。
#[derive(Debug, Clone, PartialEq)]
pub struct FlashState {
    pub opacity: f64,
    pub color: String,
}

/// 字幕片段。
#[derive(Debug, Clone, PartialEq)]
pub struct CaptionPart {
    pub text: String,
    pub hl: bool,
}

/// 与 JS `getActiveSubs` 对齐的字幕状态。
#[derive(Debug, Clone, PartialEq)]
pub struct CaptionState {
    pub parts: Vec<CaptionPart>,
    pub opacity: f64,
    pub y_offset: f64,
}

/// 与 JS `getActiveDms` 对齐的弹幕状态。
#[derive(Debug, Clone, PartialEq)]
pub struct DanmakuState {
    pub text: String,
    pub opacity: f64,
    pub x: f64,
    pub y: f64,
}

/// 与 JS `getActiveSticker` 对齐的贴纸状态（图片资源由外层持有）。
#[derive(Debug, Clone, PartialEq)]
pub struct StickerState {
    /// 贴纸资源名（assets/stickers/ 下的文件 stem / emoji 码点）。
    pub name: String,
    /// 事件开始时间（GIF 帧相位按事件相对时间计算，与 JS `(t-et)*1000` 一致）。
    pub et: f64,
    pub index: usize,
    pub opacity: f64,
    pub scale: f64,
    pub rot: f64,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// 单帧所有视觉状态。
#[derive(Debug, Clone, PartialEq)]
pub struct FrameState {
    pub camera: CameraState,
    pub shake: ShakeState,
    pub flash: FlashState,
    pub blur: f64,
    pub zoom_bounce: f64,
    pub pulse: f64,
    pub hue_shift: Option<FlashState>,
    pub stutter: Option<(f64, f64)>,
    pub split: Option<SplitState>,
    pub noise: f64,
    pub color_shift: String,
    pub overlay: Option<OverlayState>,
    pub vignette: VignetteState,
    pub intro_black: f64,
    pub captions: Vec<CaptionState>,
    pub danmaku: Vec<DanmakuState>,
    pub stickers: Vec<StickerState>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SplitState {
    pub scale: f64,
    pub skew_x: f64,
    pub x: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OverlayState {
    Color {
        color: String,
        opacity: f64,
    },
    Gradient {
        colors: Vec<String>,
        shape: String,
        opacity: f64,
    },
}

/// 与 JS `handlers.vignette` 对齐的暗角状态（Canvas 批量渲染缺画，wgpu 补齐）。
#[derive(Debug, Clone, PartialEq)]
pub struct VignetteState {
    pub opacity: f64,
}

/// 中间事件索引，等价于 JS `buildIndex()` 的 `EVENT_IDX`。
#[derive(Debug, Clone, Default)]
pub struct EventIndex {
    pub camera: Vec<IndexedEvent>,
    pub sticker: Vec<IndexedSticker>,
    pub shake: Vec<IndexedEvent>,
    pub flash: Vec<IndexedEvent>,
    pub blur: Vec<IndexedEvent>,
    pub overlay: Vec<IndexedEvent>,
    pub stutter: Vec<IndexedEvent>,
    pub split: Vec<IndexedEvent>,
    pub zoom_bounce: Vec<IndexedEvent>,
    pub vignette: Vec<IndexedEvent>,
    pub hue_shift: Vec<IndexedEvent>,
    pub noise: Vec<IndexedEvent>,
    pub pulse: Vec<IndexedEvent>,
    pub color_shift: Vec<IndexedEvent>,
}

#[derive(Debug, Clone)]
pub struct IndexedEvent {
    pub time: f64,
    pub duration: f64,
    pub ev: Event,
}

#[derive(Debug, Clone)]
pub struct IndexedSticker {
    pub time: f64,
    pub duration: f64,
    pub ev: Event,
    /// 全局贴纸序号，对应 JS `sticker_N` 创建顺序。
    pub order: usize,
}

/// 从项目配置构建事件索引。
pub fn build_index(segments: &[Segment]) -> EventIndex {
    let mut idx = EventIndex::default();
    let mut sticker_order = 0usize;
    for seg in segments {
        for ev in &seg.events {
            let time = ev.get_time();
            let duration = ev.duration();
            match ev {
                Event::Camera { .. } => idx.camera.push(IndexedEvent {
                    time,
                    duration,
                    ev: ev.clone(),
                }),
                Event::Sticker { .. } => {
                    idx.sticker.push(IndexedSticker {
                        time,
                        duration,
                        ev: ev.clone(),
                        order: sticker_order,
                    });
                    sticker_order += 1;
                }
                Event::Shake { .. } => idx.shake.push(IndexedEvent {
                    time,
                    duration,
                    ev: ev.clone(),
                }),
                Event::Flash { .. } => idx.flash.push(IndexedEvent {
                    time,
                    duration,
                    ev: ev.clone(),
                }),
                Event::Blur { .. } => idx.blur.push(IndexedEvent {
                    time,
                    duration,
                    ev: ev.clone(),
                }),
                Event::Overlay { .. } => idx.overlay.push(IndexedEvent {
                    time,
                    duration,
                    ev: ev.clone(),
                }),
                Event::Stutter { .. } => idx.stutter.push(IndexedEvent {
                    time,
                    duration,
                    ev: ev.clone(),
                }),
                Event::Split { .. } => idx.split.push(IndexedEvent {
                    time,
                    duration,
                    ev: ev.clone(),
                }),
                Event::ZoomBounce { .. } => idx.zoom_bounce.push(IndexedEvent {
                    time,
                    duration,
                    ev: ev.clone(),
                }),
                Event::Vignette { .. } => idx.vignette.push(IndexedEvent {
                    time,
                    duration,
                    ev: ev.clone(),
                }),
                Event::HueShift { .. } => idx.hue_shift.push(IndexedEvent {
                    time,
                    duration,
                    ev: ev.clone(),
                }),
                Event::Noise { .. } => idx.noise.push(IndexedEvent {
                    time,
                    duration,
                    ev: ev.clone(),
                }),
                Event::Pulse { .. } => idx.pulse.push(IndexedEvent {
                    time,
                    duration,
                    ev: ev.clone(),
                }),
                Event::ColorShift { .. } => idx.color_shift.push(IndexedEvent {
                    time,
                    duration,
                    ev: ev.clone(),
                }),
                _ => {}
            }
        }
    }
    idx
}

/// 计算某一帧的状态（对应 JS `renderFrame` 中的状态取值部分）。
#[allow(clippy::too_many_arguments)]
pub fn evaluate_frame(
    t: f64,
    idx: &EventIndex,
    layout: &crate::types::Layout,
    _sub_style: &crate::types::SubtitleStyle,
    _dm_style: &crate::types::DanmakuStyle,
    camera_style: &crate::types::CameraStyle,
    subs: &[crate::types::Subtitle],
    dms: &[crate::types::Danmaku],
    dm_widths: &[f64],
    vertical: bool,
    vertical_position: &str,
) -> FrameState {
    let camera = get_active_camera(t, &idx.camera, layout, camera_style);
    let shake = get_shake(t, &idx.shake);
    let flash = get_flash(t, &idx.flash);
    let blur = get_blur(t, &idx.blur);
    let cur_zoom = camera.zoom;
    let zoom_bounce = get_zoom_bounce(t, &idx.zoom_bounce, cur_zoom);
    let pulse = get_pulse(t, &idx.pulse, cur_zoom);
    let hue_shift = get_hue_shift_flash(t, &idx.hue_shift);
    let stutter = get_stutter_video(t, &idx.stutter);
    let split = get_split_video(t, &idx.split);
    let noise = get_noise_opacity(t, &idx.noise);
    let color_shift = get_color_shift(t, &idx.color_shift);
    let overlay = get_overlay_state(t, &idx.overlay);
    let vignette = get_vignette(t, &idx.vignette);
    let intro_black = get_intro_black(t);
    let captions = get_active_subs(t, subs);
    let danmaku = get_active_dms(t, dms, dm_widths);
    let stickers = get_active_sticker(t, &idx.sticker, vertical, vertical_position);

    FrameState {
        camera,
        shake,
        flash,
        blur,
        zoom_bounce,
        pulse,
        hue_shift,
        stutter,
        split,
        noise,
        color_shift,
        overlay,
        vignette,
        intro_black,
        captions,
        danmaku,
        stickers,
    }
}

/// 与 JS `getActiveCamera` 一致。
pub fn get_active_camera(
    t: f64,
    cam: &[IndexedEvent],
    layout: &crate::types::Layout,
    camera_style: &crate::types::CameraStyle,
) -> CameraState {
    let mut zoom = 1.0;
    let mut ox = 65.0;
    let mut oy = 50.0;

    let def_origin =
        crate::types::region_origin(&layout.liver).unwrap_or_else(|| "65% 50%".into());
    let dp: Vec<&str> = def_origin.split_whitespace().collect();
    if let Some(v) = dp.first().and_then(|s| parse_percent(s)) {
        ox = v;
    }
    if let Some(v) = dp.get(1).and_then(|s| parse_percent(s)) {
        oy = v;
    }

    for c in cam {
        let Event::Camera {
            zoom: zoom_name,
            origin,
            ..
        } = &c.ev
        else {
            continue;
        };
        let tgt = if zoom_name == "近" { camera_style.zoom_close } else { camera_style.zoom_mid };
        let td = (c.duration * 0.5).min(0.6);
        let ce = c.time + td;
        if t < c.time {
            break;
        }
        if t >= ce {
            zoom = tgt;
        } else {
            let p = (t - c.time) / td;
            zoom = lerp(zoom, tgt, super::easing::pow2_out(p));
        }
        let o = origin.as_str();
        if let Some(v) = o.split_whitespace().next().and_then(|s| parse_percent(s)) {
            ox = v;
        }
        if let Some(v) = o.split_whitespace().nth(1).and_then(|s| parse_percent(s)) {
            oy = v;
        }
    }

    CameraState {
        zoom,
        ox: ox * 1920.0 / 100.0,
        oy: oy * 1080.0 / 100.0,
    }
}

/// 与 JS `getActiveSubs` 一致。
pub fn get_active_subs(t: f64, subs: &[crate::types::Subtitle]) -> Vec<CaptionState> {
    let mut out = Vec::new();
    for s in subs {
        if t < s.start || t >= s.end {
            continue;
        }
        let mut opacity = 1.0;
        let mut y_offset = 0.0;
        let fade_in = 0.35;
        let fade_out = 0.18;
        if t - s.start < fade_in {
            let p = (t - s.start) / fade_in;
            opacity = super::easing::pow2_out(p);
            y_offset = 20.0 * (1.0 - opacity);
        } else if s.end - t < fade_out {
            let p = (s.end - t) / fade_out;
            opacity = super::easing::pow2_in(p);
            y_offset = -8.0 * (1.0 - p);
        }
        out.push(CaptionState {
            parts: parse_highlight_parts(&s.html, &s.text),
            opacity,
            y_offset,
        });
    }
    out
}

/// 解析 `<span class="hl">` 高亮结构为分段。
pub fn parse_highlight_parts(html: &str, fallback: &str) -> Vec<CaptionPart> {
    let mut parts = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<span class=\"hl\">") {
        let before = &rest[..start];
        if !before.trim().is_empty() {
            parts.push(CaptionPart {
                text: before.to_string(),
                hl: false,
            });
        }
        let after_open = &rest[start + "<span class=\"hl\">".len()..];
        if let Some(end_rel) = after_open.find("</span>") {
            let hl_text = &after_open[..end_rel];
            if !hl_text.is_empty() {
                parts.push(CaptionPart {
                    text: hl_text.to_string(),
                    hl: true,
                });
            }
            rest = &after_open[end_rel + "</span>".len()..];
        } else {
            break;
        }
    }
    if !rest.trim().is_empty() {
        parts.push(CaptionPart {
            text: rest.to_string(),
            hl: false,
        });
    }
    if parts.is_empty() {
        parts.push(CaptionPart {
            text: fallback.to_string(),
            hl: false,
        });
    }
    parts
}

/// 与 JS `getActiveDms` 一致；弹幕宽度用实测宽度（swash 量宽），影响滚动距离与剔除。
pub fn get_active_dms(
    t: f64,
    dms: &[crate::types::Danmaku],
    widths: &[f64],
) -> Vec<DanmakuState> {
    let mut out = Vec::new();
    for (i, d) in dms.iter().enumerate() {
        if t < d.start || t >= d.end {
            continue;
        }
        let travel = d.travel_duration;
        let text_w = widths.get(i).copied().unwrap_or_else(|| (d.text.chars().count() as f64 * 55.0) + 20.0);
        let el = t - d.start;
        let fade_in = 0.3;
        let fade_out = 0.2;
        let y = (d.y / 1920.0) * 540.0;

        let progress = ((el - fade_in) / travel).max(0.0);
        let x = 1920.0 - progress.min(1.0) * (1920.0 + text_w);

        let mut opacity = 0.9;
        if el < fade_in {
            opacity = 0.9 * (el / fade_in);
            out.push(DanmakuState {
                text: d.text.clone(),
                opacity,
                x: 1920.0,
                y,
            });
            continue;
        } else if t > d.end - fade_out {
            opacity = 0.9 * ((d.end - t) / fade_out).max(0.0);
        }

        if opacity < 0.005 || x < -text_w - 50.0 {
            continue;
        }
        out.push(DanmakuState {
            text: d.text.clone(),
            opacity,
            x,
            y,
        });
    }
    out
}

/// 与 JS `getActiveSticker` 一致；竖屏时按 vertical_position 覆盖/左右交替 x（对齐模板 GSAP 段）。
pub fn get_active_sticker(
    t: f64,
    items: &[IndexedSticker],
    vertical: bool,
    vertical_position: &str,
) -> Vec<StickerState> {
    let mut out = Vec::new();
    let mut side_toggle = false;
    for (i, e) in items.iter().enumerate() {
        let et = e.time;
        let dur = e.duration;
        if t < et || t > et + dur + 0.65 {
            continue;
        }
        let pop_in = 0.35;
        let float_up = 0.4;
        let fade_out = 0.3;
        let opacity;
        let mut scale = 1.0;
        let mut rot = 0.0;
        let mut y_off = 0.0;

        if t - et < pop_in {
            let p = (t - et) / pop_in;
            let bp = super::easing::back_out(p, Some(3.0));
            scale = 0.3 + 0.7 * bp;
            rot = -15.0 + 15.0 * bp;
            opacity = super::easing::pow2_out(p);
        } else if t < et + pop_in + float_up {
            let p = (t - et - pop_in) / float_up;
            opacity = 1.0;
            scale = 1.0;
            rot = 0.0;
            y_off = -18.0 * p;
        } else if t < et + pop_in + float_up + dur {
            opacity = 1.0;
            y_off = -18.0;
        } else {
            let p = (t - et - pop_in - float_up - dur) / fade_out;
            opacity = 1.0 - p;
            scale = 1.0 - 0.6 * p;
            y_off = -18.0;
        }
        if opacity < 0.005 {
            continue;
        }

        let (x, y) = match &e.ev {
            Event::Sticker { x, y, .. } => {
                let mut lx = parse_percent(x).unwrap_or(50.0);
                let ly = parse_percent(y).unwrap_or(50.0);
                // 竖屏：视频在左 → 贴纸居中；视频居中 → 左右交替（对齐模板 GSAP 段）
                if vertical {
                    if vertical_position == "left" {
                        lx = 50.0;
                    } else {
                        side_toggle = !side_toggle;
                        lx = if side_toggle { 15.0 } else { 85.0 };
                    }
                }
                (lx / 100.0 * 1920.0, ly / 100.0 * 1080.0)
            }
            _ => (0.0, 0.0),
        };

        let name = match &e.ev {
            Event::Sticker { name, keyword, .. } => {
                name.clone().unwrap_or_else(|| keyword.clone())
            }
            _ => String::new(),
        };

        // 贴纸安全范围：sprite 层按中心绘制，保留 280 盒子一半 + 24px 边距
        let sticker_safe = 32.0;
        let half = 140.0;
        let cx = x.clamp(sticker_safe + half, 1920.0 - sticker_safe - half);
        let cy = (y + y_off).clamp(sticker_safe + half, 1080.0 - sticker_safe - half);

        out.push(StickerState {
            name,
            et,
            index: i,
            opacity,
            scale,
            rot,
            // 绘制层会把 x/y 作为中心并减去 w/2、h/2
            x: cx,
            y: cy,
            w: 280.0,
            h: 280.0,
        });
    }
    out
}

/// 与 JS `getShake` 一致。
pub fn get_shake(t: f64, items: &[IndexedEvent]) -> ShakeState {
    for sh in items {
        if t >= sh.time && t < sh.time + 0.2 {
            let off = match &sh.ev {
                Event::Shake { intensity, .. } => *intensity,
                _ => 14.0,
            };
            let dt = t - sh.time;
            if dt < 0.04 {
                return ShakeState { x: off, y: -off };
            }
            if dt < 0.08 {
                return ShakeState { x: -off, y: off };
            }
            if dt < 0.12 {
                return ShakeState {
                    x: off / 2.0,
                    y: -off / 2.0,
                };
            }
            if dt < 0.16 {
                return ShakeState {
                    x: -off / 2.0,
                    y: off / 2.0,
                };
            }
            return ShakeState { x: 0.0, y: 0.0 };
        }
    }
    ShakeState { x: 0.0, y: 0.0 }
}

/// 与 JS `getFlash` 一致。
pub fn get_flash(t: f64, items: &[IndexedEvent]) -> FlashState {
    for f in items {
        let et = f.time;
        let color = match &f.ev {
            Event::Flash { color, .. } => color.clone().unwrap_or_else(|| "#ffffff".into()),
            _ => "#ffffff".into(),
        };
        let dur = f.duration.max(0.1);
        if t >= et && t < et + 0.04 + dur {
            let dt = t - et;
            if dt < 0.04 {
                return FlashState {
                    opacity: 0.20 * (dt / 0.04),
                    color,
                };
            }
            let fade = (dt - 0.04) / dur;
            return FlashState {
                opacity: 0.20 * (1.0 - fade),
                color,
            };
        }
    }
    FlashState {
        opacity: 0.0,
        color: "#ffffff".into(),
    }
}

/// 与 JS `getBlur` 一致。
pub fn get_blur(t: f64, items: &[IndexedEvent]) -> f64 {
    for b in items {
        let et = b.time;
        if t >= et && t < et + 0.80 {
            let dt = t - et;
            if dt < 0.48 {
                return 8.0;
            }
            return 8.0 * (1.0 - (dt - 0.48) / 0.32);
        }
    }
    0.0
}

/// 与 JS `getOverlayState` 一致。
pub fn get_overlay_state(t: f64, items: &[IndexedEvent]) -> Option<OverlayState> {
    for o in items {
        let et = o.time;
        let dur = o.duration.max(0.1);
        if t >= et && t < et + dur {
            let dt = t - et;
            let fade_in = 0.4;
            let fade_out = 0.4;
            let mut opacity = match &o.ev {
                Event::Overlay { opacity, .. } => opacity.unwrap_or(1.0),
                _ => 1.0,
            };
            if dt < fade_in {
                opacity *= dt / fade_in;
            } else if dur - dt < fade_out {
                opacity *= (dur - dt) / fade_out;
            }
            if opacity < 0.005 {
                continue;
            }
            match &o.ev {
                Event::Overlay {
                    overlay_type,
                    color,
                    colors,
                    shape,
                    ..
                } => {
                    if overlay_type == "color" && color.is_some() {
                        return Some(OverlayState::Color {
                            color: color.clone().unwrap(),
                            opacity,
                        });
                    }
                    if overlay_type == "gradient" && colors.is_some() {
                        return Some(OverlayState::Gradient {
                            colors: colors.clone().unwrap(),
                            shape: shape.clone().unwrap_or_else(|| "linear".into()),
                            opacity,
                        });
                    }
                }
                _ => {}
            }
            return None;
        }
    }
    None
}

/// 与 JS `handlers.vignette` 一致。
pub fn get_vignette(t: f64, items: &[IndexedEvent]) -> VignetteState {
    for v in items {
        let et = v.time;
        let dur = v.duration.max(0.1);
        if t >= et && t < et + dur {
            let hold_end = et + dur * 0.4;
            let fade_end = et + dur;
            let opacity = if t < hold_end {
                1.0
            } else {
                1.0 - (t - hold_end) / (fade_end - hold_end).max(0.001)
            };
            return VignetteState { opacity };
        }
    }
    VignetteState { opacity: 0.0 }
}

/// 与 JS `getIntroBlack` 一致。
pub fn get_intro_black(t: f64) -> f64 {
    if t < 0.5 {
        return 1.0 - super::easing::sine_out(t / 0.5);
    }
    0.0
}

/// 与 JS `getStutterVideo` 一致。
pub fn get_stutter_video(t: f64, items: &[IndexedEvent]) -> Option<(f64, f64)> {
    for st in items {
        let et = st.time;
        let off = match &st.ev {
            Event::Stutter { intensity, .. } => intensity.unwrap_or(12.0),
            _ => 12.0,
        };
        if t >= et && t < et + 0.12 {
            let dt = t - et;
            if dt < 0.06 {
                return Some((-off, off));
            }
            return Some((off, -off));
        }
    }
    None
}

/// 与 JS `getSplitVideo` 一致。
pub fn get_split_video(t: f64, items: &[IndexedEvent]) -> Option<SplitState> {
    for sp in items {
        let et = sp.time;
        let inten = match &sp.ev {
            Event::Split { intensity, .. } => intensity.unwrap_or(30.0),
            _ => 30.0,
        };
        if t >= et && t < et + 0.12 {
            let dt = t - et;
            if dt < 0.06 {
                return Some(SplitState {
                    scale: 0.95,
                    skew_x: -inten,
                    x: -inten * 0.3,
                });
            }
            return Some(SplitState {
                scale: 1.0,
                skew_x: 0.0,
                x: 0.0,
            });
        }
    }
    None
}

/// 与 JS `getZoomBounce` 一致。
pub fn get_zoom_bounce(t: f64, items: &[IndexedEvent], cur_zoom: f64) -> f64 {
    for zb in items {
        let et = zb.time;
        if t >= et && t < et + 0.30 {
            let dt = t - et;
            if dt < 0.08 {
                return cur_zoom * 1.10;
            }
            return cur_zoom;
        }
    }
    cur_zoom
}

/// 与 JS `getPulse` 一致。
pub fn get_pulse(t: f64, items: &[IndexedEvent], cur_zoom: f64) -> f64 {
    for p in items {
        let et = p.time;
        if t >= et && t < et + 0.48 {
            let dt = t - et;
            if dt < 0.12 {
                return cur_zoom * 1.06;
            }
            if dt < 0.28 {
                return cur_zoom * 0.97;
            }
            return cur_zoom;
        }
    }
    cur_zoom
}

/// 与 JS `getHueShiftFlash` 一致。
pub fn get_hue_shift_flash(t: f64, items: &[IndexedEvent]) -> Option<FlashState> {
    for h in items {
        let et = h.time;
        let color = match &h.ev {
            Event::HueShift { color, .. } => color.clone().unwrap_or_else(|| "#ff0040".into()),
            _ => "#ff0040".into(),
        };
        if t >= et && t < et + 0.95 {
            let dt = t - et;
            if dt < 0.15 {
                return Some(FlashState {
                    opacity: 0.25 * (dt / 0.15),
                    color,
                });
            }
            return Some(FlashState {
                opacity: 0.25 * (1.0 - (dt - 0.15) / 0.80),
                color,
            });
        }
    }
    None
}

/// 与 JS `getNoiseOpacity` 一致。
pub fn get_noise_opacity(t: f64, items: &[IndexedEvent]) -> f64 {
    for n in items {
        let et = n.time;
        if t >= et && t < et + 0.80 {
            let dt = t - et;
            if dt < 0.08 {
                return 0.08 * (dt / 0.08);
            }
            return 0.08 * (1.0 - (dt - 0.08) / 0.72);
        }
    }
    0.0
}

/// 与 JS `getColorShift` 一致。
pub fn get_color_shift(t: f64, items: &[IndexedEvent]) -> String {
    for c in items {
        let et = c.time;
        if t >= et && t < et + 1.0 {
            let dt = t - et;
            if dt < 0.1 {
                return "sepia(0.6) saturate(1.4)".into();
            }
            let p = (dt - 0.1) / 0.6;
            let sepia = 0.6 * (1.0 - p);
            let saturate = 1.4 - 0.4 * p;
            return format!("sepia({}) saturate({})", sepia, saturate);
        }
    }
    String::new()
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn parse_percent(s: &str) -> Option<f64> {
    let v = s.trim_end_matches('%').trim().parse::<f64>().ok()?;
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Segment;

    fn event(ev: Event) -> Segment {
        Segment {
            start: 0.0,
            end: 10.0,
            fx: "low".into(),
            note: String::new(),
            events: vec![ev],
            extra: Default::default(),
        }
    }

    #[test]
    fn camera_zoom_close_matches_js() {
        let seg = event(Event::Camera {
            time: 0.0,
            duration: 2.0,
            zoom: "近".into(),
            origin: "65% 50%".into(),
        });
        let mut cfg = ProjectFile::example();
        cfg.segments = vec![seg];
        let idx = build_index(&cfg);
        let cam = get_active_camera(2.0, &idx.camera, &cfg.layout, &cfg.style.camera);
        assert!((cam.zoom - cfg.style.camera.zoom_close).abs() < 1e-9);
        assert!((cam.ox - 0.65 * 1920.0).abs() < 1e-6);
        assert!((cam.oy - 0.5 * 1080.0).abs() < 1e-6);
    }

    #[test]
    fn shake_returns_expected_phases() {
        let seg = event(Event::Shake {
            time: 1.0,
            intensity: 14.0,
            duration: Some(0.3),
            anchor: None,
            sfx: None,
            sfx_volume: None,
        });
        let mut cfg = ProjectFile::example();
        cfg.segments = vec![seg];
        let idx = build_index(&cfg);
        assert_eq!(
            get_shake(1.01, &idx.shake),
            ShakeState { x: 14.0, y: -14.0 }
        );
        assert_eq!(
            get_shake(1.05, &idx.shake),
            ShakeState { x: -14.0, y: 14.0 }
        );
        assert_eq!(get_shake(1.2, &idx.shake), ShakeState { x: 0.0, y: 0.0 });
    }

    #[test]
    fn intro_black_uses_sine_out() {
        assert!((get_intro_black(0.0) - 1.0).abs() < 1e-9);
        assert!((get_intro_black(0.5) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn vignette_matches_js_hold_and_fade() {
        let seg = event(Event::Vignette {
            time: 1.0,
            duration: Some(0.4),
            anchor: None,
            sfx: None,
            sfx_volume: None,
        });
        let mut cfg = ProjectFile::example();
        cfg.segments = vec![seg];
        let idx = build_index(&cfg);
        assert_eq!(
            get_vignette(1.0, &idx.vignette),
            VignetteState { opacity: 1.0 }
        );
        assert_eq!(
            get_vignette(1.1, &idx.vignette),
            VignetteState { opacity: 1.0 }
        );
        // 0.4s 事件: hold 到 1.16s，之后线性消退
        let mid = get_vignette(1.3, &idx.vignette);
        assert!(mid.opacity > 0.0 && mid.opacity < 1.0);
        // 事件 1.0+0.4=1.4 结束，事件外为 0
        assert_eq!(
            get_vignette(1.5, &idx.vignette),
            VignetteState { opacity: 0.0 }
        );
        assert_eq!(
            get_vignette(2.0, &idx.vignette),
            VignetteState { opacity: 0.0 }
        );
    }

    #[test]
    fn parse_highlight_parts_handles_spans() {
        let parts = parse_highlight_parts("今天<span class=\"hl\">游戏</span>很好", "fallback");
        assert_eq!(
            parts,
            vec![
                CaptionPart {
                    text: "今天".into(),
                    hl: false
                },
                CaptionPart {
                    text: "游戏".into(),
                    hl: true
                },
                CaptionPart {
                    text: "很好".into(),
                    hl: false
                },
            ]
        );
    }
}

/// 贴纸 GIF 帧序号：按事件相对时间循环（供 sprite 层调用）。
pub fn sticker_frame_index_for(event_start: f64, t: f64, delays: &[u16]) -> usize {
    if delays.is_empty() { return 0; }
    let total: u32 = delays.iter().map(|d| *d as u32).sum();
    if total == 0 { return 0; }
    let elapsed_ms = ((t - event_start).max(0.0) * 1000.0) as u32;
    let loop_pos = elapsed_ms % total;
    let mut acc = 0u32;
    for (i, d) in delays.iter().enumerate() {
        acc += *d as u32;
        if loop_pos < acc { return i; }
    }
    0
}
