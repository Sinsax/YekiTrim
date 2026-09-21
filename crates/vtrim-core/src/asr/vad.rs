//! VAD（silero）+ **动态电平兜底**：让 SenseVoice 只解码"真有人说话"的区间。
//!
//! ## 为什么值得做
//!
//! SenseVoice 是非自回归模型，**代价基本与喂进去的音频长度成正比**，而固定分块
//! （`chunk_duration` 60s / `chunk_overlap` 10s）会把整段录播——含静音、BGM、游戏声——
//! 全送进模型，解码量 = 音频 × 1.2（60/50 步进）。
//! 实测三场真实录播（184/204/288 分钟）里"真有字"的时间只有 **32.6% / 37.1% / 43.2%**，
//! 电台那场纯静音 **41.6%**（`ffmpeg silencedetect -30dB`，2615 段、均 1.76s）。
//! 只解码语音段 → SenseVoice 那段快约 2.3~3×（**端到端只对 single 模式有意义**：
//! hybrid 的 8 成时间是 qwen3，VAD 动不了它）。
//!
//! ## 为什么不直接相信 VAD
//!
//! VAD 漏判一次 = **整条字幕消失**（不是变差，是没有了）。所以这里除了 silero 的概率判定，
//! 还挂一层**动态电平兜底** —— 参考精修字幕那套响度处理的口径（`measure_loudness_curve`：
//! 按这一场**自己的**电平判断，而不是一个固定的 dB 门限）：
//!
//! - 先量出**本片语音电平**（VAD 判为语音的那些帧的 75 分位），再减 `vad_rescue_db` 得到"允许跳过"的电平线；
//! - 任何连响 ≥ [`RESCUE_MIN_SPAN_SECS`] 的空档，只要电平够这条线，就**捞回来解码**。
//!
//! 兜底只会**多**解码、绝不会少 —— 它的失效方向是"退化成现在的固定分块"（省不到时间），
//! 而不是"字幕少一条"。这也是"效果对齐单模型"这条验收线的结构性保证。
//!
//! ## 落点
//!
//! 检测结果交给 `local::plan_speech_windows` 变成解码窗口；本模块只负责
//! 「哪些区间是语音」这一件事，不碰解码、不碰时间轴（纯函数都能单测）。
use std::ffi::CString;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sherpa_onnx_sys as sys;

/// 动态电平的帧长（秒）。20ms 足够细，3 小时音频也只有约 55 万个点。
pub const LEVEL_FRAME_SECS: f64 = 0.02;

/// 每次 `AcceptWaveform` 喂多少采样。
///
/// ⚠️ **必须是一次窗口的量（512 ≈ 32ms），不能图省事喂一大块**。上游
/// `voice-activity-detector.cc` 的循环是「先把这一批的窗口全部 push 进环形缓冲、再看这批
/// 有没有语音」，而段的起点/终点是按**这批结束时**的 `buffer_.Tail()` 反推的：
/// `start = Tail - 2*WindowSize - MinSpeechDuration`、`end = Tail - MinSilenceDuration`。
/// 于是喂 4 秒一块时，**段边界会被量化到 4 秒**（实测合成音频：真语音 20.75~25.59s
/// 被报成 23.69~31.50s，且换模型、换阈值都不变 —— 那是块边界说了算，不是模型说了算）。
/// 上游源码里那句 `NOTE(fangjun): Please don't use a very large n.` 说的就是这件事。
///
/// 代价：3 小时素材要喂 ~124 万次。实测（184 分钟素材）整段 VAD 用时与喂 4s 块同量级，
/// 所以按窗口喂是白拿的精度。
const FEED_SAMPLES: usize = 512;

/// 兜底捞回的最短连响时长（秒）：比这更短的响动当作瞬时噪声（咳嗽/按键/音效）。
pub const RESCUE_MIN_SPAN_SECS: f64 = 0.5;

/// 语音段两侧的保护余量（秒）。
///
/// silero 的段尾取的是"概率掉到负门限以下的那一刻"，句中最后那几个字如果读得轻
/// （实测合成音频末尾 -37dB 的那 0.12s），会被判成非语音而**切掉**。VAD 只是**加速器**，
/// 切掉半截字比多解 0.4 秒贵得多，所以两侧各留一点 —— 实测每段多解 0.4s，
/// 3 小时素材（约 500 段）总共多解约 2%。
pub const GUARD_SECS: f64 = 0.2;

