//! 目录布局 v2：`app/`（只读程序件）+ 扁平 `data/`（全部可写）。
//!
//! 2026-11-27 起，安装目录不再有 `.yekitrim/`、`cloud/`、`global/resources/`、
//! `resources/` 这些"要解释才懂"的目录。本模块负责把 0.5 及更早的目录**一次性、
//! 幂等、可回滚**地搬成新形状：
//!
//! ```text
//! 0.5                                   v2
//! <程序>/.yekitrim/server.toml     →   <程序>/data/server.toml
//! <程序>/.yekitrim/cloud/          →   <程序>/data/ledger/
//! <程序>/.yekitrim/models/         →   <程序>/data/models/
//! <程序>/.yekitrim/bin/            →   <程序>/data/bin/
//! <程序>/.yekitrim/projects/       →   <程序>/data/projects/
//! <程序>/.yekitrim/llm_debug/      →   <程序>/data/logs/llm/
//! <程序>/global/resources/<kind>/  →   <程序>/data/shared/<kind>/
//! <程序>/users/<uid>/.yekitrim/*.toml → <程序>/data/users/<uid>/*.toml
//! <程序>/users/<uid>/resources/<kind>/ → <程序>/data/users/<uid>/<kind>/
//! <程序>/runtime/                  →   <程序>/app/runtime/
//! <程序>/dhampir/                  →   <程序>/app/dhampir/
//! ```
//!
//! 约定：
//! - 所有 rename 都要求"目标不存在"；已存在则**跳过并告警**，绝不覆盖。
//! - 未知条目搬到 `data/legacy/<name>`，**不删除**。
//! - 每一步都记进 `layout-migration-journal-<ts>.json`，供 `--rollback` 逆序恢复。
//! - 空壳目录（`.yekitrim/`、`global/`、空 `resources/`）在搬完后删除。
//! - 迁移只做 `rename`；跨卷由调用方决定（本模块不做 copy，避免无声复制 GB 级素材）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 当前目录布局版本（写进 `data/layout.json`）。
pub const LAYOUT_VERSION: u32 = 2;

/// 一次 rename。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Move {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// 迁移计划（纯函数产物，dry-run 直接打印它）。
#[derive(Debug, Clone, Default)]
pub struct LayoutPlan {
    pub program: PathBuf,
    pub data: PathBuf,
    pub moves: Vec<Move>,
    pub deletes: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub already_v2: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    version: u32,
    moves: Vec<Move>,
}

#[derive(Debug, Deserialize)]
struct Marker {
    version: u32,
}

/// `data/layout.json` 里的版本（没有/读不出 = None）。
pub fn marker_version(data_home: &Path) -> Option<u32> {
    let raw = std::fs::read_to_string(data_home.join("layout.json")).ok()?;
    serde_json::from_str::<Marker>(&raw).ok().map(|m| m.version)
}

