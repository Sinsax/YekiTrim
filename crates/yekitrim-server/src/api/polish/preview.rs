//! `api::polish::preview`：预览端口的 LRU 与两条预览入口（DOM 预览 / 封面预览）。

use std::path::Path;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use crate::model::AppState;

use super::*;


/// 取（或启动）该工程目录的预览服务端口，按 `dir|kind` 复用。
///
/// `RenderServer` 起的监听无法关闭：旧实现每次 `/polish/preview`、
/// `/polish/cover-preview` 都新起一个，端口与后台任务会无限泄漏。
/// 服务端在每次请求时按磁盘配置重建 HTML，所以复用不会端出过期内容。
pub(super) async fn preview_port_for(state: &AppState, dir: &Path, kind: &str) -> anyhow::Result<u16> {
    let key = format!("{}|{kind}", dir.display());
    let cached = |key: &str| -> Option<u16> {
        state
            .preview_servers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .copied()
    };
    if let Some(port) = cached(&key) {
        return Ok(port);
    }
    // 同目录串行：并发首访不要同时起两个监听
    let _guard = yekitrim_core::filelock::lock_path(&dir.join(format!(".yekitrim-preview-{kind}"))).await;
    if let Some(port) = cached(&key) {
        return Ok(port);
    }
    let mut server = yekitrim_polish::render::server::RenderServer::new(dir.to_path_buf());
    let port = if kind == "cover" {
        server.start_cover().await?
    } else {
        server.start().await?
    };
    state
        .preview_servers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, port);
    Ok(port)
}


pub(crate) async fn polish_preview(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
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
    // 启动预览服务（不阻塞，返回 URL 供前端 iframe）
    let config = yekitrim_polish::project::load_project(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    if !config.processed {
        // 隐式 process 只在这个片段目录空闲时才跑：**别的**片段/项目的任务不该挡住它
        // （以前只要有任何任务在跑就跳过），同目录有任务才真的会撞。
        if state
            .jobs
            .busy_with(&crate::scope_key::polish_keys(None, &path))
            .await
            .is_some()
        {
            tracing::info!("该片段有任务在运行，跳过预览前的隐式 process: {}", path.display());
        } else {
            let _ = yekitrim_polish::process::cmd_process(&path).await;
        }
    }
    yekitrim_polish::process::ensure_assets(&path).await;
    let port = preview_port_for(&state, &path, "video")
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("预览启动失败: {e:#}")))?;
    let video_url = format!("http://127.0.0.1:{}/", port);
    // 前端需要通过 API 代理访问本机 127.0.0.1 随机端口；这里直接返回绝对 URL（浏览器同源可访问）
    ok(serde_json::json!({ "ok": true, "video_url": video_url, "cover_url": video_url }))
}


pub(crate) async fn polish_cover_preview(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
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
    // 缺关键帧时现场截一帧（ffmpeg，可能数百 ms~数秒）→ 整段下 blocking 池
    let cover_path = path.clone();
    let got_frame = blocking(move || {
        let config = yekitrim_polish::project::load_project(&cover_path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        let resolved = yekitrim_polish::render::resolve_paths(&cover_path, &config);
        let cover_image = cover_path.join("_cover_frame.jpg");
        if !cover_image.exists() {
            let duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
            let frame_time = (duration * config.cover.frame_pos).max(0.1);
            let frame_out = yekitrim_core::process::cmd("ffmpeg")
                .args(["-ss", &frame_time.to_string(), "-i", &resolved.video,
                       "-vframes", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "3", "-"])
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
                .output().map(|o| o.stdout);
            if let Ok(frame_data) = frame_out {
                if !frame_data.is_empty() {
                    std::fs::write(&cover_image, frame_data).ok();
                }
            }
        }
        Ok(cover_image.exists())
    })
    .await?;
    if !got_frame {
        return Err(err(StatusCode::BAD_REQUEST, "无法截取封面帧"));
    }
    // 封面服务按 dir|cover 复用；HTML 每次请求按当前 polish.toml 重建，改完设计刷新即可
    let port = preview_port_for(&state, &path, "cover")
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("封面预览启动失败: {e:#}")))?;
    let url = format!("http://127.0.0.1:{}/", port);
    ok(serde_json::json!({ "ok": true, "cover_url": url }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;
    use axum::body::Body;

    // ⚠️ 这些用例是**按原字节搬来**的（含原本在文件顶层、缩进为 0 的那批）：
    //    统一重排缩进会动到多行字符串字面量，判据会因此悄悄变。


    /// **`preview=1` 要把"预览该读哪份视频"交给前端**（2026-11-21，局域网提速）。
    ///
    /// 在此之前 DOM 那条腿永远读**源** `clip.mp4`，`_proxy_720.mp4` 只喂给底座 —— 于是拖时间轴 /
    /// 跳片段要按**原码率**取 Range（局域网上一卡就是几秒）。这条钉住"有新鲜代理就交出去、没有就
    /// `null`"，而且与底座那条腿用**同一个判定**（`proxy::preview_video_for`，含"源比代理新 ⇒ 作废"）。
    #[tokio::test]
    async fn preview_reports_the_proxy_video_for_the_dom_leg() {
        let t = test_app().await;
        let clip = t.root.join("output/clips/proxyvis");
        std::fs::create_dir_all(&clip).unwrap();
        let cfg = yekitrim_polish::project::ProjectFile::example();
        std::fs::write(clip.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();
        let dir = clip.to_string_lossy().into_owned();

        let ask = |dir: String| {
            let t = &t;
            async move {
                let req = t
                    .req("GET", &format!("/polish/project?dir={dir}&preview=1"))
                    .body(Body::empty())
                    .unwrap();
                let res = t.call(req).await;
                assert_eq!(res.status(), StatusCode::OK, "preview=1 应当 200");
                body_json(res).await
            }
        };

        // ① 没有代理 ⇒ `null`（前端回落源，不能凭空编一个文件名）
        let v = ask(dir.clone()).await;
        assert!(
            v.get("_preview_video").is_some(),
            "preview=1 必须给出 `_preview_video`（哪怕是 null）：{v}"
        );
        assert!(
            v["_preview_video"].is_null(),
            "没有代理时必须回 null，实际：{}",
            v["_preview_video"]
        );

        // ② 有新鲜代理 ⇒ 交出去（前端据此改读 720p）
        std::fs::write(clip.join("_proxy_720.mp4"), b"not-a-real-video").unwrap();
        let v = ask(dir).await;
        assert_eq!(
            v["_preview_video"].as_str(),
            Some("_proxy_720.mp4"),
            "有新鲜的 `_proxy_720.mp4` 时 preview=1 必须把它交给 DOM 腿（否则它白生成了）"
        );
    }

}