/// VAD 语音段之间小于这个间隔就连起来解码（秒）。
///
/// 判据是"解码成本"而不是"语义"：0.5~0.8s 的停顿被 silero 切开是很正常的
/// （`min_silence_duration` 就是 0.5），但每一段单独解码会丢掉左上下文，
/// 段数也会膨胀。0.8s 以内连起来，多解的那点停顿远小于固定分块里被浪费的静音。
pub const MERGE_GAP_SECS: f64 = 0.8;

/// 一小段被判为"有人在说话"的区间（绝对秒）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechRegion {
    pub start: f64,
    pub end: f64,
}

impl SpeechRegion {
    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

/// silero VAD 的旋钮（`[asr] vad_threshold` + 内部常量）。
#[derive(Debug, Clone)]
pub struct VadParams {
    pub model: PathBuf,
    pub threshold: f32,
    pub min_silence: f32,
    pub min_speech: f32,
    pub sample_rate: usize,
}

impl VadParams {
    /// silero 的 C 结构体（`max_speech_duration` 取 `chunk_duration`：让 VAD 自己把超长连续语音
    /// 切开，它在内部环形缓冲里要留的样本就有上界，见 [`buffer_secs`]）。
    ///
    /// 切开后相邻两段是无缝的（gap≈0），会被 [`merge_regions`] 重新连成一段，
    /// 再由 `local::plan_speech_windows` 按"固定分块 + 重叠"切开 —— 与原来的口径一致。
    fn silero_sys(&self, model: *const std::os::raw::c_char, chunk_duration: f32) -> sys::SileroVadModelConfig {
        sys::SileroVadModelConfig {
            model,
            threshold: self.threshold,
            min_silence_duration: self.min_silence,
            min_speech_duration: self.min_speech,
            window_size: 512, // silero v4/v5 在 16kHz 下固定 512
            max_speech_duration: chunk_duration.max(1.0),
        }
    }
}

/// VAD 内部缓冲要留多久的音频：`max_speech_duration` 的两倍，且至少 60s。
///
/// 留足是有意的：万一上游的 `max_speech_duration` 没生效（或某段语音特别长），
/// 缓冲不够会让段样本被截断，而那是**静默**的错误结果。
pub fn buffer_secs(chunk_duration: f32) -> f32 {
    (chunk_duration * 2.0).max(60.0)
}

/// silero 检测器的薄封装：只用到 C API 的 Create / AcceptWaveform / Front / Pop / Flush / Destroy。
///
/// ⚠️ **为什么走 FFI 而不是 Rust 封装**：`sherpa-onnx` 1.13.3 的 Rust 侧 `vad` 模块是**私有**的
/// （lib.rs 里写的是 `mod vad;`，上游到 1.13.6 之后才 `pub`），公开路径根本 use 不到。
/// 本仓库本来就把 `sherpa-onnx-sys` 换成 `vendor/sherpa-onnx-sys`（为了不带 GPL 的 espeak-ng），
/// 所以直接用那份绑定最省事，也不必动依赖版本（动版本要连带重验 fork 的 pin 与哈希）。
///
/// 生命周期：`Front()` 返回的是 **C++ 侧 new 出来的副本**，读走 start/n 后必须
/// `DestroySpeechSegment`；忘掉就是每段泄漏一次（3 小时素材几千段）。
struct SileroVad {
    ptr: *const sys::VoiceActivityDetector,
    /// Create 时传进去的 C 字符串：C++ 会拷贝，但留着更省心（也不贵）。
    _keepalive: Vec<CString>,
}

impl SileroVad {
    fn create(params: &VadParams, chunk_duration: f32) -> Result<Self> {
        let model = CString::new(params.model.to_string_lossy().as_bytes())
            .context("VAD 模型路径含 NUL 字节")?;
        let provider = CString::new("cpu").expect("字面量无 NUL");
        let keepalive = vec![model, provider];
        let config = sys::VadModelConfig {
            silero_vad: params.silero_sys(keepalive[0].as_ptr(), chunk_duration),
            sample_rate: params.sample_rate as i32,
            num_threads: 1,
            provider: keepalive[1].as_ptr(),
            debug: 0,
            ten_vad: sys::TenVadModelConfig {
                model: std::ptr::null(),
                threshold: 0.5,
                min_silence_duration: 0.5,
                min_speech_duration: 0.25,
                window_size: 256,
                max_speech_duration: 1e9,
            },
        };
        let ptr = unsafe {
            sys::SherpaOnnxCreateVoiceActivityDetector(&config, buffer_secs(chunk_duration))
        };
        if ptr.is_null() {
            bail!("创建 silero VAD 失败（模型: {}）", params.model.display());
        }
        Ok(Self {
            ptr,
            _keepalive: keepalive,
        })
    }

