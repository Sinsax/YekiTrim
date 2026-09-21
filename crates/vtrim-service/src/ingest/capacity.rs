//! 入库前的**水位/配额预检**（设计稿 §7）。
//!
//! 从 `server::ingest_worker` 下沉到这里，理由有两条：
//! 1. 它**与录播姬无关** —— 只认「谁的配额 / 全局上限 / 磁盘水位」，B站下载通道要同一套规则；
//! 2. 两个 worker 不该互相调用（否则录播姬那套待分配池语义会渗进 B站通道）。
//!
//! 纪律：**不足一律报错，绝不自动删任何用户数据**。

use std::path::Path;

use anyhow::{bail, Result};

use crate::db::Db;
use crate::disk;

/// 保留量的**绝对下界**：再小的卷也要留下这么多，避免把盘写到 0。
const RESERVE_MIN_BYTES: u64 = 512 * 1024 * 1024; // 512 MiB

/// 按比例算出的保留量的**封顶**。
///
/// 没有这个封顶，`disk_watermark_pct = 10` 在 931.5 GB 的数据盘上会要求保留 93.2 GB，
/// 于是"可用 80.7 GB"的盘连一个 **8.8 MB** 的入库都拒，而提示还让用户"清理磁盘"——
/// 那条线永远清不到。比例只该对中小卷生效，大盘上的保留量必须有绝对上限。
const RESERVE_MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024; // 8 GiB

/// 由「卷总容量 + 水位百分比」算出**必须保留**的字节数。
///
/// - `pct == 0` = **关闭**水位检查（历史约定，单测靠它把水位关掉）；
/// - 其余按 `total * pct / 100` 算，再钳制到 `[RESERVE_MIN_BYTES, RESERVE_MAX_BYTES]`。
pub fn reserve_bytes(total: u64, pct: u32) -> u64 {
    if pct == 0 {
        return 0;
    }
    (total / 100 * pct.min(99) as u64).clamp(RESERVE_MIN_BYTES, RESERVE_MAX_BYTES)
}

/// 水位判定：**写完这一份之后**剩余空间跌破保留底线才拒绝。
///
/// 抽成纯函数是为了能用确定性的 `(avail, total, pct, incoming)` 复现线上场景，
/// 不受跑测试那台机器当下剩余空间的影响。
pub fn watermark_rejects(avail: u64, total: u64, pct: u32, incoming: u64) -> bool {
    let reserve = reserve_bytes(total, pct);
    reserve > 0 && avail.saturating_sub(incoming) < reserve
}

