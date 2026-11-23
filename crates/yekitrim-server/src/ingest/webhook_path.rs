//! 容器路径 → 本机路径的换算，以及「一次录制完成」的产物面：视频 / 伴生弹幕 / 标题 / 建工程。
//!
//! ⚠️ `map_webhook_path` 是录播姬**容器根**与**宿主机根**两个视角的唯一换算点（`api/rooms.rs` 也用它）：
//! 路径逃逸（`..` / 绝对路径 / 跳出根）必须挡在这里。

use std::path::Path;
use std::path::PathBuf;

use axum::http::StatusCode;
use yekitrim_service::ingest::landing;

use super::*;
use super::webhook_event::WebhookCompletion;
use super::channel_map::first_admin;

/// 一条已经落地的"录像 + 弹幕"。
#[derive(Debug, Clone)]
pub(super) struct ResolvedWebhook {
    // ⚠️ 字段也要 `pub(super)`：`custom_webhook`（webhook.rs）读它 —— 拆文件后
    // `super` 从 crate 根变成了 `crate::ingest`，私有字段跨不了同级模块。
    pub(super) project_root: PathBuf,
    pub(super) input_name: String,
    pub(super) danmaku_name: Option<String>,
    pub(super) title: String,
    pub(super) room: Option<String>,
    pub(super) platform: Option<String>,
    pub(super) video_display: String,
}

impl ResolvedWebhook {
    fn source_label(&self) -> String {
        let platform = self.platform.as_deref().unwrap_or("custom");
        match self.room.as_deref().filter(|s| !s.is_empty()) {
            Some(room) => format!("Webhook · {platform} · 房间 {room}"),
            None => format!("Webhook · {platform}"),
        }
    }
}

/// 通用 webhook 接受的录像容器（小写、不含点）。
///
/// 录播姬/B站下载落盘的都是这几个；`-后处理.mp4` 优先之后仍然是 mp4。
/// 其余扩展名一律 400（见 `resolve_webhook_completion`）。
const WEBHOOK_VIDEO_EXTS: &[&str] = &[
    "mp4", "flv", "mkv", "ts", "mov", "webm", "avi", "m4v", "wmv", "mpg", "mpeg",
];

/// 把事件里的路径解析成项目内的真实文件。
///
/// - `container_root` 非空且命中时先剥前缀（biliLive-tools 的 `/app/video/...`）；
/// - 其余相对/绝对路径都走 `tenant::ensure_within` —— **唯一**的越界判据；
/// - 同目录的 `-后处理.mp4` 优先（ffmpeg 转封装后的版本更通用）。
pub(super) fn resolve_webhook_completion(
    cfg: &yekitrim_core::config::Config,
    c: &WebhookCompletion,
) -> Result<ResolvedWebhook, String> {
    let root = cfg.ingest.webhook.root.trim();
    if root.is_empty() {
        return Err("通用 webhook 未配置 [ingest.webhook].root".into());
    }
    let root_path = PathBuf::from(root);
    if !root_path.is_dir() {
        return Err(format!("[ingest.webhook].root 不是目录: {}", root_path.display()));
    }
    let container = cfg.ingest.webhook.container_root.trim();
    let original = map_webhook_path(&root_path, container, &c.file)?;
    let video = prefer_postprocessed(&original);
    // ⚠️ 只接受**录像容器**：事件里的路径是一个已存在的普通文件就建工程的话，
    // 把 `.xml` / `.jpg` / `.txt` 推过来会建出一个坏工程（后续转写/剪切报错，还可能白烧 ASR 额度）。
    let ext = video
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !WEBHOOK_VIDEO_EXTS.contains(&ext.as_str()) {
        return Err(format!(
            "不像录像文件（扩展名 {:?}）：只接受 {}",
            ext,
            WEBHOOK_VIDEO_EXTS.join(" / ")
        ));
    }

    let danmaku = match c
        .danmaku
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(d) => Some(map_webhook_path(&root_path, container, d)?),
        // 弹幕按**原始文件名**找：`-后处理.mp4` 的 stem 多了后缀，拿它找 `rec.xml` 会落空。
        None => find_danmaku(&original).or_else(|| find_danmaku(&video)),
    };
    if let Some(dm) = &danmaku {
        if dm.parent() != video.parent() {
            return Err(format!(
                "弹幕必须与视频同目录（或省略 danmaku 让服务端自动找同名）：{}",
                dm.display()
            ));
        }
    }

    let parent = video
        .parent()
        .ok_or_else(|| "无法确定项目目录".to_string())?;
    let input_name = file_name(&video)?;
    let danmaku_name = danmaku.as_ref().map(|d| file_name(d)).transpose()?;
    let title = c
        .title
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            parent
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .or_else(|| video.file_stem().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "Webhook 录像".to_string());
    Ok(ResolvedWebhook {
        project_root: parent.to_path_buf(),
        input_name,
        danmaku_name,
        title: sanitize_title(&title),
        room: c.room.clone().filter(|s| !s.trim().is_empty()),
        platform: c.platform.clone().filter(|s| !s.trim().is_empty()),
        video_display: video.display().to_string(),
    })
}

