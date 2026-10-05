//! 预览代理：把源压成 **720p / GOP10** 的"找帧友好"副本，**只给预览那条腿读**。
//!
//! # 为什么要有它
//!
//! 预览卡在**定位**上：源是长 GOP 的成片时，每次 seek 都要从关键帧重解一整段
//! （实测 1080p60 / GOP60 **165ms/帧**，预览 5fps）。把关键帧变密就解决了这件事：
//!
//! | 形态 | 体积（10s 1080p60 切片实测） | 随机定位中位 |
//! |---|---|---|
//! | 源（GOP 长） | 12.3MB（9.7Mbps） | **165ms** |
//! | 720p GOP10 CRF20 | **6.0MB（0.52× 源）** | **18ms** |
//! | 720p 全 I 帧 | 25.0MB（2.2× 源） | 7ms |
//! | 1080p 全 I 帧 | 48.1MB（4.2× 源） | ~40ms |
//!
//! ⇒ 取 **GOP10**：全 I 帧只为把 18ms 再压到 7ms，就要 2.2–4.2 倍的**磁盘**，
//! 而本地与服务器两种模式下**容量都是约束**（带宽/延迟在本地根本不是问题）。
//!
//! # 三条纪律
//!
//! 1. **保持源帧率**：不传 `-r`，用 `-fps_mode passthrough`。时基一变，工程里所有帧号
//!    映射都会错（转译器按**实际媒体**探测时基，见 `tools/polish-to-dhampir.mjs::probeMedia`）。
//! 2. **音轨 `-c:a copy`**：预览的声音就来自这条 `<video>`，重编码没意义；容器不兼容时
//!    退一次 AAC，**不因为声音丢掉整条代理**。
//! 3. **绝不阻断 init**：任何失败只 warn 并返回 `None`（预览回落源）。
//!
//! # 只给预览，不给渲染
//!
//! 出片永远用**源**（`dhampir::prepare_project` 的 `video` 参数只有预览那条腿传）。
//! 预览那条腿把"实际媒体"换成代理时，转译器会**按代理**探测时基/分辨率写进工程 ——
//! 所以层的 `scale`/`FIT` 也是按 720p 算的，画面不会被放大歪。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 代理文件名。**派生固定名**（不是配置字段）—— 配置里多一个可写路径就多一个路径逃逸面。
pub const PREVIEW_PROXY_FILE: &str = "_proxy_720.mp4";

/// 目标高度上限；源本来就比这矮就**不放大**（放大只会更大更慢）。
///
/// 想看"分辨率一点也不降"的档：`VTEDIT_PROXY_HEIGHT=1080` —— 实测那档体积 ≈ 0.95× 源、
/// 41.2dB/0.982（几乎无可察），代价是磁盘不再减半、定位也比 720p 慢一倍。
const PROXY_MAX_HEIGHT: u32 = 720;
/// CRF：实测 720p GOP10 CRF20 = 38.8dB / SSIM 0.974（同分辨率口径，肉眼基本无感）。
const PROXY_CRF: u32 = 20;
/// 关键帧间隔（帧）。10 帧 ⇒ 60fps 素材上 0.17 秒一个关键帧，最坏解 10 帧。
const PROXY_GOP: u32 = 10;

/// 代理被显式关掉？（`VTEDIT_NO_PROXY=1` —— CI/测试与"服务器配额紧"时用）
pub fn proxy_disabled() -> bool {
    matches!(
        std::env::var("VTEDIT_NO_PROXY").ok().as_deref(),
        Some("1") | Some("true") | Some("yes")
    )
}

fn max_height() -> u32 {
    std::env::var("VTEDIT_PROXY_HEIGHT")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|h| *h >= 180)
        .unwrap_or(PROXY_MAX_HEIGHT)
}

fn mtime(p: &Path) -> Option<SystemTime> {
    p.metadata().ok()?.modified().ok()
}

/// 代理此刻**能不能用**：存在、且不比源旧。返回它在片段目录里的**相对名**（给转译器 `--video`）。
///
/// 新鲜度按 mtime 判：源被重切过（`clip.mp4` 变新）⇒ 代理作废，回落源。
pub fn preview_video_rel(clip_dir: &Path, source_rel: &str) -> Option<String> {
    let proxy = clip_dir.join(PREVIEW_PROXY_FILE);
    if !proxy.is_file() {
        return None;
    }
    let src = clip_dir.join(source_rel);
    match (mtime(&proxy), mtime(&src)) {
        (Some(p), Some(s)) if p < s => None,
        _ => Some(PREVIEW_PROXY_FILE.to_string()),
    }
}

/// 给 `polish.toml` 所在目录算出"预览该读哪份视频"：有可用代理就返回代理的相对名，否则 `None`。
///
/// 自己读 `polish.toml`（而不是让调用方传源名）：调用点只有一个（预览 API），
/// 而"源是谁"的权威就在工程配置里。
pub fn preview_video_for(clip_dir: &Path) -> Option<String> {
    if proxy_disabled() {
        return None;
    }
    let config = crate::project::load_project(clip_dir).ok()?;
    preview_video_rel(clip_dir, &config.source.video)
}

