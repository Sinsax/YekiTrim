//! 录播姬 / B站 采集接入的共享底座（多用户服务 P3，设计稿 §4）。
//!
//! 这个目录原来是一个 3,235 行的 `ingest.rs`：录播姬的「待分配池」语义与 B站的「提交即下载」
//! 语义住在同一个文件里（`handle_event` 与 `bilibili_submit` 隔了 2000 行）。拆开之后按语义分文件：
//!
//! | 文件 | 语义 |
//! |---|---|
//! | `webhook` | 录播姬 webhook 的**会话流程**：验 token → 去重 → 累积文件 → 推 `closing` |
//! | `webhook_event` | payload 解析与归一化（官方 PascalCase / biliLive-tools 两种方言） |
//! | `webhook_path` | 容器路径 → 本机路径、产物 / 伴生弹幕 / 标题、建工程 |
//! | `records` | 录制记录、`/rooms/:id/recordings`、单文件导入 |
//! | `channel_map` | 房间 → 归属映射（待分配池的指派 / 重试 / 兜底扫描） |
//! | `config` | 采集配置的读视图、保存通道、令牌轮换 |
//! | `patch` | `build_ingest_patch`：PATCH「缺席 vs 空」语义的唯一实现 |
//! | `bilibili` | B站链接通道（resolve / submit） |
//! | `downloader` | yt-dlp 安装与状态 |
//!
//! ⚠️ **`webhook*` 与 `bilibili` 不许出现对方的领域词**（`scripts/check_module_boundaries.py`
//! 的成对判据，文件级）：录播姬是「无身份、进待分配池、靠空闲窗口完结」，B站是「用户主动提交、
//! 有身份、进程退出即完结」—— 语义混在一起会互相踩，这正是当年那条禁令想防的事。
//!
//! 本文件只放**被两个以上子模块用到**的东西（`err` / `ok` / `off` / `field*` + 类型别名）与对外
//! 再导出；handler 一律留在各自的领域文件里。

use axum::Json;
use axum::http::StatusCode;

mod webhook;
mod webhook_event;
mod webhook_path;
mod records;
mod channel_map;
mod config;
mod patch;
mod bilibili;
mod downloader;

/// handler 的统一返回面（与拆分前的 ingest.rs 逐字相同）。
pub(crate) type AppError = (StatusCode, Json<serde_json::Value>);
pub(crate) type ApiResult<T = serde_json::Value> = Result<Json<T>, AppError>;

// ---- 对外出口（`api/routes.rs` 的路由表、`api/rooms.rs` 的路径换算）----
pub use bilibili::{bilibili_resolve, bilibili_submit};
pub use channel_map::{assign, channel_map_get, channel_map_put, retry, scan_now};
pub use config::{config_view, put_config, rotate_token};
pub use downloader::{downloader_install, downloader_status};
pub use records::{import_recording, records, room_recordings};
pub use webhook::{custom_webhook, webhook};
// 这两个的可见性本来就只有 crate（`pub(crate)`），所以出口也只能是 `pub(crate) use`。
pub(crate) use records::owned_room;
pub(crate) use webhook_path::map_webhook_path;

/// 测试骨架：一张临时账号库 + 一条 webhook 事件的构造器（子模块的测试共用）。
#[cfg(test)]
pub(crate) mod testutil {
    use yekitrim_service::db::Db;

    pub(crate) fn db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("yekitrim.db")).unwrap();
        (dir, db)
    }

    pub(crate) fn ev(kind: &str, id: &str, session: &str, extra: serde_json::Value) -> serde_json::Value {
        let mut data = serde_json::json!({
            "SessionId": session,
            "RoomId": 23058,
            "RelativePath": "23058/rec.flv",
            "FileSize": 1234,
        });
        if let (Some(dst), Some(src)) = (data.as_object_mut(), extra.as_object()) {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
        serde_json::json!({ "EventType": kind, "EventId": id, "EventData": data })
    }
}

fn err(status: StatusCode, msg: impl Into<String>) -> AppError {
    (
        status,
        Json(serde_json::json!({ "ok": false, "error": msg.into() })),
    )
}

fn ok<T: serde::Serialize>(v: T) -> ApiResult<T> {
    Ok(Json(v))
}

