<template>
  <div>
    <div v-if="loadError" class="error-banner"><Icon name="alert" :size="15" /> {{ loadError }}</div>

    <div class="grid cols-main">
      <!-- 左：播放器 + 字幕轨道 + 切片标记 -->
      <div>
        <div class="card player-card">
          <div class="row spread mb-8">
            <b class="t m-0">原视频</b>
            <span class="muted small mono">{{ inputName || '未记录输入文件' }}</span>
          </div>
          <div v-if="src" class="video-shell">
            <video
              ref="videoRef"
              controls
              playsinline
              preload="metadata"
              class="preview-video"
              :src="src"
              @timeupdate="currentTime = $event.target.currentTime"
              @play="playing = true"
              @pause="playing = false"
              @loadedmetadata="onMeta"
            ></video>
            <!-- 字幕叠层：与精修预览同一套样式（72px@1920 缩放、同色同描边） -->
            <div v-if="activeSub" class="cap-overlay">{{ activeSub.text }}</div>
          </div>
          <div v-else class="empty small" title="先在项目设置里填写 input_file，或跑一次音频提取">暂无输入视频</div>
          <div class="row mt-8">
            <button class="mini-btn" @click="togglePlay">{{ playing ? '暂停' : '播放' }}（空格）</button>
            <span class="muted small mono">{{ fmtTime(currentTime) }}</span>
            <!-- 总秒数：切片的入点/出点填的就是秒，读数直接跟着播放位置走（用户 2026-10-06 要求） -->
            <span class="muted small mono cur-secs" title="当前播放位置的总秒数（切片的入点/出点填的就是这个）">{{ fmtSecs(currentTime) }}</span>
          </div>
        </div>

        <div class="card mt-12">
          <div class="row spread mb-6">
            <b class="t m-0">字幕轨道</b>
            <span class="muted small" :class="{ err: saveState === 'error' }">{{ saveHint }}</span>
          </div>
          <SubtitleTrack
            :segments="segments"
            :duration="duration"
            :current-time="currentTime"
            :mark-in="markIn"
            :mark-out="markOut"
            :ranges="sliceRows"
            :active-range="activeRow"
            @save="onTrackSave"
            @seek="seekTo"
          />
        </div>
      </div>

      <!-- 右：预处理 + 切片（动作集中在一列；切片块紧贴预处理下方，别占左列的竖高） -->
      <div class="side-col">
        <div class="card card-blk">
          <b class="t">预处理</b>
          <div class="row" style="gap:6px;flex-wrap:wrap;margin:8px 0 10px">
            <span class="chip" :class="chipClass('transcribe')" :title="transcribeChipTitle">转写 {{ chipText('transcribe') }}</span>
            <span class="chip" :class="chipClass('detect')">事件检测 {{ chipText('detect') }}</span>
          </div>

          <!-- ASR 模式：写进本工程 `yekitrim.toml` 的 `[asr] mode`（merge 时覆盖全局配置，
               接口走 `PUT /api/project-data/config {patch}` —— 保留注释与其他 section）。
               **两种模式各只能出一次产物**（用户要求"输出一次绑定一次、只能两次"）：
               跑过的模式槽就占了（`assets.slots`，与后端 `assets::slot_bound` 同口径），
               那一格的「跑 ASR」置灰；两格都占满时模式下拉也失去意义（一并置灰）。 -->
          <div class="asr-row" :title="asrTip">
            <span class="asr-name">ASR 模式</span>
            <select
              v-model="asrMode"
              class="asr-select"
              :disabled="!canPickAsr || busy"
              @change="saveAsrMode"
            >
              <option value="hybrid">hybrid · 更准（双模型）</option>
              <option value="single">single · 更快（单模型）</option>
            </select>
            <button
              class="mini-btn flex-none"
             
              :disabled="!canRunAsr || busy"
              :title="asrRunTitle"
              @click="runAsr"
            >{{ canRunAsr ? '跑 ASR' : '已跑过' }}</button>
          </div>
          <div class="hint asr-hint">{{ asrHint }}</div>
          <!-- 主播名：全局 `[detection] vtuber_name` 只是**默认值**，每个工程可以自己写一份
               （写进 `yekitrim.toml` 的 `[detection] vtuber_name`，merge 时覆盖全局）。
               ⚠️ `merge` 对**空串不覆盖** ⇒ 清空就是"恢复跟随全局"，不需要额外的开关。 -->
          <div class="asr-row">
            <span class="asr-name">主播名</span>
            <input
              v-model="vtuberName"
              class="fld-input flex-1"
              placeholder="留空 = 跟随全局"
              :disabled="busy"
              @change="saveVtuberName"
            >
            <span class="muted small" :title="vtuberHint">生效：{{ effectiveVtuber || '（未设）' }}</span>
          </div>

          <!-- 输入素材：弹幕 / 字幕都是**随时可换的输入**（生效值写进 yekitrim.toml），不是产物。
               清单式（两块的显示与交互完全一致）：
               - ASR 链路产物带 `ASR·<mode>` 标记、**不可删** —— 它就是那个模式用掉的转写名额；
               - 用户自己选的（含 admin 浏览服务器选的）可删，选中即加入列表；
               - 点「切换」立刻改生效值：弹幕决定热度评分与 clip.dm.srt，
                 字幕是事件检测的输入 —— 都要重新检测才会用上。 -->
          <div class="assets">
            <MaterialList
              :items="dmItems"
              kind="danmaku"
              :busy="busy"
              empty-text="还没选弹幕 —— 热度评分会退化成只用字幕密度，切片也不生成 clip.dm.srt"
              @pick="openPicker('danmaku')"
              @activate="activate('danmaku', $event)"
              @remove="removeItem('danmaku', $event)"
            />
            <div class="hint mt-6">{{ dmHint }}</div>
            <MaterialList
              class="mt-10"
              :items="txItems"
              kind="transcript"
              :busy="busy"
              :empty-text="txEmptyText"
              @pick="openPicker('transcript')"
              @activate="activate('transcript', $event)"
              @remove="removeItem('transcript', $event)"
            />
            <div class="hint mt-6">{{ assetsHint }}</div>
          </div>

          <!-- 转写 + 事件检测合成一个按钮：两段仍是**两个任务**依次跑（转写完再起检测），
               所以后一段失败只需再点一次，已经跑完的前一段不会重跑。
               已有字幕时它只做（重新）检测 —— 检测随时能重跑，那个形态**永不置灰**；
               没有字幕时才走转写，而转写受**模式槽**限制：当前模式跑过就置灰
               （`canRunAsr`，与上面那颗「跑 ASR」同口径），要换字幕走下面的清单，
               要再出一份就换个模式（换模式后这个按钮自己就活了）。 -->
          <button
            class="btn btn-primary run-btn"
            :disabled="busy || (!hasTranscript && !canRunAsr)"
            :title="prepareTitle"
            @click="runPrepare"
          >
            <span v-if="busy" class="spinner"></span>
            {{ prepareLabel }}
          </button>

          <div v-if="job.id" class="job-box mt-12">
            <div class="row spread small">
              <b>{{ kindText(job.kind) }}</b>
              <span class="chip" :class="statusClass(job.status)">{{ statusText(job.status) }}</span>
            </div>
            <div class="muted small mt-2">{{ job.message || '运行中' }}</div>
            <div v-if="job.total > 0" class="progress mt-6"><div :style="{ width: jobPct + '%' }"></div></div>
            <button class="mini-btn mt-6" @click="openJobs()">查看任务</button>
          </div>

          <div class="hint mt-8">{{ prepareHint }}</div>
        </div>

        <!-- 切片：一组入点/出点 → 填标题 →「切片」即建事件并开始剪切（没填标题会打开表单）。
             多组 = **分段剪、合并输出**（粗剪拓展 2026-10-06）。 -->
        <div class="card card-blk mt-12">
          <b class="t">切片</b>
          <!-- 多段（粗剪拓展 2026-10-06）：一组入点/出点 = 一条片段；**多组 = 分段剪、合并输出**。
               段间空隙里"有内容"的部分会被丢掉，无字幕的保留（交给后处理「去静音」变屏蔽区间）。
               点一行选中它，「标记入点/出点」落到选中那一行；单行时与老行为完全等价。 -->
          <div class="slice-rows mt-6">
            <!-- ⚠️ **整行都可点选**（含输入框）：点/聚焦任一格都把它设为"正在编辑的那一段"。
                 以前输入框上挂了 `@click.stop`，于是"改完数字再点标记入点"落到**别人**身上 ——
                 用户报的「多段切片没法回过头来处理」就是这个（判据 ㉔ 的变异钉着它）。
                 ⚠️ **还没标记的行不是错误态**（用户 2026-10-30 报「红框空白条目 0 0 0.0s ×」）：
                 空行的入/出点留空（占位符）、时长「—」、段号按钮中性色；段落色（`segColor`）
                 表达的是"这一段在字幕轨上占哪儿"，没有区间时不该有颜色。 -->
            <div v-for="(r, i) in sliceRows" :key="i" class="slice-row" :class="{ on: i === activeRow }"
                 :title="'编辑第 ' + (i + 1) + ' 段（入点/出点只作用于选中的这段）'"
                 @click="activeRow = i">
              <button class="seg-pick" :class="{ on: i === activeRow }" :title="'选这一段来标记入点/出点'"
                      :style="{ borderColor: segColor(i).line, color: segColor(i).line,
                                background: i === activeRow ? segColor(i).fill : 'transparent' }"
                      @click.stop="activeRow = i">{{ i + 1 }}</button>
              <!-- ⚠️ 属性顺序**别动**：`v-model.number="r.start"` 必须在行尾、下一行紧接 `:title=`
                   （判据 ㉔ 的变异台就锚在这个形状上 —— 把 placeholder 插到中间会让"挂回 @click.stop"
                   那条变异抓不住）。 -->
              <input class="fld-input slice-time" type="number" step="0.1" min="0" placeholder="入点"
                     v-model.number="r.start"
                     :title="'第 ' + (i + 1) + ' 段入点（秒）'" @focus="activeRow = i">
              <input class="fld-input slice-time" type="number" step="0.1" min="0" placeholder="出点"
                     v-model.number="r.end"
                     :title="'第 ' + (i + 1) + ' 段出点（秒）'" @focus="activeRow = i">
              <span class="mono small flex-none">{{ hasSpan(r) ? fmtDur(Number(r.end) - Number(r.start)) : '—' }}</span>
              <button class="mini-btn flex-none" :disabled="sliceRows.length <= 1" title="删掉这一段"
                      @click.stop="removeSliceRow(i)">×</button>
            </div>
          </div>
          <div class="row mt-6" style="gap:6px">
            <button class="mini-btn flex-1 jc-center" :title="markHint('in')" @click="mark('in')">标记入点</button>
            <button class="mini-btn flex-1 jc-center" :title="markHint('out')" @click="mark('out')">标记出点</button>
            <button class="mini-btn flex-none" title="再加一段（与已有的合成同一条片段，加完自动转到新段）" @click="addSliceRow">＋ 一段</button>
          </div>
          <div class="row spread small mt-4">
            <span class="muted">
              {{ sliceRows.length }} 段<template v-if="sliceRows.length > 1">
                · 编辑中 <b class="seg-cur-num">{{ activeRow + 1 }}</b></template>
            </span>
            <span
              v-if="sliceOverlaps.length"
              class="seg-overlap"
              :title="'第 ' + sliceOverlaps.join('、') + ' 段压在前一段上：同一段内容会被剪两遍，落盘时会自动并成一段'"
            >⚠️ 第 {{ sliceOverlaps.join('、') }} 段重叠</span>
            <span class="mono">{{ fmtDur(sliceTotal) }}</span>
          </div>
          <input
            v-model="label"
            class="fld-input mt-6"
            placeholder="片段标题"
            title="标题只用于人工查看（events.json / clip.json / 卡片展示）"
            @keyup.enter="sliceNow"
          >
          <div class="row" style="gap:6px;margin-top:8px">
            <button class="btn btn-primary flex-1 jc-center"
                    :disabled="!canSlice || busy"
                    :title="canSlice ? '按标记的起止建事件并立即开始剪切' : '每一段都要有入点和出点（出点大于入点）'"
                    @click="sliceNow">切片</button>
            <button class="mini-btn" :disabled="!canSlice" title="补类型 / 详情（标题已填则直接切）" @click="openAdd">更多设置</button>
          </div>
          <div class="hint mt-6" title="标题只用于人工查看，不影响目录与文件名；没填标题会打开表单">
            标题只用于展示<span class="mono">（clip-NN/clip.mp4）</span>
          </div>
        </div>
      </div>
    </div>

    <!-- 选择弹幕 / 字幕文件：候选来自项目目录（服务端扫），admin 还能浏览服务器任意目录。
         选中的这份会**加入清单**并立即生效（清单里随时能切回来）。 -->
    <AppModal :show="picker.show" :title="pickerTitle" wide @close="closePicker">
      <div class="muted small mb-6">
        项目目录里的候选，选中后写进
        <span class="mono">yekitrim.toml</span>。
      </div>
      <div class="file-box">
        <div v-if="!candidateList.length" class="empty small" title="管理员可以在下面换目录找">项目目录里没有匹配的文件</div>
        <div
          v-for="c in candidateList"
          :key="c.rel"
          class="file-row"
          :class="{ selected: pickerRel === c.rel }"
          @click="pickCandidate(c.rel)"
        >
          <span class="mono flex-1">{{ c.rel }}</span>
          <span v-if="c.same_stem" class="chip a">同名</span>
          <span class="muted small flex-none">{{ humanBytes(c.size) }}</span>
        </div>
      </div>

      <template v-if="isAdmin">
        <label class="mt-10">在服务器上浏览别的目录（可选）</label>
        <ServerDirPicker
          mode="file"
          :exts="pickerExts"
          :start-dir="assetsRoot"
          @update:dir="onPickerDir"
          @pick-file="pickServerFile"
        />
      </template>
      <div v-else class="hint mt-8">只能选项目目录内的文件</div>

      <div v-if="pickerPicked" class="hint mt-8">已选：<span class="mono">{{ pickerLabel }}</span></div>

      <template #footer>
        <button class="btn" @click="closePicker">取消</button>
        <button class="btn btn-primary" :disabled="!pickerPicked || picker.busy" @click="applyPick">
          <span v-if="picker.busy" class="spinner"></span>
          使用所选
        </button>
      </template>
    </AppModal>

    <!-- 手动添加片段（轨道标记的落地表单） -->
    <AppModal :show="showAdd" title="添加片段" @close="showAdd = false">
      <div class="row">
        <div class="flex-1 minw-120"><label>开始（秒）</label>
          <input v-model.number="start" type="number" step="0.1" min="0" :disabled="sliceMulti" :title="modalStartTitle">
        </div>
        <div class="flex-1 minw-120"><label>结束（秒）</label>
          <input v-model.number="end" type="number" step="0.1" min="0" :disabled="sliceMulti"
                 :title="sliceMulti ? modalStartTitle : '结束（秒）'">
        </div>
      </div>
      <div class="row mt-8">
        <div class="flex-2 minw-180"><label>标题（仅人工查看，不影响文件名）</label><input v-model="label" placeholder="片段标题"></div>
        <div class="flex-1 minw-120">
          <label>类型</label>
          <select v-model="eventType">
            <option v-for="t in EVENT_TYPES" :key="t" :value="t">{{ TYPE_ZH[t] || t }}</option>
          </select>
        </div>
      </div>
      <label class="mt-8">详情（可选）</label>
      <textarea v-model="summary" rows="2" placeholder="给片段写点备注，留空也行"></textarea>
      <template #footer>
        <button class="btn" @click="showAdd = false">取消</button>
        <button class="btn btn-primary" :disabled="!canAddClip" @click="saveClip">生成片段</button>
      </template>
    </AppModal>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Icon from '../components/Icon.vue'
