//! 程序级字体库：**`<数据目录>/resources/fonts/`**（默认就是程序目录，见
//! [`crate::path::data_dir`]）。读写同一个位置，[`ensure_dir`] 建的就是它。
//!
//! ⚠️ **2026-10-02 之前是"用户固定目录 `~/.yekitrim/fonts` 优先"**，已整条改掉
//! （用户口径：*所有内容围绕程序目录来，不要用户目录*）。老那份用户目录不再被读，
//! 留 [`legacy_user_dir_report`] 只做"报路径和体积"，**不自动删也不搬**。
//!
//! 用户把 `.ttf` / `.otf` 丢进这个目录，界面就能把它们列出来当字幕/封面字体用。
//! 解析规则见 [`crate::path::font_dir`]。
//!
//! # 自带字体（\`BUILTIN_FONTS\`）**字节内嵌在二进制里**（2026-10-02 起，不再依赖下载）
//!
//! 实测两支自带字体加起来约 **34 MB**（Noto Sans SC 10.0 MB + Noto Color Emoji 24.7 MB）。
//! 字节走 \`include_bytes!\`（源在 `assets/fonts/`），**首次启动自动落盘、不联网** ——
//! 下载路径（`url` 字段 + `download_builtin_fonts`）仅作内嵌字节不可用时的兜底，
//! `sha256` 仍是那条路径的最终防线。两支都是 **OFL-1.1**（可商用/可嵌入/可再分发），
//! 原样附 `LICENSE.txt`。
//!
//! ⚠️ **没落盘 ≠ 看不见**：`scan_all` 会把"还没装的自带字体"也列出来（`installed: false`），
//! 界面据此给"安装"按钮。所以别把自带字体当成"库里必然存在"，
//! `is_builtin_id` 才是"这个 id 是不是自带的"的判据。
//!
//! 规矩与贴纸库一致：**以磁盘为准**（扫目录，没有索引文件）、id 取文件名去扩展名，
//! 所以"丢一个文件进去"就是全部操作。显示名从字体的 `name` 表里读（解析失败退回 id）——
//! 那只是给人看的标签，**配置里存的一律是 id**。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// 认的扩展名。**只收这三种**：Chrome 与 swash 都读得动；woff/woff2 只有浏览器认，
/// 收进来会出现"预览里好看、出片是另一个样"，不如直接不收。
pub const FONT_EXTS: [&str; 3] = ["ttf", "otf", "ttc"];

/// 目录是否已就绪（读侧一律不创建目录，方便判断"库是空的"和"库还没有"）。
pub fn dir() -> PathBuf {
    crate::path::font_dir()
}

/// **按归属**取字体库（`Owner::Local` = 现状位置；`Owner::User(uid)` = 各自的树）。
///
/// ⚠️ 渲染子进程（出片/封面）走这条 —— 它经 [`crate::owner::Owner::from_env`] 拿到父进程的
/// 身份。**别让子进程退回 `dir()`**：那就是"多用户下 A 导入的字体、B 出片时被用上"。
pub fn dir_for(owner: crate::owner::Owner) -> PathBuf {
    crate::owner::asset_dir(owner, crate::owner::AssetKind::Fonts)
}

/// [`ensure_dir`] 的按归属版本。
pub fn ensure_dir_for(owner: crate::owner::Owner) -> std::io::Result<PathBuf> {
    let root = dir_for(owner);
    std::fs::create_dir_all(&root)?;
    // ⚠️ 迁移闸是**按进程一次**的（见 `ASCII_MIGRATED`），而同一个进程里可能先后处理
    // 不同 owner（server 是长驻进程）⇒ 这里也要为每个 root 各跑一次。
    // 幂等的：判据是"名字还不是 ASCII / 有没有 id"。
    let moved = migrate_ascii_layouts(&root);
    if !moved.is_empty() {
        tracing::info!("字体库：{} 支字体的路径搬成 ASCII（{})", moved.len(), moved.join(", "));
    }
    Ok(root)
}

/// **按归属**解析字体 id → 文件路径（`None` = 库里没有这一支）。
///
/// ⚠️ **两层都搜**（用户库优先、全局补缺，2026-10-10）：自带字体装在 `global/`，
/// 只搜用户库的话**出片会找不到字体**（表现为"预览有字、成片没有"或直接报
/// "至少一个字体文件"）—— 这正是这条函数不能只看一层的理由。
pub fn resolve_file_for(owner: crate::owner::Owner, id: &str) -> Option<PathBuf> {
    resolve_all_roots(&crate::owner::asset_dirs(owner, crate::owner::AssetKind::Fonts), id)
}


/// “这次进程里已经跑过 ASCII 迁移了” —— 见 [`migrate_ascii_layouts`]，**每个进程只跑一次**。
///
/// 两个理由：① `ensure_dir` 会被每个导入/上传的 handler 调用，不设闸就是每次请求都扫一遍库；
/// ② 迁移要**改名**，窗口里正好有读请求就会看到“这一支不见了”（自愈，但没必要制造）。
/// 进程里的第一个调用者通常是 **server 启动 / `config init` / 出片前** —— 那正是希望它跑的时刻。
/// ⚠️ 磁盘上仍然是幂等的（判据是“名字还不是 ASCII / 有没有 id”），这个闸只是省重复扫描。
static ASCII_MIGRATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 建好字体库目录并返回**写路径**（`config init` / `server` 启动 / 导入前用）。
///
/// ⚠️ **只有一个库**：`<数据目录>/resources/fonts/`（见 [`crate::path::font_dir`]）。
/// 2026-10-02 之前这里会"用户目录优先 + 老库一次性搬家"，那是为了躲开构建目录漂移；
/// 代价是**同一个库有两个可能的位置**，CLI 与 WebUI 各看一份、互不同步。
/// 现在建目录就是建目录，没有任何"搬"的动作。
pub fn ensure_dir() -> std::io::Result<PathBuf> {
    ensure_dir_at(&crate::path::data_dir())
}

/// [`ensure_dir`] 的可注入版本（单测用）。
pub fn ensure_dir_at(data_home: &Path) -> std::io::Result<PathBuf> {
    let root = crate::path::font_dir_at(data_home);
    std::fs::create_dir_all(&root)?;
    // 老库里“路径含非 ASCII”的条目在这里一次性搬成 ASCII 形状（幂等；失败只 warn）。
    // 放在 ensure_dir 里是有意的：它是**写路径**（server 启动 / config init 都会走），
    // 读路径（scan/resolve）保持只读 —— 别让一次 GET 去改磁盘。
    if !ASCII_MIGRATED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        let moved = migrate_ascii_layouts(&root);
        if !moved.is_empty() {
            tracing::info!("字体库：{} 支字体的路径搬成 ASCII（{})", moved.len(), moved.join(", "));
        }
    }
    Ok(root)
}

/// 老用户目录 `~/.yekitrim/fonts` 的**只报不删**巡检（2026-10-02 资产收回程序目录之后）。
///
/// 为什么要留这一条：口径切换之后，老机器上那份用户目录**不会再被读**（[`dir`] 已经
/// 只认 `<数据目录>/resources/fonts/`）。但它占着盘、而且用户会在里面找不到"我刚导入的
/// 字体怎么不见了"。所以启动时探一下、把路径与体积**如实报出来**，让用户自己决定删不删。
///
/// ⚠️ **绝不自动删**：那是用户的磁盘，里面可能有他手工放进去的字体。
/// ⚠️ 也别把它"搬"进程序目录：两个库并存正是这次要收掉的问题，搬家只是把它换个地方复发。
pub fn legacy_user_dir_report() -> Option<(PathBuf, usize, u64)> {
    let dir = crate::path::user_home()?.join(".yekitrim").join("fonts");
    if !dir.is_dir() {
        return None;
    }
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let p = entry.path();
        if p.is_dir() {
            if font_file_in(&p).is_some() {
                count += 1;
                bytes += dir_size(&p);
            }
        } else if is_font_file_name(&name) {
            count += 1;
            bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    (count > 0).then_some((dir, count, bytes))
}

/// 递归求目录体积（记字节，给上面那条巡检报数用）。
fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            total += dir_size(&p);
        } else {
            total += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    total
}

/// 把库里**路径含非 ASCII 字符**的条目搬成 ASCII 形状（幂等；失败只跳过那一条）。
///
/// 做三件事，**只在需要时动**（能不动就不动，绝不半途删/覆盖任何文件）：
///   ① `FONT.toml` 补 `id = <旧目录名>`（顺手补 `family` / `display_name` / `sha256`）；
///   ② 目录里的字体文件若不是 ASCII 名 ⇒ 改名成 `font.<ext>`；
///   ③ 目录名若不是 ASCII ⇒ 改名成 [`ascii_slug`]（重名时加 `-2` / `-3` …）。
///
/// # 为什么值得跑这一趟
///
/// 底座那条出片腿把字体路径塞进 ffmpeg 的 `drawtext`，**非 ASCII 路径**会让 ffmpeg
/// 每条字幕访问违例一次（见 [`ascii_slug`] 的长注释）。一次性迁移比“每次出片拷一份别名”
/// 更稳：库里从此没有非 ASCII 路径，任何宿主都不会再踩。
///
/// 幂等判据就是“名字还是不是 ASCII / `FONT.toml` 里有没有 id” —— 跑第二遍是空转。
/// 返回改过的条目（`旧名 → 新名`）给人看；空 = 什么都不用做。
pub fn migrate_ascii_layouts(root: &Path) -> Vec<String> {
    let mut moved = Vec::new();
    for (slug, dir, path, meta) in dir_entries(root) {
        let id = entry_id(&slug, &meta);
        let file_name = path.file_name().and_then(|f| f.to_str()).unwrap_or("").to_string();
        let need_meta = meta.id.trim().is_empty();
        let need_file = !file_name.is_ascii();
        let need_dir = !slug.is_ascii();
        if !(need_meta || need_file || need_dir) {
            continue;
        }
        // ① 先把 id 落进元数据：**即便后面改名失败，这一支也不会失联**（解析走元数据那条路）
        if need_meta {
            let mut m = meta.clone();
            m.id = id.clone();
            if m.display_name.trim().is_empty() {
                m.display_name = id.clone();
            }
            if m.family.trim().is_empty() {
                m.family = cached_family(&path).unwrap_or_else(|| id.clone());
            }
            if m.sha256.is_empty() {
                if let Ok(bytes) = std::fs::read(&path) {
                    m.sha256 = sha256_hex(&bytes);
                }
            }
            if write_meta(&dir, &m).is_err() {
                continue;
            }
        }
        // ② 字体文件名也必须是 ASCII（老目录里那支可能就叫中文名）
        if need_file {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("ttf").to_ascii_lowercase();
            let to = dir.join(format!("font.{ext}"));
            if to.exists() || std::fs::rename(&path, &to).is_err() {
                continue;
            }
        }
        // ③ 目录名 → ASCII slug
        if need_dir {
            let base = ascii_slug(&id, &id);
            let mut to = root.join(&base);
            let mut n = 2;
            while to.exists() && n < 100 {
                to = root.join(format!("{base}-{n}"));
                n += 1;
            }
            match std::fs::rename(&dir, &to) {
                Ok(()) => {
                    let new_name = to.file_name().and_then(|f| f.to_str()).unwrap_or("?").to_string();
                    moved.push(format!("{slug} → {new_name}"));
                }
                // 文件被占用（Windows 常见：预览正读着它）⇒ 放过这一条，下次启动再试
                Err(e) => tracing::warn!("字体库迁移：{} 改名失败（下次启动再试）：{e}", dir.display()),
            }
        } else if need_file || need_meta {
            moved.push(format!(
                "{slug}（就地{}）",
                if need_file { "改字体文件名" } else { "补元数据" }
            ));
        }
    }
    // 老平铺布局：文件名含非 ASCII 的散文件**也是**非 ASCII 路径，搬成目录布局
    let mut flat: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let p = e.path();
            if name.starts_with('.') || !p.is_file() || !is_font_file_name(&name) || name.is_ascii() {
                continue;
            }
            flat.push(p);
        }
    }
    for p in flat {
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").trim().to_string();
        if !is_safe_id(&stem) {
            continue;
        }
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("ttf").to_ascii_lowercase();
        let dest = root.join(ascii_slug(&stem, &stem));
        if dest.exists() || std::fs::create_dir_all(&dest).is_err() {
            continue;
        }
        let target = dest.join(format!("font.{ext}"));
        if std::fs::rename(&p, &target).is_err() {
            let _ = std::fs::remove_dir(&dest);
            continue;
        }
        let family = cached_family(&target).unwrap_or_else(|| stem.clone());
        let sha = std::fs::read(&target).map(|b| sha256_hex(&b)).unwrap_or_default();
        let meta = FontMeta {
            id: stem.clone(),
            display_name: stem.clone(),
            family,
            sha256: sha,
            added_at: chrono::Utc::now().to_rfc3339(),
            ..Default::default()
        };
        if write_meta(&dest, &meta).is_ok() {
            let new_name = dest.file_name().and_then(|f| f.to_str()).unwrap_or("?").to_string();
            moved.push(format!("{stem}.{ext} → {new_name}"));
        }
    }
    moved
}


pub fn is_font_file_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| FONT_EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

/// id（= 文件名去扩展名）能不能当"配置值 + 请求参数"用。
///
/// 只拦路径穿越与控制字符：空格 / 括号 / 汉字都放行（文件名本来就长这样，
/// 拦了会变成"列得出来但选不了"）。CSS 那边另有转义，不靠这里。
pub fn is_safe_id(id: &str) -> bool {
    let id = id.trim();
    !id.is_empty()
        && id != "."
        && id != ".."
        && !id.starts_with('.')
        && id.chars().count() <= 128
        && !id.chars().any(|c| c == '/' || c == '\\' || c == ':' || c.is_control())
}

/// 一个可选字体。`bytes` 只用来给人看体积，不影响功能。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct FontEntry {
    /// 文件名去扩展名 —— 配置里存的就是它
    pub id: String,
    /// 带扩展名的文件名（渲染服务器 `/vt-fonts/<file>` 用）
    pub file: String,
    /// 字体的字族名（读 `name` 表；失败时等于 id）
    pub family: String,
    pub bytes: u64,
    /// ⚠️ **v2 元数据**（见 docs/plan/font-library.md §5/§10）
    /// 给人看的名字（可改；配置里存的始终是 `id`）
    #[serde(default)]
    pub display_name: String,
    /// ⚠️ **字体内部族名**（唯一权威：预览 / 出片 @font-face / Takumi 三处都认它）
    #[serde(default)]
    pub family_internal: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub scripts: Vec<String>,
    #[serde(default)]
    pub emoji: bool,
    /// 程序自带 ⇒ **不可删除**（删除返回"程序自带字体不可删除"）
    ///
    /// ⚠️ 判据是**动态的**：元数据写了 `builtin = true` **且** id 仍在 `BUILTIN_FONTS` 清单里
    /// 才算 —— 只认元数据会让**换下来的旧自带字体**永远标着「程序自带」且删不掉（见 `scan_dirs`）。
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub is_default: bool,
    /// ⚠️ **只对自带字体有意义**：清单里有它、但磁盘上还没落盘 ⇒ `false`（界面给"下载"）。
    /// 用户自己导入的字体永远是 `true`（它本来就在盘上）。
    /// ⚠️ 不要写 `#[serde(default = …)]`：`FontEntry` **只 `Serialize`、从不 `Deserialize`**
    /// （它是"发给界面看的视图"，不是配置），那个 default 永远不会被调用，只会换来一条
    /// "function is never used" 警告。构造点**显式给值**。
    pub installed: bool,
    /// **老平铺布局**（散文件直接躺在库根目录，id = 文件名去扩展名）⇒ `true`。
    ///
    /// v2 目录布局与未下载的自带字体都是 `false`。界面据此打「老布局」标签 ——
    /// 这类条目常是当年手工拷进来的系统字体（`msyh.ttc` / `seguiemj.ttf`），
    /// 与"正经导入/下载进来的库字体"不是一回事，得让人一眼分出来。
    #[serde(default)]
    pub flat: bool,
    /// **彩色字形能力**（`"cbdt"` / `"sbix"` / `"colrv0"` / `"colrv1"` / `"svg"` / `"none"`）。
    ///
    /// 界面据此：① 给 emoji 候选标"能不能用"② 对 COLRv1/SVG 明确警告"这个格式出不了彩色"。
    /// ⚠️ 对**纯文字字体**它通常是 `"none"` —— 那是正常的，不是缺陷，别当成错误提示。
    #[serde(default)]
    pub color_format: String,
}

