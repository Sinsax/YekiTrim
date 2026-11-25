//! `api::project_data::clips`：片段（clips/ 目录）列表、缩略图、发布标题、删片段与重剪。

use std::path::{Path, PathBuf};

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use yekitrim_service::Workspace;

use crate::model::AppState;
use crate::media;
use crate::scope::Scope;

// 跨子模块 / 外部域：显式点名（不许 glob）
use super::assets::count_srt_cues;
use super::status::danmaku_starts;

use super::*;


pub(crate) async fn project_data_clips(
    scope: Scope,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let ws = scope.ws;
    let inm = headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    // 120+ 片段的列表 ≈ 120KB：序列化 + gzip 都在请求路径上，而内容只在任务完成 / 用户编辑后
    // 变化 ⇒ ETag/304 让切片页反复轮询几乎零成本（2026-11-25 排查「任务排队时预览卡」）。
    match blocking(move || data_clips_impl(&ws)).await {
        Ok(Json(v)) => crate::api::etag_json(inm.as_deref(), v),
        Err(e) => e.into_response(),
    }
}


pub(crate) async fn project_data_clip_thumb(scope: Scope, AxumPath(rank): AxumPath<String>) -> Response {
    let ws = scope.ws;
    // 路径参数可以是数字 rank，也可以是目录名（`clip-out`）—— 卡片墙按目录名请求缩略图。
    // 解析 + 两次 exists 都在项目目录上（网络盘上就是两次往返），下 blocking 池；
    // 卡片墙一次会打出几十个缩略图请求，留在 worker 上就是几十次白占。
    let rank_dir = rank.clone();
    let resolved = blocking(move || resolve_clip_dir(&ws.output_dir(), &rank_dir)).await;
    let clip_dir = match resolved {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let video = clip_dir.join("clip.mp4");
    let thumb = clip_dir.join("_thumb.jpg");
    let (v2, t2) = (video.clone(), thumb.clone());
    let (has_thumb, has_video) = match blocking(move || Ok((t2.exists(), v2.exists()))).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    if !has_thumb && has_video {
        // ffmpeg 是阻塞调用；同一张图用路径锁串行化，后到者拿到锁后重查即可跳过重复生成
        let _guard = yekitrim_core::filelock::lock_path(&thumb).await;
        if !thumb.exists() {
            let v = video.clone();
            let t = thumb.clone();
            match tokio::task::spawn_blocking(move || generate_clip_thumb(&v, &t)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::warn!("生成缩略图失败 ({}): {e}", thumb.display()),
                Err(e) => tracing::warn!("生成缩略图任务失败 ({}): {e}", thumb.display()),
            }
        }
    }
    match tokio::fs::read(&thumb).await {
        Ok(bytes) => ([(axum::http::header::CONTENT_TYPE, "image/jpeg")], bytes).into_response(),
        Err(_) => media::json_error(StatusCode::NOT_FOUND, "缩略图不存在"),
    }
}


/// 截取切片首帧为 `_thumb.jpg`（同步阻塞，调用方负责 spawn_blocking）。
pub(super) fn generate_clip_thumb(video: &Path, thumb: &Path) -> std::io::Result<()> {
    let status = yekitrim_core::process::cmd("ffmpeg")
        .args(["-ss","1","-i", &video.to_string_lossy(), "-vframes","1","-q:v","4","-y", &thumb.to_string_lossy()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other("ffmpeg 退出码非 0"));
    }
    Ok(())
}


pub(crate) async fn project_data_clip_silence(scope: Scope, State(state): State<Arc<AppState>>, AxumPath(rank): AxumPath<String>) -> ApiResult {
    // 隐式初始化精修工程 + 扫描静音 + 写屏蔽轨道：整段同步 IO，下 blocking 池
    let (ws, st, pid) = (scope.ws, state.clone(), scope.project.id);
    blocking(move || block_on_here(clip_silence_impl(&ws, &rank, &st, pid))).await
}


/// `DELETE /api/project-data/clips/:rank` —— 删除该切片目录（前端二次确认后调用）。
/// 路径参数接受数字 rank（`2` → `clip-02`）或目录名（`clip-out`），见 `resolve_clip_dir`。
pub(crate) async fn project_data_clip_delete(scope: Scope, State(state): State<Arc<AppState>>, AxumPath(rank): AxumPath<String>) -> ApiResult {
    // 删目录 + 重写发布索引都是对项目目录的同步 IO（可能在网络盘上）：整段下 blocking 池
    let (ws, pid) = (scope.ws, scope.project.id);
    blocking(move || block_on_here(clip_delete_impl(&ws, &rank, &state, pid))).await
}


/// 单个路径的 mtime（unix 秒）；取不到（不存在 / 权限 / 时钟怪值）时是 0。
fn mtime_secs(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}


/// 目录「最后一次改动」的 unix 秒 = 目录本身与**最多两层**子项里最新的 mtime。
///
/// 首页「切片总览」按这个值倒序（最新改动的片段浮到最前），所以它要覆盖「用户刚动过这个片段」
/// 的几条路：剪切（`clip.mp4` / `clip.json`）、精修保存（`polish.toml` / `clip.srt`）、
/// 出片与封面（`vtuber-clip_N.mp4` / `cover_N.jpg` —— 都是**直接子项**），精修侧的数据文件
/// 在 `data/` 里（只多一层，且新增/删除会带起 `data/` 自己的 mtime）。
/// ⚠️ **不递归到底**：项目目录经常挂在网络盘上（RaiDrive / WebDAV），深扫一个项目几百个片段
/// 的代价不可控，而这个接口每次页面加载都会打。
fn last_touched_secs(dir: &Path) -> u64 {
    fn walk(path: &Path, depth: u8, best: &mut u64) {
        *best = (*best).max(mtime_secs(path));
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(path) else { return };
        for entry in entries.flatten() {
            // `DirEntry::metadata()` 在 Windows 上直接用目录项里的数据，不额外 `stat` ——
            // 网络盘上这一条很要紧（这里每个片段要走一遍）。
            let Ok(meta) = entry.metadata() else { continue };
            if let Some(secs) = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
            {
                *best = (*best).max(secs);
            }
            if depth > 1 && meta.is_dir() {
                walk(&entry.path(), depth - 1, best);
            }
        }
    }
    let mut best = 0;
    walk(dir, 2, &mut best);
    best
}


/// 一条切片的**剪辑质量分**（总方案 §3）；算不出来回 `None`（前端不显示那一栏）。
///
/// ⚠️ **三条"不算"的口径**（都是"宁可不显示，也不给一个错的数"）：
/// - 没有 `polish.toml` ⇒ 还没精修：没有分段/事件，算出来是"零干扰满分" —— **假的高分**；
/// - 没有 `data/subtitles.json` ⇒ 句子判据无源：只剩"爆点/间隔"两条在工作，
///   分数**偏高且不可比**（用户无从知道少算了一条）；
/// - 解析失败 ⇒ 同上。
///
/// 时长取 `clip.json` 的 `duration`（即 `duration_secs`）；它 ≤ 0 时退回"末段 end"——
/// 老片段可能没有 duration，退回后至少还能算出一个分数。
///
/// ⚠️ **下面有两道冗余闸**（`polish.toml` 存在性 + `load_project` 成功），**两道都要留**：
/// 它们现在**互相遮蔽** —— 单独去掉任一道，没精修的片段仍会因为另一道回 `None`，
/// 所以**变异自证必须两道一起拆**才翻红（实测：只拆一道时
/// `clips_回质量分_但没精修过的回_null` 照样绿，差点被误判成"测试没锁住"）。
/// 留两道是因为第一道是**语义**（"没精修过"）、第二道是**容错**（文件在但读坏了）——
/// 判据不同，不是重复。
fn clip_quality(dir: &Path, duration_secs: f64) -> Option<serde_json::Value> {
    if !dir.join("polish.toml").exists() {
        return None;
    }
    let subs_raw = std::fs::read_to_string(dir.join("data").join("subtitles.json")).ok()?;
    let subtitles: Vec<yekitrim_polish::parse::srt::Subtitle> = serde_json::from_str(&subs_raw).ok()?;
    let cfg = yekitrim_polish::project::load_project(dir).ok()?;
    let duration = if duration_secs > 0.0 {
        duration_secs
    } else {
        cfg.segments.last().map(|s| s.end).unwrap_or(0.0)
    };
    let q = yekitrim_polish::process::quality::evaluate(&cfg.segments, &subtitles, duration);

    // 逐条干扰也回给前端：切片页 tooltip 要写"哪儿扣的分" ——
    // 只回一个数的话用户没法判断"这个 62 分到底怎么了"（与 §1 同一套理由）。
    let items: Vec<serde_json::Value> = q
        .interruptions
        .iter()
        .map(|i| serde_json::json!({ "kind": i.kind.key(), "at": i.at, "label": i.kind.label() }))
        .collect();
    Some(serde_json::json!({
        // 展示分保留 1 位小数 —— 界面上就显示到这个精度，别传一串浮点尾数
        "score": (q.score() * 10.0).round() / 10.0,
        "smoothness": (q.smoothness() * 100.0).round() / 100.0,
        "per_minute": (q.interruptions_per_minute() * 100.0).round() / 100.0,
        "count": q.interruption_count(),
        "segments": q.segments,
        "visual_events": q.visual_events,
        "interruptions": items,
    }))
}


/// 统计落在 [start, end] 内的起始点数量（starts 必须升序）。
pub(super) fn count_in_range(starts: &[f64], start: f64, end: f64) -> u64 {
    if starts.is_empty() || end < start { return 0; }
    let lo = starts.partition_point(|&x| x < start);
    let hi = starts.partition_point(|&x| x <= end);
    hi.saturating_sub(lo) as u64
}


pub(super) fn data_clips_impl(ws: &Workspace) -> ApiResult {
    let clips = ws.output_dir().join("clips");
    // ⚠️ 目录不存在 = **还没有片段**，是正常空态，不是"资源不存在"（2026-11-15 修）：
    // 以前回 404「暂无 clips」，而切片页把四个接口放在 Promise.all 里 ——
    // 一个 404 让整页 catch，连**已经读到的 events 也一起丢掉**，界面显示
    // 「切片数据加载失败：暂无 clips」+ 事件列表空。实测用户报的就是这个
    // （刚跑完「查找事件」、67 条事件明明在 events.json 里，只是还没剪片段）。
    if !clips.exists() { return ok(serde_json::json!([])); }
    // 发布标题（`publish_config.json` 的 `clips[].title`）：页面上那个「发布标题」框要显示
    // **投稿真正会用的那句**，而不是 `clip.json` 的原始标题 —— 改过标题的片段刷新后
    // 必须还是改过的那句（否则用户会以为没保存成功，再改一遍）。整份文件读一次即可。
    // ⚠️ 这里读的 `title` 是**纯标题**（切片产物不带主播名，2026-11-22 起）；
    // 名字在下面用 `naming` 实时合成。第二项是片段的覆盖值（`None` = 跟随项目级）。
    let publish_entries: std::collections::HashMap<String, (String, Option<String>)> =
        std::fs::read_to_string(ws.output_dir().join("publish_config.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|v| {
                v.get("clips").and_then(|c| c.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|c| {
                            let dir = c.get("directory").and_then(|d| d.as_str())?;
                            let title = c.get("title").and_then(|t| t.as_str()).unwrap_or("");
                            let streamer = c
                                .get("streamer_name")
                                .and_then(|s| s.as_str())
                                .map(str::to_string);
                            Some((dir.to_string(), (title.to_string(), streamer)))
                        })
                        .collect()
                })
            })
            .unwrap_or_default();
    // 项目级主播名：产物标题的唯一来源（`[detection].vtuber_name`）。读不到就当空 ——
    // **绝不因为配置读失败而拦掉整个切片列表**（那是用户唯一的操作面）。
    let project_streamer = ws
        .load_config()
        .map(|c| c.detection.vtuber_name)
        .unwrap_or_default();
    let mut out = vec![];
    // 源弹幕的起始时间表：只有遇到「没有 clip.dm.srt 的老切片」时才解析一次（见下）
    let mut dm_starts: Option<Vec<f64>> = None;
    let entries = std::fs::read_dir(&clips).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    for entry in entries.flatten() {
        if !entry.path().is_dir() { continue; }
        let dir_path = entry.path();
        let dir_name = entry.file_name().to_string_lossy().into_owned();
        let meta_path = dir_path.join("clip.json");
        let meta = std::fs::read_to_string(&meta_path).ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .unwrap_or(serde_json::json!({}));
        let video_path = dir_path.join("clip.mp4");
        let (size_bytes, duration_secs) = if video_path.exists() {
            let size = std::fs::metadata(&video_path).map(|m| m.len()).unwrap_or(0);
            let dur = meta.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
            (size, dur)
        } else { (0,0.0) };
        let silent_files: Vec<String> = if let Ok(rd)=std::fs::read_dir(&dir_path) { rd.flatten().filter_map(|f| { let n=f.file_name().to_string_lossy().to_lowercase(); if n.contains("cleaned") && n.ends_with(".mp4") { Some(f.file_name().to_string_lossy().into_owned()) } else { None } }).collect() } else { Vec::new() };
        // 「消音版」= 静音区间已写入屏蔽轨道（不再产生 *_cleaned.mp4 文件）
        let silenced = yekitrim_polish::project::is_silent(&dir_path);
        let mute_range_count = if dir_path.join("polish.toml").exists() { yekitrim_polish::project::mute_range_count(&dir_path) } else { 0 };
        // 切片自己的弹幕/字幕条数（切片页的「弹幕量 / 字幕量」排序用）
        let subtitle_count = count_srt_cues(&dir_path.join("clip.srt"));
        let dm_file = dir_path.join("clip.dm.srt");
        let danmaku_count = if dm_file.exists() {
            count_srt_cues(&dm_file)
        } else {
            // 老切片没有 clip.dm.srt（这个文件是后加的）：改成按 clip.json 的原片区间从源弹幕里数。
            // 不算这一步整列都是 0，「弹幕量」排序会退化成按 rank 排（用户反馈过"排序没生效"）。
            let starts = dm_starts.get_or_insert_with(|| danmaku_starts(ws));
            let start = meta.get("original_start").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let end = meta.get("original_end").and_then(|v| v.as_f64()).unwrap_or(0.0);
            count_in_range(&starts[..], start, end)
        };
        // 剪辑质量分（总方案 §3）：**只在已初始化精修工程时**才算 —— 没有 `polish.toml`
        // 就没有分段/事件，算出来是"零干扰满分"，那是**假的高分**、比没有更糟。
        // 算不出来（工程缺失/字幕缺失/解析失败）一律 `null`，前端不显示那一栏。
        let quality = clip_quality(&dir_path, duration_secs);
        // 先取出来再进 `json!`：`dir_name` 在下面会被 `json!` 吃掉（`to_value` 按值），
        // 之后再 `get(&dir_name)` 就借不到了。
        // 纯标题（编辑框那一份）：发布索引优先，没有再回落到 `clip.json` 的标题。
        let (stored_title, streamer_override) = publish_entries
            .get(&dir_name)
            .cloned()
            .unwrap_or_default();
        let base_title = if stored_title.trim().is_empty() {
            meta.get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string()
        } else {
            stored_title
        };
        // 三态归一化（含"老数据里 title 已经带着 `【旧名】`"那段）：
        // `(正文, 生效的名字)` —— 规则只有 `yekitrim_core::naming` 一份。
        let (title_core, streamer_effective) = yekitrim_core::naming::resolve_streamer(
            streamer_override.as_deref(),
            &base_title,
            &project_streamer,
        );
        let publish_title = yekitrim_core::naming::streamer_title(&title_core, &streamer_effective);
        out.push(serde_json::json!({
            "dir": yekitrim_core::path::display(&dir_path),
            "name": dir_name,
            // 发布标题 = **合成后**的那句（显示/复制用；投稿送出去的也是它）
            "publish_title": publish_title,
            // 纯标题（编辑框的值；不含主播名）
            "publish_title_core": title_core,
            // 片段级覆盖：null = 跟随项目级；"" = 这段不用；其它 = 锁定
            "streamer_name": streamer_override,
            // 实际生效的名字（界面拿它显示「跟随中 · 某某」）
            "streamer_name_effective": streamer_effective,
            "video_exists": video_path.exists(),
            "size_bytes": size_bytes,
            "duration_secs": duration_secs,
            // 「最后一次改动」= 这个片段目录里最新的 mtime（unix 秒，见 `last_touched_secs`）：
            // 首页「切片总览」按它倒序，刚剪完 / 刚精修的片段排在最前面。
            "mtime": last_touched_secs(&dir_path),
            "silenced": silenced,
            "mute_range_count": mute_range_count,
            "silent_files": silent_files,
            "meta": meta,
            "subtitle_count": subtitle_count,
            "danmaku_count": danmaku_count,
            "quality": quality,
        }));
    }
    // 目录名不再保证是 `clip-NN`（新片段用标题 slug）：优先按 `clip.json` 的 rank 排，
    // 没有 rank 的老数据再尝试解析 `clip-NN`，最后按目录名兜底。
    let rank_of = |v: &serde_json::Value| -> u64 {
        if let Some(r) = v
            .get("meta")
            .and_then(|m| m.get("rank"))
            .and_then(|r| r.as_u64())
            .filter(|r| *r > 0)
        {
            return r;
        }
        v.get("name")
            .and_then(|n| n.as_str())
            .and_then(|n| n.strip_prefix("clip-"))
            .and_then(|r| r.parse::<u64>().ok())
            .unwrap_or(u64::MAX)
    };
    out.sort_by(|a, b| {
        rank_of(a)
            .cmp(&rank_of(b))
            .then_with(|| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")))
    });
    ok(serde_json::json!(out))
}


