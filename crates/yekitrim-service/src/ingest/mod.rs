//! `yekitrim ingest`：录播姬采集相关的 CLI（P3）。
//!
//! - `yekitrim ingest token`：生成 webhook token（明文只显示一次），把哈希写进全局配置
//!   `[ingest] token_hash` 并开启 `webhook_enabled`；
//! - `yekitrim ingest token --disable`：只关闭 webhook（保留 token）；
//! - `yekitrim ingest status`：显示当前采集配置（不打印任何秘密）。
//!
//! ## 这里同时是"入库共享逻辑"的落点
//!
//! `capacity`（水位/配额预检）与 `landing`（把一份素材落成项目）**与录播姬无关**，
//! 只是历史上长在 `server::ingest_worker` 里。B站下载通道要复用同一套规则，
//! 所以它们被下沉到这里 —— 见 `docs/plan/p4-implementation-plan.md` §2。
//!
//! ⚠️ 它们的**消费者**是 `yekitrim-server` 的两个 worker（`ingest_worker` / `download_worker`）。
//! **两个 worker 之间不许互相调用**：共享的东西一律下沉到本模块，否则录播姬那套
//! 「待分配池 / is_finalized 空闲窗口」的语义会顺着调用链渗进 B站通道。

use std::path::PathBuf;

use anyhow::{Context, Result};

use super::auth;

pub mod capacity;
pub mod landing;

/// 程序配置的唯一位置（见 `crate::init_config::global_config_path` 的说明）。
///
/// ⚠️ 2026-11-13 起是 **owner 路径**（`users/<uid>/.yekitrim/config.toml`），不再是顶层那份。
fn global_config_path() -> PathBuf {
    yekitrim_core::config::global_config_path()
}

/// 录播姬 webhook token 的前缀。
///
/// ⚠️ **唯一权威，生成端与校验端必须同源**。校验端在 `yekitrim-server/src/ingest/webhook.rs`
/// 的 `token_shape_ok`（它是 webhook 鉴权的**快速否决闸**，形状不对直接 401）。
/// 这两处**曾经各写一个字面量**：改名时只改了生成端 ⇒ `generate_token()` 产出的每个 token
/// 都通不过自己的校验 ⇒ **用户在设置页点「生成/轮换」拿到的 token 100% 401**，
/// 录播姬采集整条链**完全不可用且不报错**（只回 401）。
/// 现在两端都引用本常量，不可能再分叉。
pub const TOKEN_PREFIX: &str = "yekitrim_";

/// 生成不可猜测的 token（URL 安全、人可复制：`yekitrim_` + 40 位 base64url）。
pub fn generate_token() -> String {
    let mut raw = String::new();
    for _ in 0..3 {
        raw.push_str(&auth::new_token());
    }
    format!("{TOKEN_PREFIX}{}", &raw[..40])
}

/// 把 `[ingest]` 的若干键合并写进全局配置（保留注释与其他 section）。
fn patch_ingest(pairs: serde_json::Value) -> Result<()> {
    let path = global_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建配置目录失败: {}", parent.display()))?;
    }
    if !path.exists() {
        // 与 `yekitrim server` 一致的补齐行为
        let _ = crate::ensure_global_config();
    }
    yekitrim_core::config::patch_config_file(&path, &serde_json::json!({ "ingest": pairs }))
        .with_context(|| format!("写入 {} 失败", path.display()))?;
    Ok(())
}

/// `yekitrim ingest token [--disable]`
pub fn cmd_token(disable: bool) -> Result<()> {
    if disable {
        patch_ingest(serde_json::json!({ "webhook_enabled": false }))?;
        println!("已关闭录播姬 webhook（token 仍保留在 [ingest] token_hash）");
        return Ok(());
    }

    let token = generate_token();
    let hash = auth::hash_password(&token)?;
    patch_ingest(serde_json::json!({
        "token_hash": hash,
        "webhook_enabled": true,
    }))?;

    let cfg = yekitrim_core::config::load_global_config();
    println!();
    println!("  录播姬 webhook token（只显示这一次，请立即填入录播姬）");
    println!("  {token}");
    println!();
    println!("  在录播姬「Webhook」里填：");
    println!(
        "    http://<本机IP>:{}/api/ingest/bililive/{token}",
        crate::doctor::DEFAULT_PORT
    );
    println!("  并在配置文件 {} 里确认：", global_config_path().display());
    println!("    [ingest] webhook_enabled = true");
    println!("    [ingest.recorder] root = \"<录播姬录制目录>\"");
    if !cfg.ingest.allow_ips.is_empty() {
        println!("  来源白名单: {:?}", cfg.ingest.allow_ips);
    }
    println!("  （token 只存哈希；丢失请重新运行本命令生成）");
    Ok(())
}

/// `yekitrim ingest status`
pub fn cmd_status() -> Result<()> {
    let cfg = yekitrim_core::config::load_global_config();
    let ing = &cfg.ingest;
    println!("录播姬采集配置（{}）", global_config_path().display());
    println!("  webhook:    {}", if ing.webhook_enabled { "已启用" } else { "未启用" });
    println!(
        "  token:      {}",
        if ing.token_hash.trim().is_empty() { "未生成" } else { "已配置（哈希）" }
    );
    println!(
        "  来源白名单: {}",
        if ing.allow_ips.is_empty() { "不限".to_string() } else { ing.allow_ips.join(", ") }
    );
    println!("  录制目录:   {}", if ing.recorder.root.is_empty() { "（未设置，入库不可用）".to_string() } else { ing.recorder.root.clone() });
    println!("  兜底扫描:   每 {} 分钟（0 = 关闭）", ing.recorder.watch_interval_mins);
    println!("  收尾空闲:   SessionEnded 后 {} 分钟无新文件视为结束", ing.recorder.finalize_idle_mins);
    println!(
        "  落位策略:   {}",
        if ing.recorder.move_into_managed { "移入托管项目目录" } else { "留在录制目录" }
    );
    println!(
        "  多分片:     {}",
        if ing.recorder.concat_files { "合并为 source.mp4" } else { "只取第一个分片" }
    );
    println!("  自动策略:   {}", ing.recorder.auto_pipeline);
    println!("  扩展名:     {}", ing.recorder.extensions.join(", "));
    Ok(())
}
