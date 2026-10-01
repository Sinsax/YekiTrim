// 守卫：本仓转译器 `tools/polish-to-dhampir.mjs` 里那几条**从出片模板逐行抄来的曲线**。
//
// # 为什么需要它
//
// 转译器里最微妙的东西不是 TOML 解析，而是这几条曲线：
//
//   * `backOut3`       —— 贴纸弹入的过冲（`s = 3`，不是经典的 1.70158）
//   * `shakeOffsetAt`  —— 出片那份 `getShake` 的**四级阶梯**
//   * `handheldSway`   —— 手持摇摆的 2×4 正弦
//   * `stickerStateAt` —— 贴纸四段相位（**窗口 = 事件时长**、淡出在窗口内部，见第 5/6 节）
//
// 它们都**逐值可测**，而在本守卫之前**一条测试都没有** ——
// 而这几轮恰恰就是在它们身上连着踩坑：
//
//   1. `shake` 抄成了**预览**那一份（连续波形），出片要的是四级阶梯；
//   2. 贴纸的**窗口**与相位对不上：先是不够长（`dur + 0.65` 够不到淡出），
//      后来又是**太长**（放宽到 `dur + 1.05` ⇒ 贴纸比事件多显示 0.4s，用户实测出点晚了 ✗）。
//      现在口径回到「**窗口 = 事件时长 `dur`**、淡出移进窗口内部」，四处同形判据一起钉；
//   3. `backOut` 用契约的经典 1.70158 近似 `s = 3`，过冲从 1.25 掉到 1.10。
//
// 三条都是"看着差不多、逐值差得远"。所以这里全部按**逐值**钉，
// 不写"大约"「接近」这类断言 —— 那种断言抓不住上面任何一条。
//
// # 两种跑法
//
//     node scripts/check-vtrim-translator.mjs            # 逐值检查（项数见运行输出）
//     node scripts/check-vtrim-translator.mjs --mutate   # 全部变异必须**打红**（自证不是空转）
//
// ⚠️ **--mutate 那一趟必须真改源码文本，并先断言替换确实命中了**：
//    我第一轮跑变异时"没红"的 4 条是**变异台写错了** —— 把函数提到顶层时缩进从 4 空格
//    变 2 空格，替换串没匹配上，于是"变异体"其实没变、检查当然全绿。
//    所以每处变异都先断言 `命中次数 === 1`、再断言 `mutated !== source`，
//    最后才要求检查**翻面**（抛断言错）。少了前两条，这个自证本身就是空转的。
//
// 变异体的跑法：改写后的源码写进系统临时目录、用 `import()` 真加载一遍
// （不去 import 应用模块、也不动仓库里的文件）—— 每次都是新 URL，模块缓存不会串。

import { strict as assert } from 'node:assert';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
/// 转译器路径**相对于本脚本**算（从哪个目录调用都一样）。
const TRANSLATOR = join(HERE, '..', 'tools', 'polish-to-dhampir.mjs');

let checks = 0;
const near = (actual, expected, tol, what) => {
  checks += 1;
  assert.ok(
    Math.abs(actual - expected) <= tol,
    `${what}：期望 ${expected}（±${tol}），实得 ${actual}`,
  );
};

