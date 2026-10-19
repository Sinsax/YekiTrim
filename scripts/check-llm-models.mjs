// @ts-check
/**
 * check-llm-models.mjs —— **能力表门禁**（零依赖 ESM，进 CI 常驻）。
 *
 * 守的是「crates/yekitrim-llm/data/llm-models.json 与 vendor/pi-ai-models/ 还对不对得上」：
 * ① 自己先做一遍结构校验（版本一致 / 逐模型 wire 齐全 / 排序与词表）；
 * ② 再 spawn gen-llm-models.mjs --check，让它重新抽一遍源数据并逐字节比对产物；
 * ③ 退出码照搬（0 一致、1 不一致、2 源/环境不对）。
 *
 * ⚠️ **缺源要明说 SKIP，不许静默算过**：vendor/pi-ai-models/ 不在（被删/被清）时打一行 SKIP 并 exit 0 ——
 * 本仓既有口径（缺依赖不许静默通过），但也不能反过来把「没装依赖」判成「表烂了」。
 *
 * 用法：
 *   node scripts/check-llm-models.mjs [--pi-ai <源目录>]   # 默认源 = vendor/pi-ai-models/
 *   node scripts/check-llm-models.mjs --self-test           # 自证判据会红（含端到端反例）
 *
 * 退出码：0 通过 / SKIP；1 不一致或结构校验失败；2 环境不齐（生成器或产物不在等）。
 */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.join(SCRIPT_DIR, '..');
const GENERATOR = path.join(SCRIPT_DIR, 'gen-llm-models.mjs');
const DEFAULT_SOURCE = path.join(REPO, 'vendor', 'pi-ai-models');
const ARTIFACT = path.join(REPO, 'crates', 'yekitrim-llm', 'data', 'llm-models.json');
const PACKAGE_NAME = '@earendil-works/pi-ai';
const ALLOWED_LEVELS = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh'];
const TEMPERATURES = ['supported', 'unsupported'];
/** 输入模态词表（顺序即产物里的顺序：text 在前 image 在后）。 */
const INPUTS = ['text', 'image'];

function readJson(file) {
  try {
    return { value: JSON.parse(fs.readFileSync(file, 'utf8')) };
  } catch (err) {
    return { error: err.message };
  }
}

function isSortedAscending(list) {
  for (let i = 1; i < list.length; i++) if (!(list[i - 1] < list[i])) return false;
  return true;
}

/**
 * 纯函数：产物结构校验。--self-test 直接喂变异体进来，所以这里**不许读盘、不许有副作用**。
 * 返回问题清单（空数组 = 绿）。
 */
