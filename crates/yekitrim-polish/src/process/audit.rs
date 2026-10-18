//! process::audit：**只报告、不阻断**的体检器（规格 §4 B1）。
//!
//! # 为什么要有它
//!
//! process 里那些守卫（配额抽稀 / 吸附 / 降级）到底是"**在修问题**"还是"**在掩盖问题**"，
//! 只有把问题**数出来**才知道 —— 规格 §11 的风险表把这件事写成缓解措施：
//! 「去掉 process 配额后片子变乱 ⇒ **C 批次先只报告**，攒一轮真实片段数据再删」。
//!
//! # 口径
//!
//! * **纯函数 + 零 IO**（audit）—— 判据好写、跑得快、不依赖磁盘；
//! * **只在有 findings 时写盘**（R4：无条件写会让每个存量工程多一个文件，
//!   而"本来就很干净"这个结论由日志承载）；
//! * **只报告、不阻断**：一条 finding 不改任何事件（规格 §1 的硬边界；
//!   改片子的事只能由 LLM（创作）或用户（manual）决定）。

use std::path::Path;

use serde::Serialize;

use crate::project::{Event, ProjectFile};

/// 一个体检项。kind 是**稳定串**（写进 JSON、脚本按它分组，别翻译）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub kind: &'static str,
    /// 问题发生的位置（秒；"全片级"的结论给 0.0）。
    pub at: f64,
    /// 人话说明（**带数字** —— 只写"有问题"等于没判据）。
    pub detail: String,
}

// 检查项名（写盘的 JSON 与判据都按这几个常量匹配）
pub const CAMERA_OVERLAP: &str = "camera_overlap";
pub const CAMERA_SAME_TIME: &str = "camera_same_time";
pub const FX_TOO_DENSE: &str = "fx_too_dense";
pub const CAMERA_RATE: &str = "camera_rate";
pub const ELEMENT_OUT_OF_SEGMENT: &str = "element_out_of_segment";
pub const ELEMENT_CROSSES_MUTE: &str = "element_crosses_mute";

/// 相邻两条相机的有效区间重叠超过这么久才算"重叠"（容差，同规格 D3）。
pub const CAMERA_OVERLAP_MIN_SECS: f64 = 0.05;
/// 相邻两次瞬时特效的最小间隔（规格 D2 的配额表）。
pub const FX_MIN_GAP_SECS: f64 = 0.8;
/// 整片每分钟允许的运镜条数上限（规格 D2）。
pub const CAMERA_PER_MINUTE_MAX: f64 = 12.0;
/// "同一时刻"的判定容差（秒）。
const SAME_TIME_EPS: f64 = 0.01;
/// 时间比较的容差（秒）：浮点求和 (time + 默认时长) 会差 1 ulp。
const EPS: f64 = 1e-6;

/// 片长：优先 duration，其次"最后一段的结束"（与 process 其它地方同口径）。
pub fn duration_of(cfg: &ProjectFile) -> f64 {
    cfg.duration
        .filter(|d| *d > 0.0)
        .or_else(|| {
            cfg.segments
                .iter()
                .map(|s| s.end)
                .fold(None, |acc: Option<f64>, e| Some(acc.map_or(e, |a| a.max(e))))
        })
        .unwrap_or(0.0)
}

/// 元素类事件（会占一块画面的那些：贴纸 / 文字与贴纸元素 / 覆盖层）。
///
/// ⚠️ **不含运镜与音效**：运镜是"整段状态"、音效没有画面，它们的"越界"不是同一个概念
/// （越界的运镜由"相机重叠 / 同刻多状态"那两条管）。
fn is_element(ev: &Event) -> bool {
    matches!(ev, Event::Sticker { .. } | Event::Element { .. } | Event::Overlay { .. })
}

