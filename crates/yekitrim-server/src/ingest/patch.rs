//! `build_ingest_patch`：把设置页表单的 JSON 翻成 `patch_config_file` 的补丁。
//!
//! **PATCH 语义「键缺席 = 不改 / 空串空数组 = 清空」的唯一实现**（335 行）：别把它拆成两处判断 ——
//! 拆开就会有一边把「没写这个键」当成「清空」（实测：改个录像目录把口令抹了）。


use axum::http::StatusCode;

use super::*;
use super::config::check_projects_root;
use super::bilibili::check_quality;

/// 允许的自动入库终点（与 `RecorderConfig::auto_pipeline` 的取值一致）。
const AUTO_PIPELINES: [&str; 4] = ["none", "transcribe", "detect", "cut"];

/// 把 `PUT /api/admin/ingest` 的 body 变成"要合并进配置的字段" + 警告列表。
///
/// 抽成纯函数有两个理由：校验规则能直接单测（不需要起服务、也不碰真实全局配置），
/// 而且"哪些键会被写"这件事在一处看得完。
///
/// 返回值是 `(要并进 [ingest] 的字段, 要并进 [storage] 的字段, 警告)` ——
/// `[storage]` 那一份只有 `projects_root` 一个键（项目存放位置），
/// 它跟采集无关，但设置页把它摆在下载器卡里，所以走同一个保存端点。
///
/// 语义（表单友好，关键是让"清空"与"不改"分得开）：
/// - 键**缺席** = 保持原值；
/// - 字符串给 `""` = 显式清空（`api_pass` 同此：给空串才清掉，缺省则保留原密码；
///   `projects_root` 给空串 = 回默认的"程序目录下"）；
/// - `allow_ips` 给 `[]` = 清空白名单。
pub(super) fn build_ingest_patch(
    body: &serde_json::Value,
) -> Result<
    (
        serde_json::Map<String, serde_json::Value>,
        serde_json::Map<String, serde_json::Value>,
        Vec<String>,
    ),
    AppError,
