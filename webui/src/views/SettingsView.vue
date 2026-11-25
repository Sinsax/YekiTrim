<template>
  <div>
    <!-- 只留标题：分区说明在左侧导航里每条都有，页头再写一遍当前那条就是重复 -->
    <UiPageHeader title="设置" />

    <!-- v7：顶部 tabs → **左侧二级导航 + 右侧内容**。每项仍是一条路由（`settings-<key>`），
         切页即换 URL，可直接刷新/分享；标签与路由同源（`utils.js::SETTINGS_SECTIONS`）。
         窄屏（≤1000px）折回横向可滚动导航，`sub` 说明隐藏只留标签。 -->

    <div class="settings-wrap">
      <nav class="settings-nav" aria-label="设置分区">
        <router-link v-for="s in visibleSections" :key="s.key" :to="{ name: 'settings-' + s.key }"
                     class="settings-nav-item" :class="{ on: s.key === section }">
          <b>{{ s.label }}</b>
          <span>{{ s.sub }}</span>
        </router-link>
      </nav>
      <div class="settings-body">

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
              <label>主播名（默认值）</label>
              <input v-model="form.detection.vtuber_name" type="text"
                     placeholder="只用于产物标题，切片时不用"
                     title="只用于产物标题（`标题【名】`）：不参与切片、不参与目录命名。填错不影响已剪片段，改完刷新精修页即可；每个工程可在出片页覆盖">
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
              <!-- ⚠️ S4：「LLM 评分权重」已搬进 llm.toml 的 [llm.links.detect.run].llm_score_weight，
                   入口在「AI 模型」页的高级配置里（这一页只放 config.toml 的项）。 -->
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
        <!-- ⚠️ 「测试这个接口」在 ① 里（**不落盘也能测**：草稿进 POST /api/llm/test 的 body）；
             这里只留「保存 LLM」，且它**只写 llm.toml** —— 提示词有自己的按钮（⑤）。 -->
        <button class="btn" @click="saveLlm()" :disabled="!loaded">
          <Icon name="save" :size="13" /> 保存 LLM
        </button>
      </span>
    </div>

    <!-- ═══ L3 五块 IA（用户口径 2026-11-25，配置重构 §3.3 + §8.2）══════════════════════
         ① 当前接口 / ② 默认参数 / ③ 按链路覆盖 / ④ 模型列表 / ⑤ 提示词（+ 高级配置）。
         从上到下 = 从常用到罕见。⚠️ ⑤ 有自己的保存按钮（写的是 config.toml 的 [detection]），
         绝不与上面的「保存 LLM」共用一个 —— "一个按钮写两个文件"是这一轮要消灭的病根（§2-⑤）。 -->

    <!-- ① 当前接口：常显、占首屏（下拉 + 名称 + 地址 + 密钥 + 模型 + 能力 chip + 测试） -->
    <div class="card" data-cap="settings.llm-quick">
      <div class="form-section-title">当前接口<span class="t-sub">连哪一家 · 用哪支模型 · 先测再存</span></div>
      <LlmEndpointCard
        :loaded="loaded"
        :providers="form.llm.providers"
        :active-id="form.llm.active"
        :model-options="endpointModelOptions"
        :caps="caps"
        :caps-text="capsText"
        :caps-error="capsError"
        @update:active-id="form.llm.active = $event"
      />
    </div>

    <!-- ② 默认参数：常显。并发数从「高级 → 运行时」提上来（它是批处理速度的第一杠杆） -->
    <div class="card">
      <div class="form-section-title">默认参数<span class="t-sub">所有链路的基准；每条链路可在下面单独覆盖</span></div>
      <div class="form-grid">
        <LlmParamFields class="span-all" :caps="caps" :target="form.llm.defaults" id-prefix="llm-def"
                        :fields="['temperature', 'max_tokens', 'reasoning', 'json_mode']"
                        :json-mode="jsonModeOn" @update:jsonMode="setJsonMode" />
        <SliderField v-model="form.llm.max_concurrent" class="span-2" label="并发请求数"
                     :min="1" :max="32" :step="1" hint="对所有 LLM 调用生效（进程级旋钮）" />
      </div>
    </div>

    <!-- ③ 按链路覆盖：行 = 链路，列 = 接口/模型/温度/输出上限/推理/状态。
         未覆盖的格子显示**后端算的**生效值（/api/config/llm/effective），点开才可编辑。 -->
    <div class="card" >
      <div class="form-section-title">按链路覆盖
        <span class="t-sub">行 = 链路；未覆盖的格子是灰字实值</span></div>
      <LlmLinkTable :links="llmLinks" :rows="form.llm.links" :effective="effective"
                    :providers="form.llm.providers" :model-options="allModelOptions"
                    :levels="linkLevels" />
      <div v-if="effectiveError" class="muted small mt-4 caps-err" :title="effectiveError">
        生效值读取失败（灰字留空）—— 覆盖功能仍然可用
      </div>
    </div>

    <!-- ④ 模型列表：**独立折叠、与「高级配置」平级**（D1 的清单是常用面，不是高级项） -->
    <details class="form-adv">
      <summary data-cap="settings.llm-models">模型列表（{{ form.llm.models.length }} 支）</summary>
      <div class="card">
        <LlmModelList :models="form.llm.models" :providers="form.llm.providers"
                      :active-id="form.llm.active" :base-url="effBaseUrl" :api-key="curApiKeyDraft"
                      :dialects="dialects" @probe="onProbeCaps" />
      </div>
    </details>

    <!-- ⑤ 提示词：折叠 + **自己的保存按钮**。⚠️ 写的是 config.toml 的 [detection]（不是 llm.toml） -->
    <details class="form-adv">
      <summary data-cap="settings.llm-prompts">提示词（写入 config.toml 的 [detection]）</summary>
      <div class="card">
        <div class="form-section-title">提示词
          <span class="t-sub">写 config.toml 的 [detection] · 与「保存 LLM」互不影响</span></div>
        <div class="prompt-pair">
          <div>
            <label title="每行一条，可含 emoji；清空保存则运行时用内建示例">参考标题</label>
            <textarea v-model="referenceTitlesText" rows="8" placeholder="每行一条参考标题"
                      @input="referenceTitlesFromDefault = false"></textarea>
            <!-- 「参考标题」的两条口径（2026-10-18 用户拍板，**推翻了上一版**）：
                 ① **留空就把内建默认值填进输入框**（与其他设置项一致：表单里显示的就是
                    "实际会用的那一份"）—— 用户要改就在默认值基础上改，保存即写进 config.toml；
                 ② **清空再保存 = 回到预设**（patch_config_file 是深合并，空数组会真的清掉 ⇒
                    resolve_reference_titles 回落到内建示例）。此时下面那条灰字预览把
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
            <label>检测 system_prompt<span class="muted small">（写 llm.toml 的 [llm.links.detect.prompt]）</span></label>
            <textarea v-model="form.llm.detectPrompt.system_prompt" rows="8" placeholder="留空使用内建默认"></textarea>
          </div>
        </div>
        <div class="mt-8">
          <label>检测 prompt_template<span class="muted small">（{segments} 会被替换）</span></label>
          <textarea v-model="form.llm.detectPrompt.prompt_template" rows="12" placeholder="留空使用内建默认"></textarea>
        </div>
        <div class="row mt-12">
          <button class="btn btn-primary" :disabled="!loaded || promptSaving" @click="savePrompts()">
            <Icon name="save" :size="13" /> {{ promptSaving ? '保存中…' : '保存提示词' }}
          </button>
          <span class="muted small">只写这三项（config.toml 的 [detection]）</span>
        </div>
      </div>
    </details>

    <!-- 高级：接口管理（增删 / 改名 / 换协议）· extra_params 逃逸口。
         详细表单默认折叠 —— 隐藏 ≠ 删除；「源文件」那块仍不在这页（用户：没必要有）。 -->
    <details class="form-adv">
      <summary data-cap="settings.llm-advanced">高级配置（接口管理 / extra_params）</summary>
      <div class="card">
        <div class="form-section">
          <div class="form-section-title">接口管理
            <span class="t-sub">增删 / 改名 / 换协议；地址与密钥在「当前接口」里改</span></div>
          <LlmProviders :providers="form.llm.providers" :active-id="form.llm.active"
                        :protocols="protocols" :protocols-source="protocolsSource"
                        @update:active-id="form.llm.active = $event"
                        @add="addProvider" @remove="removeProvider" />
        </div>
        <div class="form-section">
          <div class="form-section-title">检测评分权重
            <span class="t-sub">[llm.links.detect.run] llm_score_weight</span></div>
          <div class="form-grid">
            <SliderField v-model="form.llm.detectRun.llm_score_weight" class="span-2"
                         label="LLM 评分权重" :min="0" :max="1" :step="0.05"
                         hint="0 = 只用弹幕/字幕热度（也不再发那次评分请求）" />
          </div>
        </div>
        <div class="form-section">
          <div class="form-section-title">额外请求参数
            <span class="t-sub">原样进请求体 · 优先级最低 · 已知键一律拒绝</span></div>
          <textarea v-model="form.llm.extra_params" class="mono llm-extra" spellcheck="false"
                    rows="10" placeholder="留空 = 不带额外参数"></textarea>
          <div class="muted small mt-4">
            只用于这里没有的、provider 私有的键（如 chat_template_kwargs）；temperature / max_tokens /
            response_format 请用上面的结构化设置。
          </div>
        </div>
      </div>
    </details>
    </div>
    </template>

    <!-- 接入：录制软件 / 自动入库 / B站下载 -->
    <template v-else-if="section === 'integrations'">
    <div class="settings-block" data-cap="settings.integrations">
    <div class="section-title">录播接入（biliLive-tools / 录播姬 二选一）</div>
    <div class="card">
      <RecorderConfig />
    </div>

    <div class="section-title">B站链接下载</div>
    <div class="card">
      <DownloaderConfig />
    </div>
    </div>
    </template>

    <!-- 系统：账号/邀请（单人默认隐藏） + LLM 用量 + 运行日志。
         ⚠️ 隐藏的是**多人相关块**，不是整页：用量与日志单人也要用。
         判据 = `multi_user`（默认 false = 单人）+ 原来的本地免登录位（本地模式没有会话，邀请/配额无意义）。
         两个位都在路由守卫里 await 过（`stores/health.js` 同一份缓存），首帧就是最终答案，不会先闪一下再消失。 -->
    <template v-else-if="section === 'system'">
    <div class="settings-block" data-cap="settings.account">
      <div v-if="multiUser && !localNoAuth">
        <div class="section-title">注册与邀请码</div>
        <div class="card">
          <InviteManager />
        </div>

        <div class="section-title">账号配额</div>
        <div class="card">
          <UserQuotaManager />
        </div>
      </div>
    </div>

    <!-- 修改密码（2026-11-23 用户口径：入口放系统子页，侧栏不再放）—— 与登录后横幅那条
         「立即修改」共用同一个 API（PUT /api/auth/password）；本地免登录没有会话可改，整块不渲染。 -->
    <div class="settings-block" data-cap="settings.password" v-if="!localNoAuth">
      <div class="section-title">修改密码</div>
      <div class="card">
        <div class="mb-12"><label for="spwd-old">当前密码</label><input id="spwd-old" v-model="pwdOld" type="password" autocomplete="current-password" /></div>
        <div class="mb-12"><label for="spwd-new">新密码（至少 8 位）</label><input id="spwd-new" v-model="pwdNew" type="password" autocomplete="new-password" /></div>
        <div class="mb-12"><label for="spwd-new2">确认新密码</label><input id="spwd-new2" v-model="pwdNew2" type="password" autocomplete="new-password" /></div>
        <div class="error-banner" v-if="pwdErr">{{ pwdErr }}</div>
        <div class="row mt-12">
          <button class="btn btn-primary" :disabled="pwdBusy" @click="submitPwd">{{ pwdBusy ? '保存中…' : '保存新密码' }}</button>
        </div>
      </div>
    </div>

    <div class="settings-block" data-cap="settings.monitor">
      <div class="section-title">LLM 用量</div>
      <div class="card">
        <LlmUsage />
      </div>

      <div class="section-title row spread">
        <span>运行日志</span>
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
    </div>
  </div>
