/**
 * 39 档文字动效 —— **逐字/整块的求值内核**。
 *
 * ⚠️⚠️ **这份不是"再设计一遍"**，是 Dhampir 权威实现的**逐值移植**：
 *
 *   几何 + 时间：`crates/dhampir-timeline/src/text_perchar_anim.rs`（`char_transform_at`）
 *   整块运动  ：`crates/dhampir-timeline/src/text_motion.rs`
 *                 · `text_shake_px`   ← 抖动（两个正弦叠加）
 *                 · `back_out3`       ← 弹入曲线（S=3.0，峰值 1.25）
 *   缓动      ：`crates/dhampir-timeline/src/easing.rs`（`EasingForm::apply`）
 *   摇摆：**不在底座**，是 YekiTrim 转译器把 `sway_deg`/`sway_ms` 展开成
 *         `layers[].keyframes` 上的 `rotation` 关键帧
 *         （`tools/polish-to-dhampir.mjs::swayKeyframes`）。
 *
 * 成片是 Rust 画的 ⇒ **权威在那两份 Rust**。本文件只负责让 HTML 预览与它逐值一致。
 * ⚠️ 别在这里"顺手优化"任何一个系数 —— 那会让预览与成片分叉，
 *    而分叉的症状是"预览里动了、成片里没动"（或反过来），两边各自看都正常。
 */

// ===================== 缓动 =====================
// 与 Rust `EasingForm::apply` 逐式相同。
function easing(name, t) {
  const x = Math.min(1, Math.max(0, t));
  switch (name) {
    case 'back_out': {
      // `const S: f32 = 1.70158; u*u*((S+1)*u+S)+1`
      const S = 1.70158;
      const u = x - 1;
      return u * u * ((S + 1) * u + S) + 1;
    }
    case 'ease_in': return x * x;
    case 'ease_out': return 1 - (1 - x) * (1 - x);
    case 'ease_in_out': return x < 0.5 ? 2 * x * x : 1 - 2 * (1 - x) * (1 - x);
    default: return x; // linear（认不出的串在 Rust 侧也退回线性）
  }
}

// ⚠️ `back_out3`（S=3.0）是**弹入专用**的第三条曲线，与上面的 `back_out`（1.70158）
//    **不是同一条**。混用会让 slam/pop 的过冲幅度差一截。
function backOut3(t) {
  const S = 3.0;
  const x = Math.min(1, Math.max(0, t));
  const u = x - 1;
  return u * u * ((S + 1) * u + S) + 1;
}

// ===================== 逐字 =====================
const PER_CHAR_DEFAULT = {
  scaleFrom: 1, scaleTo: 1, rotationFrom: 0, rotationTo: 0,
  offsetXFrom: 0, offsetXTo: 0, offsetYFrom: 0, offsetYTo: 0,
  perCharFrames: 0, staggerFrames: 0, easing: 'linear', reverse: false,
  spreadXEm: 0, spreadYEm: 0, spreadXFrom: 0, spreadXTo: 0, spreadYFrom: 0, spreadYTo: 0,
  scaleSpread: 0,
};

/** 逐字档在 `localFrame` 帧、第 `index` 个字上的变换。
 *  逐式对应 Rust `char_transform_at`（三条边界一条不少）。 */