/// 把同步 IO 搬出 async worker（与 `api::blocking` / `scope::off` 同款，只是用本文件自己的 `AppError`）。
///
/// ⚠️ 本文件的接口里有**未认证**入口（录播姬 webhook），读配置、写审计、事件落库全是同步 IO
/// —— 项目/数据目录在网络盘上时一次 `read_to_string` 就是几百毫秒，十几个 webhook 就能把
/// tokio worker 占满。细则见 `src/api/AGENTS.md`「同步 IO 纪律」。
async fn off<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r,
        Err(e) => Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("阻塞任务失败: {e}"),
        )),
    }
}


/// 录像目录探针的进程级缓存槽（与 [`root_probe_cached`] / [`last_root_probe`] 共用）。
type RootProbeSlot = std::sync::Mutex<
    Option<(std::time::Instant, String, bool, yekitrim_service::recorder::RootProbe)>,
>;
static ROOT_PROBE_CACHE: std::sync::OnceLock<RootProbeSlot> = std::sync::OnceLock::new();

fn root_probe_slot() -> &'static RootProbeSlot {
    ROOT_PROBE_CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

/// 录像目录探针（60s 内复用上次结果）。
///
/// ⚠️ **GET 接口绝不允许现场探测**：设置页会反复读配置，而一个死 UNC 路径的 `metadata`
/// 能挂几十秒 —— 那等于把页面拖死。只有「测试连接 / 保存配置」这两条显式路径会真探测，
/// 并把结果留在这里；读侧只拿缓存（`None` = 还没测过）。
pub(crate) fn root_probe_cached(
    root: &str,
    want_writable: bool,
) -> yekitrim_service::recorder::RootProbe {
    const TTL: std::time::Duration = std::time::Duration::from_secs(60);
    let key = root.trim().to_string();
    {
        let g = root_probe_slot().lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, path, ww, probe)) = g.as_ref() {
            if *ww == want_writable && *path == key && at.elapsed() < TTL {
                return probe.clone();
            }
        }
    }
    let probe = yekitrim_service::recorder::probe_recorder_root(&key, want_writable);
    *root_probe_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some((
        std::time::Instant::now(),
        key,
        want_writable,
        probe.clone(),
    ));
    probe
}

/// 读上一次校验结论（**不探测、不看 TTL**）：配置视图用它显示「上次校验」。
pub(crate) fn last_root_probe(root: &str) -> Option<yekitrim_service::recorder::RootProbe> {
    let key = root.trim();
    let g = root_probe_slot().lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref()
        .filter(|(_, path, _, _)| path == key)
        .map(|(_, _, _, probe)| probe.clone())
}
/// 大小写不敏感的多别名取值（`SessionId` / `session_id` / `sessionId` 都认）。
///
/// 官方的 `RoomId` / `ShortId` 是**数字**，所以这里同时接受字符串与数字。
fn field<'a>(v: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    let obj = v.as_object()?;
    for (k, val) in obj {
        for want in keys {
            if k.eq_ignore_ascii_case(want) {
                if let Some(s) = val.as_str() {
                    return Some(s);
                }
            }
        }
    }
    None
}

/// 与 [`field`] 相同，但把数字也转成字符串（`RoomId: 23058`）。
fn field_text(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    if let Some(s) = field(v, keys) {
        return Some(s.to_string());
    }
    let obj = v.as_object()?;
    for (k, val) in obj {
        for want in keys {
            if k.eq_ignore_ascii_case(want) {
                if let Some(n) = val.as_i64() {
                    return Some(n.to_string());
                }
                if let Some(f) = val.as_f64() {
                    return Some(format!("{}", f as i64));
                }
            }
        }
    }
    None
}

fn field_u64(v: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    let obj = v.as_object()?;
    for (k, val) in obj {
        for want in keys {
            if k.eq_ignore_ascii_case(want) {
                if let Some(n) = val.as_u64() {
                    return Some(n);
                }
                if let Some(s) = val.as_str() {
                    if let Ok(n) = s.parse::<u64>() {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

fn field_f64(v: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    let obj = v.as_object()?;
    for (k, val) in obj {
        for want in keys {
            if k.eq_ignore_ascii_case(want) {
                if let Some(n) = val.as_f64() {
                    return Some(n);
                }
                if let Some(s) = val.as_str() {
                    if let Ok(n) = s.parse::<f64>() {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_lookup_is_case_insensitive_with_aliases() {
        let v = serde_json::json!({ "session_id": "abc", "FileSize": 42 });
        assert_eq!(field(&v, &["SessionId", "session_id"]), Some("abc"));
        assert_eq!(field_u64(&v, &["FileSize"]), Some(42));
        assert_eq!(field(&v, &["Missing"]), None);
    }
}
