//! `yekitrim user`：账号账本管理（多用户服务 P1 的 CLI 侧）。
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
    db::ledger_dir().join("initial-admin-password.txt")
}

/// **密码一经变更，把初始密码文件处置掉**（2026-10-02）。
///
/// ⚠️ 为什么要单独有这个函数、而不是各处手写 remove_initial_password_file()：
/// 这个文件的**唯一语义**是「这里躺着的就是当前有效密码」。一旦密码变了它就必须被处置，
/// 否则它**会撒谎** —— 用户照着 initial-admin-password.txt 里的串输入却登不进去。
/// 实测踩到：target/debug/.yekitrim/cloud/ 里那份是早已失效的旧密码，
/// yekitrim user check 报「❌ 口令不匹配」，而 CLI 当初的提示还写着「已同步写入 …」。
///
/// 处置方式两种，取决于新密码**是怎么来的**：
/// - 新密码是**这次现场生成**的（generated）⇒ **覆写**成它：文件继续有效，
///   且它是用户的第二条退路（终端滚走了 / 无控制台部署时唯一的那条）。
/// - 新密码是**用户自己提供**的（--stdin / --password-file）⇒ **删除**：
///   它本来就不在文件里，留着只会留一个**过期的**密码在那里骗人。
///
/// ⚠️ 所有改密码的入口**都必须走这里**（cmd_passwd / cmd_reset_admin_password /
/// HTTP 改密 / 登录成功后的清理）。漏掉任何一条 = 那个入口之后文件就开始撒谎。
pub fn retire_initial_password_file(generated: Option<&str>) {
    retire_initial_password_file_in(&db::ledger_dir(), generated);
}

/// [`retire_initial_password_file`] 的可注入版本（单测用，不碰真实数据目录）。
///
/// ⚠️ 测试**不能**靠 `YEKITRIM_DATA_DIR` 做隔离 —— 测试是并行跑的，改全局环境变量会互相踩
/// （同 `yekitrim-server` 的 `test_support` 那条注释）。
pub fn retire_initial_password_file_in(dir: &Path, generated: Option<&str>) {
    match generated {
        Some(pw) => {
            let path = dir.join("initial-admin-password.txt");
            // ⚠️ 写失败不能「看起来成功」：这是**刚生成**的新密码，文件是唯一的第二条退路。
            match std::fs::write(&path, format!("{pw}\n")) {
                Ok(()) => tracing::info!("初始密码文件已更新: {}", path.display()),
                Err(e) => tracing::warn!("写初始密码文件失败（密码本身已生效）: {} - {e}", path.display()),
            }
        }
        // 用户自设的密码：文件里那份已经作废，删掉（不是覆写 —— 我们没有明文可写）
        None => {
            let path = dir.join("initial-admin-password.txt");
            // 不存在就是常态（本来没生成过，或早被登录清掉了），不必报错
            if !path.exists() {
                return;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => tracing::info!("已删除初始密码文件: {}", path.display()),
                Err(e) => tracing::warn!("删除初始密码文件失败（不影响使用）: {} - {e}", path.display()),
            }
        }
    }
}

/// 删除初始密码文件（首次成功登录 / 改密后调用）。删不掉只记 warning，不阻塞（§3.5.2）。
pub fn remove_initial_password_file() {
    let path = initial_password_path();
    if !path.exists() {
        return;
    }
    match std::fs::remove_file(&path) {
        Ok(()) => tracing::info!("已删除初始密码文件: {}", path.display()),
        Err(e) => tracing::warn!("删除初始密码文件失败（不影响使用）: {} - {e}", path.display()),
    }
}

/// `yekitrim server` 启动时的管理员引导结果。
#[derive(Debug, Clone)]
pub struct Bootstrap {
    /// 本次启动是否新建了 admin（重启不会重建，见 §3.5.2）
    pub created: bool,
    /// 账号数
    pub accounts: i64,
    /// 是否仍在使用初始/重置密码（对外监听的门槛，§3.5.4）
    pub must_change_password: bool,
    /// 本次新建时的初始密码（明文，**只在这里出现一次**）
    pub initial_password: Option<String>,
    /// 初始密码文件路径（只有"生成随机密码"时才写）
    pub password_file: Option<PathBuf>,
}

