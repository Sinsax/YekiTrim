//! 数据归属（`Owner`）—— **身份 → 数据位置**的唯一通道。
//!
//! ## 口径（2026-10-10 定稿）：**只有一种布局 —— 多用户**
//!
//! 「单人 / local」**不是另一套目录**，它就是 **admin 这个账号**：
//! 托盘与 `--local-no-auth` 注入的合成身份是 admin，于是它们的数据落在
//! `users/<首个管理员>/` 下 —— 与"admin 在 server 里登录"**逐字节相同**。
//!
//! ```text
//! <程序目录>/
//! ├── app/                随版本发布的只读交付物（runtime + dhampir）
//! └── data/
//!     ├── server.toml     当前工程 + 端口
//!     ├── ledger/         账本 + 运行锁
//!     ├── logs/           唯一日志根
//!     ├── models/         ASR 模型
//!     ├── bin/            内置下载器
//!     ├── projects/       托管项目（可被 [storage] projects_root 覆盖）
//!     ├── shared/         公用素材库 {fonts,stickers,videos}
//!     └── users/<uid>/    config.toml + llm.toml + {fonts,stickers,videos}
//! ```
//!
//! **没有隐藏的 `.yekitrim/`，也没有 `resources/` 这一层。**
//! 完整结构与"谁是谁"见 `docs/file-layout.md`。
//!
//! ⚠️ **托盘与"admin 登录"必须解析到同一处**：它们本来就是同一个人的两种进入方式。
//! 以前托盘落 `<数据>/resources/`、admin 登录落 `<数据>/users/1/`，于是"同一个我"
//! 在两个地方各有一套素材 —— 这正是 2026-10-10「贴纸构建了但没识别」的根
//! （构建按单人库写 A、列表按 `User(1)` 读 B）。
//!
//! 业务代码只问「这份数据属于谁」，**不许自己拼数据目录**。
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
/// ⚠️ **两种取值都落进 `users/<uid>/`**（2026-10-10 起）：`Local` 只是
/// "**还没查过账本**"的那一档，解析时会被替换成当前管理员。
/// 见 [`set_local_admin`] 与 [`resolve`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    /// 托盘 / CLI / `server --local-no-auth` —— **uid 待解析**（= 当前管理员）。
    Local,
    /// 某个具体账号（`users.id`）。
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
    /// `<…>/fonts/`
    Fonts,
    /// `<…>/stickers/`
    Stickers,
    /// `<…>/videos/`
    Videos,
    /// `<…>/sfx/` —— **用户导入的音效包**（`<包名>/` 一个子目录 = 一个包）。
    ///
    /// ⚠️ 与内建预设包（编译期 `assets/sfx_catalog.json`）是两回事：
    /// 预设包不在磁盘上、不可删；这里的包全是用户自己传的、可删。
    Sfx,
}

impl AssetKind {
    /// 目录名（与 `path.rs` 里的拼写**必须一致**）。
    pub fn dir_name(&self) -> &'static str {
        match self {
            AssetKind::Fonts => "fonts",
            AssetKind::Stickers => "stickers",
            AssetKind::Videos => "videos",
            AssetKind::Sfx => "sfx",
        }
    }
}

/// 该 owner 的**数据根**。
///
/// - [`Owner::Local`] → `<数据目录>/users/<当前管理员>/`（**与 `User(admin)` 相同**）；
/// - [`Owner::User`] → `<数据目录>/users/<uid>/`。
pub fn owner_root(owner: Owner) -> PathBuf {
    owner_root_at(owner, &path::data_dir())
}

/// [`owner_root`] 的可注入版本（单测用：不碰环境变量）。
///
/// ⚠️ 入参 owner 一律先过 [`resolve`]：`Local` **不是**"顶层目录"，而是
/// "uid 还没查出来、等于当前管理员"。见模块头部的口径。
pub fn owner_root_at(owner: Owner, data_home: &Path) -> PathBuf {
    let uid = resolve(owner);
    data_home.join("users").join(uid.to_string())
}

