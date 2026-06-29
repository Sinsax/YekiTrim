use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use cli::{stage_name, Cli, Command, ConfigAction};
use tracing_subscriber::EnvFilter;
use vtrim_core::download;
use vtrim_core::config::{self, Config};
use vtrim_core::pipeline::{STAGE_AUDIO, STAGE_CUT, STAGE_DETECT, STAGE_TRANSCRIBE};


use crate::progress::IndicatifProgress;

const GLOBAL_CONFIG_TOML: &str = r#"# V-Trim 全局默认配置 — 所有工程共享
# 相同参数会被 vtrim.toml 覆盖

[llm]
# LLM API Key
api_key = ""
# API 基础地址
api_base = ""
# 模型名称（推荐使用非推理模型）
model = ""
# 温度参数
temperature = 0.1
# 最大 Token 数
max_tokens = 16384

[asr]
# CPU 并行转写线程数 (0=CPU/4取整, 上限4)
num_threads = 0
# 音频分块长度(秒) — 0=Silero VAD 自动分割
chunk_duration = 0
# 块间重叠(秒，仅固定分块模式有效)
chunk_overlap = 10.0
# LLM 订正转录文本（需配置 [llm]）
llm_correct = false

[correct]
# 订正 system_prompt（修改此处覆盖内建默认）
system_prompt = "你是虚拟主播直播转录文本的订正助手。修正 ASR 常见的错别字、主播名/VTuber名、游戏名、歌曲名、网络用语。保留口语风格和语气词（啊、哦、嗯、嘛、呗），不要书面化润色。保持序号不变，只输出行。"

# 订正 user_prompt（{segments} 会被替换为实际转录内容）
prompt = """请修正以下语音转文字转录，注意修正：
 - 错别字和语音识别常见错误（同音字、谐音字）
 - 主播名、VTuber 名、嘉宾名（如"永雏塔菲"、"小非"、"塔菲"）
 - 游戏名、动画名、歌曲名（如"法环"不会写成"发还"）
 - 日韩文娱名称（歌手/乐队/偶像/动画曲/V家）：如椎名林檎、YOASOBI、Ado、NewJeans
 - V 圈 / B 站常用术语（SC、舰长、提督、舰礼、切片、串、纯良、米线等）
 - 网络流行语和常见缩写（xp、yyds、kpi、sc、dlc、op、ww等）
 - 其他语音识别易出错的缩写，请结合上下文自行修正
 - 语速快、口糊、吞音导致的识别错误，根据上下文还原正确文字
 - 保留口语风格：不要去掉啊、哦、嗯、嘛、呗、啦等语气词
 - 保留重复和停顿的修辞效果（如"我我我真的"、"然后然后"）
 - 保留情绪化表达（大笑、尖叫、哭腔、破音等通过文字体现）

按原样返回每一行，只修改文字内容，不要修改序号。
格式：序号|修正后文字"""

[detection]
# 主播名称 (非空时自动追加到切片标题末尾)
vtuber_name = ""
# 弹幕密度统计窗口(秒)
danmaku_window_secs = 10
system_prompt = "你是一个虚拟主播/VTuber 直播切片分析助手。分析直播转录文本，将内容划分为多个有看点的视频片段。标题要使用 B 站风格夸张口语化表达：善用反问句(?)、感叹句(!)、问号叠加(??)、省略号(...)；嵌入主播原话或动作注释；善用网络用语：爆了、难绷、神人、破防、上头、下头、串、好女孩、魔法期。摘要要详尽描述内容看点。质量优先于数量，每个事件必须有明显的看点、转折、爆点或清晰的话题边界，无话题切换的过渡内容应合并。"

