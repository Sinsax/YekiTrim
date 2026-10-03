//! polish::project::config：`polish.toml` 的类型真源（`ProjectFile` 与各 section）与画面构成推导、时间轴工具函数。
//!
//! 入口：`ProjectFile`、`MuteRange`、`CoverConfig`、`CoverEffect`、`FilterConfig`、`SourceRef`、`CanvasConfig`、`StageConfig`、`StickerConfig`、`StageFit`；`resolve_stage`、`to_toml_commented`、`clamp_segments_to_duration`。
//! 边界：`resolve_stage` 是画面构成的**唯一推导**（模板/编辑器只读注入结果）；`CANVAS_*` / `CAPTION_*` / `STICKER_SAFE_MARGIN` 等常量与前端 JS 是镜像，改一边要改另一边 —— 见 `crates/yekitrim-polish/src/project/AGENTS.md`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 完整项目文件 (polish.toml)
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ProjectFile {
    /// 1. 项目信息
    #[serde(default)]
    pub project: ProjectMeta,

    /// 2. 素材路径
    pub source: SourceRef,

    /// 3. 封面
    #[serde(default)]
    pub cover: CoverConfig,

    /// 4. 关键词过滤 (从 polish.toml 读取)
    ///    LLM API 配置使用 .yekitrim/llm.toml（与 yekitrim 自身一致）
    #[serde(default)]
    pub filter: FilterConfig,

    /// 5. 视频时长 (可选, 从 clip.json 读取)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,

    /// 6. 直播间布局
    #[serde(default)]
    pub layout: Layout,

    /// 7. 输出画布（方向 → 宽高，见 [`CanvasConfig::size`]）
    #[serde(default)]
    pub canvas: CanvasConfig,

    /// 8. 画面构成（主画面位置 / 背景）
    #[serde(default)]
    pub stage: StageConfig,

    /// 9. 贴纸落点策略（**贴图来源**仍是根级的 `sticker_source`，不在这个表里）
    #[serde(default)]
    pub sticker: StickerConfig,

    /// 10. 视觉样式
    #[serde(default)]
    pub style: StyleRoot,

    /// 11. 贴图来源配置
    #[serde(default = "default_sticker_source")]
    pub sticker_source: Vec<String>,

    /// 12. 音效
    #[serde(default)]
    pub sfx: SfxConfig,

    /// 13. 时间轴
    #[serde(default)]
    pub segments: Vec<Segment>,

    /// 14. 视频屏蔽区间：这些时间段会被预览/导出跳过，LLM 也会忽略其中内容
    #[serde(default)]
    pub mute_ranges: Vec<MuteRange>,

    /// 15. 「去静音」自动检测出的区间记录（对应时间轴「屏蔽」轨道里的红色块）
    ///     仅作记录：撤销去静音时按它精确回退，不影响手动添加的屏蔽区间
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub silence_ranges: Vec<MuteRange>,

    /// 是否已执行过 process（preview 用它判断是否需要先 process）
    #[serde(default, skip_serializing_if = "is_false")]
    pub processed: bool,

    /// 16. 片头淡入（`overlay_type = "fade_in"`）的补种时机**已经用掉**：初次 process 判定过片头之后置位，
    ///     往后这条事件在不在、长什么样都由用户说了算 —— 删掉再跑 process 也不会补回来（那是用户意图，
    ///     不是"默认观感没生效"）。想重新拿回默认片头：删掉这一行再跑 process。
    ///     判定细节与"什么时候不算用掉"见 `process::ensure_intro_fade_in`。
    #[serde(default, skip_serializing_if = "is_false")]
    pub intro_fade_seeded: bool,

    /// 扩展兜底
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// 一个屏蔽区间：start~end 之间的内容会被手动剪掉。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MuteRange {
    pub start: f64,
    pub end: f64,
}

impl MuteRange {
    pub fn normalized(&self) -> Option<(f64, f64)> {
        let s = self.start.min(self.end).max(0.0);
        let e = self.start.max(self.end);
        if e - s > 0.001 { Some((s, e)) } else { None }
    }

    pub fn overlaps(&self, start: f64, end: f64) -> bool {
        let Some((s, e)) = self.normalized() else { return false; };
        start < e && end > s
    }
}

fn is_false(v: &bool) -> bool {
    !*v
}

// ===== 项目元信息 =====

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectMeta {
    #[serde(default)]
    pub name: String,

    #[serde(default = "default_output")]
    pub output: String,

    #[serde(default = "default_created")]
    pub created: String,

    #[serde(default = "default_version")]
    pub version: u32,
}

// ===== 样式根 =====

// ===== 运镜样式 =====

// ===== 关键词过滤 =====

#[derive(Debug, Serialize, Deserialize, Clone)]
#[derive(Default)]
pub struct FilterConfig {
    #[serde(default)]
    pub danmaku: Vec<String>,
    #[serde(default)]
    pub highlight: Vec<String>,
}


fn default_created() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn default_version() -> u32 {
    1
}

fn default_sticker_source() -> Vec<String> {
    vec!["emoji".into()]
}

impl Default for ProjectMeta {
    fn default() -> Self {
        Self {
            name: String::new(),
            output: default_output(),
            created: default_created(),
            version: default_version(),
        }
    }
}

// ===== 素材引用 =====

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub video: String,
    pub subtitles: String,
    #[serde(default)]
    pub danmaku: String,
    /// clip.json 路径 (可选, 提供 duration/title/event_type)
    #[serde(default = "default_metadata")]
    pub metadata: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

// ===== 布局 =====
// （类型定义已迁入 yekitrim-render-core/types.rs，此处重导出保持兼容）
pub use yekitrim_render_core::types::{
    free_rect, grid_rect, region_origin, region_rects, parse_pct, CameraStyle, DanmakuStyle, Event,
    Layout,
    LayoutRegion, Point, Segment, StyleRoot, SubtitleStyle,
};

// ===== 九宫格 =====

/// 兼容旧名：多个格子合并为包围盒（已废弃，保留别名避免破坏外部引用）。
#[deprecated(note = "请改用 region_rects，多个格子不再合并")]
pub fn region_rect(region: &LayoutRegion) -> Option<(Point, Point)> {
    let rects = region_rects(region);
    if rects.is_empty() {
        return None;
    }
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    for (tl, br) in &rects {
        min_x = min_x.min(parse_pct(&tl.x));
        min_y = min_y.min(parse_pct(&tl.y));
        max_x = max_x.max(parse_pct(&br.x));
        max_y = max_y.max(parse_pct(&br.y));
    }
    let pt = |x: f64, y: f64| Point {
        x: format!("{x:.2}%"),
        y: format!("{y:.2}%"),
    };
    Some((pt(min_x, min_y), pt(max_x, max_y)))
}


// ===== 字幕样式 =====

// ===== 弹幕样式 =====

// ===== 音效配置 =====

fn default_sfx_volume() -> f64 { 0.1 }

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct SfxConfig {
    /// 全局默认音效音量 (0.0~1.0)，事件级 sfx_volume/volume 可覆盖
    #[serde(default = "default_sfx_volume")]
    pub volume: f64,

    #[serde(default = "default_true")]
    pub reactions_enabled: bool,

    #[serde(default = "default_true")]
    pub shake_enabled: bool,

    /// 音效屏蔽列表: 精确 id 屏蔽单个音效, "tag:xxx" 屏蔽整个标签族
    /// (与全局 .yekitrim/config.toml [sfx] blacklist 合并, 此处追加)
    #[serde(default)]
    pub blacklist: Vec<String>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for SfxConfig {
    fn default() -> Self {
        Self {
            volume: default_sfx_volume(),
            reactions_enabled: true,
            shake_enabled: true,
            blacklist: Vec::new(),
            extra: HashMap::new(),
        }
    }
}

fn default_true() -> bool { true }

// ===== 封面配置 =====

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CoverConfig {
    #[serde(default)]
    pub image: String,
    /// 封面标题字体：字体库（`<程序目录>/resources/fonts/`）里的 id，空 = 系统字体栈。
    /// 与 `style.subtitle.font` 同一套取值规则（见 `yekitrim_core::fonts`）。
    #[serde(default)]
    pub font: String,
    #[serde(default = "default_cover_frame_pos")]
    pub frame_pos: f64,
    #[serde(default = "default_cover_font_size")]
    pub font_size: u32,
    #[serde(default = "default_cover_stroke")]
    pub stroke: u32,
    #[serde(default = "default_cover_hl_color")]
    pub hl_color: String,
    /// 高亮词的相对字号（1.0 = 与标题同号）。只作用于 `<span class="hl">` 那几段，
    /// 出片模板与设计器用**同一个** `--hl-fs` 变量（镜像清单里的一条）。
    #[serde(default = "default_cover_hl_font_scale")]
    pub hl_font_scale: f64,
    #[serde(default = "default_cover_text_width")]
    pub text_width: f64,
    /// 文字中心点百分比, 如 "30%"
    #[serde(default = "default_cover_text_x")]
    pub text_x: String,
    #[serde(default = "default_cover_text_y")]
    pub text_y: String,
    /// 渐变方向: left/right/top/bottom/radial
    #[serde(default = "default_cover_gradient_dir")]
    pub gradient_dir: String,
    /// 暗角强度 0~1
    #[serde(default = "default_cover_vignette")]
    pub vignette: f64,
    #[serde(default = "default_cover_letter_spacing")]
    pub letter_spacing: String,
    #[serde(default = "default_cover_line_height")]
    pub line_height: f64,
    /// 手动断行 (由 cover 命令自动生成, 可手动编辑)
    #[serde(default)]
    pub title_lines: Vec<String>,
    /// 文字对齐: left/center/right
    #[serde(default = "default_cover_text_align")]
    pub text_align: String,
    /// 阴影方向: none/left/right/top/bottom/center
    #[serde(default = "default_cover_shadow_dir")]
    pub shadow_dir: String,
    /// 阴影强度 0~1（0 = 无阴影；旧字段，保留兼容）
    #[serde(default = "default_cover_shadow_intensity")]
    pub shadow_intensity: f64,
    /// 文字阴影开关（新的语义：文字阴影只控制开关，方向由背景渐变控制）
    #[serde(default = "default_true")]
    pub text_shadow: bool,
    /// 封面标题高亮关键词
    #[serde(default)]
    pub highlight_keywords: Vec<String>,
    /// 可叠加封面效果
    #[serde(default)]
    pub effects: Vec<CoverEffect>,
    /// 帧层（当前帧清晰图）的位移/缩放：**活字段**，写进模板 `#frame img` 的 transform。
    /// 背景（纯色 / 模糊帧）铺在帧**之下**，这几个值和背景无关；设计器拖预览/滚轮就是改它们。
    #[serde(default)]
    pub bg_x: f64,
    #[serde(default)]
    pub bg_y: f64,
    /// 帧层缩放倍率（在 `layout.liver` 聚焦基准 1.3/1.4 之上再乘）
    #[serde(default = "default_cover_bg_zoom")]
    pub bg_zoom: f64,
    /// 背景模式: color（纯色，默认黑）/ blur（当前帧模糊平铺）。
    /// 只认这两个值，其它（含老的 frame）一律当 color —— 背景不再有"截帧"模式。
    #[serde(default = "default_cover_bg_mode")]
    pub bg_mode: String,
    /// 纯色背景的颜色（bg_mode = color；同时是 blur 模式漏边时的兜底色）
    #[serde(default = "default_cover_bg_color")]
    pub bg_color: String,
    /// 模糊平铺的模糊倍率（bg_mode = blur，1.0 = 基准 28px）
    #[serde(default = "default_cover_blur_scale")]
    pub blur_scale: f64,
    /// **图层栈**（从下到上）：顺序 + 可见性。空 = 老工程 → `cover::default_cover_stack()`（零迁移）。
    /// id：`bg` / `frame` / `filter`（渐变+暗角，一层两块）/ `title` / `fx:<CoverEffect::id>`。
    ///
    /// ⚠️ **声明在最后**：`to_toml_commented` 的行扫描碰到 `[[cover.layers]]` 就会停止给 `[cover]`
    /// 标注释，放最后才不会让前面的字段一起掉注释（`effects` 那条老问题同理）。
    #[serde(default)]
    pub layers: Vec<CoverLayer>,
}

/// 图层栈的一项（`[[cover.layers]]`）。
///
/// **顺序 = 数组顺序，从下到上**；`visible = false` = 这一层还在（属性一个字节都不动），只是不画。
/// 解析（补齐/忽略未知 id/默认可见）只在 `cover::resolve_cover_stack()` 一处做，前端是镜像。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CoverLayer {
    #[serde(default)]
    pub id: String,
    #[serde(default = "default_true")]
    pub visible: bool,
}

