//! 账号账本（SQLite）。
//!
//! P1 最小切片只落 `users` + `audit`（供 `vtrim user` 使用）；`sessions` 表一并建好，
//! HTTP 登录/会话在下一步接入。库位置：`$VTRIM_DATA_DIR/vtrim.db`，
//! 未设置时取 `<exe_dir>/.vtrim/cloud/vtrim.db`（与设计稿 §3.5.7 一致）。
//!
//! 身份数据不可缺省：打不开/写不了库一律返回错误，由调用方升级为"启动失败 + 修复指引"。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

/// 账本 schema 版本（`PRAGMA user_version`）；新增迁移时递增并在 `migrate()` 中分支。
/// - v1：users / sessions / audit
/// - v2：projects（项目托管）+ storage（占用计量）
pub const SCHEMA_VERSION: i64 = 2;

pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("VTRIM_DATA_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    vtrim_core::path::vtrim_home().join("cloud")
}

pub fn db_path() -> PathBuf {
    data_dir().join("vtrim.db")
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub disabled: bool,
    pub must_change_password: bool,
    pub quota_bytes: i64,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
}

/// 会话 + 账号快照（登录校验与 `/api/me` 都读这一份）。
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub user_id: i64,
    pub csrf: String,
    pub expires_at: i64,
    pub username: String,
    pub role: String,
    pub disabled: bool,
    pub must_change_password: bool,
}

/// 项目行（归属由 DB 决定：用户可见范围 = 他拥有的 projects 行，§5.1）。
#[derive(Debug, Clone)]
pub struct ProjectRow {
    pub id: i64,
    pub owner_id: i64,
    pub name: String,
    /// 房间号（录播姬来源）
    pub room: Option<String>,
    /// `local` | `recorder` | `bilibili`
    pub source_kind: String,
    /// 展示用来源文字（`录制 23058` / `B站 av123`）
    pub source_label: Option<String>,
    pub root_dir: PathBuf,
    pub config_file: PathBuf,
    /// 录播姬会话 id（P3 入库时写入，用于"同一场直播只建一个项目"）
    #[allow(dead_code)]
    pub session_id: Option<String>,
    pub created_at: i64,
    pub last_active_at: Option<i64>,
    pub source_bytes: u64,
    pub output_bytes: u64,
    /// 目录/vtrim.toml 已不存在（列表里显示"缺失"而不是直接消失）
    pub missing: bool,
}

impl ProjectRow {
    pub fn total_bytes(&self) -> u64 {
        self.source_bytes + self.output_bytes
    }
}

