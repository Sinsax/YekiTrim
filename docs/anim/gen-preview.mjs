/**
 * 生成「元素文字效果类别」HTML 预览。
 *
 * ⚠️ 页面里的求值**全部**走 `animcore.mjs`（与权威 Rust 逐值对拍过），
 *    页面自身**不许**再写第二份数学。
 */
import fs from 'node:fs';
import * as A from './anim-core.mjs';

const KINDS = JSON.parse(fs.readFileSync('./anim-kinds.json', 'utf8'));

const KIND_ORDER = ['none', 'tilt', 'shake', 'sway', 'slam', 'shrink_in', 'pop',
  'arc', 'size', 'scatter', 'shrink', 'grow', 'hop', 'vtext'];
const GROUP_OF = {
  none: '不设', tilt: 'A 组 · 整块变换', shake: 'B 组 · 整块动效', sway: 'B 组 · 整块动效',
  slam: 'B 组 · 整块动效', shrink_in: 'B 组 · 整块动效', pop: 'B 组 · 整块动效',
  arc: 'C 组 · 逐字（同时动、幅度不同）', size: 'C 组 · 逐字（同时动、幅度不同）',
  scatter: 'C 组 · 逐字（同时动、幅度不同）', shrink: 'C 组 · 逐字（同时动、幅度不同）',
  grow: 'C 组 · 逐字（同时动、幅度不同）',
  hop: 'D 组 · 逐字错峰（依次动、幅度相同）', vtext: 'E 组 · 竖排（改排版方向）',
};

// 两轴归属：与 ANIM_KINDS 的 `axis` 一致（`both` = 两栏都要填）。
const AXIS_NOTE = {
  none: { zh: '两栏都能选', cls: 'ax-none' },
  style: { zh: '只填「样式」栏', cls: 'ax-style' },
  effect: { zh: '只填「动效」栏', cls: 'ax-effect' },
  both: { zh: '两栏都要填', cls: 'ax-both' },
};

const FPS = 30;                 // 面板按 30fps 播（与工程默认一致）
const DEMO_TEXT = '今晚八点开播';
const CHARS = [...DEMO_TEXT];
const DUR_MS = 2200;            // 一轮演示时长
const LOOP_MS = 2600;           // 含停顿，让"演完不动"那一段也看得见

/** 一帧里某一档的完整状态。**权威求值**都在这里，页面只消费结果。 */
function stateAt(k, localMs) {
  const p = k.preset || {};
  const tSec = localMs / 1000;
  const localFrame = Math.round((localMs / 1000) * FPS);

  // ---- 整块：弹入 ----
  const scaleIn = A.scaleInAt(localMs,
    p.scaleInFrom ?? 1, p.scaleInPeak ?? 1, p.scaleInMs ?? 0);
  // ---- 整块：抖动（相位按 style 档错开，这里用 comment = 3.8）----
  const [shx, shy] = A.textShakePx(tSec, p.shakePx ?? 0, p.shakeHz ?? 18, 3.8);
  // ---- 整块：摇摆（三角波，来自转译器的关键帧展开）----
  const sway = A.swayDegAt(tSec, p.swayDeg ?? 0, p.swayMs ?? 0);
  // ---- 整块：倾斜（样式轴，静态）----
  const tilt = p.tiltDeg ?? 0;
  // ---- 整块：压扁（样式轴，静态）----
  const squashY = p.squashY ?? 1;

  // ---- 逐字 ----
  const perChar = p.perChar ? CHARS.map((_, i) =>
    A.charTransformAt(p.perChar, i, CHARS.length, localFrame)) : null;

  return { scaleIn, shx, shy, sway, tilt, squashY, perChar, vertical: !!p.vertical };
}

const files = [];

// ============ 只读数据区：整块 vs 逐字 的判定（写进页面供核对）============
const isPerChar = (k) => !!k.preset?.perChar;
const perCharCount = KINDS.filter(isPerChar).length;

const grid = [];
for (const g of KIND_ORDER) {
  const ks = KINDS.filter((k) => k.kind === g);
  if (!ks.length) continue;
  grid.push(`<h2 class="grp">${GROUP_OF[g]}<span class="gcount">${ks.length} 档</span></h2>`);
  grid.push('<div class="cards">');
  for (const k of ks) {
    const ax = AXIS_NOTE[k.axis];
    grid.push(`<div class="card" data-ax="${k.axis}">
  <div class="chd">
    <span class="kname">${k.kindLabel}</span>
    <span class="vname">${k.label}</span>
    <span class="axbadge ${ax.cls}">${ax.zh}</span>
  </div>
  <div class="stage"><div class="txt" id="t_${k.value || 'none'}">${CHARS.map((c, i) =>
      `<span class="ch" data-i="${i}">${c}</span>`).join('')}</div></div>
  <div class="meta"><code>${k.value || '(空)'}</code>
    <span class="badge">${isPerChar(k) ? '逐字路径' : '整块路径'}</span></div>
  <pre class="preset">${JSON.stringify(k.preset)}</pre>
</div>`);
  }
  grid.push('</div>');
}

