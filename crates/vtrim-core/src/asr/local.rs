use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::config::AsrConfig;
use crate::types::TranscriptSegment;

#[derive(Deserialize)]
struct SherpaOutput {
    text: String,
    #[serde(default)]
    timestamps: Vec<f64>,
    #[serde(default)]
    tokens: Vec<String>,
}

fn model_dir(cfg: &AsrConfig) -> PathBuf {
    if !cfg.local_model_dir.is_empty() {
        PathBuf::from(&cfg.local_model_dir)
    } else {
        crate::download::vtrim_home().join("models").join(&cfg.local_model)
    }
}

fn find_model(model_dir: &Path) -> Result<PathBuf> {
    let candidates = ["model.int8.onnx", "model.onnx", "sense-voice.onnx"];
    for name in &candidates {
        let p = model_dir.join(name);
        if p.exists() {
            return Ok(p);
        }
    }
    bail!(
        "模型目录 {} 中未找到模型文件 (model.int8.onnx / model.onnx)",
        model_dir.display()
    );
}

pub fn transcribe(audio: &Path, cfg: &AsrConfig, offset: f64) -> Result<Vec<TranscriptSegment>> {
    let binaries = resolve_binaries();
    if binaries.is_empty() {
        bail!("sherpa-onnx-offline 未找到，请运行 `vtrim download-models` 或手动安装");
    }

    let mdir = model_dir(cfg);
    let model = find_model(&mdir)?;
    let tokens = mdir.join("tokens.txt");
    if !tokens.exists() {
        bail!("模型目录缺少 tokens.txt: {}", mdir.display());
    }

    let mut last_err = None;
    for binary in &binaries {
        match run_sherpa(binary, audio, &model, &tokens, offset) {
            Ok(Some(segs)) => return Ok(segs),
            Ok(None) => return Ok(vec![]),
            Err(e) => {
                tracing::warn!("{} 失败: {}", binary.display(), e);
                last_err = Some(e);
            }
        }
    }

    bail!("所有 binary 均失败: {}", last_err.unwrap())
}

fn resolve_binaries() -> Vec<PathBuf> {
    let cpu = crate::download::cpu_binary_path();
    if cpu.exists() {
        return vec![cpu];
    }
    if let Ok(which) = which::which("sherpa-onnx-offline") {
        return vec![which];
    }
    Vec::new()
}

