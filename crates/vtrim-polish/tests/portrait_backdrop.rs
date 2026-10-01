//! 竖屏「模糊衬底」的**像素实证**：主画面被特效（震动 / 画中画 / 手持摇摆 / 运镜）
//! 挪开时，让出来的那条边必须是**当前帧的模糊衬底**，不能是纯黑。
//!
//! 为什么必须是像素级、而不是"看代码里有没有那个函数"：
//! 出片路径上"衬底"这件事由**转译器 + 底座**共同决定（参照实现在 `templates/index.html`
//! 的 `drawStageBackdrop`，转译器逐条对着它重写），只把 `stage.bg` 消费在 DOM 预览上，
//! 表现就是「预览里有模糊底、成片被 shake 挪开露纯黑」—— 不报错、不崩溃，
//! 只有逐帧像素能发现（见 `docs/plan/polish-vertical-canvas.md` §八.13 / §九.7）。
//!
//! ⚠️ **2026-10-01 起探针不再起浏览器**：它当年驱动本仓的 headless Chrome 逐帧腿
//! （`ChromeBackend`），那条腿已整条删除。现在跑的是**出片真正的后端**（底座），
//! 变异打在**转译器**上 —— 改 `templates/index.html` 对底座那条腿一个像素都不影响
//! （转译器根本不读它），A/B 会永远"看不出差别"而判据照旧全绿。细节见 `tests/common/mod.rs`。
//!
//! 本测试跑 **A/B 两次**，第二次把转译器里画衬底的那段关掉（等价于改造前的实现）：
//! - A（真转译器）：四角亮度必须 > 阈值（= 画上了衬底）
//! - B（变异体）：在**真露出衬底的那几帧**上，四角必须明显更暗
//!   （= 证明这个探针**真的能看见**这个缺陷）
//! 只断言 A 不断言 B，等于不知道探针是否空转；但 B 只能断在"衬底真露出来"的帧上 ——
//! 判据与理由见下面 `EXPOSED_BACKDROP_MAX_LUMA`。
//!
//! 运行（需要底座二进制 + Node 转译器）：
//! ```text
//! VTEDIT_BACKDROP_DIR="F:/para/Code/V-Trim/target/tmp-vertprobe/proj" \
//! VTEDIT_BACKDROP_OUT="F:/para/Code/V-Trim/target/tmp-vertprobe/out" \
//! cargo test -p vtrim-polish --test portrait_backdrop -- --ignored --nocapture
//! ```
//! - `VTEDIT_BACKDROP_T`（秒，默认 `1.5`）= 探针时刻，要落在**主画面被挪开**的那一段里；
//! - `VTEDIT_BACKDROP_SPAN`（秒，默认 `1.0`）= 以 T 为中心渲染多长一段（30fps → 帧数）；
//! - `VTEDIT_BACKDROP_ANCHOR`（默认 = 转译器里那道衬底闸）= 锚点必须真实存在且只出现一次，
//!   变异就打在它的**调用点**上（转译器改了措辞时用它兜，不必改探针源码）。
//!
//! ⚠️ 跑完先看输出的「真露底 n / 总帧」：**n = 0 就换个 `T`**（这一段里主画面把四角全盖住了，
//! 衬底压根没露出来，A/B 比的是同一份像素，什么都看不出来）。
//! ⚠️ 探针工程请用**副本**：渲染产物（`_temp_video.mp4` / `_dhampir_project.json`）会落在工程里。
//!
//! 探针工程可以用 `target/tmp-vertprobe/proj`（内置生成脚本没有，是手里那个竖屏工程的最小复刻：
//! 720×1280 源 + 画中画 + 全特效 + 一条 30 字必折行的字幕 + 两条弹幕），也可以指向真实工程
//! —— 但真实工程得挑一个主画面真被挪开的时刻，否则四角本来就是画面本身。
//!
//! ⚠️ 这个"画中画"**不许**再用 `[[segments.events]] type="camera"` 做（探针工程最初是那样写的）：
//! 竖素材的运镜已被门控成 `zoom=1.0 / handheld=0`，运镜再也挪不开主画面 —— 靠它露底
//! 就是靠在一个**刚被修掉的缺陷**上（实测：门控落地后 30 帧里只剩 1 帧真露底，探针近乎空转）。
//! 现在改由 `[stage] scale` 结构性露底，运镜事件留着只为跑那条被门控的代码路径。

mod common;

use std::path::PathBuf;

use vtrim_polish::project::load_project;

/// 「有衬底」判定阈值：衬底是当前帧 `brightness(0.5)` 的模糊拷贝，
/// 只要源画面不是纯黑，四角亮度必然远高于它。测试素材是亮色（#FF7040 + 白色方块）。
const BACKDROP_MIN_LUMA: f64 = 20.0;
/// A/B 里"衬底到底贡献了多少亮度"的下限：同一帧、只差衬底那一段代码，
/// 真转译器的四角必须比变异体（不画衬底）**亮这么多**。
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
/// 变异体那条边什么都不画，而画布每帧都清过 → 透明编码成纯黑（实测 0.0）。
///
/// 阈值 12 与上面 ① 是配套的：① 要求真转译器四角 > `BACKDROP_MIN_LUMA`(20)，而四角被盖住的帧里
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

