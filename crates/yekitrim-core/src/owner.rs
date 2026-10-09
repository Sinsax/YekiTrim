//! 数据归属（`Owner`）—— **身份 → 数据位置**的唯一通道。
//!
//! ## 为什么有这个模块
//!
//! 这个程序有两种形态，**两种都要能用**：
//!
//! ① **单人 / local**（托盘、CLI）：本机唯一使用者，数据就在程序目录下，今天的样子；
//! ② **多用户**（`yekitrim server` 正常模式）：每个账号有自己的素材库与配置。
//!
//! 两种形态**共用同一套业务逻辑**，差别只允许出现在「**身份从哪来**」这一个点上
//! （单人 = [`Owner::Local`]，多用户 = [`Owner::User`]）。业务代码只问
//! 「这份数据属于谁」，**不许自己拼数据目录**。
//!
//! ## 三条硬规矩（违反即判负）
//!
//! 1. **只有一条通道**：不许再出现 `data_dir().join("resources")` 这类自己拼的路径，
//!    一律经 [`owner_root`] / [`asset_dir`] / [`config_path`]。
//! 2. **不许"候选位置 + 逐级回落"**：`docs/file-layout.md` 记着那次事故 ——
//!    同一个东西有两个可能的位置，于是必然有地方只查一处，实测 CLI 与 WebUI 各看一份
//!    字体库、**界面上看不出任何异常**。一个 owner、一个位置、一个判据。
//! 3. **不许业务分支**：一旦出现 `if local { A } else { B }` 的业务分支，两个形态就漂移了。
//!
//! ## 关键设计：`Owner` 只回答"是谁"，不回答"在哪"
//!
//! 路径一律由本模块的函数算。这样将来真要落成用户级目录时，
//! **只改这里的几行，业务代码一行不动**。

use std::path::{Path, PathBuf};

use super::path;

/// 身份**跨进程**传递的环境变量名。
///
/// ## 为什么必须有它
///
/// 出片 / 封面 / 预览都走**子进程**（`yekitrim polish <sub> <dir>`，见 `render/AGENTS.md`：
/// 分钟级的重活不能跑在服务进程里）。子进程要自己解析字体库、贴纸库的位置 ——
/// 拿不到父进程的 `Owner`，它就只会看全局那一份 ⇒
/// **多用户下 A 导入的字体，B 出片时会被用上**（实测的失效形态：不报错、只是字体不对）。
///
/// ## 为什么用环境变量，而不是命令行参数
///
/// - `polish` 有 render / cover / preview / init / process… **每一条都得带上**它。
///   加参数要动 `clap` 的每个子命令 + 每处 `extra` 拼装；环境变量在 `Command` 上**一处生效**。
/// - 子进程还会再拉起孙进程（ffmpeg、底座 Dhampir），环境变量**自动继承**，命令行不会。
/// - 本仓已有先例：`YEKITRIM_DATA_DIR` 就是同一种"路径面的进程级开关"（见 [`path::data_dir`]）。
///
/// ⚠️ 认不出的值**回落 `Local` 并打 warn**（不报错）：出片直接失败比"用错了库"更难查，
/// 而这个值理论上只可能来自手改环境变量或旧版父进程。
pub const OWNER_ENV: &str = "YEKITRIM_OWNER";

impl Owner {
    /// 从 [`OWNER_ENV`] 读身份；没设 = [`Owner::Local`]（CLI 与单人的常态）。
    pub fn from_env() -> Self {
        Self::from_env_value(std::env::var(OWNER_ENV).ok().as_deref())
    }

    /// [`from_env`] 的可注入版本（单测用：edition 2024 下 `set_var` 是 `unsafe`，
    /// 与 `path::data_dir_in` 同一条理由）。
    pub fn from_env_value(raw: Option<&str>) -> Self {
        let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Owner::Local;
        };
        if raw == "local" {
            return Owner::Local;
        }
        match raw.parse::<i64>() {
            Ok(uid) if uid > 0 => Owner::User(uid),
            // ⚠️ 认不出**不静默**：多用户下"出片用了全局字体库"这种症状
            // 在日志里一个字都看不到就太难查了。
            _ => {
                tracing::warn!("{OWNER_ENV}={raw} 认不出（只认 local 或正整数 uid），按单人形态继续");
                Owner::Local
            }
        }
    }

    /// 写进子进程环境的字符串（与 [`from_env_value`] 互逆）。
    pub fn as_env_value(&self) -> String {
        match self {
            Owner::Local => "local".to_string(),
            Owner::User(uid) => uid.to_string(),
        }
    }
}

