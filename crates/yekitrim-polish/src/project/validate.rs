//! polish::project::validate：`polish.toml` 的加载 / 原子写 / 校验，以及读-改-写的唯一通道。
//!
//! 入口：`load_project`、`load_config_from`、`update_polish_toml`、`write_atomic`、`validate_project`。
//! 边界：长任务落盘只能走 `update_polish_toml`（锁内重新 load + 只覆盖自己负责的字段），不许"开头 load、结尾整份写回" —— 见 `crates/yekitrim-polish/src/project/AGENTS.md`

use std::path::Path;
use super::ProjectFile;

/// 成片时间线（EDL）**单级变速的合法区间**。
///
/// ⚠️ **这两个数字不是随便定的**：它们就是 ffmpeg `atempo` 的**单级范围**
/// （`docs/plan/edl-retiming-spec.md` §3.2 / §4.1 第 8 条）。音轨变速**必须**用 `atempo`
/// （用 `asetpts` 缩放会改采样率/变调 = 花栗鼠音，§9 R6），而单级 `atempo` 只吃
/// `[0.5, 2.0]` —— 想要 `speed=0.25` 得写成 `atempo=0.5,atempo=0.5`（串联多级）。
///
/// ⇒ 常量表与 `atempo` 单级范围**必须一致**：把这里放宽到 `atempo` 之外，
/// 第 4 批的真出片就会在音频滤镜上失败（或者更糟：静默取不到效果）。
/// 反过来收紧就是白丢用户能用的档位。
pub const SPEED_MIN: f64 = 0.5;
/// 见 [`SPEED_MIN`]。
pub const SPEED_MAX: f64 = 2.0;

/// 从项目目录加载 polish.toml
pub fn load_project(project_path: &Path) -> anyhow::Result<ProjectFile> {
    load_config_from(project_path, "polish.toml")
}

/// **读-改-写 polish.toml**：`cmd_llm` / `cmd_process` / `cmd_cover` 这些长任务落盘都走这里。
///
/// 语义是"只覆盖我负责的字段"——闭包拿到的基线是**磁盘此刻的最新内容**，所以 LLM 剪辑
/// 跑了两分钟期间用户存的选框 / 封面 / 屏蔽轨道不会被结尾那次整份覆写吞掉（以前是开头
/// `load_project()` 读一次、结束把内存里那份写回去，中间的改动全没了）。
///
/// 锁是 `yekitrim_core::filelock` 的路径锁（与服务端 handler 同一把，同进程内串行）；
/// 闭包只做内存改动，写文件在本函数内完成。
pub async fn update_polish_toml<F>(project_path: &Path, edit: F) -> anyhow::Result<()>
where
    F: FnOnce(&mut ProjectFile),
{
    let config_path = project_path.join("polish.toml");
    let write_path = config_path.clone();
    let dir = project_path.to_path_buf();
    yekitrim_core::filelock::with_path_lock(&config_path, move || -> anyhow::Result<()> {
        let mut cfg = load_project(&dir)?;
        edit(&mut cfg);
        let raw = cfg.to_toml_commented()?;
        write_atomic(&write_path, &raw)?;
        Ok(())
    })
    .await
}

