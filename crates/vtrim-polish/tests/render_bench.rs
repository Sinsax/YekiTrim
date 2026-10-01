//! 出片**吞吐基准**（技术探索用，不参与默认测试）：同一个工程、同一段帧，
//! 量「抽帧 / 渲染 / 每帧毫秒 / 有效帧率」，用来给任何加速改动做 A/B。
//!
//! 为什么需要它：出片链路的日志里只有分块级的「抽帧 Xs / 渲染 Ys / 编码 Zs」，
//! 而**渲染那一段把浏览器里的逐帧耗时和两端的 base64/JSON 搬运混在一起**（见
//! `docs/plan/polish-render-speed.md` §1）。要在改之前知道该改哪，就得有一个
//! 只跑渲染、不写 mp4、可重复的基准。
//!
//! 运行（Chrome 路径）：
//! ```text
//! VTEDIT_BENCH_DIR="F:/para/Code/V-Trim/target/tmp-vertprobe/proj" \
//! VTEDIT_BENCH_FRAMES=120 \
//! cargo test -p vtrim-polish --test render_bench -- --ignored --nocapture
//! ```
//! 换后端：**只有 chrome 了** —— `wgpu` 那条 GPU 链路已删除（用户拍板：GPU 出片交给独立的
//! Dhampir 底座），`VTEDIT_RENDERER=wgpu` 现在会带一句 warn 回退到 chrome。
//! 环境变量：
//! - `VTEDIT_BENCH_DIR`（必填）工程目录
//! - `VTEDIT_BENCH_FRAMES`（默认 120）渲染多少帧
//! - `VTEDIT_BENCH_T0`（默认 0.5）从第几秒开始取帧
//! - `VTEDIT_RENDERER`（默认 chrome）现在只认 `chrome`

use std::path::PathBuf;
use std::time::Instant;

use vtrim_polish::project::load_project;
use vtrim_polish::render::backend::{create_backend, RenderBackendKind};
use vtrim_polish::render::context::RenderContext;
use vtrim_polish::render::frame::extract_chunk_frames;
use vtrim_polish::render::server::{start_file_server, RenderServer};

