//! asr::local：sherpa-onnx 识别器构建与分块解码（SenseVoice 打轴 / qwen3-asr 填词）。
//!
//! 入口：`create_recognizer`、`create_qwen3_recognizer`、`transcribe_sensevoice`、`transcribe_qwen3_samples`、`resolve_model_dir`、`read_wav_range`、`ensure_ready`。
//! 边界：模型选择与解码线程数（`decode_threads`）只在这里；sherpa-onnx 一律 CPU only，改并行方式要连带看 qwen3 全局锁 —— 见 `crates/vtrim-core/src/asr/AGENTS.md`

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use memmap2::Mmap;
use serde::Serialize;
use sherpa_onnx::{
    HomophoneReplacerConfig, OfflineQwen3ASRModelConfig, OfflineRecognizer,
    OfflineRecognizerConfig, OfflineSenseVoiceModelConfig,
};

use crate::asr::align::{self, CharTimeline, SplitPolicy};
use crate::asr::vad;
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
    /// 程序目录 `<数据目录>/.vtrim/models/<model>`
    ExeDir,
}

impl ModelDirSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelDirSource::Config => "config",
            ModelDirSource::ExeDir => "exe",
        }
    }

    /// 中文标签（WebUI / 日志共用）
    pub fn label(self) -> &'static str {
        match self {
            ModelDirSource::Config => "配置目录",
            ModelDirSource::ExeDir => "程序目录",
        }
    }
}

/// 指定模型名对应的模型目录**与来源**。
///
/// 查找优先级：
/// 1. 配置的 `local_model_dir`（直接目录或 models 根目录）
/// 2. 程序目录 `<数据目录>/.vtrim/models/<model>`
///
/// ⚠️ 这里是"模型在哪"的**唯一真源**：运行时（`model_dir_for`）与界面/诊断
/// （`download::model_install`）都走它 —— 曾出过"模型管理说没下载、实际转写一直在用"
/// 的错位，就是界面那边自己写了另一套路径（只看程序目录）。
///
/// ⚠️ **2026-10-02 去掉了"用户目录优先"那一档**（用户口径：*不要用户目录*）。
/// 那一档是"开发时模型常放这里"留下的，代价是**模型有了两个可能的位置** ——
/// 上面记的那次错位（界面只看程序目录、模型实际在 `~/.vtrim/models`）正是它造成的。
/// 现在位置只有一个，"只查了一处"这类 bug 从根上没有了。
pub fn resolve_model_dir(cfg: &AsrConfig, model: &str) -> (PathBuf, ModelDirSource) {
    resolve_model_dir_in(cfg, model, &crate::path::vtrim_home())
}

/// `resolve_model_dir` 的可注入版本（单测用：不碰环境变量，edition 2024 下 `set_var` 是 unsafe）。
pub fn resolve_model_dir_in(
    cfg: &AsrConfig,
    model: &str,
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

/// qwen3 单流解码用几个线程：`min(4, 核数 - 1)`。
///
/// 上游 sherpa-onnx 对该 session 的默认是 1；本项目的用法是**串行**的
/// （`QWEN3_RECOGNIZER` 那个全局 Mutex），所以多给线程不会互相抢，
/// 而是让唯一在跑的那次解码快一些。
///
/// ⚠️ **但必须给服务端留至少一个核。** ASR 与 HTTP 服务器是**同一个进程**：
/// 把这台机器的核全给解码，tokio 的 worker 与 blocking 池会**一起饿死** ——
/// 症状不是"慢"，而是**连 `/api/jobs` 这种只读任务表都超时**、界面切不动 Dhampir。
/// 用户实测：ASR 期间 `POST /api/polish/dhampir-project` 挂 **182 秒后连接失败**，
/// 而同一个请求在 ASR 结束后只要 **314ms**。
/// 所以这里从核数里扣掉一个 —— 少一个核，换"ASR 期间界面还能用"。
fn decode_threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(1).clamp(1, 4))
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
        // 报清楚"在哪找的"：`local_model_dir` 指到别处时，看日志就知道用的是哪一份
        format!(
            "创建 Qwen3-ASR recognizer 失败（模型目录: {}）——hybrid 需要 qwen3-asr，\
             在设置页「模型管理」里下载，或跑 yekitrim download",
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
        mode: cfg.mode.trim().to_ascii_lowercase(),
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

/// 一个解码窗口：**解码区间**与 **token 归属区间**是两件事。
///
/// 解码时可以往前多喂一点（`segment_padding` 的余量、VAD 段两侧的余量），
/// 但**只有归属区间里的字会被采纳** —— 否则同一句话会被相邻两个窗口各写一次
/// （时间轴是逐字的，没有段级去重兜底，见 `token_belongs_to_chunk` 的注释）。
///
/// 采样下标用整数存：切音频必须按样本对齐；秒只用于时间戳还原与归属判定。
#[derive(Debug, Clone, Copy, PartialEq)]
struct DecodeWindow {
    decode_start_sample: usize,
    decode_end_sample: usize,
    /// 时间戳是相对**解码起点**报的，绝对时间要加它还原
    decode_start_secs: f64,
    claim_start: f64,
    /// 最后一块传 `f64::INFINITY`（解码到哪收到哪）
    claim_end: f64,
}

/// VAD 只用在 **single 模式**。
///
/// hybrid 的时间几乎全在 qwen3 上（实测 3 小时素材 12.4 min vs 打轴 2.5 min，占八成），
/// 而两模式共用这一遍 SenseVoice 打轴 —— 想在 hybrid 上也省，得先动 qwen3，
/// 那是另一件事（见 `docs/plan/`）。所以先按用户诉求落在 single：**hybrid 的输出一个字都不变**。
fn use_vad(cfg: &AsrConfig) -> bool {
    cfg.vad_enabled() && cfg.mode.trim().eq_ignore_ascii_case("single")
}

/// silero 权重的位置。
///
/// ⚠️ 与 `resolve_model_dir` 同一套查找顺序，但 silero 是**裸 onnx 文件**（没有注册表 marker），
/// 所以这里逐个目录找文件，而不是问 `download::model_install`。
fn vad_model_path(cfg: &AsrConfig) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if !cfg.local_model_dir.is_empty() {
        let base = PathBuf::from(&cfg.local_model_dir);
        dirs.push(base.join(vad::MODEL_DIR_NAME));
        dirs.push(base);
    }
    dirs.push(resolve_model_dir(cfg, vad::MODEL_DIR_NAME).0);
    dirs.into_iter().find_map(|d| vad::find_vad_model(&d))
}

