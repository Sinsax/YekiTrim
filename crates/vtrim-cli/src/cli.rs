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

    /// 下载 sherpa-onnx binary + 模型到 .vtrim/
    Download,

    /// 配置管理
    #[command(subcommand)]
    Config(ConfigAction),

    /// 提取音频 (Stage 1)
    Extract {
        /// 输入视频文件 (默认使用 vtrim.toml 中记录的 input_file)
        #[arg(short, long)]
        input: Option<PathBuf>,

        /// 输出目录
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// 语音转文字 (Stage 2)
    Transcribe {
        /// 输入音频/视频文件 (默认使用 vtrim.toml 中记录的 input_file)
        #[arg(short, long)]
        input: Option<PathBuf>,

        /// 输出目录
        #[arg(short, long)]
        output: Option<PathBuf>,

    },

    /// LLM 订正转录文本（需先运行 vtrim transcribe）
    #[command(name = "correct")]
    Correct {
        /// 转录 JSON 文件路径（默认自动发现）
        #[arg(short, long)]
        transcript: Option<PathBuf>,
    },

    /// 事件检测 (Stage 3)
    Detect {
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

    /// 全自动管线处理（无视 state，从头跑到尾）
    Process {
        /// 输入视频文件 (默认使用 vtrim.toml 中记录的 input_file)
        #[arg(short, long)]
        input: Option<PathBuf>,

        /// 输出目录
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// 仅分析不剪切（运行到 detect 为止）
        #[arg(long)]
        dry_run: bool,
    },

    /// 把已输出切片中的长静音段映射为屏蔽轨道（写入 polish.toml，不改动视频文件）
    Silence {
        /// 要处理的视频/目录（默认扫描 output/clips/）
        #[arg(required = false)]
        files: Vec<PathBuf>,

        /// 静音阈值(秒)，超过此时长的连续静音将被写入屏蔽轨道
        #[arg(long, default_value_t = 2.0)]
        duration: f64,

        /// 音频分析噪声门限 dB（无字幕文件时才使用，默认 -30）
        #[arg(long)]
        threshold: Option<f64>,
    },

    /// 视频精修（GSAP 特效 + 字幕 + 弹幕 + 封面，需先 vtrim cut）
    #[command(subcommand)]
    Polish(PolishAction),

    /// 显示当前处理进度
    Status,

    /// 账号管理（多用户服务：账号账本 / 密码 / 角色）
    #[command(subcommand)]
    User(UserAction),

    /// 启动本地 WebUI 服务（浏览器可视化操作全流程）
    Server(crate::server::ServerArgs),
}

/// 账号管理子命令（多用户服务 P1：CLI 侧；HTTP 登录在下一步接入）
#[derive(Subcommand)]
pub enum UserAction {
    /// 列出账号（用户名/角色/状态/配额）
    List,
    /// 显示账号账本与初始密码文件位置（诊断）
    Info,
    /// 新建账号（不给密码则生成随机密码并只打印一次）
    Add {
        username: String,
        /// 角色: admin | user
        #[arg(long, default_value = "user")]
        role: String,
        /// 从标准输入读取密码
        #[arg(long)]
        stdin: bool,
        /// 从文件读取密码（推荐：不进进程列表）
        #[arg(long)]
        password_file: Option<PathBuf>,
    },
    /// 重置账号密码（不给密码则生成随机密码并只打印一次）
    Passwd {
        username: String,
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        password_file: Option<PathBuf>,
    },
    /// 校验账号口令是否正确（诊断）
    Check {
        username: String,
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        password_file: Option<PathBuf>,
    },
    /// 禁用账号（`--enable` 反向启用）
    Disable {
        username: String,
        #[arg(long)]
        enable: bool,
    },
    /// 首次部署初始化管理员（幂等：已有账号则不做任何改动）
    InitAdmin {
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        password_file: Option<PathBuf>,
    },
    /// 救援：重置管理员密码（忘记密码时在本机执行）
    ResetAdminPassword {
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        password_file: Option<PathBuf>,
    },
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

/// 视频精修子命令
#[derive(Subcommand)]
pub enum PolishAction {
    /// 初始化项目（创建 config.toml + 解析字幕弹幕）
    Init {
        /// clip 目录（默认当前目录）
        #[arg(default_value = ".")]
        input: PathBuf,
    },
    /// LLM 分析生成 segments 和关键词（使用 .vtrim/llm.toml）
    Llm {
        /// clip 目录（默认当前目录）
        #[arg(default_value = ".")]
        input: PathBuf,
    },
    /// 后处理（解析 emoji/irasu keyword、下载资源、计算位置）
    Process {
        /// clip 目录（默认当前目录）
        #[arg(default_value = ".")]
        input: PathBuf,
    },
    /// 渲染视频（自动先跑 process）
    Render {
        /// clip 目录（默认当前目录）
        #[arg(default_value = ".")]
        input: PathBuf,
        /// 渲染后端: chrome | wgpu (默认 chrome)
        #[arg(long, default_value = "chrome")]
        renderer: String,
    },
    /// 生成封面图
    Cover {
        /// clip 目录（默认当前目录）
        #[arg(default_value = ".")]
        input: PathBuf,
        /// 封面模板预设 (left-text/center-text/right-text/minimal)
        #[arg(long)]
        template: Option<String>,
    },
    /// 浏览器预览（自动先跑 process）
    Preview {
        /// clip 目录（默认当前目录）
        #[arg(default_value = ".")]
        input: PathBuf,
        /// 预览模式 (video / cover)，默认同时预览
        mode: Option<String>,
    },
    /// 系统检查（ffmpeg, Chrome, LLM 配置）
    Doctor {
        /// clip 目录（默认当前目录）
        #[arg(default_value = ".")]
        input: PathBuf,
    },
    /// 测试（生成测试素材 + 预览）
    Test {
        /// clip 目录（默认当前目录）
        #[arg(default_value = ".")]
        input: PathBuf,
    },
    /// 本地贴纸工具（索引/匹配测试/清单扫描）
    Stickers {
        #[command(subcommand)]
        action: StickerAction,
    },
}

/// 本地贴纸子命令
#[derive(Subcommand)]
pub enum StickerAction {
    /// 列出本地贴纸索引（组名 + 成员数 + tags）
    List,
    /// 测试 keyword → 贴纸匹配（可多个 keyword）
    Test {
        /// 要测试的 keyword
        #[arg(required = true)]
        keywords: Vec<String>,
    },
    /// 扫描本地贴纸目录生成/更新 stickers.json 清单
    Scan {
        /// 用 LLM 批量生成英文 tags
        #[arg(long)]
        llm: bool,
    },
    /// 预处理贴纸包，生成 <包名>.toml 候选清单
    Build {
        /// 贴纸包名（如 ysyk）
        #[arg(required = true)]
        package: String,
    },
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
