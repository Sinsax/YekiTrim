//! polish::cover：封面生成（图层栈 文字/关键帧/背景 → HTML 构建 + Takumi 静态渲染）。
//!
//! 入口：`cmd_cover`、`cmd_cover_template`、`cmd_preview_cover`、`build_cover_html`、`cover_bg_vars`、`cover_frame_vars`、`cover_blur_css`、`sanitize_hex_color`、`cover_bg_mode`。
//! 边界：`cover.image` 在消费侧要再做一次 `tenant::ensure_within`（纵深防御第③道，别删）；颜色进 HTML/CSS 前必须 `sanitize_hex_color`，公式只此一份（前端是镜像） —— 见 `crates/yekitrim-polish/AGENTS.md`

use std::path::Path;

use crate::project;
use crate::render::{ResolvedPaths, resolve_paths, server};

// `pub(crate)`：render/server.rs 的测试要断言模板里还留着字体占位符
pub(crate) const COVER_HTML: &str = include_str!("../templates/cover.html");

fn cover_text_align(cc: &crate::project::CoverConfig) -> (&'static str, &'static str) {
    match cc.text_align.as_str() {
        "left" => ("left", "flex-start"),
        "right" => ("right", "flex-end"),
        _ => ("center", "center"),
    }
}

fn cover_shadow_css(cc: &crate::project::CoverConfig) -> String {
    // 新语义：文字阴影只控制开关；方向感由背景渐变滤镜（gradient_dir）承担。
    // shadow_dir / shadow_intensity 仅用于兼容旧配置：旧配置关闭阴影时也能正确关闭。
    let legacy_off = cc.shadow_dir == "none" || cc.shadow_intensity <= 0.0;
    if cc.text_shadow && !legacy_off {
        "0 5px 22px rgba(0,0,0,0.78)".to_string()
    } else {
        "none".to_string()
    }
}

/// 出片模式追加的静态规则（模板占位 `__EXPORT_CSS__`）。
/// **为什么必须在 CSS 里、而不是靠页面 JS 加那个 `.export` 类**：出片这条路径（Takumi 静态渲染）
/// **不跑时间** —— `animation: fadeUp … both` 在"动画时间 0"下会把元素钉在起始帧
/// （`opacity:0`），静态渲染出来的标题**整块不可见**（实测：真封面 38.44/255，肉眼标题消失）。
///
/// 三条**各自独立**的规则：有的静态渲染器遇到不可解析的值会**丢掉整条声明块**
/// （Takumi 就是），所以不要把三条挤在一个块里。
///
/// 当年对 headless Chrome 逐字节验过**像素等价**：动画跑完的终态就是 `opacity:1 / translateY(0)`，
/// 与这三条一致（`transform:none` ≡ `translateY(0)`），零回归 SHA 不变。那条截图腿已删除，
/// 但**这三条只喂静态通道、不挂到预览链上**的理由不变（见 `build_cover_page_static`）。
const COVER_EXPORT_CSS: &str = "#title{animation:none}#title{opacity:1}#title{transform:none}";

/// 封面渲染通道名（`VTEDIT_COVER_RENDERER`）——静态渲染那条，也是**唯一一条**。
const RENDERER_TAKUMI: &str = "takumi";

/// 读封面渲染通道：空 / `takumi` = 静态渲染（Takumi，**唯一**）。
///
/// ⚠️ 认不出的值**当场报错**，不静默回退 —— 与 `render/dhampir.rs` 同一条口径（静默回退会让人
/// 以为"我在用静态渲染"，而那种错只有对比两版成片才看得出来）。
/// ⚠️ `chrome` 单独点名报错（**不是说它"认不出"**）：那条 headless Chrome 截图腿已于
/// 2026-10-01 整条删除，老脚本 / 老配置里还写着它的值。把它归到"认不出的值"里会让用户
/// 以为是打错字、去反复改拼写；这里必须直说"这条通道没了"。
fn resolve_cover_renderer(explicit: Option<&str>) -> anyhow::Result<String> {
    // 优先级：**显式参数（CLI/WebUI）> 环境变量 > takumi**。认不出的值一律当场报错 ——
    // 静默回退会让人以为"我在用静态渲染"，而那种错只有对比两版成片才看得出来。
    // ⚠️ 取值口径只此一份：服务端与 CLI 都不抄第二份（同 `--renderer` 默认值那条纪律）。
    //
    // 2026-10-01 切换：默认通道由 chrome 改为 **takumi**（用户拍板）；同日把 chrome 那条腿删除。
    // 依据：字体三处来源收敛之后重跑封面门禁（`scripts/check-cover-takumi.mjs`，clip-14 真工程）
    // 非文字区 **1.17/255**、整图 **11.44/255**，都在阈值内 —— 而且现在是**同库同字体**的对比，
    // 不再有"预览与实际对不上"那层偏差。参照帧已冻结在 `scripts/fixtures/cover-golden/`。
    if let Some(v) = explicit.map(str::trim).filter(|v| !v.is_empty()) {
        return normalize_cover_renderer(v, "--renderer");
    }
    let raw = std::env::var("VTEDIT_COVER_RENDERER").unwrap_or_default();
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(RENDERER_TAKUMI.to_string());
    }
    normalize_cover_renderer(raw, "VTEDIT_COVER_RENDERER")
}

fn normalize_cover_renderer(raw: &str, src: &str) -> anyhow::Result<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        RENDERER_TAKUMI => Ok(RENDERER_TAKUMI.to_string()),
        // ⚠️ 这条分支是**给老配置/老脚本的明确答复**，不是"还没删干净"：别把它简化掉，
        // 也别把它合并进下面的 `other`（那样用户只会看到"认不出 `chrome`"，去猜拼写）。
        "chrome" => anyhow::bail!(
            "{src} 传了 `chrome` —— 封面的 headless Chrome 截图通道已于 2026-10-01 **整条删除**，\
             现在只有 `takumi` 一条（静态渲染，进程内出图、不需要装浏览器）。\
             去掉这个参数即可用默认值；`VTEDIT_COVER_RENDERER` 请改成 `takumi` 或留空。"
        ),
        other => anyhow::bail!("{src} 认不出 `{other}`（现在只认 takumi）"),
    }
}

/// **静态封面渲染（Takumi）在本构建 / 本机上能不能用**：`(编了 feature, 字体库字体数, 字体库目录)`。
///
/// 给 `/api/health` 的能力位与排查用。判据 = ① 编了 `cover-takumi` feature ② 字体库里至少一个字体
/// —— Takumi **不读系统字体**，空库会明确报错（不静默出豆腐块）。
///
/// ⚠️ 计数判据与 `cover_font_files()` **同源**（同一个函数），别再手写第二份"数文件"的逻辑。
pub fn cover_static_capability() -> (bool, usize, std::path::PathBuf) {
    // ⚠️ **必须与 `cover_font_files()` 同源**（都走 `fonts::scan_all` + `resolve_all`）。
    // 以前这里手写"只数顶层文件"，v2 目录布局下**恒为 0** ⇒ `/api/health.cover_static.fonts` 报 0
    // ⇒ 前端把 Takumi 选项置灰 —— 明明有字体却"用不了静态封面"。
    // ⚠️ 数的是**真能用的**（`resolve_all` 解析得到的）："清单里有、还没下载"的自带字体不算，
    // 否则会告诉用户"有字体"然后出片时炸在"至少一个字体文件"上。
    let fonts = cover_font_files().len();
    (cfg!(feature = "cover-takumi"), fonts, yekitrim_core::fonts::dir())
}

/// 字体库里的全部字体文件（**真能用的那些**）。
///
/// 静态渲染**必须显式喂字体**（Takumi 不读系统字体、内置只有拉丁），所以这条路要求字体库里
/// 至少有一个字体；一个都没有时由 `render/takumi.rs` 明确报错（不静默出豆腐块）。
///
/// ⚠️ **不按 feature 门控**：`cover_static_capability()`（health 那个"有没有字体"的计数）
/// 也要用它，而那个函数在**没编 cover-takumi** 时同样要能算 —— 否则前端连
/// "为什么用不了静态封面"都问不出来（门控会让它编译不过，而不是回 0）。
#[cfg_attr(not(feature = "cover-takumi"), allow(dead_code))]
/// **这个进程干活用的字体库**（按 `Owner`，经 `YEKITRIM_OWNER` 从父进程继承）。
///
/// ⚠️ 封面这条腿也是**子进程**（`yekitrim polish cover <dir>`），所以它同样不能写死
/// `path::font_dir()` —— 多用户下那会让 A 导入的字体出现在 B 的封面里。
/// 单人（CLI / 托盘）没有 `YEKITRIM_OWNER`，解析结果就是现状位置，**零变化**。
fn font_dir_owned() -> std::path::PathBuf {
    yekitrim_core::fonts::dir_for(yekitrim_core::owner::Owner::from_env())
}

/// 封面这条腿该看的**全部**字体库，按优先级：`[用户库, 全局库]`。
///
/// ⚠️ **必须两层**（2026-10-10）：自带字体装在 `global/` 里，只看用户库会让封面
/// **一个字体都找不到**（Takumi 不读系统字体 ⇒ 直接报"字体库为空"）。
fn font_roots_owned() -> Vec<std::path::PathBuf> {
    yekitrim_core::owner::asset_dirs(
        yekitrim_core::owner::Owner::from_env(),
        yekitrim_core::owner::AssetKind::Fonts,
    )
}

fn cover_font_files() -> Vec<std::path::PathBuf> {
    cover_font_files_in_roots(&font_roots_owned())
}

/// [`cover_font_files`] 的可注入版本（单测用临时库）。
///
/// ⚠️ **必须走 `fonts::scan_all`**（两种布局都认：v2 目录 `<slug>/font.ttf` + 老平铺文件）。
// 以前这里手写"只列顶层文件"，v2 布局下**一个都扫不到** —— 而"一个字体都没有"这条路上
// `render_cover_png` 是**明确报错**的（不静默出豆腐块），所以症状是"静态封面直接出不来"。
// 同时它也保证"渲染看到的字体集合"与界面上列出来的**同一份**（同一个权威）。
//
// ⚠️ `FontEntry` 不带路径（它是元数据），所以按 id 走 `resolve_all` 反查文件 —— 与渲染路由
// `/vt-fonts/:name` 解析的是同一套规则；这一步也顺手滤掉"清单里有、还没下载"的自带字体
// （`installed: false`，那时它没有文件）。
//
// ⚠️ **顺序有意义（2026-10-01 收敛）**：Takumi 对认不出的族名会回退到"已注册字体按注册序"
// 的全集（实测见 `tests/takumi_font_id.rs`），所以**注册序第一支 ≈ 真正画标题的那支**。
// 这里把**库里设了默认的那支排最前**、其余按 id 稳定排序 —— 不再是裸 `out.sort()`：
// 那只按路径字典序排，"设为默认"对封面完全无效（LXGW 排在 Emoji 前纯属 L < N 的运气）。
#[cfg(test)]
fn cover_font_files_in(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    cover_font_files_in_roots(std::slice::from_ref(&root.to_path_buf()))
}

/// 多根版本：用户库优先、全局补缺（`roots[0]` 赢同 id 的后者）。
fn cover_font_files_in_roots(roots: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    let mut defaults: Vec<std::path::PathBuf> = Vec::new();
    let mut rest: Vec<(String, std::path::PathBuf)> = Vec::new();
    // ⚠️ **emoji 字体单独拎出来、排到最后注册**（2026-10-02，字体职责分离）。
    //
    // 为什么"排最后"就是"能找到 emoji"：Takumi 的 fallback 桶 = **全部已注册字体按注册序**
    // （本仓不设 `font_families`，见 `render/takumi.rs` 的 `RenderOptions` 与
    // `tests/takumi_font_id.rs` 的实测记录），所以"认不出的字形"按注册序找**第一个有它的**。
    // emoji 摆在最后 ⇒ 中文/标点先命中前面的正文字体，只有它们都没有的字形（就是 emoji）
    // 才落到 emoji 字体 —— 正是我们要的"独立补充槽位"。
    //
    // ⚠️ 反过来（emoji 排在前面）**不会**让它去画中文：Takumi 是**逐字形**找字体，
    // emoji 字体没有"中"字就去下一支。但把它排前面仍有两个坏处：
    // ① 数字/字母这类 emoji 字体**也有**的字形会被它抢走（观感突变）；
    // ② 与下面 `is_default` 的语义冲突（默认位是给正文用的）。
    let mut emojis: Vec<std::path::PathBuf> = Vec::new();
    for e in yekitrim_core::fonts::scan_all_roots(roots) {
        if let Some(p) = yekitrim_core::fonts::resolve_all_roots(roots, &e.id).filter(|p| p.is_file()) {
            if e.emoji {
                emojis.push(p);
            } else if e.is_default {
                defaults.push(p);
            } else {
                rest.push((e.id.to_lowercase(), p));
            }
        }
    }
    // 多个默认 = 手改过 FONT.toml 的病态情况：也排个稳定的序，别 panic
    defaults.sort();
    rest.sort();
    emojis.sort();
    defaults.extend(rest.into_iter().map(|(_, p)| p));
    // emoji 收尾（注册序最后 = fallback 桶的最后一档 = 只兜底别人都没有的字形）
    defaults.extend(emojis);
    defaults
}

/// 静态渲染（Takumi）出封面：与预览那条**同一份模板 + 同一条替换链**（只多 `__EXPORT_CSS__`），
/// 产物同样走 `unique_output_path`（同名加 `_1/_2`）。
#[cfg(feature = "cover-takumi")]
fn cover_render_static(
    project_path: &Path,
    config: &project::ProjectFile,
    title_lines: &[String],
    (cw, ch): (u32, u32),
) -> anyhow::Result<()> {
    let html = build_cover_page_static(config, project_path.join(COVER_BG_IMG).is_file(), title_lines);
    // CSS 由桥那边从同一份 HTML 里抽（`extract_style_blocks` 会先剥 HTML 注释 —— 模板里那句注释
    // 提到了 `<style>`，不剥就会被当成样式表）。
    let css = crate::render::takumi::extract_style_blocks(&html);
    let fonts = cover_font_files();
    let cover_file = crate::render::unique_output_path(&project_path.join("cover.jpg"));
    let t = std::time::Instant::now();
    crate::render::takumi::render_cover_png(&html, &css, project_path, &cover_file, cw, ch, &fonts)?;
    // 与预览那条同样的里程碑文本（`api/render.rs` 靠 `Cover saved:` 归类进任务日志）
    tracing::info!("   ✓ Cover saved: {}", cover_file.display());
    tracing::info!("   ⏱ 封面 {:.1}s（takumi / {} 个字体）", t.elapsed().as_secs_f64(), fonts.len());
    Ok(())
}

