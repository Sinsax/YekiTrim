//! tracing 装配 —— 由**两个 bin 共用**（`main.rs` 与 `tray.rs` 各自 `mod tracing_setup;` 引同一份文件）。
//!
//! ⚠️ 为什么要有这个文件：这份装配原来只在 `main.rs` 里、且是私有的。托盘是**另一个 bin**，
//! 拿不到它 —— 结果托盘起服务时**一个订阅者都没有**：日志文件是空的、WebUI 的「运行日志」面板也是空的。
//! 一个没有控制台的后台进程如果连日志都没有，出问题就完全瞎了（实测踩到：logger 目录建了但空的）。

use tracing_subscriber::EnvFilter;

/// 装上订阅者。
///
/// `terminal=true`（CLI）额外挂 `fmt::layer()` 打到终端；
/// `terminal=false`（托盘）**只挂日志总线** —— 托盘是 `windows_subsystem="windows"`，
/// 根本没有 stdout，挂终端层只会白写。
///
/// `LogBusLayer` 两种模式都**必须**挂：`yekitrim server` 的访问日志、任务日志、错误文案都靠它
/// 进设置页「运行日志」与日志文件。
pub fn setup(terminal: bool) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // 全局过滤器（RUST_LOG，默认 info）：所有层都受它约束，
    // 想看高频接口的逐条访问日志用 `RUST_LOG=yekitrim=debug`
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // `Option<Layer>` 本身就是个 no-op 层，所以"要不要终端"不用分两条 builder 链
    // （两条链的返回类型不同，没法赋给同一个变量）
    let term = terminal.then(tracing_subscriber::fmt::layer);
    tracing_subscriber::registry()
        .with(filter)
        .with(term)
        .with(yekitrim_service::logbus::LogBusLayer::new(
            yekitrim_service::logbus::bus(),
        ))
        .init();
}
