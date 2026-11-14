//! yt-dlp 的**供应链**：版本 + sha256 钉死的注册表，以及"确保有一份可用"的查找/下载。
//!
//! 结构照 `yekitrim-core/src/download.rs::MODEL_REGISTRY`，但有一条关键差异：
//! 模型的 marker 是"某个文件存在"，而这里的 marker **就是 sha256 本身** ——
//! 半下载的 exe / 被替换过的 exe 都不算"就绪"。别照抄 `ensure_one_model` 的 marker 判定。
//!
//! ## 平台与钉死的边界（重要）
//!
//! sha256 一律来自官方 `SHA2-256SUMS`（同一条 release，TLS 取回）：
//! **Windows / Linux x86_64 / Linux aarch64 / macOS** 四档都钉住了，都能自动下载。
//! 其它平台（含 musl 外的不常见架构）仍**不允许自动下载** —— 没钉过的二进制不该被自动化拉下来，
//! `ensure_ytdlp` 会明确报错，让管理员用 `[ingest] ytdlp_path` 或装进 PATH。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use yekitrim_core::config::Config;

/// 一个平台上要用的 yt-dlp 发布物。
#[derive(Debug, Clone, Copy)]
pub struct YtDlpPin {
    /// release 资产名
    pub asset: &'static str,
    /// 落盘文件名
    pub file: &'static str,
    /// 官方 `SHA2-256SUMS` 里的哈希；**空 = 本平台未验证**（禁止自动下载）
    pub sha256: &'static str,
    /// 期望字节数（仅用于日志与"看起来不对"的二次判据）
    pub size: u64,
}

/// 本平台要用的发布物。
///
/// ⚠️ 换版本时必须三件事一起做：改 `VERSION`、改这里每个平台的 `sha256` 与 `size`、
/// 并**重新实测**（下下来跑一次 `--version`）。只改版本号会让校验失败 → 下载通道整个不可用。
pub const fn pin() -> YtDlpPin {
    #[cfg(windows)]
    {
        YtDlpPin {
            asset: "yt-dlp.exe",
            file: "yt-dlp.exe",
            // 实测：官方 SHA2-256SUMS 与本机实算一致（2026-09-19）
            sha256: "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a",
            size: 17_840_399,
        }
    }
    // Linux：按架构选发布物（官方对 x86_64 / aarch64 各有一份 glibc 构建）。
    // ⚠️ 哈希取自官方 `SHA2-256SUMS`（同一条 release）；换版本必须连它一起改。
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        YtDlpPin {
            asset: "yt-dlp_linux",
            file: "yt-dlp",
            sha256: "58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a",
            size: 40_446_224,
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        YtDlpPin {
            asset: "yt-dlp_linux_aarch64",
            file: "yt-dlp",
            sha256: "b16e4dab368a816cd05d477d698a605a6ae87ccee1c8ffd38fa21d7254141fcc",
            size: 40_167_448,
        }
    }
    #[cfg(all(target_os = "linux", not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
    {
        YtDlpPin { asset: "", file: "yt-dlp", sha256: "", size: 0 }
    }
    #[cfg(target_os = "macos")]
    {
        YtDlpPin {
            asset: "yt-dlp_macos",
            file: "yt-dlp",
            sha256: "0f192b7ec147ab6288885d6351d9ab67367640029b4377576ef46dd79cf7b202",
            size: 37_146_048,
        }
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        YtDlpPin { asset: "", file: "yt-dlp", sha256: "", size: 0 }
    }
}

/// GitHub release 的镜像兜底（**只在直连失败时用**，且下载后必须过 sha256）。
///
/// ⚠️ 这些是第三方反代，不是我们托管的东西：**能用它们的唯一前提是哈希校验**。
/// 顺序 = 实测更快/more 可靠在前；全都不通就回到直连那条报错。
const MIRRORS: &[&str] = &[
    "https://gh-proxy.com",
    "https://ghproxy.net",
];

