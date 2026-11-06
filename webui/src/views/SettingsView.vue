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
    <!-- 内容列：所有分页共用 .settings-block（min(760px,100%)、左对齐）—— 别在每页各写一份 max-width -->
    <div class="settings-block" data-cap="settings.config">
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
          </div>
        </div>

        <!-- ② 检测（首屏只留主播名；LLM 评分权重等调参进「高级参数」） -->
        <div class="form-section">
          <div class="form-section-title">检测</div>
          <div class="form-grid">
            <div class="span-2">
              <label>主播名</label>
              <input v-model="form.detection.vtuber_name" type="text" placeholder="追加到切片标题">
            </div>
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
          <summary>高级参数<span class="muted small">断句 / 调参</span></summary>

          <!-- 识别与检测的**开关/调参**：首屏只留「模式 / 主播名 / 输出目录 / 覆盖已有文件」，
               这些默认值已按实测调好的旋钮收进这里（隐藏 ≠ 删除，表单一字未改）。 -->
          <div class="form-section">
            <div class="form-section-title">识别与检测<span class="t-sub">默认值已按实测调好</span></div>
            <div class="form-grid">
              <div class="check-field span-2">
                <input id="vadEnabled" v-model="form.asr.vad" type="checkbox">
                <label for="vadEnabled" title="single 模式快 2~3 倍，hybrid 不受影响">VAD 只解码语音段</label>
              </div>
              <div class="check-field span-2">
                <input id="llmCorrect" v-model="form.asr.llm_correct" type="checkbox">
                <label for="llmCorrect">LLM 订正转录</label>
              </div>
              <SliderField v-model="form.detection.llm_score_weight" class="span-4"
                           label="LLM 评分权重" :min="0" :max="1" :step="0.05"
                           hint="0 = 只用弹幕/字幕热度" />
            </div>
          </div>

          <!-- ⚠️ 「提示词」整组**已搬到 LLM 配置页**（2026-10，用户要求）：参考标题与两个检测
               prompt 都是喂给 LLM 的文本，摆在"程序配置"里要跨页找。**别在这里加回来。** -->

          <div class="form-section">
            <div class="form-section-title">断句与分块<span class="t-sub">默认值已按实测调好</span></div>
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
            <div class="form-section-title">VAD 与解码<span class="t-sub">默认值已按实测调好</span></div>
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
    </div>
    </template>

    <!-- LLM 配置：写入 .yekitrim/llm.toml -->
    <template v-else-if="section === 'llm'">
    <div class="settings-block">
    <!-- /api/config 读失败：表单里是**默认值**，而「保存」只是变灰、没有任何说明 ——
         这一页必须和程序配置页一样把原因摆出来（读不到就别让用户以为界面自己变了）。 -->
    <div v-if="loadFailed" class="error-banner" data-llm-load-error>
      <Icon name="alert" :size="15" />
      <div>
        <b>LLM 配置未加载，显示的是默认值</b>
        <div class="muted small mt-4">{{ configLoadError || '读取 /api/config 失败' }}；保存已禁用，重试成功后再改</div>
      </div>
      <button class="mini-btn flex-none" @click="loadConfig">重试</button>
    </div>
    <!-- ⚠️ 保存失败**常驻**显示（P0：后端 400 曾被 toast 一闪吞掉，用户以为存上了）；回读不一致也在这里 -->
    <div v-if="llmSaveError" class="error-banner" data-llm-save-error>
      <Icon name="alert" :size="15" />
      <div>
        <b>保存失败</b>
        <div class="muted small mt-4" data-llm-save-error-text>{{ llmSaveError }}</div>
      </div>
    </div>
    <div v-if="llmSaveWarn" class="error-banner" data-llm-save-warn>
      <Icon name="alert" :size="15" />
      <div>
        <b>已提交，但回读不一致</b>
        <div class="muted small mt-4">{{ llmSaveWarn }}</div>
      </div>
    </div>
    <div class="section-title row spread">
      <span>LLM 配置 llm.toml</span>
      <span class="row">
        <!-- 「测试连接」已经挪到**每一份配置的行尾**（只测那一份）；这里不再有全局按钮 -->
        <button class="btn" @click="saveLlm()" :disabled="!loaded">
          <Icon name="save" :size="13" /> 保存 LLM
        </button>
      </span>
    </div>

    <!-- 快速配置（P3）：四个字段 + [获取模型] + [保存并测试]。
         ⚠️ 保存仍走本页既有的 saveLlm（buildLlmPatch + PUT + 回读 + 错误横幅）——
         本卡只负责把「填好的那四个值」交给它，不在组件里另写一套落盘。 -->
    <div class="card" data-cap="settings.llm-quick">
      <LlmQuickSetup
        :loaded="loaded"
        :provider-mode="llmProviderMode"
        :providers="form.llm.providers || []"
        v-model:active-id="form.llm.active"
        :base-url="form.llm.base_url"
        :model="form.llm.model"
        :key-set="quickKeySet"
        :persist="quickPersist"
        @update:base-url="form.llm.base_url = $event"
        @update:model="form.llm.model = $event"
        @update:api-key="form.llm.api_key = $event"
      />
    </div>

    <!-- 详细表单（多接口 / 能力 / 参数 / 提示词）：默认折叠 —— 隐藏 ≠ 删除，所有入口仍可达 -->
    <details class="form-adv">
      <summary data-cap="settings.llm-advanced">高级配置（多接口 / 能力 / 参数 / 提示词）</summary>
    <div class="card">
        <!-- ① 接口：填好 key → 取模型 → 选中（模型选择器见 components/llm/LlmModelPicker.vue） -->
        <div class="form-section">
          <div class="form-section-title">接口<span class="t-sub">{{ llmProviderMode
            ? '多份配置 · 同一时间只展开一份' : '填好 key → 获取模型 → 选中' }}</span></div>
          <!-- 多 provider（P3，照 DSH：列表 → 只展开一份 → 模型行 → 档位行）。
               ⚠️ **R4**：后端没回 providers ⇒ llmProviderMode=false ⇒ 走下面这个老的单配置表单 -->
          <LlmProviders v-if="llmProviderMode" :providers="form.llm.providers"
                        v-model:active-id="form.llm.active"
                        :protocols="protocols" :protocols-source="protocolsSource" :caps="caps"
                        @probe="onProbeCaps" />
          <div v-else class="form-grid">
            <div class="span-2">
              <label>base_url</label>
              <input v-model="form.llm.base_url" type="text" placeholder="https://api.deepseek.com/v1">
            </div>
            <div class="span-2">
              <label>api_key</label>
              <!-- 只写不读：服务端不再回明文（P3），永远不预填；留空 = 不动这个键 -->
              <input v-model="form.llm.api_key" type="password" autocomplete="off"
                     :placeholder="apiKeyPlaceholder" :title="apiKeyTitle">
            </div>
            <div class="span-all">
              <label>model</label>
              <LlmModelPicker v-model="form.llm.model" :base-url="form.llm.base_url" :api-key="form.llm.api_key" @fetched="onModelsFetched" />
            </div>
          </div>
        </div>

        <!-- ② 生成（**只有 R4 单配置模式**）：能力驱动的两个旋钮 —— 输入类型 + 推理档位。
             ⚠️ 温度 / 输出上限 / JSON 都挪去「按链路覆盖」的**全局默认**那一行（2026-10-20 用户口径：
                它们不属于接口配置，属于请求参数）；provider 模式下这里整块不渲染。
             并发数见「高级」（进程级旋钮）。 -->
        <div v-if="!llmProviderMode" class="form-section">
          <div class="form-section-title">生成<span class="t-sub caps-src" :title="capsTitle">{{ capsText }}</span></div>
          <LlmParamFields :caps="caps" :target="llmParamsTarget" id-prefix="llm-g"
                          :fields="['input', 'reasoning']"
                          :input-text="inputText" :input-image="inputImage" :input-declared="inputDeclared"
                          @update:inputText="setInputKind('text', $event)"
                          @update:inputImage="setInputKind('image', $event)" />
          <div v-if="capsError" class="muted small mt-4 caps-err" :title="capsError">{{ capsError }}</div>
        </div>


        <!-- ③ 按链路：行由 llm_config.overrides 的键**自动生成**（后端加链路，这里一行都不用改）。
             「跟随全局」由**勾选框**表达：不勾 = 保存时发 null = 删键（别用空输入框 —— 那表达不出"没有值"）。 -->
        <div class="form-section">
          <div class="form-section-title">按链路覆盖<span class="t-sub">勾上才覆盖，不勾 = 跟随全局</span></div>
          <div class="llm-cols">
            <!-- 全局默认（写 [llm.params]；provider 模式下 = 当前接口自己的 params）——
                 温度 / 输出上限 / JSON 从接口面板挪到这里：它们是**请求参数**，不是接口字段 -->
            <div class="llm-default-row">
              <span class="ln-name">全局默认<span class="ln-hint">（{{ llmProviderMode ? '当前接口的参数' : '[llm.params]' }}）</span></span>
              <LlmParamFields :caps="caps" :target="llmParamsTarget" id-prefix="llm-def"
                              :fields="['temperature', 'max_tokens', 'json_mode']"
                              :json-mode="jsonModeOn" @update:jsonMode="setJsonMode" compact />
            </div>
            <div class="llm-link-row" v-for="l in llmLinks" :key="l.key">
              <label class="ln-name">
                <input type="checkbox" v-model="form.llm.overrides[l.key].on">
                <span class="ln-text">{{ l.label }}<span v-if="l.hint" class="ln-hint">（{{ l.hint }}）</span></span>
              </label>
              <LlmParamFields v-if="form.llm.overrides[l.key].on" :caps="caps"
                              :target="form.llm.overrides[l.key]"
                              :fields="['reasoning', 'temperature', 'max_tokens']"
                              :id-prefix="'llm-' + l.key" compact />
              <span v-else class="muted small ln-inherit" :title="inheritTitle(l.key)">
                跟随全局（{{ llmParamsTarget.temperature }} · {{ llmParamsTarget.max_tokens }}）
              </span>
            </div>
          </div>
        </div>

        <!-- ④ 高级：并发（进程级旋钮）· extra_params 逃逸口。⚠️「源文件」那块已删（用户：没必要有） -->
        <div class="form-section">
          <div class="form-section-title">高级<span class="t-sub">并发 · 逃逸口</span></div>
          <div class="form-grid">
            <SliderField v-model="form.llm.max_concurrent" class="span-2" label="并发请求数"
                         :min="1" :max="32" :step="1" hint="对所有 LLM 调用生效（进程级旋钮）" />
          </div>
          <div>
            <label title="原样进请求体，覆盖上面所有同名参数；留空 = 不带额外参数">extra_params（JSON）</label>
            <textarea v-model="form.llm.extra_params" class="mono llm-extra" spellcheck="false"
                      rows="10" placeholder="留空 = 不带额外参数"></textarea>
          </div>
        </div>

        <!-- ⚠️ 「提示词」从**程序配置页**搬来这里（2026-10，用户要求）：参考标题 + 两个检测
             prompt 都是喂给 LLM 的文本。⚠️ 写的是 **config.toml 的 [detection]**（不是 llm.toml）
             —— 搬的是**位置**，不是归属，保存仍走 buildGlobalPatch。 -->
        <div class="form-section">
          <div class="form-section-title">提示词<span class="t-sub">写入 config.toml 的 [detection]</span></div>
          <!-- 默认折叠：三块 textarea 合计 ~600px，展开会把「按链路覆盖」顶到第二屏。
               ⚠️ 只是**视觉折叠**（内容仍在 DOM、保存读的是 reactive 表单）—— 不改任何交互契约。 -->
          <details class="form-adv">
          <summary>参考标题 / 检测 system_prompt / prompt_template</summary>
          <div class="prompt-pair">
            <div>
              <label title="每行一条，可含 emoji；清空保存则运行时用内建示例">参考标题</label>
              <textarea v-model="referenceTitlesText" rows="8" placeholder="每行一条参考标题"
                        @input="referenceTitlesFromDefault = false"></textarea>
              <!-- 「参考标题」的两条口径（2026-10-18 用户拍板，**推翻了上一版**）：
                   ① **留空就把内建默认值填进输入框**（与其他设置项一致：表单里显示的就是
                      "实际会用的那一份"）—— 用户要改就在默认值基础上改，保存即写进 config.toml；
                   ② **清空再保存 = 回到预设**（`patch_config_file` 是深合并，`[]` 会真的清掉数组 ⇒
                      `resolve_reference_titles` 回落到内建示例）。此时下面那条灰字预览把
                      "实际会用哪几条"列出来 —— 清空**不等于没有参考标题**。 -->
              <div v-if="referenceTitlesFromDefault && referenceTitlesText.trim()" class="muted small mt-4">
                以上是内建默认值（本配置原先留空）。改了保存即生效；<b>清空再保存</b> = 回到内建示例。
              </div>
              <div v-if="!referenceTitlesText.trim() && referenceTitlesDefault.length" class="rt-preview">
                <div class="muted small">
                  留空时使用内建示例（{{ referenceTitlesDefault.length }} 条）：
                </div>
                <ul class="rt-list">
                  <li v-for="(t, i) in referenceTitlesDefault" :key="i">{{ t }}</li>
                </ul>
              </div>
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
          </details>
        </div>

    </div>
    </details>
    </div>
    </template>

    <!-- 账号与模型：邀请码/注册 与 ASR 模型清单 -->
    <template v-else-if="section === 'account'">
    <div class="settings-block" data-cap="settings.account">
    <div class="two-up" :class="{ 'one-up': localNoAuth }">
      <!-- 本地模式（`--local-no-auth`）下**没有登录这回事**：邀请码与注册开关都是给
           多用户部署用的，摆在这里只会让人以为需要它。 -->
      <div v-if="!localNoAuth">
        <div class="section-title">注册与邀请码</div>
        <div class="card">
          <InviteManager />
        </div>

        <!-- 账号配额（P5）：与邀请码同一页 —— 都是"管别人那个账号"的事。
             ⚠️ 配额只对**普通账号**有意义（管理员豁免，他要能清理超额的账号）。 -->
        <div class="section-title">账号配额</div>
        <div class="card">
          <UserQuotaManager />
        </div>
      </div>
    </div>
    </div>
    </template>

    <!-- 系统集成：录播姬采集（webhook / 导入规则）与 B 站链接下载 -->
    <template v-else-if="section === 'integrations'">
    <div class="settings-block" data-cap="settings.integrations">
    <div class="section-title">采集（录播姬 / 通用 Webhook）</div>
    <div class="card">
      <RecorderConfig />
    </div>

    <div class="section-title">B站链接下载</div>
    <div class="card">
      <DownloaderConfig />
    </div>
    </div>
    </template>

    <!-- 用量与日志：LLM 账本与运行日志 -->
    <template v-else-if="section === 'monitor'">
    <div class="settings-block" data-cap="settings.monitor">
    <div class="section-title">LLM 用量</div>
    <div class="card">
      <LlmUsage />
    </div>

    <div class="section-title row spread">
      <span>运行日志</span>
      <!-- 日志默认**不挂载**（一进页连 SSE 会一直占着连接）；要看再点开。 -->
      <button class="mini-btn" :aria-expanded="logsOpen ? 'true' : 'false'"
              :title="logsOpen ? '断开实时日志（SSE）' : '连接实时日志（SSE）'"
              @click="toggleLogs">{{ logsOpen ? '隐藏实时日志' : '显示实时日志' }}</button>
    </div>
    <div class="card">
      <LogConsole v-if="logsOpen" />
      <div v-else class="muted small">默认不连接；点上面的按钮再开始接收</div>
    </div>
    </div>
    </template>
  </div>
