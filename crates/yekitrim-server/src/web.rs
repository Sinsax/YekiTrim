//! WebUI 静态资源服务。
//!
//! 两个来源，**按构建形态自动选**：
//!
//! | 形态 | 来源 | 为什么 |
//! |---|---|---|
//! | **release** | **内嵌**（`build.rs` 把 `webui/dist` 编进二进制） | 于是"只换 exe 就能升级"，而且"新 exe + 旧 dist"这种错配**在物理上不存在**（原先它会发生，且**静默**：候选链会老老实实命中 exe 旁边那份旧 dist） |
//! | **debug** | **磁盘**（下面那四级候选） | 保住开发常态：`npm run build`（1.4 s）后刷新即生效，**不用重新链接服务端** |
//!
//! 显式覆盖：`--webui-dir <path>` 或环境变量 `YEKITRIM_WEBUI_DIR`（**任何形态**都优先走磁盘）——
//! 用途是"生产上热修一份界面"与排障。
//!
//! ⚠️ **开发首选其实是 `npm run dev`**（vite dev server + HMR，:5173，`/api` 与 `/media` 反代到本服务）：
//! 那条路前端资源由 vite 自己发，**根本不经过这里**，所以内嵌对日常改界面的速度**零影响**。
//! 磁盘这条候选链主要兜 `cargo run` / `npm run build` + 刷新那种形态。
//!
//! ⚠️ **别再回退到"手写的内嵌单文件页面"**：那份页面长期与 API 漂移，已删除。现在内嵌的是
//! **构建产物本身**（同一批文件，只是换了载体），不是第二份实现。
//!
//! 内嵌表由 `crates/yekitrim-server/build.rs` 生成；`dist` 缺失或比 `webui/src` 旧都会让**编译失败**。

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode, header};
use axum::response::IntoResponse;

// 由 build.rs 生成：`pub static WEBUI_DIST: &[(&str, &[u8])]`（键是 `/` 分隔的相对路径）
include!(concat!(env!("OUT_DIR"), "/webui_dist.rs"));

/// 显式指定 WebUI 目录的环境变量（`--webui-dir` 优先级更高）。
pub const WEBUI_DIR_ENV: &str = "YEKITRIM_WEBUI_DIR";

const INDEX: &str = "index.html";

/// 当前 WebUI 资源的来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebuiSource {
    /// 编进二进制的 `webui/dist`（release 的默认）
    Embedded,
    /// 磁盘上的目录（debug 的默认，或显式覆盖）
    Disk(PathBuf),
}

/// `--webui-dir` 的一次性登记（由 `run_with` 启动时写入）。
static OVERRIDE: OnceLock<Option<PathBuf>> = OnceLock::new();

/// 登记 `--webui-dir`。启动时调一次；后调无效（进程内只认第一次，避免两处配置打架）。
pub fn set_webui_dir(dir: Option<PathBuf>) {
    let _ = OVERRIDE.set(dir);
}