/// 「消音版」：把该切片的长静音段写入屏蔽轨道（polish.toml 的 mute_ranges）。
/// 不生成任何新文件，原始 clip.mp4 / clip.srt 保持不变。
pub(super) async fn clip_silence_impl(ws: &Workspace, rank: &str, state: &AppState, pid: i64) -> ApiResult {
    // rank 直接来自 URL：先解析成数字再拼路径（`clip-../../x` 会走出项目目录）
    let Ok(rank) = rank.parse::<u32>() else {
        return Err(err(StatusCode::BAD_REQUEST, "rank 无效"));
    };
    let clip_dir = ws.output_dir().join("clips").join(format!("clip-{:0>2}", rank));
    let video = clip_dir.join("clip.mp4");
    if !video.exists() { return Err(err(StatusCode::NOT_FOUND, "clip.mp4 不存在")); }
    // 会隐式初始化精修工程 + 写屏蔽轨道：该片段有任务在跑时当场拒绝
    if let Some(busy) = state.jobs.busy_with(&crate::scope_key::polish_keys(Some(pid), &clip_dir)).await {
        tracing::info!("拒绝在 {} 上改屏蔽轨道：{busy}", busy.resource);
        return Err(err(StatusCode::CONFLICT, format!("{busy}，请等它结束再改屏蔽轨道")));
    }
    ensure_polish_project(&clip_dir).await?;
    // polish.toml 的 read-modify-write 要和整修页的保存走**同一把**路径锁，否则
    // "编辑器保存设计" 与 "去静音" 并发时后写者会把先写者的改动整段盖掉。
    // 锁在 await 里持有；ffmpeg 静音扫描 + 写盘下 blocking 池。
    let _guard = yekitrim_core::filelock::lock_path(&clip_dir.join("polish.toml")).await;
    let applied = blocking(move || {
        yekitrim_polish::project::apply_silence(&clip_dir)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入屏蔽轨道失败: {e:#}")))
    })
    .await?;
    ok(serde_json::json!({
        "ok": true,
        "detected": applied.ranges.len(),
        "added": applied.added,
        "total": applied.total,
        "ranges": applied.ranges.iter().map(|(s, e)| serde_json::json!({ "start": s, "end": e })).collect::<Vec<_>>(),
    }))
}


/// 把切片接口的路径参数解析成 `clips/` 下的目录：**数字 rank**（`2` → `clip-02`，老契约）
/// 或**目录名本身**（`clip-out` 这类手工命名的目录）。
///
/// 目录名优先 —— 卡片墙列的是 `clips/` 下的**真实目录名**，名字不一定规范；以前只认数字，
/// 前端把 `clip-out` 解析成 rank 0 → 请求 `clips/0` → 稳定报「clip-00 不存在」。
/// 参数直接来自 URL，所以先过 `is_safe_component`（`..` / 分隔符 / 盘符 / 保留名一律拒绝），
/// 拼出来的路径就只可能落在 `clips/` 里面。
pub(super) fn resolve_clip_dir(output_dir: &Path, arg: &str) -> Result<PathBuf, AppError> {
    if !yekitrim_polish::assets::sticker_index::is_safe_component(arg) {
        return Err(err(StatusCode::BAD_REQUEST, "rank 无效"));
    }
    let clips = output_dir.join("clips");
    let by_name = clips.join(arg);
    if by_name.is_dir() {
        return Ok(by_name);
    }
    match arg.parse::<u32>() {
        // 没有同名目录时按老契约理解成 rank（`clips/2` → `clips/clip-02`）
        Ok(rank) => Ok(clips.join(format!("clip-{:0>2}", rank))),
        // 名字不存在：原样返回，调用方按「{arg} 不存在」报 404
        Err(_) => Ok(by_name),
    }
}


/// 删除一个已剪切片段：整个 `clips/<目录名>/` 目录移除（精修产物、成片、封面一并删掉），
/// 并把引用从 `publish_config.json` / `clips_order.txt` 里摘掉 —— 否则发布配置指向不存在的文件。
///
/// 路径参数可以是数字 rank，也可以是目录名（见 `resolve_clip_dir`）。
/// `events.json` 不动：目录没了，该事件在切片页自动回到「未剪切」，可以重新切。
pub(super) async fn clip_delete_impl(ws: &Workspace, arg: &str, state: &AppState, pid: i64) -> ApiResult {
    let clip_dir = resolve_clip_dir(&ws.output_dir(), arg)?;
    let Some(dir_name) = clip_dir.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Err(err(StatusCode::BAD_REQUEST, "rank 无效"));
    };
    if !clip_dir.is_dir() {
        return Err(err(StatusCode::NOT_FOUND, format!("{dir_name} 不存在")));
    }
    // 删目录是同步操作：该片段上有任务在跑（渲染/精修/重切）时当场拒绝 —— 删到一半
    // 被任务写回来会留下半个工程
    if let Some(busy) = state
        .jobs
        .busy_with(&[
            crate::scope_key::project_clip(pid, &dir_name),
            crate::scope_key::dir_key(&clip_dir),
            crate::scope_key::project_index(pid),
        ])
        .await
    {
        tracing::info!("拒绝删除 {}：{busy}", busy.resource);
        return Err(err(StatusCode::CONFLICT, format!("{busy}，请等它结束再删这个片段")));
    }
    // 下面两步都是同步文件访问，一起下 blocking 池：`remove_dir_all` 删的是**含成片的整个片段目录**
    // （项目在网络盘上时最重的就是这一处），`prune_clip_from_index` 内部要读
    // `publish_config.json` / `clips_order.txt` 再重建 `index.json` —— 同样同步 IO，
    // 所以必须跟删除落在**同一个闭包**里，不能只搬一半。
    // ⚠️ `busy_with` 是 async，留在闭包外（见上）。
    let del_dir = clip_dir.clone();
    let del_name = dir_name.clone();
    let output_dir = ws.output_dir();
    blocking(move || {
        std::fs::remove_dir_all(&del_dir).map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("删除 {del_name} 失败: {e}"),
            )
        })?;
        prune_clip_from_index(&output_dir, &del_name);
        Ok(())
    })
    .await?;
    tracing::info!("已删除片段目录: {}", clip_dir.display());
    ok(serde_json::json!({ "ok": true, "removed": dir_name }))
}