/// **把 `Local` 解析成具体 uid** —— 本模块唯一一处"谁是谁"的判定。
///
/// `Local`（托盘 / CLI / `--local-no-auth`）代表的**就是当前管理员**，
/// 所以它和 `User(admin_uid)` 必须落到同一个目录。以前 `Local` 直接映射到**顶层**
/// `<data>/resources/`，于是"同一个我"在 `resources/` 与 `users/1/` 各有一套素材 ——
/// 实测症状：贴纸构建写进了 A、列表读的是 B（2026-10-10）。
///
/// ## 谁装这个解析器
///
/// core **不认识账本**（它连 `yekitrim_service` 都不许 `use`），所以"当前管理员是谁"
/// 由上层用 [`set_local_admin`] 注入一次（server 启动查到首个管理员后；
/// CLI / 托盘的 `--local-no-auth` 同理）。**没装**时的兜底见下。
///
/// ## 没装时为什么是 1、而不是报错
///
/// `data_dir()` 在**单元测试**与**纯 CLI**（不碰账本的那些命令）里都会被调用，
/// 那些场景下账本可能压根不存在。此时用 uid=1 是**唯一合理**的选择：
/// 全新库的首个账号必然是 1（`users.id` 自增），而"首个管理员"的判定本来就是 min-id。
///
/// ⚠️ 兜底**不是**"猜"：真有多用户库时，上层一定装过（server 启动就装）。
/// 这里不 panic 是因为 core 的纯函数不该依赖"上层有没有调过某个初始化"。
fn resolve(owner: Owner) -> i64 {
    match owner {
        Owner::User(uid) => uid,
        Owner::Local => local_admin_uid(),
    }
}

/// `Local` 解析成的 uid（默认 1 = 全新库的首个账号）。
static LOCAL_ADMIN_UID: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);

/// 装一次"当前管理员是谁"（server 启动 / 托盘拉起服务前调）。
///
/// 幂等；只认正数（<=0 视为"没查到"，保持原值 —— 静默写成 0 会让所有人落到 `users/0/`）。
pub fn set_local_admin(uid: i64) {
    if uid > 0 {
        LOCAL_ADMIN_UID.store(uid, std::sync::atomic::Ordering::Relaxed);
        // ⚠️ 打一行（整改 T22）：漏设的失败模式是**所有人静默落到 users/1/**，
        // 而"到底解析到谁"只能从这一行看出来。
        tracing::info!("local admin uid = {uid}（Owner::Local 的落点）");
    } else {
        tracing::warn!("set_local_admin 收到非正 uid={uid}，保持原值（写 0 会让所有人落到 users/0/）");
    }
}

/// 当前 `Local` 解析成的 uid（单测与诊断用；生产代码不该拿它自己拼路径）。
pub fn local_admin_uid() -> i64 {
    LOCAL_ADMIN_UID.load(std::sync::atomic::Ordering::Relaxed)
}

/// **公用素材库根** —— `<data>/shared/`。
///
/// 口径（2026-10-10 用户拍板，2026-11-27 改名）：公用素材库就是这一件事 ——
/// 字体 / 贴纸 / 视频素材，对所有账号可见（用户库优先、公用补缺）。
///
/// ⚠️ **它不装配置**：配置在账号树 `<data>/users/<uid>/{config,llm}.toml`。
/// 别再把配置类的目录塞进 `shared/`：那会让"公用素材库"这个概念变浑。
pub fn shared_assets_dir() -> PathBuf {
    path::shared_dir()
}

/// 公用目录名。
pub const SHARED_DIR_NAME: &str = "shared";

/// 某类素材**该读哪几个目录**，**按优先级从高到低**：`[用户库, 公用库]`。
///
/// ## 为什么是"候选列表"而不是违反硬规矩 2
///
/// 硬规矩 2 禁的是**同一个东西有多个可能位置、而解析时只查一处**（那种漂移不报错）。
/// 这里是**有意的分层**：用户库优先、公用库补缺，两层**都查**、顺序**固定**，
/// 且"写到哪"永远只有一处（[`asset_dir`] = 用户库）。
///
/// ⚠️ **写绝不能走这个列表** —— 往"第一个存在的"里写会让"公用只读"变成"谁先写谁占用"。
pub fn asset_dirs(owner: Owner, kind: AssetKind) -> Vec<PathBuf> {
    let mut v = vec![asset_dir(owner, kind)];
    let g = shared_assets_dir().join(kind.dir_name());
    if !v.contains(&g) {
        v.push(g);
    }
    v
}

/// 某一类素材的库目录。
///
/// ⚠️ 三者的父目录一律是 `users/<uid>/`（`Local` 先经 [`resolve`] 换成管理员 uid），
/// 不再有中间的 `resources/` 层。
pub fn asset_dir(owner: Owner, kind: AssetKind) -> PathBuf {
    asset_dir_at(owner, kind, &path::data_dir())
}

/// [`asset_dir`] 的可注入版本（单测用）。
pub fn asset_dir_at(owner: Owner, kind: AssetKind, data_home: &Path) -> PathBuf {
    owner_root_at(owner, data_home).join(kind.dir_name())
}