/// 计算迁移计划。**只读磁盘，不改任何东西**。
pub fn plan(program: &Path, data_override: Option<&Path>) -> LayoutPlan {
    let data_override = data_override.filter(|p| !p.as_os_str().is_empty());
    let data = crate::path::data_dir_in(data_override, program);
    let old_root = if data_override.is_some() {
        data.clone()
    } else {
        program.to_path_buf()
    };

    let mut plan = LayoutPlan {
        program: program.to_path_buf(),
        data: data.clone(),
        already_v2: marker_version(&data).is_some_and(|v| v >= LAYOUT_VERSION),
        ..LayoutPlan::default()
    };

    // 1) 只读程序件：runtime / dhampir → app/
    push_move(&mut plan, &program.join("runtime"), &program.join("app/runtime"));
    push_move(&mut plan, &program.join("dhampir"), &program.join("app/dhampir"));

    // 2) 扁平化状态目录（老根 / 覆盖根都可能有）
    for state in dedup([old_root.join(".yekitrim"), data.join(".yekitrim")]) {
        plan_state(&mut plan, &state, &data);
    }

    // 3) 公用素材：global/resources/<kind> → shared/<kind>
    for global in dedup([old_root.join("global"), data.join("global")]) {
        plan_global(&mut plan, &global, &data);
    }

    // 4) 账号树：users/<uid>/{.yekitrim,resources} → users/<uid>/
    for users in dedup([old_root.join("users"), data.join("users")]) {
        plan_users(&mut plan, &users, &data);
    }

    // 5) 日志：老顶层 logs/ 与 ledger/logs 都并进 data/logs
    let logs_target = data.join("logs");
    for logs in dedup([old_root.join("logs"), data.join("ledger/logs")]) {
        if logs != logs_target {
            merge_children(&mut plan, &logs, &logs_target);
        }
    }
    // `.yekitrim/llm_debug` 已由 plan_state 处理成 logs/llm。

    // 6) 旧 `--data-dir` 语义：覆盖根直挂的账本 → ledger/
    if data_override.is_some() {
        plan_direct_ledger(&mut plan, &data);
    }

    // 7) 旧顶层 resources/（单人或更老版本）：非空搬进 data/resources 留给 owner 迁移；
    //    空壳直接删。
    let old_resources = old_root.join("resources");
    if old_resources != data.join("resources") && old_resources.is_dir() {
        if is_empty_dir(&old_resources) {
            plan.deletes.push(old_resources);
        } else {
            push_move(&mut plan, &old_resources, &data.join("resources"));
        }
    }
    let data_resources = data.join("resources");
    if data_resources.is_dir() && is_empty_dir(&data_resources) {
        plan.deletes.push(data_resources);
    }

    // 8) 空壳：.yekitrim / global / 空 users 目录
    for shell in [
        old_root.join(".yekitrim"),
        data.join(".yekitrim"),
        old_root.join("global"),
        data.join("global"),
    ] {
        if shell.is_dir() && is_empty_dir(&shell) {
            plan.deletes.push(shell);
        }
    }
    // 旧 `users/` 根：账号树整棵搬进 data/ 后只剩空壳。只删空目录。
    for users in dedup([old_root.join("users"), data.join("users")]) {
        if users != data.join("users") && users.is_dir() && is_empty_dir(&users) {
            plan.deletes.push(users);
        }
    }

    plan.deletes.sort();
    plan.deletes.dedup();
    plan
}

fn dedup<const N: usize>(paths: [PathBuf; N]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

fn is_empty_dir(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut it| it.next().is_none())
        .unwrap_or(false)
}

fn push_move(plan: &mut LayoutPlan, from: &Path, to: &Path) {
    if !from.exists() {
        return;
    }
    if to.exists() {
        plan.warnings.push(format!(
            "目标已存在，跳过（不覆盖）: {} → {}",
            from.display(),
            to.display()
        ));
        return;
    }
    plan.moves.push(Move {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
    });
}

/// 把 `from` 目录的子项逐个挪进 `to` 目录（目录合并），最后删空的 `from`。
fn merge_children(plan: &mut LayoutPlan, from: &Path, to: &Path) {
    if !from.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(from) else {
        return;
    };
    for entry in entries.flatten() {
        push_move(plan, &entry.path(), &to.join(entry.file_name()));
    }
    plan.deletes.push(from.to_path_buf());
}

/// `.yekitrim/` 的子项按用途拆到 `data/` 的实名目录。
fn plan_state(plan: &mut LayoutPlan, state: &Path, data: &Path) {
    if !state.is_dir() {
        return;
    }
    let known: [(&str, PathBuf); 7] = [
        ("server.toml", data.join("server.toml")),
        ("cloud", data.join("ledger")),
        ("models", data.join("models")),
        ("bin", data.join("bin")),
        ("projects", data.join("projects")),
        ("llm_debug", data.join("logs/llm")),
        ("logs", data.join("logs")),
    ];
    for (name, to) in &known {
        push_move(plan, &state.join(name), to);
    }
    // 退役的顶层配置：留给启动后的 config 迁移合并到 admin
    for name in ["config.toml", "llm.toml"] {
        push_move(plan, &state.join(name), &data.join("legacy-config").join(name));
    }
    // 未知条目：搬进 legacy/，不删
    if let Ok(entries) = std::fs::read_dir(state) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy().into_owned();
            if known.iter().any(|(n, _)| *n == name_str.as_str())
                || name_str == "config.toml"
                || name_str == "llm.toml"
            {
                continue;
            }
            let dest = data.join("legacy").join(&name);
            push_move(plan, &entry.path(), &dest);
        }
    }
    plan.deletes.push(state.to_path_buf());
}

