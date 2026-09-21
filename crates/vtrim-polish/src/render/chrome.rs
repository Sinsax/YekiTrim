//! polish::render::chrome：headless Chrome 的浏览器/标签页管理与卡死守卫。
//!
//! 入口：`ChromeRenderer`、`launch`、`create_tab`、`wait_for_ready`、`ChromeBackend`、`DEFAULT_STALL_SECS`。
//! 边界：卡死守卫是"渲染卡住、日志里一行都没有"事故的防线，别退回无超时轮询；改超时口径要连带看页面侧的 `__CAPTURE_PROGRESS` —— 见 `crates/vtrim-polish/src/render/AGENTS.md`

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

/// 页面无进展多久算卡死（秒）。页面每 30 帧回写一次进度，默认值给了 60 倍余量。
pub const DEFAULT_STALL_SECS: u64 = 180;

/// Chrome CDP 控制
pub struct ChromeRenderer;

impl ChromeRenderer {
    /// 启动 Chrome (headless)。视口尺寸跟随 `[canvas]` 推导的画布
    /// （对 canvas 抓帧无强制力，但影响页面布局/视频解码视口，方向变了要一起改）。
    pub fn launch(size: (u32, u32)) -> anyhow::Result<Browser> {
        use std::ffi::OsStr;
        let ws = format!("--window-size={},{}", size.0, size.1);
        let args = [
            OsStr::new("--headless=new"),
            OsStr::new(&ws),
            OsStr::new("--disable-dev-shm-usage"),
            OsStr::new("--autoplay-policy=no-user-gesture-required"),
            OsStr::new("--enable-gpu-rasterization"),
            OsStr::new("--enable-zero-copy"),
            // 组件更新/翻译之类的后台任务与出片无关，但会在这台机器上跑满好几秒到几分钟
            // （实测临时目录里 component crx 一直在写），一次渲染要起 6 个 Chrome，乘起来很可观。
            OsStr::new("--disable-component-update"),
            OsStr::new("--disable-features=Translate,MediaRouter,OptimizationHints"),
        ];
        let args_vec = args.to_vec();
        let mut base = LaunchOptions::default_builder();
        let mut builder = base
            .headless(true)
            .sandbox(false)
            .window_size(Some(size))
            .args(args_vec);
        // 全局配置 [chrome] path 优先（留空走 headless_chrome 自动检测）
        let global_cfg = vtrim_core::config::load_global_config();
        let custom_path = global_cfg.chrome.path.trim().to_string();
        if !custom_path.is_empty() {
            tracing::info!("使用自定义 Chrome 路径: {}", custom_path);
            builder = builder.path(Some(std::path::PathBuf::from(&custom_path)));
        }
        let browser = Browser::new(builder.build()?)?;
        Ok(browser)
    }

    /// 创建新标签页并导航到 URL
    pub fn create_tab(browser: &Browser, url: &str) -> anyhow::Result<Arc<Tab>> {
        let tab = browser.new_tab()?;
        tab.set_default_timeout(Duration::from_secs(60));
        tab.navigate_to(url)?;
        tab.wait_until_navigated()?;
        Ok(tab)
    }