/// 贴纸库**该读哪个目录**。
///
/// ⚠️ **2026-10-10 起不再分"单人/多人"两条路**：`Local` 也被解析成管理员 uid，
/// 于是它读的是**用户树**（`users/<uid>/resources/stickers/`）。
///
/// 因此这里**不再**套 [`path::sticker_dir_readable_at`] 那套"老位置回落"
/// —— 那套回落找的是**顶层** `<data>/stickers/` 与 `<data>/resources/stickers/`，
/// 而用户树从来不是"从顶层长出来"的：套回落 = 凭空造出第二个候选位置，
/// 正是本模块硬规矩 2 禁止的事。
///
/// 顶层存量怎么处理？**靠迁移**（`migrate_legacy_assets_to`）：升级时把
/// `<data>/resources/*` 搬进首个管理员的树，搬完顶层就该是空的。
pub fn sticker_dir_readable(owner: Owner) -> PathBuf {
    sticker_dir_readable_at(owner, &path::data_dir())
}

/// [`sticker_dir_readable`] 的可注入版本（单测用）。
pub fn sticker_dir_readable_at(owner: Owner, data_home: &Path) -> PathBuf {
    asset_dir_at(owner, AssetKind::Stickers, data_home)
}

/// 视频素材库**该读哪个目录**（口径同 [`sticker_dir_readable`]）。
pub fn video_dir_readable(owner: Owner) -> PathBuf {
    video_dir_readable_at(owner, &path::data_dir())
}

/// [`video_dir_readable`] 的可注入版本（单测用）。
pub fn video_dir_readable_at(owner: Owner, data_home: &Path) -> PathBuf {
    asset_dir_at(owner, AssetKind::Videos, data_home)
}

/// 用户导入的**音效包库**该读哪个目录（口径同 [`sticker_dir_readable`]）。
///
/// ⚠️ 与内建预设（`assets/sfx_catalog.json`，编译期内嵌、**不在磁盘上**）是两回事：
/// 这里只放用户自己传的包，`<包名>/` 一个子目录 = 一个包。
pub fn sfx_dir_readable(owner: Owner) -> PathBuf {
    sfx_dir_readable_at(owner, &path::data_dir())
}

/// [`sfx_dir_readable`] 的可注入版本（单测用）。
pub fn sfx_dir_readable_at(owner: Owner, data_home: &Path) -> PathBuf {
    asset_dir_at(owner, AssetKind::Sfx, data_home)
}

/// 该 owner 的**程序配置**（`config.toml`，就在用户根下）。
///
/// ⚠️ **账本（`users` / `sessions` / `invites` 表）永远全局、不走这里** ——
/// 它是**身份的来源**，按用户分 = 登录前先得知道你是谁 = 循环依赖。
/// 见 `yekitrim_service::db::db_path`。
pub fn config_path(owner: Owner) -> PathBuf {
    config_path_at(owner, &path::data_dir())
}

/// [`config_path`] 的可注入版本（单测用）。
pub fn config_path_at(owner: Owner, data_home: &Path) -> PathBuf {
    owner_root_at(owner, data_home).join("config.toml")
}

/// 该 owner 的 LLM 配置（`llm.toml`，与 `config.toml` 同级）。
pub fn llm_config_path(owner: Owner) -> PathBuf {
    llm_config_path_at(owner, &path::data_dir())
}

/// [`llm_config_path`] 的可注入版本（单测用）。
pub fn llm_config_path_at(owner: Owner, data_home: &Path) -> PathBuf {
    owner_root_at(owner, data_home).join("llm.toml")
}

