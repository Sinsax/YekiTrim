//! `api::polish::dhampir`：底座（Dhampir）出片通道的就绪位、工程转译缓存与 wasm 资源转发。

use std::path::Path;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;

use crate::model::AppState;

use super::*;



// ===================== Dhampir 浏览器端预览（wasm + 工程 JSON） =====================
//
// 「预览交给浏览器」：渲染跑在页面上（Dhampir 的 wasm + WebGPU），**后端只做两件事** ——
//
//   ① 把 wasm 包与宿主引擎 `engine.js` 发给浏览器（[`polish_dhampir_wasm`]）；
//   ② 把 `polish.toml` 转译成 Dhampir 的工程 JSON（页面 `loadEngine` 要吃它，
//      [`polish_dhampir_project`]）。
//
// **后端不渲染任何预览帧。** 实测口径：服务端那条路 PNG 出口 **1250 ms/帧** ✗；
// 浏览器腿 1080p 顺序播放 **7.0 ms/帧（142.9 fps）** ✓。像素不过边界，就没有"传输"这个瓶颈。

/// 把阻塞的转译丢到 blocking 线程池，并统一成接口错误。
///
/// 转译要起一个 `node` 子进程（百毫秒级），直接在 async 上下文里等会占住运行时线程。
async fn run_blocking<T, F>(what: &str, job: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(job).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{what}失败: {}", yekitrim_polish::render::dhampir::explain(&e)),
        )),
        Err(e) => Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{what}的任务崩了: {e}"),
        )),
    }
}


/// `GET /polish/dhampir-wasm/*path` —— 静态转发 Dhampir 的预览资源。
///
/// 两份根：`pkg/` 前缀走 wasm 包，其余走 `<Dhampir>/web`（`engine.js` 在那）。
/// **不复制**：Dhampir 那边重新构建 wasm 之后，刷新页面就是新的（靠 ETag 的
/// mtime 校验自然失效，不是靠禁缓存）。
pub(crate) async fn polish_dhampir_wasm(
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    // ⚠️ **这一整段必须走 blocking 池** —— 它是本路由曾经的真实故障 ✗：
    //
    // 这段以前直接在 async worker 上做同步 FS（`wasm_dirs()` 里的一串 `is_file()`、
    // 两次 `canonicalize()`、一次 `is_file()`）。工程/仓库目录在慢盘或网络盘上时
    // （本项目头顶那条纪律记的就是这个：一次 `exists()` 就要几百 ms 到几秒 ✗），
    // worker 被占住 ⇒ **本路由自己的响应产不出来** ⇒ 浏览器报
    // `error loading dynamically imported module`（**没有 net:: 码** ✓ 正是"连上了、拿不到"）
    // ⇒ 画布黑 + 同一时刻 `/api/jobs` 10s 超时 ✓✓（用户实测）。
    //
    // 注意日志形态：请求被受理但响应迟迟完不成 ⇒ **服务端日志里根本不出现这一条** ✓
    // （`→ 200 OK (1ms)` 是响应写完之后才打的）。所以"日志里没有它"不是"没发请求" ✗。
    let dirs = tokio::task::spawn_blocking(yekitrim_polish::render::dhampir::wasm_dirs).await;
    let Ok((web, pkg)) = dirs.unwrap_or_else(|e| Err(anyhow::anyhow!("定位任务崩了: {e}"))) else {
        return crate::media::json_error(
            StatusCode::NOT_FOUND,
            "找不到 Dhampir 的 wasm 预览包（设 VTEDIT_DHAMPIR_WASM=<Dhampir 仓根>，或先在那边跑 wasm-pack）",
        );
    };
    let (root, rel) = match path.strip_prefix("pkg/") {
        Some(rest) => (pkg, rest.to_string()),
        None => (web, path.clone()),
    };
    // 越界防护：`..` 与绝对路径都挡掉 —— 与 `polish_sticker_file` 同一条纪律。
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.starts_with('\\')
        || rel.split(['/', '\\']).any(|seg| seg == "..")
    {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "路径不合法");
    }
    let file = root.join(rel.replace('\\', "/"));
    // 同上：`canonicalize` / `is_file` 都是同步 FS，一并挪出去。
    let probe_root = root.clone();
    let probed = tokio::task::spawn_blocking(move || {
        let real = file.canonicalize().unwrap_or_else(|_| file.clone());
        let root_real = probe_root
            .canonicalize()
            .unwrap_or_else(|_| probe_root.clone());
        let ok = real.starts_with(&root_real) && real.is_file();
        (real, ok)
    })
    .await;
    let Ok((real, ok)) = probed else {
        return crate::media::json_error(StatusCode::NOT_FOUND, "资源不存在");
    };
    if !ok {
        return crate::media::json_error(StatusCode::NOT_FOUND, "资源不存在");
    }
    // `.wasm` 必须给对 MIME：给错浏览器会拒绝流式实例化（症状是"加载没反应"）。
    let mime = if real.extension().and_then(|e| e.to_str()) == Some("wasm") {
        "application/wasm".to_string()
    } else {
        mime_guess::from_path(&real).first_or_octet_stream().to_string()
    };
    let range = headers
        .get(axum::http::header::RANGE)
        .and_then(|v| v.to_str().ok());
    // **压缩 + ETag/304**（2026-10-02，Dhampir 首屏提速）。
    //
    // 这里的 `pkg/dhampir_wasm_bg.wasm` 是 **7.66 MB**，而它以前是原样发出的
    // ⇒ 每次进精修页都搬 7.66 MB。实测同一份字节 gzip -6 → 1.43 MB（19%）。
    // ETag 则让「文件没变」的那次刷新直接 304（几乎零字节）。
    //
    // ⚠️ 压缩与 ETag 都**只在这条路由上开**，没动 `serve_file` 的既有调用点：
    // `/api/media/*`（视频）走的是老签名，保持「Range + 不压缩」不变 ——
    // 对 Range 响应做压缩会破坏播放（见 `media.rs::serve_file_cond` 的说明）。
    //
    // ⚠️ **ETag 不能改成「永不变」**：这条路由的既有意图是「Dhampir 重建 wasm 后
    // 刷新即新」。`file_etag` 取的是 len+mtime ⇒ 重建必然改 mtime ⇒ 自然失效。
    let etag = tokio::fs::metadata(&real)
        .await
        .ok()
        .map(|m| crate::media::file_etag(&m));
    let if_none_match = headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    let accept_encoding = headers
        .get(axum::http::header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok());
    crate::media::serve_file_cond(&real, &mime, range, etag.as_deref(), if_none_match, accept_encoding)
        .await
}