/// **体检**：把规格 §4 B1 的六项都扫一遍，返回 findings（可能为空）。
pub fn audit(cfg: &ProjectFile) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    let duration = duration_of(cfg);

    // 事件先按时间摊平：段之间时间会重叠（LLM 按窗口生成 + 去静音切段），
    // 所以"相邻"必须按**时间**排，不能按文件顺序（与 close_pip_shots 同一条口径）。
    let mut cameras: Vec<(f64, f64)> = Vec::new(); // (time, end)
    let mut transient: Vec<f64> = Vec::new();
    for seg in &cfg.segments {
        for ev in &seg.events {
            if matches!(ev, Event::Camera { .. }) {
                let end = ev.end_or_default().unwrap_or(ev.get_time());
                cameras.push((ev.get_time(), end));
            }
            // 瞬时特效：类型名 → 上限有限即瞬时（那张表只在 fx 一处）
            let name = super::fx::event_type_name(ev);
            if !name.is_empty() && super::fx::transient_duration_cap(name).is_finite() {
                transient.push(ev.get_time());
            }
        }
    }
    cameras.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    transient.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // ① 相机重叠：前一条的有效区间还没结束，后一条就开始了
    for w in cameras.windows(2) {
        let (prev_t, prev_end) = w[0];
        let next_t = w[1].0;
        if prev_end - next_t > CAMERA_OVERLAP_MIN_SECS {
            out.push(Finding {
                kind: CAMERA_OVERLAP,
                at: next_t,
                detail: format!(
                    "相机 {:.2}s–{:.2}s 与 {:.2}s 起的下一条重叠 {:.2}s（> {:.2}s）",
                    prev_t, prev_end, next_t, prev_end - next_t, CAMERA_OVERLAP_MIN_SECS
                ),
            });
        }
    }
    // ② 同刻多状态：同一时刻 ≥2 条相机（两条同时生效 ⇒ 求值器只能任取一条）
    for w in cameras.windows(2) {
        if (w[1].0 - w[0].0).abs() < SAME_TIME_EPS {
            out.push(Finding {
                kind: CAMERA_SAME_TIME,
                at: w[1].0,
                detail: format!("{:.2}s 处有两条相机同时开始（求值器只能取其一）", w[1].0),
            });
        }
    }
    // ③ 瞬时效过密
    for w in transient.windows(2) {
        let gap = w[1] - w[0];
        if gap < FX_MIN_GAP_SECS {
            out.push(Finding {
                kind: FX_TOO_DENSE,
                at: w[1],
                detail: format!(
                    "瞬时特效 {:.2}s → {:.2}s 间隔 {:.2}s（< {:.2}s）",
                    w[0], w[1], gap, FX_MIN_GAP_SECS
                ),
            });
        }
    }
    // ④ 镜头总量：camera 数 / 片长 × 60 > 12
    if duration > 0.0 {
        let per_min = cameras.len() as f64 / duration * 60.0;
        if per_min > CAMERA_PER_MINUTE_MAX {
            out.push(Finding {
                kind: CAMERA_RATE,
                at: 0.0,
                detail: format!(
                    "运镜 {} 条 / 片长 {:.1}s ⇒ {:.1} 条每分钟（> {:.0}）",
                    cameras.len(), duration, per_min, CAMERA_PER_MINUTE_MAX
                ),
            });
        }
    }
    // ⑤ 元素越界：end 超出所属段、或 time 早于所属段
    // ⑥ 跨静音元素：元素窗口与 mute_ranges 相交（**只预警**：屏蔽是标记、不是删除 —— 撤销后它要能回来）
    for seg in &cfg.segments {
        for ev in &seg.events {
            if !is_element(ev) {
                continue;
            }
            let t = ev.get_time();
            let end = ev.end_or_default().unwrap_or(t);
            if t < seg.start - EPS || end > seg.end + EPS {
                out.push(Finding {
                    kind: ELEMENT_OUT_OF_SEGMENT,
                    at: t,
                    detail: format!(
                        "元素 {:.2}s–{:.2}s 超出所属段 {:.2}s–{:.2}s",
                        t, end, seg.start, seg.end
                    ),
                });
            }
            if let Some(m) = cfg.mute_ranges.iter().find(|r| r.overlaps(t, end)) {
                out.push(Finding {
                    kind: ELEMENT_CROSSES_MUTE,
                    at: t,
                    detail: format!(
                        "元素 {:.2}s–{:.2}s 跨进屏蔽区间 {:.2}s–{:.2}s（出片会被剪掉一截）",
                        t, end, m.start, m.end
                    ),
                });
            }
        }
    }
    out
}

