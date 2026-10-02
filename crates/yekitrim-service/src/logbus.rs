//! 进程内日志总线：`tracing` 事件 → 内存环形缓冲 + SSE + 可选文件。
//!
//! 解决两件事：
//! 1. `tracing` 默认只写 stdout，终端一滚（或服务在后台跑）就什么都不剩；
//! 2. 设置页要看**服务端**的实时日志（"刚才那步到底报了什么错"）。
//!
//! [`LogBusLayer`] 挂在 `main.rs` 的 subscriber 上，把每条事件同时写进：
//! - 2000 条环形缓冲（`GET /api/admin/logs` 拉历史 / 断线补齐）
//! - broadcast（`GET /api/admin/logs/events` 的 SSE 实时流）
//! - 文件 `<data_dir>/logs/yekitrim-YYYYMMDD.log`（[`enable_file_log`] 之后；按天分文件）
//!
//! **纪律**：日志层自身绝不 panic、绝不阻塞调用方 —— 任何 IO 失败静默忽略，
//! 否则"记日志"会把被记录的那件事一起弄挂。
//!
//! **为什么放在 `service/` 而不是 `server/`**：`LogBusLayer` 是 CLI 唯一需要 server
//! 侧东西的地方（`main.rs` 的 subscriber 上挂它）。它若留在 `server/`，CLI 就会反向
//! `use` 服务端，`service → server` 的单向依赖就断了 —— 那正是把服务端拆成独立 crate
//! 的前提。它本身也不碰 HTTP，只依赖 `db`，本来就属于应用层。

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, Layer};

use super::db;

/// 环形缓冲条数（面板"最近 2000 条"）+ SSE 广播队列长度。
const RING_CAP: usize = 2000;
const BROADCAST_CAP: usize = 1024;

/// 一条日志（前端 `stores/log.js` 直接消费这个形状）。
#[derive(Debug, Clone, Serialize)]
pub struct LogRecord {
    /// 进程内单调递增，前端用它做"断线重连补齐"的游标
    pub seq: u64,
    /// `HH:MM:SS.mmm`（本地时间，面板直接显示）
    pub time: String,
    pub level: String,
    /// 来源模块（`yekitrim_cli::` 前缀已剥掉）
    pub target: String,
    pub text: String,
}

#[derive(Debug)]
pub struct LogBus {
    ring: Mutex<VecDeque<LogRecord>>,
    tx: tokio::sync::broadcast::Sender<LogRecord>,
    seq: AtomicU64,
}

impl Default for LogBus {
    fn default() -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(BROADCAST_CAP);
        Self {
            ring: Mutex::new(VecDeque::new()),
            tx,
            seq: AtomicU64::new(0),
        }
    }
}

impl LogBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, level: &str, target: &str, text: String) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let record = LogRecord {
            seq,
            time: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
            level: level.to_string(),
            target: target.to_string(),
            text,
        };
        {
            let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
            ring.push_back(record.clone());
            while ring.len() > RING_CAP {
                ring.pop_front();
            }
        }
        // 没有订阅者时 send 只是返回 Err，忽略即可（不能因此丢日志的其它去处）
        let _ = self.tx.send(record);
    }

    /// 拉历史（`seq > after`，最多 `limit` 条，取**最近**的那批）。
    ///
    /// 返回 `(记录, 当前最大 seq)`：前端拿 `latest` 当 SSE 的 `after`，中间不会漏。
    pub fn snapshot(&self, after: u64, limit: usize) -> (Vec<LogRecord>, u64) {
        let ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        let latest = self.seq.load(Ordering::SeqCst);
        let mut out: Vec<LogRecord> = ring.iter().filter(|r| r.seq > after).cloned().collect();
        if out.len() > limit {
            out.drain(0..out.len() - limit);
        }
        (out, latest)
    }

    /// `(历史, 实时接收端)`：**先订阅再在同一把锁里快照**，两者之间不会漏行
    /// （抄 `JobManager::subscribe` 的做法）。
    pub fn subscribe(&self) -> (Vec<LogRecord>, tokio::sync::broadcast::Receiver<LogRecord>) {
        let rx = self.tx.subscribe();
        let ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        (ring.iter().cloned().collect(), rx)
    }
}

/// 全局总线（进程内唯一；Layer 与 HTTP handler 共用同一个）。
pub fn bus() -> Arc<LogBus> {
    static BUS: OnceLock<Arc<LogBus>> = OnceLock::new();
    BUS.get_or_init(|| Arc::new(LogBus::new())).clone()
}

// ---- 文件落盘 ----

