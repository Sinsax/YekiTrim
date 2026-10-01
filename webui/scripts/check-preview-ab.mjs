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
import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const SFC = join(HERE, '..', 'src', 'components', 'PolishPreview.vue');
const SRC = readFileSync(SFC, 'utf8');

let checks = 0;
const ok = (cond, what, mutated = false) => {
  checks += 1;
  assert.ok(mutated ? !cond : cond, what);
};

/** 把源码里某段替换一次（变异自证用）。 */
function mutate(from, to) {
  const hits = SRC.split(from).length - 1;
  assert.equal(hits, 1, `变异台自己错了：「${from.slice(0, 40)}…」在 SFC 里命中 ${hits} 次（必须是 1）`);
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

console.log(`✓ 精修预览 A/B 预滚：${checks} 项全绿（含 3 处变异自证）`);
console.log('  台上那块 = computed(activeStage) / 换台先翻 activeStage 再 pause / Dhampir 源没就绪不画 + 备用源换绑');