/// 数据归属 —— 决定"这份数据属于谁"。
///
/// 单人部署下恒为 [`Owner::Local`]，此时**所有解析结果等于 2026-10 现状的位置**
/// （`<数据目录>/resources/*`、`<数据目录>/.yekitrim/config.toml`），
/// 因而**单人升级到本版零迁移**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    /// 本机唯一使用者：托盘 / CLI / `server --local-no-auth`。
    Local,
    /// 多用户模式下的某个账号（`users.id`）。
    User(i64),
}

impl Owner {
    /// 多用户模式下的身份；`uid` 必须是账本里真实存在的 `users.id`。
    pub fn user(uid: i64) -> Self {
        Owner::User(uid)
    }

    /// 是不是单人形态。
    pub fn is_local(&self) -> bool {
        matches!(self, Owner::Local)
    }

    /// `Owner::User` 里的 uid（单人形态 = `None`）。
    pub fn uid(&self) -> Option<i64> {
        match self {
            Owner::Local => None,
            Owner::User(uid) => Some(*uid),
        }
    }
}

/// 素材种类 —— [`asset_dir`] 的入参。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetKind {
    /// `<…>/resources/fonts/`
    Fonts,
    /// `<…>/resources/stickers/`
    Stickers,
    /// `<…>/resources/videos/`
    Videos,
}

impl AssetKind {
    /// 目录名（与 `path.rs` 里的拼写**必须一致**）。
    pub fn dir_name(&self) -> &'static str {
        match self {
            AssetKind::Fonts => "fonts",
            AssetKind::Stickers => "stickers",
            AssetKind::Videos => "videos",
        }
    }
}

/// 该 owner 的**数据根**。
///
/// - [`Owner::Local`] → [`path::data_dir`]（**现状不变**）；
/// - [`Owner::User`] → `<数据目录>/users/<uid>/`。
pub fn owner_root(owner: Owner) -> PathBuf {
    owner_root_at(owner, &path::data_dir())
}

/// [`owner_root`] 的可注入版本（单测用：不碰环境变量）。
pub fn owner_root_at(owner: Owner, data_home: &Path) -> PathBuf {
    match owner {
        Owner::Local => data_home.to_path_buf(),
        Owner::User(uid) => data_home.join("users").join(uid.to_string()),
    }
}

/// 该 owner 的**素材库根**。
///
/// - [`Owner::Local`] → `<数据目录>/resources/`（**现状不变**）；
/// - [`Owner::User`] → `<数据目录>/users/<uid>/resources/`。
pub fn resources_dir(owner: Owner) -> PathBuf {
    resources_dir_at(owner, &path::data_dir())
}

/// [`resources_dir`] 的可注入版本（单测用）。
pub fn resources_dir_at(owner: Owner, data_home: &Path) -> PathBuf {
    owner_root_at(owner, data_home).join("resources")
}

/// 某一类素材的库目录。
///
/// ⚠️ [`Owner::Local`] 的三个结果**必须逐字节等于** `path::{font_dir, sticker_dir, video_dir}`
/// 今天的返回值 —— 这是"单人零迁移"的判据，有单测钉住。
pub fn asset_dir(owner: Owner, kind: AssetKind) -> PathBuf {
    asset_dir_at(owner, kind, &path::data_dir())
}

/// [`asset_dir`] 的可注入版本（单测用）。
pub fn asset_dir_at(owner: Owner, kind: AssetKind, data_home: &Path) -> PathBuf {
    resources_dir_at(owner, data_home).join(kind.dir_name())
}