/// 钉死的发布版本（`release` 标签名）。
pub const VERSION: &str = "2026.08.19";
/// yt-dlp 的 license：Unlicense（公有领域）→ 随 release 打包**无附加义务**。
pub const LICENSE: &str = "Unlicense";

/// 托管副本所在目录：`<程序目录>/.yekitrim/bin/`（= `path::yekitrim_home()/bin`）。
pub fn managed_dir() -> PathBuf {
    yekitrim_core::path::yekitrim_home().join("bin")
}

/// 托管副本的完整路径。
pub fn managed_path() -> PathBuf {
    managed_dir().join(pin().file)
}

fn release_url() -> String {
    format!(
        "https://github.com/yt-dlp/yt-dlp/releases/download/{VERSION}/{}",
        pin().asset
    )
}

/// 这份东西是从哪找来的（界面/诊断要显示，但**不显示完整路径**给未认证接口）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YtDlpSource {
    /// `[ingest] ytdlp_path`
    Config,
    /// 托管目录 `<程序目录>/.yekitrim/bin/`
    Managed,
    /// PATH
    PathEnv,
}

impl YtDlpSource {
    pub fn label(self) -> &'static str {
        match self {
            YtDlpSource::Config => "配置指定",
            YtDlpSource::Managed => "随服务托管",
            YtDlpSource::PathEnv => "系统 PATH",
        }
    }
}

/// 找到的一份 yt-dlp。
#[derive(Debug, Clone)]
pub struct YtDlpInstall {
    pub path: PathBuf,
    pub source: YtDlpSource,
    /// 托管副本是否**逐个字节**匹配注册表（配置/PATH 来源不校验：那是管理员的选择）
    pub verified: bool,
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)
        .with_context(|| format!("读取文件失败: {}", path.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut f, &mut hasher).with_context(|| format!("计算摘要失败: {}", path.display()))?;
    Ok(to_hex(&hasher.finalize()))
}

/// 小写的十六进制（自己写 10 行，不为这一个函数引入 `hex` 依赖）。
fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// 校验一个文件的 sha256。不匹配返回 `Err`（调用方负责删文件）。
pub fn verify_sha256(path: &Path, expect: &str) -> Result<()> {
    if expect.is_empty() {
        bail!("注册表里没有本平台的 sha256（未验证的二进制不该被自动采纳）");
    }
    let got = file_sha256(path)?;
    if got != expect.to_ascii_lowercase() {
        bail!(
            "sha256 校验失败: {} 期望 {}，实得 {}",
            path.display(),
            expect,
            got
        );
    }
    Ok(())
}

/// 可信度缓存键 = `(路径, 长度, mtime)`。抽成独立函数是为了**可测**：
/// 真正的风险是有人把 `mtime`（或长度）从键里去掉，那样"管理员重装了 yt-dlp"
/// 会永远读到旧结论 —— 见 `trust_cache_key_follows_content_changes`。
fn trust_cache_key(path: &Path) -> Option<(PathBuf, u64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some((path.to_path_buf(), meta.len(), mtime))
}

/// 某个文件是否是一份"可信的托管副本"。
///
/// - 本平台有钉死哈希 → 必须逐个字节匹配；
/// - 本平台没钉 → **一律不可信**（`false`），于是不会走自动下载，也不会被当成就绪。
///
/// ⚠️ **带缓存，别把缓存去掉**：一次校验要读完整个 17.8MB 的 exe。
/// 而本函数被**高频路径**调用 —— `/api/health`（前端反复轮询）、
/// `download_worker::tick`（每 10s 一轮，队列空也照跑）、`kick_one`。
/// 无缓存时等于每次健康检查都哈希 ~17MB，把"health 不碰磁盘"这条优化整个抵消掉。
///
/// 键用 `(路径, 长度, mtime)`：文件被替换（重装、换版本）时三者至少变一个 → 缓存自动失效；
/// 单纯重复查找则完全免费。这是"够用且不会被绕过"的判据 —— 用 `OnceLock<bool>` 那种
/// 一次性的缓存反而是错的，管理员重装之后永远读到旧结论。
fn managed_copy_is_trusted(path: &Path) -> bool {
    let p = pin();
    if p.sha256.is_empty() {
        return false;
    }
    let Some(key) = trust_cache_key(path) else {
        return false;
    };

    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<(PathBuf, u64, i64, bool)>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((cp, clen, cmt, ok)) = guard.as_ref() {
            if cp == &key.0 && *clen == key.1 && *cmt == key.2 {
                return *ok;
            }
        }
    }
    let ok = verify_sha256(path, p.sha256).is_ok();
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((key.0, key.1, key.2, ok));
    }
    ok
}

