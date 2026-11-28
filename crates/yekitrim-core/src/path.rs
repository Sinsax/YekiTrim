use std::path::{Path, PathBuf};

/// 工程配置文件名（`<工程目录>/yekitrim.toml`）。
///
/// ⚠️ **唯一权威**：这个字面量以前散在 ~35 处（跨 core / service / server / cli，含
/// `yekitrim-server/src/api/projects.rs` 那条安全守卫），改名时只要漏一处就**静默**坏：
/// 写侧写 `yekitrim.toml`、读侧找 `yekitrim.toml` ⇒ 工程"打不开"或守卫误判 400。
/// **新增/修改任何读写工程配置的地方，一律引用本常量**，别再写字面量。
pub const PROJECT_CONFIG_FILE: &str = "yekitrim.toml";

/// **工程内**状态目录名（`<工程目录>/.yekitrim/`：断点续传 checkpoint、LLM 状态…）。
///
/// ⚠️ 2026-11-27 起程序数据目录**不再有**这个隐藏目录（见 [`data_dir`] 与
/// `docs/file-layout.md`）；这里只剩"每个工程一份"的语义。
/// 改这个名会让**已有工程的 checkpoint / status 文件找不到** —— 按不兼容口径这是可接受的
/// （不会迁移、不会告警），但**写侧与读侧必须同时改**，否则每次精修都白跑一遍 LLM。
pub const PROJECT_STATE_DIR_NAME: &str = ".yekitrim";

/// 数据目录覆盖用的环境变量名（见 [`data_dir`]）。
pub const DATA_DIR_ENV: &str = "YEKITRIM_DATA_DIR";

/// 程序所在目录（exe 的同级目录）。
///
/// 只读程序件（`app/`、`LICENSE` 等）与可写数据（`data/`）都以它为基准。
/// 取不到可执行文件路径时回退当前工作目录。这个函数在 HTTP handler 里被频繁调用，
/// 一旦 panic 会连累整条调用链（任务体的 panic 由 `spawn_job` 兜住、但那次任务就失败了），
/// 所以这里绝不 `expect`。
pub fn program_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// **只读程序件**的根 —— `<程序目录>/app/`（`runtime/` + `dhampir/`）。
///
/// 与 `data/` 的分界是**谁往里放东西**：`app/` 随版本发布、升级整目录替换；
/// `data/` 是用户与运行态，升级时必须保留。
pub fn app_dir() -> PathBuf {
    app_dir_in(&program_dir())
}

/// [`app_dir`] 的可注入版本（单测用）。
pub fn app_dir_in(program: &Path) -> PathBuf {
    program.join("app")
}

/// **程序数据的唯一根** —— `YEKITRIM_DATA_DIR` ?? `<程序目录>/data/`。
///
/// 2026-11-27 起所有可写状态都挂它下面，且**扁平命名**（`ledger/`、`logs/`、`models/`、
/// `bin/`、`projects/`、`shared/`、`users/`），不再有 `.yekitrim/` 这种隐藏目录。
///
/// ⚠️ **`YEKITRIM_DATA_DIR` 是唯一的口子**：`target/debug` 与 `target/release`
/// （以及换 `CARGO_TARGET_DIR`）各是一份程序目录，想共用 1 GB 级模型就把它指到同一处。
/// ⚠️ 别再退回"用户目录优先"：那条路是 2026-10 之前的设计，代价是同一个东西有两个
/// 可能的位置，实测出过"模型管理说没下载、转写却一直在用"。
pub fn data_dir() -> PathBuf {
    let override_dir = std::env::var_os(DATA_DIR_ENV).map(PathBuf::from);
    data_dir_in(override_dir.as_deref(), &program_dir())
}

/// [`data_dir`] 的可注入版本（单测用：不碰环境变量，edition 2024 下 `set_var` 是 unsafe）。
pub fn data_dir_in(override_dir: Option<&Path>, program: &Path) -> PathBuf {
    match override_dir {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => program.join("data"),
    }
}

/// 用户主目录（`USERPROFILE` → `HOME`；取不到 = None）。
///
/// ⚠️ **程序资产不再挂它下面**（2026-10-02 起）。仅存两个用途：
/// ① 识别用户目录里**遗留**的老资产（`fonts::legacy_user_dir_report`）；
/// ② 老库巡检的报数。**别拿它当资产的落点**。
pub fn user_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

