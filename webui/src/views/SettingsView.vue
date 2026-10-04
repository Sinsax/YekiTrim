<template>
  <div>
    <div class="page-head">
      <div><h1>设置</h1><div class="sub">{{ active.sub }}</div></div>
    </div>

    <!-- 子页标签：每项是一条路由（settings-<key>），切页即换 URL，可直接刷新/分享。
         标签与路由同源（utils.js 的 SETTINGS_SECTIONS），别在别处再抄一份。 -->
    <nav class="tabs">
      <router-link v-for="s in visibleSections" :key="s.key" :to="{ name: 'settings-' + s.key }"
                   class="tab" :class="{ on: s.key === section }">{{ s.label }}</router-link>
    </nav>

    <!-- 程序配置：ASR / 检测 / 剪切 / Chrome（写入 .yekitrim/config.toml） -->
    <template v-if="section === 'config'">
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
        <b>config.toml 解析失败，配置已忽略</b>
        <div class="muted small mt-4">{{ configError }}</div>
        <div class="muted small mt-4">按上面的行号改好再保存（会校验语法）</div>
      </div>
    </div>
    <div v-if="!loaded" class="error-banner">
      <Icon name="alert" :size="15" />
      <div>
        <b>配置未加载，显示的是默认值</b>
        <div class="muted small mt-4">保存已禁用，避免写回默认值；刷新重试</div>
      </div>
    </div>
    <div class="card">
      <template v-if="!editSource">
        <div class="muted small mb-8">只放常用项，其余在「高级参数」里</div>

        <!-- ① 转写 -->
        <div class="form-section">
          <div class="form-section-title">转写</div>
          <div class="form-grid">
            <div class="mode-field span-2">
              <label for="asrMode">模式</label>
              <select id="asrMode" v-model="form.asr.mode">
                <option value="single">单模型</option>
                <option value="hybrid">双模型</option>
              </select>
              <span class="muted small mode-hint">{{ asrModeHint }}</span>
            </div>
            <div class="check-field span-2">
              <input id="vadEnabled" v-model="form.asr.vad" type="checkbox">
              <label for="vadEnabled" title="single 模式快 2~3 倍，hybrid 不受影响">VAD 只解码语音段</label>
            </div>
            <div class="check-field span-2">
              <input id="llmCorrect" v-model="form.asr.llm_correct" type="checkbox">
              <label for="llmCorrect">LLM 订正转录</label>
            </div>
          </div>
        </div>

        <!-- ② 检测 -->
        <div class="form-section">
          <div class="form-section-title">检测</div>
          <div class="form-grid">
            <div class="span-2">
              <label>主播名</label>
              <input v-model="form.detection.vtuber_name" type="text" placeholder="追加到切片标题">
            </div>
            <SliderField v-model="form.detection.llm_score_weight" class="span-4"
                         label="LLM 评分权重" :min="0" :max="1" :step="0.05"
                         hint="0 = 只用弹幕/字幕热度" />
          </div>
        </div>

        <!-- ③ 输出 -->
        <div class="form-section">
          <div class="form-section-title">输出</div>
          <div class="form-grid">
            <div class="span-4">
              <label>输出目录</label>
              <input v-model="form.cutting.output_dir" type="text">
            </div>
            <div class="check-field span-2">
              <input id="overwrite" v-model="form.cutting.overwrite" type="checkbox">
              <label for="overwrite">覆盖已有文件</label>
            </div>
          </div>
        </div>

        <!-- ④ 高级：提示词 + 全部调参。默认收起 —— 这些是"默认值就是最佳实践"的旋钮，
             摊开只会把常用项淹掉（用户口径："死配置或者最佳配置就隐藏吧"）。 -->
        <details class="form-adv">
          <summary>高级参数<span class="muted small">提示词 / 断句 / 调参</span></summary>

          <div class="form-section">
            <div class="form-section-title">提示词</div>
            <div class="prompt-pair">
              <div>
                <label title="每行一条，可含 emoji；留空用内建示例">参考标题</label>
                <textarea v-model="referenceTitlesText" rows="8" placeholder="每行一条参考标题"></textarea>
              </div>
              <div>
                <label>检测 system_prompt</label>
                <textarea v-model="form.detection.system_prompt" rows="8" placeholder="留空使用内建默认"></textarea>
              </div>
            </div>
            <div class="mt-8">
              <label>检测 prompt_template<span class="muted small">（{segments} 会被替换）</span></label>
              <textarea v-model="form.detection.prompt_template" rows="12" placeholder="留空使用内建默认"></textarea>
            </div>
          </div>

          <div class="form-section">
            <div class="form-section-title">断句与分块<span class="muted small">默认值已按实测调好</span></div>
            <div class="form-grid">
              <SliderField v-model="form.asr.chunk_duration" class="span-2" label="分块时长" unit="s"
                           :min="20" :max="300" :step="10" />
              <SliderField v-model="form.asr.chunk_overlap" class="span-2" label="块间重叠" unit="s"
                           :min="0" :max="60" :step="1" />
              <SliderField v-model="form.asr.segment_gap" class="span-2" label="强停顿阈值" unit="s"
                           :min="0.3" :max="5" :step="0.1" />
              <SliderField v-model="form.asr.breath_gap" class="span-2" label="气口" unit="s"
                           :min="0.1" :max="1" :step="0.05" />
              <SliderField v-model="form.asr.soft_cut_secs" class="span-2" label="标点软切" unit="s"
                           :min="1" :max="20" :step="0.5" />
              <SliderField v-model="form.asr.max_segment_chars" class="span-2" label="字数硬上限"
                           :min="10" :max="80" :step="1" />
              <SliderField v-model="form.asr.snap_window" class="span-2" label="吸附半径" unit="s"
                           :min="0" :max="2" :step="0.05" />
              <SliderField v-model="form.asr.num_workers" class="span-2" label="并行数" hint="0 = 自动"
                           :min="0" :max="32" :step="1" />
            </div>
          </div>

          <div class="form-section">
            <div class="form-section-title">VAD 与解码<span class="muted small">默认值已按实测调好</span></div>
            <div class="form-grid">
              <SliderField v-model="form.asr.vad_threshold" class="span-2" label="VAD 阈值"
                           :min="0" :max="1" :step="0.05" hint="调高 = 多解码、少省时间" />
              <SliderField v-model="form.asr.vad_rescue_db" class="span-2" label="电平兜底" unit="dB"
                           :min="0" :max="40" :step="1" hint="0 = 关" />
              <SliderField v-model="form.asr.max_new_tokens" class="span-2" label="解码上限" hint="token"
                           :min="128" :max="2048" :step="64" />
              <SliderField v-model="form.asr.segment_padding" class="span-2" label="解码余量" unit="s"
                           :min="0" :max="1" :step="0.05" hint="实测非 0 会扰动文本" />
              <SliderField v-model="form.detection.danmaku_window_secs" class="span-2" label="弹幕窗口" unit="s"
                           :min="1" :max="60" :step="1" />
            </div>
          </div>
        </details>
      </template>

      <template v-else>
        <div class="muted small mb-8" title="保存后立即生效；「返回表单」会放弃未保存的修改">直接编辑 config.toml 原文</div>
        <textarea v-model="globalRaw" rows="24" class="mono source-editor"></textarea>
      </template>
    </div>
    </template>

    <!-- LLM 配置：写入 .yekitrim/llm.toml -->
    <template v-else-if="section === 'llm'">
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
        <div class="form-section">
          <div class="form-section-title">接口</div>
          <div class="form-grid">
            <div class="span-2"><label>base_url</label><input v-model="form.llm.base_url" type="text" placeholder="https://api.openai.com/v1"></div>
            <div class="span-2"><label>api_key</label><input v-model="form.llm.api_key" type="text" placeholder="sk-..."></div>
            <div class="span-2"><label>model</label><input v-model="form.llm.model" type="text"></div>
          </div>
        </div>

        <div class="form-section">
          <div class="form-section-title">生成</div>
          <div class="form-grid">
            <SliderField v-model="form.llm.temperature" class="span-2" label="temperature" :min="0" :max="2" :step="0.1" />
            <SliderField v-model="form.llm.max_tokens" class="span-2" label="max_tokens" :min="256" :max="65536" :step="256" />
            <SliderField v-model="form.llm.max_concurrent" class="span-2" label="并发请求数" :min="1" :max="32" :step="1"
                         hint="对所有 LLM 调用生效" />
          </div>
        </div>

        <!-- 按链路覆盖 / extra_params 左右并排（用户要求）：左边是覆盖项、右边是 JSON 正文。
             ⚠️ 原来 .llm-link-row 的滑块列写死 300px，半宽列里会顶破 —— 见下方 CSS 的覆盖。 -->
        <div class="llm-cols">
          <div class="form-section">
            <div class="form-section-title">按链路覆盖<span class="muted small">不勾 = 跟随全局</span></div>
            <!-- 各链路要的输出长度天差地别，别用同一个上限互相迁就。
                 一行一条链路：勾上才出现滑块，没勾的只占一行灰字（跟随全局）。 -->
            <div class="llm-links">
              <div class="llm-link-row" v-for="l in llmLinks" :key="l.key">
                <label class="ln-name">
                  <input type="checkbox" v-model="form.llm.overrides[l.key].on"> {{ l.label }}
                </label>
                <template v-if="form.llm.overrides[l.key].on">
                  <SliderField v-model="form.llm.overrides[l.key].max_tokens"
                               label="max_tokens" :min="256" :max="65536" :step="256" />
                  <SliderField v-model="form.llm.overrides[l.key].temperature"
                               label="temperature" :min="0" :max="2" :step="0.1" />
                  <select v-model="form.llm.overrides[l.key].thinking"
                          aria-label="thinking"
                          title="勾选「单独设置」后会写进 llm.toml；选「默认 thinking」则不写这个键">
                    <option value="">默认 thinking</option>
                    <option value="disabled">关思考</option>
                    <option value="enabled">开思考</option>
                  </select>
                </template>
                <span v-else class="muted small ln-inherit">
                  跟随全局（{{ form.llm.max_tokens }} · {{ form.llm.temperature }} · {{ defaultThinking(l.key) }}）
                </span>
              </div>
            </div>
          </div>

          <div class="form-section">
            <div class="form-section-title">extra_params（JSON）</div>
            <textarea v-model="form.llm.extra_params" class="mono llm-extra" spellcheck="false"
                      rows="10" placeholder="留空 = 不带额外参数"></textarea>
          </div>
        </div>

        <div class="muted small mt-8">{{ llmResult }}</div>
      </template>
      <template v-else>
        <div class="muted small mb-8" title="「返回表单」会放弃未保存的修改">直接编辑 llm.toml 原文</div>
        <textarea v-model="llmRaw" rows="14" class="mono source-editor"></textarea>
      </template>
    </div>
    </template>

    <!-- 账号与模型：邀请码/注册 与 ASR 模型清单 -->
    <template v-else-if="section === 'account'">
    <div class="two-up" :class="{ 'one-up': localNoAuth }">
      <!-- 本地模式（`--local-no-auth`）下**没有登录这回事**：邀请码与注册开关都是给
           多用户部署用的，摆在这里只会让人以为需要它。 -->
      <div v-if="!localNoAuth">
        <div class="section-title">注册与邀请码</div>
        <div class="card">
          <InviteManager />
        </div>
      </div>
    </div>
    </template>

    <!-- 系统集成：录播姬采集（webhook / 导入规则）与 B 站链接下载 -->
    <template v-else-if="section === 'integrations'">
    <div class="section-title">录播姬（采集）</div>
    <div class="card">
      <RecorderConfig />
    </div>

    <div class="section-title">B站链接下载</div>
    <div class="card">
      <DownloaderConfig />
    </div>
    </template>

    <!-- 用量与日志：LLM 账本与运行日志 -->
    <template v-else-if="section === 'monitor'">
    <div class="section-title">LLM 用量</div>
    <div class="card">
      <LlmUsage />
    </div>

    <div class="section-title">运行日志</div>
    <div class="card">
      <LogConsole />
    </div>
    </template>
  </div>
