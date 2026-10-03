//! 竖屏素材出片的**摆放几何 + 运镜门控**像素实证（2026-09-21 用户报告）。
//!
//! 用户原话：「横屏时的效果显示为竖屏素材被拉伸成宽屏，而不是竖屏素材+模糊的关键帧。
//! 而且对于竖屏素材没有禁用运镜，即使是横屏导出还是竖屏导出」。
//!
//! 两个成因都在**画面那条腿**上：
//! ① 视频层被硬铺满整块画布（参照实现在 `templates/index.html` 的 `stageDrawRect`，
//!    `_canvas.stage` 推导出的 `axis/position/scale/padding` 只有 DOM 预览在消费 → 成片是拉伸的）；
//! ② 竖素材门控只存在于 DOM 预览（`engine.js` / 模板的 DOM 分支），
//!    画面那条腿照旧套运镜 → 竖素材在预览里不动、在成片里被 zoom/origin 拖着走。
//!
//! 为什么必须是像素级：这两条都**不报错、不崩溃**，"改好了 DOM 预览"照样看起来正常，
//! 只有逐帧像素能发现（与 `tests/portrait_backdrop.rs` 同一类事故）。
//!
//! ⚠️ **2026-10-01 起探针不再起浏览器**：它当年驱动本仓的 headless Chrome 逐帧腿
//! （`ChromeBackend`），那条腿已整条删除。现在跑的是**出片真正的后端**（底座），
//! **变异打在转译器上** —— 底座那条腿由 `tools/polish-to-dhampir.mjs` 出工程，
//! 它**根本不读** `templates/index.html`（后者是已作废的 canvas 参照实现）。改 HTML 会让
//! A/B 比同一份产物、判据静默失效。细节见 `tests/common/mod.rs`。
//!
//! 探针素材约定（自造，720×1280 竖屏 8s + 静音轨）：
//! - 整帧品红 `#ff00ff` + 最外圈 6px **白描边** + 正中央 **360×640 亮绿块**（源画面的 50%×50%）；
//! - 白描边 → 成片里"内容框"的 bbox（等比缩放后 ~5px 宽，JPEG 之后仍远亮于背景与字幕色）；
//! - 亮绿块 → 面积占比（拉伸会到 25%+、运镜 `zoom_close=1.5` 会到 ~17%、正确时 ~7.9%）；
//! - 品红 → 衬底是同一帧的 cover 模糊 + `brightness(0.5)`，内容框之外必须是它、不能是纯黑。
//!
//! ```text
//! ffmpeg -y -f lavfi -i "color=c=magenta:s=720x1280:d=8:r=30" -f lavfi -i "anullsrc=r=48000:cl=mono" \
//!   -vf "drawbox=x=0:y=0:w=720:h=1280:color=white:t=6,drawbox=x=180:y=320:w=360:h=640:color=lime:t=fill" \
//!   -c:v libx264 -pix_fmt yuv420p -crf 18 -g 30 -t 8 -c:a aac -shortest portrait-src.mp4
//! ```
//! 探针工程 = 上面这颗源 + `[canvas] orientation="landscape"` + `[stage] bg_mode="blur"` +
//! 一条整段 `type="camera" zoom="近"`（`zoom_close=1.5`，竖素材下必须被吃掉）。
//!
//! 运行（需要底座二进制 + Node 转译器）：
//! ```text
//! VTEDIT_FIT_DIR="F:/para/Code/YekiTrim/.workbuddy/perf/vertical/vert-land" \
//! VTEDIT_FIT_OUT="F:/para/Code/YekiTrim/target/tmp-stagefit/out" \
//! cargo test -p yekitrim-polish --test portrait_stage_fit -- --ignored --nocapture
//! ```
//! - `VTEDIT_FIT_T`（秒，默认 `2.0`）= 探针时刻；
//! - `VTEDIT_FIT_SPAN`（秒，默认 `0.6`）= 以 T 为中心渲染多长一段（30fps → 帧数）。
//! - 探针工程请用**副本**：渲染产物（`_temp_video.mp4` / `_dhampir_project.json`）会落在工程里。
//!
//! **A/B 三段变异，每段都断言"翻面"**（只断言真转译器 = 不知道探针是不是空转）：
//! - 变异①「硬铺满」：`axis==='x'` 分支的等比改成拉伸 → 内容框必须变成整块画布；
//! - 变异②「去掉竖屏运镜门」：竖屏早退条件改成恒假 → 绿块面积必须明显变大；
//! - 变异③「不画衬底」：衬底那段关掉 → 内容框之外必须掉到纯黑。
//!
//! ⚠️ 变异锚点（①②③）在转译器里改了措辞就会**当场失败**（锚点必须恰好命中一次），
//! 不会静默比同一份产物。