/// 原子写配置文件：先写同目录临时文件再 rename。
///
/// 渲染/后处理是被父进程按 pid **杀**的（取消）——直接 `fs::write` 有可能留下半个
/// polish.toml，下次加载直接报 TOML 解析错误，整个片段打不开。
pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, contents)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// 从项目目录加载指定名称的配置文件
pub fn load_config_from(project_path: &Path, filename: &str) -> anyhow::Result<ProjectFile> {
    let config_path = project_path.join(filename);
    if !config_path.exists() {
        anyhow::bail!(
            "{} not found at {}. Run 'yekitrim polish init' first.",
            filename,
            config_path.display()
        );
    }
    let raw = std::fs::read_to_string(&config_path)?;
    // 配置**版本**：老工程缺 `config_version` ⇒ 在这里补上当前版本（只在内存里，磁盘不动）。
    // ⚠️ 版本键随后要从值里**摘掉**再进 `ProjectFile` —— 它是 `deny_unknown_fields`，
    //    多一个键就直接反序列化失败。版本是"文件的元数据"，不属于配置 schema（写侧同理，
    //    见 `ProjectFile::to_toml_commented`）。
    let migrated = yekitrim_core::config::version::migrate_text(
        yekitrim_core::config::ConfigKind::Polish,
        &raw,
    )
    .unwrap_or_else(|_| yekitrim_core::config::version::Prepared {
        text: raw.clone(),
        from: 0,
        changed: false,
        // ⚠️ 兜底那一支**没有迁移**，所以没有"清掉的键"可报（这条读路径本来也不打日志：
        //    工程级配置不写回，键会一直在，报一次就够且由 `config migrate` 负责真清）。
        retired: Vec::new(),
    });
    // 老工程的 `duration` → `end` 在反序列化前升级（见 project/legacy.rs）
    let text = yekitrim_core::config::version::strip_version_key(&migrated.text)?;
    let toml_value: toml::Value = toml::from_str(&text)?;
    let json = super::legacy::upgrade_toml_value(&toml_value)?;
    let mut config: ProjectFile = serde_json::from_value(json)
        .map_err(|e| retired_effect_hint(e, &config_path))?;
    strip_blocked_sticker_sources(&mut config.sticker_source);
    fill_default_local_packages(&mut config.sticker_source);
    // 老键 `[stage] portrait_*` → `[stage.lanes]` 的单向吸收（**只在内存里**；下一次保存才落盘）。
    // ⚠️ 不做的话：竖画布下用户显式开过的运镜/特效/音效会**静默回到默认关**。
    config.migrate_portrait_lanes();
    Ok(config)
}

/// **已下线特效类型**的人话报错（2026-10-01，为 `docs/plan/fx-type-consolidation.md` 的 D1/D2 铺路）。
///
/// 类型收敛（删 `split` / `stutter` / `zoom_bounce` / `color_shift`）之后，老工程会以
/// serde 的 `unknown variant \`stutter\`, expected one of …` 读不出来 —— **那句话对人没用**：
/// 它既不说"这是本版本删掉的效果"，也不说怎么办。
///
/// 这里把它包成人话（并保留原始 serde 文本供排查）。⚠️ 这份名单是**历史清单**，
/// **不要**拿效果词汇表（`spec/effects.json`）去校验它 —— 那几个类型正是要在词汇表里消失的。
fn retired_effect_hint(err: serde_json::Error, config_path: &Path) -> anyhow::Error {
    match retired_effect_message(&err.to_string(), config_path) {
        Some(msg) => anyhow::anyhow!(msg),
        None => anyhow::Error::new(err),
    }
}

/// 纯函数：serde 的报错文本 → 人话（认不出那四类就 `None` = 原样透传）。
///
/// 抽成纯函数是为了**可测**：那四个类型在收敛落地前仍然合法，构造不出真实的 serde 报错，
/// 所以测试用**合成文本**（`unknown variant \`stutter\`, expected one of …`）来钉这段逻辑。
fn retired_effect_message(raw: &str, config_path: &Path) -> Option<String> {
    const RETIRED: [&str; 4] = ["split", "stutter", "zoom_bounce", "color_shift"];
    // 只在 serde 明确说"unknown variant `X`"时才算 —— 别拿类型名裸匹配（`expected one of …` 会把
    // 当前仍合法的名字都列出来，裸匹配必然误判）。
    let hit: Vec<&str> = RETIRED
        .iter()
        .copied()
        .filter(|t| raw.contains(&format!("unknown variant `{t}`")))
        .collect();
    if hit.is_empty() {
        return None;
    }
    Some(format!(
        "该工程含**已下线的特效事件**（{}）—— 本版本不兼容这类工程。\n  \
         两条路：① 用旧版本打开、把这几条事件删掉；\
         ② 直接编辑 {}：删掉对应 `[[segments.events]]` 里 `type = \"{}\"` 的那一段。\n  \
         原始报错：{}",
        hit.join(" / "),
        config_path.display(),
        hit[0],
        raw
    ))
}