</template>

<script setup>
import { ref, reactive, computed, onMounted } from 'vue'
import { api } from '../api/client'
import { toast } from '../stores/toast'
import Icon from '../components/Icon.vue'
import SliderField from '../components/SliderField.vue'
import InviteManager from '../components/InviteManager.vue'
import RecorderConfig from '../components/RecorderConfig.vue'
import DownloaderConfig from '../components/DownloaderConfig.vue'
import LlmUsage from '../components/LlmUsage.vue'
import LogConsole from '../components/LogConsole.vue'
import { SETTINGS_SECTIONS, settingsSection } from '../utils'
import { localNoAuth as localNoAuthFromServer, localNoAuthKnown } from '../stores/health'

// 子页 key 由路由的 props 传进来（config / llm / account / integrations / monitor）。
// 五块内容都留在这一个组件里：表单状态、/api/config 的加载与保存、源码编辑器都在这一处，
// 拆成子组件反而要额外铺一层 props/emit。
const props = defineProps({
  section: { type: String, default: 'config' }
})
const active = computed(() => settingsSection(props.section))

const globalRaw = ref('')
const llmRaw = ref('')
const configError = ref('')
const llmResult = ref('')
const referenceTitlesText = ref('')
const asrModeHint = computed(() => form.asr.mode === 'hybrid' ? 'SenseVoice 打轴 + qwen3 填词' : 'SenseVoice')
/**
 * 本地免登录位（`GET /api/health.local_no_auth`）—— **与 `App.vue` 同一口径**：
 * 本地模式下没有"注册/邀请"这回事，收起「注册与邀请码」那张卡。
 * ⚠️ 读失败就当 `false`（照旧显示）—— 别默认 true，否则一次请求抖动就把整块藏了。
 */
