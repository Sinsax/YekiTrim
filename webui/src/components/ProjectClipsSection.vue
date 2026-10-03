<template>
  <div>
    <div v-if="loadError" class="error-banner"><Icon name="alert" :size="15" /> {{ loadError }}</div>

    <div class="sort-bar">
      <span class="muted small">排序</span>
      <button
        v-for="o in SORT_OPTIONS"
        :key="o.key"
        class="mini-btn"
        :class="{ selected: sortKey === o.key }"
        :title="sortKey === o.key ? '再次点击切换升/降序' : o.hint"
        @click="pickSort(o)"
      >
        {{ o.label }}
        <Icon v-if="sortKey === o.key" :name="sortDir === 'asc' ? 'arrow-up' : 'arrow-down'" :size="11" />
      </button>
      <span class="v-sep"></span>
      <span class="muted small">筛选</span>
      <!-- 一级：已剪切 / 未剪切 / **全部放最后**（用户要求）。
           「已剪切」被选中时，紧挨着它展开**二级**「精修中 / 已导出」（用户要求：
           `已剪切[精修中][已导出] 未剪切 全部` 这个手感）：一级回答"剪出来没有"，
           二级回答"剪出来的里面出没出片"。
           ⚠️ 二级只渲染在 `cut` 档下 —— 它筛的是**已剪片段**，挂到「未剪切 / 全部」上
           就会出现"点了筛选但另一边没反应"的空按钮（见 `SUB_FILTERS` 的注释）。 -->
      <template v-for="f in FILTERS" :key="f.key">
        <button
          class="mini-btn"
          :class="{ selected: filterKey === f.key }"
          :title="f.hint"
          @click="pickFilter(f)"
        >{{ f.label }}</button>
        <span v-if="f.key === 'cut' && filterKey === 'cut'" class="subfilters">
          <button
            v-for="s in SUB_FILTERS"
            :key="s.key"
            class="sub-btn"
            :class="{ selected: polishFilter === s.key }"
            :title="s.hint"
            @click="pickPolishFilter(s)"
          >{{ s.label }}</button>
        </span>
      </template>
      <span class="muted small sort-count">
        {{ shownTotal }}<template v-if="filterKey !== 'all'">/{{ poolTotal }}</template> 个片段<template v-if="uncutEvents.length"> · 未剪切 {{ uncutEvents.length }}</template>
      </span>
    </div>

    <!-- 已剪切片段与未剪切事件**同在一面卡片墙**上（用户要求合并），按同一个排序键**混排**：
         热度是数值（两边都算得出来），所以重新检测后剩下的旧片段也能一起参与排序。
         未剪切事件没有缩略图，动作是「剪切 / 编辑」，带一枚「未剪切」chip 作区分；
         已剪卡片先认自己的 clip.json（`orphan` = 独立片段），不会跟着新事件改名。 -->
    <div class="grid cols-3">
      <template v-for="it in shownItems" :key="it.id">
        <div
          v-if="it.kind === 'clip'"
          :data-clipname="it.view.c.name"
          class="pcard clip-card"
        >
          <span class="ribbon">{{ typeZh[it.view.type] || it.view.type || 'other' }}</span>
          <div class="head card-title">
            <div>
              <h2>{{ it.view.title }}</h2>
            </div>
          </div>
          <!-- 媒体区固定高度：已剪显示缩略图；缺图用虚线占位，保证下面每个区域都从同一高度开始。
               只有点预览图才弹出视频预览，卡片其他区域不响应点击。 -->
          <div class="media-slot" :class="{ 'media-empty': !hasThumb(it.view.c) }">
            <img
              v-if="hasThumb(it.view.c)"
              :src="thumbSrc(it.view.c)"
              class="thumb"
              alt="海报"
              title="点击预览片段"
              @click="previewClip(it.view.c)"
              @error="$event.target.style.display='none'"
            >
            <template v-else>
              <Icon name="film" :size="22" />
              <span>剪出后显示预览</span>
            </template>
          </div>
          <!-- tag 统一成一行紧凑文字，不再用多个彩色药丸；状态只保留文字色。
               导出相关仍**只剩一枚**：出过片或出过封面都算「已导出」；进过精修但还没导出 → 「精修中」。 -->
          <div class="tag-row mt-8">
            <span class="tag b">{{ Math.round(it.view.c.duration_secs) }}s</span>
            <span class="tag n">{{ Math.round(it.view.c.size_bytes / 1024 / 1024) }}MB</span>
            <span v-if="Number(it.view.c.danmaku_count)" class="tag a">弹幕 {{ it.view.c.danmaku_count }}</span>
            <span v-if="Number(it.view.c.subtitle_count)" class="tag n">字幕 {{ it.view.c.subtitle_count }}</span>
            <span
              v-if="it.view.orphan"
              class="tag n orphan-chip"
              title="重新检测后已经没有和它对应的事件了：这里按剪出时的信息显示，删掉重切才会重新关联"
            >独立</span>
            <span v-if="isExported(it.view.c)" class="tag g">已导出</span>
            <span v-else-if="polishMap[it.view.c.dir]?.polish_ready" class="tag b">精修中</span>
          </div>
          <div class="row mt-8 meta-slot">
            <span class="muted small mono">{{ fmt(it.view.start) }} → {{ fmt(it.view.end) }}</span>
            <span class="tag b">{{ Math.round(it.view.popularity * 100) }}%</span>
          </div>
          <div class="summary-slot mt-8">
            <div v-if="it.view.summary" class="muted small">{{ it.view.summary }}</div>
          </div>
          <div class="foot">
            <button v-if="it.view.ev" class="mini-btn" :disabled="!!outBusy"
                    :title="outBusy ? '该项目正在' + kindText(outBusy.kind) : ''"
                    @click.stop="editEvent(it.view.ev)">编辑</button>
            <button class="mini-btn" :disabled="!!clipBusyOf(it.view.c.name) || polishOpening === it.view.c.dir"
                    :title="polishOpening === it.view.c.dir
                      ? '正在初始化精修工程…'
                      : (clipBusyOf(it.view.c.name) ? '该片段正在' + kindText(clipBusyOf(it.view.c.name).kind) : '')"
                    @click.stop="goPolish(it.view.c)">{{ polishOpening === it.view.c.dir ? '初始化中…' : '精修' }} <Icon name="arrow-right" :size="11" /></button>
            <button class="mini-btn danger clip-del"
                    :disabled="deletingDirs.has(it.view.c.dir)"
                    :title="deletingDirs.has(it.view.c.dir) ? '正在删除…' : '删除该片段（含精修产物）'"
                    @click.stop="removeClip(it.view.c)"><Icon name="trash" :size="11" /> 删除</button>
          </div>
        </div>

        <div
          v-else
          :data-eventrank="it.ev.rank"
          class="pcard clip-card uncut-card"
        >
          <span class="ribbon">{{ typeZh[it.ev.event_type] || it.ev.event_type || 'other' }}</span>
          <div class="head card-title">
            <div>
              <h2>{{ it.ev.label }}</h2>
            </div>
          </div>
          <!-- 未剪切事件还没有成片，**不占缩略图区域**：省下来的高度全部让给下面的摘要。 -->
          <div class="tag-row mt-8">
            <span class="tag a">未剪切</span>
            <span class="tag b">{{ Math.round((it.ev.end_sec || 0) - (it.ev.start_sec || 0)) }}s</span>
            <span v-if="Number(it.ev.danmaku_count)" class="tag a">弹幕 {{ it.ev.danmaku_count }}</span>
            <span v-if="Number(it.ev.subtitle_count)" class="tag n">字幕 {{ it.ev.subtitle_count }}</span>
            <span
              v-if="!it.ev.clip_dir && occupiedRanks.has(Number(it.ev.rank))"
              class="tag r"
              title="这个序号上有一个属于别的事件的旧片段：先把它删掉再剪，否则会覆盖它"
            >序号被占用</span>
          </div>
          <div class="row mt-8 meta-slot">
            <span class="muted small mono">{{ fmt(it.ev.start_sec) }} → {{ fmt(it.ev.end_sec) }}</span>
            <span class="tag b">{{ Math.round((it.ev.popularity || 0) * 100) }}%</span>
          </div>
          <div class="summary-slot mt-8">
            <div v-if="it.ev.summary" class="muted small">{{ it.ev.summary }}</div>
          </div>
          <div class="foot">
            <button class="mini-btn" :disabled="!!outBusy"
                    :title="outBusy ? '该项目正在' + kindText(outBusy.kind) : ''"
                    @click.stop="editEvent(it.ev)">编辑</button>
            <button
              class="mini-btn ev-preview"
              :disabled="!sourceUrl"
              :title="sourceUrl
                ? '预览原片里这一段（' + fmt(it.ev.start_sec) + ' → ' + fmt(it.ev.end_sec) + '，播到出点自动停）'
                : '找不到原片：源视频必须在项目目录内才能在这里预览，可去预处理页查看'"
              @click.stop="previewEvent(it.ev)"
            ><Icon name="play" :size="11" /> 预览</button>
            <!-- 剪切**不禁用**：服务端遇到占用是排队而不是报错，拦在这里只会让用户
                 连点下一条都做不到（用户要求"允许点、排队等"）。占用时改文案说明会排队。 -->
            <button class="btn btn-primary cut-btn"
                    :title="cutBusy
                      ? '该项目正在' + kindText(cutBusy.kind) + '，点下去会排队等它结束'
                      : ''"
                    @click.stop="cutEvent(it.ev)">{{ cutBusy ? '排队中' : '剪切' }}</button>
          </div>
        </div>
      </template>

      <div v-if="!shownTotal && !loading" class="empty">
        <div class="empty-icon"><Icon :name="emptyIcon" :size="24" /></div>
        {{ emptyText }}
      </div>
      <div v-for="i in 3" v-show="loading" :key="'sk' + i" class="skeleton"></div>
    </div>

    <AppModal :show="showAdd" title="添加片段" @close="showAdd = false">
      <label>标题（仅人工查看，不影响文件名）</label>
      <input v-model="newLabel" placeholder="片段标题">
      <div class="row mt-8">
        <div class="flex-1"><label>开始时间（秒）</label><input v-model.number="newStart" type="number" step="0.1" min="0"></div>
        <div class="flex-1"><label>结束时间（秒）</label><input v-model.number="newEnd" type="number" step="0.1" min="0"></div>
        <div class="flex-1"><label>时长（秒）</label><input :value="newDuration" type="number" readonly></div>
      </div>
      <label class="mt-8">类型</label>
      <select v-model="newType">
        <option v-for="t in types" :key="t" :value="t">{{ typeZh[t] || t }}</option>
      </select>
      <label class="mt-8">详情（可选）</label>
      <textarea v-model="newSummary" rows="3" placeholder="给片段写点备注，留空也行"></textarea>
      <template #footer>
        <button class="btn btn-primary" @click="addClip" :disabled="!canAddClip">保存</button>
        <button class="btn" @click="showAdd = false">取消</button>
      </template>
    </AppModal>

    <AppModal :show="!!editing" title="编辑事件" @close="editing = null">
      <label>标题</label>
      <input v-model="editLabel" placeholder="事件标题">
      <div class="row mt-8">
        <div class="flex-1"><label>开始时间（秒）</label><input v-model.number="editStart" type="number" step="0.1" min="0"></div>
        <div class="flex-1"><label>结束时间（秒）</label><input v-model.number="editEnd" type="number" step="0.1" min="0"></div>
      </div>
      <label class="mt-8">类型</label>
      <select v-model="editType">
        <option v-for="t in types" :key="t" :value="t">{{ typeZh[t] || t }}</option>
      </select>
      <label class="mt-8">详情</label>
      <textarea v-model="editSummary" rows="3"></textarea>
      <template #footer>
        <button class="btn btn-primary" @click="saveEdit">保存</button>
        <button class="btn" @click="editing = null">取消</button>
      </template>
    </AppModal>

    <AppModal :show="!!playing" wide :title="playing?.name || ''" @close="playing = null">
      <!-- 未剪切事件：播**原片**，播放头锁在 [start, end] —— 入点起播、到出点停住（可勾循环）、
           拖到区间外回弹。已剪片段就是它自己那 35 秒，走下面那个裸播放器即可。 -->
      <template v-if="playing && isRangeMode">
        <div class="ev-preview-bar">
          <span class="muted small mono">
            本段 {{ fmt(playing.start) }} → {{ fmt(playing.end) }}
            · {{ Math.round(playing.end - playing.start) }}s
            <template v-if="playing.heat != null"> · 热度 {{ Math.round(playing.heat * 100) }}%</template>
          </span>
          <label class="small loop-toggle">
            <input v-model="loopRange" type="checkbox">
            循环本段
          </label>
        </div>
        <!-- **不用原生 controls**：原生那条进度条是整片全长的独立控件，跟下面这条带标记的
             不是同一个坐标系，看着就是"两条进度条对不上"（用户报的）。这里自写一套控件，
             进度条只有下面这一条 —— 整片时长 + 事件区间高亮带 + 播放头，位置完全由我们算。 -->
        <div ref="stageRef" class="ev-stage">
          <video
            ref="rangeRef"
            preload="metadata"
            playsinline
            :src="playing.url"
            title="点击播放/暂停"
            @click="toggleRangePlay"
            @loadedmetadata="onRangeMeta"
            @timeupdate="onRangeTime"
            @seeking="onRangeSeek"
            @seeked="onRangeSeek"
            @play="onRangePlay"
            @pause="onRangePause"
            @error="onRangeError"
          ></video>
          <!-- 字幕叠层：与预处理页/精修预览同一套样式（72px@1920 缩放、同色同描边），
               文字来自 transcript（原片时间轴），按当前播放位置取那一条 -->
          <div v-if="activeCaption" class="ev-cap">{{ activeCaption }}</div>
        </div>
        <div class="ev-controls">
          <button class="mini-btn" @click="toggleRangePlay">
            <Icon :name="rangePlaying ? 'pause' : 'play'" :size="12" />
            {{ rangePlaying ? '暂停' : '播放' }}
          </button>
          <!-- 唯一的进度条：点/拖即跳转，位置吸附进事件区间；滚轮缩放、双击复位视窗 -->
          <span v-if="viewLabels" class="muted small mono ev-win">{{ viewLabels.start }}</span>
          <div
            ref="rangeBarRef"
            class="rng-track"
            title="进度条：高亮段=本条事件的播放范围（入点/出点）· 点或拖跳转 · 滚轮缩放 · 双击回到事件范围"
            @pointerdown="onRangePointerDown"
            @wheel.prevent="onRangeWheel"
            @dblclick="resetView"
          >
            <div class="rng-band" :style="{ left: bandLeft + '%', width: bandWidth + '%' }"></div>
            <div class="rng-head" :style="{ left: headLeft + '%' }"></div>
          </div>
          <span v-if="viewLabels" class="muted small mono ev-win">{{ viewLabels.end }}</span>
          <span class="muted small mono ev-time">
            段内 {{ fmtDur(Math.max(0, rangePos - playing.start)) }} / {{ fmtDur(playing.end - playing.start) }}
            <template v-if="rangeDuration > 0"> · 整片 {{ fmt(rangeDuration) }}</template>
          </span>
          <button class="mini-btn" :class="{ selected: rangeSilenced }" :title="rangeSilenced ? '取消静音' : '静音'" @click="toggleRangeMute">
            <Icon :name="rangeSilenced ? 'volume-x' : 'volume'" :size="13" />
          </button>
          <input class="ev-vol" type="range" min="0" max="1" step="0.05" :value="rangeVol" title="音量（全应用共用）" @input="onRangeVolume">
          <button class="mini-btn" title="全屏" @click="rangeFullscreen"><Icon name="maximize" :size="13" /></button>
        </div>
      </template>
      <div v-else class="ev-stage">
        <video controls autoplay class="video-lg" :src="playing?.url"
               @timeupdate="clipPos = $event.target.currentTime"></video>
        <div v-if="activeCaption" class="ev-cap">{{ activeCaption }}</div>
      </div>
    </AppModal>
  </div>
