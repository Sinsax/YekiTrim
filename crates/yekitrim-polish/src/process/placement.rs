//! 贴纸落点推导：从 **人物 / 字幕 / 弹幕** 反选安全区，并在安全区**中心**撒点。
//!
//! 坐标系与渲染端一致（`templates/index.html` 的 Canvas，1920×1080 整帧）：
//! - `Event::Sticker` 的 x/y 是**贴纸盒子左上角**百分比 —— 模板 DOM
//!   (`index.html` 的 `el.style.left/top`) 与 `webui/src/polish/engine.js::stickersAt`
//!   都按左上角消费；盒子边长 `STICKER_SIZE × sticker_scale`（见 [`Frame::sticker_scale`]）。
//! - ⚠️ **`[sticker] position`（manual 固定落点）不是这个口径**：它是**盒子中心**百分比
//!   （`project/config.rs` 的 `StickerConfig::position` 注释），两条渲染路径的
//!   `stickerAnchor` 在 manual 分支里减去半个盒子换成左上角，本模块不参与 manual。
//!   两者是**不同的键**、不同的基准 —— 别顺手"统一"成一种（GPU 路径
//!   `yekitrim-render-core/src/timeline.rs` 曾按中心读事件 x/y，与 DOM 差半个盒子）。
//! - 渲染端把 left/top 钳制在 `[margin, W-margin-sticker] × [margin, H-margin-sticker]`，
//!   本模块的候选网格限制在同一范围内（`STICKER_MARGIN`）。
//!
//! 障碍来源（渲染顺序：视频 → 字幕 → 弹幕 → 贴纸，贴纸画在最上层，压住谁就是挡住谁）：
//! - **人物** `layout.liver`：在**源帧**坐标里框选（选框模式下运镜暂停、缩放复位），
//!   输出里它要经**两步**变换才落到画布上（顺序与 `renderFrame` 的 CTM 一致）：
//!   ① 源帧 → **主画面矩形 `sr`**（[`stage_rect`]；`layout.<区域>` 的百分比是 `sr` 的百分比，
//!   **W1 口径** —— 见 [`crate::project::stage_draw_rect`]）；② 再按相机变换绕不动点缩放
//!   （`index.html:1439-1463`；不动点同样是 `sr` 的百分比，模板 `:1888-1889` 把它映射进 `sr`）。
//!   ⚠️ **漏掉 ①就是「障碍算的位置与成片不一致」**：4:3 源进 16:9 画布时真障碍 `[384,816]`、
//!   按整帧读是 `[192,768]`（实测差 192px，临界候选会压住主播）。
//! - **弹幕区域** `layout.danmaku_area`：同上，源帧坐标 → `sr` → 相机变换。
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

/// 输出帧尺寸基准（横画布）。竖画布（1080×1920）见 [`Frame`]；
/// 模板 Canvas 已改为从 `_canvas` 读宽高，这里的常量只作 **k/kh 缩放的基准**。
pub const FRAME_W: f64 = 1920.0;
pub const FRAME_H: f64 = 1080.0;
/// 贴纸盒子基准边长（模板 `.sticker-img` = `280 × sticker_scale`）。
pub const STICKER_SIZE: f64 = 280.0;
/// 贴纸安全边距基准（渲染端 clamp 的 `32 × sticker_scale` / `engine.js` 同源）。
pub const STICKER_MARGIN: f64 = 32.0;
/// 贴纸 floatUp 动画的基准上浮量（渲染端 `yOff = -18 × sticker_scale`），测试障碍时算进去。
pub const FLOAT_UP: f64 = 18.0;

/// 输出画布（`[canvas] orientation` 唯一推导，横 1920×1080 / 竖 1080×1920）。
///
/// 本模块所有几何都以 `Frame` 为坐标系：**贴纸**像素量（盒、边距、上浮）按
/// [`Frame::sticker_scale`] = **短边/1080** 缩放（横竖两种画布的短边都是 1080 →
/// 贴纸绝对像素相同），弹幕带高按 `k = W/1920`，纵向锚点（字幕底距）按 `kh = H/1080`
/// —— 横画布下三者皆 = 1，与改造前的硬编码逐像素一致。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub w: f64,
    pub h: f64,
}

impl Frame {
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            w: w as f64,
            h: h as f64,
        }
    }

    /// 横画布基准帧（1920×1080）—— **只有单测用**（正常运行时的 Frame 由画布尺寸构造），
    /// 所以 `#[cfg(test)]`：留在非测试构建里就是一个 dead_code 警告（仓库规矩是 0 warnings）。
    #[cfg(test)]
    pub fn landscape() -> Self {
        Self::new(1920, 1080)
    }

    /// 宽度基准比例 `W/1920`。
    pub fn k(&self) -> f64 {
        self.w / FRAME_W
    }

    /// 高度基准比例 `H/1080`。
    pub fn kh(&self) -> f64 {
        self.h / FRAME_H
    }

    /// **贴纸**缩放基准：短边 / 1080 —— 横竖画布下贴纸的绝对像素相同。
    ///
    /// 横 1920×1080 → 1（280px，零回归）；竖 1080×1920 → 1（**也是 280px**）。
    /// ⚠️ 竖屏别再退回 `k() = W/1920`（= 0.5625 → 157.5px）：贴纸会明显偏小，
    /// 而同一画布上的字幕/弹幕字号本来就不随画布缩放（决策表第 8 条），
    /// 于是贴纸成了唯一"竖屏缩水"的元素 —— 用户报的就是这个。
    pub fn sticker_scale(&self) -> f64 {
        self.w.min(self.h) / FRAME_H
    }

    /// 贴纸盒边长（渲染端 `.sticker-img` = `280 × sticker_scale`，两处必须同源）。
    pub fn sticker_size(&self) -> f64 {
        STICKER_SIZE * self.sticker_scale()
    }

    /// 贴纸安全边距（渲染端 clamp 的 `32 × sticker_scale`）。
    pub fn margin(&self) -> f64 {
        STICKER_MARGIN * self.sticker_scale()
    }

    /// floatUp 上浮量（渲染端 `yOff = -18 × sticker_scale`）。
    pub fn float_up(&self) -> f64 {
        FLOAT_UP * self.sticker_scale()
    }
}

/// 一个**视觉元素**在落点求解时占的盒子（左上角 + 宽高，像素）。
///
/// # 为什么要有这个类型（§5 E3 的地基）
///
/// 求解器原来到处用 `f.sticker_size()`（= `280 × sticker_scale`）—— 那是**贴纸**的尺寸。
/// 文字是**变宽高**的（字号 + 折行 → 可能 520×120，也可能 300×240），
/// 沿用 280×280 的症状是**"大字压住主播脸"**：求解器以为只占了 280 宽，实际画出去一大片。
///
/// ⚠️ **`h` 与 `w` 分开**，不是"正方形"：文字盒明显是扁的。
/// 用正方形近似会让纵向预测偏大（文字实际只占一行高，却按宽度那么多去避让）
/// ⇒ 能放下的位置被判成放不下，落点被迫跑到角落。
///
/// ⚠️ **`float_up` 也在这里**：它是"入场动画往上飘"的额外占位，只有贴纸有
/// （文字的动效另有其法，见规格 §4）。文字传 0 即可。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxSize {
    pub w: f64,
    pub h: f64,
    /// 入场动画的额外上浮占位（贴纸的 `floatUp`；文字传 0）。
    pub float_up: f64,
}