const localNoAuth = ref(localNoAuthKnown())
/** 本地模式下整页不显示的那些设置子页（见 utils.js 的 `localHidden`，与路由守卫同一判据） */
const visibleSections = computed(() =>
  SETTINGS_SECTIONS.filter((s) => !(s.localHidden && localNoAuth.value))
)
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
    breath_gap: 0.25,
    soft_cut_secs: 6,
    max_segment_chars: 30,
    snap_window: 0.3,
    segment_padding: 0,
    max_new_tokens: 256,
    vad: true,
    vad_threshold: 0.5,
    vad_rescue_db: 16,
    llm_correct: false
  },
  detection: {
    danmaku_window_secs: 10,
    llm_score_weight: 0.3,
    vtuber_name: '',
    reference_titles: [],
    system_prompt: '',
    prompt_template: ''
  },
  cutting: {
    output_dir: './output',
    overwrite: false,
  },
  llm: {
    api_key: '',
    base_url: '',
    model: '',
    temperature: 0,
    max_tokens: 65536,
    max_concurrent: 3,
    extra_params: '',
    // 按链路覆盖：每行 = { on, max_tokens, temperature, thinking }；
    // `on` 决定保存时写值还是发 null（服务端 patch 的 null = 删键 = 回到跟随全局，见 buildLlmPatch）
    overrides: {}
  }
})

