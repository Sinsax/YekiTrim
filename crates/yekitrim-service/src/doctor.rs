//! `yekitrim doctor`：安全与运行状况摘要（设计稿 §3.5.9）。
//!
//! 只读诊断：**绝不创建**账号库、不改任何配置；库里没有账号就如实说"尚未初始化"。
//! 内容：数据目录可写性与剩余磁盘、账号与初始密码状态、会话/审计计数、单实例锁持有者、
//! 以及对本机服务的可达性探测（`/api/health` 的 `listening` / `tls_terminated`）。

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use anyhow::Result;

use super::db::{self, Db};
use super::disk;
use super::instancelock::{self, LockInfo};
use super::user;

/// `yekitrim server` 默认监听端口（唯一一份，其余位置一律引用它）。
///
/// 挑 26320 是为了躲开 Windows 的动态端口段与保留段：本机 `netsh int ipv4 show dynamicport tcp`
/// 从 1024 起（13977 个），保留段实测有 1029–1937 / 5357 / 6633–6862 / 7635–7834 /
/// 50000–50059 / 59100，而动态段与保留段都会**动态重排**。26320 落在跨平台安全带
/// 15001–32767 内、不在 Linux 动态段（32768–60999）里，实测可绑。
///
/// ⚠️ 老默认是 7788，正好掉在 Windows 保留段 7635–7834 里 —— 那台机器上直接
/// `10013 WSAEACCES`，而 `netstat` 里**一个监听者都没有**（因为根本不是"被占用"）。
pub const DEFAULT_PORT: u16 = 26320;

/// 默认探测地址（与 `yekitrim server` 默认监听一致）。
pub const DEFAULT_URL: &str = "http://127.0.0.1:26320";

fn line(title: &str) {
    println!();
    println!("{title}");
}