/// 把**旧布局**的存量（`<data>/resources/*` 与 `<data>/legacy-config/{config,llm}.toml`）
/// 归到某个账号名下 —— **一次性、幂等、不复制内容**。
///
/// ## 为什么要它
///
/// 旧布局把单人素材放在 `<data>/resources/`、把退役的顶层配置放在
/// `<data>/.yekitrim/`（迁移引擎会把后者改名成 `<data>/legacy-config/`）；
/// 而多用户形态下 [`Owner::User`] 解析到 `<data>/users/<uid>/…`。不做这一步的话，
/// 一台已用很久的机器一开多用户，**新账号看到的是空素材库与默认配置**，
/// 用户会以为"我的东西全丢了"。
///
/// ## 语义：**移动**，不是复制
///
/// 素材可能有几个 GB（`op.mp4`、贴纸包），**复制一份是错的**（占双倍磁盘、
/// 且两份从此分叉）。所以是 `rename`；跨盘时 `rename` 会失败，那时**如实报错、
/// 保持原状**，让调用方（或用户手工）决定 —— 绝不悄悄退化成"复制一半"。
///
/// ⚠️ **只在目标不存在时动手**（幂等）：`users/<uid>/<kind>/` 已经在了就整段跳过，
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
    let legacy_resources = path::legacy_resources_dir_at(data_home);

    let mut moved = Vec::new();

    // ① 素材：<data>/resources/<kind>/ → <data>/users/<uid>/<kind>/
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
        // 搬完的空壳不留（非空说明还有别的东西，保持原状）
        if std::fs::read_dir(&legacy_resources)
            .map(|mut it| it.next().is_none())
            .unwrap_or(false)
        {
            let _ = std::fs::remove_dir(&legacy_resources);
        }
    }

    // ② 配置：<data>/legacy-config/{config,llm}.toml → <data>/users/<uid>/
    let legacy_state = data_home.join("legacy-config");
    let dest_state = owner_root_at(owner, data_home);
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
    if std::fs::read_dir(&legacy_state)
        .map(|mut it| it.next().is_none())
        .unwrap_or(false)
    {
        let _ = std::fs::remove_dir(&legacy_state);
    }

    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️ **口径判据（2026-10-10 改）**：`Owner::Local` 必须与"当前管理员"**逐字节相同**。
    ///
    /// 这条**取代**了原来的 `local_owner_resolves_to_exactly_todays_locations`
    /// （它钉的是"`Local` = 顶层 `<data>/resources/`"）。那条口径就是这次
    /// "贴纸构建了但没识别"的根 —— 托盘与 admin 登录指向了两个地方。
    #[test]
    fn local_owner_resolves_to_the_admin_user_tree() {
        let home = Path::new("/data");
        let admin = local_admin_uid();
        assert!(admin > 0, "管理员 uid 必须是正数，实得 {admin}");

        // 顶层**不该**再出现在任何解析结果里。
        let top_resources = path::legacy_resources_dir_at(home);
        for kind in [AssetKind::Fonts, AssetKind::Stickers, AssetKind::Videos] {
            let local = asset_dir_at(Owner::Local, kind, home);
            let as_user = asset_dir_at(Owner::User(admin), kind, home);
            assert_eq!(local, as_user, "{kind:?}：Local 必须等于 User({admin})");
            assert!(
                local.starts_with(home.join("users")),
                "{kind:?} 必须落在 users/ 下，实得 {}",
                local.display()
            );
            assert_ne!(local, top_resources.join(kind.dir_name()),
                "{kind:?} 不许再解析到顶层 resources/");
        }

        // 每个具体 uid 都有自己的一棵树，且互不相等。
        let a = asset_dir_at(Owner::User(1), AssetKind::Fonts, home);
        let b = asset_dir_at(Owner::User(2), AssetKind::Fonts, home);
        assert_ne!(a, b, "不同账号的库必须分开");
        let admin_home = home.join("users").join(local_admin_uid().to_string());
        assert_eq!(owner_root_at(Owner::Local, home), admin_home);
        assert_eq!(
            asset_dir_at(Owner::Local, AssetKind::Fonts, home),
            admin_home.join("fonts")
        );
        assert_eq!(
            config_path_at(Owner::Local, home),
            admin_home.join("config.toml")
        );
        assert_eq!(
            llm_config_path_at(Owner::Local, home),
            admin_home.join("llm.toml")
        );
    }

    /// 多用户形态：三类素材都落在 `<data>/users/<uid>/<kind>/`。
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
                Path::new("/data/users/7").join(sub)
            );
        }
        assert_eq!(owner_root_at(Owner::User(uid), home), Path::new("/data/users/7"));
        assert_eq!(
            config_path_at(Owner::User(uid), home),
            Path::new("/data/users/7/config.toml")
        );
        assert_eq!(
            llm_config_path_at(Owner::User(uid), home),
            Path::new("/data/users/7/llm.toml")
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
        let admin = local_admin_uid();
        let mut seen: Vec<(PathBuf, &str)> = Vec::new();
        // ⚠️ `local` 与 `admin`（即 `User(admin_uid)`）**本来就该撞** ——
        // 它们是同一个人。所以只把"**不同人**"放进互斥检查。
        for (label, owner, is_admin_self) in [
            ("local", Owner::Local, true),
            ("admin", Owner::User(admin), true),
            ("u2", Owner::User(2), false),
            ("u3", Owner::User(3), false),
        ] {
            for kind in kinds {
                let d = asset_dir_at(owner, kind, home);
                // 只有"不同的账号"之间才不许撞。
                if !is_admin_self {
                    if let Some((_, other)) = seen.iter().find(|(p, _)| *p == d) {
                        panic!("{label} 的 {kind:?} 与 {other} 撞在 {d:?}");
                    }
                }
                seen.push((d, label));
            }
        }

        // 正面判据：`Local` 与 admin **必须**是同一处（这正是本版的口径）。
        assert_eq!(
            asset_dir_at(Owner::Local, AssetKind::Fonts, home),
            asset_dir_at(Owner::User(admin), AssetKind::Fonts, home),
            "Local 就是 admin，两者必须是同一棵库树"
        );
    }

    /// **公用层**（`<data>/shared/<kind>/`）：素材共享、且**用户库优先**。
    ///
    /// 这条钉住三件事：① 顺序是 `[用户库, 公用库]`；② 两者不同；③ **写路径**只有一个
    /// （`asset_dir` = 用户库）—— 写如果也跟着"候选列表"走，"公用只读"就变成"谁先写谁占用"。
    #[test]
    fn shared_library_is_a_second_read_layer_but_never_a_write_target() {
        let home = Path::new("/data");
        for kind in [AssetKind::Fonts, AssetKind::Stickers, AssetKind::Videos] {
            let user = asset_dir_at(Owner::User(7), kind, home);
            let shared = home.join(SHARED_DIR_NAME).join(kind.dir_name());

            // 公用库与任何账号的库都不是同一个目录。
            assert_ne!(user, shared, "{kind:?}：用户库不能等于公用库");

            // 顺序固定：用户库在前（同 id 时用户赢）。
            // ⚠️ `asset_dirs` 走真实 data_dir()，所以这里只断言**关系**，不比绝对路径。
            let dirs = asset_dirs(Owner::User(7), kind);
            assert_eq!(dirs.len(), 2, "{kind:?}：正好两层");
            assert_ne!(dirs[0], dirs[1], "{kind:?}：两层必须不同");
            assert!(
                dirs[1].ends_with(Path::new(SHARED_DIR_NAME).join(kind.dir_name())),
                "{kind:?}：第二层必须是 shared/<kind>，实得 {}",
                dirs[1].display()
            );
        }

        // admin 那棵树与公用库也是分开的（admin ≠ 公用）。
        assert_ne!(
            asset_dir_at(Owner::User(local_admin_uid()), AssetKind::Fonts, home),
            home.join(SHARED_DIR_NAME).join("fonts"),
            "管理员库不能等于公用库"
        );
    }

    /// **`shared/` 只放素材** —— 它不是"第二个配置目录"。
    #[test]
    fn shared_root_holds_assets_only() {
        let assets = shared_assets_dir();
        assert_eq!(assets, path::data_dir().join(SHARED_DIR_NAME));
        assert_eq!(
            assets.file_name().and_then(|s| s.to_str()),
            Some(SHARED_DIR_NAME)
        );

        // 三个素材种类都能在 shared 下取到。
        for kind in [AssetKind::Fonts, AssetKind::Stickers, AssetKind::Videos] {
            assert!(assets.join(kind.dir_name()).ends_with(kind.dir_name()));
        }

        // ⚠️ 配置**不在** `shared/` 里（它在 users/<uid>/）。
        assert!(
            !assets.starts_with(path::data_dir().join("users")),
            "配置目录不该在 shared/ 下"
        );
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
        let legacy_stickers = path::legacy_sticker_dir_in_resources_at(home);
        std::fs::create_dir_all(&legacy_stickers).unwrap();
        std::fs::write(legacy_stickers.join("pack.toml"), b"x").unwrap();
        let legacy_fonts = path::legacy_font_dir_at(home);
        std::fs::create_dir_all(&legacy_fonts).unwrap();
        std::fs::write(legacy_fonts.join("a.ttf"), b"font").unwrap();
        let legacy_state = home.join("legacy-config");
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
        let new_llm = llm_config_path_at(Owner::User(7), home);
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
        let legacy = path::legacy_font_dir_at(home);
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
        let legacy = path::legacy_font_dir_at(home);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("a.ttf"), b"font").unwrap();

        let moved = migrate_legacy_assets_to(Owner::Local, home).unwrap();
        assert!(moved.is_empty());
        assert!(legacy.join("a.ttf").is_file(), "单人形态一个字节都不该动");
    }
}
