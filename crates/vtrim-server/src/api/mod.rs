//! `api` 模块根：拆分后留下的**共享底座**。
//!
//! - `AppError` / `ApiResult` / `err` / `ok`：全部 handler 的返回约定
//! - `blocking` / `block_on_here` / `spawn_job`：同步 IO 与任务的落点（见 AGENTS「同步 IO 绝不能留在 async 线程上」）
//! - `workspace_from_state` / 命令探测缓存：被多个领域模块复用
//!
//! 这里只放**被两个以上子模块用到**的东西；只服务一个领域的 helper 跟着那个模块走。

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use vtrim_service::Workspace;

use crate::model::AppState;

pub(crate) type AppError = (StatusCode, Json<serde_json::Value>);
pub(crate) type ApiResult<T = serde_json::Value> = Result<Json<T>, AppError>;

pub(crate) fn err(status: StatusCode, msg: impl Into<String>) -> AppError {
    (
        status,
        Json(serde_json::json!({ "ok": false, "error": msg.into() })),
    )
}

pub(crate) fn ok<T: serde::Serialize>(v: T) -> ApiResult<T> {
    Ok(Json(v))
}

/// 把**同步文件访问**挪出 async worker。
///
/// 项目目录经常在网络盘上（RaiDrive/WebDAV 挂载的录播目录）：盘被剪切/渲染的整段读取占满时，
/// 一次 `exists()`、一次读 `vtrim.toml` 就要几百毫秒到几秒。这类调用留在 worker 上做，
/// 十几个并发（前端一次页面加载就会打出一串）就能把 runtime 的 worker 全占住 ——
/// 表现是"任务在跑的时候整个服务器连新连接都建立不了"（用户实测）。
/// 同步 IO 一律走 blocking 线程池，worker 留给 accept 与轻量接口。
///
/// `pub(crate)`：兄弟模块（如 `stickers.rs` 解压上传的压缩包）也要用同一条通道。
pub(crate) async fn blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r,
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("阻塞任务失败: {e}"))),
    }
}

/// 在**当前线程**上把一个 future 跑完（自带一个 current-thread runtime）。
///
/// 只给任务专用线程 / blocking 线程用 —— 在 runtime worker 上调用它正是我们要避免的事。
pub(crate) fn block_on_here<F: std::future::Future>(fut: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("创建任务运行时失败");
    rt.block_on(fut)
}

/// 把任务体放到**专用线程**上跑（自带一个 current-thread runtime）。
///
/// 管线（转写 / 事件检测 / 剪切 / 精修 process）是一整段同步代码：跑在 async worker 上就是
/// 白占一个 worker 到它结束（实测一条 59s 的剪切 = 37s；转写是分钟级），任务一多 worker 就见底。
/// 任务线程与 HTTP 的 worker 彻底分开后，跑多少任务都不影响服务器收发请求。
///
/// **panic 兜底**：任务体的 `complete/fail` 在它自己的末尾，一旦中间 panic，Job 会永远停在
/// `running`、资源键永不释放 —— 同资源（项目/片段）的后续任务就全部排队饿死。
/// 这里接住 panic 并把它记成任务失败（管线里 `join().unwrap()`、下标越界这类都可能触发）。
pub(crate) fn spawn_job<JobBody, Fut>(
    jobs: Arc<crate::jobs::JobManager>,
    job_id: String,
    body: JobBody,
) where
    JobBody: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()>,
{
    // 线程起不来时也要给任务收尾（见下），所以失败分支得留着这两个句柄
    let fail_jobs = jobs.clone();
    let fail_id = job_id.clone();
    let spawned = std::thread::Builder::new().name("vtrim-job".into()).spawn(move || {
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block_on_here(body())));
        let Err(payload) = caught else { return };
        let msg = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "载荷不是字符串".into());
        tracing::error!("任务线程 panic（job={job_id}）：{msg}");
        // 任务体的 complete/fail 在它自己末尾，panic 就永远调不到 —— 这里替它收尾，
        // 否则 Job 停在 running、资源键不释放，同项目的后续任务会一直排队
        block_on_here(async move {
            jobs.fail(&job_id, format!("任务内部错误（panic）: {msg}")).await;
        });
    });
    if let Err(e) = spawned {
        // ⚠️ 不能只打日志：`create` 已经把这个 Job 标成 Running（或 Queued），
        // 没有工作协程就没有任何人会调 complete/fail —— 它会**永久**占着资源键与档位名额，
        // 同项目/同片段的后续任务全部排队饿死，而队列里只有一个"运行中"的任务什么都查不出来。
        // 兜底跟 panic 那条一样：替它把这个任务记成失败。**两条兜底缺一不可。**
        tracing::error!("启动任务线程失败（job={fail_id}）：{e}");
        block_on_here(async move {
            fail_jobs.fail(&fail_id, format!("启动任务线程失败: {e}")).await;
        });
    }
}

