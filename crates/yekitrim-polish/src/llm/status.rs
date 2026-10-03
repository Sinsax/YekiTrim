//! LLM 生成结果的**状态快照**：哪一段被成功处理了、哪一段没产出事件（以及为什么）。
//!
//! 为什么要有它：完整剪辑是「一窗一段」的并发调用，任何一窗都可能因为 429 / 超时 / 解析失败
//! 而产不出事件。失败以前**完全不可见** —— 日志里一行 `窗口 seg_4 失败`，
//! `apply_outputs` 跳过它就完事，用户看到的是「某一段 20 秒没有任何事件」，
//! 而界面上没有任何提示（2026-09-20 用户报的就是这个）。
//!
//! 现在每次 LLM 生成都会把逐段结果写成一份快照（成功/失败 + 原因 + 条数 + 时间区间），
//! 编辑器读它给分段上色（成功 = 当前颜色，失败 = 警示色 + tooltip 写原因）。
//!
//! 落点：`<project>/.yekitrim/polish_events.status.json`（与 `polish_events.checkpoint.json` 同处）。
//! 它只是"最近一次生成的结果"，**不是**渲染/后处理的输入 —— 读失败一律当没有。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 快照文件名（放在工程的 `.yekitrim/` 下）。
pub const STATUS_FILE: &str = "polish_events.status.json";

/// 单段的结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegmentGenStatus {
    /// 段序号（`polish.toml` 里 `segments` 的下标）
    pub index: usize,
    /// 段的时间区间（写进去便于人看与自检：序号在重新分段后会变，时间不会）
    pub start: f64,
    pub end: f64,
    /// 这一段是否产出了事件（空数组与调用失败都算 false）
    pub ok: bool,
    /// 没产出时的原因；`ok = true` 时是空串
    #[serde(default)]
    pub reason: String,
    /// 该段最终事件数
    #[serde(default)]
    pub events: usize,
}

impl SegmentGenStatus {
    pub fn ok(index: usize, start: f64, end: f64, events: usize) -> Self {
        Self { index, start, end, ok: true, reason: String::new(), events }
    }
    pub fn failed(index: usize, start: f64, end: f64, reason: impl Into<String>, events: usize) -> Self {
        Self { index, start, end, ok: false, reason: reason.into(), events }
    }
}

/// 一份快照：最近一次生成的时间 + 逐段结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LlmRunStatus {
    /// unix 秒
    pub ts: i64,
    /// `full` = 整片重做（全部段）；`segment` = 只重做了某一段
    pub mode: String,
    /// 本次涉及的段数
    pub total: usize,
    /// 其中产出事件的段数
    pub ok: usize,
    pub segments: Vec<SegmentGenStatus>,
}

impl LlmRunStatus {
    /// 整片重做的快照。
    pub fn from_full(segments: Vec<SegmentGenStatus>) -> Self {
        let ok = segments.iter().filter(|s| s.ok).count();
        Self {
            ts: chrono::Utc::now().timestamp(),
            mode: "full".into(),
            total: segments.len(),
            ok,
            segments,
        }
    }

    /// 只重做某一段：把该段结果换掉（没有就插到按 index 排序的位置），其余段保持不变。
    ///
    /// ⚠️ **不能**把旧的其它段一起清掉：用户可能是在整片跑完之后单点重做某一段，
    /// 那一段之外的成功/失败记录仍然是有效的（界面上还在给它们上色）。
    pub fn upsert_segment(&mut self, seg: SegmentGenStatus) {
        match self.segments.iter_mut().find(|s| s.index == seg.index) {
            Some(slot) => *slot = seg,
            None => {
                self.segments.push(seg);
                self.segments.sort_by_key(|s| s.index);
            }
        }
        self.ts = chrono::Utc::now().timestamp();
        self.mode = "segment".into();
        self.total = self.segments.len();
        self.ok = self.segments.iter().filter(|s| s.ok).count();
    }
}

/// 把「按 LLM 分段算出来的窗口结果」映射到**最终落盘的 segments**。
///
/// 为什么需要：`cmd_llm` 拿到窗口结果之后还会砍掉超出视频时长的段
/// （`clamp_segments_to_duration`）—— 于是「第 4 个窗口」与「polish.toml 里第 4 段」不再是同一段，
/// 界面就会给错段上色。（屏蔽区间**不再**改段的构成：它只做标记，出片时剪掉、事件原样留着 ——
/// 见 `project/AGENTS.md` 的"屏蔽只做标记"。）
///
/// 映射规则：每段取**时间重叠最多**的那个窗口（切开的子段因此继承整段的成败），
/// 一个窗口的成败**与最终事件条数无关** —— 空事件段会被 process 补上默认运镜，
/// 拿 `events.is_empty()` 当判据会让失败永远显示成成功。
pub fn remap_to_segments(
    windows: &[SegmentGenStatus],
    segments: &[crate::project::Segment],
) -> Vec<SegmentGenStatus> {
    segments
        .iter()
        .enumerate()
        .map(|(i, seg)| {
            let best = windows
                .iter()
                .map(|w| {
                    let overlap = (w.end.min(seg.end) - w.start.max(seg.start)).max(0.0);
                    (overlap, w)
                })
                .filter(|(o, _)| *o > 0.001)
                .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(_, w)| w);
            match best {
                Some(w) => SegmentGenStatus {
                    index: i,
                    start: seg.start,
                    end: seg.end,
                    ok: w.ok,
                    reason: w.reason.clone(),
                    events: seg.events.len(),
                },
                // 找不到对应窗口（理论上只会在"段是后来手工加的"时出现）：按最终事件数兜底
                None => SegmentGenStatus {
                    index: i,
                    start: seg.start,
                    end: seg.end,
                    ok: !seg.events.is_empty(),
                    reason: String::new(),
                    events: seg.events.len(),
                },
            }
        })
        .collect()
}

