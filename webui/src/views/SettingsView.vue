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
        <button class="btn btn-primary" @click="saveGlobal">
          <Icon name="save" :size="13" /> {{ editSource ? '保存源文件' : '保存配置' }}
        </button>
      </span>
    </div>
    <div class="card">
      <template v-if="!editSource">
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
          <div class="grid-2">
            <div><label>弹幕窗口 danmaku_window_secs（秒）</label><input v-model.number="form.detection.danmaku_window_secs" type="number" step="0.1"></div>
            <div><label>主播名 vtuber_name</label><input v-model="form.detection.vtuber_name" type="text" placeholder="非空时追加到切片标题"></div>
          </div>
          <div class="prompt-pair">
            <div>
              <label>检测 system_prompt（留空使用内建默认）</label>
              <textarea v-model="form.detection.system_prompt" rows="5"></textarea>
            </div>
            <div>
              <label>检测 prompt_template（{segments} 会被替换；留空使用内建默认）</label>
              <textarea v-model="form.detection.prompt_template" rows="5"></textarea>
            </div>
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
        <button class="btn btn-primary" @click="saveLlm">
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
const models = ref([])
const llmResult = ref('')
const sfxBlacklistInput = ref('')
const asrModeHint = computed(() => form.asr.mode === 'hybrid' ? 'SenseVoice 打轴 + qwen3 填词' : 'SenseVoice')
const editSource = ref(false)
const editLlmSource = ref(false)

const form = reactive({
  asr: {
    model: 'sensevoice',
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

function tomlString(v) {
  if (typeof v === 'string') return JSON.stringify(v)
  if (typeof v === 'number') return String(v)
  if (typeof v === 'boolean') return v ? 'true' : 'false'
  return String(v)
}

function buildGlobalToml() {
  const f = form
  let s = ''
  s += '[asr]\n'
  s += `mode = ${tomlString(f.asr.mode || 'single')}\n`
  s += `num_workers = ${f.asr.num_workers ?? 0}\n`
  s += `chunk_duration = ${f.asr.chunk_duration ?? 60}\n`
  s += `chunk_overlap = ${f.asr.chunk_overlap ?? 10}\n`
  s += `segment_gap = ${f.asr.segment_gap ?? 1}\n`
  s += `llm_correct = ${f.asr.llm_correct ? 'true' : 'false'}\n`
  s += '\n[detection]\n'
  s += `danmaku_window_secs = ${f.detection.danmaku_window_secs ?? 10}\n`
  s += `vtuber_name = ${tomlString(f.detection.vtuber_name || '')}\n`
  s += `system_prompt = ${tomlString(f.detection.system_prompt || '')}\n`
  s += `prompt_template = ${tomlString(f.detection.prompt_template || '')}\n`
  s += '\n[cutting]\n'
  s += `output_dir = ${tomlString(f.cutting.output_dir || './output')}\n`
  s += `overwrite = ${f.cutting.overwrite ? 'true' : 'false'}\n`
  s += `silence_noise_db = ${f.cutting.silence_noise_db ?? -30}\n`
  s += '\n[sfx]\n'
  s += `blacklist = ${JSON.stringify(form.sfx.blacklist || [])}\n`
  s += '\n[chrome]\n'
  s += `path = ${tomlString(f.chrome.path || '')}\n`
  return s
}

function buildLlmToml() {
  const f = form.llm
  let s = '[llm]\n'
  s += `base_url = ${tomlString(f.base_url || '')}\n`
  s += `api_key = ${tomlString(f.api_key || '')}\n`
  s += `model = ${tomlString(f.model || '')}\n`
  s += `temperature = ${f.temperature ?? 0}\n`
  s += `max_tokens = ${f.max_tokens ?? 65536}\n`
  if (f.extra_params && f.extra_params.trim()) {
    s += `extra_params = ${f.extra_params.trim()}\n`
  }
  return s
}

async function loadConfig() {
  try {
    const d = await api('/api/config')
    globalRaw.value = d.global_raw || ''
    llmRaw.value = d.llm_raw || ''
    const c = d.config || {}
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
      system_prompt: c.detection.system_prompt || '',
      prompt_template: c.detection.prompt_template || ''
    })
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
  } catch (e) {}
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
  try {
    if (editSource.value) {
      await api('/api/config/global', { method: 'PUT', body: JSON.stringify({ raw: globalRaw.value }) })
      toast('config.toml 已保存')
    } else {
      await api('/api/config/global', { method: 'PUT', body: JSON.stringify({ raw: buildGlobalToml() }) })
      toast('程序配置已保存')
    }
    await loadConfig()
    editSource.value = false
  } catch (e) { toast(e.message, 'err') }
}

async function saveLlm() {
  try {
    if (editLlmSource.value) {
      await api('/api/config/llm', { method: 'PUT', body: JSON.stringify({ raw: llmRaw.value }) })
      toast('llm.toml 已保存')
    } else {
      await api('/api/config/llm', { method: 'PUT', body: JSON.stringify({ raw: buildLlmToml() }) })
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
.llm-fields { display:flex; flex-direction:column; gap:8px; }
.llm-extra { display:flex; flex-direction:column; }
.llm-extra textarea { flex:1; min-height:230px; resize:vertical; font-size:12px; line-height:1.6; }
.form-section { padding:10px 0; border-bottom:1px dashed var(--line-soft); }
.form-section:first-child { padding-top:0; }
.form-section:last-child { border-bottom:none; padding-bottom:0; }
.form-section-title { font-weight:700; font-size:13px; margin-bottom:8px; }
.form-grid { display:grid; grid-template-columns:repeat(auto-fit, minmax(220px, 1fr)); gap:10px 14px; align-items:start; }
.grid-2 { display:grid; grid-template-columns:1fr 1fr; gap:10px 14px; }
.field-hint { margin-top:4px; }
.span-all { grid-column:1 / -1; }
.prompt-pair { display:grid; grid-template-columns:1fr 1fr; gap:10px 14px; margin-top:10px; }
.prompt-pair textarea { min-height:120px; resize:vertical; }
.trio-row { display:grid; grid-template-columns:1fr 1fr 1fr; gap:14px; }
.vertical-form { display:flex; flex-direction:column; gap:6px; }
.tag-list { display:flex; flex-wrap:wrap; gap:6px; margin-top:2px; }
.tag-chip { display:inline-flex; align-items:center; gap:4px; padding:3px 10px; border-radius:999px; background:var(--main-soft); color:var(--main-deep); font-size:11px; }
.tag-remove { border:none; background:none; color:inherit; cursor:pointer; font-size:14px; line-height:1; padding:0; }
.check-field { display:flex; align-items:center; gap:7px; align-self:flex-start; padding:2px 0 6px; }
.check-field label { margin:0; cursor:pointer; color:var(--ink); }
.source-editor { font-size:12px; line-height:1.7; }
@media (max-width: 900px) {
  .llm-grid { grid-template-columns:1fr; }
  .llm-extra textarea { min-height:160px; }
}
</style>
