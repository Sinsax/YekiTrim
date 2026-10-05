//! 任务资源键：**哪些任务会写同一批文件**。
//!
//! `JobManager` 用这串键做互斥（相等匹配，见 `jobs::resources_conflict`）：键相交 → 排队。
//! 所以这里只做一件事：把"这个请求要动哪儿"翻译成稳定的字符串。
//!
//! | 键 | 持有者 | 为什么 |
//! |---|---|---|
//! | `p{pid}:out` | pipeline / stage_* / correct / ingest_* / 所有 cut | 都读 `events.json`、写 `output/*` |
//! | `p{pid}:idx` | 所有 cut | `publish_config.json` + `clips_order.txt` 是共写文件 |
//! | `p{pid}:clip:{clip-07}` | `cut_one(rank=7)`、该目录上的 `polish_*` | 同一个片段目录 |
//! | `models` | `download_models`、`stage_transcribe` | 都会写全局模型目录 `ensure_models` |
//! | `stickers` | `sticker_build` | 全局贴纸库索引 `sticker.toml` |
//! | `dir:{规范化绝对路径}` | `polish_*` 兜底键 | pid 没解析出来（老工程/任意目录）时也能和 cut 的 dir 键对上 |
//!
//! ⚠️ **全切不带 `clip:*`**：它只处理没有 `clip.mp4` 的目录（`event_cut_all_impl`），
//! 而精修工程一定建在已剪好的目录里 —— 所以「全切」与「精修」可以并行，这是刻意的。

use std::path::Path;

/// 任务占用的资源键集合。
pub type Keys = Vec<String>;

/// 项目级：输出目录（events.json / transcript / 各阶段产物）。
pub fn project_out(pid: i64) -> String {
    format!("p{pid}:out")
}

/// 项目级：发布索引（publish_config.json + clips_order.txt，只有 cut 写）。
pub fn project_index(pid: i64) -> String {
    format!("p{pid}:idx")
}

/// 项目级：单个片段目录（`clips/clip-07`）。
pub fn project_clip(pid: i64, dir_name: &str) -> String {
    format!("p{pid}:clip:{}", dir_name.to_ascii_lowercase())
}

/// 路径级兜底键：规范化（canonicalize + 剥 `\\?\` + Windows 下小写）。
///
/// 大小写不敏感只对 Windows 做 —— Linux 上 `Dir` 与 `dir` 是两个目录。
/// 目录还不存在时（剪切目标目录）退一步规范化父目录再拼名字，否则"剪切前"与
/// "剪完之后"算出来的键会不一样（一个带 `\\?\`、一个不带）。
pub fn dir_key(path: &Path) -> String {
    let canonical = canonical_or_join(path);
    let text = yekitrim_core::path::display(&canonical);
    let text = if cfg!(windows) { text.to_ascii_lowercase() } else { text };
    format!("dir:{text}")
}

fn canonical_or_join(path: &Path) -> std::path::PathBuf {
    if let Ok(p) = std::fs::canonicalize(path) {
        return p;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => match std::fs::canonicalize(parent) {
            Ok(p) => p.join(name),
            Err(_) => path.to_path_buf(),
        },
        _ => path.to_path_buf(),
    }
}

/// 全局：ASR 模型目录。
pub const MODELS: &str = "models";

/// 下载任务（B站链接通道）：**按会话**互斥，不与项目资源键相交。
///
/// 为什么不用 `dir:{project_root}`：下载启动时项目目录刚建、还没登记项目，
/// 而 `dir_key` 在目录不存在时要回退到父目录 —— 那时算出来的键与"下载完成后"不一致，
/// 会让同一个下载在两次 tick 之间被认成两把不同的锁。会话 id 是稳定的。
pub fn download_key(session_id: &str) -> String {
    format!("dl:{session_id}")
}

/// 全局：贴纸库（`<程序目录>/stickers`）。
pub const STICKERS: &str = "stickers";