impl BoxSize {
    /// 贴纸盒（**历史行为**：`280 × sticker_scale` 见方 + `18 × sticker_scale` 上浮）。
    ///
    /// 这条是**默认路径**，`pick_sticker_position` 的就是它 ⇒ 贴纸落点逐像素不变（R4）。
    pub fn sticker(f: Frame) -> Self {
        Self { w: f.sticker_size(), h: f.sticker_size(), float_up: f.float_up() }
    }

    /// 文字盒：按**排版算出来的**实际宽高（无入场上浮）。
    ///
    /// ⚠️ **目前只有单测在用它** —— E3 这一步交付的是"求解器支持动态盒"这个**能力**，
    /// 真正的调用方（把 `elements.text` 的排版结果接进来）在 E3 的下一步。
    /// 挂 `cfg(test)` 而不是压一个 `#[allow(dead_code)]`：后者会让它悄悄烂在那里
    /// （仓库规矩是 0 warnings，而这个 warning 正是"还没接上"的提醒）。
    /// 接上生产调用点时把这个属性摘掉即可。
    #[cfg(test)]
    pub fn text(w: f64, h: f64) -> Self {
        Self { w, h, float_up: 0.0 }
    }
}

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

/// 近景档位的档位名 —— 手持摇摆（`handheld`）唯一可用的景别，见 `process::fx::cap_handheld_sway`。
pub const CLOSE_LABEL: &str = "近";

/// **特写**档位的档位名（规格 `camera-and-noise-spec.md` §3.3 方案 A，系数 1.8）。
///
/// ⚠️ 它与 `近` 是**两个独立档位**（1.8 vs 1.5），别把 `特写` 当成 `近` 的同义词 ——
/// 手持摇摆仍然只允许 `近`（用户口径没变），特写不继承它。
pub const CLOSEUP_LABEL: &str = "特写";

