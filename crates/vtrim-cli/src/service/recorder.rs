//! 录播姬（BililiveRecorder）HTTP API 客户端：房间增删查。
//!
//! 端点是它 WebUI 自己用的那套（源码 `BililiveRecorder.Web/Api/RoomController.cs`）：
//! - `GET  {base}/api/room`            列出房间（`streaming` / `recording` / `autoRecord`）
//! - `POST {base}/api/room`            `{"roomId": 123, "autoRecord": true}` 加房间
//! - `DELETE {base}/api/room/{roomId}` 删房间（不删已录文件）
//! - `GET  {base}/api/version`         版本（对接自检用，路径按 `/swagger/` 校准）
//!
//! 认证是 HTTP Basic（容器里设 `BREC_HTTP_BASIC_USER/PASS` 时）；录播姬 2.11 起在没设密码时
//! 只接受看起来来自局域网的请求，所以**推荐与 V-Trim 同机/同内网部署**。

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

use vtrim_core::config::RecorderConfig;

/// 一个房间的实时状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecorderRoom {
    /// 直播间号（录播姬给的 long id）
    pub room_id: i64,
    /// 房间标题（拿不到就是空串）
    pub title: String,
    /// 是否正在直播
    pub streaming: bool,
    /// 是否正在录制
    pub recording: bool,
    /// 录播姬侧是否自动录制（加房间时我们固定 true）
    pub auto_record: bool,
}

#[derive(Debug, Deserialize)]
struct RoomDto {
    #[serde(rename = "roomId", alias = "RoomId")]
    room_id: i64,
    #[serde(default, alias = "Name", alias = "name")]
    name: Option<String>,
    #[serde(default, alias = "Title", alias = "title")]
    title: Option<String>,
    #[serde(default, rename = "streaming", alias = "Streaming")]
    streaming: bool,
    #[serde(default, rename = "recording", alias = "Recording")]
    recording: bool,
    #[serde(default, rename = "autoRecord", alias = "AutoRecord")]
    auto_record: bool,
}

impl From<RoomDto> for RecorderRoom {
    fn from(d: RoomDto) -> Self {
        Self {
            room_id: d.room_id,
            title: d.title.filter(|s| !s.trim().is_empty()).or(d.name).unwrap_or_default(),
            streaming: d.streaming,
            recording: d.recording,
            auto_record: d.auto_record,
        }
    }
}

pub struct RecorderClient {
    base: String,
    http: reqwest::Client,
}

impl RecorderClient {
    /// `api_base` 为空 → `None`（= 未对接录播姬，用户端只能走"待分配池"兜底）。
    pub fn from_config(cfg: &RecorderConfig) -> Option<Self> {
        let base = cfg.api_base.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return None;
        }
        let mut headers = reqwest::header::HeaderMap::new();
        if !cfg.api_user.is_empty() {
            let mut v = reqwest::header::HeaderValue::from_str(&basic_auth(&cfg.api_user, &cfg.api_pass))
                .unwrap_or_else(|_| reqwest::header::HeaderValue::from_static(""));
            v.set_sensitive(true);
            headers.insert(reqwest::header::AUTHORIZATION, v);
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .default_headers(headers)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Some(Self { base, http })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// 把 HTTP 响应变成「成功 JSON / 带正文的 Err」，错误文案能直接给用户看。
    async fn json<T: serde::de::DeserializeOwned>(&self, resp: reqwest::Response, what: &str) -> Result<T> {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let brief = text.trim();
            let brief = if brief.len() > 200 { &brief[..200] } else { brief };
            anyhow::bail!("录播姬{what}失败（HTTP {}）{}", status.as_u16(), if brief.is_empty() { String::new() } else { format!(": {brief}") });
        }
        serde_json::from_str::<T>(&text).with_context(|| format!("录播姬{what}返回的不是预期 JSON: {}", &text[..text.len().min(200)]))
    }

    pub async fn list_rooms(&self) -> Result<Vec<RecorderRoom>> {
        let resp = self
            .http
            .get(format!("{}/api/room", self.base))
            .send()
            .await
            .with_context(|| format!("连不上录播姬（{}），检查 api_base / 端口 / Basic 账号", self.base))?;
        let rooms: Vec<RoomDto> = self.json(resp, "读取房间列表").await?;
        Ok(rooms.into_iter().map(RecorderRoom::from).collect())
    }