impl Default for CoverLayer {
    fn default() -> Self {
        Self { id: String::new(), visible: true }
    }
}

fn default_cover_bg_zoom() -> f64 { 1.0 }
fn default_cover_bg_mode() -> String { "color".into() }
fn default_cover_bg_color() -> String { "#000000".into() }
fn default_cover_blur_scale() -> f64 { 1.0 }
fn default_cover_effect_color() -> String { "#000000".into() }

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CoverEffect {
    /// 稳定 id：`[cover] layers` 用 `fx:<id>` 指向它，也是设计器图层列表的 key。
    /// 老数据为空 → `cover::resolve_cover_stack` 按数组序合成槽位（**不回写**，老工程零迁移）。
    #[serde(default)]
    pub id: String,
    pub kind: String,
    #[serde(default = "default_cover_effect_amount")]
    pub amount: f64,
    #[serde(default = "default_cover_effect_x")]
    pub x: f64,
    #[serde(default = "default_cover_effect_y")]
    pub y: f64,
    /// overlay 等特效使用的颜色
    #[serde(default = "default_cover_effect_color")]
    pub color: String,
    /// kind = emoji 时贴的那串表情字符（会做 HTML 转义后再进模板）
    #[serde(default)]
    pub text: String,
}
fn default_cover_frame_pos() -> f64 { 0.4 }
fn default_cover_font_size() -> u32 { 120 }
fn default_cover_stroke() -> u32 { 20 }
fn default_cover_hl_color() -> String { "#ffcc00".into() }

/// 高亮词与标题同号 = 1.0（老工程零回归：不写这个键就是原来的样子）
fn default_cover_hl_font_scale() -> f64 { 1.0 }
fn default_cover_text_width() -> f64 { 0.62 }
fn default_cover_text_x() -> String { "35%".into() }
fn default_cover_text_y() -> String { "50%".into() }
fn default_cover_gradient_dir() -> String { "left".into() }
fn default_cover_vignette() -> f64 { 0.08 }
fn default_cover_letter_spacing() -> String { "8px".into() }
fn default_cover_line_height() -> f64 { 1.7 }
/// 默认左对齐：与默认预设「左文」（`CoverDesigner.vue` 的 presets、
/// `cover.rs::cmd_cover_template` 的 left-text 分支）以及默认 `text_x = "35%"` 一致 ——
/// 这三处以前是「预设左文、配置居中」，自相矛盾。
/// 注意：已有工程的 `polish.toml` 里显式写着 `text_align = "center"`，字段存在就不走默认，不受影响。
fn default_cover_text_align() -> String { "left".into() }
fn default_cover_shadow_dir() -> String { "left".into() }
fn default_cover_shadow_intensity() -> f64 { 1.0 }
fn default_cover_effect_amount() -> f64 { 0.3 }
fn default_cover_effect_x() -> f64 { 50.0 }
fn default_cover_effect_y() -> f64 { 50.0 }
impl Default for CoverConfig {
    fn default() -> Self {
        Self {
            image: String::new(),
            font: String::new(),
            frame_pos: default_cover_frame_pos(),
            font_size: default_cover_font_size(),
            stroke: default_cover_stroke(),
            hl_color: default_cover_hl_color(),
            hl_font_scale: default_cover_hl_font_scale(),
            text_width: default_cover_text_width(),
            text_x: default_cover_text_x(),
            text_y: default_cover_text_y(),
            gradient_dir: default_cover_gradient_dir(),
            vignette: default_cover_vignette(),
            letter_spacing: default_cover_letter_spacing(),
            line_height: default_cover_line_height(),
            title_lines: Vec::new(),
            text_align: default_cover_text_align(),
            shadow_dir: default_cover_shadow_dir(),
            shadow_intensity: default_cover_shadow_intensity(),
            text_shadow: true,
            highlight_keywords: Vec::new(),
            effects: Vec::new(),
            bg_x: 0.0,
            bg_y: 0.0,
            bg_zoom: default_cover_bg_zoom(),
            bg_mode: default_cover_bg_mode(),
            bg_color: default_cover_bg_color(),
            blur_scale: default_cover_blur_scale(),
            layers: Vec::new(),
        }
    }
}

// ===== 输出画布 / 画面构成（竖屏支持）=====

/// 横画布（默认）：1920×1080。
pub const CANVAS_LANDSCAPE_WH: (u32, u32) = (1920, 1080);
/// 竖画布 9:16：1080×1920。
pub const CANVAS_PORTRAIT_WH: (u32, u32) = (1080, 1920);
/// 字幕安全区宽度占画布宽度的比例（剩下的是左右留白）。
///
/// ⚠️ 1920 下正好等于改造前的 `.captions{left:120; width:1680}`（120 = 1920×(1-0.875)/2），
/// 所以**横画布下逐像素不变**。模板的 `_canvas.caps` 与 WebUI 的 `canvas.js` 都从这里取。
pub const CAPTION_SAFE_WIDTH_RATIO: f64 = 0.875;
/// 字幕最多折几行（超了就按比例缩字号，见 [`CAPTION_MIN_FONT_RATIO`]）。
pub const CAPTION_MAX_LINES: u32 = 3;
/// 折行后仍超行数上限时的最小字号倍率。
pub const CAPTION_MIN_FONT_RATIO: f64 = 0.7;
/// 贴纸安全边距（px，1920×1080 基准）—— 与 `process/placement.rs` 的 `STICKER_MARGIN` 同源。
pub const STICKER_SAFE_MARGIN: f64 = 32.0;