import AppModal from '../components/AppModal.vue'
import ServerDirPicker from '../components/ServerDirPicker.vue'
import SubtitleTrack from '../components/prepare/SubtitleTrack.vue'
import MaterialList from '../components/prepare/MaterialList.vue'
import { projectData } from '../api/projectData'
import { humanBytes } from '../stores/project'
import { isAdmin } from '../stores/auth'
import { watchJob, kindText, statusClass, statusText, busyOn, outKey, QUEUED_HINT } from '../api/jobs'
import { useJobs } from '../composables/useJobs'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import { openJobs } from '../stores/ui'
import { segColor, EVENT_TYPES, TYPE_ZH, mediaUrl, redetectWarning, isEditableKeyEvent } from '../utils'

const route = useRoute()
const router = useRouter()
const pid = computed(() => route.params.pid || '')

const src = ref('')
const inputName = ref('')
const videoDuration = ref(0)
const videoRef = ref(null)
const currentTime = ref(0)
// 当前时间点正在显示的字幕（叠层渲染，效果对齐精修预览）
const activeSub = computed(() => segments.value.find(
  (s) => currentTime.value >= (Number(s.start) || 0) && currentTime.value < (Number(s.end) || 0)
) || null)
const playing = ref(false)
const loadError = ref('')

const segments = ref([])
/** 轨道总长：视频元数据时长与最后一条字幕取大者（视频还没加载时只能靠字幕兜底） */
const duration = computed(() => Math.max(
  videoDuration.value,
  segments.value.reduce((m, s) => Math.max(m, Number(s.end) || 0), 0)
))
const status = ref(null)
const markIn = ref(0)
const markOut = ref(0)
/** **多段切片列表**（粗剪拓展 2026-10-06）：每行一组入点/出点，剪出来是**一条**片段。
 *  多行 = 分段剪、合并输出（段间"有内容"的空隙丢掉，无字幕的保留给「去静音」）。
 *  ⚠️ 只有一行时载荷与老的「标记入点/出点」**完全一致**（不带 `ranges`）——服务端与老路径零差异。 */
