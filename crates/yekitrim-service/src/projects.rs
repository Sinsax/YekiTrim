//! 项目托管：目录布局、占用计量、列表摘要（多用户服务 P2，设计稿 §5.1 / §7）。
//!
//! - **归属由 DB 决定**：用户可见范围 = 他拥有的 `projects` 行；
//! - **目录布局**：`<projects_root>/users/<uid>/<room>/<YYYYMMDD-HHMMSS>-<pid>/`；
//! - **占用计量**：源录播（`input_file` + 根目录里的分片/弹幕）与产物（`output/`），
//!   在入库/切片/渲染/删除后刷新，避免每次请求都全盘遍历；
//! - 本模块**不依赖 `server::*`**（保持 `server → service` 单向）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::db::{Db, ProjectRow};
use super::Workspace;

/// 源录播分片的扩展名（录播姬输出 + 合并后的 source）。
const VIDEO_EXT: [&str; 4] = ["mp4", "flv", "mkv", "ts"];
const DANMAKU_EXT: [&str; 4] = ["xml", "ass", "srt", "json"];

/// 托管项目根：`[storage] projects_root`；留空 → `<exe_dir>/.yekitrim/projects`。
pub fn projects_root() -> PathBuf {
    let cfg = yekitrim_core::config::load_global_config();
    let raw = cfg.storage.projects_root.trim();
    if raw.is_empty() {
        yekitrim_core::path::yekitrim_home().join("projects")
    } else {
        PathBuf::from(raw)
    }
}

/// 租户根：`local` 来源与 `/api/fs/list` 的囚笼基准（§0.2.7）。
pub fn tenant_root(uid: i64) -> PathBuf {
    projects_root().join("users").join(uid.to_string())
}

/// 允许作为"本机文件"来源的根目录列表：**租户根** + `[storage] local_roots`（只保留存在的）。
///
/// 这份列表在管理员场景下只是**快捷入口**（前端下拉预置）；管理员可以直接用任意绝对路径
/// （与改造前的单用户 `yekitrim init <video>` 体验一致）。非管理员一律只能用租户根。
pub fn local_roots(uid: i64) -> Vec<PathBuf> {
    let cfg = yekitrim_core::config::load_global_config();
    let mut out = vec![tenant_root(uid)];
    for raw in &cfg.storage.local_roots {
        let p = PathBuf::from(raw.trim());
        if p.as_os_str().is_empty() {
            continue;
        }
        let canon = p.canonicalize().unwrap_or(p);
        if canon.is_dir() && !out.contains(&canon) {
            out.push(canon);
        }
    }
    out
}

/// 目录解析失败的友好文案：Windows 上 ERROR_INVALID_NAME(123) 基本只有一个原因——
/// 用户粘了 `\\?\` 前缀又混进 `/`（`canonicalize` 的产物被拿去做字符串拼接）。
fn dir_error(p: &Path, e: &std::io::Error) -> anyhow::Error {
    match e.raw_os_error() {
        Some(123) => anyhow::anyhow!(
            "目录路径不合法（{}）：不要用 \\\\?\\ 前缀或混用 / 与 \\，直接填 F:/media/xxx 即可",
            p.display()
        ),
        Some(2) | Some(3) => anyhow::anyhow!("目录不存在: {}", p.display()),
        _ => anyhow::anyhow!("目录不可用（{}）: {e}", p.display()),
    }
}

/// 把前端传来的 `root` 解析成一个目录。
///
/// - 空 / `"tenant"` → 租户根；
/// - 命中 `local_roots` → 用那一项；
/// - **管理员**：任意已存在的绝对目录都接受（= 单用户时代的"随便选位置"）；
/// - 非管理员：只允许租户根，其余报错。
pub fn pick_local_root(uid: i64, hint: &str, is_admin: bool) -> anyhow::Result<PathBuf> {
    let roots = local_roots(uid);
    let tenant = roots[0].clone();
    let hint = hint.trim();
    if hint.is_empty() || hint.eq_ignore_ascii_case("tenant") {
        return Ok(tenant);
    }
    let want = PathBuf::from(hint);
    let want_canon = want.canonicalize().unwrap_or(want.clone());
    if let Some(hit) = roots.iter().find(|r| **r == want_canon || **r == want) {
        return Ok(hit.clone());
    }
    if is_admin {
        if want_canon.is_dir() {
            return Ok(want_canon);
        }
        anyhow::bail!("目录不存在: {hint}");
    }
    anyhow::bail!("只能用你自己的托管目录: {hint}")
}