pub(crate) async fn workspace_from_state(state: &AppState) -> Workspace {
    state.workspace().await
}

/// 外部命令探测结果缓存（30s TTL）。
///
/// `/api/health` 会被前端在页面挂载与「环境检查」时调用，同步 spawn
/// ffmpeg/ffprobe/chrome（最多 7 个进程）会占住 tokio worker，故探测放
/// `spawn_blocking` 并缓存。
pub(crate) type ProbeCache = std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, Option<String>)>>;

pub(crate) fn probe_cache() -> &'static ProbeCache {
    static CACHE: std::sync::OnceLock<ProbeCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub(crate) fn cache_get(key: &str, ttl: Duration) -> Option<Option<String>> {
    let cache = probe_cache().lock().unwrap_or_else(|e| e.into_inner());
    cache
        .get(key)
        .filter(|(t, _)| t.elapsed() < ttl)
        .map(|(_, v)| v.clone())
}

pub(crate) fn cache_put(key: String, value: Option<String>) {
    let mut cache = probe_cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.insert(key, (std::time::Instant::now(), value));
}

pub(crate) fn command_available_blocking(cmd: &str) -> Option<String> {
    let output = std::process::Command::new(cmd).arg("-version").output().ok()?;
    if !output.status.success() { return None; }
    let first = String::from_utf8_lossy(&output.stdout).lines().next().map(|s| s.to_string()).unwrap_or_default();
    Some(first)
}

pub(crate) async fn command_available(cmd: &str) -> Option<String> {
    let key = format!("cmd:{cmd}");
    if let Some(cached) = cache_get(&key, Duration::from_secs(30)) {
        return cached;
    }
    let owned = cmd.to_string();
    let value = tokio::task::spawn_blocking(move || command_available_blocking(&owned))
        .await
        .ok()
        .flatten();
    cache_put(key, value.clone());
    value
}

pub(crate) fn chrome_available_blocking(custom: &str) -> Option<String> {
    let probe = |c: &str| -> Option<String> {
        if Path::new(c).exists() {
            return Some(c.to_string());
        }
        std::process::Command::new(c)
            .arg("--version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|_| c.to_string())
    };
    // 全局配置 [chrome] path 优先
    if !custom.is_empty() {
        return probe(custom);
    }
    let candidates = [
        "chrome",
        "google-chrome",
        "chromium",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ];
    candidates.iter().find_map(|c| probe(c))
}

pub(crate) async fn chrome_available(cfg: Option<&vtrim_core::config::Config>) -> bool {
    let custom = cfg.map(|c| c.chrome.path.trim()).unwrap_or("").to_string();
    let key = format!("chrome:{custom}");
    if let Some(cached) = cache_get(&key, Duration::from_secs(30)) {
        return cached.is_some();
    }
    let value = tokio::task::spawn_blocking(move || chrome_available_blocking(&custom))
        .await
        .ok()
        .flatten();
    let ok = value.is_some();
    cache_put(key, value);
    ok
}

/// `/api/health`：白名单接口（未登录也要能判断"服务是否活着/要不要登录"）。
///
/// §3.5.9：**不暴露任何路径**，也没有 `mode/auto_login/setup_required` 这类分支字段。
///
/// ⚠️ **账号数量与"是否仍在使用初始密码"不在这里**（曾经有过）——这是**未认证**接口，
/// 对远端说出来等于直接告诉对方"这台机器还在用初始密码，来打它"。
/// 两者都有各自的出口：账号列表 `/api/admin/users`（admin）、自己的改密状态 `/api/me`（已登录）、
/// 本地排查 `vtrim doctor`（直接读本机账本，信息更全）。
/// 顺带一个好处：`/api/health` 会被前端反复轮询，去掉这里两次同步 SQLite 查询后它不再碰磁盘。
pub(crate) async fn health(State(state): State<Arc<AppState>>, headers: axum::http::HeaderMap) -> ApiResult {
    // 能力位只看平台级配置（global-only）：health 是公开接口，不该泄露任何工程信息
    let cfg = vtrim_core::config::load_global_config();
    let llm = !cfg.llm.api_key.is_empty() && cfg.llm.api_key != "sk-your-api-key-here";
    ok(serde_json::json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "ffmpeg": command_available("ffmpeg").await,
        "ffprobe": command_available("ffprobe").await,
        "chrome": chrome_available(Some(&cfg)).await,
        "llm": llm,
        // 单帧真渲染（/api/polish/render-frame）只在启用该 feature 的构建里可用
        "wgpu_render": cfg!(feature = "wgpu-render"),
        // 采集就绪位（P3）：webhook 已启用 + 录播姬目录已配置，前端据此决定是否显示引导卡
        "ingest_ready": cfg.ingest.webhook_enabled && !cfg.ingest.recorder.root.trim().is_empty(),
        // 注册开关（登录页据此决定要不要显示「用邀请码注册」入口）
        "registration_enabled": cfg.auth.registration_enabled,
        // 本机/远端与是否走 HTTPS 反代：只影响本地排查提示，不含任何账号信息
        "listening": state.auth.listening().as_str(),
        "tls_terminated": crate::auth::tls_terminated(&headers),
    }))
}

