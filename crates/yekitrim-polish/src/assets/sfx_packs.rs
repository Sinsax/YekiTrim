//! 用户音效包：`<data>/users/<uid>/sfx/<包名>/` 下的音频文件集合。
//!
//! # 与内建预设包的关系
//!
//! 音效库有**两种包**，素材库音效页把它们排在同一张卡片网格里：
//!
//! | | 预设包（内建） | 用户包（这里） |
//! |---|---|---|
//! | 来源 | 编译期内嵌 `assets/sfx_catalog.json`（1213 组） | 用户上传的 zip |
//! | 落点 | 不在磁盘上 | `<data>/users/<uid>/sfx/<包名>/` |
//! | 可删 | ❌（`is_reserved_pack_name` 挡同名磁盘包） | ✅ |
//! | 包名 | 7 个大类（`cat`） | 用户填的（`sanitize_pack_name`） |
//!
//! # ⚠️ 用户包是**扁平的**
//!
//! 目录结构刻意只有一层：`<包名>/<扁平化后的文件名>.mp3`。原因与 `variant_stem`
//! 相同（见 `sfx_catalog`）：出片侧按 `sfx/<name>.mp3` 找文件，而 `collect_sound_events`
//! 拼的是 `format!("sfx/{}.mp3", name)` —— 子目录会让它**只 warn 一句「文件缺失」**，
//! 成片少一声且不报错。
//!
//! 所以 zip 里的子目录名**只用于分组**（进 `group` 字段），文件本身一律拍平到包根；
//! 拍平后重名（不同子目录里的同名文件）**只留第一个**，其余计入 `skipped`。
//!
//! # 安全
//!
//! 这是本仓**第三处**有意接收用户上传的入口（另两处：贴纸 zip、封面背景图 base64）。
//! 闸门与贴纸那条**同一套**（`sticker_zip::ZipLimits`），所以这里直接复用它的解压实现，
//! 只换掉"哪些条目算素材"这一条判据（图片 → 音频）。

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::assets::sticker_index::{is_safe_component, sanitize_pack_name};
use crate::assets::sticker_zip::ZipLimits;

/// 用户音效包的根目录（调用方拼：`path::user_home()` 下）。
pub const SFX_PACK_DIR: &str = "sfx";

/// `pack.toml` 的大小上限（64 KiB）。它是一份索引，不是素材 —— 超了当没有。
const MANIFEST_MAX_BYTES: u64 = 64 * 1024;

/// 音频扩展名 —— **唯一**一份判据（上传、扫描、列表、试听四处都走它）。
///
/// ⚠️ 与 `sticker_index::is_image_file_name` 同一个道理：别在各处再抄一份扩展名清单，
/// 抄了就会漂（症状是"这个文件明明在包里，列表里却没有"）。
pub fn is_audio_file_name(name: &str) -> bool {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    matches!(ext.as_str(), "mp3" | "wav" | "ogg" | "m4a" | "flac" | "aac")
}

/// **变体键**：用户包里的文件 → 扁平化后的单文件名（不含扩展名）。
///
/// 与 `sfx_catalog::variant_stem` **同一套规则**（去扩展名、分隔符换成 `-`）：
/// 出片侧只认 `sfx/<key>.mp3`，两套规则不一致就等于"导入的包出片时没声音"。
/// ⚠️ 唯一的区别是这里要先去扩展名——`variant_stem` 只去 `.mp3`，
/// 而用户包可能给 `.wav` / `.ogg`。
pub fn stem_of(file_name: &str) -> String {
    let stem = file_name.rsplit_once('.').map(|(s, _)| s).unwrap_or(file_name);
    stem.replace(['/', '\\'], "-")
}

/// 预设包名去掉序号前缀（`① 主播情绪反应` → `主播情绪反应`）。
///
/// 与前端 `String(cat).replace(/^[①②③④⑤⑥⑦]\s*/, '')` **同一条规则**：
/// 包名要能过 `sanitize_pack_name`，所以保留名比较也只认去掉序号后的形态。
pub fn category_bare_name(cat: &str) -> &str {
    let s = cat.trim_start_matches(|c: char| "①②③④⑤⑥⑦".contains(c));
    s.trim()
}

/// 扫描一个用户包目录：返回 **`(变体键, 显示名, 细分组)`** 三元组，按文件名排序。
///
/// `group` 来自 zip 里的子目录名（已拍平进文件名的那部分反查不回来），
/// 所以这里读的是包目录下的清单 —— 见 [`load_manifest`] 与 [`pack_groups_file`]。
pub fn scan_pack(dir: &Path) -> Vec<PackEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let manifest = load_manifest(dir);
    let groups = load_groups(dir);
    let mut out: Vec<PackEntry> = entries
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let file_name = e.file_name().to_str()?.to_string();
            if file_name.starts_with('.') || !is_audio_file_name(&file_name) {
                return None;
            }
            let stem = stem_of(&file_name);
            if !is_safe_component(&stem) {
                return None;
            }
            let m = manifest.as_ref().and_then(|x| x.entry(&stem));
            let group = m
                .and_then(|x| x.group.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| groups.get(stem.as_str()).cloned())
                .unwrap_or_else(|| crate::assets::sfx_catalog::UNCATEGORIZED_GROUP.to_string());
            Some(PackEntry {
                // 显示名优先级：清单 `name` > 文件名去扩展名（用户自己起的名字，
                // 比生成的 id 有信息量）。清单缺了就退回现状，行为不变。
                label: m
                    .and_then(|x| x.name.as_deref())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        file_name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&file_name).to_string()
                    }),
                tags: m.map(|x| x.tags.clone()).unwrap_or_default(),
                gain_db: m.and_then(|x| x.gain_db),
                stem,
                group,
                file_name,
            })
        })
        .collect();
    out.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    out
}

