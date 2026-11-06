//! yekitrim-cli 入口：装配 tracing 订阅者，把 clap 命令分发到各实现（二进制名 `yekitrim`）。
//!
//! 入口：`#[tokio::main] async fn main()`、`tracing_setup::setup`、`load_cfg`；子模块 `cli`（命令定义）、`progress`（终端进度条）。
//! 边界：这里只剩入口装配 —— CLI 要用服务端能力只能经 `yekitrim-server` 暴露的 `run()` / `ServerArgs` 进入，别在本地重实现领域逻辑 —— 见 `crates/yekitrim-cli/AGENTS.md`

use std::path::{Path, PathBuf};
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

/// 相对路径一律按**进程 CWD** 绝对化（CLI 的老口径：`-i ./out/video.mp4` 这类照旧可用）。
///
/// 以前这段 `if p.is_absolute() { … } else { current_dir()?.join(p) }` 在 extract / process /
/// transcribe / detect / cut 里各抄了一份 —— 五份一致只是运气：加参数时漏改一处，症状是
/// "某个子命令的相对路径忽然变成相对别处"，而且不报错。
fn abs(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    }
}

/// [`abs`] 的 `Option` 版（clap 的覆盖参数都是可选的）。
fn abs_override(p: &Option<PathBuf>) -> Option<PathBuf> {
    p.as_deref().map(abs)
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

/// `status` 里每一级 stage 的标记。
///
/// `current_stage` 是**最后一个跑完的** stage（状态机判据 `stage > current + 1` 才拒绝，见
/// `ProjectConfig::current_stage`），所以 `num <= current` 打勾、`num == current + 1` 才是"下一步"。
/// 老实现写成 `num < current → ✓ / num == current → →`：最后跑完那一级被标成"正在跑"、
/// 真正的下一步反而是"未开始"（片段跑完 2 级后 EventDetector 那行是 ·，不会出现在任何提示里），
/// 而且每行把标记打了两遍（`[✓] AudioExtractor ✓`）。
fn stage_mark(done: u32, num: u32) -> &'static str {
    if num <= done {
        "✓"
    } else if num == done + 1 {
        "→"
    } else {
        "·"
    }
}

/// `polish preview` 要在 WebUI 里打开的 hash 路由（`/#/p/<pid>/polish[/<clip>]`）。
///
/// ⚠️ **v5 之后路由只认数字项目 id**：`webui/src/router.js` 里 `/project/:cfg/:rest(.*)` 对
/// **非数字**的 `:cfg` 一律 redirect 到 `/workbench`，而且**没有任何视图再读 `?dir=`**。
/// 旧实现拼的 `/#/project/<配置路径>/polish?dir=<片段目录>` 于是永远落到工作台首页
/// （`:cfg` 解出来是整条路径 ⇒ `/^\d+$/` 不成立；实测 `router.resolve` 只匹配到那条 redirect）。
/// 命令照旧退出 0，用户看到的却不是那个片段 —— 这就是本函数存在的全部理由。
///
/// `:clip` 必须是**目录名**（精修页拿它匹配 `clips[].name`，传绝对路径永远匹配不上），
/// 所以只取 `clip_dir` 的最后一段（percent-encode；vue-router 会把 `%20` 解回来，实测过）。
fn polish_preview_path(pid: Option<i64>, cfg_file: &Path, clip_dir: &Path) -> String {
    let Some(pid) = pid else {
        // 没登记 ⇒ 首页还能导入工程；拼 `/p/0/polish` 只会撞一个读不到的项目。
        return "/#/workbench".to_string();
    };
    let mut path = format!("/#/p/{pid}/polish");
    // 输入目录就是工程根 ⇒ 让精修页自己列片段；否则直接定位到那一个片段。
    let root = cfg_file.parent().unwrap_or_else(|| Path::new(""));
    let rel = clip_dir.strip_prefix(root).unwrap_or(Path::new(""));
    if !rel.as_os_str().is_empty() {
        if let Some(name) = clip_dir.file_name().and_then(|n| n.to_str()) {
            path.push('/');
            path.push_str(&percent_encode_component(name));
        }
    }
    path
}

