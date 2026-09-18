//! 贴纸落点推导：从 **人物 / 字幕 / 弹幕** 反选安全区，并在安全区**中心**撒点。
//!
//! 坐标系与渲染端一致（`templates/index.html` 的 Canvas，1920×1080 整帧）：
//! - `Event::Sticker` 的 x/y 是**贴纸盒子左上角**百分比 —— 模板 DOM/canvas
//!   (`index.html:675` 钳制 + `:1246` 画 `left+140`) 与 `webui/src/polish/engine.js:517`
//!   都按左上角消费；盒子固定 `STICKER_SIZE` × `STICKER_SIZE`。
//! - 模板把 left/top 钳制在 `[32, 1920-280-32] × [32, 1080-280-32]`，
//!   本模块的候选网格限制在同一范围内（`STICKER_MARGIN`）。
//!
//! 障碍来源（渲染顺序：视频 → 字幕 → 弹幕 → 贴纸，贴纸画在最上层，压住谁就是挡住谁）：
//! - **人物** `layout.liver`：在**源帧**坐标里框选（选框模式下运镜暂停、缩放复位），
//!   但输出里视频整体被相机缩放（`index.html:1439-1463` 以 origin 为中心 scale），
//!   所以要先按相机变换投到输出帧。
//! - **弹幕区域** `layout.danmaku_area`：同上，源帧坐标 → 相机变换。
//! - **字幕带**：输出帧固定（Canvas 以 `cy=960` 居中一行；DOM 容器 `bottom:80px`）。
//! - **弹幕带**：只有 `data/danmaku.json` 真有弹幕时才是障碍，按模板的
//!   `y = (d.y / 1920) * 540` + 字号/描边推出实际占位。
//!
//! 落点规则：候选 = 网格上所有「盒子不与任何障碍相交」的位置；取 clearance
//! （到最近障碍/帧边的像素距离）最高的一档（`CENTRAL_RATIO`），在该档里按
//! 「盒心到帧心距离」排序取前 `CENTRAL_PICK` 个，再用 seed 伪随机挑一个 ——
//! 即「安全区的中心，而不是边缘」，同时保留逐条变化。全部候选都被挡时退化为
//! 「重叠面积最小」（`Pick::relaxed == true`，调用方打警告）。

use crate::project::{parse_pct, Point};

/// 输出帧尺寸（模板 Canvas 硬编码 1920×1080）。
pub const FRAME_W: f64 = 1920.0;
pub const FRAME_H: f64 = 1080.0;
/// 贴纸盒子边长（模板 `.sticker-img` 280×280）。
pub const STICKER_SIZE: f64 = 280.0;
/// 贴纸安全边距（模板 `index.html:675` / `engine.js:124` 的 32px）。
pub const STICKER_MARGIN: f64 = 32.0;
/// 贴纸 floatUp 动画的上浮量（`getActiveSticker`: `yOff = -18`），测试障碍时算进去。
pub const FLOAT_UP: f64 = 18.0;

/// 候选网格步长（16px ≈ 帧宽的 0.83%）。
const GRID_STEP: f64 = 16.0;
/// 只在 clearance ≥ 最高档 × 该比例的候选里撒点（“中心而不是边缘”）。
const CENTRAL_RATIO: f64 = 0.9;
/// 中心档里参与伪随机挑选的候选个数（步长 16px → 落点抖动约 ±64px）。
const CENTRAL_PICK: usize = 8;
/// 相交判定容差：缝隙小于半像素也算相交，避免贴到障碍边上。
const EPS: f64 = 0.5;
/// 临界候选（贴着障碍边缘）保留的缝隙，保证仍能“刚好放下”。
const TOUCH: f64 = 1.0;
/// 运镜缩放动画时长（模板 `getActiveCamera` 调用方: `td = min(duration*0.5, 0.6)`）。
pub const TRANSITION_SECS: f64 = 0.6;

/// 画中画档位的档位名。
pub const WIDE_LABEL: &str = "远";

/// 这一段运镜的过渡时长：**硬切条件**（两端任一为远景「画中画」，或本事件自带 `cut`）
/// 时是 **0 = 硬切**，否则 0.6s 缓动。
///
/// 远景进出是「切」不是「推」——缓动缩到画中画看着像镜头在拉远，而它要的是喜剧式的切换。
/// `cut`（编辑器里的「方式 = 硬切」）把同一条规则放开给任意景别。
/// 模板 canvas、GSAP 与 `engine.js` 三处同步实现同一条规则；贴纸避让靠这个值知道
/// 那一段根本没有过渡窗口（否则会凭空多算出一个早已结束的旧状态当障碍）。
pub fn transition_secs(label: &str, prev_label: &str, cut: bool) -> f64 {
    if cut || label.trim() == WIDE_LABEL || prev_label.trim() == WIDE_LABEL {
        0.0
    } else {
        TRANSITION_SECS
    }
}