/// 输出画布。**只给方向，宽高由 [`CanvasConfig::size`] 推导**（不做分辨率档位、不做自定义宽高）。
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct CanvasConfig {
    /// `landscape`（默认，1920×1080）| `portrait`（1080×1920，9:16）。其它值一律当 landscape。
    #[serde(default = "default_canvas_orientation")]
    pub orientation: String,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

fn default_canvas_orientation() -> String {
    "landscape".into()
}

impl Default for CanvasConfig {
    fn default() -> Self {
        Self {
            orientation: default_canvas_orientation(),
            extra: HashMap::new(),
        }
    }
}

impl CanvasConfig {
    pub fn is_portrait(&self) -> bool {
        self.orientation.trim().eq_ignore_ascii_case("portrait")
    }

    /// 输出画布尺寸 —— **唯一推导点**。
    pub fn size(&self) -> (u32, u32) {
        if self.is_portrait() {
            CANVAS_PORTRAIT_WH
        } else {
            CANVAS_LANDSCAPE_WH
        }
    }
}

/// 画面构成：源视频比例 ≠ 画布比例时，主画面怎么摆、空出来的地方补什么。
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct StageConfig {
    /// 背景：`auto`（默认）/ `black` / `color` / `blur`。其它值（含老配置里的 `frame`）一律当 `auto`。
    #[serde(default = "default_stage_bg_mode")]
    pub bg_mode: String,

    /// `bg_mode = color` 时的颜色（注入模板前经 [`crate::cover::sanitize_hex_color`] 净化）。
    #[serde(default = "default_cover_bg_color")]
    pub bg_color: String,

    /// 模糊倍率（`bg_mode = blur`，1.0 = 基准 28px，区间与 `[cover].blur_scale` 同一套）。
    #[serde(default = "default_cover_blur_scale")]
    pub blur_scale: f64,

    /// **主画面位置 = 画布内的对齐百分比**（0 = 贴左 / 贴上，50 = 居中，100 = 贴右 / 贴下）。
    ///
    /// UI 是**五档吸附**（0/25/50/75/100）；数据模型留 0–100 连续，是为了以后加档位不必再动 schema。
    /// 缺省 `None` = **老工程**：按 [`stage_position_pct`] 从老枚举 [`StageConfig::position`] 换算 ——
    /// **`auto` 一律掉档为 50 居中**（2026-10-02 用户口径），`left`/`top` → 0、`right`/`bottom` → 100。
    #[serde(default)]
    pub position_pct: Option<f64>,

    /// ⚠️ **老字段（只读兼容，不再写出）**：2026-10-02 起位置的权威是 [`StageConfig::position_pct`]。
    /// 保留它只为读得懂老 `polish.toml`；`skip_serializing` 保证新工程/再次保存的工程不再写这一行。
    /// 老语义映射见 [`stage_position_pct`]（`auto` = 居中，不再是「竖素材靠左」）。
    #[serde(default = "default_stage_position", skip_serializing)]
    pub position: String,

    /// 主画面缩放倍率（1.0 = contain，完整可见不裁切）。
    #[serde(default = "default_stage_scale")]
    pub scale: f64,

    /// 主画面额外留白（px，横画布基准，按 `k` 缩放）。
    ///
    /// ⚠️ **字段保留、UI 不再出现**（2026-10-02）：它与 [`StageConfig::scale`] 不是同一个量
    /// （留白改的是「可用空间」、scale 是绕中心缩放，非居中位置下位移轨迹不同，**无法无损折算**）——
    /// 所以老工程继续按原值渲染，界面只留「缩放比例」。而 `[stage]` 是 `deny_unknown_fields`，
    /// 删字段会让带 `padding` 的老 toml **解析失败**，这也是它必须留着的硬理由。
    #[serde(default)]
    pub padding: u32,

    /// **竖屏画布**下是否渲染**特效**事件（`flash/blur/hue_shift/noise/vignette/overlay/pulse/shake`）。
    ///
    /// ⚠️ **缺字段 = false = 关**（2026-10-02 用户口径：竖屏默认全关，**老工程也一并关**）。
    /// 语义 = **按整类事件屏蔽**（整轨），只影响渲染与预览求值，**绝不改事件** ——
    /// 与「屏蔽只做标记」同一条纪律（打开即完全恢复）。字幕 / 弹幕 / 贴纸**不受它影响**。
    #[serde(default)]
    pub portrait_effects: bool,

    /// **竖屏画布**下是否渲染**运镜**（`Event::Camera`）。缺字段 = 关。
    /// ⚠️ 与「竖素材早退」（`CFG.vertical`，按源 `h > w`）是**两条独立的闸**，任一要求禁就禁。
    #[serde(default)]
    pub portrait_camera: bool,

    /// **竖屏画布**下是否混入**音效**（`Event::Sfx`，含事件引用的 `sfx` / `sfx_volume`）。缺字段 = 关。
    /// 收口在 `crate::render::sound::collect_sound_events`（cmd_render 的唯一 SFX 入口）。
    #[serde(default)]
    pub portrait_sfx: bool,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// ⚠️ 2026-10-02 起缺省是 **`blur`**（UI 删掉了「自动」按钮 —— 在 `resolve_stage` 里
/// `auto` 与 `blur` **完全等价**：需要背景时都是当前帧模糊，不需要时都是 `none`）。
/// 老工程显式写的 `bg_mode = "auto"` 读进来仍按 blur 处理（[`stage_bg_mode`]），零迁移。
fn default_stage_bg_mode() -> String {
    "blur".into()
}
fn default_stage_position() -> String {
    "auto".into()
}
fn default_stage_scale() -> f64 {
    1.0
}

impl Default for StageConfig {
    fn default() -> Self {
        Self {
            bg_mode: default_stage_bg_mode(),
            bg_color: default_cover_bg_color(),
            blur_scale: default_cover_blur_scale(),
            position_pct: None,
            position: default_stage_position(),
            scale: default_stage_scale(),
            padding: 0,
            portrait_effects: false,
            portrait_camera: false,
            portrait_sfx: false,
            extra: HashMap::new(),
        }
    }
}

/// 贴纸落点策略。`auto` = 沿用安全区算法（逐条变化）；`manual` = 渲染时固定到 `position`。
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct StickerConfig {
    /// `auto`（默认）| `manual`。其它值一律当 `auto`。
    #[serde(default = "default_sticker_position_mode")]
    pub position_mode: String,

    /// `manual` 的固定落点：百分比，相对输出画布，中心基准，如 `"50% 50%"`。
    #[serde(default = "default_sticker_position")]
    pub position: String,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

fn default_sticker_position_mode() -> String {
    "auto".into()
}
fn default_sticker_position() -> String {
    "50% 50%".into()
}

impl Default for StickerConfig {
    fn default() -> Self {
        Self {
            position_mode: default_sticker_position_mode(),
            position: default_sticker_position(),
            extra: HashMap::new(),
        }
    }
}

/// 背景模式规范化：`blur`（**缺省**）/ `black` / `color`。
/// 写法照抄 [`crate::cover::cover_bg_mode`]：未知值一律回落，绝不原样吃用户字符串。
///
/// ⚠️ 2026-10-02：「自动」按钮删掉了 —— 老值 `auto`（以及任何认不出的值）**一律当 `blur`**，
/// 与 `resolve_stage` 里的等价关系一致（需要背景时 auto 就是当前帧模糊）⇒ 老工程零迁移。
pub fn stage_bg_mode(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "black" => "black",
        "color" => "color",
        _ => "blur",
    }
}

/// 主画面位置规范化。**方向不跨用**：横画布只认 left/center/right，竖画布只认 top/center/bottom；
/// 其余（含 `auto` 与写错方向的值）→ `auto`。
pub fn stage_position(raw: &str, portrait: bool) -> &'static str {
    match (raw.trim().to_ascii_lowercase().as_str(), portrait) {
        ("left", false) => "left",
        ("right", false) => "right",
        ("top", true) => "top",
        ("bottom", true) => "bottom",
        ("center", _) => "center",
        _ => "auto",
    }
}

/// 老 `[stage] position` 枚举 → **百分比**（只给没有 `position_pct` 的老工程读）。
///
/// - `left` / `top` → 0（贴左 / 贴上）；`right` / `bottom` → 100（贴右 / 贴下）；
/// - `center` → 50；
/// - **`auto` → 50 居中**（2026-10-02 用户口径：默认与老工程的 `auto` 一律掉档为居中 ——
///   改造前竖素材的 auto 是「靠左」，现在统一居中）；
/// - 方向写错的（横画布写 `top` 等）→ 50（[`stage_position`] 已经把它们净化成 `auto`）。
pub fn stage_position_pct(raw: &str, portrait: bool) -> f64 {
    match stage_position(raw, portrait) {
        "left" | "top" => 0.0,
        "right" | "bottom" => 100.0,
        _ => 50.0,
    }
}

/// `[stage] position_pct` 规范化：NaN / 无穷 → 50 居中，其余夹到 `[0, 100]`。
///
/// **不吸附到五档**：五档是 UI 的交互（[`crate::project`] 这层只保证数值合法），
/// 数据模型留连续是给以后加档位用的（例：0/10/…/90/100）。
pub fn normalize_stage_pct(raw: f64) -> f64 {
    if !raw.is_finite() {
        return 50.0;
    }
    raw.clamp(0.0, 100.0)
}

/// 贴纸落点模式规范化：只认 `manual`，其余一律 `auto`。
pub fn sticker_position_mode(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "manual" => "manual",
        _ => "auto",
    }
}

/// 主画面摆放的解析结果 —— **唯一推导点**：模板读 `_canvas.stage`、WebUI 读 `canvas.js`，都不再自己算。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageFit {
    /// 缩放轴：`x` = 按高度铺满、左右留边；`y` = 按宽度铺满、上下留边；`fill` = 直接铺满画布。
    pub axis: String,
    /// 内容层（`#source-video` 与 `#video-canvas`）的 `object-fit`。
    pub object_fit: String,
    /// 主画面位置：`left|center|right`（axis=x）或 `top|center|bottom`（axis=y）。恒为具体值。
    ///
    /// ⚠️ **过渡期兼容输出**：五档里的 25/75 这个枚举表达不了（会被压成 `center`），
    /// **消费侧一律改用 [`StageFit::position_pct`]**；这一项等三处消费侧全改完就删。
    pub position: String,
    /// **主画面位置（0–100，权威）**：0 = 贴左 / 贴上，50 = 居中，100 = 贴右 / 贴下。
    /// 模板 `stageDrawRect`、`canvas.js`/`engine.js`、转译器都按它算偏移。
    pub position_pct: f64,
    /// 背景：`none` | `black` | `color` | `blur`。只有**横画布铺满**才是 `none`；
    /// 竖画布铺满也给底色（特效会把主画面挪出画布，没有底色就露透明）。
    pub bg: String,
    pub bg_color: String,
    /// 模糊半径（px，已按 `blur_scale` 换算）。
    pub blur_px: u32,
    /// 模糊层放大倍数（把模糊溢出的边缘推出画面）。
    pub blur_img_scale: f64,
    pub scale: f64,
    /// 主画面额外留白（px，已按 `k` 缩放）。
    pub padding: f64,
}

// ===== 时间轴 =====

// Segment 仅话题分组, 运镜由 Camera event 控制

#[allow(dead_code)] fn default_zoom() -> String { "中".into() }
#[allow(dead_code)] fn default_origin() -> String { "50% 50%".into() }



#[allow(dead_code)] fn is_zero(v: &f64) -> bool { *v == 0.0 }

// ===== 默认值 =====

fn default_output() -> String { "vtuber-clip.mp4".into() }
fn default_metadata() -> String { "clip.json".into() }


impl ProjectFile {
    /// 输出画布尺寸（唯一推导点）。
    pub fn canvas_size(&self) -> (u32, u32) {
        self.canvas.size()
    }