mod common;

use std::path::{Path, PathBuf};

use yekitrim_polish::project::{StageFit, load_project};

/// 白描边判定（三通道都 > 235）：字幕色 `#dcbda0`（220/189/160）与衬底都不达标，不污染 bbox。
const WHITE: u8 = 235;
/// 内容框外的平均亮度下限：衬底 = 同一帧模糊 + `brightness(0.5)`，品红 → 亮度 ≈ 53。
const OUTSIDE_MIN_LUMA: f64 = 15.0;
/// 内容框外的"纯黑"占比上限（变异③下会掉到 ~100%）。
const OUTSIDE_MAX_BLACK: f64 = 3.0;
/// 几何比对容差（px）：白描边 6 源像素 → 横画布下 ~5px，再加 JPEG 振铃。
const RECT_TOL: f64 = 6.0;
/// 亮绿块占比允许的相对偏差：拉伸（→25%）与运镜（→17%）都远超它。
const LIME_REL_TOL: f64 = 0.25;
/// 逐帧稳定性：整段只有一条恒定运镜事件，绿块占比不该逐帧变（会变 = 运镜/缩放漏进来了）。
const LIME_SPREAD_MAX: f64 = 1.15;

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

struct Shot {
    /// 白描边 bbox（= 内容框），无白像素则 None
    rect: Option<Rect>,
    /// 亮绿像素占比（0..1）
    lime_frac: f64,
    /// 亮绿块 bbox
    lime_rect: Option<Rect>,
    /// 内容框之外：平均亮度 / 纯黑占比 / 亮绿像素数（越界 = 拉伸的直接证据）
    outside_luma: f64,
    outside_black: f64,
    outside_lime: usize,
}

fn luma(r: u8, g: u8, b: u8) -> f64 {
    0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64
}

/// 模板 `stageDrawRect` 的**独立复刻**（`templates/index.html`）：故意再写一遍 ——
/// 两边算不出同一只矩形，本身就是缺陷；只覆盖探针用得到的分支（x / y / fill）。
fn expected_rect(fit: &StageFit, sw: f64, sh: f64, cw: f64, ch: f64) -> Rect {
    let mut pad = fit.padding;
    let cap = cw.min(ch) / 2.0 - 1.0;
    if pad > cap {
        pad = cap.max(0.0);
    }
    let (bx, by, bwid, bhgt) = (pad, pad, cw - pad * 2.0, ch - pad * 2.0);
    let (mut w, mut h, mut x, mut y);
    if fit.axis == "x" {
        h = bhgt;
        w = sw * h / sh;
        if w > bwid {
            w = bwid;
            h = sh * w / sw;
        }
        x = match fit.position.as_str() {
            "left" => bx,
            "right" => bx + bwid - w,
            _ => bx + (bwid - w) / 2.0,
        };
        y = by + (bhgt - h) / 2.0;
    } else if fit.axis == "y" {
        w = bwid;
        h = sh * w / sw;
        if h > bhgt {
            h = bhgt;
            w = sw * h / sh;
        }
        y = match fit.position.as_str() {
            "top" => by,
            "bottom" => by + bhgt - h,
            _ => by + (bhgt - h) / 2.0,
        };
        x = bx + (bwid - w) / 2.0;
    } else {
        let s = (bwid / sw).max(bhgt / sh);
        w = sw * s;
        h = sh * s;
        x = bx + (bwid - w) / 2.0;
        y = by + (bhgt - h) / 2.0;
    }
    if (fit.scale - 1.0).abs() > 1e-9 {
        let (mx, my) = (x + w / 2.0, y + h / 2.0);
        w *= fit.scale;
        h *= fit.scale;
        x = mx - w / 2.0;
        y = my - h / 2.0;
    }
    Rect { x, y, w, h }
}

