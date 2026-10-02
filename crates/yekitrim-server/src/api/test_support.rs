//! `api::test_support`：`api::*` 各模块单测共用的 HTTP 夹具。
//!
//! `TestApp` 建的是**整张路由表**（`crate::api::router`），这些用例因此是
//! 接口级的：它们验证的是「请求打进来会发生什么」，不是某个函数的单元行为。
//! 所以夹具只有一份，测试却按领域散在各个模块里（评审 §5.6）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::Request as HttpRequest;
use axum::response::Response;
use tower::util::ServiceExt;

use yekitrim_service::Workspace;

use super::*;
use super::{project_data::*, routes::*};

pub(crate) fn tmp_workspace(tag: &str) -> (Workspace, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!("vtrim-api-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("output")).unwrap();
    let ws = Workspace {
        config_file: root.join("vtrim.toml"),
        root_dir: root.clone(),
    };
    (ws, root)
}

pub(crate) fn put_transcript(ws: &Workspace, texts: &[&str]) -> ApiResult {
    let segments: Vec<serde_json::Value> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| {
            serde_json::json!({ "start": i as f64, "end": i as f64 + 1.0, "text": t })
        })
        .collect();
    data_transcript_put_impl(ws, &serde_json::json!({ "segments": segments }))
}

/// 日志行前缀（tracing 默认格式）里的时间戳，测试用。
pub(crate) fn now_stamp() -> String {
    "2026-09-15T07:24:55.118063Z".to_string()
}

/// 带一套独立账号账本 + 已登录会话的测试服务。
///
/// 账号库放在临时目录里（不经 `VTRIM_DATA_DIR` 环境变量：测试是并行跑的，
/// 改全局环境变量会互相踩）。登录本身也走真实 KDF 与真实会话表。
pub(crate) struct TestApp {
    pub(crate) app: Router,
    pub(crate) auth: Arc<crate::auth::AuthService>,
    /// 同一个 AppState（任务管理器在它里面）：用例可以直接造"占着资源"的任务来验互斥
    pub(crate) state: Arc<AppState>,
    pub(crate) _dir: tempfile::TempDir,
    /// 测试项目根目录（`output/clips` 就在它下面），给需要造/验文件的用例用
    pub(crate) root: PathBuf,
    pub(crate) sid: String,
    pub(crate) csrf: String,
}

pub(crate) async fn test_app_with_must_change(must_change: bool) -> TestApp {
    use crate::auth::{AuthService, Listening};
    let dir = tempfile::tempdir().unwrap();
    let auth = AuthService::open(dir.path().to_path_buf(), Listening::Loopback).unwrap();
    let hash = yekitrim_service::auth::hash_password("initial-pass-1234").unwrap();
    let uid = auth
        .db()
        .create_user("admin", &hash, "admin", must_change, 0)
        .unwrap();
    // 真项目目录 + 真 vtrim.toml：P2 之后所有接口都要经过项目作用域
    let root = dir.path().join("proj");
    std::fs::create_dir_all(root.join("output/clips")).unwrap();
    std::fs::write(root.join("a.mp4"), b"fake").unwrap();
    let cfg_path = root.join("vtrim.toml");
    std::fs::write(&cfg_path, "input_file = \"a.mp4\"\n").unwrap();
    let project_id = auth
        .db()
        .create_project(uid, "proj", None, "local", Some("测试"), &root, &cfg_path, None)
        .unwrap();
    assert_eq!(project_id, 1, "测试项目固定为 id=1");
    let bundle = auth
        .login("admin", "initial-pass-1234", None, None)
        .expect("测试账号应能登录");
    let state = Arc::new(AppState::with_auth(auth.clone()));
    TestApp {
        app: router(state.clone()),
        auth,
        state,
        _dir: dir,
        root,
        sid: bundle.session_id,
        csrf: bundle.csrf,
    }
}

pub(crate) async fn test_app() -> TestApp {
    test_app_with_must_change(false).await
}

impl TestApp {
    pub(crate) fn cookie(&self) -> String {
        format!(
            "{}={}; {}={}",
            yekitrim_service::auth::COOKIE_SID,
            self.sid,
            yekitrim_service::auth::COOKIE_CSRF,
            self.csrf
        )
    }

    /// 构造已带会话 cookie 的请求；写请求自动带 CSRF 头。
    pub(crate) fn req(&self, method: &str, uri: &str) -> axum::http::request::Builder {
        let b = HttpRequest::builder()
            .method(method)
            .uri(uri)
            .header(axum::http::header::COOKIE, self.cookie());
        if matches!(method, "GET" | "HEAD" | "OPTIONS") {
            b
        } else {
            b.header(yekitrim_service::auth::CSRF_HEADER, self.csrf.clone())
        }
    }

    pub(crate) async fn call(&self, req: axum::http::Request<Body>) -> Response {
        self.app.clone().oneshot(req).await.unwrap()
    }

    /// 用**另一个账号**的会话构造请求（跨租户回归用 —— `req()` 固定用测试自己的 admin 会话）。
    pub(crate) fn req_as(
        &self,
        sid: &str,
        csrf: &str,
        method: &str,
        uri: &str,
    ) -> axum::http::request::Builder {
        let b = HttpRequest::builder()
            .method(method)
            .uri(uri)
            .header(
                axum::http::header::COOKIE,
                format!(
                    "{}={}; {}={}",
                    yekitrim_service::auth::COOKIE_SID,
                    sid,
                    yekitrim_service::auth::COOKIE_CSRF,
                    csrf
                ),
            );
        if matches!(method, "GET" | "HEAD" | "OPTIONS") {
            b
        } else {
            b.header(yekitrim_service::auth::CSRF_HEADER, csrf.to_string())
        }
    }
}

pub(crate) async fn body_json(res: Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(res.into_body(), 256 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

/// `GET /api/projects` 的项目 id 列表。
///
/// 断言"某项目在/不在列表里"时**一定要先确认响应是好的**：接口报错会返回 `{error:…}`，
/// 拿不到 `projects` 数组，直接断言"不在里面"就会白通过。
pub(crate) async fn project_ids(t: &TestApp) -> Vec<u64> {
    let res = t.call(t.req("GET", "/projects").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK, "读取项目列表失败");
    let json = body_json(res).await;
    json.get("projects")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("响应里没有 projects 数组: {json}"))
        .iter()
        .filter_map(|p| p.get("id").and_then(|i| i.as_u64()))
        .collect()
}

/// 造一个切片目录：`clip-NN/clip.json`（原片区间）+ 假 clip.mp4（video_exists 才为真）。
pub(crate) fn write_clip_dir(clips: &Path, rank: u32, start: f64, end: f64, srt_cues: usize, dm_cues: Option<usize>) {
    let dir = clips.join(format!("clip-{:0>2}", rank));
    std::fs::create_dir_all(&dir).unwrap();
    let meta = serde_json::json!({
        "duration": end - start,
        "original_start": start,
        "original_end": end,
        "rank": rank,
        "title": format!("片段{rank}"),
    });
    std::fs::write(dir.join("clip.json"), meta.to_string()).unwrap();
    std::fs::write(dir.join("clip.mp4"), b"fake").unwrap();
    let srt = |n: usize| (0..n).map(|i| format!("{i}\n00:00:0{i}.000 --> 00:00:0{}.500\n字幕{i}\n", i + 1)).collect::<String>();
    std::fs::write(dir.join("clip.srt"), srt(srt_cues)).unwrap();
    if let Some(n) = dm_cues {
        std::fs::write(dir.join("clip.dm.srt"), srt(n)).unwrap();
    }
}
