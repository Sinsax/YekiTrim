// 精修预览引擎（`webui/src/polish/engine.js`）的**运行期**守卫。
//
// # 为什么要有它（2026-10-01 的真实事故）
//
// 上一轮删四个效果类型时，同一个手法在三处留下连带损伤，而**门禁一条都没红**：
//   ① `buildIndex` 的桶清单丢了 `vignette` / `sfx`（模板那份是全的）⇒ 工程里**恰好没有该类型事件**时
//      桶不会按需创建 ⇒ `getVignette(t, idx.vignette)` 里 `for (const v of undefined)` 抛
//      `can't access property Symbol.iterator, d.vignette is undefined`；
//   ② `videoFxAt` 的返回对象被削成 `{shake, blur}` ⇒ 预览读 `f.camera.handheld` 抛
//      `can't access property "handheld", i.camera is undefined`；
//   ③ 词汇表里 `overlay.ui` 写成对象 `{}` ⇒ 编辑器 `(e.ui ?? []).includes(...)` 抛
//      `(intermediate value).includes is not a function`。
// 三者都会让 **Vue 把整块子树 unmount** —— 用户看到的是"预览区的视频和控件一起消失、还不报错"，
// 而服务端日志干干净净（全是 200）。**静态读源码抓不住**，所以这里真跑一遍。
//
// engine.js 是**纯 ESM、零依赖**（浏览器预览的移植副本），因此可以在 node 里直接 import：
// 用一份"**一个事件都没有**"的最小工程建索引（正是 ① 的触发条件），再把预览会调的那一族求值器
// 逐个跑一遍。判据写错会"永远绿"，所以自带 `--self-test`：故意删一个桶，必须能抛出来。
import { readFileSync } from 'node:fs';

const ENGINE = 'webui/src/polish/engine.js';
const TEMPLATE = 'crates/vtrim-polish/templates/index.html';

/** 预览真正会调的那一族（签名都是 `(idx, t)`）。 */
const IDX_T_FAMILY = [
  'videoFxAt', 'cameraAt', 'getFlash', 'getHueShiftFlash', 'getNoiseOpacity', 'getVignette',
  'getOverlayState', 'subsAt', 'danmakuAt', 'stickersAt',
];

let checks = 0;
const fail = (msg) => {
  throw new Error(msg);
};

/** 模板 `buildIndex` 里预建的桶集合（出片权威路径那份）。 */
function templateBuckets(text) {
  const at = text.indexOf('function buildIndex()');
  if (at < 0) fail(`模板里找不到 buildIndex（${TEMPLATE}）`);
  const seg = text.slice(at, at + 4000);
  const names = new Set();
  // 形如 `camera: [], sticker: [],`（**一行多个**也要全抓到 —— 我第一版带 `^` 锚点，只抓到每行第一个，
  // 于是把"两边不一致"误报成真：模板那份其实是全的）
  for (const m of seg.matchAll(/([a-z_]+):\s*\[\]/g)) names.add(m[1]);
  return names;
}

