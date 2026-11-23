//! 录播姬（BililiveRecorder）HTTP API 客户端：房间增删查。
//!
//! 端点是它 WebUI 自己用的那套（源码 `BililiveRecorder.Web/Api/RoomController.cs`）：
//! - `GET  {base}/api/room`            列出房间（`streaming` / `recording` / `autoRecord`）
//! - `POST {base}/api/room`            `{"roomId": 123, "autoRecord": true}` 加房间
//! - `DELETE {base}/api/room/{roomId}` 删房间（不删已录文件）
//! - `GET  {base}/api/version`         版本（对接自检用，路径按 `/swagger/` 校准）
//!
//! 认证是 HTTP Basic（容器里设 `BREC_HTTP_BASIC_USER/PASS` 时）；录播姬 2.11 起在没设密码时
//! 只接受看起来来自局域网的请求，所以**推荐与 YekiTrim 同机/同内网部署**。

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

use yekitrim_core::config::{IngestConfig, RecorderConfig};

/// 一个房间的实时状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecorderRoom {
    /// 直播间号（录播姬给的 long id）
    pub room_id: i64,
    /// 平台标识（录播姬固定 `Bilibili`；biliLive-tools 用它自己的 `providerId`）
    pub platform: String,
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
            platform: "Bilibili".into(),
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

/// biliLive-tools 的一条录像记录（`GET /record-history/list`）。
#[derive(Debug, Clone)]
pub struct RecordHistoryItem {
    pub title: String,
    /// 接口给的文件路径（容器内路径，例 `/app/video/Bilibili/…/x.flv`）
    pub video_file: String,
    pub duration_secs: u64,
    pub danma_num: u64,
    pub record_start_ms: i64,
}

/// biliLive-tools HTTP API 客户端（v3.x）。
///
/// 和录播姬**不是一套接口**：
/// - 鉴权是 `Authorization: <passkey>`（**原样，不是 Bearer**）
/// - 录制用户：`GET /recorder/list` → `payload.data[]`（`channelId`/`providerId`/`remarks`/`state`）
/// - 每场录像：`GET /record-history/list?room_id=&platform=&page=&pageSize=` → `data[].video_file`
/// - 文件浏览：`GET /files/list?path=`（本项目只读用不到）
pub struct BiliLiveToolsClient {
    base: String,
    http: reqwest::Client,
}

impl BiliLiveToolsClient {
    pub fn from_config(cfg: &RecorderConfig) -> Option<Self> {
        let base = cfg.api_base.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return None;
        }
        let mut headers = reqwest::header::HeaderMap::new();
        if !cfg.api_key.trim().is_empty() {
            let mut v = reqwest::header::HeaderValue::from_str(cfg.api_key.trim())
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

    async fn json<T: serde::de::DeserializeOwned>(
        &self,
        resp: reqwest::Response,
        what: &str,
    ) -> Result<T> {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let brief = text.trim();
            let brief = if brief.len() > 200 { &brief[..200] } else { brief };
            // 401/403 单独给一句能直接照做的提示（2026-11-24 实测踩坑）：最常见的原因不是"网络不通"，
            // 而是**用户抄的那把密钥不是实际生效的那把** —— biliLive-tools 的 passKey 优先读环境变量
            // BILILIVE_TOOLS_PASSKEY（容器里常设），而它的设置界面显示的是 appConfig.json 里那份，
            // 两者不一致时"从界面抄来的"永远 401。
            let hint = match status.as_u16() {
                401 | 403 => {
                    "\n  → 密钥不匹配：以 biliLive-tools 里**实际生效**的 passKey 为准 \
（容器设了 BILILIVE_TOOLS_PASSKEY 环境变量时，它优先于设置界面显示的那份）。\
填进「访问密钥」后点「测试连接」再确认一次。"
                }
                _ => "",
            };
            anyhow::bail!(
                "biliLive-tools{what}失败（HTTP {}）{}{hint}",
                status.as_u16(),
                if brief.is_empty() { String::new() } else { format!(": {brief}") }
            );
        }
        serde_json::from_str::<T>(&text)
            .with_context(|| format!("biliLive-tools{what}返回的不是预期 JSON: {}", &text[..text.len().min(200)]))
    }

