fn main() {
    let cfg = yekitrim_core::config::load_config().unwrap();
    // S4 起窗口 / 并发 / 思考档位都住 llm.toml 的 [llm.links.correct.run] 与 [llm.links.correct]
    let run = cfg.llm.run_for(yekitrim_core::config::LlmLink::Correct);
    println!(
        "window_size={} max_concurrent={} reasoning={:?}",
        run.window_size,
        run.max_concurrent,
        cfg.llm
            .links
            .get("correct")
            .and_then(|l| l.params.reasoning.as_deref())
    );
}
