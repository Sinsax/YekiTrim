use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::config::AsrConfig;
use crate::types::TranscriptSegment;

pub fn find_binary() -> Option<PathBuf> {
    let path = crate::download::vad_binary_path();
    if path.exists() {
        return Some(path);
    }
    let name = if cfg!(target_os = "windows") {
        "sherpa-onnx-vad-with-offline-asr.exe"
    } else {
        "sherpa-onnx-vad-with-offline-asr"
    };
    which::which(name).ok()
}

fn model_dir(cfg: &AsrConfig) -> PathBuf {
    if cfg.local_model_dir.is_empty() {
        crate::download::vtrim_home()
            .join("models")
            .join(&cfg.local_model)
    } else {
        PathBuf::from(&cfg.local_model_dir)
    }
}

fn find_model(cfg: &AsrConfig, model_dir: &Path) -> Result<PathBuf> {
    let candidates = [
        "model.int8.onnx",
        "model.onnx",
        "sense-voice.onnx",
    ];
    for name in &candidates {
        let p = model_dir.join(name);
        if p.exists() {
            return Ok(p);
        }
    }
    bail!(
        "模型目录 {} 中未找到模型文件 (model.int8.onnx / model.onnx / sense-voice.onnx) for model={}",
        model_dir.display(),
        cfg.local_model
    );
}

pub fn ensure_ready(cfg: &AsrConfig) -> Result<()> {
    let binary = find_binary().context(
        "sherpa-onnx-vad-with-offline-asr 未安装，请运行 `vtrim download-models`",
    )?;

    let mdir = model_dir(cfg);
    let model = find_model(cfg, &mdir)?;
    let tokens = mdir.join("tokens.txt");
    if !tokens.exists() {
        bail!("模型目录缺少 tokens.txt: {}", mdir.display());
    }

    let vad = crate::download::vad_model_path();
    if !vad.exists() {
        bail!("Silero VAD 模型未下载: {}", vad.display());
    }

    tracing::info!(
        "VAD+ASR 就绪: {}, 模型: {}",
        binary.display(),
        model.display()
    );
    Ok(())
}

pub fn transcribe_with_vad(audio: &Path, cfg: &AsrConfig) -> Result<Vec<TranscriptSegment>> {
    let binary = find_binary().context("sherpa-onnx-vad-with-offline-asr 未找到")?;

    let mdir = model_dir(cfg);
    let model = find_model(cfg, &mdir)?;
    let tokens = mdir.join("tokens.txt");
    let vad_model = crate::download::vad_model_path();

    let providers = if crate::download::gpu_binary_path().exists() {
        vec!["cuda", "cpu"]
    } else {
        vec!["cpu"]
    };

    for (i, provider) in providers.iter().enumerate() {
        let mut cmd = Command::new(&binary);

        cmd.arg(format!("--silero-vad-model={}", vad_model.display()))
            .arg(format!(
                "--silero-vad-threshold={}",
                cfg.vad.threshold
            ))
            .arg(format!(
                "--silero-vad-min-silence-duration={}",
                cfg.vad.min_silence_duration
            ))
            .arg(format!(
                "--silero-vad-min-speech-duration={}",
                cfg.vad.min_speech_duration
            ))
            .arg(format!(
                "--silero-vad-max-speech-duration={}",
                cfg.vad.max_speech_duration
            ))
            .arg(format!("--tokens={}", tokens.display()))
            .arg(format!("--provider={}", provider))
            .arg("--num-threads=2");

        if cfg.local_model == "sensevoice" {
            cmd.arg(format!("--sense-voice-model={}", model.display()));
            cmd.arg("--sense-voice-use-itn=true");
        } else {
            cmd.arg(format!("--paraformer={}", model.display()));
        }

        cmd.arg(audio);

        tracing::debug!("VAD+ASR 命令 (provider={}): {:?}", provider, cmd);

        let output = match cmd.output() {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!("VAD+ASR 启动失败 (provider={}): {}", provider, e);
                continue;
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if i < providers.len() - 1 {
                tracing::warn!(
                    "VAD+ASR provider={} 失败 ({}), 降级到 {}",
                    provider,
                    output.status,
                    providers[i + 1]
                );
                if i == 0 {
                    tracing::debug!("stderr: {}", stderr.trim());
                }
            } else {
                bail!(
                    "VAD+ASR provider={} 失败 ({}): {}",
                    provider,
                    output.status,
                    stderr.trim()
                );
            }
            continue;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        let segments = parse_vad_output(&stdout);
        if !segments.is_empty() {
            tracing::info!(
                "VAD+ASR: {} 个片段 (provider={}, model={})",
                segments.len(),
                provider,
                cfg.local_model
            );
            return Ok(segments);
        }

        tracing::warn!(
            "VAD+ASR provider={} 输出为空或格式不符, 降级",
            provider
        );
    }

    bail!("所有 provider 均失败，无法完成转写");
}

fn parse_vad_output(stdout: &str) -> Vec<TranscriptSegment> {
    let mut segments = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Some((time_part, text)) = line.split_once(": ") else {
            continue;
        };
        let Some((start_str, end_str)) = time_part.split_once(" -- ") else {
            continue;
        };

        let start: f64 = match start_str.trim().parse() {
            Ok(s) => s,
            Err(_) => continue,
        };
        let end: f64 = match end_str.trim().parse() {
            Ok(e) => e,
            Err(_) => continue,
        };
        let text = text.trim().to_string();

        if text.is_empty() {
            continue;
        }

        segments.push(TranscriptSegment {
            start,
            end,
            text,
            confidence: 1.0,
        });
    }

    segments
}
