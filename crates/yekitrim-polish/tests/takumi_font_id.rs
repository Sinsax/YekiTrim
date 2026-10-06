//! **P1-e 验收：库 id 别名注册真的生效**（Takumi 那条腿的字体统一）。
//!
//! # 为什么必须有这条
//!
//! 模板里的字体栈写的是**配置里存的库 id**（`.cap{font-family:"<id>"}` / 封面 `__FONT_FAMILY__`），
//! 而 Takumi `Fonts::register` **只按字体文件内嵌的族名**建族。两者不等时
//! （很常见：id 是文件名 `SimHeiTest`、内嵌族名是别的）它**静默回退**到内置 Geist ——
//! 标题字体不对，**不报错**。这条就是"预览与实际对不上"的 Takumi 版。
//!
//! # ⚠️ 先读这段：这条测试**测不出它原本想测的东西**（2026-10-01 实测结论）
//!
//! 原判据是"请求库 id ≠ 请求不存在的族名"。**那个不等的基线不存在**：
//! Takumi 的 fallback 桶 = **已注册字体按注册序的全集**（`takumi-core`
//! `resources/font.rs` 的 `order` + `last_resort_order`），认不出的族名**不是**回退到内置
//! Geist，而是回退到**整个字体库**、挑第一支"有该字形"的字体画。
//!
//! 实测（本机字体库）：
//!  - 只注册 LXGW：请求 `__nope__` / 库 id / 内嵌族名 → 三张图**逐字节相同**；
//!  - 注册 [Emoji, LXGW]（Emoji 在前）：请求 `__nope__` / `NotoColorEmoji` /
//!    `LXGWWenKaiLite-Regular` → 三张图**仍然逐字节相同**，中文都正常（落到 LXGW）。
//!
//! ⇒ 上面那段"三张图逐字节相同"的观察**有两个混淆项**，别把它的结论当成"族名无用"：
//!   ① 那两次只注册**一支**字体 —— 那种设置下任何族名都只能落到它，**必然**相同，
//!      对"族名有没有用"**没有分辨力**（所以下面的断言只是"兜底桶"的事实，不是"族名无效"）；
//!   ② `render_cover_png` 那条**真封面**链路当时还有一个 CSS 适配 bug（见
//!      `render/takumi.rs::find_outside_comments`）：模板注释里的 `@font-face` 会让
//!      `strip_at_rule_blocks` 把 `html,body{…font-family:…}` 整条删掉 ⇒ 渲染器**从来没收到**
//!      `font-family`，于是"换三支字体，出图逐字节相同"。用户 2026-10-05 报的
//!      「设计器里选的字体和出图不一致」就是它。
//! **族名是有效的**（2026-10-05 受控实测，三支真字体、同一条链）：元素上显式写
//! `font-family` 三种值 ⇒ 逐像素差 2.5 万级；只靠 `html,body` 继承三种值 ⇒ 同样互不相同。
//! 判据见下面的 `cover_font_change_must_change_the_pixels`（走真封面那张模板）。
//!
//! 这条现在只钉**能钉的事实**（见下面 `assert`）。真正的"用哪支"控制点是
//! `cover.rs::cover_font_files()` 给出的**顺序** —— 那才是该加门禁的地方。
//!
//! # 两次踩过的度量陷阱（都写进注释里，别再犯）
//!
//! 1. **不能用"暗像素数"当判据**：`SimHeiTest.ttf` 没有拉丁字形、Takumi 内置 Geist 没有中文字形，
//!    于是"请求内嵌族名"和"请求不存在的族名"都回退到同一支字体，**暗像素数完全相同**
//!    （实测 1713 == 1713），而**真正被验证的是 `by_id` 那张**（它渲染出的是真字形、字节数不同）。
//!    绝对亮度统计会被回退掩盖，**逐像素比较**才看得出"这两张图不一样"。
//! 2. **CSS 要写在元素上**（`style=`），不能只给 `render_cover_png` 的 css 参数：
//!    Takumi 的 HTML 前端只认内联样式，用外部选择器那张图是**全黑**的。
//!
//! 字体从**程序字体库**里找（`yekitrim_core::fonts::scan_all`，两种布局都认）；
//! 库里没有可用字体、或元数据没有内部族名时**跳过**（打印原因，不让门禁红）——
//! 这条测试的意义是"机器上有字体时必须对"，不是"必须造一支字体出来"。
#![cfg(feature = "cover-takumi")]

