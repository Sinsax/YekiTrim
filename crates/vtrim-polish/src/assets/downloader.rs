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
        assignments: &[(usize, String)], // (segment_idx, source) — 预分配的来源
    ) -> DownloadReport {
        let sticker_dir = self.project_path.join("assets").join("stickers");
        std::fs::create_dir_all(&sticker_dir).ok();
        let mut report = DownloadReport::default();

        // 收集每位 sticker 事件的 keyword
        for (seg_idx, source) in assignments {
            if let Some(seg) = segments.get(*seg_idx) {
                for event in &seg.events {
                    if let Event::Sticker { keyword, .. } = event {
                        match source.as_str() {
                            "emoji" => {
                                let keywords = vec![keyword.clone()];
                                let (new_files, existing) = super::emoji_search::download_images(
                                    &keywords, &sticker_dir, &self.client, 1,
                                ).await;
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
                            "local" => {
                                let copied = self.copy_sticker_from_local(keyword, &sticker_dir);
                                report.sticker_downloaded += copied;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        report
    }

    /// 从程序目录的 stickers/ 复制匹配的贴纸文件
    fn copy_sticker_from_local(&self, keyword: &str, dest_dir: &Path) -> usize {
        let sticker_dir = vtrim_core::path::sticker_dir();
        if !sticker_dir.exists() {
            return 0;
        }
        let kw_lower = keyword.to_lowercase();
        let entries = match std::fs::read_dir(&sticker_dir) {
            Ok(e) => e.flatten().collect::<Vec<_>>(),
            Err(_) => return 0,
        };

        // 找 keyword 包含在文件名中的第一个文件
        for entry in &entries {
            if let Some(name) = entry.file_name().to_str() {
                let stem = Path::new(name).file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                if stem.to_lowercase().contains(&kw_lower) || kw_lower.contains(stem) {
                    let dest = dest_dir.join(name);
                    if !dest.exists() {
                        if std::fs::copy(entry.path(), &dest).is_ok() {
                            return 1;
                        }
                    }
                }
            }
        }
        0
    }

    /// 按需下载 segments 中引用的 SFX
    pub async fn download_sfx_for_segments(&self, segments: &[Segment]) -> DownloadReport {
        let mut report = DownloadReport::default();
        let sfx_dir = self.project_path.join("sfx");
        std::fs::create_dir_all(&sfx_dir).ok();

        let ids = collect_sfx_ids(segments);
        if ids.is_empty() { return report; }

        println!("  ── SFX 按需下载 ──");
        let base_url = "https://sounddictionary.info";
        for id in &ids {
            // tag: 前缀 → 取第1个匹配下载（不再展开全部）
            if let Some(tag_name) = id.strip_prefix("tag:") {
                let matches = super::sfx_catalog::search_by_tags(&[tag_name.to_string()]);
                let entry = match matches.first() {
                    Some(e) => e,
                    None => {
                        report.errors.push(format!("sfx tag「{}」: 未匹配到音效", tag_name));
                        println!("  ⚠ sfx tag「{}」: 未匹配到音效", tag_name);
                        continue;
                    }
                };
                let n = self.download_one_sfx(&sfx_dir, base_url, &entry.id, &mut report).await;
                report.sfx_downloaded += n;
                println!("  ✓ sfx tag「{}」→ {}", tag_name, entry.id);
            } else {
                let n = self.download_one_sfx(&sfx_dir, base_url, id, &mut report).await;
                report.sfx_downloaded += n;
            }
        }
        report
    }

    /// 下载单个 SFX 文件，返回是否新下载（1=新下载, 0=已存在）
    async fn download_one_sfx(&self, sfx_dir: &Path, base_url: &str, id: &str, report: &mut DownloadReport) -> usize {
        let path = sfx_dir.join(format!("{}.mp3", id));
        if path.exists() {
            report.sfx_existing += 1;
            return 0;
        }
        let entry = super::sfx_catalog::lookup(id);
        let url = match entry {
            Some(e) => format!("{}{}", base_url, e.url),
            None => {
                report.errors.push(format!("sfx {}: 未在 catalog 中找到，请检查 sfx_catalog.json", id));
                println!("  ⚠ sfx {}: 未在 catalog 中找到", id);
                return 0;
            }
        };
        match self.download_file(&url, &path).await {
            Ok(_) => {
                println!("  ✓ sfx {} → {}", id, path.display());
                1
            }
            Err(e) => {
                report.errors.push(format!("sfx {}: {}", id, e));
                println!("  ⚠ sfx {} download failed: {}", id, e);
                0
            }
        }
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
