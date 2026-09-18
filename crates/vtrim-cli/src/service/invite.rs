//! `vtrim user invite*`：自助注册用的邀请码（一次性 token）。
//!
//! - `vtrim user invite [--note 给谁] [--days N] [--count N]`：生成邀请码，
//!   **明文落库**（管理员可随时 `vtrim user invites` 再看一次、复制再发），并开启
//!   `[auth] registration_enabled`；一个码只能绑一个账号（用过即失效）；
//! - `vtrim user invite --disable`：关闭注册（已生成的码保留，恢复只需再生成一个）；
//! - `vtrim user invites`：列出邀请码（只见脱敏 hint）与注册开关状态；
//! - `vtrim user invite-revoke <id|selector>`：撤销一个码。
//!
//! 与 `vtrim ingest token` 不同：邀请码要能重复查看再发（用户要求），所以明文直接入库。

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use super::auth;
use super::db::{self, Db, InviteRow};
use super::user::fmt_time;

fn global_config_path() -> PathBuf {
    vtrim_core::path::vtrim_home().join("config.toml")
}

/// 把 `[auth]` 的若干键合并写进全局配置（保留注释与其他 section）。
fn patch_auth(pairs: serde_json::Value) -> Result<()> {
    let path = global_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建配置目录失败: {}", parent.display()))?;
    }
    if !path.exists() {
        let _ = crate::service::ensure_global_config();
    }
    vtrim_core::config::patch_config_file(&path, &serde_json::json!({ "auth": pairs }))
        .with_context(|| format!("写入 {} 失败", path.display()))?;
    Ok(())
}

/// 注册开关当前是否打开（读全局配置；服务端每次请求都会重读，改完立即生效，不必重启）。
pub fn registration_enabled() -> bool {
    vtrim_core::config::load_global_config().auth.registration_enabled
}

pub fn set_registration_enabled(on: bool) -> Result<()> {
    patch_auth(serde_json::json!({ "registration_enabled": on }))
}

/// 一行状态文案（列表与 `invites` 共用）。**未使用 / 已使用** 是第一信息（用户要求），
/// 撤销与过期作为限定。
pub fn status_text(row: &InviteRow, now: i64) -> String {
    if let Some(at) = row.used_at {
        let who = match row.used_by {
            Some(uid) => format!("账号 #{uid}"),
            None => "已注销账号".to_string(),
        };
        let tail = if row.revoked { "，已撤销" } else { "" };
        return format!("已使用 · 绑定 {who} · {}{tail}", fmt_time(at));
    }
    if row.revoked {
        return "未使用 · 已撤销".into();
    }
    if let Some(exp) = row.expires_at {
        if exp <= now {
            return format!("未使用 · 已过期（{}）", fmt_time(exp));
        }
        return format!("未使用 · 有效至 {}", fmt_time(exp));
    }
    "未使用".into()
}

/// 生成 N 个邀请码并落库，返回明文（调用方负责"只显示一次"）。
///
/// 与 CLI/HTTP 解耦，方便直接测「生成的码真的能注册成功」。
pub fn create_invites(db: &Db, note: Option<&str>, days: Option<u32>, count: u32) -> Result<Vec<String>> {
    let days = validate_days(days)?;
    if count == 0 || count > 50 {
        bail!("--count 取值范围 1..=50");
    }
    let expires_at = days.map(|d| db::now_secs() + (d as i64) * 86400);
    let note = note.filter(|s| !s.trim().is_empty());
    let mut codes = Vec::new();
    for _ in 0..count {
        let code = auth::new_invite_code();
        db.create_invite(&code, note, None, expires_at)?;
        codes.push(code);
    }
    Ok(codes)
}

/// `vtrim user invite [--note ...] [--days N] [--count N] [--disable]`
pub fn cmd_invite(note: Option<String>, days: Option<u32>, count: u32, disable: bool) -> Result<()> {
    if disable {
        set_registration_enabled(false)?;
        println!("已关闭注册（已有的邀请码仍可用，重新开启：vtrim user invite）");
        return Ok(());
    }
    let db = Db::open_default()?;
    let codes = create_invites(&db, note.as_deref(), days, count)?;
    set_registration_enabled(true)?;
    db.audit(None, "invite_create", &format!("count={count}"))?;

    println!();
    println!("  注册邀请码（随时可用 `vtrim user invites` 再查看 / 复制）");
    for c in &codes {
        println!("    {c}");
    }
    println!();
    println!("  被邀请人访问 <服务地址>/#/register 填 用户名 / 密码 / 邀请码 即可注册（角色固定 user）。");
    println!("  注册开关已开启（[auth] registration_enabled = true）；关闭：vtrim user invite --disable");
    if let Some(d) = days {
        println!("  有效期：{d} 天");
    }
    if let Some(n) = note {
        println!("  备注：{n}");
    }
    println!("  一个码只能绑一个账号；用过之后列表里标「已使用」并显示绑给了谁。");
    Ok(())
}