static FILE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 打开文件落盘，返回目录。**`vtrim server` 在解析完 `--data-dir` 后第一时间调用**
/// （之后的日志才进文件，所以要在启动横幅之前）。目录 = `<data_dir>/logs`。
pub fn enable_file_log() -> PathBuf {
    let dir = FILE_DIR
        .get_or_init(|| db::data_dir().join("logs"))
        .clone();
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn file_log_dir() -> Option<PathBuf> {
    FILE_DIR.get().cloned()
}

/// 当前（或最近的）日志文件路径：`<dir>/yekitrim-YYYYMMDD.log`。
pub fn file_log_path(dir: &std::path::Path) -> PathBuf {
    dir.join(format!("yekitrim-{}.log", chrono::Local::now().format("%Y%m%d")))
}

#[derive(Default)]
struct FileState {
    /// 已打开文件对应的日期（`%Y%m%d`），跨天时重开
    date: String,
    file: Option<std::fs::File>,
}

// ---- tracing Layer ----

/// 把事件写进 [`LogBus`]（+ 可选文件）。
///
/// 字段渲染：`message` 是正文，其余字段按 `k=v` 追加到**第一行**末尾
/// （`user=admin`、`job=…` 这类结构化信息照样看得见）。
pub struct LogBusLayer {
    bus: Arc<LogBus>,
    /// 显式目录（单测用）；`None` = 跟随 [`enable_file_log`] 的全局目录
    explicit_dir: Option<PathBuf>,
    file: Mutex<FileState>,
}

impl LogBusLayer {
    pub fn new(bus: Arc<LogBus>) -> Self {
        Self { bus, explicit_dir: None, file: Mutex::new(FileState::default()) }
    }

    /// 固定落盘目录（单测用：不依赖全局状态）。
    #[cfg(test)]
    pub fn with_file_dir(bus: Arc<LogBus>, dir: PathBuf) -> Self {
        Self { bus, explicit_dir: Some(dir), file: Mutex::new(FileState::default()) }
    }

    fn write_file(&self, level: &str, target: &str, text: &str) {
        let dir = match self.explicit_dir.clone().or_else(file_log_dir) {
            Some(d) => d,
            None => return,
        };
        let now = chrono::Local::now();
        let date = now.format("%Y%m%d").to_string();
        let mut st = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if st.file.is_none() || st.date != date {
            if std::fs::create_dir_all(&dir).is_err() {
                return;
            }
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(format!("yekitrim-{date}.log")))
            {
                Ok(f) => {
                    st.file = Some(f);
                    st.date = date;
                }
                Err(_) => return,
            }
        }
        if let Some(f) = st.file.as_mut() {
            let _ = writeln!(
                f,
                "{} {} {} {}",
                now.format("%Y-%m-%d %H:%M:%S%.3f"),
                level.to_uppercase(),
                target,
                text
            );
        }
    }
}

impl<S> Layer<S> for LogBusLayer
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);

        let meta = event.metadata();
        let level = level_str(meta.level());
        let target = short_target(meta.target());
        let message = visitor.message.unwrap_or_default();

        let mut extra = String::new();
        for (k, v) in &visitor.extra {
            extra.push(' ');
            extra.push_str(k);
            extra.push('=');
            extra.push_str(v);
        }

        // 换行拆成多条：子进程输出的多行尾/报错栈不会挤成一坨
        let mut lines: Vec<&str> = message.split('\n').map(|l| l.trim_end_matches('\r')).collect();
        if lines.len() > 1 {
            lines.retain(|l| !l.trim().is_empty());
        }
        if lines.is_empty() {
            lines.push("");
        }
        for (i, line) in lines.iter().enumerate() {
            let text = if i == 0 {
                format!("{line}{extra}")
            } else {
                (*line).to_string()
            };
            self.bus.push(level, &target, text.clone());
            self.write_file(level, &target, &text);
        }
    }
}

#[derive(Default)]
struct FieldVisitor {
    message: Option<String>,
    extra: Vec<(String, String)>,
}

impl FieldVisitor {
    fn put(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = Some(value);
        } else {
            self.extra.push((field.name().to_string(), value));
        }
    }
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        // `%x`（Display）与 `?x`（Debug）都走到这里；message 是 `fmt::Arguments`，
        // 它的 Debug 就是渲染后的正文。
        self.put(field, format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, value.to_string());
    }
}

fn level_str(level: &tracing::Level) -> &'static str {
    match *level {
        tracing::Level::ERROR => "error",
        tracing::Level::WARN => "warn",
        tracing::Level::INFO => "info",
        tracing::Level::DEBUG => "debug",
        tracing::Level::TRACE => "trace",
    }
}

