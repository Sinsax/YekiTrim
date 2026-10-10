//! yekitrim-cli 入口：装配 tracing 订阅者，把 clap 命令分发到各实现（二进制名 `yekitrim`）。
//!
//! 入口：`#[tokio::main] async fn main()`、`tracing_setup::setup`、`load_cfg`；子模块 `cli`（命令定义）、`progress`（终端进度条）。
//! 边界：这里只剩入口装配 —— CLI 要用服务端能力只能经 `yekitrim-server` 暴露的 `run()` / `ServerArgs` 进入，别在本地重实现领域逻辑 —— 见 `crates/yekitrim-cli/AGENTS.md`

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use cli::{stage_name, Cli, Command, ConfigAction};
use yekitrim_core::config::{self, Config};
use yekitrim_core::pipeline::STAGE_CUT;


use crate::progress::IndicatifProgress;

// (GLOBAL_CONFIG_TOML / LLM_CONFIG_TOML 模板已移入 yekitrim_core::config)

mod cli;
mod progress;
// 与 tray.rs 共用同一份 tracing 装配（各 bin 各自 `mod` 引同一个文件）
mod tracing_setup;

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
    cli_config.clone().unwrap_or_else(|| PathBuf::from(yekitrim_core::path::PROJECT_CONFIG_FILE))
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