/// `GET /polish/dhampir-status` —— **底座出片通道的就绪位**（M3）。
///
/// 回包（`dhampir::Readiness` 直接序列化）：`{ ready, binary, preview, translator, font,
/// contract_version, problems[] }`。
///
/// 为什么要它：默认出片通道就是底座，而"底座不可用"以前**只在渲染时报错**（几分钟的任务跑到一半才失败）。
/// 这条让前端在**点渲染之前**就能看见 —— 前端那颗通道下拉旁边会挂提示，`yekitrim doctor` 用同一份判断。
///
/// ⚠️ 只做**定位**（六项各查一次文件），不渲染、不跑转译器 —— 它会在打开渲染弹窗时被调，
/// 也可能被反复调。⚠️ 路径**只回给已登录用户**（这条路由在 `require_user` 层内），
/// 与 `/api/health` 的口径不同：health 是公开接口、只回布尔位（见 `api/mod.rs::health`）。
pub(crate) async fn polish_dhampir_status(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    // 带了 `dir` 就按那个工程选的字体判（`style.subtitle.font`）——字体是"按工程"解析的，
    // 不带就按缺省（系统字体栈）判。读工程要落盘 IO，所以整段走 `blocking`。
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let readiness = blocking(move || {
        let font = if dir.trim().is_empty() {
            String::new()
        } else {
            yekitrim_polish::project::load_project(std::path::Path::new(&dir))
                .map(|cfg| cfg.style.subtitle.font)
                .unwrap_or_default()
        };
        // `blocking` 要 `Result`：读工程失败**不该**让就绪位请求 500 ——
        // 字体会退成"系统字体栈"那条判断，问题照样在 `problems` 里说得清。
        // `deep = true`：这条是**已登录**接口、打开渲染弹窗时调一次 —— 顺带跑 `node --version`，
        // 把"PATH 上有 node 但它跑不通（商店桩）/ 版本太低"这两类也说清楚。
        // `/api/health` 那条（无鉴权 + 被轮询）只走便宜判断。
        Ok(yekitrim_polish::render::dhampir::readiness(&font, true))
    })
    .await?;
    ok(serde_json::json!(readiness))
}


// ── Dhampir 转译结果缓存（2026-11-21）───────────────────────────────────────────
//
// `POST /polish/dhampir-project` **每次**都 spawn 一次 Node 转译器（+ 若干 ffprobe），实测几百 ms ~ 2 s；
// 而精修页在「进片段」与「每次改动落盘后」都会调它。用户在局域网里感知到的「点一下要等一下」，
// 一半来自这里（另一半才是带宽）。转译结果对**同一份输入**是确定的，所以可以缓存。
//
// # 键 = 六项，全部满足「输入变了 ⇒ 键必变」
// `polish.toml` / `_proxy_720.mp4`（有就参与：它决定转译器探测到的时基与分辨率）/ `clip.srt` /
// `clip.dm.srt` / **转译器脚本自身**（开发时改 `tools/polish-to-dhampir.mjs` 必须失效）/
// **字体库映射指纹**（`dhampir::fonts_stamp` —— 换字体必须失效，否则就是历史上那个「换字体没反应」）。
//
// # ⚠️ 只存内存、只存**回包 JSON**
// 转译产物写在工程 `.yekitrim/` 的 scratch 名上，`cleanup_project` 立刻删掉它 ——
// 缓存「产物路径」会拿到已删除的文件；而回包里带的是**内容**（project + subtitles + 报告），所以能缓存。
// **绝不落盘**：工程目录里多一个缓存文件就多一条用户看不懂的残留（R4 的同一条纪律）。
//
// # 容量 2、最近使用优先
// 一次预览只碰一个片段，切来切去时 2 条足够；内存钉在「两份工程 JSON」这个量级。

#[derive(Clone, PartialEq, Eq, Debug)]
struct TranslationKey {
    /// 片段目录（绝对、已校验）——多用户/多片段之间必须互不串味。
    dir: String,
    polish_toml: (u64, u64),
    proxy: (u64, u64),
    srt: (u64, u64),
    dm: (u64, u64),
    translator: (u64, u64),
    fonts: String,
}


/// `(mtime_nanos, len)`；文件不在就 `(0, 0)`（「没写过」本身就是一个有效状态）。
fn file_stamp(p: &Path) -> (u64, u64) {
    use std::time::UNIX_EPOCH;
    match std::fs::metadata(p) {
        Ok(m) => {
            let t = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            (t, m.len())
        }
        Err(_) => (0, 0),
    }
}


/// 算一次键（5 次 `metadata` + 一次字体库扫描）—— **必须放 blocking 池**，工程常在网络盘上。
fn translation_key(dir: &Path) -> TranslationKey {
    let translator = yekitrim_polish::render::dhampir::find_translator().ok();
    TranslationKey {
        dir: yekitrim_core::path::display(dir),
        polish_toml: file_stamp(&dir.join("polish.toml")),
        proxy: file_stamp(&dir.join(yekitrim_polish::render::proxy::PREVIEW_PROXY_FILE)),
        srt: file_stamp(&dir.join("clip.srt")),
        dm: file_stamp(&dir.join("clip.dm.srt")),
        translator: translator.as_deref().map(file_stamp).unwrap_or((0, 0)),
        fonts: yekitrim_polish::render::dhampir::fonts_stamp(),
    }
}


/// 最近使用的在前（`Vec` 当 LRU，容量 2）。
static TRANSLATION_CACHE: std::sync::OnceLock<
    std::sync::Mutex<Vec<(TranslationKey, Arc<serde_json::Value>)>>,
> = std::sync::OnceLock::new();


fn translation_cache() -> &'static std::sync::Mutex<Vec<(TranslationKey, Arc<serde_json::Value>)>> {
    TRANSLATION_CACHE.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}