/** ⚠️ 初值是 **null**，不是 0：`0` 会让"还没标记"渲染成「0 / 0 / 0.0s」——
 *  看着像一条坏掉的草稿、还带着段落色（用户 2026-10-30 报「红框空白条目」）。
 *  输入框在 null 时是空的、显示占位符，时长显示「—」，段号用中性色。 */
const sliceRows = ref([{ start: null, end: null }])
/** 「标记入点/出点」落到哪一行（点行选中）。 */
const activeRow = ref(0)
/** 列表总时长（各段内容之和；多段成片就是这个长度）。
 *  ⚠️ **重叠的段先并掉再求和**：写侧（`data_events_create_impl` → `merge_overlapping_ranges`）
 *  落盘前会把重叠/相接的段并成一段 —— 不并的话这里显示 158s、剪出来 147s，两边对不上。 */
const sliceTotal = computed(() => {
  const rs = sliceRows.value
    .map((r) => ({ start: Number(r.start) || 0, end: Number(r.end) || 0 }))
    .filter((r) => r.end > r.start)
    .sort((a, b) => a.start - b.start)
  let total = 0
  let lastEnd = -Infinity
  for (const r of rs) {
    if (r.start <= lastEnd) {
      if (r.end > lastEnd) {
        total += r.end - lastEnd
        lastEnd = r.end
      }
      continue
    }
    total += r.end - r.start
    lastEnd = r.end
  }
  return total
})
/** 被**上一段压住**（或正好接上）的行号（1 基）：落盘时会并成一段。
 *  重叠 = 同一段内容会被剪两遍，要让用户在点「切片」**之前**看见 —— 见 `merge_overlapping_ranges`。 */
const sliceOverlaps = computed(() => {
  const rs = sliceRows.value
    .map((r, i) => ({ i, start: Number(r.start) || 0, end: Number(r.end) || 0 }))
    .filter((r) => r.end > r.start)
    .sort((a, b) => a.start - b.start)
  const out = []
  for (let k = 1; k < rs.length; k++) {
    if (rs[k].start <= rs[k - 1].end + 1e-6) out.push(rs[k].i + 1)
  }
  return out
})
const showAdd = ref(false)
const start = ref(0)
const end = ref(10)
const label = ref('')
const summary = ref('')
const eventType = ref('other')

const job = reactive({ id: '', kind: '', status: '', current: 0, total: 0, message: '' })
let jobClose = null
let saveTimer = null

// 字幕自动保存状态（编辑内容/起止都即时落盘，用状态文案代替每次 toast 刷屏）
const saveState = ref('')      // '' | 'saving' | 'saved' | 'error'
const saveError = ref('')
const saveAt = ref('')
const saveHint = computed(() => {
  if (saveState.value === 'saving') return '保存中…'
  if (saveState.value === 'error') return '保存失败：' + saveError.value
  if (saveState.value === 'saved') return '已保存 ' + saveAt.value
  return '点块在上方属性条改文本与入点 / 出点，改动自动保存'
})

