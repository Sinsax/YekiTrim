//! 手动保真度采样：对真实 clip 渲染若干代表性帧并输出 JPEG 供人工比对。
//!
//! 运行方式（不影响用户的 export.mp4，只写 OUT 目录）：
//! ```text
//! VTEDIT_SAMPLE_DIR="F:/.../clip-03" \
//! VTEDIT_SAMPLE_OUT="F:/tmp/wgpu-samples" \
//! cargo test -p vtrim-polish --features wgpu-render \
//!   --test render_sample -- --ignored --nocapture
//! ```
#![cfg(feature = "wgpu-render")]

use vtrim_polish::project::load_project;
use vtrim_polish::render::frame::extract_chunk_frames;
use vtrim_polish::render::native::WgpuBackend;
use vtrim_polish::render::{backend::FrameRenderer, context::RenderContext};

fn sample_times(duration: f64) -> Vec<f64> {
    // 覆盖开场（intro black / 渐变 overlay / 贴纸弹出）与中后段
    let mut times = vec![0.05, 0.5, 1.5, 3.0];
    if duration > 30.0 {
        times.push(24.0);
    }
    if duration > 50.0 {
        times.push(duration - 1.0);
    }
    times
}

#[test]
#[ignore = "手动采样：需要 VTEDIT_SAMPLE_DIR / VTEDIT_SAMPLE_OUT 指向真实 clip"]
fn render_sample_frames() {
    let Some(dir) = std::env::var_os("VTEDIT_SAMPLE_DIR").map(std::path::PathBuf::from) else {
        panic!("set VTEDIT_SAMPLE_DIR");
    };
    let out_dir = std::env::var_os("VTEDIT_SAMPLE_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("target/wgpu-samples"));
    std::fs::create_dir_all(&out_dir).unwrap();

    let config = load_project(&dir).expect("load polish.toml");
    let video_path = dir.join(&config.source.video);
    let duration = config.duration.unwrap_or(f64::MAX);

    let mut backend = WgpuBackend::new(&dir, &config).expect("backend");
    let mut ctx = RenderContext::new(dir.clone(), config.clone());
    if let Some((vw, vh)) = vtrim_polish::render::get_video_size(&video_path.to_string_lossy()) {
        if vh > vw {
            ctx.vertical = true;
        }
    }

    for t in sample_times(duration.min(f64::MAX)) {
        if t > duration {
            continue;
        }
        // 每个采样点单独抽 1 帧（0.1s 窗口保证抽到目标时刻）
        let frames = extract_chunk_frames(&video_path.to_string_lossy(), t, t + 0.05, 30, true)
            .expect("extract frame");
        assert!(!frames.is_empty(), "no frame at t={t}");
        let frames: Vec<_> = frames
            .into_iter()
            .map(|mut f| {
                f.t = t;
                f
            })
            .collect();
        backend.prepare(&ctx).unwrap();
        let jpegs = backend.render_chunk(&frames).expect("render frame");
        let path = out_dir.join(format!("frame_{t:.2}.jpg"));
        std::fs::write(&path, &jpegs[0]).unwrap();
        println!("wrote {}", path.display());
    }
}
