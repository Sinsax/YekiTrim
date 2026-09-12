//! Server 支持：axum HTTP + WebUI + JobManager。
//!
//! - `vtrim server` 子命令启动
//! - REST API 覆盖项目/状态/配置/数据/管线/精修
//! - SSE 推送任务日志与进度
//! - WebUI = `webui/dist`（Vue3 构建产物）；**不再内嵌单文件页面**，
//!   缺失时由 `web::webui_fallback` 返回 503 + 构建指引
//!
//! 设计文档: docs/superpowers/specs/2026-09-03-vtrim-webui-design.md

pub mod api;
pub mod jobs;
pub mod media;
pub mod model;
pub mod persist;
pub mod web;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::routing::get;
use clap::Parser;
use tokio::net::TcpListener;
use tracing::info;

use crate::service::Workspace;

/// `vtrim server` CLI 参数（由 main.rs 转发）。
#[derive(Debug, Clone, Parser)]
pub struct ServerArgs {
    /// 监听地址
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    /// 监听端口
    #[arg(long, default_value_t = 7788)]
    pub port: u16,
    /// 启动后自动打开浏览器
    #[arg(long)]
    pub open: bool,
    /// 项目配置文件路径
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// 允许监听非 loopback 地址。
    ///
    /// 服务无鉴权且可读写本机任意工程目录（并会调用 ffmpeg/Chrome），
    /// 默认只允许 127.0.0.1 / ::1；确需对外监听必须显式打开。
    #[arg(long)]
    pub allow_remote: bool,

    /// 打开浏览器时使用的初始路径（例如 hash 路由；CLI 内部使用）
    #[arg(long, hide = true, default_value = "/")]
    pub open_path: String,
}

/// 启动 HTTP server（阻塞直至监听关闭）。
pub async fn run(args: ServerArgs) -> anyhow::Result<()> {
    // 首次运行（没有程序配置）时自动生成默认配置；已有配置不会被覆盖
    match crate::service::ensure_global_config() {
        Ok(report) => {
            for path in &report.created {
                info!("未找到程序配置，已生成默认配置: {}", path.display());
            }
            for (path, backup, parse_err) in &report.repaired {
                info!("⚠ 程序配置解析失败，已备份并重建: {} → {}（原因: {parse_err}）",
                    path.display(), backup.display());
            }
        }
        // 配置目录不可写（如只读安装位置）不应阻止 server 启动，用默认值继续
        Err(e) => info!("⚠ 自动生成程序配置失败（将使用内建默认值）: {e:#}"),
    }

    let saved = persist::load_state();
    let workspace = if args.config.is_none() {
        // 未显式指定工程时，优先恢复上次记忆的工程
        match saved.as_ref().and_then(|s| s.last_config_file.clone()) {
            Some(cfg) if Path::new(&cfg).exists() => {
                info!("恢复上次工程: {}", cfg);
                Workspace::load(Some(Path::new(&cfg)))?
            }
            _ => Workspace::load(args.config.as_deref())?,
        }
    } else {
        Workspace::load(args.config.as_deref())?
    };
    info!("Workspace: {}", workspace.config_file.display());
    if workspace.config_file.exists() {
        let _ = vtrim_core::config::normalize_project_paths_in(&workspace.config_file);
    }

    // 若工作区根未持久化或目录失效，退回工程父目录
    let workspace_root: PathBuf = match saved.as_ref().and_then(|s| s.workspace_root.clone()) {
        Some(root) if Path::new(&root).is_dir() => PathBuf::from(root),
        _ => workspace
            .root_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| workspace.root_dir.clone()),
    };
    let mut server_state = persist::load_state().unwrap_or_default();
    server_state.last_config_file = Some(workspace.config_file.to_string_lossy().into_owned());
    server_state.workspace_root = Some(workspace_root.to_string_lossy().into_owned());
    let cfg_str = workspace.config_file.to_string_lossy().into_owned();
    if !server_state.projects.contains(&cfg_str) {
        server_state.projects.push(cfg_str);
    }
    persist::save_state(&server_state);
    info!("Workspace root: {}", workspace_root.display());

    let state = Arc::new(model::AppState::with_root(workspace, workspace_root)?);

    let router = api::router(state.clone());
    let app = Router::new()
        .nest("/api", router)
        .fallback(get(web::webui_fallback));

    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .with_context(|| format!("无法解析监听地址: {}:{}", args.host, args.port))?;
    let is_loopback = match addr.ip() {
        std::net::IpAddr::V4(v4) => v4.is_loopback(),
        std::net::IpAddr::V6(v6) => v6.is_loopback(),
    };
    if !is_loopback {
        if !args.allow_remote {
            anyhow::bail!(
                "拒绝监听非 loopback 地址 {addr}：本服务无鉴权，可读写本机任意工程目录并调用外部命令。\
                 如确需对外监听，请显式加上 --allow-remote"
            );
        }
        tracing::warn!("⚠ 正在监听非 loopback 地址 {addr}：接口无鉴权，请仅在可信网络中使用");
    }
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("端口 {} 已被占用？", addr))?;
    let actual = listener.local_addr()?;
    info!("WebUI: http://{actual}/");
    info!("API:   http://{actual}/api/health");
    info!("按 Ctrl+C 退出");

    if args.open {
        let url = format!("http://{actual}{}", args.open_path);
        open_browser(&url);
    }
    if !web::dist_ready() {
        tracing::warn!(
            "⚠ 未找到 WebUI 构建产物（webui/dist/index.html），页面将返回 503。\
             请在仓库根目录执行 `cd webui && npm install && npm run build`，\
             或把 webui/dist 放到可执行文件同级目录"
        );
    }

    axum::serve(listener, app).await?;
    Ok(())
}

pub(crate) fn open_browser(url: &str) {
    let opened = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    match opened {
        Ok(_) => info!("已打开浏览器: {url}"),
        Err(e) => info!("无法自动打开浏览器: {e}"),
    }
}