fn ext_rank(ext: &str) -> usize {
    let ext = ext.to_ascii_lowercase();
    FONT_EXTS.iter().position(|x| *x == ext).unwrap_or(FONT_EXTS.len())
}

/// 列出库里的字体（磁盘为准，按 id 排序）。目录不存在 → 空列表，不报错。
pub fn scan() -> Vec<FontEntry> {
    scan_all(&dir())
}

pub fn scan_at(root: &Path) -> Vec<FontEntry> {
    let mut cands: Vec<(String, String, PathBuf)> = Vec::new(); // (id, file, path)
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    for entry in rd.flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        // 点开头的一律跳过（与 `scan_packages` 同规矩：暂存目录、系统伴随文件都在这一档）
        if file.starts_with('.') || !is_font_file_name(&file) {
            continue;
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(id) = Path::new(&file).file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !is_safe_id(id) {
            continue;
        }
        cands.push((id.to_string(), file, path));
    }
    // 同名不同扩展名（foo.ttf + foo.otf）只留一条，按 ttf > otf > ttc
    cands.sort_by(|a, b| {
        let ea = ext_rank(Path::new(&a.1).extension().and_then(|e| e.to_str()).unwrap_or(""));
        let eb = ext_rank(Path::new(&b.1).extension().and_then(|e| e.to_str()).unwrap_or(""));
        a.0.to_lowercase().cmp(&b.0.to_lowercase()).then(ea.cmp(&eb))
    });
    let mut out: Vec<FontEntry> = Vec::new();
    for (id, file, path) in cands {
        if out.iter().any(|e| e.id.eq_ignore_ascii_case(&id)) {
            continue;
        }
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let family = cached_family(&path).unwrap_or_else(|| id.clone());
        // ⚠️ `installed: true` **必须显式写**：`..Default::default()` 给的是 `false`
        // （bool 的默认值），那会把这支**真实存在**的老布局字体标成"没装"。
        // `flat: true` 同理：能走到这里就是老平铺布局（散文件直接在库根目录）。
        out.push(FontEntry { id, file, family, bytes, installed: true, flat: true, ..Default::default() });
    }
    out.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()));
    out
}

/// 把配置里的字体 id 解析成库内文件。解析不到 → `None`（调用方回退系统字体栈）。
pub fn resolve_file(id: &str) -> Option<PathBuf> {
    resolve_all(&dir(), id)
}

pub fn resolve_file_at(root: &Path, id: &str) -> Option<PathBuf> {
    let id = id.trim();
    if !is_safe_id(id) {
        return None;
    }
    for ext in FONT_EXTS {
        let p = root.join(format!("{id}.{ext}"));
        if p.is_file() {
            return Some(p);
        }
    }
    // 扩展名大小写不一致（Linux / 手工改名）时回退到扫目录比对
    let rd = std::fs::read_dir(root).ok()?;
    for entry in rd.flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        if !is_font_file_name(&file) {
            continue;
        }
        let stem = Path::new(&file).file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if stem.eq_ignore_ascii_case(id) {
            let p = entry.path();
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

// ===== 字族名（`name` 表）=====

type FamilyCache = Mutex<HashMap<String, (u64, u64, Option<String>)>>;

fn family_cache() -> &'static FamilyCache {
    static C: OnceLock<FamilyCache> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// 读字族名，按 `(路径, mtime, 体积)` 缓存 —— 一次请求扫十几个字体文件、
/// 每个几十 MB，不缓存就是每次刷新都把库整个读一遍。
fn cached_family(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok();
    let mtime = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let key = path.to_string_lossy().to_string();
    if let Ok(c) = family_cache().lock() {
        if let Some((m, l, fam)) = c.get(&key) {
            if *m == mtime && *l == len {
                return fam.clone();
            }
        }
    }
    let fam = std::fs::read(path).ok().and_then(|b| family_of(&b));
    if let Ok(mut c) = family_cache().lock() {
        c.insert(key, (mtime, len, fam.clone()));
    }
    fam
}

/// 从字体字节里读字族名（nameID 1，退化到 16）。
///
/// 手写的最小 `name` 表解析：`swash` 只在 `wgpu` feature 下才有，而这里只是给人看的标签，
/// 不值得为它拉一个依赖进来。`ttf` / `otf` / `ttc`（取集合里第一个字体）都认。
pub fn family_of(bytes: &[u8]) -> Option<String> {
    let base = sfnt_base(bytes)?;
    let num_tables = be_u16(bytes, base + 4)? as usize;
    if num_tables == 0 || num_tables > 512 {
        return None;
    }
    let mut name_table = None;
    for i in 0..num_tables {
        let rec = base + 12 + i * 16;
        let Some(tag) = bytes.get(rec..rec + 4) else { break };
        if tag == &b"name"[..] {
            name_table = be_u32(bytes, rec + 8).map(|off| off as usize);
            break;
        }
    }
    let table = bytes.get(name_table?..)?;
    let count = be_u16(table, 2)? as usize;
    let string_off = be_u16(table, 4)? as usize;

    // 打分取最优：(nameID 优先 1 再 16, 平台优先 Windows, 语言优先中文再英文)
    let mut best: Option<((u8, u8, u8), String)> = None;
    for i in 0..count.min(512) {
        let rec = 6 + i * 12;
        let Some(platform) = be_u16(table, rec) else { break };
        let Some(encoding) = be_u16(table, rec + 2) else { break };
        let Some(language) = be_u16(table, rec + 4) else { break };
        let Some(name_id) = be_u16(table, rec + 6) else { break };
        let Some(length) = be_u16(table, rec + 8) else { break };
        let Some(offset) = be_u16(table, rec + 10) else { break };
        let name_rank = match name_id {
            1 => 0u8,
            16 => 1,
            _ => continue,
        };
        if length == 0 || length > 512 {
            continue;
        }
        let start = string_off + offset as usize;
        let Some(raw) = table.get(start..start + length as usize) else { continue };
        let Some(text) = decode_name(platform, encoding, raw) else { continue };
        let text = text.trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string();
        if text.is_empty() {
            continue;
        }
        let platform_rank = match platform {
            3 => 0u8, // Windows / UTF-16BE
            0 => 1,   // Unicode
            1 => 2,   // Mac Roman
            _ => 3,
        };
        let lang_rank = match (platform, language) {
            (3, 0x0804) | (3, 0x0404) | (3, 0x0C04) | (3, 0x1004) => 0u8, // zh-CN/TW/HK/SG
            (3, 0x0409) => 1,                                            // en-US
            (3, _) => 2,
            (_, 0) => 0,
            _ => 2,
        };
        let score = (name_rank, platform_rank, lang_rank);
        if best.as_ref().is_none_or(|(s, _)| score < *s) {
            best = Some((score, text));
        }
    }
    best.map(|(_, t)| t)
}

/// 字体的**彩色能力**（2026-10-02 加：emoji 字体选错格式会静默坏掉）。
///
/// ⚠️ 为什么需要这个：emoji 字体的格式**不是都能用**。封面走 Takumi 静态光栅化，
/// 它对彩色字形的支持是**硬编码**的（`takumi-core` 只认 `ColorGlyphFormat::ColrV0`），
/// 实测四档：
///
/// | 格式 | 结果 |
/// |---|---|
/// | `CBDT`/`CBLC` 位图 | 彩色正常（当前自带那支就是它） |
/// | `sbix` 位图 | 彩色正常 |
/// | `COLR` **v0** | 彩色正常 |
/// | `COLR` **v1** | **emoji 整片消失**（不是变黑白 —— 更隐蔽：无报错、界面还显示已装） |
/// | `SVG ` in-font | 不支持 |
///
/// ⚠️ **口径是「能不能真的出彩色」，不是「是不是 CBDT」**：
/// COLRv0 / sbix 同样能用，一律要求 CBDT 会误伤它们。真正要拦的只有 COLRv1 与 SVG。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorGlyphSupport {
    /// 有彩色字形表，且实测能渲染
    Color(ColorGlyphFormatKind),
    /// 有彩色字形表，但**当前渲染器出不来**（COLRv1 / SVG）—— 界面要明确警告
    Unsupported(ColorGlyphFormatKind),
    /// 没有任何彩色字形表（纯轮廓字体；当 emoji 字体用只会出黑白或豆腐块）
    None,
}

/// 彩色字形表的种类（诊断与界面提示用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorGlyphFormatKind {
    /// `CBDT`+`CBLC` 位图（Google 的 Noto Color Emoji 走的这条）
    Cbdt,
    /// `sbix` 位图（Apple 的 Apple Color Emoji）
    Sbix,
    /// `COLR` v0（矢量分层，Takumi 认这条）
    ColrV0,
    /// `COLR` v1（矢量分层 + 渐变/变换，**Takumi 不认**）
    ColrV1,
    /// 字体里内嵌 SVG 文档（**Takumi 不认**）
    Svg,
}

impl ColorGlyphFormatKind {
    /// 给界面看的短名（纯 ASCII）。
    pub fn short(&self) -> &'static str {
        match self {
            Self::Cbdt => "CBDT",
            Self::Sbix => "sbix",
            Self::ColrV0 => "COLRv0",
            Self::ColrV1 => "COLRv1",
            Self::Svg => "SVG",
        }
    }
    /// 界面契约串（**改值要同步前端** `FontLibrary.vue`）。
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Cbdt => "cbdt",
            Self::Sbix => "sbix",
            Self::ColrV0 => "colrv0",
            Self::ColrV1 => "colrv1",
            Self::Svg => "svg",
        }
    }
}

/// 判一支字体的彩色能力（读 sfnt 表目录，**不解析字形数据**）。
///
/// ⚠️ 判据只看**表在不在**（加 COLR 的 version）：这是「这支字体声称有什么格式」，
/// 不是「每个 emoji 都能出彩色」—— 后者的完整判定要真的栅格化一遍（代价远超一次列表请求）。
/// 对界面提示来说前者够用，且不会误报：真正的坑（COLRv1 / SVG）恰好是**表级的**。
pub fn color_glyph_support(bytes: &[u8]) -> ColorGlyphSupport {
    let Some(base) = sfnt_base(bytes) else { return ColorGlyphSupport::None };
    let Some(num) = be_u16(bytes, base + 4) else { return ColorGlyphSupport::None };
    let has = |tag: &[u8; 4]| -> bool {
        // 表目录最多 num 条；越界一律当没有（坏文件不该 panic）
        (0..num as usize).any(|i| {
            let off = base + 12 + 16 * i;
            bytes.get(off..off + 4) == Some(&tag[..])
        })
    };
    // ⚠️ COLR 必须先判、且**要读 version**：v1 是向后兼容的（v1 字体通常也带 v0 子表），
    // 而 Takumi 的判定是读 COLR 的 version 字段 —— 我们必须照它的口径，见 COLR 就当能用。
    if has(b"COLR") {
        return match colr_version(bytes, base, num) {
            Some(v) if v >= 1 => ColorGlyphSupport::Unsupported(ColorGlyphFormatKind::ColrV1),
            _ => ColorGlyphSupport::Color(ColorGlyphFormatKind::ColrV0),
        };
    }
    if has(b"CBDT") && has(b"CBLC") {
        return ColorGlyphSupport::Color(ColorGlyphFormatKind::Cbdt);
    }
    if has(b"sbix") {
        return ColorGlyphSupport::Color(ColorGlyphFormatKind::Sbix);
    }
    if has(b"SVG ") {
        return ColorGlyphSupport::Unsupported(ColorGlyphFormatKind::Svg);
    }
    ColorGlyphSupport::None
}

/// 读 `COLR` 表的 version（0 = v0，1 = v1）。表缺失/越界返回 None。
fn colr_version(bytes: &[u8], base: usize, num: u16) -> Option<u16> {
    for i in 0..num as usize {
        let off = base + 12 + 16 * i;
        if bytes.get(off..off + 4) == Some(&b"COLR"[..]) {
            let table_off = be_u32(bytes, off + 8)? as usize;
            return be_u16(bytes, table_off);
        }
    }
    None
}

/// 彩色能力 → 界面契约串（`"cbdt"` / `"sbix"` / `"colrv0"` / `"colrv1"` / `"svg"` / `"none"`）。
///
/// ⚠️ 返回值是**契约**：前端按它决定标什么角标、显示什么警告，改值要同步前端。
pub fn color_format_tag(support: ColorGlyphSupport) -> &'static str {
    match support {
        ColorGlyphSupport::Color(k) => k.tag(),
        ColorGlyphSupport::Unsupported(k) => k.tag(),
        ColorGlyphSupport::None => "none",
    }
}

/// 从**文件**判彩色能力（只读表目录，不整份读进来 —— 字体可达 14 MB）。
///
/// ⚠️ 只读前 `HEAD_BYTES`：sfnt 表目录固定在文件最前面（12 字节头 + 16×n 条），
/// 表目录本身不会超过几千字节，不需要读整个字体（`scan_all` 是**每次开字体下拉都跑**的热路径）。
pub fn color_format_of_file(path: &Path) -> String {
    use std::io::Read as _;
    // 表目录最多 128 张表 × 16 B + 12 B 头 ≈ 2 KB；给 16 KB 富余（含 ttcf 集合的偏移）
    const HEAD_BYTES: usize = 16 * 1024;
    let Ok(mut f) = std::fs::File::open(path) else { return "none".to_string() };
    let mut buf = vec![0u8; HEAD_BYTES];
    let n = match f.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return "none".to_string(),
    };
    buf.truncate(n);
    color_format_tag(color_glyph_support(&buf)).to_string()
}

/// 这支字体的彩色能力**能不能用**（界面用它决定要不要警告）。
pub fn color_supported(support: ColorGlyphSupport) -> bool {
    matches!(support, ColorGlyphSupport::Color(_))
}

/// sfnt 表目录起点：普通字体从 0 开始，`ttcf` 字体集合取第一个字体的目录偏移。
fn sfnt_base(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 12 {
        return None;
    }
    if bytes.get(0..4)? == &b"ttcf"[..] {
        let first = be_u32(bytes, 12)? as usize;
        if first < 12 || first >= bytes.len() {
            return None;
        }
        Some(first)
    } else {
        Some(0)
    }
}