/// 衬底那一段代码在转译器里的锚点。
///
/// ⚠️ **这里必须写转译器里真实存在、且只出现一次的那一行**。
/// 老锚点是 `drawStageBackdrop(`（照抄 `templates/index.html` 的函数名）—— 可转译器
/// **根本不调这个函数**：它把衬底内联重写成 `backdropTracks`，那个名字只在**注释**里出现过一次。
/// 于是 `matches(..).count() == 0` ⇒ 变异体 == 原件 ⇒ A/B 比的是同一份产物，
/// 判据变成"自己和自己比"（正是这段注释下面警告的那种静默失效）。
/// 现在锚在**真实的那道闸**上（`if (BACKDROP_ENABLED && STAGE_BG === 'blur') {`，恰好 1 次）。
fn backdrop_guard() -> String {
    std::env::var("VTEDIT_BACKDROP_ANCHOR")
        .unwrap_or_else(|_| "if (BACKDROP_ENABLED && STAGE_BG === 'blur') {".to_string())
}

/// 转译器里那道"衬底总开关"。默认 `false` ⇒ 真实产物**本来就不画衬底**，
/// 所以只想比"默认行为 vs 关掉衬底"是没有意义的（两边都不画）。
/// 探针必须**先把它打开**，再比"打开后画了没"。
const BACKDROP_FLAG: &str = "const BACKDROP_ENABLED = false;";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "手动像素探针：需要 VTEDIT_BACKDROP_DIR 指向真实竖屏工程，且本机能跑底座（二进制 + node 转译器）"]
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

    // 变异体：把衬底那段关掉。
    //
    // ⚠️ 变异打在**转译器**上、不是 `templates/index.html` 上：底座那条腿由转译器出工程，
    // 它**根本不读** `index.html`（那是已作废的 canvas 参照实现）。改 HTML 会让 A/B 比同一份
    // 产物、判据静默失效 —— 那正是"探针在空转"最坏的一种。
    // A = **把总开关打开**的真转译器（衬底应该画出来）。
    // B = 同一个转译器、但把整道衬底闸拆掉（`if (…)` → `if (false)`）。
    //
    // ⚠️ 两次都要动那个开关：只变异 A 的话，A 和 B 都不画衬底 —— 又变成自己比自己。
    let on = common::mutated_translator(
        "backdrop-on",
        &[(BACKDROP_FLAG, "const BACKDROP_ENABLED = true;")],
    )
    .unwrap_or_else(|e| panic!("无法打开衬底总开关：{e:#}"));

    let anchor = backdrop_guard();
    let off = common::mutated_translator(
        "backdrop-off",
        &[
            (BACKDROP_FLAG, "const BACKDROP_ENABLED = true;"),
            (anchor.as_str(), "if (false) {"),
        ],
    )
    .unwrap_or_else(|e| panic!("无法构造变异转译器：{e:#}"));

    println!("A/B 里的 A = 总开关**打开**的转译器（衬底应当画出来），渲染 [{start:.2}, {end:.2}) 每帧");
    let real = common::render_range(&dir, Some(&on), start, end).unwrap();
    common::save_frames(&real, Some(&out_dir), "real").unwrap();

    println!(
        "A/B 里的 B = 同一个转译器、但把衬底闸（`{anchor}`）改成 `if (false)`，同样渲染 [{start:.2}, {end:.2})"
    );
    let mutated = common::render_range(&dir, Some(&off), start, end).unwrap();
    common::save_frames(&mutated, Some(&out_dir), "mutant").unwrap();

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
    println!("  [真转译器] {}", brief(&real));
    println!("  [变异体]   {}", brief(&mutated));

    let min_corner = |jpeg: &[u8]| -> f64 {
        stats(jpeg).corners.iter().cloned().fold(f64::MAX, f64::min)
    };

    // ① 真转译器：整段每一帧的**每个**角都得是衬底（不是黑）
    for (jpeg, tt) in &real {
        let s = stats(jpeg);
        for (i, c) in s.corners.iter().enumerate() {
            assert!(
                *c > BACKDROP_MIN_LUMA,
                "t={tt:.2}s 第 {i} 个角亮度 {c:.1} ≤ {BACKDROP_MIN_LUMA} —— 主画面挪开处露的是黑色，不是模糊衬底"
            );
        }
    }
    // ② A/B 自证：同帧只差衬底那一段，**在真露底的那几帧**变异体的四角必须明显更暗
    //    （否则探针看不见缺陷、在空转）+ 整段至少得有一帧真露底。判据为什么只落在露底帧上、
    //    为什么不能逐帧都要求见 `EXPOSED_BACKDROP_MAX_LUMA` 的注释。
    assert_eq!(
        real.len(),
        mutated.len(),
        "A/B 两边的帧数必须一致才能逐帧比（真转译器 {} / 变异体 {}）",
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
            "t={tt:.2}s（露底帧）真转译器最暗角 {r:.1} vs 变异体 {m:.1}，只差 {:.1} < {AB_DELTA_MIN_LUMA} —— 衬底没起作用",
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
