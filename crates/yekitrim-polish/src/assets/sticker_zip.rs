//! 上传 zip → 新贴纸包：`import_package_from_zip`。
//!
//! WebUI 的「＋ 添加表情包」走这里（`POST /api/polish/stickers/upload`，
//! 请求体就是压缩包原始字节）。这是全站**第二处**有意接收用户上传的入口
//! （第一处是封面背景图 base64），所以闸门都在这儿：
//!
//! - **只取 basename**：条目名里的 `../`（zip-slip）、绝对路径、盘符全部失去意义
//!   ——解压目标永远只是 `<包目录>/<文件名>`，不需要再做一次 containment 校验。
//! - **闸门**（`ZipLimits`）：条目数 / 单文件解压后大小 / 解压后总量。
//!   单文件那层按**实际写入字节数**判（`take(limit+1)` 后比对），
//!   所以中央目录里谎报尺寸也没用。
//! - **先解到暂存目录再 rename**：`root/.upload-<pid>-<rand>/` → 成功后 `fs::rename`
//!   到 `root/<包名>/`（同卷原子）。任何失败路径都 `remove_dir_all(暂存)`，因此
//!   **不存在半个包**。暂存名以 `.` 开头：`scan_packages` 只认非 `.` 开头的目录、
//!   `sanitize_pack_name` 也拒绝 `.` 开头，所以它既不出现在列表里也不可能被接口访问。
//!
//! 摊平（不保留子目录）是有意的：贴纸库本来就是扁平的（`list_package_files`
//! 只看包目录第一层），包内文件名还要当 `/api/polish/stickers/:pack/files/:name`
//! 的路径参数（`is_safe_component` 拒绝分隔符）。摊平后的**重名只留第一个**，
//! 其余计入 `skipped`（不静默丢——报给用户）。

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use crate::assets::sticker_index::{
    is_image_file_name, is_safe_component, sanitize_pack_name,
};

/// 解压闸门。默认值刻意宽松（贴纸都是小图），够装下一个几百张的包，
/// 又不至于让一个 zip 炸弹把磁盘/内存吃掉。
#[derive(Debug, Clone, Copy)]
pub struct ZipLimits {
    /// 中央目录里的条目数上限（防"十万条空条目"这种纯 DoS）
    pub max_entries: usize,
    /// 单个文件解压后的字节上限
    pub max_file_bytes: u64,
    /// 所有文件解压后的总字节上限
    pub max_total_bytes: u64,
}

impl ZipLimits {
    /// 条目数上限：防"十万条空条目"这种纯 DoS，与包的大小无关，因此**不做成配置**。
    pub const MAX_ENTRIES: usize = 2000;

    /// 按调用方的配置构造（服务端 `[stickers]`）：只覆盖单文件与总量两层，
    /// 条目数仍用固定的 [`Self::MAX_ENTRIES`]。
    ///
    /// 默认值（32 MiB / 256 MiB）是"小图包"口径；`[stickers]` 默认放到
    /// 64 MiB / 2 GiB，因为用户实测有 500 MB 的包（2026-11-19）。
    pub fn with_sizes(max_file_bytes: u64, max_total_bytes: u64) -> Self {
        Self {
            max_entries: Self::MAX_ENTRIES,
            max_file_bytes,
            max_total_bytes,
        }
    }
}

impl Default for ZipLimits {
    fn default() -> Self {
        Self {
            max_entries: Self::MAX_ENTRIES,
            max_file_bytes: 32 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
        }
    }
}

/// 上传导入结果。`skipped` = 被忽略的条目数（非图片、系统垃圾、目录、摊平后重名），
/// `total_entries` = 压缩包里一共有多少条目（让用户看得出"上传了 200、进来 180"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipImportReport {
    pub copied: usize,
    pub skipped: usize,
    pub total_entries: usize,
}

