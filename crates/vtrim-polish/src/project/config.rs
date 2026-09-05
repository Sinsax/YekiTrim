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
    ///    LLM API 配置使用 .vtrim/llm.toml（与 vtrim 自身一致）
    #[serde(default)]
    pub filter: FilterConfig,

    /// 5. 视频时长 (可选, 从 clip.json 读取)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,

    /// 6. 直播间布局
    #[serde(default)]
    pub layout: Layout,

    /// 7. 视觉样式
    #[serde(default)]
    pub style: StyleRoot,

    /// 8. 贴图来源配置
    #[serde(default = "default_sticker_source")]
    pub sticker_source: Vec<String>,

    /// 9. 音效
    #[serde(default)]
    pub sfx: SfxConfig,

    /// 10. 时间轴
    #[serde(default)]
    pub segments: Vec<Segment>,

    /// 是否已执行过 process（preview 用它判断是否需要先 process）
    #[serde(default, skip_serializing_if = "is_false")]
    pub processed: bool,

    /// 扩展兜底
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[derive(Default)]
pub struct StyleRoot {
    #[serde(default)]
    pub subtitle: SubtitleStyle,
    #[serde(default)]
    pub danmaku: DanmakuStyle,
    #[serde(default)]
    pub camera: CameraStyle,
}

