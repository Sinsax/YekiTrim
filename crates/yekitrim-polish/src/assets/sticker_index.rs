//! 本地贴纸索引: 扫描 `sticker_dir()` 根目录, 归一化分组, 评分匹配。
//!
//! 数据来源: 程序目录 `stickers/` 下的贴纸文件 (EmoteLab 导出经独立脚本重命名) +
//! 可选清单 `stickers.json` (为每组提供 tags/aliases, 用于中英同义匹配)。
//!
//! 命名解析 (三级结构):
//! - **组** = 括号前主名相同: `呆.gif` / `呆_1.gif` / `呆(贴纸)_1.gif` → 组 `呆`
//! - **变体** = 括号内容 (无括号为基础变体 ""): `呆` 组含基础变体 + 「贴纸」变体
//! - **帧** = 变体内末尾 `_数字`: `呆(贴纸)_1/2/3.gif` → 「贴纸」变体的 3 帧
//! 例: `打字(恼怒).gif` → 组 `打字`, 变体「恼怒」; `爱心_1.gif` → 组 `爱心`, 基础变体帧

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 一个贴纸变体 (如「呆」组的「贴纸」变体, 含多帧)
#[derive(Debug, Clone)]
pub struct StickerVariant {
    /// 变体词: 基础变体为 "" (无括号), 否则为括号内容 (如 "贴纸" / "恼怒")
    pub name: String,
    /// 该变体的帧文件 (相对 sticker_dir, 保序)
    pub files: Vec<PathBuf>,
}

/// 一个本地贴纸包：`stickers/<包名>/` 子文件夹。
/// 这是新的推荐结构：一个文件夹 = 一个主播/主题包，文件夹名即 LLM 候选名。
#[derive(Debug, Clone)]
pub struct StickerPackage {
    /// 包名（文件夹名，如 "ysyk"）
    pub name: String,
    /// 包内所有图片文件（绝对路径，保序）
    pub files: Vec<PathBuf>,
}

impl StickerPackage {
    /// 包内文件数
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

/// 「一个文件名算不算贴纸图片」的**唯一判据**：只认这 5 种扩展名（大小写不敏感）。
///
/// 扫描包（`scan_packages`）、列包内文件（`list_package_files`）、上传压缩包解压
/// （`sticker_zip`）三条路径都走它 —— 这张清单以前在两个地方各写了一遍，
/// 新增格式时最容易漏改的就是这种散落副本。
pub fn is_image_file_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|x| x.to_str())
        .map(|x| {
            matches!(
                x.to_ascii_lowercase().as_str(),
                "gif" | "png" | "jpg" | "jpeg" | "webp"
            )
        })
        .unwrap_or(false)
}

/// 路径版判据（`scan_packages` / `list_package_files` 用）。
pub fn is_image_path(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .map(is_image_file_name)
        .unwrap_or(false)
}

/// 贴纸包 TOML 元数据（`stickers/<包名>.toml`，由 `stickers build --llm` 生成）。
/// 采用 tag → 文件列表的双向索引，供 LLM 两步选择使用。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StickerPackageMeta {
    /// 包名（与文件夹同名，如 "ysyk"）
    pub name: String,
    /// 简短描述（可人工补充）
    #[serde(default)]
    pub description: String,
    /// tag → 文件列表
    #[serde(default)]
    pub tags: Vec<StickerTagMeta>,
}

/// 一个 tag 及其关联文件。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StickerTagMeta {
    /// 语义 tag（如 "害怕" / "laugh"）
    pub tag: String,
    /// 该 tag 下的文件列表（如 ["害怕_1.gif", "害怕_2.gif"]）
    pub files: Vec<String>,
}

/// 扫描贴纸包目录，返回包内所有图片文件名（不含路径，保序）。
pub fn list_package_files(package_dir: &Path) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(package_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| {
                    let p = e.path();
                    if p.is_file() && is_image_path(&p) {
                        p.file_name().and_then(|n| n.to_str()).map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// 从 LLM 生成的 `文件 -> tags` 映射聚合出 `StickerPackageMeta`。
/// 一个文件可出现在多个 tag 下。
pub fn build_package_meta_from_tags(
    package_name: &str,
    file_tags: &[(String, Vec<String>)],
) -> Option<StickerPackageMeta> {
    if file_tags.is_empty() {
        return None;
    }
    let mut tag_map: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for (file, tags) in file_tags {
        for tag in tags {
            let t = tag.trim().to_string();
            if t.is_empty() {
                continue;
            }
            tag_map.entry(t).or_default().push(file.clone());
        }
    }
    if tag_map.is_empty() {
        return None;
    }
    let tags: Vec<StickerTagMeta> = tag_map
        .into_iter()
        .map(|(tag, files)| StickerTagMeta { tag, files })
        .collect();
    Some(StickerPackageMeta {
        name: package_name.to_string(),
        description: String::new(),
        tags,
    })
}

/// 读取 `stickers/<包名>.toml` 元数据。
pub fn load_package_meta(root: &Path, package_name: &str) -> Option<StickerPackageMeta> {
    let path = root.join(format!("{package_name}.toml"));
    let raw = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&raw).ok()
}

/// 将 `StickerPackageMeta` 序列化为 TOML 字符串。
pub fn package_meta_to_toml(meta: &StickerPackageMeta) -> anyhow::Result<String> {
    Ok(toml::to_string_pretty(meta)?)
}

/// 获取某 tag 下的候选文件列表。
pub fn tag_files(meta: &StickerPackageMeta, tag: &str) -> Vec<String> {
    meta.tags
        .iter()
        .find(|t| t.tag == tag)
        .map(|t| t.files.clone())
        .unwrap_or_default()
}

/// 全局已 build 贴纸包索引（`stickers/sticker.toml`）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct StickerIndexFile {
    /// 已 build 的贴纸包名列表
    #[serde(default)]
    pub packages: Vec<String>,
}

/// 读取 `stickers/sticker.toml` 全局索引。
pub fn load_sticker_index(root: &Path) -> StickerIndexFile {
    let path = root.join("sticker.toml");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default()
}

/// 写入 `stickers/sticker.toml` 全局索引。
pub fn save_sticker_index(root: &Path, index: &StickerIndexFile) -> anyhow::Result<()> {
    let path = root.join("sticker.toml");
    let toml_str = toml::to_string_pretty(index)?;
    std::fs::write(&path, toml_str)?;
    Ok(())
}

/// 注册一个已 build 的贴纸包到全局索引（幂等）。
pub fn register_package(root: &Path, package_name: &str) -> anyhow::Result<()> {
    let mut index = load_sticker_index(root);
    if !index.packages.iter().any(|p| p == package_name) {
        index.packages.push(package_name.to_string());
        index.packages.sort();
        save_sticker_index(root, &index)?;
    }
    Ok(())
}

/// 判断 `sticker_source` 中的某项是否表示本地贴纸包（具体包名）。
pub fn is_package_source(root: &Path, source: &str) -> bool {
    let dir = root.join(source);
    dir.is_dir()
}

// ============================================================
// 贴纸包管理（WebUI：导入 / 逐文件 tags / 删除）
// ============================================================

/// Windows 保留设备名（`CON` / `NUL` / `COM1`…）：同名的目录根本建不出来。
fn is_windows_reserved(stem: &str) -> bool {
    let up = stem.to_ascii_uppercase();
    matches!(up.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (up.len() == 4
            && (up.starts_with("COM") || up.starts_with("LPT"))
            && up.as_bytes()[3].is_ascii_digit())
}

/// 单段名能否安全地当作一段路径用：非空、非 `.`/`..`、不含分隔符与冒号、
/// 无控制字符、不是 Windows 保留名。
///
/// 用于 `<包名>` 与包内文件名（两者都会变成 `/api/admin/stickers/...` 的路径参数）。
/// 真正的越权拦截是 `canonicalize()` 之后的 `starts_with(root)`，这里只是第一道。
pub fn is_safe_component(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars().count() <= 128
        && !s.chars().any(|c| c == '/' || c == '\\' || c == ':' || c.is_control())
        && !is_windows_reserved(s)
}

/// 包名规范化：字母/数字（含汉字、假名等 Unicode 字母）、`_`、`-`，1~32 字符。
///
/// 包名会变成目录名、`<包名>.toml` 文件名、LLM prompt 里的候选名，也是接口的路径参数，
/// 所以拒绝路径分隔符、`..`、以 `.` 开头、空白与控制字符。
pub fn sanitize_pack_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    if name.is_empty() || name.starts_with('.') || name.chars().count() > 32 {
        return None;
    }
    let ok = name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-');
    if !ok || is_windows_reserved(name) {
        return None;
    }
    Some(name.to_string())
}

/// 导入结果：`copied` = 复制进来的图片数，`skipped` = 来源目录里被忽略的其它条目数
/// （非图片文件与子目录都算）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportReport {
    pub copied: usize,
    pub skipped: usize,
}