const jobRunning = computed(() => job.status === 'running' || job.status === 'queued')
const jobPct = computed(() => (job.total > 0 ? Math.round((job.current * 100) / job.total) : 0))
const runningStage = computed(() => (jobRunning.value ? jobStage.value : ''))
const jobStage = computed(() => {
  const k = job.kind || ''
  if (k === 'stage_detect') return 'detect'
  if (k === 'stage_transcribe' || k === 'pipeline') return 'transcribe'
  return ''
})
// 项目级作用域（`p{pid}:out`）：转写/检测/订正/剪切都写 output/*，互相排队。
// 别的项目在跑、或某个片段在精修，都不会让这个按钮变灰（并行能力）。
const { jobs: allJobs, reload: reloadJobs } = useJobs()
const scopeBusyJob = computed(() => busyOn(allJobs.value, [outKey(pid.value)]))
const busy = computed(() => jobRunning.value || !!scopeBusyJob.value)
const busyText = computed(() => (jobRunning.value ? kindText(job.kind) : scopeBusyJob.value ? kindText(scopeBusyJob.value.kind) : ''))
const hasTranscript = computed(
  () => segments.value.length > 0 || (status.value?.files || []).some((f) => String(f).startsWith('字幕'))
)
/** 字幕清单为空时那句话。**必须区分"清单没登记"与"真的没有"**（2026-10-30 实测报的）：
 *  左栏字幕轨明明读到了 6 段、右边却写「还没有字幕」—— 因为生效值是 `output/transcript.json`
 *  而清单（`[project.materials].transcript`）里一条都没登记。两句话指向的下一步完全不同：
 *  前者"重新检测照常生效"，后者才需要去跑 ASR / 选文件。 */
const txEmptyText = computed(() => (segments.value.length > 0
  ? `清单里没登记 —— 当前用 transcript.json（${segments.value.length} 段）`
  : '还没有字幕 —— 用上面的「跑 ASR」生成，或自己选一份字幕文件（srt / json）'))
/** 已有检测结果（`status.files` 里带「事件」那一项）→ 重新检测要二次确认 */
const hasEvents = computed(() => hasFile('事件'))
/** 转写成功后自动接着跑检测（两段之间不留手动步骤） */
const chainDetect = ref(false)

// ---- 输入素材（弹幕 / 字幕）：清单式，生效值写进 `yekitrim.toml`，随时可换 ----
// 声明放在 ASR 块之前：ASR 槽位来自同一份 `assets`（下面 `asrSlots` 要读它）。
const assets = ref(null)
const assetDirty = ref(false)
const assetsRoot = computed(() => assets.value?.root_dir || '')

// ---- ASR 模式（本工程 `yekitrim.toml` 的 `[asr] mode`）----
// 显示的是 **merged 配置** 的值（项目覆盖全局）：`single` = 只用 SenseVoice 直出，
// `hybrid` = SenseVoice 打轴 + qwen3-asr 填词（默认）。fallback 与
// `config/types.rs::default_asr_mode()` 一致，读失败时别显示成 single。
const asrMode = ref('hybrid')
/** 本工程自己写的主播名（空 = 跟随全局）与其合并后的生效值 */
const vtuberName = ref('')
const effectiveVtuber = ref('')
const vtuberHint = computed(() =>
  vtuberName.value
    ? '本工程自己设的值（写在 yekitrim.toml 的 [detection] vtuber_name）'
    : '当前用的是全局默认值；填一个就只对本工程生效'
)
/** 每个 ASR 模式的槽是否已占 —— **服务端算的**（`assets::slot_bound`：清单里登记过该模式
 *  产物，或盘上已有 `output/transcript.<mode>.json`）。前端不从 `items` 自己推：
 *  判槽还要看盘上那些没登记过的老产物，抄一份到前端就会漂。 */
const asrSlots = computed(() => assets.value?.slots || null)
const slotUsedOf = (mode) => !!asrSlots.value?.[mode]
/** 当前选中模式的槽：占了就不能再跑这个模式（后端对 `stage=transcribe` 也按它 409） */
const slotUsed = computed(() => slotUsedOf(asrMode.value))
/** 还有空槽吗：两格都占满时模式下拉也失去意义（跑不了了，改它只是空转） */
const slotFree = computed(() => !asrSlots.value || Object.values(asrSlots.value).some((v) => !v))
const canPickAsr = computed(() => slotFree.value)
/** 「跑 ASR」的置灰条件 —— 「只能两次绑定、用尽置灰」就落在这一条上 */
const canRunAsr = computed(() => !slotUsed.value)
const asrRunTitle = computed(() => (canRunAsr.value
  ? `按「${asrMode.value}」模式跑一次 ASR：产物登记进下面的字幕列表并置为生效字幕（每种模式各一次）`
  : `「${asrMode.value}」模式的 ASR 已经跑过（产物在字幕列表里）—— 想再跑一次就换另一个模式`))
const asrTip = computed(() => {
  if (!canRunAsr.value) return `「${asrMode.value}」模式的 ASR 已经跑过，不能再跑（每种模式各一次）`
  return asrMode.value === 'hybrid'
    ? 'hybrid = SenseVoice 打轴 + qwen3-asr 填词（需要 qwen3-asr 模型 879 MB，缺失时转写前自动下载）'
    : 'single = 只用 SenseVoice（模型 165 MB）'
})
const asrHint = computed(() => {
  if (!slotFree.value) {
    return '两种模式的 ASR 都跑过了（每种各一次）—— 字幕列表里就是全部 ASR 产物，只能在它们和你自己选的字幕之间切换。'
  }
  if (slotUsed.value) return `「${asrMode.value}」模式的产物已经在字幕列表里了；想再出一份就换个模式再跑。`
  // 两个模式的差别 + 速度/精度（速度按实测：qwen3 阶段占整条转写约八成，hybrid ≈ 5 倍于 single）
  return 'hybrid：SenseVoice 打轴 + qwen3-asr 填词，更准、耗时约 5 倍；single：只用 SenseVoice，快，字幕质量略低。'
})

// ---- 输入素材（弹幕 / 字幕）----
const dm = computed(() => assets.value?.danmaku || null)
const tx = computed(() => assets.value?.transcript || null)
/** 两类素材的**清单**（服务端 `AssetStatus.items`：ASR 产物在前，用户素材在后） */
const dmItems = computed(() => dm.value?.items || [])
const txItems = computed(() => tx.value?.items || [])
const picker = reactive({ show: false, kind: 'danmaku', rel: '', dir: '', name: '', busy: false })
const pickerTitle = computed(() => (picker.kind === 'danmaku' ? '选择弹幕文件' : '选择字幕文件'))
/** 弹幕只认 xml/ass/srt；字幕认 srt/json（与后端 `assets::classify` 同一张表） */
const pickerExts = computed(() => (
  picker.kind === 'danmaku' ? ['xml', 'ass', 'srt'] : ['srt', 'json']
))
const candidateList = computed(() => assets.value?.candidates?.[picker.kind] || [])
/** 候选（项目内相对路径）或服务器浏览选中的文件，二者取其一 */
const pickerRel = computed(() => picker.rel || '')
const pickerPicked = computed(() => picker.rel || picker.name)
const pickerLabel = computed(() => (
  picker.name ? `${picker.dir}\\${picker.name}` : picker.rel
))

