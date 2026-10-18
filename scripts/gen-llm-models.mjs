// @ts-check
/**
 * gen-llm-models.mjs —— 从 @earendil-works/pi-ai 的模型数据里抽出 YekiTrim 自己的
 * 「模型能力表」(crates/yekitrim-llm/data/llm-models.json)。
 *
 * 为什么要有这张表：/v1/models 只回 id，不回能力。要让用户「填了 base_url + api_key
 * 就能挑模型、自动出现推理强度/温度」，能力必须来自随版本发布的静态表。
 *
 * 只抽数据，不引 pi-ai 的运行时；脚本零依赖（node:fs / node:path / node:crypto）。
 *
 * 用法：
 *   node scripts/gen-llm-models.mjs --pi-ai <pi-ai 包目录>            # 重写产物
 *   node scripts/gen-llm-models.mjs --pi-ai <pi-ai 包目录> --check    # 只比对不写
 *   node scripts/gen-llm-models.mjs --pi-ai <目录> --print-analysis   # 额外打印源侧统计
 *
 * 退出码：0 成功/一致；1 --check 不一致；2 缺 --pi-ai 或源目录不可用。
 *
 * 产物逐字节稳定：2 空格缩进、LF、末尾一个换行；providers 按 id 升序、models 按 id 升序。
 */

import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const SCHEMA = 1;
const PACKAGE_NAME = '@earendil-works/pi-ai';
const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const OUT_FILE = path.join(SCRIPT_DIR, '..', 'crates', 'yekitrim-llm', 'data', 'llm-models.json');

/** 产物里允许出现的档位（源里 pi-ai 还有 max，本表不收；理由见 data/NOTICE.md）。 */
const ALLOWED_LEVELS = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh'];

/** 我们自己的 wire 词表。每一项都由 pi-ai api/openai-completions.js 的 createClient() 分支印证。 */
const THINKING_WIRES = [
  'openai_effort',        // 顶层 reasoning_effort=<档位映射值>（pi-ai 的 openai / zai / together / ant-ling 都落到这里）
  'thinking_type',        // thinking:{type:enabled|disabled}（deepseek 系）
  'qwen_enable_thinking', // enable_thinking:bool（qwen）；chat_template_kwargs.enable_thinking 同义
  'openrouter_effort',    // reasoning:{effort:<档位映射值>}
  'none',                 // 源里无 compat 且无法归纳时的兜底（本表当前没有 provider 落到这里）
];
const MAX_OUTPUT_WIRES = ['max_tokens', 'max_completion_tokens'];

/**
 * 选哪些 provider：显式白名单，**多一个不抽、少一个不补**（此表是产品面，不是 pi-ai 的镜像）。
 * pi-ai 0.82.1 的 dist/providers/data 下共 36 个 *.json；本表之外的（云厂商托管 / 聚合网关 /
 * 编程订阅）一律不要，取舍理由见 crates/yekitrim-llm/data/NOTICE.md。
 * skip / keep 是**人工策展的正则**：筛掉的都是与「直播切片」无关的档位（实时语音、图像生成、
 * 日期快照、免费层、以及被同代更新型号取代的老型号）。
 */
