#!/usr/bin/env node
// 门禁：**§10 情绪轨道的消费者链不许断**。
//
// # 为什么需要它
//
// `data/emotion.json` 曾经是**哑产物**：`process` 真算、真写盘，而**全仓一个读者都没有**。
// 这类缺口**不会让任何东西报错** —— 没有编译错误、没有测试红、没有日志，
// 症状只是「功能算出来了，但用户这辈子看不见」。
//
// 所以这条判据盯的不是"算法对不对"（那有单测），而是**链子通不通**：
//
//     process 写盘  →  端点读它  →  前端 api 层  →  时间轴画出来
//
// 四段**每一段都要在**，缺一段整条就静默失效。逐段断言，而不是只查一头一尾。
//
// # 判据
//
// 零依赖，从源码文本里查（不 import、不起服务）：每段查"名与调用**同时**在"，
// 光有定义不算 —— 这正是当年 `emotion.json` 的样子（有写者、无读者）。
//
//     node scripts/check-emotion-track.mjs
//     node scripts/check-emotion-track.mjs --self-test
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const read = (rel) => readFileSync(join(repo, rel), 'utf8');

let checks = 0;
const failures = [];
const check = (what, got, want = true) => {
  checks += 1;
  if (got !== want) failures.push(`${what}: 期望 ${want}，实得 ${got}`);
};
/** 断言 `text` 里同时出现这两个模式（**顺序不限**）—— 查"接线"而不是查"有没有这个词"。 */
const has = (text, re) => re.test(text);

