//! 节奏调度（§26）：把「爆点 + 空档」排成节拍网格上的**重音表**，再落实成事件。
//!
//! 入口：`params` / `plan` / `apply` / `clear` / `metrics` / `report_json`。
//! 规格与用户口径见 `docs/plan/rhythm-richness-plan.md`（2026-10-19「整体升一档」）。
//!
//! # 为什么是「调度」而不是「更多特效」
//!
//! 后处理此前只有**总量配额**（贴纸每 5 条字幕、音效每 8 条…），没有「什么时候该给一下」。
//! 于是观感是「一直在动」：每段都默认同档运镜，反而没有对比；而 `pacing.rs` 那条
//! 「每 3–6 秒一次视觉变化」的纪律因为把静态运镜当成持续变化，**从未真正生效**。
//!
//! # 三条硬纪律
//!
//! 1. **只写派生事件**：所有生成的事件都登记进 `polish.toml` 的 `[rhythm] auto_events` 账本；
//!    每趟先按账本逐条删除（字段逐值匹配才删），再按新计划重建 ⇒ **幂等**。
//!    用户改过的事件字段变了 ⇒ 认不出 ⇒ 不删，那一条就归用户。
//! 2. **`manual` 一律不碰**，且触发点要避开它们（`avoid_secs`）。
//! 3. **屏蔽区间里不生成重音**（与 process 其余守卫同一条：屏蔽只做标记，生成侧不产出）。
//!
//! # 手法（2026-10-19 用户二次口径）
//!
//! 「呼吸感运镜应该作为一个预设，只应用于一次事件，不要频繁出现，有点刻意了」⇒
//! **每一档的重音不是「推近再回来」**：
//!
//! | 预设 | 画面 | 用在哪 |
//! |---|---|---|
//! | `Punch`（轻） | 只抖一下，**不动镜头** | 空档补位、普通节拍 |
//! | `Hold`（中） | 换到更近一档并**保持**，不回来 | 爆点、情绪抬升 |
//! | `Cut`（重） | 硬切到特写 + 闪白，保持 | 强爆点（受 `heavy_every` 约束）|
//! | `Breathe`（呼吸） | 推近 1.2s → 回位 0.32s | **整片最多 `breath_max` 次**（默认 1，0 = 不用）|
//! | `Push`（慢推） | `slow` 过渡推到近景并保持 | 只给 ≥8s 的长段，每段 1 次、整片 `push_max` |
//! | `Pan`（甩视线） | 聚焦甩到弹幕 1.2s → 甩回主播 1.0s | 爆点里的「中」，整片 `pan_max`（特写档不做）|
//! | `Dutch`（歪镜） | 歪 5° 保持 2s → 回正 | 强爆点，整片 `dutch_max`，还与 `cap_dutch_angle` 的 2 条取小 |
//! | `Impact`（重击） | 重抖(24) + 白闪，**不换机位** | 强爆点，整片 `impact_max` |
//!
//! 只有当重音**真的换了档位**时才插运镜（同档连续重音只叠抖动）—— 否则每 3.6s 做一次
//! 同样的小动作，看着就是编排痕迹。
//!
//! # 档位（2026-10-19 用户口径：整体升一档）
//!
//! | 档 | 重音密度目标 | 节拍 | 重音之间 | 特点 |
//! |---|---|---|---|---|
//! | `calm` | 0（今天的行为） | — | — | 只清账本，不生成 |
//! | `balanced`（默认） | 13~16 / 分钟 | 3.6s | ≥ 10s | 轻 45% / 中 35% / 重 20% |
//! | `intense` | 18~22 / 分钟 | 3.0s | ≥ 7s | 轻 40% / 中 35% / 重 25% |

use crate::project::{Event, MuteRange, ProjectFile, RhythmAuto, Segment};

/// `calm`：**今天的行为**（不生成任何重音，只把上一趟的账本收干净）。
pub const PROFILE_CALM: &str = "calm";
/// 默认档（用户口径「整体升一档」）。
pub const PROFILE_BALANCED: &str = "balanced";
/// 更密的一档（节拍 3.0s、重音之间 7s）。
pub const PROFILE_INTENSE: &str = "intense";

/// 一档节奏参数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub enabled: bool,
    /// 节拍间隔（秒）—— 网格步长。
    pub beat_secs: f64,
    /// 相邻两个重音的最小间隔（秒）。
    pub min_gap_secs: f64,
    /// 允许的最长「没有重音」的空档（秒）—— 超过就必须补一个「轻」。
    pub max_gap_secs: f64,
    /// 两个**重**音之间至少隔多久（秒）。
    pub heavy_every_secs: f64,
    /// 每段最多几个重音。
    pub per_segment_max: usize,
    /// 短于这个长度的段不调度（秒）。
    pub min_segment_secs: f64,
    /// 爆点的影响半径：落在 ±它之内的格点算「爆点重音」（秒）。
    pub punch_reach_secs: f64,
    /// 已有事件（贴纸/元素/瞬时特效/手工运镜）周围这么久之内不插重音（秒）。
    pub avoid_secs: f64,
    /// **「呼吸」预设整片最多几次**（推近 → 回位那种圆舞曲）。
    ///
    /// 用户口径（2026-10-19）：「呼吸感运镜应该作为一个预设，只应用于一次事件，不要频繁出现，
    /// 有点刻意了」—— 它**不是每一档的模板**，而是一次性手法（默认整片 1 次）；`0` = 不用。
    pub breath_max: usize,
    /// 慢推（`slow`）整片最多几次。
    pub push_max: usize,
    /// 甩视线（把聚焦甩到弹幕再甩回来）整片最多几次。
    pub pan_max: usize,
    /// 歪镜（荷兰角）整片最多几次；还要与 `cap_dutch_angle` 的全片配额取小。
    pub dutch_max: usize,
    /// 重击（重抖 + 白闪、**不换机位**）整片最多几次。
    pub impact_max: usize,
}

/// 歪镜用的角度（度）。必须 ≤ `fx::MAX_DUTCH_ANGLE`（6°）—— 超过像拍摄事故。
///
/// 取 5°：4° 在大屏上几乎看不出，6° 已经到「歪了」的边上；两侧都要留一点余量。
pub const DUTCH_DEG: f64 = 5.0;

