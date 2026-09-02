#[test]
fn dump_text_png() {
    for sw in [2.0, 12.0] {
        let res = vtrim_polish::render::native::composite::render_text_rgba(
            "测试字幕", 72.0, [255,255,255,255], [0,0,0,255], sw);
        let (rgba, w, h, _) = res.unwrap();
        let vis = rgba.chunks(4).filter(|p| p[3] > 0).count();
        eprintln!("sw={} size={}x{} vis={}", sw, w, h, vis);
    }
}
