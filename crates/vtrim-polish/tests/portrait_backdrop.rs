//! 竖屏「模糊衬底」的**像素实证**：主画面被特效（震动 / 画中画 / 手持摇摆 / 运镜）
//! 挪开时，让出来的那条边必须是**当前帧的模糊衬底**，不能是纯黑。
//!
//! 为什么必须是像素级、而不是"看代码里有没有那个函数"：
//! 出片路径是 canvas `renderFrame` 逐帧绘制，DOM 的 `#bg-video-wrap` **完全不参与导出**。
//! 只把 `stage.bg` 消费在 DOM 上，表现就是「预览里有模糊底、成片被 shake 挪开露纯黑」——
//! 不报错、不崩溃，只有逐帧像素能发现（见 `docs/plan/polish-vertical-canvas.md` §八.13 / §九.7）。
//!
//! 本测试跑 **A/B 两次**，第二次把服务给页面的 HTML 里的 `STAGE_BG` 强制改成 `'none'`
//! （等价于改造前的实现）：
//! - A（真模板）：四角亮度必须 > 阈值（= 画上了衬底）
//! - B（变异体）：在**真露出衬底的那几帧**上，四角必须明显更暗
//!   （= 证明这个探针**真的能看见**这个缺陷）
//! 只断言 A 不断言 B，等于不知道探针是否空转；但 B 只能断在"衬底真露出来"的帧上 ——
//! 判据与理由见下面 `EXPOSED_BACKDROP_MAX_LUMA`。
//!
//! 运行（需要能起 Chrome 的环境）：
//! ```text
//! VTEDIT_BACKDROP_DIR="F:/para/Code/V-Trim/target/tmp-vertprobe/proj" \
//! VTEDIT_BACKDROP_OUT="F:/para/Code/V-Trim/target/tmp-vertprobe/out" \
//! cargo test -p vtrim-polish --test portrait_backdrop -- --ignored --nocapture
//! ```
//! - `VTEDIT_BACKDROP_T`（秒，默认 `1.5`）= 探针时刻，要落在**主画面被挪开**的那一段里；
//! - `VTEDIT_BACKDROP_SPAN`（秒，默认 `1.0`）= 以 T 为中心渲染多长一段（30fps → 帧数）。
//! - 跑完先看输出的「真露底 n / 总帧」：**n = 0 就换个 `T`**（这一段里主画面把四角全盖住了，
//!   衬底压根没露出来，A/B 比的是同一份像素，什么都看不出来）。
//!   这一段**每一帧都要画出来**，所以它同时是"逐帧绘制路径"的冒烟测试：
//!   页面里任何一处 `ReferenceError`（实测踩过：字幕分支里一个没声明的 `fs`
//!   → `Chunk: Render error: fs is not defined`，**所有带字幕的片段都导不出来**）
//!   都会让整块 chunk 失败、这里帧数对不上。
//!
//! 探针工程可以用 `target/tmp-vertprobe/proj`（内置生成脚本没有，是手里那个竖屏工程的最小复刻：
//! 720×1280 源 + 画中画 + 全特效 + 一条 30 字必折行的字幕 + 两条弹幕），也可以指向真实工程
//! —— 但真实工程得挑一个主画面真被挪开的时刻，否则四角本来就是画面本身。
//!
//! ⚠️ 这个"画中画"**不许**再用 `[[segments.events]] type="camera"` 做（探针工程最初是那样写的）：
//! 竖素材的运镜已被模板门控成 `zoom=1.0 / handheld=0`，运镜再也挪不开主画面 —— 靠它露底
//! 就是靠在一个**刚被修掉的缺陷**上（实测：门控落地后 30 帧里只剩 1 帧真露底，探针近乎空转）。
//! 现在改由 `[stage] scale` 结构性露底，运镜事件留着只为跑那条被门控的代码路径。

use std::path::{Path, PathBuf};

use vtrim_polish::project::load_project;
use vtrim_polish::render::backend::FrameRenderer;
use vtrim_polish::render::chrome::ChromeBackend;
use vtrim_polish::render::context::RenderContext;
use vtrim_polish::render::frame::extract_chunk_frames;
use vtrim_polish::render::server::{RenderServer, start_file_server};