</template>

<script setup>
import { ref, reactive, computed, onMounted, onBeforeUnmount, watch, nextTick } from 'vue'
import { onBeforeRouteLeave } from 'vue-router'
import UiPageHeader from '../components/ui/UiPageHeader.vue'
import { api } from '../api/client'
import { fetchCatalog, fetchEffective, fetchProtocols, saveLlm as putLlm } from '../api/llm'
import { BUILTIN_PROTOCOLS } from '../components/llm/protocols.js'
import { normalizeParams } from '../components/llm/params.js'
import { CAPS_SOURCE_ZH, mergeCaps, normalizeCaps, normalizeLevel } from '../components/llm/caps.js'
import { toast } from '../stores/toast'
import Icon from '../components/Icon.vue'
import SliderField from '../components/SliderField.vue'
import LlmEndpointCard from '../components/llm/LlmEndpointCard.vue'
import LlmLinkTable from '../components/llm/LlmLinkTable.vue'
import LlmModelList from '../components/llm/LlmModelList.vue'
import LlmParamFields from '../components/llm/LlmParamFields.vue'
import LlmProviders from '../components/llm/LlmProviders.vue'
import InviteManager from '../components/InviteManager.vue'
import UserQuotaManager from '../components/UserQuotaManager.vue'
import RecorderConfig from '../components/RecorderConfig.vue'
import DownloaderConfig from '../components/DownloaderConfig.vue'
import LlmUsage from '../components/LlmUsage.vue'
import LogConsole from '../components/LogConsole.vue'
import { SETTINGS_SECTIONS, settingsSection } from '../utils'
import { authState, changePassword } from '../stores/auth'
import { localNoAuth as localNoAuthFromServer, localNoAuthKnown, multiUser as multiUserFromServer, multiUserKnown } from '../stores/health'
import { useDisclosure } from '../composables/useDisclosure'
import { confirmDialog } from '../stores/dialog'

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

