//! polish::assets::downloader：统一的资产下载管理器（贴纸图 + SFX 落盘）。
//!
//! 入口：`AssetDownloader::{new, download_stickers, download_sfx_for_segments, asset_map}`、`DownloadReport`、`collect_sfx_ids`。
//! 边界：贴纸库固定在 `<程序目录>/stickers/`（不是项目里那份拷贝），debug / release 各一份、互不相干；文件名一律过 `sticker_index::is_image_file_name` —— 见 `crates/yekitrim-polish/src/assets/AGENTS.md`

use std::path::{Path, PathBuf};
use crate::project::Segment;
use crate::project::Event;

/// 资产下载管理器
pub struct AssetDownloader {
    project_path: PathBuf,
    client: reqwest::Client,
    /// 仅测试用：把贴纸**库**根换成临时目录（见 `sticker_lib_root`）。
    #[cfg(test)]
    test_lib_root: Option<PathBuf>,
}

/// 下载报告
#[derive(Debug, Default)]
pub struct DownloadReport {
    pub sticker_downloaded: usize,
    pub sticker_existing: usize,
    pub sfx_downloaded: usize,
    pub sfx_existing: usize,
    /// 复制进项目的**视频素材**份数（第 6 批 / 规格 §11.2 第 2 条）。
    pub video_downloaded: usize,
    /// 已经在项目里、跳过复制的视频素材份数（幂等）。
    pub video_existing: usize,
    pub errors: Vec<String>,
}