/// 「有衬底」判定阈值：衬底是当前帧 `brightness(0.5)` 的模糊拷贝，
/// 只要源画面不是纯黑，四角亮度必然远高于它。测试素材是亮色（#FF7040 + 白色方块）。
const BACKDROP_MIN_LUMA: f64 = 20.0;
/// A/B 里"衬底到底贡献了多少亮度"的下限：同一帧、只差 `STAGE_BG`，
/// 真模板的四角必须比变异体（不画衬底）**亮这么多**。
///
/// ⚠️ 为什么不用"变异体必须纯黑（<8）"：flash / noise 这类**全屏白覆盖**是画在
/// 视频层之后的，它会把变异体那圈黑也一起抬亮（实测 noise 淡出期 0 → 14）。
/// 相对判据不受这类覆盖层影响，而且正是"探针看得见这个缺陷"的直接表述。
const AB_DELTA_MIN_LUMA: f64 = 40.0;

/// 「这一帧真的把衬底露出来了」的判定：变异体（不画衬底）的最暗角必须接近它下面的底色。
///
/// ⚠️ 为什么 A/B 判据需要这个前置条件：四角被主画面盖住的帧里，A/B 比的就是**同一份画面像素**
/// （衬底压根没露出来），差值天然 ≈ 0 —— 实测同一段 30 帧里，主画面只在特效把它推开的一两帧
/// 才让出边角。把这类帧也要求成"变异体必须更暗"，等于要求**每一帧**都露底，那是拿判据去赌
/// 运镜/特效的轨迹，不是判据。所以：
/// - 「至少有一帧露底」= 探针有效性的下限（否则整段都在空转）；
/// - 「每个露底帧的 A/B 差 ≥ `AB_DELTA_MIN_LUMA`」= 缺陷已修的直接表述。
///
/// 变异体那条边什么都不画，而画布每帧都 `clearRect` 过 → 透明编码成纯黑（实测 0.0）。
///
/// 阈值 12 与上面 ① 是配套的：① 要求真模板四角 > `BACKDROP_MIN_LUMA`(20)，而四角被盖住的帧里
/// A/B 是同一份像素 → 变异体那角也 ≳ 20（> 12）→ 被跳过；反过来只有真缺了衬底的角才会掉到 12
/// 以下。全屏覆盖层（`noise`）把缺衬底的角抬到 13~21 的帧同样被跳过 —— 它们不提供证据、
/// 也不需要（只要窗口里还剩得下能判定的帧）。
const EXPOSED_BACKDROP_MAX_LUMA: f64 = 12.0;

fn luma(r: u8, g: u8, b: u8) -> f64 {
    0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64
}

/// 取四个角的平均亮度 + 整帧平均值（边角 24×24 区域，避开 JPEG 边缘振铃）。
struct Stats {
    corners: [f64; 4],
    all: f64,
}

fn stats(jpeg: &[u8]) -> Stats {
    let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
        .expect("解码渲染结果 JPEG")
        .to_rgb8();
    let (w, h) = (img.width(), img.height());
    let patch = 24.min(w / 8).min(h / 8);
    let corners_at = [(0u32, 0u32), (w - patch, 0), (0, h - patch), (w - patch, h - patch)];
    let mut corners = [0.0f64; 4];
    for (i, (x0, y0)) in corners_at.iter().enumerate() {
        let mut sum = 0.0;
        let mut n = 0.0;
        for y in *y0..(*y0 + patch) {
            for x in *x0..(*x0 + patch) {
                let p = img.get_pixel(x, y);
                sum += luma(p[0], p[1], p[2]);
                n += 1.0;
            }
        }
        corners[i] = sum / n;
    }
    let mut sum = 0.0;
    for p in img.pixels() {
        sum += luma(p[0], p[1], p[2]);
    }
    Stats { corners, all: sum / (w as f64 * h as f64) }
}

