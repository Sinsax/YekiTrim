//! download：ASR 模型下载与就位检查（`yekitrim download` 与转写前的 `ensure_models`）。
//!
//! 入口：`model_names`、`model_install`、`download_all`（`download_all_with` 带整批进度）、
//! `download_vad_model`、`ensure_models`；
//! 另有两个**通用**下载件给别处复用：`fetch_url_to` / `fetch_text_to`（自带字体下载用，见 `fonts::BUILTIN_FONTS`）。
//! 边界：模型落在 `<数据目录>/models/`（`YEKITRIM_DATA_DIR` 可改指；debug / release
//! 默认各一份，互不相干）；许可上别把 espeak-ng 等 GPL 组件带回来 —— 见 `crates/yekitrim-core/AGENTS.md`

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use reqwest::StatusCode;
use tokio::io::AsyncWriteExt;

use crate::config::AsrConfig;

const ASR_MODELS_BASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models";

/// 支持的 ASR 模型: (配置名, 模型目录名, 下载文件名, 已安装标记文件)
///
/// 只有真正有推理入口的模型才在这里：`single` 用 sensevoice、`hybrid` 用
/// sensevoice 打轴 + qwen3-asr 填词（见 `asr::local` / `asr::mod` 的模式分派）。
/// firered-zh 已下线（CTC 时间戳在 BGM 段造幽灵时间戳，见 ROADMAP）。
const MODEL_REGISTRY: &[(&str, &str, &str, &str)] = &[
    (
        "sensevoice",
        "sensevoice",
        "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09.tar.bz2",
        "tokens.txt",
    ),
    (
        "qwen3-asr",
        "qwen3-asr",
        "sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25.tar.bz2",
        "tokenizer/tokenizer_config.json",
    ),
];

/// 注册表里的模型名（界面/诊断列表的唯一来源，别再各处手写数组）。
pub fn model_names() -> Vec<&'static str> {
    MODEL_REGISTRY.iter().map(|(cfg, _, _, _)| *cfg).collect()
}

/// 一个模型的安装情况（**运行时视角**）。
pub struct ModelInstall {
    /// 运行时实际会用的目录（`asr::resolve_model_dir` 的解析结果）
    pub dir: PathBuf,
    /// 这个目录是从哪一路找来的
    pub source: crate::asr::local::ModelDirSource,
    /// 目录里是否已经有注册表要求的标记文件
    pub installed: bool,
}

/// 查某个模型是否可用 + 用的是哪一份。
///
/// ⚠️ 判定必须与运行时（`asr::local::resolve_model_dir`）**同一套查找顺序**，
/// 否则会出现"模型管理说没下载、转写其实一直在用用户目录那份"的错位；
/// `installed` 认的是注册表 marker，不是"目录存在"（半下载的目录不算装好）。
pub fn model_install(cfg: &AsrConfig, model: &str) -> ModelInstall {
    let (dir, source) = crate::asr::local::resolve_model_dir(cfg, model);
    let installed = model_marker(model).map(|m| dir.join(m).exists()).unwrap_or(false);
    ModelInstall { dir, source, installed }
}

fn model_archive(name: &str) -> Option<&'static str> {
    MODEL_REGISTRY
        .iter()
        .find(|(cfg, _, _, _)| *cfg == name)
        .map(|(_, _, archive, _)| *archive)
}

fn model_marker(name: &str) -> Option<&'static str> {
    MODEL_REGISTRY
        .iter()
        .find(|(cfg, _, _, _)| *cfg == name)
        .map(|(_, _, _, marker)| *marker)
}

fn model_dir(name: &str) -> Result<&'static str> {
    MODEL_REGISTRY
        .iter()
        .find(|(cfg, _, _, _)| *cfg == name)
        .map(|(_, dir, _, _)| *dir)
        .ok_or_else(|| anyhow::anyhow!("未知模型: {}", name))
}

fn hr_base_url() -> &'static str {
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/hr-files"
}

/// silero VAD 权重（裸 onnx 文件，不走 tar.bz2 归档那条路）。
///
/// 只下 int8（209KB）：判定质量与 fp32（629KB）在实听上无可见差别，体积小一个量级。
/// 想换 fp32 就把 `silero_vad.onnx` 丢进同一个目录 —— `asr::vad::find_vad_model`
/// 两个文件名都认，且优先 int8。
const VAD_MODEL_FILES: &[(&str, &str)] = &[(
    "silero_vad.int8.onnx",
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.int8.onnx",
)];

/// 下载进度（**整批口径**，字节数）。
///
/// core 只定义"报什么"，显示成什么样是上层的事（WebUI 任务条 / CLI 终端条 / 什么都不报）。
/// ⚠️ 按**字节**而不是文件数报：模型包之间差三个数量级（838 MB 对 209 KB），按文件数报
/// 等于"下完 7/9 个文件"然后卡在 99% 一小时。
pub trait DownloadProgress: Send + Sync {
    /// 开始下某一份文件。`label` 是给人看的名字（`qwen3-asr` / `lexicon.txt`）。
    fn file_started(&self, _label: &str) {}
    /// 整批累计已下载字节 / 整批总字节；`total == 0` = 预知不到总量，退回"当前文件"口径。
    fn bytes(&self, _current: u64, _total: u64) {}
    /// 一份下完、开始解包（tar.bz2 解压是分钟级同步 IO，这期间没有字节进度）。
    /// 默认不报 —— 不实现它的调用方不会因此少任何行为。
    fn extracting(&self, _label: &str) {}
    /// 是否已被请求取消。**每个数据块之后都查**：模型包是分钟级的，只在文件边界查等于没取消。
    fn cancelled(&self) -> bool {
        false
    }
}

/// 不关心进度的调用方（`config init`、转写前的 `ensure_models`）用它。
pub struct NoopDownload;

impl DownloadProgress for NoopDownload {}

/// 下载 silero VAD 权重到 `<models_dir>/silero-vad/`（已有则跳过）。
pub async fn download_vad_model(models_dir: &Path) -> Result<()> {
    download_vad_into(&models_dir.join(crate::asr::vad::MODEL_DIR_NAME)).await
}

/// 把 VAD 权重下到**给定目录**（调用方按 `resolve_model_dir` 算出来的那一处）。
///
/// ⚠️ 单独一个函数是为了 `ensure_vad_model`：运行时读取的目录由 `resolve_model_dir` 决定，
/// 它**不一定**是 `<models 根>/silero-vad` —— `[asr] local_model_dir` 指向 models 根、
/// 而 `silero-vad/` 还没下载时，resolver 会回落到**根目录本身**。
async fn download_vad_into(dir: &Path) -> Result<()> {
    if crate::asr::vad::find_vad_model(dir).is_some() {
        tracing::info!("silero VAD 模型已存在，跳过下载");
        return Ok(());
    }
    tokio::fs::create_dir_all(dir)
        .await
        .context("创建 VAD 模型目录失败")?;
    for item in pending_vad(dir) {
        run_pending(item, &Batch::noop()).await?;
    }
    Ok(())
}

/// 一件待下载：清单（[`pending_all`]）与执行（[`run_pending`]）共用同一份。
///
/// ⚠️ **"要不要下"只判一次**：算总量按一套条件、真正下载按另一套，两套早晚漂移，
/// 表现出来的就是进度条永远到不了 100%（或者到了 100% 还在传）。
struct Pending {
    /// 界面上显示的名字
    label: String,
    /// 发布页地址（`github.com/<owner>/<repo>/releases/download/<tag>/<asset>`）
    url: String,
    /// 能走 GitHub API 时换成 asset 直链（见 [`github_release_assets`]）：
    /// 同一份官方字节，但落点是 `release-assets.githubusercontent.com` 的 CDN
    api_url: Option<String>,
    /// 下载临时文件（`.part`，断了能续）
    part: PathBuf,
    /// 最终落点
    dest: PathBuf,
    /// `true` = tar.bz2 模型包：下完解到 `dest` 的父目录，再删掉包
    archive: bool,
}

