use std::path::{Path, PathBuf};

/// 工程配置文件名（`<工程目录>/yekitrim.toml`）。
///
/// ⚠️ **唯一权威**：这个字面量以前散在 ~35 处（跨 core / service / server / cli，含
/// `yekitrim-server/src/api/projects.rs` 那条安全守卫），改名时只要漏一处就**静默**坏：
/// 写侧写 `yekitrim.toml`、读侧找 `yekitrim.toml` ⇒ 工程"打不开"或守卫误判 400。
/// **新增/修改任何读写工程配置的地方，一律引用本常量**，别再写字面量。
pub const PROJECT_CONFIG_FILE: &str = "yekitrim.toml";

/// 程序状态目录名（`<data_dir>/.yekitrim/`：全局配置、模型、server.toml、账本…）。
///
/// ⚠️ **与 [`PROJECT_STATE_DIR_NAME`] 是两个不同的东西**，别混：
/// 这个是**全局**的（一处），那个是**每工程一份**的（`<工程目录>/.yekitrim/`）。
/// 两者刻意同名同改，但**拼接点不同**（一个挂 `data_dir()`、一个挂工程根）。
pub const STATE_DIR_NAME: &str = ".yekitrim";

/// 工程内状态目录名（`<工程目录>/.yekitrim/`：断点续传 checkpoint、LLM 状态…）。
///
/// ⚠️ 它**不是**全局状态目录（见 [`STATE_DIR_NAME`]）。改这个名会让**已有工程的
/// checkpoint / status 文件找不到** —— 按不兼容口径这是可接受的（不会迁移、不会告警），
/// 但**写侧与读侧必须同时改**，否则每次精修都白跑一遍 LLM。
pub const PROJECT_STATE_DIR_NAME: &str = ".yekitrim";

/// 数据目录覆盖用的环境变量名（见 [`data_dir`]）。
pub const DATA_DIR_ENV: &str = "YEKITRIM_DATA_DIR";

/// 程序所在目录：`.yekitrim/`（配置、模型、server.toml）与 `stickers/` 的父目录。
///
/// 取不到可执行文件路径时回退当前工作目录。这两个函数在 HTTP handler 里被频繁调用，
/// 一旦 panic 会连累整条调用链（任务体的 panic 由 `spawn_job` 兜住、但那次任务就失败了），
/// 所以这里绝不 `expect`。
pub(crate) fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// **程序资产的根目录**（用户口径 2026-10-02：*所有内容围绕程序目录来，不要用户目录*）。
///
/// 默认 = [`exe_dir`]，即 `target/debug/`（debug 构建）或安装目录（发行版）——
/// 字体库、ASR 模型、配置全都挂它下面。
///
/// ⚠️ **`YEKITRIM_DATA_DIR` 是唯一的口子**，留着它是为了应付一个具体的痛点：
/// `target/debug` 与 `target/release`（以及换 `CARGO_TARGET_DIR`）各是一份程序目录，
/// 那样 **1.1 GB 的 ASR 模型会被复制两份**。想共用就把这个变量指到同一处。
///
/// ⚠️ 别再退回"用户目录优先"：那条路是 2026-10 之前的设计（当年为了躲开构建目录漂移
/// 才把资产挂到 `~/.yekitrim/`），代价是**同一个东西有了两个可能的位置**，于是界面上出过
/// "模型管理说没下载、实际转写一直在用"（`api/config.rs` 记着那次错位）。
/// 资产只有一个位置，就没有"只查了一处"的 bug。
pub fn data_dir() -> PathBuf {
    std::env::var_os(DATA_DIR_ENV)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(exe_dir)
}

/// [`data_dir`] 的可注入版本（单测用：不碰环境变量，edition 2024 下 `set_var` 是 unsafe）。
pub fn data_dir_in(override_dir: Option<&Path>, exe_home: &Path) -> PathBuf {
    match override_dir {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => exe_home.to_path_buf(),
    }
}

/// 用户主目录（`USERPROFILE` → `HOME`；取不到 = None）。
///
/// ⚠️ **程序资产不再挂它下面**（2026-10-02 起，见 [`data_dir`]）。仅存两个用途：
/// ① 识别用户目录里**遗留**的老资产（`fonts::legacy_user_dir_report`）；
/// ② 老库巡检的报数。**别拿它当资产的落点**。
pub fn user_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// 程序状态目录（配置、模型、server.toml）—— 挂在 [`data_dir`] 下的 `.yekitrim/`。
pub fn yekitrim_home() -> PathBuf {
    data_dir().join(STATE_DIR_NAME)
}

