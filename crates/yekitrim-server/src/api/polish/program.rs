//! `api::polish::program`：**起精修任务**的六个入口（init / llm / llm-segment / process / render / cover）。

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use crate::model::AppState;

use super::*;


/// `POST /api/polish/init` / `process` 的请求体：`dir` = **项目内相对目录**
/// （越界由 `scope::enforce_scope` 挡，绝对路径与存在性由 `validated_project_dir` 挡）。
///
/// ⚠️ `dir` **必填**：空目录会让 `cmd_init` 在 server 进程的 CWD 里写出 polish.toml / data / sfx
/// （见 `validated_project_dir` 的注释），所以这里不给默认值 —— 老前端从来都带它。
#[derive(serde::Deserialize)]
pub(super) struct PolishDirBody {
    pub dir: String,
}


/// `POST /api/polish/render` 的请求体。
#[derive(serde::Deserialize)]
pub(super) struct PolishRenderBody {
    pub dir: String,
    /// 输出文件名（项目内相对路径）。空串 = 跟随工程 `[project] output`（缺省 `vtuber-clip.mp4`）。
    #[serde(default)]
    pub output_file: Option<String>,
    /// 出片通道：**白名单只有 `dhampir`**，认不出的值由 handler 回 400（不用 enum：
    /// 报错文案要把可用值列出来，且 `chrome` 那条腿已删除要能明确说清）。
    #[serde(default)]
    pub renderer: Option<String>,
}


/// `POST /api/polish/cover` 的请求体。`renderer` 只**透传**（白名单在 `cover.rs`，别抄第二份）。
#[derive(serde::Deserialize)]
pub(super) struct PolishCoverBody {
    pub dir: String,
    #[serde(default)]
    pub renderer: Option<String>,
}


pub(crate) async fn polish_init(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let PolishDirBody { dir } = parse_body(body)?;
    polish_spawn(&state, crate::auth::owner_of(&user), scope_pid(pid), "init", dir, PolishJobOpts::default()).await
}


/// LLM 剪辑（整片）。
///
/// `full`（默认 **true**）= 全部重做：清掉断点续传缓存、重新分段 + 逐段生成。
/// 用户点这颗按钮的意图就是"重做一遍"；半吊子的续传反而会让事件看起来被换乱
/// （2026-09-20 用户报的「续传把所有事件删了只留一个 auto」）。
/// 想省调用请用**单段重做**（`/api/polish/llm-segment`）。
pub(crate) async fn polish_llm(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    // 缺省 true：宁可重做，也不要静默续传
    let full = body.get("full").and_then(|v| v.as_bool()).unwrap_or(true);
    polish_spawn(&state, crate::auth::owner_of(&user), scope_pid(pid), "llm", dir, PolishJobOpts { full, ..Default::default() }).await
}


/// 只重做某一段的事件（编辑器：时间轴 → 选中分段 → 「只重做本段事件」）。
pub(crate) async fn polish_llm_segment(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let index = body
        .get("index")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少分段下标 index"))? as usize;
    polish_spawn(
        &state,
        crate::auth::owner_of(&user),
        scope_pid(pid),
        "llm_segment",
        dir,
        PolishJobOpts { segment_index: Some(index), ..Default::default() },
    )
    .await
}


pub(crate) async fn polish_process(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let PolishDirBody { dir } = parse_body(body)?;
    polish_spawn(&state, crate::auth::owner_of(&user), scope_pid(pid), "process", dir, PolishJobOpts::default()).await
}