/// 解码窗口的**唯一来源**：VAD 开了走语音段，否则（或 VAD 不可用）走固定分块。
///
/// VAD 失败一律**回退**而不是报错：转写了一辈子都是固定分块，模型没下、判定异常
/// 都不该让整场转写停在半路 —— 这也是"效果对齐"的底线（退化成老口径，不会更差）。
fn plan_windows(
    mapped: &MappedWave,
    cfg: &AsrConfig,
    _progress: &dyn ProgressReporter,
) -> Result<Vec<DecodeWindow>> {
    if use_vad(cfg) {
        match plan_vad_windows(mapped, cfg) {
            Ok(w) if !w.is_empty() => return Ok(w),
            Ok(_) => tracing::warn!("VAD 没找到语音段（整段无语音？），回退固定分块"),
            Err(e) => tracing::warn!("VAD 不可用（{e:#}），回退固定分块"),
        }
    }
    plan_fixed_windows(cfg, mapped.total_samples, mapped.sample_rate)
}

/// 固定分块（原口径，未动）：60s 一块、10s 重叠、步进 `chunk_duration - chunk_overlap`。
///
/// 重叠区归属**按下一块的起点切**，每块只认自己那一个 `step`。
///
/// ⚠️ 这里曾经是按「块中点之后交给下一块」（`offset + duration - overlap/2`，即 55s），
/// 于是 `[50s, 55s)` 这一截**两块都留下**；旧代码靠 `dedup_overlaps` 在段一级去重兜住，
/// 而时间轴是逐字的、没有那层兜底 —— 5 秒的字会被写进去两次，进而切出两个覆盖同一段音频的
/// 窗口（qwen3 白跑一遍、字幕重复）。按下一块起点切是**无重叠的分区**：
/// token ∈ `[offset, offset+step)` 归本块，它同时被下一块（从前 10s 预卷开始解码）正确识别，不会漏。
/// 最后一块没有"下一块起点"，它解码到哪就收到哪（`claim_end = INFINITY`）。
fn plan_fixed_windows(
    cfg: &AsrConfig,
    total_samples: usize,
    sample_rate: usize,
) -> Result<Vec<DecodeWindow>> {
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

    // 左余量：多喂 `segment_padding` 秒**已经被上一块认领过的**音频，只为了让本块开头的字
    // 有左上下文 —— 从块起点硬解码等于让识别器从半个词中间开始听，块首几个字容易丢/错。
    // ⚠️ 归属判定不含余量，所以分区不变；但 token 时间戳是相对**解码起点**报的。
    let pad_samples = (cfg.segment_padding.max(0.0) * sample_rate as f64) as usize;
    let total_chunks = total_samples.div_ceil(step_samples);

    let mut out = Vec::with_capacity(total_chunks);
    for i in 0..total_chunks {
        let sample_start = i * step_samples;
        let sample_end = (sample_start + chunk_samples).min(total_samples);
        let decode_start = sample_start.saturating_sub(pad_samples);
        let offset = sample_start as f64 / sample_rate as f64;
        let is_last = i + 1 == total_chunks;
        out.push(DecodeWindow {
            decode_start_sample: decode_start,
            decode_end_sample: sample_end,
            decode_start_secs: decode_start as f64 / sample_rate as f64,
            claim_start: offset,
            claim_end: if is_last {
                f64::INFINITY
            } else {
                offset + step_samples as f64 / sample_rate as f64
            },
        });
    }
    Ok(out)
}

/// 并行度：默认 CPU/4（上限 4），`num_workers` 显式给了就按它（不超过核数）。
///
/// VAD 分块与 SenseVoice 解码**共用这一个口径** —— 两处各写一份必然漂移，
/// 而"并行度"这件事没有第二个语义。
fn decode_workers(cfg: &AsrConfig, total_items: usize) -> usize {
    let base = if cfg.num_workers == 0 {
        std::thread::available_parallelism()
            .map(|n| (n.get() / 4).clamp(1, 4))
            .unwrap_or(1)
    } else {
        (cfg.num_workers as usize).min(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
        )
    };
    base.min(total_items.max(1)).max(1)
}