fn translation_cache_get(key: &TranslationKey) -> Option<serde_json::Value> {
    let mut guard = translation_cache().lock().unwrap_or_else(|e| e.into_inner());
    let idx = guard.iter().position(|(k, _)| k == key)?;
    let hit = guard.remove(idx);
    let value = (*hit.1).clone();
    guard.insert(0, hit);
    Some(value)
}


fn translation_cache_put(key: TranslationKey, value: &serde_json::Value) {
    let mut guard = translation_cache().lock().unwrap_or_else(|e| e.into_inner());
    guard.retain(|(k, _)| k != &key);
    guard.insert(0, (key, Arc::new(value.clone())));
    guard.truncate(2);
}


/// `POST /polish/dhampir-project` —— 转译 `polish.toml`，把工程 JSON **内容**交给浏览器。
///
/// 请求：`{ "dir": "<片段目录（绝对路径）>" }`
/// 回包：`{ "ok": true, "project": { …Dhampir 工程… } }`
///
/// 页面那边是 `JSON.stringify(project)` 之后交给 `dhampir_project_open`。
pub(crate) async fn polish_dhampir_project(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("");
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").is_file() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    // ① 先算缓存键（5 次 metadata + 一次字体库扫描 —— 走 blocking 池，工程常在网络盘上）。
    //    命中就直接回上次那份回包：**省掉一整个 Node 转译**（几百 ms ~ 2 s）。
    let key = {
        let p = path.clone();
        blocking(move || Ok::<_, AppError>(translation_key(&p))).await?
    };
    if let Some(hit) = translation_cache_get(&key) {
        tracing::debug!("  · Dhampir 转译命中缓存（{}）", key.dir);
        return ok(hit);
    }
    let scratch = path.join(yekitrim_core::path::PROJECT_STATE_DIR_NAME);
    std::fs::create_dir_all(&scratch)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("建缓存目录失败: {e}")))?;
    let clip = path.clone();
    // **顺带把字幕/弹幕的文本带回去。**
    //
    // 转译器把这两个写成**旁写文件**，而工程里那两条资产的 `uri` 是**绝对路径**
    // （指向转译输出目录）—— 浏览器拿不到它：`/api/media` 只服务项目内相对路径，
    // 而且 `cleanup_project` 之后那个文件就没了。
    //
    // 症状是最难查的那一类：**预览看起来正常，只是没有字幕和弹幕**，还不报错。
    // 所以这里在 cleanup **之前**把内容读出来，让前端用 blob URL 喂给
    // `engine.loadSubtitles([{ assetId, url, format }])`（解析在 wasm 里，走 parse_srt/parse_ass）。
    let (json, subs, translate_report) = run_blocking(
        "Dhampir 转译",
        move || -> anyhow::Result<(String, Vec<(String, String)>, yekitrim_polish::render::dhampir::TranslateReport)> {
            // ⚠️ **预览这条腿必须 `keep_muted = false`**（压缩时间线）：客户端靠"落在被掐掉的段里
        // = 没有对应帧"去跳屏蔽区间，整条时间线会让它一路播过那些段。
        // 出片那条腿相反（`render_video_only` 传 true，屏蔽区间交给尾部那一剪）。
        // **有预览代理就用它**（`polish init` 生成，见 `render/proxy.rs`）：转译器会按代理
        // 探测时基/分辨率写进工程，所以画面缩放/取景也是按它算的。
        // 没有/过期 ⇒ `None`，照旧读源。
        let preview_video = yekitrim_polish::render::proxy::preview_video_for(&clip);
        // ⚠️ 用 `_reporting` 那条腿（不是 `prepare_project`）：总方案 §4「转译报告可见」
        // 要求 DROP/CLAMP **看得见**。预览是用户最常碰的那条路，报告在这里被丢掉
        // 等于"这个效果没生效"永远没人知道 —— 与出片那条腿同口径。
        let (translated, translate_report) =
            yekitrim_polish::render::dhampir::prepare_project_reporting(
                &clip,
                &scratch,
                false,
                preview_video.as_deref(),
            )?;
            let text = std::fs::read_to_string(&translated)
                .map_err(|e| anyhow::anyhow!("读不出转译产物 {}：{e}", translated.display()))?;
            let subs = read_side_subtitles(&text, &translated);
            // 草稿用完就清（与出片那条同一个纪律：只删自己造的）。
            // ⚠️ 传**本次产物路径**（`translated`），不是目录 —— 草稿名带独占后缀
            // （见 `dhampir::scratch_tag`），按目录重算会一个都删不掉；
            // 而且并发预览时，只有传自己的路径才不会删掉**另一个请求**正在用的文件
            // （用户报的 `ENOENT ... _temp_font_families.json` 就是那么来的）。
            yekitrim_polish::render::dhampir::cleanup_project(&translated);
            // 报告也带出去（§4）：DROP>0 必须让用户看见，不能只在服务端打一行日志。
            for (level, msg) in yekitrim_polish::render::dhampir::report_lines(&translate_report) {
                match level {
                    "warn" => tracing::warn!("{msg}"),
                    "debug" => tracing::debug!("{msg}"),
                    _ => tracing::info!("{msg}"),
                }
            }
            Ok((text, subs, translate_report))
        },
    )
    .await?;
    let project: serde_json::Value = serde_json::from_str(&json)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("转译产物不是合法 JSON: {e}")))?;
    let subtitles: serde_json::Map<String, serde_json::Value> = subs
        .into_iter()
        .map(|(name, text)| (name, serde_json::Value::String(text)))
        .collect();
    // 转译报告进回包（总方案 §4）：`translate_report` 是**接口产物**，
    // 前端据此提示"这个效果其实没生效"。零 DROP 也回（那是有意义的好消息，且可核对）。
    let value = serde_json::json!({
        "ok": true,
        "project": project,
        "subtitles": subtitles,
        "translate_report": translate_report,
    });
    // ② 只把**成功**的那份放进缓存（失败下次还要重试；转译报错会走上面的 `?` 提前返回）。
    translation_cache_put(key, &value);
    ok(value)
}