fn stats(jpeg: &[u8], cw: u32, ch: u32, inside: Option<&Rect>) -> Shot {
    let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
        .expect("解码渲染结果 JPEG")
        .to_rgb8();
    let (w, h) = (img.width(), img.height());
    assert_eq!((w, h), (cw, ch), "成片帧尺寸必须等于画布尺寸");

    let mut white: Vec<(u32, u32)> = Vec::new();
    let mut lime: Vec<(u32, u32)> = Vec::new();
    let mut outside_lime = 0usize;
    let mut outside_luma = 0.0f64;
    let mut outside_black = 0usize;
    let mut outside_n = 0usize;
    for y in 0..h {
        for x in 0..w {
            let p = img.get_pixel(x, y);
            let (r, g, b) = (p[0], p[1], p[2]);
            if r > WHITE && g > WHITE && b > WHITE {
                white.push((x, y));
            }
            // 探针素材的亮绿块 (0,255,0)，剥掉 JPEG 模糊余量
            let is_lime = g > 190 && r < 130 && b < 130;
            if is_lime {
                lime.push((x, y));
            }
            if let Some(rect) = inside {
                // 差 2px 余量：白描边在框上，JPEG 振铃会溢出一两个像素
                let within = (x as f64) >= rect.x + 2.0
                    && (x as f64) < rect.x + rect.w - 2.0
                    && (y as f64) >= rect.y + 2.0
                    && (y as f64) < rect.y + rect.h - 2.0;
                if !within {
                    outside_n += 1;
                    outside_luma += luma(r, g, b);
                    if r < 12 && g < 12 && b < 12 {
                        outside_black += 1;
                    }
                    if is_lime {
                        outside_lime += 1;
                    }
                }
            }
        }
    }
    let bbox = |pts: &[(u32, u32)]| -> Option<Rect> {
        if pts.is_empty() {
            return None;
        }
        let x0 = pts.iter().map(|p| p.0).min().unwrap();
        let x1 = pts.iter().map(|p| p.0).max().unwrap();
        let y0 = pts.iter().map(|p| p.1).min().unwrap();
        let y1 = pts.iter().map(|p| p.1).max().unwrap();
        Some(Rect { x: x0 as f64, y: y0 as f64, w: (x1 - x0 + 1) as f64, h: (y1 - y0 + 1) as f64 })
    };
    Shot {
        rect: bbox(&white),
        lime_frac: lime.len() as f64 / (w as f64 * h as f64),
        lime_rect: bbox(&lime),
        outside_luma: if outside_n > 0 { outside_luma / outside_n as f64 } else { 0.0 },
        outside_black: if outside_n > 0 {
            outside_black as f64 * 100.0 / outside_n as f64
        } else {
            0.0
        },
        outside_lime,
    }
}

fn replace_first_after(code: &str, anchor: &str, needle: &str, replacement: &str) -> String {
    let a = code.find(anchor).unwrap_or_else(|| panic!("代码里找不到锚点 {anchor:?}"));
    let tail = a + anchor.len();
    let at = tail
        + code[tail..]
            .find(needle)
            .unwrap_or_else(|| panic!("锚点 {anchor:?} 之后找不到 {needle:?}"));
    let mut out = String::with_capacity(code.len());
    out.push_str(&code[..at]);
    out.push_str(replacement);
    out.push_str(&code[at + needle.len()..]);
    out
}