/// 剪切任务的资源键。`clip_dir = None` = 全切（只占 out + idx，**不带** `clip:*`，
/// 因此可与精修并行）；`clip_dir = Some` = 单条重切，会写那个目录，与精修互斥。
pub fn cut_keys(pid: i64, output_dir: &Path, clip_dir: Option<&str>) -> Keys {
    let mut keys = vec![project_out(pid), project_index(pid)];
    if let Some(name) = clip_dir {
        keys.push(project_clip(pid, name));
        keys.push(dir_key(&output_dir.join("clips").join(name)));
    }
    keys
}

/// 管线任务的资源键（`stages` 用管线里的阶段名）。
///
/// `p{pid}:out` 恒有；带 `transcribe`/`process` 时加 `models`（转写会 `ensure_models`
/// 往全局模型目录写）；带 `cut`/`process` 时加 `p{pid}:idx`（写发布索引）。
pub fn pipeline_keys(pid: i64, stages: &[String]) -> Keys {
    let has = |name: &str| stages.iter().any(|s| s == name);
    let mut keys = vec![project_out(pid)];
    if has("transcribe") || has("process") {
        keys.push(MODELS.to_string());
    }
    if has("cut") || has("process") {
        keys.push(project_index(pid));
    }
    keys
}

/// 管线里"要剪"的全部片段键（`clips/clip-NN`）。
///
/// 全量 process / 含剪切断的管线会把 events.json 里每个事件都过一遍 Cutter：同名同区间的
/// 片段会被**重切**（`foreign_clip_conflict` 只拦"不是同一条事件"的目录），所以它必须把
/// 这些 `clip-NN` 都占住，不能和正在精修/渲染的片段并行。读不到 events.json 时返回空
/// （此时 Cutter 也切不出东西）。
pub fn cut_stage_clip_keys(pid: i64, output_dir: &Path, events_json: &Path) -> Keys {
    let Ok(raw) = std::fs::read_to_string(events_json) else {
        return Vec::new();
    };
    let Ok(store) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    for ev in store.get("events").and_then(|v| v.as_array()).into_iter().flatten() {
        let Some(rank) = ev.get("rank").and_then(|r| r.as_u64()) else {
            continue;
        };
        let dir = ev
            .get("clip_dir")
            .and_then(|d| d.as_str())
            .map(str::trim)
            .filter(|d| !d.is_empty() && yekitrim_core::cutting::clip_index::is_safe_component(d))
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("clip-{rank:02}"));
        keys.push(project_clip(pid, &dir));
        keys.push(dir_key(&output_dir.join("clips").join(dir)));
    }
    keys.dedup();
    keys
}

/// 精修任务的资源键：`clips/clip-NN` 下的目录用片段键（与 cut_one 对得上），
/// 其它目录退回路径键。
pub fn polish_keys(pid: Option<i64>, dir: &Path) -> Keys {
    let mut keys = vec![dir_key(dir)];
    let dir_name = dir.file_name().map(|s| s.to_string_lossy().into_owned());
    let under_clips = dir
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.eq_ignore_ascii_case("clips"))
        .unwrap_or(false);
    if under_clips {
        if let (Some(pid), Some(name)) = (pid, dir_name) {
            keys.push(project_clip(pid, &name));
        }
    }
    keys
}