// 修改密码（系统子页）：校验与初始密码横幅那条腿同口径（不一致 / 不足 8 位在本地拦下），
// 服务端仍是最终裁判（旧密码错 → 400 原样显示）。
const pwdOld = ref('')
const pwdNew = ref('')
const pwdNew2 = ref('')
const pwdBusy = ref(false)
const pwdErr = ref('')
async function submitPwd() {
  if (pwdBusy.value) return
  pwdErr.value = ''
  if (pwdNew.value !== pwdNew2.value) { pwdErr.value = '两次输入的新密码不一致'; return }
  if (pwdNew.value.length < 8) { pwdErr.value = '新密码至少 8 个字符'; return }
  pwdBusy.value = true
  try {
    await changePassword(pwdOld.value, pwdNew.value)
    pwdOld.value = pwdNew.value = pwdNew2.value = ''
    toast('密码已修改')
  } catch (e) {
    pwdErr.value = e?.message || '修改密码失败'
  } finally {
    pwdBusy.value = false
  }
}
/**
 * 多人位（`GET /api/health.multi_user`，默认 false = 单人）—— 与 `localNoAuth` **同一份缓存**。
 * 决定「系统」页里邀请码 / 账号配额这两块渲染不渲染（隐藏 ≠ 删除：组件与接口都保留）。
 * ⚠️ 读失败/缺字段当 `false`（单人优先）—— 与 `localNoAuth` 相反，拿不准时不露多人入口。
 * ⚠️ 初值取 `multiUserKnown()`：路由守卫已经 await 过，首帧就是最终值，不会先显示再消失。
 */