/// 一次批量下载的累计器：把"某文件的第 n 字节"换算成"整批的第 n 字节"。
struct Batch<'a> {
    on: &'a dyn DownloadProgress,
    /// 前面几份文件已经下完的字节（续传过的部分也算）
    base: AtomicU64,
    /// 整批总字节；`0` = 预知不到 ⇒ 退回"当前文件"口径
    total: u64,
}

impl Batch<'static> {
    /// 不报进度的调用方用（`ensure_models` 那条路）。
    fn noop() -> Self {
        static NOOP: NoopDownload = NoopDownload;
        Self {
            on: &NOOP,
            base: AtomicU64::new(0),
            total: 0,
        }
    }
}

impl Batch<'_> {
    fn bytes(&self, file_done: u64, file_total: u64) {
        let current = self.base.load(Ordering::Relaxed) + file_done;
        let total = if self.total > 0 { self.total } else { file_total };
        self.on.bytes(current, total);
    }

    /// 一份文件下完了：算进累计基数。
    fn file_done(&self, size: u64) {
        self.base.fetch_add(size, Ordering::Relaxed);
    }
}

/// 某个模型要不要下：注册表 `marker` 在 = 已装好（半下载的目录不算）。
fn pending_model(models_root: &Path, model: &str) -> Result<Option<Pending>> {
    let marker = model_marker(model).ok_or_else(|| anyhow::anyhow!("未知模型: {}", model))?;
    let target = models_root.join(model_dir(model)?);
    if target.join(marker).exists() {
        return Ok(None);
    }
    Ok(Some(pending_in_dir(&target, model)?))
}

/// 落点已知时（`ensure_one_model` 已按运行时的查找顺序算好目录）直接建清单项。
fn pending_in_dir(target: &Path, model: &str) -> Result<Pending> {
    // ⚠️ 别 `unwrap()`：`model_marker` 与 `model_archive` 是两张表，用户手改
    // `[asr] model` 后完全可能只命中一张 —— 那是配置错误，该报错而不是 panic。
    let archive = model_archive(model).ok_or_else(|| anyhow::anyhow!("未知模型: {}", model))?;
    Ok(Pending {
        label: model.to_string(),
        url: format!("{ASR_MODELS_BASE}/{archive}"),
        api_url: None,
        part: target.join("_model.tar.bz2.part"),
        dest: target.join("_model.tar.bz2"),
        archive: true,
    })
}

/// 同音纠错文件（`lexicon.txt` + `replace.fst`）：`replace.fst` 在 = 两个都在，跳过。
fn pending_hr(model_dir: &Path) -> Vec<Pending> {
    if model_dir.join("replace.fst").exists() {
        return Vec::new();
    }
    ["lexicon.txt", "replace.fst"]
        .iter()
        .map(|name| Pending {
            label: (*name).to_string(),
            url: format!("{}/{}", hr_base_url(), name),
            api_url: None,
            part: model_dir.join(format!("{name}.part")),
            dest: model_dir.join(name),
            archive: false,
        })
        .collect()
}

/// VAD 权重：`find_vad_model` 认得（int8 / fp32）就算在，逐文件跳过。
fn pending_vad(dir: &Path) -> Vec<Pending> {
    if crate::asr::vad::find_vad_model(dir).is_some() {
        return Vec::new();
    }
    VAD_MODEL_FILES
        .iter()
        .filter(|(name, _)| !dir.join(name).exists())
        .map(|(name, url)| Pending {
            label: (*name).to_string(),
            url: (*url).to_string(),
            api_url: None,
            part: dir.join(format!("{name}.part")),
            dest: dir.join(name),
            archive: false,
        })
        .collect()
}

/// 一次 `download_all` 要下的全部文件（顺序 = 下载顺序）。
fn pending_all(home: &Path) -> Result<Vec<Pending>> {
    let models_root = home.join("models");
    let mut all = Vec::new();
    for (cfg, dir, _, _) in MODEL_REGISTRY {
        if let Some(item) = pending_model(&models_root, cfg)? {
            all.push(item);
        }
        // 同音纠错文件只服务 sensevoice，且**模型已装好时也要补**（老版本没下过）
        if *cfg == "sensevoice" {
            all.extend(pending_hr(&models_root.join(dir)));
        }
    }
    // VAD 是 single 模式的加速器：一起下（209KB），别让用户"以为开了 VAD 其实没有"
    all.extend(pending_vad(&models_root.join(crate::asr::vad::MODEL_DIR_NAME)));
    Ok(all)
}

pub async fn download_all(target_dir: Option<&Path>) -> Result<()> {
    download_all_with(target_dir, &NoopDownload).await
}

/// 下载全部 ASR 模型，并把**整批**进度报给 `progress`（CLI 不报，WebUI 报）。
pub async fn download_all_with(
    target_dir: Option<&Path>,
    progress: &dyn DownloadProgress,
) -> Result<()> {
    let home = target_dir.map_or_else(crate::path::data_dir, |p| p.to_path_buf());
    let mut pending = pending_all(&home)?;

    // 总量与下载地址一起问清楚：GitHub 的资产清单同时给出 `size` 和 `id`
    // （省掉每份文件一次 HEAD），并把 URL 换成 CDN 直链（见 `github_release_assets`）。
    let mut total = 0u64;
    let mut known = !pending.is_empty();
    let mut releases: HashMap<(String, String, String), Vec<(String, GithubAsset)>> = HashMap::new();
    for item in pending.iter_mut() {
        match resolve_source(item, &mut releases).await {
            Some(n) => total += n,
            None => known = false,
        }
    }
    let batch = Batch {
        on: progress,
        base: AtomicU64::new(0),
        total: if known { total } else { 0 },
    };

    for item in pending {
        run_pending(item, &batch).await?;
    }

    tracing::info!("下载完成！");
    tracing::info!("模型目录: {}", home.join("models").display());
    tracing::info!("运行 `yekitrim transcribe` 开始语音识别");

    Ok(())
}

/// `https://github.com/<owner>/<repo>/releases/download/<tag>/<asset>` 拆出来的四段。
struct GithubRelease<'a> {
    owner: &'a str,
    repo: &'a str,
    tag: &'a str,
    asset: &'a str,
}

fn parse_github_release(url: &str) -> Option<GithubRelease<'_>> {
    let rest = url.strip_prefix("https://github.com/")?;
    let mut it = rest.split('/');
    let owner = it.next()?;
    let repo = it.next()?;
    if it.next()? != "releases" || it.next()? != "download" {
        return None;
    }
    let tag = it.next()?;
    let asset = it.next()?;
    Some(GithubRelease {
        owner,
        repo,
        tag,
        asset,
    })
}

/// 发布里的一件资产：`id` 用来拼 API 直链，`size` 用来算进度总量。
struct GithubAsset {
    id: u64,
    size: u64,
}

/// 把 GitHub release 的**网页下载地址**换成 API 资产直链（问不到 = `None`）。
///
/// 给「单文件、非模型」的下载方复用（如 `yekitrim-service` 的 yt-dlp）：
/// 网页入口在部分网络上会连接超时（实测：同一条宽带网页入口 ~150 KB/s 且常超时，
/// API 入口 3.7 MB/s）。**这不是第三方镜像** —— 仍是 GitHub 官方那条 release。
pub async fn github_asset_direct_url(url: &str) -> Option<String> {
    github_asset_direct_url_with(url, None).await
}

