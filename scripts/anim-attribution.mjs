// ⚠️ **本文件是生成的** —— 别手改，改 `tools/polish-to-dhampir.mjs` 的
//    `ANIM_PRESETS` 后跑 `node scripts/gen-anim-attribution.mjs` 重新生成。
//    判据模式：`node scripts/check-anim-attribution.mjs`（不一致就红）。
//
// **三轴 9 档**的参数归属表：每一档归到「排版 / 过渡 / 动效」之一，
// 并保留"这一档**实际给了哪些字段**"这一层。
//
// # 这张表是怎么来的（⚠️ 别改成手写）
//
// `node scripts/gen-anim-attribution.mjs` **真跑一遍转译器**，逐档单独一条事件、
// 看产物里**真的出现了哪些字段**，再按下面两条口径分到两路。
// **不是**从预设表按键名推的 —— 键名分类看不见 `motionFields` 那几道门
// （`on` / `onTime` / `onTransition` / `onStyle` / `popInOn`）写错时
// "这一档静默不写字段"这件事。
//
// # 判据（别改这两条）
//   * **form** = 决定"字静态长什么样"（**与帧无关**）：`vertical` /
//     `safe_width_ratio` / `font_weight` / `per_char`。
//   * **motion** = 画面**随时间变**：`scale_in_*` / `shake_*`，以及
//     `layers[].keyframes:rotation`（**图层旋转关键帧**那条腿 —— 它不写 `subtitle`）。
//
// ⚠️ `axis` 与"给了哪些字段"是**两件事**：`axis` 回答"用户该填哪一栏"
//    （界面分组用），`form`/`motion` 回答"这一档实际写了什么"（`motionFields` 用）。
//    `fade` 就是那个反例：`axis='transition'`，但**一个字段都不给** ——
//    它靠"压掉全局 `[elements]`"表达"只淡入淡出"，不是"没实现"。
//
// # 统计（9 档）
//   style 3 · transition 2 · effect 4
export const ANIM_ATTRIBUTION = {
  'arc': { axis: 'style', form: ['per_char'], motion: [] },
  'fade': { axis: 'transition', form: [], motion: [] },
  'heartbeat': { axis: 'effect', form: [], motion: ['scale_in_from', 'scale_in_peak', 'scale_in_ms'] },
  'perchar_big': { axis: 'effect', form: ['per_char'], motion: [] },
  'perchar_small': { axis: 'effect', form: ['per_char'], motion: [] },
  'pop': { axis: 'transition', form: [], motion: ['scale_in_from', 'scale_in_peak', 'scale_in_ms'] },
  'shake': { axis: 'effect', form: [], motion: ['shake_px', 'shake_hz'] },
  'tilt': { axis: 'style', form: ['layers[].transform'], motion: [] },
  'vtext': { axis: 'style', form: ['vertical', 'safe_width_ratio'], motion: [] },
};

/** 这一档属于哪条轴（`''` = 表里没有这一档）。 */
export function axisOf(name) {
  const a = ANIM_ATTRIBUTION[name];
  return a ? a.axis : '';
}

/** 这一档在「排版」轴上有东西吗。 */
export function hasFormPart(name) {
  const a = ANIM_ATTRIBUTION[name];
  return !!a && a.form.length > 0;
}

/** 这一档在「动效」轴上有东西吗（含图层关键帧那条腿）。 */
export function hasMotionPart(name) {
  const a = ANIM_ATTRIBUTION[name];
  return !!a && a.motion.length > 0;
}
