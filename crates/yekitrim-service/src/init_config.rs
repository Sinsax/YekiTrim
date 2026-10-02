//! 初始化工程与配置管理 Service。

use std::path::{Path, PathBuf};

pub fn init_project(
    input: &Path,
    danmaku: Option<&Path>,
    output_dir: Option<&str>,
    cfg_path: &Path,
) -> anyhow::Result<()> {
    if input.is_dir() {
        anyhow::bail!("'{}' 是目录路径。vtrim init 需要视频文件路径", input.display());
    }
    let valid_ext = ["mp4", "mkv", "mov", "avi", "flv", "webm", "mts", "m2ts"];
    let ext = input.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).unwrap_or_default();
    if !valid_ext.contains(&ext.as_str()) {
        anyhow::bail!("'{}' 不是支持的视频格式。支持: {}", input.display(), valid_ext.join(", "));
    }
    if !input.exists() {
        anyhow::bail!("视频文件不存在: {}", input.display());
    }
    yekitrim_core::config::write_project_config(cfg_path, input, danmaku, output_dir)?;
    Ok(())
}

/// 程序配置 `config.toml` 的标准路径（可执行文件旁的 `.vtrim/`）。
pub fn global_config_path() -> PathBuf {
    yekitrim_core::path::vtrim_home().join("config.toml")
}

/// 程序配置同目录的 `llm.toml` 路径。
fn llm_config_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .map(|p| p.join("llm.toml"))
        .unwrap_or_else(|| PathBuf::from(".yekitrim/llm.toml"))
}