/// [`github_asset_direct_url`] 的带代理版本（代理本身也要能访问 `api.github.com`）。
pub async fn github_asset_direct_url_with(url: &str, proxy: Option<&str>) -> Option<String> {
    let Some(gh) = parse_github_release(url) else {
        tracing::warn!("api 直链：URL 不是标准 release 地址: {url}");
        return None;
    };
    let Some(list) = github_release_assets_with(gh.owner, gh.repo, gh.tag, proxy).await else {
        tracing::warn!("api 直链：问不到 release 资产清单（{}）", gh.tag);
        return None;
    };
    let (_, asset) = list.iter().find(|(name, _)| name == gh.asset)?;
    Some(format!(
        "https://api.github.com/repos/{}/{}/releases/assets/{}",
        gh.owner, gh.repo, asset.id
    ))
}

/// 问一次 GitHub release 的资产清单（`name → (id, size)`）。
///
/// ⚠️ **这不是第三方镜像**：还是 GitHub 官方那一条 release，只是入口从网页域名
/// `github.com/.../releases/download/...` 换成 `api.github.com/.../releases/assets/<id>`，
/// 由它 302 到 `release-assets.githubusercontent.com` 的 CDN。实测同一条宽带：
/// 网页入口 ~150 KB/s、十几分钟后连接超时；API 入口 **3.7 MB/s**。
/// 未认证 API 限 60 次/小时，一次"下载全部"只用 2 次；问不到就退回网页入口。
async fn github_release_assets(
    owner: &str,
    repo: &str,
    tag: &str,
) -> Option<Vec<(String, GithubAsset)>> {
    github_release_assets_with(owner, repo, tag, None).await
}

async fn github_release_assets_with(
    owner: &str,
    repo: &str,
    tag: &str,
    proxy: Option<&str>,
) -> Option<Vec<(String, GithubAsset)>> {
    let mut b = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .user_agent(concat!("yekitrim/", env!("CARGO_PKG_VERSION")));
    if let Some(raw) = proxy.map(str::trim).filter(|s| !s.is_empty()) {
        // ⚠️ **必须原样传**：`socks5h://` = 代理端 DNS 解析，`socks5://` = 本地解析。
        // 改写成 socks5 会让 api.github.com 由本地 DNS 解析出被墙的 IP，请求直接失败。
        b = b.proxy(reqwest::Proxy::all(raw).ok()?);
    }
    let client = b.build().ok()?;
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/tags/{tag}");
    // ⚠️ **必须重试**：到 GitHub 的链路本身会抽风（实测同一条网络：`api.github.com`
    // 有时 3 秒返回、有时 12 秒连接超时；`github.com` 网页入口几乎必超时）。
    // 一次失败就放弃 ⇒ 用户看到"下载失败"，而其实再试两次就成了。
    let mut resp = None;
    for attempt in 1..=3u32 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => {
                resp = Some(r);
                break;
            }
            Ok(r) => {
                let code = r.status();
                if code.as_u16() == 403 || code.as_u16() == 429 {
                    // 未认证 API 限流（60 次/小时）⇒ 等一分钟再试
                    tracing::warn!("GitHub API 限流（{code}），60s 后重试");
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                } else {
                    tracing::warn!("GitHub API 返回 {code}，重试 {attempt}/3");
                    tokio::time::sleep(std::time::Duration::from_secs(2 * attempt as u64)).await;
                }
            }
            Err(e) => {
                tracing::warn!("问 GitHub release 失败（{e}），重试 {attempt}/3");
                tokio::time::sleep(std::time::Duration::from_secs(2 * attempt as u64)).await;
            }
        }
    }
    let resp = resp?;
    let v: serde_json::Value = resp.json().await.ok()?;
    let mut out = Vec::new();
    for a in v.get("assets")?.as_array()? {
        let name = a.get("name")?.as_str()?.to_string();
        let id = a.get("id")?.as_u64()?;
        let size = a.get("size").and_then(serde_json::Value::as_u64).unwrap_or(0);
        out.push((name, GithubAsset { id, size }));
    }
    Some(out)
}

/// 把一件待下载换成"最快且能算总量"的那条路，返回字节数（问不到 = `None`）。
async fn resolve_source(
    item: &mut Pending,
    releases: &mut HashMap<(String, String, String), Vec<(String, GithubAsset)>>,
) -> Option<u64> {
    if let Some(gh) = parse_github_release(&item.url) {
        let key = (
            gh.owner.to_string(),
            gh.repo.to_string(),
            gh.tag.to_string(),
        );
        if !releases.contains_key(&key) {
            let list = github_release_assets(&key.0, &key.1, &key.2)
                .await
                .unwrap_or_default();
            releases.insert(key.clone(), list);
        }
        if let Some((_, asset)) = releases
            .get(&key)
            .and_then(|list| list.iter().find(|(name, _)| name == gh.asset))
        {
            let api_url = format!(
                "https://api.github.com/repos/{}/{}/releases/assets/{}",
                gh.owner, gh.repo, asset.id
            );
            item.api_url = Some(api_url);
            return Some(asset.size);
        }
    }
    remote_size(&item.url).await
}

/// 问远端文件多大（`HEAD`，跟着 GitHub release 的 302 走到 asset）。拿不到就 `None`。
async fn remote_size(url: &str) -> Option<u64> {
    // ⚠️ 必须带超时：这只是"算总量"的预检，卡住它等于整个下载任务停在 0% 不动
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?
        .head(url)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.content_length()
}

/// 执行一件：下载 → 改名 →（模型包）解压。
async fn run_pending(item: Pending, batch: &Batch<'_>) -> Result<()> {
    if let Some(parent) = item.dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("创建目录失败: {}", parent.display()))?;
    }
    // 走了 GitHub API 就用 CDN 直链；没取到（限流/离线）就退回发布页地址 —— 慢，但能下
    let url = item.api_url.clone().unwrap_or_else(|| item.url.clone());
    tracing::info!("下载 {} ...", item.label);
    batch.on.file_started(&item.label);

    let size =
        download_with_retry(&url, &item.part, batch, PARALLEL_MIN_BYTES, PARALLEL_CHUNKS, None)
            .await?;
    tokio::fs::rename(&item.part, &item.dest)
        .await
        .with_context(|| format!("重命名 {} 失败", item.label))?;

    if item.archive {
        tracing::info!("  解压模型...");
        // 解压是分钟级、且没有字节进度：让上层把文案换成"解压 xxx"（也顺手刷一次活动时间，
        // 否则界面会在这一段误报"已几分钟没有新进展"）
        batch.on.extracting(&item.label);
        let archive = item.dest.clone();
        let target = item.dest.parent().map(Path::to_path_buf).unwrap_or_default();
        tokio::task::spawn_blocking(move || extract_model_tarbz2(&archive, &target))
            .await
            .context("模型解压线程异常")??;
        tokio::fs::remove_file(&item.dest).await.ok();
    } else {
        tracing::info!("  保存: {}", item.dest.display());
    }

    batch.file_done(size);
    Ok(())
}

/// 少于这个大小不值得分块（多几次握手，还多占一个段文件的盘）。
const PARALLEL_MIN_BYTES: u64 = 64 * 1024 * 1024;
/// 分段数上限。实测同一条宽带单流被限到 ~110 KB/s，8 路能跑到 ~4.7 MB/s。
const PARALLEL_CHUNKS: u64 = 8;
/// 每段的目标大小。**不是"总大小 ÷ 段数"** —— 段数由这个常量反推，
/// 这样"剩余 7 MB"也会分段（而不是退化成单流）。
const PARALLEL_CHUNK_BYTES: u64 = 4 * 1024 * 1024;