/// [`yekitrim_home`] 的可注入版本（单测用：不碰环境变量）。
/// 与 [`data_dir_in`] 同一形状 —— 拼规则只有这一处，测试直接验它。
pub fn yekitrim_home_at(data_dir: &Path) -> PathBuf {
    data_dir.join(STATE_DIR_NAME)
}

/// **用户素材库的根** —— `<数据目录>/resources/`（贴纸、视频、字体都在它下面）。
///
/// ⚠️ **2026-10-09 收拢**：这三类曾经分散在 `<数据目录>/stickers/`（含它下面的 `videos/`
/// 子目录）与 `<数据目录>/resources/fonts/`，即"同类资产住两个不同名字的根"。
/// 现在统一到 `resources/` 下按**素材种类**分子目录 —— 贴纸与视频是**并列**的两类，
/// 视频不再是贴纸的子目录。
///
/// 与 [`data_dir`] 同一条规矩：**只有一个位置**，`YEKITRIM_DATA_DIR` 是唯一的口子。
pub fn resources_dir() -> PathBuf {
    resources_dir_at(&data_dir())
}

/// [`resources_dir`] 的可注入版本（单测用：不碰环境变量）。
pub fn resources_dir_at(data_home: &Path) -> PathBuf {
    data_home.join("resources")
}

/// 贴纸库路径 —— `<数据目录>/resources/stickers/`。
///
/// ⚠️ **老位置 `<数据目录>/stickers/` 仍然读**（见 [`sticker_dir_readable`]）：
/// 存量安装里那个目录里有用户自己的贴纸包，改名不迁移就会让"贴纸库空了"。
/// **写一律往新位置**，读两个位置都认。
pub fn sticker_dir() -> PathBuf {
    sticker_dir_at(&data_dir())
}

/// [`sticker_dir`] 的可注入版本（单测用）。
pub fn sticker_dir_at(data_home: &Path) -> PathBuf {
    resources_dir_at(data_home).join("stickers")
}

/// 贴纸库的**老位置**（2026-10-09 之前）：`<数据目录>/stickers/`。
///
/// 只为**读回落**存在。新位置一旦有内容就以新位置为准。
pub fn legacy_sticker_dir_at(data_home: &Path) -> PathBuf {
    data_home.join("stickers")
}

/// 贴纸库**该读哪个目录**：新位置存在就用它，否则回落老位置，都没有则返回新位置
/// （调用方据此报"库为空"，而不是报一个不存在的路径）。
///
/// ⚠️ **判据是"目录存在"而不是"目录里有东西"**：用户可以先建个空库再放。
/// 用"有没有内容"当判据会让"新建的空库"被老位置顶掉 —— 那是个只会出现在迁移期的怪状态。
pub fn sticker_dir_readable() -> PathBuf {
    sticker_dir_readable_at(&data_dir())
}

/// [`sticker_dir_readable`] 的可注入版本（单测用）。
pub fn sticker_dir_readable_at(data_home: &Path) -> PathBuf {
    let new = sticker_dir_at(data_home);
    if new.is_dir() {
        return new;
    }
    let old = legacy_sticker_dir_at(data_home);
    if old.is_dir() {
        return old;
    }
    new
}

/// 视频素材库路径 —— `<数据目录>/resources/videos/`。
///
/// ⚠️ **2026-10-09 从 `stickers/videos/` 挪出来**：它是**独立的一类素材**，
/// 挂在贴纸库下面有两个坏处 ——（a）名字骗人；（b）`scan_packages` 会把 `videos/`
/// 当成一个贴纸包扫出来，今天只靠"包内 `files` 全非图片 ⇒ 被丢掉"**碰巧**没污染列表。
/// 挪出来之后这条隐患从"碰巧"变成"结构上不可能"。
///
/// 老位置仍读（存量库），见 [`video_dir_readable`]。
pub fn video_dir() -> PathBuf {
    video_dir_at(&data_dir())
}

/// [`video_dir`] 的可注入版本（单测用）。
pub fn video_dir_at(data_home: &Path) -> PathBuf {
    resources_dir_at(data_home).join("videos")
}

/// 视频库的**该读哪个目录**：新位置 → 老 `<data>/resources/stickers/videos` →
/// 更老 `<data>/stickers/videos`，都没有则返回新位置。
///
/// ⚠️ 中间那一档是必须的：`resources/stickers/videos` 是"贴纸库刚挪进 `resources/`
/// 但视频还没挪"那一步的中间态 —— 只认头尾两档会让那一刻的视频库变空。
pub fn video_dir_readable() -> PathBuf {
    video_dir_readable_at(&data_dir())
}