/// 解析一个"目录"参数：绝对路径直接用（管理员场景 / 兼容旧前端），相对路径按 `root` 囚笼。
pub fn resolve_target_dir(uid: i64, root_hint: &str, dir: &str, is_admin: bool) -> anyhow::Result<PathBuf> {
    let dir = dir.trim();
    if dir.is_empty() {
        return pick_local_root(uid, root_hint, is_admin);
    }
    let p = PathBuf::from(dir);
    if p.is_absolute() {
        if !is_admin {
            // 非管理员只允许"自己租户根内的绝对路径"
            let tenant = tenant_root(uid);
            let tenant_canon = tenant.canonicalize().unwrap_or(tenant);
            let real = p.canonicalize().map_err(|e| dir_error(&p, &e))?;
            if !real.starts_with(&tenant_canon) {
                anyhow::bail!("只能用你自己的托管目录");
            }
            return Ok(real);
        }
        return p.canonicalize().map_err(|e| dir_error(&p, &e));
    }
    let root = pick_local_root(uid, root_hint, is_admin)?;
    yekitrim_core::tenant::jail_existing(&root, dir)
}

/// 根的展示名（租户根显示为「我的托管目录」，其余用目录名）。
pub fn root_label(uid: i64, root: &Path) -> String {
    let tenant = tenant_root(uid);
    let a = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let b = tenant.canonicalize().unwrap_or_else(|_| tenant.clone());
    if a == b {
        "我的托管目录".to_string()
    } else {
        root.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.to_string_lossy().into_owned())
    }
}

/// 把任意外部字符串净化成安全的单层目录名（房间号可能来自 webhook payload）。
#[allow(dead_code)] // P3：录播姬房间号 → 目录名
pub fn sanitize_segment(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    let out: String = trimmed.chars().take(48).collect();
    if out.is_empty() || out == ".." {
        "unassigned".to_string()
    } else {
        out
    }
}

/// 新项目的托管目录（尚不创建）：`.../users/<uid>/<room>/<YYYYMMDD-HHMMSS>-<pid>`。
#[allow(dead_code)] // P3：录播姬入库落位
pub fn new_project_dir(uid: i64, room: &str) -> PathBuf {
    let room = sanitize_segment(if room.trim().is_empty() { "unassigned" } else { room });
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    tenant_root(uid)
        .join(room)
        .join(format!("{stamp}-{}", std::process::id()))
}

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// 递归统计目录占用（不跟随符号链接到目录之外；出错按 0 计）。
pub fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            total += dir_size(&entry.path());
        } else if meta.is_file() {
            total += meta.len();
        }
    }
    total
}

/// 计量一个项目：`(源录播字节, 产物字节)`。
///
/// 源录播 = 配置里的 `input_file`/`danmaku_file` + 项目根下的分片/弹幕文件
/// （按路径去重，避免同一文件算两次）；产物 = `output/` 整棵树。
pub fn measure_project(project: &ProjectRow) -> (u64, u64) {
    let root = project.root_dir.clone();
    let mut counted: HashSet<PathBuf> = HashSet::new();
    let mut source = 0u64;

    if let Ok(ws) = Workspace::load(Some(&project.config_file)) {
        if let Ok(cfg) = ws.load_config() {
            for raw in [
                cfg.project.input_file.clone().unwrap_or_default(),
                cfg.project.danmaku_file.clone().unwrap_or_default(),
                cfg.project.audio_file.clone().unwrap_or_default(),
            ] {
                if raw.trim().is_empty() {
                    continue;
                }
                let path = ws.resolve(&raw);
                let key = path.canonicalize().unwrap_or_else(|_| path.clone());
                if counted.insert(key) {
                    source += file_len(&path);
                }
            }
        }
    }

    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                continue; // output/、.yekitrim/、data/ 等都不是源录播
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !VIDEO_EXT.contains(&ext.as_str()) && !DANMAKU_EXT.contains(&ext.as_str()) {
                continue;
            }
            let key = path.canonicalize().unwrap_or_else(|_| path.clone());
            if counted.insert(key) {
                source += file_len(&path);
            }
        }
    }

    (source, dir_size(&root.join("output")))
}

