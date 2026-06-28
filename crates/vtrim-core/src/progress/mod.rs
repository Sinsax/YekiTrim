pub trait ProgressReporter: Send + Sync {
    fn stage_started(&self, name: &str);
    fn stage_progress(&self, current: u64, total: u64);
    fn stage_completed(&self, name: &str);
    fn error(&self, msg: &str);
}

pub struct NoopProgress;

impl ProgressReporter for NoopProgress {
    fn stage_started(&self, _name: &str) {}
    fn stage_progress(&self, _current: u64, _total: u64) {}
    fn stage_completed(&self, _name: &str) {}
    fn error(&self, _msg: &str) {}
}
