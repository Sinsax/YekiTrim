//! 口令 KDF 与初始密码生成。
//!
//! **与设计稿的偏差（必读）**：设计稿 §3 原定 argon2id。本次实现时 crates.io 不可达、
//! 且本地 cargo 缓存里没有 `argon2`（`rusqlite`/`scrypt` 有），因此 P1 采用 **scrypt**
//! （内存硬，OWASP 认可的密码存储 KDF）+ **自描述哈希串**
//! `scrypt$log_n$r$p$<salt_b64>$<hash_b64>`；将来拿到 argon2 依赖后，只需在登录成功时
//! 检测哈希前缀并"重哈希升级"为 argon2id，**不需要迁移数据库**。

use anyhow::{anyhow, bail, Result};
use base64::Engine as _;
use rand::RngCore;
use scrypt::{scrypt, Params};
use subtle::ConstantTimeEq;

/// N = 2^15（约 32 MiB 内存/次），r=8，p=1：登录频率极低，这个强度对 ≤10 人自托管足够。
const LOG_N: u8 = 15;
const R: u32 = 8;
const P: u32 = 1;
const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32;
/// 用户自设密码的最小长度（生成的初始密码为 16 字符，远超此值）。
pub const MIN_PASSWORD_LEN: usize = 8;

/// 会话有效期：7 天，**每次请求滑动续期**（§3.5.6）。
pub const SESSION_TTL_SECS: i64 = 7 * 24 * 60 * 60;

/// 距离过期不足这个时长时才写库续期，避免每个 GET 都做一次 UPDATE。
pub const SESSION_RENEW_AFTER_SECS: i64 = 60 * 60;

pub const COOKIE_SID: &str = "yekitrim_sid";
pub const COOKIE_CSRF: &str = "yekitrim_csrf";
/// CSRF 令牌自定义头（写请求必带）。
pub const CSRF_HEADER: &str = "x-csrf-token";
/// 设置页/文档里显示的静态资源 cookie 路径。
pub const COOKIE_PATH: &str = "/";

/// CSPRNG 令牌（会话 id / CSRF），256 bit，URL 安全（可安全放进 cookie 与头）。
pub fn new_token() -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let mut raw = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    URL_SAFE_NO_PAD.encode(raw)
}

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// 邀请码前缀。码是**一次性 token**：明文落库（管理员要能重复查看、复制再发 —— 用户要求），
/// 一次性由 `used_at/used_by` 保证：一个码只能绑一个账号。
pub const INVITE_PREFIX: &str = "yekitrim_inv_";

/// 生成邀请码：`yekitrim_inv_<43 字符 base64url>`（32 字节 CSPRNG）。
pub fn new_invite_code() -> String {
    format!("{INVITE_PREFIX}{}", new_token())
}

pub fn hash_password(password: &str) -> Result<String> {
    let mut salt = [0u8; SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let mut out = [0u8; KEY_LEN];
    let params = Params::new(LOG_N, R, P, KEY_LEN).map_err(|e| anyhow!("scrypt 参数无效: {e}"))?;
    scrypt(password.as_bytes(), &salt, &params, &mut out).map_err(|e| anyhow!("scrypt 失败: {e}"))?;
    Ok(format!(
        "scrypt${LOG_N}${R}${P}${}${}",
        b64().encode(salt),
        b64().encode(out)
    ))
}

/// 校验口令。未知前缀一律视为不匹配（绝不接受明文/空哈希）。
pub fn verify_password(stored: &str, password: &str) -> Result<bool> {
    let parts: Vec<&str> = stored.split('$').collect();
    if parts.len() != 6 || parts[0] != "scrypt" {
        bail!("不支持的密码哈希格式");
    }
    let log_n: u8 = parts[1].parse().map_err(|_| anyhow!("哈希 log_n 无效"))?;
    let r: u32 = parts[2].parse().map_err(|_| anyhow!("哈希 r 无效"))?;
    let p: u32 = parts[3].parse().map_err(|_| anyhow!("哈希 p 无效"))?;
    let salt = b64().decode(parts[4]).map_err(|_| anyhow!("哈希 salt 无效"))?;
    let expected = b64().decode(parts[5]).map_err(|_| anyhow!("哈希值无效"))?;
    let params = Params::new(log_n, r, p, expected.len())
        .map_err(|e| anyhow!("scrypt 参数无效: {e}"))?;
    let mut out = vec![0u8; expected.len()];
    scrypt(password.as_bytes(), &salt, &params, &mut out).map_err(|e| anyhow!("scrypt 失败: {e}"))?;
    Ok(out.ct_eq(&expected).into())
}

pub fn validate_password(password: &str) -> Result<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        bail!("密码至少 {MIN_PASSWORD_LEN} 个字符");
    }
    Ok(())
}

/// 生成初始/重置密码：CSPRNG、16 字符、排除易混字符 `0O1lI`、分组 `XXXX-XXXX-XXXX-XXXX` 便于手输。
pub fn generate_initial_password() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let mut raw = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let chars: Vec<char> = raw
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_verify_round_trip() {
        let h = hash_password("correct horse battery").unwrap();
        assert!(h.starts_with("scrypt$15$8$1$"), "自描述前缀: {h}");
        assert!(verify_password(&h, "correct horse battery").unwrap());
        assert!(!verify_password(&h, "wrong").unwrap());
    }

    #[test]
    fn verify_rejects_garbage_and_plaintext() {
        assert!(verify_password("", "x").is_err());
        assert!(verify_password("plaintext", "plaintext").is_err());
        assert!(verify_password("md5$deadbeef", "x").is_err());
    }

    #[test]
    fn generated_password_shape_and_uniqueness() {
        let a = generate_initial_password();
        let b = generate_initial_password();
        assert_eq!(a.len(), 19, "16 位 + 3 个分隔符: {a}");
        assert_eq!(a.matches('-').count(), 3);
        assert!(a.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-'));
        for bad in ['0', 'O', '1', 'l', 'I'] {
            assert!(!a.contains(bad), "不应包含易混字符 {bad}: {a}");
        }
        assert_ne!(a, b);
    }

    #[test]
    fn password_policy() {
        assert!(validate_password("short").is_err());
        assert!(validate_password("longenough").is_ok());
    }

    #[test]
    fn session_tokens_are_url_safe_and_unique() {
        let a = new_token();
        let b = new_token();
        assert_ne!(a, b);
        // 32 字节 base64url 无填充 = 43 字符，且不含 cookie/头里需要转义的字符
        assert_eq!(a.len(), 43, "{a}");
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn invite_code_shape_and_uniqueness() {
        let code = new_invite_code();
        assert!(code.starts_with("yekitrim_inv_"), "{code}");
        assert_eq!(code.len(), "yekitrim_inv_".len() + 43, "{code}");
        assert!(code.is_ascii(), "可安全放进 URL/表单: {code}");
        assert_ne!(code, new_invite_code());
    }
}
