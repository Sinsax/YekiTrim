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
      <!-- ① 运行环境：ffmpeg / ffprobe / 出片底座 -->
      <div class="sc-row" :class="{ ok: envReady }">
        <span class="sc-dot" :class="envReady ? 'g' : 'a'"></span>
        <div class="sc-main">
          <b class="sc-name">运行环境</b>
          <div v-if="!health" class="sc-sub">
            没读出来 ·
            <button class="linklike" type="button" @click="emit('refresh')">重新检查</button>
          </div>
          <template v-else-if="!envReady">
            <div v-if="envMissingNames.length" class="sc-sub">缺 {{ envMissingNames.join('、') }}</div>
            <div v-if="!health.dhampir_ready" class="sc-sub mono">
              运行 yekitrim polish doctor 查看缺什么
            </div>
          </template>
          <div v-else class="sc-sub">ffmpeg、ffprobe、出片底座都就绪</div>
        </div>
        <span v-if="health" class="chip" :class="envReady ? 'g' : 'a'">{{ envReady ? '就绪' : '待处理' }}</span>
        <span v-else class="chip n">未读取</span>
      </div>

      <!-- ② 字幕模型：doctor 里的模型清单（下载入口在下方「运行状况」） -->
      <div class="sc-row" :class="{ ok: modelsReady }">
        <span class="sc-dot" :class="modelsReady ? 'g' : 'a'"></span>
        <div class="sc-main">
          <b class="sc-name">字幕模型</b>
          <div v-if="!doctor" class="sc-sub">
            没读出来 ·
            <button class="linklike" type="button" @click="emit('refresh')">重新检查</button>
          </div>
          <div v-else-if="!modelsReady" class="sc-sub" :title="modelsMissingHint">
            {{ installedModelCount }}/{{ modelTotal }} 已安装
          </div>
          <div v-else class="sc-sub">已安装</div>
        </div>
        <button
          v-if="doctor && !modelsReady"
          class="mini-btn"
          type="button"
          :title="modelsMissingHint"
          @click="emit('open-doctor')"
        >去下载</button>
        <span v-else class="chip" :class="doctor ? 'g' : 'n'">{{ doctor ? '就绪' : '未读取' }}</span>
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
// 首页「开始使用」检查：纯展示组件 —— 只读 props、只 emit，不自己发请求。
//
// 行 = 启动真正会卡住的地方：运行环境（ffmpeg/ffprobe/底座）→ 字幕模型 → AI 模型（可选）→ 第一段录像。
// ⚠️ 「读不到」不等于「缺失」：health/doctor 为 null 时只标未读取 + 重新检查，别把用户吓一跳。
// ⚠️ AI 模型是可选项：不配也能走完流程，只是事件检测可能只切出一整段 —— 文案要说清。
// ⚠️ 行内文案 ≤20 字，解释（缺哪些、为什么不配也行）一律进 title。
import { computed, ref, watch } from 'vue'
import Icon from './Icon.vue'

const props = defineProps({
  health: { type: Object, default: null },
  doctor: { type: Object, default: null },
  models: { type: Array, default: () => [] },
  projectCount: { type: Number, default: 0 },
  loading: { type: Boolean, default: false },
  /** 覆写标题（首页把它降级成「检查项」——上面的状态卡才是主标题） */
  head: { type: String, default: '' }
})
const emit = defineEmits(['open-doctor', 'open-wizard', 'go-llm', 'refresh'])

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

const modelsMissing = computed(() =>
  (props.doctor?.models || []).filter((m) => !m.installed)
)
const modelTotal = computed(() => (props.doctor?.models || []).length)
const installedModelCount = computed(() => modelTotal.value - modelsMissing.value.length)
const modelsReady = computed(() => !!props.doctor && modelsMissing.value.length === 0)
const modelsMissingHint = computed(() => {
  const names = modelsMissing.value.map((m) => m.name).filter(Boolean)
  const who = names.length ? `缺：${names.join('、')}。` : ''
  return who + '在下方「运行状况」里下载（下载仅管理员可操作）。'
})

const llmReady = computed(() => props.health?.llm === true)
const hasProject = computed(() => props.projectCount > 0)

/** 还有哪些没读到（读失败 ≠ 缺失：此时只能说"待检查"，不能报"N 项待处理"）。 */
const unknown = computed(() => !props.health || !props.doctor)
/** 必做项缺几个（只数**确认缺**的；AI 模型是可选项，不计入）。 */
const missingCount = computed(
  () => (props.health && !envReady.value ? 1 : 0)
    + (props.doctor && !modelsReady.value ? 1 : 0)
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
</style>