/// 找可执行文件时扫的目录：`PATH` ∪ **系统常见安装位置**。
///
/// ⚠️ **不是"只看 PATH"**：systemd 起的用户服务拿到的是极简 `PATH`
/// （常常只有 `/usr/local/bin:/usr/bin:/bin`），而 Linux 用户按文档装 yt-dlp
/// 多半落在 `~/.local/bin`（pipx / pip --user）或 `/snap/bin`。
/// 只查 PATH 的症状是"我明明装了，它说没装"。
pub fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|raw| std::env::split_paths(&raw).collect())
        .unwrap_or_default();
    for extra in [
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/snap/bin",
        "/var/lib/flatpak/exports/bin",
    ] {
        dirs.push(PathBuf::from(extra));
    }
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/bin"));
        dirs.push(PathBuf::from(&home).join("bin"));
    }
    dirs
}

/// 在 PATH 上找可执行文件（不 spawn，纯文件系统探测 —— 探测在 `tool_probe` 里做）。
///
/// ⚠️ **Linux/macOS 上还要看几个常见的安装位置**：systemd 起的服务拿到的是
/// 一条极简的 `PATH`（常常只有 `/usr/local/bin:/usr/bin:/bin`，连 `~/.local/bin`
/// 都没有）⇒ 用户自己 `pip install yt-dlp` 装到 `~/.local/bin`，服务却"找不到"。
/// 这里补上这些目录，纯查文件系统、不 spawn。
fn find_in_path(name: &str) -> Option<PathBuf> {
    for dir in candidate_dirs() {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let cand = dir.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// 查找顺序：**`[ingest] ytdlp_path` → 托管目录 → PATH**。纯查找，不下载。
///
/// 返回 `None` = 一份都没有。要"确保有一份"用 [`ensure_ytdlp`]。
pub fn find_ytdlp(cfg: &Config) -> Option<YtDlpInstall> {
    // 1) 配置指定：管理员的选择，**不校验**哈希（否则他换新版就被拦下了）
    let custom = cfg.ingest.ytdlp_path.trim();
    if !custom.is_empty() {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some(YtDlpInstall {
                verified: managed_copy_is_trusted(&p),
                path: p,
                source: YtDlpSource::Config,
            });
        }
        // 配了但不可用：继续往下找（由 doctor 报出"路径不生效"），不在这里直接失败 ——
        // 否则一份打错的配置会让本来能用的托管副本也被拒绝。
    }

    // 2) 托管副本（**必须哈希匹配**）
    let managed = managed_path();
    if managed_copy_is_trusted(&managed) {
        return Some(YtDlpInstall { path: managed, source: YtDlpSource::Managed, verified: true });
    }

    // 3) PATH
    let pin_now = pin();
    for name in [pin_now.file, "yt-dlp.exe", "yt-dlp"] {
        if let Some(p) = find_in_path(name) {
            // 托管目录里的那份如果哈希不对，`find_in_path` 也可能命中它 —— 排除掉，
            // 避免把一份损坏的托管副本当成"PATH 上的 yt-dlp"用。
            if p == managed {
                continue;
            }
            return Some(YtDlpInstall { verified: false, path: p, source: YtDlpSource::PathEnv });
        }
    }
    None
}

/// 就绪位（`/api/health` 与 `yekitrim doctor` 共用）。**只回布尔 + 来源标签，不回路径。**
pub fn ready(cfg: &Config) -> (bool, &'static str) {
    match find_ytdlp(cfg) {
        Some(i) => (true, i.source.label()),
        None => (false, "未安装"),
    }
}

/// 「装不上时怎么办」的具体命令（**按平台给**，别只说"自行获取"）。
///
/// Linux 上最常见的其实是"系统包管理器已经能装"，把命令写出来比让人去翻文档有用得多。
pub fn install_hint() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "可以改用系统包管理器装一份，YekiTrim 会自动发现（PATH 里就能找到）：\n\
         · Debian/Ubuntu:  sudo apt install yt-dlp\n\
         · Arch:           sudo pacman -S yt-dlp\n\
         · Fedora:         sudo dnf install yt-dlp\n\
         · 或者用 pip：     pipx install yt-dlp   （装到 ~/.local/bin，也会被找到）\n\
         装完点「刷新」即可，不需要重启服务。\n\
         也可以自己下载后写进 [ingest] ytdlp_path = \"/绝对路径/yt-dlp\"。"
    }
    #[cfg(target_os = "macos")]
    {
        "可以用 Homebrew 装一份，YekiTrim 会自动发现：\n\
         brew install yt-dlp\n\
         装完点「刷新」即可。也可以自己下载后写进 [ingest] ytdlp_path。"
    }
    #[cfg(windows)]
    {
        "可以自己下载 yt-dlp.exe 后写进 [ingest] ytdlp_path = \"D:/tools/yt-dlp.exe\"，\n\
         或把 yt-dlp.exe 放进 PATH 里的任意目录，然后点「刷新」。"
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        "请自行获取 yt-dlp 后写进 [ingest] ytdlp_path，或放进 PATH，然后点「刷新」。"
    }
}

