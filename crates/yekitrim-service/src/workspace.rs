//! Workspace：一个 V-Trim 工程在磁盘上的固定根。
//!
//! CLI 历史上依赖进程当前工作目录（CWD）定位 `vtrim.toml` 与输出目录。
//! `vtrim server` 是长驻进程，需要在同一个进程内切换多个工程，因此服务层统一使用
//! 基于配置文件绝对路径的 `Workspace` 来解析所有相对路径。
//!
//! CLI 原有命令仍走 `./vtrim.toml` + CWD 语义；Service 层使用本类型。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// 表示一个 V-Trim 工程及其文件根。
#[derive(Debug, Clone)]
pub struct Workspace {
    /// `vtrim.toml` 的绝对路径。
    pub config_file: PathBuf,
    /// 工程根目录：`config_file` 所在目录。
    pub root_dir: PathBuf,
}

impl Workspace {
    /// 从显式 `--config` 参数或默认 `./vtrim.toml` 创建 Workspace。
    ///
    /// 不会因为项目文件不存在而报错；项目文件可以在稍后 `init` 时创建。
    pub fn load(config_flag: Option<&Path>) -> Result<Self> {
        let config_file = match config_flag {
            Some(p) => absolute(p)?,
            None => std::env::current_dir()
                .context("无法获取当前目录")?
                .join("vtrim.toml"),
        };
        let root_dir = config_file
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Ok(Self {
            config_file,
            root_dir,
        })
    }

    /// 读取当前工作区配置（合并全局配置 + 项目文件 + llm.toml）。
    pub fn load_config(&self) -> Result<yekitrim_core::config::Config> {
        yekitrim_core::config::load_config_at(&self.config_file)
    }

    /// 将配置中记录的相对路径解析为绝对路径；绝对路径原样返回。
    pub fn resolve(&self, value: &str) -> PathBuf {
        let p = Path::new(value);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root_dir.join(p)
        }
    }

    /// 将任意路径解析为相对工程根的路径（供写入 vtrim.toml 时保持可移植性）。
    #[allow(dead_code)]
    pub fn rel(&self, path: &Path) -> PathBuf {
        match path.strip_prefix(&self.root_dir) {
            Ok(rel) => rel.to_path_buf(),
            Err(_) => path.to_path_buf(),
        }
    }

    /// 输出目录（绝对路径）：项目配置优先，未初始化时默认 `./output` 相对工程根。
    pub fn output_dir(&self) -> PathBuf {
        self.load_config()
            .map(|cfg| self.resolve(&cfg.cutting.output_dir))
            .unwrap_or_else(|_| self.root_dir.join("output"))
    }

    /// 当前生效的转写文件：`transcript_file` → `output/transcript.json` → `output/video.srt`。
    ///
    /// ⚠️ `transcript_file` 一律经 [`Workspace::resolve`]（相对**工程根**）解析。
    /// 曾有两处口径不一：`data_transcript_impl` 用裸 `PathBuf::from(...).exists()`
    /// （相对**进程 CWD**），`transcript_starts` 用 `ws.resolve` —— 靠 `output/` 兜底才没暴露，
    /// 一旦把字幕指到外部文件就是"选了不生效"。**新增读取方一律走这里。**
    pub fn transcript_path(&self, cfg: &yekitrim_core::config::Config) -> Option<PathBuf> {
        let configured = cfg
            .project
            .transcript_file
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|s| self.resolve(s))
            .filter(|p| p.is_file());
        if configured.is_some() {
            return configured;
        }
        let output = self.output_dir();
        let json = output.join("transcript.json");
        if json.is_file() {
            return Some(json);
        }
        let srt = output.join("video.srt");
        if srt.is_file() { Some(srt) } else { None }
    }

    /// 当前生效的弹幕文件：`danmaku_file` → 视频同目录的 `danmu.ass` / `danmaku.ass`。
    ///
    /// 只定位文件、不管格式（XML / ASS / SRT 都认，由 `danmaku::load_danmaku_auto` 分派）。
    pub fn danmaku_path(&self, cfg: &yekitrim_core::config::Config) -> Option<PathBuf> {
        if let Some(raw) = cfg.project.danmaku_file.as_ref().filter(|s| !s.is_empty()) {
            let p = self.resolve(raw);
            if p.is_file() {
                return Some(p);
            }
        }
        let input = cfg
            .project
            .input_file
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|s| self.resolve(s))?;
        let dir = input.parent()?;
        for name in ["danmu.ass", "danmaku.ass"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    /// 输出目录是否存在工程文件（供 CLI 旧 `current_stage` 语义使用）。
    #[allow(dead_code)]
    pub fn current_stage(&self) -> u32 {
        let output = self.output_dir();
        self.load_config()
            .map(|cfg| cfg.project.current_stage(&output))
            .unwrap_or(0)
    }
}

fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .context("无法获取当前目录")?
            .join(path)
            .canonicalize()
            .or_else(|_| {
                // canonicalize 对尚不存在的文件会失败；此时保留绝对化但不要求存在。
                Ok(std::env::current_dir()
                    .context("无法获取当前目录")?
                    .join(path))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn load_creates_absolute_config_path() {
        let cwd = std::env::current_dir().unwrap();
        let ws = Workspace::load(Some(Path::new("vtrim.toml"))).unwrap();
        assert!(ws.config_file.is_absolute());
        assert_eq!(ws.root_dir, cwd);
    }

    #[test]
    fn resolve_joins_root_for_relative() {
        let ws = Workspace {
            config_file: PathBuf::from("C:/projects/demo/vtrim.toml"),
            root_dir: PathBuf::from("C:/projects/demo"),
        };
        assert_eq!(
            ws.resolve("output/clips/1.mp4"),
            PathBuf::from("C:/projects/demo/output/clips/1.mp4")
        );
        assert_eq!(
            ws.resolve("C:/other/video.mp4"),
            PathBuf::from("C:/other/video.mp4")
        );
    }

    #[test]
    fn rel_strips_root_prefix() {
        let ws = Workspace {
            config_file: PathBuf::from("C:/projects/demo/vtrim.toml"),
            root_dir: PathBuf::from("C:/projects/demo"),
        };
        assert_eq!(
            ws.rel(Path::new("C:/projects/demo/output/video.wav")),
            PathBuf::from("output/video.wav")
        );
        assert_eq!(
            ws.rel(Path::new("C:/elsewhere/video.mp4")),
            PathBuf::from("C:/elsewhere/video.mp4")
        );
    }

    #[test]
    fn config_file_missing_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vtrim.toml");
        let ws = Workspace {
            config_file: path.clone(),
            root_dir: dir.path().to_path_buf(),
        };
        let cfg = ws.load_config().unwrap();
        assert!(cfg.project.input_file.is_none());
        assert_eq!(
            ws.output_dir(),
            dir.path().join("output")
        );
        let _ = fs::remove_dir_all(dir.path());
    }

    /// `transcript_file` 必须相对**工程根**解析（而不是进程 CWD），且优先于 output/ 兜底。
    #[test]
    fn transcript_path_resolves_configured_against_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("output")).unwrap();
        fs::write(root.join("sub.srt"), "1\n00:00:01,000 --> 00:00:02,000\nhi\n").unwrap();
        fs::write(root.join("output/transcript.json"), "{\"segments\":[]}").unwrap();
        let ws = Workspace {
            config_file: root.join("vtrim.toml"),
            root_dir: root.to_path_buf(),
        };
        let mut cfg = yekitrim_core::config::Config::default();
        cfg.project.transcript_file = Some("sub.srt".to_string());
        assert_eq!(ws.transcript_path(&cfg), Some(root.join("sub.srt")));
    }

    /// 没配置时依次回退 output/transcript.json → output/video.srt。
    #[test]
    fn transcript_path_falls_back_to_output_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("output")).unwrap();
        let ws = Workspace {
            config_file: root.join("vtrim.toml"),
            root_dir: root.to_path_buf(),
        };
        let cfg = yekitrim_core::config::Config::default();
        assert_eq!(ws.transcript_path(&cfg), None);
        fs::write(root.join("output/video.srt"), "1\n00:00:01,000 --> 00:00:02,000\nhi\n").unwrap();
        assert_eq!(ws.transcript_path(&cfg), Some(root.join("output/video.srt")));
        fs::write(root.join("output/transcript.json"), "{\"segments\":[]}").unwrap();
        assert_eq!(ws.transcript_path(&cfg), Some(root.join("output/transcript.json")));
    }

    /// 弹幕：配置优先；缺失时回退视频同目录的 danmu.ass / danmaku.ass。
    #[test]
    fn danmaku_path_prefers_config_then_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("videos")).unwrap();
        fs::write(root.join("videos/a.mp4"), b"x").unwrap();
        let ws = Workspace {
            config_file: root.join("vtrim.toml"),
            root_dir: root.to_path_buf(),
        };
        let mut cfg = yekitrim_core::config::Config::default();
        cfg.project.input_file = Some("videos/a.mp4".to_string());
        assert_eq!(ws.danmaku_path(&cfg), None);
        fs::write(root.join("videos/danmu.ass"), "[Events]\n").unwrap();
        assert_eq!(ws.danmaku_path(&cfg), Some(root.join("videos/danmu.ass")));

        // 配置指向别处（工程根相对）时以配置为准。
        fs::write(root.join("dm.xml"), "<i/>").unwrap();
        cfg.project.danmaku_file = Some("dm.xml".to_string());
        assert_eq!(ws.danmaku_path(&cfg), Some(root.join("dm.xml")));
    }
}
