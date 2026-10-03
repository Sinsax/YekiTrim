//! polish::render::pipeline：出片主链路（转译成底座工程 → `dhampir.exe` 整段出片 → ffmpeg 编码混流 → 统计）。
//!
//! 入口：`cmd_render`、`resolve_paths` / `ResolvedPaths`、`inject_assets`、`prepare_sticker_data`、`unique_output_path`、`measure_loudness_curve_cached`、`cleanup`。
//! 边界：子进程链路 —— 输出必须落文件、绝不能用管道（ffmpeg 是孙进程，管道永不 EOF）；`<工程>/data/` 的磁盘缓存别去掉；同名产物加 `_1` / `_2` —— 见 `crates/yekitrim-polish/src/render/AGENTS.md`

use base64::Engine;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::project::ProjectFile;
use crate::render::backend::RenderBackendKind;
use crate::render::ffmpeg::Ffmpeg;
use crate::render::sound::collect_sound_events;
use crate::{assets, process, project, render};

/// 跨磁盘移动文件：rename 失败且为跨盘错误时回退为 copy + remove。
fn move_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(17) => {
            std::fs::copy(src, dst)?;
            std::fs::remove_file(src)?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// 效果词汇表（`spec/effects.json`）的定位
// ---------------------------------------------------------------------------

/// 效果词汇表原文：**运行时**先找包内（`<程序目录>/spec/effects.json`）或开发树
/// （`<程序目录>/../../spec/…`）那一份，找不到就用**编译期内嵌**的那份。
///
/// 为什么两边都要：发布包随包带上它（`xtask package`），而开发树里它就在仓库根 ——
/// 内嵌那份保证"文件被删了/坏了"时仍有权威可用（与转译器读不到就退化是同一套裁决）。
fn effect_spec_json() -> String {
    spec_effects_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| include_str!("../../../../spec/effects.json").to_string())
}

/// 词汇表（`spec/effects.json`）在**包内 / 开发树**里的路径；找不到就 `None`（改用编译期内嵌那份）。
///
/// 抽成函数是为了让 `polish doctor` 也能**把它印出来**（2026-10-01）：它是包内的第 4 件交付物，
/// 也是转译器与出片两侧共同的输入 —— doctor 只印二进制/转译器/预览产物的话，
/// 会让人以为词汇表不在包里（包内路径与内嵌那份**行为上等价**，但"在哪"这件事必须可见）。
pub(crate) fn spec_effects_path() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("spec").join("effects.json"));
            if let Some(up2) = dir.parent().and_then(Path::parent) {
                candidates.push(up2.join("spec").join("effects.json"));
            }
        }
    }
    candidates.into_iter().find(|p| {
        std::fs::read_to_string(p)
            .map(|t| serde_json::from_str::<serde_json::Value>(&t).is_ok())
            .unwrap_or(false)
    })
}

/// canvas 通道**支持**的效果集合（词汇表 `anchors.canvas` 里非 null 的那些）。
///
/// `None` = 词汇表读不出来（理论上只有内嵌那份坏了才会）⇒ 调用方**只警告不拦**
/// —— 因为一个坏文件不该让整条出片链路停摆。
///
/// ⚠️ **现在没有生产调用者**：它当年是"canvas 兜底通道冻结闸门"（S4）的输入，而 canvas 那条腿
/// （`render/chrome.rs`）已于 2026-10-01 删除。留着是因为两件事都要它：① `spec_effects_path()`
/// 的词汇表定位仍被 `polish doctor` 与转译链使用；② 下面的单测钉住"`anchors.canvas` 与
/// `effects[].id` 必须对得上"（词汇表与模板 `index.html` 的漂移防线）。**别再把它接回一条
/// 出片分支** —— 出片只有底座那条腿。
#[cfg_attr(not(test), allow(dead_code))]
fn canvas_supported_effects() -> Option<std::collections::HashSet<String>> {
    let spec: serde_json::Value = serde_json::from_str(&effect_spec_json()).ok()?;
    let canvas = spec.get("anchors")?.get("canvas")?.as_object()?;
    Some(
        canvas
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, _)| k.clone())
            .collect(),
    )
}

/// 工程里**实际用到**的事件类型（去重、保持出现顺序）。
///
/// 走序列化而不是 match 枚举：`serde_json::to_value(config)` 得到的就是写进 `polish.toml` 的那套
/// 类型名，**加新类型时这里不用改**（而 match 枚举会漏掉新变体）。
///
/// ⚠️ 同 `canvas_supported_effects()`：生产路径上暂时没有调用者（它当年喂给 canvas 冻结闸门），
/// 留着是因为它自带单测、且任何"这个工程用了哪些效果"的判断都得走它。
#[cfg_attr(not(test), allow(dead_code))]
fn used_effect_types(config: &ProjectFile) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Ok(value) = serde_json::to_value(config) else {
        return out;
    };
    let Some(segments) = value.get("segments").and_then(|v| v.as_array()) else {
        return out;
    };
    for seg in segments {
        let Some(events) = seg.get("events").and_then(|v| v.as_array()) else {
            continue;
        };
        for ev in events {
            if let Some(t) = ev.get("type").and_then(|v| v.as_str()) {
                if !out.iter().any(|x| x == t) {
                    out.push(t.to_string());
                }
            }
        }
    }
    out
}