fn be_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn decode_name(platform: u16, encoding: u16, raw: &[u8]) -> Option<String> {
    // 平台 0（Unicode）/ 平台 3 编码 0,1（UTF-16BE）/ 平台 3 编码 10（UCS-4，少见）之外
    // 一律按单字节（Mac Roman / Latin-1）读 —— 对 ASCII 名字足够。
    if platform == 0 || (platform == 3 && (encoding == 0 || encoding == 1)) {
        if raw.len() % 2 != 0 {
            return None;
        }
        let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        Some(char::decode_utf16(units).map(|r| r.unwrap_or('\u{fffd}')).collect())
    } else {
        Some(raw.iter().map(|&b| b as char).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_file_name_accepts_only_known_extensions() {
        assert!(is_font_file_name("LXGWWenKai-Regular.ttf"));
        assert!(is_font_file_name("思源宋体.OTF"));
        assert!(is_font_file_name("msyh.ttc"));
        assert!(!is_font_file_name("cover.jpg"));
        assert!(!is_font_file_name("woff2.woff2"));
        assert!(!is_font_file_name("noext"));
    }

    #[test]
    fn safe_id_rejects_traversal_but_allows_cjk_and_spaces() {
        assert!(is_safe_id("LXGW WenKai (v1.0)"));
        assert!(is_safe_id("思源黑体"));
        assert!(!is_safe_id(""));
        assert!(!is_safe_id(".."));
        assert!(!is_safe_id("../secret"));
        assert!(!is_safe_id(r"..\secret"));
        assert!(!is_safe_id("C:evil"));
        assert!(!is_safe_id(".hidden"));
    }

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yekitrim-fonts-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_file_finds_by_stem_and_ignores_case() {
        let root = temp_root("resolve");
        std::fs::write(root.join("My Font.ttf"), b"x").unwrap();
        assert!(resolve_file_at(&root, "My Font").is_some());
        assert!(resolve_file_at(&root, "my font").is_some());
        assert!(resolve_file_at(&root, "missing").is_none());
        // 穿越请求一律拒绝（第一道闸；真正的围栏由调用方的 canonicalize 负责）
        assert!(resolve_file_at(&root, "../My Font").is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scan_keeps_one_entry_per_id_and_skips_junk() {
        let root = temp_root("scan");
        std::fs::write(root.join("Dup.ttc"), b"x").unwrap();
        std::fs::write(root.join("Dup.otf"), b"x").unwrap();
        std::fs::write(root.join("Dup.ttf"), b"x").unwrap();
        std::fs::write(root.join("pic.png"), b"x").unwrap();
        std::fs::write(root.join(".hidden.ttf"), b"x").unwrap();
        let list = scan_at(&root);
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!(list[0].id, "Dup");
        assert_eq!(list[0].file, "Dup.ttf", "同名多扩展名按 ttf > otf > ttc");
        // 解析不出字族名时退回 id（这几个文件根本不是字体）
        assert_eq!(list[0].family, "Dup");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scan_missing_dir_is_empty_not_error() {
        let missing = std::env::temp_dir().join("yekitrim-fonts-does-not-exist-xyz");
        assert!(scan_at(&missing).is_empty());
    }

    #[test]
    fn family_of_rejects_garbage() {
        assert!(family_of(b"").is_none());
        assert!(family_of(b"not a font at all").is_none());
    }

    /// 只读诊断：本机系统字体能不能读出字族名。
    /// `cargo test -p yekitrim-core --lib diagnose_font_family -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn diagnose_font_family() {
        for p in ["C:/Windows/Fonts/msyh.ttc", "C:/Windows/Fonts/simhei.ttf"] {
            let path = Path::new(p);
            if !path.is_file() {
                println!("跳过（不存在）: {p}");
                continue;
            }
            let bytes = std::fs::read(path).unwrap();
            println!("{p} → {:?}", family_of(&bytes));
        }
    }
}


// ===== 字体库 v2：每支一个目录 + FONT.toml 元数据 =====
//
// 布局：`resources/fonts/<slug>/font.ttf|otf` + `FONT.toml` + `LICENSE.txt`，**id = 目录名**。
// 老布局（直接把 `.ttf` 丢在 `resources/fonts/` 下）**继续支持**：`scan_all` / `resolve_all` 两者都看，
// 目录布局优先。原因见 `docs/plan/font-library.md`：id 稳定、能存元数据、能让"预览 / 出片 / Takumi"
// 三处只认一个 **内部族名**（`FONT.toml` 的 `family`），而不是各自拼名字。

/// 元数据文件名（放在每支字体的目录里）。
pub const META_FILE: &str = "FONT.toml";

/// 一支字体的元数据。`family` 是**字体内部族名**（权威），`display_name` 只是给人看的标签。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FontMeta {
    /// **配置/界面里存的那串 id**（= 导入时给的名字，可含汉字/空格）。
    ///
    /// ⚠️ **2026-10-02 起 id 不再等于目录名**（见 [`ascii_slug`] 与 `docs/plan/font-library.md` §24）：
    /// 目录名与字体文件名一律落 **ASCII**，给人看的那串只住在这里（`display_name` 是它的兄弟，
    /// 可以再改；**id 是配置里存的值，不许变**）。
    ///
    /// 为什么必须 ASCII：底座出片那条腿把字体路径塞进 ffmpeg 的
    /// `drawtext=fontfile=…`，路径里**任何一个非 ASCII 字符**都会让 ffmpeg 访问违例
    /// （`0xC0000005`）—— 实测 240 帧 4.2s → 80.3s，且**一个字都画不出来**。
    /// ffmpeg 我们改不了，所以只能在"落盘形状"这一层保证纯 ASCII。
    ///
    /// **空 = 老库还没迁移** ⇒ 回落"目录名就是 id"（存量零迁移）。
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub scripts: Vec<String>,
    #[serde(default)]
    pub emoji: bool,
    /// 程序自带 ⇒ 不可删除
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub is_default: bool,
    /// ⚠️ 这条注释原来是"当前恒为空串（本仓没有 hasher 依赖）"—— **已过时**：
    /// `sha256_hex` 就在本文件里，导入与自带字体安装都会写它。它是"盘上这支到底是不是
    /// 当初那支"的唯一凭据（用户手动替换过字体时也一样）。
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub added_at: String,
}

/// 目录里那一支字体文件（`ttf` > `otf` > `ttc`）。
pub fn font_file_in(dir: &Path) -> Option<PathBuf> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut hits: Vec<(usize, PathBuf)> = Vec::new();
    for entry in rd.flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        if file.starts_with('.') || !is_font_file_name(&file) {
            continue;
        }
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        hits.push((ext_rank(ext), p));
    }
    hits.sort_by_key(|(r, _)| *r);
    hits.into_iter().next().map(|(_, p)| p)
}

pub fn read_meta(slug_dir: &Path) -> Option<FontMeta> {
    let text = std::fs::read_to_string(slug_dir.join(META_FILE)).ok()?;
    toml::from_str(&text).ok()
}

pub fn write_meta(slug_dir: &Path, meta: &FontMeta) -> std::io::Result<()> {
    let text = toml::to_string_pretty(meta).unwrap_or_default();
    std::fs::write(slug_dir.join(META_FILE), text)
}

/// 库根下的**目录布局（v2）条目**：`(目录名, 目录, 字体文件, 元数据)`。
///
/// 缺 `FONT.toml` 的目录**也算**一条（老库 / 手工拷进来的），元数据给默认值 ——
/// 那时 id 回落目录名（见 [`entry_id`]），与改造前的行为一字不差。
fn dir_entries(root: &Path) -> Vec<(String, PathBuf, PathBuf, FontMeta)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else { return out };
    for entry in rd.flatten() {
        let slug = entry.file_name().to_string_lossy().to_string();
        let dir = entry.path();
        if slug.starts_with('.') || !dir.is_dir() || !is_safe_id(&slug) {
            continue;
        }
        let Some(path) = font_file_in(&dir) else { continue };
        let meta = read_meta(&dir).unwrap_or_default();
        out.push((slug, dir, path, meta));
    }
    out
}

/// 一个目录条目的**有效 id**：`FONT.toml.id` 优先，缺省回落目录名（老库零迁移）。
fn entry_id(slug: &str, meta: &FontMeta) -> String {
    let id = meta.id.trim();
    if !id.is_empty() && is_safe_id(id) {
        id.to_string()
    } else {
        slug.to_string()
    }
}

/// id → **目录**（目录布局）。先试“目录名就是 id”（老库 / 自带字体 / 手建目录那条路），
/// 再按 `FONT.toml.id` 扫一遍 —— 迁移成 ASCII slug 之后，id 只住在元数据里。
fn dir_for_id(root: &Path, id: &str) -> Option<PathBuf> {
    let id = id.trim();
    if !is_safe_id(id) {
        return None;
    }
    let direct = root.join(id);
    if direct.is_dir() && font_file_in(&direct).is_some() {
        return Some(direct);
    }
    dir_entries(root)
        .into_iter()
        .find(|(slug, _, _, meta)| entry_id(slug, meta).eq_ignore_ascii_case(id))
        .map(|(_, dir, _, _)| dir)
}

/// 库内**目录名与字体文件名**的 ASCII slug —— 只由 id 派生，1:1 且稳定。
///
/// # 为什么必须 ASCII（一次线上事故的收口）
///
/// 底座出片那条腿把字体路径塞进 ffmpeg 的 `drawtext=fontfile='…'`。ffmpeg 那条路
/// **打不开含非 ASCII 字符的路径**（FreeType 拿 UTF-8 字节按 ANSI 打开），而且打不开时
/// **不报错、直接访问违例**（`0xC0000005`）—— 表现是**每条字幕崩一次**：实测 240 帧
/// 4.2s → 80.3s，且 0 行字幕 / 0 条弹幕画出来。裸 ffmpeg 可复现：同一支字体拷到中文路径就崩，
/// 路径不存在也崩出同一个退出码。ffmpeg 与底座都不在本仓手里，所以
/// **只能保证库内落盘路径是纯 ASCII**；给人看的那串住 `FONT.toml`（`id` / `display_name`）。
///
/// 形状：`<hint>-<sha256(id) 前 12 位>`。hint 取 id/显示名里能用的 ASCII 片段
/// （纯 ASCII 名保持可读，全中文退化成 `font-`），**只是给人排查用的前缀**。
/// 由 id 派生（不是内容）⇒ 重复导入不换目录、改显示名不换目录、两个 id 绝不撞同一个目录。
pub fn ascii_slug(id: &str, hint: &str) -> String {
    const MAX_HINT: usize = 32;
    let mut h = String::new();
    for ch in hint.chars() {
        if h.len() >= MAX_HINT {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            h.push(ch.to_ascii_lowercase());
        } else if !h.is_empty() && !h.ends_with('-') {
            h.push('-');
        }
    }
    let h = h.trim_matches('-');
    let hint = if h.is_empty() { "font" } else { h };
    let digest = sha256_hex(id.as_bytes());
    format!("{hint}-{}", &digest[..12])
}

/// 目录布局（v2）的扫描。
pub fn scan_dirs(root: &Path) -> Vec<FontEntry> {
    let mut out = Vec::new();
    for (slug, _dir, path, meta) in dir_entries(root) {
        let id = entry_id(&slug, &meta);
        let internal = if meta.family.trim().is_empty() {
            cached_family(&path).unwrap_or_else(|| id.clone())
        } else {
            meta.family.clone()
        };
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        // ⚠️ 在 struct 字面量**之前**算：`id` 会被 `display_name` 那行 move 走，之后不能再借。
        let builtin = meta.builtin && is_builtin_id(&id);
        out.push(FontEntry {
            id: id.clone(),
            file: path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
            family: internal.clone(),
            bytes,
            // ⚠️ 兜底是 **id**（不是目录名）：迁移之后目录名是 ASCII slug，拿它显示等于
            // 把 `乐米波波体` 变成 `font-9c1d…`，用户就再也对不上自己选的那支了。
            display_name: if meta.display_name.trim().is_empty() { id } else { meta.display_name },
            family_internal: internal,
            license: meta.license,
            scripts: meta.scripts,
            emoji: meta.emoji,
            // ⚠️ **"自带"必须同时满足两件事：元数据这么写 + id 仍在当前清单里**（2026-10-02）。
            //
            // 只看 `meta.builtin` 会留下**孤儿**：自带字体换掉之后，用户在旧版下载到盘上的那支
            // 元数据照样写着 `builtin = true`，于是界面永远标它「程序自带」、`remove_font` 又
            // 永远拒删 —— 用户**删不掉一支已经不是自带的字体**（实测：霞鹜文楷 Lite 被
            // Noto Sans SC 换下之后，13.23 MB 留在库里且删不掉）。
            //
            // 判据必须是**动态的**：清单是唯一权威（`is_builtin_id`），元数据只是它的副本。
            // 副本一旦与清单脱节就以清单为准 —— 这样下次再换自带字体，旧那支会**自动降级**
            // 成普通条目，不需要谁记得去清理。
            builtin,
            // ⚠️ **emoji 字体强制 `is_default = false`**（字体职责分离，2026-10-02）。
            // 这是 A4 的**第三处、也是最关键的一处**拦截：`FONT.toml` 是文本文件，
            // 用户能手改成 `emoji = true` + `is_default = true`；而 `is_default` 有两个真消费者——
            //   * `render/server.rs::preferred_fallback_family` 用它选**字体栈首项**
            //     ⇒ 栈首变 emoji 族 ⇒ **整条栈的中文全部落空**；
            //   * `cover.rs::cover_font_files_in` 用它把该支排到**注册序最前**
            //     ⇒ 封面上"认不出的族名"全落到 emoji 字体。
            // 两处都不是"emoji 当正文"那么直白，所以**必须在数据出口这一层掐掉**，
            // 而不是指望每个消费者各判一次（那正是本仓反复吃亏的"四处各写一份"）。
            is_default: meta.is_default && !meta.emoji,
            // 目录布局不是平铺（`flat` 只属于散文件直接躺在库根目录的老布局）
            flat: false,
            // 它在磁盘上 scanned 出来的 ⇒ 当然是"装好了"（`installed: false` 只属于"清单里有、盘上没有"）
            installed: true,
            color_format: color_format_of_file(&path),
        });
    }
    out
}

/// **「库里设了默认的那支」的唯一查询入口**（默认位给正文用）。
///
/// 返回**内部族名**（与 `fallback_chain()` 首项、`SYSTEM_FONT_STACK` 同一种东西）。
///
/// ⚠️ **为什么要有这个函数（2026-10-06）**：这个判据原先在 `yekitrim-polish` 里抄了两份
/// （`render::server::preferred_fallback_family` 与 `cover::cover_font_files_in`），各自
/// `.find(|e| e.is_default)` 一遍。而**新工程的「默认字体」这条路上一个消费者都没有**：
/// `polish init` 写死 `render_core::default_font()` 的 `"Noto Sans SC"`，用户设的默认位
/// 对新切出来的片段完全无效（用户报「每次切出切片，字体都不是字体库设置的默认字体」）。
/// 现在 `cmd_init` 也走这里 —— **判据只此一处**。
///
/// 用**族名**而不是 id：`polish.toml` 里 `font` 那栏两条路都认（id 走库内解析、族名走系统栈），
/// 而"库里默认哪支"要表达的是"排正文用哪支"，与栈首同一个语义。
///
/// emoji 字体**天然出不来**：`scan_dirs` 出口已把它强制成 `is_default = false`（职责分离）。
/// 未安装的自带字体也出不来（`installed` 过滤）—— 否则会把一个盘上还没有的文件选成新工程默认。
pub fn default_text_family(root: &Path) -> Option<String> {
    scan_all(root)
        .into_iter()
        .filter(|e| e.installed && !e.emoji)
        .find(|e| e.is_default)
        .map(|e| {
            let fam = e.family_internal.trim();
            if fam.is_empty() { e.id } else { fam.to_string() }
        })
}

/// 两种布局合并：**目录布局优先**（同名 id 时目录赢）。
///
/// ⚠️ **还没下载的自带字体也会出现在结果里**（`installed: false`）—— 界面要能显示
/// "有这支、可以下载"。所以**别把 `scan_all().len()` 当"库里有多少字体"用**，
/// 要判"真能用"请过滤 `installed`（或直接 `resolve_all` 试解析）。
/// 多根扫描：**按传入顺序优先**（`roots[0]` 赢），后面的只补前面没有的 id。
///
/// 这是"用户库优先、全局补缺"的落点（`owner::asset_dirs` 给出的就是那个顺序）。
/// 空根不存在时照常扫（`scan_all` 对不存在的目录返回空），所以调用方不必先过滤。
pub fn scan_all_roots(roots: &[PathBuf]) -> Vec<FontEntry> {
    let mut out: Vec<FontEntry> = Vec::new();
    for root in roots {
        for e in scan_all(root) {
            match out.iter().position(|x| x.id.eq_ignore_ascii_case(&e.id)) {
                // 前面那层已经有了 ⇒ 通常以它为准。
                None => out.push(e),
                // ⚠️ **例外：前面那层只是"清单里的占位"（`installed: false`，盘上没有），
                // 而后面这层真装着** ⇒ 换成后面那个。
                //
                // 不换的后果（实测）：自带字体装在**全局库**里，而每个用户库的
                // `scan_all` 都会补一条"自带字体、未安装"的占位。用户库在前 ⇒ 占位赢
                // ⇒ 界面永远显示"未安装"（还有一个「下载」按钮），可它其实好好的 ——
                // 而且出片时是能用的。**显示与实际不一致**正是这批改动要消灭的形态。
                Some(i) => {
                    if !out[i].installed && e.installed {
                        out[i] = e;
                    }
                }
            }
        }
    }
    out
}

