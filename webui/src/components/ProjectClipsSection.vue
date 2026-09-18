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
      <!-- 顺序：已剪切 / 未剪切 / 精修中 / **全部放最后**（用户要求） -->
      <button
        v-for="f in FILTERS"
        :key="f.key"
        class="mini-btn"
        :class="{ selected: filterKey === f.key }"
        :title="f.hint"
        @click="pickFilter(f)"
      >{{ f.label }}</button>
      <span class="muted small sort-count">
        {{ shownTotal }}<template v-if="filterKey !== 'all'">/{{ cutClips.length + uncutEvents.length }}</template> 个片段<template v-if="uncutEvents.length"> · 未剪切 {{ uncutEvents.length }}</template>
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
          <div class="head">
            <div>
              <h2>{{ it.view.title }}</h2>
            </div>
          </div>
          <!-- 只有点预览图才弹出视频预览，卡片其他区域不响应点击 -->
          <img
            v-if="hasThumb(it.view.c)"
            :src="thumbSrc(it.view.c)"
            class="thumb"
            alt="海报"
            title="点击预览片段"
            @click="previewClip(it.view.c)"
            @error="$event.target.style.display='none'"
          >
          <div class="chips mt-8">
            <span class="chip b">{{ Math.round(it.view.c.duration_secs) }}s</span>
            <span class="chip n">{{ Math.round(it.view.c.size_bytes / 1024 / 1024) }}MB</span>
            <span v-if="Number(it.view.c.danmaku_count)" class="chip a">弹幕 {{ it.view.c.danmaku_count }}</span>
            <span v-if="Number(it.view.c.subtitle_count)" class="chip n">字幕 {{ it.view.c.subtitle_count }}</span>
            <span
              v-if="it.view.orphan"
              class="chip n orphan-chip"
              title="重新检测后已经没有和它对应的事件了：这里按剪出时的信息显示，删掉重切才会重新关联"
            >独立</span>
            <!-- 不再显示「已去静音 / 屏蔽 N 段」：屏蔽轨道是精修编辑器内部的事（那里的时间轴有状态与撤销），
                 对外部列表没有意义 -->
            <!-- 导出相关**只剩一枚** chip（用户要求：「已精修」+「有封面」合并成「已导出」）：
                 出过片或出过封面都算 —— 两者都是"精修产物已经落盘"，分成两枚会让同一张卡
                 经常两枚并排，把"渲了没出封面 / 出了封面没渲"这种小差别放大得很显眼。
                 进过精修但还没导出 → 「精修中」；从没进过精修 → 不挂 chip（那状态就是"已剪切"本身）。 -->
            <span v-if="isExported(it.view.c)" class="chip g">已导出</span>
            <span v-else-if="polishMap[it.view.c.dir]?.polish_ready" class="chip b">精修中</span>
          </div>
          <div class="row mt-8">
            <span class="muted small mono">{{ fmt(it.view.start) }} → {{ fmt(it.view.end) }}</span>
            <span class="chip b">{{ Math.round(it.view.popularity * 100) }}%</span>
          </div>
          <div v-if="it.view.summary" class="muted small mt-8">{{ it.view.summary }}</div>
          <div class="foot">
            <button v-if="it.view.ev" class="mini-btn" :disabled="!!outBusy"
                    :title="outBusy ? '该项目正在' + kindText(outBusy.kind) : ''"
                    @click.stop="editEvent(it.view.ev)">编辑</button>
            <button class="mini-btn" :disabled="!!clipBusyOf(it.view.c.name)"
                    :title="clipBusyOf(it.view.c.name) ? '该片段正在' + kindText(clipBusyOf(it.view.c.name).kind) : ''"
                    @click.stop="goPolish(it.view.c)">精修 <Icon name="arrow-right" :size="11" /></button>
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
          <div class="head">
            <div>
              <h2>{{ it.ev.label }}</h2>
            </div>
          </div>
          <div class="chips mt-8">
            <span class="chip a">未剪切</span>
            <span class="chip b">{{ Math.round((it.ev.end_sec || 0) - (it.ev.start_sec || 0)) }}s</span>
            <span v-if="Number(it.ev.danmaku_count)" class="chip a">弹幕 {{ it.ev.danmaku_count }}</span>
            <span v-if="Number(it.ev.subtitle_count)" class="chip n">字幕 {{ it.ev.subtitle_count }}</span>
            <span
              v-if="occupiedRanks.has(Number(it.ev.rank))"
              class="chip r"
              title="这个序号上有一个属于别的事件的旧片段：先把它删掉再剪，否则会覆盖它"
            >序号被占用</span>
          </div>
          <div class="row mt-8">
            <span class="muted small mono">{{ fmt(it.ev.start_sec) }} → {{ fmt(it.ev.end_sec) }}</span>
            <span class="chip b">{{ Math.round((it.ev.popularity || 0) * 100) }}%</span>
          </div>
          <div v-if="it.ev.summary" class="muted small mt-8">{{ it.ev.summary }}</div>
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
                    @click.stop="cutEvent(it.ev)">{{ cutBusy ? '剪切（排队）' : '剪切' }}</button>
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
      <video v-else controls autoplay class="video-lg" :src="playing?.url"></video>
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
import { EVENT_TYPES, TYPE_ZH, fmtClock, mediaUrl, redetectWarning, clipMatchesEvent, uncutEventsOf } from '../utils'
import { readMasterVolume, readMuted, saveMasterVolume, saveMuted } from '../polish/audio'