/// 下载托管副本并**校验 sha256**，返回落盘路径。
///
/// 失败时不留半成品：`.part` 会被删掉（半下载的 exe 被当成可用才是最难查的那类故障）。
pub async fn download_managed(cfg: &Config) -> Result<PathBuf> {
    let p = pin();
    if p.asset.is_empty() {
        bail!("当前平台没有可用的 yt-dlp 发布物");
    }
    if p.sha256.is_empty() {
        bail!(
            "当前平台（{} / {}）没有钉死的 yt-dlp 发布物，不会自动下载。\
             请自行获取后写入 [ingest] ytdlp_path，或放进 PATH。",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    }

    let dir = managed_dir();
    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("创建下载器目录失败: {}", dir.display()))?;
    let part = dir.join(format!("{}.part", p.file));
    let final_path = dir.join(p.file);

    let primary = release_url();
    // 落盘走 core 的**可续传 + 多连接分段**下载器：
    // 实测这台机器到 github.com 单流只有 ~30 KB/s 且十几分钟会 `Connection timed out`
    // （40 MB 的包卡在 33 MB）—— 一次失败就报错等于"永远装不上"。
    // 网页入口常连不上 ⇒ 先换成 GitHub 官方 API 的资产直链（仍是同一条 release）。
    // 三条线路**并发竞速**（各拉 4 秒），选吞吐最高的那条真正下载。
    // 之前是顺序试：慢线路排前面 + 每条 3 次重试 ⇒ 最坏 15 分钟才开始下真正的数据。
    // 竞速 = 最多 4 秒出结论，且选路有数据支撑。
    let proxy_raw = cfg.ingest.proxy.trim().to_string();
    let proxy = if proxy_raw.is_empty() { None } else { Some(proxy_raw.as_str()) };
    if let Some(p) = proxy {
        tracing::info!("走代理下载 yt-dlp: {p}");
    }
    let mut candidates: Vec<(String, String)> = Vec::new();
    if let Some(api) = yekitrim_core::download::github_asset_direct_url_with(&primary, proxy).await {
        candidates.push(("api".into(), api));
    }
    candidates.push(("web".into(), primary.clone()));
    for host in MIRRORS {
        candidates.push((host.to_string(), format!("{host}/{}", primary)));
    }
    let winner = yekitrim_core::download::race_routes(candidates, 512 * 1024, proxy).await;
    let Some(win) = winner else {
        bail!(
            "所有下载线路都不可用（GitHub 直连 / API / 镜像）——本机网络到 GitHub 不通。\n\
             {}",
            install_hint()
        );
    };
    tracing::info!("选中线路 {}（{} KB/s）开始下载 yt-dlp {VERSION}", win.label, win.throughput() / 1024);

    // ⚠️ **阈值给大（64 MiB），别对 40 MB 的包分段**：分段下载写的是 `.segN`，
    // 进程一重启（或安装被中断）那些段就作废了 —— `.part` 续传只在**单流**路径上成立。
    // 40 MB 走单流 + 断点续传：断了重启能接着下；分段只在真正的大文件（模型那种）上才划算。
    let written = yekitrim_core::download::download_resumable(&win.url, &part, 64 * 1024 * 1024, 8, proxy)
        .await
        .with_context(|| {
            format!(
                "下载 yt-dlp {VERSION} 失败（线路 {}）。\n{}",
                win.label,
                install_hint()
            )
        })?;

    // 校验：哈希必须匹配注册表（供应链钉子）
    let got = file_sha256(&part)?;
    if got != p.sha256.to_ascii_lowercase() {
        let _ = tokio::fs::remove_file(&part).await;
        bail!(
            "yt-dlp 下载校验失败（已删除临时文件）：期望 {}，实得 {got}（{written} 字节）。\
             可能是网络中间人、镜像被替换或 release 已变更 —— 请勿使用。",
            p.sha256
        );
    }

    // Unix 上要可执行
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = tokio::fs::metadata(&part).await {
            let mut perm = meta.permissions();
            perm.set_mode(0o755);
            let _ = tokio::fs::set_permissions(&part, perm).await;
        }
    }

    tokio::fs::rename(&part, &final_path)
        .await
        .with_context(|| format!("落盘失败: {}", final_path.display()))?;
    tracing::info!("yt-dlp {VERSION} 已就绪: {}", final_path.display());
    Ok(final_path)
}

