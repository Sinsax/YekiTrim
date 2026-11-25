//! `api::polish::project`：精修工程本体（`polish.toml`）、字幕与情绪轨道。

use std::path::Path;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;

use crate::model::AppState;

use super::*;


/// 读精修字幕（`data/subtitles.json` 或 `clip.srt`）：项目目录上的同步 IO，下 blocking 池。
pub(crate) async fn polish_subtitles_get(State(state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String,String>>) -> ApiResult {
    blocking(move || block_on_here(polish_subtitles_get_impl(State(state), Query(q)))).await
}


pub(super) async fn polish_subtitles_get_impl(
    State(_state): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String,String>>,
) -> ApiResult {
    let dir = q.get("dir").ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !{
        let __p = path.join("polish.toml");
        // 同步 stat 不能留在 async worker 上（api/mod.rs:44-51 ✓）：
        // 工程常在慢盘/网络盘上，一次 exists() 就要几百 ms ✗
        tokio::task::spawn_blocking(move || __p.exists())
            .await
            .unwrap_or(false)
    } {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    let json_path = path.join("data").join("subtitles.json");
    if json_path.exists() {
        let raw = std::fs::read_to_string(&json_path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
        let segments = if let Some(arr) = v.as_array() {
            arr.clone()
        } else if let Some(arr) = v.get("segments").and_then(|x| x.as_array()) {
            arr.clone()
        } else {
            Vec::new()
        };
        return ok(serde_json::json!({ "segments": segments }));
    }
    let srt_path = path.join("clip.srt");
    if srt_path.exists() {
        let raw = std::fs::read_to_string(&srt_path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
        let t = yekitrim_core::srt::parse_srt(&raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析失败: {e}")))?;
        return ok(serde_json::json!({ "segments": t.segments }));
    }
    Err(err(StatusCode::NOT_FOUND, "暂无字幕文件"))
}


/// **§10 情绪轨道** —— 把 `process` 算出来的 `data/emotion.json` 交给界面。
///
/// # ⚠️ 这个端点存在的唯一理由：`emotion.json` 以前**一个读者都没有**
///
/// `process` 早就把情绪轨道算出来并写盘了（`process/mod.rs::write_emotion_track`），
/// 但 2026-10-12 那次全仓核查的结论是：**只有写者、没有任何读它的代码** ——
/// 演出脚本、LLM 提示、界面三条都没接。⇒ 它是**一个"算出真数据、然后扔掉"的产物**。
///
/// 这个端点 + 前端那条轨道就是补上的**读者**。
///
/// ⚠️ **404 是正常空态，不是错误**：`emotion.json` 只在"有弹幕且判出了窗口"时才写
/// （调用方用 `if !track.is_empty()` 把着，口径与 `punches.json` 一致）。
/// 没有弹幕的工程本来就不该有情绪轨道 —— 前端按空态画一条淡提示，**不许报错**。
///
/// ⚠️ 路径同样过 `validated_project_dir`（路径面三防线之一），
/// 且整段是同步 IO ⇒ 必须 `blocking`（慢盘上一次 `exists()` 就是几百 ms）。
pub(crate) async fn polish_emotion_get(
    State(state): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult {
    blocking(move || block_on_here(polish_emotion_get_impl(State(state), Query(q)))).await
}


pub(super) async fn polish_emotion_get_impl(
    State(_state): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult {
    let dir = q.get("dir").ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    let json_path = path.join("data").join("emotion.json");
    // 同步 stat 不能留在 async worker 上（api/mod.rs 的纪律，与上面 subtitles 同一条）
    let exists = {
        let p = json_path.clone();
        tokio::task::spawn_blocking(move || p.exists()).await.unwrap_or(false)
    };
    if !exists {
        // ⚠️ 空态用 404 而不是 `{windows:[]}`：让"没有这个产物"与"有但是空的"
        //    在协议上就分得开 —— 前端才不会把"没跑过 process"画成"跑了但没判出情绪"。
        return Err(err(StatusCode::NOT_FOUND, "暂无情绪轨道（该工程没跑过 process，或没有弹幕）"));
    }
    let raw = std::fs::read_to_string(&json_path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    // ⚠️ 解析失败要**报出来**（别 `unwrap_or(Null)` 静默吞掉）——
    //    这是本仓自己的产物，坏了属于内部不一致，装看不见只会让它更难查。
    let v: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("情绪轨道解析失败: {e}")))?;
    ok(serde_json::json!({
        "window_secs": v.get("window_secs").cloned().unwrap_or(serde_json::Value::Null),
        "windows": v.get("windows").cloned().unwrap_or_else(|| serde_json::json!([])),
    }))
}


pub(crate) async fn polish_subtitles_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    // ⚠️ 整段都是同步 IO：`validated_project_dir` 的 canonicalize + 建 data/ + 写两份文件。
    // GET 孪生（polish_subtitles_get）早就这么包了，PUT 这一半漏了。
    blocking(move || polish_subtitles_put_impl(body)).await
}


fn polish_subtitles_put_impl(body: serde_json::Value) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    let segments = body.get("segments").cloned().ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 segments"))?;
    let mut segs: Vec<yekitrim_core::types::TranscriptSegment> = segments.as_array()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "segments 必须是数组"))?
        .iter().map(|v| {
            let start = v.get("start").and_then(|x| x.as_f64()).unwrap_or(0.0);
            let end = v.get("end").and_then(|x| x.as_f64()).unwrap_or(start + 1.0);
            let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let confidence = v.get("confidence").and_then(|x| x.as_f64()).unwrap_or(1.0);
            yekitrim_core::types::TranscriptSegment { start, end, text, confidence }
        }).collect();
    // **按开始时间排序后再落盘**：字幕轨道必须有序 —— 去静音按相邻间隔算静音、
    // 预览按时间二分取当前字幕都依赖它。编辑器「拆分」曾经把后半段追加到数组末尾
    // （`TimelineEditor.splitSelected`，已修），存过的那种文件是乱序的；
    // 这里顺手归一化，用户再存一次就自愈（老数据不用手工修）。
    segs.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    let data_dir = path.join("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?;
    // 写回 data/subtitles.json，保持 polish 使用的列表结构
    let out: Vec<serde_json::Value> = segs.iter().enumerate().map(|(i, seg)| {
        serde_json::json!({
            "index": i + 1,
            "start": seg.start,
            "end": seg.end,
            "text": seg.text,
            "html": seg.text,
        })
    }).collect();
    std::fs::write(data_dir.join("subtitles.json"), serde_json::to_string_pretty(&out).unwrap_or_default())
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
    // 同步写回 clip.srt，方便外部工具读取
    let transcript = yekitrim_core::types::Transcript {
        segments: segs,
        language: body.get("language").and_then(|l| l.as_str()).unwrap_or("zh").to_string(),
        asr_metadata: None,
    };
    yekitrim_core::srt::write_srt(&transcript, &path.join("clip.srt"))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写 SRT 失败: {e}")))?;
    ok(serde_json::json!({ "ok": true, "saved": out.len() }))
}


/// 响度曲线（带缓存）：同一源视频只测一次，结果存 `data/loudness.json`。
///
/// 实现已收敛到 `yekitrim_polish::render::measure_loudness_curve_cached`（出片页与混音共用同一份）。
pub(super) fn loudness_curve_cached(project_dir: &Path, video_path: &Path) -> serde_json::Value {
    let curve = yekitrim_polish::render::measure_loudness_curve_cached(project_dir, video_path);
    let points: Vec<serde_json::Value> = curve
        .iter()
        .map(|(t, db)| serde_json::json!({ "t": t, "db": db }))
        .collect();
    serde_json::json!(points)
}


/// 读一个精修工程（`polish.toml` + 贴纸映射 + 探测视频尺寸 + 响度曲线）——
/// 全是同步 IO / ffprobe，工程又在项目目录里（可能网络盘）。整段下 blocking 池：
/// 精修页每次加载与每次保存后都会打这个接口。
pub(crate) async fn polish_project_get(State(state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String,String>>) -> ApiResult {
    blocking(move || block_on_here(polish_project_get_impl(State(state), Query(q)))).await
}


/// 读一份**可选**的 `data/` 侧车 JSON（缺失 / 坏了都回 `null`，绝不报错）。
///
/// ⚠️ **坏文件按"没有"处理而不是 500**：这些报告是 `process` 写的**派生数据**，
/// 不是工程本身。回 500 会让整个精修页读不出配置 —— 用户为了一个统计数字
/// **连编辑器都打不开**，代价完全不成比例。与 `/polish/video-thumb` "抽不出帧回 404
/// 不是 500"是同一条纪律。
fn read_data_json(project_dir: &std::path::Path, name: &str) -> serde_json::Value {
    let p = project_dir.join("data").join(name);
    match std::fs::read_to_string(&p) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null),
        Err(_) => serde_json::Value::Null,
    }
}


pub(super) async fn polish_project_get_impl(
    State(_state): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String,String>>,
) -> ApiResult {
    let dir = q.get("dir").ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    let cfg = yekitrim_polish::project::load_project(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    let mut value = serde_json::to_value(&cfg).unwrap_or_default();
    // preview=1: 附加预览渲染所需的计算字段（sticker_map / 竖屏检测 / 响度曲线）
    if q.get("preview").map(|v| v == "1").unwrap_or(false) {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("_assets".into(), serde_json::json!({
                "sticker_map": yekitrim_polish::render::server::build_sticker_map(&path),
            }));
            // §23 节奏报告（`data/pacing.json`，2026-10-13）。
            //
            // ⚠️ **只读回、绝不落盘 / 不改配置**：规格自己强调过
            //    「留白是'没有说话'，不是'画面死着'」—— 自动往安静叙述切片上补运镜是
            //    **改观感**的行为，得用户先看数字再拍板。那份 JSON 里的 `applied: false`
            //    就是给读它的人一个明确交代，**别在这里把它改成 true**。
            //
            // ⚠️ 没有这个文件 = **本来就很密**（`process` 只在扫出空档时才落，
            //    见 `process::write_pacing_report` 的 R4 说明）⇒ 回 `null`，
            //    而不是空对象：两者语义不同（"没算过/很密" vs "算过、有 0 段"）。
            obj.insert("_pacing".into(), read_data_json(&path, "pacing.json"));
            let video_path = path.join(&cfg.source.video);
            // 画布契约与出片读的是**同一份推导**（`ProjectFile::canvas_contract`）。
            // 这里以前自己写了一份「竖屏检测」，与 `render/server.rs` 各一份 —— 改画布方向会不同步。
            let src_size = yekitrim_polish::render::get_video_size(&video_path.to_string_lossy());
            let contract = cfg.canvas_contract(src_size);
            obj.insert(
                "vertical".into(),
                serde_json::json!(src_size.map(|(w, h)| h > w).unwrap_or(false)),
            );
            // 六条轨道的显示开关（时间轴那六枚眼睛）：与出片模板/转译器**同一个判据**
            // （`ProjectFile::lane_enabled`，缺字段按素材+画布方向取默认）。
            obj.insert("_lanes".into(), cfg.lanes_json(src_size));
            obj.insert(
                "vertical_position".into(),
                contract["stage"]["position"].clone(),
            );
            obj.insert("_canvas".into(), contract);
            // **预览该读哪份视频**（2026-11-21，局域网提速）：有新鲜的 720p 代理就把它的文件名交出去，
            // 前端 DOM 那条腿据此改读代理（底座那条腿早就经转译器 `--video` 拿到它了）。
            // 判定只此一处（`proxy::preview_video_for`：代理缺失 / 比源旧 / `VTEDIT_NO_PROXY=1` ⇒ `None`）。
            // ⚠️ 出片**永远**用源 —— 这里只是给预览挑一份低码率副本。
            obj.insert(
                "_preview_video".into(),
                match yekitrim_polish::render::proxy::preview_video_for(&path) {
                    Some(rel) => serde_json::json!(rel),
                    None => serde_json::Value::Null,
                },
            );
            // 最近一次 LLM 生成的结果（哪段成/哪段失败），编辑器按它给分段上警示色。
            // 只是展示用：读不到就当没有，绝不影响加载。
            if let Some(st) = yekitrim_polish::llm::status::load(&path) {
                obj.insert(
                    "_llm_status".into(),
                    serde_json::to_value(st).unwrap_or(serde_json::Value::Null),
                );
            }
            let res = { let __p = video_path.clone(); tokio::task::spawn_blocking(move || __p.exists()).await.unwrap_or(false) }; if res {
                obj.insert("loudness_curve".into(), loudness_curve_cached(&path, &video_path));
            }
        }
    }
    ok(value)
}


pub(crate) async fn polish_project_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    // polish.toml 的 read-modify-write：时间轴保存 / 屏蔽区间 / 贴纸配置可能同时提交，
    // 无锁时后写者会覆盖先写者（前端确实会并发发多个 PUT）。
    // 锁在 await 里持有，读改写下 blocking 池
    let _guard = yekitrim_core::filelock::lock_path(&path.join("polish.toml")).await;
    blocking(move || polish_project_put_locked(&path, &body)).await
}


/// 同步实现（调用方保证持有 polish.toml 的进程内锁）。
pub(super) fn polish_project_put_locked(dir: &Path, body: &serde_json::Value) -> ApiResult {
    let mut cfg = yekitrim_polish::project::load_project(dir).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    if let Some(segments) = body.get("segments") {
        // 老的第三方调用方 / 手写 JSON 里的 `duration` 在这里升级成 `end`（与 polish.toml 同一条路）
        let mut up = segments.clone();
        yekitrim_polish::project::upgrade_event_times(&mut up);
        let segs = serde_json::from_value(up).map_err(|e| err(StatusCode::BAD_REQUEST, format!("segments 解析失败: {e}")))?;
        cfg.segments = segs;
    }
    if let Some(ranges) = body.get("mute_ranges") {
        let rs = serde_json::from_value(ranges.clone()).map_err(|e| err(StatusCode::BAD_REQUEST, format!("mute_ranges 解析失败: {e}")))?;
        cfg.mute_ranges = rs;
    }
    if let Some(layout) = body.get("layout") {
        let lay: yekitrim_polish::project::Layout = serde_json::from_value(layout.clone()).map_err(|e| err(StatusCode::BAD_REQUEST, format!("layout 解析失败: {e}")))?;
        cfg.layout = lay;
    }
    if let Some(ss) = body.get("sticker_source") {
        let list: Vec<String> = serde_json::from_value(ss.clone()).map_err(|e| err(StatusCode::BAD_REQUEST, format!("sticker_source 解析失败: {e}")))?;
        cfg.sticker_source = list;
    }
    // 字幕字体：值 = 字体库里的 id（文件名去扩展名），空串 = 回到系统字体栈。
    // 只影响渲染与预览（process 的贴纸/运镜重算不读它），所以**保存即生效、不用重跑任务**。
    if let Some(f) = body.get("subtitle_font").and_then(|v| v.as_str()) {
        cfg.style.subtitle.font = f.trim().to_string();
    }
    // ── 字幕样式（2026-10）：颜色 / 描边 / 字号 / 垂直落点 ──────────────────────
    //
    // 与 `subtitle_font` 同一条路：**保存即生效、不用重跑任务**（只影响渲染与预览）。
    // ⚠️ 这四条以前**不在白名单里** ⇒ 前端发过来被静默丢掉，而 PUT 仍回 `{"ok":true}`
    //    （正是下面那段注释说的「没列进来的键保存时不报错、但静默存不进去」；
    //    实测：发出 4 个键，`polish.toml` 一个字都没变、控件随即回弹到原值）。
    // 四条都**只认 body 里真的出现过的键**：键缺席 = 不动磁盘 —— 这是老工程逐字节零回归（R4）的前提。
    if let Some(c) = body.get("subtitle_color").and_then(|v| v.as_str()) {
        let c = c.trim();
        // 坏颜色**不写**（写进去会让封面与预览各自净化成不同的兜底色 —— 静默变色）。
        // 只做形态检查；真正的净化仍在消费侧（`sanitize_hex_color` 那份不许在这里抄第二份）。
        if c.starts_with('#') && c.len() > 1 {
            cfg.style.subtitle.color = c.to_string();
        }
    }
    if let Some(s) = body.get("subtitle_stroke").and_then(|v| v.as_str()) {
        // 格式 `"{宽度}px {颜色}"`（与 `SubtitleStyle::default_sub_stroke` 同形）。
        // 转译器按这个形状解析，**写歪了描边会整条回落到默认值**且不报错，所以宁可拒收。
        let s = s.trim();
        if !s.is_empty() {
            cfg.style.subtitle.stroke = s.to_string();
        }
    }
    if let Some(s) = body.get("subtitle_outer_stroke").and_then(|v| v.as_str()) {
        // **第二层描边（白环）**（2026-10-16）：三种写法 ——
        //   · `"auto #ffffff"` = 粗细**跟随第一层描边**（缺键时的默认，也是开关"开"写下去的值）；
        //   · `"8px #ffffff"` = 固定宽度（老工程与手写的值）；
        //   · **空串 = 显式关** —— ⚠️ 与 `subtitle_stroke` 不同，空串在这里是**合法值**
        //     （面板的开关"关"写的就是它），所以这一支**不做非空判断**。
        // ⚠️ 认不出的形状**照样写进去**：转译器对认不出的值回落到默认（`auto`）并把它写进
        //    转译报告，比"悄悄拒收"好 —— 拒收会让开关按下去没反应，而前端以为存上了。
        cfg.style.subtitle.outer_stroke = s.trim().to_string();
    }
    if let Some(v) = body.get("subtitle_size") {
        // 字号（px）：范围收在 8~400（与模板/转译器都对得上），**坏值当场报错、不静默**。
        let n = v
            .as_u64()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "subtitle_size 必须是整数"))?;
        cfg.style.subtitle.size = n.clamp(8, 400) as u32;
    }
    if let Some(v) = body.get("subtitle_position_pct") {
        // 垂直落点 = **文字带底边**距画布底的百分比（0~50）。
        //
        // ⚠️ `null` 与「键缺席」**语义不同**，必须分辨（本字段唯一的设计要点）：
        //   · 键缺席 → 不动磁盘（老工程保持"没写过"，走历史公式）；
        //   · `null`  → **显式恢复"未设置"**（`Some → None`）。
        //     这是「位置 → 默认」那颗按钮的**唯一通路** —— 少了这一支，用户就再也回不到未设置。
        if v.is_null() {
            cfg.style.subtitle.position_pct = None;
        } else {
            let n = v.as_f64().ok_or_else(|| {
                err(StatusCode::BAD_REQUEST, "subtitle_position_pct 必须是数字或 null")
            })?;
            if !n.is_finite() {
                return Err(err(StatusCode::BAD_REQUEST, "subtitle_position_pct 必须是有限数"));
            }
            cfg.style.subtitle.position_pct = Some(n.clamp(0.0, 50.0));
        }
    }
    // 弹幕字体：同 `subtitle_font`（值 = 字体库 id，空串 = 跟随字幕那条字体栈）。
    // ⚠️ 它还决定弹幕的**丈量宽度**（滚动时长按宽度算），所以三处量宽都要读它。
    if let Some(f) = body.get("danmaku_font").and_then(|v| v.as_str()) {
        cfg.style.danmaku.font = f.trim().to_string();
    }
    // 弹幕时间偏移（秒，负 = 提前）：精修工程参数，只影响弹幕显示时刻（三条消费腿各自
    // 在入口平移：出片模板 DM 注入点 / engine.js::buildIndex / 转译器 remap 之前），
    // 所以**保存即生效、不用重跑任务**。范围收在 ±10s，坏值当场报错不静默。
    if let Some(v) = body.get("danmaku_offset_secs") {
        let raw = v
            .as_f64()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "danmaku_offset_secs 必须是数字"))?;
        if !raw.is_finite() {
            return Err(err(StatusCode::BAD_REQUEST, "danmaku_offset_secs 必须是有限数字"));
        }
        cfg.style.danmaku.offset_secs = raw.clamp(-10.0, 10.0);
    }
    // 输出画布 / 画面构成 / 贴纸落点：整块提交（前端给全量字段，缺键走 serde 默认）。
    // ⚠️ 这里是**逐字段白名单**：没列进来的键保存时不报错、但静默存不进去。
    // `sticker` 只落点策略（来源仍是根级 `sticker_source`，见上一段）。
    if let Some(v) = body.get("canvas") {
        cfg.canvas = serde_json::from_value(v.clone())
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("canvas 解析失败: {e}")))?;
    }
    if let Some(v) = body.get("stage") {
        cfg.stage = serde_json::from_value(v.clone())
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("stage 解析失败: {e}")))?;
    }
    if let Some(v) = body.get("sticker") {
        cfg.sticker = serde_json::from_value(v.clone())
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("sticker 解析失败: {e}")))?;
    }
    // 音效块（`[sfx]`：全局默认音量等）—— 音效轨的设置面板提交它。
    if let Some(v) = body.get("sfx") {
        cfg.sfx = serde_json::from_value(v.clone())
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("sfx 解析失败: {e}")))?;
    }
    // 成片时间线（`[edl]`：重放 / 冷开场 / 回忆回溯）—— 第 3 批的成片轨提交它。
    //
    // ⚠️ **漏了这一支就是"保存不报错、值静默丢掉"**（见本函数上面那段注释）：界面上
    // 成片轨改完显示"已保存"、PUT 照回 `{"ok":true}`，而 `polish.toml` 一个字节没变。
    // 第 3 批实测过：前端提交 `{edl:{clips:[…]}}`，白名单里没有 `edl` ⇒ 探针单测当场红。
    // ⚠️ 空 `Edl`（`clips` 为空）**不写盘**是 `ProjectFile.edl` 字段上那个
    // `skip_serializing_if = "Edl::is_empty"` 负责的 —— 那正是老工程重存逐字节不变（R4）的前提，
    // 所以这里**照收**即可，别在这里特判"空的就别赋值"（那会让"清空 EDL"这个操作失灵）。
    if let Some(v) = body.get("edl") {
        cfg.edl = serde_json::from_value(v.clone())
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("edl 解析失败: {e}")))?;
    }
    // 文字元素动效（`[elements]`：打字机 / 弹入 / 抖动 / 底衬 / 摇摆）。
    //
    // ⚠️ **漏了这一支就是"保存不报错、值静默丢掉"**（同上那段注释警告的那类）：
    //    界面上改完显示"已保存"、PUT 照回 `{"ok":true}`，而 `polish.toml` 一个字节没变。
    //
    // ⚠️ 与 `edl` 那一支**同形**：空配置**不写盘**由 `ProjectFile.elements` 字段上的
    //    `skip_serializing_if = "ElementsConfig::is_empty"` 负责，所以这里**照收**即可
    //    —— 别在这里特判"空的就别赋值"（那会让"清空动效"这个操作失灵）。
    //
    // ⚠️ 反序列化即校验：`ElementsConfig` 是 `deny_unknown_fields` ⇒
    //    前端写错键名会**当场 400**，而不是静默存下一个没人读的键。
    if let Some(v) = body.get("elements") {
        cfg.elements = serde_json::from_value(v.clone())
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("elements 解析失败: {e}")))?;
    }
    // 运镜档位缩放（`[style.camera]`，总方案 §18-C：**只暴露配置、不改默认**）。
    //
    // ⚠️ **漏了这一支就是"保存不报错、值静默丢掉"**（就是上面那段注释警告的那类）：
    // 界面上改完显示"已保存"、PUT 照回 `{"ok":true}`，而 `polish.toml` 一个字节没变。
    //
    // ⚠️ **这里是"档位表"的第 6 个消费点**。另外 4 处（`render-core::CameraStyle::factor()`、
    // 模板 canvas、`engine.js`、转译器）都从**同一份配置**读，所以这里只要把值存进去就自动一致；
    // 也正因如此，**存进去的是一个会被四条渲染路径直接采信的数**，必须在这里夹住：
    //   * `zoom_mid` / `zoom_close` ∈ [1.0, 3.0] —— 下限 1.0 = "不许比原始画面更小"
    //     （`< 1` 会缩出黑边，那是 `zoom_wide` 的语义，写在中景上必是手滑）；
    //   * `zoom_wide` ∈ [0.05, 1.0] —— 它就是画中画，**大于 1 就不再是画中画**；
    //     下限取正数：0 会让画面缩成一个点。
    // 上限 3.0 的依据是规格 §0.6 / §3.2 的**露边**实测（系数越大、平移时露出的画面外区域越多，
    // 实测 `scale=1.3` 已有 622px 暴露面）—— 3.0 是"还能靠夹取兜住"的范围，再大就该走
    // 基线守卫而不是配置。**别把上限放开**：放开的症状是"出片时画面边缘出现黑/糊边"，
    // 而那时用户已经看不出是哪个值的问题。
    if let Some(v) = body.get("camera") {
        let mut cam: yekitrim_polish::project::CameraStyle = serde_json::from_value(v.clone())
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("camera 解析失败: {e}")))?;
        for (name, value, lo, hi) in [
            ("zoom_mid", &mut cam.zoom_mid, 1.0_f64, 3.0_f64),
            // 特写（§18-A / 规格 §3.3 方案 A）：与中/近同区间 —— 它同样是"放大"，
            // < 1 会缩出黑边（那是画中画的语义）。⚠️ 三处必须同批：
            // 这里、`webui/src/views/PolishView.vue` 的滑杆、以及门禁的期望表。
            ("zoom_closeup", &mut cam.zoom_closeup, 1.0, 3.0),
            ("zoom_close", &mut cam.zoom_close, 1.0, 3.0),
            // 「全」（原画不缩放，缺省档，2026-10-20）：与中/近同区间（< 1 会缩出黑边 = 画中画语义）。
            ("zoom_full", &mut cam.zoom_full, 1.0, 3.0),
            ("zoom_wide", &mut cam.zoom_wide, 0.05, 1.0),
        ] {
            if !value.is_finite() {
                return Err(err(StatusCode::BAD_REQUEST, format!("{name} 必须是有限数")));
            }
            // 夹取而不是报错：滑杆拖到边界是正常操作。但**存的是夹取后的值** ——
            // 界面提交 5.0、配置里落 3.0，用户下次打开看到的就是真实生效的数。
            *value = value.clamp(lo, hi);
        }
        cfg.style.camera = cam;
    }
    // ── BGM（§11）：把已实现的能力接上界面入口 ────────────────────────────────
    //
    // ⚠️ 这一段补的是一个**"实现了但没人能打开"**的洞：
    //   `render/bgm.rs` 的 BgmConfig / `bgm_filter_chain` / `duck_gain_expr` 全都写好了、
    //   出片链路（`render/ffmpeg.rs`、`render/pipeline.rs`）也真的调它们，
    //   但**写配置的入口一条都没有** —— 本函数是逐字段白名单，`bgm` 不在里面，
    //   于是前端（乃至手写的 API 调用）发过来会被**静默丢掉**，而 PUT 照旧回 `{"ok":true}`
    //   （正是下面那段注释说的那个形状）。用户**没有任何办法**打开 BGM。
    //
    // ⚠️ **只认 body 里真的出现过的键**：键缺席 = 不动磁盘 —— R4 的前提
    //   （老工程 `polish.toml` 里没有 `[bgm]`，本条不许把它写出来）。
    //   `BgmConfig::is_empty()` ⇔ `!enabled && file.is_empty()` 配 `skip_serializing_if`
    //   决定了"关着且没选文件"就**不写这个键** ⇒ 只看不点开关的老工程逐字节不变。
    if let Some(bgm) = body.get("bgm") {
        let b = bgm
            .as_object()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "bgm 必须是对象"))?;
        // 逐键覆盖：只认出现过的键，缺席的保持原值。
        if let Some(v) = b.get("enabled") {
            cfg.bgm.enabled = v
                .as_bool()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "bgm.enabled 必须是布尔"))?;
        }
        if let Some(v) = b.get("file").and_then(|v| v.as_str()) {
            // ⚠️ **相对路径**：与前缀面同一套口径（`scope.rs::is_safe_project_relative`，
            //    唯一一份）。BGM 文件是"工程目录里的一个文件"，不是服务器任意路径 ——
            //    这里是第二道兜底，第一道在 scope 中间件。
            let f = v.trim();
            if !f.is_empty() && !crate::scope::is_safe_project_relative(f) {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    "bgm.file 必须是工程内的相对路径（不许绝对路径 / 不许 ..）",
                ));
            }
            cfg.bgm.file = f.to_string();
        }
        // 数值三项：坏值**当场报错、不静默**（写进去会让出片按一个没人预期的音量混合）。
        if let Some(v) = b.get("volume") {
            let n = v
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "bgm.volume 必须是有限数"))?;
            cfg.bgm.volume = n.clamp(0.0, 1.0);
        }
        if let Some(v) = b.get("duck_ratio") {
            let n = v
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "bgm.duck_ratio 必须是有限数"))?;
            cfg.bgm.duck_ratio = n.clamp(0.0, 1.0);
        }
        if let Some(v) = b.get("fade_in") {
            let n = v
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "bgm.fade_in 必须是有限数"))?;
            // 上限 30s：再长就不是"淡入"而是"整片都没起来"了（滑杆拖到边界是正常操作，
            // 所以夹取而不是报错 —— 与 `[style.camera]` 那段的处置一致）。
            cfg.bgm.fade_in = n.clamp(0.0, 30.0);
        }
        if let Some(v) = b.get("fade_out") {
            let n = v
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "bgm.fade_out 必须是有限数"))?;
            cfg.bgm.fade_out = n.clamp(0.0, 30.0);
        }
        if let Some(v) = b.get("loop_until_end") {
            cfg.bgm.loop_until_end = v
                .as_bool()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "bgm.loop_until_end 必须是布尔"))?;
        }
    }
    let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    std::fs::write(dir.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;
    use axum::body::Body;
    // 只给侧写字幕那族用例用：函数本体住在 dhampir（转译/回读同一条链路）
    use super::dhampir::read_side_subtitles;

    // ⚠️ 这些用例是**按原字节搬来**的（含原本在文件顶层、缩进为 0 的那批）：
    //    统一重排缩进会动到多行字符串字面量，判据会因此悄悄变。



/// 成片时间线（EDL）必须能经 PUT 写进 `polish.toml`，且**没提到的键一个字节都不动**。
///
/// 为什么单独钉：`polish_project_put_locked` 是**逐字段白名单**，它自己上面就写着
/// 「没列进来的键保存时不报错、但静默存不进去」。第 3 批（前端成片轨）提交
/// `{edl: {clips: [...]}}` —— 白名单里**没有 `edl` 这一支**的话，界面显示"已保存"、
/// 磁盘上一个字节都没变、PUT 照回 `{"ok":true}`（正是那段注释预言的那类事故；
/// 交付前用探针单测真跑出来过，所以这条是**回归判据**，不是预防性装饰）。
#[test]
fn polish_project_put_writes_edl_and_leaves_unmentioned_keys_alone() {
    let dir = std::env::temp_dir().join(format!("yekitrim-edl-put-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = yekitrim_polish::project::ProjectFile::example();
    // 放一条屏蔽区间：PUT 只提交 `edl` 时，它必须**原样保留**（逐字段白名单的语义）。
    cfg.mute_ranges.push(yekitrim_polish::project::MuteRange { start: 1.0, end: 2.0 });
    std::fs::write(dir.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    // 1) 只有 `edl`：写进去了，而且没提到的 `mute_ranges` 没被动
    let body = serde_json::json!({ "edl": { "clips": [
        { "source_start": 3.0, "source_end": 6.0, "speed": 1.0, "kind": "replay", "label": "爆点重放" }
    ] } });
    let _ = polish_project_put_locked(&dir, &body).expect("保存 edl");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.edl.clips.len(), 1, "edl 没写进 polish.toml（PUT 白名单漏了 edl 这一支）");
    assert_eq!(after.edl.clips[0].source_start, 3.0);
    assert_eq!(after.edl.clips[0].kind.as_deref(), Some("replay"));
    assert_eq!(after.mute_ranges.len(), 1, "只提交 edl 时 mute_ranges 必须原样保留");
    assert_eq!(after.mute_ranges[0].start, 1.0);

    // 2) **老工程零回归**：清空 EDL 之后 `polish.toml` 里**不许留空的 `[edl]` 段头**。
    //    判据挂在 `ProjectFile.edl` 字段的 `skip_serializing_if = "Edl::is_empty"` 上 ——
    //    只挂在 `Edl::clips` 自己身上是不够的（toml 照样写出空的 `[edl]` 段头），
    //    那样老工程重存一次就"多一段"（data-model 的第一版单测正是这么红的）。
    let clear = serde_json::json!({ "edl": { "clips": [] } });
    let _ = polish_project_put_locked(&dir, &clear).expect("清空 edl");
    let raw = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    assert!(
        !raw.contains("[edl]"),
        "清空 EDL 之后不许留空的 `[edl]` 段头（老工程重存必须逐字节不变）:\n{raw}"
    );
    let cleared = yekitrim_polish::project::load_project(&dir).unwrap();
    assert!(cleared.edl.is_empty(), "清空之后内存里也该是空的");
    assert_eq!(cleared.mute_ranges.len(), 1, "清空 edl 不该碰 mute_ranges");

    let _ = std::fs::remove_dir_all(&dir);
}


/// **`[elements]` 必须能经 PUT 存下来**（2026-10-13）。
///
/// # 为什么补这条
///
/// `polish_project_put_locked` 是**逐字段白名单** —— 请求体里有一个键、白名单里没有它，
/// 就是**保存不报错、值静默丢掉**：界面显示"已保存"、PUT 照回 `{"ok":true}`，
/// 而 `polish.toml` 一个字节没变。本文件上面已经为同一个坑补过 `edl` 与 `[style.camera]`。
///
/// ⚠️ 起因是加 **T7 摇摆**（`sway_deg` / `sway_ms`）时发现：那一批文字动效字段
/// （打字机 / 弹入 / 抖动 / 底衬 / 摇摆）**连一个 UI 都没有，也没有 PUT 分支** ——
/// 用户只能手改 `polish.toml`，而**改完点保存会被静默丢掉**。
#[test]
fn elements_put_落盘且清空不留空段头() {
    let dir = std::env::temp_dir().join(format!("yekitrim-elements-put-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = yekitrim_polish::project::ProjectFile::example();
    // 放一条屏蔽区间：PUT 只提交 `elements` 时它必须**原样保留**（白名单的语义）。
    cfg.mute_ranges.push(yekitrim_polish::project::MuteRange { start: 1.0, end: 2.0 });
    std::fs::write(dir.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    // 1) 只提交 `elements`：摇摆 + 抖动都写进去，没提到的键没被动。
    let body = serde_json::json!({ "elements": {
        "sway_deg": 8.0,
        "sway_ms": 900,
        "shake_px": 14.0,
    } });
    let _ = polish_project_put_locked(&dir, &body).expect("保存 elements");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(
        after.elements.sway_deg,
        Some(8.0),
        "sway_deg 没写进 polish.toml（PUT 白名单漏了 elements 这一支 ⇒ 用户存了等于没存）"
    );
    assert_eq!(after.elements.sway_ms, Some(900), "sway_ms 没落盘");
    assert_eq!(after.elements.shake_px, Some(14.0), "shake_px 没落盘");
    assert_eq!(after.mute_ranges.len(), 1, "只提交 elements 时 mute_ranges 必须原样保留");

    // 2) **老工程零回归**：清空之后不许留空的 `[elements]` 段头。
    //    判据挂在 `ProjectFile.elements` 的 `skip_serializing_if` 上（与 `edl` 同款）。
    let clear = serde_json::json!({ "elements": {} });
    let _ = polish_project_put_locked(&dir, &clear).expect("清空 elements");
    let raw = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    assert!(
        !raw.contains("[elements]"),
        "清空之后不许留空的 `[elements]` 段头（老工程重存必须逐字节不变）:\n{raw}"
    );
    let cleared = yekitrim_polish::project::load_project(&dir).unwrap();
    assert!(cleared.elements.is_empty(), "清空之后内存里也该算空");
    assert_eq!(cleared.mute_ranges.len(), 1, "清空 elements 不该碰 mute_ranges");

    // 3) **脏键必须当场 400**（`deny_unknown_fields`）—— 拼错的键名静默存下来
    //    比报错坏得多：用户以为配上了，而没有任何东西读它。
    let bad = serde_json::json!({ "elements": { "sway_degrees": 8.0 } });
    assert!(
        polish_project_put_locked(&dir, &bad).is_err(),
        "拼错的键名必须报错，不许静默存下一个没人读的键"
    );

    let _ = std::fs::remove_dir_all(&dir);
}


/// **元素的 `color` 必须能经 PUT 存-读往返**（2026-10-13；2026-10-30 换夹具）。
///
/// # 为什么补这条
///
/// `Event::Element.color` 住在 `segments[].events[]` 里，而 `polish_project_put_locked`
/// 对 `segments` 这一支是**整份反序列化**（不是逐字段白名单）——
/// 所以它**理论上**自动往返。但"理论上自动"正是本仓最不该信的那句话：
/// 上游 `Event` 是 `deny_unknown_fields` 的 tagged enum，
/// **少一个 `#[serde(default)]`、或者字段加在 enum 的哪个变体上搞错了**，
/// 症状就是"面板上选了粉色、PUT 回 `{"ok":true}`、磁盘上一个字节没变"
/// （与 `edl` / `[elements]` / `[style.camera]` 那三次是**同一种事故形状**）。
///
/// ⚠️ 判据必须**读回磁盘上的 `polish.toml`**（不只是内存里的 `ProjectFile`）——
/// 只验内存的话，`skip_serializing_if` 写错（比如漏了 `color`）照样绿，
/// 而症状是"这次会话里是对的、重启之后颜色没了"。
///
/// ⚠️ **夹具用 `kind = "sticker"`，不是 `kind = "text"`**（2026-10-30）：文字层已于
/// 2026-10-20 整条退役，`yekitrim_polish::project::legacy` 在**读盘那一刻**就把
/// `type="element"` + `kind="text"` 的事件丢掉了（见 `legacy.rs` 的 `RetiredCounts::text_layer`）。
/// 原夹具写 `text` ⇒ PUT 之后读回来事件已经不存在 ⇒ 这条用例测的不再是 `color` 的往返，
/// 而是"退役是否生效"。判据本身与 kind 无关（`color` 是 `Event::Element` 上的字段），
/// 所以改用仍然存活的 `sticker`：往返通路一样、`color` 的 serde 属性一样被钉住。
/// ⚠️ `sticker` 只有 `time` + `text` 是必填，其余（含 `color`）都走 `#[serde(default)]`。
/// 文字层/`color` 链路本身的存废由精修侧决定，本用例只负责"字段能往返"。
#[test]
fn element_color_round_trips_through_put() {
    let dir = std::env::temp_dir().join(format!("yekitrim-elem-color-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = yekitrim_polish::project::ProjectFile::example();
    std::fs::write(dir.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    // 造一条带 `color` 的元素（字段名逐字照 `Event::Element`）。
    // ⚠️ kind 必须是 `sticker` —— `text` 会在读盘时被退役逻辑丢掉（见上方说明）。
    let body = serde_json::json!({ "segments": [{
        "start": 0.0,
        "end": 5.0,
        "events": [{
            "type": "element",
            "kind": "sticker",
            "time": 0.5,
            "end": 3.0,
            "text": "愤怒",
            "style": "hook",
            "color": "pink"
        }]
    }] });
    let _ = polish_project_put_locked(&dir, &body).expect("保存带 color 的元素");

    // ① 内存里读得回来。
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    let found = after
        .segments
        .iter()
        .flat_map(|s| s.events.iter())
        .find_map(|e| match e {
            yekitrim_polish::project::Event::Element { color, text, .. } if text == "愤怒" => {
                Some(color.clone())
            }
            _ => None,
        });
    assert_eq!(
        found,
        Some(Some("pink".to_string())),
        "color 没经 PUT 存下来 —— 面板上选了颜色、磁盘没变（白名单 / serde 漏了这一支）"
    );

    // ② 磁盘上也读得回来（不是只在内存里对）。
    let raw = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    assert!(
        raw.contains("color = \"pink\""),
        "`color` 没写进 polish.toml —— 重启之后颜色会消失。文件内容:\n{raw}"
    );

    // ③ **不写 `color` 的事件不许凭空多出这个键**（R4：存量工程逐字节不变）。
    //
    //    ⚠️ 这条**做过变异自证，结果是"抓不到"** —— 去掉 `color` 上的
    //    `skip_serializing_if` 之后它照样绿。真因记在 `types.rs:176`（2026-10-10 已实测过）：
    //    **`toml` 序列化本来就跳过 `Option::None`**，所以那个属性在这个字段上是冗余的。
    //    ⇒ 留着这条断言是**照实说明"这一档由类型本身保证"**，不是"有判据在拦"；
    //      它仍会抓到"把 `color` 改成 `String` / 给出默认空串"那类改动
    //      （那时 `None` 会变成 `Some("")` ⇒ 真写出 `color = ""`）。
    //    ⚠️ **别为它再写一次"变异自证"** —— 那个变异永远不会红（同 `speed` 字段那条记录）。
    let plain = serde_json::json!({ "segments": [{
        "start": 0.0,
        "end": 5.0,
        "events": [{
            "type": "element",
            "kind": "sticker",
            "time": 0.5,
            "end": 3.0,
            "text": "普通",
            "style": "hook"
        }]
    }] });
    let _ = polish_project_put_locked(&dir, &plain).expect("保存不带 color 的元素");
    let raw2 = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    let block = raw2
        .split("[[segments.events]]")
        .nth(1)
        .expect("应当有 events 段");
    assert!(
        !block.contains("color"),
        "没写 color 的事件被写出了 `color` 键 —— 存量工程重存一次就变样（R4 破）:\n{block}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}


/// **旁写字幕要按 doc 声明的资产读全**（2026-10-13，修"预览里没有文字元素"）。
///
/// # 这条钉的是什么
///
/// 原先只按**两个写死的名字**在 `--out` 旁边找，而**文字元素轨的 ASS 在片段根目录**
/// ⇒ 一条都没读过 ⇒ wasm 预览里文字元素**一个字都不显示**（成片正常、报告全绿）。
///
/// 这里直接打 `read_side_subtitles` 本体（它的输入就是"doc 文本 + 产物路径"），
/// 所以不需要真的跑 node 转译器 —— 判据要压的正是"**按谁的说法去找文件**"这件事。
#[test]
fn side_subtitles_follow_the_doc_not_a_hardcoded_name_list() {
    let root = std::env::temp_dir().join(format!("yekitrim-side-subs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    // 两个**不同**的目录：字幕/弹幕在 `--out` 旁边，文字轨在片段根 —— 复刻真实现状。
    let scratch = root.join("scratch");
    let clip = root.join("clip");
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::create_dir_all(&clip).unwrap();

    std::fs::write(scratch.join("dhampir-subtitle.srt"), "1\n00:00:01,000 --> 00:00:02,000\n字幕\n").unwrap();
    std::fs::write(scratch.join("dhampir-danmaku.ass"), "[Script Info]\n").unwrap();
    // ⚠️ 名字里带档位与颜色 —— 与转译器实际写出来的形状一致。
    let text_base = "dhampir-text_hook_center_pink.ass";
    std::fs::write(clip.join(text_base), "[Script Info]\nStyle: Default\n").unwrap();
    let translated = scratch.join("_dhampir_project_test.json");
    std::fs::write(&translated, "{}").unwrap();

    // doc 里三条 `kind == "subtitle"` 资产，**uri 是绝对路径**（与真产物同款）。
    let doc = serde_json::json!({
        "assets": [
            { "id": "dhampir-subtitle.srt", "kind": "subtitle",
              "uri": scratch.join("dhampir-subtitle.srt").to_string_lossy() },
            { "id": "dm", "kind": "subtitle",
              "uri": scratch.join("dhampir-danmaku.ass").to_string_lossy() },
            { "id": "text_hook_center_pink", "kind": "subtitle",
              "uri": clip.join(text_base).to_string_lossy() },
            // 非字幕资产**不许**被读进来（视频/贴纸那些读了就是白花 IO）。
            { "id": "clip.mp4", "kind": "video", "uri": clip.join("clip.mp4").to_string_lossy() }
        ]
    })
    .to_string();

    let subs = read_side_subtitles(&doc, &translated);
    let names: Vec<&str> = subs.iter().map(|(n, _)| n.as_str()).collect();

    // ① **文字轨必须在**（这一条就是本次修的缺陷；改动前它是红的）。
    assert!(
        names.contains(&text_base),
        "文字元素的 ASS 没被读进来（正是\"预览里没有文字元素\"那条缺陷）：{names:?}"
    );
    // ② 字幕与弹幕照旧（零回归）。
    assert!(names.contains(&"dhampir-subtitle.srt"), "字幕没读进来：{names:?}");
    assert!(names.contains(&"dhampir-danmaku.ass"), "弹幕没读进来：{names:?}");
    // ③ 只认 `kind == "subtitle"` —— 视频资产不该出现在这张表里。
    assert!(!names.contains(&"clip.mp4"), "非字幕资产被当成字幕读了：{names:?}");
    // ④ 键是 **basename**（前端按 uri 的 basename 查表，两处必须同口径）。
    assert!(
        names.iter().all(|n| !n.contains('/') && !n.contains('\\')),
        "键必须是纯文件名（前端按 basename 查表）：{names:?}"
    );
    // ⑤ 真去读了内容（不是只报了个名字）。
    let text_ass = subs.iter().find(|(n, _)| n == text_base).unwrap();
    assert!(text_ass.1.contains("Style: Default"), "内容没真的读出来");

    // ⑥ **doc 没有资产清单时回落到老路**（行为与改造前逐字节相同）。
    //
    //    ⚠️ 这条不是装饰：老产物 / 结构异常时若什么都不读，就是"预览里突然
    //    连字幕都没了"，而那种退化正是最容易被当成"另一个 bug"的形状。
    let legacy = read_side_subtitles("{\"timeline\":{}}", &translated);
    let legacy_names: Vec<&str> = legacy.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        legacy_names.contains(&"dhampir-subtitle.srt") && legacy_names.contains(&"dhampir-danmaku.ass"),
        "doc 里没有 assets 时，必须回落到那两个固定名字：{legacy_names:?}"
    );

    // ⑦ 读不出来的文件**不中断**别的（这条片段没有弹幕是常态）。
    std::fs::remove_file(scratch.join("dhampir-danmaku.ass")).unwrap();
    let partial = read_side_subtitles(&doc, &translated);
    let partial_names: Vec<&str> = partial.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        partial_names.contains(&text_base) && partial_names.contains(&"dhampir-subtitle.srt"),
        "一个旁写文件读不出来，不许拖垮其余那些：{partial_names:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}


/// 早就写好了，出片链路也真的在调它们 —— **缺的只是"写配置的入口"**。
/// 而本函数是**逐字段白名单**，`bgm` 不在名单里时：前端（乃至手写 API）发过来被**静默丢掉**，
/// PUT 照旧回 `{"ok":true}`，用户**没有任何办法**打开 BGM。
/// 与上面 `edl` 那条是**同一种事故形状**（白名单漏一支 ⇒ 界面说"已保存"、磁盘没变）。
#[test]
fn polish_project_put_writes_bgm() {
    let dir = std::env::temp_dir().join(format!("yekitrim-bgm-put-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = yekitrim_polish::project::ProjectFile::example();
    cfg.mute_ranges.push(yekitrim_polish::project::MuteRange { start: 1.0, end: 2.0 });
    std::fs::write(dir.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    // 1) 打开 BGM + 选文件：必须真的落盘，且没提到的 `mute_ranges` 原样保留
    let body = serde_json::json!({ "bgm": {
        "enabled": true,
        "file": "bgm/song.mp3",
        "volume": 0.3,
        "duck_ratio": 0.4,
    } });
    let _ = polish_project_put_locked(&dir, &body).expect("保存 bgm");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert!(after.bgm.enabled, "bgm.enabled 没写进 polish.toml（PUT 白名单漏了 bgm 这一支）");
    assert_eq!(after.bgm.file, "bgm/song.mp3");
    assert!((after.bgm.volume - 0.3).abs() < 1e-9, "bgm.volume 不对");
    assert!((after.bgm.duck_ratio - 0.4).abs() < 1e-9, "bgm.duck_ratio 不对");
    // 没提交的键保持默认（`fade_in` 1.0 / `fade_out` 2.0 / `loop_until_end` true）
    assert!((after.bgm.fade_in - 1.0).abs() < 1e-9, "缺席的 fade_in 该保持默认 1.0");
    assert!((after.bgm.fade_out - 2.0).abs() < 1e-9, "缺席的 fade_out 该保持默认 2.0");
    assert!(after.bgm.loop_until_end, "缺席的 loop_until_end 该保持默认 true");
    assert_eq!(after.mute_ranges.len(), 1, "只提交 bgm 时 mute_ranges 必须原样保留");

    // 2) **路径面**：`file` 必须是工程内相对路径（绝对路径 / `..` 一律拒收）。
    //    ⚠️ 这条是第二道兜底，第一道在 `scope.rs`；两道都要有。
    for bad in ["/etc/passwd", "C:/Windows/win.ini", "../outside.mp3", "a/../../b.mp3"] {
        let body = serde_json::json!({ "bgm": { "file": bad } });
        assert!(
            polish_project_put_locked(&dir, &body).is_err(),
            "bgm.file = {bad:?} 必须被拒收（它是工程内相对路径，不是服务器任意路径）"
        );
    }
    // 拒收之后磁盘上那份不许被改坏
    let still = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(still.bgm.file, "bgm/song.mp3", "拒收非法值之后，原来的 file 必须原样保留");

    // 3) **坏值当场报错、不静默**（写进去会让出片按一个没人预期的音量混合）
    let body = serde_json::json!({ "bgm": { "enabled": "yes" } });
    assert!(polish_project_put_locked(&dir, &body).is_err(), "bgm.enabled 必须是布尔");

    // 4) **老工程零回归**：关掉且清空文件之后，`polish.toml` 里**不许留空的 `[bgm]` 段头**。
    //    判据挂在 `BgmConfig::is_empty()` 的 `skip_serializing_if` 上 ——
    //    少了它，老工程重存一次就"多一段"（与 `[edl]` 那条完全同形）。
    let clear = serde_json::json!({ "bgm": { "enabled": false, "file": "" } });
    let _ = polish_project_put_locked(&dir, &clear).expect("关闭 bgm");
    let raw = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    assert!(
        !raw.contains("[bgm]"),
        "关掉且没选文件之后不许留空的 `[bgm]` 段头（老工程重存必须逐字节不变）:\n{raw}"
    );
    assert!(yekitrim_polish::project::load_project(&dir).unwrap().bgm.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}


/// 成片时间线（EDL）必须能**经整张路由表**（真 HTTP：会话 + CSRF + 作用域 + 路径锁）存下来、
/// 再读回来。这是第 3 批交付时唯一**没被验到**的那条缝（前端成片轨改完 → F5 → 还在不在）。
///
/// 为什么 `polish_project_put_writes_edl_and_leaves_unmentioned_keys_alone` 不够：
/// 那条直接调 `polish_project_put_locked`，绕过了 ① 路由挂没挂 ② `scope.rs::enforce_scope`
/// 会不会把 `dir` 拦掉 ③ 请求体解析 ④ GET 侧有没有把 `edl` 回给前端。
/// 这四处任意一处错，前端都是"显示已保存、刷新就没了"，而上面那条单测**照样绿**。
#[tokio::test]
async fn polish_project_http_roundtrips_edl() {
    let app = crate::api::test_support::test_app().await;
    let dir = app.root.to_string_lossy().replace('\\', "/");
    let cfg = yekitrim_polish::project::ProjectFile::example();
    std::fs::write(app.root.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    // 1) PUT：走真 HTTP（带会话 cookie + CSRF 头）
    let body = serde_json::json!({
        "dir": dir,
        "edl": { "clips": [
            { "source_start": 3.0, "source_end": 6.0, "speed": 1.0, "kind": "replay", "label": "爆点重放" },
            { "source_start": 0.0, "source_end": 12.0, "speed": 1.0 },
        ] },
    });
    let req = app
        .req("PUT", "/polish/project")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = app.call(req).await;
    assert_eq!(res.status(), 200, "PUT /polish/project 应当 200");

    // 2) 磁盘上真的写了
    let on_disk = yekitrim_polish::project::load_project(&app.root).unwrap();
    assert_eq!(on_disk.edl.clips.len(), 2, "edl 没落到 polish.toml");
    assert_eq!(on_disk.edl.clips[0].kind.as_deref(), Some("replay"));

    // 3) GET 要把它读回来（前端刷新靠这一步；GET 丢了 `edl` 同样是"刷新就没了"）
    let req = app
        .req("GET", &format!("/polish/project?dir={dir}"))
        .body(Body::empty())
        .unwrap();
    let res = app.call(req).await;
    assert_eq!(res.status(), 200);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let clips = json
        .get("edl")
        .and_then(|e| e.get("clips"))
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("GET 没把 edl.clips 回给前端（刷新后成片轨会空）: {json}"));
    assert_eq!(clips.len(), 2);
    assert_eq!(clips[0].get("kind").and_then(|k| k.as_str()), Some("replay"));
    assert_eq!(clips[0].get("source_start").and_then(|v| v.as_f64()), Some(3.0));

    // 4) 再 PUT 一次清空 ⇒ 磁盘上不许留空的 `[edl]` 段头（R4：老工程重存逐字节不变）
    let clear = serde_json::json!({ "dir": dir, "edl": { "clips": [] } });
    let req = app
        .req("PUT", "/polish/project")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(clear.to_string()))
        .unwrap();
    assert_eq!(app.call(req).await.status(), 200);
    let raw = std::fs::read_to_string(app.root.join("polish.toml")).unwrap();
    assert!(!raw.contains("[edl]"), "清空后不许留空的 `[edl]` 段头:\n{raw}");
}


/// **§11 BGM 必须能经整张路由表**（真 HTTP：会话 + CSRF + 作用域 + 路径锁）存下来、再读回来。
///
/// 为什么上面那条 `polish_project_put_writes_bgm` 不够：那条直接调
/// `polish_project_put_locked`，绕过了 ① 路由挂没挂 ② `scope.rs::enforce_scope`
/// 会不会把 `dir` 拦掉 ③ 请求体解析 ④ GET 侧有没有把 `bgm` 回给前端。
/// 这四处任意一处错，用户都是"显示已保存、刷新就弹回去"，而上面那条**照样绿**。
/// （`edl` / `camera` 两条同形用例就是这个道理。）
///
/// ⚠️ 顺带钉住**路径面**：`bgm.file` 是"工程内相对路径"，绝对路径必须被拒 ——
/// 它与 `cover.image` 是同一类字段，多一个漏收口的口子就是一条任意文件读取。
#[tokio::test]
async fn polish_project_http_roundtrips_bgm() {
    let app = crate::api::test_support::test_app().await;
    let dir = app.root.to_string_lossy().replace('\\', "/");
    let cfg = yekitrim_polish::project::ProjectFile::example();
    std::fs::write(app.root.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    // 1) PUT：前端发的形状（`saveProjectPatch(pid, dir, { bgm: patch })`）
    let body = serde_json::json!({
        "dir": dir,
        "bgm": { "enabled": true, "file": "bgm/song.mp3", "volume": 0.3, "duck_ratio": 0.4 },
    });
    let req = app
        .req("PUT", "/polish/project")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    assert_eq!(app.call(req).await.status(), 200, "PUT /polish/project 应当 200");

    // 2) 磁盘上真的写了
    let on_disk = yekitrim_polish::project::load_project(&app.root).unwrap();
    assert!(on_disk.bgm.enabled, "bgm 没落到 polish.toml（白名单漏了 bgm 这一支）");
    assert_eq!(on_disk.bgm.file, "bgm/song.mp3");
    assert!((on_disk.bgm.volume - 0.3).abs() < 1e-9);

    // 3) GET 要把它读回来 —— 前端刷新靠这一步；GET 丢了 `bgm` 同样是"刷新就弹回去"
    let req = app
        .req("GET", &format!("/polish/project?dir={dir}"))
        .body(Body::empty())
        .unwrap();
    let res = app.call(req).await;
    assert_eq!(res.status(), 200);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let bgm = json
        .get("bgm")
        .unwrap_or_else(|| panic!("GET 没把 bgm 回给前端（勾选框刷新后会弹回去）: {json}"));
    assert_eq!(bgm.get("enabled").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(bgm.get("file").and_then(|v| v.as_str()), Some("bgm/song.mp3"));

    // 4) **路径面**：绝对路径必须被拒（这是第二道；第一道在 scope 中间件）
    let bad = serde_json::json!({ "dir": dir, "bgm": { "file": "C:/Windows/win.ini" } });
    let req = app
        .req("PUT", "/polish/project")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(bad.to_string()))
        .unwrap();
    assert_eq!(app.call(req).await.status(), 400, "绝对路径必须被拒收");

    // 5) 关掉并清空 ⇒ 不许留空的 `[bgm]` 段头（R4：老工程重存逐字节不变）
    let clear = serde_json::json!({ "dir": dir, "bgm": { "enabled": false, "file": "" } });
    let req = app
        .req("PUT", "/polish/project")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(clear.to_string()))
        .unwrap();
    assert_eq!(app.call(req).await.status(), 200);
    let raw = std::fs::read_to_string(app.root.join("polish.toml")).unwrap();
    assert!(!raw.contains("[bgm]"), "关掉后不许留空的 `[bgm]` 段头:\n{raw}");
}


/// §18-C：`[style.camera]` 三个档位系数必须能被**存进去**、**读回来**、且**越界被夹住**。
///
/// 为什么值得一条真 HTTP 用例（而不是只测 `CameraStyle` 的反序列化）：
/// 白名单里漏一个键 = **保存不报错、值静默丢掉**（本文件上面那段注释记着这个坑，
/// `edl` 那次就是被探针单测抓出来的）。`camera` 是同一个形状的新增键，
/// 只测结构体的话，**handler 里忘了接**它照样绿。
#[tokio::test]
async fn polish_project_http_roundtrips_camera_zoom() {
    let app = crate::api::test_support::test_app().await;
    let dir = app.root.to_string_lossy().replace('\\', "/");
    let cfg = yekitrim_polish::project::ProjectFile::example();
    std::fs::write(app.root.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    // 1) PUT：三档都给一个**合法**值，近景调到用户想要的方向（1.5 → 1.8，即方案 A 的目标倍率）
    let body = serde_json::json!({
        "dir": dir,
        "camera": { "zoom_mid": 1.3, "zoom_close": 1.8, "zoom_wide": 0.55 },
    });
    let req = app
        .req("PUT", "/polish/project")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    assert_eq!(app.call(req).await.status(), 200, "PUT /polish/project 应当 200");

    // 2) 磁盘上真的写了（这是"静默丢掉"唯一能被抓到的地方）
    let on_disk = yekitrim_polish::project::load_project(&app.root).unwrap();
    assert_eq!(on_disk.style.camera.zoom_close, 1.8, "camera.zoom_close 没落到 polish.toml");
    assert_eq!(on_disk.style.camera.zoom_mid, 1.3);
    assert_eq!(on_disk.style.camera.zoom_wide, 0.55);
    // `factor()` 是四条渲染路径共用的那张表 —— 存进去的值要真的从它读出来
    assert_eq!(on_disk.style.camera.factor("近"), 1.8, "factor() 没读到新值");

    // 3) 越界要被**夹住**（不是照单全收，也不是报错）
    //    zoom_close 太小（< 1 会缩出黑边，那是画中画的语义）、zoom_wide 太大（> 1 就不是画中画了）
    let wild = serde_json::json!({
        "dir": dir,
        "camera": { "zoom_mid": 1.3, "zoom_close": 0.2, "zoom_wide": 9.0 },
    });
    let req = app
        .req("PUT", "/polish/project")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(wild.to_string()))
        .unwrap();
    assert_eq!(app.call(req).await.status(), 200, "越界是夹取、不是报错（滑杆拖到边界是正常操作）");
    let clamped = yekitrim_polish::project::load_project(&app.root).unwrap();
    assert_eq!(clamped.style.camera.zoom_close, 1.0, "zoom_close 下限该夹到 1.0");
    assert_eq!(clamped.style.camera.zoom_wide, 1.0, "zoom_wide 上限该夹到 1.0");
    assert_eq!(clamped.style.camera.zoom_mid, 1.3, "没越界的档位不许被顺手改");

    // 4) GET 要把它读回来（刷新后界面上还是真实生效的值）
    let req = app
        .req("GET", &format!("/polish/project?dir={dir}"))
        .body(Body::empty())
        .unwrap();
    let res = app.call(req).await;
    assert_eq!(res.status(), 200);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let got = json
        .get("style")
        .and_then(|s| s.get("camera"))
        .and_then(|c| c.get("zoom_close"))
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| panic!("GET 没把 style.camera.zoom_close 回给前端: {json}"));
    assert_eq!(got, 1.0, "GET 要回**夹取后**的值（界面据此回弹到真实生效的数）");

    // 5) 非有限数要明确拒（`clamp` 对 NaN 是恒等，会把它一路写进渲染）
    let nan = serde_json::json!({ "dir": dir, "camera": { "zoom_close": 0.0 } });
    let req = app
        .req("PUT", "/polish/project")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(nan.to_string()))
        .unwrap();
    assert_eq!(app.call(req).await.status(), 200, "0 会被夹到 1.0（不是 400）");
    let z = yekitrim_polish::project::load_project(&app.root).unwrap();
    assert_eq!(z.style.camera.zoom_close, 1.0, "0 该夹到下限 1.0");
}


/// **§1 决策理由必须经整张路由表回给前端**（真 HTTP：会话 + CSRF + 作用域 + 路径锁）。
///
/// ⚠️ 为什么不能只测 Rust 侧：`serde_json::to_value(&cfg)` 里**自动**带着 `decisions`，
/// 但这只说明"序列化器认得它"。前端拿不到它还有**另外三处**能坏：
/// ① `enforce_scope` 把 `dir` 拦掉；② 请求体解析；③ GET 侧改成手工挑字段的 map
/// （`polish_project_get_impl` 里就有一串 `obj.insert(...)` 的手工装配，随时可能被改成白名单）。
/// 任意一处错，用户看到的就是"理由面板永远空的"，而 Rust 那侧测试**照样绿**。
#[tokio::test]
async fn polish_project_http_returns_decisions() {
    let app = crate::api::test_support::test_app().await;
    let dir = app.root.to_string_lossy().replace('\\', "/");
    let mut cfg = yekitrim_polish::project::ProjectFile::example();
    cfg.decisions = vec![yekitrim_polish::project::Decision::at(
        "boundary_sentence_end",
        12.5,
        "切点原来落在一句话中间；已挪到最近一条字幕的结束处",
        Some("10.00s → 12.50s".to_string()),
    )];
    std::fs::write(app.root.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    let req = app
        .req("GET", &format!("/polish/project?dir={dir}"))
        .body(Body::empty())
        .unwrap();
    let res = app.call(req).await;
    assert_eq!(res.status(), 200);
    let json = body_json(res).await;
    let ds = json
        .get("decisions")
        .and_then(|d| d.as_array())
        .unwrap_or_else(|| panic!("GET 没把 decisions 回给前端（理由面板会永远空）: {json}"));
    assert_eq!(ds.len(), 1, "条数不对：{ds:?}");
    assert_eq!(ds[0].get("kind").and_then(|v| v.as_str()), Some("boundary_sentence_end"));
    assert_eq!(ds[0].get("at").and_then(|v| v.as_f64()), Some(12.5));
    assert!(
        ds[0].get("text").and_then(|v| v.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false),
        "理由文案要真的传过来（界面直接显示它）: {ds:?}"
    );
    assert_eq!(ds[0].get("detail").and_then(|v| v.as_str()), Some("10.00s → 12.50s"));
}


/// 画布方向 / 画面构成 / 贴纸落点要能真写进 `polish.toml`，且**不碰**没提到的段。
///
/// 回归：PUT 是**逐字段白名单**（见 `polish_project_put_locked`）——漏一个分支就是
/// "保存不报错、但值静默丢掉"，前端还会以为存上了。
#[test]
fn polish_project_put_writes_canvas_stage_sticker() {
    let dir = std::env::temp_dir().join(format!("yekitrim-stage-put-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = yekitrim_polish::project::ProjectFile::example();
    cfg.mute_ranges.push(yekitrim_polish::project::MuteRange { start: 1.0, end: 2.0 });
    std::fs::write(dir.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    // 1) 只切画布方向：其它段原样保留
    let body = serde_json::json!({ "canvas": { "orientation": "portrait" } });
    let _ = polish_project_put_locked(&dir, &body).expect("保存画布方向");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.canvas.size(), (1080, 1920));
    assert_eq!(after.mute_ranges.len(), 1, "没提到的字段必须原样保留");
    // ⚠️ 2026-10-05 起位置的权威是**两轴** `position_x_pct` / `position_y_pct`（老单值
    // `position_pct` 与老枚举 `position` 都已 `skip_serializing`、只读兼容）——默认 = 没有新字段
    // ⇒ 走老枚举换算，`auto` → 50 居中。
    assert_eq!(after.stage.position_x_pct, None, "没提交的段走默认（None），不能被顺手重置");
    assert_eq!(after.stage.position_y_pct, None, "没提交的段走默认（None），不能被顺手重置");
    let fit0 = after.resolve_stage(Some((1920, 1080)));
    assert_eq!(
        (fit0.position_x_pct, fit0.position_y_pct),
        (50.0, 50.0),
        "缺两轴的老工程按老枚举换算：auto → 50 居中（两轴同值）"
    );

    // 2) 画面构成（两轴各自独立）
    let body = serde_json::json!({ "stage": {
        "bg_mode": "color", "bg_color": "#112233", "blur_scale": 2.0,
        "position_x_pct": 25.0, "position_y_pct": 100.0, "scale": 1.25, "padding": 30,
    }});
    let _ = polish_project_put_locked(&dir, &body).expect("保存画面构成");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.canvas.size(), (1080, 1920), "没提交 canvas 就不能被重置");
    let fit = after.resolve_stage(Some((1920, 1080)));
    assert_eq!(fit.axis, "y", "竖画布 × 横素材 = 上下留边");
    // 位置 = **两轴百分比**（各五档之一）；老枚举只是过渡期兼容输出。
    assert_eq!((fit.position_x_pct, fit.position_y_pct), (25.0, 100.0));
    assert_eq!(fit.position, "bottom", "竖画布 + Y=100 ⇒ 兼容枚举给 bottom");
    assert_eq!(fit.bg, "color");
    assert_eq!(fit.bg_color, "#112233");
    assert!((fit.padding - 16.875).abs() < 1e-9, "30px × k(0.5625)");

    // 3) 贴纸落点
    let body = serde_json::json!({ "sticker": { "position_mode": "manual", "position": "20% 80%" } });
    let _ = polish_project_put_locked(&dir, &body).expect("保存贴纸落点");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.sticker.position_mode, "manual");
    assert_eq!(after.sticker.position, "20% 80%");
    assert_eq!(after.stage.position_x_pct, Some(25.0), "上一段必须还在");
    assert_eq!(after.stage.position_y_pct, Some(100.0), "上一段必须还在");

    // 4) 落到 toml 文本上：段在、段注释在、根级 sticker_source 没被 [sticker] 顶掉
    let raw = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    for needle in [
        "[canvas]",
        "[stage]",
        "[sticker]",
        "orientation = \"portrait\"",
        "position_mode = \"manual\"",
        // ⚠️ 位置写的是两轴新字段；老枚举 `position` 与老单值 `position_pct` 都已 `skip_serializing`、
        // **不该再出现在 [stage] 段里**（判据只在 [stage] 段内断：贴纸与事件也各有一个同名 `position` 键）。
        "position_x_pct = 25.0",
        "position_y_pct = 100.0",
        "# ===== 画面构成",
        "# ===== 贴纸落点",
        "sticker_source",
    ] {
        assert!(raw.contains(needle), "缺 {needle}:\n{raw}");
    }
    let stage_sec = raw.split("[stage]").nth(1).unwrap_or("");
    let stage_sec = stage_sec.split("\n[").next().unwrap_or("");
    assert!(!stage_sec.contains("position_pct ="), "老单值字段不该再写出:\n{stage_sec}");

    // 5) 非法值：文件照存（不篡改用户输入），但渲染侧取到的是安全值
    let body = serde_json::json!({ "stage": {
        "bg_mode": "frame", "bg_color": "url(evil)", "position_x_pct": 0.0, "position_y_pct": 0.0
    }});
    let _ = polish_project_put_locked(&dir, &body).expect("保存非法值也不应报错");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    let fit = after.resolve_stage(Some((1920, 1080)));
    assert_eq!((fit.position_x_pct, fit.position_y_pct), (0.0, 0.0));
    assert_eq!(fit.position, "top");
    assert_eq!(fit.bg, "blur", "未知 bg_mode 回落 auto → 补边时按模糊处理");
    assert_eq!(fit.bg_color, "#000000", "颜色必须净化，不能把 url(...) 带进模板 CSS");

    // 6) **老单值 `position_pct` 在 API 面上仍能迁移，而且写盘时会固化到两轴**：
    //    提交它 = 两轴都吃这个值（逐像素不变）；落盘的必须是**新键**，否则 `skip_serializing`
    //    会让老单值在下次加载时凭空消失（用户只动一条轴、另一条轴跳回 50 —— 静默丢值）。
    let body = serde_json::json!({ "stage": { "position_pct": 75.0 } });
    let _ = polish_project_put_locked(&dir, &body).expect("保存老单值字段");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    let fit = after.resolve_stage(Some((1920, 1080)));
    assert_eq!(
        (after.stage.position_x_pct, after.stage.position_y_pct),
        (Some(75.0), Some(75.0)),
        "写盘时必须把老单值固化到两轴（`migrate_position_pct`）"
    );
    assert_eq!(
        (fit.position_x_pct, fit.position_y_pct),
        (75.0, 75.0),
        "老单值 ⇒ 两轴同值（贴满的那条轴余量为 0，与老行为逐像素等价）"
    );
    // 只动一条轴：另一条轴必须**保持**老值（不是跳回 50）
    let body = serde_json::json!({ "stage": { "position_pct": 75.0, "position_x_pct": 25.0 } });
    let _ = polish_project_put_locked(&dir, &body).expect("只提交一条轴 + 老单值");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(
        (after.stage.position_x_pct, after.stage.position_y_pct),
        (Some(25.0), Some(75.0)),
        "缺的那条轴补老单值（缺一补一），已给的那条不被覆盖"
    );

    let _ = std::fs::remove_dir_all(&dir);
}


/// 字幕样式五键（`subtitle_color` / `subtitle_stroke` / `subtitle_outer_stroke` /
/// `subtitle_size` / `subtitle_position_pct`）**必须真的写进 `polish.toml`**。
///
/// ⚠️ `subtitle_outer_stroke` 是本条**唯一允许空串**的键：空串 = **显式关**（面板开关"关"写它），
/// 与 `subtitle_stroke` 的"空串拒收"正好相反 —— 写成同一支会让开关关不上（或描边被清空）。
///
/// ⚠️ 这条测试的存在理由就是上面那段注释里的坑：`polish_project_put_locked` 是**逐字段白名单**，
/// 没列进来的键 **PUT 照样回 `{"ok":true}`、而磁盘一个字都不动**。实测过（2026-10）：
/// 前端把 4 个键全发出来，`polish.toml` 里 4 个值一个都没变，控件随即回弹到原值，
/// **全程不报任何错**。字幕样式这套控件的后端口子就曾经整条缺失。
///
/// 同时钉住本字段的**语义核心**：`position_pct: null` 与「键缺席」**不是一回事** ——
/// 前者是"显式恢复未设置"（`Some → None`），后者是"别动磁盘"。
#[test]
fn polish_project_put_writes_subtitle_style() {
    let dir = std::env::temp_dir().join(format!("yekitrim-substyle-put-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = yekitrim_polish::project::ProjectFile::example();
    std::fs::write(dir.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    // 1) 四个键一次提交：全部要落到盘上
    let body = serde_json::json!({
        "subtitle_color": "#ff0000",
        "subtitle_stroke": "8px #00ff00",
        // ⚠️ 用一个**非默认值**（默认现在是 `auto #ffffff`）：拿默认值测的话，
        // "写没写进去"这条断言**永远是绿的**（不写也读回这个值）—— 判据就白写了。
        "subtitle_outer_stroke": "8px #ffffff",
        "subtitle_size": 96,
        "subtitle_position_pct": 25.0,
    });
    let _ = polish_project_put_locked(&dir, &body).expect("保存字幕样式");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.style.subtitle.color, "#ff0000", "颜色必须写进去");
    assert_eq!(after.style.subtitle.stroke, "8px #00ff00", "描边必须写进去");
    assert_eq!(
        after.style.subtitle.outer_stroke, "8px #ffffff",
        "第二层（白环）必须写进去 —— 固定宽度那一种写法"
    );
    assert_eq!(after.style.subtitle.size, 96, "字号必须写进去");
    assert_eq!(after.style.subtitle.position_pct, Some(25.0), "落点必须写进去");
    // 没提交的键不能被顺手重置。
    // ⚠️ 期望值取**当期默认**（不是写死的色值）：换色（2026-10-20 A 套柔和莫兰迪）不该再让这条红第二次。
    assert_eq!(
        after.style.subtitle.highlight_color,
        yekitrim_polish::project::ProjectFile::example().style.subtitle.highlight_color,
        "没提到的字段必须原样保留"
    );


    // 2) 落到 toml 文本上（防"内存里对了但序列化没写"）。
    //    ⚠️ 颜色/描边/字号在 `[style.subtitle]` 段内，但 `position_pct` 我们**只在这里**断言，
    //    因为 `[stage]` 段有个同名的老字段 —— 见第 3 步那段注释。
    let raw = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    let sub_sec = || {
        raw.split("[style.subtitle]")
            .nth(1)
            .unwrap_or("")
            .split("\n[")
            .next()
            .unwrap_or("")
            .to_string()
    };
    let sec = sub_sec();
    for needle in [
        "color = \"#ff0000\"",
        "stroke = \"8px #00ff00\"",
        "outer_stroke = \"8px #ffffff\"",
        "size = 96",
    ] {
        assert!(sec.contains(needle), "`[style.subtitle]` 缺 {needle}:\n{sec}");
    }
    assert!(
        sec.lines().any(|l| l.trim_start() == "position_pct = 25.0"),
        "`[style.subtitle]` 缺 `position_pct = 25.0`:\n{sec}"
    );

    // 3) ⚠️ 语义核心：`null` = 显式恢复"未设置"（这是「位置 → 默认」按钮的唯一通路）
    let body = serde_json::json!({ "subtitle_position_pct": serde_json::Value::Null });
    let _ = polish_project_put_locked(&dir, &body).expect("恢复默认落点");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.style.subtitle.position_pct, None, "null 必须把 Some 打回 None");
    assert_eq!(after.style.subtitle.size, 96, "null 只作用于落点，别的字段不能跟着被重置");
    // ⚠️ **判据只能看 `[style.subtitle]` 那一段里的行**，两个坑都要躲开：
    //   ① `to_toml_commented` 的**注释文本**里就写着 "position_pct" 这个词（说明缺省语义那段）
    //      ⇒ `raw.contains("position_pct")` 恒真、断言会空转；
    //   ② `[stage]` 段**本来就有一个同名的** `position_pct`（画面构成的单值落点，与本字段无关）
    //      ⇒ 全文扫 `starts_with("position_pct =")` 会被它误伤。
    //    所以先切出 `[style.subtitle]` 段、再逐行看行首。
    //
    // ⚠️⚠️ **必须重新读盘**：上面那个 `raw` 是第 2 步（写 null **之前**）读的 —— 直接复用会让这条
    //     断言永远看着"删掉了"（假绿），而那正是本测试要防的东西（第一版就是这么写的，一跑就露）。
    let raw2 = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    let sub_sec = raw2
        .split("[style.subtitle]")
        .nth(1)
        .unwrap_or("")
        .split("\n[")
        .next()
        .unwrap_or("");
    assert!(
        !sub_sec.lines().any(|l| l.trim_start().starts_with("position_pct =")),
        "恢复默认后 [style.subtitle] 里不该再有 position_pct 键:\n{sub_sec}"
    );

    // 4) **键缺席 = 不动磁盘**（老工程零回归 R4 的前提）
    let body = serde_json::json!({ "subtitle_color": "#123456" });
    let _ = polish_project_put_locked(&dir, &body).expect("只改颜色");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.style.subtitle.color, "#123456");
    assert_eq!(after.style.subtitle.size, 96, "缺席的键不能被默认值顶掉");
    assert_eq!(after.style.subtitle.position_pct, None, "缺席的键不能被顶回某个默认百分比");

    // 5) 坏值当场报 400，**不静默**
    let bad = serde_json::json!({ "subtitle_size": "很大" });
    assert!(polish_project_put_locked(&dir, &bad).is_err(), "字号非数字必须报错");
    let bad = serde_json::json!({ "subtitle_position_pct": "中间" });
    assert!(polish_project_put_locked(&dir, &bad).is_err(), "落点非数字且非 null 必须报错");
    // 坏颜色**不写**（也不报错 —— 与既有净化口径一致），但不能污染磁盘
    let bad = serde_json::json!({ "subtitle_color": "not-a-color" });
    let _ = polish_project_put_locked(&dir, &bad).expect("坏颜色不报错");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.style.subtitle.color, "#123456", "坏颜色必须被拒收、不能写脏");

    // 6) **白环的空串 = 显式关**（本键唯一允许空串的语义；面板开关"关"写它）。
    //    ⚠️ 必须排在这一条**最后**：它会把 `[style.subtitle]` 里的值改掉，
    //    放在前面会让第 2 步那条"落到 toml 文本上"的断言看到空串（第一版就是这么写的，一跑就露）。
    //    ⚠️ 与 `subtitle_stroke` 的"空串拒收"正好相反 —— 写成同一支会让开关**关不上**。
    let _ = polish_project_put_locked(
        &dir,
        &serde_json::json!({ "subtitle_outer_stroke": "" }),
    )
    .expect("关闭白环");
    let after_off = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(
        after_off.style.subtitle.outer_stroke, "",
        "空串必须写进去 —— 它是「显式关」，不是「坏值」"
    );

    // 6) 越界值被夹住（0~50 / 8~400），不是写进野值
    let body = serde_json::json!({ "subtitle_position_pct": 999.0, "subtitle_size": 9999 });
    let _ = polish_project_put_locked(&dir, &body).expect("越界值应当被夹住");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.style.subtitle.position_pct, Some(50.0), "落点夹到 50");
    assert_eq!(after.style.subtitle.size, 400, "字号夹到 400");

    let _ = std::fs::remove_dir_all(&dir);
}



/// 手持摇摆（`Event::Camera::handheld`）随 `segments` 整体收下 ——
/// 运镜新字段走的是既有的 segments 通道（不是逐字段白名单里的新分支），
/// 但正因如此必须**断言它真的落到 toml 上**：反序列化若把未知字段静默丢掉，
/// PUT 依旧返回 ok、前端以为存上了（同一个坑见上一测试的注释）。
///
/// 同时钉住「老配置零迁移」：`handheld` 缺省 false 不落盘（`skip_serializing_if`），
/// 没开摇摆的事件在 toml 里不该多出 `handheld = false` 这行噪音。
#[test]
fn polish_project_put_roundtrips_handheld_sway() {
    let dir = std::env::temp_dir().join(format!("yekitrim-handheld-put-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("polish.toml"),
        "[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n",
    )
    .unwrap();

    let body = serde_json::json!({ "segments": [
        { "start": 0.0, "end": 30.0, "fx": "high", "note": "",
          "events": [
            { "type": "camera", "time": 1.0, "end": 3.0, "zoom": "近", "origin": "50% 50%", "handheld": true },
            { "type": "camera", "time": 5.0, "end": 7.0, "zoom": "中", "origin": "50% 50%" }
          ] }
    ]});
    let _ = polish_project_put_locked(&dir, &body).expect("保存运镜事件");

    // 1) 重新加载：值还在
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    let flags: Vec<bool> = after.segments[0]
        .events
        .iter()
        .map(|ev| match ev {
            yekitrim_polish::project::Event::Camera { handheld, .. } => *handheld,
            other => panic!("应当是运镜事件: {other:?}"),
        })
        .collect();
    assert_eq!(flags, vec![true, false]);

    // 2) 落盘文本：true 必须写出、缺省 false 不写
    let raw = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    assert!(raw.contains("handheld = true"), "手持摇摆没落到 toml:\n{raw}");
    assert!(!raw.contains("handheld = false"), "缺省值不该落盘:\n{raw}");

    let _ = std::fs::remove_dir_all(&dir);
}


/// 字幕保存要**按开始时间排序**后再落盘（去静音/预览都依赖字幕有序）。
///
/// 回归：编辑器「拆分」曾把后半段 push 到数组末尾，存出来的字幕文件是乱序的 ——
/// 去静音按文件顺序取相邻间隔，于是判出 13 段/121s 的假静音（实测 clip-19，
/// 用户"去静音完全对不上，像用了别的素材"）。乱序的旧数据**再存一次就自愈**。
#[tokio::test]
async fn subtitle_save_normalizes_time_order() {
    let t = test_app().await;
    let dir = t.root.join("output").join("clips").join("clip-19");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("polish.toml"),
        "[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n",
    )
    .unwrap();

    // 乱序：末尾那条其实是开头那段的「拆分后半段」（就是出问题的形状）
    let body = serde_json::json!({
        "dir": yekitrim_core::path::display(&dir),
        "segments": [
            { "start": 0.0,  "end": 2.07,  "text": "前半" },
            { "start": 99.7, "end": 102.4, "text": "后面一句" },
            { "start": 2.07, "end": 5.79,  "text": "拆分出来的后半" },
        ],
    });
    let res = t
        .call(
            t.req("PUT", "/polish/subtitles")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    let saved: Vec<f64> = {
        let raw = std::fs::read_to_string(dir.join("data").join("subtitles.json")).unwrap();
        serde_json::from_str::<serde_json::Value>(&raw)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["start"].as_f64().unwrap())
            .collect()
    };
    assert_eq!(saved, vec![0.0, 2.07, 99.7], "落盘顺序要按开始时间");

    // clip.srt 同步归一化（外部工具读它）
    let srt = std::fs::read_to_string(dir.join("clip.srt")).unwrap();
    let order: Vec<&str> = srt
        .lines()
        .filter(|l| l.contains("-->"))
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(order, vec!["00:00:00,000", "00:00:02,070", "00:01:39,700"], "{srt}");
}


/// §23 节奏报告必须**读得到**，而且必须是**只读**的（2026-10-13）。
///
/// # 为什么补这条
///
/// `process` 早就把 >6s 静止区间落成 `data/pacing.json`（`write_pacing_report`），
/// 但**全仓没有第二个消费者** —— 界面读不到、API 不暴露。
/// 那等于"算了、也落盘了，但用户永远看不见"，正是规格要避免的
///（「报告必须是可查的，不能只活在控制台里 —— 那等于没算」）。
///
/// ⚠️⚠️ **这条的重心是"只读"**：`applied` 必须**原样**是 `false`。
/// 规格强调过「留白是'没有说话'，不是'画面死着'」——
/// 自动往安静叙述切片上补运镜是**改观感**的行为，得用户先看数字再拍板。
/// 哪天有人"顺手"把 `applied` 改成 true，或让这个接口真的去补运镜，这条要红。
#[tokio::test]
async fn pacing_report_is_readable_and_stays_unapplied() {
    let t = test_app().await;
    let clip = t.root.join("output/clips/clip-23");
    std::fs::create_dir_all(clip.join("data")).unwrap();
    std::fs::write(clip.join("polish.toml"), "[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n").unwrap();

    // ① 没有报告（= 画面本来就很密）⇒ `null`，**不是**空对象。
    //    两者语义不同："没算过/很密" vs "算过、有 0 段"。
    let q = format!("dir={}&preview=1", clip.to_string_lossy().replace('\\', "/"));
    let res = t.call(t.req("GET", &format!("/polish/project?{q}")).body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let v: serde_json::Value = body_json(res).await;
    assert_eq!(v.get("_pacing"), Some(&serde_json::Value::Null), "没有报告必须是 null");

    // ② 放一份真报告 ⇒ 逐字段读得到，且 `applied` 仍是 false。
    std::fs::write(
        clip.join("data/pacing.json"),
        r#"{"min_gap_secs":6.0,"count":1,"total_secs":38.0,"longest_secs":38.0,"applied":false,
            "gaps":[{"start":2.0,"end":40.0,"duration":38.0,"suggest_at":21.0,
                     "suggest_zoom_tier":"mid","suggest_duration_secs":2.5}]}"#,
    )
    .unwrap();
    let res = t.call(t.req("GET", &format!("/polish/project?{q}")).body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let v: serde_json::Value = body_json(res).await;
    let p = v.get("_pacing").expect("要读得到 _pacing");
    assert_eq!(p.get("count").and_then(|x| x.as_u64()), Some(1), "{p}");
    assert_eq!(p.get("applied").and_then(|x| x.as_bool()), Some(false), "applied 必须原样是 false");
    let gaps = p.get("gaps").and_then(|x| x.as_array()).expect("gaps");
    assert_eq!(gaps[0].get("start").and_then(|x| x.as_f64()), Some(2.0));
    assert_eq!(gaps[0].get("suggest_at").and_then(|x| x.as_f64()), Some(21.0));

    // ③ **坏文件按"没有"处理**，不许 500：这些是派生数据，为一个统计数字
    //    连编辑器都打不开，代价完全不成比例。
    std::fs::write(clip.join("data/pacing.json"), "{ 这不是 JSON").unwrap();
    let res = t.call(t.req("GET", &format!("/polish/project?{q}")).body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK, "坏报告不许让整个工程读不出来");
    let v: serde_json::Value = body_json(res).await;
    assert_eq!(v.get("_pacing"), Some(&serde_json::Value::Null), "坏文件按没有处理");
}

}