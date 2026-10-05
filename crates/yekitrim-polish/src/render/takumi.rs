//! **静态封面渲染（Takumi 底座）** —— 不跑浏览器、不跑 JS、不推进时间。
//!
//! # 它解决什么
//!
//! 现在出封面要起一个 headless Chrome：实测一张封面 **3.18s / 11 个进程 / 峰值 520MB**
//! （其中 1.5s 还是模板自己 `setTimeout(1500)` 的白等）。Takumi 是 MIT/Apache 的 Rust 光栅化器，
//! 吃 HTML+CSS 直接出图：实测同一张真封面 **render 250ms / 单进程 / 峰值 54MB**，
//! 与 Chrome 的像素差 **3.03/255**（残差只在字形边缘 —— 两个栅格化器的固有差异）。
//!
//! # 为什么是"薄桥"
//!
//! 这一层**只做三件事**：调 `from_html` + `StyleSheet::parse` + `render`；
//! **不碰**模板拼接（归 `cover.rs::build_cover_page_static`）、**不碰**配置（归 `project`）、
//! **不碰**产物命名（归 `render::unique_output_path`）。要加引擎差异也**只加在这里**。
//!
//! # 三条实测踩出来的规矩
//!
//! 1. **CSS 必须由调用方给**（`build_cover_page_static` 回传，或本模块的
//!    [`extract_style_blocks`]）—— Takumi 的 HTML 前端 **`from_html` 会丢掉 `<style>` 块**，
//!    它只认 `style=` 内联属性；而引擎本身有完整的样式表（`StyleSheet::parse` + `RenderOptions`）。
//! 2. **`paint-order` 必须删**：写了它描边会盖住填充（8px 描边 / 56px 字 → 一片黑）。
//!    实测删掉后 18.24/255 → **0.47/255**。
//! 3. **`repeating-linear-gradient` 的角度语义与 CSS 相反**（`0deg` ≡ Chrome 的 `180deg`）。
//!    实测 30.50/255 → **0.04/255**。封面"扫描线"特效就是它。
//!
//! # 字体：**必须显式给**
//!
//! Takumi **不读系统字体**（内置只有 Geist，拉丁）。中文/emoji 都要注册字体文件，
//! 否则出的是豆腐块 —— 所以这条路要求"字体库里有字体"，一个都没有时**明确报错**，不静默降级。
#![cfg(feature = "cover-takumi")]

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::Engine as _;
// 入口函数在 crate 根，**不在 prelude 里**（prelude 只有数据类型）
use takumi::prelude::*;
// `FontOverride.family_name`：把一支字体**按我们指定的族名**注册（而不是它内嵌的名字）。
// 这是让模板里的 `font-family:"<字体库 id>"` 能命中 Takumi 的唯一手段。
use std::sync::Arc;
use takumi::{from_html, render, write_image};
use std::hash::{Hash, Hasher};

/// 短 hash（非加密，只为"输入/输出变没变"一眼可见）。
fn short_hash(bytes: &[u8]) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{:08x}", h.finish() as u32)
}

/// 从封面 HTML 里按**文档序**取出所有 `<style>…</style>` 的内容。
///
/// ⚠️ **先剥 HTML 注释**：模板里那句「单独一个 `<style>` 放在主样式之后…」的注释**提到了 `<style>`**，
/// 不剥注释就会把注释文字当成 CSS → Takumi 报 `StyleSheetParseError { kind: DanglingCombinator }`
/// （实测踩到过）。这条有单测钉着（`extract_ignores_the_comment_trap`）。
///
/// ⚠️ 优先用 `cover.rs::build_cover_page_static` 回传的 CSS；这个函数是给"手上只有 HTML"的
/// 场景兜底的（例如已经落盘的预览页）。
pub fn extract_style_blocks(html: &str) -> String {
    let mut cleaned = String::with_capacity(html.len());
    let mut parts = html.split("<!--");
    if let Some(first) = parts.next() {
        cleaned.push_str(first);
    }
    for p in parts {
        // 没闭合的注释：整段丢掉（宁可少 CSS，也不要把注释当样式表喂进去）
        if let Some(k) = p.find("-->") {
            cleaned.push_str(&p[k + 3..]);
        }
    }

    let mut out = String::new();
    let mut rest = cleaned.as_str();
    while let Some(i) = rest.find("<style") {
        let Some(gt) = rest[i..].find('>') else { break };
        let body = &rest[i + gt + 1..];
        let Some(end) = body.find("</style>") else { break };
        out.push_str(&body[..end]);
        out.push('\n');
        rest = &body[end + "</style>".len()..];
    }
    out
}

/// 只改**喂给 Takumi 的那一份** CSS，修三处引擎差异（见模块头 2、3 条 + 这条）：
///
/// 4. **删掉 `@font-face` 整块**：项目的 `[cover] font` 选了库内字体时，`cover.rs` 会往模板里注入
///    `@font-face`；而 Takumi 的样式表解析器**不认这个 at-rule**（实测
///    `AtRuleInvalid("font-face")` 直接让整张封面失败）。这条路本来也不靠 CSS 声明字体 ——
///    字体是 [`render_cover_png`] 用 `Fonts::register` 注册进去的。
///
/// ⚠️ 千万别把这几条改进模板：模板那份是 Chrome 的输入，
/// 改了 `paint-order` 会**改掉 Chrome 自己的出图**（存量零回归 SHA 立刻变）。
pub fn adapt_css_for_takumi(css: &str) -> String {
    let css = strip_at_rule_blocks(css, "@font-face");
    // 标定开关：Takumi 的 `filter:blur(Npx)` 与 CSS 的 blur 口径（σ vs 半径）可能不同 ——
    // 在**真照片**上量得出来（铺满全图的小幅差），在纯色测试图案上量不出来。
    // `VTEDIT_TAKUMI_BLUR_SCALE=0.5` 把所有 blur(N) 换成 blur(N×0.5)。
    let blur_scale: f64 = std::env::var("VTEDIT_TAKUMI_BLUR_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let css = if (blur_scale - 1.0).abs() > f64::EPSILON {
        scale_blur(&css, blur_scale)
    } else {
        css
    };
    let mut out = String::with_capacity(css.len());
    for line in css.lines() {
        // `paint-order:stroke fill;` → 整条删掉（Takumi 里有它就会把描边画在填充之上）
        // ⚠️ 注意：Takumi **完全忽略 `paint-order`**，所以"删掉它"并不能让画序变成 CSS 的默认
        //（`normal` = 描边在上）—— 它永远描边在上。粗描边（本工程 20px/120px）会把字形糊死。
        // 这里的描边剥离是**诊断开关**：用来量"问题全在描边上"这个判断的上限。
        let stripped = strip_declaration(line, "paint-order");
        let stripped = if std::env::var("VTEDIT_TAKUMI_NO_STROKE").is_ok() {
            strip_declaration(&stripped, "-webkit-text-stroke")
        } else {
            stripped
        };
        // 扫描线：Takumi 的 0deg 与 CSS 相反
        // ⚠️ Takumi 的**无前缀** `text-stroke` 是"填充在上"（实验台 10e：观感正常），
        // 而 `-webkit-text-stroke` 会把描边画在填充之上（10_stroke：糊成黑块）⇒ 就地换写法，别做双层。
        out.push_str(&stripped.replace("repeating-linear-gradient(0deg", "repeating-linear-gradient(180deg"));
        out.push('\n');
    }
    out
}

/// 把 `blur(Npx)` 里的 N 乘一个系数（标定模糊口径用；见 [`adapt_css_for_takumi`]）。
/// 把 `var(--x)` **就地换成值**（只认 CSS 里定义过的那些，含 `var(--x, fallback)`）。
///
/// **为什么必须**：实测两个真实的失效点 —— `#frame img{transform:var(--bg-transform)}` 在 Takumi 里
/// 不生效（帧不缩放 ⇒ 上下露黑边），`filter:blur(var(--bg-blur))` 同理。变量在 `:root{}` 里是有定义的，
/// 所以这是**取值解析**的差异，不是"没有定义"。
pub fn inline_css_vars(css: &str) -> String {
    let Ok(def_re) = regex_lite::Regex::new(r"--([A-Za-z0-9-]+)\s*:\s*([^;}]+)") else { return css.to_string() };
    let Ok(use_re) = regex_lite::Regex::new(r"var\(\s*--([A-Za-z0-9-]+)\s*(?:,\s*([^)]*))?\)") else { return css.to_string() };
    let mut vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for cap in def_re.captures_iter(css) {
        vars.insert(cap[1].to_string(), cap[2].trim().to_string());
    }
    if vars.is_empty() { return css.to_string(); }
    let mut out = css.to_string();
    // 两轮：值里还可能引用别的变量（如 `--grad-bg` 引用 `--bg-color`）
    for _ in 0..2 {
        let mut next = String::with_capacity(out.len());
        let mut last = 0;
        for cap in use_re.captures_iter(&out) {
            let whole = cap.get(0).expect("group 0");
            next.push_str(&out[last..whole.start()]);
            let name = cap[1].to_string();
            match vars.get(&name) {
                Some(v) => next.push_str(v),
                None => next.push_str(cap.get(2).map(|m| m.as_str()).unwrap_or("")),
            }
            last = whole.end();
        }
        next.push_str(&out[last..]);
        if next == out { break; }
        out = next;
    }
    out
}

