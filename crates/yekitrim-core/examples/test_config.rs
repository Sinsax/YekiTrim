fn main() {
    let cfg = yekitrim_core::config::load_config().unwrap();
    // `max_concurrent` 是可选覆盖（None = 跟随 [llm] max_concurrent），所以用 Debug 打
    println!(
        "window_size={} max_concurrent={:?} thinking={}",
        cfg.correct.window_size, cfg.correct.max_concurrent, cfg.correct.thinking
    );
}