const dmHint = computed(() => {
  if (!assets.value) return ''
  if (!dm.value?.value) return '现在没在用弹幕 —— 热度评分退化成只用字幕密度，切片也不生成 clip.dm.srt。'
  return '弹幕决定热度评分与 clip.dm.srt；「切换」立即改选择，重新检测后才用上。'
})
const assetsHint = computed(() => {
  if (!assets.value) return '素材读取失败（不影响预处理，重进页面可再试）'
  if (assetDirty.value) return '素材已切换 —— 点下面的按钮重新检测才会生效。'
  return '字幕是事件检测的输入：ASR 产物每种模式各一条（带 ASR 标记、不可删），自己选的随时能删。'
})
const transcribeChipTitle = computed(() => {
  const used = Object.keys(asrSlots.value || {}).filter((m) => slotUsedOf(m))
  if (!used.length) return '还没有 ASR 产物：用「跑 ASR」生成，或自己选一份字幕文件'
  return `ASR 产物已出（${used.join(' / ')}）—— 每种模式各一次，要再跑就换模式`
})

const prepareLabel = computed(() => {
  if (runningStage.value === 'transcribe') return `转写中 ${jobPct.value}%`
  if (runningStage.value === 'detect') return `检测中 ${jobPct.value}%`
  if (!hasTranscript.value) return '开始预处理'
  if (!hasEvents.value) return '开始事件检测'
  return '重新检测'
})
const prepareTitle = computed(() => {
  if (!hasTranscript.value && !canRunAsr.value) {
    return `「${asrMode.value}」模式的 ASR 已经跑过（产物在字幕列表里）—— 从列表里选一份字幕，或换个模式再跑`
  }
  if (!hasTranscript.value) return '依次执行：转写 → 事件检测（转写会自动先提取音频）'
  if (!hasEvents.value) return '转写已完成，接着跑事件检测'
  return '重新检测：整份覆盖 events.json（已剪片段不受影响）'
})
const prepareHint = computed(() => {
  if (runningStage.value) return '正在跑，完成后按钮会回到可用状态。'
  if (!hasTranscript.value) {
    if (!canRunAsr.value) {
      return `「${asrMode.value}」模式的 ASR 已经出过产物，就在上面的字幕列表里 —— 选一份（或换个模式再跑），才有字幕可检测。`
    }
    return '一次点击依次跑：转写 → 事件检测。两段是两个任务，后一段失败再点一次即可，已完成的转写不会重跑。'
  }
  if (!hasEvents.value) return '转写已完成，接着跑事件检测（会整份覆盖 events.json，已剪片段不受影响）。'
  return '字幕和弹幕都随时能换（上面列表里切换）—— 换了之后点这个按钮重新检测生效。'
})
const canSlice = computed(
  () => sliceRows.value.length > 0
    && sliceRows.value.every((r) => Number(r.end) > Number(r.start)),
)
/** 多段（>1 行且都合法）时，模态框的起止由列表决定。 */
const sliceMulti = computed(() => sliceRows.value.length > 1 && canSlice.value)
const canAddClip = computed(
  () => !!(label.value.trim() && (sliceMulti.value || end.value > start.value)),
)

/** 秒 → 紧凑时长（列表里用：12.5s / 1m02s）。 */
function fmtDur(t) {
  const v = Math.max(0, Number(t) || 0)
  if (v < 60) return `${v.toFixed(1)}s`
  const m = Math.floor(v / 60)
  return `${m}m${String(Math.round(v % 60)).padStart(2, '0')}s`
}

function fmtTime(t) {
  const v = Math.max(0, Number(t) || 0)
  const m = Math.floor(v / 60)
  const s = (v % 60).toFixed(1)
  return `${String(m).padStart(2, '0')}:${s.padStart(4, '0')}`
}

/** 同一个时刻的**总秒数**（1 位小数，与切片入点/出点输入框的精度一致）。
 *  用户口径 2026-10-06：「09:36.5 需要显示秒数，方便定位」—— 时钟负责"读数"，
 *  秒数负责"填数"（列表里那两个框要的就是它）。 */
function fmtSecs(t) {
  return (Math.max(0, Number(t) || 0)).toFixed(1) + 's'
}

/** chips：由 status.files + 运行中任务推导（待开始 / 进行中 % / ✓）。 */
function hasFile(prefix) {
  return (status.value?.files || []).some((f) => String(f).startsWith(prefix))
}
function chipText(key) {
  if (runningStage.value === key) return `${jobPct.value}%`
  // 转写这一格看的是**字幕文件**：音频只是转写的副产品，光有 wav 不算转写完。
  if (key === 'transcribe') return hasTranscript.value ? '✓' : '待开始'
  return hasFile('事件') ? '✓' : '待开始'
}
function chipClass(key) {
  const t = chipText(key)
  if (t === '✓') return 'g'
  if (t.endsWith('%')) return 'p'
  return 'n'
}

async function load() {
  loadError.value = ''
  try {
    const st = await projectData.status(pid.value)
    status.value = st
    inputName.value = st.input_file || ''
    src.value = mediaUrl(pid.value, st.input_file)
  } catch (e) {
    loadError.value = '项目状态读取失败：' + (e?.message || '')
  }
  // ASR 模式：读 **merged** 配置（项目 `yekitrim.toml` 覆盖全局）—— 这页要显示的是
  // 「这个工程实际会用的」那个值，所以不能用 `/api/config` 的 global-only
  try {
    const d = await projectData.config(pid.value, '读取项目配置')
    asrMode.value = d.config?.asr?.mode || 'hybrid'
    // 本工程**自己**写的那份（空 = 跟随全局）与合并后的生效值，两个都要
    vtuberName.value = d.project_config?.detection?.vtuber_name || ''
    effectiveVtuber.value = d.config?.detection?.vtuber_name || ''
  } catch {}
  try {
    const d = await projectData.transcript(pid.value, '读取字幕')
    segments.value = (d.segments || []).slice().sort((a, b) => (Number(a.start) || 0) - (Number(b.start) || 0))
  } catch (e) {
    // ⚠️ **404 = 还没有转写**（服务端 `data_transcript_impl` 在没有转过写时就是 404），那是正常空态 ——
    // 但其它错误（500 / 解析失败）**不许**伪装成"没有字幕"：那会把"读取失败"显示成干净的空轨道，
    // 用户唯一的下一步就是点「开始预处理」重跑一遍转写（可能覆盖掉盘上真实存在的那份）。
    segments.value = []
    if (e?.status !== 404) loadError.value = '字幕读取失败：' + (e?.message || e)
  }
  saveState.value = ''
  await loadAssets()
}

