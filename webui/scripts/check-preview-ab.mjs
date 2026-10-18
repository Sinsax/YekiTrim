// 精修预览的**跳屏蔽区间（A/B 预滚）**契约守卫 —— 零依赖、不 import 应用模块。
//
// 守的是三件"错了也不报错、只是又变回黑/卡"的事（都是 2026-09-30 这一轮踩出来的）：
//   ① `videoRef` 必须是 **computed**（台上那块 = `activeStage` 选中的那块）。
//      几十处代码都在读 `videoRef.value`；一旦有人"顺手清理"成 `const videoRef = ref(null)`
//      甚至改回模板 `ref="videoRef"`，换台就只会换样式、读时钟的人还盯着旧元素 —— A/B 静默失效。
//   ② 换台的实现必须**先翻 `activeStage` 再暂停旧那块**（顺序反了，旧元素的 `pause` 事件会被
//      当成"用户按了暂停"，按钮/时间轴跟着抖一下）。
//   ③ Dhampir 那条腿必须在 `engine.seek` **之前**挡一道"源没就绪就别画"
//      （跟播时引擎不 await seek，源没像素时合出来就是整块黑，而那块 WebGPU 画布读不回来）。
//      并要有**备用那套源**（`dhampirTwins`）与换绑（`bindSource`）。
//
// 每条断言后面都跟一处**变异自证**：把源码改成"错的那一版"，检查必须翻面 ——
// 少了它，断言可能只是匹配了注释里的字样（这个坑本仓踩过）。
//
// 2026-10-19 补 ⑥⑦⑧⑨（用户报「播放头落在屏蔽段里开始播 = 持续卡顿」）：
//   ⑥ **根因**：`dhampirFrameAt` 的段起点比较要留半个素材帧容差。跳转目标是
//      `sourceIn / srcFps` 这个秒值，乘回去会得到 122.99999999999999 < 123（实测 30fps 下
//      0..200000 帧里 3270 个帧踩中）⇒ 判定永远说"这里没有帧" ⇒ tick 每帧重跳同一个位置。
//      判据分静态 + **行为级**（把四个函数从源码里抠出来真跑，喂一份 123 帧起点的段落表）。
//   ⑦ 跳转目标要经 `dhampirSnapForward` 前挪到"真的有帧"处，且 **tick / scheduleDhampir /
//      dhampirPrerollTarget / playableJumpTarget 全走同一个入口**（预滚与跳错开 = 白备一趟）。
//   ⑧ 从屏蔽段里按播放：`togglePlay` 必须**在 play() 之前**调 `playableJumpTarget()` 并落位，
//      不许上定格遮罩（那时旧画面还没上屏）。按**函数体里的调用顺序**判，不是 grep 名字。
//   ⑨ 同一跳转目标 400ms 内被第二次要求时不许再无脑 seek（防"未来回归变成无限重跳"）。
import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const SFC = join(HERE, '..', 'src', 'components', 'PolishPreview.vue');
const SRC = readFileSync(SFC, 'utf8');

let checks = 0;
/** 变异自证条数（只用于汇总行；不参与判据） */
let mutations = 0;
const ok = (cond, what, mutated = false) => {
  checks += 1;
  assert.ok(mutated ? !cond : cond, what);
};

/** 把源码里某段替换一次（变异自证用）。 */
function mutate(from, to) {
  const hits = SRC.split(from).length - 1;
  assert.equal(hits, 1, `变异台自己错了：「${from.slice(0, 40)}…」在 SFC 里命中 ${hits} 次（必须是 1）`);
  mutations += 1;
  return SRC.replace(from, to);
}

