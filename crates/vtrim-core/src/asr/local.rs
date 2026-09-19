use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use memmap2::Mmap;
use serde::Serialize;
use sherpa_onnx::{
    HomophoneReplacerConfig, OfflineQwen3ASRModelConfig, OfflineRecognizer,
    OfflineRecognizerConfig, OfflineSenseVoiceModelConfig,
};

use crate::asr::align::{self, CharTimeline, SplitPolicy};
use crate::config::AsrConfig;
use crate::progress::ProgressReporter;
use crate::types::{AsrMetadata, TranscriptSegment};

fn model_dir(cfg: &AsrConfig) -> PathBuf {
    model_dir_for(cfg, &cfg.local_model)
}

/// 模型目录是从哪一路找来的（界面要如实告诉用户"这份模型在哪"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelDirSource {
    /// 配置里的 `local_model_dir`
    Config,
    /// 用户目录 `~/.vtrim/models/<model>`
    UserHome,
    /// 程序目录 `<exe>/.vtrim/models/<model>`
    ExeDir,
}

impl ModelDirSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelDirSource::Config => "config",
            ModelDirSource::UserHome => "user",
            ModelDirSource::ExeDir => "exe",
        }
    }

    /// 中文标签（WebUI / 日志共用）
    pub fn label(self) -> &'static str {
        match self {
            ModelDirSource::Config => "配置目录",
            ModelDirSource::UserHome => "用户目录",
            ModelDirSource::ExeDir => "程序目录",
        }
    }
}

/// 指定模型名对应的模型目录**与来源**。
///
/// 查找优先级：
/// 1. 配置的 `local_model_dir`（直接目录或 models 根目录）
/// 2. 用户目录 `~/.vtrim/models/<model>`
/// 3. 程序目录 `.vtrim/models/<model>`
///
/// ⚠️ 这里是"模型在哪"的**唯一真源**：运行时（`model_dir_for`）与界面/诊断
/// （`download::model_install`）都走它 —— 曾出过"模型管理说没下载、实际转写一直在用"
/// 的错位，就是界面那边自己写了另一套路径（只看程序目录）。
pub fn resolve_model_dir(cfg: &AsrConfig, model: &str) -> (PathBuf, ModelDirSource) {
    let user_home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    resolve_model_dir_in(cfg, model, user_home.as_deref(), &crate::path::vtrim_home())
}

/// `resolve_model_dir` 的可注入版本（单测用：不碰环境变量，edition 2024 下 `set_var` 是 unsafe）。
pub fn resolve_model_dir_in(
    cfg: &AsrConfig,
    model: &str,
    user_home: Option<&Path>,
    exe_home: &Path,
) -> (PathBuf, ModelDirSource) {
    if !cfg.local_model_dir.is_empty() {
        let base = PathBuf::from(&cfg.local_model_dir);
        // 兼容两种写法：直接指向模型目录，或指向 models 根目录（自动拼接模型名）
        let direct = base.join(model);
        if base.join("model.int8.onnx").exists() || base.join("model.onnx").exists() {
            return (base, ModelDirSource::Config);
        }
        if direct.exists() {
            return (direct, ModelDirSource::Config);
        }
        return (base, ModelDirSource::Config);
    }

    // 用户目录优先（开发时模型常放这里）
    if let Some(home) = user_home {
        let user_dir = home.join(".vtrim").join("models").join(model);
        if user_dir.exists() {
            return (user_dir, ModelDirSource::UserHome);
        }
    }

    (exe_home.join("models").join(model), ModelDirSource::ExeDir)
}

fn model_dir_for(cfg: &AsrConfig, model: &str) -> PathBuf {
    resolve_model_dir(cfg, model).0
}

fn find_model(model_dir: &Path, model: &str) -> Result<PathBuf> {
    // 只认实际有推理入口的模型：single/hybrid 走 SenseVoice + qwen3-asr。
    // （firered-zh 已下线：CTC 时间戳在 BGM 段会造幽灵时间戳，见 ROADMAP）
    let candidates: &[&str] = match model {
        "qwen3-asr" => &["encoder.int8.onnx", "encoder.onnx"],
        _ => &["model.int8.onnx", "model.onnx", "sense-voice.onnx"],
    };
    for name in candidates {
        let p = model_dir.join(name);
        if p.exists() {
            return Ok(p);
        }
    }
    bail!(
        "模型目录 {} 中未找到模型文件 (需要: {})",
        model_dir.display(),
        candidates.join(" / ")
    );
}

fn require_file(dir: &Path, name: &str) -> Result<PathBuf> {
    let p = dir.join(name);
    if p.exists() {
        Ok(p)
    } else {
        bail!("模型目录 {} 缺少必需文件: {}", dir.display(), name)
    }
}