/** 读输入素材（弹幕 / 字幕）的当前值与候选。读不到不该挡住整个页面，只少一块信息。 */
async function loadAssets() {
  try {
    assets.value = await projectData.assets(pid.value, '读取输入素材')
  } catch {
    assets.value = null
  }
}

function openPicker(kind) {
  picker.kind = kind
  picker.rel = ''
  picker.name = ''
  picker.dir = assetsRoot.value
  picker.show = true
}

function closePicker() {
  picker.show = false
  picker.rel = ''
  picker.name = ''
}

/** 选一个候选（项目内相对路径） */
function pickCandidate(rel) {
  picker.rel = rel
  picker.name = ''
}

/**
 * 在服务器目录里选中的文件：只把**目录与文件名分开发给服务端**，前端不拼路径。
 * 服务端回传的目录可能是 `\\?\F:\a` 这种 verbatim 形式，前端拼成 `\\?\F:\a/b`
 * 在 Windows 上是 ERROR_INVALID_NAME（首页目录选择器同款教训）。
 */
function pickServerFile(name) {
  picker.name = name
  picker.rel = ''
}

/** 换了浏览目录就把已选的服务器文件清掉（否则会提交"新目录 + 旧文件名"）。 */
function onPickerDir(dir) {
  picker.dir = dir
  picker.name = ''
}

/** 提交选择：服务端写前会解析一遍（坏文件不落盘），返回条数供提示。 */
async function applyPick() {
  if (!pickerPicked.value || picker.busy) return
  const body = picker.name ? { dir: picker.dir, name: picker.name } : { path: picker.rel }
  picker.busy = true
  try {
    const r = await projectData.setAsset(pid.value, picker.kind, body)
    const n = Number(r?.count) || 0
    toast(picker.kind === 'danmaku' ? `弹幕已载入：${n} 条` : `字幕已载入：${n} 条`)
    closePicker()
    await load()
    assetDirty.value = true
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    picker.busy = false
  }
}

/** 清单里点「切换」：把这一条置为**生效值**（选中即补登记，老产物也认）。
 *  切换后要整页 `load()` —— 字幕轨读的就是生效值，切完立刻要看到新的那条。 */
async function activate(kind, it) {
  const label = kind === 'danmaku' ? '弹幕' : '字幕'
  try {
    await projectData.setAsset(pid.value, kind, { path: it.path })
    toast(`${label}已切换：${it.name}`)
    await load()
    assetDirty.value = true
  } catch (e) {
    toast(e.message, 'err')
  }
}

/** 清单里点「删除」：**只从选择里移除**（清 `*_file` + 摘清单），不删磁盘文件。
 *  ASR 产物（文件还在）服务端一律拒 —— 它代表那个模式用掉的转写名额。 */
async function removeItem(kind, it) {
  const isDm = kind === 'danmaku'
  const tail = isDm
    ? '热度评分会退化成只用字幕密度，切片也不再生成 clip.dm.srt'
    : '没有别的字幕可选时，事件检测就跑不了'
  const ok = await confirmDialog(
    `把${isDm ? '弹幕' : '字幕'}「${it.name}」从列表里移除？`
    + (it.active ? '（它正是当前生效的那一份，移除后就不再使用；' + tail + '）' : '')
    + '\n磁盘上的文件不会删，随时能再选回来。重新检测后才生效。',
    { title: '从列表移除', danger: true }
  )
  if (!ok) return
  try {
    await projectData.removeAsset(pid.value, kind, it.path)
    toast('已移除')
    await load()
    assetDirty.value = true
  } catch (e) {
    toast(e.message, 'err')
  }
}

function togglePlay() {
  const v = videoRef.value
  if (!v) return
  if (v.paused) v.play(); else v.pause()
}

/** 视频元数据到位后才知道整场时长：轨道总长用它（没有它时靠最后一条字幕兜底）。 */
function onMeta(e) {
  videoDuration.value = Number(e?.target?.duration) || 0
}

function seekTo(t) {
  // 先记时间（轨道播放头/标记都读它），再试着拖动视频：
  // 媒体源没加载好时 `currentTime` 赋值可能被忽略甚至抛错，不能让跳转失效。
  const n = Math.max(0, Number(t) || 0)
  currentTime.value = n
  const v = videoRef.value
  if (!v) return
  try { v.currentTime = n } catch {}
}

function mark(which) {
  const t = Math.round((videoRef.value?.currentTime || currentTime.value || 0) * 10) / 10
  const row = sliceRows.value[activeRow.value] || sliceRows.value[0]
  if (which === 'in') row.start = t
  else row.end = t
  // 与老版同一套兜底：另一头没设/设反了就补 1s，免得标一个点永远是"无效段"
  if (!(Number(row.end) > Number(row.start))) {
    if (which === 'in') row.end = t + 1
    else row.start = Math.max(0, t - 1)
  }
  markIn.value = Number(row.start) || 0
  markOut.value = Number(row.end) || 0
  start.value = markIn.value
  end.value = Math.max(markOut.value, markIn.value + 1)
}

/** 「标记入点/出点」落点的悬停说明：多段时把段号说出来（按钮本身保持短文案）。 */
function markHint(which) {
  const n = activeRow.value + 1
  const what = which === 'in' ? '入点' : '出点'
  if (sliceRows.value.length <= 1) return `把当前播放位置记为第 ${n} 段的${what}`
  return `把当前播放位置记为**第 ${n} 段**的${what}（只影响选中那段；先点行/编号换段）`
}

/** 加一段（起止先取当前播放位置：标好出点才算数）。**加完自动转到最新段**（用户口径）。 */
function addSliceRow() {
  const t = Math.round((videoRef.value?.currentTime || currentTime.value || 0) * 10) / 10
  // 入点预填当前播放位置、出点留空（出点没标之前这一段本来就无效，别写成 0 假充数据）
  sliceRows.value.push({ start: t, end: null })
  activeRow.value = sliceRows.value.length - 1
}

/** 这一段是否已经是一个**有效区间**（出点 > 入点）。
 *  ⚠️ 空行（`start/end` 为 null）与"标反了"都算没有区间 —— 它们都不该显示段落色/时长。 */
function hasSpan(r) {
  return Number(r?.end) > Number(r?.start)
}


/** 删一段；只剩一段时不给删（那会变成"没有要切的东西"）。 */
function removeSliceRow(i) {
  if (sliceRows.value.length <= 1) return
  sliceRows.value.splice(i, 1)
  activeRow.value = Math.max(0, Math.min(activeRow.value, sliceRows.value.length - 1))
}

/** 列表 → 请求载荷（秒）。多段时服务端以 `ranges` 为准（`start/end` 拉成首段起、末段末）。 */
function sliceRangePayload() {
  return sliceRows.value.map((r) => ({
    start_sec: Number(r.start) || 0,
    end_sec: Number(r.end) || 0,
  }))
}