    fn accept(&self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        unsafe {
            sys::SherpaOnnxVoiceActivityDetectorAcceptWaveform(
                self.ptr,
                samples.as_ptr(),
                samples.len() as i32,
            )
        }
    }

    fn is_empty(&self) -> bool {
        unsafe { sys::SherpaOnnxVoiceActivityDetectorEmpty(self.ptr) != 0 }
    }

    fn pop(&self) {
        unsafe { sys::SherpaOnnxVoiceActivityDetectorPop(self.ptr) }
    }

    fn flush(&self) {
        unsafe { sys::SherpaOnnxVoiceActivityDetectorFlush(self.ptr) }
    }

    /// 队首语音段的 `[start_sample, end_sample)`（绝对样本下标，相对**喂进去的全部音频**）。
    fn front_range(&self) -> Option<(usize, usize)> {
        self.front_samples().map(|(s, e, _)| (s, e))
    }

    /// 诊断用：连段样本一起取回来（`Front()` 给的是 C++ new 出来的副本，读完必须销毁）。
    pub fn front_samples(&self) -> Option<(usize, usize, Vec<f32>)> {
        if self.is_empty() {
            return None;
        }
        let seg = unsafe { sys::SherpaOnnxVoiceActivityDetectorFront(self.ptr) };
        if seg.is_null() {
            return None;
        }
        let (start, n, samples) = unsafe {
            let start = (*seg).start;
            let n = (*seg).n;
            let samples = if (*seg).samples.is_null() || n <= 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts((*seg).samples, n as usize).to_vec()
            };
            (start, n, samples)
        };
        unsafe { sys::SherpaOnnxDestroySpeechSegment(seg) };
        if n <= 0 || start < 0 {
            return None;
        }
        Some((start as usize, (start + n) as usize, samples))
    }
}

impl Drop for SileroVad {
    fn drop(&mut self) {
        unsafe { sys::SherpaOnnxDestroyVoiceActivityDetector(self.ptr) }
    }
}

/// 调 silero VAD 扫一遍，返回语音区间（绝对秒，已排序）。
///
/// `read(start_sample, count)` 由调用方提供（内存映射的 WAV），本函数按
/// [`FEED_BLOCK_SECS`] 的粒度一边喂一边收段 —— 3 小时音频不会因此多占内存。
pub fn detect_speech(
    read: &mut dyn FnMut(usize, usize) -> Vec<f32>,
    total_samples: usize,
    params: &VadParams,
    chunk_duration: f32,
) -> Result<Vec<SpeechRegion>> {
    Ok(run_vad(read, total_samples, params, chunk_duration, false)?
        .into_iter()
        .map(|(r, _)| r)
        .collect())
}

/// 诊断用：与 [`detect_speech`] 同一条路径，但把每段的**样本**也带回来。
///
/// 用来钉死"时间戳与样本是不是同一段音频"——只有两边对得上，`start` 才能当绝对时间用。
pub fn detect_speech_with_samples(
    read: &mut dyn FnMut(usize, usize) -> Vec<f32>,
    total_samples: usize,
    params: &VadParams,
    chunk_duration: f32,
) -> Result<Vec<(SpeechRegion, Vec<f32>)>> {
    run_vad(read, total_samples, params, chunk_duration, true)
}