</template>

<script setup>
import { ref, computed, watch, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'
import { api } from '../api/client'
import { projectData } from '../api/projectData'
import { initPolish, polishPost, runRender } from '../api/polish'
import { watchJob, busyOn, clipKey, idxKey, outKey, kindText, QUEUED_HINT } from '../api/jobs'
import { useJobs } from '../composables/useJobs'
import { toast } from '../stores/toast'
import { confirmDialog, promptDialog } from '../stores/dialog'
import Icon from './Icon.vue'
import AppModal from './AppModal.vue'
import { EVENT_TYPES, TYPE_ZH, fmtClock, clipMediaUrl, redetectWarning, clipMatchesEvent, uncutEventsOf } from '../utils'
import { readMasterVolume, readMuted, saveMasterVolume, saveMuted } from '../polish/audio'

const props = defineProps({
  project: { type: Object, required: true },
  projectData: { type: Object, default: null },
  /** 原片播放地址（`mediaUrl(pid, status.input_file)`，由切片页传入）；空串 = 拿不到源视频 */
  sourceUrl: { type: String, default: '' }
})

// 资源域键（与后端 server/scope_key.rs 一致）：项目输出目录 / 发布索引 / 单个片段目录。
// 按钮只在自己那档被占用时才禁用 —— 别的项目转写、别的片段渲染都不该挡住这里。
const { jobs: allJobs, reload: reloadJobs, stale: jobsStale } = useJobs()
const pid = computed(() => props.project?.id)
/**
 * 项目级作用域占用（`p{pid}:out` / `p{pid}:idx`）。
 *
 * ⚠️ **快照过期（`jobsStale`）时一律当作"没占用"**：这两个键是项目级的，`cut_one` 必持它们，
 * 所以任何一条剪切在跑都会让**所有**卡片的门控为真。门控只是"提前告诉用户要等"，
 * **不是正确性保证** —— 正确性由服务端的资源键排队兜（点下去最多进队列，不会撞车）。
 * 一旦任务列表读不出来，保留旧快照就会把整个切片页的按钮永久锁死；
 * 宁可放行让服务端去排队，也比"功能不可用"强。
 */
const outBusy = computed(() => (jobsStale.value ? null : busyOn(allJobs.value, [outKey(pid.value)])))
const idxBusy = computed(() => (jobsStale.value ? null : busyOn(allJobs.value, [idxKey(pid.value)])))
/** 剪切按钮的占用提示（不用于禁用，只用于文案 —— 见上）。 */
const cutBusy = computed(() => outBusy.value || idxBusy.value)
/** 某片段目录上有没有活动任务（渲染/精修/重切） */
function clipBusyOf(name) {
  return busyOn(allJobs.value, [clipKey(pid.value, name)])
}
/**
 * `changed`：**写完盘且不产生任务**的动作（删片段 / 改事件 / 新建事件）请求父页面重新取数。
 * 数据由父页面持有（`props.projectData` 是快照），子组件自己 `load()` 只是照抄那份旧快照 ——
 * 不发这个事件，界面上就什么都看不到（删除尤其明显：卡片不消失，再点一次就是 404「不存在」）。
 * ⚠️ 别在 `load()` 里发：props 变化会触发 `load()`，那样父 load → props 变 → 子 load → emit → 父 load 会成环。
 */
const emit = defineEmits(['changed'])
const router = useRouter()

const events = ref([])
const clips = ref([])
const polishMap = ref({})
const loading = ref(false)
const loadError = ref('')
/** 正在删除的片段目录（删除是慢 IO：目录里可能有几百 MB 成片，删的过程中禁掉那颗按钮防连点） */
const deletingDirs = ref(new Set())
/** 正在"补 init 后进精修"的片段目录：双击防抖，也避免同一片段发两次 init 任务 */
const polishOpening = ref('')
/** 等 init 任务期间开着的 watchJob 订阅，组件卸载时统一关掉（与首页 `HomeClipsOverview` 同款） */
const jobClosers = []
const showAdd = ref(false)
const playing = ref(null)
// 区间预览（未剪切事件播原片时）：
// `rangeRef` = 那个 <video>；`loopRange` = 到出点是循环还是停住；`clampBusy` 防回写 currentTime 时的重入。
const rangeRef = ref(null)
const loopRange = ref(false)
const clampBusy = ref(false)
/** 到出点后的判定余量（timeupdate ≈4Hz，留一点免得正好卡在边界来回写） */
const RANGE_EPS = 0.05
const newLabel = ref('')
const newStart = ref(0)
const newEnd = ref(10)
const newType = ref('other')
const newSummary = ref('')
const editing = ref(null)
const editLabel = ref('')
const editStart = ref(0)
const editEnd = ref(10)
const editType = ref('other')
const editSummary = ref('')
const types = EVENT_TYPES
const typeZh = TYPE_ZH
const fmt = fmtClock

const newDuration = computed(() => Math.max(0, Math.round((newEnd.value - newStart.value) * 10) / 10))
const canAddClip = computed(() => {
  // 标题是唯一必填（纯人工查看的元数据）；详情可选 —— 与预处理页的切片表单一致
  return !!(newLabel.value.trim() && newEnd.value > newStart.value)
})

// ── 排序切换：热度 / 开始时间 / 弹幕量 / 字幕量 ──────────────────
// 「热度」是**数值**（0~1 的 popularity：弹幕峰值 0.6 + 均值 0.4 + 字幕密度，
// 再与「标题+概括」LLM 传播潜力分按 detection.llm_score_weight 加权，见 detection::rank_events），
// 不是 rank —— rank 只是按热度排出来的**名次**，重新检测后整体重排，旧片段就再也排不进去；
// popularity 两类卡片都有（事件在 events.json、片段在 clip.json 里冻结），所以能一起排。
const SORT_OPTIONS = [
  { key: 'popularity', label: '热度', dir: 'desc', hint: '按热度值排序（0~100%，越高越热）' },
  { key: 'start', label: '开始时间', dir: 'asc', hint: '按在原片里的开始时间排序' },
  { key: 'danmaku', label: '弹幕量', dir: 'desc', hint: '按弹幕条数排序' },
  { key: 'subtitle', label: '字幕量', dir: 'desc', hint: '按字幕条数排序' }
]
const SORT_STORE = 'yekitrim.slices.sort'

function readSort() {
  try {
    const raw = JSON.parse(localStorage.getItem(SORT_STORE) || '{}')
    const opt = SORT_OPTIONS.find(o => o.key === raw.key)
    if (opt) return { key: opt.key, dir: raw.dir === 'asc' || raw.dir === 'desc' ? raw.dir : opt.dir }
  } catch {}
  // 回落取**第一个选项**的默认值：以前在这里写死了 `rank`，换成按 popularity 排之后
  // 旧偏好 `{key:'rank'}` 失配 —— 排序栏一个按钮都不高亮，还按一个不存在的键静默排序
  return { key: SORT_OPTIONS[0].key, dir: SORT_OPTIONS[0].dir }
}

const initialSort = readSort()
const sortKey = ref(initialSort.key)
const sortDir = ref(initialSort.dir)

function pickSort(opt) {
  if (sortKey.value === opt.key) sortDir.value = sortDir.value === 'asc' ? 'desc' : 'asc'
  else { sortKey.value = opt.key; sortDir.value = opt.dir }
}

watch([sortKey, sortDir], () => {
  try { localStorage.setItem(SORT_STORE, JSON.stringify({ key: sortKey.value, dir: sortDir.value })) } catch {}
})

// ── 未剪切事件的排序 ─────────────────────────────────────────────
// 未剪切事件**并进同一面卡片墙**后，排序栏只此一条：热度 rank / 开始时间 / 弹幕量 / 字幕量
// （事件同样有 rank / start_sec / danmaku_count / subtitle_count，四个键都有效）。
/** 片段自己认领的事件：目录名（rank）对得上**且**标题或原片区间对得上（见 utils 的 clipMatchesEvent）。
 *  重新检测之后，新事件会拿到新的 rank/标题/区间 —— 那时老片段就"没有事件了"，
 *  卡片按它自己的 clip.json 显示（独立片段），新事件也不会被误标成"已剪切"。 */
function matchedEventOf(c) {
  return events.value.find(ev => clipMatchesEvent(c, ev)) || null
}

/** 已剪片段的展示模型：有匹配事件就跟着事件（编辑后卡片会更新），否则用 clip.json 里冻结的信息。 */
function clipView(c) {
  const ev = matchedEventOf(c)
  const meta = c.meta || {}
  return {
    c,
    ev,
    orphan: !ev,
    title: ev ? ev.label : (meta.title || c.name),
    type: ev ? ev.event_type : (meta.event_type || 'other'),
    summary: ev ? (ev.summary || '') : (meta.summary || ''),
    start: ev ? Number(ev.start_sec) : Number(meta.original_start),
    end: ev ? Number(ev.end_sec) : Number(meta.original_end),
    popularity: ev ? Number(ev.popularity) || 0 : Number(meta.popularity) || 0
  }
}

function rankOf(name) {
  return Number(String(name || '').replace(/^clip-/, '')) || 0
}

function clipStats(c) {
  // 时间/热度尽量取"属于自己的那条事件"（没有就退回 clip.json 里冻结的值）——
  // 不能按 rank 抓事件：重新检测后那个 rank 上已经是别的事件了
  const ev = matchedEventOf(c)
  const meta = c.meta || {}
  return {
    // 排序兜底用的「进过精修」标记：只认 polish_ready（≠ 卡片上的「已导出」＝ rendered||cover）
    polishReady: polishMap.value[c.dir]?.polish_ready ? 1 : 0,
    popularity: ev ? Number(ev.popularity) || 0 : Number(meta.popularity) || 0,
    start: ev ? Number(ev.start_sec) || 0 : Number(meta.original_start) || 0,
    danmaku: Number(c.danmaku_count) || 0,
    subtitle: Number(c.subtitle_count) || 0,
    rank: rankOf(c.name) || Number(meta.rank) || 0
  }
}

function eventStats(ev) {
  return {
    polishReady: 0,
    popularity: Number(ev.popularity) || 0,
    start: Number(ev.start_sec) || 0,
    danmaku: Number(ev.danmaku_count) || 0,
    subtitle: Number(ev.subtitle_count) || 0,
    rank: Number(ev.rank) || 0
  }
}

/** 通用比较器：先按当前排序键升降序，同值时进过精修的优先，再按序号兜底（保证顺序稳定）。 */
function cmpStats(sa, sb) {
  const va = sa[sortKey.value]
  const vb = sb[sortKey.value]
  if (va !== vb) return (va - vb) * (sortDir.value === 'asc' ? 1 : -1)
  if (sa.polishReady !== sb.polishReady) return sb.polishReady - sa.polishReady
  return sa.rank - sb.rank
}

// ── 筛选：一级 已剪切 / 未剪切 / 全部（**全部放最后**，用户要求）──
// 一级只回答「剪出来了没有」；「精修」不再是一级档位，降为**「已剪切」下的二级筛选**
// （用户要求：`已剪切[精修中][已导出] 未剪切 全部`）——
// 片状态只对已剪片段成立（未剪切事件还没进精修流程），混在一级里会让人点了
// 二级却以为漏了未剪切事件那一半。
const FILTERS = [
  { key: 'cut', label: '已剪切', hint: '已经剪出成片的片段' },
  { key: 'uncut', label: '未剪切', hint: '检测到但还没剪的事件' },
  { key: 'all', label: '全部', hint: '已剪切 + 未剪切全部显示' }
]
// 二级（只在 `cut` 档出现）：精修中 / 已导出。
//
// ⚠️ 判据是**是否已经导出**（`isExported` = `rendered || cover`），与卡片上那枚 chip 是**同一套口径**。
// 不再用"进没进过精修"（旧的 `isInPolish` = `polish_ready || rendered || cover`）：
// **剪切收尾本身就会 `polish init`**（见 `crates/yekitrim-service/AGENTS.md`「剪切收尾 = 补精修工程」），
// 于是每个已剪片段都 `polish_ready` → 旧判据对已剪片段**恒为真** → 「未精修」那一档永远是空的。
// 两档互斥互补：精修中 = 已剪但还没出片/出封面；已导出 = 出过其中任意一个。
// 保留「全部」在前：一级「已剪切」的老行为（所有已剪片段）由此一键回到，不用再点回一级。
const SUB_FILTERS = [
  { key: 'all', label: '全部', hint: '已剪片段全部都看' },
  { key: 'polishing', label: '精修中', hint: '已剪但还没有导出成片或封面' },
  { key: 'exported', label: '已导出', hint: '已经出过片或出过封面（与卡片上的 chip 同口径）' }
]
const FILTER_STORE = 'yekitrim.slices.filter'
const SUB_FILTER_STORE = 'yekitrim.slices.subfilter'
// 两级化之前存的 key 要**翻译成"一级 + 二级"**再回填，否则老用户打开切片页会退到默认档、
// 以为筛选坏了：上一版的 `polish`（一级「精修中」）→ 已剪切 + 已导出。
const LEGACY_FILTERS = {
  polish: { primary: 'cut', sub: 'exported' }
}
// 二级自己也有历史值要翻译：`unpolished` / `polished` 是"进没进过精修"那一代的口径（2026-09 改成"出没出片"）。
// ⚠️ 这对映射**有损、且是有意为之**：旧 `polished`（进过精修，含没出片的）在新口径下只有"已导出"那一半能表达；
//   旧 `unpolished`（从没进过精修）在新口径下属于「精修中」。
//   还能这么迁的理由：`polish_ready` 现在对所有已剪片段都为真，旧两档本身已经退化成"全满 / 恒空"，
//   继续认旧 key 只会让用户看到一排永远筛不出东西（或永远等于没筛）的按钮。
const LEGACY_SUB_FILTERS = {
  unpolished: 'polishing',
  polished: 'exported'
}
// 只在**用户自己点过**之后才记住；没存过时按数量定默认（见 applyDefaultFilter）——
// 存下来的默认值会在项目状态变化后过期（比如本来全是未剪切、后来全剪完了）。
const savedFilter = (() => {
  try {
    const v = localStorage.getItem(FILTER_STORE) || ''
    if (FILTERS.some(f => f.key === v)) return { primary: v, sub: '' }
    if (LEGACY_FILTERS[v]) return { primary: LEGACY_FILTERS[v].primary, sub: LEGACY_FILTERS[v].sub }
  } catch {}
  return { primary: '', sub: '' }
})()
const savedSub = (() => {
  if (savedFilter.sub) return savedFilter.sub
  try {
    const v = localStorage.getItem(SUB_FILTER_STORE) || ''
    if (SUB_FILTERS.some(s => s.key === v)) return v
    // 旧口径的二级值（见 `LEGACY_SUB_FILTERS`）翻译过来；认不出的交给默认档
    return LEGACY_SUB_FILTERS[v] || ''
  } catch { return '' }
})()
const filterKey = ref(savedFilter.primary)
/** 二级筛选：没存过就用「全部」（= 一级「已剪切」的原义）。 */
const polishFilter = ref(savedSub || SUB_FILTERS[0].key)

function pickFilter(f) {
  filterKey.value = f.key
  try { localStorage.setItem(FILTER_STORE, f.key) } catch {}
}

function pickPolishFilter(s) {
  polishFilter.value = s.key
  try { localStorage.setItem(SUB_FILTER_STORE, s.key) } catch {}
}

/** 没存过偏好时：有已剪切就勾「已剪切」，一个都没有就勾「未剪切」（用户要求）。 */
function applyDefaultFilter() {
  if (filterKey.value) return
  filterKey.value = cutClips.value.length ? 'cut' : 'uncut'
}

const cutClips = computed(() => clips.value
  .filter(c => c.video_exists !== false)
  .slice()
  .sort((a, b) => cmpStats(clipStats(a), clipStats(b))))
// 未剪切事件 = 没有"属于自己的"片段（序号被别的片段占着也算未剪切，见 utils.uncutEventsOf）
const uncutEvents = computed(() => uncutEventsOf(events.value, clips.value)
  .slice()
  .sort((a, b) => cmpStats(eventStats(a), eventStats(b))))
// 旧契约才需要「序号被占用」提示：新事件有 clip_dir，不再拿 rank 当目录名。
// 这里只统计还能解析出 `clip-NN` 的旧片段；模板里再排除已有 clip_dir 的事件。
const occupiedRanks = computed(() => new Set(
  cutClips.value
    .filter(c => !matchedEventOf(c))
    .map(c => rankOf(c.name))
    .filter(n => n > 0)
))

/**
 * 这枚片段"导出产物已经落盘"了吗 —— 出过片或出过封面都算。
 * 卡片上那枚「已导出」chip 与二级筛选**共用这一条判据**（`rendered`/`cover` 是服务端
 * `/api/project-data/polish` 按文件存在性给的）。
 *
 * ⚠️ 判据刻意**不含** `polish_ready`：剪切收尾就会 `polish init`，那个字段对所有已剪片段
 * 都为真，拿它当筛选条件等于没筛 —— 旧的二级「未精修 / 已精修」就是这么退化成恒空/恒满的。
 */
const isExported = (c) => {
  const m = polishMap.value[c.dir]
  return !!(m?.rendered || m?.cover)
}
/** 二级「精修中」= 已剪但还没导出（**不是**"没进过精修"—— 剪切收尾就 init 了）。 */
const isPolishing = (c) => !isExported(c)
const cutCards = computed(() => cutClips.value.map(clipView))
/** 墙面上的条目：已剪片段 + 未剪切事件**按同一个排序键混排**（热度是数值，两边都算得出来）。 */
const shownItems = computed(() => {
  const k = filterKey.value
  // 已剪片段：一级 `uncut` 不显示；`cut` 档再按二级收窄一次（全部 / 精修中 / 已导出）
  let clips = k === 'uncut' ? [] : cutCards.value
  if (k === 'cut' && polishFilter.value === 'polishing') clips = clips.filter(v => isPolishing(v.c))
  if (k === 'cut' && polishFilter.value === 'exported') clips = clips.filter(v => isExported(v.c))
  // 未剪切事件只在「未剪切 / 全部」出现 —— 二级筛选是已剪片段的事，不参与这两档
  const events = (k === 'uncut' || k === 'all') ? uncutEvents.value : []
  const items = [
    ...clips.map(v => ({ kind: 'clip', id: 'c:' + v.c.dir, view: v })),
    ...events.map(ev => ({ kind: 'event', id: 'e:' + ev.rank, ev }))
  ]
  return items.sort((a, b) => cmpStats(statsOfItem(a), statsOfItem(b)))
})
const statsOfItem = (it) => (it.kind === 'clip' ? clipStats(it.view.c) : eventStats(it.ev))
const shownTotal = computed(() => shownItems.value.length)
/** 计数分母 = 当前**一级**档位涉及的片段总数（二级只收窄，不改分母）——
 *  否则「已剪切 / 精修中」的读数会拿"已剪 + 未剪切"当分母，看起来像筛丢了。 */
const poolTotal = computed(() => {
  if (filterKey.value === 'uncut') return uncutEvents.value.length
  return cutClips.value.length
})

const emptyText = computed(() => {
  if (filterKey.value === 'uncut') return '暂无未剪切事件'
  if (filterKey.value === 'cut') {
    // 二级档位下的空态要能自解释：别让人以为"已剪片段全没了"
    if (polishFilter.value === 'polishing') return '已剪片段都导出过了'
    if (polishFilter.value === 'exported') return '已剪片段还没有导出过（去精修页出一片或导出封面）'
    return '暂无已剪切片段'
  }
  return '暂无片段'
})
const emptyIcon = computed(() => (filterKey.value === 'uncut' ? 'film' : 'scissors'))

function hasThumb(c) {
  return c && c.video_exists
}

function thumbSrc(c) {
  // 传**目录名**（服务端两种都收：数字 rank 或真实目录名）——
  // 以前这里剥掉 `clip-` 只留数字，手工命名的目录（如 `clip-out`）会拼成 `/clips/out/thumb` 直接 400。
  return projectData.thumbUrl(props.project?.id, c.name)
}

function secToTs(sec) {
  sec = Math.max(0, Number(sec) || 0)
  const h = Math.floor(sec / 3600)
  const m = Math.floor((sec % 3600) / 60)
  const s = Math.floor(sec % 60)
  const ms = Math.round((sec - Math.floor(sec)) * 1000)
  return { h, m, s, ms }
}

function previewClip(c) {
  playing.value = { name: c.name, url: clipMediaUrl(props.project?.id, c.name, 'clip.mp4') }
  loadClipSubs(c)
}

/** 未剪切事件：播原片并把播放头锁在该事件的区间里（见模板里的区间分支）。 */
function previewEvent(ev) {
  if (!props.sourceUrl || !ev) return
  clampBusy.value = false
  loopRange.value = false
  rangeDuration.value = 0 // 等新视频的 metadata，别拿上一条的时长画标记带
  rangePos.value = 0
  viewStart.value = 0
  viewEnd.value = 0
  rangePlaying.value = false
  rangeVol.value = readMasterVolume() // 与精修预览共用一份音量
  rangeMuted.value = readMuted()
  lastRangeVol = rangeVol.value > 0 ? rangeVol.value : 1
  window.addEventListener('keydown', onRangeKey)
  ensureTranscript() // 懒加载字幕，到了就跟着播放显示
  playing.value = {
    name: ev.label,
    url: props.sourceUrl,
    start: Math.max(0, Number(ev.start_sec) || 0),
    end: Math.max(0, Number(ev.end_sec) || 0),
    heat: Number(ev.popularity) || 0
  }
}

/** 当前播放的是不是"原片区间预览"（未剪切事件），而不是成片。 */
const isRangeMode = computed(() => {
  const p = playing.value
  return !!(p && p.start != null && p.end != null && p.end > p.start)
})

// ── 播放范围标记（整片时长条上的高亮带 + 带内播放头）────────────────
const rangeBarRef = ref(null)
const stageRef = ref(null)     // 播放区（全屏用）
const rangeDuration = ref(0)   // 原片总时长（来自 metadata）
const rangePos = ref(0)        // 当前播放位置（秒）
let rangeClock = 0

// 进度条的**视窗**（秒）：整片 3 小时时事件那 35 秒只占 0.35%，糊成一根细线没法用 ——
// 所以条上画的不是整片，而是 [viewStart, viewEnd] 这一段（默认 = 事件区间前后各留点余量），
// 滚轮缩放（以光标为锚点）、双击复位。视窗永远夹在 [0, duration] 内。
const viewStart = ref(0)
const viewEnd = ref(0)
const viewSpan = computed(() => Math.max(0.001, viewEnd.value - viewStart.value))
const MIN_WINDOW = 2           // 最多放大到只看 2 秒
const DEFAULT_PAD_RATIO = 0.5  // 默认视窗 = 事件前后的余量（与事件时长同比例）
const DEFAULT_PAD_MIN = 10

/** 默认视窗：事件区间前后各留一点（用户要求"显示当前播放范围前后多出来一点的部分"）。 */
function defaultView(p) {
  const len = Math.max(1, Number(p.end) - Number(p.start))
  const pad = Math.max(DEFAULT_PAD_MIN, len * DEFAULT_PAD_RATIO)
  const d = rangeDuration.value
  let s = Math.max(0, Number(p.start) - pad)
  let e = Number(p.end) + pad
  if (d > 0) e = Math.min(d, e)
  if (e - s < MIN_WINDOW) e = s + MIN_WINDOW
  return [s, e]
}
function resetView() {
  const p = playing.value
  if (!p || !isRangeMode.value) return
  const [s, e] = defaultView(p)
  viewStart.value = s
  viewEnd.value = e
}

/** 滚轮缩放：以光标位置为锚点（那儿的时间保持不动），夹在 [0, duration] 内。
 *  步长 = max(2 秒, 当前视窗的 1/4)：小窗口细调、大窗口几下就能拉到整片。 */
function zoomAt(clientX, zoomIn) {
  const el = rangeBarRef.value
  const d = rangeDuration.value
  if (!el || !(d > 0)) return
  const r = el.getBoundingClientRect()
  if (!r.width) return
  const frac = Math.min(1, Math.max(0, (clientX - r.left) / r.width))
  const cur = viewSpan.value
  const step = Math.max(MIN_WINDOW, cur * 0.25)
  const win = zoomIn
    ? Math.max(MIN_WINDOW, cur - step)
    : Math.min(d, cur + step)
  const anchor = viewStart.value + frac * cur
  let s = anchor - frac * win
  let e = s + win
  if (s < 0) {
    s = 0
    e = win
  }
  if (e > d) {
    e = d
    s = Math.max(0, d - win)
  }
  viewStart.value = s
  viewEnd.value = e
}
function onRangeWheel(e) {
  if (!isRangeMode.value || !(rangeDuration.value > 0)) return
  e.preventDefault()
  zoomAt(e.clientX, e.deltaY < 0) // 上滚 = 放大（视窗变窄）
}

// ── 字幕（预览里跟着播放位置显示当前这一句）────────────────────────
// 懒加载：只有真的预览时才拉整条 transcript（3 小时录播几千条，别拖累页面首屏）。
const transcript = ref([])
let transcriptLoaded = false
async function ensureTranscript() {
  if (transcriptLoaded) return
  try {
    const t = await projectData.transcript(props.project.id, '读取字幕')
    transcript.value = (t && t.segments) || []
    transcriptLoaded = true
  } catch {
    transcript.value = [] // 没有转写文件（404）等：不显示字幕，下次预览再试
  }
}

// ── 已剪片段预览：读 clip.srt，按片段本地时间叠字幕 ──────────────
const clipPos = ref(0)
const clipSubs = ref([])
let clipSubsToken = 0

/** 极简 SRT 解析：只认 `00:00:01,000 --> 00:00:03,500` 这种块。 */
function parseSrtText(raw) {
  const text = String(raw || '').replace(/\r\n/g, '\n').replace(/\r/g, '\n')
  const out = []
  const toSec = (s) => {
    const m = String(s || '').trim().match(/^(\d+):(\d+):(\d+)[,.](\d+)$/)
    if (!m) return NaN
    return Number(m[1]) * 3600 + Number(m[2]) * 60 + Number(m[3])
      + Number(m[4].padEnd(3, '0').slice(0, 3)) / 1000
  }
  for (const block of text.split(/\n{2,}/)) {
    const lines = block.split('\n').map((l) => l.trim()).filter(Boolean)
    const timeLine = lines.find((l) => l.includes('-->'))
    if (!timeLine) continue
    const [a, b] = timeLine.split('-->').map((s) => s.trim().split(/\s+/)[0])
    const start = toSec(a)
    const end = toSec(b)
    if (!Number.isFinite(start) || !Number.isFinite(end)) continue
    const body = lines.filter((l) => l !== timeLine && !/^\d+$/.test(l)).join(' ').trim()
    if (body) out.push({ start, end, text: body })
  }
  return out.sort((a, b) => a.start - b.start)
}

async function loadClipSubs(c) {
  const token = ++clipSubsToken
  clipPos.value = 0
  clipSubs.value = []
  if (!c || !Number(c.subtitle_count)) return
  try {
    const r = await fetch(clipMediaUrl(props.project?.id, c.name, 'clip.srt'))
    if (token !== clipSubsToken) return
    if (r.ok) clipSubs.value = parseSrtText(await r.text())
  } catch {
    // 没有 clip.srt / 读取失败：不显示字幕即可，不要干扰预览
  }
}

/** 已剪片段：video.currentTime 就是 clip.srt 的时间轴。 */
function clipCaptionAt(t) {
  const arr = clipSubs.value
  if (!arr.length) return ''
  let lo = 0
  let hi = arr.length - 1
  let best = -1
  while (lo <= hi) {
    const mid = (lo + hi) >> 1
    if (arr[mid].start <= t) {
      best = mid
      lo = mid + 1
    } else {
      hi = mid - 1
    }
  }
  for (let i = best; i >= 0 && i > best - 4; i--) {
    if (t >= arr[i].start && t < arr[i].end) return arr[i].text
  }
  return ''
}

/** 取 t 时刻的字幕文本：transcript 按 start 升序，二分找最后一条 start ≤ t，
 *  再往回看几条（手动编辑可能造成轻微重叠）。rAF 每帧都会调，别写成线性扫描。 */
function captionAt(t) {
  const arr = transcript.value
  if (!arr.length) return ''
  let lo = 0
  let hi = arr.length - 1
  let best = -1
  while (lo <= hi) {
    const mid = (lo + hi) >> 1
    if ((Number(arr[mid].start) || 0) <= t) {
      best = mid
      lo = mid + 1
    } else {
      hi = mid - 1
    }
  }
  for (let i = best; i >= 0 && i > best - 4; i--) {
    const s = Number(arr[i].start) || 0
    const e = Number(arr[i].end) || 0
    if (t >= s && t < e) return String(arr[i].text || '').trim()
  }
  return ''
}

const activeCaption = computed(() => (
  isRangeMode.value ? captionAt(rangePos.value) : clipCaptionAt(clipPos.value)
))

// ── 自写播放控件（原生 controls 那条进度条与标记条坐标系不同，会"对不上"）──
const rangePlaying = ref(false)
const rangeVol = ref(1)
const rangeMuted = ref(false)
const rangeSilenced = computed(() => rangeMuted.value || rangeVol.value <= 0)
let lastRangeVol = 1

/** 音量/静音写进全应用共用的那两个 localStorage 键（polish/audio.js）——与精修预览同语义 */
function applyRangeAudio() {
  const v = rangeRef.value
  if (!v) return
  const n = Number(rangeVol.value)
  v.volume = Number.isFinite(n) ? Math.min(1, Math.max(0, n)) : 1
  v.muted = !!rangeMuted.value
}
function toggleRangePlay() {
  const v = rangeRef.value
  if (!v) return
  if (v.paused) v.play?.().catch(() => {})
  else v.pause()
}
function toggleRangeMute() {
  if (rangeSilenced.value) {
    rangeMuted.value = false
    if (rangeVol.value <= 0) rangeVol.value = lastRangeVol || 1
  } else {
    rangeMuted.value = true
  }
  saveMuted(rangeMuted.value)
  saveMasterVolume(rangeVol.value)
  applyRangeAudio()
}
function onRangeVolume(e) {
  const n = Number(e.target.value)
  rangeVol.value = Number.isFinite(n) ? Math.min(1, Math.max(0, n)) : 1
  if (rangeVol.value > 0) {
    lastRangeVol = rangeVol.value
    if (rangeMuted.value) {
      rangeMuted.value = false
      saveMuted(false)
    }
  }
  saveMasterVolume(rangeVol.value)
  applyRangeAudio()
}
function rangeFullscreen() {
  stageRef.value?.requestFullscreen?.()
}
function onRangePlay() {
  rangePlaying.value = true
  startRangeClock()
}
function onRangePause() {
  rangePlaying.value = false
  syncRangePos()
}
/** 键盘：空格播放/暂停、←→ 各 5 秒（只在区间预览里生效；输入框内不抢键） */
function onRangeKey(e) {
  if (!isRangeMode.value) return
  const tag = (e.target && e.target.tagName) || ''
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || e.metaKey || e.ctrlKey) return
  if (e.code === 'Space' || e.key === ' ') {
    e.preventDefault()
    toggleRangePlay()
  } else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
    e.preventDefault()
    const v = rangeRef.value
    seekRange((v?.currentTime || 0) + (e.key === 'ArrowRight' ? 5 : -5))
  }
}