/// 贴纸库**该读哪个目录** —— 单人形态带回存量回落，多用户不带。
///
/// ## 为什么多用户**不**回落
///
/// `path::sticker_dir_readable()` 那套回落是为了"2026-10-09 之前的老安装别突然变空"：
/// 老位置 `<data>/stickers/` → 新位置 `<data>/resources/stickers/`。
/// 那是**这台机器**的历史，只对 [`Owner::Local`] 成立。
///
/// 用户自己的树（`users/<uid>/resources/stickers`）是**新长出来的**，从来没有老位置 ——
/// 给它套一套回落等于凭空制造"两个可能的位置"，正是本模块明令禁止的第 2 条。
pub fn sticker_dir_readable(owner: Owner) -> PathBuf {
    sticker_dir_readable_at(owner, &path::data_dir())
}

/// [`sticker_dir_readable`] 的可注入版本（单测用）。
pub fn sticker_dir_readable_at(owner: Owner, data_home: &Path) -> PathBuf {
    match owner {
        // 单人：**逐字沿用现状那条**（含回落），保证零回归。
        Owner::Local => path::sticker_dir_readable_at(data_home),
        Owner::User(_) => asset_dir_at(owner, AssetKind::Stickers, data_home),
    }
}

/// 视频素材库**该读哪个目录** —— 单人形态带回存量回落，多用户不带
/// （理由同 [`sticker_dir_readable`]）。
pub fn video_dir_readable(owner: Owner) -> PathBuf {
    video_dir_readable_at(owner, &path::data_dir())
}

/// [`video_dir_readable`] 的可注入版本（单测用）。
pub fn video_dir_readable_at(owner: Owner, data_home: &Path) -> PathBuf {
    match owner {
        Owner::Local => path::video_dir_readable_at(data_home),
        Owner::User(_) => asset_dir_at(owner, AssetKind::Videos, data_home),
    }
}

/// 该 owner 的**状态目录**（`<…>/.yekitrim/`：配置、server.toml…）。
///
/// ⚠️ **账本（`users` / `sessions` / `invites` 表）永远全局、不走这里** ——
/// 它是**身份的来源**，按用户分 = 登录前先得知道你是谁 = 循环依赖。
/// 见 `yekitrim-service::db::db_path`。
pub fn state_dir(owner: Owner) -> PathBuf {
    state_dir_at(owner, &path::data_dir())
}

/// [`state_dir`] 的可注入版本（单测用）。
pub fn state_dir_at(owner: Owner, data_home: &Path) -> PathBuf {
    owner_root_at(owner, data_home).join(path::STATE_DIR_NAME)
}

/// 该 owner 的**全局配置文件**（`config.toml`）。
pub fn config_path(owner: Owner) -> PathBuf {
    config_path_at(owner, &path::data_dir())
}

/// [`config_path`] 的可注入版本（单测用）。
pub fn config_path_at(owner: Owner, data_home: &Path) -> PathBuf {
    state_dir_at(owner, data_home).join("config.toml")
}