// ===== 运镜样式 =====

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct CameraStyle {
    /// 中景缩放 (camera zoom="中")
    #[serde(default = "default_zoom_mid")]
    pub zoom_mid: f64,
    /// 近景特写缩放 (camera zoom="近")
    #[serde(default = "default_zoom_close")]
    pub zoom_close: f64,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for CameraStyle {
    fn default() -> Self {
        Self {
            zoom_mid: default_zoom_mid(),
            zoom_close: default_zoom_close(),
            extra: HashMap::new(),
        }
    }
}

fn default_zoom_mid() -> f64 { 1.15 }
fn default_zoom_close() -> f64 { 1.3 }


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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    #[serde(default = "default_liver")]
    pub liver: LayoutRegion,

    #[serde(default = "default_danmaku_area")]
    pub danmaku_area: LayoutRegion,

    #[serde(default = "default_sticker_area")]
    pub sticker_area: LayoutRegion,

    #[serde(default)]
    pub reaction_zone: Option<LayoutRegion>,

    #[serde(default)]
    pub game_area: Option<LayoutRegion>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            liver: default_liver(),
            danmaku_area: default_danmaku_area(),
            sticker_area: default_sticker_area(),
            reaction_zone: None,
            game_area: None,
            extra: HashMap::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Point {
    /// x 百分比字符串，如 "65%"
    pub x: String,
    /// y 百分比字符串，如 "25%"
    pub y: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct LayoutRegion {
    /// 九宫格格子列表（可多个合并成更大区域）；旧版配置可能没有该字段，默认空数组
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grid: Vec<String>,
    /// 运镜聚焦中心（新）：九宫格格子名，如 "mid-right" / "center"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_grid: Option<String>,
    /// 运镜聚焦中心 (旧，格式 "x% y%")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// 旧版布局区域左上角/右下角（新字段为 grid，旧字段保留兼容）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_left: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom_right: Option<Point>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl LayoutRegion {
    /// 是否为空（无有效格子）
    pub fn is_empty(&self) -> bool {
        self.grid.is_empty()
    }
}

fn default_liver() -> LayoutRegion {
    LayoutRegion {
        grid: vec!["mid-right".into()],
        origin_grid: Some("mid-right".into()),
        origin: None,
        top_left: None,
        bottom_right: None,
        extra: HashMap::new(),
    }
}

fn default_danmaku_area() -> LayoutRegion {
    LayoutRegion {
        grid: vec!["top-left".into()],
        origin_grid: None,
        origin: None,
        top_left: None,
        bottom_right: None,
        extra: HashMap::new(),
    }
}

/// 默认表情/插画区域: 优先放在 liver 左侧，覆盖中左 + 下左
fn default_sticker_area() -> LayoutRegion {
    LayoutRegion {
        grid: vec!["mid-left".into(), "bot-left".into()],
        origin_grid: None,
        origin: None,
        top_left: None,
        bottom_right: None,
        extra: HashMap::new(),
    }
}

// ===== 九宫格 =====

/// 返回九宫格格子的标准矩形（百分比字符串）。
pub fn grid_rect(name: &str) -> Option<(Point, Point)> {
    let p = |x: f64, y: f64| Point {
        x: format!("{x:.2}%"),
        y: format!("{y:.2}%"),
    };
    let rect = match name {
        "top-left"   => (p(0.0, 0.0),    p(33.33, 33.33)),
        "top"        => (p(33.33, 0.0),  p(66.67, 33.33)),
        "top-right"  => (p(66.67, 0.0),  p(100.0, 33.33)),
        "mid-left"   => (p(0.0, 33.33),  p(33.33, 66.67)),
        "center"     => (p(33.33, 33.33),p(66.67, 66.67)),
        "mid-right"  => (p(66.67, 33.33),p(100.0, 66.67)),
        "bot-left"   => (p(0.0, 66.67),  p(33.33, 100.0)),
        "bottom"     => (p(33.33, 66.67),p(66.67, 100.0)),
        "bot-right"  => (p(66.67, 66.67),p(100.0, 100.0)),
        _ => return None,
    };
    Some(rect)
}

/// 解析百分比字符串为数值。
fn parse_pct(s: &str) -> f64 {
    s.trim_end_matches('%').trim().parse::<f64>().unwrap_or(0.0)
}

/// 将 LayoutRegion 的多个格子解析为独立矩形列表（不合并包围盒）。
/// 每个格子一个矩形，调用方按需选择（如贴纸随机选一个格子）。
/// 返回空 Vec 表示没有有效格子。
pub fn region_rects(region: &LayoutRegion) -> Vec<(Point, Point)> {
    let mut out = Vec::new();
    for name in &region.grid {
        if let Some((tl, br)) = grid_rect(name) {
            out.push((tl, br));
        }
    }
    out
}

/// 解析运镜聚焦中心（百分比字符串 "x% y%"）。
/// 优先级：origin_grid（格子中心） > origin（旧百分比） > grid 第一个格子中心。
pub fn region_origin(region: &LayoutRegion) -> Option<String> {
    if let Some(g) = &region.origin_grid {
        if let Some((tl, br)) = grid_rect(g) {
            let cx = (parse_pct(&tl.x) + parse_pct(&br.x)) / 2.0;
            let cy = (parse_pct(&tl.y) + parse_pct(&br.y)) / 2.0;
            return Some(format!("{cx:.2}% {cy:.2}%"));
        }
    }
    if let Some(o) = &region.origin {
        return Some(o.clone());
    }
    // 自动取第一个格子的中心
    if let Some(first) = region.grid.first() {
        if let Some((tl, br)) = grid_rect(first) {
            let cx = (parse_pct(&tl.x) + parse_pct(&br.x)) / 2.0;
            let cy = (parse_pct(&tl.y) + parse_pct(&br.y)) / 2.0;
            return Some(format!("{cx:.2}% {cy:.2}%"));
        }
    }
    None
}

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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct SubtitleStyle {
    #[serde(default = "default_font")]
    pub font: String,

    #[serde(default = "default_sub_size")]
    pub size: u32,

    #[serde(default = "default_sub_color")]
    pub color: String,

    #[serde(default = "default_sub_stroke")]
    pub stroke: String,

    #[serde(default = "default_highlight_color")]
    pub highlight_color: String,

    #[serde(default = "default_entrance")]
    pub entrance: String,

    #[serde(default = "default_sub_position")]
    pub position: String,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            font: default_font(),
            size: default_sub_size(),
            color: default_sub_color(),
            stroke: default_sub_stroke(),
            highlight_color: default_highlight_color(),
            entrance: default_entrance(),
            position: default_sub_position(),
            extra: HashMap::new(),
        }
    }
}

fn default_font() -> String { "LXGW WenKai".into() }
fn default_sub_size() -> u32 { 72 }
fn default_sub_color() -> String { "#dcbda0".into() }
fn default_sub_stroke() -> String { "12px #403c3b".into() }
fn default_highlight_color() -> String { "#f56b41".into() }
fn default_entrance() -> String { "cycle-5".into() }
fn default_sub_position() -> String { "bottom".into() }