/** 秒 → 整片条上的百分比 */
/** 秒 → 条上的百分比（**按当前视窗**换算，不是整片；视窗外的会被夹到两端，靠 overflow 裁掉） */
function pct(t) {
  if (!(rangeDuration.value > 0)) return 0
  return Math.min(100, Math.max(0, ((Number(t) || 0) - viewStart.value) / viewSpan.value * 100))
}
const bandLeft = computed(() => pct(playing.value?.start))
const bandWidth = computed(() => Math.max(0, pct(playing.value?.end) - pct(playing.value?.start)))
const headLeft = computed(() => pct(rangePos.value))
/** 视窗两端的时刻（贴在条左右，让人知道现在放大到哪一段了） */
const viewLabels = computed(() => (rangeDuration.value > 0
  ? { start: fmt(rangeStartLabel.value), end: fmt(rangeEndLabel.value) }
  : null))
const rangeStartLabel = computed(() => Math.min(viewStart.value, Math.max(0, rangeDuration.value - 0.001)))
const rangeEndLabel = computed(() => Math.min(viewEnd.value, Math.max(0, rangeDuration.value - 0.001)))

/** 段内进度读数用（`0:08`），入点在内就是 0 */
function fmtDur(sec) {
  const v = Math.max(0, Math.floor(Number(sec) || 0))
  const h = Math.floor(v / 3600)
  const m = Math.floor((v % 3600) / 60)
  const s = v % 60
  return h > 0
    ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
    : `${m}:${String(s).padStart(2, '0')}`
}