const html = `<!DOCTYPE html>
<html lang="zh-CN"><head><meta charset="utf-8">
<title>元素文字效果类别 · 39 档</title>
<style>
:root{--bg:#14161a;--fg:#e8e6e1;--mut:#9aa0a6;--line:#2a2e35;--acc:#f56b41}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);
  font:14px/1.6 "Microsoft YaHei","Noto Sans SC",system-ui,sans-serif}
header{padding:22px 26px;border-bottom:1px solid var(--line);position:sticky;top:0;
  background:rgba(20,22,26,.96);backdrop-filter:blur(6px);z-index:9}
h1{margin:0 0 6px;font-size:19px}
.sub{color:var(--mut);font-size:12.5px}
.ctrl{margin-top:12px;display:flex;gap:10px;align-items:center;flex-wrap:wrap}
button{background:#22262d;color:var(--fg);border:1px solid var(--line);border-radius:6px;
  padding:6px 13px;cursor:pointer;font-size:13px}
button:hover{border-color:var(--acc)}
button.on{background:var(--acc);border-color:var(--acc);color:#14161a;font-weight:700}
button:disabled{opacity:.35;cursor:not-allowed}
main{padding:18px 26px 60px}
h2.grp{margin:26px 0 12px;font-size:14px;color:var(--acc);letter-spacing:.4px;
  border-left:3px solid var(--acc);padding-left:9px}
h2.grp:first-child{margin-top:6px}
.gcount{color:var(--mut);font-weight:400;font-size:12px;margin-left:9px}
.cards{display:grid;grid-template-columns:repeat(auto-fill,minmax(268px,1fr));gap:13px}
.card{background:#1a1d22;border:1px solid var(--line);border-radius:9px;padding:11px}
.card.hide{display:none}
.chd{display:flex;align-items:baseline;gap:7px;flex-wrap:wrap;margin-bottom:9px}
.kname{font-size:12px;color:var(--mut)}
.kname::after{content:"·";margin-left:6px}
.vname{font-weight:700;font-size:13.5px}
.axbadge{font-size:10.5px;padding:1.5px 6px;border-radius:9px;margin-left:auto;white-space:nowrap}
.ax-none{background:#3a3f47;color:#c9ced6}
.ax-style{background:#1f4d3a;color:#7ee2b0}
.ax-effect{background:#4a3a1c;color:#f0c674}
.ax-both{background:#4a2338;color:#ff9ec4}
.stage{height:104px;background:
  repeating-conic-gradient(#15171b 0 25%,#101216 0 50%) 0 0/16px 16px;
  border-radius:6px;display:flex;align-items:center;justify-content:center;
  overflow:hidden;position:relative}
.txt{font-size:23px;font-weight:700;color:#fff;white-space:nowrap;
  text-shadow:0 0 5px #000,0 0 2px #000;transform-origin:center}
.ch{display:inline-block;transform-origin:center}
.meta{display:flex;align-items:center;gap:8px;margin-top:8px;font-size:11.5px;color:var(--mut)}
.badge{background:#22262d;border-radius:8px;padding:1px 7px;font-size:10.5px}
code{background:#22262d;padding:1.5px 6px;border-radius:4px;font-size:11px;color:#c9ced6}
.preset{margin:7px 0 0;font-size:10.5px;color:#7d848c;white-space:pre-wrap;
  word-break:break-all;max-height:74px;overflow:auto}
.note{background:#1a1d22;border:1px solid var(--line);border-left:3px solid var(--acc);
  border-radius:7px;padding:12px 15px;margin:14px 0;font-size:12.5px;color:var(--mut)}
.note b{color:var(--fg)}
.note code{font-size:11px}
</style></head><body>
<header>
  <h1>元素文字效果类别 —— ${KINDS.length - 1} 档全览</h1>
  <div class="sub">求值内核与 Dhampir 权威 Rust 逐值对拍（<code>text_perchar_anim.rs</code> ·
    <code>text_motion.rs</code> · <code>easing.rs</code>）；摇摆来自转译器的关键帧展开。</div>
  <div class="ctrl">
    <button id="play" class="on">⏸ 暂停</button>
    <button id="slow">慢速 ×0.35</button>
    <button id="loop">循环</button>
    <span class="sub" id="clock">t = 0.00s</span>
  </div>
</header>
<main>
  <div class="note">
    <b>每条演示的都是「这一档单独作用」</b>：同一句
    <code>${DEMO_TEXT}</code>（${CHARS.length} 字），30fps，弹入从条首起算。
    ⚠️ 实际是<b>两轴叠加</b>的 —— 每张卡的角标写清它该填哪一栏：
    <span class="axbadge ax-style">只填样式栏</span>
    <span class="axbadge ax-effect">只填动效栏</span>
    <span class="axbadge ax-both">两栏都要填</span>
    （<code>both</code> 的档两边各给一半，<b>只填一栏只生效一半</b>）。
  </div>
  ${grid.join('\n')}
</main>
<script>
// ⚠️ 求值全部走随附的 animcore（已与 Rust 对拍），这里只做 DOM 落地。
const KINDS = ${JSON.stringify(KINDS)};
const CHARS = ${JSON.stringify(CHARS)};
const FPS = ${FPS}, DUR = ${DUR_MS}, LOOP = ${LOOP_MS};

// ---- 与 animcore.mjs 逐式相同的内联副本（页面必须能单文件打开）----
${fs.readFileSync('./anim-core.mjs', 'utf8')
    .replace(/^export /gm, '')}

function stateAt(k, localMs){
  const p = k.preset || {};
  const tSec = localMs / 1000;
  const localFrame = Math.round((localMs/1000)*FPS);
  const scaleIn = scaleInAt(localMs, p.scaleInFrom ?? 1, p.scaleInPeak ?? 1, p.scaleInMs ?? 0);
  const [shx, shy] = textShakePx(tSec, p.shakePx ?? 0, p.shakeHz ?? 18, 3.8);
  const sway = swayDegAt(tSec, p.swayDeg ?? 0, p.swayMs ?? 0);
  const perChar = p.perChar ? CHARS.map((_, i) =>
    charTransformAt(p.perChar, i, CHARS.length, localFrame)) : null;
  return { scaleIn, shx, shy, sway, tilt: p.tiltDeg ?? 0, squashY: p.squashY ?? 1,
           perChar, vertical: !!p.vertical };
}

const nodes = KINDS.map(k => byId('t_' + (k.value || 'none')));
function byId(id){ return document.getElementById(id); }

// ⚠️ \`?t=毫秒\` 强制定格在某一时刻（**截图与逐值核对用**）——
//    纯 rAF 在无头截图里不推进，靠 \`--virtual-time-budget\` 截到的是同一帧。
const FORCE = (() => { const m = location.search.match(/[?&]t=(-?[\d.]+)/); return m ? parseFloat(m[1]) : null; })();
let t = FORCE ?? 0, playing = FORCE === null, rate = 1, looping = true, last = performance.now();
const clock = document.getElementById('clock');

function paint(){
  for (let n = 0; n < KINDS.length; n++){
    const k = KINDS[n], node = nodes[n];
    if (!node) continue;
    // 每张卡都从它自己的条首起算（= 一条 cue 的 local time）
    const s = stateAt(k, Math.min(t, DUR));
    const chars = node.children;
    // 整块变换：缩放 × 抖动位移 × 摇摆旋转；倾斜/压扁是**样式轴**的静态量
    node.style.transform =
      'translate(' + s.shx.toFixed(2) + 'px,' + s.shy.toFixed(2) + 'px) '
      + 'rotate(' + (s.sway + s.tilt).toFixed(2) + 'deg) '
      + 'scale(' + s.scaleIn.toFixed(4) + ',' + (s.scaleIn * s.squashY).toFixed(4) + ')';
    node.style.writingMode = s.vertical ? 'vertical-rl' : '';
    node.style.textOrientation = s.vertical ? 'upright' : '';
    if (s.perChar){
      const fs = 23; // px，与 CSS 的 font-size 同值
      for (let i = 0; i < chars.length; i++){
        const c = s.perChar[i];
        chars[i].style.transform =
          'translate(' + (c.offsetXEm*fs).toFixed(2) + 'px,'
          + (c.offsetYEm*fs).toFixed(2) + 'px) '
          + 'rotate(' + c.rotationDeg.toFixed(2) + 'deg) '
          + 'scale(' + c.scale.toFixed(4) + ')';
      }
    } else {
      for (const ch of chars) ch.style.transform = '';
    }
  }
}

function tick(now){
  const dt = Math.min(120, now - last); last = now;
  if (FORCE !== null){ t = FORCE; } else if (playing){ t += dt * rate; if (t > LOOP){ t = looping ? 0 : LOOP; } }
  clock.textContent = 't = ' + (t/1000).toFixed(2) + 's'
    + (t >= DUR ? '（演完了，之后不再动）' : '');
  paint();
  requestAnimationFrame(tick);
}

document.getElementById('play').onclick = function(){
  playing = !playing; this.textContent = playing ? '⏸ 暂停' : '▶ 播放';
  this.classList.toggle('on', playing);
};
document.getElementById('slow').onclick = function(){
  rate = rate === 1 ? 0.35 : 1; this.textContent = rate === 1 ? '慢速 ×0.35' : '常速 ×1';
  this.classList.toggle('on', rate !== 1);
};
document.getElementById('loop').onclick = function(){
  looping = !looping; this.classList.toggle('on', looping);
  this.textContent = looping ? '循环' : '播完停';
};
document.getElementById('loop').classList.add('on');
requestAnimationFrame(tick);
</script></body></html>`;

fs.writeFileSync(process.argv[2], html, 'utf8');
console.log('写出:', process.argv[2]);
console.log('档数:', KINDS.length - 1, '| 逐字路径:', perCharCount, '| 整块路径:', KINDS.length - 1 - perCharCount);