// ===== 弹幕样式 =====

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct DanmakuStyle {
    #[serde(default = "default_dm_size")]
    pub font_size: u32,

    #[serde(default = "default_dm_color")]
    pub color: String,

    #[serde(default = "default_dm_stroke")]
    pub stroke: String,

    #[serde(default = "default_dm_highlight_color")]
    pub highlight_color: String,

    #[serde(default = "default_dm_speed")]
    pub speed: f64,

    #[serde(default = "default_dm_area")]
    pub area: String,

    #[serde(default = "default_dm_direction")]
    pub scroll_direction: String,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for DanmakuStyle {
    fn default() -> Self {
        Self {
            font_size: default_dm_size(),
            color: default_dm_color(),
            stroke: default_dm_stroke(),
            highlight_color: default_dm_highlight_color(),
            speed: default_dm_speed(),
            area: default_dm_area(),
            scroll_direction: default_dm_direction(),
            extra: HashMap::new(),
        }
    }
}

fn default_dm_size() -> u32 { 42 }
fn default_dm_color() -> String { "#ffffff".into() }
fn default_dm_stroke() -> String { "2px #000".into() }
fn default_dm_highlight_color() -> String { "#ff6bcb".into() }
fn default_dm_speed() -> f64 { 8.0 }
fn default_dm_area() -> String { "danmaku_area".into() }
fn default_dm_direction() -> String { "rtl".into() }

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
    /// (与全局 .vtrim/config.toml [sfx] blacklist 合并, 此处追加)
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
    #[serde(default = "default_cover_frame_pos")]
    pub frame_pos: f64,
    #[serde(default = "default_cover_font_size")]
    pub font_size: u32,
    #[serde(default = "default_cover_stroke")]
    pub stroke: u32,
    #[serde(default = "default_cover_hl_color")]
    pub hl_color: String,
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
}
fn default_cover_frame_pos() -> f64 { 0.4 }
fn default_cover_font_size() -> u32 { 120 }
fn default_cover_stroke() -> u32 { 20 }
fn default_cover_hl_color() -> String { "#ffcc00".into() }
fn default_cover_text_width() -> f64 { 0.62 }
fn default_cover_text_x() -> String { "35%".into() }
fn default_cover_text_y() -> String { "50%".into() }
fn default_cover_gradient_dir() -> String { "left".into() }
fn default_cover_vignette() -> f64 { 0.08 }
fn default_cover_letter_spacing() -> String { "8px".into() }
fn default_cover_line_height() -> f64 { 1.7 }
impl Default for CoverConfig {
    fn default() -> Self {
        Self {
            image: String::new(),
            frame_pos: default_cover_frame_pos(),
            font_size: default_cover_font_size(),
            stroke: default_cover_stroke(),
            hl_color: default_cover_hl_color(),
            text_width: default_cover_text_width(),
            text_x: default_cover_text_x(),
            text_y: default_cover_text_y(),
            gradient_dir: default_cover_gradient_dir(),
            vignette: default_cover_vignette(),
            letter_spacing: default_cover_letter_spacing(),
            line_height: default_cover_line_height(),
            title_lines: Vec::new(),
        }
    }
}

// ===== 时间轴 =====