const multiUser = ref(multiUserKnown())
/** 整页不显示的那些设置子页（见 utils.js 的 `localHidden` / `multiOnly` / `adminOnly`，与路由守卫同一判据） */
const visibleSections = computed(() =>
  SETTINGS_SECTIONS.filter((s) =>
    !(s.localHidden && localNoAuth.value) && !(s.multiOnly && !multiUser.value)
    && !(s.adminOnly && authState.user?.role !== 'admin')
  )
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
  // ⚠️ S4：提示词三件与评分权重已经搬进 llm.toml（见下 form.llm.detectPrompt / detectRun），
  // 这里只剩 config.toml 真正还在的 [detection] 键。
  detection: {
    danmaku_window_secs: 10,
    vtuber_name: ''
  },
  cutting: {
    output_dir: './output',
    overwrite: false,
  },
  // **v4 形状**（L1 定案；L3 界面接的就是它）：active / max_concurrent / defaults / extra_params /
  // providers[] / models[] / links{}。⚠️ 别把 v3 的顶层 api_key / base_url / model / params / overrides
  // 写回来 —— 后端 v4 里那些键**已经没人读**（写了就是静默不生效）。
  llm: {
    /** 当前用哪一份接口（= [llm] active；空 = 第一份，与后端 active_provider() 同口径） */
    active: '',
    /** 并发数：**唯一旋钮**（[llm] max_concurrent）。从「高级 → 运行时」提上来的第一屏 */
    max_concurrent: 3,
    /** 参数基准（[llm.defaults]）：所有链路的第一级；链路覆盖是第二级 */
    defaults: { temperature: 0, max_tokens: 65536, reasoning: '', json_mode: false },
    /** 载入时那份 defaults —— 保存时**只发改过的键**（没改的不发 = 后端保留原值与注释） */
    defaultsOrig: { temperature: 0, max_tokens: 65536, reasoning: '', json_mode: false },
    /** extra_params 的文本形态（JSON）。空串 = 显式发 null = 删键（合并语义下"不发"删不掉） */
    extra_params: '',
    /** [[llm.providers]]：只描述"怎么连"；api_key **只写不读**（服务端只回 api_key_set） */
    providers: [],
    /** [[llm.models]]：唯一模型清单（(provider, id) 唯一）；链路与接口只引用它的 id */
    models: [],
    /** [llm.links.<link>]：{ provider, model, temperature, max_tokens, reasoning }（null / '' = 跟随） */
    links: {},
    /** 界面可见的链路键（后端 override_links；不含 correct —— 它只隐藏不删） */
    overrideLinks: [],
    /** 检测链路的提示词（[llm.links.detect.prompt]；S4 从 config.toml 搬来） */
    detectPrompt: { system_prompt: '', prompt_template: '', reference_titles: [] },
    /** 载入时那份（保存时只发改过的键，同上） */
    detectPromptOrig: { system_prompt: '', prompt_template: '', reference_titles: [] },
    /** 检测链路的本地行为参数（[llm.links.detect.run]）—— 只有评分权重有 UI（它原来是程序配置页的滑块） */
    detectRun: { llm_score_weight: 0.3 },
    /** 刚探测到、还没加入清单的模型（只做 ① 的候选，**不落盘**） */
    probeModels: [],
    /** providers / models 的**载入快照**（保存时"没改不发"：数组是整份替换，发一次就把
     *  [[llm.providers]] / [[llm.models]] 的数组-of-表格式压成一行内联数组） */
    providersOrig: '',
    modelsOrig: ''
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
/* ⚠️ 原来这里有一张"链路默认 thinking"表（default_thinking 的镜像）——
   v4 之后它没有读者了：链路不写 reasoning 就是**不表态**，由 [llm.defaults] 与方言决定。
   别再按链路写死一档（那是 F7 双轨的另一半）。 */

/** 「生成」组标题右边那行小字：**能力来源**（内置目录 / 联网探测 / 手填）+ 有没有读到（细节挂 title）。 */
const capsText = computed(() => {
  const bits = []
  if (capsSource.value) bits.push(CAPS_SOURCE_ZH[capsSource.value] || capsSource.value)
  // ⚠️ 手填能力现在住 [[llm.models]].overrides（④ 模型列表），不再是单独的"手填"档
  // ⚠️ 别带"能力："前缀：它接在标题「生成」后面会被读成"生成能力：…"（上一版就是这样）
  if (!bits.length && capsError.value) return '能力读取失败'
  if (!bits.length) return ''
  const base = bits.join(' · ')
  if (caps.value && caps.value.hit === false) return base + ' · 未收录'
  return base
})
const capsTitle = computed(() => capsError.value
  || '生成能力 / 推理档位 / 温度 / 输出上限这些旋钮由能力面决定：内置目录打底，联网探测覆盖它，'
  + '[[llm.models]].overrides（④ 模型列表里的「手填能力」）覆盖两者')

// ---- v4：当前接口 / 参数基准 / 链路表 ----
/** 当前生效的接口：active 命中就用它，空/认不出 ⇒ 第一份（与后端 `active_provider()` 同一条口径） */
const activeProvider = computed(() => form.llm.providers.find((p) => p.id === form.llm.active)
  || form.llm.providers[0] || null)
/** 能力（内置目录 / 探针）按**当前接口的** (base_url, model) 取 */
const effBaseUrl = computed(() => String((activeProvider.value && activeProvider.value.base_url) || ''))
const effModel = computed(() => String((activeProvider.value && activeProvider.value.model) || ''))
/** ① 的密钥草稿：只写不读 —— 表单里永远是空的，填了才发（保存后由回读清掉） */
const curApiKeyDraft = computed(() => String((activeProvider.value && activeProvider.value.api_key) || ''))

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

// base_url / model 一变就重取能力（等 400ms 再发，别边打字边打接口）
let capsTimer = null
watch(() => [effBaseUrl.value, effModel.value], () => {
  if (capsTimer) clearTimeout(capsTimer)
  capsTimer = setTimeout(loadCaps, 400)
})
onBeforeUnmount(() => { if (capsTimer) clearTimeout(capsTimer) })

/* ⚠️ 原来这里有两块东西，v4 之后都搬走了（别再复活第二份）：
   · 「当前模型那一行 [llm.models.<id>]」的整套逻辑（手填行 / 输入模态勾选 / 给能力控件喂值）
     —— v4 的模型清单是**顶层数组**、手填能力住 [[llm.models]].overrides，整块住
     components/llm/LlmModelList.vue；
   · 链路行 computed（原来按 overrides 的键凑行 + 「跟随全局」的灰字文案）。 */

/** 链路行：**可见列表由后端 override_links 给**（不含 correct；后端加链路前端不用改）。 */
const llmLinks = computed(() => form.llm.overrideLinks.map((key) => ({
  key,
  label: LLM_LINK_ZH[key] || key,
  hint: LLM_LINK_HINT[key] || ''
})))

/** ① 的模型候选：属于当前接口的清单行（一条都没有就全给）+ 刚探测到、还没加入清单的 */
const endpointModelOptions = computed(() => {
  const mine = form.llm.models.filter((m) => m.id && m.provider === form.llm.active)
  const list = (mine.length ? mine : form.llm.models.filter((m) => m.id))
    .map((m) => ({ id: m.id, label: m.label || m.id }))
  for (const p of form.llm.probeModels) {
    if (p.id && !list.some((x) => x.id === p.id)) list.push({ id: p.id, label: p.label || p.id })
  }
  return list
})
/** ③ 的模型候选：**全清单**（链路可以跨接口选模型 —— D1 的全部意义之一） */
const allModelOptions = computed(() => form.llm.models.filter((m) => m.id)
  .map((m) => ({ id: m.id, provider: m.provider, label: m.label || m.id })))
/** ③ 推理下拉的档位候选：当前模型能力面的那份（空 = 只画「跟默认」） */
const linkLevels = computed(() => (caps.value && Array.isArray(caps.value.levels) ? caps.value.levels : []))

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
    // ⚠️ S4：提示词三件 / 评分权重**不住 config.toml 了**（住 llm.toml 的 [llm.links.detect.*]）。
    // 这里再写就会把已退役的键复活 —— patch 只合并不删，而文件已是 v5、迁移不会再跑。
    detection: {
      danmaku_window_secs: Number(f.detection.danmaku_window_secs) || 10,
      vtuber_name: f.detection.vtuber_name || ''
    },
    cutting: {
      output_dir: f.cutting.output_dir || './output',
      overwrite: !!f.cutting.overwrite
    },
  }
}

/** 「要求 JSON 输出」的真源 = **[llm.defaults] json_mode**（v4 的一等公民键）。
 *
 *  ⚠️ 以前它是个"视图"：勾选要同时改 `[llm.params] json_mode` 与 `extra_params.response_format`
 *  两处，否则界面显示与实际请求不一致。v4 之后 json_mode 就是一等的 bool 键，**不用再改 extra_params**
 *  —— 反过来：extra_params 里若还手写着 response_format，保存时会被已知键冲突判据拒绝（见 parseExtraParams）。
 */
const jsonModeOn = computed(() => form.llm.defaults.json_mode === true)
function setJsonMode(on) { form.llm.defaults.json_mode = !!on }

/**
 * 参数面**只发改过的键**：patch 是合并语义，没改的不发 ⇒ 后端保留原值与注释。
 *
 * ⚠️ 值一律过**类型归一**（空串 / NaN / 认不出的档位 ⇒ 不发这个键）；
 *    `json_mode` 的 true→false 发 **null**（= 删键）：写 false 会盖掉源文件里手写的那份语义。
 * ⚠️ reasoning 清空是**删键**（发 null），不是"不发"—— 两者在合并语义下是两件事。
 */
function changedParams(v, orig) {
  const o = orig || {}
  const now = normalizeParams(v)
  const out = {}
  if (now.temperature !== undefined && now.temperature !== o.temperature) out.temperature = now.temperature
  if (now.max_tokens !== undefined && now.max_tokens !== o.max_tokens) out.max_tokens = now.max_tokens
  const r = now.reasoning === undefined ? '' : now.reasoning
  if (r !== (o.reasoning || '')) out.reasoning = r === '' ? null : r
  if (typeof now.json_mode === 'boolean' && now.json_mode !== o.json_mode) out.json_mode = now.json_mode ? true : null
  return out
}

/** extra_params 里**已知的**结构化键：出现即拒绝（D4）—— 否则"手写 thinking"与"档位"会互相打架。 */
const KNOWN_EXTRA_KEYS = [
  'temperature', 'max_tokens', 'max_completion_tokens', 'top_p', 'reasoning',
  'reasoning_effort', 'thinking', 'thinking_budget', 'response_format'
]

/**
 * extra_params → 要写进 patch 的值：**空串 = null（删键）**，非空 = JSON 对象。
 * 清空必须显式发 null：合并语义下"没提到这个键"= 保持原值，用户会看到"清空后又回来了"。
 */
