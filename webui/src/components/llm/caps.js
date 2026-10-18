// components/llm/caps.js — **能力（caps）归一化**：把服务端给的形状折成界面唯一认识的那一种。
//
// 为什么单独一个模块：能力表在 Rust 侧（yekitrim_llm::ModelCaps），接口 JSON 由服务端拼；「界面认哪几个
// 键」只在这一个文件里收口，别让每个组件各判一次。
//
// ⚠️ **认不出 = 未知**（保守：只画通用控件），**绝不当成「支持」** —— 画出一个模型其实不认的旋钮，
//    用户调完发出去被 provider 静默忽略 / 400，比「少一个控件」糟得多。
//
// ⚠️ **档位清单与顺序都原样用服务端给的**（levels 对象的键）：不许自己排序、不许补齐 ——
//    「档位下拉的选项数 == levels 的条数」正是判据。这里只给显示名（LEVEL_ZH）。

/** 档位的显示名（认不出的档位原样显示 key，不丢也不猜）。 */
export const LEVEL_ZH = { off: '关思考', minimal: '最小', low: '低', medium: '中', high: '高', xhigh: '极高' }

/** 能力值的来源（界面上写明这个旋钮是哪来的）。目前只有一条：内置能力表。 */
export const CAPS_SOURCE_ZH = { catalog: '能力表' }

/**
 * 档位别名 → 规范档位名。**这是 Rust `ReasoningLevel::from_thinking_flag` 的镜像**（老配置里
 * 的 `[llm] thinking = "enabled"/"disabled"` 就是靠它显示的），写回时用规范名 —— 服务端两套都认。
 * 认不出的值原样返回（不静默改写用户手改出来的东西）。
 */
const LEVEL_ALIAS = {
  enabled: 'high', on: 'high', true: 'high', yes: 'high',
  disabled: 'off', none: 'off', false: 'off', no: 'off',
  minimal: 'minimal', low: 'low', medium: 'medium', mid: 'medium',
  high: 'high', xhigh: 'xhigh', max: 'xhigh'
}

/** 归一一个档位值（'' / null / undefined → ''）。 */
export function normalizeLevel(v) {
  const s = String(v ?? '').trim()
  if (!s) return ''
  return LEVEL_ALIAS[s.toLowerCase()] || s
}

function num(v) {
  const n = Number(v)
  return Number.isFinite(n) && n > 0 ? Math.round(n) : null
}

/**
 * 归一化。**返回 null = 能力未知**（调用方据此只画通用控件）。
 *
 * 认得的形状（容错：caps 包一层或平铺都认）：
 *   levels / reasoning_levels / thinking_levels：{ "off": null, "low": "low" } 或 ["low","high"]；
 *   temperature / supports_temperature：'supported' | 'unsupported' | bool；
 *   max_output / max_output_ceiling / max_tokens_ceiling：number（能力上限，不是每请求默认值）；
 *   json_mode / supports_json_mode：bool（表里目前没有这个维度 ⇒ null = 未知，界面按"通用控件"画）；
 *   hit：bool（这对 (host, model) 命中能力表没有）。
 */
export function normalizeCaps(raw) {
  if (!raw || typeof raw !== 'object') return null
  const c = (raw.caps && typeof raw.caps === 'object') ? raw.caps : raw

  const levelsRaw = c.levels ?? c.reasoning_levels ?? c.thinking_levels ?? null
  let levels = []
  if (Array.isArray(levelsRaw)) levels = levelsRaw.map(String)
  else if (levelsRaw && typeof levelsRaw === 'object') levels = Object.keys(levelsRaw)

  const tempRaw = c.temperature ?? c.supports_temperature ?? null
  const temperature = (tempRaw === false || tempRaw === 'unsupported' || tempRaw === 'no')
    ? 'unsupported'
    : (tempRaw == null ? 'unknown' : 'supported')

  const jsonRaw = c.json_mode ?? c.supports_json_mode ?? null
  const jsonMode = (jsonRaw === false || jsonRaw === 'unsupported')
    ? false
    : (jsonRaw == null ? null : true)

  const hit = (typeof c.hit === 'boolean') ? c.hit : ((typeof raw.hit === 'boolean') ? raw.hit : null)

  return {
    levels,
    temperature,
    jsonMode,
    maxOutput: num(c.max_output ?? c.max_output_ceiling ?? c.max_tokens_ceiling),
    contextWindow: num(c.context_window),
    hit
  }
}

/** 能力是否**已知**（命中能力表）：一个字段都没答上来就是未知。 */
export function capsKnown(caps) {
  if (!caps) return false
  if (caps.hit != null) return caps.hit
  return caps.levels.length > 0 || caps.temperature !== 'unknown'
    || caps.maxOutput != null || caps.jsonMode != null
}