</template>

<script setup>
import { ref, reactive, computed, onMounted, onBeforeUnmount, watch } from 'vue'
import { api } from '../api/client'
import { fetchCatalog, fetchProtocols, saveLlm as putLlm } from '../api/llm'
import { BUILTIN_PROTOCOLS } from '../components/llm/protocols.js'
import { normalizeParams } from '../components/llm/params.js'
import { CAPS_SOURCE_ZH, DEFAULT_INPUT, inputToArray, mergeCaps, normalizeCaps, normalizeLevel } from '../components/llm/caps.js'
import { toast } from '../stores/toast'
import Icon from '../components/Icon.vue'
import SliderField from '../components/SliderField.vue'
import LlmModelPicker from '../components/llm/LlmModelPicker.vue'
import LlmParamFields from '../components/llm/LlmParamFields.vue'
import LlmProviders from '../components/llm/LlmProviders.vue'
import LlmQuickSetup from '../components/llm/LlmQuickSetup.vue'
import InviteManager from '../components/InviteManager.vue'
import UserQuotaManager from '../components/UserQuotaManager.vue'
import RecorderConfig from '../components/RecorderConfig.vue'
import DownloaderConfig from '../components/DownloaderConfig.vue'
import LlmUsage from '../components/LlmUsage.vue'
import LogConsole from '../components/LogConsole.vue'
import { SETTINGS_SECTIONS, settingsSection } from '../utils'
import { localNoAuth as localNoAuthFromServer, localNoAuthKnown } from '../stores/health'
import { useDisclosure } from '../composables/useDisclosure'

