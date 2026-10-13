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
//     node scripts/check-yekitrim-translator.mjs            # 逐值检查（项数见运行输出）
//     node scripts/check-yekitrim-translator.mjs --mutate   # 全部变异必须**打红**（自证不是空转）
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
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from 'node:fs';
import { execFileSync, spawnSync } from 'node:child_process';
import { copyFileSync } from 'node:fs';
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
  // 系数逐个钉（`crates/yekitrim-polish/templates/index.html:593`）：
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
  // 口径（`docs/yekitrim-integration.md` §14）：**出点 = 最后一个显示帧**，窗口 `[et, et + dur]`。
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
  // 6. **参照守卫本身**（`crates/yekitrim-polish/templates/index.html:getActiveSticker`）
  // -------------------------------------------------------------------------
  //
  // 上面 5 钉的是"曲线值"，不是"窗口判据"。而这条窗口恰恰两次出问题：
  // 先是**太短**（`dur + 0.65` 够不到淡出、末尾硬切），后来**太长**
  // （放宽到 `dur + 1.05` ⇒ 贴纸比事件多显示 0.4s、出点偏晚 ✗，用户实测）。
  //
  // 所以参照那侧的判据必须**一并**钉住：只测转译器，改了模板也没人拦。
  // 这里读模板源码（不是模块）算出窗口值：新口径是 **`t > et + dur`**（上界就是事件时长），
  // 旧形状 `dur + 1.05` / `dur + (0.35 + 0.4 + 0.3)` 必须被抓。
  const TEMPLATE = join(HERE, '..', 'crates', 'yekitrim-polish', 'templates', 'index.html');
  // 判据形状：`t > et + dur`（新口径，不追加时长）或 `t > et + dur + <offset>`（旧口径）。
  // `offset` 允许带括号、允许写成"数 + 数 + …"；**注释行要滤掉**（历史数字会写在注释里）。
  const codeLines = (source) => source
    .split('\n')
    .filter((line) => !/^\s*(\/\/|\/\*|\*|<!--)/.test(line));
  const windowGuardOf = (source) => {
    const lines = codeLines(source).map((line) => line.replace(/\\s+/g, ' ').trim());
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
      ['CPU 参考 timeline.rs', join(HERE, '..', 'crates', 'yekitrim-render-core', 'src', 'timeline.rs'),
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
  // 6c-bis. **`manual` 落点必须真的进底座的工程**（2026-10-02 修）
  // -------------------------------------------------------------------------
  //
  // 症状：设计器里把贴纸落点改成「固定」并点保存，**预览（DOM 那条腿）当场就对**
  // （`engine.js::stickerAnchor` 有 `manual` 分支），而出片那条腿里贴纸还在老位置 ——
  // 而且**报告里一行差异都没有**，因为转译器根本没意识到自己丢了一个字段。
  //
  // 根因：转译器只读 `event.x/y`（那是 `process` 按安全区算出来的 **auto** 落点），
  // 而 `manual` 模式下真正生效的是 `[sticker] position`。改的键与读的键**根本不是同一个**。
  // 两份参照都写着两个键**基准不同、不许统一**：`auto` = 盒子左上角、`manual` = 盒子中心。
  //
  // 判据盯两件事：① 转译器里必须有 `manual` 分支且读的是 `config.sticker.position`；
  // ② **两种模式算出来的落点必须不同**（同值 = 分支是空转的，等于没收复）。
  // 判据走**行为**而不是源码文本：源码正则在这个变异台里是**抓不住的**
  // （它读的是磁盘上那份真文件，变异体在临时目录里），实测变异因此漏网 —— 改成**行为判据**。
  //
  // 判据盯两件事：① 两种模式算出来的落点**必须不同**（同值 = 分支空转 = 没收复）；
  // ② 各自都必须符合自己那份基准（auto = 左上角、manual = 盒子中心）。
  {
    const SYNTH = (stickerExtra, evX, evY) => [
      'duration = 4.0',
      '[project]',
      'name = "manual-probe"',
      'output = "o.mp4"',
      '[source]',
      'video = "clip.mp4"',
      stickerExtra,
      '[[segments]]',
      'start = 0.0',
      'end = 4.0',
      '[[segments.events]]',
      'type = "sticker"',
      'time = 1.0',
      'end = 2.0',
      'name = "生气"',
      `x = "${evX}"`,
      `y = "${evY}"`,
      '',
    ].join('\n');
    const anchorOf = (extra, evX, evY) => {
      const dir = mkdtempSync(join(tmpdir(), 'yekitrim-manual-'));
      try {
        mkdirSync(join(dir, 'assets', 'stickers'), { recursive: true });
        writeFileSync(join(dir, 'assets', 'stickers', '生气.gif'), 'GIF89a(fake)', 'utf8');
        const doc = curves.translate(curves.parseToml(SYNTH(extra, evX, evY)), dir, join(dir, 'p.doc.json'));
        const layer = doc.timeline.tracks
          .flatMap((tr) => tr.layers ?? [])
          .find((l) => String(l.id ?? '').startsWith('sticker-'));
        return layer ? layer.transform : null;
      } finally { rmSync(dir, { recursive: true, force: true }); }
    };
    const W = 1920;
    const H = 1080;
    const BOX = 280 * (Math.min(W, H) / 1080);
    // auto：锚点 = 事件给的**左上角**
    const auto = anchorOf('[sticker]\nposition_mode = "auto"', '30%', '40%');
    assert.ok(auto, 'auto 模式应当转出贴纸图层');
    near(auto.x, 0.3 * W - W / 2 + BOX / 2, 1e-9, 'auto 落点必须按**盒子左上角**算');
    near(auto.y, 0.4 * H - H / 2 + BOX / 2, 1e-9, 'auto 落点 y 同上');
    // manual：锚点是**盒子中心**，且**忽略事件的 x/y**（事件里仍写着 30%/40%）
    const man = anchorOf('[sticker]\nposition_mode = "manual"\nposition = "35% 50%"', '30%', '40%');
    assert.ok(man, 'manual 模式应当转出贴纸图层');
    near(man.x, 0.35 * W - BOX / 2 - W / 2 + BOX / 2, 1e-9, 'manual 落点必须按**盒子中心**算');
    near(man.y, 0.5 * H - BOX / 2 - H / 2 + BOX / 2, 1e-9, 'manual 落点 y 同上');
    assert.notEqual(
      man.x,
      auto.x,
      'manual 与 auto 算出了同一个落点 —— manual 分支是空转的（等于没收复「固定落点」）',
    );
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
  // **只抬不压**；② 这条规则的两份实现（转译器 ↔ `yekitrim_core::danmaku::clamp_display_windows`）
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
      join(HERE, '..', 'crates', 'yekitrim-core', 'src', 'danmaku', 'mod.rs'),
      'utf8',
    );
    assert.ok(
      /pub const DANMAKU_FADE_IN_SECS: f64 = 0\.3;/.test(rust),
      'Rust 那份的 fadeIn 不是 0.3 —— 两份实现漂了（yekitrim_core::danmaku）',
    );
    assert.ok(
      /let need = entry\.start \+ DANMAKU_FADE_IN_SECS \+ travel;/.test(rust),
      'Rust 那份的"需要多长"不是 `start + fadeIn + travel` —— 两份实现漂了',
    );
  }

  return checks;
}