/// `vtrim user invites`：列出邀请码（不打印明文）与注册开关状态。
pub fn cmd_invites() -> Result<()> {
    let db = Db::open_default()?;
    let rows = db.list_invites()?;
    let now = db::now_secs();
    println!("注册开关: {}", if registration_enabled() { "已开启" } else { "已关闭" });
    println!("账号账本: {}", vtrim_core::path::display(&db::data_dir()));
    if rows.is_empty() {
        println!();
        println!("暂无邀请码。生成：vtrim user invite [--note 给谁] [--days 7]");
        return Ok(());
    }
    println!();
    for r in &rows {
        println!("  #{:<3} {}", r.id, r.token);
        println!(
            "       {} · 备注：{} · 创建 {}",
            status_text(r, now),
            r.note.clone().unwrap_or_default(),
            fmt_time(r.created_at)
        );
    }
    println!();
    println!("撤销：vtrim user invite-revoke <id 或完整邀请码>");
    Ok(())
}

/// `vtrim user invite-revoke <id|selector|hint>`
pub fn cmd_invite_revoke(target: &str) -> Result<()> {
    let db = Db::open_default()?;
    let t = target.trim();
    // id 直接查一行；否则按完整邀请码查
    let row = if let Ok(id) = t.parse::<i64>() {
        db.find_invite_by_id(id)?
    } else {
        db.find_invite_by_token(t)?
    };
    let Some(row) = row else {
        bail!("找不到邀请码: {t}（用 vtrim user invites 看 id / 完整码）");
    };
    if row.revoked {
        println!("#{} 已经是撤销状态", row.id);
        return Ok(());
    }
    db.revoke_invite(row.id)?;
    db.audit(None, "invite_revoke", &format!("id={}", row.id))?;
    println!("已撤销 #{} {}", row.id, row.token);
    if row.used_at.is_some() {
        println!("提示：这个码已经被使用过，撤销不影响已注册的账号（要停用账号用 vtrim user disable）。");
    } else if !db.list_invites()?.iter().any(|r| r.is_open(db::now_secs())) {
        println!("提示：已没有可用的邀请码；注册开关仍开着，需要时用 vtrim user invite --disable 关闭。");
    }
    Ok(())
}

/// `--days` 参数校验：0 天没有意义（生成即过期），直接挡住。
pub fn validate_days(days: Option<u32>) -> Result<Option<u32>> {
    match days {
        Some(0) => bail!("--days 至少为 1"),
        Some(d) if d > 3650 => bail!("--days 最多 3650"),
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成的码必须真的能用、能重复查看、且只能绑一个账号
    /// （CLI 与 HTTP 两条路最终都走 `create_invites`，所以这条覆盖它们共用的那一段）。
    #[test]
    fn created_codes_are_readable_and_single_use() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("vtrim.db")).unwrap();
        let codes = create_invites(&db, Some("给小明"), Some(7), 2).unwrap();
        assert_eq!(codes.len(), 2);
        assert_ne!(codes[0], codes[1]);

        for code in &codes {
            let row = db.find_invite_by_token(code).unwrap().expect("明文能按码查回");
            assert_eq!(row.token, *code, "明文可重复查看");
            assert_eq!(row.note.as_deref(), Some("给小明"));
            assert!(row.expires_at.unwrap() > db::now_secs(), "7 天有效");
            assert!(row.is_open(db::now_secs()), "未使用");
        }

        // 每个码只能绑一个账号
        let inv = db.find_invite_by_token(&codes[0]).unwrap().unwrap();
        let uid = db.register_with_invite(inv.id, "friend", "scrypt$h").unwrap();
        assert_eq!(uid, db.find_user("friend").unwrap().unwrap().id);
        let after = db.find_invite_by_token(&codes[0]).unwrap().unwrap();
        assert_eq!(after.used_by, Some(uid), "用过的码仍可查看，并显示绑给了谁");
        assert!(!after.is_open(db::now_secs()));
        let again = db.register_with_invite(after.id, "friend2", "scrypt$h");
        assert!(again.is_err(), "同一个码第二次注册必须失败");

        // 参数边界
        assert!(create_invites(&db, None, Some(0), 1).is_err(), "0 天");
        assert!(create_invites(&db, None, None, 0).is_err(), "0 个");
        assert!(create_invites(&db, None, None, 51).is_err(), "超过上限");
    }

    #[test]
    fn status_text_puts_used_or_unused_first() {
        let now = 1_700_000_000;
        let base = InviteRow {
            id: 1,
            token: "vtrim_inv_abcd1234".into(),
            note: None,
            created_by: None,
            created_at: now,
            expires_at: None,
            used_by: None,
            used_at: None,
            revoked: false,
        };
        assert_eq!(status_text(&base, now), "未使用");
        assert!(status_text(&InviteRow { expires_at: Some(now + 3600), ..base.clone() }, now).starts_with("未使用 · 有效至"));
        assert!(status_text(&InviteRow { expires_at: Some(now - 1), ..base.clone() }, now).starts_with("未使用 · 已过期"));
        assert_eq!(status_text(&InviteRow { revoked: true, ..base.clone() }, now), "未使用 · 已撤销");
        let used = status_text(&InviteRow { used_at: Some(now), used_by: Some(7), ..base }, now);
        assert!(used.starts_with("已使用 · 绑定 账号 #7"), "{used}");
    }

    #[test]
    fn days_bounds() {
        assert!(validate_days(Some(0)).is_err());
        assert!(validate_days(Some(4000)).is_err());
        assert_eq!(validate_days(Some(7)).unwrap(), Some(7));
        assert_eq!(validate_days(None).unwrap(), None);
    }
}
