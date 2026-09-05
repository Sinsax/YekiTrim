//! 项目状态快照：CLI `status` 与 WebUI `/api/project/status` 共用。

use std::path::{Path, PathBuf};

use serde_json::json;

use super::Workspace;

/// 将 output 相对路径规范化（去掉 ./ 或 .. 组件）。
fn absolute_output(ws: &Workspace, output: &Path) -> PathBuf {
    let root = &ws.root_dir;
    let rel = output.strip_prefix(root).unwrap_or(output);
    let mut joined = root.clone();
    for comp in rel.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => { joined.pop(); }
            other => joined.push(other.as_os_str()),
        }
    }
    joined
}

/// 生成当前项目结构化快照（JSON）。
pub fn project_snapshot(ws: &Workspace) -> serde_json::Value {
    let cfg = ws.load_config().ok();
    let output = ws.output_dir();
    let current = cfg.as_ref().map(|c| c.project.current_stage(&output)).unwrap_or(0);
    let input = cfg.as_ref().and_then(|c| c.project.input_file.clone()).unwrap_or_default();
    let input_abs = if input.is_empty() { String::new() } else { ws.resolve(&input).to_string_lossy().into_owned() };
    let mut files = vec![];
    for (rel, name) in [
        ("video.wav", "音频"),
        ("video.srt", "字幕"),
        ("transcript.json", "转录 JSON"),
        ("events.json", "事件"),
        ("clips/clips_order.txt", "片段顺序"),
        ("publish_config.json", "发布配置"),
    ] {
        let p = output.join(rel);
        if p.exists() { files.push(format!("{name}: {}", p.to_string_lossy())); }
    }
    let project_name = ws.root_dir.file_name().and_then(|s| s.to_str()).unwrap_or("工程").to_string();
    json!({
        "config_file": ws.config_file.to_string_lossy(),
        "root_dir": ws.root_dir.to_string_lossy(),
        "name": project_name,
        "output_dir": absolute_output(ws, &output).to_string_lossy(),
        "input_file": input,
        "input_abs": input_abs,
        "current_stage": current,
        "files": files,
        "config": cfg.map(|c| serde_json::to_value(c).unwrap_or_default()).unwrap_or(serde_json::Value::Null),
    })
}

/// 项目摘要（列表页用）。
pub fn project_summary(ws: &Workspace) -> serde_json::Value {
    let cfg = ws.load_config().ok();
    let output = ws.output_dir();
    let current = cfg.as_ref().map(|c| c.project.current_stage(&output)).unwrap_or(0);
    let input = cfg.as_ref().and_then(|c| c.project.input_file.clone()).unwrap_or_default();
    let input_abs = if input.is_empty() { String::new() } else { ws.resolve(&input).to_string_lossy().into_owned() };
    let clips = output.join("clips");
    let clip_videos = if clips.exists() {
        std::fs::read_dir(&clips).map(|entries| entries.flatten().filter(|e| e.path().is_dir()).count()).unwrap_or(0)
    } else { 0 };
    json!({
        "id": ws.root_dir.file_name().and_then(|s| s.to_str()).unwrap_or("project").to_string(),
        "dir": ws.root_dir.to_string_lossy(),
        "config_file": ws.config_file.to_string_lossy(),
        "name": cfg.as_ref().and_then(|c| c.project.input_file.as_ref()).map(|p| {
            Path::new(p).file_stem().and_then(|x| x.to_str()).unwrap_or("project").to_string()
        }).unwrap_or_else(|| "未命名工程".into()),
        "input_file": input,
        "input_abs": input_abs,
        "stage": current,
        "output_dir": absolute_output(ws, &output).to_string_lossy(),
        "clip_videos": clip_videos,
        "has_danmaku": cfg.as_ref().and_then(|c| c.project.danmaku_file.as_ref()).map(|p| !p.is_empty()).unwrap_or(false),
        "files": project_snapshot(ws).get("files").cloned().unwrap_or(serde_json::json!([])),
    })
}
