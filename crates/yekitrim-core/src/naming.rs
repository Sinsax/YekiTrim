//! 产物标题的命名规则（**唯一一份**）。
//!
//! # 为什么单独一个模块
//!
//! 主播名以前在**检测**阶段就被烧进 `Event.label`（`detection` 卡有说明），后果是
//! 「名字填错 → 整批片段标题全错 → 只能整项目重切」。2026-11-22 起它只活在**产物标题**里，
//! 而"怎么拼"有三处消费者（投稿 `publish.rs`、精修页 `project_data`、CLI 预览）——
//! 必须共用同一份实现，否则迟早出现「界面显示带名、投稿不带」。
//!
//! # 三条语义（用户口径，别自己发明）
//!
//! 1. **位置不变**：tag 一律拼在**末尾**（`标题【名】`）。老数据里曾经允许放开头，
//!    读取时会归一化到末尾。
//! 2. **空名不拼**：名字为空 ⇒ 原样返回（不留下 `【】`）。
//! 3. **幂等**：`streamer_title(streamer_title(t, n), n) == streamer_title(t, n)` ——
//!    精修页会反复读、反复写，不幂等就会越拼越长。
//!
//! # 老数据的处理（重要）
//!
//! 升级前切出来的片段，`publish_config.json` 的 `title` 里**已经带着** `【旧名】`。读取侧用
//! [`resolve_streamer`] 归一化：
//! * tag 名与**当前**有效名一致 ⇒ 当成"没改过"，剥掉后按跟随重拼（否则会双份）；
//! * 不一致 ⇒ 认成该片段的**覆盖值**（用户当初就想要那个名），从此锁定、不跟随全局。
//!
//! 都不改磁盘 —— 只有用户真的在界面上动过一次才会落盘。

/// 名字的字符上限。与 `yekitrim-server` 那边 `MAX_TITLE_CHARS` 不是一回事：
/// 这个只用来判定"老数据末尾那个 `【…】` 是不是主播标签"（太长的一定是标题自带的梗）。
pub const MAX_NAME_CHARS: usize = 32;

const OPEN: char = '【';
const CLOSE: char = '】';

/// 名字里出现方括号会破坏 tag 的边界判定 —— 一律去掉（用户输入别在标题里制造歧义）。
pub fn sanitize_name(name: &str) -> String {
    name.trim()
        .chars()
        .filter(|c| *c != OPEN && *c != CLOSE)
        .collect::<String>()
        .trim()
        .to_string()
}

/// 拆出**开头或结尾**那一个 `【…】`。返回 `(名字, 是否在开头, 剩下的正文)`。
///
/// 与前端 `PolishView.vue::splitNameTag` 逐条同规则（那边是回填输入框用的）。
pub fn split_name_tag(raw: &str) -> Option<(String, bool, String)> {
    let t = raw.trim();
    if let Some(rest) = t.strip_prefix(OPEN) {
        if let Some(end) = rest.find(CLOSE) {
            let name = &rest[..end];
            let body = &rest[end + CLOSE.len_utf8()..];
            if !name.contains(OPEN) {
                return Some((name.trim().to_string(), true, body.trim().to_string()));
            }
        }
    }
    if let Some(head) = t.strip_suffix(CLOSE) {
        if let Some(start) = head.rfind(OPEN) {
            let name = &head[start + OPEN.len_utf8()..];
            let body = &head[..start];
            if !name.contains(CLOSE) {
                return Some((name.trim().to_string(), false, body.trim().to_string()));
            }
        }
    }
    None
}

/// 去掉开头/结尾那一个 tag（只去一个、幂等）。不是 tag 就原样返回。
pub fn strip_name_tag(raw: &str) -> String {
    match split_name_tag(raw) {
        Some((_, _, rest)) => rest,
        None => raw.trim().to_string(),
    }
}

/// 把名字以 `【名】` 拼到标题**末尾**（位置不变）。空名 ⇒ 原样；已有同名 tag ⇒ 不重复。
pub fn streamer_title(title: &str, name: &str) -> String {
    let body = title.trim();
    let name = sanitize_name(name);
    if name.is_empty() {
        return body.to_string();
    }
    let tag = format!("{OPEN}{name}{CLOSE}");
    if body.is_empty() {
        return tag;
    }
    // 已经有同名 tag（且位置合法）⇒ 只把位置归一到末尾
    if let Some((existing, at_head, rest)) = split_name_tag(body) {
        if existing == name {
            return if at_head { format!("{rest}{tag}") } else { body.to_string() };
        }
    }
    format!("{body}{tag}")
}