/// `global/resources/<kind>` → `shared/<kind>`；未知条目进 `legacy/`。
fn plan_global(plan: &mut LayoutPlan, global: &Path, data: &Path) {
    if !global.is_dir() {
        return;
    }
    let resources = global.join("resources");
    let shared = data.join("shared");
    for kind in ["fonts", "stickers", "videos"] {
        push_move(plan, &resources.join(kind), &shared.join(kind));
    }
    if let Ok(entries) = std::fs::read_dir(&resources) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy().into_owned();
            if ["fonts", "stickers", "videos"].contains(&name_str.as_str()) {
                continue;
            }
            let dest = data.join("legacy/global-resources").join(&name);
            push_move(plan, &entry.path(), &dest);
        }
    }
    if let Ok(entries) = std::fs::read_dir(global) {
        for entry in entries.flatten() {
            if entry.file_name() == "resources" {
                continue;
            }
            let dest = data.join("legacy/global").join(entry.file_name());
            push_move(plan, &entry.path(), &dest);
        }
    }
    plan.deletes.push(resources);
    plan.deletes.push(global.to_path_buf());
}

/// `users/<uid>/{.yekitrim,resources}/...` → `users/<uid>/...`。
fn plan_users(plan: &mut LayoutPlan, users: &Path, data: &Path) {
    if !users.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(users) else {
        return;
    };
    let target_users = data.join("users");
    for entry in entries.flatten() {
        let uid_path = entry.path();
        if !uid_path.is_dir() {
            // 不认识的散文件：搬进 legacy/，不删
            push_move(
                plan,
                &uid_path,
                &data.join("legacy/users").join(entry.file_name()),
            );
            continue;
        }
        if uid_path.parent() != Some(target_users.as_path()) {
            // 老根 users/ 下的 uid 目录：整个挪到 data/users/<uid>
            push_move(plan, &uid_path, &target_users.join(entry.file_name()));
            continue;
        }
        // 已经是目标位置：只扁平化它内部的 .yekitrim / resources
        let state = uid_path.join(".yekitrim");
        for name in ["config.toml", "llm.toml"] {
            push_move(plan, &state.join(name), &uid_path.join(name));
        }
        if let Ok(entries) = std::fs::read_dir(&state) {
            for e in entries.flatten() {
                let n = e.file_name();
                let s = n.to_string_lossy().into_owned();
                if s == "config.toml" || s == "llm.toml" {
                    continue;
                }
                let dest = uid_path.join("legacy-state").join(&n);
                push_move(plan, &e.path(), &dest);
            }
        }
        plan.deletes.push(state);

        let resources = uid_path.join("resources");
        for kind in ["fonts", "stickers", "videos"] {
            push_move(plan, &resources.join(kind), &uid_path.join(kind));
        }
        if let Ok(entries) = std::fs::read_dir(&resources) {
            for e in entries.flatten() {
                let n = e.file_name();
                let s = n.to_string_lossy().into_owned();
                if ["fonts", "stickers", "videos"].contains(&s.as_str()) {
                    continue;
                }
                let dest = uid_path.join("legacy-assets").join(&n);
                push_move(plan, &e.path(), &dest);
            }
        }
        plan.deletes.push(resources);
    }
}

/// 旧 `--data-dir` 直挂账本：`<data>/yekitrim.db*` → `<data>/ledger/`。
fn plan_direct_ledger(plan: &mut LayoutPlan, data: &Path) {
    for name in [
        "yekitrim.db",
        "yekitrim.db-wal",
        "yekitrim.db-shm",
        "yekitrim.lock",
        "initial-admin-password.txt",
        "render-procs",
    ] {
        push_move(plan, &data.join(name), &data.join("ledger").join(name));
    }
}

/// 写 marker（`{"version":2}`）。
pub fn write_marker(data_home: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(data_home)?;
    let body = format!("{{\"version\":{LAYOUT_VERSION}}}\n");
    std::fs::write(data_home.join("layout.json"), body)
}

/// 执行迁移。默认不抢占别人的锁；`force` 才接管。
pub fn apply(plan: &LayoutPlan) -> std::io::Result<Vec<Move>> {
    apply_with(plan, false)
}

