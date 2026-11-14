//! 账号账本（SQLite）。
//!
//! P1 最小切片只落 `users` + `audit`（供 `yekitrim user` 使用）；`sessions` 表一并建好，
//! HTTP 登录/会话在下一步接入。库位置：`$YEKITRIM_DATA_DIR/yekitrim.db`，
//! 未设置时取 `<exe_dir>/.yekitrim/cloud/yekitrim.db`（与设计稿 §3.5.7 一致）。
//!
//! 身份数据不可缺省：打不开/写不了库一律返回错误，由调用方升级为"启动失败 + 修复指引"。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

/// 账本 schema 版本（`PRAGMA user_version`）；新增迁移时递增并在 `migrate()` 中分支。
/// - v1：users / sessions / audit
/// - v2：projects（项目托管）+ storage（占用计量）
/// - v3：录播姬采集（ingest_events / ingest_sessions / ingest_files / channel_map）
/// - v4：invites（邀请码：自助注册用的一次性 token）
/// - v6：channel_map 加 `title`（房间标题缓存）与 `source`（user=用户自己加的 / admin=兜底指派）
/// - v5：invites 改为存**明文**（管理员要能重复查看，用户要求）——
///   不再存 selector + scrypt 哈希：明文既然要能读回来，哈希就不再提供任何保护，留着只是噪声。
///   一个码仍然只能绑一个账号（`used_at/used_by` + 原子消费）。
/// - v7：llm_usage（每次 LLM 调用一行：tokens / 缓存命中 / 耗时 / 成败，设置页「LLM 用量」用）
/// - v8：ingest_sessions 加 `kind` / `source_ref` —— 同一张会话表被两个来源共用
///   （`recorder` 录播姬 / `bilibili` 链接下载）。**必须显式判别**：B站是"用户主动提交、
///   有身份、进程退出即完结"，录播姬是"webhook 无身份、进待分配池、靠空闲窗口完结"，
///   两条流程的语义完全不同，混在一起会互相踩（见 `docs/plan/p4-implementation-plan.md` §2）。
pub const SCHEMA_VERSION: i64 = 8;

/// 账号账本 / 会话 / 初始密码所在目录（`<数据目录>/.yekitrim/cloud`）。
///
/// ⚠️ **这是 [`yekitrim_core::path::data_dir`] 的第二个实现，不是它的调用者** ——
/// 它多一级 `cloud`，且**在有覆盖时返回覆盖值本身**（不是覆盖值 + `/cloud`）。
/// 这个"两种拼接规则"是刻意的（`--data-dir` 指的就是账本目录），但**改环境变量名时
/// 必须与 `yekitrim-core/src/path.rs` 的 `DATA_DIR_ENV` 和 `server/src/lib.rs` 的
/// `set_var` 三端成对改** —— 只改一端 = `--data-dir` 静默失效、CLI 与 server 读**两个不同的库**，
/// 而且只有传了 `--data-dir` 才暴露。
pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var(yekitrim_core::path::DATA_DIR_ENV) {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    yekitrim_core::path::yekitrim_home().join("cloud")
}