fn seg_path(part: &Path, i: u64) -> PathBuf {
    let mut name = part.as_os_str().to_owned();
    name.push(format!(".seg{i}"));
    PathBuf::from(name)
}

/// 多连接分段下载：把 `[existing, total)` 切成 N 段并发拉，拼回 `part_path`。
///
/// ⚠️ 存在的理由：部分网络对**单条连接**限速（实测 GitHub 的 release-assets 给到
/// ~110 KB/s），但总带宽没被限 —— 8 条并发能跑到 4.7 MB/s。838 MB 从两小时压到十分钟内。
/// 段文件 `*.seg{i}` 是临时产物，拼完就删；峰值只多一个段的大小。
async fn download_parallel(
    client: &reqwest::Client,
    url: &str,
    part_path: &Path,
    total: u64,
    existing: u64,
    batch: &Batch<'_>,
    max_chunks: u64,
) -> Result<u64> {
    let remaining = total.saturating_sub(existing);
    // ⚠️ **分段数按"每段 8 MiB"算**，不是"总大小 ÷ 16 MiB"：
    // 旧算法对"剩余 7 MB"会算出 0 段 → 退化成单流（实测单流被限到 ~30 KB/s，
    // 40 MB 的包要 20 分钟）。新算法：7 MB → 1 段 7 MB；40 MB → 5 段各 8 MB。
    let chunks = remaining.div_ceil(PARALLEL_CHUNK_BYTES).clamp(1, max_chunks.max(1));
    let chunk_size = remaining.div_ceil(chunks);

    // 清掉上一轮留下的段文件：段与 `.part` 前缀的对应关系只在"同一套边界"下成立，
    // 跨轮复用很容易拼错 —— 宁可多下一次，也别把模型拼坏。
    let mut segs = Vec::new();
    for i in 0..chunks {
        let start = existing + i * chunk_size;
        let end = (start + chunk_size).min(total);
        if start >= end {
            break;
        }
        let seg = seg_path(part_path, i);
        let _ = tokio::fs::remove_file(&seg).await;
        segs.push((seg, start, end));
    }

    let done = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let mut set = tokio::task::JoinSet::new();
    for (seg, start, end) in segs {
        let client = client.clone();
        let url = url.to_string();
        let done = done.clone();
        let stop = stop.clone();
        set.spawn(async move {
            fetch_range(&client, &url, &seg, start, end, &done, &stop).await
        });
    }

    let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
    loop {
        tokio::select! {
            _ = tick.tick() => {
                batch.bytes(existing + done.load(Ordering::Relaxed), total);
                if batch.on.cancelled() {
                    stop.store(true, Ordering::Relaxed);
                    set.abort_all();
                    while set.join_next().await.is_some() {}
                    bail!("任务已取消");
                }
            }
            joined = set.join_next() => {
                match joined {
                    Some(Ok(Ok(()))) => {}
                    Some(Ok(Err(e))) => {
                        stop.store(true, Ordering::Relaxed);
                        set.abort_all();
                        while set.join_next().await.is_some() {}
                        return Err(e);
                    }
                    Some(Err(e)) => {
                        stop.store(true, Ordering::Relaxed);
                        set.abort_all();
                        while set.join_next().await.is_some() {}
                        bail!("下载分块任务异常: {e}");
                    }
                    None => break,
                }
            }
        }
    }

    // 按顺序拼回 `.part`：段 0 从 `existing` 开始，正好接在已有前缀后面
    let mut out = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(part_path)
        .await
        .with_context(|| format!("打开临时文件失败: {}", part_path.display()))?;
    for i in 0..chunks {
        let seg = seg_path(part_path, i);
        let Ok(mut f) = tokio::fs::File::open(&seg).await else {
            continue;
        };
        tokio::io::copy(&mut f, &mut out)
            .await
            .with_context(|| format!("拼接分段失败: {}", seg.display()))?;
        tokio::fs::remove_file(&seg).await.ok();
    }
    tokio::io::AsyncWriteExt::flush(&mut out).await.ok();

    let size = tokio::fs::metadata(part_path).await.map(|m| m.len()).unwrap_or(0);
    if size != total {
        bail!(
            "分段下载拼接后大小不对（{} != {}）：保留 .part 供下次续传",
            size,
            total
        );
    }
    Ok(total)
}

/// 拉一段 `[start, end)` 到段文件（`Range` 必须真的生效，200 就是不支持，直接报错）。
async fn fetch_range(
    client: &reqwest::Client,
    url: &str,
    seg: &Path,
    start: u64,
    end: u64,
    done: &AtomicU64,
    stop: &AtomicBool,
) -> Result<()> {
    // ⚠️ **每段自己重试**：一段卡住就把整个批次拖死（实测 GitHub 的 CDN 连接会
    // 静默十几分钟，然后 `Connection timed out`）。段内从断点续（已写多少就接着要多少）。
    let want = end - start;
    let mut got: u64 = 0;
    for attempt in 1..=3u32 {
        if stop.load(Ordering::Relaxed) {
            bail!("任务已取消");
        }
        let from = start + got;
        if from >= end {
            return Ok(());
        }
        let resp = match client
            .get(url)
            .header("Accept", "application/octet-stream")
            .header("Range", format!("bytes={from}-{}", end - 1))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("分段 {start}-{end} 请求失败（{e}），重试 {attempt}/3");
                tokio::time::sleep(std::time::Duration::from_secs(2 * u64::from(attempt))).await;
                continue;
            }
        };
        if resp.status() != StatusCode::PARTIAL_CONTENT {
            bail!(
                "分段下载返回 {}（服务器不支持 Range）",
                resp.status()
            );
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(got > 0)
            .open(seg)
            .await
            .with_context(|| format!("创建分段文件失败: {}", seg.display()))?;
        let mut stream = resp;
        let mut seg_got = got;
        let mut failed: Option<anyhow::Error> = None;
        loop {
            let chunk = match stream.chunk().await {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(e) => {
                    failed = Some(anyhow::anyhow!("读取分段数据失败: {e}"));
                    break;
                }
            };
            if let Err(e) = file.write_all(&chunk).await {
                failed = Some(anyhow::anyhow!("写分段文件失败: {e}"));
                break;
            }
            seg_got += chunk.len() as u64;
            done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            if stop.load(Ordering::Relaxed) {
                bail!("任务已取消");
            }
        }
        got = seg_got;
        if got >= want {
            return Ok(());
        }
        let e = failed
            .unwrap_or_else(|| anyhow::anyhow!("分段 {start}-{end} 提前结束（{got}/{want} 字节）"));
        tracing::warn!("{e:#}，重试 {attempt}/3");
        tokio::time::sleep(std::time::Duration::from_secs(2 * u64::from(attempt))).await;
    }
    bail!("分段 {start}-{end} 下载失败（{got}/{want} 字节）")
}

/// 下载一份文件；网络断在 `.part` 上就歇几秒、**从断点续**再试（最多 3 次）。
///
/// 不是"从头再来"：`download_file` 每次都按 `.part` 现有长度发 `Range`。
/// 实测过一次 838 MB 的包在 312 MB 处 `Connection timed out` —— 没有这一层，
/// 整条任务直接失败，用户只能自己发现再点一次。
async fn download_with_retry(
    url: &str,
    part: &Path,
    batch: &Batch<'_>,
    parallel_min: u64,
    max_chunks: u64,
    proxy: Option<&str>,
) -> Result<u64> {
    let mut last: Option<anyhow::Error> = None;
    for attempt in 1..=3u32 {
        match download_file(url, part, batch, parallel_min, max_chunks, proxy).await {
            Ok(size) => return Ok(size),
            Err(e) => {
                // 取消是用户意志，不是网络抖动：别再重试
                if batch.on.cancelled() {
                    return Err(e);
                }
                tracing::warn!("下载失败（第 {attempt}/3 次）: {e:#}；稍后从断点重试");
                last = Some(e);
                tokio::time::sleep(std::time::Duration::from_secs(3 * u64::from(attempt))).await;
            }
        }
    }
    Err(last.expect("循环至少跑过一次"))
}