/// 尝试把 polish clip 目录定位到所属 yekitrim 工程，并启动/打开同一 WebUI 精修页。
/// 找不到工程时回退旧的独立 preview，保证命令仍然可用。
async fn run_polish_preview(input: &std::path::Path, mode: Option<&str>) -> anyhow::Result<()> {
    use yekitrim_server::ServerArgs;

    let abs = if input.is_absolute() {
        input.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(input)
    };
    let mut cursor = Some(abs.as_path());
    let mut cfg_file = None;
    while let Some(dir) = cursor {
        let cand = dir.join(yekitrim_core::path::PROJECT_CONFIG_FILE);
        if cand.exists() {
            cfg_file = Some(cand);
            break;
        }
        cursor = dir.parent();
    }
    let Some(cfg) = cfg_file else {
        tracing::info!("未找到 clip 所属的 yekitrim.toml，回退到旧版独立 polish preview");
        yekitrim_polish::commands::cmd_preview(input, mode.unwrap_or("")).await?;
        return Ok(());
    };

    let cfg_str = percent_encode_component(&cfg.to_string_lossy());
    let dir_str = percent_encode_component(&abs.to_string_lossy());
    let open_path = format!("/#/project/{}/polish?dir={}", cfg_str, dir_str);
    let args = ServerArgs {
        host: "127.0.0.1".to_string(),
        port: yekitrim_service::doctor::DEFAULT_PORT,
        open: true,
        config: Some(cfg),
        // WebUI 来源按构建形态自动定（release 内嵌 / debug 读盘）；`polish preview` 不覆盖它。
        webui_dir: None,
        allow_remote: false,
        trust_proxy: false,
        data_dir: None,
        admin_password_file: None,
        force: false,
        // `polish preview` 仍是**要登录**的（与 `yekitrim server` 一致）。
        // 免登录只给托盘那条"双击就用"的桌面形态 —— 它是显式的、且只限 loopback。
        local_no_auth: false,
        open_path,
    };
    yekitrim_server::run(args).await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_setup::setup(true);

    let cli = Cli::parse();
    let global_config = &cli.config;

    let progress: Arc<dyn yekitrim_core::progress::ProgressReporter> = Arc::new(IndicatifProgress::new());

    match &cli.command {
        Command::Init {
            input,
            danmaku,
            output,
            config: cfg_path,
        } => {
            let out = cfg_path.clone().or_else(|| {
                input.parent().map(|p| p.join(yekitrim_core::path::PROJECT_CONFIG_FILE))
            }).unwrap_or_else(|| PathBuf::from(yekitrim_core::path::PROJECT_CONFIG_FILE));
            yekitrim_service::init_project(
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
            yekitrim_service::run_stages_with_ext(
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
            yekitrim_service::usage::try_install_cli_sink();
            let _cfg = load_cfg(global_config)?;
            let input_override = input.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            let output_override = output_dir.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            let config_file = global_config.clone().unwrap_or_else(|| PathBuf::from(yekitrim_core::path::PROJECT_CONFIG_FILE));
            let stages = vec!["process".to_string()];
            yekitrim_service::run_stages_with(
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
            yekitrim_service::run_stages_with_ext(
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
            yekitrim_service::usage::try_install_cli_sink();
            let config_file = config_path(global_config);
            let transcript_override = transcript.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            yekitrim_service::run_stages_with_ext(
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
            yekitrim_service::run_stages_with_ext(
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
            let src = |stdin: bool, file: &Option<PathBuf>| yekitrim_service::source_from(stdin, file.clone());
            match action {
                UserAction::List => yekitrim_service::cmd_list()?,
                UserAction::Info => yekitrim_service::cmd_info()?,
                UserAction::Add { username, role, stdin, password_file } => {
                    yekitrim_service::cmd_add(username, role, src(*stdin, password_file))?
                }
                UserAction::Passwd { username, stdin, password_file } => {
                    yekitrim_service::cmd_passwd(username, src(*stdin, password_file))?
                }
                UserAction::Check { username, stdin, password_file } => {
                    yekitrim_service::cmd_check(username, src(*stdin, password_file))?
                }
                UserAction::Disable { username, enable } => {
                    yekitrim_service::cmd_disable(username, *enable)?
                }
                UserAction::InitAdmin { stdin, password_file } => {
                    yekitrim_service::cmd_init_admin(src(*stdin, password_file))?
                }
                UserAction::ResetAdminPassword { stdin, password_file } => {
                    yekitrim_service::cmd_reset_admin_password(src(*stdin, password_file))?
                }
                UserAction::Invite { note, days, count, disable } => {
                    yekitrim_service::invite::cmd_invite(note.clone(), *days, *count, *disable)?
                }
                UserAction::Invites => yekitrim_service::invite::cmd_invites()?,
                UserAction::InviteRevoke { target } => {
                    yekitrim_service::invite::cmd_invite_revoke(target)?
                }
            }
        }

        Command::Ingest(action) => {
            use cli::IngestAction;
            match action {
                IngestAction::Token { disable } => yekitrim_service::ingest::cmd_token(*disable)?,
                IngestAction::Status => yekitrim_service::ingest::cmd_status()?,
            }
        }

Command::Doctor { url } => {
            yekitrim_service::cmd_doctor(url)?;
        }

        Command::Usage { days } => {
            yekitrim_service::usage::cmd_usage(*days)?;
        }

Command::Server(args) => {
            yekitrim_server::run(args.clone()).await?;
        }

        Command::Status => {
            let config_file = config_path(global_config);
            let ws = yekitrim_service::Workspace::load(Some(&config_file))?;
            let snap = yekitrim_service::project_snapshot(&ws);
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
                yekitrim_service::init_global_config(path.as_deref())?;
            }
            ConfigAction::Show => {
                println!("{}", yekitrim_service::show_config(global_config.as_deref())?);
            }
        },

Command::Silence { files, duration, threshold } => {
            yekitrim_service::run_silence(files, *duration, *threshold).await?;
        }

Command::Download => {
            yekitrim_service::download_models().await?;
        }

Command::Correct { transcript } => {
            yekitrim_service::usage::try_install_cli_sink();
            let config_file = config_path(global_config);
            let transcript_override = transcript.clone().map(|p| {
                if p.is_absolute() { p } else { std::env::current_dir().unwrap_or_default().join(p) }
            });
            yekitrim_service::run_correct(&config_file, transcript_override, true, None).await?;
            tracing::info!("LLM 订正完成");
        }

Command::Polish(action) => {
            use yekitrim_polish::{commands, cover, process, render};
            yekitrim_service::usage::try_install_cli_sink();

            match action {
                cli::PolishAction::Init { input } => {
                    commands::cmd_init(input).await?;
                }
                cli::PolishAction::Llm { input } => {
                    // CLI 保留断点续传（长片重跑很贵）；编辑器那颗按钮走的是 full=true
                    commands::cmd_llm(input, false, None).await?;
                }
                cli::PolishAction::Process { input } => {
                    process::cmd_process(input).await?;
                }
                cli::PolishAction::Render {
                    input,
                    renderer,
                    strict,
                    versions,
                } => {
                    std::env::set_var("VTEDIT_RENDERER", renderer);
                    // `--strict` 走环境变量：出片的判据在 `pipeline.rs` / `dhampir.rs` 里，
                    // 而那条链路是**库**（WebUI 与 CLI 共用），不给它塞 CLI 类型的参数。
                    std::env::set_var("VTEDIT_STRICT", if *strict { "1" } else { "0" });
                    // §9 多版本成片：`versions <= 1` 时**原样走老路**（一条成片、
                    // 逐字节零回归）；大于 1 才进多版本编排。
                    if *versions > 1 {
                        yekitrim_polish::render::cmd_render_versions(input, *versions).await?;
                    } else {
                        render::cmd_render(input).await?;
                    }
                }
                cli::PolishAction::Cover { input, template, renderer } => {
                    if let Some(t) = template {
                        if !t.is_empty() {
                            cover::cmd_cover_template(input, t).await?;
                        }
                    }
                    cover::cmd_cover(input, None, renderer.as_deref()).await?;
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
        Command::Fonts(action) => match action {
            // 字体库是**程序级全局资产**，与贴纸库同规矩；这两条命令就是它的 CLI 面
            cli::FontAction::List => yekitrim_polish::commands::cmd_fonts_list()?,
            cli::FontAction::Install { id } => {
                yekitrim_polish::commands::cmd_fonts_install(id.as_deref())?
            }
        },
    }

    Ok(())
}