/// 服务启动引导：无账号时创建 `admin`（幂等——已有账号绝不重置，§3.5.2）。
///
/// `explicit` = `--admin-password-file` / `YEKITRIM_ADMIN_PASSWORD` 提供的密码：
/// 显式指定时**不生成随机密码、不写初始密码文件**，且视为"已改密"（可直接对外监听）。
pub fn ensure_admin_bootstrap(explicit: Option<String>) -> Result<Bootstrap> {
    let db = Db::open_default()?;
    let accounts = db.count_users()?;

    if accounts == 0 {
        let (password, generated) = match explicit {
            Some(pw) => {
                auth::validate_password(&pw)
                    .context("--admin-password-file / YEKITRIM_ADMIN_PASSWORD 提供的密码不合法")?;
                (pw, false)
            }
            None => (auth::generate_initial_password(), true),
        };
        let hash = auth::hash_password(&password)?;
        let id = db.create_user("admin", &hash, "admin", generated, 0)?;
        db.audit(Some(id), "admin_bootstrap", if generated { "generated" } else { "explicit" })?;

        let password_file = if generated {
            let path = initial_password_path();
            std::fs::write(&path, format!("{password}\n"))
                .with_context(|| format!("写入初始密码文件失败: {}", path.display()))?;
            Some(path)
        } else {
            None
        };
        return Ok(Bootstrap {
            created: true,
            accounts: 1,
            must_change_password: generated,
            initial_password: Some(password),
            password_file,
        });
    }

    // 已有账号：不重建、不重置；只报告"是否仍在用初始密码"
    let must_change = match db.find_user("admin")? {
        Some(u) => u.must_change_password,
        None => {
            tracing::warn!("账号账本里没有 admin（可能只建了普通账号）：请用 `yekitrim user init-admin` 或管理页确认");
            false
        }
    };
    Ok(Bootstrap {
        created: false,
        accounts,
        must_change_password: must_change,
        initial_password: None,
        password_file: None,
    })
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
        println!("暂无账号。首次部署请运行: yekitrim user init-admin");
        return Ok(());
    }
    println!("{:<16} {:<6} {:<8} {:<10} {:<18} 最后登录", "用户名", "角色", "状态", "配额", "创建时间");
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