prompt_template = """请分析以下直播转录文本（虚拟主播/VTuber），将其划分为多个事件片段。要求：

1. 每个事件片段时长控制在30秒-30分钟，短于30秒的需与相邻片段合并
2. 标题(label)创作要求：
   - 口语化、夸张、悬念化，使用 B 站观众熟悉的口头禅/反问/感叹
   - 长度 8~50 字都可以（爆点短标题、细节并列长标题皆可），不超过 50 字
   - 包含 1-3 个 emoji，常用：🤣😱🥵😡😅🤭🤑👻😊🐶🐮🎵🔥💥😭😫😋💩😈🥺😎
   - 优先使用以下句式：
     a) 反问式：句末用 ? 或 ??，如"这也能信？"
     b) 感叹式：句末用 ! 或 !!，如"爆了！"
     c) 问号叠加：?? ??? ?!! 表示震惊/无语
     d) 悬念式：句中用 ... 或（动作）收尾，如"难绷..."
     e) 对话引用：直接嵌入主播原话，用 "" 或冒号
      f) 列点式：用 、 ， 并列多个细节
    - 特殊场景可用后缀标注：(弹幕版) 【新衣回】 【自用】 等
   - 善用网络用语：爆了、难绷、神人、破防、上头、下头、串、好女孩、魔法期、名场面
   - 质量门槛：每个事件必须有明显的看点、转折、爆点或清晰的话题边界。仅用于填充沉默的过渡语句、无话题切换的重复附和内容不要单独切出，合并到相邻事件
3. 摘要(summary)详尽描述片段核心内容和看点，50-100字
4. 事件类型(event_type)从以下选择：
   - gameplay(游戏/操作)
   - interaction(互动/连麦/双播)
   - qna(问答/读 SC)
   - reaction(看视频/听歌/观影/鉴赏)
   - storytelling(讲故事/分享经历/回忆)
   - music(唱歌/演奏/歌切)
   - food(吃播/美食/试吃)
   - chat(聊天/杂谈/日常)
   - discussion(讨论/争论/锐评)
   - ranking(盘点/排行/排行榜)
   - irl(线下/旅行/Vlog/外出)
   - rant(吐槽/破防/红温)
   - other(其他)

参考标题风格（覆盖多句式、多题材）：
  爆了！牢豆直播看塔菲中之人视频😱
  被初王葛孚雷打破防，妈的一天天使不完个牛劲我超！【七海Nana7mi】
  海子姐难绷90岁治疗ED【七海Nana7mi】
  这什么雷霆坐姿雷霆视角🥵羞死人了🥵
  灰泽满妈妈：他们连你成绩很差都知道了吗？？【灰泽满Hazel】
  难绷花礼连自己拉的屎都想吃😅真是老吃家了🤤
  岁己亲推脚臭味薯片：不是我脚臭，这是世俗意义上的脚臭【岁己SUI】
  谁家小土豆地雷？【灰泽满Hazel】
  我站着上厕所！【七海Nana7mi】
  弹幕问豆今色，豆特意低头看了下脱口而出：梅川😊

输出格式：只输出一个 JSON 数组，不要 markdown 或解释：

转录片段：
{segments}

只输出JSON数组：
[{"start": 0.0, "end": 120.0, "label": "这操作给我看傻了??🤣", "summary": "详尽描述这段内容的核心看点...", "event_type": "interaction"}]"""

[cutting]
# 输出目录
output_dir = "./output"
# 是否覆盖已有文件
overwrite = false
"#;

mod cli;
mod progress;

fn setup_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

