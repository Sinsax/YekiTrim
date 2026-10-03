//! 出片**吞吐基准**（技术探索用，不参与默认测试）。
//!
//! ⚠️ **2026-10-01 起只剩「字节搬运」微基准那条**：另一条 `render_throughput` 量的
//! 是**本仓 headless Chrome 逐帧腿**的"抽帧 / 渲染 / 每帧毫秒"，而那条腿已整条删除
//! （用户拍板：出片只有独立的 Dhampir 底座一条腿）。底座是"整段一起出"的子进程，
//! 它自己的吞吐由底座那一侧的性能测试负责，本仓再去量一个已经在做别的事的中间层没有意义
//! —— 所以那条基准跟它的被测对象一起删掉了，**不是**被跳过或改名保留。
//!
//! 留下来这条量的是**协议搬运本身**（base64 编码 → 拼 JSON → 解析 → base64 解码）：
//! 它是当年那条逐帧腿每帧 75ms 里占 ~45~55ms 的那一段（页面自报只有 ~26ms）。
//! 现在没有逐帧腿了，但它仍是"任何逐帧/分批协议"的成本下限 —— 底座将来若也开逐帧通道，
//! 这些数字就是它的预算参考。**它不起浏览器、不依赖外部二进制，所以谁都能跑。**
//!
//! ```text
//! cargo test -p yekitrim-polish --test render_bench -- --ignored plumbing --nocapture
//! cargo test --release -p yekitrim-polish --test render_bench -- --ignored plumbing --nocapture
//! ```
//! `VTEDIT_BENCH_FRAMES`（默认 300，与出片日志里的分块帧数同量级）、
//! `VTEDIT_BENCH_FRAME_KB`（默认 200 = 1080p JPEG q0.6 的典型体积）。

use std::time::Instant;

/// 出片「字节搬运」微基准：把逐帧协议每帧的进出两趟（base64 编码 → 拼 JSON → 传输 →
/// serde 解析 → base64 解码）照原样复刻一遍，**不起任何外部进程**。
///
/// 为什么单独量它：日志里每帧 75ms 中只有 ~26ms 是页面/renderer 自报的（JPEG 解码 + canvas +
/// toBlob + base64），剩下 ~45~55ms 全在这一段字节搬运上，而它正是 debug 构建被放大得最狠的地方。
#[test]
#[ignore = "手动微基准：量 base64/JSON 搬运的每帧成本（debug vs release）"]
fn plumbing_microbench() {
    use base64::Engine as _;

    let n: usize = std::env::var("VTEDIT_BENCH_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let frame_kb: usize = std::env::var("VTEDIT_BENCH_FRAME_KB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    let frame: Vec<u8> = (0..frame_kb * 1024).map(|i| (i * 31 % 251) as u8).collect();
    let b64 = base64::engine::general_purpose::STANDARD;
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };

    // ① 发送侧：每批 50 帧一次 JSON（原来那条逐帧腿的批大小）
    let t_send = Instant::now();
    let mut sent_json_bytes = 0usize;
    for _batch in (0..n).collect::<Vec<_>>().chunks(50) {
        let batch_b64: Vec<String> = (0..50).map(|_| b64.encode(&frame)).collect();
        let json = serde_json::to_string(&batch_b64).unwrap();
        sent_json_bytes += json.len();
        std::hint::black_box(&json);
    }
    let send_s = t_send.elapsed().as_secs_f64();

    // ② 接收侧：每批 20 帧一次解析 + 解码（同那条腿的接收批大小）
    let wire: Vec<String> = (0..n).map(|_| b64.encode(&frame)).collect();
    let t_recv = Instant::now();
    let mut decoded = 0usize;
    for chunk in wire.chunks(20) {
        let json = serde_json::to_string(chunk).unwrap();
        let parsed: Vec<String> = serde_json::from_str(&json).unwrap();
        for s in parsed {
            decoded += b64.decode(s.as_bytes()).unwrap().len();
        }
    }
    let recv_s = t_recv.elapsed().as_secs_f64();

    println!(
        "[{profile}] {} 帧 × {frame_kb}KB · 发送侧 {send_s:.2}s（{:.2} ms/帧，出网 {:.0} MB）· \
         接收侧 {recv_s:.2}s（{:.2} ms/帧，回来 {:.0} MB）· 合计 {:.2} ms/帧",
        n,
        send_s / n as f64 * 1000.0,
        sent_json_bytes as f64 / 1e6,
        recv_s / n as f64 * 1000.0,
        decoded as f64 / 1e6,
        (send_s + recv_s) / n as f64 * 1000.0
    );
}
