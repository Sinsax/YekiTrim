//! webhook payload 的解析与归一化：官方 PascalCase 与 biliLive-tools 两种方言。
//!
//! 官方明确「以后可能新增事件类型」，所以字段一律走 `field*`（大小写不敏感 + 多别名）；
//! **认不出的事件类型不进池**（解析返回 `None`，由接收端回 2xx「已收到、不作处理」）。

use super::*;

/// 事件类型（`FileOpening` → `file_opening`）用于审计与去重表。
pub(super) fn normalize_kind(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 4);
    for (i, c) in raw.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// 一次"录制完成"事件（两条入口共用的中间结构）。
#[derive(Debug, Clone)]
pub(super) struct WebhookCompletion {
    // 字段也要 `pub(super)`：读它的是另一张文件里的 `resolve_webhook_completion`（同目录不同模块）。
    pub(super) file: String,
    pub(super) danmaku: Option<String>,
    pub(super) room: Option<String>,
    pub(super) platform: Option<String>,
    pub(super) title: Option<String>,
}

pub(super) enum ParsedWebhookEvent {
    Completion(WebhookCompletion),
    Ignored(String),
}

/// 事件类型归一化：`FileClosed` / `file_closed` / `file-closed` / `FILE_CLOSED`
/// 全部变成 `file_closed`。
///
/// 三种来源的写法不一样（biliLive-tools 的 `type` 是 snake_case、录播姬的 `EventType`
/// 是 PascalCase、手写脚本什么都可能），归一化只做一次、只此一份。
pub(super) fn normalize_event_type(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(raw.len() + 4);
    let mut prev_lower = false;
    for ch in raw.chars() {
        if ch == '-' || ch == ' ' || ch == '.' {
            push_event_sep(&mut out);
            prev_lower = false;
        } else if ch.is_ascii_uppercase() {
            // `FileClosed` → `file_closed`；`file_closed` / `FILE_CLOSED` 原样保留。
            if prev_lower {
                push_event_sep(&mut out);
            }
            out.push(ch.to_ascii_lowercase());
            prev_lower = false;
        } else {
            out.push(ch);
            prev_lower = true;
        }
    }
    out
}

fn push_event_sep(out: &mut String) {
    if !out.ends_with('_') {
        out.push('_');
    }
}