/// 同音纠错（`lexicon.txt` 是"字→拼音"表、`replace.fst` 是官方规则集）：
/// 两个文件都在就挂上去。**SenseVoice 与 qwen3 都要挂** ——
/// hybrid 下成片字幕的文本来自 qwen3，只给 SenseVoice 挂等于没效果。
fn apply_homophone_replacer(config: &mut OfflineRecognizerConfig, mdir: &Path, who: &str) {
    let lexicon = mdir.join("lexicon.txt");
    let rule_fst = mdir.join("replace.fst");
    if lexicon.exists() && rule_fst.exists() {
        config.hr = HomophoneReplacerConfig {
            lexicon: Some(lexicon.to_string_lossy().into()),
            rule_fsts: Some(rule_fst.to_string_lossy().into()),
        };
        tracing::info!("{who} 同音纠错已启用 (lexicon.txt + replace.fst)");
    } else {
        // 目录里没有 → 多半是"下载守卫"与运行时读的不是同一份（见 download.rs::ensure_one_model）
        tracing::info!(
            "{who} 同音纠错未启用（{} 下未找到 lexicon.txt / replace.fst）",
            mdir.display()
        );
    }
}

pub fn create_recognizer(cfg: &AsrConfig) -> Result<OfflineRecognizer> {
    let mdir = model_dir(cfg);

    let mut config = OfflineRecognizerConfig::default();
    config.model_config.num_threads = 1;

    let model_path = find_model(&mdir, "sensevoice")?;
    let tokens_path = require_file(&mdir, "tokens.txt")?;
    let language = if cfg.language.is_empty() {
        "auto".to_string()
    } else {
        cfg.language.clone()
    };
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(model_path.to_string_lossy().into()),
        language: Some(language),
        use_itn: true,
    };
    config.model_config.tokens = Some(tokens_path.to_string_lossy().into());
    tracing::info!("ASR 模型: SenseVoice (int8)");
    apply_homophone_replacer(&mut config, &mdir, "SenseVoice");

    OfflineRecognizer::create(&config)
        .context("创建 OfflineRecognizer 失败，请检查模型文件是否正确")
}

/// sensevoice 模型目录（同音纠错的 `lexicon.txt` / `replace.fst` 放在那里）。
/// qwen3 自己那份目录里没有这两个文件，所以要单独找一份。
fn sensevoice_dir(cfg: &AsrConfig) -> Option<PathBuf> {
    let dir = model_dir_for(cfg, "sensevoice");
    if dir.join("lexicon.txt").exists() && dir.join("replace.fst").exists() {
        Some(dir)
    } else {
        None
    }
}

/// qwen3 单流解码用几个线程：`min(4, 核数)`。
///
/// 上游 sherpa-onnx 对该 session 的默认是 1；本项目的用法是**串行**的
/// （`QWEN3_RECOGNIZER` 那个全局 Mutex），所以多给线程不会互相抢，
/// 而是让唯一在跑的那次解码快一些。
fn decode_threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get().min(4).max(1))
        .unwrap_or(1) as i32
}

/// 创建 qwen3-asr recognizer（用于双模型识别填词）。
pub fn create_qwen3_recognizer(cfg: &AsrConfig) -> Result<OfflineRecognizer> {
    let mdir = model_dir_for(cfg, "qwen3-asr");

    let mut config = OfflineRecognizerConfig::default();
    // 这个 session 是**串行**用的（全局 Mutex 保证同一时刻只有一次解码），
    // 所以多给线程是纯赚：单流解码是 matmul 密集型，1 线程等于把核闲着。
    // 实测 qwen3 阶段占整条转写的约 8 成时间（3 小时素材 12.4 分钟 / 1182 个窗口）。
    // 之后若改成多实例并行，这里要回到 1（否则 N 实例 × 4 线程会互相抢核）。
    config.model_config.num_threads = decode_threads();
    config.model_config.qwen3_asr = OfflineQwen3ASRModelConfig {
        conv_frontend: Some(mdir.join("conv_frontend.onnx").to_string_lossy().into()),
        encoder: Some(mdir.join("encoder.int8.onnx").to_string_lossy().into()),
        decoder: Some(mdir.join("decoder.int8.onnx").to_string_lossy().into()),
        tokenizer: Some(mdir.join("tokenizer").to_string_lossy().into()),
        max_total_len: 2048,
        max_new_tokens: cfg.max_new_tokens as i32,
        temperature: 1e-6,
        top_p: 0.8,
        seed: 42,
        hotwords: None,
    };
    config.model_config.model_type = Some("qwen3_asr".into());

    tracing::info!("ASR 模型: Qwen3-ASR (sherpa-onnx)");
    // hybrid 下成片字幕的文本来自这里，所以同音纠错必须挂在**这个** recognizer 上；
    // 只给 SenseVoice 挂等于只改了字轴（窗口边界），字幕正文一点不变。
    if let Some(sv_dir) = sensevoice_dir(cfg) {
        apply_homophone_replacer(&mut config, &sv_dir, "Qwen3");
    }
    OfflineRecognizer::create(&config).with_context(|| {
        // 报清楚"在哪找的"：用户目录/程序目录各有一份时，看日志就知道用的是哪一份
        format!(
            "创建 Qwen3-ASR recognizer 失败（模型目录: {}）——hybrid 需要 qwen3-asr，\
             在设置页「模型管理」里下载，或跑 vtrim download",
            mdir.display()
        )
    })
}