/// `<project>/.yekitrim/polish_events.status.json`
pub fn status_path(project: &Path) -> PathBuf {
    project.join(yekitrim_core::path::PROJECT_STATE_DIR_NAME).join(STATUS_FILE)
}

/// 原子写快照（`.yekitrim/` 不存在就建）。
pub fn save(project: &Path, status: &LlmRunStatus) -> anyhow::Result<()> {
    let path = status_path(project);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(status)?;
    crate::project::write_atomic(&path, &json)?;
    Ok(())
}

/// 读快照；文件不存在 / 解析失败一律 `None`（它只是展示用，绝不能因此挡住生成或出片）。
pub fn load(project: &Path) -> Option<LlmRunStatus> {
    let raw = std::fs::read_to_string(status_path(project)).ok()?;
    match serde_json::from_str::<LlmRunStatus>(&raw) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!("polish_events.status.json 解析失败（忽略）: {e}");
            None
        }
    }
}

/// 只重做某段之后合并进快照（读写一步到位）。
pub fn upsert_segment(project: &Path, seg: SegmentGenStatus) -> anyhow::Result<()> {
    let mut status = load(project).unwrap_or_default();
    status.upsert_segment(seg);
    save(project, &status)
}

/// 清掉快照（重新初始化工程 / 全部重做之前调）。
pub fn clear(project: &Path) {
    let _ = std::fs::remove_file(status_path(project));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_snapshot_counts_ok() {
        let s = LlmRunStatus::from_full(vec![
            SegmentGenStatus::ok(0, 0.0, 11.2, 4),
            SegmentGenStatus::failed(1, 12.5, 20.3, "429", 0),
        ]);
        assert_eq!((s.mode.as_str(), s.total, s.ok), ("full", 2, 1));
    }

    #[test]
    fn upsert_replaces_one_segment_and_keeps_the_rest() {
        let mut s = LlmRunStatus::from_full(vec![
            SegmentGenStatus::ok(0, 0.0, 11.2, 4),
            SegmentGenStatus::failed(1, 12.5, 20.3, "429", 0),
        ]);
        s.upsert_segment(SegmentGenStatus::ok(1, 12.5, 20.3, 3));
        assert_eq!(s.mode, "segment");
        assert_eq!(s.ok, 2, "重做成功后 ok 数要跟着变");
        assert_eq!(s.segments.len(), 2, "别的段不能被清掉");
        assert_eq!(s.segments[1].events, 3);
    }

    #[test]
    fn upsert_inserts_and_sorts_when_segment_is_new() {
        let mut s = LlmRunStatus::from_full(vec![SegmentGenStatus::ok(2, 30.0, 40.0, 2)]);
        s.upsert_segment(SegmentGenStatus::ok(0, 0.0, 10.0, 1));
        assert_eq!(s.segments.iter().map(|x| x.index).collect::<Vec<_>>(), vec![0, 2]);
    }

    #[test]
    fn remap_follows_split_segments() {
        use crate::project::Segment;
        // 一个 0~30s 的窗口（成功），被屏蔽区间切成 0~10 / 20~30 两段
        let windows = vec![SegmentGenStatus::ok(0, 0.0, 30.0, 5)];
        let segs = vec![
            Segment { start: 0.0, end: 10.0, fx: "med".into(), note: String::new(), events: vec![], extra: Default::default() },
            Segment { start: 20.0, end: 30.0, fx: "med".into(), note: String::new(), events: vec![], extra: Default::default() },
        ];
        let out = remap_to_segments(&windows, &segs);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|s| s.ok), "切出来的子段继承整段的成败");
        assert_eq!(out[0].index, 0);
        assert_eq!((out[0].start, out[0].end), (0.0, 10.0), "区间要是最终段的区间");
        assert!(out.iter().all(|s| s.events == 0), "成败不看最终事件数（会被默认运镜补上）");
    }

    #[test]
    fn remap_keeps_failure_across_split() {
        use crate::project::Segment;
        let windows = vec![SegmentGenStatus::failed(3, 60.0, 90.0, "429", 0)];
        let segs = vec![Segment { start: 70.0, end: 90.0, fx: "med".into(), note: String::new(), events: vec![], extra: Default::default() }];
        let out = remap_to_segments(&windows, &segs);
        assert!(!out[0].ok, "切/裁之后仍然要能看出这一段没生成成功");
        assert_eq!(out[0].reason, "429");
    }

    #[test]
    fn save_load_roundtrip_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        assert!(load(p).is_none(), "还没有快照时应当是 None");
        let s = LlmRunStatus::from_full(vec![SegmentGenStatus::failed(0, 0.0, 9.0, "调用失败", 0)]);
        save(p, &s).unwrap();
        assert!(status_path(p).exists(), "要落在 .yekitrim/ 下（目录不存在时要自动建）");
        assert_eq!(load(p).as_ref(), Some(&s));
        clear(p);
        assert!(load(p).is_none());
    }
}