function parseExtraParams(raw) {
  const t = String(raw || '').trim()
  if (!t) return null
  let obj
  try { obj = JSON.parse(t) } catch (e) { throw new Error('extra_params 不是合法 JSON，请检查格式') }
  if (!obj || typeof obj !== 'object' || Array.isArray(obj)) throw new Error('extra_params 必须是 JSON 对象')
  const bad = Object.keys(obj).filter((k) => KNOWN_EXTRA_KEYS.includes(k))
  if (bad.length) {
    throw new Error('extra_params 里有已知键：' + bad.join(' / ')
      + ' —— 这些请用上面的结构化设置（温度 / 输出上限 / 推理档位 / 要求 JSON），否则两处会互相打架')
  }
  return obj
}

/**
 * 链路覆盖 → 只写非空的键；**一条都没写就发 null**（删掉这张小表，别在文件里留空表）。
 * ""/null 都表示"跟随"，所以这里只收有值的键。
 */
/** 提示词三件：全空 ⇒ `null`（= 删掉这张子表 / 回到内建默认） */
function cleanPrompt(p) {
  const o = {
    system_prompt: String((p && p.system_prompt) || ''),
    prompt_template: String((p && p.prompt_template) || ''),
    reference_titles: Array.isArray(p && p.reference_titles) ? p.reference_titles : []
  }
  const empty = !o.system_prompt.trim() && !o.prompt_template.trim() && !o.reference_titles.length
  return empty ? null : o
}

function buildLinks(f) {
  const out = {}
  for (const [key, row] of Object.entries(f.links || {})) {
    const one = {}
    if (String(row.provider || '').trim()) one.provider = String(row.provider).trim()
    if (String(row.model || '').trim()) one.model = String(row.model).trim()
    const p = normalizeParams(row)
    if (p.temperature !== undefined) one.temperature = p.temperature
    if (p.max_tokens !== undefined) one.max_tokens = p.max_tokens
    if (p.reasoning !== undefined) one.reasoning = p.reasoning
    // 检测链路还带两张**不进请求体**的子表（S4）：提示词与本地行为参数
    if (key === 'detect') {
      const pr = cleanPrompt(f.detectPrompt)
      if (pr) one.prompt = pr
      else if (cleanPrompt(f.detectPromptOrig)) one.prompt = null
      const w = Number(f.detectRun && f.detectRun.llm_score_weight)
      one.run = { llm_score_weight: Number.isFinite(w) ? Math.min(1, Math.max(0, w)) : 0.3 }
    }
    out[key] = Object.keys(one).length ? one : null
  }
  return out
}

/**
 * 模型清单 → 数组（**整份替换**）。catalog / overrides 原样带回（它们是嵌套内联表，
 * 前端只改 id / label / provider，别在这里重排它们的字段）。
 */
function buildModels(f) {
  return (f.models || []).map((m) => {
    const row = { id: String(m.id || '').trim(), provider: String(m.provider || '').trim() }
    if (String(m.label || '').trim()) row.label = String(m.label).trim()
    if (m.catalog && typeof m.catalog === 'object') row.catalog = m.catalog
    if (m.overrides && typeof m.overrides === 'object' && Object.keys(m.overrides).length) row.overrides = m.overrides
    return row
  }).filter((m) => m.id)
}

/**
 * 接口清单 → 数组（**整份替换**）。
 * ⚠️ **api_key 空 = 不发这个键**：数组整份替换，但后端 fill_blank_provider_keys 会按 id
 *    把已存的 key 填回去 —— "不发"最明确（发空串会被 drop_blank_api_key 剔掉，等价但绕）。
 * ⚠️ **api_key_env 必须带回**（"钥匙在环境变量里"那条路）：漏了 = 整份替换时把它删掉。
 */
function buildProviders(f) {
  return (f.providers || []).map((p) => {
    const row = {
      id: String(p.id || '').trim(),
      name: String(p.name || ''),
      protocol: String(p.protocol || ''),
      base_url: String(p.base_url || '').trim(),
      model: String(p.model || '').trim()
    }
    if (String(p.api_key_env || '').trim()) row.api_key_env = String(p.api_key_env).trim()
    if (String(p.api_key || '').trim()) row.api_key = String(p.api_key).trim()
    return row
  }).filter((p) => p.id)
}

/** v4 的整份 patch（**唯一写通道**：PUT /api/config/llm）。 */
function buildLlmPatch() {
  const f = form.llm
  const llm = {
    active: String(f.active || ''),
    max_concurrent: Math.max(1, Math.round(Number(f.max_concurrent) || 3)),
    defaults: changedParams(f.defaults, f.defaultsOrig),
    extra_params: parseExtraParams(f.extra_params),
    links: buildLinks(f)
  }
  // ⚠️ **数组是整份替换**（合并语义只对对象递归）⇒ 没改就**不发**：
  //    发一次会把 [[llm.providers]] / [[llm.models]] 的"数组-of-表 + 段内注释"压成一行内联数组。
  //    这条只是"少降级一次"，真正的 decor 保留要后端另开一条写法（见本页脚本顶部注释）。
  const provs = buildProviders(f)
  if (JSON.stringify(provs) !== JSON.stringify(JSON.parse(f.providersOrig || '[]'))) llm.providers = provs
  const models = buildModels(f)
  if (JSON.stringify(models) !== JSON.stringify(JSON.parse(f.modelsOrig || '[]'))) llm.models = models
  return { llm }
}

/**
 * `/api/config` 的 `llm_config` → 表单（**v4 形状**）。
 *
 * 抽出来是为了"只重读 LLM 那一半"：保存 LLM 后如果整份 reload，用户在「⑤ 提示词」里
 * 还没保存的编辑会被一起冲掉（那是"一个按钮动两个文件"留下的坑，L3 拆开后必须堵上）。
 */