/// `polish init` 用：确保有一份**不过期**的预览代理。失败一律不阻断调用方。
pub fn ensure_preview_proxy(clip_dir: &Path, source_rel: &str) -> Option<PathBuf> {
    match ensure_preview_proxy_with(clip_dir, source_rel, !proxy_disabled()) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("  ⚠ 预览代理生成失败（预览将直接读源，功能不受影响）：{e:#}");
            None
        }
    }
}

/// [`ensure_preview_proxy`] 的可注入版本（`enabled=false` ⇒ 直接跳过；单测用，不吃环境变量）。
pub fn ensure_preview_proxy_with(
    clip_dir: &Path,
    source_rel: &str,
    enabled: bool,
) -> anyhow::Result<Option<PathBuf>> {
    if !enabled {
        return Ok(None);
    }
    let src = clip_dir.join(source_rel);
    if !src.is_file() {
        return Ok(None); // 没有源就没什么可做的（调用方那边本来就有一条"素材缺失"的提示）
    }
    let proxy = clip_dir.join(PREVIEW_PROXY_FILE);

    // **幂等**：不比源旧的代理直接用。重复 init 不该反复压一遍。
    if proxy.is_file() {
        if let (Some(p), Some(s)) = (mtime(&proxy), mtime(&src)) {
            if p >= s {
                tracing::info!(
                    "  · 预览代理已是最新：{}（{}）",
                    PREVIEW_PROXY_FILE,
                    human_size(proxy.metadata().map(|m| m.len()).unwrap_or(0))
                );
                return Ok(Some(proxy));
            }
        }
    }

    let src_str = src.to_string_lossy().to_string();
    let height = crate::render::pipeline::get_video_size(&src_str)
        .map(|(_, h)| h)
        .unwrap_or(0);
    let cap = max_height();
    // **不放大**：源比上限矮就不写 `-vf`（写 `scale=-2:720` 会把小源放大，更大更慢）
    let scale: Option<String> = if height > cap {
        Some(format!("scale=-2:{cap}"))
    } else {
        None
    };

    let run = |audio: &[&str]| -> anyhow::Result<std::process::Output> {
        let mut c = yekitrim_core::process::cmd("ffmpeg");
        c.args(["-v", "error", "-nostdin", "-y", "-i"]).arg(&src);
        if let Some(vf) = &scale {
            c.args(["-vf", vf]);
        }
        c.args(["-map", "0:v:0", "-map", "0:a?"]);
        c.args([
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            &PROXY_CRF.to_string(),
            "-g",
            &PROXY_GOP.to_string(),
            "-keyint_min",
            &PROXY_GOP.to_string(),
            "-sc_threshold",
            "0",
            // 保持源帧率与时间戳：重复/丢帧的自动同步会改时间轴
            "-fps_mode",
            "passthrough",
        ]);
        c.args(audio);
        c.args(["-movflags", "+faststart"]).arg(&proxy);
        c.output()
            .map_err(|e| anyhow::anyhow!("跑 ffmpeg 失败（ffmpeg 不在 PATH？）：{e}"))
    };

    let out = run(&["-c:a", "copy"])?;
    if !out.status.success() {
        // 音轨 copy 不了（源音频放不进 mp4，例如 Vorbis）⇒ 退一次 AAC。
        // ⚠️ **不能因为声音丢掉整条代理**：代理的价值在画面找帧。
        tracing::debug!(
            "  预览代理：音轨 copy 失败，退 AAC 重试（{}）",
            String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("")
        );
        let _ = std::fs::remove_file(&proxy);
        let retry = run(&["-c:a", "aac", "-b:a", "128k"])?;
        if !retry.status.success() {
            let _ = std::fs::remove_file(&proxy);
            anyhow::bail!(
                "ffmpeg 两次都失败：{}",
                String::from_utf8_lossy(&retry.stderr).trim()
            );
        }
    }

    let src_len = src.metadata().map(|m| m.len()).unwrap_or(0);
    let proxy_len = proxy.metadata().map(|m| m.len()).unwrap_or(0);
    let ratio = if src_len > 0 {
        format!(" / 源的 {:.0}%", proxy_len as f64 / src_len as f64 * 100.0)
    } else {
        String::new()
    };
    tracing::info!(
        "  ✓ 预览代理：{}（{}，GOP{} CRF{}，{}x{}→上限{}{}）",
        PREVIEW_PROXY_FILE,
        human_size(proxy_len),
        PROXY_GOP,
        PROXY_CRF,
        crate::render::pipeline::get_video_size(&src_str)
            .map(|(w, _)| w.to_string())
            .unwrap_or_else(|| "?".into()),
        height,
        cap,
        ratio
    );
    Ok(Some(proxy))
}

fn human_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= MB {
        format!("{:.1}MB", bytes as f64 / MB)
    } else {
        format!("{:.0}KB", bytes as f64 / 1024.0)
    }
}