// ---------------------------------------------------------------------------
// data/ 下的实名目录（每个都只有这一个拼接点）
// ---------------------------------------------------------------------------

/// 迁移标记：`<data>/layout.json`（内容 `{"version":2}`）。
pub fn layout_marker() -> PathBuf {
    layout_marker_in(&data_dir())
}

/// [`layout_marker`] 的可注入版本。
pub fn layout_marker_in(data_home: &Path) -> PathBuf {
    data_home.join("layout.json")
}

/// 当前工程 + 端口：`<data>/server.toml`。
pub fn server_toml_path() -> PathBuf {
    server_toml_path_in(&data_dir())
}

/// [`server_toml_path`] 的可注入版本。
pub fn server_toml_path_in(data_home: &Path) -> PathBuf {
    data_home.join("server.toml")
}

/// 账本与运行锁：`<data>/ledger/`。
pub fn ledger_dir() -> PathBuf {
    ledger_dir_in(&data_dir())
}

/// [`ledger_dir`] 的可注入版本。
pub fn ledger_dir_in(data_home: &Path) -> PathBuf {
    data_home.join("ledger")
}

/// 唯一日志根：`<data>/logs/`。
pub fn logs_dir() -> PathBuf {
    logs_dir_in(&data_dir())
}

/// [`logs_dir`] 的可注入版本。
pub fn logs_dir_in(data_home: &Path) -> PathBuf {
    data_home.join("logs")
}

/// ASR 模型：`<data>/models/`。
pub fn models_dir() -> PathBuf {
    models_dir_in(&data_dir())
}

/// [`models_dir`] 的可注入版本。
pub fn models_dir_in(data_home: &Path) -> PathBuf {
    data_home.join("models")
}

/// 内置下载器（yt-dlp）：`<data>/bin/`。
pub fn bin_dir() -> PathBuf {
    bin_dir_in(&data_dir())
}

/// [`bin_dir`] 的可注入版本。
pub fn bin_dir_in(data_home: &Path) -> PathBuf {
    data_home.join("bin")
}

/// 托管项目根（默认）：`<data>/projects/`。
///
/// ⚠️ 实际根可能被 `[storage] projects_root` 覆盖（见 `yekitrim_service::projects`）；
/// 这个函数只给"默认值"与迁移用。
pub fn projects_dir() -> PathBuf {
    projects_dir_in(&data_dir())
}

/// [`projects_dir`] 的可注入版本。
pub fn projects_dir_in(data_home: &Path) -> PathBuf {
    data_home.join("projects")
}

/// 公用素材库：`<data>/shared/{fonts,stickers,videos}/`。
pub fn shared_dir() -> PathBuf {
    shared_dir_in(&data_dir())
}

/// [`shared_dir`] 的可注入版本。
pub fn shared_dir_in(data_home: &Path) -> PathBuf {
    data_home.join("shared")
}

/// 可丢弃的运行期缓存：`<data>/cache/`。
///
/// 与 `shared/` 的分界：**删掉它不需要重新下载或重新配置任何东西**（丢了只会重算/重抓）。
/// 今天的住户只有音效试听缓存 `cache/sfx/`（`api/polish/sfx-preview` 落盘的那一份）。
///
/// ⚠️ 别把"用户资产"（贴纸/字体/视频/音效库本体）放到这里 —— 那些删了是要重传的。
pub fn cache_dir() -> PathBuf {
    cache_dir_in(&data_dir())
}

/// [`cache_dir`] 的可注入版本。
pub fn cache_dir_in(data_home: &Path) -> PathBuf {
    data_home.join("cache")
}

/// 账号树：`<data>/users/<uid>/`。
pub fn users_dir() -> PathBuf {
    users_dir_in(&data_dir())
}

/// [`users_dir`] 的可注入版本。
pub fn users_dir_in(data_home: &Path) -> PathBuf {
    data_home.join("users")
}

// ---------------------------------------------------------------------------
// app/ 下的只读程序件
// ---------------------------------------------------------------------------

/// 转译器 + 效果词汇表：`<program>/app/runtime/`。
pub fn runtime_dir() -> PathBuf {
    runtime_dir_in(&program_dir())
}