/// 默认自动载入已 build 的贴纸包（从 stickers/sticker.toml 索引读取）。
/// 仅当配置里没有任何本地贴纸包时才追加，避免重复。
fn fill_default_local_packages(sources: &mut Vec<String>) {
    use crate::assets::sticker_index::{load_sticker_index, is_package_source};
    let root = yekitrim_core::owner::sticker_dir_readable(yekitrim_core::owner::Owner::from_env());
    // 判断是否已有本地包（非 emoji/irasu 且目录存在）
    let has_package = sources.iter().any(|s| is_package_source(&root, s));
    if has_package {
        return;
    }
    let index = load_sticker_index(&root);
    for pkg in &index.packages {
        if !sources.iter().any(|s| s == pkg) {
            sources.push(pkg.clone());
        }
    }
}

/// irasu 搜索/匹配能力有问题，读取配置时统一屏蔽（默认配置已不含，旧工程显式配置也在此剔除）
fn strip_blocked_sticker_sources(sources: &mut Vec<String>) {
    if sources.iter().any(|s| s == "irasu") {
        sources.retain(|s| s != "irasu");
        tracing::warn!(
            "⚠ irasu 贴纸来源已屏蔽（搜索/匹配能力有问题），有效来源: {}",
            if sources.is_empty() {
                "无（不生成贴纸）".to_string()
            } else {
                sources.join(" / ")
            }
        );
    }
}

/// 校验项目文件中的路径和字段合法性
pub fn validate_project(config: &ProjectFile, project_path: &Path) -> Vec<String> {
    let mut warnings = Vec::new();

    let resolve = |relative: &str| -> String {
        let p = Path::new(relative);
        if p.is_relative() {
            project_path.join(p).to_string_lossy().to_string()
        } else {
            p.to_string_lossy().to_string()
        }
    };

    let video = resolve(&config.source.video);
    if !std::path::Path::new(&video).exists() {
        warnings.push(format!("Video not found: {}", video));
    }

    let srt = resolve(&config.source.subtitles);
    if !std::path::Path::new(&srt).exists() {
        warnings.push(format!("Subtitles not found: {}", srt));
    }

    if !config.source.danmaku.is_empty() {
        let dm = resolve(&config.source.danmaku);
        if !std::path::Path::new(&dm).exists() {
            warnings.push(format!("Danmaku not found: {}", dm));
        }
    }

    if !config.source.metadata.is_empty() {
        let meta = resolve(&config.source.metadata);
        if !std::path::Path::new(&meta).exists() {
            warnings.push(format!("Metadata not found: {}", meta));
        }
    }

    if config.segments.is_empty() {
        warnings.push("No segments defined — video will use default camera".into());
    }

    for (i, seg) in config.segments.iter().enumerate() {
        if seg.end <= seg.start {
            warnings.push(format!("Segment {i}: end ({}) <= start ({})", seg.end, seg.start));
        }
    }

    warnings.extend(validate_edl(config));

    warnings
}