/// 「慢推」只给段长 ≥ 它的段（秒）：`slow` 的过渡上限是 1.5s，短段里还没推完就该切了。
pub const MIN_PUSH_SEGMENT_SECS: f64 = 8.0;

impl Params {
    /// 今天的行为：不生成、只清账本。
    pub const CALM: Self = Self {
        enabled: false,
        beat_secs: 0.0,
        min_gap_secs: 0.0,
        max_gap_secs: 0.0,
        heavy_every_secs: 0.0,
        per_segment_max: 0,
        min_segment_secs: 0.0,
        punch_reach_secs: 0.0,
        avoid_secs: 0.0,
        breath_max: 0,
        push_max: 0,
        pan_max: 0,
        dutch_max: 0,
        impact_max: 0,
    };

/// 默认档（= 最初提案再高一档：节拍 3.6s、重音之间 10s）。
    pub const BALANCED: Self = Self {
        enabled: true,
        beat_secs: 3.6,
        min_gap_secs: 2.8,
        max_gap_secs: 5.0,
        heavy_every_secs: 10.0,
        per_segment_max: 2,
        min_segment_secs: 1.2,
        punch_reach_secs: 1.5,
        avoid_secs: 0.7,
        breath_max: 1,
        push_max: 2,
        pan_max: 1,
        dutch_max: 1,
        impact_max: 1,
    };

    /// 更密的一档。
    pub const INTENSE: Self = Self {
        enabled: true,
        beat_secs: 3.0,
        min_gap_secs: 2.2,
        max_gap_secs: 4.0,
        heavy_every_secs: 7.0,
        per_segment_max: 3,
        min_segment_secs: 1.2,
        punch_reach_secs: 1.6,
        avoid_secs: 0.6,
        breath_max: 1,
        push_max: 3,
        pan_max: 2,
        dutch_max: 1,
        impact_max: 2,
    };

    /// 按档位名取参数（认不出 ⇒ 默认档，**不静默关闭**）。
    pub fn for_profile(profile: &str) -> Self {
        match profile.trim().to_ascii_lowercase().as_str() {
            PROFILE_CALM => Self::CALM,
            PROFILE_INTENSE => Self::INTENSE,
            _ => Self::BALANCED,
        }
    }
}

/// 从「档位名 + 可选节拍」算参数（`llm` 那条腿拿不到整个 `ProjectFile`，用这一支）。
pub fn params_for(profile: &str, beat_secs: Option<f64>) -> Params {
    let mut p = Params::for_profile(profile);
    if let Some(b) = beat_secs {
        if b.is_finite() && b >= 1.0 {
            p.beat_secs = b;
        }
    }
    p
}

/// 从工程配置读参数（`[rhythm] beat_secs` / `breath_max` 可覆盖）。
pub fn params(cfg: &ProjectFile) -> Params {
    let mut p = params_for(cfg.rhythm.profile_name(), cfg.rhythm.beat_secs);
    if let Some(b) = cfg.rhythm.breath_max {
        p.breath_max = b;
    }
    p
}

/// 重音强度：三档模板（轻 = 只推近；中 = 推近 + 抖；重 = 硬切 + 闪白）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strength {
    Light,
    Medium,
    Heavy,
}

impl Strength {
    pub fn label(self) -> &'static str {
        match self {
            Strength::Light => "轻",
            Strength::Medium => "中",
            Strength::Heavy => "重",
        }
    }
}

/// **一次重音的「手法」**（用户 2026-10-19：呼吸是预设、整片一次，别每下都呼吸）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// 只抖一下，**不动镜头**（「轻」的默认手法）—— 画面在变、但不换机位。
    Punch,
    /// 换到更近一档并**保持**（不回来）—— 下一个重音再决定去哪。
    Hold,
    /// 硬切到特写并保持（「重」的默认手法）。
    Cut,
    /// **「呼吸」预设**：推近 → 回位。整片最多 `breath_max` 次（默认 1）。
    Breathe,
    /// **慢推**：`slow` 过渡推到近景并保持 —— 和 `Hold` 的「快推」是两种语气。
    Push,
    /// **甩视线**：把聚焦从主播甩到弹幕、再甩回来（一次「看观众」）。
    Pan,
    /// **歪镜**：荷兰角 5° 保持 2s 再回正（强情绪，全片 1 次左右）。
    Dutch,
    /// **重击**：重抖 + 白闪，**不换机位**（比 `Cut` 轻、比 `Hold` 狠）。
    Impact,
}

impl Preset {
    pub fn label(self) -> &'static str {
        match self {
            Preset::Punch => "抖动",
            Preset::Hold => "推近保持",
            Preset::Cut => "硬切",
            Preset::Breathe => "呼吸",
            Preset::Push => "慢推",
            Preset::Pan => "甩视线",
            Preset::Dutch => "歪镜",
            Preset::Impact => "重击",
        }
    }
}

impl Strength {
    /// 默认手法（`Breathe` 不在这里 —— 它是调度器单独指派的一次性预设）。
    pub fn preset(self) -> Preset {
        match self {
            Strength::Light => Preset::Punch,
            Strength::Medium => Preset::Hold,
            Strength::Heavy => Preset::Cut,
        }
    }
}

/// 一个重音（**成片时间轴**上的时刻；本仓不做自动切，段表就是源时间）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accent {
    pub at: f64,
    pub strength: Strength,
    /// 这一下用哪种手法（默认由强度决定；`Breathe` 由调度器指派、整片一次）。
    pub preset: Preset,
    /// 这一下是不是为了「爆点」而给（报告与命中率用）。
    pub on_punch: bool,
}