// ---- ① 台上那块必须是 computed，模板里不许再有 ref="videoRef" ----
const computedOk = (src) => /const videoRef = computed\(\(\) => \(activeStage\.value === 0 \? stage0Ref\.value : stage1Ref\.value\)\)/.test(src);
const templateRefOk = (src) => !/ref="videoRef"/.test(src);
const twoStagesOk = (src) => (src.match(/ref="stage[01]Ref"/g) || []).length === 2;
const standbyOk = (src) => /const standbyRef = computed\(\(\) => \(activeStage\.value === 0 \? stage1Ref\.value : stage0Ref\.value\)\)/.test(src);
ok(computedOk(SRC), '`videoRef` 不是 computed（台上那块靠 `activeStage` 选）—— A/B 换台会静默失效');
ok(templateRefOk(SRC), '模板里还有 `ref="videoRef"`：换台之后读时钟的人仍盯着旧元素');
ok(twoStagesOk(SRC), '台上/备用两块 `<video>` 没同时挂在 `.fit-box` 里（stage0Ref/stage1Ref）');
ok(standbyOk(SRC), '`standbyRef` 不是"另外那一块"—— 预滚会备到台上那块身上');
// 变异自证：把它改回模板 ref（A/B 就废了）必须翻面
{
  const bad = mutate('const videoRef = computed(() => (activeStage.value === 0 ? stage0Ref.value : stage1Ref.value))', 'const videoRef = ref(null)');
  ok(computedOk(bad), '变异自证：`videoRef` 改回普通 ref 竟然没被识别出来', true);
}

// ---- ② 换台顺序：先翻 activeStage、再 pause 旧那块 ----
const swapOrderOk = (src) => {
  const i = src.indexOf('activeStage.value = 1 - activeStage.value');
  const j = src.indexOf('prev.pause()');
  return i > 0 && j > i;
};
ok(swapOrderOk(SRC), '换台顺序反了：先 `prev.pause()` 再翻 `activeStage` —— 旧元素的 pause 会被当成"用户暂停"');
{
  const bad = SRC.replace(
    /  activeStage\.value = 1 - activeStage\.value([^\n]*)\n  prev\.pause\(\)/,
    '  prev.pause()\n  activeStage.value = 1 - activeStage.value$1',
  );
  assert.notEqual(bad, SRC, '变异自证：换台顺序那一处没匹配上 —— 自证台自己错了');
  mutations += 1;
  ok(swapOrderOk(bad), '变异自证：顺序反过来竟然没被识别出来', true);
}

// ---- ③ Dhampir 那条腿：源没就绪别画 + 备用源 + 换绑 ----
const gateOk = (src) => {
  const seekAt = src.indexOf('dhampirEngine.seek(f,');
  const gateAt = src.indexOf('if (!dhampirSourcesReady(f))');
  return gateAt > 0 && seekAt > gateAt;
};
ok(gateOk(SRC), '`dhampirSourcesReady(f)` 没有挡在 `engine.seek(f, …)` 之前 —— 源没像素时又会画出整块黑');
ok(/function dhampirSourcesReady\(f\)/.test(SRC), '没有 `dhampirSourcesReady`');
ok(/let dhampirTwins = new Map\(\)/.test(SRC) && /dhampirWatchTwin\(assetId, twin\)/.test(SRC),
  'Dhampir 那条腿没有"备用那套源"（`dhampirTwins` + seeked 监听）—— 预滚无处可备');
ok(/dhampirEngine\.bindSource\(assetId, tw\.id\)/.test(SRC),
  '`dhampirSwapSources` 没有换绑（`bindSource`）—— 备用源备好了也上不了台');
ok(/function dhampirPrerollTarget\(t\)/.test(SRC) && /dhampirPrerollTarget\(v\.currentTime\)/.test(SRC),
  '没有"该不该预滚"的判据（`dhampirPrerollTarget`）或没人在 tick 里问它');
ok(/const PREROLL_SECS = 0\.8/.test(SRC), '`PREROLL_SECS` 不是 0.8（两条腿共用这个提前量）');
{
  const bad = mutate('    if (!dhampirSourcesReady(f)) {', '    if (false) {');
  ok(gateOk(bad), '变异自证：把"源没就绪不画"这道闸摘掉竟然没被识别出来', true);
}

