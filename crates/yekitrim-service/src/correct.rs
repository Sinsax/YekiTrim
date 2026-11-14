//! LLM 字幕订正 Service：CLI / WebUI 共用。

use std::path::{Path, PathBuf};

use yekitrim_core::config::Config;

use super::Workspace;

pub async fn run_correct(
    config_file: &Path,
    transcript_override: Option<PathBuf>,
    resume: bool,
    progress: Option<std::sync::Arc<dyn yekitrim_core::progress::ProgressReporter>>,
) -> anyhow::Result<()> {
    let cfg = yekitrim_core::config::load_config_at(config_file)?;
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
    let mut transcript_data: yekitrim_core::types::Transcript = if ext == "json" {
        serde_json::from_str(&content)?
    } else {
        yekitrim_core::srt::parse_srt(&content)?
    };

    let danmaku = load_danmaku_for_correct(&cfg, &ws);
    let (checkpoint, partial_path) = prepare_correction_state(&output, resume);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<yekitrim_llm::agent::AgentProgress<Vec<yekitrim_llm::agent::AgentOutput>>>(64);
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

    // 取消探针：订正是分钟级链路，必须让 Agent 在窗口边界轮询它
    //（判据与检测/精修同源：`ProgressReporter::cancelled()`）。
    let cancel: Option<yekitrim_llm::agent::CancelProbe> = progress.clone().map(|p| {
        std::sync::Arc::new(move || p.cancelled()) as yekitrim_llm::agent::CancelProbe
    });
    let result = yekitrim_core::asr::correct::correct_transcript_with_checkpoint(
        &transcript_data.segments,
        &danmaku,
        &cfg.llm,
        &cfg.correct,
        output.parent(),
        // ⚠️ **恒定传 Some**：断点写不写由这里决定，续不续由 `resume` 决定（见 prepare_correction_state）
        Some(checkpoint.as_path()),
        Some(tx),
        cancel,
    ).await;
    let _ = partial_task.await;

    match result {
        Ok(corrected) => {
            transcript_data.segments = corrected;
            let llm_srt = output.join("video_llm.srt");
            std::fs::create_dir_all(&output)?;
            yekitrim_core::srt::write_srt(&transcript_data, &llm_srt)?;
            // 与 `core_pipeline` 的订正落账同一口径：登记成普通清单条目（不占 ASR 槽，
            // 可删除），别让它只在 `output/` 里躺着 —— 列表里切不回去等于白订正。
            if crate::assets::bind_derived_material(&ws, crate::assets::AssetKind::Transcript, &llm_srt)
                .is_none()
            {
                let rel = rel_for_config(config_file, &llm_srt);
                yekitrim_core::config::save_transcript_path_in(config_file, &rel)?;
            }
            let _ = std::fs::remove_file(&checkpoint);
            let _ = std::fs::remove_file(&partial_path);
            Ok(())
        }
        Err(e) => anyhow::bail!("LLM 订正失败: {}", e),
    }
}

/// 订正用的弹幕：走 `Workspace::danmaku_path`（`danmaku_file` → 视频同目录 `danmu.ass`/`danmaku.ass`）
/// + `load_danmaku_auto`（XML / ASS / SRT 都认）。
///
/// ⚠️ 以前这里用 ASS-only 的 `danmaku::load_danmaku`：预处理页选的是录播姬 `danmaku.xml` 或 `.srt` 时
/// **吐 0 条且不报错**，订正 prompt 里的「弹幕高频词 / 当前时段弹幕」两段整段消失（静默降级）。
/// 与 `core_pipeline::load_danmaku_for_correct` 同一口径 —— 两条入口不许再各写一份。
fn load_danmaku_for_correct(cfg: &Config, ws: &Workspace) -> Vec<yekitrim_core::danmaku::DanmakuEntry> {
    let Some(path) = ws.danmaku_path(cfg) else {
        return Vec::new();
    };
    match yekitrim_core::danmaku::load_danmaku_auto(&path) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!("订正：弹幕解析失败（{}），本次不带弹幕上下文: {e:#}", path.display());
            Vec::new()
        }
    }
}

