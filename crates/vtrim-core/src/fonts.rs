//! 程序级字体库：**用户固定目录 `~/.vtrim/fonts`**（存在即用）；否则回落程序目录的
//! `resources/fonts/`（老安装兼容）。写路径一律落用户目录（[`ensure_dir`] 含老库一次性搬家）。
//!
//! 用户把 `.ttf` / `.otf` 丢进这个目录，界面就能把它们列出来当字幕/封面字体用。
//! 解析规则见 [`crate::path::font_dir`]。
//!
//! # 自带字体（[\`BUILTIN_FONTS\`]）走**首次使用下载**，不随发行包分发
//!
//! 实测两支自带字体加起来约 **38 MB**（霞鹜文楷 Lite 13.5 MB + Noto Color Emoji 24.7 MB），
//! 提交进 git 会让仓库体积与克隆时间明显变差，所以**仓库只放清单**（URL + 校验和 + 许可），
//! 字体由 [\`download_builtin_fonts\`] 首次要用时落到库里。两支都是 **OFL-1.1**（可商用/可嵌入/可再分发），
//! 原样附 `LICENSE.txt`。
//!
//! ⚠️ **没下载 ≠ 看不见**：[\`scan_all\`] 会把"还没装的自带字体"也列出来（`installed: false`），
//! 界面据此给"下载"按钮。所以别把自带字体当成"库里必然存在"，
//! [\`is_builtin_id\`] 才是"这个 id 是不是自带的"的判据。
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

/// 一次性搬家记号：老库（程序目录 `resources/fonts/`）的内容复制进用户目录之后落下的。
/// 有了它就**不再搬第二次** —— 用户之后在库里删了哪支，下次启动也不会从老库又长回来。
const MIGRATED_MARKER: &str = ".migrated-from-program-dir";

/// 建好字体库目录并返回**写路径**（`config init` / `server` 启动 / 下载导入前用）。
///
/// ⚠️ 写一律落在**用户固定目录** `~/.vtrim/fonts`（字体库不随构建/安装目录漂移，见
/// [`crate::path::font_dir`]）：存在即直接返回；不存在时，若程序目录老库里真有字体，
/// 先**整体复制一次**（不动老库的任何文件），再落 [`MIGRATED_MARKER`]。取不到用户
/// 主目录时退回程序目录（老行为）。读侧的 [`dir`] 用"存在即用"的同一套解析，所以
/// 本函数返回之后，[`dir`] 与这里返回的就是同一个目录。
pub fn ensure_dir() -> std::io::Result<PathBuf> {
    ensure_dir_in(crate::path::user_home().as_deref(), &crate::path::exe_dir())
}

/// [`ensure_dir`] 的可注入版本（单测用）。
pub fn ensure_dir_in(user_home: Option<&Path>, exe_home: &Path) -> std::io::Result<PathBuf> {
    let Some(home) = user_home else {
        let d = exe_home.join("resources").join("fonts");
        std::fs::create_dir_all(&d)?;
        return Ok(d);
    };
    let user = home.join(".vtrim").join("fonts");
    let legacy = exe_home.join("resources").join("fonts");
    if !user.is_dir() && legacy.is_dir() {
        migrate_program_library(&user, &legacy)?;
    }
    std::fs::create_dir_all(&user)?;
    Ok(user)
}

/// 老库 → 用户目录的一次性复制（复制、不移动：绝不碰安装目录里的东西）。
fn migrate_program_library(user: &Path, legacy: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(user)?;
    // 顶层逐条搬：字体目录整目录复制、平铺字体文件按文件复制。点开头（记号/暂存）与
    // 非字体文件（发行包的 `放这里.txt`）都不搬。
    for entry in std::fs::read_dir(legacy)?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let src = entry.path();
        let dst = user.join(&name);
        if src.is_dir() {
            if font_file_in(&src).is_some() && !dst.exists() {
                copy_dir_recursive(&src, &dst)?;
            }
        } else if is_font_file_name(&name) && !dst.exists() {
            std::fs::copy(&src, &dst)?;
        }
    }
    std::fs::write(user.join(MIGRATED_MARKER), "")?;
    Ok(())
}