/// 用户包里的一个音效变体。
#[derive(Debug, Clone)]
pub struct PackEntry {
    /// 变体键（出片侧、试听 URL 用它）
    pub stem: String,
    /// 显示名（清单 `name` > 原文件名去扩展名）
    pub label: String,
    /// 细分组（清单 `group` > zip 子目录名 > 「其他」）
    pub group: String,
    /// 磁盘上的真实文件名（读字节用）
    pub file_name: String,
    /// 清单里的语义 tag（必须取自内建 84 个词；空 = 参与不了 tag 匹配）
    pub tags: Vec<String>,
    /// 清单里的增益（dB）；`None` = 不调整
    pub gain_db: Option<f64>,
}

/// 列出根目录下的**全部用户包**：`[(包名, [条目])]`，按包名排序。
///
/// 与 `scan_pack` 的区别：那个扫**一个包**，这个扫**库根下所有包**（音效页左侧、LLM 候选、
/// 出片下载都要这个视图）。
///
/// ⚠️ 空目录（导入到一半崩了）**不列**：它在界面上是个点开没内容的卡片，
/// 在 LLM 候选里是噪音，在下载时是个必然失败的引用。
pub fn scan_all(root: &Path) -> Vec<(String, Vec<PackEntry>)> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        // 暂存目录（`.upload-*`）与隐藏目录不算包
        .filter(|d| {
            d.file_name()
                .and_then(|n| n.to_str())
                .map(|n| !n.starts_with('.'))
                .unwrap_or(false)
        })
        .collect();
    dirs.sort();
    dirs.into_iter()
        .filter_map(|dir| {
            let name = dir.file_name()?.to_str()?.to_string();
            let entries = scan_pack(&dir);
            if entries.is_empty() {
                return None;
            }
            Some((name, entries))
        })
        .collect()
}

/// 单个条目的**全局 id**（`<包名>/<变体键>`）—— 事件里存的就是它。
///
/// ⚠️ 这个格式在**三处**必须一致：服务端 `user_pack_list` 下发、试听路由分流、
/// 这里出片/LLM 用。别在任何一处自己拼第二种形态。
pub fn entry_id(pack: &str, stem: &str) -> String {
    format!("{pack}/{stem}")
}

/// 把一个全局 id 拆回 `(包名, 变体键)`；不带 `/` 的（预设包的 id）返回 `None`。
///
/// 预设包的变体键**永远不含 `/`**（`variant_stem` 把分隔符拍平成 `-`），
/// 所以「带 `/`」就是用户包——这是两套 id 的分界线，判据只此一份。
pub fn split_entry_id(id: &str) -> Option<(&str, &str)> {
    id.split_once('/')
}

/// 生成用户包的 **LLM 候选文本**（与 `sfx_catalog::candidates_text` 同一段 prompt 的
/// 第二个来源）。
///
/// # 为什么必须拼进去
///
/// 以前 `candidates_text` 只遍历内建目录，用户说「导入的包 LLM 从来不用」——
/// LLM 根本**看不到**这些 id，写不出来。这条把用户包按 **包 → 组** 两行列进去。
///
/// 格式（刻意写满 `包名/变体键`，因为事件里存的就是这个完整 id）：
/// ```text
/// ## 用户导入的音效（同样只能从这里选）
/// 我的音效: 敲门声(我的音效/敲门) 小雨(我的音效/下雨)[分组: 雨]
/// ```
///
/// # ⚠️ 只列**有清单**（`pack.toml`）的包
///
/// 与贴纸 `local_sticker_candidates_text` 只列已构建的包**同一口径**：
/// 没有清单 = 没有语义 tag = LLM 拿不到任何可判断的信息，列进去只会让它瞎选。
/// 未构建的包仍然**能手选、能试听**，只是不参与自动生成。
///
/// ⚠️ 与内建候选**分开两段**：混在一起时 LLM 会拿中文 label 去匹配内建目录，
/// 而用户包的 label 是用户自己起的文件名、内建目录里没有。
pub fn candidates_text(root: &Path, used: &std::collections::HashSet<String>, limit: usize) -> String {
    let packs = scan_all(root);
    if packs.is_empty() {
        return String::new();
    }
    let mut lines = Vec::new();
    let mut total = 0usize;
    let mut unbuilt = 0usize;
    for (pack, entries) in &packs {
        // 未构建的包不进候选（见上）
        if load_manifest(&root.join(pack)).is_none() {
            unbuilt += 1;
            continue;
        }
        let mut items = Vec::new();
        for e in entries {
            if total >= limit {
                break;
            }
            let id = entry_id(pack, &e.stem);
            if used.contains(&id) || used.contains(&e.stem) {
                continue;
            }
            // 带 tag 的条目把 tag 也写出来 —— 那是 LLM 判断"这一声合不合适"的唯一依据
            let extra = if e.tags.is_empty() {
                String::new()
            } else {
                format!("[tag: {}]", e.tags.join(" "))
            };
            items.push(format!("{}({}){}", e.label, id, extra));
            total += 1;
        }
        if !items.is_empty() {
            lines.push(format!("{pack}: {}", items.join(" ")));
        }
        if total >= limit {
            break;
        }
    }
    if unbuilt > 0 {
        // ⚠️ 说出来而不是静默跳过：用户看到"我有包但 LLM 不用"时会去查，
        //    日志里这一行就是答案（界面上的「未构建」角标是同一件事的另一种呈现）。
        tracing::info!("  ⚠ {unbuilt} 个用户音效包没有清单（未构建），不进 LLM 候选");
    }
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "## 用户导入的音效（共 {total} 个，同样只能从这里选）\n\n{body}\n",
        total = total,
        body = lines.join("\n")
    )
}

