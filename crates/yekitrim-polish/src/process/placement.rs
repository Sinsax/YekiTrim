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

/// **文字元素的字号基准**（文档像素，1080p 基准）—— 四档**统一**用这一个值。
///
/// 用户口径（2026-10-13）：「将元素的文字字体大小固定为 80」。
///
/// ⚠️ **唯一真源在 Rust**：转译器 `TEXT_STYLE_FALLBACK` 与预览
/// `engine.js::parseTextSource` 的兜底值都必须与它同值 ——
/// 那两处只在"工程没跑过 `process`"时才被读到，**不同值会让那类工程的字号悄悄分叉**。
/// 判据：`scripts/check-text-style.mjs` ⑦。
///
/// ⚠️ 它是**文档像素**，消费点一律除以 [`FRAME_H`] 再乘实际帧高
/// （`TextLayout::solve`）⇒ 竖画布自动按比例放大，不是一个写死的像素数。
pub const ELEMENT_FONT_PX: f64 = 80.0;

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
    /// ⚠️ **2026-10-11（§5 E3）摘掉了 `#[cfg(test)]`** —— 生产调用方接上了
    /// （[`TextLayout::box_size`]，由 `process::mod` 的文字排版步骤驱动）。
    /// 在那之前它挂着 `cfg(test)`：那不是"藏起来"，是**让它一直接着编译、
    /// 但没接上生产时不会漏掉**（dead_code 警告就是"还没接上"的提醒）。
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

/// 文字元素的四个 `style` 预设（规格 `elements-spec.md` §2）。
///
/// ⚠️ **四档的数值不是"观感调优"**：`hook`/`chapter` 是**画面主体**（大字，
/// 要压得住画面）、`comment`/`count` 是**附注**（小字，不能抢戏）。两者字号
/// 差了近 3 倍，混用会让"吐槽"看起来像"标题"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextStyle {
    /// 钩子标题：大号、高对比、强描边。
    Hook,
    /// 章节卡：中号、带底、居中。
    Chapter,
    /// 吐槽：小号、带底色块。
    Comment,
    /// 计数/积累（"第 3 次"）：中号。
    Count,
}

impl TextStyle {
    /// 从工程里的字符串解析。**认不出的一律当 `Comment`**（最保守的一档）。
    ///
    /// ⚠️ 不报错、不 panic：`style` 是 LLM 写的自由文本，写错一个字母就让整条
    /// 出片失败，代价远大于"这条字小了一点"。但**也不静默** ——
    /// 调用方据 [`Self::is_known`] 打一行 note，用户能从报告里看到。
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "hook" => Self::Hook,
            "chapter" => Self::Chapter,
            "count" => Self::Count,
            _ => Self::Comment,
        }
    }

    /// 这个串是不是四个预设之一（用于"认不出就报一行"）。
    pub fn is_known(raw: &str) -> bool {
        matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "hook" | "chapter" | "comment" | "count"
        )
    }

    /// 字号的基准值（**文档像素，1080p 基准**）—— 比例由它推出来。
    ///
    /// # 2026-10-13：**四档统一 80**（用户口径「将元素的文字字体大小固定为 80」）
    ///
    /// 在此之前四档各写死一个值（`120 / 84 / 44 / 64`），依据是"档位的定义就是字号差"。
    /// 用户改为**固定 80** 之后，档位的区分**整体平移到其余三个维度**
    /// （字号不再承担区分的职责）：
    ///
    /// | 档 | 描边 | 底衬 | 字重 |
    /// |---|---|---|---|
    /// | `hook` | 6（最粗） | 无 | 900 |
    /// | `chapter` | 5 | **有** | 700 |
    /// | `comment` | 2（最细） | **有** | 700 |
    /// | `count` | 4 | 无 | 800 |
    ///
    /// ⚠️ **"四档互相可区分"这条不变式没有消失，只是换了承载它的字段** ——
    /// 由 `四档字号统一而其余维度可区分` 钉着（老判据 `四档字号从大到小且互不相等`
    /// 断言的正是这件事，这里把它的观察点从字号平移到三元组上）。
    ///
    /// ⚠️ **参照点仍是字幕的 72**（`templates/index.html` 的 `.cap`）——
    /// 80 是它的 1.11 倍，所以文字元素比字幕略大、又不像老 `hook` 那样能压满画面。
    ///
    /// ⚠️ **保留这个函数**（而不是让调用方直接读常量）：它是"档位 → 字号"的**唯一接口点**，
    /// 将来若要恢复逐档，只改这一处即可。`TextLayout::solve` 里
    /// `style.base_font_px() / FRAME_H * f.h` 一行不动 —— 比例语义天然保持。
    pub const fn base_font_px(self) -> f64 {
        ELEMENT_FONT_PX
    }

    /// 描边宽度的基准值（**文档像素，外侧宽度**）。
    ///
    /// ⚠️ **2026-10-13 随字号一起重算**：规格 `elements-spec.md` §4.2 要求
    /// 「**描边粗度 = 字号的 5–8%**」。老值 `8` 是当年按 `hook = 120` 配的
    /// （`8/120 = 6.7%`，合规）；字号降到 80 之后 `8/80 = 10%` **超出规格上限**，
    /// 大粗边会把字腔吃掉 ⇒ `hook` 从 8 降到 **6**（`6/80 = 7.5%`，且仍是四档最粗）。
    /// 其余三档本来就在区间内（`5/80 = 6.3%`、`4/80 = 5%`、`2/80 = 2.5%` ——
    /// `comment` 是"吐槽"档，刻意细，维持原值不放大）。
    pub const fn base_stroke_px(self) -> f64 {
        match self {
            Self::Hook => 6.0,
            Self::Chapter => 5.0,
            Self::Comment => 2.0,
            Self::Count => 4.0,
        }
    }

    /// 文字颜色（RGBA）。
    pub const fn color(self) -> [u8; 4] {
        match self {
            // 白字 + 深描边：与字幕同一套对比度做法。
            Self::Hook | Self::Chapter | Self::Count => [255, 255, 255, 255],
            // 吐槽用暖黄 —— 它是"旁白"，与正文字幕要分得开。
            Self::Comment => [255, 226, 122, 255],
        }
    }

    /// 描边颜色。四档都是近黑 —— 亮背景上这才压得住。
    pub const fn stroke_color(self) -> [u8; 4] {
        [26, 26, 26, 255]
    }

    /// 字重（CSS 刻度）。
    pub const fn font_weight(self) -> u32 {
        match self {
            Self::Hook => 900,
            Self::Chapter => 700,
            Self::Comment => 700,
            Self::Count => 800,
        }
    }

    /// 要不要底衬色块（`chapter`/`comment` 是"卡片"）。
    pub const fn has_backdrop(self) -> bool {
        matches!(self, Self::Chapter | Self::Comment)
    }

    /// **倾斜角度**（度，**正 = 顺时针**）。`0.0` = 不歪。
    ///
    /// # 值域是硬约束，不是口味
    ///
    /// 规格给的上限是 **±7°** —— 再大就开始影响可读性（汉字是方块字，
    /// 歪过去之后行基线不再水平，长句读起来要"转头"）。
    /// ⚠️ 所以本函数返回的**每**一档都必须落在 `[-7, 7]` 内，
    /// 由 `tilt_within_readability_limit` 钉住。
    ///
    /// # 为什么只有两档非零
    ///
    /// 倾斜是**强调**手段，用在"这句话我要你停一下"的地方：
    /// `hook`（钩子标题）与 `count`（"第 3 次了"这种累积计数）才是那个语气。
    /// `chapter`（章节卡）与 `comment`（吐槽）是**常态阅读**的两档，歪着反而添乱。
    /// ⚠️ 认不出的 `style` 会解析成 `Comment`（最保守档，见 [`Self::parse`]）
    /// ⇒ **兜底档是不歪的那一档**，这与 `TextAnchor::Auto` 的"认不出走最保守"同一条规矩。
    pub const fn tilt_deg(self) -> f64 {
        match self {
            // 钩子最抢眼，给到上限附近；计数次之（它要"跳出来"但不能喧宾夺主）。
            Self::Hook => 6.0,
            Self::Count => -4.0,
            // 常态阅读档：不歪。
            Self::Chapter | Self::Comment => 0.0,
        }
    }

    /// **纵向压缩倍数**（`1.0` = 不压）。`< 1.0` = 压扁。
    ///
    /// # 值域同样是硬约束
    ///
    /// 规格给的下限是 **0.85** —— 再压字就"瘪"了（汉字的重心与字面高度是设计好的，
    /// 压过头看着像被裁掉一截）。由 `squash_within_readability_limit` 钉住。
    ///
    /// # 语义
    ///
    /// 这是**纵向**倍数，横向不动（即"瘦高/矮胖"那个维度），
    /// 与等比缩放 `scale` 是两个独立旋钮。`0.92` = 压掉 8% 的高度。
    ///
    /// ⚠️ 只有 `chapter`（章节卡）压一点点 —— 卡片式排版略扁显得"稳"。
    /// 其余三档一律 `1.0`（不压），**包含兜底档 `Comment`**。
    pub const fn squash_y(self) -> f64 {
        match self {
            Self::Chapter => 0.92,
            Self::Hook | Self::Comment | Self::Count => 1.0,
        }
    }
}

