//! asr：Stage 2 转写链路（sherpa-onnx 本地识别 → 逐字时间轴 / 断句 → 文本后处理）。
//!
//! 入口：`Transcriber`；子模块 `align` / `artifacts` / `correct` / `local` / `text` / `vad`。
//! 边界：产物固定名 `output/video.srt` + `output/transcript.json` 被 `status.rs` / `current_stage()` 认死，不许改成按模式命名；登记生效值只有 `assets::bind_asr_artifact` 一条路 —— 见 `crates/vtrim-core/src/asr/AGENTS.md`

use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::pipeline::{Stage, STAGE_TRANSCRIBE};
use crate::srt::write_srt;
use crate::types::{AsrMetadata, PipelineContext, Transcript, TranscriptSegment};

pub mod align;
pub mod artifacts;
pub mod correct;
pub mod local;
pub mod text;
pub mod vad;

pub struct Transcriber;

/// 全局 qwen3 recognizer，用 Mutex 保证同一时间只有一个识别任务在跑。
/// 超时/异常后丢弃，下次调用会重建，避免状态污染。
static QWEN3_RECOGNIZER: OnceLock<Mutex<Option<sherpa_onnx::OfflineRecognizer>>> = OnceLock::new();

fn qwen3_recognizer_lock() -> &'static Mutex<Option<sherpa_onnx::OfflineRecognizer>> {
    QWEN3_RECOGNIZER.get_or_init(|| Mutex::new(None))
}

/// 在全局锁内执行一次 qwen3 识别。
///
/// - 从锁中 take 出 recognizer，避免并发访问
/// - 放到独立线程中执行，避免阻塞 async runtime
/// - 超时/异常时丢弃 recognizer（可能状态污染），下次重建
/// - 正常完成后放回 recognizer 复用
///
/// 用 detach 的普通线程而不是 `tokio::task::spawn_blocking`：超时只是不再等它，
/// 解码本身会跑到自然结束（退化的解码要几分钟）。留在 blocking 池里的这种遗留任务
/// 会让 runtime 在退出时等它——表现是最后一行日志打完之后进程还要空转几分钟
/// （实测 60 分钟素材挂 220 秒，期间稳定烧掉约 3 个核）。
/// 普通线程 detach 掉，主线程一返回进程就被终结，不拖时间。
async fn transcribe_segment_with_qwen3(
    cfg: &crate::config::AsrConfig,
    samples: Vec<f32>,
    sample_rate: usize,
    timeout: std::time::Duration,
) -> Result<String> {
    let mut guard = qwen3_recognizer_lock().lock().await;
    if guard.is_none() {
        *guard = Some(local::create_qwen3_recognizer(cfg)?);
    }

    let rec = guard.take().expect("recognizer 已初始化");
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let r = local::transcribe_qwen3_samples(&rec, &samples, sample_rate);
        let _ = tx.send((rec, r));
    });

    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok((rec, Ok(text)))) => {
            *guard = Some(rec);
            Ok(text)
        }
        Ok(Ok((_rec, Err(e)))) => {
            // recognizer 可能状态异常，不回收，下次重建
            tracing::warn!("qwen3 识别失败，丢弃 recognizer 待重建: {e:#}");
            Err(e)
        }
        Ok(Err(_)) => {
            // 发送端被丢弃：识别线程 panic 了
            tracing::warn!("qwen3 任务异常，丢弃 recognizer 待重建");
            Err(anyhow::anyhow!("qwen3 任务异常"))
        }
        Err(_) => {
            // 超时：rec 仍在后台线程中，无法回收，丢弃
            tracing::warn!("qwen3 推理超时({timeout:?})，丢弃 recognizer 待重建");
            Err(anyhow::anyhow!("qwen3 推理超时"))
        }
    }
}

