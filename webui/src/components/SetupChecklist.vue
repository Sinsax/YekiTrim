<template>
  <div class="card card-blk mb-12 setup-card">
    <!-- 全部就绪时只留这一行（「开始使用 · 环境就绪」），点一下仍能展开逐项看。 -->
    <button class="sc-head" type="button" data-cap="home.setup-checklist"
            :aria-expanded="open ? 'true' : 'false'" @click="toggle">
      <Icon name="check" :size="15" class="sc-ic" />
      <b class="sc-title">{{ head || headText }}</b>
      <span v-if="!loading && !allReady" class="chip a">{{ unknown ? '待检查' : missingCount + ' 项待处理' }}</span>
      <span class="flex-1"></span>
      <span v-if="!loading" class="muted small">{{ open ? '收起' : '展开' }}</span>
      <Icon :name="open ? 'chevron-down' : 'chevron-right'" :size="13" />
    </button>

    <div v-show="open && !loading" class="sc-rows">
      <!-- ① 运行环境：ffmpeg / ffprobe / 出片底座（原「运行状况」卡的实测 chips 并进这一行） -->
      <div class="sc-row" :class="{ ok: envReady }">
        <span class="sc-dot" :class="envReady ? 'g' : 'a'"></span>
        <div class="sc-main">
          <b class="sc-name">运行环境</b>
          <div v-if="!health" class="sc-sub">
            没读出来 ·
            <button class="linklike" type="button" @click="emit('refresh')">重新检查</button>
          </div>
          <!-- 实测 chips：数据源从 /api/doctor 换成 /api/health（同一批命令，一样是现场实测）。
               ⚠️ 必须逐项可见 —— 不许再出现"卡片说 N 项待处理、展开却看不出缺哪项"的错位
               （2026-11-15 用户报的就是它：底座缺失在运行状况卡里没有任何展示）。 -->
          <div v-else class="sc-chips" title="由服务端在当前进程里实测，不是缓存的旧值">
            <span class="chip" :class="health.ffmpeg ? 'g' : 'n'">ffmpeg {{ health.ffmpeg ? '就绪' : '缺失' }}</span>
            <span class="chip" :class="health.ffprobe ? 'g' : 'n'">ffprobe {{ health.ffprobe ? '就绪' : '缺失' }}</span>
            <span class="chip" :class="health.dhampir_ready ? 'g' : 'n'">出片底座 {{ health.dhampir_ready ? '就绪' : '缺失' }}</span>
          </div>
          <div v-if="envMissingNames.length" class="sc-sub" title="装好缺的外部命令后点「重新检查」">
            缺 {{ envMissingNames.join('、') }}
          </div>
          <div v-if="health && !health.dhampir_ready" class="sc-sub" :title="dhampirHint">
            底座没装 · 放到程序目录 <span class="mono">dhampir/bin/</span>
          </div>
        </div>
        <span v-if="health" class="chip" :class="envReady ? 'g' : 'a'">{{ envReady ? '就绪' : '待处理' }}</span>
        <span v-else class="chip n">未读取</span>
      </div>

      <!-- ② 字幕模型：模型清单与下载（原「运行状况」卡的 ModelList）并进这一行，缺哪个下哪个 -->
      <div class="sc-row" :class="{ ok: modelsReady }">
        <span class="sc-dot" :class="modelsReady ? 'g' : 'a'"></span>
        <div class="sc-main">
          <b class="sc-name">字幕模型</b>
          <div v-if="doctorError" class="sc-sub">
            {{ doctorError }} ·
            <button class="linklike" type="button" @click="emit('refresh')">重新检查</button>
          </div>
          <div v-else-if="!doctor" class="sc-sub">
            没读出来 ·
            <button class="linklike" type="button" @click="emit('refresh')">重新检查</button>
          </div>
          <div v-else-if="!modelsReady" class="sc-sub" :title="modelsMissingHint">
            {{ installedModelCount }}/{{ modelTotal }} 已安装 · 下面可下载（仅管理员）
          </div>
          <div v-else class="sc-sub">已安装</div>
        </div>
        <span v-if="doctorError || !doctor" class="chip n">未读取</span>
        <span v-else-if="modelsReady" class="chip g">就绪</span>
        <span v-else class="chip a">待处理</span>
        <div v-if="doctor && !modelsReady" class="sc-ml">
          <div v-if="doctor.models_dir" class="sc-sub mono" :title="doctor.models_dir">模型目录：{{ doctor.models_dir }}</div>
          <ModelList :models="models" @changed="emit('refresh')" />
        </div>
      </div>

      <!-- ③ AI 模型（可选）：不配置也能用，只是可能只切出一整段 -->
      <div class="sc-row" :class="{ ok: llmReady }">
        <span class="sc-dot" :class="llmReady ? 'g' : 'n'"></span>
        <div class="sc-main">
          <b class="sc-name">AI 模型 <span class="muted small">（可选）</span></b>
          <div v-if="!health" class="sc-sub">
            没读出来 ·
            <button class="linklike" type="button" @click="emit('refresh')">重新检查</button>
          </div>
          <div
            v-else-if="!llmReady"
            class="sc-sub"
            title="不配也能识别、挑片段、出片；只是事件检测会退化成整段视频，可能只产出一条整段片段。"
          >不配也能用，可能只切整段</div>
          <div v-else class="sc-sub">已配置</div>
        </div>
        <button v-if="health && !llmReady" class="mini-btn" type="button" @click="emit('go-llm')">去配置</button>
        <span v-else-if="health" class="chip g">已配置</span>
        <span v-else class="chip n">未读取</span>
      </div>

      <!-- ④ 第一段录像：没有项目时把主行动放到这里 -->
      <div class="sc-row" :class="{ ok: hasProject }">
        <span class="sc-dot" :class="hasProject ? 'g' : 'a'"></span>
        <div class="sc-main">
          <b class="sc-name">第一段录像</b>
          <div class="sc-sub">{{ hasProject ? `已有 ${projectCount} 个项目` : '还没有录像' }}</div>
        </div>
        <button v-if="!hasProject" class="mini-btn" type="button" @click="emit('open-wizard')">添加录像</button>
        <span v-else class="chip g">已添加</span>
      </div>
    </div>
  </div>