/// 给 `font-family` 里的通用族（`sans-serif` 等）**前面补上 emoji 家族**。
/// 把 `#frame img` 上的 `transform` / `transform-origin` **搬到父 `#frame`**，并显式去掉 img 上那两条。
///
/// **为什么必须**：实测 Takumi **不认 `<img>` 上的 `transform`** —— 关键帧层不缩放/不位移，画面缩在中间、
/// 上下露黑边（真工程实测整图差 35/255）；同一个 transform 作用在**普通 div** 上是生效的
///（实验台 case 17：`#w{transform:translate(-50%,-50%)}` 定位正确）。
pub fn hoist_frame_transform(css: &str) -> String {
    let Ok(rule_re) = regex_lite::Regex::new(r"#frame img\s*\{[^}]*\}") else { return css.to_string() };
    let Some(rule) = rule_re.find(css) else { return css.to_string() };
    let body = rule.as_str();
    let grab = |key: &str| -> Option<String> {
        regex_lite::Regex::new(&format!(r"{key}\s*:\s*([^;}}]+)"))
            .ok()
            .and_then(|re| re.captures(body))
            .map(|c| c[1].trim().to_string())
    };
    let (Some(t), Some(o)) = (grab("transform"), grab("transform-origin")) else { return css.to_string() };
    let stripped = match regex_lite::Regex::new(r"transform(-origin)?\s*:\s*[^;}}]+;?") {
        Ok(re) => re.replace_all(body, "").to_string(),
        Err(_) => body.to_string(),
    };
    let mut out = css.replacen(body, &stripped, 1);
    out.push_str(&format!("\n#frame{{transform:{t};transform-origin:{o};}}\n#frame img{{transform:none;}}"));
    out
}

///
/// **为什么必须**：封面的字体栈是 `'Noto Sans SC','Microsoft YaHei','Noto Sans SC',sans-serif` ——
/// 浏览器会退回**系统** emoji 字体，而 Takumi 只在**注册过**的字体里找 ⇒ 标题里的 🩸 变成黑块。
///
/// ⚠️ **三个修正**（2026-10-02，输入层收敛 + 职责分离）：
///
/// ① **族名来自唯一权威**（`cover::emoji_font_stack`），不再就地写死。原来这里与
///    `cover.rs::EMOJI_FONT` 各写一份，靠注释互相提醒"要同源"——漂移的症状是
///    "设计器里有 emoji、出片里没有"，**不报错**。
///
/// ② **幂等**（修一个既有 bug）。原来的正则**无条件**往通用族前插前缀，
///    对已经带 emoji 的栈会插第二遍：
///
///    `'Noto Color Emoji','Segoe UI Emoji','Noto Color Emoji',sans-serif` ← 多一段
///
///    当时**没暴露**纯属侥幸：封面 emoji 特效层是 **inline style**，而本函数的调用点
///    只扫 `<style>` 块（`extract_style_blocks`，见 `adapt_css_for_takumi`），所以那层根本走不到这里。
///    但一旦有人改了调用点（或模板里用了 `<style>` 写 emoji 层），重复就生效了。
///    现在带 "已在其中就不插" 的判据。
///
/// ③ 插入的族名以**库里那支**为主（没有才用系统名）—— 与 `emoji_font_stack()` 同源。
pub fn ensure_emoji_fallback(css: &str) -> String {
    let stack = crate::cover::emoji_font_stack();
    // 权威栈形如 `'族名','Segoe UI Emoji',...,sans-serif`；这里要的是**通用族之前**那一段。
    let Some(inject) = stack.rsplit_once(',').map(|(head, _)| head) else {
        return css.to_string();
    };
    let Ok(re) = regex_lite::Regex::new(r"(font-family\s*:[^;}]*?)(sans-serif|serif|monospace)") else {
        return css.to_string();
    };
    re.replace_all(css, |c: &regex_lite::Captures| {
        let prefix = c.get(1).map(|m| m.as_str()).unwrap_or("");
        let generic = c.get(2).map(|m| m.as_str()).unwrap_or("sans-serif");
        // ⚠️ 幂等判据：这一段 font-family 里**已经出现 emoji 族名**就不插。
        // 探针用子串 `emoji`（覆盖 `'Noto Color Emoji'` / `'Segoe UI Emoji'` / `'Apple Color Emoji'`）。
        if prefix.to_ascii_lowercase().contains("emoji") {
            return format!("{prefix}{generic}");
        }
        format!("{prefix}{inject},{generic}")
    })
    .to_string()
}

fn scale_blur(css: &str, factor: f64) -> String {
    let Ok(re) = regex_lite::Regex::new(r"blur\((\d+(?:\.\d+)?)px\)") else { return css.to_string() };
    let mut out = String::with_capacity(css.len());
    let mut last = 0;
    for cap in re.captures_iter(css) {
        let whole = cap.get(0).expect("group 0");
        let n: f64 = cap[1].parse().unwrap_or(0.0);
        out.push_str(&css[last..whole.start()]);
        out.push_str(&format!("blur({:.2}px)", n * factor));
        last = whole.end();
    }
    out.push_str(&css[last..]);
    out
}

/// 与 [`split_title_layers`] 配套的三条规则。
///
/// ⚠️ **全部写成"类 + 后代"形式**（`.vt-outer #title`），**不用 `#title.vt-fill` 这种复合选择器** ——
/// 实测 Takumi 不匹配 `#id.class`（`selector-probe`：Chrome A−B = 104.11 / Takumi 2.01）。
/// ⚠️ 最后一条 `#title{font-weight:400}` 是 **2026-10-02 的实测修正**（用户报「中文描边不够准确」）：
/// 模板给标题写的是 `font-weight:900`，而 **Chrome 对库里那支单字面字体不做合成加粗**
/// （受控实测：同一字体同一字号，Chrome 的 400 与 900 **逐像素相同**，填充 bbox 154×180、
/// 描边外扩 9px）；**Takumi 会合成加粗**（`synthesis_embolden_strength` = font_size/32，
/// 走 `draw_text_embolden` 的 `Stroke::new(e)`）⇒ 填充向外撑 `e/2`（200px 字实测 166×192 →
/// 172×198），而描边层是按**未加粗**的原始轮廓描的 ⇒ 填充盖掉描边内缘，**可见描边从 10px 掉到 7px**。
/// 两处一起失真就是"描边不准确 + 字比预览粗"。库里每支字体只有一个字面（`font.ttf`），
/// Takumi 永远拿不到真粗体 ⇒ 合成加粗必然是**两边不一致**的来源，所以在这一层明确关掉。
/// 同特异度按文档序覆盖：这条排在模板那条 `#title{…font-weight:900…}` 之后 ⇒ 生效。
const TAKUMI_TITLE_LAYER_CSS: &str = "\
.vt-outer{z-index:2}\
.vt-inner-fill{position:absolute;left:0;top:0;width:100%}\
.vt-inner-fill #title{-webkit-text-stroke:0}\
.vt-outer #title{color:transparent}\
.vt-outer .hl{color:transparent}\
\
#title{font-weight:400}";