/// 起服务 + Chrome 渲染 `[start, end)` 这一段每一帧（**一次 ffmpeg 抽帧 + 一次 Chrome**），
/// 返回 (jpeg, t)。索引从 `start*fps` 起算，与出片时的帧号一致。
async fn render_range(
    dir: &Path,
    html: String,
    start: f64,
    end: f64,
    out_dir: &Path,
    label: &str,
) -> anyhow::Result<Vec<(Vec<u8>, f64)>> {
    let cfg = load_project(dir)?;
    let fps = 30usize;
    let video = dir.join(&cfg.source.video);

    let frames = extract_chunk_frames(&video.to_string_lossy(), start, end, fps, false)?;
    anyhow::ensure!(!frames.is_empty(), "[{start},{end}) 抽帧为空");

    let port = start_file_server(dir.to_path_buf(), Some(html)).await?;
    let url = format!("http://127.0.0.1:{}/?capture=1", port);

    let mut ctx = RenderContext::new(dir.to_path_buf(), cfg.clone());
    ctx.fps = fps;
    ctx.server_url = Some(url);
    let _ = &ctx; // 贴纸帧数据（GIF）留给出片链路自己准备；探针只关心衬底像素 + 不抛异常
    //
    // ⚠️ 渲染必须挪到 blocking 线程：Chrome 抓帧是**整段同步**的（等页面回写进度），
    // 而本测试页面要访问的那个 HTTP server 就跑在同一个 tokio runtime 上 ——
    // 直接 await 里同步跑，runtime 线程被占死、server 永远不回包，
    // 页面 `load` 事件等不到，报一个和根因无关的 `The event waited for never came`。
    // 生产路径同理（`pipeline.rs` 用 spawn_blocking / 专用线程 + current-thread runtime）。
    let frame_times: Vec<f64> = frames.iter().map(|f| f.t).collect();
    let dir_b = dir.to_path_buf();
    let cfg_b = cfg.clone();
    let jpegs = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Vec<u8>>> {
        let mut backend = ChromeBackend::new(&dir_b, &cfg_b)?;
        backend.prepare(&ctx)?;
        backend.render_chunk(&frames)
    })
    .await??;

    anyhow::ensure!(
        jpegs.len() == frame_times.len(),
        "{label}: 引擎回了 {} 帧、期望 {} 帧（页面里只要有异常就整块 chunk 失败）",
        jpegs.len(),
        frame_times.len()
    );

    std::fs::create_dir_all(out_dir)?;
    let mut out = Vec::new();
    for (i, (jpeg, t)) in jpegs.into_iter().zip(frame_times).enumerate() {
        let path = out_dir.join(format!("{label}_{i:04}_t{t:.2}.jpg"));
        std::fs::write(&path, &jpeg)?;
        out.push((jpeg, t));
    }
    println!("  [{label}] 写出 {} 帧 → {}", out.len(), out_dir.display());
    Ok(out)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "手动像素探针：需要 VTEDIT_BACKDROP_DIR 指向真实竖屏工程，且本机能起 Chrome"]