/// 已有事件占住的区间（重音要避让的东西）。
fn blocked_spans(segments: &[Segment]) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::new();
    for seg in segments {
        for ev in &seg.events {
            let blocks = match ev {
                Event::Sticker { .. }
                | Event::Element { .. }
                | Event::Shake { .. }
                | Event::Flash { .. }
                | Event::Blur { .. }
                | Event::Noise { .. }
                | Event::Pulse { .. }
                | Event::HueShift { .. }
                | Event::Vignette { .. }
                | Event::Overlay { .. } => true,
                Event::Camera { manual, .. } => *manual,
                Event::Sfx { .. } => false,
            };
            if !blocks {
                continue;
            }
            let s = ev.get_time();
            let e = ev.end().unwrap_or(s + 1.0).max(s + 0.3);
            out.push((s, e));
        }
    }
    out
}

/// 纯规划：网格 → 候选 → 配额 → 空档补位（不改任何事件）。
///
/// `punches` = `(at, score)`，来自 `process::punch`（字幕 + 弹幕 + 响度；无弹幕自动降级）。
pub fn plan(segments: &[Segment], punches: &[(f64, f64)], params: Params, mutes: &[MuteRange]) -> Vec<Accent> {
    if !params.enabled || params.beat_secs <= 0.0 {
        return Vec::new();
    }
    let blocked = blocked_spans(segments);
    let muted = |t: f64| mutes.iter().any(|m| m.contains(t));
    let is_blocked =
        |t: f64| blocked.iter().any(|(s, e)| t >= *s - params.avoid_secs && t <= *e + params.avoid_secs);

    // 1) 网格：每段从 start+0.6 起按 beat 走（段首 0.6s 通常是在交代边界，不抢）。
    let mut slots: Vec<f64> = Vec::new();
    for seg in segments {
        if seg.end - seg.start < params.min_segment_secs {
            continue;
        }
        let mut t = seg.start + 0.6;
        while t < seg.end - 0.4 {
            if !muted(t) && !is_blocked(t) {
                slots.push(t);
            }
            t += params.beat_secs;
        }
    }
    if slots.is_empty() {
        return Vec::new();
    }

    // 2) 候选：爆点格点（分高、强度中/重）+ 其余格点（分低、强度轻）。
    struct Cand {
        at: f64,
        seg: usize,
        score: f64,
        strength: Strength,
        on_punch: bool,
    }
    let mut cands: Vec<Cand> = Vec::new();
    for t in slots.iter().copied() {
        let si = segment_index_for(segments, t).unwrap_or(0);
        let mut best: Option<(f64, f64)> = None; // (distance, score)
        for (pat, score) in punches.iter().copied() {
            let d = (pat - t).abs();
            if d <= params.punch_reach_secs {
                let better = match best {
                    None => true,
                    Some((bd, bs)) => score > bs || (score == bs && d < bd),
                };
                if better {
                    best = Some((d, score));
                }
            }
        }
        match best {
            Some((d, score)) => {
                let near = 1.0 - (d / params.punch_reach_secs).clamp(0.0, 1.0);
                let strength = if score >= 0.75 && near >= 0.6 {
                    Strength::Heavy
                } else {
                    Strength::Medium
                };
                cands.push(Cand { at: t, seg: si, score: 2.0 + score + near, strength, on_punch: true });
            }
            None => cands.push(Cand { at: t, seg: si, score: 1.0, strength: Strength::Light, on_punch: false }),
        }
    }

    // 3) 配额分配：分高的先挑；间隔不够就让位；重的太密降成「中」（不是丢掉）。
    cands.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    let mut picked: Vec<(f64, Strength, bool)> = Vec::new();
    let mut heavy_at: Vec<f64> = Vec::new();
    let mut per_seg: Vec<usize> = vec![0; segments.len()];
    for c in cands {
        if per_seg.get(c.seg).copied().unwrap_or(0) >= params.per_segment_max {
            continue;
        }
        if picked.iter().any(|(t, _, _)| (t - c.at).abs() < params.min_gap_secs) {
            continue;
        }
        let mut strength = c.strength;
        if strength == Strength::Heavy && heavy_at.iter().any(|t| (t - c.at).abs() < params.heavy_every_secs) {
            strength = Strength::Medium;
        }
        if strength == Strength::Heavy {
            heavy_at.push(c.at);
        }
        picked.push((c.at, strength, c.on_punch));
        per_seg[c.seg] += 1;
    }
    picked.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    // 4) **空档优先**：> max_gap 的区间补一个「轻」（挑离目标点最近、且不与已选重音相撞的格点）。
    //
    // ⚠️ **必须循环到收敛**：一次只能填一个空档，而填出来的重音会把一个大空档劈成两个
    //    新的空档（20s 段只填 1 个仍然剩 12s 空档）。上限 64 次是防呆 —— 真到那一步说明
    //    网格比 max_gap 还稀（`beat_secs` 被配大了），再填也没用。
    let total_end = segments.iter().map(|s| s.end).fold(0.0_f64, f64::max);
    let mut extra: Vec<(f64, Strength, bool)> = Vec::new();
    for _ in 0..64 {
        let mut all: Vec<(f64, Strength, bool)> = picked.iter().chain(extra.iter()).copied().collect();
        all.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut gaps: Vec<(f64, f64)> = Vec::new();
        let mut cursor = 0.0_f64;
        for (at, _, _) in all.iter() {
            if at - cursor > params.max_gap_secs {
                gaps.push((cursor, *at));
            }
            cursor = cursor.max(*at);
        }
        if total_end - cursor > params.max_gap_secs {
            gaps.push((cursor, total_end));
        }
        if gaps.is_empty() {
            break;
        }
        let mut added = false;
        for (gs, ge) in gaps {
            let target = gs + params.max_gap_secs * 0.75;
            let best = slots
                .iter()
                .copied()
                .filter(|t| *t > gs + 0.25 && *t < ge - 0.25)
                .filter(|t| all.iter().all(|(a, _, _)| (a - t).abs() >= params.min_gap_secs))
                .min_by(|x, y| {
                    (x - target).abs().partial_cmp(&(y - target).abs()).unwrap_or(std::cmp::Ordering::Equal)
                });
            if let Some(t) = best {
                extra.push((t, Strength::Light, false));
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    let mut out: Vec<(f64, Strength, bool)> = picked;
    out.extend(extra);
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut accents: Vec<Accent> = out
        .into_iter()
        .map(|(at, strength, on_punch)| Accent { at, strength, preset: strength.preset(), on_punch })
        .collect();

    // **「呼吸」是一整片一次的手法**（用户口径），不是每一档的模板；而且只挂在
    // **中/重**上 —— 它是「镜头动了一下再回来」，给「轻」（本来就不动机位）等于凭空加戏。
    // 挑「最值得」的那一下：爆点里的中/重 > 第一个中/重。
    if params.breath_max > 0 && !accents.is_empty() {
        let order: Vec<usize> = accents
            .iter()
            .enumerate()
            .filter(|(_, a)| a.on_punch && a.strength != Strength::Light)
            .map(|(i, _)| i)
            .chain(
                accents
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| a.strength != Strength::Light)
                    .map(|(i, _)| i),
            )
            .collect();
        let mut used = 0usize;
        let mut seen: Vec<usize> = Vec::new();
        for i in order {
            if used >= params.breath_max {
                break;
            }
            if seen.contains(&i) {
                continue;
            }
            seen.push(i);
            accents[i].preset = Preset::Breathe;
            used += 1;
        }
    }
    // 其余手法都是**例外**：各自有整片上限，只从对应强度里挑，挑不到就不加（绝不为凑数而用）。
    let mut pan_left = params.pan_max;
    for a in accents.iter_mut() {
        if pan_left == 0 {
            break;
        }
        if a.preset == Preset::Hold && a.on_punch {
            a.preset = Preset::Pan;
            pan_left -= 1;
        }
    }

    // 歪镜还要与 `cap_dutch_angle` 的全片配额取小（那一步排在节奏之前，本趟数不到我们新加的）。
    let existing_tilt = segments
        .iter()
        .flat_map(|s| s.events.iter())
        .filter(|e| matches!(e, Event::Camera { tilt: Some(_), manual: false, .. }))
        .count();
    let mut dutch_left = params
        .dutch_max
        .min(crate::process::fx::MAX_DUTCH_SHOTS.saturating_sub(existing_tilt));
    for a in accents.iter_mut() {
        if dutch_left == 0 {
            break;
        }
        if a.preset == Preset::Cut {
            a.preset = Preset::Dutch;
            dutch_left -= 1;
        }
    }

    let mut impact_left = params.impact_max;
    for a in accents.iter_mut() {
        if impact_left == 0 {
            break;
        }
        if a.preset == Preset::Cut {
            a.preset = Preset::Impact;
            impact_left -= 1;
        }
    }

    // 慢推只给**长段**里的「中」，且每段最多 1 次。
    let mut push_left = params.push_max;
    let mut pushed: Vec<usize> = vec![0; segments.len()];
    for a in accents.iter_mut() {
        if push_left == 0 {
            break;
        }
        if a.preset != Preset::Hold {
            continue;
        }
        let Some(si) = segment_index_for(segments, a.at) else { continue };
        if segments[si].end - segments[si].start < MIN_PUSH_SEGMENT_SECS || pushed[si] > 0 {
            continue;
        }
        pushed[si] += 1;
        a.preset = Preset::Push;
        push_left -= 1;
    }

    accents
}

/// 段下标：`start <= t < end`；落在段外时取最近的一段。
fn segment_index_for(segments: &[Segment], t: f64) -> Option<usize> {
    if segments.is_empty() {
        return None;
    }
    segments
        .iter()
        .position(|s| t >= s.start && t < s.end)
        .or_else(|| {
            segments
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    ((a.start + a.end) / 2.0 - t)
                        .abs()
                        .partial_cmp(&((b.start + b.end) / 2.0 - t).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(i, _)| i)
        })
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

/// 账本条目与事件的**逐值匹配**（用户改过任一个字段 ⇒ 认不出 ⇒ 不删）。
fn matches_auto(ev: &Event, a: &RhythmAuto) -> bool {
    match (a.kind.as_str(), ev) {
        ("camera", Event::Camera { time, end, zoom, cut, tilt, .. }) => {
            near(*time, a.time)
                && a.zoom.as_deref() == Some(zoom.as_str())
                && match (a.end, end) {
                    (Some(x), Some(y)) => near(x, *y),
                    (None, None) => true,
                    _ => false,
                }
                && *cut == a.param.is_some()
                && match (a.tilt, tilt) {
                    (Some(x), Some(y)) => near(x, *y),
                    (None, None) => true,
                    _ => false,
                }
        }
        ("shake", Event::Shake { time, intensity, .. }) => {
            near(*time, a.time) && a.param.is_some_and(|p| near(p, *intensity))
        }
        ("flash", Event::Flash { time, .. }) => near(*time, a.time),
        _ => false,
    }
}

/// **按账本删除**上一趟的派生事件（返回删了几条）。
pub fn clear(segments: &mut [Segment], ledger: &[RhythmAuto]) -> usize {
    if ledger.is_empty() {
        return 0;
    }
    let mut removed = 0usize;
    for seg in segments.iter_mut() {
        let before = seg.events.len();
        seg.events.retain(|ev| !ledger.iter().any(|a| matches_auto(ev, a)));
        removed += before - seg.events.len();
    }
    removed
}

fn insert_event(seg: &mut Segment, ev: Event) {
    let t = ev.get_time();
    let idx = seg.events.iter().position(|e| e.get_time() > t).unwrap_or(seg.events.len());
    seg.events.insert(idx, ev);
}

fn camera(time: f64, end: Option<f64>, zoom: &str, cut: bool) -> Event {
    camera_full(time, end, zoom, cut, "fast", "liver", None, None)
}

/// 完整版：`anchor` / `anchor_to` / `tilt` / `speed` 都可指定（新手法要用到）。
fn camera_full(
    time: f64,
    end: Option<f64>,
    zoom: &str,
    cut: bool,
    speed: &str,
    anchor: &str,
    anchor_to: Option<&str>,
    tilt: Option<f64>,
) -> Event {
    Event::Camera {
        time,
        end,
        zoom: zoom.to_string(),
        origin: "50% 50%".into(),
        anchor: Some(anchor.into()),
        anchor_to: anchor_to.map(str::to_string),
        to_origin: None,
        cut,
        handheld: false,
        manual: false,
        speed: Some(speed.into()),
        tilt,
    }
}

/// 落实：先按账本清掉上一趟，再按重音表生成（返回生成的事件条数与新账本）。
pub fn apply(segments: &mut Vec<Segment>, accents: &[Accent], old: &[RhythmAuto]) -> (usize, Vec<RhythmAuto>) {
    clear(segments.as_mut_slice(), old);
    let mut ledger: Vec<RhythmAuto> = Vec::new();
    // 当前段 / 当前机位：「保持」型手法**只在档位真的变了**时插一条运镜 —— 否则连续两个
    // 「中」重音会插出两条一样的推近，看着就是反复做同一个小动作（用户说的「有点刻意」）。
    let mut cur_seg: Option<usize> = None;
    let mut cur_zoom: String = "中".into();
    for a in accents {
        let Some(si) = segment_index_for(segments, a.at) else { continue };
        let seg_start = segments[si].start;
        let seg_end = segments[si].end;
        // 换段 ⇒ 段首那条默认运镜把机位复位成「中」（Step 1 补的，整段铺满）。
        if cur_seg != Some(si) {
            cur_seg = Some(si);
            cur_zoom = "中".into();
        }
        let start = a.at.max(seg_start + 0.05);
        // 下一个重音（同段）—— 换机位后的「保持」一直占到它为止。
        let next = accents
            .iter()
            .map(|b| b.at)
            .filter(|t| *t > start && *t <= seg_end)
            .fold(seg_end, f64::min);
        match a.preset {
            // 轻 / 重击：**不动镜头**（画面在变、但不换机位）。
            Preset::Punch | Preset::Impact => {}
            // 单向换档：换到目标档并**保持**到下一个重音 —— 不回来。
            Preset::Hold | Preset::Cut | Preset::Push => {
                let (tier, cut, speed) = match a.preset {
                    Preset::Cut => ("特写", true, "fast"),
                    Preset::Push => ("近", false, "slow"),
                    _ => ("近", false, "fast"),
                };
                let mut end = next.min(seg_end);
                if a.preset == Preset::Push {
                    // `slow` 的过渡最长 1.5s ⇒ 至少要停留到 2.5s 才看得出「推」。
                    end = end.max((start + 2.5).min(seg_end));
                }
                if tier != cur_zoom.as_str() && end - start >= 0.6 {
                    insert_event(
                        &mut segments[si],
                        camera_full(start, Some(end), tier, cut, speed, "liver", None, None),
                    );
                    ledger.push(RhythmAuto {
                        kind: "camera".into(),
                        time: start,
                        end: Some(end),
                        zoom: Some(tier.into()),
                        param: if cut { Some(1.0) } else { None },
                        tilt: None,
                    });
                    cur_zoom = tier.to_string();
                }
            }
            // 「呼吸」：推近 1.2s → 回位 0.32s（整片只指派一次）。
            Preset::Breathe => {
                let mid = (start + 1.2).min(seg_end);
                if cur_zoom != "近" && mid - start >= 0.6 {
                    insert_event(&mut segments[si], camera(start, Some(mid), "近", false));
                    ledger.push(RhythmAuto {
                        kind: "camera".into(),
                        time: start,
                        end: Some(mid),
                        zoom: Some("近".into()),
                        param: None,
                        tilt: None,
                    });
                }
                let r1 = (mid + 0.32).min(seg_end);
                if r1 - mid > 0.12 {
                    insert_event(&mut segments[si], camera(mid, Some(r1), "中", false));
                    ledger.push(RhythmAuto {
                        kind: "camera".into(),
                        time: mid,
                        end: Some(r1),
                        zoom: Some("中".into()),
                        param: None,
                        tilt: None,
                    });
                }
                cur_zoom = "中".into();
            }
            // 「甩视线」：聚焦甩到弹幕、再甩回主播（一次「看观众」）。
            // ⚠️ 特写档多半已经看不到弹幕区（`cap_closeup_anchor` 同一个理由）⇒ 只在 中/近 做。
            Preset::Pan => {
                if cur_zoom != "特写" {
                    let mid = (start + 1.2).min(seg_end);
                    let back = (mid + 1.0).min(seg_end);
                    if mid - start >= 0.5 && back - mid >= 0.4 {
                        let tier = cur_zoom.clone();
                        insert_event(
                            &mut segments[si],
                            camera_full(start, Some(mid), &tier, false, "fast", "liver", Some("danmaku"), None),
                        );
                        ledger.push(RhythmAuto {
                            kind: "camera".into(),
                            time: start,
                            end: Some(mid),
                            zoom: Some(tier.clone()),
                            param: None,
                            tilt: None,
                        });
                        insert_event(
                            &mut segments[si],
                            camera_full(mid, Some(back), &tier, false, "fast", "danmaku", Some("liver"), None),
                        );
                        ledger.push(RhythmAuto {
                            kind: "camera".into(),
                            time: mid,
                            end: Some(back),
                            zoom: Some(tier),
                            param: None,
                            tilt: None,
                        });
                    }
                }
            }
            // 「歪镜」：歪 5° 保持 2s，再回正 —— 回正是**另一条事件**（倾斜是持续状态）。
            Preset::Dutch => {
                let tier = if cur_zoom == "特写" { "特写" } else { "近" };
                let mid = (start + 2.0).min(seg_end);
                let back = (mid + 0.3).min(seg_end);
                if mid - start >= 0.8 {
                    insert_event(
                        &mut segments[si],
                        camera_full(start, Some(mid), tier, false, "fast", "liver", None, Some(DUTCH_DEG)),
                    );
                    ledger.push(RhythmAuto {
                        kind: "camera".into(),
                        time: start,
                        end: Some(mid),
                        zoom: Some(tier.into()),
                        param: None,
                        tilt: Some(DUTCH_DEG),
                    });
                    cur_zoom = tier.into();
                    if back - mid > 0.1 {
                        insert_event(
                            &mut segments[si],
                            camera_full(mid, Some(back), tier, false, "fast", "liver", None, None),
                        );
                        ledger.push(RhythmAuto {
                            kind: "camera".into(),
                            time: mid,
                            end: Some(back),
                            zoom: Some(tier.into()),
                            param: None,
                            tilt: None,
                        });
                    }
                }
            }
        }
        // 瞬时特效：轻/中 = 抖；重 = 闪白；重击 = 重抖 + 白闪；镜头类不叠特效。
        let (intensity, flash) = match a.preset {
            Preset::Punch => (Some(8.0_f64), false),
            Preset::Hold | Preset::Push => (Some(10.0), false),
            Preset::Cut => (None, true),
            Preset::Impact => (Some(24.0), true),
            Preset::Breathe | Preset::Pan | Preset::Dutch => (None, false),
        };
        if let Some(intensity) = intensity {
            let t = start.min((seg_end - 0.35).max(start));
            if t + 0.3 <= seg_end + 0.001 {
                insert_event(
                    &mut segments[si],
                    Event::Shake {
                        time: t,
                        intensity,
                        end: Some(t + 0.3),
                        anchor: None,
                        sfx: None,
                        sfx_volume: None,
                    },
                );
                ledger.push(RhythmAuto {
                    kind: "shake".into(),
                    time: t,
                    end: Some(t + 0.3),
                    zoom: None,
                    param: Some(intensity),
                    tilt: None,
                });
            }
        }
        if flash && start + 0.18 <= seg_end + 0.001 {
            insert_event(
                &mut segments[si],
                Event::Flash {
                    time: start,
                    color: Some("#ffffff".into()),
                    end: Some(start + 0.18),
                    anchor: None,
                    sfx: None,
                    sfx_volume: None,
                },
            );
            ledger.push(RhythmAuto {
                kind: "flash".into(),
                time: start,
                end: Some(start + 0.18),
                zoom: None,
                param: None,
                tilt: None,
            });
        }
    }
    let n = ledger.len();
    (n, ledger)
}

/// 节奏指标（报告与 A/B 用）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub accents: usize,
    pub per_minute: f64,
    pub heavies: usize,
    pub on_punch: usize,
    /// 命中率：重音里为了爆点而给的比例（0~1）。
    pub punch_hit_rate: f64,
    /// 最长「没有重音」的空档（秒）。
    pub max_gap_secs: f64,
}

/// 算指标。`duration` ≤ 0 时按 0 处理（不除零）。
pub fn metrics(accents: &[Accent], duration: f64) -> Metrics {
    let duration = if duration.is_finite() && duration > 0.0 { duration } else { 0.0 };
    let mut sorted: Vec<f64> = accents.iter().map(|a| a.at).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut max_gap = 0.0_f64;
    let mut cursor = 0.0_f64;
    for t in sorted.iter().copied() {
        max_gap = max_gap.max(t - cursor);
        cursor = cursor.max(t);
    }
    if duration > cursor {
        max_gap = max_gap.max(duration - cursor);
    }
    let n = accents.len();
    let on_punch = accents.iter().filter(|a| a.on_punch).count();
    Metrics {
        accents: n,
        per_minute: if duration > 0.0 { n as f64 / (duration / 60.0) } else { 0.0 },
        heavies: accents.iter().filter(|a| a.strength == Strength::Heavy).count(),
        on_punch,
        punch_hit_rate: if n > 0 { on_punch as f64 / n as f64 } else { 0.0 },
        max_gap_secs: max_gap,
    }
}

/// `data/rhythm.json` 的正文（只读报告；**不驱动任何东西**）。
pub fn report_json(accents: &[Accent], metrics: &Metrics, params: &Params) -> String {
    let list: Vec<serde_json::Value> = accents
        .iter()
        .map(|a| {
            serde_json::json!({
                "at": (a.at * 1000.0).round() / 1000.0,
                "strength": a.strength.label(),
                "preset": a.preset.label(),
                "on_punch": a.on_punch,
            })
        })
        .collect();
    serde_json::json!({
        "profile": if params.enabled { "on" } else { "off" },
        "beat_secs": params.beat_secs,
        "min_gap_secs": params.min_gap_secs,
        "max_gap_secs": params.max_gap_secs,
        "heavy_every_secs": params.heavy_every_secs,
        "accents": list,
        "metrics": {
            "accents": metrics.accents,
            "per_minute": (metrics.per_minute * 10.0).round() / 10.0,
            "heavies": metrics.heavies,
            "on_punch": metrics.on_punch,
            "punch_hit_rate": (metrics.punch_hit_rate * 100.0).round() / 100.0,
            "max_gap_secs": (metrics.max_gap_secs * 100.0).round() / 100.0,
        },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64) -> Segment {
        Segment {
            role: String::new(),
            start,
            end,
            fx: "med".into(),
            note: String::new(),
            events: Vec::new(),
            extra: Default::default(),
        }
    }

    fn manual_camera(time: f64, end: f64) -> Event {
        Event::Camera {
            time,
            end: Some(end),
            zoom: "近".into(),
            origin: "50% 50%".into(),
            anchor: None,
            anchor_to: None,
            to_origin: None,
            cut: false,
            handheld: false,
            manual: true,
            speed: None,
            tilt: None,
        }
    }

    #[test]
    fn calm_profile_plans_nothing() {
        let segs = vec![seg(0.0, 20.0)];
        assert!(plan(&segs, &[(3.0, 0.95)], Params::CALM, &[]).is_empty());
    }

    /// 网格 + 空档补位：任何一段都不能有 > max_gap 的「没有重音」区间。
    #[test]
    fn gaps_never_exceed_the_limit() {
        let segs = vec![seg(0.0, 20.0)];
        let accents = plan(&segs, &[], Params::BALANCED, &[]);
        let m = metrics(&accents, 20.0);
        assert!(
            m.max_gap_secs <= Params::BALANCED.max_gap_secs + 1e-6,
            "最长空档 {} 超了：{accents:?}",
            m.max_gap_secs
        );
        // 全部落在网格上（段首 0.6s + k × beat）
        for a in &accents {
            let k = ((a.at - 0.6) / Params::BALANCED.beat_secs).round();
            assert!(((a.at - 0.6) - k * Params::BALANCED.beat_secs).abs() < 1e-6, "{} 不在网格上", a.at);
        }
    }

    /// 重音之间至少隔 `heavy_every`：相邻两个爆点只有一个能当「重」，另一个降成「中」。
    #[test]
    fn heavies_are_spaced_and_demoted() {
        let segs = vec![seg(0.0, 30.0)];
        let accents = plan(&segs, &[(7.5, 0.95), (11.2, 0.9)], Params::BALANCED, &[]);
        let heavies = accents.iter().filter(|a| a.strength == Strength::Heavy).count();
        assert_eq!(heavies, 1, "重的必须被 heavy_every 压住：{accents:?}");
        let mediums = accents.iter().filter(|a| a.strength == Strength::Medium && a.on_punch).count();
        assert!(mediums >= 1, "第二个爆点降成中、不能被丢掉：{accents:?}");
    }

    /// 手工运镜周围不插重音（用户自己摆的东西不被后处理叠上另一个机位）。
    #[test]
    fn manual_camera_is_avoided() {
        let mut s = seg(0.0, 20.0);
        s.events.push(manual_camera(7.8, 12.0));
        let accents = plan(&[s], &[], Params::BALANCED, &[]);
        assert!(
            accents.iter().all(|a| (a.at - 7.8).abs() > Params::BALANCED.avoid_secs),
            "手工运镜附近插了重音：{accents:?}"
        );
    }

    /// 屏蔽区间里不生成重音（屏蔽只做标记，生成侧不产出）。
    #[test]
    fn muted_ranges_get_no_accents() {
        let segs = vec![seg(0.0, 20.0)];
        let mutes = vec![MuteRange { start: 6.0, end: 10.0 }];
        let accents = plan(&segs, &[], Params::BALANCED, &mutes);
        assert!(
            accents.iter().all(|a| !(a.at >= 6.0 && a.at < 10.0)),
            "屏蔽区里插了重音：{accents:?}"
        );
    }

    /// **幂等**：带着账本重建不能叠加（process 每次预览/出片都会被隐式触发）。
    #[test]
    fn apply_is_idempotent_with_the_ledger() {
        let mut segs = vec![seg(0.0, 20.0)];
        let accents = plan(&segs, &[], Params::BALANCED, &[]);
        let (n1, ledger) = apply(&mut segs, &accents, &[]);
        assert!(n1 > 0);
        let total1 = segs[0].events.len();
        let (n2, ledger2) = apply(&mut segs, &accents, &ledger);
        assert_eq!(segs[0].events.len(), total1, "重建叠加了事件");
        assert_eq!(n2, n1);
        assert_eq!(ledger2, ledger);
    }

    /// 用户改过字段的派生事件**不再被删**（改完就归用户）。
    #[test]
    fn user_edited_auto_event_is_kept() {
        let mut segs = vec![seg(0.0, 20.0)];
        let accents = plan(&segs, &[], Params::BALANCED, &[]);
        let (_n, ledger) = apply(&mut segs, &accents, &[]);
        let idx = segs[0]
            .events
            .iter()
            .position(|e| matches!(e, Event::Shake { .. }))
            .expect("至少有一条抖动");
        if let Event::Shake { intensity, .. } = &mut segs[0].events[idx] {
            *intensity = 23.0;
        }
        let removed = clear(&mut segs, &ledger);
        assert!(removed < ledger.len(), "改过的那条被删了");
        assert!(
            segs[0].events.iter().any(|e| matches!(e, Event::Shake { intensity, .. } if (*intensity - 23.0).abs() < 1e-9)),
            "用户改过的特效必须留着"
        );
    }

    #[test]
    fn metrics_report_density_and_hit_rate() {
        let accents = vec![
            Accent { at: 2.0, strength: Strength::Light, preset: Preset::Punch, on_punch: false },
            Accent { at: 6.0, strength: Strength::Heavy, preset: Preset::Cut, on_punch: true },
        ];
        let m = metrics(&accents, 60.0);
        assert_eq!(m.accents, 2);
        assert!((m.per_minute - 2.0).abs() < 1e-9);
        assert_eq!(m.heavies, 1);
        assert!((m.punch_hit_rate - 0.5).abs() < 1e-9);
        assert!((m.max_gap_secs - 54.0).abs() < 1e-9);
    }
    /// **「呼吸」整片只用一次**（用户口径）：不是每一档的模板，`breath_max = 0` 时完全不用。
    #[test]
    fn breathe_is_a_once_per_clip_preset() {
        let segs = vec![seg(0.0, 40.0)];
        let punches = vec![(7.5, 0.95), (18.2, 0.9), (29.4, 0.85)];
        let accents = plan(&segs, &punches, Params::BALANCED, &[]);
        assert_eq!(
            accents.iter().filter(|a| a.preset == Preset::Breathe).count(),
            1,
            "呼吸只能有一次：{accents:?}"
        );
        // 关掉呼吸 ⇒ 一条都没有（其余重音照旧）
        let no_breath = Params { breath_max: 0, ..Params::BALANCED };
        let accents = plan(&segs, &punches, no_breath, &[]);
        assert!(accents.iter().all(|a| a.preset != Preset::Breathe));
    }

    /// **换档不回来**（只有「呼吸」才回位）：轻度重音只抖、连续同档不重复插运镜。
    #[test]
    fn camera_moves_are_one_way_and_only_on_tier_change() {
        // 两个相邻爆点 ⇒ 都是「中」（Hold）⇒ 第二条不该再插一条推近。
        // 关掉全部「一次性手法」，才好单独看「保持」的行为。
        let hold_only = Params {
            breath_max: 0,
            push_max: 0,
            pan_max: 0,
            dutch_max: 0,
            impact_max: 0,
            ..Params::BALANCED
        };
        let mut segs = vec![seg(0.0, 30.0)];
        let accents = plan(&segs, &[(7.5, 0.7), (11.2, 0.7)], hold_only, &[]);
        assert!(accents.iter().filter(|a| a.preset == Preset::Hold).count() >= 2, "{accents:?}");
        let (_n, _ledger) = apply(&mut segs, &accents, &[]);
        let cams: Vec<_> = segs[0]
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Camera { zoom, .. } => Some(zoom.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(cams.iter().filter(|z| z.as_str() == "近").count(), 1, "同档重音不重复插运镜：{cams:?}");
        // 单向：没有任何「回到中」的短事件（只有呼吸才会回位）
        let returns = segs[0].events.iter().filter(|e| {
            matches!(e, Event::Camera { zoom, end, time, .. }
                if zoom == "中" && end.is_some_and(|en| en - *time <= 0.6))
        });
        assert_eq!(returns.count(), 0, "换档不该回位：{cams:?}");
    }

    /// 轻度（Punch）**不动机位**：只给一下瞬时特效。
    #[test]
    fn light_accents_never_move_the_camera() {
        let no_breath = Params { breath_max: 0, ..Params::BALANCED };
        let mut segs = vec![seg(0.0, 20.0)];
        let accents = plan(&segs, &[], no_breath, &[]);
        assert!(accents.iter().all(|a| a.preset == Preset::Punch), "无爆点时全是轻：{accents:?}");
        apply(&mut segs, &accents, &[]);
        assert!(
            !segs[0].events.iter().any(|e| matches!(e, Event::Camera { .. })),
            "轻档不该插运镜"
        );
        assert!(segs[0].events.iter().any(|e| matches!(e, Event::Shake { .. })), "轻档要抖一下");
    }
    /// 新手法都受**整片上限**约束（攒不到条件就不加，绝不为凑数而用）。
    #[test]
    fn new_presets_respect_their_caps() {
        let segs = vec![seg(0.0, 120.0)];
        let mut punches = Vec::new();
        let mut t = 6.0;
        while t < 116.0 {
            punches.push((t, 0.95));
            t += 7.0;
        }
        let accents = plan(&segs, &punches, Params::BALANCED, &[]);
        let n = |p: Preset| accents.iter().filter(|a| a.preset == p).count();
        assert!(n(Preset::Pan) <= Params::BALANCED.pan_max, "甩视线超配额: {accents:?}");
        assert!(n(Preset::Dutch) <= Params::BALANCED.dutch_max, "歪镜超配额: {accents:?}");
        assert!(n(Preset::Push) <= Params::BALANCED.push_max, "慢推超配额: {accents:?}");
        assert!(n(Preset::Impact) <= Params::BALANCED.impact_max, "重击超配额: {accents:?}");
        assert!(n(Preset::Breathe) <= Params::BALANCED.breath_max, "呼吸超配额: {accents:?}");
    }

    /// 甩视线是**两段**：甩到弹幕、再甩回主播（不能停在弹幕上）。
    #[test]
    fn pan_goes_to_danmaku_and_comes_back() {
        let mut segs = vec![seg(0.0, 30.0)];
        let accents = vec![Accent { at: 8.0, strength: Strength::Medium, preset: Preset::Pan, on_punch: true }];
        apply(&mut segs, &accents, &[]);
        let cams: Vec<(String, Option<String>, String)> = segs[0]
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Camera { anchor, anchor_to, zoom, .. } => {
                    Some((anchor.clone().unwrap_or_default(), anchor_to.clone(), zoom.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(cams.len(), 2, "甩出去 + 甩回来 = 两条事件：{cams:?}");
        assert_eq!(cams[0].0, "liver");
        assert_eq!(cams[0].1.as_deref(), Some("danmaku"));
        assert_eq!(cams[1].0, "danmaku");
        assert_eq!(cams[1].1.as_deref(), Some("liver"));
    }

    /// 歪镜：**只歪一次**（≤ DUTCH_DEG）、且后面有一条回正。
    #[test]
    fn dutch_is_bounded_and_restored() {
        let mut segs = vec![seg(0.0, 30.0)];
        let accents = vec![Accent { at: 8.0, strength: Strength::Heavy, preset: Preset::Dutch, on_punch: true }];
        apply(&mut segs, &accents, &[]);
        let tilts: Vec<f64> = segs[0]
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Camera { tilt: Some(t), .. } => Some(*t),
                _ => None,
            })
            .collect();
        assert_eq!(tilts.len(), 1, "只歪一下：{tilts:?}");
        assert!(tilts[0].abs() <= crate::process::fx::MAX_DUTCH_ANGLE);
        // 回正：歪的那条之后还有一条不带 tilt 的运镜
        let restored = segs[0].events.iter().any(|e| matches!(e, Event::Camera { tilt: None, time, .. } if *time >= 10.0));
        assert!(restored, "歪完必须回正（倾斜是持续状态）");
    }

    /// 重击**不换机位**：只有重抖 + 白闪。
    #[test]
    fn impact_keeps_the_camera_still() {
        let mut segs = vec![seg(0.0, 20.0)];
        let accents = vec![Accent { at: 8.0, strength: Strength::Heavy, preset: Preset::Impact, on_punch: true }];
        apply(&mut segs, &accents, &[]);
        assert!(
            !segs[0].events.iter().any(|e| matches!(e, Event::Camera { .. })),
            "重击不该插运镜"
        );
        assert!(
            segs[0].events.iter().any(|e| matches!(e, Event::Shake { intensity, .. } if (*intensity - 24.0).abs() < 1e-9)),
            "重击要有重抖"
        );
        assert!(segs[0].events.iter().any(|e| matches!(e, Event::Flash { .. })), "重击要有白闪");
    }
    /// **默认档位冻结在 `calm`**（2026-10-19 拍板）：process 不再自己生成节奏 ——
    /// 用户口径「全靠 LLM 生产事件 + 意图和整体剪辑安排，process 只做资源热加载和应用」。
    /// 显式写 `profile = "balanced"` 才跑（保留给对拍/迁移验证）。
    #[test]
    fn default_profile_is_frozen_to_calm() {
        let cfg = ProjectFile::example();
        assert_eq!(cfg.rhythm.profile_name(), "calm");
        assert!(!params(&cfg).enabled, "空 profile 必须是「不生成」");
        let mut on = cfg.clone();
        on.rhythm.profile = "balanced".into();
        assert!(params(&on).enabled, "显式写 balanced 才能开");
    }
}
