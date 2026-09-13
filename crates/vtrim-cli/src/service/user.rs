//! `vtrim user`：账号账本管理（多用户服务 P1 的 CLI 侧）。
//!
//! 与 HTTP 侧共用 `service::db` / `service::auth`；本文件不引入任何 HTTP 依赖。
//! 口令来源三选一：`--stdin`（管道）、`--password-file <f>`（推荐，不进进程列表）、
//! 都不给则**生成随机密码并打印一次**（与首次部署体验一致）。

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::auth;
use super::db::{self, Db};

/// 口令来源。
#[derive(Debug, Clone)]
pub enum PasswordSource {
    /// 未指定 → 生成随机密码（打印一次）
    Generate,
    /// `--stdin`：从标准输入读一行
    Stdin,
    /// `--password-file <路径>`：读文件（去掉行尾换行）
    File(PathBuf),
}

pub fn source_from(stdin: bool, password_file: Option<PathBuf>) -> PasswordSource {
    match (stdin, password_file) {
        (true, _) => PasswordSource::Stdin,
        (false, Some(p)) => PasswordSource::File(p),
        (false, None) => PasswordSource::Generate,
    }
}

/// 解析口令；返回 (口令, 是否为本次生成的随机口令)。
fn resolve_password(source: &PasswordSource) -> Result<(String, bool)> {
    match source {
        PasswordSource::Generate => Ok((auth::generate_initial_password(), true)),
        PasswordSource::Stdin => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("读取标准输入失败")?;
            let pw = buf.trim_end_matches(['\r', '\n']).to_string();
            if pw.is_empty() {
                bail!("标准输入为空");
            }
            auth::validate_password(&pw)?;
            Ok((pw, false))
        }
        PasswordSource::File(path) => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("读取密码文件失败: {}", path.display()))?;
            let pw = raw.trim_end_matches(['\r', '\n']).to_string();
            if pw.is_empty() {
                bail!("密码文件为空: {}", path.display());
            }
            auth::validate_password(&pw)?;
            Ok((pw, false))
        }
    }
}

/// 初始密码文件位置（首次部署打印此路径；登录成功后由 HTTP 侧删除）。
pub fn initial_password_path() -> PathBuf {
    db::data_dir().join("initial-admin-password.txt")
}

fn require_role(role: &str) -> Result<()> {
    match role {
        "admin" | "user" => Ok(()),
        other => bail!("角色只能是 admin 或 user（收到 {other}）"),
    }
}

pub fn cmd_list() -> Result<()> {
    let db = Db::open_default()?;
    let users = db.list_users()?;
    if users.is_empty() {
        println!("暂无账号。首次部署请运行: vtrim user init-admin");
        return Ok(());
    }
    println!("{:<16} {:<6} {:<8} {:<10} {:<18} {}", "用户名", "角色", "状态", "配额", "创建时间", "最后登录");
    for u in users {
        let state = if u.disabled { "已禁用" } else { "启用" };
        let flag = if u.must_change_password { " (初始密码)" } else { "" };
        println!(
            "{:<16} {:<6} {:<8} {:<10} {:<18} {}{}",
            u.username,
            u.role,
            state,
            if u.quota_bytes == 0 { "不限".to_string() } else { u.quota_bytes.to_string() },
            fmt_time(u.created_at),
            u.last_login_at.map(fmt_time).unwrap_or_else(|| "—".to_string()),
            flag
        );
    }
    Ok(())
}

fn fmt_time(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| secs.to_string())
}

pub fn cmd_add(username: &str, role: &str, source: PasswordSource) -> Result<()> {
    require_role(role)?;
    let db = Db::open_default()?;
    if db.find_user(username)?.is_some() {
        bail!("账号已存在: {username}");
    }
    let (password, generated) = resolve_password(&source)?;
    let hash = auth::hash_password(&password)?;
    let id = db.create_user(username, &hash, role, generated, 0)?;
    db.audit(Some(id), "user_add", &format!("role={role}"))?;
    println!("已创建账号 {username}（角色 {role}）");
    if generated {
        print_once(&password, &format!("账号 {username} 的初始密码"));
    }
    Ok(())
}

