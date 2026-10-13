// ⚠️ **本文件是生成的** —— 别手改，改 `ANIM_PRESETS` 后跑
//    `node scripts/gen-anim-attribution.mjs` 重新生成。
//    判据模式：`node scripts/gen-anim-attribution.mjs --check`（不一致就红）。
//
// 39 档的**参数归属表**：每一档的每个参数属于「形态」还是「动效」。
//
// # 判据（别改这两条）
//   * **形态** = 决定"字静态长什么样"（**与帧无关**）。`scaleSpread` 属形态 ——
//     它住在 `perChar` 里，但四个端点全相等 ⇒ 是一条**静止的**序号阶梯。
//   * **动效** = 画面**随时间变**。两个来源：① 整块时间函数（shake/sway/scaleIn*）；
//     ② 逐字 spec 的**端点不相等** ⇒ `char_transform_at` 的第 4 参（帧）真的起作用。
//
// # 统计（39 档）
//   form 15 · motion 12 · both 11 · none 1
//
// ⚠️ `none` 只有一档：`fade_only` —— 它动的是 **opacity**，不在这两条路上
//    （**不是"没实现"**，见 `docs/design/39-tiers-split-style-vs-effect.md`）。
export const ANIM_ATTRIBUTION = {
  'arc_down': { kind: 'both', form: ['rotationFrom', 'rotationTo'], pcTime: ['rotationFrom↔rotationTo'] },
  'arc_up': { kind: 'both', form: ['rotationFrom', 'rotationTo', 'spreadXEm'], pcTime: ['rotationFrom↔rotationTo'] },
  'arc_wave': { kind: 'both', form: ['rotationFrom', 'rotationTo'], pcTime: ['rotationFrom↔rotationTo'] },
  'fade_only': { kind: 'none', form: [], pcTime: [] },
  'grow_asc': { kind: 'form', form: ['scaleSpread'], pcTime: [] },
  'grow_burst': { kind: 'form', form: ['scaleSpread'], pcTime: [] },
  'grow_first': { kind: 'form', form: ['reverse', 'scaleSpread'], pcTime: [] },
  'hop_big': { kind: 'both', form: ['offsetYFrom', 'offsetYTo'], pcTime: ['offsetYFrom↔offsetYTo'] },
  'hop_fast': { kind: 'both', form: ['offsetYFrom', 'offsetYTo'], pcTime: ['offsetYFrom↔offsetYTo'] },
  'hop_std': { kind: 'both', form: ['offsetYFrom', 'offsetYTo'], pcTime: ['offsetYFrom↔offsetYTo'] },
  'pop_perchar': { kind: 'both', form: ['scaleFrom', 'scaleTo'], pcTime: ['scaleFrom↔scaleTo'] },
  'pop_soft': { kind: 'motion', form: [], pcTime: [] },
  'pop_std': { kind: 'motion', form: [], pcTime: [] },
  'scatter_lg': { kind: 'both', form: ['spreadXEm', 'spreadXFrom', 'spreadXTo', 'spreadYEm', 'spreadYFrom', 'spreadYTo'], pcTime: ['spreadXFrom↔spreadXTo', 'spreadYFrom↔spreadYTo'] },
  'scatter_shake': { kind: 'both', form: ['spreadXEm', 'spreadXFrom', 'spreadXTo', 'spreadYEm', 'spreadYFrom', 'spreadYTo'], pcTime: ['spreadXFrom↔spreadXTo', 'spreadYFrom↔spreadYTo'] },
  'scatter_sm': { kind: 'both', form: ['spreadXEm', 'spreadXFrom', 'spreadXTo', 'spreadYEm', 'spreadYFrom', 'spreadYTo'], pcTime: ['spreadXFrom↔spreadXTo', 'spreadYFrom↔spreadYTo'] },
  'shake_lg': { kind: 'motion', form: [], pcTime: [] },
  'shake_slow': { kind: 'motion', form: [], pcTime: [] },
  'shake_std': { kind: 'motion', form: [], pcTime: [] },
  'shrink_desc': { kind: 'form', form: ['scaleSpread'], pcTime: [] },
  'shrink_drop': { kind: 'form', form: ['scaleSpread'], pcTime: [] },
  'shrink_in_fast': { kind: 'motion', form: [], pcTime: [] },
  'shrink_in_std': { kind: 'motion', form: [], pcTime: [] },
  'shrink_tail': { kind: 'form', form: ['scaleSpread'], pcTime: [] },
  'size_first': { kind: 'form', form: ['reverse', 'scaleSpread'], pcTime: [] },
  'size_last': { kind: 'form', form: ['scaleSpread'], pcTime: [] },
  'size_mid': { kind: 'form', form: ['scaleSpread'], pcTime: [] },
  'slam_lg': { kind: 'motion', form: [], pcTime: [] },
  'slam_squash': { kind: 'both', form: ['squashY'], pcTime: [] },
  'slam_std': { kind: 'motion', form: [], pcTime: [] },
  'sway_lg': { kind: 'motion', form: [], pcTime: [] },
  'sway_slow': { kind: 'motion', form: [], pcTime: [] },
  'sway_std': { kind: 'motion', form: [], pcTime: [] },
  'tilt_rev': { kind: 'form', form: ['tiltDeg'], pcTime: [] },
  'tilt_sm': { kind: 'form', form: ['tiltDeg'], pcTime: [] },
  'tilt_std': { kind: 'form', form: ['tiltDeg'], pcTime: [] },
  'vtext_bold': { kind: 'form', form: ['fontFamily', 'fontWeight', 'safeWidthRatio', 'vertical'], pcTime: [] },
  'vtext_shrink': { kind: 'form', form: ['fontFamily', 'fontWeight', 'safeWidthRatio', 'scaleSpread', 'vertical'], pcTime: [] },
  'vtext_thin': { kind: 'form', form: ['fontFamily', 'fontWeight', 'safeWidthRatio', 'vertical'], pcTime: [] },
};

/** 这一档在「样式」轴上有东西吗。 */
export function hasFormPart(name) {
  const a = ANIM_ATTRIBUTION[name];
  return !!a && (a.kind === 'form' || a.kind === 'both');
}

/** 这一档在「动效」轴上有东西吗。 */
export function hasMotionPart(name) {
  const a = ANIM_ATTRIBUTION[name];
  return !!a && (a.kind === 'motion' || a.kind === 'both');
}
