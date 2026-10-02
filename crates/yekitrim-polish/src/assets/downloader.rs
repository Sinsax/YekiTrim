//! polish::assets::downloader：统一的资产下载管理器（贴纸图 + SFX 落盘）。
//!
//! 入口：`AssetDownloader::{new, download_stickers, download_sfx_for_segments, asset_map}`、`DownloadReport`、`collect_sfx_ids`。
//! 边界：贴纸库固定在 `<程序目录>/stickers/`（不是项目里那份拷贝），debug / release 各一份、互不相干；文件名一律过 `sticker_index::is_image_file_name` —— 见 `crates/vtrim-polish/src/assets/AGENTS.md`

use std::path::{Path, PathBuf};
use crate::project::Segment;
use crate::project::Event;

/// 资产下载管理器
pub struct AssetDownloader {
    project_path: PathBuf,
    client: reqwest::Client,
}

/// 下载报告
#[derive(Debug, Default)]
pub struct DownloadReport {
    pub sticker_downloaded: usize,
    pub sticker_existing: usize,
    pub sfx_downloaded: usize,
    pub sfx_existing: usize,
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
        }
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
            if let Some(seg) = segments.get(*seg_idx)
                && let Some(event) = seg.events.get(*event_idx)
                && let Event::Sticker { keyword, name, .. } = event
                && !keyword.trim().is_empty()
            {
                match source.as_str() {
                            "emoji" => {
                                // 优先下载 process 已解析的具体 codepoint (去重可能选了非首个),
                                // 无则按 keyword 搜索首个
                                let codepoints = match name.as_deref() {
                                    Some(n) if !n.is_empty() && n.chars().all(|c| c.is_ascii_hexdigit()) => vec![n.to_string()],
                                    _ => vec![],
                                };
                                let (new_files, existing) = if codepoints.is_empty() {
                                    super::emoji_search::download_images(
                                        &[keyword.clone()], &sticker_dir, &self.client, 1,
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
                                    source, name.as_deref(), &sticker_dir, &mut report,
                                );
                                report.sticker_downloaded += copied;
                            }
                        }
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
        let sticker_dir = yekitrim_core::path::sticker_dir();
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
        match self.download_file(&url, &path).await {
            Err(_) => {
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
            Ok(_) => {}
        }
        // 下载成功路径
        println!("  ✓ sfx {} → {}", id, path.display());
        1
    }

    /// 生成资产路径映射 (注入到 config JSON)
    /// 扫描 assets/stickers/ 目录, 返回 stem → URL 映射
    pub fn asset_map(&self) -> serde_json::Value {
        let sticker_dir = self.project_path.join("assets").join("stickers");
        let mut sticker_map = serde_json::Map::new();
        if sticker_dir.exists()
            && let Ok(entries) = std::fs::read_dir(&sticker_dir) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        let stem = Path::new(name).file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or(name);
                        sticker_map.insert(stem.to_string(), serde_json::Value::String(
                            format!("/assets/stickers/{}", name)
                        ));
                    }
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
