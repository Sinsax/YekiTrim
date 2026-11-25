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
    /// 手持摇摆的**幅度** 0~1（`Event::Camera::handheld` 的状态机值，与 `zoom` 同一段缓动）。
    /// 波形本身见 `types::handheld_sway(t, amp)`。
    pub handheld: f64,
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
    pub pulse: f64,
    pub hue_shift: Option<FlashState>,
    pub noise: f64,
    pub overlay: Option<OverlayState>,
    pub vignette: VignetteState,
    pub captions: Vec<CaptionState>,
    pub danmaku: Vec<DanmakuState>,
    pub stickers: Vec<StickerState>,
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
        /// 线性渐变方向（CSS 约定；`None` = 参照缺省 `to bottom right`）。
        angle: Option<f64>,
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
    pub vignette: Vec<IndexedEvent>,
    pub hue_shift: Vec<IndexedEvent>,
    pub noise: Vec<IndexedEvent>,
    pub pulse: Vec<IndexedEvent>,
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
    // 竖屏（源素材竖）不做运镜 —— 手持摇摆是叠加在运镜上的状态，跟着一起屏蔽
    let camera = if vertical {
        CameraState { handheld: 0.0, ..camera }
    } else {
        camera
    };
    let shake = get_shake(t, &idx.shake);
    let flash = get_flash(t, &idx.flash);
    let blur = get_blur(t, &idx.blur);
    let cur_zoom = camera.zoom;
    let pulse = get_pulse(t, &idx.pulse, cur_zoom);
    let hue_shift = get_hue_shift_flash(t, &idx.hue_shift);
    let noise = get_noise_opacity(t, &idx.noise);
    let overlay = get_overlay_state(t, &idx.overlay);
    let vignette = get_vignette(t, &idx.vignette);
    let captions = get_active_subs(t, subs);
    let danmaku = get_active_dms(t, dms, dm_widths);
    let stickers = get_active_sticker(t, &idx.sticker, vertical, vertical_position);

    FrameState {
        camera,
        shake,
        flash,
        blur,
        pulse,
        hue_shift,
        noise,
        overlay,
        vignette,
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
    // 手持摇摆幅度：与 zoom 完全同一套状态机（同一段 td / 同一条缓动），
    // 差别只是它的"档位"只有 0/1 —— 开了就跟到下一条不带 handheld 的运镜。
    let mut handheld = 0.0;

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
            to_origin,
            handheld: handheld_ev,
            ..
        } = &c.ev
        else {
            continue;
        };
        let tgt = camera_style.factor(zoom_name);
        let td = (c.duration * 0.5).min(0.6);
        let ce = c.time + td;
        if t < c.time {
            break;
        }
        // td 为 0 时 ce == c.time，到这里必然走「过渡已结束」分支，不会除零
        let p = if t >= ce {
            1.0
        } else {
            super::easing::pow2_out((t - c.time) / td)
        };
        zoom = if t >= ce { tgt } else { lerp(zoom, tgt, p) };
        // ⚠️ 这一份（wgpu 求值）还没有 JS 侧的 hardCut（cut / 任一端远景 → 瞬间到位）口径，
        // 是老差异；手持摇摆这里就按同一段 p 缓动，等 wgpu 路径启用时与 JS 一起对齐。
        handheld = lerp(handheld, if *handheld_ev { 1.0 } else { 0.0 }, p);

        // 不动点：本事件的 origin → to_origin（缺省即原地不动）。
        // 起点取事件自己的 origin，与 zoom 同一段窗口、同一条缓动。
        //
        // ⚠️⚠️ **这一份是旧语义，别照抄**（2026-11-21）：生产链路（模板 `getActiveCamera` /
        // `engine.js cameraAt` / 转译器）已经把**位置改成连续状态** —— 接缝先接上一块的落点、
        // 块内再走到 `to_origin`。这里仍是"每块绝对赋值"（`p=0` 直接等于本块 origin），
        // 那正是块缝瞬跳 928px 的写法。本文件只作 CPU 参考、无生产消费者（见本 crate 卡），
        // 所以**不跟着改**；但也**不许**被当成口径抄回去。
        let mut it = origin.split_whitespace();
        let ax = it.next().and_then(parse_percent).unwrap_or(ox);
        let ay = it.next().and_then(parse_percent).unwrap_or(oy);
        let (bx, by) = match to_origin.as_deref() {
            Some(to) => {
                let mut it = to.split_whitespace();
                (
                    it.next().and_then(parse_percent).unwrap_or(ax),
                    it.next().and_then(parse_percent).unwrap_or(ay),
                )
            }
            None => (ax, ay),
        };
        ox = lerp(ax, bx, p);
        oy = lerp(ay, by, p);
    }

    CameraState {
        zoom,
        ox: ox * 1920.0 / 100.0,
        oy: oy * 1080.0 / 100.0,
        handheld,
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

/// 与 JS `getActiveSticker` 一致（DOM 路径回读 `style.left/top` 再 +`140*CK`）。
///
/// 落点取事件自带的 `x` / `y`（**盒子左上角**百分比），即 `process/placement.rs`
/// 安全区算法写进 segments 的值 —— 与 Chrome 出片路径的 `auto` 模式一致。
/// 返回的 [`StickerState`] 的 `x` / `y` 是**盒子中心**（sprite 层按中心绘制），
/// 盒子 280×280（本路径固定 1920×1080 画布，`sticker_scale` 恒为 1）。
///
/// ⚠️ **本路径还没接 `[sticker] position_mode = manual`**（固定落点）—— 它需要把
/// `_canvas.sticker` 传进来，即改签名 + 重生成 wasm-bindgen 胶水，本阶段不动；
/// 已知缺口见 `docs/plan/polish-vertical-canvas.md` §1.4 第 7 条。
///
/// ⚠️ 这里原本还有一段「竖屏时按 `vertical_position` 覆盖 x / 左右交替」的强制覆盖，
/// 已随 `[sticker] position_mode` 落地而删除（该行为由 `manual` 模式正式替代，
/// `auto` 一律走安全区算法）。见 `docs/plan/polish-vertical-canvas.md` §A4。
///
/// `vertical` / `vertical_position` **保留形参**：落点已与这两个参数无关，而当年传它们的
/// GPU 路径（`yekitrim-wasm` 的 `init_renderer` + `yekitrim-polish/wgpu-render`）已整条删除；
/// 形参留着只是为了不再改一次签名（本文件现在只有 CPU 参考与单测在用）。
pub fn get_active_sticker(
    t: f64,
    items: &[IndexedSticker],
    _vertical: bool,
    _vertical_position: &str,
) -> Vec<StickerState> {
    let mut out = Vec::new();
    for (i, e) in items.iter().enumerate() {
        let et = e.time;
        let dur = e.duration;
        // 窗口 = **事件时长 `dur`**（不溢出到事件之外）：淡出在窗口**内部**消化
        // （`dur - 0.3 -> dur` 渐隐）⇒ 出点 = 最后一帧。
        // 旧判据 `dur + (0.35 + 0.4 + 0.3)` = `dur + 1.05` 多显示 0.4 秒 ✗
        // （与 `templates/index.html` 的 `getActiveSticker`、`engine.js::stickersAt` 同形）。
        if t < et || t > et + dur {
            continue;
        }
        // 装不下 `pop_in 0.35 + float_up 0.4 + fade_out 0.3` 时**优先保淡出**：
        // 前两段按可用时长等比压缩（淡出起点不得早于 0）；退化时长没有可见窗口。
        let fade_out = dur.min(0.3);
        if fade_out <= 0.0 {
            continue;
        }
        let k = ((dur - fade_out).max(0.0) / (0.35 + 0.4)).min(1.0);
        let pop_in = 0.35 * k;
        let float_up = 0.4 * k;
        let fade_start = dur - fade_out;
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
        } else if t < et + fade_start {
            opacity = 1.0;
            y_off = -18.0;
        } else {
            let p = (t - et - fade_start) / fade_out;
            // `.max(0.0)` 只吃掉上界那一帧的浮点尾数：出点必须**恰好 0**。
            opacity = (1.0 - p).max(0.0);
            scale = 1.0 - 0.6 * p;
            y_off = -18.0;
        }
        if opacity < 0.005 {
            continue;
        }

        // 落点：`ev.x` / `ev.y` 是**贴纸盒子左上角**百分比（`process/placement.rs` 契约）
        // → 先换成盒子中心。⚠️ 曾经直接把百分比当中心用，比 DOM 路径（`index.html`
        // 的 `el.style.left/top` + `engine.js::stickersAt` 都按左上角）整整偏了半个盒子。
        let (tl_x, tl_y) = match &e.ev {
            Event::Sticker { x, y, .. } => {
                // 落点只用事件自带值：竖屏不再覆盖（原先「视频在左→居中 / 否则左右交替」
                // 是一刀切 hack，现由 [sticker] position_mode=manual 正式表达；
                // ⚠️ manual 的固定落点本路径还没接 —— 它需要新的形参，而签名连着
                // wasm-bindgen 胶水，见函数头注释）
                let lx = parse_percent(x).unwrap_or(50.0);
                let ly = parse_percent(y).unwrap_or(50.0);
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

        // 贴纸安全范围：sprite 层按**中心**绘制（`draw_stickers_into` 减 w/2、h/2），
        // 所以中心必须落在 `[32 + 半盒, 画布边 - 32 - 半盒]` —— 与 DOM 路径对盒子左上角的
        // `[32, 边 - 280 - 32]` 钳制等价。
        // ⚠️ **先 clamps 再叠加 `y_off`**：DOM 侧的 `y:-18` 是 GSAP transform，作用在已钳制
        // 的位置上（允许越出安全区一个上浮量）；顺序反过来两边就差 18px。
        let sticker_safe = 32.0;
        let half = 140.0;
        let cx = (tl_x + half).clamp(sticker_safe + half, 1920.0 - sticker_safe - half);
        let cy =
            (tl_y + half).clamp(sticker_safe + half, 1080.0 - sticker_safe - half) + y_off;

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

/// 「该色 → 透明」里的**第二档**。
///
/// ⚠ **不要用 CSS 的 `transparent`**（它等于 `rgba(0,0,0,0)`）：canvas 的渐变在**非预乘**空间
/// 插值，`#ff0000 → transparent` 会从中间插出 `rgba(127,0,0,·)` 那种**暗红**。实测同一条
/// 红色覆盖层（opacity 0.3，同一个源帧、同一个时刻）：
///   · 第二档 `transparent`          → 画面平均 R **184.12 → 178.82**（红了反而变暗）
///   · 第二档 `rgba(255,0,0,0)`      → 画面平均 R **184.12 → 193.66**（干净的淡出）
/// 而 CSS 那边两者**完全等价**（按预乘插值，实测取样逐点相同）—— 所以用 `transparent` 会
/// 让**浏览器预览与出片观感不一致**，正是覆盖层这一类 bug 的同一个坑。
/// 认不出的颜色串（命名色等）退回 `transparent`（偏暗，但至少有东西）。
pub fn overlay_fade_stop(color: &str) -> String {
    match rgb_triplet(color) {
        Some((r, g, b)) => format!("rgba({r},{g},{b},0)"),
        None => "transparent".into(),
    }
}

/// 从 `#rgb` / `#rrggbb` / `#rrggbbaa` / `rgb()` / `rgba()` 里取 RGB 三元组。
fn rgb_triplet(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        return match h.len() {
            3 => {
                let mut it = h.chars().map(|c| c.to_digit(16).map(|d| (d * 17) as u8));
                Some((it.next()??, it.next()??, it.next()??))
            }
            6 | 8 => {
                let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
                Some((p(0)?, p(2)?, p(4)?))
            }
            _ => None,
        };
    }
    let lower = s.to_ascii_lowercase();
    let body = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))?
        .split(')')
        .next()?;
    let mut nums = body.split(',').map(|p| p.trim().parse::<f64>().ok());
    let r = nums.next()??.clamp(0.0, 255.0) as u8;
    let g = nums.next()??.clamp(0.0, 255.0) as u8;
    let b = nums.next()??.clamp(0.0, 255.0) as u8;
    Some((r, g, b))
}

/// 覆盖层「渐变」的有效色档 —— 与 JS `overlayStops(ev)` 逐条对齐。
///
/// 四处实现必须同改：本文件、`crates/yekitrim-polish/templates/index.html`（出片权威路径）、
/// `webui/src/polish/engine.js`、`webui/src/components/PolishPreview.vue`（经 engine）。
///
/// **返回的色档保证 ≥2 档且无空串**，这不是美观问题：
/// canvas 按 `addColorStop(i / (n - 1))` 铺档，`n == 1` 时是 `0/0 = NaN` → 抛
/// `IndexSizeError`，会把整个 chunk 的渲染打断；空串色档则抛 `SyntaxError`。
/// 老工程与 LLM 一直只写单个 `color`（`colors` 压根没出现过），所以这条兜底是常规路径。
pub fn overlay_stops(color: &Option<String>, colors: &Option<Vec<String>>) -> Option<Vec<String>> {
    let cleaned: Vec<String> = colors
        .iter()
        .flatten()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();
    if cleaned.len() >= 2 {
        return Some(cleaned);
    }
    // 只剩一档（或只有 color）：按「该色 → 透明」渲染
    let base = cleaned.into_iter().next().or_else(|| {
        color
            .as_ref()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
    })?;
    let fade = overlay_fade_stop(&base);
    Some(vec![base, fade])
}

/// 与 JS `getOverlayState` 一致。
///
/// `overlay_type` 认四档：`color`（纯色）、`gradient`（色档渐变）、`fade_in` / `fade_out`
/// （**黑场淡入 / 淡出** —— 名字说的是**画面**：`fade_in` 画面从遮罩里浮起来、`fade_out` 沉进去）。
/// 这两档**整窗就是过渡本身**（`fade_in`：窗口起点全遮 → 窗口末端全透；`fade_out` 反过来），
/// 不叠那 0.4s 的进出包络；`color` 是遮罩颜色（缺省黑）、`opacity` 是遮罩的最大不透明度。
/// 想让画面**停在**黑场，把窗口末端拖到片子结束 —— 事件窗口之外照常是正常画面，
/// 覆盖层没有"无限保持"的语义（这是刻意的）。
///
/// ⚠ 片头那 0.5s 黑场原来是**内建** `intro-black`（四处硬编码、没有任何手动入口，用户
/// 2026-09-22 提出要能自己改）。现在由 `process` 在首段补一条 `fade_in` 事件表达：它出现在
/// 时间轴上，可拖、可改时长、可换色、可删。内建那套**已整条退役**（本文件、出片模板、
/// `engine.js`、`PolishPreview.vue`、wgpu effects 同步删），别再往回加。
/// `overlay{overlay_type:"hue"}` 的**缺省窗口**（秒）。
///
/// 老 `hue_shift`（三端 `getHueShiftFlash`）的窗口是**写死 0.95s**、只认 `time`；
/// 2026-10-20 并入 overlay 之后窗口改由 `[time, end]` 决定，而读时迁移会把 `end` 归一化成
/// `time + 0.95` ⇒ 曲线与老实现**逐点相同**。手写 `overlay{hue}` 不写 `end` 时用这个缺省。
pub const HUE_WINDOW_SECS: f64 = 0.95;

pub fn get_overlay_state(t: f64, items: &[IndexedEvent]) -> Option<OverlayState> {
    for o in items {
        let et = o.time;
        let dur = o.duration.max(0.1);
        if t < et || t >= et + dur {
            continue;
        }
        let dt = t - et;
        let fade_in = 0.4;
        let fade_out = 0.4;
        let mut opacity = match &o.ev {
            Event::Overlay { opacity, .. } => opacity.unwrap_or(1.0),
            _ => 1.0,
        };
        let Event::Overlay {
            overlay_type,
            color,
            colors,
            shape,
            angle,
            ..
        } = &o.ev
        else {
            continue;
        };
        // **hue 档**（2026-10-20 由退役的 `hue_shift` 并入）：整帧纯色 + **归一化包络**。
        //
        // ⚠️ 曲线必须与老 `get_hue_shift_flash` **逐点相同**：峰位 `R = 0.15/0.95`、
        //    峰值 0.25；`frac < R` 线性升、之后线性落到 0；`frac ∉ [0,1)` 由上面的窗口判断挡掉。
        //    ⇒ 迁移把 `end` 归一化成 `time+0.95` 之后，两者在任意时刻都相等（有单测钉住 5 点 + 边界）。
        // ⚠️ 它与 `color` 档**共用同一套落点**（返回 `OverlayState::Color`）——"一档覆盖层"的语义：
        //    同一时刻有多条覆盖层时**只画第一条**（与其它档一致，别在这里新造一条例外）。
        if overlay_type == "hue" {
            let hdur = match &o.ev {
                Event::Overlay { end, .. } => end
                    .map(|e| e - o.time)
                    .filter(|d| *d > 0.0)
                    .unwrap_or(HUE_WINDOW_SECS),
                _ => HUE_WINDOW_SECS,
            };
            let frac = (t - o.time) / hdur;
            let r = 0.15 / HUE_WINDOW_SECS;
            let env = if frac < r {
                0.25 * frac / r
            } else {
                0.25 * (1.0 - (frac - r) / (1.0 - r))
            };
            let col = color
                .as_ref()
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| "#ff0040".to_string());
            return Some(OverlayState::Color {
                color: col,
                opacity: (opacity * env).clamp(0.0, 1.0),
            });
        }
        let stops = overlay_stops(color, colors);
        if overlay_type == "fade_in" || overlay_type == "fade_out" {
            // 整窗线性斜坡：窗口就是过渡，不叠 0.4s 包络（叠了会变成"两头各一段缓入缓出"）
            opacity *= if overlay_type == "fade_in" {
                1.0 - dt / dur
            } else {
                dt / dur
            };
            if opacity < 0.005 {
                continue;
            }
            let solid = color
                .as_ref()
                .map(|c| c.trim())
                .filter(|c| !c.is_empty())
                .or_else(|| stops.as_ref().and_then(|s| s.first()).map(|s| s.as_str()))
                // 淡入/淡出缺省就是黑场：面板切到这两档时不写颜色也能用
                .unwrap_or("#000000");
            return Some(OverlayState::Color {
                color: solid.to_string(),
                opacity,
            });
        }
        if dt < fade_in {
            opacity *= dt / fade_in;
        } else if dur - dt < fade_out {
            opacity *= (dur - dt) / fade_out;
        }
        if opacity < 0.005 {
            continue;
        }
        if overlay_type == "color" {
            // color 缺失时退回第一档：面板把类型从「渐变」切到「纯色」不该把颜色弄丢
            let solid = color
                .as_ref()
                .map(|c| c.trim())
                .filter(|c| !c.is_empty())
                .or_else(|| stops.as_ref().and_then(|s| s.first()).map(|s| s.as_str()));
            if let Some(c) = solid {
                return Some(OverlayState::Color {
                    color: c.to_string(),
                    opacity,
                });
            }
            continue;
        }
        if overlay_type == "gradient" {
            if let Some(stops) = stops {
                return Some(OverlayState::Gradient {
                    colors: stops,
                    shape: shape.clone().unwrap_or_else(|| "linear".into()),
                    angle: *angle,
                    opacity,
                });
            }
        }
        // 类型不认识 / 值不全：继续看窗口里的下一条（整窗 bail 会把后面有效的覆盖层一起吃掉）
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


fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn parse_percent(s: &str) -> Option<f64> {
    let v = s.trim_end_matches('%').trim().parse::<f64>().ok()?;
    Some(v)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Layout, Segment, StyleRoot};

    /// 渲染核不持有 `ProjectFile`（那是 polish 侧 `project::config` 的类型，本 crate 看不到），
    /// 测试只用到 segments / layout / style 三样，这里给个最小替身。
    struct Cfg {
        segments: Vec<Segment>,
        layout: Layout,
        style: StyleRoot,
    }

    impl Cfg {
        fn example() -> Self {
            Self {
                segments: Vec::new(),
                layout: Layout::default(),
                style: StyleRoot::default(),
            }
        }
    }

    fn event(ev: Event) -> Segment {
        Segment {
            start: 0.0,
            end: 10.0,
            fx: "low".into(),
            note: String::new(),
            role: String::new(),
            events: vec![ev],
            extra: Default::default(),
        }
    }

    /// 只有 origin 的运镜事件（无位移）。
    fn camera_ev(time: f64, dur: f64, zoom: &str, origin: &str) -> Event {
        camera_ev_h(time, dur, zoom, origin, false)
    }

    /// 同上，可带手持摇摆。
    fn camera_ev_h(time: f64, dur: f64, zoom: &str, origin: &str, handheld: bool) -> Event {
        Event::Camera {
            time,
            end: Some(time + dur),
            zoom: zoom.into(),
            origin: origin.into(),
            anchor: None,
            anchor_to: None,
            to_origin: None, cut: false,
            handheld,
            speed: None,
            tilt: None,
            manual: false,
        }
    }

    #[test]
    fn camera_zoom_close_matches_js() {
        let mut cfg = Cfg::example();
        cfg.segments = vec![event(camera_ev(0.0, 2.0, "近", "65% 50%"))];
        let idx = build_index(&cfg.segments);
        let cam = get_active_camera(2.0, &idx.camera, &cfg.layout, &cfg.style.camera);
        assert!((cam.zoom - cfg.style.camera.zoom_close).abs() < 1e-9);
        assert!((cam.ox - 0.65 * 1920.0).abs() < 1e-6);
        assert!((cam.oy - 0.5 * 1080.0).abs() < 1e-6);
    }

    /// 「远」是画中画档：倍率小于 1，且必须真的落到 CameraState 上。
    /// 渲染侧靠 `|zoom-1| > 0.001` 判断是否施加变换，曾经写成 `> 1` 把这一档整个吞掉。
    #[test]
    fn camera_wide_level_is_below_one() {
        let mut cfg = Cfg::example();
        cfg.segments = vec![event(camera_ev(0.0, 2.0, "远", "50% 50%"))];
        let idx = build_index(&cfg.segments);
        let cam = get_active_camera(2.0, &idx.camera, &cfg.layout, &cfg.style.camera);
        assert!(
            (cam.zoom - cfg.style.camera.zoom_wide).abs() < 1e-9,
            "远 档应取 zoom_wide: {}",
            cam.zoom
        );
        assert!(cam.zoom < 1.0, "远 档必须小于 1（画中画）");
        assert!(cfg.style.camera.zoom_wide < 1.0, "默认配置的 远 档就该是画中画");
        // 具体数值也钉住：0.55 = 四周各留 432px 黑边（0.7 是迁移前的旧默认）
        assert_eq!(cfg.style.camera.zoom_wide, 0.55);
    }

    /// 位移：不动点从 origin 移到 to_origin，与 zoom 同一段窗口、同一条 pow2Out 缓动。
    #[test]
    fn camera_pans_origin_over_transition() {        let mut cfg = Cfg::example();
        let ev = match camera_ev(0.0, 4.0, "中", "80% 50%") {
            Event::Camera { time, end, zoom, origin, anchor, anchor_to, cut, handheld, speed, .. } => {
                Event::Camera {
                    time,
                    end,
                    zoom,
                    origin,
                    anchor,
                    anchor_to,
                    speed,
                    to_origin: Some("20% 50%".into()),
                    cut,
                    handheld,
                    manual: false,
                    tilt: None,
                }
            }
            other => other,
        };
        cfg.segments = vec![event(ev)];
        let idx = build_index(&cfg.segments);
        let cam_at = |t: f64| get_active_camera(t, &idx.camera, &cfg.layout, &cfg.style.camera);

        // 过渡窗口 = min(duration*0.5, 0.6) = 0.6s
        assert!((cam_at(0.0).ox - 0.80 * 1920.0).abs() < 1e-6, "起点应是 origin");
        assert!((cam_at(0.6).ox - 0.20 * 1920.0).abs() < 1e-6, "过渡结束应落到 to_origin");
        // 中点：pow2Out(0.5) = 0.75 → 1536 + (384-1536)*0.75 = 672
        let mid = cam_at(0.3).ox;
        assert!((mid - 672.0).abs() < 1e-6, "中点应在两者之间: {mid}");
        assert!((cam_at(1.5).ox - 0.20 * 1920.0).abs() < 1e-6, "过渡结束后不再变");
        // y 没有终点，全程不动
        assert!((cam_at(0.3).oy - 0.5 * 1080.0).abs() < 1e-6);
    }

    /// 手持摇摆是**持续状态**：开了就一直跟到下一条不带 handheld 的运镜，
    /// 起停与景别共用同一段 `min(dur*0.5, 0.6)` 缓动。
    #[test]
    fn camera_handheld_follows_state_machine() {
        let mut cfg = Cfg::example();
        cfg.segments = vec![Segment {
            start: 0.0,
            end: 12.0,
            fx: "low".into(),
            note: String::new(),
            role: String::new(),
            events: vec![
                camera_ev_h(0.0, 4.0, "近", "65% 50%", true),
                camera_ev_h(6.0, 4.0, "中", "65% 50%", false),
            ],
            extra: Default::default(),
        }];
        let idx = build_index(&cfg.segments);
        let amp = |t: f64| get_active_camera(t, &idx.camera, &cfg.layout, &cfg.style.camera).handheld;

        assert_eq!(amp(0.0), 0.0, "起点幅度为 0（缓动刚起步）");
        assert!((amp(0.6) - 1.0).abs() < 1e-9, "过渡结束即全幅摇摆: {}", amp(0.6));
        assert!((amp(5.9) - 1.0).abs() < 1e-9, "一路跟到下一条运镜");
        // 下一条不带 handheld：同一段缓动收回去（pow2Out(0.5)=0.75 → 0.25）
        let mid = amp(6.3);
        assert!((mid - 0.25).abs() < 1e-9, "收尾中点: {mid}");
        assert_eq!(amp(6.6), 0.0, "收完就静止（不能留残余摆动）");
    }

    /// 摇摆波形必须与 JS 两份实现逐值一致（漂移了预览就和成片不一样）。
    /// 期望值取自 `webui/src/polish/engine.js::handheldSway`（由 `check-handheld.mjs` 对拍的那份）。
    #[test]
    fn handheld_sway_matches_js_reference() {
        let v = |t: f64, amp: f64| {
            let s = crate::types::handheld_sway(t, amp);
            [s.dx, s.dy, s.rot, s.scale]
        };
        // amp=0 → 完全静止（OFF），与 t 无关
        assert_eq!(v(3.7, 0.0), [0.0, 0.0, 0.0, 1.0]);
        // 全幅：四个采样点把两个正弦分量的相位都覆盖到
        for (t, want) in [
            (0.0, [2.974_994_431_357_405_5, 5.537_746_181_460_179, 0.162_893_762_213_005_36, 1.006_238_451_520_430_2]),
            (1.5, [-6.534_804_875_504_133, 1.998_507_667_634_624_3, -0.358_933_426_843_904_06, 0.998_691_374_050_889_2]),
            (4.25, [-2.966_335_968_174_989, -1.859_259_740_722_605_2, -0.100_907_167_151_848_72, 1.003_076_791_799_218_5]),
            (9.9, [-5.095_502_881_135_531, -1.502_144_180_998_791_4, -0.287_127_406_065_84, 1.003_292_193_865_482]),
        ] {
            let got = v(t, 1.0);
            for i in 0..4 {
                assert!(
                    (got[i] - want[i]).abs() < 1e-9,
                    "t={t} 第 {i} 项漂移: 实得 {} 期望 {}",
                    got[i],
                    want[i]
                );
            }
        }
        // 幅度按比例缩放（起停过渡的那 0.6s）
        let half = v(1.5, 0.5);
        for i in 0..3 {
            assert!((half[i] - v(1.5, 1.0)[i] * 0.5).abs() < 1e-9);
        }
        assert!((half[3] - 1.0 - (v(1.5, 1.0)[3] - 1.0) * 0.5).abs() < 1e-9);
    }

    #[test]
    fn shake_returns_expected_phases() {
        let seg = event(Event::Shake {
            time: 1.0,
            intensity: 14.0,
            end: Some(1.3),
            anchor: None,
            sfx: None,
            sfx_volume: None,
        });
        let mut cfg = Cfg::example();
        cfg.segments = vec![seg];
        let idx = build_index(&cfg.segments);
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
    fn vignette_matches_js_hold_and_fade() {
        let seg = event(Event::Vignette {
            time: 1.0,
            end: Some(1.4),
            anchor: None,
            sfx: None,
            sfx_volume: None,
        });
        let mut cfg = Cfg::example();
        cfg.segments = vec![seg];
        let idx = build_index(&cfg.segments);
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

    // ── 覆盖层色档契约（四处实现同改，见 overlay_stops 的文档注释）──────────────
    fn overlay_ev(
        time: f64,
        dur: f64,
        overlay_type: &str,
        color: Option<&str>,
        colors: Option<Vec<&str>>,
    ) -> Event {
        Event::Overlay {
            time,
            end: Some(time + dur),
            overlay_type: overlay_type.into(),
            color: color.map(|c| c.into()),
            opacity: Some(1.0),
            colors: colors.map(|v| v.into_iter().map(|c| c.into()).collect()),
            shape: None,
            angle: None,
            sfx: None,
            sfx_volume: None,
            anchor: None,
        }
    }

    fn stops(color: Option<&str>, colors: Option<Vec<&str>>) -> Option<Vec<String>> {
        overlay_stops(
            &color.map(|c| c.to_string()),
            &colors.map(|v| v.into_iter().map(|c| c.to_string()).collect()),
        )
    }

    // ── hue 并入覆盖层（2026-10-20）：与老 getHueShiftFlash 逐点相等 ────────────────

    fn legacy_hue(t: f64) -> Option<(f64, String)> {
        let items = vec![IndexedEvent {
            time: 0.0,
            duration: 0.95,
            ev: Event::HueShift {
                time: 0.0,
                color: Some("#ff0040".into()),
                end: Some(0.95),
                anchor: None,
                sfx: None,
                sfx_volume: None,
            },
        }];
        get_hue_shift_flash(t, &items).map(|f| (f.opacity, f.color))
    }

    fn hue_at(items: &[IndexedEvent], t: f64) -> Option<(f64, String)> {
        match get_overlay_state(t, items) {
            Some(OverlayState::Color { color, opacity }) => Some((opacity, color)),
            other => panic!("t={t}: hue 档必须返回 Color，实得 {other:?}"),
        }
    }

    /// **曲线逐点相等**（5 点 + 两个边界）：迁移后的 `overlay{hue}` 与老 `hue_shift` 必须同值。
    ///
    /// 变异自证：把峰位 `0.15/0.95` 改成 `0.20/0.95` ⇒ 0.05/0.15 两点当场不等 ⇒ 红。
    #[test]
    fn hue_overlay_matches_legacy_hue_shift_at_sample_points() {
        let items = vec![IndexedEvent {
            time: 0.0,
            duration: 0.95,
            ev: overlay_ev(0.0, 0.95, "hue", Some("#ff0040"), None),
        }];
        for t in [0.0, 0.05, 0.15, 0.30, 0.60, 0.90] {
            let old = legacy_hue(t).expect("老实现窗口内应当有值");
            let new = hue_at(&items, t).expect("新档窗口内应当有值");
            assert!(
                (old.0 - new.0).abs() < 1e-9,
                "t={t}: 老 {} vs 新 {}（曲线漂了）",
                old.0,
                new.0
            );
            assert_eq!(old.1, new.1, "t={t}: 颜色必须一致");
        }
        // 边界 ①：t == time 恰为 0（老实现与新的都得是 0，不是 None）
        assert_eq!(hue_at(&items, 0.0).unwrap().0, 0.0);
        // 边界 ②：t == time + 0.95 恰为 null（窗口右开，与老三端同口径）
        assert!(get_overlay_state(0.95, &items).is_none(), "窗口右边界必须是开区间");
        assert!(legacy_hue(0.95).is_none());
    }

    /// **`end` 缺失 ⇒ 缺省 0.95s**（手写 `overlay{hue}` 不写 end 也要能用）。
    #[test]
    fn hue_overlay_defaults_to_0_95_when_end_missing() {
        let mut ev = overlay_ev(0.0, 0.95, "hue", Some("#ff0040"), None);
        if let Event::Overlay { end, .. } = &mut ev {
            *end = None;
        }
        let items = vec![IndexedEvent { time: 0.0, duration: 0.95, ev }];
        for t in [0.05, 0.15, 0.30, 0.60, 0.90] {
            let old = legacy_hue(t).expect("老值");
            let new = hue_at(&items, t).expect("缺 end 也要落到缺省 0.95");
            assert!((old.0 - new.0).abs() < 1e-9, "t={t}: 缺省窗口下曲线必须同值");
        }
    }

    /// **同窗多条覆盖层 ⇒ 只画第一条**（overlay 的既有语义；hue 并入后自然也这样）。
    ///
    /// 这不是回归：迁移前 `hue_shift` 在**独立桶**里画，所以「同一时刻既有 hue 又有 overlay」
    /// 是两条都画；`hue` 并入 overlay 之后二者同桶 ⇒ 只画第一条（一档覆盖层）。
    /// ⚠️ 第二段断言「第一条在窗外 ⇒ 画第二条」是**防止顺序巧合**：只断言"画第一条"时，
    ///    "永远取列表第一条"也能满足它。
    #[test]
    fn overlay_takes_only_the_first_in_window_event() {
        let both = vec![
            IndexedEvent { time: 0.0, duration: 1.0, ev: overlay_ev(0.0, 1.0, "color", Some("#111111"), None) },
            IndexedEvent { time: 0.0, duration: 1.0, ev: overlay_ev(0.0, 1.0, "hue", Some("#ff0040"), None) },
        ];
        let (_, first_color) = hue_at(&both, 0.2).expect("第一条应当画出来");
        assert_eq!(first_color, "#111111", "同窗只画第一条");
        // 第一条在窗外（t=2.0 时它的窗口 0..1 已过）⇒ 必须画第二条 hue
        let late = vec![
            IndexedEvent { time: 0.0, duration: 1.0, ev: overlay_ev(0.0, 1.0, "color", Some("#111111"), None) },
            IndexedEvent { time: 1.5, duration: 1.0, ev: overlay_ev(1.5, 1.0, "hue", Some("#ff0040"), None) },
        ];
        let (op, second_color) = hue_at(&late, 2.0).expect("第二条应当画出来");
        assert_eq!(second_color, "#ff0040");
        assert!(op > 0.0);
    }

    #[test]
    fn overlay_stops_single_color_fades_to_transparent() {
        assert_eq!(
            stops(Some("#ff0000"), None),
            Some(vec!["#ff0000".to_string(), "rgba(255,0,0,0)".to_string()])
        );
    }

    /// 第二档必须是**同色零透明**，不能是 CSS 的 `transparent`：canvas 在非预乘空间插值，
    /// 红 → transparent 会插成暗红（实测画面平均 R 从 184.12 掉到 178.82），
    /// 而 `rgba(255,0,0,0)` 是干净的淡出（R 升到 193.66）。CSS 那边两者等价。
    #[test]
    fn overlay_fade_stop_keeps_hue() {
        assert_eq!(overlay_fade_stop("#ff0000"), "rgba(255,0,0,0)");
        assert_eq!(overlay_fade_stop("  #00FF00 "), "rgba(0,255,0,0)");
        assert_eq!(overlay_fade_stop("#f00"), "rgba(255,0,0,0)");
        assert_eq!(overlay_fade_stop("#12345678"), "rgba(18,52,86,0)");
        assert_eq!(overlay_fade_stop("rgba(255,50,50,0.3)"), "rgba(255,50,50,0)");
        assert_eq!(overlay_fade_stop("rgb(1,2,3)"), "rgba(1,2,3,0)");
        // 认不出的（命名色）退回 transparent：canvas 里会偏暗，但至少有东西
        assert_eq!(overlay_fade_stop("rebeccapurple"), "transparent");
        assert_eq!(overlay_fade_stop(""), "transparent");
    }

    #[test]
    fn overlay_stops_single_stop_is_padded_to_two() {
        // 一档：canvas 的 i/(n-1) 会变成 0/0 = NaN，必须补到两档
        assert_eq!(
            stops(Some("#ff0000"), Some(vec!["#00ff00"])),
            Some(vec!["#00ff00".to_string(), "rgba(0,255,0,0)".to_string()])
        );
    }

    #[test]
    fn overlay_stops_three_stops_kept_verbatim() {
        // demo 用的是 3 档 + 每档自带 alpha
        let three = vec!["#140A28CC", "#28143CCC", "#140A2888"];
        assert_eq!(
            stops(None, Some(three.clone())),
            Some(three.into_iter().map(|c| c.to_string()).collect())
        );
    }

    #[test]
    fn overlay_stops_filters_blank_entries() {
        // 空串色档会让 canvas addColorStop 抛 SyntaxError
        assert_eq!(
            stops(None, Some(vec!["#ff0000", "", "   "])),
            Some(vec!["#ff0000".to_string(), "rgba(255,0,0,0)".to_string()])
        );
        assert_eq!(stops(Some("  "), Some(vec!["", " "])), None);
        assert_eq!(stops(None, None), None);
    }

    #[test]
    fn gradient_renders_with_only_color() {
        // 真实数据 8/8 都是这个形态：overlay_type=gradient + 单个 color，colors 从未出现过
        let mut cfg = Cfg::example();
        cfg.segments = vec![event(overlay_ev(1.0, 3.0, "gradient", Some("#ff0000"), None))];
        let idx = build_index(&cfg.segments);
        match get_overlay_state(1.5, &idx.overlay) {
            Some(OverlayState::Gradient { colors, shape, .. }) => {
                assert_eq!(colors, vec!["#ff0000".to_string(), "rgba(255,0,0,0)".to_string()]);
                assert_eq!(shape, "linear");
            }
            other => panic!("应解析成渐变，实得 {other:?}"),
        }
    }

    #[test]
    fn gradient_without_any_color_renders_nothing() {
        let mut cfg = Cfg::example();
        cfg.segments = vec![event(overlay_ev(1.0, 3.0, "gradient", None, None))];
        let idx = build_index(&cfg.segments);
        assert_eq!(get_overlay_state(1.5, &idx.overlay), None);
    }

    #[test]
    fn color_overlay_falls_back_to_first_stop() {
        let mut cfg = Cfg::example();
        cfg.segments = vec![event(overlay_ev(
            1.0,
            3.0,
            "color",
            None,
            Some(vec!["#00ff00", "#000000"]),
        ))];
        let idx = build_index(&cfg.segments);
        assert_eq!(
            get_overlay_state(1.5, &idx.overlay),
            Some(OverlayState::Color {
                color: "#00ff00".into(),
                opacity: 1.0
            })
        );
    }

    #[test]
    fn unknown_overlay_type_does_not_swallow_later_overlay() {
        // 窗口里第一条类型不认识时，要接着看下一条 —— 不能整窗 bail
        let mut cfg = Cfg::example();
        cfg.segments = vec![
            event(overlay_ev(1.0, 3.0, "love", Some("#ff0000"), None)),
            event(overlay_ev(1.2, 3.0, "gradient", Some("#0033ff"), None)),
        ];
        let idx = build_index(&cfg.segments);
        match get_overlay_state(1.8, &idx.overlay) {
            Some(OverlayState::Gradient { colors, .. }) => {
                assert_eq!(colors, vec!["#0033ff".to_string(), "rgba(0,51,255,0)".to_string()]);
            }
            other => panic!("应落到第二条覆盖层，实得 {other:?}"),
        }
    }

    /// fade 两档的专用构造：`opacity` 是遮罩**最大**不透明度。
    fn fade_ev(time: f64, dur: f64, kind: &str, color: Option<&str>, opacity: f64) -> Event {
        let mut ev = overlay_ev(time, dur, kind, color, None);
        if let Event::Overlay { opacity: o, .. } = &mut ev {
            *o = Some(opacity);
        }
        ev
    }

    /// ⚠ `fade_in` 是**整窗线性斜坡**：窗口起点全遮、末端全透、中点 0.5。
    /// 关键是**不叠**那 0.4s 进出包络 —— 叠了起点会变成 0，黑场压根打不开
    /// （用户看到的症状就是「开头渐入没生效」）。出片模板与 `engine.js` 同口径，
    /// JS 侧对拍见 `node webui/scripts/check-overlay.mjs`。
    #[test]
    fn fade_in_is_a_whole_window_ramp() {
        let mut cfg = Cfg::example();
        cfg.segments = vec![event(fade_ev(1.0, 2.0, "fade_in", None, 1.0))];
        let idx = build_index(&cfg.segments);
        // 起点：全遮 + 缺省黑（面板切到「淡入」不写颜色也能用）
        assert_eq!(
            get_overlay_state(1.0, &idx.overlay),
            Some(OverlayState::Color {
                color: "#000000".into(),
                opacity: 1.0
            })
        );
        match get_overlay_state(2.0, &idx.overlay) {
            Some(OverlayState::Color { color, opacity }) => {
                assert_eq!(color, "#000000");
                assert!((opacity - 0.5).abs() < 1e-9, "中点应 0.5，实得 {opacity}");
            }
            other => panic!("中点应仍是黑幕，实得 {other:?}"),
        }
        // 窗口之外没有"无限保持"的语义：想停在黑场就把末端拖到片子结束
        assert_eq!(get_overlay_state(0.99, &idx.overlay), None);
        assert_eq!(get_overlay_state(3.0, &idx.overlay), None);
    }

    /// `fade_out` 是 `fade_in` 的镜像；窗口起点不透明度 0（没画东西）→ 返回 `None`。
    #[test]
    fn fade_out_mirrors_fade_in() {
        let mut cfg = Cfg::example();
        cfg.segments = vec![event(fade_ev(1.0, 2.0, "fade_out", None, 1.0))];
        let idx = build_index(&cfg.segments);
        assert_eq!(get_overlay_state(1.0, &idx.overlay), None);
        // 前 1% 窗口低于 0.005 阈值（与 color / gradient 同一条判据）
        assert_eq!(get_overlay_state(1.005, &idx.overlay), None);
        match get_overlay_state(2.0, &idx.overlay) {
            Some(OverlayState::Color { color, opacity }) => {
                assert_eq!(color, "#000000");
                assert!((opacity - 0.5).abs() < 1e-9, "中点应 0.5，实得 {opacity}");
            }
            other => panic!("中点应仍是黑幕，实得 {other:?}"),
        }
    }

    /// `opacity` 是**遮罩最大不透明度**（乘在斜坡上），颜色可换 —— 「黑色遮罩」只是缺省。
    #[test]
    fn fade_opacity_scales_the_ramp() {
        let mut cfg = Cfg::example();
        cfg.segments = vec![event(fade_ev(1.0, 2.0, "fade_in", Some("#ff3232"), 0.6))];
        let idx = build_index(&cfg.segments);
        assert_eq!(
            get_overlay_state(1.0, &idx.overlay),
            Some(OverlayState::Color {
                color: "#ff3232".into(),
                opacity: 0.6
            })
        );
        match get_overlay_state(1.5, &idx.overlay) {
            Some(OverlayState::Color { opacity, .. }) => {
                assert!((opacity - 0.45).abs() < 1e-9, "0.6 × 0.75 应 0.45，实得 {opacity}");
            }
            other => panic!("实得 {other:?}"),
        }
    }

    /// fade 取色链与 `color` 档一致：`color` → `colors[0]` → 黑（LLM 只写 `colors` 也不会没颜色）。
    #[test]
    fn fade_color_falls_back_to_first_stop() {
        let mut cfg = Cfg::example();
        cfg.segments = vec![event(overlay_ev(
            1.0,
            2.0,
            "fade_in",
            None,
            Some(vec!["#00ff00", "#000000"]),
        ))];
        let idx = build_index(&cfg.segments);
        assert_eq!(
            get_overlay_state(1.0, &idx.overlay),
            Some(OverlayState::Color {
                color: "#00ff00".into(),
                opacity: 1.0
            })
        );
    }

    /// 老两档（`color` / `gradient`）的 0.4s 进出包络**没被动过** —— fade 走的是另一条分支。
    /// 这是"零行为回归"的钉子：改动 fade 时别顺手把包络改了。
    #[test]
    fn color_overlay_keeps_the_0_4s_envelope() {
        let mut cfg = Cfg::example();
        cfg.segments = vec![event(overlay_ev(1.0, 3.0, "color", Some("#000000"), None))];
        let idx = build_index(&cfg.segments);
        assert_eq!(get_overlay_state(1.0, &idx.overlay), None);
        match get_overlay_state(1.2, &idx.overlay) {
            Some(OverlayState::Color { opacity, .. }) => {
                assert!((opacity - 0.5).abs() < 1e-9, "0.2s 应在包络半程，实得 {opacity}");
            }
            other => panic!("实得 {other:?}"),
        }
        assert_eq!(
            get_overlay_state(2.0, &idx.overlay),
            Some(OverlayState::Color {
                color: "#000000".into(),
                opacity: 1.0
            })
        );
    }

    /// 贴纸事件（`x` / `y` 是**盒子左上角**百分比，与 `process/placement.rs` 的契约一致）。
    fn sticker_ev(time: f64, dur: f64, x: &str, y: &str) -> Event {
        Event::Sticker {
            time,
            end: Some(time + dur),
            keyword: "ysyk".into(),
            sticker_tag: None,
            name: Some("1f600".into()),
            source: None,
            x: x.into(),
            y: y.into(),
            sfx: None,
            sfx_volume: None,
            manual: false,
            manual_pos: None,
        }
    }

    /// 落点：事件 `x` / `y` 是**盒子左上角**，本函数返回的 `x` / `y` 是**盒子中心**
    /// （sprite 层按中心绘制，`x - w/2`）。曾经直接把百分比当中心用，整整偏半个盒子。
    /// 同一套数字由 JS 侧对拍：`node webui/scripts/check-sticker-anchor.mjs`（横屏那几例）。
    #[test]
    fn sticker_anchor_is_box_top_left_plus_half_box() {
        let mut cfg = Cfg::example();
        cfg.segments = vec![event(sticker_ev(0.0, 2.5, "12%", "42%"))];
        let idx = build_index(&cfg.segments);
        // 12%*1920 = 230.4、42%*1080 = 453.6 → 加半个盒子 140 → 中心 (370.4, 593.6)。
        // t=1.5 落在「保持」段（入场 0.35 + 浮起 0.4 = 0.75 之后），y_off 恒为 -18。
        let s = get_active_sticker(1.5, &idx.sticker, false, "").pop().expect("应有贴纸");
        assert!((s.x - 370.4).abs() < 1e-6, "x 应为盒子中心: {}", s.x);
        assert!((s.y - (593.6 - 18.0)).abs() < 1e-6, "y 应为「盒子中心 - 上浮」: {}", s.y);
        assert!((s.w - 280.0).abs() < 1e-9 && (s.h - 280.0).abs() < 1e-9, "盒子应为 280×280");
    }

    /// 左上角越界时：**先把中心钳进安全区、再叠加 `y_off`**。
    /// ⚠️ 顺序反过来（先叠上浮再钳）两边的数会差 18px —— DOM 侧的 `y:-18` 是作用在
    /// 已钳制的 CSS 位置上的 transform，允许越出安全区一个上浮量。
    #[test]
    fn sticker_center_is_clamped_before_float_up() {
        let mut cfg = Cfg::example();
        cfg.segments = vec![event(sticker_ev(0.0, 2.5, "0%", "0%"))];
        let idx = build_index(&cfg.segments);
        // 左上角 (0,0) → 中心 (140,140) → 钳到下限 32 + 140 = 172 → 再减 18
        // （= DOM 侧盒子 top 14px，与 index.html `stickerBox` + GSAP `y:-18*SK` 一致）
        let s = get_active_sticker(1.5, &idx.sticker, false, "").pop().expect("应有贴纸");
        assert!((s.x - 172.0).abs() < 1e-6, "x 应被钳到 172: {}", s.x);
        assert!((s.y - 154.0).abs() < 1e-6, "y 应「先钳到 172 再减 18」: {}", s.y);
    }
}