/// `yekitrim doctor [--url ...]`
pub fn cmd_doctor(url: &str) -> Result<()> {
    let data_dir = db::ledger_dir();
    let db_path = db::db_path();
    let pw_file = user::initial_password_path();
    // 供结尾的"建议"使用（只有真正适用的才打印）
    let mut admin_must_change: Option<bool> = None;
    let mut remote_without_tls = false;

    println!("YekiTrim 安全与运行状况摘要（版本 {}）", env!("CARGO_PKG_VERSION"));

    // ---- 两处进程级全局（整改 T22）----
    // 失败模式都是"静默降级"：漏装 sink = token 全不记账；漏设 uid = 所有人落到 users/1/。
    // doctor 是只读诊断、自己不跑 LLM，所以这里的"本进程未装"是**正常**，
    // 要看服务端启动日志里那句 usage sink: installed。
    println!(
        "  数据归谁: local admin uid = {}（Owner::Local 的落点；漏设会让所有人落到 users/1/）",
        yekitrim_core::owner::local_admin_uid()
    );
    println!(
        "  LLM 用量记账: {}（doctor 进程不跑 LLM；服务端启动会打一行 usage sink: installed）",
        if yekitrim_llm::usage::sink_installed() { "已装" } else { "本进程未装" }
    );

    // ---- 账号数据 ----
    line("账号数据");
    // 只读诊断：数据目录还不存在时不去创建它，改测父目录的可写性
    let dir_exists = data_dir.exists();
    let probe_dir = if dir_exists {
        data_dir.clone()
    } else {
        data_dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| Path::new(".").to_path_buf())
    };
    let writable = disk::writable(&probe_dir);
    let space = disk::space_bytes(&probe_dir);
    let space_text = match space {
        Some((avail, total)) => format!(
            "剩余 {} / 共 {}",
            disk::human_bytes(avail),
            disk::human_bytes(total)
        ),
        None => "剩余空间未知".to_string(),
    };
    println!(
        "  数据目录: {}（{}{space_text} · 可写 {}）",
        data_dir.display(),
        if dir_exists { "" } else { "尚未创建 · " },
        if writable { "✓" } else { "✗" }
    );
    if !writable {
        println!("  ⚠ 身份数据不可缺省：目录不可写时 `yekitrim server` 会拒绝启动（用 --data-dir 指到可写路径）");
    }
    if !db_path.exists() {
        println!("  账号账本: 尚未初始化（首次 `yekitrim server` 启动会自动创建 admin）");
    } else {
        match Db::open(&db_path) {
            Ok(db) => {
                let users = db.list_users()?;
                println!("  账号账本: {}（{} 个账号）", db_path.display(), users.len());
                for u in &users {
                    let flags = [
                        if u.disabled { Some("已禁用") } else { None },
                        if u.must_change_password { Some("初始密码") } else { None },
                        if u.role == "admin" { Some("管理员") } else { None },
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ");
                    println!(
                        "    - {}（{}{}）",
                        u.username,
                        if flags.is_empty() { "正常".to_string() } else { flags },
                        u.last_login_at
                            .map(|t| format!("，最后登录 {}", fmt_time(t)))
                            .unwrap_or_else(|| "，从未登录".to_string())
                    );
                }
                admin_must_change = users
                    .iter()
                    .find(|u| u.username == "admin")
                    .map(|u| u.must_change_password);
                match admin_must_change {
                    Some(true) => println!("  初始密码: 仍在使用 ⚠（对外监听会被拒绝启动）"),
                    Some(false) => println!("  初始密码: 已修改 ✓"),
                    None => println!("  初始密码: 没有 admin 账号 ⚠（`yekitrim user init-admin`）"),
                }
                println!(
                    "  会话: {} 条未过期",
                    db.count_sessions(db::now_secs()).unwrap_or(0)
                );
                let failed = db.count_audit("login_failed").unwrap_or(0);
                if failed > 0 {
                    println!("  审计: 历史登录失败 {failed} 次（连续 5 次会触发 15 分钟限流）");
                }
            }
            Err(e) => println!("  账号账本: 打不开 ✗（{e:#}）"),
        }
    }
    if pw_file.exists() {
        println!(
            "  初始密码文件: 存在 ⚠ {}（首次登录成功后自动删除；可手动删掉）",
            pw_file.display()
        );
    } else {
        println!("  初始密码文件: 不存在 ✓");
    }
    // 运行日志（`yekitrim server` 每进程一份当天的文件；顺带说清"日志在哪"）
    let log_dir = yekitrim_core::path::logs_dir();
    let latest_log = std::fs::read_dir(&log_dir)
        .ok()
        .and_then(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("yekitrim-") && n.ends_with(".log"))
                .max()
        });
    match latest_log {
        Some(name) => println!("  运行日志: {}（最新 {name}）", log_dir.display()),
        None => println!(
            "  运行日志: {}（暂无；`yekitrim server` 运行时会按天写在这里）",
            log_dir.display()
        ),
    }

    // ---- 单实例锁 ----
    line("单实例锁");
    let lock_file = instancelock::lock_path(&data_dir);
    if !lock_file.exists() {
        println!("  未发现锁文件（服务未运行过，或数据目录刚被清理）");
    } else {
        match instancelock::read_lock(&lock_file) {
            Some(info) => print_lock(&info),
            None => println!(
                "  {} → 读不到内容（独占持有 = 有实例正在运行；正常退出的实例会写 pid=0）",
                lock_file.display()
            ),
        }
    }

    // ---- 服务可达性 ----
    line("服务探测");
    match probe_health(url) {
        Some(body) => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let listening = v.get("listening").and_then(|x| x.as_str()).unwrap_or("?");
            let tls = v.get("tls_terminated").and_then(|x| x.as_bool()).unwrap_or(false);
            println!(
                "  {url}/api/health → 200（版本 {}，listening={listening}，tls_terminated={tls}）",
                v.get("version").and_then(|x| x.as_str()).unwrap_or("?"),
            );
            if listening == "remote" && !tls {
                remote_without_tls = true;
                println!(
                    "  ⚠ 对外监听但未见 TLS 终止：请用 Caddy/nginx 反代 + HTTPS，别直接暴露明文口令。\
                     \n    （反代部署还要加 `--trust-proxy`，否则服务不采信 X-Forwarded-Proto，这里永远检测不到 TLS）"
                );
            }
            // ⚠️「是否仍在使用初始密码」不再由 `/api/health` 暴露（未认证接口不该说这件事，
            // 那等于告诉远端"这台机器还在用初始密码"）。上面「初始密码」那一节直接读本机账本，
            // 信息比这里更全（逐账号列出）。
        }
        None => println!("  未检测到运行中的服务（默认探测 {DEFAULT_URL}，可用 `--url` 指定）"),
    }

    // ---- 建议（只给当前真正适用的） ----
    line("建议");
    let mut advice: Vec<String> = Vec::new();
    if admin_must_change == Some(true) {
        advice.push("仍在使用初始密码 → `yekitrim user passwd admin`（非交互：--stdin / --password-file）".into());
    }
    if pw_file.exists() {
        advice.push(format!(
            "初始密码文件仍在 {} → 首次登录成功后会自动删除；不再需要可直接删掉",
            pw_file.display()
        ));
    }
    if !writable {
        advice.push("数据目录不可写 → 用 `--data-dir <可写路径>` 或环境变量 YEKITRIM_DATA_DIR 指定别处".to_string());
    }
    if remote_without_tls {
        advice.push("对外监听但未见 TLS 终止 → 用 Caddy/nginx 反代 + HTTPS，别明文暴露口令".into());
    }
    advice.push(format!(
        "备份 = 停服后拷贝 {} + 项目目录（WAL 模式下运行中拷贝可能不一致）",
        db_path.display()
    ));
    advice.push("同一 data_dir 只能有一个实例；对外监听前必须先改密".into());
    for a in &advice {
        println!("  - {a}");
    }
    Ok(())
}