/// 递归复制目录（字体库搬家用：字体目录里是 `font.ttf` + `FONT.toml` + `LICENSE.txt`，很浅）。
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)?.flatten() {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
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
        let dir = std::env::temp_dir().join(format!("vtrim-fonts-test-{tag}-{}", std::process::id()));
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
        let missing = std::env::temp_dir().join("vtrim-fonts-does-not-exist-xyz");
        assert!(scan_at(&missing).is_empty());
    }

    #[test]
    fn family_of_rejects_garbage() {
        assert!(family_of(b"").is_none());
        assert!(family_of(b"not a font at all").is_none());
    }

    /// 只读诊断：本机系统字体能不能读出字族名。
    /// `cargo test -p vtrim-core --lib diagnose_font_family -- --ignored --nocapture`
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
    /// ⚠️ **当前恒为空串**：`vtrim-core` 没有 hasher 依赖（加依赖要单独评估），
    /// 字段先留着，等接上再加（见 `docs/plan/font-library.md` §10.2 的影响面）。
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

/// 目录布局（v2）的扫描。
pub fn scan_dirs(root: &Path) -> Vec<FontEntry> {
    let Ok(rd) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let slug = entry.file_name().to_string_lossy().to_string();
        if slug.starts_with('.') || !entry.path().is_dir() || !is_safe_id(&slug) {
            continue;
        }
        let Some(path) = font_file_in(&entry.path()) else { continue };
        let meta = read_meta(&entry.path()).unwrap_or_default();
        let internal = if meta.family.trim().is_empty() {
            cached_family(&path).unwrap_or_else(|| slug.clone())
        } else {
            meta.family.clone()
        };
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        out.push(FontEntry {
            id: slug.clone(),
            file: path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
            family: internal.clone(),
            bytes,
            display_name: if meta.display_name.trim().is_empty() { slug } else { meta.display_name },
            family_internal: internal,
            license: meta.license,
            scripts: meta.scripts,
            emoji: meta.emoji,
            builtin: meta.builtin,
            is_default: meta.is_default,
            // 目录布局不是平铺（`flat` 只属于散文件直接躺在库根目录的老布局）
            flat: false,
            // 它在磁盘上 scanned 出来的 ⇒ 当然是"装好了"（`installed: false` 只属于"清单里有、盘上没有"）
            installed: true,
        });
    }
    out
}

/// 两种布局合并：**目录布局优先**（同名 id 时目录赢）。
///
/// ⚠️ **还没下载的自带字体也会出现在结果里**（`installed: false`）—— 界面要能显示
/// "有这支、可以下载"。所以**别把 `scan_all().len()` 当"库里有多少字体"用**，
/// 要判"真能用"请过滤 `installed`（或直接 `resolve_all` 试解析）。
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
        });
    }
    out.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()));
    out
}

/// 目录布局下把 id 解析成文件。
pub fn resolve_dir_file(root: &Path, id: &str) -> Option<PathBuf> {
    let id = id.trim();
    if !is_safe_id(id) {
        return None;
    }
    font_file_in(&root.join(id))
}

/// 两种布局都能解析（目录优先）。
pub fn resolve_all(root: &Path, id: &str) -> Option<PathBuf> {
    resolve_dir_file(root, id).or_else(|| resolve_file_at(root, id))
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
    BuiltinFont {
        id: "LXGWWenKaiLite-Regular",
        file: "LXGWWenKaiLite-Regular.ttf",
        url: "https://github.com/lxgw/LxgwWenKai-Lite/releases/download/v1.522/LXGWWenKaiLite-Regular.ttf",
        sha256: "140c99ba4e28e817cec49bf82a0c5fcdc4fe633fb9dfda16d0ee8d59a8545f15",
        display_name: "霞鹜文楷 Lite",
        family: "霞鹜文楷 轻便版",
        license: "OFL-1.1",
        license_url: "https://github.com/lxgw/LxgwWenKai-Lite/raw/main/OFL.txt",
    },
    BuiltinFont {
        id: "NotoColorEmoji",
        file: "NotoColorEmoji-Regular.ttf",
        url: "https://github.com/google/fonts/raw/main/ofl/notocoloremoji/NotoColorEmoji-Regular.ttf",
        sha256: "4d82a18d8d95f60ba883ce242bbadbf84a576e987745dd7ba38d71c67cff2d73",
        display_name: "Noto Color Emoji",
        family: "Noto Color Emoji",
        license: "OFL-1.1",
        license_url: "https://github.com/google/fonts/raw/main/ofl/notocoloremoji/OFL.txt",
    },
];