/// [`apply`] 的强制版（CLI `--force`）。
pub fn apply_with(plan: &LayoutPlan, force: bool) -> std::io::Result<Vec<Move>> {
    if plan.moves.is_empty() {
        for dir in &plan.deletes {
            if is_empty_dir(dir) {
                let _ = std::fs::remove_dir(dir);
            }
        }
        return Ok(Vec::new());
    }

    let _lock = MigrationLock::acquire(&plan.program, force)?;
    let journal = journal_path(&plan.program);
    let mut done: Vec<Move> = Vec::new();

    for mv in &plan.moves {
        if let Some(parent) = mv.to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if mv.to.exists() {
            // plan 阶段已经过滤过一次；执行期再遇到的（并发/半途）不覆盖。
            continue;
        }
        if let Err(e) = std::fs::rename(&mv.from, &mv.to) {
            write_journal(&journal, &done)?;
            return Err(std::io::Error::new(
                e.kind(),
                format!(
                    "目录迁移失败：{} → {}（{e}）。已完成的 {} 项记在 {}，可 `migrate-layout --rollback` 恢复",
                    mv.from.display(),
                    mv.to.display(),
                    done.len(),
                    journal.display()
                ),
            ));
        }
        done.push(mv.clone());
        write_journal(&journal, &done)?;
    }

    for dir in &plan.deletes {
        if is_empty_dir(dir) {
            let _ = std::fs::remove_dir(dir);
        }
    }
    Ok(done)
}

/// 启动时调用：算计划 → 执行 → 写 marker。**幂等**。
pub fn ensure_layout() -> std::io::Result<()> {
    let program = crate::path::program_dir();
    let override_dir = std::env::var_os(crate::path::DATA_DIR_ENV).map(PathBuf::from);
    ensure_layout_at(&program, override_dir.as_deref())
}

/// [`ensure_layout`] 的可注入版本（单测用）。
///
/// ⚠️ 可能要多轮：第一轮把 `users/<uid>/` 整棵挪进 `data/`，第二轮才把它内部的
/// `.yekitrim/` 与 `resources/` 扁平化；`cloud/logs` 也是同理。每轮都重算计划，
/// 直到没有可搬的东西。
pub fn ensure_layout_at(program: &Path, data_override: Option<&Path>) -> std::io::Result<()> {
    let mut last_data = crate::path::data_dir_in(data_override, program);
    for _ in 0..8 {
        let plan = plan(program, data_override);
        last_data = plan.data.clone();
        if plan.moves.is_empty() {
            if !plan.already_v2 {
                write_marker(&plan.data)?;
            }
            for dir in &plan.deletes {
                if is_empty_dir(dir) {
                    let _ = std::fs::remove_dir(dir);
                }
            }
            return Ok(());
        }
        let done = apply(&plan)?;
        if done.is_empty() {
            // 计划里有移动但全部被"目标已存在"挡掉 —— 不覆盖，也不无限重试。
            return Err(std::io::Error::other(format!(
                "目录迁移未能完成：{} 项被目标占用，未做覆盖。告警：{}",
                plan.moves.len(),
                plan.warnings.join("；")
            )));
        }
    }
    Err(std::io::Error::other(format!(
        "目录迁移超过 8 轮仍未收敛（data={}）",
        last_data.display()
    )))
}

/// 回滚 journal（逆序 rename）。仅在新版本尚未启动过时可靠。
pub fn rollback(journal: &Path, force: bool) -> std::io::Result<()> {
    let raw = std::fs::read_to_string(journal)?;
    let journal: Journal = serde_json::from_str(&raw).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("journal 解析失败 {}: {e}", journal.display()),
        )
    })?;
    if journal.version != LAYOUT_VERSION {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("journal 版本 {} 与当前 {} 不符", journal.version, LAYOUT_VERSION),
        ));
    }
    if !force {
        // 新版本启动过就会写 db/日志；此时回滚可能丢新数据，要求显式 --force。
        let marker = journal
            .moves
            .first()
            .and_then(|m| m.to.ancestors().find(|p| p.join("layout.json").is_file()));
        if let Some(data) = marker {
            if marker_version(data).is_some_and(|v| v >= LAYOUT_VERSION) {
                return Err(std::io::Error::other(format!(
                        "{} 已存在 layout.json（新版本可能已启动过）。确要回滚请加 --force",
                        data.display()
                    )));
            }
        }
    }
    for mv in journal.moves.iter().rev() {
        if mv.from.exists() {
            tracing::warn!("回滚跳过（源已存在）: {}", mv.from.display());
            continue;
        }
        if !mv.to.exists() {
            tracing::warn!("回滚跳过（目标不在）: {}", mv.to.display());
            continue;
        }
        if let Some(parent) = mv.from.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&mv.to, &mv.from)?;
    }
    Ok(())
}