/// 出片「字节搬运」微基准：把 `chrome.rs` 每帧的进出两趟（base64 编码 → 拼 JSON → CDP 传 →
/// serde 解析 → base64 解码）照原样复刻一遍，**不起浏览器**，所以谁都能跑，也能量 debug/release 的差距。
///
/// 为什么单独量它：日志里每帧 75ms 中只有 ~26ms 是页面自报的（JPEG 解码 + canvas + toBlob + base64），
/// 剩下 ~45~55ms 全在这一段字节搬运上，而它正是 debug 构建被放大得最狠的地方。
///
/// 运行（两条都跑，看差多少倍）：
/// ```text
/// cargo test -p vtrim-polish --test render_bench -- --ignored plumbing --nocapture
/// cargo test --release -p vtrim-polish --test render_bench -- --ignored plumbing --nocapture
/// ```
/// `VTEDIT_BENCH_FRAMES`（默认 300，与出片日志里的分块帧数同量级）、
/// `VTEDIT_BENCH_FRAME_KB`（默认 200 = 1080p JPEG q0.6 的典型体积）。
#[test]
#[ignore = "手动微基准：量 base64/JSON 搬运的每帧成本（debug vs release）"]
fn plumbing_microbench() {
    use base64::Engine as _;

    let n: usize = std::env::var("VTEDIT_BENCH_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let frame_kb: usize = std::env::var("VTEDIT_BENCH_FRAME_KB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    let frame: Vec<u8> = (0..frame_kb * 1024).map(|i| (i * 31 % 251) as u8).collect();
    let b64 = base64::engine::general_purpose::STANDARD;
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };

    // ① 发送侧：`ChromeBackend::send_frames` 的路径（每批 50 帧一次 JSON）
    let t_send = Instant::now();
    let mut sent_json_bytes = 0usize;
    for _batch in (0..n).collect::<Vec<_>>().chunks(50) {
        let batch_b64: Vec<String> = (0..50).map(|_| b64.encode(&frame)).collect();
        let json = serde_json::to_string(&batch_b64).unwrap();
        sent_json_bytes += json.len();
        std::hint::black_box(&json);
    }
    let send_s = t_send.elapsed().as_secs_f64();

    // ② 接收侧：`capture_and_read` 的路径（每批 20 帧一次解析 + 解码）
    let wire: Vec<String> = (0..n).map(|_| b64.encode(&frame)).collect();
    let t_recv = Instant::now();
    let mut decoded = 0usize;
    for chunk in wire.chunks(20) {
        let json = serde_json::to_string(chunk).unwrap();
        let parsed: Vec<String> = serde_json::from_str(&json).unwrap();
        for s in parsed {
            decoded += b64.decode(s.as_bytes()).unwrap().len();
        }
    }
    let recv_s = t_recv.elapsed().as_secs_f64();

    println!(
        "[{profile}] {} 帧 × {frame_kb}KB · 发送侧 {send_s:.2}s（{:.2} ms/帧，出网 {:.0} MB）· \
         接收侧 {recv_s:.2}s（{:.2} ms/帧，回来 {:.0} MB）· 合计 {:.2} ms/帧",
        n,
        send_s / n as f64 * 1000.0,
        sent_json_bytes as f64 / 1e6,
        recv_s / n as f64 * 1000.0,
        decoded as f64 / 1e6,
        (send_s + recv_s) / n as f64 * 1000.0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "手动吞吐基准：需要 VTEDIT_BENCH_DIR 指向真实工程（Chrome 路径还需能起 Chrome）"]
async fn render_throughput() {
    let dir = PathBuf::from(
        std::env::var("VTEDIT_BENCH_DIR").expect("set VTEDIT_BENCH_DIR to a polish project dir"),
    );
    let frames_wanted: usize = std::env::var("VTEDIT_BENCH_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    let t0: f64 = std::env::var("VTEDIT_BENCH_T0")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.5);
    let kind = std::env::var("VTEDIT_RENDERER")
        .ok()
        .and_then(|v| RenderBackendKind::parse(&v))
        .unwrap_or(RenderBackendKind::Chrome);

    let cfg = load_project(&dir).expect("加载 polish.toml");
    let video = dir.join(&cfg.source.video);
    let src = vtrim_polish::render::get_video_size(&video.to_string_lossy());
    let fit = cfg.resolve_stage(src);
    let (cw, ch) = cfg.canvas_size();
    println!(
        "工程 {} · 画布 {cw}×{ch} ({}) · 源 {src:?} · axis={} position={} bg={} blur={}px×{}",
        dir.display(),
        if cfg.canvas.is_portrait() { "竖" } else { "横" },
        fit.axis,
        fit.position,
        fit.bg,
        fit.blur_px,
        fit.blur_img_scale
    );
    println!("后端 {} · 帧数 {frames_wanted} · 起点 {t0:.2}s · fps 30", kind.as_str());

    let fps = 30usize;
    // 抽帧只要 MJPEG（RGBA 那条路是给已删除的 wgpu 后端用的）
    let need_rgba = false;
    let end = t0 + frames_wanted as f64 / fps as f64;

    // ① 抽帧（与出片同一条路径：ffmpeg → 管道）
    let t_extract = Instant::now();
    let frames = extract_chunk_frames(&video.to_string_lossy(), t0, end, fps, need_rgba)
        .expect("抽帧失败");
    let extract_s = t_extract.elapsed().as_secs_f64();
    let n = frames.len();
    println!(
        "① 抽帧 {n} 帧 {extract_s:.1}s（{:.1} ms/帧，含 {}）",
        extract_s / n as f64 * 1000.0,
        if need_rgba { "JPEG 解码为 RGBA" } else { "MJPEG 管道" }
    );

    // ② 渲染
    let mut ctx = RenderContext::new(dir.clone(), cfg.clone());
    ctx.fps = fps;
    let mut backend = create_backend(kind, &dir, &cfg).expect("创建后端");
    if kind == RenderBackendKind::Chrome {
        // 页面要访问的 HTTP server 必须跑在 runtime 上，而抓帧是整段同步的 → 渲染挪到 blocking 线程
        let html = RenderServer::build_html(&dir);
        let port = start_file_server(dir.clone(), Some(html)).await.unwrap();
        ctx.server_url = Some(format!("http://127.0.0.1:{}/?capture=1", port));
    }
    backend.prepare(&ctx).expect("prepare");

    let t_render = Instant::now();
    let out = tokio::task::spawn_blocking(move || backend.render_chunk(&frames))
        .await
        .expect("join")
        .expect("render_chunk 失败");
    let render_s = t_render.elapsed().as_secs_f64();
    assert_eq!(out.len(), n, "返回帧数不符（页面里抛异常会让整块 chunk 失败）");

    let bytes: usize = out.iter().map(|f| f.len()).sum();
    println!(
        "② 渲染 {n} 帧 {render_s:.1}s → {:.1} ms/帧 · {:.1} fps · 回传 {:.1} MB（{:.0} KB/帧）",
        render_s / n as f64 * 1000.0,
        n as f64 / render_s,
        bytes as f64 / 1e6,
        bytes as f64 / n as f64 / 1024.0
    );
    println!(
        "③ 合计（抽帧+渲染）{:.1}s → {:.1} fps 有效 → 相当于 {:.2}× 实时（不含编码/合流/去静音）",
        extract_s + render_s,
        n as f64 / (extract_s + render_s),
        n as f64 / (extract_s + render_s) / fps as f64
    );
}