/** 链路 key → 中文名（key 与后端 `LlmLink::key()` 一致；表行按 llm_config.overrides 的键来） */
const LLM_LINK_ZH = { detect: '事件检测', polish: '精修剪辑', correct: '字幕订正', utility: '零散单发' }
/** 该链路 thinking 不写时的生效值（与 `LlmLink::default_thinking` 一致）—— 只用于"跟随全局"那句灰字的展示 */
const LLM_LINK_THINKING = { polish: '关思考', utility: '关思考' }
function defaultThinking(key) {
  return LLM_LINK_THINKING[key] || '跟随 extra_params'
}

/** 表行：只列后端回给你的链路（新增链路不用改这里就会冒出来） */
const llmLinks = computed(() =>
  Object.keys(form.llm.overrides || {}).map((key) => ({
    key,
    label: LLM_LINK_ZH[key] || key
  }))
)

/** 只夹范围、不吸附 step（SliderField 的手填也是这个口径）；非数字回落 fallback。 */
function clampNum(v, min, max, fallback) {
  const n = Number(v)
  if (!Number.isFinite(n)) return fallback
  return Math.min(max, Math.max(min, n))
}

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
      breath_gap: Number(f.asr.breath_gap ?? 0.25),
      soft_cut_secs: Number(f.asr.soft_cut_secs ?? 6),
      max_segment_chars: Number(f.asr.max_segment_chars) || 30,
      snap_window: Number(f.asr.snap_window ?? 0.3),
      segment_padding: Number(f.asr.segment_padding ?? 0),
      max_new_tokens: Number(f.asr.max_new_tokens) || 256,
      vad: !!f.asr.vad,
      vad_threshold: Number(f.asr.vad_threshold ?? 0.5),
      vad_rescue_db: Number(f.asr.vad_rescue_db ?? 16),
      llm_correct: !!f.asr.llm_correct
    },
    detection: {
      danmaku_window_secs: Number(f.detection.danmaku_window_secs) || 10,
      llm_score_weight: Number(f.detection.llm_score_weight ?? 0.3),
      vtuber_name: f.detection.vtuber_name || '',
      reference_titles: f.detection.reference_titles || [],
      system_prompt: f.detection.system_prompt || '',
      prompt_template: f.detection.prompt_template || ''
    },
    cutting: {
      output_dir: f.cutting.output_dir || './output',
      overwrite: !!f.cutting.overwrite
    },
  }
}