// 子页 key 由路由的 props 传进来（config / llm / account / integrations / monitor）。
// 五块内容都留在这一个组件里：表单状态、/api/config 的加载与保存、源码编辑器都在这一处，
// 拆成子组件反而要额外铺一层 props/emit。
const props = defineProps({
  section: { type: String, default: 'config' }
})
const active = computed(() => settingsSection(props.section))

const globalRaw = ref('')
const configError = ref('')
/** 模型能力（caps）：`GET /api/config/llm/catalog`（内置目录）+ 探针带回的那份**覆盖**它；null = 未知。 */
const caps = ref(null)
const capsSource = ref('')
/** `GET /api/llm/models` 真探到的能力（model id → 归一化 caps）—— 「能获取当然最好」：它能覆盖内置表。
 *  只有**探针那条路**会带 caps（表命中零网络，服务端不带）。按 model id 存，切回来还在。 */
const probeCaps = reactive({})
/** 能力读失败的原因 —— **不许静默**：拿不到能力就说出来，别让用户以为"这个模型本来就没这些旋钮" */
const capsError = ref('')
const referenceTitlesText = ref('')
/** 留空时**实际会用**的内建参考标题（服务端 `GET /api/config` 的 `reference_titles_default`）。
 *
 *  ⚠️ **2026-10-18 口径改了**（用户：「将默认参数写入，和其他逻辑一致就行，留空就用预设的」）：
 *  以前**刻意不预填**（怕用户"以为是自己的"），现在改成**留空就把默认值填进输入框** ——
 *  与其他设置项一致：表单显示的就是"实际会用的那一份"；用户要改就在默认值上改，
 *  清空再保存 = 回到内建示例（`patch_config_file` 深合并，`[]` 会真的清掉数组）。
 *  ⚠️ 代价（照实记）：内建默认值一旦随保存写进 config.toml，就与代码里的默认值脱钩了 ——
 *  以后改内建那几条对这份配置无效（与检测 `prompt_template` 的副本同一条规矩）。 */
const referenceTitlesDefault = ref([])
/** 输入框里那份是不是**刚从内建默认值填进去的**（用户一改/一清空就置 false）。只驱动那行提示文案。 */
const referenceTitlesFromDefault = ref(false)
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
/** 「运行日志」默认不挂载：一进 monitor 子页就连 SSE 会一直占着连接（P3）。 */
const { open: logsOpen, toggle: toggleLogs } = useDisclosure('settings.logs', false)
const editSource = ref(false)
// 只有成功读到 /api/config 才允许保存：否则表单里的内建默认值会覆盖真实配置
const loaded = ref(false)
/** 读失败（与「正在读」分开）：LLM 页据此出常驻错误条，而不是让「保存」默默变灰 */
const loadFailed = ref(false)
const configLoadError = ref('')

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
    // api_key **只写不读**：服务端不再回明文（P3），表单里永远是空的；留空 = 不动这个键
    api_key: '',
    api_key_set: false,
    base_url: '',
    model: '',
    max_concurrent: 3,
    // **请求参数面**（v2 的 [llm.params]）：'' / 缺键 = 不写这个键（也就是**不发送**）。
    // 档位清单不在这里写死 —— 由模型能力表给（components/llm/caps.js 归一化）
    // ⚠️ 不要给 reasoning 一个 `''` 默认值（P0 事故：空串被写进 llm.toml ⇒ 整份配置回退默认）；
    //    键缺席 = 不表态，由 normalizeParams 决定发不发
    params: { temperature: 0, max_tokens: 65536, json_mode: false },
    // 按链路覆盖：每行 = { on, max_tokens, temperature, reasoning }（**与 params 同一套字段**）；
    // `on` 决定保存时写值还是发 null（服务端 patch 的 null = 删键 = 回到跟随全局，见 buildLlmPatch）
    overrides: {},
    /** **界面可见的链路**（后端 `override_links`，不含 correct）；null/空 = 退回老的四行（R4） */
    overrideLinks: null,
    /** 后端这次真的回了哪几个 overrides 键 —— 只在这些键上做"不勾 = 删键"，别给新键写空表 */
    knownLinks: [],
    // **能力面**（v2 的 [llm.models."<id>"]）：内置表没收录这个模型时，由「高级」那一小块编辑
    models: {},
    // **多 provider**（P3）：`active` = 当前在用的那份；`providers === null` = 后端还没回这个字段
    // ⇒ 走老的单配置界面（R4：不许白屏、不许报错、行为逐字节不变）
    active: '',
    providers: null
  }
})

