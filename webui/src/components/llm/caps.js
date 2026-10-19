// components/llm/caps.js — **能力（caps）归一化 + 覆盖合并**：界面唯一认识的形状。
//
// 为什么单独一个模块：能力表在 Rust 侧（yekitrim_llm::ModelCaps），接口 JSON 由服务端拼；「界面认哪几个
// 键」只在这一个文件里收口，别让每个组件各判一次。
//
// ⚠️ **认不出 = 未知 / 保守默认**，**绝不当成「支持」**：
//    · 整份 caps 认不出 ⇒ 返回 null（调用方只画通用控件）；
//    · `input` 缺失 ⇒ `{ text: true, image: false }`（与 Rust `InputModalities::default()` 同一个默认，
//      也是"不认识的老网关不许被猜成支持图片"那条保守口径）；`declared:false` = 服务端**没说**。
//    · 未知档位 / 未知 wire **原样留着**（不丢、也不猜）。
//
// ⚠️ **档位顺序按规格排**（off→minimal→low→medium→high→xhigh→**max**），认不出的档位按服务端给的原序追加；
//    **只排不改集合** —— 「档位下拉的选项数 == levels 的条数」仍是判据。
// ⚠️ **下拉选项完全由能力面派生**：`levels` 的键就是全部候选，`can_disable:false` / `only_reasoning:true`
//    时把 `off` 摘掉（关不掉就别摆）；**`can_disable` 缺失/null = 未声明 ⇒ 按老行为**（R4）。

/** 档位的显示名（认不出的档位原样显示 key，不丢也不猜）。 */
export const LEVEL_ZH = { off: '关思考', minimal: '最小', low: '低', medium: '中', high: '高', xhigh: '极高', max: '最高' }

/** 规格里的显示顺序（**只排序**：不补齐、不裁剪）。`max` 是那家自描述网关在用的档（2026-10-19 实测）。 */
const LEVEL_ORDER = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max']

/** 输入模态的显示名。 */
export const INPUT_ZH = { text: '文字', image: '图片' }

/** 能力值的来源（界面上写明这个旋钮是哪来的）。 */
export const CAPS_SOURCE_ZH = { catalog: '内置目录', probe: '联网探测', manual: '手填' }

/** 输入模态的缺省（与 Rust `InputModalities::default()` 同值）。 */
export const DEFAULT_INPUT = { text: true, image: false, declared: false }

/** 档位别名 → 规范档位名（Rust `ReasoningLevel::from_thinking_flag` 的镜像）。 */
const LEVEL_ALIAS = {
  enabled: 'high', on: 'high', true: 'high', yes: 'high',
  disabled: 'off', none: 'off', false: 'off', no: 'off',
  minimal: 'minimal', low: 'low', medium: 'medium', mid: 'medium',
  high: 'high', xhigh: 'xhigh'
  // ⚠️ 别再把 `max` 折成 `xhigh`：它现在是**独立档**（那家网关的清单里就有）。
  //    折了 = 界面显示 xhigh、文件里写的是 max，用户看到与生效值不一致。
}

/** 归一一个档位值（'' / null / undefined → ''）。认不出的值原样返回（不静默改写用户手改的东西）。 */
export function normalizeLevel(v) {
  const s = String(v ?? '').trim()
  if (!s) return ''
  return LEVEL_ALIAS[s.toLowerCase()] || s
}

/** 归一输入模态：对象 `{text,image}` / 数组 `["text","image"]` / 缺失 ⇒ 保守默认。 */
export function normalizeInput(raw) {
  if (Array.isArray(raw)) {
    const set = new Set(raw.map((s) => String(s).trim().toLowerCase()))
    const image = set.has('image') || set.has('vision') || set.has('img')
    // 只说了 image 没提 text：文字仍可用（与 Rust InputModalities::parse 同一条口径）
    return { text: set.has('text') || image, image, declared: true }
  }
  if (raw && typeof raw === 'object') {
    const image = raw.image === true || raw.image === 'true' || raw.image === 1
    const t = raw.text
    // 没写 text 键 = 不否认可用文字（每一家 chat/completions 都吃文字）
    const text = (t === undefined || t === null)
      ? true
      : !(t === false || t === 'false' || t === 0 || t === 'unsupported')
    return { text, image, declared: true }
  }
  return Object.assign({}, DEFAULT_INPUT)
}