pub fn scan_all(root: &Path) -> Vec<FontEntry> {
    let mut out = scan_dirs(root);
    for e in scan_at(root) {
        if out.iter().any(|x| x.id.eq_ignore_ascii_case(&e.id)) {
            continue;
        }
        // 老布局没有元数据：显示名沿用"解析出的族名"（与改动前前端看到的完全一致，不回归）
        out.push(FontEntry { display_name: e.family.clone(), family_internal: e.family.clone(), ..e });
    }
    // 清单里有、盘上没有的自带字体：补一条"未安装"的占位（用户自己导入的不会走到这里）
    for b in BUILTIN_FONTS {
        if out.iter().any(|x| x.id.eq_ignore_ascii_case(b.id)) {
            continue;
        }
        out.push(FontEntry {
            id: b.id.to_string(),
            // 还没落盘 ⇒ 没有文件名；给约定值，界面不该拿它拼 URL（未安装的没有 URL）
            file: String::new(),
            family: b.family.to_string(),
            bytes: 0,
            display_name: b.display_name.to_string(),
            family_internal: b.family.to_string(),
            license: b.license.to_string(),
            scripts: Vec::new(),
            emoji: b.id.to_ascii_lowercase().contains("emoji"),
            builtin: true,
            is_default: false,
            // 未下载的自带字体走目录布局（下载后落 `<id>/font.ttf`），不是平铺
            flat: false,
            installed: false,
            // ⚠️ 还没落盘也能判：**直接用内嵌字节**（`BUILTIN_FONTS[i].bytes`）——
            // 这样"未装"的卡片也能如实告诉用户这支格式能不能用。
            color_format: color_format_tag(color_glyph_support(b.bytes)).to_string(),
        });
    }
    out.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()));
    out
}

/// 目录布局下把 id 解析成文件。
///
/// ⚠️ **不再是 `root.join(id)`**（2026-10-02）：目录名现在是 ASCII slug，id 住在
/// `FONT.toml` 里。先走“目录名就是 id”那条老路（老库 / 自带字体 / 手建目录），
/// 再按元数据里的 id 扫一遍 —— 两条都对，且**老库不需要先迁移就能解析**。
pub fn resolve_dir_file(root: &Path, id: &str) -> Option<PathBuf> {
    dir_for_id(root, id).and_then(|d| font_file_in(&d))
}

/// 两种布局都能解析（目录优先）。
pub fn resolve_all(root: &Path, id: &str) -> Option<PathBuf> {
    resolve_dir_file(root, id).or_else(|| resolve_file_at(root, id))
}

/// 多根解析（`roots[0]` 优先）—— 用户库里有就用用户的，没有才去全局库找。
pub fn resolve_all_roots(roots: &[PathBuf], id: &str) -> Option<PathBuf> {
    roots.iter().find_map(|r| resolve_all(r, id))
}

/// 字体 id → **页面里该用的 CSS 字族名**（三处字体来源的唯一权威）。
///
/// ⚠️ 这是"预览与实际对不上"的收敛点。同一个 id 在历史上被三处各自解释过：
/// ① WebUI 预览用 `new FontFace(id, …)` 注册；② 出片用 `@font-face{font-family:"<id>"}` 注入；
/// ③ Takumi 静态封面**按字体文件里写的内部族名**注册（`Fonts::register` 读的就是它）。
/// 前两处按 id 一致、第三处对不上 ⇒ 封面与预览/出片用不同字体，且**不报错**。
///
/// 现在统一口径：**库里认这个 id 就返回它的内部族名**（`FONT.toml` 的 `family`，唯一权威），
/// 库外/解析不到的值**原样返回** —— 老工程写 `font = "LXGW WenKai"` 那种"系统字体族名"
/// 的行为必须一字不变（那时它本来就没进库，什么都不该注入）。
pub fn family_for(id: &str) -> String {
    family_for_at(&dir(), id)
}

/// [`family_for`] 的目录版（测试用临时目录，免得读程序目录）。
pub fn family_for_at(root: &Path, id: &str) -> String {
    let id = id.trim();
    if id.is_empty() {
        return String::new();
    }
    scan_all(root)
        .into_iter()
        .find(|e| e.id == id)
        .map(|e| {
            if e.family_internal.trim().is_empty() {
                e.id
            } else {
                e.family_internal
            }
        })
        .unwrap_or_else(|| id.to_string())
}

// ===== 字体解析的唯一入口 =====
//
// ⚠️ **这一节是"用哪支字体"的唯一权威**（2026-10-02 字体职责分离）。
// 在此之前，"字体栈"在四处各写一份、靠注释互相提醒"要同序"：
//   ① \`render/server.rs::SYSTEM_FONT_STACK\`（兜底链）
//   ② \`cover.rs::EMOJI_FONT\`（封面 emoji 特效层）
//   ③ \`webui PolishPreview.vue\` 的字面量兜底
//   ④ \`render/takumi.rs::ensure_emoji_fallback\` 的正则注入
// 四处漂移的症状都是"预览和成片不一样，且不报错"。现在全部改为问这里。
//
// **职责分离**：\`FontPlan\` 把"文字"与"emoji"分成**两个独立字段**，而不是拼成一串。
// 原因是三条消费链的语义根本不同（各自决定怎么用这个槽位）：
//   * 字幕**预览**（浏览器）：可以把 emoji 族追加到 font-family 栈尾；
//   * 封面（Takumi）：**不看 font-family 栈**，看的是**注册顺序**（feed 桶 = 全部已注册字体按注册序）——
//     所以它要的是"把 emoji 排到最后注册"，不是"拼进栈"；
//   * 字幕**成片**（Dhampir 底座）：**根本没有逐字形回退**（find_font 只交出第一个能解析的文件，
//     最终落 ffmpeg \`drawtext=fontfile=<单支>\`）—— 所以它**用不上**这个槽位。
// 把它们统一成"一个字符串栈"会让第二条与第三条都被误导（试过：往栈尾插 emoji 对成片零效果，
// 对封面也几乎无效）。所以这里返回**结构化**的槽位，由消费点自己决定。

/// **字体解析的唯一入口**：给定用户选择，返回角色化的字体组合。
///
/// * \`selected_id\`：配置里那串 id（\`[style.subtitle] font\` / \`[cover] font\`）。
///   空串 = 用户没选（走兜底）。库外/解析不到的值**原样当作系统族名**（老工程行为，一字不变）。
/// * \`text_family\`：正文该用的族名（\`Some\` = 库里认这个 id，返回它的内部族名；
///   \`None\` = 没选或不在库里，消费点走 \`fallback\`）。
/// * \`emoji_family\`：**emoji 补充槽位** —— 库里 \`emoji==true\` 且**已装**的字体，
///   id 稳定序取第一支；库里没有就是 \`None\`（那时消费点回落到系统 emoji 族名栈）。
/// * \`fallback\`：兜底链（**不含** emoji）。空库时逐字等于老的 \`SYSTEM_FONT_STACK\` 语义。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FontPlan {
    pub text_family: Option<String>,
    /// 用户选了、但库里解析不到 ⇒ 原样保留（当系统字体族名用）。
    pub selected_raw: String,
    pub emoji_family: Option<String>,
    pub fallback: Vec<String>,
}

impl FontPlan {
    /// 页面用的 CSS \`font-family\` 串：选中的排最前，后面跟兜底链。
    ///
    /// ⚠️ **不含 emoji** —— emoji 由消费点自己决定插不插、插在哪（见上面的说明）。
    /// ⚠️ **声明名 = 配置里那串 id**（\`@font-face\` 也按 id 声明），别改成内部族名：
    /// 模板与 \`webui/src/polish/fonts.js\` 都按 id 请求，改了就是"注入了但页面请求另一个名字"
    /// —— 浏览器静默回退系统字体（P1-e 收敛的就是这条）。所以这里的"正文族名"用**用户选的 id**，
    /// 而 \`text_family\`（内部族名）是**给 Takumi/底座**那两条不看 id 的链用的。
    pub fn css_stack(&self, selected: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let sel = selected.trim();
        if !sel.is_empty() {
            out.push(sel.to_string());
        }
        for f in &self.fallback {
            if !out.iter().any(|x| x.eq_ignore_ascii_case(f)) {
                out.push(f.clone());
            }
        }
        out
    }
}

/// 库里**至多一支生效**的 emoji 字体：\`emoji == true\` 且**已装**，id 稳定序取第一支。
///
/// ⚠️ **只取已装的**：\`scan_all\` 会把"清单里有、盘上没有"的自带字体也列出来（\`installed: false\`），
/// 拿它当"库里有 emoji 字体"会告诉用户"有"然后在渲染时炸掉 —— 与 \`cover_font_files_in\` 同口径
/// （都要求 \`resolve_all\` 真的解析得到文件）。
///
/// ⚠️ **\`is_default\` 在这里被无视**：emoji 字体的"默认位"没有意义（它不参与正文竞争），
/// 见 \`set_default\` 的拒绝逻辑与 \`read_meta\` 侧的口径。
pub fn emoji_font(root: &Path) -> Option<FontEntry> {
    emoji_font_roots(std::slice::from_ref(&root.to_path_buf()))
}

/// 多根版本（用户库优先、全局补缺）—— 自带 emoji 字体在**全局库**里。
pub fn emoji_font_roots(roots: &[PathBuf]) -> Option<FontEntry> {
    let mut hits: Vec<FontEntry> = scan_all_roots(roots)
        .into_iter()
        .filter(|e| {
            e.emoji && e.installed && resolve_all_roots(roots, &e.id).is_some_and(|p| p.is_file())
        })
        .collect();
    // ⚠️ **优先序：自带(清单内) > 其它，再按 id 稳定序**（2026-10-02，实测踩到）。
    //
    // 为什么不能只按 id 排：库里同时存在**新旧两支 emoji 字体**时（用户装过旧版、而我们换了新 id
    // —— 这正是本次换字体造成的情形），id 字典序会让**旧那支赢**：
    //
    //     "NotoColorEmoji" < "NotoColorEmoji-CBDT"  ⇒ 旧的 COLRv1 被选中，新的 CBDT 永远轮不上
    //
    // 症状极隐蔽：封面上的 emoji **整片消失**（旧 COLRv1 对 Takumi 等于不存在），
    // 而"库里明明有 emoji 字体"、界面也显示它已装 —— 只有对着出图看才发现。
    // 自带字体是我们**验证过格式**的那支（单测钉着它必须是 CBDT / COLRv0），所以让它优先。
    let builtin_ids: Vec<String> = builtin_fonts().iter().map(|b| b.id.to_lowercase()).collect();
    // 自带排在后面（false < true），供最后 pop 取走；同档内再按 id 稳定序
    hits.sort_by(|a, b| {
        let ab = builtin_ids.contains(&a.id.to_lowercase());
        let bb = builtin_ids.contains(&b.id.to_lowercase());
        ab.cmp(&bb)
            .then_with(|| b.id.to_lowercase().cmp(&a.id.to_lowercase()))
    });
    hits.pop()
}

/// 兜底链（**不含 emoji**）：与老的 \`SYSTEM_FONT_STACK\` 同族名同顺序。
///
/// ⚠️ 这是"空库回落态"的**唯一一份**定义。\`render/server.rs::SYSTEM_FONT_STACK\` 常量仍保留
/// （它是文档锚点 + 单测钉子），两者必须逐字一致（有单测钉着）。
pub fn fallback_chain() -> Vec<String> {
    // ⚠️ **2026-10-02 首项由 `LXGW WenKai` 换成 `Noto Sans SC`**（自带中文那支换成了它）。
    // 三项顺序不变（首项 = 库里那支，其余是系统回退），只把首项对上新的内部族名。
    // `Microsoft YaHei` 仍排在 `Noto Sans SC` 之前：它对 Windows 用户是**一定有**的字体，
    // 而"库里那支装好了"由首项保证 —— 所以这个顺序既保了同源、又保了系统兜底。
    vec![
        "Noto Sans SC".to_string(),
        "Microsoft YaHei".to_string(),
        "Noto Sans SC".to_string(),
        "sans-serif".to_string(),
    ]
}

/// **字体解析的唯一入口**（见 \`FontPlan\` 的说明）。所有消费点都必须经它取字体。
pub fn resolve_font_plan(root: &Path, selected_id: Option<&str>) -> FontPlan {
    let selected = selected_id.unwrap_or("").trim().to_string();
    let entries = scan_all(root);

    // 正文族名：用户选的那支（库里认 ⇒ 内部族名；不认 ⇒ None，由消费点走 fallback）。
    //
    // ⚠️ **不许把 emoji 字体选成正文**：它管的是"表情"，拿去排中文就是一个字都出不来。
    // 两道拦截（这里 + \`set_default\`）是"职责分离"落在代码上的地方：
    // 界面已经把 emoji 从正文下拉里滤掉了，但配置是文本文件，用户能手改，
    // 所以**消费侧必须再挡一次**（与路径面三防线同一个理由）。
    let text_family = entries
        .iter()
        .find(|e| e.id.eq_ignore_ascii_case(&selected) && !e.emoji && e.installed)
        .map(|e| {
            if e.family_internal.trim().is_empty() {
                e.id.clone()
            } else {
                e.family_internal.clone()
            }
        })
        .filter(|s| !s.trim().is_empty());

    FontPlan {
        text_family,
        selected_raw: selected,
        emoji_family: emoji_font(root).map(|e| {
            if e.family_internal.trim().is_empty() {
                e.id
            } else {
                e.family_internal
            }
        }),
        fallback: fallback_chain(),
    }
}