/// 计量并写回 DB（入库/切片/渲染/删除源之后调用）。
pub fn refresh_bytes(db: &Db, project: &ProjectRow) -> Result<(u64, u64)> {
    let (source, output) = measure_project(project);
    db.update_project_bytes(project.id, source, output)?;
    Ok((source, output))
}

/// 全量刷新所有项目的占用（启动时一次；项目少，成本可接受）。
pub fn refresh_all_bytes(db: &Db) -> usize {
    let projects = db.list_projects(None).unwrap_or_default();
    let mut n = 0;
    for p in &projects {
        if refresh_bytes(db, p).is_ok() {
            n += 1;
        }
    }
    n
}

/// 取项目并校验归属：`is_admin` 为真时可看全部，否则只看自己的。
/// 返回 `None` = 不存在或不属于该用户（上层一律回 **404**，不泄露"存在但无权限"）。
pub fn find_owned(db: &Db, uid: i64, is_admin: bool, id: i64) -> Result<Option<ProjectRow>> {
    let Some(p) = db.project_by_id(id)? else {
        return Ok(None);
    };
    if is_admin || p.owner_id == uid {
        Ok(Some(p))
    } else {
        Ok(None)
    }
}

/// 从 `?project=` 查询参数里取 id。
pub fn parse_project_id(raw: &str) -> Option<i64> {
    raw.trim().parse::<i64>().ok().filter(|id| *id > 0)
}

/// `/api/projects` 的单行摘要（阶段来自项目配置的推导，见 `config::current_stage`）。
pub fn summary_json(project: &ProjectRow, owner_name: Option<&str>) -> serde_json::Value {
    let (stage, has_clips, missing_config) = match Workspace::load(Some(&project.config_file)) {
        Ok(ws) => {
            let snap = crate::project_snapshot(&ws);
            let stage = snap.get("current_stage").and_then(|v| v.as_u64()).unwrap_or(0);
            let has_clips = ws.output_dir().join("clips/clips_order.txt").exists();
            (stage, has_clips, false)
        }
        Err(_) => (0, false, true),
    };
    let missing = project.missing || missing_config;
    serde_json::json!({
        "id": project.id,
        "name": project.name,
        "room": project.room,
        "source": {
            "kind": project.source_kind,
            "label": project.source_label.clone().unwrap_or_default(),
        },
        "stage": stage,
        "has_clips": has_clips,
        "bytes": project.total_bytes(),
        "source_bytes": project.source_bytes,
        "output_bytes": project.output_bytes,
        "missing": missing,
        "created_at": project.created_at,
        "last_active_at": project.last_active_at,
        "owner": owner_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_segment_blocks_path_tricks() {
        assert_eq!(sanitize_segment("23058"), "23058");
        assert_eq!(sanitize_segment("../etc"), ".._etc".trim_start_matches('.').to_string());
        assert_eq!(sanitize_segment("a/b\\c"), "a_b_c");
        assert_eq!(sanitize_segment("   "), "unassigned");
        assert_eq!(sanitize_segment(".."), "unassigned");
        assert_eq!(sanitize_segment("C:"), "C_");
        assert!(sanitize_segment(&"x".repeat(200)).len() <= 48);
    }

    #[test]
    fn new_project_dir_is_inside_tenant_root() {
        let dir = new_project_dir(7, "23058");
        let root = projects_root();
        // 路径形态: <projects_root>/users/7/23058/<stamp>-<pid>
        assert!(dir.starts_with(root.join("users").join("7").join("23058")), "{}", dir.display());
        let leaf = dir.file_name().unwrap().to_string_lossy().to_string();
        assert!(leaf.contains('-'), "目录名应带时间戳与 pid: {leaf}");
    }

    #[test]
    fn dir_size_sums_recursively() {
        let base = std::env::temp_dir().join(format!("yekitrim-measure-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("output/clips/a")).unwrap();
        std::fs::write(base.join("output/clips/a/clip.mp4"), vec![0u8; 100]).unwrap();
        std::fs::write(base.join("output/events.json"), vec![0u8; 20]).unwrap();
        assert_eq!(dir_size(&base.join("output")), 120);
        assert_eq!(dir_size(&base.join("nope")), 0);
        let _ = std::fs::remove_dir_all(&base);
    }
}