/// [`runtime_dir`] 的可注入版本（参数是**程序目录**，不是数据目录）。
pub fn runtime_dir_in(program: &Path) -> PathBuf {
    app_dir_in(program).join("runtime")
}

/// 出片底座：`<program>/app/dhampir/`。
pub fn dhampir_dir() -> PathBuf {
    dhampir_dir_in(&program_dir())
}

/// [`dhampir_dir`] 的可注入版本。
pub fn dhampir_dir_in(program: &Path) -> PathBuf {
    app_dir_in(program).join("dhampir")
}

/// 转译器/词汇表的**旧位置**（2026-10-09 之前）：`<程序目录>/tools/` 与 `<程序目录>/spec/`。
///
/// 只为裸换 exe 的存量安装回落；新写入一律走 [`runtime_dir`]。
pub fn legacy_runtime_dirs_in(program: &Path) -> (PathBuf, PathBuf) {
    (program.join("tools"), program.join("spec"))
}

/// 旧包里的 `runtime/`（2026-11-27 之前它在程序根，不在 `app/` 下）。
pub fn legacy_runtime_dir_in(program: &Path) -> PathBuf {
    program.join("runtime")
}

/// 旧包里的 `dhampir/`（2026-11-27 之前它在程序根，不在 `app/` 下）。
pub fn legacy_dhampir_dir_in(program: &Path) -> PathBuf {
    program.join("dhampir")
}

// ---------------------------------------------------------------------------
// 旧素材位置（只供迁移与只读回落；生产写入禁止）
// ---------------------------------------------------------------------------

/// **旧**顶层素材根（≤0.5）：`<data>/resources/`。
///
/// ⚠️ 2026-11-27 起不再创建、不再写入；启动迁移会把内容并入
/// `users/<uid>/<kind>/`，空目录删除。
pub fn legacy_resources_dir_at(data_home: &Path) -> PathBuf {
    data_home.join("resources")
}

/// [`legacy_resources_dir_at`] 的实时版本。
pub fn legacy_resources_dir() -> PathBuf {
    legacy_resources_dir_at(&data_dir())
}

/// 旧贴纸库（2026-10-09 之前）：`<data>/stickers/`。
pub fn legacy_sticker_dir_at(data_home: &Path) -> PathBuf {
    data_home.join("stickers")
}

/// 旧字体库：`<data>/resources/fonts/`（只读回落）。
pub fn legacy_font_dir_at(data_home: &Path) -> PathBuf {
    legacy_resources_dir_at(data_home).join("fonts")
}

/// 旧贴纸库：`<data>/resources/stickers/`（只读回落）。
pub fn legacy_sticker_dir_in_resources_at(data_home: &Path) -> PathBuf {
    legacy_resources_dir_at(data_home).join("stickers")
}

/// 旧视频库：`<data>/resources/videos/`（只读回落）。
pub fn legacy_video_dir_at(data_home: &Path) -> PathBuf {
    legacy_resources_dir_at(data_home).join("videos")
}

/// 旧贴纸库**该读哪个目录**：`resources/stickers/` → `<data>/stickers/` → 前者。
///
/// ⚠️ 判据是"目录存在"而不是"里面有没有东西"：用户可以先建空库再放。
pub fn legacy_sticker_dir_readable_at(data_home: &Path) -> PathBuf {
    let new = legacy_sticker_dir_in_resources_at(data_home);
    if new.is_dir() {
        return new;
    }
    let old = legacy_sticker_dir_at(data_home);
    if old.is_dir() {
        return old;
    }
    new
}

/// 旧视频库**该读哪个目录**：`resources/videos` → `resources/stickers/videos` →
/// `<data>/stickers/videos`，都没有则返回 `resources/videos`。
///
/// ⚠️ 这一档要挑"真有素材"的那个（用户手工搬迁会先建空目录、后搬文件）。
pub fn legacy_video_dir_readable_at(data_home: &Path) -> PathBuf {
    let candidates = [
        legacy_video_dir_at(data_home),
        legacy_sticker_dir_in_resources_at(data_home).join("videos"),
        legacy_sticker_dir_at(data_home).join("videos"),
    ];
    if let Some(hit) = candidates.iter().find(|p| dir_has_entries(p)) {
        return hit.clone();
    }
    candidates
        .iter()
        .find(|p| p.is_dir())
        .cloned()
        .unwrap_or_else(|| legacy_video_dir_at(data_home))
}