impl AssetDownloader {
    pub fn new(project_path: &Path) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Referer", "https://sounddictionary.info/".parse().unwrap());
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("yekitrim-polish/0.1")
            .build()
            .unwrap_or_default();
        Self {
            project_path: project_path.to_path_buf(),
            client,
            #[cfg(test)]
            test_lib_root: None,
        }
    }

    /// 贴纸**库**根目录。生产恒为程序目录下那一份；测试可注入一个临时根，
    /// 免得并发跑的用例互相删对方的包（各用例共享同一个全局库目录）。
    fn sticker_lib_root(&self) -> PathBuf {
        #[cfg(test)]
        if let Some(r) = &self.test_lib_root {
            return r.clone();
        }
        yekitrim_core::owner::sticker_dir_readable(yekitrim_core::owner::Owner::from_env())
    }

    /// 统一 sticker 下载入口: 根据 source 类型分发
    pub async fn download_stickers(
        &self,
        segments: &[Segment],
        assignments: &[(usize, usize, String)], // (segment_idx, event_idx, source) — 事件级预分配
    ) -> DownloadReport {
        let sticker_dir = self.project_path.join("assets").join("stickers");
        std::fs::create_dir_all(&sticker_dir).ok();
        let mut report = DownloadReport::default();

        // 按事件精确下载，与 process Step 2 的 name 解析保持同一来源分配
        for (seg_idx, event_idx, source) in assignments {
            // ⚠️ **两个载体都要认**（`process/AGENTS.md`「贴纸有两个载体」）：编辑器「贴纸轨」建的
            // 贴纸是 `Element{kind:"sticker"}` —— 关键词在 `text`、`name` 也在它自己身上。
            // 只认 `Event::Sticker` 的话手动贴纸**永远拿不到文件**（`assignments` 里明明有它），
            // 出片时转译器找不到图就静默 drop。
            if let Some(seg) = segments.get(*seg_idx)
                && let Some(event) = seg.events.get(*event_idx)
                && let Some(keyword) = event.sticker_keyword()
                && !keyword.trim().is_empty()
            {
                let name = event.sticker_name();
                match source.as_str() {
                            "emoji" => {
                                // 优先下载 process 已解析的具体 codepoint (去重可能选了非首个),
                                // 无则按 keyword 搜索首个
                                // ⚠️ 多码点序列（`203c_fe0f`）带 `_`，不是纯十六进制 —— 判据要放它过去，
                                // 否则 39% 的表情会退回"按 keyword 搜首个"、拿到另一个表情。
                                let codepoints = match name {
                                    Some(n) if !n.is_empty() && n.chars().all(|c| c.is_ascii_hexdigit() || c == '_') => vec![n.to_string()],
                                    _ => vec![],
                                };
                                let (new_files, existing) = if codepoints.is_empty() {
                                    super::emoji_search::download_images(
                                        &[keyword.to_string()], &sticker_dir, &self.client, 1,
                                    ).await
                                } else {
                                    super::emoji_search::download_images_by_codepoints(
                                        &codepoints, &sticker_dir, &self.client,
                                    ).await
                                };
                                report.sticker_downloaded += new_files.len();
                                report.sticker_existing += existing;
                            }
                            "irasu" => {
                                let (new_files, existing) = super::irasu_search::download_images(
                                    keyword, &sticker_dir, &self.client, 1,
                                ).await;
                                report.sticker_downloaded += new_files.len();
                                report.sticker_existing += existing;
                            }
                            // 非 emoji/irasu 的来源 = 贴纸包名（如 ysyk），只复制该事件实际选中的文件
                            _ => {
                                let copied = self.copy_sticker_from_local(
                                    source, name, &sticker_dir, &mut report,
                                );
                                report.sticker_downloaded += copied;
                            }
                        }
            }
        }
        report
    }

    /// 把 EDL 里引用到的**视频素材**从素材库复制进项目（第 6 批 / 规格 §11.2 第 2 条）。
    ///
    /// # 纪律与贴纸那条**完全同源**（`assets/AGENTS.md` 那条 ⚠️）
    ///
    /// 库在 `<程序目录>/stickers/videos/`，而**出片与预览只读项目目录**
    /// （`<项目>/assets/videos/`）。这是"单个 polish 项目能独立复现"的**唯一**保证 ——
    /// 库改名 / 换机器之后老项目照样出得来。所以这里**只复制、绝不写库里的绝对路径**。
    ///
    /// # 判据取 EDL 段自己的 `asset_id`
    ///
    /// 段里写的 `asset_id` 就是**素材名**（去扩展名，与 [`sticker_index::VideoAsset::name`]
    /// 同口径），不是路径 —— 用户换台机器、素材库搬家，那个字符串照样指得对。
    ///
    /// # 两条硬规矩（照抄贴纸那条的两条）
    ///
    /// ① **复制失败必须报出来**（`report.errors`）—— "项目里没有这个素材"以前对贴纸是一条
    ///    看不见的失败（日志一个字都没有，出片时贴纸凭空消失）。视频更贵：底座的
    ///    `file_for(asset_id)` 找不到文件是**出片整段失败**，所以这里必须点名。
    /// ② **幂等**：目标已存在就跳过（复制过的项目重跑 `process` 不该反复拷几百 MB）。
    pub fn download_videos(&self, asset_ids: &[String]) -> DownloadReport {
        // ⚠️ 判据是 `video_dir_readable()`（**不是** `sticker_dir().join("videos")`）：
        // 视频库 2026-10-09 从贴纸根底下挪到 `<数据目录>/resources/videos/`，
        // 且带存量回落。拼错 ⇒ 每个视频素材都报"库里没有它"，成片整段失败。
        self.download_videos_from(
            &yekitrim_core::owner::video_dir_readable(yekitrim_core::owner::Owner::from_env()),
            asset_ids,
        )
    }

    /// [`Self::download_videos`] 的**视频库目录可注入**版本。
    ///
    /// ⚠️ 参数是**视频库目录本身**（`<数据目录>/resources/videos/`），不是贴纸根
    /// —— 2026-10-09 挪库时收紧了语义，与 `sticker_index::scan_video_library` 同一口径。
    /// 老签名收的是贴纸根、内部再 `join("videos")`，于是"视频库在哪"有两份推导。
    ///
    /// ⚠️ 它存在的理由只有一个：**让单测能真跑**。`path::video_dir_readable()` 是
    /// `data_dir()/resources/videos`，而"指到临时库上"才是这条逻辑唯一值得验的地方
    /// —— 靠改环境变量去测会拿到一个**不确定的**结果（两个并行用例抢同一个进程级变量），
    /// 那种测试的失败是噪声、不是信号。
    pub fn download_videos_from(
        &self,
        video_dir: &Path,
        asset_ids: &[String],
    ) -> DownloadReport {
        let mut report = DownloadReport::default();
        if asset_ids.is_empty() {
            return report;
        }
        let dest_dir = self.project_path.join("assets").join(super::sticker_index::VIDEO_LIBRARY_SUBDIR);
        if let Err(e) = std::fs::create_dir_all(&dest_dir) {
            report.errors.push(format!(
                "视频素材：建项目素材目录失败（{}）: {e}",
                dest_dir.display()
            ));
            return report;
        }
        let library = video_dir;
        let mut seen: Vec<&str> = Vec::new();
        for id in asset_ids {
            let id = id.trim();
            if id.is_empty() || seen.contains(&id) {
                continue;
            }
            seen.push(id);
            let Some(asset) = super::sticker_index::find_video_asset(library, id) else {
                report.errors.push(format!(
                    "视频素材「{id}」: 程序目录素材库里没有它（找的是 {}）—— \
                     出片时这一段会失败，请先把素材放进素材库再跑一次后处理",
                    library.display()
                ));
                continue;
            };
            let dest = dest_dir.join(asset.file_name());
            if dest.exists() {
                report.video_existing += 1;
                continue;
            }
            match std::fs::copy(&asset.path, &dest) {
                Ok(_) => {
                    report.video_downloaded += 1;
                    println!("  ✓ 视频素材 {} → {}", id, dest.display());
                }
                Err(e) => report.errors.push(format!(
                    "视频素材「{id}」: 复制 {} 进项目失败: {e}",
                    asset.path.display()
                )),
            }
        }
        report
    }

    /// 从程序目录的 stickers/ 复制当前事件实际使用的贴纸文件。
    /// 优先按 `name` 精确复制包内对应文件；**找不到就把原因写进 `report.errors`，不再静默返回 0**
    /// ——"项目里没有这张图"以前是一条看不见的失败：日志一个字都没有，出片时贴纸凭空消失。
    ///
    /// ⚠️ `name` 缺失或为 `auto`（Step 2 没解析出文件）时**不复制**：以前会退化成"复制包内第一个"，
    /// 而渲染侧一律按 `name` 找文件（`prepare_sticker_data` / `build_sticker_map`），
    /// 复制那个文件没有任何用处，只会让项目里多一份用不到的图。
    fn copy_sticker_from_local(
        &self,
        source: &str,
        name: Option<&str>,
        dest_dir: &Path,
        report: &mut DownloadReport,
    ) -> usize {
        let Some(name) = name.map(str::trim).filter(|n| !n.is_empty() && *n != "auto") else {
            report.errors.push(format!(
                "贴纸「{source}」: 事件还没解析出文件名（name 缺失或 auto）—— 先跑一次 polish process，\
                 否则这张贴纸不会进项目、出片会缺"
            ));
            return 0;
        };
        let sticker_dir = self.sticker_lib_root();
        if !sticker_dir.exists() {
            report.errors.push(format!(
                "贴纸「{source}」: 程序目录贴纸库不存在（{}），复制不进项目",
                sticker_dir.display()
            ));
            return 0;
        }

        let copy_file = |path: &PathBuf, report: &mut DownloadReport| -> usize {
            let Some(fname) = path.file_name() else {
                return 0;
            };
            let dest = dest_dir.join(fname);
            if dest.exists() {
                return 0; // 幂等：已经在项目里了
            }
            match std::fs::copy(path, &dest) {
                Ok(_) => 1,
                Err(e) => {
                    report.errors.push(format!("贴纸「{source}」: 复制 {} 进项目失败: {e}", path.display()));
                    0
                }
            }
        };

        // 新结构：贴纸包目录按 event.name 精确复制
        let packages = super::sticker_index::scan_packages(&sticker_dir);
        if let Some(pkg) = packages.iter().find(|p| p.name == source) {
            return match pkg
                .files
                .iter()
                .find(|p| p.file_stem().and_then(|s| s.to_str()) == Some(name))
            {
                Some(found) => copy_file(found, report),
                None => {
                    report.errors.push(format!(
                        "贴纸包「{source}」里没有「{name}」这个文件（包内共 {} 张）—— 事件引用的名字与库对不上",
                        pkg.files.len()
                    ));
                    0
                }
            };
        }

        // 旧结构兜底：按文件名/tags 匹配后只复制匹配到的具体文件
        let idx = super::sticker_index::StickerIndex::scan(&sticker_dir);
        let Some(m) = idx.match_keyword(source) else {
            report.errors.push(format!(
                "贴纸「{source}」: 程序目录贴纸库里没有这个包/分组，复制不进项目"
            ));
            return 0;
        };
        let files = idx.groups()[m.group_index].all_files();
        match files
            .iter()
            .find(|p| p.file_stem().and_then(|s| s.to_str()) == Some(name))
        {
            Some(found) => copy_file(found, report),
            None => {
                report.errors.push(format!("贴纸「{source}」: 同名分组里没有「{name}」这个文件"));
                0
            }
        }
    }

    /// 按需下载 segments 中引用的 SFX (blacklist 中的跳过)
    pub async fn download_sfx_for_segments(&self, segments: &[Segment], blacklist: &[String]) -> DownloadReport {
        let mut report = DownloadReport::default();
        let sfx_dir = self.project_path.join("sfx");
        std::fs::create_dir_all(&sfx_dir).ok();

        let ids = collect_sfx_ids(segments);
        if ids.is_empty() { return report; }

        println!("  ── SFX 按需下载 ──");
        for id in &ids {
            // tag: 前缀 → 取第1个未被屏蔽的匹配下载（不再展开全部）
            if let Some(tag_name) = id.strip_prefix("tag:") {
                let matches = super::sfx_catalog::search_by_tags(&[tag_name.to_string()]);
                let entry = matches.into_iter()
                    .find(|e| !super::sfx_catalog::is_blacklisted(&e.id, blacklist));
                let entry = match entry {
                    Some(e) => e,
                    None => {
                        report.errors.push(format!("sfx tag「{}」: 未匹配到音效", tag_name));
                        println!("  ⚠ sfx tag「{}」: 未匹配到音效", tag_name);
                        continue;
                    }
                };
                let n = self.download_one_sfx(&sfx_dir, &entry.id, &mut report, blacklist).await;
                report.sfx_downloaded += n;
                println!("  ✓ sfx tag「{}」→ {}", tag_name, entry.id);
            } else {
                let n = self.download_one_sfx(&sfx_dir, id, &mut report, blacklist).await;
                report.sfx_downloaded += n;
            }
        }
        report
    }

    /// 下载单个 SFX 文件（id 可能是组 id 或变体文件名），返回是否新下载（1=新下载, 0=已存在/跳过）
    async fn download_one_sfx(&self, sfx_dir: &Path, id: &str, report: &mut DownloadReport, blacklist: &[String]) -> usize {
        if super::sfx_catalog::is_blacklisted(id, blacklist) {
            println!("  ⚠ sfx {}: 已在屏蔽列表中，跳过下载", id);
            return 0;
        }
        // 确定 (entry, 变体文件名): 组 id 或变体名均可
        let (entry, variant) = if let Some(e) = super::sfx_catalog::lookup(id) {
            let v = e.variants.first().cloned().unwrap_or_default();
            (e, v)
        } else if let Some(e) = super::sfx_catalog::lookup_variant(id) {
            let v = format!("{}.mp3", id);
            (e, v)
        } else {
            report.errors.push(format!("sfx {}: 未在 catalog 中找到，请检查 sfx_catalog.json", id));
            println!("  ⚠ sfx {}: 未在 catalog 中找到", id);
            return 0;
        };
        let stem = variant.strip_suffix(".mp3").unwrap_or(&variant);
        let path = sfx_dir.join(format!("{}.mp3", stem));
        if path.exists() {
            report.sfx_existing += 1;
            return 0;
        }
        let url = super::sfx_catalog::variant_url(entry, &variant);
        // 404 容错: 小森平站部分文件在备用目录 sound_os2/ 下 (页面引用 sound/ 但文件缺失)
        if let Err(_) = self.download_file(&url, &path).await {
            let alt = url.replace("/sound/", "/sound_os2/");
            if alt != url {
                match self.download_file(&alt, &path).await {
                    Ok(_) => {
                        report.sfx_downloaded += 1;
                        return 1;
                    }
                    Err(_) => {
                        // 双目录都失败 → 尝试同组其他变体
                        report.errors.push(format!("sfx {}: 下载失败 ({})", id, url));
                        println!("  ⚠ sfx {}: 下载失败, 尝试其他变体", id);
                        return 0;
                    }
                }
            }
            report.errors.push(format!("sfx {}: 下载失败 ({})", id, url));
            return 0;
        }
        // 下载成功路径
        println!("  ✓ sfx {} → {}", id, path.display());
        1
    }

    /// 生成资产路径映射 (注入到 config JSON)
    ///
    /// 扫描 `assets/stickers/`（**含子目录**），返回 stem → URL 映射。
    ///
    /// ⚠️⚠️ 与 `render::server::build_sticker_map` **必须同口径**（2026-10-14 一起修）：
    /// 贴纸库实测有**扁平**与**带版本子目录**（`ysyk_0.9.0/*.gif`）两种布局。
    /// 老实现只扫顶层、且不筛扩展名 ⇒ 子目录那个工程只算出一个**目录**当成图片的键
    /// （`"ysyk_0.9" -> "/assets/stickers/ysyk_0.9.0"`），210 张真图一张都没进映射
    /// ⇒ **预览里贴纸一张都不出现且不报错**。
    ///
    /// ⚠️ 只收图片走 `is_image_file_name`（**唯一扩展名判据**，别再抄一份清单）；
    /// URL 必须是**含子目录的相对路径**，否则 ServeDir 按真实路径找不到。
    pub fn asset_map(&self) -> serde_json::Value {
        use super::sticker_index::is_image_file_name;
        let sticker_dir = self.project_path.join("assets").join("stickers");
        let mut sticker_map = serde_json::Map::new();
        let mut stack = vec![sticker_dir.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                if !is_image_file_name(&name) {
                    continue;
                }
                let Some(stem) = Path::new(&name).file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let rel = path
                    .strip_prefix(&sticker_dir)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                sticker_map.insert(
                    stem.to_string(),
                    serde_json::Value::String(format!("/assets/stickers/{rel}")),
                );
            }
        }
        serde_json::json!({
            "sticker_map": sticker_map,
        })
    }

    async fn download_file(&self, url: &str, dest: &Path) -> anyhow::Result<()> {
        let response = self.client.get(url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("HTTP {}", response.status());
        }
        let bytes = response.bytes().await?;
        std::fs::write(dest, &bytes)?;
        Ok(())
    }
}