function syncRangePos() {
  const v = rangeRef.value
  if (v) rangePos.value = v.currentTime || 0
}
/** 播放头跟随：timeupdate 只有 ~4Hz，看着一顿一顿的，改成 rAF（PolishPreview 同款） */
function startRangeClock() {
  if (rangeClock) return
  rangeClock = requestAnimationFrame(rangeTick)
}
function stopRangeClock() {
  if (rangeClock) cancelAnimationFrame(rangeClock)
  rangeClock = 0
}
function rangeTick() {
  rangeClock = 0
  const v = rangeRef.value
  if (!v || !isRangeMode.value) return
  rangePos.value = v.currentTime || 0
  if (!v.paused) rangeClock = requestAnimationFrame(rangeTick)
}

/** 跳转到 t，并**吸附进事件区间**（这就是"锁定"在标记带内的那一步）。 */
function seekRange(t) {
  const v = rangeRef.value
  const p = playing.value
  if (!v || !isRangeMode.value || !p) return
  const clamped = Math.min(p.end - RANGE_EPS, Math.max(p.start, Number(t) || 0))
  clampBusy.value = true
  try {
    v.currentTime = clamped
  } catch {}
  rangePos.value = clamped
  setTimeout(() => { clampBusy.value = false }, 0)
}

function rangeTimeAt(clientX) {
  const el = rangeBarRef.value
  if (!el || !(rangeDuration.value > 0)) return null
  const r = el.getBoundingClientRect()
  if (!r.width) return null
  const frac = Math.min(1, Math.max(0, (clientX - r.left) / r.width))
  return viewStart.value + frac * viewSpan.value // 按**视窗**换算成秒
}
function onRangePointerDown(e) {
  const t = rangeTimeAt(e.clientX)
  if (t == null) return
  e.preventDefault()
  try {
    rangeBarRef.value?.setPointerCapture?.(e.pointerId)
  } catch {}
  seekRange(t)
  window.addEventListener('pointermove', onRangePointerMove)
  window.addEventListener('pointerup', onRangePointerUp)
}
function onRangePointerMove(e) {
  const t = rangeTimeAt(e.clientX)
  if (t != null) seekRange(t)
}
function onRangePointerUp() {
  window.removeEventListener('pointermove', onRangePointerMove)
  window.removeEventListener('pointerup', onRangePointerUp)
}
function stopRangeDrag() {
  window.removeEventListener('pointermove', onRangePointerMove)
  window.removeEventListener('pointerup', onRangePointerUp)
}