const PROVIDERS = [
  { id: 'deepseek', file: 'deepseek.json' },
  { id: 'openai', file: 'openai.json', skip: /realtime|^gpt-4o|^gpt-4-turbo|^gpt-4$|^gpt-3\.5|codex|deep-research|^o1|^o3|^o4-mini|chat-latest|-spark/ },
  { id: 'anthropic', file: 'anthropic.json', skip: /-\d{8}$/ },
  { id: 'google', file: 'google.json', skip: /-\d{2}-\d{4}$|deep-research|computer-use|live-preview|robotics|-image/ },
  { id: 'qwen-token-plan', file: 'qwen-token-plan.json' },
  { id: 'qwen-token-plan-cn', file: 'qwen-token-plan-cn.json' },
  { id: 'moonshotai', file: 'moonshotai.json' },
  { id: 'moonshotai-cn', file: 'moonshotai-cn.json' },
  { id: 'zai', file: 'zai.json' },
  { id: 'zai-coding-cn', file: 'zai-coding-cn.json' },
  { id: 'minimax', file: 'minimax.json' },
  { id: 'minimax-cn', file: 'minimax-cn.json' },
  { id: 'xiaomi', file: 'xiaomi.json', skip: /ultraspeed/ },
  { id: 'groq', file: 'groq.json', skip: /llama-3\.1-8b|llama-3\.3-70b|llama-4-scout/ },
  { id: 'cerebras', file: 'cerebras.json', skip: /gemma/ },
  { id: 'together', file: 'together.json', skip: /Qwen2\.5-7B|gemma-4-31B|Llama-3\.3-70B|nemotron-3-ultra|Inkling/ },
  { id: 'openrouter', file: 'openrouter.json', keep: /^(?:openai\/(?:gpt-5(?:-pro|-mini|-nano|\.\d+(?:-pro|-mini|-nano)?)?|gpt-oss-120b)|anthropic\/claude-(?:opus|sonnet|haiku)-[45](?:\.\d)?|google\/gemini-3\.\d-(?:pro|flash)|deepseek\/deepseek-(?:r1|v3\.2|v4-pro)|qwen\/qwen3\.\d-(?:max|plus|flash)|moonshotai\/kimi-k[23]|z-ai\/glm-(?:4\.7|5)|minimax\/minimax-m3|x-ai\/grok-4\.5)$/ },
];

// ---------------------------------------------------------------- 源侧工具

function die(code, message) {
  console.error(message);
  process.exit(code);
}

function readJsonFile(file) {
  let text;
  try {
    text = fs.readFileSync(file, 'utf8');
  } catch (err) {
    die(2, '[gen-llm-models] 读不到源文件 ' + file + '：' + err.message);
  }
  try {
    return { value: JSON.parse(text), text };
  } catch (err) {
    die(2, '[gen-llm-models] 源 JSON 解析失败 ' + file + '：' + err.message);
  }
}

/** 摊平 {"<api>": {"<model>": {...}}} —— 一家文件里可能有多个 api 桶。 */
function flattenCatalog(raw) {
  const models = [];
  for (const [api, bucket] of Object.entries(raw)) {
    for (const [key, model] of Object.entries(bucket)) {
      models.push({ api, key, model });
    }
  }
  return models;
}

function hostOf(baseUrl) {
  return new URL(baseUrl).host;
}

// --------------------------------------------- 从源数据归纳「该发哪个 wire 字段」
//
// pi-ai 的 api/openai-completions.js::detectCompat() 会在 model.compat 缺项时按 provider 名 /
// baseUrl 推默认值，getCompat() 再让显式 compat 覆盖。我们不引它的运行时，所以把同一套推断
// **抄成纯函数**——否则「源里没写 compat」的模型会被误判成 none。归纳出的 wire 是**该家每个
// 模型自己的**，不是一个全局常量。

function detectThinkingFormat(providerId, baseUrl) {
  const isZai = providerId === 'zai' || providerId === 'zai-coding-cn' ||
    baseUrl.includes('api.z.ai') || baseUrl.includes('open.bigmodel.cn');
  const isTogether = providerId === 'together' || baseUrl.includes('api.together.');
  const isMoonshot = providerId === 'moonshotai' || providerId === 'moonshotai-cn' || baseUrl.includes('api.moonshot.');
  const isOpenRouter = providerId === 'openrouter' || baseUrl.includes('openrouter.ai');
  const isAntLing = providerId === 'ant-ling' || baseUrl.includes('api.ant-ling.com');
  const isDeepSeek = providerId === 'deepseek' || baseUrl.includes('deepseek.com');
  if (isDeepSeek) return 'deepseek';
  if (isZai) return 'zai';
  if (isTogether) return 'together';
  if (isAntLing) return 'ant-ling';
  if (isOpenRouter) return 'openrouter';
  return 'openai';
}

function detectMaxTokensField(providerId, baseUrl) {
  const isMoonshot = providerId === 'moonshotai' || providerId === 'moonshotai-cn' || baseUrl.includes('api.moonshot.');
  const isTogether = providerId === 'together' || baseUrl.includes('api.together.');
  const useMaxTokens = baseUrl.includes('chutes.ai') || isMoonshot || isTogether ||
    baseUrl.includes('api.cloudflare.com') || baseUrl.includes('integrate.api.nvidia.com') ||
    baseUrl.includes('api.ant-ling.com');
  return useMaxTokens ? 'max_tokens' : 'max_completion_tokens';
}

