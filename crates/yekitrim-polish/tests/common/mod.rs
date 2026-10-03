//! 手动像素探针的共享工具。
//!
//! ⚠️ **2026-10-01 起探针不再起浏览器**：它们当年驱动的是本仓的 headless Chrome 逐帧腿
//! （`render/chrome.rs::ChromeBackend`），那条腿已整条删除。
//!
//! ## 探针现在验的是谁
//!
//! 出片只有**底座一条腿**（`dhampir::render_video_only` → `tools/polish-to-dhampir.mjs` →
//! `dhampir.exe`），所以只有它算"成片到底长什么样"的权威。探针因此：
//!
//! 1. **把转译器复制到临时目录并改一行**（A/B 里的 B = 变异体）；
//! 2. 用 `VTEDIT_TRANSLATOR=<那份副本>` 让底座那一趟走变异体；
//! 3. 渲染整段，再从**源素材**按同一时间基抽帧比像素。
//!
//! ## 为什么不能改 `templates/index.html`
//!
//! `templates/index.html` 是**参照实现**（已作废的 canvas 那条腿的模板），底座那条腿
//! **根本不读它** —— 转译器是它的平行实现（`tools/polish-to-dhampir.mjs` 里逐条对着
//! `index.html:NNN` 重写）。改 HTML 对底座出片**一个像素都不影响**，A/B 会永远"看不出差别"
//! 而判据照旧全绿 —— 那正是"探针在空转"最坏的一种：不报错、还给假信心。
//!
//! ## 为什么必须 A **和** B 都断言
//!
//! 只断言 A（真模板）等于不知道探针能不能看见缺陷。所以每条判据都要配一个变异体，
//! 并要求变异体**按预期翻面**（例：把衬底关掉 → 那几帧的四角必须掉到接近纯黑）。
//!
//! ⚠️ 探针会在**临时目录**里跑转译器副本，但渲染产物（`_temp_video.mp4` /
//! `_dhampir_project.json`）仍落在**探针工程**里。请对着工程副本跑。

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use yekitrim_polish::project::load_project;
use yekitrim_polish::render::frame::extract_chunk_frames;