/** 链路 key → 中文名（key 与后端 `LlmLink::key()` 一致；**哪几行可见由后端 `override_links` 给**） */
const LLM_LINK_ZH = {
  detect: '事件检测',
  polish: '精修剪辑',
  title: '标题生成',
  correct: '字幕订正',       // ⚠️ 只从界面隐藏（后端 override_links 不含它），代码/TOML 仍在用 —— 别删这一项
  utility: '零散单发'
}
/** 行内补一句"这条链路具体管什么"（只在需要时） */
const LLM_LINK_HINT = { title: '封面文案', utility: '贴纸包·扫描标签·测试连接' }
/** 该链路 reasoning 不写时的生效值（与 `LlmLink::default_thinking` 一致）—— 只用于"跟随全局"那句灰字的展示 */
const LLM_LINK_THINKING = { polish: '关思考', utility: '关思考' }
function defaultThinking(key) {
  return LLM_LINK_THINKING[key] || '跟随 extra_params'
}

/** 「生成」组标题右边那行小字：**能力来源**（内置目录 / 联网探测 / 手填）+ 有没有读到（细节挂 title）。 */
const capsText = computed(() => {
  const bits = []
  if (capsSource.value) bits.push(CAPS_SOURCE_ZH[capsSource.value] || capsSource.value)
  if (manualCapsOn.value) bits.push(CAPS_SOURCE_ZH.manual)
  // ⚠️ 别带"能力："前缀：它接在标题「生成」后面会被读成"生成能力：…"（上一版就是这样）
  if (!bits.length && capsError.value) return '能力读取失败'
  if (!bits.length) return ''
  const base = bits.join(' · ')
  if (caps.value && caps.value.hit === false) return base + ' · 未收录'
  return base
})
const capsTitle = computed(() => capsError.value
  || '生成能力 / 推理档位 / 温度 / 输出上限这些旋钮由能力表决定；探针真拿到的能力会覆盖内置目录，'
  + '手填的 [llm.models] 覆盖两者')

/** api_key 只写不读：占位符照实说「已设置」（P3 起 /api/config 只回 api_key_set 布尔，不回明文）。 */
const apiKeyPlaceholder = computed(() => (form.llm.api_key_set ? '已设置（留空 = 不修改）' : 'sk-…'))
const apiKeyTitle = computed(() => (form.llm.api_key_set
  ? '已存有 key（服务端不回明文）。留空 = 不动它；要改就直接填新的'
  : '还没存 key；填一个再「获取模型」'))

/** 「跟随全局」那句灰字的细节（行内只留值，细节进 title —— 行内有 20 字预算）。 */
function inheritTitle(key) {
  return '不勾 = 这条链路不写任何覆盖键（保存时发 null，服务端删键）⇒ '
    + '输出上限 / 温度跟随上面「生成」（[llm.params]）；推理档位用 ' + defaultThinking(key)
}

// ---- 多 provider（P3）：列表 + 同一时间只展开一份（照 DSH）----
// ⚠️ **R4**：后端还没回 `providers` 字段 ⇒ `form.llm.providers` 保持 **null** ⇒ 模板走老的单配置。
//    判据是 `Array.isArray` —— 一个空数组也算"后端回了"（那就是"一份配置都没有"的正常空列表）。
const llmProviderMode = computed(() => Array.isArray(form.llm.providers))
const activeProvider = computed(() => {
  if (!llmProviderMode.value) return null
  const list = form.llm.providers
  return list.find((p) => p.id === form.llm.active) || list[0] || null
})
/** 快速配置里「API 密钥」的占位符：provider 模式看当前那份，legacy 模式看顶层。 */
const quickKeySet = computed(() => (llmProviderMode.value
  ? !!(activeProvider.value && activeProvider.value.api_key_set)
  : !!form.llm.api_key_set))
/** 快速配置的「保存并测试」：保存交给本页既有链路（quiet = 只留行内结果，不再弹 toast）。 */
function quickPersist() {
  return saveLlm({ quiet: true })
}
/** 「生成」组改的是**当前 provider 的参数面**；legacy 模式改 `[llm.params]` */
const llmParamsTarget = computed(() => (llmProviderMode.value && activeProvider.value && activeProvider.value.params)
  ? activeProvider.value.params
  : form.llm.params)
/** 能力（内置目录 / 探针）按**当前生效的** (base_url, model) 取 */
const effBaseUrl = computed(() => String((llmProviderMode.value && activeProvider.value
  ? activeProvider.value.base_url : form.llm.base_url) || ''))
const effModel = computed(() => String((llmProviderMode.value && activeProvider.value
  ? activeProvider.value.model : form.llm.model) || ''))

// API **协议**清单（请求怎么发）：后端 `/config/llm/protocols`；没有就内置只读清单并标注「内置」。
const protocols = ref([])
const protocolsSource = ref('')
const protocolsError = ref('')
async function loadProtocols() {
  try {
    const list = await fetchProtocols()
    if (Array.isArray(list) && list.length) {
      protocols.value = list; protocolsSource.value = 'server'; protocolsError.value = ''
      return
    }
    protocols.value = BUILTIN_PROTOCOLS.slice(); protocolsSource.value = 'builtin'
    protocolsError.value = '后端协议清单为空 ⇒ 用了内置只读清单'
  } catch (e) {
    protocols.value = BUILTIN_PROTOCOLS.slice(); protocolsSource.value = 'builtin'
    protocolsError.value = '协议接口不可用（' + ((e && e.message) || e) + '）⇒ 用了内置只读清单'
  }
}

/** 取这个模型的能力。失败**照实说**（404 = 服务端还没接这条接口），界面退化成「只画通用控件」。 */
async function loadCaps() {
  // ⚠️ 用**当前生效的** (base_url, model)：多 provider 模式下它们是 active 那份的
  const base = effBaseUrl.value.trim()
  const model = effModel.value.trim()
  if (!base || !model) { caps.value = null; capsSource.value = ''; capsError.value = ''; return }
  capsError.value = ''
  // 探针真拿到的能力**覆盖**内置目录那份（只覆盖它声明过的字段，见 caps.js::mergeCaps）
  const probe = probeCaps[model] || null
  try {
    const r = await fetchCatalog(base, model)
    caps.value = mergeCaps(normalizeCaps(r), probe)
    capsSource.value = probe ? 'probe' : 'catalog'
  } catch (e) {
    // 目录接口没接上/失败，但探针给过能力 ⇒ 照用探针那份（比"什么都画不出来"强），来源照实写
    if (probe) { caps.value = probe; capsSource.value = 'probe'; return }
    caps.value = null
    capsSource.value = ''
    capsError.value = (e && e.status === 404)
      ? '模型能力接口还没接入（404）—— 只显示通用控件'
      : '模型能力读取失败：' + ((e && e.message) || e)
  }
}

/**
 * provider 面板「获取模型」带回的**探针能力**（每支模型可能带 `caps`）。
 * ⚠️ 探针优先于内置表（表里查不到就老实回 null，界面上那个数才不会是别人的）——
 *    存进 `probeCaps`（同 id **覆盖**，重取就跟着变）后重算 caps；**只影响展示与滑块 max，不写盘**。
 */
function onProbeCaps(payload) {
  let changed = false
  for (const m of ((payload && payload.models) || [])) {
    if (!m || !m.id) continue
    const id = String(m.id)
    if (m.caps) { probeCaps[id] = normalizeCaps({ caps: m.caps }); changed = true }
    // 这次探针**没给**能力 ⇒ 清掉旧的那份（别缓存住上一次的，否则"探针没有"会被旧值骗过去）
    else if (probeCaps[id]) { delete probeCaps[id]; changed = true }
  }
  if (changed) loadCaps()
}