fn run_vad(
    read: &mut dyn FnMut(usize, usize) -> Vec<f32>,
    total_samples: usize,
    params: &VadParams,
    chunk_duration: f32,
    keep_samples: bool,
) -> Result<Vec<(SpeechRegion, Vec<f32>)>> {
    let vad = SileroVad::create(params, chunk_duration)?;
    let sr = params.sample_rate as f64;
    let block = FEED_SAMPLES;

    let mut out: Vec<(SpeechRegion, Vec<f32>)> = Vec::new();
    let drain = |vad: &SileroVad, out: &mut Vec<(SpeechRegion, Vec<f32>)>| {
        loop {
            let next = if keep_samples {
                vad.front_samples()
            } else {
                vad.front_range().map(|(s, e)| (s, e, Vec::new()))
            };
            let Some((start, end, samples)) = next else {
                break;
            };
            out.push((
                SpeechRegion {
                    start: start as f64 / sr,
                    end: end as f64 / sr,
                },
                samples,
            ));
            vad.pop();
        }
    };

    let mut pos = 0usize;
    while pos < total_samples {
        let count = block.min(total_samples - pos);
        let samples = read(pos, count);
        vad.accept(&samples);
        drain(&vad, &mut out);
        pos += count;
    }
    vad.flush();
    drain(&vad, &mut out);

    out.sort_by(|a, b| {
        a.0.start
            .partial_cmp(&b.0.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(out)
}

/// 逐帧 RMS 电平（dBFS）。`frame_samples` 由 [`LEVEL_FRAME_SECS`] 换算。
///
/// 这是"动态电平"的来源：算的是**这一场自己的**响度分布，不是绝对 dB 门限 ——
/// 所以不同增益的录播（手机麦 / 声卡 / 录播姬直出）都能用同一套参数。
pub fn frame_levels_db(
    read: &mut dyn FnMut(usize, usize) -> Vec<f32>,
    total_samples: usize,
    frame_samples: usize,
) -> Vec<f32> {
    if frame_samples == 0 {
        return Vec::new();
    }
    let mut levels = Vec::with_capacity(total_samples / frame_samples + 1);
    let mut pos = 0usize;
    while pos < total_samples {
        let count = frame_samples.min(total_samples - pos);
        let samples = read(pos, count);
        let sum_sq: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        let rms = (sum_sq / samples.len().max(1) as f64).sqrt();
        // 1e-9 ≈ -180dBFS，纯数字零样本也不会变成 -inf
        levels.push((20.0 * (rms + 1e-9).log10()) as f32);
        pos += count;
    }
    levels
}

/// `p` 分位（0~1）。空输入返回 `None`。会就地排序副本。
pub fn percentile(values: &[f32], p: f64) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((v.len() - 1) as f64 * p.clamp(0.0, 1.0)).round() as usize;
    Some(v[idx])
}

/// 本片"语音电平"（dB）：落在语音段内的帧的 75 分位。
///
/// 取 75 分位而不是均值：段内也夹着字与字之间的停顿（电平低），均值会被它们拉下去。
pub fn speech_level_db(levels: &[f32], regions: &[SpeechRegion], frame_secs: f64) -> Option<f32> {
    if levels.is_empty() || regions.is_empty() || frame_secs <= 0.0 {
        return None;
    }
    // 两指针走一遍：3 小时音频有 55 万帧、语音段有几千个，
    // "每帧都扫一遍所有段"是 10^9 量级的白跑（段已排序，游标即可）。
    let mut inside: Vec<f32> = Vec::new();
    let mut ri = 0usize;
    for (i, db) in levels.iter().enumerate() {
        let t = i as f64 * frame_secs;
        while ri < regions.len() && regions[ri].end <= t {
            ri += 1;
        }
        if ri < regions.len() && t >= regions[ri].start && t < regions[ri].end {
            inside.push(*db);
        }
    }
    percentile(&inside, 0.75)
}

/// 合并靠得很近的语音段（间隔 ≤ `gap`）。
pub fn merge_regions(mut regions: Vec<SpeechRegion>, gap: f64) -> Vec<SpeechRegion> {
    regions.retain(|r| r.end > r.start);
    regions.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<SpeechRegion> = Vec::with_capacity(regions.len());
    for r in regions {
        match out.last_mut() {
            Some(last) if r.start - last.end <= gap => {
                last.end = last.end.max(r.end);
            }
            _ => out.push(r),
        }
    }
    out
}

/// 动态电平兜底：把"电平够响"的空档捞回成语音段。
///
/// 判据（全部相对**本片自己的**语音电平，不是固定 dB）：
/// 连响 ≥ `min_span` 秒 且 帧电平 ≥ `floor_db`。返回 (合并后的区间, 捞回秒数)。
///
/// 只做加法：捞回来的区间与 `regions` 取并集，**绝不会**因此少解码任何东西。
pub fn rescue_by_level(
    regions: Vec<SpeechRegion>,
    levels: &[f32],
    frame_secs: f64,
    floor_db: f32,
    min_span: f64,
) -> (Vec<SpeechRegion>, f64) {
    if levels.is_empty() || frame_secs <= 0.0 {
        return (regions, 0.0);
    }
    let before: f64 = regions.iter().map(SpeechRegion::duration).sum();

    let mut runs: Vec<SpeechRegion> = Vec::new();
    let mut run_start: Option<usize> = None;
    for (i, db) in levels.iter().enumerate() {
        let loud = *db >= floor_db;
        match (loud, run_start) {
            (true, None) => run_start = Some(i),
            (false, Some(s)) => {
                let r = SpeechRegion {
                    start: s as f64 * frame_secs,
                    end: i as f64 * frame_secs,
                };
                if r.duration() >= min_span {
                    runs.push(r);
                }
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = run_start {
        let r = SpeechRegion {
            start: s as f64 * frame_secs,
            end: levels.len() as f64 * frame_secs,
        };
        if r.duration() >= min_span {
            runs.push(r);
        }
    }

    let mut all = regions;
    all.extend(runs);
    let merged = merge_regions(all, 0.0);
    let after: f64 = merged.iter().map(SpeechRegion::duration).sum();
    (merged, (after - before).max(0.0))
}

/// 裁剪到 `[0, total_secs]` 并丢掉空段（VAD 的时间戳理论上不会越界，越界了也不能写进时间轴）。
pub fn clamp_regions(regions: Vec<SpeechRegion>, total_secs: f64) -> Vec<SpeechRegion> {
    regions
        .into_iter()
        .map(|r| SpeechRegion {
            start: r.start.clamp(0.0, total_secs.max(0.0)),
            end: r.end.clamp(0.0, total_secs.max(0.0)),
        })
        .filter(|r| r.end - r.start > 1e-3)
        .collect()
}

/// 两侧各放 `pad` 秒（见 [`GUARD_SECS`]），裁剪后重新合并（放完可能贴到一起）。
pub fn expand_regions(
    regions: Vec<SpeechRegion>,
    pad: f64,
    total_secs: f64,
) -> Vec<SpeechRegion> {
    let grown: Vec<SpeechRegion> = regions
        .into_iter()
        .map(|r| SpeechRegion {
            start: r.start - pad,
            end: r.end + pad,
        })
        .collect();
    clamp_regions(merge_regions(grown, 0.0), total_secs)
}

/// 在候选模型目录里找 silero 权重（优先 int8：208KB、判定质量与 fp32 无可见差别）。
pub fn find_vad_model(dir: &Path) -> Option<PathBuf> {
    ["silero_vad.int8.onnx", "silero_vad.onnx"]
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.exists())
}

/// 模型安装的落点目录名（与 `download::VAD_MODEL_DIR` 同源，别在两处各写一份字符串）。
pub const MODEL_DIR_NAME: &str = "silero-vad";

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: f64, end: f64) -> SpeechRegion {
        SpeechRegion { start, end }
    }

    #[test]
    fn merge_glues_only_close_regions() {
        let merged = merge_regions(vec![r(0.0, 1.0), r(1.5, 2.0), r(5.0, 6.0)], 0.8);
        assert_eq!(merged, vec![r(0.0, 2.0), r(5.0, 6.0)]);
    }

    #[test]
    fn merge_is_order_insensitive_and_drops_empty() {
        let merged = merge_regions(vec![r(5.0, 6.0), r(0.2, 1.0), r(0.0, 0.1), r(0.9, 2.0)], 0.2);
        assert_eq!(merged, vec![r(0.0, 2.0), r(5.0, 6.0)]);
    }

    #[test]
    fn percentile_picks_expected_bucket() {
        let v: Vec<f32> = (1..=100).map(|x| x as f32).collect();
        assert_eq!(percentile(&v, 0.0), Some(1.0));
        assert_eq!(percentile(&v, 0.75), Some(75.0));
        assert_eq!(percentile(&v, 1.0), Some(100.0));
        assert_eq!(percentile(&[], 0.5), None);
    }

    #[test]
    fn speech_level_uses_frames_inside_regions_only() {
        // 帧长 1s：前 3 帧在语音段内（-10dB），后面全是 -60dB 的静音
        let levels = vec![-10.0, -10.0, -10.0, -60.0, -60.0];
        let level = speech_level_db(&levels, &[r(0.0, 3.0)], 1.0).unwrap();
        assert!((level + 10.0).abs() < 1e-6, "got {level}");
    }

    #[test]
    fn rescue_keeps_loud_gaps_and_ignores_quiet_ones() {
        // 0~2s 语音，2~6s 空档（其中 3~5s 很响），6~8s 语音；floor=-25dB
        let mut levels = vec![f32::NEG_INFINITY; 10];
        for f in levels.iter_mut().take(2) {
            *f = -12.0;
        }
        for f in levels.iter_mut().take(5).skip(3) {
            *f = -14.0; // 3~5s：够响 → 捞回
        }
        for f in levels.iter_mut().take(8).skip(6) {
            *f = -12.0;
        }
        let (out, rescued) =
            rescue_by_level(vec![r(0.0, 2.0), r(6.0, 8.0)], &levels, 1.0, -25.0, 0.5);
        assert_eq!(out, vec![r(0.0, 2.0), r(3.0, 5.0), r(6.0, 8.0)]);
        assert!((rescued - 2.0).abs() < 1e-6, "rescued={rescued}");
    }

    #[test]
    fn rescue_drops_spans_shorter_than_min_span() {
        // 帧长 0.5s、只有第 1 帧响 → 连响 0.5s；min_span 0.6s 时不够长，不捞
        let levels = vec![-10.0, -80.0, -80.0];
        let (out, rescued) = rescue_by_level(vec![], &levels, 0.5, -25.0, 0.6);
        assert_eq!(out, vec![]);
        assert_eq!(rescued, 0.0);
        // min_span 放到 0.5s 就该捞回来
        let (out, rescued) = rescue_by_level(vec![], &levels, 0.5, -25.0, 0.5);
        assert_eq!(out, vec![r(0.0, 0.5)]);
        assert!((rescued - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rescue_never_shrinks_coverage() {
        let levels = vec![-80.0; 20];
        let base = vec![r(1.0, 3.0), r(7.0, 9.0)];
        let (out, rescued) = rescue_by_level(base.clone(), &levels, 0.1, -30.0, 0.5);
        assert_eq!(out, base, "捞回只做加法，静音不该改动原区间");
        assert_eq!(rescued, 0.0);
    }

    #[test]
    fn clamp_regions_bounds_and_drops_empty() {
        let out = clamp_regions(vec![r(-1.0, 2.0), r(3.0, 99.0), r(5.0, 5.0)], 10.0);
        assert_eq!(out, vec![r(0.0, 2.0), r(3.0, 10.0)]);
    }

    #[test]
    fn expand_grows_clamps_and_reglues() {
        // 两侧各 +0.2s：两个原本相隔 0.1s 的段放完就重叠了 → 并回一段
        let out = expand_regions(vec![r(10.0, 12.0), r(12.1, 13.0)], 0.2, 100.0);
        assert_eq!(out, vec![r(9.8, 13.2)]);
        // 隔得远的不会因为放 0.2s 就被粘起来
        let out = expand_regions(vec![r(10.0, 12.0), r(20.0, 21.0)], 0.2, 100.0);
        assert_eq!(out, vec![r(9.8, 12.2), r(19.8, 21.2)]);
        // 越界要被夹回音频范围（0.1+0.2 有浮点尾巴，比区间而不是比字面量）
        let out = expand_regions(vec![r(0.05, 0.1), r(99.95, 100.0)], 0.2, 100.0);
        assert_eq!(out.len(), 2);
        for (got, want) in out.iter().zip([r(0.0, 0.3), r(99.75, 100.0)]) {
            assert!(
                (got.start - want.start).abs() < 1e-9 && (got.end - want.end).abs() < 1e-9,
                "got {got:?} want {want:?}"
            );
        }
    }

    #[test]
    fn frame_levels_are_relative_not_absolute() {
        // 同样的波形，整体放大 10 倍 → 电平 +20dB（这就是"动态"的含义：只看本片自己的相对电平）
        let quiet: Vec<f32> = vec![0.01; 100];
        let loud: Vec<f32> = vec![0.1; 100];
        let a = frame_levels_db(&mut |s, n| quiet[s..s + n].to_vec(), 100, 50);
        let b = frame_levels_db(&mut |s, n| loud[s..s + n].to_vec(), 100, 50);
        assert_eq!(a.len(), 2);
        assert!((b[0] - a[0] - 20.0).abs() < 0.01, "a={:?} b={:?}", a, b);
    }
}