/// 让"描边在下、填充在上"在 Takumi 里成立：把 `#title-wrap` **整块复制一份放在它前面**当描边层。
///
/// **为什么不能靠 `paint-order`**：Takumi 完全忽略它，永远把描边画在填充之上
/// （模板给 Chrome 写的是 `paint-order:stroke fill`，即填充在上）。20px 描边 / 120px 字时，
/// 描边会吃掉整个字腔 —— 真工程上标题糊成黑块（区域分解：非标题区 ≤1.6/255，标题那两行 43~49/255）。
///
/// **为什么复制整块 wrap 而不是复制里面的 `#title`**：`#title-wrap` 带
/// `transform:translate(-50%,-50%)` + `width:var(--tw)` + `display:flex`，两层各自的盒心由它决定；
/// 复制 wrap 则两层的几何**天然完全相同**（同一套 CSS、同一个 `left/top/transform`）。
/// 两个绝对定位元素同 z-index 时按 DOM 序绘制，所以描边层放在前面就在下面。
///
/// 找不到那块就原样返回（宁可标题观感差，也不要动坏 HTML）。
pub fn split_title_layers(html: &str, stroke: &str, css: &str) -> String {
    // 目的：等价 Chrome 的 `paint-order:stroke fill`（填充在上）—— Takumi 忽略 paint-order，
    // 只能画两层：描边层（透明填充 + 描边）在下，填充层（纯填充）在上。
    // ⚠️ 两层的**盒子必须完全一致**（实测：一层 flex 子项 max-content、另一层 width:100% 时，
    // 两层错位/放大，整图差从 21 掉到 25）。所以两层都 absolute + 同 left/top/width，
    // 并由 wrap 的**内联 height**（= 行数 × --fs × --lh）兜住 translate(-50%,-50%) 的居中。
    let Ok(re) = regex_lite::Regex::new(r#"<div id="title-wrap">([\s\S]*?</div>)</div>"#) else {
        return html.to_string();
    };
    let Some(cap) = re.captures(html) else { return html.to_string() };
    let whole = cap.get(0).expect("group 0").as_str();
    let inner = &cap[1];
    let num = |pat: &str, dflt: f64| -> f64 {
        regex_lite::Regex::new(pat)
            .ok()
            .and_then(|r| r.captures(css))
            .and_then(|c| c[1].parse().ok())
            .unwrap_or(dflt)
    };
    let fs = num(r"--fs\s*:\s*([\d.]+)px", 120.0);
    let lh = num(r"--lh\s*:\s*([\d.]+)", 1.7);
    // ⚠️ **结尾那个 `<br>` 不算一行**：模板的 title_html 每行后都跟 `<br>`（末尾也有），
    // 而 Chrome 的块高只算**可见行**（实测：按 5 行算高度 → 多 204px → translate(-50%,-50%) 整体下移 ~102px）。
    let text_end = inner.rfind("</div>").unwrap_or(inner.len());
    let mut body = inner[..text_end].trim_end();
    if let Some(t) = body.strip_suffix("<br>") {
        body = t.trim_end();
    }
    let lines = regex_lite::Regex::new(r"<br\s*/?>")
        .map(|r| r.find_iter(body).count())
        .unwrap_or(0) + 1;
    let h = (fs * lh * lines as f64).round();
    let box_style = "position:absolute;left:0;top:0;width:100%;";
    // ⚠️ 描边默认 **Miter join**（Takumi `Stroke::new` 默认 + CJK 尖笔画放大地）⇒ 20px 描边在
    // "幼/马/跷"这类字上出尖刺。Takumi 认 `stroke-linejoin`（stylesheets.rs:1275 →
    // `draw_text_stroke` 的 `stroke.join`），但模板从没写过它 ⇒ 这里**逐层内联补** `round`。
    // 只补**描边层**：填充层没有描边，写了也是死代码（还误导读者）。
    let stroke_inline = if stroke.is_empty() {
        stroke.to_string()
    } else {
        format!("{stroke};stroke-linejoin:round")
    };
    let inner_stroke = inner.replacen(
        "<div id=\"title\"",
        &format!("<div id=\"title\" style=\"{box_style}color:transparent;{stroke_inline}\""),
        1,
    );
    // ⚠️ `.hl` 高亮词**自己是 span**，带 `color:#ffcc00` —— 从 `#title` 继承的 `transparent` 压不过它，
    // 于是描边层里的高亮词还是"黄填充 + 描边"（实测：只有含 .hl 的词变肥）。所以**逐个内联**改透明。
    let mut hl_transparent = inner_stroke.clone();
    // Rust 的 `replace` 替换**全部**出现 ⇒ 一次调用即可（循环会加出重复 style 属性，实测 4 个）
    hl_transparent = hl_transparent.replace("<span class=\"hl\"", "<span class=\"hl\" style=\"color:transparent\"");
    let inner_fill = inner.replacen(
        "<div id=\"title\"",
        &format!("<div id=\"title\" style=\"{box_style}-webkit-text-stroke:0\""),
        1,
    );
    // ⚠️ **模板的 `.hl` 规则自带 `-webkit-text-stroke:var(--sk)`** —— 填充层的 `#title` 去了描边，
    // 但 `.hl` 是**子元素、自己有描边** ⇒ 高亮词依旧是"描边压填充"的实心黑块（看图实锤）。
    // 所以填充层的高亮词也要**逐个内联**去掉描边。
    let hl_fill = inner_fill.replace("<span class=\"hl\"", "<span class=\"hl\" style=\"-webkit-text-stroke:0\"");
    let block = format!(r#"<div id="title-wrap" style="height:{h}px">{hl_transparent}{hl_fill}</div>"#);
    html.replacen(whole, &block, 1)
}

/// 往 `#fx-extra` 开标签后插一个**只带权威 emoji 字族**的空 div。
///
/// **为什么必须**：Takumi 对 `font-family` 的解析是**全局命名空间**（`FontContext::families`），
/// 不存在"这一层用 A、那一层用 B"的局部作用域；而 cover 的 emoji 特效层是 **inline style**
/// （`ensure_emoji_fallback` 只扫 `<style>` 块，扫不到它）。字族的**唯一权威** =
/// `crate::cover::emoji_font_stack()`（库里那支 emoji 字体的内部族名打头）——把同一份栈
/// 以**空 div** 的形式放进 `#fx-extra`（所有特效的父容器），字体解析器就会先在这一层
/// 建立"族名 → 字体"的映射；emoji 特效层那份 inline 栈请求的是**同名族**，于是两端
/// （设计器预览注册的 FontFace / 出片注册的字节）收敛到同一支字体。
///
/// 空盒（`font-size:0`）**零布局影响**：所有特效都是 absolute，DOM 序里它又排在最前，
/// 画的时候永远在其他特效之下。带 `data-vt-emoji-font` 标记 ⇒ 幂等（重复调用不再插）。
pub fn inject_emoji_layer_font(html: &str) -> String {
    const MARKER: &str = "data-vt-emoji-font";
    // emoji 特效层的 inline style 独有签名（glow/blur/… 都是 background/backdrop-filter，没有 font-family）
    if !html.contains(";line-height:1;font-family:") || html.contains(MARKER) {
        return html.to_string();
    }
    let open = "<div id=\"fx-extra\"";
    let Some(start) = html.find(open) else { return html.to_string() };
    let Some(gt) = html[start..].find('>') else { return html.to_string() };
    let at = start + gt + 1;
    let stack = crate::cover::emoji_font_stack();
    let layer = format!(
        "<div style=\"font-family:{stack};font-size:0;line-height:1;\" {MARKER}></div>"
    );
    format!("{}{}{}", &html[..at], layer, &html[at..])
}

/// 把 **复合选择器** `#id.class` 扁平化成 `#id`（仅当那份 HTML 里该元素确实带着这个 class）。
///
/// **为什么必须**：实测 Takumi 2.14 **不匹配 `#a.k img`**（最小单点：Chrome A−B = 104.11、
/// Takumi A−B = 2.01）。而真封面的背景层可见性**正是靠这条**：
///
/// ```css
/// #bg img { display:none }
/// #bg.bg-blur img  { display:block; filter:blur(var(--bg-blur)); transform:scale(var(--bg-blur-scale)) }
/// #bg.bg-image img { display:block; … }
/// ```
///
/// ⇒ 复合选择器不匹配时，`display:none` 一直生效、**背景层整块不画**（`bg_mode=blur|image` 全中，
/// 真工程上这一条值 ~15/255）。
///
/// 扁平化是安全的：基规则在前、被扁平的规则在后，同特异度时按文档序覆盖，结果等价。
/// 只在"HTML 里真有这个 class"时替换 —— 没出现的 class 一律不碰（免得把 `#x.only-js` 这类
/// 只给别的通道用的规则也拉进来）。
pub fn flatten_compound_selectors(css: &str, html: &str) -> String {
    let Ok(re_id_first) = regex_lite::Regex::new(r#"id="([A-Za-z0-9_-]+)"[^>]*?class="([^"]*)""#) else {
        return css.to_string();
    };
    let Ok(re_class_first) = regex_lite::Regex::new(r#"class="([^"]*)"[^>]*?id="([A-Za-z0-9_-]+)""#) else {
        return css.to_string();
    };
    let mut pairs: Vec<(String, String)> = Vec::new();
    for c in re_id_first.captures_iter(html) {
        for cls in c[2].split_whitespace() {
            pairs.push((c[1].to_string(), cls.to_string()));
        }
    }
    for c in re_class_first.captures_iter(html) {
        for cls in c[1].split_whitespace() {
            pairs.push((c[2].to_string(), cls.to_string()));
        }
    }
    let mut out = css.to_string();
    for (id, cls) in pairs {
        let from = format!("#{id}.{cls}");
        if out.contains(&from) {
            out = out.replace(&from, &format!("#{id}"));
        }
    }
    out
}

/// 删掉某个 at-rule 的**整块**（`@name … { … }`）。字体声明块里没有嵌套花括号，所以
/// "从 `@name` 找到第一个 `{`、再找到下一个 `}`" 就够；认不出来（缺花括号）时**原样留着** ——
/// 宁可让 Takumi 报错，也不要静默吞掉内容。
fn strip_at_rule_blocks(css: &str, name: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = find_outside_comments(rest, name) {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        match (after.find('{'), after.find('}')) {
            (Some(open), Some(close)) if close > open => rest = &after[close + 1..],
            _ => {
                out.push_str(after);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 在**不在注释里**的位置找 `name`（CSS 注释 = `/* … */`）。
///
/// ⚠️ **2026-10-05 用户报的 bug 就是漏了这一条**（「封面设计器里选的字体和出图不一致」）：
/// 模板里那句注释写着「…「该字体的 `@font-face` 名 + 原栈」…」，而 `strip_at_rule_blocks`
/// 从**注释里的** `@font-face` 开始删、删到**第一个 `}`** —— 那正好是紧随其后的
/// `html,body{… font-family:… }` 的右括号 ⇒ **整条规则（含字体栈）被吃掉**。
/// 表现：CSS 文本每次都变（注释里引用了族名），产物却**逐字节相同** —— 三次不同
/// `[cover] font`（含一个不存在的字体）出图完全一致，因为 Takumi 从来没收到过 `font-family`，
/// 落到"注册序第一支"。
fn find_outside_comments(hay: &str, name: &str) -> Option<usize> {
    let bytes = hay.as_bytes();
    let needle = name.as_bytes();
    if needle.is_empty() {
        return None;
    }
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // 跳过整段注释（没闭合就到底 —— 与 `extract_style_blocks` 同一条口径）
            match hay[i + 2..].find("*/") {
                Some(k) => {
                    i += 2 + k + 2;
                    continue;
                }
                None => return None,
            }
        }
        if bytes[i..].starts_with(needle) {
            return Some(i);
        }
        i += 1;
    }
    if bytes[i..].starts_with(needle) {
        return Some(i);
    }
    None
}

/// 去掉某一条声明（`name: value;`），同一行里其它声明原样保留。
///
/// ⚠️ 按 **`;` / `}` / `{`** 三类边界切（不止 `;`）：适配层生成的内联 style 常把多条声明挤在一行里
/// —— `split_title_layers` 发出的描边层就是 `style="position:…;color:transparent;-webkit-text-stroke:20px #2a2522"`，
/// 行尾那条声明**不带分号**（到 `}` 或行尾为止）。只按 `;` 切会把它整条漏掉不删，
/// `-webkit-text-stroke` 残留 ⇒ Takumi 把描边画在填充之上，透明层只剩描边、填充层黑块。
/// （`{` 也算边界：`-webkit-text-stroke:20px #2a2522{…}` 这种坏输入宁可整体跳过，也别把选择器当声明留下。）
fn strip_declaration(css: &str, name: &str) -> String {
    if !css.contains(name) {
        return css.to_string();
    }
    let mut out = String::with_capacity(css.len());
    for piece in css.split_inclusive(|c| c == ';' || c == '}' || c == '{') {
        if piece.trim_start().starts_with(name) {
            continue;
        }
        out.push_str(piece);
    }
    out
}

/// 把 HTML 里的**相对路径图片**（`src="xxx.jpg"`）内联成 data URI。
///
/// **为什么必须**：Takumi 没有资源解析器（既不读文件也不发请求），`src="_cover_frame.jpg"` 这种
/// 相对路径它一张都拿不到 —— 实测真封面走完整条产品链路时：图片层整块缺失、平均差 **146.44/255**。
/// （它的 `RenderOptions.images` 是 `pub(crate)`，外部塞不进去，所以只能内联。）
///
/// `data:` / `http(s):` / `/` 开头的一律不碰（那些要么已经内联、要么由调用方保证可达）。
pub fn inline_local_images(html: &str, asset_root: &Path) -> String {
    let re = match regex_lite::Regex::new(r#"src="([^"]+)""#) {
        Ok(re) => re,
        Err(_) => return html.to_string(),
    };
    let mut out = html.to_string();
    for cap in re.captures_iter(html) {
        let Some(m) = cap.get(1) else { continue };
        let src = m.as_str();
        if src.starts_with("data:") || src.starts_with("http") || src.starts_with('/') || src.is_empty() {
            continue;
        }
        let path = asset_root.join(src);
        let Ok(bytes) = fs::read(&path) else { continue }; // 读不到就原样留着，让引擎自己报/自己空
        let mime = match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
            Some("png") => "image/png",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            _ => "image/jpeg",
        };
        let uri = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes));
        out = out.replace(&format!("src=\"{src}\""), &format!("src=\"{uri}\""));
    }
    out
}

