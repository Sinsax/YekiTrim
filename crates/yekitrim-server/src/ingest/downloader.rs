//! 下载器（yt-dlp）的安装与状态：`POST /api/admin/downloader/install`、`GET /api/ingest/downloader`。
//!
//! 版本与 sha256 钉死在 `yekitrim_service::downloader`；本文件只做 admin 编排与状态回显。

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;

use crate::auth::CurrentUser;
use crate::model::AppState;

use super::*;

/// `POST /api/admin/downloader/install`：下载内置 yt-dlp 并校验 sha256（仅 admin）。
///
/// ⚠️ **下载必须跑在 `tokio::spawn` 里，不能直接 `await` 在 handler 上**（2026-11-12 事故）：
/// axum 的 handler future 会在**客户端断开时被丢弃**，连带把下载一起取消 —— 40 MB 的包
/// 在慢网络上要几十分钟，用户一刷新 / 一切标签页 / 代理一超时，下载就死在中途，
/// 界面上表现为"一直转圈然后失败"，而 `.part` 停在那里不动。
/// `tokio::spawn` 出来的任务**不随 handler 一起 drop**，断了也照下完。
pub async fn downloader_install(
    State(_state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
) -> ApiResult {
    if !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "仅管理员可安装下载器"));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let cfg = yekitrim_core::config::load_global_config();
        let r = yekitrim_service::downloader::registry::ensure_ytdlp(&cfg).await;
        let _ = tx.send(r);
    });
    let path = match rx.await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            return Err(err(
                StatusCode::BAD_GATEWAY,
                format!(
                    "安装下载器失败: {e:#}\n\n{}",
                    yekitrim_service::downloader::registry::install_hint()
                ),
            ))
        }
        // 发信号的 task 挂了：下载本身不受影响（它在自己的 task 里），让用户稍后刷新
        Err(_) => {
            return Err(err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "安装任务异常结束；下载可能仍在后台进行，稍后点「刷新」查看",
            ))
        }
    };
    let cfg = yekitrim_core::config::load_global_config();
    // ⚠️ 只回版本与是否通过哈希校验，**不回路径**（与 §3.5.9 一致）
    let (verified, source) = match yekitrim_service::downloader::registry::find_ytdlp(&cfg) {
        Some(i) => {
            let s = i.source.label();
            (i.verified, s)
        }
        None => (false, "未知"),
    };
    ok(serde_json::json!({
        "ok": true,
        "path_present": path.is_file(),
        "version": yekitrim_service::downloader::registry::VERSION,
        "verified": verified,
        "source": source,
    }))
}

/// `GET /api/ingest/downloader`：下载器就绪位（任何登录用户可看，供前端引导卡）。
pub async fn downloader_status(
    State(_state): State<Arc<AppState>>,
) -> ApiResult {
    let cfg = yekitrim_core::config::load_global_config();
    let found = yekitrim_service::downloader::registry::find_ytdlp(&cfg);
    let kind = yekitrim_service::downloader::DownloaderKind::parse(&cfg.ingest.downloader);
    // 安装进度（前端轮询画进度条）：托管目录里 `.part*` 的当前字节 vs 钉死的总大小
    let mut part_bytes: u64 = 0;
    let bin_dir = yekitrim_service::downloader::registry::managed_dir();
    if let Ok(rd) = std::fs::read_dir(&bin_dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with("yt-dlp.part") {
                part_bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    // 同样**不含路径**：这个响应给非管理员看
    ok(serde_json::json!({
        "ok": true,
        "kind": kind.label(),
        "ready": found.is_some(),
        "verified": found.map(|i| i.verified).unwrap_or(false),
        "pinned_version": yekitrim_service::downloader::registry::VERSION,
        "license": yekitrim_service::downloader::registry::LICENSE,
        "part_bytes": part_bytes,
        "total_bytes": yekitrim_service::downloader::registry::pin().size,
    }))
}