// Segment 仅话题分组, 运镜由 Camera event 控制

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Segment {
    pub start: f64,
    pub end: f64,

    /// 情绪阈值: low/med/high, 控制该段反应密度
    #[serde(default = "default_fx")]
    pub fx: String,

    #[serde(default)]
    pub note: String,

    #[serde(default)]
    pub events: Vec<Event>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

fn default_fx() -> String { "low".into() }
#[allow(dead_code)] fn default_zoom() -> String { "中".into() }
#[allow(dead_code)] fn default_origin() -> String { "50% 50%".into() }



// Tagged Event Enum — 每个变体只有自己的字段
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    // === 贴图 (统一视觉元素, 来源由 sticker_source 配置决定) ===
    Sticker {
        time: f64, duration: f64,
        /// LLM 生成的描述关键词（贴纸包名，如 "ysyk"）
        keyword: String,
        /// Step2a LLM 选出的语义 tag（如 "害怕"）
        #[serde(default)]
        sticker_tag: Option<String>,
        /// process/Step2b 解析后的文件名 (不含扩展名)
        #[serde(default)]
        name: Option<String>,
        /// process/Step2b 解析成功后实际使用的来源 (emoji/irasu/local/包名)
        #[serde(default)]
        source: Option<String>,
        x: String, y: String,
        #[serde(default, deserialize_with = "flex_sfx")] sfx: Option<String>,
        sfx_volume: Option<f64>,
    },

    // === 运镜 (持续) ===
    Camera {
        time: f64, duration: f64,
        zoom: String,  // "中" | "近"
        origin: String,
    },

    // === 瞬时特效 (所有 sfx 接受布尔值) ===
    // duration 可选: LLM 可写特效持续秒数, 缺省用前端默认
    Shake      { time: f64, intensity: f64, #[serde(default)] duration: Option<f64>, #[serde(default)] anchor: Option<String>, #[serde(default, deserialize_with = "flex_sfx")] sfx: Option<String>, sfx_volume: Option<f64> },
    Flash      { time: f64, color: Option<String>, #[serde(default)] duration: Option<f64>, #[serde(default)] anchor: Option<String>, #[serde(default, deserialize_with = "flex_sfx")] sfx: Option<String>, sfx_volume: Option<f64> },
    Blur       { time: f64, #[serde(default)] duration: Option<f64>, #[serde(default)] anchor: Option<String>, #[serde(default, deserialize_with = "flex_sfx")] sfx: Option<String>, sfx_volume: Option<f64> },
    ZoomBounce { time: f64, #[serde(default)] duration: Option<f64>, #[serde(default)] anchor: Option<String>, #[serde(default, deserialize_with = "flex_sfx")] sfx: Option<String>, sfx_volume: Option<f64> },
    Vignette   { time: f64, #[serde(default)] duration: Option<f64>, #[serde(default)] anchor: Option<String>, #[serde(default, deserialize_with = "flex_sfx")] sfx: Option<String>, sfx_volume: Option<f64> },
    HueShift   { time: f64, color: Option<String>, #[serde(default)] duration: Option<f64>, #[serde(default)] anchor: Option<String>, #[serde(default, deserialize_with = "flex_sfx")] sfx: Option<String>, sfx_volume: Option<f64> },
    Stutter    { time: f64, intensity: Option<f64>, #[serde(default)] duration: Option<f64>, #[serde(default)] anchor: Option<String>, #[serde(default, deserialize_with = "flex_sfx")] sfx: Option<String>, sfx_volume: Option<f64> },
    Split      { time: f64, intensity: Option<f64>, #[serde(default)] duration: Option<f64>, #[serde(default)] anchor: Option<String>, #[serde(default, deserialize_with = "flex_sfx")] sfx: Option<String>, sfx_volume: Option<f64> },
    Noise      { time: f64, #[serde(default)] duration: Option<f64>, #[serde(default)] anchor: Option<String>, #[serde(default, deserialize_with = "flex_sfx")] sfx: Option<String>, sfx_volume: Option<f64> },
    Pulse      { time: f64, #[serde(default)] duration: Option<f64>, #[serde(default)] anchor: Option<String>, #[serde(default, deserialize_with = "flex_sfx")] sfx: Option<String>, sfx_volume: Option<f64> },
    ColorShift { time: f64, color: Option<String>, #[serde(default)] duration: Option<f64>, #[serde(default)] anchor: Option<String>, #[serde(default, deserialize_with = "flex_sfx")] sfx: Option<String>, sfx_volume: Option<f64> },

    // === 覆盖层 (持续) ===
    Overlay {
        time: f64, duration: f64, overlay_type: String,
        color: Option<String>, opacity: Option<f64>,
        colors: Option<Vec<String>>, shape: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")] sfx: Option<String>,
        sfx_volume: Option<f64>,
        #[serde(default)] anchor: Option<String>,
    },

    // === 纯音效 (无视觉) ===
    // name 由规则引擎按字幕上下文填充, LLM 可以不写
    Sfx {
        time: f64,
        #[serde(default)]
        name: String,
        #[serde(default)]
        volume: Option<f64>,
        /// 播放时长(秒)，不指定则文件原始长度
        #[serde(default)]
        duration: Option<f64>,
    },
}

impl Event {
    pub fn get_time(&self) -> f64 {
        match self {
            Event::Sticker { time, .. } => *time,
            Event::Camera { time, .. } => *time,
            Event::Shake { time, .. } => *time,
            Event::Flash { time, .. } => *time,
            Event::Blur { time, .. } => *time,
            Event::ZoomBounce { time, .. } => *time,
            Event::Vignette { time, .. } => *time,
            Event::HueShift { time, .. } => *time,
            Event::Stutter { time, .. } => *time,
            Event::Split { time, .. } => *time,
            Event::Noise { time, .. } => *time,
            Event::Pulse { time, .. } => *time,
            Event::ColorShift { time, .. } => *time,
            Event::Overlay { time, .. } => *time,
            Event::Sfx { time, .. } => *time,
        }
    }

    pub fn duration(&self) -> f64 {
        match self {
            Event::Sticker { duration, .. } => *duration,
            Event::Camera { duration, .. } => *duration,
            Event::Overlay { duration, .. } => *duration,
            _ => 0.5,
        }
    }
}

#[allow(dead_code)] fn is_zero(v: &f64) -> bool { *v == 0.0 }

/// sfx 字段的灵活反序列化: 接受字符串/null/布尔值
/// "impact-light" → Some("impact-light"), null → None
/// false → None, true → Some("pop-notice")
fn flex_sfx<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    use serde::Deserialize;
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::String(s) => Ok(if s.is_empty() { None } else { Some(s) }),
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Bool(b) => Ok(if b { Some("pop-notice".into()) } else { None }),
        _ => Ok(None),
    }
}

// ===== 默认值 =====

fn default_output() -> String { "vtuber-clip.mp4".into() }
fn default_metadata() -> String { "clip.json".into() }


impl ProjectFile {
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

            // [layout.*] 前输出九宫格总说明（TOML 无 [layout] 头，挂在第一个子表前）
            if trimmed == "[layout.liver]" {
                result.push_str("\n# ===== 布局（九宫格） =====\n");
                result.push_str("# 可用格子: top-left / top / top-right / mid-left / center / mid-right / bot-left / bottom / bot-right\n");
                result.push_str("# 每个区域可填一个或多个格子，多个格子会合并成包围盒区域\n");
                result.push_str("# 例如: grid = [\"mid-left\", \"center\"] 表示左中+正中合并\n");
                result.push_str("# liver: 主播区域，origin 为运镜聚焦中心\n");
                result.push_str("# grid 示例: [\"mid-right\"] 或 [\"mid-left\", \"center\", \"mid-right\"]\n");
                result.push_str(line); result.push('\n');
                continue;
            }
            if trimmed == "[layout.danmaku_area]" {
                result.push_str("# danmaku_area: 弹幕区域\n");
                result.push_str("# grid 示例: [\"top-left\"]\n");
                result.push_str(line); result.push('\n');
                continue;
            }
            if trimmed == "[layout.sticker_area]" {
                result.push_str("# sticker_area: 贴纸区域\n");
                result.push_str("# grid 示例: [\"mid-left\", \"bot-left\"]\n");
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
                result.push_str("# [[segments.events]]\n# type = \"camera\"\n# time = 0\n# duration = 10\n# zoom = \"中\"\n");
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
            processed: false,
            extra: HashMap::new(),
        }
    }
}



/// 将 segments/events 截断到指定时长 (防止 LLM 生成的 segment 超出视频长度)
pub fn clamp_segments_to_duration(segments: &mut Vec<Segment>, duration: f64) {
    if duration <= 0.0 { return; }
    // 移除起始时间 >= duration 的 segment
    segments.retain(|s| s.start < duration);
    for seg in segments.iter_mut() {
        seg.end = seg.end.min(duration);
        // 移除 segment 内起始时间 >= duration 的事件
        seg.events.retain(|e| e.get_time() < duration);
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
}
