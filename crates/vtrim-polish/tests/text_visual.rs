use vtrim_render_core::composite::{self, FontSpec};

fn font() -> FontSpec {
    let bytes = std::fs::read("C:/Windows/Fonts/msyh.ttc")
        .or_else(|_| std::fs::read("crates/vtrim-core/resources/lemi-bobo.ttf"))
        .expect("test font");
    composite::set_font_bytes("LXGW WenKai", Some(bytes));
    FontSpec::new("LXGW WenKai")
}

#[test]
fn text_visual_reports() {
    let res = vtrim_render_core::composite::render_text_rgba(
        "测试字幕", 72.0, [220,189,160,255], [64,60,59,255], 12.0, &font());
    let (rgba, w, h, _) = res.unwrap();
    eprintln!("size {}x{}", w, h);
    let mut stroke_px=0;
    let mut fill_px=0;
    for p in rgba.chunks(4) {
        if p[3] > 0 {
            if p[0]==64 && p[1]==60 && p[2]==59 { stroke_px+=1; } else { fill_px+=1; }
        }
    }
    eprintln!("stroke_px={} fill_px={}", stroke_px, fill_px);
}