</template>

<script setup>
// 首页「检查项」：环境与模型的**唯一**状态卡（v8 用户口径 2026-11-15：原「运行状况」卡并入这里）。
//
// 行 = 启动真正会卡住的地方：运行环境（ffmpeg/ffprobe/底座）→ 字幕模型 → AI 模型（可选）→ 第一段录像。
// ⚠️ 「读不到」不等于「缺失」：health 为 null、doctor 读失败时只标未读取 + 重新检查，别把用户吓一跳。
// ⚠️ 「运行环境」行的 chips 与原 doctor 卡同一批命令（数据源 /api/health，同样现场实测），缺哪项当场可见
//     —— 不许退回"只给一句『运行 yekitrim polish doctor 查看缺什么』"：待处理项必须在这一行里看得见。
// ⚠️ AI 模型是可选项：不配也能走完流程，只是事件检测可能只切出一整段 —— 文案要说清。
// ⚠️ 行内文案 ≤20 字，解释（缺哪些、为什么不配也行）一律进 title。
import { computed, ref, watch } from 'vue'
import Icon from './Icon.vue'
import ModelList from './ModelList.vue'

const props = defineProps({
  health: { type: Object, default: null },
  doctor: { type: Object, default: null },
  /** 「运行状况」读失败要照实报（读失败 ≠ 缺失；置了它就不许拿旧 doctor 快照冒充现状） */
  doctorError: { type: String, default: '' },
  models: { type: Array, default: () => [] },
  projectCount: { type: Number, default: 0 },
  loading: { type: Boolean, default: false },
  /** 覆写标题（首页在未就绪时传「检查项」——上面的状态卡才是主标题；就绪时传空，露出 headText） */
  head: { type: String, default: '' }
})
const emit = defineEmits(['open-wizard', 'go-llm', 'refresh'])

const open = ref(false)
const toggle = () => { open.value = !open.value }

const envMissing = computed(() => {
  const h = props.health
  if (!h) return []
  const out = []
  if (!h.ffmpeg) out.push('ffmpeg')
  if (!h.ffprobe) out.push('ffprobe')
  if (!h.dhampir_ready) out.push('出片底座')
  return out
})
/** 缺的命令名（`出片底座` 单独一句提示，不挤进"缺 ffmpeg、ffprobe"这一行）。 */
const envMissingNames = computed(() => envMissing.value.filter((n) => n !== '出片底座'))
const envReady = computed(() => !!props.health && envMissing.value.length === 0)