pub fn db_path() -> PathBuf {
    data_dir().join("yekitrim.db")
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 邀请码行的列清单 + 行映射（几处查询共用，列序必须与 `invite_from_row` 一致）。
const INVITE_SELECT: &str = "SELECT id, token, note, created_by, created_at,
                                    expires_at, used_by, used_at, revoked
                             FROM invites";

fn channel_rule_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChannelRule> {
    Ok(ChannelRule {
        room_id: r.get(0)?,
        user_id: r.get(1)?,
        auto_pipeline: r.get(2)?,
        enabled: r.get::<_, i64>(3)? != 0,
        title: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
        source: r.get::<_, Option<String>>(5)?.unwrap_or_else(|| "admin".into()),
    })
}

fn invite_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<InviteRow> {
    Ok(InviteRow {
        id: r.get(0)?,
        token: r.get(1)?,
        note: r.get(2)?,
        created_by: r.get(3)?,
        created_at: r.get(4)?,
        expires_at: r.get(5)?,
        used_by: r.get(6)?,
        used_at: r.get(7)?,
        revoked: r.get::<_, i64>(8)? != 0,
    })
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

/// 邀请码行（v5）。**明文就存在库里**（管理员可重复查看、复制再发；用户要求），
/// 因此不再有 selector/哈希那一套。一次性由 `used_at` 保证。
#[derive(Debug, Clone)]
pub struct InviteRow {
    pub id: i64,
    /// `yekitrim_inv_<43 字符 base64url>`
    pub token: String,
    pub note: Option<String>,
    pub created_by: Option<i64>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub used_by: Option<i64>,
    pub used_at: Option<i64>,
    pub revoked: bool,
}

impl InviteRow {
    /// 还能用来注册：未使用、未撤销、未过期。
    pub fn is_open(&self, now: i64) -> bool {
        self.used_at.is_none() && !self.revoked && self.expires_at.map(|e| e > now).unwrap_or(true)
    }
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
    /// 目录/yekitrim.toml 已不存在（列表里显示"缺失"而不是直接消失）
    pub missing: bool,
}

impl ProjectRow {
    pub fn total_bytes(&self) -> u64 {
        self.source_bytes + self.output_bytes
    }
}

/// 录播姬会话（P3）：一场直播可能推送多个事件、输出多个分片。
#[derive(Debug, Clone)]
pub struct IngestSessionRow {
    pub session_id: String,
    pub room_id: Option<String>,
    pub short_id: Option<String>,
    /// `None` = 尚未指派（进"待分配池"，仅 admin 可见）
    pub owner_id: Option<i64>,
    /// `open` | `closing` | `finalized` | `failed` | `unassigned`
    ///（B站下载另用 `downloading`，见 `kind`）
    pub state: String,
    pub opened_files: Vec<String>,
    pub closed_files: Vec<String>,
    pub first_event_at: Option<i64>,
    pub last_event_at: Option<i64>,
    pub project_id: Option<i64>,
    pub auto_pipeline: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    /// 来源判别列（v8）：`recorder`（默认，录播姬）/ `bilibili`（链接下载）。
    ///
    /// 这张表被两个来源共用（复用的理由：重试端点、记录页、容量预检、SSE 全是现成的）。
    /// ⚠️ 别拿 `room_id` 塞 bvid 来区分来源 —— `room_id` 会被 `channel_map`
    /// 按房间号查的规则命中，把 B站会话误判成某个直播间。
    pub kind: String,
    /// 来源侧标识：录播姬侧留 `None`（用 `room_id`），B站侧放 bvid（多P 用 `BV1xx-p3`）。
    pub source_ref: Option<String>,
}

/// 房间 → 账号 的自动导入规则（§4.2）。
#[derive(Debug, Clone)]
pub struct ChannelRule {
    pub room_id: String,
    pub user_id: i64,
    pub auto_pipeline: String,
    pub enabled: bool,
    /// 房间标题缓存（用户加的直播间列表要显示）
    pub title: String,
    /// `user` = 用户自己在界面上加的；`admin` = 管理员兜底指派的
    pub source: String,
}

/// 兜底扫描见过的文件（§4.1）。P3b/P3c 的 worker 会读这些字段。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct IngestFileRow {
    pub path: String,
    pub session_id: Option<String>,
    pub room_id: Option<String>,
    pub size: u64,
    pub mtime: i64,
    pub state: String,
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
                self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "yekitrim.db".into())
            ));
            // WAL 下用 VACUUM INTO 拿到一致快照；失败退回文件拷贝
            let vac = self.with(|conn| {
                conn.execute("VACUUM INTO ?1", params![backup.to_string_lossy()])
                    .map_err(|e| anyhow::anyhow!(e))
            });
            if vac.is_err() {
                if let Err(e) = std::fs::copy(&self.path, &backup) {
                    // ⚠️ 备份是 schema 迁移前**唯一的退路**，失败必须留痕：
                    // 以前是 `let _ =`，真出问题时才发现"备份根本没生成"。
                    tracing::error!(
                        "迁移前备份失败（VACUUM INTO 与文件拷贝都没成），继续迁移有风险: {e}"
                    );
                }
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
                -- v3：录播姬采集（webhook 去重 / 会话聚合 / 兜底扫描 / 房间映射，§4）
                CREATE TABLE IF NOT EXISTS ingest_events (
                    event_id    TEXT PRIMARY KEY,
                    received_at INTEGER NOT NULL,
                    kind        TEXT NOT NULL,
                    session_id  TEXT
                );
                CREATE TABLE IF NOT EXISTS ingest_sessions (
                    session_id     TEXT PRIMARY KEY,
                    room_id        TEXT,
                    short_id       TEXT,
                    owner_id       INTEGER REFERENCES users(id) ON DELETE SET NULL,
                    state          TEXT NOT NULL,
                    opened_files   TEXT NOT NULL DEFAULT '[]',
                    closed_files   TEXT NOT NULL DEFAULT '[]',
                    first_event_at INTEGER,
                    last_event_at  INTEGER,
                    project_id     INTEGER REFERENCES projects(id) ON DELETE SET NULL,
                    auto_pipeline  TEXT,
                    error          TEXT,
                    created_at     INTEGER NOT NULL,
                    -- v8：来源判别（recorder | bilibili）+ 来源侧标识（bvid）
                    kind           TEXT NOT NULL DEFAULT 'recorder',
                    source_ref     TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_ingest_sessions_owner ON ingest_sessions(owner_id);
                CREATE INDEX IF NOT EXISTS idx_ingest_sessions_state ON ingest_sessions(state);
                CREATE TABLE IF NOT EXISTS ingest_files (
                    path       TEXT PRIMARY KEY,
                    session_id TEXT,
                    room_id    TEXT,
                    size       INTEGER NOT NULL DEFAULT 0,
                    mtime      INTEGER NOT NULL DEFAULT 0,
                    state      TEXT NOT NULL DEFAULT 'seen',
                    seen_at    INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS channel_map (
                    room_id       TEXT PRIMARY KEY,
                    user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                    auto_pipeline TEXT NOT NULL DEFAULT 'detect',
                    enabled       INTEGER NOT NULL DEFAULT 1,
                    title         TEXT,
                    source        TEXT NOT NULL DEFAULT 'admin'
                );
                -- v4/v5：邀请码（自助注册用的一次性 token）
                -- **明文落库**：管理员要能随时翻出来再发一次（用户要求）；
                -- 一次性由 used_at/used_by 保证 —— 一个码只能绑一个账号。
                CREATE TABLE IF NOT EXISTS invites (
                    id         INTEGER PRIMARY KEY AUTOINCREMENT,
                    token      TEXT NOT NULL UNIQUE,
                    note       TEXT,
                    created_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER,
                    used_by    INTEGER REFERENCES users(id) ON DELETE SET NULL,
                    used_at    INTEGER,
                    revoked    INTEGER NOT NULL DEFAULT 0
                );
                -- v7：LLM 用量（每次调用一行；采集点在 yekitrim_llm::LlmClient::send）
                -- label = 链路名（事件检测 / 字幕订正 / 精修段事件…），project = 项目或切片目录。
                -- attempts = 这一行包含了多少次 HTTP 尝试（重试不另起一行）。
                CREATE TABLE IF NOT EXISTS llm_usage (
                    id                INTEGER PRIMARY KEY AUTOINCREMENT,
                    ts                INTEGER NOT NULL,
                    label             TEXT NOT NULL,
                    project           TEXT,
                    model             TEXT NOT NULL DEFAULT '',
                    host              TEXT NOT NULL DEFAULT '',
                    prompt_tokens     INTEGER NOT NULL DEFAULT 0,
                    completion_tokens INTEGER NOT NULL DEFAULT 0,
                    cached_tokens     INTEGER NOT NULL DEFAULT 0,
                    reasoning_tokens  INTEGER NOT NULL DEFAULT 0,
                    latency_ms        INTEGER NOT NULL DEFAULT 0,
                    attempts          INTEGER NOT NULL DEFAULT 1,
                    ok                INTEGER NOT NULL DEFAULT 1,
                    status            INTEGER,
                    error             TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_llm_usage_ts ON llm_usage(ts);
                "#,
            )
            .context("建表失败")?;
            // v4 → v5：老 invites 表只有哈希（取不回明文），重建表并把历史迁过来
            // （已用的码保留 used_by/used_at，这样"谁用过"不丢；token 用占位串，界面上标注为旧码）
            let has_token = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('invites') WHERE name = 'token'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(1)
                > 0;
            if !has_token {
                conn.execute_batch(
                    r#"
                    ALTER TABLE invites RENAME TO invites_v4;
                    CREATE TABLE invites (
                        id         INTEGER PRIMARY KEY AUTOINCREMENT,
                        token      TEXT NOT NULL UNIQUE,
                        note       TEXT,
                        created_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
                        created_at INTEGER NOT NULL,
                        expires_at INTEGER,
                        used_by    INTEGER REFERENCES users(id) ON DELETE SET NULL,
                        used_at    INTEGER,
                        revoked    INTEGER NOT NULL DEFAULT 0
                    );
                    INSERT INTO invites (id, token, note, created_by, created_at, expires_at, used_by, used_at, revoked)
                        SELECT id, '<旧邀请码 #' || id || '（明文不可查看，请重新生成）>', note, created_by,
                               created_at, expires_at, used_by, used_at, revoked
                        FROM invites_v4;
                    DROP TABLE invites_v4;
                    "#,
                )
                .context("v4 → v5 迁移失败")?;
            }
            // v5 → v6：channel_map 补 title/source（用户自助加的直播间要记标题与来源）
            for (col, ddl) in [
                ("title", "ALTER TABLE channel_map ADD COLUMN title TEXT"),
                ("source", "ALTER TABLE channel_map ADD COLUMN source TEXT NOT NULL DEFAULT 'admin'"),
            ] {
                let has = conn
                    .query_row(
                        "SELECT COUNT(*) FROM pragma_table_info('channel_map') WHERE name = ?1",
                        params![col],
                        |r| r.get::<_, i64>(0),
                    )
                    .unwrap_or(1)
                    > 0;
                if !has {
                    conn.execute_batch(ddl).with_context(|| format!("加列 {col} 失败"))?;
                }
            }
            // v7 → v8：ingest_sessions 加 kind/source_ref（B站下载会话与录播姬共用这张表）。
            // 默认值 `'recorder'`：已存在的行全是录播姬会话，不用回填也能语义正确。
            for (col, ddl) in [
                (
                    "kind",
                    "ALTER TABLE ingest_sessions ADD COLUMN kind TEXT NOT NULL DEFAULT 'recorder'",
                ),
                ("source_ref", "ALTER TABLE ingest_sessions ADD COLUMN source_ref TEXT"),
            ] {
                let has = conn
                    .query_row(
                        "SELECT COUNT(*) FROM pragma_table_info('ingest_sessions') WHERE name = ?1",
                        params![col],
                        |r| r.get::<_, i64>(0),
                    )
                    .unwrap_or(1)
                    > 0;
                if !has {
                    conn.execute_batch(ddl).with_context(|| format!("加列 {col} 失败"))?;
                }
            }
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

    /// 设置某账号的**配额上限**（字节，`0` = 跟随 `[storage] default_quota_bytes`）。
    ///
    /// ⚠️ **`0` 不等于"不限"**，是"跟随全局默认"（见 `projects::effective_quota_bytes`）。
    /// 账号级与全局级都是 `0` 时才真的不限 —— 这正是全新安装的状态。
    /// 返回值 = 有没有这个账号（`false` 时调用方应回 404，别静默成功）。
    ///
    /// 为什么需要它：注册流程固定写 `quota_bytes = 0`，此前**没有任何地方能改这个字段** ——
    /// 于是"每用户配额"这条链实际上只有全局默认一个旋钮可用。
    pub fn set_quota(&self, username: &str, quota_bytes: i64) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute(
                "UPDATE users SET quota_bytes = ?2 WHERE username = ?1",
                params![username, quota_bytes.max(0)],
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

    /// 统计某动作的审计条数（`yekitrim doctor` 的安全摘要用）。
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
            .with_context(|| format!("登记项目 {name} 失败（同一 yekitrim.toml 是否已登记？）"))?;
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

    /// 按 `yekitrim.toml` 路径反查**某账号名下**的项目（2026-11-14 后端 review B2）：
    /// `/api/recorder/recordings` 的 `added`/`project_id` 只反映调用者自己的登记 ——
    /// 别人在同一目录登记的项目，id 不许回给普通用户。
    pub fn project_by_config_for(
        &self,
        config_file: &Path,
        owner_id: i64,
    ) -> Result<Option<ProjectRow>> {
        self.with(|conn| {
            let row = conn
                .query_row(
                    &format!(
                        "SELECT {} FROM projects WHERE config_file = ?1 AND owner_id = ?2",
                        Self::PROJECT_COLS
                    ),
                    params![config_file.to_string_lossy(), owner_id],
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

    // ---- 邀请码（v4/v5：明文落库，管理员可重复查看） ----

    /// 落一条邀请码（明文直接入库）。
    pub fn create_invite(
        &self,
        token: &str,
        note: Option<&str>,
        created_by: Option<i64>,
        expires_at: Option<i64>,
    ) -> Result<i64> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO invites (token, note, created_by, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![token, note, created_by, now_secs(), expires_at],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// 按明文取（唯一索引命中一行）。注册校验走它，没有 KDF 开销。
    pub fn find_invite_by_token(&self, token: &str) -> Result<Option<InviteRow>> {
        self.with(|conn| {
            let row = conn
                .query_row(&format!("{INVITE_SELECT} WHERE token = ?1"), params![token], invite_from_row)
                .optional()?;
            Ok(row)
        })
    }

    pub fn find_invite_by_id(&self, id: i64) -> Result<Option<InviteRow>> {
        self.with(|conn| {
            let row = conn
                .query_row(&format!("{INVITE_SELECT} WHERE id = ?1"), params![id], invite_from_row)
                .optional()?;
            Ok(row)
        })
    }

    /// 全部邀请码（新的在前），管理页/CLI 列表用。
    pub fn list_invites(&self) -> Result<Vec<InviteRow>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!("{INVITE_SELECT} ORDER BY id DESC"))?;
            let rows = stmt.query_map([], invite_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// 撤销（幂等）：已撤销/不存在都返回影响行数 0，调用方据此报错。
    pub fn revoke_invite(&self, id: i64) -> Result<usize> {
        self.with(|conn| {
            let n = conn.execute("UPDATE invites SET revoked = 1 WHERE id = ?1 AND revoked = 0", params![id])?;
            Ok(n)
        })
    }

    /// **原子**完成「消费邀请码 + 建账号」：注册两个人同时用同一个码时，只有一个人能建成。
    ///
    /// 用户名查重在调用方做（这里再判一次 `UNIQUE` 约束兜底）。返回新账号 id。
    pub fn register_with_invite(
        &self,
        invite_id: i64,
        username: &str,
        password_hash: &str,
    ) -> Result<i64> {
        self.with(|conn| {
            let now = now_secs();
            let used = conn.execute(
                "UPDATE invites SET used_at = ?2
                 WHERE id = ?1 AND used_at IS NULL AND revoked = 0
                   AND (expires_at IS NULL OR expires_at > ?2)",
                params![invite_id, now],
            )?;
            if used == 0 {
                bail!("邀请码无效或已被使用");
            }
            conn.execute(
                "INSERT INTO users (username, password_hash, must_change_password, role, quota_bytes, created_at)
                 VALUES (?1, ?2, 0, 'user', 0, ?3)",
                params![username, password_hash, now],
            )
            .with_context(|| format!("创建账号 {username} 失败（用户名是否已存在？）"))?;
            let user_id = conn.last_insert_rowid();
            // 归属回填：账号 id 要等 INSERT 之后才有
            conn.execute(
                "UPDATE invites SET used_by = ?2 WHERE id = ?1",
                params![invite_id, user_id],
            )?;
            Ok(user_id)
        })
    }

    /// 大小写不敏感地查账号：注册时挡掉 `Admin` 这类与既有账号同名不同大小写的冒充。
    pub fn find_user_ci(&self, username: &str) -> Result<Option<UserRow>> {
        self.with(|conn| {
            let row = conn
                .query_row(
                    "SELECT id, username, role, disabled, must_change_password, quota_bytes, created_at, last_login_at
                     FROM users WHERE username = ?1 COLLATE NOCASE LIMIT 1",
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

    // ---- 录播姬采集（v3，§4） ----

    /// 记录一个 webhook 事件；返回 `false` = 重复投递（同 `EventId` 已处理过）。
    pub fn ingest_event_record(&self, event_id: &str, kind: &str, session_id: Option<&str>) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute(
                "INSERT OR IGNORE INTO ingest_events (event_id, received_at, kind, session_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![event_id, now_secs(), kind, session_id],
            )?;
            Ok(n > 0)
        })
    }

    /// 新建一个**非录播姬来源**的会话（目前只有 B站链接下载）。
    ///
    /// 与 [`Db::ingest_session_touch`] 的区别（这也是为什么不复用它）：
    /// - `owner_id` 立刻有值 —— B站是**用户主动提交**的，不需要"待分配池 + admin 指派"；
    /// - `kind` 由调用方指定（`touch` 永远写默认 `recorder`）；
    /// - `room_id`/`short_id` 一律留空，来源标识走 `source_ref`（避免撞 `channel_map`）。
    ///
    /// 幂等：`session_id` 已存在时**不覆盖** `kind`/`owner_id`/`state`，只刷新 `last_event_at`
    /// 并补上缺失的 `source_ref`/`auto_pipeline` —— 重试同一份下载不该把状态打回去。
    ///
    /// 返回 `true` = 本次真的新建了（调用方据此决定要不要记日志/建目录）。
    pub fn create_ingest_session(
        &self,
        session_id: &str,
        kind: &str,
        owner_id: i64,
        source_ref: Option<&str>,
        state: &str,
        auto_pipeline: Option<&str>,
    ) -> Result<bool> {
        self.with(|conn| {
            let now = now_secs();
            // 先查存在性：`INSERT OR IGNORE` 分不出"新建"还是"已存在"
            let exists = conn
                .query_row(
                    "SELECT COUNT(*) FROM ingest_sessions WHERE session_id = ?1",
                    params![session_id],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            if exists {
                conn.execute(
                    "UPDATE ingest_sessions
                     SET last_event_at = ?2,
                         source_ref = COALESCE(source_ref, ?3),
                         auto_pipeline = COALESCE(auto_pipeline, ?4)
                     WHERE session_id = ?1",
                    params![session_id, now, source_ref, auto_pipeline],
                )?;
                return Ok(false);
            }
            conn.execute(
                "INSERT INTO ingest_sessions
                   (session_id, kind, owner_id, source_ref, state, auto_pipeline,
                    first_event_at, last_event_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?7)",
                params![session_id, kind, owner_id, source_ref, state, auto_pipeline, now],
            )?;
            Ok(true)
        })
    }

    /// 会话不存在则建（`state=open`），已存在则刷新 `last_event_at` / 房间信息。
    pub fn ingest_session_touch(
        &self,
        session_id: &str,
        room_id: Option<&str>,
        short_id: Option<&str>,
    ) -> Result<()> {
        self.with(|conn| {
            let now = now_secs();
            conn.execute(
                "INSERT INTO ingest_sessions (session_id, room_id, short_id, state, first_event_at, last_event_at, created_at)
                 VALUES (?1, ?2, ?3, 'open', ?4, ?4, ?4)
                 ON CONFLICT(session_id) DO UPDATE SET
                   last_event_at = ?4,
                   room_id = COALESCE(ingest_sessions.room_id, excluded.room_id),
                   short_id = COALESCE(ingest_sessions.short_id, excluded.short_id)",
                params![session_id, room_id, short_id, now],
            )?;
            Ok(())
        })
    }

    fn ingest_session_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<IngestSessionRow> {
        let parse_list = |raw: String| -> Vec<String> {
            // 读侧：解析失败按空处理是安全的（不会写回），但必须留痕 ——
            // 否则"记着 3 个文件"静默变成"0 个"，没有任何地方能看出来。
            match serde_json::from_str::<Vec<String>>(&raw) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("ingest_sessions 的文件列表 JSON 解析失败（按空处理）: {e}");
                    Vec::new()
                }
            }
        };
        Ok(IngestSessionRow {
            session_id: r.get(0)?,
            room_id: r.get(1)?,
            short_id: r.get(2)?,
            owner_id: r.get(3)?,
            state: r.get(4)?,
            opened_files: parse_list(r.get::<_, String>(5)?),
            closed_files: parse_list(r.get::<_, String>(6)?),
            first_event_at: r.get(7)?,
            last_event_at: r.get(8)?,
            project_id: r.get(9)?,
            auto_pipeline: r.get(10)?,
            error: r.get(11)?,
            created_at: r.get(12)?,
            // v8 的新列**追加在 SELECT 末尾**，这样 0..12 的索引不用动（少改一处就少错一处）
            kind: r.get(13)?,
            source_ref: r.get(14)?,
        })
    }

    const INGEST_SESSION_COLS: &'static str = "session_id, room_id, short_id, owner_id, state, \
         opened_files, closed_files, first_event_at, last_event_at, project_id, auto_pipeline, error, created_at, \
         kind, source_ref";

    pub fn ingest_session(&self, session_id: &str) -> Result<Option<IngestSessionRow>> {
        self.with(|conn| {
            let row = conn
                .query_row(
                    &format!(
                        "SELECT {} FROM ingest_sessions WHERE session_id = ?1",
                        Self::INGEST_SESSION_COLS
                    ),
                    params![session_id],
                    Self::ingest_session_from_row,
                )
                .optional()?;
            Ok(row)
        })
    }

    /// `owner=None` = 全平台（admin）；`states` 为空 = 不限状态。按最近事件倒序。
    pub fn list_ingest_sessions(
        &self,
        owner: Option<i64>,
        states: &[&str],
        limit: usize,
    ) -> Result<Vec<IngestSessionRow>> {
        self.with(|conn| {
            let mut sql = format!(
                "SELECT {} FROM ingest_sessions WHERE 1=1",
                Self::INGEST_SESSION_COLS
            );
            let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            if let Some(uid) = owner {
                sql.push_str(" AND owner_id = ?");
                args.push(Box::new(uid));
            }
            if !states.is_empty() {
                let placeholders = vec!["?"; states.len()].join(",");
                sql.push_str(&format!(" AND state IN ({placeholders})"));
                for s in states {
                    args.push(Box::new((*s).to_string()));
                }
            }
            sql.push_str(" ORDER BY COALESCE(last_event_at, created_at) DESC LIMIT ?");
            args.push(Box::new(limit as i64));
            let mut stmt = conn.prepare(&sql)?;
            let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
            let rows = stmt.query_map(refs.as_slice(), Self::ingest_session_from_row)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    pub fn ingest_session_add_file(&self, session_id: &str, rel_path: &str, opened: bool) -> Result<()> {
        self.with(|conn| {
            let col = if opened { "opened_files" } else { "closed_files" };
            let current: Option<String> = conn
                .query_row(
                    &format!("SELECT {col} FROM ingest_sessions WHERE session_id = ?1"),
                    params![session_id],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(current) = current else { return Ok(()) };
            // ⚠️ 解析失败**绝不能**当成空列表再写回去：那会把这一列里本来记着的其它路径
            // **永久丢掉**（`opened_files`/`closed_files` 是 JSON 数组）。损坏时唯一安全的
            // 动作是"这一次不写 + 记 error"。
            let mut list: Vec<String> = match serde_json::from_str(&current) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(
                        "解析 ingest_sessions.{col} 失败（session={session_id}），本次不写入以免覆盖原值: {e}"
                    );
                    return Ok(());
                }
            };
            if !list.iter().any(|p| p == rel_path) {
                list.push(rel_path.to_string());
            }
            // 同理：序列化失败也不能写 "[]" —— 那等于把这一列清空
            let encoded = match serde_json::to_string(&list) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("序列化 ingest_sessions.{col} 失败，本次不写入: {e}");
                    return Ok(());
                }
            };
            conn.execute(
                &format!(
                    "UPDATE ingest_sessions SET {col} = ?2, last_event_at = ?3 WHERE session_id = ?1"
                ),
                params![session_id, encoded, now_secs()],
            )?;
            Ok(())
        })
    }

    pub fn set_ingest_state(&self, session_id: &str, state: &str, error: Option<&str>) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE ingest_sessions SET state = ?2, error = COALESCE(?3, error) WHERE session_id = ?1",
                params![session_id, state, error],
            )?;
            Ok(())
        })
    }

    /// 指派归属（待分配池 → 某账号 + 自动策略），**并把它推到可入库状态**。
    ///
    /// ⚠️ 待分配的会话要被指派后置成 `closing`（不是 `finalized`）：
    /// 入库 worker 的完结判定 `is_finalized()` 只认 `state == "closing"`，而 `assign` 完
    /// 只在 `state='unassigned'` 时才改状态。以前这里写成 `'finalized'`，于是「指派并入库」
    /// 只改了归属、**永远不入库**（真实踩过：指派后会话停在 `finalized` 且 `project_id` 一直为空，
    /// 得再点一次「重试」把它推回 closing 才会跑）。
    ///
    /// `'finalized'` 是"已经建完项目"的状态（由 [`Db::set_ingest_project`] 写），别混用。
    pub fn assign_ingest_session(
        &self,
        session_id: &str,
        owner_id: i64,
        auto_pipeline: Option<&str>,
    ) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute(
                "UPDATE ingest_sessions
                 SET owner_id = ?2, auto_pipeline = COALESCE(?3, auto_pipeline),
                     state = CASE WHEN state = 'unassigned' THEN 'closing' ELSE state END
                 WHERE session_id = ?1",
                params![session_id, owner_id, auto_pipeline],
            )?;
            Ok(n > 0)
        })
    }

    /// P3b 入库 worker 用（合并/落位后把会话指向新建的项目）。
    #[allow(dead_code)]
    pub fn set_ingest_project(&self, session_id: &str, project_id: i64) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE ingest_sessions SET project_id = ?2, state = 'finalized' WHERE session_id = ?1",
                params![session_id, project_id],
            )?;
            Ok(())
        })
    }

    pub fn upsert_ingest_file(
        &self,
        path: &str,
        session_id: Option<&str>,
        room_id: Option<&str>,
        size: u64,
        mtime: i64,
        state: &str,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO ingest_files (path, session_id, room_id, size, mtime, state, seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(path) DO UPDATE SET
                   session_id = COALESCE(excluded.session_id, ingest_files.session_id),
                   room_id = COALESCE(excluded.room_id, ingest_files.room_id),
                   size = excluded.size, mtime = excluded.mtime,
                   state = excluded.state, seen_at = excluded.seen_at",
                params![path, session_id, room_id, size as i64, mtime, state, now_secs()],
            )?;
            Ok(())
        })
    }

    /// P3b 入库 worker 用（合并/落位后把会话指向新建的项目）。
    #[allow(dead_code)]
    pub fn ingest_file(&self, path: &str) -> Result<Option<IngestFileRow>> {        self.with(|conn| {
            let row = conn
                .query_row(
                    "SELECT path, session_id, room_id, size, mtime, state FROM ingest_files WHERE path = ?1",
                    params![path],
                    |r| {
                        Ok(IngestFileRow {
                            path: r.get(0)?,
                            session_id: r.get(1)?,
                            room_id: r.get(2)?,
                            size: r.get::<_, i64>(3)?.max(0) as u64,
                            mtime: r.get(4)?,
                            state: r.get(5)?,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
    }

    /// 一组录像文件的总字节（会话列表的"大小"列）。
    pub fn ingest_file_sizes(&self, paths: &[String]) -> Result<u64> {
        if paths.is_empty() {
            return Ok(0);
        }
        self.with(|conn| {
            let placeholders = vec!["?"; paths.len()].join(",");
            let sql = format!("SELECT COALESCE(SUM(size), 0) FROM ingest_files WHERE path IN ({placeholders})");
            let mut stmt = conn.prepare(&sql)?;
            let args: Vec<&dyn rusqlite::ToSql> =
                paths.iter().map(|p| p as &dyn rusqlite::ToSql).collect();
            let total: i64 = stmt.query_row(args.as_slice(), |r| r.get(0))?;
            Ok(total.max(0) as u64)
        })
    }

    /// P3c 兜底扫描用。
    #[allow(dead_code)]
    pub fn list_ingest_files(&self, limit: usize) -> Result<Vec<IngestFileRow>> {        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT path, session_id, room_id, size, mtime, state FROM ingest_files
                 ORDER BY seen_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], |r| {
                Ok(IngestFileRow {
                    path: r.get(0)?,
                    session_id: r.get(1)?,
                    room_id: r.get(2)?,
                    size: r.get::<_, i64>(3)?.max(0) as u64,
                    mtime: r.get(4)?,
                    state: r.get(5)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// 某用户的直播间（用户自己加的 + 管理员给他指派的都在内）。
    pub fn rooms_of_user(&self, user_id: i64) -> Result<Vec<ChannelRule>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT room_id, user_id, auto_pipeline, enabled, title, source
                 FROM channel_map WHERE user_id = ?1 ORDER BY room_id",
            )?;
            let rows = stmt.query_map(params![user_id], channel_rule_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// 用户自助加直播间：写/覆盖一行（`source='user'`）。
    ///
    /// 同一个房间号被**别人**先登记过时：`POST /api/rooms`（用户自助路径）会先查到归属并
    /// 返回 409「这个直播间已被其他用户登记」——先到先得，不会静默改归属；能覆盖的只有
    /// admin 的路径（`/api/channel-map` 全量替换、或 admin 替人指派）。
    pub fn room_upsert(&self, room_id: &str, user_id: i64, title: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO channel_map (room_id, user_id, auto_pipeline, enabled, title, source)
                 VALUES (?1, ?2, 'detect', 1, ?3, 'user')
                 ON CONFLICT(room_id) DO UPDATE SET
                   user_id = excluded.user_id, title = excluded.title,
                   source = 'user', enabled = 1",
                params![room_id, user_id, title],
            )?;
            Ok(())
        })
    }

    /// 删直播间登记。`owner` 为 `Some(uid)` 时只能删自己的（普通用户）；`None` = admin 全权。
    pub fn room_delete(&self, room_id: &str, owner: Option<i64>) -> Result<usize> {
        self.with(|conn| {
            let n = match owner {
                Some(uid) => conn.execute(
                    "DELETE FROM channel_map WHERE room_id = ?1 AND user_id = ?2",
                    params![room_id, uid],
                )?,
                None => conn.execute("DELETE FROM channel_map WHERE room_id = ?1", params![room_id])?,
            };
            Ok(n)
        })
    }

    pub fn channel_map_list(&self) -> Result<Vec<ChannelRule>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT room_id, user_id, auto_pipeline, enabled, title, source FROM channel_map ORDER BY room_id",
            )?;
            let rows = stmt.query_map([], channel_rule_from_row)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// 整体替换某用户的房间规则（前端 PUT 的语义）。
    ///
    /// 返回 **被别的账号占着的房间号**：非空时**整批不生效**（事务不提交），调用方回 409。
    /// `allow_takeover = true`（admin）时不做这项检查 —— 与 `POST /api/rooms` 同一口径
    /// （那里 admin 也能接管别人的登记）。
    ///
    /// ⚠️ **为什么必须查这一下**：`room_id` 是 `channel_map` 的**全局主键**，而 upsert 的
    /// `DO UPDATE SET user_id = excluded.user_id` 原来是无条件的 ⇒ 任何账号发一条
    /// `PUT /api/channel-map {"rules":[{"room_id":"<别人的房间>"}]}` 就能把归属改到自己名下
    /// （2026-11-14 后端 review 的 P1：之后那台机器的录像会入库到抢的人账号里，
    /// 或者被 `enabled:false` 直接掐掉）。**未被任何人登记**的房间仍然允许自助登记。
    pub fn replace_channel_map(
        &self,
        user_id: i64,
        allow_takeover: bool,
        rules: &[(String, String, bool)],
    ) -> Result<Vec<String>> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            if !allow_takeover {
                let mut conflicts: Vec<String> = Vec::new();
                {
                    let mut stmt = tx.prepare("SELECT room_id FROM channel_map WHERE user_id != ?1")?;
                    let taken: std::collections::HashSet<String> = stmt
                        .query_map(params![user_id], |r| r.get::<_, String>(0))?
                        .collect::<rusqlite::Result<_>>()?;
                    for (room_id, _, _) in rules {
                        if taken.contains(room_id) {
                            conflicts.push(room_id.clone());
                        }
                    }
                }
                if !conflicts.is_empty() {
                    // 直接返回 ⇒ 事务未提交（Drop 即回滚），本次一条规则都不动
                    return Ok(conflicts);
                }
            }
            tx.execute("DELETE FROM channel_map WHERE user_id = ?1", params![user_id])?;
            for (room_id, pipeline, enabled) in rules {
                tx.execute(
                    "INSERT INTO channel_map (room_id, user_id, auto_pipeline, enabled)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(room_id) DO UPDATE SET
                       user_id = excluded.user_id, auto_pipeline = excluded.auto_pipeline,
                       enabled = excluded.enabled",
                    params![room_id, user_id, pipeline, *enabled as i64],
                )?;
            }
            tx.commit()?;
            Ok(Vec::new())
        })
    }

    pub fn channel_for_room(&self, room_id: &str) -> Result<Option<ChannelRule>> {
        self.with(|conn| {
            let row = conn
                .query_row(
                    "SELECT room_id, user_id, auto_pipeline, enabled, title, source FROM channel_map WHERE room_id = ?1",
                    params![room_id],
                    channel_rule_from_row,
                )
                .optional()?;
            Ok(row)
        })
    }

    /// 待分配池：还没有归属的会话（仅 admin 可见并指派）。
    pub fn unassigned_ingest_sessions(&self, limit: usize) -> Result<Vec<IngestSessionRow>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {} FROM ingest_sessions
                 WHERE owner_id IS NULL AND state IN ('open','closing','finalized','unassigned')
                 ORDER BY COALESCE(last_event_at, created_at) DESC LIMIT ?1",
                Self::INGEST_SESSION_COLS
            ))?;
            let rows = stmt.query_map(params![limit as i64], Self::ingest_session_from_row)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// 按 id 查账号（会话/指派接口用）。
    pub fn find_user_by_id(&self, id: i64) -> Result<Option<UserRow>> {
        self.with(|conn| {
            let row = conn
                .query_row(
                    "SELECT id, username, role, disabled, must_change_password, quota_bytes, created_at, last_login_at
                     FROM users WHERE id = ?1",
                    params![id],
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

    // ===== v7：LLM 用量 =====

    /// 记一次 LLM 调用（`DbSink` 的唯一写入路径）。
    pub fn insert_llm_usage(&self, u: &yekitrim_llm::usage::UsageRecord) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO llm_usage
                   (ts, label, project, model, host, prompt_tokens, completion_tokens,
                    cached_tokens, reasoning_tokens, latency_ms, attempts, ok, status, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    u.ts, u.label, u.project, u.model, u.host,
                    u.prompt_tokens as i64, u.completion_tokens as i64,
                    u.cached_tokens as i64, u.reasoning_tokens as i64,
                    u.latency_ms as i64, u.attempts as i64, u.ok as i64,
                    u.status.map(|s| s as i64), u.error,
                ],
            )?;
            Ok(())
        })
    }

    /// 跑一条分组聚合 SQL（`key` + 计数/求和列），供 `service::usage::summarize` 用。
    pub fn query_llm_usage_groups(&self, sql: &str, since: i64) -> Result<Vec<super::usage::UsageGroup>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map(params![since], llm_usage_group_from_row)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// 同上，但只取一行（总量）。
    pub fn query_llm_usage_row(&self, sql: &str, since: i64) -> Result<Option<super::usage::UsageGroup>> {
        self.with(|conn| Ok(conn.query_row(sql, params![since], llm_usage_group_from_row).optional()?))
    }
}