/// 目录存在**且里面至少有一个条目**。
fn dir_has_entries(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// 旧 API 别名（只供回落/测试；生产写入请走 owner::asset_dir）
// ---------------------------------------------------------------------------

/// 旧顶层素材根（≤0.5）—— 只读回落。
pub fn resources_dir() -> PathBuf {
    legacy_resources_dir()
}

/// [`resources_dir`] 的可注入版本。
pub fn resources_dir_at(data_home: &Path) -> PathBuf {
    legacy_resources_dir_at(data_home)
}

/// 旧贴纸库（`resources/stickers`）—— 只读回落。
pub fn sticker_dir() -> PathBuf {
    sticker_dir_at(&data_dir())
}

/// [`sticker_dir`] 的可注入版本。
pub fn sticker_dir_at(data_home: &Path) -> PathBuf {
    legacy_sticker_dir_in_resources_at(data_home)
}

/// 旧贴纸库该读哪个目录（带更老位置回落）。
pub fn sticker_dir_readable() -> PathBuf {
    sticker_dir_readable_at(&data_dir())
}

/// [`sticker_dir_readable`] 的可注入版本。
pub fn sticker_dir_readable_at(data_home: &Path) -> PathBuf {
    legacy_sticker_dir_readable_at(data_home)
}

/// 旧视频库（`resources/videos`）—— 只读回落。
pub fn video_dir() -> PathBuf {
    video_dir_at(&data_dir())
}

/// [`video_dir`] 的可注入版本。
pub fn video_dir_at(data_home: &Path) -> PathBuf {
    legacy_video_dir_at(data_home)
}

/// 旧视频库该读哪个目录（带更老位置回落）。
pub fn video_dir_readable() -> PathBuf {
    video_dir_readable_at(&data_dir())
}

/// [`video_dir_readable`] 的可注入版本。
pub fn video_dir_readable_at(data_home: &Path) -> PathBuf {
    legacy_video_dir_readable_at(data_home)
}

/// 旧字体库（`resources/fonts`）—— 只读回落。
pub fn font_dir() -> PathBuf {
    font_dir_at(&data_dir())
}

/// [`font_dir`] 的可注入版本。
pub fn font_dir_at(data_home: &Path) -> PathBuf {
    legacy_font_dir_at(data_home)
}

/// 给"人要看的/前端要回传的"路径用的显示形式：去掉 Windows 的 `\\?\` 逐字前缀
/// （canonicalize 的产物）。
///
/// 逐字前缀一旦被前端拿去做字符串拼接（`dir + '/' + name`）或塞进 URL，就会变成
/// `\\?\F:\a/b` 这种非法形式（ERROR_INVALID_NAME 123），所以**出 JSON 的路径必须过这里**。
/// `\\?\UNC\server\share` 还原成 `\\server\share`。
pub fn display(path: &Path) -> String {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_strips_verbatim_prefix() {
        assert_eq!(display(Path::new(r"\\?\F:\media\a")), r"F:\media\a");
        assert_eq!(display(Path::new(r"\\?\UNC\srv\share\a")), r"\\srv\share\a");
        assert_eq!(display(Path::new(r"F:\media\a")), r"F:\media\a");
        assert_eq!(display(Path::new("/tmp/a")), "/tmp/a");
    }

    /// **新布局没有任何隐藏状态目录**：`data/` 下全是实名目录。
    #[test]
    fn data_root_is_flat_and_has_no_hidden_state_dir() {
        let program = Path::new("/install");
        let data = data_dir_in(None, program);
        assert_eq!(data, Path::new("/install/data"));
        assert_eq!(layout_marker_in(&data), Path::new("/install/data/layout.json"));
        assert_eq!(server_toml_path_in(&data), Path::new("/install/data/server.toml"));
        assert_eq!(ledger_dir_in(&data), Path::new("/install/data/ledger"));
        assert_eq!(logs_dir_in(&data), Path::new("/install/data/logs"));
        assert_eq!(models_dir_in(&data), Path::new("/install/data/models"));
        assert_eq!(bin_dir_in(&data), Path::new("/install/data/bin"));
        assert_eq!(projects_dir_in(&data), Path::new("/install/data/projects"));
        assert_eq!(shared_dir_in(&data), Path::new("/install/data/shared"));
        assert_eq!(cache_dir_in(&data), Path::new("/install/data/cache"));
        assert_eq!(users_dir_in(&data), Path::new("/install/data/users"));
        // 正向判据：整条路径里不得再出现旧的全局 `.yekitrim`
        for p in [
            layout_marker_in(&data),
            server_toml_path_in(&data),
            ledger_dir_in(&data),
            logs_dir_in(&data),
            models_dir_in(&data),
            users_dir_in(&data),
        ] {
            assert!(
                !p.components().any(|c| c.as_os_str() == ".yekitrim"),
                "data 下不该再有 .yekitrim: {}",
                p.display()
            );
        }
    }

    /// 只读程序件在 `app/` 下，与 `data/` 分开。
    #[test]
    fn program_payload_lives_under_app() {
        let program = Path::new("/install");
        assert_eq!(app_dir_in(program), Path::new("/install/app"));
        assert_eq!(runtime_dir_in(program), Path::new("/install/app/runtime"));
        assert_eq!(dhampir_dir_in(program), Path::new("/install/app/dhampir"));
        // 旧包位置仍能算出来（裸换 exe 回落）
        assert_eq!(legacy_runtime_dir_in(program), Path::new("/install/runtime"));
        assert_eq!(legacy_dhampir_dir_in(program), Path::new("/install/dhampir"));
        assert_eq!(
            legacy_runtime_dirs_in(program),
            (PathBuf::from("/install/tools"), PathBuf::from("/install/spec"))
        );
    }

    /// **存量升级不丢库**：旧位置仍可算出/回落到。
    #[test]
    fn legacy_asset_locations_are_still_computable() {
        let data = Path::new("/install/data");
        assert_eq!(legacy_resources_dir_at(data), Path::new("/install/data/resources"));
        assert_eq!(legacy_font_dir_at(data), Path::new("/install/data/resources/fonts"));
        assert_eq!(
            legacy_sticker_dir_in_resources_at(data),
            Path::new("/install/data/resources/stickers")
        );
        assert_eq!(
            legacy_video_dir_at(data),
            Path::new("/install/data/resources/videos")
        );
        assert_eq!(legacy_sticker_dir_at(data), Path::new("/install/data/stickers"));
    }

    #[test]
    fn legacy_readable_dirs_fall_back_to_old_locations() {
        let tmp = std::env::temp_dir().join(format!(
            "_yekitrim_path_fallback_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // 什么都没有 ⇒ 返回新（resources）位置
        assert_eq!(
            legacy_sticker_dir_readable_at(&tmp),
            legacy_sticker_dir_in_resources_at(&tmp)
        );
        assert_eq!(
            legacy_video_dir_readable_at(&tmp),
            legacy_video_dir_at(&tmp)
        );

        // 只有更老的位置 ⇒ 回落
        let old_stickers = legacy_sticker_dir_at(&tmp);
        std::fs::create_dir_all(old_stickers.join("ysyk")).unwrap();
        assert_eq!(legacy_sticker_dir_readable_at(&tmp), old_stickers);
        let old_videos = old_stickers.join("videos");
        std::fs::create_dir_all(&old_videos).unwrap();
        assert_eq!(legacy_video_dir_readable_at(&tmp), old_videos);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// **资产只有一个位置**：`YEKITRIM_DATA_DIR` 是唯一口子，不掺用户目录。
    #[test]
    fn data_dir_override_is_the_only_escape_hatch() {
        let program = Path::new("/install");
        assert_eq!(data_dir_in(None, program), Path::new("/install/data"));
        // 空串等同没设
        assert_eq!(data_dir_in(Some(Path::new("")), program), Path::new("/install/data"));
        // 显式覆盖 ⇒ 用覆盖值（debug/release 共用模型靠它）
        let shared = Path::new("/shared/yekitrim-data");
        assert_eq!(data_dir_in(Some(shared), program), shared);
        assert_eq!(models_dir_in(shared), Path::new("/shared/yekitrim-data/models"));
        // 覆盖值里也不该被塞 `.yekitrim`
        assert!(!models_dir_in(shared).to_string_lossy().contains(".yekitrim"));
    }
}