fn resolve_input(cfg: &Config, cli_input: &Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(input) = cli_input {
        return Ok(input.clone());
    }
    if let Some(ref path) = cfg.project.input_file {
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    anyhow::bail!(
        "未指定输入文件。请提供 -i <视频路径>，或先运行 `vtrim init <video>` 初始化工程"
    );
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

fn check_stage(stage_num: u32, current: u32, _cfg: &Config) -> anyhow::Result<()> {
    let mut needed: Vec<u32> = Vec::new();
    for n in current + 1..stage_num {
        if !vtrim_core::pipeline::stage_enabled(n, _cfg) {
            continue;
        }
        needed.push(n);
    }

    if !needed.is_empty() {
        anyhow::bail!(
            "无法执行阶段 '{}' (#{})，当前进度为 #{}\n请先完成前面的阶段：{}",
            stage_name(stage_num),
            stage_num,
            current,
            needed
                .iter()
                .map(|&n| format!("#{} {}", n, stage_name(n)))
                .collect::<Vec<_>>()
                .join(" → ")
        );
    }
    Ok(())
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
            let out = cfg_path
                .clone()
                .unwrap_or_else(|| PathBuf::from("vtrim.toml"));

            // 优先从全局配置复制，不存在则使用内建默认
            let global_path = vtrim_core::path::vtrim_home().join("config.toml");
            let mut body = if global_path.exists() {
                tracing::info!("从全局配置复制: {}", global_path.display());
                let content = std::fs::read_to_string(&global_path)?
                    .replace("# V-Trim 全局默认配置 — 所有工程共享\n# 相同参数会被 vtrim.toml 覆盖", "");
                content
            } else {
                GLOBAL_CONFIG_TOML.to_string()
                    .replace("# V-Trim 全局默认配置 — 所有工程共享\n# 相同参数会被 vtrim.toml 覆盖", "")
            };

            let mut doc: toml_edit::DocumentMut = body.parse()?;
            doc.remove("llm");
            doc.remove("correct");

            // 移除 [detection] 中的 prompt 字段，仅保留在 config.toml
            if let Some(detection) = doc.get_mut("detection") {
                if let Some(table) = detection.as_table_mut() {
                    table.remove("system_prompt");
                    table.remove("prompt_template");
                }
            }

            if let Some(dir) = &output {
                doc["cutting"]["output_dir"] = toml_edit::value(dir.to_string_lossy().as_ref());
            }

            body = doc.to_string();

            // 构建 [project] 块（带注释），放在最前面
            let danmaku_val = if let Some(df) = &danmaku {
                df.to_string_lossy().to_string()
            } else {
                let auto_ass = input.with_extension("ass");
                if auto_ass.exists() {
                    tracing::info!("自动发现弹幕文件: {}", auto_ass.display());
                    auto_ass.to_string_lossy().to_string()
                } else {
                    String::new()
                }
            };

            let mut project = toml_edit::Table::new();
            project.set_implicit(true);
            project.insert("input_file", toml_edit::value(input.to_string_lossy().as_ref()));
            project.insert("danmaku_file", toml_edit::value(&danmaku_val));
            project.insert("transcript_file", toml_edit::value(""));
            project.insert("audio_file", toml_edit::value(""));
            project.insert("events_file", toml_edit::value(""));
            project.insert("current_stage", toml_edit::value(0));

            let mut header = toml_edit::DocumentMut::new();
            header.insert("project", toml_edit::Item::Table(project));
            let mut project_str = header.to_string();

            // 给 project 字段添加注释
            let comments = [
                ("input_file", "输入视频文件路径"),
                ("danmaku_file", "弹幕文件路径"),
                ("transcript_file", "转写文件路径 (由 transcribe 写入)"),
                ("audio_file", "音频文件路径 (由 extract-audio 写入)"),
                ("events_file", "事件文件路径 (由 detect-events 写入)"),
                ("current_stage", "当前处理进度 (0=未开始, 1~4=各阶段)"),
            ];
            for (key, comment) in &comments {
                let line = format!("{} = ", key);
                if project_str.contains(&line) {
                    let commented = format!("# {}\n{}", comment, line);
                    project_str = project_str.replace(&line, &commented);
                }
            }

            let full = format!("# V-Trim 项目配置 — 优先级高于 .vtrim/config.toml\n# 相同的参数以本文件为准\n\n{}", project_str.trim()) + "\n\n" + body.trim();
            std::fs::write(&out, full)?;

            tracing::info!("视频工程已初始化: {}", input.display());
            tracing::info!("配置文件: {}", out.display());
        }

        Command::Extract { input, output } => {
            let cfg = load_cfg(global_config)?;
            let input_path = resolve_input(&cfg, input)?;
            let output_dir = output.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join(&cfg.cutting.output_dir)
            });

            let mut ctx = vtrim_core::types::PipelineContext::new(input_path, output_dir);
            let mut runner = vtrim_core::pipeline::PipelineRunner::new(cfg);
            runner.set_progress(progress.clone());
            runner.add_stage(Box::new(vtrim_core::audio::AudioExtractor));

            runner.run(&mut ctx).await?;
            if let Some(ref ap) = ctx.audio_path {
                let _ = config::save_audio_path(&ap.to_string_lossy());
            }
            tracing::info!("音频提取完成");
        }

        Command::Process {
            input,
            output: output_dir,
            stages: _stages,
            dry_run,
        } => {
            let cfg = load_cfg(global_config)?;
            let input_path = resolve_input(&cfg, input)?;
            let output = output_dir.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join(&cfg.cutting.output_dir)
            });

            let current = config::load_current_stage()?;
            let mut ctx = vtrim_core::types::PipelineContext::new(input_path, output.clone());

            // Step 1: 音频提取（未完成才跑）
            if current < STAGE_AUDIO {
                let mut extract_runner = vtrim_core::pipeline::PipelineRunner::new(cfg.clone());
                extract_runner.set_progress(progress.clone());
                extract_runner.add_stage(Box::new(vtrim_core::audio::AudioExtractor));
                extract_runner.run_until(&mut ctx, STAGE_AUDIO).await?;
                config::save_current_stage(STAGE_AUDIO)?;
            } else {
                tracing::info!("音频已提取，跳过");
            }

            // Step 2: 转写（未完成才跑）
            if current < STAGE_TRANSCRIBE {
                vtrim_core::download::ensure_models().await?;
                let mut transcribe_runner = vtrim_core::pipeline::PipelineRunner::new(cfg.clone());
                transcribe_runner.set_progress(progress.clone());
                transcribe_runner.add_stage(Box::new(vtrim_core::asr::Transcriber));
                transcribe_runner.run_until(&mut ctx, STAGE_TRANSCRIBE).await?;
                config::save_current_stage(STAGE_TRANSCRIBE)?;
            } else {
                // 从已保存的文件恢复 transcript
                let recovery_path = cfg.project.transcript_file
                    .as_ref()
                    .filter(|p| !p.is_empty())
                    .map(PathBuf::from)
                    .filter(|p| p.exists())
                    .or_else(|| {
                        let json = ctx.output_dir.join("transcript.json");
                        if json.exists() { Some(json) } else { None }
                    })
                    .or_else(|| {
                        let srt = ctx.output_dir.join("video.srt");
                        if srt.exists() { Some(srt) } else { None }
                    });

                match recovery_path {
                    Some(path) => {
                        let content = std::fs::read_to_string(&path)?;
                        let ext = path.extension()
                            .map(|e| e.to_string_lossy().to_lowercase())
                            .unwrap_or_default();
                        ctx.transcript = Some(if ext == "json" {
                            serde_json::from_str(&content)?
                        } else {
                            vtrim_core::srt::parse_srt(&content)?
                        });
                        tracing::info!("转写已存在: {}", path.display());
                    }
                    None => {
                        tracing::warn!("current_stage >= 2 但未找到转写文件，重新转写");
                        vtrim_core::download::ensure_models().await?;
                        let mut transcribe_runner = vtrim_core::pipeline::PipelineRunner::new(cfg.clone());
                        transcribe_runner.set_progress(progress.clone());
                        transcribe_runner.add_stage(Box::new(vtrim_core::asr::Transcriber));
                        transcribe_runner.run_until(&mut ctx, STAGE_TRANSCRIBE).await?;
                        config::save_current_stage(STAGE_TRANSCRIBE)?;
                    }
                }
            }

            // LLM 订正：仅当事件尚未检测时执行，避免重复订正已完成的检测结果
            if current < STAGE_DETECT && cfg.asr.llm_correct && !cfg.llm.api_key.is_empty() {
                if let Some(ref transcript) = ctx.transcript {
                    tracing::info!("LLM 订正转录文本...");
                    match vtrim_core::asr::correct::correct_transcript(&transcript.segments, &cfg.llm, &cfg.correct).await {
                        Ok(corrected) => {
                            let new = vtrim_core::types::Transcript {
                                segments: corrected,
                                language: "zh".into(),
                            };
                            let llm_srt = ctx.output_dir.join("video_llm.srt");
                            vtrim_core::srt::write_srt(&new, &llm_srt)?;
                            config::save_transcript_path(&llm_srt.to_string_lossy())?;
                            ctx.transcript = Some(new);
                            tracing::info!("LLM 订正完成");
                        }
                        Err(e) => tracing::warn!("LLM 订正失败，使用原始转录: {}", e),
                    }
                }
            }

            // Step 4: 检测 + 剪切（未完成才跑）
            if current < STAGE_DETECT {
                let mut detect_runner = vtrim_core::pipeline::PipelineRunner::new(cfg.clone());
                detect_runner.set_progress(progress.clone());
                detect_runner.add_stage(Box::new(vtrim_core::detection::EventDetector));
                if *dry_run {
                    detect_runner.run_until(&mut ctx, STAGE_DETECT).await?;
                    tracing::info!("(--dry-run) 分析完成，未执行剪切。输出目录: {}", output.display());
                    return Ok(());
                }
                detect_runner.run_until(&mut ctx, STAGE_DETECT).await?;
                config::save_current_stage(STAGE_DETECT)?;
            } else {
                tracing::info!("事件检测已存在，跳过");
            }

            // Step 5: 剪切（未完成才跑）
            if current < STAGE_CUT {
                if *dry_run {
                    tracing::info!("(--dry-run) 未执行剪切。输出目录: {}", output.display());
                    return Ok(());
                }
                // 从文件恢复 events
                let events_path = ctx.output_dir.join("events.json");
                if events_path.exists() {
                    ctx.events = vtrim_core::detection::load_events(&events_path)?;
                }
                let mut cut_runner = vtrim_core::pipeline::PipelineRunner::new(cfg);
                cut_runner.set_progress(progress.clone());
                cut_runner.add_stage(Box::new(vtrim_core::cutting::Cutter));
                cut_runner.run_until(&mut ctx, STAGE_CUT).await?;
                config::save_current_stage(STAGE_CUT)?;
            } else {
                tracing::info!("剪切已完成，跳过");
            }

            tracing::info!("处理完成! 输出目录: {}", output.display());
        }

        Command::Transcribe {
            input,
            output,
            backend,
        } => {
            let mut cfg = load_cfg(global_config)?;

            if let Some(backend) = backend {
                cfg.asr.backend = backend.clone();

                // 同步写入 vtrim.toml
                let config_path = global_config
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("vtrim.toml"));
                if config_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&config_path) {
                        if let Ok(mut doc) = content.parse::<toml_edit::DocumentMut>() {
                            doc["asr"]["backend"] = toml_edit::value(backend.clone());
                            let _ = std::fs::write(&config_path, doc.to_string());
                            tracing::info!("vtrim.toml: asr.backend = {}", backend);
                        }
                    }
                }
            }

            let current = config::load_current_stage()?;
            check_stage(2, current, &cfg)?;
            let input_path = resolve_input(&cfg, input)?;
            let output = output.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join(&cfg.cutting.output_dir)
            });

            let mut ctx = vtrim_core::types::PipelineContext::new(input_path, output);
            let mut runner = vtrim_core::pipeline::PipelineRunner::new(cfg);
            runner.set_progress(progress.clone());
            runner.add_stage(Box::new(vtrim_core::asr::Transcriber));

            runner.run(&mut ctx).await?;
            // 保存转写文件路径到配置，供 detect-events 自动发现
            config::save_transcript_path(
                &ctx.output_dir
                    .join("video.srt")
                    .to_string_lossy(),
            )?;
            tracing::info!("语音转文字完成");
        }

        Command::DetectEvents { transcript } => {
            let cfg = load_cfg(global_config)?;
            let current = config::load_current_stage()?;
            check_stage(3, current, &cfg)?;

            let input_path = resolve_input(&cfg, &None)?;
            let output_dir = std::env::current_dir()
                .unwrap_or_default()
                .join(&cfg.cutting.output_dir);

            // 优先级: --transcript 参数 > config.project.transcript_file > 自动发现
            let transcript_path = transcript
                .clone()
                .or_else(|| {
                    cfg.project.transcript_file
                        .as_ref()
                        .filter(|p| !p.is_empty())
                        .map(PathBuf::from)
                })
                .or_else(|| {
                    let json = output_dir.join("transcript.json");
                    let srt = output_dir.join("video.srt");
                    if json.exists() {
                        tracing::info!("自动发现转写 JSON: {}", json.display());
                        Some(json)
                    } else if srt.exists() {
                        tracing::info!("自动发现转写 SRT: {}", srt.display());
                        Some(srt)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "未找到转写文件，请通过 --transcript 指定或先运行 vtrim transcribe"
                    )
                })?;

            let content = std::fs::read_to_string(&transcript_path)
                .with_context(|| format!("无法读取转写文件: {}", transcript_path.display()))?;

            let ext = transcript_path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let transcript = if ext == "json" {
                serde_json::from_str(&content)
                    .with_context(|| format!("解析转写 JSON 失败: {}", transcript_path.display()))?
            } else if ext == "srt" {
                vtrim_core::srt::parse_srt(&content)
                    .with_context(|| format!("解析 SRT 失败: {}", transcript_path.display()))?
            } else {
                anyhow::bail!("不支持的转写文件格式: .{} (仅支持 .json / .srt)", ext);
            };

            let mut ctx = vtrim_core::types::PipelineContext::new(input_path, output_dir.clone());
            ctx.transcript = Some(transcript);

            let mut runner = vtrim_core::pipeline::PipelineRunner::new(cfg);
            runner.set_progress(progress.clone());
            runner.add_stage(Box::new(vtrim_core::detection::EventDetector));

            runner.run(&mut ctx).await?;
            let ev_path = output_dir.join("events.json");
            let _ = config::save_events_path(&ev_path.to_string_lossy());
            tracing::info!("事件检测完成, 共 {} 个事件", ctx.events.len());
        }

        Command::Cut {
            input,
            events,
            output,
        } => {
            let cfg = load_cfg(global_config)?;
            let current = config::load_current_stage()?;
            check_stage(4, current, &cfg)?;
            let input_path = resolve_input(&cfg, input)?;
            let output_dir = output.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join(&cfg.cutting.output_dir)
            });

            let events_path = events
                .clone()
                .or_else(|| {
                    cfg.project.events_file
                        .as_ref()
                        .filter(|p| !p.is_empty())
                        .map(PathBuf::from)
                        .filter(|p| p.exists())
                })
                .or_else(|| {
                    let auto = output_dir.join("events.json");
                    if auto.exists() {
                        Some(auto)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "未找到事件 JSON 文件, 请通过 --events 指定或先运行 detect-events"
                    )
                })?;

            let events = vtrim_core::detection::load_events(&events_path)
                .with_context(|| format!("无法读取或解析事件文件: {}", events_path.display()))?;

            let mut ctx = vtrim_core::types::PipelineContext::new(input_path, output_dir);
            ctx.events = events;

            let mut runner = vtrim_core::pipeline::PipelineRunner::new(cfg);
            runner.set_progress(progress.clone());
            runner.add_stage(Box::new(vtrim_core::cutting::Cutter));

            runner.run(&mut ctx).await?;
            tracing::info!("剪切完成, 共 {} 个片段", ctx.segments.len());
        }

        Command::Status => {
            let current = config::load_current_stage()?;
            let cfg = load_cfg(global_config)?;
            if let Some(ref input_file) = cfg.project.input_file {
                if !input_file.is_empty() {
                    println!("视频工程: {}", input_file);
                }
            }
            println!("当前进度: Stage #{}", current);
            println!();
            for num in 1..=STAGE_CUT {
                let status = if num < current {
                    "✓"
                } else if num == current {
                    "→"
                } else {
                    "·"
                };
                let name = stage_name(num);
                println!("  [{}] {:<20} {}", status, name, status);
            }
        }

        Command::Config(action) => match action {
            ConfigAction::Init { path } => {
                let out = path.clone().unwrap_or_else(|| {
                    vtrim_core::path::vtrim_home().join("config.toml")
                });
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&out, GLOBAL_CONFIG_TOML)?;
                tracing::info!("全局配置文件已生成: {}", out.display());
            }
            ConfigAction::Show => {
                let cfg = load_cfg(global_config)?;
                println!("{}", toml::to_string_pretty(&cfg)?);
            }
        },

        Command::SilenceRemove { files, duration, output } => {
            let clips_dir = std::env::current_dir()
                .unwrap_or_default()
                .join("output")
                .join("clips");

            let targets: Vec<PathBuf> = if files.is_empty() {
                if clips_dir.exists() {
                    std::fs::read_dir(&clips_dir)?
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| {
                            p.extension()
                                .map(|ext| ext == "mp4")
                                .unwrap_or(false)
                        })
                        .collect()
                } else {
                    anyhow::bail!("未指定文件且 {} 不存在", clips_dir.display());
                }
            } else {
                files.clone()
            };

            if targets.is_empty() {
                anyhow::bail!("没有找到要处理的视频文件");
            }

            tracing::info!("静音移除: duration={}s, 共 {} 个文件", duration, targets.len());

            for (i, input) in targets.iter().enumerate() {
                let out_dir = output.clone().unwrap_or_else(|| {
                    input.parent()
                        .expect("无法获取输入文件所在目录")
                        .to_path_buf()
                });
                std::fs::create_dir_all(&out_dir)?;
                let stem = input
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("clip_{}", i));
                let output_path = out_dir.join(format!("{}_cleaned.mp4", stem));
                tracing::info!("[{}/{}] {} → {}", i + 1, targets.len(), input.display(), output_path.display());
                let srt_path = input.with_extension("srt");
                let srt_opt = if srt_path.exists() { Some(srt_path.as_path()) } else { None };
                let segments = vtrim_core::cutting::remove_silence(input, &output_path, *duration, srt_opt)?;

                if srt_path.exists() {
                    let out_srt = output_path.with_extension("srt");
                    vtrim_core::cutting::adjust_srt(&srt_path, &out_srt, &segments)?;
                    tracing::info!("  字幕已调整: {}", out_srt.display());
                }
            }

            tracing::info!("静音移除完成!");
        }

        Command::DownloadModels { model: _ } => {
            tracing::info!("下载 SenseVoice 模型...");
            download::download_all(None).await?;
        }

        Command::Correct { transcript } => {
            let cfg = load_cfg(global_config)?;
            let output_dir = std::env::current_dir()
                .unwrap_or_default()
                .join(&cfg.cutting.output_dir);

            let transcript_path = transcript
                .clone()
                .or_else(|| {
                    cfg.project.transcript_file
                        .as_ref()
                        .filter(|p| !p.is_empty())
                        .map(PathBuf::from)
                })
                .or_else(|| {
                    let json = output_dir.join("transcript.json");
                    if json.exists() { Some(json) } else { None }
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("未找到转录文件，请通过 --transcript 指定或先运行 vtrim transcribe")
                })?;

            let content = std::fs::read_to_string(&transcript_path)
                .with_context(|| format!("无法读取转录文件: {}", transcript_path.display()))?;
            let ext = transcript_path.extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let mut transcript_data: vtrim_core::types::Transcript = if ext == "json" {
                serde_json::from_str(&content)
                    .with_context(|| "解析转录 JSON 失败")?
            } else {
                vtrim_core::srt::parse_srt(&content)
                    .with_context(|| "解析 SRT 失败")?
            };

            tracing::info!("LLM 订正 {} 个片段...", transcript_data.segments.len());
            match vtrim_core::asr::correct::correct_transcript(&transcript_data.segments, &cfg.llm, &cfg.correct).await {
                Ok(corrected) => {
                    transcript_data.segments = corrected;

                    let llm_srt = output_dir.join("video_llm.srt");
                    std::fs::create_dir_all(&output_dir)?;
                    vtrim_core::srt::write_srt(&transcript_data, &llm_srt)?;
                    config::save_transcript_path(&llm_srt.to_string_lossy())?;

                    tracing::info!("LLM 订正完成");
                    tracing::info!("  字幕: {}", llm_srt.display());
                }
                Err(e) => anyhow::bail!("LLM 订正失败: {}", e),
            }
        }

        Command::Version => {
            println!("vtrim {}", env!("CARGO_PKG_VERSION"));
        }
    }

    Ok(())
}