/// **封面任务**（`polish cover`）的资源域：只与**另一个封面任务**互斥，与渲染 / 后处理 / LLM **并行**。
///
/// 为什么可以并行（不是"先把冲突去掉再说"）：
/// - 封面只写 `cover.jpg` / `_cover_bg.jpg` / `_cover_frame.jpg`，而且同名会由
///   `render::unique_output_path` 加 `_1` / `_2`；
/// - 渲染写的是视频文件，`process` 写的是 `polish.toml` —— 后者每次都是 `write_atomic`，
///   读者只会看到"旧整份"或"新整份"，不会读到半截。
///
/// 以前封面和渲染共用 [`polish_keys`] ⇒ 导出期间点「生成封面」只能排队（用户 2026-10-05 报的
/// "视频导出时无法手动生成封面操作"）。**两个封面**仍然互斥（同一片段上会抢 `_cover_frame.jpg`）。
///
/// ⚠️ 键与 [`polish_keys`] **刻意完全不相交**（冲突判据是字符串相等）⇒ 前端不能再靠
/// `clipKey` 判断封面是否在跑，要按 `JobSummary.kind == "polish_cover"` 门控。
pub fn cover_keys(dir: &Path) -> Keys {
    vec![format!("{}#cover", dir_key(dir))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_one_and_polish_on_same_clip_share_a_key() {
        let pid = 3;
        let out = std::path::PathBuf::from(r"F:\proj\output");
        let cut = cut_keys(pid, &out, Some("clip-07"));
        let pol = polish_keys(Some(pid), &out.join("clips").join("clip-07"));
        assert!(
            crate::jobs::resources_conflict(&cut, &pol),
            "单条重切与同片段精修必须互斥: {cut:?} vs {pol:?}"
        );
    }

    /// 用户 2026-10-05 报的「视频导出时无法手动生成封面」：封面与渲染/后处理**必须不相交**
    /// （以前两者共用 `polish_keys` ⇒ 导出期间生成封面只能排队；前端还按 `clipKey` 把按钮置灰）。
    #[test]
    fn cover_runs_in_parallel_with_render_on_the_same_clip() {
        let pid = 3;
        let clip = std::path::PathBuf::from(r"F:\proj\output\clips\clip-07");
        let cover = cover_keys(&clip);
        let render = polish_keys(Some(pid), &clip);
        assert!(
            !crate::jobs::resources_conflict(&cover, &render),
            "封面不该与渲染/后处理互斥（导出期间要能生成封面）: {cover:?} vs {render:?}"
        );
        // 但**两个封面**在同一片段上仍然互斥（抢 `_cover_frame.jpg`）
        assert!(
            crate::jobs::resources_conflict(&cover, &cover_keys(&clip)),
            "同一片段的两个封面必须互斥"
        );
        // 别的片段的封面互不相干
        let other = std::path::PathBuf::from(r"F:\proj\output\clips\clip-08");
        assert!(
            !crate::jobs::resources_conflict(&cover, &cover_keys(&other)),
            "不同片段的封面互不相干"
        );
    }

    #[test]
    fn cut_all_does_not_block_polish() {
        let pid = 3;
        let out = std::path::PathBuf::from(r"F:\proj\output");
        let cut = cut_keys(pid, &out, None);
        let pol = polish_keys(Some(pid), &out.join("clips").join("clip-02"));
        assert!(
            !crate::jobs::resources_conflict(&cut, &pol),
            "全切只动没有 clip.mp4 的目录，不该挡住精修: {cut:?} vs {pol:?}"
        );
    }

    #[test]
    fn two_polish_jobs_on_different_clips_are_independent() {
        let out = std::path::PathBuf::from(r"F:\proj\output");
        let a = polish_keys(Some(1), &out.join("clips").join("clip-01"));
        let b = polish_keys(Some(1), &out.join("clips").join("clip-02"));
        assert!(!crate::jobs::resources_conflict(&a, &b));
    }

    #[test]
    fn polish_dir_key_matches_even_without_project_scope() {
        let out = std::path::PathBuf::from(r"F:\proj\output");
        let dir = out.join("clips").join("clip-05");
        // 一端有 pid、一端没有：路径键必须兜住
        let with_pid = polish_keys(Some(9), &dir);
        let without = polish_keys(None, &dir);
        assert!(crate::jobs::resources_conflict(&with_pid, &without));
    }

    #[test]
    fn non_clip_polish_dir_only_gets_the_path_key() {
        let keys = polish_keys(Some(1), Path::new(r"F:\proj\polish-here"));
        assert_eq!(keys.len(), 1, "不在 clips/ 下的目录不该蹭到片段键: {keys:?}");
        assert!(keys[0].starts_with("dir:"));
    }
}