// ---- ④ 共用的两块元素：事件只让台上那块改 UI 播放态 ----
ok(/function onStagePlay\(e\) \{\n  if \(e && e\.target === videoRef\.value\) setPlaying\(true\)/.test(SRC),
  '`onStagePlay` 没按"台上那块"过滤 —— 备用块的事件会改 UI 播放态');
ok(/function onStagePause\(e\) \{\n  if \(e && e\.target === videoRef\.value\) setPlaying\(false\)/.test(SRC),
  '`onStagePause` 没按"台上那块"过滤');
ok(/\.source-video-standby \{ position:absolute/.test(SRC),
  '缺 `.source-video-standby`：备用那块会铺满舞台（或用了 display:none —— 那样它不再解码）');

// ---- ⑤ **P3：wasm 是主腿、DOM 是兜底**（2026-10 补） ----
//
// ⚠️ 为什么必须钉：P3 的形态是"**默认**进 Dhampir，失败**自动退回** DOM"，
//    而它的实现只有**一处**（`onMounted` 里那个 `toggleDhampir()`）。
//    那一处掉了**不会报任何错** —— 所有门禁照旧全绿，只是预览静默退回 DOM 腿，
//    于是"`engine.js` 是模板的手工移植副本"这整类耦合**又回来了**（P3 存在的唯一理由）。
//    ⇒ 三件事都要钉：① 默认进；② 失败退回；③ DOM 腿**仍在**（它是兜底，不许被删）。
//
// ⚠️ 判据刻意**不钉那个 300ms 延时值**：它是体验调参（等布局稳定），
//    钉死会让"调成 200ms"这种无害改动报假红。钉的是**存在性**。
const autoSwitch = /if \(!dhampirMode\.value && !dhampirError\.value\) toggleDhampir\(\)/.test(SRC);
ok(autoSwitch, '没有"挂载后自动进 Dhampir"那一处 —— 预览会静默退回 DOM 腿（P3 归零，且不报错）');
ok(
  SRC.slice(SRC.indexOf('// ===== 默认进 Dhampir')).includes('toggleDhampir()'),
  '"自动进 Dhampir"没写在 `onMounted` 段里 —— 那样它根本不会被调用',
);

// ② 失败必须**自动**退回 DOM（而不是停在白屏/黑屏上）。
//    判据 = `toggleDhampir` 的 `catch` 里把 `dhampirMode` 翻回 false。
{
  const fnAt = SRC.indexOf('async function toggleDhampir()');
  const catchAt = SRC.indexOf('} catch (e) {', fnAt);
  ok(
    catchAt > fnAt && SRC.slice(catchAt, catchAt + 1200).includes('dhampirMode.value = false'),
    '`toggleDhampir` 的 catch 里没有把 `dhampirMode` 翻回 false —— 起引擎失败时**退不回 DOM**（白屏）',
  );
}

// ③ 兜底那条腿还在（P3 的口径是"DOM **降兜底**"，**不是**删除；删除是 P3 之外的独立决策）。
ok(/ref="stage[01]Ref"/.test(SRC), 'DOM 预览腿被删了 —— P3 要求它**降为兜底**、不是消失');

// 变异自证：把"默认进"那一行摘掉，必须被识别成"不再默认进"。
{
  const noAuto = mutate(
    '    if (!dhampirMode.value && !dhampirError.value) toggleDhampir()',
    '    // 有人顺手注释掉了',
  );
  // ⚠️ `ok(..., true)` 的约定是"传**未变异时**该成立的条件，由它取反"（见上面的 `ok`）。
  //    这里直接传"还找得到那句话"，变异后应当**找不到** ⇒ 取反后成立 ✓。
  ok(
    /if \(!dhampirMode\.value && !dhampirError\.value\) toggleDhampir\(\)/.test(noAuto),
    '变异自证：把"自动进 Dhampir"摘掉竟然没被识别出来',
    true,
  );
}

// ---- ⑥ **根因：段起点的浮点陷阱**（2026-10-19）----
//
// 用户报「播放头落在屏蔽段里开始播 = 持续卡顿」的根因不在 tick 的跳转逻辑，而在
// dhampirFrameAt 的**边界判据**：跳转目标是 sourceIn / srcFps 这个秒值，乘回去会得到
// 122.99999999999999 < 123（实测 30fps 下 0..200000 帧里 **3270** 个帧踩中、
// 25fps **9174** 个、60fps **3270** 个）⇒ 判定永远说"这里没有帧" ⇒ 每帧重跳同一个位置。
//
// 判据分两层，缺一不可：
//   ① 静态：段起点比较必须减掉半个素材帧的容差（edgeTol = 0.5）；
//   ② 行为：把 dhampirFrameAt / dhampirNextSourceStart / dhampirSnapForward /
//      dhampirJumpTarget 从源码里抠出来**真跑一遍**，喂一份带 123 帧起点的段落表。
//      变异（改回 srcPos >= seg.sourceIn）必须让**两层一起翻面**。
const fnBody = (src, name) => {
  const start = src.indexOf('function ' + name + '(');
  if (start < 0) return '';
  let i = src.indexOf('{', start);
  let depth = 0;
  for (; i < src.length; i += 1) {
    const ch = src[i];
    if (ch === '/' && src[i + 1] === '/') {
      const nl = src.indexOf('\n', i);
      if (nl < 0) break;
      i = nl;
      continue;
    }
    if (ch === '/' && src[i + 1] === '*') {
      const e = src.indexOf('*/', i + 2);
      i = e < 0 ? src.length : e + 1;
      continue;
    }
    if (ch === "'" || ch === '"' || ch === '`') {
      const q = ch;
      i += 1;
      while (i < src.length && src[i] !== q) {
        if (src[i] === '\\') i += 1;
        i += 1;
      }
      continue;
    }
    if (ch === '{') depth += 1;
    else if (ch === '}') {
      depth -= 1;
      if (depth === 0) return src.slice(start, i + 1);
    }
  }
  return '';
};
const constNum = (src, name) => {
  const m = src.match(new RegExp('const ' + name + ' = ([0-9.]+)'));
  return m ? Number(m[1]) : null;
};
const edgeTolOk = (src) => {
  const f = fnBody(src, 'dhampirFrameAt');
  return /const edgeTol = 0\.5/.test(f)
    && /srcPos >= seg\.sourceIn - edgeTol && srcPos < seg\.sourceIn \+ lenSrc/.test(f);
};
ok(edgeTolOk(SRC), 'dhampirFrameAt 的段起点比较没留半个素材帧容差 —— 跳转目标乘回去 122.99999999999999 就永远判"没有帧"，每帧重跳 = 持续卡顿');

// 行为级：真跑一遍抠出来的四个函数。段落表 A[0,100) 帧 / 屏蔽 [100,123) / B 从 **123 帧**起 / C 从 900 帧起。
const jumpMathOf = (src) => {
  const parts = ['dhampirFrameAt', 'dhampirNextSourceStart', 'dhampirSnapForward', 'dhampirJumpTarget']
    .map((n) => fnBody(src, n));
  if (parts.some((p) => !p)) return null;
  const step = constNum(src, 'DHAMPIR_SNAP_STEP_FRAMES');
  const maxFrames = constNum(src, 'DHAMPIR_SNAP_MAX_FRAMES');
  if (step === null || maxFrames === null) return null;
  const segments = [
    { frameStart: 0, frameEnd: 100, sourceIn: 0 },
    { frameStart: 100, frameEnd: 200, sourceIn: 123 },
    { frameStart: 200, frameEnd: 300, sourceIn: 900 },
  ];
  try {
    return new Function(
      'let dhampirFps={value:30},dhampirSourceFps=30;let dhampirSegments=' + JSON.stringify(segments) + ';'
        + 'const DHAMPIR_SNAP_STEP_FRAMES=' + step + ',DHAMPIR_SNAP_MAX_FRAMES=' + maxFrames + ';'
        + parts.join('\n')
        + ';return { frameAt: dhampirFrameAt, nextStart: dhampirNextSourceStart, jumpTarget: dhampirJumpTarget };',
    )();
  } catch (e) {
    return null;
  }
};
const jumpMathOk = (m) => !!m
  && m.frameAt(110 / 30) === null                 // 110 帧（3.666s）落在被掐掉的 [100,123) 里
  && m.nextStart(110 / 30) !== null
  && m.frameAt(m.nextStart(110 / 30)) === 100     // ★ 保证：nextSourceStart 的原样输出落回段落表必须"有帧"
  && m.jumpTarget(110 / 30) !== null
  && m.frameAt(m.jumpTarget(110 / 30)) === 100
  && m.jumpTarget(32) === null;                   // 片尾：后面没有保留段 ⇒ 不许瞎跳
ok(jumpMathOk(jumpMathOf(SRC)), '行为级：跳转目标（含 nextSourceStart 的原样输出）落回段落表是 null —— 素材帧率换算 / 容差 / 收敛有一处不对');
{
  const bad = mutate('seg.sourceIn - edgeTol', 'seg.sourceIn');
  ok(edgeTolOk(bad), '变异自证：段起点改回 srcPos >= seg.sourceIn 竟然没被识别出来', true);
  ok(jumpMathOk(jumpMathOf(bad)), '变异自证：去掉容差后行为级判据竟然没翻面（123/30*30 那一下没抓到）', true);
}

// 段尾**不许**跟着放宽：会把下一段的开头吞进上一段（跨屏蔽区间时多放出该掐掉的内容）。
const endStrictOk = (src) => {
  const f = fnBody(src, 'dhampirFrameAt');
  return /srcPos < seg\.sourceIn \+ lenSrc\)/.test(f) && !/seg\.sourceIn \+ lenSrc \+/.test(f);
};
ok(endStrictOk(SRC), 'dhampirFrameAt 的段尾比较被放宽了 —— 会把下一段开头吞进上一段（跨屏蔽区间多放内容）');
{
  const bad = mutate('srcPos < seg.sourceIn + lenSrc)', 'srcPos < seg.sourceIn + lenSrc + edgeTol)');
  ok(endStrictOk(bad), '变异自证：段尾也加容差竟然没被识别出来', true);
}

// ---- ⑦ 跳转目标要**推进到真的有帧**，且所有消费者吃同一个入口 ----
const snapOk = (src) => {
  const snap = fnBody(src, 'dhampirSnapForward');
  const jump = fnBody(src, 'dhampirJumpTarget');
  if (!snap || !jump) return false;
  // 契约：1/4 素材帧为步长、最多前挪 2 帧（再大就会把下一段开头挪成第 3 帧 = 丢内容）
  if (constNum(src, 'DHAMPIR_SNAP_STEP_FRAMES') !== 0.25) return false;
  if (constNum(src, 'DHAMPIR_SNAP_MAX_FRAMES') !== 2) return false;
  return /const raw = dhampirNextSourceStart\(seconds\)/.test(jump)
    && /return raw === null \? null : dhampirSnapForward\(raw\)/.test(jump)
    && /if \(dhampirFrameAt\(t\) !== null\) return t/.test(snap)
    && /dhampirFrameAt\(x\) !== null/.test(snap);
};
ok(snapOk(SRC), '没有 dhampirSnapForward（1/4 素材帧、最多 2 帧）或 dhampirJumpTarget 没让 nextSourceStart 的输出过它');
const consumersOk = (src) => {
  const preroll = fnBody(src, 'dhampirPrerollTarget');
  const sched = fnBody(src, 'scheduleDhampir');
  const tickFn = fnBody(src, 'tick');
  const playable = fnBody(src, 'playableJumpTarget');
  return /return dhampirJumpTarget\(t\)/.test(preroll)
    && /dhampirJumpTarget\(el \? el\.currentTime : current\.value\)/.test(sched)
    && /const target = dhampirJumpTarget\(v\.currentTime\)/.test(tickFn)
    && /const next = dhampirJumpTarget\(t\)/.test(playable)
    && !/const target = dhampirNextSourceStart\(/.test(src);
};
ok(consumersOk(SRC), 'tick / scheduleDhampir / dhampirPrerollTarget / playableJumpTarget 没全走 dhampirJumpTarget —— 预滚与跳会错开（白备一趟）');
{
  const bad = mutate('  return raw === null ? null : dhampirSnapForward(raw)', '  return raw');
  ok(snapOk(bad), '变异自证：wrapper 里去掉 snap 竟然没被识别出来', true);
}
{
  const bad = mutate('      const target = dhampirJumpTarget(v.currentTime)', '      const target = dhampirNextSourceStart(v.currentTime)');
  ok(consumersOk(bad), '变异自证：tick 退回裸 nextSourceStart 竟然没被识别出来', true);
}

// ---- ⑧ 从屏蔽段里按播放 = **play() 之前**就落到可播放位置 ----
// 判据看的是 togglePlay 的**函数体里的调用顺序**（不是全文 grep 名字）：
//   playableJumpTarget()  →  v.currentTime = jumpTarget  →  v.play()
const prePlayJumpOk = (src) => {
  const body = fnBody(src, 'togglePlay');
  if (!body) return false;
  const callAt = body.indexOf('playableJumpTarget()');
  const seekAt = body.indexOf('v.currentTime = jumpTarget');
  const playAt = body.indexOf('v.play()');
  // 起播这一跳不许上定格遮罩：那时旧画面还没上屏（遮罩是"已经在播"才用的）
  return callAt > 0 && seekAt > callAt && playAt > seekAt && !/beginSkipFreeze\(/.test(body);
};
ok(prePlayJumpOk(SRC), 'togglePlay 没在 play() 之前调 playableJumpTarget() 并落位 —— 从屏蔽段起播仍会先放一帧旧画面再抖一下');
{
  // 变异：把整套起播跳挪到 play() **之后**（= 回到旧行为），顺序判据必须翻面。
  const preBlock = [
    '    const jumpTarget = playableJumpTarget()',
    '    if (jumpTarget !== null) {',
    '      v.currentTime = jumpTarget',
    '      syncDhampirTime(jumpTarget)',
    '      current.value = v.currentTime',
    '      player.seek(current.value)',
    '      prerollReset()        // 手动跳位后上一趟预滚的目标就作废了（与 seekTo 同一条纪律）',
    "      emit('time', v.currentTime)",
    '      if (bgOn.value && bgRef.value) bgRef.value.currentTime = jumpTarget',
    '    }',
    '',
  ].join('\n');
  const moved = SRC.replace(preBlock, '');
  const bad = moved.replace(
    '    v.play().catch(() => {})\n    setPlaying(true)',
    '    v.play().catch(() => {})\n    playableJumpTarget()\n    setPlaying(true)',
  );
  assert.ok(moved !== SRC && bad !== moved, '变异自证：起播跳那一块没匹配上 —— 自证台自己错了');
  mutations += 1;
  ok(prePlayJumpOk(bad), '变异自证：把起播跳挪到 play() 之后竟然没被识别出来', true);
}

// ---- ⑨ 防呆：同一个跳转目标不允许短时间被重复 seek ----
// 这不是当前 bug 的修法（那是 ⑥⑦），而是给"未来任何一环被改回去"准备的护栏：
// 失败模式是每帧重跳同一个位置（浏览器每帧打断解码器 ⇒ 永久卡顿，且**不报错**）。
const jumpGuardOk = (src) => {
  const fn = fnBody(src, 'dhampirJumpGuardAllows');
  const tickFn = fnBody(src, 'tick');
  const sched = fnBody(src, 'scheduleDhampir');
  return /const DHAMPIR_JUMP_GUARD_MS = 400/.test(src)
    && /Math\.abs\(target - st\.target\) < 1e-6/.test(fn)
    && /addClient\(/.test(fn)
    && /dhampirJumpGuardAllows\('tick', target\)/.test(tickFn)
    && /dhampirJumpGuardAllows\('schedule', target\)/.test(sched);
};
ok(jumpGuardOk(SRC), '没有"同一跳转目标 400ms 内不许重跳"的闸门（tick 与 scheduleDhampir 两处都要挂）—— 未来任何回归都会变成无限重跳');
{
  const bad = mutate("if (target !== null && dhampirJumpGuardAllows('tick', target)) {", 'if (target !== null) {');
  ok(jumpGuardOk(bad), '变异自证：摘掉 tick 的防重跳闸竟然没被识别出来', true);
}


console.log('✓ 精修预览 A/B 预滚：' + checks + ' 项全绿（含 ' + mutations + ' 处变异自证）');
console.log('  台上那块 = computed(activeStage) / 换台先翻 activeStage 再 pause / Dhampir 源没就绪不画 + 备用源换绑');
console.log('  P3：wasm 主腿（挂载自动进 + 失败自动退回 DOM + DOM 兜底仍在）');
console.log('  屏蔽段起播：段起点半个素材帧容差 / 跳转目标 snap 到有帧处 / 同一目标 400ms 防重跳 / play() 前落位');
