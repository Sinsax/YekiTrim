use headless_chrome::{Browser, LaunchOptions, Tab};
use std::sync::Arc;

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
        tab.set_default_timeout(std::time::Duration::from_secs(15));
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
            if let Ok(val) = result {
                if val.value.map(|v| v.as_bool().unwrap_or(false)).unwrap_or(false) {
                    return Ok(());
                }
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

            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        anyhow::bail!("Page failed to initialize after 20s — check GSAP CDN reachability and data/ files exist")
    }

}