/// 对单个 WAV 文件用 qwen3-asr 识别文本。
pub fn transcribe_qwen3_file(recognizer: &OfflineRecognizer, wav_path: &Path) -> Result<String> {
    let mapped = MappedWave::open(wav_path)?;
    let stream = recognizer.create_stream();
    let samples = mapped.samples_f32(0, mapped.total_samples);
    stream.accept_waveform(mapped.sample_rate as i32, &samples);
    recognizer.decode(&stream);

    // 某些段（纯音乐/静音）可能没有 result，返回空字符串让上层跳过。
    let result = stream.get_result();
    Ok(result
        .map(|r| r.text.trim().to_string())
        .unwrap_or_default())
}

/// 转写结果 + 聚合后的 ASR 质量元数据。
pub struct TranscribeOutput {
    pub segments: Vec<TranscriptSegment>,
    pub metadata: AsrMetadata,
    /// 字符级时间轴（SenseVoice 是唯一时间权威；hybrid 用它给 qwen3 的文本定位切点）
    pub timeline: CharTimeline,
    /// 每块的原始识别结果（中间产物：断句调参的原料）
    pub chunks: Vec<RawChunk>,
}

/// 一块音频的原始识别输出。落盘到 `output/asr/raw/`，
/// 让"断句为什么长这样"可以离线复算，而不是只能重跑一遍听天由命。
#[derive(Debug, Clone, Serialize)]
pub struct RawChunk {
    pub index: usize,
    /// 该块在整片时间轴上的起点（秒）
    pub offset: f64,
    /// 该块被采纳的第一个 token 的绝对时刻（重叠区交给谁的分界证据）
    pub first_kept_time: Option<f64>,
    pub tokens: Vec<String>,
    pub timestamps: Vec<f64>,
    /// 重叠区过滤掉的 token 数（交给下一块）
    pub dropped: usize,
}

/// 内存映射的 WAV 文件读取器 — 避免将整个 PCM 数据转换为 f32 加载到内存。
struct MappedWave {
    mmap: Mmap,
    sample_rate: usize,
    data_start: usize,
    total_samples: usize,
}

impl MappedWave {
    fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).context("打开音频文件失败")?;
        let mmap = unsafe { Mmap::map(&file) }.context("内存映射音频文件失败")?;

        if mmap.len() < 44 {
            bail!("WAV 文件损坏或过小");
        }
        if &mmap[0..4] != b"RIFF" || &mmap[8..12] != b"WAVE" {
            bail!("文件不是有效的 WAV 格式");
        }

        let channels = u16::from_le_bytes([mmap[22], mmap[23]]) as usize;
        let sample_rate = u32::from_le_bytes([mmap[24], mmap[25], mmap[26], mmap[27]]) as usize;
        let bits_per_sample = u16::from_le_bytes([mmap[34], mmap[35]]) as usize;

        if bits_per_sample != 16 {
            bail!("仅支持 16-bit PCM WAV (当前: {} bit)", bits_per_sample);
        }
        if channels != 1 {
            bail!("仅支持单声道 WAV (当前: {} 声道)", channels);
        }

        let mut data_start = 12;
        let mut data_size = 0;
        while data_start + 8 <= mmap.len() {
            let chunk_id = &mmap[data_start..data_start + 4];
            let chunk_size = u32::from_le_bytes([
                mmap[data_start + 4],
                mmap[data_start + 5],
                mmap[data_start + 6],
                mmap[data_start + 7],
            ]) as usize;
            if chunk_id == b"data" {
                data_start += 8;
                data_size = chunk_size;
                break;
            }
            data_start += 8 + chunk_size;
            if !chunk_size.is_multiple_of(2) {
                data_start += 1;
            }
        }

        if data_size == 0 || data_start >= mmap.len() {
            bail!("WAV 文件中未找到音频数据");
        }

        let total_samples = data_size / 2;

        Ok(Self {
            mmap,
            sample_rate,
            data_start,
            total_samples,
        })
    }

    fn samples_f32(&self, start_sample: usize, count: usize) -> Vec<f32> {
        let byte_start = self.data_start + start_sample * 2;
        let byte_end = (byte_start + count * 2).min(self.mmap.len());
        self.mmap[byte_start..byte_end]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect()
    }
}