    /// 录制用户（= 它的 recorder 列表）。`state` 只有 `recording` 才算正在录。
    pub async fn list_rooms(&self) -> Result<Vec<RecorderRoom>> {
        let resp = self
            .http
            .get(format!("{}/recorder/list", self.base))
            .send()
            .await
            .with_context(|| format!("连不上 biliLive-tools（{}），检查 api_base / 密钥", self.base))?;
        let dto: BltRecorderListResp = self.json(resp, "读取录制用户").await?;
        Ok(dto
            .payload
            .data
            .into_iter()
            .map(|r| RecorderRoom {
                room_id: r.channel_id.as_deref().unwrap_or("").parse().unwrap_or(0),
                platform: r
                    .provider_id
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| "Bilibili".into()),
                title: r.remarks.unwrap_or_default(),
                streaming: false,
                recording: r
                    .state
                    .as_deref()
                    .map(|s| s.eq_ignore_ascii_case("recording"))
                    .unwrap_or(false),
                auto_record: true,
            })
            .collect())
    }

    /// 某个房间的录像历史（新版在前）。`platform` = `Bilibili` / `DouYu` / ...
    pub async fn list_history(&self, room_id: &str, platform: &str) -> Result<Vec<RecordHistoryItem>> {
        let resp = self
            .http
            .get(format!("{}/record-history/list", self.base))
            .query(&[
                ("room_id", room_id),
                ("platform", platform),
                ("page", "1"),
                ("pageSize", "200"),
            ])
            .send()
            .await
            .with_context(|| format!("连不上 biliLive-tools（{}）", self.base))?;
        let dto: BltHistoryResp = self.json(resp, "读取录像历史").await?;
        if dto.code != 0 && dto.code != 200 {
            anyhow::bail!("biliLive-tools 读取录像历史失败：{}", dto.message.unwrap_or_default());
        }
        Ok(dto
            .data
            .into_iter()
            .map(|h| RecordHistoryItem {
                title: h.title.unwrap_or_default(),
                video_file: h.video_file.unwrap_or_default(),
                duration_secs: h.video_duration.unwrap_or(0),
                danma_num: h.danma_num.unwrap_or(0),
                record_start_ms: h.record_start_time.unwrap_or(0),
            })
            .collect())
    }

    /// 版本（自检用；biliLive-tools 没有 `/api/version`，读 `/config` 只证明"连通且鉴权过"）。
    pub async fn version(&self) -> Result<String> {
        let resp = self
            .http
            .get(format!("{}/config", self.base))
            .send()
            .await
            .with_context(|| format!("连不上 biliLive-tools（{}）", self.base))?;
        let v: serde_json::Value = self.json(resp, "读取配置").await?;
        Ok(v.get("version").and_then(|x| x.as_str()).unwrap_or("biliLive-tools").to_string())
    }
}

#[derive(Debug, Deserialize)]
struct BltRecorderListResp {
    payload: BltRecorderListPayload,
}
#[derive(Debug, Deserialize)]
struct BltRecorderListPayload {
    #[serde(default)]
    data: Vec<BltRecorderDto>,
}
#[derive(Debug, Deserialize)]
struct BltRecorderDto {
    // 这些字段在它那边可能是 null：一律 Option，缺/空都按"没给"处理
    #[serde(default, rename = "providerId")]
    provider_id: Option<String>,
    #[serde(default, rename = "channelId")]
    channel_id: Option<String>,
    #[serde(default)]
    remarks: Option<String>,
    #[serde(default)]
    state: Option<String>,
}
#[derive(Debug, Deserialize)]
struct BltHistoryResp {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    data: Vec<BltHistoryDto>,
}
#[derive(Debug, Deserialize)]
struct BltHistoryDto {
    #[serde(default)]
    title: Option<String>,
    #[serde(default, rename = "video_file")]
    video_file: Option<String>,
    #[serde(default, rename = "video_duration")]
    video_duration: Option<u64>,
    #[serde(default, rename = "danma_num")]
    danma_num: Option<u64>,
    #[serde(default, rename = "record_start_time")]
    record_start_time: Option<i64>,
}

/// 录制软件后端：按 `[ingest.recorder] kind` 选一套实现。
pub enum RecorderBackend {
    Bililive(RecorderClient),
    BiliLiveTools(BiliLiveToolsClient),
}