pub(crate) fn map_webhook_path(root: &Path, container_root: &str, raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains('\0') {
        return Err("路径为空或含非法字符".into());
    }
    let mut candidate = raw.to_string();
    if !container_root.is_empty() {
        if let Some(rest) = raw.strip_prefix(container_root) {
            if rest.is_empty() {
                return Err("容器路径只有前缀，没有文件名".into());
            }
            if !(rest.starts_with('/') || rest.starts_with('\\')) {
                return Err(format!("容器路径前缀边界不匹配: {raw}"));
            }
            candidate = rest.trim_start_matches(['/', '\\']).to_string();
        }
    }
    let abs = yekitrim_core::tenant::ensure_within(root, Path::new(&candidate))
        .map_err(|e| format!("路径不在 webhook.root 内: {e:#}"))?;
    if !abs.is_file() {
        return Err(format!("文件不存在或不是普通文件: {}", abs.display()));
    }
    Ok(abs)
}

fn prefer_postprocessed(video: &Path) -> PathBuf {
    let Some(stem) = video.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
        return video.to_path_buf();
    };
    let Some(parent) = video.parent() else {
        return video.to_path_buf();
    };
    let candidate = parent.join(format!("{stem}-后处理.mp4"));
    if candidate.is_file() {
        candidate
    } else {
        video.to_path_buf()
    }
}

pub(super) fn find_danmaku(video: &Path) -> Option<PathBuf> {
    let stem = video.file_stem()?.to_string_lossy().into_owned();
    let dir = video.parent()?;
    for ext in ["xml", "ass", "srt"] {
        let cand = dir.join(format!("{stem}.{ext}"));
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

pub(super) fn file_name(p: &Path) -> Result<String, String> {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| format!("文件没有文件名: {}", p.display()))
}

fn sanitize_title(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string();
    let out: String = cleaned.chars().take(80).collect();
    if out.is_empty() {
        "Webhook 录像".to_string()
    } else {
        out
    }
}

pub(super) struct PreparedWebhook {
    pub(super) project_id: i64,
    pub(super) config_path: PathBuf,
    pub(super) duplicate: bool,
    pub(super) owner: i64,
    pub(super) pipeline: String,
}

/// 建项目 + 登记（同步部分；调用方已经在 blocking 池里）。
pub(super) fn prepare_webhook_project(
    db: &yekitrim_service::db::Db,
    cfg: &yekitrim_core::config::Config,
    r: &ResolvedWebhook,
) -> Result<PreparedWebhook, AppError> {
    let config_path = r.project_root.join(yekitrim_core::path::PROJECT_CONFIG_FILE);
    if let Some(p) = db
        .project_by_config(&config_path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("查询项目失败: {e:#}")))?
    {
        return Ok(PreparedWebhook {
            project_id: p.id,
            config_path,
            duplicate: true,
            owner: p.owner_id,
            pipeline: "none".into(),
        });
    }
    if config_path.is_file() {
        return Err(err(
            StatusCode::CONFLICT,
            format!(
                "目录已有 {} 但不是已登记项目；请用「添加项目」入口，别让 webhook 覆盖它",
                yekitrim_core::path::PROJECT_CONFIG_FILE
            ),
        ));
    }
    let owner = first_admin(db)?;
    let label = r.source_label();
    let landed = landing::land_project(
        db,
        &landing::ProjectLanding {
            owner_id: owner,
            name: r.title.clone(),
            source_kind: "webhook",
            source_label: Some(label.as_str()),
            room: r.room.as_deref(),
            project_root: &r.project_root,
            input_file: &r.input_name,
            danmaku_file: r.danmaku_name.as_deref(),
            session_id: None,
        },
    )
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建项目失败: {e:#}")))?;
    Ok(PreparedWebhook {
        project_id: landed.project_id,
        config_path: landed.config_path,
        duplicate: !landed.created,
        owner,
        pipeline: cfg.ingest.webhook.auto_pipeline.clone(),
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    
    use crate::ingest::webhook_event::WebhookCompletion;

    #[test]
    fn webhook_maps_container_path_and_prefers_postprocessed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let session = root.join("Bilibili").join("a").join("[2026-10-29 16-00-00-000][t]");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("rec.flv"), b"flv").unwrap();
        std::fs::write(session.join("rec-后处理.mp4"), b"mp4").unwrap();
        std::fs::write(session.join("rec.xml"), b"<i/>").unwrap();

        let mut cfg = yekitrim_core::config::Config::default();
        cfg.ingest.webhook.root = root.to_string_lossy().into_owned();
        cfg.ingest.webhook.container_root = "/app/video".into();
        let completion = WebhookCompletion {
            file: "/app/video/Bilibili/a/[2026-10-29 16-00-00-000][t]/rec.flv".into(),
            danmaku: None,
            room: Some("123456".into()),
            platform: Some("Bilibili".into()),
            title: None,
        };
        let r = resolve_webhook_completion(&cfg, &completion).unwrap();
        assert_eq!(r.project_root, session);
        assert_eq!(r.input_name, "rec-后处理.mp4");
        assert_eq!(r.danmaku_name.as_deref(), Some("rec.xml"));
        assert!(r.source_label().contains("123456"));
    }
    #[test]
    fn webhook_rejects_path_escape() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("Bilibili")).unwrap();
        let mut cfg = yekitrim_core::config::Config::default();
        cfg.ingest.webhook.root = root.to_string_lossy().into_owned();
        cfg.ingest.webhook.container_root = "/app/video".into();
        let completion = WebhookCompletion {
            file: "/app/video/../outside.mp4".into(),
            danmaku: None,
            room: None,
            platform: None,
            title: None,
        };
        assert!(resolve_webhook_completion(&cfg, &completion).is_err());
    }
}
