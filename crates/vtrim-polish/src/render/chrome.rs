use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use base64::Engine;
use headless_chrome::{Browser, LaunchOptions, Tab};

use crate::project::ProjectFile;

use super::backend::FrameRenderer;
use super::context::RenderContext;
use super::frame::TimedFrame;

/// Chrome CDP 控制
pub struct ChromeRenderer;

impl ChromeRenderer {
    /// 启动 Chrome (headless)
    pub fn launch() -> anyhow::Result<Browser> {
        use std::ffi::OsStr;
        let builder = LaunchOptions::default_builder()
            .headless(true)
            .sandbox(false)
            .window_size(Some((1920, 1080)))
            .args(vec![
                OsStr::new("--headless=new"),
                OsStr::new("--window-size=1920,1080"),
                OsStr::new("--disable-dev-shm-usage"),
                OsStr::new("--autoplay-policy=no-user-gesture-required"),
                OsStr::new("--enable-gpu-rasterization"),
                OsStr::new("--enable-zero-copy"),
            ])
            .build()?;
        let browser = Browser::new(builder)?;
        Ok(browser)
    }

    /// 创建新标签页并导航到 URL
    pub fn create_tab(browser: &Browser, url: &str) -> anyhow::Result<Arc<Tab>> {
        let tab = browser.new_tab()?;
        tab.set_default_timeout(Duration::from_secs(15));
        tab.navigate_to(url)?;
        tab.wait_until_navigated()?;
        Ok(tab)
    }

    /// 等待页面初始化完成 — 检查 __TIMELINE
    pub fn wait_for_ready(tab: &Tab) -> anyhow::Result<()> {
        for i in 0..100 {
            let result = tab.evaluate(
                "typeof window.__TIMELINE !== 'undefined' && window.__TIMELINE !== null",
                false,
            );
            if let Ok(val) = result
                && val
                    .value
                    .map(|v| v.as_bool().unwrap_or(false))
                    .unwrap_or(false)
            {
                return Ok(());
            }

            if i == 15 || i == 40 {
                let state = tab.evaluate(
                    r#"JSON.stringify({
                        readyState: document.readyState,
                        bodyClass: document.body.className,
                        hasGSAP: typeof gsap !== 'undefined',
                        hasConfig: typeof CFG !== 'undefined',
                        cfgKeys: typeof CFG !== 'undefined' ? Object.keys(CFG).join(',') : '',
                        fetchCount: typeof SUB !== 'undefined' ? SUB.length : -1,
                        timelineReady: typeof window.__TIMELINE !== 'undefined'
                    })"#,
                    false,
                );
                if let Ok(s) = state {
                    eprintln!("  ⏳ Page state ({}s): {:?}", i / 5, s.value);
                }
            }

            std::thread::sleep(Duration::from_millis(200));
        }

        anyhow::bail!(
            "Page failed to initialize after 20s — check GSAP CDN reachability and data/ files exist"
        )
    }
}

/// 封装现有 Chrome CDP 批量渲染逻辑的 `FrameRenderer`。
///
/// 保持与旧 `render_chunk_sync` 相同行为：
/// - `render_chunk` 一次性发送全部 JPEG → `__CAPTURE_CHUNK` → 轮询 → `__GET_FRAMES`。
pub struct ChromeBackend {
    server_url: String,
    sticker_data_json: String,
    fps: usize,
}

impl ChromeBackend {
    pub fn new(project_path: &Path, _config: &ProjectFile) -> anyhow::Result<Self> {
        let _ = project_path;
        Ok(Self {
            server_url: String::new(),
            sticker_data_json: "{}".into(),
            fps: 30,
        })
    }

    fn send_frames(&self, tab: &Tab, jpegs: &[Vec<u8>]) -> anyhow::Result<()> {
        let b64_engine = base64::engine::general_purpose::STANDARD;
        let batch_size = 50;
        for (bi, chunk) in jpegs.chunks(batch_size).enumerate() {
            let batch_b64: Vec<String> = chunk.iter().map(|d| b64_engine.encode(d)).collect();
            let json = serde_json::to_string(&batch_b64)?;
            tab.evaluate(&format!("window.__LOAD_FRAMES({})", json), false)?;
            let pct =
                ((bi * batch_size + chunk.len()) as f64 / jpegs.len() as f64 * 100.0) as usize;
            tracing::debug!("    chrome: sent frames {:.0}%", pct);
        }
        Ok(())
    }

