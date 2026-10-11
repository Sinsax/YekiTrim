pub mod charlog;
/// **T5 真卡拉OK**：把字级时间轴的对齐结论落成 `data/karaoke-cues.json`（派生产物）。
///
/// ⚠️ 它落的是**结论**，与 `charlog`（落**数据**）分工明确；
/// **为什么结论不落进 `polish.toml`** 见模块头（两条理由，任一条都够否掉它）。
pub mod karaoke;
pub mod srt;
pub mod text;