// ===== 矩形 =====

/// 帧内矩形（像素，左上原点）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    /// 构造并规范化（拖拽可能给反的角）。
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
        }
    }

    /// 百分比角点（`region_rects` 的输出）→ 帧像素。
    pub fn from_points(tl: &Point, br: &Point) -> Self {
        Self::new(
            parse_pct(&tl.x) / 100.0 * FRAME_W,
            parse_pct(&tl.y) / 100.0 * FRAME_H,
            parse_pct(&br.x) / 100.0 * FRAME_W,
            parse_pct(&br.y) / 100.0 * FRAME_H,
        )
    }

    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }

    /// 是否退化（面积可忽略）。
    pub fn is_degenerate(&self) -> bool {
        self.width() < EPS || self.height() < EPS
    }

    /// 是否与另一矩形相交（容差 `EPS`）。
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x0 < o.x1 - EPS && o.x0 < self.x1 - EPS && self.y0 < o.y1 - EPS && o.y0 < self.y1 - EPS
    }

    pub fn intersect_area(&self, o: &Rect) -> f64 {
        let w = (self.x1.min(o.x1) - self.x0.max(o.x0)).max(0.0);
        let h = (self.y1.min(o.y1) - self.y0.max(o.y0)).max(0.0);
        w * h
    }

    /// 到另一矩形的距离（相交 → 0）。
    pub fn gap_to(&self, o: &Rect) -> f64 {
        let dx = (self.x0 - o.x1).max(o.x0 - self.x1).max(0.0);
        let dy = (self.y0 - o.y1).max(o.y0 - self.y1).max(0.0);
        (dx * dx + dy * dy).sqrt()
    }

    /// 到帧边（安全边距之外的部分）的距离。
    pub fn gap_to_frame_edge(&self) -> f64 {
        self.x0.min(self.y0).min(FRAME_W - self.x1).min(FRAME_H - self.y1)
    }

    pub fn center(&self) -> (f64, f64) {
        ((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }

    /// 裁剪到帧内（相机变换会把矩形推到帧外）。
    pub fn clamp_to_frame(&self) -> Self {
        Self::new(
            self.x0.max(0.0).min(FRAME_W),
            self.y0.max(0.0).min(FRAME_H),
            self.x1.max(0.0).min(FRAME_W),
            self.y1.max(0.0).min(FRAME_H),
        )
    }
}

// ===== 障碍 =====

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObstacleKind {
    /// 人物（主播）：绝不主动压住。
    Liver,
    /// 字幕带：观众要读的字。
    Captions,
    /// 弹幕区域（源帧里直播间自己的弹幕面板）。
    DanmakuArea,
    /// 输出帧里我们自己滚动的弹幕。
    DanmakuBand,
    /// 同刻已落位的另一张贴纸。
    Sticker,
}

impl ObstacleKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Liver => "人物",
            Self::Captions => "字幕",
            Self::DanmakuArea => "弹幕区域",
            Self::DanmakuBand => "弹幕",
            Self::Sticker => "贴纸",
        }
    }

    /// 兜底（无安全位置时）重叠代价权重。
    fn weight(self) -> f64 {
        match self {
            Self::Liver => 3.0,
            Self::Captions => 3.0,
            Self::DanmakuArea => 2.0,
            Self::DanmakuBand => 1.0,
            Self::Sticker => 0.5,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Blocked {
    pub rect: Rect,
    pub kind: ObstacleKind,
}

impl Blocked {
    pub fn new(rect: Rect, kind: ObstacleKind) -> Self {
        Self { rect, kind }
    }
}

// ===== 相机（源帧 → 输出帧） =====

/// 一段运镜：`zoom` 缩放（"远"/"中"/"近" 之外的标签按中景处理），
/// 不动点从 `origin` 移到 `origin_to`（无位移时两者相等）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraShot {
    pub time: f64,
    pub end: f64,
    pub origin: (f64, f64),
    /// 过渡结束后的落点；等于 `origin` 即原地聚焦。
    pub origin_to: (f64, f64),
    pub zoom: f64,
    /// 过渡时长（秒）；0 = 硬切（远景进出）。见 `transition_secs`。
    pub transition: f64,
}

/// 瞬时运镜（zoom_bounce ×1.10 / pulse ×1.06）的预留系数，避免贴纸被瞬时放大的人物压住。
pub const TRANSIENT_ZOOM: f64 = 1.10;

/// 解析 `"50% 50%"` 形式的运镜中心。
pub fn parse_origin(s: &str) -> Option<(f64, f64)> {
    let mut it = s.split_whitespace();
    let x = parse_pct(it.next()?);
    let y = parse_pct(it.next()?);
    Some((x / 100.0 * FRAME_W, y / 100.0 * FRAME_H))
}

/// 以 `(ox, oy)` 为中心缩放 `rect`（与 `renderFrame` 的 translate/scale/translate 一致）。
pub fn camera_expand(rect: Rect, ox: f64, oy: f64, zoom: f64) -> Rect {
    if (zoom - 1.0).abs() < 1e-6 {
        return rect;
    }
    Rect::new(
        ox + (rect.x0 - ox) * zoom,
        oy + (rect.y0 - oy) * zoom,
        ox + (rect.x1 - ox) * zoom,
        oy + (rect.y1 - oy) * zoom,
    )
}

/// 贴纸可见窗口 `[t0, t1]` 内出现过的所有相机状态。
///
/// 输出里视频一直在运镜：窗口开始时的稳态 + 窗口内发生的缩放过渡两端。
/// 只有过渡真的落在窗口里才带上「上一个状态」——否则会把早已结束的旧缩放也当成障碍，
/// 让选框贴得紧的工程凭空没有落点（实测 `clip-61` 就是这样：稳态 1.65 有 466px 走廊，
/// 强行并上静止态 1.0 后只剩 2.4px）。
pub fn camera_states_for(
    shots: &[CameraShot],
    default_origin: (f64, f64),
    t0: f64,
    t1: f64,
) -> Vec<((f64, f64), f64)> {
    let mut out: Vec<((f64, f64), f64)> = Vec::new();
    let mut push = |o: (f64, f64), z: f64| {
        if !out.iter().any(|(p, q)| *p == o && (*q - z).abs() < 1e-6) {
            out.push((o, z));
        }
    };
    // 某时刻的稳态落点：过渡早已结束就是终点，否则还在起点
    let state_at = |t: f64| {
        shots
            .iter()
            .rev()
            .find(|s| s.time <= t)
            .map(|s| {
                if t >= s.time + s.transition {
                    (s.origin_to, s.zoom)
                } else {
                    (s.origin, s.zoom)
                }
            })
            .unwrap_or((default_origin, 1.0))
    };
    let (o0, z0) = state_at(t0);
    push(o0, z0);
    for (i, s) in shots.iter().enumerate() {
        if s.end <= t0 || s.time >= t1 {
            continue;
        }
        // 过渡与窗口相交时，这一段里出现过的状态是「上一个落点 + 本段起点」：
        // 起点只有过渡期间才在画面上（硬切 transition = 0，窗口看不到起点）。
        if s.time + s.transition > t0 {
            let (po, pz) = shots[..i]
                .iter()
                .rev()
                .find(|p| p.time <= s.time)
                .map(|p| (p.origin_to, p.zoom))
                .unwrap_or((default_origin, 1.0));
            push(po, pz);
            push(s.origin, s.zoom);
        }
        // 落点：无位移时与起点相同（去重后无影响），有位移时是扫过去的另一端
        push(s.origin_to, s.zoom);
    }
    out
}

// ===== 文字图层占位 =====

/// `"12px #403c3b"` → 12.0（无法解析时 0）。
pub fn parse_px(s: &str) -> f64 {
    let head: String = s
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    head.parse().unwrap_or(0.0)
}

/// 字幕带（输出帧固定）：Canvas 一行居中于 `cy=960`；DOM 容器 `bottom:80px` + 行高 1.5 + 内边距。
///
/// 两条渲染路径都取并集，宁可多让一点。`style.subtitle.position` 目前渲染端未消费
/// （只有 `"bottom"` 有实现），所以不区分。
pub fn caption_band(font_size: f64, stroke_px: f64) -> Rect {
    const PAD: f64 = 8.0;
    const BOTTOM_OFFSET: f64 = 80.0;
    const DOM_PAD_V: f64 = 14.0;
    let line_h = font_size * 1.5;
    let dom_top = FRAME_H - BOTTOM_OFFSET - (line_h + DOM_PAD_V * 2.0);
    let canvas_half = font_size * 0.5 + stroke_px * 0.5;
    let top = dom_top.min(960.0 - canvas_half) - PAD;
    let bottom = (FRAME_H - BOTTOM_OFFSET).max(960.0 + canvas_half) + PAD;
    // 横向取 DOM 容器（`#captions`: left 120 / width 1680），居中文字落不出这个盒子。
    Rect::new(120.0, top, 1800.0, bottom)
}

/// 输出帧弹幕带：按模板 `y = (d.y / 1920) * 540` 映射实际轨道高度，取最小/最大轨道并加字高。
/// 没有弹幕（`ys` 为空）→ `None`（此时弹幕图层根本不画）。
pub fn danmaku_band(font_size: f64, stroke_px: f64, ys: &[f64]) -> Option<Rect> {
    const PAD: f64 = 8.0;
    /// 模板 `#danmaku { height:540px }`，超出即被裁掉。
    const LAYER_H: f64 = 540.0;
    if ys.is_empty() {
        return None;
    }
    let mapped: Vec<f64> = ys.iter().map(|y| y / 1920.0 * LAYER_H).collect();
    let lo = mapped.iter().cloned().fold(f64::MAX, f64::min);
    let hi = mapped.iter().cloned().fold(f64::MIN, f64::max);
    let half = font_size * 0.5 + stroke_px * 0.5;
    Some(Rect::new(
        0.0,
        (lo - half).max(0.0) - PAD,
        FRAME_W,
        (hi + half).min(LAYER_H) + PAD,
    ))
}

// ===== 落点 =====

#[derive(Debug, Clone, Copy)]
pub struct Pick {
    /// 贴纸盒子左上角（帧像素）。
    pub x: f64,
    pub y: f64,
    /// 到最近障碍/帧边的距离（越大越“居中”）。
    pub clearance: f64,
    /// 是否退化成「重叠面积最小」兜底。
    pub relaxed: bool,
}

impl Pick {
    pub fn x_pct(&self) -> String {
        format!("{:.0}%", self.x / FRAME_W * 100.0)
    }

    pub fn y_pct(&self) -> String {
        format!("{:.0}%", self.y / FRAME_H * 100.0)
    }
}

/// 贴纸盒子占位（含 floatUp 上浮；碰撞测试与「同刻贴纸互斥」共用）。
pub fn sticker_box(x: f64, y: f64) -> Rect {
    Rect::new(x, y - FLOAT_UP, x + STICKER_SIZE, y + STICKER_SIZE)
}

fn probe(x: f64, y: f64) -> Rect {
    sticker_box(x, y)
}

/// 候选坐标轴：等距网格 + 各障碍边缘的「临界位」（贴着障碍刚好放下）。
/// 只靠 16px 网格会漏掉窄缝里的可行位（实测 `clip-61` 的 2.4px 缝）。
fn axis(min: f64, max: f64, critical: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut out = Vec::new();
    let mut v = min;
    while v <= max + 1e-6 {
        out.push(v);
        v += GRID_STEP;
    }
    for c in critical {
        if c >= min - 1e-6 && c <= max + 1e-6 {
            out.push(c.clamp(min, max));
        }
    }
    out.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    out.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    if out.is_empty() {
        out.push(min);
    }
    out
}

fn center_dist(x: f64, y: f64) -> f64 {
    let dx = x + STICKER_SIZE / 2.0 - FRAME_W / 2.0;
    let dy = y + STICKER_SIZE / 2.0 - FRAME_H / 2.0;
    (dx * dx + dy * dy).sqrt()
}

/// 从可行候选里挑「安全区中心」的一个：clearance 最高档 → 最靠近帧心的前 N 个 → seed 伪随机。
fn central_pick(cands: &[(f64, f64, f64)], seed: f64) -> Pick {
    let best = cands.iter().map(|c| c.2).fold(f64::MIN, f64::max);
    let mut band: Vec<&(f64, f64, f64)> = cands
        .iter()
        .filter(|c| c.2 >= best * CENTRAL_RATIO - 1e-6)
        .collect();
    band.sort_by(|a, b| {
        center_dist(a.0, a.1)
            .partial_cmp(&center_dist(b.0, b.1))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    let n = band.len().min(CENTRAL_PICK);
    let idx = ((seed.clamp(0.0, 0.999_999) * n as f64) as usize).min(n - 1);
    let c = band[idx];
    Pick { x: c.0, y: c.1, clearance: c.2, relaxed: false }
}

/// 推导落点。
///
/// - `blocked`：障碍（人物/字幕/弹幕…）。
/// - `allow`：可选的偏好区域（手写的 `layout.sticker_area` 自由选框）；**只作偏好**，
///   当它里面没有安全位置时自动放弃（安全优先）。
/// - `seed`：0~1 的确定性伪随机数（调用方按 段起点 + 事件序号 生成）。
pub fn pick_sticker_position(blocked: &[Blocked], allow: &[Rect], seed: f64) -> Pick {
    // 临界候选：盒子刚好贴在障碍右侧/下方，或刚好停在左侧/上方
    let xs = axis(
        STICKER_MARGIN,
        FRAME_W - STICKER_MARGIN - STICKER_SIZE,
        blocked
            .iter()
            .flat_map(|b| [b.rect.x1 + TOUCH, b.rect.x0 - STICKER_SIZE - TOUCH]),
    );
    let ys = axis(
        STICKER_MARGIN + FLOAT_UP,
        FRAME_H - STICKER_MARGIN - STICKER_SIZE,
        blocked
            .iter()
            .flat_map(|b| [b.rect.y1 + FLOAT_UP + TOUCH, b.rect.y0 - STICKER_SIZE - TOUCH]),
    );

    let mut free: Vec<(f64, f64, f64)> = Vec::new();
    let mut free_in_allow: Vec<(f64, f64, f64)> = Vec::new();
    let mut best_cost = f64::MAX;
    let mut lowest: Vec<(f64, f64, f64)> = Vec::new(); // (cost, x, y)

    for &x in &xs {
        for &y in &ys {
            let p = probe(x, y);
            let mut clearance = p.gap_to_frame_edge();
            let mut cost = 0.0;
            for b in blocked {
                if b.rect.intersects(&p) {
                    cost += b.kind.weight() * p.intersect_area(&b.rect).max(EPS * EPS);
                    continue;
                }
                let g = p.gap_to(&b.rect);
                if g <= EPS {
                    // 缝隙 <0.5px 也算压住：宁可换位也别贴着障碍边画
                    cost += b.kind.weight() * EPS * EPS;
                } else if g < clearance {
                    clearance = g;
                }
            }
            if cost <= 0.0 {
                free.push((x, y, clearance));
                if allow.is_empty() || allow.iter().any(|a| a.contains_point(p.center().0, p.center().1)) {
                    free_in_allow.push((x, y, clearance));
                }
            } else if cost < best_cost - 1e-9 {
                best_cost = cost;
                lowest.clear();
                lowest.push((cost, x, y));
            } else if (cost - best_cost).abs() <= 1e-9 {
                lowest.push((cost, x, y));
            }
        }
    }

    let pool = if !free_in_allow.is_empty() { &free_in_allow } else { &free };
    if !pool.is_empty() {
        return central_pick(pool, seed);
    }

    // 兜底：所有位置都压着东西 → 取重叠代价最小者，并列时取最靠近帧心的。
    lowest.sort_by(|a, b| {
        center_dist(a.1, a.2)
            .partial_cmp(&center_dist(b.1, b.2))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let n = lowest.len().min(CENTRAL_PICK).max(1);
    let idx = ((seed.clamp(0.0, 0.999_999) * n as f64) as usize).min(n - 1);
    let (_, x, y) = lowest[idx];
    let p = probe(x, y);
    let clearance = blocked
        .iter()
        .map(|b| p.gap_to(&b.rect))
        .fold(p.gap_to_frame_edge(), f64::min);
    Pick { x, y, clearance, relaxed: true }
}

impl Rect {
    fn contains_point(&self, x: f64, y: f64) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band_caption() -> Rect {
        caption_band(72.0, 12.0)
    }

    #[test]
    fn caption_band_covers_canvas_and_dom() {
        let b = band_caption();
        assert!(b.y0 <= 864.0, "DOM 容器上沿 864 必须在带内: {b:?}");
        assert!(b.y1 >= 1002.0, "Canvas 描边下沿 1002 必须在带内: {b:?}");
        assert_eq!(b.x0, 120.0);
        assert_eq!(b.x1, 1800.0);
    }

    #[test]
    fn danmaku_band_tracks_real_lanes() {
        // TRACK_YS 300..1500 → (y/1920)*540 = 84.4..421.9
        let b = danmaku_band(42.0, 2.0, &[300.0, 900.0, 1500.0]).unwrap();
        assert!((b.y0 - (84.375 - 22.0 - 8.0)).abs() < 0.01, "{b:?}");
        assert!((b.y1 - (421.875 + 22.0 + 8.0)).abs() < 0.01, "{b:?}");
        assert_eq!(b.x1, FRAME_W);
        assert!(danmaku_band(42.0, 2.0, &[]).is_none());
    }

    #[test]
    fn camera_expand_scales_around_origin() {
        let r = Rect::new(1028.5, 12.2, 1727.2, 656.4);
        let z = camera_expand(r, 960.0, 540.0, 1.5).clamp_to_frame();
        assert!((z.x0 - 1062.75).abs() < 0.05, "{z:?}");
        assert!((z.y1 - 714.6).abs() < 0.05, "{z:?}");
        assert_eq!(z.y0, 0.0);
    }

    #[test]
    fn camera_states_follow_transitions() {
        let shots = vec![
            CameraShot { time: 0.0, end: 5.0, origin: (960.0, 540.0), origin_to: (960.0, 540.0), zoom: 1.5, transition: TRANSITION_SECS },
            CameraShot { time: 5.0, end: 10.0, origin: (400.0, 540.0), origin_to: (400.0, 540.0), zoom: 1.8, transition: TRANSITION_SECS },
        ];
        // 稳态窗口：运镜早已到位 → 只有当前 shot
        assert_eq!(
            camera_states_for(&shots, (1600.0, 540.0), 2.0, 4.0),
            vec![((960.0, 540.0), 1.5)]
        );
        // 缩放过渡落在窗口里 → 过渡起点也算可见状态
        let st = camera_states_for(&shots, (1600.0, 540.0), 5.2, 7.0);
        assert!(st.contains(&((400.0, 540.0), 1.8)), "{st:?}");
        assert!(st.contains(&((960.0, 540.0), 1.5)), "{st:?}");
        // 开场过渡：静止 1.0 → 1.5
        let st = camera_states_for(&shots, (1600.0, 540.0), 0.1, 2.0);
        assert!(st.contains(&((1600.0, 540.0), 1.0)), "{st:?}");
        assert!(st.contains(&((960.0, 540.0), 1.5)), "{st:?}");
        // 窗口内没有 shot：沿用最后一个运镜状态（渲染端不会自动复位）
        assert_eq!(
            camera_states_for(&shots, (1600.0, 540.0), 20.0, 22.0),
            vec![((400.0, 540.0), 1.8)]
        );
        // 项目里没有任何运镜事件 → 静止
        assert_eq!(
            camera_states_for(&[], (1600.0, 540.0), 1.0, 3.0),
            vec![((1600.0, 540.0), 1.0)]
        );
    }

    /// 带位移的运镜：窗口里出现过的状态是**两端**——人物在过渡期间会一路扫过去，
    /// 贴纸只避开起点或只避开终点都会被扫到。
    #[test]
    fn camera_states_cover_both_ends_of_a_pan() {
        // 0~5s 从主播扫到弹幕面板，之后停在那里
        let shots = vec![CameraShot {
            time: 0.0,
            end: 5.0,
            origin: (1600.0, 540.0),
            origin_to: (400.0, 300.0),
            zoom: 1.5,
            transition: TRANSITION_SECS,
        }];
        let st = camera_states_for(&shots, (960.0, 540.0), 0.1, 2.0);
        assert!(st.contains(&((1600.0, 540.0), 1.5)), "缺位移起点: {st:?}");
        assert!(st.contains(&((400.0, 300.0), 1.5)), "缺位移终点: {st:?}");

        // 过渡结束之后：稳态是落点，不再是起点
        assert_eq!(
            camera_states_for(&shots, (960.0, 540.0), 3.0, 4.0),
            vec![((400.0, 300.0), 1.5)]
        );

        // 下一段的「上一状态」要取上一段的**落点**，否则位移结束后的稳态会算错
        let mut two = shots.clone();
        two.push(CameraShot {
            time: 5.0,
            end: 8.0,
            origin: (200.0, 900.0),
            origin_to: (200.0, 900.0),
            zoom: 1.8,
            transition: TRANSITION_SECS,
        });
        let st = camera_states_for(&two, (960.0, 540.0), 5.2, 7.0);
        assert!(st.contains(&((400.0, 300.0), 1.5)), "上一段落点应参与避让: {st:?}");
        assert!(st.contains(&((200.0, 900.0), 1.8)), "{st:?}");
    }

    /// 远景进出是硬切：那一段**没有**过渡窗口，所以窗口里不该凭空多出一个早已结束的旧状态。
    #[test]
    fn wide_shot_is_a_hard_cut_with_no_transition_window() {
        assert_eq!(transition_secs("远", "中", false), 0.0);
        assert_eq!(transition_secs("中", "远", false), 0.0, "退出远景同样是切");
        assert_eq!(transition_secs("近", "远", false), 0.0);
        assert_eq!(transition_secs("远", "远", false), 0.0);
        assert_eq!(transition_secs("中", "近", false), TRANSITION_SECS, "普通档位仍走缓动");
        // 硬切（编辑器「方式 = 硬切」）：任意档位都能瞬间到位，不只有画中画
        assert_eq!(transition_secs("中", "近", true), 0.0);
        assert_eq!(transition_secs("近", "中", true), 0.0);

        // 0.5s 处在「刚切到远景」之后：窗口 [0.5, 2.0] 看不到切走之前的旧状态
        let shots = vec![
            CameraShot {
                time: 0.0,
                end: 0.5,
                origin: (1600.0, 540.0),
                origin_to: (1600.0, 540.0),
                zoom: 1.15,
                transition: TRANSITION_SECS,
            },
            CameraShot {
                time: 0.5,
                end: 3.0,
                origin: (960.0, 540.0),
                origin_to: (960.0, 540.0),
                zoom: 0.55,
                transition: transition_secs("远", "中", false),
            },
        ];
        let st = camera_states_for(&shots, (1600.0, 540.0), 0.5, 2.0);
        assert_eq!(st, vec![((960.0, 540.0), 0.55)], "硬切不该留下旧状态: {st:?}");

        // 对照：普通档位在同样的时间关系下会把旧状态算进来（过渡还在进行）
        let soft = vec![
            CameraShot { time: 0.0, end: 0.5, origin: (1600.0, 540.0), origin_to: (1600.0, 540.0), zoom: 1.15, transition: TRANSITION_SECS },
            CameraShot { time: 0.5, end: 3.0, origin: (960.0, 540.0), origin_to: (960.0, 540.0), zoom: 1.3, transition: TRANSITION_SECS },
        ];
        let st = camera_states_for(&soft, (1600.0, 540.0), 0.5, 2.0);
        assert!(st.contains(&((1600.0, 540.0), 1.15)), "缓动下旧状态可见: {st:?}");
    }

    #[test]
    fn picks_center_of_corridor_not_edges() {
        // 左右两堵墙 + 上下字幕/弹幕：安全区是一条竖直走廊，落点应贴在走廊中心线上
        let blocked = vec![
            Blocked::new(Rect::new(0.0, 0.0, 640.0, 1080.0), ObstacleKind::DanmakuArea),
            Blocked::new(Rect::new(1060.0, 0.0, 1920.0, 716.0), ObstacleKind::Liver),
            Blocked::new(band_caption(), ObstacleKind::Captions),
        ];
        for seed in [0.0, 0.3, 0.6, 0.99] {
            let p = pick_sticker_position(&blocked, &[], seed);
            assert!(!p.relaxed);
            // 盒子完全落在走廊里（640..1060）
            assert!(p.x >= 640.0 && p.x + STICKER_SIZE <= 1060.0, "seed {seed}: {p:?}");
            // 走廊中心 850 → 盒心最多偏离 ~64px（中心档 8 个候选）
            assert!((p.x + 140.0 - 850.0).abs() <= 80.0, "seed {seed}: {p:?}");
            // 竖直方向取“安全区中心”，落在中带而不是贴顶/贴字幕
            assert!(p.y >= 180.0 && p.y <= 500.0, "seed {seed}: {p:?}");
        }
    }

    #[test]
    fn picks_far_from_frame_edge_when_space_allows() {
        let blocked = vec![Blocked::new(band_caption(), ObstacleKind::Captions)];
        let p = pick_sticker_position(&blocked, &[], 0.5);
        assert!(!p.relaxed);
        // 帧心就是最优：盒心几乎落在 (960, 540)
        assert!((p.x + 140.0 - 960.0).abs() <= 40.0, "{p:?}");
        assert!((p.y + 140.0 - 540.0).abs() <= 80.0, "{p:?}");
        assert!(p.x >= STICKER_MARGIN && p.y >= STICKER_MARGIN);
    }

    #[test]
    fn respects_allow_rect_but_never_sacrifices_safety() {
        let blocked = vec![Blocked::new(
            Rect::new(0.0, 0.0, 1000.0, 1080.0),
            ObstacleKind::Liver,
        )];
        // 偏好区落在人物上 → 放弃偏好，改去右侧空白
        let allow = vec![Rect::new(100.0, 300.0, 600.0, 800.0)];
        let p = pick_sticker_position(&blocked, &allow, 0.4);
        assert!(!p.relaxed);
        assert!(p.x >= 1000.0, "{p:?}");
    }

    #[test]
    fn falls_back_to_least_overlap_when_frame_is_full() {
        let blocked = vec![Blocked::new(
            Rect::new(0.0, 0.0, FRAME_W, FRAME_H),
            ObstacleKind::Liver,
        )];
        let p = pick_sticker_position(&blocked, &[], 0.1);
        assert!(p.relaxed);
        assert!(p.x >= STICKER_MARGIN && p.y >= STICKER_MARGIN);
    }

    #[test]
    fn seed_is_deterministic() {
        let blocked = vec![Blocked::new(band_caption(), ObstacleKind::Captions)];
        let a = pick_sticker_position(&blocked, &[], 0.42);
        let b = pick_sticker_position(&blocked, &[], 0.42);
        assert_eq!((a.x, a.y), (b.x, b.y));
    }

    /// 真实工程几何回归：`media/[2026-09-06 20-39-39-210][T T]/output/clips/clip-61`
    /// （liver 53.57/1.13–89.96/60.78、danmaku_area 17.01/24.35–38.86/74.92、
    /// 每段 Camera(zoom=中, origin 50% 50%)、字幕 72/12px、该片无弹幕）。
    /// 期望：落到「弹幕面板」与「人物」之间的竖直走廊中心，而不是贴帧边。
    fn clip61_obstacles(states: &[((f64, f64), f64)], caption: Rect) -> (Vec<Blocked>, f64, f64) {
        let liver = Rect::new(0.5357 * FRAME_W, 0.0113 * FRAME_H, 0.8996 * FRAME_W, 0.6078 * FRAME_H);
        let panel = Rect::new(0.1701 * FRAME_W, 0.2435 * FRAME_H, 0.3886 * FRAME_W, 0.7492 * FRAME_H);
        let mut blocked = vec![Blocked::new(caption, ObstacleKind::Captions)];
        let (mut corridor_l, mut corridor_r) = (0.0f64, FRAME_W);
        for (o, z) in states {
            let l = camera_expand(liver, o.0, o.1, *z).clamp_to_frame();
            let p = camera_expand(panel, o.0, o.1, *z).clamp_to_frame();
            corridor_l = corridor_l.max(p.x1);
            corridor_r = corridor_r.min(l.x0);
            blocked.push(Blocked::new(l, ObstacleKind::Liver));
            blocked.push(Blocked::new(p, ObstacleKind::DanmakuArea));
        }
        (blocked, corridor_l, corridor_r)
    }

    #[test]
    fn real_project_clip61_lands_centered_in_corridor() {
        let shots = vec![CameraShot {
            time: 0.0,
            end: 60.0,
            origin: (960.0, 540.0),
            origin_to: (960.0, 540.0),
            zoom: crate::project::CameraStyle::default().factor("中") * TRANSIENT_ZOOM,
            transition: TRANSITION_SECS,
        }];
        // 片中的贴纸：运镜早已到位 → 只需避开稳态
        let states = camera_states_for(&shots, (1600.0, 540.0), 10.0, 12.5);
        assert_eq!(states.len(), 1, "{states:?}");
        let (blocked, l, r) = clip61_obstacles(&states, band_caption());
        assert!(r - l >= STICKER_SIZE, "走廊 {l}~{r} 放不下贴纸");
        for seed in [0.0, 0.25, 0.5, 0.75, 0.99] {
            let p = pick_sticker_position(&blocked, &[], seed);
            assert!(!p.relaxed, "seed {seed}: {p:?}");
            assert_no_overlap(&blocked, &p, seed);
            assert!(p.x >= l && p.x + STICKER_SIZE <= r, "seed {seed}: {p:?}");
            let (sl, sr) = (p.x - l, r - (p.x + STICKER_SIZE));
            assert!((sl - sr).abs() <= 2.0 * GRID_STEP, "seed {seed} 贴边: {sl} vs {sr}");
            assert!(p.y + STICKER_SIZE <= 856.0, "seed {seed} 压字幕: {p:?}");
        }
    }

    #[test]
    fn clip61_opening_transition_still_finds_a_slot() {
        let shots = vec![CameraShot {
            time: 0.0,
            end: 60.0,
            origin: (960.0, 540.0),
            origin_to: (960.0, 540.0),
            zoom: crate::project::CameraStyle::default().factor("中") * TRANSIENT_ZOOM,
            transition: TRANSITION_SECS,
        }];
        // 开场 0.2s：缩放过渡（1.0 → 1.65）与窗口相交 → 两个状态都要避让
        let states = camera_states_for(&shots, (1600.0, 540.0), 0.2, 2.7);
        assert_eq!(states.len(), 2, "{states:?}");
        let (blocked, l, r) = clip61_obstacles(&states, band_caption());
        // 面板右沿(746.1) 与人物左沿(1028.5) 之间只剩 ~2.4px：靠临界候选仍能放下
        assert!(r - l >= STICKER_SIZE, "走廊 {l}~{r}");
        let p = pick_sticker_position(&blocked, &[], 0.5);
        assert!(!p.relaxed, "{p:?}");
        assert_no_overlap(&blocked, &p, 0.5);
        assert!(p.x >= l - TOUCH && p.x + STICKER_SIZE <= r + TOUCH, "{p:?}");
    }

    fn assert_no_overlap(blocked: &[Blocked], p: &Pick, seed: f64) {
        let box_ = sticker_box(p.x, p.y);
        for b in blocked {
            assert!(!box_.intersects(&b.rect), "seed {seed} 压住 {:?}: {p:?}", b.kind);
        }
    }
}