/** metadata 到位后才写 currentTime —— 之前写会被忽略甚至抛（PrepareView.seekTo 同款教训）。 */
function onRangeMeta() {
  const v = rangeRef.value
  const p = playing.value
  if (!v || !isRangeMode.value || !p) return
  rangeDuration.value = Number(v.duration) || 0
  resetView() // 知道了整片多长，才能算"事件前后各留一点"的默认视窗
  try {
    v.currentTime = p.start
  } catch {}
  rangePos.value = p.start
  applyRangeAudio()
  v.play?.().catch(() => {}) // 浏览器可能因自动播放策略拒绝，交给用户点播放键
  startRangeClock()
}

/** timeupdate：到出点 → 停住（默认）或跳回入点（勾了「循环本段」）。 */
function onRangeTime() {
  const v = rangeRef.value
  const p = playing.value
  if (!v || !isRangeMode.value || !p || clampBusy.value) return
  if (v.currentTime < p.end - RANGE_EPS) return
  clampBusy.value = true
  try {
    if (loopRange.value) {
      v.currentTime = p.start
      rangePos.value = p.start
    } else {
      v.pause()
      v.currentTime = Math.max(p.start, p.end - RANGE_EPS)
      rangePos.value = v.currentTime
    }
  } catch {}
  setTimeout(() => { clampBusy.value = false }, 0)
}