/** 档位键 → 规格顺序（认不出的按原序追加）。 */
function orderLevels(keys) {
  return LEVEL_ORDER.filter((k) => keys.includes(k)).concat(keys.filter((k) => !LEVEL_ORDER.includes(k)))
}

function num(v) {
  const n = Number(v)
  return Number.isFinite(n) && n > 0 ? Math.round(n) : null
}

function str(v) {
  return (typeof v === 'string' && v.trim()) ? v.trim() : ''
}

/**
 * 归一化。**返回 null = 能力未知**（调用方据此只画通用控件）。
 *
 * 认得的形状（容错：caps 包一层或平铺都认）：
 *   hit：bool（这对 (host, model) 命中能力表没有）；
 *   input / inputs / modalities：{ "text": true, "image": false } 或 ["text","image"]；
 *   levels / reasoning_levels / thinking_levels：{ "off": null, "low": "low" } 或 ["low","high"]；
 *   temperature / supports_temperature：'supported' | 'unsupported' | bool；
 *   max_output / max_output_ceiling / max_tokens_ceiling：number（能力上限，不是每请求默认值）；
 *   json_mode / supports_json_mode：bool（表里目前没有这个维度 ⇒ null = 未知）；
 *   thinking_wire / max_output_wire：方言名（只做展示与手填对照，界面不拿它做判断）。
 */
export function normalizeCaps(raw) {
  if (!raw || typeof raw !== 'object') return null
  const c = (raw.caps && typeof raw.caps === 'object') ? raw.caps : raw

  const levelsRaw = c.levels ?? c.reasoning_levels ?? c.thinking_levels ?? null
  let keys = []
  if (Array.isArray(levelsRaw)) keys = levelsRaw.map(String)
  else if (levelsRaw && typeof levelsRaw === 'object') keys = Object.keys(levelsRaw)

  const tempRaw = c.temperature ?? c.supports_temperature ?? null
  const temperature = (tempRaw === false || tempRaw === 'unsupported' || tempRaw === 'no')
    ? 'unsupported'
    : (tempRaw == null ? 'unknown' : 'supported')

  const jsonRaw = c.json_mode ?? c.supports_json_mode ?? null
  const jsonMode = (jsonRaw === false || jsonRaw === 'unsupported')
    ? false
    : (jsonRaw == null ? null : true)

  const hit = (typeof c.hit === 'boolean') ? c.hit : ((typeof raw.hit === 'boolean') ? raw.hit : null)

  // **关得掉吗**：契约叫 `can_disable`，那家网关自己的字段叫 `can_disable_thinking`（两种都认）。
  // ⚠️ **null = 未声明 ⇒ 按老行为**（表里有没有 off 就照画）—— R4：不能因为缺字段就把"关闭"拿掉。
  const canDisable = (typeof c.can_disable === 'boolean') ? c.can_disable
    : ((typeof c.can_disable_thinking === 'boolean') ? c.can_disable_thinking : null)
  // **只会推理**（没有"不思考"这一档）。缺 = null = 未声明。
  const onlyReasoning = (typeof c.only_reasoning === 'boolean') ? c.only_reasoning : null
  // 服务端声明的**默认档**：只用来把"跟随默认"的落点写清楚，**不拿它改用户的值**。
  const defaultLevel = normalizeLevel(c.default_level ?? c.reasoning_default_effort ?? '')

  return {
    canDisable,
    onlyReasoning,
    defaultLevel,
    levels: orderLevels(keys),
    temperature,
    jsonMode,
    maxOutput: num(c.max_output ?? c.max_output_ceiling ?? c.max_tokens_ceiling),
    contextWindow: num(c.context_window),
    input: normalizeInput(c.input ?? c.inputs ?? c.modalities),
    thinkingWire: str(c.thinking_wire),
    maxOutputWire: str(c.max_output_wire),
    hit
  }
}