> {
    let mut ingest = serde_json::Map::new();
    let mut storage = serde_json::Map::new();
    let mut warnings: Vec<String> = Vec::new();

    if let Some(v) = body.get("webhook_enabled") {
        let b = v
            .as_bool()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "webhook_enabled 必须是布尔值"))?;
        ingest.insert("webhook_enabled".into(), serde_json::json!(b));
    }

    if let Some(v) = body.get("allow_ips") {
        let arr = v
            .as_array()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "allow_ips 必须是字符串数组"))?;
        let mut ips: Vec<String> = Vec::new();
        for item in arr {
            let s = item.as_str().unwrap_or("").trim();
            if s.is_empty() {
                continue;
            }
            // 运行时是**精确字符串相等**（webhook 校验那段），所以只收 IP 字面量：
            // 放行 `192.168.1.0/24` 这种写法只会让来源被静默拦掉、还很难查。
            if s.parse::<std::net::IpAddr>().is_err() {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("allow_ips 只接受单个 IP（不支持网段/CIDR，也不接受主机名）：{s}"),
                ));
            }
            if !ips.iter().any(|x| x == s) {
                ips.push(s.to_string());
            }
        }
        ingest.insert("allow_ips".into(), serde_json::json!(ips));
    }

    if let Some(v) = body.get("proxy") {
        let raw = v
            .as_str()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "proxy 必须是字符串"))?
            .trim();
        // 允许清空（空串 = 直连）；非空则必须是带 scheme 的地址
        if !raw.is_empty()
            && !["http://", "https://", "socks5://", "socks5h://"]
                .iter()
                .any(|p| raw.starts_with(p))
        {
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!("proxy 要写成 http:// / socks5:// / socks5h:// 开头：{raw}"),
            ));
        }
        ingest.insert("proxy".into(), serde_json::json!(raw));
    }

    if let Some(blt) = body.get("bililive_tools") {
        let blt = blt
            .as_object()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "bililive_tools 必须是对象"))?;
        let mut out = serde_json::Map::new();
        for key in ["api_base", "root", "container_root"] {
            if let Some(v) = blt.get(key) {
                let raw = v
                    .as_str()
                    .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("bililive_tools.{key} 必须是字符串")))?
                    .trim();
                if key == "api_base"
                    && !raw.is_empty()
                    && !(raw.starts_with("http://") || raw.starts_with("https://"))
                {
                    return Err(err(
                        StatusCode::BAD_REQUEST,
                        format!("bililive_tools.api_base 需要以 http:// 或 https:// 开头：{raw}"),
                    ));
                }
                out.insert(key.into(), serde_json::json!(raw));
            }
        }
        if let Some(v) = blt.get("api_key") {
            let raw = v
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "bililive_tools.api_key 必须是字符串"))?;
            if !raw.trim().is_empty() {
                out.insert("api_key".into(), serde_json::json!(raw));
            }
        }
        if !out.is_empty() {
            ingest.insert("bililive_tools".into(), serde_json::Value::Object(out));
        }
    }

    if let Some(rec) = body.get("recorder") {
        let rec = rec
            .as_object()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "recorder 必须是对象"))?;
        let mut out = serde_json::Map::new();

        if let Some(v) = rec.get("kind") {
            let s = v
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "recorder.kind 必须是字符串"))?
                .trim();
            if !["bililive", "bililive-tools", "none"].contains(&s) {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("recorder.kind 只能是 bililive / bililive-tools / none：{s}"),
                ));
            }
            out.insert("kind".into(), serde_json::json!(s));
        }
        for key in ["root", "api_base", "api_user", "container_root"] {
            if let Some(v) = rec.get(key) {
                let s = v
                    .as_str()
                    .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("recorder.{key} 必须是字符串")))?
                    .trim();
                out.insert(key.into(), serde_json::json!(s));
            }
        }
        // 口令**不 trim**：首尾空格是合法字符，替用户"修"掉比留着更容易出怪问题
        if let Some(v) = rec.get("api_pass") {
            let s = v
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "recorder.api_pass 必须是字符串"))?;
            out.insert("api_pass".into(), serde_json::json!(s));
        }
        // biliLive-tools 的访问密钥同理：空串 = 不修改（前端只回 api_key_set 布尔）
        if let Some(v) = rec.get("api_key") {
            let s = v
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "recorder.api_key 必须是字符串"))?;
            if !s.trim().is_empty() {
                out.insert("api_key".into(), serde_json::json!(s));
            }
        }
        if let Some(v) = rec.get("auto_pipeline") {
            let s = v
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "recorder.auto_pipeline 必须是字符串"))?;
            if !AUTO_PIPELINES.contains(&s) {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("recorder.auto_pipeline 只能是 {} 之一：{s}", AUTO_PIPELINES.join(" / ")),
                ));
            }
            out.insert("auto_pipeline".into(), serde_json::json!(s));
        }
        if let Some(v) = rec.get("watch_interval_mins") {
            let n = v
                .as_u64()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "recorder.watch_interval_mins 必须是非负整数"))?;
            out.insert("watch_interval_mins".into(), serde_json::json!(n));
        }

        // 目录不可达**不拒绝**：网络盘/跨机部署时它可能只是暂时没挂上，直接 400 会让人
        // 连"先存下来"都做不到。改成回一条 warning，前端显示出来。
        //
        // ⚠️ 文案全部来自**同一个探针**（`probe_recorder_root`）：这里曾经只判 `is_dir()`，
        // 于是「填了对方机器上的 /rec」「盘符映射服务看不到」「只读共享」全都只说一句
        // "目录不可访问"，用户照着那句什么也修不了。
        if let Some(root) = out.get("root").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            let p = yekitrim_service::recorder::probe_recorder_root(root, true);
            match p.status {
                yekitrim_service::recorder::RootStatus::Ok => {
                    if p.entries == Some(0) {
                        warnings.push(format!(
                            "录像目录可读写，但里面没有录像（{root}）：确认共享的就是录播姬的输出目录。"
                        ));
                    }
                }
                _ => warnings.push(if p.hint.is_empty() {
                    format!("录像目录当前不可用（{root}）。")
                } else {
                    format!("录像目录当前不可用（{root}）：{}", p.hint)
                }),
            }
        }
        if let Some(base) = out.get("api_base").and_then(|v| v.as_str()) {
            if !base.is_empty() && !(base.starts_with("http://") || base.starts_with("https://")) {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("recorder.api_base 需要以 http:// 或 https:// 开头（例 http://127.0.0.1:2356）：{base}"),
                ));
            }
        }

        ingest.insert("recorder".into(), serde_json::Value::Object(out));
    }

    // 通用 webhook（`[ingest.webhook]` 子表）：root / container_root / auto_pipeline。
    // ⚠️ 它**必须**是子表（`IngestConfig::webhook`），与 downloader 那个字符串键相反。
    if let Some(wh) = body.get("webhook") {
        let wh = wh
            .as_object()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "webhook 必须是对象"))?;
        let mut out = serde_json::Map::new();
        for key in ["root", "container_root"] {
            if let Some(v) = wh.get(key) {
                let s = v
                    .as_str()
                    .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("webhook.{key} 必须是字符串")))?
                    .trim();
                out.insert(key.into(), serde_json::json!(s));
            }
        }
        if let Some(v) = wh.get("auto_pipeline") {
            let s = v
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "webhook.auto_pipeline 必须是字符串"))?;
            if !AUTO_PIPELINES.contains(&s) {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("webhook.auto_pipeline 只能是 {} 之一：{s}", AUTO_PIPELINES.join(" / ")),
                ));
            }
            out.insert("auto_pipeline".into(), serde_json::json!(s));
        }
        let root = out.get("root").and_then(|v| v.as_str()).unwrap_or("");
        let container = out
            .get("container_root")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !root.is_empty() && !std::path::Path::new(root).is_dir() {
            warnings.push(format!(
                "通用 webhook 的录像目录当前不可访问（{root}）：事件里的路径会解析失败。若在网络盘/另一台机器上，请确认 YekiTrim 进程也读得到。"
            ));
        }
        // 相对路径会按**服务进程的 CWD**解析（`resolve_webhook_completion` 里就是
        // `Path::is_dir()` + 相对 root join）—— 换个启动方式/换个工作目录就静默失效，
        // 而配置里那句「宿主机绝对路径」只有人读得懂。
        if !root.is_empty() && !std::path::Path::new(root).is_absolute() {
            warnings.push(
                "通用 webhook 的录像目录要填**绝对路径**：相对路径按服务进程的当前工作目录解析，换个启动方式就会失效。"
                    .into(),
            );
        }
        if root.is_empty() && !container.is_empty() {
            warnings.push(
                "通用 webhook 填了 container_root 但 root 为空 —— 入口不会启用（root 是宿主机上的落点）。"
                    .into(),
            );
        }
        if !out.is_empty() {
            ingest.insert("webhook".into(), serde_json::Value::Object(out));
        }
    }

    // B站下载通道（P4）。⚠️ 这里必须**平铺**成 `[ingest]` 的扁平键，不能嵌套成子表：
    // `[ingest] downloader` 本身是一个**字符串键**（引擎名）。提交 `{"downloader": {…}}`
    // 会把这一项写成 inline table —— 文件仍是合法 TOML、写入也不报错，
    // 但下次读配置时 `downloader` 反序列化成 `String` 直接失败，**整份配置被静默忽略**。
    // 回归钉子见 `patch_download_keys_are_flat_not_nested`。
    if let Some(v) = body.get("download") {
        let d = v
            .as_object()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "download 必须是对象"))?;

        for (key, field) in [
            ("engine", "downloader"),
            ("ytdlp_path", "ytdlp_path"),
            ("cookie_file", "cookie_file"),
        ] {
            if let Some(x) = d.get(key) {
                let s = x
                    .as_str()
                    .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("download.{key} 必须是字符串")))?
                    .trim();
                ingest.insert(field.into(), serde_json::json!(s));
            }
        }

        if let Some(x) = d.get("max_quality") {
            let s = x
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "download.max_quality 必须是字符串"))?;
            check_quality(s)
                .map_err(|r| err(StatusCode::BAD_REQUEST, format!("download.max_quality：{r}")))?;
            ingest.insert("max_quality".into(), serde_json::json!(s.trim()));
        }

        for (key, field) in [
            ("rate_limit_kbps", "rate_limit_kbps"),
            ("max_concurrent_downloads", "max_concurrent_downloads"),
            ("idle_timeout_secs", "download_idle_timeout_secs"),
        ] {
            if let Some(x) = d.get(key) {
                let n = x
                    .as_u64()
                    .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("download.{key} 必须是非负整数")))?;
                // 0 是合法的（不限速 / 不限并发由 core 侧 `.max(1)` 兜住）；
                // 但并发调大就是给 B站风控送人头，这里给一条 warning 说清楚。
                if key == "max_concurrent_downloads" && n > 2 {
                    warnings.push(format!(
                        "同时下载数设为 {n}：B站的 412 风控与 IP 频次相关，调大更容易被拦（建议 1）"
                    ));
                }
                ingest.insert(field.into(), serde_json::json!(n));
            }
        }

        // 自备路径/引擎名写错了不拒绝（可能只是文件还没放好），但要提醒
        if let Some(p) = ingest.get("ytdlp_path").and_then(|v| v.as_str()) {
            if !p.is_empty() && !std::path::Path::new(p).is_file() {
                warnings.push(format!(
                    "yt-dlp 路径当前不可执行（{p}）：会自动回退到托管副本或 PATH 里的 yt-dlp。"
                ));
            }
        }
    }

    // 项目存放位置（`[storage] projects_root`）：下载与入库的**共同**落点。
    // 放在 `[storage]` 而不是 `[ingest]` —— 它是平台级存储配置，录播姬入库/本机导入也用同一个根。
    if let Some(v) = body.get("storage") {
        let s = v
            .as_object()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "storage 必须是对象"))?;
        if let Some(x) = s.get("projects_root") {
            let raw = x
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "storage.projects_root 必须是字符串"))?;
            storage.insert("projects_root".into(), serde_json::json!(check_projects_root(raw)?));
        }
    }

    Ok((ingest, storage, warnings))
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::config::{config_patch, write_global_config_at};
    

    /// 校验：坏 IP / 坏 scheme / 坏 auto_pipeline / 类型不对，都要给出**可读的 400**。
    ///
    /// 特别是 `192.168.1.0/24` 这种写法：运行时白名单是精确字符串相等，
    /// 放行它只会让来源被静默拦掉、极难排查，所以这里必须挡住。
    #[test]
    fn ingest_patch_rejects_invalid_input() {
        let cases = [
            (serde_json::json!({"allow_ips": ["192.168.1.0/24"]}), "allow_ips"),
            (serde_json::json!({"allow_ips": ["recorder.local"]}), "allow_ips"),
            (serde_json::json!({"allow_ips": "127.0.0.1"}), "allow_ips"),
            (serde_json::json!({"recorder": {"api_base": "127.0.0.1:2356"}}), "api_base"),
            (serde_json::json!({"recorder": {"auto_pipeline": "everything"}}), "auto_pipeline"),
            (serde_json::json!({"recorder": {"root": 123}}), "root"),
            (serde_json::json!({"webhook_enabled": "yes"}), "webhook_enabled"),
            (serde_json::json!({"recorder": {"watch_interval_mins": -1}}), "watch_interval_mins"),
        ];
        for (body, needle) in cases {
            let e = build_ingest_patch(&body).unwrap_err();
            assert_eq!(e.0, StatusCode::BAD_REQUEST, "{body}");
            let msg = serde_json::to_string(&e.1 .0).unwrap();
            assert!(msg.contains(needle), "错误文案该点出是哪个键（{needle}）：{msg}");
        }
        // 合法的照常通过：单个 IP（v4/v6）、带 scheme 的地址、四个终点
        for ok_body in [
            serde_json::json!({"allow_ips": ["127.0.0.1", "::1"]}),
            serde_json::json!({"recorder": {"api_base": "http://127.0.0.1:2356"}}),
            serde_json::json!({"recorder": {"api_base": ""}}),
            serde_json::json!({"recorder": {"auto_pipeline": "cut"}}),
            serde_json::json!({"recorder": {"watch_interval_mins": 0}}),
        ] {
            assert!(build_ingest_patch(&ok_body).is_ok(), "{ok_body}");
        }
    }
    /// ⚠️ 回归钉子：**B站下载那几项必须平铺在 `[ingest]` 下，不能嵌套成子表**。
    ///
    /// `[ingest] downloader` 是一个**字符串键**（引擎名）。如果这里手滑写成
    /// `ingest.insert("downloader", json!({...}))`，文件写出来仍是合法 TOML、写入也不报错，
    /// 但下次读配置时 `downloader` 反序列化成 `String` 会失败 → **整份配置被静默忽略**
    /// （表现为"改完设置重启后所有设置都回到默认"，极难归因）。
    #[test]
    fn patch_download_keys_are_flat_not_nested() {
        let (m, _, _) = build_ingest_patch(&serde_json::json!({
            "download": { "engine": "D:/tools/yt-dlp.exe", "max_quality": "720p", "max_concurrent_downloads": 2 }
        }))
        .unwrap();

        assert!(
            m.get("downloader").map(|v| v.is_string()).unwrap_or(false),
            "downloader 必须是字符串（平铺键），不是对象：{m:?}"
        );
        assert_eq!(m.get("downloader").unwrap(), &serde_json::json!("D:/tools/yt-dlp.exe"));
        assert_eq!(m.get("max_quality").unwrap(), &serde_json::json!("720p"));
        assert_eq!(m.get("max_concurrent_downloads").unwrap(), &serde_json::json!(2));
        // 分组名本身不能出现在 patch 里（它只是 HTTP 层的容器）
        assert!(m.get("download").is_none(), "分组名不该落进配置：{m:?}");
        // 并发调大要给出风控提醒
        let (_, _, w) = build_ingest_patch(&serde_json::json!({
            "download": { "max_concurrent_downloads": 5 }
        }))
        .unwrap();
        assert!(w.iter().any(|s| s.contains("412")), "并发调大应提醒风控：{w:?}");

        // 非法值同样要挡（清晰度写错会静默变成"不限高"，把磁盘吃光）
        for (body, needle) in [
            (serde_json::json!({"download": {"max_quality": "超清"}}), "max_quality"),
            (serde_json::json!({"download": {"engine": 123}}), "engine"),
            (serde_json::json!({"download": "yt-dlp"}), "download"),
            (serde_json::json!({"download": {"rate_limit_kbps": -1}}), "rate_limit_kbps"),
        ] {
            let e = build_ingest_patch(&body).unwrap_err();
            assert_eq!(e.0, StatusCode::BAD_REQUEST, "{body}");
            let msg = serde_json::to_string(&e.1 .0).unwrap();
            assert!(msg.contains(needle), "{needle}: {msg}");
        }
    }
    /// 表单语义：**键缺席 = 不改**、给空串 = 清空、空数组 = 清空白名单。
    ///
    /// 这条是"留空不改"能不能成立的基础：如果缺席也被写成空串，
    /// 用户没碰密码框就会把已保存的密码清掉。
    #[test]
    fn ingest_patch_distinguishes_absent_from_empty() {
        // 缺席：patch 里连键都没有（于是文件里那行原样不动）
        let (m, _, w) = build_ingest_patch(&serde_json::json!({"webhook_enabled": true})).unwrap();
        assert_eq!(m.len(), 1);
        assert!(m.get("recorder").is_none(), "没提到 recorder 就不该出现：{m:?}");
        assert!(w.is_empty());

        // 显式空串 / 空数组 = 清空
        let (m, _, _) = build_ingest_patch(&serde_json::json!({
            "allow_ips": [],
            "recorder": {"api_base": "", "api_pass": ""}
        }))
        .unwrap();
        assert_eq!(m["allow_ips"], serde_json::json!([]));
        assert_eq!(m["recorder"]["api_base"], serde_json::json!(""));
        assert_eq!(m["recorder"]["api_pass"], serde_json::json!(""));

        // 口令不 trim（首尾空格是合法字符），其余路径/地址 trim；空项被丢掉、重复项去重
        let (m, _, _) = build_ingest_patch(&serde_json::json!({
            "allow_ips": [" 127.0.0.1 ", "", "127.0.0.1"],
            "recorder": {"root": "  D:/rec  ", "api_pass": " p w "}
        }))
        .unwrap();
        assert_eq!(m["allow_ips"], serde_json::json!(["127.0.0.1"]), "去重 + 去空");
        assert_eq!(m["recorder"]["root"], serde_json::json!("D:/rec"));
        assert_eq!(m["recorder"]["api_pass"], serde_json::json!(" p w "), "口令原样");

        // 目录不可达只给 warning，不拒绝（网络盘可能只是暂时没挂上）
        let (_, _, w) = build_ingest_patch(&serde_json::json!({
            "recorder": {"root": "Z:/definitely-not-here-yekitrim"}
        }))
        .unwrap();
        assert_eq!(w.len(), 1, "该有一条 warning：{w:?}");
        assert!(w[0].contains("Z:/definitely-not-here-yekitrim"), "{w:?}");
    }
    /// 通用 webhook 的补丁必须落成 **`[ingest.webhook]` 子表**，且不碰 `[ingest]` 兄弟键。
    ///
    /// ⚠️ 与 downloader 那条**相反**：`[ingest] downloader` 本身是字符串键，所以必须平铺；
    /// 而 `webhook` 是子表（`IngestConfig::webhook`），提交成 inline table / 顶层键都不会报错，
    /// 只会在**下次读配置时静默变成默认值**（表单显示空白，用户以为没保存）。
    #[tokio::test]
    async fn webhook_patch_writes_a_subtable_and_keeps_siblings() {
        let dir = std::env::temp_dir().join(format!("yekitrim-ingest-wh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[ingest]\nwebhook_enabled = true\ntoken_hash = \"scrypt$keepme\"\n").unwrap();

        let (ingest, _, warnings) = build_ingest_patch(&serde_json::json!({
            "webhook": {
                "root": "/srv/rec",
                "container_root": "/app/video",
                "auto_pipeline": "detect"
            }
        }))
        .unwrap();
        write_global_config_at(&path, config_patch(ingest, serde_json::Map::new()))
            .await
            .unwrap();

        let out = std::fs::read_to_string(&path).unwrap();
        let section = out
            .split("[ingest.webhook]")
            .nth(1)
            .unwrap_or_else(|| panic!("必须有 [ingest.webhook] 表头：\n{out}"));
        for key in [
            "root = \"/srv/rec\"",
            "container_root = \"/app/video\"",
            "auto_pipeline = \"detect\"",
        ] {
            assert!(section.contains(key), "{key} 不在 [ingest.webhook] 里：\n{out}");
        }
        assert!(!out.contains("webhook = {"), "不能退化成 inline table：\n{out}");
        assert!(
            out.contains("token_hash = \"scrypt$keepme\"") && out.contains("webhook_enabled = true"),
            "[ingest] 的兄弟键被冲掉了：\n{out}"
        );
        // root 不可达**只告警不拒绝**（网络盘可能只是暂时没挂上）
        assert!(
            warnings.iter().any(|w| w.contains("/srv/rec")),
            "不可达的 root 要给出警告：{warnings:?}"
        );
        // 相对 root 要单独点名（它按进程 CWD 解析，换启动方式就静默失效）
        let (_, _, warnings) =
            build_ingest_patch(&serde_json::json!({"webhook": {"root": "relative/rec"}})).unwrap();
        assert!(
            warnings.iter().any(|w| w.contains("绝对路径")),
            "相对 root 必须给出警告：{warnings:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **录像目录**的 warning 必须指名下一步（跨机三件事：填了对方机器的路径 / 只读 / 空目录）。
    #[test]
    fn recorder_root_warnings_are_actionable() {
        let (_, _, w) = build_ingest_patch(&serde_json::json!({
            "recorder": { "root": "rec" }
        }))
        .unwrap();
        assert!(!w.is_empty(), "相对路径必须给警告");
        assert!(
            w.iter().any(|s| s.contains("绝对路径") || s.contains("共享地址")),
            "{w:?}"
        );

        // 可达但空的目录：提醒"确认共享的就是录播姬输出目录"
        let dir = tempfile::tempdir().unwrap();
        let (_, _, w) = build_ingest_patch(&serde_json::json!({
            "recorder": { "root": dir.path().to_string_lossy() }
        }))
        .unwrap();
        assert!(w.iter().any(|s| s.contains("没有录像")), "{w:?}");
    }
}