/// 从 WAV 中读取 `[start_sec, end_sec]` 的 f32 samples（内存操作，不写盘）。
pub fn read_wav_range(path: &Path, start_sec: f64, end_sec: f64) -> Result<(Vec<f32>, usize)> {
    let mapped = MappedWave::open(path)?;
    let start_sample = (start_sec * mapped.sample_rate as f64).max(0.0) as usize;
    let end_sample = ((end_sec - start_sec) * mapped.sample_rate as f64).max(0.0) as usize;
    let count = end_sample.min(mapped.total_samples.saturating_sub(start_sample));
    Ok((mapped.samples_f32(start_sample, count), mapped.sample_rate))
}

/// 用内存 samples 直接喂给 qwen3 recognizer，返回识别文本。
pub fn transcribe_qwen3_samples(
    recognizer: &OfflineRecognizer,
    samples: &[f32],
    sample_rate: usize,
) -> Result<String> {
    let stream = recognizer.create_stream();
    stream.accept_waveform(sample_rate as i32, samples);
    recognizer.decode(&stream);
    let result = stream.get_result();
    Ok(result.map(|r| r.text.trim().to_string()).unwrap_or_default())
}

/// 主流程：sensevoice 单模型并行转写。
pub fn transcribe_sensevoice(
    cfg: &AsrConfig,
    audio_path: &Path,
    progress: &dyn ProgressReporter,
) -> Result<TranscribeOutput> {
    let mapped = MappedWave::open(audio_path)?;
    let recognizer = create_recognizer(cfg)?;
    let policy = SplitPolicy::from_config(cfg);

    let (chunks, timeline) = scan_sensevoice_chunks(&mapped, &recognizer, cfg, progress)?;
    if timeline.is_empty() {
        bail!("SenseVoice 转写结果为空");
    }

    let segments = cleanup_segments(
        align::timeline_segments(&timeline, &policy)
            .into_iter()
            .map(|p| TranscriptSegment {
                start: p.start,
                end: p.end,
                text: p.text,
                confidence: 1.0,
            })
            .collect(),
    );

    let metadata = AsrMetadata {
        anchors: 0,
        anchor_coverage: 0.0,
        max_interp_span: 0,
        fallback_reason: Some("sensevoice_single".into()),
        model: "sensevoice".into(),
    };

    Ok(TranscribeOutput {
        segments,
        metadata,
        timeline,
        chunks,
    })
}

/// token 归哪一块：**半开区间** `[chunk_offset, claim_end)` 归本块。
///
/// `claim_end` = 下一块的起点（`offset + step`）；最后一块没有下一块，传 `f64::INFINITY`
/// （它解码到哪就收到哪）。写成显式区间是为了能单测"每一刻有且只有一个归属"——
/// "5 秒的字被相邻两块各留一次"这种错误，只在把分区算清楚时才看得出（旧口径正是这个坑）。
fn token_belongs_to_chunk(abs: f64, chunk_offset: f64, claim_end: f64) -> bool {
    abs >= chunk_offset && abs < claim_end
}