/// 导入结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// 真正进包的音频条数
    pub copied: usize,
    /// 跳过的条目数（目录条目 / 非音频 / 拍平后重名 / 超限）
    pub skipped: usize,
    /// **转 mp3 失败**的条数。⚠️ 不是错误：原文件保留、出片按真实扩展名找
    /// （`render::sound::resolve_sound_file`）。
    pub transcode_failed: usize,
}

/// 分组表的文件名（`<包名>.groups.toml`，与贴纸的 `<包名>.toml` 同位置同思路）。
///
/// 内容是一行一个 `键 = "分组名"`。**不写进音频文件**：容器里没有地方放自定义元数据，
/// 而重新扫描 zip 是不可能的（只留解压后的文件）。
///
/// ⚠️ 它是**导入期派生物**（zip 子目录名 → 键），只在**没有 `pack.toml`** 时生效 ——
/// 一个事实一个写者，见 [`manifest_file`]。
pub fn pack_groups_file(pack_dir: &Path) -> PathBuf {
    pack_dir.join(".groups.toml")
}

// ============================================================
// pack.toml —— 包清单（作者写 / 构建生成 / 都可以没有）
// ============================================================

/// 清单文件名。放在**包目录里**（不是库根）。
///
/// 与贴纸的 `<包名>.toml` **不同**：那份在 `stickers/` 根下、是库级索引；音效这份
/// 要跟着包一起导出/搬迁，所以必须进包。`scan_pack` 只认音频扩展名 ⇒ 它不会被当成条目。
pub fn manifest_file(pack_dir: &Path) -> PathBuf {
    pack_dir.join("pack.toml")
}

/// 清单是谁写的 —— 只影响显示与"要不要重建"，不影响读。
pub const BUILT_BY_AUTHOR: &str = "author";
/// 由 yekitrim 的 LLM 构建写出的清单。
pub const BUILT_BY_YKT: &str = "yekitrim";

/// 包的元信息（`[pack]` 段）。全部可选。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PackMeta {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub license: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// `author` | `yekitrim`。缺省（手写的清单）当 `author` 处理。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub built_by: String,
}

/// 清单里的一个条目。`stem` 是唯一必填项。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ManifestEntry {
    /// = 文件名去扩展名（与 `stem_of` 同规则）。必须与磁盘上的文件对得上。
    pub stem: String,
    /// 显示名；缺省 = `stem`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 细分组；缺省走 `.groups.toml` → 「其他」
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// 语义 tag，**必须取自内建 84 个词**（见 `sfx_catalog::all_tags`）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// 本条增益（dB）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f64>,
}

/// `pack.toml` 的完整文件形态。
///
/// 用**数组表**（`[[entry]]`）而不是 `[entries.<键>]`：键由文件名派生、可能带空格与点，
/// 写成 TOML 裸键要转义；数组表没这个坑，也保留了作者想要的顺序。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PackManifest {
    #[serde(default)]
    pub pack: PackMeta,
    #[serde(default)]
    pub entry: Vec<ManifestEntry>,
}

impl PackManifest {
    /// 按键查一条（`scan_pack` 每条都调一次，条目数不大；线性查找足够）。
    pub fn entry(&self, stem: &str) -> Option<&ManifestEntry> {
        self.entry.iter().find(|e| e.stem == stem)
    }
    /// 清单是否由 yekitrim 构建写出（决定 UI 上要不要给「重新构建」）。
    pub fn built_by_yekitrim(&self) -> bool {
        self.pack.built_by == BUILT_BY_YKT
    }
}

/// 读包清单。**坏清单不报错、返回 `None`** —— 与"转码失败不是错误"同一条哲学：
/// 一个手滑的 TOML 不该让 200 个音效全部不可用（调用方会退回纯文件名行为）。
pub fn load_manifest(pack_dir: &Path) -> Option<PackManifest> {
    let raw = std::fs::read_to_string(manifest_file(pack_dir)).ok()?;
    match toml::from_str::<PackManifest>(&raw) {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!(
                "音效包清单解析失败（已忽略，退回文件名行为）: {} — {e}",
                manifest_file(pack_dir).display()
            );
            None
        }
    }
}