fn rel_for_config(config_file: &Path, path: &Path) -> String {
    if let Some(dir) = config_file.parent() {
        if let Ok(rel) = path.strip_prefix(dir) { return rel.to_string_lossy().into_owned(); }
    }
    path.to_string_lossy().into_owned()
}

/// 订正的断点 / 中间产物路径。`resume = false` 时**先删旧断点**（用户明确要求重跑）。
///
/// ⚠️ **返回的断点路径必须恒传给 `Agent`（`Some`）**，绝不能写成
/// `if checkpoint.exists() { Some(path) } else { None }` —— 那样首次运行传 `None`，
/// `Agent::run_windows` 就不写盘 ⇒ 文件永不出现 ⇒ 第二次仍然 `None`……**自锁**。
/// 后果是订正（分钟级链路，长片重跑很贵）的断点续传从未生效过。
/// 「要不要续传」只由 `resume` 决定，与「要不要写断点」无关。
pub(crate) fn prepare_correction_state(output: &Path, resume: bool) -> (PathBuf, PathBuf) {
    let checkpoint = output.join("correct.checkpoint.json");
    let partial = output.join("correct.partial.json");
    if !resume {
        let _ = std::fs::remove_file(&checkpoint);
    }
    (checkpoint, partial)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **回归钉子**：断点路径与「文件在不在」**无关**（旧实现就是在这一步返回 `None` 的，
    /// 于是 Agent 永远不写断点，续传形同虚设）。
    #[test]
    fn correction_checkpoint_path_is_reported_even_when_absent() {
        let tmp = tempfile::TempDir::new().expect("临时目录");
        let output = tmp.path().join("output");
        std::fs::create_dir_all(&output).unwrap();

        // 文件还不存在（首次运行）⇒ 仍必须给出路径
        let (checkpoint, partial) = prepare_correction_state(&output, true);
        assert!(!checkpoint.exists(), "前提：首次运行时断点还不存在");
        assert_eq!(checkpoint.file_name().unwrap(), "correct.checkpoint.json");
        assert_eq!(partial.file_name().unwrap(), "correct.partial.json");

        // resume = false ⇒ 旧断点被删（用户选了「重新开始」），路径照样给出
        std::fs::write(&checkpoint, "{}").unwrap();
        let (checkpoint2, _) = prepare_correction_state(&output, false);
        assert!(!checkpoint2.exists(), "resume=false 要先清掉旧断点");
        assert_eq!(checkpoint2, checkpoint);

        // resume = true ⇒ 旧断点留着（这就是续传的输入）
        std::fs::write(&checkpoint, "{}").unwrap();
        let (checkpoint3, _) = prepare_correction_state(&output, true);
        assert!(checkpoint3.exists(), "resume=true 不许动旧断点");
    }

    /// **回归钉子**：用户弹幕是录播姬 XML 时订正也必须读到（ASS-only 解析器对它吐 0 条且不报错）。
    #[test]
    fn load_danmaku_for_correct_reads_bililive_xml() {
        let tmp = tempfile::TempDir::new().expect("临时目录");
        let dir = tmp.path();
        let xml = dir.join("danmaku.xml");
        std::fs::write(
            &xml,
            r#"<?xml version="1.0" encoding="utf-8"?><i><d p="1.5,1,25,16777215,0,0,0,0">测试弹幕一</d></i>"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("video.mp4"),
            b"x",
        )
        .unwrap();
        // 工程配置：danmaku_file 相对工程根（`yekitrim init` 的写法）
        let cfg_path = dir.join("yekitrim.toml");
        std::fs::write(
            &cfg_path,
            "[project]\ninput_file = \"video.mp4\"\ndanmaku_file = \"danmaku.xml\"\n",
        )
        .unwrap();
        let cfg = yekitrim_core::config::load_config_at(&cfg_path).expect("读工程配置");
        let ws = Workspace::load(Some(&cfg_path)).expect("加载 Workspace");

        let entries = load_danmaku_for_correct(&cfg, &ws);
        assert_eq!(entries.len(), 1, "XML 弹幕必须解析出 1 条（ASS-only 会给 0）");
        assert_eq!(entries[0].text, "测试弹幕一");
    }
}