pub(crate) async fn polish_render(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let PolishRenderBody { dir, output_file, renderer } = parse_body(body)?;
    let path = validated_project_dir(&dir)?;
    if !{
        let __p = path.join("polish.toml");
        // 同步 stat 不能留在 async worker 上（api/mod.rs:44-51 ✓）：
        // 工程常在慢盘/网络盘上，一次 exists() 就要几百 ms ✗
        tokio::task::spawn_blocking(move || __p.exists())
            .await
            .unwrap_or(false)
    } { return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish")); }
    // 输出文件名**不在这里写**：任务可能被排到后面，等它真开跑时再落到 polish.toml
    // （见 polish_spawn 的 render 分支）—— 否则排队期间别的任务/用户改动会被这次预写覆盖。
    let output_file = output_file.filter(|s| !s.is_empty());
    // 出片通道：**白名单**（不给就跟随 CLI 的默认值 —— 现在也只有底座 dhampir 一个值）。
    // ⚠️ 非法值回 **400**，不静默当默认：那会让人以为"选了别的通道却在跑底座"，
    // 而两版成片的差别只有对着看才发现（与 CLI 侧"认不出就报错"同一条口径）。
    // ⚠️ `chrome` 也要落进 400：那条本仓 headless Chrome 的兜底腿已于 2026-10-01 整条删除，
    // 但还会有人照老文档/老脚本传它 —— 白名单是最先能给出明确答复的地方。
    let renderer = match renderer.as_deref() {
        None | Some("") => None,
        Some(r @ "dhampir") => Some(r.to_string()),
        Some(other) => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                &format!(
                    "认不出的出片通道 `{other}`（现在只有 dhampir —— \
                     本仓 headless Chrome 那条兜底腿已于 2026-10-01 整条删除）"
                ),
            ))
        }
    };
    // ⚠️ 「版本数」（§9 多版本成片）**已整条删除**（2026-10-18 用户口径：没有理由）。
    //    它与 `unique_output_path` 的 `_n` 后缀**无关**：后者是「同名不覆盖」，保留。
    // 配额预检（review B7/D14）：成片是产出大户 —— 已超配额的普通账号建任务前就 409。
    super::quota_gate(&state, &user, scope_pid(pid)).await?;
    polish_spawn(
        &state,
        crate::auth::owner_of(&user),
        scope_pid(pid),
        "render",
        dir,
        PolishJobOpts { output_file, renderer, ..Default::default() },
    )
    .await
}