/// 把某个切片从 `publish_config.json` / `clips_order.txt` 里摘掉（删目录后的索引同步）。
/// 两个文件都是 Cutter 的产物，缺失或格式异常就静默跳过 —— 目录本身已经删掉了。
pub(super) fn prune_clip_from_index(output_dir: &Path, dir_name: &str) {
    let publish = output_dir.join("publish_config.json");
    if let Ok(raw) = std::fs::read_to_string(&publish) {
        if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(arr) = v.get_mut("clips").and_then(|c| c.as_array_mut()) {
                let before = arr.len();
                arr.retain(|c| c.get("directory").and_then(|d| d.as_str()) != Some(dir_name));
                if arr.len() != before {
                    if let Ok(out) = serde_json::to_string_pretty(&v) {
                        let _ = std::fs::write(&publish, out);
                    }
                }
            }
        }
    }
    // clips_order.txt：每行 `clip-NN / 标题`（没有标题时只有 `clip-NN`）
    let order = output_dir.join("clips").join("clips_order.txt");
    if let Ok(raw) = std::fs::read_to_string(&order) {
        let kept: Vec<&str> = raw
            .lines()
            .filter(|l| {
                let t = l.trim();
                t != dir_name && !t.starts_with(&format!("{dir_name} "))
            })
            .collect();
        if kept.len() != raw.lines().count() {
            let _ = std::fs::write(&order, kept.join("\n"));
        }
    }
    // 目录已经删掉：从磁盘重建 index.json，让被删条目同步消失。
    if let Err(e) = yekitrim_core::cutting::clip_index::refresh_index(output_dir) {
        tracing::warn!("刷新片段索引失败（删除 {} 后）: {e:#}", dir_name);
    }
}


/// 发布标题的字数上限（B站标题 80 字，这里留足余量给「标题 + 主播名」）。
const MAX_TITLE_CHARS: usize = 200;