/// 旧调用点（`ensure_one_model`）用：按"运行时算好的模型目录"确保某个模型就位。
async fn download_model(target: &Path, model: &str) -> Result<()> {
    let marker = model_marker(model).ok_or_else(|| anyhow::anyhow!("未知模型: {}", model))?;
    if target.join(marker).exists() {
        tracing::info!("模型 {model} 已存在，跳过下载");
        return Ok(());
    }
    run_pending(pending_in_dir(target, model)?, &Batch::noop()).await
}

fn extract_model_tarbz2(archive: &Path, target: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut archive_tar = tar::Archive::new(decoder);

    for entry in archive_tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();

        let components: Vec<_> = path.components().skip(1).collect();
        if components.is_empty() {
            continue;
        }

        let rel_path: PathBuf = components.iter().collect();
        let target_path = target.join(&rel_path);

        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&target_path)?;
            continue;
        }

        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut outfile = std::fs::File::create(&target_path)?;
        std::io::copy(&mut entry, &mut outfile)?;
        tracing::info!("  解压: {}", rel_path.display());
    }

    Ok(())
}

/// 下到 `part_path`（`.part` 续传），返回这份文件的最终字节数。
///
/// `batch` 把"本文件第 n 字节"换算成整批进度；`DownloadProgress::cancelled` **每个数据块
/// 之后都查** —— 取消后停在 `.part` 上，下次从断点续，不用重下 800 MB。
async fn download_file(
    url: &str,
    part_path: &Path,
    batch: &Batch<'_>,
    parallel_min: u64,
    max_chunks: u64,
    proxy: Option<&str>,
) -> Result<u64> {
    let mut existing = std::fs::metadata(part_path)
        .map(|m| m.len())
        .unwrap_or(0);

    // ⚠️ `read_timeout` 是"多久没收到一个字节"的上限，不是总时长：卡死的连接
    // （实测 312 MB 处 `Connection timed out` 前静默了十几分钟）会在这里断掉，
    // 交给外层 `download_with_retry` 从 `.part` 续。
    // ⚠️ 三层超时缺一不可：read（读到一半断流）/ connect（连不上）/
    // 「收到响应头前一直等」交给下面 `.timeout(900s)` 兜底。少任何一层都会"卡在 0%"。
    let mut b = reqwest::Client::builder()
        .read_timeout(std::time::Duration::from_secs(60))
        .connect_timeout(std::time::Duration::from_secs(20))
        .user_agent(concat!("yekitrim/", env!("CARGO_PKG_VERSION")));
    if let Some(raw) = proxy.map(str::trim).filter(|s| !s.is_empty()) {
        // ⚠️ 原样传（`socks5h://` 是代理端解析，别改写成 `socks5://`）
        b = b.proxy(
            reqwest::Proxy::all(raw).with_context(|| format!("代理地址不合法: {raw}"))?,
        );
    }
    let client = b.build().context("创建下载客户端失败")?;
    let mut req = client
        .get(url)
        // `api.github.com/.../releases/assets/<id>` 必须带这个头才返回资产本体（否则回 JSON）
        .header("Accept", "application/octet-stream")
        // 单流只用于 < 64 MiB 的文件 ⇒ 给一个宽松的总超时，防止"连上但不回数据"卡死
        .timeout(std::time::Duration::from_secs(900));
    if existing > 0 {
        req = req.header("Range", format!("bytes={}-", existing));
    }
    let resp = req
        .send()
        .await
        .with_context(|| format!("下载失败: {url}"))?;

    let status = resp.status();
    let total = if status == StatusCode::PARTIAL_CONTENT {
        existing + resp.content_length().unwrap_or(0)
    } else if status == StatusCode::OK {
        if existing > 0 {
            std::fs::remove_file(part_path).ok();
            existing = 0;
        }
        resp.content_length().unwrap_or(0)
    } else if status == StatusCode::RANGE_NOT_SATISFIABLE {
        // 远端说"起点已经超出文件尾部" ⇒ 这个 `.part` 早就下完了
        return Ok(existing);
    } else {
        bail!("下载返回 {} ({url})", status);
    };
    let total = total.max(existing);

    // 大文件走多连接分段：单流在部分网络里会被限到 ~110 KB/s（总带宽却没限），
    // 838 MB 单流两小时、8 路十分钟内。小文件单流足够，别为它多开连接。
    if total.saturating_sub(existing) >= parallel_min {
        return download_parallel(&client, url, part_path, total, existing, batch, max_chunks).await;
    }

    let bar = indicatif::ProgressBar::new(total);
    bar.set_position(existing);
    bar.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
            .unwrap()
            .progress_chars("##-"),
    );

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(existing > 0)
        .open(part_path)
        .await
        .with_context(|| format!("创建临时文件失败: {}", part_path.display()))?;

    let mut stream = resp;
    let mut written = existing;
    batch.bytes(written, total);
    while let Some(chunk) = stream.chunk().await? {
        file.write_all(&chunk).await?;
        written += chunk.len() as u64;
        bar.inc(chunk.len() as u64);
        // `total == 0` = 不知道总量：照实报 0，别拿"已下载"当总量（那会永远显示 100%）
        batch.bytes(written, total);
        if batch.on.cancelled() {
            bar.finish_and_clear();
            bail!("任务已取消");
        }
    }

    bar.finish_and_clear();
    Ok(written)
}

/// 一条下载线路的**测速结果**。
#[derive(Debug, Clone)]
pub struct RouteSpeed {
    /// 测速用的完整下载地址
    pub url: String,
    /// 这条线路的标签（日志/前端显示用）：`api` / `web` / 镜像 host
    pub label: String,
    /// 4 秒内实际拉到的字节数 —— **唯一判据**：谁的 throughput 高用谁
    pub bytes: u64,
    /// 实际耗时（毫秒）。`bytes==0` 时也保留，用来区分"超时"和"立即失败"
    pub ms: u128,
}

impl RouteSpeed {
    /// 吞吐（字节/毫秒）；0 字节按 0 算。
    pub fn throughput(&self) -> u64 {
        if self.ms == 0 { 0 } else { (self.bytes as u128 * 1000 / self.ms) as u64 }
    }
}