/// [`video_dir_readable`] 的可注入版本（单测用）。
pub fn video_dir_readable_at(data_home: &Path) -> PathBuf {
    let candidates = [
        video_dir_at(data_home),
        sticker_dir_at(data_home).join("videos"),
        legacy_sticker_dir_at(data_home).join("videos"),
    ];
    candidates
        .iter()
        .find(|p| p.is_dir())
        .cloned()
        .unwrap_or_else(|| video_dir_at(data_home))
}

/// **程序自带的运行时资产**目录 —— `<数据目录>/runtime/`（转译器 + 效果词汇表）。
///
/// ⚠️ 与 `resources/` 的区别是**谁往里放东西**：`resources/` 是**用户素材**
/// （用户可以增删贴纸/视频/字体），`runtime/` 是**随版本发布的只读交付物**
/// （用户不该动）。发布包只装后者。
///
/// 老位置是两个分开的目录 `<数据目录>/tools/` 与 `<数据目录>/spec/`，仍作回落读。
pub fn runtime_dir() -> PathBuf {
    runtime_dir_at(&data_dir())
}

/// [`runtime_dir`] 的可注入版本（单测用）。
pub fn runtime_dir_at(data_home: &Path) -> PathBuf {
    data_home.join("runtime")
}

/// 老位置：`<数据目录>/tools/`（转译器）与 `<数据目录>/spec/`（词汇表）。
///
/// 返回 `(老 tools, 老 spec)`，供两个消费点各自回落。
pub fn legacy_runtime_dirs_at(data_home: &Path) -> (PathBuf, PathBuf) {
    (data_home.join("tools"), data_home.join("spec"))
}

/// 字体库路径 —— **程序目录下**：`<数据目录>/resources/fonts/`。
///
/// ⚠️ **2026-10-02 由"用户固定目录优先"改成程序目录**（用户口径：*不要用户目录*）。
/// 历史：这条路径曾经优先取 `~/.yekitrim/fonts`（为了躲开 `target/debug` 与
/// `target/cl/debug` 各存一份的问题），再用 `resources/fonts/` 当老安装的回落态。
/// 代价是**同一个库有两个可能的位置**，而"用户目录那份"与"程序目录那份"内容一旦分叉，
/// CLI 与 WebUI 就各看各的（实测：CLI 报 `target/debug/resources/fonts`、
/// 网页用 `~/.yekitrim/fonts`，两边字体列表完全不同）。
///
/// 现在只有一处：`<数据目录>/resources/fonts/`。
/// 相关读写见 [`crate::fonts`]。
pub fn font_dir() -> PathBuf {
    font_dir_at(&data_dir())
}