    /// 加房间（`autoRecord = true`：录播姬侧开播即录）。
    pub async fn add_room(&self, room_id: i64) -> Result<RecorderRoom> {
        let resp = self
            .http
            .post(format!("{}/api/room", self.base))
            .json(&serde_json::json!({ "roomId": room_id, "autoRecord": true }))
            .send()
            .await
            .with_context(|| format!("连不上录播姬（{}），检查 api_base / 端口 / Basic 账号", self.base))?;
        let room: RoomDto = self.json(resp, "添加房间").await?;
        Ok(room.into())
    }

    /// 删房间（录像文件保留）。
    pub async fn remove_room(&self, room_id: i64) -> Result<()> {
        let resp = self
            .http
            .delete(format!("{}/api/room/{room_id}", self.base))
            .send()
            .await
            .with_context(|| format!("连不上录播姬（{}），检查 api_base / 端口 / Basic 账号", self.base))?;
        let _: serde_json::Value = self.json(resp, "删除房间").await?;
        Ok(())
    }

    /// 版本字符串（对接自检；拿不到就报错，由调用方决定怎么呈现）。
    pub async fn version(&self) -> Result<String> {
        let resp = self
            .http
            .get(format!("{}/api/version", self.base))
            .send()
            .await
            .with_context(|| format!("连不上录播姬（{}）", self.base))?;
        let v: serde_json::Value = self.json(resp, "读取版本").await?;
        Ok(v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string()))
    }
}

fn basic_auth(user: &str, pass: &str) -> String {
    use base64::Engine as _;
    let raw = format!("{user}:{pass}");
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(raw))
}

/// 从录播姬的**目录名**里解析房间号。
///
/// 录播姬的文件夹名模板是用户可配的，默认形如 `<房间号>-<主播名>`（实测 `26320007-耶斯yeki`），
/// 也可能就是纯房间号；模板把名字放前面时形如 `某主播-26320007`。所以：
/// 1. 先取"首个非数字之前的那段数字"（覆盖默认模板与纯号）；
/// 2. 没有前导数字时，退回"按分隔符切开、找全数字的那一段"。
///
/// ⚠️ 这是**兜底扫描 / 录像列表**判定归属的唯一依据。以前直接拿整个目录名当房间号去查
/// `channel_map`，于是 `26320007-耶斯yeki` 永远匹配不上 `26320007` —— 扫到的录像
/// 全部被判成"没登记"、掉进待分配池（实测踩过）。
pub fn room_id_from_dir_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let leading: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
    if let Some(id) = normalize_room_id(&leading) {
        return Some(id);
    }
    name.split(['-', '_', ' ', '.', '('])
        .find_map(normalize_room_id)
}

/// 纯数字串 → 规范化的房间号（与 [`parse_room_id`] 同一个取值范围）。
fn normalize_room_id(digits: &str) -> Option<String> {
    if digits.is_empty() {
        return None;
    }
    let id: i64 = digits.parse().ok()?;
    if !(1..=99_999_999).contains(&id) {
        return None;
    }
    // 去掉前导零，保证与 `channel_map` 里存的写法一致
    Some(id.to_string())
}

/// 从一条**相对路径**里找出它属于哪个房间（逐段套 [`room_id_from_dir_name`]）。
///
/// 逐段而不是只看上一层目录：录播姬的模板可能带日期子目录（`<房间号>-<名>/2026-09/…`）。
pub fn room_id_from_rel_path(rel: &str) -> Option<String> {
    rel.split(['/', '\\'])
        .filter(|seg| !seg.is_empty())
        .find_map(room_id_from_dir_name)
}