async function main() {
  const mod = await import('../webui/src/polish/engine.js');
  const buckets = mod.INDEXED_TYPES;
  if (!Array.isArray(buckets) || buckets.length === 0) fail('engine.js 没有导出 INDEXED_TYPES（桶清单只许有一份）');
  checks += 1;

  // ---- ① 桶清单：engine.js ↔ 模板 必须同一个集合 ----
  const tpl = templateBuckets(readFileSync(TEMPLATE, 'utf8'));
  const eng = new Set(buckets);
  const onlyTpl = [...tpl].filter((n) => !eng.has(n));
  const onlyEng = [...eng].filter((n) => !tpl.has(n));
  if (onlyTpl.length || onlyEng.length) {
    fail(
      `桶清单两边不一致（两份实现必须同一个集合）：\n` +
        `  只有模板有：${onlyTpl.join(', ') || '(无)'}\n` +
        `  只有 engine.js 有：${onlyEng.join(', ') || '(无)'}`,
    );
  }
  checks += 1;
  console.log(`  ✓ 桶清单两边一致（${buckets.length} 个：${buckets.join(', ')}）`);

  // ---- ② 空工程建索引：每个桶都必须是数组（不是 undefined）----
  const empty = mod.buildIndex({ config: { segments: [] }, subtitles: [], danmaku: [], duration: 10 });
  const missing = buckets.filter((t) => !Array.isArray(empty[t]));
  if (missing.length) {
    fail(
      `空工程下这些桶不是数组：${missing.join(', ')}\n` +
        `  ⇒ 工程里没有该类型事件时它就是 undefined，求值器一迭代就抛（预览整块消失）`,
    );
  }
  checks += 1;
  console.log(`  ✓ 空工程的每一个桶都是数组（这正是事故的触发条件）`);

  // ---- ③ 预览那一族求值器：逐个真跑，不许抛 ----
  for (const name of IDX_T_FAMILY) {
    const fn = mod[name];
    if (typeof fn !== 'function') fail(`engine.js 没导出 ${name}（预览会 import 它）`);
    checks += 1;
    try {
      fn(empty, 1.0);
    } catch (e) {
      fail(`${name}(idx, 1.0) 抛了：${e && e.message}`);
    }
  }
  console.log(`  ✓ ${IDX_T_FAMILY.length} 个求值器在空工程上都能跑（不抛）`);

  // ---- ④ `videoFxAt` 的返回字段就是预览的契约 ----
  const vfx = mod.videoFxAt(empty, 1.0);
  for (const field of ['shake', 'blur', 'camera', 'zoom']) {
    checks += 1;
    if (vfx[field] === undefined) {
      fail(`videoFxAt(...).${field} 是 undefined —— 预览读它（如 \`f.camera.handheld\`）会抛，整块预览消失`);
    }
  }
  if (vfx.camera === null || typeof vfx.camera !== 'object') fail('videoFxAt(...).camera 必须是个对象（预览读 .handheld/.ox/.oy）');
  checks += 1;
  console.log(`  ✓ videoFxAt 返回 ${Object.keys(vfx).join(', ')}（camera.handheld=${vfx.camera.handheld}）`);

  // ---- ⑤ 词汇表的 `ui` 必须是数组（编辑器 `.includes` 的前提）----
  const spec = JSON.parse(readFileSync('spec/effects.json', 'utf8'));
  const badUi = spec.effects.filter((e) => e.ui !== undefined && !Array.isArray(e.ui));
  if (badUi.length) {
    fail(`这些效果的 ui 不是数组：${badUi.map((e) => `${e.id}=${JSON.stringify(e.ui)}`).join(', ')} —— 编辑器会抛 .includes is not a function`);
  }
  checks += 1;
  console.log(`  ✓ 词汇表 ${spec.effects.length} 个效果的 ui 都是数组（或缺省）`);
}

// ---------------------------------------------------------------------------
// 自证：往空索引里删一个桶，那一族求值器**必须**抛 —— 否则上面那些断言是空转的
// ---------------------------------------------------------------------------
async function selfTest() {
  const mod = await import('../webui/src/polish/engine.js');
  const idx = mod.buildIndex({ config: { segments: [] }, subtitles: [], danmaku: [], duration: 10 });
  delete idx.vignette;
  let threw = false;
  try {
    mod.getVignette(idx, 1.0);
  } catch {
    threw = true;
  }
  if (!threw) fail('自证失败：删掉 `vignette` 桶之后 getVignette 竟然没抛 —— 说明判据是空转的');
  delete idx.camera;
  let threw2 = false;
  try {
    mod.videoFxAt(idx, 1.0).camera.handheld;
  } catch {
    threw2 = true;
  }
  if (!threw2) fail('自证失败：删掉 `camera` 桶之后 videoFxAt(...).camera 竟然没抛');
  console.log('✓ 预览引擎守卫自证：删桶之后那两族求值器都会抛（判据不是空转的）');
}

if (process.argv.includes('--self-test')) {
  await selfTest();
} else {
  await main();
  console.log(`✓ 预览引擎守卫：${checks} 项全绿（空工程不炸 / 求值器不抛 / 视频层字段齐 / 两份桶清单一致）`);
}