/// **文字颜色的命名预设**（2026-10-13）—— 让 LLM 能表达颜色。
///
/// # 为什么是"命名枚举"而不是 hex
///
/// LLM 之前**完全无法表达颜色**：颜色只有 [`TextStyle::color`] 那四个写死的常量
/// （`hook`/`chapter`/`count` 白、`comment` 暖黄）。要让 LLM 能"按情绪上色"，
/// 就得给它一个字段。
///
/// 但**不能让它写 `#ff80c0`**：模型写 hex 的出错率很高（少一位、`#` 丢掉、
/// 写成 `rgb()`），而每一种错法都只在出片时才看得出来；而且 hex 对 LLM
/// **没有语义**——它没法从 `#FF4040` 推出"愤怒"。命名预设两头都解决：
/// 名字本身携带情绪语义（提示词可以写"愤怒用 red"），且拼错时能报出
/// **具体是哪个名字认不出**（与 [`TextStyle::parse`] 同一条纪律）。
///
/// # 与 `style` 的关系
///
/// **颜色覆盖档位默认色**：不写 `color` ⇒ 走 [`TextStyle::color`]（存量工程逐字节不变）；
/// 写了 ⇒ 用预设色。不做"底色 + 强调色"两套。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementColor {
    /// 白 —— 万能解，正式场合。
    White,
    /// 粉 —— 可爱 / 调侃 / 萌。
    Pink,
    /// 红 —— 愤怒 / 警报 / 反驳。
    Red,
    /// 黄 —— **与现 `comment` 档的默认色逐值相同**（`255,226,122`）⇒ 不写时的观感可以显式复现。
    Yellow,
    /// 青 —— 冷静 / 疑问 / 科普。
    Cyan,
    /// 绿 —— 肯定 / 涨 / 通过。
    Green,
    /// 黑 —— **只配浅色底衬用**，见 [`Self::stroke_for`] 那条硬约束。
    Black,
}

impl ElementColor {
    /// 全部预设（供提示词与下拉生成，**顺序即展示顺序**）。
    ///
    /// ⚠️ 提示词、下拉、转译器、预览四处都读这一张表 —— 它们的一致性由
    /// `scripts/check-element-color.mjs` 钉着（四处各存一份的症状是
    /// "LLM 写 `pink`、面板里没有它、或者成片里是白的"，而四处各自都不报错）。
    pub const ALL: [Self; 7] = [
        Self::White,
        Self::Pink,
        Self::Red,
        Self::Yellow,
        Self::Cyan,
        Self::Green,
        Self::Black,
    ];