pub(crate) mod routes;
pub(crate) mod auth;
pub(crate) mod invite;
pub(crate) mod logs;
pub(crate) mod rooms;
pub(crate) mod config;
pub(crate) mod projects;
pub(crate) mod project_data;
pub(crate) mod pipeline;
pub(crate) mod render;
pub(crate) mod polish;
pub(crate) mod core;

/// 单测共用的 HTTP 夹具（只在这些测试里编译）。
#[cfg(test)]
pub(crate) mod test_support;

/// 路由表的唯一入口（`lib.rs` 与路由单测都走 `crate::api::router`）。
pub use routes::router;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobClass, JobSpec};




/// 阻塞工作必须真的跑到 blocking 池上：worker 全程空转（这里用一个多线程 runtime
/// 只留 1 个 worker 来模拟"worker 被占满"，同步阻塞跑在 worker 上就会互相排队）。
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn blocking_helper_keeps_workers_free() {
    // 两个"重活"各睡 300ms：走 spawn_blocking 时它们并行（~300ms），
    // 若留在 worker 上就只能串行（~600ms）。
    let t0 = std::time::Instant::now();
    let (a, b) = tokio::join!(
        blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok::<_, AppError>(1)
        }),
        blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok::<_, AppError>(2)
        }),
    );
    let elapsed = t0.elapsed();
    assert_eq!((a.unwrap(), b.unwrap()), (1, 2));
    assert!(elapsed.as_millis() < 550, "两段阻塞工作应当是并行的，实测 {elapsed:?}");

    // worker 只有一个，但上面两段阻塞期间调度器仍然能跑别的任务
    let ticked = tokio::time::timeout(std::time::Duration::from_millis(200), async {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        true
    })
    .await
    .unwrap_or(false);
    assert!(ticked, "blocking 池占用期间 worker 仍应能推进其它任务");
}

/// 任务体 panic 也要把 Job 收尾（否则永远 running、资源键不释放、同项目后续任务全部排队）
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_job_marks_job_failed_on_panic() {
    let jobs = Arc::new(crate::jobs::JobManager::new());
    let handle = jobs
        .create(1, JobSpec::new("test_panic", "panic 测试", vec!["dir:x".into()], JobClass::Medium))
        .await;
    let id = handle.id.clone();
    spawn_job(jobs.clone(), id.clone(), move || async move {
        let _ = handle.wait_start().await;
        panic!("模拟管线里的 join().unwrap() / 下标越界");
    });
    let mut status = String::new();
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        if let Some(j) = jobs.get(&id).await {
            status = j.status.clone();
            if status == "failed" {
                break;
            }
        }
    }
    assert_eq!(status, "failed", "任务 panic 后应当被标记为 failed");
    // 资源键必须释放：同资源的新任务能立刻开工
    assert!(jobs.busy_with(&["dir:x".to_string()]).await.is_none(), "panic 的任务不应继续占着资源键");
}

/// 错误原样透传（handler 靠它把 4xx/5xx 文案给前端）
#[tokio::test]
async fn blocking_helper_passes_error_through() {
    let r = blocking(|| Err::<i32, _>(err(StatusCode::NOT_FOUND, "没有这条"))).await;
    match r {
        Err((status, Json(body))) => {
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(body["error"], "没有这条");
        }
        Ok(_) => panic!("应当把错误透传出去"),
    }
}
}