/// 读回**全部旁写字幕**（内容, 文件名）—— 预览那条腿靠它把文本喂给 wasm。
///
/// # 为什么不能按文件名猜（2026-10-13 修的真缺陷）
///
/// 这里原先是写死的两个名字：
///
/// ```ignore
/// for name in ["dhampir-subtitle.srt", "dhampir-danmaku.ass"] {
///     let side = translated.with_file_name(name);   // ← 只在 --out 旁边找
/// ```
///
/// 而**文字元素那条轨的 ASS 不在那个目录**：转译器把它写在 `--out` **同目录的
/// `<工程文件名去扩展名>.assets/` 子目录**里（2026-10-18 起；此前直接写片段根目录 ——
/// 文件名带落点/动效/颜色 ⇒ 每改一次就多一个，用户的片段目录越积越多，见
/// `tools/polish-to-dhampir.mjs` 里 `sideDir` 那段）。字幕/弹幕仍走 `dirname(outPath)`。
/// 于是：
///
/// * **出片腿（`dhampir.exe`）是对的** —— 它按 `asset.uri`（**绝对路径**）自己找文件；
/// * **wasm 预览腿整条文字不显示** —— 浏览器没有文件系统，文本只能由这里读出来喂。
///   名字没读过 ⇒ `dhampir_project_set_subtitles` 从没被调用 ⇒ 引擎手里
///   **有轨、零 cue** ⇒ 一个字都不画。
///
/// 症状是"预览里没有文字元素，而**报告、日志、`doc.json` 三者全都正常**" ——
/// 极难归因（成片里那些字好好的）。
///
/// # 为什么改成「读 doc 自己声明的资产」而不是"多列几个名字"
///
/// 文字轨的文件名里**带着档位、动效、颜色**（`dhampir-text_<样式>_<档>[_<动效>][_<颜色>].ass`），
/// 规则注定会继续长。再写一份名字清单就是第二份会漂的实现 ——
/// 而漂了的表现又是静默（少一条轨 = 那类文字在预览里不出现）。
/// **`assets[].uri` 是转译器自己写的绝对路径**，照它读永远与写入侧同源。
///
/// # 键用**文件名**、不是 `asset.id`
///
/// 前端 `PolishPreview.vue` 按 `uri` 的 basename 查表
/// （`String(a.uri).split(/[\\\\/]/).pop()`）。两处口径必须一致 ——
/// 用 `id` 会要求把前端也改一遍，换不来任何东西（两者在本仓产物里恰好同串）。
///
/// # 三态都要照顾到
///
/// * 读不出来 ⇒ `warn!` **点名文件**、不中断（与"缺一个不是错误"同口径：
///   这份片段可能就没有弹幕/没有文字元素）；
/// * doc 里**没有** `assets`（老产物 / 结构异常）⇒ 回落到那两个固定名字，
///   行为与改造前**逐字节相同**；
/// * 同一 basename 出现两次 ⇒ 后写覆盖（map 语义），本仓产物不会发生。
pub(super) fn read_side_subtitles(doc_json: &str, translated: &std::path::Path) -> Vec<(String, String)> {
    /// 老的固定两个名字 —— **只在 doc 读不出资产时**当兜底。
    const LEGACY_NAMES: [&str; 2] = ["dhampir-subtitle.srt", "dhampir-danmaku.ass"];

    let read_one = |path: &std::path::Path| -> Option<String> {
        match std::fs::read_to_string(path) {
            Ok(content) => Some(content),
            Err(e) => {
                tracing::warn!("读不出旁写字幕 {}：{e}", path.display());
                None
            }
        }
    };

    let declared: Vec<String> = serde_json::from_str::<serde_json::Value>(doc_json)
        .ok()
        .and_then(|doc| {
            doc.get("assets").and_then(|a| a.as_array()).map(|assets| {
                assets
                    .iter()
                    // ⚠️ 只认 `kind == "subtitle"`：字幕、弹幕、文字元素**三者都是这一档**
                    //    （文字元素不是新 kind，它复用的就是字幕轨）。
                    .filter(|a| a.get("kind").and_then(|k| k.as_str()) == Some("subtitle"))
                    .filter_map(|a| a.get("uri").and_then(|u| u.as_str()))
                    .map(str::to_string)
                    .collect()
            })
        })
        .unwrap_or_default();

    if declared.is_empty() {
        // doc 里没有资产清单（老产物 / 结构异常）：照老路走，**行为不变**。
        return LEGACY_NAMES
            .iter()
            .filter_map(|name| {
                let side = translated.with_file_name(name);
                read_one(&side).map(|content| ((*name).to_string(), content))
            })
            .collect();
    }

    let mut subs: Vec<(String, String)> = Vec::new();
    for uri in declared {
        // 键 = `uri` 的 basename（与前端同口径，见上面那段注释）。
        // ⚠️ Windows 的 `uri` 是 `F:\a\b.ass`、POSIX 是 `/a/b.ass`
        //    —— 两种分隔符都要切（`split(['/', '\\'])`）。
        let Some(name) = uri
            .rsplit(['/', '\\'])
            .next()
            .filter(|n| !n.is_empty())
            .map(str::to_string)
        else {
            tracing::warn!("旁写字幕的 uri 取不出文件名（{uri}）—— 这条喂不进预览");
            continue;
        };
        if let Some(content) = read_one(std::path::Path::new(&uri)) {
            subs.push((name, content));
        }
    }
    subs
}


/// `POST /polish/dhampir-project` **转译缓存**的判据（2026-11-21）。
///
/// 单独一个模块：上面的 `mod tests` 是路由级用例（要 `TestApp` + 真跑 node），
/// 而这个缓存是**纯进程内状态**，不需要起服务、也不需要 node。
#[cfg(test)]
mod translation_cache_tests {
    use super::*;

    fn key_for(dir: &str, toml_len: u64) -> TranslationKey {
        TranslationKey {
            dir: dir.to_string(),
            polish_toml: (1, toml_len),
            proxy: (0, 0),
            srt: (0, 0),
            dm: (0, 0),
            translator: (0, 0),
            fonts: "abc".into(),
        }
    }