/// 这一段运镜的过渡时长：**硬切条件**（两端任一为远景「画中画」，或本事件自带 `cut`）
/// 时是 **0 = 硬切**，否则 0.6s 缓动。
///
/// 远景进出是「切」不是「推」——缓动缩到画中画看着像镜头在拉远，而它要的是喜剧式的切换。
/// `cut`（编辑器里的「方式 = 硬切」）把同一条规则放开给任意景别。
/// 模板 canvas、GSAP 与 `engine.js` 三处同步实现同一条规则；贴纸避让靠这个值知道
/// 那一段根本没有过渡窗口（否则会凭空多算出一个早已结束的旧状态当障碍）。
/// 运镜过渡时长（秒）—— **带上 §16 的 `speed`**。
///
/// ⚠️ 这里的口径必须与**求值器**一致，否则贴纸避让会算错障碍窗口：
/// 求值器（模板 `getActiveCamera` / `engine.js cameraAt` / 转译器）用的是
/// `cameraTransitionSecs(ev, dur, hardCut)` ——
/// `snap 0.1 / fast 0.25 / med 老式子 / slow min(1.5, dur×0.8)`。
/// **在这条路径上照抄一份定值会在写了 `speed` 的工程里偏**（偏了的表现是
/// "贴纸偶尔压在正在运动的画面上"—— 不报错、只有逐帧看才发现）。
///
/// 三档「硬切」（`cut` / 本档是远 / 上一档是远）一律 0、**优先于 `speed`**，
/// 与求值器 `hardCut` 的判据同源。
///
/// `speed = None` 时**逐值等于**改动前的历史行为（= [`TRANSITION_SECS`]）——
/// 存量工程（今天全都没写 `speed`）走的就是这一档。
pub fn transition_secs(
    label: &str,
    prev_label: &str,
    cut: bool,
    speed: Option<&str>,
    dur: f64,
) -> f64 {
    if cut || label.trim() == WIDE_LABEL || prev_label.trim() == WIDE_LABEL {
        return 0.0;
    }
    // 与 `webui/src/polish/engine.js::cameraTransitionSecs` 同一条分支：
    // `med` / 没写 / 认不出 ⇒ 老式子；`snap`/`fast` 固定；`slow` 夹到 `dur × 0.8`。
    match speed.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("snap") => 0.10,
        Some("fast") => 0.25,
        Some("slow") => 1.5_f64.min(dur * 0.8),
        _ => TRANSITION_SECS,
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

    // 这里曾有个 `Rect::from_points(tl, br, f)`（源帧百分比 → **整帧**像素）。
    // W1 起 `.layout.<区域>` 的百分比是**主画面矩形 `sr`** 的百分比，整帧读就是那个
    // 「障碍与成片不一致」的旧口径 —— 已删除，一律走 `stage_region_rect(tl, br, sr)`。

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
    pub fn gap_to_frame_edge(&self, f: Frame) -> f64 {
        self.x0.min(self.y0).min(f.w - self.x1).min(f.h - self.y1)
    }

    pub fn center(&self) -> (f64, f64) {
        ((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }

    /// 裁剪到帧内（相机变换会把矩形推到帧外）。
    pub fn clamp_to_frame(&self, f: Frame) -> Self {
        Self::new(
            self.x0.max(0.0).min(f.w),
            self.y0.max(0.0).min(f.h),
            self.x1.max(0.0).min(f.w),
            self.y1.max(0.0).min(f.h),
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
    /// 主画面（视频层）在画布上实际占的那块 —— 只在视频**没铺满**画布时才是障碍
    /// （`axis = x/y`；铺满时见 `main_frame_rect` 的说明：那时加它等于处处不安全）。
    MainFrame,
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
            Self::MainFrame => "主画面",
            Self::Sticker => "贴纸",
        }
    }

    /// 兜底（无安全位置时）重叠代价权重。
    fn weight(self) -> f64 {
        match self {
            Self::Liver => 3.0,
            Self::Captions => 3.0,
            // 略低于 Liver：非要压的话，压在主画面的**非人物区域**比压住主播好
            Self::MainFrame => 2.5,
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

/// 解析 `"50% 50%"` 形式的**源帧**百分比中心 → 画布坐标（`sr` = 主画面矩形，见 [`stage_rect`]）。
///
/// **W1 口径**：运镜不动点百分比是**主画面矩形 `sr`** 的百分比，与模板一致 ——
/// `renderFrame` 里 `anchorX = sr.x + cam.ox/CW*sr.w`（`index.html:1888-1889`）。
pub fn parse_origin_in_stage(s: &str, sr: Rect) -> Option<(f64, f64)> {
    let mut it = s.split_whitespace();
    let x = parse_pct(it.next()?);
    let y = parse_pct(it.next()?);
    Some((
        sr.x0 + x / 100.0 * sr.width(),
        sr.y0 + y / 100.0 * sr.height(),
    ))
}

// 这里曾有个 `parse_origin(s, f)`（`"50% 50%"` → **整帧**像素）。W1 起运镜不动点百分比是
// 主画面矩形 `sr` 的百分比（模板 `index.html:1888-1889` 就是 `sr.x + pct/100*sr.w`），
// 整帧读同样会算错位置 —— 已删除，一律走 `parse_origin_in_stage(s, sr)`。

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

/// 字幕带（输出帧随画布参数化）：Canvas 一行居中于 `cy = H - 120×kh`；
/// DOM 容器 `bottom: 80×kh` + 行高 1.5 + 内边距；横向 = `#captions` 的
/// `left = W×0.0625 / width = W×0.875`（A5 的 maxLineW，同一个常量）。
///
/// 两条渲染路径都取并集，宁可多让一点。`style.subtitle.position` 目前渲染端未消费
/// （只有 `"bottom"` 有实现），所以不区分。
pub fn caption_band(f: Frame, font_size: f64, stroke_px: f64) -> Rect {
    const PAD: f64 = 8.0;
    /// 横画布基准底距（渲染端 80×kh）。
    const BOTTOM_OFFSET: f64 = 80.0;
    /// 基准基线高度（渲染端 Canvas `cy = H - 120×kh`）。
    const BASELINE_INSET: f64 = 120.0;
    const DOM_PAD_V: f64 = 14.0;
    let (bottom_off, baseline_inset) = (BOTTOM_OFFSET * f.kh(), BASELINE_INSET * f.kh());
    let line_h = font_size * 1.5;
    let dom_top = f.h - bottom_off - (line_h + DOM_PAD_V * 2.0);
    let cy = f.h - baseline_inset;
    let canvas_half = font_size * 0.5 + stroke_px * 0.5;
    let top = dom_top.min(cy - canvas_half) - PAD;
    let bottom = (f.h - bottom_off).max(cy + canvas_half) + PAD;
    // 横向取 DOM 容器（`#captions`），居中文字落不出这个盒子。
    Rect::new(f.w * 0.0625, top, f.w * 0.9375, bottom)
}

/// 输出帧弹幕带：按模板 `y = (d.y / 1920) * (540×k)` 映射实际轨道高度，
/// 取最小/最大轨道并加字高。没有弹幕（`ys` 为空）→ `None`（此时弹幕图层根本不画）。
pub fn danmaku_band(f: Frame, font_size: f64, stroke_px: f64, ys: &[f64]) -> Option<Rect> {
    const PAD: f64 = 8.0;
    /// 模板 `#danmaku { height: 540×k px }`，超出即被裁掉。
    fn layer_h(f: Frame) -> f64 {
        540.0 * f.k()
    }
    if ys.is_empty() {
        return None;
    }
    let layer_h = layer_h(f);
    let mapped: Vec<f64> = ys.iter().map(|y| y / 1920.0 * layer_h).collect();
    let lo = mapped.iter().cloned().fold(f64::MAX, f64::min);
    let hi = mapped.iter().cloned().fold(f64::MIN, f64::max);
    let half = font_size * 0.5 + stroke_px * 0.5;
    Some(Rect::new(
        0.0,
        (lo - half).max(0.0) - PAD,
        f.w,
        (hi + half).min(layer_h) + PAD,
    ))
}

// ===== 主画面 =====

/// **主画面矩形 `sr`**（画布 px，可越出画布）：源帧经 `[stage]` 摆放/缩放后落的那块。
///
/// 它是 **W1 坐标系的唯一入口** —— `layout.<区域>` 的百分比、运镜不动点百分比都是它的百分比。
/// 几何本身**不在这里算**：唯一真源是模板 `stageDrawRect`，Rust 镜像 =
/// [`crate::project::stage_draw_rect`]（逐式一致，含 padding 夹取与绕盒心缩放）。
///
/// `src` 拿不到（ffprobe 失败）时用**画布自身**当源 —— `resolve_stage(None)` 此时给 `axis=fill`，
/// 而按画布比例铺满在 `padding=0 / scale=1`（绝大多数工程）下与**整块画布**逐值相同，
/// 所以这是「拿不到源尺寸就退化成改造前行为」的那条降级路，不是新语义。
pub fn stage_rect(
    stage: &crate::project::StageFit,
    src: Option<(u32, u32)>,
    canvas: (u32, u32),
) -> Rect {
    let src = match src {
        Some((w, h)) if w > 0 && h > 0 => (w, h),
        _ => canvas,
    };
    let r = crate::project::stage_draw_rect(stage, src, canvas);
    Rect::new(r.x, r.y, r.x + r.w, r.y + r.h)
}

/// `layout.<区域>`（**源帧**百分比矩形）→ 画布矩形：百分比是**主画面矩形 `sr`** 的百分比。
///
/// 与四个渲染端同口径（模板 `liverOrigin` / `regionOrigin` 的消费方式、`engine.js`、
/// `PolishPreview.vue`、转译器都是 `sr.x + pct/100*sr.w`）。**W1 前这里按整帧读**，
/// `sr ≠ 整块画布`（源比例≠画布 / `scale≠1` / `padding≠0`）时三方不一致。
///
/// 优先级不变，仍走 [`crate::project::region_rects`]：自由选框 > 九宫格格子。
pub fn layout_region_rects(region: &crate::project::LayoutRegion, sr: Rect) -> Vec<Rect> {
    crate::project::region_rects(region)
        .iter()
        .map(|(tl, br)| stage_region_rect(tl, br, sr))
        .collect()
}

/// 源帧百分比角点 → 画布矩形（W1：百分比是 `sr` 的百分比）。见 [`layout_region_rects`]。
pub fn stage_region_rect(tl: &Point, br: &Point, sr: Rect) -> Rect {
    Rect::new(
        sr.x0 + parse_pct(&tl.x) / 100.0 * sr.width(),
        sr.y0 + parse_pct(&tl.y) / 100.0 * sr.height(),
        sr.x0 + parse_pct(&br.x) / 100.0 * sr.width(),
        sr.y0 + parse_pct(&br.y) / 100.0 * sr.height(),
    )
}

/// 主画面矩形的**纯几何**（坐标系 = `canvas`，**未**做整帧钳制）。`axis = fill` 返回 `None`。
///
/// 直接委托 [`crate::project::stage_draw_rect`] —— **不再有第二份公式**。历史上这里自己算过一遍，
/// 与模板有两处不一致：① `scale × position` 的合成顺序反了（先缩放再插值，`pct=0/scale=2` 差
/// **303.75px**，见 ⑤ 4b）；② padding 走 `aw<=0 → None` 而不是模板的 `cap = min(cw,ch)/2-1` 夹取。
/// 现在两个偏差都由 SSOT 一处决定。
fn main_frame_geometry(
    stage: &crate::project::StageFit,
    src: Option<(u32, u32)>,
    canvas: (u32, u32),
) -> Option<Rect> {
    if stage.axis != "x" && stage.axis != "y" {
        return None;
    }
    let (sw, sh) = src?;
    if sw == 0 || sh == 0 || canvas.0 == 0 || canvas.1 == 0 {
        return None;
    }
    let r = crate::project::stage_draw_rect(stage, (sw, sh), canvas);
    Some(Rect::new(r.x, r.y, r.x + r.w, r.y + r.h))
}

/// 把主画面当作贴纸障碍时的矩形；不需要避让时返回 `None`。
///
/// 本模块的其它障碍（人物 / 弹幕区域选框、字幕带、弹幕带）都在 1920×1080 整帧坐标系里，
/// 隐含假设「视频铺满画布」。横画布 × 竖素材（`axis = x`）时视频其实只占一侧一条，
/// 于是 `auto` 会把贴纸放到主视频身上 —— 把这块矩形也当障碍，auto 才算真的自动避让。
///
/// **`axis = fill` 时返回 `None`**（视频铺满画布，含改造前的全部横素材项目）：
/// 整帧都是画面，拿它当障碍等价于「处处不安全」，只会把所有贴纸推进 relaxed 兜底，
/// 反而变差。那种情况下的避让本来就由 `layout.liver` 选框负责 → 行为与改造前一致。
///
/// 几何口径 = **模板** `templates/index.html::stageDrawRect`（经 [`crate::project::stage_draw_rect`]）：
/// 内容盒 = 画布去掉四周留白，`axis=x` 按高度铺满/`axis=y` 按宽度铺满，`position_pct` 沿留边轴定位，
/// **最后**才由 `scale` 绕盒心放大（顺序不能反，见 [`crate::project::stage_draw_rect`]）。
pub fn main_frame_rect(
    stage: &crate::project::StageFit,
    src: Option<(u32, u32)>,
    canvas: (u32, u32),
) -> Option<Rect> {
    let f = Frame::new(canvas.0, canvas.1);
    let rect = main_frame_geometry(stage, src, canvas)?.clamp_to_frame(f);
    if rect.is_degenerate() {
        return None;
    }
    // 只在「画面上还剩下能放下一张贴纸的地方」时才把它当障碍。
    //
    // 这一条不能省：主画面几乎铺满但没完全铺满时（例如 `scale = 3` 只剩 48px 侧边、
    // 或 4:3 源显式居中只剩 240px 边），加上障碍会让**所有**候选都相交 →
    // 每张贴纸都退化到 relaxed 兜底，结果比不加还差。
    // 阈值口径与候选网格一致：可用的左沿范围是
    // `[margin, W - sticker - margin]`，所以一条边带至少要 `sticker + margin` 才放得下
    // （`TOUCH` 再放宽 1px）。
    let need = f.sticker_size() + f.margin() - TOUCH;
    let bands = [rect.x0, f.w - rect.x1, rect.y0, f.h - rect.y1];
    if !bands.iter().any(|b| *b >= need) {
        return None;
    }
    Some(rect)
}

// ===== 落点 =====

/// 落点结果。`PartialEq` 是给"重构前后逐点相同"那条判据用的（贴纸路径的 R4 保证）。
#[derive(Debug, Clone, Copy, PartialEq)]
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
    pub fn x_pct(&self, f: Frame) -> String {
        format!("{:.0}%", self.x / f.w * 100.0)
    }

    pub fn y_pct(&self, f: Frame) -> String {
        format!("{:.0}%", self.y / f.h * 100.0)
    }
}

/// 贴纸盒子占位（含 floatUp 上浮；碰撞测试与「同刻贴纸互斥」共用）。
pub fn sticker_box(x: f64, y: f64, f: Frame) -> Rect {
    box_at(x, y, &BoxSize::sticker(f))
}

/// 把「左上角 + 盒子」变成矩形。
///
/// ⚠️ **`y - float_up` 是有意的**：盒子的**视觉**左上角是 `y`，
/// 但入场动画期间它还会往上飘 `float_up`，所以避让时要连飘上去的那块一起算。
/// 文字盒的 `float_up = 0` ⇒ 退化成 `Rect::new(x, y, x+w, y+h)`。
pub fn box_at(x: f64, y: f64, b: &BoxSize) -> Rect {
    Rect::new(x, y - b.float_up, x + b.w, y + b.h)
}

fn probe(x: f64, y: f64, b: &BoxSize) -> Rect {
    box_at(x, y, b)
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

/// 盒子中心到帧心的距离（用于"挑最靠近画面中心的那一个"）。
///
/// ⚠️ 盒子**宽高分开**：`b.w / b.h` 而不是同一个数。文字盒是扁的，
/// 用宽度当高会让"哪个候选更居中"的判断整体偏移 —— 那不会报错，只会让落点看着"偏了一点"。
fn center_dist(x: f64, y: f64, f: Frame, b: &BoxSize) -> f64 {
    let dx = x + b.w / 2.0 - f.w / 2.0;
    let dy = y + b.h / 2.0 - f.h / 2.0;
    (dx * dx + dy * dy).sqrt()
}

/// 从可行候选里挑「安全区中心」的一个：clearance 最高档 → 最靠近帧心的前 N 个 → seed 伪随机。
fn central_pick(cands: &[(f64, f64, f64)], seed: f64, f: Frame, box_: &BoxSize) -> Pick {
    let best = cands.iter().map(|c| c.2).fold(f64::MIN, f64::max);
    let mut band: Vec<&(f64, f64, f64)> = cands
        .iter()
        .filter(|c| c.2 >= best * CENTRAL_RATIO - 1e-6)
        .collect();
    band.sort_by(|p, q| {
        center_dist(p.0, p.1, f, box_)
            .partial_cmp(&center_dist(q.0, q.1, f, box_))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(p.0.partial_cmp(&q.0).unwrap_or(std::cmp::Ordering::Equal))
            .then(p.1.partial_cmp(&q.1).unwrap_or(std::cmp::Ordering::Equal))
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
/// - `f`：输出画布（`[canvas]` 推导；所有边界/盒子尺寸随它缩放）。
pub fn pick_sticker_position(blocked: &[Blocked], allow: &[Rect], seed: f64, f: Frame) -> Pick {
    pick_box_position(blocked, allow, seed, f, &BoxSize::sticker(f))
}

/// 推导落点（**任意尺寸的盒子** —— §5 E3 的动态盒）。
///
/// # 与 [`pick_sticker_position`] 的关系
///
/// 贴纸那条是这一条的**薄包装**（盒子固定为 `BoxSize::sticker(f)`）—— 所以
/// **贴纸落点逐像素不变**，而文字可以传自己的宽高。
///
/// ⚠️ **这不是"顺手抽出个公共函数"**：求解器里 `sticker_size()` 出现过 **6 次**
/// （两轴的下界/上界/临界位、以及 probe 的矩形）。漏改任意一处，
/// 症状都是**文字落点算错一个方向**（比如横向避让按 280 算、纵向按文字真实高算），
/// 而单看某一处代码完全"合理"。全部收敛到 `b` 之后，漏改在**类型上**就不可能了。
///
/// # 参数
///
/// - `blocked`：障碍（人物/字幕/弹幕…）。
/// - `allow`：可选的偏好区域（手写的 `layout.sticker_area` 自由选框）；**只作偏好**，
///   当它里面没有安全位置时自动放弃（安全优先）。
/// - `seed`：0~1 的确定性伪随机数（调用方按 `段起点 + 事件序号` 生成）。
/// - `f`：输出画布（`[canvas]` 推导；边界与边距随它缩放）。
/// - `b`：盒子尺寸（**像素**；贴纸见 [`BoxSize::sticker`]，文字见 [`BoxSize::text`]）。
pub fn pick_box_position(
    blocked: &[Blocked],
    allow: &[Rect],
    seed: f64,
    f: Frame,
    b: &BoxSize,
) -> Pick {
    // 临界候选：盒子刚好贴在障碍右侧/下方，或刚好停在左侧/上方
    let xs = axis(
        f.margin(),
        f.w - f.margin() - b.w,
        blocked
            .iter()
            .flat_map(|bg| [bg.rect.x1 + TOUCH, bg.rect.x0 - b.w - TOUCH]),
    );
    let ys = axis(
        f.margin() + b.float_up,
        f.h - f.margin() - b.h,
        blocked
            .iter()
            .flat_map(|bg| [bg.rect.y1 + b.float_up + TOUCH, bg.rect.y0 - b.h - TOUCH]),
    );

    let mut free: Vec<(f64, f64, f64)> = Vec::new();
    let mut free_in_allow: Vec<(f64, f64, f64)> = Vec::new();
    let mut best_cost = f64::MAX;
    let mut lowest: Vec<(f64, f64, f64)> = Vec::new(); // (cost, x, y)

    for &x in &xs {
        for &y in &ys {
            let p = probe(x, y, b);
            let mut clearance = p.gap_to_frame_edge(f);
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
        return central_pick(pool, seed, f, b);
    }

    // 兜底：所有位置都压着东西 → 取重叠代价最小者，并列时取最靠近帧心的。
    lowest.sort_by(|a, c| {
        center_dist(a.1, a.2, f, b)
            .partial_cmp(&center_dist(c.1, c.2, f, b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let n = lowest.len().min(CENTRAL_PICK).max(1);
    let idx = ((seed.clamp(0.0, 0.999_999) * n as f64) as usize).min(n - 1);
    let (_, x, y) = lowest[idx];
    let p = probe(x, y, b);
    let clearance = blocked
        .iter()
        .map(|b| p.gap_to(&b.rect))
        .fold(p.gap_to_frame_edge(f), f64::min);
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

    /// 横画布基准（与改造前硬编码逐像素一致）。
    const F: Frame = Frame { w: 1920.0, h: 1080.0 };

    fn band_caption() -> Rect {
        caption_band(F, 72.0, 12.0)
    }

    // ── §5 E3：动态盒（文字用）─────────────────────────────────────

    /// ⚠️ **贴纸路径必须逐像素不变**（R4）：`pick_sticker_position` 与
    /// `pick_box_position(.., BoxSize::sticker(f))` 在**任意**输入下结果相同。
    ///
    /// 这条是这次重构（把 `sticker_size()` 换成 `BoxSize`）**唯一**可以出错的地方：
    /// 求解器里那个尺寸出现过 6 次，换错任意一处都会让贴纸落点漂移，
    /// 而单看代码每一处都"合理"。
    #[test]
    fn sticker_path_is_byte_identical_after_dynamic_box_refactor() {
        let blocked = vec![
            Blocked::new(Rect::new(0.0, 0.0, 640.0, 1080.0), ObstacleKind::DanmakuArea),
            Blocked::new(band_caption(), ObstacleKind::Captions),
        ];
        for seed_i in 0..64 {
            let seed = seed_i as f64 / 64.0;
            let a = pick_sticker_position(&blocked, &[], seed, F);
            let b = pick_box_position(&blocked, &[], seed, F, &BoxSize::sticker(F));
            assert_eq!(a, b, "seed {seed}：贴纸路径不许因为动态盒重构而漂移");
        }
        // 竖屏同样对一遍（`sticker_scale` 在那边是 1.0，横竖数值路径不同）
        let port = Frame::new(1080, 1920);
        for seed_i in 0..16 {
            let seed = seed_i as f64 / 16.0;
            let a = pick_sticker_position(&blocked, &[], seed, port);
            let b = pick_box_position(&blocked, &[], seed, port, &BoxSize::sticker(port));
            assert_eq!(a, b, "竖屏 seed {seed} 同样不许漂移");
        }
    }

    /// **盒子越大，落点越靠边** —— 动态盒真的在起作用（而不是"传了但没用"）。
    ///
    /// 判据取**相关性**而不是某个具体坐标：文字宽高变化的组合太多，
    /// 钉死一个坐标只能证明"这条路我跑过"，证明不了"它按盒子大小避让"。
    #[test]
    fn bigger_box_is_pushed_further_from_the_caption_band() {
        // 一条底部字幕带：盒子越宽，越难在它上方找到位置
        let blocked = vec![Blocked::new(band_caption(), ObstacleKind::Captions)];
        let narrow = BoxSize::text(300.0, 100.0);
        let wide = BoxSize::text(1500.0, 100.0);

        let mut narrow_clear = 0.0;
        let mut wide_clear = 0.0;
        for seed_i in 0..32 {
            let seed = seed_i as f64 / 32.0;
            let a = pick_box_position(&blocked, &[], seed, F, &narrow);
            let b = pick_box_position(&blocked, &[], seed, F, &wide);
            // 宽盒子的下沿必须**在字幕带上方**（否则就是"大字压住字幕"）
            assert!(
                b.y + wide.h <= band_caption().y0 + 1e-6 || b.x + wide.w <= band_caption().x0 || b.x >= band_caption().x1,
                "seed {seed}：1500 宽的盒子压进了字幕带: {b:?}"
            );
            narrow_clear += a.clearance;
            wide_clear += b.clearance;
        }
        assert!(
            wide_clear <= narrow_clear + 1e-9,
            "宽盒子的平均余量不该比窄盒子大（{wide_clear} vs {narrow_clear}）—— 动态盒没生效？"
        );
    }

    /// **纵向避让必须用盒子的 `h`，不是 `w`**（正方形近似的变异要能被抓到）。
    ///
    /// ⚠️ 这条是补的：`bigger_box_is_pushed_further_from_the_caption_band` 对
    /// **替换 `b.h` → `b.w`** 的变异**不翻面**（实测过）—— 因为宽盒子本来就够宽，
    /// 两种算法都会把它推离字幕带。抓这个变异要用**又宽又扁**的盒子配**又薄又宽的障碍**。
    /// ⚠️ 判据也不是"它应当贴着障碍"：求解器**偏好画面中心**（`center_dist`），
    /// 会主动躲到中间去。真正能分辨两种算法的是**"有没有位置可放"**：
    ///   * 用真实高度（120）：障碍下方那一大片都能放 ⇒ `relaxed = false`；
    ///   * 用宽度当高（1400）：盒子比画面还高 ⇒ **一个候选都放不下** ⇒ `relaxed = true`。
    #[test]
    fn vertical_fit_uses_box_height_not_width() {
        // 一块薄障碍（横向铺满、只有 60px 高）
        let blocked = vec![Blocked::new(Rect::new(0.0, 0.0, 1920.0, 60.0), ObstacleKind::Liver)];
        let flat = BoxSize::text(1400.0, 120.0);
        const N: usize = 32;
        for seed_i in 0..N {
            let seed = seed_i as f64 / N as f64;
            let p = pick_box_position(&blocked, &[], seed, F, &flat);
            assert!(
                !p.relaxed,
                "seed {seed}: 扁盒子（1400×120）退化到了 relaxed ——                  说明纵向避让把它当成了 1400 高的巨型盒子（拿了宽度当高）: {p:?}"
            );
            assert!(p.y + flat.h <= F.h + 1e-6, "seed {seed}: 盒子出画: {p:?}");
        }
    }

    /// 文字盒**不带**入场上浮（`: `float_up = 0`）—— 用错的后果是落点整体偏上 18px。
    #[test]
    fn text_box_has_no_float_up() {
        let t = BoxSize::text(400.0, 120.0);
        assert_eq!(t.float_up, 0.0, "文字没有贴纸那种入场飘起，占位不该往上留");
        let r = box_at(100.0, 200.0, &t);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (100.0, 200.0, 500.0, 320.0));
        // 对照：贴纸盒**要**往上留 `float_up`（否则入场那一瞬会压住东西）
        let s = BoxSize::sticker(F);
        assert!(s.float_up > 0.0, "贴纸盒必须留入场上浮占位");
        let rs = box_at(0.0, 0.0, &s);
        assert!(rs.y0 < 0.0, "贴纸盒的上沿应当在 y 之上（留出上浮量）");
    }

    /// 竖屏贴纸尺寸 = **短边**基准：横竖画布下贴纸绝对像素相同（都是 280×280），
    /// 不再按 `k = W/1920` 缩成 157.5（用户报「贴纸在竖屏模式没有正常大小」）。
    #[test]
    fn sticker_scale_is_short_edge_based() {
        let port = Frame::new(1080, 1920);
        for f in [Frame::landscape(), port] {
            assert!((f.sticker_scale() - 1.0).abs() < 1e-9, "{f:?}");
            assert!((f.sticker_size() - STICKER_SIZE).abs() < 1e-9, "{f:?}");
            assert!((f.margin() - STICKER_MARGIN).abs() < 1e-9, "{f:?}");
            assert!((f.float_up() - FLOAT_UP).abs() < 1e-9, "{f:?}");
        }
        // `k`（宽度比）**不参与**贴纸尺寸：竖屏 k=0.5625，按它会得到 157.5
        assert!((port.k() * STICKER_SIZE - 157.5).abs() < 1e-9);

        // 短边更小的画布按短边等比缩（720×1280 → 720/1080，而不是按宽 720/1920）
        let small = Frame::new(720, 1280);
        assert!((small.sticker_size() - STICKER_SIZE * 720.0 / 1080.0).abs() < 1e-9);
    }

    /// 竖屏落点网格跟着新尺寸走：盒子 280 + 边距 32 → 候选左上角范围
    /// 恰好是 `[32, 1080-280-32] × [50, 1920-280-32]`（y 下界含上浮量）。
    #[test]
    fn portrait_grid_fits_a_full_size_box() {
        let port = Frame::new(1080, 1920);
        let blocked = vec![Blocked::new(
            Rect::new(0.0, 0.0, 400.0, 1920.0),
            ObstacleKind::Liver,
        )];
        let p = pick_sticker_position(&blocked, &[], 0.42, port);
        assert!(!p.relaxed, "{p:?}");
        assert!(p.x >= port.margin(), "{p:?}");
        assert!(p.y >= port.margin() + port.float_up(), "{p:?}");
        assert!(p.x + port.sticker_size() <= port.w - port.margin(), "{p:?}");
        assert!(p.y + port.sticker_size() <= port.h - port.margin(), "{p:?}");
        assert_no_overlap(&blocked, &p, 0.42, port);
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
        let b = danmaku_band(F, 42.0, 2.0, &[300.0, 900.0, 1500.0]).unwrap();
        assert!((b.y0 - (84.375 - 22.0 - 8.0)).abs() < 0.01, "{b:?}");
        assert!((b.y1 - (421.875 + 22.0 + 8.0)).abs() < 0.01, "{b:?}");
        assert_eq!(b.x1, F.w);
        assert!(danmaku_band(F, 42.0, 2.0, &[]).is_none());
    }

    #[test]
    fn camera_expand_scales_around_origin() {
        let r = Rect::new(1028.5, 12.2, 1727.2, 656.4);
        let z = camera_expand(r, 960.0, 540.0, 1.5).clamp_to_frame(F);
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
        assert_eq!(transition_secs("远", "中", false, None, TRANSITION_SECS), 0.0);
        assert_eq!(transition_secs("中", "远", false, None, TRANSITION_SECS), 0.0, "退出远景同样是切");
        assert_eq!(transition_secs("近", "远", false, None, TRANSITION_SECS), 0.0);
        assert_eq!(transition_secs("远", "远", false, None, TRANSITION_SECS), 0.0);
        assert_eq!(transition_secs("中", "近", false, None, TRANSITION_SECS), TRANSITION_SECS, "普通档位仍走缓动");
        // 硬切（编辑器「方式 = 硬切」）：任意档位都能瞬间到位，不只有画中画
        assert_eq!(transition_secs("中", "近", true, None, TRANSITION_SECS), 0.0);
        assert_eq!(transition_secs("近", "中", true, None, TRANSITION_SECS), 0.0);

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
                transition: transition_secs("远", "中", false, None, TRANSITION_SECS),
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

    /// §16 `speed` 要传进贴纸避让：**与求值器同口径**，且硬切优先于 `speed`。
    #[test]
    fn transition_window_follows_camera_speed() {
        // 四档与 `engine.js::cameraTransitionSecs` 逐值同口径（@ dur=5s，`slow` 装得下）
        assert_eq!(transition_secs("中", "中", false, Some("snap"), 5.0), 0.10);
        assert_eq!(transition_secs("中", "中", false, Some("fast"), 5.0), 0.25);
        assert_eq!(transition_secs("中", "中", false, Some("slow"), 5.0), 1.5);
        // `med` = 老式子（这里即 `TRANSITION_SECS`），**不是**一个新常数
        assert_eq!(transition_secs("中", "中", false, Some("med"), 5.0), TRANSITION_SECS);
        // `slow` 要夹到 `dur × 0.8`（短镜头推不完就会被切走）
        assert_eq!(transition_secs("中", "中", false, Some("slow"), 1.0), 0.8);
        assert_eq!(transition_secs("中", "中", false, Some("slow"), 0.5), 0.4);
        // 大小写/空白归一（LLM 会写 "Slow "）；认不出的值退回老式子（不猜）
        assert_eq!(transition_secs("中", "中", false, Some(" SLOW "), 5.0), 1.5);
        assert_eq!(transition_secs("中", "中", false, Some("warp"), 5.0), TRANSITION_SECS);

        // ⚠️ **硬切优先于 speed**：`cut` 与本档/上一档是远，都必须 0 —— 哪怕写了 `slow`。
        // 求值器那边也是这个优先级（`hardCut` 先判）。
        assert_eq!(transition_secs("中", "近", true, Some("slow"), 5.0), 0.0);
        assert_eq!(transition_secs("远", "中", false, Some("slow"), 5.0), 0.0);
        assert_eq!(transition_secs("中", "远", false, Some("slow"), 5.0), 0.0);

        // ⚠️ **存量零回归**：`speed = None` 时逐值等于改动前，**不论 dur 给什么**
        for dur in [0.0, 0.5, 1.0, 5.0, 60.0] {
            assert_eq!(
                transition_secs("中", "近", false, None, dur),
                TRANSITION_SECS,
                "没写 speed ⇒ 老行为（dur={dur}）"
            );
        }
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
            let p = pick_sticker_position(&blocked, &[], seed, F);
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
        let p = pick_sticker_position(&blocked, &[], 0.5, F);
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
        let p = pick_sticker_position(&blocked, &allow, 0.4, F);
        assert!(!p.relaxed);
        assert!(p.x >= 1000.0, "{p:?}");
    }

    #[test]
    fn falls_back_to_least_overlap_when_frame_is_full() {
        let blocked = vec![Blocked::new(
            Rect::new(0.0, 0.0, FRAME_W, FRAME_H),
            ObstacleKind::Liver,
        )];
        let p = pick_sticker_position(&blocked, &[], 0.1, F);
        assert!(p.relaxed);
        assert!(p.x >= STICKER_MARGIN && p.y >= STICKER_MARGIN);
    }

    #[test]
    fn seed_is_deterministic() {
        let blocked = vec![Blocked::new(band_caption(), ObstacleKind::Captions)];
        let a = pick_sticker_position(&blocked, &[], 0.42, F);
        let b = pick_sticker_position(&blocked, &[], 0.42, F);
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
            let l = camera_expand(liver, o.0, o.1, *z).clamp_to_frame(F);
            let p = camera_expand(panel, o.0, o.1, *z).clamp_to_frame(F);
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
            let p = pick_sticker_position(&blocked, &[], seed, F);
            assert!(!p.relaxed, "seed {seed}: {p:?}");
            assert_no_overlap(&blocked, &p, seed, F);
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
        let p = pick_sticker_position(&blocked, &[], 0.5, F);
        assert!(!p.relaxed, "{p:?}");
        assert_no_overlap(&blocked, &p, 0.5, F);
        assert!(p.x >= l - TOUCH && p.x + STICKER_SIZE <= r + TOUCH, "{p:?}");
    }

    fn assert_no_overlap(blocked: &[Blocked], p: &Pick, seed: f64, f: Frame) {
        let box_ = sticker_box(p.x, p.y, f);
        for b in blocked {
            assert!(!box_.intersects(&b.rect), "seed {seed} 压住 {:?}: {p:?}", b.kind);
        }
    }

    // ===== 主画面障碍 =====

    /// 测试用 `StageFit`：只关心 axis / position_x+y_pct / scale / padding 四项。
    /// `position` 字符串只当**旧三档**用（left/top → 0、right/bottom → 100、其余居中），
    /// 与 `stage_position_pct` 同口径，方便老用例原样迁移；**两轴同值** = 单值时代的几何
    /// （贴满的那条轴余量为 0 ⇒ 与老行为逐像素等价）。
    fn stage(axis: &str, position: &str, scale: f64, padding: f64) -> crate::project::StageFit {
        let pct = match position {
            "left" | "top" => 0.0,
            "right" | "bottom" => 100.0,
            _ => 50.0,
        };
        crate::project::StageFit {
            axis: axis.into(),
            object_fit: if axis == "fill" { "cover" } else { "contain" }.into(),
            position: position.into(),
            position_x_pct: pct,
            position_y_pct: pct,
            free_x: 0.0,
            free_y: 0.0,
            bg: if axis == "fill" { "none" } else { "blur" }.into(),
            bg_color: "#000000".into(),
            blur_px: 28,
            blur_img_scale: 1.187,
            scale,
            padding,
        }
    }

    /// **零回归钉子**：`axis = fill`（视频铺满画布，含改造前全部横素材项目）不产生主画面障碍。
    /// 否则整帧都是障碍 → 所有贴纸退化成 relaxed 兜底，比改造前更差。
    #[test]
    fn fill_axis_yields_no_main_frame_obstacle() {
        let s = stage("fill", "center", 1.0, 0.0);
        assert!(main_frame_rect(&s, Some((1920, 1080)), (1920, 1080)).is_none());
        // 4:3 源也是 fill（改造前就是裁切），同样不产生障碍
        assert!(main_frame_rect(&s, Some((1440, 1080)), (1920, 1080)).is_none());
    }

    /// **W1 判据**：`layout.<区域>` 的百分比是**主画面矩形 `sr`** 的百分比，不是整块画布。
    ///
    /// 4:3 源进 16:9 画布（`axis = x`，左右留边）：`sr = (240, 0, 1440, 1080)`。
    /// 同一个「x 10%~40% / y 30%~70%」的选框：
    /// - 按 `sr` 读（**W1，现在**）→ 画布 `[384, 324, 816, 756]`
    /// - 按整帧读（**改造前的旧模型**）→ `[192, 324, 768, 756]`
    ///
    /// x 上差 **192px**：旧模型把"人物在框内"判成"框外"，临界候选**真的会压住主播**
    /// （`docs/plan/polish-preview-bugs-2026-10-05.md` §⑤ 4a 实测 47px）。
    #[test]
    fn layout_region_percent_is_relative_to_the_main_frame_rect() {
        let s = stage("x", "center", 1.0, 0.0);
        let sr = stage_rect(&s, Some((1440, 1080)), (1920, 1080));
        assert_eq!(sr, Rect::new(240.0, 0.0, 1680.0, 1080.0), "4:3 源 → 左右留边");

        let tl = crate::project::Point { x: "10%".into(), y: "30%".into() };
        let br = crate::project::Point { x: "40%".into(), y: "70%".into() };
        assert_eq!(
            stage_region_rect(&tl, &br, sr),
            Rect::new(384.0, 324.0, 816.0, 756.0)
        );
        // 旧口径（整帧）就是那个会压住主播的模型 —— 钉住它**不再**是生产取值
        let whole_frame = Rect::new(1920.0 * 0.10, 1080.0 * 0.30, 1920.0 * 0.40, 1080.0 * 0.70);
        assert_eq!(whole_frame, Rect::new(192.0, 324.0, 768.0, 756.0));
        assert_ne!(stage_region_rect(&tl, &br, sr).x0, whole_frame.x0);
    }

    /// ⑤ 4a 的**实测量 47px**：`pick_sticker_position` 的候选里有一类是「贴着障碍右沿的临界位」
    /// （`b.rect.x1 + TOUCH`）。按整帧读时那条临界位由**模型**障碍的右沿生成（`768+1 = 769`），
    /// 它的盒子 `[769,1049]` 与**真**障碍 `[384,816]` 重叠 `816-769 = 47px` —— 即实测的
    /// 「临界候选真压住主播 47px」。改走 `sr` 后临界位从真障碍右沿生成（`816+1 = 817`），不重叠。
    ///
    /// 两个矩形都由生产函数（[`stage_rect`] + [`stage_region_rect`]）算出，不是手抄常数。
    #[test]
    fn critical_candidate_at_the_model_edge_covers_the_star_by_47px() {
        let s = stage("x", "center", 1.0, 0.0);
        let sr = stage_rect(&s, Some((1440, 1080)), (1920, 1080));
        let tl = crate::project::Point { x: "10%".into(), y: "10%".into() };
        let br = crate::project::Point { x: "40%".into(), y: "40%".into() };
        let real = stage_region_rect(&tl, &br, sr);
        let model = stage_region_rect(&tl, &br, Rect::new(0.0, 0.0, F.w, F.h));
        assert_eq!(real, Rect::new(384.0, 108.0, 816.0, 432.0));
        assert_eq!(model, Rect::new(192.0, 108.0, 768.0, 432.0));

        // 模型右沿的临界位 → 盒子左侧落在真障碍内部 47px（真能压住主播）
        let at_model = sticker_box(model.x1 + TOUCH, model.y0, F);
        assert_eq!(at_model.x0, real.x1 - 47.0, "实测 47px");
        assert!(at_model.intersects(&real), "{at_model:?} vs {real:?}");
        // 真障碍右沿的临界位 → 不重叠（修好之后候选集里是这一条）
        assert!(!sticker_box(real.x1 + TOUCH, model.y0, F).intersects(&real));
    }

    #[test]
    fn main_frame_needs_a_source_size() {
        let s = stage("x", "left", 1.0, 0.0);
        assert!(main_frame_rect(&s, None, (1920, 1080)).is_none(), "拿不到尺寸就不计障碍");
        assert!(main_frame_rect(&s, Some((0, 0)), (1920, 1080)).is_none());
    }

    /// 横画布 × 竖素材：视频按高度铺满 → 宽 = 1080 * 1080/1920 = 607.5
    #[test]
    fn portrait_source_on_landscape_canvas_sits_on_one_side() {
        let (cw, ch) = (1920u32, 1080u32);
        let src = Some((1080u32, 1920u32));

        let l = main_frame_rect(&stage("x", "left", 1.0, 0.0), src, (cw, ch)).unwrap();
        assert!((l.x0 - 0.0).abs() < 0.01 && (l.x1 - 607.5).abs() < 0.01, "{l:?}");
        assert!((l.y0 - 0.0).abs() < 0.01 && (l.y1 - 1080.0).abs() < 0.01, "{l:?}");

        let c = main_frame_rect(&stage("x", "center", 1.0, 0.0), src, (cw, ch)).unwrap();
        assert!((c.x0 - 656.25).abs() < 0.01 && (c.x1 - 1263.75).abs() < 0.01, "{c:?}");

        let r = main_frame_rect(&stage("x", "right", 1.0, 0.0), src, (cw, ch)).unwrap();
        assert!((r.x0 - 1312.5).abs() < 0.01 && (r.x1 - 1920.0).abs() < 0.01, "{r:?}");
    }

    /// 竖画布 × 横素材：视频按宽度铺满 → 高 = 1080 * 1080/1920 = 607.5（对称支持 上/中/下）。
    ///
    /// P3 把坐标系参数化之后，`main_frame_rect` 直接接受竖画布 —— 这条改测完整路径。
    #[test]
    fn landscape_source_on_portrait_canvas_stacks_vertically() {
        let (cw, ch) = (1080u32, 1920u32);
        let src = Some((1920u32, 1080u32));

        let t = main_frame_rect(&stage("y", "top", 1.0, 0.0), src, (cw, ch)).unwrap();
        assert!((t.y0 - 0.0).abs() < 0.01 && (t.y1 - 607.5).abs() < 0.01, "{t:?}");
        assert!((t.x0 - 0.0).abs() < 0.01 && (t.x1 - 1080.0).abs() < 0.01, "{t:?}");

        let c = main_frame_rect(&stage("y", "center", 1.0, 0.0), src, (cw, ch)).unwrap();
        assert!((c.y0 - 656.25).abs() < 0.01 && (c.y1 - 1263.75).abs() < 0.01, "{c:?}");

        let b = main_frame_rect(&stage("y", "bottom", 1.0, 0.0), src, (cw, ch)).unwrap();
        assert!((b.y0 - 1312.5).abs() < 0.01 && (b.y1 - 1920.0).abs() < 0.01, "{b:?}");
    }

    /// P3 起竖画布与横画布走同一套几何与坐标系（P2 时的守卫已随 `Frame` 参数化移除）：
    /// 竖画布 × 横素材的主画面**计入**，且与纯几何钳制后逐像素一致。
    #[test]
    fn portrait_canvas_main_frame_is_now_considered() {
        let src = Some((1920u32, 1080u32));
        let f = Frame::new(1080, 1920);
        let s = stage("y", "center", 1.0, 0.0);
        let rect = main_frame_rect(&s, src, (1080, 1920)).unwrap();
        let geo = main_frame_geometry(&s, src, (1080, 1920)).unwrap().clamp_to_frame(f);
        assert_eq!(rect, geo);
    }

    /// 竖画布 × 横素材（画面靠上）：auto 落点要落到主画面**下方**的空带，
    /// 不再压在主视频身上 —— 与横画布 `sticker_avoids_the_main_frame_it_cannot_fill` 对称。
    #[test]
    fn sticker_avoids_main_frame_on_portrait_canvas() {
        let (cw, ch) = (1080u32, 1920u32);
        let f = Frame::new(cw, ch);
        let main = main_frame_rect(&stage("y", "top", 1.0, 0.0), Some((1920, 1080)), (cw, ch)).unwrap();
        let blocked = vec![Blocked::new(main, ObstacleKind::MainFrame)];
        for seed in [0.0, 0.3, 0.5, 0.8, 0.99] {
            let p = pick_sticker_position(&blocked, &[], seed, f);
            assert!(!p.relaxed, "下方 {}px 空间充足: {p:?}", f.h - main.y1);
            assert_no_overlap(&blocked, &p, seed, f);
            assert!(p.y >= main.y1 - TOUCH, "落点应在主画面下方: {p:?}");
        }
    }

    /// `scale` 绕盒子中心放大（对应模板把 scale 挂在 `#fit-box` 上）
    #[test]
    fn scale_grows_main_frame_around_its_center() {
        let base = main_frame_rect(
            &stage("x", "center", 1.0, 0.0),
            Some((1080, 1920)),
            (1920, 1080),
        )
        .unwrap();
        let big = main_frame_rect(
            &stage("x", "center", 2.0, 0.0),
            Some((1080, 1920)),
            (1920, 1080),
        )
        .unwrap();
        assert!((big.width() - base.width() * 2.0).abs() < 0.01, "{big:?}");
        let (bcx, bcy) = ((base.x0 + base.x1) / 2.0, (base.y0 + base.y1) / 2.0);
        assert!(((big.x0 + big.x1) / 2.0 - bcx).abs() < 0.01, "中心不动: {big:?}");
        assert!(((big.y0 + big.y1) / 2.0 - bcy).abs() < 0.01, "中心不动: {big:?}");
    }

    /// 留白缩小内容盒：视频跟着变窄，靠边时留白量就是偏移量
    #[test]
    fn padding_shrinks_the_content_box() {
        let r = main_frame_rect(
            &stage("x", "left", 1.0, 60.0),
            Some((1080, 1920)),
            (1920, 1080),
        )
        .unwrap();
        assert!((r.x0 - 60.0).abs() < 0.01, "靠左时偏移 = padding: {r:?}");
        assert!((r.height() - 960.0).abs() < 0.01, "内容盒高 1080-120: {r:?}");
        assert!((r.width() - 960.0 * 0.5625).abs() < 0.01, "{r:?}");
    }

    /// 放大到铺满整帧时不算障碍（没有可避让的余地）
    #[test]
    fn oversized_main_frame_is_not_an_obstacle() {
        let r = main_frame_rect(
            &stage("x", "center", 3.0, 0.0),
            Some((1080, 1920)),
            (1920, 1080),
        );
        assert!(r.is_none(), "铺满整帧 → 不该当障碍: {r:?}");
    }

    /// **A4b 的目的**：主画面算障碍后，auto 落点不再压在主视频身上。
    ///
    /// 用 `center` 而不是 `left`：没有任何障碍时落点会跑到帧中心（x≈820），
    /// 靠左那条竖带（0~607）本来就不在帧中心，测不出东西；居中的竖带（656~1264）
    /// 才真正套住帧中心 —— 这正是「横画布 × 竖素材居中摆放」时的实际缺陷。
    #[test]
    fn sticker_avoids_the_main_frame_it_cannot_fill() {
        for pos in ["center", "left", "right"] {
            let main = main_frame_rect(
                &stage("x", pos, 1.0, 0.0),
                Some((1080, 1920)),
                (1920, 1080),
            )
            .unwrap();
            let blocked = vec![Blocked::new(main, ObstacleKind::MainFrame)];
            for seed in [0.0, 0.13, 0.5, 0.77, 0.99] {
                let p = pick_sticker_position(&blocked, &[], seed, F);
                assert!(
                    !p.relaxed,
                    "position={pos}: 两侧各有 656px 空带, 不该兜底: {p:?}"
                );
                assert_no_overlap(&blocked, &p, seed, F);
            }
        }
    }

    /// 对照：**不加**主画面障碍时（= 改造前行为）同一个 seed 会压在主画面上。
    /// 没有这条，上面那条测试可能只是「本来就压不到」而白过。
    #[test]
    fn without_the_obstacle_the_same_seed_lands_on_the_video() {
        let main = main_frame_rect(
            &stage("x", "center", 1.0, 0.0),
            Some((1080, 1920)),
            (1920, 1080),
        )
        .unwrap();
        let p = pick_sticker_position(&[], &[], 0.5, F);
        assert!(
            sticker_box(p.x, p.y, F).intersects(&main),
            "对照组应该压在主画面上, 否则那条钉子没意义: {p:?} vs {main:?}"
        );
    }
}