/// 用 sensevoice 并行扫描音频：返回**每块的原始 token** 与合并后的**字符级时间轴**。
///
/// 时间轴是断句的唯一时间权威 —— 逐字时间戳在这里就位，之后无论是 single 直接切，
/// 还是 hybrid 用 qwen3 的文本按标点切，切点时间都从这里取。
fn scan_sensevoice_chunks(
    mapped: &MappedWave,
    recognizer: &OfflineRecognizer,
    cfg: &AsrConfig,
    progress: &dyn ProgressReporter,
) -> Result<(Vec<RawChunk>, CharTimeline)> {
    let sample_rate = mapped.sample_rate;
    let total_samples = mapped.total_samples;

    let chunk_samples = (cfg.chunk_duration * sample_rate as f64) as usize;
    let overlap_samples = (cfg.chunk_overlap * sample_rate as f64) as usize;
    let step_samples = chunk_samples.saturating_sub(overlap_samples);

    if step_samples == 0 {
        bail!(
            "chunk_duration ({}) 必须大于 chunk_overlap ({})",
            cfg.chunk_duration,
            cfg.chunk_overlap
        );
    }

    let total_chunks = total_samples.div_ceil(step_samples);
    tracing::info!(
        "sensevoice 扫描: 音频分 {total_chunks} 块 (每块 {:.0}s, 重叠 {:.0}s, 步进 {:.1}s)",
        cfg.chunk_duration,
        cfg.chunk_overlap,
        step_samples as f64 / sample_rate as f64
    );

    let workers = if cfg.num_workers == 0 {
        std::thread::available_parallelism()
            .map(|n| (n.get() / 4).clamp(1, 4))
            .unwrap_or(1)
    } else {
        (cfg.num_workers as usize).min(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
        )
    }
    .min(total_chunks)
    .max(1);

    let mut all_chunks: Vec<RawChunk> = Vec::with_capacity(total_chunks);

    for batch_start in (0..total_chunks).step_by(workers) {
        // 取消在批次边界生效（进行中的单块识别不会被中断）
        if progress.cancelled() {
            bail!("任务已取消");
        }
        let batch_end = (batch_start + workers).min(total_chunks);
        let spawned: Vec<RawChunk> = std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(batch_end - batch_start);

            for chunk_idx in batch_start..batch_end {
                let sample_start = chunk_idx * step_samples;
                let sample_end = (sample_start + chunk_samples).min(total_samples);
                let chunk_offset = sample_start as f64 / sample_rate as f64;
                // 左余量：多喂 `segment_padding` 秒**已经被上一块认领过的**音频，
                // 只为了让本块开头的字有左上下文 —— 从块起点硬解码等于让识别器
                // 从半个词中间开始听，块首几个字容易丢/错。
                // ⚠️ 归属判定仍用 `chunk_offset`（不含余量），所以分区不变；
                // 但 token 时间戳是相对**解码起点**报的，绝对时间要用 `decode_offset` 还原。
                let pad_samples = (cfg.segment_padding.max(0.0) * sample_rate as f64) as usize;
                let decode_start = sample_start.saturating_sub(pad_samples);
                let decode_offset = decode_start as f64 / sample_rate as f64;
                // 重叠区归属：**按下一块的起点切**，每块只认自己那 50s（`step`）。
                //
                // ⚠️ 这里曾经是按「块中点之后交给下一块」（`offset + duration - overlap/2`，
                // 即 55s），于是 [50s, 55s) 这一截**两块都留下**；旧代码靠 `dedup_overlaps`
                // 在段一级去重兜住，而时间轴是逐字的，没有那层兜底 —— 5 秒的字会被写进去两次，
                // 进而切出两个覆盖同一段音频的窗口（qwen3 白跑一遍、字幕重复）。
                // 按下一块起点切是**无重叠的分区**：token ∈ [offset, offset+step) 归本块，
                // 它同时被下一块（从前 10s 预卷开始解码）正确识别，不会漏。
                // 最后一块没有"下一块起点"，它解码到哪就收到哪
                let step_secs = step_samples as f64 / sample_rate as f64;
                let is_last = chunk_idx == total_chunks - 1;
                let claim_end = if is_last {
                    f64::INFINITY
                } else {
                    chunk_offset + step_secs
                };

                // 从 `decode_start`（含左余量）解码，但只有 `[chunk_offset, claim_end)` 的字被采纳
                let chunk_f32 = mapped.samples_f32(decode_start, sample_end - decode_start);

                handles.push(scope.spawn(move || {
                    let mut out = RawChunk {
                        index: chunk_idx,
                        offset: chunk_offset,
                        first_kept_time: None,
                        tokens: Vec::new(),
                        timestamps: Vec::new(),
                        dropped: 0,
                    };
                    let stream = recognizer.create_stream();
                    stream.accept_waveform(sample_rate as i32, &chunk_f32);
                    recognizer.decode(&stream);

                    if let Some(result) = stream.get_result() {
                        if let Some(ts) = &result.timestamps {
                            for (tok, &t) in result.tokens.iter().zip(ts.iter()) {
                                // 时间戳相对**解码起点**（含左余量）→ 用 decode_offset 还原
                                let abs = decode_offset + t as f64;
                                if token_belongs_to_chunk(abs, chunk_offset, claim_end) {
                                    if out.first_kept_time.is_none() {
                                        out.first_kept_time = Some(abs);
                                    }
                                    out.tokens.push(tok.clone());
                                    out.timestamps.push(abs);
                                } else {
                                    out.dropped += 1;
                                }
                            }
                        }
                    }
                    out
                }));
            }

            // 子线程 panic 只丢这一块（并留下日志），整个转写不该跟着崩
            handles
                .into_iter()
                .map(|h| match h.join() {
                    Ok(chunk) => chunk,
                    Err(_) => {
                        tracing::error!("转写线程 panic，该块结果丢弃");
                        RawChunk {
                            index: 0,
                            offset: 0.0,
                            first_kept_time: None,
                            tokens: Vec::new(),
                            timestamps: Vec::new(),
                            dropped: 0,
                        }
                    }
                })
                .collect()
        });

        all_chunks.extend(spawned);
        progress.stage_progress(batch_end as u64, total_chunks as u64);
    }

    // 块是按 batch 推进的，结果顺序即块顺序；仍按 offset 排一次以防 workers>1 时的乱序
    all_chunks.sort_by(|a, b| a.offset.partial_cmp(&b.offset).unwrap());

    // 合并成全局字符级时间轴：块间重叠已被上面的中点过滤处理，这里再强制单调
    let mut timeline = CharTimeline::default();
    for chunk in &all_chunks {
        let part = CharTimeline::from_tokens(&chunk.tokens, &chunk.timestamps);
        timeline.chars.extend(part.chars);
        timeline.times.extend(part.times);
    }
    timeline.monotonic_fix();

    tracing::info!(
        "时间轴: {} 字（{} 块，气口 {} 处）",
        timeline.len(),
        all_chunks.len(),
        timeline.breath_points(cfg.breath_gap).len()
    );

    Ok((all_chunks, timeline))
}

