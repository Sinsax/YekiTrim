<template>
  <div>
    <div class="page-head">
      <div><h1>设置</h1><div class="sub">程序配置 / LLM / 模型管理</div></div>
    </div>

    <div class="section-title row spread">
      <span>程序配置 config.toml</span>
      <span class="row">
        <button class="btn" @click="toggleSource">
          <Icon name="edit" :size="13" /> {{ editSource ? '返回表单' : '编辑源文件' }}
        </button>
        <button class="btn btn-primary" @click="saveGlobal" :disabled="!loaded">
          <Icon name="save" :size="13" /> {{ editSource ? '保存源文件' : '保存配置' }}
        </button>
      </span>
    </div>
    <div v-if="configError" class="error-banner">
      <Icon name="alert" :size="15" />
      <div>
        <b>config.toml 解析失败，整份程序配置被忽略，表单显示的是内建默认值。</b>
        <div class="muted small mt-4">{{ configError }}</div>
        <div class="muted small mt-4">用「编辑源文件」按上面的行号修好后保存即可；保存时会校验语法，坏文件不会被写入。</div>
      </div>
    </div>
    <div v-if="!loaded" class="error-banner">
      <Icon name="alert" :size="15" />
      <div>
        <b>程序配置未加载成功，表单显示的是内建默认值。</b>
        <div class="muted small mt-4">为避免把默认值写回配置文件，保存已禁用；请确认服务端可用后刷新页面重试。</div>
      </div>
    </div>
    <div class="card">
      <template v-if="!editSource">
        <div class="muted small mb-8">
          表单只覆盖常用项，保存时按字段合并进 config.toml，不会动到未列出的配置（如 <code>[correct]</code> 订正 prompt、<code>detection.max_concurrent</code>）；要改这些请点「编辑源文件」。
        </div>
        <div class="form-section">
          <div class="form-section-title">ASR 转写</div>
          <div class="form-grid">
            <div>
              <label>Mode</label>
              <select v-model="form.asr.mode">
                <option value="single">单模型</option>
                <option value="hybrid">双模型</option>
              </select>
              <div class="muted small field-hint">{{ asrModeHint }}</div>
            </div>
            <div><label>并行数 num_workers（0=自动）</label><input v-model.number="form.asr.num_workers" type="number" step="1"></div>
            <div><label>分块时长 chunk_duration（秒）</label><input v-model.number="form.asr.chunk_duration" type="number" step="0.1"></div>
            <div><label>重叠时长 chunk_overlap（秒）</label><input v-model.number="form.asr.chunk_overlap" type="number" step="0.1"></div>
            <div><label>段间隔 segment_gap（秒）</label><input v-model.number="form.asr.segment_gap" type="number" step="0.1"></div>
            <div class="check-field">
              <input id="llmCorrect" v-model="form.asr.llm_correct" type="checkbox">
              <label for="llmCorrect">LLM 订正转录文本（需配置 LLM）</label>
            </div>
          </div>
        </div>

        <div class="form-section">
          <div class="form-section-title">检测 Detection</div>
          <div class="form-grid det-top">
            <div><label>弹幕窗口 danmaku_window_secs（秒）</label><input v-model.number="form.detection.danmaku_window_secs" type="number" step="0.1"></div>
            <div class="span-2"><label>主播名 vtuber_name</label><input v-model="form.detection.vtuber_name" type="text" placeholder="非空时追加到切片标题"></div>
          </div>
          <div class="prompt-pair">
            <div>
              <label>参考标题 reference_titles（每行一条，可含 emoji；检测和封面标题生成都会参考）</label>
              <textarea v-model="referenceTitlesText" rows="8" placeholder="每行一条参考标题"></textarea>
            </div>
            <div>
              <label>检测 system_prompt（留空使用内建默认）</label>
              <textarea v-model="form.detection.system_prompt" rows="8"></textarea>
            </div>
          </div>
          <div class="prompt-solo">
            <label>检测 prompt_template（{segments} 会被替换；留空使用内建默认）</label>
            <textarea v-model="form.detection.prompt_template" rows="10"></textarea>
          </div>
        </div>

        <div class="form-section trio-row">
          <div>
            <div class="form-section-title">剪切 Cutting</div>
            <div class="vertical-form">
              <div><label>输出目录 output_dir</label><input v-model="form.cutting.output_dir" type="text"></div>
              <div><label>静音阈值 silence_noise_db（dB）</label><input v-model.number="form.cutting.silence_noise_db" type="number" step="1"></div>
              <div class="check-field">
                <input id="overwrite" v-model="form.cutting.overwrite" type="checkbox">
                <label for="overwrite">覆盖已有文件 overwrite</label>
              </div>
            </div>
          </div>
          <div>
            <div class="form-section-title">SFX 音效屏蔽</div>
            <div class="vertical-form">
              <label>blacklist（精确 id 或 "tag:xxx" 屏蔽标签族；回车添加）</label>
              <input
                v-model="sfxBlacklistInput"
                type="text"
                placeholder="输入后回车添加"
                @keyup.enter="addSfxBlacklist"
              >
              <div class="tag-list">
                <span v-for="(item, i) in form.sfx.blacklist" :key="item + i" class="tag-chip">
                  {{ item }}
                  <button class="tag-remove" @click="removeSfxBlacklist(i)">×</button>
                </span>
                <span v-if="!form.sfx.blacklist.length" class="muted small">暂无屏蔽项</span>
              </div>
            </div>
          </div>
          <div>
            <div class="form-section-title">Chrome 渲染</div>
            <div class="vertical-form">
              <label>Chrome 可执行文件路径 chrome.path</label>
              <input v-model="form.chrome.path" type="text" placeholder="留空自动检测">
              <div class="muted small">留空时按常见位置自动检测；精修渲染 / 封面生成 / 健康检查均使用该路径。</div>
            </div>
          </div>
        </div>
      </template>

      <template v-else>
        <div class="muted small mb-8">直接编辑 config.toml 原文，保存后立即生效；「返回表单」将放弃未保存的修改。</div>
        <textarea v-model="globalRaw" rows="24" class="mono source-editor"></textarea>
      </template>
    </div>

    <div class="section-title row spread">
      <span>LLM 配置 llm.toml</span>
      <span class="row">
        <button class="btn" @click="toggleLlmSource">
          <Icon name="edit" :size="13" /> {{ editLlmSource ? '返回表单' : '编辑源文件' }}
        </button>
        <button class="btn" @click="testLlm">测试连接</button>
        <button class="btn btn-primary" @click="saveLlm" :disabled="!loaded">
          <Icon name="save" :size="13" /> {{ editLlmSource ? '保存源文件' : '保存 LLM' }}
        </button>
      </span>
    </div>
    <div class="card">
      <template v-if="!editLlmSource">
        <div class="llm-grid">
          <div class="llm-fields">
            <div><label>base_url</label><input v-model="form.llm.base_url" type="text" placeholder="https://api.openai.com/v1"></div>
            <div><label>api_key</label><input v-model="form.llm.api_key" type="text" placeholder="sk-..."></div>
            <div><label>model</label><input v-model="form.llm.model" type="text"></div>
            <div><label>temperature</label><input v-model.number="form.llm.temperature" type="number" step="0.1"></div>
            <div><label>max_tokens</label><input v-model.number="form.llm.max_tokens" type="number" step="1"></div>
          </div>
          <div class="llm-extra">
            <label>extra_params（JSON 文本）</label>
            <textarea v-model="form.llm.extra_params" class="mono" spellcheck="false"></textarea>
          </div>
        </div>
        <div class="muted small mt-8">{{ llmResult }}</div>
      </template>
      <template v-else>
        <div class="muted small mb-8">直接编辑 llm.toml 原文；「返回表单」将放弃未保存的修改。</div>
        <textarea v-model="llmRaw" rows="14" class="mono source-editor"></textarea>
      </template>
    </div>

    <div class="section-title">模型管理</div>
    <div class="card">
      <ModelList :models="models" @changed="loadModels" />
    </div>
  </div>
