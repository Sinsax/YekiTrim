//! 渲染核心的输入类型定义。
//!
//! 这些类型从 vtrim-polish 的 project/config.rs 迁入，作为渲染核心与
//! vtrim-polish / vtrim-wasm 的共享契约。vtrim-polish 通过 `pub use`
//! 重导出保持既有代码路径兼容。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ===== 时间轴分段 =====

/// Segment 仅话题分组, 运镜由 Camera event 控制
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

fn default_fx() -> String {
    "low".into()
}

// ===== Tagged Event Enum =====

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    // === 贴图 (统一视觉元素, 来源由 sticker_source 配置决定) ===
    Sticker {
        time: f64,
        duration: f64,
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
        x: String,
        y: String,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },

    // === 运镜 (持续) ===
    Camera {
        time: f64,
        duration: f64,
        zoom: String, // "中" | "近"
        origin: String,
    },

    // === 瞬时特效 (所有 sfx 接受布尔值) ===
    Shake {
        time: f64,
        intensity: f64,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    Flash {
        time: f64,
        color: Option<String>,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    Blur {
        time: f64,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    ZoomBounce {
        time: f64,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    Vignette {
        time: f64,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    HueShift {
        time: f64,
        color: Option<String>,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    Stutter {
        time: f64,
        intensity: Option<f64>,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    Split {
        time: f64,
        intensity: Option<f64>,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    Noise {
        time: f64,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    Pulse {
        time: f64,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    ColorShift {
        time: f64,
        color: Option<String>,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },

    // === 覆盖层 (持续) ===
    Overlay {
        time: f64,
        duration: f64,
        overlay_type: String,
        color: Option<String>,
        opacity: Option<f64>,
        colors: Option<Vec<String>>,
        shape: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
    },

    // === 纯音效 (无视觉) ===
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

/// sfx 字段的灵活反序列化: 接受字符串/null/布尔值
/// "impact-light" → Some("impact-light"), null → None
/// false → None, true → Some("pop-notice")
fn flex_sfx<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::String(s) => Ok(if s.is_empty() { None } else { Some(s) }),
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Bool(b) => Ok(if b { Some("pop-notice".into()) } else { None }),
        _ => Ok(None),
    }
}

// ===== 视觉样式 =====

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
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

fn default_zoom_mid() -> f64 {
    1.15
}
fn default_zoom_close() -> f64 {
    1.3
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

fn default_font() -> String {
    "LXGW WenKai".into()
}
fn default_sub_size() -> u32 {
    72
}
fn default_sub_color() -> String {
    "#dcbda0".into()
}
fn default_sub_stroke() -> String {
    "12px #403c3b".into()
}
fn default_highlight_color() -> String {
    "#f56b41".into()
}
fn default_entrance() -> String {
    "cycle-5".into()
}
fn default_sub_position() -> String {
    "bottom".into()
}

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

fn default_dm_size() -> u32 {
    42
}
fn default_dm_color() -> String {
    "#ffffff".into()
}
fn default_dm_stroke() -> String {
    "2px #000".into()
}
fn default_dm_highlight_color() -> String {
    "#ff6bcb".into()
}
fn default_dm_speed() -> f64 {
    8.0
}
fn default_dm_area() -> String {
    "danmaku_area".into()
}
fn default_dm_direction() -> String {
    "rtl".into()
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
    /// 是否为空（既没有九宫格格子，也没有自由选框）
    pub fn is_empty(&self) -> bool {
        self.grid.is_empty() && free_rect(self).is_none()
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
        "top-left" => (p(0.0, 0.0), p(33.33, 33.33)),
        "top" => (p(33.33, 0.0), p(66.67, 33.33)),
        "top-right" => (p(66.67, 0.0), p(100.0, 33.33)),
        "mid-left" => (p(0.0, 33.33), p(33.33, 66.67)),
        "center" => (p(33.33, 33.33), p(66.67, 66.67)),
        "mid-right" => (p(66.67, 33.33), p(100.0, 66.67)),
        "bot-left" => (p(0.0, 66.67), p(33.33, 100.0)),
        "bottom" => (p(33.33, 66.67), p(66.67, 100.0)),
        "bot-right" => (p(66.67, 66.67), p(100.0, 100.0)),
        _ => return None,
    };
    Some(rect)
}

/// 解析百分比字符串为数值。
pub fn parse_pct(s: &str) -> f64 {
    s.trim_end_matches('%').trim().parse::<f64>().unwrap_or(0.0)
}

/// 自由选框（新）：「框选布局」得到的精确矩形，落在 `top_left` / `bottom_right` 两个字段上。
///
/// 两个角都存在且面积有效时返回规范化的 (左上, 右下)：
/// 顺序化（拖拽可能左上右下颠倒）、百分比裁剪到 0~100、过小/退化返回 None。
/// 该矩形**优先于** `grid` —— 框选是更精确的表达，`grid` 保留作为清除选框后的回退。
pub fn free_rect(region: &LayoutRegion) -> Option<(Point, Point)> {
    let tl = region.top_left.as_ref()?;
    let br = region.bottom_right.as_ref()?;
    let x0 = parse_pct(&tl.x).clamp(0.0, 100.0);
    let y0 = parse_pct(&tl.y).clamp(0.0, 100.0);
    let x1 = parse_pct(&br.x).clamp(0.0, 100.0);
    let y1 = parse_pct(&br.y).clamp(0.0, 100.0);
    let (x0, x1) = (x0.min(x1), x0.max(x1));
    let (y0, y1) = (y0.min(y1), y0.max(y1));
    if x1 - x0 < 0.5 || y1 - y0 < 0.5 {
        return None;
    }
    let p = |x: f64, y: f64| Point {
        x: format!("{x:.2}%"),
        y: format!("{y:.2}%"),
    };
    Some((p(x0, y0), p(x1, y1)))
}

/// 将 LayoutRegion 解析为矩形列表（不合并包围盒）。
/// 优先级：自由选框（单矩形） > 九宫格格子（各自独立矩形）。
pub fn region_rects(region: &LayoutRegion) -> Vec<(Point, Point)> {
    if let Some(rect) = free_rect(region) {
        return vec![rect];
    }
    let mut out = Vec::new();
    for name in &region.grid {
        if let Some((tl, br)) = grid_rect(name) {
            out.push((tl, br));
        }
    }
    out
}

/// 解析运镜聚焦中心（百分比字符串 "x% y%"）。
/// 优先级：自由选框中心（框选出来的精确位置） > origin_grid（格子中心） >
/// origin（旧百分比） > grid 第一个格子中心。
pub fn region_origin(region: &LayoutRegion) -> Option<String> {
    if let Some((tl, br)) = free_rect(region) {
        let cx = (parse_pct(&tl.x) + parse_pct(&br.x)) / 2.0;
        let cy = (parse_pct(&tl.y) + parse_pct(&br.y)) / 2.0;
        return Some(format!("{cx:.2}% {cy:.2}%"));
    }
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

// ===== 文本输入（渲染核心的最小字幕/弹幕结构，与 vtrim-core 解耦）=====

/// 字幕输入：与 data/subtitles.json 的条目结构一致。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Subtitle {
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub html: String,
}

/// 弹幕输入：与 data/danmaku.json 的条目结构一致。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Danmaku {
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub y: f64,
    #[serde(default = "default_travel")]
    pub travel_duration: f64,
}

fn default_travel() -> f64 {
    8.0
}
