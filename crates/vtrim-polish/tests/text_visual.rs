#[test]
fn text_visual_reports() {
    let res = vtrim_polish::render::native::composite::render_text_rgba(
        "测试字幕", 72.0, [220,189,160,255], [64,60,59,255], 12.0);
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