/// 把已读进内存的字体字节交给 Takumi —— **共享、不复制**。
///
/// `FontResource::new` 收 `impl Into<FontSource>`，而 `FontSource::from_shared` 接
/// `Arc<dyn AsRef<[u8]> + Send + Sync>`：同一支几十 MB 的字体要注册两个族名（内嵌名 + 库 id），
/// 每支 `fs::read` 一次、`Arc` 克隆两次指针 —— 不然峰值内存直接翻倍。
fn shared_font_bytes(bytes: std::sync::Arc<Vec<u8>>) -> FontSource<'static> {
    FontSource::from_shared(bytes)
}

/// 一支字体文件在 CSS 里**可能被请求到的库 id**（不含内嵌族名，那个 Takumi 自己就会注册）。
///
/// 模板与配置里存的是**库 id**，而 `scan_all` 的规则是：
/// **v2 目录布局**（`<slug>/font.ttf|otf`）⇒ id = **目录名**；
/// **老平铺布局**（`<id>.ttf` 直接躺在库里）⇒ id = **文件名去扩展名**。
/// 两种布局**互斥**（判据就是"文件名是不是约定的 `font.<ext>`"），所以这里也按同一条判据走，
/// 命中的那个才是别名；都算不出来（路径不在库里）就返回空 —— 只按内嵌族名注册，与改造前一致。
///
/// ⚠️ 这里**不能**无条件把父目录名也当别名：库里那些老平铺文件（`/fonts/X.ttf`）的父目录
/// 就叫 `fonts`，多注册一个叫 `fonts` 的族名纯属污染（测试逮到过）。
///
/// ⚠️ 为什么不是"直接问 `fonts::scan_all` 反查"：`cover_font_files()` 给的是**路径列表**，
/// 而 id 是"目录名或文件名去扩展名"这条**由路径即可算出**的规则；反查还要建映射，
/// 且库里同名文件/目录的存在与否不该影响"这支文件怎么注册"。
fn font_aliases(path: &Path) -> Vec<String> {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").trim();
    // v2 目录布局的判据：文件名就是约定的 `font.<ext>`
    if stem.eq_ignore_ascii_case("font") {
        let dir = path.parent();
        // ① **权威：同目录 `FONT.toml` 里的 `id`**（2026-10-02 起）。
        //    目录名现在是 ASCII slug（`font-<hash>`，因为 ffmpeg 打不开非 ASCII 字体路径），
        //    配置/模板里请求的那串 id（可能就是汉字）只住在元数据里 —— 不认它，
        //    封面就会静默落到“注册序第一支”（§13 P1-e 那次的症状原样复发）。
        let meta_id = dir
            .and_then(yekitrim_core::fonts::read_meta)
            .map(|m| m.id.trim().to_string())
            .filter(|id| !id.is_empty() && yekitrim_core::fonts::is_safe_id(id));
        if let Some(id) = meta_id {
            return vec![id];
        }
        // ② 老库 / 手建目录没有元数据（或没写 id）⇒ 沿用“id = 目录名”，改造前的行为一字不变
        if let Some(slug) = dir.and_then(|p| p.file_name()).and_then(|s| s.to_str()) {
            let slug = slug.trim();
            if !slug.is_empty() && yekitrim_core::fonts::is_safe_id(slug) {
                return vec![slug.to_string()];
            }
        }
        return Vec::new();
    }
    // 老平铺布局：id = 文件名去扩展名
    if !stem.is_empty() && yekitrim_core::fonts::is_safe_id(stem) {
        return vec![stem.to_string()];
    }
    Vec::new()
}

