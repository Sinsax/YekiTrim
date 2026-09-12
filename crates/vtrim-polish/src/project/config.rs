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

    /// 11. 视频屏蔽区间：这些时间段会被预览/导出跳过，LLM 也会忽略其中内容
    #[serde(default)]
    pub mute_ranges: Vec<MuteRange>,

    /// 12. 「去静音」自动检测出的区间记录（对应时间轴「屏蔽」轨道里的红色块）
    ///     仅作记录：撤销去静音时按它精确回退，不影响手动添加的屏蔽区间
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub silence_ranges: Vec<MuteRange>,

    /// 是否已执行过 process（preview 用它判断是否需要先 process）
    #[serde(default, skip_serializing_if = "is_false")]
    pub processed: bool,

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
// （类型定义已迁入 vtrim-render-core/types.rs，此处重导出保持兼容）
pub use vtrim_render_core::types::{
    grid_rect, region_origin, region_rects, parse_pct, CameraStyle, DanmakuStyle, Event,
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
    /// 背景手动平移（1920×1080 像素坐标，预览拖拽）
    #[serde(default)]
    pub bg_x: f64,
    #[serde(default)]
    pub bg_y: f64,
    /// 背景手动缩放倍率（预览滚轮）
    #[serde(default = "default_cover_bg_zoom")]
    pub bg_zoom: f64,
}

fn default_cover_bg_zoom() -> f64 { 1.0 }
fn default_cover_effect_color() -> String { "#000000".into() }

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CoverEffect {
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
fn default_cover_text_align() -> String { "center".into() }
fn default_cover_shadow_dir() -> String { "left".into() }
fn default_cover_shadow_intensity() -> f64 { 1.0 }
fn default_cover_effect_amount() -> f64 { 0.3 }
fn default_cover_effect_x() -> f64 { 50.0 }
fn default_cover_effect_y() -> f64 { 50.0 }
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
            text_align: default_cover_text_align(),
            shadow_dir: default_cover_shadow_dir(),
            shadow_intensity: default_cover_shadow_intensity(),
            text_shadow: true,
            highlight_keywords: Vec::new(),
            effects: Vec::new(),
            bg_x: 0.0,
            bg_y: 0.0,
            bg_zoom: default_cover_bg_zoom(),
        }
    }
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
                        "bg_x" => "# 背景水平偏移(px)",
                        "bg_y" => "# 背景垂直偏移(px)",
                        "bg_zoom" => "# 背景缩放倍率",
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
            mute_ranges: Vec::new(),
            silence_ranges: Vec::new(),
            processed: false,
            extra: HashMap::new(),
        }
    }
}



/// 将分段按屏蔽区间切开：被屏蔽的部分从 segments 中移除，剩下的部分保留原时间坐标。
pub fn split_segments_by_mute_ranges(segments: &mut Vec<Segment>, ranges: &[MuteRange]) {
    let ranges: Vec<(f64, f64)> = ranges.iter().filter_map(|r| r.normalized()).collect();
    if ranges.is_empty() || segments.is_empty() {
        return;
    }
    let mut out: Vec<Segment> = Vec::new();
    for seg in segments.drain(..) {
        let mut kept: Vec<(f64, f64)> = vec![(seg.start, seg.end)];
        for (ms, me) in &ranges {
            let mut next: Vec<(f64, f64)> = Vec::new();
            for (s, e) in kept {
                if *me <= s || *ms >= e {
                    next.push((s, e));
                    continue;
                }
                if s < *ms {
                    next.push((s, ms.min(e)));
                }
                if *me < e {
                    next.push((me.max(s), e));
                }
            }
            kept = next;
        }
        for (s, e) in kept {
            if e - s < 0.05 {
                continue;
            }
            let mut ns = seg.clone();
            ns.start = s;
            ns.end = e;
            let kept_events: Vec<Event> = seg
                .events
                .iter()
                .filter(|ev| {
                    let t = ev.get_time();
                    t >= s - 0.001 && t < e + 0.001
                })
                .cloned()
                .collect();
            ns.events = kept_events;
            out.push(ns);
        }
    }
    *segments = out;
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