/** 轨道编辑统一入口：`next` 是整份 segments；transient（拖动中）只更新本地，落盘由拖动结束/防抖触发。 */
function onTrackSave(next, opts = {}) {
  segments.value = next
  if (opts.transient) return
  clearTimeout(saveTimer)
  saveTimer = setTimeout(saveSubs, opts.committed ? 0 : 500)
}

async function saveSubs() {
  saveState.value = 'saving'
  try {
    await projectData.saveTranscript(pid.value, segments.value)
    saveState.value = 'saved'
    saveAt.value = new Date().toLocaleTimeString('zh-CN', { hour12: false })
  } catch (e) {
    saveState.value = 'error'
    saveError.value = e.message || '未知错误'
    toast('字幕保存失败：' + (e.message || ''), 'err')
  }
}

/** 改 ASR 模式 = 立刻写进项目 `yekitrim.toml`（patch 模式：保留注释与其他 section）。
 *  只影响**还没跑**的转写 —— 两格槽都占满时才置灰（`canPickAsr`：那种情况下
 *  跑不了任何新模式，改它只是空转）。单格占满时仍可切到另一个模式去跑。 */
/** 主播名存进本工程 `yekitrim.toml` 的 `[detection] vtuber_name`（patch，保留注释）。
 *
 *  ⚠️ 清空 = 写空串 = **恢复跟随全局**（`merge` 对空串不覆盖），不是"设成空名字"。
 */
async function saveVtuberName() {
  try {
    const v = vtuberName.value.trim()
    await projectData.patchConfig(pid.value, { detection: { vtuber_name: v } })
    effectiveVtuber.value = v
    toast(v ? '主播名已保存到本工程：' + v : '主播名已清空，恢复跟随全局')
  } catch (e) {
    toast('主播名保存失败：' + (e.message || ''), 'err')
  }
}

async function saveAsrMode() {
  if (!canPickAsr.value) return
  try {
    await projectData.patchConfig(pid.value, { asr: { mode: asrMode.value } })
    toast('ASR 模式已保存：' + (asrMode.value === 'hybrid' ? 'hybrid（打轴 + 填词）' : 'single（纯 SenseVoice）'))
  } catch (e) {
    toast('ASR 模式保存失败：' + (e.message || ''), 'err')
  }
}

/** 「跑 ASR」：**只转写**（不接检测）。
 *
 *  主按钮那条链是"没字幕时转写 → 检测"，这个按钮是**换模式再出一份**的入口：
 *  跑完产物登记进字幕列表并置为生效值（服务端 `assets::bind_asr_artifact`），
 *  要不要换成它、要不要接着检测，都由用户在上面决定。
 *  同一模式跑过就不再跑（`canRunAsr`，服务端对 `stage=transcribe` 也按模式 409 兜底）。 */
async function runAsr() {
  if (busy.value) { toast('该项目正在' + busyText.value + '，请稍候'); return }
  if (!canRunAsr.value) {
    toast(`「${asrMode.value}」模式的 ASR 已经跑过 —— 换个模式再跑，或直接在字幕列表里选一份`)
    return
  }
  chainDetect.value = false
  await startStage('transcribe')
}

/** 预处理主按钮：没转写就先转写（完成后自动接检测），已有转写就（重新）检测。
 *
 *  ⚠️ 这里**不会再起第二次转写**：只要工程里已经有转写（ASR 产物或选中的外部字幕），
 *  这个按钮永远只做（重新）检测。想再出一份 ASR 产物走上面那个「跑 ASR」
 *  （每种模式各一次），想换字幕走字幕列表里的「切换」。 */
async function runPrepare() {
  if (busy.value) { toast('该项目正在' + busyText.value + '，请稍候'); return }
  if (!hasTranscript.value) {
    chainDetect.value = true
    await startStage('transcribe')
  } else {
    await startStage('detect')
  }
}

async function startStage(stage) {
  if (busy.value) { toast('该项目正在' + busyText.value + '，请稍候'); chainDetect.value = false; return }
  // 重新检测会整份覆盖 events.json（clips/ 不动，见 utils.js 的 redetectWarning）：
  // 已经有检测结果时先二次确认，别让一次误点把上次的结果冲掉。
  if (stage === 'detect' && hasEvents.value) {
    const ok = await confirmDialog(redetectWarning(0), { title: '覆盖上一次的检测结果？', danger: true })
    if (!ok) { chainDetect.value = false; return }
  }
  try {
    const r = await projectData.pipeline(pid.value, { stage, resume: 'auto' })
    attachJob(r.id, stage === 'transcribe' ? 'stage_transcribe' : 'stage_detect', !!r.queued)
    if (r.queued) toast(QUEUED_HINT)
  } catch (e) {
    chainDetect.value = false
    toast(e.message, 'err')
  }
}

function attachJob(id, kind, queued = false) {
  Object.assign(job, { id, kind, status: queued ? 'queued' : 'running', current: 0, total: 0, message: queued ? '排队中' : '已启动' })
  // `watchJob()` 返回 `{ close() }` 对象（不是函数）：当函数调会抛错，
  // 且下面的订阅就不会执行 → 任务永远停在「运行中」。
  jobClose?.close?.()
  jobClose = watchJob(id, {
    onEvent(e) {
      if (e.type === 'progress') { job.current = e.current; job.total = e.total }
      if (e.type === 'log') job.message = e.message
      if (e.type === 'stage_started') job.message = e.stage + ' 运行中'
    },
    onSummary(s) {
      job.status = s.status
      job.current = s.current
      job.total = s.total
      job.message = s.message || job.message
      if (s.kind) job.kind = s.kind
    },
    onDone: async (s) => {
      job.status = s.status
      jobClose = null
      reloadJobs() // 别等 3 秒轮询：作用域门控要立刻松开
      if (s.status === 'succeeded') toast('任务完成')
      else toast('任务失败：' + (s.error || s.message || '未知错误'), 'err')
      await load()
      // 转写成功 → 接着起检测任务（转写失败就别往下走，留着让用户处理）
      const chained = chainDetect.value && kind === 'stage_transcribe'
      // "素材已切换"那句提示只在**检测真的跑了**之后才过期：单独跑一次 ASR
      // （「跑 ASR」按钮，不接检测）没有消费用户切过的弹幕/字幕，提示要留着。
      if (s.status === 'succeeded' && (chained || kind === 'stage_detect')) assetDirty.value = false
      if (chained) {
        chainDetect.value = false
        if (s.status === 'succeeded') startStage('detect')
      }
    }
  })
  toast('已启动')
}

function openAdd() {
  start.value = markIn.value
  end.value = markOut.value > markIn.value ? markOut.value : markIn.value + 10
  showAdd.value = true
}

/** 模态框那两个输入框在多段时是不生效的（起止由列表决定）—— 用 disabled + title 说清。 */
const modalStartTitle = computed(() =>
  sliceMulti.value
    ? `多段（${sliceRows.value.length} 段）：起止由左侧列表决定，这里只显示首尾`
    : '开始（秒）',
)

