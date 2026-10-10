//! 初始化工程与配置管理 Service。

use std::path::{Path, PathBuf};

pub fn init_project(
    input: &Path,
    danmaku: Option<&Path>,
    output_dir: Option<&str>,
    cfg_path: &Path,
) -> anyhow::Result<()> {
    if input.is_dir() {
        anyhow::bail!("'{}' 是目录路径。yekitrim init 需要视频文件路径", input.display());
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

/// 程序配置 `config.toml` 的标准路径（可执行文件旁的 `.yekitrim/`）。
/// **全局（公用）配置文件**：`<数据目录>/global/.yekitrim/config.toml`。
///
/// 口径（2026-10-10 用户拍板）：
/// - 它是**初始化模板** —— 账号首次初始化时拷一份到 `<数据>/users/<uid>/.yekitrim/`；
///   之后各自独立演进（**不是**每次读取都合并：那样改全局会悄悄改变所有人的行为，
///   而"我改了自己的配置"会莫名其妙被全局盖掉）。
/// - ⚠️ 与**账本**（`<数据>/.yekitrim/cloud/yekitrim.db`）不同：账本永远在顶层全局一份，
///   因为它是**身份的来源**；而配置是"每个账号自己的"，全局那份只是出厂默认。
pub fn global_config_path() -> PathBuf {
    yekitrim_core::owner::global_root()
        .join(yekitrim_core::path::STATE_DIR_NAME)
        .join("config.toml")
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

/// 生成程序配置（覆盖写）— `yekitrim config init`。
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

/// 程序配置缺失时补齐（`yekitrim server` 启动时调用）。
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

    // ⚠️ **不再建顶层 `resources/`**（2026-10-10）：素材库现在只有"用户库 + 全局库"两层，
    // 顶层那份是这次要消灭的。这里改为保证**全局库**存在（自带字体落在那儿）。
    if !result.created.is_empty() || !result.repaired.is_empty() {
        if let Err(e) = std::fs::create_dir_all(yekitrim_core::owner::global_resources_dir()) {
            tracing::warn!("全局素材库目录建不出来（{}）：{e}",
                yekitrim_core::path::display(&yekitrim_core::owner::global_resources_dir()));
        }
    }

    // 字体库目录：**每次都保证存在** —— 界面上要写清"把字体丢进哪个目录"，
    // 目录不存在那个提示就没法用。装到只读位置（Program Files）时建不出来
    // 也不能拦住 server 启动，所以这里只告警。
    //
    // ⚠️ **建全局那一个**（`<数据>/global/resources/fonts`）：自带字体落在这里。
    // 各账号的用户库由 `fonts::ensure_dir_for(owner)` 在写入时按需创建 —— 这里
    // 不预先给每个账号建空目录（那样新账号一多就是一堆空壳）。
    let global_fonts = yekitrim_core::owner::global_resources_dir()
        .join(yekitrim_core::owner::AssetKind::Fonts.dir_name());
    if let Err(e) = std::fs::create_dir_all(&global_fonts) {
        tracing::warn!(
            "全局字体库目录不可用（{}）：界面里选不了自定义字体。{e}",
            yekitrim_core::path::display(&global_fonts)
        );
    }
    // ⚠️ **老用户目录只报一次、不删也不搬**（2026-10-02 资产收回程序目录之后）：
    // 那份 `~/.yekitrim/fonts` 已经不在读取路径上了，但用户会疑惑"我导入的字体去哪了"。
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

/// **把全局配置当模板，给某个账号落一份**（幂等；已有就不动）。
///
/// 口径：`global/.yekitrim/{config,llm}.toml` 是**出厂默认**，账号首次初始化时拷一份，
/// 之后各自独立演进 —— **不是**每次读取都合并（那样"我改了自己的配置"会莫名被全局盖掉）。
///
/// ⚠️ 只在**账号自己的那份不存在**时拷。已存在 = 用户已经用过了，绝不许覆盖
/// （覆盖会把用户改过的 API key / 检测参数静默冲掉）。
/// ⚠️ 全局那份也不存在时**不报错**：走各处的内置默认值即可（全新安装就是这样）。
pub fn seed_account_config(owner: yekitrim_core::owner::Owner) -> anyhow::Result<Vec<PathBuf>> {
    let src_dir = yekitrim_core::owner::global_root().join(yekitrim_core::path::STATE_DIR_NAME);
    let dst_dir = yekitrim_core::owner::state_dir(owner);
    let mut out = Vec::new();
    for name in ["config.toml", "llm.toml"] {
        let from = src_dir.join(name);
        let to = dst_dir.join(name);
        if !from.is_file() || to.exists() {
            continue; // 没模板 / 用户已经有了 —— 都跳过
        }
        std::fs::create_dir_all(&dst_dir)?;
        std::fs::copy(&from, &to)?;
        out.push(to);
    }
    Ok(out)
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
    Ok(format!("{}\n\n# LLM 配置 (.yekitrim/llm.toml)\n{}", toml::to_string_pretty(&cfg)?, toml::to_string_pretty(&llm)?))
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