function buildLlmPatch() {
  const f = form.llm
  const llm = {
    base_url: f.base_url || '',
    api_key: f.api_key || '',
    model: f.model || '',
    temperature: Number(f.temperature) || 0,
    max_tokens: Number(f.max_tokens) || 65536,
    max_concurrent: Number(f.max_concurrent) || 1
  }
  if (f.extra_params && f.extra_params.trim()) {
    try {
      llm.extra_params = JSON.parse(f.extra_params)
    } catch (e) {
      throw new Error('extra_params 不是合法 JSON，请检查格式')
    }
  } else {
    // ⚠️ 清空输入框 = **要删掉这个键**，必须显式发 null。
    // 省略它是不行的：patch 是**合并**语义（本次没提到的键保持原值），
    // 于是模板里那条 `extra_params = { seed = 42, thinking = ... }` 永远在那儿 ——
    // 用户清空后保存、页面重新加载，看到的又是"预设内容"（真机报的就是这个）。
    // 同一个机制在下面的 overrides 已经用了：null = 删键。
    llm.extra_params = null
  }
  // 按链路覆盖：勾上的写值，没勾的**发 null** —— `patch_config_file` 把 null 当「删键」，
  // 这样"取消勾选 = 回到跟随全局"才表达得出来（发空串会写出 max_tokens = ""，
  // 下次读配置直接解析失败、整份 LLM 配置被打回默认值）。
  llm.overrides = {}
  for (const [key, row] of Object.entries(f.overrides || {})) {
    llm.overrides[key] = row.on
      ? {
          max_tokens: Math.round(clampNum(row.max_tokens, 256, 65536, f.max_tokens)),
          temperature: clampNum(row.temperature, 0, 2, f.temperature),
          // thinking 留「默认」就不写这个键（生效值由链路默认 / [llm].thinking 决定）
          thinking: row.thinking ? row.thinking : null
        }
      : { max_tokens: null, temperature: null, thinking: null }
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
    // 用 global_config（只含 .yekitrim/config.toml）：这块表单编辑的就是**程序配置**，
    // merged 配置会被某个工程的 yekitrim.toml 覆盖，表单就会和源文件对不上。
    // （2026-10-03 起 /api/config 不再回 merged 的 `config`，那个兜底随之删掉。）
    const c = d.global_config || {}
    if (c.asr) Object.assign(form.asr, {
      mode: c.asr.mode || 'single',
      num_workers: c.asr.num_workers ?? 0,
      chunk_duration: c.asr.chunk_duration ?? 60,
      chunk_overlap: c.asr.chunk_overlap ?? 10,
      segment_gap: c.asr.segment_gap ?? 1,
      llm_correct: !!c.asr.llm_correct,
      breath_gap: c.asr.breath_gap ?? 0.25,
      soft_cut_secs: c.asr.soft_cut_secs ?? 6,
      max_segment_chars: c.asr.max_segment_chars ?? 30,
      snap_window: c.asr.snap_window ?? 0.3,
      segment_padding: c.asr.segment_padding ?? 0,
      max_new_tokens: c.asr.max_new_tokens ?? 256,
      // 没写这一项 = 跟随默认（开）；表单要给个能点的复选框，所以在这里落到布尔值
      vad: c.asr.vad ?? true,
      vad_threshold: c.asr.vad_threshold ?? 0.5,
      vad_rescue_db: c.asr.vad_rescue_db ?? 16
    })
    if (c.detection) Object.assign(form.detection, {
      danmaku_window_secs: c.detection.danmaku_window_secs ?? 10,
      llm_score_weight: c.detection.llm_score_weight ?? 0.3,
      vtuber_name: c.detection.vtuber_name || '',
      reference_titles: Array.isArray(c.detection.reference_titles) ? [...c.detection.reference_titles] : [],
      system_prompt: c.detection.system_prompt || '',
      prompt_template: c.detection.prompt_template || ''
    })
    referenceTitlesText.value = (form.detection.reference_titles || []).join('\n')
    if (c.cutting) Object.assign(form.cutting, {
      output_dir: c.cutting.output_dir || './output',
      overwrite: !!c.cutting.overwrite,
    })
    const l = d.llm_config || {}
    const gMax = l.max_tokens ?? 65536
    const gTemp = l.temperature ?? 0
    form.llm = {
      api_key: l.api_key || '',
      base_url: l.base_url || '',
      model: l.model || '',
      temperature: gTemp,
      max_tokens: gMax,
      max_concurrent: l.max_concurrent ?? 3,
      extra_params: l.extra_params ? JSON.stringify(l.extra_params, null, 2) : '',
      // 后端把四张覆盖表都序列化出来（缺席的项是 null）→ 行按它的键走。
      // on = 这条链路在 llm.toml 里已经有一项覆盖；滑块显示值取"覆盖值，否则当前全局值"。
      overrides: Object.fromEntries(
        Object.entries(l.overrides || {}).map(([k, v]) => [
          k,
          {
            on: v?.max_tokens != null || v?.temperature != null || !!v?.thinking,
            max_tokens: v?.max_tokens ?? gMax,
            temperature: v?.temperature ?? gTemp,
            thinking: v?.thinking || ''
          }
        ])
      )
    }
    loaded.value = true
  } catch (e) { loaded.value = false }
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

async function testLlm() {
  llmResult.value = '测试中…'
  try {
    const r = await api('/api/llm/test', { method: 'POST' })
    llmResult.value = '连接成功 ' + r.latency_ms + 'ms'
  } catch (e) {
    llmResult.value = e.message
  }
}

onMounted(async () => {
  // /api/config 只有程序配置与 LLM 两个子页用（含源码编辑器）；模型列表只有「账号与模型」用。
  // 其余子页的组件自带接口，别白拉一遍。
  if (props.section === 'config' || props.section === 'llm') loadConfig()
  // 每个子页都要：标签条靠它过滤掉本地模式下不显示的项（store 有缓存，只有首次真发请求）。
  // ⚠️ 别挪回 `account` 分支里 —— 那样只有进账号页时标签才收缩，别处照旧显示 5 个。
  localNoAuth.value = await localNoAuthFromServer()
})
</script>

<style scoped>
/* 标签条下面紧接第一个区块标题：section-title 自带 20px 上边距，这里已经翻到新的一页了 */
.tabs { margin-bottom: 10px; }
.tabs + .section-title { margin-top: 4px; }
/* 表单密度：字段是一行「标签·滑块·数字框」，行高本来就矮，留白按行距给（8px 上下 / 12px 左右）。
   下面的 section/prompt 间距都跟着这条口径收，别再单独放大某一处。 */
.llm-grid { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1.3fr); gap:8px 14px; align-items:stretch; }
.llm-fields { display:grid; grid-template-columns:1fr 1fr; gap:6px 12px; align-content:start; }
.llm-extra { display:flex; flex-direction:column; }
.llm-extra textarea { flex:1; min-height:230px; resize:vertical; font-size:12px; line-height:1.6; }
/* 按链路覆盖：一行一条链路 —— [勾选+名字] [max_tokens 滑块] [temperature 滑块] [thinking 下拉]。
   没勾的只占一行灰字（跟随全局），所以默认状态是 4 行矮的、勾开才长出滑块。
   数值项走 SliderField（与上面那几格同一套控件），别退回裸数字框。
   ⚠️ 两个数值格子给**上限 300px + justify-content:start**：默认的 1fr 会把滑块拉到格子中央，
   一行里三样东西散开、看着很空（实测过）。`:deep(.slf-name)` 改成不伸展，标签紧贴滑块；
   滑块宽度仍是格子宽的 1/3（SliderField 自己的约定，不改）。 */