fn print_lock(info: &LockInfo) {
    if info.pid == 0 {
        println!("  {}（上次已正常退出）", lock_label(info));
    } else if instancelock::process_alive(info.pid) {
        println!("  {} → 进程仍在运行（服务在跑，第二个实例会被拒绝）", lock_label(info));
    } else {
        println!(
            "  {} → 进程已不存在（崩溃残留；下次启动会自动接管，或 `yekitrim server --force`）",
            lock_label(info)
        );
    }
}

fn lock_label(info: &LockInfo) -> String {
    if info.started_at.is_empty() {
        format!("pid={}", info.pid)
    } else {
        format!("pid={}（启动于 {}）", info.pid, info.started_at)
    }
}

fn fmt_time(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| secs.to_string())
}

/// 极简 HTTP GET（只为读 `/api/health` 的 JSON，不引入 HTTP 客户端依赖）。
pub fn probe_health(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://")?;
    let (hostport, base_path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let path = format!("{base_path}/api/health");
    let addr = hostport.to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(1500)).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(2500))).ok()?;
    stream.set_write_timeout(Some(Duration::from_millis(1500))).ok()?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {hostport}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).ok()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok()?;
    let text = String::from_utf8_lossy(&raw).to_string();
    let (head, body) = text.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return None;
    }
    let chunked = head.to_ascii_lowercase().contains("transfer-encoding: chunked");
    Some(if chunked { decode_chunked(body) } else { body.to_string() })
}

/// 解 `Transfer-Encoding: chunked`（axum 对小 JSON 通常给 content-length，这里兜底）。
fn decode_chunked(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    loop {
        let Some((size_line, after)) = rest.split_once("\r\n") else { break };
        let size = usize::from_str_radix(size_line.trim().split(';').next().unwrap_or("0"), 16)
            .unwrap_or(0);
        if size == 0 {
            break;
        }
        if after.len() < size {
            out.push_str(after);
            break;
        }
        out.push_str(&after[..size]);
        rest = after[size..].strip_prefix("\r\n").unwrap_or(&after[size..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_chunked_body() {
        assert_eq!(decode_chunked("5\r\nhello\r\n0\r\n\r\n"), "hello");
        assert_eq!(decode_chunked("2\r\nab\r\n3\r\ncde\r\n0\r\n\r\n"), "abcde");
        assert_eq!(decode_chunked("garbage"), "");
    }

    #[test]
    fn probe_health_returns_none_when_nothing_listens() {
        // 关闭的端口：必须安静地返回 None，不 panic
        assert!(probe_health("http://127.0.0.1:1").is_none());
        assert!(probe_health("not-a-url").is_none());
    }

    /// 默认端口与默认探测地址必须指向同一个端口（改一个忘另一个 = `yekitrim doctor` 探测不到自己）。
    #[test]
    fn default_url_matches_default_port() {
        assert!(
            DEFAULT_URL.ends_with(&format!(":{}", DEFAULT_PORT)),
            "DEFAULT_URL({DEFAULT_URL}) 与 DEFAULT_PORT({DEFAULT_PORT}) 不一致"
        );
    }
}