/// 一支自带字体的清单项。
#[derive(Debug, Clone)]
pub struct BuiltinFont {
    pub id: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    /// 下载后必须对得上（不符 = 上游换了内容，**停下来**，别把不确定的字节装进用户环境）
    pub sha256: &'static str,
    pub display_name: &'static str,
    /// 期望的内部族名（写进 `FONT.toml`；真值以落盘后读到的为准）
    pub family: &'static str,
    pub license: &'static str,
    pub license_url: &'static str,
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

/// 下载**全部还缺的**自带字体（已装好的跳过）。返回实际下载的 id 列表。
///
/// ⚠️ 幂等：`builtin_installed` 为真就**不碰磁盘**（用户可能已经手动替换过那支字体）。
/// ⚠️ 下载完**必须校验 sha256**：不符 ⇒ 删掉刚下的文件 + 报错（**绝不**把不确定的字节留下）。
pub async fn download_builtin_fonts(root: &Path) -> anyhow::Result<Vec<String>> {
    let mut done = Vec::new();
    for b in BUILTIN_FONTS {
        if builtin_installed(root, b) {
            continue;
        }
        install_builtin_font(root, b).await?;
        done.push(b.id.to_string());
    }
    Ok(done)
}

/// 下载并安装**一支**自带字体（写 `<root>/<id>/{font.ttf,FONT.toml,LICENSE.txt}`）。
///
/// ⚠️ 幂等由**调用方**决定：这个函数本身"你叫我装我就装"（会覆盖已存在的）。
/// 想跳过已装好的用 [`builtin_installed`] 先判，或直接用 [`download_builtin_fonts`]。
pub async fn download_builtin_font(root: &Path, b: &BuiltinFont) -> anyhow::Result<()> {
    install_builtin_font(root, b).await
}

async fn install_builtin_font(root: &Path, b: &BuiltinFont) -> anyhow::Result<()> {
    use anyhow::Context as _;

    let dest_dir = builtin_dir(root, b);
    std::fs::create_dir_all(&dest_dir)
        .with_context(|| format!("建字体目录失败: {}", dest_dir.display()))?;
    // 下载到目录内的 `.part`（与模型下载同一套：先 part 再改名，中断可续）
    let part = dest_dir.join(format!("{}.part", b.file));

    crate::download::fetch_url_to(b.url, &part)
        .await
        .with_context(|| format!("下载字体失败: {} ({})", b.id, b.url))?;

    // 校验和：不符就删掉，别留一个"看着下载完了但内容不对"的文件
    let bytes = std::fs::read(&part).with_context(|| format!("读下载文件失败: {}", part.display()))?;
    let got = sha256_hex(&bytes);
    if !got.eq_ignore_ascii_case(b.sha256) {
        let _ = std::fs::remove_file(&part);
        anyhow::bail!(
            "字体校验和不符（{}）：期望 {}，实际 {} —— 上游内容可能已变，已删除下载文件",
            b.id, b.sha256, got
        );
    }

    // 内部族名以**文件里读到的**为准（清单里的只是提示）
    let family = family_of(&bytes).unwrap_or_else(|| b.family.to_string());
    let dest = dest_dir.join("font.ttf");
    std::fs::rename(&part, &dest).with_context(|| format!("改名失败: {}", dest.display()))?;

    write_meta(&dest_dir, &FontMeta {
        display_name: b.display_name.to_string(),
        family,
        license: b.license.to_string(),
        source_url: b.url.to_string(),
        emoji: b.id.to_ascii_lowercase().contains("emoji"),
        builtin: true,
        // ⚠️ 这两个**必须写**（别留空）：`sha256` 是"盘上这支到底是不是清单里那支"的凭据
        // （用户手动替换过字体时，它就是唯一的证据），`added_at` 是"什么时候装进来的"。
        // 以前这里漏了，落盘的 `FONT.toml` 里两个字段都是空串 —— 校验过的东西**没留下来**。
        sha256: got.clone(),
        added_at: chrono::Utc::now().to_rfc3339(),
        ..Default::default()
    })
    .with_context(|| format!("写 FONT.toml 失败: {}", dest_dir.display()))?;

    // 许可原文：**OFL 要求随附**，所以与字体一起落盘
    match crate::download::fetch_text_to(b.license_url, &dest_dir.join("LICENSE.txt")).await {
        Ok(()) => {}
        Err(e) => {
            // 许可拿不到不算致命（字体本身合法且已校验），但要在日志里说清楚
            tracing::warn!("自带字体的许可原文没下下来（{}）：{e}", b.id);
        }
    }
    Ok(())
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

/// 导入一支字体到库（目录布局 + 写 `FONT.toml`）。**失败原因要说清**，别静默塞进去。
///
/// ⚠️ 拒收 `.ttc`/`.otc`（字体集合）：**浏览器的 `@font-face` 不认集合**，
/// 收进来就会出现"预览回退到系统字体、出片却用了它"——这正是"预览与实际对不上"的一个来源。
/// 要收集合得先拆包，留后续。
pub fn import_font(root: &Path, src: &Path, display_name: Option<&str>) -> anyhow::Result<FontEntry> {
    let file = src.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let ext = Path::new(&file).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !matches!(ext.as_str(), "ttf" | "otf") {
        anyhow::bail!("只收 .ttf / .otf（.ttc / .otc 是字体集合，浏览器 @font-face 不认，请先拆包）");
    }
    let bytes = std::fs::read(src)?;
    let family = family_of(&bytes)
        .ok_or_else(|| anyhow::anyhow!("读不出字体内部族名（文件可能损坏或不是字体）"))?;
    let id = Path::new(&file).file_stem().and_then(|s| s.to_str()).unwrap_or("").trim().to_string();
    if !is_safe_id(&id) {
        anyhow::bail!("文件名不能当 id 用（含路径分隔符或控制字符）：{file}");
    }
    let dest_dir = root.join(&id);
    std::fs::create_dir_all(&dest_dir)?;
    std::fs::copy(src, dest_dir.join(&file))?;
    let meta = FontMeta {
        display_name: display_name.map(|d| d.trim().to_string()).filter(|d| !d.is_empty())
            .unwrap_or_else(|| family.clone()),
        family,
        license: String::new(),
        added_at: chrono::Utc::now().to_rfc3339(),
        // 导入时也记一份校验和：与自带字体那条路同口径，"盘上这支是什么"永远有据可查
        sha256: sha256_hex(&bytes),
        ..Default::default()
    };
    write_meta(&dest_dir, &meta)?;
    Ok(FontEntry {
        id,
        file,
        family: meta.family.clone(),
        bytes: bytes.len() as u64,
        display_name: meta.display_name,
        family_internal: meta.family,
        // 刚拷进来的，就在盘上
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
    let dir = root.join(id);
    if dir.is_dir() {
        if read_meta(&dir).map(|m| m.builtin).unwrap_or(false) {
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
pub fn set_default(root: &Path, id: &str) -> anyhow::Result<()> {
    let id = id.trim();
    if !is_safe_id(id) {
        anyhow::bail!("非法 id");
    }
    let dir = root.join(id);
    if !dir.is_dir() {
        anyhow::bail!("这支字体不在目录布局里（老布局没有元数据），先重新导入一次：{id}");
    }
    let mut meta = read_meta(&dir).unwrap_or_default();
    let mut entries = scan_dirs(root);
    for e in entries.iter_mut() {
        if e.id.eq_ignore_ascii_case(id) {
            meta.display_name = if e.display_name.is_empty() { e.id.clone() } else { e.display_name.clone() };
            meta.family = if e.family_internal.is_empty() { e.family.clone() } else { e.family_internal.clone() };
        }
    }
    // 先把其它支的 is_default 清掉（唯一默认）
    for e in scan_dirs(root) {
        if e.id.eq_ignore_ascii_case(id) {
            continue;
        }
        let d = root.join(&e.id);
        if let Some(mut m) = read_meta(&d) {
            if m.is_default {
                m.is_default = false;
                let _ = write_meta(&d, &m);
            }
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
        let slug = root.join("my-font");
        std::fs::create_dir_all(&slug).unwrap();
        std::fs::write(slug.join("font.ttf"), b"not-a-real-font").unwrap();
        write_meta(&slug, &FontMeta {
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
        assert_eq!(mine[0].id, "my-font");
        assert_eq!(mine[0].display_name, "我的字体");
        assert_eq!(mine[0].family_internal, "Real Internal Family");
        assert!(mine[0].builtin);
        assert!(resolve_all(&root, "my-font").is_some());
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

    /// 自带字体不可删除（判据在元数据的 builtin）
    #[test]
    fn builtin_cannot_be_removed() {
        let root = tmp("builtin");
        let slug = root.join("builtin-font");
        std::fs::create_dir_all(&slug).unwrap();
        std::fs::write(slug.join("font.ttf"), b"x").unwrap();
        write_meta(&slug, &FontMeta { builtin: true, ..Default::default() }).unwrap();
        let err = remove_font(&root, "builtin-font").unwrap_err().to_string();
        assert!(err.contains("程序自带字体不可删除"), "{err}");
        assert!(root.join("builtin-font").exists());
        // 非自带的能删
        let other = root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("font.ttf"), b"x").unwrap();
        assert!(remove_font(&root, "other").is_ok());
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

    /// ⚠️ **字体库固定到用户目录的钉子**：老库（程序目录）在首次 `ensure_dir` 时整体
    /// 复制进 `~/.vtrim/fonts`，之后写路径不再随构建/安装目录漂移。
    ///
    /// 三件事各自对应一次真实风险：
    /// ① 只搬**字体**（目录布局 + 平铺文件），发行包的 `放这里.txt`、点开头的伴随文件不搬；
    /// ② 搬完落**记号**，且"删了又搬回来"不会发生（记号在，第二次启动不重搬）；
    /// ③ 取不到主目录时退回程序目录（老行为），复制是**复制**不是移动 —— 绝不碰安装目录。
    #[test]
    fn ensure_dir_adopts_user_dir_and_migrates_legacy_once() {
        let home = tmp("ensure-home");
        let exe = tmp("ensure-exe");
        let legacy = exe.join("resources").join("fonts");
        // 老库：一支目录布局 + 一支平铺 + 杂项（不该搬的）
        let slug = legacy.join("MyFont");
        std::fs::create_dir_all(&slug).unwrap();
        std::fs::write(slug.join("font.ttf"), b"x").unwrap();
        write_meta(&slug, &FontMeta { display_name: "我的字体".into(), ..Default::default() }).unwrap();
        std::fs::write(legacy.join("Old.ttf"), b"y").unwrap();
        std::fs::write(legacy.join("放这里.txt"), b"hint").unwrap();
        std::fs::write(legacy.join(".hidden.ttf"), b"z").unwrap();

        let user = ensure_dir_in(Some(&home), &exe).unwrap();
        assert_eq!(user, home.join(".vtrim").join("fonts"));
        assert!(user.join("MyFont").join("font.ttf").is_file(), "目录布局要整目录搬过来");
        assert!(user.join("MyFont").join("FONT.toml").is_file(), "元数据跟着走");
        assert!(user.join("Old.ttf").is_file(), "平铺字体也要搬");
        assert!(!user.join("放这里.txt").exists(), "杂项文件不搬");
        assert!(!user.join(".hidden.ttf").exists(), "点开头的不搬");
        assert!(user.join(MIGRATED_MARKER).exists(), "搬完要落记号");
        // 老库原样保留（复制不是移动）
        assert!(slug.join("font.ttf").is_file());

        // 记号在 ⇒ 用户目录里删掉的东西**不会**从老库长回来
        std::fs::remove_dir_all(user.join("MyFont")).unwrap();
        ensure_dir_in(Some(&home), &exe).unwrap();
        assert!(!user.join("MyFont").exists(), "有记号就不再搬第二次");

        // 老库不存在：用户目录照样建（全新安装的路径）
        let home2 = tmp("ensure-home2");
        let exe2 = tmp("ensure-exe2");
        let user2 = ensure_dir_in(Some(&home2), &exe2).unwrap();
        assert!(user2.is_dir());

        // 取不到主目录：退回程序目录（老行为）
        let exe3 = tmp("ensure-exe3");
        let d3 = ensure_dir_in(None, &exe3).unwrap();
        assert_eq!(d3, exe3.join("resources").join("fonts"));
        assert!(d3.is_dir());
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