/// 报告的 counts 段（给人看 / 给脚本分组；不是判据）。纯函数。
pub fn counts(cfg: &ProjectFile) -> serde_json::Value {
    let mut cameras = 0usize;
    let mut transient = 0usize;
    let mut elements = 0usize;
    for seg in &cfg.segments {
        for ev in &seg.events {
            if matches!(ev, Event::Camera { .. }) {
                cameras += 1;
            }
            let name = super::fx::event_type_name(ev);
            if !name.is_empty() && super::fx::transient_duration_cap(name).is_finite() {
                transient += 1;
            }
            if is_element(ev) {
                elements += 1;
            }
        }
    }
    serde_json::json!({
        "duration": duration_of(cfg),
        "segments": cfg.segments.len(),
        "cameras": cameras,
        "transient_fx": transient,
        "elements": elements,
        "mute_ranges": cfg.mute_ranges.len(),
    })
}

/// 报告正文（findings + counts）。纯函数 —— 写盘在 write_report。
pub fn report_json(cfg: &ProjectFile, findings: &[Finding]) -> String {
    let body = serde_json::json!({ "findings": findings, "counts": counts(cfg) });
    serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".to_string())
}

/// 落 data/process-report.json。**调用方只在有 findings 时叫它**（R4）。
///
/// 写盘失败只告警、不阻断 —— 与 write_pacing_report / write_punch_report 同一条口径：
/// 一份报告不该让整条后处理失败。
pub fn write_report(data_dir: &Path, cfg: &ProjectFile, findings: &[Finding]) -> bool {
    let path = data_dir.join("process-report.json");
    match std::fs::write(&path, report_json(cfg, findings)) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("  ⚠ 体检报告写盘失败（{e}）：{}", path.display());
            false
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{MuteRange, Segment};

    fn seg(start: f64, end: f64, events: Vec<Event>) -> Segment {
        Segment {
            start,
            end,
            fx: "med".into(),
            note: String::new(),
            role: String::new(),
            events,
            extra: Default::default(),
        }
    }
    fn proj(duration: Option<f64>, segments: Vec<Segment>) -> ProjectFile {
        let mut c = ProjectFile::example();
        c.duration = duration;
        c.segments = segments;
        c.mute_ranges.clear();
        c
    }
    fn camera(t: f64, end: Option<f64>) -> Event {
        Event::Camera {
            time: t,
            end,
            zoom: "中".into(),
            origin: "50% 50%".into(),
            anchor: None,
            anchor_to: None,
            to_origin: None,
            cut: false,
            handheld: false,
            manual: false,
            speed: None,
            tilt: None,
        }
    }
    fn flash(t: f64) -> Event {
        Event::Flash { time: t, color: None, end: Some(t + 0.25), anchor: None, sfx: None, sfx_volume: None }
    }
    fn overlay(t: f64, end: f64) -> Event {
        Event::Overlay {
            time: t,
            end: Some(end),
            overlay_type: "fade_in".into(),
            color: Some("#000000".into()),
            opacity: Some(1.0),
            colors: None,
            shape: None,
            angle: None,
            sfx: None,
            sfx_volume: None,
            anchor: None,
        }
    }
    fn kinds(f: &[Finding]) -> Vec<&'static str> {
        f.iter().map(|x| x.kind).collect()
    }

    /// **干净工程 ⇒ 0 findings** —— 防判据空转（每条都红的话"体检"就成了噪音）。
    #[test]
    fn clean_project_has_no_findings() {
        // 每条相机在自己段内、不重叠；不插瞬时特效；元素都在段内且不碰屏蔽
        let cfg = proj(
            Some(20.0),
            vec![
                seg(0.0, 10.0, vec![camera(0.0, Some(9.5)), overlay(1.0, 2.0)]),
                seg(10.0, 20.0, vec![camera(10.0, Some(19.5))]),
            ],
        );
        let f = audit(&cfg);
        assert!(f.is_empty(), "干净工程不该有 findings，实得 {:?}", kinds(&f));
        // counts 是"给人看的数字"，顺手钉一下它不是空壳
        let c = counts(&cfg);
        assert_eq!(c["cameras"], 2);
        assert_eq!(c["elements"], 1);
        assert_eq!(c["duration"], 20.0);
    }

    /// ① 相机重叠：前一条还没结束、后一条就开始了。
    #[test]
    fn camera_overlap_is_reported() {
        let cfg = proj(Some(20.0), vec![seg(0.0, 10.0, vec![camera(0.0, Some(6.0)), camera(5.0, Some(9.0))])]);
        let f = audit(&cfg);
        assert_eq!(kinds(&f), vec![CAMERA_OVERLAP], "实得 {:?}", f);
        assert!((f[0].at - 5.0).abs() < 1e-9);
        assert!(f[0].detail.contains("重叠"), "{}", f[0].detail);
    }

    /// ② 同刻多状态：两条相机同一时刻开始。
    #[test]
    fn camera_same_time_is_reported() {
        let cfg = proj(Some(20.0), vec![seg(0.0, 10.0, vec![camera(3.0, Some(4.0)), camera(3.0, Some(8.0))])]);
        let f = audit(&cfg);
        assert!(kinds(&f).contains(&CAMERA_SAME_TIME), "实得 {:?}", f);
    }

    /// ③ 瞬时效过密：间隔 < 0.8s。
    #[test]
    fn transient_fx_density_is_reported() {
        let cfg = proj(Some(20.0), vec![seg(0.0, 10.0, vec![flash(2.0), flash(2.5)])]);
        let f = audit(&cfg);
        let dense: Vec<&Finding> = f.iter().filter(|x| x.kind == FX_TOO_DENSE).collect();
        assert_eq!(dense.len(), 1, "实得 {:?}", f);
        assert!((dense[0].at - 2.5).abs() < 1e-9);
        // 间隔够开 ⇒ 不该报
        let ok = proj(Some(20.0), vec![seg(0.0, 10.0, vec![flash(2.0), flash(3.0)])]);
        assert!(!kinds(&audit(&ok)).contains(&FX_TOO_DENSE));
    }

    /// ④ 镜头总量：> 12 条每分钟。
    #[test]
    fn camera_rate_is_reported() {
        // 6s 里 3 条 ⇒ 30 条/分钟
        let cfg = proj(Some(6.0), vec![seg(0.0, 6.0, vec![camera(0.0, Some(1.0)), camera(2.0, Some(3.0)), camera(4.0, Some(5.0))])]);
        let f = audit(&cfg);
        assert!(kinds(&f).contains(&CAMERA_RATE), "实得 {:?}", f);
        // 60s 里 3 条 ⇒ 3 条/分钟，不报
        let ok = proj(Some(60.0), vec![seg(0.0, 60.0, vec![camera(0.0, Some(1.0)), camera(20.0, Some(21.0)), camera(40.0, Some(41.0))])]);
        assert!(!kinds(&audit(&ok)).contains(&CAMERA_RATE));
    }

    /// ⑤ 元素越界：end 超出所属段（time 早于段起点同理）。
    #[test]
    fn element_out_of_segment_is_reported() {
        let cfg = proj(Some(20.0), vec![seg(0.0, 5.0, vec![overlay(1.0, 7.0)])]);
        let f = audit(&cfg);
        assert!(kinds(&f).contains(&ELEMENT_OUT_OF_SEGMENT), "实得 {:?}", f);
        let early = proj(Some(20.0), vec![seg(5.0, 10.0, vec![overlay(4.0, 6.0)])]);
        assert!(kinds(&audit(&early)).contains(&ELEMENT_OUT_OF_SEGMENT));
    }

    /// ⑥ 跨静音元素：**只预警**（屏蔽是标记不是删除，撤销后它要能回来）。
    #[test]
    fn element_crosses_mute_is_reported() {
        let mut cfg = proj(Some(20.0), vec![seg(0.0, 10.0, vec![overlay(4.0, 6.5)])]);
        cfg.mute_ranges = vec![MuteRange { start: 6.0, end: 8.0 }];
        let f = audit(&cfg);
        assert!(kinds(&f).contains(&ELEMENT_CROSSES_MUTE), "实得 {:?}", f);
        // 真的只报告：事件本身一个字都没被改
        match &cfg.segments[0].events[0] {
            Event::Overlay { time, end, .. } => {
                assert_eq!((*time, *end), (4.0, Some(6.5)), "体检不许改事件");
            }
            other => panic!("还是那条 overlay：{other:?}"),
        }
    }

    /// 报告正文的形状（写盘那条路的判据：字段名与 counts 都在）。
    #[test]
    fn report_json_has_findings_and_counts() {
        let cfg = proj(Some(20.0), vec![seg(0.0, 10.0, vec![camera(0.0, Some(6.0)), camera(5.0, Some(9.0))])]);
        let f = audit(&cfg);
        assert!(!f.is_empty());
        let text = report_json(&cfg, &f);
        let v: serde_json::Value = serde_json::from_str(&text).expect("报告必须是合法 JSON");
        assert!(v["findings"].is_array());
        assert_eq!(v["counts"]["cameras"], 2);
        assert!(text.contains(CAMERA_OVERLAP));
    }
}
