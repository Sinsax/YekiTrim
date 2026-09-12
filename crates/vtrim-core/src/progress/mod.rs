pub trait ProgressReporter: Send + Sync {
    fn stage_started(&self, name: &str);
    fn stage_progress(&self, current: u64, total: u64);
    fn stage_completed(&self, name: &str);
    fn error(&self, msg: &str);
    /// 任务是否已被请求取消。长任务在**阶段/批次边界**轮询它，返回 true 时应尽快
    /// `bail!` 退出。默认 false：CLI 等同步场景不需要取消语义。
    ///
    /// 语义约定：进行中的单次 ffmpeg / ASR / LLM 调用不会被中断，
    /// 取消在下一个阶段或批次边界生效。
    fn cancelled(&self) -> bool {
        false
    }
}

pub struct NoopProgress;

impl ProgressReporter for NoopProgress {
    fn stage_started(&self, _name: &str) {}
    fn stage_progress(&self, _current: u64, _total: u64) {}
    fn stage_completed(&self, _name: &str) {}
    fn error(&self, _msg: &str) {}
}