/// **线路竞速**：每条候选线路各拉 4 秒，选吞吐最高的一条。
///
/// ⚠️ 为什么不"顺序试"：慢线路排前面时，后面的线路要白等它把 3 次重试跑完
/// （实测单条 3 次重试 ≈ 5 分钟，三条排下来 15 分钟才开始真正下载）。
/// 并发探针 + 短窗口（4s）= 最多 4 秒就出结论，且结论**有数据支撑**而不是猜。
///
/// `probe_bytes` 拉够了就提前结束该线路（不用真等满 4 秒）。
pub async fn race_routes(
    urls: Vec<(String, String)>,
    probe_bytes: u64,
    proxy: Option<&str>,
) -> Option<RouteSpeed> {
    use tokio::time::{timeout, Duration};
    const WINDOW: Duration = Duration::from_secs(4);

    let proxy_owned = proxy.map(str::to_string);
    let mut set = tokio::task::JoinSet::new();
    for (label, url) in urls {
        let proxy_owned = proxy_owned.clone();
        set.spawn(async move {
            let t0 = std::time::Instant::now();
            let client = build_client_with(proxy_owned.as_deref(), Duration::from_secs(8)).ok()?;
            let mut resp = client
                .get(&url)
                .header("Accept", "application/octet-stream")
                .timeout(WINDOW)
                .send()
                .await
                .ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let mut got: u64 = 0;
            while got < probe_bytes {
                match timeout(Duration::from_secs(2), resp.chunk()).await {
                    Ok(Ok(Some(c))) => got += c.len() as u64,
                    _ => break, // EOF / 读失败 / 2s 没数据 —— 都按"到此为止"算
                }
            }
            Some(RouteSpeed {
                url,
                label,
                bytes: got,
                ms: t0.elapsed().as_millis(),
            })
        });
    }

    let mut best: Option<RouteSpeed> = None;
    while let Some(r) = set.join_next().await {
        if let Ok(Some(speed)) = r {
            tracing::info!(
                "线路测速 {}: {} KB / {} ms = {} KB/s",
                speed.label,
                speed.bytes / 1024,
                speed.ms,
                speed.throughput() / 1024
            );
            if best.as_ref().is_none_or(|b| speed.throughput() > b.throughput()) {
                best = Some(speed);
            }
        }
    }
    best
}

/// 按 `[ingest] proxy` 建 reqwest client（`None`/空 = 直连）。
///
/// ⚠️ **为什么不靠环境变量**：`reqwest` 只认 `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`，
/// 而 WebUI 服务常常是 systemd / 计划任务起的，环境里根本没有你 shell 里 `export` 的那份
/// —— 症状是"我明明配了代理，它还是直连超时"。显式配置项各平台一致，也不受启动方式影响。
///
/// 接受的写法：`http://host:port`、`https://host:port`、`socks5://host:port`、
/// `socks5h://host:port`（**远程 DNS**，墙外站点建议用它）。
pub fn build_client(proxy: Option<&str>) -> Result<reqwest::Client> {
    build_client_with(proxy, std::time::Duration::from_secs(20))
}

/// [`build_client`] 的可调连接超时版本（测速用更短的窗口）。
pub fn build_client_with(
    proxy: Option<&str>,
    connect_timeout: std::time::Duration,
) -> Result<reqwest::Client> {
    let mut b = reqwest::Client::builder()
        .connect_timeout(connect_timeout)
        .user_agent(concat!("yekitrim/", env!("CARGO_PKG_VERSION")));
    if let Some(raw) = proxy.map(str::trim).filter(|s| !s.is_empty()) {
        // socks5h → socks5：reqwest 不认 `h` 后缀，但默认就是远程解析（要本地解析用 socks5+ 语义）
        // ⚠️ 原样传（`socks5h://` 是代理端解析，别改写成 `socks5://`）
        let p = reqwest::Proxy::all(raw)
            .with_context(|| format!("代理地址不合法: {raw}（例：socks5://127.0.0.1:20170）"))?;
        b = b.proxy(p);
    }
    b.build().context("创建下载客户端失败")
}

/// **通用可续传下载**：把 `url` 的内容下到 `part_path`（`.part` 语义），返回最终字节数。
///
/// 给「非模型」的大文件复用（yt-dlp 就是一个 40 MB 的单文件）：
/// - **断点续传**：网络中断后重试从 `.part` 现有长度接着下（实测一次 40 MB 的包在 33 MB 处
///   `Connection timed out`，没有续传就得从头再来）；
/// - **多连接分段**：单流在部分网络里被限到 ~30 KB/s，2~8 路并发能快好几倍；
/// - **重试 3 次**：到 GitHub 的链路会抽风（实测网页入口几乎必超时）。
///
/// `parallel_min` = 超过多少字节才值得分段（每段至少 16 MiB）；`max_chunks` = 最多几路并发。
pub async fn download_resumable(
    url: &str,
    part_path: &Path,
    parallel_min: u64,
    max_chunks: u64,
    proxy: Option<&str>,
) -> Result<u64> {
    let batch = Batch::noop();
    download_with_retry(url, part_path, &batch, parallel_min, max_chunks, proxy).await
}

/// **通用**下载：把 `url` 的内容落到 `dest`（先写 `dest`，调用方自己决定用不用 `.part`）。
///
/// 与 `download_file` 的区别：那个是"模型那条路"的带续传 + 进度条实现（写 `.part` 再改名），
/// 这个是给**别的调用方**（自带字体）用的最小件 —— 同一套 `reqwest` 与错误口径，不再各写一份。
/// ⚠️ 目标目录必须已存在（调用方建），失败**不留半个文件**。
pub async fn fetch_url_to(url: &str, dest: &Path) -> Result<()> {
    let resp = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .with_context(|| format!("请求失败: {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        bail!("下载返回 {} ({url})", status);
    }
    let bytes = resp.bytes().await.with_context(|| format!("读响应体失败: {url}"))?;
    if bytes.is_empty() {
        bail!("下载到 0 字节 ({url})");
    }
    tokio::fs::write(dest, &bytes)
        .await
        .with_context(|| format!("写入失败: {}", dest.display()))?;
    Ok(())
}

/// **通用**下载：取一段**文本**（许可原文这类）落到 `dest`。
///
/// 与 [`fetch_url_to`] 分开是因为"文本"与"二进制"的失败语义不同：文本拿不到通常只该警告
/// （字体本体已校验过、合法），不该把整件事判失败。
pub async fn fetch_text_to(url: &str, dest: &Path) -> Result<()> {
    let text = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .with_context(|| format!("请求失败: {url}"))?
        .error_for_status()
        .with_context(|| format!("下载返回非 2xx: {url}"))?
        .text()
        .await
        .with_context(|| format!("读文本失败: {url}"))?;
    if text.trim().is_empty() {
        bail!("下载到空文本 ({url})");
    }
    tokio::fs::write(dest, text.as_bytes())
        .await
        .with_context(|| format!("写入失败: {}", dest.display()))?;
    Ok(())
}

/// 下载同音纠错文件 (lexicon.txt + replace.fst)
async fn download_hr_files(target: &Path) -> Result<()> {
    if target.join("replace.fst").exists() {
        tracing::info!("同音纠错文件已存在，跳过下载");
        return Ok(());
    }
    for item in pending_hr(target) {
        run_pending(item, &Batch::noop()).await?;
    }
    Ok(())
}

pub async fn ensure_models(asr: &AsrConfig) -> Result<()> {
    let configured = if asr.model.is_empty() { "sensevoice" } else { asr.model.as_str() };
    let model = if model_marker(configured).is_some() {
        configured
    } else {
        // 老配置里可能还写着已下线的模型（如 firered-zh）：只告警并回退，
        // 别把整个转写任务直接打死（`create_recognizer` 本来就固定用 SenseVoice）。
        tracing::warn!(
            "[asr] model = {configured:?} 不是受支持的模型（可选: {}），这次按 sensevoice 走",
            model_names().join(" / ")
        );
        "sensevoice"
    };
    for name in required_models(asr, model) {
        ensure_one_model(asr, name).await?;
    }
    // VAD 权重同样要在转写前就位：不然跑到转写里才发现没有，静默回退固定分块 ——
    // 用户以为 VAD 生效了、实际一点没省。失败只告警（VAD 是加速器，不是必要条件）。
    if asr.vad_enabled() && asr.mode.trim().eq_ignore_ascii_case("single") {
        if let Err(e) = ensure_vad_model(asr).await {
            tracing::warn!("silero VAD 模型准备失败（{e:#}），本次转写将回退固定分块");
        }
    }
    Ok(())
}

/// VAD 权重的下载目标 = **运行时读取的那一处**（`resolve_model_dir` 是唯一真源）。
///
/// ⚠️ **别在调用方用 `dir.parent()` 反推"models 根"**：`[asr] local_model_dir` 指向
/// models 根、而 `silero-vad/` 还没下载时，resolver 会回落成**根目录本身** —— 那 `parent()`
/// 就是 models 根外面（配置目录是相对路径时甚至是空路径）。权重会下到别处、
/// `find_vad_model(dir)` 永远找不到，VAD 静默失效，还往数据目录外留一个 `silero-vad/`。
fn vad_download_dir_in(
    cfg: &AsrConfig,
    exe_home: &Path,
) -> (PathBuf, crate::asr::local::ModelDirSource) {
    crate::asr::local::resolve_model_dir_in(cfg, crate::asr::vad::MODEL_DIR_NAME, exe_home)
}

/// 确保 VAD 权重落在**运行时真正会读的**目录里（与 `vad_model_path` 同一套查找顺序）。
async fn ensure_vad_model(asr: &AsrConfig) -> Result<()> {
    let (dir, source) = vad_download_dir_in(asr, &crate::path::data_dir());
    if crate::asr::vad::find_vad_model(&dir).is_some() {
        tracing::info!("silero VAD 已就绪（{}）", dir.display());
        return Ok(());
    }
    tokio::fs::create_dir_all(&dir).await?;
    tracing::info!(
        "下载 silero VAD 模型到 {}（{}）",
        dir.display(),
        source.label()
    );
    // 落点就是上面 resolve 出来的 `dir` —— 与运行时 `vad_model_path` 逐字节同一处。
    download_vad_into(&dir).await?;
    if crate::asr::vad::find_vad_model(&dir).is_none() {
        bail!("VAD 模型下载后仍未找到（{}）", dir.display());
    }
    Ok(())
}

/// 这次转写**真正要读**的模型（判据与 `asr/mod.rs` 的 `mode == "single"` 一致）。
///
/// - **SenseVoice 一律要在位**：`asr::local::create_recognizer` 固定用它打轴，
///   `[asr] model` 写 `qwen3-asr` 也一样（那个键只影响下载清单，运行时选不了模型）。
/// - `single` 之外（含空值）= hybrid：**成片字幕的文本来自 qwen3-asr** —— 缺它时
///   `create_qwen3_recognizer` 会在打轴**跑完之后**才失败，而普通用户无权在设置页下载模型，
///   等于白等一场。所以两个都在同一处确保（这也是转写任务占着 `models` 资源键的由来）。
///
/// ⚠️ 以前这里是 `vec![model]`，于是 `model = "qwen3-asr"` 时**不会**下 sensevoice：全新机器上
/// 打轴第一步就报「模型目录 …/sensevoice 中未找到模型文件」。
fn required_models<'a>(asr: &AsrConfig, model: &'a str) -> Vec<&'a str> {
    let mut out: Vec<&'a str> = vec!["sensevoice"];
    if asr.mode.trim() != "single" {
        out.push("qwen3-asr");
    }
    // `[asr] model` 指定的那一份也要确保（去重，别下两次）
    if !out.contains(&model) {
        out.push(model);
    }
    out
}