/// 纯函数：`used` 里哪些不在 `supported` 里（去重、保持顺序）—— 抽出来是为了可测。
#[cfg_attr(not(test), allow(dead_code))]
fn missing_support<'a>(
    used: impl Iterator<Item = &'a str>,
    supported: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in used {
        if !supported.contains(t) && !out.iter().any(|x| x == t) {
            out.push(t.to_string());
        }
    }
    out
}


/// 若目标文件已存在，返回 `stem_1.ext` / `stem_2.ext` 等不冲突路径。
pub fn unique_output_path(path: &Path) -> std::path::PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("output");
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((s, e)) => (s, e),
        None => (file_name, ""),
    };
    let mut n = 1u32;
    loop {
        let name = if ext.is_empty() {
            format!("{stem}_{n}")
        } else {
            format!("{stem}_{n}.{ext}")
        };
        let candidate = parent.join(&name);
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

pub async fn cmd_render(project_path: &Path) -> anyhow::Result<()> {
    // ⚠️ **先把工程目录变成绝对路径**（2026-09-30 实测踩到）：`resolve_paths` 把
    // `project.output` 归一成 `<工程目录>/export.mp4` —— 工程目录是**相对**的时候，
    // 后面"不是绝对路径就再 join 一次工程目录"那一步会把目录名**拼两遍**
    // （`<dir>\<dir>\export.mp4`，合流那一步报 No such file or directory）。
    // CLI 的默认输入就是 `.`（`yekitrim polish render` 在片段目录里直接跑），所以这条一定会被踩到。
    let project_path = &std::path::absolute(project_path).unwrap_or_else(|_| project_path.to_path_buf());
    tracing::info!(
        "  Rendering pipeline...（{} 构建{})",
        if cfg!(debug_assertions) { "debug" } else { "release" },
        if cfg!(debug_assertions) { "，逐帧的字节处理比 release 慢得多，出片请用打包版" } else { "" }
    );
    // 自动先跑 process（幂等）
    let t_process = std::time::Instant::now();
    if let Err(e) = process::cmd_process(project_path).await {
        tracing::info!("  ⚠ Process before render: {}", e);
    }
    tracing::info!("  ⏱ process {:.1}s", t_process.elapsed().as_secs_f64());
    let start_time = std::time::Instant::now();

    let config = project::load_project(project_path)?;
    let resolved = resolve_paths(project_path, &config);
    for (label, p) in [("Video", &resolved.video), ("SRT", &resolved.subtitles)] {
        if !Path::new(p).exists() {
            anyhow::bail!("{} not found: {}", label, p);
        }
    }
    tracing::info!("  Output: {}", resolved.output);

    let duration = std::fs::read_to_string(project_path.join("data").join("clip-meta.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("duration").and_then(|d| d.as_f64()))
        .or(config.duration)
        .unwrap_or(0.0);
    if duration <= 0.0 {
        anyhow::bail!("Video duration unknown");
    }
    tracing::info!("  Duration: {:.1}s", duration);

    let fps = 30;
    // ⚠️ `VTEDIT_PARALLELISM` 与这里那个 30fps 的 `frame_count` 都是**已删除的 Chrome 分块腿**
    // 留下的（它按 fps × 时长切块、4 路并行）。底座那条腿**不用它们**：帧数由
    // `dhampir::render_video_only` 从转译产物里读（唯一权威，见 `render/AGENTS.md`）。
    // 留着只为了下面那条日志（"这个工程多少秒 / 多少帧"是排查时第一眼要看的数字）。
    let parallelism = std::env::var("VTEDIT_PARALLELISM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4)
        .max(1);
    let frame_count = (duration * fps as f64).ceil() as usize;
    tracing::info!("    {} frames at {} fps", frame_count, fps);
    tracing::debug!("    (VTEDIT_PARALLELISM={parallelism} —— 仅历史遗留，底座那条腿不分块)");

    // 选择渲染后端。
    //
    // ⚠️ **现在只有 Dhampir（底座）一条腿**（2026-09-30 用户拍板切默认；2026-10-01 把本仓那条
    // headless Chrome 逐帧腿整条删掉）。取不到环境变量时的兜底值也必须与 CLI 的
    // `--renderer` 默认值一致 —— 两处不一致会让"库里调用"与"命令行调用"走出两条不同的路。
    // 老命令行里的 `wgpu` / `native` 由 `RenderBackendKind::parse` 认成 Dhampir（它们本来就是
    // "用 GPU 出片"的意思，而 GPU 出片现在只有底座这一条）。
    let renderer = match std::env::var("VTEDIT_RENDERER") {
        Ok(v) => RenderBackendKind::parse(&v).ok_or_else(|| {
            // **打错字不许被静静当成别的通道**：静默误路由表现为"选错了渲染器却没报错"，
            // 而两版成片的差别只有对着看才发现。
            anyhow::anyhow!(
                "认不出的渲染后端 `{v}`（VTEDIT_RENDERER / --renderer）。现在只有 dhampir 一个值\n  \
                 （老写法 wgpu / native 等同 dhampir —— GPU 出片现在只有底座那一条路；\n  \
                 `chrome` 那条本仓 headless Chrome 的兜底腿已于 2026-10-01 **整条删除**，\n  \
                 装外部浏览器的出片路径不再存在）"
            )
        })?,
        Err(_) => RenderBackendKind::Dhampir,
    };
    tracing::info!("  Renderer: {}", renderer.as_str());

    // 网页服务器与页面 `window.__SOUND_DATA` 都是**已删除的 Chrome 那条腿**的依赖：
    // 底座那条腿不需要页面，声音事件由 `collect_sound_events` 在 Rust 里直接收。
    tracing::info!("\n  -- Sound data --");
    let t_sound = std::time::Instant::now();
    let sfx_data = collect_sound_events(&config);
    tracing::info!(
        "  v {} sound events  ⏱ {:.1}s",
        sfx_data.len(),
        t_sound.elapsed().as_secs_f64()
    );

    // 贴纸帧**不再在这里预解码**。
    //
    // 这一步曾经是给已删除的 Chrome 那条腿用的（页面靠 `sticker_frames.json` 里的 PNG base64
    // 逐帧画贴纸），它唯一的消费者 `render_chunk_sync`（逐帧接缝）已随 2026-10-02 的 B2 收口
    // **整段删除** —— 出片只剩底座一条腿，贴纸由转译器自己带图。
    //
    // ⚠️ **别把它加回来**：实测 `clip-04`（4 张贴纸）冷解码 **20.2s**、产物 **67.7MB**
    // （源 GIF 合计才 38MB），而这份 JSON 写完盘**没有任何人读**。它以前是"顺带预热磁盘缓存"，
    // 但只有同一工程反复出片才省；第一次出片与每次改贴纸之后都要白付这 20 秒。
    //
    // 需要这份数据的地方**自己调** `prepare_sticker_data`（缓存照旧落 `data/sticker_frames.json`，
    // 按「文件名 + mtime + 大小」失效）：诊断用例 `diagnose_sticker_cache` 就是这么用的，
    // 将来真接第二个逐帧后端时也由那个后端自己的入口决定要不要付这笔钱。

    // ---------------------------------------------------------------------
    // **出片**：整段交给独立仓的 Dhampir 二进制（唯一一条腿）
    // ---------------------------------------------------------------------
    //
    // 产物落在 `_temp_video.mp4` 上，后面的音轨链路（抽音轨 / 音效混音 / 响度 / 合流）一行不改。
    //
    // **画面归 Dhampir、声音归本仓**：二进制那一趟带 `--no-audio`。
    let tv = project_path.join("_temp_video.mp4");
    {
        tracing::info!("\n  -- Dhampir（二进制后端，只出画面）--");
        for line in render::dhampir::describe(&config.style.subtitle.font).lines() {
            tracing::info!("  {line}");
        }
        // `resolved.video` 是 `String`（不是一个 PathBuf）—— 取它所在目录当 clip 目录。
        let clip_dir = std::path::Path::new(&resolved.video)
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| project_path.to_path_buf());
        // ⚠️ **不传帧数**：这里那个 `frame_count` 是按写死的 30fps 估的，而工程的时间基是
        // 素材自己的（转译器写 60）—— 传进去会让底座只出一半。帧数由 `render_video_only`
        // 从转译产物里读（唯一权威）。
        // `style.subtitle.font` 要传下去：库里选了字体就得让底座用那一支（与预览/@font-face 同一份字节）。
        // `VTEDIT_STRICT=1`（`polish render --strict`）= 转译报告里 DROP > 0 就拒绝出片。
        let strict = std::env::var("VTEDIT_STRICT")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        render::dhampir::render_video_only(project_path, &clip_dir, &config.style.subtitle.font, strict)?;
    }
    tracing::info!("\n  -- Audio --");
    let t_audio = std::time::Instant::now();
    let ta = project_path.join("_temp_audio.aac");
    if let Err(e) = Ffmpeg::extract_audio(&resolved.video, &ta.to_string_lossy()) {
        cleanup(project_path, &tv, &ta);
        anyhow::bail!("音频提取失败: {e:#}");
    }

    if !sfx_data.is_empty() {
        tracing::info!("  -- SFX mix ({}) --", sfx_data.len());
        // 实时响度曲线 (ebur128 每 0.1s): 音效按所在时刻的视频响度动态调整。
        // 走缓存版（与出片页同一份），不再每次重跑整段 ebur128。
        let loudness_curve =
            measure_loudness_curve_cached(project_path, Path::new(&resolved.video));
        // 段级响度作为曲线测量失败的兜底
        let seg_loudness: Vec<(f64, f64, Option<f64>)> = config
            .segments
            .iter()
            .map(|s| {
                let db = measure_segment_loudness(&ta.to_string_lossy(), s.start, s.end - s.start);
                (s.start, s.end, db)
            })
            .collect();
        if !loudness_curve.is_empty() {
            tracing::info!("  v 响度曲线 {} 个采样点 (实时跟随)", loudness_curve.len());
        }
        // LLM volume → 相对强度系数: 0.5=正常, 0.3=轻, 0.9=响 (影响 ±20%)
        let llm_factor =
            |v: Option<f64>| -> f64 { v.map(|x| (0.5 + x).clamp(0.4, 1.2)).unwrap_or(1.0) };
        let ma = project_path.join("_temp_audio_m.aac");
        // 通用响度对齐: 每个音效文件测量自身响度 → gain = 视频时刻响度 - 8dB - 文件响度
        // 无论音效文件本身多响多轻, 最终都落在「视频响度 - 8dB」, 且实时跟随视频。
        // 同一文件被多个事件引用时只量一次（每个事件各起一个 ffmpeg 是纯浪费）。
        let mut file_db_cache: HashMap<String, f64> = HashMap::new();
        let se: Vec<_> = sfx_data
            .iter()
            .filter_map(|sd| {
                let f = project_path.join("sfx").join(sd.file.replace("sfx/", ""));
                // ⚠️ 缺文件**必须留痕**（以前是静默 filter —— 成片少了一个音效且查不到原因）：
                // 转译器那边同样情况会进 DROP 报告，这里至少打一行 warn（进任务日志）。
                if !f.exists() {
                    tracing::warn!("  ⚠ 音效文件缺失，这一条混不进成片（预览里同样不会响）：{}", sd.file);
                    return None;
                }
                let key = f.to_string_lossy().to_string();
                let file_db = *file_db_cache
                    .entry(key.clone())
                    .or_insert_with(|| measure_audio_loudness(&key).unwrap_or(-14.0));
                // 事件时刻的视频实时响度 (曲线优先, 段级兜底)
                let video_db = loudness_curve
                    .iter()
                    .rev()
                    .find(|(t, _)| sd.time >= *t)
                    .map(|(_, d)| *d)
                    .or_else(|| {
                        seg_loudness
                            .iter()
                            .find(|(s, e, _)| sd.time >= *s && sd.time < *e)
                            .and_then(|(_, _, d)| *d)
                    })
                    .unwrap_or(-26.0);
                // 目标: 音效最终响度 = 视频响度 - 8dB (点缀不盖人声); 再乘 LLM 相对系数
                let target = video_db - 8.0;
                let vol = 10.0_f64.powf((target - file_db) / 20.0) * llm_factor(sd.volume);
                let vol = vol.clamp(0.005, 1.0);
                Some(render::ffmpeg::SfxEvent {
                    time: sd.time,
                    file: f.to_string_lossy().to_string(),
                    volume: vol,
                    duration: sd.duration,
                })
            })
            .collect();
        if !se.is_empty() {
            // 混音 + 限幅保护 (防止音效峰值爆音)
            Ffmpeg::mix_sfx(&ta.to_string_lossy(), &se, &ma.to_string_lossy())?;
            let lim = project_path.join("_temp_audio_l.aac");
            Ffmpeg::apply_limiter(&ma.to_string_lossy(), &lim.to_string_lossy())?;
            move_file(&lim, &ma)?;
            move_file(&ma, &ta)?;
        }
        tracing::info!("  ⏱ 音效混音 {:.1}s", t_audio.elapsed().as_secs_f64());
    }

    tracing::info!("\n  -- Mux --");
    let t_mux = std::time::Instant::now();
    let op_path = if Path::new(&resolved.output).is_absolute() {
        PathBuf::from(&resolved.output)
    } else {
        project_path.join(&resolved.output)
    };
    let op_path = unique_output_path(&op_path);
    let op = op_path.to_string_lossy().into_owned();
    if let Err(e) = Ffmpeg::mux(&tv.to_string_lossy(), &ta.to_string_lossy(), &op) {
        cleanup(project_path, &tv, &ta);
        anyhow::bail!("音视频合流失败: {e:#}");
    }

    tracing::info!("  ⏱ 合流 {:.1}s", t_mux.elapsed().as_secs_f64());

    // 手动屏蔽区间：导出时直接从成片中剪掉
    let mute_ranges: Vec<(f64, f64)> = config
        .mute_ranges
        .iter()
        .filter_map(|r| r.normalized())
        .collect();
    if !mute_ranges.is_empty() {
        let t_mute = std::time::Instant::now();
        let muted_out = project_path.join("_temp_muted.mp4");
        match Ffmpeg::cut_ranges(&op, &mute_ranges, &muted_out.to_string_lossy()) {
            Ok(_) => {
                if std::fs::copy(&muted_out, &op_path).is_ok() {
                    std::fs::remove_file(&muted_out).ok();
                    tracing::info!(
                        "  ✂ 已导出去除 {} 个屏蔽区间（整段重编码 ⏱ {:.1}s）",
                        mute_ranges.len(),
                        t_mute.elapsed().as_secs_f64()
                    );
                } else {
                    tracing::info!("  ⚠ 屏蔽区间结果写入失败，保留原成片");
                }
            }
            Err(e) => tracing::info!("  ⚠ 屏蔽区间导出失败: {e:#}"),
        }
    }

    cleanup(project_path, &tv, &ta);
    // ⚠️ **封面不再搭渲染的便车**（2026-10-02 用户拍板"视频导出就只导出"）：
    // 原来这里串 `cover::cmd_cover(project_path, Some(&op), None)`，导致
    // ① 每次导出强制重截关键帧/重写封面（用户手导的 `_cover_frame.jpg` 被覆盖的心智负担）；
    // ② 渲染任务时长被封面通道（字体库非空检查/光栅化）拖长；
    // ③ 封面失败把"成片已导出"的任务尾行搅成 warning。
    // 封面走自己的入口：CLI `yekitrim polish cover` / WebUI 封面设计器「生成封面」
    // （`POST /api/polish/cover`，独立任务）。截图帧来源那里本来就会回落 `resolved.video`。
    print_final_stats(&op, start_time);
    Ok(())
}

/// 与 `render::sound::SoundEvent` 相同；保留旧 pub 名避免破坏外部引用。
pub type SoundEvent = crate::render::sound::SoundEvent;

// ============================================================
// process
// ============================================================

pub struct ResolvedPaths {
    pub video: String,
    pub subtitles: String,
    pub danmaku: String,
    pub metadata: String,
    pub output: String,
}

pub fn resolve_paths(project_path: &Path, config: &ProjectFile) -> ResolvedPaths {
    let to_abs = |relative: &str| -> String {
        if relative.is_empty() {
            return String::new();
        }
        let p = Path::new(relative);
        if p.is_relative() {
            project_path.join(p).to_string_lossy().to_string()
        } else {
            p.to_string_lossy().to_string()
        }
    };

    ResolvedPaths {
        video: to_abs(&config.source.video),
        subtitles: to_abs(&config.source.subtitles),
        danmaku: to_abs(&config.source.danmaku),
        metadata: to_abs(&config.source.metadata),
        output: to_abs(&config.project.output),
    }
}

// ============================================================
// 辅助
// ============================================================
/// 注入资产路径到 config JSON (_assets 字段)
pub fn inject_assets(project_path: &Path, config_json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(config_json) {
        Ok(mut cfg) => {
            let downloader = assets::downloader::AssetDownloader::new(project_path);
            cfg["_assets"] = downloader.asset_map();
            serde_json::to_string(&cfg).unwrap_or_else(|_| config_json.to_string())
        }
        Err(_) => config_json.to_string(),
    }
}

/// 预解码所有 sticker 的 GIF 帧为 PNG，返回 JSON（按 name→{frames, delays}）
/// 仅 "emoji" 来源的 GIF 有动画帧，"irasu" 和 "local" 来源是静态图
pub fn prepare_sticker_data(project_path: &Path, config: &project::ProjectFile) -> String {
    let b64_engine = base64::engine::general_purpose::STANDARD;

    // ① 收集这份配置真正用到的贴纸（按名字去重，保持首次出现顺序）
    let mut wanted: Vec<(String, PathBuf)> = Vec::new();
    for seg in &config.segments {
        for ev in &seg.events {
            if let project::Event::Sticker {
                name: Some(name), ..
            } = ev
            {
                if name.is_empty() || wanted.iter().any(|(n, _)| n == name) {
                    continue;
                }
                let gif_path = project_path
                    .join("assets")
                    .join("stickers")
                    .join(format!("{}.gif", name));
                if gif_path.exists() {
                    wanted.push((name.clone(), gif_path));
                }
            }
        }
    }
    if wanted.is_empty() {
        return "{}".into();
    }

    // ② 解码结果带缓存：GIF 没变（名字 + mtime + 大小一致）就直接复用上次的。
    //    debug 构建下 6 个贴纸要解 40s（实测），而贴纸资产几乎从不变化。
    let cache_file = project_path.join("data").join("sticker_frames.json");
    let key = wanted
        .iter()
        .map(|(n, p)| format!("{n}|{}", media_key(p).unwrap_or_default()))
        .collect::<Vec<_>>()
        .join(";");
    if let Ok(raw) = std::fs::read_to_string(&cache_file) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            if v.get("key").and_then(|x| x.as_str()) == Some(key.as_str()) {
                if let Some(data) = v.get("data") {
                    tracing::debug!("  贴纸帧解码命中缓存（{} 个）", wanted.len());
                    return data.to_string();
                }
            }
        }
    }

    // ③ 逐个解码 GIF → PNG 帧 → base64
    let mut data: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    for (name, gif_path) in &wanted {
        if let Ok(frames) = render::gif::decode_gif_frames_png(gif_path) {
            let frame_b64: Vec<String> = frames.iter().map(|f| b64_engine.encode(&f.0)).collect();
            let delays: Vec<u16> = frames.iter().map(|f| f.1).collect();
            data.insert(
                name.clone(),
                serde_json::json!({
                    "frames": frame_b64,
                    "delays": delays,
                }),
            );
        }
    }
    let json = serde_json::Value::Object(data.clone()).to_string();
    let _ = std::fs::create_dir_all(project_path.join("data"));
    let _ = std::fs::write(
        &cache_file,
        serde_json::json!({ "key": key, "data": data }).to_string(),
    );
    json
}

/// 用 ffprobe 获取视频文件实际时长 (秒)
/// clip.json 的 duration 是 B站原始元数据含 padding，不可信
/// 获取视频宽高 (ffprobe stream 信息), 竖屏检测用
pub fn get_video_size(video_path: &str) -> Option<(u32, u32)> {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0",
            video_path,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = std::str::from_utf8(&out.stdout).ok()?.trim().to_string();
    let mut parts = s.split(',');
    let w = parts.next()?.trim().parse::<u32>().ok()?;
    let h = parts.next()?.trim().parse::<u32>().ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

pub fn get_video_duration(video_path: &str) -> Option<f64> {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            video_path,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = std::str::from_utf8(&out.stdout).ok()?.trim().to_string();
    s.parse::<f64>().ok().filter(|d| *d > 0.0)
}

/// 递归复制目录
pub fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        std::fs::create_dir_all(dst)?;
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

pub fn cleanup(_project_path: &Path, temp_video: &Path, temp_audio: &Path) {
    for p in [temp_video, temp_audio] {
        std::fs::remove_file(p).ok();
    }
}

/// 用 ffmpeg volumedetect 检测音频响度，返回 mean_volume (dB)
/// 检测音频某时间段的响度 (mean_volume dB) — 段级响度对齐用
pub fn measure_segment_loudness(audio_path: &str, start: f64, dur: f64) -> Option<f64> {
    if dur <= 0.0 {
        return None;
    }
    let output = Command::new("ffmpeg")
        .args([
            "-ss",
            &format!("{:.2}", start),
            "-t",
            &format!("{:.2}", dur),
            "-i",
            audio_path,
            "-af",
            "volumedetect",
            "-f",
            "null",
            "-",
        ])
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .output()
        .ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    for line in stderr.lines() {
        if let Some(val_str) = line.split("mean_volume: ").nth(1)
            && let Some(db_str) = val_str.trim().strip_suffix(" dB")
            && let Ok(db) = db_str.parse::<f64>()
        {
            return Some(db);
        }
    }
    None
}

/// 用 ebur128 测量音频瞬时响度曲线 (每 ~0.1s 一个点, LUFS)
/// 返回 [(时间, 响度)] — 音效按所在时刻的响度实时调整音量
///
/// ⚠ **`-vn` 不能省**：这条只关心音频。不带它 ffmpeg 会把整段**视频**也解码一遍再丢掉
/// （实测同一片段 5.6s → 0.4s；曲线逐点一致，1330 个采样点全等）—— 它只是不往输出写视频流，
/// 音频滤镜链一分不变。剪切收尾会给每个新片段预热这条曲线
/// （`yekitrim-service` 的 `core_pipeline::warm_loudness_cache`），热路径上一次剪切要跑 N 遍。
pub fn measure_loudness_curve(audio_path: &str) -> Vec<(f64, f64)> {
    let output = Command::new("ffmpeg")
        .args([
            "-i",
            audio_path,
            // 输出参数段：作用于 `-f null -`，只解码音频
            "-vn",
            "-af",
            "ebur128=peak=true",
            "-f",
            "null",
            "-",
        ])
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .output()
        .ok();
    let Some(output) = output else {
        return Vec::new();
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut curve: Vec<(f64, f64)> = Vec::new();
    for line in stderr.lines() {
        // [Parsed_ebur128_0 @ ...] t: 0.399312  TARGET:-23 LUFS    M: -27.6
        let Some(t_pos) = line.find(" t: ") else {
            continue;
        };
        let Some(m_pos) = line.find(" M:") else {
            continue;
        };
        let t_str: String = line[t_pos + 4..]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        let m_str: String = line[m_pos + 3..]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        let (Ok(t), Ok(m)) = (t_str.parse::<f64>(), m_str.parse::<f64>()) else {
            continue;
        };
        // 跳过静音窗口 (-120 表示无信号)
        if m > -100.0 {
            curve.push((t, m));
        }
    }
    curve
}

/// 带磁盘缓存的响度曲线：同一份媒体只跑一次整段 ebur128。
///
/// 出片页 (`server::RenderServer::build_html`) 与 `cmd_render` 的混音各要一份曲线，
/// 以前每次渲染都要重跑两遍；预览服务的 HTML 还是**每个请求**重建 → 每刷一次预览页再烧一遍。
/// 缓存按「文件名 + mtime + 字节数」失效，落在 `<工程>/data/loudness.json`。
///
/// ⚠ 出片混音这里量的是**源视频**而不是抽出来的 `_temp_audio.aac`：两者是同一段音频
/// （AAC 只是重编码），量出来的曲线差异 ≪1dB，但换来的是可缓存、且与页面用的是同一份曲线。
pub fn measure_loudness_curve_cached(cache_dir: &Path, media_path: &Path) -> Vec<(f64, f64)> {
    let cache_file = cache_dir.join("data").join("loudness.json");
    let key = media_key(media_path);
    if let Some(key) = key.as_deref() {
        if let Ok(raw) = std::fs::read_to_string(&cache_file) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if v.get("key").and_then(|x| x.as_str()) == Some(key) {
                    if let Some(points) = v.get("points").and_then(|p| p.as_array()) {
                        let curve: Vec<(f64, f64)> = points
                            .iter()
                            .filter_map(|p| Some((p.get("t")?.as_f64()?, p.get("db")?.as_f64()?)))
                            .collect();
                        if !curve.is_empty() {
                            tracing::debug!("  响度曲线命中缓存（{} 个采样点）", curve.len());
                            return curve;
                        }
                    }
                }
            }
        }
    }
    let curve = measure_loudness_curve(&media_path.to_string_lossy());
    if curve.is_empty() || key.is_none() {
        return curve; // 量不到就不写缓存，下次再试
    }
    let points: Vec<serde_json::Value> = curve
        .iter()
        .map(|(t, db)| serde_json::json!({ "t": t, "db": db }))
        .collect();
    let _ = std::fs::create_dir_all(cache_dir.join("data"));
    let _ = std::fs::write(
        &cache_file,
        serde_json::json!({ "key": key, "points": points }).to_string(),
    );
    curve
}

/// 缓存键：文件名 + 修改时间(秒) + 字节数（内容变了就失效）。
fn media_key(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(format!("{name}|{mtime}|{}", meta.len()))
}

/// 检测音频整体响度 (mean_volume dB)
pub fn measure_audio_loudness(audio_path: &str) -> Option<f64> {
    let output = Command::new("ffmpeg")
        .args(["-i", audio_path, "-af", "volumedetect", "-f", "null", "-"])
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .output()
        .ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    for line in stderr.lines() {
        if let Some(val_str) = line.split("mean_volume: ").nth(1)
            && let Some(db_str) = val_str.trim().strip_suffix(" dB")
            && let Ok(db) = db_str.parse::<f64>()
        {
            return Some(db);
        }
    }
    None
}

pub fn print_final_stats(output: &str, start: std::time::Instant) {
    let elapsed = start.elapsed().as_secs_f64();
    let size = std::fs::metadata(output)
        .map(|m| m.len() as f64 / 1048576.0)
        .unwrap_or(0.0);
    tracing::info!(" FINISHED: {} ({:.1}MB, {:.0}s)", output, size, elapsed);
}

#[cfg(test)]
mod tests {
    use super::{
        canvas_supported_effects, effect_spec_json, missing_support, unique_output_path, used_effect_types,
    };
    use std::path::{Path, PathBuf};

    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "yekitrim-unique-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&base).unwrap();
            TmpDir(base)
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 成片与封面共用这一条规则：**同名不覆盖**，已存在就往后找 `_1` / `_2`。
    /// 用户明确要求过（"如果已经存在导出文件，就加 _n，类似封面的操作"）——
    /// 别改成覆盖写：`Ffmpeg::mux` 带 `-y`，路径一旦重名就直接盖掉上一份成片。
    #[test]
    fn unique_output_path_appends_counter() {
        let t = TmpDir::new("clip");
        let base = t.0.join("vtuber-clip.mp4");

        // 不存在 → 原样返回
        assert_eq!(unique_output_path(&base), base);

        std::fs::write(&base, b"1").unwrap();
        let one = unique_output_path(&base);
        assert_eq!(one.file_name().unwrap().to_str().unwrap(), "vtuber-clip_1.mp4");

        std::fs::write(&one, b"2").unwrap();
        let two = unique_output_path(&base);
        assert_eq!(two.file_name().unwrap().to_str().unwrap(), "vtuber-clip_2.mp4");

        // 封面同一套（cover.jpg → cover_1.jpg）
        let cover = t.0.join("cover.jpg");
        std::fs::write(&cover, b"x").unwrap();
        assert_eq!(unique_output_path(&cover).file_name().unwrap().to_str().unwrap(), "cover_1.jpg");

        // 无扩展名的路径也要能用
        let bare = t.0.join("noext");
        std::fs::write(&bare, b"x").unwrap();
        assert_eq!(unique_output_path(&bare).file_name().unwrap().to_str().unwrap(), "noext_1");

        // 目录仍在原处（不会把产物写到别的地方）
        assert_eq!(unique_output_path(Path::new("rel/keep.mp4")), Path::new("rel/keep.mp4"));
    }

    /// 1×1 GIF89a（透明）：贴纸帧解码缓存的最小素材。
    const TINY_GIF: &[u8] = &[
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00,
        0x00, 0xFF, 0xFF, 0xFF, 0x21, 0xF9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2C, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3B,
    ];

    fn sticker_project(tag: &str) -> TmpDir {
        let t = TmpDir::new(tag);
        std::fs::create_dir_all(t.0.join("assets").join("stickers")).unwrap();
        std::fs::write(t.0.join("assets").join("stickers").join("blob.gif"), TINY_GIF).unwrap();
        t
    }

    fn config_with_sticker() -> crate::project::ProjectFile {
        let mut cfg = crate::project::ProjectFile::example();
        cfg.segments[0].events = vec![crate::project::Event::Sticker {
            time: 1.0,
            end: Some(3.0),
            keyword: "blob".into(),
            sticker_tag: None,
            name: Some("blob".into()),
            source: None,
            x: "0%".into(),
            y: "0%".into(),
            sfx: None,
            sfx_volume: None,
            manual: false,
        }];
        cfg
    }

    /// 贴纸帧解码带缓存：GIF 没变就直接复用（debug 构建下 6 个贴纸要解 40s = 整场渲染的 14%）。
    /// 缓存必须**按内容失效**：换了 GIF 就得重新解，否则贴纸会一直显示旧图。
    #[test]
    fn sticker_frames_are_cached_and_invalidated_on_change() {
        let t = sticker_project("stk-cache");
        let cfg = config_with_sticker();

        let first = super::prepare_sticker_data(&t.0, &cfg);
        assert!(first.contains("blob"), "首次应解出贴纸帧: {first}");
        let cache = t.0.join("data").join("sticker_frames.json");
        assert!(cache.exists(), "缓存文件应落盘");

        // 第二次：命中缓存，内容一致
        let second = super::prepare_sticker_data(&t.0, &cfg);
        assert_eq!(first, second);

        // 换掉 GIF（内容 + 大小都变）→ 缓存 key 失效，重新解码
        let gif = t.0.join("assets").join("stickers").join("blob.gif");
        let mut bigger = TINY_GIF.to_vec();
        bigger.extend_from_slice(&[0u8; 16]);
        std::fs::write(&gif, &bigger).unwrap();
        let cached_raw = std::fs::read_to_string(&cache).unwrap();
        let _ = super::prepare_sticker_data(&t.0, &cfg);
        let new_raw = std::fs::read_to_string(&cache).unwrap();
        assert_ne!(cached_raw, new_raw, "GIF 变了之后缓存 key 必须变");
    }

    /// 只读诊断：贴纸帧解码的耗时（未命中 vs 命中缓存）。会写/删该工程的缓存文件。
    ///
    /// `VT_POLISH_DIR=<项目目录> cargo test -p yekitrim-polish --lib diagnose_sticker_cache -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn diagnose_sticker_cache() {
        let Ok(dir) = std::env::var("VT_POLISH_DIR") else {
            eprintln!("需要 VT_POLISH_DIR=<项目目录>");
            return;
        };
        let dir = PathBuf::from(dir);
        let cfg = crate::project::load_project(&dir).expect("加载 polish.toml");
        let cache = dir.join("data").join("sticker_frames.json");
        if cache.exists() {
            eprintln!("已有缓存: {} 字节", std::fs::metadata(&cache).map(|m| m.len()).unwrap_or(0));
        }
        let _ = std::fs::remove_file(&cache);

        let t0 = std::time::Instant::now();
        let cold = super::prepare_sticker_data(&dir, &cfg);
        let cold_secs = t0.elapsed().as_secs_f64();

        let t1 = std::time::Instant::now();
        let warm = super::prepare_sticker_data(&dir, &cfg);
        let warm_secs = t1.elapsed().as_secs_f64();

        assert_eq!(cold, warm, "命中缓存的输出必须与重新解码一致");
        eprintln!(
            "贴纸帧 JSON {} 字节：解码 {cold_secs:.1}s → 命中缓存 {warm_secs:.2}s",
            cold.len()
        );
    }

    // ---- 效果词汇表（`anchors.canvas` 与 `effects[].id` 的对齐）----
    //
    // ⚠️ 这两条**不再是出片闸门**（canvas 兜底腿已于 2026-10-01 删除）。它们守的是**词汇表自身**：
    // `anchors.canvas` 与模板 `index.html` 的漂移防线 —— 词汇表是转译器与出片两侧共同的输入，
    // 写错了不会有任何东西报错，只会静默换个渲染结果。

    /// **支持矩阵的纯函数**：不在集合里的效果必须被点名（去重 + 保持出现顺序）。
    #[test]
    fn canvas_guard_names_the_unsupported_effects() {
        let supported: std::collections::HashSet<String> =
            ["shake", "flash", "blur"].iter().map(|s| s.to_string()).collect();
        // 全支持 ⇒ 不报
        assert!(missing_support(["shake", "blur"].into_iter(), &supported).is_empty());
        // 有一个不支持 ⇒ 点名它（且**只**点它）
        assert_eq!(
            missing_support(["shake", "brand_new", "blur"].into_iter(), &supported),
            vec!["brand_new".to_string()]
        );
        // 去重 + 保持出现顺序
        assert_eq!(
            missing_support(["x", "shake", "y", "x"].into_iter(), &supported),
            vec!["x".to_string(), "y".to_string()]
        );
        // 空工程 ⇒ 空
        assert!(missing_support(std::iter::empty(), &supported).is_empty());
    }

    /// **词汇表本身要有内容**：`anchors.canvas` 至少得覆盖当前这批效果（模板 canvas 实现的那批）。
    #[test]
    fn canvas_support_matrix_comes_from_the_vocabulary() {
        let supported = canvas_supported_effects().expect("内嵌的词汇表必须可解析");
        for id in ["shake", "flash", "blur", "noise", "pulse", "overlay", "camera"] {
            assert!(supported.contains(id), "词汇表里 {id} 应当在 canvas 支持集合里：{supported:?}");
        }
        // 每个条目都必须是词汇表里的类型（防止 `anchors.canvas` 写了拼错的键）
        let spec: serde_json::Value = serde_json::from_str(&effect_spec_json()).unwrap();
        let ids: Vec<String> = spec["effects"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["id"].as_str().map(str::to_string))
            .collect();
        for k in &supported {
            assert!(ids.contains(k), "anchors.canvas 里的 `{k}` 不在 effects 列表里");
        }
    }

    /// 工程里用到的事件类型要能从 `ProjectFile` 直接读出来（走序列化 ⇒ **加新类型不用改这里**）。
    #[test]
    fn used_effect_types_reads_the_project() {
        let mut cfg = crate::project::ProjectFile::example();
        cfg.segments[0].events.clear();
        let t = used_effect_types(&cfg);
        assert!(t.is_empty(), "清空事件后应当读不到类型：{t:?}");

        cfg.segments[0].events.push(
            serde_json::from_value(serde_json::json!({"type": "shake", "time": 0.0, "end": 0.3, "intensity": 1.0}))
                .unwrap(),
        );
        cfg.segments[0].events.push(
            serde_json::from_value(serde_json::json!({"type": "blur", "time": 1.0, "end": 1.3})).unwrap(),
        );
        assert_eq!(used_effect_types(&cfg), vec!["shake".to_string(), "blur".to_string()]);
    }
}