/* 按链路覆盖 / extra_params 左右并排（用户要求"做成左右结构"）。
   右栏给固定上限宽度：JSON 正文不需要更宽，再宽也是换行看不清。 */
.llm-cols { display:grid; grid-template-columns:minmax(0,1fr) minmax(300px,420px); gap:0 20px; align-items:start; }
@media (max-width:1200px) { .llm-cols { grid-template-columns:minmax(0,1fr); gap:14px 0; } }
/* ⚠️ 半宽列里那一行放不下两个 300px 滑块（默认 grid-template-columns 是 max-content 300px 300px max-content）——
   改成两列弹性宽度，滑块自己会收。 */
.llm-cols .llm-link-row { grid-template-columns:max-content minmax(0,1fr) minmax(0,1fr) max-content; }
.llm-links { margin-top:12px; }
.llm-links-head { display:flex; align-items:baseline; gap:8px; margin-bottom:6px; font-weight:700; font-size:12.5px; }
.llm-link-row { display:grid; grid-template-columns:max-content minmax(0,300px) minmax(0,300px) max-content;
  justify-content:start; gap:2px 10px; align-items:center; min-width:0; }
.llm-link-row + .llm-link-row { margin-top:2px; }
.llm-link-row .ln-name { display:flex; align-items:center; gap:6px; font-size:12.5px; white-space:nowrap; cursor:pointer; }
.llm-link-row :deep(.slf-name) { flex:0 1 auto; }
.llm-link-row select { width:auto; min-width:112px; padding:3px 6px; font-size:12.5px; }
.ln-inherit { grid-column:2 / -1; }
.form-section { padding:8px 0; border-bottom:1px dashed var(--line-soft); }
.form-section:first-child { padding-top:0; }
.form-section:last-child { border-bottom:none; padding-bottom:0; }
.form-section-title { font-weight:700; font-size:13px; margin-bottom:6px; }
.form-grid { display:grid; grid-template-columns:repeat(6, 1fr); gap:8px 12px; align-items:start; }
/* 注册与邀请码 / 模型管理：两张卡都短，并排省一屏竖高（窄屏回落成上下） */
.two-up { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr); gap:0 16px; align-items:start; }
/* 本地模式只留一张卡（注册与邀请码被收起）—— 让它占满整行，别半宽留白 */
.one-up { grid-template-columns:minmax(0,1fr); }
.span-2 { grid-column:span 2; }
.span-4 { grid-column:span 4; }
/* 「高级参数」：默认收起。里面是"默认值就是最佳实践"的调参与提示词 ——
   摊开会把常用项淹掉（用户口径："死配置或者最佳配置就隐藏吧"）。 */