/// **自带字体注册表**（唯一权威：清单、下载、界面三处都读它）。
///
/// 字段：`(id, 下载文件名, URL, sha256, 显示名, 内部族名提示, license, 许可 URL)`。
///
/// ⚠️ 三条规矩：
/// - **id 必须与落盘目录名一致**（落成 `<root>/<id>/font.ttf`），因为它就是配置/界面里那串 id。
/// - **sha256 必须对得上**：下载完当场校验，不符就删掉重来 —— 字体文件是**可执行内容的载体**
///   （解析器要读它），不能"下到盘上就信"。
/// - **内部族名只是"提示"**（写进 `FONT.toml` 的 `family`）；真值以落盘后 `family_of` 读到的为准，
///   上游改字重/子集时族名可能变，硬信提示会静默对不上（P1-e 那条"字体对不上"的老路）。
const BUILTIN_FONTS: &[BuiltinFont] = &[
    // ⚠️ **中文正文那支：2026-10-02 由霞鹜文楷 Lite 换成 Noto Sans SC**（用户口径：
    // "lxgw 有点不是很好看"）。选它的四条理由：
    //
    // ① **观感更中性**：霞鹜文楷是**手写楷体**风格，当正文字幕偏"文艺"；Noto Sans SC 是黑体，
    //    字幕要的正是中性、可读。
    // ② **体积更小**：10.05 MB（原 13.23 MB），exe 反而缩了 3 MB。
    // ③ **本就在设计意图里**：`fallback_chain()` 的第二/三项早就有 `"Noto Sans SC"`，
    //    而族名恰好就是 `Noto Sans SC`（实测 name 表 platform=3 enc=1）⇒ **栈不用改**，
    //    装进库后"库里那支"与"栈里那项"天然对上（这正是当年 LXGW 对不上的老坑）。
    // ④ **OFL-1.1**，与原来同规矩（`bundled/embedded/redistributed` 明确许可，见 §授权原文）；
    //    字节同样**内嵌**，不联网。
    //
    // ⚠️ **不是 CC0**：CJK 领域基本没有 CC0 正文字体（CC0 不含 OFL 的 RFN 保护、也不要求
    //    保留许可，中文项目普遍不愿用）。现实上限就是 OFL，别去找 CC0 了。
    //
    // ⚠️ 上游是 **Google Fonts 的 TTF 直链**（`fonts.gstatic.com`），不是 GitHub release：
    //    notofonts/noto-cjk 的 release 是**整套**（`09_NotoSerifSC.zip` 65 MB 起、含多字重），
    //    Google Fonts 给的是**单字重单文件**，这才是能内嵌的粒度。
    // ⚠️ URL 带版本段（`v40`）：上游改版会换路径 ⇒ 直链可能 404。**sha256 是最终防线**，
    //    校验不过就停下来（`install_builtin_font` 会拒），别把不确定的字节写进用户环境。
    BuiltinFont {
        id: "NotoSansSC-Regular",
        file: "NotoSansSC-Regular.ttf",
        url: "https://fonts.gstatic.com/s/notosanssc/v40/k3kCo84MPvpLmixcA63oeAL7Iqp5IZJF9bmaG9_FnYw.ttf",
        sha256: "450625c8d46ab3df97b7904ded955ec2746d17ec76740cb1e91d1ba63a0f89af",
        display_name: "Noto Sans SC",
        // ⚠️ 内部族名就是 "Noto Sans SC"（实测 name 表），**与 fallback_chain 里的字面量一致**
        // —— 这是它比 LXGW 省事的关键（LXGW 的族名是中文"霞鹜文楷 轻便版"，栈里写英文名，
        // 两者永远匹配不上，当年专门写了 `authority_tail` 去绕）。
        family: "Noto Sans SC",
        license: "OFL-1.1",
        license_url: "https://raw.githubusercontent.com/notofonts/noto-cjk/main/Sans/LICENSE",
        bytes: include_bytes!("../assets/fonts/NotoSansSC-Regular.ttf"),
        license_bytes: include_bytes!("../assets/fonts/NotoSansSC-Regular.OFL"),
    },
    // ⚠️ **emoji 字体必须是 CBDT、不能是 COLRv1**（2026-10-02，实测字节定的）。
    //
    // Takumi 的彩色字形解析**写死只问 COLRv0**（takumi-core-0.25.0/src/resources/glyph.rs:440-443
    // 的 ColorGlyphFormat::ColrV0），三级顺序是「位图 strikes → COLRv0 → 普通 glyf 描边」。
    // 上游 Noto Color Emoji v2.x 已迁到 COLRv1 —— 而 COLRv1 那张表 Takumi 不问、SVG-in-font 也没实现，
    // 于是对一支 COLRv1-only 字体它一路落空到 glyf。旧那版（google/fonts 的 main 分支）
    // 带 3.4 MB glyf ⇒ 封面上的 emoji 渲染成 **单色实心剪影**，不是彩色、也不是豆腐块，
    // **看着就有东西所以没人会去查**（最坏的失败形态）。
    //
    // 钉死到 googlefonts/noto-emoji 的 **v2020-09-16-unicode13_1**（COLRv1 迁移前的最后一版）：
    // 实测表结构 CBDT + CBLC、**无 COLR、无 sbix**，且**连 glyf 都没有** ⇒ 不存在退化成剪影这条路。
    // 内部族名仍是 Noto Color Emoji（与旧那支相同）⇒ 字体匹配口径不变。
    //
    // ⚠️ **id 必须换新**：下载三入口都幂等（builtin_installed 为真就跳过），
    // 复用 NotoColorEmoji 会让**已装过的机器永远不重下** —— 症状是我改了字体但用户那边没变。
    // 旧 id 遗留的目录（COLRv1 那份）仍留在库里，用户可在字体库界面自行删除。
    BuiltinFont {
        id: "NotoColorEmoji-CBDT",
        file: "NotoColorEmoji-CBDT.ttf",
        url: "https://github.com/googlefonts/noto-emoji/raw/v2020-09-16-unicode13_1/fonts/NotoColorEmoji.ttf",
        sha256: "2b106b88034043001f28b3ee8e58f62e867f0f0666ea072a4acf6cbd956ca160",
        display_name: "Noto Color Emoji",
        family: "Noto Color Emoji",
        license: "OFL-1.1",
        // ⚠️ 许可 URL 必须指到**字体**那层，不是仓库那层：noto-emoji 仓库根是 **Apache-2.0**，
        // 而 fonts/ 下才是 **SIL OFL-1.1**（我们用的就是它）。指错会让随附的 LICENSE.txt
        // 变成 Apache 原文、与清单里写的 OFL-1.1 自相矛盾。
        // 也**别用 main**：实测 main/fonts/LICENSE 早就 404 了（文件挪过位置），
        // 而 GitHub 对 404 回的是 **HTML 错误页** —— 下载器不校验内容，会把那一坨 HTML
        // 原样存成 LICENSE.txt（实测 268 KB 的 GitHub 页面）。所以钉到**我们那份字体的 tag**，
        // 与 url 同源、永远能拿到对应的许可原文。
        license_url: "https://raw.githubusercontent.com/googlefonts/noto-emoji/v2020-09-16-unicode13_1/fonts/LICENSE",
        bytes: include_bytes!("../assets/fonts/NotoColorEmoji-CBDT.ttf"),
        license_bytes: include_bytes!("../assets/fonts/NotoColorEmoji-CBDT.OFL"),
    },
];

/// 一支自带字体的清单项。
#[derive(Debug, Clone)]
pub struct BuiltinFont {
    pub id: &'static str,
    pub file: &'static str,
    /// 上游地址。**只作留档/溯源用**（内嵌之后运行时不再下载），见 `bytes`。
    pub url: &'static str,
    /// 字节的 sha256：`bytes`（内嵌）与旧下载路径**共用**这一份校验值。
    /// 不符 = 上游换了内容，**停下来**，别把不确定的字节装进用户环境。
    pub sha256: &'static str,
    pub display_name: &'static str,
    /// 期望的内部族名（写进 `FONT.toml`；真值以落盘后读到的为准）
    pub family: &'static str,
    pub license: &'static str,
    pub license_url: &'static str,
    /// **字体字节本身**（编译期内嵌，见 `assets/fonts/`）。
    ///
    /// ⚠️ 这是 2026-10-02 从"首次用时下载"改成"内嵌"的落点。两个理由：
    /// ① **离线可用** —— 下载方案在没网/内网/慢速环境下，封面与字幕会用不了库内字体，
    ///    而症状是"字体静默回落"，用户根本不知道少了什么；
    /// ② **省掉一个莫名其妙的操作** —— 用户不该先"下载字体"才能选字体。
    ///
    /// 授权依据：两支都是 **SIL OFL-1.1**，OFL 正文明确写了
    /// "The fonts ... can be bundled, embedded, redistributed and/or sold with any software"，
    /// 所以随二进制分发是许可允许的（`LICENSE` 原文随字节一起内嵌）。
    /// ⚠️ README 里"不含嵌入式用途"那句说的是**乐米波波体**（第三方字体），**不是这两支** ——
    /// 别看到那句话就把这里的字节又拿掉。
    pub bytes: &'static [u8],
    /// 许可原文（OFL 全文），随字体一起落盘到 `LICENSE.txt`。
    /// **OFL 要求随附**，所以它是内嵌内容的一部分，不是可选装饰。
    pub license_bytes: &'static [u8],
}

/// 自带字体清单（界面/CLI 列"可以下载什么"的唯一来源）。
pub fn builtin_fonts() -> &'static [BuiltinFont] {
    BUILTIN_FONTS
}

/// 这个 id 是不是自带的（**判据只看清单**，不看磁盘 —— 没下过也算自带）。
///
/// ⚠️ 界面把"自带"的删除键置灰要靠它，**不能**靠 `entry.builtin`：
/// 还没下载的自带字体在库里根本没有条目，那时 `builtin` 无从谈起。
pub fn is_builtin_id(id: &str) -> bool {
    BUILTIN_FONTS.iter().any(|b| b.id.eq_ignore_ascii_case(id.trim()))
}

/// 清单项 → 它在库里的目标目录（`<root>/<id>/`）。
pub fn builtin_dir(root: &Path, b: &BuiltinFont) -> PathBuf {
    root.join(b.id)
}

/// 这支自带字体**在库里装好了没有**（判据与 `scan_all` 同源：目录布局 + 能解析出文件）。
pub fn builtin_installed(root: &Path, b: &BuiltinFont) -> bool {
    font_file_in(&builtin_dir(root, b)).is_some()
}

/// 自带字体的安装情况（界面/CLI 用）。
pub fn builtin_status(root: &Path) -> Vec<(String, bool)> {
    BUILTIN_FONTS
        .iter()
        .map(|b| (b.id.to_string(), builtin_installed(root, b)))
        .collect()
}

/// **把还缺的自带字体装上**（已装好的跳过）。返回实际写入的 id 列表。
///
/// ⚠️ 幂等：`builtin_installed` 为真就**不碰磁盘**（用户可能已经手动替换过那支字体）。
/// ⚠️ "装"= 把**内嵌字节**写到 `<root>/<id>/`，**不再联网**（2026-10-02 从下载改成内嵌）。
pub fn install_builtin_fonts(root: &Path) -> anyhow::Result<Vec<String>> {
    let mut done = Vec::new();
    for b in BUILTIN_FONTS {
        if builtin_installed(root, b) {
            continue;
        }
        install_builtin_font(root, b)?;
        done.push(b.id.to_string());
    }
    Ok(done)
}

/// 装上**一支**自带字体（写 `<root>/<id>/{font.ttf,FONT.toml,LICENSE.txt}`）。
///
/// ⚠️ 幂等由**调用方**决定：这个函数"你叫我装我就装"（会覆盖已存在的）。
/// 想跳过已装好的用 [`builtin_installed`] 先判，或直接用 [`install_builtin_fonts`]。
pub fn install_builtin_font(root: &Path, b: &BuiltinFont) -> anyhow::Result<()> {
    use anyhow::Context as _;

    // ⚠️ **内嵌字节也要校验**（别以为"编译进去的就一定对"）：它是构建期从上游取的，
    // 而 `sha256` 是**钉在源码里**的那条独立事实 —— 两者对不上说明 `assets/fonts/` 里的
    // 文件被谁换过（或替换时忘了更新常量）。这里当场停住，绝不把没核过的字节装进用户环境。
    let got = sha256_hex(b.bytes);
    if !got.eq_ignore_ascii_case(b.sha256) {
        anyhow::bail!(
            "内嵌字体校验和不符（{}）：期望 {}，实际 {} —— assets/fonts/ 里那份与源码常量不一致",
            b.id, b.sha256, got
        );
    }

    let dest_dir = builtin_dir(root, b);
    std::fs::create_dir_all(&dest_dir)
        .with_context(|| format!("建字体目录失败: {}", dest_dir.display()))?;

    // 内部族名以**字节里读到的**为准（清单里的只是提示）
    let family = family_of(b.bytes).unwrap_or_else(|| b.family.to_string());
    let dest = dest_dir.join("font.ttf");
    std::fs::write(&dest, b.bytes)
        .with_context(|| format!("写字体失败: {}", dest.display()))?;

    write_meta(&dest_dir, &FontMeta {
        // id = 清单里那串（配置/界面里存的就是它）；目录名恰好也是它，但**别再依赖这个巧合**
        id: b.id.to_string(),
        display_name: b.display_name.to_string(),
        family,
        license: b.license.to_string(),
        source_url: b.url.to_string(),
        emoji: b.id.to_ascii_lowercase().contains("emoji"),
        builtin: true,
        // ⚠️ 这两个**必须写**（别留空）：`sha256` 是"盘上这支到底是不是清单里那支"的凭据
        // （用户手动替换过字体时，它就是唯一的证据），`added_at` 是"什么时候装进来的"。
        sha256: got,
        added_at: chrono::Utc::now().to_rfc3339(),
        ..Default::default()
    })
    .with_context(|| format!("写 FONT.toml 失败: {}", dest_dir.display()))?;

    // 许可原文：**OFL 要求随附**，所以与字体一起落盘。
    // ⚠️ 同样走内嵌字节：以前是 `fetch_text_to(license_url)`，而 GitHub 对 404 回的是
    // **HTML 错误页**、下载器又不校验内容 —— 实测把 268 KB 的 GitHub 页面存成了 `LICENSE.txt`。
    // 许可原文**不该依赖网络**（它必须与字节同在，否则这个包的授权状态说不清）。
    let lic = dest_dir.join("LICENSE.txt");
    std::fs::write(&lic, b.license_bytes)
        .with_context(|| format!("写许可原文失败: {}", lic.display()))?;

    Ok(())
}
/// **首次运行时把自带字体一次装好**（幂等；已经装过就是空转）。
///
/// 这是 2026-10-02 内嵌化的**落地点**：调用方（`ensure_dir` 之后）不再需要用户做任何操作，
/// 字体库一建出来就是"能选字"的状态。**失败只 warn、不返回 Err** —— 字体装不上不该拦住
/// `yekitrim server` 启动或一条 CLI 命令（没有它顶多是回落到系统字体栈，功能降级不是故障）。
///
/// ⚠️ **装在全局库**（`<数据>/global/resources/fonts/`，2026-10-10 用户拍板）：
/// 自带字体是"程序自带的**公用**素材"，不属于任何账号 —— 装一份所有人共用，
/// 而不是每个账号首次启动各落一份（约 20 MB × 人数）。
/// 各账号通过 `owner::asset_dirs()` 的"用户库优先、全局补缺"看到它。
pub fn ensure_builtin_fonts() {
    let root = crate::owner::global_resources_dir().join("fonts");
    if let Err(e) = std::fs::create_dir_all(&root) {
        tracing::warn!("全局字体库目录不可用，跳过自带字体安装：{e}");
        return;
    }
    match install_builtin_fonts(&root) {
        Ok(ids) if ids.is_empty() => {}
        Ok(ids) => tracing::info!("自带字体已就绪（本次安装 {} 支）：{}", ids.len(), ids.join(", ")),
        // 只 warn：装不上不影响启动，用户仍可用系统字体栈 + 手动导入
        Err(e) => tracing::warn!("自带字体安装失败（不影响启动）：{e}"),
    }
}

/// 十六进制 sha256（字体校验和用；只此一处实现）。
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut s = String::with_capacity(64);
    for byte in out {
        use std::fmt::Write as _;
        let _ = write!(s, "{byte:02x}");
    }
    s
}

/// 导入一支字体到库（目录布局 + 写 `FONT.toml`）—— 从**服务器上的文件路径**。
///
/// ⚠️ 实际校验与落盘全在 [`import_font_bytes`]；这里只负责读文件再转调。
/// 拖拽上传那条链路走的是 `import_font_bytes`（同一个库、同一套校验，只是字节来源不同）。
///
/// ⚠️ 拒收 `.ttc`/`.otc`（字体集合）：**浏览器的 `@font-face` 不认集合**，
/// 收进来就会出现"预览回退到系统字体、出片却用了它"——这正是"预览与实际对不上"的一个来源。
/// 要收集合得先拆包，留后续。
pub fn import_font(root: &Path, src: &Path, display_name: Option<&str>) -> anyhow::Result<FontEntry> {
    let file = src.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let bytes = std::fs::read(src)?;
    import_font_bytes(root, &file, &bytes, display_name)
}