function applyLlmConfig(d) {
  const l = (d && d.llm_config) || {}
  const def = l.defaults || {}
  const gTemp = def.temperature ?? 0
  const gMax = def.max_tokens ?? 65536
  // 链路行 = 后端 override_links（不含 correct；后端加链路前端不用改）
  const linkList = Array.isArray(l.override_links) ? l.override_links.filter((k) => typeof k === 'string' && k) : []
  const rows = {}
  for (const k of linkList) {
    const ov = (l.links && l.links[k]) || {}
    rows[k] = {
      provider: String(ov.provider || ''),
      model: String(ov.model || ''),
      temperature: ov.temperature ?? null,
      max_tokens: ov.max_tokens ?? null,
      // 老配置里的 reasoning 可能是 enabled/disabled（v1 的 thinking 迁过来的）⇒ 归一成档位名
      reasoning: normalizeLevel(ov.reasoning)
    }
  }
  const extra = (l.extra_params && Object.keys(l.extra_params).length)
    ? JSON.stringify(l.extra_params, null, 2)
    : ''
  // 检测链路的提示词与评分权重（S4 起住 llm.toml；[detection] 里那两个键已经没有了）
  const dp = (l.links && l.links.detect && l.links.detect.prompt) || {}
  const dprompt = {
    system_prompt: String(dp.system_prompt || ''),
    prompt_template: String(dp.prompt_template || ''),
    reference_titles: Array.isArray(dp.reference_titles) ? [...dp.reference_titles] : []
  }
  const drun = (l.links && l.links.detect && l.links.detect.run) || {}
  form.llm = {
    active: typeof l.active === 'string' ? l.active : '',
    max_concurrent: l.max_concurrent ?? 3,
    defaults: {
      temperature: gTemp,
      max_tokens: gMax,
      reasoning: normalizeLevel(def.reasoning),
      json_mode: def.json_mode === true
    },
    // 保存时只发**和这里不一样**的键（没改的不发 ⇒ 后端保留原值与注释）
    defaultsOrig: {
      temperature: gTemp,
      max_tokens: gMax,
      reasoning: normalizeLevel(def.reasoning),
      json_mode: def.json_mode === true
    },
    extra_params: extra,
    detectPrompt: { ...dprompt, reference_titles: [...dprompt.reference_titles] },
    detectPromptOrig: { ...dprompt, reference_titles: [...dprompt.reference_titles] },
    detectRun: { llm_score_weight: drun.llm_score_weight ?? 0.3 },
    providers: (Array.isArray(l.providers) ? l.providers : []).map((p) => ({
      id: String(p.id || ''),
      name: p.name || String(p.id || ''),
      protocol: p.protocol || 'openai-chat',
      base_url: p.base_url || '',
      // api_key 只写不读（服务端只回 api_key_set 布尔）—— 表单里永远是空的，填了才覆盖
      api_key: '',
      api_key_env: p.api_key_env || '',
      api_key_set: p.api_key_set === true,
      model: p.model || ''
    })),
    // 能力面原样带过来（catalog / overrides 是嵌套内联表，只整份回写、不在前端拆字段）
    models: (Array.isArray(l.models) ? l.models : []).map((m) => ({
      id: String(m.id || ''),
      provider: String(m.provider || ''),
      label: m.label || '',
      catalog: (m.catalog && typeof m.catalog === 'object') ? JSON.parse(JSON.stringify(m.catalog)) : null,
      overrides: (m.overrides && typeof m.overrides === 'object') ? JSON.parse(JSON.stringify(m.overrides)) : null
    })),
    links: rows,
    overrideLinks: linkList,
    // ⚠️ **数组的载入快照**：保存时"没改就不发"。合并语义对数组是**整份替换**，
    //    发一次就把 [[llm.providers]] / [[llm.models]] 的**数组-of-表**格式压成一行内联数组
    //    （toml_edit 的已知行为，v3 时代 providers 就这样）—— 少发一次就少一次降级。
    providersOrig: '',
    modelsOrig: '',
    // 探针候选只是"建议"，重读配置时清掉（它从来不落盘）
    probeModels: []
  }
  // 载入快照 = 刚装进表单的那两份数组**按 patch 形态**（"没改就不发"的判据）。
  // ⚠️ 必须过 build* 再比：表单里的对象还带 api_key: '' / api_key_set 这类**不上送**的字段，
  //    直接 JSON.stringify(form.llm.providers) 会永远"看起来改过"（于是每次保存都降级格式）。
  form.llm.providersOrig = JSON.stringify(buildProviders(form.llm))
  form.llm.modelsOrig = JSON.stringify(buildModels(form.llm))
}

// ---- ③ 的生效值（/api/config/llm/effective）----
/** link → 生效快照（缺失 = 没取到）。**只来自后端**，前端不自己算。 */
const effective = reactive({})
const effectiveError = ref('')
/**
 * 逐链路取生效值：一条一个请求（`?link=` 是后端的契约）。
 * 失败**照实说**（表格里灰字留空 + 一行原因），不许拿 defaults 在本地凑一个假的生效值。
 */
async function refreshEffective() {
  effectiveError.value = ''
  const keys = (form.llm.overrideLinks || []).slice()
  if (!keys.length) { for (const k of Object.keys(effective)) delete effective[k]; return }
  const got = await Promise.all(keys.map(async (k) => {
    try { return [k, await fetchEffective(k)] } catch (e) { return [k, null] }
  }))
  const failed = []
  for (const [k, v] of got) {
    if (v) effective[k] = v
    else { delete effective[k]; failed.push(k) }
  }
  if (failed.length) effectiveError.value = '这些链路没取到生效值：' + failed.join(' / ')
}

// ---- 高级：接口增删（④ 与 ③ 的引用要跟着清）----
/** 新建一份接口：先把 active 切过去，否则"新建了却看不到"（① 只渲染当前那一份） */
function addProvider() {
  const used = new Set(form.llm.providers.map((p) => String(p.id || '')))
  let n = form.llm.providers.length + 1
  while (used.has('p' + n)) n++
  const id = 'p' + n
  form.llm.providers.push({
    id, name: '新接口 ' + n, protocol: 'openai-chat', base_url: '',
    api_key: '', api_key_env: '', api_key_set: false, model: ''
  })
  form.llm.active = id
}
/**
 * 删一份接口：**连带**摘掉它名下的模型与指向它的链路覆盖。
 * ⚠️ 不摘的话保存会被写通道拦下（"模型 X 的 provider 认不出"），而且是**整份 400**。
 */
function removeProvider(id) {
  const pid = String(id || '')
  form.llm.providers = form.llm.providers.filter((p) => p.id !== pid)
  form.llm.models = form.llm.models.filter((m) => m.provider !== pid)
  for (const row of Object.values(form.llm.links || {})) {
    if (row.provider === pid) row.provider = ''
  }
  if (form.llm.active === pid) form.llm.active = (form.llm.providers[0] && form.llm.providers[0].id) || ''
}