use std::path::PathBuf;

/// 渲染一次，返回 RGBA 像素。
fn render_rgba(html: &str, fonts: &[PathBuf], tag: &str) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("vt-font-id-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let out = dir.join(format!("out-{tag}.jpg"));
    yekitrim_polish::render::takumi::render_cover_png(html, "", &dir, &out, 320, 120, fonts).ok()?;
    Some(image::open(&out).ok()?.to_rgba8().into_raw())
}

/// 两张图不同的像素个数（0 = 逐像素一致）。
fn diff_pixels(a: &[u8], b: &[u8]) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(x, y)| x != y)
        .count()
}

#[test]
fn library_id_alias_makes_takumi_use_the_font() {
    let dir = yekitrim_core::fonts::dir();
    let Some(entry) = yekitrim_core::fonts::scan_all(&dir).into_iter().next() else {
        eprintln!("跳过：字体库（{}）里一支字体都没有", dir.display());
        return;
    };
    let Some(path) = yekitrim_core::fonts::resolve_all(&dir, &entry.id) else {
        eprintln!("跳过：id={} 解析不到文件", entry.id);
        return;
    };
    let internal = entry.family_internal.trim().to_string();
    if internal.is_empty() {
        eprintln!("跳过：id={} 没有内部族名元数据（老平铺文件）", entry.id);
        return;
    }
    // id 与内嵌族名相同时这条测试没有分辨力（别名与原生注册是同一个名字）
    if internal == entry.id {
        eprintln!(
            "跳过：id={} 与内嵌族名同名，别名与原生注册无法区分",
            entry.id
        );
        return;
    }
    let fonts = vec![path];
    // ⚠️ 库里只有**一支**可用字体时这条断言没有分辨力：请求不存在族名会回退到"整个字体库"
    // （本机实测就是这个机制），而那库里只有这一支 ⇒ 像素差恒为 0，断言必红。
    // 兄弟测试 `cover_font_change_must_change_the_pixels` 早就按"不足两支就跳过"处理了，
    // 这里补齐同一条（2026-10-06：本机字体库只剩 1 支非 emoji 字体 ⇒ `cargo test --workspace` 红）。
    let usable = yekitrim_core::fonts::scan_all(&dir)
        .into_iter()
        .filter(|f| {
            !f.id.to_ascii_lowercase().contains("emoji")
                && !f.family_internal.to_ascii_lowercase().contains("emoji")
        })
        .count();
    if usable < 2 {
        eprintln!("跳过：字体库里非 emoji 字体不足两支（找到 {usable}）—— 这条断言没有分辨力");
        return;
    }

    // 内联样式（Takumi 的 HTML 前端只认它）+ 显式白底；文字用**拉丁**（Geist 有、
    // 所以"没接上"时会真的回退成不同字形；中文会被回退掩盖 —— 见模块头注释）。
    let html_for = |family: &str| {
        format!(
            r#"<div style="width:320px;height:120px;background:#fff"><span style="font-family:'{family}';font-size:48px;font-weight:700;color:#000">Wgpq</span></div>"#
        )
    };

    let base = render_rgba(&html_for(&internal), &fonts, "base").expect("内嵌族名那次应渲染成功");
    let by_id = render_rgba(&html_for(&entry.id), &fonts, "byid").expect("库 id 那次应渲染成功");
    let miss =
        render_rgba(&html_for("__no_such_family__"), &fonts, "miss").expect("回退那次应渲染成功");

    let d_id_miss = diff_pixels(&by_id, &miss);
    let d_base_miss = diff_pixels(&base, &miss);
    let d_base_id = diff_pixels(&base, &by_id);
    eprintln!(
        "字体 id={} 内嵌族名={:?} ⇒ 逐像素差：id vs 缺失={} 内嵌 vs 缺失={} 内嵌 vs id={}",
        entry.id, internal, d_id_miss, d_base_miss, d_base_id
    );

    // ⚠️ **这条断言 2026-10-05 翻回来了，而且它一直是红的**（本机实测：这条测试在
    // 我动手之前就失败，数字一模一样 3960px）。
    //
    // 原先这里断言 `d_id_miss == 0`（"请求库 id 与请求不存在的族名结果相同"），依据是
    // 模块头那段"三张图逐字节相同"的观察 —— 而那两处观察各带一个混淆项（只注册**一支**字体、
    // 且真封面链路的 CSS 被适配层吃掉），合起来给出了**"族名无用"的假结论**。
    // 实测（三支真字体）：族名是有效的。
    //
    // 现在钉的是**别名真的生效**：请求库 id ≠ 请求不存在的族名。
    assert!(
        d_id_miss > 0,
        "请求库 id 与请求不存在的族名**结果相同**（{d_id_miss} 像素）⇒ 库 id 别名没生效：         用户选的字体到不了渲染器（症状：设计器与出图不一致，改字体不出图变）"
    );
    // ⚠️ **不再断言 `d_base_id`**：本机实测"请求内嵌族名"的行为**与"请求不存在的族名"一致**
    // （`d_base_miss == 0`）而**与请求库 id 不一致**（`d_base_id = 3960`）—— 为什么内嵌族名
    // 没命中它自己那次注册（`FontResource::new` 走的正是内嵌族名），我**没有查到结论**。
    // 猜一个断言写上去，只会把"我不知道"伪装成"我验过"；所以这里只把它打成一行读数。
    eprintln!(
        "   （读数，不算判据）内嵌族名 vs 缺失={} 内嵌族名 vs 库 id={}          —— 内嵌族名为什么没命中自注册，未定论",
        d_base_miss, d_base_id
    );
    // ⚠️ **不要断言 `base == by_id`**（第一版这么写，错）：`base` 请求的是**内嵌族名**，
    // 而这支字体未必有拉丁字形（`SimHeiTest.ttf` 就没有）⇒ 它照样回退成 Geist，
    // 与 `by_id`（真的用了这支字体）**理所当然地不同**（实测差 2879 像素）。
    // 那个比较测的是"字体里有没有这些字形"，不是"别名有没有生效" —— 两件事。
    //
    // 真正要钉的第二件事：**别名只该在库 id 上生效**，不能顺手把"任意名字"都接上。
    // 用一个跟库 id、内嵌族名都无关的第三串去请求，必须仍然回退（= 与 miss 一致）。
    let alias_only = render_rgba(&html_for("SimHeiTestAliasProbe"), &fonts, "probe")
        .expect("探针那次应渲染成功");
    assert_eq!(
        diff_pixels(&alias_only, &miss),
        0,
        "请求一个无关族名结果变了 ⇒ 字体选择不再只由注册序决定"
    );
}