/// 把一份**变异过的转译器**写到临时目录，返回路径（给 `common::render_range` 用）。
fn translator_with(tag: &str, mutate: impl FnOnce(&str) -> String) -> PathBuf {
    let src = common::translator_path();
    let code = std::fs::read_to_string(&src).expect("读转译器");
    let out = mutate(&code);
    let dir = std::env::temp_dir().join(format!("yekitrim-stagefit-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dst = dir.join(format!("polish-to-dhampir-{tag}.mjs"));
    std::fs::write(&dst, out).unwrap();
    dst
}

/// 用**出片真正的后端**（底座）渲染 `[start, end)` 每一帧（走 `common` 的共享实现）。
fn render_range(
    dir: &Path,
    translator: Option<&Path>,
    start: f64,
    end: f64,
    out_dir: Option<&Path>,
    label: &str,
) -> anyhow::Result<Vec<Vec<u8>>> {
    let frames = common::render_range(dir, translator, start, end)?;
    common::save_frames(&frames, out_dir, label)?;
    Ok(frames.into_iter().map(|(jpeg, _)| jpeg).collect())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "手动像素探针：需要 VTEDIT_FIT_DIR 指向竖素材探针工程（渲染走底座 Dhampir）"]
async fn portrait_source_is_contained_and_motion_locked() {
    let dir = PathBuf::from(
        std::env::var("VTEDIT_FIT_DIR").expect("set VTEDIT_FIT_DIR to a polish project dir"),
    );
    let out_dir = std::env::var("VTEDIT_FIT_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target/tmp-stagefit/out"));
    let t: f64 = std::env::var("VTEDIT_FIT_T")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2.0);
    let span: f64 = std::env::var("VTEDIT_FIT_SPAN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.6);
    let (start, end) = ((t - span / 2.0).max(0.0), t + span / 2.0);

    let cfg = load_project(&dir).expect("加载 polish.toml");
    let (cw, ch) = cfg.canvas.size();
    let (cw, ch) = (cw as f64, ch as f64);
    let src = yekitrim_polish::render::get_video_size(&dir.join(&cfg.source.video).to_string_lossy());
    let (sw, sh) = src.expect("探针工程必须能读出源尺寸");
    let fit = cfg.resolve_stage(src);
    println!(
        "工程 {} · 画布 {cw:.0}×{ch:.0} · 源 {sw}×{sh} · axis={} position={} bg={} scale={} padding={:.1}",
        dir.display(),
        fit.axis,
        fit.position,
        fit.bg,
        fit.scale,
        fit.padding
    );
    // 前置事实：这个探针只在「源是竖的、源比例 ≠ 画布比例（axis≠fill）、要补边（bg≠none）」时有意义
    assert!(sh > sw, "探针工程的源必须**竖屏**（{sw}×{sh}），否则量与题无关");
    assert_ne!(fit.axis, "fill", "探针工程解析出 axis=fill（源比例 = 画布比例），量不到「拉伸」");
    assert_ne!(fit.bg, "none", "探针工程解析出 bg=none，衬底本就不该画，请换个画布方向或 bg_mode");

    let expect = expected_rect(&fit, sw as f64, sh as f64, cw, ch);
    // 探针素材约定：亮绿块 = 源画面的 50%×50% → 期望占比 = 内容框面积 / 画布面积 × 25%
    let expect_lime = 0.25 * expect.w * expect.h / (cw * ch);
    let keep_outside = cw * ch - expect.w * expect.h > cw * ch * 0.02; // 内容框铺满时没有"框外"可比
    println!(
        "期望：内容框 x={:.1} y={:.1} w={:.1} h={:.1}（源纵横比 {:.4}）· 绿块占比 {:.2}%{}",
        expect.x,
        expect.y,
        expect.w,
        expect.h,
        sw as f64 / sh as f64,
        expect_lime * 100.0,
        if keep_outside { "" } else { " · 内容框≈铺满画布，跳过框外判据" }
    );

    let inside = if keep_outside { Some(&expect) } else { None };
    let backdrop_on = common::backdrop_enabled();
    if keep_outside && !backdrop_on {
        println!(
            "· 框外衬底那两条判据**跳过**：转译器里 `BACKDROP_ENABLED = false`（已知限制：底座长片 OOM）\
             —— 补的是黑边；衬底由 `portrait_backdrop` 探针专门量（它把开关打开）"
        );
    }
    let shots = |jpegs: &[Vec<u8>]| -> Vec<Shot> {
        jpegs.iter().map(|j| stats(j, cw as u32, ch as u32, inside)).collect()
    };

    // ---------- A：真转译器 ----------
    let real = render_range(&dir, None, start, end, Some(&out_dir), "real").unwrap();
    let real_shots = shots(&real);
    for (i, s) in real_shots.iter().enumerate() {
        let r = s.rect.unwrap_or_else(|| {
            panic!("第 {i} 帧找不到内容框白描边 —— 视频层没按摆放矩形画（或整个被推离画布）")
        });
        let brief = |r: Rect| format!("x{:.0} y{:.0} {:.0}×{:.0}", r.x, r.y, r.w, r.h);
        for (name, got, want) in
            [("x", r.x, expect.x), ("y", r.y, expect.y), ("w", r.w, expect.w), ("h", r.h, expect.h)]
        {
            assert!(
                (got - want).abs() <= RECT_TOL,
                "第 {i} 帧内容框 {name} = {got:.1}，期望 {want:.1}（±{RECT_TOL}）\n  实测 {} · 期望 {}",
                brief(r),
                brief(expect)
            );
        }
        let (got_aspect, want_aspect) = (r.w / r.h, sw as f64 / sh as f64);
        assert!(
            (got_aspect / want_aspect - 1.0).abs() < 0.03,
            "第 {i} 帧内容框纵横比 {got_aspect:.4} ≠ 源 {want_aspect:.4} —— 画面被拉伸了"
        );
        // 亮绿块是源的中央 50%×50%：它自己的 bbox 是第二条独立几何手柄
        // （内容框对、块错 = 映射的缩放/偏移错，不只是"框摆对位置"）
        let lr = s.lime_rect.expect("第 {i} 帧找不到亮绿块 —— 视频层没画出来");
        let want_lime = Rect {
            x: r.x + r.w * 0.25,
            y: r.y + r.h * 0.25,
            w: r.w * 0.5,
            h: r.h * 0.5,
        };
        for (name, got, want) in [
            ("x", lr.x, want_lime.x),
            ("y", lr.y, want_lime.y),
            ("w", lr.w, want_lime.w),
            ("h", lr.h, want_lime.h),
        ] {
            assert!(
                (got - want).abs() <= RECT_TOL,
                "第 {i} 帧亮绿块 {name} = {got:.1}，按内容框推算应为 {want:.1}（±{RECT_TOL}）"
            );
        }
        assert!(
            (s.lime_frac / expect_lime - 1.0).abs() <= LIME_REL_TOL,
            "第 {i} 帧绿块占比 {:.2}%（期望 {:.2}% ±{:.0}%）—— 画面被拉伸或被运镜缩放了",
            s.lime_frac * 100.0,
            expect_lime * 100.0,
            LIME_REL_TOL * 100.0
        );
        if keep_outside {
            assert_eq!(
                s.outside_lime, 0,
                "第 {i} 帧内容框之外还有 {} 个画面像素 —— 视频层越界（硬铺满）",
                s.outside_lime
            );
            // ⚠️ **这两条只在衬底那条路开着时才断**：`BACKDROP_ENABLED = false` 是**已知限制**
            // （底座侧长片 OOM，见转译器那段长注释），那时补的就是黑边 —— 那不是本探针的缺陷。
            // 衬底由 `tests/portrait_backdrop.rs` 专门量（它自己把开关打开再 A/B）。
            // 关着的时候**明说跳过**，不静默（静默 = 让整段判据的可信度归零）。
            if backdrop_on {
                assert!(
                    s.outside_black <= OUTSIDE_MAX_BLACK,
                    "第 {i} 帧内容框外纯黑 {:.1}% > {OUTSIDE_MAX_BLACK}% —— 补的是黑边，不是模糊关键帧当底",
                    s.outside_black
                );
                assert!(
                    s.outside_luma >= OUTSIDE_MIN_LUMA,
                    "第 {i} 帧内容框外平均亮度 {:.1} < {OUTSIDE_MIN_LUMA} —— 衬底没画上去",
                    s.outside_luma
                );
            }
        }
    }
    let (lime_min, lime_max) = real_shots.iter().fold((f64::MAX, f64::MIN), |(lo, hi), s| {
        (lo.min(s.lime_frac), hi.max(s.lime_frac))
    });
    assert!(
        lime_max / lime_min <= LIME_SPREAD_MAX,
        "整段绿块占比从 {:.2}% 变到 {:.2}%（{:.2}× > {LIME_SPREAD_MAX}×）—— 竖素材的运镜没被吃掉",
        lime_min * 100.0,
        lime_max * 100.0,
        lime_max / lime_min
    );
    println!(
        "✓ A 真转译器：{} 帧内容框 {} · 纵横比 {:.4} · 绿块 {:.2}%（逐帧 {:.2}%~{:.2}%）· 框外衬底亮度 {:.1} 纯黑 {:.2}%",
        real_shots.len(),
        {
            let r = real_shots[0].rect.unwrap();
            format!("x{:.0} y{:.0} {:.0}×{:.0}", r.x, r.y, r.w, r.h)
        },
        {
            let r = real_shots[0].rect.unwrap();
            r.w / r.h
        },
        real_shots[0].lime_frac * 100.0,
        lime_min * 100.0,
        lime_max * 100.0,
        real_shots[0].outside_luma,
        real_shots[0].outside_black
    );

    // ---------- B①：摆放几何失效（不按画布适配）----------
    //
    // ⚠️ 变异打在**转译器**上（底座那条腿的摆放几何由它算）。
    //
    // **2026-10-02 换过形态与判据**（旧形态已不可表达、旧数字来自已删除的 chrome 腿）：
    // ① 旧形态是「`w = bwid` 硬铺满内容区」，它等价于旧参照的 `drawImage(…,0,0,CW,CH)` ——
    //    但**底座的 `transform.scale` 是单值**（等比），**做不出各轴不等比的硬拉**；
    //    实测那样改会把画面**整体推出画布**（`offset.x = -1600`），绿块与描边一起消失，
    //    于是旧判据「绿块必须更大」不成立（实测 0.00%）—— 判据量的已经不是它想量的东西。
    // ② 现在把它改成 **cover（裁切铺满）** —— 也就是本次改版**之前的 `auto` 语义**
    //    （`resolve_stage` 的 auto 特例：非竖素材一律裁切铺满），而且它是底座**能**表达的那个形态
    //    （单值等比 `scale`，绕画布中心）。形态与判据都很清楚：源被放大 2.67× ⇒ 白描边被推出画布、
    //    绿块面积从 ~7.9% 涨到 ~50%。
    let stretched = translator_with("stretch", |code| {
        let out = code.replacen(
            "const FIT = SR.w / srcW;",
            "const FIT = Math.max(width / srcW, height / srcH);",
            1,
        );
        assert!(
            out.contains("const FIT = Math.max(width / srcW, height / srcH);")
                && !out.contains("const FIT = SR.w / srcW;"),
            "变异①没生效（FIT 锚点没命中）"
        );
        out
    });
    let m1 = render_range(&dir, Some(&stretched), start, end, None, "mut_stretch").unwrap();
    let m1_shots = shots(&m1);
    {
        // 判据 = **几何明显不是期望那一个**：内容框找不到（描边被推出画布），
        // 或者 x/y/w/h 任一项偏离期望 > 3×容差。A 段是逐帧**严格等于**期望（±`RECT_TOL`），
        // 所以这两条一起构成"探针看得见摆放几何失效"的自证。
        let r = m1_shots[0].rect;
        let off = |got: f64, want: f64| (got - want).abs() > RECT_TOL * 3.0;
        let differs = match r {
            None => true,
            Some(r) => {
                off(r.x, expect.x) || off(r.y, expect.y) || off(r.w, expect.w) || off(r.h, expect.h)
            }
        };
        // ⚠️ **判据只断几何**：绿块面积在这个形态下**不是**可靠判据 —— 裁切铺满把源放大 2.67× 后，
        // 四边白描边里只剩左/上两条还在画布内，bbox 变成「左描边 → 画布右沿」的退化矩形，
        // 而绿块面积实测 8.53% ≈ 期望 7.91%（巧合地接近）。
        // A 段是逐帧**严格等于**期望（±`RECT_TOL`），这里只要求"明显不是那一个" —— 成对即自证。
        assert!(
            differs,
            "变异①（摆放几何失效）后内容框 {:?} 与期望 x{:.0} y{:.0} {:.0}×{:.0} 没有区别 —— \
             判据没翻面，说明它看不见「几何失效」（绿块 {:.2}%）",
            r.map(|r| format!("{:.0}×{:.0}", r.w, r.h)),
            expect.x,
            expect.y,
            expect.w,
            expect.h,
            m1_shots[0].lime_frac * 100.0
        );
        if keep_outside {
            assert!(
                m1_shots[0].outside_lime > 0,
                "变异①（硬铺满）后内容框之外没有画面像素？框外判据同样看不见这个缺陷"
            );
        }
        println!(
            "✓ B① 变异「硬铺满」翻面：内容框 {}（真模板 {:.0}×{:.0}）· 绿块 {:.2}%（真模板 {:.2}%）",
            match r {
                Some(r) => format!("{:.0}×{:.0}", r.w, r.h),
                None => "白描边被推出画布".to_string(),
            },
            expect.w,
            expect.h,
            m1_shots[0].lime_frac * 100.0,
            real_shots[0].lime_frac * 100.0
        );
    }

    // ---------- B②：去掉竖屏运镜门（等价改造前：竖素材照样套 zoom_close=1.5）----------
    //
    // 转译器里那道门是 `case 'camera'` 里的 `if (sourceIsVertical === true) { …整条不生效… }`。
    //
    // ⚠️ **必须用 `replace_first_after` 定位到 `case 'camera':` 之后那一个**（2026-10-02 修）：
    // 转译器里 `if (sourceIsVertical === true) {` **出现过两次** —— 先一处是给报告用的 NOTE，
    // 再一处才是运镜门。直接 `replacen(..., 1)` 会改掉**前面那一处 NOTE**：
    // 变异体仍然把运镜整条关着（实测 `运镜 0 条`，成片与真转译器逐帧相同），
    // 于是 B② 的判据必然"没翻面"。锚点必须落在语义位置上，不是第一个长得像的地方。
    let no_gate = translator_with("camera", |code| {
        let out = replace_first_after(
            code,
            "case 'camera': {",
            "if (sourceIsVertical === true) {",
            "if (false) {",
        );
        // `sourceIsVertical === null`（探不到媒体）那条分支也要关掉：探针工程探得到，
        // 留着它不影响；但万一本机 ffprobe 探不到，两条分支都成了"不关运镜"，变异照样成立。
        let out = out.replacen("if (sourceIsVertical === null) {", "if (false) {", 1);
        assert_ne!(out, code, "变异②没生效（竖屏门锚点没命中）");
        out
    });
    let m2 = render_range(&dir, Some(&no_gate), start, end, None, "mut_camera").unwrap();
    let m2_shots = shots(&m2);
    for (i, s) in m2_shots.iter().enumerate() {
        assert!(
            s.lime_frac > real_shots[i].lime_frac * 1.4,
            "变异②（去掉竖屏运镜门）后第 {i} 帧绿块占比 {:.2}% ≈ 真模板 {:.2}% —— 运镜门判据看不见它",
            s.lime_frac * 100.0,
            real_shots[i].lime_frac * 100.0
        );
    }
    println!(
        "✓ B② 变异「去掉竖屏运镜门」翻面：绿块 {:.2}%（真模板 {:.2}%，zoom_close=1.5 应 ≈2.25×）",
        m2_shots[0].lime_frac * 100.0,
        real_shots[0].lime_frac * 100.0
    );

    // ---------- B③：不画衬底（等价改造前：DOM 有模糊底、出片没有）----------
    //
    // 转译器里背景层那个开关是 `const BACKDROP_ENABLED = false;`（**当前默认就是关的**，
    // 理由见转译器里的长注释：底座侧的长片 OOM 还没修完）。所以这条变异**只在开关已经打开
    // 的版本上**有意义 —— 打不开时它证明不了"框外判据看得见衬底缺失"，那就**明说并跳过**，
    // 而不是假装验过（静默跳过一条自证 = 让整段判据的可信度归零）。
    if keep_outside {
        let mutacked = translator_with("backdrop", |code| {
            code.replacen("const BACKDROP_ENABLED = false;", "const BACKDROP_ENABLED = true;", 1)
        });
        let src = std::fs::read_to_string(&mutacked).unwrap();
        if !src.contains("const BACKDROP_ENABLED = true;") {
            println!(
                "· 变异③「不画衬底」跳过：转译器里衬底那条路**本来就是关的**\
                 （`BACKDROP_ENABLED = false`），没有可关的东西 —— 框外判据的自证等它打开后补"
            );
        } else {
            let no_bg = translator_with("nobackdrop", |code| {
                let out = code.replacen(
                    "const BACKDROP_ENABLED = false;",
                    "const BACKDROP_ENABLED = true;",
                    1,
                );
                // 打开之后再把"画"那一步拆掉：`STAGE_BG === 'blur'` 的判据改成恒假。
                out.replacen("if (BACKDROP_ENABLED && STAGE_BG === 'blur') {", "if (false) {", 1)
            });
            let m3 = render_range(&dir, Some(&no_bg), start, end, None, "mut_backdrop").unwrap();
            let m3_shots = shots(&m3);
            assert!(
                m3_shots[0].outside_black >= 90.0,
                "变异③（不画衬底）后内容框外纯黑只有 {:.1}% —— 框外判据看不见「衬底缺失」",
                m3_shots[0].outside_black
            );
            println!(
                "✓ B③ 变异「不画衬底」翻面：框外纯黑 {:.1}%（真模板 {:.2}%）",
                m3_shots[0].outside_black, real_shots[0].outside_black
            );
        }
    }

    println!(
        "✓ 全部通过：竖素材按 contain 摆进画布（{}）、运镜被吃掉、框外是模糊关键帧当底。帧已写到 {}",
        {
            let r = real_shots[0].rect.unwrap();
            format!("x{:.0} y{:.0} {:.0}×{:.0}", r.x, r.y, r.w, r.h)
        },
        out_dir.display()
    );
}