/// 把一个 zip 压缩包里的图片解压成新贴纸包 `root/<name>/`。
pub fn import_package_from_zip(
    root: &Path,
    name: &str,
    bytes: &[u8],
) -> anyhow::Result<ZipImportReport> {
    import_package_from_zip_with(root, name, bytes, &ZipLimits::default())
}

/// 同 [`import_package_from_zip`]，但闸门可调（测试用）。
pub fn import_package_from_zip_with(
    root: &Path,
    name: &str,
    bytes: &[u8],
    limits: &ZipLimits,
) -> anyhow::Result<ZipImportReport> {
    let name = sanitize_pack_name(name).ok_or_else(|| {
        anyhow::anyhow!("包名不合法（1~32 个字母/数字/下划线/短横线/汉字）: {name}")
    })?;
    let dst = root.join(&name);
    if dst.exists() {
        anyhow::bail!("贴纸包「{name}」已存在：换个包名，或先删掉那一个");
    }
    std::fs::create_dir_all(root)?;

    let staging = root.join(staging_name());
    let _ = std::fs::remove_dir_all(&staging); // 撞名概率≈0，纯保险
    std::fs::create_dir_all(&staging)?;

    let stats = match extract_into(&staging, bytes, limits) {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    if stats.copied == 0 {
        let _ = std::fs::remove_dir_all(&staging);
        anyhow::bail!("压缩包里没有图片（支持 gif/png/jpg/jpeg/webp）");
    }
    if let Err(e) = std::fs::rename(&staging, &dst) {
        let _ = std::fs::remove_dir_all(&staging);
        anyhow::bail!("贴纸包落盘失败（{}）: {e}", dst.display());
    }
    Ok(ZipImportReport {
        copied: stats.copied,
        skipped: stats.skipped,
        total_entries: stats.total_entries,
    })
}

struct ExtractStats {
    copied: usize,
    skipped: usize,
    total_entries: usize,
}

/// 暂存目录名。**必须以 `.` 开头**（见模块注释），`upload-` 前缀便于出问题时一眼认出来。
fn staging_name() -> String {
    let salt: u32 = rand::random();
    format!(".upload-{}-{:08x}", std::process::id(), salt)
}

/// 解压到 `staging`（调用方负责建/清）。
fn extract_into(staging: &Path, bytes: &[u8], limits: &ZipLimits) -> anyhow::Result<ExtractStats> {
    // 中央目录在这里被整份解析（zip crate 的内部表示）。清单规模由请求体上限
    // （`[stickers] upload_max_mb`，默认 1 GiB；路由硬顶 4 GiB）兜着，所以不额外预扫 EOCD。
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| anyhow::anyhow!("不是有效的 zip 压缩包: {e}"))?;
    let total_entries = archive.len();
    if total_entries > limits.max_entries {
        anyhow::bail!(
            "压缩包条目太多（{total_entries} 个，上限 {}）",
            limits.max_entries
        );
    }

    let mut copied = 0usize;
    let mut skipped = 0usize;
    let mut total_bytes: u64 = 0;
    for i in 0..total_entries {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| anyhow::anyhow!("压缩包第 {} 个条目读不出来: {e}", i + 1))?;
        if entry.is_dir() {
            skipped += 1;
            continue;
        }
        // 反斜杠也当分隔符（Windows 压出来的包会用 `\`）
        let raw = entry.name().replace('\\', "/");
        let file_name = raw.rsplit('/').next().unwrap_or("").to_string();
        // `__MACOSX/` 与 `.` 开头的条目要单独挡：macOS 的伴随文件叫 `._x.gif`，
        // **它有 .gif 后缀**，只靠扩展名过滤会把它当贴纸收进来。
        if raw.starts_with("__MACOSX/")
            || file_name.starts_with('.')
            || !is_image_file_name(&file_name)
            || !is_safe_component(&file_name)
        {
            skipped += 1;
            continue;
        }
        let out: PathBuf = staging.join(&file_name);
        if out.exists() {
            // 摊平后的重名：第一个胜。不覆盖（覆盖等于让后一个悄悄改掉前一个）。
            skipped += 1;
            continue;
        }
        let mut w = std::fs::File::create(&out)?;
        // 多要 1 字节：真写满了才说明"实际内容超限"，zip 里声明的尺寸不算数
        let written = std::io::copy(&mut entry.by_ref().take(limits.max_file_bytes + 1), &mut w)?;
        w.flush()?;
        if written > limits.max_file_bytes {
            anyhow::bail!(
                "「{file_name}」解压后超过单文件上限 {} MB",
                limits.max_file_bytes / 1024 / 1024
            );
        }
        total_bytes += written;
        if total_bytes > limits.max_total_bytes {
            anyhow::bail!(
                "解压后总量超过上限 {} MB（疑似 zip 炸弹）",
                limits.max_total_bytes / 1024 / 1024
            );
        }
        copied += 1;
    }
    Ok(ExtractStats {
        copied,
        skipped,
        total_entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    /// 造一个内存 zip。名字以 `/` 结尾的条目建成目录条目。
    fn make_zip(entries: &[(&str, &[u8])], method: CompressionMethod) -> Vec<u8> {
        let opts = SimpleFileOptions::default().compression_method(method);
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in entries {
            if name.ends_with('/') {
                w.add_directory(*name, opts).unwrap();
            } else {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
        }
        w.finish().unwrap().into_inner()
    }

    /// PNG 魔数 + 填充（内容不重要，我们只按扩展名认图片）
    fn png(fill: u8, len: usize) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        v.extend(std::iter::repeat_n(fill, len));
        v
    }

    /// 库里不该有 `.upload-*` 暂存目录残留（成功路径也要查）。
    fn assert_no_staging(root: &Path) {
        let leftovers: Vec<String> = std::fs::read_dir(root)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| n.starts_with(".upload-"))
                    .collect()
            })
            .unwrap_or_default();
        assert!(leftovers.is_empty(), "暂存目录没清掉: {leftovers:?}");
    }

    /// 失败路径：既没有留下目标包，也没有暂存目录。
    fn assert_no_residue(root: &Path, pack: &str) {
        assert!(!root.join(pack).exists(), "目标包不该存在: {pack}");
        assert_no_staging(root);
    }

    #[test]
    fn flat_images_are_extracted_stored_and_deflated() {
        for method in [CompressionMethod::Stored, CompressionMethod::Deflated] {
            let root = tempfile::tempdir().unwrap();
            let zip = make_zip(
                &[("a.gif", &png(1, 20)), ("b.png", &png(2, 20)), ("c.webp", &png(3, 20))],
                method,
            );
            let r = import_package_from_zip(root.path(), "pack", &zip).unwrap();
            assert_eq!(r.copied, 3, "{method:?}");
            assert_eq!(r.total_entries, 3, "{method:?}");
            let files = crate::assets::sticker_index::list_package_files(&root.path().join("pack"));
            assert_eq!(files, vec!["a.gif", "b.png", "c.webp"], "{method:?}");
            assert_no_staging(root.path());
        }
    }

    #[test]
    fn nested_dirs_are_flattened() {
        let root = tempfile::tempdir().unwrap();
        let zip = make_zip(
            &[("sub1/x.gif", &png(1, 10)), ("sub2/deep/y.jpg", &png(2, 10))],
            CompressionMethod::Stored,
        );
        let r = import_package_from_zip(root.path(), "flat", &zip).unwrap();
        assert_eq!(r.copied, 2);
        let files = crate::assets::sticker_index::list_package_files(&root.path().join("flat"));
        assert_eq!(files, vec!["x.gif", "y.jpg"]);
    }

    #[test]
    fn duplicate_basenames_keep_the_first_and_count_the_rest() {
        let root = tempfile::tempdir().unwrap();
        let zip = make_zip(
            &[
                ("a/same.gif", &png(0xAA, 10)),
                ("b/same.gif", &png(0xBB, 10)),
                ("c/other.gif", &png(0xCC, 10)),
            ],
            CompressionMethod::Stored,
        );
        let r = import_package_from_zip(root.path(), "dup", &zip).unwrap();
        assert_eq!(r.copied, 2);
        assert_eq!(r.skipped, 1);
        // 第一个胜：留下的是 0xAA 那份
        let kept = std::fs::read(root.path().join("dup").join("same.gif")).unwrap();
        assert_eq!(kept[8], 0xAA);
    }

    #[test]
    fn system_junk_and_non_images_are_skipped() {
        let root = tempfile::tempdir().unwrap();
        let zip = make_zip(
            &[
                ("keep.gif", &png(1, 10)),
                ("readme.txt", b"hello"),
                (".DS_Store", b"junk"),
                // macOS 伴随文件：有 .gif 后缀，必须靠"`.` 开头"挡掉
                ("__MACOSX/keep/._keep.gif", &png(2, 10)),
                // 隐藏图片同理
                (".hidden.gif", &png(3, 10)),
                ("dir/", b""),
            ],
            CompressionMethod::Stored,
        );
        let r = import_package_from_zip(root.path(), "junk", &zip).unwrap();
        assert_eq!(r.copied, 1);
        assert_eq!(r.skipped, 5);
        assert_eq!(r.total_entries, 6);
        let files = crate::assets::sticker_index::list_package_files(&root.path().join("junk"));
        assert_eq!(files, vec!["keep.gif"]);
    }

    /// zip-slip：条目名里带 `../`、绝对路径、反斜杠分隔 —— 都只取 basename，
    /// 所以文件只会落在包目录里，库外一个新文件都不该有。
    #[test]
    fn traversal_names_stay_inside_the_pack() {
        let root = tempfile::tempdir().unwrap();
        let outside = root.path().parent().unwrap().to_path_buf();
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        // zip crate 对 `../` 名字的写入是允许的（规范化只作用于分隔符），正合我们要测的场景
        w.start_file("../../evil.gif", opts).unwrap();
        w.write_all(&png(0x11, 10)).unwrap();
        w.start_file("/abs/evil2.gif", opts).unwrap();
        w.write_all(&png(0x22, 10)).unwrap();
        w.start_file("..\\evil3.gif", opts).unwrap();
        w.write_all(&png(0x33, 10)).unwrap();
        let zip = w.finish().unwrap().into_inner();

        let r = import_package_from_zip(root.path(), "slip", &zip).unwrap();
        assert_eq!(r.copied, 3);
        let pack = root.path().join("slip");
        for n in ["evil.gif", "evil2.gif", "evil3.gif"] {
            assert!(pack.join(n).is_file(), "{n} 应该落在包目录里");
        }
        // 库外不许出现任何东西
        assert!(!outside.join("evil.gif").exists());
        assert!(!outside.join("evil2.gif").exists());
        assert!(!outside.join("evil3.gif").exists());
        assert!(!root.path().join("../evil.gif").exists());
    }

    #[test]
    fn only_non_images_bails_and_leaves_nothing() {
        let root = tempfile::tempdir().unwrap();
        let zip = make_zip(
            &[("a.txt", b"x"), ("b.md", b"y")],
            CompressionMethod::Stored,
        );
        let e = import_package_from_zip(root.path(), "none", &zip).unwrap_err();
        assert!(e.to_string().contains("没有图片"), "{e}");
        assert_no_residue(root.path(), "none");
    }

    #[test]
    fn existing_pack_is_rejected_and_left_untouched() {
        let root = tempfile::tempdir().unwrap();
        let pack = root.path().join("taken");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("old.gif"), b"original").unwrap();

        let zip = make_zip(&[("new.gif", &png(1, 10))], CompressionMethod::Stored);
        let e = import_package_from_zip(root.path(), "taken", &zip).unwrap_err();
        assert!(e.to_string().contains("已存在"), "{e}");
        // 一个字节都没被合并进去
        assert_eq!(std::fs::read(pack.join("old.gif")).unwrap(), b"original");
        assert!(!pack.join("new.gif").exists());
        assert_no_residue(root.path(), "never");
    }

    #[test]
    fn bad_pack_name_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let zip = make_zip(&[("a.gif", &png(1, 10))], CompressionMethod::Stored);
        for bad in ["../x", "a/b", "a\\b", ".hidden", "", "  ", "CON"] {
            let e = import_package_from_zip(root.path(), bad, &zip).unwrap_err();
            assert!(e.to_string().contains("包名不合法"), "{bad}: {e}");
        }
        assert!(std::fs::read_dir(root.path()).unwrap().next().is_none());
    }

    #[test]
    fn garbage_bytes_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let e = import_package_from_zip(root.path(), "junk", b"not a zip at all").unwrap_err();
        assert!(e.to_string().contains("不是有效的 zip"), "{e}");
        assert_no_residue(root.path(), "junk");

        // 空 body 同理
        let e = import_package_from_zip(root.path(), "junk", b"").unwrap_err();
        assert!(e.to_string().contains("不是有效的 zip"), "{e}");
        assert_no_residue(root.path(), "junk");
    }

    #[test]
    fn entry_count_limit_is_enforced() {
        let root = tempfile::tempdir().unwrap();
        let zip = make_zip(
            &[("a.gif", &png(1, 10)), ("b.gif", &png(2, 10)), ("c.gif", &png(3, 10))],
            CompressionMethod::Stored,
        );
        let limits = ZipLimits {
            max_entries: 2,
            ..Default::default()
        };
        let e = import_package_from_zip_with(root.path(), "many", &zip, &limits).unwrap_err();
        assert!(e.to_string().contains("条目太多"), "{e}");
        assert_no_residue(root.path(), "many");
    }

    /// 单文件上限按**实际解压出的字节**判，不看 zip 里声明的尺寸。
    #[test]
    fn per_file_limit_counts_real_bytes() {
        let root = tempfile::tempdir().unwrap();
        let zip = make_zip(&[("big.gif", &png(1, 500))], CompressionMethod::Stored);
        let limits = ZipLimits {
            max_file_bytes: 100,
            ..Default::default()
        };
        let e = import_package_from_zip_with(root.path(), "big", &zip, &limits).unwrap_err();
        assert!(e.to_string().contains("单文件上限"), "{e}");
        assert_no_residue(root.path(), "big");
    }

    #[test]
    fn total_size_limit_is_enforced() {
        let root = tempfile::tempdir().unwrap();
        let a = png(1, 200);
        let zip = make_zip(
            &[("a.gif", &a), ("b.gif", &a), ("c.gif", &a)],
            CompressionMethod::Stored,
        );
        let limits = ZipLimits {
            max_total_bytes: 500, // 单文件 208 字节 → 第三个必然越界
            ..Default::default()
        };
        let e = import_package_from_zip_with(root.path(), "bomb", &zip, &limits).unwrap_err();
        assert!(e.to_string().contains("总量超过上限"), "{e}");
        assert_no_residue(root.path(), "bomb");
    }

    /// 暂存目录名必须不被 `scan_packages` 看见（否则上传中途列表里会冒出一个假包）。
    #[test]
    fn staging_dir_is_invisible_to_package_scan() {
        let root = tempfile::tempdir().unwrap();
        let name = staging_name();
        assert!(name.starts_with('.'));
        std::fs::create_dir_all(root.path().join(&name)).unwrap();
        std::fs::write(root.path().join(&name).join("x.gif"), png(1, 10)).unwrap();
        assert!(crate::assets::sticker_index::scan_packages(root.path()).is_empty());
    }
}
