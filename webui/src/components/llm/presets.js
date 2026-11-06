// components/llm/presets.js — 常见 LLM 服务商的**静态填表预设**。
//
// ⚠️ 这**不是第二张能力表**：这里只有「地址 + 模型提示」，用来省打字。
//    模型能不能用、支持哪些旋钮（温度 / 推理档位 / 输入模态）一律问后端：
//    `GET /api/llm/models`（清单）与 `GET /api/config/llm/catalog`（能力），别在这里抄值。
//    `model` 是**提示**：用户可手打任何 id；预设里的模型也可能改名/下线，认不出就以用户填的为准。
//
// `id` 只用于界面选择与 provider id 生成；`custom` 永远在最后（base_url/model 都空 = 不动当前值）。
export const LLM_PRESETS = [
  {
    id: 'deepseek',
    label: 'DeepSeek',
    base_url: 'https://api.deepseek.com/v1',
    model: 'deepseek-chat'
  },
  {
    id: 'openai',
    label: 'OpenAI',
    base_url: 'https://api.openai.com/v1',
    model: 'gpt-4o-mini'
  },
  {
    id: 'moonshot',
    label: 'Moonshot',
    base_url: 'https://api.moonshot.cn/v1',
    model: 'moonshot-v1-8k'
  },
  {
    id: 'zhipu',
    label: '智谱',
    base_url: 'https://open.bigmodel.cn/api/paas/v4',
    model: 'glm-4-flash'
  },
  {
    id: 'qwen',
    label: '通义',
    base_url: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
    model: 'qwen-plus'
  },
  {
    id: 'ollama',
    label: 'Ollama',
    // Ollama 的 OpenAI 兼容层；本机部署不需要密钥
    base_url: 'http://127.0.0.1:11434/v1',
    model: 'qwen2.5:7b'
  },
  {
    id: 'litellm',
    label: 'LiteLLM',
    base_url: 'http://127.0.0.1:4000/v1',
    model: ''
  },
  {
    id: 'custom',
    label: '自定义',
    base_url: '',
    model: ''
  }
]

/** 按当前地址反查预设（用于回填「服务商」；匹配不上 = 自定义）。 */
export function presetForUrl(url) {
  const norm = (s) => String(s || '').trim().replace(/\/+$/, '').toLowerCase()
  const want = norm(url)
  if (!want) return 'custom'
  const hit = LLM_PRESETS.find((p) => p.base_url && norm(p.base_url) === want)
  return hit ? hit.id : 'custom'
}

/** id → 预设；认不出回落「自定义」（永远有返回值，调用方不用判空）。 */
export function getPreset(id) {
  return LLM_PRESETS.find((p) => p.id === id) || LLM_PRESETS[LLM_PRESETS.length - 1]
}