/// 从 EDL 里收集所有被引用到的**视频素材 id**（去重、保序、过滤空串）。
///
/// 与 [`collect_sfx_ids`] 同一个形状：**消费点拿它去调 [`AssetDownloader::download_videos`]**，
/// 而"哪些素材被引用"这个判断只写在这里一份 —— 出片那条腿要复现"这段用哪个素材"时
/// 也读同一份 EDL 段（`render/dhampir.rs` 在拼 asset 列表时按 `asset_id` 查）。
///
/// ⚠️ **只收 `is_asset()` 成立的段**：`asset_id` 写在本条片段上是**无意义**的
/// （`EdlClip::asset_id_ref` 的文档写着这条），收进来会让"没引用的素材被复制进项目"——
/// 项目白白胖几百 MB，而用户看不到这件事。
pub fn collect_edl_asset_ids(clips: &[yekitrim_render_core::types::EdlClip]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for clip in clips {
        if let Some(id) = clip.asset_id_ref() {
            let id = id.to_string();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

/// 从 segments 中收集所有被引用的 SFX id
pub fn collect_sfx_ids(segments: &[Segment]) -> Vec<String> {
    let mut ids: Vec<String> = vec![];
    for seg in segments {
        for event in &seg.events {
            match event {
                // Sfx 事件: name 字段
                Event::Sfx { name, .. } => {
                    if !name.is_empty() && !ids.contains(name) {
                        ids.push(name.clone());
                    }
                }
                // 其他事件: sfx 字段
                _ => {
                    if let Some(sfx) = extract_sfx_field(event)
                        && !sfx.is_empty() && !ids.contains(sfx) {
                            ids.push(sfx.clone());
                        }
                }
            }
        }
    }
    ids
}

/// 从事件中提取 sfx 字段值 (所有可带 sfx 的事件变体)
fn extract_sfx_field(event: &Event) -> Option<&String> {
    match event {
        Event::Sticker { sfx: Some(s), .. } => Some(s),
        Event::Shake { sfx: Some(s), .. } => Some(s),
        Event::Flash { sfx: Some(s), .. } => Some(s),
        Event::Blur { sfx: Some(s), .. } => Some(s),
        Event::Vignette { sfx: Some(s), .. } => Some(s),
        Event::HueShift { sfx: Some(s), .. } => Some(s),
        Event::Noise { sfx: Some(s), .. } => Some(s),
        Event::Pulse { sfx: Some(s), .. } => Some(s),
        Event::Overlay { sfx: Some(s), .. } => Some(s),
        _ => None,
    }
}

#[cfg(test)]
mod video_asset_tests {
    use super::*;
    use yekitrim_render_core::types::EdlClip;

    fn asset_clip(id: &str) -> EdlClip {
        EdlClip {
            source_start: 1.0,
            source_end: 2.0,
            speed: 1.0,
            kind: None,
            label: None,
            source_kind: Some("asset".into()),
            asset_id: Some(id.into()),
            transition_in: None,
        }
    }

    fn source_clip() -> EdlClip {
        EdlClip {
            source_start: 0.0,
            source_end: 1.0,
            speed: 1.0,
            kind: None,
            label: None,
            source_kind: None,
            asset_id: None,
            transition_in: None,
        }
    }

    /// 收集判据：**只收 `is_asset()` 成立的段**、去重、保序。
    ///
    /// ⚠️ 本条片段上写了 `asset_id` 也**不算引用** —— 收进来会让没被用到的素材
    /// 白白复制几百 MB 进项目，而用户看不到这件事（`EdlClip::asset_id_ref` 的注释同一句）。
    #[test]
    fn collect_edl_asset_ids_only_asset_clips_dedup_and_order() {
        let clips = vec![
            source_clip(),
            asset_clip("op-ed"),
            asset_clip("b-roll"),
            asset_clip("op-ed"), // 去重
            EdlClip { asset_id: Some("not-referenced".into()), ..source_clip() }, // 非 asset ⇒ 不收
            EdlClip { source_kind: Some("asset".into()), ..source_clip() },      // asset 但没 id ⇒ 不收
            EdlClip { source_kind: Some("asset".into()), asset_id: Some("  ".into()), ..source_clip() },
        ];
        assert_eq!(collect_edl_asset_ids(&clips), vec!["op-ed", "b-roll"]);
        assert!(collect_edl_asset_ids(&[]).is_empty());
        assert!(collect_edl_asset_ids(&[source_clip()]).is_empty());
    }

    /// **复制进项目**：从素材库复制到 `<项目>/assets/videos/`、**幂等**、
    /// 库里没有时**报出来**（不许静默）。
    ///
    /// ⚠️ 走 [`AssetDownloader::download_videos_from`]（库根可注入）—— 用 `sticker_dir()`
    /// 那条路要靠环境变量把库指到临时目录，而那是**进程级**的：两个并行用例抢同一个变量
    /// 会让结论变成噪声。这里要验的恰恰是"指到某个库上会发生什么"。
    ///
    /// ⚠️ 真实工程上那条判据（**删掉库里的素材后项目照样出片**）在 `.workbuddy/task8-e2e/`
    /// 里真跑过、逐帧像素逐点相同；这条是它的单元级对照。
    #[test]
    fn download_videos_copies_into_project_and_reports_missing() {
        let lib = tempfile::tempdir().unwrap();
        // ⚠️ `lib.path()` 就是**视频库目录本身**（2026-10-09 挪库后语义收紧：
        //    以前传的是贴纸根、函数内部再 `join("videos")`，于是"视频库在哪"有两份推导）。
        let vids = lib.path().to_path_buf();
        std::fs::write(vids.join("op-ed.mp4"), b"fake-video-bytes").unwrap();
        // 库里的贴纸不该被当成视频
        std::fs::write(vids.join("sticker.gif"), b"x").unwrap();

        let proj = tempfile::tempdir().unwrap();
        let downloader = AssetDownloader::new(proj.path());
        let copy = |ids: &[&str]| {
            let owned: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
            downloader.download_videos_from(&vids, &owned)
        };

        // ① 真的复制进去了，而且内容一个字节不差
        let r = copy(&["op-ed"]);
        assert_eq!((r.video_downloaded, r.video_existing, r.errors.len()), (1, 0, 0), "{:?}", r.errors);
        let copied = proj.path().join("assets").join("videos").join("op-ed.mp4");
        assert!(copied.exists(), "必须复制进项目目录");
        assert_eq!(std::fs::read(&copied).unwrap(), b"fake-video-bytes");

        // ② 幂等：第二次是 existing，不是又复制一份（项目不会反复拷几百 MB）
        let again = copy(&["op-ed"]);
        assert_eq!((again.video_downloaded, again.video_existing), (0, 1), "必须是幂等的");

        // ③ 库里没有的 id ⇒ 一条 errors（点名 + 说清去哪儿找），且不产生文件
        let missing = copy(&["nope"]);
        assert_eq!(missing.errors.len(), 1, "{:?}", missing.errors);
        assert!(missing.errors[0].contains("nope"), "{:?}", missing.errors);
        assert!(!proj.path().join("assets").join("videos").join("nope.mp4").exists());

        // ④ 同一个 id 传两遍**只处理一次**（去重发生在 seen 里，第二遍直接 continue）
        let dup = copy(&["op-ed", "op-ed"]);
        assert_eq!((dup.video_downloaded, dup.video_existing, dup.errors.len()), (0, 1, 0), "去重后只处理一次");

        // ⑤ 空列表 ⇒ 一个文件都不碰、不报错
        let empty = copy(&[]);
        assert_eq!((empty.video_downloaded, empty.video_existing), (0, 0));
        assert!(empty.errors.is_empty());

        // ⑥ 空白的 id 不当成素材（否则会去库里找一个叫 "" 的文件）
        assert!(copy(&["   "]).errors.is_empty());
    }

    /// **两个载体都要把贴纸文件复制进项目**（2026-10-15 用户报「贴纸无法加入」）。
    ///
    /// 钉的是「贴纸轨加的贴纸在成片里凭空消失」那条链的**第一环**：
    /// `download_stickers` 只认 `Event::Sticker` 时，编辑器产的
    /// `Element{kind:"sticker"}` **一个文件都不会落进 `<项目>/assets/stickers/`**，
    /// 于是转译器找不到图、成片里静默没有 —— 而 DOM 预览走 engine.js 另一条路
    /// **照常显示**，所以两边各自看都正常。
    ///
    /// ⚠️ 判据是「文件真的出现在项目目录里」，不是「函数被调到了」。
    #[tokio::test]
    async fn element_sticker_gets_its_file_copied_into_the_project() {
        use crate::project::{Event, ProjectFile, Segment};

        // ⚠️ 路径**不带 pid**：带 pid 的话每次跑都是新目录，`remove_dir_all` 清不掉上一次那份，
        // 而 `copy_sticker_from_local` 是幂等的（目的文件已存在就不复制、计数 0）⇒ 断言假红。
        let tmp = std::env::temp_dir().join("vt-sticker-copy-dual-carrier");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("assets/stickers")).unwrap();

        // 假包：<程序目录>/resources/stickers/<包>/x.gif（1x1 透明 GIF，只为让 is_image_path 认它）
        // ⚠️ **库根用临时目录，不碰进程级共享的那一份**：贴纸库是全局的，
        // 而 cargo 默认并发跑用例 —— 共享它会让"单独重跑绿、全量跑红"这种假红随机出现（踩过）。
        let pkg_root = tmp.join("libroot");
        let pkg_name = "dual_carrier_pkg";
        let pkg = pkg_root.join(pkg_name);
        let _ = std::fs::remove_dir_all(&pkg);
        std::fs::create_dir_all(&pkg).unwrap();
        const GIF: &[u8] = &[
            0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00,
            0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00,
            0x2c, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01,
            0x00, 0x3b,
        ];
        std::fs::write(pkg.join("x.gif"), GIF).unwrap();
        // ⚠️ 先自证桩已就位：库扫不到它的话，这条测的就不是"载体认不认"，而是"库在不在"，
        // 失败信息会指向完全错误的方向（踩过）。
        let scanned = crate::assets::sticker_index::scan_packages(&pkg_root);
        assert!(
            scanned.iter().any(|p| p.name == pkg_name && !p.files.is_empty()),
            "桩包没被库扫到（root={}）—— 这条测的前提就不成立",
            pkg_root.display()
        );

        let mut cfg = ProjectFile::example();
        cfg.segments = vec![Segment {
            role: String::new(),
            start: 0.0,
            end: 10.0,
            fx: "low".into(),
            note: String::new(),
            events: vec![Event::Element {
                time: 1.0,
                end: Some(3.0),
                kind: "sticker".into(),
                // ⚠️ 关键词在 text（Sticker 那个载体叫 keyword）
                text: "生气".into(),
                style: None,
                anim: None,
                style_anim: None,
                effect_anim: None,
                transition_anim: None,
                layout_angle_deg: None,
                pos: None,
                color: None,
                sticker_tag: Some("emotion".into()),
                x: None,
                y: None,
                manual: true,
                name: Some("x".into()),
                source: Some(pkg_name.into()),
                manual_pos: None,
            }],
            extra: Default::default(),
        }];

        let mut dl = AssetDownloader::new(&tmp);
        dl.test_lib_root = Some(pkg_root.clone());
        // assignments 的形状 = download_keyword_assets 产出的那个：(段下标, 事件下标, source)
        let report = dl
            .download_stickers(&cfg.segments, &[(0, 0, pkg_name.to_string())])
            .await;

        let landed = tmp.join("assets/stickers/x.gif");
        let names: Vec<String> = crate::assets::sticker_index::scan_packages(&pkg_root)
            .iter()
            .map(|p| format!("{}[{}]{:?}", p.name, p.files.len(), p.files))
            .collect();
        let dest_listing: Vec<String> = std::fs::read_dir(tmp.join("assets/stickers"))
            .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect())
            .unwrap_or_default();
        assert!(
            landed.exists(),
            "贴纸轨的贴纸必须被复制进项目（errors={:?}；库={:?}；dest={:?}；downloaded={}）",
            report.errors,
            names,
            dest_listing,
            report.sticker_downloaded,
        );
        assert_eq!(report.sticker_downloaded, 1, "应当记一次新复制");

        let _ = std::fs::remove_dir_all(&tmp);
        // ⚠️ **不删库里的桩包**：它是共享资源，删掉会让"单独重跑这一条"失去前提（踩过）。
    }
}