#[async_trait]
impl Stage for Transcriber {
    fn name(&self) -> &'static str {
        "Transcriber"
    }

    fn stage_number(&self) -> u32 {
        STAGE_TRANSCRIBE
    }

    async fn run(&self, ctx: &mut PipelineContext) -> Result<()> {
        let audio_path = match &ctx.audio_path {
            Some(p) => p.clone(),
            None => {
                let fallback = ctx.output_dir.join("video.wav");
                let from_cfg = ctx
                    .config
                    .project
                    .audio_file
                    .as_ref()
                    .filter(|p| !p.is_empty())
                    .map(std::path::PathBuf::from);
                let path = from_cfg.filter(|p| p.exists()).or_else(|| {
                    if fallback.exists() {
                        Some(fallback.clone())
                    } else {
                        None
                    }
                });
                match path {
                    Some(p) => p,
                    None => anyhow::bail!(
                        "AudioExtractor 阶段未执行，缺少音频文件 (查找路径: {} 或 config.audio_file)",
                        fallback.display()
                    ),
                }
            }
        };

        let total_duration = wav_duration_secs(&audio_path)?;
        tracing::info!("音频时长: {:.1}s", total_duration);

        // ========== 判断模式 ==========
        let mode = ctx.config.asr.mode.trim().to_string();
        let policy = align::SplitPolicy::from_config(&ctx.config.asr);

        let mut all_segments: Vec<TranscriptSegment>;
        let metadata;
        // 中间产物：靠它们才能事后解释"断句为什么长这样"
        let mut windows: Vec<artifacts::WindowRecord> = Vec::new();
        let raw_segments: Vec<TranscriptSegment>;

        // ========== 打轴：两种模式共用同一遍 SenseVoice，同一条时间轴 ==========
        // SenseVoice 是唯一的时间权威（逐字时间戳 + 气口）；qwen3 没有时间戳，只提供文本。
        local::ensure_ready(&ctx.config.asr)?;
        let asr_out = local::transcribe_sensevoice(&ctx.config.asr, &audio_path, &*ctx.progress)?;
        let timeline = asr_out.timeline;
        if timeline.is_empty() {
            bail!("打轴未检测到语音");
        }

        if mode == "single" {
            // 单模型模式：SenseVoice 直接转写（文本与时间轴同源，断句只可能来自气口/字数上限）
            tracing::info!("单模型模式: SenseVoice 直接转写");
            let mut segs = asr_out.segments;
            segs.retain(|s| s.text.trim() != "\u{FFFD}");
            if segs.is_empty() {
                bail!("SenseVoice 转写结果为空");
            }
            raw_segments = segs.clone();
            // 单模型没有 qwen3 窗口，"切分决策"就是打轴本身 —— 记成一条窗口，
            // 免得 align.json 是空的、看着像没跑
            windows.push(artifacts::WindowRecord {
                window_index: 0,
                window_start: timeline.char_time(0),
                window_end: align::last_end(&timeline),
                text: timeline.text(),
                align_hit_rate: 1.0,
                resyncs: 0,
                timeline_chars: timeline.len(),
                duration_secs: align::last_end(&timeline) - timeline.char_time(0),
                text_chars: timeline.len(),
                pieces: segs
                    .iter()
                    .map(|s| artifacts::PieceRecord {
                        start: s.start,
                        end: s.end,
                        text: s.text.clone(),
                        boundary: None,
                    })
                    .collect(),
            });
            all_segments = segs;
            metadata = asr_out.metadata;
        } else {
            // 混合模式：SenseVoice 打轴 + qwen3 填词
            tracing::info!("混合模式: SenseVoice 打轴 + qwen3 填词");

            // 打轴给出的窗口（≤ max_segment_chars 字）就是喂 qwen3 的单位
            let speech_segments = asr_out.segments;
            tracing::info!("打轴检测到 {} 个语音段", speech_segments.len());

            // ========== 识别/填词 ==========
            const MAX_QWEN_SEGMENT_SECS: f64 = 60.0;
            let mut segs: Vec<TranscriptSegment> = Vec::new();
            let mut total_items = 0usize;
            // 解码余量（`[asr] segment_padding`，默认 0.3s）：这个旋钮以前没人读，
            // 现在就是在干它名字上的事 —— 给切给识别器的音频留白。
            let pad = ctx.config.asr.segment_padding.max(0.0);

            for (i, seg) in speech_segments.iter().enumerate() {
                // 取消在语音段边界生效
                if ctx.progress.cancelled() {
                    bail!("任务已取消");
                }
                let mut sub_start = seg.start;
                while sub_start < seg.end {
                    let sub_end = (sub_start + MAX_QWEN_SEGMENT_SECS).min(seg.end);
                    tracing::info!(
                        "识别语音段 {}/{} [{:.1}s - {:.1}s]",
                        i + 1,
                        speech_segments.len(),
                        sub_start,
                        sub_end
                    );
                    // 前后留一点余量再解码：从 token 时间戳处硬切会削掉首音，
                    // 而句首恰恰是识别最不稳的位置（且 qwen3 的文本就是成片字幕）。
                    // 前置余量只吃"停顿"，不碰上一个字的语音 —— 见 `decode_span`。
                    let (dec_start, dec_end) = decode_span(&timeline, sub_start, sub_end, pad);
                    let (samples, sample_rate) =
                        local::read_wav_range(&audio_path, dec_start, dec_end)?;

                    let seg_secs = (sub_end - sub_start).max(0.5);
                    let timeout_secs = (seg_secs * 3.0 + 5.0).clamp(10.0, 180.0);
                    let timeout = std::time::Duration::from_secs_f64(timeout_secs);

                    let text = match transcribe_segment_with_qwen3(
                        &ctx.config.asr,
                        samples,
                        sample_rate,
                        timeout,
                    )
                    .await
                    {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!(
                                "语音段 [{:.1}s - {:.1}s] 跳过: {e:#}",
                                sub_start,
                                sub_end
                            );
                            String::new()
                        }
                    };
                    // ⚠️ 不再 `trim_end_matches` 删尾标点：标点是断句信号，
                    // 删掉等于把"这条话说到哪"的证据扔了（旧口径的成片字幕结尾一个标点都没有）。
                    //
                    // 先截掉生成标记之后的胡说（`<|endoftext|>` 后面的百科文），
                    // **必须在切分前**：否则那串东西会自成一条字幕，还会漏进 clip.srt。
                    let text = text::strip_generation_markers(&text);
                    if !text.is_empty() {
                        // 关键一步：qwen3 给"在哪切"（它有标点），SenseVoice 的时间轴给
                        // "切在哪一刻"（它有气口）—— 逐窗口把两者合起来，而不是
                        // 一个窗口无脑 push 一条（那正是三个问句挤成一条的原因）。
                        let slice = timeline.slice_by_time(sub_start, sub_end);
                        let align_chars: Vec<char> = text.chars().collect();
                        let alignment = align::align_to_timeline(&align_chars, &slice);
                        let hit_rate = if alignment.map.is_empty() {
                            0.0
                        } else {
                            alignment.map.iter().filter(|m| m.is_some()).count() as f64
                                / alignment.map.len() as f64
                        };
                        let pieces = align::split_text_on_timeline(
                            &text,
                            &slice,
                            &policy,
                            (sub_start, sub_end),
                        );
                        if pieces.is_empty() {
                            // 一个切点都没有（或文本对不上）时退回整段一条，别丢内容
                            segs.push(TranscriptSegment {
                                start: sub_start,
                                end: sub_end,
                                text: text.clone(),
                                confidence: 1.0,
                            });
                            total_items += 1;
                        } else {
                            windows.push(artifacts::WindowRecord {
                                window_index: i,
                                window_start: sub_start,
                                window_end: sub_end,
                                text: text.clone(),
                                align_hit_rate: hit_rate,
                                resyncs: alignment.resyncs,
                                timeline_chars: slice.len(),
                                duration_secs: sub_end - sub_start,
                                text_chars: text.chars().count(),
                                pieces: pieces
                                    .iter()
                                    .map(|p| artifacts::PieceRecord {
                                        start: p.start,
                                        end: p.end,
                                        text: p.text.clone(),
                                        boundary: p.boundary,
                                    })
                                    .collect(),
                            });
                            for p in pieces {
                                if p.text.trim().is_empty() {
                                    continue;
                                }
                                segs.push(TranscriptSegment {
                                    start: p.start,
                                    end: p.end,
                                    text: p.text,
                                    confidence: 1.0,
                                });
                            }
                            total_items += 1;
                        }
                    }
                    sub_start = sub_end;
                }
                ctx.progress
                    .stage_progress((i + 1) as u64, speech_segments.len() as u64);
            }

            if segs.is_empty() {
                bail!("qwen3 填词结果为空");
            }
            raw_segments = segs.clone();
            // 相邻窗口的重叠区可能都报了同一句：按时间排好后丢掉完全被后一条包含的碎块
            segs.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
            segs = drop_contained(segs);
            all_segments = segs;
            metadata = AsrMetadata {
                anchors: total_items,
                anchor_coverage: 1.0,
                max_interp_span: 0,
                fallback_reason: None,
                model: "qwen3-asr".into(),
                mode: mode.clone(),
            };
        }

        // 中间产物：断句的每一步都留痕，方便离线复算与调参
        if let Err(e) = artifacts::reset(&ctx.output_dir) {
            tracing::warn!("清理旧的 ASR 中间产物失败: {e:#}");
        }
        artifacts::write_all(
            &ctx.output_dir,
            &asr_out.chunks,
            &timeline,
            ctx.config.asr.breath_gap,
            &windows,
            &raw_segments,
        );

        // 字幕文本后处理：只改外观（行尾中性标点、转述引号），不动断句与时间。
        // 放在**中间产物之后**是有意的 —— `output/asr/` 记录的是"切分时看到的文本"，
        // 保持原文，别让它跟落盘字幕对不上（见 `artifacts` 模块注释）。
        let mut normalized = 0usize;
        for seg in all_segments.iter_mut() {
            let before = seg.text.clone();
            seg.text = text::normalize(&seg.text);
            if seg.text != before {
                normalized += 1;
            }
        }
        if normalized > 0 {
            tracing::info!("字幕文本规范: {normalized}/{} 条被改写", all_segments.len());
        }

        tracing::info!("转写完成: {} 个片段", all_segments.len());
        let transcript = Transcript {
            segments: all_segments,
            language: "zh".into(),
            asr_metadata: Some(metadata),
        };

        let srt_path = ctx.output_dir.join("video.srt");
        write_srt(&transcript, &srt_path)?;
        let json_path = ctx.output_dir.join("transcript.json");
        save_transcript_json(&transcript, &json_path)?;
        tracing::info!("字幕已生成: {}", srt_path.display());
        tracing::info!("转录 JSON 已保存: {}", json_path.display());

        // 分模式留档：`output/transcript.<mode>.json`。
        //
        // 预处理页的素材清单里，**每个 ASR 模式下最多一条产物**（hybrid / single 各一条，
        // 也就是用户说的"输出一次绑定一次、只能两次"）。`output/` 下那两份是"最新一次"的
        // 覆盖式产物，两条清单条目指着同一份文件就没法切换了，所以每个模式各留一份。
        // 同一模式重跑直接覆盖自己那一份，不新增条目。
        //
        // ⚠️ **别把 `video.srt` / `transcript.json` 改成按模式命名**：`status.rs` 的
        // 「字幕」判定、`ProjectConfig::current_stage` 的 stage 2 推导、老 CLI 都认这两个固定名。
        if let Some(slug) = mode_slug(&mode) {
            let mode_path = ctx.output_dir.join(format!("transcript.{slug}.json"));
            save_transcript_json(&transcript, &mode_path)?;
            tracing::info!("分模式转录 JSON 已保存: {}", mode_path.display());
        }

        ctx.transcript = Some(transcript);

        tracing::info!("语音转文字完成");
        Ok(())
    }
}