fn run_sherpa(
    binary: &Path,
    audio: &Path,
    model: &Path,
    tokens: &Path,
    offset: f64,
) -> Result<Option<Vec<TranscriptSegment>>> {
    let mut cmd = Command::new(binary);
    cmd.arg(format!("--tokens={}", tokens.display()));
    cmd.arg("--num-threads=2");
    cmd.arg(format!("--sense-voice-model={}", model.display()));
    cmd.arg("--sense-voice-use-itn=true");
    cmd.arg(audio);

    let output = cmd.output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("返回错误 ({}): {}", output.status, stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let sherpa: SherpaOutput = serde_json::from_str(&stdout)
        .with_context(|| format!("解析 JSON 输出失败: {}", stdout.chars().take(200).collect::<String>()))?;

    let text = sherpa.text.trim().to_string();

    if text.is_empty() {
        return Ok(None);
    }

    if !sherpa.timestamps.is_empty() && sherpa.timestamps.len() >= 2 {
        let mut segs = build_segments_from_timestamps(
            &sherpa.tokens,
            &sherpa.timestamps,
            offset,
        );
        merge_short_segments(&mut segs);
        Ok(Some(segs))
    } else {
        let duration_secs = audio_duration(audio);
        Ok(Some(vec![TranscriptSegment {
            start: offset,
            end: offset + duration_secs,
            text,
            confidence: 1.0,
        }]))
    }
}

fn build_segments_from_timestamps(
    tokens: &[String],
    timestamps: &[f64],
    offset: f64,
) -> Vec<TranscriptSegment> {
    // Group tokens into sentences on punctuation or time gaps >0.6s
    let sentence_enders = ['。', '！', '？', '.', '!', '?'];
    let mut segments: Vec<TranscriptSegment> = Vec::new();
    let mut buf_text = String::new();
    let mut seg_start = timestamps[0];

    for i in 0..tokens.len() {
        let tok = &tokens[i];
        let ts = timestamps[i];

        // If there's a big time gap, start a new segment
        if i > 0 {
            let prev_ts = timestamps[i - 1];
            // Estimate end of previous token as current timestamp
            // (since durations is empty)
            if ts - prev_ts > 0.4 && !buf_text.is_empty() {
                let end = (ts + prev_ts) / 2.0;
                segments.push(TranscriptSegment {
                    start: offset + seg_start,
                    end: offset + end,
                    text: buf_text.trim().to_string(),
                    confidence: 1.0,
                });
                buf_text.clear();
                seg_start = ts;
            }
        }

        buf_text.push_str(tok);

        // Check if this character is a sentence ender
        if let Some(ch) = tok.chars().next() {
            if sentence_enders.contains(&ch) {
                let end = if i + 1 < timestamps.len() {
                    (timestamps[i + 1] + ts) / 2.0
                } else {
                    ts + 0.3
                };
                segments.push(TranscriptSegment {
                    start: offset + seg_start,
                    end: offset + end,
                    text: buf_text.trim().to_string(),
                    confidence: 1.0,
                });
                buf_text.clear();
                if i + 1 < timestamps.len() {
                    seg_start = timestamps[i + 1];
                }
                continue;
            }
        }

        // Force split if segment is getting too long
        if !buf_text.is_empty() && ts - seg_start > 15.0 {
            let end = if i + 1 < timestamps.len() {
                (timestamps[i + 1] + ts) / 2.0
            } else {
                ts + 0.3
            };
            segments.push(TranscriptSegment {
                start: offset + seg_start,
                end: offset + end,
                text: buf_text.trim().to_string(),
                confidence: 1.0,
            });
            buf_text.clear();
            if i + 1 < timestamps.len() {
                seg_start = timestamps[i + 1];
            }
        }
    }

    // Flush remaining
    if !buf_text.is_empty() {
        let last_ts = timestamps[timestamps.len() - 1];
        let dur = audio_duration_estimate(timestamps);
        let end = last_ts + dur.max(0.3);
        segments.push(TranscriptSegment {
            start: offset + seg_start,
            end: offset + end,
            text: buf_text.trim().to_string(),
            confidence: 1.0,
        });
    }

    segments
}

/// Remove garbled segments and merge consecutive short ones into the next.
fn merge_short_segments(segments: &mut Vec<TranscriptSegment>) {
    segments.retain(|s| {
        let text = s.text.trim();
        if text.is_empty() || text == "\u{FFFD}" {
            return false;
        }
        // Reject segments with rare/extended CJK (likely hallucination)
        text.chars().all(|ch| {
            ch.is_ascii()
                || ('\u{4E00}'..='\u{9FFF}').contains(&ch)
                || ('\u{3000}'..='\u{303F}').contains(&ch)
                || ('\u{FF00}'..='\u{FFEF}').contains(&ch)
        })
    });

    let mut i = 0;
    while i + 1 < segments.len() {
        let text = segments[i].text.trim();
        let dur = segments[i].end - segments[i].start;
        let char_count = text.chars().count();

        let too_short = (char_count == 1 && dur < 2.0)
            || (char_count < 3 && dur < 0.5);

        if too_short {
            let next = segments.remove(i + 1);
            let cur = &mut segments[i];
            cur.text = format!("{}{}", cur.text.trim(), next.text.trim());
            cur.end = next.end;
            continue;
        }
        i += 1;
    }
}

fn audio_duration_estimate(timestamps: &[f64]) -> f64 {
    if timestamps.len() < 2 {
        return 0.5;
    }
    let gaps: Vec<f64> = timestamps.windows(2).map(|w| w[1] - w[0]).collect();
    let median = {
        let mut sorted = gaps.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        sorted[sorted.len() / 2]
    };
    median
}

fn audio_duration(path: &Path) -> f64 {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return 0.0,
    };
    let len = meta.len();
    if len < 44 {
        return 0.0;
    }
    let bytes_per_sec = 16000_u64 * 2;
    len.saturating_sub(44) as f64 / bytes_per_sec as f64
}

pub fn ensure_ready(cfg: &AsrConfig) -> Result<()> {
    let bins = resolve_binaries();
    if bins.is_empty() {
        bail!(
            "sherpa-onnx-offline 未安装。请运行 `vtrim download-models` 自动下载到 {}",
            crate::download::vtrim_home().join("bin").display()
        );
    }

    let mdir = model_dir(cfg);
    let _model = find_model(&mdir)?;
    if !mdir.join("tokens.txt").exists() {
        bail!("模型目录缺少 tokens.txt: {}", mdir.display());
    }

    tracing::info!(
        "本地 ASR 就绪: binary ✓, 模型目录: {}",
        mdir.display()
    );

    Ok(())
}
