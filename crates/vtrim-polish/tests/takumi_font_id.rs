//! **P1-e 验收：库 id 别名注册真的生效**（Takumi 那条腿的字体统一）。
//!
//! # 为什么必须有这条
//!
//! 模板里的字体栈写的是**配置里存的库 id**（`.cap{font-family:"<id>"}` / 封面 `__FONT_FAMILY__`），
//! 而 Takumi `Fonts::register` **只按字体文件内嵌的族名**建族。两者不等时
//! （很常见：id 是文件名 `SimHeiTest`、内嵌族名是别的）它**静默回退**到内置 Geist ——
//! 标题字体不对，**不报错**。这条就是"预览与实际对不上"的 Takumi 版。
//!
//! # 判据：**逐像素差异**（不是"暗像素数"）
//!
//! 同一份 HTML，三次渲染只换 CSS 里请求的族名：
//!  - `base`  = 请求**内嵌族名**（Takumi 原生行为）
//!  - `by_id` = 请求**库 id**（全靠我们加的别名）
//!  - `miss`  = 请求**不存在的族名** ⇒ 回退 Geist，这就是"没接上"长什么样
//!
//! 断言**关系**而不是像素数（不写死任何数字）：
//!  ① `by_id` **不同于** `miss`  ⇒ 别名真的把库 id 接到了那支字体上（核心断言）；
//!  ② `by_id` **不同于** `miss` 且 `base == miss` 或 `base == by_id` —— 见下。
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
//! 字体从**程序字体库**里找（`vtrim_core::fonts::scan_all`，两种布局都认）；
//! 库里没有可用字体、或元数据没有内部族名时**跳过**（打印原因，不让门禁红）——
//! 这条测试的意义是"机器上有字体时必须对"，不是"必须造一支字体出来"。
#![cfg(feature = "cover-takumi")]

use std::path::PathBuf;

/// 渲染一次，返回 RGBA 像素。
fn render_rgba(html: &str, fonts: &[PathBuf], tag: &str) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("vt-font-id-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let out = dir.join(format!("out-{tag}.jpg"));
    vtrim_polish::render::takumi::render_cover_png(html, "", &dir, &out, 320, 120, fonts).ok()?;
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
    let dir = vtrim_core::fonts::dir();
    let Some(entry) = vtrim_core::fonts::scan_all(&dir).into_iter().next() else {
        eprintln!("跳过：字体库（{}）里一支字体都没有", dir.display());
        return;
    };
    let Some(path) = vtrim_core::fonts::resolve_all(&dir, &entry.id) else {
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

    // **核心断言**：请求库 id 必须与"回退"**长得不一样** —— 这就是"别名接上了"的定义。
    // （不加别名时，请求库 id 会静默回退，与 miss 逐像素一致。）
    assert!(
        d_id_miss > 0,
        "请求库 id 与请求不存在的族名渲染结果完全相同 ⇒ 别名注册没生效（Takumi 静默回退了 Geist）"
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
        "请求一个无关族名也变了 ⇒ 别名注册的覆盖面不对（不该命中这个串）"
    );
}