fn journal_path(program: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    program.join(format!(
        "layout-migration-journal-{secs}-{}-{seq}.json",
        std::process::id()
    ))
}

fn write_journal(path: &Path, moves: &[Move]) -> std::io::Result<()> {
    let body = serde_json::to_string_pretty(&Journal {
        version: LAYOUT_VERSION,
        moves: moves.to_vec(),
    })
    .map_err(std::io::Error::other)?;
    std::fs::write(path, body)
}

/// 迁移独占锁：`<program>/.layout-migration.lock`。
struct MigrationLock {
    path: PathBuf,
}

impl MigrationLock {
    fn acquire(program: &Path, force: bool) -> std::io::Result<Self> {
        let path = program.join(".layout-migration.lock");
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                use std::io::Write;
                let _ = writeln!(f, "pid={}", std::process::id());
                Ok(Self { path })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .map(|d| d.as_secs() > 600)
                    .unwrap_or(false);
                if force || stale {
                    let _ = std::fs::remove_file(&path);
                    return Self::acquire(program, false);
                }
                Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    format!(
                        "已有迁移在进行（锁 {}）。确认没有别的实例后加 --force 重试",
                        path.display()
                    ),
                ))
            }
            Err(e) => Err(e),
        }
    }
}

impl Drop for MigrationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yekitrim-layout-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, body: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    /// 全新安装：没有可搬的东西，只写 marker。
    #[test]
    fn fresh_install_writes_marker_only() {
        let program = temp_root("fresh");
        let plan = plan(&program, None);
        assert!(plan.moves.is_empty(), "{:?}", plan.moves);
        ensure_layout_at(&program, None).unwrap();
        assert_eq!(marker_version(&program.join("data")), Some(LAYOUT_VERSION));
        assert!(!program.join(".yekitrim").exists());
        let _ = std::fs::remove_dir_all(&program);
    }

    /// 0.5 全量 fixture：每一类都到实名位置，旧壳消失。
    #[test]
    fn migrates_a_0_5_tree() {
        let program = temp_root("v05");
        write(&program.join(".yekitrim/server.toml"), b"port=1");
        write(&program.join(".yekitrim/cloud/yekitrim.db"), b"db");
        write(&program.join(".yekitrim/models/sensevoice/model.onnx"), b"m");
        write(&program.join(".yekitrim/bin/yt-dlp.exe"), b"bin");
        write(&program.join(".yekitrim/projects/users/1/x/yekitrim.toml"), b"p");
        write(&program.join(".yekitrim/llm_debug/a.log"), b"d");
        write(&program.join("users/1/.yekitrim/config.toml"), b"cfg");
        write(&program.join("users/1/.yekitrim/llm.toml"), b"llm");
        write(&program.join("users/1/resources/fonts/f/FONT.toml"), b"f");
        write(&program.join("users/1/resources/stickers/pack/p.toml"), b"s");
        write(&program.join("global/resources/fonts/Noto/FONT.toml"), b"gf");
        write(&program.join("logs/tray.log"), b"log");
        write(&program.join("runtime/polish-to-dhampir.mjs"), b"t");
        write(&program.join("dhampir/bin/dhampir.exe"), b"d");
        std::fs::create_dir_all(program.join("resources")).unwrap();

        ensure_layout_at(&program, None).unwrap();

        let data = program.join("data");
        assert!(data.join("server.toml").is_file());
        assert!(data.join("ledger/yekitrim.db").is_file());
        assert!(data.join("models/sensevoice/model.onnx").is_file());
        assert!(data.join("bin/yt-dlp.exe").is_file());
        assert!(data.join("projects/users/1/x/yekitrim.toml").is_file());
        assert!(data.join("logs/llm/a.log").is_file());
        assert!(data.join("logs/tray.log").is_file());
        assert!(data.join("users/1/config.toml").is_file());
        assert!(data.join("users/1/llm.toml").is_file());
        assert!(data.join("users/1/fonts/f/FONT.toml").is_file());
        assert!(data.join("users/1/stickers/pack/p.toml").is_file());
        assert!(data.join("shared/fonts/Noto/FONT.toml").is_file());
        assert!(program.join("app/runtime/polish-to-dhampir.mjs").is_file());
        assert!(program.join("app/dhampir/bin/dhampir.exe").is_file());
        assert!(!program.join(".yekitrim").exists());
        assert!(!program.join("global").exists());
        assert!(!program.join("resources").exists());
        assert!(!program.join("runtime").exists());
        assert!(!program.join("dhampir").exists());
        assert_eq!(marker_version(&data), Some(LAYOUT_VERSION));
        let _ = std::fs::remove_dir_all(&program);
    }

    /// 第二遍是 no-op（幂等）。
    #[test]
    fn migration_is_idempotent() {
        let program = temp_root("idem");
        write(&program.join(".yekitrim/server.toml"), b"x");
        ensure_layout_at(&program, None).unwrap();
        let plan = plan(&program, None);
        assert!(plan.moves.is_empty(), "{:?}", plan.moves);
        ensure_layout_at(&program, None).unwrap();
        let _ = std::fs::remove_dir_all(&program);
    }

    /// 目标已存在：跳过 + warning，绝不覆盖。
    #[test]
    fn never_overwrites_an_existing_target() {
        let program = temp_root("conflict");
        write(&program.join(".yekitrim/server.toml"), b"old");
        write(&program.join("data/server.toml"), b"new");
        let plan = plan(&program, None);
        assert!(plan.moves.iter().all(|m| m.to != program.join("data/server.toml")));
        assert!(plan.warnings.iter().any(|w| w.contains("server.toml")));
        ensure_layout_at(&program, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(program.join("data/server.toml")).unwrap(),
            "new"
        );
        let _ = std::fs::remove_dir_all(&program);
    }

    /// 覆盖根（`YEKITRIM_DATA_DIR`）里已经是数据根：不搬程序目录里的旧状态。
    #[test]
    fn override_root_is_not_nested_under_program() {
        let program = temp_root("override-program");
        let data = temp_root("override-data");
        write(&data.join(".yekitrim/server.toml"), b"x");
        let plan = plan(&program, Some(&data));
        assert_eq!(plan.data, data);
        assert!(plan.moves.iter().any(|m| m.to == data.join("server.toml")));
        assert!(!plan.moves.iter().any(|m| m.from.starts_with(&program)));
        ensure_layout_at(&program, Some(&data)).unwrap();
        assert!(data.join("server.toml").is_file());
        let _ = std::fs::remove_dir_all(&program);
        let _ = std::fs::remove_dir_all(&data);
    }

    /// rollback 逆序还原。
    #[test]
    fn rollback_restores_the_old_tree() {
        let program = temp_root("rollback");
        write(&program.join(".yekitrim/server.toml"), b"x");
        ensure_layout_at(&program, None).unwrap();
        let journal = std::fs::read_dir(&program)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("layout-migration-journal-"))
            })
            .expect("journal");
        rollback(&journal, true).unwrap();
        assert!(program.join(".yekitrim/server.toml").is_file());
        assert!(!program.join("data/server.toml").exists());
        let _ = std::fs::remove_dir_all(&program);
    }

    /// 非空 resources 不删，空壳才删。
    #[test]
    fn only_empty_shells_are_deleted() {
        let program = temp_root("shells");
        std::fs::create_dir_all(program.join("resources")).unwrap();
        let empty_plan = plan(&program, None);
        assert!(empty_plan.deletes.contains(&program.join("resources")));
        write(&program.join("resources/a.mp4"), b"x");
        let nonempty_plan = plan(&program, None);
        assert!(!nonempty_plan.deletes.contains(&program.join("resources")));
        let _ = std::fs::remove_dir_all(&program);
    }
}