const props = defineProps({
  project: { type: Object, required: true },
  projectData: { type: Object, default: null },
  /** 原片播放地址（`inputMediaUrl(status.input_abs)`，由切片页传入）；空串 = 拿不到源视频 */
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
// 「热度」是**数值**（0~1 的 popularity：弹幕峰值 0.6 + 均值 0.4 + 字幕密度，见 detection::rank_events），
// 不是 rank —— rank 只是按热度排出来的**名次**，重新检测后整体重排，旧片段就再也排不进去；
// popularity 两类卡片都有（事件在 events.json、片段在 clip.json 里冻结），所以能一起排。
const SORT_OPTIONS = [
  { key: 'popularity', label: '热度', dir: 'desc', hint: '按热度值排序（0~100%，越高越热）' },
  { key: 'start', label: '开始时间', dir: 'asc', hint: '按在原片里的开始时间排序' },
  { key: 'danmaku', label: '弹幕量', dir: 'desc', hint: '按弹幕条数排序' },
  { key: 'subtitle', label: '字幕量', dir: 'desc', hint: '按字幕条数排序' }
]
const SORT_STORE = 'vtrim.slices.sort'

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

// ── 筛选：已剪切 / 未剪切 / 精修中 / 全部（**全部放最后**，用户要求）──
// 「精修中」只对已剪片段有意义（未剪切事件还没进精修流程），且**含义放宽成"进过精修的都算"**：
// 包括已经导出的（用户要求把原来的「已精修＝已出片」那一档合并进来，不再单列）。
// 想只看已导出的，看卡片上那枚「已导出」chip —— 筛选栏不再为它单开一档。
// ⚠ key 从 `unpolished` 改成了 `polish`：含义变了（旧＝还没出片，新＝进过精修），
//   沿用旧 key 会让 localStorage 里存过的偏好静默指向另一批片段。
//   这里靠启动时的 `FILTERS.some(f => f.key === v)` 把两个旧 key 一并挡掉、回落到默认档。
const FILTERS = [
  { key: 'cut', label: '已剪切', hint: '已经剪出成片的片段' },
  { key: 'uncut', label: '未剪切', hint: '检测到但还没剪的事件' },
  { key: 'polish', label: '精修中', hint: '进过精修的片段（含已经导出的）' },
  { key: 'all', label: '全部', hint: '已剪切 + 未剪切全部显示' }
]
const FILTER_STORE = 'vtrim.slices.filter'
// 只在**用户自己点过**之后才记住；没存过时按数量定默认（见 applyDefaultFilter）——
// 存下来的默认值会在项目状态变化后过期（比如本来全是未剪切、后来全剪完了）。
const filterKey = ref((() => {
  try {
    const v = localStorage.getItem(FILTER_STORE) || ''
    return FILTERS.some(f => f.key === v) ? v : ''
  } catch { return '' }
})())

function pickFilter(f) {
  filterKey.value = f.key
  try { localStorage.setItem(FILTER_STORE, f.key) } catch {}
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
// 序号被别的片段占着的事件：剪之前得先清掉那个目录（服务端 Cutter 也会拒绝覆盖）
const occupiedRanks = computed(() => new Set(
  cutClips.value.filter(c => !matchedEventOf(c)).map(c => rankOf(c.name))
))

/**
 * 这枚片段"精修产物已经落盘"了吗 —— 出过片或出过封面都算。
 * 卡片上那枚「已导出」chip 与「进过精修」的判据都从这里来（`rendered`/`cover` 是服务端
 * `/api/project-data/polish` 按文件存在性给的，见 `polish_ready` 同一处）。
 */
const isExported = (c) => {
  const m = polishMap.value[c.dir]
  return !!(m?.rendered || m?.cover)
}
/** 「精修中」筛选 = 进过精修的（含已导出）。`polish_ready` 是 `polish.toml` 存在，
 *  产物文件都长在精修目录里，所以正常三者同真；这里仍 OR 一遍防单侧缺文件。 */
const isInPolish = (c) => {
  const m = polishMap.value[c.dir]
  return !!(m?.polish_ready || m?.rendered || m?.cover)
}
const cutCards = computed(() => cutClips.value.map(clipView))
/** 墙面上的条目：已剪片段 + 未剪切事件**按同一个排序键混排**（热度是数值，两边都算得出来）。 */
const shownItems = computed(() => {
  const k = filterKey.value
  const clips = k === 'uncut' ? [] : cutCards.value.filter(v =>
    k === 'polish' ? isInPolish(v.c) : true)
  const events = (k === 'uncut' || k === 'all') ? uncutEvents.value : []
  const items = [
    ...clips.map(v => ({ kind: 'clip', id: 'c:' + v.c.dir, view: v })),
    ...events.map(ev => ({ kind: 'event', id: 'e:' + ev.rank, ev }))
  ]
  return items.sort((a, b) => cmpStats(statsOfItem(a), statsOfItem(b)))
})
const statsOfItem = (it) => (it.kind === 'clip' ? clipStats(it.view.c) : eventStats(it.ev))
const shownTotal = computed(() => shownItems.value.length)

const emptyText = computed(() => {
  if (filterKey.value === 'uncut') return '暂无未剪切事件'
  if (filterKey.value === 'cut') return '暂无已剪切片段'
  if (filterKey.value === 'polish') return '还没有片段进过精修（去精修页跑一次）'
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
  playing.value = { name: c.name, url: mediaUrl(c.dir) }
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

const activeCaption = computed(() => (isRangeMode.value ? captionAt(rangePos.value) : ''))

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

async function goPolish(c) {
  const busy = clipBusyOf(c.name)
  if (busy) { toast('该片段正在' + kindText(busy.kind) + '，请等它结束', 'err'); return }
  try {
    const list = await projectData.polishMap(props.project.id, '读取精修状态').catch(() => [])
    const found = (list || []).find(p => sameDir(p.dir, c.dir))
    // 传**绝对目录**（服务端 scope 守卫会校验它确实落在本人项目内）：
    // 相对路径要自己算 output_dir 的前缀，容易错（旧写法就退化成 'output'，静音/渲染会打错目录）。
    if (!found || !found.polish_ready) {
      await initPolish(props.project.id, c.dir)
      toast('已自动 init')
    }
    // v5 路由：`/p/:pid/polish/:clip`（旧 `/project/:cfg/slices/:clip` 已不存在，会被 catch-all 送回首页）
    router.push({ name: 'project-polish-clip', params: { pid: props.project.id, clip: c.name } })
  } catch (e) { toast(e.message, 'err') }
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
  }
})
onUnmounted(() => {
  stopRangeClock()
  stopRangeDrag()
  window.removeEventListener('keydown', onRangeKey)
})
</script>

<style scoped>
.sort-bar { display:flex; align-items:center; gap:6px; flex-wrap:wrap; margin-bottom:10px; }
.sort-bar .sort-count { margin-left:auto; }
/* 删除（破坏性）推到卡片右下角，离「精修」远一点，别误点 */
.clip-card .clip-del { margin-left:auto; }
.v-sep { width:1px; height:14px; background:var(--line); margin:0 4px; flex:none; }
.thumb { width:100%; aspect-ratio:16/9; object-fit:cover; border-radius:var(--r-sm); margin-top:10px; border:1px solid var(--line); cursor:pointer; transition:filter .15s, border-color .15s; }
.thumb:hover { filter:brightness(1.06); border-color:var(--main); }
/* 卡片本身不再响应点击（只有预览图会打开视频窗口），去掉手型光标 */
.pcard.clip-card { cursor:default; }
/* 未剪切事件卡：没有缩略图，用虚线顶边和「剪切」主按钮区分「还没剪」这件事 */
.uncut-card { border-style:dashed; }
.uncut-card .cut-btn { margin-left:auto; }
/* 项目忙时这颗按钮会变成「剪切（排队）」，宽度多出两个字 —— 而 `.foot` 是**不换行**的
   flex 行（app.css），窄卡片上撑不下就会挤压按钮、把文字折成两行。允许换行比挤变形好，
   最多是多一行（`.cut-btn` 的 `margin-left:auto` 让它靠右单独成行，看着也是刻意的）。 */
.uncut-card .foot { flex-wrap:wrap; }
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