/** pi-ai 的 thinkingFormat 字符串 -> 我们表里的 wire 名。 */
function thinkingFormatToWire(fmt) {
  switch (fmt) {
    case 'deepseek': return 'thinking_type';
    case 'qwen':
    case 'qwen-chat-template':
    case 'chat-template': return 'qwen_enable_thinking';
    case 'openrouter': return 'openrouter_effort';
    case 'openai': return 'openai_effort';
    case 'string-thinking': return 'thinking_type';
    case 'together':
    case 'zai':
    case 'ant-ling': return 'openai_effort'; // 都发顶层 reasoning_effort（zai/together 另带一个开关对象）
    default: return null; // 认不出来的格式：不猜，报错让人裁决
  }
}

function maxTokensFieldToWire(field) {
  if (field === 'max_tokens') return 'max_tokens';
  if (field === 'max_completion_tokens') return 'max_completion_tokens';
  return null;
}

/** 模型级 temperature：源里只有显式信号才算，没有信号就保守保留 supported。 */
function detectTemperatureSignal(compat) {
  for (const key of ['supportsTemperature', 'temperature']) {
    if (typeof compat[key] === 'boolean') return compat[key];
  }
  return null;
}

// ------------------------------------------------------------------ 抽取

function missingSourceMessage(pkgDir) {
  return [
    '[gen-llm-models] 找不到可用的 pi-ai 源包：' + pkgDir,
    '',
    '需要准备什么（三选一）：',
    '  1) 用本机 npm 缓存里已有的那份（只读即可）：',
    '     node scripts/gen-llm-models.mjs --pi-ai <npm-cache>/node_modules/@earendil-works/pi-ai',
    '  2) 现场装一份到临时目录再跑：',
    '     npm i --prefix <tmp> @earendil-works/pi-ai',
    '     node scripts/gen-llm-models.mjs --pi-ai <tmp>/node_modules/@earendil-works/pi-ai',
    '  3) 从 GitHub 取源码包：https://github.com/earendil-works/pi （packages/ai）',
    '',
    '要求：该目录下必须有 package.json（name=@earendil-works/pi-ai，读 version/license）',
    '      与 dist/providers/data/*.json（每家的模型表）。',
    '本脚本不会联网、也不会替你安装依赖。',
  ].join('\n');
}