    /// 解析主画面摆放。`src` = 源视频尺寸（ffprobe；拿不到给 `None`）。
    ///
    /// # 2026-10-02 起的语义（「画面构成」改版）
    ///
    /// - **轴只看比例**：源与画布同比 → `fill`（铺满）；源更高 → `x`（左右留边）；源更扁 → `y`（上下留边）。
    ///   ⚠️ **`auto` 特例已删**：改造前 `auto` 对「不匹配但非竖素材」的源（4:3 / 2.35:1）是**裁切铺满**，
    ///   现在是**按比例适配 + 留边 + 背景**（用户 2026-10-02 拍板「适配留边」；想要裁切就把缩放调到所需比值）。
    /// - **位置 = `position_pct`（0–100，权威）**：新字段优先；老工程按 [`stage_position_pct`] 从老枚举换算
    ///   （`auto` → 50 居中、`left`/`top` → 0、`right`/`bottom` → 100；方向不跨用）。
    /// - **背景**：只有**横画布铺满**是 `none`；竖画布铺满仍按 `bg_mode` 给底色（见下）。
    ///
    /// ⚠️ 铺满**不等于**不要背景层：竖画布（1080×1920）下竖素材正好铺满画布，而特效
    /// （`shake` / `blur` / 运镜）会把主画面挪出画布，让出来的那条边在出片里
    /// 是**透明**的（JPEG 里就是纯黑）—— 所以**竖画布**铺满时仍给一层底色。
    /// 横画布铺满保持 `none`，存量横画布项目逐帧零回归。
    pub fn resolve_stage(&self, src: Option<(u32, u32)>) -> StageFit {
        let (cw, ch) = self.canvas.size();
        let portrait = self.canvas.is_portrait();
        let k = cw as f64 / CANVAS_LANDSCAPE_WH.0 as f64;

        // 位置：新字段优先；缺它（老工程）= 按老枚举换算（`auto` → 50 居中）
        let pct = self
            .stage
            .position_pct
            .map(normalize_stage_pct)
            .unwrap_or_else(|| stage_position_pct(&self.stage.position, portrait));

        // 比例比较用交叉相乘，避免浮点除法
        let aspect = src
            .filter(|(w, h)| *w > 0 && *h > 0)
            .map(|(sw, sh)| {
                ((sh as u128) * (cw as u128)).cmp(&((sw as u128) * (ch as u128)))
            });

        // 轴**只看比例**（不再有 auto 特例）
        let axis = match aspect {
            Some(std::cmp::Ordering::Greater) => "x", // 源比画布"高" → 左右留边
            Some(std::cmp::Ordering::Less) => "y",    // 源比画布"扁" → 上下留边
            _ => "fill",                              // 同比（或拿不到源尺寸）→ 铺满
        };

        // 位置枚举只作**过渡期兼容输出**（五档里的 25/75 它表达不了 ⇒ 只有两端精确，其余给 center）
        let position = if pct <= 0.0 {
            if portrait { "top" } else { "left" }
        } else if pct >= 100.0 {
            if portrait { "bottom" } else { "right" }
        } else {
            "center"
        };

        // 背景：**横画布**铺满才不需要背景层（改造前就是不匹配才补边，存量零回归）；
        // 竖画布铺满仍按 `bg_mode` 给底色 —— 否则特效（shake / split / 运镜）把主画面挪开时
        // 让出来的边是透明的，出片 JPEG 里就是纯黑。
        // 补边时 auto = 改造前的竖屏行为（当前帧模糊）。
        let bg = if axis == "fill" && !portrait {
            "none"
        } else {
            match stage_bg_mode(&self.stage.bg_mode) {
                "black" => "black",
                "color" => "color",
                _ => "blur",
            }
        };

        let (blur_px, blur_img_scale) = crate::cover::cover_blur_css(self.stage.blur_scale);

        StageFit {
            axis: axis.into(),
            object_fit: if axis == "fill" { "cover" } else { "contain" }.into(),
            position: position.into(),
            position_pct: pct,
            bg: bg.into(),
            bg_color: crate::cover::sanitize_hex_color(&self.stage.bg_color, "#000000"),
            blur_px,
            blur_img_scale,
            scale: if self.stage.scale.is_finite() && self.stage.scale > 0.0 {
                self.stage.scale.clamp(0.1, 4.0)
            } else {
                1.0
            },
            padding: self.stage.padding as f64 * k,
        }
    }

    /// 注入给模板与 WebUI 的画布契约（`_canvas`）—— 尺寸 / 摆放 / 字幕安全宽 / 贴纸落点的**唯一来源**。
    ///
    /// 模板 `index.html` 与前端 `webui/src/polish/canvas.js` 都只读它，杜绝第二套常量。
    pub fn canvas_contract(&self, src: Option<(u32, u32)>) -> serde_json::Value {
        let (cw, ch) = self.canvas.size();
        let (sw, sh) = src.unwrap_or((0, 0));
        serde_json::json!({
            "width": cw,
            "height": ch,
            // k = 宽度基准（横画布 1.0），kh = 高度基准（竖画布 1.778）—— 9:16 下两者不等
            "k": cw as f64 / CANVAS_LANDSCAPE_WH.0 as f64,
            "kh": ch as f64 / CANVAS_LANDSCAPE_WH.1 as f64,
            "orientation": if self.canvas.is_portrait() { "portrait" } else { "landscape" },
            "src": { "width": sw, "height": sh },
            "stage": self.resolve_stage(src),
            "caps": {
                "safe_width_ratio": CAPTION_SAFE_WIDTH_RATIO,
                "margin_x": cw as f64 * (1.0 - CAPTION_SAFE_WIDTH_RATIO) / 2.0,
                "max_width": cw as f64 * CAPTION_SAFE_WIDTH_RATIO,
                "max_lines": CAPTION_MAX_LINES,
                "min_font_ratio": CAPTION_MIN_FONT_RATIO,
            },
            "sticker": {
                "safe_margin": STICKER_SAFE_MARGIN,
                "position_mode": sticker_position_mode(&self.sticker.position_mode),
                "position": self.sticker.position.trim(),
            },
        })
    }