/** seeking/seeked：拖到区间外就回弹（原生进度条没有范围概念，只能事后纠）。 */
function onRangeSeek() {
  const v = rangeRef.value
  const p = playing.value
  if (!v || !isRangeMode.value || !p || clampBusy.value) return
  const t = v.currentTime
  let back = null
  if (t < p.start - 0.2) back = p.start
  else if (t > p.end + 0.2) back = Math.max(p.start, p.end - RANGE_EPS)
  if (back == null) {
    // 区间内正常跳转：标记带的播放头要跟上（暂停时没有 rAF 在跑）
    rangePos.value = t
    return
  }
  clampBusy.value = true
  try {
    v.currentTime = back
  } catch {}
  rangePos.value = back
  setTimeout(() => { clampBusy.value = false }, 0)
}

function onRangeError() {
  toast('浏览器放不了这个原片格式（可能是 flv/mkv 或 HEVC 编码），可以先剪出来再看成片', 'err')
}

async function load() {
  if (props.projectData) {
    // props 模式：数据归父页面（切片页）所有，这里只是把那份快照**照抄**一份 —— 不重新取数。
    // 所以写完盘（删片段 / 改事件 / 新建事件）必须 `emit('changed')` 让父页面重新 load，
    // 光在这里 `load()` 等于什么都不做（删除后卡片不消失、再点一次就撞 404 的老 bug）。
    events.value = (props.projectData.events || []).slice().sort((a, b) => a.start_sec - b.start_sec)
    clips.value = props.projectData.clips || []
    polishMap.value = props.projectData.polishMap || {}
    applyDefaultFilter()
    return
  }
  loading.value = true
  loadError.value = ''
  try {
    const [ev, cl, pl] = await Promise.all([
      projectData.events(props.project.id, '读取事件列表').catch(() => ({ events: [] })),
      projectData.clips(props.project.id, '读取切片列表').catch(() => []),
      projectData.polishMap(props.project.id, '读取精修状态').catch(() => [])
    ])
    events.value = (ev.events || []).sort((a, b) => a.start_sec - b.start_sec)
    clips.value = cl || []
    const m = {}
    ;(pl || []).forEach(p => { m[p.dir] = p })
    polishMap.value = m
    applyDefaultFilter()
  } catch (e) {
    loadError.value = '片段数据加载失败：' + e.message
  } finally {
    loading.value = false
  }
}

async function run(kind, c) {
  if (kind === 'init') {
    const ok = await confirmDialog('将解析字幕/弹幕并创建 polish.toml', { title: '初始化该项目？' })
    if (!ok) return
  }
  if (kind === 'llm') {
    const ok = await confirmDialog('执行 LLM 生成后会自动 process 并刷新预览', { title: '执行 LLM 生成？' })
    if (!ok) return
  }
  try {
    await polishPost('/api/polish/' + kind, props.project.id, c.dir)
    toast('已启动 ' + kind)
    load()
  } catch (e) { toast(e.message, 'err') }
}

async function render(c) {
  const out = await promptDialog(
    '导出渲染视频的文件路径（可含文件夹，留空使用项目配置 output）。\n'
    + '同名文件不会被覆盖：已经存在时自动加 _1 / _2（与封面同一套规则）。',
    { title: '渲染输出', value: c.dir + '/export.mp4' }
  )
  if (out === null) return
  try {
    await runRender(props.project.id, c.dir, out || '')
    toast('渲染已启动')
  } catch (e) { toast(e.message, 'err') }
}