/// 确保**运行时真正会读的那一份**模型目录存在且完整。
///
/// ⚠️ 目录一律走 `model_install`（= 运行时的 `asr::resolve_model_dir`），**别自己拼目录**。
/// 这条纪律来自一次真实事故：当年这里直接拼程序目录，而运行时优先用户目录，后果有两个 ——
///   ① 同一份模型被下载两次（程序目录那份永远不会被读）；
///   ② 同音纠错文件（`lexicon.txt` + `replace.fst`）下到程序目录，而识别器读的是用户目录
///      那份 → **同音纠错从未生效**（`create_recognizer` 只看到"没有这两个文件"）。
/// 2026-10-02 资产收回程序目录之后，两边**恰好**落在同一个地方了，但这条纪律**照样成立**：
/// `resolve_model_dir` 还多认一层 `local_model_dir`（用户可指向自备模型），自己拼就会漏掉它。
async fn ensure_one_model(asr: &AsrConfig, model: &str) -> Result<()> {
    if model_marker(model).is_none() {
        anyhow::bail!(
            "未知 ASR 模型: {} (可选: {})",
            model,
            model_names().join(" / ")
        );
    }
    let install = model_install(asr, model);
    let model_path = install.dir.clone();

    if !install.installed {
        tokio::fs::create_dir_all(&model_path).await?;
        tracing::info!(
            "下载模型 {model} 到 {}（{}）",
            model_path.display(),
            install.source.label()
        );
        download_model(&model_path, model).await?;
    } else {
        tracing::info!("本地 ASR 模型已就绪 ({model}, {})", model_path.display());
    }

    if model == "sensevoice" {
        // 静默下载同音纠错文件（不影响主流程）——下到同一个被读的目录
        download_hr_files(&model_path).await?;
    }

    Ok(())
}

#[cfg(test)]
mod model_install_tests {
    use super::*;

    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "yekitrim-model-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&base).unwrap();
            TmpDir(base)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn cfg_with(local_model_dir: &Path) -> AsrConfig {
        AsrConfig {
            local_model_dir: local_model_dir.to_string_lossy().into_owned(),
            ..Default::default()
        }
    }

    /// 界面/诊断的"已安装"必须和运行时**同一套查找顺序**（配置 → 用户目录 → 程序目录），
    /// 而且认的是注册表 marker：目录在、marker 不在 = 没装好（下到一半的 `.part` 目录）。
    #[test]
    fn model_install_follows_runtime_lookup_and_marker() {
        let t = TmpDir::new("marker");
        let models = t.path().join("models");
        let qwen = models.join("qwen3-asr");
        std::fs::create_dir_all(&qwen).unwrap();

        let cfg = cfg_with(&models);
        // 目录在、marker 不在 → 未安装（旧实现只看 dir.exists()，会把半下载判成已装）
        let st = model_install(&cfg, "qwen3-asr");
        assert!(!st.installed, "没有 marker 不应算已安装");
        assert!(st.dir.ends_with("qwen3-asr"), "{}", st.dir.display());

        // 补上注册表要求的 marker → 已安装，且来源是配置目录
        let tk = qwen.join("tokenizer");
        std::fs::create_dir_all(&tk).unwrap();
        std::fs::write(tk.join("tokenizer_config.json"), b"{}").unwrap();
        let st = model_install(&cfg, "qwen3-asr");
        assert!(st.installed);
        assert_eq!(st.source, crate::asr::local::ModelDirSource::Config);

        // sensevoice 的 marker 是 tokens.txt
        let sv = models.join("sensevoice");
        std::fs::create_dir_all(&sv).unwrap();
        assert!(!model_install(&cfg, "sensevoice").installed);
        std::fs::write(sv.join("tokens.txt"), b"a 1\n").unwrap();
        assert!(model_install(&cfg, "sensevoice").installed);

        // 未知模型 → 不 panic、判未安装
        assert!(!model_install(&cfg, "firered-zh").installed);
        assert!(!model_names().contains(&"firered-zh"));
    }