/** 「获取模型」回来了：候选由选择器自己显示；**带 caps 的那些存下来**（"能获取当然最好"），
 *  当前模型立刻用它覆盖内置目录。 */
function onModelsFetched(r) {
  for (const m of ((r && r.models) || [])) {
    if (m && m.id && m.caps) probeCaps[String(m.id)] = normalizeCaps({ caps: m.caps })
  }
  if (String(form.llm.model || '').trim()) loadCaps()
}

// base_url / model 一变就重取能力（等 400ms 再发，别边打字边打接口）
let capsTimer = null
watch(() => [effBaseUrl.value, effModel.value], () => {
  if (capsTimer) clearTimeout(capsTimer)
  capsTimer = setTimeout(loadCaps, 400)
})
onBeforeUnmount(() => { if (capsTimer) clearTimeout(capsTimer) })

/** [llm.models.<id>] 的手填行仍然保留（生成能力勾选写它的 input，R4：老 TOML 照读照写） —— 界面上的方言下拉/温度下拉已随「模型能力覆盖」整块删掉（方言由能力读取器自动判定）。 */



/** 当前模型那一行（reactive 对象；由下面的 watch 保证存在） */
const modelCapsRow = computed(() => {
  const id = String(form.llm.model || '').trim()
  return id ? (form.llm.models[id] || null) : null
})
/** 手填过没有（`input` 数组 或 三个 wire 任一）—— 来源小字与"这块留不留"都用它 */
const manualCapsOn = computed(() => {
  const row = modelCapsRow.value
  if (!row) return false
  return Array.isArray(row.input) || !!(row.thinking_wire || row.max_output_wire || row.temperature)
})
/** 勾选（生成能力）的**当前值**：手填优先 → 否则能力表/探针的值 */
const modelInputRow = computed(() => {
  const row = modelCapsRow.value
  return row && Array.isArray(row.input) ? row.input : null
})
const capsInput = computed(() => (caps.value && caps.value.input) || DEFAULT_INPUT)
const inputText = computed(() => (modelInputRow.value ? modelInputRow.value.includes('text') : capsInput.value.text))
const inputImage = computed(() => (modelInputRow.value ? modelInputRow.value.includes('image') : capsInput.value.image))
const inputDeclared = computed(() => !!(caps.value && caps.value.input && caps.value.input.declared))

/** 勾 / 取消一个输入模态：**写进手填那一行**（`[llm.models."<id>"].input`）。
 *  ⚠️ 手填优先于能力表 —— 否则用户勾了、保存后又被能力表的值盖回去（"改了没反应"）。 */
function setInputKind(kind, on) {
  const id = String(form.llm.model || '').trim()
  if (!id) return
  if (!form.llm.models[id]) {
    form.llm.models[id] = { thinking_wire: '', max_output_wire: '', temperature: '', input: null }
  }
  const row = form.llm.models[id]
  // 第一次勾：从"当前显示的"那份起算（只改用户点的那一个）
  const cur = new Set(Array.isArray(row.input) ? row.input : inputToArray(capsInput.value))
  if (on) cur.add(kind)
  else cur.delete(kind)
  row.input = ['text', 'image'].filter((k) => cur.has(k))
}
// 模型一变就把那一行补出来（值按模型 id 存，切回来还在）；四项都留空 = 保存时把这条删掉
watch(() => String(form.llm.model || '').trim(), (id) => {
  if (!id) return
  if (!form.llm.models[id]) {
    form.llm.models[id] = { thinking_wire: '', max_output_wire: '', temperature: '', input: null }
  }
}, { immediate: true })

/** 链路行：**可见列表由后端 `override_links` 给**（不含 correct / 以后加链路前端不用改）。
 *  ⚠️ **R4**：没有 `override_links`（老后端）⇒ 退回 `overrides` 的键 = 老四行（含字幕订正）。 */
const llmLinks = computed(() => {
  const ov = form.llm.overrides || {}
  const listed = (Array.isArray(form.llm.overrideLinks) && form.llm.overrideLinks.length)
    ? form.llm.overrideLinks.filter((k) => typeof k === 'string' && k)
    : Object.keys(ov)
  return listed.map((key) => ({
    key,
    label: LLM_LINK_ZH[key] || key,
    hint: LLM_LINK_HINT[key] || ''
  }))
})

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

/** 「要求 JSON 输出」的真源 = `[llm.params] json_mode`（v2 的一等公民键）；
 *  老工程把同一件事写在 `extra_params.response_format` 里 —— 那边也算"开"（否则界面会显示没开、
 *  实际却在发）。 */
const jsonModeOn = computed(() => {
  // 多 provider 模式下看**当前生效那份**的参数面（legacy 就是 [llm.params]）
  if (llmParamsTarget.value && llmParamsTarget.value.json_mode === true) return true
  const t = String(form.llm.extra_params || '').trim()
  if (!t) return false
  try {
    const o = JSON.parse(t)
    return !!(o && typeof o === 'object' && o.response_format && o.response_format.type === 'json_object')
  } catch (e) { return false }
})

/** 勾 / 取消「要求 JSON 输出」：**新位置（params.json_mode）与老位置（extra_params.response_format）
 *  一起改** —— 只改一处就会出现"勾着却没生效 / 没勾却在发"。
 *  ⚠️ 正文不是合法 JSON 时**拒绝改**（toast 说明）—— 不能为了一个开关把用户手写的逃逸口覆盖掉。 */
function setJsonMode(on) {
  const raw = String(form.llm.extra_params || '').trim()
  let obj = {}
  if (raw) {
    try { obj = JSON.parse(raw) } catch (e) {
      toast('extra_params 不是合法 JSON，先修好再改「要求 JSON 输出」', 'err'); return
    }
    if (!obj || typeof obj !== 'object' || Array.isArray(obj)) {
      toast('extra_params 必须是 JSON 对象，先修好再改「要求 JSON 输出」', 'err'); return
    }
  }
  // 多 provider 模式下写的是 active 那份的 params（legacy 模式就是 [llm.params]）
  llmParamsTarget.value.json_mode = !!on
  if (on) obj.response_format = { type: 'json_object' }
  else delete obj.response_format
  form.llm.extra_params = Object.keys(obj).length ? JSON.stringify(obj, null, 2) : ''
}

/**
 * 按链路覆盖 → patch。**不勾 = 发 null = 删键**（回到跟随全局）。
 * ⚠️ 后端这次**没回**的键（不在 `knownLinks` 里）且没勾 ⇒ 整条发 `null`（连空表都别留）——
 *    否则"后端刚列出来的 title"会写出一个空 `[llm.overrides.title]`；勾上了才写值。
 */
function buildOverrides(f) {
  const known = Array.isArray(f.knownLinks) ? f.knownLinks : Object.keys(f.overrides || {})
  const out = {}
  for (const [key, row] of Object.entries(f.overrides || {})) {
    if (!row.on && !known.includes(key)) { out[key] = null; continue }
    if (!row.on) { out[key] = { max_tokens: null, temperature: null, reasoning: null }; continue }
    // ⚠️ 值一律过**类型归一**（空串/NaN/认不出的档位 ⇒ 不写这个值）；缺的那项发 null = 删键
    //    （= 回到跟随全局）。**绝不许**把 '' 写进 patch —— 那是 P0 事故（整份配置回退默认）。
    const v = normalizeParams(row)
    out[key] = {
      max_tokens: v.max_tokens ?? null,
      temperature: v.temperature ?? null,
      reasoning: v.reasoning ?? null
    }
  }
  return out
}