    /// 序列化为 TOML 并注入注释
    pub fn to_toml_commented(&self) -> anyhow::Result<String> {
        let raw = toml::to_string_pretty(self)?;
        let lines: Vec<String> = raw.lines().map(|s| s.to_string()).collect();

        let mut result = String::new();
        let mut in_events = false;
        let mut in_cover = false;
        let mut seg_count = 0;

        for line in &lines {
            let trimmed = line.trim();

            // 在 [project] 后加注释
            if trimmed == "[project]" {
                result.push_str("# ============================================================\n");
                result.push_str("# Vtedit 项目配置\n");
                result.push_str("# ============================================================\n");
                result.push_str(line); result.push('\n');
                result.push_str("# output: 输出文件名\n");
                continue;
            }

            // [source] 后加注释
            if trimmed == "[source]" {
                result.push_str("\n# ----- 素材路径 -----\n");
                result.push_str(line); result.push('\n');
                result.push_str("# danmaku: 可选, 没有就留空\n");
                result.push_str("# metadata: clip.json, 提供 duration/title/event_type\n");
                continue;
            }

            // [cover] 后加注释
            if trimmed == "[cover]" {
                result.push_str("\n# ===== 封面 =====\n");
                result.push_str(line); result.push('\n');
                in_cover = true;
                continue;
            }
            if in_cover {
                if trimmed.starts_with('[') { in_cover = false; }
                else if !trimmed.starts_with('#') && !trimmed.is_empty() {
                    let field = trimmed.split('=').next().unwrap_or("").trim();
                    let comment = match field {
                        "image" => "# 背景图(相对项目目录),空=自动截帧",
                        "frame_pos" => "# 截图时间位置 0~1",
                        "gradient_dir" => "# 渐变方向: left/right/top/bottom/radial/none",
                        "vignette" => "# 四角暗度 0~1",
                        "letter_spacing" => "# 字间距",
                        "line_height" => "# 行高",
                        "title_lines" => "# 手动断行,修改后不会被覆盖",
                        "hl_color" => "# 高亮词颜色",
                        "text_x" => "# 文字水平中心点",
                        "text_y" => "# 文字垂直中心点",
                        "text_width" => "# 文字区域宽度",
                        "text_align" => "# 文字对齐: left/center/right",
                        "shadow_dir" => "# 阴影方向: none/left/right/top/bottom/center",
                        "shadow_intensity" => "# 阴影强度 0~1",
                        "effects" => "# 可叠加封面效果",
                        "bg_x" => "# 帧层(当前帧)水平偏移(px)：拖动预览即可改",
                        "bg_y" => "# 帧层(当前帧)垂直偏移(px)",
                        "bg_zoom" => "# 帧层(当前帧)缩放倍率(在 liver 聚焦基准之上再乘)",
                        "bg_mode" => "# 背景模式(在帧之下): color(纯色,默认黑)/blur(当前帧模糊平铺)",
                        "bg_color" => "# 纯色背景颜色, 默认 #000000 (bg_mode=color；blur 漏边时的兜底色)",
                        "blur_scale" => "# 模糊倍率 (bg_mode=blur，1.0 = 基准 28px)",
                        "layers" => "# 图层栈(从下到上): id = bg/frame/filter/title/fx:<特效id>；visible=false 只隐藏不删除",
                        _ => "",
                    };
                    if !comment.is_empty() {
                        // 避免重复注释 (多次调用 to_toml_commented 时)
                        let prev = result.trim_end().lines().last().unwrap_or("");
                        if prev != comment { result.push_str(comment); result.push('\n'); }
                    }
                }
            }
            if in_cover {
                if trimmed.starts_with('[') { in_cover = false; }
                else if trimmed.starts_with("image") { result.push_str("# 背景图(相对项目目录),空=自动截帧\n"); }
                else if trimmed.starts_with("frame_pos") { result.push_str("# 截图时间位置 0~1\n"); }
                else if trimmed.starts_with("gradient_dir") { result.push_str("# 渐变方向: left/right/top/bottom/radial/none\n"); }
                else if trimmed.starts_with("vignette") { result.push_str("# 四角暗度 0~1\n"); }
                else if trimmed.starts_with("letter_spacing") { result.push_str("# 字间距\n"); }
                else if trimmed.starts_with("line_height") { result.push_str("# 行高\n"); }
                else if trimmed.starts_with("title_lines") { result.push_str("# 手动断行,修改后不会被覆盖\n"); }
                else if trimmed.starts_with("hl_color") { result.push_str("# 高亮词颜色\n"); }
                else if trimmed.starts_with("text_x") { result.push_str("# 文字水平中心点\n"); }
                else if trimmed.starts_with("text_y") { result.push_str("# 文字垂直中心点\n"); }
                else if trimmed.starts_with("text_width") { result.push_str("# 文字区域宽度\n"); }
            }

            // [canvas] / [stage] / [sticker]：段头 + 字段速查（本次新增的三张表）
            if trimmed == "[canvas]" || trimmed == "[stage]" || trimmed == "[sticker]" {
                match trimmed {
                    "[canvas]" => {
                        result.push_str("\n# ===== 输出画布 =====\n");
                        result.push_str("# orientation: landscape(1920x1080, 默认) / portrait(1080x1920, 9:16)\n");
                        result.push_str("# 宽高由方向推导，不单独配置\n");
                    }
                    "[stage]" => {
                        result.push_str("\n# ===== 画面构成（源比例 ≠ 画布比例时才有可见效果） =====\n");
                        result.push_str("# 源与画布同比例时位置/缩放/背景都没有可见效果（界面会整组隐藏）\n");
                        result.push_str("# bg_mode: blur(默认, 当前帧模糊铺满) / black / color；老值 auto 等同 blur\n");
                        result.push_str("# bg_color: bg_mode=color 的颜色(#rrggbb)\n");
                        result.push_str("# blur_scale: 模糊倍率, 1.0 = 基准 28px\n");
                        result.push_str("# position_pct: 主画面位置 0~100（0 贴左/贴上, 50 居中(默认), 100 贴右/贴下）\n");
                        result.push_str("#   界面是五档吸附 0/25/50/75/100；老字段 position 只读兼容(auto = 居中)\n");
                        result.push_str("# scale: 主画面缩放, 1.0 = 按当前轴适配到位（<1 再缩, >1 放大裁切）\n");
                        result.push_str("# padding: 老字段(px, 横画布基准)，界面已不再提供；留着仅为老工程照旧生效\n");
                        result.push_str("# portrait_effects / portrait_camera / portrait_sfx: 竖屏画布是否启用\n");
                        result.push_str("#   特效/运镜/音效，缺字段 = false = 关（整类事件不渲染，事件本身不动）\n");
                    }
                    _ => {
                        result.push_str("\n# ===== 贴纸落点 =====\n");
                        result.push_str("# position_mode: auto(默认, 安全区算法自动避让) / manual(固定到 position)\n");
                        result.push_str("# position: manual 的固定落点, 相对画布的百分比 \"x% y%\"\n");
                    }
                }
                result.push_str(line); result.push('\n');
                continue;
            }

            // [filter] 后加注释
            if trimmed == "[filter]" {
                result.push_str("\n# ===== 关键词过滤 =====\n");
                result.push_str(line); result.push('\n');
                continue;
            }

            // [style] 后加注释
            if trimmed == "[style]" {
                result.push_str("\n# ===== 视觉样式 =====\n");
                result.push_str(line); result.push('\n');
                continue;
            }

            // [style.camera] 后加注释
            if trimmed == "[style.camera]" {
                result.push_str("\n# ===== 运镜缩放 =====\n");
                result.push_str("# zoom_mid: 中景缩放 (camera zoom=中), zoom_close: 近景特写缩放\n");
                result.push_str(line); result.push('\n');
                continue;
            }

            // [style.danmaku] 后加注释（offset_secs 由 WebUI「弹幕时间偏移」滑块写入）
            if trimmed == "[style.danmaku]" {
                result.push_str("\n# ===== 弹幕样式 =====\n");
                result.push_str("# offset_secs: 弹幕时间偏移（秒，负 = 提前出现，默认 -3）\n");
                result.push_str(line); result.push('\n');
                continue;
            }

            // [layout.*] 前输出布局总说明（TOML 无 [layout] 头，挂在第一个子表前）
            if trimmed == "[layout.liver]" {
                result.push_str("\n# ===== 布局（区域） =====\n");
                result.push_str("# 两种写法，可能同时存在，自由选框优先：\n");
                result.push_str("# 1) grid: 九宫格名，多个格子各自成为一个矩形（不合并）\n");
                result.push_str("#    可用格子: top-left / top / top-right / mid-left / center / mid-right / bot-left / bottom / bot-right\n");
                result.push_str("# 2) 自由选框: [layout.<区域>.top_left] + [layout.<区域>.bottom_right] (x/y 百分比)\n");
                result.push_str("#    WebUI「框选布局」写入, 比九宫格更精确; 清除选框后回退到 grid\n");
                result.push_str("# liver: 主播区域, 运镜聚焦其中心 (自由选框 > origin_grid > origin > 首个格子)\n");
                result.push_str(line); result.push('\n');
                continue;
            }
            if trimmed == "[layout.danmaku_area]" {
                result.push_str("# danmaku_area: 弹幕显示区域 (供运镜参考)\n");
                result.push_str(line); result.push('\n');
                continue;
            }
            if trimmed == "[layout.sticker_area]" {
                result.push_str("# sticker_area: 贴纸落点**偏好**（已废弃：process 每次按 人物/字幕/弹幕 动态推导安全区）\n");
                result.push_str("#   自由选框(top_left/bottom_right) 仍作偏好：里面有安全位就优先用，没有就自动放弃;\n");
                result.push_str("#   grid 不再参与落点。要固定某张贴纸的位置请直接手改该事件的 x/y。\n");
                result.push_str(line); result.push('\n');
                continue;
            }

            // [sfx] 后加注释
            if trimmed == "[sfx]" {
                result.push_str("\n# ===== 音效 =====\n");
                result.push_str("# blacklist: 屏蔽列表, 精确 id 或 tag:xxx 整族 (与全局配置合并)\n");
                result.push_str(line); result.push('\n');
                continue;
            }

            // [[segments]] 标记
            if trimmed.starts_with("[[segments]]") {
                if seg_count > 0 { result.push('\n'); }
                result.push_str("# ============================================================\n");
                result.push_str(&format!("# Segment {}\n", seg_count + 1));
                result.push_str("# ============================================================\n");
                result.push_str(line); result.push('\n');
                result.push_str("# fx: low=安静 / med=常规 / high=爆点\n");
                result.push_str("# note: 话题备注 (仅供阅读, LLM 生成)\n");
                seg_count += 1;
                continue;
            }

            // [[segments.events]] 标记
            if trimmed.starts_with("[[segments.events]]") {
                result.push('\n');
                result.push_str(line); result.push('\n');
                in_events = true;
                continue;
            }

            // 特定事件类型的注释
            if trimmed.starts_with("type = ") && in_events {
                let ev_type = trimmed.trim_matches('"').trim_start_matches("type = ").trim();
                match ev_type {
                    "sticker" => result.push_str("# sticker: 贴图 (来源由 sticker_source 配置决定)\n"),
                    "camera" => result.push_str("# camera: 运镜事件 (zoom=\"中\"/\"近\")\n"),
                    "sfx" => result.push_str("# sfx: 纯音效, 无视觉\n"),
                    _ => {}
                }
                result.push_str(line); result.push('\n');
                in_events = false;
                continue;
            }

            // events = [] 补注释 (在 segment 内)
            if trimmed == "events = []" {
                result.push_str("# 在此 segment 中添加事件:\n");
                result.push_str("# [[segments.events]]\n");
                result.push_str("# [[segments.events]]\n# type = \"camera\"\n# time = 0\n# end = 10\n# zoom = \"中\"\n");
                result.push_str(line); result.push('\n');
                continue;
            }


            // 空表补注释: 可选布局（九宫格）
            if trimmed == "[layout.reaction_zone]" || trimmed == "[layout.game_area]" {
                result.push_str("# 可选: 不配置则使用默认值\n");
                result.push_str(line); result.push('\n');
                if trimmed == "[layout.reaction_zone]" {
                    result.push_str("# grid = [\"center\"]\n");
                }
                continue;
            }

            result.push_str(line); result.push('\n');
        }

        // 清理重复注释行
        let raw = result.clone();
        result.clear();
        for (i, line) in raw.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                let prev = if i > 0 { raw.lines().nth(i-1).unwrap_or("").trim() } else { "" };
                if prev == trimmed { continue; }
            }
            result.push_str(line); result.push('\n');
        }

        Ok(result)
    }

    // ===== 示例 =====
    pub fn example() -> Self {
        ProjectFile {
            project: ProjectMeta {
                name: "My VTuber Clip".into(),
                output: "vtuber-clip.mp4".into(),
                created: default_created(),
                version: 1,
            },
            source: SourceRef {
                video: "clip.mp4".into(),
                subtitles: "clip.srt".into(),
                danmaku: "clip.dm.srt".into(),
                metadata: "clip.json".into(),
                extra: HashMap::new(),
            },
            duration: None,
            layout: Layout::default(),
            canvas: CanvasConfig::default(),
            stage: StageConfig::default(),
            sticker: StickerConfig::default(),
            style: StyleRoot::default(),
            sticker_source: vec!["emoji".into(), "ysyk".into()],
            filter: FilterConfig {
                danmaku: vec!["OBS".into(), "游戏".into(), "直播".into()],
                highlight: vec!["OBS".into(), "游戏".into()],
            },
            sfx: SfxConfig::default(),
            cover: CoverConfig::default(),
            segments: vec![
                Segment {
                    start: 0.0, end: 10.0, fx: "low".into(), note: "开场".into(),
                    events: vec![], extra: HashMap::new(),
                },
            ],
            mute_ranges: Vec::new(),
            silence_ranges: Vec::new(),
            processed: false,
            intro_fade_seeded: false,
            extra: HashMap::new(),
        }
    }
}



/// 把运镜的结束时间夹到「从它起点到所属分段结束」为止。
///
/// 分段被截断到视频长度之后，事件自带的 `end` 还停在截断前的值。
/// 对瞬时特效无所谓（窗口是固定常数、由 `normalize_event_ends` 接管），但运镜的
/// `end - time` 决定过渡窗口、位移事件还靠它决定「移到那里」的时机，不夹就会移到段外。
fn clamp_camera_end(seg: &mut Segment) {
    for ev in seg.events.iter_mut() {
        let room = (seg.end - ev.get_time()).max(0.0);
        if let Event::Camera { time, end, .. } = ev {
            if let Some(e) = *end {
                *end = Some(*time + (e - *time).min(room).max(0.1));
            }
        }
    }
}

// 这里曾经有个 `split_segments_by_mute_ranges`（按屏蔽区间切开分段、丢掉屏蔽区内的事件）。
// ⚠️ **已删除**（用户要求：屏蔽只做标记，不删事件）：它以前由 `cmd_process` / `cmd_llm` 调用并落盘，
// 于是"屏蔽一段"等于把那一段里的事件**永久删掉**（`cmd_process` 还会被预览/出片隐式触发），
// 撤销屏蔽也回不来。现在 `mute_ranges` 只由消费侧生效：出片合流后 `Ffmpeg::cut_ranges` 剪掉、
// 预览跳过、字幕在 `cmd_process` 里按 `MuteRange::overlaps` 过滤。**别再把它加回来。**

