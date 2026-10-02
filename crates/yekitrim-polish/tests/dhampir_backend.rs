//! **Dhampir 后端**的集成测试（`#[ignore]`，要真二进制 + 真片段才能跑）。
//!
//! 跑：
//! ```text
//! VTEDIT_TEST_CLIP="<片段目录>" \
//! VTEDIT_DHAMPIR="<Dhampir>/target/release/dhampir.exe" \
//! VTEDIT_DHAMPIR_FONT="C:/Windows/Fonts/msyh.ttc" \
//! cargo test -p yekitrim-polish --test dhampir_backend -- --ignored --nocapture
//! ```
//!
//! # 这个测试在守什么
//!
//! 守**接口**，不守**像素**：像素那条由 `F:\para\Code\Dhampir\scripts\yekitrim-compare.mjs`
//! 负责（它比的就是"Dhampir 这条路 vs 参照 chrome"，基线 7.01）。这里守的是
//! **"V-Trim 调二进制"这条缝**：
//!
//! 1. 三个东西（二进制 / 转译器 / 字体）**都能定位**，且 `describe()` 把路径说清楚
//! 2. **出一条只含画面的片子**：有视频、**没有音轨**（`--no-audio` 是这条缝的核心约定 ——
//!    音轨归本仓，两边都做就会有两份混音实现）
//! 3. 帧数与尺寸**对得上**：帧数**从转译产物里读**（工程的时基是素材自己的，不是上层那个
//!    写死 30fps 的估算 —— 拿错那一份会只出一半；2026-09-30 实测踩过），
//!    而 Dhampir 的 `--to` 是闭区间（差一帧这种事只有真跑一次才看得出来）
//!
//! # 为什么是 `#[ignore]`
//!
//! 它需要**外部二进制**与**一个真片段** —— 默认测试里没有那两样，硬跑必然红。
//! 与 `render_bench.rs` 同一套约定。
//!
//! # 找不到东西时**必须报错**（这条也在守）
//!
//! `describe()` 在定位失败时会把原因写进字符串（`<找不到：…>`），而不是返回空。
//! 下面有一条断言专门盯它 —— 因为"静默回退到 chrome"正是这个后端最该避免的失败形状：
//! 那会让人以为自己在用 Dhampir 出片。

use std::path::PathBuf;

use yekitrim_polish::render::dhampir;

/// 片段目录（必填）。没给就**明确跳过**并说清要设什么。
fn clip_dir() -> Option<PathBuf> {
    match std::env::var("VTEDIT_TEST_CLIP") {
        Ok(v) if !v.trim().is_empty() => {
            let p = PathBuf::from(&v);
            if p.is_dir() {
                Some(p)
            } else {
                panic!("VTEDIT_TEST_CLIP={v} 不是一个目录");
            }
        }
        _ => None,
    }
}

/// 三个依赖**都能定位**，而且 `describe()` 把路径说出来。
///
/// 这条不需要真片段，所以它能在只设了 `VTEDIT_DHAMPIR` / `VTEDIT_DHAMPIR_FONT`
/// 的机器上跑 —— 用来确认"这条缝有没有接通"。
#[test]
#[ignore = "手动：要真的 Dhampir 二进制与字体"]
fn 三个依赖都能定位并说清路径() {
    // 空串 = "没在库里选字体"（走系统字体栈那条路），与工程缺省一致。
    let d = dhampir::describe("");
    println!("{d}");
    for needle in ["dhampir=", "translator=", "font="] {
        assert!(d.contains(needle), "describe() 少了 {needle}：\n{d}");
    }
    // **不许静默失败**：定位不到时必须写 `<找不到：…>`，而不是留空。
    assert!(
        !d.contains("dhampir=<\n") && !d.contains("dhampir=\n"),
        "二进制那一行是空的 —— 定位失败要写清原因：\n{d}"
    );
}