/// VAD 分块并行扫描。
///
/// silero 是**单线程流式**的：实测 184 分钟素材要 61s，而 SenseVoice 那一段总共才 2.5 分钟
/// —— 串行跑 VAD 会把省下来的一半吐回去。所以按块并行（块数 = worker 数）：
///
/// - 每块从 `start - SEAM_MARGIN` 开始喂（把 LSTM 状态喂热），**名义区间之外的结果不采纳**；
/// - 接缝处相邻两块的段本来就会小幅重叠（后一块会重新检出延续过来的那段话音），
///   交给后面的 `merge_regions` 收口 —— 宁可重叠也不能留缝（留缝就是丢字幕）。
fn detect_speech_parallel(
    mapped: &MappedWave,
    params: &vad::VadParams,
    workers: usize,
    chunk_duration: f32,
) -> Result<Vec<vad::SpeechRegion>> {
    /// 接缝余量：喂给检测器把状态喂热的那一段（秒）。
    const SEAM_MARGIN_SECS: f64 = 1.0;

    let sr = mapped.sample_rate;
    let total = mapped.total_samples;
    let workers = workers.clamp(1, 16);
    let chunk_samples = total.div_ceil(workers).max(1);
    let margin = (SEAM_MARGIN_SECS * sr as f64) as usize;

    let parts: Vec<Vec<vad::SpeechRegion>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .filter_map(|i| {
                let nominal_start = i * chunk_samples;
                if nominal_start >= total {
                    return None;
                }
                let nominal_end = ((i + 1) * chunk_samples).min(total);
                let feed_start = nominal_start.saturating_sub(margin);
                let feed_len = nominal_end - feed_start;
                Some(scope.spawn(move || {
                    // 相对喂入起点的下标 → 绝对样本下标
                    let mut read = |start: usize, count: usize| {
                        let abs = feed_start + start;
                        let count = count.min(total.saturating_sub(abs));
                        mapped.samples_f32(abs, count)
                    };
                    match vad::detect_speech(&mut read, feed_len, params, chunk_duration) {
                        // ⚠️ `detect_speech` 报的是**相对喂入起点**的时间，必须加回 `feed_start`：
                        // 忘了这一步，第 2 块之后的段会全部落在 [0, feed_len] 里，
                        // 与第 1 块大段重叠，一合并就整片消失（实测 1410 段塌成 79 段）。
                        Ok(regions) => {
                            let off = feed_start as f64 / sr as f64;
                            regions
                                .into_iter()
                                .map(|r| vad::SpeechRegion {
                                    start: r.start + off,
                                    end: r.end + off,
                                })
                                .collect()
                        }
                        Err(e) => {
                            tracing::warn!("VAD 分块 {} 失败（{e:#}），该块按无语音处理", i);
                            Vec::new()
                        }
                    }
                }))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });

    let total_secs = total as f64 / sr as f64;
    let mut regions: Vec<vad::SpeechRegion> = parts.into_iter().flatten().collect();
    regions.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    Ok(vad::clamp_regions(regions, total_secs))
}

/// VAD：只把语音段切给识别器。
///
/// 流程 = silero 判定 → 合并近邻 → **动态电平兜底** → 转成解码窗口。每一步都会打日志，
/// 因为"这次到底省了多少、兜底捞回多少"必须能在日志里对上（不然只能靠感觉）。
fn plan_vad_windows(mapped: &MappedWave, cfg: &AsrConfig) -> Result<Vec<DecodeWindow>> {
    let sample_rate = mapped.sample_rate;
    let total_samples = mapped.total_samples;
    let total_secs = total_samples as f64 / sample_rate as f64;
    let started = std::time::Instant::now();

    let model = vad_model_path(cfg).ok_or_else(|| {
        anyhow!(
            "未找到 silero VAD 模型（找过 {} 与 {}）",
            cfg.local_model_dir,
            resolve_model_dir(cfg, vad::MODEL_DIR_NAME).0.display()
        )
    })?;
    let params = vad::VadParams {
        model: model.clone(),
        threshold: cfg.vad_threshold.clamp(0.05, 0.95) as f32,
        min_silence: 0.5,
        min_speech: 0.25,
        sample_rate,
    };

    let read = |start: usize, count: usize| mapped.samples_f32(start, count);
    let vad_workers = decode_workers(cfg, mapped.total_samples);
    let raw = detect_speech_parallel(mapped, &params, vad_workers, cfg.chunk_duration as f32)?;
    let regions = vad::expand_regions(
        vad::clamp_regions(vad::merge_regions(raw, vad::MERGE_GAP_SECS), total_secs),
        vad::GUARD_SECS,
        total_secs,
    );
    let speech_secs: f64 = regions.iter().map(vad::SpeechRegion::duration).sum();

    // 动态电平兜底：按**本片自己的**语音电平放宽，宁可多解码也不漏语音。
    let mut rescued = 0.0;
    let regions = if cfg.vad_rescue_db > 0.0 {
        let frame = (vad::LEVEL_FRAME_SECS * sample_rate as f64).max(1.0) as usize;
        let levels = vad::frame_levels_db(&mut { read }, total_samples, frame);
        match vad::speech_level_db(&levels, &regions, vad::LEVEL_FRAME_SECS) {
            Some(level) => {
                let floor = level - cfg.vad_rescue_db as f32;
                let (r, secs) = vad::rescue_by_level(
                    regions,
                    &levels,
                    vad::LEVEL_FRAME_SECS,
                    floor,
                    vad::RESCUE_MIN_SPAN_SECS,
                );
                rescued = secs;
                r
            }
            None => regions,
        }
    } else {
        regions
    };
    let kept_secs: f64 = regions.iter().map(vad::SpeechRegion::duration).sum();

    let windows = plan_speech_windows(&regions, cfg, sample_rate, total_samples);
    // 固定分块要解的量是 `音频 × chunk/step`（默认 1.2 倍）—— 这才是"省了多少"的对照基准
    let fixed_secs = total_secs * (cfg.chunk_duration / (cfg.chunk_duration - cfg.chunk_overlap).max(0.1));
    tracing::info!(
        "VAD: {} 段语音 {:.0}s（占 {:.1}%）+ 电平兜底捞回 {:.0}s → 解码 {:.0}s（固定分块要解 {:.0}s，约 {:.2}×）; \
         用时 {:.1}s; 模型 {}",
        regions.len(),
        speech_secs,
        speech_secs / total_secs.max(1.0) * 100.0,
        rescued,
        kept_secs,
        fixed_secs,
        fixed_secs / kept_secs.max(0.1),
        started.elapsed().as_secs_f64(),
        model.display()
    );

    Ok(windows)
}