function extract(piAiDir, analysis) {
  const pkgDir = path.resolve(piAiDir);
  if (!fs.existsSync(pkgDir) || !fs.statSync(pkgDir).isDirectory()) die(2, missingSourceMessage(pkgDir));
  const pkgFile = path.join(pkgDir, 'package.json');
  if (!fs.existsSync(pkgFile)) die(2, missingSourceMessage(pkgDir));
  const dataDir = path.join(pkgDir, 'dist', 'providers', 'data');
  if (!fs.existsSync(dataDir)) die(2, missingSourceMessage(pkgDir));

  const pkg = readJsonFile(pkgFile).value;
  if (pkg.name !== PACKAGE_NAME) {
    die(2, '[gen-llm-models] ' + pkgFile + ' 的 name 是 ' + JSON.stringify(pkg.name) + '，期望 ' + PACKAGE_NAME + '。');
  }

  // source_sha256：参与抽取的源文件（package.json + 用到的 data/*.json）按文件名升序拼接后求哈希。
  const sourceParts = [{ name: 'package.json', text: fs.readFileSync(pkgFile) }];

  const providers = [];
  const observedFormats = new Map(); // pi-ai 原始 thinkingFormat -> 命中数
  const observedMaxFields = new Map();
  const wireOfFormat = new Map();
  const notes = [];

  for (const spec of PROVIDERS) {
    const file = path.join(dataDir, spec.file);
    const { value: raw, text } = readJsonFile(file);
    sourceParts.push({ name: spec.file, text: Buffer.from(text) });

    const picked = flattenCatalog(raw).filter(({ model }) => {
      if (spec.keep && !spec.keep.test(model.id)) return false;
      if (spec.skip && spec.skip.test(model.id)) return false;
      return true;
    });
    if (picked.length === 0) die(2, '[gen-llm-models] ' + spec.file + ' 里没有模型通过筛选，筛子写错了。');

    const seen = new Map(); // id -> model（跨 api 桶去重；同 id 不同定义就报冲突，不替人裁决）
    for (const { model } of picked) {
      const id = model.id;
      const prev = seen.get(id);
      if (prev && JSON.stringify(prev) !== JSON.stringify(model)) {
        die(2, '[gen-llm-models] ' + spec.file + ' 里 id=' + id + ' 在不同 api 桶里定义不一致，人工裁决。');
      }
      seen.set(id, model);
    }

    const hosts = new Set();
    const providerApis = new Set();
    const temperatures = new Set();
    const providerWires = new Set();
    const providerMaxFields = new Set();
    const models = [];

    for (const id of [...seen.keys()].sort()) {
      const m = seen.get(id);
      const compat = m.compat || {};
      const baseUrl = m.baseUrl || null;
      if (baseUrl) hosts.add(hostOf(baseUrl));

      // 显式 compat 优先；缺项时按 pi-ai 的 detectCompat() 规则推。
      const rawFormat = compat.thinkingFormat || detectThinkingFormat(spec.id, baseUrl || '');
      observedFormats.set(rawFormat, (observedFormats.get(rawFormat) || 0) + 1);
      const wire = thinkingFormatToWire(rawFormat);
      if (wire === null) die(2, '[gen-llm-models] 认不出的 thinkingFormat=' + rawFormat + '（' + spec.id + '/' + id + '）。');
      if (!THINKING_WIRES.includes(wire)) die(2, '[gen-llm-models] wire=' + wire + ' 不在 thinking_wires 词表里。');
      wireOfFormat.set(rawFormat, wire);
      providerWires.add(wire);

      const rawField = compat.maxTokensField || detectMaxTokensField(spec.id, baseUrl || '');
      observedMaxFields.set(rawField, (observedMaxFields.get(rawField) || 0) + 1);
      const maxWire = maxTokensFieldToWire(rawField);
      if (maxWire === null) die(2, '[gen-llm-models] 认不出的 maxTokensField=' + JSON.stringify(compat.maxTokensField) + '（' + spec.id + '/' + id + '）。');
      providerMaxFields.add(maxWire);

      const tempSignal = detectTemperatureSignal(compat);
      temperatures.add(tempSignal === null ? 'supported' : (tempSignal ? 'supported' : 'unsupported'));
      if (tempSignal === false) notes.push(spec.id + '/' + id + ' 的 compat 显式声明「不支持温度」');

      let levels = null;
      if (m.reasoning === true && m.thinkingLevelMap && typeof m.thinkingLevelMap === 'object') {
        levels = {};
        for (const level of ALLOWED_LEVELS) {
          if (!(level in m.thinkingLevelMap)) continue;
          const v = m.thinkingLevelMap[level];
          if (v !== null && typeof v !== 'string') {
            die(2, '[gen-llm-models] thinkingLevelMap.' + level + ' 是 ' + JSON.stringify(v) + '，本表只收 string|null。');
          }
          levels[level] = v;
        }
        if (Object.keys(levels).length === 0) levels = null;
      }

      models.push({
        id,
        name: m.name || id,
        context_window: m.contextWindow === undefined ? null : m.contextWindow,
        max_output: m.maxTokens === undefined ? null : m.maxTokens,
        reasoning: m.reasoning === true,
        thinking_levels: levels,
        // 模型级 wire：pi-ai 的 compat 本来就是模型级字段（同一家里 kimi-k3 与其余 9 个 Kimi 不同）。
        // 只给 provider 级数组的话，消费侧无法知道哪支模型该用哪个，只能整体降级成「不发推理参数」。
        thinking_wire: wire,
        max_output_wire: maxWire,
        _host: baseUrl ? hostOf(baseUrl) : null, // 校验用，序列化前删掉
      });
      providerApis.add(m.api || '(unknown)');
    }

    // 一家之内 wire 不统一**不是错误**（pi-ai 的 compat 是模型级字段），但要留痕：消费侧要么按模型看，
    // 要么只认这个并集里最保守的那个。
    if (providerWires.size !== 1) {
      notes.push(spec.id + ' 内部 thinking wire 不统一：' + [...providerWires].sort().join(' / ') + '（模型级差异，消费侧按模型判）');
    }
    if (providerMaxFields.size !== 1) {
      notes.push(spec.id + ' 内部 maxTokensField 不统一：' + [...providerMaxFields].sort().join(' / ') + '（模型级差异）');
    }

    providers.push({
      id: spec.id,
      hosts: [...hosts].sort(),
      base_url: seen.values().next().value.baseUrl || null,
      // 数组：单值时长度 1；一家之内模型级 wire 不同就并列（消费侧要么按模型判，要么取最保守的那个）。
      // 不写 'mixed:xxx' 这种合成字符串——它不在 wire 词表里，消费侧还得再解析一遍。
      thinking_wire: [...providerWires].sort(),
      max_output_wire: [...providerMaxFields].sort(),
      temperature: temperatures.has('unsupported') ? 'unsupported' : 'supported',
      models,
      _apis: [...providerApis].sort(),
    });
  }

  providers.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  for (const p of providers) {
    if (!p.thinking_wire.length || !p.max_output_wire.length) {
      die(2, '[gen-llm-models] provider ' + p.id + ' 归纳不出 wire。');
    }
  }

  // 自校验：产物必须真的"自洽"——每个模型都能在该家 hosts 里找到落点，否则这张表选出来的模型没法发请求。
  const knownHosts = new Set(providers.flatMap((p) => p.hosts));
  for (const p of providers) {
    if (!p.hosts.length) die(2, '[gen-llm-models] provider ' + p.id + ' 一个 host 都没取到（源里缺 baseUrl？）');
    for (const m of p.models) {
      if (!m._host) die(2, '[gen-llm-models] ' + p.id + '/' + m.id + ' 的 baseUrl 取不出 host，人造不出这个模型。');
      if (!knownHosts.has(m._host)) die(2, '[gen-llm-models] ' + p.id + '/' + m.id + ' 的 host ' + m._host + ' 不在产物 hosts 集合里。');
      if (!THINKING_WIRES.includes(m.thinking_wire) || !MAX_OUTPUT_WIRES.includes(m.max_output_wire)) {
        die(2, '[gen-llm-models] ' + p.id + '/' + m.id + ' 的模型级 wire 不在词表里。');
      }
      if (!p.thinking_wire.includes(m.thinking_wire)) die(2, '[gen-llm-models] ' + p.id + '/' + m.id + ' 的 thinking_wire 不在 provider 级数组里。');
      if (!p.max_output_wire.includes(m.max_output_wire)) die(2, '[gen-llm-models] ' + p.id + '/' + m.id + ' 的 max_output_wire 不在 provider 级数组里。');
    }
  }

  const sourceSha = crypto.createHash('sha256');
  for (const part of sourceParts.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))) {
    sourceSha.update(part.name + '\n');
    sourceSha.update(part.text);
    sourceSha.update('\n');
  }

  const doc = {
    schema: SCHEMA,
    generated_from: { package: PACKAGE_NAME, version: pkg.version, license: pkg.license || 'MIT' },
    source_sha256: sourceSha.digest('hex'),
    thinking_wires: THINKING_WIRES,
    max_output_wires: MAX_OUTPUT_WIRES,
    providers,
  };

  analysis.push('源包：' + PACKAGE_NAME + '@' + pkg.version + '  (' + pkgDir + ')');
  analysis.push('参与哈希的源文件（按名升序）：' + sourceParts.map((x) => x.name).sort().join(', '));
  analysis.push('distinct thinkingFormat（源侧原值 -> 本表 wire）：');
  for (const [fmt, n] of [...observedFormats.entries()].sort()) {
    analysis.push('  ' + fmt.padEnd(20) + ' -> ' + (wireOfFormat.get(fmt) || '(未知)') + '   x' + n);
  }
  analysis.push('distinct maxTokensField（源侧原值 -> 本表 wire）：');
  for (const [field, n] of [...observedMaxFields.entries()].sort()) {
    analysis.push('  ' + field.padEnd(22) + ' -> ' + field + '   x' + n);
  }
  const modelTotal = providers.reduce((n, x) => n + x.models.length, 0);
  const reasoningTotal = providers.reduce((n, x) => n + x.models.filter((m) => m.reasoning).length, 0);
  const levelTotal = providers.reduce((n, x) => n + x.models.filter((m) => m.thinking_levels).length, 0);
  analysis.push('provider 数：' + providers.length + '，模型总数：' + modelTotal +
    '，reasoning=true：' + reasoningTotal + '，带 thinking_levels：' + levelTotal);
  analysis.push('模型级 wire 分布（provider -> thinking_wire / max_output_wire）：');
  const wireTally = new Map();
  for (const x of providers) for (const m of x.models) {
    const key = x.id + ' -> ' + m.thinking_wire + ' / ' + m.max_output_wire;
    wireTally.set(key, (wireTally.get(key) || 0) + 1);
  }
  for (const [key, n] of [...wireTally.entries()].sort()) analysis.push('  ' + key + '  x' + n);
  analysis.push('provider 明细（id / hosts / base_url / thinking_wire / max_output_wire / 模型数）：');
  for (const x of providers) {
    analysis.push('  ' + x.id.padEnd(20) + x.hosts.join(',') + '  ' + x.base_url + '  [' + x.thinking_wire.join('|') + ']  [' + x.max_output_wire.join('|') + ']  api=' + x._apis.join('/') + '  n=' + x.models.length);
  }
  if (notes.length) {
    analysis.push('源侧可疑点：');
    for (const n of notes) analysis.push('  ' + n);
  }
  return doc;
}

