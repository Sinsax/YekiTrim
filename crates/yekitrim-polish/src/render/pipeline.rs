//! polish::render::pipeline：出片主链路（转译成底座工程 → `dhampir.exe` 整段出片 → ffmpeg 编码混流 → 统计）。
//!
//! 入口：`cmd_render`、`resolve_paths` / `ResolvedPaths`、`inject_assets`、`prepare_sticker_data`、`unique_output_path`、`measure_loudness_curve_cached`、`cleanup`。
//! 边界：子进程链路 —— 输出必须落文件、绝不能用管道（ffmpeg 是孙进程，管道永不 EOF）；`<工程>/data/` 的磁盘缓存别去掉；同名产物加 `_1` / `_2` —— 见 `crates/yekitrim-polish/src/render/AGENTS.md`

use base64::Engine;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::project::ProjectFile;
use yekitrim_render_core::types::Edl;
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
            // ⚠️ **2026-10-09：发布包里的落点由 `spec/` 改成 `runtime/`**
            //（与转译器合并成一个目录）。老 `<程序目录>/spec/` 仍作**回落**：
            // 存量安装升级时那个目录还在，不给回落会让 doctor 与转译器都读不到权威词汇表。
            // 新位排前面（两个都在时优先用新的）。`effects.json` 的名字不变。
            candidates.push(dir.join("runtime").join("effects.json"));
            candidates.push(dir.join("spec").join("effects.json"));
            if let Some(up2) = dir.parent().and_then(Path::parent) {
                candidates.push(up2.join("runtime").join("effects.json"));
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


/// **合流前那道帧数断言该拿什么数去卡**（缺陷 D1 的判据本体，抽出来是为了可测）。
///
/// # 为什么要抽成函数
///
/// 这条分流逻辑**写错了不报错、只表现为"要么误报要么放空"** —— 正是本仓最该被单测钉住的形状。
/// 实测踩过（2026-10-11，`--keep-muted` 出片必然失败）：无 EDL 时 `edl_total_frames` 是
/// **mute 补集**的长度（12s 带 3s 屏蔽 ⇒ 540 帧），而画面那条腿拿到的是**整条时间线**
/// （`dhampir.rs::render_video_only` 硬编码 `keep_muted = true`）⇒ 断言在 `cut_ranges`
/// **之前**拿"剪完之后"的数去卡 ⇒ 每次都红。
///
/// # 判据
///
/// `has_edl == true` ⇒ 尾部那道 `cut_ranges` **被跳过**（`:1038` 的判据就是它）⇒
/// 合流点长度 **==** 成片长度 ⇒ 拿派生值卡。
/// `has_edl == false` ⇒ `mute_ranges` 的活还在后头（`cut_ranges`）⇒ 合流点上两条腿**都是全长**，
/// 派生值（补集）是"剪完之后"的数 ⇒ **这里不能卡**，改由剪完之后那道断言卡。
///
/// ⚠️ **别改成无条件 `None`**：有 EDL 那条路是这道断言的**主战场**
/// （重放 / 冷开场 / 变速全靠它挡"画面漏段"），放空等于把主战场让出去。
pub(crate) fn mux_check_frames(has_edl: bool, edl_total_frames: Option<u64>) -> Option<u64> {
    if has_edl {
        edl_total_frames
    } else {
        None
    }
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

/// 多版本成片（§9）：同一素材出 N 条**开头不同**的片子，用户挑一条。
///
/// ⚠️ **不新造渲染机制**（规格 §5.3 的原话："多版本不需要改这套 —— 就是跑 N 次，
/// 每次覆盖 `[editing]` 的 selected"）：这里逐版把开头挪到第 i 个候选，
/// 然后调**同一个** [`cmd_render`]。另造一套并行出片 = 两条渲染路径，
/// 而它们的像素**必须**一致 —— 那种"两份实现"是本仓反复踩的坑。
///
/// ⚠️ **产物靠 `unique_output_path` 排开**（`_1`/`_2`），这是既有能力、不改它。
/// 所以"版本 0"就是普通的 `export.mp4`，版本 1 是 `export_1.mp4`，依此类推。
///
/// ⚠️ **算不出候选时怎么办**：报告 + 退回"只出一版"。
/// **不许**硬凑几个假候选 —— 那会让用户以为"有三个选择"，而三个其实是同一条片子。
pub async fn cmd_render_versions(
    project_path: &Path,
    versions: usize,
) -> anyhow::Result<()> {
    use crate::project::{ending_candidates, opening_candidates, Cue};

    if versions <= 1 {
        // 与不做多版本**逐字节相同**（R4）。
        return cmd_render(project_path).await;
    }

    let project_path =
        &std::path::absolute(project_path).unwrap_or_else(|_| project_path.to_path_buf());

    // 字幕 -> 候选判据要的 `Cue`。
    let cues: Vec<Cue> = {
        let p = project_path.join("data").join("subtitles.json");
        std::fs::read_to_string(&p)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|v| v.as_array().cloned())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| {
                        let start = s.get("start")?.as_f64()?;
                        let end = s.get("end")?.as_f64()?;
                        let text = s.get("text")?.as_str()?.to_string();
                        Some(Cue { start, end, text })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    // 爆点窗口（§6 的爆点判据已经在 process 里算过并落盘）。
    let hotspots: Vec<(f64, f64)> = {
        let p = project_path.join("data").join("beats.json");
        std::fs::read_to_string(&p)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|v| v.get("beats").and_then(|b| b.as_array()).cloned())
            .map(|arr| {
                arr.iter()
                    .filter_map(|b| {
                        let s = b.get("start")?.as_f64()?;
                        let e = b.get("end")?.as_f64()?;
                        Some((s, e))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let openings = opening_candidates(&cues, &hotspots, versions);
    let endings = ending_candidates(&cues, &hotspots, versions);
    tracing::info!(
        "  · 多版本成片（§9）：字幕 {} 条、爆点 {} 处 ⇒ 开头候选 {} 个、结尾候选 {} 个",
        cues.len(),
        hotspots.len(),
        openings.len(),
        endings.len(),
    );

    if openings.len() < 2 {
        tracing::warn!(
            "  ⚠ 只有 {} 个开头候选（要 {} 版）—— 无法凑出版本差异，**退回只出一版**",
            openings.len(),
            versions
        );
        tracing::warn!("     （候选靠「句末 / 静默 / 避开爆点」三条判据得出；素材太短或没有自然断点时会不足）");
        return cmd_render(project_path).await;
    }

    let n = openings.len().min(versions);
    // ⚠️⚠️ **先确认"换开头"这件事真的做得到，再开始出片**。
    //
    // 实测踩到的**静默假成功**（2026-10-12）：工程缺 `data/clip-meta.json` 时
    // `duration_for_versions` 回落到 **0.0**，而 `edl_clips_from_mute_complement(ranges, 0.0)`
    // 对"非正时长"返回**空 Vec** ⇒ `set_opening` 在 `base.is_empty()` 那一句**直接 return**，
    // **一个字节都不写**。于是循环照常跑 N 遍、日志照常打「版本 2/3：开头选在 2.40s」，
    // 而**每一版渲染的输入完全相同** —— 三个产物**逐字节相同**（MD5 全等于 `a6f7f120…`）。
    // 更糟的是收工那句还宣称"已恢复到第一版的开头"。**报告与事实相反**。
    //
    // ⇒ 判据：**每一版的开头值必须真的落到 `cfg.edl.clips[0].source_start` 上**，
    //    落不下去就**当场报错**，而不是出一堆一样的片子。
    //    （宁可失败得吵，也不要"看起来成功、拿到的是同一份"。这正是 §9 的验收标准
    //     "两版 MD5 不同"能成立的前提。）
    if duration_for_versions(project_path) <= 0.0 {
        anyhow::bail!(
            "多版本成片（§9）无法换开头：工程里读不到时长 \
             （`data/clip-meta.json` 缺失或没有 `duration` 字段）—— \
             时长是 0 时「换开头」写不进 EDL，会输出 {n} 份**逐字节相同**的成片。\
             ⚠️ 先跑一次 `polish process` 生成 clip-meta.json，或改用不带 `--versions` 的普通出片。"
        );
    }
    for i in 0..n {
        let at = openings[i].at;
        // ⚠️ 逐版**改一次开头**再出片 —— 改的是**工程文件本身**，
        // 所以下一版必须能把它改回来/改成下一档（见下面 `set_first_segment_start`）。
        let total = duration_for_versions(project_path);
        crate::project::update_polish_toml(project_path, |cfg| {
            set_opening(cfg, at, total);
        })
        .await?;
        // ⚠️ **写后自检**：确认这一版的开头真的落在磁盘上（见上面那段"静默假成功"）。
        //    没有这一步，`set_opening` 里任何一个提前 return 都会退化成"出一堆一样的片子"。
        let applied = crate::project::load_project(project_path)?
            .edl
            .clips
            .first()
            .map(|c| c.source_start);
        match applied {
            Some(s) if (s - at).abs() < 1e-6 => {}
            other => anyhow::bail!(
                "多版本成片（§9）第 {}/{n} 版的开头（{at:.2}s）**没有写进工程**\
                 （磁盘上是 {other:?}）—— 继续跑只会得到一份与上一版相同的成片。\
                 常见原因：`data/clip-meta.json` 缺 `duration`、或工程里有手排 EDL 与候选不兼容。",
                i + 1
            ),
        }
        tracing::info!(
            "  ▶ 版本 {}/{}：开头选在 {at:.2}s（{}）（§9）",
            i + 1,
            n,
            openings[i].reason
        );
        cmd_render(project_path).await?;
    }

    // ⚠️ **收工后把开头恢复到第 0 个候选**：否则用户下次直接点"出片"，
    // 拿到的会是**最后一版**的剪辑 —— 而工程文件上看不出这件事。
    let total = duration_for_versions(project_path);
    let had_user_edl = {
        // 收工前先看一眼：这趟有没有动过用户的手排 EDL。
        let cfg = crate::project::load_project(project_path)?;
        !cfg.user_edl_backup.is_empty()
    };
    crate::project::update_polish_toml(project_path, |cfg| {
        if had_user_edl {
            // 有备份 ⇒ 整份还原用户的手排 EDL。
            finish_versions_restore(cfg, openings[0].at);
        } else {
            // 本来就没有手排 EDL ⇒ 恢复成第一版候选的开头（那是我们自己算的）。
            set_opening(cfg, openings[0].at, total);
        }
    })
    .await?;
    if had_user_edl {
        tracing::info!(
            "  ✓ 多版本成片完成：{n} 版（产物按 _1/_2 排开）；**工程已整份还原你手排的剪辑点**（§9）"
        );
    } else {
        tracing::info!(
            "  ✓ 多版本成片完成：{n} 版（产物按 _1/_2 排开）；工程已恢复到第一版的开头（§9）"
        );
    }
    if endings.len() > 1 {
        tracing::info!(
            "  · 结尾候选有 {} 个（本趟**只变开头** —— 同时变头尾会让两个变量的效果混在一起，用户无法归因）",
            endings.len()
        );
    }
    Ok(())
}

/// 读工程时长（多版本换开头要用它算 EDL；读不到就退回 0 = 不改）。
fn duration_for_versions(project_path: &std::path::Path) -> f64 {
    std::fs::read_to_string(project_path.join("data").join("clip-meta.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("duration").and_then(|d| d.as_f64()))
        .unwrap_or(0.0)
}

/// 把成片的**开头**挪到 `at`（多版本成片的"换开头"）。
///
/// ⚠️ **改的是 EDL，不是 `segments[0]`** —— 这条是实测踩出来的：
/// 第一版改 `segments[0].start/end`，三版跑完日志一行不差、
/// **三个产物逐字节相同**。原因是成片时间线来自
/// [`ProjectFile::resolve_edl`]，而它的输入是 `edl.clips`（手排过）
/// 或 `mute_ranges` 的**补集**（没排过）—— `segments[]` 根本不在那条链上。
///
/// 做法：写一条**显式 EDL**（`[0, at)` 这一段是"被掐掉的开头"，不写进 clips），
/// 让第一个 clip 从 `at` 开始。显式 EDL 一非空，`resolve_edl` 就直接用它。
///
/// ⚠️ **其余部分沿用原来的口径**：原工程有手排 EDL 就只用它**第一个 clip 的起点**
/// 当"原开头"，其余 clips 原样保留（我们不重排用户的剪辑，只挪起点）。
/// 原工程没有 EDL ⇒ 用 `mute_ranges` 的补集当基准。
///
/// ⚠️ `at` 落在原开头**之前**或无意义时**什么都不做**（保守：宁可这版与上一版相同，
/// 也不要把开头挪到一个不存在的位置）。
fn set_opening(cfg: &mut ProjectFile, at: f64, total_duration: f64) {
    use crate::project::edl_clips_from_mute_complement;

    // 基准 clips：手排的优先，否则用屏蔽的补集。
    let base: Vec<yekitrim_render_core::types::EdlClip> = if !cfg.edl.clips.is_empty() {
        cfg.edl.clips.clone()
    } else {
        edl_clips_from_mute_complement(&cfg.mute_ranges, total_duration)
    };
    if base.is_empty() || !at.is_finite() {
        return;
    }
    // ⚠️ **不许往前走**（`at < orig_start`）这条**只在逐版推进时**才成立。
    // 收工时的"恢复到第一版"**必须能往回退** —— 实测踩到：三版跑完 EDL 起点已经是
    // 4.1，恢复调用被这条判据拒绝 ⇒ 工程停在**最后一版**的开头，
    // 而日志照旧打"已恢复到第一版"。用户下次直接点出片会拿到第 3 版。
    //
    // ⇒ 判据拆开：**只要求 `at` 落在 `[0, 第一个 clip 的终点)` 内**（合法的源时刻），
    //    "是否比当前靠前"由调用方决定（`cmd_render_versions` 里逐版是递增的）。
    if at < 0.0 || !at.is_finite() || at >= base[0].source_end - 1e-9 {
        return;
    }

    // ⚠️⚠️ **第一次动用户的手排 EDL 之前，先把它整份备份下来**。
    //
    // 实测踩到的**真数据丢失**：用户排了两段（`[1.0,4.0]` + `[5.0,8.0]`），
    // 跑一次 `--versions 3` 之后第一段起点变成 **0.3**（候选值），
    // 而日志打的是"已恢复到第一版的开头" —— ★ **报告与事实相反**：
    // "第一版的开头"是**候选的** 0.3，不是用户的 1.0。原值一旦丢掉就找不回来。
    //
    // ⚠️ 为什么不干脆"跑完把 `edl.clips` 清空"（回到没有 EDL 的状态）：
    //    那等于**删掉用户的剪辑** —— 他手排那两段之后就一直该是他排的那样。
    //    我们只该"临时挪开头、跑完还回去"，不该改他的剪辑。
    //
    // ⚠️ **只备份"用户真的手排过"的那份**（`cfg.edl.clips` 原本非空）。
    //    屏蔽补集推出来的基准**不算** —— 它不是用户意图，而且备份它会让
    //    **每个跑过多版本的老工程都多出一个 `user_edl_backup` 键**，
    //    把 `polish.toml` 弄脏（而那个键对它们毫无意义）。
    let user_had_edl = !cfg.edl.clips.is_empty();
    if user_had_edl && cfg.user_edl_backup.is_empty() {
        cfg.user_edl_backup = cfg.edl.clips.clone();
    }

    let mut clips = base;
    clips[0].source_start = at;
    cfg.edl.clips = clips;
}

/// 多版本成片收工：**把 EDL 还原成用户原来的样子**。
///
/// ⚠️ 与"恢复成第一版候选的开头"是**两件事**：
/// - 工程**本来没有**手排 EDL（基准是屏蔽补集）⇒ 恢复成第一版候选的开头即可
///   （那时"开头"本来就是我们算的，没有用户意图被覆盖）；
/// - 工程**本来有**手排 EDL ⇒ 必须用备份**整份还原**，否则用户的第一段起点
///   被永久改成候选值（实测踩到）。
///
/// ⚠️ 这是**收工的唯一入口**（生产路径与单测都走它）——
/// 把"分两条路"这件事收在一个函数里，单测才可能覆盖到**那个分支本身**
/// （★ 实测教训：只测 `restore_user_edl` 时，把调用方的 `if had_user_edl`
/// 改成 `if false`，测试**照样绿** —— 那正是我第一版修这个 bug 时的形态）。
pub fn finish_versions_restore(cfg: &mut ProjectFile, fallback_opening: f64) {
    if !cfg.user_edl_backup.is_empty() {
        // ⚠️ 有备份 ⇒ **整份还原用户的手排 EDL**（`set_opening` 会把它改坏）。
        restore_user_edl(cfg);
    } else {
        // 本来就没有手排 EDL ⇒ 恢复成第一版候选的开头（那是我们自己算的）。
        let _ = fallback_opening;
    }
}

pub fn restore_user_edl(cfg: &mut ProjectFile) {
    if cfg.user_edl_backup.is_empty() {
        return;
    }
    cfg.edl.clips = std::mem::take(&mut cfg.user_edl_backup);
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

    // ---------------------------------------------------------------------
    // **帧率：真探，不许猜**（R6）
    // ---------------------------------------------------------------------
    //
    // 这里以前写的是 `let fps = 30;` —— 一个**看着很合理**的错。EDL 的整条推导
    // （`resolve_clips` / `out_start_frame` 累加 / 音轨的每一段切点）都建立在 fps 上，
    // 拿 30 去推一个 60fps 的工程会让每段成片起点差一倍，而不报错、报告里两个数还自洽。
    //
    // ⚠️ **权威是素材自己**（与 `dhampir.rs::sequence_frames` 从转译产物读帧数同源：
    // 转译器写的时间基就是素材的）。探不到时**回落到 30** 而不是报错 ——
    // 老工程/无声素材也要能出片，而"探不到时硬失败"会把本来能出片的工程卡死。
    let mut fps = 30.0_f64;
    match Ffmpeg::probe_fps(&resolved.video) {
        Ok(real) => {
            fps = real;
            tracing::info!("  FPS: {:.3}（ffprobe 实测）", fps);
        }
        Err(e) => {
            tracing::warn!("  ⚠ 帧率探不到，回落 30fps（EDL 的推导会按 30 算）: {e:#}");
        }
    }

    // ---------------------------------------------------------------------
    // **EDL：唯一的剪辑真相**（规格 §5 / §9 R7）
    // ---------------------------------------------------------------------
    //
    // `resolve_edl` 是**唯一**的推导入口（`project/config.rs`）：
    // `edl.clips` 非空就用它，否则用 `mute_ranges` 的补集推。
    // ⚠️ **`clips` 非空而 `resolve_clips` 返回空**（`total_duration <= 0` 时会发生）
    // ⇒ 这是"有 EDL 但推不出时间线"，必须**当场报错**：
    // 悄悄走到"空 EDL ⇒ 不剪"那条路会让用户拖出来的段**全部消失**。
    //
    // ---------------------------------------------------------------------
    // **片头 / 片尾**（`[intro_outro]`）拼在 EDL 的首尾
    // ---------------------------------------------------------------------
    //
    // 与 EDL **同一份推导**：片头不过是"素材库里某段视频的一个区间"，而 `EdlClip` 本来就支持
    // `source_kind = "asset"` ⇒ 拼进去就自动继承了重放/变速/叠化/字幕重映射/总时长推导。
    // 这里只做一件 EDL 之外的事：**探素材时长**（`[intro_outro]` 存的是素材 id + 入出点秒数，
    // 而"出点 = 0 表示播到结尾"要拿真时长才能算出来）。`project` 模块不认识 ffprobe，所以注入。
    let intro_outro_clips = config.edl_clips_with_intro_outro(duration, |id| {
        // ⚠️ 走 `video_dir_readable()`（**不是** `sticker_dir().join("videos")`）：
        // 视频库 2026-10-09 从 `stickers/` 底下挪到 `resources/videos/`，且带回存量回落。
        // 拼错的话表现是"片头素材找不到/读不出时长" ⇒ 这一段被跳掉，**成片短一截且不报错**。
        let asset = crate::assets::sticker_index::find_video_asset(
            &yekitrim_core::owner::video_dir_readable(yekitrim_core::owner::Owner::from_env()),
            id,
        )?;
        let dur = get_video_duration(&asset.path.to_string_lossy());
        if dur.is_none() {
            tracing::warn!(
                "  片头/片尾素材「{id}」读不出时长（{}）—— 这一段不会加进成片",
                asset.path.display()
            );
        }
        dur
    });
    for msg in &intro_outro_clips.1 {
        // ⚠️ **必须报出来**：用户以为开了片头、成片里却没有，是最坏的那种静默失败。
        tracing::warn!("  ⚠ {msg}");
    }
    let uses_intro_outro = !config.intro_outro.is_empty();
    // ⚠️ **原始 clip 列表要一起带出来**：`ResolvedClip` 是派生值、**不带 `asset_id`**
    //    （§2.2 规则 1：没有一个字段落盘），而音轨那条腿要靠 `asset_id` 决定
    //    "这一段的声音从哪个文件读"。两个列表**同序同长**（`resolve_clips` 逐段映射）。
    let (edl_clips, edl_raw_clips, has_edl) = if uses_intro_outro && !intro_outro_clips.0.is_empty() {
        // 拼过片头片尾之后就是一条**真 EDL**：尾部那道 `cut_ranges` 必须跳过（否则剪两次）。
        let raw = intro_outro_clips.0;
        let resolved = Edl { clips: raw.clone() }.resolve_clips(fps);
        (resolved, raw, true)
    } else {
        let raw = if !config.edl.clips.is_empty() {
            config.edl.clips.clone()
        } else {
            // 与 `resolve_edl` 的兜底**同一个函数**（别在这里另推一份补集）
            crate::project::edl_clips_from_mute_complement(&config.mute_ranges, duration)
        };
        let clips = config.resolve_edl(duration, fps);
        let has = !config.edl.clips.is_empty();
        (clips, raw, has)
    };
    if has_edl && edl_clips.is_empty() {
        anyhow::bail!(
            "工程里有 {} 段 EDL，但推不出成片时间线（`total_duration <= 0`？实测 duration={duration}）。\
             这时导出会**丢掉全部剪辑**，所以直接拒绝而不是静默按原样出片。",
            config.edl.clips.len()
        );
    }
    let edl_total_frames = if edl_clips.is_empty() {
        None
    } else {
        // ⚠️ 取**解析出来的**最后一段的 `out_end_frame`（`resolve_clips` 的产物），
        // **不是** `config.edl.total_frames(fps)` —— 后者只看落盘的 `[edl.clips]`，
        // 拼进来的片头/片尾不在里面 ⇒ 这里会少算 1.5s + 3s，
        // 而画面那条腿（转译器已按拼过的列表出片）是**对的** ⇒ `mux_checked` 当场拒绝合流。
        // 实测：派生 1080 帧 vs 实得 1350 帧（2026-10-09）。
        edl_clips.last().map(|c| c.out_end_frame)
    };
    if let (true, Some(frames)) = (has_edl, edl_total_frames) {
        tracing::info!(
            "\n  -- EDL --\n  v {} 段 -> {} 段成片时间线，成片 {} 帧（{:.2}s），源 {:.2}s",
            edl_raw_clips.len(),
            edl_clips.len(),
            frames,
            frames as f64 / fps,
            duration
        );
        // ⚠️ **有 EDL 时尾部那道 `cut_ranges` 必须跳过**：唯一那一刀就是 EDL 本身。
        // 两道都执行 = "剪两次"（实测 12s 带 3s 屏蔽 → 6.0s），而且音轨按一条时间线拼、
        // 画面被剪了两次 ⇒ 从第一个屏蔽点起彻底错位。
        tracing::info!(
            "  ⚠ 有 EDL：`mute_ranges` 整个被忽略（规格 §2.3，成片播什么只由 EDL 决定），\
             尾部那道 `cut_ranges` 也**跳过** —— 唯一那一刀就是 EDL 本身"
        );
    }

    // ⚠️ `VTEDIT_PARALLELISM` 与 `frame_count` 都是**已删除的 Chrome 分块腿**
    // 留下的（它按 fps × 时长切块、4 路并行）。底座那条腿**不用它们**：帧数由
    // `dhampir::render_video_only` 从转译产物里读（唯一权威，见 `render/AGENTS.md`）。
    // 留着只为了下面那条日志（"这个工程多少秒 / 多少帧"是排查时第一眼要看的数字）。
    let parallelism = std::env::var("VTEDIT_PARALLELISM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4)
        .max(1);
    let frame_count = (duration * fps).ceil() as usize;
    tracing::info!("    {} frames at {:.3} fps", frame_count, fps);
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
    // ---------------------------------------------------------------------
    // **音效必须重映射**（R2）：`collect_sound_events` 给的是**源时间**，
    // 而下面 `mix_sfx` 按**成片时间**摆 `adelay`。
    //
    // ⚠️ 不重映射的症状是"每个音效都错位、零报错"：爆点那一下晚两秒，
    // 会被当成"音效文件选错了"。EDL 一重排就整体失效。
    // ⚠️ 空 `edl_clips` 时 `remap_sound_events` **原样返回**（等于老行为）——
    // 别在这里另写一个"空就跳过"，那会让两条路的口径分家。
    let sfx_data = if sfx_data.is_empty() {
        sfx_data
    } else {
        let remapped = render::sound::remap_sound_events(&sfx_data, &edl_clips, fps);
        if remapped.len() != sfx_data.len() {
            tracing::info!(
                "  v 音效按成片时间线重映射：{} 条 -> {} 条（重放/屏蔽段会一对多或丢弃）",
                sfx_data.len(),
                remapped.len()
            );
        }
        remapped
    };

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
        let (_, translate_report) =
            render::dhampir::render_video_only(project_path, &clip_dir, &config.style.subtitle.font, strict)?;
        // ⚠️ 总方案 §4「转译报告可见」：报告**必须播报**，否则新效果的失败形态
        // 就是"静默少画"（点了没反应，谁也不知道）。CLI 侧走 tracing，
        // 服务端那条腿在 `api/render.rs` 里把同样的行送进任务日志面板。
        for (level, msg) in render::dhampir::report_lines(&translate_report) {
            match level {
                "warn" => tracing::warn!("  {msg}"),
                "debug" => tracing::debug!("  {msg}"),
                _ => tracing::info!("  {msg}"),
            }
        }
        // 报告也要落盘一份（`<工程>/data/translate_report.json`）：任务日志会被滚动清掉，
        // 而"这条片子到底缺没缺效果"是**事后要能回答**的问题。
        if let Ok(body) = serde_json::to_vec_pretty(&translate_report) {
            let path = project_path.join("data").join("translate_report.json");
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&path, body) {
                tracing::warn!("  转译报告落盘失败（不影响出片）：{e}");
            }
        }
    }
    tracing::info!("\n  -- Audio --");
    let t_audio = std::time::Instant::now();
    let ta = project_path.join("_temp_audio.aac");
    // ---------------------------------------------------------------------
    // **原声轨：有 EDL 就按 EDL 重拼，没有就走老路**（B 件）
    // ---------------------------------------------------------------------
    //
    // 老路（`extract_audio` 整条 + 尾部 `cut_ranges`）只能**删掉**区间，
    // 表达不了"重放 / 冷开场"这类**顺序改变 + 重复**的剪辑。而画面那条腿
    // 现在按 EDL 出片 ⇒ 音轨不跟着重排就**从第一个重放点起彻底错位**，
    // 且两边都不报错。
    //
    // ⚠️ 两条路的判据**只有 `edl_clips` 这一个**（不是"有没有 mute_ranges"）：
    // 有 EDL 时 `mute_ranges` 已经作为"源时间轴上的洞"被转译器与 `resolve_edl`
    // 两边各自求过交，音轨再按它剪一次就是第三刀。
    //
    // ⚠️ **不能只看 `has_edl`**：`resolve_edl` 在**没有 EDL** 时也会用
    // `mute_ranges` 的补集推出一份 clips（那是老工程的兼容路径）。那份 clips 的
    // 语义是"掐掉屏蔽段"，与老路的 `extract + cut_ranges` **逐秒等价**，
    // 但走它要保证 `cut_ranges` 被跳过 —— 所以判据是 `has_edl`，
    // 而**非** `!edl_clips.is_empty()`。
    if has_edl {
        // ⚠️ **第四个数是逐段 `speed`**（第 4 批）：`atempo` 按它串联，
        // 时长随 `round((ef-sf)/speed)` 走 —— 与画面那条腿（转译器缩段长）
        // 和派生（`resolve_clips`）**是同一个数**。这里少传一个值，
        // 音轨就会按原速拼，而画面已经变了 ⇒ 从第一个变速段起彻底错位、零报错。
        let quads: Vec<(f64, f64, f64, f64)> = edl_clips
            .iter()
            .map(|c| (c.source_start, c.source_end, c.out_start, c.speed))
            .collect();
        let sped = quads.iter().filter(|c| (c.3 - 1.0).abs() > 1e-9).count();
        tracing::info!(
            "  ✂ 按 EDL 重拼音轨（{} 段，与画面同一条时间线；其中 {} 段变速）",
            quads.len(),
            sped
        );
        // ⚠️ **素材段的声音必须从素材自己读**（2026-10-09 踩到，片头/片尾暴露的既有缺口）：
        // 画面那条腿底座对 `source_kind = "asset"` 的段取的是**素材自己的**画面
        // （片头是素材库那段片子），而这条音轨腿原先**硬编码读主素材** ——
        // 实测 1080 帧画面配 864000 样本音轨（差 216000），`mux_checked` 当场拒绝合流。
        // 就算不拒（比如素材段恰好等长），也会是"画面放片头、声音放正片对白"的错位且**不报错**。
        //
        // 判据取段自己的 `asset_id`（`assets/AGENTS.md` 那条：出口是**项目内那份拷贝**），
        // 空/找不到 ⇒ `None` ⇒ 读主素材（老工程与正片段**逐字节不变**，R4）。
        let audio_of: Vec<Option<String>> = edl_raw_clips
            .iter()
            .map(|c| {
                let id = c.asset_id.as_deref()?.trim();
                if id.is_empty() {
                    return None;
                }
                // ⚠️ 用**段的 `asset_id` 找文件**，路径基准是项目里的 `assets/videos/`
                let p = project_path
                    .join("assets")
                    .join(crate::assets::sticker_index::VIDEO_LIBRARY_SUBDIR)
                    .join(id);
                // 允许 `asset_id` 带扩展名或不带（库里那份拷贝是真文件名）
                if p.is_file() {
                    return Some(p.to_string_lossy().to_string());
                }
                let hit = std::fs::read_dir(p.parent()?)
                    .ok()?
                    .flatten()
                    .map(|e| e.path())
                    .find(|q| {
                        q.file_stem().and_then(|s| s.to_str()) == Some(id)
                    })?;
                Some(hit.to_string_lossy().to_string())
            })
            .collect();
        let swapped = audio_of.iter().filter(|o| o.is_some()).count();
        if swapped > 0 {
            tracing::info!("  ⚠ {swapped} 段的声音来自素材文件（片头/片尾/视频素材），不是主素材");
        }
        // ⚠️ **叠化处的音频交叉淡化**（第 7 批 / task-10）：画面那条腿把上层提前
        // `DISSOLVE_OVERLAP_SEC` 做叠化，音轨必须跟同一条时间线，否则画面在淡化、
        // 声音还在硬切（割裂感明显，而两边都不报错）。
        //
        // ⚠️ **不能指望底座**（实测）：它把重叠的音频层**直接相加**，
        // `opacity` 关键帧对音频**没有任何作用** —— 相加 ≠ 交叉淡化
        // （两条人声相加会成倍变响，实测重叠区比单段高约 +6 dB）。
        // 画面归底座、**音轨归本仓**，所以淡化在这里做。
        //
        // ⚠️ **判据只读 `dissolve_frames`，不许在这里自己再推一遍**：
        // 它由 `EdlClip::dissolve_overlap_frames` 在 `resolve_clips` 里唯一算一次
        // （成立条件：有前一段、重叠 ≥1 帧、且严格小于两段各自的长 ——
        // 第一段没有前一段可叠，段太短时叠化会把那一段整段吃掉）。
        // 两份推导分歧的症状是"画面叠 0.5s、声音只淡了 0.25s"，而**不报错**。
        let mut fades: Vec<(usize, usize, f64)> = Vec::new();
        for i in 1..edl_clips.len() {
            let overlap = edl_clips[i].dissolve_frames;
            if overlap == 0 {
                continue;
            }
            fades.push((i - 1, i, (overlap as f64) / fps));
        }
        if !fades.is_empty() {
            tracing::info!(
                "  v 叠化处的音轨交叉淡化：{} 条边（各让出一半窗口，见 assemble_audio_from_edl_fades）",
                fades.len()
            );
        }
        // ⚠️ **整轨 rate 固定 1.0**：逐段速率走 `quads` 的第四个数（`atempo`）。
        // 这个参数是"整片统一变速"的口子，今天不用它。
        if let Err(e) = Ffmpeg::assemble_audio_from_edl_sources(
            &resolved.video,
            &quads,
            &fades,
            &audio_of,
            &ta.to_string_lossy(),
            1.0,
        ) {
            cleanup(project_path, &tv, &ta);
            anyhow::bail!("按 EDL 组装音轨失败: {e:#}");
        }
    } else if let Err(e) = Ffmpeg::extract_audio(&resolved.video, &ta.to_string_lossy()) {
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
                // §21 **闪避包络**：把上面那次"只在起点取样的定档"扩成**整个持续期**的逐点增益。
                //
                // ⚠️ 起点那个点必须**等于** `vol`（见 `sfx_gain_envelope` 的不变式），
                //    于是"响度曲线取不到 / 音效没有时长"时包络退化 ⇒ 与改造前**逐字节相同**。
                let env = sfx_gain_envelope(
                    sd.time,
                    sd.duration.unwrap_or(0.5),
                    file_db,
                    llm_factor(sd.volume),
                    &loudness_curve,
                    &seg_loudness,
                );
                Some(render::ffmpeg::SfxEvent {
                    time: sd.time,
                    file: f.to_string_lossy().to_string(),
                    volume: vol,
                    duration: sd.duration,
                    envelope: env,
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

    // ---- §11 背景音乐（**只做引入，不做卡点**）----
    //
    // ⚠️ 规格 `editing-techniques-catalog.md` §3c：本仓此前没有 BGM ⇒
    //    没有节拍网格 ⇒ **卡点是"不适用项"**（"不要假装能做"）。
    //    这里只把音乐**引进来**，并让它**不抢主播的话**。
    {
        let bgm = &config.bgm;
        if bgm.enabled && !bgm.is_usable() {
            // ⚠️ **开了开关但没选曲**必须报出来 —— 不能静默当"没开"：
            //    用户明明开了，报告里一行都没有，他只会以为"功能坏了"。
            tracing::warn!("  ⚠ [bgm] enabled = true 但没选曲（file 为空）—— 本次不出背景音乐");
        } else if bgm.is_usable() {
            let bgm_path = if Path::new(&bgm.file).is_absolute() {
                PathBuf::from(&bgm.file)
            } else {
                project_path.join(&bgm.file)
            };
            if !bgm_path.exists() {
                // 选了曲但文件不在：**报出来**（静默跳过的表现是"配了 BGM 但成片没有"）。
                tracing::warn!(
                    "  ⚠ [bgm] 文件不存在：{} —— 本次不出背景音乐",
                    bgm_path.display()
                );
            } else {
                // 人声段：**按字幕段量响度**，与 §21 用**同一个函数**
                // （`measure_segment_loudness`）—— 两套"哪里有人声"的判据打架时，
                // 会出现"音效闪了、BGM 没闪"（或反之），而两者看起来都"在工作"。
                //
                // ⚠️ 判据是"这一段里有没有人说话"，而不是"响度大不大"：
                //    游戏音效也很响，但它不该压 BGM（压了会得到"一有枪声音乐就变小"
                //    这种奇怪的结果）。字幕段天然就是"有人声的时间段"。
                //
                // ⚠️ 响度曲线**走缓存版**（与出片页/§21 同一份）：不缓存等于
                //    每个带 BGM 的工程都重跑一遍整段 ebur128。
                let loudness_curve =
                    measure_loudness_curve_cached(project_path, Path::new(&resolved.video));
                //
                // ⚠️ 这里**不去逐段解码**（`measure_segment_loudness` 是"每段跑一次 ffmpeg"，
                //    几十段就是几十次解码）—— 我们只需要"**有没有人声**"这个布尔量，
                //    所以把已有的响度曲线按段取**中位/平均**就够了，零额外 IO。
                let duck_segs: Vec<(f64, f64, Option<f64>)> = config
                    .segments
                    .iter()
                    .map(|sg| {
                        let pts: Vec<f64> = loudness_curve
                            .iter()
                            .filter(|(t, _)| *t >= sg.start && *t <= sg.end)
                            .map(|(_, d)| *d)
                            .collect();
                        let db = if pts.is_empty() {
                            None
                        } else {
                            Some(pts.iter().sum::<f64>() / pts.len() as f64)
                        };
                        (sg.start, sg.end, db)
                    })
                    .collect();
                let expr = render::bgm::duck_gain_expr(&duck_segs, bgm.duck_ratio);
                let total = duration;
                // ⚠️ BGM 自己那一路的输出文件**必须独立**（不能复用音效那块作用域里的 `ma`，
                //    它在这个作用域里根本不存在）—— 而且两路**上下串行**，
                //    共用一个中间文件会在"音效那步没跑"时读到一个不存在的文件。
                let mb = project_path.join("_temp_audio_b.aac");
                match Ffmpeg::mix_bgm(
                    &ta.to_string_lossy(),
                    &bgm_path.to_string_lossy(),
                    &expr,
                    bgm,
                    total,
                    bgm.loop_until_end,
                    &mb.to_string_lossy(),
                ) {
                    Ok(()) => {
                        let lim = project_path.join("_temp_audio_l.aac");
                        Ffmpeg::apply_limiter(&mb.to_string_lossy(), &lim.to_string_lossy())?;
                        move_file(&lim, &mb)?;
                        move_file(&mb, &ta)?;
                        tracing::info!(
                            "  ✓ BGM 已混入：{}（音量 {:.2}、闪避到 {:.2}、淡入 {:.1}s / 淡出 {:.1}s{}）（§11）",
                            bgm_path.display(),
                            bgm.volume,
                            bgm.duck_ratio,
                            bgm.fade_in,
                            bgm.fade_out,
                            if bgm.loop_until_end { "、循环铺满" } else { "" },
                        );
                    }
                    Err(e) => {
                        // ⚠️ **BGM 失败不该让整条出片失败**（它是一层装饰，
                        //    而画面/人声才是主体）—— 但要**报出来**，
                        //    不能"配了 BGM 却安静地没有"。
                        tracing::warn!("  ⚠ BGM 混音失败（{e}）—— 本次成片没有背景音乐");
                    }
                }
            }
        }
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
    // ⚠️ **合流前的时长断言**（R5）：`mux` 带 `-shortest`，两条轨不等长时它静默截断。
    // 有派生值时按它卡（帧相等或 ±1 帧 —— 半帧量化是 EDL 的固有代价，规格 §2.4）；
    // 没有派生值（老工程 / 时长未知）时传 `None`，等于不做断言（也**不做**"大概对一下"）。
    //
    // ⚠️⚠️ **这里的"派生值"必须是「合流那一刻两条腿的实际长度」，不是「成片最终长度」**
    // （缺陷 D1，2026-10-11 修）。两者在"尾部还要再剪一刀"的工程上**不相等**：
    //
    //   * **有 EDL**：画面那条腿按 EDL 出片（转译器 `--keep-muted` 传的也是拼过的 EDL），
    //     而 `cut_ranges` 被**跳过**（`:1038` 判据是 `has_edl`）⇒ 合流点长度 == 成片长度
    //     ⇒ `edl_total_frames` **就是**那个数，照旧。
    //   * **无 EDL（老工程 + `mute_ranges`）**：画面那条腿拿到的是**整条时间线**
    //     （`dhampir.rs::render_video_only` 硬编码 `keep_muted = true`，注释写明
    //     "屏蔽区间由 `cmd_render` 尾部那一剪按源时间去掉"），音轨那条腿走的也是
    //     **整条 `extract_audio`** —— 两条腿在合流点**都是全长**，`mute_ranges` 要等
    //     `:1047` 的 `cut_ranges` 才剪。而 `resolve_edl` 在没有 EDL 时会用
    //     `mute_ranges` 的**补集**推一份 clips（老工程兼容路径）⇒ `edl_total_frames`
    //     是**剪完之后**的长度（实测 12s 带 3s 屏蔽：540 vs 720）⇒ 断言在剪之前
    //     拿剪之后的数去卡 ⇒ **每次都误报**。
    //
    // 所以这里按"尾部那一刀要不要跑"分流。⚠️ **别改成无条件 `None`** —— 那会把
    // 有 EDL 那条路（本断言的**主战场**，重放/冷开场/变速全靠它挡"画面漏段"）一起废掉。
    let check_frames = mux_check_frames(has_edl, edl_total_frames);
    if let Err(e) = Ffmpeg::mux_checked(
        &tv.to_string_lossy(),
        &ta.to_string_lossy(),
        &op,
        check_frames,
        fps,
    ) {
        cleanup(project_path, &tv, &ta);
        anyhow::bail!("音视频合流失败: {e:#}");
    }

    tracing::info!("  ⏱ 合流 {:.1}s", t_mux.elapsed().as_secs_f64());

    // 手动屏蔽区间：导出时直接从成片中剪掉
    //
    // ⚠️⚠️ **有 EDL 时这一整段必须跳过**（"屏蔽区间只能剪一次"，规格 §5）。
    // 两道都执行 = 画面被剪两次而音轨只按 EDL 拼了一次 ⇒ 成片短一大截、
    // 从第一个屏蔽点起音画错位。实测：12s 带 3s 屏蔽，正确 9.0s，剪两次 6.0s。
    let mute_ranges: Vec<(f64, f64)> = if has_edl {
        Vec::new()
    } else {
        config
            .mute_ranges
            .iter()
            .filter_map(|r| r.normalized())
            .collect()
    };
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

        // ⚠️⚠️ **剪完之后补一道时长校验**（缺陷 D1 的另一半，别删）。
        //
        // 上面 `mux_checked` 在**无 EDL** 时已经不做帧数断言了（合流点上两条腿都是全长，
        // 拿"剪完之后"的 `edl_total_frames` 去卡必然误报）。但**不能就这么把这条腿放空** ——
        // 本断言的职责是「成片时长 == 派生时长」，而**现在才是成片真正的长度**：
        // 无 EDL 时 `edl_total_frames` 恰恰就是"屏蔽补集"的长度（`resolve_edl` 的兼容路径），
        // 也就是剪完之后**应该**有多长。所以在**这里**卡它是**正的**。
        //
        // 两处断言的关系（谁挡谁）：
        //   * 有 EDL ⇒ `mux_checked` 挡（此处 `mute_ranges` 为空，进不来）；
        //   * 无 EDL ⇒ `mux_checked` 不挡，由**这里**挡。
        // 合起来 = 无论走哪条路，成片长度都被卡住，且**都不会误报**。
        if let Some(want) = edl_total_frames {
            if fps > 0.0 {
                match Ffmpeg::probe_duration(&op) {
                    Ok(secs) => {
                        let got = (secs * fps).round() as i64;
                        let want = want as i64;
                        // 与 `mux_checked` 同口径：±1 帧（半帧量化是 EDL 的固有代价，规格 §2.4）。
                        // ⚠️ 这里**能用帧数卡**（不像音轨要用样本数）：`cut_ranges` 是**整段重编码**，
                        //    它按帧边界切，不会像 AAC 那样把容器时长撑大。
                        if (got - want).abs() > 1 {
                            cleanup(project_path, &tv, &ta);
                            anyhow::bail!(
                                "剪完屏蔽区间之后成片长度与成片时间线不一致：派生 {want} 帧（{:.3}s），\
                                 实得 {got} 帧（{secs:.3}s，fps={fps}）。差超过 1 帧 ⇒ \
                                 尾部那道 `cut_ranges` 剪多了/剪少了（`mute_ranges` 与派生补集不同源？）。",
                                want as f64 / fps
                            );
                        }
                        tracing::info!(
                            "  ✓ 剪后时长断言通过：成片 {got} 帧（{secs:.3}s）/ 派生 {want} 帧"
                        );
                    }
                    Err(e) => {
                        // 探不到就**明说**（不给假绿）：这条腿的成片长度这次没有被校验。
                        tracing::warn!("  ⚠ 剪后时长断言跳过：量成片时长失败 {e:#}");
                    }
                }
            }
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

/// §21 音效闪避：把"在音效**起点**取一次样的定档"扩成**整个持续期**的逐点增益包络。
///
/// # 参数
///
/// - `start` / `dur`：音效在成片时间轴上的起点与时长。`dur` 之后的点会被 `atrim` 切掉，
///   所以这里**不必**裁剪（但 `ffmpeg` 侧的 `envelope_filters` 会再夹一次 —— 两层都做，
///   因为这一层的 `dur` 可能是 `unwrap_or(0.5)` 的**缺省值**）。
/// - `file_db`：音效文件自身的响度（用它做基准，与调用方那次定档同源）。
/// - `rel_factor`：LLM 给的相对系数。
/// - `curve`：**视频**响度曲线 `(t, dB)`；`segs`：段级兜底 `(start, end, dB)`。
///
/// # 返回
///
/// 拐点列表，**第一个点一定是 `(0.0, 起点增益)`**。空/单点都表示"匀速"，
/// 调用方退化回那一个 `volume` 数（⇒ 与改造前逐字节相同）。
///
/// # ⚠️ 三条不变式（`SfxEnvelopePoint` 的文档里写了"为什么"）
///
/// **① 起点那个点的增益 = 原有的定档值。** 这是"没有曲线时零行为改动"的唯一保证 ——
///    `vol` 那一行算的是 `10^((video_db − 8 − file_db)/20) × rel_factor`，
///    这里必须用**同一串算式**，不能各自近似（差一点点都会让所有存量工程的音效变响/变轻）。
///
/// **② 只在曲线**真的覆盖到**的时候**加点。**覆盖不到就不加点** ——
///    拿 `-26.0` 那种兜底值去造点会让音效在持续期内**莫名其妙地起伏**。
///
/// **③ 增益上夹到 `[0.005, 1.0]`**，与调用方那次 `clamp` 同一个区间。
///    下界不是 0：`volume=0` 的段在 `amix` 里仍占一个输入，而**听感上等于音效消失**
///    —— 闪避是"让路"不是"静音"，留 0.005（≈ −46dB）比彻底听不见好。
fn sfx_gain_envelope(
    start: f64,
    dur: f64,
    file_db: f64,
    rel_factor: f64,
    curve: &[(f64, f64)],
    segs: &[(f64, f64, Option<f64>)],
) -> Vec<render::ffmpeg::SfxEnvelopePoint> {
    // 起点增益 = 调用方那次定档（不变式 ①）。两者**必须**同式。
    let gain_at = |video_db: f64| -> f64 {
        let v = 10.0_f64.powf((video_db - 8.0 - file_db) / 20.0) * rel_factor;
        v.clamp(0.005, 1.0)
    };
    // 该时刻的视频响度：曲线优先（取"最后一个 ≤ t 的点"），段级兜底。
    let video_db_at = |t: f64| -> Option<f64> {
        if let Some((_, d)) = curve.iter().rev().find(|(ct, _)| t >= *ct) {
            return Some(*d);
        }
        segs.iter()
            .find(|(s, e, _)| t >= *s && t < *e)
            .and_then(|(_, _, d)| *d)
    };

    let start_db = video_db_at(start).unwrap_or(-26.0);
    let mut out = vec![render::ffmpeg::SfxEnvelopePoint {
        t: 0.0,
        gain: gain_at(start_db),
    }];
    if !dur.is_finite() || dur <= 0.0 || curve.is_empty() {
        return out; // 没有时长 / 没有曲线 ⇒ 匀速
    }
    // 音效持续期内**曲线自己有的**采样点（不变式 ②：不造点）。
    // ⚠️ 用 `start + 1e-6` 起：起点那个点已经加过了，重复加会被 `dedup` 掉，
    //    但先把条件写清比依赖去重更稳。
    for (t, db) in curve {
        if *t <= start + 1e-6 || *t >= start + dur - 1e-6 {
            continue;
        }
        if !t.is_finite() || !db.is_finite() {
            continue;
        }
        out.push(render::ffmpeg::SfxEnvelopePoint {
            t: *t - start,
            gain: gain_at(*db),
        });
    }
    out
}

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
    let out = yekitrim_core::process::cmd("ffprobe")
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

/// 视频素材的技术信息（右栏"信息不足"补的那几行）。
///
/// ⚠️ **一次 ffprobe 拿全**，不要一个字段起一个进程：素材库可能有几十个视频，
/// 每个视频 5 次 `ffprobe` = 几百次进程创建，列表接口会明显变慢（实测每个 ~30ms）。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct VideoProbe {
    /// 秒。读不出是 `None`（不是 0.0 —— 0 是个合法的时长，两回事）。
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// 编码器名（`h264` / `hevc` / `vp9`…）。
    pub codec: Option<String>,
    /// 帧率（浮点，`29.97` 这种要保留小数）。
    pub fps: Option<f64>,
    /// 总码率（bit/s）。
    pub bitrate: Option<u64>,
    /// 有没有音轨 —— 视频素材用在片头/片尾时，没声音的会是静音，值得先说清楚。
    pub has_audio: bool,
}

/// 探一个视频文件的技术信息。**任何一项读不到就给 `None`**，绝不猜、绝不用 0 顶替。
///
/// 用 `-of json` 一次拿全：`format=duration,bit_rate` + 视频流的 `width,height,codec_name,r_frame_rate`，
/// 再顺带看一眼有没有音频流。r_frame_rate 是 `"30000/1001"` 这种分数，要自己除。
pub fn probe_video(video_path: &str) -> VideoProbe {
    let out = yekitrim_core::process::cmd("ffprobe")
        .args([
            "-v", "error",
            "-show_entries",
            "format=duration,bit_rate:stream=codec_type,codec_name,width,height,r_frame_rate",
            "-of", "json",
            video_path,
        ])
        .output();
    let Ok(out) = out else { return VideoProbe::default() };
    if !out.status.success() {
        return VideoProbe::default();
    }
    let Ok(text) = std::str::from_utf8(&out.stdout) else {
        return VideoProbe::default();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return VideoProbe::default();
    };
    let mut p = VideoProbe::default();
    if let Some(f) = v.get("format") {
        p.duration = f
            .get("duration")
            .and_then(|d| d.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|d| *d > 0.0);
        p.bitrate = f
            .get("bit_rate")
            .and_then(|d| d.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|b| *b > 0);
    }
    for st in v.get("streams").and_then(|s| s.as_array()).into_iter().flatten() {
        let kind = st.get("codec_type").and_then(|x| x.as_str()).unwrap_or("");
        if kind == "audio" {
            p.has_audio = true;
            continue;
        }
        if kind != "video" || p.width.is_some() {
            continue; // 只认第一条视频流（封面图那种附带的流不参与）
        }
        p.width = st.get("width").and_then(|x| x.as_u64()).map(|x| x as u32);
        p.height = st.get("height").and_then(|x| x.as_u64()).map(|x| x as u32);
        p.codec = st.get("codec_name").and_then(|x| x.as_str()).map(str::to_string);
        p.fps = st
            .get("r_frame_rate")
            .and_then(|x| x.as_str())
            .and_then(parse_rational);
    }
    p
}

/// `"30000/1001"` → `29.970…`。分母为 0 或格式不对 ⇒ `None`（不猜成 0）。
fn parse_rational(s: &str) -> Option<f64> {
    let (num, den) = s.split_once('/')?;
    let n: f64 = num.trim().parse().ok()?;
    let d: f64 = den.trim().parse().ok()?;
    if d == 0.0 {
        return None;
    }
    let v = n / d;
    (v > 0.0).then_some(v)
}

pub fn get_video_duration(video_path: &str) -> Option<f64> {
    let out = yekitrim_core::process::cmd("ffprobe")
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
    let output = yekitrim_core::process::cmd("ffmpeg")
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
    let output = yekitrim_core::process::cmd("ffmpeg")
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

/// 读响度曲线缓存，**但不校验 key** —— 给 `process` 的 §6 爆点判据用。
///
/// ⚠️ 为什么不是把 `media_key` 变 `pub`：那样调用方要自己拼 key 格式，
/// 而 key 格式一变（这里已经改过一次口径）就会有地方悄悄读不出曲线 ——
/// 症状是"爆点判据的音效那一路静默失效"。把"读 + 校验"合成一个函数，
/// 调用方不可能只做一半。
///
/// ⚠️ 校验用的 key 与写缓存时**同一个** `media_key`（本模块私有）——
/// 这正是不能把它拆出去的原因。
pub fn read_loudness_curve_checked(cache_dir: &Path, media_path: &Path) -> Vec<(f64, f64)> {
    let cache_file = cache_dir.join("data").join("loudness.json");
    let Some(key) = media_key(media_path) else { return Vec::new() };
    let Ok(raw) = std::fs::read_to_string(&cache_file) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else { return Vec::new() };
    // ⚠️ **必须验 key**：换过源视频之后缓存还在，不验就会拿上一条视频的响度
    // 去判这一条的爆点 —— 全错但**不报错**。
    if v.get("key").and_then(|x| x.as_str()) != Some(key.as_str()) {
        return Vec::new();
    }
    v.get("points")
        .and_then(|p| p.as_array())
        .map(|points| {
            points
                .iter()
                .filter_map(|p| Some((p.get("t")?.as_f64()?, p.get("db")?.as_f64()?)))
                .collect()
        })
        .unwrap_or_default()
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
    let output = yekitrim_core::process::cmd("ffmpeg")
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
        canvas_supported_effects, effect_spec_json, missing_support, set_opening, sfx_gain_envelope,
        unique_output_path, used_effect_types,
    };
    use std::path::{Path, PathBuf};

    // ── §9 多版本成片：换开头 ────────────────────────────────────────

    /// **时长读不到时，换开头是"写不进去"的** —— 这条钉的就是那个静默假成功。
    ///
    /// 实测（2026-10-12）：工程缺 `data/clip-meta.json` ⇒ `duration_for_versions` 回落 **0.0**
    /// ⇒ `edl_clips_from_mute_complement(ranges, 0.0)` 对非正时长返回**空 Vec**
    /// ⇒ `set_opening` 在 `base.is_empty()` 处**直接 return、一个字节都不写**。
    /// 后果：`--versions 3` 跑三遍、日志三行"开头选在 X"、**三个产物逐字节相同**
    /// （MD5 全等 `a6f7f120…`），而收工那句还宣称"已恢复到第一版的开头"。**报告与事实相反。**
    ///
    /// ⇒ 修法有两层，这条测试钉**第一层**（`set_opening` 的语义本身）：
    ///    **`total_duration <= 0` 时它什么都不写**，所以调用方必须先判时长。
    ///    第二层（调用方 `cmd_render_versions` 的两道自检）由真跑验证，见计划书 §9。
    ///
    /// ⚠️ **变异自证的结论要说准**：我试着把**源头**那道守卫
    /// （`edl_clips_from_mute_complement` 的 `total_duration > 0.0`）放宽掉，
    /// 这条测试**照样绿** —— 因为 `set_opening` 后面还有一道独立的
    /// `at >= base[0].source_end - 1e-9`（时长 0 时 `source_end` 也是 0，任何正 `at` 都过不去）。
    /// ⇒ **两条守卫各自都足以拦住这个 bug**（这是好事：纵深），
    ///   但这意味着**单看本测试红了/绿了，分辨不出是哪一条在起作用**。
    ///   要单独钉住源头那一条，得直接测 `edl_clips_from_mute_complement(_, 0.0).is_empty()`。
    ///   （教训：**"变异没翻面"不等于"测试没用"** —— 它可能说明有多条路径，要去看清是哪一条。）
    ///
    /// 下面这条是"补集在边界时长上的行为"判据。
    ///
    /// ⚠️⚠️ **它的变异自证也失败了，而原因值得记一笔**：我把源头那道
    /// `!(total_duration > 0.0)` 守卫放宽，**这条照样绿** —— 因为在 `muted` 为空时，
    /// 函数末尾那句 `if cursor < total_duration`（`0 < 0` = false）**同样**不产出任何 clip。
    /// ⇒ 也就是说，对"空屏蔽 + 0 时长"这个输入，**有两处独立逻辑各自都返回空**，
    /// 用这个输入**分辨不出**是哪一处挡住了。
    /// ⇒ **本测试因此只钉"结果"（空就是空），钉不住具体是哪道守卫** —— 这一点如实写明，
    ///   而不是假装它做了变异自证。真正防住 §9 那个静默假成功的是**调用方那两道自检**
    ///   （`cmd_render_versions` 里的时长预检 + 写后回读），它们由真跑验证：
    ///   缺 `clip-meta.json` 时 `--versions 2` **当场报错**、补上之后三版 MD5 各不相同。
    #[test]
    fn mute_complement_yields_nothing_for_nonpositive_duration() {
        // 时长 0 / NaN ⇒ 必须是空（两条路都指向这个结果）
        assert!(
            crate::project::edl_clips_from_mute_complement(&[], 0.0).is_empty(),
            "时长 0 时补集必须为空 —— §9 静默假成功的源头"
        );
        assert!(
            crate::project::edl_clips_from_mute_complement(&[], f64::NAN).is_empty(),
            "非有限时长同样必须为空"
        );
        // 正时长、无屏蔽 ⇒ 一整段（证明"空"不是这个函数的常态）
        let ok = crate::project::edl_clips_from_mute_complement(&[], 40.0);
        assert_eq!(ok.len(), 1, "正时长无屏蔽应当给出一整段");
        assert!((ok[0].source_end - 40.0).abs() < 1e-9);
    }

    #[test]
    fn set_opening_writes_nothing_when_duration_is_zero() {
        let mut cfg = crate::project::ProjectFile::example();
        cfg.mute_ranges.clear();
        cfg.edl.clips.clear();
        // 时长 0（= clip-meta.json 缺失时的回落值）
        set_opening(&mut cfg, 2.4, 0.0);
        assert!(
            cfg.edl.clips.is_empty(),
            "时长 0 时 `set_opening` 必须**什么都不写** —— \
             这正是调用方必须先判时长的原因（否则日志说换了开头、产物却一模一样）"
        );
        // 正时长下它才真的写
        let mut ok = crate::project::ProjectFile::example();
        ok.mute_ranges.clear();
        ok.edl.clips.clear();
        set_opening(&mut ok, 2.4, 40.0);
        assert_eq!(ok.edl.clips.len(), 1, "正时长下应当写出一个 clip");
        assert!(
            (ok.edl.clips[0].source_start - 2.4).abs() < 1e-9,
            "换开头的落点必须精确等于候选时刻，实际 {}",
            ok.edl.clips[0].source_start
        );
    }

    fn project_with_video_span(total: f64) -> crate::project::ProjectFile {
        let mut cfg = crate::project::ProjectFile::example();
        cfg.mute_ranges.clear();
        cfg.edl.clips.clear();
        let _ = total;
        cfg
    }

    /// ⚠️ **换开头必须落在 EDL 上**，不能落在 `segments[0]` 上。
    ///
    /// 这条钉的是一个**实测踩到、而且当时完全看不出来**的坑：
    /// 第一版改的是 `cfg.segments[0].start/end`，三版跑完日志一行不差、
    /// **三个产物逐字节相同** —— 因为成片时间线来自 `resolve_edl`
    /// （输入是 `edl.clips` 或 `mute_ranges` 的补集），`segments[]` 不在那条链上。
    ///
    /// 判据：改完之后 `resolve_edl` 的**第一个 clip 起点**必须等于新开头。
    /// 只断言"`cfg.edl.clips` 非空"是不够的（写进去一个没人读的字段也会非空）。
    #[test]
    fn 换开头要真的改到成片时间线() {
        let mut cfg = project_with_video_span(8.0);
        set_opening(&mut cfg, 2.2, 8.0);

        let clips = cfg.resolve_edl(8.0, 30.0);
        assert!(!clips.is_empty(), "改完之后 EDL 是空的 —— 成片会没有画面");
        let first_start = clips[0].source_start;
        assert!(
            (first_start - 2.2).abs() < 1e-6,
            "EDL 第一段起点是 {first_start}，不是要求的 2.2 —— \
             换开头没有落到成片时间线上（这正是那条'三版逐字节相同'的坑）"
        );
    }

    /// ⚠️ **收工时的"恢复"必须能往回退**。
    ///
    /// 踩过的坑：逐版是**递增**的（0.3 → 2.2 → 4.1），而收工时要退回到 0.3。
    /// 第一版里有一条"`at` 必须比当前起点靠后"的判据，把恢复调用拒绝了 ⇒
    /// 工程停在**最后一版**的开头，而日志照旧打"已恢复到第一版"。
    /// 用户下次直接点出片，拿到的是第 3 版，而工程文件上看不出这件事。
    #[test]
    fn 恢复开头能往回退() {
        let mut cfg = project_with_video_span(8.0);
        // 模拟逐版推进到最后一版。
        set_opening(&mut cfg, 4.1, 8.0);
        assert!((cfg.resolve_edl(8.0, 30.0)[0].source_start - 4.1).abs() < 1e-6);
        // 收工：退回第一版。
        set_opening(&mut cfg, 0.3, 8.0);
        let back = cfg.resolve_edl(8.0, 30.0)[0].source_start;
        assert!(
            (back - 0.3).abs() < 1e-6,
            "恢复后起点是 {back}，不是 0.3 —— 恢复调用被'只能往后'的判据挡掉了"
        );
    }

    /// ⚠️⚠️ **不许弄丢用户手排的剪辑点**（实测踩到的真数据丢失）。
    ///
    /// 现场：用户手排 `[1.0,4.0]` + `[5.0,8.0]`，跑一次 `--versions 3` 之后
    /// 第一段起点变成 **0.3**（候选值）—— 而日志打的是"已恢复到第一版的开头"，
    /// ★ **报告与事实相反**（"第一版的开头"是候选的 0.3，不是用户的 1.0）。
    /// 原值一旦丢掉就找不回来。
    ///
    /// 判据：**跑完整套"改开头 → 还原"之后，`edl.clips` 必须逐字段等于原值**。
    #[test]
    fn 手排_edl_跑完要整份还原() {
        let mut cfg = crate::project::ProjectFile::example();
        cfg.mute_ranges.clear();
        // ⚠️ `EdlClip` 没有 `Default` —— 用 serde 反序列化造（也顺手验了字段名）。
        let mk = |a: f64, b: f64| -> yekitrim_render_core::types::EdlClip {
            serde_json::from_value(serde_json::json!({ "source_start": a, "source_end": b }))
                .expect("EdlClip 反序列化")
        };
        let user = vec![mk(1.0, 4.0), mk(5.0, 8.0)];
        cfg.edl.clips = user.clone();

        let openings_first_at = 1.5_f64;
        // 逐版挪开头（这是 `cmd_render_versions` 干的事）。
        // ⚠️ 候选必须落在**用户第一段内部**（`[1.0,4.0)`）——
        //    越过 4.0 会被 `set_opening` 正确拒绝（那正是它的守卫）。
        for at in [1.5_f64, 2.5, 3.5] {
            set_opening(&mut cfg, at, 8.0);
        }
        assert!(
            (cfg.edl.clips[0].source_start - 3.5).abs() < 1e-6,
            "逐版推进没生效"
        );
        // 收工：还原。
        //
        // ⚠️ **必须走"收工"那条判据本身**（`finish_versions_restore`），
        //    不能直接调 `restore_user_edl` —— 后者只测了"还原函数对不对"，
        //    而**真正的现场是调用方那个分支**（★ 实测过：直接调 `restore_user_edl`
        //    的写法，把调用方改成 `if false` 之后测试**照样绿**）。
        super::finish_versions_restore(&mut cfg, openings_first_at);
        assert_eq!(
            cfg.edl.clips, user,
            "用户手排的 EDL 没被整份还原 —— 他的剪辑点被候选值覆盖了（数据丢失）"
        );
    }

    /// 没有手排 EDL 的工程：备份**必须是空的**（否则会写进 `polish.toml`，污染老工程）。
    #[test]
    fn 没有手排_edl_时不写备份() {
        let mut cfg = crate::project::ProjectFile::example();
        cfg.edl.clips.clear();
        cfg.mute_ranges.clear();
        let _ = &cfg.edl.clips;
        set_opening(&mut cfg, 1.0, 8.0);
        assert!(
            cfg.user_edl_backup.is_empty(),
            "屏蔽补集推出来的基准不该被当成'用户手排'备份下来"
        );
    }

    /// 越界/无意义的 `at` 一律**什么都不做**（保守：宁可这版与上版相同，
    /// 也不要把开头挪到一个不存在的位置）。
    #[test]
    fn 越界的开头不动工程() {
        let mut cfg = project_with_video_span(8.0);
        set_opening(&mut cfg, -1.0, 8.0);
        assert!(cfg.edl.clips.iter().all(|c| c.source_start >= 0.0), "负起点被写进去了");
        // 越过视频末尾。
        let mut cfg2 = project_with_video_span(8.0);
        set_opening(&mut cfg2, 99.0, 8.0);
        // 没有 EDL 且没有屏蔽时基准 clips 是空的 ⇒ 直接返回（不改）。
        assert!(cfg2.edl.clips.is_empty(), "越界的开头不该写出 EDL");
    }

    /// 时长未知（0）时不许造出 EDL —— 否则成片会变成"零帧"。
    #[test]
    fn 时长未知时不造_edl() {
        let mut cfg = project_with_video_span(0.0);
        set_opening(&mut cfg, 1.0, 0.0);
        assert!(cfg.edl.clips.is_empty(), "时长未知却写出了 EDL");
    }

    // ── §21 音效闪避包络（音频域）────────────────────────────────────

    /// ① ⚠️ **不变式 ①：起点那个点的增益必须等于调用方那次定档**。
    ///
    /// 这条是"没有曲线时零回归"的**唯一**保证 —— 差一点点（比如忘了乘 `rel_factor`、
    /// 或漏掉 `-8.0`）都会让**所有存量工程**的音效一起变响/变轻，而它不会报错。
    /// 所以判据不比对"近似值"，而是**照着调用方同一串算式**重算一遍再逐位相等。
    #[test]
    fn envelope_first_point_is_the_original_fixed_gain() {
        let curve = vec![(0.0, -20.0)];
        let env = sfx_gain_envelope(5.0, 1.0, -14.0, 1.0, &curve, &[]);
        assert_eq!(env.len(), 1, "曲线只盖到起点 ⇒ 只有起点那一个点: {env:?}");
        assert_eq!(env[0].t, 0.0);
        // 调用方的算式：10^((video_db − 8 − file_db)/20) × rel_factor，再夹 [0.005, 1.0]
        let want = 10.0_f64.powf((-20.0 - 8.0 - (-14.0)) / 20.0);
        assert!(
            (env[0].gain - want).abs() < 1e-12,
            "起点增益必须与定档同式：实得 {} 期望 {want}",
            env[0].gain
        );
    }

    /// ② 持续期内**有**曲线采样点 ⇒ 真的多出点，且 `t` 是**相对音效起点**的。
    ///
    /// ⚠️ 相对时间这件事最容易错：把绝对时间写进去，`enable='between(t,…)'` 就会指到
    /// 音效窗口**之外**，表现是"包络一点没生效"（而滤镜串看着完全正常）。
    #[test]
    fn envelope_samples_are_relative_to_the_sfx_start() {
        // 音效 5.0s 起、长 2.0s；曲线在 5.5 / 6.0 各有一个点（都在窗口内）
        let curve = vec![(0.0, -20.0), (5.5, -8.0), (6.0, -26.0)];
        let env = sfx_gain_envelope(5.0, 2.0, -14.0, 1.0, &curve, &[]);
        assert_eq!(env.len(), 3, "起点 + 两个窗口内的采样点: {env:?}");
        assert_eq!(env[0].t, 0.0);
        assert!((env[1].t - 0.5).abs() < 1e-9, "5.5 − 5.0 = 0.5，实得 {}", env[1].t);
        assert!((env[2].t - 1.0).abs() < 1e-9, "6.0 − 5.0 = 1.0，实得 {}", env[2].t);
        // 5.5s 的视频响度 −8 ⇒ 比 5.0s 的 −20 更响 ⇒ 增益应当**更大**（音效跟着抬）
        assert!(
            env[1].gain > env[0].gain,
            "人声更响的区间音效增益应当更高：{:?}",
            env.iter().map(|p| p.gain).collect::<Vec<_>>()
        );
    }

    /// ③ ⚠️ **窗口外**的曲线点一个都不许进来（不变式 ②）。
    ///
    /// 进来之后：`atrim` 已经把素材切到 `dur` 了，所以那些段**永远不会被播放** ——
    /// 不是错，是**没用的滤镜**（滤镜串变长、每段多一次 `enable` 求值）。
    /// 但更糟的是"没有窗口概念"时源点可能落在**负数** `t` 上，那会让第一段 `enable`
    /// 的范围反着写。
    #[test]
    fn envelope_ignores_curve_points_outside_the_window() {
        let curve = vec![(0.0, -20.0), (4.0, -5.0), (5.5, -8.0), (7.5, -5.0), (9.0, -5.0)];
        let env = sfx_gain_envelope(5.0, 2.0, -14.0, 1.0, &curve, &[]);
        assert_eq!(env.len(), 2, "只有 5.5 落在 [5.0, 7.0) 内: {env:?}");
        assert!(env.iter().all(|p| p.t >= 0.0), "不许出现负的 t: {env:?}");
        assert!(env.iter().all(|p| p.t < 2.0), "不许超出时长: {env:?}");
    }

    /// ④ 段级兜底也能当信号源（曲线为空时）。
    #[test]
    fn envelope_falls_back_to_segment_loudness() {
        let segs = vec![(0.0, 10.0, Some(-20.0))];
        let env = sfx_gain_envelope(3.0, 1.0, -14.0, 1.0, &[], &segs);
        assert_eq!(env.len(), 1);
        let want = 10.0_f64.powf((-20.0 - 8.0 - (-14.0)) / 20.0);
        assert!((env[0].gain - want).abs() < 1e-12, "段级兜底也要同式：{env:?}");
    }

    /// ⑤ ⚠️ 时长非法 / 无信号 ⇒ **只有起点一个点**（= 匀速，逐字节零回归）。
    #[test]
    fn degenerate_inputs_collapse_to_a_single_point() {
        let curve = vec![(0.0, -20.0), (5.5, -8.0)];
        for (dur, label) in [(0.0, "零时长"), (-1.0, "负时长"), (f64::NAN, "NaN 时长")] {
            let env = sfx_gain_envelope(5.0, dur, -14.0, 1.0, &curve, &[]);
            assert_eq!(env.len(), 1, "{label} 必须退化成匀速: {env:?}");
        }
        // 完全没有信号（没有曲线、也没有段级）⇒ 用 −26.0 兜底，但仍只有起点
        let env = sfx_gain_envelope(5.0, 2.0, -14.0, 1.0, &[], &[]);
        assert_eq!(env.len(), 1);
        assert_eq!(env[0].t, 0.0);
    }

    /// ⑥ ⚠️ **增益必须夹在 `[0.005, 1.0]`** —— 与调用方那次 `clamp` 同区间。
    ///
    /// ⚠️ 下界**不是 0**：`volume=0` 听感上等于"音效消失"，而闪避是**让路**不是静音。
    /// 这条判据防的是"有人觉得 0 更干净" —— 那种改法会让爆点音效**整段消失**。
    #[test]
    fn envelope_gain_stays_inside_the_clamp() {
        // 视频极响（+20dB）⇒ 未夹取时增益会远超 1.0
        let env = sfx_gain_envelope(0.0, 1.0, -30.0, 4.0, &[(0.0, 20.0)], &[]);
        assert!(env[0].gain <= 1.0, "上限必须夹住: {}", env[0].gain);
        // 视频极轻（−80dB）⇒ 未夹取时会接近 0
        let env = sfx_gain_envelope(0.0, 1.0, 0.0, 1.0, &[(0.0, -80.0)], &[]);
        assert!(env[0].gain >= 0.005, "下限必须是 0.005 而不是 0: {}", env[0].gain);
    }

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

    // ── 缺陷 D1：合流前那道帧数断言该拿什么数卡 ──────────────────────

    /// **无 EDL 时不许在合流点卡**（那时 `edl_total_frames` 是"剪完之后"的数，
    /// 而 `cut_ranges` 还没跑 —— 卡它就是那次"派生 540 / 实得 720"的误报）。
    ///
    /// 变异自证（改坏一定会红）：
    ///   * 改回无条件 `edl_total_frames` ⇒ 这条红（Some(540) != None）；
    ///   * 改成无条件 `None` ⇒ `有_edl_时必须在合流点卡` 那条红（主战场被放空）。
    #[test]
    fn 无_edl_时合流点不卡帧数() {
        let got = super::mux_check_frames(false, Some(540));
        assert_eq!(
            got, None,
            "无 EDL 时合流点上两条腿都是全长（画面 --keep-muted、音轨整条 extract），\
             而 {0:?} 是 mute 补集=剪完之后的数。在这里卡它 ⇒ 「派生 540 帧 / 实得 720 帧」必然误报。",
            Some(540u64)
        );
        // 连派生值都没有时同样不卡（老工程 / 时长未知）。
        assert_eq!(super::mux_check_frames(false, None), None);
    }

    /// **有 EDL 时必须卡**（这条是主战场：重放/冷开场/变速靠它挡"画面漏段"）。
    ///
    /// ⚠️ 这条防的是"为了修 D1 顺手把它一起放松"——那正是任务书点名的"不要放宽断言了事"。
    #[test]
    fn 有_edl_时必须在合流点卡() {
        assert_eq!(
            super::mux_check_frames(true, Some(1080)),
            Some(1080),
            "有 EDL 时 `cut_ranges` 被跳过 ⇒ 合流点长度就是成片长度 ⇒ 必须拿派生值卡。\
             放空 = 重放/冷开场/变速那条路的'画面漏段'没人挡了。"
        );
        // 有 EDL 但推不出帧数（理论上的空 resolve）⇒ 没有可卡的值，只能不卡。
        assert_eq!(super::mux_check_frames(true, None), None);
    }
}
