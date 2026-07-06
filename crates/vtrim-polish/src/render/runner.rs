use crate::project::ProjectFile;
use std::path::Path;

/// 渲染管线编排
pub struct RenderRunner {
    config: ProjectFile,
    project_path: std::path::PathBuf,
}

impl RenderRunner {
    pub fn new(config: ProjectFile, project_path: &Path) -> Self {
        Self {
            config,
            project_path: project_path.to_path_buf(),
        }
    }

    /// 运行完整渲染流程
    pub async fn run(&self) -> anyhow::Result<()> {
        let resolved = self.resolve_paths();

        println!("  Video: {}", resolved.video);
        println!("  Frames: {}", resolved.frames_dir.display());
        println!("  Output: {}", resolved.output);

        // Step 1: 预提取视频帧
        let fps = 30;
        let frame_count = Self::extract_frames_to_dir(&resolved.video, &resolved.frames_dir, fps)?;
        println!("  ✓ {} frames extracted", frame_count);

        // Step 2: 渲染 (将在 chrome.rs 中实现)
        println!("  (Chrome CDP frame capture coming in next phase)");

        Ok(())
    }

    /// 提取视频帧 (public static)
    pub fn extract_frames_to_dir(video_path: &str, frame_dir: &std::path::Path, fps: usize) -> anyhow::Result<usize> {
        use std::process::Command;

        // 转换为绝对路径避免相对路径歧义
        let abs_frame_dir = std::path::absolute(frame_dir)
            .unwrap_or_else(|_| frame_dir.to_path_buf());
        let abs_video = std::path::absolute(video_path)
            .unwrap_or_else(|_| std::path::PathBuf::from(video_path));

        if abs_frame_dir.exists() {
            std::fs::remove_dir_all(&abs_frame_dir).ok();
        }
        std::fs::create_dir_all(&abs_frame_dir)?;

        let output_pattern = abs_frame_dir.join("frame_%d.jpg");
        let status = Command::new("ffmpeg")
            .args([
                "-i", &abs_video.to_string_lossy(),
                "-q:v", "2",
                "-r", &fps.to_string(),
                "-start_number", "0",
                &output_pattern.to_string_lossy(),
            ])
            .status()?;

        if !status.success() {
            anyhow::bail!("ffmpeg frame extraction failed");
        }

        let count = std::fs::read_dir(&abs_frame_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "jpg"))
            .count();

        Ok(count)
    }

    fn resolve_paths(&self) -> ResolvedPaths {
        let to_abs = |relative: &str| -> String {
            let p = Path::new(relative);
            if p.is_relative() {
                self.project_path.join(p).to_string_lossy().to_string()
            } else {
                p.to_string_lossy().to_string()
            }
        };

        ResolvedPaths {
            video: to_abs(&self.config.source.video),
            srt: to_abs(&self.config.source.subtitles),
            danmaku: to_abs(&self.config.source.danmaku),
            output: to_abs(&self.config.project.output),
            frames_dir: self.project_path.join("frames"),
        }
    }
}

#[allow(dead_code)] struct ResolvedPaths {
    video: String,
    srt: String,
    danmaku: String,
    output: String,
    frames_dir: std::path::PathBuf,
}