async fn portrait_backdrop_is_not_black() {
    let dir = PathBuf::from(
        std::env::var("VTEDIT_BACKDROP_DIR")
            .expect("set VTEDIT_BACKDROP_DIR to a polish project dir"),
    );
    let out_dir = std::env::var("VTEDIT_BACKDROP_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target/tmp-vertprobe/out"));
    let t: f64 = std::env::var("VTEDIT_BACKDROP_T")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.5);
    // 探针窗口：以 T 为中心各取 `span/2` 秒（默认 ±0.5s = 30 帧 @30fps）。
    // **区间里每一帧都要能画出来** —— 页面里任何一处 `ReferenceError`（曾经是字幕分支里
    // 一个没声明的 `fs`）都会让整块 chunk 失败，所以这一段顺带是全帧路径的冒烟测试。
    let span: f64 = std::env::var("VTEDIT_BACKDROP_SPAN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);
    let (start, end) = ((t - span / 2.0).max(0.0), t + span / 2.0);

    // 前置事实：这个工程解析出来的 `bg` 必须**不是** none —— 否则衬底本来就不该画
    // （横画布铺满 = 改造前的零回归路径），本测试对那种工程没有意义。
    let cfg = load_project(&dir).expect("加载 polish.toml");
    let src = vtrim_polish::render::get_video_size(&dir.join(&cfg.source.video).to_string_lossy());
    let fit = cfg.resolve_stage(src);
    println!(
        "工程 {} · 画布 {:?} · 源 {:?} · axis={} position={} bg={} blur={}px×{}",
        dir.display(),
        cfg.canvas.size(),
        src,
        fit.axis,
        fit.position,
        fit.bg,
        fit.blur_px,
        fit.blur_img_scale
    );
    assert_ne!(fit.bg, "none", "该工程解析出的 bg=none，衬底本就不该画，请换个工程/画布方向");

    // A：真模板，渲染 [start, end) 整段
    let html = RenderServer::build_html(&dir);
    assert!(
        html.contains("drawStageBackdrop"),
        "出片页面里没有背景层消费点 —— 这正是「预览有模糊底、成片纯黑」的成因"
    );
    println!("A/B 里的 A = 真模板，渲染 [{start:.2}, {end:.2}) 每帧");
    let real = render_range(&dir, html.clone(), start, end, &out_dir, "real").await.unwrap();

    // B：变异体 —— 把 `STAGE_BG` 钉成 'none'（等价于改造前：DOM 有衬底、出片没有）。
    // **同一段每一帧都渲染**，这样 A/B 是逐帧可比的（只有 `STAGE_BG` 一个变量）。
    let marker = "var STAGE_BG=STAGE.bg||'none';";
    assert!(html.contains(marker), "模板里找不到 STAGE_BG 的赋值点（探针需要它做变异）");
    let mutated_html = html.replace(marker, "var STAGE_BG='none';");
    println!("A/B 里的 B = 变异体（STAGE_BG 强制 none），同样渲染 [{start:.2}, {end:.2})");
    let mutated = render_range(&dir, mutated_html, start, end, &out_dir, "mutant").await.unwrap();

    let brief = |frames: &[(Vec<u8>, f64)]| -> String {
        let mut out = Vec::new();
        for (jpeg, tt) in frames.iter().take(6) {
            let s = stats(jpeg);
            out.push(format!(
                "t={tt:.2}s 四角{:?} 均值{:.1}",
                s.corners.map(|v| (v * 10.0).round() / 10.0),
                s.all
            ));
        }
        out.push(format!("…共 {} 帧", frames.len()));
        out.join(" | ")
    };
    println!("  [真模板] {}", brief(&real));
    println!("  [变异体] {}", brief(&mutated));

    let min_corner = |jpeg: &[u8]| -> f64 {
        stats(jpeg).corners.iter().cloned().fold(f64::MAX, f64::min)
    };

    // ① 真模板：整段每一帧的**每个**角都得是衬底（不是黑）
    for (jpeg, tt) in &real {
        let s = stats(jpeg);
        for (i, c) in s.corners.iter().enumerate() {
            assert!(
                *c > BACKDROP_MIN_LUMA,
                "t={tt:.2}s 第 {i} 个角亮度 {c:.1} ≤ {BACKDROP_MIN_LUMA} —— 主画面挪开处露的是黑色，不是模糊衬底"
            );
        }
    }
    // ② A/B 自证：同帧只差一个 `STAGE_BG`，**在真露底的那几帧**变异体的四角必须明显更暗
    //    （否则探针看不见缺陷、在空转）+ 整段至少得有一帧真露底。判据为什么只落在露底帧上、
    //    为什么不能逐帧都要求见 `EXPOSED_BACKDROP_MAX_LUMA` 的注释。
    assert_eq!(
        real.len(),
        mutated.len(),
        "A/B 两边的帧数必须一致才能逐帧比（真模板 {} / 变异体 {}）",
        real.len(),
        mutated.len()
    );
    let mut exposed: Vec<(f64, f64)> = Vec::new(); // (时刻, A/B 最暗角之差)
    for ((real_jpeg, tt), (mut_jpeg, _)) in real.iter().zip(mutated.iter()) {
        let (r, m) = (min_corner(real_jpeg), min_corner(mut_jpeg));
        if m > EXPOSED_BACKDROP_MAX_LUMA {
            continue; // 四角还是主画面本身（衬底没露出来）→ 这一帧 A/B 不可比
        }
        assert!(
            r - m >= AB_DELTA_MIN_LUMA,
            "t={tt:.2}s（露底帧）真模板最暗角 {r:.1} vs 变异体 {m:.1}，只差 {:.1} < {AB_DELTA_MIN_LUMA} —— 衬底没起作用",
            r - m
        );
        exposed.push((*tt, r - m));
    }
    assert!(
        !exposed.is_empty(),
        "整段 [{start:.2}, {end:.2}) 没有一帧把主画面挪开（变异体四角最暗 {:.1}，全是画面本身）—— \
         探针在空转，把 VTEDIT_BACKDROP_T 挪到主画面真被推出画布的时刻（或让探针工程结构性露底）",
        mutated.iter().map(|(j, _)| min_corner(j)).fold(f64::MAX, f64::min)
    );
    let worst_delta = exposed.iter().map(|(_, d)| *d).fold(f64::MAX, f64::min);
    let real_min = real.iter().map(|(j, _)| min_corner(j)).fold(f64::MAX, f64::min);
    println!(
        "✓ {} 帧全部渲染成功、四角都是衬底（最暗角 {:.1}）；其中 {} / {} 帧真露底（t={:.2}~{:.2}s），\
         这些帧的 A/B 差最小 {:.1} —— 探针有效且缺陷已修",
        real.len(),
        real_min,
        exposed.len(),
        real.len(),
        exposed[0].0,
        exposed[exposed.len() - 1].0,
        worst_delta
    );
}
