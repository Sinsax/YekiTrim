#!/usr/bin/env node
// 门禁：**文字元素的纵向档位（`pos`）"预览生效 / 成片不生效"这个落差必须一直写着**。
//
// # 为什么需要它
//
// `elements.text` 有一个用户可见的下拉：纵向位置 = 上 / 中 / 下（`pos`）。
// `process` **真的**按它选落点（`placement.rs::TextAnchor::parse`），
// **预览**也真的按那对 `x`/`y` 摆 —— 所以三档在预览里**看得出差别**。
//
// **但成片一律贴底**：转译器把 ASS 对齐码硬编码成 `2`（底部居中），`pos` 既没进对齐码、
// 也没进底座任何字段。实测根因是**底座没有可用的纵向锚**：
//
//     // dhampir text_layout::layout
//     let margin = style.bottom_margin.clamp(0.0, 0.5);
//     let max_width_px = sequence_width * (1.0 - 2.0 * margin);   // ← 左右安全宽
//     let block_top    = 1.0 - margin - keep * line_height_norm;  // ← 纵向位置
//
// **同一个字段身兼两职**：左右安全宽要它 = 0.0625，把 `top` 抬上去要它 ≈ 0.6 ——
// 抬上去 `max_width` 变负数 ⇒ **一个字都排不出来**（有真实事故：误填 0.5 时
// "每一条文字元素都静默消失"，报告里一行差异都没有）。
//
// # 这条判据盯什么
//
// 不盯"算法对不对"（那有单测），盯的是**"这个落差有没有被如实说出来"**：
//
//   ① 属性面板上必须有那条 `warn-hint`（用户选「上」的时候看得见）；
//   ② 规格文档必须记着这件事（后来的人不能靠读 spec 以为它生效）；
//   ③ 需求文档必须有「T7 纵向锚」这一节（否则这个缺口没人接手）。
//
// ⚠️ **它不会因为"将来接通了"而变绿** —— 接通那天要么删掉这条判据、
//    要么把它翻过来（那时应该断言"成片真的按档位摆"）。**删它之前先读这段。**
//
//     node scripts/check-text-pos-divergence.mjs
//     node scripts/check-text-pos-divergence.mjs --self-test
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const read = (rel) => readFileSync(join(repo, rel), 'utf8');

let checks = 0;
const failures = [];
const check = (name, ok) => {
  checks += 1;
  if (!ok) failures.push(name);
};

const TL = read('webui/src/components/TimelineEditor.vue');
const SPEC = read('docs/plan/elements-spec.md');
const REQ = read('docs/plan/dhampir-elements-request.md');