/// [`font_dir`] 的可注入版本（单测用）。
pub fn font_dir_at(data_home: &Path) -> PathBuf {
    data_home.join("resources").join("fonts")
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

    /// 2026-10-09 的布局：`resources/{stickers,videos,fonts}` + `runtime/`。
    ///
    /// 钉住**纯拼接**（`.is_dir()` 那层回落测不到静态路径，见下一条用例）。
    #[test]
    fn assets_live_under_resources_and_runtime_is_separate() {
        let d = Path::new("/install");
        assert_eq!(resources_dir_at(d), Path::new("/install/resources"));
        assert_eq!(font_dir_at(d), Path::new("/install/resources/fonts"));
        assert_eq!(sticker_dir_at(d), Path::new("/install/resources/stickers"));
        assert_eq!(video_dir_at(d), Path::new("/install/resources/videos"));
        assert_eq!(runtime_dir_at(d), Path::new("/install/runtime"));
        // ⚠️ 视频**不是**贴纸的子目录（挪库的全部意义就是这个）
        assert!(!video_dir_at(d).starts_with(sticker_dir_at(d)));
        // 老位置仍可算出来（回落要用）
        assert_eq!(legacy_sticker_dir_at(d), Path::new("/install/stickers"));
        assert_eq!(
            legacy_runtime_dirs_at(d),
            (PathBuf::from("/install/tools"), PathBuf::from("/install/spec"))
        );
    }

    /// **存量升级不丢库**：新位置不存在时，两个 `*_readable` 必须回落到老位置。
    ///
    /// 为什么值得单测：`sticker_dir_readable()` 的判据是**目录存在与否**，
    /// 写错（比如永远返回新位置）的表现是"升级后贴纸库/视频库全空"——
    /// 而**界面只会显示"库是空的"，不报错**，用户会以为自己的素材被删了。
    /// 这条同时钉住"中间态"：`resources/stickers/videos`（贴纸挪了、视频还没挪）。
    #[test]
    fn readable_dirs_fall_back_to_legacy_locations() {
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

        // ① 什么都没有 ⇒ 返回**新位置**（调用方据此报"库为空"，而不是一个瞎编的路径）
        assert_eq!(sticker_dir_readable_at(&tmp), sticker_dir_at(&tmp));
        assert_eq!(video_dir_readable_at(&tmp), video_dir_at(&tmp));

        // ② 只有老位置 ⇒ 回落到它（**这条就是"存量不丢"**）
        let old_stickers = legacy_sticker_dir_at(&tmp);
        std::fs::create_dir_all(old_stickers.join("ysyk")).unwrap();
        assert_eq!(sticker_dir_readable_at(&tmp), old_stickers);
        let old_videos = old_stickers.join("videos");
        std::fs::create_dir_all(&old_videos).unwrap();
        assert_eq!(video_dir_readable_at(&tmp), old_videos);

        // ③ 新位置一建出来就**立刻**以新位置为准（不看内容多少，只看存在）
        let new_stickers = sticker_dir_at(&tmp);
        std::fs::create_dir_all(&new_stickers).unwrap();
        assert_eq!(sticker_dir_readable_at(&tmp), new_stickers);

        // ④ 中间态：贴纸已挪进 resources，视频还在贴纸库底下 ⇒ 视频回落命中中间那一档
        let mid_videos = new_stickers.join("videos");
        std::fs::create_dir_all(&mid_videos).unwrap();
        assert_eq!(video_dir_readable_at(&tmp), mid_videos);
        // 新位置也建出来 ⇒ 两档都以新位置为准
        std::fs::create_dir_all(video_dir_at(&tmp)).unwrap();
        assert_eq!(video_dir_readable_at(&tmp), video_dir_at(&tmp));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// **资产只有一个位置**：默认在程序目录，`YEKITRIM_DATA_DIR` 是唯一的口子。
    ///
    /// 回归的是 2026-10-02 的口径切换（*所有内容围绕程序目录来，不要用户目录*）：
    /// 字体库曾经优先取 `~/.yekitrim/fonts`，导致 CLI 与 WebUI 各看一份库（实测两边
    /// 列出的字体完全不同）。现在 `font_dir` 里**不再有任何用户目录**参与。
    #[test]
    fn font_dir_lives_under_data_dir_no_user_home_involved() {
        let exe = Path::new("/install");
        // 没有覆盖 ⇒ 程序目录
        assert_eq!(data_dir_in(None, exe), Path::new("/install"));
        assert_eq!(font_dir_at(&data_dir_in(None, exe)), Path::new("/install/resources/fonts"));
        // 空串等同没设（环境变量为空时别把资产写到根目录去）
        assert_eq!(data_dir_in(Some(Path::new("")), exe), Path::new("/install"));
        // 显式覆盖 ⇒ 用覆盖值（debug/release 共用一份 1.1 GB 模型靠它）
        let shared = Path::new("/shared/yekitrim-data");
        assert_eq!(data_dir_in(Some(shared), exe), shared);
        assert_eq!(font_dir_at(shared), Path::new("/shared/yekitrim-data/resources/fonts"));
        // ⚠️ 判据是"字体库在**程序目录**下、不掺用户目录"。要断言的是**正向**那条 ——
        //    当年出事的是"字体库优先取 ~/.yekitrim/fonts"，程序目录与用户目录各看一份库。
        //    改写名时这里曾被机械替换成"不包含 `.yekitrim/fonts`"，那句话恒真、等于没断言。
        let d = font_dir_at(&data_dir_in(None, exe));
        assert_eq!(d, Path::new("/install/resources/fonts"));
        assert!(!d.to_string_lossy().contains("/.yekitrim"));
    }

    /// ⚠️ 状态目录就一个名字：`.yekitrim`。**旧 `.yekitrim` 一律不再被创建**（无兼容期）。
    #[test]
    fn state_dir_uses_the_new_name() {
        assert_eq!(STATE_DIR_NAME, ".yekitrim");
        let exe = Path::new("/install");
        assert_eq!(yekitrim_home_at(exe), Path::new("/install/.yekitrim"));
        // 判据是"不再写旧目录"（不是"路径里出现过 `yekitrim` 这个子串" —— 新名里本来就没有它）
        assert!(!yekitrim_home_at(exe).to_string_lossy().contains("/.yekitrim"));
    }
}