fn override_dir() -> Option<PathBuf> {
    OVERRIDE.get().cloned().flatten().or_else(|| {
        std::env::var_os(WEBUI_DIR_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    })
}

/// `dist` 候选目录（**磁盘**那条路），按优先级：
/// 1. 可执行文件同级 `webui/dist` —— 发布包形状 / 手动放一份覆盖
/// 2. 可执行文件上两级 `webui/dist` —— **开发树**（`target/debug/yekitrim.exe` 往上两级是仓库根）
/// 3. 仓库内 `webui/dist`（构建期常量）—— **只当最后的保险**
/// 4. 当前工作目录下 `webui/dist`
///
/// ⚠️ **次序是 2026-10-02 改的：构建期常量从前排挪到了保险位**（和 `render::dhampir::find_translator`
/// 同一个形状）。原先它排第一，而 `CARGO_MANIFEST_DIR` 是**编译期烧进去的绝对路径** ——
/// 于是「exe 同级」那条**永远轮不到**：在开发机上把 exe 拷走、或从别的 cwd 起服务，
/// 它照旧端出**这台机器源码树里**的 `webui/dist`，看不出任何异常（实测：
/// `target/debug/webui/dist` 不存在，命中的是构建期那条）。
///
/// ⚠️ 第 3 条的 `../..` 是**按 crate 深度**算的：`crates/yekitrim-server` → 仓库根。
/// **别把本 crate 挪到更深的目录**（如 `crates/server/yekitrim-server`）——
/// 那时 `../..` 指到 `crates/`，WebUI 会静默 404（构建与测试都不报错）。
fn candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("webui").join("dist"));
            if let Some(up2) = dir.parent().and_then(Path::parent) {
                out.push(up2.join("webui").join("dist"));
            }
        }
    }
    out.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("webui")
            .join("dist"),
    );
    out.push(PathBuf::from("webui").join("dist"));
    out
}

/// 磁盘候选里第一个含 `index.html` 的。
fn disk_dir() -> Option<PathBuf> {
    candidates().into_iter().find(|c| c.join(INDEX).is_file())
}

/// 选来源的**纯函数**（把环境量当参数传进来，四种组合才能被钉住而不受本机状态影响）。
fn choose(override_dir: Option<PathBuf>, debug: bool, disk: Option<PathBuf>) -> WebuiSource {
    if let Some(dir) = override_dir {
        return WebuiSource::Disk(dir); // 显式指定：即便指错也照它报错，别偷偷换一份
    }
    if debug {
        if let Some(dir) = disk {
            return WebuiSource::Disk(dir);
        }
    }
    WebuiSource::Embedded
}

/// 当前生效的来源。
pub fn source() -> WebuiSource {
    choose(override_dir(), cfg!(debug_assertions), disk_dir())
}

/// 内嵌资源的文件数（启动日志与排障用）。
pub fn embedded_file_count() -> usize {
    WEBUI_DIST.len()
}

/// 启动时判断 WebUI 是否可用（不可用只提示，不阻止 server 启动）。
pub fn dist_ready() -> bool {
    match source() {
        WebuiSource::Embedded => WEBUI_DIST.iter().any(|(p, _)| *p == INDEX),
        WebuiSource::Disk(dir) => dir.join(INDEX).is_file(),
    }
}

/// 给错误页与启动日志用的"当前从哪读"。
pub fn source_label() -> String {
    match source() {
        WebuiSource::Embedded => format!("内嵌（{} 个文件）", embedded_file_count()),
        WebuiSource::Disk(dir) => format!("磁盘 {}", dir.display()),
    }
}

/// 请求路径是否安全（两条来源共用同一套判断，行为才不会分叉）。
///
/// 只允许 `Normal` 分量：`..`、空分量、绝对路径、盘符、`\` 都会被挡。
/// ⚠️ 内嵌那条是按字符串**精确匹配**表里的键，`..` 本来就匹配不到东西；
/// 仍然统一走这里 —— 行为一致，且防住以后有人把匹配改成前缀匹配。
fn safe_rel(uri_path: &str) -> Option<&str> {
    // 只剥**一个**前导斜杠：`//etc/passwd` 剥完仍以 `/` 开头 ⇒ 下面按 RootDir 被挡
    let rel = uri_path.strip_prefix('/').unwrap_or(uri_path);
    if rel.is_empty() {
        return Some(INDEX);
    }
    // ⚠️ 反斜杠**不单独判**：内嵌那条是精确匹配表里的键、磁盘那条会 join 进 dist，
    // 两条都取不到 dist 之外的东西；而表的键另有「不带反斜杠」的判据（见 tests，用字节 92 写，
    // 免得在这个多层转义的工具链里被吃掉）。
    // 空分量（`a//b`）与 `.` / `..` 一律拒绝：合法请求里不会出现，放行只会给自己留缝
    if rel.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
        return None;
    }
    if Path::new(rel).components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(rel)
}