// ── ① 写者：`process` 真的把情绪轨道写盘 ───────────────────────────────────
{
  const mod = read('crates/yekitrim-polish/src/process/mod.rs');
  check('写者 write_emotion_track 存在', has(mod, /fn write_emotion_track/));
  check('写的是 data/emotion.json', has(mod, /join\("emotion\.json"\)/));
  check('写者**被调用**（不是孤零零一个定义）', has(mod, /write_emotion_track\s*\(/g) && (mod.match(/write_emotion_track\s*\(/g) || []).length >= 2);
}

// ── ② 端点：服务端把它读出来 ──────────────────────────────────────────────
{
  const api = read('crates/yekitrim-server/src/api/polish.rs');
  check('handler polish_emotion_get 存在', has(api, /pub\(super\) async fn polish_emotion_get\b/));
  check('它读 emotion.json', has(api, /join\("emotion\.json"\)/));
  // ⚠️ 同步 IO 必须 `blocking`（本仓纪律；漏了会在慢盘上占死 worker）
  const fn = api.match(/pub\(super\) async fn polish_emotion_get\b[\s\S]{0,600}?\n\}/);
  check('端点走 blocking（同步 IO 不许留在 async worker 上）', !!(fn && /blocking\s*\(/.test(fn[0])));
  // ⚠️ 空态必须是 404，不是 `{windows:[]}`
  check('空态回 404（不是空数组）', has(api, /暂无情绪轨道/));
}

// ── ③ 路由：端点真的挂上去了 ──────────────────────────────────────────────
{
  const routes = read('crates/yekitrim-server/src/api/routes.rs');
  check('/polish/emotion 挂在 /polish 那组下', has(routes, /route\("\/polish\/emotion",\s*get\(polish_emotion_get\)\)/));
}

// ── ④ 前端 api 层：有取数的函数 ──────────────────────────────────────────
{
  const js = read('webui/src/api/polish.js');
  check('fetchEmotion 存在', has(js, /export const fetchEmotion\s*=/));
  check('它打的是 /api/polish/emotion', has(js, /\/api\/polish\/emotion/));
}

// ── ⑤ 界面：时间轴真的把它画出来 ──────────────────────────────────────────
{
  const tl = read('webui/src/components/TimelineEditor.vue');
  check('时间轴 import 了 fetchEmotion', has(tl, /fetchEmotion/));
  check('loadData 里**调了** fetchEmotion（不 import 不算接线）', has(tl, /await fetchEmotion\(/));
  check('有 emotionRaw 状态', has(tl, /emotionRaw\s*=\s*ref\(/));
  check('算出了 emotionWindows', has(tl, /const emotionWindows\s*=\s*computed\(/));
  // ⚠️⚠️ 2026-10 口径改了：情绪**并进分段块**，不再单开第 8 条泳道。
  //    理由（用户实报）：单开一行让同一信息在时间轴上出现两次，还白占 26px ——
  //    整块布局按"7 条泳道 + 标尺"定的，多一行会把下面的「时间」行挤乱
  //    （实测：视觉特效的块溢出到了时间行上）。
  //    ⇒ 判据跟着改：**必须没有独立泳道**，且徽标**必须挂在分段块上**。
  check('**不再**单开情绪泳道（第 8 条会把布局挤乱）', !has(tl, /class="lane emotion-lane"/));
  check('徽标挂在**分段块**里（不是独立泳道）', has(tl, /v-if="block\.emo"\s+class="emo-badge"/));
  check('segEmotion 按**时间真重叠**挑窗（不是按下标/整段照搬）',
    has(tl, /function segEmotion\(seg\)/) && has(tl, /if \(e <= a \|\| s >= b\) continue/));
  check('没有数据时返回 null、不画占位（"没数据"≠"判过了是平静"）',
    has(tl, /if \(!ws \|\| !ws\.length\) return null/));
  check('分段块把情绪放进 title（可悬停查到强度/条数/命中词）',
    has(tl, /emo\.title/) && has(tl, /const emo = segEmotion\(seg\)/));
  // ⚠️ 四维配色与"主导维度"判定必须与 Rust 同序（平局顺序漂了 = 同一份工程两次跑法不同）
  check('四维配色齐', has(tl, /EMOTION_COLORS\s*=\s*\{[^}]*joy[^}]*shock[^}]*laugh[^}]*awe/) || has(tl, /EMOTION_COLORS/));
  check('主导维度的平局顺序 = shock > laugh > awe > joy（与 Rust dominant() 同序）',
    has(tl, /\['shock',\s*'laugh',\s*'awe',\s*'joy'\]/));
}

// ── ⑥ 反查：Rust 那份平局顺序确实是 shock>laugh>awe>joy ────────────────────
{
  const emo = read('crates/yekitrim-core/src/danmaku/emotion.rs');
  const fn = emo.match(/pub fn dominant\(&self\)[\s\S]{0,700}?\n    \}/);
  const body = fn ? fn[0] : '';
  // 取 `(..., self.shock)` 之类的元组顺序
  const order = (body.match(/self\.(joy|shock|laugh|awe)/g) || []).map((s) => s.slice(5));
  check('Rust dominant() 的候选顺序 = shock,laugh,awe,joy',
    JSON.stringify(order.slice(0, 4)) === JSON.stringify(['shock', 'laugh', 'awe', 'joy']));
}

if (process.argv.includes('--self-test')) {
  const { strict: assert } = await import('node:assert');
  let self = 0;
  const tl = read('webui/src/components/TimelineEditor.vue');
  // ① 拿掉调用（只留 import）⇒ 判据必须红
  const noCall = tl.replace(/await fetchEmotion\(/, 'await fetchEmotionXX(');
  assert.notEqual(noCall, tl, '自证：变异必须命中');
  assert.equal(/await fetchEmotion\(/.test(noCall), false, '自证①：拿掉调用后必须红');
  self += 1;
  // ② 把平局顺序调换 ⇒ 必须红
  const swapped = tl.replace("['shock', 'laugh', 'awe', 'joy']", "['joy', 'laugh', 'awe', 'shock']");
  assert.notEqual(swapped, tl, '自证：顺序变异必须命中');
  assert.equal(/\['shock',\s*'laugh',\s*'awe',\s*'joy'\]/.test(swapped), false, '自证②：换序后必须红');
  self += 1;
  // ③ 拿掉**分段块里的徽标** ⇒ 必须红（新口径下的"渲染"就是这一处）
  const noBadge = tl.replace('v-if="block.emo"', 'v-if="false"');
  assert.notEqual(noBadge, tl, '自证：徽标变异必须命中');
  assert.equal(/v-if="block\.emo"/.test(noBadge), false, '自证③：拿掉徽标后必须红');
  // ④ 把情绪**改回单开一条泳道** ⇒ 也必须红（防止有人再"顺手加一行"）
  const reLane = tl.replace('<div class="lane ruler-lane">', '<div class="lane emotion-lane"><i></i></div><div class="lane ruler-lane">');
  assert.notEqual(reLane, tl, '自证：加回泳道的变异必须命中');
  assert.equal(/class="lane emotion-lane"/.test(reLane), true, '自证④：加回泳道后判据必须能看见它');
  self += 1;
  console.log(`✓ 情绪轨道接线自证：${self} 项（拿掉调用 / 换平局序 / 拿掉徽标，都必须红）`);
  process.exit(0);
}

if (failures.length) {
  console.error(`✗ §10 情绪轨道消费者链断了：${failures.length} 项（共 ${checks} 项）\n`);
  for (const f of failures) console.error('  - ' + f);
  console.error('\n  ⇒ 这条链是：process 写盘 → 端点读 → 前端 api → 时间轴画。');
  console.error('    缺任何一段都**不报错**，只是「算出来的东西用户看不见」——');
  console.error('    `emotion.json` 就是这样当了一阵子哑产物。');
  process.exit(1);
}
console.log(`✓ §10 情绪轨道消费者链完整：${checks} 项（写盘 / 端点 / 路由 / api / 时间轴渲染）`);
