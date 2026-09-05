//! 静音压缩 Service：CLI / WebUI 共用。

use std::path::PathBuf;

pub fn run_silence(
    files: &[PathBuf],
    duration: f64,
    threshold: Option<f64>,
    output: Option<PathBuf>,
) -> anyhow::Result<()> {
    let noise_db = threshold.unwrap_or(-30.0);
    let targets: Vec<PathBuf> = if files.is_empty() {
        let clips = std::env::current_dir().unwrap_or_default().join("output").join("clips");
        let mut list = Vec::new();
        if clips.exists() {
            for e in std::fs::read_dir(&clips)? {
                let e = e?;
                if e.path().is_dir() {
                    let f = e.path().join("clip.mp4");
                    if f.exists() { list.push(f); }
                }
            }
        }
        list.sort();
        list
    } else {
        files.to_vec()
    };
    if targets.is_empty() {
        anyhow::bail!("没有找到要处理的视频文件");
    }
    for (i, input) in targets.iter().enumerate() {
        let out_dir = output.clone().unwrap_or_else(|| input.parent().map(|p| p.to_path_buf()).unwrap_or_default());
        std::fs::create_dir_all(&out_dir)?;
        let stem = input.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| format!("clip_{}", i));
        let output_path = out_dir.join(format!("{}_cleaned.mp4", stem));
        let srt_path = input.with_extension("srt");
        let srt_opt = if srt_path.exists() { Some(srt_path.as_path()) } else { None };
        let segments = vtrim_core::cutting::remove_silence(input, &output_path, duration, srt_opt, noise_db)?;
        if srt_path.exists() {
            let out_srt = output_path.with_extension("srt");
            vtrim_core::cutting::adjust_srt(&srt_path, &out_srt, &segments)?;
        }
        let dm_srt_path = input.with_extension("dm.srt");
        if dm_srt_path.exists() {
            let out_dm = output_path.with_extension("dm.srt");
            vtrim_core::cutting::adjust_srt(&dm_srt_path, &out_dm, &segments)?;
        }
    }
    Ok(())
}
