use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use cli::{stage_name, Cli, Command, ConfigAction};
use tracing_subscriber::EnvFilter;
use vtrim_core::config::{self, Config};
use vtrim_core::pipeline::STAGE_CUT;


use crate::progress::IndicatifProgress;

// (GLOBAL_CONFIG_TOML / LLM_CONFIG_TOML 模板已移入 vtrim_core::config)

mod cli;
mod progress;
mod server;
mod service;

/// 订阅者 = 终端 + 进程内日志总线（设置页「运行日志」/ 日志文件）。
///
/// `LogBusLayer` 必须挂上：`vtrim server` 的访问日志、任务日志、错误文案都靠它
/// 进面板与文件；对纯 CLI 命令它只是多一份内存环形缓冲，没有副作用。
fn setup_tracing() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // 全局过滤器（RUST_LOG，默认 info）：三个层都受它约束，
    // 想看高频接口的逐条访问日志用 `RUST_LOG=vtrim=debug`
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(server::logbus::LogBusLayer::new(server::logbus::bus()))
        .init();
}

fn load_cfg(
    cli_config: &Option<PathBuf>,
) -> anyhow::Result<Config> {
    if let Some(path) = cli_config {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    } else {
        config::load_config()
    }
}

fn config_path(cli_config: &Option<PathBuf>) -> PathBuf {
    cli_config.clone().unwrap_or_else(|| PathBuf::from("vtrim.toml"))
}

fn percent_encode_component(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::new();
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 15) as usize] as char);
            }
        }
    }
    out
}

