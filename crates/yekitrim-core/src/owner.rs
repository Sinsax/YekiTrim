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
                assert!(
                    !seen.iter().any(|(p, _)| *p == d),
                    "{label} 的 {kind:?} 与 {} 撞在 {d:?}",
                    seen.iter().find(|(p, _)| *p == d).unwrap().1
                );
                seen.push((d, label));
            }
        }
    }
}