async function preview(c) {
  try {
    const r = await polishPost('/api/polish/preview', props.project.id, c.dir)
    const url = r.video_url || r.cover_url
    if (url) window.open(url, '_blank')
    else toast('预览返回空')
  } catch (e) { toast(e.message, 'err') }
}

async function redetect() {
  // 已经有检测结果时先问一遍（文案与预处理页共用，见 utils.js 的 redetectWarning）：
  // 重跑会整份覆盖 events.json，但不动 output/clips/ —— 已剪片段不会被删，
  // 只是事件按热度重新编号后，旧片段目录（clip-NN = rank）与新事件可能对不上。
  if (events.value.length) {
    const ok = await confirmDialog(redetectWarning(events.value.length), {
      title: '覆盖上一次的检测结果？',
      danger: true
    })
    if (!ok) return
  }
  if (outBusy.value) {
    toast('该项目正在' + kindText(outBusy.value.kind) + '，请等它结束', 'err')
    return
  }
  try {
    const r = await projectData.pipeline(props.project.id, { stage: 'detect', resume: 'auto' })
    if (r?.id) watchCutJob(r, '重新检测') // 盯一下进度，结束即刷新（以前不带 id，界面看不到动静）
    toast(r?.queued ? QUEUED_HINT : '重新检测已启动')
  } catch (e) { toast(e.message, 'err') }
}

// 剪切任务是异步的：发完请求就结束的话，列表永远停在「未剪切」，
// 这里盯住任务 id，终态（成功/失败/取消）时刷新列表。
//
// ⚠️ 这里**只刷任务列表**，不再 `load()` 项目数据：刷新项目数据由父页面的 `onJobDone`
// 负责（它那条路已经改成静默刷新）。在 props 模式下 `load()` 也只是照抄父页面那份快照，
// 调了等于什么都没做；而三条刷新路径叠起来正是慢盘上"任务结束整页卡 3–4 秒"的放大器。
function watchCutJob(started, label) {
  const r = started
  if (!r?.id) return
  watchJob(r.id, {
    onDone(s) {
      reloadJobs() // 立刻刷新全局任务列表，按钮的占用提示别多灰 3 秒
      if (s?.status === 'failed') toast(label + '失败：' + (s.message || '未知错误'), 'err')
    }
  })
}

async function cutAll() {
  const list = uncutEvents.value.slice()
  if (!list.length) return
  const ok = await confirmDialog(`将对 ${list.length} 个未剪切事件执行一键全切，是否继续？`, { title: '一键全切', danger: true })
  if (!ok) return
  try {
    const r = await projectData.cutAll(props.project.id, { ranks: list.map(ev => Number(ev.rank)) })
    toast(r?.queued
      ? `已排队：轮到时自动开始（共 ${list.length} 个事件，任务抽屉能看到排队位次）`
      : `已启动一键全切（${list.length} 个事件），可在“运行任务”查看整体进度`)
    watchCutJob(r, '一键全切')
  } catch (e) {
    toast('启动全切失败: ' + e.message, 'err')
    return
  }
  load()
}

async function cutEvent(ev) {
  const ok = await confirmDialog(`将剪切「${ev.label}」（${fmt(ev.start_sec)} → ${fmt(ev.end_sec)}）`, { title: '剪切该事件？' })
  if (!ok) return
  try {
    const r = await projectData.cutOne(props.project.id, ev.rank)
    toast(r?.queued ? QUEUED_HINT : '剪切任务已启动')
    watchCutJob(r, '剪切')
    load()
  } catch (e) { toast(e.message, 'err') }
}

function editEvent(ev) {
  if (!ev) return
  editing.value = ev
  editLabel.value = ev.label || ''
  editStart.value = ev.start_sec || 0
  editEnd.value = ev.end_sec || 0
  editType.value = ev.event_type || 'other'
  editSummary.value = ev.summary || ''
}

/**
 * 删除一个已剪切片段（破坏性，必须二次确认）：
 * 服务端删掉整个 `clips/<目录名>/` 目录（精修产物 / 成片 / 封面一并没），
 * 事件会回到「未剪切」，需要时可以重新剪。
 *
 * 目录名**原样传**给服务端（它按目录名解析，数字 rank 也照收）—— 卡片墙列的就是 `clips/`
 * 下的真实目录，名字不一定规范（`clip-out` 这类手工命名的目录以前会被解析成 rank 0，
 * 请求变成 `clips/0` → 永远报「clip-00 不存在」）。
 *
 * 删完 `emit('changed')` 让父页面重新取数：删除不产生任务，`onJobDone` 那条刷新路径管不到它，
 * 不刷新的话卡片留在墙上、再点一次就是 404（用户报过「删除报错找不到内容」）。
 * 正在预览的就是它时先关掉预览，否则播放器会指着已删除的文件报错。
 */
async function removeClip(c) {
  if (deletingDirs.value.has(c.dir)) return // 上一发还在删，别叠第二次（目录已没了 → 404）
  const label = clipView(c).title
  const ok = await confirmDialog(
    `将删除「${label}」（${c.name}）整个目录：精修产物、成片、封面一并删除，不可恢复。事件会回到「未剪切」，需要时可重新剪。`,
    { title: '删除片段？', danger: true }
  )
  if (!ok) return
  deletingDirs.value.add(c.dir)
  try {
    await projectData.deleteClip(props.project.id, c.name)
    if (playing.value?.name === c.name) playing.value = null
    toast('已删除 ' + c.name)
    load()
  } catch (e) {
    // 404 = 目录已经不在了（别处删过 / 列表是旧的）：不是失败，刷新一下让它从墙上消失
    if (e?.status === 404) toast('该片段已经不在了，列表已刷新')
    else toast('删除失败：' + e.message, 'err')
  } finally {
    deletingDirs.value.delete(c.dir)
    emit('changed')
  }
}

async function saveEdit() {
  if (!editing.value) return
  if (editEnd.value <= editStart.value) { toast('结束必须大于开始'); return }
  const ev = editing.value
  ev.label = editLabel.value
  ev.event_type = editType.value
  ev.summary = editSummary.value
  ev.start_sec = editStart.value
  ev.end_sec = editEnd.value
  ev.start = secToTs(editStart.value)
  ev.end = secToTs(editEnd.value)
  try {
    await projectData.saveEvents(props.project.id, events.value)
    editing.value = null
    toast('已保存')
    load()
    // 事件列表也归父页面：改标题/时间会改变"片段 ↔ 事件"的匹配结果（配套的还是变「独立」）
    emit('changed')
  } catch (e) { toast(e.message, 'err') }
}

/**
 * 去精修页。切片本身已经在**剪切收尾**里建好精修工程（`yekitrim-service::init_polish_for_events`），
 * 所以正常情况下这儿是"直接进"；只有在**老片段**（这次改动之前剪的）上才需要补一次 init。
 *
 * ⚠️ 补 init 必须**等任务跑完再跳**（与首页「切片总览」的 `openClip` 同一手感）：
 * `POST /api/polish/init` 建完任务就返回，工程还没落盘 —— 跳过去时 `GET /api/polish/project`
 * 会 500「polish.toml not found」，精修页渲染成「片段配置读取失败」，用户还得手动重试。
 */
async function goPolish(c) {
  const busy = clipBusyOf(c.name)
  if (busy) { toast('该片段正在' + kindText(busy.kind) + '，请等它结束', 'err'); return }
  // 双击防抖：第二次点击不该再发一次 init（服务端会因"已存在"把它记成失败任务）
  if (polishOpening.value) return
  const toPolish = { name: 'project-polish-clip', params: { pid: props.project.id, clip: c.name } }
  try {
    const list = await projectData.polishMap(props.project.id, '读取精修状态').catch(() => [])
    const found = (list || []).find(p => sameDir(p.dir, c.dir))
    if (found && found.polish_ready) {
      router.push(toPolish)
      return
    }
    // 传**绝对目录**（服务端 scope 守卫会校验它确实落在本人项目内）：
    // 相对路径要自己算 output_dir 的前缀，容易错（旧写法就退化成 'output'，静音/渲染会打错目录）。
    polishOpening.value = c.dir
    const r = await initPolish(props.project.id, c.dir)
    if (r?.queued) toast(QUEUED_HINT)
    toast('正在初始化精修工程，完成后自动进入…')
    await waitJobDone(r?.id)
    router.push(toPolish)
  } catch (e) {
    toast('精修初始化失败：' + e.message, 'err')
  } finally {
    polishOpening.value = ''
  }
}

/**
 * 把一次 `watchJob` 订阅包成 promise：终态 `succeeded` 才 resolve，其余按错误抛。
 * 订阅在终态里自己关（`watchJob::finish`），所以这里只记在 `jobClosers` 里供卸载时兜底。
 */
function waitJobDone(id) {
  return new Promise((resolve, reject) => {
    if (!id) { resolve(null); return }
    let settled = false
    const w = watchJob(id, {
      onDone(s) {
        if (settled) return
        settled = true
        if (s?.status === 'succeeded') resolve(s)
        else reject(new Error(s?.message || s?.error || '任务' + (s?.status || '异常结束')))
      }
    })
    jobClosers.push(() => w.close())
  })
}

/** 比较两条绝对目录（忽略大小写与分隔符差异）。 */
function sameDir(a, b) {
  const norm = (s) => String(s || '').split(String.fromCharCode(92)).join('/').toLowerCase()
  return norm(a) === norm(b)
}