/// 没编 `cover-takumi` 时的占位：**明确报错**（封面只剩这一条通道，没有可退的）。
#[cfg(not(feature = "cover-takumi"))]
fn cover_render_static(
    _project_path: &Path,
    _config: &project::ProjectFile,
    _title_lines: &[String],
    (_cw, _ch): (u32, u32),
) -> anyhow::Result<()> {
    anyhow::bail!(
        "`VTEDIT_COVER_RENDERER=takumi` 但这个二进制没编 `cover-takumi` feature。\
         重编：cargo build -p yekitrim-cli --features yekitrim-polish/cover-takumi"
    )
}

/// 封面字体：`[cover] font` 选中的库内字体。
///
/// - `family` 直接进 CSS `__FONT_FAMILY__`（选中字体 + 系统栈兜底）；
/// - `face` 是给页面 `<head>` 的 `@font-face` 片段（浏览器只认系统已装字体，
///   不注入这一条，库里的字体在出片时根本不会生效）。
struct CoverFont {
    family: String,
    face: String,
}

/// 库里**默认位**那支的 id（没设默认 / 不可用 ⇒ `None`）。
///
/// ⚠️ 用 **id** 而不是内部族名：浏览器侧的 `@font-face` 声明的族名 = **id**
/// （`server::font_face_css`），栈首写内部族名就永远匹配不上 ⇒ 预览/设计器落回系统字体，
/// 而出片（Takumi 按内部族名注册）用的是库默认那支 —— 正是用户报的
/// 「没应用素材库设置的默认字体」。
fn cover_default_font_id() -> Option<String> {
    let roots = font_roots_owned();
    yekitrim_core::fonts::scan_all_roots(&roots)
        .into_iter()
        .find(|e| e.is_default && e.installed && !e.emoji && !e.id.trim().is_empty())
        .map(|e| e.id)
}

fn cover_font_css(cc: &crate::project::CoverConfig) -> CoverFont {
    let id = cc.font.trim();
    // ⚠️ **emoji 那支也要注入 @font-face**：标题里的 emoji 段（`emoji_run_span`）按库 id 请求它，
    // 不注入就是"浏览器解析不到 → 静默落回系统 Segoe"，而出片（Takumi）用库里的 Noto ——
    // 正是用户报的"前端与出片的 emoji 字体不一致"。
    let emoji = cover_emoji_font_id();
    // ⚠️ **空白 = 跟随字体库的默认位**（2026-10-19 用户报「没应用素材库设置的默认字体」）。
    // 以前这里直接给 `fallback_stack()`：首项是默认那支的**内部族名**，而浏览器只认库内字体的
    // **id**（`@font-face` 声明的名字 = id）⇒ 设计器/预览落回系统字体、出片却是库默认那支。
    // 现在统一按 id：注入它的 `@font-face`、栈首也写 id —— 与「选了具体字体」完全同一条路。
    let default_id = if id.is_empty() { cover_default_font_id() } else { None };
    let effective = if id.is_empty() { default_id.as_deref().unwrap_or("") } else { id };
    let face_ids: Vec<&str> = std::iter::once(effective)
        .chain(std::iter::once(emoji.as_str()))
        .filter(|s| !s.trim().is_empty())
        .collect();
    if effective.trim().is_empty() {
        // 库里连默认位都没有（空库）⇒ 动态兜底栈（与老的 `SYSTEM_FONT_STACK` 同族名同序）。
        return CoverFont { family: server::fallback_stack(), face: server::font_face_css(&face_ids) };
    }
    // 值不在字体库里时也照写（当系统字体族名用），与字幕字体同一条规矩
    CoverFont {
        family: server::font_family_with_stack(effective),
        face: server::font_face_css(&face_ids),
    }
}

fn cover_highlight_words(config: &crate::project::ProjectFile) -> Vec<String> {
    // 优先使用封面自己的高亮关键词；未设置时兼容旧的 filter.highlight。
    let mut out: Vec<String> = config
        .cover
        .highlight_keywords
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if out.is_empty() {
        out = config
            .filter
            .highlight
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    out
}

/// `#rrggbb` / `#rgb` → RGB。非法值一律回 (0,0,0)。
///
/// ⚠ 颜色是**用户输入**（手改 polish.toml、老工程、前端取色器）：这里绝不能索引越界 panic。
/// 3 位简写按 CSS 语义展开（`#abc` = `#aabbcc`，与前端 `fxHexToRgb` 同款），
/// 非 ASCII 或长度不对直接回退。
fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let h = hex.trim().trim_start_matches('#');
    let full: String = match h.len() {
        3 if h.is_ascii() => h.chars().flat_map(|c| [c, c]).collect(),
        6 if h.is_ascii() => h.to_string(),
        _ => return (0, 0, 0),
    };
    let part = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).unwrap_or(0);
    (part(0), part(2), part(4))
}

// ===== 封面背景模式（[cover] bg_mode / bg_color / blur_scale）=====

/// 模糊平铺的基准模糊半径（px，1920×1080 画布）；预览侧同源，改动要两边一起改。
pub const COVER_BLUR_BASE_PX: f64 = 28.0;
/// 模糊倍率的合法区间（滑杆与接口都按它夹取）。
pub const COVER_BLUR_SCALE_MIN: f64 = 0.2;
pub const COVER_BLUR_SCALE_MAX: f64 = 4.0;

/// 背景模式规范化：`color`（纯色，默认）/ `blur`（当前帧模糊平铺）/ `image`（导入的自定义背景图）。
/// 其它任何值（含老配置里的 `frame`）一律当 `color`。
///
/// ⚠️ `image` 还要**文件真的在**才成立 —— 缺文件时退回 `color`，见 [`cover_bg_vars`]。
pub fn cover_bg_mode(cc: &crate::project::CoverConfig) -> &'static str {
    match cc.bg_mode.trim().to_ascii_lowercase().as_str() {
        "blur" => "blur",
        "image" => "image",
        _ => "color",
    }
}

/// 只接受 `#rrggbb` / `#rgb`（注入 HTML 前必须净化：颜色直接进模板与 CSS，不能原样吃字符串）。
pub fn sanitize_hex_color(raw: &str, fallback: &str) -> String {
    let s = raw.trim();
    let hex = s.strip_prefix('#').unwrap_or(s);
    if (hex.len() == 6 || hex.len() == 3) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        format!("#{}", hex.to_ascii_lowercase())
    } else {
        fallback.to_string()
    }
}

/// 模糊倍率 → CSS 的 `blur(<px>)` 与铺满用的放大倍数。
///
/// 放大是为了把模糊溢出的边缘推出画面（blur 的有效扩散约 3σ，`blur_px` 就是 σ）。
/// 公式在预览（CoverDesigner.vue）里有一份镜像，改这里要同步改那边。
pub fn cover_blur_css(scale: f64) -> (u32, f64) {
    let s = if scale.is_finite() {
        scale.clamp(COVER_BLUR_SCALE_MIN, COVER_BLUR_SCALE_MAX)
    } else {
        1.0
    };
    let blur_px = COVER_BLUR_BASE_PX * s;
    let img_scale = 1.0 + blur_px / 150.0;
    (blur_px.round() as u32, (img_scale * 1000.0).round() / 1000.0)
}

/// 关键帧层（当前帧清晰图）的文件名。**只属于帧层** —— 背景层的图源见 [`COVER_BG_IMG`]。
pub const COVER_FRAME_IMG: &str = "_cover_frame.jpg";
/// 背景层「自定义图片」的文件名（导入背景图片写这里，不再覆盖关键帧）。
pub const COVER_BG_IMG: &str = "_cover_bg.jpg";

/// 背景层 → 模板变量。**预览与出片两个构建器共用**，别再各写一份（旧代码已有过重复分叉）。
/// `bg_image` = `_cover_bg.jpg` 是否存在（不存在时 `image` 模式退回 `color`，免得模板里挂个死图）。
pub struct CoverBgVars {
    pub class: String,
    pub color: String,
    pub blur: String,
    pub blur_scale: String,
    /// `#bg img` 的 src：blur 模式读关键帧、image 模式读背景图、color 模式随便（图不显示）
    pub src: String,
}

pub fn cover_bg_vars(config: &crate::project::ProjectFile, bg_image: bool) -> CoverBgVars {
    let cc = &config.cover;
    let color = sanitize_hex_color(&cc.bg_color, "#000000");
    let (blur_px, img_scale) = cover_blur_css(cc.blur_scale);
    let mut mode = cover_bg_mode(cc);
    if mode == "image" && !bg_image {
        mode = "color";
    }
    CoverBgVars {
        class: format!("bg-{mode}"),
        color,
        blur: format!("{blur_px}px"),
        blur_scale: format!("{img_scale}"),
        src: if mode == "image" { COVER_BG_IMG } else { COVER_FRAME_IMG }.to_string(),
    }
}

/// 帧层（当前帧清晰图）的位移/缩放 → 模板变量。
///
/// 原点取 `layout.liver` 的聚焦点（优先序见 `region_origin`：自由选框中心 > 格子 > 百分比；
/// 老逻辑：liver 在右半边 → 基准 1.4 倍，否则 1.3 倍），再乘 `[cover] bg_zoom`，
/// 最后按 `bg_x/bg_y`（px）平移。
/// **帧层与背景层是两回事**：背景（颜色/模糊帧）铺在帧之下，帧缩小/平移时才会露出来。
/// 前端 `CoverDesigner.vue` 的 `mediaStyle`/`liverOrigin`/`GRID_CENTERS` 是同一套公式的镜像，
/// 改一边要改另一边（漏改过的：自由选框优先序、bg_zoom 下限）。
pub struct CoverFrameVars {
    pub liver_x: String,
    pub liver_y: String,
    pub transform: String,
}

pub fn cover_frame_vars(config: &crate::project::ProjectFile) -> CoverFrameVars {
    let cc = &config.cover;
    let origin = crate::project::region_origin(&config.layout.liver).unwrap_or_else(|| "65% 50%".into());
    let nums: Vec<f64> = origin
        .split_whitespace()
        .filter_map(|s| s.trim_end_matches('%').parse().ok())
        .collect();
    let lx = nums.first().copied().unwrap_or(65.0);
    let ly = nums.get(1).copied().unwrap_or(50.0);
    let base_zoom = if lx >= 50.0 { 1.4 } else { 1.3 };
    // 夹取范围与设计器的滚轮/后端 patch（`api.rs` 的 `bg_zoom.clamp(0.5, 3.0)`）**一致**：
    // 以前这里是 0.2 起步，手写过 0.3 的工程出片会与预览对不上
    let zoom = base_zoom * cc.bg_zoom.clamp(0.5, 3.0);
    CoverFrameVars {
        liver_x: format!("{lx}%"),
        liver_y: format!("{ly}%"),
        transform: format!("translate({:.1}px, {:.1}px) scale({zoom:.3})", cc.bg_x, cc.bg_y),
    }
}

// ===== 图层栈（`[cover] layers`）=================================================
//
// 出片的叠放序**只此一份推导** = [`resolve_cover_stack`] + [`plan_cover_layers`]；
// 设计器（`webui/src/components/CoverDesigner.vue`）里有一份 JS 镜像，默认序两边必须一致
// （`node webui/scripts/check-cover-stage.mjs` 钉住）。
//
// 老工程没有 `layers` 字段 → 走 [`default_cover_stack`]，它与改造前模板写死的叠放序**逐像素等价**
// （bg < frame < 暗角 < 渐变 < 每个特效 < 标题），所以存量工程零迁移（判据 = 改前改后成片 SHA-256 相同）。

/// 栈里一层的**类型**。`Filter` 是"一层两块"：暗角 + 渐变永远相邻（设计器与出片都按这个前提走）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverLayerKind {
    Bg,
    Frame,
    Filter,
    Effect(usize),
    Title,
}

/// 解析出来的一层：`id` + 类型 + 可见性（`visible = false` = **在，但不画**）。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCoverLayer {
    pub id: String,
    pub kind: CoverLayerKind,
    pub visible: bool,
}

/// 图层 id（`[cover] layers` 与设计器共用一套字面量）
pub const COVER_LAYER_BG: &str = "bg";
pub const COVER_LAYER_FRAME: &str = "frame";
pub const COVER_LAYER_FILTER: &str = "filter";
pub const COVER_LAYER_TITLE: &str = "title";
/// 特效/表情层的 id 前缀：`fx:<CoverEffect::id>`
pub const COVER_LAYER_FX_PREFIX: &str = "fx:";

/// 特效在栈里的 id：优先用它自己的 `id`；老数据（没有 id）按数组序合成 `fx:#<i>`
/// —— 只用来在栈里匹配，**不回写**（老工程照旧零迁移）。
fn effect_slot_id(i: usize, fx: &crate::project::CoverEffect) -> String {
    let raw = fx.id.trim();
    if raw.is_empty() {
        format!("{COVER_LAYER_FX_PREFIX}#{i}")
    } else {
        format!("{COVER_LAYER_FX_PREFIX}{raw}")
    }
}

/// 默认栈：**就是改造前模板写死的叠放序**（从下到上）。
pub fn default_cover_stack(cc: &crate::project::CoverConfig) -> Vec<ResolvedCoverLayer> {
    let mut out = vec![
        ResolvedCoverLayer { id: COVER_LAYER_BG.into(), kind: CoverLayerKind::Bg, visible: true },
        ResolvedCoverLayer { id: COVER_LAYER_FRAME.into(), kind: CoverLayerKind::Frame, visible: true },
        ResolvedCoverLayer { id: COVER_LAYER_FILTER.into(), kind: CoverLayerKind::Filter, visible: true },
    ];
    for (i, fx) in cc.effects.iter().enumerate() {
        out.push(ResolvedCoverLayer {
            id: effect_slot_id(i, fx),
            kind: CoverLayerKind::Effect(i),
            visible: true,
        });
    }
    out.push(ResolvedCoverLayer { id: COVER_LAYER_TITLE.into(), kind: CoverLayerKind::Title, visible: true });
    out
}