/// 从账本里找这个工程的项目 id（`polish preview` 拼路由用）；找不到 / 读不了 ⇒ `None`。
///
/// **刻意不在这里建库**：账本由 `yekitrim server` / 账号命令负责创建，一个"预览"命令
/// 不该顺手在程序目录里造一个空账本出来（所以先看文件在不在，能读就打不开的错只 warn）。
fn project_id_for_config(db_file: &Path, cfg_file: &Path) -> Option<i64> {
    if !db_file.is_file() {
        return None;
    }
    let db = match yekitrim_service::db::Db::open(db_file) {
        Ok(db) => db,
        Err(e) => {
            tracing::warn!("打开账号账本失败（polish preview 退回首页）: {e:#}");
            return None;
        }
    };
    // 账本里存的是**登记那一刻的路径字符串**：原样比一次，再试 canonicalize
    // （符号链接 / `..` 会让同一份配置有两种写法）。
    let canon = cfg_file.canonicalize().ok();
    for cand in std::iter::once(cfg_file.to_path_buf()).chain(canon) {
        match db.project_by_config(&cand) {
            Ok(Some(row)) => return Some(row.id),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("查项目 id 失败（polish preview 退回首页）: {e:#}");
                return None;
            }
        }
    }
    None
}

/// 尝试把 polish clip 目录定位到所属 yekitrim 工程，并启动/打开同一 WebUI 精修页。
/// 找不到工程时回退旧的独立 preview，保证命令仍然可用。
async fn run_polish_preview(input: &std::path::Path, mode: Option<&str>) -> anyhow::Result<()> {
    use yekitrim_server::ServerArgs;

    let clip_dir = abs(input);
    let mut cursor = Some(clip_dir.as_path());
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

    // 项目 id 只有账本里有（WebUI 的 `/p/:pid` 路由认的是它）。查不到就如实退回首页 ——
    // 别再拼一条前端已经不认的老路由：那会"退出码 0、页面却不是用户要的那个片段"。
    let pid = project_id_for_config(&yekitrim_service::db::db_path(), &cfg);
    if pid.is_none() {
        tracing::info!("工程还没登记进项目库（在 WebUI 里导入过一次即可直达精修页），本次打开工作台首页");
    }
    let open_path = polish_preview_path(pid, &cfg, &clip_dir);
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
            yekitrim_service::run_stages_with_ext(
                &config_file,
                &["extract".to_string()],
                false,
                true,
                abs_override(input),
                abs_override(output),
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
            // 这一行不是摆设：「-c」指错文件时**在这里**就失败。下面 run_stages_with 读的是
            // "全局 + 工程"合并配置，而 load_config_at 对**不存在的工程文件按空工程处理、不报错**
            // （server 侧要先能算路径）—— 少了这道预检，打错的 -c 会静默落到全局默认值上继续跑。
            let _ = load_cfg(global_config)?;
            let stages = vec!["process".to_string()];
            yekitrim_service::run_stages_with(
                &config_path(global_config),
                &stages,
                *dry_run,
                true,
                abs_override(input),
                abs_override(output_dir),
                Some(progress.clone()),
            ).await?;
            tracing::info!("处理完成!");
        }

Command::Transcribe { input, output } => {
            let config_file = config_path(global_config);
            yekitrim_service::run_stages_with_ext(
                &config_file,
                &["transcribe".to_string()],
                false,
                true,
                abs_override(input),
                abs_override(output),
                None,
                None,
                Some(progress.clone()),
            ).await?;
            tracing::info!("语音转文字完成");
        }

Command::Detect { transcript } => {
            yekitrim_service::usage::try_install_cli_sink();
            let config_file = config_path(global_config);
            yekitrim_service::run_stages_with_ext(
                &config_file,
                &["detect".to_string()],
                false,
                true,
                None,
                None,
                abs_override(transcript),
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
            yekitrim_service::run_stages_with_ext(
                &config_file,
                &["cut".to_string()],
                false,
                true,
                abs_override(input),
                abs_override(output),
                None,
                abs_override(events),
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
            // ⚠️ Workspace::load / load_config_at 都**刻意容忍"配置不存在"**（server 要先能算路径；
            // 工程文件缺失按空工程处理）。但 status 是在回答"**这个工程**走到哪了"：目录里根本
            // 没有工程时打 Stage #0、退出码 0，等于把"找错目录"伪装成"全新工程"（脚本里
            // `yekitrim status && 下一步` 也跟着错）。这里如实报错。
            if !ws.config_file.is_file() {
                anyhow::bail!(
                    "未找到工程配置 {}（先运行 `yekitrim init <视频>`，或用 -c 指定配置文件）",
                    yekitrim_core::path::display(&ws.config_file)
                );
            }
            let snap = yekitrim_service::project_snapshot(&ws);
            let input = snap.get("input_file").and_then(|v| v.as_str()).unwrap_or("");
            if !input.is_empty() { println!("视频工程: {}", input); }
            let done = snap.get("current_stage").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            println!("当前进度: Stage #{}", done);
            println!();
            for num in 1..=STAGE_CUT {
                println!("  [{}] {:<20}", stage_mark(done, num), stage_name(num));
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
            ConfigAction::Migrate { paths } => {
                // ⚠️ **为什么需要这条命令**：工程级配置（`yekitrim.toml` / `polish.toml`）按设计
                //    **只在内存里升、不写回磁盘**（工程可能几百个、还可能在网络盘上）⇒ 那几个
                //    已下线的键会**永远留在文件里**，每次读都重新发现它。程序照常跑（迁移在内存里），
                //    但用户需要一个"按一下就真的清掉"的出口 —— 就是这一条。
                //    ⚠️ 清之前每个文件都会留 `.bak-v<旧>`（原样回退的唯一手段）。
                let mut targets: Vec<std::path::PathBuf> = Vec::new();
                if paths.is_empty() {
                    let home = yekitrim_core::path::yekitrim_home();
                    targets.push(home.join("config.toml"));
                    targets.push(home.join("llm.toml"));
                } else {
                    for p in paths {
                        if p.is_dir() {
                            targets.push(p.join("yekitrim.toml"));
                            targets.push(p.join("polish.toml"));
                        } else {
                            targets.push(p.clone());
                        }
                    }
                }
                let (mut cleaned_total, mut failed) = (0usize, 0usize);
                for target in targets {
                    let shown = yekitrim_core::path::display(&target);
                    if !target.is_file() {
                        println!("· 跳过（文件不存在）：{shown}");
                        continue;
                    }
                    match yekitrim_core::config::clean_retired_keys_in(&target) {
                        Ok(keys) if keys.is_empty() => println!("✓ 无需改动：{shown}"),
                        Ok(keys) => {
                            cleaned_total += keys.len();
                            println!("✓ {shown} 清掉了 {} 个没人读的键：{}", keys.len(), keys.join(" / "));
                        }
                        Err(e) => {
                            failed += 1;
                            println!("✗ {shown} 处理失败：{e:#}");
                        }
                    }
                }
                if failed == 0 {
                    println!("（本次共清掉 {cleaned_total} 个键；原文件都留了 .bak-v<旧版本> 备份）");
                } else {
                    // ⚠️ 有失败就**非零退出**：脚本里 `config migrate && 下一步` 才有意义。
                    anyhow::bail!("有 {failed} 个文件没处理成功（见上面的 ✗）");
                }
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
            yekitrim_service::run_correct(&config_file, abs_override(transcript), true, None).await?;
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
                } => {
                    std::env::set_var("VTEDIT_RENDERER", renderer);
                    // `--strict` 走环境变量：出片的判据在 `pipeline.rs` / `dhampir.rs` 里，
                    // 而那条链路是**库**（WebUI 与 CLI 共用），不给它塞 CLI 类型的参数。
                    std::env::set_var("VTEDIT_STRICT", if *strict { "1" } else { "0" });
                    render::cmd_render(input).await?;
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
                // ⚠️ 内建 emoji 分类**没有 CLI 面**（用户无权重建/更改，见 cli.rs 里那条注释）
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `status` 的标记要跟"最后跑完的 stage"这个定义对齐（以前最后跑完那一级被标成 →，
    /// 真正的下一步是 ·，而且每行打了两遍标记）。
    #[test]
    fn stage_marks_follow_last_finished_stage() {
        let marks = |done: u32| (1..=STAGE_CUT).map(|n| stage_mark(done, n)).collect::<Vec<_>>();
        assert_eq!(marks(0), vec!["→", "·", "·", "·"], "新工程：第一级待跑");
        assert_eq!(marks(1), vec!["✓", "→", "·", "·"]);
        assert_eq!(marks(2), vec!["✓", "✓", "→", "·"]);
        assert_eq!(marks(3), vec!["✓", "✓", "✓", "→"]);
        assert_eq!(marks(4), vec!["✓", "✓", "✓", "✓"], "全跑完：没有「下一步」可指");
        // 单级标记不再重复打印（回归：`[✓] AudioExtractor ✓`）
        assert_ne!(stage_mark(2, 2), stage_mark(2, 3));
    }

    /// `polish preview` 拼的路由必须是前端**现在**认识的那条。
    ///
    /// 回归钉：WebUI v5 起 `/project/:cfg/:rest` 对非数字 `:cfg` 一律 redirect 到 /workbench，
    /// 且没有任何视图读 `?dir=`；旧实现拼 `/#/project/<配置路径>/polish?dir=…` 恒落首页。
    #[test]
    fn polish_preview_path_targets_the_v5_route() {
        let root = Path::new("/srv/yekitrim/demo");
        let cfg = root.join("yekitrim.toml");
        let clip = root.join("output/clips/007 标题-中文");

        // 片段目录 ⇒ 直达那一条；`:clip` 只用**目录名**且做 percent-encode
        // （vue-router 会把 %20 解回空格，实测）。
        assert_eq!(
            polish_preview_path(Some(7), &cfg, &clip),
            "/#/p/7/polish/007%20%E6%A0%87%E9%A2%98-%E4%B8%AD%E6%96%87"
        );
        // 工程根 ⇒ 精修页自己列片段。
        assert_eq!(polish_preview_path(Some(7), &cfg, root), "/#/p/7/polish");
        // 项目没登记 ⇒ 首页（回去还能导入工程）。
        assert_eq!(polish_preview_path(None, &cfg, &clip), "/#/workbench");
        // 不在工程里的目录 ⇒ 不瞎猜片段名。
        assert_eq!(
            polish_preview_path(Some(7), &cfg, Path::new("/srv/other/clip")),
            "/#/p/7/polish"
        );
        // 老路由一个字符都不许再出现（`/project/<cfg>` 已被前端重定向掉）。
        assert!(!polish_preview_path(Some(7), &cfg, &clip).contains("/project/"));
    }

    /// 账本里查项目 id：查得到就返回那个 id；账本文件不存在时返回 None，且**不顺手建库**。
    #[test]
    fn project_id_lookup_reads_the_ledger_without_creating_it() {
        let root = std::env::temp_dir().join(format!("yekitrim-cli-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let proj = root.join("demo");
        std::fs::create_dir_all(&proj).unwrap();
        let cfg = proj.join("yekitrim.toml");
        std::fs::write(&cfg, "").unwrap();
        let db_file = root.join("cloud/yekitrim.db");

        let id = {
            let db = yekitrim_service::db::Db::open(&db_file).unwrap();
            let uid = db.create_user("admin", "x", "admin", false, 0).unwrap();
            db.create_project(uid, "demo", None, "local", Some("本机"), &proj, &cfg, None)
                .unwrap()
        };
        assert_eq!(project_id_for_config(&db_file, &cfg), Some(id));

        // 账本文件不存在：返回 None，并且**不**创建它（`polish preview` 不该造空账本）。
        let nowhere = root.join("nope/cloud/yekitrim.db");
        assert_eq!(project_id_for_config(&nowhere, &cfg), None);
        assert!(!nowhere.exists());

        std::fs::remove_dir_all(&root).ok();
    }
}