export function charTransformAt(spec, index, count, localFrame) {
  const s = { ...PER_CHAR_DEFAULT, ...(spec || {}) };
  // ① `is_active`：端点全相等且无 spread ⇒ 什么都不会动（Rust 侧同样返回恒等）。
  const active = s.scaleFrom !== s.scaleTo || s.rotationFrom !== s.rotationTo
    || s.offsetXFrom !== s.offsetXTo || s.offsetYFrom !== s.offsetYTo
    || (s.spreadXEm !== 0 && s.spreadXFrom !== s.spreadXTo)
    || (s.spreadYEm !== 0 && s.spreadYFrom !== s.spreadYTo) || s.scaleSpread !== 0;
  if (!active) return { scale: 1, rotationDeg: 0, offsetXEm: 0, offsetYEm: 0 };

  // `start_frame_for`：错峰起点（`reverse` 时从最后一个字开始）。
  let start = 0;
  if (count > 1) {
    const last = count - 1;
    const i = Math.min(index, last);
    start = (s.reverse ? last - i : i) * s.staggerFrames;
  }
  const dur = Math.max(1, s.perCharFrames);
  // ⚠️ 浮点除法。整数除法会让 t 恒为 0 ⇒ 所有字停在起始值（"配了不动"）。
  const t = Math.min(1, Math.max(0, (localFrame - start) / dur));
  const e = easing(s.easing, t);
  const mix = (a, b) => a + (b - a) * e;

  // ⚠️ `rank` 归一化 ⇒ 最后一个字挪满 `spread_*_em`，**与字数无关**。
  let rank = 0;
  if (count > 1) {
    const last = count - 1;
    const i = Math.min(index, last);
    rank = (s.reverse ? last - i : i) / last;
  }
  return {
    // `scale_spread` 是**乘数**（倍率的比例才是"大一点"的直觉）。
    scale: mix(s.scaleFrom, s.scaleTo) * (1 + s.scaleSpread * rank),
    rotationDeg: mix(s.rotationFrom, s.rotationTo),
    offsetXEm: mix(s.offsetXFrom, s.offsetXTo) + mix(s.spreadXFrom, s.spreadXTo) * s.spreadXEm * rank,
    offsetYEm: mix(s.offsetYFrom, s.offsetYTo) + mix(s.spreadYFrom, s.spreadYTo) * s.spreadYEm * rank,
  };
}

// ===================== 整块：抖动 =====================
/** 与 Rust `text_shake_px` 逐式相同（两个正弦叠加，相位表是固定的）。 */
export function textShakePx(tSec, shakePx, shakeHz, phase = 0) {
  if (!(shakePx > 0) || !Number.isFinite(shakePx)) return [0, 0];
  if (!(shakeHz > 0) || !Number.isFinite(shakeHz) || !Number.isFinite(tSec)) return [0, 0];
  const w = 2 * Math.PI * shakeHz;
  const dx = shakePx * (0.62 * Math.sin(w * tSec + phase) + 0.38 * Math.sin(w * 2.7 * tSec + phase * 1.9));
  const dy = shakePx * (0.62 * Math.sin(w * 1.3 * tSec + phase + 1.1)
    + 0.38 * Math.sin(w * 3.1 * tSec + phase * 0.6 + 2.3));
  return [dx, dy];
}

// ===================== 整块：摇摆 =====================
/** 复刻转译器 `swayKeyframes`：0, +A, 0, −A, 0, … 每四分之一周期一个键，
 *  线性插值。⇒ 它是一个**三角波**，不是正弦。 */
export function swayDegAt(tSec, swayDeg, swayMs) {
  if (!(swayDeg > 0) || !(swayMs > 0)) return 0;
  const quarterSec = swayMs / 1000 / 4;
  // 在三角波上定位：每 quarter 段切换 [0,+1,0,−1]。
  const seg = Math.floor(tSec / quarterSec);
  const frac = (tSec / quarterSec) - seg;
  const PATTERN = [0, 1, 0, -1];
  const a = PATTERN[((seg % 4) + 4) % 4];
  const b = PATTERN[(((seg + 1) % 4) + 4) % 4];
  return swayDeg * (a + (b - a) * frac); // 相邻键之间 linear ⇒ 三角波
}

// ===================== 整块：弹入 =====================
/** 弹入缩放：`from + (peak − from)·back_out3(p)`。
 *  ⚠️ `peak` 是**端点**不是峰值：曲线在中点冲到 1.25 ⇒ 实际最大
 *     `from + (peak−from)·1.25`。 */
export function scaleInAt(localMs, from, peak, ms) {
  if (!(ms > 0) || localMs >= ms) return peak;
  if (localMs < 0) return from;
  const p = localMs / ms;
  return from + (peak - from) * backOut3(p);
}