/// 水位/配额预检。
///
/// 阈值全部来自传入的 `cfg`（而不是在这里重新读全局配置）：调用方拿的是同一份快照，
/// 测试也能构造确定性的阈值。
///
/// 三档依次判定，**都通过才放行**：
/// 1. 每用户配额（`users.quota_bytes` 优先，其次 `[storage] default_quota_bytes`）；
/// 2. 全平台磁盘上限 `[storage] max_disk_bytes`（0 = 不限）；
/// 3. 目标磁盘**写完这一份之后**的剩余空间不得跌破保留底线
///    （底线由 `[storage] disk_watermark_pct` 算出，但被钳制，见 [`reserve_bytes`]）。
///
/// `incoming` 是**预计占用**（录像文件大小 / 下载器预估体积）。取不到准确值时宁可给大一点：
/// 误拒的代价是"用户重试一次"，放过的代价是把磁盘写满。
///
/// ⚠️ 水位这一档曾经只看"可用 / 整盘比例"，与 `incoming` 完全无关，于是在大容量数据盘上
/// 会把一个几 MB 的入库判死（931.5 GB 的盘要求保留 93.2 GB）。现在两处修正：
/// 保留量有绝对封顶，且判定必须带上本次占用。
pub fn check_capacity(
    db: &Db,
    owner_id: i64,
    incoming: u64,
    target_dir: &Path,
    cfg: &vtrim_core::config::Config,
) -> Result<()> {
    // 每用户配额
    let user_quota = db
        .find_user_by_id(owner_id)?
        .map(|u| u.quota_bytes.max(0) as u64)
        .filter(|q| *q > 0)
        .or_else(|| {
            let q = cfg.storage.default_quota_bytes;
            if q > 0 {
                Some(q)
            } else {
                None
            }
        });
    if let Some(quota) = user_quota {
        let used = db
            .storage_summary(Some(owner_id))?
            .first()
            .map(|(_, b)| *b)
            .unwrap_or(0);
        if used + incoming > quota {
            bail!(
                "入库被拒绝：本账号配额不足（预计需要 {}，可用 {}）；请删除部分项目的源录播后重试",
                disk::human_bytes(incoming),
                disk::human_bytes(quota.saturating_sub(used))
            );
        }
    }

    // 全局上限
    let max_disk = cfg.storage.max_disk_bytes;
    if max_disk > 0 {
        let used: u64 = db.storage_summary(None)?.iter().map(|(_, b)| *b).sum();
        if used + incoming > max_disk {
            bail!(
                "入库被拒绝：服务端磁盘上限已达（预计需要 {}，可用 {}）；请在存储页删除部分项目的源录播",
                disk::human_bytes(incoming),
                disk::human_bytes(max_disk.saturating_sub(used))
            );
        }
    }

    // 磁盘水位：写完这一份之后剩余空间不得跌破保留底线
    if let Some((avail, total)) = disk::space_bytes(target_dir) {
        let pct = cfg.storage.disk_watermark_pct;
        if watermark_rejects(avail, total, pct, incoming) {
            bail!(
                "入库被拒绝：目标磁盘可用空间不足（本次需要 {}，磁盘可用 {}，需保留底线 {}）；请清理磁盘后重试",
                disk::human_bytes(incoming),
                disk::human_bytes(avail),
                disk::human_bytes(reserve_bytes(total, pct))
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_precheck_reports_quota_shortage() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("vtrim.db")).unwrap();
        let uid = db.create_user("alice", "h", "user", false, 1000).unwrap();
        db.create_project(
            uid,
            "p",
            None,
            "local",
            None,
            dir.path(),
            &dir.path().join("vtrim.toml"),
            None,
        )
        .unwrap();
        db.update_project_bytes(1, 900, 0).unwrap();
        // 关掉水位，专心验配额（水位有自己的纯函数单测，不受本机剩余空间影响）
        let mut cfg = vtrim_core::config::Config::default();
        cfg.storage.disk_watermark_pct = 0;

        let err = check_capacity(&db, uid, 500, dir.path(), &cfg).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("预计需要"), "{msg}");
        assert!(msg.contains("可用"), "{msg}");
        // 配额内放行
        assert!(check_capacity(&db, uid, 50, dir.path(), &cfg).is_ok());
    }

    #[test]
    fn capacity_precheck_enforces_global_max_and_watermark() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("vtrim.db")).unwrap();
        let uid = db.create_user("bob", "h", "user", false, 0).unwrap();

        // 全局上限 100 字节 → 入库 200 字节被拒
        let mut cfg = vtrim_core::config::Config::default();
        cfg.storage.disk_watermark_pct = 0;
        cfg.storage.max_disk_bytes = 100;
        let err = format!("{:#}", check_capacity(&db, uid, 200, dir.path(), &cfg).unwrap_err());
        assert!(err.contains("磁盘上限"), "{err}");

        // 水位 0 = 关闭 → 放行（不依赖跑测试那台机器当下的剩余空间）
        cfg.storage.max_disk_bytes = 0;
        assert!(check_capacity(&db, uid, 1, dir.path(), &cfg).is_ok());
    }

    /// 保留量必须在两端都被钳住：比例只对中小容量卷有意义。
    #[test]
    fn reserve_bytes_is_capped_on_large_volumes() {
        let gib = 1024u64 * 1024 * 1024;
        // 0% = 关闭水位
        assert_eq!(reserve_bytes(931 * gib, 0), 0);
        // 931.5 GB @ 10% = 93.2 GB → 封顶 8 GiB
        assert_eq!(reserve_bytes(931 * gib, 10), 8 * gib);
        // 100% 也照样封顶（否则等于把整盘锁死）
        assert_eq!(reserve_bytes(931 * gib, 100), 8 * gib);
        // 中小卷按比例：50 GB @ 10% = 5 GiB
        assert_eq!(reserve_bytes(50 * gib, 10), 5 * gib);
        // 极小卷保底 512 MiB（不留 0，避免把盘写到满）
        assert_eq!(reserve_bytes(gib, 10), 512 * 1024 * 1024);
        assert_eq!(reserve_bytes(0, 10), 512 * 1024 * 1024);
    }

    /// 线上真实报障场景：F: 盘总量 931.5 GB / 可用 80.7 GB，入库 8.8 MB 被拒。
    ///
    /// 修复前 `floor = total * 10% = 93.2 GB > 可用 80.7 GB` → **无论请求多大都拒**，
    /// 而且提示让用户"清理磁盘"永远清不到那条线。修复后 8 GiB 底线完全放得下。
    #[test]
    fn watermark_allows_small_ingest_on_large_but_full_volume() {
        let gib = 1024u64 * 1024 * 1024;
        let total = 931 * gib; // 931.5 GB
        let avail = 80 * gib; // 80.7 GB
        let incoming = 8_800_000; // 8.8 MB

        assert!(
            !watermark_rejects(avail, total, 10, incoming),
            "80 GB 可用的盘不该因为「整盘 10%」而拒绝 8.8 MB 的入库"
        );

        // 真的跌破 8 GiB 底线时才拒；且此时大请求同样被拒
        assert!(watermark_rejects(7 * gib, total, 10, incoming));
        assert!(watermark_rejects(8 * gib, total, 10, 2 * gib));
        // 关闭水位 = 永不因水位拒绝
        assert!(!watermark_rejects(0, total, 0, incoming));
    }

    /// 真实盘体检（人工触发，CI 不跑）：
    /// `cargo test -p vtrim-service --lib real_volume_probe -- --ignored --nocapture`
    ///
    /// 用来复现"明明还有几十 GB 却拒绝入库"的现场 —— 逐行打出
    /// 可用 / 总量 / 各百分比下的保留底线 / 本次 8.8 MB 的判定结果。
    #[test]
    #[ignore = "要读真实磁盘容量，默认不跑"]
    fn real_volume_probe() {
        let dir = std::env::current_dir().unwrap();
        let Some((avail, total)) = disk::space_bytes(&dir) else {
            eprintln!("{} 所在卷读不到容量，水位这一档会被跳过", dir.display());
            return;
        };
        eprintln!("卷 {}: 可用={} 总量={}",
            dir.display(), disk::human_bytes(avail), disk::human_bytes(total));
        for pct in [10u32, 0] {
            eprintln!(
                "  pct={pct:<3} 保留底线={:>10}  拒绝 8.8 MB? {}",
                disk::human_bytes(reserve_bytes(total, pct)),
                if watermark_rejects(avail, total, pct, 8_800_000) { "是" } else { "否" }
            );
        }
    }
}