pub(crate) async fn polish_cover(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let PolishCoverBody { dir, renderer } = parse_body(body)?;
    // 通道由**前端**给（现在只有 `takumi`）；这里只透传，取值口径在 `cover.rs`（只一份）——
    // 传 `chrome` 会由那里**明确报错**（那条截图腿已删除），这一层不抄第二份白名单。
    let renderer = renderer.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    // 配额预检（review B7/D14）：与 polish_render 同一道闸（封面 jpg 相对小，但口径要一致）。
    super::quota_gate(&state, &user, scope_pid(pid)).await?;
    polish_spawn(&state, crate::auth::owner_of(&user), scope_pid(pid), "cover", dir, PolishJobOpts { renderer, ..Default::default() }).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;
    use axum::body::Body;

    // ⚠️ 这些用例是**按原字节搬来**的（含原本在文件顶层、缩进为 0 的那批）：
    //    统一重排缩进会动到多行字符串字面量，判据会因此悄悄变。


    /// 出片通道是**白名单**：认不出的值必须 **400**，**不许静默当默认** ——
    /// 那会让人以为"选了别的通道却在跑底座"，而两版成片的差别只有对着看才发现。
    ///
    /// ⚠️ 只打 **400 那条路**：合法值会真的起一个渲染任务（几分钟 + 要底座 + 要 ffmpeg），
    /// 那不是接口级单测该干的事（真跑一次出片的验收见 `crates/yekitrim-polish/src/render/AGENTS.md`）。
    #[tokio::test]
    async fn 出片通道认不出就_400_不静默当默认() {
        let t = test_app().await;
        let clip = t.root.join("output/clips/chan");
        std::fs::create_dir_all(&clip).unwrap();
        let cfg = yekitrim_polish::project::ProjectFile::example();
        std::fs::write(clip.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

        let body = serde_json::json!({ "dir": clip.to_string_lossy(), "renderer": "chrom" }).to_string();
        let res = t
            .call(
                t.req("POST", "/polish/render")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        let status = res.status();
        let detail = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&detail);
        assert_eq!(status, StatusCode::BAD_REQUEST, "认不出的通道必须 400；响应：{text}");
        assert!(text.contains("dhampir"), "报错要把可用值列出来：{text}");
    }


#[test]
fn project_dir_validation_rejects_empty_and_missing() {
    assert!(validated_project_dir("").is_err());
    assert!(validated_project_dir("   ").is_err());
    let missing = std::env::temp_dir().join("yekitrim-dir-does-not-exist-xyz");
    assert!(validated_project_dir(&missing).is_err());
    assert!(validated_project_dir(std::env::temp_dir()).is_ok());
    assert!(validated_project_dir(std::env::temp_dir().to_string_lossy().to_string()).is_ok());
}


/// 三个"路径类 body 字段"都不能把项目外的文件拉进来（普通用户）。
///
/// 回归钉子：`cover-background` 的 `source` 与 `cover-config` 的 `cover.image` 此前完全
/// 没做归属校验（`scope.rs` 只归一化 `dir` / `output_file`，而模块注释却声称覆盖了
/// `source`）。后果不是"多读一个文件"：`source` 指向 `<数据目录>/.yekitrim/llm.toml`
/// 就能把**全局 LLM API key** 复制进自己的项目，再用 `/api/media/<自己的项目>/…` 读回来；
/// 同样能拿到账号账本 `cloud/yekitrim.db`（明文邀请码 + 全部项目真实路径）。
#[tokio::test]
async fn polish_path_fields_cannot_read_outside_files() {
    let t = test_app().await;

    // 受害者（admin 的项目）里的一个"机密"文件
    let secret = t.root.join("output").join("secret.txt");
    std::fs::write(&secret, b"api_key = 'sk-real-secret'").unwrap();

    // 普通用户 + 他自己的项目目录（含 clip-01 与 polish.toml，够走完前置校验）
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
    let uid = t.auth.db().create_user("plain_paths", &hash, "user", false, 0).unwrap();
    let my_root = t.root.join("plain-proj");
    let my_clip = my_root.join("output/clips/clip-01");
    std::fs::create_dir_all(&my_clip).unwrap();
    std::fs::write(
        my_clip.join("polish.toml"),
        "[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n",
    )
    .unwrap();
    let my_own_bg = my_root.join("bg.jpg");
    std::fs::write(&my_own_bg, b"not a secret").unwrap();
    let my_cfg = my_root.join("yekitrim.toml");
    std::fs::write(&my_cfg, "input_file = \"a.mp4\"\n").unwrap();
    let my_pid = t
        .auth
        .db()
        .create_project(uid, "plain", None, "local", Some("测试"), &my_root, &my_cfg, None)
        .unwrap();
    let b = t.auth.login("plain_paths", "pass-12345678", Some("10.1.1.1"), None).unwrap();
    let (sid, csrf) = (b.session_id.clone(), b.csrf.clone());

    let clip = my_clip.to_string_lossy().to_string();

    // ① cover-background：项目外的 source → 404，且**不许**复制出文件
    let body = serde_json::json!({ "dir": clip, "source": secret.to_string_lossy() }).to_string();
    let res = t
        .call(
            t.req_as(&sid, &csrf, "POST", "/polish/cover-background")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "项目外 source 必须 404");
    assert!(
        !my_clip.join("_cover_bg.jpg").exists(),
        "被拒的请求不得留下复制出来的文件"
    );
    // 正对照：自己项目内的文件照常可用（证明不是"一律拒绝"）
    let body = serde_json::json!({ "dir": clip, "source": my_own_bg.to_string_lossy() }).to_string();
    let res = t
        .call(
            t.req_as(&sid, &csrf, "POST", "/polish/cover-background")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "自己项目内的背景图应可用");

    // ② cover-config：cover.image 指向项目外 → 404
    let body = serde_json::json!({ "dir": clip, "cover": { "image": secret.to_string_lossy() } })
        .to_string();
    let res = t
        .call(
            t.req_as(&sid, &csrf, "POST", "/polish/cover-config")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "项目外 cover.image 必须 404");
    let cfg = yekitrim_polish::project::load_project(&my_clip).unwrap();
    assert!(cfg.cover.image.is_empty(), "被拒的请求不得写进 polish.toml");
    // 正对照：自己项目内的图能存进去（证明 404 来自归属校验，不是别的校验在挡）
    let body = serde_json::json!({ "dir": clip, "cover": { "image": my_own_bg.to_string_lossy() } })
        .to_string();
    let res = t
        .call(
            t.req_as(&sid, &csrf, "POST", "/polish/cover-config")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "自己项目内的图应能存进配置");
    assert!(
        !yekitrim_polish::project::load_project(&my_clip).unwrap().cover.image.is_empty(),
        "正对照应当真的写进 polish.toml"
    );

    // ③ project-data/config：绝对的 input_file / output_dir → 400
    for patch in [
        serde_json::json!({ "project": { "input_file": secret.to_string_lossy() } }),
        serde_json::json!({ "cutting": { "output_dir": t.root.to_string_lossy() } }),
    ] {
        let body = serde_json::json!({ "patch": patch }).to_string();
        let res = t
            .call(
                t.req_as(&sid, &csrf, "PUT", &format!("/project-data/config?project={my_pid}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "绝对路径键必须 400：{patch}");
    }
    let saved = tokio::task::spawn_blocking({ let __p = my_cfg.clone(); move || std::fs::read_to_string(__p) }).await.unwrap_or_else(|_| Err(std::io::Error::other("读取任务崩了"))).unwrap();
    assert!(!saved.contains("secret.txt"), "被拒的配置不得落盘：{saved}");
    // 正对照：项目内相对路径照常能存
    let body = serde_json::json!({ "patch": { "project": { "input_file": "a.mp4" } } }).to_string();
    let res = t
        .call(
            t.req_as(&sid, &csrf, "PUT", &format!("/project-data/config?project={my_pid}"))
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "项目内相对路径应能保存");
}


/// polish 的 `dir` 只能落在本人项目内：项目外绝对路径 → 404。
#[tokio::test]
async fn polish_dir_must_be_inside_own_project() {
    let t = test_app().await;
    let outside = std::env::temp_dir().join("yekitrim-not-my-project");
    std::fs::create_dir_all(&outside).unwrap();
    let body = serde_json::json!({ "dir": outside.to_string_lossy() }).to_string();
    let res = t
        .call(
            t.req("POST", "/polish/init")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "项目外目录必须 404");
    let _ = std::fs::remove_dir_all(&outside);
}


// ===================== T9：请求体强类型化（缺字段 / 类型错 → 400 且**点名字段**）=====================

/// 把一次拒绝拆成 `(状态码, error 文案)`。
///
/// ⚠️ 结构体层的判据**故意不走 HTTP**：`/polish/*` 外面还有 `scope::enforce_scope`，它先按
/// `dir` 判归属 —— `dir` 给数字/空串会先被它回 **404**，那样测到的是中间件，而不是"结构体
/// 把字段类型/必填性收紧了"。HTTP 那一条另有用例（valid dir + 错类型）证明 handler 真的用它。
#[cfg(test)]
fn t9_reject<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> (StatusCode, String) {
    match parse_body::<T>(v) {
        Ok(_) => panic!("这个 body 应当被拒"),
        Err((status, body)) => (
            status,
            body.0.get("error").and_then(|e| e.as_str()).unwrap_or("").to_string(),
        ),
    }
}


/// `PolishDirBody`（`/polish/init` 与 `/polish/process` 共用一份结构体）。
///
/// 判据：缺 `dir` / `dir` 类型错 **都** 400 且报错**点名 dir**（不是 axum 的裸 422）；
/// 多传的陌生键**照旧被忽略**（宽松解析）。
#[tokio::test]
async fn polish_dir_body_is_typed_and_names_the_field() {
    use crate::api::test_support::{call_json, test_app};

    // ① 缺字段：结构体层
    let (status, msg) = t9_reject::<PolishDirBody>(serde_json::json!({}));
    assert_eq!(status, StatusCode::BAD_REQUEST, "缺 dir 必须 400：{msg}");
    assert!(msg.contains("missing field"), "要是 serde 的缺字段提示：{msg}");
    assert!(msg.contains("dir"), "报错要点名 dir：{msg}");

    // ② 类型错：结构体层
    let (status, msg) = t9_reject::<PolishDirBody>(serde_json::json!({ "dir": 5 }));
    assert_eq!(status, StatusCode::BAD_REQUEST, "dir 类型错必须 400：{msg}");
    assert!(msg.contains("dir"), "报错要点名 dir：{msg}");

    // ③ 宽松解析：多传一个没人认识的键，解析照样过
    assert!(
        parse_body::<PolishDirBody>(serde_json::json!({ "dir": "output/clips/x", "junk": [1, 2] })).is_ok(),
        "陌生键必须被忽略（不 deny_unknown_fields）"
    );

    // ④ HTTP 面：空 body 也必须是 400 + 点名字段（`{}` 不触发 scope，能真走到 handler）
    let t = test_app().await;
    for uri in ["/polish/init", "/polish/process"] {
        let (status, text) = call_json(&t, "POST", uri, serde_json::json!({})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri} 缺 dir 必须 400：{text}");
        assert!(text.contains("dir"), "{uri} 报错要点名字段 dir：{text}");
    }
}


/// `PolishRenderBody`：`dir` 必填；`output_file` / `renderer` 可选。
///
/// ⚠️ 三个字段都必须在**解析阶段**就 400 —— 等走到 `validated_project_dir` 之后才拦的话，
/// 文件名/通道写错会变成一个"已经开始渲染的任务"，用户看不到错在哪。
#[tokio::test]
async fn polish_render_body_is_typed_and_names_the_field() {
    use crate::api::test_support::{call_json, test_app};

    let (status, msg) = t9_reject::<PolishRenderBody>(serde_json::json!({}));
    assert_eq!(status, StatusCode::BAD_REQUEST, "缺 dir 必须 400：{msg}");
    assert!(msg.contains("dir"), "报错要点名 dir：{msg}");

    let (status, msg) = t9_reject::<PolishRenderBody>(serde_json::json!({ "dir": "x", "output_file": 5 }));
    assert_eq!(status, StatusCode::BAD_REQUEST, "output_file 类型错必须 400：{msg}");
    assert!(msg.contains("output_file"), "报错要点名 output_file：{msg}");

    let (status, msg) = t9_reject::<PolishRenderBody>(serde_json::json!({ "dir": "x", "renderer": 5 }));
    assert_eq!(status, StatusCode::BAD_REQUEST, "renderer 类型错必须 400：{msg}");
    assert!(msg.contains("renderer"), "报错要点名 renderer：{msg}");

    // HTTP 面（valid dir 才过得去 scope）：handler 真的用了这个结构体
    let t = test_app().await;
    let clip = t.root.join("output/clips/t9-render");
    std::fs::create_dir_all(&clip).unwrap();
    let (status, text) = call_json(
        &t,
        "POST",
        "/polish/render",
        serde_json::json!({ "dir": clip.to_string_lossy(), "renderer": 5 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "renderer 类型错必须 400：{text}");
    assert!(text.contains("renderer"), "报错要点名 renderer：{text}");
}


/// `PolishCoverBody`：`dir` 必填、`renderer` 可选（只透传给 `cover.rs` 的白名单）。
#[tokio::test]
async fn polish_cover_body_is_typed_and_names_the_field() {
    use crate::api::test_support::{call_json, test_app};

    let (status, msg) = t9_reject::<PolishCoverBody>(serde_json::json!({}));
    assert_eq!(status, StatusCode::BAD_REQUEST, "缺 dir 必须 400：{msg}");
    assert!(msg.contains("dir"), "报错要点名 dir：{msg}");

    let (status, msg) = t9_reject::<PolishCoverBody>(serde_json::json!({ "dir": "x", "renderer": [1] }));
    assert_eq!(status, StatusCode::BAD_REQUEST, "renderer 类型错必须 400：{msg}");
    assert!(msg.contains("renderer"), "报错要点名 renderer：{msg}");

    // HTTP 面
    let t = test_app().await;
    let clip = t.root.join("output/clips/t9-cover");
    std::fs::create_dir_all(&clip).unwrap();
    let (status, text) = call_json(
        &t,
        "POST",
        "/polish/cover",
        serde_json::json!({ "dir": clip.to_string_lossy(), "renderer": [1] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "renderer 类型错必须 400：{text}");
    assert!(text.contains("renderer"), "报错要点名 renderer：{text}");
}

}