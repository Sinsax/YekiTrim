use std::process::{Command, Stdio};

/// ffmpeg 操作包装
pub struct Ffmpeg;

impl Ffmpeg {
    /// 检测可用编码器 (NVENC 优先)
    pub fn detect_encoder() -> EncoderInfo {
        let output = Command::new("ffmpeg")
            .args(["-hide_banner", "-encoders"])
            .output()
            .ok();

        if let Some(out) = output {
            let text = String::from_utf8_lossy(&out.stdout);
            if text.contains("nvenc") {
                return EncoderInfo {
                    codec: "h264_nvenc".into(),
                    args: vec!["-preset".into(), "p1".into(), "-cq".into(), "22".into()],
                };
            }
        }

        EncoderInfo {
            codec: "libx264".into(),
            args: vec!["-preset".into(), "veryfast".into(), "-crf".into(), "22".into()],
        }
    }

    /// Concat 多个 MP4 文件
    pub fn concat(inputs: &[String], output: &str) -> anyhow::Result<()> {
        let mut concat_content = String::new();
        for input in inputs {
            // 使用绝对路径，因为 concat 文件在临时目录中
            let abs_path = std::path::absolute(input)
                .unwrap_or_else(|_| std::path::PathBuf::from(input));
            concat_content.push_str(&format!("file '{}'\n", abs_path.to_string_lossy().replace('\\', "/")));
        }

        let abs_output = std::path::absolute(output)
            .unwrap_or_else(|_| std::path::PathBuf::from(output));

        let concat_file = std::env::temp_dir().join("_polish_concat.txt");
        std::fs::write(&concat_file, &concat_content)?;

        let status = Command::new("ffmpeg")
            .args([
                "-y", "-f", "concat", "-safe", "0",
                "-i", &concat_file.to_string_lossy(),
                "-c", "copy",
                &abs_output.to_string_lossy(),
            ])
            .stderr(Stdio::null()).stdout(Stdio::null()).status()?;

        std::fs::remove_file(&concat_file).ok();

        if !status.success() {
            anyhow::bail!("ffmpeg concat failed");
        }
        Ok(())
    }

    /// 提取音频
    pub fn extract_audio(video_path: &str, audio_output: &str) -> anyhow::Result<()> {
        let status = Command::new("ffmpeg")
            .args([
                "-y", "-i", video_path,
                "-vn", "-acodec", "aac", "-b:a", "128k",
                audio_output,
            ])
            .stderr(Stdio::null()).stdout(Stdio::null()).status()?;

        if !status.success() {
            anyhow::bail!("ffmpeg audio extraction failed");
        }
        Ok(())
    }

    /// 混入音效
    /// 混音后限幅: 防止音效峰值爆音 (alimiter)
    pub fn apply_limiter(input: &str, output: &str) -> anyhow::Result<()> {
        let out = Command::new("ffmpeg")
            .args(["-y", "-i", input,
                   "-af", "alimiter=limit=0.95:level=false",
                   "-c:a", "aac", output])
            .output()
            .map_err(|e| anyhow::anyhow!("limiter 启动失败: {}", e))?;
        if !out.status.success() {
            anyhow::bail!("limiter 失败: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }

    pub fn mix_sfx(
        audio_input: &str,
        sfx_events: &[SfxEvent],
        output: &str,
    ) -> anyhow::Result<()> {
        if sfx_events.is_empty() {
            std::fs::copy(audio_input, output)?;
            return Ok(());
        }

        let mut filter_parts = Vec::new();
        let mut input_args = vec!["-y".to_string(), "-i".to_string(), audio_input.to_string()];
        let mut mix_inputs = vec!["[0:a]".to_string()];

        for (i, event) in sfx_events.iter().enumerate() {
            input_args.push("-i".to_string());
            input_args.push(event.file.clone());
            let delay_ms = (event.time * 1000.0) as u64;
            let dur = event.duration.unwrap_or(0.5);
            let fade_st = (dur - 0.2).max(0.0);
            filter_parts.push(format!(
                "[{}:a]atrim=0:{},afade=t=out:st={}:d=0.2,adelay={}|{},volume={}[s{}]",
                i + 1, dur, fade_st, delay_ms, delay_ms, event.volume, i
            ));
            mix_inputs.push(format!("[s{}]", i));
        }

        let mix_str = mix_inputs.join("");
        let amix = format!(
            "{}amix=inputs={}:duration=first:normalize=0[a]",
            mix_str,
            sfx_events.len() + 1
        );

        let filter_complex = if filter_parts.is_empty() {
            amix
        } else {
            format!("{};{}", filter_parts.join(";"), amix)
        };

        let mut args: Vec<String> = input_args;
        args.extend_from_slice(&[
            "-filter_complex".into(),
            filter_complex,
            "-map".into(),
            "[a]".into(),
            "-acodec".into(),
            "aac".into(),
            "-b:a".into(),
            "128k".into(),
            output.into(),
        ]);

        let status = Command::new("ffmpeg").args(&args).stderr(Stdio::null()).stdout(Stdio::null()).status()?;
        if !status.success() {
            anyhow::bail!("ffmpeg SFX mix failed");
        }
        Ok(())
    }

    /// 视频 + 音频合流
    pub fn mux(video: &str, audio: &str, output: &str) -> anyhow::Result<()> {
        let status = Command::new("ffmpeg")
            .args([
                "-y", "-i", video,
                "-i", audio,
                "-c:v", "copy",
                "-c:a", "aac",
                "-b:a", "128k",
                "-shortest",
                output,
            ])
            .stderr(Stdio::null()).stdout(Stdio::null()).status()?;

        if !status.success() {
            anyhow::bail!("ffmpeg mux failed");
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct EncoderInfo {
    pub codec: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SfxEvent {
    pub time: f64,
    pub file: String,
    pub volume: f64,
    pub duration: Option<f64>,
}