/// 转译器在仓库里的位置（与 `dhampir::find_translator` 同一套候选）。
pub fn translator_path() -> PathBuf {
    if let Ok(explicit) = std::env::var("VTEDIT_TRANSLATOR") {
        let p = PathBuf::from(explicit);
        if p.is_file() {
            return p;
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo = manifest.parent().and_then(Path::parent).expect("仓库根");
    repo.join("tools").join("polish-to-dhampir.mjs")
}

/// 转译器里衬底那条路**当前**是不是开的（`BACKDROP_ENABLED`）。
///
/// 它默认 **false**：底座侧模糊中间纹理逐帧累积，**长片必 OOM**（实测阈值 1100~1500 帧，
/// 转译器里那段长注释有原话），所以「留边处补的是黑边」是**已知限制**、不是探针的缺陷。
/// 探针照它决定要不要断衬底那两条判据，并**明说跳过** —— 静默跳过一条判据等于让整段可信度归零。
/// （衬底本身由 `tests/portrait_backdrop.rs` 专门量：它自己把开关打开再 A/B。）
pub fn backdrop_enabled() -> bool {
    std::fs::read_to_string(translator_path())
        .map(|s| s.contains("const BACKDROP_ENABLED = true;"))
        .unwrap_or(false)
}

/// 把转译器复制一份、应用 `replacements`（`(原文, 替换文)`，每处必须**恰好命中一次**）。
///
/// 返回副本路径。每处替换都做"命中次数必须恰好 1"的断言 —— 转译器改了措辞时，
/// **在这里当场失败**，而不是悄悄变异了一个不相关的字符串、让 A/B 比同一份产物。
pub fn mutated_translator(
    tag: &str,
    replacements: &[(&str, &str)],
) -> anyhow::Result<PathBuf> {
    let src = translator_path();
    let code = std::fs::read_to_string(&src)?;
    let mut out = code.clone();
    for (from, to) in replacements {
        let n = out.matches(from).count();
        anyhow::ensure!(
            n == 1,
            "变异锚点在转译器里出现了 {n} 次（要求恰好 1 次）：{from:?}\n  \
             转译器改了措辞？改这里的同时要改探针的锚点 —— 锚点失配会让 A/B 比同一份产物、判据静默失效。"
        );
        out = out.replace(from, to);
    }
    let dir = std::env::temp_dir().join(format!("yekitrim-probe-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let dst = dir.join(format!("polish-to-dhampir-{tag}.mjs"));
    std::fs::write(&dst, out)?;
    Ok(dst)
}

/// 用**出片真正的后端**（底座）渲染整个工程，返回中间成片 `<dir>/_temp_video.mp4`。
///
/// `translator` = 这一趟要用的转译器（`None` = 仓库里那份真的）。
/// ⚠️ **必须用同一个进程跑**：`VTEDIT_TRANSLATOR` 是进程级环境变量，而 `render_video_only`
/// 会自己读它。所以这里的做法是"设变量 → 渲染 → 还原"，调用方**不许并发**跑多条探针
/// （这些测试本来就串行、且 `#[ignore]` 手动跑）。
pub fn render_project(dir: &Path, translator: Option<&Path>) -> anyhow::Result<PathBuf> {
    let prev = std::env::var("VTEDIT_TRANSLATOR").ok();
    match translator {
        Some(t) => unsafe { std::env::set_var("VTEDIT_TRANSLATOR", t) },
        None => unsafe { std::env::remove_var("VTEDIT_TRANSLATOR") },
    }
    let font = load_project(dir).map(|c| c.style.subtitle.font).unwrap_or_default();
    let result = yekitrim_polish::render::dhampir::render_video_only(dir, dir, &font, false);
    // 还原环境变量**先于**错误返回：否则一次失败会把变异体泄漏给后面的探针。
    match prev {
        Some(v) => unsafe { std::env::set_var("VTEDIT_TRANSLATOR", v) },
        None => unsafe { std::env::remove_var("VTEDIT_TRANSLATOR") },
    }
    result?;
    let out = dir.join("_temp_video.mp4");
    anyhow::ensure!(
        out.is_file(),
        "底座跑完却没有 {}（渲染失败但没报错？）",
        out.display()
    );
    Ok(out)
}

/// 渲染整个工程，再从**成片**按 30fps 抽出 `[start, end)` 的每一帧（JPEG + 时刻）。
///
/// ⚠️ **必须抽成片**（`_temp_video.mp4`，画布尺寸），不是源素材：探针验的是「画面摆放 / 衬底
/// 在出片这条路上对不对」，而**源素材上根本没有 stage 几何**（摆位是出片时才画的）。
/// 2026-10-02 修：这里以前抽源素材，于是两条探针里那句「成片帧尺寸必须等于画布尺寸」
/// **必然失败**（探针源 720×1280 vs 画布 1920×1080）—— chrome 那条腿删掉之后没人再跑过它们。
pub fn render_range(
    dir: &Path,
    translator: Option<&Path>,
    start: f64,
    end: f64,
) -> anyhow::Result<Vec<(Vec<u8>, f64)>> {
    let movie = render_project(dir, translator)?;
    let frames = extract_chunk_frames(&movie.to_string_lossy(), start, end, 30, false)?;
    anyhow::ensure!(
        !frames.is_empty(),
        "[{start},{end}) 抽帧为空 —— {} 里那一段没有帧？",
        movie.display()
    );
    Ok(frames
        .into_iter()
        .filter_map(|f| f.frame.jpeg.map(|jpeg| (jpeg, f.t)))
        .collect())
}

/// 逐帧落盘（`out_dir` 为 `None` 时只返回）。
pub fn save_frames(
    frames: &[(Vec<u8>, f64)],
    out_dir: Option<&Path>,
    label: &str,
) -> anyhow::Result<()> {
    if let Some(out_dir) = out_dir {
        std::fs::create_dir_all(out_dir)?;
        for (i, (jpeg, t)) in frames.iter().enumerate() {
            std::fs::write(out_dir.join(format!("{label}_{i:04}_t{t:.2}.jpg")), jpeg)?;
        }
    }
    println!("  [{label}] {} 帧", frames.len());
    Ok(())
}