    /// 从工程里的字符串解析。**认不出返回 `None`**（由调用方决定怎么办）。
    ///
    /// ⚠️ 与 [`TextStyle::parse`] **不同**：那个认不出退回 `Comment`（最保守的**观感**档），
    /// 因为它们管的是"字长什么样"，总得有一档。颜色这里**返回 `None`** ——
    /// "没写"（⇒ 走档位默认色）与"写错了"（⇒ 报一行且走档位默认色）是两件事，
    /// 合并成一个 `Option` 之外的值就再也分不出来了。调用方据此打警告。
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "white" => Some(Self::White),
            "pink" => Some(Self::Pink),
            "red" => Some(Self::Red),
            "yellow" => Some(Self::Yellow),
            "cyan" => Some(Self::Cyan),
            "green" => Some(Self::Green),
            "black" => Some(Self::Black),
            _ => None,
        }
    }

    /// 预设名（写回 `source` 串用；与 [`Self::parse`] **互为逆运算**）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Pink => "pink",
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Cyan => "cyan",
            Self::Green => "green",
            Self::Black => "black",
        }
    }

    /// 填充色（RGBA）。
    ///
    /// 取值都**刻意避开了纯饱和**（纯红 `#FF0000` 在视频压缩后边缘会糊、
    /// 且与深描边对比过硬），并按"深色描边下仍读得清"校过。
    pub const fn rgba(self) -> [u8; 4] {
        match self {
            Self::White => [255, 255, 255, 255],
            Self::Pink => [255, 128, 192, 255],
            Self::Red => [255, 64, 64, 255],
            Self::Yellow => [255, 226, 122, 255],
            Self::Cyan => [96, 224, 255, 255],
            Self::Green => [128, 232, 128, 255],
            Self::Black => [24, 24, 24, 255],
        }
    }

    /// **这一档该配什么描边色** —— 黑字必须翻成白描边，这是硬约束不是口味。
    ///
    /// # 为什么必须成对返回
    ///
    /// [`TextStyle::stroke_color`] 四档一律近黑（`[26,26,26]`，亮背景上才压得住）。
    /// 黑色填充 + 近黑描边 = **糊成一团墨**，字还在、但读不出来 ——
    /// 而报告里一行差异都没有（颜色字段是对的、描边字段也是对的，只是两者不搭）。
    ///
    /// ⇒ 与其让调用方"记得"给黑字换描边（四个消费点都得记），
    /// 不如**由这里成对给出**：调用点只有"取填充 + 取描边"这一条路。
    /// ⚠️ 其余六档的描边仍归 `TextStyle` —— 只有 `Black` 这一档需要翻转。
    pub const fn stroke_for(self, style: TextStyle) -> [u8; 4] {
        match self {
            // 白描边 + 稍低 alpha：纯白边在黑字周围太"跳"，230 略柔和。
            Self::Black => [255, 255, 255, 230],
            _ => style.stroke_color(),
        }
    }
}

/// 文字在画面上的**纵向锚点**（规格里的 `pos`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAnchor {
    /// 顶部。
    Top,
    /// 居中。
    Center,
    /// 底部（与字幕带错开）。
    Bottom,
    /// **自动**：不给纵向带，让求解器在**整块安全区**里挑。
    ///
    /// ⚠️ 这是现在的**默认**（2026-10-12 用户定）：面板上去掉了「纵向位置」下拉，
    /// 文字与贴纸一样"只说什么、不指定摆哪"。原来的三条带语义一律保留
    /// （老工程里写过的 `pos` 照旧生效），只是**新事件不再会自动落到带上**。
    Auto,
}

impl TextAnchor {
    /// 认不出的一律 `Auto` —— 与贴纸同口径（"没指定" = "让后处理挑"）。
    ///
    /// ⚠️ 以前兜底是 `Bottom`（贴底）。改成 `Auto` 是因为 `pos` 已从界面去掉
    /// （用户口径「和贴纸同逻辑」）⇒ "字段缺失"是**常态**而不是"写坏了"，
    /// 老兜底会让所有没带 `pos` 的文字统统贴底、与"自动算"背道而驰。
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "top" => Self::Top,
            "center" => Self::Center,
            "bottom" => Self::Bottom,
            _ => Self::Auto,
        }
    }
}

/// 一段文字的**排版结果**：折行、字号、以及由此推出的盒子。
///
/// ⚠️ 这是 E3 的核心交付物 —— 在它之前，落点求解器只会算**固定 280px 的贴纸盒**，
/// 而文字是**变宽高**的（字号 + 折行）⇒ 不接上它，`pick_box_position` 会把
/// 一段三行的字按"一个 280 方块"去避让，结果是**大字压住主播脸**。
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayout {
    /// 折好行的文本（每行一条）。
    pub lines: Vec<String>,
    /// 实际字号（文档像素）。
    pub font_size: f64,
    /// 行高（文档像素）。
    pub line_height: f64,
    /// 盒子宽（文档像素）。
    pub w: f64,
    /// 盒子高（文档像素）。
    pub h: f64,
    /// 纵向锚点。
    pub anchor: TextAnchor,
    /// 样式预设。
    pub style: TextStyle,
}

/// 一个字符的**估算宽度**占字号的比例。
///
/// ⚠️ **这是估算，不是度量** —— 本仓不引字体度量库（那会把一个排版问题变成
/// 一个"字体文件在哪"的问题，而 `polish-render` 那条纪律是**不猜系统字体**）。
/// 取 1.0 是 CJK 全角的常见比例；ASCII 窄于它，所以**估出来偏宽**。
///
/// ⚠️ **偏宽是刻意的方向**：盒子算大 ⇒ 落点求解器避让得更多 ⇒
/// "字压住主播脸"更难发生。反方向（估窄）会让避让不足，而那正是本项要消灭的症状。
const CHAR_W_RATIO: f64 = 1.0;