/// 读取侧归一化：`(标题正文, 该片段的有效主播名)`。
///
/// `override_name` 三态（就是 `publish_config.clips[].streamer_name` 的三种落地）：
/// * `None`（字段缺席）= **跟随**项目级名字；
/// * `Some("")` = 这个片段**明确不用**主播名；
/// * `Some("名")` = 覆盖（用户改过 ⇒ 从此不跟随）。
///
/// 老数据（`override_name == None` 但 `stored_title` 里已经带 tag）按模块头那段处理：
/// 与当前名字一致 ⇒ 当成没改过；不一致 ⇒ 认成覆盖。
pub fn resolve_streamer(
    override_name: Option<&str>,
    stored_title: &str,
    project_name: &str,
) -> (String, String) {
    let project_name = sanitize_name(project_name);
    let legacy = split_name_tag(stored_title);
    let body = match &legacy {
        Some((_, _, rest)) => rest.clone(),
        None => stored_title.trim().to_string(),
    };
    let effective = match override_name {
        Some(raw) => sanitize_name(raw),
        None => match legacy {
            // 名字没变过：清掉老 tag，继续跟随
            Some((tag, _, _)) if tag == project_name => project_name,
            // 名字变过：把老 tag 认成这条片段的覆盖值（锁定，不再被全局改名带走）
            Some((tag, _, _)) if tag.chars().count() <= MAX_NAME_CHARS => tag,
            _ => project_name,
        },
    };
    (body, effective)
}

/// 一次到位的最终标题（读取/投稿共用的那一条）。
pub fn publish_title(override_name: Option<&str>, stored_title: &str, project_name: &str) -> String {
    let (body, name) = resolve_streamer(override_name, stored_title, project_name);
    let base = if body.trim().is_empty() {
        // 标题为空时用调用方兜底（`publish.rs` 会先用目录名补），这里只负责拼
        String::new()
    } else {
        body
    };
    streamer_title(&base, &name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_is_always_appended_at_the_end() {
        assert_eq!(streamer_title("标题", "主播"), "标题【主播】");
        // 位置不变：老数据里在开头的，归一化到末尾
        assert_eq!(streamer_title("【主播】标题", "主播"), "标题【主播】");
        // 空名 ⇒ 一个字节都不加（不留下 `【】`）
        assert_eq!(streamer_title("标题", ""), "标题");
        assert_eq!(streamer_title("标题", "   "), "标题");
        // 只有名字没有标题
        assert_eq!(streamer_title("", "主播"), "【主播】");
    }

    #[test]
    fn compose_is_idempotent() {
        let once = streamer_title("标题", "主播");
        assert_eq!(streamer_title(&once, "主播"), once, "反复读写的路径必须收敛");
        assert_eq!(streamer_title(&streamer_title(&once, "主播"), "主播"), once);
    }

    #[test]
    fn name_with_brackets_cannot_break_the_tag() {
        assert_eq!(streamer_title("标题", "主【播】"), "标题【主播】");
        // 剥回来还是同一段，不会出现半个 tag
        assert_eq!(strip_name_tag("标题【主播】"), "标题");
    }

    #[test]
    fn legacy_tag_is_normalized_on_read() {
        // ① 名字没变过 ⇒ 不会双份，继续跟随
        assert_eq!(publish_title(None, "标题【主播】", "主播"), "标题【主播】");
        // 位置在开头的老数据也归一到末尾
        assert_eq!(publish_title(None, "【主播】标题", "主播"), "标题【主播】");
        // ② 名字改过 ⇒ 老 tag 认成这条片段的覆盖，锁定
        assert_eq!(publish_title(None, "标题【旧名】", "新名"), "标题【旧名】");
        // ③ 没有覆盖、标题也没 tag ⇒ 跟随项目名
        assert_eq!(publish_title(None, "标题", "新名"), "标题【新名】");
        // ④ 片段明确不用 ⇒ 项目级有名字也不拼
        assert_eq!(publish_title(Some(""), "标题", "新名"), "标题");
        // ⑤ 片段覆盖 ⇒ 用片段的
        assert_eq!(publish_title(Some("片段名"), "标题", "新名"), "标题【片段名】");
    }

    #[test]
    fn split_name_tag_matches_the_frontend_rules() {
        assert_eq!(split_name_tag("【名】标题"), Some(("名".into(), true, "标题".into())));
        assert_eq!(split_name_tag("标题【名】"), Some(("名".into(), false, "标题".into())));
        assert_eq!(split_name_tag("标题"), None);
        // 标题自带的梗在**中间** ⇒ 不是 tag
        assert_eq!(split_name_tag("【梗】标题【梗】"), Some(("梗".into(), true, "标题【梗】".into())));
        assert_eq!(strip_name_tag("【梗】标题【梗】"), "标题【梗】");
    }
}