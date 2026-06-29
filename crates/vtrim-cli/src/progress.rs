use indicatif::{ProgressBar, ProgressStyle};
use vtrim_core::progress::ProgressReporter;

pub struct IndicatifProgress {
    bar: ProgressBar,
}

impl IndicatifProgress {
    pub fn new() -> Self {
        let bar = ProgressBar::new(0);
        bar.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] {msg} {bar:40} {percent}%")
                .unwrap()
                .progress_chars("#>-"),
        );
        Self { bar }
    }
}

impl ProgressReporter for IndicatifProgress {
    fn stage_started(&self, name: &str) {
        self.bar.reset();
        self.bar.set_length(1);
        self.bar.set_position(0);
        self.bar.set_message(format!("{} ...", name));
    }

    fn stage_progress(&self, current: u64, total: u64) {
        if total > 0 {
            self.bar.set_length(total);
        }
        self.bar.set_position(current);
    }

    fn stage_completed(&self, name: &str) {
        self.bar.finish_with_message(format!("✓ {} 完成", name));
    }

    fn error(&self, msg: &str) {
        self.bar
            .finish_with_message(format!("✗ 错误: {}", msg));
    }
}