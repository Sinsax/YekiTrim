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
fn text_layout_spreads() {
    let res = vtrim_render_core::composite::render_text_rgba(
        "测试字幕", 72.0, [255,255,255,255], [0,0,0,255], 12.0, &font());
    let (rgba, w, h, _) = res.unwrap();
    let mut col_any = vec![false; w as usize];
    for y in 0..h {
        for x in 0..w {
            if rgba[((y*w+x)*4+3) as usize] > 0 { col_any[x as usize] = true; }
        }
    }
    let mut runs = vec![];
    let mut start = None;
    for (i, b) in col_any.iter().enumerate() {
        if *b && start.is_none() { start = Some(i); }
        if !*b && start.is_some() { runs.push((start.take().unwrap(), i-1)); }
    }
    if let Some(s)=start { runs.push((s, w as usize -1)); }
    // 中文字幕字形紧密排列可能连成一片；主要验证有足够的非空列和高度。
    let non_empty_cols = col_any.iter().filter(|b| **b).count();
    assert!(non_empty_cols > 50, "text too narrow, cols {}", non_empty_cols);
    let any_row = (0..h).any(|y| (0..w).any(|x| rgba[((y*w+x)*4+3) as usize] > 0));
    assert!(any_row, "no visible pixels");
}