function validateArtifact(doc, sourceVersion) {
  const bad = [];
  const say = (m) => bad.push(m);
  if (!doc || typeof doc !== 'object') {
    say('产物不是对象');
    return bad;
  }
  if (doc.schema !== 1) say('schema 期望 1，实际 ' + JSON.stringify(doc.schema));
  const from = doc.generated_from || {};
  if (from.package !== PACKAGE_NAME) say('generated_from.package 期望 ' + PACKAGE_NAME + '，实际 ' + JSON.stringify(from.package));
  if (sourceVersion && from.version !== sourceVersion) {
    say('generated_from.version = ' + JSON.stringify(from.version) + ' 与源 package.json 的 ' + JSON.stringify(sourceVersion) + ' 不一致（表没跟着源更新）');
  }
  if (!Array.isArray(doc.thinking_wires) || doc.thinking_wires.length === 0) say('thinking_wires 必须是非空数组');
  if (!Array.isArray(doc.max_output_wires) || doc.max_output_wires.length === 0) say('max_output_wires 必须是非空数组');
  if (typeof doc.source_sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(doc.source_sha256)) say('source_sha256 必须是 64 位小写十六进制');
  const wires = doc.thinking_wires || [];
  const maxWires = doc.max_output_wires || [];
  if (!Array.isArray(doc.providers) || doc.providers.length === 0) {
    say('providers 必须是非空数组');
    return bad;
  }
  const ids = doc.providers.map((x) => x && x.id);
  if (!isSortedAscending(ids)) say('providers 未按 id 升序（产物必须逐字节稳定）');
  if (new Set(ids).size !== ids.length) say('providers 的 id 有重复');
  for (const prov of doc.providers) {
    const tag = (prov && prov.id) || '<无 id>';
    if (!prov || typeof prov.id !== 'string' || !prov.id) { say('有 provider 缺 id'); continue; }
    if (!Array.isArray(prov.hosts) || prov.hosts.length === 0) say(tag + '：hosts 必须是非空数组');
    else if (!isSortedAscending(prov.hosts)) say(tag + '：hosts 未升序');
    if (typeof prov.base_url !== 'string' || !prov.base_url) say(tag + '：base_url 缺失');
    for (const key of ['thinking_wire', 'max_output_wire']) {
      const v = prov[key];
      const vocab = key === 'thinking_wire' ? wires : maxWires;
      if (!Array.isArray(v) || v.length === 0) { say(tag + '：' + key + ' 必须是非空数组'); continue; }
      if (!isSortedAscending(v)) say(tag + '：' + key + ' 未升序');
      for (const w of v) if (!vocab.includes(w)) say(tag + '：' + key + ' 里的 ' + JSON.stringify(w) + ' 不在词表里');
    }
    if (!TEMPERATURES.includes(prov.temperature)) say(tag + '：temperature 期望 supported/unsupported，实际 ' + JSON.stringify(prov.temperature));
    if (!Array.isArray(prov.models) || prov.models.length === 0) { say(tag + '：models 必须是非空数组'); continue; }
    const mids = prov.models.map((m) => m && m.id);
    if (!isSortedAscending(mids)) say(tag + '：models 未按 id 升序');
    if (new Set(mids).size !== mids.length) say(tag + '：models 的 id 有重复');
    for (const m of prov.models) {
      const mtag = tag + '/' + ((m && m.id) || '<无 id>');
      if (typeof m.reasoning !== 'boolean') say(mtag + '：reasoning 必须是布尔');
      if (typeof m.thinking_wire !== 'string' || !wires.includes(m.thinking_wire)) say(mtag + '：thinking_wire 缺失或不在词表里（消费侧就靠这条，缺了整家退化成不发推理参数）');
      else if (!prov.thinking_wire.includes(m.thinking_wire)) say(mtag + '：thinking_wire 不在 provider 级并集里');
      if (typeof m.max_output_wire !== 'string' || !maxWires.includes(m.max_output_wire)) say(mtag + '：max_output_wire 缺失或不在词表里');
      else if (!prov.max_output_wire.includes(m.max_output_wire)) say(mtag + '：max_output_wire 不在 provider 级并集里');
      if (!Array.isArray(m.input) || m.input.length === 0) {
        say(mtag + '：input 必须是非空数组（输入模态；宁少不多 = 至少要有 text）');
      } else {
        for (const k of m.input) {
          if (!INPUTS.includes(k)) say(mtag + '：input 里的 ' + JSON.stringify(k) + ' 不是 ' + INPUTS.join('|') + ' —— 上层会拿它当"能传这种输入"用');
        }
        if (new Set(m.input).size !== m.input.length) say(mtag + '：input 有重复：' + JSON.stringify(m.input));
        const ti = m.input.indexOf('text');
        const ii = m.input.indexOf('image');
        if (ti !== -1 && ii !== -1 && ti > ii) say(mtag + '：input 里 text 必须排在 image 前面（顺序固定产物才逐字节稳定）：' + JSON.stringify(m.input));
      }
      if (m.reasoning === false && m.thinking_levels !== null) say(mtag + '：不推理的模型 thinking_levels 必须是 null');
      if (m.thinking_levels === null) continue;
      if (typeof m.thinking_levels !== 'object') { say(mtag + '：thinking_levels 必须是对象或 null'); continue; }
      for (const [level, val] of Object.entries(m.thinking_levels)) {
        if (!ALLOWED_LEVELS.includes(level)) say(mtag + '：档位 ' + JSON.stringify(level) + ' 不在 ' + ALLOWED_LEVELS.join('|') + ' 里');
        if (val !== null && typeof val !== 'string') say(mtag + '：档位 ' + level + ' 的值必须是 string|null');
      }
    }
  }
  return bad;
}

function countOf(doc) {
  const provs = (doc && doc.providers) || [];
  return { providers: provs.length, models: provs.reduce((n, x) => n + ((x && x.models) || []).length, 0) };
}