/// 尝试把 polish clip 目录定位到所属 vtrim 工程，并启动/打开同一 WebUI 精修页。
/// 找不到工程时回退旧的独立 preview，保证命令仍然可用。
async fn run_polish_preview(input: &std::path::Path, mode: Option<&str>) -> anyhow::Result<()> {
    use crate::server::ServerArgs;

    let abs = if input.is_absolute() {
        input.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(input)
    };
    let mut cursor = Some(abs.as_path());
    let mut cfg_file = None;
    while let Some(dir) = cursor {
        let cand = dir.join("vtrim.toml");
        if cand.exists() {
            cfg_file = Some(cand);
            break;
        }
        cursor = dir.parent();
    }
    let Some(cfg) = cfg_file else {
        tracing::info!("未找到 clip 所属的 vtrim.toml，回退到旧版独立 polish preview");
        vtrim_polish::commands::cmd_preview(input, mode.unwrap_or("")).await?;
        return Ok(());
    };

    let cfg_str = percent_encode_component(&cfg.to_string_lossy());
    let dir_str = percent_encode_component(&abs.to_string_lossy());
    let open_path = format!("/#/project/{}/polish?dir={}", cfg_str, dir_str);
    let args = ServerArgs {
        host: "127.0.0.1".to_string(),
        port: crate::service::doctor::DEFAULT_PORT,
        open: true,
        config: Some(cfg),
        allow_remote: false,
        trust_proxy: false,
        data_dir: None,
        admin_password_file: None,
        force: false,
        open_path,
    };
    crate::server::run(args).await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    setup_tracing();

    let cli = Cli::parse();
    let global_config = &cli.config;

    let progress: Arc<dyn vtrim_core::progress::ProgressReporter> = Arc::new(IndicatifProgress::new());

    match &cli.command {
        Command::Init {
            input,
            danmaku,
            output,
            config: cfg_path,
        } => {
            let out = cfg_path.clone().or_else(|| {
                input.parent().map(|p| p.join("vtrim.toml"))
            }).unwrap_or_else(|| PathBuf::from("vtrim.toml"));
            crate::service::init_project(
                input,
                danmaku.as_deref(),
                output.as_deref().map(|d| d.to_string_lossy().to_string()).as_deref(),
                &out,
            )?;
            tracing::info!("视频工程已初始化: {}", input.display());
            tracing::info!("配置文件: {}", out.display());
        }

Command::Extract { input, output } => {
            let config_file = config_path(global_config);
            let input_override = input.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            let output_override = output.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            crate::service::run_stages_with_ext(
                &config_file,
                &["extract".to_string()],
                false,
                true,
                input_override,
                output_override,
                None,
                None,
                Some(progress.clone()),
            ).await?;
            tracing::info!("音频提取完成");
        }

Command::Process {
            input,
            output: output_dir,
            dry_run,
        } => {
            crate::service::usage::try_install_cli_sink();
            let _cfg = load_cfg(global_config)?;
            let input_override = input.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            let output_override = output_dir.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            let config_file = global_config.clone().unwrap_or_else(|| PathBuf::from("vtrim.toml"));
            let stages = vec!["process".to_string()];
            crate::service::run_stages_with(
                &config_file,
                &stages,
                *dry_run,
                true,
                input_override,
                output_override,
                Some(progress.clone()),
            ).await?;
            tracing::info!("处理完成!");
        }

Command::Transcribe {
            input,
            output,
            ..
        } => {
            let config_file = config_path(global_config);
            let input_override = input.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            let output_override = output.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            crate::service::run_stages_with_ext(
                &config_file,
                &["transcribe".to_string()],
                false,
                true,
                input_override,
                output_override,
                None,
                None,
                Some(progress.clone()),
            ).await?;
            tracing::info!("语音转文字完成");
        }

Command::Detect { transcript } => {
            crate::service::usage::try_install_cli_sink();
            let config_file = config_path(global_config);
            let transcript_override = transcript.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            crate::service::run_stages_with_ext(
                &config_file,
                &["detect".to_string()],
                false,
                true,
                None,
                None,
                transcript_override,
                None,
                Some(progress.clone()),
            ).await?;
            tracing::info!("事件检测完成");
        }

Command::Cut {
            input,
            events,
            output,
        } => {
            let config_file = config_path(global_config);
            let input_override = input.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            let output_override = output.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            let events_override = events.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            crate::service::run_stages_with_ext(
                &config_file,
                &["cut".to_string()],
                false,
                true,
                input_override,
                output_override,
                None,
                events_override,
                Some(progress.clone()),
            ).await?;
            tracing::info!("剪切完成");
        }

Command::User(action) => {
            use cli::UserAction;
            let src = |stdin: bool, file: &Option<PathBuf>| crate::service::source_from(stdin, file.clone());
            match action {
                UserAction::List => crate::service::cmd_list()?,
                UserAction::Info => crate::service::cmd_info()?,
                UserAction::Add { username, role, stdin, password_file } => {
                    crate::service::cmd_add(username, role, src(*stdin, password_file))?
                }
                UserAction::Passwd { username, stdin, password_file } => {
                    crate::service::cmd_passwd(username, src(*stdin, password_file))?
                }
                UserAction::Check { username, stdin, password_file } => {
                    crate::service::cmd_check(username, src(*stdin, password_file))?
                }
                UserAction::Disable { username, enable } => {
                    crate::service::cmd_disable(username, *enable)?
                }
                UserAction::InitAdmin { stdin, password_file } => {
                    crate::service::cmd_init_admin(src(*stdin, password_file))?
                }
                UserAction::ResetAdminPassword { stdin, password_file } => {
                    crate::service::cmd_reset_admin_password(src(*stdin, password_file))?
                }
                UserAction::Invite { note, days, count, disable } => {
                    crate::service::invite::cmd_invite(note.clone(), *days, *count, *disable)?
                }
                UserAction::Invites => crate::service::invite::cmd_invites()?,
                UserAction::InviteRevoke { target } => {
                    crate::service::invite::cmd_invite_revoke(target)?
                }
            }
        }

        Command::Ingest(action) => {
            use cli::IngestAction;
            match action {
                IngestAction::Token { disable } => crate::service::ingest::cmd_token(*disable)?,
                IngestAction::Status => crate::service::ingest::cmd_status()?,
            }
        }

Command::Doctor { url } => {
            crate::service::cmd_doctor(url)?;
        }

        Command::Usage { days } => {
            crate::service::usage::cmd_usage(*days)?;
        }

Command::Server(args) => {
            server::run(args.clone()).await?;
        }

        Command::Status => {
            let config_file = config_path(global_config);
            let ws = crate::service::Workspace::load(Some(&config_file))?;
            let snap = crate::service::project_snapshot(&ws);
            let input = snap.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
            if !input.is_empty() { println!("视频工程: {}", input); }
            let current = snap.get("current_stage").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            println!("当前进度: Stage #{}", current);
            println!();
            for num in 1..=STAGE_CUT {
                let status = if num < current { "✓" } else if num == current { "→" } else { "·" };
                println!("  [{}] {:<20} {}", status, stage_name(num), status);
            }
            let output = ws.output_dir();
            let transcript_json = output.join("transcript.json");
            if transcript_json.exists() {
                if let Ok(content) = std::fs::read_to_string(&transcript_json) {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(meta) = value.get("asr_metadata") {
                            println!(); println!("ASR 元数据:");
                            if let Some(model) = meta.get("model").and_then(|m| m.as_str()) { println!("  模型: {}", model); }
                            if let Some(anchors) = meta.get("anchors").and_then(|a| a.as_u64()) { println!("  锚点数: {}", anchors); }
                            if let Some(cov) = meta.get("anchor_coverage").and_then(|c| c.as_f64()) { println!("  锚点覆盖率: {:.1}%", cov * 100.0); }
                            if let Some(span) = meta.get("max_interp_span").and_then(|s| s.as_u64()) { println!("  最大插值跨度: {}", span); }
                            if let Some(reason) = meta.get("fallback_reason").and_then(|r| r.as_str()) { println!("  降级原因: {}", reason); }
                        }
                    }
                }
            }
        }

Command::Config(action) => match action {
            ConfigAction::Init { path } => {
                crate::service::init_global_config(path.as_deref())?;
            }
            ConfigAction::Show => {
                println!("{}", crate::service::show_config(global_config.as_deref())?);
            }
        },

Command::Silence { files, duration, threshold } => {
            crate::service::run_silence(files, *duration, *threshold).await?;
        }

Command::Download => {
            crate::service::download_models().await?;
        }

Command::Correct { transcript } => {
            crate::service::usage::try_install_cli_sink();
            let config_file = config_path(global_config);
            let transcript_override = transcript.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            crate::service::run_correct(&config_file, transcript_override, true).await?;
            tracing::info!("LLM 订正完成");
        }

Command::Polish(action) => {
            use vtrim_polish::{commands, cover, process, render};
            crate::service::usage::try_install_cli_sink();

            match action {
                cli::PolishAction::Init { input } => {
                    commands::cmd_init(input).await?;
                }
                cli::PolishAction::Llm { input } => {
                    commands::cmd_llm(input).await?;
                }
                cli::PolishAction::Process { input } => {
                    process::cmd_process(input).await?;
                }
                cli::PolishAction::Render { input, renderer } => {
                    std::env::set_var("VTEDIT_RENDERER", renderer);
                    render::cmd_render(input).await?;
                }
                cli::PolishAction::Cover { input, template } => {
                    if let Some(t) = template {
                        if !t.is_empty() {
                            cover::cmd_cover_template(input, t).await?;
                        }
                    }
                    cover::cmd_cover(input, None).await?;
                }
                cli::PolishAction::Preview { input, mode } => {
                    run_polish_preview(input, mode.as_deref()).await?;
                }
                cli::PolishAction::Doctor { input } => {
                    commands::cmd_doctor(input)?;
                }
                cli::PolishAction::Test { input } => {
                    commands::cmd_test(input).await?;
                }
                cli::PolishAction::Stickers { action } => {
                    match action {
                        cli::StickerAction::List => commands::cmd_stickers_list()?,
                        cli::StickerAction::Test { keywords } => {
                            commands::cmd_stickers_test(keywords)?
                        }
                        cli::StickerAction::Scan { llm } => {
                            commands::cmd_stickers_scan(*llm).await?
                        }
                        cli::StickerAction::Build { package } => {
                            commands::cmd_stickers_build(package).await?
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
