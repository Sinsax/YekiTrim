//! 程序级字体库：`<程序目录>/resources/fonts/`。
//!
//! 用户把 `.ttf` / `.otf` / `.ttc` 丢进这个目录，界面就能把它们列出来当字幕/封面字体用。
//! 目录随程序走（[`crate::path::font_dir`]），**不随包分发任何字体文件**（授权原因，见 README）。
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

/// 建好字体库目录（`config init` / `server` 启动时用）。
pub fn ensure_dir() -> std::io::Result<PathBuf> {
    let d = dir();
    std::fs::create_dir_all(&d)?;
    Ok(d)
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
#[derive(Debug, Clone, serde::Serialize)]
pub struct FontEntry {
    /// 文件名去扩展名 —— 配置里存的就是它
    pub id: String,
    /// 带扩展名的文件名（渲染服务器 `/vt-fonts/<file>` 用）
    pub file: String,
    /// 字体的字族名（读 `name` 表；失败时等于 id）
    pub family: String,
    pub bytes: u64,
}

fn ext_rank(ext: &str) -> usize {
    let ext = ext.to_ascii_lowercase();
    FONT_EXTS.iter().position(|x| *x == ext).unwrap_or(FONT_EXTS.len())
}

/// 列出库里的字体（磁盘为准，按 id 排序）。目录不存在 → 空列表，不报错。
pub fn scan() -> Vec<FontEntry> {
    scan_at(&dir())
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
        out.push(FontEntry { id, file, family, bytes });
    }
    out.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()));
    out
}

/// 把配置里的字体 id 解析成库内文件。解析不到 → `None`（调用方回退系统字体栈）。
pub fn resolve_file(id: &str) -> Option<PathBuf> {
    resolve_file_at(&dir(), id)
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
