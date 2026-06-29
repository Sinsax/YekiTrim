use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "vtrim", version, about = "直播录像自动切片工具")]
pub struct Cli {
    /// 配置文件路径 (默认 ./vtrim.toml)
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// 初始化视频工程 (创建 vtrim.toml)
    Init {
        /// 输入视频文件
        input: PathBuf,

        /// 弹幕文件 (可选)
        #[arg(long)]
        danmaku: Option<PathBuf>,

        /// 输出目录 (默认 ./output)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// 配置文件路径 (默认 ./vtrim.toml)
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// 提取音频 (Stage 1)
    Extract {
        /// 输入视频文件 (默认使用 vtrim.toml 中记录的 input_file)
        #[arg(short, long)]
        input: Option<PathBuf>,

        /// 输出目录
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// 全自动管线处理（无视 state，从头跑到尾）
    Process {
        /// 输入视频文件 (默认使用 vtrim.toml 中记录的 input_file)
        #[arg(short, long)]
        input: Option<PathBuf>,

        /// 输出目录
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// 指定运行阶段 (逗号分隔)
        #[arg(long, value_delimiter = ',', default_value = "audio,transcribe,detect,cut")]
        stages: Vec<String>,

        /// 仅分析不剪切（运行到 detect 为止）
        #[arg(long)]
        dry_run: bool,
    },

    /// 语音转文字 (Stage 2)
    Transcribe {
        /// 输入音频/视频文件 (默认使用 vtrim.toml 中记录的 input_file)
        #[arg(short, long)]
        input: Option<PathBuf>,

        /// 输出目录
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// ASR 后端 (local / bcut / kuaishou), 会同步写入 vtrim.toml
        #[arg(long)]
        backend: Option<String>,
    },

    /// 事件检测 (Stage 3)
    DetectEvents {
        /// 转录结果 JSON 文件 (默认自动从输出目录发现)
        #[arg(short, long)]
        transcript: Option<PathBuf>,
    },

    /// 剪切片段 (Stage 4)
    Cut {
        /// 输入视频文件 (默认使用 vtrim.toml 中记录的 input_file)
        #[arg(short, long)]
        input: Option<PathBuf>,

        /// 事件 JSON 文件 (默认自动从输出目录发现)
        #[arg(short, long)]
        events: Option<PathBuf>,

        /// 输出目录
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// 显示当前处理进度
    Status,

    /// 配置管理
    #[command(subcommand)]
    Config(ConfigAction),

    /// 移除已输出视频中的长静音段（重编码，输出到输入文件同目录）
    SilenceRemove {
        /// 要处理的视频文件（默认扫描 output/clips/）
        #[arg(required = false)]
        files: Vec<PathBuf>,

        /// 静音阈值(秒)，超过此时长的连续静音将被移除
        #[arg(long, default_value_t = 2.0)]
        duration: f64,

        /// 输出目录 (默认输出到输入文件同目录)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// 下载 sherpa-onnx binary + 模型到 .vtrim/
    DownloadModels {
        /// 模型类型 (已忽略，仅下载 sensevoice)
        #[arg(long, default_value = "sensevoice")]
        model: String,
    },

    /// 安装 spleeter 人声分离到 .vtrim/venv/ (需 uv)
    SetupSpleeter,

    /// 人声分离 (需先 vtrim setup-spleeter)
    Separate {
        /// 输入音频文件 (默认使用 vtrim.toml 的 audio_file)
        #[arg(required = false)]
        input: Option<PathBuf>,

        /// 输出目录 (默认 output/)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// 显示版本信息
    Version,
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// 生成默认配置文件 (不带视频路径)
    Init {
        /// 输出路径 (默认 ./vtrim.toml)
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// 显示当前配置
    Show,
}

pub fn stage_name(num: u32) -> &'static str {
    match num {
        1 => "AudioExtractor",
        2 => "Transcriber",
        3 => "EventDetector",
        4 => "Cutter",
        _ => "Unknown",
    }
}