/// 成片时间线（EDL）的校验（规格 §4.1 第 7 条）。
///
/// 判据四条：
/// 1. 区间**非负**（`source_start >= 0`）—— 负的源起点会被 `edl_frame_at` 夹成 0，
///    表现是"这段和上一段重合了一截"，**不报错**；
/// 2. `source_end > source_start` —— 零长/倒置的段在推导里会被**静默跳过**，
///    用户看到的是"我加了一段它没出现"；
/// 3. `speed ∈ [SPEED_MIN, SPEED_MAX]`（与 `atempo` 单级范围一致，见 [`SPEED_MIN`]）；
/// 4. **空 `clips` 且 `mute_ranges` 非空** ⇒ 报 NOTE：此时成片走的是补集派生（§2.2 规则 3），
///    用户以为"成片轨是空的 = 什么都不剪"的误解在这里说清。
///
/// ⚠️ **不重叠是允许的**（重放/冷开场本来就是同一段出现两次，§2.1）—— 所以这里**没有**
/// 重叠判据。乱序同样允许（数组顺序 = 成片播放顺序）。
///
/// ⚠️ **`[edl.clips]]` 里的路径字段 / `mute_ranges` 的"相加"语义都不在这里** ——
/// `mute_ranges` 由消费侧生效，见 `project/AGENTS.md`。
fn validate_edl(config: &ProjectFile) -> Vec<String> {
    let mut warnings = Vec::new();

    for (i, clip) in config.edl.clips.iter().enumerate() {
        if clip.source_start < 0.0 {
            warnings.push(format!(
                "EDL clip {i}: source_start ({}) < 0 —— 起点会被夹到 0，与上一段重合",
                clip.source_start
            ));
        }
        if clip.source_end <= clip.source_start {
            warnings.push(format!(
                "EDL clip {i}: source_end ({}) <= source_start ({}) —— 这一段会被整段跳过",
                clip.source_end, clip.source_start
            ));
        }
        if !clip.speed.is_finite() || clip.speed < SPEED_MIN || clip.speed > SPEED_MAX {
            warnings.push(format!(
                "EDL clip {i}: speed ({}) 超出 [{SPEED_MIN}, {SPEED_MAX}]（= ffmpeg atempo 单级范围）",
                clip.speed
            ));
        }
    }

    // NOTE：成片轨是空的、但屏蔽轨道非空 ⇒ 成片其实**按补集剪过**（§2.2 规则 3）。
    if config.edl.clips.is_empty() && config.mute_ranges.iter().any(|m| m.normalized().is_some()) {
        warnings.push(
            "NOTE: EDL 为空、但屏蔽轨道非空 —— 成片按 mute_ranges 的补集派生（今天的行为）".into(),
        );
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::{
        load_config_from, load_project, retired_effect_hint, retired_effect_message,
        strip_blocked_sticker_sources, update_polish_toml, validate_edl, validate_project,
        write_atomic, SPEED_MAX, SPEED_MIN,
    };
    use crate::project::{Edl, EdlClip, MuteRange, ProjectFile};
    use yekitrim_render_core::types::Point;

    /// 临时精修工程：只放一份 polish.toml（`example()` 起手）。
    fn tmp_project(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yekitrim-rmw-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = ProjectFile::example();
        std::fs::write(dir.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();
        dir
    }

    /// 原子写：内容整体替换、不留下临时文件（取消渲染是**杀进程**的，
    /// 半个 polish.toml 会让片段直接打不开）。
    #[test]
    fn write_atomic_replaces_and_cleans_up() {
        let dir = tmp_project("atomic");
        let path = dir.join("polish.toml");
        write_atomic(&path, "a = 1\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 1\n");
        write_atomic(&path, "b = 2\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "b = 2\n");
        assert!(!path.with_extension("toml.tmp").exists(), "临时文件要清掉");
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "目录里不该有残留: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **长任务落盘只写自己负责的字段**：LLM 剪辑跑着的时候用户存的选框 /
    /// 屏蔽轨道 / 封面不能被结尾那次写回吞掉（用户报的"改了没生效"根因）。
    #[tokio::test]
    async fn update_polish_toml_keeps_concurrent_edits() {
        let dir = tmp_project("keep");
        // ① 任务开头读一次（模拟 cmd_llm 的起始 load，之后不再看磁盘）
        let snapshot = load_project(&dir).unwrap();

        // ② 任务跑的过程中，用户存了选框（走的是同一把路径锁的读-改-写）
        update_polish_toml(&dir, |cfg| {
            cfg.layout.liver.top_left = Some(Point { x: "11.00%".into(), y: "22.00%".into() });
            cfg.mute_ranges.push(MuteRange { start: 3.0, end: 5.0 });
        })
        .await
        .unwrap();

        // ③ 任务结束落盘：只写它负责的 segments / filter
        let mut segments = snapshot.segments.clone();
        segments.truncate(1);
        update_polish_toml(&dir, move |cfg| {
            cfg.segments = segments;
            cfg.filter.highlight = vec!["新关键词".into()];
        })
        .await
        .unwrap();

        let after = load_project(&dir).unwrap();
        let tl = after.layout.liver.top_left.expect("并行改的选框必须还在");
        assert_eq!(tl.x, "11.00%");
        assert_eq!(tl.y, "22.00%");
        assert_eq!(after.mute_ranges.len(), 1, "并行改的屏蔽轨道必须还在");
        assert!(after.filter.highlight.iter().any(|w| w == "新关键词"), "本次负责的字段要写进去");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn strips_irasu_keeps_others() {
        let mut src = vec!["emoji".to_string(), "irasu".to_string(), "ysyk".to_string()];
        strip_blocked_sticker_sources(&mut src);
        assert_eq!(src, vec!["emoji".to_string(), "ysyk".to_string()]);
    }

    #[test]
    fn no_irasu_unchanged() {
        let mut src = vec!["ysyk".to_string()];
        strip_blocked_sticker_sources(&mut src);
        assert_eq!(src, vec!["ysyk".to_string()]);
    }

    #[test]
    fn only_irasu_becomes_empty() {
        let mut src = vec!["irasu".to_string()];
        strip_blocked_sticker_sources(&mut src);
        assert!(src.is_empty());
    }

    /// **已下线特效的人话报错**（类型收敛铺路）：含 `stutter` 这类已删类型的工程，
    /// 报错要说清"这是本版本删掉的"并给出两条路，同时**保留原始 serde 文本**。
    ///
    /// ⚠️ 这条测试用**合成**的 serde 错误来跑（那四种类型现在还合法，读得出来）——
    /// 类型收敛（`docs/plan/fx-type-consolidation.md`）落地后，它测的正是真实路径。
    #[test]
    fn retired_effect_error_is_human_readable() {
        let project = std::path::Path::new("X:/clips/clip-01/polish.toml");
        // ① serde 真实报错的形状（收敛落地后就是这样）⇒ 必须给人话
        let raw = "unknown variant `stutter`, expected one of `camera`, `shake`, `flash`";
        let msg = retired_effect_message(raw, project).expect("含已下线类型时必须给人话");
        assert!(msg.contains("已下线"), "要说清是本版本删掉的：{msg}");
        assert!(msg.contains("stutter"), "要点名是哪个类型：{msg}");
        assert!(msg.contains("X:/clips/clip-01/polish.toml"), "要点名是哪个文件：{msg}");
        assert!(msg.contains(raw), "原始 serde 文本要保留（排查用）：{msg}");
        // ② 普通解析错误 ⇒ **原样透传**（不许都包装成"已下线"）
        let other = "invalid type: string \"不是数组\", expected a sequence";
        assert!(retired_effect_message(other, project).is_none(), "普通错误不该被包装：{other}");
        // ③ 只是**提到**这些名字（`expected one of` 列表里）不算 —— 裸匹配会误判
        let listed = "unknown variant `nonexistent`, expected one of `shake`, `stutter`, `split`";
        assert!(
            retired_effect_message(listed, project).is_none(),
            "只是列在 expected one of 里、真正错的是别的类型 ⇒ 不该包装：{listed}"
        );
        // ④ 包装函数对真实 serde 错误的行为（普通错误透传）
        let json = serde_json::json!({"source": {"video": "clip.mp4"}, "segments": "不是数组"});
        let err = serde_json::from_value::<ProjectFile>(json).expect_err("类型不对必须报错");
        let raw2 = err.to_string();
        assert_eq!(retired_effect_hint(err, project).to_string(), raw2, "普通错误要原样透传");
    }

    /// 真工程里读一遍（含 `[[segments.events]]`）—— 走的就是被包装过的那条路。
    #[test]
    fn load_config_from_reads_events_and_wraps_errors() {
        let dir = tmp_project("load-config");
        let mut cfg = ProjectFile::example();
        cfg.segments[0].events.push(
            serde_json::from_value(serde_json::json!({"type": "flash", "time": 0.0, "end": 0.3})).unwrap(),
        );
        // `update_polish_toml` 是 async ⇒ 这里直接用同步的 `write_atomic` 落盘（测的是**读**那条路）
        write_atomic(&dir.join("polish.toml"), &cfg.to_toml_commented().unwrap()).unwrap();
        let loaded = load_config_from(&dir, "polish.toml").expect("正常工程必须读得出来");
        assert_eq!(loaded.segments[0].events.len(), 1);
    }

    // ===== 成片时间线（EDL）的校验（规格 §4.1 第 7 条）=====

    fn edl_cfg(clips: Vec<EdlClip>) -> ProjectFile {
        ProjectFile { edl: Edl { clips }, ..ProjectFile::example() }
    }

    fn clip(s: f64, e: f64, speed: f64) -> EdlClip {
        EdlClip {
            source_start: s,
            source_end: e,
            speed,
            kind: None,
            label: None,
            source_kind: None,
            asset_id: None,
            transition_in: None,
        }
    }

    /// `SPEED_MIN` / `SPEED_MAX` **必须**等于 ffmpeg `atempo` 的单级范围
    /// （§3.2 / §4.1 第 8 条 / R6）。把这两个数写死在这里，是为了**改常量必须改这里** ——
    /// 常量表与 atempo 漂了，第 4 批的真出片会在音频滤镜上失败或静默失效。
    #[test]
    fn speed_bounds_match_atempo_single_stage() {
        assert_eq!(SPEED_MIN, 0.5);
        assert_eq!(SPEED_MAX, 2.0);
        // 区间两端**必须合法**（边界值就是用户能选的档位），越界一格必须报。
        assert!(validate_edl(&edl_cfg(vec![clip(0.0, 1.0, SPEED_MIN)])).is_empty());
        assert!(validate_edl(&edl_cfg(vec![clip(0.0, 1.0, SPEED_MAX)])).is_empty());
    }

    /// 四条判据各自能翻面：负起点 / 倒置 / 零长 / 速率越界。
    #[test]
    fn edl_validation_reports_bad_ranges_and_speeds() {
        // ① 负源起点（会被 `edl_frame_at` 夹成 0 ⇒ 与上一段重合，不报错就没人知道）
        let w = validate_edl(&edl_cfg(vec![clip(-1.0, 2.0, 1.0)]));
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("source_start"), "{w:?}");

        // ② end <= start（零长与倒置）
        for c in [clip(3.0, 3.0, 1.0), clip(9.0, 7.0, 1.0)] {
            let w = validate_edl(&edl_cfg(vec![c.clone()]));
            assert!(
                w.iter().any(|m| m.contains("source_end")),
                "零长/倒置段必须报：{}\n{w:?}",
                format!("{:?}", c)
            );
        }

        // ③ 速率越界（两侧各一格）+ 非有限
        for sp in [0.25, 2.5, 0.0, -1.0, f64::NAN] {
            let w = validate_edl(&edl_cfg(vec![clip(0.0, 1.0, sp)]));
            assert!(
                w.iter().any(|m| m.contains("speed") && m.contains("atempo")),
                "speed={sp} 必须报且点明 atempo：{w:?}"
            );
        }

        // ④ **不重叠是允许的**（重放/冷开场：同一段出现两次，§2.1）⇒ 不许有重叠判据
        let replay_ok = edl_cfg(vec![clip(5.0, 7.0, 1.0), clip(5.0, 7.0, 1.0)]);
        assert!(validate_edl(&replay_ok).is_empty(), "重复段不是错误");
        // 乱序同样允许（数组顺序 = 成片播放顺序，不是源顺序）
        let out_of_order = edl_cfg(vec![clip(20.0, 22.0, 1.0), clip(0.0, 2.0, 1.0)]);
        assert!(validate_edl(&out_of_order).is_empty(), "乱序剪辑不是错误");
    }

    /// 空 EDL + 非空 `mute_ranges` ⇒ 报 **NOTE**（成片其实按补集剪过，§2.2 规则 3）。
    /// 不说清的话，用户会以为"成片轨是空的 = 什么都不剪"。
    #[test]
    fn empty_edl_with_mute_ranges_gets_a_note() {
        let mut cfg = ProjectFile::example();
        assert!(validate_edl(&cfg).is_empty(), "干净的老工程不该有新告警（R4 面）");
        cfg.mute_ranges = vec![MuteRange { start: 3.0, end: 5.0 }];
        let w = validate_edl(&cfg);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].starts_with("NOTE:"), "{w:?}");
        assert!(w[0].contains("补集"), "{w:?}");

        // 一旦有 EDL，NOTE 就消失（此时 mute_ranges 被忽略，不再是"补集"）
        cfg.edl.clips = vec![clip(0.0, 2.0, 1.0)];
        assert!(validate_edl(&cfg).is_empty(), "{:?}", validate_edl(&cfg));
    }

    /// `validate_project`（路径那条路）确实把 EDL 判据带出去了 —— 别让新校验只挂在私有函数上。
    #[test]
    fn validate_project_includes_edl_warnings() {
        let dir = tmp_project("edl-validate");
        let mut cfg = ProjectFile::example();
        cfg.edl.clips = vec![clip(0.0, 2.0, 4.0)];
        let w = validate_project(&cfg, &dir);
        assert!(w.iter().any(|m| m.contains("EDL clip 0")), "{w:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