/**
 * 参数面**只发改过的键**（2026-10-20 硬约束）：没改的键不发 ⇒ 后端深合并保留原值。
 * 温度 / 输出上限在 interface 面板里已不可见，若每次都发，一改名字就会把它们写回默认值。
 * ⚠️ json_mode 的 true→false 发 `null`（= 删键），不是写 false（false 会覆盖源文件里手写的那份）。
 */
function changedParams(v, orig) {
  const o = orig || {}
  const now = normalizeParams(v)
  const out = {}
  if (now.temperature !== undefined && now.temperature !== o.temperature) out.temperature = now.temperature
  if (now.max_tokens !== undefined && now.max_tokens !== o.max_tokens) out.max_tokens = now.max_tokens
  if (now.reasoning !== undefined && now.reasoning !== o.reasoning) out.reasoning = now.reasoning
  if (typeof now.json_mode === 'boolean' && now.json_mode !== o.json_mode) out.json_mode = now.json_mode ? true : null
  return out
}

/** provider 的参数面（与 `[llm.params]` 同一套字段；**类型归一 + 只发改过的键**） */
function buildProviderParams(p) {
  return changedParams(p && p.params, p && p.paramsOrig)
}

/**
 * **多 provider 模式**的 patch：`active` + 整份 `providers`（数组 = **整份替换**）+ 全局的
 * `extra_params` / `overrides` / `max_concurrent`。
 * ⚠️ **不写** legacy 的 `base_url` / `model` / `params` / `models` —— 那会把文件里原来那套盖掉。
 * ⚠️ 每条 provider 的 `api_key`：**没填就整个不发**（数组项里"空串 = 不修改"不成立，省略最稳）。
 */
function buildProviderPatch(f) {
  const llm = {
    active: f.active || '',
    max_concurrent: Math.max(1, Math.round(Number(f.max_concurrent) || 3)),
    providers: (f.providers || []).map((p) => {
      const row = {
        id: String(p.id || '').trim(),
        name: p.name || '',
        api: p.api || 'openai-chat',
        base_url: p.base_url || '',
        model: p.model || '',
        params: buildProviderParams(p.params),
        models: (p.models || [])
          .map((m) => {
            // ⚠️ **只发知道的字段**：后端 `LlmModel` 里 `reasoning` 是 `bool`（不是 Option）、
            //    `context_window/max_output` 是 Option ⇒ 发 `null` 会被 400
            //    「providers 形状不对：invalid type: null, expected a boolean」，**整份保存失败**（P0 事故）。
            const row = { id: String(m.id || '').trim() }
            if (m.name) row.name = m.name
            if (m.context_window != null) row.context_window = Math.max(0, Math.round(Number(m.context_window) || 0))
            if (m.max_output != null) row.max_output = Math.max(0, Math.round(Number(m.max_output) || 0))
            if (Array.isArray(m.input)) row.input = m.input
            if (typeof m.reasoning === 'boolean') row.reasoning = m.reasoning
            const levels = (m.levels || []).filter((l) => l && l.level)
              .map((l) => ({ level: String(l.level), wire_value: String(l.wire_value || '') }))
            if (levels.length) row.levels = levels
            return row
          })
          .filter((m) => m.id)
      }
      if (String(p.api_key || '').trim()) row.api_key = String(p.api_key).trim()
      return row
    }),
    extra_params: null,
    overrides: {}
  }
  if (f.extra_params && f.extra_params.trim()) {
    try { llm.extra_params = JSON.parse(f.extra_params) } catch (e) { throw new Error('extra_params 不是合法 JSON，请检查格式') }
  }
  llm.overrides = buildOverrides(f)
  return llm
}

function buildLlmPatch() {
  const f = form.llm
  const p = f.params || {}
  // **多 provider 模式**（P3）：数组 = 整份替换；legacy 字段一个都不写（见 buildProviderPatch）
  if (Array.isArray(f.providers)) return { llm: buildProviderPatch(f) }
  const llm = {
    base_url: f.base_url || '',
    model: f.model || '',
    max_concurrent: Math.max(1, Math.round(Number(f.max_concurrent) || 3)),
    // ⚠️ **请求参数面写 [llm.params]**（v2）：只发**改过的键**（见 changedParams）——
    // 没改的不发 = 后端保留原值；类型不对/空串在 changedParams 里已经被归一掉了。
    params: changedParams(p, f.paramsOrig)
  };
  // **能力面**（[llm.models."<id>"]）：只发**有内容的行**；四项都空的那行整条删掉（别在文件里留空表）
  const models = {}
  for (const [id, row] of Object.entries(f.models || {})) {
    // `input`：Array = 用户勾过（**空数组也是"勾过"**：表示"都不支持"），null/undefined = 没手填
    const hasInput = !!row && Array.isArray(row.input)
    if (!row || (!row.thinking_wire && !row.max_output_wire && !row.temperature && !hasInput)) {
      models[id] = null
      continue
    }
    models[id] = {
      thinking_wire: row.thinking_wire ? row.thinking_wire : null,
      max_output_wire: row.max_output_wire ? row.max_output_wire : null,
      temperature: row.temperature ? row.temperature : null,
      input: hasInput ? row.input : null
    }
  }
  if (Object.keys(models).length) llm.models = models
  // ⚠️ api_key 只写不读：**没填就整个不发这个键**。新服务端把空串当「不修改」，旧服务端却会真写一个
  //    空串（把 key 清掉）—— 省略键在两边都等于「不修改」，所以留空时不发它（契约见 api/llm.js）。
  if (String(f.api_key || '').trim()) llm.api_key = String(f.api_key).trim()
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
  llm.overrides = buildOverrides(f)
  return { llm }
}