/// 行高 / 字号的默认比例（与底座 `text_layout::LINE_HEIGHT_EM` 同值）。
const LINE_HEIGHT_EM: f64 = 1.2;

impl TextLayout {
    /// 把一段文字按给定样式与画布排出来。
    ///
    /// * `text`：原始文本（可含显式换行）；
    /// * `style`：四档预设；
    /// * `anchor`：纵向锚点；
    /// * `f`：输出帧（画布尺寸）。
    ///
    /// 折行规则与字幕**同一套常量**（`CAPTION_SAFE_WIDTH_RATIO` 等），
    /// 不是另发明一套 —— 两套折行规则会在"字幕装得下、大字装不下"这种边界上打架。
    pub fn solve(text: &str, style: TextStyle, anchor: TextAnchor, f: Frame) -> Self {
        use crate::project::{
            CAPTION_MAX_LINES, CAPTION_MIN_FONT_RATIO, CAPTION_SAFE_WIDTH_RATIO,
        };

        let max_w = f.w * CAPTION_SAFE_WIDTH_RATIO;
        // 字号用**比例**：像素在预览（640×360）与成片（1920×1080）里含义不同。
        let mut font_size = style.base_font_px() / FRAME_H * f.h;

        // 字号自适应：**先按原字号折行，超行数就按比例缩，再折一次**。
        //
        // ⚠️ 只缩不重折是错的：缩了字号之后每行能装更多字，行数会**变少** ——
        // 不重折会留下"缩过头的窄盒子"（字小了、行还没合并）。
        let mut lines = wrap_text(text, font_size, max_w);
        if lines.len() as u32 > CAPTION_MAX_LINES {
            let ratio = (CAPTION_MAX_LINES as f64 / lines.len() as f64).max(CAPTION_MIN_FONT_RATIO);
            font_size *= ratio;
            lines = wrap_text(text, font_size, max_w);
        }
        // 缩到下限仍超行数 ⇒ **截断**（与字幕的 `max_lines` 同语义：
        // 宁可少一行也不让它铺满全屏）。⚠️ 截断这件事调用方要如实报出来。
        lines.truncate(CAPTION_MAX_LINES as usize);
        if lines.is_empty() {
            lines.push(String::new());
        }

        let line_height = font_size * LINE_HEIGHT_EM;
        let text_w = lines
            .iter()
            .map(|l| display_width(l) * font_size * CHAR_W_RATIO)
            .fold(0.0_f64, f64::max);
        // 描边在外侧 ⇒ 盒子要把描边算进去，否则贴边时会切掉一圈。
        let stroke = style.base_stroke_px() / FRAME_H * f.h;
        let w = (text_w + stroke * 2.0).min(f.w).max(1.0);
        let h = (line_height * lines.len() as f64 + stroke * 2.0).min(f.h).max(1.0);

        Self { lines, font_size, line_height, w, h, anchor, style }
    }

    /// 送给落点求解器的盒子。
    pub fn box_size(&self) -> BoxSize {
        BoxSize::text(self.w, self.h)
    }

    /// 盒子左上角的目标位置（**未避让前**的锚点落点）。
    ///
    /// ⚠️ `top`/`bottom` 都留 `STICKER_MARGIN` 的边距 —— 与贴纸同一套安全边距，
    /// 两套边距会让"文字贴边"看起来像 bug。
    ///
    /// ⚠️ [`TextAnchor::Auto`] 返回**安全区中心**，是个"还没决定"的占位值：
    /// 真正的落点由 Step 1d3 之后的 2b2 走 `pick_box_position` 定
    /// （那一步在整块安全区里挑，与贴纸同一个求解器）—— 见 `mod.rs` 的
    /// `layout_text_elements` 注释「落点不在这里定」。
    /// 这里给中心而不是 `(0,0)`：万一有路径只读这个值，中心也是"没有偏好"的
    /// 最合理表达，不会把字甩到左上角。
    pub fn anchor_origin(&self, f: Frame) -> (f64, f64) {
        let margin = STICKER_MARGIN;
        let x = (f.w - self.w) / 2.0;
        let y = match self.anchor {
            TextAnchor::Top => margin,
            TextAnchor::Center | TextAnchor::Auto => (f.h - self.h) / 2.0,
            // 与字幕带错开：字幕在底，文字压上去两行都读不了。
            TextAnchor::Bottom => (f.h - self.h - margin - self.caption_clearance(f)).max(margin),
        };
        (x, y)
    }

    /// `bottom` 锚点时，文字底边**离画面底**要留多少 —— 也就是字幕带占掉的高度。
    ///
    /// ⚠️ **是「字幕带上沿到画面底」的距离，不是「字幕带自己的高」** ——
    /// 我第一版写成了后者（`band.y1 - band.y0`），结果是底部锚点的盒底落在 894、
    /// 而字幕带上沿在 856：**文字压住字幕**，正是本项要消灭的症状。
    /// 差的正是字幕带下方那 70px 留白（`bottom_off`）—— 那一段文字也进不去。
    ///
    /// ⚠️ 用 `band.y0` 反推而不是写死一个数：字幕带随画布与字号变，
    /// 写死会在竖屏上错位。
    fn caption_clearance(&self, f: Frame) -> f64 {
        let band = caption_band(f, 72.0 * f.kh(), 12.0 * f.kh());
        (f.h - band.y0).max(0.0)
    }
}

/// 按可用宽度折行。**显式换行优先**（用户写的换行不能被自动折行吃掉）。
///
/// 折行单位是**字符**（不是词）—— 中文没有词边界，按词折会得到
/// "这一行只有一个词、下一行十个词"那种参差结果。英文长词会被硬断，
/// 那是有意的（宁可断词也不溢出）。
fn wrap_text(text: &str, font_size: f64, max_w: f64) -> Vec<String> {
    let per_line = ((max_w / (font_size * CHAR_W_RATIO)).floor() as usize).max(1);
    let mut out = Vec::new();
    for para in text.split('\n') {
        let chars: Vec<char> = para.chars().collect();
        if chars.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut start = 0;
        while start < chars.len() {
            let end = (start + per_line).min(chars.len());
            out.push(chars[start..end].iter().collect());
            start = end;
        }
    }
    out
}