</template>

<script setup>
import { ref, reactive, computed, onMounted } from 'vue'
import { api } from '../api/client'
import { toast } from '../stores/toast'
import Icon from '../components/Icon.vue'
import ModelList from '../components/ModelList.vue'

const globalRaw = ref('')
const llmRaw = ref('')
const configError = ref('')
const models = ref([])
const llmResult = ref('')
const sfxBlacklistInput = ref('')
const referenceTitlesText = ref('')
const asrModeHint = computed(() => form.asr.mode === 'hybrid' ? 'SenseVoice 打轴 + qwen3 填词' : 'SenseVoice')
const editSource = ref(false)
const editLlmSource = ref(false)
// 只有成功读到 /api/config 才允许保存：否则表单里的内建默认值会覆盖真实配置
const loaded = ref(false)

const form = reactive({
  asr: {
    mode: 'single',
    num_workers: 0,
    chunk_duration: 60,
    chunk_overlap: 10,
    segment_gap: 1,
    llm_correct: false
  },
  detection: {
    danmaku_window_secs: 10,
    vtuber_name: '',
    reference_titles: [],
    system_prompt: '',
    prompt_template: ''
  },
  cutting: {
    output_dir: './output',
    overwrite: false,
    silence_noise_db: -30
  },
  sfx: { blacklist: [] },
  chrome: { path: '' },
  llm: {
    api_key: '',
    base_url: '',
    model: '',
    temperature: 0,
    max_tokens: 65536,
    extra_params: ''
  }
})