/// 面板里显示的模块名：剥掉 crate 前缀，只留 `http` / `job` / `api::…` 这种。
///
/// 要把三个前缀都列上：日志来自三个 crate（service / server / cli），而 target 既可能是
/// `module_path!()` 推出来的，也可能是调用点硬写的字符串字面量。少一个前缀，面板里就会
/// 突然出现 `yekitrim_server::api` 这种带 crate 名的长 target —— 只是难看，但很难查。
fn short_target(target: &str) -> String {
    for prefix in ["yekitrim_cli::", "yekitrim_service::", "yekitrim_server::"] {
        if let Some(rest) = target.strip_prefix(prefix) {
            return rest.to_string();
        }
    }
    target.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    /// 把当前线程的默认 subscriber 换成"只挂 LogBusLayer"的 registry。
    fn capture(bus: Arc<LogBus>) -> tracing::subscriber::DefaultGuard {
        let sub = tracing_subscriber::registry().with(LogBusLayer::new(bus));
        tracing::subscriber::set_default(sub)
    }

    #[test]
    fn captures_message_level_target_and_fields() {
        let bus = Arc::new(LogBus::new());
        let _guard = capture(bus.clone());
        tracing::info!(target: "yekitrim_cli::http", user = %"admin", "GET /api/me → 200 (3ms)");

        let recs = bus.snapshot(0, 10).0;
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].level, "info");
        assert_eq!(recs[0].target, "http");
        assert_eq!(recs[0].text, "GET /api/me → 200 (3ms) user=admin");
        assert_eq!(recs[0].seq, 1);
    }

    /// 三个 crate 的前缀都要剥掉。少一个（最典型的是只写 `yekitrim_cli::`）
    /// 面板里就会冒出 `yekitrim_server::api` 这种带 crate 名的 target —— 不报错、只是难看，
    /// 属于"搬家之后没人会回头查"的那类退化。
    #[test]
    fn short_target_strips_every_crate_prefix() {
        assert_eq!(short_target("yekitrim_cli::http"), "http");
        assert_eq!(short_target("yekitrim_service::db"), "db");
        assert_eq!(short_target("yekitrim_server::api::router"), "api::router");
        // 不是自家 crate 的 target 原样保留（第三方库的日志也在总线里）
        assert_eq!(short_target("axum::serve"), "axum::serve");
        assert_eq!(short_target("http"), "http");
    }

    #[test]
    fn multiline_message_splits_into_records() {
        let bus = Arc::new(LogBus::new());
        let _guard = capture(bus.clone());
        tracing::warn!(target: "yekitrim_cli::job", job = %"j1", "第一行\n第二行\n\n第三行");

        let recs = bus.snapshot(0, 10).0;
        let texts: Vec<&str> = recs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, vec!["第一行 job=j1", "第二行", "第三行"]);
        assert!(recs.iter().all(|r| r.level == "warn"));
    }

    #[test]
    fn ring_is_capped_and_seq_monotonic() {
        let bus = LogBus::new();
        for i in 0..(RING_CAP + 100) {
            bus.push("info", "t", format!("line {i}"));
        }
        let (recs, latest) = bus.snapshot(0, usize::MAX);
        assert_eq!(recs.len(), RING_CAP);
        assert_eq!(latest as usize, RING_CAP + 100);
        // 最早的一条已被挤掉，且 seq 严格递增
        assert_eq!(recs[0].seq, 101);
        assert!(recs.windows(2).all(|w| w[0].seq < w[1].seq));
    }

    #[test]
    fn snapshot_filters_by_after_and_limit() {
        let bus = LogBus::new();
        for i in 0..10 {
            bus.push("info", "t", format!("line {i}"));
        }
        let (recs, latest) = bus.snapshot(7, 100);
        assert_eq!(latest, 10);
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[0].seq, 8);
        // limit 只保留最近的那批
        let (recs, _) = bus.snapshot(0, 3);
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[0].seq, 8);
    }

    #[test]
    fn subscribe_returns_history_then_live() {
        let bus = LogBus::new();
        bus.push("info", "t", "before".into());
        let (history, mut rx) = bus.subscribe();
        assert_eq!(history.len(), 1);
        bus.push("info", "t", "after".into());
        let rec = rx.try_recv().expect("实时接收端应拿到订阅后的一条");
        assert_eq!(rec.text, "after");
        assert_eq!(rec.seq, 2);
    }

    #[test]
    fn file_layer_appends_daily_file() {
        let dir = tempfile::tempdir().unwrap();
        let bus = Arc::new(LogBus::new());
        let sub = tracing_subscriber::registry()
            .with(LogBusLayer::with_file_dir(bus.clone(), dir.path().to_path_buf()));
        let _guard = tracing::subscriber::set_default(sub);

        tracing::error!(target: "yekitrim_cli::job", "渲染失败: 退出码 1");

        let path = file_log_path(dir.path());
        let raw = std::fs::read_to_string(&path).expect("日志文件应已写入");
        assert!(raw.contains("ERROR"), "文件里应有级别：{raw}");
        assert!(raw.contains("渲染失败: 退出码 1"), "文件里应有正文：{raw}");
        assert!(raw.contains("job"), "文件里应有 target：{raw}");
    }
}
