pub trait ProgressReporter: Send + Sync {
    fn stage_started(&self, name: &str);
    fn stage_progress(&self, current: u64, total: u64);
    fn stage_completed(&self, name: &str);
    fn error(&self, msg: &str);
    /// **任务消息**：值得让用户看见、但**不该把任务判成失败**的一句话
    /// （剪切"按配置跳过 N 条 / M 条失败但其余已出片"这类汇总）。
    ///
    /// ⚠️ 与 `error` 的分工：server 侧 `error` = 整条任务 fail；部分失败 / 跳过只能走这条。
    /// 默认 no-op：不接任务消息面的实现（CLI 打印、纯测试）什么都不用做。
    fn note(&self, _msg: &str) {}
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