/** 源目录在不在（SKIP 判据）。 */
function sourceReady(dir) {
  return fs.existsSync(dir) && fs.existsSync(path.join(dir, 'package.json'));
}

function skipMessage(dir) {
  return [
    '[check-llm-models] SKIP：源数据不在，没法校验能力表。',
    '  缺的是：' + dir,
    '  这不是通过，只是没条件判 —— 恢复办法见 vendor/pi-ai-models/README.md：',
    '    重新取一份 pi-ai 数据补进 vendor/pi-ai-models/（含 17 个 <id>.json 与 package.json），',
    '    然后 node scripts/gen-llm-models.mjs 重算产物；临时验一下也可以 --pi-ai <包目录>。',
    '  正常仓库里 vendor/pi-ai-models/ 是入库的；它不在通常是拷贝/清理漏了这一层。',
  ];
}

function spawnGeneratorCheck(sourceDir) {
  const args = [GENERATOR, '--check'];
  if (sourceDir) args.push('--pi-ai', sourceDir);
  const res = spawnSync(process.execPath, args, { stdio: 'inherit' });
  if (res.error) return { status: 2, error: res.error.message };
  return { status: res.status === null ? 2 : res.status };
}

// ------------------------------------------------------------------ 自检

function selfTest() {
  let failed = 0;
  const ok = (name, cond) => { if (cond) console.log('  PASS  ' + name); else { console.log('  FAIL  ' + name); failed++; } };
  console.log('[check-llm-models] --self-test');

  const real = readJson(ARTIFACT);
  if (real.error) { console.log('  FAIL  产物读不出来：' + real.error); return 1; }
  const src = readJson(path.join(DEFAULT_SOURCE, 'package.json'));
  const srcVersion = src.value && src.value.version;

  // 判据不许空转：先证明它对真表是绿的
  ok('真表通过结构校验（0 问题）', validateArtifact(real.value, srcVersion).length === 0);

  const clone = () => JSON.parse(JSON.stringify(real.value));
  const withMut = (fn) => { const d = clone(); fn(d); return validateArtifact(d, srcVersion); };
  const hits = (bad, needle) => bad.some((m) => m.includes(needle));

  ok('版本与源不一致必须报', hits(withMut((d) => { d.generated_from.version = '0.0.0-fake'; }), 'version'));
  ok('模型缺 thinking_wire 必须报', hits(withMut((d) => { delete d.providers[0].models[0].thinking_wire; }), 'thinking_wire'));
  ok('模型 wire 不在词表必须报', hits(withMut((d) => { d.providers[0].models[0].thinking_wire = 'nope'; }), 'thinking_wire'));
  ok('模型 wire 不在 provider 并集必须报', hits(withMut((d) => { d.providers[0].models[0].thinking_wire = d.providers[0].thinking_wire[0] === 'none' ? 'openai_effort' : 'none'; }), '并集'));
  ok('providers 未排序必须报', hits(withMut((d) => { d.providers.reverse(); }), '升序'));
  ok('providers 空必须报', hits(withMut((d) => { d.providers = []; }), 'providers'));
  ok('temperature 非法值必须报', hits(withMut((d) => { d.providers[0].temperature = 'maybe'; }), 'temperature'));
  ok('不推理却有档位必须报', hits(withMut((d) => { const m = d.providers.flatMap((x) => x.models).find((x) => x.reasoning === false); if (m) m.thinking_levels = { off: null }; }), 'thinking_levels') || !clone().providers.flatMap((x) => x.models).some((x) => x.reasoning === false));
  ok('未知档位必须报', hits(withMut((d) => { const m = d.providers.flatMap((x) => x.models).find((x) => x.thinking_levels); if (m) m.thinking_levels.max = 'max'; }), '档位'));
  ok('source_sha256 形状必须报', hits(withMut((d) => { d.source_sha256 = 'XYZ'; }), 'source_sha256'));
  // 输入模态（2026-10-19 新增）：这条是任务点名的反例 —— `["audio"]` 必须红
  ok('input 含未知模态必须报（["audio"]）', hits(withMut((d) => { d.providers[0].models[0].input = ['audio']; }), 'input'));
  ok('input 空必须报', hits(withMut((d) => { d.providers[0].models[0].input = []; }), 'input'));
  ok('缺 input 必须报', hits(withMut((d) => { delete d.providers[0].models[0].input; }), 'input'));
  ok('input 有重复必须报', hits(withMut((d) => { d.providers[0].models[0].input = ['text', 'text']; }), 'input'));
  ok('input 顺序反了必须报', hits(withMut((d) => { d.providers[0].models[0].input = ['image', 'text']; }), 'input'));

  // 缺源 ⇒ SKIP（exit 0 + 明说）
  const gone = path.join(os.tmpdir(), 'yekitrim-llm-models-definitely-missing');
  ok('缺源时 sourceReady=false（走 SKIP 分支）', sourceReady(gone) === false);
  ok('SKIP 文案里点出缺的目录', skipMessage(gone).join('\n').includes(gone));

  // 端到端反例：源被改一个数字 ⇒ 生成器 --check 必须退 1
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'yekitrim-llm-models-'));
  try {
    for (const f of fs.readdirSync(DEFAULT_SOURCE)) fs.copyFileSync(path.join(DEFAULT_SOURCE, f), path.join(tmp, f));
    const target = path.join(tmp, 'deepseek.json');
    const text = fs.readFileSync(target, 'utf8');
    const mutated = text.replace('"maxTokens":384000', '"maxTokens":123456');
    if (mutated === text) { console.log('  FAIL  端到端反例没改到源（锚点失效，变异等于没做）'); failed++; }
    else {
      fs.writeFileSync(target, mutated);
      console.log('  （下面这两行不一致输出是自检的**预期**噪声）');
      const res = spawnGeneratorCheck(tmp);
      ok('源被改后 --check 退 1', res.status === 1);
    }
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }

  console.log('[check-llm-models] --self-test ' + (failed === 0 ? '全过' : failed + ' 项失败'));
  return failed === 0 ? 0 : 1;
}