pub struct Db {
    conn: Mutex<Connection>,
    /// 库文件位置（迁移前备份用）
    path: PathBuf,
}impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("无法创建数据目录 {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("无法打开账号账本 {}", path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .context("初始化 SQLite PRAGMA 失败")?;
        let db = Self { conn: Mutex::new(conn), path: path.to_path_buf() };
        db.migrate()?;
        Ok(db)
    }

    /// 库文件位置。
    #[allow(dead_code)] // 诊断/备份路径展示用
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn open_default() -> Result<Self> {
        Self::open(&db_path())
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        f(&conn)
    }

    pub fn migrate(&self) -> Result<()> {
        let current: i64 = self.with(|conn| Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?))?;
        if current >= SCHEMA_VERSION {
            return Ok(());
        }
        // 迁移前备份（§3.5.8）：只有"已存在的旧库"才需要，新库 current==0
        if current > 0 {
            let backup = self.path.with_file_name(format!(
                "{}.bak-{current}",
                self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "vtrim.db".into())
            ));
            // WAL 下用 VACUUM INTO 拿到一致快照；失败退回文件拷贝
            let vac = self.with(|conn| {
                conn.execute("VACUUM INTO ?1", params![backup.to_string_lossy()])
                    .map_err(|e| anyhow::anyhow!(e))
            });
            if vac.is_err() {
                let _ = std::fs::copy(&self.path, &backup);
            }
            tracing::info!("账号账本 schema 迁移 v{current} → v{SCHEMA_VERSION}，已备份到 {}", backup.display());
        }
        self.with(|conn| {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS users (
                    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
                    username             TEXT NOT NULL UNIQUE,
                    password_hash        TEXT NOT NULL,
                    must_change_password INTEGER NOT NULL DEFAULT 0,
                    role                 TEXT NOT NULL DEFAULT 'user',
                    quota_bytes          INTEGER NOT NULL DEFAULT 0,
                    disabled             INTEGER NOT NULL DEFAULT 0,
                    created_at           INTEGER NOT NULL,
                    last_login_at        INTEGER
                );
                CREATE TABLE IF NOT EXISTS sessions (
                    id         TEXT PRIMARY KEY,
                    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                    csrf       TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL,
                    user_agent TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
                CREATE TABLE IF NOT EXISTS audit (
                    id      INTEGER PRIMARY KEY AUTOINCREMENT,
                    at      INTEGER NOT NULL,
                    user_id INTEGER,
                    action  TEXT NOT NULL,
                    detail  TEXT,
                    ip      TEXT
                );
                -- v2：项目托管（归属由 DB 决定，路径面归零的地基，§5.1）
                CREATE TABLE IF NOT EXISTS projects (
                    id             INTEGER PRIMARY KEY AUTOINCREMENT,
                    owner_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                    name           TEXT NOT NULL,
                    room           TEXT,
                    source_kind    TEXT NOT NULL DEFAULT 'local',
                    source_label   TEXT,
                    root_dir       TEXT NOT NULL,
                    config_file    TEXT NOT NULL,
                    session_id     TEXT,
                    created_at     INTEGER NOT NULL,
                    last_active_at INTEGER,
                    source_bytes   INTEGER NOT NULL DEFAULT 0,
                    output_bytes   INTEGER NOT NULL DEFAULT 0,
                    missing        INTEGER NOT NULL DEFAULT 0
                );
                CREATE UNIQUE INDEX IF NOT EXISTS idx_projects_config ON projects(config_file);
                CREATE INDEX IF NOT EXISTS idx_projects_owner ON projects(owner_id);
                CREATE TABLE IF NOT EXISTS storage (
                    user_id    INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
                    bytes      INTEGER NOT NULL DEFAULT 0,
                    updated_at INTEGER NOT NULL
                );
                "#,
            )
            .context("建表失败")?;
            conn.execute_batch(&format!("PRAGMA user_version={SCHEMA_VERSION};"))?;
            Ok(())
        })
    }

    pub fn count_users(&self) -> Result<i64> {
        self.with(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?))
    }

    pub fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        role: &str,
        must_change_password: bool,
        quota_bytes: i64,
    ) -> Result<i64> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO users (username, password_hash, must_change_password, role, quota_bytes, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![username, password_hash, must_change_password as i64, role, quota_bytes, now_secs()],
            )
            .with_context(|| format!("创建账号 {username} 失败（用户名是否已存在？）"))?;
            Ok(conn.last_insert_rowid())
        })
    }

    pub fn list_users(&self) -> Result<Vec<UserRow>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, username, role, disabled, must_change_password, quota_bytes, created_at, last_login_at
                 FROM users ORDER BY id",
            )?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(UserRow {
                        id: r.get(0)?,
                        username: r.get(1)?,
                        role: r.get(2)?,
                        disabled: r.get::<_, i64>(3)? != 0,
                        must_change_password: r.get::<_, i64>(4)? != 0,
                        quota_bytes: r.get(5)?,
                        created_at: r.get(6)?,
                        last_login_at: r.get(7)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn find_user(&self, username: &str) -> Result<Option<UserRow>> {
        self.with(|conn| {
            let row = conn
                .query_row(
                    "SELECT id, username, role, disabled, must_change_password, quota_bytes, created_at, last_login_at
                     FROM users WHERE username = ?1",
                    params![username],
                    |r| {
                        Ok(UserRow {
                            id: r.get(0)?,
                            username: r.get(1)?,
                            role: r.get(2)?,
                            disabled: r.get::<_, i64>(3)? != 0,
                            must_change_password: r.get::<_, i64>(4)? != 0,
                            quota_bytes: r.get(5)?,
                            created_at: r.get(6)?,
                            last_login_at: r.get(7)?,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
    }

    /// 设置密码；`must_change` 为 true 表示仍是"初始/重置密码"。返回是否命中账号。
    pub fn set_password(&self, username: &str, password_hash: &str, must_change: bool) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute(
                "UPDATE users SET password_hash = ?2, must_change_password = ?3 WHERE username = ?1",
                params![username, password_hash, must_change as i64],
            )?;
            Ok(n > 0)
        })
    }

    pub fn set_disabled(&self, username: &str, disabled: bool) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute(
                "UPDATE users SET disabled = ?2 WHERE username = ?1",
                params![username, disabled as i64],
            )?;
            Ok(n > 0)
        })
    }

    /// 取某账号的密码哈希（仅供校验/诊断；不放进 `UserRow`，避免列表接口带出哈希）。
    pub fn user_password_hash(&self, username: &str) -> Result<Option<String>> {
        self.with(|conn| {
            let hash = conn
                .query_row(
                    "SELECT password_hash FROM users WHERE username = ?1",
                    params![username],
                    |r| r.get::<_, String>(0),
                )
                .optional()?;
            Ok(hash)
        })
    }

    pub fn audit(&self, user_id: Option<i64>, action: &str, detail: &str) -> Result<()> {
        self.audit_ip(user_id, action, detail, None)
    }

    pub fn audit_ip(
        &self,
        user_id: Option<i64>,
        action: &str,
        detail: &str,
        ip: Option<&str>,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO audit (at, user_id, action, detail, ip) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![now_secs(), user_id, action, detail, ip],
            )?;
            Ok(())
        })
    }

    /// 统计某动作的审计条数（`vtrim doctor` 的安全摘要用）。
    pub fn count_audit(&self, action: &str) -> Result<i64> {
        self.with(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM audit WHERE action = ?1",
                params![action],
                |r| r.get(0),
            )?)
        })
    }

    /// 统计未过期会话数。
    pub fn count_sessions(&self, now: i64) -> Result<i64> {
        self.with(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM sessions WHERE expires_at > ?1",
                params![now],
                |r| r.get(0),
            )?)
        })
    }

    // ---- 项目托管（v2） ----

    #[allow(clippy::too_many_arguments)]
    pub fn create_project(
        &self,
        owner_id: i64,
        name: &str,
        room: Option<&str>,
        source_kind: &str,
        source_label: Option<&str>,
        root_dir: &Path,
        config_file: &Path,
        session_id: Option<&str>,
    ) -> Result<i64> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO projects
                   (owner_id, name, room, source_kind, source_label, root_dir, config_file,
                    session_id, created_at, last_active_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                params![
                    owner_id,
                    name,
                    room,
                    source_kind,
                    source_label,
                    root_dir.to_string_lossy(),
                    config_file.to_string_lossy(),
                    session_id,
                    now_secs(),
                ],
            )
            .with_context(|| format!("登记项目 {name} 失败（同一 vtrim.toml 是否已登记？）"))?;
            Ok(conn.last_insert_rowid())
        })
    }

    fn project_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRow> {
        Ok(ProjectRow {
            id: r.get(0)?,
            owner_id: r.get(1)?,
            name: r.get(2)?,
            room: r.get(3)?,
            source_kind: r.get(4)?,
            source_label: r.get(5)?,
            root_dir: PathBuf::from(r.get::<_, String>(6)?),
            config_file: PathBuf::from(r.get::<_, String>(7)?),
            session_id: r.get(8)?,
            created_at: r.get(9)?,
            last_active_at: r.get(10)?,
            source_bytes: r.get::<_, i64>(11)?.max(0) as u64,
            output_bytes: r.get::<_, i64>(12)?.max(0) as u64,
            missing: r.get::<_, i64>(13)? != 0,
        })
    }

    const PROJECT_COLS: &'static str = "id, owner_id, name, room, source_kind, source_label, \
         root_dir, config_file, session_id, created_at, last_active_at, source_bytes, output_bytes, missing";

    pub fn project_by_id(&self, id: i64) -> Result<Option<ProjectRow>> {
        self.with(|conn| {
            let row = conn
                .query_row(
                    &format!("SELECT {} FROM projects WHERE id = ?1", Self::PROJECT_COLS),
                    params![id],
                    Self::project_from_row,
                )
                .optional()?;
            Ok(row)
        })
    }

    pub fn project_by_config(&self, config_file: &Path) -> Result<Option<ProjectRow>> {
        self.with(|conn| {
            let row = conn
                .query_row(
                    &format!("SELECT {} FROM projects WHERE config_file = ?1", Self::PROJECT_COLS),
                    params![config_file.to_string_lossy()],
                    Self::project_from_row,
                )
                .optional()?;
            Ok(row)
        })
    }

    /// `owner=None` = 全部项目（仅 admin 用）。
    pub fn list_projects(&self, owner: Option<i64>) -> Result<Vec<ProjectRow>> {
        self.with(|conn| {
            let mut out = Vec::new();
            match owner {
                Some(uid) => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {} FROM projects WHERE owner_id = ?1 ORDER BY id",
                        Self::PROJECT_COLS
                    ))?;
                    let rows = stmt.query_map(params![uid], Self::project_from_row)?;
                    for r in rows {
                        out.push(r?);
                    }
                }
                None => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {} FROM projects ORDER BY owner_id, id",
                        Self::PROJECT_COLS
                    ))?;
                    let rows = stmt.query_map([], Self::project_from_row)?;
                    for r in rows {
                        out.push(r?);
                    }
                }
            }
            Ok(out)
        })
    }

    pub fn delete_project(&self, id: i64) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
            Ok(n > 0)
        })
    }

    /// 记录项目最近活动时间（打开/跑管线时调用）。
    #[allow(dead_code)]
    pub fn touch_project(&self, id: i64) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE projects SET last_active_at = ?2 WHERE id = ?1",
                params![id, now_secs()],
            )?;
            Ok(())
        })
    }

    pub fn update_project_bytes(&self, id: i64, source_bytes: u64, output_bytes: u64) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE projects SET source_bytes = ?2, output_bytes = ?3 WHERE id = ?1",
                params![id, source_bytes as i64, output_bytes as i64],
            )?;
            Ok(())
        })
    }

    pub fn set_project_missing(&self, id: i64, missing: bool) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE projects SET missing = ?2 WHERE id = ?1",
                params![id, missing as i64],
            )?;
            Ok(())
        })
    }

    /// 占用汇总（按用户；`None` = 全局）。storage 表在 P5 做增量维护，
    /// 这里直接从 projects 汇总，避免两个真相来源。
    #[allow(dead_code)] // P5：`/api/storage` 与管理员页用
    pub fn storage_summary(&self, owner: Option<i64>) -> Result<Vec<(i64, u64)>> {
        self.with(|conn| {
            let mut out = Vec::new();
            let sql = match owner {
                Some(_) => "SELECT owner_id, SUM(source_bytes + output_bytes) FROM projects \
                            WHERE owner_id = ?1 GROUP BY owner_id",
                None => "SELECT owner_id, SUM(source_bytes + output_bytes) FROM projects \
                         GROUP BY owner_id",
            };
            let mut stmt = conn.prepare(sql)?;
            let map = |r: &rusqlite::Row<'_>| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?.max(0) as u64));
            match owner {
                Some(uid) => {
                    for r in stmt.query_map(params![uid], map)? {
                        out.push(r?);
                    }
                }
                None => {
                    for r in stmt.query_map([], map)? {
                        out.push(r?);
                    }
                }
            }
            Ok(out)
        })
    }

    // ---- 会话（HTTP 登录） ----
    /// 建会话行。`id`/`csrf` 都是 CSPRNG 令牌（见 `service::auth`）。
    pub fn create_session(
        &self,
        id: &str,
        user_id: i64,
        csrf: &str,
        expires_at: i64,
        user_agent: Option<&str>,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, user_id, csrf, created_at, expires_at, user_agent)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, user_id, csrf, now_secs(), expires_at, user_agent],
            )
            .context("创建会话失败")?;
            Ok(())
        })
    }

    /// 按会话 id 取会话 + 账号快照（不过滤过期，由调用方判定，便于给出"已过期"文案）。
    pub fn session(&self, id: &str) -> Result<Option<SessionRow>> {
        self.with(|conn| {
            let row = conn
                .query_row(
                    "SELECT s.id, s.user_id, s.csrf, s.expires_at,
                            u.username, u.role, u.disabled, u.must_change_password
                     FROM sessions s JOIN users u ON u.id = s.user_id
                     WHERE s.id = ?1",
                    params![id],
                    |r| {
                        Ok(SessionRow {
                            id: r.get(0)?,
                            user_id: r.get(1)?,
                            csrf: r.get(2)?,
                            expires_at: r.get(3)?,
                            username: r.get(4)?,
                            role: r.get(5)?,
                            disabled: r.get::<_, i64>(6)? != 0,
                            must_change_password: r.get::<_, i64>(7)? != 0,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
    }

    /// 滑动续期；返回是否命中会话行。
    pub fn touch_session(&self, id: &str, expires_at: i64) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute(
                "UPDATE sessions SET expires_at = ?2 WHERE id = ?1",
                params![id, expires_at],
            )?;
            Ok(n > 0)
        })
    }

    pub fn delete_session(&self, id: &str) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
            Ok(n > 0)
        })
    }

    /// 删除某账号的其他会话（改密/禁用后清理），`keep` 为要保留的当前会话 id。
    pub fn delete_user_sessions(&self, user_id: i64, keep: Option<&str>) -> Result<usize> {
        self.with(|conn| {
            let n = match keep {
                Some(k) => conn.execute(
                    "DELETE FROM sessions WHERE user_id = ?1 AND id <> ?2",
                    params![user_id, k],
                )?,
                None => conn.execute("DELETE FROM sessions WHERE user_id = ?1", params![user_id])?,
            };
            Ok(n)
        })
    }

    pub fn purge_expired_sessions(&self, now: i64) -> Result<usize> {
        self.with(|conn| {
            let n = conn.execute("DELETE FROM sessions WHERE expires_at <= ?1", params![now])?;
            Ok(n)
        })
    }

    pub fn touch_last_login(&self, user_id: i64) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE users SET last_login_at = ?2 WHERE id = ?1",
                params![user_id, now_secs()],
            )?;
            Ok(())
        })
    }

    /// 按 id 改密（HTTP 侧用；CLI 侧按用户名走 `set_password`）。
    pub fn set_password_by_id(&self, user_id: i64, hash: &str, must_change: bool) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute(
                "UPDATE users SET password_hash = ?2, must_change_password = ?3 WHERE id = ?1",
                params![user_id, hash, must_change as i64],
            )?;
            Ok(n > 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("vtrim.db")).unwrap();
        (dir, db)
    }

    #[test]
    fn migrate_is_idempotent_and_starts_empty() {
        let (_d, db) = tmp_db();
        assert_eq!(db.count_users().unwrap(), 0);
        db.migrate().unwrap(); // 二次迁移不应报错
        assert_eq!(db.count_users().unwrap(), 0);
    }

    #[test]
    fn create_list_and_unique_username() {
        let (_d, db) = tmp_db();
        db.create_user("admin", "hash1", "admin", true, 0).unwrap();
        db.create_user("alice", "hash2", "user", false, 1024).unwrap();
        assert!(db.create_user("alice", "hash3", "user", false, 0).is_err(), "用户名必须唯一");

        let users = db.list_users().unwrap();
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].username, "admin");
        assert!(users[0].must_change_password);
        assert_eq!(users[1].quota_bytes, 1024);
    }

    #[test]
    fn session_lifecycle_and_user_join() {
        let (_d, db) = tmp_db();
        let uid = db.create_user("admin", "hash", "admin", true, 0).unwrap();
        db.create_session("sid-1", uid, "csrf-1", 1000, Some("pytest")).unwrap();

        let s = db.session("sid-1").unwrap().expect("会话应存在");
        assert_eq!(s.username, "admin");
        assert_eq!(s.role, "admin");
        assert_eq!(s.csrf, "csrf-1");
        assert!(s.must_change_password);
        assert!(!s.disabled);
        assert_eq!(s.expires_at, 1000);

        assert!(db.touch_session("sid-1", 2000).unwrap());
        assert!(db.session("sid-1").unwrap().unwrap().expires_at == 2000);

        db.touch_last_login(uid).unwrap();
        assert!(db.find_user("admin").unwrap().unwrap().last_login_at.is_some());

        // 级联：删账号会带走会话（foreign_keys=ON）
        assert!(db.delete_session("sid-1").unwrap());
        assert!(!db.delete_session("sid-1").unwrap());
        assert!(db.session("sid-1").unwrap().is_none());
    }

    #[test]
    fn expired_sessions_are_purged_and_scoped_deletes_work() {
        let (_d, db) = tmp_db();
        let a = db.create_user("a", "h", "admin", false, 0).unwrap();
        let b = db.create_user("b", "h", "user", false, 0).unwrap();
        db.create_session("a1", a, "c", 100, None).unwrap();
        db.create_session("a2", a, "c", 9_999, None).unwrap();
        db.create_session("b1", b, "c", 9_999, None).unwrap();

        assert_eq!(db.purge_expired_sessions(200).unwrap(), 1);
        assert!(db.session("a1").unwrap().is_none());
        assert!(db.session("a2").unwrap().is_some());

        // 改密只清掉"其他"会话，当前会话保留
        assert_eq!(db.delete_user_sessions(a, Some("a2")).unwrap(), 0);
        assert_eq!(db.delete_user_sessions(b, None).unwrap(), 1);
        assert!(db.session("b1").unwrap().is_none());
        assert!(db.session("a2").unwrap().is_some());

        // 会话里的账号快照跟着用户表走（禁用/改密即时可见）
        assert!(db.set_disabled("a", true).unwrap());
        assert!(db.session("a2").unwrap().unwrap().disabled);
        assert!(db.set_password_by_id(a, "h2", false).unwrap());
        assert!(!db.session("a2").unwrap().unwrap().must_change_password);
    }

    #[test]
    fn audit_records_ip_column() {
        let (_d, db) = tmp_db();
        let uid = db.create_user("admin", "h", "admin", false, 0).unwrap();
        db.audit_ip(Some(uid), "login_failed", "bad password", Some("127.0.0.1")).unwrap();
        assert_eq!(db.count_audit("login_failed").unwrap(), 1);
    }

    #[test]
    fn password_and_disabled_flags_round_trip() {
        let (_d, db) = tmp_db();
        db.create_user("admin", "old", "admin", true, 0).unwrap();
        assert!(db.set_password("admin", "new", false).unwrap());
        assert!(!db.set_password("ghost", "x", false).unwrap());

        let u = db.find_user("admin").unwrap().unwrap();
        assert!(!u.must_change_password);
        assert!(!u.disabled);
        assert!(db.set_disabled("admin", true).unwrap());
        assert!(db.find_user("admin").unwrap().unwrap().disabled);
    }
}
