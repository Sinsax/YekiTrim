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
            .user_agent("vtrim-polish/0.1")
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
                            // 非 emoji/irasu 的来源 = 贴纸包名（如 ysyk），复制包内文件
                            _ => {
                                let copied = self.copy_sticker_from_local(source, &sticker_dir);
                                report.sticker_downloaded += copied;
                            }
                        }
            }
        }
        report
    }

    /// 从程序目录的 stickers/ 复制匹配的贴纸文件
    /// 新结构：keyword = 贴纸包名（如 ysyk），复制包内全部文件。
    /// 旧结构兜底：按文件名/tags 匹配后复制整组。
    fn copy_sticker_from_local(&self, keyword: &str, dest_dir: &Path) -> usize {
        let sticker_dir = vtrim_core::path::sticker_dir();
        if !sticker_dir.exists() {
            return 0;
        }

        let mut copied = 0;

        // 新结构：贴纸包目录直接复制
        let packages = super::sticker_index::scan_packages(&sticker_dir);
        if let Some(pkg) = packages.iter().find(|p| p.name == keyword) {
            for path in &pkg.files {
                let Some(fname) = path.file_name() else { continue };
                let dest = dest_dir.join(fname);
                if dest.exists() {
                    continue;
                }
                if std::fs::copy(path, &dest).is_ok() {
                    copied += 1;
                }
            }
            if copied > 0 {
                return copied;
            }
        }

        // 旧结构兜底：根目录直接放文件，评分匹配后复制整组
        let idx = super::sticker_index::StickerIndex::scan(&sticker_dir);
        let Some(m) = idx.match_keyword(keyword) else {
            return 0;
        };
        for path in idx.groups()[m.group_index].all_files() {
            let Some(fname) = path.file_name() else { continue };
            let dest = dest_dir.join(fname);
            if dest.exists() {
                continue;
            }
            if std::fs::copy(path, &dest).is_ok() {
                copied += 1;
            }
        }
        copied
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
        Event::ZoomBounce { sfx: Some(s), .. } => Some(s),
        Event::Vignette { sfx: Some(s), .. } => Some(s),
        Event::HueShift { sfx: Some(s), .. } => Some(s),
        Event::Stutter { sfx: Some(s), .. } => Some(s),
        Event::Split { sfx: Some(s), .. } => Some(s),
        Event::Noise { sfx: Some(s), .. } => Some(s),
        Event::Pulse { sfx: Some(s), .. } => Some(s),
        Event::ColorShift { sfx: Some(s), .. } => Some(s),
        Event::Overlay { sfx: Some(s), .. } => Some(s),
        _ => None,
    }
}