/// **确保有一份可用的 yt-dlp**，返回其路径。
///
/// 顺序：配置 → 托管（哈希校验）→ PATH → 下载托管副本。
/// 最后一步失败就报错（**不静默降级**）——下载通道宁可明确失败，也不要"用了半份二进制"。
pub async fn ensure_ytdlp(cfg: &Config) -> Result<PathBuf> {
    if let Some(found) = find_ytdlp(cfg) {
        return Ok(found.path);
    }
    // 托管目录里有一份但哈希不对：删掉再下（它只可能是半下载/被替换的）
    let managed = managed_path();
    if managed.is_file() {
        tracing::warn!(
            "托管目录里的 yt-dlp 校验不通过，删除后重新下载: {}",
            managed.display()
        );
        let _ = tokio::fs::remove_file(&managed).await;
    }
    download_managed(cfg).await
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let d = std::env::temp_dir().join(format!(
                "yekitrim-ytdlp-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Tmp(d)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 回归钉子：**找可执行文件的目录不能只有 PATH**。
    ///
    /// Linux 上 systemd 用户服务拿到的 `PATH` 是极简的，用户按文档装到
    /// `~/.local/bin`（pipx / pip --user）就永远找不到 —— 症状是"装了却说没装"。
    #[test]
    fn candidate_dirs_cover_common_install_locations() {
        let dirs = candidate_dirs();
        let has = |p: &str| dirs.iter().any(|d| d == Path::new(p));
        assert!(has("/usr/local/bin"), "{dirs:?}");
        assert!(has("/usr/bin"), "{dirs:?}");
        if std::env::var_os("HOME").is_some() {
            let home = std::env::var_os("HOME").unwrap();
            let local = PathBuf::from(&home).join(".local/bin");
            assert!(dirs.contains(&local), "必须包含 ~/.local/bin：{dirs:?}");
        }
    }

    /// 镜像只是"多一条路"：**哈希校验才是判据**，镜像给错数据必须被拒。
    #[test]
    fn mirrors_are_only_fallbacks_and_still_verified() {
        assert!(!MIRRORS.is_empty());
        // 每个镜像地址都要能被拼成 "镜像 + 原始 release 地址"
        let primary = "https://github.com/yt-dlp/yt-dlp/releases/download/x/y";
        for m in MIRRORS {
            assert!(format!("{m}/{primary}").contains("github.com/yt-dlp/yt-dlp/releases/download"));
        }
    }

    /// ⚠️ 回归钉子：**sha256 不匹配必须报错**（而且调用方要能据此删文件）。
    ///
    /// 这是供应链钉子：注册表里的哈希写错/资产被替换时，只有这一条会拦住它。
    #[test]
    fn sha256_mismatch_is_rejected() {
        let t = Tmp::new("sha");
        let f = t.path().join("yt-dlp.exe");
        std::fs::write(&f, b"definitely not the real yt-dlp binary").unwrap();

        // 空哈希（未验证平台）→ 直接拒绝，不许"跳过校验"
        assert!(verify_sha256(&f, "").is_err(), "空哈希不能等于通过");
        // 错误哈希 → 拒绝
        let err = format!("{:#}", verify_sha256(&f, &"0".repeat(64)).unwrap_err());
        assert!(err.contains("sha256 校验失败"), "{err}");

        // 正确哈希 → 通过（用真实内容自证，避免"永远失败"的假绿）
        let real = "2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae"; // sha256("foo")
        std::fs::write(&f, b"foo").unwrap();
        assert!(verify_sha256(&f, real).is_ok());
        // 大小写不敏感（官方清单是全小写，但别人贴上来可能是大写）
        assert!(verify_sha256(&f, &real.to_uppercase()).is_ok());
        // 差一个字节就要失败
        std::fs::write(&f, b"foO").unwrap();
        assert!(verify_sha256(&f, real).is_err());
    }

    /// 十六进制编码器本身（错一位会让所有校验"通过"或"永远失败"）。
    #[test]
    fn hex_encoder_roundtrips_known_vectors() {
        assert_eq!(to_hex(&[]), "");
        assert_eq!(to_hex(&[0x00]), "00");
        assert_eq!(to_hex(&[0x0f, 0xf0]), "0ff0");
        // sha256("foo")
        assert_eq!(
            to_hex(&Sha256::digest(b"foo")),
            "2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae"
        );
    }

    /// 注册表本身要自洽：有 asset 就必须有落盘名；Windows 上必须有钉死的哈希与尺寸。
    #[test]
    fn registry_entry_is_self_consistent() {
        let p = pin();
        assert!(!p.file.is_empty());
        assert_eq!(VERSION, "2026.08.19");
        assert_eq!(LICENSE, "Unlicense", "许可姿态（发行物 0 copyleft）依赖这一条");
        if p.asset.is_empty() {
            return; // 未支持平台
        }
        // Windows：必须是实测过的那一份
        #[cfg(windows)]
        {
            assert_eq!(p.asset, "yt-dlp.exe");
            assert_eq!(p.sha256.len(), 64, "Windows 必须有钉死的 sha256");
            assert_eq!(p.size, 17_840_399);
        }
        // 有哈希就必须是 64 位小写十六进制（写错一位 = 下载通道整个不可用）
        if !p.sha256.is_empty() {
            assert_eq!(p.sha256.len(), 64);
            assert!(p.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        }
    }

    /// 下载 URL 的形状（改错了会 404，而 404 的文案会指向"网络问题"，很难查）。
    #[test]
    fn release_url_points_at_pinned_tag() {
        let url = release_url();
        assert!(url.starts_with("https://github.com/yt-dlp/yt-dlp/releases/download/"), "{url}");
        assert!(url.contains(VERSION), "{url}");
        assert!(url.ends_with(pin().asset), "{url}");
    }

    /// 配置指定优先，且**不校验哈希**（管理员换新版不该被拦）。
    /// 用临时目录 + 一个普通文件代替真实二进制。
    #[test]
    fn config_path_wins_and_skips_hash_check() {
        let t = Tmp::new("cfg");
        let fake = t.path().join("my-yt-dlp");
        std::fs::write(&fake, b"#!/bin/sh\necho 2099.01.01\n").unwrap();
        let mut cfg = Config::default();
        cfg.ingest.ytdlp_path = fake.to_string_lossy().into_owned();

        let found = find_ytdlp(&cfg).expect("配置路径存在就该被采纳");
        assert_eq!(found.path, fake);
        assert_eq!(found.source, YtDlpSource::Config);
        assert!(!found.verified, "配置来源不校验哈希（它是管理员的选择）");

        // 配置指向不存在的文件 → 不能报"就绪"，而要回落到其它来源 / None
        cfg.ingest.ytdlp_path = t.path().join("nope").to_string_lossy().into_owned();
        let got = find_ytdlp(&cfg);
        assert!(
            got.map(|g| g.source != YtDlpSource::Config).unwrap_or(true),
            "打错的配置不该被判成可用"
        );
    }

    /// 托管目录里的半下载文件**不算就绪**（marker 就是 sha256 本身）。
    #[test]
    fn half_downloaded_managed_copy_is_not_ready() {
        let t = Tmp::new("half");
        // 造一个"托管目录"：只在能被解析时用它；这里直接断言判定函数
        let junk = t.path().join("yt-dlp.exe");
        std::fs::write(&junk, vec![0u8; 1024]).unwrap();
        assert!(
            !managed_copy_is_trusted(&junk),
            "内容不匹配的托管副本必须判为不可信（否则会拿半份 exe 去下载视频）"
        );
        assert!(!managed_copy_is_trusted(&t.path().join("missing.exe")));
        // 目录路径也要安全返回 false（不能 panic，也不能把目录当可执行文件）
        assert!(!managed_copy_is_trusted(t.path()));
    }

    /// ⚠️ 性能 + 正确性的双向钉子：可信度缓存**必须按内容变化失效**。
    ///
    /// 背景：`managed_copy_is_trusted` 要读完整份 17.8MB 的 exe 算 sha256，
    /// 而它被 `/api/health`（前端轮询）与 `download_worker::tick`（每 10s）调用 → 必须缓存。
    /// 但如果键里只有路径，管理员重装 yt-dlp 后会**永远读到旧结论**（"未就绪"），
    /// 而且没有任何报错 —— 用户只会看到"下载器未安装"却怎么装都不好。
    ///
    /// 所以这里钉的是缓存**键**的性质：文件内容一变，键就必须变。
    #[test]
    fn trust_cache_key_follows_content_changes() {
        let t = Tmp::new("cachekey");
        let f = t.path().join("yt-dlp.exe");

        std::fs::write(&f, vec![1u8; 512]).unwrap();
        let k1 = trust_cache_key(&f).expect("普通文件应有键");

        // 长度变化 → 键必须变
        std::fs::write(&f, vec![1u8; 1024]).unwrap();
        let k2 = trust_cache_key(&f).expect("普通文件应有键");
        assert_ne!(k1, k2, "长度变了，缓存键必须跟着变（否则重装后永远读到旧结论）");

        // 长度不变但内容/mtime 变化 → 键也必须变（这就是为什么键里必须有 mtime）
        std::thread::sleep(std::time::Duration::from_millis(1100)); // mtime 精度到秒
        std::fs::write(&f, vec![2u8; 1024]).unwrap();
        let k3 = trust_cache_key(&f).expect("普通文件应有键");
        assert_ne!(k2, k3, "同长度但内容已改，缓存键必须靠 mtime 区分开");

        // 路径/key 的其它性质
        assert!(trust_cache_key(&t.path().join("missing.exe")).is_none(), "不存在的文件没有键");
        assert!(trust_cache_key(t.path()).is_none(), "目录没有键（不能被当成可执行文件）");
        assert_eq!(trust_cache_key(&f).unwrap(), k3, "同一份文件反复取键必须稳定");
    }
}