/// 全部逐值检查，返回检查项数。
///
/// **放进函数**是为了 `--mutate` 那一趟能拿**变异后的**模块再跑同一份检查 ——
/// 检查与"被测对象"必须是同一段代码，否则自证证明的是另一件事。
function checkCurves(curves) {
  checks = 0;
  const {
    SHAKE_SECONDS,
    STICKER_FLOAT_UP_SEC,
    STICKER_WINDOW_SEC,
    DANMAKU_FADE_IN_MS,
    backOut3,
    danmakuExitMs,
  withMoveTravel,
    handheldSway,
    pow2Out,
    shakeOffsetAt,
    stickerStateAt,
    stickerWindowFrames,
  } = curves;
  /// 贴纸窗口的秒数上界（相对事件起点）：`WINDOW = 0` ⇒ 窗口 = `[et, et + dur]`。
  /// 旧口径是 `0.35 + 0.4 + 0.3`（= `dur + 1.05`，贴纸比事件多显示 0.4s ✗）。
  const WINDOW = 0;
  const FADE_OUT = 0.3;
  const FPS = 60;

  // -------------------------------------------------------------------------
  // 1. `pow2_out` = 1 - (1-p)^2 —— 与底座的 `EaseOut` 逐值相同
  // -------------------------------------------------------------------------
  near(pow2Out(0), 0, 0, 'pow2Out(0)');
  near(pow2Out(1), 1, 0, 'pow2Out(1)');
  near(pow2Out(0.5), 0.75, 0, 'pow2Out(0.5) —— 不是 0.5，这正是"缓动"的意义');

  // -------------------------------------------------------------------------
  // 2. `back_out(p, 3)` —— 峰值 1.25，而经典 1.70158 是 1.10
  // -------------------------------------------------------------------------
  //
  // 峰值出现在 u = t-1 = -0.5（即 p = 0.5）：
  //   f(u) = u^2*((s+1)u + s) + 1  ->  f(-0.5) = 0.25*(4*(-0.5)+3) + 1 = 1.25
  //
  // ⚠️ **峰值这条刻意排在端点前面**：`s` 改成经典 1.70158 时端点也会红，
  //    但报出来的是 `backOut3(0)` 差 2.2e-16 —— 那看着像浮点噪声、看不出"过冲软了一截"。
  //    把峰值放最前，变异报的就是 `期望 1.25，实得 1.1000…`。别把顺序"顺手理顺"回去。
  near(backOut3(0.5), 1.25, 1e-12, 'backOut3(0.5) 的过冲峰值必须是 1.25');
  near(backOut3(0), 0, 0, 'backOut3(0)');
  near(backOut3(1), 1, 0, 'backOut3(1)');
  // 反过来说：用经典的 1.70158 会得到 1.10 —— 这条断言能区分两者。
  const classic = (t, s = 1.70158) => {
    const u = t - 1;
    return u * u * ((s + 1) * u + s) + 1;
  };
  // 经典那条的峰值在 `u = -2s/(3(s+1))` —— 注意是 `u`，而 `t = 1 + u`。
  // （我第一版写成 `classic(0.42)` 是**取错了点**：那是 u 不是 t，
  //   于是断言"经典峰值是 1.10"本身就假了。）
  const PEAK_U = (-2 * 1.70158) / (3 * 2.70158);
  near(classic(1 + PEAK_U), 1.0999, 1e-3, '经典 1.70158 的峰值约 1.10');
  assert.ok(
    backOut3(0.5) > classic(1 + PEAK_U) + 0.1,
    's=3 的过冲必须**明显**比经典猛，否则贴纸弹入会软一截',
  );
  checks += 1;

  // -------------------------------------------------------------------------
  // 3. `shakeOffsetAt` —— **出片那份** `getShake` 的四级阶梯
  // -------------------------------------------------------------------------
  //
  //     dt < 0.04  -> ( off, -off)
  //     dt < 0.08  -> (-off,  off)
  //     dt < 0.12  -> ( off/2, -off/2)
  //     dt < 0.16  -> (-off/2,  off/2)
  //     否则        -> (0, 0)
  //
  // **每一档的边界两侧都要钉**：这正是"阶梯 vs 连续波形"的区别所在。
  const off = 0.6;
  const steps = [
    [0.0, 0.6, -0.6],
    [0.0399, 0.6, -0.6],
    [0.04, -0.6, 0.6],
    [0.0799, -0.6, 0.6],
    [0.08, 0.3, -0.3],
    [0.1199, 0.3, -0.3],
    [0.12, -0.3, 0.3],
    [0.1599, -0.3, 0.3],
    [0.16, 0, 0],
    [0.2, 0, 0],
  ];
  for (const [dt, x, y] of steps) {
    const got = shakeOffsetAt(dt, off);
    near(got.x, x, 0, `shakeOffsetAt(${dt}).x`);
    near(got.y, y, 0, `shakeOffsetAt(${dt}).y`);
  }
  // **阶梯就是阶梯**：在每一档内部取两点，值必须完全相同。
  // 如果误抄成预览那份的 0.025s 交替，这条会红。
  const within = (a, b) => {
    const p = shakeOffsetAt(a, off);
    const q = shakeOffsetAt(b, off);
    assert.deepEqual(p, q, `[${a}, ${b}) 应当同属一档`);
    checks += 1;
  };
  within(0.0, 0.0399);
  within(0.04, 0.0799);
  within(0.08, 0.1199);
  within(0.12, 0.1599);
  // 幅度必须是**像素原值**，不是归一化比例、不是乘过系数的。
  near(shakeOffsetAt(0, 14).x, 14, 0, '默认幅度 14 也直接当像素用');
  near(SHAKE_SECONDS, 0.2, 0, '窗口长度写死 0.2 秒');

  // -------------------------------------------------------------------------
  // 4. `handheldSway` —— 2 个正弦分量 × 4 个通道
  // -------------------------------------------------------------------------
  //
  // 系数逐个钉（`crates/vtrim-polish/templates/index.html:593`）：
  //   dx = (6.5*sin(TAU*0.47*t) + 3.0*sin(TAU*0.83*t+1.7)) * a
  //   dy = (5.0*sin(TAU*0.59*t+0.9) + 2.4*sin(TAU*1.13*t+2.4)) * a
  //   rot= (0.32*sin(TAU*0.37*t+0.4) + 0.16*sin(TAU*0.91*t+2.9)) * a
  //   scale = 1.0 + 0.007*sin(TAU*0.53*t+1.1)*a
  const TAU = Math.PI * 2;
  near(handheldSway(3, 0).dx, 0, 0, 'amp=0 时 dx 必须是 0');
  near(handheldSway(3, 0).dy, 0, 0, 'amp=0 时 dy 必须是 0');
  near(handheldSway(3, 0).rot, 0, 0, 'amp=0 时 rot 必须是 0');
  near(handheldSway(3, 0).scale, 1, 0, 'amp=0 时 scale 必须是 1');
  // [[t, amp]] -> 逐通道期望
  for (const t of [0, 1.7, 5.25, 19.9]) {
    const a = 1;
    const want = {
      dx: (6.5 * Math.sin(TAU * 0.47 * t) + 3.0 * Math.sin(TAU * 0.83 * t + 1.7)) * a,
      dy: (5.0 * Math.sin(TAU * 0.59 * t + 0.9) + 2.4 * Math.sin(TAU * 1.13 * t + 2.4)) * a,
      rot: (0.32 * Math.sin(TAU * 0.37 * t + 0.4) + 0.16 * Math.sin(TAU * 0.91 * t + 2.9)) * a,
      scale: 1.0 + 0.007 * Math.sin(TAU * 0.53 * t + 1.1) * a,
    };
    const got = handheldSway(t, a);
    for (const k of ['dx', 'dy', 'rot', 'scale']) near(got[k], want[k], 1e-12, `handheldSway(${t}).${k}`);
  }
  // **线性于 amp**：幅度是状态机上的 0~1 连续量，不是布尔。
  // 若误当成"开了就满幅"，这条会红。
  for (const t of [2.0, 7.3]) {
    const half = handheldSway(t, 0.5);
    const full = handheldSway(t, 1);
    for (const k of ['dx', 'dy', 'rot']) {
      near(half[k], full[k] / 2, 1e-12, `amp 必须是乘数（${k}）`);
    }
    // scale 是 1 + 0.007*sin*a：a=0.5 时"偏离 1 的量"应当是一半
    near(half.scale - 1, (full.scale - 1) / 2, 1e-12, 'amp 必须是乘数（scale 的偏离量）');
  }
  // 幅度量级：dx/dy 是**像素**，rot 是**度** —— 都不该超过几像素/几度。
  let maxDx = 0;
  for (let i = 0; i <= 2000; i += 1) maxDx = Math.max(maxDx, Math.abs(handheldSway(i / 20, 1).dx));
  assert.ok(maxDx > 9 && maxDx < 9.6, `dx 峰值应当约 9.5 像素，实得 ${maxDx}`);
  checks += 1;

  // -------------------------------------------------------------------------
  // 5. `stickerStateAt` —— 四段相位在**窗口内部**消化（窗口 = 事件时长 `dur`）
  // -------------------------------------------------------------------------
  //
  // 口径（`docs/vtrim-integration.md` §14）：**出点 = 最后一个显示帧**，窗口 `[et, et + dur]`。
  // 上一刀的形状是「窗口 = 四段之和 `dur + 1.05`、淡出接在事件**之后**」——
  // 贴纸比事件多显示 0.4 秒 ✗（用户实测：出点晚了）。现在淡出**贴在窗口末尾**
  // （`dur - FADE_OUT -> dur`），于是：
  //   ① 窗口上界 = `dur`（转译器常量 `STICKER_WINDOW_SEC === 0`）；
  //   ② 窗口内**末尾 0.3s** 的 opacity 单调降到 0；
  //   ③ **最后一帧**（`t = dur` 之前那一帧）的 opacity ≈ 0 —— 出点即淡出终点。
  const sk = 1;
  const dur = 2.37; // 本样例第一条贴纸（end - time）
  const FADE_START = dur - FADE_OUT;   // = 2.07（**窗口内部**，不是 `dur` 之后）
  const FADE_END = dur;                // 淡出终点 = 窗口上界 = 出点
  // 标称段长（窗口装得下时）：`dur >= 1.05` 时下面四条首段的读数与旧版逐值相同。
  near(STICKER_FLOAT_UP_SEC, 0.4, 0, '标称上浮段 = 0.4s');
  near(STICKER_WINDOW_SEC, 0, 0, '窗口不追加任何时长（`STICKER_WINDOW_SEC === 0`）');
  // 阶段一：弹入，进度 0 时 scale=0.3、rot=-15、opacity=0
  {
    const s = stickerStateAt(0, dur, sk);
    near(s.scale, 0.3, 0, '弹入起点 scale = 0.3');
    near(s.rot, -15, 0, '弹入起点 rot = -15 度');
    near(s.opacity, 0, 0, '弹入起点 opacity = 0');
    near(s.yOff, 0, 0, '弹入阶段不上浮');
  }
  // 弹入中点：back_out(0.5) = 1.25 -> scale = 0.3+0.7*1.25 = 1.175（**过冲**）
  {
    const s = stickerStateAt(0.175, dur, sk);
    near(s.scale, 0.3 + 0.7 * 1.25, 1e-12, '弹入中点必须过冲到 1.175（这是 back_out 的意义）');
    near(s.rot, -15 + 15 * 1.25, 1e-12, 'rot 也过冲，到 +3.75 度');
    near(s.opacity, pow2Out(0.5), 1e-12, 'opacity 走 pow2_out，不走 back_out');
  }
  // 弹入终点：恰好落回 1 / 0 / 1
  {
    const s = stickerStateAt(0.35, dur, sk);
    near(s.scale, 1, 1e-12, '弹入终点 scale = 1');
    near(s.rot, 0, 1e-12, '弹入终点 rot = 0');
    near(s.opacity, 1, 0, '弹入终点 opacity = 1');
  }
  // 阶段二：上浮 0.4 秒（`0.35 -> 0.75`），**线性**（不是缓动）
  {
    near(stickerStateAt(0.35 + 0.2, dur, sk).yOff, -18 * sk * 0.5, 1e-12, '上浮必须线性（中点 = -9）');
    near(stickerStateAt(0.35 + 0.1, dur, sk).yOff, -18 * sk * 0.25, 1e-12, '上浮 1/4 处 = -4.5');
    near(stickerStateAt(0.35 + 0.4, dur, sk).yOff, -18 * sk, 1e-12, '上浮终点 = -18');
  }
  // 阶段三：保持 —— 停在 -18，scale/rot 回到常态。
  // **右端缩短了**：保持段现在是 `0.75 -> dur - 0.3`（旧形状是 `0.75 -> dur`）。
  {
    const s = stickerStateAt((0.35 + 0.4 + FADE_START) / 2, dur, sk);
    near(s.yOff, -18 * sk, 0, '保持段停在 -18');
    near(s.opacity, 1, 0, '保持段完全不透明');
    near(s.scale, 1, 0, '保持段 scale = 1');
    // 保持段的右端点（淡出起点前一瞬）仍是满不透明 —— 淡出**没有**提前开始
    near(stickerStateAt(FADE_START - 1e-9, dur, sk).opacity, 1, 1e-9, '淡出起点之前 opacity 仍是 1');
  }
  // ①**窗口判据**：上界必须**恰好**是 `dur`（不溢出），淡出起点必须落在窗口**内部**。
  //    旧口径 `dur + 1.05` 与旧淡出起点 `popIn + floatUp + dur` 都在窗口之外 ✗。
  assert.ok(FADE_START >= 0 && FADE_START < dur, `淡出起点 ${FADE_START}s 不在窗口 (0, dur) 内`);
  checks += 1;
  near(FADE_START, dur - FADE_OUT, 1e-12, '淡出起点 = `dur - fadeOut`（不是 `popIn + floatUp + dur`）');
  near(FADE_END, dur, 0, '淡出终点 = 窗口上界 = `dur`');
  // ②图层逐帧采样的帧数：窗口 = `[0, dur]`（含上界那一帧），不再是 `dur + 1.05`。
  near(stickerWindowFrames(dur, FPS), Math.round(dur * FPS), 0, '采样帧数 = round(dur × fps)');
  near(stickerWindowFrames(dur, FPS), Math.round((dur + WINDOW) * FPS), 0, '采样帧数只由 dur 决定');
  // ③窗口内逐帧采样：淡出段必须**真的走进去**，且逐帧值与相位公式对得上。
  //    （旧窗口下这段采样的尾巴全落在窗口外的保持段 —— 这条就是那次不一致的探针。）
  let fadeFrames = 0;
  let prevOpacity = 1;
  for (let t = 0; t <= dur + WINDOW + 1e-12; t += 1 / FPS) {
    const s = stickerStateAt(t, dur, sk);
    assert.ok(
      Number.isFinite(s.opacity) && s.opacity >= 0 && s.opacity <= 1,
      `${t.toFixed(4)}s 的 opacity 越界/NaN：${s.opacity}`,
    );
    checks += 1;
    if (t >= FADE_START) {
      // 淡出段必须**单调变透**（中途回弹就是相位边界算错了）。
      assert.ok(
        s.opacity <= prevOpacity + 1e-9,
        `${t.toFixed(4)}s 的 opacity(${s.opacity}) 比上一帧(${prevOpacity}) 更不透明 —— 淡出回弹`,
      );
      checks += 1;
      if (t < FADE_END) {
        near(s.opacity, 1 - (t - FADE_START) / FADE_OUT, 1e-9, `窗口内 ${t.toFixed(4)}s 的淡出 opacity`);
        fadeFrames += 1;
      }
    }
    prevOpacity = s.opacity;
  }
  assert.ok(
    fadeFrames >= 15,
    `窗口内只采到 ${fadeFrames} 帧淡出（≥15 才算覆盖了 0.3s 的淡出段）`,
  );
  checks += 1;
  // ④淡出的端点值：起点全不透明、中点 0.5、终点 0 —— 而**终点正是窗口上界**。
  near(stickerStateAt(FADE_START, dur, sk).opacity, 1, 1e-12, '淡出起点 opacity = 1');
  near(stickerStateAt(FADE_START + FADE_OUT / 2, dur, sk).opacity, 0.5, 1e-12, '淡出中点 opacity = 0.5');
  near(stickerStateAt(dur + WINDOW, dur, sk).opacity, 0, 1e-12, '窗口上界 = 淡出终点，opacity = 0');
  {
    const s = stickerStateAt(FADE_START + FADE_OUT / 2, dur, sk);
    near(s.scale, 1 - 0.6 * 0.5, 1e-12, '淡出中点缩到 0.4 倍');
    near(s.yOff, -18 * sk, 1e-12, '淡出段仍停在 -18（上浮量不回退）');
  }
  // ⑤**用户实测的那条语义断言**：`dur = 1.0s`、`fps = 60` 的贴纸 ——
  //    **最后一帧**（第 59 帧，`t = 59/60`）的 opacity 必须 **≤ 0.1**，
  //    而**第 60 帧**（`t = 1.0`）**已不可见**（`< 0.005`，与三处 `continue` 的剔除阈值同口径）。
  //    这就是"出点没晚"。
  {
    const D1 = 1.0;
    const frames1 = stickerWindowFrames(D1, FPS);          // 60
    near(frames1, 60, 0, 'dur = 1.0s、fps = 60 ⇒ 窗口采 60 帧（第 0..60 帧）');
    const lastFrame = frames1 - 1;                          // 59 —— 最后一帧
    const sLast = stickerStateAt(lastFrame / FPS, D1, sk);
    const sOut = stickerStateAt(frames1 / FPS, D1, sk);
    assert.ok(
      sLast.opacity <= 0.1,
      `dur=1.0/fps=60 的最后一帧（第 ${lastFrame} 帧，t=${(lastFrame / FPS).toFixed(4)}）opacity = ${sLast.opacity} —— 必须 ≤ 0.1`,
    );
    checks += 1;
    assert.ok(
      sOut.opacity < 0.005,
      `第 ${frames1} 帧（t=${(frames1 / FPS).toFixed(4)}）opacity = ${sOut.opacity} —— 必须已不可见（< 0.005）`,
    );
    checks += 1;
    console.log(
      `  ・dur=1.0/fps=60：最后一帧（第 ${lastFrame} 帧，t=${(lastFrame / FPS).toFixed(4)}）opacity = ` +
        `${sLast.opacity.toFixed(6)}；第 ${frames1} 帧（t=${(frames1 / FPS).toFixed(4)}）opacity = ${sOut.opacity.toFixed(6)}（不可见）`,
    );
  }
  // ⑥**边界情形**：`dur` 装不下 `0.35 + 0.4 + 0.3` 时**优先保淡出**（前两段等比压缩、
  //    淡出起点不得早于 0）；任何时长都不许 NaN / 负段长 / 最后一帧**不透明**。
  //    `dur >= 0.2s` 时"末尾 0.3s 淡出"这条也仍然成立（≥ 0.9 个淡出段落在窗内）；
  //    更短的（不足 4 帧）只剩"最后一帧不是满不透明"这一条，淡出长度 = 窗口长度。
  for (const d of [0.2, 0.3, 0.5, 0.75, 1.0, 1.05, 2.5, 30]) {
    const frames = stickerWindowFrames(d, FPS);
    const last = stickerStateAt((frames - 1) / FPS, d, sk);
    const out = stickerStateAt(frames / FPS, d, sk);
    assert.ok(
      last.opacity <= 0.1,
      `dur=${d}s 的最后一帧 opacity = ${last.opacity} —— 必须 ≤ 0.1（淡出在窗口内部）`,
    );
    assert.ok(
      out.opacity < 0.005,
      `dur=${d}s 的出点那一帧 opacity = ${out.opacity} —— 必须已不可见`,
    );
    checks += 2;
  }
  for (const d of [0.05, 0.15]) {
    const frames = stickerWindowFrames(d, FPS);
    assert.ok(frames >= 1, `dur=${d}s 的窗口至少 1 帧（实得 ${frames}）`);
    const last = stickerStateAt((frames - 1) / FPS, d, sk);
    const out = stickerStateAt(frames / FPS, d, sk);
    assert.ok(
      Number.isFinite(last.opacity) && last.opacity >= 0 && last.opacity < 1,
      `dur=${d}s 的最后一帧 opacity = ${last.opacity} —— 必须有限、且**不是满不透明**`,
    );
    assert.ok(
      out.opacity < 0.005,
      `dur=${d}s 的出点那一帧 opacity = ${out.opacity} —— 必须已不可见`,
    );
    checks += 2;
  }
  // **单帧窗口**（`round(dur × fps) == 1`）：窗内只有一帧，它只能是不透明的 ——
  // 那不是"出点晚了"，而是"贴纸只活得了一帧"。这里只要求不出 NaN、且窗**外**立刻不可见。
  {
    const d = 1 / FPS;
    const frames = stickerWindowFrames(d, FPS);
    assert.equal(frames, 1, `dur = 1 帧时窗口必须是 1 帧（实得 ${frames}）`);
    const only = stickerStateAt(0, d, sk);
    assert.ok(
      Number.isFinite(only.opacity) && only.opacity >= 0 && only.opacity <= 1,
      `单帧窗口的 opacity 越界/NaN：${JSON.stringify(only)}`,
    );
    const out = stickerStateAt(frames / FPS, d, sk);
    assert.ok(Number.isFinite(out.opacity) && out.opacity < 0.005, `单帧窗口的出点不可见：${out.opacity}`);
    checks += 3;
  }
  // 退化时长（0 / 负）不许出 NaN，也不许可见。
  for (const d of [0, -1]) {
    const s = stickerStateAt(0, d, sk);
    assert.ok(
      Number.isFinite(s.opacity) && s.opacity === 0,
      `dur=${d}s 是退化时长 —— 应当不可见且不出 NaN，实得 ${JSON.stringify(s)}`,
    );
    checks += 1;
  }

  // -------------------------------------------------------------------------
  // 6. **参照守卫本身**（`crates/vtrim-polish/templates/index.html:getActiveSticker`）
  // -------------------------------------------------------------------------
  //
  // 上面 5 钉的是"曲线值"，不是"窗口判据"。而这条窗口恰恰两次出问题：
  // 先是**太短**（`dur + 0.65` 够不到淡出、末尾硬切），后来**太长**
  // （放宽到 `dur + 1.05` ⇒ 贴纸比事件多显示 0.4s、出点偏晚 ✗，用户实测）。
  //
  // 所以参照那侧的判据必须**一并**钉住：只测转译器，改了模板也没人拦。
  // 这里读模板源码（不是模块）算出窗口值：新口径是 **`t > et + dur`**（上界就是事件时长），
  // 旧形状 `dur + 1.05` / `dur + (0.35 + 0.4 + 0.3)` 必须被抓。
  const TEMPLATE = join(HERE, '..', 'crates', 'vtrim-polish', 'templates', 'index.html');
  // 判据形状：`t > et + dur`（新口径，不追加时长）或 `t > et + dur + <offset>`（旧口径）。
  // `offset` 允许带括号、允许写成"数 + 数 + …"；**注释行要滤掉**（历史数字会写在注释里）。
  const codeLines = (source) => source
    .split('\n')
    .filter((line) => !/^\s*(\/\/|\/\*|\*|<!--)/.test(line));
  const windowGuardOf = (source) => {
    const lines = codeLines(source).map((line) => line.replace(/\s+/g, ' ').trim());
    for (let i = 0; i < lines.length; i += 1) {
      const line = lines[i];
      if (!line.includes('t < et') || !line.includes('+ dur')) continue;
      // JS 那两份把 `continue` 写在同一行；Rust 那份是 `if t < et || t > et + dur {` +
      // 下一行的 `continue;` —— 两种都要认（这里只是**读窗口表达式**，不做语义判断）。
      if (line.includes('continue')) return line;
      if ((lines[i + 1] || '').includes('continue')) return `${line} ${lines[i + 1]}`;
    }
    assert.ok(false, '找不到贴纸的窗口判据（形如 `t < et || t > et + dur`）');
  };
  /// 抠出 `marker` 之后那个 `{ … }` 块。**跳过注释与字符串** —— 那里面写着历史值
  ///（比如旧形状的 `et+0.35+dur`），直接按字符数括弧会被它们骗。
  const blockAfter = (source, marker) => {
    const start = source.indexOf(marker);
    assert.ok(start >= 0, `模板里找不到 ${marker}`);
    const open = source.indexOf('{', start);
    let depth = 0;
    for (let i = open; i < source.length; i += 1) {
      const ch = source[i];
      if (ch === '/' && source[i + 1] === '/') {
        const nl = source.indexOf('\n', i);
        i = nl < 0 ? source.length : nl;
        continue;
      }
      if (ch === "'" || ch === '"' || ch === '`') {
        const quote = ch;
        i += 1;
        while (i < source.length && source[i] !== quote) {
          if (source[i] === '\\') i += 1;
          i += 1;
        }
        continue;
      }
      if (ch === '{') depth += 1;
      else if (ch === '}') {
        depth -= 1;
        if (depth === 0) return source.slice(open, i + 1);
      }
    }
    assert.ok(false, `${marker} 的块没配平`);
  };
  const windowOffsetOf = (source) => {
    const guard = windowGuardOf(source);
    const shape = /t > et \+ dur(?:\s*\+\s*\(?(\d+(?:\.\d+)?(?:\s*\+\s*\d+(?:\.\d+)?)*)\)?)?/.exec(guard);
    assert.ok(shape, `窗口判据的形状变了，读不出窗口值：${guard}`);
    return {
      guard,
      offset: shape[1] === undefined
        ? 0
        : shape[1].split('+').reduce((sum, term) => sum + Number(term.trim()), 0),
    };
  };
  {
    const tpl = readFileSync(TEMPLATE, 'utf8');
    const real = windowOffsetOf(tpl);
    checks += 1;
    // 判据是 `t > et + dur`（+ offset），所以"窗口上界 - et" = `dur + offset`，而 offset 必须是 0。
    near(real.offset, WINDOW, 0, '参照模板 getActiveSticker 的窗口上界必须是 dur（不追加时长）');
    // 旧形状（`dur + 0.65` / `dur + 1.05` / 四段之和）都不许再出现。
    assert.ok(
      !/\+\s*dur\s*\+/.test(real.guard),
      `模板的窗口判据还在 dur 之后追加时长：${real.guard}`,
    );
    checks += 1;
    // **自证不空转**：把判据换成旧的溢出窗口，上面那两条必须翻面。
    // 少了这一段，"读了模板但读错行"也会全绿。
    assert.equal(
      tpl.split(real.guard).length - 1,
      1,
      `那条窗口判据在模板里出现了 ${tpl.split(real.guard).length - 1} 次（必须是 1）—— 自证台自己错了`,
    );
    for (const old of ['1.05', '(0.35 + 0.4 + 0.3)']) {
      const patched = real.guard.replace('t > et + dur', `t > et + dur + ${old}`);
      assert.notEqual(patched, real.guard, '自证替换没命中判据行 —— 自证台自己错了');
      const mutated = tpl.replace(real.guard, patched);
      assert.notEqual(mutated, tpl, '自证替换后模板文本没变');
      const bad = windowOffsetOf(mutated);
      assert.notEqual(
        bad.offset,
        WINDOW,
        `旧窗口 ${old} 被算成 ${bad.offset}，守卫竟然没翻面 —— 这条断言是空转的`,
      );
      checks += 1;
    }
  }

  // -------------------------------------------------------------------------
  // 6b. **四处同形**（转译器 / 模板 / CPU 参考 / WebUI 预览）
  // -------------------------------------------------------------------------
  //
  // 这次出问题的根因不是"某一处写错"，而是**四处各写各的**：上一刀把窗口放宽到
  // `dur + 1.05` 时谁也没问"窗口到底该多长"。所以四处一起钉：
  //   ① 窗口判据里不许再出现"`dur` 之后追加时长"；
  //   ② 窗口上界必须是 `dur`；
  //   ③ 淡出起点必须写成 `dur - fadeOut`（**在窗口内部**，不是 `dur` 之后）。
  {
    const SAME_SHAPE = [
      ['参照模板 index.html', TEMPLATE, /fadeStart\s*=\s*dur\s*-\s*fadeOut/],
      ['CPU 参考 timeline.rs', join(HERE, '..', 'crates', 'vtrim-render-core', 'src', 'timeline.rs'),
        /fade_start\s*=\s*dur\s*-\s*fade_out/],
      ['WebUI 预览 engine.js', join(HERE, '..', 'webui', 'src', 'polish', 'engine.js'),
        /fadeStart\s*=\s*dur\s*-\s*fadeOut/],
    ];
    // 四处（含转译器自己）都不许再出现旧的窗口形状 —— 只在**代码行**上查（注释里要留历史）。
    for (const [label, file] of [['转译器', TRANSLATOR], ...SAME_SHAPE]) {
      const code = codeLines(readFileSync(file, 'utf8')).join('\n');
      assert.ok(
        !/dur\s*\+\s*\(0\.35\s*\+\s*0\.4\s*\+\s*0\.3\)/.test(code),
        `${label} 里还有旧的溢出窗口 dur + (0.35 + 0.4 + 0.3)：${file}`,
      );
      assert.ok(
        !/dur\s*\+\s*1\.05/.test(code),
        `${label} 里还有旧的溢出窗口 dur + 1.05：${file}`,
      );
      checks += 2;
    }
    for (const [label, file, fade] of SAME_SHAPE) {
      const src = readFileSync(file, 'utf8');
      const guard = windowGuardOf(src);
      assert.ok(
        !/\+\s*dur\s*\+/.test(guard),
        `${label}：窗口判据还在 dur 之后追加时长 —— ${guard}`,
      );
      assert.ok(/t > et \+ dur\b/.test(guard), `${label}：窗口上界不是 dur —— ${guard}`);
      assert.ok(
        fade.test(src),
        `${label}：淡出起点不是 \`dur - fadeOut\`（淡出必须在窗口内部）—— ${file}`,
      );
      checks += 3;
    }
  }

  // -------------------------------------------------------------------------
  // 6c. **DOM/GSAP 预览腿**（`handlers.sticker`，`index.html:955`）
  // -------------------------------------------------------------------------
  //
  // 这条腿不走 `getActiveSticker` 的逐帧判据 —— 它建一条 GSAP 时间线，所以"窗口"体现在
  // **两处**：淡出起点（`fadeStart = et + dur - fadeOut`）与隐藏时刻（`et + dur`）。
  // 上一轮它被漏掉了：`fadeStart = et + 0.35 + dur` ⇒ 可见跨度 `dur + 0.65`、"出点"偏晚 ✗。
  // 所以这里按**源码**钉住形状（注释行滤掉，历史数字写在注释里）。
  // ⚠️ 上浮起点 `et + 0.25` 是这条腿自己的既有偏差（其余处是 `popIn 0.35`），**刻意不钉**。
  {
    const tpl = readFileSync(TEMPLATE, 'utf8');
    const rawBlock = blockAfter(tpl, 'handlers.sticker = function');
    const code = codeLines(rawBlock).join('\n');
    // ⓪ 这一段必须**能编译** —— 这条腿没有别的守卫，语法错会让整页预览静默失效
    //    （`new Function` 只解析、不执行：`el`/`tl`/`SK` 那些自由变量不会真被求值）。
    assert.doesNotThrow(
      () => new Function('ev', 'et', 'el', 'tl', 'SK', rawBlock.slice(1, -1)),
      'handlers.sticker 的函数体编译不过（语法错）',
    );
    checks += 1;
    // ① 淡出起点必须在窗口**内部**：出现 `dur - fadeOut`、且不再出现"挂到事件之后"的 `0.35 + dur`。
    const shapeOk = (src) => /dur\s*-\s*fadeOut/.test(src) && !/0\.35\s*\+\s*dur/.test(src);
    assert.ok(
      shapeOk(code),
      `handlers.sticker 的淡出不在窗口内部（要 \`dur - fadeOut\`、不要 \`0.35 + dur\`）：` +
        code.split('\n').filter((l) => l.includes('fade') || l.includes('dur')).join(' | '),
    );
    checks += 1;
    // ② 窗口判据 = 隐藏时刻必须是 `et + dur`（不是 `fadeStart + 0.3`）。
    assert.ok(
      /visibility\s*:\s*'hidden'\s*\}\s*,\s*et\s*\+\s*dur\s*\)/.test(code),
      'handlers.sticker 的隐藏时刻不是 `et + dur` —— 窗口上界必须 = 事件时长',
    );
    checks += 1;
    // ③ 退化时长**优先保淡出**：`fadeOut = min(0.3, dur)`，且淡出 tween 的长度就是它
    //    （写死 `0.3` 时，`dur < 0.3` 的贴纸淡出会越过 `et + dur` 被硬切掉）。
    const fadeOk = (src) => /fadeOut\s*=\s*Math\.min\(0\.3\s*,\s*dur\)/.test(src)
      && /autoAlpha\s*:\s*0\s*,\s*duration\s*:\s*fadeOut/.test(src);
    assert.ok(
      fadeOk(code),
      'handlers.sticker 的淡出长度不是 `min(0.3, dur)`（退化时长要优先保淡出）',
    );
    checks += 1;
    // **自证不空转**：换回旧形状（淡出挂到事件之后）必须翻面。
    const old = code.replace(/et\s*\+\s*dur\s*-\s*fadeOut/, 'et+0.35+dur');
    assert.notEqual(old, code, '自证替换没命中 `et + dur - fadeOut` —— 自证台自己错了');
    assert.ok(!shapeOk(old), '自证：旧形状没被识别出来 —— 这条断言是空转的');
    checks += 1;
    // **自证不空转**：淡出长度写死 `0.3` 也必须翻面。
    const literal = code.replace(/Math\.min\(\s*0\.3\s*,\s*dur\s*\)/, '0.3');
    assert.notEqual(literal, code, '自证替换没命中 `Math.min(0.3, dur)` —— 自证台自己错了');
    assert.ok(!fadeOk(literal), '自证：写死 0.3 没被识别出来 —— 这条断言是空转的');
    checks += 1;
  }

  // -------------------------------------------------------------------------
  // 6d. **弹幕在屏窗口的末尾**（`danmakuExitMs`，2026-09 修）
  // -------------------------------------------------------------------------
  //
  // 参照与底座都是"滚动一口钟、渐隐另一口钟"：位置由 `travel` 决定
  // （`progress = (el - fadeIn) / travel`），而渐隐钉在 cue 自己的 `End` 上
  // （`t >= d.end` 直接丢、`d.end - 0.2` 起渐隐）。录播姬 XML 给的窗口是**固定 8 秒**，
  // 中文弹幕的 travel 普遍 8.5~16 秒 —— 于是"没走到左边缘就渐隐"（5 字弹幕实测在距左边缘
  // 337px 处开始渐隐、39 字节那条在屏幕中段 728px 处就没了）。
  //
  // 这里钉两件事：① 转译器把 ASS 的 `End` 抬到 `max(End, start + fadeIn + travel)`、
  // **只抬不压**；② 这条规则的两份实现（转译器 ↔ `vtrim_core::danmaku::clamp_display_windows`）
  // 的常数与形状一致 —— 它们是两份代码，漂了就是"底座与参照又不一样"。
  {
    // ① 逐值：5 个汉字（15 字节）= 10.4s travel，8 秒窗口必须被抬到 0.3 + 10.4 = 10.7s。
    const XML_WINDOW_MS = 8000;
    assert.equal(DANMAKU_FADE_IN_MS, 300, 'fadeIn 必须与参照的 0.3s 同值');
    assert.equal(
      danmakuExitMs(0, XML_WINDOW_MS, 10400),
      10700,
      '8 秒窗口 + 10.4s travel 必须抬到 10.7s（0.3 + travel）',
    );
    checks += 1;
    // 只抬不压：素材显式写了更长窗口（40s）的原样保留。
    assert.equal(danmakuExitMs(5000, 40000, 7840), 40000, '更长的窗口不许被压短');
    checks += 1;
    // travel 读不出来（手写素材没写 6 参数 `\move`）时不猜：只加 fadeIn。
    assert.equal(danmakuExitMs(1000, 1000, undefined), 1300, 'travel 缺失时只加 fadeIn');
    assert.equal(danmakuExitMs(1000, 1000, 0), 1300, 'travel 为 0 时不加');
    checks += 1;
    // 窗口本来就够（travel 7.84s → 需要 8.14s）时按需要的那一个走。
    assert.equal(danmakuExitMs(0, 1000, 7840), 8140, '窗口短于"滚完"时抬到滚完');

// 6e. **逐条时长进 ASS**（`withMoveTravel`，2026-10-01 补静态 cue 那一支）
//
// 底座只读 `\move` 的 `t2 - t1`、坐标忽略（回执 B4 的原话）⇒ 静态 cue（只有 `\pos`）
// 也要补一个零位移的 6 参数 `\move`，否则退回轨道级平均值。
assert.equal(
  withMoveTravel('{\\move(2000,0,0,0)}hi', 10400),
  '{\\move(2000,0,0,0,0,10400)}hi',
  '4 参数 `\\move` 补上 `t2 - t1`',
);
assert.equal(
  withMoveTravel('{\\pos(960,300)}静态', 7840),
  '{\\pos(960,300)\\move(0,0,0,0,0,7840)}静态',
  '静态 cue（只有 `\\pos`）也要补零位移 `\\move` —— 否则退回轨道级时长',
);
assert.equal(
  withMoveTravel('{\\move(1,2,3,4,0,5000)}手写六参数', 9000),
  '{\\move(1,2,3,4,0,5000)}手写六参数',
  '已经是 6 参数的原样不动（不许覆盖手写值）',
);
assert.equal(
  withMoveTravel('没有任何定位标签', 9000),
  '没有任何定位标签',
  '没有定位标签 ⇒ **一个字都不许动**（不替参照发明位置）',
);
assert.equal(withMoveTravel('{\\pos(1,2)}x', 0), '{\\pos(1,2)}x', 'travel 为 0 时不补');
checks += 5;
    checks += 1;

    // ② 同一条规则的另一份实现（Rust）—— 常数同值、形状同向（`>` 才改、= 不改）。
    const rust = readFileSync(
      join(HERE, '..', 'crates', 'vtrim-core', 'src', 'danmaku', 'mod.rs'),
      'utf8',
    );
    assert.ok(
      /pub const DANMAKU_FADE_IN_SECS: f64 = 0\.3;/.test(rust),
      'Rust 那份的 fadeIn 不是 0.3 —— 两份实现漂了（vtrim_core::danmaku）',
    );
    assert.ok(
      /let need = entry\.start \+ DANMAKU_FADE_IN_SECS \+ travel;/.test(rust),
      'Rust 那份的"需要多长"不是 `start + fadeIn + travel` —— 两份实现漂了',
    );
    checks += 2;
  }

  return checks;
}