/// 一个字符串的**显示宽度**（按字符数；制表符算 4）。
///
/// ⚠️ 与底座 `text_layout` 的宽度口径**必须一致**，否则"本仓说装得下、
/// 底座画出来溢出"。底座用的是字符数 + 制表符展开，这里照抄。
fn display_width(s: &str) -> f64 {
    s.chars()
        .map(|c| if c == '\t' { 4.0 } else { 1.0 })
        .sum()
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

    /// **动态盒真的被求解器用了** —— 大盒子必须比小盒子让得更远。
    ///
    /// ⚠️ 这条是 §5 E3 的**核心判据**：本项存在的理由是"文字按真盒子避让，
    /// 而不是按固定的 280 方块"。判据若只验"求解器能跑"，把盒子换回
    /// `BoxSize::sticker` 也照样绿 —— 而那正是要防的回归。
    #[test]
    fn 大盒子比小盒子让得更远() {
        let blocked = vec![Blocked {
            rect: Rect::new(600.0, 300.0, 1320.0, 800.0),
            kind: ObstacleKind::Liver,
        }];
        let seed = 0.5;
        let small = BoxSize::text(120.0, 80.0);
        let big = BoxSize::text(900.0, 300.0);
        let ps = pick_box_position(&blocked, &[], seed, F, &small);
        let pb = pick_box_position(&blocked, &[], seed, F, &big);
        assert!(
            pb.clearance <= ps.clearance,
            "大盒子的余量 {} 反而大于小盒子的 {} —— 盒子尺寸没进求解器",
            pb.clearance,
            ps.clearance
        );
        // 至少有一维要真的被推得更远（否则"余量相同"可能只是都没碰到）。
        let moved = (pb.x - ps.x).abs() > 1.0 || (pb.y - ps.y).abs() > 1.0;
        assert!(
            moved,
            "大盒子与小盒子的落点完全相同（({}, {}) vs ({}, {})）—— 盒子尺寸没生效",
            pb.x, pb.y, ps.x, ps.y
        );
    }

    /// 排版出来的盒子**必须与求解器口径一致**（都按画布参数化）。
    ///
    /// ⚠️ 分开验"排版"与"求解"会漏掉它们**互不匹配**这一类错：
    /// 排版按文档像素算、求解按别的尺度算，两边各自单测都过，
    /// 合起来落点是错的。
    #[test]
    fn 排版盒与求解器同在文档像素口径() {
        let l = TextLayout::solve("测试文字", TextStyle::Chapter, TextAnchor::Top, F);
        let r = box_at(l.anchor_origin(F).0, l.anchor_origin(F).1, &l.box_size());
        assert!(
            (r.x1 - r.x0 - l.w).abs() < 1e-6 && (r.y1 - r.y0 - l.h).abs() < 1e-6,
            "盒子的矩形尺寸 ({}, {}) 与排版 ({}, {}) 不一致 —— 两者口径不同",
            r.x1 - r.x0,
            r.y1 - r.y0,
            l.w,
            l.h
        );
    }

    // ── §5 E3：文字排版 ────────────────────────────────────────────

    /// 四档**字号统一**，而"档位"这个词的意义由**其余三个维度**承载。
    ///
    /// # 这条判据是从 `四档字号从大到小且互不相等` 平移来的（2026-10-13）
    ///
    /// 老判据断言的**真意图**是"四档在画面上必须分得开" —— 当时承载它的是字号。
    /// 用户把字号固定成 80 之后，字号不再承担区分职责 ⇒ 判据的**观察点必须跟着平移**
    /// 到 `(描边, 底衬, 字重)` 这个三元组上。
    ///
    /// ⚠️ **不是"删掉挡路的判据"**：如果只把老断言删掉、不留替代，那么
    /// "四档全部变成一模一样的常量"这种改动会**全绿通过** —— 而那正是
    /// 「档位失去意义」这个症状的最坏形态（`style` 字段还在、但选哪档画出来都一样）。
    ///
    /// ⚠️ 判据仍然写**不等式/两两不等**而不是具体数值：值会随观感调，
    /// 而"四档两两可区分"是档位的定义。
    #[test]
    fn 四档字号统一而其余维度可区分() {
        // ① 字号：四档必须**全等于** `ELEMENT_FONT_PX`（用户口径：固定 80）。
        let all = [TextStyle::Hook, TextStyle::Chapter, TextStyle::Comment, TextStyle::Count];
        for ts in all {
            assert_eq!(
                ts.base_font_px(),
                ELEMENT_FONT_PX,
                "{ts:?} 的字号不是 {ELEMENT_FONT_PX} —— 用户要求四档统一固定为 80"
            );
        }
        // ② 区分度：把承载"档位"的三个维度打成一个可比较的键，要求**两两不等**。
        let keys: Vec<(u64, bool, u32)> = all
            .iter()
            .map(|ts| {
                (
                    ts.base_stroke_px().to_bits(),
                    ts.has_backdrop(),
                    ts.font_weight(),
                )
            })
            .collect();
        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                assert_ne!(
                    keys[i], keys[j],
                    "「{:?}」与「{:?}」在 (描边, 底衬, 字重) 上完全相同 —— \
                     字号已统一，这两档选哪一个画出来都一样，'档位'失去了意义",
                    all[i], all[j]
                );
            }
        }
        // ③ 描边必须落在规格区间（`elements-spec.md` §4.2：字号的 5–8%）。
        //    ⚠️ 只对**非 comment** 那三档要求 —— `comment` 是"吐槽"档，
        //       刻意用细边（2/80 = 2.5%），写成硬区间会把它误伤。
        for ts in [TextStyle::Hook, TextStyle::Chapter, TextStyle::Count] {
            let pct = ts.base_stroke_px() / ts.base_font_px();
            assert!(
                (0.05..=0.08).contains(&pct),
                "{ts:?} 的描边占字号 {:.1}%（{} px / {} px）—— 超出规格区间 5–8%",
                pct * 100.0,
                ts.base_stroke_px(),
                ts.base_font_px()
            );
        }
        // ④ 最粗的那档必须是 hook 且**严格**大于其余档 —— 老判据里
        //    "hook 是最大的"这条语义在这边对应"hook 的描边最粗"。
        let max_stroke = all
            .iter()
            .map(|t| t.base_stroke_px())
            .fold(f64::MIN, f64::max);
        assert_eq!(
            TextStyle::Hook.base_stroke_px(),
            max_stroke,
            "最粗的描边不在 hook 上 —— '钩子标题'这一档失去了它的视觉权重"
        );
        let second = all
            .iter()
            .filter(|t| **t != TextStyle::Hook)
            .map(|t| t.base_stroke_px())
            .fold(f64::MIN, f64::max);
        assert!(
            TextStyle::Hook.base_stroke_px() > second,
            "hook 的描边（{}）与次粗那档（{}）相等 —— 分不开",
            TextStyle::Hook.base_stroke_px(),
            second
        );
    }

    /// 认不出的 `style` 必须**退到 comment**，且 `is_known` 要如实说是"不认识"。
    ///
    /// ⚠️ 两条一起验：只验 `parse` 的话，"把 `is_known` 写成恒真"这种改动
    /// 会让"认不出"这件事**静默消失**（用户看不到那一行报告），
    /// 而 `parse` 仍然正确 —— 于是判据全绿、报告少一行。
    #[test]
    fn 认不出的样式退到吐槽档但会被报出来() {
        for bad in ["", "  ", "标题", "typo", "hooks"] {
            let parsed = TextStyle::parse(bad);
            assert_eq!(parsed, TextStyle::Comment, "`{bad}` 应当退到 comment");
            assert!(
                !TextStyle::is_known(bad),
                "`{bad}` 被 `is_known` 认成合法值了 —— 那行'认不出'的报告会消失"
            );
        }
        // 四个合法值必须全部认出来（大小写不敏感）。
        assert_eq!(TextStyle::parse("hook"), TextStyle::Hook);
        assert_eq!(TextStyle::parse(" CHAPTER "), TextStyle::Chapter);
        assert_eq!(TextStyle::parse("Count"), TextStyle::Count);
        assert_eq!(TextStyle::parse("comment"), TextStyle::Comment);
        assert!(TextStyle::is_known("HOOK"));
    }

    /// **排版结果必须随画布缩放**（比例语义）—— 否则竖屏上字会大得离谱。
    ///
    /// ⚠️ 这条抓的是"把字号写成像素常量"这类改动：横屏下它是对的，
    /// 竖屏下 `1080` 高的画布会得到与 `1920` 一样宽的字 ⇒ **溢出安全宽**。
    #[test]
    fn 字号随画布高度缩放() {
        let land = Frame { w: 1920.0, h: 1080.0 };
        let port = Frame { w: 1080.0, h: 1920.0 };
        let a = TextLayout::solve("标题", TextStyle::Hook, TextAnchor::Top, land);
        let b = TextLayout::solve("标题", TextStyle::Hook, TextAnchor::Top, port);
        assert!(
            (b.font_size / a.font_size - 1920.0 / 1080.0).abs() < 1e-9,
            "竖屏字号 {} 与横屏 {} 的比不是 1920/1080 —— 字号没按画布缩放",
            b.font_size,
            a.font_size
        );
    }

    /// 长文本必须**折行**，且折出来的每一行都装得进安全宽。
    ///
    /// ⚠️ "每行都装得下"是核心判据：只验"行数 > 1"会让
    /// "折了行但每行仍溢出"的改动通过 —— 而那正是本项要消灭的症状。
    #[test]
    fn 长文本折行且每行都在安全宽内() {
        let text = "这是一段相当长的文字".repeat(6);
        let l = TextLayout::solve(&text, TextStyle::Hook, TextAnchor::Top, F);
        assert!(l.lines.len() > 1, "这么长的文本应当折行，实得 {} 行", l.lines.len());
        let max_w = F.w * crate::project::CAPTION_SAFE_WIDTH_RATIO;
        for (i, line) in l.lines.iter().enumerate() {
            let w = display_width(line) * l.font_size * CHAR_W_RATIO;
            assert!(
                w <= max_w + 1e-6 + l.font_size,
                "第 {i} 行宽 {w} 超出安全宽 {max_w} —— 折行没按安全宽折"
            );
        }
    }

    /// 超行数上限时**缩字号**（不是直接截断），且缩到下限后才截。
    ///
    /// ⚠️ 底座字幕那条的语义是"缩到恰好装得下"（`shrink_min_scale`），
    /// 文字这条沿用 —— 直接截断会让用户写的字**凭空少一截**。
    ///
    /// ⚠️⚠️ **字数不能写死**（2026-10-13 修）：这里原来写 `"字".repeat(60)`，
    /// 注释理由是"120px 下约 5 行"。字号改成 80 之后同样 60 个字只有 **3 行**
    /// （每行 21 个），**根本不超上限** ⇒ 那条 `l.font_size < base_font_px()`
    /// 会红。而如果当时是"改成 64 个字凑够 4 行"，那**下一次**调字号它还会红 ——
    /// 判据就变成了"改动的障碍"。
    /// ⇒ 现在**按字号与安全宽现算**需要多少字才能超上限，字号再调也不会腐烂。
    #[test]
    fn 超行数先缩字号到最后才截断() {
        use crate::project::CAPTION_MAX_LINES;
        // 每行放得下几个字（与 `wrap_text` 同一条式子：`max_w / (font_size × CHAR_W_RATIO)`）。
        let max_w = F.w * crate::project::CAPTION_SAFE_WIDTH_RATIO;
        let font_size = TextStyle::Hook.base_font_px();
        let per_line = ((max_w / (font_size * CHAR_W_RATIO)).floor() as usize).max(1);
        // 取「刚好多一行」的字数（上限 +1 行）—— 保证一定超、且超得最少。
        let over = per_line * (CAPTION_MAX_LINES as usize + 1);
        assert!(
            over > per_line * CAPTION_MAX_LINES as usize,
            "算出来的超限字数（{over}）没超过 {CAPTION_MAX_LINES} 行 —— 判据的前提不成立"
        );
        let text = "字".repeat(over);
        let l = TextLayout::solve(&text, TextStyle::Hook, TextAnchor::Top, F);
        assert!(
            l.font_size < TextStyle::Hook.base_font_px(),
            "{} 个字在 {font_size}px 下排了 {} 行，超了上限却没缩字号（仍是 {}）—— 会直接丢字",
            over,
            over / per_line,
            l.font_size
        );
        assert!(
            l.lines.len() <= CAPTION_MAX_LINES as usize,
            "缩完之后仍有 {} 行（上限 {}）",
            l.lines.len(),
            CAPTION_MAX_LINES
        );
        // 缩到下限仍超 ⇒ 截断到上限。
        let huge = "字".repeat(400);
        let l2 = TextLayout::solve(&huge, TextStyle::Hook, TextAnchor::Top, F);
        assert_eq!(
            l2.lines.len(),
            crate::project::CAPTION_MAX_LINES as usize,
            "极端长文本必须截到上限，实得 {}",
            l2.lines.len()
        );
    }

    /// 显式换行**不能被自动折行吃掉**。
    ///
    /// ⚠️ 用户写两行是**有意的排版意图**；自动折行把两行并成一行
    /// （或把一行拆成三行）会让"我明明写了两行"变成"成片里不是那样"。
    #[test]
    fn 显式换行被保留() {
        let l = TextLayout::solve("第一行\n第二行\n第三行", TextStyle::Comment, TextAnchor::Top, F);
        assert_eq!(l.lines.len(), 3, "三条显式换行应当得到 3 行，实得 {:?}", l.lines);
        assert_eq!(l.lines[0], "第一行");
        assert_eq!(l.lines[2], "第三行");
    }

    /// 盒子必须**把描边算进去**（外侧宽度），且随样式变。
    ///
    /// ⚠️ 描边是**外侧**的（与 `SubtitleStyle::stroke_ratio` 同口径）——
    /// 不计入盒子的话，贴边时会被切掉一圈，而那是"看起来像裁切 bug"、
    /// 实际是"盒子算小了"。
    #[test]
    fn 盒子含外侧描边且随样式变() {
        let short = TextLayout::solve("字", TextStyle::Hook, TextAnchor::Top, F);
        let thin = TextLayout::solve("字", TextStyle::Comment, TextAnchor::Top, F);
        assert!(
            short.w > thin.w,
            "hook（粗描边 + 大字号）的盒子应当比 comment 宽：{} vs {}",
            short.w,
            thin.w
        );
        // 描边只算一次、且是两侧各一份：盒宽 ≥ 文本宽 + 2×描边。
        let stroke = TextStyle::Hook.base_stroke_px() / FRAME_H * F.h;
        let text_w = display_width("字") * short.font_size;
        assert!(
            (short.w - (text_w + stroke * 2.0)).abs() < 1e-6,
            "盒宽 {} 与 文本 {text_w} + 2×描边 {stroke} 对不上",
            short.w
        );
    }

    /// 三个纵向锚点**互不相同**、且底部那个**与字幕带错开**。
    ///
    /// ⚠️ 最后那条是本项的目标之一（规格 §8："文字不压主播脸、不与字幕带重叠"）：
    /// 底部锚点若贴到 `H - h - margin`，文字会正好压在字幕上 —— 两行都读不了。
    #[test]
    fn 三个纵向锚点分开且底部让开字幕带() {
        let top = TextLayout::solve("字", TextStyle::Chapter, TextAnchor::Top, F);
        let mid = TextLayout::solve("字", TextStyle::Chapter, TextAnchor::Center, F);
        let bot = TextLayout::solve("字", TextStyle::Chapter, TextAnchor::Bottom, F);
        let (_, ty) = top.anchor_origin(F);
        let (_, my) = mid.anchor_origin(F);
        let (_, by) = bot.anchor_origin(F);
        assert!(ty < my, "top({ty}) 应当在 center({my}) 之上");
        assert!(my < by, "center({my}) 应当在 bottom({by}) 之上");
        // 底部让开字幕带：盒底必须在字幕带上沿之上。
        let band = band_caption();
        assert!(
            by + bot.h <= band.y0 + 1e-6,
            "底部锚点的盒底 {} 侵入了字幕带（带上沿 {}）—— 文字会压住字幕",
            by + bot.h,
            band.y0
        );
    }

    /// **`Auto` 是"没偏好"**（2026-10-12，用户口径「和贴纸同逻辑」）。
    ///
    /// 面板撤掉「纵向位置」下拉之后，`pos` 字段缺失是**常态**，
    /// 所以 `parse("")` 必须落到 `Auto` —— 落成 `Bottom` 会让所有没带 `pos`
    /// 的文字统统贴底，与"让后处理按安全落点算"正好相反，**而且不报错**。
    ///
    /// ⚠️ 这条钉**两件事**，它们会各自单独坏：
    ///   ① `parse` 的兜底档（认不出 = 让后处理挑）；
    ///   ② `Auto` 的占位落点**不是**任何一条实档 —— 它用中线，且**不**走
    ///      `Bottom` 那条"减字幕带"的路（那会让自动档偷偷继承贴底语义）。
    #[test]
    fn 认不出的纵向档落到自动且占位取中线() {
        // ① 兜底：空串、脏值、大小写混杂，一律 `Auto`。
        assert_eq!(TextAnchor::parse(""), TextAnchor::Auto);
        assert_eq!(TextAnchor::parse("  "), TextAnchor::Auto);
        assert_eq!(TextAnchor::parse("middle"), TextAnchor::Auto);
        assert_eq!(TextAnchor::parse("AUTO"), TextAnchor::Auto);
        // ⚠️ 三条实档**必须仍认** —— 老工程里写过的 `pos` 照旧生效，
        //    这次改的是"不写时怎么办"，不是"不认它们了"。
        assert_eq!(TextAnchor::parse("top"), TextAnchor::Top);
        assert_eq!(TextAnchor::parse("center"), TextAnchor::Center);
        assert_eq!(TextAnchor::parse("bottom"), TextAnchor::Bottom);

        // ② 占位落点 = 中线，与 `Center` 逐值相同、且**不等于** `Bottom`。
        let auto = TextLayout::solve("字", TextStyle::Chapter, TextAnchor::Auto, F);
        let mid = TextLayout::solve("字", TextStyle::Chapter, TextAnchor::Center, F);
        let bot = TextLayout::solve("字", TextStyle::Chapter, TextAnchor::Bottom, F);
        let (ax, ay) = auto.anchor_origin(F);
        let (mx, my) = mid.anchor_origin(F);
        let (_, by) = bot.anchor_origin(F);
        assert!((ay - my).abs() < 1e-9, "自动档的占位应当就是中线：{ay} vs {my}");
        assert!((ax - mx).abs() < 1e-9, "自动档的横向占位也该是中线：{ax} vs {mx}");
        assert!(
            (ay - by).abs() > 1.0,
            "自动档的占位**不能**等于贴底档（{ay} vs {by}）—— \
             那等于让『自动』偷偷继承贴底语义"
        );
    }

    /// `BoxSize::text` 用的就是排版结果（不是默认值）。
    ///
    /// ⚠️ 这条抓的是"排版算了、但求解器仍拿固定 280 盒"这类改动 ——
    /// 那正是本项存在的原因（大字压住主播脸），而**从代码上看不出来**。
    #[test]
    fn 送给求解器的盒子就是排版结果() {
        let l = TextLayout::solve("一段挺长的标题文字", TextStyle::Hook, TextAnchor::Top, F);
        let b = l.box_size();
        assert_eq!(b.w, l.w);
        assert_eq!(b.h, l.h);
        assert_eq!(b.float_up, 0.0, "文字没有入场上浮（与贴纸不同）");
        assert!(
            (b.w - crate::process::placement::STICKER_SIZE).abs() > 1.0
                || (b.h - crate::process::placement::STICKER_SIZE).abs() > 1.0,
            "文字盒恰好等于贴纸的 280×280 —— 那多半是没接上排版结果"
        );
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

    // ---- T3：倾斜 / 纵向压缩的值域（可读性硬约束）----

    /// 档位名（只给断言消息用；`process::style_name` 是私有的，别为测试放开它）。
    fn label(s: TextStyle) -> &'static str {
        match s {
            TextStyle::Hook => "hook",
            TextStyle::Chapter => "chapter",
            TextStyle::Comment => "comment",
            TextStyle::Count => "count",
        }
    }

    /// 四档的参照表（加字段时要一起想：**这一档歪/压真的合适吗**）。
    const ALL_STYLES: [TextStyle; 4] = [
        TextStyle::Hook,
        TextStyle::Chapter,
        TextStyle::Comment,
        TextStyle::Count,
    ];

    /// ★ **倾斜不得越过可读性上限 ±7°**。
    ///
    /// ⚠️ 这条是**值域门禁**，不是"某个数字对不对"：将来谁想把 Hook 调到 10°，
    /// 必须**先改这条判据并说明理由** —— 而不是悄悄越过去。
    #[test]
    fn tilt_within_readability_limit() {
        for s in ALL_STYLES {
            let t = s.tilt_deg();
            assert!(
                (-7.0..=7.0).contains(&t),
                "{} 的倾斜 {t}° 越过 ±7° 可读性上限",
                label(s)
            );
        }
    }

    /// ★ **纵向压缩不得小于 0.85**（同上，值域门禁）。
    #[test]
    fn squash_within_readability_limit() {
        for s in ALL_STYLES {
            let q = s.squash_y();
            assert!(
                (0.85..=1.0).contains(&q),
                "{} 的纵向压缩 {q} 越界（须在 [0.85, 1.0]）",
                label(s)
            );
        }
    }

    /// **兜底档（`Comment`）必须是最保守的：不歪、不压。**
    ///
    /// ⚠️ 这条钉的是"认不出的 `style` 会退化成 `Comment`"这条链——
    /// 若哪天把 `Comment` 调成歪的，那所有写错 style 的事件都会跟着歪，
    /// 而用户从任何界面都看不出那个歪是从哪来的。
    #[test]
    fn fallback_style_is_the_most_conservative() {
        assert_eq!(TextStyle::parse("认不出的档位"), TextStyle::Comment);
        assert_eq!(TextStyle::Comment.tilt_deg(), 0.0, "兜底档不许歪");
        assert_eq!(TextStyle::Comment.squash_y(), 1.0, "兜底档不许压");
    }

    /// 四档两两之间**至少有一项不同** —— 防"加了字段但四档写成一模一样"。
    #[test]
    fn the_four_styles_are_not_all_identical() {
        let tilts: Vec<f64> = ALL_STYLES.iter().map(|s| s.tilt_deg()).collect();
        let squashes: Vec<f64> = ALL_STYLES.iter().map(|s| s.squash_y()).collect();
        assert!(
            tilts.iter().any(|t| *t != tilts[0]) || squashes.iter().any(|q| *q != squashes[0]),
            "四档的 (tilt, squash) 完全相同 —— 那这两个字段等于没加：{tilts:?} / {squashes:?}"
        );
    }
}