/// 将 segments/events 截断到指定时长 (防止 LLM 生成的 segment 超出视频长度)
pub fn clamp_segments_to_duration(segments: &mut Vec<Segment>, duration: f64) {
    if duration <= 0.0 { return; }
    // 移除起始时间 >= duration 的 segment
    segments.retain(|s| s.start < duration);
    for seg in segments.iter_mut() {
        seg.end = seg.end.min(duration);
        // 移除 segment 内起始时间 >= duration 的事件
        seg.events.retain(|e| e.get_time() < duration);
        clamp_camera_end(seg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_rect_all_nine() {
        let names = [
            "top-left", "top", "top-right",
            "mid-left", "center", "mid-right",
            "bot-left", "bottom", "bot-right",
        ];
        for n in names {
            assert!(grid_rect(n).is_some(), "missing {n}");
        }
        assert!(grid_rect("invalid").is_none());
    }

    #[test]
    fn region_rects_single_grid() {
        let region = LayoutRegion {
            grid: vec!["center".into()],
            origin_grid: None,
            origin: None,
            top_left: None,
            bottom_right: None,
            extra: HashMap::new(),
        };
        let rects = region_rects(&region);
        assert_eq!(rects.len(), 1);
        let (tl, br) = &rects[0];
        assert_eq!(tl.x, "33.33%");
        assert_eq!(tl.y, "33.33%");
        assert_eq!(br.x, "66.67%");
        assert_eq!(br.y, "66.67%");
    }

    #[test]
    fn region_rects_keeps_separate_rects() {
        let region = LayoutRegion {
            grid: vec!["mid-left".into(), "mid-right".into()],
            origin_grid: None,
            origin: None,
            top_left: None,
            bottom_right: None,
            extra: HashMap::new(),
        };
        let rects = region_rects(&region);
        assert_eq!(rects.len(), 2);
        // mid-left
        assert_eq!(rects[0].0.x, "0.00%");
        assert_eq!(rects[0].1.x, "33.33%");
        // mid-right
        assert_eq!(rects[1].0.x, "66.67%");
        assert_eq!(rects[1].1.x, "100.00%");
        // 不包含 center：两个矩形之间有空隙
        assert_ne!(rects[0].1.x, rects[1].0.x);
    }

    #[test]
    fn region_rects_empty_returns_empty() {
        let region = LayoutRegion {
            grid: vec![],
            origin_grid: None,
            origin: None,
            top_left: None,
            bottom_right: None,
            extra: HashMap::new(),
        };
        assert!(region_rects(&region).is_empty());
    }

    #[test]
    fn region_rect_legacy_merges_bounding_box() {
        let region = LayoutRegion {
            grid: vec!["top-left".into(), "center".into()],
            origin_grid: None,
            origin: None,
            top_left: None,
            bottom_right: None,
            extra: HashMap::new(),
        };
        #[allow(deprecated)]
        let (tl, br) = region_rect(&region).unwrap();
        assert_eq!(tl.x, "0.00%");
        assert_eq!(tl.y, "0.00%");
        assert_eq!(br.x, "66.67%");
        assert_eq!(br.y, "66.67%");
    }

    #[test]
    fn region_origin_prefers_grid() {
        let region = LayoutRegion {
            grid: vec!["mid-right".into()],
            origin_grid: Some("mid-right".into()),
            origin: Some("10% 20%".into()),
            top_left: None,
            bottom_right: None,
            extra: HashMap::new(),
        };
        assert_eq!(region_origin(&region).unwrap(), "83.34% 50.00%");
    }

    #[test]
    fn region_origin_falls_back_to_legacy() {
        let region = LayoutRegion {
            grid: vec!["mid-right".into()],
            origin_grid: None,
            origin: Some("10% 20%".into()),
            top_left: None,
            bottom_right: None,
            extra: HashMap::new(),
        };
        assert_eq!(region_origin(&region).unwrap(), "10% 20%");
    }

    #[test]
    fn region_origin_auto_from_first_grid() {
        let region = LayoutRegion {
            grid: vec!["center".into()],
            origin_grid: None,
            origin: None,
            top_left: None,
            bottom_right: None,
            extra: HashMap::new(),
        };
        assert_eq!(region_origin(&region).unwrap(), "50.00% 50.00%");
    }

    fn pt(x: &str, y: &str) -> Point {
        Point { x: x.into(), y: y.into() }
    }

    #[test]
    fn free_rect_normalizes_and_clamps() {
        // 左上/右下颠倒 + 越界 → 顺序化并裁剪到 0~100
        let region = LayoutRegion {
            grid: vec![],
            origin_grid: None,
            origin: None,
            top_left: Some(pt("120%", "80%")),
            bottom_right: Some(pt("-10%", "20%")),
            extra: HashMap::new(),
        };
        let (tl, br) = free_rect(&region).unwrap();
        assert_eq!((tl.x.as_str(), tl.y.as_str()), ("0.00%", "20.00%"));
        assert_eq!((br.x.as_str(), br.y.as_str()), ("100.00%", "80.00%"));
        assert!(!region.is_empty());
    }

    #[test]
    fn free_rect_rejects_degenerate() {
        let region = LayoutRegion {
            grid: vec![],
            origin_grid: None,
            origin: None,
            top_left: Some(pt("10%", "10%")),
            bottom_right: Some(pt("10.2%", "60%")),
            extra: HashMap::new(),
        };
        assert!(free_rect(&region).is_none());
    }

    #[test]
    fn region_rects_prefers_free_rect_over_grid() {
        let region = LayoutRegion {
            grid: vec!["mid-left".into(), "bot-left".into()],
            origin_grid: None,
            origin: None,
            top_left: Some(pt("40%", "10%")),
            bottom_right: Some(pt("90%", "70%")),
            extra: HashMap::new(),
        };
        let rects = region_rects(&region);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].0.x, "40.00%");
        assert_eq!(rects[0].0.y, "10.00%");
        assert_eq!(rects[0].1.x, "90.00%");
        assert_eq!(rects[0].1.y, "70.00%");
    }

    #[test]
    fn region_origin_uses_free_rect_center() {
        let region = LayoutRegion {
            grid: vec!["mid-right".into()],
            origin_grid: Some("mid-right".into()),
            origin: Some("10% 20%".into()),
            top_left: Some(pt("50%", "20%")),
            bottom_right: Some(pt("90%", "60%")),
            extra: HashMap::new(),
        };
        assert_eq!(region_origin(&region).unwrap(), "70.00% 40.00%");
    }

    #[test]
    fn free_rect_survives_toml_roundtrip() {
        let mut cfg = ProjectFile::example();
        cfg.layout.liver.top_left = Some(pt("55%", "15%"));
        cfg.layout.liver.bottom_right = Some(pt("95%", "75%"));
        let raw = cfg.to_toml_commented().unwrap();
        assert!(raw.contains("[layout.liver.top_left]"), "缺 top_left 子表:\n{raw}");
        assert!(raw.contains("[layout.liver.bottom_right]"), "缺 bottom_right 子表:\n{raw}");
        let back: ProjectFile = toml::from_str(&raw).expect("框选布局 TOML 应可回读");
        assert_eq!(region_origin(&back.layout.liver).unwrap(), "75.00% 45.00%");
        assert_eq!(region_rects(&back.layout.sticker_area).len(), 2);
    }

    /// 前端「框选布局」的 JSON 形状：GET 拿到的 layout → spread 区域 → 写 top_left/bottom_right → PUT 回来。
    /// 锁住 `Layout` 的 JSON 契约（deny_unknown_fields + flatten extra 下也必须能反序列化）。
    #[test]
    fn layout_free_rect_json_roundtrip_from_frontend() {
        let mut cfg = ProjectFile::example();
        cfg.layout.sticker_area.grid = vec!["mid-left".into(), "bot-left".into()];
        let mut lay = serde_json::to_value(&cfg.layout).unwrap();
        let mut region = lay["sticker_area"].clone();
        region["top_left"] = serde_json::json!({ "x": "10.00%", "y": "40.00%" });
        region["bottom_right"] = serde_json::json!({ "x": "45.00%", "y": "95.00%" });
        lay["sticker_area"] = region;
        // 清除选框：删掉两个角后仍可解析（回到 grid）
        let back: Layout = serde_json::from_value(lay.clone()).expect("前端 layout patch 应可解析");
        let rects = region_rects(&back.sticker_area);
        assert_eq!(rects.len(), 1, "自由选框应取代多格子");
        assert_eq!(rects[0].0.x, "10.00%");
        assert_eq!(rects[0].1.y, "95.00%");
        assert_eq!(back.sticker_area.grid.len(), 2, "grid 保留作为清除后的回退");

        let mut cleared = lay["sticker_area"].clone();
        cleared.as_object_mut().unwrap().remove("top_left");
        cleared.as_object_mut().unwrap().remove("bottom_right");
        lay["sticker_area"] = cleared;
        let back: Layout = serde_json::from_value(lay).expect("清除选框后应可解析");
        assert_eq!(region_rects(&back.sticker_area).len(), 2, "清除后回到九宫格");
    }

    /// 片头淡入的补种标记：默认 false（老工程没有这一行，也照旧解析）、true 能落进 toml 并回读。
    /// 这条标记是"初次 process 用掉补种时机"的唯一判据，落了 false 就不该出现在文件里。
    #[test]
    fn intro_fade_seeded_roundtrip_and_legacy_default() {
        let mut cfg = ProjectFile::example();
        assert!(!cfg.intro_fade_seeded);
        let raw = cfg.to_toml_commented().unwrap();
        assert!(!raw.contains("intro_fade_seeded"), "false 不该写进文件:\n{raw}");

        cfg.intro_fade_seeded = true;
        let raw = cfg.to_toml_commented().unwrap();
        assert!(raw.contains("intro_fade_seeded = true"), "{raw}");
        let back: ProjectFile = toml::from_str(&raw).expect("置位后应可回读");
        assert!(back.intro_fade_seeded);

        // 老工程（没有这一行）→ 默认 false，等于"还没补过片头"，下次 process 照常补
        let legacy: ProjectFile =
            toml::from_str("[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n").unwrap();
        assert!(!legacy.intro_fade_seeded);
    }
}

#[cfg(test)]
mod camera_clamp_tests {
    use super::*;

    fn cam(time: f64, dur: f64) -> Event {
        Event::Camera {
            time,
            end: Some(time + dur),
            zoom: "中".into(),
            origin: "50% 50%".into(),
            anchor: None,
            anchor_to: None,
            to_origin: None, cut: false,
            handheld: false,
            manual: false,
        }
    }

    fn seg(start: f64, end: f64, events: Vec<Event>) -> Segment {
        Segment {
            start,
            end,
            fx: "low".into(),
            note: String::new(),
            events,
            extra: Default::default(),
        }
    }

    /// 截断到视频长度之后同理
    #[test]
    fn duration_clamp_shortens_camera() {
        let mut segs = vec![seg(0.0, 60.0, vec![cam(0.0, 60.0)])];
        clamp_segments_to_duration(&mut segs, 20.0);
        let Event::Camera { time, end, .. } = &segs[0].events[0] else {
            panic!()
        };
        assert!((end.unwrap() - *time - 20.0).abs() < 1e-9, "{end:?}");
    }
}

#[cfg(test)]
mod canvas_stage_tests {
    use super::*;

    fn src_portrait() -> Option<(u32, u32)> {
        Some((1080, 1920))
    }
    fn src_landscape() -> Option<(u32, u32)> {
        Some((1920, 1080))
    }

    /// 画布尺寸只由方向推导
    #[test]
    fn canvas_size_follows_orientation() {
        let mut cfg = ProjectFile::example();
        assert_eq!(cfg.canvas_size(), (1920, 1080), "默认必须是横画布");
        cfg.canvas.orientation = "portrait".into();
        assert_eq!(cfg.canvas_size(), (1080, 1920));
        cfg.canvas.orientation = " PORTRAIT ".into();
        assert_eq!(cfg.canvas_size(), (1080, 1920), "大小写/空白不敏感");
        cfg.canvas.orientation = "garbage".into();
        assert_eq!(cfg.canvas_size(), (1920, 1080), "非法值回落横画布");
    }