/// 渲染一张封面 PNG。`fonts` 是字体文件（**至少一个**，否则中文/emoji 是豆腐块）；
/// `asset_root` 是工程目录（相对路径图片从它下面读）。
pub fn render_cover_png(
    html: &str,
    css: &str,
    asset_root: &Path,
    out: &Path,
    width: u32,
    height: u32,
    fonts: &[PathBuf],
) -> Result<()> {
    if fonts.is_empty() {
        anyhow::bail!(
            "静态渲染（Takumi）需要至少一个字体文件：Takumi **不读系统字体**（内置只有拉丁的 Geist），\
             中文标题与 emoji 都会是豆腐块。请在 <程序目录>/resources/fonts/ 里放字体，\
             或在封面面板里选一个库内字体。"
        );
    }

    let mut font_ctx = Fonts::default();
    // 每支字体注册**两次**：
    //   ① 内嵌族名 —— Takumi 默认行为；
    //   ② **字体库 id（别名）** —— CSS 里存的是配置那串 id（`.cap{font-family:"<id>"}`、
    //      封面 `__FONT_FAMILY__`），与内嵌族名常常不同（id `LXGWWenKaiLite-Regular`
    //      vs 内嵌 `霞鹜文楷 轻便版`）。
    //
    // ⚠️ **族名是认的**（2026-10-02 受控实测，**纠正**先前写在这里的"由注册顺序决定、族名无用"）：
    // 注册 [乐米波波体, LXGW, NotoSansSC, NotoColorEmoji-CBDT] 后，某行请求
    // `font-family:'NotoColorEmoji-CBDT'` ⇒ 连**数字**都换成 emoji 字体那套字形（逐像素可辨），
    // 而请求正文族名时数字落在正文字体上。先前那条"三者输出逐字节相同"的实测只说明
    // **认不出的族名**会落进"已注册字体按注册序"的兜底桶（`fonts.rs` 的 `order` +
    // `last_resort_order`，不是内置 Geist）——**不是**族名不起作用。
    //
    // ⇒ 两条控制点都有效，且必须**一起**看：
    //   ① `cover_font_files()` 的顺序（见 `cover.rs`）决定"认不出的族名"落哪一支；
    //   ② CSS 的族名（库 id）决定"指定了族名"时用哪一支 —— 标题里的 emoji 段就靠它
    //      （`cover.rs::emoji_run_span` 写库 id；浏览器那条路同 id 注册 FontFace ⇒ 两边同一支）。
    // 所以别名注册**不是前瞻保险，是当前就在用的机制**（别删）。
    // 字节用 `Arc` 共享：`FontSource::from_shared` 只克隆指针，不复制整支字体。
    for path in fonts {
        let bytes: Arc<Vec<u8>> =
            Arc::new(fs::read(path).with_context(|| format!("读字体失败: {}", path.display()))?);
        font_ctx
            .register(FontResource::new(shared_font_bytes(bytes.clone())))
            .with_context(|| format!("注册字体失败: {}", path.display()))?;
        for alias in font_aliases(path) {
            let res = FontResource::new(shared_font_bytes(bytes.clone())).override_info(FontOverride {
                family_name: Some(Arc::from(alias.as_str())),
                ..Default::default()
            });
            font_ctx.register(res).with_context(|| {
                format!("按 id 别名注册字体失败: {} -> {}", path.display(), alias)
            })?;
        }
    }

    // 图片内联 + emoji 字族权威层 + 标题双层（描边在下、填充在上）—— 都在交给 HTML 前端之前做完
    let inlined = inline_local_images(html, asset_root);
    // ⚠️ **必须赶在 `split_title_layers` 之前**：那一层包上 wrapper 后，"找 `<div id="fx-extra">`
    // 的 `>` 插一个子节点"的判据虽然不变，但"emoji 层的 inline 签名"会不会被标题层的
    // 复制带乱，谁也不想赌 —— 趁 HTML 还是 `build_cover_page` 的原始形状时先插。
    let inlined = inject_emoji_layer_font(&inlined);
    // 描边宽度/颜色先从 CSS 里取（值已在 `:root{}` 里有定义，直接读字面量），交给双层拆分做内联 style
    // 描边取**变量展开之后**的字面量：内联 style 里不能留 `var()` ——
    // Takumi 遇到不可解析的值会**丢掉整条内联样式**（连带 `color:transparent` 一起没，标题就还是黑块）。
    let css_inlined = inline_css_vars(css);
    let stroke = regex_lite::Regex::new(r"-webkit-text-stroke\s*:\s*([^;}]+)")
        .ok()
        .and_then(|re| re.captures(&css_inlined))
        .map(|c| format!("-webkit-text-stroke:{}", c[1].trim()))
        .unwrap_or_default();
    let html = split_title_layers(&inlined, &stroke, &css_inlined);
    // ⚠️ **遥测**：这条路上的每一步都是"静默不生效"的高危形态（正则没命中、选择器没匹配、属性没覆盖），
    // 日志里不写清楚，改没改就只能靠像素去猜（实测踩过：双层标题改完数字几乎不动，
    // 分不清是"没命中"还是"命中了但没用"）。
    let title_split = html != inlined;
    let emoji_font_layer = html.contains("data-vt-emoji-font");
    let images = html.matches("src=\"data:").count();
    tracing::info!(
        "封面(takumi) 适配：标题双层={} emoji字族层={} 内联图片={} 张 HTML={}B CSS={}B",
        title_split, emoji_font_layer, images, html.len(), css.len()
    );
    if let Ok(dir) = std::env::var("VTEDIT_TAKUMI_DUMP") {
        let dir = PathBuf::from(dir);
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(dir.join("takumi-input.html"), &html);
        let _ = fs::write(dir.join("takumi-input.css"), &css);
    }
    let node = from_html(&html, FromHtmlOptions::default())
        .map_err(|e| anyhow::anyhow!("封面 HTML 解析失败: {e:?}"))?;
    // 复合选择器扁平化要**看着 HTML**（只扁平化真的出现的 class），所以放在这里而不是
    // `adapt_css_for_takumi` 里。标题双层那三条规则跟在最后（同特异度按文档序覆盖）。
    let css = flatten_compound_selectors(&adapt_css_for_takumi(css), &html);
    // 变量就地展开 + emoji 兜底后再喂给 Takumi（见两个函数上方的实测理由）
    let css = ensure_emoji_fallback(&inline_css_vars(&css));
    let before = css.len();
    let css = hoist_frame_transform(&css);
    let hoisted = css.len() != before;
    if let Ok(dir) = std::env::var("VTEDIT_TAKUMI_DUMP") {
        let _ = fs::write(std::path::PathBuf::from(dir).join("takumi-final.css"), &css);
    }
    let css = format!("{css}\n{TAKUMI_TITLE_LAYER_CSS}");
    // ⚠️ **这一行是定位"输入变了但输出没变"的关键**：三个 hash 一起看 ——
    // HTML/CSS 变了而出图没变 = 渲染器没吃这份输入；三个都没变 = 没走到这段代码。
    // 诊断开关（CSS-only 探针）：往**适配后的 CSS** 里注入一条必然看得见的规则。
    // 产物 hash 变了 = CSS 链路通；没变 = 追加的 CSS 没进 StyleSheet::parse。
    let css = if std::env::var("VTEDIT_TAKUMI_CSS_PROBE").is_ok() {
        format!("{css}\n#title-wrap{{background:#00ffff;}}")
    } else {
        css
    };
    tracing::info!(
        "封面(takumi) 生效：标题双层={} 内联图片={} frame搬移={} stroke={:?} HTML={} CSS={}",
        title_split, images, hoisted, stroke, short_hash(html.as_bytes()), short_hash(css.as_bytes())
    );
    let sheet = StyleSheet::parse(&css)
        .map_err(|e| anyhow::anyhow!("封面 CSS 解析失败: {e:?}"))?;

    let bitmap = render(
        RenderOptions::builder()
            .viewport(Viewport::new((width, height)))
            .node(node)
            .fonts(&font_ctx)
            .stylesheet(sheet.into())
            .build(),
    )
    .with_context(|| format!("静态渲染失败（{width}×{height}）"))?;

    let mut file = fs::File::create(out).with_context(|| format!("写封面失败: {}", out.display()))?;
    // ⚠️ 必须是 **JPEG q85**：产物就叫 `cover.jpg`，而 Chrome 那条通道用的正是
    // `CaptureScreenshotFormatOption::Jpeg` + `quality: 85`。写成 PNG 会同时踩两件事 ——
    // 扩展名与内容不符，以及"对拍里混进 PNG vs JPEG 的格式差"（实测能占十几/255）。
    write_image(&bitmap, &mut file, OutputFormat::Jpeg { quality: Quality::new(85) })
        .with_context(|| format!("编码封面失败: {}", out.display()))?;
    // 产物侧的 hash：与上面两个一起就能判定"是输入没变、还是渲染没响应输入"
    if let Ok(bytes) = fs::read(out) {
        tracing::info!("封面(takumi) 产物：{} bytes hash {}", bytes.len(), short_hash(&bytes));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️ **P1-e 的钉子**：一支库里字体在 CSS 里可能被请求到的 id，必须**两种布局都算得出来**。
    ///
    /// 背景：模板里的字体栈写的是**配置里存的 id**（库 id），而 Takumi 只按字体文件内嵌的族名匹配。
    /// 两者不等时（常见：id 是文件名、内嵌族名是"LXGW WenKai Lite"）它**静默回退**内置 Geist
    /// —— 中文标题变豆腐块、不报错。所以"能不能算出别名"直接决定封面字体对不对。
    #[test]
    fn font_aliases_cover_both_layouts() {
        // v2 目录布局：id = 目录名（约定文件名 font.ttf **不**当 id 用）
        assert_eq!(
            font_aliases(Path::new("/fonts/LXGW-WenKai-Lite/font.ttf")),
            vec!["LXGW-WenKai-Lite".to_string()],
            "目录布局的 id 是目录名，且 font.ttf 本身不是 id"
        );
        // 老平铺布局：id = 文件名去扩展名
        assert_eq!(
            font_aliases(Path::new("/fonts/LXGWWenKaiLite-Regular.ttf")),
            vec!["LXGWWenKaiLite-Regular".to_string()],
            "老布局的 id 是文件名去扩展名"
        );
        // 带空格的 id（库允许）照样算得出来
        assert_eq!(
            font_aliases(Path::new("/fonts/My Font/font.otf")),
            vec!["My Font".to_string()]
        );
        // 目录组件不参与取名（`evil` 是合法 id，`..` 在目录那一段被忽略 ——
        // 而且 `cover_font_files()` 只会给出 `scan_all` 产出的安全 id，这条路上不会有穿越路径）
        assert_eq!(
            font_aliases(Path::new("/fonts/../evil.ttf")),
            vec!["evil".to_string()]
        );
        // `font.ttf` 一律按 v2 目录布局解释、id = 父目录名 —— 与 `scan_all` 同口径
        // （哪怕父目录恰好叫 `fonts`：那是库里真有一个叫 `fonts` 的 slug，不是特例）
        assert_eq!(
            font_aliases(Path::new("/fonts/font.ttf")),
            vec!["fonts".to_string()]
        );
    }

    /// ⚠️ **id 住在 `FONT.toml` 里时以元数据为准**（2026-10-02：目录名改成 ASCII slug）。
    ///
    /// 落盘是 `font-<hash>/font.ttf` + `FONT.toml{id="乐米波波体"}`，而封面 CSS 请求的是
    /// 那串汉字 id —— 别名必须认它，否则中文标题会落到“注册序第一支”（静默换字体）。
    #[test]
    fn font_aliases_prefer_meta_id_over_slug_dir() {
        let dir = std::env::temp_dir().join(format!("yekitrim-takumi-alias-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let slug = dir.join("font-9c1d2e3f4a5b");
        std::fs::create_dir_all(&slug).unwrap();
        std::fs::write(slug.join("font.ttf"), b"x").unwrap();
        std::fs::write(slug.join("FONT.toml"), "id = \"乐米波波体\"\ndisplay_name = \"乐米波波体\"\n").unwrap();
        let out = font_aliases(&slug.join("font.ttf"));
        assert_eq!(out, vec!["乐米波波体".to_string()], "必须以元数据里的 id 为准");
        assert!(!out.iter().any(|a| a.contains("9c1d2e3f4a5b")), "slug 不该当别名：{out:?}");
        // 元数据在、但 id 是空的 ⇒ 回落目录名（老库形状，行为不变）
        std::fs::write(slug.join("FONT.toml"), "display_name = \"x\"\n").unwrap();
        assert_eq!(font_aliases(&slug.join("font.ttf")), vec!["font-9c1d2e3f4a5b".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ⚠️ **内联 style 是多条声明挤一行**（`split_title_layers` 发的描边层就是）——
    /// `strip_declaration` 必须按 分号 / 花括号切，不能只按分号切（行尾那条没有分号）。
    #[test]
    fn strip_declaration_handles_multi_declaration_lines() {
        let line = r#"<div id="title" style="position:absolute;color:transparent;-webkit-text-stroke:20px #2a2522">"#;
        // 情况 1：行尾那条不带分号（到行尾为止）—— 必须能删
        let out = strip_declaration(line, "-webkit-text-stroke");
        assert!(!out.contains("-webkit-text-stroke"), "行尾无分号的声明没删掊：{out}");
        assert!(out.contains("color:transparent"), "顺手把别的声明删了：{out}");
        // 情况 2：后面还有别的声明（带分号）—— 原行为保持
        let line2 = "-webkit-text-stroke:20px #2a2522;color:red;}";
        let out2 = strip_declaration(line2, "-webkit-text-stroke");
        assert_eq!(out2, "color:red;}", "带分号那条照旧删：{out2}");
        // 情况 3：paint-order 照旧整条删
        assert_eq!(strip_declaration("paint-order:stroke fill;}", "paint-order"), "}", "paint-order 照旧整条删");
    }

    /// emoji 字族权威层：签名命中才插、幂等、零布局影响。
    #[test]
    fn inject_emoji_layer_font_is_targeted_and_idempotent() {
        let fx = r#"<div id="fx-extra"><div style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:120px;line-height:1;font-family:'Segoe UI Emoji','Noto Color Emoji',sans-serif;filter:drop-shadow(0 6px 14px rgba(0,0,0,0.45));">🙂</div></div>"#;
        let once = inject_emoji_layer_font(fx);
        assert!(once.contains("data-vt-emoji-font"), "没插上：{once}");
        // 权威栈打头的是库里那支（测试环境下库里是内置 Noto Color Emoji CBDT）
        assert!(once.contains("font-family:'"), "栈必须是引号包住的族名列表：{once}");
        // 插在 #fx-extra 开标签**之后**（子节点，不是兄弟节点）
        let i_extra = once.find(r#"<div id="fx-extra">"#).expect("#fx-extra 还在");
        let i_layer = once.find("data-vt-emoji-font").expect("标记还在");
        assert!(i_layer > i_extra, "权威层必须在 #fx-extra 之内");
        // 幂等
        assert_eq!(inject_emoji_layer_font(&once), once);
        // 没有 emoji 特效（无 inline font-family 签名）就不动
        let no_fx = r#"<div id="fx-extra"></div>"#;
        assert_eq!(inject_emoji_layer_font(no_fx), no_fx);
    }

    /// 标题双层：描边层必须带 stroke-linejoin:round（Miter 出尖刺，见函数上方注释），
    /// 填充层不带（它没有描边）。
    #[test]
    fn title_stroke_layer_gets_round_join() {
        let html = r#"<div id="page"><div id="fx-extra"></div><div id="title-wrap"><div id="title">测试<br>标题</div></div></div>"#;
        let out = split_title_layers(html, "-webkit-text-stroke:20px #2a2522", ":root{--fs:120px;--lh:1.7}");
        let n = out.matches("stroke-linejoin:round").count();
        assert_eq!(n, 1, "只有描边层带 round join：{out}");
        assert!(out.contains(r#"style="position:absolute;left:0;top:0;width:100%;color:transparent;-webkit-text-stroke:20px #2a2522;stroke-linejoin:round""#), "内联形状不对：{out}");
        // 空描边（诊断开关剥光）时不多出一个分号
        let out0 = split_title_layers(html, "", "");
        assert!(!out0.contains("stroke-linejoin"), "空描边不该注入 join：{out0}");
    }
    /// **真渲染探针**（ignored，默认不进门禁）：跑一次微型 Takumi 光栅化，验证
    /// ① 新加的 emoji 描边环 filter 链能被消化（不 panic / 不静默丢层）；
    /// ② 描边层 + stroke-linejoin:round 的内联不会让 CSS 解析失败。
    /// 本机跑法（要真实字体库；缺中文/emoji 字体时探针 panic 提示，不算功能回归）：
    /// `VTEDIT_TAKUMI_PROBE=1 cargo test -p yekitrim-polish --lib --features cover-takumi probe_renders_ring_and_join -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_renders_ring_and_join() {
        let dir = match std::env::var("VTEDIT_TAKUMI_PROBE") {
            Ok(v) if v.is_empty() => std::env::temp_dir().join("yekitrim-takumi-probe"),
            Ok(_) => std::env::temp_dir().join("yekitrim-takumi-probe"),
            Err(_) => {
                eprintln!("VTEDIT_TAKUMI_PROBE 未设，跳过（探针只做人肉验收）");
                return;
            }
        };
        let mut fonts: Vec<std::path::PathBuf> = Vec::new();
        // 字体目录：`font_dir()` 是现在唯一的位置（`<数据目录>/resources/fonts`）；
        // 后两项是**探针专用**的回落态（人肉验收时也许还没跑过 server，库是空的）
        // cargo test 的 cwd = crate 目录 ⇒ 用 CARGO_MANIFEST_DIR 的父目录锚 repo 根
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).map(Path::to_path_buf).unwrap_or_default();
        let candidates = [
            yekitrim_core::path::font_dir(),
            repo.join("target/debug/resources/fonts"),
            repo.join("resources/fonts"),
        ];
        for root in candidates.iter() {
            if !root.is_dir() { continue; }
            if let Ok(entries) = fs::read_dir(root) {
                for e in entries.flatten() {
                    let p = e.path();
                    let emoji = p.file_stem().and_then(|s| s.to_str()).map(|s| s.contains("emj")).unwrap_or(false);
                    if emoji { fonts.push(p); } else { fonts.insert(0, p); }
                }
            }
        }
        assert!(!fonts.is_empty(), "探针需要字体：target/debug/resources/fonts（缺了不算功能回归）");
        let html = format!(
            r#"<div style="width:100%;height:100%;position:relative"><div id="fx-extra"><div style="font-family:{};font-size:0;line-height:1;" data-vt-emoji-font></div><div style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);font-size:48px;line-height:1;font-family:{};filter:drop-shadow(3px 0 0 #2a2522) drop-shadow(-3px 0 0 #2a2522) drop-shadow(0 3px 0 #2a2522) drop-shadow(0 -3px 0 #2a2522) drop-shadow(0 6px 14px rgba(0,0,0,0.45));">🙂</div></div><div id="title-wrap"><div id="title" style="position:absolute;left:0;top:0;width:100%;color:transparent;-webkit-text-stroke:6px #2a2522;stroke-linejoin:round">测试</div><div id="title" style="position:absolute;left:0;top:0;width:100%;-webkit-text-stroke:0">测试</div></div></div>"#,
            crate::cover::emoji_font_stack(),
            crate::cover::emoji_font_stack(),
        );
        let css = "#page{width:256px;height:128px}#title{font-size:24px;font-weight:900;stroke-linejoin:round}";
        let _ = fs::create_dir_all(&dir);
        let out = dir.join("probe.png");
        render_cover_png(&html, css, Path::new("."), &out, 256, 128, &fonts)
            .expect("探针渲染失败：新适配让 Takumi 消化不了");
        let bytes = fs::read(&out).expect("产物没写出");
        tracing::info!("探针产物 {} bytes hash {}", bytes.len(), short_hash(&bytes));
    }

    /// **引擎能力探针**（ignored）：拿**真实字体库**验三件事 ——
    /// ① `font-family` 族名能否定向选中已注册字体（Takumi 的 fallback 是按注册序，族名到底认不认）；
    /// ② 联排 emoji 外面套一个带 `filter:drop-shadow` 的 inline span，能不能出描边环；
    /// ③ 带 `display:inline-block` 时环是否照旧。
    /// 跑法：`VTEDIT_PROBE_FONT_ROOT=<字体库> VTEDIT_PROBE_OUT=<目录> cargo test -p yekitrim-polish --lib \
    ///   --features cover-takumi probe_title_emoji_variants -- --ignored --nocapture`
    #[cfg(feature = "cover-takumi")]
    #[test]
    #[ignore]
    fn probe_title_emoji_variants() {
        let root = PathBuf::from(
            std::env::var("VTEDIT_PROBE_FONT_ROOT")
                .unwrap_or_else(|_| yekitrim_core::path::font_dir().display().to_string()),
        );
        let ids = [
            "乐米波波体",
            "LXGWWenKaiLite-Regular",
            "NotoSansSC-Regular",
            "NotoColorEmoji-CBDT",
        ];
        let fonts: Vec<PathBuf> = ids
            .iter()
            .filter_map(|id| yekitrim_core::fonts::resolve_all(&root, id))
            .filter(|p| p.is_file())
            .collect();
        eprintln!("探针：字体注册序 = {:?}", fonts);
        assert!(!fonts.is_empty(), "字体库为空：{}", root.display());
        let out_dir = PathBuf::from(
            std::env::var("VTEDIT_PROBE_OUT")
                .unwrap_or_else(|_| std::env::temp_dir().join("yekitrim-cover-variants").display().to_string()),
        );
        let _ = fs::create_dir_all(&out_dir);
        let base = "position:absolute;left:40px;font-size:70px;line-height:1;color:#f0e6d3;\
                    -webkit-text-stroke:14px #2a2522;stroke-linejoin:round;font-weight:900;";
        // text-shadow 链：每条阴影都是**字形副本**（不累积），与 drop-shadow 链对照
        let ts4 = "text-shadow:5px 0 0 #2a2522,-5px 0 0 #2a2522,0 5px 0 #2a2522,0 -5px 0 #2a2522;";
        let ts8 = "text-shadow:5px 0 0 #2a2522,-5px 0 0 #2a2522,0 5px 0 #2a2522,0 -5px 0 #2a2522,3.5px 3.5px 0 #2a2522,3.5px -3.5px 0 #2a2522,-3.5px 3.5px 0 #2a2522,-3.5px -3.5px 0 #2a2522;";
        let html = format!(
            r#"<div id="page" style="position:absolute;inset:0;background:#808080">
<div style="{base}top:10px;font-family:'乐米波波体'">幼儿园🩸 12</div>
<div style="{base}top:100px;font-family:'乐米波波体'">幼儿园<span style="font-family:'NotoColorEmoji-CBDT'">🩸</span> 12</div>
<div style="{base}top:190px;font-family:'乐米波波体'">幼儿园<span style="font-family:'NotoColorEmoji-CBDT';filter:drop-shadow(5px 0 0 #2a2522) drop-shadow(-5px 0 0 #2a2522) drop-shadow(0 5px 0 #2a2522) drop-shadow(0 -5px 0 #2a2522)">🩸</span> 12</div>
<div style="{base}top:280px;font-family:'乐米波波体'">幼儿园<span style="font-family:'NotoColorEmoji-CBDT';display:inline-block;filter:drop-shadow(5px 0 0 #2a2522) drop-shadow(-5px 0 0 #2a2522) drop-shadow(0 5px 0 #2a2522) drop-shadow(0 -5px 0 #2a2522)">🩸</span> 12</div>
<div style="{base}top:450px;font-family:'乐米波波体'">幼儿园<span style="font-family:'NotoColorEmoji-CBDT';{ts4}">🩸</span> 12</div>
<div style="{base}top:530px;font-family:'乐米波波体'">幼儿园<span style="font-family:'NotoColorEmoji-CBDT';display:inline-block;{ts8}">🩸</span> 12</div>
<div style="{base}top:370px;font-family:'NotoColorEmoji-CBDT'">12 幼儿园🩸</div>
</div>"#
        );
        let css = "#page{width:1920px;height:620px}";
        let out = out_dir.join("variants.jpg");
        render_cover_png(&html, css, &out_dir, &out, 1920, 620, &fonts).expect("探针渲染失败");
        eprintln!("探针：产物 = {}", out.display());
    }

    /// 标题层必须**关掉 Takumi 的合成加粗**（用户报「中文描边不够准确」的根因）：
    /// 模板写 `font-weight:900`，Chrome 对库里那支单字面字体**不加粗**（实测 400/900 逐像素相同），
    /// 而 Takumi 会合成加粗（填充向外撑 `font_size/32`）⇒ 填充盖掉描边内缘、可见描边变细 3px。
    #[test]
    fn title_layer_disables_synthetic_bold() {
        assert!(
            TAKUMI_TITLE_LAYER_CSS.contains("#title{font-weight:400}"),
            "标题层必须显式关掉合成加粗：{TAKUMI_TITLE_LAYER_CSS}"
        );
    }
    /// ⚠️ **emoji 兜底注入必须幂等**（2026-10-02 修掉的既有 bug）。
    ///
    /// 原来的实现**无条件**往通用族前插前缀：对已经带 emoji 的栈会插第二遍，
    /// 拼出 `'Noto Color Emoji','Segoe UI Emoji','Noto Color Emoji',sans-serif`。
    /// 它当时没暴露纯属侥幸（emoji 特效层是 inline style、本函数的调用点只扫 `<style>` 块），
    /// 所以这条判据是**防止将来有人改了调用点就中招**。
    #[test]
    fn emoji_fallback_is_idempotent() {
        let once = ensure_emoji_fallback(".cap{font-family:'Noto Sans SC',sans-serif;}");
        assert!(once.contains("Emoji"), "第一遍该插进去：{once}");
        let twice = ensure_emoji_fallback(&once);
        assert_eq!(once, twice, "第二遍不许再插一次（幂等）");
        // 幂等的实质判据：通用族前那段只出现**一套** emoji 族名
        // （权威栈形状 = 权威首项（'Noto Color Emoji' 之类）+ 系统尾部 3 个 ⇒ 上限 4；
        //  旧判据 n<=2 是权威栈改形（emoji_font_stack 打头带库内族名）之前的口径）
        let head = twice.split("sans-serif").next().unwrap_or("");
        let n = head.matches("Emoji").count();
        assert!(n <= 4, "emoji 族名重复注入（{n} 处）：{twice}");
    }

    /// emoji 兜底注入要覆盖三种通用族，且不动别处。
    #[test]
    fn emoji_fallback_covers_generic_families() {
        let out = ensure_emoji_fallback("a{font-family:'X',serif;}b{font-family:'Y',monospace;}");
        assert!(out.contains("Emoji"), "serif/monospace 也要覆盖：{out}");
        // 没有 font-family 的规则行不受影响
        let plain = ensure_emoji_fallback("c{color:red;}");
        assert_eq!(plain, "c{color:red;}", "没有 font-family 就别动它");
    }

    /// ⚠️ 这条钉的是**真实模板**：它的注释里提到了 `<style>`，naive 抽取会把注释当 CSS
    /// （Takumi 报 `DanglingCombinator`）。实测踩到过，所以判据按真模板来。
    #[test]
    fn extract_ignores_the_comment_trap() {
        let html = crate::cover::COVER_HTML;
        let css = extract_style_blocks(html);
        assert!(!css.contains("-->"), "注释文字漏进 CSS 了：{css}");
        assert!(!css.contains("单独一个"), "注释文字漏进 CSS 了：{css}");
        assert!(css.contains("#bg"), "没抽到主样式：{css}");
        assert!(css.contains(":root{"), "没抽到静态变量块：{css}");

        // 直接对真模板断言一句注释确实存在（免得以后有人删了注释、这条测试变成空转）
        assert!(html.contains("单独一个 <style>"), "模板里的那句注释没了，这条判据要重写");
    }

    /// 两处引擎差异只改给 Takumi 的那一份。
    #[test]
    fn adapt_fixes_paint_order_and_repeating_angle() {
        let css = "#t{color:#fff;paint-order:stroke fill;font-size:56px}\n\
                   #s{background:repeating-linear-gradient(0deg,rgba(255,255,255,.2) 0 1px,transparent 1px 4px)}";
        let out = adapt_css_for_takumi(css);
        assert!(!out.contains("paint-order"), "paint-order 没删掉：{out}");
        assert!(out.contains("color:#fff"), "顺手把别的声明删了：{out}");
        assert!(out.contains("font-size:56px"), "顺手把别的声明删了：{out}");
        assert!(!out.contains("(0deg"), "扫描线角度没改（`180deg` 里含 `0deg` 子串，所以判据要带括号）：{out}");
        assert!(out.contains("repeating-linear-gradient(180deg"), "扫描线角度没改：{out}");
    }

    /// E2E 抓到的第 5 条：项目选了库内字体后模板会注入 `@font-face`，而 Takumi 的样式表解析器
    /// **不认这个 at-rule**（`AtRuleInvalid("font-face")` → 整张封面直接失败）。
    #[test]
    fn adapt_drops_font_face_blocks() {
        let css = "@font-face { font-family:'SimHeiTest'; src:url('/vt-fonts/SimHeiTest'); }\n\
                   #title{color:#fff;font-size:56px}\n\
                   @font-face{font-family:'X';src:url(/vt-fonts/X)}";
        let out = adapt_css_for_takumi(css);
        assert!(!out.contains("@font-face"), "`@font-face` 没删掉：{out}");
        assert!(!out.contains("vt-fonts"), "字体块的内容漏出来了：{out}");
        assert!(out.contains("#title{color:#fff;font-size:56px}"), "顺手删掉了别的规则：{out}");
    }

    /// 标题双层：描边层插在**前面**（同 z-index 时 DOM 序决定上下），填充层是原块。
    /// 标题双层：**钉住我踩过的三个真 bug** ——
    /// ① 发出去的 HTML 必须配平（第一版正则少吃了 `</div>` ⇒ `<div>12 vs </div>11`，两层关系全乱）；
    /// ② 两层必须**同一个盒子**（一层 flex 子项 max-content、另一层 width:100% 时错位，整图差反而变大）；
    /// ③ 模板的 `.hl` **自带描边** ⇒ 两层都要各自处理（描边层透明、填充层去描边），漏了就是实心黑块；
    /// ④ wrap 高度按**可见行**算（结尾那个 `<br>` 不算 —— 算成 5 行会整体下移 ~102px，实测整图差 27→3.5）。
    #[test]
    fn title_layers_are_balanced_positioned_and_use_visible_lines() {
        let html = r#"<div id="page"><div id="fx-extra"></div><div id="title-wrap"><div id="title"><span class="hl">幼儿园</span>印象<br>小红马和<span class="hl">跷跷板</span><br>玩上头<br>直接飞出去流鼻血<br></div></div></div>"#;
        let out = split_title_layers(html, "-webkit-text-stroke:20px #2a2522", ":root{--fs:120px;--lh:1.7}");
        assert_eq!(
            out.matches("<div").count(),
            out.matches("</div>").count(),
            "发出的 HTML 必须配平：{out}"
        );
        assert_eq!(
            out.matches("position:absolute;left:0;top:0;width:100%;").count(),
            2,
            "两层必须同盒：{out}"
        );
        assert_eq!(
            out.matches(r#"<span class="hl" style="color:transparent">"#).count(),
            2,
            "描边层的高亮词要透明：{out}"
        );
        assert_eq!(
            out.matches(r#"<span class="hl" style="-webkit-text-stroke:0">"#).count(),
            2,
            "填充层的高亮词要去描边：{out}"
        );
        assert!(
            out.contains(r#"<div id="title-wrap" style="height:816px">"#),
            "高度要按可见行算（4×120×1.7=816，不是 1020）：{out}"
        );
        assert_eq!(split_title_layers(r#"<div id="x"></div>"#, "", ""), r#"<div id="x"></div>"#);
    }

    /// 复合选择器：**只扁平化 HTML 里真出现的 class**（`#bg.bg-blur` → `#bg`）。
    /// 最小单点证据：Chrome `#a.k img` 的 A−B = 104.11，Takumi 只有 2.01（= 没匹配）。
    #[test]
    fn flatten_only_the_classes_that_are_really_there() {
        let html = r#"<div id="bg" class="bg-blur"><img src="x"></div><div class="other" id="v"></div>"#;
        let css = "#bg img{display:none}#bg.bg-blur img{display:block}#bg.bg-image img{display:block}\
                   #v.other{opacity:0}#v.missing{opacity:1}";
        let out = flatten_compound_selectors(css, html);
        assert!(out.contains("#bg img{display:block}"), "`#bg.bg-blur` 没扁平化：{out}");
        assert!(out.contains("#bg.bg-image img"), "HTML 里没有 bg-image 这个 class，不该动：{out}");
        assert!(out.contains("#v{opacity:0}"), "class 写在 id 前面也要认：{out}");
        assert!(out.contains("#v.missing{opacity:1}"), "没出现的 class 不该动：{out}");
    }

    /// 没有字体时必须**明确报错**（Takumi 不读系统字体，静默出豆腐块是最坏的形态）。
    #[test]
    fn no_fonts_is_an_error_not_tofu() {
        let e = render_cover_png("<div>x</div>", "", Path::new("."), Path::new("nope.png"), 32, 32, &[]).unwrap_err();
        assert!(format!("{e}").contains("字体"), "{e}");
    }
}