/// 导入一支字体到库 —— **字节版**（2026-10-02 加，服务拖拽上传）。
///
/// ⚠️ 与 [`import_font`]（从服务器上的路径拷）是**同一条管线**：后者现在只是"读文件 + 转调这里"。
/// 别把校验/落盘逻辑写两份 —— 两份必然漂移，而这里漂移的后果是**坏字体进了库却没人拦**。
///
/// `file_name` 只当**文件名**用（它同时决定 id），所以要过 basename 与 `is_safe_id` 两道。
pub fn import_font_bytes(
    root: &Path,
    file_name: &str,
    bytes: &[u8],
    display_name: Option<&str>,
) -> anyhow::Result<FontEntry> {
    // ⚠️ 只取 basename：`file_name` 可能来自查询串/请求体，`..\\..\\x.ttf` 不能决定落盘位置
    let file = Path::new(file_name)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = Path::new(&file).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !matches!(ext.as_str(), "ttf" | "otf") {
        anyhow::bail!("只收 .ttf / .otf（.ttc / .otc 是字体集合，浏览器 @font-face 不认，请先拆包）");
    }
    if bytes.is_empty() {
        anyhow::bail!("文件是空的：{file}");
    }
    let family = family_of(bytes)
        .ok_or_else(|| anyhow::anyhow!("读不出字体内部族名（文件可能损坏或不是字体）"))?;
    let id = Path::new(&file).file_stem().and_then(|s| s.to_str()).unwrap_or("").trim().to_string();
    if !is_safe_id(&id) {
        anyhow::bail!("文件名不能当 id 用（含路径分隔符或控制字符）：{file}");
    }
    let display = display_name.map(|d| d.trim().to_string()).filter(|d| !d.is_empty())
        .unwrap_or_else(|| family.clone());
    // ⚠️ **落盘名是 ASCII slug，给人看的那串（id）住 `FONT.toml`**（见 [`ascii_slug`]）：
    // 底座那条腿把路径塞进 ffmpeg 的滤镜串，非 ASCII 路径会让它**每条字幕崩一次**。
    //
    // 已经存在同 id 的那一支 ⇒ **写回同一个目录**（不是再建一个）：
    // 老库的目录名恰好等于 id（自带字体、手工目录都是这样），而新 slug 与它不同 ——
    // 不复用就会出现“两条同 id 的条目”（`scan_all` 列两条、`resolve_all` 看目录顺序返回其一）。
    let dest_dir = match dir_for_id(root, &id) {
        Some(d) => d,
        None => root.join(ascii_slug(&id, &display)),
    };
    let old = read_meta(&dest_dir).unwrap_or_default();
    std::fs::create_dir_all(&dest_dir)?;
    // 字体文件名也必须是 ASCII：统一落 `font.<ext>`（老目录里那支可能叫中文名）。
    let font_file = format!("font.{ext}");
    // 换扩展名/换名字时把旧的那支收掉：`font_file_in` 只取一支，留两份会成谜
    if let Some(prev) = font_file_in(&dest_dir) {
        if prev.file_name().and_then(|f| f.to_str()) != Some(font_file.as_str()) {
            let _ = std::fs::remove_file(&prev);
        }
    }
    std::fs::write(dest_dir.join(&font_file), bytes)?;
    let meta = FontMeta {
        id: id.clone(),
        display_name: display,
        family,
        style: old.style,
        scripts: old.scripts,
        // ⚠️ 这几个是**这个槽位的角色**（自带 / 默认 / emoji / 许可），不是这支字节的属性：
        // 覆盖同一支字体时不该把它们悄悄清掉（清掉 = 自带变得可删、默认位跑掉、emoji 槽失效）。
        license: old.license,
        source_url: old.source_url,
        emoji: old.emoji,
        is_default: old.is_default,
        added_at: chrono::Utc::now().to_rfc3339(),
        // 导入时也记一份校验和：与自带字体那条路同口径，"盘上这支是什么"永远有据可查
        sha256: sha256_hex(bytes),
        // `builtin` **有意不继承**：用户导入进来的就是用户字体，删得掉（与改造前一致）
        ..Default::default()
    };
    write_meta(&dest_dir, &meta)?;
    Ok(FontEntry {
        id,
        file: font_file,
        family: meta.family.clone(),
        bytes: bytes.len() as u64,
        display_name: meta.display_name,
        family_internal: meta.family,
        // 刚写进来的，就在盘上
        installed: true,
        ..Default::default()
    })
}

/// 删除一支字体（整目录或老布局的单文件）。**自带字体拒删**。
pub fn remove_font(root: &Path, id: &str) -> anyhow::Result<()> {
    let id = id.trim();
    if !is_safe_id(id) {
        anyhow::bail!("非法 id");
    }
    // ⚠️ 目录名不再等于 id（ASCII slug / 老目录都可能），所以必须走 `dir_for_id`
    if let Some(dir) = dir_for_id(root, id) {
        // ⚠️ **与 `scan_dirs` 同一条判据**：元数据说自带 **且** id 还在清单里才拒删。
        // 只看 `meta.builtin` 会让**换下来的旧自带字体永远删不掉**（界面标着「程序自带」+
        // 这里拒删 = 用户既删不掉也看不懂为什么）—— 见 `scan_dirs` 里那段长注释。
        if read_meta(&dir).map(|m| m.builtin).unwrap_or(false) && is_builtin_id(id) {
            anyhow::bail!("程序自带字体不可删除");
        }
        std::fs::remove_dir_all(&dir)?;
        return Ok(());
    }
    match resolve_file_at(root, id) {
        Some(p) => { std::fs::remove_file(p)?; Ok(()) }
        None => anyhow::bail!("库里的这支字体不存在：{id}"),
    }
}

/// 设为默认字体（写进 `FONT.toml`；**目录布局专用** —— 老平铺文件没有元数据可写）。
///
/// ⚠️ **emoji 字体一律拒绝**（字体职责分离，2026-10-02）：emoji 是**独立的补充槽位**
/// （`emoji_font()` 自动选、至多一支），不参与"正文默认位"的竞争。
/// 放它进默认位的后果不是"emoji 当正文"那么简单 —— `preferred_fallback_family()` 会把它当
/// 栈首，于是**整条字体栈的中文全部落空**（emoji 字体没有中文字形）。
/// 界面（`FontLibrary.vue`）已经把「设为默认」按钮对 emoji 卡片隐藏了，这里是**第二道**
/// —— 配置是文本文件，用户能手改 `FONT.toml`。
pub fn set_default(root: &Path, id: &str) -> anyhow::Result<()> {
    let id = id.trim();
    if !is_safe_id(id) {
        anyhow::bail!("非法 id");
    }
    let Some(dir) = dir_for_id(root, id) else {
        anyhow::bail!("这支字体不在目录布局里（老布局没有元数据），先重新导入一次：{id}");
    };
    let mut meta = read_meta(&dir).unwrap_or_default();
    // ⚠️ 判据用**元数据 + id 兜底**：老库/手写的 `FONT.toml` 可能没写 `emoji` 字段，
    // 那时按 id 里含 "emoji" 判（与 `scan_all` 给未下载自带字体打标的口径同源）。
    if meta.emoji || id.to_ascii_lowercase().contains("emoji") {
        anyhow::bail!("emoji 字体自动补充，不参与默认（它只负责表情，不排正文）");
    }
    let mut entries = scan_dirs(root);
    for e in entries.iter_mut() {
        if e.id.eq_ignore_ascii_case(id) {
            meta.display_name = if e.display_name.is_empty() { e.id.clone() } else { e.display_name.clone() };
            meta.family = if e.family_internal.is_empty() { e.family.clone() } else { e.family_internal.clone() };
        }
    }
    // 先把其它支的 is_default 清掉（唯一默认）。⚠️ 目录名不再等于 id ⇒ 用 `dir_entries` 拿真实目录。
    for (slug, other, _file, mut m) in dir_entries(root) {
        if entry_id(&slug, &m).eq_ignore_ascii_case(id) {
            continue;
        }
        if m.is_default {
            m.is_default = false;
            let _ = write_meta(&other, &m);
        }
    }
    meta.is_default = true;
    write_meta(&dir, &meta)?;
    Ok(())
}

#[cfg(test)]
mod tests_v2 {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vt-fonts-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// ⚠️ **三处字体来源的收敛钉子（P1-e）**：同一个 id，
    /// `family_for` 必须给出**内部族名**（页面/预览/Takumi 三方唯一认的那个）；
    /// 而**库外**的值必须**原样返回** —— 老工程 `font = "LXGW WenKai"`（系统字体族名）
    /// 靠这条保持"什么都不注入、当系统族名用"的老行为。
    #[test]
    fn family_for_uses_internal_family_and_passes_through_unknown() {
        let root = tmp("family-for");
        let slug = root.join("my-font");
        std::fs::create_dir_all(&slug).unwrap();
        std::fs::write(slug.join("font.ttf"), b"not-a-real-font").unwrap();
        write_meta(&slug, &FontMeta {
            display_name: "我的字体".into(),
            family: "Real Internal Family".into(),
            ..Default::default()
        }).unwrap();
        // 库里认这个 id ⇒ 给内部族名（**不是 id、也不是 display_name**）
        assert_eq!(family_for_at(&root, "my-font"), "Real Internal Family");
        // 库外 ⇒ 原样返回（老工程系统族名的逃生口）
        assert_eq!(family_for_at(&root, "LXGW WenKai"), "LXGW WenKai");
        assert_eq!(family_for_at(&root, "   "), "");
        // 老平铺布局：family_internal 回落到解析名/id，仍然自洽
        std::fs::write(root.join("flat.ttf"), b"x").unwrap();
        let flat = family_for_at(&root, "flat");
        assert_eq!(flat, "flat", "老布局没有元数据时回落 id");
    }

    /// 目录布局：扫得到、解析得到、display_name 与内部族名**分开存**
    #[test]
    fn dir_layout_scan_and_resolve() {
        let root = tmp("dir");
        // ⚠️ 用**真清单里的 id**：`builtin` 现在是"元数据 + 清单"两件事（见 `stale_builtin_flag_no_longer_blocks_removal`），
        // 随手编一个 id 再写 `builtin: true` 已经不再等于自带 —— 那是**有意的**新语义。
        let slug = root.join(BUILTIN_FONTS[0].id);
        std::fs::create_dir_all(&slug).unwrap();
        std::fs::write(slug.join("font.ttf"), b"not-a-real-font").unwrap();
        write_meta(&slug, &FontMeta {
            id: BUILTIN_FONTS[0].id.into(),
            display_name: "我的字体".into(),
            family: "Real Internal Family".into(),
            builtin: true,
            ..Default::default()
        }).unwrap();
        let list = scan_all(&root);
        // ⚠️ `scan_all` 现在还会列出"没下载的自带字体"（`installed: false`），
        // 所以判据要**挑出自己造的那一支**，不能拿 `list.len() == 1` 当"目录布局正常"的证据。
        let mine: Vec<_> = list.iter().filter(|e| e.installed).collect();
        assert_eq!(mine.len(), 1, "{list:?}");
        assert_eq!(mine[0].id, BUILTIN_FONTS[0].id);
        assert_eq!(mine[0].display_name, "我的字体");
        assert_eq!(mine[0].family_internal, "Real Internal Family");
        assert!(mine[0].builtin, "清单里的 id + 元数据 builtin=true ⇒ 自带");
        assert!(resolve_all(&root, BUILTIN_FONTS[0].id).is_some());
    }

    /// 老布局（平铺文件）继续能扫到、能解析
    #[test]
    fn legacy_flat_layout_still_works() {
        let root = tmp("flat");
        std::fs::write(root.join("OldFont.ttf"), b"x").unwrap();
        let list = scan_all(&root);
        // 同上：只看"真在盘上"的那些（自带字体未安装的占位不算）
        let mine: Vec<_> = list.iter().filter(|e| e.installed).collect();
        assert_eq!(mine.len(), 1, "{list:?}");
        assert_eq!(mine[0].id, "OldFont");
        assert_eq!(mine[0].file, "OldFont.ttf");
        assert!(resolve_all(&root, "OldFont").is_some());
    }

    /// 导入：拒收 .ttc（字体集合）与其它扩展名；接受真字体并落 `FONT.toml`
    #[test]
    fn import_rejects_collections_and_writes_meta() {
        let root = tmp("import");
        let coll = root.join("coll.ttc");
        std::fs::write(&coll, b"ttcf").unwrap();
        let err = import_font(&root, &coll, None).unwrap_err().to_string();
        assert!(err.contains(".ttc"), "{err}");
        let bad = root.join("nope.woff2");
        std::fs::write(&bad, b"x").unwrap();
        assert!(import_font(&root, &bad, None).is_err());

        // 造一个最小的真 ttf：>=12 字节且表数为 0 ⇒ family_of 返回 None ⇒ 应当被拒（读不出族名）
        let fake = root.join("Fake.ttf");
        std::fs::write(&fake, [0u8; 16]).unwrap();
        let err = import_font(&root, &fake, None).unwrap_err().to_string();
        assert!(err.contains("族名"), "{err}");
    }

    /// 换下来的**旧自带字体**必须能删：「自带」判据是**元数据 + 当前清单**两件事。
    ///
    /// 回归的是实测踩到的形态：自带中文那支由霞鹜文楷 Lite 换成 Noto Sans SC 之后，
    /// 用户在旧版下载到盘上的那份 `FONT.toml` 仍写着 `builtin = true`，于是界面标它
    /// 「程序自带」、`remove_font` 又拒删 —— **13.23 MB 卡在库里，用户删不掉也看不懂为什么**。
    #[test]
    fn stale_builtin_flag_no_longer_blocks_removal() {
        let root = tmp("stale-builtin");
        let slug = root.join("gone-font");
        std::fs::create_dir_all(&slug).unwrap();
        std::fs::write(slug.join("font.ttf"), b"x").unwrap();
        // 元数据说它是自带的，但 id 不在清单里（= 曾经自带、现在被换下来了）
        write_meta(&slug, &FontMeta { id: "gone-font".into(), builtin: true, ..Default::default() }).unwrap();
        assert!(slug.exists());

        // ① 列表侧：不能再标「程序自带」（否则界面把删除键置灰）
        let e = scan_dirs(&root).into_iter().find(|e| e.id == "gone-font").expect("应当扫得到");
        assert!(!e.builtin, "脱离清单的旧自带字体不该再标 builtin");

        // ② 删除侧：必须放行
        assert!(remove_font(&root, "gone-font").is_ok(), "脱离清单的旧自带字体应当可删");
        assert!(!slug.exists());
    }