/// 对最终字幕段做噪声清理。
///
/// ⚠️ **不再合并"短段"**：旧口径会把 `dur < 1.0`（或 `dur < 3.0 且 ≤5 字`）且间隙 < 1.0s
/// 的段并回邻居 —— 那正是把刚切好的短问句重新粘成一条的元凶
/// （实测"你们不用上班的吗？你们不用赚钱的吗？"就是这么回来的）。
/// 边界该不该留由断句器决定，这里只负责丢掉明显的噪声。
fn cleanup_segments(all_segments: Vec<TranscriptSegment>) -> Vec<TranscriptSegment> {
    let mut all_segments = all_segments;
    all_segments.retain(|s| s.text.trim() != "\u{FFFD}");
    if all_segments.is_empty() {
        return all_segments;
    }

    // 开头 300s 内的孤立短噪声（"嗯"、"啊"这类一拍子）——连续出现的不算噪声
    let start_noise = all_segments
        .iter()
        .take_while(|s| s.start < 300.0 && s.text.trim().chars().count() <= 2)
        .count();

    let mut cleaned: Vec<TranscriptSegment> = Vec::with_capacity(all_segments.len());
    for (idx, seg) in all_segments.iter().enumerate().skip(start_noise) {
        let text = seg.text.trim();
        let chars = text.chars().count();
        // 一个字却占了好几秒：多半是识别噪声
        if chars <= 1 && (seg.end - seg.start) > 5.0 {
            continue;
        }
        // 很短的段，且**前后都是大空档** → 孤立噪声
        if chars <= 2 && (seg.end - seg.start) < 5.0 {
            let gap_to_prev = if idx > 0 {
                seg.start - all_segments[idx - 1].end
            } else {
                f64::MAX
            };
            let gap_to_next = all_segments
                .get(idx + 1)
                .map(|n| n.start - seg.end)
                .unwrap_or(f64::MAX);
            if gap_to_prev > 3.0 && gap_to_next > 3.0 {
                continue;
            }
        }
        cleaned.push(seg.clone());
    }

    tracing::info!(
        "片段后处理: {} → {}{}",
        all_segments.len(),
        cleaned.len(),
        if cleaned.len() < all_segments.len() {
            format!(" (丢弃 {} 个噪声段)", all_segments.len() - cleaned.len())
        } else {
            String::new()
        },
    );

    cleaned
}