    /// 命中 / 容量 2 / 最近使用优先 —— LRU 很朴素，但它错了就是「改了配置还拿旧工程」。
    #[test]
    fn cache_remembers_two_and_evicts_the_least_recent() {
        // 用一对不可能出现在真实路径里的 dir 造键，免得和别的用例/真实请求串味
        let (a, b, c) = (
            key_for("<cache-test-a>", 1),
            key_for("<cache-test-b>", 1),
            key_for("<cache-test-c>", 1),
        );
        assert!(translation_cache_get(&a).is_none(), "一开始是空的");

        translation_cache_put(a.clone(), &serde_json::json!({ "v": "A" }));
        assert_eq!(translation_cache_get(&a).unwrap()["v"], "A");

        translation_cache_put(b.clone(), &serde_json::json!({ "v": "B" }));
        // 摸一下 A ⇒ 它变成「最近使用」
        assert_eq!(translation_cache_get(&a).unwrap()["v"], "A");

        translation_cache_put(c.clone(), &serde_json::json!({ "v": "C" }));
        assert_eq!(translation_cache_get(&a).unwrap()["v"], "A", "最近用过的不能被挤掉");
        assert!(
            translation_cache_get(&b).is_none(),
            "最久没用的那条要被挤掉（容量 2）"
        );
        assert_eq!(translation_cache_get(&c).unwrap()["v"], "C");
    }

    /// 键必须**跟着输入变**：`polish.toml` 改一个字节就得换一个键；同内容两次必须稳定。
    #[test]
    fn key_tracks_the_inputs_and_is_stable() {
        let dir = tempfile::tempdir().expect("临时目录");
        std::fs::write(dir.path().join("polish.toml"), "a = 1\n").unwrap();
        let k1 = translation_key(dir.path());
        assert_eq!(k1, translation_key(dir.path()), "同一份输入两次算出的键必须相同");

        std::fs::write(dir.path().join("polish.toml"), "a = 2222\n").unwrap();
        let k2 = translation_key(dir.path());
        assert_ne!(k1.polish_toml, k2.polish_toml, "polish.toml 变了，键必须变");
        assert_ne!(k1, k2);

        // 代理文件出现也要变 —— 它决定转译器探测到的时基与分辨率
        std::fs::write(dir.path().join("_proxy_720.mp4"), b"x").unwrap();
        let k3 = translation_key(dir.path());
        assert_ne!(k2.proxy, k3.proxy, "代理出现 ⇒ 键必须变");
    }

