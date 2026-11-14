//! 初始化工程与配置管理 Service。

use std::path::{Path, PathBuf};

/// 初始化一个工程（写 `yekitrim.toml`）。
///
/// `force = false`（默认）时**拒绝覆盖已存在的工程配置**：`write_project_config` 是整份重写，
/// 会把用户调过的素材清单（`[project.materials]`，也就是 ASR 槽位）、`transcript_file`、
/// `[cutting] output_dir` 一起换回模板值 —— `output_dir` 一变，既有产物在工程视角里直接消失。
/// API 侧早就有这道确认（`POST /projects {source.overwrite}`，有用例钉住），CLI 以前没有。
/// `force = true` 时先把手头这份备份成 `<名字>.bak-<yyyymmdd-hhmmss>` 再重建。
pub fn init_project(
    input: &Path,
    danmaku: Option<&Path>,
    output_dir: Option<&str>,
    cfg_path: &Path,
    force: bool,
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
    if cfg_path.exists() {
        if !force {
            anyhow::bail!(
                "{} 已存在，本次未做任何改动（不会覆盖你已经调过的工程设置）。\n\
                 要按当前视频重建它，请加 --force：会先备份成 {} 再写。",
                cfg_path.display(),
                backup_path(cfg_path).display()
            );
        }
        let backup = backup_path(cfg_path);
        std::fs::copy(cfg_path, &backup).map_err(|e| {
            anyhow::anyhow!("覆盖前备份 {} 失败（已放弃重建）: {e:#}", cfg_path.display())
        })?;
        tracing::warn!(
            "工程配置已存在，先备份为 {} 再按 --force 重建",
            backup.display()
        );
    }
    yekitrim_core::config::write_project_config(cfg_path, input, danmaku, output_dir)?;
    Ok(())
}

/// 覆盖前的备份名：`<文件名>.bak-<yyyymmdd-hhmmss>`（与原文件同目录，秒级时间戳）。
fn backup_path(cfg_path: &Path) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let name = cfg_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "yekitrim.toml".to_string());
    cfg_path.with_file_name(format!("{name}.bak-{stamp}"))
}

/// 程序配置 `config.toml` 的标准路径 —— **就是 `config::global_config_path()`**。
///
/// ⚠️ 口径（2026-11-13 收敛，方案 A）：程序配置**只有一个位置** =
/// `owner::config_path(Owner::from_env())` = 当前管理员的 `users/<uid>/.yekitrim/config.toml`。
/// 顶层 `<数据>/.yekitrim/config.toml` **已退役**（启动时一次性迁走并改名留档）：
/// 那份与账号树那份是同一个模板长出来的两兄弟，各被一半消费者读到 ——
/// 症状就是"设置保存成功、行为完全没变"（2026-11-13 用户实测）。
///
/// **别再在别处拼 `.yekitrim/config.toml`**，一律经这里 / `global_config_path()`。
pub fn global_config_path() -> PathBuf {
    yekitrim_core::config::global_config_path()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 占位视频 + 工程配置路径（`init_project` 只查扩展名与存在性）。
    fn project(tmp: &Path) -> (PathBuf, PathBuf) {
        let video = tmp.join("video.mp4");
        std::fs::write(&video, b"x").expect("写占位视频");
        (video, tmp.join("yekitrim.toml"))
    }

    fn list_backups(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .map(|n| n.to_string_lossy().contains(".bak-"))
                            .unwrap_or(false)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// **回归钉子（P0）**：工程配置已存在时 `init` 必须**拒绝**，且原文件一个字节都不动。
    ///
    /// 现场：重跑一次 `yekitrim init` 会把 `[project.materials]`（ASR 槽位清单）/
    /// `transcript_file` / `[cutting] output_dir` 全换回模板值（rc=0、无提示、无备份）；
    /// `output_dir` 一变，既有产物在工程视角里直接消失。
    #[test]
    fn init_refuses_to_clobber_existing_config() {
        let tmp = tempfile::TempDir::new().expect("临时目录");
        let (video, cfg) = project(tmp.path());
        init_project(&video, None, None, &cfg, false).expect("首次初始化应成功");
        // 模拟用户改过的工程：换成一份手工内容当哨兵
        let sentinel = "[project]\ninput_file = \"video.mp4\"\ntranscript_file = \"subs/fixed.srt\"\n";
        std::fs::write(&cfg, sentinel).expect("写哨兵");

        let err = init_project(&video, None, None, &cfg, false).expect_err("已存在必须拒绝");
        let msg = format!("{err:#}");
        assert!(msg.contains("--force"), "提示里要给出出路（--force）: {msg}");
        assert_eq!(
            std::fs::read_to_string(&cfg).expect("读回"),
            sentinel,
            "拒绝时原文件一个字节都不许动"
        );
        let backups = list_backups(tmp.path());
        assert!(backups.is_empty(), "拒绝时不该产生备份: {backups:?}");
    }

    /// `--force` 才重建，而且**先备份**（原样回退的唯一手段）。
    #[test]
    fn force_backs_up_before_rebuilding() {
        let tmp = tempfile::TempDir::new().expect("临时目录");
        let (video, cfg) = project(tmp.path());
        init_project(&video, None, None, &cfg, false).expect("首次初始化");
        let sentinel = "[project]\ninput_file = \"video.mp4\"\ntranscript_file = \"subs/fixed.srt\"\n";
        std::fs::write(&cfg, sentinel).expect("写哨兵");

        init_project(&video, None, None, &cfg, true).expect("--force 应成功");
        let backups = list_backups(tmp.path());
        assert_eq!(backups.len(), 1, "要留一份备份: {backups:?}");
        assert_eq!(
            std::fs::read_to_string(&backups[0]).expect("读备份"),
            sentinel,
            "备份必须是覆盖前那份原文"
        );
        let rebuilt = std::fs::read_to_string(&cfg).expect("读新配置");
        assert!(
            rebuilt.contains("input_file = \"video.mp4\""),
            "新配置要指向本次视频:\n{rebuilt}"
        );
        // 模板拼接坏了（例如版本键落错表）会在这里被 TOML 解析拦住
        validate_toml(&rebuilt).expect("重建的工程配置必须是合法 TOML");
    }
}