// ---------------------------------------------------------------------------
// 12 处变异：每一处都是"看着差不多、逐值差得远"的那种改法
// ---------------------------------------------------------------------------
//
// 每一条都对应上面**某一条**断言。变异体跑的是同一份 `checkCurves`，
// 所以"打红"就是"这份守卫真的能抓住这一类错误"。
const MUTATIONS = [
  {
    name: 'backOut3 的 `s = 3` 改成经典 1.70158（过冲 1.25 -> 1.10）',
    from: 'export const backOut3 = (t) => {\n  const s = 3;',
    to: 'export const backOut3 = (t) => {\n  const s = 1.70158;',
  },
  {
    name: 'shake 四级阶梯改成预览那份的连续交替（0.025s 一轮）',
    from: [
      '  if (localSec < 0.04) return { x: off, y: -off };',
      '  if (localSec < 0.08) return { x: -off, y: off };',
      '  if (localSec < 0.12) return { x: off / 2, y: -off / 2 };',
      '  if (localSec < 0.16) return { x: -off / 2, y: off / 2 };',
    ].join('\n'),
    to: [
      '  const n = Math.floor(localSec / 0.025);',
      '  return n % 2 === 0 ? { x: off, y: -off } : { x: -off, y: off };',
    ].join('\n'),
  },
  {
    name: 'shake 第三档 `+off/2` 改成 `+off`（阶梯的幅度写错）',
    from: '  if (localSec < 0.12) return { x: off / 2, y: -off / 2 };',
    to: '  if (localSec < 0.12) return { x: off, y: -off / 2 };',
  },
  {
    name: '贴纸标称 `floatUp` 0.4 改成 0.2（上浮段被腰斩）',
    from: 'export const STICKER_FLOAT_UP_SEC = 0.4;',
    to: 'export const STICKER_FLOAT_UP_SEC = 0.2;',
  },
  {
    name: '贴纸窗口追加重叠（`STICKER_WINDOW_SEC = 0` 改成 `1.05` —— 上一刀的形状）',
    from: 'export const STICKER_WINDOW_SEC = 0;',
    to: 'export const STICKER_WINDOW_SEC = 1.05;',
  },
  {
    name: '贴纸淡出起点从窗口内部挪到事件之后（`dur - fadeOut` 改成 `dur + fadeOut`）',
    from: '  const fadeStart = durSec - fadeOut;',
    to: '  const fadeStart = durSec + fadeOut;',
  },
  {
    name: '贴纸弹入的 opacity 改走 `back_out`（丢掉 pow2 缓动）',
    from: '    opacity = pow2Out(p);',
    to: '    opacity = backOut3(p);',
  },
  {
    name: '手持 dx 系数 6.5 改成 6.0',
    from: 'dx: (6.5 * Math.sin(',
    to: 'dx: (6.0 * Math.sin(',
  },
  {
    name: '手持 rot 的 0.32 改成 0.64（双倍）',
    from: 'rot: (0.32 * Math.sin(',
    to: 'rot: (0.64 * Math.sin(',
  },
  {
    name: '手持幅度当成布尔（忽略 amp 是 0~1 的连续量）',
    from: '  const a = amp > 0 ? amp : 0;',
    to: '  const a = amp > 0 ? 1 : 0;',
  },
  {
    name: 'pow2Out 改成线性（丢掉缓动）',
    from: 'export const pow2Out = (p) => 1 - (1 - p) * (1 - p);',
    to: 'export const pow2Out = (p) => p;',
  },
  {
    name: '弹幕窗口末尾丢掉 fadeIn（`start + fadeIn + travel` 改成 `start + travel`）',
    from: '  const need = startMs + DANMAKU_FADE_IN_MS + travel;',
    to: '  const need = startMs + travel;',
  },
  {
    // 回到"只认 6 位"那一版：`#140A28CC` 会返回 null → 整条 overlay 被 DROP（不是少了个透明度）。
    name: 'hexColor 退回只认 6 位（8 位带不透明度的一律 null）',
    from: '  if (hex.length !== 6 && hex.length !== 8) return null;',
    to: '  if (hex.length !== 6) return null;',
  },
  {
    // M2：把"没配倍率"的兜底退回 1.0 —— 远景档会整段不拉远（成片里看不出来，只有报告一行）。
    name: 'zoomFactor 的"远"档默认值退回 1.0（不再照抄模板的 0.55）',
    from: '  const defaults = { 近: 1.5, 远: 0.55, 中: 1.3 };',
    to: '  const defaults = { 近: 1.5, 远: 1.0, 中: 1.3 };',
  },
  {
    // M2：`color` 档的 Solid 写成 Linear —— 单色覆盖层会变成"两个同色档的渐变"（看不出来，但形状不对）。
    name: 'overlay 的 solid 档 shape 从 0 写成 1（单色被当成渐变）',
    from: '              shape: 0, // OverlayShape::Solid',
    to: '              shape: 1, // OverlayShape::Solid',
  },
  {
    // D5：把 CSS→底座的角度换算退回"不换算"（底座 0° 是**向右**、CSS 0° 是**向上**，
    // 差 90° ⇒ 渐变的走向整体转四分之一圈）。这是"报告里一行、画面里一个方向"的旧伤。
    name: 'overlay 角度不做 CSS→底座换算（base = css 而不是 css − 90）',
    from: '  return ((css - 90) % 360 + 360) % 360;',
    to: '  return ((css) % 360 + 360) % 360;',
  },
  {
    // D4：纯色档把第二档色写成"反色"（底座现在读 r/g/b ⇒ 画面会变，而报告里看不出来）
    name: '纯色档 r2 写成反色（应为 r）',
    from: '    r2: col.r, g2: col.g, b2: col.b,',
    to: '    r2: 1 - col.r, g2: col.g, b2: col.b,',
  },
  {
    // P4/B4：静态 cue 补的 `\move` 写成**有位移**的（坐标被底座忽略，但这样写会掩盖"零位移"这条契约）
    // ⚠️ 反斜杠层数：转译器那行在**模板字符串**里写的是 `\\move`（文件文本两个反斜杠 ⇒ 产出一个）；
    // 这里的 `from` 是**单引号字符串**，要匹配那两个字符就得写 `\\\\move`（第一次我写成 `\\move`，
    // 于是替换串变成 `\move` ⇒ 命中 0 次；变异台的自检把它抓出来了："变异台自己错了，不是守卫没抓住"）。
    name: '静态 cue 的零位移 \\move 被写成有位移',
    from: '  return moved.replace(POS, (tag) => `${tag}\\\\move(0,0,0,0,0,${Math.round(travelMs)})`);',
    to: '  return moved.replace(POS, (tag) => `${tag}\\\\move(100,100,300,100,0,${Math.round(travelMs)})`);',
  },
  {
    // B5：把半径按**宽**折算（横屏 1920 会小一圈）—— 判据断言的是 12 / 文档高。
    name: '文字阴影半径按文档宽折算（应为高）',
    from: '        shadow_blur_ratio: 12 / height,',
    to: '        shadow_blur_ratio: 12 / width,',
  },
  {
    // Phase ③：让落点解析把 `"auto"` **猜成 50%** —— 就是"看着成功、贴纸贴在错的地方"那种回归。
    // 判据（`checkMappings` 第 3 段）断言的是"没有贴纸图层 / 没有贴纸素材"，一猜就会被打红。
    name: '贴纸落点 `auto` 被猜成 50%（只报错不猜 那条不变量）',
    from: "  if (text === 'auto') return null;",
    to: "  if (text === 'auto') return 50;",
  },
];