/// 聚合行的列顺序固定：`key, calls, failed, prompt, completion, cached, reasoning, latency, attempts`。
fn llm_usage_group_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<super::usage::UsageGroup> {
    Ok(super::usage::UsageGroup {
        key: r.get(0)?,
        calls: r.get(1)?,
        failed: r.get(2)?,
        prompt_tokens: r.get(3)?,
        completion_tokens: r.get(4)?,
        cached_tokens: r.get(5)?,
        reasoning_tokens: r.get(6)?,
        latency_ms: r.get(7)?,
        attempts: r.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("yekitrim.db")).unwrap();
        (dir, db)
    }

    /// 指派必须把待分配会话推到 `closing`。
    ///
    /// 回归钉子：入库 worker 的完结判定 `is_finalized()` **只认 `state == "closing"`**，
    /// 而这里以前写成 `'finalized'` —— 于是「指派并入库」只改了归属、永远不进库
    /// （实测：指派后会话停在 `finalized`、`project_id` 一直是空，得再点一次「重试」才行）。
    /// `'finalized'` 是"项目已建好"（[`Db::set_ingest_project`] 写），别混用。
    #[test]
    fn assign_moves_unassigned_session_to_closing() {
        let (_d, db) = tmp_db();
        let alice = db.create_user("alice", "h", "user", false, 0).unwrap();
        db.ingest_session_touch("s1", Some("23058"), None).unwrap();
        db.set_ingest_state("s1", "unassigned", None).unwrap();

        assert!(db.assign_ingest_session("s1", alice, Some("detect")).unwrap());
        let s = db.ingest_session("s1").unwrap().unwrap();
        assert_eq!(s.owner_id, Some(alice));
        assert_eq!(s.state, "closing", "指派完必须能被 worker 入库，否则按钮在骗人");
        assert_eq!(s.auto_pipeline.as_deref(), Some("detect"));

        // 已经建完项目的会话不该被指派动作改回去
        db.set_ingest_state("s1", "finalized", None).unwrap();
        db.assign_ingest_session("s1", alice, None).unwrap();
        assert_eq!(db.ingest_session("s1").unwrap().unwrap().state, "finalized");
    }

    #[test]
    fn migrate_is_idempotent_and_starts_empty() {
        let (_d, db) = tmp_db();
        assert_eq!(db.count_users().unwrap(), 0);
        db.migrate().unwrap(); // 二次迁移不应报错
        assert_eq!(db.count_users().unwrap(), 0);
    }

    /// v7 的 llm_usage：写入 + 汇总（总量 / 按链路 / 按项目 / 按天）。
    #[test]
    fn llm_usage_insert_and_summarize() {
        use yekitrim_llm::usage::{now_secs, UsageRecord};
        let (_d, db) = tmp_db();
        let mk = |label: &str, project: Option<&str>, prompt: u64, completion: u64, cached: u64, ok: bool| UsageRecord {
            ts: now_secs(),
            label: label.into(),
            project: project.map(|p| p.to_string()),
            model: "m1".into(),
            host: "example.com".into(),
            prompt_tokens: prompt,
            completion_tokens: completion,
            cached_tokens: cached,
            reasoning_tokens: 0,
            latency_ms: 1000,
            attempts: 1,
            ok,
            status: None,
            error: None,
        };
        db.insert_llm_usage(&mk("事件检测", Some("F:/p1"), 1000, 100, 768, true)).unwrap();
        db.insert_llm_usage(&mk("事件检测", Some("F:/p1"), 500, 50, 0, false)).unwrap();
        db.insert_llm_usage(&mk("精修段事件", Some("F:/p1/clip-01"), 200, 20, 0, true)).unwrap();

        let s = super::super::usage::summarize(&db, 30).unwrap();
        assert_eq!(s.totals.calls, 3);
        assert_eq!(s.totals.failed, 1);
        assert_eq!(s.totals.prompt_tokens, 1700);
        assert_eq!(s.totals.completion_tokens, 170);
        assert_eq!(s.totals.cached_tokens, 768);

        let d = s.by_label.iter().find(|g| g.key == "事件检测").unwrap();
        assert_eq!((d.calls, d.prompt_tokens), (2, 1500));
        let p = s.by_project.iter().find(|g| g.key == "F:/p1").unwrap();
        assert_eq!(p.calls, 2, "同一项目两条归一组");
        assert!(s.by_model.iter().any(|g| g.key == "m1@example.com"));
        assert_eq!(s.by_day.len(), 1, "今天只有一天");

        // 超出时间窗 → 不计入
        let s0 = super::super::usage::summarize(&db, 1).unwrap();
        assert_eq!(s0.totals.calls, 3);
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

    /// 邀请码：**明文可重复查看**（管理员权限，用户要求）+ 状态（未使用/已用/已撤销/已过期）。
    #[test]
    fn invite_rows_are_readable_and_stateful() {
        let (_d, db) = tmp_db();
        let admin = db.create_user("admin", "h", "admin", false, 0).unwrap();
        let now = now_secs();
        let code = "yekitrim_inv_TESTCODE0000000000000000000000000000";
        let id = db.create_invite(code, Some("给小明"), Some(admin), Some(now + 86400)).unwrap();

        // 明文能原样读回来 —— 这正是"管理员可以重复查看"
        let row = db.find_invite_by_token(code).unwrap().expect("按明文能查到");
        assert_eq!(row.token, code);
        assert_eq!(db.find_invite_by_id(id).unwrap().unwrap().token, code);
        assert!(row.is_open(now), "未使用 = 可用");
        assert!(!row.is_open(now + 86401), "过期后不可用");
        assert_eq!(row.note.as_deref(), Some("给小明"));
        assert_eq!(row.used_at, None, "还没人用");

        // 用掉之后：仍然可查看明文，状态变成"已使用 + 绑定了谁"
        let uid = db.register_with_invite(id, "xiaoming", "scrypt$y").unwrap();
        let used = db.find_invite_by_token(code).unwrap().unwrap();
        assert_eq!(used.token, code, "用过的码也要能查看");
        assert_eq!(used.used_by, Some(uid));
        assert!(used.used_at.is_some());
        assert!(!used.is_open(now_secs()), "已使用即不可再用");

        let id2 = db.create_invite("yekitrim_inv_REVOKE_ME", None, None, None).unwrap();
        assert_eq!(db.revoke_invite(id2).unwrap(), 1);
        assert_eq!(db.revoke_invite(id2).unwrap(), 0, "重复撤销幂等");
        assert!(!db.find_invite_by_id(id2).unwrap().unwrap().is_open(now));
        assert!(db.find_invite_by_token("yekitrim_inv_nope").unwrap().is_none(), "不存在的码查不到");
    }

    /// 用户自助登记直播间：写/覆盖、按人查、按归属删（别人删不动）。
    #[test]
    fn rooms_round_trip_and_scoped_delete() {
        let (_d, db) = tmp_db();
        let alice = db.create_user("alice", "h", "user", false, 0).unwrap();
        let bob = db.create_user("bob", "h", "user", false, 0).unwrap();

        db.room_upsert("12345", alice, "某主播").unwrap();
        db.room_upsert("777", alice, "").unwrap();
        let mine = db.rooms_of_user(alice).unwrap();
        assert_eq!(mine.len(), 2);
        assert!(mine.iter().all(|r| r.source == "user"), "界面加的都标 user");
        assert!(mine.iter().all(|r| r.enabled));
        let t = db.channel_for_room("12345").unwrap().unwrap();
        assert_eq!(t.user_id, alice);
        assert_eq!(t.title, "某主播");
        assert!(db.rooms_of_user(bob).unwrap().is_empty(), "看不到别人的");

        // 同一个房间号再登记（换人 / 换标题）→ 覆盖，不产生第二行
        db.room_upsert("12345", bob, "改名了").unwrap();
        assert_eq!(db.rooms_of_user(alice).unwrap().len(), 1, "被 bob 接管了");
        assert_eq!(db.channel_for_room("12345").unwrap().unwrap().title, "改名了");

        // 删：限本人；admin（None）全权
        assert_eq!(db.room_delete("12345", Some(alice)).unwrap(), 0, "不是他的删不动");
        assert!(db.channel_for_room("12345").unwrap().is_some());
        assert_eq!(db.room_delete("12345", Some(bob)).unwrap(), 1);
        assert!(db.channel_for_room("12345").unwrap().is_none());
        assert_eq!(db.room_delete("777", None).unwrap(), 1, "admin 可删任意");
    }

    /// **`PUT /api/channel-map` 不许抢走别人已登记的房间**（2026-11-14 后端 review 的 P1）。
    ///
    /// 判据四条：
    /// ① 别人的房间 → 返回冲突列表、**整批不生效**（连自己本来合法的规则也不动，避免"半保存"）；
    /// ② 未被任何人登记的房间 → 照旧可以自助登记（先到先得）；
    /// ③ 改**自己**的房间 → 正常覆盖，不误报冲突；
    /// ④ admin（`allow_takeover = true`）→ 可接管，与 `POST /api/rooms` 的 admin 口径一致。
    #[test]
    fn replace_channel_map_never_steals_another_users_room() {
        let (_d, db) = tmp_db();
        let alice = db.create_user("alice", "h", "user", false, 0).unwrap();
        let bob = db.create_user("bob", "h", "user", false, 0).unwrap();

        // alice 先登记 26320007
        assert!(db.replace_channel_map(alice, false, &[("26320007".into(), "detect".into(), true)])
            .unwrap()
            .is_empty());

        // ② bob 登记**未登记**的房间：允许（自助登记照旧）
        let conflicts = db
            .replace_channel_map(bob, false, &[("18001".into(), "cut".into(), true)])
            .unwrap();
        assert!(conflicts.is_empty(), "没被登记过的房间该允许自助登记：{conflicts:?}");
        assert_eq!(db.channel_for_room("18001").unwrap().unwrap().user_id, bob);

        // ① bob 试图接管 alice 的 26320007 —— 而且把自己那条合法规则一起带上（验"整批不生效"）
        let conflicts = db
            .replace_channel_map(
                bob,
                false,
                &[
                    ("26320007".into(), "cut".into(), true),
                    ("18002".into(), "detect".into(), true),
                ],
            )
            .unwrap();
        assert_eq!(conflicts, vec!["26320007".to_string()], "该且只该报这一条冲突：{conflicts:?}");
        let stolen = db.channel_for_room("26320007").unwrap().unwrap();
        assert_eq!(stolen.user_id, alice, "归属绝不能被改到 bob");
        assert!(stolen.enabled, "也不能被 enabled:false 掐掉");
        assert_eq!(stolen.auto_pipeline, "detect", "pipeline 不能被别人改");
        assert!(
            db.channel_for_room("18002").unwrap().is_none(),
            "冲突时整批不生效：同批的 18002 也不该被写进去"
        );
        assert_eq!(
            db.channel_for_room("18001").unwrap().unwrap().user_id,
            bob,
            "冲突的那次替换不该把 bob 原有规则删掉（事务未提交）"
        );

        // ③ 改自己的：正常覆盖
        assert!(db
            .replace_channel_map(bob, false, &[("18001".into(), "none".into(), false)])
            .unwrap()
            .is_empty());
        let own = db.channel_for_room("18001").unwrap().unwrap();
        assert_eq!(own.user_id, bob);
        assert_eq!(own.auto_pipeline, "none");
        assert!(!own.enabled);

        // ④ admin 可以接管（与 /api/rooms 的 admin 口径一致）
        assert!(db
            .replace_channel_map(alice, true, &[("26320007".into(), "cut".into(), true)])
            .unwrap()
            .is_empty());
        assert_eq!(db.channel_for_room("26320007").unwrap().unwrap().user_id, alice);
    }

    /// 注册的两步必须原子：**一个码只能绑一个账号**（第二个拿到的是"已被使用"）。
    #[test]
    fn register_with_invite_consumes_once_and_binds_user() {
        let (_d, db) = tmp_db();
        let code = "yekitrim_inv_ONCE0000000000000000000000000000";
        let inv = db.create_invite(code, None, None, None).unwrap();
        let uid = db.register_with_invite(inv, "newbie", "scrypt$y").unwrap();
        let user = db.find_user("newbie").unwrap().unwrap();
        assert_eq!(user.id, uid);
        assert_eq!(user.role, "user", "自助注册只能是 user");
        assert!(!user.must_change_password, "自己设的密码不算初始密码");
        assert_eq!(db.find_invite_by_token(code).unwrap().unwrap().used_by, Some(uid), "绑定到谁要记下来");

        let again = db.register_with_invite(inv, "another", "scrypt$z");
        assert!(again.is_err(), "同一个码不能绑第二个账号: {again:?}");
        assert!(db.find_user("another").unwrap().is_none(), "失败时不应留下半个账号");

        // 用户名大小写不同也算重名（挡冒充）
        assert!(db.find_user_ci("NEWBIE").unwrap().is_some());
        assert!(db.find_user_ci("nobody").unwrap().is_none());

        // 撤销的 / 过期的码不能注册
        let inv2 = db.create_invite("yekitrim_inv_REVOKED", None, None, None).unwrap();
        db.revoke_invite(inv2).unwrap();
        assert!(db.register_with_invite(inv2, "revoked-try", "scrypt$w").is_err(), "撤销的码不能注册");
        let inv3 = db.create_invite("yekitrim_inv_EXPIRED", None, None, Some(now_secs() - 1)).unwrap();
        assert!(db.register_with_invite(inv3, "expired-try", "scrypt$w").is_err(), "过期的码不能注册");
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

    /// ⚠️ **数据目录环境变量的回归钉子**（同时钉住 `path.rs` / `db.rs` / `server/lib.rs` 三处）。
    ///
    /// 这条测的是"改名时最容易静默坏"的那条链：`--data-dir` 是通过
    /// `set_var(DATA_DIR_ENV)` 传给 service 侧的，而 `db::data_dir()` 是**独立第二实现**。
    /// 只改一端 ⇒ `--data-dir` 静默失效：server 用一个库、CLI 侧命令读另一个库，
    /// **不报错**，而且只有传了 `--data-dir` 才暴露。
    ///
    /// ⚠️ 测试**必须串行**：它改的是进程级环境变量。用一把专用锁把读-改-写包起来，
    /// 并在结束时**无论如何都还原**（`user.rs` 明写"测试不能靠环境变量隔离，会互相踩"）。
    #[test]
    fn data_dir_env_var_is_the_only_override_and_points_the_ledger_at_it() {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let old = std::env::var_os(yekitrim_core::path::DATA_DIR_ENV);
        let tmp = std::env::temp_dir().join(format!("yekitrim-dd-{}", std::process::id()));

        // ① 名字必须是新名（旧名不再被认）
        assert_eq!(yekitrim_core::path::DATA_DIR_ENV, "YEKITRIM_DATA_DIR");

        // ② 设了覆盖 ⇒ 账本落在**覆盖目录本身**（不是它下面的 .yekitrim/cloud）
        std::env::set_var(yekitrim_core::path::DATA_DIR_ENV, &tmp);
        assert_eq!(data_dir(), tmp, "覆盖值就是账本目录");
        assert_eq!(db_path(), tmp.join("yekitrim.db"));

        // ③ 覆盖为空串 ⇒ 等同没设（别把账本写到根目录去）
        std::env::set_var(yekitrim_core::path::DATA_DIR_ENV, "");
        assert_eq!(data_dir(), yekitrim_core::path::yekitrim_home().join("cloud"));

        // ④ 没设 ⇒ 落到 `<数据目录>/.yekitrim/cloud`，且**状态目录名是新名**
        std::env::remove_var(yekitrim_core::path::DATA_DIR_ENV);
        let expected = yekitrim_core::path::yekitrim_home().join("cloud");
        assert_eq!(data_dir(), expected);
        assert!(
            expected.to_string_lossy().contains(".yekitrim"),
            "状态目录必须是新名: {}",
            expected.display()
        );

        match old {
            Some(v) => std::env::set_var(yekitrim_core::path::DATA_DIR_ENV, v),
            None => std::env::remove_var(yekitrim_core::path::DATA_DIR_ENV),
        }
    }
}