/// 把**单人时期**的存量（`<data>/resources/*` 与 `<data>/.yekitrim/{config,llm}.toml`）
/// 归到某个账号名下 —— **一次性、幂等、不复制内容**。
///
/// ## 为什么要它
///
/// 单人形态（[`Owner::Local`]）的数据就在 `<数据>/resources/` 与 `<数据>/.yekitrim/`；
/// 而多用户形态下 [`Owner::User`] 解析到 `<数据>/users/<uid>/…`。不做这一步的话，
/// 一台已用很久的机器一开多用户，**新账号看到的是空素材库与默认配置**，
/// 用户会以为"我的东西全丢了"。
///
/// ## 语义：**移动**，不是复制
///
/// 素材可能有几个 GB（`op.mp4`、贴纸包），**复制一份是错的**（占双倍磁盘、
/// 且两份从此分叉）。所以是 `rename`；跨盘时 `rename` 会失败，那时**如实报错、
/// 保持原状**，让调用方（或用户手工）决定 —— 绝不悄悄退化成"复制一半"。
///
/// ⚠️ **只在目标不存在时动手**（幂等）：`users/<uid>/resources/` 已经在了就整段跳过，
/// 跑第二遍是 no-op。这也是"用户自己已经把东西放好了"的情况。
///
/// ⚠️ **`Owner::Local` 不参与**：它就是源头，没有可迁的。
///
/// ## ⚠️ 判据是"**源头有没有东西**"，不是"用户树在不在"（2026-10-10 修的真 bug）
///
/// 第一版写的是 `if !data_home.join("users").is_dir() { return }`，理由是
/// "还没有任何用户树 ⇒ 还没进过多用户模式"。**那是错的**：`users/` 目录只在
/// **第一次按 `Owner::User` 写过东西时**才被创建，而升级后的第一件事正是启动 server ——
/// 那一刻 `users/` 根本不存在，于是迁移**静默不跑**，用户的素材永远留在
/// `<data>/resources/`，首个管理员登录后看到一个**空素材库**。
/// 实测复现：全新数据目录 + 老布局素材 → 启动 → 日志里"归入首个管理员"一次都没出现。
///
/// 现在只问一件事：**源头有东西吗**。没有就是不迁（全新安装天然满足）；
/// 有就迁。目标已存在仍然会跳过（幂等，见下）。
pub fn migrate_legacy_assets_to(owner: Owner, data_home: &Path) -> std::io::Result<Vec<PathBuf>> {
    let Owner::User(_) = owner else {
        return Ok(Vec::new()); // 单人形态：数据本来就在那儿，无事可做
    };
    let legacy_resources = path::resources_dir_at(data_home);

    let mut moved = Vec::new();

    // ① 素材：<data>/resources/* → <data>/users/<uid>/resources/*
    if legacy_resources.is_dir() {
        for kind in [AssetKind::Fonts, AssetKind::Stickers, AssetKind::Videos] {
            let from = legacy_resources.join(kind.dir_name());
            if !from.is_dir() {
                continue;
            }
            let to = asset_dir_at(owner, kind, data_home);
            if to.exists() {
                continue; // 幂等：已经有目标了，不动
            }
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&from, &to)?;
            moved.push(to);
        }
    }

    // ② 配置：<data>/.yekitrim/{config,llm}.toml → <data>/users/<uid>/.yekitrim/
    let legacy_state = path::yekitrim_home_at(data_home);
    let dest_state = state_dir_at(owner, data_home);
    for name in ["config.toml", "llm.toml"] {
        let from = legacy_state.join(name);
        if !from.is_file() {
            continue;
        }
        let to = dest_state.join(name);
        if to.exists() {
            continue; // 幂等
        }
        std::fs::create_dir_all(&dest_state)?;
        std::fs::rename(&from, &to)?;
        moved.push(to);
    }

    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️ **单人零迁移的判据**：`Owner::Local` 的每个解析结果必须**逐字节等于**
    /// `path::*` 今天的返回值。这条一旦红了，就说明有人悄悄挪了单人的数据位置。
    #[test]
    fn local_owner_resolves_to_exactly_todays_locations() {
        let home = Path::new("/data");
        for (owner_kind, want) in [
            (AssetKind::Fonts, path::font_dir_at(home)),
            (AssetKind::Stickers, path::sticker_dir_at(home)),
            (AssetKind::Videos, path::video_dir_at(home)),
        ] {
            assert_eq!(
                asset_dir_at(Owner::Local, owner_kind, home),
                want,
                "单人形态下 {:?} 的位置必须与现状一致",
                owner_kind
            );
        }
        assert_eq!(owner_root_at(Owner::Local, home), home);
        assert_eq!(resources_dir_at(Owner::Local, home), path::resources_dir_at(home));
        assert_eq!(state_dir_at(Owner::Local, home), path::yekitrim_home_at(home));
    }

    /// 多用户形态：三类素材都落在 `<data>/users/<uid>/resources/<kind>/`。
    #[test]
    fn user_owner_gets_its_own_tree() {
        let home = Path::new("/data");
        let uid = 7;
        for (kind, sub) in [
            (AssetKind::Fonts, "fonts"),
            (AssetKind::Stickers, "stickers"),
            (AssetKind::Videos, "videos"),
        ] {
            assert_eq!(
                asset_dir_at(Owner::User(uid), kind, home),
                Path::new("/data/users/7/resources").join(sub)
            );
        }
        assert_eq!(owner_root_at(Owner::User(uid), home), Path::new("/data/users/7"));
        assert_eq!(
            state_dir_at(Owner::User(uid), home),
            Path::new("/data/users/7/.yekitrim")
        );
        assert_eq!(
            config_path_at(Owner::User(uid), home),
            Path::new("/data/users/7/.yekitrim/config.toml")
        );
    }

    /// 硬规矩 1 的机器判据：**同一个 owner 只解析出一个位置**。
    ///
    /// 这条是用来钉住"不许候选列表/逐级回落"的 —— 解析函数是纯函数，
    /// 结果只由 `(owner, kind, data_home)` 决定，**不看磁盘上有没有东西**。
    /// 老那套 `*_readable()` 的"存在就选它"正是被禁掉的形状。
    #[test]
    fn resolution_is_pure_and_has_no_candidate_list() {
        let home = Path::new("/data");
        // 连算两次必须完全一样（没有"第一次算出来、第二次因为建了目录就变了"）。
        for kind in [AssetKind::Fonts, AssetKind::Stickers, AssetKind::Videos] {
            let a = asset_dir_at(Owner::User(3), kind, home);
            let b = asset_dir_at(Owner::User(3), kind, home);
            assert_eq!(a, b);
            // 且只可能有一个位置：结果必然是 owner 根的直接后代。
            assert!(
                a.starts_with(owner_root_at(Owner::User(3), home)),
                "素材库必须落在该 owner 的根之内"
            );
        }
    }

    /// 不同 owner 的库**必须互不重叠** —— 这是"多用户互不干扰"的结构前提。
    #[test]
    fn different_owners_never_share_a_directory() {
        let home = Path::new("/data");
        let kinds = [AssetKind::Fonts, AssetKind::Stickers, AssetKind::Videos];
        let mut seen: Vec<(PathBuf, &str)> = Vec::new();
        for (label, owner) in [
            ("local", Owner::Local),
            ("u1", Owner::User(1)),
            ("u2", Owner::User(2)),
        ] {
            for kind in kinds {
                let d = asset_dir_at(owner, kind, home);
                // 不许有人已经占了这个位置。
                if let Some((_, other)) = seen.iter().find(|(p, _)| *p == d) {
                    panic!("{label} 的 {kind:?} 与 {other} 撞在 {d:?}");
                }
                seen.push((d, label));
            }
        }
    }

    /// 身份跨进程往返：`as_env_value` → `from_env_value` 必须还原同一个 owner。
    ///
    /// 这条是"子进程与父进程看同一个库"的**唯一**根据 —— 它两边一旦漂移，
    /// 症状是**多用户下出片用了别人的字体库**，而且不报错。
    #[test]
    fn owner_env_value_roundtrips() {
        for owner in [Owner::Local, Owner::User(1), Owner::User(7), Owner::User(i64::MAX)] {
            let v = owner.as_env_value();
            assert_eq!(
                Owner::from_env_value(Some(&v)),
                owner,
                "{owner:?} 经环境变量往返后变了"
            );
        }
    }

    /// 没设 / 空串 / 认不出 ⇒ 一律回落单人形态（**不报错**：出片失败更难查）。
    #[test]
    fn owner_env_falls_back_to_local_when_missing_or_bogus() {
        assert_eq!(Owner::from_env_value(None), Owner::Local);
        assert_eq!(Owner::from_env_value(Some("")), Owner::Local);
        assert_eq!(Owner::from_env_value(Some("   ")), Owner::Local);
        assert_eq!(Owner::from_env_value(Some("local")), Owner::Local);
        // 认不出的值 —— 包括 uid 不许为 0/负数（`users.id` 从 1 起）
        for bad in ["abc", "0", "-3", "1.5", "uid7"] {
            assert_eq!(
                Owner::from_env_value(Some(bad)),
                Owner::Local,
                "{bad} 应回落单人形态"
            );
        }
        // 两侧空白照常认
        assert_eq!(Owner::from_env_value(Some(" 42 ")), Owner::User(42));
    }

    /// 临时根目录（core **没有** dev-dependencies，`tempfile` 不在依赖表里 ——
    /// 与 `fonts.rs` / `download.rs` 的测试同一套做法：`std::env::temp_dir()` + 唯一名）。
    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yekitrim-owner-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 迁移把存量**搬到**该账号名下，而不是复制 —— 复制会让两份从此分叉、还占双倍磁盘。
    #[test]
    fn migration_moves_legacy_assets_into_the_users_tree() {
        let tmp = temp_root("move");
        let home = tmp.as_path();

        // 造出"单人时期"的存量。
        let legacy_stickers = path::sticker_dir_at(home);
        std::fs::create_dir_all(&legacy_stickers).unwrap();
        std::fs::write(legacy_stickers.join("pack.toml"), b"x").unwrap();
        let legacy_fonts = path::font_dir_at(home);
        std::fs::create_dir_all(&legacy_fonts).unwrap();
        std::fs::write(legacy_fonts.join("a.ttf"), b"font").unwrap();
        let legacy_state = path::yekitrim_home_at(home);
        std::fs::create_dir_all(&legacy_state).unwrap();
        std::fs::write(legacy_state.join("llm.toml"), b"[llm]\napi_key = \"k\"\n").unwrap();
        // ⚠️ **刻意不建 `users/7`** —— 真实升级路径上它就是不存在的：
        // `users/` 只在第一次按 `Owner::User` 写过东西时才出现，而升级后第一件事
        // 正是启动 server。第一版迁移代码用"users/ 在不在"当判据，于是**静默不跑**
        // （实测复现过）。这条测试现在钉住那个场景。

        let moved = migrate_legacy_assets_to(Owner::User(7), home).unwrap();
        assert!(!moved.is_empty(), "应当真的搬了东西");

        // 到了目标位置……
        let new_fonts = asset_dir_at(Owner::User(7), AssetKind::Fonts, home);
        assert!(new_fonts.join("a.ttf").is_file(), "字体应已搬到用户树");
        let new_llm = state_dir_at(Owner::User(7), home).join("llm.toml");
        assert!(new_llm.is_file(), "llm.toml 应已搬到用户树");
        // ……而**源头不再留着**（= 真搬，不是复制）。
        assert!(!legacy_fonts.join("a.ttf").exists(), "搬运后源头不该还有同一份");
        assert!(!legacy_state.join("llm.toml").exists(), "搬运后源头不该还有同一份");
    }

    /// 幂等：第二遍是 no-op，且**不会**拿存量覆盖用户自己已经放好的东西。
    #[test]
    fn migration_is_idempotent_and_never_clobbers_the_target() {
        let tmp = temp_root("idem");
        let home = tmp.as_path();
        // 同上：刻意不建 `users/7`，走真实升级路径。
        // 用户已经在自己的库里放了东西。
        let own = asset_dir_at(Owner::User(7), AssetKind::Fonts, home);
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(own.join("mine.ttf"), b"mine").unwrap();
        // 同时老位置也有东西。
        let legacy = path::font_dir_at(home);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("old.ttf"), b"old").unwrap();

        let moved = migrate_legacy_assets_to(Owner::User(7), home).unwrap();
        assert!(moved.is_empty(), "目标已存在时不该动任何东西");
        assert!(own.join("mine.ttf").is_file(), "用户自己的东西必须原样留着");
        assert!(legacy.join("old.ttf").is_file(), "存量也不该被删（人没让删）");

        // 再跑一遍仍然没事（幂等）。
        let again = migrate_legacy_assets_to(Owner::User(7), home).unwrap();
        assert!(again.is_empty());
    }

    /// 单人形态**不参与迁移** —— 它就是源头。
    #[test]
    fn migration_is_a_noop_for_the_local_owner() {
        let tmp = temp_root("local");
        let home = tmp.as_path();
        let legacy = path::font_dir_at(home);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("a.ttf"), b"font").unwrap();

        let moved = migrate_legacy_assets_to(Owner::Local, home).unwrap();
        assert!(moved.is_empty());
        assert!(legacy.join("a.ttf").is_file(), "单人形态一个字节都不该动");
    }
}