/** 切片行主按钮：标题已填 → 直接建事件并剪切（一次点击）；标题空 → 打开完整表单（类型/详情）。 */
function sliceNow() {
  if (!canSlice.value) { toast('每一段都要有入点和出点（出点大于入点）'); return }
  if (!label.value.trim()) { toast('填个片段标题，或点「更多设置」补类型/详情'); openAdd(); return }
  const ranges = sliceRangePayload()
  doCreate({
    start: ranges[0].start_sec,
    end: ranges[ranges.length - 1].end_sec,
    ranges,
    label: label.value.trim(),
    summary: summary.value,
    type: eventType.value,
  })
}

async function saveClip() {
  // 多段（列表里 >1 段）⇒ 起止由列表决定，模态框那两个输入框只是单段时的编辑口
  const ranges = sliceMulti.value ? sliceRangePayload() : null
  if (!sliceMulti.value && end.value <= start.value) { toast('结束必须大于开始'); return }
  doCreate({
    start: ranges ? ranges[0].start_sec : start.value,
    end: ranges ? ranges[ranges.length - 1].end_sec : end.value,
    ranges,
    label: label.value.trim(),
    summary: summary.value,
    type: eventType.value,
  })
}

/** 建事件（**标题是唯一必填**，纯展示元数据）→ 立刻剪切 → 进切片页。
 *  目录/文件名固定 `clips/clip-NN/clip.mp4`（见 yekitrim-core cutting），标题只写进
 *  `events.json` / `clip.json(labels)` / `publish_config.json` 与界面展示。 */
async function doCreate({ start: s, end: e, ranges, label: lb, summary: sm, type: ty }) {
  if (!lb) { toast('请填写标题'); return }
  if (e <= s) { toast('结束必须大于开始'); return }
  try {
    const created = await projectData.createEvent(pid.value, {
      label: lb,
      start_sec: s,
      end_sec: e,
      event_type: ty,
      summary: sm,
      // **多段才带 `ranges`**：单段（含老路径）载荷一个字段都不变
      ...(ranges && ranges.length > 1 ? { ranges } : {})
    })
    const rank = created?.event?.rank
    if (rank) await projectData.cutOne(pid.value, rank)
    showAdd.value = false
    label.value = ''
    summary.value = ''
    toast(`已生成片段「${lb}」，正在剪切`)
    router.push(`/p/${encodeURIComponent(pid.value)}/slices`)
  } catch (e2) {
    toast(e2.message, 'err')
  }
}

function onKey(e) {
  if (isEditableKeyEvent(e)) return
  if (e.code === 'Space') {
    e.preventDefault()
    togglePlay()
  }
}

onMounted(() => {
  load()
  window.addEventListener('keydown', onKey)
})
onUnmounted(() => {
  jobClose?.close?.()
  clearTimeout(saveTimer)
  window.removeEventListener('keydown', onKey)
})
</script>

<style scoped>
.player-card { display:flex; flex-direction:column; }
.video-shell { position:relative; container-type:inline-size; }
.video-shell .preview-video { display:block; width:100%; }
/* 与精修预览字幕同参：默认 72px @1920、#dcbda0、描边 12px #403c3b（types.rs 默认值） */
.cap-overlay { position:absolute; left:5%; right:5%; bottom:15%; text-align:center; pointer-events:none;
  font-weight:700; font-size:3.75cqw; line-height:1.45; color:#dcbda0;
  -webkit-text-stroke:0.625cqw #403c3b; paint-order:stroke fill; }
.preview-video { width:100%; max-height:62vh; object-fit:contain; background:#000; border-radius:var(--r-sm); display:block; }
/* 右列：预处理 + 切片两块上下相邻，顶部对齐（不被左列的视频拉长） */
.side-col { display:flex; flex-direction:column; align-self:start; min-width:0; }
/* 切片块里的入点/出点读数：与左侧「标记」按钮同一行，占满剩余宽度。
   注意 flex-basis 必须写成 0 —— 全局 `input { width:100% }` 会让基线是整行宽，
   按钮被挤到上一行（flex-wrap 下就变成两块各占一行）。 */
.cur-secs { color:var(--ink-dim); }
.slice-time { flex:1 1 0; width:auto; min-width:0; padding:4px 8px; font-size:12px; text-align:center; }
/* 多段列表：一行一组入点/出点；点一行 = 选中它（「标记入点/出点」落到选中行） */
/* ⚠️ `--chip-bg` 在 tokens.css 里**不存在**（全仓只有这一处用它）⇒ 写 `var(--chip-bg)` 的
   `background` 会被浏览器整条丢弃，选中行的高亮从来没亮过（不报错，只有截图才看得出来）。
   在容器上补一个指向真 token 的别名：`.slice-row.on` 的写法与判据 ㉔ 的锚点都不动。 */
.slice-rows { --chip-bg: var(--main-soft); }
.slice-row { display:flex; align-items:center; gap:6px; padding:2px 4px; border-radius:6px;
  border-left:3px solid transparent; cursor:pointer; }
/* 选中的那段：底色 + **左侧色条**（光靠底色在多段时看不出"现在编辑的是哪一段"） */
.slice-row.on { background:var(--chip-bg); border-left-color:var(--pink-text); }
.slice-row .slice-time { max-width:86px; }
/* 段号 = 选段按钮（点它换段，和点整行等效） */
.seg-pick { flex:none; width:20px; height:20px; padding:0; border-radius:var(--r-full);
  border:1px solid var(--line); background:var(--card); color:var(--ink-dim);
  font-size:10.5px; line-height:1; cursor:pointer; }
.seg-pick.on { border-color:var(--pink-text); background:var(--main-soft); color:var(--pink-text); font-weight:700; }
.seg-cur-num { color:var(--pink-text); }
/* 重叠警告：这一段压在前一段上 ⇒ 同一段内容会被剪两遍（落盘时会并成一段，见
   `merge_overlapping_ranges`）—— 琥珀色，只在真重叠时出现，别做成常驻 */
.seg-overlap { color:var(--amber-deep); font-size:11px; white-space:nowrap; }
.run-btn { width:100%; justify-content:center; margin-bottom:8px; }
/* 输入素材（弹幕 / 字幕）两块清单：每块自带分隔虚线（MaterialList），这里别再画一条，
   否则第一块上面会出现双线。 */
.assets { margin-bottom:10px; }
.asr-row { display:flex; align-items:center; gap:6px; }
.asr-name { flex:none; font-size:12px; font-weight:600; color:var(--ink-dim); }
/* 选择框占满名称之外的剩余宽度（`flex-basis:0` 同 `.slice-time`：全局 `width:100%`
   会让基线是整行宽，名称被挤到下一行） */
.asr-select { flex:1 1 0; width:auto; min-width:0; font-size:12px; padding:4px 8px; }
.asr-select:disabled { opacity:.6; }
.asr-hint { margin:5px 0 9px; }
.hint { font-size:12px; color:var(--ink-dim); }
.job-box { padding:10px; border:1px dashed var(--line); border-radius:var(--r-sm); background:var(--bg-soft); }
</style>
