// 精修预览的**出声**契约守卫 —— 零依赖、不 import 应用模块（与其它 check-*.mjs 同构）。
//
// 守的是 2026-10-02 修的那条"预览完全没有声音"：`55f2f16` 让 **Dhampir 默认开启**，
// 而声音那条链上三处仍按"Dhampir 是可选模式、预览不带声音"写 —— 于是：
//
//   ① `startDhampir` 建源时 `v.muted = true` ⇒ 说话声/音乐一点都没有
//      （台上那块 `<video>` 与 `dhampirVideos` 是**同一批元素**，它同时是引擎的帧来源）
//   ② `tick()` 的 Dhampir 分支在 `player.sync(...)` **之前就 return** ⇒ 音效也没有
//      （"让出重活"让的是 DOM/CSS **渲染**，而音效是**出声**，两件事被混成了一件）
//   ③ `applyAudio()` 只写 `videoRef.value` ⇒ 音量/静音控件对 Dhampir 那批源**完全打空**
//      （用户看到的正是"音量控制不好使"）
//
// 还有一条是修①的过程中**由行为测试实测抓出来的**（不是读代码想出来的）：
//   ④ `applyAudio` 只遍历 `dhampirVideos` 不够 —— `dhampirSwapSources` 换绑时把旧元素
//      **换出**那个数组（同时记成新备用），那一瞬间它**不在**数组里却**仍在播**
//      （`pause()` 之后下一帧镜像循环又按时钟把它 play 起来）⇒ **两路同源同时在轨、声音翻倍**。
//      判据：必须覆盖 `dhampirSourceVideos` ∪ `dhampirTwins`。
//
// 每条断言都配一处**变异自证**（把源码改成"错的那一版"必须翻面）——
// 少了它，断言可能只是匹配了注释里的字样（本仓踩过这个坑）。
import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';

// ⚠️ 本脚本住在 `scripts/`（与 check-preview-engine.mjs 同处），路径**相对于仓库根** ——
// 门禁链 `.workbuddy/gates.mjs` 是以仓库根为 cwd 跑它的，别改成 `import.meta.url` 相对。
const SFC = 'webui/src/components/PolishPreview.vue';
const SRC = readFileSync(SFC, 'utf8');

let checks = 0;
let mutations = 0;
const ok = (cond, what, mutated = false) => {
  checks += 1;
  assert.ok(mutated ? !cond : cond, what);
};
/** 把源码里某段替换一次（变异自证用）。 */
function mutate(from, to) {
  const hits = SRC.split(from).length - 1;
  assert.equal(hits, 1, `变异台自己错了：「${from.slice(0, 48)}…」在 SFC 里命中 ${hits} 次（必须是 1）`);
  mutations += 1;
  return SRC.replace(from, to);
}

/** 取一个函数的**函数体文本**（从 `function NAME(` 到它那个收尾的 `}\n`）。 */
function bodyOf(src, name) {
  const start = src.indexOf(`function ${name}(`);
  if (start < 0) return '';
  const end = src.indexOf('\n}\n', start);
  return end < 0 ? src.slice(start) : src.slice(start, end + 2);
}

// ---- ① 源必须按用户声明的状态出声（不许写死 muted = true）----
const sourcesAudibleOk = (src) => {
  const fn = bodyOf(src, 'startDhampir');
  if (!fn) return false;
  // 建源那个循环里再出现 `v.muted = true` 就是回退
  return !/for \(const v of dhampirVideos\)[^}]*\.muted = true/.test(fn);
};
ok(sourcesAudibleOk(SRC), 'startDhampir 又把源写死成 `v.muted = true`（说话声/音乐全没了）—— 出声必须由 applyAudio 决定');
{
  const bad = mutate(
    '  for (const v of dhampirVideos) {\n    dhampirPreservePitch(v)\n    v.play().catch(() => {})\n  }',
    '  for (const v of dhampirVideos) {\n    v.muted = true\n    v.play().catch(() => {})\n  }',
  );
  ok(sourcesAudibleOk(bad), '变异自证：写死 muted=true 竟然没被识别出来', true);
}