    fn capture_and_read(
        &self,
        tab: &Tab,
        frame_count: usize,
        chunk_idx: usize,
    ) -> anyhow::Result<Vec<Vec<u8>>> {
        let b64_engine = base64::engine::general_purpose::STANDARD;
        let start_frame = 0;
        let end_frame = frame_count;
        tab.evaluate(&format!(
            "window.__CAPTURE_CHUNK({}, {}, {}).catch(function(e){{ window.__CAPTURE_ERROR = e.message; }})",
            start_frame, end_frame, self.fps,
        ), false)?;
        let chunk_start = std::time::Instant::now();

        loop {
            let done_val = tab.evaluate("window.__CAPTURE_DONE === true", false)?;
            let done = done_val.value.and_then(|v| v.as_bool()).unwrap_or(false);
            if done {
                break;
            }

            let err_val = tab.evaluate("window.__CAPTURE_ERROR", false)?;
            let err = err_val
                .value
                .and_then(|v| v.as_str().map(|s| s.to_string()));
            if let Some(e) = err
                && !e.is_empty()
            {
                anyhow::bail!("Render error: {}", e);
            }

            let pct_val = tab.evaluate("window.__CAPTURE_PROGRESS || '0'", false)?;
            let pct = pct_val
                .value
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            let elapsed = chunk_start.elapsed().as_secs_f64();
            tracing::info!("    chunk#{}: {}%  {}s", chunk_idx, pct, elapsed as usize);

            std::thread::sleep(Duration::from_millis(1500));
        }

        let batch_size_read = 20;
        let mut offset = 0;
        let mut out = Vec::with_capacity(frame_count);
        while offset < frame_count {
            let js = format!(
                "window.__GET_FRAMES({}, {})",
                offset,
                offset + batch_size_read
            );
            let result = tab.evaluate(&js, false)?;
            let json_str = result
                .value
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "[]".into());
            let frames: Vec<String> = serde_json::from_str(&json_str).unwrap_or_default();
            for b64 in &frames {
                out.push(b64_engine.decode(b64.as_bytes())?);
            }
            offset += batch_size_read;
        }
        Ok(out)
    }
}

impl FrameRenderer for ChromeBackend {
    fn name(&self) -> &str {
        "chrome"
    }

    fn prepare(&mut self, ctx: &RenderContext) -> Result<()> {
        self.server_url = ctx.server_url.clone().unwrap_or_default();
        self.sticker_data_json = ctx.sticker_data_json.clone();
        self.fps = ctx.fps;
        if self.server_url.is_empty() {
            anyhow::bail!("ChromeBackend requires server_url in RenderContext");
        }
        Ok(())
    }

    fn render_chunk(&mut self, frames: &[TimedFrame]) -> Result<Vec<Vec<u8>>> {
        let jpegs: Vec<Vec<u8>> = frames
            .iter()
            .map(|f| f.frame.jpeg.clone().unwrap_or_default())
            .collect();
        let frame_count = frames.len();
        if frame_count == 0 {
            return Ok(vec![]);
        }

        let browser = ChromeRenderer::launch()?;
        let tab = ChromeRenderer::create_tab(&browser, &self.server_url)?;
        tab.set_default_timeout(Duration::from_secs(600));
        ChromeRenderer::wait_for_ready(&tab)?;

        tab.evaluate("window.__CAPTURE_RESET()", false)?;
        if !self.sticker_data_json.is_empty() && self.sticker_data_json != "{}" {
            tab.evaluate(
                &format!("window.__EMOJI_DATA = {}", self.sticker_data_json),
                false,
            )?;
        }

        self.send_frames(&tab, &jpegs)?;
        let out = self.capture_and_read(&tab, frame_count, 0)?;
        if out.len() != frame_count {
            anyhow::bail!(
                "Chrome returned {} frames, expected {}",
                out.len(),
                frame_count
            );
        }
        Ok(out)
    }
}
