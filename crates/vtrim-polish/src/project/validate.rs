use std::path::Path;
use super::ProjectFile;

/// 从项目目录加载 polish.toml
pub fn load_project(project_path: &Path) -> anyhow::Result<ProjectFile> {
    load_config_from(project_path, "polish.toml")
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
    let mut config: ProjectFile = toml::from_str(&raw)?;
    strip_blocked_sticker_sources(&mut config.sticker_source);
    Ok(config)
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
    use super::strip_blocked_sticker_sources;

    #[test]
    fn strips_irasu_keeps_others() {
        let mut src = vec!["emoji".to_string(), "irasu".to_string(), "local".to_string()];
        strip_blocked_sticker_sources(&mut src);
        assert_eq!(src, vec!["emoji".to_string(), "local".to_string()]);
    }

    #[test]
    fn no_irasu_unchanged() {
        let mut src = vec!["local".to_string()];
        strip_blocked_sticker_sources(&mut src);
        assert_eq!(src, vec!["local".to_string()]);
    }

    #[test]
    fn only_irasu_becomes_empty() {
        let mut src = vec!["irasu".to_string()];
        strip_blocked_sticker_sources(&mut src);
        assert!(src.is_empty());
    }
}