// ---- ② `startDhampir` 末尾必须铺一次音频状态 ----
const startAppliesOk = (src) => {
  const fn = bodyOf(src, 'startDhampir');
  return !!fn && /applyAudio\(\)/.test(fn);
};
ok(startAppliesOk(SRC), 'startDhampir 建完源没有 applyAudio()：源要么静音、要么音量不对');
{
  const bad = mutate('    dhampirPreservePitch(v)\n    v.play().catch(() => {})\n  }\n  applyAudio()',
                     '    dhampirPreservePitch(v)\n    v.play().catch(() => {})\n  }');
  ok(startAppliesOk(bad), '变异自证：去掉 startDhampir 里的 applyAudio 竟然没被识别出来', true);
}

// ---- ③ Dhampir 分支的 tick 必须调 player.sync（音效不能跟着"让位"让掉）----
const sfxInDhampirOk = (src) => {
  // ⚠️ 必须锚在 `tick` 里那个分支上：`if (dhampirMode.value) {` 在 `toggleDhampir` 里
  //    也出现一次（那里只管进出模式，没有 player.sync，也没有 emit('time')）——
  //    直接 indexOf 会命中它，判据就变成永远红（本脚本第一版就是这么错的）。
  //
  // ⚠️ **判据不是"sync 排在第一个 return 之前"**（第一版这么写，结果自己红了）：
  //    那个分支里本来就有两处**合法**的提前 return（A/B 换绑命中、定格遮罩兜底），
  //    它们只是"这一帧先不画/先跳"，不是"以后都不出声"。要钉的是
  //    **正常那一帧（收尾前）确实调了 sync** —— 也就是 sync 必须出现在
  //    该分支**最后那个 `requestAnimationFrame(tick)` 之前**。
  const fn = bodyOf(src, 'tick');
  if (!fn) return false;
  const i = fn.indexOf('if (dhampirMode.value) {');
  if (i < 0) return false;
  // 边界取到"非 Dhampir 路径"那句开头：**不能用第一个 `\n  }\n`** ——
  // 分支内部有嵌套的 `}`（if/for），第一个 2 空格缩进的闭合花括号会把它从中间切断
  // （2026-10-02 加了换台路径的 sync 之后，本脚本就是这么把自己判红的）。
  const endAt = fn.indexOf('// 预览时跳过手动屏蔽区间', i);
  const seg = fn.slice(i, endAt > i ? endAt : fn.length);
  const lastRaf = seg.lastIndexOf('requestAnimationFrame(tick)');
  if (lastRaf < 0) return false;
  // 要钉的是**收尾那一处**：sync 紧挨着**最后**那个 emit('time') 之前。
  // ⚠️ 不能只写 `seg.indexOf('player.sync(') < lastRaf` —— 换台路径（2026-10-02 补）在
  //    前面也有一个 sync，删掉收尾处的那个之后整段里**还剩**换台那个，判据就永远真
  //    （本脚本第二版就是这么把变异自证判红的）。
  // ⚠️ emit 取 lastIndexOf 而不是 indexOf：前面还有提前 return 的分支各自 emit('time')。
  const emitAt = seg.lastIndexOf("emit('time', v.currentTime)");
  if (emitAt < 0) return false;
  const before = seg.slice(0, emitAt);
  const syncAt = before.lastIndexOf('player.sync(');
  if (syncAt < 0) return false;
  // 与 emit 之间只能有空白/换行
  if (before.slice(syncAt + 'player.sync('.length).includes('player.sync(')) return false;
  return /player\.sync\([^\n]*\)\s*$/.test(before.trimEnd()) ;
};
ok(sfxInDhampirOk(SRC), 'Dhampir 分支的 tick 在 player.sync 之前就 return 了 —— 预览连音效都没有');
{
  const bad = mutate('    player.sync(idx.value.soundEvents, v.currentTime)\n    emit(\'time\', v.currentTime)',
                     '    emit(\'time\', v.currentTime)');
  ok(sfxInDhampirOk(bad), '变异自证：删掉 Dhampir 分支收尾处的 player.sync 竟然没被识别出来', true);
  // **换台路径那一处也要钉住**（2026-10-02 补）：它把时钟一次跳过整段，
  // 少了这一句，落在被跳过区间里的音效没有任何一帧能覆盖 ⇒ 永久丢失
  // —— 这正是"有些音效播了、有些没有"。
  ok(/if \(dhampirSwapSources\(\)\) \{[\s\S]*?player\.sync\(/.test(SRC),
     '换台路径（一次跳过整段）没有补 player.sync —— 被跳过区间里的音效永久丢失');
  {
    const bad2 = mutate('          player.sync(idx.value.soundEvents, v.currentTime)\n          emit(\'time\', v.currentTime)',
                        '          emit(\'time\', v.currentTime)');
    ok(!/if \(dhampirSwapSources\(\)\) \{[\s\S]*?player\.sync\(/.test(bad2),
       '变异自证：删掉换台路径的 player.sync 竟然没被识别出来', true);
  }
}

// ---- ④ applyAudio 必须覆盖"所有会出声的元素"，不能只遍历 dhampirVideos ----
const coversAllOk = (src) => {
  const fn = bodyOf(src, 'applyAudio');
  if (!fn) return false;
  return /dhampirAllSourceVideos\(\)/.test(fn) && !/for \(const dv of dhampirVideos\)/.test(fn);
};
ok(coversAllOk(SRC), 'applyAudio 只遍历 dhampirVideos —— 换绑中间态里旧元素掉出数组却仍在播 ⇒ 声音翻倍');
{
  const bad = mutate('  for (const dv of dhampirAllSourceVideos()) {', '  for (const dv of dhampirVideos) {');
  ok(coversAllOk(bad), '变异自证：改回只遍历 dhampirVideos 竟然没被识别出来', true);
}

// ---- ⑤ 并集必须真的包含两批 ----
const unionOk = (src) => {
  const fn = bodyOf(src, 'dhampirAllSourceVideos');
  if (!fn) return false;
  return /dhampirSourceVideos\.values\(\)/.test(fn) && /dhampirTwins\.values\(\)/.test(fn);
};
ok(unionOk(SRC), 'dhampirAllSourceVideos 没同时取 dhampirSourceVideos 与 dhampirTwins —— 会漏掉一批');
{
  const bad = mutate('  for (const tw of dhampirTwins.values()) { if (tw && tw.el) out.push(tw.el) }',
                     '  // 变异：丢掉备用那批');
  ok(unionOk(bad), '变异自证：并集里漏掉备用那批竟然没被识别出来', true);
}

// ---- ⑥ 备用那批永远不出声（判据是 twin 身份，不是下标）----
const twinSilentOk = (src) => {
  const fn = bodyOf(src, 'applyAudio');
  return !!fn && /dhampirIsTwin\(dv\)/.test(fn);
};
ok(twinSilentOk(SRC), 'applyAudio 没把备用那批压成静音 —— 换台瞬间两路同源叠加出两声');
{
  const bad = mutate('    dv.muted = m || dhampirIsTwin(dv)', '    dv.muted = m');
  ok(twinSilentOk(bad), '变异自证：去掉 twin 判据竟然没被识别出来', true);
}

// ---- ⑦ 出声时纠偏速率收窄 + 音高保持两道防线 ----
const rateRangeOk = (src) => {
  const fn = bodyOf(src, 'dhampirAlignRateRange');
  return !!fn && /DHAMPIR_ALIGN_RATE_AUDIBLE_LO/.test(fn) && /dhampirSourceAudible\(\)/.test(fn);
};
ok(rateRangeOk(SRC), '纠偏速率没有"出声时收窄"这一档 —— ±2% 会让预览跑调（约 ±35 音分）');
{
  const bad = mutate('  if (!dhampirSourceAudible()) return { lo: DHAMPIR_ALIGN_RATE_LO, hi: DHAMPIR_ALIGN_RATE_HI }\n  if (dhampirSourcesPitchLocked()) return { lo: DHAMPIR_ALIGN_RATE_LO, hi: DHAMPIR_ALIGN_RATE_HI }',
                     '  return { lo: DHAMPIR_ALIGN_RATE_LO, hi: DHAMPIR_ALIGN_RATE_HI }');
  ok(rateRangeOk(bad), '变异自证：去掉收窄分支竟然没被识别出来', true);
}

const pitchOk = (src) => {
  const fn = bodyOf(src, 'dhampirPreservePitch');
  return !!fn && /preservesPitch/.test(fn) && /webkitPreservesPitch/.test(fn);
};
ok(pitchOk(SRC), 'dhampirPreservePitch 没覆盖三个前缀属性 —— 老实现下纠偏会跑调');
{
  const bad = mutate(`  for (const key of ['preservesPitch', 'mozPreservesPitch', 'webkitPreservesPitch']) {`,
                     `  for (const key of ['preservesPitch']) {`);
  ok(pitchOk(bad), '变异自证：只留一个前缀竟然没被识别出来', true);
}

// ---- ⑧ 纠偏真的用了那个区间（函数写了不调用 = 空转）----
const alignUsesOk = (src) => {
  const fn = bodyOf(src, 'dhampirAlignSources');
  if (!fn) return false;
  return /dhampirAlignRateRange\(\)/.test(fn) && /base \* lo/.test(fn) && !/DHAMPIR_ALIGN_RATE_LO : base \* DHAMPIR_ALIGN_RATE_HI/.test(fn);
};
ok(alignUsesOk(SRC), 'dhampirAlignSources 没走 dhampirAlignRateRange()（函数写了不调用 = 收窄空转）');
{
  const bad = mutate('    const rate = err > 0 ? base * lo : base * hi',
                     '    const rate = err > 0 ? base * DHAMPIR_ALIGN_RATE_LO : base * DHAMPIR_ALIGN_RATE_HI');
  ok(alignUsesOk(bad), '变异自证：绕开收窄区间竟然没被识别出来', true);
}

// ---- ⑨ 音高保持要在"进模式"与"换绑"两条路都装上 ----
const pitchInstalledOk = (src) => {
  const start = bodyOf(src, 'startDhampir');
  const swap = bodyOf(src, 'dhampirSwapSources');
  return /dhampirPreservePitch\(v\)/.test(start) && /dhampirPreservePitch\(tw\.el\)/.test(swap);
};
ok(pitchInstalledOk(SRC), '音高保持没在 建源 / 换绑 两条路都装上 —— 换过去那一路会带着旧速率跑调');
{
  const bad = mutate('    dhampirPreservePitch(tw.el)\n', '');
  ok(pitchInstalledOk(bad), '变异自证：换绑那条路漏装竟然没被识别出来', true);
}

// ---- ⑩ 台上那块（DOM 腿）在 Dhampir 模式下必须闭麦 ----
//
// 两条腿指向**同一段素材**：DOM 腿 = stage0/stage1 这对 A/B <video>（模板里 stage0 **没有**
// muted 属性，它就是自绘预览的声源），引擎腿 = dhampirVideos 那批（既是帧来源也是声源）。
// 两条腿都出声 = 同一句话听两遍（2026-10-02 用户实测报的"视频声音播了两次"）。
// `toggleDhampir` 只 pause 台上那块**一次**，而 `scheduleDhampir` 的镜像循环每帧把它对齐到
// 引擎源的状态（play/pause）—— 只要它被任何路径重新 play 起来就又会出声，**光靠 pause 挡不住**。
const stageMutedInDhampirOk = (src) => {
  const fn = bodyOf(src, 'applyAudio');
  if (!fn) return false;
  return /v\.muted = m \|\| dhampirMode\.value/.test(fn);
};
ok(stageMutedInDhampirOk(SRC), 'applyAudio 没在 Dhampir 模式下把台上那块闭麦 —— DOM 腿与引擎腿同源齐响 ⇒ 声音播两遍');
{
  const bad = mutate('    v.muted = m || dhampirMode.value', '    v.muted = m');
  ok(stageMutedInDhampirOk(bad), '变异自证：台上那块的 Dhampir 闭麦被判掉竟然没被识别出来', true);
}

// ---- ⑪ SFX 播放器也必须由 applyAudio 铺（否则只有两个滑块路径能改它）----
//
// `player` 以前只由 `toggleMute` / `onVolume` 直接调 setMuted/setMasterVolume，
// 而 `applyAudio()` 完全不碰它 ⇒ **任何不经过那两个控件的路径**（进/出 Dhampir、建源、
// 换绑、恢复播放）都不会把音量铺到 SFX 上：滑块看着有效，音效仍按上一次的值出声。
const sfxVolumeOk = (src) => {
  const fn = bodyOf(src, 'applyAudio');
  if (!fn) return false;
  return /player\.setMasterVolume\(vol\)/.test(fn) && /player\.setMuted\(m\)/.test(fn);
};
ok(sfxVolumeOk(SRC), 'applyAudio 没把音量铺到音效播放器 —— 不走那两个滑块控件时音效仍按旧值出声');
{
  const bad = mutate('  player.setMasterVolume(vol)\n  player.setMuted(m)\n', '');
  ok(sfxVolumeOk(bad), '变异自证：SFX 没被 applyAudio 铺竟然没被识别出来', true);
}

// ---- ⑩ 拖动之后必须把引擎源叫醒（用户报的「拖动的时候直接没声音了」）----
//
// 现场：在一条 6 Mbps 的流上 seek ⇒ 浏览器丢掉解码缓冲、等同暂停；而时钟（`videoRef`）
// 是另一块元素且在 Dhampir 下 `muted = true`，它照旧在走 ⇒ **没有任何东西会重新 play()**：
//   · `dhampirAlignSources` 尾部那条兜底要求 `clockRunning`，拖动期间可能为 false ⇒ 直接 return；
//   · tick 的镜像循环只在「`v.paused !== el.paused`」时才动 ⇒ 两者"状态一致"时漏掉。
const seekWakeOk = (src) => {
  const fn = bodyOf(src, 'seekTo');
  if (!fn) return false;
  // 必须真的续播，且必须覆盖并集视图（换台中间态里旧元素会掉出 dhampirVideos）
  return /dhampirAllSourceVideos\(\)/.test(fn) && /\.play\(\)/.test(fn) && /dhampirIsTwin\(dv\)/.test(fn);
};
ok(seekWakeOk(SRC), 'seekTo 没有把引擎源叫醒 —— 拖动一次就静音（源在 6Mbps 流上 seek 后停住，没人续播）');
{
  const bad = mutate('      if (dv.paused) dv.play().catch(() => {})', '      void dv');
  ok(seekWakeOk(bad), '变异自证：去掉续播竟然没被识别出来', true);
}

// ---- ⑪ 拖动时"落位置"必须覆盖并集，不能只看台上那批 ----
const syncUnionOk = (src) => {
  const fn = bodyOf(src, 'syncDhampirTime');
  if (!fn) return false;
  // ⚠️ **必须先去掉注释再查**：上面那段说明里就写着 `dhampirAllSourceVideos()`，
  // 直接对整个函数体做正则会让变异（把真代码换成 `dhampirVideos`）**照样命中注释**，
  // 变异自证因此抓不到它（本仓既有纪律：判据走"去注释后的源码"，见 `check-stage-backdrop.mjs` ③b）。
  const code = fn.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
  return /dhampirAllSourceVideos\(\)/.test(code);
};
ok(syncUnionOk(SRC), 'syncDhampirTime 只遍历 dhampirVideos —— 换台中间态里掉出数组的元素位置不跟着走');
{
  // ⚠️ 锚点必须**这个函数独有**：`seen.add(v)` 只在 syncDhampirTime 里出现
  //（`applyAudio` 与 `seekTo` 也各有一处同类遍历，用那句当锚点会命中错的地方
  //  —— 本脚本的 `mutate` 只替换第一处）。
  const uniqSrc = '  for (const v of dhampirAllSourceVideos()) {' + '\n' +
    '    if (!v || seen.has(v)) continue';
  const uniqBad = '  for (const v of dhampirVideos) {' + '\n' +
    '    if (!v || seen.has(v)) continue';
  const bad = mutate(uniqSrc, uniqBad);
  ok(syncUnionOk(bad), '变异自证：syncDhampirTime 退回只看台上那批竟然没被识别出来', true);
}

// ---- ⑫ 出声跟着「正在播放」走：没在播的引擎源不许出声 ----
//
// 现场（2026-11-27 用户报"加载 Dhampir 预览时浏览器会播放视频的声音"）：出片页挂载默认进
// Dhampir，`startDhampir` 建完源就 `play()`（预热/followPlayback），而把源压回暂停的镜像循环
// 要等 `toggleDhampir` 里 `await startDhampir()` 之后的第一帧 rAF 才跑 —— 中间隔着动图/字幕/
// 字体几个 await。applyAudio 若只按「用户声明」铺状态，这几秒门帘还没揪、视频声音已经出来了
//（`play()` 当时元素还是 muted 的 ⇒ 浏览器放行，随后解除静音 ⇒ 无手势也出声）。
const audibleOnlyWhenPlayingOk = (src) => {
  const fn = bodyOf(src, 'applyAudio');
  return !!fn && /dv\.muted = m \|\| dhampirIsTwin\(dv\) \|\| !playing\.value/.test(fn);
};
ok(audibleOnlyWhenPlayingOk(SRC), 'applyAudio 的引擎源静音条件丢了 !playing.value —— 加载/暂停态的源自己出声（加载 Dhampir 预览时视频在响）');
{
  const bad = mutate('    dv.muted = m || dhampirIsTwin(dv) || !playing.value',
                     '    dv.muted = m || dhampirIsTwin(dv)');
  ok(audibleOnlyWhenPlayingOk(bad), '变异自证：去掉 !playing.value 竟然没被识别出来', true);
}

// ---- ⑬ 播放状态翻转必须经 applyAudio 落到元素 ----
//
// ⑫ 有了 `!playing.value` 之后，起播/暂停各要铺一次 —— 否则加载期被压静音的源起播后没人解除
//（画面动了、声音没回来）。`setPlaying` 是 playing 的唯一正规翻转点（换片段那次直赋值
// 自带 applyAudio，不走这里）。
const playLaysAudioOk = (src) => {
  const fn = bodyOf(src, 'setPlaying');
  return !!fn && /applyAudio\(\)/.test(fn);
};
ok(playLaysAudioOk(SRC), 'setPlaying 没有 applyAudio()：起播/暂停不翻源出声态 —— 加载期静音的源起播后没声音');
{
  const bad = mutate('  applyAudio()\n  emit(\'playing\', on)', '  emit(\'playing\', on)');
  ok(playLaysAudioOk(bad), '变异自证：删掉 setPlaying 里的 applyAudio 竟然没被识别出来', true);
}

console.log(`✓ 预览出声契约：${checks} 项全绿（含 ${mutations} 处变异自证）`);
console.log('  源出声(不写死 muted) / startDhampir 铺状态 / Dhampir tick 补 SFX / applyAudio 覆盖两批 / 备用恒静音 /');
console.log('  出声时纠偏收窄 + preservesPitch / 纠偏真走那个区间 / 建源与换绑都装音高保持 /');
console.log('  Dhampir 下台上那块闭麦 / 音量铺到 SFX / 换台补 sync /');
console.log('  拖动后叫醒引擎源 + 落位置覆盖并集 / 出声⇔正在播放 + setPlaying 铺状态');