/// 把一个普通目录里的图片复制成新贴纸包 `root/<name>/`。
///
/// - `src` 必须存在、是目录，且**不在 `root` 内**（否则等于把库往自己身上拷）
/// - 目标 `root/<name>` 已存在 → 报错：宁可让人换个名字，也不合并/覆盖已有包
/// - 只复制 `list_package_files` 认的 5 种扩展名，其余条目跳过并计数
/// - 一张图片都没复制成功 → 删掉刚建的空目录再报错（不留半个包）
///
/// **复制而不是搬移**：来源目录原样留着，导错了删掉包即可。
pub fn import_package(root: &Path, name: &str, src: &Path) -> anyhow::Result<ImportReport> {
    let name = sanitize_pack_name(name).ok_or_else(|| {
        anyhow::anyhow!("包名不合法（1~32 个字母/数字/下划线/短横线/汉字）: {name}")
    })?;
    if !src.is_dir() {
        anyhow::bail!("来源目录不存在或不是目录: {}", src.display());
    }
    let src_real = src.canonicalize().unwrap_or_else(|_| src.to_path_buf());
    let root_real = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if src_real.starts_with(&root_real) {
        anyhow::bail!("来源目录就在贴纸库内，不能导入自己: {}", src_real.display());
    }
    let dst = root.join(&name);
    if dst.exists() {
        anyhow::bail!("贴纸包「{name}」已存在：换个包名，或先删掉那一个");
    }

    let files = list_package_files(src);
    if files.is_empty() {
        anyhow::bail!("来源目录里没有图片（支持 gif/png/jpg/jpeg/webp）");
    }
    std::fs::create_dir_all(&dst)?;
    let mut copied = 0usize;
    let mut failed: Vec<String> = Vec::new();
    for f in &files {
        match std::fs::copy(src.join(f), dst.join(f)) {
            Ok(_) => copied += 1,
            Err(e) => failed.push(format!("{f}: {e}")),
        }
    }
    if copied == 0 {
        let _ = std::fs::remove_dir_all(&dst);
        anyhow::bail!("一张图片都没复制成功（{}）", failed.join("; "));
    }
    let entries = std::fs::read_dir(src).map(|e| e.count()).unwrap_or(files.len());
    Ok(ImportReport {
        copied,
        skipped: entries.saturating_sub(files.len()),
    })
}

/// 反查每个文件命中的 tags（用于预览表格的「tags」列）。未构建的包返回空表。
pub fn file_tags_map(root: &Path, package_name: &str) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let Some(meta) = load_package_meta(root, package_name) else {
        return out;
    };
    for t in &meta.tags {
        for f in &t.files {
            out.entry(f.clone()).or_default().push(t.tag.clone());
        }
    }
    out
}

/// 从 `sticker.toml` 全局索引里摘掉一个包名（幂等）。
pub fn unregister_package(root: &Path, package_name: &str) -> anyhow::Result<()> {
    let mut index = load_sticker_index(root);
    let before = index.packages.len();
    index.packages.retain(|p| p != package_name);
    if index.packages.len() != before {
        save_sticker_index(root, &index)?;
    }
    Ok(())
}

/// 删除一个贴纸包：目录 + `<包名>.toml` + 全局索引项（幂等）。返回是否真的删掉了目录。
///
/// 只动贴纸库里的这三样；已经复制进各项目 `assets/stickers/` 的文件不受影响，
/// 项目 `polish.toml` 里若还写着这个包名也只是匹配不到，不会报错。
pub fn remove_package(root: &Path, package_name: &str) -> anyhow::Result<bool> {
    let name = sanitize_pack_name(package_name)
        .ok_or_else(|| anyhow::anyhow!("包名不合法: {package_name}"))?;
    let dir = root.join(&name);
    let removed = if dir.is_dir() {
        std::fs::remove_dir_all(&dir)?;
        true
    } else {
        false
    };
    let _ = std::fs::remove_file(root.join(format!("{name}.toml")));
    unregister_package(root, &name)?;
    Ok(removed)
}

/// 一个贴纸组: 括号前主名相同的所有变体
#[derive(Debug, Clone)]
pub struct StickerGroup {
    /// 主名 (去括号, 如 "呆" / "爱心" / "打字")
    pub name: String,
    /// 变体列表 (含基础变体 "")
    pub variants: Vec<StickerVariant>,
    /// 清单 tags (可为空)
    pub tags: Vec<String>,
    /// 清单 aliases (可为空)
    pub aliases: Vec<String>,
}

impl StickerGroup {
    /// 组内全部文件 (所有变体平铺)
    pub fn all_files(&self) -> Vec<&PathBuf> {
        self.variants.iter().flat_map(|v| v.files.iter()).collect()
    }

    /// 变体词列表 (含 "" 基础变体)
    pub fn variant_names(&self) -> Vec<&str> {
        self.variants.iter().map(|v| v.name.as_str()).collect()
    }
}

/// keyword → 组的匹配结果
#[derive(Debug, Clone)]
pub struct MatchResult {
    pub group_index: usize,
    pub score: u32,
    /// 命中的字段 (用于日志/诊断)
    pub via: String,
    /// 命中的变体词 (如 keyword=生气 命中 打字(生气) → Some("生气")), 用于选帧
    pub preferred_variant: Option<String>,
}

/// 贴纸索引: 扫描一次, 供匹配/诊断/复制共用
pub struct StickerIndex {
    root: PathBuf,
    groups: Vec<StickerGroup>,
}

