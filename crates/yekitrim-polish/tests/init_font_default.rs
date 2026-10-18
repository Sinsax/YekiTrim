//! **`polish init` 的字幕字体来自字体库的「默认位」**。
//!
//! 判据钉的是用户 2026-10-06 报的那条：「每次切出切片，字体都不是字体库设置的默认字体」。
//!
//! 根因：`cmd_init` 走 `ProjectFile::example()` → `SubtitleStyle::default()` →
//! `render_core::types::default_font()`，那一支返回的是**写死的** `"Noto Sans SC"`，
//! 从不查字体库 ⇒ 用户设的默认位对新切出来的片段完全无效。
//! ⚠️ 而剪切收尾**会对每个新片段自动跑一次 init**
//! （`yekitrim_service::core_pipeline::init_polish_for_clip`），所以每个新片段都中招。
//!
//! 这套判据**不依赖 ffmpeg**（只写配置、不解析素材）：`cmd_init` 里的 `cmd_parse`
//! 与预览代理两段失败都只 warn，不影响 `polish.toml` 的落盘。这样门禁在任何机器上都跑得动。

use std::path::{Path, PathBuf};

/// 造一个临时字体库 + 一支带元数据的字体（`<root>/<id>/font.ttf` + `FONT.toml`）。
fn put_font(root: &Path, id: &str, family: &str, emoji: bool, is_default: bool) {
    let d = root.join(id);
    std::fs::create_dir_all(&d).expect("建字体目录");
    std::fs::write(d.join("font.ttf"), b"not-a-real-font").expect("写字体文件");
    let meta = format!(
        "id = \"{id}\"\ndisplay_name = \"{id}\"\nfamily = \"{family}\"\n\
         emoji = {emoji}\nis_default = {is_default}\n"
    );
    std::fs::write(d.join("FONT.toml"), meta).expect("写元数据");
}

/// 跑一次真 init，返回落盘的 `[style.subtitle].font`。
///
/// ⚠️ `cmd_init` 从**全局数据目录**（`YEKITRIM_DATA_DIR` / `path::font_dir()`）读字体库，
/// 所以这里必须先摆好库再调 —— 见 `with_data_dir`。
async fn init_and_read_font(dir: &Path) -> String {
    std::fs::create_dir_all(dir).expect("建工程目录");
    yekitrim_polish::commands::cmd_init(dir)
        .await
        .expect("init 应当成功");
    let raw = std::fs::read_to_string(dir.join("polish.toml")).expect("读 polish.toml");
    let cfg: toml::Value = toml::from_str(&raw).expect("polish.toml 应当能解析");
    cfg["style"]["subtitle"]["font"]
        .as_str()
        .expect("font 应当是个字符串")
        .to_string()
}

/// 独占一个数据目录跑闭包（`YEKITRIM_DATA_DIR` 是进程级环境变量 —— 测试必须串行）。
///
/// ⚠️ 用**一把全局锁**而不是 `std::env::set_var` 裸调：`cargo test` 默认多线程，
/// 两个测试同时改这个变量会互相把字体库指到对方目录上（症状是间歇性红）。
/// 本文件只有两个 `#[tokio::test]`，串行代价可忽略。
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct DataDir {
    _guard: std::sync::MutexGuard<'static, ()>,
    root: PathBuf,
    old: Option<std::ffi::OsString>,
}

impl DataDir {
    fn new(tag: &str) -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("vt-initfont-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("建数据目录");
        let old = std::env::var_os("YEKITRIM_DATA_DIR");
        // SAFETY: 上面那把锁保证同一时刻只有一个测试改它。
        unsafe { std::env::set_var("YEKITRIM_DATA_DIR", &root) };
        Self { _guard: guard, root, old }
    }

    /// 字体库落点 = **该 owner 的**字体库目录（`owner::asset_dirs` 的约定）。
    ///
    /// ⚠️ 2026-10-16 起资产按归属落 `users/<uid>/resources/fonts`，顶层 `resources/` 已不再是库
    /// —— 这条测试原来写死顶层，于是「默认字体没生效」的**假红**（`cmd_init` 用的是 `fonts::dir_for`）。
    fn font_dir(&self) -> PathBuf {
        yekitrim_core::fonts::dir_for(yekitrim_core::owner::Owner::Local)
    }
}

impl Drop for DataDir {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => unsafe { std::env::set_var("YEKITRIM_DATA_DIR", v) },
            None => unsafe { std::env::remove_var("YEKITRIM_DATA_DIR") },
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// **主判据**：库里设过默认 ⇒ 新工程的 `font` 就是那支的**内部族名**。
#[tokio::test]
async fn init_takes_the_library_default_font() {
    let data = DataDir::new("hit");
    let fonts = data.font_dir();
    put_font(&fonts, "WenKai", "霞鹜文楷 轻便版", false, true);
    put_font(&fonts, "Other", "Other Family", false, false);

    let project = data.root.join("clip");
    let font = init_and_read_font(&project).await;

    assert_eq!(
        font, "霞鹜文楷 轻便版",
        "新工程的字幕字体必须来自字体库的默认位（用户报的就是这条没生效）。\
         实得 {font:?} —— 若仍是 \"Noto Sans SC\"，说明 `cmd_init` 里那段 \
         `default_text_family` 又被摘掉了"
    );
}

/// **零回归**：库里没设过默认 ⇒ 保持出厂值（自带 Noto Sans SC），别自己编一支出来。
///
/// 这条同时是"空库回落态"的判据：`render_core::default_font()` 与
/// `SYSTEM_FONT_STACK` 首项必须同名，改这里会波及存量工程。
#[tokio::test]
async fn init_keeps_factory_font_when_library_has_no_default() {
    let data = DataDir::new("none");
    let fonts = data.font_dir();
    put_font(&fonts, "Other", "Other Family", false, false);

    let project = data.root.join("clip");
    let font = init_and_read_font(&project).await;

    assert_eq!(
        font, "Noto Sans SC",
        "库里没人设默认位时，应当保持出厂值不动"
    );
}