async function loadConfig() {
  try {
    const d = await api('/api/config')
    globalRaw.value = d.global_raw || ''
    // config 解析失败时后端会给出原因，必须显式提示，否则表单只是静默显示默认值
    configError.value = d.config_error || ''
    // 留空「参考标题」时**实际会用**的那几条（服务端给，别在前端再抄一份字面量）。
    referenceTitlesDefault.value = Array.isArray(d.reference_titles_default) ? d.reference_titles_default : []
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
    // **留空 ⇒ 把内建默认值填进去**（用户口径，见 `referenceTitlesDefault` 的注释）：
    // 与其他设置项一致 —— 表单显示"实际会用的那一份"，保存时经 syncReferenceTitlesFromText
    // 写进 config.toml；清空再保存 = 回到预设。
    if (!referenceTitlesText.value.trim() && referenceTitlesDefault.value.length) {
      referenceTitlesText.value = referenceTitlesDefault.value.join('\n')
      referenceTitlesFromDefault.value = true
    } else {
      referenceTitlesFromDefault.value = false
    }
    if (c.cutting) Object.assign(form.cutting, {
      output_dir: c.cutting.output_dir || './output',
      overwrite: !!c.cutting.overwrite,
    })
    const l = d.llm_config || {}
    // v2：请求参数面在 llm_config.params（不再是 [llm] 上的 temperature/max_tokens/thinking）
    const p = l.params || {}
    const gMax = p.max_tokens ?? 65536
    const gTemp = p.temperature ?? 0
    // **链路行**：可见列表 = 后端 `override_links`（不含 correct）；缺失 ⇒ null（R4：老四行）
    const linkList = Array.isArray(l.override_links)
      ? l.override_links.filter((k) => typeof k === 'string' && k)
      : null
    const rowOf = (v) => ({
      on: v?.max_tokens != null || v?.temperature != null || !!v?.reasoning,
      max_tokens: v?.max_tokens ?? gMax,
      temperature: v?.temperature ?? gTemp,
      reasoning: normalizeLevel(v?.reasoning)
    })
    const knownLinks = Object.keys(l.overrides || {})
    const gOverrides = Object.fromEntries(Object.entries(l.overrides || {}).map(([k, v]) => [k, rowOf(v)]))
    // 后端**列了**但还没序列化出来的链路（比如刚加的 title）：补一行默认值，否则那一行没有可勾的对象
    for (const k of (linkList || [])) if (!gOverrides[k]) gOverrides[k] = rowOf(null)
    form.llm = {
      // ⚠️ api_key **不预填**（P3 起接口只回 api_key_set 布尔；老服务端回明文也不填进表单）——
      //    留空 = 不修改，填了才覆盖（见 buildLlmPatch）
      api_key: '',
      api_key_set: (typeof d.api_key_set === 'boolean') ? d.api_key_set : !!l.api_key,
      base_url: l.base_url || '',
      model: l.model || '',
      max_concurrent: l.max_concurrent ?? 3,
      // 老配置里的 reasoning 可能是 "enabled"/"disabled"（v1 的 thinking 迁过来的）——
      // 归一成等价档位再显示；写回的是规范档位名（服务端两套都认，见 caps.js::normalizeLevel）
      params: {
        temperature: gTemp,
        max_tokens: gMax,
        reasoning: normalizeLevel(p.reasoning),
        json_mode: p.json_mode === true
      },
      // 保存时只发**和这里不一样**的键（没改的不发 ⇒ 后端保留原值）
      paramsOrig: {
        temperature: gTemp,
        max_tokens: gMax,
        reasoning: normalizeLevel(p.reasoning),
        json_mode: p.json_mode === true
      },
      // 能力面：整份带过来（只编辑当前模型那一行，其余原样回写）
      // `input` 用 **null** 表示"没手填"（`[]` 是"手填成都不勾"，两者不能混）
      models: Object.fromEntries(
        Object.entries(l.models || {}).map(([id, m]) => [
          id,
          {
            thinking_wire: m?.thinking_wire || '',
            max_output_wire: m?.max_output_wire || '',
            temperature: m?.temperature || '',
            input: Array.isArray(m?.input) ? [...m.input] : null
          }
        ])
      ),
      // **多 provider**（P3）：只有后端真回了这个字段才进新界面；没有 ⇒ null（R4：老的单配置）
      active: typeof l.active === 'string' ? l.active : '',
      providers: Array.isArray(l.providers)
        ? l.providers.map((pr) => ({
            id: String(pr.id || ''),
            name: pr.name || String(pr.id || ''),
            api: pr.api || 'openai-chat',
            base_url: pr.base_url || '',
            // api_key 只写不读：服务端只回 api_key_set 布尔
            api_key: '',
            api_key_set: pr.api_key_set === true,
            builtin: pr.builtin === true,
            model: pr.model || '',
            params: {
              temperature: pr.params?.temperature ?? 0.3,
              max_tokens: pr.params?.max_tokens ?? 65536,
              reasoning: normalizeLevel(pr.params?.reasoning),
              json_mode: pr.params?.json_mode === true
            },
            paramsOrig: {
              temperature: pr.params?.temperature ?? 0.3,
              max_tokens: pr.params?.max_tokens ?? 65536,
              reasoning: normalizeLevel(pr.params?.reasoning),
              json_mode: pr.params?.json_mode === true
            },
            models: (pr.models || []).map((m) => ({
              id: m.id || '',
              name: m.name || '',
              context_window: m.context_window ?? null,
              max_output: m.max_output ?? null,
              input: Array.isArray(m.input) ? [...m.input] : null,
              reasoning: typeof m.reasoning === 'boolean' ? m.reasoning : null,
              levels: (m.levels || []).map((lv) => ({ level: lv.level || '', wire_value: lv.wire_value || '' }))
            }))
          }))
        : null,
      extra_params: l.extra_params ? JSON.stringify(l.extra_params, null, 2) : '',
      // 覆盖表：键 = 后端回了的那几个（+ override_links 里列了但还没回的先补默认）
      overrides: gOverrides,
      overrideLinks: linkList,
      knownLinks,
    }
    loaded.value = true
    loadFailed.value = false
    configLoadError.value = ''
  } catch (e) {
    loaded.value = false
    loadFailed.value = true
    configLoadError.value = (e && e.message) || ''
  }
}


async function toggleSource() {
  if (!editSource.value) {
    await loadConfig()
    editSource.value = true
  } else {
    editSource.value = false
  }
}

/** 文本框（每行一条）→ `form.detection.reference_titles`。
 *  ⚠️ **两页共用一个函数**（程序配置页的保存 + LLM 页的保存都要它）：
 *  抄两份必然漂移，而漂移的表现是"在某一页改的没生效"。 */
function syncReferenceTitlesFromText() {
  form.detection.reference_titles = referenceTitlesText.value
    .split('\n')
    .map(s => s.trim())
    .filter(Boolean)
}