/// 序列化清单（`[[entry]]` 按 `stem` 排序，方便人读与 diff）。
pub fn manifest_to_toml(m: &PackManifest) -> anyhow::Result<String> {
    let mut sorted = m.clone();
    sorted.entry.sort_by(|a, b| a.stem.cmp(&b.stem));
    Ok(toml::to_string_pretty(&sorted)?)
}

/// 写清单（**原子替换**：先写临时文件再 rename，中途失败不会留下半个 TOML）。
pub fn save_manifest(pack_dir: &Path, m: &PackManifest) -> anyhow::Result<()> {
    let text = manifest_to_toml(m)?;
    let dst = manifest_file(pack_dir);
    let tmp = pack_dir.join(".pack.toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &dst)?;
    // 作者/构建写了清单之后，`.groups.toml` 就是**过期的派生数据**了（一个事实一个写者）
    let _ = std::fs::remove_file(pack_groups_file(pack_dir));
    Ok(())
}

/// 过滤掉词表外的 tag（返回 (保留的, 丢弃数)）。
///
/// ⚠️ **音效的 tag 是封闭词表**（内建 84 个），与贴纸"LLM 自由发明 10~20 个情绪词"
/// 完全不同：词表外的 tag 解析不到，等于静默没声音。校验**只在 Rust 这一份**。
pub fn filter_known_tags(tags: &[String]) -> (Vec<String>, usize) {
    let known: std::collections::HashSet<&str> =
        crate::assets::sfx_catalog::all_tags().into_iter().map(|(t, _)| t).collect();
    let mut kept = Vec::new();
    let mut dropped = 0usize;
    for t in tags {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        if known.contains(t) {
            if !kept.iter().any(|k: &String| k == t) {
                kept.push(t.to_string());
            }
        } else {
            dropped += 1;
        }
    }
    (kept, dropped)
}

fn load_groups(pack_dir: &Path) -> HashMap<String, String> {
    let path = pack_groups_file(pack_dir);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    // 极简解析：`键 = "值"`，忽略空行与 `#` 注释。刻意不引 toml crate —— 这个文件
    // 只有我们写、只有我们读，形状固定；用 toml 反而要为它定义一个结构体。
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let key = k.trim().to_string();
        let val = v.trim().trim_matches('"').trim_end_matches('"').to_string();
        if key.is_empty() || val.is_empty() {
            continue;
        }
        // ⚠️ 分组名会进前端 chip、也会被写回文件 —— 含引号/换行的目录名能在
        //    `.groups.toml` 里造出一行假记录（zip 作者可控 ⇒ 算输入）。
        if val.chars().any(|c| c == '"' || c.is_control()) || val.chars().count() > 64 {
            continue;
        }
        out.insert(key, val);
    }
    out
}

/// 把非 mp3 的音频**转成 mp3**（就地替换）。
///
/// # 为什么转
///
/// 出片侧已经能按真实扩展名找文件（`render::sound::resolve_sound_file`），所以转码
/// **不是必需的**。但混音那条路（`mix_sfx`）对小众容器的兼容性没验证过，而 mp3 是
/// 内建库的唯一格式 —— 转成同一格式 = 用户包与内建库走**完全同一条**解码路径。
///
/// # 失败语义
///
/// ⚠️ **转码失败不是错误**：保留原文件，出片按原扩展名找。只在报告里计一笔
/// `converted_failed`，别让一个 .ogg 让整包导入失败。
///
/// ⚠️ 同步 IO + 子进程（可能几秒/文件），调用方必须在 blocking 池里跑。
pub fn transcode_to_mp3(path: &Path) -> anyhow::Result<()> {
    if path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("mp3"))
        .unwrap_or(false)
    {
        return Ok(());
    }
    let mp3 = path.with_extension("mp3");
    let mut cmd = yekitrim_core::process::cmd("ffmpeg");
    cmd.arg("-y")
        .arg("-i")
        .arg(path)
        // 音效是短促的单声道素材：libmp3lame 128k 足够，体积也小
        .arg("-codec:a")
        .arg("libmp3lame")
        .arg("-b:a")
        .arg("128k")
        .arg(&mp3);
    crate::render::ffmpeg::Ffmpeg::run_with_timeout(
        cmd,
        "音效转 mp3",
        std::time::Duration::from_secs(60),
    )?;
    if !mp3.is_file() {
        anyhow::bail!("转码没有产出 {} ", mp3.display());
    }
    // 原文件删掉（同一个包内不留两份，否则 scan_pack 会把它们当两条）
    let _ = std::fs::remove_file(path);
    Ok(())
}