impl StickerIndex {
    /// 扫描贴纸目录根目录 (非递归)。目录不存在时返回空索引。
    pub fn scan(root: &Path) -> Self {
        let metas = load_sticker_meta(root);
        let mut groups: Vec<StickerGroup> = Vec::new();
        let mut by_name: HashMap<String, usize> = HashMap::new();

        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue; // 跳过子目录 (backup/ 等)
                }
                let fname = match path.file_name().and_then(|s| s.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                if fname == "stickers.json" {
                    continue;
                }
                let Some((main, variant)) = parse_filename(fname) else { continue };
                let gi = match by_name.get(&main) {
                    Some(i) => *i,
                    None => {
                        let g = StickerGroup {
                            name: main.clone(),
                            variants: vec![],
                            tags: vec![],
                            aliases: vec![],
                        };
                        groups.push(g);
                        by_name.insert(main.clone(), groups.len() - 1);
                        groups.len() - 1
                    }
                };
                let g = &mut groups[gi];
                match g.variants.iter_mut().find(|v| v.name == variant) {
                    Some(v) => v.files.push(path),
                    None => g.variants.push(StickerVariant { name: variant, files: vec![path] }),
                }
            }
        }

        // 变体排序: 基础变体 "" 在前 (字典序最小)
        for g in &mut groups {
            g.variants.sort_by(|a, b| a.name.cmp(&b.name));
        }

        // 附着清单 tags/aliases (按 name 匹配)
        let meta_by_name: HashMap<&str, &StickerMeta> =
            metas.iter().map(|m| (m.name.as_str(), m)).collect();
        for g in &mut groups {
            if let Some(m) = meta_by_name.get(g.name.as_str()) {
                g.tags = m.tags.clone();
                g.aliases = m.aliases.clone();
            }
        }
        for m in &metas {
            if !meta_by_name.contains_key(m.name.as_str()) {
                // 清单里的组在磁盘上无文件: 登记为空组, 匹配后由调用方跳过
                groups.push(StickerGroup {
                    name: m.name.clone(),
                    variants: vec![],
                    tags: m.tags.clone(),
                    aliases: m.aliases.clone(),
                });
            }
        }

        Self { root: root.to_path_buf(), groups }
    }

    pub fn groups(&self) -> &[StickerGroup] {
        &self.groups
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// keyword → 得分最高的组。同分按「同义词词典顺序 → 名称字典序」确定性选择。
    pub fn match_keyword(&self, keyword: &str) -> Option<MatchResult> {
        let kw = normalize_keyword(keyword);
        if kw.is_empty() {
            return None;
        }
        let syns = synonyms(&kw);
        let mut best: Option<(u32, usize, Option<usize>, Option<String>, String)> = None;
        for (i, g) in self.groups.iter().enumerate() {
            if g.all_files().is_empty() && g.tags.is_empty() && g.aliases.is_empty() {
                continue; // 无文件且无清单信息的组不参与
            }
            if let Some((score, syn_idx, via, prefer)) = score_group(&kw, g, &syns) {
                let better = match &best {
                    None => true,
                    Some((bs, bi, bsi, _, _)) => {
                        score > *bs
                            || (score == *bs && syn_idx < *bsi)
                            || (score == *bs && syn_idx == *bsi
                                && g.name < self.groups[*bi].name)
                    }
                };
                if better {
                    best = Some((score, i, syn_idx, prefer, via));
                }
            }
        }
        best.map(|(score, i, _, prefer, via)| MatchResult {
            group_index: i,
            score,
            via,
            preferred_variant: prefer,
        })
    }

    /// 按分数降序返回前 n 个候选 (诊断/测试用)。同分按词典顺序 → 名称字典序。
    pub fn candidates(&self, keyword: &str, n: usize) -> Vec<(u32, usize, String)> {
        let kw = normalize_keyword(keyword);
        let syns = synonyms(&kw);
        let mut scored: Vec<(u32, usize, Option<usize>, String)> = self
            .groups
            .iter()
            .enumerate()
            .filter_map(|(i, g)| {
                if g.all_files().is_empty() && g.tags.is_empty() && g.aliases.is_empty() {
                    return None;
                }
                score_group(&kw, g, &syns).map(|(s, si, via, _)| (s, i, si, via))
            })
            .collect();
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.2.cmp(&b.2))
                .then_with(|| self.groups[a.1].name.cmp(&self.groups[b.1].name))
        });
        scored.truncate(n);
        scored.into_iter().map(|(s, i, _, via)| (s, i, via)).collect()
    }

    /// 匹配命中的组是否可落地 (有实际文件)
    pub fn has_files(&self, group_index: usize) -> bool {
        !self.groups[group_index].all_files().is_empty()
    }

    /// 确定性变体/帧选择:
    /// 1. `prefer` 指定变体 (keyword 命中括号变体词) → 优先在该变体的帧中选
    /// 2. 否则基础变体 (无括号) 存在时优先 (keyword 只命中主名, 选基础形态更贴合)
    /// 3. 否则组内全部文件平铺选
    /// 同一 (group, seed) 结果恒定 (渲染可复现), 不同 seed 尽量选不同帧。
    pub fn pick_variant(&self, group_index: usize, seed: f64, prefer: Option<&str>) -> Option<String> {
        let g = &self.groups[group_index];
        if let Some(p) = prefer {
            if let Some(v) = g.variants.iter().find(|v| v.name == p) {
                if let Some(f) = pick_from(&v.files, seed) {
                    return Some(f);
                }
            }
        }
        if let Some(base) = g.variants.iter().find(|v| v.name.is_empty()) {
            if let Some(f) = pick_from(&base.files, seed) {
                return Some(f);
            }
        }
        pick_from(&g.all_files().into_iter().cloned().collect::<Vec<_>>(), seed)
    }
}

/// 在文件列表中按 seed 确定性选一个, 返回文件 stem
fn pick_from(files: &[PathBuf], seed: f64) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    let idx = if files.len() == 1 {
        0
    } else {
        let frac = (seed.fract() + 1.0).abs();
        ((frac * 1000003.0) as usize) % files.len()
    };
    files[idx].file_stem().and_then(|s| s.to_str()).map(|s| s.to_string())
}

// ============================================================
// 命名解析
// ============================================================

/// 文件名 → (主名, 变体词)。
/// 变换顺序: 去扩展名 → 空格转 `_` → 剥离末尾 `_数字` 帧号 → 拆括号。
/// `呆(贴纸)_1.gif` → ("呆", "贴纸"); `爱心_1.gif` → ("爱心", ""); `被击中(爱心).gif` → ("被击中", "爱心")
fn parse_filename(fname: &str) -> Option<(String, String)> {
    let stem = Path::new(fname).file_stem()?.to_str()?.trim().to_string();
    if stem.is_empty() {
        return None;
    }
    let underscored = stem.replace(' ', "_");
    let no_frame = strip_frame_suffix(&underscored);
    match no_frame.find('(') {
        Some(pos) => {
            let head = no_frame[..pos].trim_end_matches('_').to_string();
            if head.is_empty() {
                return None;
            }
            let variant = match no_frame[pos + 1..].find(')') {
                Some(end) => no_frame[pos + 1..pos + 1 + end].trim().to_string(),
                None => String::new(),
            };
            Some((head, variant))
        }
        None => Some((no_frame, String::new())),
    }
}

/// 剥离末尾 `_数字` 帧号后缀: `爱心_1` → `爱心`; `呆(贴纸)_1` → `呆(贴纸)`
fn strip_frame_suffix(s: &str) -> String {
    if let Some(pos) = s.rfind('_') {
        let (head, tail) = s.split_at(pos);
        let tail = &tail[1..];
        if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
            return head.to_string();
        }
    }
    s.to_string()
}

/// query 归一化: 小写 + 去空格
fn normalize_keyword(kw: &str) -> String {
    kw.trim().to_lowercase().replace(' ', "_")
}

// ============================================================
// 贴纸包扫描（推荐结构: stickers/<包名>/*.gif）
// ============================================================

/// 扫描 `sticker_dir()` 根目录下的贴纸包子文件夹。
/// 每个子文件夹 = 一个 `StickerPackage`，文件夹名即 LLM 候选名。
pub fn scan_packages(root: &Path) -> Vec<StickerPackage> {
    let mut packages = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return packages;
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for dir in dirs {
        let name = match dir.file_name().and_then(|s| s.to_str()) {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };
        if name.starts_with('.') || name == "backup" {
            continue;
        }
        let mut files: Vec<PathBuf> = Vec::new();
        if let Ok(file_entries) = std::fs::read_dir(&dir) {
            let mut file_paths: Vec<PathBuf> = file_entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && is_image_path(p))
                .collect();
            file_paths.sort();
            files.append(&mut file_paths);
        }
        if !files.is_empty() {
            packages.push(StickerPackage { name, files });
        }
    }
    packages
}

