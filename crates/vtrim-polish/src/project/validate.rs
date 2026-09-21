//! polish::project::validate：`polish.toml` 的加载 / 原子写 / 校验，以及读-改-写的唯一通道。
//!
//! 入口：`load_project`、`load_config_from`、`update_polish_toml`、`write_atomic`、`validate_project`。
//! 边界：长任务落盘只能走 `update_polish_toml`（锁内重新 load + 只覆盖自己负责的字段），不许"开头 load、结尾整份写回" —— 见 `crates/vtrim-polish/src/project/AGENTS.md`

use std::path::Path;
use super::ProjectFile;

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
/// 锁是 `vtrim_core::filelock` 的路径锁（与服务端 handler 同一把，同进程内串行）；
/// 闭包只做内存改动，写文件在本函数内完成。
pub async fn update_polish_toml<F>(project_path: &Path, edit: F) -> anyhow::Result<()>
where
    F: FnOnce(&mut ProjectFile),
{
    let config_path = project_path.join("polish.toml");
    let write_path = config_path.clone();
    let dir = project_path.to_path_buf();
    vtrim_core::filelock::with_path_lock(&config_path, move || -> anyhow::Result<()> {
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
            "{} not found at {}. Run 'vtrim polish init' first.",
            filename,
            config_path.display()
        );
    }
    let raw = std::fs::read_to_string(&config_path)?;
    // 老工程的 `duration` → `end` 在反序列化前升级（见 project/legacy.rs）
    let toml_value: toml::Value = toml::from_str(&raw)?;
    let json = super::legacy::upgrade_toml_value(&toml_value)?;
    let mut config: ProjectFile = serde_json::from_value(json)?;
    strip_blocked_sticker_sources(&mut config.sticker_source);
    fill_default_local_packages(&mut config.sticker_source);
    Ok(config)
}

/// 默认自动载入已 build 的贴纸包（从 stickers/sticker.toml 索引读取）。
/// 仅当配置里没有任何本地贴纸包时才追加，避免重复。
fn fill_default_local_packages(sources: &mut Vec<String>) {
    use crate::assets::sticker_index::{load_sticker_index, is_package_source};
    let root = vtrim_core::path::sticker_dir();
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

    warnings
}

#[cfg(test)]
mod tests {
    use super::{strip_blocked_sticker_sources, update_polish_toml, load_project, write_atomic};
    use crate::project::{MuteRange, ProjectFile};
    use vtrim_render_core::types::Point;

    /// 临时精修工程：只放一份 polish.toml（`example()` 起手）。
    fn tmp_project(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vtrim-rmw-{tag}-{}-{:?}",
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
}