/// 从用户输入里解析直播间号：接受纯数字、`live.bilibili.com/123`、带参数的完整链接。
///
/// 只认「bilibili 域名里的第一段数字」或纯数字 —— 不做网络解析（长短号由录播姬自己认）。
pub fn parse_room_id(input: &str) -> Result<i64> {
    let s = input.trim();
    if s.is_empty() {
        anyhow::bail!("请填写直播间号或链接");
    }
    let digits: Option<&str> = if s.chars().all(|c| c.is_ascii_digit()) {
        Some(s)
    } else {
        // 链接形态：取 `live.bilibili.com/` 之后、非数字之前的那一段
        let lower = s.to_ascii_lowercase();
        let host = lower
            .find("bilibili.com")
            .map(|i| i + "bilibili.com".len())
            .ok_or_else(|| anyhow::anyhow!("没认出直播间号：填数字或 https://live.bilibili.com/<号> 都行"))?;
        let rest = &s[host..];
        let start = rest
            .find(|c: char| c.is_ascii_digit())
            .ok_or_else(|| anyhow::anyhow!("链接里没有直播间号: {input}"))?;
        let tail = &rest[start..];
        let end = tail.find(|c: char| !c.is_ascii_digit()).unwrap_or(tail.len());
        Some(&tail[..end])
    };
    let Some(d) = digits else {
        anyhow::bail!("没认出直播间号：填数字或 https://live.bilibili.com/<号> 都行");
    };
    let id: i64 = d.parse().with_context(|| format!("直播间号不合法: {d}"))?;
    if !(1..=99_999_999).contains(&id) {
        anyhow::bail!("直播间号超出范围: {id}");
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_room_id_accepts_common_forms() {
        assert_eq!(parse_room_id("12345").unwrap(), 12345);
        assert_eq!(parse_room_id("  12345 ").unwrap(), 12345);
        assert_eq!(parse_room_id("https://live.bilibili.com/12345").unwrap(), 12345);
        assert_eq!(parse_room_id("https://live.bilibili.com/12345?broadcast_type=0&is_room_feed=1").unwrap(), 12345);
        assert_eq!(parse_room_id("http://live.bilibili.com/12345/").unwrap(), 12345);
        // 带协议之外的前缀也能认（用户可能粘整段分享文本）
        assert_eq!(parse_room_id("https://www.bilibili.com/blackboard/live/area.html?room=12345").unwrap(), 12345);
        assert_eq!(parse_room_id("live.bilibili.com/777").unwrap(), 777);
    }

    #[test]
    fn parse_room_id_rejects_junk() {
        for bad in ["", "   ", "abc", "https://live.bilibili.com/", "https://example.com/12345", "0", "999999999999"] {
            assert!(parse_room_id(bad).is_err(), "{bad:?} 不该被接受");
        }
    }

    #[test]
    fn from_config_requires_api_base() {
        let mut cfg = RecorderConfig::default();
        assert!(RecorderClient::from_config(&cfg).is_none(), "空 api_base = 未对接");
        cfg.api_base = "http://127.0.0.1:2356/".into();
        let c = RecorderClient::from_config(&cfg).expect("有 api_base 就该建出来");
        assert_eq!(c.base(), "http://127.0.0.1:2356", "尾部斜杠要吃掉");
    }

    /// 对着一个"假录播姬"跑一遍：Basic 头带上了、body 是 `{roomId, autoRecord}`、
    /// 响应解析成 RecorderRoom（字段名按它的 DTO 给，容错别名也顺带验一下）。
    #[tokio::test]
    async fn client_talks_to_a_fake_recorder() {
        use axum::extract::Path;
        use axum::routing::{delete, get};
        use axum::{Json, Router};

        let seen_auth = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let seen_body = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let auth_slot = seen_auth.clone();
        let body_slot = seen_body.clone();
        let app = Router::new()
            .route(
                "/api/room",
                get(|| async {
                    Json(serde_json::json!([
                        { "roomId": 12345, "title": "某主播", "streaming": true, "recording": true, "autoRecord": true }
                    ]))
                })
                .post(move |headers: axum::http::HeaderMap, body: String| {
                    let auth_slot = auth_slot.clone();
                    let body_slot = body_slot.clone();
                    async move {
                        *auth_slot.lock().unwrap() = headers
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default()
                            .to_string();
                        *body_slot.lock().unwrap() = body;
                        Json(serde_json::json!({ "roomId": 777, "name": "新房间", "streaming": false, "recording": false, "autoRecord": true }))
                    }
                }),
            )
            .route("/api/room/:id", delete(|Path(id): Path<i64>| async move { Json(serde_json::json!({ "roomId": id })) }))
            .route("/api/version", get(|| async { Json(serde_json::json!("2.15.0")) }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let mut cfg = RecorderConfig::default();
        cfg.api_base = format!("http://{addr}");
        cfg.api_user = "admin".into();
        cfg.api_pass = "secret".into();
        let client = RecorderClient::from_config(&cfg).unwrap();

        let rooms = client.list_rooms().await.unwrap();
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].room_id, 12345);
        assert_eq!(rooms[0].title, "某主播");
        assert!(rooms[0].streaming && rooms[0].recording && rooms[0].auto_record);

        let added = client.add_room(777).await.unwrap();
        assert_eq!(added.room_id, 777);
        assert_eq!(added.title, "新房间", "title 空时回落到 name");
        assert_eq!(seen_auth.lock().unwrap().as_str(), "Basic YWRtaW46c2VjcmV0", "Basic 头要带上");
        let sent: serde_json::Value = serde_json::from_str(&seen_body.lock().unwrap()).unwrap();
        assert_eq!(sent["roomId"], 777);
        assert_eq!(sent["autoRecord"], true, "加房间必须开自动录制");

        client.remove_room(777).await.unwrap();
        assert_eq!(client.version().await.unwrap(), "2.15.0");
    }

    #[test]
    fn basic_auth_header_shape() {
        // base64("admin:secret") —— 录播姬容器用 BREC_HTTP_BASIC_USER/PASS 设的同一组
        assert_eq!(basic_auth("admin", "secret"), "Basic YWRtaW46c2VjcmV0");
        assert_eq!(basic_auth("u", ""), "Basic dTo=");
    }

    /// 目录名 → 房间号：这是兜底扫描归属的唯一依据，错了就会把录像判成"没登记"。
    ///
    /// 回归钉子：以前直接拿整个目录名（`26320007-耶斯yeki`）当房间号查 `channel_map`，
    /// 永远匹配不上 `26320007`，扫到的录像全掉进待分配池（真实踩过）。
    #[test]
    fn room_id_from_dir_name_handles_real_folder_templates() {
        // 录播姬默认模板：<房间号>-<主播名>
        assert_eq!(room_id_from_dir_name("26320007-耶斯yeki").as_deref(), Some("26320007"));
        // 纯号
        assert_eq!(room_id_from_dir_name("26320007").as_deref(), Some("26320007"));
        // 其它分隔符 / 带括号后缀
        assert_eq!(room_id_from_dir_name("26320007_主播").as_deref(), Some("26320007"));
        assert_eq!(room_id_from_dir_name("26320007(录播)").as_deref(), Some("26320007"));
        // 模板把名字放前面 → 退回"全数字的一段"
        assert_eq!(room_id_from_dir_name("耶斯-26320007").as_deref(), Some("26320007"));
        // 前导零规范化，与 channel_map 的写法一致
        assert_eq!(room_id_from_dir_name("0026320007-x").as_deref(), Some("26320007"));
        // 认不出来的：不瞎猜（返回 None，调用方保持原行为）
        assert_eq!(room_id_from_dir_name("recordings"), None);
        assert_eq!(room_id_from_dir_name(""), None);
        assert_eq!(room_id_from_dir_name("0"), None, "0 不在合法房间号范围");
        assert_eq!(room_id_from_dir_name("1234567890"), None, "超出范围");
    }

    #[test]
    fn room_id_from_rel_path_walks_segments() {
        assert_eq!(room_id_from_rel_path("26320007-耶斯yeki/rec.flv").as_deref(), Some("26320007"));
        // 模板带日期子目录：任意一层命中即可
        assert_eq!(
            room_id_from_rel_path("26320007-耶斯yeki/2026-09/rec.flv").as_deref(),
            Some("26320007")
        );
        // 反斜杠写法（Windows 上 rel 可能是 `a\b`）
        assert_eq!(room_id_from_rel_path("26320007-x\\rec.flv").as_deref(), Some("26320007"));
        // 扁平布局（文件直接躺在 root、文件名以房间号开头）同样算命中
        assert_eq!(room_id_from_rel_path("26320007.flv").as_deref(), Some("26320007"));
        // 跟数字无关的路径 → None（调用方保持原有行为，不瞎认领）
        assert_eq!(room_id_from_rel_path("misc/rec.flv"), None);
        assert_eq!(room_id_from_rel_path("rec.flv"), None);
    }
}