    /// **端到端**：真打一次接口之后，缓存里必须有那一份 —— 也就是说「下一次同样的请求不会再 spawn node」。
    ///
    /// 判据用「接口跑完 ⇒ `translation_cache_get(key)` 命中」，而不是计时
    /// （计时在 CI/慢盘上必然抖）。第二次请求再比一次回包**逐值相同**，把"走的是缓存那条路"钉死。
    #[tokio::test]
    async fn project_endpoint_populates_the_cache_and_replays_it() {
        use crate::api::test_support::{body_json, test_app};

        let t = test_app().await;
        let clip = t.root.join("output/clips/tcache");
        std::fs::create_dir_all(&clip).unwrap();
        let cfg = yekitrim_polish::project::ProjectFile::example();
        std::fs::write(clip.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();
        let payload = serde_json::json!({ "dir": clip.to_string_lossy() }).to_string();

        let ask = |payload: String| {
            let t = &t;
            async move {
                let res = t
                    .call(
                        t.req("POST", "/polish/dhampir-project")
                            .header(axum::http::header::CONTENT_TYPE, "application/json")
                            .body(axum::body::Body::from(payload))
                            .unwrap(),
                    )
                    .await;
                let status = res.status();
                let v = body_json(res).await;
                (status, v)
            }
        };

        let (status, first) = ask(payload.clone()).await;
        if status != StatusCode::OK {
            panic!("转译应当成功，实际 {status}：{first}");
        }
        let key = translation_key(&clip);
        assert!(
            translation_cache_get(&key).is_some(),
            "接口成功后必须把回包放进缓存（否则每次改配置都白跑一遍 node）"
        );

        let (status2, second) = ask(payload).await;
        assert_eq!(status2, StatusCode::OK);
        assert_eq!(first, second, "同样的输入两次回包必须逐值相同（第二次走缓存）");
    }

    /// 字体库指纹：进程内**稳定**且是 16 位十六进制（它参与缓存键，错了就是「换字体没反应」）。
    #[test]
    fn fonts_stamp_is_stable_hex() {
        let a = yekitrim_polish::render::dhampir::fonts_stamp();
        let b = yekitrim_polish::render::dhampir::fonts_stamp();
        assert_eq!(a, b, "同一份字体库两次指纹必须相同");
        assert_eq!(a.len(), 16, "指纹是 16 位十六进制：{a}");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()), "只许十六进制：{a}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;
    use axum::body::Body;

    // ⚠️ 这些用例是**按原字节搬来**的（含原本在文件顶层、缩进为 0 的那批）：
    //    统一重排缩进会动到多行字符串字面量，判据会因此悄悄变。


    /// 底座就绪位端点（M3）：**必须 200**，且回包里有 `ready` 布尔与 `problems` 数组。
    ///
    /// 这条守的是"渲染前可见"那条链路的**接口面**：前端渲染弹窗一打开就调它，
    /// 一次 500 或一个缺字段的回包 = 提示永远不出现（而那正是它存在的理由）。
    /// 它**不断言 ready 的真假**：那取决于跑测试这台机器装没装底座（两种都是合法结果）。
    #[tokio::test]
    async fn 底座就绪位端点回布尔与问题清单() {
        let t = test_app().await;
        let clip = t.root.join("output/clips/ready");
        std::fs::create_dir_all(&clip).unwrap();
        let cfg = yekitrim_polish::project::ProjectFile::example();
        std::fs::write(clip.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

        let body = serde_json::json!({ "dir": clip.to_string_lossy() }).to_string();
        let res = t
            .call(
                t.req("POST", "/polish/dhampir-status")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        let status = res.status();
        let v = body_json(res).await;
        assert_eq!(status, StatusCode::OK, "就绪位应当 200：{v}");
        assert_eq!(v["ready"].is_boolean(), true, "要回 `ready` 布尔：{v}");
        assert_eq!(v["problems"].is_array(), true, "要回 `problems` 数组：{v}");
        // `node_version` 这一位是**深探**才有的（这条接口是深探）：null 或字符串都合法，
        // 但**键必须在** —— 前端把它放进提示的 `title`。
        assert!(
            v.get("node_version").is_some(),
            "深探要回 `node_version`（可为 null）：{v}"
        );
        // 不 ready 时必须至少有一条原因 —— 否则前端只能显示"不可用"而说不出为什么。
        if v["ready"] == serde_json::json!(false) {
            assert!(
                v["problems"].as_array().map(|a| !a.is_empty()).unwrap_or(false),
                "不可用时必须给出原因：{v}"
            );
        }
    }


    /// **工程 JSON 走真 HTTP**：路由接上了、转译真的跑了、回来的是能喂给 wasm 的工程。
    ///
    /// 不需要 Dhampir 二进制（转译器是 node 脚本），所以这条**进默认测试**。
    #[tokio::test]
    async fn dhampir_project_走真_http_出转译后的工程() {
        let t = test_app().await;
        let clip = t.root.join("output/clips/conv");
        std::fs::create_dir_all(&clip).unwrap();
        let cfg = yekitrim_polish::project::ProjectFile::example();
        std::fs::write(clip.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();
        let dir = clip.to_string_lossy().into_owned();

        let body = serde_json::json!({ "dir": dir }).to_string();
        let res = t
            .call(
                t.req("POST", "/polish/dhampir-project")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await;
        let status = res.status();
        if status != StatusCode::OK {
            let detail = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
            panic!("工程转译应当成功，实际 {status}；响应：{}", String::from_utf8_lossy(&detail));
        }
        let v = body_json(res).await;
        assert_eq!(v["ok"], serde_json::json!(true));
        // 形状要真的是 Dhampir 工程：浏览器那边是直接喂给 dhampir_project_open 的。
        assert!(
            v["project"]["timeline"]["tracks"].is_array(),
            "回包要带可用的工程（timeline.tracks 缺失）：{v}"
        );
        // 草稿要清掉：`_dhampir_project.json` 是这一趟的中间产物，不该留在片段目录里。
        assert!(
            !clip.join("_dhampir_project.json").exists(),
            "转译草稿没有清掉"
        );
        // **转译报告要带回来**（总方案 §4「转译报告可见」）。
        //
        // 判据是"这个键**存在**且形状对"，不是"DROP 恰好等于某个数" ——
        // 合成片段转译出几条 DROP 会随转译器演进而变，钉死数值等于给未来埋一颗假红。
        // 真正不变的是：**报告必须出现在回包里**（前端据此提示"这个效果没生效"）。
        assert!(
            v.get("translate_report").is_some(),
            "回包必须带 translate_report（§4：DROP/CLAMP 不许静默消失）：{v}"
        );
        let rep = &v["translate_report"];
        assert!(
            rep["drop_count"].is_number(),
            "translate_report.drop_count 要是数字：{rep}"
        );
        assert!(
            rep["clamp_count"].is_number(),
            "translate_report.clamp_count 要是数字：{rep}"
        );
        assert!(
            rep["drops"].is_array() && rep["clamps"].is_array(),
            "逐条明细要能被前端列出来：{rep}"
        );
        // ⚠️ 形状一致性：条数必须与明细数组长度**对得上** ——
        // 否则前端"3 条 DROP"点开只看得到 1 条（`parse` 只数不填的退化形态抓不到别的）。
        assert_eq!(
            rep["drop_count"].as_u64().unwrap() as usize,
            rep["drops"].as_array().unwrap().len(),
            "drop_count 与 drops 长度必须一致：{rep}"
        );
        assert_eq!(
            rep["clamp_count"].as_u64().unwrap() as usize,
            rep["clamps"].as_array().unwrap().len(),
            "clamp_count 与 clamps 长度必须一致：{rep}"
        );
        // **字幕/弹幕的文本要带回来。**
        //
        // 工程里那两条资产的 `uri` 是**绝对路径**（指向转译输出目录），浏览器取不到 ——
        // 所以内容只能由这里带回去。少了它，预览会"看起来正常、只是没有字幕"。
        let subs = v["subtitles"].as_object().expect("回包要带 subtitles");
        // ⚠️ **条件断言**：这条用例用的是合成片段（`ProjectFile::example()`，
        // 目录里没有 `clip.srt` / `clip.dm.srt`），所以转译器**不产**旁写文件、
        // `subtitles` 就是空的 —— 那是正常的。
        //
        // 第一版我拿真片段的情况去断言它，结果红了。正确的判据是**跟着工程走**：
        // 工程里有字幕类资产（`kind == "subtitle"`）时，内容就**必须**带回来 ——
        // 这才是"浏览器取不到旁写文件"那条不变量的可检形式。
        let wants_subtitles = v["project"]["assets"]
            .as_array()
            .map(|assets| {
                assets
                    .iter()
                    .any(|a| a["kind"].as_str() == Some("subtitle"))
            })
            .unwrap_or(false);
        if wants_subtitles {
            assert!(
                !subs.is_empty(),
                "工程里有字幕类资产，但字幕内容没带回来：预览会\"看起来正常、只是没有字幕\""
            );
            for (name, text) in subs {
                assert!(
                    text.as_str().map(|s| !s.is_empty()).unwrap_or(false),
                    "{name} 的内容是空的"
                );
            }
            // ⚠️⚠️ **逐条覆盖**（2026-10-13）：上面那条"非空"曾经**漏掉整类资产**。
            //
            // 症状（实测）：文字元素轨的 ASS 写在**片段根目录**，而这里原先按
            // 两个写死的名字去 `--out` 旁边找 ⇒ **一条文字轨都没读过** ⇒
            // 预览里一个文字元素都不显示，而报告/日志/`doc.json` 三者全正常。
            // 上面那条"`subs` 非空就算过"在**只要还有字幕或弹幕**时照样绿。
            //
            // ⇒ 判据必须**按资产逐条要**：doc 里每一个 `kind == "subtitle"` 的资产，
            //    都必须能在回包里按它的 **basename** 查到内容。
            //    这才是"浏览器取不到旁写文件、内容只能由服务端带回"那条不变量的
            //    可检形式 —— "非空"只挡住了"一条都没有"，挡不住"少了一类"。
            for a in v["project"]["assets"].as_array().unwrap() {
                if a["kind"].as_str() != Some("subtitle") {
                    continue;
                }
                let uri = a["uri"].as_str().unwrap_or("");
                let base = uri.rsplit(['/', '\\']).next().unwrap_or("");
                assert!(
                    subs.get(base).is_some(),
                    "资产 `{}`（uri = {uri}）的内容没带回来 —— 预览里这一类文字不会显示，\
                     而成片里是好的（底片腿按绝对 uri 自己找文件）",
                    a["id"].as_str().unwrap_or("<无 id>")
                );
            }
        }

        // **预览代理优先**（`polish init` 生成、只给预览那条腿用 —— 出片永远用源）。
        //
        // 断言的是**接线**：片段目录里存在新鲜的 `_proxy_720.mp4` 时，工程里的视频素材
        // 必须指向代理（转译器再按代理探测时基/分辨率，所以画面缩放也是按它算的）。
        // 这里用假文件就够 —— 探不到就回落 60fps/1920x1080，而 `uri` 一定是代理名。
        let video_uri = |v: &serde_json::Value| -> String {
            v["project"]["assets"]
                .as_array()
                .and_then(|a| a.iter().find(|x| x["kind"].as_str() == Some("video")))
                .and_then(|a| a["uri"].as_str())
                .unwrap_or("<没有视频素材>")
                .to_string()
        };
        assert_eq!(video_uri(&v), "clip.mp4", "没有代理时必须读源");
        std::fs::write(clip.join("_proxy_720.mp4"), b"not-a-real-video").unwrap();
        let res2 = t
            .call(
                t.req("POST", "/polish/dhampir-project")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res2.status(), StatusCode::OK, "有代理那次也该成功");
        let v2 = body_json(res2).await;
        assert_eq!(
            video_uri(&v2),
            "_proxy_720.mp4",
            "片段目录里有新鲜代理时，预览必须读代理（否则它白生成了）"
        );
    }


    /// **文字元素轨的文本必须经真 HTTP 带回来**（2026-10-13）。
    ///
    /// # 为什么单独一条
    ///
    /// 上面那条 `dhampir_project_走真_http_出转译后的工程` 用的是
    /// `ProjectFile::example()` —— **里面一个文字元素都没有**。于是
    /// "旁写字幕按两个写死的名字读"这个缺陷在它那里**无从暴露**
    /// （它只有字幕/弹幕，而那两个恰好就在 `--out` 旁边）。
    ///
    /// 实测症状：文字元素在**成片里好好的**、`doc.json` 里轨道与资产都在、
    /// 转译报告全绿，而 **wasm 预览里一个字都不显示** ——
    /// 因为文字轨的 ASS 写在**片段根目录**，服务端从没读过它，
    /// 于是 `dhampir_project_set_subtitles` 一次都没被调用（有轨、零 cue）。
    ///
    /// 本用例让工程**真的有**一条文字元素，从而把这条链整段压住。
    #[tokio::test]
    async fn dhampir_project_带文字元素的文本() {
        let t = test_app().await;
        let clip = t.root.join("output/clips/textprev");
        std::fs::create_dir_all(&clip).unwrap();
        let mut cfg = yekitrim_polish::project::ProjectFile::example();
        // ⚠️⚠️ **必须有 `duration`**：转译器拿它算 `totalFrames`
        //     （`frameAt(config.duration ?? 0)`），再据此推 `keepRanges`（时间线映射）。
        //     `ProjectFile::example()` 里它是 **`None`** ⇒ `totalFrames = 0`
        //     ⇒ 每个事件都被判成"落在被掐掉的区间里"而**整条跳过**
        //     （那个跳过本身是对的，但会让这条用例的前提不成立）。
        //     第一版就是这么红的：报告零 DROP、资产里一个文字轨都没有。
        cfg.duration = Some(10.0);
        std::fs::write(clip.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();
        // ⚠️ 源媒体也要在：转译器按它探测时基/分辨率（探不到会走 NOTE 兜底，
        //    但"文件不存在"是另一档）。这里给个假文件就够，判据不依赖真解码。
        std::fs::write(clip.join("clip.mp4"), b"not-a-real-video").unwrap();
        // 造一条文字元素（**必须经 PUT**：`clips[].events` 是整份反序列化的，
        // 直接改文件反而绕过了"编辑器存下来的形状"这条现实约束）。
        let body = serde_json::json!({
            "dir": clip.to_string_lossy(),
            "segments": [{
                "start": 0.0,
                "end": 5.0,
                "events": [{
                    "type": "element",
                    "kind": "text",
                    "time": 0.5,
                    "end": 3.0,
                    "text": "预览可见",
                    "style": "hook",
                    "pos": "center",
                    "color": "pink"
                }]
            }]
        })
        .to_string();
        let res = t
            .call(
                t.req("PUT", "/polish/project")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        // 不同版本的这个接口形状可能不同 —— 不认就**显式报错**，别写成"失败也通过"。
        let put_status = res.status();
        if put_status != StatusCode::OK {
            let detail = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
            panic!("保存文字元素应当成功，实际 {put_status}：{}", String::from_utf8_lossy(&detail));
        }

        let res = t
            .call(
                t.req("POST", "/polish/dhampir-project")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::json!({ "dir": clip.to_string_lossy() }).to_string()))
                    .unwrap(),
            )
            .await;
        let status = res.status();
        let v = body_json(res).await;
        assert_eq!(status, StatusCode::OK, "转译应当成功：{v}");

        // ① 产物里**确实有一条文字轨资产**（前提：转译器把文字元素转出来了）。
        let text_assets: Vec<&serde_json::Value> = v["project"]["assets"]
            .as_array()
            .expect("assets 是数组")
            .iter()
            .filter(|a| {
                a["kind"].as_str() == Some("subtitle")
                    && a["uri"].as_str().unwrap_or("").contains("dhampir-text_")
            })
            .collect();
        assert!(
            !text_assets.is_empty(),
            "工程里没有文字轨资产 —— 转译侧就没接上，这条用例的前提不成立：{v}"
        );

        // ② **每一条的文本都必须带回来** —— 这就是本次修的缺陷本身。
        let subs = v["subtitles"].as_object().expect("回包要带 subtitles");
        for a in &text_assets {
            let uri = a["uri"].as_str().unwrap_or("");
            let base = uri.rsplit(['/', '\\']).next().unwrap_or("");
            let text = subs
                .get(base)
                .unwrap_or_else(|| panic!(
                    "文字轨 `{base}` 的文本没带回来 —— 预览里这条文字不会显示（成片里是好的）：{subs:?}"
                ))
                .as_str()
                .unwrap_or("");
            assert!(!text.is_empty(), "文字轨 `{base}` 的内容是空的");
        }

        // ③ 内容真的是那条文字（不是读到了别的文件）。
        let any_has_text = subs.values().any(|t| {
            t.as_str().map(|s| s.contains("预览可见")).unwrap_or(false)
        });
        assert!(any_has_text, "带回来的字幕里找不到那条文字元素的内容：{subs:?}");

        let _ = std::fs::remove_dir_all(&clip);
    }


    /// **wasm 资源转发**：`engine.js` 取得到、`.wasm` 的 MIME 正确、路径穿越被挡。
    ///
    /// `#[ignore]`：要有**同层的 Dhampir 仓**（`web/engine.js` + `crates/dhampir-wasm/www/pkg`）。
    /// 这几条断的是"浏览器能不能把 wasm 加载起来"，缺了包就无从谈起 —— 所以宁可显式跳过，
    /// 也不写成"找不到包时自动通过"（那种测试永远不会红）。
    #[tokio::test]
    #[ignore = "手动：要同层的 Dhampir 仓（含已构建的 wasm 包）"]
    async fn dhampir_wasm_资源可转发且_mime_正确() {
        let t = test_app().await;
        let get = |uri: String| {
            let t = &t;
            async move { t.call(t.req("GET", &uri).body(Body::empty()).unwrap()).await }
        };

        // engine.js：页面的宿主引擎
        let res = get("/polish/dhampir-wasm/engine.js".to_string()).await;
        assert_eq!(res.status(), StatusCode::OK, "engine.js 应当取得到");
        let ct = res
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert!(ct.contains("javascript"), "engine.js 的 MIME 不对：{ct}");
        let js = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert!(js.len() > 1024, "engine.js 太小（{} 字节），不像是真的", js.len());

        // wasm 包：**MIME 必须是 application/wasm**，给错浏览器会拒绝流式实例化
        let res = get("/polish/dhampir-wasm/pkg/dhampir_wasm_bg.wasm".to_string()).await;
        assert_eq!(res.status(), StatusCode::OK, "wasm 应当取得到");
        let ct = res
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert_eq!(ct, "application/wasm", "wasm 的 MIME 必须精确是 application/wasm，实际：{ct}");

        // 穿越必须被挡
        for bad in [
            "/polish/dhampir-wasm/../Cargo.toml",
            "/polish/dhampir-wasm/pkg/../../Cargo.toml",
            "/polish/dhampir-wasm/pkg/nope.wasm",
        ] {
            let res = get(bad.to_string()).await;
            assert!(
                res.status() == StatusCode::BAD_REQUEST || res.status() == StatusCode::NOT_FOUND,
                "{bad} 应当被挡，实际 {}",
                res.status()
            );
        }
    }


    /// **wasm 响应的压缩与 ETag/304**（2026-10-02 首屏提速）。
    ///
    /// 这条钉的是「7.66 MB 被原样搬走」的回归：
    /// * 声明 `Accept-Encoding: gzip` ⇒ 必须回 `Content-Encoding: gzip` 且**明显更小**；
    /// * 不带该头 ⇒ **不许压**（否则发给解不开的客户端 = 直接白屏）；
    /// * 带 `If-None-Match: <etag>` ⇒ **304 且空体**（省掉没变时那 7.66 MB）；
    /// * 带 `Range` ⇒ **不许压**（压缩后与原始字节不再一一对应，会破坏播放）。
    ///
    /// `#[ignore]`：同 `dhampir_wasm_资源可转发且_mime_正确`，要同层的 Dhampir 仓。
    #[tokio::test]
    #[ignore = "手动：要同层的 Dhampir 仓（含已构建的 wasm 包）"]
    async fn dhampir_wasm_压缩与_etag_304() {
        let t = test_app().await;
        let req = |uri: &str, extra: Vec<(&str, &str)>| {
            let mut b = t.req("GET", uri);
            for (k, v) in extra {
                b = b.header(k, v);
            }
            b.body(Body::empty()).unwrap()
        };
        let uri = "/polish/dhampir-wasm/pkg/dhampir_wasm_bg.wasm";

        // ① 不带 Accept-Encoding ⇒ 原始字节、无 Content-Encoding
        let res = t.call(req(uri, vec![])).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            res.headers().get(axum::http::header::CONTENT_ENCODING).is_none(),
            "客户端没声明 gzip 却发了压缩体 —— 对方解不开"
        );
        assert!(
            res.headers().get(axum::http::header::ETAG).is_some(),
            "wasm 响应必须带 ETag（否则每次刷新都重下 7.66 MB）"
        );
        let etag = res
            .headers()
            .get(axum::http::header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let raw_len = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().len();

        // ② 声明 gzip ⇒ 更小 + 正确的 Content-Encoding + Vary
        let res = t.call(req(uri, vec![("accept-encoding", "gzip, deflate, br")])).await;
        assert_eq!(res.status(), StatusCode::OK);
        let enc = res
            .headers()
            .get(axum::http::header::CONTENT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(enc, "gzip", "声明了 gzip 却没压：{}", enc);
        assert_eq!(
            res.headers().get(axum::http::header::VARY).and_then(|v| v.to_str().ok()),
            Some("Accept-Encoding"),
            "少了 Vary：中间缓存会把压缩版喂给不支持 gzip 的客户端"
        );
        let gz_len = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().len();
        assert!(
            gz_len < raw_len / 2,
            "压缩没明显生效：{raw_len} -> {gz_len}（期望至少减半）"
        );

        // ③ 命中 If-None-Match ⇒ 304 且空体
        let res = t.call(req(uri, vec![("if-none-match", etag.as_str())])).await;
        assert_eq!(res.status(), StatusCode::NOT_MODIFIED, "ETag 没变就该 304");
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert!(body.is_empty(), "304 不许带 body，实际 {} 字节", body.len());

        // ④ 带 Range ⇒ 不许压（206 的字节必须与原始文件一一对应）
        let res = t.call(req(uri, vec![("range", "bytes=0-1023"), ("accept-encoding", "gzip")])).await;
        assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
        assert!(
            res.headers().get(axum::http::header::CONTENT_ENCODING).is_none(),
            "对 Range 响应做了压缩 —— 字节与原始文件错位，播放会花屏/解码失败"
        );
        let n = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().len();
        assert_eq!(n, 1024, "Range 字节数不对：{n}");
    }

}