/// 把一个 zip 解压成用户音效包 `root/<包名>/`。
///
/// 返回 [`ImportReport`]。闸门由 `limits` 给（服务端按 `[sfx]` 或用贴纸那套默认）。
///
/// ⚠️ **先解到暂存目录再 rename**：任何失败路径都不会留下半个包。
pub fn import_pack_from_zip(
    root: &Path,
    name: &str,
    bytes: &[u8],
    limits: &ZipLimits,
) -> anyhow::Result<ImportReport> {
    let name = sanitize_pack_name(name)
        .ok_or_else(|| anyhow::anyhow!("包名不合法（1~32 个字母/数字/下划线/短横线/汉字）: {name}"))?;
    // 预设包是**保留名**：用户不能建一个同名磁盘包去顶掉内建目录。
    // ⚠️ 比较的是**去掉序号与空格**的形态：`CATEGORY_ORDER` 里是 `① 主播情绪反应`，
    //    而包名过不了 `sanitize_pack_name`（`①` 与空格都不合法）——只比原串的话
    //    `主播情绪反应` 这种"看起来同名"的包会被放进来，界面上两个包长得一模一样。
    if crate::assets::sfx_catalog::CATEGORY_ORDER
        .iter()
        .any(|c| category_bare_name(c) == name)
    {
        anyhow::bail!("「{name}」是内建预设包的名字，换一个包名");
    }
    // 预设**库**名本身（「小森平」）同样是保留名：它不在 `CATEGORY_ORDER` 里
    // （那一串是大类），所以上面那条挡不住 —— 结果就是左侧出现两张一模一样的卡。
    if name == crate::assets::sfx_catalog::PRESET_PACK_NAME {
        anyhow::bail!("「{name}」是内建音效库的名字，换一个包名");
    }
    let dst = root.join(&name);
    if dst.exists() {
        anyhow::bail!("音效包「{name}」已存在：换个包名，或先删掉那一个");
    }
    std::fs::create_dir_all(root)?;

    let staging = root.join(format!(
        ".upload-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&staging)?;

    let result = (|| -> anyhow::Result<ImportReport> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| anyhow::anyhow!("不是有效的 zip 压缩包: {e}"))?;
        if archive.len() > limits.max_entries {
            anyhow::bail!("压缩包条目太多（{} 个，上限 {}）", archive.len(), limits.max_entries);
        }
        let mut groups: Vec<(String, String)> = Vec::new();
        let mut used_stems: std::collections::HashSet<String> = std::collections::HashSet::new();
        // 已写进暂存目录的文件名（转码要按真实文件名遍历）
        let mut written: Vec<String> = Vec::new();
        // zip 里带的 `pack.toml`（作者写的清单）
        let mut manifest_bytes: Option<Vec<u8>> = None;
        let mut converted_failed = 0usize;
        let mut copied = 0usize;
        let mut skipped = 0usize;
        let mut total_bytes: u64 = 0;

        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| anyhow::anyhow!("压缩包第 {} 个条目读不出来: {e}", i + 1))?;
            if entry.is_dir() {
                skipped += 1;
                continue;
            }
            let raw = entry.name().replace('\\', "/");
            if raw.starts_with("__MACOSX/") {
                skipped += 1;
                continue;
            }
            let file_name = raw.rsplit('/').next().unwrap_or("").to_string();
            // **清单**（`pack.toml`，可放在 zip 的任意层级）：整份收下，不走音频那条路。
            // 只认最浅的那一个（`pack.toml` 或 `<任意一层>/pack.toml`），后出现的覆盖前面的。
            if file_name.eq_ignore_ascii_case("pack.toml") && entry.size() <= MANIFEST_MAX_BYTES {
                let mut buf = Vec::new();
                entry.by_ref().take(MANIFEST_MAX_BYTES + 1).read_to_end(&mut buf)?;
                if buf.len() as u64 <= MANIFEST_MAX_BYTES {
                    manifest_bytes = Some(buf);
                    continue;
                }
            }
            if file_name.starts_with('.')
                || !is_audio_file_name(&file_name)
                || !is_safe_component(&file_name)
            {
                skipped += 1;
                continue;
            }
            // 子目录名 = 细分组（只取直接父目录；更深层也归到那一层）
            let group = if raw.contains('/') {
                raw.split('/').rev().nth(1).unwrap_or("").trim().to_string()
            } else {
                String::new()
            };

            let stem = stem_of(&file_name);
            if !is_safe_component(&stem) || !used_stems.insert(stem.clone()) {
                skipped += 1; // 拍平后重名：第一个胜
                continue;
            }
            if entry.size() > limits.max_file_bytes {
                skipped += 1;
                continue;
            }
            let mut buf = Vec::new();
            // 按**实际写入字节数**判（中央目录能谎报）
            entry
                .by_ref()
                .take(limits.max_file_bytes + 1)
                .read_to_end(&mut buf)?;
            if buf.len() as u64 > limits.max_file_bytes {
                skipped += 1;
                continue;
            }
            total_bytes += buf.len() as u64;
            if total_bytes > limits.max_total_bytes {
                anyhow::bail!("解压后总量超过上限（{} 字节）", limits.max_total_bytes);
            }
            // ⚠️ 写盘用**原始文件名**（带扩展名），出片侧按 `sfx/<stem>.mp3` 找 ——
            //    扩展名不是 mp3 时由 `downloader` 那条转码，这里不转。
            std::fs::write(staging.join(&file_name), &buf)?;
            written.push(file_name.clone());
            if !group.is_empty() {
                groups.push((stem, group));
            }
            copied += 1;
        }

        if copied == 0 {
            anyhow::bail!("这个压缩包里没有音频文件（支持 mp3 / wav / ogg / m4a / flac / aac）");
        }
        // **转 mp3**（尽力而为）：失败保留原格式，不中断整个导入。
        // ⚠️ 分组表里的键是 stem（不含扩展名），转码改扩展名不影响它 —— 所以顺序无所谓。
        for name in &written {
            if transcode_to_mp3(&staging.join(name)).is_err() {
                converted_failed += 1;
            }
        }
        // **作者带的清单优先**：原样落到包根，并且**不再生成 `.groups.toml`**
        // （一个事实一个写者；分组优先级见 `scan_pack`）。
        // ⚠️ 清单能解析才采纳：坏的整份丢掉，退回 zip 子目录分组。
        let mut author_manifest = false;
        if let Some(raw) = manifest_bytes {
            match std::str::from_utf8(&raw).ok().and_then(|s| toml::from_str::<PackManifest>(s).ok()) {
                Some(mut m) => {
                    // 词表外的 tag 丢弃（不失败）；作者没写 built_by 就当 author
                    for e in &mut m.entry {
                        let (kept, _dropped) = filter_known_tags(&e.tags);
                        e.tags = kept;
                    }
                    if m.pack.built_by.is_empty() {
                        m.pack.built_by = BUILT_BY_AUTHOR.to_string();
                    }
                    save_manifest(&staging, &m)?;
                    author_manifest = true;
                }
                None => {
                    tracing::warn!("压缩包里的 pack.toml 解析失败，已忽略（分组退回 zip 子目录）");
                }
            }
        }
        if !author_manifest && !groups.is_empty() {
            let mut text = String::from("# 由导入生成：变体键 = \"细分组\"。删掉这个文件不影响试听，只会把分组变成「其他」。\n");
            groups.sort();
            for (stem, group) in &groups {
                text.push_str(&format!("{stem} = \"{group}\"\n"));
            }
            std::fs::write(pack_groups_file(&staging), text)?;
        }
        Ok(ImportReport {
            copied,
            skipped,
            transcode_failed: converted_failed,
        })
    })();

    match result {
        Ok(v) => {
            std::fs::rename(&staging, &dst)?;
            Ok(v)
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "yekitrim-sfxpack-{}-{}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn audio_extension_predicate_is_the_single_source() {
        for ok in ["a.mp3", "B.WAV", "x.ogg", "y.m4a", "z.flac", "q.aac"] {
            assert!(is_audio_file_name(ok), "{ok} 应被接受");
        }
        // ⚠️ `.mp3` 的反例是 `"x."`（空扩展名）与无扩展名，不是 `".mp3"`
        //    （后者在 Rust 里 rsplit_once 得到空 stem + mp3，是合法文件名）。
        for bad in ["a.txt", "a.gif", "a", "a.mp3.txt", "x."] {
            assert!(!is_audio_file_name(bad), "{bad} 不该被接受");
        }
    }

    #[test]
    fn stem_matches_the_catalog_rule() {
        // 与 sfx_catalog::variant_stem 对同一批输入同结果（.mp3 这一档）
        for v in ["a.mp3", "Crowds/x.mp3", "City_french\\y.mp3"] {
            assert_eq!(stem_of(v), crate::assets::sfx_catalog::variant_stem(v));
        }
        // 非 mp3 扩展名也要去掉（variant_stem 只去 .mp3，这是这里多出来的那一步）
        assert_eq!(stem_of("a.wav"), "a");
    }

    #[test]
    fn import_flattens_and_records_groups() {
        let root = tmp("import");
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            for (name, data) in [
                ("门/敲铁门.mp3", &b"A"[..]),
                ("门/敲木门.mp3", &b"B"[..]),
                ("雨/下雨.mp3", &b"C"[..]),
                ("readme.txt", &b"skip"[..]),
            ] {
                use std::io::Write;
                w.start_file(name, opts).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        let rep = import_pack_from_zip(&root, "我的音效", &buf, &ZipLimits::default()).unwrap();
        assert_eq!(rep.copied, 3, "三个音频应当都进来");
        assert!(rep.skipped >= 1, "readme.txt 应被跳过");

        let pack = root.join("我的音效");
        // ⚠️ **扁平**：子目录不许留下 —— 出片侧按 `sfx/<stem>.mp3` 找文件
        assert!(pack.join("敲铁门.mp3").is_file());
        assert!(!pack.join("门").exists(), "子目录必须被拍平");

        let entries = scan_pack(&pack);
        assert_eq!(entries.len(), 3);
        let by: std::collections::HashMap<_, _> =
            entries.iter().map(|e| (e.stem.as_str(), e.group.as_str())).collect();
        assert_eq!(by.get("敲铁门"), Some(&"门"));
        assert_eq!(by.get("下雨"), Some(&"雨"));
    }

    #[test]
    fn import_refuses_preset_pack_names() {
        let root = tmp("reserved");
        let mut buf = Vec::new();
        {
            use std::io::Write;
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            w.start_file("a.mp3", zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(b"A").unwrap();
            w.finish().unwrap();
        }
        // 预设包名带 `①`/空格 ⇒ `sanitize_pack_name` 先挡掉；这一条保证**不会**被静默接受。
        let err = import_pack_from_zip(&root, "① 主播情绪反应", &buf, &ZipLimits::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("包名不合法"), "应拒绝非法包名，实际: {err}");
        // 去掉序号与空格的形态也必须拒绝（防止有人用 `主播情绪反应` 顶掉预设包）
        let err2 = import_pack_from_zip(&root, "主播情绪反应", &buf, &ZipLimits::default())
            .unwrap_err()
            .to_string();
        assert!(err2.contains("内建预设包"), "应拒绝去掉序号的预设名，实际: {err2}");
        // ⚠️ **预设库名本身**（「小森平」）不在 `CATEGORY_ORDER` 里（那串是大类），
        //    只比大类那一份的话它能建出来 ⇒ 左侧两张一模一样的卡。
        let err3 = import_pack_from_zip(&root, "小森平", &buf, &ZipLimits::default())
            .unwrap_err()
            .to_string();
        assert!(err3.contains("内建音效库"), "应拒绝预设库名，实际: {err3}");
    }

    #[test]
    fn manifest_overrides_label_group_and_tags() {
        let root = tmp("manifest");
        let pack = root.join("我的包");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("knock.mp3"), b"A").unwrap();
        std::fs::write(pack.join("rain.mp3"), b"B").unwrap();
        // 清单：改名 + 分组 + tag（tag 里混一个词表外的，读侧**不**过滤 ——
        // 过滤是导入/构建那两处的职责，读侧只如实呈现，免得"看一眼就变"）
        let toml = r#"
[pack]
author = "me"
built_by = "author"

[[entry]]
stem = "knock"
name = "敲铁门"
group = "门窗"
tags = ["敲门推门"]

[[entry]]
stem = "rain"
name = "小雨"
"#;
        std::fs::write(manifest_file(&pack), toml).unwrap();

        let entries = scan_pack(&pack);
        let by: std::collections::HashMap<_, _> =
            entries.iter().map(|e| (e.stem.as_str(), e)).collect();
        assert_eq!(by["knock"].label, "敲铁门", "清单的 name 应覆盖文件名");
        assert_eq!(by["knock"].group, "门窗");
        assert_eq!(by["knock"].tags, vec!["敲门推门".to_string()]);
        // 清单只写了一条 name、没写 group ⇒ 落「其他」（没有 .groups.toml）
        assert_eq!(by["rain"].label, "小雨");
        assert_eq!(by["rain"].group, crate::assets::sfx_catalog::UNCATEGORIZED_GROUP);
        assert!(by["rain"].tags.is_empty());
    }

    #[test]
    fn broken_manifest_is_ignored_not_fatal() {
        let root = tmp("badmanifest");
        let pack = root.join("坏清单");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("a.mp3"), b"A").unwrap();
        std::fs::write(manifest_file(&pack), "这不是 toml {{{").unwrap();

        // 坏清单 ⇒ 退回纯文件名行为，**包照样能用**
        let entries = scan_pack(&pack);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, "a");
        assert!(load_manifest(&pack).is_none());
    }

    #[test]
    fn manifest_round_trips_and_drops_the_derived_groups_file() {
        let root = tmp("mroundtrip");
        let pack = root.join("往返");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("a.mp3"), b"A").unwrap();
        // 先有一份导入期派生的分组表
        std::fs::write(pack_groups_file(&pack), "a = \"旧分组\"\n").unwrap();
        assert_eq!(scan_pack(&pack)[0].group, "旧分组");

        let m = PackManifest {
            pack: PackMeta { built_by: BUILT_BY_YKT.to_string(), ..Default::default() },
            entry: vec![ManifestEntry {
                stem: "a".into(),
                name: Some("甲".into()),
                group: Some("新分组".into()),
                tags: vec!["敲门推门".into()],
                gain_db: Some(-3.0),
            }],
        };
        save_manifest(&pack, &m).unwrap();
        // 一个事实一个写者：清单写出来之后，派生那份必须消失
        assert!(!pack_groups_file(&pack).exists(), "写清单后 .groups.toml 应被删掉");

        let back = load_manifest(&pack).unwrap();
        assert_eq!(back.entry.len(), 1);
        assert_eq!(back.entry[0].name.as_deref(), Some("甲"));
        assert_eq!(back.entry[0].gain_db, Some(-3.0));
        assert!(back.built_by_yekitrim());
        // 读回来的分组也走清单
        assert_eq!(scan_pack(&pack)[0].group, "新分组");
    }

    #[test]
    fn author_manifest_in_zip_wins_and_no_groups_file_is_written() {
        let root = tmp("zipmanifest");
        let mut buf = Vec::new();
        {
            use std::io::Write;
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            // 作者带了清单（放在一层子目录里也要认）
            w.start_file("包/pack.toml", opts).unwrap();
            w.write_all(
                b"[pack]\nauthor = \"me\"\n\n[[entry]]\nstem = \"knock\"\nname = \"\xe6\x95\xb2\xe9\x97\xa8\"\ngroup = \"\xe9\x97\xa8\xe7\xaa\x97\"\ntags = [\"\xe6\x95\xb2\xe9\x97\xa8\xe6\x8e\xa8\xe9\x97\xa8\", \"\xe8\x87\xaa\xe9\x80\xa0tag\"]\n",
            )
            .unwrap();
            w.start_file("包/门/knock.mp3", opts).unwrap();
            w.write_all(b"A").unwrap();
            w.finish().unwrap();
        }
        let rep = import_pack_from_zip(&root, "带清单", &buf, &ZipLimits::default()).unwrap();
        assert_eq!(rep.copied, 1);
        let pack = root.join("带清单");
        assert!(manifest_file(&pack).is_file(), "作者的清单要落到包根");
        assert!(!pack_groups_file(&pack).exists(), "有清单就不写派生分组表");

        let entries = scan_pack(&pack);
        assert_eq!(entries[0].label, "敲门");
        assert_eq!(entries[0].group, "门窗", "清单的 group 优先于 zip 子目录名");
        // ⚠️ 词表外的 tag 在**导入**时就被丢掉（`自造tag`）
        assert_eq!(entries[0].tags, vec!["敲门推门".to_string()]);
        let m = load_manifest(&pack).unwrap();
        assert!(!m.built_by_yekitrim(), "作者清单不应被标成 yekitrim 构建");
        assert_eq!(m.pack.built_by, BUILT_BY_AUTHOR);
    }

    #[test]
    fn filter_known_tags_keeps_only_the_builtin_vocabulary() {
        let known = crate::assets::sfx_catalog::all_tags();
        let first = known[0].0.to_string();
        let (kept, dropped) =
            filter_known_tags(&[first.clone(), "一定不在词表里的词".into(), first.clone()]);
        assert_eq!(kept, vec![first], "词表内的保留并去重");
        assert_eq!(dropped, 1);
        // 全空输入是个 no-op
        assert_eq!(filter_known_tags(&[]), (Vec::new(), 0));
    }

    #[test]
    fn entry_id_round_trips_and_separates_preset_from_user() {
        assert_eq!(entry_id("我的包", " knocking".trim()), "我的包/knocking");
        assert_eq!(split_entry_id("我的包/knocking"), Some(("我的包", "knocking")));
        // 预设包的 id（变体键）永远不含 `/` —— 这就是两套 id 的判据
        assert_eq!(split_entry_id("knocking_an_iron_door1"), None);
        assert_eq!(split_entry_id("Crowds-Amb_Terrasse_Resto"), None);
    }

    #[test]
    fn candidates_text_lists_user_packs_and_skips_used() {
        let root = tmp("cand");
        let mut buf = Vec::new();
        {
            use std::io::Write;
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            for (name, data) in [("门窗/敲门.mp3", &b"A"[..]), ("天气/下雨.mp3", &b"B"[..])] {
                w.start_file(name, opts).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        import_pack_from_zip(&root, "我的包", &buf, &ZipLimits::default()).unwrap();

        // ⚠️ **未构建的包不进候选**（没有清单 = LLM 拿不到语义信息）——
        //    与贴纸 `local_sticker_candidates_text` 只列已构建的同一口径。
        let empty = candidates_text(&root, &std::collections::HashSet::new(), 32);
        assert!(empty.is_empty(), "未构建的包不该进候选：{empty}");

        // 补一份清单（模拟构建完成）→ 进候选，且 tag 一起写出来
        let pack_dir = root.join("我的包");
        let m = PackManifest {
            pack: PackMeta { built_by: BUILT_BY_YKT.to_string(), ..Default::default() },
            entry: vec![ManifestEntry {
                stem: "敲门".into(),
                tags: vec!["敲门推门".into()],
                ..Default::default()
            }],
        };
        save_manifest(&pack_dir, &m).unwrap();

        let text = candidates_text(&root, &std::collections::HashSet::new(), 32);
        assert!(text.contains("我的包:"), "候选里应有包名：{text}");
        assert!(text.contains("我的包/敲门"), "候选里应有完整 id：{text}");
        assert!(text.contains("tag: 敲门推门"), "有 tag 的条目应带出 tag：{text}");
        assert!(text.contains("我的包/下雨"));

        // 已用过的要跳过（`used` 里存的是完整 id）
        let mut used = std::collections::HashSet::new();
        used.insert("我的包/敲门".to_string());
        let text2 = candidates_text(&root, &used, 32);
        assert!(!text2.contains("我的包/敲门"), "已用过的仍被列出：{text2}");
        assert!(text2.contains("我的包/下雨"));
    }

    #[test]
    fn candidates_text_is_empty_without_packs() {
        let root = tmp("nocand");
        assert!(candidates_text(&root, &std::collections::HashSet::new(), 32).is_empty());
    }

    #[test]
    fn scan_all_skips_empty_and_hidden_dirs() {
        let root = tmp("scan");
        std::fs::create_dir_all(root.join("空包")).unwrap();
        std::fs::create_dir_all(root.join(".upload-123")).unwrap();
        std::fs::write(root.join(".upload-123/x.mp3"), b"x").unwrap();
        let with = root.join("有货");
        std::fs::create_dir_all(&with).unwrap();
        std::fs::write(with.join("a.mp3"), b"a").unwrap();

        let all = scan_all(&root);
        assert_eq!(all.len(), 1, "只应列出非空、非隐藏的包：{all:?}");
        assert_eq!(all[0].0, "有货");
    }

    #[test]
    fn empty_zip_leaves_nothing_behind() {
        let root = tmp("empty");
        let mut buf = Vec::new();
        {
            use std::io::Write;
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            w.start_file("a.txt", zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(b"x").unwrap();
            w.finish().unwrap();
        }
        assert!(import_pack_from_zip(&root, "空的", &buf, &ZipLimits::default()).is_err());
        assert!(!root.join("空的").exists(), "失败的导入不许留下目录");
        // 暂存目录也要清干净
        let leftovers: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().collect();
        assert!(leftovers.is_empty(), "暂存目录没清: {leftovers:?}");
    }
}