/// 改一个片段的**发布标题**（output/publish_config.json 的 clips[].title）。
///
/// 为什么写这一份、不写 clip.json：投稿链路（crate::publish → biliLive-tools）读的就是
/// publish_config.json，改标题的目的正是「投稿时用我改过的这句」。
/// ⚠️ **刻意不碰 clip.json 的 title**：它是片段的**身份** —— clip_index::clip_meta_matches
/// 用「标题相同 或 原片区间 IoU ≥ 0.5」判断重新检测时能不能复用这个目录；改了它，老片段在重新
/// 检测后会匹配不上、另开一个目录（精修产物留在旧目录里，用户看到的是「精修白做了」）。
///
/// 生命周期：重切**这个**片段会按 clip.json 重新生成标题（Cutter 的「同名覆盖」），重切**别的**
/// 片段不会动它 —— 后者由 publish_index_merges_incremental_cuts_* 钉着。
/// 片段的**主播名覆盖**改动意图（三态，别拍平成 `Option<String>` —— 那就分不出
/// "不动" 与 "清除覆盖"）。
#[derive(Default)]
pub(super) enum StreamerPatch {
    /// 这次只改标题，主播名保持原样。
    #[default]
    Keep,
    /// 清除覆盖 ⇒ 恢复跟随项目级（键直接删掉）。
    Follow,
    /// 设成这个值；空串 = 这个片段明确不用主播名。
    Set(String),
}


pub(super) fn clip_title_put_impl(
    ws: &Workspace,
    arg: &str,
    title: &str,
    streamer: &StreamerPatch,
) -> ApiResult {
    let output = ws.output_dir();
    let clip_dir = resolve_clip_dir(&output, arg)?;
    let Some(dir_name) = clip_dir.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Err(err(StatusCode::BAD_REQUEST, "rank 无效"));
    };
    if !clip_dir.is_dir() {
        return Err(err(StatusCode::NOT_FOUND, format!("{dir_name} 不存在")));
    }
    let publish = output.join("publish_config.json");
    let raw = std::fs::read_to_string(&publish).map_err(|e| {
        err(
            StatusCode::NOT_FOUND,
            format!("还没有发布索引 publish_config.json（重新剪一次会重建）: {e}"),
        )
    })?;
    let mut v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("publish_config.json 解析失败: {e}"))
    })?;
    let mut hit = false;
    if let Some(arr) = v.get_mut("clips").and_then(|c| c.as_array_mut()) {
        for c in arr.iter_mut() {
            if c.get("directory").and_then(|d| d.as_str()) == Some(dir_name.as_str()) {
                c["title"] = serde_json::json!(title);
                // 主播名三态：Keep 一个字节都不动；Follow 删键；Set 写值。
                match streamer {
                    StreamerPatch::Keep => {}
                    StreamerPatch::Follow => {
                        if let Some(obj) = c.as_object_mut() {
                            obj.remove("streamer_name");
                        }
                    }
                    StreamerPatch::Set(v) => c["streamer_name"] = serde_json::json!(v),
                }
                hit = true;
            }
        }
    }
    if !hit {
        return Err(err(
            StatusCode::NOT_FOUND,
            format!("{dir_name} 还没进发布索引（重新剪一次会重建）"),
        ));
    }
    let pretty = serde_json::to_string_pretty(&v)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    std::fs::write(&publish, pretty).map_err(|e| {
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入 publish_config.json 失败: {e}"))
    })?;
    sync_order_title(&output, &dir_name, title);
    ok(serde_json::json!({
        "ok": true,
        "directory": dir_name,
        "title": title,
        "streamer_name": match streamer {
            StreamerPatch::Keep => serde_json::Value::Null,
            StreamerPatch::Follow => serde_json::Value::Null,
            StreamerPatch::Set(v) => serde_json::json!(v),
        },
    }))
}


/// clips/clips_order.txt 每行「<目录名> <标题>」：标题跟着改，否则那份索引还念着旧标题。
/// 文件缺失/格式异常一律静默跳过（与 prune_clip_from_index 同一条口径）。
fn sync_order_title(output_dir: &Path, dir_name: &str, title: &str) {
    let order = output_dir.join("clips").join("clips_order.txt");
    let Ok(raw) = std::fs::read_to_string(&order) else { return };
    // ⚠️ 分隔符必须与 Cutter 写的一致（目录名 + ' / ' + 标题，见 cutting/mod.rs 的 order_lines）：
    // 写成空格虽然仍能被 starts_with(目录名 + ' ') 命中，但同一份文件出现两种格式，人看像两份东西。
    let prefix = format!("{dir_name} ");
    let want = if title.trim().is_empty() {
        dir_name.to_string()
    } else {
        format!("{dir_name} / {title}")
    };
    let mut changed = false;
    let lines: Vec<String> = raw
        .lines()
        .map(|line| {
            let t = line.trim();
            if t == dir_name || t.starts_with(&prefix) {
                changed |= t != want;
                want.clone()
            } else {
                line.to_string()
            }
        })
        .collect();
    if changed {
        let _ = std::fs::write(&order, lines.join("\n"));
    }
}


/// `PUT /api/project-data/clips/:rank/title` 的请求体。
#[derive(serde::Deserialize)]
pub(super) struct ClipTitleBody {
    /// 发布标题。**必填**（空串 = 清掉标题，与前端 `setClipTitle` 同口径）。
    pub title: String,
    /// 主播名覆盖，**三态**直接映射成 `StreamerPatch`（键缺席 = Keep / `null` = Follow /
    /// 字符串 = Set）。见下面那个 `Deserialize` impl。
    #[serde(default)]
    pub streamer_name: StreamerPatch,
}



/// **三态反序列化**（`#[serde(default)]` 落到 `Keep`）：键缺席 = 不动；`null` = 跟随项目级；
/// 字符串 = 锁定（空串 = 这段不用）；其它类型 ⇒ 400 且文案点名 `streamer_name`。
///
/// ⚠️ **不能**写成 `Option<serde_json::Value>`：serde 的 `Option<T>` 会把 `null` 解析成 `None`，
/// 于是"键缺席"与"显式 null"混成一档 —— 而这两个语义**相反**（一个不动、一个清掉用户的覆盖）。
/// 实测（2026-11-24）：写成 Option 后 ⑤「清回 null ⇒ 恢复跟随」那条老用例立刻红。
impl<'de> serde::Deserialize<'de> for StreamerPatch {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::Null => Ok(StreamerPatch::Follow),
            serde_json::Value::String(s) => Ok(StreamerPatch::Set(s.trim().to_string())),
            _ => Err(serde::de::Error::custom(
                "streamer_name 必须是字符串或 null",
            )),
        }
    }
}


/// PUT /api/project-data/clips/:rank/title —— 改发布标题（正文见 clip_title_put_impl）。
pub(crate) async fn project_data_clip_title_put(
    scope: Scope,
    AxumPath(rank): AxumPath<String>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let ws = scope.ws;
    let ClipTitleBody { title, streamer_name } = parse_body(body)?;
    let title = title.trim().to_string();
    let len = title.chars().count();
    if len > MAX_TITLE_CHARS {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("标题最多 {MAX_TITLE_CHARS} 字（当前 {len}）"),
        ));
    }
    // 主播名的改动意图（三态，见 `yekitrim_core::naming`）：**结构体层的 `Deserialize` impl
    // 已经把它映射好了**（缺席 = Keep / null = Follow / 字符串 = Set / 其它 = 400 点名），
    // 这里不用再 match 一遍 —— 两处判据各写一份必然漂。
    let patch = streamer_name;
    // publish_config.json 是 read-modify-write（Cutter 也在写它）：与它走同一条路径锁
    let path = ws.output_dir().join("publish_config.json");
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || clip_title_put_impl(&ws, &rank, &title, &patch)).await
}