/// 语音段 → 解码窗口。
///
/// - 短于一个 chunk 的段：整段一次解码（两侧各留 `segment_padding`，只有归属段内的字被采纳）；
/// - 超过一个 chunk 的段（连续说话）：与固定分块**同一套步进**切开，claim 依次相接、绝不重叠。
fn plan_speech_windows(
    regions: &[vad::SpeechRegion],
    cfg: &AsrConfig,
    sample_rate: usize,
    total_samples: usize,
) -> Vec<DecodeWindow> {
    let sr = sample_rate as f64;
    let chunk_samples = (cfg.chunk_duration.max(1.0) * sr) as usize;
    let overlap_samples = (cfg.chunk_overlap.max(0.0) * sr) as usize;
    let step_samples = chunk_samples.saturating_sub(overlap_samples).max(1);
    let pad_samples = (cfg.segment_padding.max(0.0) * sr) as usize;

    let mut out: Vec<DecodeWindow> = Vec::new();
    for r in regions {
        let claim_start_s = (r.start.max(0.0) * sr) as usize;
        let claim_end_s = ((r.end.max(0.0) * sr) as usize).min(total_samples);
        if claim_end_s <= claim_start_s {
            continue;
        }
        let decode_start = claim_start_s.saturating_sub(pad_samples);
        let decode_end = (claim_end_s + pad_samples).min(total_samples);

        if decode_end - decode_start <= chunk_samples {
            out.push(DecodeWindow {
                decode_start_sample: decode_start,
                decode_end_sample: decode_end,
                decode_start_secs: decode_start as f64 / sr,
                claim_start: claim_start_s as f64 / sr,
                claim_end: claim_end_s as f64 / sr,
            });
            continue;
        }

        let mut off = decode_start;
        loop {
            let win_end = (off + chunk_samples).min(decode_end);
            let is_last = win_end >= decode_end;
            let claim_s = off.max(claim_start_s);
            let claim_e = if is_last {
                claim_end_s
            } else {
                (off + step_samples).min(claim_end_s)
            };
            if claim_e > claim_s {
                // 段内的窗口同样给左余量（与固定分块同一个理由：别从半个词中间开始听）
                let dec_start = off.saturating_sub(pad_samples).max(decode_start);
                out.push(DecodeWindow {
                    decode_start_sample: dec_start,
                    decode_end_sample: win_end,
                    decode_start_secs: dec_start as f64 / sr,
                    claim_start: claim_s as f64 / sr,
                    claim_end: claim_e as f64 / sr,
                });
            }
            if is_last {
                break;
            }
            off += step_samples;
            if off >= decode_end {
                break;
            }
        }
    }
    out
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

    let windows = plan_windows(mapped, cfg, progress)?;
    let total_chunks = windows.len();
    tracing::info!(
        "sensevoice 扫描: {total_chunks} 个解码窗口（chunk {:.0}s / 重叠 {:.0}s / VAD {}）",
        cfg.chunk_duration,
        cfg.chunk_overlap,
        if use_vad(cfg) { "开" } else { "关" }
    );

    let workers = decode_workers(cfg, total_chunks);

    let mut all_chunks: Vec<RawChunk> = Vec::with_capacity(total_chunks);

    for batch_start in (0..total_chunks).step_by(workers) {
        // 取消在批次边界生效（进行中的单块识别不会被中断）
        if progress.cancelled() {
            bail!("任务已取消");
        }
        let batch_end = (batch_start + workers).min(total_chunks);
        let spawned: Vec<RawChunk> = std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(batch_end - batch_start);

            for (offset_in_batch, win) in windows[batch_start..batch_end].iter().enumerate() {
                let win_idx = batch_start + offset_in_batch;
                let win = *win;
                let decode_offset = win.decode_start_secs;
                let claim_start = win.claim_start;
                let claim_end = win.claim_end;
                // 从 `decode_start`（含余量）解码，但只有 `[claim_start, claim_end)` 的字被采纳
                let chunk_f32 = mapped.samples_f32(
                    win.decode_start_sample,
                    win.decode_end_sample.saturating_sub(win.decode_start_sample),
                );

                handles.push(scope.spawn(move || {
                    let mut out = RawChunk {
                        index: win_idx,
                        offset: claim_start,
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
                                // 时间戳相对**解码起点**（含余量）→ 用 decode_offset 还原
                                let abs = decode_offset + t as f64;
                                if token_belongs_to_chunk(abs, claim_start, claim_end) {
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
    ///  cargo test -p yekitrim-core --lib diagnose_sensevoice_tokens -- --ignored --nocapture`
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

    /// 「模型在哪」的唯一真源：`local_model_dir`（配置）→ `<数据目录>/.vtrim/models`（程序目录）。
    /// ⚠️ 界面与转写**必须**都走它 —— 曾经界面只看程序目录、转写优先用户目录，
    /// 于是"模型管理说没下载、转写其实一直在用另一份"（那条并列在 2026-10-02 收掉了）。
    #[test]
    fn resolve_model_dir_order_and_source() {
        use super::{resolve_model_dir_in, ModelDirSource};

        let base = std::env::temp_dir().join(format!(
            "vtrim-resolve-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let exe_home = base.join("exe").join(".yekitrim");
        // 程序目录里有一份 sensevoice
        std::fs::create_dir_all(exe_home.join("models").join("sensevoice")).unwrap();

        let mut cfg = AsrConfig::default();
        // ① 配置为空 → 程序目录（**唯一的位置**）
        let (dir, src) = resolve_model_dir_in(&cfg, "sensevoice", &exe_home);
        assert_eq!(src, ModelDirSource::ExeDir);
        assert!(dir.ends_with(Path::new("models").join("sensevoice")));

        // ② 没落地过的模型名也照旧给程序目录下的目标路径（下载要往这儿写）
        let (dir, src) = resolve_model_dir_in(&cfg, "qwen3-asr", &exe_home);
        assert_eq!(src, ModelDirSource::ExeDir);
        assert!(dir.starts_with(&exe_home), "{}", dir.display());
        assert!(dir.ends_with(Path::new("models").join("qwen3-asr")));

        // ③ 配了 local_model_dir（models 根目录写法）→ 配置目录，且不再看程序目录
        let cfg_dir = base.join("cfg").join("models");
        std::fs::create_dir_all(cfg_dir.join("qwen3-asr")).unwrap();
        cfg.local_model_dir = cfg_dir.to_string_lossy().into_owned();
        let (dir, src) = resolve_model_dir_in(&cfg, "qwen3-asr", &exe_home);
        assert_eq!(src, ModelDirSource::Config);
        assert_eq!(dir, cfg_dir.join("qwen3-asr"));

        // ④ local_model_dir 直接指向模型目录本体（含模型文件）→ 原样返回
        let direct = base.join("direct");
        std::fs::create_dir_all(&direct).unwrap();
        std::fs::write(direct.join("model.int8.onnx"), b"x").unwrap();
        cfg.local_model_dir = direct.to_string_lossy().into_owned();
        let (dir, src) = resolve_model_dir_in(&cfg, "sensevoice", &exe_home);
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

    /// 语音段 → 解码窗口：**claim 必须铺满每个语音段且互不重叠**。
    ///
    /// 这是 VAD 路径的核心不变量 —— claim 之间留缝会丢字（那一段没人认领），
    /// 重叠会让同一个字写两遍（时间轴是逐字的，没有段级去重兜底）。
    #[test]
    fn speech_windows_tile_every_region_without_gap() {
        let cfg = AsrConfig::default(); // chunk 60 / overlap 10 / pad 0
        let sr = 16_000usize;
        let total_secs = 400.0;
        let total_samples = (total_secs * sr as f64) as usize;
        let regions = vec![
            vad::SpeechRegion {
                start: 10.0,
                end: 20.0,
            }, // 短段：一窗
            vad::SpeechRegion {
                start: 100.0,
                end: 260.0,
            }, // 160s：切成 3 窗（步进 50）
        ];
        let windows = plan_speech_windows(&regions, &cfg, sr, total_samples);
        assert_eq!(windows.len(), 4, "{windows:#?}");

        // ① 每个语音段被完整覆盖；段**内**相邻 claim 严丝合缝（段与段之间的静音本来就没人认领）
        assert_eq!(
            (windows[0].claim_start, windows[0].claim_end),
            (10.0, 20.0),
            "短段一窗搞定: {windows:#?}"
        );
        let mut cursor = 100.0;
        for w in windows.iter().skip(1) {
            assert!(
                (w.claim_start - cursor).abs() < 1e-6,
                "段内 claim 之间有缝/重叠: 期望 {cursor}，实际 {}",
                w.claim_start
            );
            assert!(w.claim_end > w.claim_start);
            cursor = w.claim_end;
        }
        assert!((cursor - 260.0).abs() < 1e-6, "长段没铺满: 到 {cursor}");
        let second: Vec<f64> = windows
            .iter()
            .skip(1)
            .flat_map(|w| [w.claim_start, w.claim_end])
            .collect();
        assert_eq!(
            second,
            vec![100.0, 150.0, 150.0, 200.0, 200.0, 260.0],
            "长段按步进 50 切开（与固定分块的 claim 口径一致：非末窗只认一个 step，末窗收到段尾）"
        );

        // ② 解码区间永远包住 claim，且不越界
        for w in &windows {
            assert!(w.decode_start_sample as f64 / sr as f64 <= w.claim_start + 1e-6);
            assert!(w.decode_end_sample as f64 / sr as f64 >= w.claim_end - 1e-6);
            assert!(w.decode_end_sample <= total_samples);
        }
        // ③ 窗口按时间有序（时间轴合并按这个顺序）
        for pair in windows.windows(2) {
            assert!(pair[0].claim_start <= pair[1].claim_start);
        }
    }

    /// VAD 只在 **single** 模式生效：hybrid 的输出必须一个字都不变。
    #[test]
    fn vad_is_single_mode_only() {
        let mut cfg = AsrConfig {
            mode: "single".into(),
            ..Default::default()
        };
        assert!(use_vad(&cfg), "single 默认开（vad 没写 = 跟随默认）");
        cfg.vad = Some(false);
        assert!(!use_vad(&cfg), "显式关要生效");
        cfg.vad = None;
        cfg.mode = "SINGLE".into();
        assert!(use_vad(&cfg), "模式名大小写不敏感");
        cfg.mode = "hybrid".into();
        assert!(!use_vad(&cfg), "hybrid 不走 VAD");
        assert!(cfg.vad_enabled(), "但开关本身还是开着的（只是这个模式不用）");
    }

    /// 关掉 VAD 时窗口必须与老口径**逐样本一致**（这是"hybrid 输出一个字不变"的依据）。
    #[test]
    fn fixed_windows_keep_legacy_geometry() {
        let cfg = AsrConfig::default();
        let sr = 16_000usize;
        let total = (165.0 * sr as f64) as usize; // 165s → 4 块（步进 50）
        let windows = plan_fixed_windows(&cfg, total, sr).unwrap();
        assert_eq!(windows.len(), 4);
        let step = 50 * sr;
        let chunk = 60 * sr;
        for (i, w) in windows.iter().enumerate() {
            assert_eq!(w.decode_start_sample, i * step);
            assert_eq!(w.decode_end_sample, (i * step + chunk).min(total));
            assert!((w.claim_start - i as f64 * 50.0).abs() < 1e-6);
            if i + 1 == windows.len() {
                assert!(w.claim_end.is_infinite(), "最后一块的解码尽头就是尽头");
            } else {
                assert!((w.claim_end - (i as f64 * 50.0 + 50.0)).abs() < 1e-6);
            }
        }
    }

    /// VAD 自检（合成音频）：**绝对时间戳**与"静音不判语音"这两件事一起验。
    ///
    /// 用法：`VT_VAD_MODEL_DIR=<models 根> VT_VAD_SPEECH_WAV=<一段真实语音 wav> \
    ///        cargo test -p yekitrim-core --lib diagnose_vad_synthetic -- --ignored --nocapture`
    #[test]
    #[ignore = "需要真实 VAD 模型与一段语音 wav"]
    fn diagnose_vad_synthetic() {
        fn model_path_for(p: &Path) -> std::path::PathBuf {
            p.to_path_buf()
        }
        fn rms_of(v: &[f32]) -> f64 {
            if v.is_empty() {
                return 0.0;
            }
            (v.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>() / v.len() as f64).sqrt()
        }
        fn slice_of(pcm: &[f32], a: f64, b: f64, sr: usize) -> Vec<f32> {
            let s = (a * sr as f64).max(0.0) as usize;
            let e = ((b * sr as f64) as usize).min(pcm.len());
            if s >= e {
                return Vec::new();
            }
            pcm[s..e].to_vec()
        }
        let cfg = AsrConfig {
            local_model_dir: std::env::var("VT_VAD_MODEL_DIR").unwrap_or_default(),
            ..Default::default()
        };
        // 测试进程的 CWD 是 crate 根，默认样本用 CARGO_MANIFEST_DIR 拼绝对路径
        let speech_wav = std::env::var("VT_VAD_SPEECH_WAV").unwrap_or_else(|_| {
            format!(
                "{}/../../target/debug/.vtrim/models/sensevoice/test_wavs/zh.wav",
                env!("CARGO_MANIFEST_DIR")
            )
        });
        let (speech, sr) = read_wav_range(Path::new(&speech_wav), 0.0, 600.0).unwrap();
        assert!(!speech.is_empty(), "语音样本为空: {speech_wav}");

        let lead = 20.0 * sr as f64;
        let tail = 20.0 * sr as f64;
        let mut pcm = vec![0.0f32; lead as usize];
        pcm.extend_from_slice(&speech);
        let speech_span = (speech.len() as f64 / sr as f64, 0.0);
        pcm.extend(std::iter::repeat_n(0.0f32, tail as usize));
        let speech_at = (lead / sr as f64, speech_span.0);

        let model = vad_model_path(&cfg).expect("找不到 silero VAD 模型");
        let dir = model.parent().unwrap().to_path_buf();

        // 逐一体检：两个 silero 导出 × 两档阈值，外加"纯静音"这一格（模型在数字零上不该判语音）
        let cases: Vec<(String, f32, std::path::PathBuf)> = {
            let mut v = Vec::new();
            for name in ["silero_vad.onnx", "silero_vad.int8.onnx"] {
                let p = dir.join(name);
                if !p.exists() {
                    continue;
                }
                for thr in [0.5f32, 0.9] {
                    v.push((format!("{name} @{thr}"), thr, model_path_for(&p)));
                }
            }
            v.push((
                "纯静音 30s（int8@0.5）".to_string(),
                0.5,
                model_path_for(&dir.join("silero_vad.int8.onnx")),
            ));
            v
        };

        for (label, thr, path) in cases {
            let zeros_only = label.starts_with("纯静音");
            let input: Vec<f32> = if zeros_only {
                vec![0.0f32; (30.0 * sr as f64) as usize]
            } else {
                pcm.clone()
            };
            let params = vad::VadParams {
                model: path,
                threshold: thr,
                min_silence: 0.5,
                min_speech: 0.25,
                sample_rate: sr,
            };
            let segs = vad::detect_speech_with_samples(
                &mut |s, n| input[s..(s + n).min(input.len())].to_vec(),
                input.len(),
                &params,
                cfg.chunk_duration as f32,
            )
            .unwrap();
            let kept: f64 = segs.iter().map(|(r, _)| r.duration()).sum();
            let where_ = if zeros_only {
                String::new()
            } else {
                format!(
                    "（真语音 {:.2}~{:.2}s）",
                    speech_at.0 + 0.75,
                    speech_at.0 + speech_at.1
                )
            };
            println!(
                "{label:26} → {} 段 / 覆盖 {:.2}s {where_}",
                segs.len(),
                kept
            );
            for (r, samples) in segs.iter().take(4) {
                let at_reported = slice_of(&input, r.start, r.end, sr);
                println!(
                    "      {:.2}~{:.2}s ({:.2}s) RMS 段={:.4} 同区间={:.4}",
                    r.start,
                    r.end,
                    r.duration(),
                    rms_of(samples),
                    rms_of(&at_reported)
                );
            }
        }

        // 单模型断言（默认参数）：真语音必须被覆盖，且不能把整段 40s 静音吃进来
        let params = vad::VadParams {
            model: vad_model_path(&cfg).unwrap(),
            threshold: cfg.vad_threshold as f32,
            min_silence: 0.5,
            min_speech: 0.25,
            sample_rate: sr,
        };
        let regions = vad::detect_speech(
            &mut |s, n| pcm[s..(s + n).min(pcm.len())].to_vec(),
            pcm.len(),
            &params,
            cfg.chunk_duration as f32,
        )
        .unwrap();

        println!(
            "合成音频: {}s 静音 + {:.2}s 语音({:.2}~{:.2}s) + {}s 静音",
            20.0, speech_at.1, speech_at.0, speech_at.0 + speech_at.1, 20.0
        );
        let mut kept = 0.0;
        for r in &regions {
            println!("  段: {:.2} ~ {:.2}s ({:.2}s)", r.start, r.end, r.duration());
            kept += r.duration();
        }
        println!(
            "共 {} 段、覆盖 {:.2}s（真语音 {:.2}s）→ 覆盖率 {:.0}%",
            regions.len(),
            kept,
            speech_at.1,
            kept / speech_at.1 * 100.0
        );
        assert!(!regions.is_empty(), "合成音频里的语音没被检出");
        // 绝对时间戳：语音在 [20, 20+len]，检出的段必须落在这附近（不是"从 0 开始"或"从缓冲头算"）
        let first = regions[0];
        assert!(
            (first.start - speech_at.0).abs() < 1.5,
            "起点不在语音处（{:.2} vs {:.2}）—— 时间戳不是绝对的",
            first.start,
            speech_at.0
        );
        // 静音不判：整段覆盖不该把 40s 静音吃进去
        assert!(
            kept < speech_at.1 + 6.0,
            "静音被算进语音了：覆盖 {kept:.1}s ≫ 语音 {:.1}s",
            speech_at.1
        );
    }

    /// VAD 分段诊断（**只跑 VAD，不跑 ASR**）：在真实素材上看"跳过多少、兜底捞回多少"。
    ///
    /// 用法（路径写绝对路径；模型走 `VT_VAD_MODEL_DIR` 或默认查找顺序）：
    /// `VT_ASR_WAV=<wav> cargo test -p yekitrim-core --lib diagnose_vad_regions -- --ignored --nocapture`
    /// 可选：`VT_VAD_THRESHOLD` / `VT_VAD_RESCUE_DB` / `VT_VAD_MODEL_DIR`
    #[test]
    #[ignore = "需要真实 VAD 模型与音频（VT_ASR_WAV）"]
    fn diagnose_vad_regions() {
        let wav = std::env::var("VT_ASR_WAV").expect("需要 VT_ASR_WAV=<wav 绝对路径>");
        let cfg = AsrConfig {
            mode: "single".into(),
            local_model_dir: std::env::var("VT_VAD_MODEL_DIR").unwrap_or_default(),
            vad_threshold: std::env::var("VT_VAD_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.5),
            vad_rescue_db: std::env::var("VT_VAD_RESCUE_DB")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(16.0),
            ..Default::default()
        };

        let mapped = MappedWave::open(Path::new(&wav)).unwrap();
        let sr = mapped.sample_rate as f64;
        let total_secs = mapped.total_samples as f64 / sr;
        let sum = |ws: &[DecodeWindow]| -> f64 {
            ws.iter()
                .map(|w| (w.decode_end_sample - w.decode_start_sample) as f64 / sr)
                .sum()
        };

        let fixed = plan_fixed_windows(&cfg, mapped.total_samples, mapped.sample_rate).unwrap();
        let fixed_secs = sum(&fixed);

        // 一次 VAD 扫描 + 一次电平扫描，然后把各档兜底阈值**离线**算出来对比
        // （不然每试一个阈值就重扫一遍 3 小时音频）。
        let model = vad_model_path(&cfg).expect("找不到 silero VAD 模型");
        let params = vad::VadParams {
            model,
            threshold: cfg.vad_threshold as f32,
            min_silence: 0.5,
            min_speech: 0.25,
            sample_rate: mapped.sample_rate,
        };
        let t0 = std::time::Instant::now();
        let raw = detect_speech_parallel(
            &mapped,
            &params,
            decode_workers(&cfg, mapped.total_samples),
            cfg.chunk_duration as f32,
        )
        .unwrap();
        let vad_secs = t0.elapsed().as_secs_f64();
        let t1 = std::time::Instant::now();
        let frame = (vad::LEVEL_FRAME_SECS * sr) as usize;
        let levels =
            vad::frame_levels_db(&mut |s, n| mapped.samples_f32(s, n), mapped.total_samples, frame);
        let level_secs = t1.elapsed().as_secs_f64();

        let regions = vad::expand_regions(
            vad::clamp_regions(
                vad::merge_regions(raw.clone(), vad::MERGE_GAP_SECS),
                total_secs,
            ),
            vad::GUARD_SECS,
            total_secs,
        );
        println!("原始段（前 5 个 / 后 3 个）:");
        for r in raw.iter().take(5).chain(raw.iter().rev().take(3).rev()) {
            println!("  {:.2} ~ {:.2}s ({:.1}s)", r.start, r.end, r.duration());
        }
        let raw_kept: f64 = raw.iter().map(vad::SpeechRegion::duration).sum();
        // 让外部（python）能拿它跟已知正确的字幕时间戳对齐，验证"时间戳到底准不准"
        if let Ok(path) = std::env::var("VT_VAD_DUMP") {
            let j = serde_json::json!({
                "total": total_secs,
                "sample_rate": mapped.sample_rate,
                "raw": raw.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>(),
                "merged": regions.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>(),
            });
            std::fs::write(&path, j.to_string()).expect("写 VT_VAD_DUMP 失败");
            println!("已写 VAD 区间: {path}");
        }
        println!(
            "原始覆盖 {:.0}s ({:.1}%)，合并后 {:.0}s —— 若原始时间戳不是绝对时间，这里会明显对不上",
            raw_kept,
            raw_kept / total_secs * 100.0,
            regions.iter().map(vad::SpeechRegion::duration).sum::<f64>()
        );
        let pure_kept: f64 = regions.iter().map(vad::SpeechRegion::duration).sum();
        let speech_level = vad::speech_level_db(&levels, &regions, vad::LEVEL_FRAME_SECS);
        let pct = |p: f64| vad::percentile(&levels, p).unwrap_or(f32::NAN);

        println!("音频: {total_secs:.1}s ({:.1} 分钟)", total_secs / 60.0);
        println!("固定分块: {} 窗，解码 {fixed_secs:.0}s", fixed.len());
        println!(
            "silero:   {} 段（合并后）{pure_kept:.0}s = {:.1}%（探测 {vad_secs:.1}s）",
            regions.len(),
            pure_kept / total_secs * 100.0
        );
        println!(
            "电平:     语音 {:.1}dB | 帧分位 p10={:.1} p25={:.1} p50={:.1} p75={:.1} p90={:.1}（扫描 {level_secs:.1}s）",
            speech_level.unwrap_or(f32::NAN),
            pct(0.10),
            pct(0.25),
            pct(0.50),
            pct(0.75),
            pct(0.90)
        );
        println!("兜底 rescue_db → 保留 / 相对固定分块的解码比:");
        for rescue in [0.0f64, 2.0, 4.0, 6.0, 8.0, 12.0, 16.0] {
            let regions = if rescue <= 0.0 {
                regions.clone()
            } else {
                let floor = speech_level.unwrap_or(-60.0) - rescue as f32;
                vad::rescue_by_level(
                    regions.clone(),
                    &levels,
                    vad::LEVEL_FRAME_SECS,
                    floor,
                    vad::RESCUE_MIN_SPAN_SECS,
                )
                .0
            };
            let windows = plan_speech_windows(&regions, &cfg, mapped.sample_rate, mapped.total_samples);
            let kept = sum(&windows);
            let est = fixed_secs / kept.max(1.0);
            println!(
                "  {rescue:>5.1}dB → 保留 {kept:>7.0}s ({:>4.1}%)，解码量 {est:.2}× 更少，窗口 {} 个",
                kept / total_secs * 100.0,
                windows.len()
            );
        }

        let windows = plan_windows(&mapped, &cfg, &crate::progress::NoopProgress).unwrap();
        assert!(!windows.is_empty());
        let mut prev = 0.0f64;
        for w in &windows {
            assert!(w.claim_start + 1e-6 >= prev, "claim 重叠: {w:?}");
            assert!(w.claim_end <= total_secs + 1e-6, "claim 越界: {w:?}");
            assert!(w.decode_end_sample <= mapped.total_samples);
            prev = w.claim_end;
        }
    }
}