pub(crate) fn fmt_time(secs: i64) -> String {
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
    // ⚠️ 改密后必须处置初始密码文件（2026-10-02）：以前这里**什么都没做**，
    // 于是文件里留着**上一个**密码 —— 而那个文件正是用户"忘了密码"时的第二条退路，
    // 照着它输却登不进去（实测踩到过）。生成的新密码写文件、用户自设的则删除。
    retire_initial_password_file(generated.then_some(password.as_str()));
    println!("已重置 {username} 的密码");
    if generated {
        print_once(&password, &format!("账号 {username} 的新密码"));
        println!("已同步写入: {}", initial_password_path().display());
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
        println!("账号账本已存在，未做任何改动（如需重置管理员密码: yekitrim user reset-admin-password）");
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
        .context("账号账本里没有 admin（先运行 yekitrim user init-admin）")?;
    if user.role != "admin" {
        bail!("账号 admin 不是管理员角色");
    }
    let (password, generated) = resolve_password(&source)?;
    let hash = auth::hash_password(&password)?;
    db.set_password("admin", &hash, generated)?;
    db.audit(Some(user.id), "admin_reset_password", "")?;
    println!("已重置管理员密码");
    // ⚠️ 处置初始密码文件必须**两条分支都走**（2026-10-02）：
    // 以前只有 `generated` 那条覆写文件，`--stdin` / `--password-file` 时**旧文件原样留着**
    // —— 那里面是**重置前**的密码，用户照着它输必然失败（正是本次报障的形态）。
    // 生成的新密码要写盘（它是用户唯一的第二条退路），
    // 但**写失败不能"看起来成功"**：以前是 `let _ =`，写失败等于把新密码悄悄丢掉。
    if generated {
        let path = initial_password_path();
        std::fs::write(&path, format!("{password}\n"))?;
    }
    retire_initial_password_file(generated.then_some(password.as_str()));
    if generated {
        print_once(&password, "新的管理员密码");
        println!("已同步写入: {}", initial_password_path().display());
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
    let dir = db::ledger_dir();
    println!("账号账本目录: {}", dir.display());
    println!("账本文件:     {}", db::db_path().display());
    println!("初始密码文件: {}", initial_password_path().display());
    if !Path::new(&dir).exists() {
        println!("（尚未创建；运行 `yekitrim user init-admin` 或启动 `yekitrim server` 时生成）");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **密码一经变更，初始密码文件必须被处置**（2026-10-02 的钉子）。
    ///
    /// 这个文件唯一能骗人的方式：密码变了、它还留着旧的。照着它输 ⇒ 「口令不匹配」，
    /// 而它是文档与 CLI 都指过的「第二条退路」。这就是本次报障的形态。
    #[test]
    fn password_change_retires_the_initial_password_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("initial-admin-password.txt");

        // ① 现场生成的新密码 ⇒ **覆写**（文件继续有效，是用户的第二条退路）
        std::fs::write(&path, "OLD-PASS-OLD1-OLD2\n").unwrap();
        retire_initial_password_file_in(dir.path(), Some("NEW-PASS-NEW1-NEW2"));
        let got = std::fs::read_to_string(&path).unwrap();
        assert_eq!(got.trim(), "NEW-PASS-NEW1-NEW2", "生成的新密码必须写进文件");

        // ② 用户自设的密码 ⇒ **删除**（我们没有明文可写，留着旧的就是撒谎）
        retire_initial_password_file_in(dir.path(), None);
        assert!(!path.exists(), "用户自设密码后，过期的初始密码文件必须消失");

        // ③ 幂等：本来就不存在时不能报错（这是常态）
        retire_initial_password_file_in(dir.path(), None);
    }

    /// 文件内容必须是**当前**密码、且只有一行（别把历史密码堆进去）。
    #[test]
    fn initial_password_file_holds_exactly_the_current_one() {
        let dir = tempfile::tempdir().unwrap();
        retire_initial_password_file_in(dir.path(), Some("AAAA-BBBB-CCCC-DDDD"));
        let raw = std::fs::read_to_string(dir.path().join("initial-admin-password.txt")).unwrap();
        assert_eq!(raw, "AAAA-BBBB-CCCC-DDDD\n", "应当是新密码 + 单个换行，不多不少");
    }

    /// ⚠️ **反例**：所有改密码的入口都要走 `retire_*`。
    ///
    /// 判据是**源码级**的，但只看**真正的代码行** —— 剥掉注释与测试模块，
    /// 再要求「拼这个文件名的行为」只在 `retire_initial_password_file_in` 里发生一次。
    /// 直接数字面量不行：注释里也会提到它，那样这条测试会因为改文档而红（第一版就是这么写错的）。
    #[test]
    fn only_one_place_writes_or_removes_the_initial_password_file() {
        let mut src = include_str!("user.rs").to_string();
        // ① 砍掉测试模块自己（它就在文件尾部，且里面有断言用的字面量）
        if let Some(i) = src.find("#[cfg(test)]") {
            src.truncate(i);
        }
        // ② 剥掉行注释（本文件的注释大量引用这个名字，它们是文档不是行为）
        let code = src
            .lines()
            .map(|l| l.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        // ③ 现在数「真的在拼路径」的地方 —— 只该有 _in 那一处
        let hits = code.matches("initial-admin-password.txt").count();
        // 允许的 3 处：① initial_password_path()（公开的唯一路径定义）
        //             ②③ retire_initial_password_file_in 的两条分支（覆写 / 删除）
        // 新增改密入口**不该**再增加这个数字 —— 一律走 retire_*。
        assert_eq!(
            hits, 3,
            "拼 initial-admin-password.txt 只允许出现在 initial_password_path 与 retire_initial_password_file_in 里，实际 {hits} 处 —— 新增改密入口请走 retire_*，别自己拼这个路径"
        );
    }
}