// ------------------------------------------------------------------- 主流程

function main(argv) {
  let sourceDir = null;
  let selftest = false;
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--pi-ai') sourceDir = argv[++i] || null;
    else if (a.startsWith('--pi-ai=')) sourceDir = a.slice('--pi-ai='.length);
    else if (a === '--self-test') selftest = true;
    else { console.error('[check-llm-models] 不认识参数：' + a); return 2; }
  }
  if (selftest) return selfTest();

  const dir = sourceDir || DEFAULT_SOURCE;
  if (!sourceReady(dir)) {
    for (const line of skipMessage(dir)) console.log(line);
    return 0;
  }
  if (!fs.existsSync(GENERATOR)) { console.error('[check-llm-models] 缺生成器：' + GENERATOR); return 2; }
  if (!fs.existsSync(ARTIFACT)) { console.error('[check-llm-models] 缺产物：' + ARTIFACT + '（跑 node scripts/gen-llm-models.mjs 生成）'); return 2; }

  const pkg = readJson(path.join(dir, 'package.json'));
  if (pkg.error) { console.error('[check-llm-models] 读不出源 package.json：' + pkg.error); return 2; }
  const doc = readJson(ARTIFACT);
  if (doc.error) { console.error('[check-llm-models] 读不出产物：' + doc.error); return 2; }

  const problems = validateArtifact(doc.value, pkg.value.version);
  const n = countOf(doc.value);
  if (problems.length) {
    console.error('[check-llm-models] 结构校验失败（' + problems.length + ' 条）：');
    for (const m of problems) console.error('  - ' + m);
    return 1;
  }
  console.log('[check-llm-models] 结构校验通过：' + n.providers + ' 家 / ' + n.models + ' 个模型，源版本 ' + pkg.value.version + '。');

  const res = spawnGeneratorCheck(sourceDir);
  if (res.error) { console.error('[check-llm-models] 生成器起不来：' + res.error); return 2; }
  if (res.status === 0) { console.log('[check-llm-models] 通过：产物与源数据逐字节一致。'); return 0; }
  console.error('[check-llm-models] 失败（生成器退出码 ' + res.status + '）：产物与源数据对不上，跑 node scripts/gen-llm-models.mjs 重算并复核 diff。');
  return res.status === 1 ? 1 : 2;
}

process.exit(main(process.argv.slice(2)));
