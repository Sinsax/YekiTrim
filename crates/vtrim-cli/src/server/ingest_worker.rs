//! 录播姬入库 worker（P3b 主体）。
//!
//! P3a 只提供"唤醒"入口：webhook / 指派 / 重试落库后调用 [`notify`]，
//! 由 worker 去判断"这场直播是否已完结"并执行合并/落位/跑管线。
//! 当前实现是**日志占位**（真正逻辑随 P3b 落地），保证 webhook 先能跑通并立即 2xx。

use std::sync::Arc;

use super::model::AppState;

/// 唤醒 worker 处理某个会话（`None` = 扫描所有待处理会话）。
pub fn notify(state: Arc<AppState>, session_id: Option<String>) {
    match session_id {
        Some(id) => tracing::debug!("ingest worker 唤醒: session={id}（P3b 实现入库流程）"),
        None => tracing::debug!("ingest worker 唤醒: 全量扫描（P3b 实现入库流程）"),
    }
    let _ = state;
}