/// 解析通用 webhook 的 body。
///
/// 兼容三种形状（按优先级）：
/// 1. biliLive-tools 的 `externalWebhook`：`{type:"file_completed", data:{filePath,roomId,platform}}`；
/// 2. YekiTrim 自己的规范事件：`{event:"file_completed", file, danmaku?, room_id?, platform?}`；
/// 3. 录播姬的 `FileClosed`：`{EventType:"FileClosed", EventData:{RelativePath,RoomId}}`
///    （这条走 `/bililive/<token>` 也可以；这里接住是为了让同一个 URL 兼容更多软件）。
///
/// `record_start` / `record_end` / `file_created` / `task` 等**不建项目**的事件
/// 一律返回 `Ignored`，入口回 200 —— 录制软件那边不需要为"这条不归你管"重试。
pub(super) fn parse_webhook_event(body: &serde_json::Value) -> Result<ParsedWebhookEvent, String> {
    // ⚠️ 只 `to_ascii_lowercase()` 是不够的：录播姬的 `EventType` 是 **PascalCase**
    // （`FileClosed` → `fileclosed`），那样永远匹配不上 `file_closed` —— 于是上面注释里
    // "同一个 URL 兼容更多软件"只是愿望，录播姬原样打过来会回 400「不认识的事件类型」。
    let ty = normalize_event_type(
        field(
            body,
            &["type", "event_type", "event", "EventType", "eventType"],
        )
        .unwrap_or_default(),
    );
    let data = body
        .get("data")
        .or_else(|| body.get("EventData"))
        .or_else(|| body.get("event_data"))
        .unwrap_or(body);

    // biliLive-tools 的 type / 录播姬的 EventType 都会被这里归一化成 snake_case。
    if ty == "file_completed" || ty == "file_closed" {
        let file = field(
            data,
            &[
                "filePath",
                "file_path",
                "file",
                "path",
                "RelativePath",
                "relative_path",
            ],
        )
        .or_else(|| field(body, &["filePath", "file_path", "file"]))
        .ok_or_else(|| "file_completed 缺少 filePath/file".to_string())?;
        return Ok(ParsedWebhookEvent::Completion(WebhookCompletion {
            file: file.to_string(),
            danmaku: field(data, &["danmaku", "danmaku_path", "danmakuPath"]).map(str::to_string),
            room: field_text(data, &["roomId", "room_id", "RoomId", "room"]),
            platform: field(data, &["platform", "Platform"]).map(str::to_string),
            title: field(data, &["title", "Title"]).map(str::to_string),
        }));
    }

    // 没有 type 但直接给了文件：按"录制完成"处理（手写脚本最省事）。
    if ty.is_empty() {
        if let Some(file) = field(body, &["file", "filePath", "file_path"]) {
            return Ok(ParsedWebhookEvent::Completion(WebhookCompletion {
                file: file.to_string(),
                danmaku: field(body, &["danmaku", "danmaku_path"]).map(str::to_string),
                room: field_text(body, &["room_id", "roomId"]),
                platform: field(body, &["platform"]).map(str::to_string),
                title: field(body, &["title"]).map(str::to_string),
            }));
        }
    }

    if matches!(
        ty.as_str(),
        "record_start"
            | "record_end"
            | "file_created"
            | "task"
            | "session_started"
            | "session_ended"
            | "stream_started"
            | "stream_ended"
    ) {
        return Ok(ParsedWebhookEvent::Ignored(ty));
    }
    Err(format!(
        "不认识的事件类型: {ty}（支持 file_completed / file_closed，或 biliLive-tools 的 type=file_completed）"
    ))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_event_kinds() {
        assert_eq!(normalize_kind("FileClosed"), "file_closed");
        assert_eq!(normalize_kind("SessionStarted"), "session_started");
        assert_eq!(normalize_kind("UnknownThing"), "unknown_thing");
    }
    #[test]
    fn webhook_parses_bililive_tools_completion() {
        let body = serde_json::json!({
            "id": "evt-1",
            "version": 1,
            "type": "file_completed",
            "data": {
                "filePath": "/app/video/Bilibili/a/[2026-10-29 16-00-00-000][t]/rec.flv",
                "roomId": 123456,
                "platform": "Bilibili"
            }
        });
        match parse_webhook_event(&body).unwrap() {
            ParsedWebhookEvent::Completion(c) => {
                assert!(c.file.ends_with("rec.flv"));
                assert_eq!(c.room.as_deref(), Some("123456"));
                assert_eq!(c.platform.as_deref(), Some("Bilibili"));
            }
            _ => panic!("file_completed 必须解析成 Completion"),
        }
    }
    #[test]
    fn webhook_parses_canonical_and_ignores_non_completion() {
        let body = serde_json::json!({
            "event": "file_completed",
            "file": "Bilibili/a/rec.mp4",
            "room_id": "42",
            "title": "测试"
        });
        match parse_webhook_event(&body).unwrap() {
            ParsedWebhookEvent::Completion(c) => {
                assert_eq!(c.file, "Bilibili/a/rec.mp4");
                assert_eq!(c.room.as_deref(), Some("42"));
                assert_eq!(c.title.as_deref(), Some("测试"));
            }
            _ => panic!("规范事件必须解析成 Completion"),
        }
        assert!(matches!(
            parse_webhook_event(&serde_json::json!({"type": "record_start"})).unwrap(),
            ParsedWebhookEvent::Ignored(_)
        ));
        assert!(parse_webhook_event(&serde_json::json!({"type": "unknown"})).is_err());
    }
    /// 录播姬原样打过来的形状（`EventType` + `EventData`，**PascalCase**）必须认。
    ///
    /// 这条曾经回 400：`parse_webhook_event` 只 `to_ascii_lowercase()`，`FileClosed`
    /// 变成 `fileclosed`，永远匹配不上 `file_closed` —— 于是「同一个 URL 兼容更多软件」
    /// 只是注释里的愿望，而录制软件会把 4xx 当成"没送到"反复重试。
    #[test]
    fn webhook_accepts_recorder_pascal_case_event_type() {
        let body = serde_json::json!({
            "EventType": "FileClosed",
            "EventData": {
                "RelativePath": "26320007-主播/2026-10-29 16-00-00/rec.flv",
                "RoomId": 26320007
            }
        });
        match parse_webhook_event(&body).unwrap() {
            ParsedWebhookEvent::Completion(c) => {
                assert_eq!(c.file, "26320007-主播/2026-10-29 16-00-00/rec.flv");
                assert_eq!(c.room.as_deref(), Some("26320007"));
            }
            _ => panic!("录播姬 FileClosed 必须解析成 Completion"),
        }
        // 不建项目的事件（PascalCase 写法）同样要认，否则 400 会让录制软件重试。
        assert!(matches!(
            parse_webhook_event(&serde_json::json!({"EventType": "SessionEnded"})).unwrap(),
            ParsedWebhookEvent::Ignored(_)
        ));
        // 三种来源三种写法都归一（归一化只此一份，别在解析处再写一遍匹配）。
        assert_eq!(normalize_event_type("FileClosed"), "file_closed");
        assert_eq!(normalize_event_type("file_closed"), "file_closed");
        assert_eq!(normalize_event_type("file-closed"), "file_closed");
        assert_eq!(normalize_event_type("FILE_CLOSED"), "file_closed");
        assert_eq!(normalize_event_type("  FileClosed  "), "file_closed");
        assert_eq!(normalize_event_type(""), "");
    }
}