/// 渲染一次（**带外部 CSS**）—— 真封面那条链是"CSS 与 HTML 分开喂"的。
fn render_rgba_css(html: &str, css: &str, fonts: &[PathBuf], tag: &str) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("vt-font-id-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let out = dir.join(format!("out-{tag}.jpg"));
    yekitrim_polish::render::takumi::render_cover_png(html, css, &dir, &out, 640, 200, fonts).ok()?;
    Some(image::open(&out).ok()?.to_rgba8().into_raw())
}

/// **`@font-face` 的清理必须跳过注释**（2026-10-05 那个 bug 的最小判据）。
///
/// 模板里那句注释写着「该字体的 `@font-face` 名 + 原栈」，而清理函数会从**注释里的**那个词
/// 开始删到第一个 `}` —— 正好吃掉紧随其后的 `html,body{…font-family:…}`。
/// 这条不需要字体库，任何机器都能跑（所以它是常驻判据，上面那条真渲染会按需跳过）。
#[cfg(feature = "cover-takumi")]
#[test]
fn at_rule_stripping_must_ignore_comments() {
    let css = "/* 这里提到 @font-face 一次，别把它当 at-rule */
html,body{width:100%;font-family:'乐米波波体','Noto Sans SC',sans-serif;}
@font-face{font-family:x;src:url(y)}";
    let out = yekitrim_polish::render::takumi::adapt_css_for_takumi(css);
    assert!(
        out.contains("font-family:'乐米波波体'"),
        "注释里的 `@font-face` 把后面的规则吃掉了 —— 字体栈到不了渲染器：
{out}"
    );
    assert!(!out.contains("src:url(y)"), "真正的 `@font-face` 块必须被删掉：
{out}");
}

/// **换 `[cover] font` 必须换出图**（用户报的"设计器选的字体与出图不一致"的端到端判据）。
///
/// 走**真封面那条链**：`build_cover_page_static`（真模板）→ `extract_style_blocks` →
/// `adapt_css_for_takumi` → `render_cover_png`。两支不同的库内字体 ⇒ 像素必须不同。
#[cfg(feature = "cover-takumi")]
#[test]
fn cover_font_change_must_change_the_pixels() {
    let dir = yekitrim_core::fonts::dir();
    let entries = yekitrim_core::fonts::scan_all(&dir);
    let mut picked: Vec<(String, PathBuf)> = Vec::new();
    for e in &entries {
        // emoji 字体整幅都是彩色方块，拿它当"另一支"分辨力太强、也太慢；跳过它
        if e.emoji {
            continue;
        }
        if let Some(p) = yekitrim_core::fonts::resolve_all(&dir, &e.id) {
            picked.push((e.id.clone(), p));
        }
        if picked.len() == 2 {
            break;
        }
    }
    if picked.len() < 2 {
        eprintln!("跳过：字体库里非 emoji 字体不足两支（找到 {}）", picked.len());
        return;
    }
    let fonts = picked.iter().map(|(_, p)| p.clone()).collect::<Vec<_>>();
    let render_for = |id: &str, tag: &str| -> Option<Vec<u8>> {
        let mut cfg = yekitrim_polish::project::ProjectFile::example();
        cfg.cover.font = id.to_string();
        let lines = vec!["小学生跳舞".to_string()];
        let html = yekitrim_polish::cover::build_cover_page_static(&cfg, false, &lines);
        let css = yekitrim_polish::render::takumi::extract_style_blocks(&html);
        render_rgba_css(&html, &css, &fonts, tag)
    };
    let a = render_for(&picked[0].0, "cover-a").expect("字体 1 渲染失败");
    let b = render_for(&picked[1].0, "cover-b").expect("字体 2 渲染失败");
    let d = diff_pixels(&a, &b);
    eprintln!("换封面字体（{} -> {}）逐像素差 = {}", picked[0].0, picked[1].0, d);
    assert!(
        d > 0,
        "换 `[cover] font` 出图**逐字节相同** ⇒ 字体栈没走到渲染器（或落进同一个兜底桶）：{} vs {}",
        picked[0].0,
        picked[1].0
    );

    // **存量零回归**：没选字体（`""`）与选了库里没有的字体，都该落到同一个兜底桶 ——
    // 这正是修复前的行为（那时 CSS 里的字体栈被适配层吃掉，两者都落兜底桶）。
    // 修复后"空值"走的是 `fallback_stack()`（首项 = 库里**默认**那支的内部族名），
    // 而它解析出来仍是注册序第一支 = 默认那支 ⇒ 两者**依旧逐像素相同**。
    // 这条一旦变红，说明空值那条路的观感变了 —— 那时必须先重新冻结封面参照帧
    //（`scripts/fixtures/cover-golden/`），别让它悄悄漂。
    let blank = render_for("", "cover-blank").expect("空字体渲染失败");
    let unknown = render_for("__no_such_font_in_library__", "cover-unknown")
        .expect("认不出的字体渲染失败");
    let d2 = diff_pixels(&blank, &unknown);
    eprintln!("空 `[cover] font` vs 认不出的字体 逐像素差 = {d2}");
    assert_eq!(
        d2, 0,
        "空字体与认不出的字体出图不同了 ⇒ 存量工程的封面观感可能已经变了（重新冻结参照帧前先查清）"
    );
}