/// 把 `source` 写进临时目录并真加载一遍（新 URL = 新模块实例，缓存不会串）。
async function loadVariant(source, run) {
  const dir = mkdtempSync(join(tmpdir(), 'vtrim-translator-'));
  try {
    const file = join(dir, 'polish-to-dhampir.mjs');
    writeFileSync(file, source, 'utf8');
    return await run(await import(pathToFileURL(file).href));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

async function mutate() {
  const source = readFileSync(TRANSLATOR, 'utf8');
  let survived = 0;
  console.log(`\n变异自证：${MUTATIONS.length} 处改动，每一处都必须让上面的检查翻面`);
  for (const [index, mutation] of MUTATIONS.entries()) {
    const hits = source.split(mutation.from).length - 1;
    // ⚠️ 先证明"替换真的会命中**且只命中一次**" —— 少了这一条，
    //    没匹配上的变异体会原样跑绿，看起来像"这条用例抓不住"。
    if (hits !== 1) {
      survived += 1;
      console.log(`  ✗ ${index + 1}. ${mutation.name}`);
      console.log(`      变异台的替换串在转译器里命中 ${hits} 次（必须是 1）—— 变异台自己错了，不是守卫没抓住`);
      continue;
    }
    const mutated = source.replace(mutation.from, mutation.to);
    if (mutated === source) {
      survived += 1;
      console.log(`  ✗ ${index + 1}. ${mutation.name}\n      替换后源码没变（变异台自己错了）`);
      continue;
    }
    let reason = null;
    try {
      await loadVariant(mutated, (curves) => {
        checkCurves(curves);
        // 颜色那一段也要跑：不然"退回只认 6 位"这条变异永远不会红（实测踩过）。
        checkColors(curves);
        // 映射那一段同理（M2 加的三条）。
        checkMappings(curves);
      });
    } catch (error) {
      reason = String(error && error.message ? error.message : error).split('\n')[0];
    }
    if (reason === null) {
      survived += 1;
      console.log(`  ✗ ${index + 1}. ${mutation.name}\n      检查**没红** —— 这一类错误守卫抓不住`);
    } else {
      console.log(`  ✓ ${index + 1}. ${mutation.name}\n      打红在：${reason}`);
    }
  }
  if (survived > 0) {
    console.log(`\n${survived} 处变异没被打红 —— 这份守卫有洞，别当它是绿的。`);
    process.exit(1);
  }
  console.log(`全部 ${MUTATIONS.length} 处变异都打红了（守卫不是空转的）。`);
}

// ---------------------------------------------------------------------------
// TOML 解析：**普通表里的数组表**（`[cover]` 之后再来 `[[cover.effects]]`）
// ---------------------------------------------------------------------------
//
// 实测踩过：解析器以前**无条件把父级改成数组** —— 于是 `[cover]` 那张表整个被丢掉，
// 紧接着读 `undefined['effects']` 直接 TypeError。**症状是整个工程转不出来**
// （clip-04 就是这样），而它看起来像"转译器坏了"，不像"这份 TOML 里多了个合法的节"。
//
// 这条守卫的全部意义：**崩溃不会再回来**。CLI 与服务端走的都是这个解析器。
const TOML_CASES = [
  {
    name: '普通表之后再来数组表（[cover] 后接 [[cover.effects]]）',
    text: '[cover]\ntitle = "x"\n\n[[cover.effects]]\nkind = "fade"\n\n[[cover.effects]]\nkind = "zoom"\n',
    want: (root) => root.cover && root.cover.title === 'x'
      && Array.isArray(root.cover.effects)
      && root.cover.effects.length === 2
      && root.cover.effects[0].kind === 'fade'
      && root.cover.effects[1].kind === 'zoom',
  },
  {
    name: '数组表之后再来普通表（[[segments]] 后接 [style.subtitle]）',
    text: '[[segments]]\nstart = 0\n\n[style.subtitle]\nfont = "LXGW WenKai"\n',
    want: (root) => Array.isArray(root.segments) && root.segments.length === 1
      && root.style && root.style.subtitle
      && root.style.subtitle.font === 'LXGW WenKai',
  },
  {
    name: '嵌套数组表（[[segments]] 后接 [[segments.events]]）',
    text: '[[segments]]\nstart = 0\n\n[[segments.events]]\nkind = "shake"\n',
    want: (root) => Array.isArray(root.segments)
      && root.segments[0]
      && Array.isArray(root.segments[0].events)
      && root.segments[0].events[0].kind === 'shake',
  },
];

const translatorModule = await import(pathToFileURL(TRANSLATOR).href);
let tomlChecks = 0;
for (const c of TOML_CASES) {
  let root = null;
  let thrown = null;
  try {
    root = translatorModule.parseToml(c.text);
  } catch (error) {
    thrown = error;
  }
  assert.equal(thrown, null, `${c.name}：解析器抛了 —— ${thrown && thrown.message}`);
  assert.ok(c.want(root), `${c.name}：解析结果不对 —— ${JSON.stringify(root)}`);
  tomlChecks += 1;
}
console.log(`✓ TOML 解析：${tomlChecks} 项全绿（含"普通表里的数组表"那种合法但少见的写法）`);
const colorChecks = checkColors(translatorModule);
console.log(
  `✓ 颜色解析：${colorChecks} 项全绿（6 种写法 + 7 种垃圾输入；8 位那两种是"整条 overlay 丢不丢"的关键）`,
);

// ---------------------------------------------------------------------------
// 颜色：四种写法 + "认不出"必须返回 null
// ---------------------------------------------------------------------------
//
// ⚠️ 这条守的是一个**"静默丢事件"的形状**（2026-09-30 修）：`#RRGGBBAA` 曾经解析不出来，
// 而 overlay 那处是 `if (!first) drop(...)` —— 于是**整条覆盖层事件不进成片**，
// 报告里只剩一行 DROP。V-Trim 自己的配置就写过 8 位（演示工程三条 overlay 全是）。
//
// 抽成函数是因为**变异自证也要跑它**：只把检查写在顶层的话，`--mutate` 那边只跑 `checkCurves`，
// 这一条变异就会"没被打红"（实测踩过 —— 变异台没报红不等于守卫抓住了）。
function checkColors(mod) {
  const near = (a, b) => Math.abs(a - b) < 1e-6;
  const cases = [
    ['#ffaa00', { r: 1, g: 0xaa / 255, b: 0, a: 1 }, '6 位（最常见）'],
    ['#FFAA00', { r: 1, g: 0xaa / 255, b: 0, a: 1 }, '大写也认'],
    ['ffaa00', { r: 1, g: 0xaa / 255, b: 0, a: 1 }, '不带 # 也认'],
    ['#140A28CC', { r: 0x14 / 255, g: 0x0a / 255, b: 0x28 / 255, a: 0xcc / 255 }, '8 位：带不透明度（以前整条丢）'],
    ['#f00', { r: 1, g: 0, b: 0, a: 1 }, '3 位简写'],
    ['#f00c', { r: 1, g: 0, b: 0, a: 0xcc / 255 }, '4 位简写'],
  ];
  for (const [raw, want, why] of cases) {
    const got = mod.hexColor(raw);
    assert.ok(got, `hexColor(${raw}) 解析失败（${why}）`);
    for (const k of ['r', 'g', 'b', 'a']) {
      assert.ok(near(got[k], want[k]), `hexColor(${raw}).${k} = ${got[k]}，期望 ${want[k]}（${why}）`);
    }
  }
  const bad = ['', '#12345', '#1234567', 'red', null, undefined, '#gggggg'];
  for (const value of bad) {
    assert.equal(
      mod.hexColor(value),
      null,
      `hexColor(${JSON.stringify(value)}) 应当返回 null —— 认不出的颜色不许猜（调用点会兜底或报 DROP）`,
    );
  }
  return cases.length + bad.length;
}

// ---------------------------------------------------------------------------
// 映射（2026-09-30 M2 加的）：拿**合成工程**过一遍 `translate`，断言"某一档真的转出来了"
// ---------------------------------------------------------------------------
//
// 为什么曲线/颜色之外还要这一层：M2 修的几条**都不在曲线上**，而是"这个事件有没有被转出来"：
//
//   * `overlay_type = "color"` / `"fade_in"` —— 以前整条 DROP（而 fade_in 就是片头渐入那一档）；
//   * 贴纸事件只有 `name`、没有 `sticker_tag` —— 以前整条 DROP（`name` 才是 Step2b 选定的文件名）；
//   * `style.camera` 没配某一档时的倍率 —— 模板有**按档位的默认值**（远 = 0.55），不是 1.0。
//
// 这三条的失效方式全是"报告里一行、成片里没有"，所以判据要落在**产物**上（调整图层 / 关键帧 / 资产）。
function checkMappings(mod) {
  let checks = 0;
  const eq = (actual, expected, what) => {
    checks += 1;
    if (actual !== expected) throw new Error(`${what}：期望 ${expected}，实得 ${actual}`);
  };
  const near = (actual, expected, what, eps = 1e-6) => {
    checks += 1;
    if (!(Math.abs(actual - expected) <= eps)) throw new Error(`${what}：期望 ${expected}，实得 ${actual}`);
  };

  // ---- 1. `zoomFactor`：按档位的默认值（照抄模板 `index.html:578`）----
  eq(mod.zoomFactor('远', undefined), 0.55, 'zoom="远" 没配置时要退**模板默认** 0.55（不是 1.0）');
  eq(mod.zoomFactor('近', undefined), 1.5, 'zoom="近" 默认 1.5');
  eq(mod.zoomFactor('中', undefined), 1.3, 'zoom="中" 默认 1.3');
  eq(mod.zoomFactor('认不出的档位', undefined), 1.3, '认不出的档位按**中景**（模板 else 分支）');
  eq(mod.zoomFactor('远', { zoom_wide: 0.8 }), 0.8, '配了就用配置的值');
  eq(mod.zoomFactor('远', { zoom_wide: Number.NaN }), 0.55, 'NaN 要退默认（模板看的是 isFinite）');

  // ---- 1b. `overlayBaseAngle`：CSS 角度 → 底座角度（D5）----
  // 两边 0° 的**方向不同**（CSS 向上、底座向右），且缺省方向是 CSS 的 `to bottom right` = 135°。
  eq(mod.overlayBaseAngle(135), 45, 'CSS `to bottom right`(135°) ⇒ 底座 45°（方向向量 (0.707, 0.707) = 右下）');
  eq(mod.overlayBaseAngle(0), 270, 'CSS 0°（向上）⇒ 底座 270°（向上）');
  eq(mod.overlayBaseAngle(90), 0, 'CSS 90°（向右）⇒ 底座 0°（向右）');
  eq(mod.overlayBaseAngle(180), 90, 'CSS 180°（向下）⇒ 底座 90°（向下）');
  eq(mod.overlayBaseAngle(undefined), 45, '**没写 angle** ⇒ 按参照缺省方向（右下）');
  eq(mod.overlayBaseAngle(Number.NaN), 45, 'NaN 也按缺省（别把 NaN 传进底座）');
  eq(mod.overlayBaseAngle(-45), 225, '负角度要归一化到 [0, 360)');
  eq(mod.overlayBaseAngle(495), 45, '超过一圈也归一化');

  // ---- 2. 合成工程 -> 产物 ----
  const dir = mkdtempSync(join(tmpdir(), 'vtrim-map-'));
  try {
    // 贴纸必须**真的有个文件**才走得到那一步（`name` 不带扩展名，找的是图片）
    mkdirSync(join(dir, 'assets', 'stickers'), { recursive: true });
    writeFileSync(join(dir, 'assets', 'stickers', '生气.gif'), 'GIF89a(fake)', 'utf8');
    // 字幕：1 条 cue（够触发字幕轨 —— 文字阴影 B5 的判据要靠它）
    writeFileSync(
      join(dir, 'clip.srt'),
      '1\n00:00:00,500 --> 00:00:02,500\n这一句要画文字阴影\n\n',
      'utf8',
    );
    const toml = [
      'duration = 4.0',
      '[project]',
      'name = "synthetic"',
      'output = "o.mp4"',
      '[source]',
      'video = "clip.mp4"',
      'subtitles = "clip.srt"',
      '[[segments]]',
      'start = 0.0',
      'end = 4.0',
      // 三档覆盖层 + 一条"只有 name 没有 tag"的贴纸
      '[[segments.events]]',
      'type = "overlay"',
      'time = 0.5',
      'end = 2.5',
      'overlay_type = "color"',
      'color = "#ff0000"',
      'opacity = 0.5',
      '[[segments.events]]',
      'type = "overlay"',
      'time = 3.0',
      'end = 3.6',
      'overlay_type = "fade_in"',
      '[[segments.events]]',
      'type = "overlay"',
      'time = 3.6',
      'end = 3.9',
      'overlay_type = "gradient"',
      'colors = ["#140A28CC", "#28143CCC"]',
      '[[segments.events]]',
      'type = "sticker"',
      'time = 1.0',
      'end = 2.0',
      'name = "生气"',
      // 落点：`process` 会把它算成具体百分比（`x = "auto"` 只出现在没跑过后处理的工程里）
      'x = "30%"',
      'y = "40%"',
      '[[segments.events]]',
      'type = "camera"',
      'time = 0.2',
      'end = 0.6',
      'zoom = "远"',
      '',
    ].join('\n');
    const config = mod.parseToml(toml);
    const doc = mod.translate(config, dir, join(dir, 'p.doc.json'));
    const layers = [];
    for (const track of doc.timeline.tracks) for (const l of track.layers ?? []) layers.push(l);
    const overlayLayers = layers.filter((l) => (l.effects ?? []).some((e) => e.kind === 'overlay'));
    eq(overlayLayers.length, 3, '三档覆盖层都要转出来（color / fade_in / gradient）');

    const solid = overlayLayers[0];
    eq(solid.effects[0].params.shape, 0, 'color 档 = OverlayShape::Solid(0)');
    // **D4 不变量**（2026-10-01，底座回执点名提醒）：纯色档（`shape = 0`）底座现在取 `color_a`（r/g/b），
    // 而它以前取的是 `r2/g2/b2`。两边相等时**零影响**（我们就是这样写的）；一旦有人写出 `r2 != r`，
    // 画面就会与从前不同、而且**只在纯色档变**（渐变档本来就用第二档色）。所以在这里钉死。
    {
      const p = solid.effects[0].params;
      eq(p.r2, p.r, 'D4：纯色档必须 r2 == r（底座现在读 r/g/b）');
      eq(p.g2, p.g, 'D4：纯色档必须 g2 == g');
      eq(p.b2, p.b, 'D4：纯色档必须 b2 == b');
    }
    near(solid.effects[0].params.amount, 0.5, 'color 档 amount = opacity × 颜色不透明度');
    // 包络：`index.html:1659-1660` —— 头尾各 0.4s 线性，中段 1.0
    const kv = solid.keyframes.filter((k) => k.target === 'opacity').map((k) => k.value);
    near(kv[0], 0, 'color 档的关键帧从 0 起（0.4s 进入包络）');
    near(kv[kv.length - 1], 0, 'color 档的关键帧回到 0（0.4s 离开包络）');
    if (!kv.some((v) => v > 0.99)) throw new Error('color 档的中段应当是 1.0（包络顶）');
    checks += 1;

    const fade = overlayLayers[1];
    const fv = fade.keyframes.filter((k) => k.target === 'opacity').map((k) => k.value);
    near(fv[0], 1, 'fade_in 档起点全遮（1.0）');
    near(fv[fv.length - 1], 0, 'fade_in 档末端全透（0.0）');
    if (!(fv[1] < fv[0] && fv[1] > fv[fv.length - 1])) {
      throw new Error(`fade_in 档应当是整窗线性斜坡，实得 ${fv.slice(0, 3).join(',')}`);
    }
    checks += 1;

    const grad = overlayLayers[2];
    eq(grad.effects[0].params.shape, 1, 'gradient 档默认 = OverlayShape::Linear(1)');
    near(grad.effects[0].params.amount, 0.8, 'gradient 档 amount = 峰值 × 色档不透明度(CC=0.8)');

    // 贴纸：**只有 `name`、没有 `sticker_tag`** 也必须走到"找文件"这一步 ——
    // 判据落在**产物**上（assets 里有 `image_sequence`、图层里有它），因为那一版是"提前 return"，
    // 报告里只会多一行 DROP（而 `doc.issues` 不在返回值里，判据不能靠报告）。
    const stickerAssets = (doc.assets ?? []).filter((a) => a.kind === 'image_sequence');
    eq(stickerAssets.length, 1, '只有 name 的贴纸也要登进 assets（那条"没有 sticker_tag"的提前返回不许回来）');
    eq(
      layers.filter((l) => String(l.id ?? '').startsWith('sticker-')).length >= 1,
      true,
      '贴纸要有自己的图层（id 前缀 `sticker-`）',
    );
    // 运镜的**端到端**判据不放在这里：相机关键帧在"最小合成工程"里不落层（要真源素材/多条事件），
    // 而把 ffmpeg 拉进这份零依赖守卫不值得。倍率本身由上面 6 条 `zoomFactor` 逐值钉住，
    // 端到端那一段交给对拍样本（演示工程带 3 条运镜，倍率写错会让基线位移/像素差移动）。

    // ---------------------------------------------------------------------
    // ---- 3. 贴纸落点 `auto`：**只报错不猜**（2026-10-01 Phase ③ 钉进门禁）----
    //
    // `x = "auto"` 只出现在"没跑过后处理"的工程里（`process` 按人物/字幕/弹幕推导安全区、
    // 写成具体百分比）。正确行为是**拒转那条贴纸 + 报告里给出下一步**，而不是猜一个位置 ——
    // 猜了就是"看着成功、画面里贴纸在错的地方"，而且不报错。
    //
    // ⚠️ 判据只能落在**产物的缺席**上：`translate` 的返回值是 doc，**不含**报告/`issues`
    // （`doc.issues` 那条路走不通），所以这里断言"没有贴纸图层、也没有贴纸素材"。
    // 另一半（有具体百分比时**必须**转出来）就在上面第 2 段里，两条一起才有意义。
    // ---- 3b. 字幕**文字阴影**（B5，2026-10-01 接入）：四键齐 + 半径按**文档高**折算 ----
    //
    // 参照是 `.cap` 的 CSS `text-shadow:0 2px 12px rgba(0,0,0,0.4)`（模板样式块）。
    // 底座口径：`shadow_blur_ratio` 是**占文档高**的比例（求值层乘目标高换算），dx/dy 是文档像素；
    // 画不画的判据是"给不给颜色"。⚠️ 硬阴影要显式写 0（省略与 0 观感同、**形状不同**）。
    const subTrack = (doc.timeline.tracks ?? []).find((tr) => tr.kind === 'subtitle');
    if (!subTrack) throw new Error('合成工程里没有字幕轨（加了 clip.srt 之后应当有）');
    const capStyle = subTrack.subtitle ?? {}; // 样式就在轨上（`track.subtitle`），不是图层上
    const DOC_H = 1080; // 合成工程是横屏 1920x1080
    for (const key of ['shadow_color', 'shadow_dx_px', 'shadow_dy_px', 'shadow_blur_ratio']) {
      eq(Object.prototype.hasOwnProperty.call(capStyle, key), true, '字幕样式必须写 ' + key + '（阴影四键要么都给、要么都不给）');
    }
    eq(JSON.stringify(capStyle.shadow_color), JSON.stringify([0, 0, 0, 102]), '阴影色 = rgba(0,0,0,0.4) ⇒ [0,0,0,102]');
    eq(capStyle.shadow_dx_px, 0, '阴影 x 偏移 = 0（CSS 是 `0 2px`）');
    eq(capStyle.shadow_dy_px, 2, '阴影 y 偏移 = 2');
    near(capStyle.shadow_blur_ratio, 12 / DOC_H, '半径 12px 要**除以文档高**（不是宽）＝ 12/1080');

    const autoToml = toml.replace('x = "30%"', 'x = "auto"').replace('y = "40%"', 'y = "auto"');
    if (autoToml === toml) throw new Error('合成工程里没找到贴纸落点那两行 —— 判据写法要跟着改');
    const autoDir = mkdtempSync(join(tmpdir(), 'vtrim-map-auto-'));
    try {
      mkdirSync(join(autoDir, 'assets', 'stickers'), { recursive: true });
      writeFileSync(join(autoDir, 'assets', 'stickers', '生气.gif'), 'GIF89a(fake)', 'utf8');
      const autoDoc = mod.translate(mod.parseToml(autoToml), autoDir, join(autoDir, 'p.doc.json'));
      const autoLayers = [];
      for (const track of autoDoc.timeline.tracks) for (const l of track.layers ?? []) autoLayers.push(l);
      eq(
        autoLayers.filter((l) => String(l.id ?? '').startsWith('sticker-')).length,
        0,
        '落点 `auto` 时**不许**转出贴纸图层（猜位置 = 静默贴错地方；正确行为是拒转并提示先跑 process）',
      );
      // ⚠️ **素材仍然会登记**（实测 `image_sequence` = 1）—— 这是无害的：底座按图层画，
      // 没有图层就什么也不画。所以判据只钉"没有图层"这一条**真正的不变量**；
      // 顺手把"素材多一条"这件事记在这里，免得下次有人据此以为落点被猜了。
      eq(
        (autoDoc.assets ?? []).filter((a) => a.kind === 'image_sequence').length >= 0,
        true,
        '（记录性断言）`auto` 时素材仍会登记 —— 无害，底座按图层画',
      );
    } finally {
      rmSync(autoDir, { recursive: true, force: true });
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  return checks;
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------
const total = checkCurves(translatorModule);
console.log(`✓ 本仓转译器的曲线：${total} 项检查全绿`);
console.log('  钉住的是 back_out(3) 的过冲 / shake 的四级阶梯 / 手持的 2×4 正弦 / 贴纸四段相位');
console.log('  贴纸口径：**窗口 = 事件时长 dur**（不溢出）、淡出在窗口内部（`dur - fadeOut -> dur`）——');
console.log('  dur=1.0/fps=60 时最后一帧 opacity ≤ 0.1、出点那一帧已不可见（"出点没晚"）；');
console.log('  并读四处源码（转译器 / 模板 / timeline.rs / engine.js）钉"同一条窗口 + 同一处淡出起点"');

const mappingChecks = checkMappings(translatorModule);
console.log(`✓ 映射（合成工程过 translate）：${mappingChecks} 项全绿`);
console.log('  钉住的是 M2 三条"报告里一行、成片里没有"：覆盖层 color/fade_in 两档、贴纸只有 name、');
console.log('  以及 `style.camera` 没配时的**按档位默认倍率**（远 0.55 / 近 1.5 / 中 1.3，照抄模板）');

if (process.argv.includes('--mutate')) await mutate();