async function saveGlobal() {
  if (!loaded.value) { toast('配置未加载成功，已禁止保存', 'err'); return }
  try {
    syncReferenceTitlesFromText()
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

/** 保存失败的原因（**常驻**显示在页面顶部，不是一闪就没的 toast —— P0：400 被吞过，用户以为存上了） */
const llmSaveError = ref('')
/** 保存后回读的**落盘校验**结果（不一致时把差异写出来） */
const llmSaveWarn = ref('')

/** 保存后回读一次：界面上每份配置的模型数与服务端回读的条数必须一致（"刷新还是 0"的通用护栏） */
async function verifyLlmSaved(expect) {
  try {
    const d = await api('/api/config')
    const got = (d && d.llm_config && d.llm_config.providers) || []
    const diffs = []
    for (const want of expect) {
      const row = got.find((p) => p && p.id === want.id)
      const n = (row && Array.isArray(row.models)) ? row.models.length : 0
      if (n !== want.n) diffs.push(want.id + '：界面 ' + want.n + ' 条 / 落盘 ' + n + ' 条')
    }
    llmSaveWarn.value = diffs.length ? '保存后回读不一致：' + diffs.join('；') : ''
  } catch (e) {
    llmSaveWarn.value = '保存后回读失败：' + ((e && e.message) || e)
  }
}

/**
 * 保存 LLM 配置（llm.toml + 提示词所在的 config.toml）。
 *
 * @param {{ quiet?: boolean }} [opts] quiet = 不弹成功/失败 toast（快速配置把结果写在自己的行内），
 *   错误横幅 `llmSaveError` 与回读校验 `llmSaveWarn` 照旧（**不许静默**）。
 * @returns {Promise<{ok: boolean, error?: string}>} 快速配置要先保存成功才测连接，所以必须回结果。
 */
async function saveLlm(opts = {}) {
  const quiet = opts && opts.quiet === true
  if (!loaded.value) {
    const msg = '配置未加载成功，已禁止保存'
    if (!quiet) toast(msg, 'err')
    return { ok: false, error: msg }
  }
  llmSaveError.value = ''
  llmSaveWarn.value = ''
  try {
    const patch = buildLlmPatch()
    // 期望落盘的模型数（多配置时逐份比；legacy 模式没有 providers 就不比）
    const expect = ((patch.llm && patch.llm.providers) || []).map((p) => ({ id: p.id, n: (p.models || []).length }))
    await putLlm(patch)
    // ⚠️ **提示词那一组写的是 config.toml 的 [detection]**，不是 llm.toml ——
    //    它只是**摆在**这一页（用户要求），归属没变。所以这一页的保存必须**两个文件都存**，
    //    否则用户在这里改了参考标题、点「保存」看着成功、实际一个字节都没落盘。
    syncReferenceTitlesFromText()
    await api('/api/config/global', { method: 'PUT', body: JSON.stringify({ patch: buildGlobalPatch() }) })
    await verifyLlmSaved(expect)
    if (!quiet) toast(llmSaveWarn.value ? 'LLM 配置已保存（但回读不一致，见顶部提示）' : 'LLM 配置已保存')
    await loadConfig()
    return { ok: true }
  } catch (e) {
    // **不许静默失败**：把服务端的 error 原文留在页面上
    const msg = (e && e.message) || String(e)
    llmSaveError.value = '保存失败：' + msg
    if (!quiet) toast(e.message, 'err')
    return { ok: false, error: msg }
  }
}

onMounted(async () => {
  // /api/config 只有程序配置与 LLM 两个子页用（含源码编辑器）；模型列表只有「账号与模型」用。
  // 其余子页的组件自带接口，别白拉一遍。
  if (props.section === 'config' || props.section === 'llm') loadConfig()
  // API 协议清单只服务多 provider 的「API 协议」下拉（失败退内置清单，界面上标注）
  if (props.section === 'llm') loadProtocols()
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
/* 表单密度：字段是一行「标签·滑块·数字框」，行高本来就矮，留白按行距给。
   extra_params 与「源文件」并排（各 50%）：JSON 正文不需要更宽，再宽也是换行看不清。 */
textarea.llm-extra { min-height:230px; resize:vertical; font-size:12px; line-height:1.6; }
.llm-cols { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr); gap:10px 20px; align-items:start; }
/* LLM 块：内容列压到 ~760px（min() 兜住窄屏），**左对齐**；容器查询量的是内容列而不是视口 */
/* **所有分页共用的内容列**（程序配置 / LLM / 账号 / 系统集成 / 用量与日志）：
   min(760px,100%) + 左对齐 ⇒ 切标签时左边缘不跳；container-type 让下面的 @container 量的是**内容列**而不是视口。 */
.settings-block { max-width:min(760px, 100%); container-type:inline-size; }
/* 内容列 ≤820px 时：链路覆盖四条改成**单列四行**（两栏各 ~370 太挤，硬塞会腰斩滑条） */
@container (max-width: 820px) {
  .settings-block .llm-cols { grid-template-columns:minmax(0,1fr); gap:8px 0; }
}
@media (max-width:1200px) { .llm-cols { grid-template-columns:minmax(0,1fr); gap:14px 0; } }
/* 按链路覆盖：一条链路一格（两栏 = 2×2）—— [勾选框 + 链路名] + 该链路的参数控件（LlmParamFields）。
   没勾的只留一行灰字（跟随全局），勾开才长出控件。控件的出现与否由**模型能力**决定。
   ⚠️ **半宽列里的控件必须跨满整行**：并排的实测结果是滑条被压到 **59px**（1280px 下只剩 41px），
   刻度根本看不见。下面这条 `:deep(.slf)` 是判据钉着的不变式
   （webui/scripts/check-layout-invariants.mjs ①②，都带变异自证）—— 动它之前先读那个脚本。 */
.llm-cols .llm-default-row {
  grid-column:1 / -1;
  display:grid; grid-template-columns:minmax(0,1fr); gap:2px 10px; align-items:center;
  padding-bottom:6px; margin-bottom:2px; border-bottom:1px dashed var(--line-soft);
}
.llm-cols .llm-link-row {
  display:grid; grid-template-columns:minmax(0,1fr) max-content;
  gap:2px 10px; align-items:center;
  /* 一张轻卡片：4 条链路才看得出是"4 个格子"（以前只有 10px 间隙，看着像 4 个孤立勾选） */
  border:1px solid var(--line-soft); border-radius:8px; padding:6px 8px;
}
.llm-cols .llm-link-row .ln-name { grid-column:1; }
.llm-cols .llm-link-row :deep(.slf),
.llm-cols .llm-link-row .ln-inherit { grid-column:1 / -1; }
/* LlmParamFields 整块也跨满整行（它内部再排「档位 / 温度 / 输出上限」） */
.llm-cols .llm-link-row :deep(.lpf) { grid-column:1 / -1; }
.ln-name { display:flex; align-items:center; gap:6px; font-size:12.5px; white-space:nowrap; cursor:pointer; }
.ln-name input { margin:0; }
.ln-text { min-width:0; }
.ln-hint { font-size:11px; font-weight:400; color:var(--ink-faint); }
.ln-inherit { font-size:11.5px; overflow-wrap:anywhere; }
/* 能力读取失败：照实说（红色常驻，不是一闪就没的 toast） */
.caps-err { color:var(--pink-text); }
.form-section { padding:8px 0; border-bottom:1px dashed var(--line-soft); }
.form-section:first-child { padding-top:0; }
.form-section:last-child { border-bottom:none; padding-bottom:0; }
.form-section-title { font-weight:700; font-size:13px; margin-bottom:6px; }
/* 标题后面的**副标题**：与标题拉开距离、字更淡、不加粗 —— 以前是 .muted small 紧贴着标题，
   读起来像"生成能力：内置目录（未收录）"这样一个长名字（用户找不到主次）。 */
.t-sub { margin-left:8px; font-weight:400; font-size:11.5px; color:var(--ink-faint); }
/* ⚠️ 用**容器查询**按内容列宽自动降列（以前是 6 列 + 视口媒体查询：内容列压到 760 后 6 列只剩 120px/格，
   SliderField 的滑条（格子 1/3）会被压到 40px 以下）。列数：内容列 ≥1100 才 6 列；≥700 两列；更窄一列。 */
.form-grid { display:grid; grid-template-columns:minmax(0,1fr); gap:8px 12px; align-items:start; }
@container (min-width: 700px) { .settings-block .form-grid { grid-template-columns:repeat(2, minmax(0,1fr)); } }
@container (min-width: 1100px) { .settings-block .form-grid { grid-template-columns:repeat(6, minmax(0,1fr)); } }
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
/* 留空「参考标题」时的内建示例预览（只读灰字，**不是**输入框）*/
.rt-preview { margin-top:6px; padding:6px 8px; border-radius:4px;
  background:var(--bg-soft); border:1px dashed var(--line-soft); }
.rt-list { margin:4px 0 0; padding-left:16px; }
.rt-list li { font-size:11.5px; line-height:1.6; color:var(--ink-faint); }
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
}
@media (max-width: 900px) {
  textarea.llm-extra { min-height:160px; }
  /* 链路覆盖格：窄屏让链路名独占一行（控件本来就跨满整行） */
  .llm-cols .llm-link-row { grid-template-columns:minmax(0,1fr); }
  .llm-cols .llm-link-row .ln-name { grid-column:1 / -1; }
  .span-2 { grid-column:1 / -1; }
  .prompt-pair, .prompt-with-side { grid-template-columns:1fr; }
}
</style>