// ① 属性面板上的如实警示（用户看得见的那一处）
check('属性面板有 warn-hint 样式', /\.warn-hint\s*\{/.test(TL));
check('面板里挂了 warn-hint 的那条提示', /class="hint warn-hint"/.test(TL));
check(
  '提示文案写明"只在预览生效/成片贴底"',
  /纵向档位目前只在预览生效，成片会贴底/.test(TL),
);
check(
  '提示的 title 说清根因（纵向锚与安全宽解耦）',
  /纵向锚/.test(TL) && /安全宽/.test(TL),
);
// ⚠️ 光有提示不够：**提示必须挂在文字支里**（挂在贴纸支上等于没挂）
const textBranch = TL.indexOf('纵向位置');
check('「纵向位置」下拉还在（文字支没被删）', textBranch > 0);
check(
  '警示在下拉之后（同一支里）',
  textBranch > 0 && TL.indexOf('class="hint warn-hint"') > textBranch,
);

// ② 规格文档记着这件事
check('elements-spec 有 pos 更正小节', /`pos` 的更正/.test(SPEC));
check('spec 写明"成片一律贴底"', /成片一律贴底/.test(SPEC));
check('spec 写明硬编码 Alignment 2', /Alignment\s*2/.test(SPEC));
check('spec 指向需求文档的 T7', /T7 纵向锚/.test(SPEC));

// ③ 需求文档有可接手的一节（不然缺口没人管）
check('需求文档有 T7 纵向锚一节', /T7.*纵向锚|纵向锚.*T7/.test(REQ));
check('需求文档给了建议字段名', /anchor_bottom/.test(REQ));
check('需求文档写了验收口径（纵向重心单调）', /纵向重心/.test(REQ));

// ④ 转译器那一头**当前确实是硬编码**（判据的前提还成立）
//
// ⚠️ Style 行在源码里是**跨行拼接**的（模板字面量 + `+` 续一行），
//    所以不能用"匹配整行"的写法（第一版就是那么写、恒不命中）。
//    这里只钉**后半段那个字面量** —— 它含 `Alignment` 那一列。
const TRANS = read('tools/polish-to-dhampir.mjs');
// 字段序见 `cuesToAss` 的 Format 行：
//   Bold,Italic,Underline,StrikeOut,ScaleX,ScaleY,Spacing,Angle,BorderStyle,Outline,Shadow,
//   **Alignment**,MarginL,MarginR,MarginV,Encoding
// ⇒ 后半段里 `…,1,2,1,` 的第 3 个 `2` 就是 Alignment… 实际串是
//   `0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1`
//    0,0,0,0(Bold..StrikeOut) 100,100(ScaleX,ScaleY) 0(Spacing) 0(Angle)
//    1(BorderStyle) 2(Outline) 1(Shadow) **2(Alignment)** 10,10,10(Margins) 1(Encoding)
const ASS_TAIL = "'0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1'";
check('转译器的 ASS Style 后半段还能找到', TRANS.includes(ASS_TAIL));
check(
  '  ↑ 对齐码仍是 2（底部居中）—— 若这里变了，本判据的前提失效，必须重审',
  TRANS.includes(ASS_TAIL),
);
check(
  '转译器没有给 cuesToAss 传过对齐/纵向参数',
  !/cuesToAss\([\s\S]{0,600}?\balign\b/.test(TRANS),
);

if (process.argv.includes('--self-test')) {
  const { strict: assert } = await import('node:assert');
  let self = 0;
  // ① 拿掉那条 warn-hint ⇒ 必须红
  const noWarn = TL.replace('class="hint warn-hint"', 'class="hint"');
  assert.notEqual(noWarn, TL, '自证：变异必须命中');
  assert.equal(/class="hint warn-hint"/.test(noWarn), false, '自证①：拿掉警示后必须红');
  self += 1;
  // ② 样式还在、但类名没挂 ⇒ 也必须红（防"定义了没用"）
  assert.equal(/class="hint warn-hint"/.test(TL.replace(/class="hint warn-hint"/g, '')), false,
    '自证②：只留样式不挂类名必须红');
  self += 1;
  // ③ 文案被改成粉饰版（"会按档位显示"）⇒ 必须红
  const rosy = TL.replace('纵向档位目前只在预览生效，成片会贴底', '纵向位置会按档位显示');
  assert.equal(/纵向档位目前只在预览生效，成片会贴底/.test(rosy), false, '自证③：粉饰文案必须红');
  self += 1;
  // ④ spec 里那节被删 ⇒ 必须红
  const noSpec = SPEC.replace(/`pos` 的更正/, '（删掉了）');
  assert.equal(/`pos` 的更正/.test(noSpec), false, '自证④：删掉 spec 小节必须红');
  self += 1;
  console.log(`✓ 文字纵向档位落差自证：${self} 项（拿警示 / 只留样式 / 粉饰文案 / 删 spec，都必须红）`);
  process.exit(0);
}

if (failures.length) {
  console.error(`✗ 文字纵向档位的"预览成形、成片贴底"落差没写清：${failures.length} 项红（共 ${checks} 项）\n`);
  for (const f of failures) console.error('  - ' + f);
  console.error('\n  ⇒ 这条判据不是防"代码坏了"，是防**"落差被静默抹平"**：');
  console.error('     用户在面板上选「上」，成片却贴底 —— 如果界面与文档都不说，');
  console.error('     他只会以为是自己没设置对，而这是**底座缺字段**造成的。');
  console.error('     根因与建议接口见 docs/plan/dhampir-elements-request.md 的「T7 纵向锚」。');
  process.exit(1);
}
console.log(`✓ 文字纵向档位落差已如实标注：${checks} 项（界面警示 / 规格更正 / 需求 T7 / 转译器前提）`);