/// **端到端**：转译 -> Dhampir 出画面 -> 产物是一段**没有音轨**的视频，帧数对得上。
#[test]
#[ignore = "手动：要真二进制 + 真片段（VTEDIT_TEST_CLIP）"]
fn 出片是只含画面的视频且帧数对得上() {
    let Some(clip) = clip_dir() else {
        eprintln!("跳过：没有设 VTEDIT_TEST_CLIP（片段目录）");
        return;
    };

    // 用一个自己的临时目录当"工程目录"（中间文件落在那里），**不碰真片段**。
    let work = std::env::temp_dir().join(format!("vtrim-dhampir-it-{}", std::process::id()));
    std::fs::create_dir_all(&work).expect("建临时工作目录");

    // **帧数的权威是转译产物**（工程的时基 = 素材自己的，60；上层那个 30 是写死的估算）。
    // `true` = 出片那条腿：时间线保持整条（屏蔽区间交给尾部那一剪），与 `render_video_only` 一致。
    let translated = match dhampir::prepare_project(&clip, &work, true) {
        Ok(p) => p,
        Err(e) => panic!("转译失败：{}", dhampir::explain(&e)),
    };
    let want = match dhampir::sequence_frames(&translated) {
        Ok(n) => n,
        Err(e) => panic!("读不出序列长度：{}", dhampir::explain(&e)),
    };

    println!("clip   = {}", clip.display());
    println!("work   = {}", work.display());
    println!("frames = {want}（来自转译产物的层 end）");
    println!("{}", dhampir::describe(""));

    let out = match dhampir::render_video_only(&work, &clip, "", false) {
        Ok(p) => p,
        Err(e) => panic!("出片失败：{}", dhampir::explain(&e)),
    };
    assert!(out.is_file(), "产物不存在：{}", out.display());
    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    assert!(size > 0, "产物是 0 字节");

    // 用 ffprobe 核对：**有视频、没有音轨**，帧数对得上。
    // 断言 ffprobe 真的在（它出片链路的依赖，不是本测试新引入的）。
    let probe = |args: &[&str]| -> String {
        let o = std::process::Command::new("ffprobe")
            .args(args)
            .output()
            .expect("要 ffprobe（出片链路本来就依赖它）");
        assert!(o.status.success(), "ffprobe 失败：{:?}", o.status);
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let streams = probe(&[
        "-v",
        "error",
        "-show_entries",
        "stream=codec_type",
        "-of",
        "default=nw=1",
        out.to_str().unwrap(),
    ]);
    assert!(
        streams.contains("codec_type=video"),
        "产物里没有视频流：\n{streams}"
    );
    assert!(
        !streams.contains("codec_type=audio"),
        "产物里**不该有音轨** —— `--no-audio` 是这条缝的核心约定（音轨归本仓）：\n{streams}"
    );

    let frames = probe(&[
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-count_frames",
        "-show_entries",
        "stream=nb_read_frames",
        "-of",
        "default=nw=1",
        out.to_str().unwrap(),
    ]);
    let got: usize = frames
        .lines()
        .find_map(|l| l.strip_prefix("nb_read_frames="))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| panic!("读不出帧数：\n{frames}"));
    // **差一帧是这类缝最容易出的错**：Dhampir 的 `--to` 是闭区间。
    assert_eq!(
        got, want,
        "帧数不对：转译产物说 {want}，Dhampir 出了 {got}（`--to` 是闭区间，传的是 sequence-1）"
    );

    std::fs::remove_dir_all(&work).ok();
}

/// 空工程 / 算不出帧数时**报错**，不产一个空文件出来。
///
/// ⚠️ "帧数是 0" 那道闸现在住在 `sequence_frames` 里（帧数从转译产物读），所以这条改成：
/// **对着一个空目录**跑，必须报错、且**不许留下产物**。`VTEDIT_TEST_CLIP` 是空目录时
/// 转译这一步就会失败 —— 报错的是哪一步都可以，但**绝不能悄悄产出一个 0 字节的文件**。
#[test]
#[ignore = "手动：要真二进制"]
fn 空工程要报错而不是产空文件() {
    let work = std::env::temp_dir().join(format!("vtrim-dhampir-zero-{}", std::process::id()));
    std::fs::create_dir_all(&work).ok();
    let err = dhampir::render_video_only(&work, &work, "", false)
        .expect_err("空工程必须报错（宁可报错，也不产一个空文件）");
    let text = dhampir::explain(&err);
    println!("{text}");
    assert!(!text.trim().is_empty(), "错误信息不许是空的");
    assert!(
        !dhampir::video_intermediate(&work).exists(),
        "报错了却留下了产物：{}",
        dhampir::video_intermediate(&work).display()
    );
    std::fs::remove_dir_all(&work).ok();
}