// ------------------------------------------------------------------- 主流程

/** 序列化：丢掉 _ 前缀的内部字段；2 空格缩进、LF、末尾一个换行。 */
function serialize(doc) {
  const text = JSON.stringify(doc, (key, value) => (key.startsWith('_') ? undefined : value), 2);
  return text + '\n';
}

function main(argv) {
  let piAiDir = null;
  let check = false;
  let printAnalysis = false;
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--pi-ai') piAiDir = argv[++i] || null;
    else if (a.startsWith('--pi-ai=')) piAiDir = a.slice('--pi-ai='.length);
    else if (a === '--check') check = true;
    else if (a === '--print-analysis') printAnalysis = true;
    else die(2, '[gen-llm-models] 不认识参数：' + a + '（用法见文件头注释）');
  }
  if (!piAiDir) {
    die(2, '[gen-llm-models] 缺 --pi-ai <pi-ai 包目录>。\n\n' + missingSourceMessage('<未指定>'));
  }

  const analysis = [];
  const doc = extract(piAiDir, analysis);
  const text = serialize(doc);
  if (printAnalysis) console.log(analysis.join('\n'));

  if (check) {
    if (!fs.existsSync(OUT_FILE)) die(1, '[gen-llm-models] --check：产物不存在 ' + OUT_FILE);
    const onDisk = fs.readFileSync(OUT_FILE, 'utf8');
    if (onDisk !== text) {
      console.error('[gen-llm-models] --check 不一致：' + OUT_FILE);
      console.error('  磁盘 ' + onDisk.length + ' 字节，重新生成 ' + text.length + ' 字节。');
      console.error('  跑一遍不带 --check 的命令重写产物，并检查是不是源包版本变了。');
      process.exit(1);
    }
    console.log('[gen-llm-models] --check 通过：' + OUT_FILE + ' 与源包 ' + doc.generated_from.version + ' 一致（' +
      doc.providers.length + ' 家 / ' + doc.providers.reduce((n, x) => n + x.models.length, 0) + ' 个模型）。');
    return;
  }

  fs.mkdirSync(path.dirname(OUT_FILE), { recursive: true });
  fs.writeFileSync(OUT_FILE, text);
  console.log('[gen-llm-models] 写入 ' + OUT_FILE + '（' + doc.providers.length + ' 家 / ' +
    doc.providers.reduce((n, x) => n + x.models.length, 0) + ' 个模型，源 ' + PACKAGE_NAME + '@' + doc.generated_from.version + '）。');
}

main(process.argv.slice(2));