/** doctor 读到**且没有读失败**才算 usable：读失败时宁可「未读取」，别拿旧快照冒充现状。 */
const doctorUsable = computed(() => !!props.doctor && !props.doctorError)
const modelsMissing = computed(() =>
  (props.doctor?.models || []).filter((m) => !m.installed)
)
const modelTotal = computed(() => (props.doctor?.models || []).length)
const installedModelCount = computed(() => modelTotal.value - modelsMissing.value.length)
const modelsReady = computed(() => doctorUsable.value && modelsMissing.value.length === 0)
const modelsMissingHint = computed(() => {
  const names = modelsMissing.value.map((m) => m.name).filter(Boolean)
  const who = names.length ? `缺：${names.join('、')}。` : ''
  return who + '在下面的清单里下载（下载仅管理员可操作）。'
})
/** 底座缺失的指路（行内只留"是什么/放哪里"，细节进 title）。 */
const dhampirHint = '出片（导出成片）要靠底座出画面。发布包里的 dhampir/ 目录要和 yekitrim 程序放在一起（即 程序目录/dhampir/bin/）；想看机器上具体缺什么可运行 yekitrim polish doctor。'

const llmReady = computed(() => props.health?.llm === true)
const hasProject = computed(() => props.projectCount > 0)

/** 还有哪些没读到（读失败 ≠ 缺失：此时只能说"待检查"，不能报"N 项待处理"）。 */
const unknown = computed(() => !props.health || !doctorUsable.value)
/** 必做项缺几个（只数**确认缺**的；AI 模型是可选项，不计入；读失败不冒充缺失）。 */
const missingCount = computed(
  () => (props.health && !envReady.value ? 1 : 0)
    + (doctorUsable.value && !modelsReady.value ? 1 : 0)
    + (hasProject.value ? 0 : 1)
)
/** 全部就绪（AI 是可选行，不计入"能不能开工"）：收成一行"开始使用 · 环境就绪"。 */
const allReady = computed(() => envReady.value && modelsReady.value && hasProject.value)

const headText = computed(() => {
  if (props.loading) return '正在检查环境…'
  return allReady.value ? '开始使用 · 环境就绪' : '开始使用'
})

// 检查完成时：有问题就展开，全就绪就收成一行。用户手动收起后，只有状态再变才会重开。
watch(
  () => [props.loading, allReady.value],
  () => {
    if (!props.loading) open.value = !allReady.value
  },
  { immediate: true }
)
</script>

<style scoped>
.setup-card { padding-top:10px; padding-bottom:10px; }
.sc-head {
  display:flex; align-items:center; gap:8px; width:100%; padding:0;
  background:none; border:none; text-align:left; cursor:pointer; color:inherit;
}
.sc-head:hover .sc-title { color:var(--main-deep); }
.sc-ic { color:var(--main); flex:none; }
.sc-title { font-size:13px; }
.sc-rows { display:flex; flex-direction:column; gap:2px; margin-top:8px; }

.sc-row {
  display:flex; align-items:center; gap:9px;
  padding:7px 8px; border-radius:7px; flex-wrap:wrap;
}
.sc-row + .sc-row { border-top:1px dashed var(--line-soft); }
.sc-row.ok .sc-name { color:var(--ink); }
.sc-dot { width:7px; height:7px; border-radius:50%; flex:none; background:var(--ink-faint); }
.sc-dot.g { background:var(--green); }
.sc-dot.a { background:var(--amber-deep); }
.sc-main { flex:1 1 auto; min-width:0; display:flex; flex-direction:column; gap:2px; }
.sc-name { font-size:12.5px; }
.sc-sub { font-size:11.5px; color:var(--ink-dim); line-height:1.5; overflow-wrap:anywhere; }
.sc-sub.mono { font-family:var(--mono, ui-monospace, SFMono-Regular, Menlo, monospace); font-size:11px; }
.linklike {
  padding:0; border:none; background:none; cursor:pointer;
  color:var(--pink-text); font-size:11.5px; text-decoration:underline;
}
.sc-row .chip, .sc-row .mini-btn { flex:none; }
/* 运行环境行的实测 chips（原「运行状况」卡同款）；换行照 flex-wrap 走 */
.sc-chips { display:flex; flex-wrap:wrap; gap:6px; }
/* 字幕模型行内嵌的模型清单（ModelList）：独占一整行 */
.sc-ml { flex:1 1 100%; min-width:0; padding-top:2px; }
</style>