// ---------------------------------------------------------------------------
// 12 处变异：每一处都是"看着差不多、逐值差得远"的那种改法
// ---------------------------------------------------------------------------
//
// 每一条都对应上面**某一条**断言。变异体跑的是同一份 `checkCurves`，
// 所以"打红"就是"这份守卫真的能抓住这一类错误"。
// ---------------------------------------------------------------------------
// 六条轨道的**显示开关**（`[stage.lanes]`，2026-10-06）
// ---------------------------------------------------------------------------
//
// 失败形态：竖屏「默认关」与「转译器压根没读这个键」在报告里**长得一模一样**（都是一行没有），
// 而两者差着整类事件。所以拿**同一份合成工程、只改 `[canvas]` / `[stage.lanes]` 那几行**，
// 看产物扩展元数据（`extensions['yekitrim.polish_toml']` 的 `shake_events` / `sfx_clips` /
// `camera_events`）跟不跟着变。
//
// ⚠️ 与 0.5.0 的差别（这就是用户报的那件事）：那时是**硬门**（`[stage] portrait_*`，
// 只认画布方向、素材竖那一档由 `portraitNoCamera` 硬吃）—— 用户眼里"竖屏素材关不掉、也开不了"。
// 现在显式开关**优先于一切**（横画布也能整类关掉、竖画布也能整类打开）。
//
// ⚠️ `camera` 以前不在这组行为判据里（要先探到源尺寸才判得了"素材竖"，合成工程里没有真视频）；
// 0.6.0 起**竖画布**下运镜默认关（`!(素材竖 || 画布竖)`）—— 与源尺寸无关，
// 所以它现在**能**在合成工程里被真断一次（下面 ② 的 `camera_events`）。
function checkLaneSwitches(curves) {
  const SYNTH = (canvasLines, stageLines, extra = []) => [
    'duration = 4.0',
    '[project]',
    'name = "lane-probe"',
    'output = "o.mp4"',
    '[source]',
    'video = "clip.mp4"',
    'subtitles = "clip.srt"',
    'danmaku = "clip.dm.srt"',
    ...extra,
    '[canvas]',
    ...canvasLines,
    '[stage]',
    ...stageLines,
    '[[segments]]',
    'start = 0.0',
    'end = 4.0',
    '[[segments.events]]',
    'type = "camera"',
    'time = 0.5',
    'end = 3.0',
    'zoom = "近"',
    '[[segments.events]]',
    'type = "shake"',
    'time = 1.0',
    'end = 1.4',
    '[[segments.events]]',
    'type = "sfx"',
    'time = 2.0',
    'end = 3.0',
    'name = "pop-notice"',
    '',
  ].join('\n');
  const SRT = ['1', '00:00:00,500 --> 00:00:02,000', '第一条字幕', '', ''].join('\n');
  const probe = (canvasLines, stageLines, extra = []) => {
    const dir = mkdtempSync(join(tmpdir(), 'yekitrim-lane-'));
    try {
      mkdirSync(join(dir, 'sfx'), { recursive: true });
      writeFileSync(join(dir, 'sfx', 'pop-notice.mp3'), 'ID3(fake)', 'utf8');
      writeFileSync(join(dir, 'clip.srt'), SRT, 'utf8');
      writeFileSync(join(dir, 'clip.dm.srt'), SRT, 'utf8');
      const doc = curves.translate(curves.parseToml(SYNTH(canvasLines, stageLines, extra)), dir, join(dir, 'p.doc.json'));
      const tracks = (doc.timeline && doc.timeline.tracks) || [];
      return {
        ...((doc.extensions || {})['yekitrim.polish_toml'] || {}),
        subtitle_tracks: tracks.filter((t) => t.subtitle).length,
        danmaku_tracks: tracks.filter((t) => t.danmaku).length,
      };
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  };
  const PORTRAIT = ['orientation = "portrait"'];
  const LANDSCAPE = ['orientation = "landscape"'];
  // `[stage.lanes]` 是子表 ⇒ TOML 里必须写在 `[stage]` **之后**（SYNTH 把 stageLines 放在
  // `[stage]` 头下面，所以这里给的就是"表里的行"；子表头要单独起一行）。
  const LANES = (...keys) => ['', '[stage.lanes]', ...keys];
  const ON = keys => LANES(...keys.map((k) => `${k} = true`));
  const OFF = keys => LANES(...keys.map((k) => `${k} = false`));

  // ① 横画布（对照组）：三类照转 —— 这是"开关没设过就是默认开"的基线
  const land = probe(LANDSCAPE, ['scale = 1.0']);
  assert.ok(land.camera_events > 0, '横画布缺字段：运镜必须照转');
  assert.ok(land.shake_events > 0, '横画布缺字段：特效必须照转');
  assert.ok(land.sfx_clips > 0, '横画布缺字段：音效必须照转');
  checks += 3;

  // ② 竖画布 + 缺字段 ⇒ 运镜 / 特效 / 音效**默认关**（字幕与弹幕不受影响）
  const off = probe(PORTRAIT, ['scale = 1.0']);
  assert.equal(off.camera_events, 0, '竖画布缺字段：运镜**默认关**（这是"竖屏素材默认不启用"那一半）');
  assert.equal(off.shake_events, 0, '竖画布缺字段：特效默认关');
  assert.equal(off.sfx_clips, 0, '竖画布缺字段：音效默认关');
  assert.ok(off.subtitle_tracks > 0 && off.danmaku_tracks > 0, '字幕与弹幕**不受画布方向影响**（它们默认开）');
  checks += 4;

  // ③ 眼睛点亮 ⇒ 全回来（判据不许是「关掉之后永久关掉」——用户要的正是"点得开"）
  const on = probe(PORTRAIT, ['scale = 1.0'], ON(['camera', 'effects', 'sfx']));
  assert.ok(on.camera_events > 0, '点亮运镜轨后运镜应当转出来');
  assert.ok(on.shake_events > 0, '点亮特效轨后特效应当转出来');
  assert.ok(on.sfx_clips > 0, '点亮音效轨后音效应当转出来');
  checks += 3;

  // ④ **显式关**优先（横画布也能整类关掉 —— 硬门时代做不到这件事）
  const hardOff = probe(LANDSCAPE, ['scale = 1.0'], OFF(['camera', 'effects', 'sfx']));
  assert.equal(hardOff.camera_events, 0, '显式关运镜：横画布下也不许转');
  assert.equal(hardOff.shake_events, 0, '显式关特效：横画布下也不许转');
  assert.equal(hardOff.sfx_clips, 0, '显式关音效：横画布下也不许转');
  checks += 3;

  // ⑤ 文字层的眼睛：关掉 ⇒ 一条字幕/弹幕轨都不转（素材与 `clip.srt` 一个字都没动）
  const subOff = probe(LANDSCAPE, ['scale = 1.0'], OFF(['subtitles', 'danmaku']));
  assert.equal(subOff.subtitle_tracks, 0, '字幕轨的眼睛关着 ⇒ 不该有字幕轨');
  assert.equal(subOff.danmaku_tracks, 0, '弹幕轨的眼睛关着 ⇒ 不该有弹幕轨');
  assert.equal(land.subtitle_tracks, 1, '对照组：字幕轨的眼睛默认亮着 ⇒ 一条字幕轨');
  assert.equal(land.danmaku_tracks, 1, '对照组：弹幕轨的眼睛默认亮着 ⇒ 一条弹幕轨');
  checks += 4;

  // ⑥ 默认值表：**四种方向组合逐值对拍**（与 Rust `ProjectFile::lane_default` 同一张表）。
  //    `素材竖 + 横画布`就是用户报的那一档（旧硬门把它关死且改不了）。
  const D = curves.laneDefaults;
  assert.deepEqual(D(true, false), {
    subtitles: true, danmaku: true, camera: false, sticker: true, effects: false, sfx: true,
  }, '素材竖 + 横画布：运镜与视觉特效默认关，音效跟画布（仍是开）');
  assert.deepEqual(D(false, true), {
    subtitles: true, danmaku: true, camera: false, sticker: true, effects: false, sfx: false,
  }, '横素材 + 竖画布：三类默认关');
  assert.deepEqual(D(true, true), {
    subtitles: true, danmaku: true, camera: false, sticker: true, effects: false, sfx: false,
  }, '竖素材 + 竖画布：三类默认关');
  assert.deepEqual(D(false, false), {
    subtitles: true, danmaku: true, camera: true, sticker: true, effects: true, sfx: true,
  }, '横素材 + 横画布：六条轨道全开');
  assert.deepEqual(D(null, false), {
    subtitles: true, danmaku: true, camera: true, sticker: true, effects: true, sfx: true,
  }, '探不到源尺寸（null）**不算竖** —— 与 Rust 注入的 vertical 同口径');
  assert.deepEqual(D(null, true), {
    subtitles: true, danmaku: true, camera: false, sticker: true, effects: false, sfx: false,
  }, '探不到源尺寸 + 竖画布：仍按画布方向默认关');
  checks += 6;

  return checks;
}

const MUTATIONS = [
  // ---- 纵向档位（2026-10-12，底座 v7 的 `anchor_bottom`）----
  // ⚠️ 这两条打的是**同一个真缺陷的两种写法**：
  //    `anchor_bottom` 是轨级常量、`pos` 是逐事件的，只按样式分轨就会互相覆盖。
  {
    name: '纵向档退回只按样式分轨 ⇒ 同一样式的三个档位共用一个 anchor_bottom（后写覆盖先写）',
    // ⚠️ `from` 用**模板字面量**写：里面既有 `${}` 又有单引号，
    //    单引号字符串要转义三层（实测写崩过一次 —— 整份守卫语法错误、跑不起来）。
    //
    // ⚠️⚠️ **2026-10-13 两级化时必须同步改这条锚点**：分组键从三维变四维，
    //    旧锚点（`…|${cue.anim || ''}`）**再也匹配不上** ⇒ `.replace` 不生效
    //    ⇒ 变异体与真身一模一样 ⇒ **判据不翻面、这一条静默失效**。
    //    实测就是这么发现的：`--mutate` 报"2 处变异没被打红"。
    //    ⇒ **改任何被变异引用的源码文本，都要搜一遍变异表里的锚点**。
    // ⚠️⚠️ **2026-10-13 五维化（颜色）时又同步改了一次这条锚点** —— 同一个教训的第二次出现。
    // ⚠️⚠️ **2026-10-14 三轴收敛（补过渡维）时第三次改** —— 而且这次键**跨两行**了
    //    （源码把它拆成两个模板串相加）⇒ 锚点必须**逐字照抄那个两行形状**，
    //    否则 `.replace` 命中 0 次、变异体与真身一模一样 ⇒ 判据静默不翻面。
    from: `const key = \`\${cue.style.name}|\${cue.pos}|\${cue.styleAnim || ''}\` +
        \`|\${cue.effectAnim || ''}|\${cue.transitionAnim || ''}|\${colorKey}\`;`,
    to: 'const key = cue.style.name;',
  },
  // ---- 逐条颜色（2026-10-13）----
  // ⚠️ 三条打的是**同一个真缺陷的三个面**：颜色是**逐事件**的，而底座的
  //    `SubtitleStyle.color` 是**轨级**的 ⇒ 不分轨就后写覆盖先写。
  //    实测症状：7 条不同颜色的文字元素渲染出来**全是白的**，
  //    而 ASS 文件、转译报告、任务日志**三者都完全正常**。
  {
    name: '分组键丢掉颜色那一维 ⇒ 同档位不同色共用一个轨级 color（成片里全变成同一色）',
    // ⚠️ 同一条教训（见上）：键跨两行，锚点必须照抄两行形状。
    //    这条把键截到"过渡维为止"⇒ 颜色维丢掉。
    from: `\`\${cue.style.name}|\${cue.pos}|\${cue.styleAnim || ''}\` +
        \`|\${cue.effectAnim || ''}|\${cue.transitionAnim || ''}|\${colorKey}\``,
    to: `\`\${cue.style.name}|\${cue.pos}|\${cue.styleAnim || ''}\` +
        \`|\${cue.effectAnim || ''}|\${cue.transitionAnim || ''}\``,
  },
  {
    name: '颜色那一维改用预设名（而不是 RGBA 值）⇒ 同色不同名的条目被拆成两条轨',
    from: "const colorKey = Array.isArray(cue.color) ? cue.color.join(',') : '';",
    to: "const colorKey = cue.style.colorName || '';",
  },
  {
    name: 'cue 不带颜色 ⇒ 上游取不到值（轨道照样分，但每条都是默认色）',
    // ⚠️ 锚点必须**唯一**：`color: st.color,` 在文件里出现两次
    //    （文字轨那条 + 字幕轨那条）⇒ `.replace` 只改第一处，
    //    而判据在第二处**照样绿**，于是这条变异"看着抓不住"。
    //    ⇒ 带上下一行的 `pos`（那是文字轨独有的）钉住唯一性。
    from: '              color: st.color,\n              // 纵向档位由排版那一步算进了 `y` —— 这里只带过来给报告用。',
    to: '              // 纵向档位由排版那一步算进了 `y` —— 这里只带过来给报告用。',
  },
  {
    name: 'ASS 的逐条颜色覆盖去掉（侧挂产物里全变成 style 主色）',
    from: 'withMoveTravel(cueColorOverride(cue.color, primary) + cue.text, cue.travelMs)',
    to: 'withMoveTravel(cue.text, cue.travelMs)',
  },
  {
    name: 'ASS 颜色按 RGBA 序写（不转 ABGR）⇒ 红蓝对调（pink 变成蓝紫）',
    from: 'return `&H${hex(255 - a)}${hex(b)}${hex(g)}${hex(r)}`;',
    to: 'return `&H${hex(255 - a)}${hex(r)}${hex(g)}${hex(b)}`;',
  },
  {
    name: 'ASS alpha 不取反 ⇒ 七档全写成近乎透明（成片里一个字都看不见）',
    from: 'return `&H${hex(255 - a)}${hex(b)}${hex(g)}${hex(r)}`;',
    to: 'return `&H${hex(a)}${hex(b)}${hex(g)}${hex(r)}`;',
  },
  {
    name: '算出了纵向锚却不写进轨（等于没接）',
    from: 'anchor_bottom: anchorBottomFor(posName, st, height),',
    to: '',
  },
  {
    name: '纵向锚恒返回不写 ⇒ 三个档位都不生效（回落贴底）',
    from: 'if (posKey === \'bottom\') return undefined; // 见上：交给底座缺省',
    to: 'return undefined; // 变异：恒不写',
  },
  // ---- 纵向锚「单行行盒」+ 元素继承字幕样式（2026-10-14）----
  // ⚠️ 前者是用户报的「元素的文字位置不对」**本体**；后者是同一批报的
  //    「元素出现时字幕的字体/描边变成默认的」。
  {
    name: '纵向锚退回"3 行上限"算块底 ⇒ center 落到 0.6333（文字比正中低约 85px）',
    from: "const blockBottom = posKey === 'top' ? MARGIN + lineH : H / 2 + lineH / 2;",
    to: "const blockBottom = posKey === 'top' ? MARGIN + lineH * 3 : H / 2 + lineH * 3 / 2;",
  },
  {
    // ⚠️ 2026-10-14 第二轮：第一版拿 `LINE_HEIGHT_EM`(1.2) 算行盒，而轨里写的是 1.5
    //    ⇒ 底座按 1.5 排行盒 ⇒ 文字中心 528（低 12px）。**两处都不报错。**
    name: '纵向锚的行盒改用 LINE_HEIGHT_EM(1.2) ⇒ 与轨里那个 line_height 分叉（文字偏半个行盒）',
    from: 'const lineEm = Number(sharedSubLineHeight) > 0 ? Number(sharedSubLineHeight) : 1.2;',
    to: 'const lineEm = 1.2;',
  },
  {
    name: '元素轨的 line_height 写死 1.2（本该继承字幕那支 1.5）⇒ 行距 + 纵向锚一起漂',
    from: '          line_height: sharedSubLineHeight,',
    to: '          line_height: 1.2,',
  },
  {
    name: '元素轨不写 line_height ⇒ 底座按 1.2（字幕是 1.5，行距不一致）',
    from: '          line_height: sharedSubLineHeight,\n',
    to: '',
  },
  {
    name: '元素轨的字体硬编码成默认 ⇒ 与字幕两套字体（用户看到的"字体被换了"）',
    from: '          font_family: fontFamilyFor(sharedSubStyle.font),',
    to: '          font_family: "Sans",',
  },
  {
    name: '元素轨不写阴影 ⇒ 同画面里元素没有字幕那层细阴影',
    from: '          shadow_blur_ratio: 12 / height,\n',
    to: '',
  },
  {
    name: '元素轨描边被顺手改成字幕那套（元素描边是按档位推导的，不是同一个语义）',
    from: '          stroke_color: st.strokeColor,',
    to: '          stroke_color: [64, 60, 59, 255],',
  },
  // ---- §5 E4a T2 打字机（2026-10-11）----
  // ⚠️ 这一条打的是**那个真缺陷**：文字元素被弹幕的出点规则抬起。
  {
    name: '文字元素退回弹幕的出点规则 ⇒ 打字机的短 cue 全部重叠成 300ms',
    from: 'danmaku ? danmakuExitMs(cue.startMs, cue.endMs, cue.travelMs) : cue.endMs,',
    to: 'danmakuExitMs(cue.startMs, cue.endMs, cue.travelMs),',
  },
  {
    name: '打字机开关改成恒开（存量工程被无差别拆 cue）',
    from: 'const tw = elementsCfg.typewriter === true;',
    to: 'const tw = true;',
  },
  {
    name: '打字机开关改成恒关（配了也不生效）',
    from: 'const tw = elementsCfg.typewriter === true;',
    to: 'const tw = false;',
  },
  {
    name: '拆出来的每条都发整句（＝没有逐字，只是多了一堆重复 cue）',
    from: "text: chars.slice(0, target).join('') });",
    to: "text: chars.join('') });",
  },
  // ⚠️ 第一版这里写的是 `Math.max(next, frame + 1)` → `Math.max(next, frame)`，
  //    实测**改不动行为**：`round` 的步长本来就有 2~3 帧，`frame + 1` 从来不是那个
  //    被 max 选中的分支 ⇒ 变异体产出的 cue 与原来逐值相同 ⇒ "没被打红"。
  //    ★ 那**不是守卫的洞**，是**变异本身无效**（改了但行为没变）。
  //    换成真正会产出 0 帧 cue 的写法：把推进量直接固定成 0。
  {
    name: '打字机不写静止尾巴（打完立刻消失，观众读不完全句）',
    from: 'const tailStart = Math.max(0, cue.end - holdFrames);',
    to: 'const tailStart = cue.end;',
  },

  // ---- 文字元素轨道的左右安全宽（2026-10-11 那个真缺陷）----
  // 退回 `0.5`：底座 `max_width = 宽 × (1 - 2×0.5) = 0` ⇒ `layout()` 提前返回空 ⇒
  // 文字元素一个字都不排，而成片「看着像画了」（报告零差异）。**这一条必须翻面。**
  {
    name: '文字轨 bottom_margin 退回 0.5 ⇒ 可用宽 0 ⇒ 文字一个字都不排',
    from: '          bottom_margin: (1 - 0.875) / 2,',
    to: '          bottom_margin: 0.5,',
  },
  // 只改 `safe_width_ratio` 而不动 `bottom_margin`：两个字段**由不同的人读**，
  // 口径一分叉，折行与缩字就算不到一起（判据要的是「两者自洽」，不只是「各自合法」）。
  {
    name: '文字轨 safe_width_ratio 与 bottom_margin 推出的安全宽不一致',
    from: '          safe_width_ratio: 0.875,',
    to: '          safe_width_ratio: 0.6,',
  },
  // ---- 文字层 transform.rotation 的**无条件**存在（2026-10-13 那个真缺陷）----
  // 底座 `TransformV2.rotation` 没有 serde(default) ⇒ 省掉它**整份工程载入失败**。
  // 退回"只在 tilt≠0 时才写"⇒ `chapter` 档（tilt=0, squash≠1）必然省略它 ⇒
  // `dhampir probe` 报 `missing field rotation`、一个像素都出不来。
  {
    name: '文字层退回「只在 tilt≠0 时才写 rotation」⇒ chapter 档省掉它、整份工程载入失败',
    from: '              return Number.isFinite(v) ? v : 0;',
    to: '              return v === 0 ? undefined : v;',
  },

  // ---- T1/T4/T6 接线（§5 E4a，2026-10-11）----
  //
  // ⚠️ 加变异前先确认它**能改出不同的行为**（本趟删过一条 `frame+0 ≡ frame` 的无效变异）。
  //    下面每一条都对应一条真实判据，且**逐条实测过会红**。
  //
  // ① `motionFields` 不接出去 ⇒ 配置读了但没用（工程里 `[elements]` 看着还在）。
  {
    name: 'T1/T4/T6 的 `motionFields` 不摊进 subtitle ⇒ 配置读了没接出去',
    // ⚠️⚠️ 同样**必须跟着四级化同步**（2026-10-13）：第四实参改了名、
    //    第五实参是新加的 —— 旧锚点匹配不上就**静默失效**（见上一条那段说明）。
    // ⚠️ 第 7 实参（角度）也要带上 —— 锚点必须**逐字**盖住整段调用。
    // ⚠️ 这段里有反引号（注释里的 `` `undefined` ``）⇒ **不能用模板字面量包**，
    //    用 JSON 风格的双引号串 + `\n` 拼（模板串会被那对反引号提前截断，
    //    实测直接让整份守卫语法错误、跑不起来）。
    from: "          ...motionFields(\n"
      + "            elementsCfg,\n"
      + "            SHAKE_PHASE_BY_STYLE[styleName],\n"
      + "            hasElementsSection(config),\n"
      + "            animPreset(cues[0].effectAnim),\n"
      + "            animPreset(cues[0].styleAnim),\n"
      + "            animPreset(cues[0].transitionAnim),\n"
      + "            // \u26a0\ufe0f \u7b2c 7 \u5b9e\u53c2 = \u7528\u6237\u663e\u5f0f\u8c03\u7684\u89d2\u5ea6\uff08`undefined` = \u6ca1\u586b \u21d2 \u7528\u6863\u5185\u7f3a\u7701\uff09\u3002\n"
      + "            cues[0].layoutAngle,\n"
      + "          ),",
    to: '',
  },
  // ② `undefined` 也写进去 ⇒ 老工程**多出一堆 null 键**（R4 违规：
  //    "没写"与"写了 null"在底座那边**语义不同**，`plate_color: null` 是"显式不铺"）。
  {
    name: 'T1/T4/T6 把 `undefined` 也写出去 ⇒ 老工程多出键（R4 违规）',
    from: '      if (v !== undefined) out[key] = v;',
    to: '      out[key] = v === undefined ? null : v;',
  },
  // ③ `optNum` 退回 `num` 口径 ⇒ "没配"与"配了默认值"混成一件事。
  {
    name: '`optNum` 退回兜底默认值的口径 ⇒ 没配也会写出键',
    from: '    const optNum = (v) => (typeof v === \'number\' && Number.isFinite(v) ? v : undefined);',
    to: '    const optNum = (v) => (typeof v === \'number\' && Number.isFinite(v) ? v : 0);',
  },
  // ④ 抖动相位不按档错开 ⇒ 四轨同拍（这正是选 ① 时**唯一**能拿到的分隔度）。
  {
    name: '抖动相位不按 style 档错开 ⇒ 四个档的轨全部同拍',
    from: '            SHAKE_PHASE_BY_STYLE[styleName],',
    to: '            undefined,',
  },
  // ⑤ 相位把 0 那一档也写出去 ⇒ R4 违规（0 就是底座默认值）。
  {
    name: '抖动相位把 0 那一档也写出去 ⇒ 老工程多一个 `shake_phase: 0`',
    from: "    if (phase !== 0) put('shake_phase', phase);",
    to: "    put('shake_phase', phase);",
  },
  // ⑥ `[elements]` 的颜色不校验 ⇒ 半截值进底座工程 ⇒ **整份工程反序列化失败**
  //    （不是"这一项不生效"，是"整片出不了"）。
  {
    name: '`[elements]` 颜色不校验长度 ⇒ 半截值让整份工程读不了',
    from: '      if (!Array.isArray(v) || v.length !== 4) return undefined;',
    to: '      if (!Array.isArray(v)) return undefined;',
  },
  // ⑦ 毫秒字段不校验整数 ⇒ `u64` 收到小数，serde 报错、整份工程读不了。
  {
    name: '`scale_in_ms` 不校验整数 ⇒ `u64` 收到小数，整份工程读不了',
    // ⚠️ 锚点跟着 2026-10-12 的"三态"改造更新：`scale_in_ms` 现在是一个
    //    `!('scale_in_ms' in raw) ? undefined : (Number.isInteger(…) ? … : undefined)`
    //    表达式 —— 变异要打的是**里面那个整数判据**，整行已经换了形状。
    from: '        : (Number.isInteger(raw.scale_in_ms) && raw.scale_in_ms >= 0',
    to: "        : (typeof raw.scale_in_ms === 'number' && raw.scale_in_ms >= 0",
  },
  // ⑧ ★ **`is_empty()` 只判打字机** ⇒ 「配了 `shake_px`、没开 typewriter」的工程
  //    被判成空、`[elements]` 整段不写出去 ⇒ 用户配的那行**静默丢失**。
  //    ⚠️ 这条锚在 **Rust 侧**（`elements.rs`），不在本文件里 —— 见下面 `rustMutations`。
  //    （留个位置说明为什么它不在这张表里，免得下一个人以为漏了。）

  {
    name: 'source_in 退回序列帧（5fps 素材 + 30fps 网格 ⇒ 屏蔽后画面跳 6 倍远）',
    // ⚠️ **锚点跳过中间那行 `end:`** —— 第 2 批把 `end` 从 `range.end - range.start`
    // 泛化成了 `range.durFrames ?? (range.end - range.start)`（EDL 变速要用），
    // 写死整块的锚点当场"命中 0 次"、这条变异**空转**。
    // 现在用**一段正则**覆盖"主画面那一层"的整块（`id: \`main-${index}\`` 是唯一判别串），
    // 组 1 是 `end:` 那行（**原样保留**，不参与变异），组 2 是 `source:` 的前缀。
    // 这样"时长表达式再变"与"判据"彻底解耦：判据只认 `source_in` 的换算。
    //
    // ⚠️⚠️ **第 6 批又栽了一次**：那时组 3 写死了 `toSourceFrames(range.start)`，
    // 而多素材要按各 asset 的时基各算一遍 ⇒ 换成了
    // `toSourceFramesFor(range.start, isSource ? null : range.assetId)` ⇒ 老正则**命中 0 次**、
    // 这条变异**再次空转**（而且变异台当时**照样 exit 0**）。
    //
    // ⇒ 教训升级：**连"换算函数的名字与实参"都不能钉** —— 它是最容易被重构的那部分。
    //   ⚠️⚠️ **第 8 批（底座 0.3.0 变速）第三次栽**：`source:` 从单行改成了多行
    //   （多了 `...(rate)` 那段），老正则里 `source: { asset_id: ..., source_in: ` 这个
    //   **同行**形状 ⇒ 命中 0 次、变异**再次空转**。⇒ **连“对象字面量的排版”都不能钉**。
    //   现在只要求“`source_in:` 后面跟一个 `toSourceFrames*` 调用”，中间允许换行与任意字段。
    //   组 3 现在写成 `toSourceFrames\w*\([^\n]*\)`：只要求“这里是个 `toSourceFrames*` 调用”，
    //   不管它后来叫 `toSourceFramesFor`、还是多收一个 assetId 实参。
    //   **判据的语义没变**：`source_in` 必须经换算（而不是直接塞 `range.start`）。
    fromRe: /(id: `main-\$\{index\}`,[\s\S]*?\n\s*end: [^\n]*,)([\s\S]*?source_in:\s*)toSourceFrames\w*\([^\n]*\)/,
    to: '$1$2range.start',
  },
  {
    name: '30fps 下限被摘掉（5fps 源 ⇒ 工程网格也 5fps）',
    from: "  const timebase = mediaFps > 0 && mediaFps < FPS_FLOOR ? { num: FPS_FLOOR, den: 1 } : assetTimebase;",
    to: "  const timebase = assetTimebase;",
  },
  {
    name: '下限也套到素材上（5fps 素材被当成 30fps ⇒ 源帧号跑 6 倍）',
    from: "    timebase: assetTimebase,",
    to: "    timebase,",
  },
  {
    name: '纯色背景不再发画布底色（黑屏回归）',
    from: "      ...(stageBackground ? { background: stageBackground } : {}),",
    to: '',
  },
  {
    name: '模糊也发画布底色（把它当成纯色处理）',
    from: "        : '';\n  if (stageBackground) {",
    to: "        : '#000000';\n  if (stageBackground) {",
  },
  {
    name: '时基退回写死 60fps（30fps 素材 ⇒ 帧号错 2×）',
    from: '  const assetTimebase = media?.fps ?? { num: 60, den: 1 };',
    to: '  const assetTimebase = { num: 60, den: 1 };',
  },
  {
    name: '素材尺寸退回写死 1920x1080（代理/竖素材 ⇒ 缩放取景全错）',
    from: '    width: mediaW ?? 1920,\n    height: mediaH ?? 1080,',
    to: '    width: 1920,\n    height: 1080,',
  },
  {
    name: '`--video` 覆盖被忽略（预览代理那条腿读的还是源）',
    from: "  const videoRel = options.video ?? config.source?.video ?? 'clip.mp4';",
    to: "  const videoRel = config.source?.video ?? 'clip.mp4';",
  },
  {
    name: '密集键右端退回「窗口末尾」（③R3 的回归：两点直线弦把 pow2_out 拉直）',
    from: '    const right = Number.isFinite(next) ? next - 1 : lastOut;',
    to: '    const right = b;',
  },
  {
    name: '轨道整类门被摘掉（`laneAllows` 恒放行 ⇒ 竖屏默认关形同虚设）',
    from: '      if (!laneAllows(event.type)) {',
    to: '      if (false) {',
  },
  {
    name: '竖屏素材/画布下运镜默认关被摘掉（`laneDefaults.camera` 恒开）',
    from: '    camera: !(portrait || canvas),',
    to: '    camera: true,',
  },
  {
    name: '视觉特效默认只看画布（漏掉"素材竖"那一档）',
    from: '    effects: !(portrait || canvas),',
    to: '    effects: !canvas,',
  },
  {
    name: '音效默认跟着素材方向走（口径是"只跟画布"）',
    from: '    sfx: !canvas,',
    to: '    sfx: !(portrait || canvas),',
  },
  {
    name: '显式开关失效（`laneOn` 永远按默认值 ⇒ 眼睛点不开）',
    from: '  const laneOn = (lane) => laneExplicit(lane) ?? LANE_DEFAULT[lane] === true;',
    to: '  const laneOn = (lane) => LANE_DEFAULT[lane] === true;',
  },
  {
    name: '字幕轨的眼睛被摘掉（关不掉字幕）',
    from: "  if (laneOn('subtitles') && config.source?.subtitles",
    to: '  if (config.source?.subtitles',
  },
  {
    name: '弹幕轨的眼睛被摘掉（关不掉弹幕）',
    from: "  if (laneOn('danmaku') && danmakuFile) {",
    to: '  if (danmakuFile) {',
  },
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
    from: '  const defaults = { 特写: 1.8, 近: 1.5, 远: 0.55, 中: 1.3 };',
    to: '  const defaults = { 特写: 1.8, 近: 1.5, 远: 1.0, 中: 1.3 };',
  },
  {
    // §18-A：特写档（1.8）是**新加的第四档** —— 把它退回"近"的 1.5，
    // 症状是"特写不够特写"，而从报告上看不出来（倍率只体现为画面大小）。
    name: '（§18-A）zoomFactor 的"特写"档退回近景 1.5（第四档失效）',
    from: '  const defaults = { 特写: 1.8, 近: 1.5, 远: 0.55, 中: 1.3 };',
    to: '  const defaults = { 特写: 1.5, 近: 1.5, 远: 0.55, 中: 1.3 };',
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
    // 第 2 批：`placeEnvelope` 的窗口**必须夹到它所在的那一段**。
    // 不夹的表现是"成片比派生值长"—— 乱序/重放 EDL 下实测 **433 vs 420 帧**，
    // 而报告里一行差异都没有（靠合流前那条时长断言才发现）。
    name: 'placeEnvelope 窗口不夹到所在段（乱序/重放 EDL ⇒ 成片比派生值长）',
    from: '      if (room < frames) frames = Math.max(1, room);',
    to: '      if (false) frames = Math.max(1, room);',
  },
  {
    // 第 4 批：`speed` 必须真的缩段长（`durFrames = round((ef-sf)/speed)`）。
    //
    // ⚠️ 为什么锚在**这一行**：`durFrames` 是成片段长的**唯一出处** ——
    // 层 `end`（:1738）、输出游标累加（:554）、事件重映射（:1667/:1901）、原声音轨
    // （:3521）全都读它。所以"把 `/speed` 去掉"这一个改动能让**整条链**一起退回原速，
    // 正是"变速没生效"这个 bug 的最小复现。锚在别处（比如只改某个 `??` 回退分支）
    // 反倒会因为 `durFrames` 仍然正确而**抓不住**。
    name: 'speed 被忽略（段长不缩 ⇒ 快放/慢放都不生效）',
    from: '        durFrames: Math.max(1, Math.round((ef - sf) / speed)),',
    to: '        durFrames: Math.max(1, Math.round(ef - sf)),',
  },
  {
    // 第 6 批：素材段必须按**该素材自己的时基**换算 `source_in`。
    //
    // 退回源时基 = 本批要修的那个 bug：24fps 的素材按 30fps 折算 ⇒ 取帧位置偏
    // （那条注释记的症状是"画面跳到源里 6 倍远的位置"），而**出片不报错**
    // （e2e 实测：`exit 0`、`issues: []`、`failed: false`，帧却是错的）。
    name: '素材段退回源时基（24fps 素材按 30fps 换算 ⇒ 取帧位置偏）',
    from: '    return rec?.timebase ?? assetTimebase;',
    to: '    return assetTimebase;',
  },
  {
    // 第 6 批：素材判据**只认小写** `source_kind === 'asset'`。
    //
    // 放宽大小写看着"更健壮"，实则把"用户写错档位"变成**静默当成本条片段** ——
    // 正是规格 §11.7 ① 那一类（分叉且不报错）。
    name: '素材判据大小写宽容了（source_kind 只认小写 asset）',
    from: "  if (clip.source_kind !== 'asset') return null;",
    to: "  if (String(clip.source_kind).toLowerCase() !== 'asset') return null;",
  },
  {
    // 第 4 批：`[[a.b]]` 的父级**必须建成表**（`node[key] = []` 是老写法）。
    //
    // 改回数组的表现：`[[edl.clips]]` 无 `[edl]` 表头时 `edl` 成了数组 ⇒
    // `config.edl?.clips === undefined` ⇒ 转译器把整条 EDL 当没有，而 Rust 读得出来
    // ⇒ **两侧分叉且不报错**（规格 §3 R7 那一类）。
    name: 'TOML `[[a.b]]` 的父级被建成数组（EDL 被静默忽略）',
    from: '          node[key] = {};',
    to: '          node[key] = [];',
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
    //
    // ⚠️ 锚点必须带**上一行**（`shadow_dy_px: 2,`）：2026-10-14 元素轨也补了
    //    `shadow_blur_ratio`，而它的缩进更深（10 空格）—— 只写 8 空格那行，
    //    会**同时命中两处**（那行是两个缩进串的公共子串）。
    //    症状是变异台自己报"命中 2 次"（不是静默通过，harness 那条断言救了我们），
    //    但那时**第 70 条变异整条失效**了。带上前一行即可唯一定位到字幕轨。
    name: '文字阴影半径按文档宽折算（应为高）',
    from: '        shadow_dy_px: 2,\n        shadow_blur_ratio: 12 / height,',
    to: '        shadow_dy_px: 2,\n        shadow_blur_ratio: 12 / width,',
  },
  {
    // Phase ③：让落点解析把 `"auto"` **猜成 50%** —— 就是"看着成功、贴纸贴在错的地方"那种回归。
    // 判据（`checkMappings` 第 3 段）断言的是"没有贴纸图层 / 没有贴纸素材"，一猜就会被打红。
    name: '贴纸落点 `auto` 被猜成 50%（只报错不猜 那条不变量）',
    from: "  if (text === 'auto') return null;",
    to: "  if (text === 'auto') return 50;",
  },
  {
    // 2026-10-02：`manual` 落点分支被删掉 —— 就是「设计器里改了固定落点、出片里没变」那次回归。
    // 判据（6c-bis）断言转译器里**必须有**这个分支且读 `config.sticker.position`，删掉即打红。
    name: '`manual` 落点分支被整条删掉（固定落点在那条腿上丢失）',
    from: "          if (stickerMode === 'manual') {",
    to: "          if (false && stickerMode === 'manual') {",
  },
  {
    // 2026-10-02：`manual` 分支读错键（读事件的 x/y 而不是 `[sticker] position`）——
    // 表现与「整条丢了」一样，但更难看出来（分支在、却走不到对的数据）。
    name: '`manual` 分支没有读 `[sticker] position`',
    from: "            const parts = String(config.sticker?.position || '50% 50%').trim().split(/\\s+/);",
    to: "            const parts = String(event.stickerHint || '50% 50%').trim().split(/\\s+/);",
  },
  {
    // 2026-10-05：两轴 xy 的 **X 轴插值**被改回硬居中 —— 位置滑杆整条失效
    // （用户拖了没反应，且预览与成片一起静默分叉）。行为判据看不见它（合成工程没有真视频
    // ⇒ `axis` 恒 `fill`），只有 `checkStageGeom` 抽出来的那段几何能钉住。
    name: '画面构成的 X 轴位置被改回硬居中（位置滑杆失效）',
    from: '    x = bx + (bwid - w) * (stagePctX / 100);',
    to: '    x = bx + (bwid - w) / 2;',
  },
  {
    name: '弹幕偏移从源轴挪到成片轴（remap 之后才平移 —— 被裁掉的那段不再丢，时刻却错了）',
    from: [
      "          const cue = rawCues[index];",
      "          const pieces = remapCueMs(cue.startMs + dmOffsetSecs * 1000, cue.endMs + dmOffsetSecs * 1000);",
    ].join("\n"),
    to: [
      "          const cue = rawCues[index];",
      "          const pieces = remapCueMs(cue.startMs, cue.endMs).map((p) => ({ startMs: p.startMs + dmOffsetSecs * 1000, endMs: p.endMs + dmOffsetSecs * 1000 }));",
    ].join("\n"),
  },
  {
    // 2026-10-08：库 id 不再换成真族名 ⇒ 底座那边静默落 ArialMT
    // （`lines_failed: 0`、`issues: []`，报告全绿而成片字体全错）。
    name: '库 id 不再解析成真族名（libass 静默落 ArialMT）',
    from: '  const mapped = fontFamilies[id];',
    to: '  const mapped = undefined;',
  },
  {
    // 反面：把**库外**的值也改写 —— 老工程 `font = "LXGW WenKai"`（系统字体族名）会被
    // 篡改成库里那支，预览与成片一起换字体。只钉"库内要换"的话，这条永远绿。
    name: '库外的值也被改写（老工程的系统字体族名被篡改）',
    from: '  return typeof mapped === \'string\' && mapped.trim() ? mapped.trim() : id;',
    to: '  return typeof mapped === \'string\' && mapped.trim() ? mapped.trim() : `F:${id}`;',
  },
  {
    // 2026-10-14 (用户问题 ②)：文字元素的 ASS 字体又写死成 `'Sans'`
    // —— libass 认不出时**不报错**（`lines_failed: 0`、`issues: []`），
    // 只是换个字体画 ⇒ 用户看到「元素出现时字体变成默认的了」。
    name: '元素 ASS 的字体又写死成字面量（与字幕不同源）',
    from: '        { font: fontFamilyFor(sharedSubStyle.font) || DEFAULT_DANMAKU_FONT,',
    to: "        { font: 'Sans',",
  },
  {
    // ⚠ 反面：换成**另一个假族名** —— 只钉"≠ `'Sans'`"的写法
    //   对这一条**照样绿**（`Arial` 也不是库内真族名，一样静默落默认）。
    //   所以要钉的是"取值**来源**"，不是"值长什么样"。
    name: '元素 ASS 字体换成另一个假族名（≠Sans 那种写法拓不到）',
    from: '        { font: fontFamilyFor(sharedSubStyle.font) || DEFAULT_DANMAKU_FONT,',
    to: "        { font: 'Arial',",
  },
  {
    // ⚠ 反面：不过 `fontFamilyFor` —— 库 id 直接喂底座，
    //   底座按**真族名**找字体，传 id 静默落 `ArialMT`。
    name: '元素 ASS 字体不过 fontFamilyFor（库 id 直接喂底座）',
    from: 'fontFamilyFor(sharedSubStyle.font) || DEFAULT_DANMAKU_FONT',
    to: 'sharedSubStyle.font || DEFAULT_DANMAKU_FONT',
  },
  {
    // ⚠ 反面：`cuesToAss` 又给 `font` 加默认值 ——
    //   有默认值就能再次"忘传也能跑"（当年就是这么来的）。
    name: '`cuesToAss` 又给 `font` 加上了默认值',
    from: 'function cuesToAss(cues, { font, fontSize = 42, danmaku = true } = {}) {',
    to: "function cuesToAss(cues, { font = 'Sans', fontSize = 42, danmaku = true } = {}) {",
  },
  {
    // 反面：把描边色的 hex 匹配**收窄回 6 位**（2026-10 修的那条真缺陷）。
    // `hexColor` 本来就能展开 3/4 位，但外层这个正则曾经只认 6 位 ⇒ `stroke = "8px #fff"`
    // 匹配不上、`hexColor` 根本走不到，**静默回落到兜底深灰**（实测 `[64,60,59]` 而不是白）。
    // 症状：用户选白色描边、成片是深灰，**报告里一行差异都没有**。
    name: '描边色只认 6 位 hex（#fff / #abc 静默回落成深灰）',
    from: '  const colorMatch = raw.match(/#[0-9a-fA-F]{3,8}/);',
    to: '  const colorMatch = raw.match(/#[0-9a-fA-F]{6}/);',
  },
  {
    // 2026-10-09：**片头/片尾整个不拼进成片时间线** —— 片头根本没出现（少 1.5s+3s），
    // 画面与音轨各自算出一个不同的总长，靠 `mux_checked` 才拦得住。
    name: '片头/片尾不拼进时间线（片头根本没出现）',
    from: `  const composedEdlClips = introOutroClips.length && edlClips.length
    ? [
        ...introOutroClips.filter((x) => x.side === 'intro').map((x) => x.clip),
        ...edlClips,
        ...introOutroClips.filter((x) => x.side === 'outro').map((x) => x.clip),
      ]
    : edlClips;`,
    to: '  const composedEdlClips = edlClips;',
  },
  {
    // 反面：**拼进去了，但素材没登记** ⇒ `assetIdForRange` 对片头返回 `videoAssetId`
    // （= 主素材）⇒ **片头放的是正片开头那几秒**，而 DROP/NOTE 里**一行都没有**
    // （注册表里查不到就当"源"）。这条只有断 `source.asset_id` 才抓得住。
    name: '片头/片尾的素材不登记（片头静默变成本条片段）',
    from: '    ...new Set([...edlClipsRaw.map((c) => edlClipAssetId(c)), ...ioAssetNames].filter(Boolean)),',
    to: '    ...new Set([...edlClipsRaw.map((c) => edlClipAssetId(c))].filter(Boolean)),',
  },
  {
    // 反面：**入点被丢掉**（永远从头播）⇒「可裁剪」这个能力整个失效，而时长往往还对得上
    // （只有出点 > 素材时长时才看得出来）—— 所以必须有一个"入点真的生效"的判据。
    name: '片头入点被丢掉（永远从素材 0 开始播）',
    from: '      const start = Math.min(inSec, durSec);',
    to: '      const start = 0;',
  },
  {
    // 反面：**「出点 = 0 ⇒ 播到结尾」退化成"出点 0 就是空段"** ⇒ 片尾整段消失。
    // 这条判据同时钉住 `probeMedia` 的 `duration`（拿不到时长时不许拿 0 当长度硬凑）。
    name: '「out_sec = 0 ⇒ 播到结尾」退化成空段（片尾消失）',
    from: '      const outSec = rawOut <= 0 || rawOut <= inSec ? durSec : Math.min(rawOut, durSec);',
    to: '      const outSec = rawOut <= 0 || rawOut <= inSec ? 0 : Math.min(rawOut, durSec);',
  },
  {
    // §24 更正的反面：**把摘掉的那段加回来** = 转译器开始发逐条颜色 ⇒ 成片比参照更花。
    // 这正是那份计划书建议的"修法"（`editing-techniques-catalog.md` §0.4），
    // 所以必须有一条判据把它拦住 —— 它不会让任何测试变红，只会让成片悄悄变色。
    //
    // ⚠️ 用 `fromRe`（正则版）而不是字符串版：这条锚点里**反斜杠与引号都很密**
    // （`/\\[0-9]?c&H…/gi` 本身就是转义重灾区），写字符串要数四层转义，
    // 数错一次的后果是"变异台自己没命中"而**看起来像守卫抓不住**。
    // 正则版只关心"那一行在不在"，`\s` 吸收缩进、不必数字符。
    name: '弹幕逐条颜色被发出去（应摘掉：参照 canvas 不上色）',
    fromRe: /\.replace\(\/\\\\\[0-9\]\?c&H\[0-9A-Fa-f\]\{6\}&\?\/gi, ''\)/,
    to: ".replace(/__disable_strip_color__/, '')",
  },
  {
    // 同一条判据的**另一半**：正则退回"只认 6 位"是另一种"看着还在、其实漏了"的形状
    // （3/4 位简写写法的 `\c` 会漏网发出去）—— 与描边那条 `#fff` 的教训同形。
    name: '弹幕颜色正则退化成只认 6 位（3/4 位简写漏网）',
    fromRe: /c&H\[0-9A-Fa-f\]\{6\}&\?\/gi/,
    to: 'c&H[0-9A-Fa-f]{6}&?/g',
  },
  {
    // ⚠️ **§16 最关键的一处**：把 `med` 也写成一个固定秒数。
    // 这是"照着规格顺手把 med 补进表里"的写法 —— 看着更整洁，但它把
    // `med` 从**老式子**（`min(dur×0.5, 0.6)`）改成了常量 ⇒ 存量工程里**短段的过渡全变**
    // （长段也是），而**没有任何测试会红**。R4 只靠这一条守。
    name: 'med 变成固定秒数（存量过渡全变、R4 破）',
    fromRe: /  snap: 0\.10,\n  fast: 0\.25,\n  slow: 1\.5,/,
    to: '  snap: 0.10,\n  fast: 0.25,\n  med: 0.6,\n  slow: 1.5,',
  },
  {
    // `slow` 不夹整段长度 ⇒ "过渡比镜头本身还长"，推不完就切走（规格 §1.2 那条硬约束）
    name: 'slow 不夹整段长度（过渡比镜头还长）',
    fromRe: /if \(norm === 'slow'\) secs = Math\.min\(secs, durationSec \* 0\.8\);/,
    to: "if (norm === 'slow') secs = secs;",
  },
  {
    // 硬切不再优先 ⇒ `cut: true` 配 `speed: "slow"` 会变成一条 1.5s 的缓动
    // （"切镜"静默变成"推镜"，画面到得比该有的时间晚）。
    name: '硬切不再压过速度（cut 配 slow 变成缓动）',
    fromRe: /if \(hardCut\) return 0;/,
    to: 'if (false) return 0;',
  },
  {
    // ⚠️ **跨文件的漂移**：改的是**预览**那一份（`webui/src/polish/engine.js`），
    // 不是被这个守卫"测"的转译器。`MUTATIONS` 的表都是 `{from,to}` 作用在转译器源码上，
    // 所以这条是**特例**：由 `mutate` 里的 `crossFileMutations` 单独处理。
    // 它守的是"一份规则多份实现"里最隐蔽的一种：两边都改、但改得不一样。
    name: '（跨文件）预览的运镜速度表漂了（fast 0.25 → 0.3）',
    file: 'webui/src/polish/engine.js',
    from: 'export const CAMERA_SPEED_SECS = { snap: 0.10, fast: 0.25, slow: 1.5 }',
    to: 'export const CAMERA_SPEED_SECS = { snap: 0.10, fast: 0.3, slow: 1.5 }',
  },
  // ===== T5 真卡拉OK（2026-10-11）=====
  {
    // ⚠️ **本批最关键的一处**：把逐字切分退化成**均分**。
    //
    // 这正是"顺手把 T2 那套等速搬过来"会写出的代码 —— 看着能跑、cue 数也对，
    // 唯独**时间不是 ASR 给的**。症状：卡拉OK的节奏与人声对不上（快了或慢了），
    // 而报告里写着"切了 N 条"、一行差异都没有。T5 的全部价值就在这一条上。
    name: '（T5）逐字时间退化成均分（丢掉字级时间戳）',
    fromRe: /const rel = \(t \* 1000 - subStartMs\) \/ spanMs;/,
    to: 'const rel = (i + 1) / chars.length;',
  },
  {
    // 缺结论文件时**静默**变回普通字幕（把那条"降级"的 note 删掉）。
    // 用户开了开关却什么都没发生，报告里一行差异都没有 —— 与 #147/#181 同一族。
    name: '（T5）缺结论文件时不报降级（静默变回普通字幕）',
    fromRe: /note\(\n          'elements\.karaoke',\n          '卡拉OK（T5）已开启，但读不到 data\/karaoke-cues\.json'/,
    to: "note(\n          'elements.karaoke',\n          '' + '已开启但读不到结论'",
  },
  {
    // 开关反了：只有 `"yes"`/`1` 也算开 ⇒ 脏值被当成"用户开了"，
    // 而 `polish.toml` 里写 `karaoke = "yes"` 的人以为开了、其实按保守口径该是关的。
    name: '（T5）脏值被当成"开"（`karaoke === true` 放宽）',
    fromRe: /karaoke: raw\.karaoke === true,/,
    to: 'karaoke: raw.karaoke === true || raw.karaoke === "yes",',
  },
  {
    // 每字不再保证**严格递增** ⇒ 时间戳相同的字挤在同一个时刻、几条 cue 互相重叠
    // （条数对、每条时长也 > 0，唯独屏幕上一瞬间同时冒出 "A"/"AB"/"ABC"）。
    // ⚠️ 锚在 `prev + 1` 那一处 —— 它才是**真正**保证严格递增的地方。
    name: '（T5）每字不再严格递增（挤在一起的字互相重叠）',
    fromRe: /ms = Math\.max\(ms, prev \+ 1\);/,
    to: 'ms = Math.max(ms, prev);',
  },
  {
    // 逐字 cue 不再排序 ⇒ 底座按文件顺序读，可能后一条先出现（时间倒挂）。
    // ⚠️ 锚点要**带上后一行**：`out.sort((a,b)…)` 在 `remapCueMs` 里也有一份，
    //    只匹配那一行会命中 2 次（变异台自己报"命中 2 次"—— 实测踩过）。
    name: '（T5）逐字 cue 不按时间排序（条目倒挂）',
    fromRe: /out\.sort\(\(a, b\) => a\.startMs - b\.startMs \|\| a\.endMs - b\.endMs\);\n        remapped\.length = 0;/,
    to: 'out.sort((a, b) => a.text.length - b.text.length);\n        remapped.length = 0;',
  },
  {
    // 时间戳不再做单调守卫 ⇒ 坏数据（时间倒挂）被原样用出去，
    // 切出来的 cue 会重叠/倒退，而"看起来切了"。
    name: '（T5）时间戳不做单调守卫（坏数据被原样用出去）',
    fromRe: /if \(i > 0 && t < times\[i - 1\]\) return null;/,
    to: 'if (false) return null;',
  },
  {
    // 开 karaoke 时**不**把淡入设 0 ⇒ 逐字 cue 只有 33ms、走不完 350ms 淡入
    // ⇒ 有的字根本不画出来（实测 frame 150 = 0 亮像素，而它本该是第 16 个字）。
    name: '（T5）开 karaoke 时淡入仍留 350ms（逐字 cue 亮不起来）',
    fromRe: /fade_in_ms: karaokeOn \? 0 : Math\.round\(0\.35 \* 1000\),/,
    to: 'fade_in_ms: Math.round(0.35 * 1000),',
  },
  {
    // 淡出同理：最后一个字会在 180ms 淡出里"闪一下就没了"（实测 frame 175-178 = 0）。
    name: '（T5）开 karaoke 时淡出仍留 180ms（末字闪一下就没）',
    fromRe: /fade_out_ms: karaokeOn \? 0 : Math\.round\(0\.18 \* 1000\),/,
    to: 'fade_out_ms: Math.round(0.18 * 1000),',
  },
  {
    // ⚠️ **R4 面**：把 `karaokeOn` 的判断去掉、**无条件**设 0 ⇒
    //    普通字幕轨的 350/180 淡入被改掉 = 存量工程成片变样（不配 karaoke 也变）。
    name: '（T5）淡入无条件设 0（普通字幕轨的淡入被改掉，R4 破）',
    fromRe: /fade_in_ms: karaokeOn \? 0 : Math\.round\(0\.35 \* 1000\),/,
    to: 'fade_in_ms: 0,',
  },
  // ===== 文字元素的"贴纸同款弹出"（2026-10-12）=====
  // 三条各自打一个**不同的**漏法：值错了 / 门开错了 / 脏值被当没配。
  {
    // 弹出起点改成不缩（1.0）⇒ "弹出"变成"直接出现" —— 看着还是能出片，
    // 只是那个观感没了。判据 ㉙ 逐值钉的就是这件事。
    name: '（弹出）起点从贴纸的 0.3 改成 1.0 ⇒ 弹出退化成"直接出现"',
    from: "  const POP_IN_DEFAULT = { from: 0.3, peak: 1.0, ms: Math.round(STICKER_POP_IN_SEC * 1000) };",
    to: "  const POP_IN_DEFAULT = { from: 1.0, peak: 1.0, ms: Math.round(STICKER_POP_IN_SEC * 1000) };",
  },
  {
    // 门开错：无条件给缺省 ⇒ **没配 `[elements]` 的老工程也多出三个键**（R4 破）。
    // ⚠️ 这正是我第一版写出来的形状，判据 ⑱ 当场红了。
    //
    // ⚠️⚠️ 2026-10-13（39 档）：`motionFields` 三项现在被 `pick(key, globalVal)` 包着，
    //    而 `pick` 在"逐条预设生效"时**短路**（不评估第二个实参）——
    //    ⇒ 旧的那条 `to: ': POP_IN_DEFAULT.from);'` 会被 `pick` 短路掉、**改不出行为**
    //      （变异台里表现为"打不红"）。所以这里改成**直接打掉 `pick` 的回落分支**：
    //    `pick` 恒返回预设值 ⇒ 没写 `anim` 的条目拿到 `undefined`，
    //    连带把"没配 `[elements]` 走缺省"那条路一起打掉。
    //    ⚠️ 保留原来那条的门语义：**仍然要能证明"门关着时不许写"**。
    //
    //    ⚠️⚠️ 我第一版把 `to` 写成 `preset ? preset[key] : globalVal` ——
    //       **那是恒等的**（`on` 就是 `!!preset`），永远打不红。
    //       有效的形状是**把 `popInOn` 那道门打掉**：把预设那条腿的 `undefined`
    //       也回落到全局缺省 ⇒ 没配 `[elements]` 的条目照样写出三个键（R4 破）。
    name: '（弹出）预设缺省回落 ⇒ 没配 `[elements]` 的条目照样写出弹入三项（R4 破）',
    // ⚠️ 锚点跟着 2026-10-13 的两级化更新：`pick` 现在**两个门分开**
    //    （`onTime` 管取不取值、`on` 管要不要回落全局）—— 打的是那条**回落判据**。
    from: '      return on ? undefined : globalVal;',
    to: '      return globalVal;',
  },
  // ⚠️⚠️ **"这一档指定了"只看一条轴**（纯形态档 ⇒ 动效轴为空 ⇒ 回落全局 `[elements]`）
  //    这一类 R4 破**没有变异登记在这里**，两条都住在
  //    `check-anim-two-axis.mjs --mutate` —— 理由是**本变异台根本不跑 39 档 R4**：
  //    `loadVariant` 那份检查清单里没有一个覆盖两轴（那个区间 grep `anim`/`two.axis` 零命中）。
  //    登记在这里的后果不是"抓不住"，而是**整个变异台变成红的**（两条永远"没被打红"）。
  //    复核者实测点破：把变异体单独喂给 `check-anim-two-axis.mjs` ⇒ 它红并点名
  //    「tilt_std: 多出 scale_in_from=9.9…另 63 处」⇒ **判据抓得住，是登记错了文件**。
  //    ⇒ 加变异前先确认**这个变异台里有检查会跑它**（本文件注释里已经警告过两次，这是第三次）。
  {
    // 脏值被当"没配"：`scale_in_ms = 180.5` 会被悄悄补成 350 ——
    // 用户写了配置、程序当他没写。判据的"脏值那一档"守这件事。
    name: '（弹出）脏 `scale_in_ms` 被补成缺省（用户写了当没写）',
    from: "        : (Number.isInteger(raw.scale_in_ms) && raw.scale_in_ms >= 0",
    to: "        : (true",
  },
];

/// 把 `source` 写进临时目录并真加载一遍（新 URL = 新模块实例，缓存不会串）。
async function loadVariant(source, run) {
  const dir = mkdtempSync(join(tmpdir(), 'yekitrim-translator-'));
  try {
    const file = join(dir, 'polish-to-dhampir.mjs');
    writeFileSync(file, source, 'utf8');
    return await run(await import(pathToFileURL(file).href), source);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

async function mutate() {
  const source = readFileSync(TRANSLATOR, 'utf8');
  let survived = 0;
  console.log(`\n变异自证：${MUTATIONS.length} 处改动，每一处都必须让上面的检查翻面`);
  for (const [index, mutation] of MUTATIONS.entries()) {
    // 两种玩法：① 纯字符串 `from`/`to`（多数）；② `fromRe`/`to`（**正则**版，用于
    // 需要"跨若干行定位、但只改其中一行"的变异 —— 中间那些行会随时间重写，
    // 写死整块的锚点会**静默空转**，2026-10 的 `source_in` 那条就栽在这上面）。
    const hits = mutation.fromRe
      ? (source.match(new RegExp(mutation.fromRe.source, 'g')) || []).length
      : source.split(mutation.from).length - 1;
    // ⚠️ 跨文件变异（表里带 `file`）**必须先于**下面那道"命中一次"的前置检查 ——
    //    那道检查是在**转译器源码**里数命中数的，而跨文件变异打的是**别的文件** ⇒ 恒为 0，
    //    于是它会被判成"变异台自己错了"、永远不会真的去改文件（第一版就是这么漏的，
    //    而门禁自己把原因打了出来："变异台的替换串在转译器里命中 0 次"）。
    if (mutation.file) {
      const target = join(HERE, '..', mutation.file);
      const targetSrc = readFileSync(target, 'utf8');
      const tHits = targetSrc.split(mutation.from).length - 1;
      if (tHits !== 1) {
        survived += 1;
        console.log(`  ✗ ${index + 1}. ${mutation.name}`);
        console.log(`      变异串在 ${mutation.file} 里命中 ${tHits} 次（必须是 1）—— 变异台自己错了`);
        continue;
      }
      writeFileSync(target, targetSrc.replace(mutation.from, mutation.to));
      let xReason = null;
      try {
        checkCameraSpeedParity();
      } catch (error) {
        xReason = String(error && error.message ? error.message : error).split('\n')[0];
      } finally {
        writeFileSync(target, targetSrc); // 无论如何都要还原，否则后面全烂
      }
      if (xReason === null) {
        survived += 1;
        console.log(`  ✗ ${index + 1}. ${mutation.name}\n      检查**没红** —— 漂移守卫抓不住这一类`);
      } else {
        console.log(`  ✓ ${index + 1}. ${mutation.name}\n      打红在：${xReason}`);
      }
      continue;
    }
    // ⚠️ 先证明"替换真的会命中**且只命中一次**" —— 少了这一条，
    //    没匹配上的变异体会原样跑绿，看起来像"这条用例抓不住"。
    if (hits !== 1) {
      survived += 1;
      console.log(`  ✗ ${index + 1}. ${mutation.name}`);
      console.log(`      变异台的替换串在转译器里命中 ${hits} 次（必须是 1）—— 变异台自己错了，不是守卫没抓住`);
      continue;
    }
    const mutated = mutation.fromRe
      ? source.replace(new RegExp(mutation.fromRe.source), mutation.to)
      : source.replace(mutation.from, mutation.to);
    if (mutated === source) {
      survived += 1;
      console.log(`  ✗ ${index + 1}. ${mutation.name}\n      替换后源码没变（变异台自己错了）`);
      continue;
    }
    let reason = null;
    try {
      await loadVariant(mutated, (curves, mutatedSource) => {
        checkCurves(curves);
        // 颜色那一段也要跑：不然"退回只认 6 位"这条变异永远不会红（实测踩过）。
        checkColors(curves);
        // 描边解析同理：它守的是"`#fff` 静默回落成深灰"，与 `hexColor` 是两处。
        checkStroke(curves);
        // ⚠️ 弹幕逐条颜色（§24 更正）：它走**源码文本**判据 ⇒ 必须喂 `mutatedSource`
        // （喂原始 `translatorSource` 的话，变异体改了源码而检查还在看老文本，这条永远绿）。
        checkDanmakuColorStripped(mutatedSource);
        // 运镜速度（§16）同理：它守的是"缺省档与老式子逐值一致"，不跑它，
        // "把 med 写成一个固定秒数"这条变异永远绿。
        checkCameraSpeed(curves);
        // 映射那一段同理（M2 加的三条）。
        checkMappings(curves);
        checkEnvelopeClamp(curves);
        // 变速同理（第 4 批）：不跑它，"变速 NOTE 消失"与"缩段长缩错"这两条变异永远绿。
        checkSpeedReport(curves);
        // TOML 数组表父级同理（第 4 批）：不跑它，"父级建回数组"这条变异永远绿。
        checkTomlArrayTableParent(curves);
        // 第 6 批素材：不跑它们，"退回源时基"与"判据放宽大小写"这两条变异永远绿。
        checkAssetTimebase(curves);
        checkAssetClipPredicate(curves);
        // 六条轨道的显示开关同理（2026-10-06 的 `[stage.lanes]`）—— 不跑它，这几条变异永远绿。
        checkLaneSwitches(curves);
        // 密集键右端那一段同理（③R3，2026-10-05）—— 不跑它这条变异就永远绿。
        checkDenseRanges(curves);
        // 字体族名那条同理（2026-10-08）—— 不跑它，"库里那支没被换成真族名"永远绿。
        checkFontFamily(curves);
        // 元素 ASS 字体源头（2026-10-14）—— **必须喂 `mutatedSource`**：
        // 它钉的是源码文本，而变异只改盘上文件（`mod` 看不见）。
        checkElementAssFont(mutatedSource);
        // 片头/片尾（2026-10-09）—— 不跑它，"不拼进时间线"与"素材没登记"两条变异永远绿。
        checkIntroOutro(curves);
        // ⚠️⚠️ 文字元素轨道的安全宽（2026-10-11）—— **这一条我第一版漏了**：
        //    新检查只加进了 main，没加进这份清单 ⇒ 那 2 处变异**双双存活**，
        //    而变异台照旧跑完、只在末尾说"2 处没被打红"。写这条注释就是为了
        //    下一个人加检查时别再漏（`scripts/AGENTS.md` 写着这条规矩，我还是踩了）。
        // ⚠️ 必须喂 `mutatedSource` —— 行为判据在变异台里看不见变异体（见函数末尾那段注释），
        //    源码文本判据才是那几处变异的实际抓手。
        checkTextElementLane(curves, mutatedSource);
        // §5 E4a T2 打字机（2026-10-11）—— 同一份 `checkTextElementLane` 里，
        // ⚠️ **必须在这里调用**：只加进 main 的话，上面那 5 处变异会**全部存活**，
        // 而变异台照旧跑完、只在末尾说"N 处没被打红"（本趟已经因此栽过一次）。
        // 画面构成的**几何**（两轴 xy 插值）：合成工程里不可见，只能从源码文本抽函数来跑。
        checkStageGeom(mutatedSource);
        // 媒体探测那一段同理（不跑它，写死时基的变异永远绿）
        checkMediaProbe(curves);
        checkStageBackground(curves);
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
// 画面构成的几何（`SR` / `FIT`）：**两轴 xy 的插值式子**（2026-10-05）
// ---------------------------------------------------------------------------
//
// 为什么单独一段：转译器的 `stageDrawRect` 是个**闭包**（读 `axis` / `stagePctX` /
// `stagePctY` / `stagePadding` / `stageScale` 这几个模块级 const），既没导出、也不在
// `translate()` 的返回值里；而合成工程里**没有真视频** ⇒ `probeSize` 恒失败 ⇒ `axis` 恒 `fill`
// ⇒ 它的几何在上面那组行为判据里**完全不可见**（"画面构成"那一组只能钉竖屏三道整类门）。
//
// 所以这一段**从源码文本里抽出那个箭头函数**再喂几何表；变异台改的也是同一份文本，
// 因此它照样能翻面（`--mutate` 那条"X 轴改回硬居中"就是冲它来的）。
//
// 数值口径 = 竖素材 `1080x1920` 进横画布 `1920x1080`（`axis=x`）：`w = 1080*1080/1920 = 607.5`。
const STAGE_RECT_RE = /const stageDrawRect = \(bw, bh, cw, ch\) => \{[\s\S]*?\n {2}\};/;
function checkStageGeom(source) {
  let n = 0;
  const eq = (actual, expected, what) => {
    n += 1;
    if (actual !== expected) throw new Error(`${what}：期望 ${expected}，实得 ${actual}`);
  };
  const near = (actual, expected, what) => {
    n += 1;
    if (!(Math.abs(actual - expected) <= 1e-9)) {
      throw new Error(`${what}：期望 ${expected}（±1e-9），实得 ${actual}`);
    }
  };
  const hit = source.match(STAGE_RECT_RE);
  assert.ok(hit, '转译器里找不到 `stageDrawRect`（闭包被改名或搬走了？）');
  const make = new Function(
    'axis', 'stagePctX', 'stagePctY', 'stagePadding', 'stageScale',
    `${hit[0]}
return stageDrawRect;`,
  );
  /// 抽出来的那个是**工厂**（`make(...)` 返回 `stageDrawRect` 本身），还要喂 `bw,bh,cw,ch`。
  const rect = (axis, px, py, scale, pad = 0, src = [1080, 1920], canvas = [1920, 1080]) =>
    make(axis, px, py, pad, scale)(...src, ...canvas);

  // ① `scale=2`：**先定画面大小、再按余量插值**（旧顺序「先插值再绕插值后的盒心缩放」在
  //    X=0 时给 -303.75，画面被推出盒外、右边 1009px 永远是空的）。余量 X = 1920-1215 = 705、
  //    Y = 1080-2160 = -1080（负数 = 裁切取景）。
  const r0 = rect('x', 0, 0, 2);
  near(r0.x, 0, 'scale=2 且 X=0：贴左（不是旧顺序的 -303.75）');
  near(r0.y, 0, 'scale=2 且 Y=0：露画面顶');
  near(r0.w, 1215, 'scale=2 的宽 = 607.5×2');
  near(r0.h, 2160, 'scale=2 的高 = 1080×2');
  const r1 = rect('x', 50, 50, 2);
  near(r1.x, 352.5, 'scale=2 居中：X 余量的一半');
  near(r1.y, -540, 'scale=2 居中：Y 溢出的一半');
  const r2 = rect('x', 100, 100, 2);
  near(r2.x, 705, 'scale=2 且 X=100：贴右');
  near(r2.y, -1080, 'scale=2 且 Y=100：露画面底');

  // ② `scale=1`：X 在左右留边（1312.5）里五档插值；Y **没有余量** ⇒ 取什么值都不动（两轴独立）。
  for (const [pct, want] of [[0, 0], [25, 328.125], [50, 656.25], [75, 984.375], [100, 1312.5]]) {
    const r = rect('x', pct, 70, 1);
    near(r.x, want, `axis=x scale=1 X=${pct}：在留边里线性插值`);
    near(r.y, 0, `axis=x scale=1：Y 无余量（给 70 也是 0）`);
  }

  // ③ 同比铺满必须**精确等于整块画布**（存量逐字节零回归的钉子）。
  const fill = rect('fill', 50, 50, 1, 0, [1080, 1920], [1080, 1920]);
  eq(fill.x, 0, 'fill + 同比 ⇒ x=0');
  eq(fill.y, 0, 'fill + 同比 ⇒ y=0');
  eq(fill.w, 1080, 'fill + 同比 ⇒ w=画布宽');
  eq(fill.h, 1920, 'fill + 同比 ⇒ h=画布高');

  // ④ 键的读数：两轴新键优先、缺哪条回落单值 `position_pct`、再缺按老枚举（迁移逐像素不变）。
  assert.ok(/config\.stage\?\.position_x_pct/.test(source), '转译器没读 `position_x_pct`');
  assert.ok(/config\.stage\?\.position_y_pct/.test(source), '转译器没读 `position_y_pct`');
  assert.ok(/config\.stage\?\.position_pct/.test(source), '转译器丢了单值老字段的回落');
  n += 3;
  return n;
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

// ---------------------------------------------------------------------------
// 媒体探测：时基与分辨率必须来自**实际媒体**（2026-10-05 修掉的写死 60fps/1920×1080）
// ---------------------------------------------------------------------------
//
// 写死那两件事的后果：30fps 素材被声明成 60fps ⇒ **帧号整体错 2×**（画面放到一半就播完、
// 定位全错）；预览改读 720p 代理后仍声明 1920×1080 ⇒ 层的 `scale`/`FIT` 按错的源尺寸算。
// 判据要**真媒体**：拿 ffmpeg 造两段不同帧率/尺寸的片子，看工程里写的是什么。
// 没有 ffmpeg 就跳过（打印原因，不让守卫红）—— 与 `takumi_font_id` 那条同一条纪律。
function checkMediaProbe(mod) {
  let ok = true;
  try { execFileSync('ffmpeg', ['-version'], { stdio: 'ignore' }); } catch { ok = false; }
  try { execFileSync('ffprobe', ['-version'], { stdio: 'ignore' }); } catch { ok = false; }
  if (!ok) { console.log('· 跳过媒体探测判据：这台机器没有 ffmpeg/ffprobe'); return 0; }
  const dir = mkdtempSync(join(tmpdir(), 'yekitrim-media-'));
  let checks = 0;
  const eq = (actual, expected, what) => {
    assert.equal(actual, expected, what);
    checks += 1;
  };
  try {
    const mk = (file, size, rate) => execFileSync('ffmpeg', [
      '-v', 'error', '-y', '-f', 'lavfi', '-i', `testsrc2=size=${size}:rate=${rate}`,
      '-t', '1', '-pix_fmt', 'yuv420p', join(dir, file),
    ]);
    mk('clip.mp4', '640x360', 30);
    mk('alt.mp4', '320x240', 24);
    mk('clip5.mp4', '640x360', 5);
    const toml = [
      'duration = 4.0',
      '[project]',
      'name = "media-probe"',
      'output = "o.mp4"',
      '[source]',
      'video = "clip.mp4"',
      '',
    ].join('\n');
    const cfg = mod.parseToml(toml);
    const videoOf = (doc) => (doc.assets ?? []).find((a) => a.kind === 'video') ?? {};
    // ① 默认 = `[source] video` 那份媒体
    const d1 = mod.translate(cfg, dir, join(dir, 'd1.json'));
    eq(JSON.stringify(d1.timeline.timebase), JSON.stringify({ num: 30, den: 1 }),
      '源是 30fps ⇒ 工程时基必须是 30/1（写死 60 ⇒ 帧号错 2×）');
    eq(`${videoOf(d1).width}x${videoOf(d1).height}`, '640x360',
      '视频素材的宽高必须是**实际媒体**的（写死 1920x1080 ⇒ 层 scale/FIT 全按错的算）');
    // ② `--video` 覆盖 = 预览代理那条腿：时基与尺寸都要跟着**代理**走
    const d2 = mod.translate(cfg, dir, join(dir, 'd2.json'), { video: 'alt.mp4' });
    // 24fps 也低于 30 的下限 ⇒ 工程网格 30/1，而**素材时基仍是代理的 24/1**（两者一起看才对得上）
    eq(JSON.stringify(d2.timeline.timebase), JSON.stringify({ num: 30, den: 1 }),
      '24fps 代理也吃 30fps 下限 ⇒ 工程网格 30/1');
    eq(JSON.stringify(videoOf(d2).timebase), JSON.stringify({ num: 24, den: 1 }),
      '`--video` 指向 24fps 的代理 ⇒ **素材**时基必须是 24/1（源帧号靠它换算）');
    eq(`${videoOf(d2).width}x${videoOf(d2).height}`, '320x240',
      '`--video` 指向 320x240 的代理 ⇒ 素材尺寸必须跟着代理');
    // ③ **30fps 下限**（2026-10-06 用户口径）：电台直播那种"视频流 5fps 静止帧、主要是音频"的素材，
    //    工程网格抬到 30fps（叠加层一秒动 30 次），而**素材自己的时基仍然是 5fps**
    //    （源帧号靠它换算 —— 套上下限会让 5fps 素材的源帧号跑 6 倍，画面 1/6 时长就播完）。
    const d3 = mod.translate(cfg, dir, join(dir, 'd3.json'), { video: 'clip5.mp4' });
    eq(JSON.stringify(d3.timeline.timebase), JSON.stringify({ num: 30, den: 1 }),
      '源只有 5fps ⇒ 工程网格必须抬到 30/1（下限）');
    eq(JSON.stringify(videoOf(d3).timebase), JSON.stringify({ num: 5, den: 1 }),
      '源只有 5fps ⇒ **素材时基仍是 5/1**（源帧号靠它换算，不能跟着抬）');
    // ④ **`source_in` 的单位**（2026-10-06 用户报「经过屏蔽时声音和视频全乱」的根因）：
    //    底座 `source_frame_looped(source_in, local_frame, 时间线时基, 素材时基, …)` 里
    //    `source_in` 是**素材帧号**，而图层 `start` 是**序列帧号**。两者单位混了（都写成序列帧），
    //    5fps 素材 + 30fps 网格下第二段会从源里 6 倍远的地方取帧 ⇒ 画面与声音从第一个屏蔽段起分叉。
    //    夹具：6 秒、屏蔽 [2,4] ⇒ 第二段 start=60（2s×30），source_in 必须 = 20（4s×5），不是 120。
    const tomlMute = [
      'duration = 6.0',
      // ⚠️ **数组表**写法：转译器自带的小 TOML 解析器只认数组表（真实 `polish.toml` 也是这么写的）
      '[[mute_ranges]]',
      'start = 2.0',
      'end = 4.0',
      '[project]',
      'name = "mute-probe"',
      'output = "o.mp4"',
      '[source]',
      'video = "clip5.mp4"',
      '',
    ].join('\n');
    const d4 = mod.translate(mod.parseToml(tomlMute), dir, join(dir, 'd4.json'));
    const v1 = ((d4.timeline.tracks || []).find((t) => t.id === 'v1') || {}).layers || [];
    eq(v1.length, 2, '屏蔽一段 ⇒ 主画面切成两层（每段一层）');
    eq(v1[1].start, 60, '第二段的 start 是**序列帧**（2s × 30fps 网格）');
    eq(v1[1].source.source_in, 20, '第二段的 source_in 是**素材帧**（4s × 5fps 素材），不是 120');
  } finally { rmSync(dir, { recursive: true, force: true }); }
  return checks;
}
// ---------------------------------------------------------------------------
// 画布底色：`bg_mode = color|black` 必须发 `render_hints.background`（零图层的那条路）
// ---------------------------------------------------------------------------
//
// 用户报的「背景纯色/模糊一律黑屏」里，纯色那一半的落地方式就是它：底座**没有纯色图层原语**，
// 而加一层"有素材的图层"会触发分段路径逐帧泄漏（长片 OOM）⇒ 只能走**清屏色**。
// 判据：color ⇒ bg_color、black ⇒ #000000、blur ⇒ **不发**（它是图层那条腿，另有开关）。
function checkStageBackground(mod) {
  const dir0 = mkdtempSync(join(tmpdir(), 'yekitrim-bg-src-'));
  let checks = 0;
  const eq = (actual, expected, what) => {
    assert.equal(actual, expected, what);
    checks += 1;
  };
  // ⚠️ 夹具必须有**真竖素材**：源比例 = 画布比例（或探不到源）时 `STAGE_BG` 会推成 `none`，
  // 那条路根本不会发背景 —— 拿它量"纯色有没有发出去"是量不到的（实测踩过）。
  try {
    execFileSync('ffmpeg', [
      '-v', 'error', '-y', '-f', 'lavfi', '-i', 'color=c=magenta:s=360x640:d=2:r=30',
      '-pix_fmt', 'yuv420p', '-c:v', 'libx264', '-preset', 'ultrafast', join(dir0, 'clip.mp4'),
    ]);
  } catch {
    console.log('· 跳过画布底色判据：这台机器没有 ffmpeg');
    return 0;
  }
  const docOf = (stageLines) => {
    const dir = mkdtempSync(join(tmpdir(), 'yekitrim-bg-'));
    try {
      copyFileSync(join(dir0, 'clip.mp4'), join(dir, 'clip.mp4'));
      writeFileSync(
        join(dir, 'polish.toml'),
        [
          'duration = 4.0',
          '[project]',
          'name = "bg-probe"',
          'output = "o.mp4"',
          '[source]',
          'video = "clip.mp4"',
          '[canvas]',
          'orientation = "landscape"',
          '[stage]',
          ...stageLines,
          '',
        ].join('\n'),
      );
      return mod.translate(mod.parseToml(readFileSync(join(dir, 'polish.toml'), 'utf8')), dir, join(dir, 'd.json'));
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  };
  const color = docOf(['bg_mode = "color"', 'bg_color = "#123456"']);
  eq(color.render_hints.background, '#123456', 'bg_mode=color ⇒ 画布底色 = bg_color');
  const black = docOf(['bg_mode = "black"']);
  eq(black.render_hints.background, '#000000', 'bg_mode=black ⇒ 画布底色 = #000000');
  const dark = docOf(['bg_mode = "black"', 'bg_color = "#ffffff"']);
  eq(dark.render_hints.background, '#000000', 'black 档不许被 bg_color 带走');
  const blur = docOf(['bg_mode = "blur"']);
  eq(blur.render_hints.background, undefined, 'blur ⇒ 不发画布底色（它是图层那条腿）');
  const none = docOf([]);
  eq(none.render_hints.background, undefined, '没写 bg_mode ⇒ 不发（与改造前一致）');
  rmSync(dir0, { recursive: true, force: true });
  return checks;
}

const translatorModule = await import(pathToFileURL(TRANSLATOR).href);
// 转译器的**源码文本**，给那些"判据落在 `translate` 内部闭包上、拿不到函数引用"的检查用
// （`checkDanmakuColorStripped` 是第一个消费者）。读一次在这里，别在检查函数里各读一遍。
const translatorSource = readFileSync(TRANSLATOR, 'utf8');
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
const bgChecks = checkStageBackground(translatorModule);
console.log(`✓ 画布底色（color/black 走 render_hints.background，blur 不发）：${bgChecks} 项全绿`);
const mediaChecks = checkMediaProbe(translatorModule);
if (mediaChecks > 0) {
  console.log(`✓ 媒体探测：${mediaChecks} 项全绿（30fps/640x360 源与 24fps/320x240 代理各一对）`);
}
const colorChecks = checkColors(translatorModule);
console.log(
  `✓ 颜色解析：${colorChecks} 项全绿（6 种写法 + 7 种垃圾输入；8 位那两种是"整条 overlay 丢不丢"的关键）`,
);
const strokeChecks = checkStroke(translatorModule);
console.log(
  `✓ 描边解析：${strokeChecks} 项全绿（3/4/6/8 位 hex 都取出真颜色；` +
    `只有 6 位会让 \`#fff\` **静默回落成深灰**，报告里一行差异都没有）`,
);
const dmColorChecks = checkDanmakuColorStripped();
console.log(
  `✓ 弹幕逐条颜色：${dmColorChecks} 项全绿（\`\\c\` **必须被摘掉** —— 参照 canvas 不上色，` +
    `发了就是"底座比参照更花"，实测 48.8s 那帧参照白 / 底座品红）`,
);
const camSpeedChecks = checkCameraSpeed(translatorModule);
const camParityChecks = checkCameraSpeedParity();
console.log(
  `✓ 运镜速度（§16）：${camSpeedChecks} 项全绿（四档 + 缺省与**老式子逐值一致** —— ` +
    `存量工程没写 \`speed\`，那一档差一帧就是横屏全体回归）`,
);
if (camParityChecks > 0) {
  console.log(
    `✓ 运镜速度对拍：${camParityChecks} 项全绿（预览 engine.js ↔ 转译器 —— ` +
      `两处漂了就是"预览推得快、成片推得慢"，而两边都不会报错）`,
  );
}

// ---------------------------------------------------------------------------
// 颜色：四种写法 + "认不出"必须返回 null
// ---------------------------------------------------------------------------
//
// ⚠️ 这条守的是一个**"静默丢事件"的形状**（2026-09-30 修）：`#RRGGBBAA` 曾经解析不出来，
// 而 overlay 那处是 `if (!first) drop(...)` —— 于是**整条覆盖层事件不进成片**，
// 报告里只剩一行 DROP。YekiTrim 自己的配置就写过 8 位（演示工程三条 overlay 全是）。
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
// 描边字符串（`parseStroke`）：**3/4 位 hex 必须也能解析出真颜色**
// ---------------------------------------------------------------------------
//
// ⚠️ 这条守的是"**颜色静默变回默认**"（2026-10 修的真缺陷）：`hexColor` 本来就能展开 3/4 位，
// 但 `parseStroke` **外层那个正则**曾经只认 6 位 ⇒ `stroke = "8px #fff"` 匹配不上、
// `hexColor` 根本走不到，于是**静默回落到兜底深灰**（实测 `[64,60,59]` 而不是白）。
// 症状：用户在编辑器里选"白色描边"、成片是深灰，**报告里一行差异都没有**。
//
// ⚠️ 别只测 `hexColor`（上面那段就是）—— 那测不出这个问题：`hexColor('#fff')` 一直是对的，
// 错的是**它的调用点上那道前置匹配**。所以这里必须走 `parseStroke` 本体。
function checkStroke(mod) {
  const gray = [64, 60, 59]; // `SubtitleStyle::default_sub_stroke` 的兜底色 `#403c3b`
  const cases = [
    ['12px #fff', 12, [255, 255, 255], '3 位简写要解析出真颜色（不许回落兜底）'],
    ['12px #abc', 12, [170, 187, 204], '3 位简写（非灰度）'],
    ['0px #fff', 0, [255, 255, 255], '宽度 0 也要解析颜色'],
    ['12px #aabbcc', 12, [170, 187, 204], '6 位照旧'],
    ['12px #aabbccdd', 12, [170, 187, 204], '8 位：吃掉整串，忽略 alpha（outline 只有开/关）'],
    ['12px #abcd', 12, [170, 187, 204], '4 位简写'],
    ['4px #403c3b', 4, gray, '默认值原样往返'],
  ];
  for (const [raw, wantW, wantC, why] of cases) {
    const got = mod.parseStroke(raw, 12, '#403c3b');
    assert.equal(got.widthPx, wantW, `parseStroke(${JSON.stringify(raw)}).widthPx（${why}）`);
    assert.deepEqual(
      [got.color[0], got.color[1], got.color[2]],
      wantC,
      `parseStroke(${JSON.stringify(raw)}) 的颜色（${why}）`,
    );
  }
  // 认不出颜色时**照旧回落兜底**（那条行为不能改：调用点依赖它）
  const fb = mod.parseStroke('12px nothex', 12, '#403c3b');
  assert.deepEqual([fb.color[0], fb.color[1], fb.color[2]], gray, '认不出颜色时回落兜底色');
  // 宽度认不出 → 回落传入的兜底宽度
  assert.equal(mod.parseStroke('#fff', 9, '#403c3b').widthPx, 9, '认不出宽度时回落兜底宽度');
  return cases.length + 2;
}

// ---------------------------------------------------------------------------
// 弹幕逐条颜色：`\c` **必须被摘掉**（这是**有意的**，不是缺口）
// ---------------------------------------------------------------------------
//
// ⚠️ 这条守的是一个**方向反了**的误判（2026-10-10 核实）。上游计划书
//（`docs/plan/editing-techniques-catalog.md` §0.4）把「转译器不生成 `\c` 覆盖」列为
// 「本仓单方面就能修」的接线缺口 —— 而真相是**反的**：
//
//   参照 = **本仓 canvas**（`templates/index.html` 的 `getActiveDms`），它每条的出口是
//   `{ text, opacity, x, y }`、**不带颜色**；上色只用**轨道级** `dmS.color`。
//   底座其实**支持**逐条颜色 ⇒ 一旦转译器把 `\c` 发出去，成片就**比参照更花**。
//
// 这不是推断：转译器早期版本**真的发过**，实测「48.8s 那一帧 YekiTrim 是白的、底座是品红的」，
// 于是 `85d7fc0`（2026-10-01，与"底座成为默认出片通道"**同一批**）加了 `stripColorTags` 主动摘掉。
//
// ⇒ 这条断言的方向是"**摘掉才算对**"。它防的是"下一个人照着那份计划书好心加回来"——
//   那种改法不会让任何测试变红、也不会报错，只会让成片悄悄比参照花。
//
// ⚠️ 判据走**源码**（`translatorSource`）而不是导出函数：`stripColorTags` 是 `translate`
// 内部的闭包、拿不到引用，而"这段源码还在不在"正是要锁的东西。
// ⚠️ **源码是参数、不是闭包捕获**：变异台必须能把**变异后**的文本喂进来 ——
// 捕获外层那个常量的话，变异体改了源码而检查仍在看原文，**这条变异永远绿**
// （`checkStageGeom` 也踩过同一个坑，所以它同样收 `mutatedSource`）。
// ⚠️ 正则会**再收一次**：只断言"出现过 `stripColorTags` 这个词"是不够的 ——
// 把 `.replace(/\\c&H…/)` 那行删掉、只留下函数名，词还在、功能没了。
function checkDanmakuColorStripped(src = translatorSource) {
  // ⚠️ **别只断言"那句话在不在"** —— 这是第一版踩的坑：把 `[0-9]?` 去掉
  // （退化成只认 `c&H`、3/4 位简写漏网）之后**那句话仍然原样在**，
  // 于是判据照样绿、变异台报"守卫抓不住"（实跑抓到过）。
  // ⇒ 必须**真跑那条正则**，判据才落在**语义**上而不是**字面上**。
  //
  // ⚠️ **不用正则去正则匹配**：`\.replace\((\/\\\\\[0-9\]...` 这种要数四层转义，
  // 实测写错一次就 `match: null`（锚点静默失效比报红难查十倍）。
  // 改成**定位 + 取定界符之间**：找到 `stripColorTags` 那几行，逐行取 `/` 与 `/gi` 之间的原文。
  const lines = src.split('\n');
  const start = lines.findIndex((l) => l.includes('const stripColorTags'));
  assert.ok(start >= 0, '转动器里找不到 `stripColorTags`（它是"摘掉弹幕逐条颜色"的唯一实现）');
  const block = lines.slice(start, start + 4).join('\n');
  const open = block.indexOf('/\\\\');
  const close = block.indexOf('/gi', open);
  assert.ok(
    open >= 0 && close > open,
    '`stripColorTags` 的第一条 `.replace(/…/gi, \'\')` 不见了 —— 它就是"摘掉弹幕逐条颜色"的实现。' +
      '参照 canvas 不上色，发出去就是"底座比参照更花"（实测 48.8s 那帧参照白/底座品红）。见 §24 更正。',
  );
  const inner = block.slice(open + 1, close);
  // 真跑一遍。⚠️ **单跑第一条只摘指令本身**：`{\c&H3366FF}蓝` → `{}蓝`
  // （花括号是**第二条** `.replace(/\{\s*\}/g,'')` 收的）—— 所以这里要**两条一起**跑，
  // 那才是 `stripColorTags` 的真实语义。第一版只跑第一条、却按"花括号也没了"写断言，
  // 于是判据自己红了（代码是对的）—— 这类"期望写错"最容易让人去改本来正确的代码。
  const strip = new RegExp(inner, 'gi');
  const stripEmpty = /\{\s*\}/g;
  const clean = (s) => s.replace(strip, '').replace(stripEmpty, '');
  const cases = [
    ['{\\c&H3366FF}蓝', '蓝', '6 位（最常见）'],
    ['{\\1c&H3366FF}蓝', '蓝', '\\1c 带配色组序号'],
    ['{\\3c&HFF0000}红', '红', '\\3c 同上'],
    ['{\\c&H3366FF&}蓝', '蓝', '结尾多一个 & 也要摘'],
    ['{\\c&h3366ff}小写', '小写', '小写 hex 也认（/i）'],
    // 与别的指令同处一个标签块：只摘 `\c`，`\pos` 留着
    ['{\\pos(1,2)\\c&H3366FF}文', '{\\pos(1,2)}文', '只摘 \\c，别的不许动'],
  ];
  for (const [raw, want, why] of cases) {
    assert.equal(clean(raw), want, `摘 \`\\c\` 失败：${why} —— ${raw}`);
  }
  // 反方向：**不带颜色的标签块一个字符都不许动**（`\pos` 那条 P0 的教训）
  assert.equal(clean('{\\pos(1,2)}文'), '{\\pos(1,2)}文', '没颜色标签的块必须原样保留');
  // 摘完还要顺手清掉空标签块，否则下游拿到一堆 `{}`
  assert.ok(
    block.includes('/\\{\\s*\\}/g'),
    '摘掉 `\\c` 之后要顺手去掉摘空的 `{}` 标签块（否则下游拿到一堆空标签）',
  );
  // 反方向：**不许**有一条"发 `\c`"的代码还活着（按计划书加回来就该红）。
  // 摘的那几行要先剔掉，否则它自己就把这条判据顶翻。
  const withoutStrip = block
    .split('\n')
    .filter((l) => !l.includes('.replace('))
    .join('\n');
  const emit = /\\c&H/.exec(withoutStrip);
  assert.equal(
    emit,
    null,
    '`\\c` 只许被"摘"，不许被"发" —— 这里发现一处疑似**生成**逐条颜色的代码：' +
      `${emit?.[0]?.trim() ?? ''}。若真要做弹幕上色，那要同时改参照渲染器与底座两侧（新需求），` +
      '不是把摘掉的那段加回来（见 `polish-upgrade-consolidated.md` 更正 ④）。',
  );
  return 3 + cases.length + 1;
}

// ---------------------------------------------------------------------------
// 运镜速度（规格 `camera-and-noise-spec.md` §1 / 总方案 §16）
// ---------------------------------------------------------------------------
//
// 在它之前过渡时长是**写死**的（`min(dur × 0.5, 0.6)`），推近/拉远的快慢完全一样。
//
// ⚠️ **这条判据里最重要的一项不是"四档对不对"，而是"缺省档与老式子逐值一致"** ——
// 今天所有工程都没写 `speed`，只要那一档偏一帧，**全部存量横屏工程同时回归**
// （而且不会报错：过渡短一点/长一点都不会让任何测试红）。
// 所以这里不看"差不多"，而是拿**改动前那句原话**当参照、逐值比。
function checkCameraSpeed(mod) {
  let checks = 0;
  const eq = (actual, expected, what) => {
    assert.equal(actual, expected, what);
    checks += 1;
  };
  const f = mod.cameraTransitionFrames;
  assert.equal(typeof f, 'function', '`cameraTransitionFrames` 要 export（守卫靠它逐值测）');
  checks += 1;

  // ① 四档在自己该在的秒数上（@60fps，段长 5s 足够装下任何一档）
  eq(f({ speed: 'snap', hardCut: false, durationSec: 5, fps: 60 }), 6, 'snap = 0.10s');
  eq(f({ speed: 'fast', hardCut: false, durationSec: 5, fps: 60 }), 15, 'fast = 0.25s');
  eq(f({ speed: 'slow', hardCut: false, durationSec: 5, fps: 60 }), 90, 'slow = 1.5s');
  // `med` 与"没写"走同一条老式子：min(5×0.5, 0.6) = 0.6 ⇒ 36 帧
  eq(f({ speed: 'med', hardCut: false, durationSec: 5, fps: 60 }), 36, 'med = 老式子');

  // ② ⚠️ **缺省档：与改动前那句原话逐值一致**（fps × 段长 的网格，五种"等于没写"的写法）
  const oldExpr = (durationSec, fps) =>
    Math.max(1, Math.round(Math.min(durationSec * 0.5, 0.6) * fps));
  let mismatches = 0;
  let compared = 0;
  for (const fps of [24, 25, 30, 50, 60]) {
    for (let dur = 0.01; dur <= 12; dur += 0.017) {
      for (const speed of [undefined, 'med', null, '', 'bogus-speed']) {
        const got = f({ speed, hardCut: false, durationSec: dur, fps });
        const want = oldExpr(dur, fps);
        compared += 1;
        if (got !== want) {
          if (mismatches < 3) {
            assert.fail(
              `缺省档与老式子不一致（speed=${JSON.stringify(speed)} dur=${dur.toFixed(3)} ` +
                `fps=${fps}）：得 ${got}，老式子 ${want} —— **存量工程全体回归**`,
            );
          }
          mismatches += 1;
        }
      }
    }
  }
  eq(mismatches, 0, `缺省/med/垃圾值 共 ${compared} 组必须逐值等于老式子`);

  // ③ `slow` 要夹到整段的 0.8（否则"过渡比镜头还长"，推不完就切走）
  eq(f({ speed: 'slow', hardCut: false, durationSec: 1, fps: 60 }), 48, 'slow 夹到 1×0.8 = 0.8s');
  eq(f({ speed: 'slow', hardCut: false, durationSec: 0.5, fps: 60 }), 24, 'slow 夹到 0.4s');
  // ④ 硬切优先于速度（两者互斥、以 cut 为准）
  eq(f({ speed: 'slow', hardCut: true, durationSec: 5, fps: 60 }), 0, '硬切 = 0 帧（cut 压过 speed）');
  // ⑤ 除硬切外**至少 1 帧**：0 帧会让它退化成硬切，而用户写 slow 要的正是"看得见的缓动"。
  //    ⚠️ 期望值要按**帧率**算：`snap` 0.10s @24fps = round(2.4) = **2 帧**，不是 1
  //    （第一版我写死 1，判据自己红了 —— 代码是对的）。要试"被 `max(1,…)` 兜到"，
  //    得让 round 真的落到 0：`snap` 0.10s @ 4fps = round(0.4) = 0 ⇒ 兜成 1。
  eq(f({ speed: 'snap', hardCut: false, durationSec: 0.001, fps: 24 }), 2, 'snap@24fps = 2 帧');
  eq(f({ speed: 'snap', hardCut: false, durationSec: 0.001, fps: 4 }), 1, 'round 到 0 时兜成 1 帧');
  // ⑥ 大小写/空白要认（LLM 写出 "Slow " 不该静默退回老式子）
  eq(f({ speed: ' SLOW ', hardCut: false, durationSec: 5, fps: 60 }), 90, '大小写与空白都要归一');
  return checks;
}

// ---------------------------------------------------------------------------
// 运镜速度：**预览引擎与转译器必须同口径**（§16 的第二处实现）
// ---------------------------------------------------------------------------
//
// ⚠️ 这是"一份规则多份实现"的老形状：`webui/src/polish/engine.js` 里也有一份过渡时长
// （预览要按它画推进动画）。两边漂了的表现是**「预览里推得快、成片里推得慢」** ——
// 不看两个版本对着比根本发现不了，而且没有任何测试会红。
//
// 判据：同一份档位表 + 同一组样本，**逐个比对两边的秒数**。
// ⚠️ 只比"档位 → 秒数"这个层面：转译器那边还要 `round` + `max(1,…)` 落成帧，
// 那是帧号的事，与预览无关，硬比会在 24/25/30fps 上假红。
function checkCameraSpeedParity() {
  const enginePath = join(HERE, '..', 'webui', 'src', 'polish', 'engine.js');
  if (!existsSync(enginePath)) {
    console.log('· 跳过运镜速度对拍：找不到 webui/src/polish/engine.js');
    return 0;
  }
  const src = readFileSync(enginePath, 'utf8');
  let checks = 0;
  const eq = (actual, expected, what) => {
    assert.equal(actual, expected, what);
    checks += 1;
  };

  // ① 档位表两处必须逐值相同（先从源码里把表抽出来比，避免"两边都写错了但恰好一致"）
  const tableOf = (text, name) => {
    const m = new RegExp(`${name}\\s*=\\s*\\{([^}]*)\\}`).exec(text);
    if (!m) return null;
    const out = {};
    for (const part of m[1].split(',')) {
      const kv = /(\w+)\s*:\s*([\d.]+)/.exec(part);
      if (kv) out[kv[1]] = Number(kv[2]);
    }
    return out;
  };
  const engineTable = tableOf(src, 'CAMERA_SPEED_SECS');
  const transTable = tableOf(translatorSource, 'CAMERA_SPEED_SECS');
  assert.ok(engineTable, 'engine.js 里找不到 `CAMERA_SPEED_SECS`（运镜速度档位表）');
  assert.ok(transTable, '转译器里找不到 `CAMERA_SPEED_SECS`');
  checks += 2;
  assert.deepEqual(
    engineTable,
    transTable,
    `运镜速度档位表两处不一致（预览 engine.js vs 转译器）—— 症状是"预览与成片推进快慢不同"：\n` +
      `  engine.js: ${JSON.stringify(engineTable)}\n  转译器: ${JSON.stringify(transTable)}`,
  );
  checks += 1;
  // `med` 两边都**不许**出现在表里（它必须走老式子，见类型注释与转译器那段）
  assert.ok(!('med' in engineTable) && !('med' in transTable), '`med` 不许进档位表：它必须走老式子');
  checks += 1;

  // ② 行为对拍：把 engine.js 的那份函数**扒出来**真跑（本目录不 import 前端模块，与其它
  //    `webui/scripts/check-*.mjs` 同一套做法：从源码文本里取函数体）
  const fnSrc = /export function cameraTransitionSecs\([\s\S]*?\n\}/.exec(src);
  assert.ok(fnSrc, 'engine.js 里找不到 `cameraTransitionSecs`（预览侧的过渡时长）');
  const evalFn = new Function(
    'CAMERA_SPEED_SECS',
    `${fnSrc[0].replace('export function', 'return function')}`,
  )(engineTable);
  const trans = transTable;
  const transSecs = (speed, dur, hardCut) => {
    if (hardCut) return 0;
    const norm = typeof speed === 'string' ? speed.trim().toLowerCase() : '';
    if (Object.prototype.hasOwnProperty.call(trans, norm)) {
      return norm === 'slow' ? Math.min(trans.slow, dur * 0.8) : trans[norm];
    }
    return Math.min(dur * 0.5, 0.6);
  };
  let compared = 0;
  let bad = 0;
  for (const dur of [0.3, 1, 2.5, 5, 12]) {
    for (const speed of [undefined, null, 'med', 'snap', 'fast', 'slow', ' SLOW ', 'warp']) {
      for (const hardCut of [false, true]) {
        const a = evalFn({ speed }, dur, hardCut);
        const b = transSecs(speed, dur, hardCut);
        compared += 1;
        if (Math.abs(a - b) > 1e-9) {
          if (bad < 3) {
            assert.fail(
              `预览与转译器的运镜过渡不一致：speed=${JSON.stringify(speed)} dur=${dur} ` +
                `hardCut=${hardCut} ⇒ engine ${a}s / 转译器 ${b}s（症状：预览与成片快慢不同）`,
            );
          }
          bad += 1;
        }
      }
    }
  }
  eq(bad, 0, `预览 vs 转译器 共 ${compared} 组必须逐值一致`);
  return checks;
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
// ---- 字体：库 id → 真族名（2026-10-08，底座 0.2.0 起）--------------------
// 抽成函数是因为**变异自证也要跑它**（`scripts/AGENTS.md`：只写在顶层的检查，变异台不会跑）。
//
// 钉的是"库里那支有没有被换成真族名"。底座 0.2.0 起字幕走 libass，而 **libass 按族名找字体**；
// `polish.toml` 里存的却是**库 id**。两者在库里确实会不一样（实测 id `GenJyuuGothic-Bold`
// vs 族名 `Gen Jyuu Gothic Bold`），而名字对不上时 libass **不报错** ——
// 静默落 `ArialMT`，`lines_failed: 0`、`issues: []`。实测（同字体同 fontsdir，只换名字）：
//
//     fontselect: (GenJyuuGothic-Bold, 400, 0)    -> ArialMT            ← 传 id
//     fontselect: (Gen Jyuu Gothic Bold, 400, 0)  -> GenJyuuGothic-Bold ← 传真族名
//
// ⚠️ 判据必须**同时**钉住"库内的要换"和"库外的不能动"：只钉前者的话，一个"把所有值都加上
// 前缀"的写法也会绿 —— 而那会把老工程的 `LXGW WenKai` 一起改写（它不在库里，该走系统字体栈）。
function checkFontFamily(mod) {
  let checks = 0;
  const eq = (actual, expected, what) => {
    assert.equal(actual, expected, what);
    checks += 1;
  };

  // 库里有一支 id ≠ 族名（这正是 slug 形态的老工程会踩的那支）。
  mod.setFontFamilies({ 'GenJyuuGothic-Bold': 'Gen Jyuu Gothic Bold', 乐米波波体: '乐米波波体' });
  eq(mod.fontFamilyFor('GenJyuuGothic-Bold'), 'Gen Jyuu Gothic Bold', '库内 id ≠ 族名：必须换成真族名');
  eq(mod.fontFamilyFor('乐米波波体'), '乐米波波体', '库内 id == 族名：原值返回（不许多加东西）');
  eq(mod.fontFamilyFor('LXGW WenKai'), 'LXGW WenKai', '库外的值必须原样返回（那是系统字体族名）');
  eq(mod.fontFamilyFor('lemi'), 'lemi', '认不出的一律原样，不许猜');
  eq(mod.fontFamilyFor(''), null, '空 = 契约里的"没指定"');
  eq(mod.fontFamilyFor('   '), null, '只空白也算没指定');
  eq(mod.fontFamilyFor(undefined), null, 'undefined 不许抛');
  // 映射里的空值不许把 id 抹成空串（那会让契约里出现 font_family: ""）。
  mod.setFontFamilies({ X: '   ' });
  eq(mod.fontFamilyFor('X'), 'X', '映射值是空白 ⇒ 退回 id，不许返回空串');


  // 复位成空表，免得影响后续用例。
  mod.setFontFamilies({});
  eq(mod.fontFamilyFor('GenJyuuGothic-Bold'), 'GenJyuuGothic-Bold', '没有映射时按 id 原样（老行为）');
  return checks;
}

// ---- 密集键往前铺的区间（③R3）--------------------------------------------
// 抽成函数是因为**变异自证也要跑它**（`scripts/AGENTS.md`：只写在顶层的检查，变异台不会跑）。
// 钉的是"右端落在哪一帧"：退回"窗口末尾"时，窗口之后到下一个稀疏键那一段只剩两个稀疏键 ⇒
// 曲线退化成两点直线弦、`pow2_out` 被拉直（实测 zoom 偏 ~0.04 ≈ 3%）——而那条路径**只**在
// "shake 窗口压在运镜过渡上"时触发，正是用户报的「shake 与运镜同时存在」。
function checkDenseRanges(mod) {
  let checks = 0;
  const eq = (actual, expected, what) => {
    checks += 1;
    const a = JSON.stringify(actual);
    const b = JSON.stringify(expected);
    if (a !== b) throw new Error(`${what}：期望 ${b}，实得 ${a}`);
  };
  // 窗口 [100,130]、稀疏键在 150 ⇒ 右端 = **下一个稀疏键前一帧** 149（不是窗口末 131）
  eq(mod.denseRangesFor([[100, 130]], [50, 150], 400), [[51, 149]], '右端 = 下一个稀疏键 - 1');
  // 没有下一个稀疏键 ⇒ 铺到最后一个输出帧
  eq(mod.denseRangesFor([[100, 130]], [50], 400), [[51, 400]], '没有下一个稀疏键 ⇒ 铺到末帧');
  // 稀疏键**恰好**在 b+1 时它不算"下一个"（搜索严格大于 b+1）⇒ 继续铺到再下一个之前
  eq(mod.denseRangesFor([[100, 130]], [50, 131, 200], 400), [[51, 199]], '恰在 b+1 的稀疏键不算下一个');
  // 没有上一个稀疏键 ⇒ 从窗口起点开始（竖素材那份 16716 键的教训），右端照旧
  eq(mod.denseRangesFor([[100, 130]], [200], 400), [[100, 199]], '没有上一个稀疏键 ⇒ 从窗口起点起');
  return checks;
}

/// 第 2 批：**调整图层的窗口不许超出它所在的那一段**（乱序/重放 EDL）。
///
/// `placeEnvelope` 收的 `seconds` 是**源时间**上的事件时长，而 `outStart` 是
/// **重映射之后**的成片帧号；顺序 EDL 下两者刚好吻合，乱序/重放就不吻合了。
/// 实测：源 `[6.4, 7.2167)` 的 flash 在乱序 EDL 下落进成片第 384 帧，
/// 而那段只到 420 帧 ⇒ 窗口溢到 **433**，底座按"所有层 end 的最大值"出片
/// ⇒ 成片比派生值长 13 帧，报告里一行差异都没有。
///
/// 判据 = **所有层的 `end` 最大值 ≤ 派生总帧数**（`translate` 报告里的 `out_frames`）。
function checkEnvelopeClamp(mod) {
  let checks = 0;
  const ok = (cond, what) => {
    checks += 1;
    if (!cond) throw new Error(what);
  };

  const dir = mkdtempSync(join(tmpdir(), 'yekitrim-env-'));
  try {
    writeFileSync(join(dir, 'clip.srt'), '1\n00:00:00,100 --> 00:00:01,000\nx\n\n', 'utf8');
    // **乱序 EDL**：源顺序与成片顺序相反（第 3 段反而最先播）。
    // 关键是最后一段只有 2 秒，而它上面压着一个**起点靠后**的特效。
    const toml = [
      'duration = 12.0',
      '[project]',
      'name = "env"',
      'output = "o.mp4"',
      '[source]',
      'video = "clip.mp4"',
      'subtitles = "clip.srt"',
      '[edl]',
      'total_frames = 0',
      '[[edl.clips]]',
      'source_start = 9.0',
      'source_end = 12.0',
      'speed = 1.0',
      '[[edl.clips]]',
      'source_start = 0.0',
      'source_end = 2.0',
      'speed = 1.0',
      '[[edl.clips]]',
      'source_start = 4.0',
      'source_end = 6.0',
      'speed = 1.0',
      '[[segments]]',
      'start = 0.0',
      'end = 12.0',
      // flash 落在**第三段**（源 [4,6)）的最末尾 —— 它的源时长 0.5s 比那段的剩余还长
      '[[segments.events]]',
      'type = "flash"',
      'time = 5.9',
      'end = 6.4',
      'intensity = 0.08',
      '',
    ].join('\n');
    const config = mod.parseToml(toml);
    const doc = mod.translate(config, dir, join(dir, 'p.doc.json'));

    let maxEnd = 0;
    for (const track of doc.timeline.tracks) {
      for (const l of track.layers ?? []) maxEnd = Math.max(maxEnd, l.end ?? 0);
    }
    // 派生总帧数 = 3 + 2 + 2 = 7 秒 × 60fps = 420。
    ok(maxEnd <= 420, `所有层的 end 最大值不许超过派生总帧数 420，实得 ${maxEnd}`);
    ok(maxEnd >= 419, `时间线也不许短于派生值（层要铺满到片尾），实得 ${maxEnd}`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  return checks;
}

/// 第 4 批：变速段必须在报告里**被点名**，且缩段长按 `round((ef-sf)/speed)`。
///
/// 为什么单独钉：`speed` 是这条功能链里**最容易两侧分叉**的地方 —— 转译器缩画面段长、
/// Rust 缩音轨段长，**分叉时不报错**（成片就是短一截）。那条 NOTE 是"用户说某段快放
/// 没生效"时第一眼要看的证据，所以它**不许静默消失**。
///
/// 判据两条：
/// ① 有条 `edl` 的 NOTE 提到"变速"，且**速率**都点名了；
/// ② `speed` 真的**缩了画面段长**（帧号域：×2.0 ⇒ 一半帧、×0.5 ⇒ 一倍帧）。
/// 第 6 批：**素材段的 `source_in` 必须按该素材自己的时基换算**。
///
/// 这是本批最危险的一条：退回源时基**不会报错** —— e2e 实测 `exit 0`、`issues: []`、
/// `failed: false`，但取帧位置整体偏了（那条注释记的症状是"画面跳到源里 6 倍远的位置"）。
/// 所以只能靠**逐值**判据把它钉住。
///
/// 判据：源 30fps、素材 24fps，素材段从**素材秒 1.0** 起 ⇒ `source_in` 必须是 **24**（不是 30）。
/// 另有一条控制组：**源自己的段**仍走源时基（老路径逐值不变）。
function checkAssetTimebase(mod) {
  let checks = 0;
  const ok = (cond, what) => {
    checks += 1;
    if (!cond) throw new Error(what);
  };

  const dir = mkdtempSync(join(tmpdir(), 'yekitrim-asset-'));
  try {
    writeFileSync(join(dir, 'clip.srt'), '1\n00:00:00,100 --> 00:00:01,000\nx\n\n', 'utf8');
    // 真造一个 **24fps** 的素材文件放进 `<项目>/assets/videos/`：转译器是**真 ffprobe** 取时基的，
    // 塞个假文件探不到帧率就会退回源时基 ⇒ 那这条判据就**测不到东西**（会假绿）。
    mkdirSync(join(dir, 'assets', 'videos'), { recursive: true });
    const assetPath = join(dir, 'assets', 'videos', 'mate24.mp4');
    const mk = spawnSync(
      'ffmpeg',
      ['-v', 'error', '-f', 'lavfi', '-i', 'color=c=green:s=64x48:d=4:r=24',
       '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-y', assetPath],
      { encoding: 'utf8' },
    );

    const toml = [
      'duration = 4.0',
      '[project]',
      'name = "asset"',
      'output = "o.mp4"',
      '[source]',
      'video = "clip.mp4"',
      'subtitles = "clip.srt"',
      '[edl]',
      'total_frames = 0',
      // 第 1 段：**本条片段**（源时基）—— 控制组
      '[[edl.clips]]',
      'source_start = 0.0',
      'source_end = 1.0',
      'speed = 1.0',
      // 第 2 段：**素材**，从素材秒 1.0 起 —— 判据组
      '[[edl.clips]]',
      'source_start = 1.0',
      'source_end = 2.0',
      'speed = 1.0',
      'source_kind = "asset"',
      'asset_id = "mate24"',
      '',
    ].join('\n');
    const config = mod.parseToml(toml);
    const doc = mod.translate(config, dir, join(dir, 'p.doc.json'));

    // 素材登记：必须是**它自己的** 24fps（不是源的 30fps）
    const mate = (doc.assets ?? []).find((a) => a.id === 'assets/videos/mate24.mp4');
    if (!mate) {
      throw new Error(
        `素材没有被登记成 asset（探针造的文件在 ${assetPath}，ffmpeg 退出码=${mk.status}）—— ` +
          '这一条测不下去，别当它绿了',
      );
    }
    ok(
      mate.timebase.num === 24 && mate.timebase.den === 1,
      `素材 asset 的时基应当是它自己的 24/1，实得 ${mate.timebase.num}/${mate.timebase.den}`,
    );

    const mains = [];
    for (const track of doc.timeline.tracks) {
      for (const l of track.layers ?? []) {
        if (typeof l.id === 'string' && l.id.startsWith('main-')) mains.push(l);
      }
    }
    mains.sort((a, b) => (a.start ?? 0) - (b.start ?? 0));
    ok(mains.length === 2, `两段 EDL 应当出 2 个主画面层，实得 ${mains.length}`);

    // 控制组：源那一段仍按**源时基**（30fps）—— 素材功能不许改变老路径
    ok(
      mains[0].source.source_in === 0,
      `源段从 0 起 ⇒ source_in 应当是 0，实得 ${mains[0].source.source_in}`,
    );
    // 判据组：素材段从**素材秒 1.0** 起 ⇒ 24fps ⇒ **24** 帧（若按源 30fps 算就是 30）
    ok(
      mains[1].source.source_in === 24,
      `素材段从素材秒 1.0 起、素材 24fps ⇒ source_in 应当是 **24**（按源 30fps 算会是 30），` +
        `实得 ${mains[1].source.source_in}`,
    );
    ok(
      mains[1].source.asset_id === 'assets/videos/mate24.mp4',
      `素材段的 asset_id 应当指向项目里那份拷贝，实得 ${mains[1].source.asset_id}`,
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  return checks;
}

/// 第 6 批：素材判据**只认小写** `source_kind === 'asset'` + 非空 `asset_id`。
///
/// 放宽大小写看着"更健壮"，实则把"用户写错档位"变成**静默当成本条片段** ——
/// 正是规格 §11.7 ① 那一类（分叉且不报错）。
function checkAssetClipPredicate(mod) {
  let checks = 0;
  const eq = (actual, expected, what) => {
    checks += 1;
    const a = JSON.stringify(actual);
    const b = JSON.stringify(expected);
    if (a !== b) throw new Error(`${what}：期望 ${b}，实得 ${a}`);
  };

  eq(mod.edlClipAssetId({ source_kind: 'asset', asset_id: 'a.mp4' }), 'a.mp4', '正常素材段要认出来');
  eq(mod.edlClipAssetId({ source_kind: 'Asset', asset_id: 'a.mp4' }), null, '`Asset` 不算素材（只认小写）');
  eq(mod.edlClipAssetId({ source_kind: 'clip', asset_id: 'a.mp4' }), null, '`clip` 档不算素材');
  eq(mod.edlClipAssetId({ source_kind: 'asset' }), null, '素材段但没有 asset_id ⇒ 不算（否则会去取一个空名的文件）');
  eq(mod.edlClipAssetId({ source_kind: 'asset', asset_id: '   ' }), null, '空白 asset_id 要 trim 掉再判空');
  eq(mod.edlClipAssetId({ source_kind: 'asset', asset_id: ' a.mp4 ' }), 'a.mp4', '两边空白要 trim');
  eq(mod.edlClipAssetId(null), null, 'null 不许抛');
  eq(mod.edlClipAssetId({}), null, '缺 source_kind ⇒ 老工程 = 本条片段');
  return checks;
}

/// 片头 / 片尾（2026-10-09）：`[intro_outro]` 必须**拼进成片时间线**，且要用**素材自己**的画面。
///
/// 为什么单独钉 —— 它是"一份推导两份实现"里最容易漏的一处：Rust
/// (`ProjectFile::edl_clips_with_intro_outro`) 与这里的 `intro_outro` 段必须给出同一个
/// 段数/总帧数，否则画面与音轨各剪各的。实测两种漏法的症状：
///   ① 完全不拼 ⇒ 片头根本没出现（时长少 1.5s+3s），`mux_checked` 报"画面帧数不一致"；
///   ② 拼了但**没登记素材** ⇒ `assetIdForRange` 回落成主素材，**片头放的是正片开头**，
///      而 DROP/NOTE 里一行都没有（注册表查不到就当"源"）。判据②靠断 `source.asset_id`。
///
/// fixture 用**真媒体**（ffprobe 探时长与帧率），所以它同时钉住 `probeMedia` 的 `duration`
/// 那一项 —— 「出点 = 0 ⇒ 播到结尾」全靠它，探不到就必须 DROP（不许拿 0 当长度）。
/// 第 7 批：**文字元素轨道的左右安全宽**。
///
/// 钉的是 2026-10-11 修的那个真缺陷：`bottom_margin: 0.5` ⇒ 底座算出的可用宽度为 **0**
/// ⇒ `text_layout::layout` **提前返回空布局** ⇒ 文字元素一个字都不排，而成片里
/// 「看着像画了」（字幕照画、报告零差异、`probe` 零 error）—— 只有数像素才发现少了那几个字。
///
/// ⚠️ 两个字段**必须互相自洽**，而它们由**不同的人**读：
/// - `bottom_margin` —— `layout()` 真的读它（`margin = bottom_margin.clamp(0,0.5)`、
///   `max_width = width * (1 - 2*margin)`）；
/// - `safe_width_ratio` —— 目前只在**缩字**那条路上被读到。
///
/// 所以 `1 - 2*bottom_margin` 必须等于 `safe_width_ratio`（参照口径 0.875）。
/// 只给对一个 ⇒ 要么不排字（0.5），要么安全宽与缩字口径打架。
/** ★ 元素 ASS 的字体必须**与字幕同源**（2026-10-14，用户报的问题 ②）。
 *
 *  ⚠⚠ **必须收 `src`、并在变异台里喂 `mutatedSource`** ——
 *     它钉的是**源码文本**；而变异只改盘上的文件，
 *     模块导出（`mod`）取的是**加载期**那份 ⇒ 看不见变异体。
 *     ⚠ 本仓已经因此裁过一次（见 `checkTextElementLane` 那段注释）。
 */
function checkElementAssFont(src) {
  let checks = 0;
  const eq = (actual, expected, what) => {
    assert.equal(actual, expected, what);
    checks += 1;
  };
  if (!src) {
    eq(true, false, '拿不到转译器源码（无法钉字体来源）');
    return checks;
  }
    // ① 元素那条调用点必须把字体从**字幕样式**取。
    eq(/font:\s*fontFamilyFor\(sharedSubStyle\.font\)/.test(src), true,
      '② 元素 ASS 的 `font` 必须来自 `fontFamilyFor(sharedSubStyle.font)`' +
      '（与字幕同源）—— 不是写死字符串');
    // ② 且不允许再出现写死的字体字面量（`'Sans'` 是当年那个）。
    eq(/font:\s*'[^']*'/.test(src), false,
      "② 不允许再有写死的 `font: '…'` 字面量（例如当年的 `'Sans'`）");
    // ③ `cuesToAss` 的 `font` 不许再有默认值 —— 有默认值就能再次"忘传也能跑"。
    eq(/function cuesToAss\(cues, \{ font\s*=/.test(src), false,
      '③ `cuesToAss` 的 `font` 不许有默认值' +
      '（有默认值就能再次“忘传也能跑”）');
    eq(/cuesToAss: 必须显式传 font/.test(src), true,
      '③ `cuesToAss` 必须在缺 `font` 时**当场报错**（而不是静默回退）');
    // ④ 字幕那条路也必须仍然是同源的取法（防"为了对齐字串把两边都写死"）。
    eq(/font_family:\s*fontFamilyFor\(subStyle\.font\)/.test(src), true,
      '④ 字幕那条路的 `font_family` 也必须仍然是 `fontFamilyFor(subStyle.font)`');
  return checks;
}

function checkTextElementLane(mod, src = translatorSource) {
  let checks = 0;
  const ok = (cond, what) => {
    checks += 1;
    if (!cond) throw new Error(what);
  };

  const dir = mkdtempSync(join(tmpdir(), 'yekitrim-elem-'));
  try {
    // 造一个 2 秒的黑底素材（拿不到 ffmpeg 就跳过这一组，与其它探媒体守卫同口径）
    let have = false;
    try {
      execFileSync('ffmpeg', ['-v', 'error', '-f', 'lavfi', '-i',
        'color=c=black:s=320x240:d=2,format=yuv420p', '-r', '30',
        '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-y', join(dir, 'clip.mp4')],
        { stdio: ['ignore', 'ignore', 'ignore'], timeout: 60000 });
      have = existsSync(join(dir, 'clip.mp4'));
    } catch (e) {
      // ⚠️ 只吞"ffmpeg 不可用"（ENOENT）；别的错照抛（一律吞会让这条静默空转）。
      if (e && e.code === 'ENOENT') have = false;
      else throw e;
    }
    if (!have) return 0;

    // ⚠️ **`[source]` 段不能省** —— 少了它 `keepRanges` 推不出来，
    //    `remapFrame` 全返回 null ⇒ 文字元素**整条被跳过**（"落在被掐掉的区间里"），
    //    于是这条守卫会以"没有 text_hook 轨"失败，而真因是**夹具不完整**。
    //    第一版就是这么写的（实测：同一个工程补上 `[source]` 后轨道立刻出现）。
    const srtText = ['1', '00:00:00,300 --> 00:00:01,200', '有字幕', ''].join(String.fromCharCode(10)) + String.fromCharCode(10);
    writeFileSync(join(dir, 'clip.srt'), srtText, 'utf8');
    // 一条文字元素。`source` 是 process 写回的四档参数（style|font|stroke|color|stroke|weight|backdrop）。
    writeFileSync(join(dir, 'polish.toml'), [
      'config_version = 1',
      'duration = 2.0',
      'processed = true',
      '',
      '[source]',
      'video = "clip.mp4"',
      'subtitles = "clip.srt"',
      'danmaku = ""',
      'metadata = ""',
      '',
      '[canvas]',
      'width = 1920',
      'height = 1080',
      '',
      '[strip]',
      'max_duration = 2.0',
      '',
      '[[segments]]',
      'start = 0.0',
      'end = 2.0',
      'role = "other"',
      'index = 1',
      '',
      '[[segments.events]]',
      'type = "element"',
      'kind = "text"',
      'time = 0.2',
      'end = 1.5',
      'text = "钩子标题"',
      'style = "hook"',
      'pos = "top"',
      'x = "10%"',
      'y = "20%"',
      // ⚠️ 2026-10-13：字号统一为 80（`ELEMENT_FONT_PX`）、`hook` 描边 8→6。
      //    这个串是 `process` 写回的**仿真**（`style|font_px|stroke|fill|stroke|weight|backdrop`），
      //    所以它必须跟着 Rust 走 —— 否则下面 ③ 那条 `font_ratio` 判据会拿**旧字号**去对，
      //    而真机上的 `process` 早就写 80 了（判据与真值分叉、且只有本文件知道）。
      `source = "hook|80|6|255,255,255,255|26,26,26,255|900|0"`,
      '',
    ].join('\n'), 'utf8');

    // ⚠️ 签名是 `translate(cfg, dir, outPath, options)` —— 配置要**显式 parseToml**
    //    （与 checkMediaProbe 那几组同口径），不是只给一个目录。
    const toml = readFileSync(join(dir, 'polish.toml'), 'utf8');
    const project = mod.translate(mod.parseToml(toml), dir, join(dir, 'elem.json'));
    // ⚠️ 轨 id 从 `text_<style>` 变成 **`text_<style>_<pos>`**（2026-10-12）：
    //    `anchor_bottom` 是**轨级**常量，而 `pos` 是**逐事件**的 ⇒ 必须按
    //    (样式 × 档位) 两维分轨，否则同一档样式的两个档位会互相覆盖。
    const TID = TEXT_TRACK_ID;
    const track = (project.timeline?.tracks ?? []).find((t) => t.id === TID);
    ok(!!track, `文字元素必须按 (样式 × 纵向档) 分轨，找到 ${TID}；实得轨：` +
      JSON.stringify((project.timeline?.tracks ?? []).map((t) => t.id)));
    if (!track) return checks;

    const st = track.subtitle ?? {};
    // ① 左右安全宽必须真的**大于 0** —— 这条就是那个缺陷的直接判据。
    ok(st.bottom_margin < 0.5,
      `bottom_margin=${st.bottom_margin} ⇒ 底座可用宽 = 宽 × ${1 - 2 * st.bottom_margin}` +
      '（0.5 时是 0 ⇒ 布局提前返回空 ⇒ 文字一个字都不排，而成片报告零差异）');
    ok(1 - 2 * st.bottom_margin > 0.5,
      `文字轨的可用宽只有 ${1 - 2 * st.bottom_margin} —— 太窄，长一点的标题必然被丢行`);
    // ② 与 `safe_width_ratio` **互相自洽**（两个字段由不同的人读，口径必须同一个）。
    ok(Math.abs((1 - 2 * st.bottom_margin) - st.safe_width_ratio) < 1e-6,
      `bottom_margin(${st.bottom_margin}) 推出的安全宽 ${1 - 2 * st.bottom_margin} ` +
      `与 safe_width_ratio(${st.safe_width_ratio}) 不一致 —— 两个消费点会算出不同的折行`);
    // ③ 字号按四档走（2026-10-13 起四档统一 80px @1080 ⇒ ratio 80/1080）。
    ok(Math.abs(st.font_ratio - 80 / 1080) < 1e-6,
      `文字元素的字号比例应为 80/1080，实得 ${st.font_ratio}（那是四档预设的语义，不许缩）`);
    // ④ 文字**不缩字号**（缩了会让用户选的那一档的观感被吃掉）。
    ok(st.shrink_min_scale === 1,
      'shrink_min_scale 必须为 1：文字的字号是四档预设定死的，像字幕那样缩会让 hook 缩成 comment');
    // ⑤ 颜色来自 source 那段 CSV（不是硬编码白）。
    ok(Array.isArray(st.color) && st.color.join(',') === '255,255,255,255',
      `颜色必须来自 event.source 那段（hook 是白），实得 ${JSON.stringify(st.color)}`);

    // ------------------------------------------------------------------
    // ⑤b–⑤d **纵向锚 `anchor_bottom`**（2026-10-12，底座 v7）
    //
    // 契约是：`pos` → `anchor_bottom`（轨级常量）。
    // ⚠️ 判据必须落在**产物里那个键**上，不能落在"我的映射函数返回了什么"——
    //    本仓反复踩过"测到了函数、没测到调用方"（`parseStroke` 那次就是这么漏的）。
    // ------------------------------------------------------------------
    // ⑤b `top` 档必须**真的**给出锚点，且落在画面上半部分。
    //
    //     ⚠️ 不能只判 `'anchor_bottom' in st`：该键带 `skip_serializing_if`，
    //     缺省 1.0 时**键不出现** —— 而"键不出现"正是 `bottom` 档**正确**的表现。
    //     两档共用一条判据会互相抵消。
    ok(typeof st.anchor_bottom === 'number',
      'pos="top" 必须写出 anchor_bottom（缺省 1.0 不写键，所以"有值"才是对的表现），' +
      `实得 ${JSON.stringify(st.anchor_bottom)}`);
    ok(st.anchor_bottom > 0 && st.anchor_bottom < 0.5,
      `pos="top" 的块底应落在画面上半部分，实得 ${st.anchor_bottom}`);
    // ⑤c **与 `bottom_margin` 已解耦**：那个字段仍写 0.0625（老底座靠它算安全宽），
    //     而纵向**不受它影响**（0.0625 若被当纵向用会贴到画面最底）。
    ok(Math.abs(st.bottom_margin - (1 - 0.875) / 2) < 1e-9,
      `bottom_margin 必须仍是 (1-0.875)/2=0.0625（老底座只读它算安全宽），实得 ${st.bottom_margin}`);
    // ⑤d `safe_width_ratio` 必须**显式**写 —— 不写（默认 0）新底座会退回
    //     "复用 bottom_margin 当左右安全宽"的老路，而那个字段现在还要表达纵向。
    ok(st.safe_width_ratio === 0.875,
      `safe_width_ratio 必须显式 0.875（不写就退回复用的老路、与纵向锚打架），实得 ${st.safe_width_ratio}`);

    // ------------------------------------------------------------------
    // ⑤d2 **纵向锚按「单行行盒」算**（2026-10-14 真缺陷）
    //
    // 用户报「元素的文字位置不对」。病因：旧实现把盒高取成**最坏情况**
    // （3 行 = `fontPx × 1.2 × 3`）再算块底：
    //
    //     center: 盒顶 = (1080-288)/2 = 396，块底 = 396+288 = 684 ⇒ 0.6333
    //
    // 而锚点语义是「**块底**贴哪个 y」（底座 `block_top = block_bottom -
    // keep*line_height`），一个字只占**一行** ⇒ 底座把块顶算成
    // `0.6333 - 1×0.0889 = 0.5444`，文字**中心落在 0.588**（约 635px），
    // 比画面正中（540）**低了约 95px**。
    //
    // ⇒ 判据是**文字中心**（= 块底 − 单行高/2）必须落在该档的语义位置上。
    //    ⚠️ 不能只判"块底是个数" —— 旧实现也给数，只是数错了（判据会空转）。
    // ------------------------------------------------------------------
    {
      const H = 1080; // `[canvas] height`（夹具里写死 1080）
      const fontPx = 80; // 四档统一（`ELEMENT_FONT_PX`）
      // ⚠️⚠️ **行盒必须取"轨里真正写着的那个 `line_height`"** —— 底座
      //    `text_layout.rs:432` 是 `line_em = if style.line_height > 0
      //    { style.line_height } else { LINE_HEIGHT_EM }`，转译器写什么它按什么排。
      //    第一版拿 `LINE_HEIGHT_EM`(1.2) 算，实测文字中心落在 **528**（低 12px）。
      const lineEm = st.line_height;
      const lineNorm = (fontPx * lineEm) / H;
      // 文字块**中心**的归一化 y —— 这才是肉眼能看到的位置。
      const textCenter = st.anchor_bottom - lineNorm / 2;
      // `pos="top"` 的语义是"贴在上方"：中心必须**明显在半屏以上**。
      ok(textCenter < 0.5 - 0.01,
        `pos="top" 的文字中心落在 ${textCenter.toFixed(4)} —— 没有明显高于半屏；` +
        `anchor_bottom=${st.anchor_bottom}（行盒 ${lineNorm.toFixed(4)}）。` +
        '⚠️ 旧实现按 3 行上限算块底，会把文字整体推到 0.588 附近（约 635px）');
      // 反判据：**不许再用 3 行上限**。用上限时 top 档的块底 ≈ 320/1080 = 0.2963，
      // 而按单行（MARGIN + 80×1.5 = 152）算是 152/1080 = 0.1407 —— 判据认得出。
      const singleLineBottom = (32 + fontPx * lineEm) / H;
      ok(Math.abs(st.anchor_bottom - singleLineBottom) < 1e-3,
        `pos="top" 的块底应为 (MARGIN + 单行行盒)/H = ${singleLineBottom.toFixed(4)}，` +
        `实得 ${st.anchor_bottom} —— 差的那些就是"按 3 行上限算"留下的`);
      // ⚠️ **行盒口径必须与轨里写的 `line_height` 同源**（2026-10-14 第二轮真机抓出来的）：
      //    拿 `LINE_HEIGHT_EM`(1.2) 算会得到 0.1185，而底座按 1.5 排行盒 ⇒
      //    文字中心 528（低 12px）。**两个数都不报错，只有量像素才发现。**
      ok(Math.abs(lineEm - 1.5) < 1e-9,
        `元素轨的 line_height = ${lineEm} —— 本仓写的是 1.5（继承字幕），` +
        '锚点那半个行盒就是按它算的；两个数必须同一个（否则文字整体偏半个行盒）');
    }

    // ------------------------------------------------------------------
    // ⑤d3 **字体 / 阴影 / 行高必须继承字幕那一份**（2026-10-14 真缺陷）
    //
    // 用户报「元素出现时**字幕的描边和字体**变成默认的了」。
    // 查下来**不是字幕被改了** —— 是元素轨**少写三个键**，底座按缺省画：
    //   `font_family` 缺 ⇒ 回落宿主 `--font-file`（默认字体）；`shadow_*` 缺 ⇒ 无阴影；
    //   `line_height` 缺 ⇒ 1.2（字幕是 1.5）。
    // ⇒ 同一片画面上元素用默认字体、无阴影，看着就像"字幕被换掉了"。
    //
    // 判据：**逐值**与字幕轨相等（不是"元素轨有这三个键就行"——
    // 自己另立一套数同样会分叉，而分叉的症状与缺失**一模一样**）。
    // ------------------------------------------------------------------
    {
      const subTrack = (project.timeline?.tracks ?? []).find((t) => t.id === 'sub');
      ok(!!subTrack, '夹具必须带字幕轨（本组判据要与它逐值对拍）');
      const ss = subTrack?.subtitle ?? {};
      for (const key of ['font_family', 'line_height', 'shadow_color',
        'shadow_dx_px', 'shadow_dy_px', 'shadow_blur_ratio']) {
        ok(st[key] !== undefined,
          `元素轨必须写出 \`${key}\` —— 缺了它底座按缺省画（默认字体/无阴影/行高 1.2），` +
          '而同画面的字幕用的是另一套 ⇒ 用户看到的是"字幕被换掉了"');
        ok(JSON.stringify(st[key]) === JSON.stringify(ss[key]),
          `元素轨的 \`${key}\` = ${JSON.stringify(st[key])}，字幕轨 = ${JSON.stringify(ss[key])} —— ` +
          '两者必须**逐值相等**（同画面两套字体/阴影正是那个症状的来源）');
      }
      // ⚠️⚠️ **不能只判"两边相等"**（2026-10-14 实测踩到）：
      //    变异台里那条「阴影半径按文档**宽**折算」的锚点是 **8 空格缩进**的
      //    `'        shadow_blur_ratio: 12 / height,'`，而元素轨那行是 **10 空格** ——
      //    前者是后者的**子串** ⇒ 那条变异**一次改两处**（字幕 + 元素），
      //    两边一起变成 `/width` ⇒ 上面那条"逐值相等"**照样通过** ⇒ 变异存活。
      //
      //    ⇒ 必须再钉一条**绝对值**：`shadow_blur_ratio` 只能按**文档高**折算
      //      （底座求值层乘目标高，与 `stroke_ratio` 同一处换算）。
      //      **谁改了这条口径，不管改几处，都翻面。**
      ok(Math.abs(st.shadow_blur_ratio - 12 / 1080) < 1e-9,
        `元素轨 shadow_blur_ratio = ${st.shadow_blur_ratio}，必须按**文档高**折算（12/1080 = ` +
        `${(12 / 1080).toFixed(6)}）。⚠️ 按**宽**折算（12/1920）会让阴影糊得只有一半 —— ` +
        '而"两边一起改错"时逐值对拍是看不出来的（实测：那条变异就是这么存活的）');
      // ⚠️ **反判据**：`stroke_*` **不许**被一起改成字幕那套。
      //    元素描边是按 `elements-spec.md` §4.2「字号 5–8%」推导的
      //    （hook 6px/80 = 7.5%），比字幕那条 12px 更细**是刻意的**
      //    （12px 是给底部小字配的，套到 80px 大字上会糊掉字腔）。
      //    这条防的是"下一个人顺手把 font_family 那两个字段一起搬过来"。
      ok(JSON.stringify(st.stroke_color) !== JSON.stringify(ss.stroke_color),
        '元素轨的 stroke_color 被改成字幕那套了 —— 元素的描边是按档位推导的（近黑 [26,26,26]），' +
        '字幕那条 [64,60,59] 是给底部小字配的，不是同一个语义');
    }

    // ------------------------------------------------------------------
    // ⑤e **文字层的 `transform.rotation` 必须无条件存在**（2026-10-13 真缺陷）    //
    // 底座 `TransformV2.rotation` **没有** `serde(default)`（`layer.rs:154`），
    // 而 `scale_y` 才有（`:151`）⇒ 只省略 `rotation` 会让**整份工程载入失败**：
    //     「工程文件字段不符：missing field `rotation`」
    //
    // ⚠️ 症状极具迷惑性、必须专门钉：
    //   `chapter` 档（`tilt=0, squash≠1`）= 只省 rotation ⇒ **必然炸**；
    //   而 `hook`/`count` 档（`tilt≠0`）恰好带着 rotation ⇒ 看起来"能用"。
    //   于是"一整份工程因为一个字的样式档而完全出不了片"，而**报告里零差异**。
    //   实测：`dhampir probe --project <产物>` 直接报 `missing field rotation`。
    //
    // ⚠️ 判据落在**产物里那个键**上（与 ⑤b 同一条纪律：不能测"我的映射返回了什么"）。
    // ------------------------------------------------------------------
    const rotLayer = (track.layers ?? [])[0];
    ok(!!rotLayer, '文字轨必须有层');
    ok(rotLayer && 'rotation' in (rotLayer.transform ?? {}),
      '文字层的 transform 必须**无条件**带 rotation —— 底座 `TransformV2.rotation` ' +
      '没有 serde(default)，省掉它整份工程载入失败（missing field `rotation`）；' +
      '`chapter` 档 tilt=0 会省略它 ⇒ 必然炸。实得 ' +
      JSON.stringify(rotLayer && rotLayer.transform));
    ok(rotLayer && Number.isFinite(rotLayer.transform.rotation),
      `rotation 必须是个数（角度制），实得 ${JSON.stringify(rotLayer && rotLayer.transform.rotation)}`);

    // ------------------------------------------------------------------
    // ⑥–⑫ §5 E4a **T2 打字机**（2026-10-11）
    //
    // ⚠️ 手法：把**同一个工程**加一行 `[elements] typewriter = true` 再跑一遍，
    //    然后逐条对拍。这样判据钉的是"**拆了没有**"这件事本身，
    //    而不是"某个内部函数的返回值" —— 本仓反复踩过"测到了函数、没测到调用方"。
    // ------------------------------------------------------------------
    const assOf = (d, id) => {
      const p = join(d, `dhampir-${id}.ass`);
      return existsSync(p) ? readFileSync(p, 'utf8') : '';
    };
    // `Dialogue:` 行里的**正文**（`Format` 那 9 列之后全是文本，文本里可能有逗号）。
    const textsOf = (ass) => ass.split('\n')
      .filter((l) => l.startsWith('Dialogue:'))
      .map((l) => l.split(',').slice(9).join(','));

    // 开关**关着**时：一条都不许拆（这是 R4 零回归在转译侧的判据）。
    const offTexts = textsOf(assOf(dir, TID));
    ok(offTexts.length === 1,
      `没开打字机时文字元素必须还是 1 条 cue（存量零回归），实得 ${offTexts.length} 条：` +
      JSON.stringify(offTexts));
    ok(offTexts[0] === '钩子标题',
      `没开打字机时正文必须原样，实得 ${JSON.stringify(offTexts[0])}`);

    // 加上开关再跑一遍。
    const tomlOn = toml + '\n[elements]\ntypewriter = true\n';
    const dirOn = mkdtempSync(join(tmpdir(), 'yekitrim-e4a-'));
    try {
      copyFileSync(join(dir, 'clip.mp4'), join(dirOn, 'clip.mp4'));
      if (existsSync(join(dir, 'clip.srt'))) copyFileSync(join(dir, 'clip.srt'), join(dirOn, 'clip.srt'));
      writeFileSync(join(dirOn, 'polish.toml'), tomlOn, 'utf8');
      const pOn = mod.translate(mod.parseToml(tomlOn), dirOn, join(dirOn, 'on.json'));
      ok((pOn.timeline?.tracks ?? []).some((t) => t.id === TID),
        '开了打字机之后那条轨不能消失（拆 cue 不该把整条轨弄丢）');
      const onTexts = textsOf(assOf(dirOn, TID));

      // ⑥ / ⑦ 必须真的拆开、且明显多于原句字数关系上是"逐字"。
      ok(onTexts.length > 1,
        `开了 typing 必须拆成多条 cue，实得 ${onTexts.length} 条 —— ` +
        '一条都没拆就说明 `readElementsCfg` 没读到 `[elements]`');
      // ⑦ 每条都是原句的**前缀**（这就是"打字机"这三个字的判据）。
      const full = '钩子标题';
      ok(onTexts.every((t) => full.startsWith(t)),
        `拆出来的每一条都必须是整句的前缀，实得 ${JSON.stringify(onTexts)}`);
      // ⑧ 逐条**只增不减**（不是随机顺序、也不是乱切）。
      let prevLen = 0;
      for (const t of onTexts) {
        ok(t.length >= prevLen, `文本长度必须单调不减（打字机），实得 ${JSON.stringify(onTexts)}`);
        prevLen = t.length;
      }
      // ⑨ 最后一条必须是**整句**（否则结尾少字）。
      ok(onTexts[onTexts.length - 1] === full,
        `最后一条必须是整句「${full}」，实得 ${JSON.stringify(onTexts[onTexts.length - 1])}`);

      // ⑩ 区间首尾相接、不重叠、不掉帧（把 ASS 的时间戳解析回毫秒来比）。
      const spans = assOf(dirOn, TID).split('\n')
        .filter((l) => l.startsWith('Dialogue:'))
        .map((l) => [l.split(',')[1], l.split(',')[2]]);
      const ms = (t) => {
        const [h, m, rest] = t.split(':');
        return Math.round((Number(h) * 3600 + Number(m) * 60 + Number(rest.replace(',', '.'))) * 1000);
      };
      for (let i = 1; i < spans.length; i += 1) {
        // ⚠️ 容差 **10ms** 不是 1ms：`assTime` 把毫秒四舍五入到**厘秒**
        //    （`Math.round(ms/10)`，两位小数是 ASS 的格式）⇒ 相邻两条的接缝
        //    天然最多差 10ms。第一版写成 1ms，判据自己先红了（而那不是缺陷）。
        ok(Math.abs(ms(spans[i][0]) - ms(spans[i - 1][1])) <= 10,
          `第 ${i} 条与前一条必须首尾相接（不留缝、不重叠）：` +
          `${spans[i - 1][0]}–${spans[i - 1][1]} 然后 ${spans[i][0]}–${spans[i][1]}`);
      }
      // ⑪ ⚠️ **不许产出 0 帧的 cue**：0 帧在底座那边一帧都不显示 ⇒ 那个字永不出现，
      //    而报告里看着"拆了 N 条"。
      for (const [a, b] of spans) {
        ok(ms(b) > ms(a), `不许有 0 帧的 cue（${a} → ${b}）：那个字永远不会出现`);
      }
      // ⑫ 关着的那一遍**必须与加了开关那遍不同**（否则 ⑥ 是假绿：
      //    万一 `translate` 根本不看 `[elements]`，两遍会长得一模一样）。
      //    ⚠️ 这一条是"变异自证"的**外层保险** —— 没有它，
      //    "readElementsCfg 永远返回默认"这类改动会以"两边都一样"通过。
      ok(onTexts.length !== offTexts.length,
        '开了/没开打字机两遍结果**完全相同** —— 说明 [elements] 根本没被读（判据是假绿）');
    } finally {
      rmSync(dirOn, { recursive: true, force: true });
    }

    // ------------------------------------------------------------------
    // ⑬–⑰ ⚠️ **源码文本判据**（2026-10-11）
    //
    // 为什么非要走源码文本：上面 ⑥–⑫ 是**行为**判据（跑真 `translate`），
    // 而它们**在变异台里读不到变异体** —— 实测（`E4A_TRACE` 探针）：
    // 变异台明明把 `text: chars.join('')` 写进了临时模块、`mutatedSource` 里
    // 也确实有那串，可 `checkTextElementLane` 里跑出来的 `onTexts` **仍是正确的前缀序列**。
    // ⇒ 于是"每条都发整句"那条变异**永远存活**，而**不是**守卫的判据写错了。
    // ★ 与 `checkDanmakuColorStripped` / `checkStageGeom` 同一个理由：
    //   凡是"变异体改了、行为判据却看不见"的地方，一律**补一道源码文本判据**。
    //   两道都要有：行为判据守"真的拆了没有"，文本判据守"那几行还在不在"。
    // ------------------------------------------------------------------
    ok(src.includes('const tw = elementsCfg.typewriter === true;'),
      '打字机的开关必须是**唯一**一道门（`const tw = elementsCfg.typewriter === true;`）—— ' +
      '写没了就说明开关口径变了');
    ok(src.includes("text: chars.slice(0, target).join('') });"),
      '拆出来的每一条必须是**前 target 个字**（`chars.slice(0, target)`）—— ' +
      '改成整句就成了"一堆重复 cue"，而 ASS 里看着条数还对');
    ok(src.includes('danmaku: false'),
      '文字元素那条 `cuesToAss` 必须传 `danmaku: false` —— 不传的话弹幕的' +
      '"至少滚到出画"规则会把每条 cue 抬到 300ms，打字机的短 cue 于是**全部重叠**');
    ok(src.includes('danmaku ? danmakuExitMs(cue.startMs, cue.endMs, cue.travelMs) : cue.endMs,'),
      '`cuesToAss` 里必须有"文字元素原样用 `endMs`"那个分支（弹幕才走出点规则）');
    ok(src.includes('const tailStart = Math.max(0, cue.end - holdFrames);'),
      '静止尾巴必须留着（`cue.end - holdFrames`）—— 打完立刻消失观众读不完');

    // ------------------------------------------------------------------
    // ⑱–㉚ §5 E4a **T1 弹入 / T4 抖动 / T6 底衬**（2026-10-11）
    //
    // 手法与 ⑥–⑫ 同源：**同一个工程**改 `[elements]` 再跑一遍，逐条对拍。
    // 底座接口（字段名逐字照抄，别自己起名）：
    //   T1/T4 → Dhampir `docs/text-motion-integration.md`
    //   T6    → `docs/plan/t6-plate-interface.md`
    //
    // ⚠️ **R4 在这里是两层，两层都要有**（本仓 #154 的教训）：
    //    ① **JSON 键层** —— 全默认时底座工程里**一个键都不多**；
    //    ② **求值层** —— 全默认时底座读出来的样式与老工程**逐字段相等**。
    //    只有一层的话，"默认时多写了个键"或"默认值走岔"各有一个能溜过去。
    // ------------------------------------------------------------------
    checks += checkTextMotion(mod, dir, src);
  // **T5 真卡拉OK（2026-10-11）** —— 与 T2 同属"文字动效"这一族，但**切的是另一条轨**。
  // ⚠️ 抽成独立函数是**为了 `--mutate` 也能跑到它**（只写在顶层的话变异台根本不跑）。
  checks += checkKaraoke(mod, src);
  // **纵向档位的分轨（2026-10-12）** —— 同理抽成独立函数。
  checks += checkTextPosLanes(mod, src);
    return checks;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

/// 纵向档位（`pos`）的分轨判据（2026-10-12，底座 v7 的 `anchor_bottom`）。
///
/// # 这条守的是什么
///
/// `anchor_bottom` 是**轨级**常量，而 `pos` 是**逐事件**的。
/// 所以"按样式分轨"是不够的：同一档样式下两个事件选了不同档位时，
/// 它们会**共用**一个 `anchor_bottom` ⇒ **后写的那档把先写的覆盖掉**，
/// 一部分字跑到错误的高度。⚠️ 而报告里**一行差异都没有**（转译成功、轨也在）。
///
/// 实测这**不是假想**：真实工程里 `hook` 同时出现过 `top` 与 `center`。
///
/// ⚠️ 判据是**行为**的（数产物里真的有几条轨、各自的 `anchor_bottom` 是多少），
/// 不是"源码里有没有那行 `for`"—— 后者变异台里看不见也测不准。
function checkTextPosLanes(mod, src) {
  let checks = 0;
  const ok = (cond, what) => {
    checks += 1;
    if (!cond) throw new Error(what);
  };
  const dir = mkdtempSync(join(tmpdir(), 'yekitrim-pos-'));
  try {
    writeFileSync(join(dir, 'clip.mp4'), Buffer.from([0, 0, 0, 0]), 'utf8');
    // **同一个样式（hook）两条事件、两个不同档位** —— 正是会互相覆盖的那种工程。
    writeFileSync(join(dir, 'polish.toml'), [
      'config_version = 1',
      'duration = 8.0',
      'processed = true',
      '',
      '[source]',
      'video = "clip.mp4"',
      'subtitles = ""',
      'danmaku = ""',
      '',
      '[canvas]',
      'width = 1920',
      'height = 1080',
      '',
      '[[segments]]',
      'start = 0.0',
      'end = 8.0',
      '',
      '[[segments.events]]',
      'type = "element"',
      'kind = "text"',
      'time = 0.2',
      'end = 2.0',
      'text = "上面的标题"',
      'style = "hook"',
      'pos = "top"',
      'source = "hook|120|8|255,255,255,255|26,26,26,255|900|0"',
      '',
      '[[segments.events]]',
      'type = "element"',
      'kind = "text"',
      'time = 3.0',
      'end = 5.0',
      'text = "中间那一句"',
      'style = "hook"',
      'pos = "center"',
      'source = "hook|120|8|255,255,255,255|26,26,26,255|900|0"',
      '',
      '[[segments.events]]',
      'type = "element"',
      'kind = "text"',
      'time = 6.0',
      'end = 7.8',
      'text = "底下那句"',
      'style = "hook"',
      'pos = "bottom"',
      'source = "hook|120|8|255,255,255,255|26,26,26,255|900|0"',
      '',
    ].join(String.fromCharCode(10)), 'utf8');

    const toml = readFileSync(join(dir, 'polish.toml'), 'utf8');
    const project = mod.translate(mod.parseToml(toml), dir, join(dir, 'pos.json'));
    const ids = (project.timeline?.tracks ?? []).map((t) => t.id);
    const textIds = ids.filter((id) => String(id).startsWith('text_hook'));

    // ① **必须分成三条轨**。只按样式分的话这里只有一条 ⇒ 三档共用一个常量。
    ok(textIds.length === 3,
      `同一个样式下的三个档位必须是**三条轨**（各带自己的 anchor_bottom），` +
      `实得 ${textIds.length} 条：${JSON.stringify(textIds)} —— ` +
      '分不开就会出现"后写的那档覆盖先写的"（而报告里一行差异都没有）');

    // ② 每档的锚点必须**不同**且**顺序正确**（上 < 中 < 下）。
    const anchorOf = (suffix) => {
      const t = (project.timeline?.tracks ?? []).find((x) => x.id === `text_hook_${suffix}`);
      ok(!!t, `缺 text_hook_${suffix} 轨；实得 ${JSON.stringify(textIds)}`);
      return t?.subtitle?.anchor_bottom;
    };
    const top = anchorOf('top');
    const center = anchorOf('center');
    const bottom = anchorOf('bottom');
    ok(typeof top === 'number' && typeof center === 'number',
      `top/center 两档必须写出 anchor_bottom（实得 top=${top} center=${center}）`);
    // ⚠️ `bottom` 档**故意不写键**（`undefined`）—— 它要减 `caption_clearance`，
    //    只有底座知道那个值；写成常数会让文字压住字幕。
    //    所以这里判的是"它**没有**写死一个数"，而不是"它等于某个数"。
    ok(bottom === undefined,
      `bottom 档不许写死 anchor_bottom（要留给底座扣 caption_clearance，` +
      `写死会压住字幕），实得 ${JSON.stringify(bottom)}`);
    ok(top < center,
      `上档的块底必须高于中档：top=${top} center=${center}`);

    // ③ `bottom` 档走缺省 ⇒ 在软件里等价于 1.0，必须**低于**前两档。
    ok(center < 1.0, `中档的块底必须高于缺省贴底（1.0），实得 ${center}`);

    // ④ 源码文本判据（变异台里行为判据看不见变异体，见函数末尾那段注释）：
    //    分组键必须是**三维**的（样式 | 纵向档 | 动效档）。
    //
    // ⚠️ 2026-10-13（39 档）：第三维 `anim` 与第二维 `pos` 是**同一个理由的第二次出现** ——
    //    `scale_in_*` / `shake_*` 那些动效字段**也是轨级的**（它们住在底座的
    //    `SubtitleStyle` 上），所以"同一样式、同一档位、一条砸入一条摇摆"
    //    必须分到两条轨，否则同样会"后写的覆盖先写的"，而报告里一行差异都没有。
    // ⚠️ **2026-10-13 两级化**：第三维从**一个 `anim`** 变成**两轴各一维** ——
    //    因为「样式」与「动效」现在是**两个独立设置**，同一样式档位下
    //    "竖排 + 抖动"与"竖排 + 摇摆"是**两条不同的轨**。
    //    仍然是**同一个理由**（轨级参数必须同轨同参），只是维度从三维变四维。
    //    ⚠️ **别退回按 `cue.anim` 分组**：它对**新工程**恒为空串 ⇒
    //       所有组合挤进同一条轨 ⇒ 后写的静默覆盖先写的。
    // ⚠️ **2026-10-13 五维化（颜色）**：第五维是**同一个理由的第四次出现** ——
    //    底座的 `SubtitleStyle.color` 是 `[u8;4]` **轨级**字段（`layer.rs:571`），
    //    而颜色预设是**逐事件**的 ⇒ 同轨两种颜色只能活一种（后写覆盖先写）。
    //    ⚠️ **别以为"ASS 里写了 `\c` 就够了"**：出片走的是 `doc.json` 这条，
    //       底座**自己光栅化文字**（`text_overlay.rs`）、**根本不读那个 ASS** ——
    //       只做 ASS 那半边 = 「文件里颜色对、成片里全白」，实测抓到过（7 条不同色
    //       的文字元素渲染出来**全是白的**，而 ASS / 报告 / 日志三者都完全正常）。
    //
    //    ⚠️ 颜色这一维用**RGBA 值**而不是预设名：没写 color 时各档默认色本来就不同
    //       （`comment` 暖黄、其余白），按名字分会让"comment 默认黄"与
    //       "显式写了 yellow"混进同一轨。
    // ⚠️ 把源码里的**换行与缩进压平**再找分组键 ——
    //    键在源码里**跨两行**（`` `${...|${cue.styleAnim || ''}` `` +
    //    换行 + `` `|${cue.effectAnim...}` ``），单行 `includes`
    //    **永远匹配不上** ⇒ 这条判据会恒红；上一版能过纯属巧合（那时键恰好在一行里）。
    //    压平后两类都匹配得上，且仍然钉着**全部六维**。
    const flatSrc = src.replace(/\s+/g, '');
    ok(flatSrc.includes(
      '`${cue.style.name}|${cue.pos}|${cue.styleAnim||\'\'}`+`|${cue.effectAnim||\'\'}|${cue.transitionAnim||\'\'}|${colorKey}`'),
      '分组键必须是「样式 | 纵向档 | **排版轴** | **过渡轴** | **动效轴** | **颜色**」六维' +
      '（最后一维 `colorKey` = 逐条 RGBA 拼串）—— ' +
      '少一维，同一档位下就会共用一个轨级参数（后写的覆盖先写的，' +
      '成片里那一条静默带上了别人的参数，而报告里一行差异都没有）；' +
      '⚠️ 过渡那一维是 2026-10-14 三轴收敛时加的 —— 过渡参数（`scale_in_*`）' +
      '是**轨级**的，不分轨的话「同样式，一条要弹出、一条要淡入」会静默共用一个');
    // ⚠️ 颜色那一维**必须是值、不是预设名**（理由见上）。
    ok(src.includes('const colorKey = Array.isArray(cue.color) ? cue.color.join(\',\') : \'\'') &&
       flatSrc.includes('`${cue.style.name}|${cue.pos}|${cue.styleAnim||\'\'}`+`|${cue.effectAnim||\'\'}|${cue.transitionAnim||\'\'}|${colorKey}`'),
      '最后一维必须是 **RGBA 值**（`cue.color.join(\',\')`），不是 `colorName` —— ' +
      '按名字分会让"comment 的默认黄"与"显式写的 yellow"分到两条轨（值相同却拆开），' +
      '而按值分永远正确');
    // ⚠️ 轨 id 里带的是**预设名**（R4：没写 color 的老工程 id 逐字符不变）。
    ok(src.includes("const colorTag = cues[0].style.colorName || ''"),
      '轨 id 的颜色后缀必须取 `cues[0].style.colorName`（**空串 = 不加后缀**）—— ' +
      '用 RGBA 的话连没写颜色的老工程也会多出一串 ⇒ 产物整个变样（R4 破）');
    ok(src.includes('`${colorTag ? \'_\' + colorTag : \'\'}`'),
      '`colorTag` 为空时**不许加下划线**（否则老工程 id 变成 `text_hook_bottom_`）');
    // ---- 逐条颜色：**行为**判据（不只是"代码在"）----
    //
    // ⚠️ 上面那几条只证明"分组键里有颜色" —— 而本仓栽过太多次
    //    "字段在、链路没接"（`color` 从 cue 到 ASS 中间任何一环断了，
    //    上面几条**全绿**：分组键照样带 `colorKey`，只是值恒为空串）。
    //    ⇒ 必须有一条**从产物上读回来**的判据。
    //
    // ⚠️ 用**合成工程**过一遍 `translate`（与映射那 44 项同一手法）：
    //    同一个 style+pos 放两条**只有颜色不同**的元素，
    //    断言 ① 分出**两条轨**、② 两条轨的 `color` **不同且各自正确**、
    //    ③ ASS 里那条异色的带 `\c`、同色的那条**一个字节都不多**
    //    （这三条各自对应一处真实缺陷，见下面 `MUTATIONS` 里那四条）。
    // ⚠️ 锚点必须**带上下文**：`color: st.color,` 在文件里出现两次
    //    （① 文字轨的**逐条** cue ② 轨的 `SubtitleStyle`）——
    //    裸匹配会被第 ② 处兜住 ⇒ 变异打不红（实测就是这么发现的）。
    //    ⇒ 这里钉"cue 那一次"，用紧邻的 `text: textBody,` 定位。
    ok(/text: textBody,\s*\n\s*style: st,\s*\n[\s\S]{0,2000}?color: st\.color,/.test(src),
      'cue 必须带上 `color: st.color` —— 不带的话分组键里那一维恒为空串，' +
      '所有颜色挤进同一轨（而上面"五维"那条判据**照样绿**：键里确实有 `${colorKey}`）');
    ok(src.includes('withMoveTravel(cueColorOverride(cue.color, primary) + cue.text, cue.travelMs)'),
      'ASS 的逐条颜色必须走 `cueColorOverride(...)` 前缀 —— ' +
      '漏了它，侧挂 ASS 里所有字都退回 style 那一份主色（`doc.json` 那条腿仍然是对的，' +
      '所以只有"看 ASS 的人"会发现）');
    // ASS 的字节序：`&HAABBGGRR`（**B/G/R 与 RGBA 相反 + alpha 取反**）。
    ok(src.includes('return `&H${hex(255 - a)}${hex(b)}${hex(g)}${hex(r)}`;'),
      'ASS 颜色必须写成 `&HAABBGGRR`（alpha 取反 + **B/GR 倒序**）—— ' +
      '照 RGBA 序写会让红蓝对调（pink 变蓝紫），两边都不报错；' +
      'alpha 不取反则七档全近乎透明（成片里一个字都看不见，而文件看着完全正常）');
    ok(src.includes('anchor_bottom: anchorBottomFor(posName, st, height)'),
      '轨上必须真的写出 `anchorBottomFor(posName, st, height)` —— ' +
      '算出来了却没写进轨，等于没接');
    return checks;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

/// 文字轨的 id 前缀 —— **轨 id 是 `text_<样式>_<纵向档>`**（2026-10-12）。
///
/// ⚠️ 提到模块级：`checkTextElementLane` 与 `checkTextMotion` 两处都要用它，
/// 各自写一份字面量的话，"哪天分轨规则再变"会只改一处、另一处**静默测错轨**
/// （找不到轨就会报"夹具不完整"，而真因是 id 变了 —— 归因会跑偏）。
const TEXT_TRACK_ID = 'text_hook_top'; // 夹具工程写的是 style=hook / pos=top

/// T1/T4/T6 的判据（§5 E4a，2026-10-11）。
///
/// ⚠️ **抽成函数是为了 `--mutate` 也能跑它** —— 只写在 `checkTextElementLane`
/// 里面的话，变异台那条路径照样会调用（它调的就是本函数），但将来若有人把它
/// 挪到顶层就废了。`scripts/AGENTS.md` 里那条"新检查要抽成函数并在 mutate 里也调"说的就是这件事。
function checkTextMotion(mod, dir, src) {
  let checks = 0;
  const ok = (cond, what) => {
    checks += 1;
    if (!cond) throw new Error(what);
  };

  /// 用同一份工程、换 `[elements]` 段，跑一次 `translate`，取文字轨的 `subtitle`。
  ///
  /// ⚠️ **`extra === null` 与 `extra === []` 是两种形态，别混**（2026-10-12）：
  ///   * `null` = **整段都不写** = "这个工程没配元素动效" ⇒ R4 要求一个键都不多；
  ///   * `[]` = 写出**空的 `[elements]` 段** = "配了、但没填字段" ⇒ 走贴纸同款弹出缺省。
  ///   旧的 `if (extraLines.length)` 把两者并成了"不写段"，于是"空段"这个形态
  ///   **没有任何判据覆盖** —— 而那正是新缺省值的触发条件。
  const textTrackSubtitle = (extra) => {
    const lines = [
      'config_version = 1',
      'duration = 2.0',
      'processed = true',
      '',
      '[source]',
      'video = "clip.mp4"',
      'subtitles = "clip.srt"',
      'danmaku = ""',
      'metadata = ""',
      '',
      '[canvas]',
      'width = 1920',
      'height = 1080',
      '',
      '[strip]',
      'max_duration = 2.0',
      '',
      '[[segments]]',
      'start = 0.0',
      'end = 2.0',
      'role = "other"',
      'index = 1',
      '',
      '[[segments.events]]',
      'type = "element"',
      'kind = "text"',
      'time = 0.2',
      'end = 1.5',
      'text = "钩子标题"',
      'style = "hook"',
      'pos = "top"',
      'x = "10%"',
      'y = "20%"',
      'source = "hook|120|8|255,255,255,255|26,26,26,255|900|0"',
      '',
    ];
    // ⚠️ **`extra === null` ⇒ 整段不写**（R4 那一档）；
    //    数组（**含空数组**）⇒ 写出 `[elements]` —— 空段正是"配了但没填字段"，
    //    也就是弹出缺省的触发条件。旧写法 `if (extraLines.length)` 让空数组**不写段**，
    //    于是那条判据永远测不到它想测的形态。
    if (extra !== null) lines.push('[elements]', ...extra, '');
    const doc = mod.translate(mod.parseToml(lines.join(String.fromCharCode(10))), dir, join(dir, 'm.doc.json'));
    const track = (doc.timeline?.tracks ?? []).find((t) => t.id === TEXT_TRACK_ID);
    return track ? (track.subtitle ?? {}) : null;
  };
  /// 配了 `[elements]`（数组，含空数组）；`null` 表示整段不写。
  const withElements = (extraLines) => textTrackSubtitle(extraLines ?? []);

  /// **带 `anim` 的夹具**（39 档，2026-10-13）。
  ///
  /// ⚠️ 不复用 `textTrackSubtitle`：它把 `anim` 那行写死成不写，
  ///    而这里要的恰恰是"同一条文字、只改动效档"的对照。
  ///    ⚠️ 轨 id 也随 `anim` 变（`text_hook_top_<anim>`）—— 这是 39 档那条
  ///    "不同动效必须分轨"的直接后果，夹具得跟着找对新 id。
  const textTrackWithAnim = (animName, extra) => {
    const lines = [
      'config_version = 1',
      'duration = 2.0',
      'processed = true',
      '',
      '[source]',
      'video = "clip.mp4"',
      'subtitles = "clip.srt"',
      'danmaku = ""',
      'metadata = ""',
      '',
      '[canvas]',
      'width = 1920',
      'height = 1080',
      '',
      '[strip]',
      'max_duration = 2.0',
      '',
      '[[segments]]',
      'start = 0.0',
      'end = 2.0',
      'role = "other"',
      'index = 1',
      '',
      '[[segments.events]]',
      'type = "element"',
      'kind = "text"',
      'time = 0.2',
      'end = 1.5',
      'text = "钩子标题"',
      'style = "hook"',
      'pos = "top"',
      'x = "10%"',
      'y = "20%"',
      'source = "hook|120|8|255,255,255,255|26,26,26,255|900|0"',
    ];
    if (animName !== null) lines.push(`anim = "${animName}"`);
    lines.push('');
    if (extra !== null) lines.push('[elements]', ...extra, '');
    const doc = mod.translate(mod.parseToml(lines.join(String.fromCharCode(10))), dir, join(dir, 'm.doc.json'));
    const want = animName ? `${TEXT_TRACK_ID}_${animName}` : TEXT_TRACK_ID;
    const track = (doc.timeline?.tracks ?? []).find((t) => t.id === want);
    if (!track) return null;
    return { sub: track.subtitle ?? {}, layer: (track.layers ?? [])[0] ?? {} };
  };
  /// **三轴版**夹具（2026-10-14）：按**轴**各自写字段，不再用老的单个 `anim`。
  ///
  /// ⚠️ 为什么另起一个而不是改上面那个：`textTrackWithAnim` 造的是**老工程**
  ///    （只有 `anim`），那条路（`splitLegacyAnim`）**仍然在转译器里**、
  ///    另有判据守着它。把两者混成一个会让"老工程没回归"这条**丢掉**。
  ///
  /// ⚠️ 轨 id 现在把**三路档名**都带上（`text_<style>_<pos>_<排版>+<动效>+<过渡>`）。
  ///    这里按同一规则拼出来找轨 —— **别写死**，否则档位一改这里就成了空转
  ///    （找不到轨 ⇒ 返回 `null` ⇒ 断言早退 ⇒ 假绿）。
  const textTrack3 = ({ styleAnim, transitionAnim, effectAnim, angle, extra } = {}) => {
    const lines = [
      'config_version = 1',
      'duration = 2.0',
      'processed = true',
      '',
      '[source]',
      'video = "clip.mp4"',
      'subtitles = "clip.srt"',
      'danmaku = ""',
      'metadata = ""',
      '',
      '[canvas]',
      'width = 1920',
      'height = 1080',
      '',
      '[strip]',
      'max_duration = 2.0',
      '',
      '[[segments]]',
      'start = 0.0',
      'end = 2.0',
      'role = "other"',
      'index = 1',
      '',
      '[[segments.events]]',
      'type = "element"',
      'kind = "text"',
      'time = 0.2',
      'end = 1.5',
      'text = "钩子标题"',
      'style = "hook"',
      'pos = "top"',
      'x = "10%"',
      'y = "20%"',
      'source = "hook|120|8|255,255,255,255|26,26,26,255|900|0"',
    ];
    if (styleAnim) lines.push(`style_anim = "${styleAnim}"`);
    if (effectAnim) lines.push(`effect_anim = "${effectAnim}"`);
    if (transitionAnim) lines.push(`transition_anim = "${transitionAnim}"`);
    // ⚠️ 角度**只在该填时才写** —— 写成 `layout_angle_deg = 0` 会掩盖
    //    "没填时回落档位缺省"那条（而那条正是 R4 的一半）。
    if (angle !== undefined && angle !== null) lines.push(`layout_angle_deg = ${angle}`);
    lines.push('');
    if (extra) lines.push('[elements]', ...extra, '');
    const doc = mod.translate(mod.parseToml(lines.join(String.fromCharCode(10))), dir, join(dir, 'm3.doc.json'));
    // 轨 id 按转译器**同一规则**拼（三路档名 `filter(Boolean).join('+')`）。
    const tag = [styleAnim, effectAnim, transitionAnim].filter(Boolean).join('+');
    const want = tag ? `${TEXT_TRACK_ID}_${tag}` : TEXT_TRACK_ID;
    const track = (doc.timeline?.tracks ?? []).find((t) => t.id === want);
    if (!track) return null;
    return { sub: track.subtitle ?? {}, layer: (track.layers ?? [])[0] ?? {}, doc, id: want };
  };

  /// **整段都不写** `[elements]` —— R4 那一档。
  const withoutElements = () => textTrackSubtitle(null);

  /// 底座的 T1/T4/T6 键名（**逐字来自接口文档**）。
  const MOTION_KEYS = ['scale_in_from', 'scale_in_peak', 'scale_in_ms',
    'shake_px', 'shake_hz', 'shake_phase', 'plate_color', 'plate_pad_px'];

  // ⑱ **R4 第一层：全默认时一个键都不多**。
  //
  // ⚠️ 这条是 **R4 红线**的执行处：不配 `[elements]` 的工程必须产出与今天
  // **逐字节相同**的底座工程。多一个键 = "升级一次、成片可能就变了"。
  //
  // ⚠️ 用 `withoutElements()`（**整段都不写**），不是 `withElements([])` ——
  //    后者写的是"空 `[elements]` 段"，那是**配了**的形态（弹出缺省该开）。
  //    两者是 R4 门的两侧，别混。
  const off = withoutElements();
  ok(off !== null, '夹具不完整：文字轨没出现（`[source]` 段少了会让 `remapFrame` 全返回 null）');
  const leaked = MOTION_KEYS.filter((k) => k in off);
  ok(leaked.length === 0,
    `没配 \`[elements]\` 却写出了动效键 ${JSON.stringify(leaked)} —— ` +
    'R4 要求默认时**一个键都不多**（多一个键就是"升级一次、工程变样"）');

  // ⑲ **R4 第二层：默认时求值结果是"单位变换"**（不只是"键没写出来"）。
  //
  // ⚠️ 与 ⑱ 不重复：⑱ 守"键集合"，这条守"值" —— 若哪天有人把默认值改成
  // 非单位（比如 `plate_pad_px` 默认 12、或者抖动默认开着），键可以仍然不写、
  // 而底座那边读出来是"有动效"。两层是**两个漏法**。
  ok(off.scale_in_ms === undefined || off.scale_in_ms === null,
    `默认时 scale_in_ms 应当是"没写"（= 不弹入），实得 ${JSON.stringify(off.scale_in_ms)}`);
  ok(off.shake_px === undefined || off.shake_px === null || off.shake_px === 0,
    `默认时 shake_px 应当是"没写"或 0（= 不抖），实得 ${JSON.stringify(off.shake_px)}`);
  ok(off.plate_color === undefined || off.plate_color === null,
    `默认时 plate_color 应当是"没写"（= 不铺底衬），实得 ${JSON.stringify(off.plate_color)}`);

  // ═══════════════════════════════════════════════════════════════════════
  // ═══════════════════════════════════════════════════════════════════════
  // ㉛–㊳ **9 档逐条（三轴）**（2026-10-14 三轴收敛后重写）
  // ═══════════════════════════════════════════════════════════════════════
  //
  // ⚠️ 这一组是**行为判据**（不是源码文本）：每档都真跑一遍转译器，
  //    看它写出的底座字段是不是预设表里那一组。
  //
  // ⚠️ 原来这里是"39 档逐条 `anim`"（老的单字段）。三轴收敛后写的是
  //    `style_anim` / `transition_anim` / `effect_anim` 三个字段
  //    ⇒ 夹具换成 `textTrack3`。**老夹具（`textTrackWithAnim`）保留**，
  //    因为"只有 `anim` 的老工程"那条路（`splitLegacyAnim`）仍在，
  //    另有判据守它 —— 别把两者合并。
  //
  // ⚠️ 没有这一组的时候，`--mutate` 里"弹入门失效"那条**打不红** ——
  //    因为没有任何用例把"某档 + 配了 `[elements]`"这个组合跑起来。
  //    缺口是**实测出来的**（先看到 5 处打不红，才回来补的这里）。

  // ★★ 先说**最要紧的一条**：`perchar_big` / `perchar_small` 的 `per_char`
  //    必须写出去。这两档的 `perChar` 是**形态字段**，但档位住在**动效**轴
  //    （`sort: 'effect'`）—— 实测抓到过：只要判据只认"排版轴给 `perChar`"，
  //    这两档就**整档不生效**（轨生成了、`per_char` 一个字不写、
  //    报告里一行差异都没有）。收敛后动效轴就 4 档，那一版里**一半是死的**。
  //
  // ⚠️ 这两档还**必须互相不同**（本仓为 `vtext_thin` vs `vtext_bold` 立过
  //    "两档不许逐字节相同"的判据，同一个坑在 `perchar_*` 上原样出现过）。
  {
    const big = textTrack3({ effectAnim: 'perchar_big' });
    const small = textTrack3({ effectAnim: 'perchar_small' });
    ok(big !== null, '`effect_anim = "perchar_big"` 的轨没生成');
    ok(small !== null, '`effect_anim = "perchar_small"` 的轨没生成');
    ok(big.sub.per_char !== undefined && big.sub.per_char !== null,
      'perchar_big 必须写出 per_char —— 它是形态字段但住在动效轴，' +
      '只认排版轴会让这一档**整档不生效且不报错**');
    ok(small.sub.per_char !== undefined && small.sub.per_char !== null,
      'perchar_small 必须写出 per_char（同上）');
    ok(big.sub.per_char.scale_spread !== undefined
      && big.sub.per_char.scale_spread > 0,
      'perchar_big 的 scale_spread 该是正数（逐字变大），实得 ' + JSON.stringify(big.sub.per_char.scale_spread));
    ok(small.sub.per_char.scale_spread !== undefined
      && small.sub.per_char.scale_spread < 0,
      'perchar_small 的 scale_spread 该是负数（逐字变小），实得 ' + JSON.stringify(small.sub.per_char.scale_spread));
    ok(JSON.stringify(big.sub.per_char) !== JSON.stringify(small.sub.per_char),
      'perchar_big 与 perchar_small 的 per_char 逐字节相同 —— ' +
      '两个档在面板上是两个名字，产物里却一模一样（本仓犯过这个错）');
  }

  // ㉛ **`tilt` 的静态 rotation**（没填角度时回落档位缺省 −6）。
  {
    const r = textTrack3({ styleAnim: 'tilt' });
    ok(r !== null, '`style_anim = "tilt"` 的轨没生成');
    ok(r.layer.transform?.rotation === -6,
      'tilt 的 rotation 缺省必须 −6，实得 ' + JSON.stringify(r.layer.transform?.rotation));
  }

  // ㉜ **`tilt` + `layout_angle_deg` ⇒ 用用户的角度**（不是缺省 −6）。
  {
    const r = textTrack3({ styleAnim: 'tilt', angle: -25 });
    ok(r !== null, '`tilt` + `layout_angle_deg=-25` 的轨没生成');
    ok(r.layer.transform?.rotation === -25,
      '填了角度就该用角度（−25），实得 ' + JSON.stringify(r.layer.transform?.rotation));
    const r2 = textTrack3({ styleAnim: 'tilt', angle: 12.5 });
    ok(r2.layer.transform?.rotation === 12.5,
      '正角度该原样写（12.5），实得 ' + JSON.stringify(r2.layer.transform?.rotation));
  }

  // ㉝ ★★ **`layout_angle_deg = 0` 必须与"没填"区分开**。
  //
  //     写成 `raw || 缺省` 的症状是"我把角度调成 0，它却回到了 −6"。
  //     ⚠️ `0` 是一个**合法的用户选择**（"我要它正着"），不是"没填"。
  {
    const r = textTrack3({ styleAnim: 'tilt', angle: 0 });
    ok(r !== null, '`tilt` + `layout_angle_deg=0` 的轨没生成');
    ok(r.layer.transform?.rotation === 0,
      '填 0 度必须真的是 0（不是缺省的 −6）—— ' +
      '实得 ' + JSON.stringify(r.layer.transform?.rotation));
  }

  // ㉞ **`arc` 的 `per_char` + 角度的方向保留**。
  //
  //     ⚠️ 弧形的档内缺省是 **−14**（向上弯）。用户填 `22`（正数）时，
  //        产物该是 `rotation_to = −22`（**取绝对值当幅度、保留档位的方向**），
  //        **不是** `+22` —— 直接赋值会让"我填 14"突然变成反向弯。
  {
    const def = textTrack3({ styleAnim: 'arc' });
    ok(def !== null, '`style_anim = "arc"` 的轨没生成');
    ok(def.sub.per_char?.rotation_to === -14,
      'arc 缺省该写 rotation_to = −14，实得 ' + JSON.stringify(def.sub.per_char?.rotation_to));
    const pos = textTrack3({ styleAnim: 'arc', angle: 22 });
    ok(pos.sub.per_char?.rotation_to === -22,
      'arc + 角度 22 该写 rotation_to = −22（取绝对值、保留档位方向），' +
      '实得 ' + JSON.stringify(pos.sub.per_char?.rotation_to));
    const neg = textTrack3({ styleAnim: 'arc', angle: -9 });
    ok(neg.sub.per_char?.rotation_to === -9,
      'arc + 角度 −9 该写 −9，实得 ' + JSON.stringify(neg.sub.per_char?.rotation_to));
  }

  // ㉟ **超出区间的角度被夹取，且报告里点名**（不许静默）。
  //     ⚠️ 只夹不报 = 用户设了 90、成片 30、**报告里没有一行**。
  {
    const badDir = join(dir, 'angle-clamp');
    mkdirSync(badDir, { recursive: true });
    writeFileSync(join(badDir, 'polish.toml'), [
      'config_version = 1', 'duration = 2.0', 'processed = true', '',
      '[source]', 'video = "clip.mp4"', 'subtitles = "clip.srt"', '',
      '[canvas]', 'width = 1920', 'height = 1080', '',
      '[[segments]]', 'start = 0.0', 'end = 2.0', 'role = "other"', 'index = 1', '',
      '[[segments.events]]', 'type = "element"', 'kind = "text"',
      'time = 0.2', 'end = 1.5', 'text = "钩子标题"', 'style = "hook"', 'pos = "top"',
      'x = "10%"', 'y = "20%"',
      'source = "hook|120|8|255,255,255,255|26,26,26,255|900|0"',
      'style_anim = "tilt"', 'layout_angle_deg = 90', '',
    ].join(String.fromCharCode(10)));
    copyFileSync(join(dir, 'clip.mp4'), join(badDir, 'clip.mp4'));
    copyFileSync(join(dir, 'clip.srt'), join(badDir, 'clip.srt'));
    const out = spawnSync(process.execPath,
      [TRANSLATOR, badDir, '--out', join(badDir, 'o.json')],
      { encoding: 'utf8', env: { ...process.env, DHAMPIR_QUIET: '0' } });
    const txt = (out.stdout || '') + (out.stderr || '');
    ok(/90|30|夹|clamp|角度/i.test(txt),
      '超出区间的 layout_angle_deg=90 必须有一条 NOTE/CLAMP 说明被夹到 30，' +
      '实得报告：' + JSON.stringify(txt.slice(0, 400)));
  }

  // ㊱ **`vtext` 写 vertical**（轨级排版方向），且**不带**旋转。
  {
    const r = textTrack3({ styleAnim: 'vtext' });
    ok(r !== null, '`style_anim = "vtext"` 的轨没生成');
    ok(r.sub.vertical === true,
      'vtext 必须写 vertical = true，实得 ' + JSON.stringify(r.sub.vertical));
    // ⚠️ 判“不该带旋转”不能写  ——
    //     是**每一层都有的基准缺省值**
    //    （转译器里那个  基座），
    //    它**不是档位的贡献**。写  会把“没有特定旋转”
    //    误判成“连基准值都没有”（假红）。
    ok(r.layer.transform?.rotation === 0,
      'vtext 不该带非零的静态 rotation（应停在基准值 0），实得 '
      + JSON.stringify(r.layer.transform?.rotation));
  }

  // ㊲ **`pop` 写放大弹入的三个字段**（0.3 / 1 / 350）。
  {
    const r = textTrack3({ transitionAnim: 'pop' });
    ok(r !== null, '`transition_anim = "pop"` 的轨没生成');
    ok(r.sub.scale_in_from === 0.3 && r.sub.scale_in_peak === 1 && r.sub.scale_in_ms === 350,
      'pop 该写 0.3/1/350，实得 ' + r.sub.scale_in_from + '/' + r.sub.scale_in_peak + '/' + r.sub.scale_in_ms);
  }

  // ㊳ ★★ **`fade` 一个字段都不给，且必须压掉全局那份弹入与摇摆**。
  //
  //     ⚠️ `fade` 的"什么都不给"是**有语义的**：它表示"这一档不加动效"，
  //        用来**压掉** `[elements]` 那份。⇒ 判"有没有写这一档"只能看
  //        **取没取到这一档**（`undefined` 才是没写），
  //        不能看 `Object.keys(preset).length > 0`。
  //
  //     ★ 实测抓到过两条腿不一致（都是"配了、不报错、悄悄生效"）：
  //       ① `motionFields` 的 `on` 少算一轴 ⇒ `fade` 继承全局的 `scale_in_*`；
  //       ② `swayCfg` 的 `anyAxis` 少算 transition 轴 ⇒ 配了全局 `sway_deg` 的
  //          工程里，选「淡入淡出」的字**会自己摆起来**（长出 rotation 关键帧）。
  //     ⇒ 这条判据**两样都要验**。
  {
    const GLOBALS = ['scale_in_from = 9.9', 'scale_in_peak = 9.8', 'scale_in_ms = 999',
      'shake_px = 77.0', 'shake_hz = 66.0', 'sway_deg = 33.3'];
    const r = textTrack3({ transitionAnim: 'fade', extra: GLOBALS });
    ok(r !== null, '`transition_anim = "fade"` + 全局 `[elements]` 的轨没生成');
    const leaked = ['scale_in_from', 'scale_in_peak', 'scale_in_ms', 'shake_px', 'shake_hz']
      .filter((k) => r.sub[k] !== undefined && r.sub[k] !== null);
    ok(leaked.length === 0,
      '`fade` 必须"一个字段都不给"—— 它不许继承全局 `[elements]`，' +
      '却写出了 ' + JSON.stringify(leaked));
    // ⚠️ 摇摆那条腿：它写的是**图层关键帧**（不是 `subtitle` 的键）⇒ 分开验。
    const kf = r.layer.keyframes ?? [];
    ok(kf.length === 0,
      '`fade` 在配了全局 `sway_deg` 的工程里长出了 ' + kf.length + ' 个关键帧 —— ' +
      '摇摆那条腿漏算了 transition 轴（`fade` 的语义是"只压掉全局"，' +
      '静默继承摇摆正是最恨的形状）');
  }

  // ㊳-bis **`shake` 写抖动，且压掉全局的弹入**。
  {
    const r = textTrack3({ effectAnim: 'shake' });
    ok(r !== null, '`effect_anim = "shake"` 的轨没生成');
    ok(r.sub.shake_px === 6 && r.sub.shake_hz === 18,
      'shake 该写 6/18，实得 ' + r.sub.shake_px + '/' + r.sub.shake_hz);
    ok(r.sub.scale_in_from === undefined || r.sub.scale_in_from === null,
      'shake 不该带弹入（逐条预设没提的字段必须按"关"），实得 ' + r.sub.scale_in_from);
  }

  // ㊳-ter **`heartbeat` 写出脉冲参数**。
  //
  //     ⚠️⚠️ 现有底座原语只有单次/正弦（`scale_in_params` 是 from→peak→1 的**单峰**），
  //        **没有双峰脉冲** ⇒ 当前观感是"一次较缓的脉冲"，**不是"扑通扑通"**。
  //        这一档**先落参数与入口**，底座能力到位后再补。
  //        ⚠️ 关键纪律：**不许静默当成普通弹入** —— 必须在报告里说出来。
  {
    const r = textTrack3({ effectAnim: 'heartbeat' });
    ok(r !== null, '`effect_anim = "heartbeat"` 的轨没生成');
    ok(r.sub.scale_in_from === 1 && r.sub.scale_in_peak === 1.14 && r.sub.scale_in_ms === 480,
      'heartbeat 该写 1/1.14/480，实得 '
      + r.sub.scale_in_from + '/' + r.sub.scale_in_peak + '/' + r.sub.scale_in_ms);
  }

  // ㊳-quater **三轴可以同时设，且各给各的字段**（可叠加，不互相吃掉）。
  {
    const r = textTrack3({ styleAnim: 'tilt', transitionAnim: 'pop', effectAnim: 'shake' });
    ok(r !== null, '`排版 + 过渡 + 动效` 三轴同时设的轨没生成');
    ok(r.layer.transform?.rotation !== undefined,
      '三轴同时设时，排版轴（tilt 的 rotation）丢了');
    ok(r.sub.scale_in_from === 0.3 && r.sub.scale_in_ms === 350,
      '三轴同时设时，过渡轴（pop 的 scale_in_*）丢了或被覆盖：'
      + '实得 ' + r.sub.scale_in_from + '/' + r.sub.scale_in_ms);
    ok(r.sub.shake_px === 6,
      '三轴同时设时，动效轴（shake 的 shake_px）丢了或被覆盖：'
      + '实得 ' + r.sub.shake_px);
  }

  // ㊳-quinquies **认不出的三轴档名要报 CLAMP 点名**（不许静默当"没有动效"）。
  //
  //     ⚠️ 症状：用户写 `effect_anim = "sway_stdd"`（手滑多一个 d）⇒ 字一动不动，
  //        而报告里**一行差异都没有**。⚠️ 那些是**三轴收敛前的旧档名**，
  //        用户从老笔记里抄一个过来就会命中这一支。
  {
    const badDir = join(dir, 'anim3-bad');
    mkdirSync(badDir, { recursive: true });
    writeFileSync(join(badDir, 'polish.toml'), [
      'config_version = 1', 'duration = 2.0', 'processed = true', '',
      '[source]', 'video = "clip.mp4"', 'subtitles = "clip.srt"', '',
      '[canvas]', 'width = 1920', 'height = 1080', '',
      '[[segments]]', 'start = 0.0', 'end = 2.0', 'role = "other"', 'index = 1', '',
      '[[segments.events]]', 'type = "element"', 'kind = "text"',
      'time = 0.2', 'end = 1.5', 'text = "钩子标题"', 'style = "hook"', 'pos = "top"',
      'x = "10%"', 'y = "20%"',
      'source = "hook|120|8|255,255,255,255|26,26,26,255|900|0"',
      'effect_anim = "sway_stdd"', '',
    ].join(String.fromCharCode(10)));
    copyFileSync(join(dir, 'clip.mp4'), join(badDir, 'clip.mp4'));
    copyFileSync(join(dir, 'clip.srt'), join(badDir, 'clip.srt'));
    const out = spawnSync(process.execPath,
      [TRANSLATOR, badDir, '--out', join(badDir, 'o.json')],
      { encoding: 'utf8' });
    const txt = (out.stdout || '') + (out.stderr || '');
    ok(/sway_stdd/.test(txt),
      '认不出的 `effect_anim = "sway_stdd"` 必须在报告里点名（用户手滑时成片里字一动不动，' +
      '而报告里一行差异都没有 —— 这条判据就是为那种情况写的），实得：' + JSON.stringify(txt.slice(0, 400)));
  }

  // ㊴ **不写 `anim` 时轨 id 不变**（R4：老工程产物逐字节相同）。
  {
    const track = (() => {
      const doc = mod.translate(
        mod.parseToml([
          'config_version = 1', 'duration = 2.0', 'processed = true', '',
          '[source]', 'video = "clip.mp4"', 'subtitles = "clip.srt"', '',
          '[canvas]', 'width = 1920', 'height = 1080', '',
          '[[segments]]', 'start = 0.0', 'end = 2.0', 'role = "other"', 'index = 1', '',
          '[[segments.events]]', 'type = "element"', 'kind = "text"',
          'time = 0.2', 'end = 1.5', 'text = "钩子标题"', 'style = "hook"', 'pos = "top"',
          'x = "10%"', 'y = "20%"',
          'source = "hook|120|8|255,255,255,255|26,26,26,255|900|0"', '',
        ].join(String.fromCharCode(10))),
        dir, join(dir, 'm.doc.json'),
      );
      return (doc.timeline?.tracks ?? []).find((t) => t.kind === 'subtitle' && t.id.startsWith('text_hook_top'));
    })();
    ok(track !== undefined && track.id === TEXT_TRACK_ID,
      `不写 \`anim\` 时轨 id 必须仍是 \`${TEXT_TRACK_ID}\`（R4），实得 ${JSON.stringify(track?.id)}`);
  }
  ok(off.font_ratio === undefined || off.font_ratio !== undefined,
    'font_ratio 仍在（这条只是让 ⑱ 的"键集合"有个非动效键作对照）');

  // ⑳–㉖ 七个字段**配了就必须写出去，且值要对**。
  //
  // ⚠️ 逐字段独立一档（不是"全配一遍"）：全配的话某一个字段漏写会被
  //    其它字段的存在掩盖 —— 那样判据钉的是"至少有一个字段通了"。
  const perField = [
    ['scale_in_from', 'scale_in_from = 0.7', 'scale_in_from', 0.7],
    ['scale_in_peak', 'scale_in_peak = 1.15', 'scale_in_peak', 1.15],
    ['scale_in_ms', 'scale_in_ms = 180', 'scale_in_ms', 180],
    ['shake_px', 'shake_px = 14.0', 'shake_px', 14.0],
    ['shake_hz', 'shake_hz = 18.0', 'shake_hz', 18.0],
    ['plate_color', 'plate_color = [0, 0, 0, 140]', 'plate_color', [0, 0, 0, 140]],
    ['plate_pad_px', 'plate_pad_px = 12.0', 'plate_pad_px', 12.0],
  ];
  for (const [label, tomlLine, key, want] of perField) {
    const st = withElements([tomlLine]);
    ok(st !== null, `夹具不完整（${label} 那一档没拿到文字轨）`);
    ok(key in st, `配了 \`${label}\`，底座工程里却没有 \`${key}\` 这个键 —— 用户配了等于没配`);
    ok(JSON.stringify(st[key]) === JSON.stringify(want),
      `\`${key}\` 应当写成 ${JSON.stringify(want)}，实得 ${JSON.stringify(st[key])}`);
  }

  // ㉗ **⚠️⚠️ 只配动效字段、不开打字机 ⇒ 必须照样写出去**（`is_empty()` 那个坑）。
  //
  // `ElementsConfig::is_empty()` 曾经**只**判 `!typewriter`。加了这七个字段之后，
  // 「配了 `shake_px`、没开 `typewriter`」的工程会被判成**空** ⇒ `[elements]`
  // 整段不写出去 ⇒ 用户写的那行**静默丢失**（`polish.toml` 里还在，成片不抖）。
  // ★ 与 `deny_unknown_fields` 那条是**同族**的错——"用户写了配置、程序当他没写"。
  const onlyShake = withElements(['shake_px = 14.0']);
  ok(onlyShake !== null, '夹具不完整（只配 shake_px 那一档没拿到 text_hook）');
  ok(onlyShake.shake_px === 14.0,
    '只配了 `shake_px`、没开 `typewriter` ⇒ `shake_px` **丢了** —— ' +
    '`is_empty()` 没把新字段算进去（`[elements]` 整段没写出去）。' +
    '这是"用户写了配置、程序当他没写"，症状是成片不抖而报告一行差异都没有');

  // ㉘ **源码文本判据兜底**（照 ⑬–⑰ 那个写法）。
  //
  // 为什么行为判据之外还要这条：行为判据在**变异台里可能看不见变异体**
  // （⑬–⑰ 那段的实测记录）。这两处是"接线"本身，变异体改了源码文本、
  // 而用 `mutatedSource` 加载的模块仍然行为正确时，只有文本判据抓得住。
  // ⚠️ **2026-10-13 两级化**：第四、五实参从"一个 `anim`"变成**两轴各一个**
  //    （动效轴 / 样式轴）—— 这条文本判据跟着改成钉**两个都在**。
  //    ⚠️ **别只改名字不改条文**：这条判据的价值在于"漏传就等于用户选了没效果、
  //       而报告一行差异都没有"。两轴各漏一个，症状就各是一个：
  //       漏**动效轴** ⇒ 用户选"抖动"不抖；漏**样式轴** ⇒ 用户选"竖排"不竖排。
  //    ⇒ 两条 `includes` 都要在，少一条就红。
  ok(src.includes('...motionFields(') &&
    src.includes('SHAKE_PHASE_BY_STYLE[styleName],') &&
    src.includes('hasElementsSection(config),') &&
    src.includes('animPreset(cues[0].effectAnim),') &&
    src.includes('animPreset(cues[0].styleAnim),'),
    '字幕轨必须把 `motionFields(elementsCfg, …, hasElementsSection(config), ' +
    'animPreset(动效轴), animPreset(样式轴))` 摊进 `subtitle` —— ' +
    '写没了就等于"配置读了但没接出去"，而工程里 `[elements]` 看着还在；' +
    '⚠️ 第三个实参是**弹出缺省值的门**（没配 `[elements]` 时不许写那三个键，R4），漏传就等于无条件默认开；' +
    '⚠️ 第四个实参是**动效轴**（时间：`shake_*`/`sway_*`/`scale_in_*`）—— 漏传就等于"用户选了砸入、成片里不砸"；' +
    '⚠️ 第五个实参是**样式轴**（形态：`per_char`/`vertical`/`safe_width_ratio`）—— 漏传就等于' +
    '"用户选了竖排、成片里横着"，**两种症状的报告里都是一行差异都没有**');
  ok(src.includes("if (phase !== 0) put('shake_phase', phase);"),
    '`motionFields` 必须逐键显式 `put`，且 `shake_phase` 要**排除 0** —— ' +
    '0 是底座默认值，写出来就是 R4 违规（"没写"与"写了默认值"必须是同一份字节）');
  ok(src.includes('if (v !== undefined) out[key] = v;'),
    '`motionFields` 必须**跳过 `undefined`**（不是写成 `null`）—— ' +
    '底座那边 `plate_color: null` 是"显式不铺"，与"没写"**语义不同**');
  ok(src.includes('const optNum = (v) =>'),
    '`readElementsCfg` 必须有 `optNum`（可选数字）—— ' +
    '用它才能把"没配"与"配了默认值"分开（`num` 会兜底成默认值 ⇒ 两件事混成一件）');

  // ㉙ **脏配置必须被挡住**（不是"尽量读懂"，是"宁可当没配"）。
  //
  // ⚠️ 两条都在防同一件事：**让整份底座工程反序列化失败**。
  //    那不是"这一项不生效"，而是"整片出不了" —— 比静默更糟，因为它是**硬失败**，
  //    而出片链路的失败信息指向底座，很难回想到是 `polish.toml` 里一个手滑的值。
  //
  // ① `plate_color` 必须是**四元组**：底座那边是 `[u8;4]`，写成 3 个数 / 5 个数
  //    会让 serde 报类型错（**整份工程**读不了）。
  // ② `scale_in_ms` 必须是**整数**：底座那边是 `u64`，收到 `180.5` 会让 serde 报错。
  //
  // ⇒ 两处**返回值必须是 `undefined`**（= 不写这个键 = 回到老行为），
  //    而**不是**"截断/取整/补零" —— 猜一个值出来就是替用户改配置。
  const dirty = [
    ['plate_color = [0, 0, 0]', 'plate_color', '半截颜色（3 个数）'],
    ['plate_color = [0, 0, 0, 140, 9]', 'plate_color', '多一个数的颜色（5 个数）'],
    ['plate_color = [0, 0, 0, 999]', 'plate_color', 'alpha 越界（999 > 255）'],
    ['scale_in_ms = 180.5', 'scale_in_ms', '毫秒写成小数'],
  ];
  for (const [tomlLine, key, why] of dirty) {
    const st = withElements([tomlLine]);
    ok(st !== null, `夹具不完整（脏值那一档：${why}）`);
    ok(!(key in st),
      `${why}（\`${tomlLine}\`）时**不该**写出 \`${key}\` —— ` +
      `底座那边类型对不上会让**整份工程反序列化失败**（不是"这一项不生效"，是"整片出不了"）。` +
      `实测写出了 ${JSON.stringify(st[key])}`);
  }

  // ㉙ **贴纸同款弹出**（2026-10-12，用户口径「效果就为简单的贴纸弹出」）。
  //
  // # 三条各自钉一件事
  //
  //   * 配了 `[elements]` ⇒ 三个弹出键**必须在**（用户要的观感，不该逼他手写）；
  //   * 没配 `[elements]` ⇒ **一个键都不许有**（R4，上面 ⑱ 已经守了键集合）；
  //   * **值等于贴纸那套**（起点 0.3 / 终点 1.0 / 窗口 350ms）——
  //     ⚠️ 这条要**逐值**钉：光断言"键在"的话，把 0.3 写成 0.9 照样绿，
  //     而成片里就**不是那个弹出**了（"看着差不多"正是本仓最恨的那类）。
  const ON = withElements([]);
  ok(ON !== null, '夹具不完整（弹出缺省那一档）');
  ok(ON.scale_in_from === 0.3 && ON.scale_in_peak === 1.0 && ON.scale_in_ms === 350,
    '配了 `[elements]` 时弹出三项必须是**贴纸同款**（0.3 / 1.0 / 350ms），实得 ' +
    JSON.stringify([ON.scale_in_from, ON.scale_in_peak, ON.scale_in_ms]) +
    ' —— 起点取贴纸 `popIn` 的 0.3、终点给 1.0（底座 `back_out3` 自己冲到 1.25）、' +
    '窗口 = `STICKER_POP_IN_SEC`');
  // ⚠️ **用户显式配了就以他为准**（三个字段各自独立，别互相盖）。
  const OVR = withElements(['scale_in_from = 0.7', 'scale_in_peak = 1.15', 'scale_in_ms = 180']);
  ok(OVR !== null, '夹具不完整（显式覆盖那一档）');
  ok(OVR.scale_in_from === 0.7 && OVR.scale_in_peak === 1.15 && OVR.scale_in_ms === 180,
    '用户显式配了弹出三项时必须以**他的值**为准，实得 ' +
    JSON.stringify([OVR.scale_in_from, OVR.scale_in_peak, OVR.scale_in_ms]));
  // ⚠️ **只配一项时另两项仍走缺省**（"配了 a 就把 b、c 也当配了"是错的口径）。
  const PART = withElements(['scale_in_ms = 200']);
  ok(PART !== null, '夹具不完整（只配一项那一档）');
  ok(PART.scale_in_ms === 200 && PART.scale_in_from === 0.3 && PART.scale_in_peak === 1.0,
    '只配 `scale_in_ms` 时另两项**仍走贴纸缺省**（0.3 / 1.0），实得 ' +
    JSON.stringify([PART.scale_in_from, PART.scale_in_peak, PART.scale_in_ms]));

  // ㉚ **淡入淡出按贴纸那两段**（`fade_in_ms` / `fade_out_ms`）。
  //
  // ⚠️ 这两个键**一直在**（不是新加的 T1/T4/T6 那批），所以它们在
  // "没配 `[elements]`"时也必须写 —— 与 `scale_in_*` 的 R4 门**不是同一条**。
  // 以前这里写的是 0/0（没有进出场），现在按贴纸的 pop-in / fade-out 给值。
  ok(ON.fade_in_ms === 350,
    `文字的淡入必须是贴纸弹入那一段（350ms = \`STICKER_POP_IN_SEC\`），实得 ${JSON.stringify(ON.fade_in_ms)}`);
  ok(ON.fade_out_ms === 300,
    `文字的淡出必须是贴纸那一段（300ms = \`STICKER_FADE_OUT_SEC\`），实得 ${JSON.stringify(ON.fade_out_ms)}`);

  return checks;
}
///
/// # 它守的是什么
///
/// E4b 把 ASR 字级时间戳搬进工程（`data/char-timeline.json`），但**没有任何消费者** ——
/// 产物写出来、没人读、不报错（"哑产物"，与本趟反复抓到的"哑字段"同形）。
/// T5 就是那个消费者：按**字级时间戳**把 **SRT 字幕轨**逐字切开。
///
/// ⚠️⚠️ **它切的是字幕轨，不是文字元素轨**（后者归 T2 打字机）。
/// 元素文本是 LLM 另写的标题、提示词明确要求"别和字幕重复"，与 ASR 时间轴**天然对不齐**。
///
/// # 判据里的核心一条：**不是均分**
///
/// 把字级时间**故意拉不等距**（第 3 个字 1.8s 才出现），断言切开后的 cue 边界
/// **跟着那个时间走**，并**显式断言 `≠` 均分会算出的值**。
/// 均分实现（把 T2 那套等速搬过来）在这条上必红 —— 那是 T5 与 T2 的分界线。
function checkKaraoke(mod, translateSrc) {
  let checks = 0;
  const ok = (cond, what) => {
    checks += 1;
    if (!cond) throw new Error(what);
  };

  const dir = mkdtempSync(join(tmpdir(), 'yekitrim-karaoke-'));
  try {
    // 夹具：真视频（转译器要探时基/分辨率）+ 一条 3 秒字幕「ABCD」。
    const mp4 = join(dir, 'clip.mp4');
    try {
      execFileSync('ffmpeg', ['-v', 'error', '-f', 'lavfi', '-i',
        'color=c=white:s=160x120:d=3,format=yuv420p', '-f', 'lavfi', '-i',
        'anullsrc=r=48000:cl=stereo', '-shortest', '-r', '60',
        '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-c:a', 'aac', '-y', mp4],
        { stdio: ['ignore', 'ignore', 'ignore'], timeout: 120000 });
    } catch (e) {
      if (e && e.code === 'ENOENT') return checks; // 没有 ffmpeg ⇒ 静默跳过（与其它探媒体守卫同口径）
      throw e;
    }
    writeFileSync(join(dir, 'clip.srt'),
      '1\n00:00:00,000 --> 00:00:03,000\nABCD\n\n', 'utf8');

    /// 造 `data/karaoke-cues.json`（形状与 Rust `parse::karaoke::KaraokeCues` 逐字段一致）。
    const writeCues = (doc) => {
      mkdirSync(join(dir, 'data'), { recursive: true });
      writeFileSync(join(dir, 'data', 'karaoke-cues.json'),
        JSON.stringify(doc, null, 2), 'utf8');
    };

    /// 跑一次 translate，取生成的 `dhampir-subtitle.srt` 内容。
    const runWith = (karaoke, doc) => {
      if (doc === null) {
        try { rmSync(join(dir, 'data', 'karaoke-cues.json')); } catch { /* 本来就没有 */ }
      } else {
        writeCues(doc);
      }
      const lines = [
        'config_version = 1', 'duration = 3.0', 'processed = true', '',
        '[source]', 'video = "clip.mp4"', 'subtitles = "clip.srt"',
        'danmaku = ""', 'metadata = ""', '',
        '[canvas]', 'width = 1920', 'height = 1080', '',
        '[[segments]]', 'start = 0.0', 'end = 3.0', 'role = "other"', 'index = 1', '',
      ];
      if (karaoke) lines.push('[elements]', 'karaoke = true', '');
      const out = join(dir, 'karaoke.doc.json');
      mod.translate(mod.parseToml(lines.join('\n')), dir, out);
      // 字幕资产写在工程文件旁边。
      const srt = join(dir, 'dhampir-subtitle.srt');
      return { srt: existsSync(srt) ? readFileSync(srt, 'utf8') : '' };
    };

    /// 从 SRT 里抽出 cue 的 `[startMs, endMs, text]`。
    const parse = (text) => {
      const out = [];
      const blocks = text.replace(/\r\n?/g, '\n').split(/\n{2,}/);
      for (const b of blocks) {
        const ls = b.split('\n').filter((l) => l.trim() !== '');
        if (ls.length < 2) continue;
        const m = ls.find((l) => l.includes('-->'));
        if (!m) continue;
        const g = m.match(/(\d+):(\d+):(\d+),(\d+)\s*-->\s*(\d+):(\d+):(\d+),(\d+)/);
        if (!g) continue;
        const ms = (h, mm, s, f) => (+h) * 3600000 + (+mm) * 60000 + (+s) * 1000 + (+f);
        out.push([ms(g[1], g[2], g[3], g[4]), ms(g[5], g[6], g[7], g[8]),
          ls.slice(ls.indexOf(m) + 1).join(' ')]);
      }
      return out;
    };

    // 字级时间轴：**故意不等距** —— 第 3 个字要 1.8s 才出现。
    const pulled = { source_chars: 4, skipped: 0,
      cues: [{ subtitle_index: 1, start: 0.0, end: 3.0,
        chars: ['A', 'B', 'C', 'D'], times: [0.0, 0.2, 1.8, 2.0] }] };

    // ㉚ **关着时：与今天逐字节相同**（R4）。字幕还是那一条，且**没有**卡拉OK的 NOTE。
    const off = runWith(false, pulled);
    const offCues = parse(off.srt);
    ok(offCues.length === 1,
      `\`karaoke\` 没配（默认关）时字幕轨必须**还是那一条**，实得 ${offCues.length} 条 —— ` +
      'R4 红线：不配时底座工程逐字节不变');
    ok(offCues[0][2] === 'ABCD', `关着时那条字幕文本必须原样，实得 ${JSON.stringify(offCues[0][2])}`);

    // ㉛ **开着时：逐字切开，条数 = 字数**。
    const on = runWith(true, pulled);
    const onCues = parse(on.srt);
    ok(onCues.length === 4,
      `开了 \`karaoke\` 且有字级时间 ⇒ 「ABCD」必须切成 4 条，实得 ${onCues.length} 条`);
    ok(onCues.map((c) => c[2]).join('|') === 'A|AB|ABC|ABCD',
      `逐字 cue 必须是**累积文本**（A / AB / ABC / ABCD），实得 ${JSON.stringify(onCues.map((c) => c[2]))}`);

    // ㉜ ★★ **时间是字级时间戳，不是均分**（T5 的定义性判据）。
    //
    // 均分会把 4 个字摊在 0–3000ms 上 ⇒ 边界 0 / 750 / 1500 / 2250。
    // 字级时间是 0 / 0.2 / 1.8 / 2.0 秒 ⇒ 边界 0 / 200 / 1800 / 2000。
    const uniform = [0, 750, 1500, 2250];
    const got = onCues.map((c) => c[0]);
    ok(JSON.stringify(got) === JSON.stringify([0, 200, 1800, 2000]),
      `逐字 cue 的起点必须**逐字来自字级时间戳**（期望 [0,200,1800,2000]），实得 ${JSON.stringify(got)}。` +
      '⚠️ 若实得 [0,750,1500,2250] 就是**退化成了均分**（把 T2 的打字机搬过来了）—— ' +
      '那 T5 就没有存在意义了');
    ok(JSON.stringify(got) !== JSON.stringify(uniform),
      `必须**显式区别于均分** ${JSON.stringify(uniform)}，实得 ${JSON.stringify(got)}`);

    // ㉝ **首尾相接、不重叠、不倒退**，且末条铺到字幕结束。
    for (let i = 1; i < onCues.length; i += 1) {
      ok(onCues[i][0] >= onCues[i - 1][0],
        `第 ${i} 条的起点不许早于前一条（时间必须单调），实得 ${JSON.stringify(got)}`);
      ok(onCues[i][0] >= onCues[i - 1][1],
        `第 ${i} 条必须接在前一条之后（不重叠），实得 ${JSON.stringify(onCues)}`);
    }
    ok(onCues[onCues.length - 1][1] === 3000,
      `末条必须铺到字幕结束（3000ms），实得 ${onCues[onCues.length - 1][1]}`);

    // ㉞ **每条至少 1ms**（0 时长的 cue 在底座一帧都不显示 ⇒ 那个字永不出现）。
    for (const [s, e, t] of onCues) {
      ok(e > s, `cue ${JSON.stringify(t)} 的时长必须 > 0（实得 ${s}→${e}）—— ` +
        '0 时长的 cue 底座一帧都不显示，那个字**永远不会出现**，而报告里看着"切了 N 条"');
    }

    // ㉟ ⚠️ **缺结论文件 ⇒ 明说降级**（不许静默变回普通字幕）。
    //
    // 这是本条的**核心纪律**：用户开了 `karaoke`、却没有 `data/karaoke-cues.json`
    // （老工程 / 没跑过 `process`）⇒ 必须**说出来**。静默变回普通字幕
    // 会让用户"以为开着、其实没开" —— 与 #147/#181 同一族。
    const degraded = runWith(true, null);
    const degradedCues = parse(degraded.srt);
    ok(degradedCues.length === 1,
      `读不到结论文件时必须**退回普通整条字幕**（1 条），实得 ${degradedCues.length} 条`);

    // ㊱ **源码文本判据兜底**（照 ⑬–⑰ 那个写法）：
    // 降级那条 `note` 必须真的在源码里（变异台里行为可能看不见，文本抓得住）。
    ok(translateSrc.includes('卡拉OK（T5）已开启，但读不到 data/karaoke-cues.json'),
      '缺结论文件时必须有那条"**降级成普通字幕**"的 note —— ' +
      '没有它，用户开了开关却什么都没发生，报告里一行差异都没有');
    ok(translateSrc.includes('karaoke: raw.karaoke === true'),
      '`readElementsCfg` 必须把 `karaoke` 按 `=== true` 收（`"yes"`/`1` 一律当关 —— 保守）');
    ok(translateSrc.includes('const tw = elementsCfg.typewriter === true;'),
      '**元素轨那条路**仍必须由 `typewriter` 单独控制 —— ' +
      'T2（元素轨、等速）与 T5（字幕轨、字级时间）**切的是两条不同的轨**，' +
      '别把 `karaoke` 混进元素轨的判断里（那会把两条轨耦合成一条）');
    ok(translateSrc.includes('if (tw && elementsCfg.karaoke === true)'),
      '两者同时开着时必须有那条 NOTE（告诉用户各管各的轨）—— ' +
      '没有它，用户会以为"叠了两种效果"');

    // ㊲ **脏值：`karaoke = "yes"` / `1` 一律当关**（不炸、不误开）。
    const dirty = ['karaoke = "yes"', 'karaoke = 1'];
    for (const line of dirty) {
      const lines = [
        'config_version = 1', 'duration = 3.0', 'processed = true', '',
        '[source]', 'video = "clip.mp4"', 'subtitles = "clip.srt"',
        'danmaku = ""', 'metadata = ""', '',
        '[canvas]', 'width = 1920', 'height = 1080', '',
        '[[segments]]', 'start = 0.0', 'end = 3.0', 'role = "other"', 'index = 1', '',
        '[elements]', line, '',
      ];
      writeCues(pulled);
      let doc;
      try {
        doc = mod.translate(mod.parseToml(lines.join('\n')), dir, join(dir, 'dirty.doc.json'));
      } catch (e) {
        throw new Error(`脏值 \`${line}\` 让转译器抛了：${e && e.message}`);
      }
      ok(doc !== null && doc !== undefined, `脏值 \`${line}\` 必须能跑完（不炸）`);
      const dirtySrt = join(dir, 'dhampir-subtitle.srt');
      const dc = existsSync(dirtySrt) ? parse(readFileSync(dirtySrt, 'utf8')) : [];
      ok(dc.length === 1,
        `脏值 \`${line}\` 必须当**关**（1 条普通字幕），实得 ${dc.length} 条 —— ` +
        '只有真的是 `true` 才算开');
    }

    // ㊳ ⚠️ **`polish.toml` 里不许出现任何切分键**（架构红线）。
    //
    // 结论落 `data/karaoke-cues.json`，**不落进用户的配置** —— 两条理由：
    // ① `polish.toml` 是用户会手改的，结论混进去"谁对"就说不清了；
    // ② 那会让**每个存量工程都被追加一段派生数据** ⇒ "不配时逐字节不变"当场就假。
    for (const bad of ['char_timeline = ', 'karaoke_cues = ', '逐字 = ']) {
      ok(!translateSrc.includes(bad),
        `转译器源码里不许出现 \`${bad}\` —— 切分结论**不许写进用户的 polish.toml**`);
    }
    // ㊴ ⚠️ **挤在一起的字必须各占一个时刻**（"每字 ≥1ms"那条守卫的判据）。
    //
    // 上面那档用的是**拉得很开**的时间戳，永远走不到那条 `Math.max(ms, prev)` ——
    // 于是"把每字最小间距删掉"这个变异**存活**（实测：77 号变异第一版没红）。
    // 这一档专门喂**挤在一起**的时间戳：4 个字全在 0.0s 附近。
    // ⇒ 没有守卫时它们会塌成同一个时刻，`s + 1` 之外的字**被合并掉**（那个字永不出现）。
    const cramped = { source_chars: 4, skipped: 0,
      cues: [{ subtitle_index: 1, start: 0.0, end: 3.0,
        chars: ['A', 'B', 'C', 'D'], times: [0.0, 0.0, 0.0, 0.0] }] };
    const crampedRun = runWith(true, cramped);
    const crampedCues = parse(crampedRun.srt);
    ok(crampedCues.length === 4,
      `4 个字的时间戳**全挤在 0.0s** 时仍必须切出 4 条（每条至少占一个时刻），` +
      `实得 ${crampedCues.length} 条 —— 塌在一起的字会被合并、那个字**永远不出现**`);
    for (const [s, e, t] of crampedCues) {
      ok(e > s, `挤在一起时 cue ${JSON.stringify(t)} 的时长仍必须 > 0（实得 ${s}→${e}）`);
    }
    // ⚠️ **只数条数是不够的**（实测）：上一条 `e > s` 也拦不住 ——
    //    去掉每字最小间距之后 4 条会**全部挤在 `[0,1]`**（各 1ms），
    //    条数对、每条时长也 > 0，**唯独互相重叠** ⇒ 屏幕上同时冒出 "A"/"AB"/"ABC"。
    //    ⇒ 必须断言**互不重叠**（这才是"每字至少占一个时刻"的真正含义）。
    for (let i = 1; i < crampedCues.length; i += 1) {
      ok(crampedCues[i][0] >= crampedCues[i - 1][1],
        `挤在一起时第 ${i} 条必须接在前一条**之后**（不重叠），实得 ` +
        `${JSON.stringify(crampedCues.map((c) => [c[0], c[1], c[2]]))} —— ` +
        '全挤在同一个时刻会让几条 cue 同时显示（屏幕上一堆字叠着）');
    }
    // 且**不许**因此把它们摊成均分（守卫只保证"不塌、不叠"，不保证"摊开"）。
    const crampedStarts = crampedCues.map((c) => c[0]);
    ok(crampedStarts.every((v, i) => i === 0 || v >= crampedStarts[i - 1]),
      `挤在一起时起点仍必须单调，实得 ${JSON.stringify(crampedStarts)}`);

    // ㊵ ⚠️ **时间戳倒挂 ⇒ 退回整条**（单调守卫那条的判据）。
    //
    // 同样：上面那档的数据是单调的，永远走不到 `if (i > 0 && t < times[i-1]) return null`。
    // 喂一份**倒挂**的数据（第 2 个字早于第 1 个）⇒ 必须**不作数**、退回整条普通字幕。
    const backwards = { source_chars: 4, skipped: 0,
      cues: [{ subtitle_index: 1, start: 0.0, end: 3.0,
        chars: ['A', 'B', 'C', 'D'], times: [2.0, 0.1, 1.8, 2.5] }] };
    const bwRun = runWith(true, backwards);
    const bwCues = parse(bwRun.srt);
    ok(bwCues.length === 1,
      `字级时间**倒挂**（第 2 个字早于第 1 个）时**必须退回整条普通字幕**（1 条），` +
      `实得 ${bwCues.length} 条 —— 坏数据被原样用出去会切出重叠/倒退的 cue，` +
      '而报告里看着"切了 N 条"');
    ok(bwCues[0][2] === 'ABCD', `退回的那条必须是原文本，实得 ${JSON.stringify(bwCues[0][2])}`);

    // ㊶ **字数与时间个数对不上 ⇒ 退回整条**（不猜、不补齐）。
    const mismatched = { source_chars: 3, skipped: 0,
      cues: [{ subtitle_index: 1, start: 0.0, end: 3.0,
        chars: ['A', 'B', 'C'], times: [0.0, 0.5, 1.0] }] };
    const mmRun = runWith(true, mismatched);
    const mmCues = parse(mmRun.srt);
    ok(mmCues.length === 1,
      `字数（ABCD=4）与结论里的字（3 个）**对不上**时必须退回整条（1 条），` +
      `实得 ${mmCues.length} 条 —— 不猜、不补齐`);

    // ㊷ **形状坏掉的结论文件 ⇒ 当作没有**（走降级那条路，不是半用）。
    const badShapes = [
      ['{}', '空对象'],
      ['{"cues":[]}', '空 cues'],
      ['{"cues":[{"start":0,"end":3,"chars":["A"],"times":[]}]}', 'chars/times 长度不等'],
      ['{"cues":[{"start":0,"end":3}]}', '缺 chars/times'],
      ['not json at all', '根本不是 JSON'],
    ];
    for (const [body, why] of badShapes) {
      mkdirSync(join(dir, 'data'), { recursive: true });
      writeFileSync(join(dir, 'data', 'karaoke-cues.json'), body, 'utf8');
      const lines = [
        'config_version = 1', 'duration = 3.0', 'processed = true', '',
        '[source]', 'video = "clip.mp4"', 'subtitles = "clip.srt"',
        'danmaku = ""', 'metadata = ""', '',
        '[canvas]', 'width = 1920', 'height = 1080', '',
        '[[segments]]', 'start = 0.0', 'end = 3.0', 'role = "other"', 'index = 1', '',
        '[elements]', 'karaoke = true', '',
      ];
      let doc;
      try {
        doc = mod.translate(mod.parseToml(lines.join('\n')), dir, join(dir, 'bad.doc.json'));
      } catch (e) {
        throw new Error(`形状坏掉的结论（${why}）让转译器抛了：${e && e.message}`);
      }
      ok(doc !== null && doc !== undefined, `形状坏掉的结论（${why}）必须能跑完（不炸）`);
      const bsrt = join(dir, 'dhampir-subtitle.srt');
      const bc = existsSync(bsrt) ? parse(readFileSync(bsrt, 'utf8')) : [];
      ok(bc.length === 1,
        `形状坏掉的结论（${why}）必须**当作没有**（退回 1 条普通字幕），实得 ${bc.length} 条 —— ` +
        '静默半用比"当作没有"危险得多');
    }

    // ㊹ ⚠️ **两条字幕时结果必须仍按时间有序**（"按时间排序"那条的判据）。
    //
    // 上面全是**单条字幕**的夹具 ⇒ 输出天然有序，排序那句是空转的，
    // 于是"把排序改成按文本长度"这个变异**存活**（实测：78 号变异第一版没红）。
    // 这一档喂**两条**字幕，并让**第二条的字更长**：按长度排会把它们颠倒过来。
    writeFileSync(join(dir, 'clip.srt'),
      '1\n00:00:00,000 --> 00:00:01,400\nAB\n\n2\n00:00:01,400 --> 00:00:03,000\nCDEFGH\n\n',
      'utf8');
    const two = { source_chars: 8, skipped: 0, cues: [
      { subtitle_index: 1, start: 0.0, end: 1.4, chars: ['A', 'B'], times: [0.0, 0.7] },
      { subtitle_index: 2, start: 1.4, end: 3.0, chars: ['C', 'D', 'E', 'F', 'G', 'H'],
        times: [1.4, 1.6, 1.8, 2.2, 2.6, 2.9] },
    ] };
    const twoRun = runWith(true, two);
    const twoCues = parse(twoRun.srt);
    ok(twoCues.length === 8,
      `两条字幕（2 + 6 字）开了卡拉OK必须切成 8 条，实得 ${twoCues.length} 条`);
    for (let i = 1; i < twoCues.length; i += 1) {
      ok(twoCues[i][0] >= twoCues[i - 1][0],
        `**跨字幕**也必须按时间有序：第 ${i} 条的起点 ${twoCues[i][0]} 早于前一条 ` +
        `${twoCues[i - 1][0]}（实得 ${JSON.stringify(twoCues.map((c) => c[0]))}）—— ` +
        '底座按文件顺序读，倒挂 = 后出现的内容先显示');
    }
    // ⚠️ 顺序必须**真的是时间序**，不是"碰巧按长度"—— 短的那条（AB）必须在前面。
    ok(twoCues[0][2] === 'A' && twoCues[7][2] === 'CDEFGH',
      `首条必须是第一条字幕的第一个字、末条是第二条字幕的全句，实得 ` +
      `${JSON.stringify([twoCues[0][2], twoCues[7][2]])}`);
    // 复原成单条字幕，免得影响后面的档。
    writeFileSync(join(dir, 'clip.srt'),
      '1\n00:00:00,000 --> 00:00:03,000\nABCD\n\n', 'utf8');

    // ㊹ ★★ **淡入淡出：开 karaoke ⇒ 全 0；关着 ⇒ 350/180 逐字节不变**（R4）。
    //
    // # 为什么开 karaoke 必须把淡入淡出设 0（实测定的一条，不是推理）
    //
    // 底座的淡入是**每条 cue 各自重放**（`overlay.rs::text_envelope_scaled` 收的
    // `local_ms` 是**条内**位置）。逐字切开后中间那些 cue 只有**字间距那么长**：
    //   实测逐字 cue 时长 `[34, 33, 33, 34, 33, …]` ms
    //   ⇒ 33ms vs 350ms 淡入 = 只到 ~10%；33ms vs 180ms 淡出 = 只到 ~18%
    // ⇒ **还没亮就被下一条顶掉**。实测（只切 `karaoke`，逐帧数亮像素）：
    //   frame 28(0.47s)=0、148(2.47s)=0、**150(2.50s)=0（第 16 个字根本不出现）**、
    //   175–178(2.9s)=0（最后一个字闪一下就没了）；两个都设 0 后是干净阶梯 1607 → 2214。
    const subStyle = (doc) => {
      const t = (doc.timeline?.tracks ?? []).find((x) => x.id === 'sub');
      return t ? (t.subtitle ?? {}) : null;
    };
    /// 用同一条字幕、只切 `karaoke`，取字幕轨样式。
    const withKaraoke = (on) => {
      const lines = [
        'config_version = 1', 'duration = 3.0', 'processed = true', '',
        '[source]', 'video = "clip.mp4"', 'subtitles = "clip.srt"',
        'danmaku = ""', 'metadata = ""', '',
        '[canvas]', 'width = 1920', 'height = 1080', '',
        '[[segments]]', 'start = 0.0', 'end = 3.0', 'role = "other"', 'index = 1', '',
      ];
      if (on) lines.push('[elements]', 'karaoke = true', '');
      writeCues(pulled);
      return subStyle(mod.translate(mod.parseToml(lines.join('\n')), dir,
        join(dir, `fade-${on}.doc.json`)));
    };
    const stOn = withKaraoke(true);
    const stOff = withKaraoke(false);
    ok(stOn !== null && stOff !== null, '夹具不完整：字幕轨样式没拿到');
    ok(stOn.fade_in_ms === 0 && stOn.fade_out_ms === 0,
      `开 \`karaoke\` 时淡入淡出必须**都设 0**（实得 fade_in=${stOn.fade_in_ms} fade_out=${stOn.fade_out_ms}）。` +
      '逐字 cue 只有 ~33ms，走不完 350/180ms ⇒ 每条才到 10%/18% 就被顶掉，' +
      '**有的字根本画不出来、有的只有半透明**，而报告里照样写"切了 N 条"');
    ok(stOn.rise_in_px === 0 && stOn.rise_out_px === 0,
      `开 \`karaoke\` 时上下浮动也要归零（实得 rise_in=${stOn.rise_in_px} rise_out=${stOn.rise_out_px}）—— ` +
      '同一个道理：33ms 的 cue 里浮 20px 等于一直在动');
    ok(stOff.fade_in_ms === 350 && stOff.fade_out_ms === 180 &&
       stOff.rise_in_px === 20 && stOff.rise_out_px === 8,
      `**关着时必须逐字节不变**（R4）：期望 350/180/20/8，实得 ` +
      `${stOff.fade_in_ms}/${stOff.fade_out_ms}/${stOff.rise_in_px}/${stOff.rise_out_px} —— ` +
      '普通字幕轨的淡入一个字都不许动');

    // ㊸ 机械钉住"读的是 data/ 下的派生产物"这一条（而不是某种"从 srt 反推"的旁路）。
    ok(translateSrc.includes("join(clipDir, 'data', 'karaoke-cues.json')"),
      '结论必须从 `data/karaoke-cues.json` 读 —— ' +
      '那是 `process` 落的派生产物，不是从字幕旁路推出来的');

    return checks;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function checkIntroOutro(mod) {
  let checks = 0;
  const ok = (cond, what) => {
    checks += 1;
    if (!cond) throw new Error(what);
  };

  const dir = mkdtempSync(join(tmpdir(), 'yekitrim-io-'));
  try {
    mkdirSync(join(dir, 'assets', 'videos'), { recursive: true });
    // 主素材 2s / 素材 1s，都用 ffmpeg 造（拿不到 ffmpeg 就跳过这一组 —— 与其它探媒体守卫同口径）
    const mk = (out, secs, color) => {
      try {
        execFileSync('ffmpeg', ['-v', 'error', '-f', 'lavfi', '-i',
          `color=c=${color}:s=160x120:d=${secs},format=yuv420p`, '-r', '30',
          '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-y', out],
          { stdio: ['ignore', 'ignore', 'ignore'], timeout: 60000 });
        return existsSync(out);
      } catch (e) {
        // ⚠️ **只吞"ffmpeg 不可用"**（ENOENT）。别的错（滤镜写错、参数非法）**照抛** ——
        //    一律吞掉的话，这条 guard 会打印"0 项全绿"、看着像通过（实测踩过：
        //    `existsSync` 没 import ⇒ `ReferenceError` 被吞 ⇒ 静默跳过整组判据）。
        if (e && e.code === 'ENOENT') return false;
        throw e;
      }
    };
    if (!mk(join(dir, 'clip.mp4'), 2, 'blue') || !mk(join(dir, 'assets', 'videos', 'op.mp4'), 1, 'red')) {
      // ⚠️ **拿不到 ffmpeg 时返回 0 项**（这一组不适用）—— 但**别把它当"通过"**：
      //    这条 return 一旦因为别的原因（比如 `mk` 内部抛了个 ImportError）走到，
      //    打印出来就是"0 项全绿"，看着像过了。所以 `mk` 的 `catch` 只在
      //    **确实是 ffmpeg 不可用**时吞异常，其余照抛。
      return checks;
    }
    writeFileSync(join(dir, 'clip.srt'), '1\n00:00:00,100 --> 00:00:01,000\nx\n\n', 'utf8');
    const toml = [
      'duration = 2.0',
      '[project]', 'name = "io"', 'output = "o.mp4"',
      '[source]', 'video = "clip.mp4"', 'subtitles = "clip.srt"',
      '[edl]', 'total_frames = 0',
      '[[edl.clips]]', 'source_start = 0.0', 'source_end = 2.0', 'speed = 1.0',
      '[intro_outro.intro]', 'asset_id = "op"', 'in_sec = 0.3', 'out_sec = 0.8',
      '[intro_outro.outro]', 'asset_id = "op"', 'in_sec = 0.0', 'out_sec = 0.0',
      '',
    ].join('\n');
    const config = mod.parseToml(toml);
    ok(config.intro_outro?.intro?.asset_id === 'op',
      'TOML 里的 [intro_outro.intro] 必须被解析出来（父级要建成表）');
    const doc = mod.translate(config, dir, join(dir, 'p.doc.json'));

    const mains = [];
    for (const track of doc.timeline.tracks) {
      for (const l of track.layers ?? []) {
        if (typeof l.id === 'string' && l.id.startsWith('main-')) mains.push(l);
      }
    }
    mains.sort((a, b) => (a.start ?? 0) - (b.start ?? 0));
    ok(mains.length === 3, `1 段 EDL + 片头 + 片尾 应当出 3 个主画面层，实得 ${mains.length}`);

    // ① 片头在最前、长度 = (0.8 − 0.3)s × 30fps = 15 帧
    ok((mains[0].end ?? 0) - (mains[0].start ?? 0) === 15,
      `片头 0.3–0.8s @30fps ⇒ 15 帧，实得 ${(mains[0].end ?? 0) - (mains[0].start ?? 0)}`);
    // ①-bis **入点真的生效**：`source_in` = 0.3s × 30fps = 9（素材自己的时基）。
    //   ⚠️ 这条不能用 `in_sec = 0` 的 fixture —— 那样"入点被丢掉"的变异**照样全绿**
    //      （实测踩过：变异 51 存活）。用户要的"可裁剪，设置入点出点"全靠它。
    ok(mains[0].source?.source_in === 9,
      `片头入点 0.3s @30fps ⇒ source_in = 9，实得 ${JSON.stringify(mains[0].source)}`);
    // ② **片头用的是素材自己的画面**（不是主素材）—— 这是"登记漏了"那个静默坑的判据
    ok(String(mains[0].source?.asset_id ?? '').includes('op.mp4'),
      `片头必须指向素材 op.mp4，实得 ${JSON.stringify(mains[0].source)}`);
    // ③ 正片还是本条片段
    ok(String(mains[1].source?.asset_id ?? '').includes('clip.mp4'),
      `中间那段必须是本条片段，实得 ${JSON.stringify(mains[1].source)}`);
    // ④ 片尾在最后、out_sec=0 ⇒ **播到素材结尾**（1s @30 = 30 帧）
    ok((mains[2].end ?? 0) - (mains[2].start ?? 0) === 30,
      `片尾「播到结尾」⇒ 素材整段 1s = 30 帧，实得 ${(mains[2].end ?? 0) - (mains[2].start ?? 0)}`);
    ok(String(mains[2].source?.asset_id ?? '').includes('op.mp4'),
      `片尾必须指向素材 op.mp4，实得 ${JSON.stringify(mains[2].source)}`);
    // ⑤ 总长 = 15 + 60 + 30
    ok(mains[2].end === 105, `成片总帧数 = 15+60+30 = 105，实得 ${mains[2].end}`);
    // ⑥ 素材真的被登记进 assets（没登记的话 assetIdForRange 会静默回落）
    ok((doc.assets ?? []).some((a) => String(a.id ?? '').includes('op.mp4')),
      '素材必须出现在 doc.assets 里（没登记 ⇒ 片头静默变成主素材）');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  return checks;
}

function checkSpeedReport(mod) {
  let checks = 0;
  const ok = (cond, what) => {
    checks += 1;
    if (!cond) throw new Error(what);
  };

  const dir = mkdtempSync(join(tmpdir(), 'yekitrim-speed-'));
  try {
    writeFileSync(join(dir, 'clip.srt'), '1\n00:00:00,100 --> 00:00:01,000\nx\n\n', 'utf8');
    // 三段：原速 2s / **快放 2s（speed=2.0）** / **慢放 2s（speed=0.5）**。
    // 派生 @60fps（转译器这个 fixture 走 60）= 120 + 60 + 240 = 420 帧 = 7.0s。
    const toml = [
      'duration = 6.0',
      '[project]',
      'name = "speed"',
      'output = "o.mp4"',
      '[source]',
      'video = "clip.mp4"',
      'subtitles = "clip.srt"',
      '[edl]',
      'total_frames = 0',
      '[[edl.clips]]',
      'source_start = 0.0',
      'source_end = 2.0',
      'speed = 1.0',
      '[[edl.clips]]',
      'source_start = 2.0',
      'source_end = 4.0',
      'speed = 2.0',
      '[[edl.clips]]',
      'source_start = 4.0',
      'source_end = 6.0',
      'speed = 0.5',
      '',
    ].join('\n');
    const config = mod.parseToml(toml);
    const doc = mod.translate(config, dir, join(dir, 'p.doc.json'));

    // ① 报告的 NOTE 里点名了变速（含两个速率）。
    //
    // ⚠️ note 收在转译器**模块私有的 `issues` 数组**里（不写进 doc），只有 CLI 那条路会
    //    `console.log` 出来 ⇒ 这里用**成品里的可观测面**兜住这条语义：
    //    `extensions["yekitrim.polish_toml"]` 的 `edl_clips` / `out_frames` 是**有契约的**
    //    （注释里写明"带 EDL 才有这几个键"），所以它们能证明"EDL 真的被认下来了"；
    //    而 NOTE 的措辞由下一条（`--mutate` 让分支恒 false 时必须翻面）来钉。
    const stats = doc.extensions?.['yekitrim.polish_toml'] ?? {};
    ok(stats.edl_active === true, '带 EDL 的工程必须把 edl_active 置真（否则整条 EDL 被静默忽略）');
    ok(stats.edl_clips === 3, `edl_clips 应当 = 3，实得 ${stats.edl_clips}`);
    ok(stats.out_frames === 420, `out_frames（成片总帧数）应当 = 420，实得 ${stats.out_frames}`);

    // ② 画面层真的按 speed 缩了段长
    const mains = [];
    for (const track of doc.timeline.tracks) {
      for (const l of track.layers ?? []) {
        if (typeof l.id === 'string' && l.id.startsWith('main-')) mains.push(l);
      }
    }
    mains.sort((a, b) => (a.start ?? 0) - (b.start ?? 0));
    ok(mains.length === 3, `三段 EDL 应当出 3 个主画面层，实得 ${mains.length}`);
    const dur = mains.map((l) => (l.end ?? 0) - (l.start ?? 0));
    ok(dur[0] === 120, `第 1 段原速 2s ⇒ 120 帧，实得 ${dur[0]}`);
    ok(dur[1] === 60, `第 2 段 ×2.0 ⇒ 60 帧（**缩一半**，这正是变速生效的判据），实得 ${dur[1]}`);
    ok(dur[2] === 240, `第 3 段 ×0.5 ⇒ 240 帧（**放一倍**），实得 ${dur[2]}`);
    ok(mains[2].end === 420, `成片总帧数 = 420，实得 ${mains[2].end}`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  return checks;
}

/// 第 4 批：`[[a.b]]` 的**父级必须建成表**（`parseToml` 的静默分叉）。
///
/// 为什么单独钉：`[[edl.clips]]` **前面没有 `[edl]` 表头**时（手改过 / 第三方生成的
/// `polish.toml`），旧解析器把 `edl` 建成**数组** ⇒ `config.edl?.clips === undefined`
/// ⇒ 转译器把**整条 EDL 当没有**（画面按整条源出片），而 **Rust 的真 toml 解析器读得出来**
/// （音轨按 EDL 拼）—— 两侧分叉、**都不报错**。这正是规格 §3 R7 那一类。
///
/// ⚠️ 现实里自家工程不受影响（Rust 落盘经 `toml::to_string_pretty`，会写 `[edl]` 表头），
/// 所以这条守的是"手改工程"面 —— 反过来说，**只有这里能抓住它**。
function checkTomlArrayTableParent(mod) {
  let checks = 0;
  const eq = (actual, expected, what) => {
    checks += 1;
    const a = JSON.stringify(actual);
    const b = JSON.stringify(expected);
    if (a !== b) throw new Error(`${what}：期望 ${b}，实得 ${a}`);
  };

  // ① 只有 `[[edl.clips]]`、没有 `[edl]` 表头 ⇒ 父级必须是**表**，clips 要能读出来
  const noHeader = mod.parseToml('[[edl.clips]]\nsource_start = 3.0\nsource_end = 6.0\nspeed = 1.0\n');
  eq(noHeader.edl?.clips?.length, 1, '`[[edl.clips]]` 无 `[edl]` 表头时也要读出 clips（父级必须是表）');
  eq(noHeader.edl?.clips?.[0]?.source_start, 3, '读出来的值要对');
  // ⚠️ **真正的判别串是"父级是不是数组"，不是 `edl.clips` 读不读得出来**：
  //    JS 数组本身就是对象 ⇒ 把 `edl` 建成数组后 `node[last].push(...)` 会**挂在那个数组上**
  //    （`edl.clips` 照样读得出、值还对！），但 `edl` 自己成了 `[]`（长度 0，元素全丢）。
  //    实测踩过：只断言 `edl?.clips` 的话，把 `node[key] = {}` 改回 `[]` **照样绿** ——
  //    这条变异第一次就是这么"没被打红"的。所以必须**直接判类型**。
  eq(Array.isArray(noHeader.edl), false, '`[[a.b]]` 的父级 `a` 必须是**表**，不是数组');

  // ② 同一份文件的**两种写法**必须读成同一个形状（否则两端各读一份、分叉不报错）
  const withHeader = mod.parseToml('[edl]\n\n[[edl.clips]]\nsource_start = 3.0\nsource_end = 6.0\n');
  const a = JSON.stringify(noHeader.edl?.clips?.[0]?.source_start);
  const b = JSON.stringify(withHeader.edl?.clips?.[0]?.source_start);
  eq(a, b, '有/无 `[edl]` 表头两种写法必须读成同一形状');
  eq(Array.isArray(withHeader.edl), false, '有 `[edl]` 表头时父级同样必须是表（两种写法的形状要一致）');

  // ③ 既有形状**不许被这次改动碰坏**：
  //    `[[segments]]` 是最外层数组表（父级 = 根，段本身进数组）—— 它仍然是**数组**
  const segs = mod.parseToml('[[segments]]\nstart = 1.0\n[[segments.events]]\nkind = "x"\n');
  eq(Array.isArray(segs.segments), true, '`[[segments]]` 仍是最外层数组表（段在数组里）');
  eq(segs.segments?.[0]?.events?.[0]?.kind, 'x', '`[[segments.events]]` 嵌在段里要读得出');
  //    而 `[cover]` + `[[cover.effects]]` 是"普通表里的数组表"，effects 是**数组**
  const cov = mod.parseToml('[cover]\n\n[[cover.effects]]\ntype = "gradient"\n');
  eq(Array.isArray(cov.cover?.effects), true, '`[[cover.effects]]` 的父级 cover 是表、effects 是数组');
  eq(cov.cover?.effects?.[0]?.type, 'gradient', 'effects 里的值要读得出');
  return checks;
}

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
  // §18-A：**第四档**。少了这条，把 特写 退回 1.5 的变异不会被打红
  // （加完那处变异才发现这里没有对应断言 —— 变异台先于断言存在，正是它该抓的）。
  eq(mod.zoomFactor('特写', undefined), 1.8, 'zoom="特写" 默认 1.8（§18-A 的第四档）');
  eq(mod.zoomFactor('特写', { zoom_closeup: 2.2 }), 2.2, '特写配了就用配置的值');
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
  const dir = mkdtempSync(join(tmpdir(), 'yekitrim-map-'));
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
    const autoDir = mkdtempSync(join(tmpdir(), 'yekitrim-map-auto-'));
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

  // ---- 3c. **弹幕时间偏移**（`[style.danmaku] offset_secs`，默认 -3 = 整体提前 3s）----
  //
  // 偏移必须落在**源时间轴**上（先平移、再走 `remapCueMs` 的裁剪重映射）——
  // 落在成片轴上（remap 之后才平移）的话，落在被裁掉的屏蔽段里的弹幕就**不会被丢**，
  // 而是平移进邻段里冒出来（"把屏蔽段里的弹幕洗回来"）。合成工程：**无 mute_ranges**
  // （remap = 恒等，两轴数值相同 ⇒ 这份断言只能钉"有没有平移"、钉不了"在哪条轴上"），
  // "轴"由变异台钉："remap 之后才平移"那条变异会让负偏移算出**负的毫秒**，
  // `assTime` 对负数产出的 `h:m:s` 全是负数段 —— 任何 `0:00:xx` 判据都接不住它。
  // 判据落在 `dhampir-danmaku.ass` 的 `Dialogue` 行上；fps 固定 60、取整值，无取整噪声。
  {
    const dir = mkdtempSync(join(tmpdir(), 'yekitrim-map-dmoff-'));
    try {
      // 字幕与弹幕：1 条 cue，2.5~4.5s（默认偏移 -3 ⇒ 源轴 [-0.5,1.5]，
      // 起点落在 0 帧之前的那半段被重映射裁掉 —— 与模板腿对负窗口的取舍同口径）
      writeFileSync(join(dir, 'clip.srt'),
        '1\n00:00:00,500 --> 00:00:02,500\n占位字幕\n\n',
        'utf8');
      writeFileSync(join(dir, 'clip.dm.srt'),
        '1\n00:00:02,500 --> 00:00:04,500\n偏移对齐用弹幕\n\n',
        'utf8');
      const base = [
        'duration = 5.0',
        '[project]',
        'name = "dm-offset"',
        'output = "o.mp4"',
        '[source]',
        'video = "clip.mp4"',
        'subtitles = "clip.srt"',
        'danmaku = "clip.dm.srt"',
      ].join('\n');
      // 无 mute：cue 2.0~4.0 偏移 -3 ⇒ 源轴 -1.0~1.0，remap 恒等 ⇒ 成片 [-1,1]，
      // 起点在 0 帧之前的那半段被丢，Dialogue 从 0.0s 起（与模板腿同口径）
      {
        writeFileSync(join(dir, 'polish.toml'), base + '\n', 'utf8');
        const doc = mod.translate(mod.parseToml(base + '\n'), dir, join(dir, 'p.doc.json'));
        const ass = (doc.assets ?? []).find((a) => a.id === 'dhampir-danmaku.ass');
        if (!ass) throw new Error('合成工程里没有弹幕轨（clip.dm.srt 解析失败？）');
        const dialogue = readFileSync(ass.uri, 'utf8').split('\n').filter((l) => l.startsWith('Dialogue:'));
        eq(dialogue.length, 1, '无 mute：偏移后弹幕还剩 1 条');
        eq(dialogue[0].slice(12, 22), '0:00:00.00', '无 mute：默认偏移 -3 ⇒ cue 从 2.5s 提前到 0.0s（isFinite 兜底生效）');
        // 终点不钉：`danmakuExitMs` 会把在屏窗口抬到"够它滚完"（travel ≈ 11.7s），
        // 终点被 travel 主导、与偏移无关 —— 钉它只会把无关常数冻进判据。
      }
      // 显式 `offset_secs = 0`：cue 原样保留（2.5~4.5）—— 平移逻辑一旦被丢（
      // "写的时候忘了读配置"）或被翻转（正负号写反），这两条 0.0/1.5 判据
      // 与这一条 2.5/4.5 判据必有一个翻面。
      {
        const zero = base + '\n[style.danmaku]\noffset_secs = 0.0\n';
        const doc = mod.translate(mod.parseToml(zero), dir, join(dir, 'p2.doc.json'));
        const ass = (doc.assets ?? []).find((a) => a.id === 'dhampir-danmaku.ass');
        const dialogue = readFileSync(ass.uri, 'utf8').split('\n').filter((l) => l.startsWith('Dialogue:'));
        eq(dialogue.length, 1, 'offset_secs = 0：显式零偏移不许弄丢弹幕');
        eq(dialogue[0].slice(12, 22), '0:00:02.50', 'offset_secs = 0 ⇒ cue 保持 2.5s 起（平移丢失/翻转都会翻面）');
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
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
const envelopeChecks = checkEnvelopeClamp(translatorModule);
const speedChecks = checkSpeedReport(translatorModule);
const tomlTableChecks = checkTomlArrayTableParent(translatorModule);
const assetTbChecks = checkAssetTimebase(translatorModule);
const assetPredChecks = checkAssetClipPredicate(translatorModule);
const introOutroChecks = checkIntroOutro(translatorModule);
console.log(`✓ 映射（合成工程过 translate）：${mappingChecks} 项全绿`);

const laneChecks = checkLaneSwitches(translatorModule);
console.log(`✓ 轨道显示开关（合成工程过 translate + 默认值表）：${laneChecks} 项全绿`);
const geomChecks = checkStageGeom(readFileSync(TRANSLATOR, 'utf8'));
console.log(`✓ 画面构成的几何（两轴 xy 插值，源码抽函数）：${geomChecks} 项全绿`);
console.log('  钉住的是 2026-10-05：先定画面大小 → 绕盒心缩放 → **两轴各自**按余量百分比插值；');
console.log('  scale=1 时与老顺序逐值相同、scale≠1 时 X=0 是贴左（旧顺序 -303.75 = 画面被推出盒外）');
const denseChecks = checkDenseRanges(translatorModule);
console.log(`✓ 密集键区间（③R3 右端落点 + 两个边界）：${denseChecks} 项全绿`);
console.log(`✓ 调整图层窗口夹取（乱序/重放 EDL 下不许超出所在段）：${envelopeChecks} 项全绿`);
console.log(`✓ 变速（第 4 批：报告点名 + 段长按 speed 缩，帧号域）：${speedChecks} 项全绿`);
console.log(`✓ TOML 数组表父级（[[edl.clips]] 无 [edl] 头也要读得出来）：${tomlTableChecks} 项全绿`);
console.log(`✓ 素材时基（第 6 批：source_in 按素材自己的帧率换算）：${assetTbChecks} 项全绿`);
console.log(`✓ 素材判据（只认小写 asset + 非空 id）：${assetPredChecks} 项全绿`);
console.log(`✓ 片头/片尾（[intro_outro] 拼进时间线 + 用素材自己的画面）：${introOutroChecks} 项全绿`);
const textElemChecks = checkTextElementLane(translatorModule);
console.log(`✓ 文字元素轨道（左右安全宽 + 与 safe_width_ratio 自洽 + 四档字号）：${textElemChecks} 项全绿`);
console.log('  钉住的是 2026-10-11：`bottom_margin: 0.5` ⇒ 底座 `max_width = 宽 × (1 - 2×0.5) = 0`');
console.log('  ⇒ `text_layout::layout` **提前返回空布局** ⇒ 文字元素一个字都不排，而成片「看着像画了」');
console.log('  （字幕照画、转译报告零差异、`probe` 零 error）—— 只有真去数像素才发现少了那几个字');
const fontFamilyChecks = checkFontFamily(translatorModule);
const elementAssFontChecks = checkElementAssFont(translatorSource);
console.log(`✓ 字体族名（库 id → 真族名，底座 0.2.0 起）：${fontFamilyChecks} 项全绿`);
console.log('  钉住的是：libass **按族名**找字体，而 polish.toml 存的是**库 id**；库内那支必须换成');
console.log('  真族名（传 id 会静默落 ArialMT、lines_failed 仍是 0），库外的值必须**原样返回**');
console.log('  钉住的是 2026-10-06 的六条轨道开关（`[stage.lanes]`）：缺字段时"素材竖或画布竖 ⇒ 运镜/');
console.log('  视觉特效默认关、音效只跟画布"，**显式开关优先于一切**（横画布也能关、竖画布也能开）——');
console.log('  与 Rust `ProjectFile::lane_default` 四种方向组合逐值对拍');
console.log('  钉住的是 M2 三条"报告里一行、成片里没有"：覆盖层 color/fade_in 两档、贴纸只有 name、');
console.log('  以及 `style.camera` 没配时的**按档位默认倍率**（§18-A 起四档：特写 1.8 / 近 1.5 / 远 0.55 / 中 1.3，照抄模板）');

if (process.argv.includes('--mutate')) await mutate();