// ---- 方言注册表（④ 的「推理写法」下拉）----
const dialects = ref([])
async function loadDialects() {
  try {
    const r = await api('/api/config/llm/dialects')
    dialects.value = Array.isArray(r && r.dialects) ? r.dialects : []
  } catch (e) {
    // 只影响"手填推理写法"的候选数 —— 不改任何生效值，所以退化成空下拉即可（**不拦保存**）
    dialects.value = []
  }
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
      vtuber_name: c.detection.vtuber_name || ''
    })
    // LLM 那一半**先**装（提示词 / 参考标题现在住 llm.toml —— 下面那行要用它）
    applyLlmConfig(d)
    // ⚠️ S4：参考标题在 llm.toml 的 [llm.links.detect.prompt] ⇒ 必须在 applyLlmConfig 之后读
    referenceTitlesText.value = (form.llm.detectPrompt.reference_titles || []).join('\n')
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
    loaded.value = true
    loadFailed.value = false
    configLoadError.value = ''
    // ⚠️ 拍基线**必须等加载期的 watcher 跑完**（默认 flush:'pre' ⇒ 下一个微任务才补数据）。
    //    同步拍 = 刚进页面什么都没点，formDirty 就已经是 true，切任何页面都弹「未保存的设置改动」
    //    （2026-11-14 真机报障就是这个形态）。await nextTick() 之后补出来的那行进基线。
    await nextTick()
    snapshotFormBaseline()
    // ③ 的灰字：生效值**只有后端会算**（LlmV4::link_snapshot）—— 前端复算一遍就是第二份规则
    await refreshEffective()
  } catch (e) {
    loaded.value = false
    loadFailed.value = true
    configLoadError.value = (e && e.message) || ''
  }
}

// ---- 未保存改动守卫（U8/F3）----
// 设置页四个子页共用一个实例键（`route.name`），切子页 = 整块重挂：改了没保存，改动**无提示消失**。
// 做法与保存语义一致：loadConfig 成功后拍一份表单快照，离开前比对，不一致就 confirmDialog。
//
// ⚠️ **三份基线**（不是一份）：L3 起「保存 LLM」与「保存提示词」是两个独立动作，
//    一份基线会让"存了 LLM"顺手把提示词的未保存改动也标成已保存（或反之）。
//    三块的判定各自只覆盖自己会丢的输入：
//      · baseLlm     = form.llm（接口 / 参数 / 链路 / 模型清单 / extra_params）
//      · basePrompts = ⑤ 提示词那三件的文本（参考标题在 form 外）
//      · baseGlobal  = 程序配置页那几块 + 源码编辑器的 raw（提示词那三键**除外**：它们归 basePrompts）
let baseLlm = ''
let basePrompts = ''
let baseGlobal = ''
/** ⑤ 提示词那三件的状态串（唯一口径；快照与判定都用它） */
function promptStateJSON() {
  return JSON.stringify({
    rt: referenceTitlesText.value,
    sp: form.llm.detectPrompt.system_prompt || '',
    pt: form.llm.detectPrompt.prompt_template || ''
  })
}
/** 程序配置页那几块的状态串（**剔掉**提示词三键 —— 它们归 basePrompts） */
function globalStateJSON() {
  const det = Object.assign({}, form.detection)
  delete det.system_prompt
  delete det.prompt_template
  delete det.reference_titles
  return JSON.stringify({
    f: { asr: form.asr, detection: det, cutting: form.cutting },
    raw: globalRaw.value
  })
}
function snapshotFormBaseline() {
  baseLlm = JSON.stringify(form.llm)
  basePrompts = promptStateJSON()
  baseGlobal = globalStateJSON()
}
const formDirty = computed(() => {
  if (!loaded.value) return false
  return JSON.stringify(form.llm) !== baseLlm
    || promptStateJSON() !== basePrompts
    || globalStateJSON() !== baseGlobal
})
onBeforeRouteLeave(async () => {
  if (!formDirty.value) return true
  // ⚠️ confirmDialog 只认 { title, danger }（stores/dialog.js），按钮文案是默认的「确认/取消」。
  const ok = await confirmDialog(
    '这一页有还没保存的改动，离开就会丢失。确认丢弃并离开吗？',
    { title: '未保存的设置改动', danger: true },
  )
  return ok
})


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
  form.llm.detectPrompt.reference_titles = referenceTitlesText.value
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

/**
 * 保存后回读一次：**接口数与模型清单条数**必须与服务端回读一致（"刷新还是 0"的通用护栏）。
 * 不一致**不静默**：写进常驻的 llmSaveWarn 横幅（提示条在页面顶部）。
 */
async function verifyLlmSaved(expect) {
  try {
    const d = await api('/api/config')
    const got = (d && d.llm_config) || {}
    const diffs = []
    const provs = Array.isArray(got.providers) ? got.providers : []
    if (provs.length !== expect.providers) {
      diffs.push('接口 界面 ' + expect.providers + ' 条 / 落盘 ' + provs.length + ' 条')
    }
    const models = Array.isArray(got.models) ? got.models : []
    if (models.length !== expect.models) {
      diffs.push('模型清单 界面 ' + expect.models + ' 条 / 落盘 ' + models.length + ' 条')
    }
    for (const id of expect.ids) {
      if (!provs.some((p) => p && p.id === id)) diffs.push('接口 ' + id + ' 没落盘')
    }
    llmSaveWarn.value = diffs.length ? '保存后回读不一致：' + diffs.join('；') : ''
  } catch (e) {
    llmSaveWarn.value = '保存后回读失败：' + ((e && e.message) || e)
  }
}

/**
 * 只重读 LLM 那一半（/api/config 的其他字段一律不动）。
 * ⚠️ 这是「⑤ 提示词」拆开保存的配套：整份 reload 会把那边**没保存的编辑**一起冲掉。
 */
async function reloadLlm() {
  const d = await api('/api/config')
  applyLlmConfig(d)
  await refreshEffective()
  await nextTick()
  baseLlm = JSON.stringify(form.llm)
}

/**
 * 保存 LLM 配置（**只写 llm.toml**）。
 *
 * @param {{ quiet?: boolean }} [opts] quiet = 不弹成功/失败 toast（行内自己显示结果），
 *   错误横幅 `llmSaveError` 与回读校验 `llmSaveWarn` 照旧（**不许静默**）。
 * @returns {Promise<{ok: boolean, error?: string}>} 调用方要先保存成功才做下一步，所以必须回结果。
 *
 * ⚠️ **只写 llm.toml**：提示词在「⑤ 提示词」有自己的保存按钮（它写 config.toml 的 [detection]）。
 *    历史病根：这个按钮曾同时写两个文件 —— 用户改提示词后点这里"看着成功"，实际只落了一个文件。
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
    // 期望落盘条数按**表单**算（patch 里可能没带数组 —— 没改就不发）
    const expect = {
      providers: form.llm.providers.length,
      models: form.llm.models.length,
      ids: form.llm.providers.map((p) => p.id)
    }
    await putLlm(patch)
    await reloadLlm()
    await verifyLlmSaved(expect)
    if (!quiet) toast(llmSaveWarn.value ? 'LLM 配置已保存（但回读不一致，见顶部提示）' : 'LLM 配置已保存')
    return { ok: true }
  } catch (e) {
    // **不许静默失败**：把服务端的 error 原文留在页面上（400 曾被 toast 一闪吞掉）
    const msg = (e && e.message) || String(e)
    llmSaveError.value = '保存失败：' + msg
    if (!quiet) toast(e.message, 'err')
    return { ok: false, error: msg }
  }
}

/** ⑤ 提示词自己的保存态（与「保存 LLM」不是一件事，别合并） */
const promptSaving = ref(false)
/**
 * 保存提示词：**只写 config.toml 的 [detection]** 三键（参考标题 / system_prompt / prompt_template）。
 *
 * ⚠️ 与 saveLlm 完全独立（用户口径：独立保存按钮 + 明确标注写哪个文件）。
 * ⚠️ 保存后**不整份 reload**（那会冲掉还没保存的 LLM 编辑）：只把本块的基线换新。
 */