impl RecorderBackend {
    /// 录播模块**二选一**：`[ingest.recorder].kind` 决定用哪套 API（同一张配置表）。
    pub fn from_config(cfg: &IngestConfig) -> Option<Self> {
        match cfg.recorder.kind.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "none" => None,
            "bililive-tools" | "blt" => {
                BiliLiveToolsClient::from_config(&cfg.recorder).map(Self::BiliLiveTools)
            }
            _ => RecorderClient::from_config(&cfg.recorder).map(Self::Bililive),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Bililive(_) => "bililive",
            Self::BiliLiveTools(_) => "bililive-tools",
        }
    }

    pub fn is_bililive_tools(&self) -> bool {
        matches!(self, Self::BiliLiveTools(_))
    }

    pub fn base(&self) -> &str {
        match self {
            Self::Bililive(c) => c.base(),
            Self::BiliLiveTools(c) => c.base(),
        }
    }

    pub async fn list_rooms(&self) -> Result<Vec<RecorderRoom>> {
        match self {
            Self::Bililive(c) => c.list_rooms().await,
            Self::BiliLiveTools(c) => c.list_rooms().await,
        }
    }

    pub async fn version(&self) -> Result<String> {
        match self {
            Self::Bililive(c) => c.version().await,
            Self::BiliLiveTools(c) => c.version().await,
        }
    }

    pub async fn add_room(&self, room_id: i64) -> Result<RecorderRoom> {
        match self {
            Self::Bililive(c) => c.add_room(room_id).await,
            Self::BiliLiveTools(_) => anyhow::bail!("biliLive-tools 不支持在 YekiTrim 里加录制用户"),
        }
    }

    pub async fn remove_room(&self, room_id: i64) -> Result<()> {
        match self {
            Self::Bililive(c) => c.remove_room(room_id).await,
            Self::BiliLiveTools(_) => Ok(()),
        }
    }

    /// 仅 biliLive-tools 有意义：录播姬的录像归属靠目录名解析，不需要历史接口。
    pub async fn list_history(&self, room_id: &str, platform: &str) -> Result<Vec<RecordHistoryItem>> {
        match self {
            Self::BiliLiveTools(c) => c.list_history(room_id, platform).await,
            Self::Bililive(_) => Ok(Vec::new()),
        }
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

/// 从录播姬的**目录名**里取**主播名**（自动填充 `[detection].vtuber_name` 用，2026-11-22）。
///
/// 与 [`room_id_from_dir_name`] 同一张分隔符表：默认模板是 `<房间号>-<主播名>`
/// （实测 `26320007-耶斯yeki`），也可能是 `某主播-26320007`。规则 = **跳过纯数字段与空段**，
/// 取第一个非数字段 —— 所以日期目录（`2026-09-21`）与纯房间号都返回 `None`。
///
/// ⚠️ 取不到就**留空**（用户手填），别拿房间号去凑一个名字。
/// ⚠️ 这个名字**只用于产物标题**（见 `yekitrim_core::naming`），不参与目录命名。
pub fn streamer_name_from_dir_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    name.split(['-', '_', ' ', '.', '(', ')']).find_map(|seg| {
        let s = seg.trim();
        if s.is_empty() || s.chars().all(|c| c.is_ascii_digit()) {
            None
        } else {
            Some(s.to_string())
        }
    })
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

    /// 目录名里的**主播名**（自动填充项目级 `[detection].vtuber_name` 用，2026-11-22）。
    #[test]
    fn streamer_name_comes_from_the_recorder_dir_name() {
        assert_eq!(
            streamer_name_from_dir_name("26320007-耶斯yeki").as_deref(),
            Some("耶斯yeki"),
            "录播姬默认模板 `<房间号>-<主播名>`"
        );
        assert_eq!(
            streamer_name_from_dir_name("某主播-26320007").as_deref(),
            Some("某主播"),
            "模板把名字放前面时也要认"
        );
        // 拿不到就留空（绝不用房间号凑一个名字）
        assert_eq!(streamer_name_from_dir_name("26320007"), None);
        assert_eq!(streamer_name_from_dir_name("2026-09-21"), None, "日期目录不是名字");
        assert_eq!(streamer_name_from_dir_name(""), None);
        // 同一个目录名两件事都要能取到（房间号 + 名字）
        assert_eq!(
            room_id_from_dir_name("26320007-耶斯yeki").as_deref(),
            Some("26320007")
        );
    }

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
    /// 对着一个"假 biliLive-tools"跑一遍：`Authorization` 是**原样的密钥**（不是 Bearer）、
    /// 录制用户/录像历史的字段名按它 v3.x 的返回解析。
    #[tokio::test]
    async fn blt_client_reads_recorders_and_history() {
        use axum::extract::Query;
        use axum::routing::get;
        use axum::{Json, Router};
        use std::collections::HashMap;

        let seen_auth = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let auth_slot = seen_auth.clone();
        let app = Router::new()
            .route(
                "/recorder/list",
                get(move |headers: axum::http::HeaderMap| {
                    let auth_slot = auth_slot.clone();
                    async move {
                        *auth_slot.lock().unwrap() = headers
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default()
                            .to_string();
                        Json(serde_json::json!({
                            "payload": { "data": [
                                { "providerId": "Bilibili", "channelId": "237926", "remarks": "Sinsa", "state": "idle" }
                            ] }
                        }))
                    }
                }),
            )
            .route(
                "/record-history/list",
                get(|Query(q): Query<HashMap<String, String>>| async move {
                    assert_eq!(q.get("room_id").map(String::as_str), Some("237926"));
                    assert_eq!(q.get("platform").map(String::as_str), Some("Bilibili"));
                    Json(serde_json::json!({
                        "code": 200,
                        "data": [
                            {
                                "title": "测试",
                                "video_file": "/app/video/Bilibili/Sinsa/x.flv",
                                "video_duration": 253,
                                "danma_num": 7,
                                "record_start_time": 1779115900962i64
                            },
                            // 它那边的可选字段会是 null（真实数据里踩过：反序列化直接 500）
                            {
                                "title": null,
                                "video_file": "/app/video/Bilibili/Sinsa/y.flv",
                                "video_duration": null,
                                "danma_num": null,
                                "record_start_time": null
                            }
                        ]
                    }))
                }),
            )
            .route("/config", get(|| async { Json(serde_json::json!({ "version": "3.22.2" })) }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let cfg = RecorderConfig {
            api_base: format!("http://{addr}/"),
            api_key: "sekret-key".into(),
            ..Default::default()
        };
        let client = BiliLiveToolsClient::from_config(&cfg).unwrap();

        let rooms = client.list_rooms().await.unwrap();
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].room_id, 237926);
        assert_eq!(rooms[0].platform, "Bilibili");
        assert_eq!(rooms[0].title, "Sinsa");
        assert_eq!(*seen_auth.lock().unwrap(), "sekret-key", "Authorization 必须是原样密钥");

        let history = client.list_history("237926", "Bilibili").await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].video_file, "/app/video/Bilibili/Sinsa/x.flv");
        assert_eq!(history[0].duration_secs, 253);
        assert_eq!(history[0].danma_num, 7);
        // null 字段按 0/空串处理，不能整个列表反序列化失败
        assert_eq!(history[1].video_file, "/app/video/Bilibili/Sinsa/y.flv");
        assert_eq!(history[1].duration_secs, 0);
        assert_eq!(history[1].danma_num, 0);
        assert_eq!(history[1].title, "");

        assert_eq!(client.version().await.unwrap(), "3.22.2");
    }

    /// kind 选后端：默认仍是录播姬；`bililive-tools` 只在有 api_base 时建得出来。
    #[test]
    fn backend_kind_selects_client() {
        let mut cfg = IngestConfig::default();
        assert!(RecorderBackend::from_config(&cfg).is_none(), "两套都没配就不接");

        // 二选一：kind = bililive-tools
        cfg.recorder.kind = "bililive-tools".into();
        cfg.recorder.api_base = "http://127.0.0.1:18010".into();
        let b = RecorderBackend::from_config(&cfg).unwrap();
        assert_eq!(b.kind(), "bililive-tools");
        assert!(b.is_bililive_tools());

        // kind = bililive
        cfg.recorder.kind = "bililive".into();
        cfg.recorder.api_base = "http://127.0.0.1:2356".into();
        let b = RecorderBackend::from_config(&cfg).unwrap();
        assert_eq!(b.kind(), "bililive");
    }

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