    /// **零回归钉子**：横画布下字幕安全宽必须正好是改造前的 1680 / 120
    #[test]
    fn caption_safe_width_matches_legacy_1680() {
        let cfg = ProjectFile::example();
        let c = cfg.canvas_contract(src_landscape());
        assert!((c["caps"]["max_width"].as_f64().unwrap() - 1680.0).abs() < 1e-9, "{c}");
        assert!((c["caps"]["margin_x"].as_f64().unwrap() - 120.0).abs() < 1e-9, "{c}");
        assert!((c["k"].as_f64().unwrap() - 1.0).abs() < 1e-9);
        assert!((c["kh"].as_f64().unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(c["caps"]["max_lines"], 3);

        let mut p = ProjectFile::example();
        p.canvas.orientation = "portrait".into();
        let c2 = p.canvas_contract(src_portrait());
        assert!((c2["caps"]["max_width"].as_f64().unwrap() - 945.0).abs() < 1e-9, "{c2}");
        assert!((c2["k"].as_f64().unwrap() - 0.5625).abs() < 1e-9);
        assert!((c2["kh"].as_f64().unwrap() - 1920.0 / 1080.0).abs() < 1e-9);
    }

    /// 老 `position = "auto"` + 竖素材：轴照比例走 `x`（竖源 vs 横画布 → 左右留边），
    /// **位置掉档为居中**（2026-10-02 用户口径：默认与老工程的 auto 一律居中，不再「靠左」）。
    #[test]
    fn auto_position_centers_and_keeps_letterbox() {
        let cfg = ProjectFile::example();
        let f = cfg.resolve_stage(src_portrait());
        assert_eq!(f.axis, "x");
        assert_eq!(f.object_fit, "contain");
        assert_eq!(f.position, "center", "auto → 50 居中");
        assert!((f.position_pct - 50.0).abs() < 1e-9, "{}", f.position_pct);
        assert_eq!(f.bg, "blur");
        assert_eq!(f.bg_color, "#000000");
        assert_eq!(f.blur_px, 28, "blur_scale=1.0 → 基准 28px");
    }

    /// **适配留边**（2026-10-02 用户拍板）：只有**同比**才铺满；不匹配的源一律按比例适配 + 留边 + 背景。
    /// ⚠️ 这与改造前**不同** —— 老 `auto` 对 4:3 这类「不匹配但非竖素材」的源是**裁切铺满**。
    /// 想要裁切观感就把缩放往上调（`scale ≥ 所需比值`）。
    #[test]
    fn non_matching_source_gets_letterbox() {
        let cfg = ProjectFile::example();
        assert_eq!(cfg.canvas_size(), (1920, 1080), "前提：默认横画布");
        // 同比（含拿不到源尺寸）→ 铺满、无背景层
        for src in [src_landscape(), None] {
            let f = cfg.resolve_stage(src);
            assert_eq!(f.axis, "fill", "{src:?}");
            assert_eq!(f.object_fit, "cover", "{src:?}");
            assert_eq!(f.bg, "none", "{src:?}");
        }
        // 4:3 源比画布「高」→ 左右留边（改造前是裁切）
        let f = cfg.resolve_stage(Some((1440, 1080)));
        assert_eq!(f.axis, "x");
        assert_eq!(f.object_fit, "contain");
        assert_eq!(f.bg, "blur");
        // 21:9 源比画布「扁」→ 上下留边
        let f = cfg.resolve_stage(Some((2560, 1080)));
        assert_eq!(f.axis, "y");
        assert_eq!(f.object_fit, "contain");
        assert_eq!(f.bg, "blur");
    }

    /// **铺满 ≠ 不要背景层**：竖画布铺满时仍按 `bg_mode` 给底色。
    /// 特效（`shake` / `split` / 画中画运镜 / 手持摇摆）会把主画面挪出画布，
    /// 没有底色时让出来的那条边是**透明**的 —— 落进 JPEG 就是纯黑（用户报的"震动露出黑边"）。
    /// 门控是**画布方向**，不是 `bg_mode`：横画布铺满照旧 `none`（存量逐帧零回归）。
    ///
    /// 用例照抄用户实际项目（源 720×1280 / 画布 1080×1920）：同比例 ⇒ 轴 `fill`（铺满），
    /// 而竖画布铺满**仍要给底色**（这条闸与位置、与 `bg_mode` 都无关，只看画布方向）。
    #[test]
    fn portrait_canvas_fill_still_paints_backdrop() {
        let mut cfg = ProjectFile::example();
        cfg.canvas.orientation = "portrait".into();
        cfg.stage.position = "center".into();
        let same_ratio = Some((720, 1280)); // 9:16 == 画布比例

        let f = cfg.resolve_stage(same_ratio);
        assert_eq!(f.axis, "fill", "同比例源 + 显式位置 → 铺满");
        assert_eq!(f.object_fit, "cover");
        assert_eq!(f.bg, "blur", "auto → 竖画布铺满也要模糊底色");

        cfg.stage.bg_mode = "black".into();
        assert_eq!(cfg.resolve_stage(same_ratio).bg, "black");
        cfg.stage.bg_mode = "color".into();
        cfg.stage.bg_color = "#112233".into();
        let f = cfg.resolve_stage(same_ratio);
        assert_eq!(f.bg, "color");
        assert_eq!(f.bg_color, "#112233");
        cfg.stage.bg_mode = "garbage".into(); // 未知值 → auto → blur
        assert_eq!(cfg.resolve_stage(same_ratio).bg, "blur");

        // auto 位置 + **同比**源（720×1280 = 9:16 = 画布比例）⇒ 铺满：
        // 轴自 2026-10-02 起**只看比例**，与位置是不是 auto、源是不是竖的无关。
        let mut auto_cfg = ProjectFile::example();
        auto_cfg.canvas.orientation = "portrait".into();
        let f = auto_cfg.resolve_stage(same_ratio);
        assert_eq!(f.axis, "fill", "同比 ⇒ 铺满");
        assert_eq!(f.bg, "blur");

        // 反证：同一份 `bg_mode` 换回横画布同比源 → 又回到 none（门控只跟画布方向有关）
        cfg.stage.bg_mode = "blur".into();
        cfg.canvas.orientation = "landscape".into();
        let f = cfg.resolve_stage(Some((1920, 1080)));
        assert_eq!(f.axis, "fill");
        assert_eq!(f.bg, "none", "横画布铺满：即使用户显式选了 blur 也保持 none");
    }

    /// 显式位置：按比例选留边轴。4:3 源比 16:9 画布"高" → 左右留边（不是上下）
    #[test]
    fn explicit_position_derives_axis_from_aspect() {
        let mut cfg = ProjectFile::example();
        cfg.stage.position = "right".into();
        let f = cfg.resolve_stage(Some((1440, 1080)));
        assert_eq!(f.axis, "x");
        assert_eq!(f.position, "right");
        assert_eq!(f.bg, "blur", "auto 背景在需要补边时 = 当前帧模糊");
    }

    /// 竖画布 × 横素材：对称支持 top/center/bottom
    #[test]
    fn portrait_canvas_with_landscape_source() {
        let mut cfg = ProjectFile::example();
        cfg.canvas.orientation = "portrait".into();
        cfg.stage.position = "top".into();
        let f = cfg.resolve_stage(src_landscape());
        assert_eq!(f.axis, "y");
        assert_eq!(f.object_fit, "contain");
        assert_eq!(f.position, "top");
        assert_eq!(f.bg, "blur");
    }

    /// 方向不跨用：横画布写 top / 竖画布写 left 都被净化成 auto
    #[test]
    fn cross_direction_position_sanitized() {
        assert_eq!(stage_position("top", false), "auto");
        assert_eq!(stage_position("bottom", false), "auto");
        assert_eq!(stage_position("left", true), "auto");
        assert_eq!(stage_position("right", true), "auto");
        assert_eq!(stage_position("center", false), "center");
        assert_eq!(stage_position("center", true), "center");
        assert_eq!(stage_position(" left ", false), "left");
        assert_eq!(stage_position("RIGHT", false), "right");
        assert_eq!(stage_position("TOP", true), "top");
    }

    /// 跨方向的显式值在竖画布上退化成 center，而不是把 top 当 left 用
    #[test]
    fn cross_direction_position_falls_back_to_center() {
        let mut cfg = ProjectFile::example();
        cfg.canvas.orientation = "portrait".into();
        cfg.stage.position = "left".into();
        assert_eq!(cfg.resolve_stage(src_landscape()).position, "center");
    }

    /// 背景模式与颜色的净化（颜色直接进模板 CSS，绝不能原样吃字符串）
    #[test]
    fn bg_mode_and_color_sanitized() {
        // 2026-10-02：「自动」并入「模糊」—— 老值 `auto` 与任何认不出的值都当 `blur`
        // （两者在 `resolve_stage` 里本来就等价：需要背景时都是当前帧模糊）⇒ 老工程零迁移。
        assert_eq!(stage_bg_mode("frame"), "blur", "老配置里的 frame 回落 blur");
        assert_eq!(stage_bg_mode("auto"), "blur", "老 auto 与 blur 等价");
        assert_eq!(stage_bg_mode(" BLUR "), "blur");
        assert_eq!(stage_bg_mode(""), "blur");

        let mut cfg = ProjectFile::example();
        cfg.canvas.orientation = "portrait".into();
        cfg.stage.position = "center".into();
        cfg.stage.bg_mode = "color".into();
        cfg.stage.bg_color = "javascript:alert(1)".into();
        let f = cfg.resolve_stage(src_landscape());
        assert_eq!(f.bg, "color");
        assert_eq!(f.bg_color, "#000000", "非法颜色必须回落");
        cfg.stage.bg_color = "#ABC".into();
        assert_eq!(cfg.resolve_stage(src_landscape()).bg_color, "#abc");
    }

    /// scale 夹取与 padding 随 k 缩放
    #[test]
    fn scale_and_padding_clamped() {
        let mut cfg = ProjectFile::example();
        cfg.stage.position = "left".into();
        cfg.stage.scale = -2.0;
        assert_eq!(cfg.resolve_stage(src_portrait()).scale, 1.0, "非法 scale 回落 1.0");
        cfg.stage.scale = 99.0;
        assert_eq!(cfg.resolve_stage(src_portrait()).scale, 4.0);
        cfg.stage.scale = 1.0;

        cfg.stage.padding = 40;
        assert!((cfg.resolve_stage(src_portrait()).padding - 40.0).abs() < 1e-9, "横画布 k=1");
        cfg.canvas.orientation = "portrait".into();
        assert!(
            (cfg.resolve_stage(src_portrait()).padding - 22.5).abs() < 1e-9,
            "竖画布 k=0.5625"
        );
    }

    /// 老 `[stage] position` 枚举 → 百分比：**`auto` 掉档为 50 居中**（2026-10-02 口径），
    /// 方向不跨用的值同样居中；新字段 `position_pct` 压过老枚举。
    #[test]
    fn legacy_position_enum_maps_to_pct() {
        assert_eq!(stage_position_pct("auto", false), 50.0);
        assert_eq!(stage_position_pct("auto", true), 50.0, "auto 一律居中，不再「竖素材靠左」");
        assert_eq!(stage_position_pct("left", false), 0.0);
        assert_eq!(stage_position_pct("right", false), 100.0);
        assert_eq!(stage_position_pct("top", true), 0.0);
        assert_eq!(stage_position_pct("bottom", true), 100.0);
        assert_eq!(stage_position_pct("center", false), 50.0);
        assert_eq!(stage_position_pct("top", false), 50.0, "方向不跨用 ⇒ 居中");

        let mut cfg = ProjectFile::example();
        cfg.stage.position = "right".into();
        assert_eq!(cfg.resolve_stage(src_portrait()).position_pct, 100.0);
        cfg.stage.position_pct = Some(25.0);
        assert_eq!(cfg.resolve_stage(src_portrait()).position_pct, 25.0, "新字段优先");
    }

    /// `position_pct` 规范化：NaN → 50，越界夹到 `[0,100]`；五档都能落进契约
    /// （`position` 枚举只保证两端精确，中间三档一律 `center` —— 消费侧改看 `position_pct`）。
    #[test]
    fn position_pct_is_normalized_and_mapped() {
        assert_eq!(normalize_stage_pct(f64::NAN), 50.0);
        assert_eq!(normalize_stage_pct(-5.0), 0.0);
        assert_eq!(normalize_stage_pct(250.0), 100.0);
        let mut cfg = ProjectFile::example();
        for (pct, pos) in [
            (0.0, "left"),
            (25.0, "center"),
            (50.0, "center"),
            (75.0, "center"),
            (100.0, "right"),
        ] {
            cfg.stage.position_pct = Some(pct);
            let f = cfg.resolve_stage(Some((1440, 1080))); // 4:3 → axis x，位置可见
            assert_eq!(f.position, pos, "pct {pct}");
            assert!((f.position_pct - pct).abs() < 1e-9, "pct {pct}");
        }
    }

    /// 三张新表要能进 `to_toml_commented` 再读回来（含段注释）
    #[test]
    fn new_sections_survive_commented_toml_roundtrip() {
        let mut cfg = ProjectFile::example();
        cfg.canvas.orientation = "portrait".into();
        cfg.stage.bg_mode = "color".into();
        cfg.stage.bg_color = "#123456".into();
        cfg.stage.blur_scale = 2.0;
        cfg.stage.position_pct = Some(75.0);
        cfg.stage.scale = 1.2;
        cfg.stage.padding = 24;
        cfg.sticker.position_mode = "manual".into();
        cfg.sticker.position = "25% 70%".into();

        let raw = cfg.to_toml_commented().unwrap();
        for sec in ["[canvas]", "[stage]", "[sticker]"] {
            assert!(raw.contains(sec), "缺 {sec}:\n{raw}");
        }
        assert!(raw.contains("# ===== 输出画布 ====="), "段注释要在:\n{raw}");
        assert!(raw.contains("# ===== 画面构成"), "{raw}");

        // ⚠️ 老 `position` 字段**只读不写**（`skip_serializing`）：`[stage]` 段里不该再有它。
        // 判据必须**只截 [stage] 段** —— 贴纸与事件也各有一个 `position` 键（同名不同义）。
        let stage_sec = raw.split("[stage]").nth(1).expect("要有 [stage] 段");
        let stage_sec = stage_sec.split("\n[").next().unwrap_or("");
        assert!(!stage_sec.contains("position ="), "老 stage.position 不该再写出:\n{stage_sec}");
        assert!(stage_sec.contains("position_pct"), "新字段要在:\n{stage_sec}");
        let back: ProjectFile = toml::from_str(&raw).expect("新表应可回读");
        assert_eq!(back.canvas.size(), (1080, 1920));
        assert_eq!(back.stage.position_pct, Some(75.0));
        assert_eq!(back.stage.bg_mode, "color");
        assert_eq!(back.stage.bg_color, "#123456");
        assert!((back.stage.blur_scale - 2.0).abs() < 1e-9);
        assert!((back.stage.scale - 1.2).abs() < 1e-9);
        assert_eq!(back.stage.padding, 24);
        assert_eq!(back.sticker.position_mode, "manual");
        assert_eq!(back.sticker.position, "25% 70%");
    }

    /// **存量工程的零回归**：没有这三张表的老 polish.toml，解析结果必须等同改造前
    #[test]
    fn legacy_project_without_new_sections_behaves_like_before() {
        let raw = "[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n";
        let cfg: ProjectFile = toml::from_str(raw).expect("老工程应可解析");
        assert_eq!(cfg.canvas.size(), (1920, 1080));
        assert_eq!(cfg.stage.position, "auto", "老枚举仍在（只读兼容）");
        assert_eq!(cfg.stage.position_pct, None, "没写过 ⇒ None，按老枚举换算");
        // 缺省背景 2026-10-02 起是 blur（老值 auto 与它等价，解析结果一致）
        assert_eq!(cfg.stage.bg_mode, "blur");
        assert_eq!(cfg.stage.scale, 1.0);
        assert_eq!(cfg.stage.padding, 0, "padding 字段保留（老工程照旧生效）");
        assert_eq!(cfg.sticker.position_mode, "auto");
        // 竖屏三开关：**缺字段 = 关**（用户口径：老工程也一并关）
        assert!(!cfg.stage.portrait_effects && !cfg.stage.portrait_camera && !cfg.stage.portrait_sfx);
        let f = cfg.resolve_stage(Some((1080, 1920)));
        assert_eq!(
            (f.axis.as_str(), f.position.as_str(), f.bg.as_str()),
            ("x", "center", "blur"),
            "竖素材 + auto 位置 ⇒ 左右留边 + 居中 + 模糊底"
        );
        assert!((f.position_pct - 50.0).abs() < 1e-9);
    }

    /// **模板消费面的一致性钉子**：`_canvas.stage` 的每个字段、JS 里拼出的每个 class、
    /// 每个 CSS 变量，都必须真在 `templates/index.html` 里有定义/被读到 ——
    /// 这类错误**不会编译报错**，只会表现成"改了没反应"，是最难查的一类回归。
    #[test]
    fn template_stage_hooks_match_contract() {
        let html = include_str!("../../templates/index.html");
        let contract = ProjectFile::example().canvas_contract(Some((1080, 1920)));
        let stage = contract["stage"].as_object().expect("stage 必须是对象");
        // 契约字段 ↔ 模板 JS 读的字段名（同名，缺一个就是"注入了却没人消费"）
        for key in [
            "axis", "object_fit", "position", "position_pct", "bg", "bg_color", "blur_px",
            "blur_img_scale", "scale", "padding",
        ] {
            assert!(stage.contains_key(key), "契约缺 stage.{key}");
            assert!(html.contains(key), "模板没消费 stage.{key}");
        }
        // CSS class：JS 里是 'axis-'+axis / 'stage-'+position 拼出来的，CSS 必须两套都全
        for cls in [
            "axis-fill", "axis-x", "axis-y",
            "stage-center", "stage-left", "stage-right", "stage-top", "stage-bottom",
        ] {
            assert!(html.contains(cls), "模板 CSS 缺 .{cls} 规则");
        }
        for needle in ["#fit-box", "--src-ar", "--stage-pad", "--stage-scale", "--bg-blur", "--bg-img-scale"] {
            assert!(html.contains(needle), "模板缺 {needle}");
        }
        // ⚠️ **出片路径（canvas `renderFrame`）的消费面**：DOM 的 `#bg-video-wrap` 只服务预览，
        // **不参与出片**（导出是 canvas 逐帧）。只把 `stage.bg` 消费在 DOM 上，
        // 表现就是"预览里有底色、成片被 shake / 画中画挪开就露纯黑"——注释修不掉，必须有消费点。
        for needle in [
            "function drawStageBackdrop",
            "function stageCoverRect",
            "var STAGE_BG=STAGE.bg||'none'",
            "drawStageBackdrop(ctx, videoBitmap)",
            "STAGE.blur_px||0",
        ] {
            assert!(html.contains(needle), "出片 canvas 路径缺背景层消费面: {needle}");
        }
        // 视频层（主画面）的摆放面：`stageDrawRect` 必须在，且被**用在 CTM 上** ——
        // 「算了不用」的表现就是源比例 ≠ 画布比例时整帧被硬铺满：竖素材进横画布 = 被拉成宽屏。
        // （几何断言在 `webui/scripts/check-stage-backdrop.mjs` ③c，这里只盯消费面别被删掉。）
        for needle in [
            "function stageDrawRect",
            "var sr = stageDrawRect(videoBitmap.width || CW",
            "ctx.translate(sr.x, sr.y); ctx.scale(sr.w / CW, sr.h / CH);",
        ] {
            assert!(html.contains(needle), "出片 canvas 路径缺视频层摆位消费面: {needle}");
        }
        // 顺序钉子：背景层必须先于视频层绘制（画反了铺满时就整块看不到底色）；
        // 摆放矩形的计算与 CTM 都必须在视频层 drawImage 之前。
        let backdrop = html.find("drawStageBackdrop(ctx, videoBitmap)").unwrap();
        let video_layer = html.find("ctx.drawImage(videoBitmap, 0, 0, CW, CH)").unwrap();
        assert!(backdrop < video_layer, "背景层必须画在视频层之前");
        let stage_rect = html.find("var sr = stageDrawRect(").unwrap();
        assert!(backdrop < stage_rect && stage_rect < video_layer, "摆放矩形要在衬底之后、视频层之前算");
        assert!(html.find("ctx.translate(sr.x, sr.y)").unwrap() < video_layer, "CTM 摆位必须在视频层 drawImage 之前");
        // 改造前的竖屏布局（`.portrait` 类 + `left/center`）必须彻底退场，
        // 留着就会和新的 `axis-x` + `stage-*` 规则打架。
        // ⚠️ 判据只能是**类选择器**（`.portrait`）：裸 `portrait` 会把合法用法一起打红 ——
        // 2026-10-02 竖屏三开关落地后，模板里就有 `orientation === 'portrait'`（枚举值）与
        // `PORTRAIT` / `portraitAllows`（标识符），它们**都不是**布局残留。
        assert!(!html.contains(".portrait"), "模板里还有老的 .portrait 布局残留");
    }
}