async function addClip() {
  if (!newLabel.value) { toast('请填写标题'); return }
  if (newEnd.value <= newStart.value) { toast('结束必须大于开始'); return }
  try {
    const created = await projectData.createEvent(props.project.id, {
      label: newLabel.value,
      start_sec: newStart.value,
      end_sec: newEnd.value,
      event_type: newType.value,
      summary: newSummary.value
    })
    const rank = created && created.event && created.event.rank
    if (rank) await projectData.cutOne(props.project.id, rank)
    showAdd.value = false
    toast('片段已创建，正在剪切')
    load()
  } catch (e) { toast(e.message, 'err') } finally {
    // 新事件可能已经落盘（剪切那一步失败也拦不住它），父页面得重新取数才看得见
    emit('changed')
  }
}

function openAdd() {
  showAdd.value = true
}

// `cutAll` 给父级操作条的「一键全切」用（抽屉没了，批量动作提到页面操作条）
defineExpose({ redetect, openAdd, cutAll })

watch(() => props.projectData, load, { immediate: true })
watch(() => props.project?.id, load)

// 弹窗一关（playing = null）就停掉跟随播放头的 rAF、键盘与拖拽监听，别让它们挂在 window 上
watch(playing, (v) => {
  if (!v) {
    stopRangeClock()
    stopRangeDrag()
    window.removeEventListener('keydown', onRangeKey)
    rangePlaying.value = false
    rangeDuration.value = 0
    rangePos.value = 0
    clipPos.value = 0
    clipSubs.value = []
    clipSubsToken++
  }
})
onUnmounted(() => {
  stopRangeClock()
  stopRangeDrag()
  window.removeEventListener('keydown', onRangeKey)
  // 等 init 的订阅若还开着（用户中途走了），卸载时关掉；`watchJob` 在终态里也会自己关
  while (jobClosers.length) {
    try { jobClosers.pop()() } catch { /* 已关过就忽略 */ }
  }
})
</script>

<style scoped>
.sort-bar { display:flex; align-items:center; gap:6px; flex-wrap:wrap; margin-bottom:10px; }
.sort-bar .sort-count { margin-left:auto; }
/* 删除（破坏性）推到卡片右下角，离「精修」远一点，别误点 */
.clip-card .clip-del { margin-left:auto; }
.v-sep { width:1px; height:14px; background:var(--line); margin:0 4px; flex:none; }
/* 二级筛选（精修中 / 已导出）：做成挂在「已剪切」旁边的一个**内嵌分组**——
   虚线边 + 略浅的底 + 更小的字号，一眼看出它从属于左边那颗一级按钮，不是并列的第五档。
   组内选中的那颗用卡片白底「抬起」，与一级 `.mini-btn.selected` 的绿色块区分开。 */
.subfilters { display:inline-flex; align-items:center; gap:2px; padding:2px; margin:0 2px; flex:none;
  border:1px dashed var(--line); border-radius:var(--r-full); background:var(--bg-soft); }
.subfilters .sub-btn { display:inline-flex; align-items:center; padding:3px 10px; font-size:11px; line-height:1.5;
  border:0; border-radius:var(--r-full); background:transparent; color:var(--ink-dim);
  white-space:nowrap; cursor:pointer; transition:.15s; }
.subfilters .sub-btn:hover { color:var(--pink-text); }
.subfilters .sub-btn.selected { background:var(--card); color:var(--main-deep); font-weight:600;
  box-shadow:0 1px 3px rgba(216,99,83,.18); }
/* ---- 卡片固定分区：标题 / 媒体 / 标签 / 时间热度 / 摘要 / 工具栏 ----
   卡片撑满 grid 行高，工具栏用 margin-top:auto 钉底；各分区给固定高度，
   这样混排时「编辑 / 预览 / 剪切」与「编辑 / 精修 / 删除」始终落在同一条底线上。 */
.pcard.clip-card {
  display:flex; flex-direction:column;
  height:100%; min-height:390px;
  cursor:default; /* 卡片本身不响应点击（只有预览图会打开视频窗口） */
}
/* 标题固定三行：超长省略，不把下面区域顶下去 */
.clip-card .card-title { height:66px; min-height:66px; overflow:hidden; }
.clip-card .card-title h2 { display:-webkit-box; -webkit-line-clamp:3; -webkit-box-orient:vertical; overflow:hidden; }
/* 媒体区缩小并固定高度：已剪是缩略图，缺图才是虚线占位（未剪切事件整块不渲染） */
.media-slot { position:relative; height:112px; flex:none; margin-top:10px;
  border:1px solid var(--line); border-radius:var(--r-sm); overflow:hidden; background:var(--bg-soft); }
.media-slot.media-empty { display:flex; flex-direction:column; align-items:center; justify-content:center;
  gap:6px; border-style:dashed; color:var(--ink-faint); font-size:11px; }
.thumb { width:100%; height:100%; display:block; object-fit:cover; cursor:pointer; transition:filter .15s; }
.thumb:hover { filter:brightness(1.06); }
/* tag 统一成紧凑文字行，不再用多个彩色药丸；状态只改文字色，不再各自占一块背景 */
.tag-row { display:flex; flex-wrap:wrap; align-items:center; gap:3px 10px; min-height:22px; }
.tag { font-size:11px; line-height:1.5; white-space:nowrap; color:var(--ink-dim); }
.tag.b { color:var(--blue-deep); }
.tag.a { color:var(--amber-deep); }
.tag.g { color:var(--green-deep); }
.tag.r { color:var(--red-deep); }
.tag.n { color:var(--ink-dim); }
/* 时间/热度固定一行 */
.meta-slot { height:22px; overflow:hidden; white-space:nowrap; }
/* 摘要：已剪固定两行；未剪切事件没有媒体区，摘要吃掉剩余高度（最多 8 行） */
.summary-slot { height:40px; overflow:hidden; }
.summary-slot > div { display:-webkit-box; -webkit-line-clamp:2; -webkit-box-orient:vertical; overflow:hidden; }
.uncut-card .summary-slot { height:auto; flex:1; min-height:40px; }
.uncut-card .summary-slot > div { -webkit-line-clamp:8; }
/* 工具栏钉底：吃掉上面剩余空间；最小高度按最高的按钮算 */
.clip-card .foot { margin-top:auto; flex:none; min-height:50px; }
/* 未剪切事件卡：没有缩略图，用虚线顶边和「剪切」主按钮区分「还没剪」这件事 */
.uncut-card { border-style:dashed; }
/* 排队时按钮文案改成「排队中」；宽度用 min-width 固定住，文案切换不再把 foot 撑换行、
   也不会把卡片撑高。`nowrap` + 6px 间距保证最窄的 250px 卡片也放得下三个按钮。 */
.uncut-card .cut-btn { margin-left:auto; flex:none; min-width:72px; justify-content:center; white-space:nowrap; }
.uncut-card .foot { flex-wrap:nowrap; gap:6px; }
/* 「独立」= 重新检测后已经没有对应事件的片段（按 clip.json 冻结的信息显示） */
.orphan-chip { cursor:help; }

/* 未剪切事件预览：区间读数 + 循环开关，贴在原片播放器上方 */
.ev-preview-bar { display:flex; align-items:center; justify-content:space-between; gap:10px; flex-wrap:wrap; margin-bottom:8px; }
.loop-toggle { display:flex; align-items:center; gap:5px; cursor:pointer; color:var(--ink-dim); user-select:none; }
/* 全局 input{width:100%} 会把复选框撑满整行 */
.loop-toggle input { width:auto; }

/* 播放范围标记：整片时长条 + 高亮的事件区间带（左右边界 = 入点/出点）+ 带内播放头。
   自写播放控件里**只有这一条**进度条（不用原生 controls，避免两条进度条坐标系不一致）。
   视觉沿用预处理页轨道上那条 mark-band（主色半透明带 + 主色描边）。 */
.ev-stage { position:relative; display:flex; justify-content:center; background:#000; border-radius:var(--r-sm); overflow:hidden; container-type:inline-size; }
.ev-stage video { width:100%; max-height:70vh; cursor:pointer; }
/* 字幕叠层：与预处理页 .cap-overlay 同款（3.75cqw ≈ 72px@1920，奶白 + 深色描边） */
.ev-cap { position:absolute; left:5%; right:5%; bottom:15%; text-align:center; pointer-events:none;
  font-weight:700; font-size:3.75cqw; line-height:1.45; color:#dcbda0;
  -webkit-text-stroke:0.625cqw #403c3b; paint-order:stroke fill; }
.ev-controls { display:flex; align-items:center; gap:8px; margin-top:10px; flex-wrap:wrap; }
.ev-controls .time-label, .ev-time, .ev-win { flex:none; }
.ev-win { color:var(--ink-faint); }
.ev-vol { width:80px; flex:none; padding:0; border:none; background:transparent; }
.rng-track { position:relative; flex:1 1 240px; min-width:140px; height:16px; border-radius:var(--r-full);
  background:var(--bg-soft); border:1px solid var(--line-soft); overflow:hidden; cursor:pointer; touch-action:none; }
.rng-band { position:absolute; top:0; bottom:0; background:var(--main-soft);
  border-left:2px solid var(--main); border-right:2px solid var(--main); }
.rng-head { position:absolute; top:0; bottom:0; width:2px; margin-left:-1px; background:var(--red); pointer-events:none; }
</style>