    /// VAD 权重的落点必须 = **运行时读取的那一处**（`resolve_model_dir` 的口径）。
    ///
    /// 回归：`ensure_vad_model` 曾用 `dir.parent()` 反推 models 根 —— `[asr] local_model_dir`
    /// 指向 models 根、而 `silero-vad/` 还没下载时，resolver 回落到**根目录本身**，
    /// parent 就跑到 models 根外面去了：权重下错地方、`find_vad_model(dir)` 永远找不到，
    /// VAD 静默失效（还往数据目录外写了一个目录）。
    #[test]
    fn vad_download_target_is_the_runtime_read_dir() {
        let t = TmpDir::new("vad-target");
        let home = t.path().join("state");
        let models = home.join("models");
        std::fs::create_dir_all(&models).unwrap();

        // ① 默认布局：<state>/models/silero-vad
        assert_eq!(
            vad_download_dir_in(&AsrConfig::default(), &home).0,
            models.join(crate::asr::vad::MODEL_DIR_NAME)
        );

        // ② 配置指向 models 根、silero-vad 还没下 ⇒ resolver 回落成根目录本身；
        //    落点必须是根目录，**不是它的父目录**（旧实现显式取 parent）。
        let cfg = cfg_with(&models);
        let (dir, _) = vad_download_dir_in(&cfg, &home);
        assert_eq!(dir, models);
        assert_ne!(dir, models.parent().unwrap().to_path_buf());

        // ③ 配置直接指向一个 VAD 目录 ⇒ 落点就是它
        let vad_dir = t.path().join("custom-vad");
        std::fs::create_dir_all(&vad_dir).unwrap();
        assert_eq!(vad_download_dir_in(&cfg_with(&vad_dir), &home).0, vad_dir);
    }

    /// 转写前要确保的模型随 `[asr] mode` 变：hybrid 缺 qwen3-asr 就是「打轴跑完才失败」。
    ///
    /// 回归钉子：`ensure_models` 曾经只按 `[asr] model`（默认 sensevoice）下载，而
    /// hybrid 成片字幕的文本来自 qwen3-asr → 全新部署上第一次转写必然在 SenseVoice
    /// 打轴之后报"创建 Qwen3-ASR recognizer 失败"，普通用户又无权在设置页下载。
    #[test]
    fn hybrid_also_requires_qwen3_model() {
        let mut cfg = AsrConfig::default();
        assert_eq!(cfg.mode, "hybrid", "默认模式变了就要同步这条测试");
        assert_eq!(required_models(&cfg, "sensevoice"), ["sensevoice", "qwen3-asr"]);

        // 空值按 hybrid 走（与 `asr/mod.rs` 的 `mode == "single"` 判据一致）
        cfg.mode = String::new();
        assert_eq!(required_models(&cfg, "sensevoice"), ["sensevoice", "qwen3-asr"]);

        // single 只要 SenseVoice
        cfg.mode = "single".into();
        assert_eq!(required_models(&cfg, "sensevoice"), ["sensevoice"]);

        // `[asr] model = qwen3-asr` 也**必须**带上 sensevoice（打轴固定用它），
        // 且别重复下载同一份。老实现返回 ["qwen3-asr"] ⇒ 全新机器打轴必失败。
        cfg.mode = "hybrid".into();
        assert_eq!(required_models(&cfg, "qwen3-asr"), ["sensevoice", "qwen3-asr"]);
        // single 模式下 `model` 指向 qwen3 也只需 sensevoice（用不上 qwen3），但仍去重
        cfg.mode = "single".into();
        assert_eq!(required_models(&cfg, "sensevoice"), ["sensevoice"]);
    }
}

#[cfg(test)]
mod download_plan_tests {
    use super::*;

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "yekitrim-dl-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&base).unwrap();
            Tmp(base)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, b"x").unwrap();
    }

    fn labels(items: &[Pending]) -> Vec<String> {
        items.iter().map(|p| p.label.clone()).collect()
    }

    /// 清单必须**只**列真正缺的东西：算总量与真正下载共用它。
    /// 多列一项 = 进度条永远到不了 100%；少列一项 = 到了 100% 还在传。
    #[test]
    fn pending_list_is_exactly_what_is_missing() {
        let t = Tmp::new("plan");
        let models = t.0.join("models");
        std::fs::create_dir_all(&models).unwrap();

        assert_eq!(
            labels(&pending_all(&t.0).unwrap()),
            [
                "sensevoice",
                "lexicon.txt",
                "replace.fst",
                "qwen3-asr",
                "silero_vad.int8.onnx"
            ]
        );

        // sensevoice 装好（marker 在）→ 主包消失，但同音纠错文件还在清单里
        // （老版本只下主包，这两个文件常常缺）
        touch(&models.join("sensevoice/tokens.txt"));
        assert_eq!(
            labels(&pending_all(&t.0).unwrap()),
            ["lexicon.txt", "replace.fst", "qwen3-asr", "silero_vad.int8.onnx"]
        );

        touch(&models.join("sensevoice/replace.fst"));
        assert_eq!(
            labels(&pending_all(&t.0).unwrap()),
            ["qwen3-asr", "silero_vad.int8.onnx"]
        );

        // qwen3 的 marker 是嵌套文件：目录在、marker 不在 = 半下载，仍要重下
        touch(&models.join("qwen3-asr/tokenizer/tokenizer_config.json"));
        touch(&models.join("silero-vad/silero_vad.int8.onnx"));
        assert!(
            pending_all(&t.0).unwrap().is_empty(),
            "全都齐了就不该再有待下载项"
        );
    }

    /// GitHub 发布页 URL 必须能拆出 owner/repo/tag/asset：拆不出就退回网页入口（慢），
    /// 拆错就会去问一个不存在的 release（直接失败）。
    #[test]
    fn github_release_urls_are_parsed_exactly() {
        let u = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25.tar.bz2";
        let gh = parse_github_release(u).expect("标准发布地址应能拆");
        assert_eq!(
            (gh.owner, gh.repo, gh.tag),
            ("k2-fsa", "sherpa-onnx", "asr-models")
        );
        assert_eq!(gh.asset, "sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25.tar.bz2");

        assert!(parse_github_release("https://example.com/a/b").is_none());
        assert!(
            parse_github_release("https://github.com/k2-fsa/sherpa-onnx/archive/refs/tags/x.tar.gz")
                .is_none()
        );
        // 少一段（没有 asset）不能硬拆成空名
        assert!(
            parse_github_release(
                "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models"
            )
            .is_none()
        );
    }

    struct Recorder(std::sync::Mutex<Vec<(u64, u64)>>);

    impl DownloadProgress for Recorder {
        fn bytes(&self, current: u64, total: u64) {
            self.0.lock().unwrap().push((current, total));
        }
    }

    /// 整批口径：第二份文件从第一份的末尾接着数 —— 每换一个文件归零，
    /// 用户就会以为"怎么又从头下了"。
    #[test]
    fn batch_reports_cumulative_bytes() {
        let rec = Recorder(std::sync::Mutex::new(Vec::new()));
        let batch = Batch {
            on: &rec,
            base: AtomicU64::new(0),
            total: 1000,
        };
        batch.bytes(100, 300);
        batch.file_done(300);
        batch.bytes(50, 300);
        assert_eq!(*rec.0.lock().unwrap(), [(100, 1000), (350, 1000)]);
    }


    /// 预知不到总量时退回"当前文件"口径（`total` 照实报本文件大小或 0），
    /// 别拿"已下载"当总量 —— 那会永远显示 100%。
    #[test]
    fn batch_falls_back_to_single_file_totals() {
        let rec = Recorder(std::sync::Mutex::new(Vec::new()));
        let batch = Batch {
            on: &rec,
            base: AtomicU64::new(0),
            total: 0,
        };
        batch.bytes(100, 300);
        batch.bytes(200, 0);
        assert_eq!(*rec.0.lock().unwrap(), [(100, 300), (200, 0)]);
    }
}