pub fn cmd_passwd(username: &str, source: PasswordSource) -> Result<()> {
    let db = Db::open_default()?;
    let user = db
        .find_user(username)?
        .with_context(|| format!("账号不存在: {username}"))?;
    let (password, generated) = resolve_password(&source)?;
    let hash = auth::hash_password(&password)?;
    // 生成的随机密码视为"初始密码"（提示改密）；用户自设则直接生效
    db.set_password(username, &hash, generated)?;
    db.audit(Some(user.id), "user_passwd", "")?;
    println!("已重置 {username} 的密码");
    if generated {
        print_once(&password, &format!("账号 {username} 的新密码"));
    }
    Ok(())
}

pub fn cmd_disable(username: &str, enable: bool) -> Result<()> {
    let db = Db::open_default()?;
    let user = db
        .find_user(username)?
        .with_context(|| format!("账号不存在: {username}"))?;
    db.set_disabled(username, !enable)?;
    db.audit(Some(user.id), if enable { "user_enable" } else { "user_disable" }, "")?;
    println!("已{}账号 {username}", if enable { "启用" } else { "禁用" });
    Ok(())
}

/// 校验口令是否正确（诊断用；也是 `verify_password` 的第一个 CLI 消费点）。
pub fn cmd_check(username: &str, source: PasswordSource) -> Result<()> {
    let db = Db::open_default()?;
    let Some(hash) = db.user_password_hash(username)? else {
        bail!("账号不存在: {username}");
    };
    let (password, _) = resolve_password(&source)?;
    if auth::verify_password(&hash, &password)? {
        println!("✅ 口令正确: {username}");
        Ok(())
    } else {
        println!("❌ 口令不匹配: {username}");
        std::process::exit(1);
    }
}

/// 首次部署初始化管理员：**幂等**——已有账号则原样返回，绝不重置密码。
pub fn cmd_init_admin(source: PasswordSource) -> Result<()> {
    let db = Db::open_default()?;
    if db.count_users()? > 0 {
        println!("账号账本已存在，未做任何改动（如需重置管理员密码: vtrim user reset-admin-password）");
        return Ok(());
    }
    let (password, generated) = match source {
        PasswordSource::Generate => (auth::generate_initial_password(), true),
        other => {
            let (pw, _) = resolve_password(&other)?;
            (pw, false)
        }
    };
    let hash = auth::hash_password(&password)?;
    let id = db.create_user("admin", &hash, "admin", generated, 0)?;
    db.audit(Some(id), "admin_bootstrap", if generated { "generated" } else { "explicit" })?;
    if generated {
        let path = initial_password_path();
        std::fs::write(&path, format!("{password}\n"))
            .with_context(|| format!("写入初始密码文件失败: {}", path.display()))?;
        println!("已创建管理员账号: admin");
        print_once(&password, "管理员初始密码");
        println!("初始密码同时写入: {}（首次登录成功后自动删除）", path.display());
    } else {
        println!("已创建管理员账号: admin（使用你提供的密码）");
    }
    Ok(())
}

/// 救援入口：重置管理员密码（忘记密码时在本机执行；能执行 CLI 即等价于拥有主机权限）。
pub fn cmd_reset_admin_password(source: PasswordSource) -> Result<()> {
    let db = Db::open_default()?;
    let user = db
        .find_user("admin")?
        .context("账号账本里没有 admin（先运行 vtrim user init-admin）")?;
    if user.role != "admin" {
        bail!("账号 admin 不是管理员角色");
    }
    let (password, generated) = resolve_password(&source)?;
    let hash = auth::hash_password(&password)?;
    db.set_password("admin", &hash, generated)?;
    db.audit(Some(user.id), "admin_reset_password", "")?;
    println!("已重置管理员密码");
    if generated {
        let path = initial_password_path();
        let _ = std::fs::write(&path, format!("{password}\n"));
        print_once(&password, "新的管理员密码");
        println!("已同步写入: {}", path.display());
    }
    Ok(())
}

fn print_once(password: &str, what: &str) {
    println!();
    println!("  ┌──────────────────────────────────────────────┐");
    println!("  │ {what}（只显示这一次，请立即记录）");
    println!("  │   {password}");
    println!("  └──────────────────────────────────────────────┘");
    println!();
}

/// 显示账本位置（诊断用）。
pub fn cmd_info() -> Result<()> {
    let dir = db::data_dir();
    println!("账号账本目录: {}", dir.display());
    println!("账本文件:     {}", db::db_path().display());
    println!("初始密码文件: {}", initial_password_path().display());
    if !Path::new(&dir).exists() {
        println!("（尚未创建；运行 `vtrim user init-admin` 或启动 `vtrim server` 时生成）");
    }
    Ok(())
}