/// 精修工程不存在时自动初始化（与前端进入精修时的行为一致）。
///
/// 实现在 `yekitrim_service::init_polish_for_clip`（**唯一一份**）：剪切收尾也用同一条路，
/// 这里只把错误映射成 HTTP。`Ok(false)` = 已经有工程（或目录里没有 clip.mp4）= 无事可做。
pub(crate) async fn ensure_polish_project(dir: &Path) -> Result<(), AppError> {
    match yekitrim_service::init_polish_for_clip(dir).await {
        Ok(_) => Ok(()),
        Err(e) => Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("初始化精修工程失败: {e:#}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;
    use crate::jobs::{JobClass, JobSpec};
    use axum::body::Body;


    /// **§3 剪辑质量分**：`/project-data/clips` 必须回它，且**没精修过的片段回 `null`**。
    ///
    /// 两条判据缺一不可：
    /// ① 有 `polish.toml` + 字幕 ⇒ 回一个对象（含 `score` 与逐条 `interruptions`）；
    /// ② **没有 `polish.toml`** ⇒ 回 `null`。这一条是**防"假的高分"**：没有分段/事件时
    ///    四条判据全部无可判 ⇒ `evaluate` 得 0 干扰 ⇒ **满分 100**。把它显示出去比不显示更糟 ——
    ///    用户会以为"这条剪得完美"，而其实系统**根本还没看过它**。
    #[tokio::test]
    async fn clips_回质量分_但没精修过的回_null() {
        let t = test_app().await;
        let pid = project_ids(&t).await[0];
        let clips_dir = t.root.join("output/clips");
        std::fs::create_dir_all(&clips_dir).unwrap();

        // ① 没精修过的片段：**有字幕、但没有 polish.toml**。
        //    ⚠️ 必须有字幕 —— 否则它会因为"读不到 subtitles.json"而回 null，
        //    那样即使删掉 polish.toml 那道检查，测试**照样绿**（变异自证当场抓出过这个漏洞：
        //    判据没有隔离到它要锁的那一行）。带上字幕后，唯一能让它回 null 的理由
        //    就只剩"没有 polish.toml"这一条。
        let bare = clips_dir.join("bare");
        std::fs::create_dir_all(bare.join("data")).unwrap();
        std::fs::write(bare.join("clip.json"), r#"{"rank":1,"duration":20.0}"#).unwrap();
        std::fs::write(
            bare.join("data/subtitles.json"),
            r#"[{"index":1,"start":0.5,"end":3.0,"text":"这话还没说完","html":""}]"#,
        )
        .unwrap();

        // ② 已精修：polish.toml + data/subtitles.json。边界 1.0 切在一句话中间 ⇒ 该扣分。
        let done = clips_dir.join("done");
        std::fs::create_dir_all(done.join("data")).unwrap();
        std::fs::write(done.join("clip.json"), r#"{"rank":2,"duration":10.0}"#).unwrap();
        std::fs::write(
            done.join("data/subtitles.json"),
            r#"[{"index":1,"start":0.5,"end":3.0,"text":"这话还没说完","html":""}]"#,
        )
        .unwrap();
        let cfg = yekitrim_polish::project::ProjectFile::example();
        std::fs::write(done.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();
        // `example()` 的分段是 0~10（单段）⇒ 没有内部切点。手工换成两段、边界落句中。
        let mut two = cfg;
        two.segments = vec![
            yekitrim_polish::project::Segment {
                role: String::new(),
                start: 0.0, end: 1.0, fx: "low".into(), note: String::new(),
                events: vec![], extra: Default::default(),
            },
            yekitrim_polish::project::Segment {
                role: String::new(),
                start: 1.0, end: 10.0, fx: "med".into(), note: String::new(),
                events: vec![], extra: Default::default(),
            },
        ];
        std::fs::write(done.join("polish.toml"), two.to_toml_commented().unwrap()).unwrap();

        let res = t
            .call(t.req("GET", &format!("/project-data/clips?project={pid}")).body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        let json = body_json(res).await;
        let arr = json.as_array().expect("clips 要回数组");

        let by_name = |n: &str| {
            arr.iter()
                .find(|c| c.get("name").and_then(|v| v.as_str()) == Some(n))
                .unwrap_or_else(|| panic!("找不到片段 {n}: {json}"))
        };

        // ① 没精修过 ⇒ null（**不是** 100 分）
        assert!(
            by_name("bare").get("quality").map(|q| q.is_null()).unwrap_or(false),
            "没精修过的片段 quality 必须是 null —— 回满分就是「假的高分」: {}",
            by_name("bare")
        );

        // ② 已精修 ⇒ 有对象、分数被扣、逐条理由带着位置
        let q = by_name("done").get("quality").expect("要有 quality");
        assert!(!q.is_null(), "已精修过的片段必须回质量分: {q}");
        let score = q.get("score").and_then(|v| v.as_f64()).expect("score 要是数字");
        assert!(score < 100.0, "边界切在句子中间，分数必须低于满分，实得 {score}");
        assert_eq!(q.get("count").and_then(|v| v.as_u64()), Some(1), "应当正好 1 条干扰: {q}");
        let items = q.get("interruptions").and_then(|v| v.as_array()).expect("要带逐条明细");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get("kind").and_then(|v| v.as_str()), Some("mid_sentence"));
        assert_eq!(items[0].get("at").and_then(|v| v.as_f64()), Some(1.0));
        assert!(
            items[0].get("label").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false),
            "每条要有给人看的理由（界面 tooltip 直接显示）: {items:?}"
        );
    }


/// 删切片后的索引同步（破坏性路径）：publish_config.json 的条目要摘掉、
/// clips_order.txt 的对应行要删掉，且**不能误伤别的片段**（clip-13 不能带走 clip-133）。
#[test]
fn prune_clip_from_index_removes_only_target_clip() {
    let td = tempfile::tempdir().unwrap();
    let out = td.path();
    std::fs::create_dir_all(out.join("clips")).unwrap();
    let publish = serde_json::json!({
        "video_source": "x.mp4",
        "clips": [
            { "directory": "clip-01", "rank": 1, "file": "clips/clip-01/clip.mp4" },
            { "directory": "clip-02", "rank": 2, "file": "clips/clip-02/clip.mp4" },
            { "directory": "clip-13", "rank": 13, "file": "clips/clip-13/clip.mp4" },
        ]
    });
    std::fs::write(out.join("publish_config.json"), serde_json::to_string_pretty(&publish).unwrap()).unwrap();
    std::fs::write(out.join("clips").join("clips_order.txt"), "clip-01 / 甲\nclip-02 / 乙\nclip-13 / 丙\nclip-133 / 丁").unwrap();

    prune_clip_from_index(out, "clip-02");
    prune_clip_from_index(out, "clip-13");

    let raw = std::fs::read_to_string(out.join("publish_config.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let dirs: Vec<&str> = v["clips"].as_array().unwrap().iter().filter_map(|c| c["directory"].as_str()).collect();
    assert_eq!(dirs, vec!["clip-01"]);

    let order = std::fs::read_to_string(out.join("clips").join("clips_order.txt")).unwrap();
    assert_eq!(order, "clip-01 / 甲\nclip-133 / 丁");
}


/// 索引文件缺失、或本来就没有该片段时必须是静默无操作（删目录已经成功了）。
#[test]
fn prune_clip_from_index_is_no_op_when_absent() {
    let td = tempfile::tempdir().unwrap();
    let out = td.path();
    prune_clip_from_index(out, "clip-09"); // 连 clips/ 都还没有
    std::fs::create_dir_all(out.join("clips")).unwrap();
    std::fs::write(out.join("clips").join("clips_order.txt"), "clip-01 / 甲").unwrap();
    prune_clip_from_index(out, "clip-02");
    assert_eq!(std::fs::read_to_string(out.join("clips").join("clips_order.txt")).unwrap(), "clip-01 / 甲");
}


/// 切片页两条回归（走真实路由 + 真实文件）：
/// 1) 老切片没有 `clip.dm.srt` 时，弹幕量按 `clip.json` 的原片区间从**源弹幕**数出来 ——
///    否则整列都是 0，「弹幕量」排序会静默退化成按 rank 排（用户报过"排序没生效"）；
///    有 `clip.dm.srt` 时仍以文件为准。
/// 2) `DELETE /project-data/clips/:rank` 删掉整个目录，并把引用从 `publish_config.json` /
///    `clips_order.txt` 里同步摘掉（发布配置不能指向已删文件）。
#[tokio::test]
async fn clip_danmaku_count_falls_back_to_source_and_delete_prunes_index() {
    let t = test_app().await;
    let root = t.root.clone();
    // 源弹幕：0-10s 一条、10-20s 两条、20-30s 三条（ASS，走 load_danmaku_auto 的 ASS 分支）
    let mut ass = String::from("[Script Info]\nScriptType: v4.00+\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");
    for (i, sec) in [0.5f64, 10.5, 11.5, 20.5, 21.5, 22.5].iter().enumerate() {
        ass.push_str(&format!("Dialogue: 0,0:00:{:05.2},0:00:{:05.2},R2L,,0,0,0,,弹幕{i}\n", sec, sec + 1.0));
    }
    std::fs::write(root.join("dm.ass"), ass).unwrap();
    // `danmaku_file` 是 `[project]` 表里的字段（顶层写不进去，会被忽略）
    std::fs::write(root.join("yekitrim.toml"), "[project]\ninput_file = \"a.mp4\"\ndanmaku_file = \"dm.ass\"\n").unwrap();

    let clips = root.join("output").join("clips");
    std::fs::create_dir_all(&clips).unwrap();
    write_clip_dir(&clips, 1, 0.0, 10.0, 2, None);        // 无 clip.dm.srt → 数源弹幕
    write_clip_dir(&clips, 2, 10.0, 20.0, 3, None);
    write_clip_dir(&clips, 3, 20.0, 30.0, 1, None);
    write_clip_dir(&clips, 4, 30.0, 40.0, 1, Some(5));     // 有 clip.dm.srt → 以文件为准（源弹幕这里为 0）

    let res = t.call(t.req("GET", "/project-data/clips?project=1").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    let counts: std::collections::HashMap<String, u64> = v.as_array().unwrap().iter()
        .map(|c| (c["name"].as_str().unwrap_or("").to_string(), c["danmaku_count"].as_u64().unwrap_or(0)))
        .collect();
    assert_eq!(counts.get("clip-01"), Some(&1), "缺 clip.dm.srt 时应回落到源弹幕计数: {v}");
    assert_eq!(counts.get("clip-02"), Some(&2), "{v}");
    assert_eq!(counts.get("clip-03"), Some(&3), "{v}");
    assert_eq!(counts.get("clip-04"), Some(&5), "有 clip.dm.srt 时以文件为准: {v}");
    let subs: std::collections::HashMap<String, u64> = v.as_array().unwrap().iter()
        .map(|c| (c["name"].as_str().unwrap_or("").to_string(), c["subtitle_count"].as_u64().unwrap_or(0)))
        .collect();
    assert_eq!(subs.get("clip-02"), Some(&3), "{v}");

    // 发布索引里放三条，删掉中间一条后必须只剩两条
    let publish = serde_json::json!({
        "video_source": "a.mp4",
        "clips": [ { "directory": "clip-01" }, { "directory": "clip-02" }, { "directory": "clip-03" } ]
    });
    std::fs::write(root.join("output").join("publish_config.json"), serde_json::to_string_pretty(&publish).unwrap()).unwrap();
    std::fs::write(clips.join("clips_order.txt"), "clip-01 / 一\nclip-02 / 二\nclip-03 / 三").unwrap();

    let res = t.call(t.req("DELETE", "/project-data/clips/2?project=1").body(Body::empty()).unwrap()).await;
    let status = res.status();
    let j = body_json(res).await;
    assert_eq!(status, StatusCode::OK, "{j}");
    assert!(!clips.join("clip-02").exists(), "目录必须被删掉");
    assert!(clips.join("clip-01").is_dir() && clips.join("clip-03").is_dir(), "别的片段不能受影响");

    let raw = std::fs::read_to_string(root.join("output").join("publish_config.json")).unwrap();
    let pv: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let dirs: Vec<&str> = pv["clips"].as_array().unwrap().iter().filter_map(|c| c["directory"].as_str()).collect();
    assert_eq!(dirs, vec!["clip-01", "clip-03"], "{pv}");
    assert_eq!(std::fs::read_to_string(clips.join("clips_order.txt")).unwrap(), "clip-01 / 一\nclip-03 / 三");

    // 删完列表里当然也没有它了（事件会自动回到「未剪切」，这里只确认片段数据）
    let res = t.call(t.req("GET", "/project-data/clips?project=1").body(Body::empty()).unwrap()).await;
    let v = body_json(res).await;
    assert!(!v.as_array().unwrap().iter().any(|c| c["name"] == "clip-02"), "{v}");

    // 重复删 / 名字不存在：都是 404（`abc` 现在按**目录名**理解，不是非法输入）
    let res = t.call(t.req("DELETE", "/project-data/clips/2?project=1").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let res = t.call(t.req("DELETE", "/project-data/clips/abc?project=1").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "不存在的目录名 → 404");
    assert!(clips.join("clip-01").is_dir(), "不存在的名字不能误删");
}


/// **T4（同步 IO 三处补 blocking）**：`clip_delete_impl` 里的"删整个片段目录 + 重写发布索引"
/// 必须真的跑到 blocking 池上 —— 单 worker runtime 下证明它不占住 worker。
///
/// 判据与 `api::tests::blocking_helper_keeps_workers_free` 同一条思路：只留 **1 个 worker**，
/// 另起一个每 1ms 跳一次的心跳任务；**删片段期间心跳必须继续跳**。把这两段同步 IO 留在
/// worker 上（本次整改前的写法），心跳一次都跑不动（`during == 0`）—— 这正是"剪切的时候
/// 整个服务器连不上"的机制：项目常在网络盘上，一次 `remove_dir_all` 就够把 worker 占住。
///
/// fixture 是最小目录（临时目录 + 几千个几字节的小文件）：删掉它要几十毫秒，够心跳跳几十次；
/// **不依赖网络盘**，也不构造真实成片。
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn clip_delete_runs_sync_io_off_the_worker() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{sleep, Duration};

    // ---- 最小 fixture：一个待删的片段目录（内容量刚好让 remove_dir_all 有真实耗时）----
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    let clips = root.join("output").join("clips");
    let clip = clips.join("clip-01");
    std::fs::create_dir_all(&clip).unwrap();
    for i in 0..6000 {
        std::fs::write(clip.join(format!("f{i:05}.bin")), b"x").unwrap();
    }
    std::fs::write(clip.join("clip.mp4"), b"fake").unwrap();
    // 发布索引里登记它：删完这两份都得把它摘掉（`prune_clip_from_index` 也在同一个闭包里）
    std::fs::write(
        root.join("output").join("publish_config.json"),
        r#"{"video_source":"a.mp4","clips":[{"directory":"clip-01"},{"directory":"clip-02"}]}"#,
    )
    .unwrap();
    std::fs::write(clips.join("clips_order.txt"), "clip-01 / 甲\nclip-02 / 乙").unwrap();

    // `clip_delete_impl` 只需要 `AppState` 里的 JobManager（这里没有任何任务在跑）
    let auth = crate::auth::AuthService::open(tmp.path().join("auth"), crate::auth::Listening::Loopback)
        .unwrap();
    let state = Arc::new(AppState::with_auth(auth));
    let ws = Workspace {
        config_file: root.join("yekitrim.toml"),
        root_dir: root.clone(),
    };

    // ---- 心跳：只有 1 个 worker，同步 IO 若留在 worker 上它就再也跳不动 ----
    let ticks = Arc::new(AtomicUsize::new(0));
    let beats = ticks.clone();
    let beat = tokio::spawn(async move {
        loop {
            sleep(Duration::from_millis(1)).await;
            beats.fetch_add(1, Ordering::SeqCst);
        }
    });
    // 先让它真的跳起来 —— 否则"心跳 0 次"可能只是还没开始跳，什么也测不出来。
    // ⚠️ **轮询**而不是死等 5ms：并行跑整套测试时机器可能正被别的用例占满，
    //    5ms 内一次都没轮到它就会假红（2026-11-24 实测过：整套跑了 135s 的那轮）。
    let mut waited = 0u32;
    while ticks.load(Ordering::SeqCst) == 0 && waited < 2_000 {
        sleep(Duration::from_millis(5)).await;
        waited += 5;
    }
    assert!(
        ticks.load(Ordering::SeqCst) > 0,
        "心跳应当已经跳起来（等了 {waited}ms）"
    );

    let before = ticks.load(Ordering::SeqCst);
    let t0 = std::time::Instant::now();
    // ⚠️ 必须 `spawn` 上去：`#[tokio::test]` 的**测试体本身跑在 block_on 的 main 线程上**，
    // 直接在测试体里 `await` 它，被同步 IO 卡住的就是 main 线程而不是 worker ——
    // 那样无论实现对不对，心跳都在另一个线程上照跳，测不出任何东西（变异自证当场抓出来的坑）。
    let del = tokio::spawn(async move { clip_delete_impl(&ws, "clip-01", &state, 1).await });
    let res = del.await.expect("删除任务不该 panic");
    let elapsed = t0.elapsed();
    let during = ticks.load(Ordering::SeqCst) - before;
    beat.abort();

    // ---- 功能面：目录删干净了，两份索引里的引用也摘掉了 ----
    assert!(res.is_ok(), "删除片段应当成功: {:?}", res.as_ref().err());
    assert_eq!(res.unwrap().0["removed"], "clip-01");
    assert!(!clip.exists(), "整个片段目录必须被删掉");
    let publish = std::fs::read_to_string(root.join("output").join("publish_config.json")).unwrap();
    assert!(
        !publish.contains("clip-01") && publish.contains("clip-02"),
        "发布索引里不能再指向已删片段: {publish}"
    );
    assert_eq!(
        std::fs::read_to_string(clips.join("clips_order.txt")).unwrap(),
        "clip-02 / 乙",
        "clips_order.txt 里那行也要摘掉"
    );

    // ---- 判据：删除期间 worker 一直被让出来 ----
    assert!(
        elapsed.as_millis() >= 5,
        "fixture 太小（删除只用了 {elapsed:?}）：测不出 worker 是否被占，请加大文件数"
    );
    assert!(
        during >= 3,
        "删片段期间 worker 必须还能跑别的任务（心跳只跳了 {during} 次 / 删除用时 {elapsed:?}）\
         —— 同步 IO 又回到 worker 上了？"
    );
}


/// `resolve_clip_dir`：数字 rank（老契约 `2` → `clip-02`）与**目录名**（`clip-out` 这类
/// 手工命名的目录，卡片墙列的就是真实目录名）都要认；`..`/分隔符/盘符一律 400 ——
/// 这个参数直接来自 URL，拼出来的路径只能落在 `clips/` 里。
#[test]
fn resolve_clip_dir_accepts_rank_and_directory_name() {
    let td = tempfile::tempdir().unwrap();
    let out = td.path();
    let clips = out.join("clips");
    std::fs::create_dir_all(clips.join("clip-out")).unwrap();

    assert_eq!(resolve_clip_dir(out, "3").unwrap(), clips.join("clip-03"), "数字按老契约解释成 rank");
    assert_eq!(resolve_clip_dir(out, "clip-out").unwrap(), clips.join("clip-out"), "存在的目录名原样用");
    assert_eq!(resolve_clip_dir(out, "nope").unwrap(), clips.join("nope"), "不存在的名字原样返回（调用方报 404）");
    for bad in ["..", "../x", "a/b", "a\\b", "C:/x", "nul", ""] {
        let (status, _) = resolve_clip_dir(out, bad).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad:?} 必须 400");
    }
}


/// 手工命名的切片目录（`clip-out`）也要能删、能出缩略图：
/// 卡片墙列的是 `clips/` 下的**真实目录名**，名字不一定规范。
/// 以前接口只认数字 rank，前端把 `clip-out` 解析成 rank 0 → 请求 `clips/0` →
/// 稳定报「clip-00 不存在」（删除永远失败），缩略图则每次加载刷一条 400。
#[tokio::test]
async fn clip_delete_and_thumb_accept_directory_name() {
    let t = test_app().await;
    let root = t.root.clone();
    let clips = root.join("output").join("clips");
    std::fs::create_dir_all(clips.join("clip-out")).unwrap();
    std::fs::write(clips.join("clip-out").join("clip.mp4"), b"fake").unwrap();
    std::fs::write(root.join("output").join("publish_config.json"), r#"{"clips":[{"directory":"clip-out"}]}"#).unwrap();
    std::fs::write(clips.join("clips_order.txt"), "clip-out / 手工目录").unwrap();

    // 缩略图：按目录名请求，不能被当成非法 rank 拒掉（假 mp4 生成不出图 → 404 也算正常）
    let res = t.call(t.req("GET", "/project-data/clips/clip-out/thumb?project=1").body(Body::empty()).unwrap()).await;
    assert_ne!(res.status(), StatusCode::BAD_REQUEST, "目录名不该被当成非法 rank");

    let res = t.call(t.req("DELETE", "/project-data/clips/clip-out?project=1").body(Body::empty()).unwrap()).await;
    let status = res.status();
    let j = body_json(res).await;
    assert_eq!(status, StatusCode::OK, "{j}");
    assert_eq!(j["removed"], "clip-out", "{j}");
    assert!(!clips.join("clip-out").exists(), "目录必须被删掉");
    let raw = std::fs::read_to_string(root.join("output").join("publish_config.json")).unwrap();
    let pv: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(pv["clips"].as_array().unwrap().is_empty(), "索引里的引用要摘掉: {pv}");
    assert_eq!(std::fs::read_to_string(clips.join("clips_order.txt")).unwrap(), "");
}


/// 片段目录被任务占用（渲染/精修/重切）时，**同步**写操作必须当场 409 ——
/// 删除会把正在渲染的目录删掉、留下半个工程；合并会重写 events.json。
/// 任务结束后同一请求要恢复正常。
#[tokio::test]
async fn sync_writes_are_rejected_while_scope_is_busy() {
    let t = test_app().await;
    let root = t.root.clone();
    let clips = root.join("output").join("clips");
    std::fs::create_dir_all(clips.join("clip-03")).unwrap();
    std::fs::write(clips.join("clip-03").join("clip.mp4"), b"fake").unwrap();
    std::fs::write(root.join("output").join("events.json"), r#"{"events":[]}"#).unwrap();

    // 造一个占着 clip-03 的任务（真任务会立刻开跑；这里不建工作协程，
    // 它就一直是 Running，等价于"渲染进行中"）
    let busy = t
        .state
        .jobs
        .create(
            1,
            JobSpec::new(
                "polish_render",
                "渲染 clip-03",
                vec![crate::scope_key::project_clip(1, "clip-03")],
                JobClass::Heavy,
            ),
        )
        .await;
    assert!(!busy.queued, "空载时应立刻开跑");

    let res = t.call(t.req("DELETE", "/project-data/clips/clip-03?project=1").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::CONFLICT, "被占用时删除必须被拒");
    let j = body_json(res).await;
    assert!(j["error"].as_str().unwrap().contains("polish_render"), "{j}");
    assert!(clips.join("clip-03").exists(), "被拒时不能动目录");

    // 项目级资源（events.json / output/*）是另一个域：精修片段不该挡住合并事件
    let res = t.call(t.req("POST", "/project-data/events/merge-splits?project=1").body(Body::empty()).unwrap()).await;
    assert_ne!(res.status(), StatusCode::CONFLICT, "精修某片段与合并事件无关，不该互相挡");

    // 管线/剪切任务占的是 `p1:out`：那时 events.json 的同步写就必须被拒
    let pipeline = t
        .state
        .jobs
        .create(
            1,
            JobSpec::new(
                "stage_detect",
                "检测",
                vec![crate::scope_key::project_out(1)],
                JobClass::Medium,
            ),
        )
        .await;
    let res = t.call(t.req("POST", "/project-data/events/merge-splits?project=1").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::CONFLICT, "管线在跑时合并事件必须被拒");
    t.state.jobs.complete(&pipeline.id).await;

    // 任务结束后恢复正常
    t.state.jobs.complete(&busy.id).await;
    let res = t.call(t.req("DELETE", "/project-data/clips/clip-03?project=1").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(!clips.join("clip-03").exists());
}


/// `GET /project-data/clips` 要带「最后一次改动」时间（`mtime`，unix 秒）—— 首页「切片总览」
/// 按它倒序，刚剪完 / 刚精修过的片段要浮到最前面。
///
/// 取值 = 片段目录里最新的 mtime，**直接子项**（精修保存会改写 `polish.toml` / `clip.srt`）与
/// 精修数据目录 `data/` 下的文件都要算进去：只 stat 目录本身的话，改写**既有**文件不会带起
/// 目录的 mtime（只有新建/删除才会），刚精修过的片段就浮不上来。
#[tokio::test]
async fn clips_expose_last_touched_mtime() {
    let t = test_app().await;
    let clips = t.root.join("output").join("clips");
    std::fs::create_dir_all(&clips).unwrap();
    write_clip_dir(&clips, 1, 0.0, 10.0, 1, None);
    write_clip_dir(&clips, 2, 0.0, 10.0, 1, None);

    let now = std::time::SystemTime::now();
    let at = |secs: u64| now + std::time::Duration::from_secs(secs);
    let secs = |t: std::time::SystemTime| t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let touch = |p: &std::path::Path, when: std::time::SystemTime| {
        std::fs::File::options().write(true).open(p).unwrap().set_modified(when).unwrap();
    };

    // clip-01：动的是**直接子项** `polish.toml`（精修保存就改它）
    let one = clips.join("clip-01");
    std::fs::write(one.join("polish.toml"), "[source]\n").unwrap();
    touch(&one.join("polish.toml"), at(3_600));
    // clip-02：只动过 `data/` 里的文件（两层：片段目录 → data/ → 文件）
    let two_data = clips.join("clip-02").join("data");
    std::fs::create_dir_all(&two_data).unwrap();
    std::fs::write(two_data.join("subtitles.json"), "{}").unwrap();
    touch(&two_data.join("subtitles.json"), at(7_200));

    let res = t.call(t.req("GET", "/project-data/clips?project=1").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    let mtime_of = |name: &str| -> u64 {
        v.as_array().unwrap().iter()
            .find(|c| c["name"].as_str() == Some(name))
            .and_then(|c| c["mtime"].as_u64())
            .unwrap_or_else(|| panic!("{name} 没有 mtime: {v}"))
    };
    assert_eq!(mtime_of("clip-01"), secs(at(3_600)), "直接子项的 mtime 要算进来: {v}");
    assert_eq!(mtime_of("clip-02"), secs(at(7_200)), "data/ 里的改动也要算进来: {v}");
    assert!(mtime_of("clip-02") > mtime_of("clip-01"), "最后改动的片段 mtime 更大: {v}");
}


/// `GET /project-data/clips` 必须按**数字**序号返回：`clip-100` 排在 `clip-02` 之后。
/// 按字符串排会得到 clip-01, clip-02, clip-100, clip-101, …, clip-11 —— 精修页的片段条
/// 直接照这个顺序显示，用户报过"片段列表排序有问题"（只有两张卡时还隔着老远）。
#[tokio::test]
async fn clips_list_is_sorted_by_numeric_rank() {
    let t = test_app().await;
    let clips = t.root.join("output").join("clips");
    std::fs::create_dir_all(&clips).unwrap();
    for rank in [100u32, 2, 10] {
        write_clip_dir(&clips, rank, 0.0, 10.0, 1, None);
    }
    let res = t.call(t.req("GET", "/project-data/clips?project=1").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    let names: Vec<String> = v.as_array().unwrap().iter()
        .filter_map(|c| c["name"].as_str().map(|s| s.to_string()))
        .collect();
    assert_eq!(names, vec!["clip-02", "clip-10", "clip-100"], "{v}");
}


/// **主播名只进产物标题**（2026-11-22 用户口径：「主播名不参与标题以外的任何处理」）：
/// 项目级默认 ⇒ 跟随的片段自动带上；改成覆盖 ⇒ 锁定；关掉 ⇒ 不拼；清回 null ⇒ 又跟随。
/// ⚠️ 全程 `publish_config.json` 的 `title` 都是**纯标题**（切片产物里不许出现 `【名】`）——
/// 名字只在读取/投稿时合成，这正是「填错名字不用重切」的依据。
#[tokio::test]
async fn streamer_name_composes_into_the_publish_title_only() {
    let t = test_app().await;
    let root = t.root.clone();
    let clips = root.join("output").join("clips");
    std::fs::create_dir_all(&clips).unwrap();
    write_clip_dir(&clips, 1, 0.0, 10.0, 1, None);
    std::fs::write(
        root.join("output").join("publish_config.json"),
        r#"{"video_source":"a.mp4","clips":[{"directory":"clip-01","title":"标题"}]}"#,
    )
    .unwrap();
    // 项目级主播名（跟着本工程走，不写程序级配置）
    let cfg = root.join("yekitrim.toml");
    if !cfg.exists() {
        std::fs::write(&cfg, "[project]\ninput_file = \"a.mp4\"\n").unwrap();
    }
    yekitrim_core::config::patch_config_file(
        &cfg,
        &serde_json::json!({ "detection": { "vtuber_name": "主播" } }),
    )
    .unwrap();

    let list = || {
        let t = &t;
        async move {
            let res = t
                .call(t.req("GET", "/project-data/clips?project=1").body(Body::empty()).unwrap())
                .await;
            body_json(res).await
        }
    };
    let put = |body: serde_json::Value| {
        let t = &t;
        async move {
            let res = t
                .call(
                    t.req("PUT", "/project-data/clips/clip-01/title?project=1")
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await;
            (res.status(), body_json(res).await)
        }
    };
    let title_on_disk = || {
        let raw = std::fs::read_to_string(root.join("output").join("publish_config.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        v["clips"][0]["title"].as_str().unwrap_or("").to_string()
    };

    // ① 没动过 ⇒ 跟随项目级
    let v = list().await;
    assert_eq!(v[0]["publish_title"], "标题【主播】", "{v}");
    assert_eq!(v[0]["publish_title_core"], "标题", "只读标题框里不该出现名字：{v}");
    assert_eq!(v[0]["streamer_name"], serde_json::Value::Null, "从没改过 = 跟随（null）: {v}");

    // ② 改成覆盖 ⇒ 锁定成这个名字，磁盘上仍然是纯标题
    let (st, j) = put(serde_json::json!({ "title": "标题", "streamer_name": "片段名" })).await;
    assert_eq!(st, StatusCode::OK, "{j}");
    assert_eq!(title_on_disk(), "标题", "切片产物里不许写名字");
    let v = list().await;
    assert_eq!(v[0]["publish_title"], "标题【片段名】", "{v}");
    assert_eq!(v[0]["streamer_name"], "片段名", "改过 = 锁定: {v}");

    // ③ 改项目级名字 ⇒ 锁定的那条**不跟随**
    yekitrim_core::config::patch_config_file(
        &cfg,
        &serde_json::json!({ "detection": { "vtuber_name": "新主播" } }),
    )
    .unwrap();
    let v = list().await;
    assert_eq!(v[0]["publish_title"], "标题【片段名】", "锁定过的不许被全局改名带走: {v}");

    // ④ 关掉（空串）⇒ 不拼，且**也不跟随**
    let (st, _) = put(serde_json::json!({ "title": "标题", "streamer_name": "" })).await;
    assert_eq!(st, StatusCode::OK);
    let v = list().await;
    assert_eq!(v[0]["publish_title"], "标题", "{v}");

    // ⑤ 清回 null ⇒ 恢复跟随（吃到新的项目级名字）
    let (st, _) = put(serde_json::json!({ "title": "标题", "streamer_name": serde_json::Value::Null })).await;
    assert_eq!(st, StatusCode::OK);
    let v = list().await;
    assert_eq!(v[0]["publish_title"], "标题【新主播】", "{v}");
    assert_eq!(title_on_disk(), "标题");
}


/// 发布标题（2026-11-14 用户要求「产物那里加个标题，方便复制也方便改」）：
/// 写的是 `publish_config.json` 的 `clips[].title`（投稿链路读那份），**绝不动 `clip.json` 的
/// title** —— 后者是片段身份（`clip_meta_matches` 拿它判断重新检测能不能复用目录），改了会让
/// 老片段另开一个目录、精修产物留在旧目录里（用户看到的是「精修白做了」）。
/// 另外两件事一起钉住：`clips_order.txt` 那行标题跟着改；`/clips` 回包要回**改后**的
/// `publish_title`（页面刷新后靠它回填，否则用户会以为没保存）。
#[tokio::test]
async fn clip_title_writes_publish_config_and_leaves_clip_json_alone() {
    let t = test_app().await;
    let root = t.root.clone();
    let clips = root.join("output").join("clips");
    std::fs::create_dir_all(&clips).unwrap();
    write_clip_dir(&clips, 1, 0.0, 10.0, 1, None);
    std::fs::write(
        root.join("output").join("publish_config.json"),
        r#"{"video_source":"a.mp4","clips":[{"directory":"clip-01","title":"旧标题"}]}"#,
    )
    .unwrap();
    std::fs::write(clips.join("clips_order.txt"), "clip-01 旧标题").unwrap();

    let put = |uri: &str, body: serde_json::Value| {
        t.req("PUT", uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };

    // 首尾空白要去掉（文本框里手滑敲个空格不该原样落盘）
    let res = t
        .call(put(
            "/project-data/clips/clip-01/title?project=1",
            serde_json::json!({ "title": "  新标题（改过）  " }),
        ))
        .await;
    let status = res.status();
    let j = body_json(res).await;
    assert_eq!(status, StatusCode::OK, "{j}");
    assert_eq!(j["title"], "新标题（改过）", "{j}");

    let pv: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("output").join("publish_config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(pv["clips"][0]["title"], "新标题（改过）", "{pv}");
    let meta: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(clips.join("clip-01").join("clip.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(meta["title"], "片段1", "clip.json 的标题是片段身份，不许被改: {meta}");
    assert_eq!(
        std::fs::read_to_string(clips.join("clips_order.txt")).unwrap(),
        "clip-01 / 新标题（改过）",
        "clips_order.txt 那行的标题要跟着改"
    );

    // 列表接口回改后的那句（页面刷新靠它回填）
    let res = t
        .call(t.req("GET", "/project-data/clips?project=1").body(Body::empty()).unwrap())
        .await;
    let v = body_json(res).await;
    assert_eq!(v[0]["publish_title"], "新标题（改过）", "{v}");

    // 不存在的片段 → 404；没有发布索引 → 404（不许假装成功）；超长 → 400
    let res = t
        .call(put("/project-data/clips/nope/title?project=1", serde_json::json!({"title":"x"})))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    std::fs::remove_file(root.join("output").join("publish_config.json")).unwrap();
    let res = t
        .call(put("/project-data/clips/clip-01/title?project=1", serde_json::json!({"title":"x"})))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "没有 publish_config 时不许假装成功");
    let long = "长".repeat(MAX_TITLE_CHARS + 1);
    let res = t
        .call(put("/project-data/clips/clip-01/title?project=1", serde_json::json!({"title": long})))
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "超长标题必须 400");
}


/// **T9**：`ClipTitleBody`（`PUT /project-data/clips/:rank/title`）。
///
/// `title` 是**必填**（缺了它这个接口没有任何意义）；`streamer_name` 的三态由 handler 判，
/// 结构体只负责"它得是个 JSON 值"—— 传数字仍然 400，且文案点名它。
#[tokio::test]
async fn clip_title_body_is_typed_and_names_the_field() {
    let t = test_app().await;
    let uri = "/project-data/clips/clip-01/title?project=1";

    let (status, text) = call_json(&t, "PUT", uri, serde_json::json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "缺 title 必须 400：{text}");
    assert!(text.contains("title"), "报错要点名 title：{text}");

    let (status, text) = call_json(&t, "PUT", uri, serde_json::json!({ "title": 5 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "title 类型错必须 400：{text}");
    assert!(text.contains("title"), "报错要点名 title：{text}");

    // 三态之外的写法（数字/null 之外的类型）仍由 handler 明确拒绝，文案点名 streamer_name
    let (status, text) = call_json(
        &t,
        "PUT",
        uri,
        serde_json::json!({ "title": "x", "streamer_name": 5 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "streamer_name 不是字符串/null 必须 400：{text}");
    assert!(text.contains("streamer_name"), "报错要点名 streamer_name：{text}");
}

}