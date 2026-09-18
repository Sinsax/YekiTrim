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
fn dump_text_png() {
    for sw in [2.0, 12.0] {
        let res = vtrim_render_core::composite::render_text_rgba(
            "测试字幕", 72.0, [255,255,255,255], [0,0,0,255], sw, &font());
        let (rgba, w, h, _) = res.unwrap();
        let vis = rgba.chunks(4).filter(|p| p[3] > 0).count();
        eprintln!("sw={} size={}x{} vis={}", sw, w, h, vis);
    }
}