    /// 自带字体不可删除（判据 = 元数据的 `builtin` **且** id 在清单里）
    #[test]
    fn builtin_cannot_be_removed() {
        let root = tmp("builtin");
        // 同上：id 必须是**清单里的**，`builtin: true` 单独出现已不再构成"自带"
        let id = BUILTIN_FONTS[0].id;
        let slug = root.join(id);
        std::fs::create_dir_all(&slug).unwrap();
        std::fs::write(slug.join("font.ttf"), b"x").unwrap();
        write_meta(&slug, &FontMeta { id: id.into(), builtin: true, ..Default::default() }).unwrap();
        let err = remove_font(&root, id).unwrap_err().to_string();
        assert!(err.contains("程序自带字体不可删除"), "{err}");
        assert!(root.join(id).exists());
        // 非自带的能删
        let other = root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("font.ttf"), b"x").unwrap();
        assert!(remove_font(&root, "other").is_ok());
    }

    /// ⚠️ **本次事故的钉子（2026-10-02）**：id 含汉字时，**落盘路径必须纯 ASCII**。
    ///
    /// 底座出片那条腿把字体路径塞进 ffmpeg 的 `drawtext`，非 ASCII 路径会让 ffmpeg
    /// **每条字幕访问违例一次**（`0xC0000005`，240 帧 4.2s → 80.3s，且一个字都画不出来）。
    /// 这里钉三件事：① `ascii_slug` 纯函数（稳定/纯 ASCII/不撞）② 导入落盘形状
    /// ③ 同 id 重复导入**不会**多出一条（否则 `scan_all` 列两条、`resolve_all` 看目录顺序返回其一）。
    #[test]
    fn cjk_id_never_reaches_disk_as_a_path() {
        // ① 纯函数：同一 id 稳定、纯 ASCII、不同 id 不撞、ASCII 名保持可读
        let a = ascii_slug("乐米波波体", "乐米波波体");
        assert_eq!(a, ascii_slug("乐米波波体", "乐米波波体"), "同一 id 的 slug 必须稳定");
        assert!(a.is_ascii(), "{a}");
        assert!(a.starts_with("font-"), "全中文 hint 退化成 font- 前缀：{a}");
        assert_ne!(ascii_slug("NotoSansSC-Regular", "Noto Sans SC"), a);
        assert!(ascii_slug("Noto", "Noto Sans SC").starts_with("noto-sans-sc-"));

        // ② 导入：目录 ASCII、文件名 font.ttf、id 仍在元数据里、解析/扫描都不退化成 slug
        let root = tmp("cjk-id");
        let bytes = builtin_fonts()[0].bytes;
        let e = import_font_bytes(&root, "乐米波波体.ttf", bytes, Some("乐米波波体")).unwrap();
        assert_eq!(e.id, "乐米波波体", "配置里那串不变");
        assert!(e.file.is_ascii() && e.file == "font.ttf", "{}", e.file);
        let dir = dir_for_id(&root, "乐米波波体").expect("按 id 要能找到目录");
        let dir_name = dir.file_name().unwrap().to_str().unwrap().to_string();
        assert!(dir_name.is_ascii(), "目录名必须 ASCII：{dir_name}");
        assert!(dir.join("font.ttf").is_file());
        assert!(resolve_all(&root, "乐米波波体").is_some());
        let listed = scan_all(&root).into_iter().find(|x| x.id == "乐米波波体").expect("列表里要有它");
        assert_eq!(listed.display_name, "乐米波波体", "显示名不能变成 slug");
        assert!(listed.installed);

        // ③ 同 id 再导入一次：写回同一个目录，不新增条目
        let before = dir_entries(&root).len();
        import_font_bytes(&root, "乐米波波体.ttf", bytes, Some("乐米波波体")).unwrap();
        assert_eq!(dir_entries(&root).len(), before, "同 id 重复导入不该多出一条");
    }

    /// 手工探针：对**任意目录**跑一次 ASCII 迁移并列出结果（默认不跑）。
    ///
    /// ```bash
    /// VT_FONT_MIGRATE_DIR=<库目录> cargo test -p yekitrim-core --lib migrate_probe -- --ignored --nocapture
    /// ```
    ///
    /// 为什么不干脆给个 CLI：迁移是 `ensure_dir` 的副作用，正常用户不需要手动触发；
    /// 而**验证**它只需要一个可注入目录的入口（与 `diagnose_sticker_cache` 同规矩）。
    #[test]
    #[ignore]
    fn migrate_probe() {
        let Ok(dir) = std::env::var("VT_FONT_MIGRATE_DIR") else {
            eprintln!("设 VT_FONT_MIGRATE_DIR=<库目录> 再跑（--ignored）");
            return;
        };
        let root = PathBuf::from(dir);
        let moved = migrate_ascii_layouts(&root);
        println!("迁移 {} 条：{moved:?}", moved.len());
        for e in scan_all(&root).iter().filter(|e| e.installed) {
            let p = resolve_all(&root, &e.id).map(|p| p.display().to_string()).unwrap_or_default();
            println!("  id={:?} file={:?} flat={} path={}", e.id, e.file, e.flat, p);
        }
    }

    /// 老库迁移：`<中文目录>/<中文名>.ttf`（可以连 `FONT.toml` 都没有）
    /// ⇒ 目录 ASCII + `font.ttf` + `id` 回落旧目录名；**幂等**，且 ASCII 条目一个都不动。
    #[test]
    fn legacy_non_ascii_dir_migrates_to_ascii_and_keeps_id() {
        let root = tmp("migrate");
        let old = root.join("乐米波波体");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("乐米波波体.ttf"), b"not-a-real-font").unwrap();
        // 迁移前：id = 目录名，解析照旧（老行为一字不变 —— 迁移失败也不能让存量失联）
        assert!(resolve_all(&root, "乐米波波体").is_some());

        let moved = migrate_ascii_layouts(&root);
        assert_eq!(moved.len(), 1, "{moved:?}");
        assert!(!old.exists(), "中文目录应当搬走");
        let new_dir = dir_for_id(&root, "乐米波波体").expect("迁移后仍要按老 id 找得到");
        let new_name = new_dir.file_name().unwrap().to_str().unwrap().to_string();
        assert!(new_name.is_ascii(), "{new_name}");
        assert!(new_dir.join("font.ttf").is_file(), "文件也要 ASCII 名");
        assert!(read_meta(&new_dir).unwrap().id == "乐米波波体", "旧目录名要落成 id");
        assert!(scan_all(&root).iter().any(|e| e.id == "乐米波波体" && e.installed));
        // 目录里不该再有任何非 ASCII 名字
        for f in std::fs::read_dir(&new_dir).unwrap().flatten() {
            assert!(f.file_name().to_string_lossy().is_ascii(), "{:?}", f.file_name());
        }
        // 幂等 + ASCII 条目不动
        assert!(migrate_ascii_layouts(&root).is_empty(), "迁移必须幂等");
        let keep = root.join("NotoSansSC-Regular");
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::write(keep.join("font.ttf"), b"x").unwrap();
        write_meta(&keep, &FontMeta {
            id: "NotoSansSC-Regular".into(),
            display_name: "Noto Sans SC".into(),
            ..Default::default()
        }).unwrap();
        assert!(migrate_ascii_layouts(&root).is_empty(), "ASCII 条目不该被碰");
        assert!(keep.is_dir(), "ASCII 目录不该被改名");
    }

    /// ⚠️ **P1-f 的钉子**：自带字体的清单形状 + "没下载也要看得见" + "下载过就不重复装"。
    ///
    /// 三件事各自对应一次真实风险：
    /// ① 清单里的 id 与落盘目录名**必须一致**（id 就是配置/界面里那串），且 sha256 长度像话
    ///    —— 抄错一位就是"下载永远校验不过"，而它会表现成"下载失败"，很难往清单上想；
    /// ② **没下载 ≠ 看不见**：\`scan_all\` 要把它列出来（\`installed: false\`），否则界面无从提示用户去下；
    /// ③ 已装好的**不能重复下载**（用户可能手动替换过那支字体，重下会覆盖掉他的选择）。
    #[test]
    fn builtin_registry_shape_and_not_installed_visibility() {
        assert!(!builtin_fonts().is_empty(), "自带字体清单不该是空的");
        for b in builtin_fonts() {
            assert!(is_safe_id(b.id), "id 必须能当目录名: {}", b.id);
            assert!(!b.url.is_empty() && b.url.starts_with("https://"), "{} 的 URL", b.id);
            assert_eq!(b.sha256.len(), 64, "{} 的 sha256 必须是 64 位十六进制", b.id);
            assert!(
                b.sha256.chars().all(|c| c.is_ascii_hexdigit()),
                "{} 的 sha256 含非十六进制字符",
                b.id
            );
            assert!(!b.display_name.trim().is_empty(), "{} 要有给人看的显示名", b.id);
            assert!(!b.license.trim().is_empty(), "{} 要有许可标识", b.id);
        }

        // 空目录：自带字体应当**全部**出现在列表里，且都是 installed=false
        let root = tmp("builtin-list");
        let list = scan_all(&root);
        assert_eq!(
            list.len(),
            builtin_fonts().len(),
            "空库里应当列出全部自带字体（未安装也要看得见）"
        );
        for e in &list {
            assert!(is_builtin_id(&e.id), "{} 应当是自带 id", e.id);
            assert!(e.builtin, "{} 应当标成自带", e.id);
            assert!(!e.installed, "{} 还没下载，installed 必须是 false", e.id);
            assert!(e.bytes == 0, "{} 没下载就没有体积", e.id);
            // 未安装的没有文件 ⇒ 解析不到 ⇒ "零字体"那条报错路径仍然成立（不会假装有字体）
            assert!(resolve_all(&root, &e.id).is_none(), "{} 未安装不该解析出文件", e.id);
            // 但**族名是知道的**（清单里写着），CSS 那边问得出来
            assert!(!family_for_at(&root, &e.id).trim().is_empty(), "{} 应当有族名", e.id);
        }
        assert_eq!(builtin_status(&root).len(), builtin_fonts().len());
        assert!(
            builtin_status(&root).iter().all(|(_, ok)| !ok),
            "空库里没有一支装好"
        );
    }

    /// 装好一支自带字体后：\`installed\` 变真、\`builtin_installed\` 变真、幂等的下载**跳过**它。
    ///
    /// ⚠️ 这里**不真联网**：直接按 \`install_builtin_font\` 的落盘形状（\`<id>/font.ttf\` + 元数据）
    /// 手工摆一支，然后只验"判定与幂等"。真下载（含 sha256）在联网验收里跑一次。
    #[test]
    fn installed_builtin_is_detected_and_not_redownloaded() {
        let b = &builtin_fonts()[0];
        let root = tmp("builtin-installed");
        let d = builtin_dir(&root, b);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("font.ttf"), b"x").unwrap();
        write_meta(&d, &FontMeta {
            display_name: b.display_name.into(),
            family: b.family.into(),
            builtin: true,
            ..Default::default()
        })
        .unwrap();

        assert!(builtin_installed(&root, b), "摆好了就该判为已安装");
        assert!(is_builtin_id(&format!("  {}  ", b.id)), "判定应当容忍空白");
        let e = scan_all(&root).into_iter().find(|x| x.id == b.id).expect("列表里应当有它");
        assert!(e.installed, "已安装 ⇒ installed=true");
        assert!(e.builtin, "自带 ⇒ builtin=true（界面据此置灰删除）");
        assert!(resolve_all(&root, b.id).is_some(), "已安装要能解析出文件");

        // 幂等：已装好的那支不在"待下载"里（真下载由 download_builtin_fonts 跳过它）
        let pending = builtin_status(&root);
        assert!(
            pending.iter().any(|(id, ok)| id == b.id && *ok),
            "状态列表里该支应为已安装"
        );
    }

    /// 设为默认：唯一默认 + 未知 id 报错
    #[test]
    fn set_default_is_exclusive() {
        let root = tmp("default");
        for id in ["a", "b"] {
            let d = root.join(id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("font.ttf"), b"x").unwrap();
            write_meta(&d, &FontMeta { display_name: id.into(), ..Default::default() }).unwrap();
        }
        set_default(&root, "a").unwrap();
        set_default(&root, "b").unwrap();
        assert!(!read_meta(&root.join("a")).unwrap().is_default);
        assert!(read_meta(&root.join("b")).unwrap().is_default);
        assert!(set_default(&root, "nope").is_err());
    }

    // ===== 「库里默认那支」的唯一查询入口 =====
    //
    // ⚠️ 这一组钉的是 `cmd_init` 的行为来源（2026-10-06 用户报「每次切出切片，字体都不是
    //    字体库设置的默认字体」）：新工程的字幕字体必须来自这里，而不是写死的那个常量。

    /// 没设过默认 ⇒ `None`（调用方保持出厂值，别自己编一个）。
    #[test]
    fn default_text_family_none_without_flag() {
        let root = tmp("deftext-none");
        put_font(&root, "WenKai", "霞鹜文楷 轻便版", false, false);
        assert_eq!(default_text_family(&root), None, "没人设默认位就该是 None");
    }

    /// 设了默认 ⇒ 返回**内部族名**（不是 id）—— 与 `fallback_chain()` 首项同一种东西。
    #[test]
    fn default_text_family_returns_internal_family() {
        let root = tmp("deftext-hit");
        put_font(&root, "WenKai", "霞鹜文楷 轻便版", false, true);
        put_font(&root, "Other", "Other Family", false, false);
        assert_eq!(
            default_text_family(&root).as_deref(),
            Some("霞鹜文楷 轻便版"),
            "要族名不要 id（写进 polish.toml 的 font 那栏要能与栈首对上）"
        );
    }

    /// ⚠️ emoji 字体**永不出现在这里**，哪怕手改 `FONT.toml` 把两个标志都立起来。
    ///
    /// 这是"字体职责分离"落到本函数上的那道：拿一支没有中文字形的字体去排正文 = 整条字幕豆腐块。
    #[test]
    fn default_text_family_never_returns_emoji() {
        let root = tmp("deftext-emoji");
        put_font(&root, "Emoji", "Noto Color Emoji", true, true);
        assert_eq!(
            default_text_family(&root),
            None,
            "emoji 字体即使手改成立默认位，也不能被选成正文默认"
        );
    }

    /// 未安装（清单里有、盘上还没有）的自带字体也不能被选成新工程默认。
    #[test]
    fn default_text_family_skips_uninstalled() {
        let root = tmp("deftext-uninstalled");
        std::fs::create_dir_all(&root).unwrap();
        // 自带那支"没下载"时 `scan_all` 会给一条 installed=false 的占位；
        // 这里直接断言：任何 installed=false 的条目都不该被选中。
        let picked = default_text_family(&root);
        assert_eq!(picked, None, "空库不该选出一支盘上不存在的文件");
    }

    /// 设过默认的那支如果**内部族名为空**，回落用 id（界面上显示的就是 id，别给空串）。
    #[test]
    fn default_text_family_falls_back_to_id_when_family_blank() {
        let root = tmp("deftext-blank");
        put_font(&root, "NoFamily", "", false, true);
        assert_eq!(default_text_family(&root).as_deref(), Some("NoFamily"));
    }

    // ===== 字体解析唯一入口（FontPlan）的钉子 =====
    //
    // ⚠️ 这一组是"字体职责分离"的核心判据（2026-10-02）。它们**不渲染**（纯函数、毫秒级），
    // 所以能穷举分支；而"出没出彩色 emoji"那种要靠真渲染的验收在 C2/C3 那两步单独跑。
    //
    // 这一组的价值在于：往后每接一个消费点（预览 / 封面 / 成片），只要它经 `resolve_font_plan`
    // 取字体，它的行为就被这几条钉住了 —— **不用每接一处就重新对一遍像素**。

    /// 造一支"已装"的字体目录（带元数据）。
    fn put_font(root: &Path, id: &str, family: &str, emoji: bool, is_default: bool) {
        let d = root.join(id);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("font.ttf"), b"x").unwrap();
        write_meta(
            &d,
            &FontMeta {
                display_name: id.into(),
                family: family.into(),
                emoji,
                is_default,
                ..Default::default()
            },
        )
        .unwrap();
    }

    /// ⚠️ **零回归的钉子**：空库时兜底链必须与老的 `SYSTEM_FONT_STACK` **逐字同族名同顺序**。
    ///
    /// 这是"存量工程零像素回归"的判据 —— 与 `render/server.rs::stack_head_tracks_installed_library_font`
    /// 那条**互相印证**（那条钉的是 polish 侧拼出来的串，这条钉的是 core 侧那份数据）。
    /// 改这条断言之前先想清楚：它会同时改掉所有存量工程的出片像素。
    #[test]
    fn empty_library_fallback_chain_matches_legacy_stack() {
        let root = tmp("plan-empty");
        std::fs::create_dir_all(&root).unwrap();
        let plan = resolve_font_plan(&root, None);
        assert_eq!(
            plan.fallback,
            vec!["Noto Sans SC", "Microsoft YaHei", "Noto Sans SC", "sans-serif"],
            // ⚠️ 2026-10-02：首项由 LXGW WenKai 换成 Noto Sans SC（自带中文换了那支）。
            // 这条**会改变存量工程的出片像素**（首项 = "没选字体时用哪支"）—— 但那正是本次换字体的
            // 目的，属**有意变更**、不是回归。与 render/server.rs 的 SYSTEM_FONT_STACK 同步。
            "空库兜底链必须与 SYSTEM_FONT_STACK 逐字一致（两份定义不许漂）"
        );
        assert!(plan.emoji_family.is_none(), "空库没有 emoji 槽位");
        assert!(plan.text_family.is_none(), "没选 ⇒ 没有正文族名");
        // id 不在库里 ⇒ 走兜底，且原文保留（老工程的 `font = "LXGW WenKai"` 行为不变）
        let plan2 = resolve_font_plan(&root, Some("LXGW WenKai"));
        assert!(plan2.text_family.is_none(), "库外 id 不产生正文族名");
        assert_eq!(plan2.selected_raw, "LXGW WenKai", "原文保留（当系统族名用）");
    }

    /// **emoji 槽位**：库里装了 emoji 字体 ⇒ 独立字段有值，**且不影响正文族名**。
    ///
    /// 这是"职责分离"最直接的判据：emoji 走自己的槽位，正文还是正文。
    #[test]
    fn emoji_slot_is_separate_from_text_family() {
        let root = tmp("plan-emoji");
        std::fs::create_dir_all(&root).unwrap();
        put_font(&root, "WenKai", "霞鹜文楷 轻便版", false, true);
        put_font(&root, "Emoji", "Noto Color Emoji", true, false);

        let plan = resolve_font_plan(&root, Some("WenKai"));
        assert_eq!(plan.text_family.as_deref(), Some("霞鹜文楷 轻便版"), "正文 = 用户选那支");
        assert_eq!(plan.emoji_family.as_deref(), Some("Noto Color Emoji"), "emoji 走独立槽位");
        assert!(
            !plan.fallback.iter().any(|f| f.contains("Emoji")),
            "⚠️ emoji **不许**混进兜底链 —— 混进去就会撑破 polish 侧那条'四段'单测，             而且对底座(Dhampir)那条腿零效果（它没有逐字形回退）"
        );
    }

    /// **只有 emoji、没有中文字体** ⇒ 正文必须为空（**不能**把 emoji 当正文）。
    ///
    /// 这条防的是"库里只有一支 emoji 字体，于是它被当成正文"——
    /// 那会让**所有中文变成豆腐块**，而且不报错。
    #[test]
    fn emoji_only_library_never_becomes_text_font() {
        let root = tmp("plan-emoji-only");
        std::fs::create_dir_all(&root).unwrap();
        put_font(&root, "Emoji", "Noto Color Emoji", true, false);

        let plan = resolve_font_plan(&root, Some("Emoji"));
        assert!(plan.text_family.is_none(), "⚠️ emoji 字体不能当正文");
        // 即使用户**显式**选了它（手改配置），也不给正文族名
        assert_eq!(plan.emoji_family.as_deref(), Some("Noto Color Emoji"), "但仍然进 emoji 槽位");
    }

    /// ⚠️ **手改 FONT.toml 把 emoji 设成 default** ⇒ 扫描时强制视为 false。
    ///
    /// 这是 A4 最关键的一处：`is_default` 有两个真消费者 ——
    /// `preferred_fallback_family`（选字体栈**首项**）与 `cover.rs::cover_font_files_in`（选**注册序第一支**）。
    /// 让 emoji 拿到默认位 ⇒ 整条栈的中文全部落空。所以**在数据出口这一层**就掐掉。
    #[test]
    fn emoji_font_cannot_hold_the_default_slot() {
        let root = tmp("plan-emoji-default");
        std::fs::create_dir_all(&root).unwrap();
        put_font(&root, "Emoji", "Noto Color Emoji", true, true); // 手改成 default
        put_font(&root, "WenKai", "霞鹜文楷 轻便版", false, false);

        let scanned = scan_all(&root);
        let emoji = scanned.iter().find(|e| e.id == "Emoji").unwrap();
        assert!(emoji.emoji, "它确实是 emoji 字体");
        assert!(!emoji.is_default, "⚠️ emoji 的 is_default 必须被强制为 false");
        // 正文族名不受污染（它本来就不该被 emoji 影响）
        let plan = resolve_font_plan(&root, None);
        assert!(plan.text_family.is_none(), "没选 ⇒ 走兜底，不该冒出一个 emoji 正文族名");
    }

    /// **`import_font_bytes` 与 `import_font` 走的是同一条管线**（2026-10-02 加拖拽上传时）。
    ///
    /// 这条守的是「两份实现漂移」：拖拽走字节、从服务器导入走路径，若各写一套校验，
    /// 迟早出现「拖进来的坏字体没被拦」。判据 = 同样非法/合法的输入，两条路的**结论相同**。
    #[test]
    fn import_font_bytes_shares_the_same_rules_as_import_font() {
        // 用本模块既有的 tmp 助手（别再引 tempfile 依赖）
        let owned = tmp("import-bytes");
        let root = owned.as_path();
        let src = root.join("gone.ttf");

        // ① 拒收：非 ttf/otf，两条路都要拒（且理由一样）
        let a = import_font(root, &src.with_extension("ttc"), None);
        let b = import_font_bytes(root, "x.ttc", b"whatever", None);
        assert!(a.is_err() && b.is_err(), "两条路都必须拒收 .ttc");

        // ② 拒收：空文件（字节路独有 —— 路径路读一个空文件同样会栽在族名那步）
        assert!(import_font_bytes(root, "empty.ttf", b"", None).is_err(), "空文件不能进库");

        // ③ 拒收：读不出族名的垃圾字节（两条路同一个判据）
        assert!(import_font_bytes(root, "junk.ttf", b"not a font at all", None).is_err());

        // ④ `..\..\x.ttf` 不能决定落盘位置：文件名的目录部分必须被剥掉
        let evil = import_font_bytes(root, "..\\..\\evil.ttf", b"not a font", None);
        assert!(evil.is_err(), "越界路径不该成功");
        assert!(!root.join("..").join("..").join("evil.ttf").exists());

        // ⑤ 合法输入：真字体（拿内嵌那支）两条路都该成功，且落同一个布局
        let bytes = &builtin_fonts()[0].bytes;
        let e = import_font_bytes(root, "Dropped.ttf", bytes, Some("拖进来的")).unwrap();
        assert_eq!(e.id, "Dropped");
        // ⚠️ 2026-10-02 起落盘形状是「ASCII slug 目录 + `font.<ext>`」，id 只住元数据里
        // （见 `ascii_slug`：ffmpeg 那条 drawtext 打不开含非 ASCII 的路径）。
        let dir = dir_for_id(root, "Dropped").expect("按 id 要能找回目录");
        assert_eq!(dir, root.join(ascii_slug("Dropped", "拖进来的")));
        assert!(!dir.join("Dropped.ttf").exists(), "老形状（原名）不该再出现");
        assert!(dir.join("font.ttf").is_file());
        // 元数据也照写（id / 显示名 / 校验和 / 族名都该在）
        let meta = read_meta(&dir).unwrap();
        assert_eq!(meta.id, "Dropped");
        assert_eq!(meta.display_name, "拖进来的");
        assert_eq!(meta.sha256, sha256_hex(bytes), "校验和必须与字节一致");
        assert!(!meta.family.is_empty(), "族名必须读出来");
    }

    /// **自带那支 emoji 必须是渲染器真能出彩色的格式**（2026-10-02）。
    ///
    /// 这条守的是一个**静默**故障：如果哪天把资产换成 COLRv1 的 Noto Color Emoji
    /// （上游 `main` 分支就是 COLRv1），封面的 emoji 会**整片消失** —— 不报错、
    /// 界面还显示"已装"。实测控制实验：只装 COLRv1 时输出与"完全没装 emoji 字体"**逐字节相同**。
    #[test]
    fn builtin_emoji_is_a_renderable_color_format() {
        for b in builtin_fonts() {
            if !b.id.to_ascii_lowercase().contains("emoji") {
                continue;
            }
            let s = color_glyph_support(b.bytes);
            assert!(
                color_supported(s),
                "{} 的格式是 {}，渲染器出不了彩色（emoji 会整片消失）—— 换回 CBDT/COLRv0/sbix 那份",
                b.id,
                color_format_tag(s)
            );
            // 顺带钉住"就是 CBDT"：当前资产是刻意选的 CBDT 版，换格式要有人知道
            assert_eq!(color_format_tag(s), "cbdt", "{} 的格式变了", b.id);
        }
    }

    /// 格式判定要能真的把 COLRv1 与 SVG 挑出来（否则警告文案永远不触发）。
    ///
    /// 用**手搓的最小 sfnt**测：只要能摆出表目录，判定就该认。
    #[test]
    fn color_format_detection_distinguishes_the_dangerous_ones() {
        fn fake_sfnt(tags: &[&[u8; 4]], colr_version: Option<u16>) -> Vec<u8> {
            let n = tags.len() as u16;
            // ⚠️ 目录区必须先撑出来（12 字节头 + 16×n 条记录）：只给 12 字节再写目录会越界
            let dir_end = 12 + 16 * tags.len();
            let mut out = vec![0u8; dir_end];
            out[0..4].copy_from_slice(b"\x00\x01\x00\x00");
            out[4..6].copy_from_slice(&n.to_be_bytes());
            for (i, tag) in tags.iter().enumerate() {
                let off = 12 + 16 * i;
                out[off..off + 4].copy_from_slice(&tag[..]);
                // 表偏移指向目录之后；COLR 的表体就是 version(2B)
                // 表体统一放在目录区之后。COLR 的表体就是 version(2B)。
                // ⚠️ 偏移必须用**追加前的实际长度**：Rust 的 extend 落在 `out.len()` 上，
                // 不是 `dir_end + 2` —— 差 2 就会读到全 0（v0），COLRv1 的断言因此永不触发。
                let table_off = if &tag[..] == b"COLR" {
                    let o = out.len() as u32;
                    out.extend_from_slice(&colr_version.unwrap_or(0).to_be_bytes());
                    o
                } else {
                    dir_end as u32
                };
                out[off + 8..off + 12].copy_from_slice(&table_off.to_be_bytes());
            }
            out
        }
        assert!(matches!(
            color_glyph_support(&fake_sfnt(&[b"CBDT", b"CBLC"], None)),
            ColorGlyphSupport::Color(ColorGlyphFormatKind::Cbdt)
        ));
        assert_eq!(color_format_tag(color_glyph_support(&fake_sfnt(&[b"CBDT", b"CBLC"], None))), "cbdt");
        assert_eq!(color_glyph_support(&fake_sfnt(&[b"sbix"], None)), ColorGlyphSupport::Color(ColorGlyphFormatKind::Sbix));
        // COLR v0 可用；v1 **必须**被判成不可用（这是 Takumi 的真实行为）
        assert_eq!(
            color_glyph_support(&fake_sfnt(&[b"COLR", b"CPAL"], Some(0))),
            ColorGlyphSupport::Color(ColorGlyphFormatKind::ColrV0)
        );
        assert_eq!(
            color_glyph_support(&fake_sfnt(&[b"COLR", b"CPAL"], Some(1))),
            ColorGlyphSupport::Unsupported(ColorGlyphFormatKind::ColrV1)
        );
        assert_eq!(color_format_tag(color_glyph_support(&fake_sfnt(&[b"COLR"], Some(1)))), "colrv1");
        assert_eq!(color_glyph_support(&fake_sfnt(&[b"SVG "], None)), ColorGlyphSupport::Unsupported(ColorGlyphFormatKind::Svg));
        // 纯文字字体：有 glyf、无彩色表 ⇒ none（**正常**，不是缺陷）
        assert_eq!(color_glyph_support(&fake_sfnt(&[b"glyf"], None)), ColorGlyphSupport::None);
        // 垃圾字节不许 panic
        assert_eq!(color_glyph_support(b"not a font"), ColorGlyphSupport::None);
        assert_eq!(color_glyph_support(&[]), ColorGlyphSupport::None);
    }

    /// **内嵌字体的字节必须与源码里钉的 sha256 对得上**（2026-10-02 内嵌化的守门测试）。
    ///
    /// 这条比「下载校验」更要紧：内嵌之后**校验发生在编译期之外** —— 没人会去核对
    /// assets/fonts/ 里的 ttf 与 BUILTIN_FONTS 里的常量是否一致。替换字体文件时忘了同步常量
    /// （或反过来），后果是**装到用户库里的字节和 FONT.toml 记的 sha256 不一样**，
    /// 而那个 sha256 正是「盘上这支到底是不是清单里那支」的唯一凭据。
    ///
    /// install_builtin_font 会在运行时挡住这种不一致（当场 bail），这里让它在**跑测试时**就红。
    #[test]
    fn embedded_bytes_match_their_pinned_sha256() {
        for b in builtin_fonts() {
            assert!(!b.bytes.is_empty(), "{} 的内嵌字节是空的", b.id);
            let got = sha256_hex(b.bytes);
            assert_eq!(
                got.to_lowercase(),
                b.sha256.to_lowercase(),
                "{} 的内嵌字节与清单里的 sha256 不一致（换了字体文件却没同步常量？）",
                b.id
            );
            // 许可原文也要在（OFL 要求随附）—— 别让它悄悄变空
            assert!(!b.license_bytes.is_empty(), "{} 的内嵌许可原文是空的", b.id);
        }
    }

    /// 内嵌字体**真的能被解析出族名**（字节没坏、格式还是有效字体）。
    ///
    /// ⚠️ 这条同时守着「许可原文不能是 HTML 错误页」那类事故：
    /// 当年就是拿 GitHub 的 404 HTML 页面当过 LICENSE.txt（下载器不校验内容）。
    #[test]
    fn embedded_bytes_are_parseable_fonts() {
        for b in builtin_fonts() {
            let fam = family_of(b.bytes);
            assert!(fam.is_some(), "{} 的内嵌字节解不出族名（不是有效字体？）", b.id);
        }
    }

    /// ⚠️ **库里有两支 emoji 字体时，必须选中自带那支**（2026-10-02 实测踩到的坑）。
    ///
    /// 现场：我们换了 emoji 字体的 id（旧 COLRv1 → 新 CBDT），但**老机器上旧那份还在库里**。
    /// 只看 id 字典序的话：
    ///
    ///     "NotoColorEmoji" < "NotoColorEmoji-CBDT"  ⇒ 旧 COLRv1 被选中
    ///
    /// 而旧那支对 Takumi **等于不存在**（它只认 COLRv0）⇒ 封面 emoji **整片消失**，
    /// 界面却显示"已装"。所以自带（我们验证过格式）必须优先。
    #[test]
    fn builtin_emoji_wins_over_a_stale_leftover() {
        let root = tmp("plan-emoji-prio");
        std::fs::create_dir_all(&root).unwrap();
        // 旧遗留（id 字典序更小，会「赢」纯 id 排序）+ 自带那支（id 更长）
        put_font(&root, "NotoColorEmoji", "Noto Color Emoji", true, false);
        let b = builtin_fonts()
            .iter()
            .find(|b| b.id.to_lowercase().contains("emoji"))
            .expect("清单里得有 emoji 那支");
        put_font(&root, b.id, b.family, true, false);
        assert!("NotoColorEmoji" < b.id, "前提：旧 id 字典序更小（否则这条测不到那个坑）");
        let picked = emoji_font(&root).expect("应当选中一支");
        assert_eq!(picked.id, b.id, "自带那支必须赢过旧遗留");
    }

    /// \`set_default\` 对 emoji 字体**直接拒绝**（界面的第二道防线）。
    #[test]
    fn set_default_rejects_emoji_font() {
        let root = tmp("plan-set-default-emoji");
        std::fs::create_dir_all(&root).unwrap();
        put_font(&root, "Emoji", "Noto Color Emoji", true, false);
        put_font(&root, "WenKai", "霞鹜文楷 轻便版", false, false);

        let e = set_default(&root, "Emoji").unwrap_err();
        assert!(e.to_string().contains("emoji"), "报错要说清是 emoji 不受理：{e}");
        assert!(!read_meta(&root.join("Emoji")).unwrap().is_default);
        // 正常字体照旧
        set_default(&root, "WenKai").unwrap();
        assert!(read_meta(&root.join("WenKai")).unwrap().is_default);
    }

    /// ⚠️ **字体库只有一个位置**的钉子（2026-10-02 口径切换）。
    ///
    /// 口径：*所有内容围绕程序目录来，不要用户目录*。
    /// `ensure_dir` 建的就是 `<数据目录>/resources/fonts/` —— **不碰任何用户目录**，
    /// 也没有"老库一次性搬家"那个动作了（搬家是当年为了躲开构建目录漂移才有的，
    /// 代价是同一个库有两个可能的位置，CLI 与 WebUI 各看一份）。
    #[test]
    fn ensure_dir_is_the_program_dir_and_never_touches_user_home() {
        let data = tmp("ensure-data");
        let root = ensure_dir_at(&data).unwrap();
        assert_eq!(root, data.join("resources").join("fonts"));
        assert!(root.is_dir(), "ensure_dir 要把目录建出来");

        // 幂等：再调一次还是同一个目录（不新开一层、不搬东西）
        std::fs::write(root.join("Keep.ttf"), b"x").unwrap();
        assert_eq!(ensure_dir_at(&data).unwrap(), root);
        assert!(root.join("Keep.ttf").is_file(), "已装的不该被动过");

        // ⚠️ 数据根之外的任何位置都不该被建出来 —— 特别是用户目录那条老路
        let stray = data.parent().unwrap().join(".yekitrim");
        assert!(!stray.exists(), "不该在数据根旁边另建 .yekitrim/：{}", stray.display());
        // 同一条判据也钉住**新**状态目录名：字体库不该在数据根旁边开状态目录
        let stray_new = data.parent().unwrap().join(crate::path::STATE_DIR_NAME);
        assert!(
            !stray_new.exists(),
            "不该在数据根旁边另建状态目录：{}",
            stray_new.display()
        );
    }

    /// 老平铺布局的条目要带上 `flat: true`（界面打「老布局」标签的依据），
    /// 目录布局与未下载的自带字体不是 flat。
    #[test]
    fn flat_layout_entries_are_marked() {
        let root = tmp("flat-mark");
        std::fs::write(root.join("OldFont.ttf"), b"x").unwrap();
        let list = scan_all(&root);
        let flat: Vec<_> = list.iter().filter(|e| e.id == "OldFont").collect();
        assert_eq!(flat.len(), 1);
        assert!(flat[0].flat, "平铺条目必须标 flat");
        assert!(flat[0].installed);
        // 未下载的自带字体不是 flat
        let missing = list.iter().find(|e| !e.installed).expect("空库里应有未下载的自带字体");
        assert!(!missing.flat);
    }
}