.form-adv { margin-top:10px; border-top:1px dashed var(--line-soft); padding-top:6px; }
.form-adv > summary { cursor:pointer; font-weight:700; font-size:13px; color:var(--ink-dim);
  display:flex; align-items:center; gap:8px; padding:4px 0; list-style:none; }
.form-adv > summary::-webkit-details-marker { display:none; }
.form-adv > summary::before { content:"▸"; color:var(--ink-faint); flex:none; }
.form-adv[open] > summary::before { content:"▾"; }
.form-adv > summary:hover { color:var(--ink); }
/* Mode = 一行「标签 · 下拉 · 说明」：说明原本另起一行，把这一格撑到 81px，
   而同一行的数值行只有 23px —— 那一行右侧因此空出 58px 的竖向留白（量过）。
   下拉给 auto 宽 + 收窄内边距，一行放得下（窄屏自动折行）。 */
.mode-field { display:flex; flex-wrap:wrap; align-items:center; gap:2px 8px; min-width:0; }
.mode-field > label { margin:0; white-space:nowrap; }
.mode-field > select { width:auto; min-width:104px; padding:4px 8px; font-size:12.5px; }
.mode-field > .mode-hint { min-width:0; overflow-wrap:anywhere; }
.span-all { grid-column:1 / -1; }
.prompt-pair { display:grid; grid-template-columns:1fr 1fr; gap:8px 12px; margin-top:8px; }
.prompt-pair textarea, .prompt-with-side textarea { min-height:230px; resize:vertical; }
.prompt-with-side { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr); gap:8px 12px; margin-top:8px; align-items:start; }
.side-col { display:flex; flex-direction:column; gap:8px; min-width:0; }
.side-title { font-weight:700; font-size:12.5px; color:var(--ink); margin-bottom:4px; }
.side-gap { margin-top:6px; }
.side-check { align-self:flex-start; padding:0; margin-top:2px; }
.side-hint { margin-top:2px; }
.check-field { display:flex; align-items:center; gap:7px; align-self:end; padding:0 0 6px; }
.check-field label { margin:0; cursor:pointer; color:var(--ink); }
.source-editor { font-size:12px; line-height:1.7; }
button:disabled { opacity:.45; cursor:not-allowed; }
@media (max-width: 1200px) {
  .two-up { grid-template-columns:1fr; }
  .form-grid { grid-template-columns:repeat(3, 1fr); }
}
@media (max-width: 900px) {
  .llm-grid { grid-template-columns:1fr; }
  .llm-extra textarea { min-height:160px; }
  /* 链路覆盖行：窄屏一行放不下「名字 + 两个滑块 + 下拉」→ 拆成一列（名字占一行） */
  .llm-link-row { grid-template-columns:minmax(0,1fr); }
  .llm-link-row .ln-name, .ln-inherit { grid-column:1 / -1; }
  .form-grid { grid-template-columns:repeat(2, 1fr); }
  .span-2 { grid-column:1 / -1; }
  .prompt-pair, .prompt-with-side { grid-template-columns:1fr; }
}
</style>