pub fn ensure_ready(cfg: &AsrConfig) -> Result<()> {
    let mdir = model_dir(cfg);
    find_model(&mdir, "sensevoice")?;
    require_file(&mdir, "tokens.txt")?;

    tracing::info!(
        "本地 ASR 就绪: model=sensevoice, 模型目录: {}",
        mdir.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 断句调参的原料：dump 一块音频的 **原始 sensevoice token**（token 原文 / 时间戳 / 字符间隔）。
    ///
    /// 2026-09 靠它得到两个决定性事实：**token 是单字的**（89/89），且 **SenseVoice 完全不吐标点**
    /// （全文无一个 `。！？`）—— 于是"句末标点强制切"这条规则在真实数据上永远没有信号，
    /// 唯一可用的边界信号是**气口**（问句之间 +0.42s，普通字间 +0.12~0.24s）。
    /// 调 `breath_gap` / `max_segment_chars` 之前先跑一遍看真实分布。
    ///
    /// 用法（cargo test 的 CWD 是 crate 根，路径写绝对路径）：
    /// `VT_ASR_WAV=<wav> VT_ASR_START=700 VT_ASR_SECS=60 \
    ///  cargo test -p vtrim-core --lib diagnose_sensevoice_tokens -- --ignored --nocapture`
    #[test]
    #[ignore = "需要真实模型与音频（VT_ASR_WAV）"]
    fn diagnose_sensevoice_tokens() {
        let wav = std::env::var("VT_ASR_WAV").expect("需要 VT_ASR_WAV=<wav 绝对路径>");
        let start: f64 = std::env::var("VT_ASR_START")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(700.0);
        let secs: f64 = std::env::var("VT_ASR_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60.0);

        let cfg = AsrConfig::default();
        ensure_ready(&cfg).unwrap();
        let recognizer = create_recognizer(&cfg).unwrap();
        let (samples, sample_rate) = read_wav_range(Path::new(&wav), start, start + secs).unwrap();

        let stream = recognizer.create_stream();
        stream.accept_waveform(sample_rate as i32, &samples);
        recognizer.decode(&stream);
        let result = stream.get_result().expect("该块没有识别结果");

        let tokens = &result.tokens;
        let ts: Vec<f64> = result
            .timestamps
            .as_ref()
            .expect("sensevoice 应带 token 时间戳")
            .iter()
            .map(|&t| t as f64)
            .collect();

        println!(
            "窗口 [{start:.1}s, {:.1}s]  token 数 = {}",
            start + secs,
            tokens.len()
        );
        println!("全文: {}", result.text);

        let multi: Vec<&String> = tokens.iter().filter(|t| t.chars().count() > 1).collect();
        println!(
            "\n多字符 token: {} / {}（{}%）",
            multi.len(),
            tokens.len(),
            if tokens.is_empty() {
                0
            } else {
                multi.len() * 100 / tokens.len()
            }
        );
        for t in multi.iter().take(30) {
            println!("  多字符 = {:?}", t);
        }

        println!("\n每个 token（index / token / 相对时间 / 与上一 token 的间隔）:");
        for (i, (tok, t)) in tokens.iter().zip(ts.iter()).enumerate() {
            let gap = if i == 0 { 0.0 } else { t - ts[i - 1] };
            let mark = if gap >= 0.25 { "  <== 气口" } else { "" };
            println!("  [{i:3}] {:>6.2}s (+{gap:.2}) {:?}{mark}", start + t, tok);
        }

        println!("\n标点落在 token 内部的样本（问号/句号/感叹号，不是 token 首字符）:");
        for (tok, t) in tokens.iter().zip(ts.iter()) {
            for (ord, ch) in tok.chars().enumerate() {
                if ord > 0 && matches!(ch, '。' | '！' | '？' | '.' | '!' | '?') {
                    println!(
                        "  {:>6.2}s {:?}  标点 {:?} 位于该 token 第 {} 个字符（首字符判断抓不到）",
                        start + t,
                        tok,
                        ch,
                        ord + 1
                    );
                    break;
                }
            }
        }

        // 新口径的产物：同一块音频现在被切成几条
        let mut timeline = CharTimeline::from_tokens(tokens, &ts);
        for t in timeline.times.iter_mut() {
            *t += start; // 上面只喂了这一段音频，时间戳是段内相对值
        }
        let pieces = align::timeline_segments(&timeline, &SplitPolicy::from_config(&cfg));
        println!("\n新口径 timeline_segments 切出 {} 条:", pieces.len());
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
    }

    /// 「模型在哪」的唯一真源：配置目录 → 用户目录 → 程序目录，用户目录优先于程序目录
    /// （界面曾只看程序目录，于是"模型管理说没下载、转写其实一直在用用户目录那份"）。
    #[test]
    fn resolve_model_dir_order_and_source() {
        use super::{resolve_model_dir_in, ModelDirSource};

        let base = std::env::temp_dir().join(format!(
            "vtrim-resolve-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let exe_home = base.join("exe").join(".vtrim");
        let user_home = base.join("user");
        // 程序目录里有一份 sensevoice
        std::fs::create_dir_all(exe_home.join("models").join("sensevoice")).unwrap();

        let mut cfg = AsrConfig::default();
        // ① 配置为空 + 用户目录没有 → 程序目录
        let (dir, src) = resolve_model_dir_in(&cfg, "sensevoice", Some(&user_home), &exe_home);
        assert_eq!(src, ModelDirSource::ExeDir);
        assert!(dir.ends_with(Path::new("models").join("sensevoice")));

        // ② 用户目录里有 → 用户目录优先（即便程序目录也有）
        let user_models = user_home.join(".vtrim").join("models");
        std::fs::create_dir_all(user_models.join("qwen3-asr")).unwrap();
        let (dir, src) = resolve_model_dir_in(&cfg, "qwen3-asr", Some(&user_home), &exe_home);
        assert_eq!(src, ModelDirSource::UserHome, "{}", dir.display());
        assert!(dir.starts_with(&user_models));

        // ③ 配了 local_model_dir（models 根目录写法）→ 配置目录，且不再看用户/程序目录
        let cfg_dir = base.join("cfg").join("models");
        std::fs::create_dir_all(cfg_dir.join("qwen3-asr")).unwrap();
        cfg.local_model_dir = cfg_dir.to_string_lossy().into_owned();
        let (dir, src) = resolve_model_dir_in(&cfg, "qwen3-asr", Some(&user_home), &exe_home);
        assert_eq!(src, ModelDirSource::Config);
        assert_eq!(dir, cfg_dir.join("qwen3-asr"));

        // ④ local_model_dir 直接指向模型目录本体（含模型文件）→ 原样返回
        let direct = base.join("direct");
        std::fs::create_dir_all(&direct).unwrap();
        std::fs::write(direct.join("model.int8.onnx"), b"x").unwrap();
        cfg.local_model_dir = direct.to_string_lossy().into_owned();
        let (dir, src) = resolve_model_dir_in(&cfg, "sensevoice", Some(&user_home), &exe_home);
        assert_eq!(src, ModelDirSource::Config);
        assert_eq!(dir, direct);

        let _ = std::fs::remove_dir_all(&base);
    }

    /// ⚠️ 分区不变量：每一秒的 token **有且只有一个**归属块。
    ///
    /// 旧口径（按块中点）会让 [50s, 55s) 同时属于两块 —— 时间轴里同一段字写两遍，
    /// 切出两个覆盖同一段音频的窗口，qwen3 白跑、字幕重复。
    #[test]
    fn chunk_partition_has_no_overlap_and_no_gap() {
        let step = 50.0f64;
        // 3 块：0-60 / 50-110 / 100-160（最后一块收到解码尽头）
        let chunks = [
            (0.0, 0.0 + step),
            (50.0, 50.0 + step),
            (100.0, f64::INFINITY),
        ];
        for t in 0..160 {
            let t = t as f64 + 0.5;
            let owners: Vec<usize> = chunks
                .iter()
                .enumerate()
                .filter(|(_, (off, end))| token_belongs_to_chunk(t, *off, *end))
                .map(|(i, _)| i)
                .collect();
            assert_eq!(
                owners.len(),
                1,
                "t={t} 的归属块数为 {}: {owners:?}",
                owners.len()
            );
        }
        // 前 50s 归第 0 块，不归第 1 块（旧口径把 50~55s 判给了两块）
        assert!(token_belongs_to_chunk(49.9, 0.0, step));
        assert!(
            !token_belongs_to_chunk(52.0, 0.0, step),
            "50~55s 不该再归第 0 块"
        );
        assert!(token_belongs_to_chunk(52.0, 50.0, 50.0 + step));
    }

    #[test]
    fn cleanup_removes_isolated_short_noise() {
        let segs = vec![
            TranscriptSegment {
                start: 0.0,
                end: 10.0,
                text: "嗯".into(),
                confidence: 1.0,
            },
            TranscriptSegment {
                start: 400.0,
                end: 402.0,
                text: "你好".into(),
                confidence: 1.0,
            },
        ];
        let cleaned = cleanup_segments(segs);
        assert!(cleaned.is_empty(), "孤立短噪声应被清理");
    }

    /// ⚠️ 回归钉子：短段**不许**被并回邻居。
    ///
    /// 旧口径会把 `dur < 1.0` 的段并进相邻段 —— 用户报的三个连续短问句
    /// （"你们不用上班的吗？你们不用赚钱的吗？你们不用上学的吗？"）就是这么粘成一条的。
    #[test]
    fn cleanup_keeps_short_questions_separate() {
        let segs = vec![
            TranscriptSegment {
                start: 744.9,
                end: 745.6,
                text: "你们不用上班的吗？".into(),
                confidence: 1.0,
            },
            TranscriptSegment {
                start: 746.7,
                end: 748.2,
                text: "你们不用赚钱的吗？".into(),
                confidence: 1.0,
            },
            TranscriptSegment {
                start: 748.4,
                end: 750.1,
                text: "你们不用上学的吗？".into(),
                confidence: 1.0,
            },
        ];
        let cleaned = cleanup_segments(segs);
        assert_eq!(cleaned.len(), 3, "短问句被合并了: {cleaned:#?}");
    }

    /// 长停顿要留在两条字幕**之间**，不能灌进前一条的尾巴
    /// （旧的 `cut_time` 语义，换成时间轴口径后同样要成立）。
    #[test]
    fn long_pause_stays_between_cues() {
        let chars: Vec<String> = "你好再见".chars().map(|c| c.to_string()).collect();
        let timeline = CharTimeline::from_tokens(&chars, &[0.0, 0.2, 20.2, 20.4]);
        let pieces = align::timeline_segments(&timeline, &SplitPolicy::default());
        assert_eq!(pieces.len(), 2, "{pieces:#?}");
        assert!(
            pieces[0].end - pieces[0].start < 1.5,
            "首条字幕吞下了停顿: {:?}",
            pieces[0]
        );
        assert!(pieces[1].start - pieces[0].end > 18.5, "停顿未留给字幕间隙");
    }

    /// 逐字时间戳 → 时间轴：token 内共享时间戳，块间重叠靠中点过滤，
    /// 合并后时间必须单调（否则下游按时间二分取字幕会错）。
    #[test]
    fn timeline_from_chunks_is_monotonic() {
        let c0 = RawChunk {
            index: 0,
            offset: 0.0,
            first_kept_time: Some(0.0),
            tokens: vec!["你".into(), "好".into()],
            timestamps: vec![0.0, 0.2],
            dropped: 0,
        };
        let c1 = RawChunk {
            index: 1,
            offset: 50.0,
            first_kept_time: Some(50.0),
            tokens: vec!["世".into(), "界".into()],
            timestamps: vec![50.0, 50.2],
            dropped: 0,
        };
        let mut timeline = CharTimeline::default();
        for c in [&c0, &c1] {
            let part = CharTimeline::from_tokens(&c.tokens, &c.timestamps);
            timeline.chars.extend(part.chars);
            timeline.times.extend(part.times);
        }
        timeline.monotonic_fix();
        assert_eq!(timeline.text(), "你好世界");
        for w in timeline.times.windows(2) {
            assert!(w[1] >= w[0], "时间轴不单调: {:?}", timeline.times);
        }
    }
}