/** 这个模型**关不掉**思考吗（`only_reasoning` 或 `can_disable:false`）。
 *  ⚠️ `can_disable === null` / 缺字段 = **未声明 ⇒ 按老行为**（R4）：不算关不掉，照画表里的 off。 */
export function disableBlocked(caps) {
  if (!caps) return false
  return caps.onlyReasoning === true || caps.canDisable === false
}

/** 下拉该画哪些档位：**就是能力面给的那份清单**，只把"关不掉"时的 `off` 摘掉。
 *  ⚠️ 不补齐、不裁剪别的档 —— 「选项数 == levels 条数」仍是判据（关得掉时逐项相等）。 */
export function visibleLevels(caps) {
  if (!caps) return []
  const levels = Array.isArray(caps.levels) ? caps.levels : []
  return disableBlocked(caps) ? levels.filter((l) => l !== 'off') : levels
}

/** 「关不掉」那句灰字的正文（空串 = 不显示）。 */
export function disableReason(caps) {
  if (!caps) return ''
  if (caps.onlyReasoning === true) return '该模型只会推理，关不掉'
  if (caps.canDisable === false) return '能力说关不掉思考（未列 off）'
  return ''
}

/**
 * 用 `over` 里**声明过**的字段覆盖 `base`（探针真拿到的能力覆盖内置表）。
 *
 * ⚠️ 只覆盖"声明过"的：探针只带回 `input` 时，内置表那几档推理**必须留着** ——
 *    否则"能获取当然最好"会变成"获取到一点点就把表里其余全抹了"。
 */
export function mergeCaps(base, over) {
  if (!base) return over || null
  if (!over) return base
  return {
    // 三个新信号各自"声明过才覆盖"：null / 空串 都算没声明（与 levels 同一条纪律）
    canDisable: over.canDisable != null ? over.canDisable : base.canDisable,
    onlyReasoning: over.onlyReasoning != null ? over.onlyReasoning : base.onlyReasoning,
    defaultLevel: over.defaultLevel || base.defaultLevel,
    levels: over.levels.length ? over.levels : base.levels,
    temperature: over.temperature !== 'unknown' ? over.temperature : base.temperature,
    jsonMode: over.jsonMode != null ? over.jsonMode : base.jsonMode,
    maxOutput: over.maxOutput != null ? over.maxOutput : base.maxOutput,
    contextWindow: over.contextWindow != null ? over.contextWindow : base.contextWindow,
    input: over.input.declared ? over.input : base.input,
    thinkingWire: over.thinkingWire || base.thinkingWire,
    maxOutputWire: over.maxOutputWire || base.maxOutputWire,
    hit: over.hit != null ? over.hit : base.hit
  }
}

/** 能力是否**已知**（命中能力表 / 探针答了东西）：一个字段都没答上来就是未知。 */
export function capsKnown(caps) {
  if (!caps) return false
  if (caps.hit != null) return caps.hit
  return caps.levels.length > 0 || caps.temperature !== 'unknown'
    || caps.maxOutput != null || caps.jsonMode != null || caps.input.declared
}

/** 把一份 `input` 变成写进 `[llm.models."<id>"].input` 的数组（顺序固定：text, image）。 */
export function inputToArray(input) {
  return ['text', 'image'].filter((k) => !!(input && input[k]))
}

/** 已勾的模态 → 中文（行内文案 ≤20 字）。 */
export function inputZh(input) {
  const arr = ['text', 'image'].filter((k) => !!(input && input[k]))
  return arr.length ? arr.map((k) => INPUT_ZH[k]).join('、') : '都没勾'
}