/// 配置 → 图层栈（从下到上）。**容错优先**，四条规则都有单测：
/// ① 没有 `layers`（老工程）→ 默认栈；
/// ② `layers` 里缺了某层 → 按它在默认栈里的相对位置**补回来**（手改 toml 删一行不会让整层消失）；
/// ③ 未知 id → 忽略（前向兼容，别报错挡住出片）；同一个 id 出现多次 → 只认第一次；
/// ④ **没被 `layers` 列出的特效仍然渲染**（按数组序插回默认位置）—— 手改 toml 加一条 `[[cover.effects]]`
///    忘了同步 `layers`，不该"凭空少一层"；设计器删除特效层时会连 `effects` 那条一起删，所以不会有孤儿。
pub fn resolve_cover_stack(cc: &crate::project::CoverConfig) -> Vec<ResolvedCoverLayer> {
    let defaults = default_cover_stack(cc);
    if cc.layers.is_empty() {
        return defaults;
    }
    let mut out: Vec<ResolvedCoverLayer> = Vec::with_capacity(defaults.len());
    let mut seen: Vec<String> = Vec::new();
    for entry in &cc.layers {
        let id = entry.id.trim();
        if id.is_empty() || seen.iter().any(|s| s == id) {
            continue;
        }
        let Some(d) = defaults.iter().find(|d| d.id == id) else { continue };
        seen.push(d.id.clone());
        out.push(ResolvedCoverLayer { id: d.id.clone(), kind: d.kind, visible: entry.visible });
    }
    // 补齐：默认栈里没被列出的层，插在"它前面最近一个已经在栈里的默认项"之后
    for (di, d) in defaults.iter().enumerate() {
        if seen.iter().any(|s| s == &d.id) {
            continue;
        }
        let anchor = defaults[..di].iter().rev().find(|p| seen.iter().any(|s| s == &p.id));
        let pos = match anchor {
            Some(p) => out.iter().position(|l| &l.id == &p.id).map(|i| i + 1).unwrap_or(out.len()),
            None => 0,
        };
        seen.push(d.id.clone());
        out.insert(pos, ResolvedCoverLayer { id: d.id.clone(), kind: d.kind, visible: true });
    }
    out
}

/// 是不是"**默认栈 + 全部可见**"：老工程（没有 `layers`）以及没动过图层顺序的工程都走它。
///
/// 这种情况下**沿用改造前的渲染形式**（不注入逐层 z-index、特效不套 wrapper），让存量工程的成片与
/// 改造前**逐像素相同**。为什么必须这么做：实测给固定的五块都写上 z-index 之后，浏览器会换一条合成
/// 路径，缩放中的帧层重采样差 ~9/255 —— 肉眼看不出来，但 SHA-256 会变，而"横屏存量逐字节零回归"
/// 是本仓真出片对拍过的硬判据（见 `crates/yekitrim-polish/AGENTS.md`）。
pub fn is_default_stack(cc: &crate::project::CoverConfig) -> bool {
    let stack = resolve_cover_stack(cc);
    let defaults = default_cover_stack(cc);
    stack.len() == defaults.len()
        && stack.iter().zip(defaults.iter()).all(|(a, d)| a.id == d.id && a.visible)
}

/// 栈 → 每块的 `z-index`（整数，越大越靠前）与可见性。**出片只认它**（`cover_layer_css` + `cover_fx_html`）。
#[derive(Debug, Clone, PartialEq)]
pub struct CoverLayerPlan {
    pub bg: (u32, bool),
    pub frame: (u32, bool),
    pub vignette: (u32, bool),
    pub overlay: (u32, bool),
    pub title: (u32, bool),
    /// 与 `cc.effects` **同序**：`(z, visible)`
    pub effects: Vec<(u32, bool)>,
}

pub fn plan_cover_layers(cc: &crate::project::CoverConfig) -> CoverLayerPlan {
    let stack = resolve_cover_stack(cc);
    let mut plan = CoverLayerPlan {
        bg: (0, true),
        frame: (0, true),
        vignette: (0, true),
        overlay: (0, true),
        title: (0, true),
        effects: vec![(0, true); cc.effects.len()],
    };
    let mut z = 1u32;
    for layer in &stack {
        match layer.kind {
            CoverLayerKind::Bg => { plan.bg = (z, layer.visible); z += 1; }
            CoverLayerKind::Frame => { plan.frame = (z, layer.visible); z += 1; }
            // 一层两块：**先暗角后渐变**（与改造前的实际叠放序一致）
            CoverLayerKind::Filter => {
                plan.vignette = (z, layer.visible);
                z += 1;
                plan.overlay = (z, layer.visible);
                z += 1;
            }
            CoverLayerKind::Effect(i) => {
                if let Some(slot) = plan.effects.get_mut(i) {
                    *slot = (z, layer.visible);
                }
                z += 1;
            }
            CoverLayerKind::Title => { plan.title = (z, layer.visible); z += 1; }
        }
    }
    plan
}

/// 计划 → 注入模板 `__LAYER_CSS__` 的规则块。
///
/// - `legacy = true`（默认栈）：**只把模板里那条 `#fx-extra{z-index:1}` 补回来** —— 改造前它是内联的，
///   其余五块一律不写 z-index，渲染路径与改造前完全一致（逐字节零回归）。
/// - `legacy = false`：固定的五块按栈写 z-index，并且**必须**把 `#fx-extra` 退回 `z-index:auto`
///   （它一有 z-index 就是个 stacking context，所有特效的 z-index 会被锁死在同一层里 ——
///   拖动排序看着生效、出片却不动，而且不报错）。
///
/// ⚠️ 这段两个构建器都要注入（预览与出片现在同走 `build_cover_page`）—— 只加一处就是"预览有层序、出片没有"。
pub fn cover_layer_css(plan: &CoverLayerPlan, legacy: bool) -> String {
    if legacy {
        return "#fx-extra{z-index:1;}".to_string();
    }
    let disp = |v: bool| if v { "" } else { "display:none;" };
    format!(
        "#bg{{z-index:{};{}}}#frame{{z-index:{};{}}}#vignette{{z-index:{};{}}}#overlay{{z-index:{};{}}}#title-wrap{{z-index:{};{}}}#fx-extra{{z-index:auto;}}",
        plan.bg.0, disp(plan.bg.1),
        plan.frame.0, disp(plan.frame.1),
        plan.vignette.0, disp(plan.vignette.1),
        plan.overlay.0, disp(plan.overlay.1),
        plan.title.0, disp(plan.title.1),
    )
}

/// 出片时抓取的区域：就是 `#page` 的整块画布（配合模板里的 `html.export`，
/// 页面不再缩放/居中，左上角对齐视口原点）。不这么抓拿到的是浏览器视口尺寸
/// （实测 1904×929），页面被居中 → 成片四周黑边。
/// ⚠️ 画布**恒 1920×1080 横版**，不读 `[canvas] orientation`（平台封面就是横的）。
/// 竖屏素材不改比例靠模板的 `object-fit:contain`，改这两个常量前先看那个测试。

/// 进模板前必须转义：`fx.text`（emoji 字符）是用户输入，直接拼进 HTML 就是模板注入。
fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// `kind = emoji` 的默认字符（没填 text 时贴这个）。
pub const COVER_EMOJI_DEFAULT: &str = "🙂";
/// emoji 文本最多留几个字符（带 ZWJ / 肤色修饰符的一个 emoji 就要 7~8 个码点）。
pub const COVER_EMOJI_MAX_CHARS: usize = 12;
/// emoji 尺寸区间（px，1920×1080 画布）：amount 0 → 48px，1 → 288px。
pub const COVER_EMOJI_MIN_PX: f64 = 48.0;
pub const COVER_EMOJI_MAX_PX: f64 = 288.0;
/// emoji 描边环的颜色 = 标题描边色（同一契约 `#2a2522`；设计器 fxStyle 是镜像）。
pub const EMOJI_STROKE_COLOR: &str = "#2a2522";

/// emoji 的字体栈：**先问库里那支（`yekitrim_core::fonts::emoji_font`），没有才回落到系统族名**。
///
/// ⚠️ **为什么必须动态**（2026-10-02 输入层收敛）：这段栈原来在**四处各写一份**
/// （这里、`takumi.rs::ensure_emoji_fallback`、`CoverDesigner.vue` ×3、`PolishPreview.vue`），
/// 靠注释互相提醒"要同源"。漂移的症状是"设计器里有 emoji、出片里没有"，而且**不报错**。
/// 现在库里那支是**权威**，系统族名只是"库是空的"时的兜底。
///
/// ⚠️ **系统族名在 Takumi 上是死代码**：Takumi 不读系统字体、只认注册过的字体
/// （`render/takumi.rs` 的 `Fonts::register`），所以 `'Segoe UI Emoji'` 这类名字
/// **不解析、也不起兜底作用** —— 真正让 Takumi 出 emoji 的是**注册序**（见
/// `cover_font_files_in` 的 emoji 收尾）。保留系统族名只是为了浏览器预览（设计器）那条路。
pub fn emoji_font_stack() -> String {
    let roots = font_roots_owned();
    let tail = "'Segoe UI Emoji','Apple Color Emoji','Noto Color Emoji',sans-serif";
    match yekitrim_core::fonts::emoji_font_roots(&roots) {
        Some(e) => {
            // ⚠️ **用库 id，不用内部族名**（2026-10-02 纠正）：浏览器那条路（设计器 `FontFace` /
            // `@font-face`）与服务器路由 `/api/polish/font?name=` **都按 id 解析**，声明成内部族名
            // 会两头落空 —— 服务器按 id 找不到就用 msyh 兜底，浏览器于是静默落回系统 Segoe，
            // 而出片（Takumi）用库里的 Noto ⇒ 用户报的"前后端 emoji 字体不一致"。
            // Takumi 侧照旧两个都注册（`font_aliases` 读 FONT.toml 的 id），所以 id 一样管用。
            let fam = e.id.clone();
            format!("{},{}", quote_family(&fam), tail)
        }
        None => tail.to_string(),
    }
}

/// 标题里的 emoji 段：包一层 span —— ① 指明**库里那支 emoji 字体**；② 用 8 向 `drop-shadow`
/// 叠出一圈**描边环**。
///
/// ⚠️ **为什么不能用 `-webkit-text-stroke`**（用户报的「渲染的 emoji 没有描边」）：
/// CBDT/COLR **位图字形**走 Takumi 的 `ResolvedGlyph::Bitmap` 分支，**没有描边路径**
/// （`draw_text_stroke` 只在 `Outline` 上调用）—— 标题里的 emoji 因此永远不吃描边。
/// `drop-shadow` 是对**整层位图**做几何平移叠加（与字形格式无关），Chrome 侧同一条链
/// 同语义（都是"位移副本叠一圈"）⇒ 两端观感一致。环宽取 `stroke/4` = 文字描边外扩量的**一半**
/// （2026-10-02 用户口径：`stroke/2` 太厚）。
///
/// ⚠️ **必须 `display:inline-block`**：实测 Takumi 对**行内** span 的 `filter` 不生效（不出环，
/// 逐像素与无 filter 版相同），改成 inline-block 才走离屏子画布；同时实测 emoji 的尺寸与位置
/// 两版**逐像素一致**（红滴掩膜 w=50 / h=74 三行相同）⇒ 不会把它画大或挪位。
///
/// ⚠️ **族名用库 id**（不是内部族名）：浏览器那条路 `/api/polish/font` 按 **id** 解析字节、
/// `@font-face` 也按 id 声明；Takumi 注册时**内嵌族名与库 id 别名都注册**（`font_aliases`）。
/// 两边因此指向**同一支字体文件** —— 这正是"预览与出片的 emoji 必须同一支"的收口。
fn emoji_run_span(text: &str, family: &str, ring: f64) -> String {
    let fam = if family.trim().is_empty() {
        String::new()
    } else {
        format!("font-family:{};", quote_family(family))
    };
    let filter = if ring > 0.0 {
        let mut f = String::new();
        // ⚠️ **只发 4 向（正上/下/左/右），别加对角**（2026-10-02 实测纠正）：
        // `filter: drop-shadow` 是**链式**的 —— 每条阴影吃的是"已叠加的整层"（takumi-raster
        // `apply_drop_shadow_filter` 的 `composite_pixel_under` / 浏览器同规范）。
        // 8 向时对角那两条会继续沿 -x/-y 推已叠加的层 ⇒ 最外缘 = k + 2k·0.707 ≈ **2.4k**：
        // 真封面实测 k=5px 时暗带 13px、k=10px 时 25px（用户："还是太厚"）。
        // 4 向的每条方向只被推一次 ⇒ 外缘**精确等于 k**（受控实测 k=5 → 4~6px）。
        for (dx, dy) in [(ring, 0.0), (-ring, 0.0), (0.0, ring), (0.0, -ring)] {
            f.push_str(&format!("drop-shadow({dx:.1}px {dy:.1}px 0 {EMOJI_STROKE_COLOR}) "));
        }
        format!("filter:{};", f.trim_end())
    } else {
        String::new()
    };
    format!(
        "<span class=\"cov-emoji\" style=\"{fam}display:inline-block;{filter}\">{}</span>",
        escape_html(text)
    )
}

/// emoji 基字符（可否**开启**一个 emoji 段）：绘文字/表情/杂项符号/补充符号常用块。
fn is_emoji_base(cp: u32) -> bool {
    matches!(cp, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF)
}

/// emoji **延伸**字符：变体选择符 / ZWJ / 结合用键帽 / 肤色修饰符 / 标签序列。
fn is_emoji_extend(cp: u32) -> bool {
    matches!(cp, 0xFE0F | 0xFE0E | 0x200D | 0x20E3 | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F)
}