// 表单只覆盖常用项：保存时按 section/key 合并进 config.toml，
// 未在表单中暴露的 section（如 [correct] 订正 prompt）与注释都会原样保留。
function buildGlobalPatch() {
  const f = form
  return {
    asr: {
      mode: f.asr.mode || 'single',
      num_workers: Number(f.asr.num_workers) || 0,
      chunk_duration: Number(f.asr.chunk_duration) || 60,
      chunk_overlap: Number(f.asr.chunk_overlap) || 10,
      segment_gap: Number(f.asr.segment_gap) || 1,
      llm_correct: !!f.asr.llm_correct
    },
    detection: {
      danmaku_window_secs: Number(f.detection.danmaku_window_secs) || 10,
      vtuber_name: f.detection.vtuber_name || '',
      reference_titles: f.detection.reference_titles || [],
      system_prompt: f.detection.system_prompt || '',
      prompt_template: f.detection.prompt_template || ''
    },
    cutting: {
      output_dir: f.cutting.output_dir || './output',
      overwrite: !!f.cutting.overwrite,
      silence_noise_db: Number(f.cutting.silence_noise_db ?? -30)
    },
    sfx: { blacklist: f.sfx.blacklist || [] },
    chrome: { path: f.chrome.path || '' }
  }
}

function buildLlmPatch() {
  const f = form.llm
  const llm = {
    base_url: f.base_url || '',
    api_key: f.api_key || '',
    model: f.model || '',
    temperature: Number(f.temperature) || 0,
    max_tokens: Number(f.max_tokens) || 65536
  }
  if (f.extra_params && f.extra_params.trim()) {
    try {
      llm.extra_params = JSON.parse(f.extra_params)
    } catch (e) {
      throw new Error('extra_params 不是合法 JSON，请检查格式')
    }
  }
  return { llm }
}

async function loadConfig() {
  try {
    const d = await api('/api/config')
    globalRaw.value = d.global_raw || ''
    llmRaw.value = d.llm_raw || ''
    // config 解析失败时后端会给出原因，必须显式提示，否则表单只是静默显示默认值
    configError.value = d.config_error || ''
    // 用 global_config（只含 .vtrim/config.toml）而不是 merged config：
    // merged 会被当前工程的 vtrim.toml 覆盖，表单就会和源文件对不上
    const c = d.global_config || d.config || {}
    if (c.asr) Object.assign(form.asr, {
      mode: c.asr.mode || 'single',
      num_workers: c.asr.num_workers ?? 0,
      chunk_duration: c.asr.chunk_duration ?? 60,
      chunk_overlap: c.asr.chunk_overlap ?? 10,
      segment_gap: c.asr.segment_gap ?? 1,
      llm_correct: !!c.asr.llm_correct
    })
    if (c.detection) Object.assign(form.detection, {
      danmaku_window_secs: c.detection.danmaku_window_secs ?? 10,
      vtuber_name: c.detection.vtuber_name || '',
      reference_titles: Array.isArray(c.detection.reference_titles) ? [...c.detection.reference_titles] : [],
      system_prompt: c.detection.system_prompt || '',
      prompt_template: c.detection.prompt_template || ''
    })
    referenceTitlesText.value = (form.detection.reference_titles || []).join('\n')
    if (c.cutting) Object.assign(form.cutting, {
      output_dir: c.cutting.output_dir || './output',
      overwrite: !!c.cutting.overwrite,
      silence_noise_db: c.cutting.silence_noise_db ?? -30
    })
    if (c.sfx && Array.isArray(c.sfx.blacklist)) {
      form.sfx.blacklist = [...c.sfx.blacklist]
    }
    if (c.chrome) {
      form.chrome.path = c.chrome.path || ''
    }
    const l = d.llm_config || {}
    form.llm = {
      api_key: l.api_key || '',
      base_url: l.base_url || '',
      model: l.model || '',
      temperature: l.temperature ?? 0,
      max_tokens: l.max_tokens ?? 65536,
      extra_params: l.extra_params ? JSON.stringify(l.extra_params, null, 2) : ''
    }
    loaded.value = true
  } catch (e) { loaded.value = false }
}

async function loadModels() {
  try {
    const list = await api('/api/models')
    models.value = (list || []).filter(m => m.name !== 'firered-zh')
  } catch (e) {}
}

async function toggleSource() {
  if (!editSource.value) {
    await loadConfig()
    editSource.value = true
  } else {
    editSource.value = false
  }
}

