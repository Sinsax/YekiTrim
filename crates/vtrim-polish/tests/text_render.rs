//! `vtrim_render_core::composite` 由 `wgpu` feature 门控（见 `vtrim-render-core/src/lib.rs`）：
//! 不开这个 feature 时本文件整体不参与编译，否则 `cargo check --all-targets` 直接报错。
#![cfg(feature = "wgpu-render")]

use vtrim_render_core::composite::{self, FontSpec};

fn font() -> FontSpec {
    let bytes = std::fs::read("C:/Windows/Fonts/msyh.ttc")
        .or_else(|_| std::fs::read("C:/Windows/Fonts/simhei.ttf"))
        .expect("test font");
    composite::set_font_bytes("LXGW WenKai", Some(bytes));
    FontSpec::new("LXGW WenKai")
}

#[test]
fn text_render_produces_nonempty() {
    let res = vtrim_render_core::composite::render_text_rgba(
        "测试字幕",
        72.0,
        [255,255,255,255],
        [0,0,0,255],
        12.0,
        &font(),
    );
    assert!(res.is_some(), "render_text_rgba returned None");
    let (rgba, w, h, _) = res.unwrap();
    assert!(w > 0 && h > 0, "empty texture {w}x{h}");
    assert_eq!(rgba.len(), (w*h*4) as usize);
    let has_alpha = rgba.chunks(4).any(|p| p[3] > 0);
    assert!(has_alpha, "no visible pixels");
}