/// 把一行标题按 "emoji 段 / 其余文本" 切开，emoji 段交给 [`emoji_run_span`] 包起来。
///
/// 段规则：**基字符开启**，随后**紧邻的延伸字符**（VS16/ZWJ/肤色/键帽）与 ZWJ 之后的下一个基
/// 字符都算同一段（ZWJ 家族序列因此不会被切碎）。孤立的 VS16（前面不是 emoji）不进段。
fn wrap_emoji_runs(line: &str, family: &str, ring: f64) -> String {
    if family.trim().is_empty() && ring <= 0.0 {
        return line.to_string();
    }
    let mut out = String::new();
    let mut run = String::new();
    let mut prev_zwj = false;
    for ch in line.chars() {
        let cp = ch as u32;
        let join = is_emoji_base(cp) || (!run.is_empty() && (is_emoji_extend(cp) || prev_zwj));
        if join {
            run.push(ch);
        } else {
            if !run.is_empty() {
                out.push_str(&emoji_run_span(&run, family, ring));
                run.clear();
            }
            out.push(ch);
        }
        prev_zwj = cp == 0x200D;
    }
    if !run.is_empty() {
        out.push_str(&emoji_run_span(&run, family, ring));
    }
    out
}

/// 库里**那支 emoji 字体**的 id（没有 = 空串）。
fn cover_emoji_font_id() -> String {
    yekitrim_core::fonts::emoji_font(&font_dir_owned())
        .map(|e| e.id)
        .unwrap_or_default()
}

/// CSS 族名字面量：包单引号并转义（族名来自用户字体文件的 `name` 表，可能带引号）。
fn quote_family(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('\'');
    out
}

/// 特效 / 表情 → HTML。
///
/// - `legacy = true`（默认栈）：**就是改造前那份输出**（一个 div、不带 z-index、不套 wrapper）——
///   存量工程逐字节零回归的前提。
/// - `legacy = false`：每条特效套一层只负责全局 `z-index` 的 wrapper（它才是独立图层），
///   `visible=false` 写 `display:none`。
fn cover_fx_html(cc: &crate::project::CoverConfig, plan: &CoverLayerPlan, legacy: bool) -> String {
    let mut out = String::new();
    for (i, fx) in cc.effects.iter().enumerate() {
        let x = fx.x.clamp(0.0, 100.0);
        let y = fx.y.clamp(0.0, 100.0);
        let a = fx.amount.clamp(0.0, 1.0);
        // 大多数特效是铺满画面的图层，没有内容；emoji 是"一个点 + 一串字符"
        let (style, content) = match fx.kind.as_str() {
            "glow" => (
                format!(
                    "position:absolute;inset:0;background:radial-gradient(circle at {x:.1}% {y:.1}%, rgba(255,220,180,{:.2}) 0%, transparent 60%);",
                    a.min(0.9)
                ),
                String::new(),
            ),
            "vignette" => (
                format!(
                    "position:absolute;inset:0;background:radial-gradient(circle at {x:.1}% {y:.1}%, transparent 40%, rgba(0,0,0,{:.2}) 100%);",
                    a
                ),
                String::new(),
            ),
            "blur" => (
                format!(
                    "position:absolute;inset:0;backdrop-filter:blur({}px);-webkit-mask-image:radial-gradient(circle at {x:.1}% {y:.1}%, black 0%, transparent 60%);mask-image:radial-gradient(circle at {x:.1}% {y:.1}%, black 0%, transparent 60%);",
                    (1.0 + a * 14.0) as u32
                ),
                String::new(),
            ),
            "overlay" => {
                let (r, g, b) = hex_to_rgb(&fx.color);
                (
                    format!(
                        "position:absolute;inset:0;background:rgba({r},{g},{b},{:.3});mix-blend-mode:overlay;",
                        a
                    ),
                    String::new(),
                )
            }
            "scanline" => (
                format!(
                    "position:absolute;inset:0;background:repeating-linear-gradient(0deg, rgba(255,255,255,{:.3}) 0 1px, transparent 1px 4px);",
                    (a * 0.45).min(0.5)
                ),
                String::new(),
            ),
            // 表情：x/y 是**字符中心**（画布百分比）。默认落在主播位置（见 cover_emoji_origin），
            // 帧层的缩放原点就是那里 —— 那个点是变换的不动点，所以提示贴上去后缩放也不会跑偏。
            "emoji" => {
                let raw = if fx.text.trim().is_empty() { COVER_EMOJI_DEFAULT } else { fx.text.trim() };
                // 只留几个字符（emoji 带 ZWJ / 肤色修饰符一个就 7~8 个码点），
                // 别让一长串字把封面糊了
                let text: String = raw.chars().take(COVER_EMOJI_MAX_CHARS).collect();
                let size = (COVER_EMOJI_MIN_PX + a * (COVER_EMOJI_MAX_PX - COVER_EMOJI_MIN_PX)).round() as u32;
                // ⚠️ 走 `emoji_font_stack()`（唯一权威），别再就地写字体栈。
                // 这里是 **inline style** —— `takumi.rs::ensure_emoji_fallback` 只扫 `<style>` 块，
                // **永远扫不到这一处**，所以它必须自带完整栈（不能指望事后补）。
                let emoji_stack = emoji_font_stack();
                // ⚠️ **emoji 描边环**（用户报「渲染的 emoji 没有描边」）：
                // CBDT/CBLC 位图字形走 Takumi 的 `ResolvedGlyph::Bitmap` 分支，
                // **没有描边路径**（`draw_text_stroke` 只吃 `Outline`）——`-webkit-text-stroke`
                // 对 emoji 物理无效。唯一等价手法 = `filter: drop-shadow(±d 0 0 color)` 链：
                // Takumi 把每条 drop-shadow **顺序**应用在整层位图上（blur 0 = 干净平移、
                // `composite_pixel_under` 合到内容之下），四条链 ≈ 一圈描边。
                // 偏移随字号走（em 的 ~6%，与 20px 描边/320px 字的观感对齐）；颜色 =
                // 标题描边色 #2a2522（同一契约，前端 fxStyle 是镜像）。
                // 同口径削弱一半（0.06 → 0.03）：两条 emoji 路（标题联排 / 特效层）的环宽要一致
                let d = ((size as f64) * 0.03).round().max(1.0) as i32;
                let ring = format!(
                    "drop-shadow({d}px 0 0 {EMOJI_STROKE_COLOR}) drop-shadow(-{d}px 0 0 {EMOJI_STROKE_COLOR}) \
                     drop-shadow(0 {d}px 0 {EMOJI_STROKE_COLOR}) drop-shadow(0 -{d}px 0 {EMOJI_STROKE_COLOR})"
                );
                (
                    format!(
                        "position:absolute;left:{x:.1}%;top:{y:.1}%;transform:translate(-50%,-50%);font-size:{size}px;line-height:1;font-family:{emoji_stack};filter:{ring} drop-shadow(0 6px 14px rgba(0,0,0,0.45));"
                    ),
                    escape_html(&text),
                )
            }
            _ => continue,
        };
        if legacy {
            out.push_str(&format!("<div style=\"{style}\">{content}</div>"));
            continue;
        }
        // 每条特效一层：**外层 div 只负责全局 z-index**（特效之间、以及特效与固定块之间于是能任意排序），
        // 内层还是原来那个效果 div（内联样式一字不改）。
        let (z, visible) = plan.effects.get(i).copied().unwrap_or((0, true));
        let hidden = if visible { "" } else { "display:none;" };
        // ⚠️ `overlay` 额外加 `isolation:isolate`：它的 `mix-blend-mode:overlay` 在改造前**没有背景可混**
        //    （`#fx-extra` 当年是 stacking context，六条特效被锁成一个隔离组），实测"绿底叠 #ff3366"
        //    是橄榄绿；拆掉隔离会变成鲜绿 —— 那是**老封面变色**，不属于这次"图层栈"的范围。
        //    想要真正的混合走 P2 的「每层混合模式」（显式选择，才谈得上观感预期）。
        //    `isolation` 只加在这一种特效上：`blur` 的 backdrop-filter 要靠下层画面做背景，加不得。
        let iso = if fx.kind == "overlay" { "isolation:isolate;" } else { "" };
        out.push_str(&format!(
            "<div style=\"position:absolute;inset:0;z-index:{z};{hidden}{iso}\"><div style=\"{style}\">{content}</div></div>"
        ));
    }
    out
}


// ============================================================

pub async fn cmd_cover_template(project_path: &Path, template: &str) -> anyhow::Result<()> {
    // 读-改-写：只动 cover 预设字段，别把用户同时改的其它配置覆盖掉
    let tpl = template.to_string();
    project::update_polish_toml(project_path, move |cfg| {
        let cover = &mut cfg.cover;
        match tpl.as_str() {
            "center-text" => {
                cover.text_x = "50%".into();
                cover.text_width = 0.8;
                cover.gradient_dir = "radial".into();
                cover.vignette = 0.05;
                cover.text_align = "center".into();
                cover.shadow_dir = "center".into();
                cover.text_shadow = true;
            }
            "right-text" => {
                cover.text_x = "65%".into();
                cover.text_width = 0.62;
                cover.gradient_dir = "right".into();
                cover.text_align = "right".into();
                cover.shadow_dir = "right".into();
                cover.text_shadow = true;
            }
            "minimal" => {
                cover.text_x = "50%".into();
                cover.text_width = 0.7;
                cover.gradient_dir = "none".into();
                cover.vignette = 0.15;
                cover.text_align = "center".into();
                cover.shadow_dir = "none".into();
                cover.text_shadow = false;
            }
            _ => { // left-text / 未知: 恢复默认
                cover.text_x = "35%".into();
                cover.text_width = 0.62;
                cover.gradient_dir = "left".into();
                cover.vignette = 0.08;
                cover.text_align = "left".into();
                cover.shadow_dir = "left".into();
                cover.text_shadow = true;
            }
        }
    })
    .await?;
    tracing::info!("  ✓ [cover] preset: {}", template);
    Ok(())
}

pub async fn cmd_preview_cover(project_path: &Path) -> anyhow::Result<()> {
    let config = project::load_project(project_path)?;
    let resolved = resolve_paths(project_path, &config);
    let cover_image = project_path.join(COVER_FRAME_IMG);
    if !cover_image.exists() {
        // 没有封面帧则截一张
        let video_path = &resolved.video;
        // ⚠️ 时长走 `project_duration`（clip-meta.json 优先）：只 init 没跑 LLM 时 segments 是空的，
        //    以前 `frame_pos × 0` 让预览封面恒取 0.1s 那一帧（review polish.md P3；
        //    `cmd_cover` / `cmd_preview` 两处已同改，这里是最后一处）。
        let duration = crate::render::project_duration(project_path, &config);
        let frame_time = (duration * config.cover.frame_pos).max(0.1);
        let mut cmd = yekitrim_core::process::cmd("ffmpeg");
        cmd.args(["-ss", &frame_time.to_string(), "-i", video_path,
                  "-vframes", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "3", "-"]);
        // 截帧也走带超时的那个出口（E2；与 `cmd_cover` 的封面截帧同款），
        // 裸 `.output()` 在坏素材上会永久挂住。
        let out = crate::render::ffmpeg::Ffmpeg::run_with_timeout(
            cmd, "预览封面截帧", crate::render::ffmpeg::Ffmpeg::PROBE_TIMEOUT,
        )
        .map_err(|e| anyhow::anyhow!("预览封面截帧失败: {e:#}"))?;
        let data = out.stdout;
        if !data.is_empty() { std::fs::write(&cover_image, &data)?; }
    }

    let html = build_cover_html(project_path, &config, &resolved);
    let port = server::start_file_server(project_path.to_path_buf(), Some(html)).await?;
    let url = format!("http://127.0.0.1:{}", port);
    tracing::info!("  Cover URL: {}", url);
    yekitrim_core::process::cmd("cmd").args(["/c", "start", "", &url]).spawn().ok();
    loop { tokio::time::sleep(std::time::Duration::from_secs(3600)).await; }
}

/// 封面画布：**恒 1920×1080 横版**（不跟随 `[canvas] orientation` —— 平台封面就是横的）。
/// 竖屏素材在封面里靠模板的 `object-fit:contain` 保持原比例，画布尺寸本身不动。
pub const COVER_CAPTURE_W: f64 = 1920.0;
pub const COVER_CAPTURE_H: f64 = 1080.0;