    /// 等待页面初始化完成 — 检查 `__TIMELINE`。
    ///
    /// 页面里的 `showError()` 会写 `window.__ERROR`；这里轮询它并**立即**带着真实原因
    /// 退出，而不是傻等 60s 后报一句和根因无关的“GSAP/CDN”超时。
    pub fn wait_for_ready(tab: &Tab) -> anyhow::Result<()> {
        let mut last_state = String::new();
        for i in 0..300 {
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

            // 每 2s 取一次页面状态；有 __ERROR 立即退出。
            if i % 10 == 0 {
                let state = tab.evaluate(
                    r#"JSON.stringify({
                        readyState: document.readyState,
                        bodyClass: document.body.className,
                        error: window.__ERROR || '',
                        spinner: (document.getElementById('loading-spinner')||{}).textContent || '',
                        hasGSAP: typeof gsap !== 'undefined',
                        hasConfig: typeof CFG !== 'undefined',
                        cfgKeys: typeof CFG !== 'undefined' ? Object.keys(CFG).join(',') : '',
                        hasData: !!(CFG && CFG._data),
                        subCount: typeof SUB !== 'undefined' ? SUB.length : -1,
                        dmCount: typeof DM !== 'undefined' ? DM.length : -1,
                        timelineReady: typeof window.__TIMELINE !== 'undefined'
                    })"#,
                    false,
                );
                if let Ok(s) = state {
                    if let Some(v) = s.value {
                        let text = v.as_str().unwrap_or("").to_string();
                        if !text.is_empty() {
                            last_state = text.clone();
                        }
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(err) = parsed
                                .get("error")
                                .and_then(|e| e.as_str())
                                .filter(|e| !e.is_empty())
                            {
                                anyhow::bail!("Page init failed: {err}");
                            }
                            // 兜底：GSAP 连脚本都没加载出来（CDN 不可达）时尽早报清楚。
                            if i >= 75
                                && !parsed
                                    .get("hasGSAP")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false)
                            {
                                anyhow::bail!(
                                    "Page init failed: GSAP not loaded (CDN unreachable); state={text}"
                                );
                            }
                        }
                        if i % 50 == 0 {
                            tracing::warn!("Chrome 页面状态 ({}s): {}", i / 5, text);
                        }
                    }
                }
            }

            std::thread::sleep(Duration::from_millis(200));
        }

        anyhow::bail!(
            "Page failed to initialize after 60s; last state: {}",
            if last_state.is_empty() { "<no state>" } else { last_state.as_str() }
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
    /// 输出画布（`RenderContext` 的 width/height，随 `[canvas]` 推导）
    canvas: (u32, u32),
}

impl ChromeBackend {
    pub fn new(project_path: &Path, _config: &ProjectFile) -> anyhow::Result<Self> {
        let _ = project_path;
        Ok(Self {
            server_url: String::new(),
            sticker_data_json: "{}".into(),
            fps: 30,
            canvas: (1920, 1080),
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
        start_frame: usize,
        frame_count: usize,
    ) -> anyhow::Result<Vec<Vec<u8>>> {
        let b64_engine = base64::engine::general_purpose::STANDARD;
        let end_frame = start_frame + frame_count;
        tab.evaluate(&format!(
            "window.__CAPTURE_CHUNK({}, {}, {}).catch(function(e){{ window.__CAPTURE_ERROR = e.message; }})",
            start_frame, end_frame, self.fps,
        ), false)?;
        let chunk_start = std::time::Instant::now();
        // 无进展超时：页面每 30 帧回写一次 __CAPTURE_PROGRESS，正常时 1~4s 一跳；
        // 卡住（页面内的 promise 悬住、渲染进程没了）时这个循环原本会**永远转下去**，
        // 表现为"渲染长时间无反应、日志里什么都没有"。到点直接报错退出。
        let stall_limit: u64 = std::env::var("VTEDIT_STALL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_STALL_SECS);
        let mut last_pct = String::new();
        let mut last_change = std::time::Instant::now();

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
            if pct != last_pct {
                last_pct = pct.clone();
                last_change = std::time::Instant::now();
            } else if last_change.elapsed().as_secs() >= stall_limit {
                anyhow::bail!(
                    "帧 {start_frame}..{end_frame} 卡在 {}%（{}s 无进展）——页面渲染没有推进。\
                     可调小/调大 VTEDIT_STALL_SECS 复现，详见日志里的分块进度",
                    pct,
                    last_change.elapsed().as_secs()
                );
            }
            let elapsed = chunk_start.elapsed().as_secs_f64();
            // 用**帧区间**而不是块序号：4 个分块后端拿不到自己的序号（都写 0），
            // 日志里会分不清是谁在报进度；帧区间天然唯一，父进程也据此做进度聚合。
            tracing::info!("    frames {}..{}: {}%  {}s", start_frame, end_frame, pct, elapsed as usize);

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
        self.canvas = (ctx.width, ctx.height);
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

        let browser = ChromeRenderer::launch(self.canvas)?;
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
        let start_frame = frames.first().map(|f| f.index).unwrap_or(0);
        let out = self.capture_and_read(&tab, start_frame, frame_count)?;
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
