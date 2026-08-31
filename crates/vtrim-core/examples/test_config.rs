fn main() {
    let cfg = vtrim_core::config::load_config().unwrap();
    println!("window_size={} max_concurrent={} thinking={}", cfg.correct.window_size, cfg.correct.max_concurrent, cfg.correct.thinking);
}