/// 标题行：`cc.title_lines` 非空就用它；否则按中英标点拆 `project.name`（并砍掉 `【主播名】` 前缀）。
///
/// **纯函数**：出片那条路还要把首次拆出来的行写回 `polish.toml`（见 `cmd_cover`），那是调用方的事
/// —— 预览不该写盘。
fn cover_title_lines(config: &project::ProjectFile) -> Vec<String> {
    let cc = &config.cover;
    if !cc.title_lines.is_empty() {
        return cc.title_lines.clone();
    }
    let raw = &config.project.name;
    let clean = raw.split('【').next().unwrap_or(raw);
    clean
        .split(['，', '。', '！', '？', '、', ',', '.', '!'])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// 封面页 HTML 的**唯一实现**：预览（[`build_cover_html`]）与出片（`cmd_cover`）共用同一条替换链。
///
/// ⚠️ 以前两个构建器各写一份（漏改一边 = "预览有、出片没有"，**而且不报错**）。新增任何模板变量
/// **只在这里加一处**；图层栈的注入走 [`cover_layer_css`] + [`cover_fx_html`]。
fn build_cover_page(
    config: &project::ProjectFile,
    bg_image: bool,
    title_lines: &[String],
    export: bool,
) -> String {
    build_cover_page_ex(config, bg_image, title_lines, export, false)
}

/// **静态渲染器**（不执行 JS、不推进动画时间 —— 如 Takumi）用的那份：多填一张 `__EXPORT_CSS__`
/// （见 [`COVER_EXPORT_CSS`]）。
///
/// ⚠️ **必须是独立入口，不能顺手挂在预览那条链上**：实测（真封面 + headless Chrome 逐字节对拍）
/// `#title{animation:none…}` 会**改变浏览器的像素**（`72CD3A09…` → `46F19E2A…`）——
/// 动画会让该元素被提升成合成层，关掉之后回到主绘制，光栅化路径就换了（与仓库里
/// "给五块都写 z-index → 换合成路径 → SHA 变了"同一类）。所以预览侧**一个字都不改**；
/// 这三条只喂给静态通道（那条截图腿已删除，但**预览仍是浏览器渲染**，道理一样）。
pub fn build_cover_page_static(config: &project::ProjectFile, bg_image: bool, title_lines: &[String]) -> String {
    build_cover_page_ex(config, bg_image, title_lines, true, true)
}

fn build_cover_page_ex(
    config: &project::ProjectFile,
    bg_image: bool,
    title_lines: &[String],
    export: bool,
    static_render: bool,
) -> String {
    let cc = &config.cover;

    // 渐变（走 hex_to_rgb：3 位简写 / 非法值都不能越界 panic）
    let hl_color = &config.style.subtitle.highlight_color;
    let (gr, gg, gb) = if !hl_color.is_empty() && hl_color.starts_with('#') {
        let (r, g, b) = hex_to_rgb(hl_color);
        let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
        let nr = ((1.0 - r) * 30.0) as u8;
        let ng = ((1.0 - g) * 25.0) as u8;
        let nb = ((1.0 - b) * 50.0) as u8;
        (nr.max(10), ng.max(10), nb.max(15))
    } else {
        (20, 10, 40)
    };
    let grad_from = format!("rgba({},{},{},0.55)", gr, gg, gb);
    let grad_to = format!("rgba({},{},{},0.15)", gr, gg, gb);
    let grad_bg = match cc.gradient_dir.as_str() {
        "right" => format!("linear-gradient(270deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "top" => format!("linear-gradient(180deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "bottom" => format!("linear-gradient(0deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "radial" => format!("radial-gradient(ellipse at 30% 50%,{} 0%,{} 60%,transparent 100%)", grad_from, grad_to),
        _ if cc.gradient_dir == "none" => "none".into(),
        _ => format!("linear-gradient(90deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
    };

    // 标题 + 关键词高亮
    let hl_words = cover_highlight_words(config);
    // emoji 段：单独包 span（指定库里那支 emoji 字体 + 描边环）。**先切 emoji 再套高亮** ——
    // 高亮词是中文/日文关键词，不可能出现在"只有 emoji 字符"的那段里，顺序反了反而会把
    // span 标签当正文匹配。
    let emoji_id = cover_emoji_font_id();
    // 环宽 = 描边/4（= 文字描边外扩量的一半）：2026-10-02 用户口径「emoji 的描边太厚了，削弱一半」。
    // ⚠️ **不留 1px 下限**（原写法 `.max(1.0)`）：描边拖到 0 时 emoji 的环必须**一起消失** ——
    // 否则文字没描边、emoji 却挂着一圈，用户看到的就是"描边改了但它没跟着变"。
    let ring = (cc.stroke as f64 / 4.0).round();
    let title_html = title_lines.iter().map(|line| {
        let line = wrap_emoji_runs(line, &emoji_id, ring);
        if hl_words.is_empty() { return line; }
        let mut r = line;
        for w in &hl_words { if !w.is_empty() && r.contains(w.as_str()) { r = r.replace(w.as_str(), &format!("<span class=\"hl\">{}</span>", w)); } }
        r
    }).collect::<Vec<_>>().join("<br>");

    let (text_align, align_items) = cover_text_align(cc);
    let text_shadow = cover_shadow_css(cc);
    // 图层栈（`[cover] layers`）：默认栈走**改造前的渲染形式**（逐字节零回归），
    // 自定义栈才注入逐层 z-index / display
    let plan = plan_cover_layers(cc);
    let legacy = is_default_stack(cc);
    let fx_html = cover_fx_html(cc, &plan, legacy);
    let layer_css = cover_layer_css(&plan, legacy);
    let bg = cover_bg_vars(config, bg_image);
    let frame = cover_frame_vars(config);
    let font = cover_font_css(cc);
    COVER_HTML
        .replace("__COVER_W__", &COVER_CAPTURE_W.to_string())
        .replace("__COVER_H__", &COVER_CAPTURE_H.to_string())
        .replace("__TITLE__", &title_html)
        .replace("__FONT_FAMILY__", &font.family)
        .replace("__BG_CLASS__", &bg.class)
        .replace("__BG_COLOR__", &bg.color)
        .replace("__BG_BLUR__", &bg.blur)
        .replace("__BG_BLUR_SCALE__", &bg.blur_scale)
        .replace("__BG_SRC__", &bg.src)
        .replace("__LIVER_X__", &frame.liver_x)
        .replace("__LIVER_Y__", &frame.liver_y)
        .replace("__BG_TRANSFORM__", &frame.transform)
        .replace("__EXPORT__", if export { "true" } else { "false" })
        .replace("__LAYER_CSS__", &layer_css)
        .replace("__EXPORT_CSS__", if static_render { COVER_EXPORT_CSS } else { "" })
        .replace("__GRAD_BG__", &grad_bg)
        .replace("__VIGNETTE_OP__", &cc.vignette.to_string())
        .replace("__FONT_SIZE__", &format!("{}px", cc.font_size))
        .replace("__STROKE__", &format!("{}px", cc.stroke))
        .replace("__HL_COLOR__", &cc.hl_color)
        // 高亮词相对字号：与设计器的 --hl-fs 同源（镜像清单里的一条）
        .replace("__HL_FS__", &format!("{}em", cc.hl_font_scale))
        .replace("__TEXT_WIDTH__", &format!("{}%", (cc.text_width * 100.0) as u32))
        .replace("__TEXT_X__", &cc.text_x)
        .replace("__TEXT_Y__", &cc.text_y)
        .replace("__LS__", &cc.letter_spacing)
        .replace("__LH__", &cc.line_height.to_string())
        .replace("__TEXT_ALIGN__", text_align)
        .replace("__TEXT_ALIGN_ITEMS__", align_items)
        .replace("__SHADOW__", &text_shadow)
        .replace("__FX_HTML__", &fx_html)
        .replace("<!-- __FONT_FACE__ -->", &font.face)
}

/// 预览用的封面页（`polish preview` / WebUI 的封面预览）：标题走纯函数、**不写盘**。
pub fn build_cover_html(project_path: &Path, config: &project::ProjectFile, _resolved: &ResolvedPaths) -> String {
    let bg_image = project_path.join(COVER_BG_IMG).is_file();
    let title_lines = cover_title_lines(config);
    build_cover_page(config, bg_image, &title_lines, false)
}


/// 把 `[cover] image` 播种成关键帧 `_cover_frame.jpg` —— **只在关键帧还不存在时**。
///
/// ⚠️ 以前这里是**无条件** `fs::copy`：老工程 / 手改过 `cover.image` 的工程每出一次封面，
/// 就把用户导出（`/polish/cover-frame`）或上一轮截好的关键帧**覆盖掉** —— 与
/// 「出片不再删 `_cover_frame.jpg`、它就是关键帧层的图源」那条保护方向正好相反
/// （review `polish.md` E5 / D13：用户报过「又把关键帧那一层给我删了」）。
///
/// 路径囚笼保留（两道防线之一）：`cover.image` 是配置里的字符串、可手改，
/// 绝对路径下 `join` 会原样返回它本身 ⇒ 不囚笼就能把项目外任意文件复制进来当关键帧。
///
/// 返回 `true` = 这次真的播种了；`false` = 关键帧已在、保留它。
fn seed_frame_from_cover_image(project_path: &Path, image: &str) -> anyhow::Result<bool> {
    let frame = project_path.join(COVER_FRAME_IMG);
    if frame.exists() {
        tracing::info!(
            "    cover.image 未启用：关键帧 {COVER_FRAME_IMG} 已存在，保留它（不再覆盖）"
        );
        return Ok(false);
    }
    let img_path = yekitrim_core::tenant::ensure_within(project_path, Path::new(image))
        .map_err(|e| anyhow::anyhow!("Cover image 越出项目目录（{image}）: {e:#}"))?;
    if !img_path.exists() {
        anyhow::bail!("Cover image not found: {}", img_path.display());
    }
    std::fs::copy(&img_path, &frame)?;
    tracing::info!("    using custom image: {image}");
    Ok(true)
}

pub async fn cmd_cover(project_path: &Path, existing_video: Option<&str>, renderer: Option<&str>) -> anyhow::Result<()> {
    tracing::info!("\n  -- Cover --");
    let config = project::load_project(project_path)?;
    let resolved = resolve_paths(project_path, &config);
    let cc = &config.cover;

    // 首次运行时自动写入 [cover] 节（读-改-写：磁盘上其余字段原样保留）
    let raw_config = std::fs::read_to_string(project_path.join("polish.toml")).unwrap_or_default();
    if !raw_config.contains("[cover]") {
        project::update_polish_toml(project_path, |_| {}).await?;
        tracing::info!("  ✓ polish.toml: [cover] section added");
    }

    // 背景图: 自定义 image 或 ffmpeg 截帧
    // ⚠️ 时长走 `project_duration`（clip-meta.json → [project] duration → segments 末尾）：
    //    只 init 没跑 LLM 的工程 segments 是空的，以前 `frame_pos × 0` 让封面恒取 0.1s 那一帧。
    let duration = crate::render::project_duration(project_path, &config);
    if cc.image.is_empty() {
        // 优先使用已导出的关键帧（cover-frame 手动导出/导入背景生成），不要每次重新截帧覆盖
        let manual_frame = project_path.join(COVER_FRAME_IMG);
        if !manual_frame.exists() {
            let video_path = existing_video.unwrap_or(&resolved.video);
            let frame_time = (duration * cc.frame_pos).max(0.1);
            let mut cmd = yekitrim_core::process::cmd("ffmpeg");
            cmd.args(["-ss", &frame_time.to_string(), "-i", video_path,
                      "-vframes", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "3", "-"]);
            let frame_data = crate::render::ffmpeg::Ffmpeg::run_with_timeout(
                cmd, "封面截帧", crate::render::ffmpeg::Ffmpeg::PROBE_TIMEOUT,
            )?.stdout;
            if frame_data.is_empty() { anyhow::bail!("Frame extraction failed"); }
            std::fs::write(&manual_frame, &frame_data)?;
        }
    } else {
        // `cover.image` 只在**关键帧还不存在**时播种（它不再每次覆盖用户导出的那一帧）。
        seed_frame_from_cover_image(project_path, &cc.image)?;
    }

    // 标题: 先读 title_lines, 空则自动分行；**首次**把它写回 `polish.toml`（读-改-写，只动 title_lines）
    // —— 这是出片独有的副作用（预览不写盘），所以放在这里而不是 `build_cover_page` 里
    let title_lines: Vec<String> = {
        let lines = cover_title_lines(&config);
        if cc.title_lines.is_empty() && lines.len() > 1 {
            let to_write = lines.clone();
            project::update_polish_toml(project_path, move |cfg| {
                cfg.cover.title_lines = to_write;
            })
            .await?;
        }
        lines
    };

    // 封面画布**恒横版**（1920×1080），与成片方向解耦（竖屏素材靠模板的 contain 保持比例）
    let (cw, ch) = (COVER_CAPTURE_W, COVER_CAPTURE_H);

    // 渲染通道：`takumi` 静态渲染是**唯一**一条（2026-10-01 起默认，同日删掉 headless Chrome 截图腿）。
    // 传 `chrome` 会被下面那条单独点名报错，别把它折进"认不出"。
    // ⚠️ 认不出的值**当场报错**：静默回退会让人以为"我在用静态渲染"（与 `render/dhampir.rs` 同一条口径）。
    // 这里**只做校验**、不再分支 —— 解析出来不是 takumi 就是错误。
    let channel = resolve_cover_renderer(renderer)?;
    debug_assert_eq!(channel, RENDERER_TAKUMI);
    cover_render_static(project_path, &config, &title_lines, (cw as u32, ch as u32))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cover.image` → 关键帧的**播种**语义（review polish.md E5 / D13）：
    /// 只在 `_cover_frame.jpg` **不存在**时才复制；已在 ⇒ 保留（不再每次覆盖用户导出/
    /// 上轮截好的关键帧 —— 与「出片不再删 `_cover_frame.jpg`、它就是关键帧层图源」同向）；
    /// 越出项目目录的 `cover.image` 依旧报错（`tenant::ensure_within` 这道消费侧囚笼不松）。
    #[test]
    fn cover_image_only_seeds_when_frame_missing() {
        let dir = tempfile::tempdir().expect("临时目录");
        let root = dir.path();
        std::fs::write(root.join("img.jpg"), b"seed-bytes").unwrap();

        // ① 关键帧不存在 → 播种 + 返回 true
        let seeded = seed_frame_from_cover_image(root, "img.jpg").expect("播种必须成功");
        assert!(seeded, "第一次应当真的播种");
        assert_eq!(
            std::fs::read(root.join(COVER_FRAME_IMG)).unwrap(),
            b"seed-bytes",
            "关键帧 = cover.image 的内容"
        );

        // ② 关键帧已在 → 保留、不覆盖、返回 false（E5 的核心断言：老行为是每次覆盖）
        std::fs::write(root.join(COVER_FRAME_IMG), b"user-exported").unwrap();
        std::fs::write(root.join("img.jpg"), b"NEW-BYTES").unwrap();
        let again = seed_frame_from_cover_image(root, "img.jpg").expect("第二次也不该报错");
        assert!(!again, "关键帧已在 ⇒ 不播种");
        assert_eq!(
            std::fs::read(root.join(COVER_FRAME_IMG)).unwrap(),
            b"user-exported",
            "已有关键帧不许被 cover.image 覆盖（用户报过「又把关键帧那一层给我删了」）"
        );

        // ③ 指向关键帧自己：等价于"已在"，no-op
        let self_ref = seed_frame_from_cover_image(root, COVER_FRAME_IMG).expect("同文件 no-op");
        assert!(!self_ref, "关键帧已存在（它就是自己）⇒ 不播种");

        // ④ 越出项目目录 → 报错（路径囚笼保留：cover.image 是配置字符串、可手改）。
        //   ⚠️ 先**删掉关键帧**：函数在关键帧已存在时于囚笼**之前**就早退（保留语义），
        //   囚笼只在真的要播种的路径上才走到 —— 不删帧这条断言永远走不到。
        std::fs::write(dir.path().parent().unwrap().join("outside.jpg"), b"x")
            .expect("工程目录外的样本文件要写得出（囚笼才能按越界判，而不是按缺失判）");
        std::fs::remove_file(root.join(COVER_FRAME_IMG)).unwrap();
        let outside = seed_frame_from_cover_image(root, "../outside.jpg");
        assert!(outside.is_err(), "越出项目目录的 cover.image 必须报错（播种路径上的囚笼）");

        // ⑤ 指向不存在的文件 → 报错（与旧行为一致：明确失败，不静默）
        let missing = seed_frame_from_cover_image(root, "no-such.jpg");
        assert!(missing.is_err(), "cover.image 不存在时必须报错");
    }

    /// 封面字体：没选（空串）→ **跟随字体库默认位**（按 **id**：注入它的 @font-face、栈首也是它）；
    /// 库里连默认都没有 → 动态兜底栈；选了库外的值 → 当系统族名用、仍要有兜底栈，
    /// 而且**不注入 @font-face**（注了就是一条 404）。
    #[test]
    fn cover_font_css_follows_library_default() {
        let mut cc = crate::project::CoverConfig::default();
        let off = cover_font_css(&cc);
        // ⚠️ 2026-10-19：没选 ≠ 直接用兜底栈。用户报「没应用素材库设置的默认字体」——
        // 兜底栈首项是默认那支的**内部族名**，而浏览器只认 `@font-face` 声明的 **id**，
        // 于是预览落回系统字体、出片却是库默认那支。现在两边都按 id。
        match cover_default_font_id() {
            Some(id) => {
                assert!(
                    off.family.starts_with(&format!("\"{id}\",")),
                    "没选封面字体时栈首 = 库默认那支的 id（{id}）：{}",
                    off.family
                );
                assert!(
                    off.face.contains(&format!("font-family:\"{id}\"")),
                    "库默认那支必须注入 @font-face（否则浏览器解析不到）：{}",
                    off.face
                );
            }
            None => assert_eq!(off.family, server::fallback_stack(), "空库 = 兜底栈"),
        }
        // ⚠️ 比**族名与顺序**，不比引号风格：本文件的运行时串走 `quote_css`（**双引号**），
        // 而 `TAIL_STACK` 常量保留单引号（与真模板一致）—— 直接 `ends_with` 逐字比**必红**。
        // 这条断言第一版就是那么写错的，只是当时库里**没有默认字体**、`fallback_stack()` 的首项
        // 恰好也不带引号，才一直没暴露；2026-10-02 自带字体内嵌化之后库里一定有默认中文，
        // 于是它当场翻红（`cover_font_css_falls_back_to_system_stack`）。
        // 归一化写法与 `render/server.rs::stack_head_tracks_installed_library_font` 同口径。
        let norm = |s: &str| s.replace('"', "'").replace(' ', "");
        assert!(
            norm(&off.family).ends_with(&norm(",'Microsoft YaHei','Noto Sans SC',sans-serif")),
            "尾部系统栈兜底必须在: {}",
            off.family
        );
        // ⚠️ **emoji 那支照旧要注入**（2026-10-02 起）：标题里的 emoji 段（`emoji_run_span`）按库 id
        // 请求它；不注入 = 浏览器静默落回系统 Segoe，而出片用库里那支（用户报的"字体不一致"）。
        let emoji = cover_emoji_font_id();
        if emoji.is_empty() {
            assert_eq!(off.face, "", "库里没有 emoji 字体时不该注入任何 @font-face");
        } else {
            assert!(
                off.face.contains(&emoji),
                "emoji 那支必须注入（标题 emoji 段按它请求）：{}",
                off.face
            );
        }

        cc.font = "not-in-library-xyz".into();
        let miss = cover_font_css(&cc);
        assert!(miss.family.starts_with("\"not-in-library-xyz\","), "{}", miss.family);
        assert!(miss.family.contains("Microsoft YaHei"), "后面必须还有系统栈兜底: {}", miss.family);
        // 库外的字体**什么都不注入**（注入了就是一条 404 的 @font-face，老工程行为必须保持原样）；
        // 但 emoji 那支仍然在（它来自字体库，与用户选没选无关）。
        assert!(
            !miss.face.contains("not-in-library-xyz"),
            "库外字体不许注入 @font-face：{}",
            miss.face
        );
    }

    /// 标题里的 emoji 段切分：**基字符开段、延伸字符并入、ZWJ 序列不切碎、非 emoji 原样**。
    /// 判据与前端 `CoverDesigner.vue::wrapEmojiRuns` 同口径（**两份实现，改一边要改另一边**）。
    #[test]
    fn emoji_runs_are_wrapped_without_splitting_zwj_sequences() {
        let id = "NotoColorEmoji-CBDT";
        let s = wrap_emoji_runs("流鼻血🩸", id, 10.0);
        assert!(s.contains("cov-emoji"), "emoji 段要包 span：{s}");
        assert!(s.contains("font-family:'NotoColorEmoji-CBDT'"), "族名必须是库 id：{s}");
        assert!(s.contains("display:inline-block"), "必须 inline-block（Takumi 的 filter 才生效）：{s}");
        assert!(s.contains("drop-shadow(10.0px 0.0px 0 #2a2522)"), "描边环缺失：{s}");
        assert!(s.starts_with("流鼻血<span"), "非 emoji 必须原样留在外面：{s}");
        // ZWJ 家族序列整段（不许被切成三段）
        let zwj = wrap_emoji_runs("👨\u{200D}👩\u{200D}👧", id, 10.0);
        assert_eq!(zwj.matches("cov-emoji").count(), 1, "ZWJ 序列不许切碎：{zwj}");
        // VS16 跟着基字符（❤️ = U+2764 U+FE0F），后面那个「好」不进段
        let vs = wrap_emoji_runs("❤\u{FE0F}好", id, 10.0);
        assert!(vs.contains("cov-emoji") && vs.ends_with("好"), "{vs}");
        assert!(vs.contains("❤\u{FE0F}</span>"), "变体选择符要在段内：{vs}");
        // 没有 emoji 段时逐字不变
        assert_eq!(wrap_emoji_runs("幼儿园印象", id, 10.0), "幼儿园印象");
        // 没有 emoji 字体也没环 ⇒ 整条不动（老工程零变化）
        assert_eq!(wrap_emoji_runs("🩸", "", 0.0), "🩸");
    }

    /// emoji 段**内部的文本要转义**（标题是用户输入；虽然段里理论上只有 emoji，这层防御别删）。
    #[test]
    fn emoji_run_span_escapes_its_text() {
        let html = emoji_run_span("<b>&</b>", "id", 5.0);
        assert!(html.contains("&lt;b&gt;&amp;&lt;/b&gt;"), "段内文本必须转义：{html}");
        assert!(!html.contains("<b>"), "{html}");
    }

    /// 标题 emoji 段的**族名与 @font-face 同源**：两者都用库 id（浏览器按 id 解析字节，
    /// 服务器路由也按 id）。用内部族名会两头落空 —— 那正是「前后端 emoji 字体不一致」的根因。
    #[test]
    fn title_emoji_face_and_run_family_share_the_library_id() {
        let emoji = cover_emoji_font_id();
        if emoji.is_empty() {
            eprintln!("本机字体库没有 emoji 字体，跳过（CI 常态）");
            return;
        }
        let cc = crate::project::CoverConfig::default();
        let font = cover_font_css(&cc);
        assert!(
            font.face.contains(&format!("font-family:\"{emoji}\"")),
            "@font-face 必须按 id 声明：{}",
            font.face
        );
        assert!(
            wrap_emoji_runs("🩸", &emoji, 10.0).contains(&format!("font-family:'{emoji}'")),
            "emoji 段的族名必须与 @font-face 同一个 id"
        );
    }

    /// ⚠️ 「设为默认」必须真的决定 Takumi 的兜底（注册序第一支）：
    /// 默认字体排最前、其余按 id 稳定排序 —— 不再是裸 `out.sort()` 的文件名运气。
    #[test]
    fn cover_font_files_put_default_first() {
        let root = std::env::temp_dir().join(format!("vt-cover-font-order-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mk = |id: &str, def: bool| {
            let d = root.join(id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("font.ttf"), b"x").unwrap();
            yekitrim_core::fonts::write_meta(
                &d,
                &yekitrim_core::fonts::FontMeta { family: id.into(), is_default: def, ..Default::default() },
            )
            .unwrap();
        };
        mk("aaa-normal", false);
        mk("zzz-default", true);
        mk("mmm-normal", false);
        let files = cover_font_files_in(&root);
        assert_eq!(files.len(), 3, "未下载的自带字体不该算进来: {files:?}");
        assert!(
            files[0].to_string_lossy().contains("zzz-default"),
            "默认那支必须排第一（其余按 id 稳定序）: {files:?}"
        );
        let ids: Vec<String> = files.iter().map(|p| {
            p.parent().unwrap().file_name().unwrap().to_string_lossy().to_string()
        }).collect();
        assert_eq!(ids, vec!["zzz-default", "aaa-normal", "mmm-normal"], "{files:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    /// 颜色是用户输入（手改 polish.toml / 老工程 / 取色器）：3 位简写、非 ASCII、空串都不能 panic。
    /// 曾经这里是 `&h[0..2]` 直接切片 —— `#f00` 这种合法短写法会越界 panic，出片直接崩。
    #[test]
    fn hex_to_rgb_never_panics_and_expands_short_hex() {
        assert_eq!(hex_to_rgb("#ffcc00"), (255, 204, 0));
        assert_eq!(hex_to_rgb("ffcc00"), (255, 204, 0));
        assert_eq!(hex_to_rgb("#f00"), (255, 0, 0)); // 3 位简写 = #ff0000
        assert_eq!(hex_to_rgb("#abc"), (0xaa, 0xbb, 0xcc));
        assert_eq!(hex_to_rgb(""), (0, 0, 0));
        assert_eq!(hex_to_rgb("#"), (0, 0, 0));
        assert_eq!(hex_to_rgb("#12345"), (0, 0, 0)); // 长度不对
        assert_eq!(hex_to_rgb("#啊啊啊啊"), (0, 0, 0)); // 非 ASCII（字节切分会崩）
        assert_eq!(hex_to_rgb("#zzzzzz"), (0, 0, 0)); // 非十六进制
    }

    // ===== 图层栈（`[cover] layers`）=====

    fn fx(id: &str, kind: &str) -> crate::project::CoverEffect {
        crate::project::CoverEffect {
            id: id.into(),
            kind: kind.into(),
            amount: 0.3,
            x: 50.0,
            y: 50.0,
            color: "#000000".into(),
            text: String::new(),
        }
    }

    fn layer(id: &str, visible: bool) -> crate::project::CoverLayer {
        crate::project::CoverLayer { id: id.into(), visible }
    }

    /// 栈的 id 序列（从下到上）
    fn stack_ids(cc: &crate::project::CoverConfig) -> Vec<String> {
        resolve_cover_stack(cc).into_iter().map(|l| l.id).collect()
    }

    /// **零迁移的判据**：没有 `layers` 时，默认栈就是改造前模板写死的叠放序
    /// （bg < frame < 暗角<渐变 = filter < 每个特效 < 标题）。
    #[test]
    fn default_stack_matches_legacy_template_order() {
        let mut cc = crate::project::CoverConfig::default();
        cc.effects = vec![fx("a", "glow"), fx("", "emoji")];
        assert_eq!(stack_ids(&cc), vec!["bg", "frame", "filter", "fx:a", "fx:#1", "title"]);
        let plan = plan_cover_layers(&cc);
        assert!(
            plan.bg.0 < plan.frame.0 && plan.frame.0 < plan.vignette.0 && plan.vignette.0 < plan.overlay.0,
            "固定四块的层级必须严格递增：{plan:?}"
        );
        assert!(
            plan.overlay.0 < plan.effects[0].0 && plan.effects[0].0 < plan.effects[1].0 && plan.effects[1].0 < plan.title.0,
            "特效在渐变之上、标题之下：{plan:?}"
        );
        assert!(plan.bg.1 && plan.title.1, "默认全部可见");
    }

    /// `layers` 权威地描述顺序：显式栈怎么排，`plan` 就怎么给层级（表情可以跑到标题上面）。
    #[test]
    fn explicit_stack_drives_z_index() {
        let mut cc = crate::project::CoverConfig::default();
        cc.effects = vec![fx("a", "emoji")];
        cc.layers = vec![layer("title", true), layer("fx:a", true), layer("frame", false)];
        let plan = plan_cover_layers(&cc);
        assert!(plan.title.0 < plan.effects[0].0, "表情要排在标题之上：{plan:?}");
        assert!(!plan.frame.1, "frame 被隐藏");
        let got = stack_ids(&cc);
        assert_eq!(got[0], "bg", "缺项补齐：bg 应回到最底");
        assert!(got.contains(&"filter".to_string()), "缺项补齐：filter 要在：{got:?}");
        assert_eq!(got.iter().filter(|s| *s == "frame").count(), 1, "补齐不该出现重复：{got:?}");
    }

    /// 容错：未知 id 忽略、重复 id 只认第一次、空 id 跳过（手改 toml 不该让出片失败或丢层）。
    #[test]
    fn stack_tolerates_unknown_duplicate_and_empty_ids() {
        let mut cc = crate::project::CoverConfig::default();
        cc.layers = vec![
            layer("nope", true),
            layer("", true),
            layer("title", false),
            layer("title", true),
        ];
        let got = stack_ids(&cc);
        assert_eq!(got.iter().filter(|s| *s == "title").count(), 1, "{got:?}");
        assert!(!got.iter().any(|s| s == "nope"), "未知 id 要忽略：{got:?}");
        assert_eq!(got.last().map(String::as_str), Some("title"));
        assert!(!resolve_cover_stack(&cc).last().unwrap().visible, "重复项只认第一次（false）");
    }

    /// 手改 toml 加了一条 `[[cover.effects]]` 却没同步 `layers`：**仍然渲染**（插回默认位置）——
    /// 否则用户会看到"我配的特效凭空少了一层"，而且不报错。
    #[test]
    fn effects_missing_from_stack_still_render() {
        let mut cc = crate::project::CoverConfig::default();
        cc.effects = vec![fx("a", "glow"), fx("b", "scanline")];
        cc.layers = vec![layer("bg", true), layer("fx:a", true), layer("title", true)];
        let got = stack_ids(&cc);
        assert!(got.contains(&"fx:b".to_string()), "{got:?}");
        assert!(plan_cover_layers(&cc).effects[1].1, "没被列出的特效照旧可见");
    }

    /// 注入的 CSS：自定义栈时固定的五块带 z-index、隐藏的带 display:none，且 `#fx-extra` 必须退回
    /// `z-index:auto`（它有 z-index 就是 stacking context，特效全被锁死在一层）；
    /// **默认栈则一个 z-index 都不注入**（只补回模板原来那条 `#fx-extra`，存量成片逐字节不变）。
    #[test]
    fn layer_css_injects_z_index_and_display() {
        let mut cc = crate::project::CoverConfig::default();
        cc.layers = vec![layer("bg", false), layer("frame", true), layer("filter", true), layer("title", true)];
        let custom = cover_layer_css(&plan_cover_layers(&cc), false);
        assert!(custom.contains("#bg{z-index:1;display:none;}"), "{custom}");
        assert!(custom.contains("#frame{z-index:2;}"), "{custom}");
        assert!(custom.contains("#vignette{z-index:3;}") && custom.contains("#overlay{z-index:4;}"), "{custom}");
        assert!(custom.contains("#fx-extra{z-index:auto;}"), "自定义栈必须解掉 #fx-extra 的 stacking context：{custom}");

        let legacy = cover_layer_css(&plan_cover_layers(&crate::project::CoverConfig::default()), true);
        assert_eq!(legacy, "#fx-extra{z-index:1;}", "默认栈只能补回模板原来那一条");
    }

    /// 默认栈 vs 自定义栈的判定：老工程（没有 `layers`）与"顺序没动过且都可见"都算默认。
    #[test]
    fn default_stack_detection() {
        let mut cc = crate::project::CoverConfig::default();
        cc.effects = vec![fx("a", "glow")];
        assert!(is_default_stack(&cc), "没有 layers = 默认栈");
        cc.layers = vec![layer("bg", true), layer("frame", true), layer("filter", true), layer("fx:a", true), layer("title", true)];
        assert!(is_default_stack(&cc), "显式写了一遍默认序也算默认");
        cc.layers[3].visible = false;
        assert!(!is_default_stack(&cc), "隐藏了任意一层就不再是默认渲染路径");
        cc.layers[3].visible = true;
        cc.layers.swap(0, 4);
        assert!(!is_default_stack(&cc), "调过序就不再是默认渲染路径");
    }

    /// 出片 HTML 真的带上了注入，而且**预览/出片同一条链**（只有 `__EXPORT__` 不同）；
    /// 默认栈那份 HTML 必须是"改造前形状"（特效一个 div、不带 z-index）。
    #[test]
    fn cover_page_carries_layer_css_for_both_builders() {
        let mut c = cfg_with("color", "#000000", 1.0);
        c.cover.title_lines = vec!["标题".into()];
        c.cover.effects = vec![fx("e1", "glow")];
        // 默认栈（老工程的样子）：不注入逐层 z-index，特效就是原来那个 div
        let legacy = build_cover_page(&c, false, &c.cover.title_lines, false);
        assert!(legacy.contains("<style>#fx-extra{z-index:1;}</style>"), "{legacy}");
        assert!(!legacy.contains("#bg{z-index:"), "默认栈不该注入逐层 z-index：{legacy}");
        assert!(!legacy.contains("z-index:5;"), "默认栈的特效不带 z-index：{legacy}");

        // 自定义栈：逐层 z-index + 隐藏层 display:none
        c.cover.layers = vec![
            layer("bg", true),
            layer("frame", true),
            layer("filter", true),
            layer("fx:e1", false),
            layer("title", true),
        ];
        for export in [false, true] {
            let html = build_cover_page(&c, false, &c.cover.title_lines, export);
            assert!(html.contains("<style>#bg{z-index:1;}"), "export={export} 没注入图层 CSS：{html}");
            assert!(html.contains("z-index:5;display:none;"), "隐藏的特效要 display:none：{html}");
            assert!(!html.contains("__LAYER_CSS__"), "占位符没被替换：{html}");
            let want = if export { "var EXPORT = 'true' === 'true';" } else { "var EXPORT = 'false' === 'true';" };
            assert!(html.contains(want), "export={export} 的 EXPORT 变量不对：{html}");
        }
    }

    /// 高亮词字号（用户要求「给高亮区域增加一个字号设置，输出和前端要统一」）：
    /// ① 值必须真的流进 `--hl-fs`（不是只写了个占位符）；
    /// ② 老工程（toml 里没这个键）必须退成 `1em` = 与标题同号，逐像素零回归；
    /// ③ 两条链（预览 / 静态出片）读的是**同一个**变量。
    #[test]
    fn hl_font_scale_reaches_both_builders() {
        let mut c = cfg_with("color", "#000000", 1.0);
        c.cover.title_lines = vec!["标题".into()];

        // ② 默认值：没设过就是 1.0
        assert_eq!(c.cover.hl_font_scale, 1.0, "CoverConfig::default() 的高亮字号必须是 1.0");
        let html = build_cover_page(&c, false, &c.cover.title_lines, false);
        assert!(html.contains("--hl-fs:1em"), "默认没退成 1em：{html}");
        assert!(html.contains("font-size:var(--hl-fs"), "模板 .hl 没读 --hl-fs：{html}");

        // ① 改了值要跟着走
        c.cover.hl_font_scale = 1.4;
        let html = build_cover_page(&c, false, &c.cover.title_lines, false);
        assert!(html.contains("--hl-fs:1.4em"), "改值没流进 --hl-fs：{html}");

        // ③ 静态出片那条链同样
        let static_html = build_cover_page_static(&c, false, &c.cover.title_lines);
        assert!(static_html.contains("--hl-fs:1.4em"), "静态出片那条链没带上高亮字号：{static_html}");
    }

    /// 静态渲染通道（Takumi）与**预览那条**必须是两个入口，且模板里的两个新占位符都被替换掉。
    ///
    /// 1. 预览那条：**不含** `#title{animation:none…}`（当年实测它会改变 headless Chrome 的像素 ——
    ///    动画把元素提升成合成层，关掉后光栅化路径就换了，SHA 会变。那条截图腿已删除，
    ///    但**预览仍是浏览器渲染**，同理不许挂"关动画"规则）；
    /// 2. 静态那条：**必须含**这三条（不跑时间的渲染器会停在 `opacity:0`，标题整块不可见）；
    /// 3. 两条都不许残留模板占位符：模板加了占位、替换链漏了，字面量就会泄漏进 HTML（**不报错**）。
    ///    ⚠️ 判据要按**占位符名单**列，不能写成"含 `__` 就算漏" —— 模板里本来就有
    ///    `window.__COVER_READY`、`__CAPTURE_PROGRESS` 这类正常名字。
    #[test]
    fn static_builder_only_fills_export_css_and_no_placeholder_leaks() {
        let placeholders = [
            "__BG_COLOR__", "__BG_BLUR__", "__BG_BLUR_SCALE__", "__BG_TRANSFORM__", "__BG_CLASS__", "__BG_SRC__",
            "__LIVER_X__", "__LIVER_Y__", "__GRAD_BG__", "__VIGNETTE_OP__", "__FONT_SIZE__", "__STROKE__",
            "__HL_COLOR__", "__HL_FS__", "__TEXT_WIDTH__", "__TEXT_X__", "__TEXT_Y__", "__LS__", "__LH__", "__TEXT_ALIGN__",
            "__TEXT_ALIGN_ITEMS__", "__SHADOW__", "__TITLE__", "__LAYER_CSS__", "__EXPORT_CSS__",
        ];
        let no_leak = |html: &str| {
            for p in placeholders {
                assert!(!html.contains(p), "占位符 {p} 没被替换（模板加了、替换链漏了）：{html}");
            }
        };
        let mut c = cfg_with("color", "#000000", 1.0);
        c.cover.title_lines = vec!["标题".into()];

        for export in [false, true] {
            let preview_html = build_cover_page(&c, false, &c.cover.title_lines, export);
            assert!(
                !preview_html.contains("animation:none"),
                "预览这条链不该有关动画的规则（浏览器渲染会改像素）：{preview_html}"
            );
            assert!(preview_html.contains("--fs:"), "静态变量块两条链都要有：{preview_html}");
            no_leak(&preview_html);
        }

        let static_html = build_cover_page_static(&c, false, &c.cover.title_lines);
        for want in ["#title{animation:none}", "#title{opacity:1}", "#title{transform:none}"] {
            assert!(static_html.contains(want), "静态那条链缺规则 {want}：{static_html}");
        }
        assert!(static_html.contains("var EXPORT = 'true' === 'true';"), "静态渲染按出片形态：{static_html}");
        no_leak(&static_html);
    }

    fn cfg_with(bg_mode: &str, color: &str, scale: f64) -> crate::project::ProjectFile {
        let mut c: crate::project::ProjectFile =
            toml::from_str("[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n").unwrap();
        c.cover.bg_mode = bg_mode.into();
        c.cover.bg_color = color.into();
        c.cover.blur_scale = scale;
        c
    }

    fn resolved() -> ResolvedPaths {
        ResolvedPaths {
            video: "clip.mp4".into(),
            subtitles: "clip.srt".into(),
            danmaku: String::new(),
            metadata: String::new(),
            output: String::new(),
        }
    }

    /// 背景模式要落进 HTML，模板里**不留任何 `__*__` 占位符**
    /// （漏替一个占位符，出片就会出现裸字面量或整块背景失效）。
    /// 帧层（`#frame`）与背景层（`#bg`）是两层：帧可位移缩放、背景在帧之下。
    #[test]
    fn cover_html_renders_all_bg_modes() {
        for (mode, class) in [("color", "bg-color"), ("blur", "bg-blur")] {
            let html = build_cover_html(Path::new("."), &cfg_with(mode, "#123456", 2.0), &resolved());
            assert!(html.contains(&format!("class=\"{class}\"")), "{mode} 的 #bg 类不对：{html}");
            assert!(
                html.contains("'--bg-color', '#123456'"),
                "{mode} 的背景色没注入：{html}"
            );
            // 帧层必须单独一层（背景是"加的一层"，不是替换帧），而且**排在 #bg 之后**（在它之上）
            let bg_at = html.find("id=\"bg\"").expect("缺少背景层");
            let frame_at = html.find("id=\"frame\"").expect("缺少帧层");
            assert!(bg_at < frame_at, "{mode} 帧层必须在背景层之上：{html}");
            assert!(!html.contains("__BG_"), "{mode} 还有未替换的占位符：{html}");
            assert!(!html.contains("__LIVER_") && !html.contains("__BG_TRANSFORM__"),
                "{mode} 帧层占位符没替换：{html}");
            // 预览构建器必须写 false（保持缩放铺满 + 刷新按钮）；出片走 cmd_cover 的 true
            assert!(
                html.contains("var EXPORT = 'false'"),
                "{mode} 的 __EXPORT__ 没替换：{html}"
            );
            // 字体栈占位符必须被替换掉 —— 留着就是一条非法的 font-family，整个字体设置静默失效
            assert!(!html.contains("__FONT_FAMILY__"), "{mode} 字体栈占位符没替换：{html}");
            assert!(html.contains(&server::fallback_stack()), "{mode} 缺兜底字体栈（首项随库里默认走）：{html}");
        }
    }

    /// 封面画布**恒 1920×1080 横版**：即使工程是竖屏（`[canvas] orientation = portrait`）
    /// 封面也不跟着变 —— 平台封面就是横的，竖屏素材靠模板 `object-fit:contain` 保原比例。
    /// （曾经跟随过 orientation，用户要求改回固定横版。）
    #[test]
    fn cover_html_page_size_is_always_landscape() {
        for (label, orientation) in [("默认", ""), ("显式横屏", "landscape"), ("竖屏", "portrait")] {
            let mut c = cfg_with("color", "#000000", 1.0);
            if !orientation.is_empty() {
                c.canvas.orientation = orientation.into();
            }
            let html = build_cover_html(Path::new("."), &c, &resolved());
            assert!(html.contains("width:1920px; height:1080px"),
                "{label}封面 #page 必须是 1920×1080：{html}");
            assert!(html.contains("window.innerWidth/1920"),
                "{label}封面缩放基准必须是 1920：{html}");
            assert!(!html.contains("__COVER_W__") && !html.contains("__COVER_H__"),
                "{label}封面画布占位符没替换：{html}");
        }
    }

    /// 帧层 transform：原点跟 `layout.liver`（右半边 1.4 倍、左半边 1.3 倍）× `bg_zoom`，
    /// 平移取 `bg_x/bg_y`；越界倍率夹到 0.2~3。
    #[test]
    fn cover_frame_transform_follows_liver_and_zoom() {
        let mut c = cfg_with("color", "#000000", 1.0);
        // grid/origin_grid 优先于 origin（region_origin 的规则），要测 origin 就先清掉
        c.layout.liver.grid.clear();
        c.layout.liver.origin_grid = None;
        c.layout.liver.origin = Some("65% 50%".into());
        let v = cover_frame_vars(&c);
        assert_eq!(v.liver_x, "65%");
        assert_eq!(v.liver_y, "50%");
        assert!(v.transform.contains("scale(1.400)"), "{}", v.transform);
        assert!(v.transform.starts_with("translate(0.0px, 0.0px)"), "{}", v.transform);

        c.cover.bg_zoom = 0.5;
        c.cover.bg_x = -120.0;
        c.cover.bg_y = 30.0;
        let v = cover_frame_vars(&c);
        assert!(v.transform.contains("scale(0.700)"), "{}", v.transform);
        assert!(v.transform.contains("translate(-120.0px, 30.0px)"), "{}", v.transform);

        c.layout.liver.origin = Some("20% 50%".into());
        c.cover.bg_zoom = 1.0;
        let v = cover_frame_vars(&c);
        assert_eq!(v.liver_x, "20%");
        assert!(v.transform.contains("scale(1.300)"), "{}", v.transform);

        // 倍率本身夹到 3（与接口写入侧的 0.5~3 一致），所以 1.3×3 = 3.9
        c.cover.bg_zoom = 99.0;
        let v = cover_frame_vars(&c);
        assert!(v.transform.contains("scale(3.900)"), "倍率封顶 3：{}", v.transform);
        c.cover.bg_zoom = 0.0;
        let v = cover_frame_vars(&c);
        // 下限 0.5（设计器滚轮 / 后端 patch 都按 0.5~3 夹）→ 1.3×0.5 = 0.65
        assert!(v.transform.contains("scale(0.650)"), "倍率下限 0.5：{}", v.transform);
    }

    /// 模糊倍率：1.0 = 基准 28px；越界夹取；NaN 退回 1.0。
    #[test]
    fn cover_blur_px_and_scale_follow_multiplier() {
        let (px, scale) = cover_blur_css(1.0);
        assert_eq!(px, COVER_BLUR_BASE_PX as u32);
        assert!(scale > 1.0, "放大倍数必须大于 1，否则模糊边缘会露黑边：{scale}");

        assert_eq!(
            cover_blur_css(99.0).0,
            (COVER_BLUR_BASE_PX * COVER_BLUR_SCALE_MAX).round() as u32
        );
        assert_eq!(
            cover_blur_css(0.0).0,
            (COVER_BLUR_BASE_PX * COVER_BLUR_SCALE_MIN).round() as u32
        );
        assert_eq!(cover_blur_css(f64::NAN).0, COVER_BLUR_BASE_PX as u32);
        // 倍率越大铺得越满（不然高模糊时四边会露出底色）
        assert!(cover_blur_css(3.0).1 > cover_blur_css(0.5).1);
    }

    /// 颜色是**直接注进 HTML/CSS** 的字符串，必须净化（否则等于模板注入）。
    #[test]
    fn hex_color_is_sanitized_before_html_injection() {
        assert_eq!(sanitize_hex_color("#AABBCC", "#000000"), "#aabbcc");
        assert_eq!(sanitize_hex_color("abc", "#000000"), "#abc");
        assert_eq!(sanitize_hex_color("  #0f0f0f ", "#000000"), "#0f0f0f");
        assert_eq!(sanitize_hex_color("red;background:url(x)", "#000000"), "#000000");
        assert_eq!(sanitize_hex_color("", "#000000"), "#000000");

        // 走完构建器也要是净化后的值
        let html = build_cover_html(
            Path::new("."),
            &cfg_with("color", "#abc\"/><script>alert(1)</script>", 1.0),
            &resolved(),
        );
        assert!(!html.contains("alert(1)"), "恶意颜色串进了模板：{html}");
        assert!(html.contains("'--bg-color', '#000000'"), "非法颜色应退回默认色：{html}");
    }

    /// **真实像素探针**（ignored，默认不进门禁）：走完整封面生产链
    /// （`build_cover_page_static` → `extract_style_blocks` → `render_cover_png`）出 1920×1080 真图，
    /// 供人眼验收「中文描边 / emoji 字体 / emoji 描边」三件事（复刻 clip-14 黄金封面的形状）。
    ///
    /// 跑法：
    /// `VTEDIT_COVER_PROBE_DIR=<目录> VTEDIT_TAKUMI_DUMP=<目录> cargo test -p yekitrim-polish --lib \
    ///   --features cover-takumi probe_real_cover_pixels -- --ignored --nocapture`
    #[cfg(feature = "cover-takumi")]
    #[test]
    #[ignore]
    fn probe_real_cover_pixels() {
        let out_dir = std::path::PathBuf::from(
            std::env::var("VTEDIT_COVER_PROBE_DIR")
                .unwrap_or_else(|_| std::env::temp_dir().join("yekitrim-cover-probe").display().to_string()),
        );
        let _ = std::fs::create_dir_all(&out_dir);
        let mut c = cfg_with(
            "color",
            &std::env::var("VTEDIT_COVER_PROBE_BG").unwrap_or_else(|_| "#20242c".to_string()),
            2.0,
        );
        c.cover.title_lines = if let Ok(t) = std::env::var("VTEDIT_COVER_PROBE_TEXT") {
            t.split('|').map(|s| s.to_string()).collect()
        } else {
            vec![
                "幼儿园印象".into(),
                "小红马和跷跷板".into(),
                "玩上头".into(),
                "直接飞出去流鼻血".into(),
            ]
        };
        if let Ok(fs) = std::env::var("VTEDIT_COVER_PROBE_FS") {
            c.cover.font_size = fs.parse().unwrap_or(120);
        }
        c.cover.stroke = std::env::var("VTEDIT_COVER_PROBE_SK")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(20);
        c.cover.text_align = "left".into();
        if std::env::var("VTEDIT_COVER_PROBE_NOSHADOW").is_ok() {
            c.cover.text_shadow = false;
        }
        c.cover.highlight_keywords = vec!["跷跷板".into()];
        c.cover.effects = vec![crate::project::CoverEffect {
            id: "fx1".into(),
            kind: "emoji".into(),
            amount: 0.35,
            x: 78.0,
            y: 72.0,
            color: "#000000".into(),
            text: "🩸".into(),
        }];
        // 真实工程模式：直接吃用户的 polish.toml（字体/标题/描边/图层全是真值）
        if let Ok(dir) = std::env::var("VTEDIT_COVER_PROBE_PROJECT") {
            let dir = std::path::PathBuf::from(dir);
            let mut cfg = crate::project::load_project(&dir).expect("读工程失败");
            // 探针可覆盖描边宽度（量"描边滑块 → 环宽"这条链）
            if let Ok(sk) = std::env::var("VTEDIT_COVER_PROBE_SK") {
                cfg.cover.stroke = sk.parse().unwrap_or(cfg.cover.stroke);
            }
            let title_lines = cfg.cover.title_lines.clone();
            let bg = dir.join(COVER_BG_IMG).is_file();
            let html = build_cover_page_static(&cfg, bg, &title_lines);
            let css = crate::render::takumi::extract_style_blocks(&html);
            let fonts = cover_font_files();
            eprintln!("探针：真实工程 {} 字体 {} 支", dir.display(), fonts.len());
            let _ = std::fs::write(out_dir.join("cover.html"), &html);
            let out = out_dir.join("takumi.jpg");
            crate::render::takumi::render_cover_png(&html, &css, &dir, &out, 1920, 1080, &fonts)
                .expect("真实工程探针渲染失败");
            eprintln!("探针：产物 = {}；HTML 已写到 cover.html", out.display());
            return;
        }
        let title_lines = c.cover.title_lines.clone();
        let html = build_cover_page_static(&c, false, &title_lines);
        let css = crate::render::takumi::extract_style_blocks(&html);
        let css = match std::env::var("VTEDIT_COVER_PROBE_CSS") {
            Ok(extra) => format!("{css}\n{extra}"),
            Err(_) => css,
        };
        // 字体注册序与生产完全一致（defaults 先、emoji 最后）—— 本机是平铺布局那两支。
        // ⚠️ 兜底也走 `font_dir_owned()`（= 按 `YEKITRIM_OWNER` 选库）：探针在**子进程**里跑，
        // 与出片时看到的是同一份库；写死全局 `path::font_dir()` 会让多用户下探的库与真出的片对不上。
        let root = std::path::PathBuf::from(
            std::env::var("VTEDIT_COVER_PROBE_FONTS")
                .unwrap_or_else(|_| font_dir_owned().display().to_string()),
        );
        let fonts = cover_font_files_in(&root);
        eprintln!("探针：字体注册序 = {:?}", fonts);
        eprintln!("探针：HTML {}B / CSS {}B", html.len(), css.len());
        let out = out_dir.join("probe-cover.jpg");
        crate::render::takumi::render_cover_png(&html, &css, &out_dir, &out, 1920, 1080, &fonts)
            .expect("探针渲染失败");
        eprintln!("探针：产物 = {}", out.display());
    }

    /// emoji 特效：字符进模板、x/y 是字符中心、amount 映射字号；
    /// **text 是用户输入，必须转义**（否则就是往封面 HTML 里注模板）。
    #[test]
    fn cover_emoji_fx_renders_text_and_escapes_html() {
        fn fx(amount: f64, x: f64, y: f64, text: &str) -> crate::project::CoverEffect {
            crate::project::CoverEffect {
                id: String::new(),
                kind: "emoji".into(),
                amount,
                x,
                y,
                color: "#000000".into(),
                text: text.into(),
            }
        }
        let mut c = cfg_with("color", "#000000", 1.0);
        c.cover.effects = vec![
            fx(0.5, 83.3, 50.0, "😆"),
            fx(1.0, 10.0, 20.0, "😆"),
            fx(0.0, 1.0, 2.0, "😆"),
            fx(0.3, 5.0, 6.0, "<b>粗</b>"),
            fx(0.3, 7.0, 8.0, "   "),
            fx(0.3, 9.0, 9.0, "😆😆😆😆😆😆😆😆😆😆😆😆😆😆😆"),
        ];
        let html = cover_fx_html(&c.cover, &plan_cover_layers(&c.cover), false);
        // 位置：x/y 是**字符中心**（translate(-50%,-50%)），缩放/字体栈都要在
        assert!(
            html.contains("left:83.3%;top:50.0%;transform:translate(-50%,-50%)"),
            "emoji 定位不对：{html}"
        );
        assert!(html.contains("Segoe UI Emoji"), "emoji 字体栈没写：{html}");
        // amount → 字号：0.5 → 168px，1 → 288px，0 → 48px
        assert!(html.contains("font-size:168px"), "{html}");
        assert!(html.contains("font-size:288px"), "{html}");
        assert!(html.contains("font-size:48px"), "{html}");
        assert!(html.contains(">😆<"), "表情字符没进模板：{html}");
        // 转义：<b> 必须变成实体，原样的 <b> 不能出现
        assert!(html.contains("&lt;b&gt;粗&lt;/b&gt;"), "{html}");
        assert!(!html.contains("<b>粗</b>"), "用户输入没转义：{html}");
        // 全空白 → 默认表情；超长截到 12 个字符
        assert!(html.contains(&format!(">{}<", COVER_EMOJI_DEFAULT)), "{html}");
        let long = "😆".repeat(COVER_EMOJI_MAX_CHARS);
        assert!(html.contains(&format!(">{long}<")), "长串没截断：{html}");
        assert!(!html.contains(&"😆".repeat(COVER_EMOJI_MAX_CHARS + 1)), "{html}");
    }

    /// 三种模式（color / blur / image）：未知值与老的 `frame` 都当纯色（默认黑）。
    #[test]
    fn unknown_bg_mode_falls_back_to_color() {
        assert_eq!(cover_bg_mode(&cfg_with("weird", "#000000", 1.0).cover), "color");
        assert_eq!(cover_bg_mode(&cfg_with("frame", "#000000", 1.0).cover), "color");
        assert_eq!(cover_bg_mode(&cfg_with("", "#000000", 1.0).cover), "color");
        assert_eq!(cover_bg_mode(&cfg_with("BLUR", "#000000", 1.0).cover), "blur");
        assert_eq!(cover_bg_mode(&cfg_with("IMAGE", "#000000", 1.0).cover), "image");
        // 缺省 = 纯色黑
        let c: crate::project::ProjectFile = toml::from_str("[source]
video=\"a.mp4\"
subtitles=\"a.srt\"
").unwrap();
        assert_eq!(c.cover.bg_mode, "color");
        assert_eq!(c.cover.bg_color, "#000000");
    }

    /// 背景层三种模式 + **背景图与关键帧是两个文件**（导入背景不能盖掉「关键帧层」）。
    #[test]
    fn cover_bg_image_mode_uses_its_own_file() {
        let dir = std::env::temp_dir().join(format!("yekitrim-cover-bg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // 没有 _cover_bg.jpg 时：image 模式退回纯色，免得模板里挂个死图
        let html = build_cover_html(&dir, &cfg_with("image", "#123456", 1.0), &resolved());
        assert!(html.contains("class=\"bg-color\""), "缺背景图时应退回纯色：{html}");

        // 有 _cover_bg.jpg：走 image 模式，背景读它、关键帧读 _cover_frame.jpg
        std::fs::write(dir.join(COVER_BG_IMG), b"x").unwrap();
        let html = build_cover_html(&dir, &cfg_with("image", "#123456", 1.0), &resolved());
        assert!(html.contains("class=\"bg-image\""), "{html}");
        assert!(
            html.contains(&format!("<div id=\"bg\" class=\"bg-image\"><img src=\"{COVER_BG_IMG}\">")),
            "背景层该读自己的图：{html}"
        );
        assert!(
            html.contains(&format!("<div id=\"frame\"><img src=\"{COVER_FRAME_IMG}\">")),
            "关键帧层必须还是 _cover_frame.jpg：{html}"
        );

        // blur 模式读的仍是关键帧
        let html = build_cover_html(&dir, &cfg_with("blur", "#123456", 1.0), &resolved());
        assert!(html.contains("class=\"bg-blur\""), "{html}");
        assert!(html.contains(&format!("<img src=\"{COVER_FRAME_IMG}\">")), "{html}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 视觉验收用：把三种模式的真实 HTML 落到 `target/tmp-cover/`，
    /// 配一张 `_cover_frame.jpg` 就能用浏览器 1:1 看效果（预览就是这份 HTML）。
    /// 跑法：`cargo test -p yekitrim-polish --lib dump_cover_variants -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dump_cover_variants_for_visual_check() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp-cover");
        std::fs::create_dir_all(&out).unwrap();
        for (mode, scale) in [("color", 1.0), ("blur", 0.5), ("blur", 1.0), ("blur", 3.0)] {
            let mut c = cfg_with(mode, "#171320", scale);
            c.cover.title_lines = vec!["测试标题第一行".into(), "第二行带高亮词".into()];
            c.cover.gradient_dir = "left".into();
            let html = build_cover_html(&out, &c, &resolved());
            let name = format!("{mode}-x{scale}.html");
            std::fs::write(out.join(&name), html).unwrap();
            println!("wrote {}", out.join(&name).display());
        }
        // 表情特效变体（贴主播位置：layout.liver 默认 mid-right = 83.34% 50%）
        let mut c = cfg_with("blur", "#171320", 1.0);
        c.cover.title_lines = vec!["测试标题第一行".into()];
        c.cover.effects = vec![
            crate::project::CoverEffect {
                id: "e1".into(),
                kind: "emoji".into(),
                amount: 0.5,
                x: 83.3,
                y: 50.0,
                color: "#000000".into(),
                text: "😆".into(),
            },
            crate::project::CoverEffect {
                id: "e2".into(),
                kind: "emoji".into(),
                amount: 0.2,
                x: 20.0,
                y: 25.0,
                color: "#000000".into(),
                text: "❤️".into(),
            },
        ];
        let html = build_cover_html(&out, &c, &resolved());
        std::fs::write(out.join("emoji.html"), html).unwrap();
        println!("wrote {}", out.join("emoji.html").display());
    }
}