/// ASR 模式 → 文件名 slug（`hybrid` / `single`）。不认识的模式返回 `None`：
/// 模式串来自配置，手改成乱七八糟的值时**别把它拼进文件名**。
fn mode_slug(raw: &str) -> Option<String> {
    let s = raw.trim().to_ascii_lowercase();
    if s.is_empty() || s.len() > 32 {
        return None;
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(s)
}

/// 给识别器切音频时的实际解码区间：窗口 `[start, end)` 前后各留 `pad` 余量。
///
/// 为什么需要余量：窗口边界来自 SenseVoice 的**逐字时间戳**，而识别器从那一帧开始听
/// 等于削掉首音（句首本就是最不稳的位置），下一句的开头几个字因此容易丢。
///
/// ⚠️ 前置余量**只许吃停顿**：下限取"上一个字的起始 + 单字音频长度上界（0.35s）"，
/// 于是一旦上一个字离得近（说明这里是按字数上限硬切的、中间没有停顿），余量自动归零 ——
/// 否则会把上一条的尾巴喂进去，qwen3 复述一遍，首条就多出重复内容。
fn decode_span(timeline: &align::CharTimeline, start: f64, end: f64, pad: f64) -> (f64, f64) {
    /// 单个字的音频长度上界（超出这个量就认为已经进入停顿）。
    const CHAR_AUDIO_MAX: f64 = 0.35;
    if pad <= 0.0 || timeline.is_empty() {
        return (start, end);
    }
    // 时间轴上"上一个字"（严格早于窗口起点的最后一个字）
    let prev_char = timeline
        .times
        .partition_point(|&t| t < start)
        .checked_sub(1)
        .map(|i| timeline.times[i]);
    let lower = match prev_char {
        Some(t) => (t + CHAR_AUDIO_MAX).max(timeline.times[0]),
        None => timeline.times[0].min(start - pad),
    };
    // 后置余量同理：不许越过下一个字的起点（那属于下一条）
    let next_char = timeline
        .times
        .get(timeline.times.partition_point(|&t| t < end));
    let upper = match next_char {
        Some(&t) => (end + pad).min(t),
        None => end + pad,
    };
    // 别越过整片音频的两端
    let dec_start = (start - pad).max(lower).max(0.0).min(start);
    let dec_end = (end + pad).min(upper).max(dec_start);
    (dec_start, dec_end)
}

/// 相邻窗口的重叠区可能都报了同一句：丢掉**完全被前一条包含**的碎块
/// （时间相接但各自独立的短句不在此列 —— 那正是要保留的断句结果）。
fn drop_contained(segments: Vec<TranscriptSegment>) -> Vec<TranscriptSegment> {
    let mut kept: Vec<TranscriptSegment> = Vec::with_capacity(segments.len());
    for seg in segments {
        if let Some(last) = kept.last() {
            if seg.start >= last.start && seg.end <= last.end {
                if seg.text.len() <= last.text.len() {
                    continue;
                }
                kept.pop();
            }
        }
        kept.push(seg);
    }
    kept
}

pub(super) fn wav_duration_secs(path: &Path) -> Result<f64> {
    let output = Command::new("ffprobe")
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(path)
        .output()
        .with_context(|| "ffprobe 获取音频时长失败")?;
    let s = String::from_utf8_lossy(&output.stdout);
    s.trim()
        .parse::<f64>()
        .map_err(|e| anyhow::anyhow!("无法解析音频时长 '{}': {}", s.trim(), e))
}

fn save_transcript_json(transcript: &Transcript, path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(transcript)?;
    std::fs::write(path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AsrConfig;

    fn tl_with(text: &str, times: &[f64]) -> align::CharTimeline {
        let chars: Vec<String> = text.chars().map(|c| c.to_string()).collect();
        align::CharTimeline::from_tokens(&chars, times)
    }

    /// 解码余量只许吃停顿：上一个字离得近（说明是硬切、中间没停顿）时**不许**往前伸 ——
    /// 否则会把上一条的尾巴喂进去，qwen3 复述一遍，首条就多出重复内容。
    #[test]
    fn decode_span_pre_roll_only_uses_pauses() {
        // ① 前面是长停顿 → 可以往前吃满余量
        let spaced = tl_with("你好再见", &[0.0, 0.2, 20.4, 20.6]);
        let (start, end) = decode_span(&spaced, 20.4, 20.6, 0.3);
        assert!((start - 20.1).abs() < 1e-9, "应吃到 20.4-0.3，实际 {start}");
        assert!(end >= 20.6, "末尾没有字了，应加余量: {end}");

        // ② 上一个字紧邻（0.2s 处）→ 前置余量必须归零，不能吃进它的语音
        let tight = tl_with("你好世界", &[0.0, 0.2, 0.4, 0.6]);
        let (start, _) = decode_span(&tight, 0.4, 0.6, 0.3);
        assert!(
            (start - 0.4).abs() < 1e-9,
            "前置余量吃进了上一个字的语音（0.2s 那个字）: {start}"
        );
    }

    /// 后置余量不许越过下一个字的起点（那属于下一条）。
    #[test]
    fn decode_span_tail_stops_at_next_char() {
        let timeline = tl_with("你好世界", &[0.0, 0.2, 0.4, 0.6]);
        // 窗口 [0.2, 0.4) 的下一个字在 0.4 → 后置余量被挡住
        let (_, end) = decode_span(&timeline, 0.2, 0.4, 0.3);
        assert!((end - 0.4).abs() < 1e-9, "后置余量越界了: {end}");
    }

    /// pad=0 时必须是原样（设 0 = 回到旧行为，一个采样都不多喂）。
    #[test]
    fn decode_span_disabled_is_identity() {
        let timeline = tl_with("你好", &[0.0, 0.2]);
        assert_eq!(decode_span(&timeline, 1.0, 2.0, 0.0), (1.0, 2.0));
    }

    /// 空时间轴不许 panic（退化输入）。
    #[test]
    fn decode_span_handles_empty_timeline() {
        let empty = align::CharTimeline::default();
        assert_eq!(decode_span(&empty, 1.0, 2.0, 0.3), (1.0, 2.0));
    }

    /// **A/B**：同一批窗口，`segment_padding = 0` vs `0.3` 的识别文本对比。
    ///
    /// 要回答的问题：句首被削掉的那几个字，加了前置余量之后是否真的回来了？
    /// 判据：①有多少窗口文本发生变化 ②变化里有多少是"pad 让句首更完整"（pad 版更长/多出字）
    /// ③有没有"pad 版多出上一条的尾巴"（那说明余量吃进了语音，是我们要避免的）。
    ///
    /// `VT_ASR_WAV=<wav> [VT_ASR_START=730] [VT_ASR_SECS=30] cargo test -p vtrim-core --lib diagnose_head_padding -- --ignored --nocapture`
    #[test]
    #[ignore = "需要真实模型与音频（VT_ASR_WAV）"]
    fn diagnose_head_padding() {
        let wav = std::env::var("VT_ASR_WAV").expect("需要 VT_ASR_WAV=<wav 绝对路径>");
        let wav = Path::new(&wav);
        let start: f64 = std::env::var("VT_ASR_START")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(730.0);
        let secs: f64 = std::env::var("VT_ASR_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30.0);
        let pad = 0.3;

        let cfg = AsrConfig::default();
        let policy = align::SplitPolicy::from_config(&cfg);

        // ① SenseVoice 时间轴（时间权威）
        let recognizer = local::create_recognizer(&cfg).unwrap();
        let (samples, sr) = local::read_wav_range(wav, start, start + secs).unwrap();
        let stream = recognizer.create_stream();
        stream.accept_waveform(sr as i32, &samples);
        recognizer.decode(&stream);
        let res = stream.get_result().expect("该窗口没有识别结果");
        let ts: Vec<f64> = res
            .timestamps
            .as_ref()
            .expect("sensevoice 应带时间戳")
            .iter()
            .map(|&t| t as f64 + start)
            .collect();
        let timeline = align::CharTimeline::from_tokens(&res.tokens, &ts);

        // ② 按真实管线切窗口
        let windows = align::timeline_segments(&timeline, &policy);
        println!(
            "SenseVoice 时间轴 {} 字 → {} 个窗口\n",
            timeline.len(),
            windows.len()
        );

        let qwen3 = local::create_qwen3_recognizer(&cfg).unwrap();
        let run = |s: f64, e: f64| -> String {
            let (samples, sr) = local::read_wav_range(wav, s, e).unwrap();
            local::transcribe_qwen3_samples(&qwen3, &samples, sr).unwrap_or_default()
        };
        // 对齐命中率：qwen3 的字有多少在 SenseVoice 时间轴上找得到位置。
        // 句首被削/听漏时这份文本与字轴对不上，命中率会低 —— 用它当客观指标。
        let hit = |txt: &str, s: f64, e: f64| -> f64 {
            let slice = timeline.slice_by_time(s, e);
            let chars: Vec<char> = txt.chars().collect();
            if chars.is_empty() || slice.is_empty() {
                return 0.0;
            }
            let a = align::align_to_timeline(&chars, &slice);
            a.map.iter().filter(|m| m.is_some()).count() as f64 / a.map.len() as f64
        };

        let (mut changed, mut head_longer, mut spill) = (0usize, 0usize, 0usize);
        let (mut hit_a, mut hit_b, mut n) = (0.0f64, 0.0f64, 0usize);
        let (mut min_ratio_a, mut min_ratio_b) = (f64::MAX, f64::MAX);
        for (i, w) in windows.iter().enumerate() {
            let plain = run(w.start, w.end);
            let (ds, de) = decode_span(&timeline, w.start, w.end, pad);
            let padded = if (ds - w.start).abs() < 1e-6 && (de - w.end).abs() < 1e-6 {
                plain.clone()
            } else {
                run(ds, de)
            };
            let a = text::strip_generation_markers(&plain);
            let b = text::strip_generation_markers(&padded);
            // 该窗口的时间轴字数（"这段大概有几个字"的参考）
            let tl_chars = timeline.slice_by_time(w.start, w.end).len().max(1);
            let ra = a.chars().count() as f64 / tl_chars as f64;
            let rb = b.chars().count() as f64 / tl_chars as f64;
            min_ratio_a = min_ratio_a.min(ra);
            min_ratio_b = min_ratio_b.min(rb);
            hit_a += hit(&a, w.start, w.end);
            hit_b += hit(&b, w.start, w.end);
            n += 1;
            if a == b {
                continue;
            }
            changed += 1;
            // "更完整"：pad 版更长且句首一致（近似判断句首补全）
            let a_head: String = a.chars().take(4).collect();
            let b_head: String = b.chars().take(4).collect();
            if b.chars().count() > a.chars().count() && (a.is_empty() || b_head == a_head) {
                head_longer += 1;
            } else if !a.is_empty() && b.contains(&a) {
                spill += 1; // pad 版把原文整个包住了 → 多出来的部分可能是上一条的尾巴
            }
            println!(
                "窗口 {i} [{:.2}-{:.2}] tl={tl_chars}字 命中 {:.2}→{:.2}\n  无余量: {a:?}\n  有余量: {b:?}",
                w.start, w.end, hit(&a, w.start, w.end), hit(&b, w.start, w.end)
            );
        }
        if n > 0 {
            println!(
                "\n共 {n} 个窗口：文本变化 {changed}（pad 版更长 {head_longer}、疑似带出上一条 {spill}）"
            );
            println!(
                "平均对齐命中率: 无余量 {:.3} → 有余量 {:.3}",
                hit_a / n as f64,
                hit_b / n as f64
            );
            println!("最差字数比(文本/字轴): 无余量 {min_ratio_a:.2} → 有余量 {min_ratio_b:.2}");
        }
    }

    /// **对照用**：把一批窗口的 SenseVoice 字轴与 qwen3 文本逐条打出来（TSV），
    /// 用于"开/关同音纠错"两次运行做 diff。
    ///
    /// 为什么需要它：HR（`lexicon.txt` + `replace.fst`）以前因为目录错位从未生效，
    /// 现在同时对 SenseVoice 与 qwen3 生效 —— 但它到底改了多少、改好还是改坏，必须实测。
    /// 判据参照检查 `segment_padding` 时那套：对齐命中率当客观参照。
    ///
    /// `VT_ASR_WAV=<wav> [VT_ASR_START=700] [VT_ASR_SECS=180] cargo test -p vtrim-core --lib diagnose_hr_effect -- --ignored --nocapture`
    #[test]
    #[ignore = "需要真实模型与音频（VT_ASR_WAV）"]
    fn diagnose_hr_effect() {
        let wav = std::env::var("VT_ASR_WAV").expect("需要 VT_ASR_WAV=<wav 绝对路径>");
        let wav = Path::new(&wav);
        let start: f64 = std::env::var("VT_ASR_START")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(700.0);
        let secs: f64 = std::env::var("VT_ASR_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(180.0);

        let cfg = AsrConfig::default();
        let policy = align::SplitPolicy::from_config(&cfg);

        let recognizer = local::create_recognizer(&cfg).unwrap();
        let (samples, sr) = local::read_wav_range(wav, start, start + secs).unwrap();
        let stream = recognizer.create_stream();
        stream.accept_waveform(sr as i32, &samples);
        recognizer.decode(&stream);
        let res = stream.get_result().expect("该区间没有识别结果");
        let ts: Vec<f64> = res
            .timestamps
            .as_ref()
            .expect("sensevoice 应带时间戳")
            .iter()
            .map(|&t| t as f64 + start)
            .collect();
        let timeline = align::CharTimeline::from_tokens(&res.tokens, &ts);
        // 机器可读：`SVT<TAB>全文`（看 HR 对字轴的影响）
        println!("SVT\t{}", timeline.text());

        let qwen3 = local::create_qwen3_recognizer(&cfg).unwrap();
        for w in align::timeline_segments(&timeline, &policy) {
            let (s, sr) = local::read_wav_range(wav, w.start, w.end).unwrap();
            let t = local::transcribe_qwen3_samples(&qwen3, &s, sr).unwrap_or_default();
            // 机器可读：窗口号 / 起点 / 字轴字数 / qwen3 文本
            println!(
                "WIN\t{:.3}\t{}\t{}",
                w.start,
                timeline.slice_by_time(w.start, w.end).len(),
                t.trim()
            );
        }
    }

    /// **用户报的那条的回归钉子**：三个连续问句必须切成三条，且首条起点 = 窗口起点。
    ///
    /// 现场（`media/[2026-08-13 20-54-35-962][好困]`）：744.96~751.20s 原本是**一条** 6.24 秒
    /// 的字幕 `你们不用上班的吗？你们不用赚钱的吗？你们不用上学的吗`。
    /// 原因不是"标点判断写错了"，而是 ①SenseVoice 完全不吐标点（`。！？` 一个都没有），
    /// ②气口（+0.42s）没被当作边界信号（`segment_gap` = 1.0s 直接跳过），
    /// ③hybrid 一个窗口无脑 push 一条。三条现在都改了 —— 这条测试把结果钉住。
    ///
    /// 跑真实双模型，不做任何写盘：
    /// `VT_ASR_WAV=<wav 绝对路径> cargo test -p vtrim-core --lib diagnose_hybrid_three_questions -- --ignored --nocapture`
    #[test]
    #[ignore = "需要真实模型与音频（VT_ASR_WAV）"]
    fn diagnose_hybrid_three_questions() {
        let wav = std::env::var("VT_ASR_WAV").expect("需要 VT_ASR_WAV=<wav 绝对路径>");
        let wav = Path::new(&wav);
        // 包住整段（前面留一点上下文，免得气口被窗口边缘切掉）
        let (start, secs) = (742.0, 11.0);

        let cfg = AsrConfig::default();
        let policy = align::SplitPolicy::from_config(&cfg);

        // ① SenseVoice：逐字时间轴（时间权威）
        let recognizer = local::create_recognizer(&cfg).unwrap();
        let (samples, sr) = local::read_wav_range(wav, start, start + secs).unwrap();
        let stream = recognizer.create_stream();
        stream.accept_waveform(sr as i32, &samples);
        recognizer.decode(&stream);
        let res = stream.get_result().expect("该窗口没有识别结果");
        let ts: Vec<f64> = res
            .timestamps
            .as_ref()
            .expect("sensevoice 应带时间戳")
            .iter()
            .map(|&t| t as f64 + start)
            .collect();
        let timeline = align::CharTimeline::from_tokens(&res.tokens, &ts);
        println!("SenseVoice 文本（注意：没有标点）: {}", timeline.text());

        // ② 用**真实管线**的切窗口走法取窗口（不是随手挑一个区间）——
        // 窗口起点就是这条字幕该出现的时刻，测试要贴着它以覆盖"句首起点"这件事。
        let windows = align::timeline_segments(&timeline, &policy);
        let target = windows
            .iter()
            .find(|w| w.start <= 745.0 && 745.0 < w.end)
            .or_else(|| windows.get(1))
            .expect("应至少有一个窗口");
        println!(
            "目标窗口: [{:.2} - {:.2}]  {}",
            target.start, target.end, target.text
        );
        let span = (target.start, target.end);
        let slice = timeline.slice_by_time(span.0, span.1);

        // ③ qwen3：填词（有标点、没有时间戳）——喂的就是这个窗口的音频
        let qwen3 = local::create_qwen3_recognizer(&cfg).unwrap();
        let (win_samples, win_sr) = local::read_wav_range(wav, span.0, span.1).unwrap();
        let text = local::transcribe_qwen3_samples(&qwen3, &win_samples, win_sr).unwrap();
        println!("qwen3 文本（有标点）: {text}");

        // ④ 两者合起来断句
        let pieces = align::split_text_on_timeline(&text, &slice, &policy, span);
        println!("\n切成 {} 条:", pieces.len());
        for p in &pieces {
            println!(
                "  {:.2}~{:.2}s ({:.2}s) [{}]  {}",
                p.start,
                p.end,
                p.end - p.start,
                p.boundary
                    .map(|b| format!("{b:?}"))
                    .unwrap_or_else(|| "tail".into()),
                p.text
            );
        }

        assert!(
            pieces.len() >= 3,
            "三个问句应至少切成三条（旧口径这里是 1 条）: {pieces:#?}"
        );
        let joined: String = pieces.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(joined.matches('？').count(), 3, "三个问号应都在: {joined}");
        // 首条起点 = 窗口起点（A1 的钉子）：起点晚于窗口起点就是"看起来丢了句首"
        assert!(
            (pieces[0].start - span.0).abs() < 1e-9,
            "首条应从窗口起点 {:.2} 开始，实际 {:.2}",
            span.0,
            pieces[0].start
        );
        // 时间必须递增且不重叠
        for w in pieces.windows(2) {
            assert!(w[0].end <= w[1].start, "重叠: {:?} / {:?}", w[0], w[1]);
        }
    }
}
