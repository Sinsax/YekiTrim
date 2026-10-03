//! polish::render::ffmpeg：ffmpeg 调用的唯一收口（编码器探测 / 拼接 / 抽音 / 混 SFX / 按屏蔽区间重编码 / 合流）。
//!
//! 入口：`Ffmpeg::{detect_encoder, concat, extract_audio, apply_limiter, mix_sfx, cut_ranges, mux}`、`EncoderInfo`、`SfxEvent`。
//! 边界：不许改回 `Command::output()` / 管道（Chrome/ffmpeg 是孙进程，会一直持有 stdout 句柄）；编码器一律用 `detect_encoder()` 的结果，别硬编码 —— 见 `crates/yekitrim-polish/src/render/AGENTS.md`

use std::process::{Command, Stdio};

/// Concat 清单临时文件路径。
///
/// ⚠️ 必须**每个进程每次调用都不同**：以前写死 `_polish_concat.txt`，两个渲染（哪怕是
/// 不同项目、不同进程）同时跑就会互相覆盖清单，拼出来的成片是别人的片段。
fn concat_temp_path() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("_polish_concat-{}-{}.txt", std::process::id(), n))
}

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

        let concat_file = concat_temp_path();
        std::fs::write(&concat_file, &concat_content)?;

        let status = Command::new("ffmpeg")
            .args([
                "-y", "-f", "concat", "-safe", "0",
                "-i", &concat_file.to_string_lossy(),
                "-c", "copy",
                &abs_output.to_string_lossy(),
            ])
            .stderr(Stdio::null()).stdout(Stdio::null()).status();

        // 失败路径也要删（以前只删成功路径，报错时临时文件会攒下来）
        std::fs::remove_file(&concat_file).ok();
        let status = status?;

        if !status.success() {
            anyhow::bail!("ffmpeg concat failed");
        }
        Ok(())
    }

    /// 提取音频
    pub fn extract_audio(video_path: &str, audio_output: &str) -> anyhow::Result<()> {
        let out = Command::new("ffmpeg")
            .args([
                "-y", "-i", video_path,
                "-vn", "-acodec", "aac", "-b:a", "128k",
                audio_output,
            ])
            .output()
            .map_err(|e| anyhow::anyhow!("ffmpeg audio extraction 启动失败: {}", e))?;

        if !out.status.success() {
            anyhow::bail!(
                "ffmpeg audio extraction failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
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

    /// 按屏蔽区间剪掉一段时间：视频/音频同步跳过的部分会被移除。
    ///
    /// 这里是**整段重编码**（select 过滤没法流转发），所以编码器要走
    /// [`Ffmpeg::detect_encoder`] —— 硬编码 libx264 时，一个 4 分钟的切片要 17s，
    /// 而同一个出片流程的其它环节早就用上 NVENC 了。画质档与分块编码保持一致。
    pub fn cut_ranges(input: &str, ranges: &[(f64, f64)], output: &str) -> anyhow::Result<()> {
        let mut parts: Vec<String> = Vec::new();
        for (s, e) in ranges {
            if e > s {
                parts.push(format!(r"not(between(t\,{s:.3}\,{e:.3}))"));
            }
        }
        if parts.is_empty() {
            return Ok(());
        }
        let expr = parts.join("*");
        let vf = format!("select='{expr}',setpts=N/FRAME_RATE/TB");
        let af = format!("aselect='{expr}',asetpts=N/SR/TB");
        let enc = Self::detect_encoder();
        let mut args: Vec<String> = vec![
            "-y".into(), "-i".into(), input.into(),
            "-vf".into(), vf,
            "-af".into(), af,
            "-c:v".into(), enc.codec.clone(),
        ];
        args.extend(enc.args.iter().cloned());
        args.extend([
            "-pix_fmt".into(), "yuv420p".into(),
            "-c:a".into(), "aac".into(),
            "-b:a".into(), "160k".into(),
            "-movflags".into(), "+faststart".into(),
            output.into(),
        ]);
        let out = Command::new("ffmpeg")
            .args(&args)
            .output()
            .map_err(|e| anyhow::anyhow!("ffmpeg cut ranges 启动失败: {}", e))?;
        if !out.status.success() {
            anyhow::bail!("ffmpeg cut ranges failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }

    /// 视频 + 音频合流
    pub fn mux(video: &str, audio: &str, output: &str) -> anyhow::Result<()> {
        let out = Command::new("ffmpeg")
            .args([
                "-y", "-i", video,
                "-i", audio,
                "-c:v", "copy",
                "-c:a", "aac",
                "-b:a", "128k",
                "-shortest",
                output,
            ])
            .output()
            .map_err(|e| anyhow::anyhow!("ffmpeg mux 启动失败: {}", e))?;

        if !out.status.success() {
            anyhow::bail!(
                "ffmpeg mux failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
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