async function savePrompts() {
  if (!loaded.value) { toast('配置未加载成功，已禁止保存', 'err'); return }
  if (promptSaving.value) return
  promptSaving.value = true
  try {
    syncReferenceTitlesFromText()
    // ⚠️ S4：提示词**不住 config.toml 了** —— 写 llm.toml 的 [llm.links.detect.prompt]。
    // 「新建 / 清空」都靠这三个键表达：全空时发 null（= 删掉这张子表，回到内建默认）。
    const p = form.llm.detectPrompt
    const prompt = {
      system_prompt: String(p.system_prompt || ''),
      prompt_template: String(p.prompt_template || ''),
      reference_titles: Array.isArray(p.reference_titles) ? p.reference_titles : []
    }
    const empty = !prompt.system_prompt.trim() && !prompt.prompt_template.trim() && !prompt.reference_titles.length
    await putLlm({ llm: { links: { detect: { prompt: empty ? null : prompt } } } })
    form.llm.detectPromptOrig = JSON.parse(JSON.stringify(p))
    basePrompts = promptStateJSON()
    baseLlm = JSON.stringify(form.llm)
    toast('提示词已保存（llm.toml 的 [llm.links.detect.prompt]）')
  } catch (e) {
    const msg = (e && e.message) || String(e)
    llmSaveError.value = '提示词保存失败：' + msg
    toast(e.message, 'err')
  } finally {
    promptSaving.value = false
  }
}

onMounted(async () => {
  // /api/config 只有程序配置与 LLM 两个子页用（含源码编辑器）；模型列表只有「账号与模型」用。
  // 其余子页的组件自带接口，别白拉一遍。
  if (props.section === 'config' || props.section === 'llm') loadConfig()
  // API 协议清单只服务「接口管理」的协议下拉（失败退内置清单，界面上标注）
  // 方言清单只服务「模型列表 → 推理写法」的候选（失败就空下拉，不影响任何生效值）
  if (props.section === 'llm') { loadProtocols(); loadDialects() }
  // 每个子页都要：标签条靠它过滤掉本地模式 / 单人下不显示的项（store 有缓存，只有首次真发请求）。
  // ⚠️ 别挪回 `account` 分支里 —— 那样只有进账号页时标签才收缩，别处照旧显示 5 个。
  // 两个位共享同一份 `/api/health` 缓存：第二次 await 直接命中，不会多一个请求。
  localNoAuth.value = await localNoAuthFromServer()
  multiUser.value = await multiUserFromServer()
})
</script>

<style scoped>
/* ── v7 设置页布局：左侧二级导航 + 右侧内容 ── */
.settings-wrap { display:grid; grid-template-columns:196px minmax(0,1fr); gap:var(--s-5); align-items:start; }
.settings-nav { position:sticky; top:calc(var(--header-h) + 12px); display:flex; flex-direction:column; gap:2px; }
.settings-nav-item { display:flex; flex-direction:column; gap:1px; padding:7px 10px;
  border-radius:var(--r-sm); border:1px solid transparent; color:var(--ink-dim); text-decoration:none;
  transition:background-color var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease); }
.settings-nav-item b { font-size:var(--fs-base); font-weight:var(--fw-medium); }
.settings-nav-item span { font-size:var(--fs-xs); color:var(--ink-faint); line-height:1.4; }
.settings-nav-item:hover { background:var(--surface-2); color:var(--ink); }
.settings-nav-item.on { background:var(--main-soft); border-color:var(--main-glow); }
.settings-nav-item.on b { color:var(--pink-text); font-weight:var(--fw-semi); }
.settings-body { min-width:0; }
@media (max-width:1000px) {
  .settings-wrap { grid-template-columns:1fr; gap:var(--s-3); }
  .settings-nav { position:static; flex-direction:row; gap:6px; overflow-x:auto; scrollbar-width:none; }
  .settings-nav::-webkit-scrollbar { display:none; }
  .settings-nav-item { flex:none; white-space:nowrap; }
  .settings-nav-item span { display:none; }
}
/* 标签条下面紧接第一个区块标题：section-title 自带 20px 上边距，这里已经翻到新的一页了 */
.tabs { margin-bottom: 10px; }
.tabs + .section-title { margin-top: 4px; }
/* 表单密度：字段是一行「标签·滑块·数字框」，行高本来就矮，留白按行距给（8px 上下 / 12px 左右）。
   下面的 section/prompt 间距都跟着这条口径收，别再单独放大某一处。 */
/* 表单密度：字段是一行「标签·滑块·数字框」，行高本来就矮，留白按行距给。
   extra_params 与「源文件」并排（各 50%）：JSON 正文不需要更宽，再宽也是换行看不清。 */
textarea.llm-extra { min-height:230px; resize:vertical; font-size:12px; line-height:1.6; }
/* ⚠️ 原来的 .llm-cols（两栏 50/50 的链路覆盖网格）随 L3 的**链路表格**退场 ——
   表格 7 列各有语义，压成两栏只会把灰字截成省略号；窄屏改为**横滚**（LlmLinkTable 自带）。
   判据随之换成 check-layout-invariants.mjs ①②'（表格列定义 + 点开才编辑）。 */
/* LLM 块：内容列压到 ~760px（min() 兜住窄屏），**左对齐**；容器查询量的是内容列而不是视口 */
/* **所有分页共用的内容列**（程序配置 / LLM / 账号 / 系统集成 / 用量与日志）：
   min(760px,100%) + 左对齐 ⇒ 切标签时左边缘不跳；container-type 让下面的 @container 量的是**内容列**而不是视口。 */
.settings-block { max-width:min(760px, 100%); container-type:inline-size; }
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
  .span-2 { grid-column:1 / -1; }
  .prompt-pair, .prompt-with-side { grid-template-columns:1fr; }
}
</style>

