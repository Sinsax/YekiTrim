//! LLM 字幕订正 Service：CLI / WebUI 共用。

use std::path::{Path, PathBuf};

use vtrim_core::config::Config;

use super::Workspace;

pub async fn run_correct(
    config_file: &Path,
    transcript_override: Option<PathBuf>,
    resume: bool,
) -> anyhow::Result<()> {
    let cfg = vtrim_core::config::load_config_at(config_file)?;
    let ws = Workspace::load(Some(config_file))?;
    let output = ws.output_dir();

    let transcript_path = transcript_override
        .or_else(|| {
            cfg.project.transcript_file.as_ref().filter(|p| !p.is_empty()).map(|p| ws.resolve(p))
        })
        .or_else(|| {
            let j = output.join("transcript.json");
            if j.exists() { Some(j) } else { None }
        })
        .or_else(|| {
            let s = output.join("video.srt");
            if s.exists() { Some(s) } else { None }
        })
        .ok_or_else(|| anyhow::anyhow!("未找到转录文件"))?;

    let content = std::fs::read_to_string(&transcript_path)?;
    let ext = transcript_path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut transcript_data: vtrim_core::types::Transcript = if ext == "json" {
        serde_json::from_str(&content)?
    } else {
        vtrim_core::srt::parse_srt(&content)?
    };

    let danmaku = load_danmaku(&cfg, &ws);
    let checkpoint = output.join("correct.checkpoint.json");
    let partial_path = output.join("correct.partial.json");
    if !resume && checkpoint.exists() {
        let _ = std::fs::remove_file(&checkpoint);
    }
    let checkpoint_path = if checkpoint.exists() { Some(checkpoint.as_path()) } else { None };

    let (tx, mut rx) = tokio::sync::mpsc::channel::<vtrim_core::llm::agent::AgentProgress<Vec<vtrim_core::llm::agent::AgentOutput>>>(64);
    let partial_writer = partial_path.clone();
    let partial_task = tokio::spawn(async move {
        use std::collections::BTreeMap;
        let mut partial: BTreeMap<usize, String> = BTreeMap::new();
        while let Some(progress) = rx.recv().await {
            if let Some(outputs) = progress.output {
                for out in outputs { partial.insert(out.index, out.text); }
                let _ = std::fs::write(&partial_writer, serde_json::to_string_pretty(&partial).unwrap_or_default());
            }
        }
    });

    let result = vtrim_core::asr::correct::correct_transcript_with_checkpoint(
        &transcript_data.segments,
        &danmaku,
        &cfg.llm,
        &cfg.correct,
        output.parent(),
        checkpoint_path,
        Some(tx),
    ).await;
    let _ = partial_task.await;

    match result {
        Ok(corrected) => {
            transcript_data.segments = corrected;
            let llm_srt = output.join("video_llm.srt");
            std::fs::create_dir_all(&output)?;
            vtrim_core::srt::write_srt(&transcript_data, &llm_srt)?;
            // 与 `core_pipeline` 的订正落账同一口径：登记成普通清单条目（不占 ASR 槽，
            // 可删除），别让它只在 `output/` 里躺着 —— 列表里切不回去等于白订正。
            if crate::assets::bind_derived_material(&ws, crate::assets::AssetKind::Transcript, &llm_srt)
                .is_none()
            {
                let rel = rel_for_config(config_file, &llm_srt);
                vtrim_core::config::save_transcript_path_in(config_file, &rel)?;
            }
            let _ = std::fs::remove_file(&checkpoint);
            let _ = std::fs::remove_file(&partial_path);
            Ok(())
        }
        Err(e) => anyhow::bail!("LLM 订正失败: {}", e),
    }
}

fn load_danmaku(cfg: &Config, ws: &Workspace) -> Vec<vtrim_core::danmaku::DanmakuEntry> {
    if let Some(p) = cfg.project.danmaku_file.as_ref().filter(|s| !s.is_empty()) {
        let p = ws.resolve(p);
        if p.exists() { return vtrim_core::danmaku::load_danmaku(&p).unwrap_or_default(); }
    }
    Vec::new()
}

fn rel_for_config(config_file: &Path, path: &Path) -> String {
    if let Some(dir) = config_file.parent() {
        if let Ok(rel) = path.strip_prefix(dir) { return rel.to_string_lossy().into_owned(); }
    }
    path.to_string_lossy().into_owned()
}