async function toggleLlmSource() {
  if (!editLlmSource.value) {
    await loadConfig()
    editLlmSource.value = true
  } else {
    editLlmSource.value = false
  }
}

async function saveGlobal() {
  if (!loaded.value) { toast('配置未加载成功，已禁止保存', 'err'); return }
  try {
    form.detection.reference_titles = referenceTitlesText.value
      .split('\n')
      .map(s => s.trim())
      .filter(Boolean)
    if (editSource.value) {
      await api('/api/config/global', { method: 'PUT', body: JSON.stringify({ raw: globalRaw.value }) })
      toast('config.toml 已保存')
    } else {
      await api('/api/config/global', { method: 'PUT', body: JSON.stringify({ patch: buildGlobalPatch() }) })
      toast('程序配置已保存（其余未在表单中的配置保持不变）')
    }
    await loadConfig()
    editSource.value = false
  } catch (e) { toast(e.message, 'err') }
}

async function saveLlm() {
  if (!loaded.value) { toast('配置未加载成功，已禁止保存', 'err'); return }
  try {
    if (editLlmSource.value) {
      await api('/api/config/llm', { method: 'PUT', body: JSON.stringify({ raw: llmRaw.value }) })
      toast('llm.toml 已保存')
    } else {
      await api('/api/config/llm', { method: 'PUT', body: JSON.stringify({ patch: buildLlmPatch() }) })
      toast('LLM 配置已保存')
    }
    await loadConfig()
    editLlmSource.value = false
  } catch (e) { toast(e.message, 'err') }
}

function addSfxBlacklist() {
  const value = sfxBlacklistInput.value.trim()
  if (!value) return
  if (!form.sfx.blacklist.includes(value)) {
    form.sfx.blacklist.push(value)
  }
  sfxBlacklistInput.value = ''
}

function removeSfxBlacklist(i) {
  form.sfx.blacklist.splice(i, 1)
}

async function testLlm() {
  llmResult.value = '测试中…'
  try {
    const r = await api('/api/llm/test', { method: 'POST' })
    llmResult.value = '连接成功 ' + r.latency_ms + 'ms'
  } catch (e) {
    llmResult.value = e.message
  }
}

onMounted(() => {
  loadConfig()
  loadModels()
})
</script>

<style scoped>
.llm-grid { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1.3fr); gap:10px 16px; align-items:stretch; }
.llm-fields { display:grid; grid-template-columns:1fr 1fr; gap:8px 14px; align-content:start; }
.llm-extra { display:flex; flex-direction:column; }
.llm-extra textarea { flex:1; min-height:230px; resize:vertical; font-size:12px; line-height:1.6; }
.form-section { padding:10px 0; border-bottom:1px dashed var(--line-soft); }
.form-section:first-child { padding-top:0; }
.form-section:last-child { border-bottom:none; padding-bottom:0; }
.form-section-title { font-weight:700; font-size:13px; margin-bottom:8px; }
.form-grid { display:grid; grid-template-columns:repeat(6, 1fr); gap:12px 14px; align-items:start; }
.span-2 { grid-column:span 2; }
.field-hint { margin-top:4px; }
.span-all { grid-column:1 / -1; }
.prompt-pair { display:grid; grid-template-columns:1fr 1fr; gap:10px 14px; margin-top:10px; }
.prompt-pair textarea, .prompt-solo textarea { min-height:230px; resize:vertical; }
.prompt-solo { margin-top:10px; }
.trio-row { display:grid; grid-template-columns:1fr 1fr 1fr; gap:14px; }
.vertical-form { display:flex; flex-direction:column; gap:6px; }
.tag-list { display:flex; flex-wrap:wrap; gap:6px; margin-top:2px; }
.tag-chip { display:inline-flex; align-items:center; gap:4px; padding:3px 10px; border-radius:999px; background:var(--main-soft); color:var(--main-deep); font-size:11px; }
.tag-remove { border:none; background:none; color:inherit; cursor:pointer; font-size:14px; line-height:1; padding:0; }
.check-field { display:flex; align-items:center; gap:7px; align-self:end; padding:0 0 9px; }
.check-field label { margin:0; cursor:pointer; color:var(--ink); }
.source-editor { font-size:12px; line-height:1.7; }
button:disabled { opacity:.45; cursor:not-allowed; }
@media (max-width: 1200px) {
  .form-grid { grid-template-columns:repeat(3, 1fr); }
  .trio-row { grid-template-columns:1fr; }
}
@media (max-width: 900px) {
  .llm-grid { grid-template-columns:1fr; }
  .llm-extra textarea { min-height:160px; }
  .form-grid { grid-template-columns:repeat(2, 1fr); }
  .span-2 { grid-column:1 / -1; }
  .prompt-pair { grid-template-columns:1fr; }
}
</style>
