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
    pub emoji_downloaded: usize,
    pub irasu_downloaded: usize,
    pub sfx_downloaded: usize,
    pub emoji_existing: usize,
    pub irasu_existing: usize,
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

    /// 按需搜索下载 irasutoya 插图
    pub async fn download_irasu_keywords(&self, segments: &[Segment]) -> DownloadReport {
        let mut report = DownloadReport::default();
        let irasu_dir = self.project_path.join("assets").join("irasu");
        std::fs::create_dir_all(&irasu_dir).ok();

        use crate::project::Event;
        let mut tried: Vec<String> = vec![];

        for seg in segments {
            for event in &seg.events {
                if let Event::Irasu { keywords: Some(kws), .. } = event {
                    for kw in kws.iter() {
                        let kw = kw.trim();
                        if kw.is_empty() || tried.contains(&kw.to_string()) { continue; }
                        tried.push(kw.to_string());

                        let (new_files, existing) = super::irasu_search::download_images(kw, &irasu_dir, &self.client, 1).await;
                        report.irasu_downloaded += new_files.len();
                        report.irasu_existing += existing;

                        if !new_files.is_empty() {
                            break; // 当前事件已命中，跳过剩余 keywords
                        }
                    }
                }
            }
        }
        report
    }

    /// 按需搜索下载 Noto emoji
    pub async fn download_emoji_keywords(&self, segments: &[Segment]) -> DownloadReport {
        let mut report = DownloadReport::default();
        let emoji_dir = self.project_path.join("assets").join("emoji");
        std::fs::create_dir_all(&emoji_dir).ok();

        let keywords = super::emoji_search::collect_keywords(segments);
        if keywords.is_empty() { return report; }

        println!("  ── emoji 搜索下载 ({} 个关键词) ──", keywords.len());
        let (new_files, existing) = super::emoji_search::download_images(&keywords, &emoji_dir, &self.client, 1).await;
        report.emoji_downloaded += new_files.len();
        report.emoji_existing += existing;
        report
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
    pub fn asset_map(&self) -> serde_json::Value {
        let assets_dir = self.project_path.join("assets");
        let emoji_dir = assets_dir.join("emoji");
        let irasu_dir = assets_dir.join("irasu");

        // 动态扫描 irasu 目录
        let mut irasu_map = serde_json::Map::new();
        if irasu_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&irasu_dir) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        let stem = Path::new(name).file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or(name);
                        irasu_map.insert(stem.to_string(), serde_json::Value::String(
                            format!("/assets/irasu/{}", name)
                        ));
                    }
                }
            }
        }

        let has_local = emoji_dir.exists() && std::fs::read_dir(&emoji_dir).map(|mut e| e.any(|f| f.is_ok())).unwrap_or(false);
        serde_json::json!({
            "emoji_base": if has_local { "/assets/emoji/" } else { "https://fonts.gstatic.com/s/e/notoemoji/latest" },
            "emoji_local": has_local,
            "irasu_map": irasu_map,
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
                    if let Some(sfx) = extract_sfx_field(event) {
                        if !sfx.is_empty() && !ids.contains(sfx) {
                            ids.push(sfx.clone());
                        }
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
        Event::Emoji { sfx: Some(s), .. } => Some(s),
        Event::Irasu { sfx: Some(s), .. } => Some(s),
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