/// 默认程序配置内容（内建 prompt 注入）。
fn global_config_body() -> String {
    yekitrim_core::config::GLOBAL_CONFIG_TOML
        .replace(r#""{{CORRECT_SYSTEM_PROMPT}}""#, &toml_val(yekitrim_core::asr::correct::DEFAULT_SYSTEM_PROMPT))
        .replace(r#""{{CORRECT_USER_PROMPT}}""#, &toml_val(yekitrim_core::asr::correct::DEFAULT_USER_PROMPT))
        .replace(r#""{{DETECT_SYSTEM_PROMPT}}""#, &toml_val(yekitrim_core::detection::DEFAULT_SYSTEM_PROMPT))
        .replace(r#""{{DETECT_USER_PROMPT}}""#, &toml_val(yekitrim_core::detection::DEFAULT_PROMPT))
}

/// 生成程序配置（覆盖写）— `vtrim config init`。
pub fn init_global_config(path: Option<&Path>) -> anyhow::Result<()> {
    let out = path.map(PathBuf::from).unwrap_or_else(global_config_path);
    if let Some(parent) = out.parent() { std::fs::create_dir_all(parent)?; }
    std::fs::write(&out, global_config_body())?;
    std::fs::write(llm_config_path(&out), yekitrim_core::config::LLM_CONFIG_TOML)?;
    std::fs::create_dir_all(yekitrim_core::path::sticker_dir())?;
    yekitrim_core::fonts::ensure_dir()?;
    // 自带字体是**内嵌**的（2026-10-02）：建完库就地装好，用户不需要任何额外操作
    yekitrim_core::fonts::ensure_builtin_fonts();
    tracing::info!("全局配置已生成: {}", out.display());
    Ok(())
}

/// `ensure_global_config` 的执行结果。
#[derive(Debug, Default)]
pub struct EnsureConfigResult {
    /// 本次新建的文件
    pub created: Vec<PathBuf>,
    /// 解析失败 → 已备份并重建的文件：(原文件, 备份文件, 解析错误)
    pub repaired: Vec<(PathBuf, PathBuf, String)>,
}

/// 程序配置缺失时补齐（`vtrim server` 启动时调用）。
///
/// - 文件不存在 → 写入默认配置；
/// - 文件存在但**解析失败**（整个配置都会被忽略，等同于没有配置）→ 备份为
///   `config.toml.broken` 后重建，避免坏文件让所有设置静默失效；
/// - 文件存在且合法 → 原样保留，绝不覆盖。
pub fn ensure_global_config() -> anyhow::Result<EnsureConfigResult> {
    let out = global_config_path();
    let llm_out = llm_config_path(&out);
    let mut result = EnsureConfigResult::default();

    if !out.exists() {
        if let Some(parent) = out.parent() { std::fs::create_dir_all(parent)?; }
        std::fs::write(&out, global_config_body())?;
        result.created.push(out.clone());
    } else if let Err(e) = parse_check(&out) {
        let backup = out.with_extension("toml.broken");
        std::fs::copy(&out, &backup)?;
        std::fs::write(&out, global_config_body())?;
        result.repaired.push((out.clone(), backup, format!("{e:#}")));
    }

    if !llm_out.exists() {
        if let Some(parent) = llm_out.parent() { std::fs::create_dir_all(parent)?; }
        std::fs::write(&llm_out, yekitrim_core::config::LLM_CONFIG_TOML)?;
        result.created.push(llm_out.clone());
    } else if let Err(e) = parse_check(&llm_out) {
        let backup = llm_out.with_extension("toml.broken");
        std::fs::copy(&llm_out, &backup)?;
        std::fs::write(&llm_out, yekitrim_core::config::LLM_CONFIG_TOML)?;
        result.repaired.push((llm_out.clone(), backup, format!("{e:#}")));
    }

    if !result.created.is_empty() || !result.repaired.is_empty() {
        std::fs::create_dir_all(yekitrim_core::path::sticker_dir())?;
    }

    // 字体库目录：**每次都保证存在**（不分"这次有没有新建配置"）——界面上要写清
    // "把字体丢进哪个目录"，目录不存在那个提示就没法用。装到只读位置（Program Files）
    // 时建不出来也不能拦住 server 启动，所以这里只告警。
    // ⚠️ `ensure_dir` 建的就是**读路径**本身（`<数据目录>/resources/fonts`），
    // 所以告警里显示的与它建的是同一个对象。
    if let Err(e) = yekitrim_core::fonts::ensure_dir() {
        tracing::warn!(
            "字体库目录不可用（{}）：界面里选不了自定义字体。{e}",
            yekitrim_core::path::display(&yekitrim_core::path::font_dir())
        );
    }
    // ⚠️ **老用户目录只报一次、不删也不搬**（2026-10-02 资产收回程序目录之后）：
    // 那份 `~/.vtrim/fonts` 已经不在读取路径上了，但用户会疑惑"我导入的字体去哪了"。
    // 如实报出路径 + 体积，让用户自己决定要不要删 —— 那是用户的磁盘，
    // 里面可能还有他手工放进去的字体，**程序不替他做主**。
    if let Some((dir, count, bytes)) = yekitrim_core::fonts::legacy_user_dir_report() {
        tracing::warn!(
            "发现旧字体库 {}（{} 支 / {:.1} MB）—— 它已不再被使用，可以按需删除；\
             字体请放到 {} 里",
            yekitrim_core::path::display(&dir),
            count,
            bytes as f64 / 1_048_576.0,
            yekitrim_core::path::display(&yekitrim_core::path::font_dir())
        );
    }
    // ⚠️ **自带字体在这里自动装好**（内嵌字节，不联网、幂等）。
    // 放在 server 启动路径上是有意的：用户第一次打开 WebUI 时字体就该是"能选"的状态，
    // 而不是先去点一个「下载自带字体」。装失败只 warn，不拦启动。
    yekitrim_core::fonts::ensure_builtin_fonts();
    Ok(result)
}

/// 只做 TOML 语法校验（不校验字段语义），供保存前拦截坏文件。
pub fn validate_toml(raw: &str) -> anyhow::Result<()> {
    toml::from_str::<toml::Value>(raw)
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn parse_check(path: &Path) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(path)?;
    validate_toml(&raw)
}

pub fn show_config(config_file: Option<&Path>) -> anyhow::Result<String> {
    let cfg = if let Some(path) = config_file {
        let content = std::fs::read_to_string(path)?;
        toml::from_str(&content)?
    } else {
        yekitrim_core::config::load_config()?
    };
    let llm = yekitrim_core::config::load_llm_config();
    Ok(format!("{}\n\n# LLM 配置 (.vtrim/llm.toml)\n{}", toml::to_string_pretty(&cfg)?, toml::to_string_pretty(&llm)?))
}

fn toml_val(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('\"', "\\\"");
    if s.contains('\n') {
        format!("\"\"\"\n{}\n\"\"\"", escaped)
    } else {
        format!("\"{}\"", escaped)
    }
}

pub async fn download_models() -> anyhow::Result<()> {
    tracing::info!("下载 ASR 模型 ({})...", yekitrim_core::download::model_names().join(" / "));
    yekitrim_core::download::download_all(None).await
}