fn mime_for(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("html") => "text/html; charset=utf-8".to_string(),
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8".to_string(),
        Some("css") => "text/css; charset=utf-8".to_string(),
        Some("json") => "application/json; charset=utf-8".to_string(),
        Some("svg") => "image/svg+xml".to_string(),
        Some("png") => "image/png".to_string(),
        Some("jpg") | Some("jpeg") => "image/jpeg".to_string(),
        Some("webp") => "image/webp".to_string(),
        Some("gif") => "image/gif".to_string(),
        Some("ico") => "image/x-icon".to_string(),
        Some("woff") => "font/woff".to_string(),
        Some("woff2") => "font/woff2".to_string(),
        Some("ttf") => "font/ttf".to_string(),
        Some("map") => "application/json".to_string(),
        _ => mime_guess::from_path(path).first_or_octet_stream().to_string(),
    }
}

/// 组装静态资源响应。
///
/// 入口 html 必须 `no-cache`：发版后浏览器启发式缓存会让用户停在旧界面
/// （入口引用的是内容哈希资源名，资源本身可以长缓存）。
fn asset_response(key: &str, bytes: Vec<u8>) -> Response<Body> {
    let mut b = Response::builder().header(header::CONTENT_TYPE, mime_for(Path::new(key)));
    if key == INDEX {
        b = b.header(header::CACHE_CONTROL, "no-cache");
    }
    b.body(Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// 缺少可用资源时的可读指引（503）。
///
/// **只在显式覆盖指错时才会看到** —— release 一定内嵌，所以正常发行包里这一页永远不出现。
fn build_hint_page() -> Response<Body> {
    let locations = candidates()
        .iter()
        .map(|p| format!("<li><code>{}</code></li>", p.display()))
        .collect::<String>();
    let body = format!(
        r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>YekiTrim WebUI 未就绪</title>
<style>
 body{{font-family:system-ui,"Microsoft YaHei",sans-serif;max-width:760px;margin:64px auto;padding:0 20px;line-height:1.8;color:#1f2937}}
 code,pre{{background:#f3f4f6;border-radius:6px}}
 code{{padding:2px 6px}}
 pre{{padding:12px 14px;overflow:auto}}
 h1{{font-size:20px}}
</style></head><body>
<h1>WebUI 静态资源未就绪</h1>
<p>当前来源：<code>{label}</code>，里面没有 <code>index.html</code>。</p>
<p>release 二进制**内嵌**了一份 WebUI，正常不会走到这里 —— 看到这一页，说明
<b>显式指定了一个空的目录</b>（<code>--webui-dir</code> 或 <code>{env}</code>），
或者这是一份很旧的、还没内嵌的 exe。</p>
<p>磁盘候选位置（按优先级）：</p>
<ol>{locations}</ol>
<p>请在仓库根目录执行：</p>
<pre><code>cd webui
npm install
npm run build</code></pre>
<p>开发时更推荐 <code>npm run dev</code>（vite dev server + HMR），那条路前端不经服务端。</p>
</body></html>"#,
        label = source_label(),
        env = WEBUI_DIR_ENV,
    );
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::SERVICE_UNAVAILABLE.into_response())
}

/// 内嵌来源：按表精确匹配；未命中回落 `index.html`（SPA 回退，与磁盘那条一致）。
fn embedded_response(rel: &str) -> Response<Body> {
    if let Some((_, bytes)) = WEBUI_DIST.iter().find(|(p, _)| *p == rel) {
        return asset_response(rel, bytes.to_vec());
    }
    let Some((_, index)) = WEBUI_DIST.iter().find(|(p, _)| *p == INDEX) else {
        // build.rs 保证表里一定有 index.html；真没有说明生成物被人手改了
        return build_hint_page();
    };
    asset_response(INDEX, index.to_vec())
}

/// 磁盘来源：原有行为原样保留（防穿越 + SPA 回退）。
fn disk_response(dist: &Path, rel: &str) -> Response<Body> {
    let file = dist.join(rel);
    if file.is_file() {
        if let Ok(bytes) = std::fs::read(&file) {
            return asset_response(rel, bytes);
        }
    }
    match std::fs::read(dist.join(INDEX)) {
        Ok(bytes) => asset_response(INDEX, bytes),
        Err(_) => build_hint_page(),
    }
}

/// SPA 静态资源 + index 回退。
pub async fn webui_fallback(req: Request<Body>) -> Response<Body> {
    let Some(rel) = safe_rel(req.uri().path()) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    match source() {
        WebuiSource::Embedded => embedded_response(rel),
        WebuiSource::Disk(dist) => disk_response(&dist, rel),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// exe 附近那两条候选**必须排在构建期常量前面**（磁盘那条路的次序判据）。
    ///
    /// 这是 2026-10-02 修掉的那条：构建期常量（`CARGO_MANIFEST_DIR` → 仓库根）原是**第一条**，
    /// 而它是编译期烧进去的绝对路径 ⇒ 「exe 同级」那条**永远轮不到**。症状很隐蔽 ——
    /// 在开发机上把 exe 拷走、或从别的 cwd 起服务，它照旧端出**这台机器源码树里**的 dist，
    /// 一切看起来都正常。判据落在**次序**上，不是"能不能找到"（后者在本机永远为真）。
    #[test]
    fn exe_adjacent_candidates_come_before_the_build_time_constant() {
        let cands = candidates();
        let build_time = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("webui")
            .join("dist");
        let bt = cands
            .iter()
            .position(|c| *c == build_time)
            .expect("构建期那条候选必须还在（它是 `cargo run` 形态的保险）");

        // 至少有一条由 current_exe() 派生、且排在构建期常量之前。
        let exe = std::env::current_exe().expect("测试进程有自己的 exe");
        let exe_dir = exe.parent().expect("exe 必有父目录");
        let anchored: Vec<usize> = cands
            .iter()
            .enumerate()
            .filter(|(_, c)| c.starts_with(exe_dir))
            .map(|(i, _)| i)
            .collect();
        assert!(
            !anchored.is_empty(),
            "必须有一条候选锚在 exe 附近（否则换个 cwd/搬走 exe 就找不到 dist）：{cands:?}"
        );
        assert!(
            anchored.iter().all(|i| *i < bt),
            "exe 附近的候选（{anchored:?}）必须排在构建期常量（#{bt}）之前：{cands:?}"
        );
    }

    /// 兜底那条（cwd 相对路径）必须在最后：它是唯一会被**运行时 cwd** 影响的候选，
    /// 排前面会让"在哪个目录启动"决定端出哪份界面。
    #[test]
    fn cwd_relative_candidate_is_last() {
        let cands = candidates();
        assert_eq!(
            cands.last(),
            Some(&PathBuf::from("webui").join("dist")),
            "cwd 相对候选必须垫底：{cands:?}"
        );
    }

    /// **来源选择**的四种组合 —— 这是"测的就是发的"那条规则。
    ///
    /// ⚠️ 判据不能依赖本机的 `cfg!(debug_assertions)` 与磁盘上有没有 dist（两种开发机会得出
    /// 不同结论），所以用纯函数 `choose()` 把组合全钉住。
    #[test]
    fn source_selection_prefers_embedded_in_release_and_disk_in_debug() {
        let disk = PathBuf::from("/tmp/disk-dist");
        // release：内嵌优先，**即便磁盘上有 dist 也不用它** —— 否则在开发机上打 release 会
        // 内嵌一份、运行的却是仓库里那份，"测的"与"发的"就分家了。
        assert_eq!(choose(None, false, Some(disk.clone())), WebuiSource::Embedded);
        // debug：磁盘优先（开发常态：build 后刷新即生效，不用重链接服务端）
        assert_eq!(choose(None, true, Some(disk.clone())), WebuiSource::Disk(disk.clone()));
        // debug 但磁盘上没有：回落内嵌（内嵌永远可用 ⇒ 不会出现"debug 就必然 503"）
        assert_eq!(choose(None, true, None), WebuiSource::Embedded);
        // release 且磁盘没有：内嵌
        assert_eq!(choose(None, false, None), WebuiSource::Embedded);
        // 显式覆盖：**两种形态都听它的**，指错也照它报错（别偷偷换一份）
        let over = PathBuf::from("/tmp/override");
        assert_eq!(
            choose(Some(over.clone()), false, Some(disk.clone())),
            WebuiSource::Disk(over.clone())
        );
        assert_eq!(choose(Some(over), true, None), WebuiSource::Disk(PathBuf::from("/tmp/override")));
    }

    /// 内嵌表本身的形状：必须有入口，且每个键都是**安全的相对路径**。
    ///
    /// 表是 build.rs 遍历文件系统生成的，所以这条同时在看生成器有没有跑歪。
    #[test]
    fn embedded_table_has_index_and_only_safe_relative_keys() {
        assert!(WEBUI_DIST.len() > 5, "内嵌表太小，像是生成歪了：{} 项", WEBUI_DIST.len());
        assert!(WEBUI_DIST.iter().any(|(p, _)| *p == INDEX), "内嵌表里必须有 {INDEX}");
        for (key, bytes) in WEBUI_DIST {
            assert!(!bytes.is_empty(), "{key} 是空文件？更像是生成器读错了");
            assert!(!key.starts_with('/'), "键不能是绝对路径：{key}");
            assert!(!key.contains('\\'), "键必须用 `/` 分隔（与 HTTP 路径同形）：{key}");
            assert!(
                !key.split('/').any(|s| s.is_empty() || s == "." || s == ".."),
                "键里有空分量或 ..：{key}"
            );
            assert_eq!(safe_rel(key), Some(*key), "{key} 没能通过 safe_rel");
        }
    }

    /// 防穿越：两条来源共用同一套判断，行为不许分叉。
    #[test]
    fn traversal_is_rejected_for_both_sources() {
        for bad in ["/../secret", "/a/../../secret", "/a/./b", "/a//b", "//etc/passwd"] {
            assert_eq!(safe_rel(bad), None, "{bad} 竟然被放行了");
        }
        for ok in ["/", "/index.html", "/assets/index-abc123.js", "/录播 2026.mp4"] {
            assert!(safe_rel(ok).is_some(), "{ok} 不该被挡");
        }
        // ⚠️ 反斜杠路径**刻意不算违规**（见 safe_rel 的注释）：它取不到 dist 之外的东西 ——
        // 内嵌那条是精确匹配表里的键、磁盘那条 join 之后仍在 dist 里，取不到就回落 index。
        // 判据与实现必须一致，所以这里钉的是"它会被放行、但无害"，而不是"它必须 403"。
        assert!(safe_rel("/a\\b").is_some(), "反斜杠路径应落进 SPA 回退，不该 403");
        // 内嵌那条是精确匹配：穿越串在表里根本不存在
        assert!(WEBUI_DIST.iter().all(|(p, _)| *p != "../secret"));
    }

    /// release 形态必须**真的**有内嵌资源可用：否则"只换 exe 就能升级"是空话。
    /// 仓库里 dist 已入库 ⇒ 编译期必然生成表，所以这条在源码树上恒成立。
    #[test]
    fn embedded_source_is_always_ready() {
        assert!(
            WEBUI_DIST.iter().any(|(p, _)| *p == INDEX),
            "内嵌表里没有 {INDEX} —— release 的 `dist_ready()` 会返回 false"
        );
    }
}