/// 从贴纸包中确定性选一个文件 stem。
/// 同 (package, seed) 结果恒定，不同 seed 尽量选不同文件。
pub fn pick_package_file(packages: &[StickerPackage], package_name: &str, seed: f64) -> Option<String> {
    let pkg = packages.iter().find(|p| p.name == package_name)?;
    pick_from(&pkg.files, seed)
}

/// 生成本地贴纸候选文本（注入 LLM prompt）。
/// 优先读取 `<包名>.toml` 元数据（由 `stickers build` 生成），
/// 只暴露真实存在的贴纸包，避免 LLM 引用不存在的文件。
pub fn local_sticker_candidates_text(root: &Path) -> String {
    let packages = scan_packages(root);
    if packages.is_empty() {
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();
    for pkg in &packages {
        if let Some(meta) = load_package_meta(root, &pkg.name) {
            let desc = if meta.description.is_empty() {
                String::new()
            } else {
                format!(" - {}", meta.description)
            };
            let tags: Vec<String> = meta
                .tags
                .iter()
                .map(|t| format!("{}({})", t.tag, t.files.len()))
                .collect();
            lines.push(format!("{}{}  可用 tags: {}", pkg.name, desc, tags.join(" ")));
        } else {
            // 没有 TOML 时降级为仅包名（仍真实存在）
            lines.push(format!("{}({}个)", pkg.name, pkg.file_count()));
        }
    }

    format!("## 可用本地贴纸包（只能从以下包名选择）\n{}\n", lines.join("\n"))
}

// ============================================================
// 评分匹配
// ============================================================

/// 对单个组计算匹配分。返回 (score, 同义词词典索引, via, 命中的变体词)。
/// 所有比较大小写不敏感。
fn score_group(
    kw: &str,
    g: &StickerGroup,
    syns: &[(usize, &str)],
) -> Option<(u32, Option<usize>, String, Option<String>)> {
    let name_l = g.name.to_lowercase();
    let var_names: Vec<String> = g.variants.iter().map(|v| v.name.to_lowercase()).collect();

    // 1. 直接命中: query == name (100) > variant (95) > tag/alias (100)
    if name_l == kw {
        return Some((100, None, "name".into(), None));
    }
    if let Some(v) = var_names.iter().find(|v| !v.is_empty() && **v == kw) {
        return Some((95, None, "variant".into(), Some(v.clone())));
    }
    if g.tags.iter().any(|t| t.to_lowercase() == kw) || g.aliases.iter().any(|a| a.to_lowercase() == kw) {
        return Some((100, None, "tag/alias".into(), None));
    }

    // 1.5. tag/alias 词干+子串模糊: chat ↔ tag "chatting"/"chat", 短边 ≥2 字符
    let kw_stem = stem(kw);
    for t in g.tags.iter().chain(g.aliases.iter()) {
        let tl = t.to_lowercase();
        let t_stem = stem(&tl);
        if kw_stem == t_stem && !kw_stem.is_empty() {
            return Some((70, None, format!("tag-stem:{t}"), None));
        }
        let (short, long) = if tl.len() <= kw.len() {
            (tl.as_str(), kw)
        } else {
            (kw, tl.as_str())
        };
        if short.chars().count() >= 2 && long.contains(short) {
            return Some((70, None, format!("tag-contains:{t}"), None));
        }
    }

    // 2. 同义词命中: 词典词 (按词典顺序优先) + 清单 tag/alias (低优先)
    //    name==syn 90 > variant==syn 85 > 包含 70
    let mut syn_items: Vec<(usize, &str)> = syns.to_vec();
    for t in g.tags.iter().chain(g.aliases.iter()) {
        syn_items.push((usize::MAX, t.as_str()));
    }
    let mut best_syn: Option<(usize, u32, String, Option<String>)> = None;
    for (idx, syn) in syn_items {
        let s = syn.to_lowercase();
        let hit: Option<(u32, Option<String>)> = if name_l == s {
            Some((90, None))
        } else if let Some(v) = var_names.iter().find(|v| !v.is_empty() && **v == s) {
            Some((85, Some(v.clone())))
        } else if name_l.contains(&s) || s.contains(&name_l) {
            Some((70, None))
        } else {
            None
        };
        let Some((score, prefer)) = hit else { continue };
        let better = match &best_syn {
            None => true,
            Some((bi, bs, _, _)) => score > *bs || (score == *bs && idx < *bi),
        };
        if better {
            best_syn = Some((idx, score, format!("synonym:{syn}"), prefer));
        }
    }
    if let Some((si, sc, via, prefer)) = best_syn {
        return Some((sc, Some(si), via, prefer));
    }

    // 3. 包含命中
    if name_l.contains(kw) || kw.contains(&name_l) {
        return Some((60, None, "contains".into(), None));
    }

    // 4. 字符重叠 (中英混合时几乎用不上, 兜底)
    let overlap = common_chars(&g.name, kw);
    if !g.name.is_empty() && overlap as f64 / g.name.chars().count().max(1) as f64 > 0.5 {
        return Some((30, None, "overlap".into(), None));
    }

    None
}

/// 两个字符串的公共字符数 (去重计数)
fn common_chars(a: &str, b: &str) -> usize {
    let sa: std::collections::HashSet<char> = a.chars().collect();
    let sb: std::collections::HashSet<char> = b.chars().collect();
    sa.intersection(&sb).count()
}

// ============================================================
// 中英同义词词典 (英文 LLM keyword → 中文贴纸名词候选)
// ============================================================

/// 英文词干剥离: 去掉 -ing/-s/-ed/-es 后缀 + 双辅音折叠 (chatting→chat, running→run)
/// 仅 ASCII, 用于 chat/chatting 类变体匹配
fn stem(word: &str) -> &str {
    if !word.is_ascii() {
        return word;
    }
    let mut result = word;
    for suffix in ["ing", "es", "ed", "s"] {
        if let Some(stripped) = result.strip_suffix(suffix) {
            if stripped.len() >= 3 {
                result = stripped;
                // 双辅音折叠: chatting→chatt→chat, running→runn→run
                let chars: Vec<char> = result.chars().collect();
                if chars.len() >= 3
                    && chars[chars.len() - 1] == chars[chars.len() - 2]
                    && chars[chars.len() - 1].is_ascii_alphabetic()
                    && !"aeiou".contains(chars[chars.len() - 1])
                {
                    result = &result[..result.len() - 1];
                }
                break;
            }
        }
    }
    result
}

/// 概念兜底词典: 常见英文概念词 → 中文贴纸候选
/// (词典未覆盖的关键词, 不依赖 stickers.json tags; 优先级低于 SYNONYMS)
const CONCEPT_MAP: &[(&str, &[&str])] = &[
    ("chat", &["说话"]),
    ("talk", &["说话"]),
    ("speak", &["说话"]),
    ("say", &["说话"]),
    ("discuss", &["说话"]),
    ("converse", &["说话"]),
    ("sigh", &["呆"]),
    ("exhale", &["呆"]),
    ("bath", &["汗", "喝"]),
    ("wash", &["汗"]),
    ("phone", &["通知", "扩音器"]),
    ("notification", &["通知"]),
    ("message", &["通知"]),
    ("bored", &["呆", "睡觉"]),
    ("silent", &["静音"]),
    ("quiet", &["静音"]),
    ("song", &["唱歌", "吉他"]),
    ("sing", &["唱歌", "吉他"]),
    ("dance", &["跳舞", "舞蹈"]),
    ("walk", &["走路", "脚步"]),
    ("run", &["跑"]),
    ("angry", &["生气", "恼怒"]),
    ("mad", &["生气"]),
    ("sad", &["哭", "难过"]),
    ("cry", &["哭"]),
    ("laugh", &["笑"]),
    ("smile", &["笑"]),
    ("shock", &["惊吓", "呆"]),
    ("scared", &["害怕", "惊吓"]),
    ("fear", &["害怕"]),
    ("happy", &["庆祝", "加油", "干杯"]),
    ("excited", &["期待", "跳舞"]),
    ("love", &["爱心", "玫瑰"]),
    ("gift", &["礼物", "红包"]),
    ("money", &["钱", "刷卡"]),
    ("food", &["吃", "馋", "蛋糕"]),
    ("eat", &["吃", "馋"]),
    ("drink", &["喝", "干杯"]),
    ("sleep", &["睡觉", "催眠"]),
    ("tired", &["睡觉", "困"]),
    ("work", &["工作", "打字"]),
    ("type", &["打字"]),
    ("computer", &["打字", "工作"]),
    ("game", &["打游戏"]),
    ("play", &["打游戏", "跳舞"]),
    ("music", &["唱歌", "吉他"]),
    ("guitar", &["吉他"]),
    ("magic", &["魔法"]),
    ("spell", &["魔法"]),
    ("fire", &["烧"]),
    ("hot", &["烧", "汗"]),
    ("cold", &["冰", "冷"]),
    ("snow", &["冰"]),
    ("rain", &["汗"]),
    ("water", &["喝"]),
    ("swim", &["喝"]),
    ("cook", &["吃"]),
    ("kitchen", &["吃"]),
    ("door", &["到达"]),
    ("open", &["到达"]),
    ("hello", &["打招呼"]),
    ("hi", &["打招呼"]),
    ("bye", &["打招呼"]),
    ("wave", &["打招呼"]),
    ("clap", &["鼓掌"]),
    ("applause", &["鼓掌"]),
    ("point", &["指"]),
    ("button", &["按钮"]),
    ("push", &["按钮"]),
    ("ok", &["一切都好", "点赞"]),
    ("yes", &["点赞", "一切都好"]),
    ("no", &["摇头"]),
    ("think", &["思考", "主意"]),
    ("idea", &["主意"]),
    ("question", &["问号"]),
    ("confused", &["问号", "呆"]),
    ("dizzy", &["头晕"]),
    ("nervous", &["紧张", "害羞"]),
    ("shy", &["害羞"]),
    ("cute", &["可爱", "害羞"]),
    ("heart", &["爱心"]),
    ("kiss", &["舔舔"]),
    ("pet", &["摸头", "舔舔"]),
    ("head", &["摸头", "敲头"]),
    ("hit", &["被击中"]),
    ("attack", &["被击中"]),
    ("gun", &["枪"]),
    ("knife", &["刀"]),
    ("sword", &["刀"]),
    ("kill", &["死亡"]),
    ("die", &["死亡"]),
    ("ghost", &["惊吓", "害怕"]),
    ("monster", &["害怕"]),
    ("pray", &["自我安慰"]),
    ("comfort", &["自我安慰", "摸头"]),
    ("record", &["记录", "抓拍"]),
    ("photo", &["抓拍"]),
    ("camera", &["抓拍"]),
    ("bell", &["摇铃"]),
    ("shake", &["摇铃"]),
    ("cola", &["摇可乐"]),
    ("celebration", &["庆祝", "干杯"]),
    ("party", &["庆祝", "荧光棒"]),
    ("cheer", &["加油", "荧光棒"]),
    ("raise", &["荧光棒", "举手"]),
    ("trash", &["垃圾桶"]),
    ("fly", &["拍蝇"]),
    ("spray", &["喷剂"]),
    ("tape", &["胶带"]),
    ("board", &["画板"]),
    ("draw", &["画板"]),
    ("egg", &["复活节", "蛋糕"]),
    ("prison", &["坐牢"]),
    ("jail", &["坐牢"]),
];

/// 由 keyword 查同义词候选 (已归一化小写)。返回 (全局词序索引, 词)。
/// 全局词序 = SYNONYMS 展平后的顺序, 同条目内词序即优先级 (love → 爱心 先于 情书)。
/// 词干变体 (chatting→chat) 与 CONCEPT_MAP 兜底排在 SYNONYMS 之后 (低优先)。
fn synonyms(kw: &str) -> Vec<(usize, &'static str)> {
    let mut global = 0usize;
    let mut out = Vec::new();
    for (k, v) in SYNONYMS {
        for s in *v {
            if *k == kw {
                out.push((global, *s));
            }
            global += 1;
        }
    }
    // 词干变体: 同一词典再查一次 (chatting → chat)
    let kw_stem = stem(kw);
    if kw_stem != kw && !kw_stem.is_empty() {
        for (k, v) in SYNONYMS {
            for s in *v {
                if *k == kw_stem {
                    out.push((global, *s));
                }
                global += 1;
            }
        }
    }
    // CONCEPT_MAP 兜底 (原词 + 词干)
    let mut concept_global = global;
    for (k, v) in CONCEPT_MAP {
        for s in *v {
            if *k == kw || *k == kw_stem {
                out.push((concept_global, *s));
            }
            concept_global += 1;
        }
    }
    out
}

static SYNONYMS: &[(&str, &[&str])] = &[
    ("love", &["爱心", "喜欢", "情书", "玫瑰"]),
    ("heart", &["爱心", "情书"]),
    ("laugh", &["笑", "小丑", "搞笑"]),
    ("happy", &["庆祝", "加油", "干杯", "一切都好", "期待", "开心"]),
    ("cheer", &["加油", "庆祝", "干杯", "荧光棒"]),
    ("excited", &["期待", "跳舞", "荧光棒", "开心"]),
    ("cry", &["哭", "流泪"]),
    ("sad", &["哭", "难过", "低落"]),
    ("angry", &["生气", "恼怒", "愤怒"]),
    ("mad", &["生气", "恼怒"]),
    ("sleep", &["睡觉", "催眠", "困"]),
    ("sleepy", &["睡觉", "催眠", "工作(小睡)"]),
    ("tired", &["工作(疲倦)", "睡觉", "困"]),
    ("dream", &["睡觉", "催眠", "困"]),
    ("nightmare", &["惊吓", "害怕", "死亡"]),
    ("relief", &["一切都好", "笑"]),
    ("calm", &["一切都好"]),
    ("bored", &["呆", "睡觉", "困"]),
    ("waiting", &["呆", "到达", "期待"]),
    ("wait", &["呆", "到达"]),
    ("panic", &["惊吓", "害怕", "叹号"]),
    ("wow", &["呆", "叹号", "惊吓"]),
    ("amazing", &["呆", "叹号"]),
    ("eat", &["吃", "馋", "蛋糕"]),
    ("eating", &["吃", "馋", "蛋糕"]),
    ("drink", &["喝", "干杯", "饮料"]),
    ("hungry", &["吃", "馋", "舔舔"]),
    ("game", &["打游戏", "游戏", "电风扇"]),
    ("gaming", &["打游戏", "游戏"]),
    ("play", &["打游戏", "跳舞", "摇铃"]),
    ("hit", &["被击中", "敲头", "拍蝇"]),
    ("attack", &["被击中", "枪", "刀"]),
    ("shock", &["惊吓", "害怕", "呆", "叹号"]),
    ("surprise", &["惊吓", "惊讶", "叹号"]),
    ("scared", &["害怕", "惊吓", "紧张"]),
    ("fear", &["害怕", "惊吓", "紧张"]),
    ("afraid", &["害怕", "惊吓"]),
    ("nervous", &["紧张", "害羞", "害怕"]),
    ("dance", &["跳舞", "舞蹈"]),
    ("sing", &["唱歌", "吉他"]),
    ("music", &["唱歌", "吉他", "扩音器"]),
    ("song", &["唱歌", "音乐"]),
    ("guitar", &["吉他", "唱歌"]),
    ("gift", &["礼物", "红包", "蛋糕"]),
    ("present", &["礼物", "红包"]),
    ("money", &["钱", "拿走我的钱", "刷卡", "要米"]),
    ("cash", &["钱", "刷卡"]),
    ("hello", &["打招呼", "到达"]),
    ("hi", &["打招呼", "到达"]),
    ("bye", &["打招呼", "挥手", "到达"]),
    ("welcome", &["打招呼", "干杯"]),
    ("yes", &["点赞", "点头", "一切都好"]),
    ("ok", &["点赞", "一切都好", "点头"]),
    ("good", &["一切都好", "点赞", "加油"]),
    ("no", &["摇头", "反向点赞"]),
    ("nope", &["摇头", "反向点赞"]),
    ("think", &["思考", "主意", "问号"]),
    ("idea", &["主意", "思考", "叹号"]),
    ("question", &["问号", "思考"]),
    ("confused", &["问号", "呆", "头晕"]),
    ("dizzy", &["头晕", "呆"]),
    ("work", &["工作", "打字", "带薪拉屎"]),
    ("typing", &["打字", "工作"]),
    ("dead", &["死亡", "停止工作"]),
    ("die", &["死亡", "停止工作"]),
    ("kill", &["死亡", "枪", "刀"]),
    ("weapon", &["枪", "刀", "撬棍", "喷剂"]),
    ("gun", &["枪", "刀"]),
    ("knife", &["刀", "枪"]),
    ("magic", &["魔法", "复活节"]),
    ("cool", &["墨镜反光", "墨镜循环", "像素墨镜反光", "摘掉墨镜"]),
    ("sunglasses", &["墨镜反光", "墨镜循环", "像素墨镜反光", "摘掉墨镜"]),
    ("hot", &["烧", "汗"]),
    ("sweat", &["汗", "烧"]),
    ("shy", &["害羞", "害羞"]),
    ("cute", &["害羞", "可爱", "爱心"]),
    ("kiss", &["舔舔", "情书"]),
    ("record", &["记录", "抓拍"]),
    ("camera", &["抓拍", "记录"]),
    ("photo", &["抓拍", "记录"]),
    ("trash", &["垃圾桶", "停止工作"]),
    ("bug", &["bug", "bug"]),
    ("attention", &["通知", "叹号", "扩音器"]),
    ("notice", &["通知", "提示"]),
    ("coin", &["硬币", "得分"]),
    ("score", &["得分", "记录"]),
    ("sticker", &["贴纸"]),
    ("point", &["指", "荧光棒"]),
    ("raise", &["荧光棒", "加油", "举手"]),
    ("clap", &["鼓掌", "拍手"]),
    ("wave", &["打招呼", "挥手", "荧光棒"]),
    ("hand", &["指", "摸头", "抓拍"]),
    ("head", &["摸头", "敲头", "摇头"]),
    ("sleep_prep", &["准备阶段1", "准备阶段2"]),
    ("smile", &["笑", "一切都好"]),
    ("wink", &["眨眼", "笑"]),
    ("shut", &["静音", "停止工作"]),
    ("mute", &["静音", "扩音器"]),
    ("loud", &["扩音器", "唱歌"]),
    ("bubble", &["冒泡", "思考"]),
    ("pet", &["摸头", "舔舔"]),
    ("push", &["按钮", "按"]),
    ("button", &["按钮"]),
    ("egg", &["复活节", "蛋糕"]),
    ("easter", &["复活节"]),
    ("jail", &["坐牢"]),
    ("prison", &["坐牢"]),
    ("flip", &["反转", "反转"]),
    ("reverse", &["反转", "反向点赞"]),
    ("crown", &["小丑", "庆祝"]),
    ("clown", &["小丑"]),
    ("shower", &["洗澡", "汗"]),
    ("rain", &["下雨", "汗"]),
    ("sunglasses_pop", &["像素墨镜反光", "墨镜反光"]),
    ("shake", &["摇可乐", "摇铃", "摇头"]),
    ("bell", &["摇铃", "通知"]),
    ("cola", &["摇可乐", "喝"]),
    ("drive", &["驾驶", "到达"]),
    ("car", &["驾驶", "到达"]),
    ("tape", &["胶带", "胶带"]),
    ("board", &["画板", "记录"]),
    ("draw", &["画板", "记录"]),
    ("cheese", &["奶酪糊脸", "吃"]),
    ("fly", &["拍蝇", "到达"]),
    ("spray", &["喷剂", "喷"]),
    ("mop", &["拖把", "清洁"]),
    ("self", &["自我安慰", "自我"]),
    ("comfort", &["自我安慰", "摸头"]),
    ("destiny", &["命运", "六七"]),
    ("six", &["六七"]),
    ("glow", &["荧光棒", "冒泡"]),
    ("fan", &["电风扇", "摇扇"]),
    ("swing", &["折扇", "摇扇"]),
    ("explosion", &["爆炸", "叹号"]),
    ("boom", &["爆炸", "叹号", "烧"]),
    ("fire", &["烧", "叹号", "爆炸"]),
    ("burn", &["烧", "死亡"]),
    ("melt", &["烧", "融化"]),
    ("ice", &["冰", "冷"]),
    ("cold", &["冷", "冰"]),
    ("alien", &["外星人", "小丑"]),
    ("ufo", &["外星人", "飞行"]),
    ("tea", &["喝茶", "喝"]),
    ("coffee", &["咖啡", "喝"]),
];

// ============================================================
// stickers.json 清单 (可选)
// ============================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StickerMeta {
    pub name: String,
    /// 组内文件 (可选, 空则按 name 自动分组)
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

fn load_sticker_meta(root: &Path) -> Vec<StickerMeta> {
    let p = root.join("stickers.json");
    let raw = match std::fs::read_to_string(&p) {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn group_names(idx: &StickerIndex) -> Vec<String> {
        idx.groups().iter().map(|g| g.name.clone()).collect()
    }

    /// 构造临时贴纸目录
    fn temp_sticker_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "爱心.gif", "爱心_1.gif", "爱心_2.gif",
            "呆.gif", "呆_1.gif",
            "呆(贴纸).gif", "呆(贴纸)_1.gif",
            "被击中(爱心).gif",
            "被击中(拖鞋).gif",
            "打字(恼怒).gif", "打字(普通).gif", "打字(生气).gif",
            "睡觉.gif", "睡觉_1.gif",
            "问号.gif",
            "说话.gif",
            "Bug.gif",
        ] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        let p = dir.path().to_path_buf();
        (dir, p)
    }

    #[test]
    fn groups_by_name() {
        let (_d, root) = temp_sticker_dir();
        let idx = StickerIndex::scan(&root);
        let names = group_names(&idx);
        assert!(names.contains(&"爱心".to_string()));
        assert!(names.contains(&"呆".to_string()));
        assert!(names.contains(&"被击中".to_string()));
        assert!(names.contains(&"打字".to_string()));
        assert!(names.contains(&"Bug".to_string()));
        // 爱心 3 帧 → 1 组 3 文件 (基础变体)
        let love = names.iter().position(|n| n == "爱心").unwrap();
        assert_eq!(idx.groups()[love].all_files().len(), 3);
        // 呆 与 呆(贴纸) 合并: 基础 2 帧 + 贴纸变体 2 帧
        let dai = names.iter().position(|n| n == "呆").unwrap();
        assert_eq!(idx.groups()[dai].all_files().len(), 4);
        assert_eq!(idx.groups()[dai].variant_names(), vec!["", "贴纸"]);
        // 打字 3 变体 (无基础文件, 无 "" 变体)
        let typing = names.iter().position(|n| n == "打字").unwrap();
        assert_eq!(idx.groups()[typing].variant_names(), vec!["恼怒", "普通", "生气"]);
        // 被击中 2 变体 (爱心/拖鞋)
        let hit = names.iter().position(|n| n == "被击中").unwrap();
        assert_eq!(idx.groups()[hit].variant_names(), vec!["拖鞋", "爱心"]);
    }

    #[test]
    fn match_scoring() {
        let (_d, root) = temp_sticker_dir();
        let idx = StickerIndex::scan(&root);

        // 主名精确命中
        let m = idx.match_keyword("爱心").unwrap();
        assert_eq!(m.score, 100);
        assert_eq!(idx.groups()[m.group_index].name, "爱心");
        // 变体精确命中 → 95 + preferred_variant (name 命中优先于 variant 命中)
        let m = idx.match_keyword("生气").unwrap();
        assert_eq!(m.score, 95);
        assert_eq!(idx.groups()[m.group_index].name, "打字");
        assert_eq!(m.preferred_variant.as_deref(), Some("生气"));
        // name 命中优先于 variant 命中: 呆 → 呆组 (不是 六七(呆) 变体)
        let m = idx.match_keyword("呆").unwrap();
        assert_eq!(m.score, 100);
        assert_eq!(idx.groups()[m.group_index].name, "呆");
        // 同义词: love → 爱心 (90)
        let m = idx.match_keyword("love").unwrap();
        assert_eq!(m.score, 90);
        assert_eq!(idx.groups()[m.group_index].name, "爱心");
        // 同义词命中变体: angry → 生气 → 打字(生气) (85 + prefer)
        let m = idx.match_keyword("angry").unwrap();
        assert_eq!(m.score, 85);
        assert_eq!(m.preferred_variant.as_deref(), Some("生气"));
        // 英文直接命中: bug → Bug
        let m = idx.match_keyword("bug").unwrap();
        assert_eq!(m.score, 100);
        // 新同义词: dream → 睡觉
        let m = idx.match_keyword("dream").unwrap();
        assert_eq!(idx.groups()[m.group_index].name, "睡觉");
        // question → 问号 (词典顺序: 问号 先于 思考)
        let m = idx.match_keyword("question").unwrap();
        assert_eq!(idx.groups()[m.group_index].name, "问号");
        // 概念兜底: chat → 说话, sigh → 呆
        let m = idx.match_keyword("chat").unwrap();
        assert_eq!(idx.groups()[m.group_index].name, "说话");
        let m = idx.match_keyword("sigh").unwrap();
        assert_eq!(idx.groups()[m.group_index].name, "呆");
        // 词干变体: chatting → chat → 说话
        let m = idx.match_keyword("chatting").unwrap();
        assert_eq!(idx.groups()[m.group_index].name, "说话");
        // 无命中
        assert!(idx.match_keyword("zzz_nothing").is_none());
    }

    #[test]
    fn pick_variant_prefers_variant() {
        let (_d, root) = temp_sticker_dir();
        let idx = StickerIndex::scan(&root);
        let dai = idx.groups().iter().position(|g| g.name == "呆").unwrap();
        // prefer=贴纸 → 选 呆(贴纸) 帧
        let s = idx.pick_variant(dai, 1.234, Some("贴纸")).unwrap();
        assert!(s.starts_with("呆(贴纸)"), "got {}", s);
        // 无 prefer → 基础变体帧 (呆/呆_1)
        let s2 = idx.pick_variant(dai, 1.234, None).unwrap();
        assert!(s2 == "呆" || s2 == "呆_1", "got {}", s2);
        // 确定性
        assert_eq!(idx.pick_variant(dai, 1.234, Some("贴纸")).unwrap(), s);
    }

    #[test]
    fn frame_suffix_stripped() {
        assert_eq!(strip_frame_suffix("爱心_1"), "爱心");
        assert_eq!(strip_frame_suffix("呆(贴纸)_1"), "呆(贴纸)");
        assert_eq!(strip_frame_suffix("通知_提示_爱心_喜欢"), "通知_提示_爱心_喜欢");
        assert_eq!(strip_frame_suffix("爱心"), "爱心");
    }

    #[test]
    fn parse_filename_breaks_paren() {
        assert_eq!(parse_filename("呆(贴纸)_1.gif"), Some(("呆".into(), "贴纸".into())));
        assert_eq!(parse_filename("爱心_1.gif"), Some(("爱心".into(), "".into())));
        assert_eq!(parse_filename("被击中(爱心).gif"), Some(("被击中".into(), "爱心".into())));
        assert_eq!(parse_filename("打字(恼怒).gif"), Some(("打字".into(), "恼怒".into())));
        assert_eq!(parse_filename("通知_提示_爱心_喜欢.gif"), Some(("通知_提示_爱心_喜欢".into(), "".into())));
        assert_eq!(parse_filename("睡觉_(UU).gif"), Some(("睡觉".into(), "UU".into())));
    }

    #[test]
    fn scan_packages_finds_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ysyk")).unwrap();
        std::fs::write(dir.path().join("ysyk").join("害怕_1.gif"), b"x").unwrap();
        std::fs::write(dir.path().join("ysyk").join("笑.gif"), b"x").unwrap();
        // 根目录直接放的文件不应进入 packages
        std::fs::write(dir.path().join("直接.gif"), b"x").unwrap();
        // 空目录忽略
        std::fs::create_dir_all(dir.path().join("empty")).unwrap();

        let packages = scan_packages(dir.path());
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "ysyk");
        assert_eq!(packages[0].file_count(), 2);
        assert!(packages[0].files.iter().all(|p| p.parent().unwrap().file_name().unwrap() == "ysyk"));
    }

    #[test]
    fn pick_package_file_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ysyk")).unwrap();
        for n in ["a.gif", "b.gif", "c.gif"] {
            std::fs::write(dir.path().join("ysyk").join(n), b"x").unwrap();
        }
        let packages = scan_packages(dir.path());
        let stem1 = pick_package_file(&packages, "ysyk", 0.123).unwrap();
        let stem2 = pick_package_file(&packages, "ysyk", 0.123).unwrap();
        assert_eq!(stem1, stem2);
        assert!(["a", "b", "c"].contains(&stem1.as_str()));
        assert!(pick_package_file(&packages, "nonexistent", 0.5).is_none());
    }

    #[test]
    fn local_sticker_candidates_text_lists_packages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ysyk")).unwrap();
        std::fs::write(dir.path().join("ysyk").join("a.gif"), b"x").unwrap();
        let text = local_sticker_candidates_text(dir.path());
        assert!(text.contains("ysyk(1个)"));
        assert!(text.contains("只能从以下包名选择"));
    }

    #[test]
    fn build_package_meta_from_tags_aggregates() {
        let file_tags = vec![
            ("害怕_1.gif".to_string(), vec!["害怕".to_string(), "恐惧".to_string()]),
            ("害怕_2.gif".to_string(), vec!["害怕".to_string()]),
            ("笑.gif".to_string(), vec!["笑".to_string(), "开心".to_string()]),
        ];
        let meta = build_package_meta_from_tags("ysyk", &file_tags).unwrap();
        assert_eq!(meta.name, "ysyk");
        assert_eq!(meta.tags.len(), 4);
        assert_eq!(tag_files(&meta, "害怕"), vec!["害怕_1.gif", "害怕_2.gif"]);
        assert_eq!(tag_files(&meta, "笑"), vec!["笑.gif"]);
        assert_eq!(tag_files(&meta, "开心"), vec!["笑.gif"]);
    }

    #[test]
    fn package_meta_toml_roundtrip() {
        let file_tags = vec![("a.gif".to_string(), vec!["笑".to_string()])];
        let meta = build_package_meta_from_tags("ysyk", &file_tags).unwrap();
        let toml = package_meta_to_toml(&meta).unwrap();
        let parsed: StickerPackageMeta = toml::from_str(&toml).unwrap();
        assert_eq!(parsed.name, "ysyk");
        assert_eq!(parsed.tags[0].tag, "笑");
        assert_eq!(parsed.tags[0].files, vec!["a.gif"]);
    }

    #[test]
    fn sticker_index_register_and_load() {
        let dir = tempfile::tempdir().unwrap();
        register_package(dir.path(), "ysyk").unwrap();
        register_package(dir.path(), "komori").unwrap();
        register_package(dir.path(), "ysyk").unwrap(); // 幂等
        let idx = load_sticker_index(dir.path());
        assert_eq!(idx.packages, vec!["komori", "ysyk"]);
    }

    #[test]
    fn image_extension_check_is_the_shared_judgement() {
        for ok in ["a.gif", "b.PNG", "c.jpg", "d.jpeg", "e.webp", "x.GIF"] {
            assert!(is_image_file_name(ok), "{ok}");
        }
        for no in ["a.txt", "b", "无扩展名", ".DS_Store", "", "a.gif.bak"] {
            assert!(!is_image_file_name(no), "{no}");
        }
        // 路径版只看文件名那一段（调用方给的是包里的条目路径）
        assert!(is_image_path(Path::new("dir/sub/x.webp")));
        assert!(!is_image_path(Path::new("dir/sub")));
    }

    #[test]
    fn sanitize_pack_name_accepts_plain_names_and_rejects_paths() {
        assert_eq!(sanitize_pack_name("ysyk"), Some("ysyk".into()));
        assert_eq!(sanitize_pack_name("  komori_2 "), Some("komori_2".into()));
        assert_eq!(sanitize_pack_name("表情包-2"), Some("表情包-2".into()));
        // 路径与保留名：包名会变成目录名 + URL 路径段，这两种都必须挡住
        assert_eq!(sanitize_pack_name(""), None);
        assert_eq!(sanitize_pack_name("   "), None);
        assert_eq!(sanitize_pack_name(".."), None);
        assert_eq!(sanitize_pack_name("../x"), None);
        assert_eq!(sanitize_pack_name("a/b"), None);
        assert_eq!(sanitize_pack_name("a\\b"), None);
        assert_eq!(sanitize_pack_name("C:pack"), None);
        assert_eq!(sanitize_pack_name(".hidden"), None);
        assert_eq!(sanitize_pack_name("has space"), None);
        assert_eq!(sanitize_pack_name("con"), None); // Windows 保留设备名
        assert_eq!(sanitize_pack_name(&"a".repeat(33)), None);
        assert_eq!(sanitize_pack_name(&"a".repeat(32)).unwrap().len(), 32);
    }

    #[test]
    fn safe_component_rejects_separators() {
        assert!(is_safe_component("害怕_1.gif"));
        assert!(is_safe_component("呆(贴纸)_2.GIF"));
        assert!(!is_safe_component(""));
        assert!(!is_safe_component(".."));
        assert!(!is_safe_component("../a.gif"));
        assert!(!is_safe_component("a\\b.gif"));
        assert!(!is_safe_component("C:/x.gif"));
        assert!(!is_safe_component("nul"));
    }

    /// 造一个来源目录：3 张图 + 1 个 txt + 1 个子目录。
    fn temp_source_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for name in ["害怕_1.gif", "笑.png", "呆.webp"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        std::fs::write(dir.path().join("readme.txt"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        dir
    }

    #[test]
    fn import_package_copies_images_and_skips_others() {
        let root = tempfile::tempdir().unwrap();
        let src = temp_source_dir();
        let r = import_package(root.path(), "meme", src.path()).unwrap();
        assert_eq!(r.copied, 3);
        assert_eq!(r.skipped, 2, "readme.txt 与 sub/ 都算被跳过");
        assert_eq!(
            list_package_files(&root.path().join("meme")),
            vec!["呆.webp", "害怕_1.gif", "笑.png"]
        );
        // 来源目录原样留着（复制而非搬移）
        assert!(src.path().join("害怕_1.gif").exists());
        // 未构建的包不进全局索引
        assert!(load_sticker_index(root.path()).packages.is_empty());
    }

    #[test]
    fn import_package_refuses_existing_target_and_source_inside_root() {
        let root = tempfile::tempdir().unwrap();
        let src = temp_source_dir();
        std::fs::create_dir(root.path().join("meme")).unwrap();

        let e = import_package(root.path(), "meme", src.path()).unwrap_err();
        assert!(e.to_string().contains("已存在"), "{e}");

        // 来源就在库内 → 拒绝（否则会把整个库往自己身上拷）
        let inside = root.path().join("ysyk");
        std::fs::create_dir(&inside).unwrap();
        std::fs::write(inside.join("a.gif"), b"x").unwrap();
        let e = import_package(root.path(), "copy", &inside).unwrap_err();
        assert!(e.to_string().contains("不能导入自己"), "{e}");

        assert!(!root.path().join("copy").exists());
    }

    #[test]
    fn import_package_rolls_back_when_no_image() {
        let root = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("readme.txt"), b"x").unwrap();

        let e = import_package(root.path(), "empty", src.path()).unwrap_err();
        assert!(e.to_string().contains("没有图片"), "{e}");
        // 不留半个空包
        assert!(!root.path().join("empty").exists());
    }

    #[test]
    fn file_tags_map_inverts_package_meta() {
        let root = tempfile::tempdir().unwrap();
        let meta = build_package_meta_from_tags(
            "ysyk",
            &[
                ("笑.gif".to_string(), vec!["笑".to_string(), "开心".to_string()]),
                ("害怕_1.gif".to_string(), vec!["害怕".to_string()]),
            ],
        )
        .unwrap();
        std::fs::write(
            root.path().join("ysyk.toml"),
            package_meta_to_toml(&meta).unwrap(),
        )
        .unwrap();

        let map = file_tags_map(root.path(), "ysyk");
        let mut laugh = map.get("笑.gif").cloned().unwrap();
        laugh.sort();
        assert_eq!(laugh, vec!["开心", "笑"]);
        assert_eq!(map.get("害怕_1.gif").unwrap(), &vec!["害怕".to_string()]);
        // 未构建的包 / 不存在的包 → 空表，不报错
        assert!(file_tags_map(root.path(), "nope").is_empty());
    }

    #[test]
    fn remove_package_clears_dir_meta_and_index() {
        let root = tempfile::tempdir().unwrap();
        let src = temp_source_dir();
        import_package(root.path(), "meme", src.path()).unwrap();
        register_package(root.path(), "meme").unwrap();
        std::fs::write(root.path().join("meme.toml"), "name = \"meme\"\n").unwrap();

        assert!(remove_package(root.path(), "meme").unwrap());
        assert!(!root.path().join("meme").exists());
        assert!(!root.path().join("meme.toml").exists());
        assert!(load_sticker_index(root.path()).packages.is_empty());

        // 幂等：再删一次不报错，也没目录可删
        assert!(!remove_package(root.path(), "meme").unwrap());
    }
}
