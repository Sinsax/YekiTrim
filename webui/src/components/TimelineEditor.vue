<template>
  <div ref="rootRef" class="timeline" tabindex="0">
    <!-- 头部一行：调用方给的标题/提示（slot）+ 去静音 + 重新后处理 + 保存状态。
         这样页面不用再单独放一个卡片标题行 —— 时间轴本来就占竖高（用户要求精简）。 -->
    <div class="tl-head">
      <div class="tl-head-title"><slot name="head" /></div>
      <div class="row" style="gap:8px">
        <button
          class="mini-btn"
          :class="{ selected: silenceApplied }"
          :disabled="silenceBusy"
          :title="silenceApplied
            ? '从「屏蔽」轨道里减掉当初自动加进去的静音段，手动添加的屏蔽不受影响'
            : '自动检测片段里的长停顿，并入「屏蔽」轨道（与当前屏蔽相加：重叠的合成一段）；导出时剪掉，原始视频不动'"
          @click="toggleSilence"
        >
          <Icon name="sparkles" :size="11" />
          {{ silenceBusy ? '处理中…' : (silenceApplied ? '撤销去静音' : '去静音') }}
        </button>
        <button class="mini-btn" :disabled="reprocessBusy" title="按当前 polish.toml 重跑一次后处理：贴纸落点、运镜锚点、画中画守卫都会重算"
                @click="emit('reprocess')">
          <Icon name="refresh" :size="11" /> 重新后处理
        </button>
        <!-- 单级撤销：只保留**最近一次删除**（双击/面板删的块或分段）。做这一枚是因为
             删除是"双击即生效 + 700ms 落盘 + 随后自动后处理"，误删点完就已经不可逆了。 -->
        <button v-if="lastDeleted" class="mini-btn" :title="undoTitle" @click="undoDelete">
          撤销删除
        </button>
        <span class="muted small">
          {{ silenceApplied
            ? `已自动屏蔽 ${silenceCount} 段静音 · 屏蔽轨道共 ${muteRanges.length} 段`
            : `一键跳过片段里的长停顿${muteRanges.length ? `（当前已屏蔽 ${muteRanges.length} 段，会自动相加）` : ''}` }}
        </span>
      </div>
    </div>
    <!-- 读取失败：这份时间轴的内容**不可信**（可能整个是空的，也可能是上一个片段的残留）。
         必须摆在这里（不放进 Teleport 的属性面板 —— 那个挂载点没解析到就整块不渲染），
         并且同时**暂停写入**：否则一次拖拽就会把空数组 PUT 回去，抹掉磁盘上的分段/字幕。 -->
    <div v-if="loadError" class="error-banner">
      <Icon name="alert" :size="15" />
      <span>时间轴数据读取失败，<b>已暂停保存</b>：{{ loadError }}</span>
      <button class="mini-btn" style="margin-left:auto" :disabled="loadRetrying" @click="retryLoad">
        {{ loadRetrying ? '重试中…' : '重试' }}
      </button>
    </div>
    <!-- 视觉特效轨道的 ＋ ：先选细分类型再加（fixed 定位，不受时间轴横向滚动与卡片裁切影响）。
         类型由轨道按钮分配，面板里只允许在同轨道的细分类型之间换。 -->
    <div v-if="fxMenu" class="fx-menu" :style="{ left: fxMenu.x + 'px', top: fxMenu.y + 'px' }">
      <div class="fx-menu-t muted small">在播放头处添加</div>
      <button v-for="t in FX_TYPES" :key="t" class="fx-menu-item" @click="addEventAt(t)">
        {{ TYPE_LABELS[t] }}<span class="muted">{{ t }}</span>
      </button>
    </div>
    <!-- 属性面板：用 Teleport 投到页面给的挂载点（PolishView 把它放在视频左侧，与时间轴分离）。
         选中状态与全部编辑逻辑仍留在这个组件里，页面只决定它出现在哪儿。
         ⚠️ Teleport 的目标是**挂载时**用 document.querySelector 解析的：本组件挂载时，
         整个编辑器子树还没插进 document（Vue 先建子节点、最后才 hostInsert），
         直接写 :to="propsTarget" 会解析不到、面板完全不渲染（只剩一块空白）。
         所以用 teleportTo 门控，等 onMounted（本组件的 mounted 在 post-flush 阶段跑，
         那时页面已插入文档）再设值。 -->
    <Teleport v-if="teleportTo" :to="teleportTo">
      <div class="props-panel card" @input="onPanelEdit" @change="onPanelCommit">
        <div class="row spread">
          <b>{{ panelTitle }}</b>
          <button v-if="selected" class="mini-btn" @click="selected = null">关闭</button>
        </div>
        <div v-if="!selected" class="props-empty muted small">点击时间轴上的分段 / 字幕 / 屏蔽 / 事件块，在这里编辑属性</div>
        <template v-else-if="selItem">
          <!-- ── 标配：开始 / 结束 / 入点 / 出点 / 定位（所有类型都有） ── -->
          <div class="row">
            <div class="flex-1"><label>开始（秒）</label>
              <input v-model.number="selStart" type="number" step="0.01" min="0"></div>
            <div class="flex-1"><label>结束（秒）</label>
              <input v-model.number="selEnd" type="number" step="0.01" min="0"></div>
          </div>
          <div class="row mt-6" style="flex-wrap:wrap;gap:4px">
            <button class="mini-btn" title="把「开始」设为当前播放时间" @click="setInPoint">设为入点</button>
            <button class="mini-btn" title="把「结束」设为当前播放时间" @click="setOutPoint">设为出点</button>
            <button class="mini-btn" title="预览跳到这个块的开始" @click="seekToStart">定位</button>
            <button v-if="canSplit" class="mini-btn" title="从当前播放时间拆成两块" @click="splitSelected">拆分</button>
            <button v-if="selected.kind === 'event'" class="mini-btn" @click="duplicateSelected">复制</button>
          </div>
          <div v-if="spanHint" class="hint mt-4">{{ spanHint }}</div>
          <!-- 贴纸/运镜的时间：手工改过之后后处理就不动它了，这里给状态与回退口 -->
          <div v-if="MANUAL_TYPES.includes(selItem.type)" class="manual-row mt-4">
            <span class="chip" :class="selItem.manual ? 'p' : 'n'">{{ selItem.manual ? '手动' : '自动对齐' }}</span>
            <span class="muted small">{{ selItem.manual
              ? '后处理不改写它：不吸附字幕、也不参与数量抽稀'
              : '后处理会吸附到最近字幕；数量超出额度时可能被抽稀删掉' }}</span>
            <button v-if="selItem.manual" class="mini-btn" title="交回自动处理：下一次后处理会把它吸附到最近字幕（数量超出额度时也可能被抽稀）" @click="followSubtitles">跟随字幕</button>
          </div>

          <!-- ── 类型专属 ── -->
          <template v-if="selected.kind === 'seg'">
            <label>备注</label>
            <input v-model="selItem.note" placeholder="分段备注">
            <label>情绪</label>
            <select v-model="selItem.fx">
              <option value="low">low · 安静叙述</option>
              <option value="med">med · 日常</option>
              <option value="high">high · 爆点</option>
            </select>

            <!-- 最近一次 LLM 生成：这一段成/败 + 只重做这一段。
                 整片重做很贵（一次 LLM 调用一段），而且限流时总会有一两段落空；
                 以前失败**完全不可见**，用户只看到"这段 20 秒没事件"。 -->
            <div class="seg-gen mt-6">
              <div class="row spread small" style="gap:6px;align-items:center">
                <span class="muted">上次生成</span>
                <span v-if="!segGen(selected.si)" class="chip n">无记录</span>
                <span v-else-if="segGen(selected.si).ok" class="chip p">成功 · {{ segGen(selected.si).events }} 条</span>
                <span v-else class="chip warn">失败</span>
              </div>
              <div v-if="segGen(selected.si) && !segGen(selected.si).ok" class="mini-hint">
                {{ segGen(selected.si).reason || '没有产出事件' }}（这一段的事件保持原样，没被清空）
              </div>
              <div v-else-if="llmStatus && llmStatus.ts" class="muted small mt-4">
                整片最近一次：{{ llmStatus.ok }}/{{ llmStatus.total }} 段成功 · {{ fmtTs(llmStatus.ts) }}
              </div>
              <button class="mini-btn mt-6" style="width:100%;justify-content:center"
                      :disabled="reprocessBusy"
                      title="只重新生成这一段的事件（其余分段、筛选词/高亮词都不动）；失败会保留该段原事件"
                      @click="emit('redo-segment', selected.si)">
                <Icon name="sparkles" :size="11" /> 只重做本段事件
              </button>
            </div>
          </template>

          <template v-else-if="selected.kind === 'sub'">
            <label>字幕内容</label>
            <textarea v-model="selItem.text" rows="2"></textarea>
          </template>

          <template v-else-if="selected.kind === 'event'">
            <!-- 类型由所在轨道决定：只有「视觉特效」轨道有 11 个细分类型可换，
                 其余轨道（运镜/贴纸/覆盖层/音效）在这里只读展示 —— 想换类型就删掉重加。 -->
            <template v-if="isFxType(selItem.type)">
              <label>特效类型</label>
              <select :value="selItem.type" @change="changeEventType($event.target.value)">
                <option v-for="t in FX_TYPES" :key="t" :value="t">{{ TYPE_LABELS[t] }}（{{ t }}）</option>
              </select>
            </template>
            <div v-else class="type-ro">
              <span class="muted small">类型</span>
              <b>{{ TYPE_LABELS[selItem.type] || selItem.type }}</b>
              <span class="muted small mono">{{ selItem.type }}</span>
            </div>

            <!-- 运镜 -->
            <template v-if="selItem.type === 'camera'">
              <label>方式</label>
              <select :value="selItem.cut ? 'cut' : 'smooth'" @change="selItem.cut = $event.target.value === 'cut'">
                <option value="smooth">平滑运镜（0.6s 推移）</option>
                <option value="cut">硬切（瞬间切换）</option>
              </select>
              <label>景别</label>
              <select :value="selItem.zoom" @change="setZoom(selItem, $event.target.value)">
                <option value="远">远（画中画）</option>
                <option value="中">中景</option>
                <option value="近">近景</option>
              </select>
              <!-- 手持摇摆：运镜的附加变种，门槛只在近景（用户定的口径）。
                   关掉即删字段（保持配置干净）；景别改离近景由 setZoom 顺带剥掉。 -->
              <label class="ck-row" :title="selItem.zoom === '近' ? '叠加在运镜上的持续摇摆' : '只在近景可用'">
                <input type="checkbox" :checked="selItem.handheld === true" :disabled="selItem.zoom !== '近'"
                       @change="setHandheld(selItem, $event.target.checked)">
                手持摇摆<span class="muted small">（仅近景）</span>
              </label>
              <div class="row">
                <div class="flex-1">
                  <label>聚焦</label>
                  <select :value="selItem.anchor || ''" @change="setEvField(selItem, 'anchor', $event.target.value)">
                    <option value="">（不指定）</option>
                    <option value="liver">主播</option>
                    <option value="danmaku">弹幕</option>
                    <option value="sticker">贴纸</option>
                    <option value="center">画面中央</option>
                  </select>
                </div>
                <div class="flex-1">
                  <label>移到</label>
                  <select :value="selItem.anchor_to || ''" @change="setEvField(selItem, 'anchor_to', $event.target.value)">
                    <option value="">（原地不动）</option>
                    <option value="liver">主播</option>
                    <option value="danmaku">弹幕</option>
                    <option value="sticker">贴纸</option>
                    <option value="center">画面中央</option>
                  </select>
                </div>
              </div>
              <div class="hint">锚点在「后处理」时解析成坐标；「移到」非空就是从聚焦点平移过去。<br>
                手持摇摆：持续晃动到下一条不带它的运镜，只在近景生效（非近景会被后处理剔除）。</div>
            </template>

            <!-- 贴纸：keyword 一级 → 贴纸二级 -->
            <template v-else-if="selItem.type === 'sticker'">
              <StickerPicker :lib="stickerLib" :pid="pid" :dir="dir"
                             :pack="selItem.keyword || ''" :tag="selItem.sticker_tag || ''"
                             :name="selItem.name || ''" :source="selItem.source || ''"
                             @pick="onStickerPick" />
              <div class="hint">位置 x/y 由后处理算（每次重跑覆盖）。</div>
            </template>

            <!-- 覆盖层 -->
            <template v-else-if="selItem.type === 'overlay'">
              <label>覆盖类型</label>
              <select v-model="selItem.overlay_type" @change="onOverlayTypePick(selItem, $event.target.value)">
                <option v-for="o in OVERLAY_TYPES" :key="o.value" :value="o.value">{{ o.label }}</option>
              </select>
              <template v-if="selItem.overlay_type === 'gradient'">
                <div class="row">
                  <div class="flex-1"><label>起色</label>
                    <input type="color" :value="gradientColor(selItem, 0)" @input="setGradientColor(selItem, 0, $event.target.value)"></div>
                  <div class="flex-1"><label>终色</label>
                    <input type="color" :value="gradientColor(selItem, 1)" @input="setGradientColor(selItem, 1, $event.target.value)"></div>
                </div>
                <p v-if="!(selItem.colors && selItem.colors.length >= 2)" class="mini-hint">
                  未设色档：按「该色 → 透明」渲染。改动任一色档即成为两档渐变。
                </p>
                <label>形状</label>
                <select v-model="selItem.shape">
                  <option value="linear">线性（左上 → 右下）</option>
                  <option value="radial">径向（中心实、边缘淡出）</option>
                </select>
              </template>
              <!-- 淡入 / 淡出：整块就是过渡本身（遮罩由全遮线性化到全透，反向同理），
                   所以这一档没有形状，颜色缺省就是黑色 -->
              <template v-else-if="isOverlayFade(selItem)">
                <label>颜色</label>
                <input type="color" :value="hexOf(selItem.color, '#000000')" @input="selItem.color = $event.target.value">
                <p class="mini-hint">整块时长＝渐变时长，拖右缘改。</p>
              </template>
              <template v-else>
                <label>颜色</label>
                <input type="color" :value="hexOf(selItem.color)" @input="selItem.color = $event.target.value">
              </template>
              <label>不透明度（{{ selItem.opacity ?? 1 }}）</label>
              <input v-model.number="selItem.opacity" type="number" step="0.05" min="0" max="1">
            </template>

            <!-- 纯音效 -->
            <template v-else-if="selItem.type === 'sfx'">
              <label>音效</label>
              <SfxPicker :entries="sfxEntries" :value="selItem.name || ''" @pick="selItem.name = $event" />
              <label>音量（{{ selItem.volume ?? 0.3 }}）</label>
              <input v-model.number="selItem.volume" type="number" step="0.05" min="0" max="2">
            </template>

            <!-- 瞬时特效 -->
            <template v-else>
              <template v-if="INTENSITY_TYPES.includes(selItem.type)">
                <label>强度（{{ selItem.intensity ?? 10 }}）</label>
                <input v-model.number="selItem.intensity" type="number" step="1" min="0">
              </template>
              <template v-if="COLOR_TYPES.includes(selItem.type)">
                <label>颜色</label>
                <input type="color" :value="hexOf(selItem.color)" @input="selItem.color = $event.target.value">
              </template>
            </template>

            <!-- 所有带 sfx 引用的事件：配声 -->
            <template v-if="SFX_CAPABLE.includes(selItem.type)">
              <label>配声（可选）</label>
              <SfxPicker :entries="sfxEntries" :value="selItem.sfx || ''" placeholder="筛选音效…"
                         @pick="selItem.sfx = $event || undefined" />
              <label>配声音量（{{ selItem.sfx_volume ?? 0.2 }}）</label>
              <input v-model.number="selItem.sfx_volume" type="number" step="0.05" min="0" max="2">
            </template>
          </template>

          <div class="row mt-8">
            <button class="mini-btn danger" @click="deleteSelected">删除</button>
            <span class="muted small" style="margin-left:auto">{{ saveText }}</span>
          </div>
        </template>
      </div>
    </Teleport>

    <!-- 没有分段时**整条时间轴照常渲染**，这里只给一句提示（不再放「新建分段」按钮：
         用户指出那是为「删空分段后整块消失」那个 bug 加的临时入口，bug 已修，位置让给去静音）。
         以前是 `v-if` 把整块换成"暂无时间轴数据，请先运行 LLM 自动生成分段" —— 删掉最后一段后
         时间轴直接消失，连字幕/屏蔽都没法改（用户报过）。 -->
    <div v-if="!segments.length" class="tl-no-seg">
      <span class="muted small">还没有分段：事件要挂在分段上（字幕 / 屏蔽不受影响）。跑一次「LLM 剪辑」即可生成分段。</span>
    </div>
    <div ref="bodyRef" class="timeline-body" @wheel.passive="markUserScroll" @pointerdown="markUserScroll">
      <div class="timeline-labels">
        <div v-for="lane in lanes" :key="'label-' + lane.type" class="lane-label">
          <span>{{ lane.label }}</span>
          <div class="lane-adds">
            <button v-if="lane.type !== 'segments'" class="add-lane"
                    :title="lane.type === 'fx' ? '在播放头处添加视觉特效（先选细分类型）' : '在播放头处添加'"
                    @click.stop="onLaneAdd(lane.type, $event)"><Icon name="plus" :size="11" /></button>
            <button v-else class="add-lane mute-add" title="在播放头处添加屏蔽区间" @click="addMuteAtCurrent"><Icon name="scissors" :size="11" /></button>
          </div>
        </div>
        <div class="lane-label ruler-label">时间</div>
    </div>
    <div ref="tracksRef" class="timeline-tracks">
      <div v-for="lane in lanes" :key="'track-' + lane.type" class="lane">
        <div class="lane-track" :style="{ width: trackWidth + 'px' }">
          <svg v-if="lane.type === 'subtitles' && wavePath" class="wave-layer" :width="trackWidth" :height="WAVE_H">
            <!-- 双层描边：这条线要同时压在不透明的字幕块（`#3b82f6`）**和**近白背景
                 （`--bg` #FFF7F3）上，单色必然在一边糊掉 —— 深色 halo 负责近白底上的边，
                 亮色芯负责蓝块上的对比。改颜色前先想清楚落在哪两种底上。 -->
            <path :d="wavePath" fill="none" stroke="rgba(15,23,42,.55)" :stroke-width="WAVE_HALO_W" stroke-linejoin="round" stroke-linecap="round" />
            <path :d="wavePath" fill="none" stroke="#ffffff" :stroke-width="WAVE_CORE_W" stroke-linejoin="round" stroke-linecap="round" />
          </svg>
          <div class="playhead" :style="{ left: (currentTime || 0) * scale + 'px' }"></div>
          <div
            v-for="(block, i) in laneBlocks(lane)"
            :key="lane.type + '-' + i"
            class="block"
            :class="[block.cls, { selected: isBlockSelected(lane, block) }]"
            :style="{ left: block.left + 'px', width: block.width + 'px', background: block.color }"
            :title="block.title"
            @click="onBlockClick(lane, block)"
            @pointerdown.stop="startPointer($event, lane, block, 'move')"
            @dblclick.stop="onBlockDblClick(block)"
          >
            <span v-if="block.resizable && block.leftResizable" class="resize resize-l" @pointerdown.stop="startPointer($event, lane, block, 'resize-l')"></span>
            <span v-if="block.resizable" class="resize" @pointerdown.stop="startPointer($event, lane, block, 'resize-r')"></span>
            {{ block.label }}
          </div>
        </div>
      </div>
      <div class="lane ruler-lane">
        <div ref="rulerTrackRef" class="lane-track ruler-track" :style="{ width: trackWidth + 'px' }"
             @pointerdown="onRulerPointerDown">
          <div v-for="(tk, i) in timeTicks" :key="'tick' + i" class="ruler-tick" :style="{ left: tk.left + 'px' }">
            <span class="ruler-time">{{ tk.label }}</span>
          </div>
          <div class="playhead ruler-playhead" :style="{ left: (currentTime || 0) * scale + 'px' }"></div>
        </div>
      </div>
    </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { confirmDialog } from '../stores/dialog'
import { addClient } from '../stores/log'
import { toast } from '../stores/toast'
import Icon from './Icon.vue'
import StickerPicker from './polish/StickerPicker.vue'
import SfxPicker from './polish/SfxPicker.vue'
import { TIMELINE_COLORS } from '../utils'
import {
  fetchProject, fetchSubtitles, fetchSfxCatalog, fetchStickerLibrary,
  saveProjectPatch, saveSubtitles, runSilence,
} from '../api/polish'

const props = defineProps({
  dir: { type: String, required: true },
  /** 项目 id（贴纸缩略图接口需要作用域参数） */
  pid: { type: [String, Number], default: '' },
  currentTime: { type: Number, default: 0 },
  /** 预览是否正在播放：**只有播放时**才跟随播放头（用户要求：拖动/跳转不该自己动视角） */
  playing: { type: Boolean, default: false },
  /** 后处理任务是否在跑（「重新后处理」按钮的禁用态） */
  reprocessBusy: { type: Boolean, default: false },
  /** 属性面板的 Teleport 挂载点（CSS 选择器）；默认由 PolishView 渲染在视频左侧 */
  propsTarget: { type: String, default: '#tl-props-host' }
})
// `preview` = 本地改动立刻镜像给页面（预览实时刷新，不落盘、不重挂载）
// `done`    = **一次编辑真的做完了**（面板控件确定 / 取消选中 / 切片段 / 离开页面）——
//             页面据此跑一次后处理。落盘成功**不算**完成：那时你可能还在改这个事件。
const emit = defineEmits(['seek', 'toggle', 'preview', 'done', 'reprocess', 'retried', 'redo-segment'])
const rootRef = ref(null)
/**
 * 属性面板的 Teleport 目标。**必须等 onMounted 才设值**：本组件挂载时整棵编辑器子树
 * 还没插进 document，这时用选择器解析会拿到 null，Teleport 就整块不渲染。
 */
const teleportTo = ref('')
onMounted(() => {
  const target = String(props.propsTarget || '')
  if (!target) return
  const attach = () => {
    if (!document.querySelector(target)) return false
    teleportTo.value = target
    return true
  }
  if (attach()) return
  // 兜底：挂载点万一还没进文档（过渡/异步组件），下一帧再试一次
  nextTick(() => {
    if (!attach()) console.warn('[TimelineEditor] 属性面板挂载点不存在，属性面板不会显示：', target)
  })
})
const tracksRef = ref(null)
const bodyRef = ref(null)
const rulerTrackRef = ref(null)
const segments = ref([])
/** 最近一次 LLM 生成的结果快照（`polish_project_get?preview=1` 注入的 `_llm_status`）：
 *  哪一段成功/失败、失败原因、生成时间。用于给分段上警示色 + 面板里显示状态。 */
const llmStatus = ref(null)
const subtitles = ref([])
const muteRanges = ref([])
const loudness = ref([])
const scale = ref(60)
const duration = ref(60)
const trackWidth = computed(() => Math.max((duration.value || 60) * scale.value, 600))
const timeTicks = computed(() => {
  const dur = duration.value || 60
  const step = dur > 300 ? 60 : dur > 120 ? 30 : dur > 60 ? 10 : 5
  const out = []
  for (let t = 0; t <= dur + 0.001; t += step) {
    out.push({ t, left: t * scale.value, label: t.toFixed(0) + 's' })
  }
  return out
})
// ── 响度包络线 ────────────────────────────────────────────────
// ⚠️ 泳道高度：改 `.lane` / `.lane-track` 的 26px 时，要同步 `WAVE_H` 与下方 SVG 的 height。
const WAVE_H = 26
/**
 * 电平窗口（LUFS）——「看得出高低差」的关键就在这两个数。
 * 以前用 [-60, 0]，而实测语音落在 **-12 ~ -32**（静音 -33 ~ -56），
 * 于是整段语音的高度只在 11~17.6px 之间抖，26px 泳道里**动态范围只有 6.6px**，看着就是一条平线。
 * 收成 [-40, -8] 后同样的语音占 4.5~20.1px，动态范围 12.9px —— 约两倍，且没有削顶风险。
 */
const WAVE_DB_TOP = -8
const WAVE_DB_BOTTOM = -40
/** 上下各留的余量（px）：描边有宽度，贴边会被泳道裁掉半个线宽 */
const WAVE_PAD = 1.5
/** 每列多少像素（列数 = 轨道宽度 / 这个值）。2px 已比 0.1s 的采样间隔细，不会丢细节 */
const WAVE_COL_STEP = 2
const WAVE_HALO_W = 3.4
const WAVE_CORE_W = 1.5

/**
 * 响度包络线路径（`M…L…`，未闭合、只描边）。
 *
 * 从「一个采样点一根矩形的柱状图」改成「按像素列取峰值的折线」：采样是定长 0.1s，
 * 柱状图两头都不对 —— 缩小时（1px/s）10 根柱子挤进 1px 糊成一片，放大时（120px/s）
 * 点距 12px 而柱宽只有 1px、中间全是缝。按列聚合后一列最多一个顶点，
 * **顶点位置永远跟着缩放走**，既不糊也不出缝。
 *
 * 取每列的**最大** db（峰值保持）：这是波形显示的常规做法，也正好是编辑器要的语义 ——
 * "这一刻最响到过多少"。取均值会把瞬时爆点抹平，而找爆点正是看这条线的目的。
 */
const wavePath = computed(() => {
  const pts = loudness.value || []
  if (!pts.length) return ''
  const dur = duration.value || 60
  const s = scale.value || 1
  // 只画到内容结束为止：`trackWidth` 有 600px 下限，片段很短时轨道比内容宽，那一段没有数据
  const width = Math.min(trackWidth.value, Math.ceil(dur * s) + WAVE_COL_STEP)
  const yOf = (db) => {
    const n = (db - WAVE_DB_BOTTOM) / (WAVE_DB_TOP - WAVE_DB_BOTTOM)
    return (WAVE_H - WAVE_PAD) - Math.max(0, Math.min(1, n)) * (WAVE_H - WAVE_PAD * 2)
  }
  const parts = []
  let firstY = null
  let i = 0
  // 单趟线性扫（点是按时间递增的），不是每列重新找一遍
  for (let x = 0; x <= width; x += WAVE_COL_STEP) {
    const tEnd = (x + WAVE_COL_STEP) / s
    let peak = null
    while (i < pts.length && (Number(pts[i].t) || 0) <= tEnd) {
      const db = Number(pts[i].db)
      if (Number.isFinite(db)) peak = peak === null ? db : Math.max(peak, db)
      i++
    }
    if (peak === null) continue      // 放大到比采样还细时会有空列：跳过，折线直接连过去
    const y = yOf(peak)
    if (!parts.length) {
      firstY = y
      parts.push('M' + x.toFixed(0) + ' ' + y.toFixed(2))
    } else {
      parts.push('L' + x.toFixed(0) + ' ' + y.toFixed(2))
    }
  }
  // 只有一个顶点连不成线（只有 M），补一段短横线
  if (parts.length === 1) parts.push('L' + WAVE_COL_STEP + ' ' + firstY.toFixed(2))
  return parts.length > 1 ? parts.join('') : ''
})

// ── 选中态：`{ kind: 'seg'|'event'|'sub'|'mute', si?, ei?, subIndex?, muteIndex?, lane }` ──
const selected = ref(null)
const selectedSeg = computed(() => {
  if (!selected.value || selected.value.kind !== 'seg') return null
  return segments.value[selected.value.si] || null
})
const selectedEv = computed(() => {
  if (!selected.value || selected.value.kind !== 'event') return null
  const seg = segments.value[selected.value.si]
  return seg && seg.events ? seg.events[selected.value.ei] || null : null
})
const selectedSub = computed(() => {
  if (!selected.value || selected.value.kind !== 'sub') return null
  const i = selected.value.subIndex
  return i != null ? (subtitles.value[i] || null) : null
})
const selectedMute = computed(() => {
  if (!selected.value || selected.value.kind !== 'mute') return null
  const i = selected.value.muteIndex
  return i != null ? (muteRanges.value[i] || null) : null
})
const selItem = computed(() => selectedSeg.value || selectedEv.value || selectedSub.value || selectedMute.value)
const panelTitle = computed(() => {
  if (!selected.value) return '属性'
  return { seg: '分段属性', event: '事件属性', sub: '字幕属性', mute: '屏蔽属性' }[selected.value.kind] || '属性'
})

// 视觉特效泳道的**细分类型**（顺序 = 面板下拉与 ＋ 菜单的顺序）。
// 类型不是让用户从 15 种里随便挑的：**轨道上的 ＋ 负责定类型**，
// 面板里只允许在同一个轨道的细分类型之间换（全片只有这一条轨道有 11 个细分）。
const FX_TYPES = ['shake', 'flash', 'blur', 'zoom_bounce', 'vignette', 'hue_shift',
  'stutter', 'split', 'noise', 'pulse', 'color_shift']

const LANE_DEFS = [
  { type: 'segments', label: '分段 / 屏蔽', color: TIMELINE_COLORS.segments, resizable: true },
  { type: 'subtitles', label: '字幕', color: 'rgba(59,130,246,0.35)', resizable: true },
  { type: 'camera', label: '运镜', color: TIMELINE_COLORS.camera, resizable: true },
  { type: 'sticker', label: '贴纸', color: TIMELINE_COLORS.sticker, resizable: true },
  { type: 'fx', label: '视觉特效', color: TIMELINE_COLORS.flash, types: FX_TYPES, resizable: true },
  { type: 'overlay', label: '覆盖层', color: TIMELINE_COLORS.overlay, resizable: true },
  { type: 'sfx', label: '音效', color: TIMELINE_COLORS.sfx, resizable: true }
]
const lanes = LANE_DEFS

/** 类型中文名（只用于展示；可选性由轨道决定，见 `isFxType`）。 */
const TYPE_LABELS = {
  camera: '运镜', sticker: '贴纸', shake: '抖动', flash: '闪白', blur: '模糊',
  zoom_bounce: '缩放弹跳', vignette: '暗角', hue_shift: '色相偏移', stutter: '卡顿',
  split: '撕裂', noise: '噪点', pulse: '脉冲', color_shift: '色偏',
  overlay: '覆盖层', sfx: '音效'
}
const isFxType = (t) => FX_TYPES.includes(t)
/** 覆盖类型（`overlay_type`）→ 中文名：属性面板下拉与泳道块标签共用一份，别各写一遍。 */
const OVERLAY_TYPES = [
  { value: 'color', label: '纯色' },
  { value: 'gradient', label: '渐变' },
  { value: 'fade_in', label: '淡入（黑场）' },
  { value: 'fade_out', label: '淡出（黑场）' }
]
const overlayLabel = (t) => OVERLAY_TYPES.find((o) => o.value === t)?.label || t || '覆盖'
/** 淡入/淡出是「整块就是过渡本身」的两档（与出片模板同口径）：只有颜色 + 不透明度有意义。 */
const isOverlayFade = (ev) => ev.overlay_type === 'fade_in' || ev.overlay_type === 'fade_out'
/** 新建覆盖层的默认色/不透明度（`defaultEvent` 给的那两个值）：切到淡入/淡出时按值迁移成黑色遮罩。 */
const OVERLAY_NEW_COLOR = '#ff3232'
const OVERLAY_NEW_OPACITY = 0.3
// 与 Rust 端 Event::default_end 的默认时长逐值一致
const DEFAULT_SPAN = {
  camera: 1.0, sticker: 2.0, overlay: 1.5, sfx: 1.0,
  shake: 0.3, flash: 0.25, blur: 0.3, zoom_bounce: 0.4, vignette: 0.4,
  hue_shift: 0.35, stutter: 0.25, split: 0.25, noise: 0.3, pulse: 0.4, color_shift: 0.35
}
const INTENSITY_TYPES = ['shake', 'stutter', 'split']
const COLOR_TYPES = ['flash', 'hue_shift', 'color_shift']
/** 时间会被后处理吸附/延长的类型（有 `manual` 字段的那两类）。 */
const MANUAL_TYPES = ['sticker', 'camera']
const SFX_CAPABLE = ['sticker', 'shake', 'flash', 'blur', 'zoom_bounce', 'vignette',
  'hue_shift', 'stutter', 'split', 'noise', 'pulse', 'color_shift', 'overlay']

const round2 = (v) => Math.round((Number(v) || 0) * 100) / 100
const span0 = (type) => DEFAULT_SPAN[type] ?? 0.5

/** 事件跨度（秒）= end - time；老数据的 duration 兜底（与 engine.js evDuration 一致）。 */
function evDur(ev) {
  if (ev.end != null) return Math.max(0.05, Number(ev.end) - (Number(ev.time) || 0))
  if (ev.duration != null) return Math.max(0.05, Number(ev.duration) || 0.05)
  return span0(ev.type)
}
/** 写回「开始 + 结束」，并清掉老字段（下一次保存即完成迁移）。 */
function setEvSpan(ev, start, end) {
  ev.time = round2(start)
  ev.end = round2(end)
  delete ev.duration
}

function getDuration() { return duration.value || 60 }

// ── 贴纸库 / 音效目录（属性面板的列表数据）──
const stickerLib = ref([])
const sfxEntries = ref([])
async function loadPickers() {
  try {
    const r = await fetchStickerLibrary(props.pid, props.dir, '读取贴纸库')
    stickerLib.value = r?.packs || []
  } catch { stickerLib.value = [] }
  try {
    const r = await fetchSfxCatalog(props.pid, props.dir, '读取音效目录')
    sfxEntries.value = r?.entries || []
  } catch { sfxEntries.value = [] }
}

// 运镜锚点：空值 = 删除该字段。绑成空串会被 serde 读成 Some("")，解析时报「无法识别」
function setEvField(ev, key, value) {
  if (value === '') delete ev[key]
  else ev[key] = value
}
/**
 * 手持摇摆开关（运镜的附加变种）：关掉即删字段，保持配置干净 ——
 * `Event::Camera::handheld` 是 `skip_serializing_if` 的可选布尔，老配置零迁移。
 */
function setHandheld(ev, on) {
  if (on) ev.handheld = true
  else delete ev.handheld
}
/**
 * 景别切换：离开近景时顺带剥掉手持摇摆 —— 与后处理守卫（`process::fx::cap_handheld_sway`）
 * 同一语义「只剥不补」，免得留下一个"看着开着、实际必被剔除"的悬空开关。
 */
function setZoom(ev, z) {
  ev.zoom = z
  if (z !== '近') delete ev.handheld
}
/** `#rgb` / `#rrggbb` → 取色器能认的 `#rrggbb`（认不出时给 `fallback`，淡入/淡出用黑色）。 */
function hexOf(v, fallback = '#ffffff') {
  const s = String(v || '').trim()
  if (/^#[0-9a-f]{6}/i.test(s)) return s.slice(0, 7)
  if (/^#[0-9a-f]{3}$/i.test(s)) return '#' + s.slice(1).split('').map((c) => c + c).join('')
  // rgb()/rgba()：老事件里有过 `rgba(255,50,50,0.3)` 这种写法（alpha 归「不透明度」管），
  // 不认的话取色器会一律显示白色、首次编辑就把原色丢掉
  const m = s.match(/^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/i)
  if (m) return '#' + [m[1], m[2], m[3]].map((n) => Math.min(255, Number(n)).toString(16).padStart(2, '0')).join('')
  return fallback
}
function gradientColor(ev, i) {
  const arr = ev.colors || []
  // 面板显示的必须是**实际生效**的色档：没设 colors 时起色就是 ev.color（单色 → 透明）
  if (arr[i]) return hexOf(arr[i])
  if (i === 0 && ev.color) return hexOf(ev.color)
  return '#000000'
}
function setGradientColor(ev, i, color) {
  const arr = [...(ev.colors || [])]
  while (arr.length < 2) arr.push(arr.length === 0 ? hexOf(ev.color) : '#ffffff')
  arr[i] = color
  ev.colors = arr
}

/**
 * 覆盖类型切换。淡入/淡出是「黑色遮罩」的语义，而新建覆盖层给的是 `#ff3232` + 0.3 ——
 * 切过去先得手改两处才能用。所以只把**还是新建默认值**的那两项按值迁移成黑色 + 全遮
 * （与 Rust 侧「旧内建默认按值迁移」同一套做法），用户改过的一律原样保留。
 */
function onOverlayTypePick(ev, ot) {
  ev.overlay_type = ot
  if (ot !== 'fade_in' && ot !== 'fade_out') return
  if (!ev.color || hexOf(ev.color) === OVERLAY_NEW_COLOR) ev.color = '#000000'
  if (ev.opacity == null || Number(ev.opacity) === OVERLAY_NEW_OPACITY) ev.opacity = 1
}

function labelFor(ev) {
  if (ev.type === 'sticker') return ev.keyword || ev.name || '贴纸'
  if (ev.type === 'camera') {
    // 硬切的块加个「切」前缀：泳道上一眼能看出哪几段是瞬间换构图（用户要的切镜）
    // 「摇」= 手持摇摆（附加变种），排在最前：它是叠加状态，跟切的"瞬间换构图"是两回事
    const z = (ev.handheld ? '摇·' : '') + (ev.cut ? '切·' : '') + (ev.zoom || '运镜')
    if (ev.anchor && ev.anchor_to) return `${z} ${ev.anchor}→${ev.anchor_to}`
    return ev.anchor ? `${z} ${ev.anchor}` : z
  }
  if (ev.type === 'sfx') return ev.name || '音效'
  if (ev.type === 'overlay') return overlayLabel(ev.overlay_type)
  return ev.type
}

/**
 * 泳道 → 块。每个块都带 `kind`，拖拽/选中/属性面板都按它分发 ——
 * 字幕、屏蔽、分段、事件现在**同一套控件**（拖动 / 左右缘缩放 / 双击删除）。
 * 唯一例外：分段**不给左缘** —— 缩它的 start 会把事件留在段外，正确处理要连带搬事件，
 * 风险太大（体拖已经把事件一起搬了，改起点请用属性面板）。
 */
/**
 * 最近一次 LLM 生成里这一段的结果（没有记录 = 没跑过、或跑在旧版本上）。
 *
 * 用它给分段上色：**成功就是当前颜色**，只有"没产出事件"的段才换成警示色。
 * 这条以前完全没有 —— 限流导致某一窗失败时，界面上和"模型写得少"长得一模一样
 * （用户 2026-09-20 报的就是"某段 20 秒没有任何事件，也看不出为什么"）。
 */
function segGen(si) {
  const list = llmStatus.value?.segments
  if (!Array.isArray(list) || si === undefined || si === null) return null
  return list.find((s) => Number(s.index) === Number(si)) || null
}

/** unix 秒 → `MM-DD HH:MM`（本地时区），用于"整片最近一次生成"那行。 */
function fmtTs(ts) {
  const d = new Date(Number(ts) * 1000)
  if (Number.isNaN(d.getTime())) return ''
  const p = (n) => String(n).padStart(2, '0')
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

function laneBlocks(lane) {
  const out = []
  if (lane.type === 'subtitles') {
    subtitles.value.forEach((sub, i) => {
      const start = Number(sub.start) || 0
      const end = Number(sub.end) || start
      const text = (sub.text || sub.html || '').replace(/<[^>]+>/g, ' ').trim()
      out.push({
        kind: 'sub', subIndex: i,
        time: start,
        left: start * scale.value,
        width: Math.max((end - start) * scale.value, 10),
        label: text.length > 18 ? text.slice(0, 18) + '…' : text,
        title: '字幕 ' + start.toFixed(2) + 's → ' + end.toFixed(2) + 's ' + text,
        color: '#3b82f6',
        resizable: true,
        leftResizable: true,
        minLen: 0.1,
        cls: 'type-subtitles'
      })
    })
    return out
  }
  if (lane.type === 'segments') {
    segments.value.forEach((seg, si) => {
      // 上次 LLM 生成没产出事件的段：换个颜色标出来（成功 = 当前颜色，用户要求）
      const gen = segGen(si)
      const bad = !!gen && gen.ok === false
      out.push({
        kind: 'seg', si,
        time: seg.start,
        left: seg.start * scale.value,
        width: Math.max((seg.end - seg.start) * scale.value, 8),
        label: seg.note || ('分段 ' + (si + 1)),
        title: '分段 ' + Number(seg.start).toFixed(1) + 's → ' + Number(seg.end).toFixed(1) + 's'
          + (bad ? `\n⚠ 上次 LLM 生成失败：${gen.reason || '没有产出事件'}（选中它可「只重做本段事件」）` : ''),
        color: bad ? '#f59e0b' : TIMELINE_COLORS.segments,
        resizable: true,
        leftResizable: false,
        minLen: 0.1,
        cls: bad ? 'type-segments seg-gen-failed' : 'type-segments'
      })
    })
    muteRanges.value.forEach((m, i) => {
      const start = Number(m.start) || 0
      const end = Number(m.end) || start
      out.push({
        kind: 'mute', muteIndex: i,
        time: start,
        left: start * scale.value,
        width: Math.max((end - start) * scale.value, 10),
        label: '屏蔽',
        title: '屏蔽 ' + start.toFixed(2) + 's → ' + end.toFixed(2) + 's',
        color: '#ef4444',
        resizable: true,
        leftResizable: true,
        minLen: 0.05,
        cls: 'type-mutes'
      })
    })
    return out
  }
  segments.value.forEach((seg, si) => {
    ;(seg.events || []).forEach((ev, ei) => {
      const type = ev.type || ''
      const types = lane.types || [lane.type]
      if (!types.includes(type)) return
      const dur = evDur(ev)
      const t = Number(ev.time) || 0
      out.push({
        kind: 'event', si, ei,
        time: t,
        left: t * scale.value,
        width: Math.max(dur * scale.value, 10),
        label: labelFor(ev),
        title: type + ' @ ' + t.toFixed(2) + 's → ' + (t + dur).toFixed(2) + 's',
        color: TIMELINE_COLORS[type] || lane.color,
        resizable: true,
        leftResizable: true,
        minLen: 0.1,
        cls: 'type-' + type
      })
    })
  })
  return out
}

function isBlockSelected(lane, block) {
  const s = selected.value
  if (!s || s.lane !== lane.type) return false
  if (block.kind !== s.kind) return false
  if (block.kind === 'sub') return s.subIndex === block.subIndex
  if (block.kind === 'mute') return s.muteIndex === block.muteIndex
  return s.si === block.si && s.ei === block.ei
}

/** 块 → 它背后的对象（拖拽/面板都直接改它）。 */
function blockItem(block) {
  if (block.kind === 'sub') return subtitles.value[block.subIndex] || null
  if (block.kind === 'mute') return muteRanges.value[block.muteIndex] || null
  if (block.kind === 'seg') return segments.value[block.si] || null
  const seg = segments.value[block.si]
  return seg && seg.events ? seg.events[block.ei] || null : null
}
function selectBlock(lane, block) {
  if (block.kind === 'sub') selected.value = { kind: 'sub', lane: lane.type, subIndex: block.subIndex }
  else if (block.kind === 'mute') selected.value = { kind: 'mute', lane: lane.type, muteIndex: block.muteIndex }
  else selected.value = { kind: block.kind, lane: lane.type, si: block.si, ei: block.ei }
}
/** 块/选中 → 落盘分组（决定 PUT 到哪个接口、以及要不要跑后处理）。 */
const dirtyKindOf = (kind) => (kind === 'sub' ? 'subtitles' : kind === 'mute' ? 'mute' : 'segments')

function seek(t) { emit('seek', t) }

function onBlockClick(lane, block) {
  selectBlock(lane, block)
  // 字幕/屏蔽点击只选中不跳时间（配合「设为入/出点」用）；事件/分段顺带把预览定位过去
  if (block.kind === 'seg' || block.kind === 'event') seek(block.time)
}

function onBlockDblClick(block) {
  // 快捷删除：字幕 / 屏蔽 / 事件（分段是破坏性操作，只能在面板里确认后删）
  if (block.kind === 'sub') { selected.value = { kind: 'sub', lane: 'subtitles', subIndex: block.subIndex }; deleteSelectedSubtitle(); return }
  if (block.kind === 'mute') { deleteMuteAt(block.muteIndex); return }
  if (block.kind === 'event') { selected.value = { kind: 'event', lane: '', si: block.si, ei: block.ei }; deleteSelected() }
}

// ── 自动保存 ────────────────────────────────────────────────────
// 改动 → ①立刻 emit('preview') 镜像给页面（预览实时变，不重挂载）②700ms 防抖 PUT 落盘。
// **落盘不等于完成**：后处理要等 `emit('done')`（见下面的 commitEdit），
// 否则你加完贴纸还在面板里填 keyword 时它就会插进来处理这个半成品。
const dirty = { segments: false, subtitles: false, mute: false }
const saveState = ref('')   // '' | 'saving' | 'ok' | 'err'
const saveErr = ref('')
const savedAt = ref('')
/** 读不到时间轴数据时的原因（非空 = 基线不可信 → **一律不写盘**，见 `flushSave` 开头）。 */
const loadError = ref('')
const loadRetrying = ref(false)
let saveTimer = null
let previewTimer = null
let saving = false
/** 本次脏数据连续失败的次数（成功即清零）：失败后自己退避重试，别干等下一次编辑。 */
let saveAttempts = 0

const saveText = computed(() => {
  // 读取失败优先：这时写着"已保存"或"改动会自动保存"都是骗人的
  if (loadError.value) return '读取失败，已暂停保存'
  if (saveState.value === 'saving') return '保存中…'
  if (saveState.value === 'err') return '保存失败：' + saveErr.value
  if (saveState.value === 'ok') return '已保存 ' + savedAt.value
  return '改动会自动保存'
})

function touch(kind) {
  markDirty(kind)
  clearTimeout(saveTimer)
  saveTimer = setTimeout(flushSave, 700)
}
/** 只标脏 + 节流镜像（拖拽过程中用：预览要跟着动，落盘等松手）。 */
function markDirty(kind) {
  dirty[kind] = true
  editTouched = true
  schedulePreview()
}
/** 拖拽过程中用节流镜像（60ms），别每个 pointermove 都重建预览索引。 */
function schedulePreview() {
  if (previewTimer) return
  previewTimer = setTimeout(() => { previewTimer = null; emitPreview() }, 60)
}
function emitPreview() {
  const payload = {}
  if (dirty.segments) payload.segments = segments.value
  if (dirty.subtitles) payload.subtitles = subtitles.value
  if (dirty.mute) payload.mute_ranges = muteRanges.value
  if (Object.keys(payload).length) emit('preview', payload)
}
/** 面板里任何输入都算「选中项被改了」（容器上事件委托，省掉每个字段挂一遍）。 */
function onPanelEdit() {
  if (selected.value) touch(dirtyKindOf(selected.value.kind))
}
/** 面板控件的 **change**（失焦 / 下拉选定 / 滑块松手）＝ 这一项改完了 → 可以跑后处理。 */
function onPanelCommit() {
  onPanelEdit()
  commitEdit('面板确定')
}

// ── 「编辑完成」信号 ─────────────────────────────────────────────
// 后处理只在收到 `done` 之后跑，而且**只在一次编辑真的做完**的时候发：
// 面板控件确定 / 取消选中 / 切片段 / 离开页面。
//
// 老做法是"落盘成功后 1.2s 自动跑"，于是你刚点「＋」加完贴纸、还在面板里填 keyword 的
// 时候它就插进来了 —— 半成品被当成成品处理：数量抽稀会把新贴纸删掉、吸附会把时间挪走，
// 用户看到的是"我自己加的东西过两秒就没了"。
let editTouched = false // 有改动、还没跑过后处理

function commitEdit(reason, dirOverride) {
  if (!editTouched) return
  editTouched = false
  // 切片段时 props.dir 已经是新的，必须显式传**旧目录**，否则后处理会跑到新片段上
  emit('done', { dir: dirOverride || props.dir, reason })
}
// 取消选中（点空白 / 点别的块 / 删除后 / 点面板「关闭」）= 这一轮编辑结束了
watch(selected, (now, before) => { if (before && !now) commitEdit('取消选中') })

/** 轻量归一化：只保证区间合法 + 2 位小数，**不重排**（重排会让选中下标漂移）。 */
function normalizeLocal() {
  if (dirty.subtitles) {
    subtitles.value.forEach((s) => {
      const st = Number(s.start) || 0
      let en = Number(s.end)
      if (!Number.isFinite(en) || en <= st) en = st + 0.1
      s.start = round2(st); s.end = round2(en)
      if (!s.html) s.html = s.text || ''
    })
  }
  if (dirty.mute) {
    muteRanges.value.forEach((m) => {
      const st = Number(m.start) || 0
      let en = Number(m.end)
      if (!Number.isFinite(en) || en <= st) en = st + 0.05
      m.start = round2(st); m.end = round2(en)
    })
  }
}

async function flushSave() {
  // ⚠️ 定时器已经触发，必须把句柄清掉：`reload()` 用 `saveTimer` 判断"还有没有待落的改动"，
  // 留着这个（已失效的）句柄会让它**永远提前返回** —— 后处理改写完事件后编辑器就再也不会同步。
  saveTimer = null
  // 读失败时基线不可信：数组可能是空的、也可能是上一个片段的残留，写下去就是把真数据抹掉。
  // 脏标记**留着**，等重试读到数据后由 `reload()` 先落盘再重载。
  if (loadError.value) return
  if (saving) { scheduleSaveRetry(); return }
  const segs = dirty.segments ? segments.value : null
  const mutes = dirty.mute ? muteRanges.value : null
  const subs = dirty.subtitles ? subtitles.value : null
  if (!segs && !mutes && !subs) return
  normalizeLocal()
  saving = true
  saveState.value = 'saving'
  try {
    if (segs || mutes) {
      const patch = {}
      if (segs) patch.segments = segs
      if (mutes) patch.mute_ranges = mutes
      await saveProjectPatch(props.pid, props.dir, patch)
    }
    if (subs) {
      await saveSubtitles(props.pid, props.dir, subs)
    }
    dirty.segments = dirty.subtitles = dirty.mute = false
    saveAttempts = 0
    saveState.value = 'ok'
    savedAt.value = new Date().toTimeString().slice(0, 8)
    // 这里**不**通知页面跑后处理：落盘 ≠ 编辑完成（见 commitEdit）
  } catch (e) {
    saveState.value = 'err'
    saveErr.value = e.message
    addClient('warn', '时间轴保存失败：' + e.message, 'timeline')
    toast('时间轴保存失败：' + e.message, 'err')
    // 失败后自己退避重试（最多 3 次）。以前只有"下一次编辑"才会重试 ——
    // 网络盘抖一下那次改动就一直挂在 dirty 上，界面写着"保存失败"却再也不动了。
    if (++saveAttempts <= 3) { clearTimeout(saveTimer); saveTimer = setTimeout(flushSave, 3000) }
  } finally {
    saving = false
    drainQueuedReload()
  }
}
function scheduleSaveRetry() {
  clearTimeout(saveTimer)
  saveTimer = setTimeout(flushSave, 400)
}

/**
 * 重新从磁盘拉一份（后处理跑完后由页面调用，把吸附/落点结果同步回块与面板）。
 *
 * ⚠️ 这里**不许静默丢弃**。老写法是 `if (drag || saving || saveTimer) return`：页面在
 * `process` 跑完后调它来同步，可只要当时在 700ms 防抖窗口里、或用户正拖着块，这次同步就被
 * 无声跳过 —— 编辑器留着后处理**之前**的值，紧接着那次 `flushSave` 又把旧值 PUT 回去，
 * 把刚算出来的贴纸落点/锚点坐标整个顶掉，而且再也没人重试。
 *
 * 现在：忙 → 排队（`reloadQueued`），空闲时由 `flushSave` 的 finally 补做；真要跑的这一刻
 * 若手上还有没落盘的改动，**先落盘再重载** —— 用户的改动不丢。代价是这一次后处理的改写被
 * 顶掉，但渲染导出前本来还会再跑一次 `process`（`cmd_render` 第一件事就是它），下次就重算
 * 回来了，所以只留痕、不拦。
 */
let reloadQueued = false
let reloadRunning = false

async function reload() {
  if (reloadRunning || drag || saving) { reloadQueued = true; return }
  reloadRunning = true
  try {
    reloadQueued = false
    const pending = !!saveTimer || dirty.segments || dirty.subtitles || dirty.mute
    if (saveTimer) { clearTimeout(saveTimer); saveTimer = null }
    // 基线不可信（上次读失败）时不能落盘：空数组/上一个片段的内容写下去就是把真数据抹掉
    const canFlush = pending && !loadError.value
    if (canFlush) {
      addClient('warn', '重载时有未落盘的改动：已先落盘再重载，本次后处理的改写待下次重算', 'timeline')
      await flushSave()
    }
    const ok = await loadData()
    // 只有"这次真读回来了、而且手上那几处改动没能落盘"才谈得上被覆盖 —— 读又失败时什么都没载入，
    // 这时弹"改动已被磁盘内容覆盖"是假消息。
    if (ok && pending && !canFlush) {
      addClient('warn', '读取失败期间编辑过：已按磁盘内容重新载入，那几处改动被覆盖', 'timeline')
      toast('读取失败期间的改动已被磁盘内容覆盖', 'err')
    }
    // 后处理刚跑完：这之前的改动已经被它消化过了，别为同一批改动再触发一次
    editTouched = false
  } finally {
    reloadRunning = false
    // 重载期间又有人要求重载（页面任务回调 / 用户操作）→ 再跑一轮，不丢意图。
    // 此时 reloadRunning 已复位，只有"忙"才会再次入队并留到下次 drain。
    if (reloadQueued) { reloadQueued = false; reload() }
  }
}

/** 把排队的那次重载做掉（`flushSave` 结束 / 拖拽结束时调；还忙就再排一次）。 */
function drainQueuedReload() {
  if (!reloadQueued || reloadRunning) return
  reload()
}

/** 读取失败横幅上的「重试」。 */
async function retryLoad() {
  if (loadRetrying.value) return
  loadRetrying.value = true
  try {
    await reload()
    // 读回来了 → 告诉页面：同一条 `/api/polish/project` 在它那边也失败过，那条横幅已经过期。
    // 不通知的话，页面那条「显示的是上一次的数据」会一直挂着，明明已经好了 —— 这正是要消灭的
    // "界面在说谎"。页面收到后自己重拉一次（silent，不卸载编辑器）。
    if (!loadError.value) emit('retried')
  } finally { loadRetrying.value = false }
}

/** 换片段前把状态清干净：旧数组属于上一个片段，留着就是跨片段串写的引信。 */
function resetTimelineState() {
  segments.value = []
  subtitles.value = []
  muteRanges.value = []
  loudness.value = []
  duration.value = 60
  dirty.segments = dirty.subtitles = dirty.mute = false
  saveState.value = ''
  saveErr.value = ''
  saveAttempts = 0
  lastDeleted.value = null
}

// ── 拖拽（所有块统一：体拖 = 移动；左缘 = 改开始；右缘 = 改结束）──
let drag = null

function startPointer(e, lane, block, mode = 'move') {
  const item = blockItem(block)
  if (!item) return
  e.preventDefault()
  e.stopPropagation()
  selectBlock(lane, block)
  const startX = e.clientX
  const dur = getDuration()
  const kind = block.kind
  const dk = dirtyKindOf(kind)
  const minLen = block.minLen || 0.1
  const isSeg = kind === 'seg'
  const origStart = isSeg || kind === 'event' ? (Number(item.time) || 0) : (Number(item.start) || 0)
  const origSpan = kind === 'event' ? evDur(item) : Math.max(minLen, (Number(item.end) || 0) - origStart)
  const origEnd = origStart + origSpan
  // 分段体拖要连带搬段内事件：先快照原始时间，避免逐帧累加漂移
  const origEvents = isSeg ? (item.events || []).map((x) => Number(x.time) || 0) : null

  function put(start, end) {
    const s = Math.max(0, Math.min(dur - (end - start), start))
    if (kind === 'event') { setEvSpan(item, s, s + (end - start)); markManual(item) }
    else { item.start = round2(s); item.end = round2(s + (end - start)) }
    return s
  }

  const apply = {
    move(evt) {
      const delta = (evt.clientX - startX) / scale.value
      let ns = origStart + delta
      ns = Math.max(0, Math.min(dur - origSpan, ns))
      if (isSeg) {
        item.start = round2(ns)
        item.end = round2(ns + origSpan)
        ;(item.events || []).forEach((x, i) => {
          x.time = round2(Math.min(dur, Math.max(0, (origEvents[i] ?? 0) + (ns - origStart))))
        })
      } else {
        put(ns, ns + origSpan)
      }
      markDirty(dk)
    },
    'resize-l'(evt) {
      const delta = (evt.clientX - startX) / scale.value
      const ns = Math.max(0, Math.min(origEnd - minLen, origStart + delta))
      put(ns, origEnd)
      markDirty(dk)
    },
    'resize-r'(evt) {
      const delta = (evt.clientX - startX) / scale.value
      const ne = Math.max(origStart + minLen, Math.min(dur, origEnd + delta))
      put(origStart, ne)
      markDirty(dk)
    }
  }[mode]

  if (!apply) return
  drag = { move: apply }
  const onMove = (evt) => { if (drag) drag.move(evt) }
  const onUp = () => {
    removeListeners()
    drag = null
    touch(dk)
  }
  function removeListeners() {
    window.removeEventListener('pointermove', onMove)
    window.removeEventListener('pointerup', onUp)
    window.removeEventListener('pointercancel', onUp)
  }
  window.addEventListener('pointermove', onMove)
  window.addEventListener('pointerup', onUp)
  window.addEventListener('pointercancel', onUp)
}

// ── 面板：标配字段 ──────────────────────────────────────────────
const itemStart = computed(() => {
  const it = selItem.value
  if (!it) return 0
  return selected.value.kind === 'event' ? (Number(it.time) || 0) : (Number(it.start) || 0)
})
const itemEnd = computed(() => {
  const it = selItem.value
  if (!it) return 0
  return selected.value.kind === 'event' ? itemStart.value + evDur(it) : (Number(it.end) || 0)
})
const selStart = computed({
  get: () => round2(itemStart.value),
  set: (v) => {
    const it = selItem.value
    if (!it || !Number.isFinite(Number(v))) return
    const minLen = selected.value.kind === 'mute' ? 0.05 : 0.1
    const s = Math.max(0, Math.min(itemEnd.value - minLen, Number(v)))
    writeSpan(it, s, itemEnd.value)
    touch(dirtyKindOf(selected.value.kind))
  }
})
const selEnd = computed({
  get: () => round2(itemEnd.value),
  set: (v) => {
    const it = selItem.value
    if (!it || !Number.isFinite(Number(v))) return
    const minLen = selected.value.kind === 'mute' ? 0.05 : 0.1
    const e = Math.min(getDuration(), Math.max(itemStart.value + minLen, Number(v)))
    writeSpan(it, itemStart.value, e)
    touch(dirtyKindOf(selected.value.kind))
  }
})
function writeSpan(item, start, end) {
  if (selected.value.kind === 'event') { setEvSpan(item, start, end); markManual(item) }
  else { item.start = round2(start); item.end = round2(end) }
}

/**
 * 手工产物打 `manual` 标记：**后处理不再改写它**。
 * - 不改时间：`align_stickers_to_subtitles` / Step 2c / `align_pip_shots` 都跳过它
 *   （否则用户改了时间，1 秒后后处理把值吸回去，看起来就是"改了没成功"）。
 * - 不参与数量抽稀：`process::cap_events` 跳过它（否则"我新建的贴纸过两秒自己没了"）。
 * 触发点：手工改时间、新建事件、复制事件。面板上的「跟随字幕」清掉它（交回自动处理）。
 * 只有贴纸与运镜有这个字段，其余类型是 no-op。
 */
function markManual(ev) {
  if (!ev) return
  if (ev.type === 'sticker' || ev.type === 'camera') ev.manual = true
}
/** 「跟随字幕」：清掉手动标记，下一次后处理恢复自动吸附（数量超额度时也可能被抽稀）。 */
function followSubtitles() {
  const ev = selectedEv.value
  if (!ev) return
  delete ev.manual
  touch('segments')
}
const canSplit = computed(() => selected.value && (selected.value.kind === 'sub' || selected.value.kind === 'mute'))
const spanHint = computed(() => {
  const ev = selectedEv.value
  if (!ev) return ''
  if (ev.type === 'sticker') return '贴纸会在「结束」之后再淡出 0.3s；入场 0.35s / 浮起 0.4s 也另计。'
  if (ev.type === 'camera') return '运镜是状态机：档位保持到下一条运镜；「远」（画中画）的结束时间 = 到点回镜头。'
  if (SFX_CAPABLE.includes(ev.type) && ev.type !== 'overlay') return '瞬时特效只表现爆发瞬间（后处理会把跨度夹到 0.1~0.6s）。'
  return ''
})

function setInPoint() {
  const it = selItem.value
  if (!it) return
  const t = round2(Math.max(0, Number(props.currentTime) || 0))
  if (t >= itemEnd.value) { toast('当前时间必须早于结束时间', 'err'); return }
  writeSpan(it, t, itemEnd.value)
  touch(dirtyKindOf(selected.value.kind))
}
function setOutPoint() {
  const it = selItem.value
  if (!it) return
  const t = round2(Math.max(0, Number(props.currentTime) || 0))
  if (t <= itemStart.value) { toast('当前时间必须晚于开始时间', 'err'); return }
  writeSpan(it, itemStart.value, t)
  touch(dirtyKindOf(selected.value.kind))
}
function seekToStart() { seek(itemStart.value) }

// ── 事件类型切换 / 新建 ─────────────────────────────────────────
function defaultEvent(type, t) {
  const ev = { type, time: round2(t), end: round2(t + span0(type)) }
  if (type === 'camera') { ev.zoom = '中'; ev.origin = '50% 50%'; ev.anchor = 'liver'; ev.cut = false }
  else if (type === 'sticker') { ev.keyword = ''; ev.x = '50%'; ev.y = '50%' }
  else if (type === 'overlay') { ev.overlay_type = 'color'; ev.color = '#ff3232'; ev.opacity = 0.3 }
  else if (type === 'sfx') { ev.name = ''; ev.volume = 0.3 }
  else if (INTENSITY_TYPES.includes(type)) ev.intensity = type === 'shake' ? 10 : type === 'stutter' ? 12 : 30
  else if (COLOR_TYPES.includes(type)) ev.color = '#ffffff'
  return ev
}

function changeEventType(type) {
  const ev = selectedEv.value
  if (!ev || ev.type === type) return
  const t = Number(ev.time) || 0
  const span = evDur(ev)
  const keep = { sfx: ev.sfx, sfx_volume: ev.sfx_volume }
  for (const k of Object.keys(ev)) delete ev[k]
  Object.assign(ev, defaultEvent(type, t))
  ev.end = round2(t + span)
  if (keep.sfx !== undefined) ev.sfx = keep.sfx
  if (keep.sfx_volume !== undefined) ev.sfx_volume = keep.sfx_volume
  touch('segments')
}

function onStickerPick(patch) {
  const ev = selectedEv.value
  if (!ev || ev.type !== 'sticker') return
  if (patch.keyword !== undefined) ev.keyword = patch.keyword || ''
  if (patch.sticker_tag !== undefined) ev.sticker_tag = patch.sticker_tag || undefined
  if (patch.name !== undefined) {
    if (patch.name) ev.name = patch.name
    else delete ev.name
  }
  if (patch.source !== undefined) {
    if (patch.source) ev.source = patch.source
    else delete ev.source
  }
  touch('segments')
}

/**
 * 泳道标签上的 ＋ ：**类型由轨道决定**。
 * 字幕/屏蔽轨直接加对应块；运镜/贴纸/覆盖层/音效轨直接加该类型事件；
 * 视觉特效轨有 11 个细分类型 → 先弹菜单选一个再加（选完就定了，面板里只换同轨道的细分类型）。
 */
function onLaneAdd(laneType, evt) {
  if (laneType === 'subtitles') { addSubtitleAtCurrent(); return }
  if (laneType === 'segments') { addMuteAtCurrent(); return }
  if (laneType === 'fx') { openFxMenu(evt); return }
  addEventAt(laneType)
}

const fxMenu = ref(null) // { x, y } | null —— 视觉特效的细分类型菜单（fixed 定位，不受时间轴滚动影响）

function openFxMenu(evt) {
  const r = evt.currentTarget.getBoundingClientRect()
  fxMenu.value = { x: Math.round(r.left), y: Math.round(r.bottom + 4) }
  // 点别处关掉（capture 阶段；点是菜单自己的项就放行，否则 click 还没触发菜单就没了）
  document.addEventListener('pointerdown', closeFxMenu, true)
}
function closeFxMenu(e) {
  if (e && e.target && e.target.closest && e.target.closest('.fx-menu')) return
  if (!fxMenu.value) return
  fxMenu.value = null
  document.removeEventListener('pointerdown', closeFxMenu, true)
}

/** 在播放头处加一条**指定类型**的事件（类型只能由调用方给，面板里不提供跨轨道选择）。 */
function addEventAt(type) {
  closeFxMenu()
  if (!segments.value.length) {
    toast('还没有分段：事件要挂在分段里 —— 先跑一次「LLM 剪辑」', 'err')
    return
  }
  const t = round2(Math.max(0, Number(props.currentTime) || 0))
  const seg = segments.value.find((s) => t >= s.start && t <= s.end) || segments.value[0]
  if (!seg.events) seg.events = []
  // 手工放的事件 = 用户的东西：**新建时**就打 manual，后处理不再改写它
  // （不吸附字幕、不延长尾帧，也不参与数量抽稀）——否则"加完还在填 keyword"的这个中间状态
  // 会被自动后处理当成成品处理掉，用户看到的是"我刚加的贴纸没了"。
  const ev = defaultEvent(type, t)
  markManual(ev)
  seg.events.push(ev)
  const lane = isFxType(type) ? 'fx' : type
  selectBlock({ type: lane }, { kind: 'event', si: segments.value.indexOf(seg), ei: seg.events.length - 1 })
  touch('segments')
}

function addSubtitleAtCurrent() {
  const start = round2(Math.max(0, Number(props.currentTime) || 0))
  const sub = { start, end: round2(start + 3), text: '', html: '' }
  subtitles.value.push(sub)
  selected.value = { kind: 'sub', lane: 'subtitles', subIndex: subtitles.value.length - 1 }
  touch('subtitles')
}

function addMuteAtCurrent() {
  const start = round2(Math.max(0, Number(props.currentTime) || 0))
  const m = { start, end: round2(start + 3) }
  muteRanges.value.push(m)
  selected.value = { kind: 'mute', lane: 'segments', muteIndex: muteRanges.value.length - 1 }
  touch('mute')
}

function duplicateSelected() {
  const ev = selectedEv.value
  if (!ev) return
  const seg = segments.value[selected.value.si]
  if (!seg) return
  const copy = JSON.parse(JSON.stringify(ev))
  markManual(copy) // 复制是手工动作 → 同样是"用户的东西"，不被后处理抽稀/吸附
  const span = evDur(copy)
  setEvSpan(copy, (Number(copy.time) || 0) + 0.1, (Number(copy.time) || 0) + 0.1 + span)
  seg.events.push(copy)
  selected.value = { kind: 'event', lane: selected.value.lane, si: selected.value.si, ei: seg.events.length - 1 }
  touch('segments')
}

// ── 单级撤销删除 ────────────────────────────────────────────────
// 删除是"双击即生效 → 700ms 落盘 → 随后自动后处理"，误删点完就已经不可逆了（后处理还会按
// 新状态重算，删掉的事件不会自己回来）。所以保留**最近一次删除**给一枚「撤销删除」兜底。
// 只做一级，不是撤销栈：再删第二次，第一次就找不回来了（要整栈属于另一个方向）。
const UNDO_WINDOW_MS = 15000
const lastDeleted = ref(null)   // { label, item, restore }
let undoTimer = null

/** 记一笔删除；`restore(item)` 负责把深拷贝的 item 放回原来的位置。 */
function rememberDeleted(label, item, restore) {
  lastDeleted.value = { label, item: JSON.parse(JSON.stringify(item)), restore }
  addClient('info', `已删除${label}（15 秒内可撤销）`, 'timeline')
  if (undoTimer) clearTimeout(undoTimer)
  undoTimer = setTimeout(() => { lastDeleted.value = null; undoTimer = null }, UNDO_WINDOW_MS)
}

const undoTitle = computed(() => lastDeleted.value
  ? `把刚才删掉的${lastDeleted.value.label}原样放回（15 秒内有效）` : '')

function undoDelete() {
  const d = lastDeleted.value
  if (!d) return
  if (undoTimer) { clearTimeout(undoTimer); undoTimer = null }
  lastDeleted.value = null
  d.restore(d.item)
  addClient('info', `已撤销删除${d.label}`, 'timeline')
  toast(`已恢复${d.label}`)
}

/** 把 item 插回数组；原下标可能因重新载入而失效，夹一下。 */
const insertAt = (arr, i, item) => arr.splice(Math.min(Math.max(0, i), arr.length), 0, item)

// ── 删除 / 拆分 ─────────────────────────────────────────────────
// 三类块都不再弹确认框（双击即删才顺手），统一由「撤销删除」兜底；只有**分段**仍要确认 ——
// 它连带整个分段的事件一起没，破坏面比单个块大一个量级。
async function deleteSelected() {
  const kind = selected.value && selected.value.kind
  if (kind === 'sub') { deleteSelectedSubtitle(); return }
  if (kind === 'mute') { deleteSelectedMute(); return }
  if (kind === 'seg') {
    const si = selected.value.si
    const seg = segments.value[si]
    if (!seg) return
    const ok = await confirmDialog('将删除当前分段及其全部事件', { title: '删除当前分段？', danger: true })
    if (!ok) return
    segments.value.splice(si, 1)
    rememberDeleted('分段', seg, (item) => { insertAt(segments.value, si, item); touch('segments') })
    selected.value = null
    touch('segments')
    return
  }
  if (kind === 'event') {
    const si = selected.value.si
    const ei = selected.value.ei
    const seg = segments.value[si]
    if (!seg || !seg.events) return
    const ev = seg.events[ei]
    if (!ev) return
    seg.events.splice(ei, 1)
    // 认**分段对象本身**（不是下标）：后处理可能整体改写 segments，下标会漂；
    // 那个分段对象要是已经被换掉了，就只能放弃原位恢复。
    rememberDeleted('事件', ev, (item) => {
      if (!segments.value.includes(seg)) { toast('原分段已被改写，无法原位恢复事件', 'err'); return }
      if (!Array.isArray(seg.events)) seg.events = []
      insertAt(seg.events, ei, item)
      touch('segments')
    })
    selected.value = null
    touch('segments')
  }
}

function deleteSelectedSubtitle() {
  if (!selectedSub.value) return
  const idx = selected.value.subIndex
  if (!(idx >= 0)) return
  const sub = subtitles.value[idx]
  if (!sub) return
  subtitles.value.splice(idx, 1)
  rememberDeleted('字幕', sub, (item) => { insertAt(subtitles.value, idx, item); touch('subtitles') })
  selected.value = null
  touch('subtitles')
}

function deleteSelectedMute() {
  const idx = selected.value && selected.value.muteIndex
  if (!(idx >= 0)) return
  deleteMuteAt(idx)
}

function deleteMuteAt(idx) {
  if (idx < 0 || idx >= muteRanges.value.length) return
  const m = muteRanges.value[idx]
  muteRanges.value.splice(idx, 1)
  rememberDeleted('屏蔽区间', m, (item) => { insertAt(muteRanges.value, idx, item); touch('mute') })
  selected.value = null
  touch('mute')
}

function splitSelected() {
  const t = round2(Number(props.currentTime) || 0)
  if (selected.value.kind === 'sub') {
    const sub = selectedSub.value
    if (!sub) return
    if (t <= sub.start || t >= sub.end) { toast('当前时间不在该字幕范围内，无法拆分', 'err'); return }
    const i = subtitles.value.indexOf(sub)
    const tail = { start: t, end: sub.end, text: sub.text || '', html: sub.html || sub.text || '' }
    sub.end = t
    // ⚠ 必须插在原块**后面**，不能 push 到数组末尾：字幕轨道是有序的，
    // 追加到末尾会让列表（以及落盘的 clip.srt / data/subtitles.json）乱序 ——
    // 乱序会连带毁掉任何按顺序读它的东西（去静音按相邻间隔算静音曾因此判出
    // 13 段/121s 的假静音，预览按时间二分取当前字幕也会取错）。
    if (i >= 0) subtitles.value.splice(i + 1, 0, tail)
    else subtitles.value.push(tail)
    touch('subtitles')
    return
  }
  if (selected.value.kind === 'mute') {
    const m = selectedMute.value
    if (!m) return
    if (t <= m.start || t >= m.end) { toast('当前时间不在该屏蔽范围内，无法拆分', 'err'); return }
    const i = muteRanges.value.indexOf(m)
    const end = m.end
    m.end = t
    if (i >= 0) muteRanges.value.splice(i + 1, 0, { start: t, end })
    else muteRanges.value.push({ start: t, end })
    touch('mute')
  }
}

// ── 快捷操作「去静音」────────────────────────────────────────────
// 后端把检测到的长停顿**并入**「屏蔽」轨道（与当前屏蔽相加：重叠/相邻的合成一段），
// `silence_ranges` 记的是"自动加进去的那部分"，撤销就是从轨道里减掉它们 —— 手动屏蔽不受影响。
const silenceApplied = ref(false)
const silenceCount = ref(0)
const silenceBusy = ref(false)

function applySilenceState(cfg) {
  const recorded = Array.isArray(cfg?.silence_ranges) ? cfg.silence_ranges : []
  silenceApplied.value = recorded.length > 0
  silenceCount.value = recorded.length
}

async function toggleSilence() {
  if (silenceBusy.value) return
  silenceBusy.value = true
  const action = silenceApplied.value ? 'clear' : 'apply'
  try {
    const res = await runSilence(props.pid, props.dir, { action })
    await loadData()
    emit('preview', { mute_ranges: muteRanges.value })
    if (action === 'clear') {
      toast(`已撤销去静音，从屏蔽轨道移除 ${res?.removed ?? 0} 段自动静音`)
    } else if ((res?.added ?? 0) > 0) {
      toast(`去静音：新增 ${res.added} 段静音屏蔽，与已有屏蔽相加 → 轨道共 ${res?.total ?? 0} 段`)
    } else if ((res?.detected ?? 0) > 0) {
      toast(`检测到 ${res.detected} 段静音，屏蔽轨道里已经全部包含`)
    } else {
      toast('未检测到 2s 以上的长停顿')
    }
  } catch (e) {
    toast('去静音失败：' + e.message, 'err')
  } finally {
    silenceBusy.value = false
  }
}

// ── 标尺 / 播放头 / 快捷键 ──────────────────────────────────────
let rulerDragging = false
let userScrollTimer = null
const userScrolling = ref(false)

function markUserScroll() {
  userScrolling.value = true
  if (userScrollTimer) clearTimeout(userScrollTimer)
  userScrollTimer = setTimeout(() => {
    userScrolling.value = false
    userScrollTimer = null
  }, 2000)
}

function onTimelineKey(e) {
  if (e.code !== 'Space') return
  const el = rootRef.value
  if (!el) return
  const r = el.getBoundingClientRect()
  const x = e.clientX
  const y = e.clientY
  if (x < r.left || x > r.right || y < r.top || y > r.bottom) return
  const tag = (e.target?.tagName || '').toUpperCase()
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || e.target?.isContentEditable) return
  e.preventDefault()
  emit('toggle')
}

function seekFromRuler(e) {
  const track = rulerTrackRef.value
  if (!track || !scale.value) return
  const rect = track.getBoundingClientRect()
  const x = e.clientX - rect.left
  const t = Math.min(duration.value || 60, Math.max(0, x / scale.value))
  emit('seek', Math.round(t * 100) / 100)
}

function onRulerPointerDown(e) {
  if (e.button !== 0) return
  e.preventDefault()
  seekFromRuler(e)
  rulerDragging = true
  const onMove = (ev) => seekFromRuler(ev)
  const onUp = () => {
    rulerDragging = false
    window.removeEventListener('pointermove', onMove)
    window.removeEventListener('pointerup', onUp)
    window.removeEventListener('pointercancel', onUp)
  }
  window.addEventListener('pointermove', onMove)
  window.addEventListener('pointerup', onUp)
  window.addEventListener('pointercancel', onUp)
}

onUnmounted(() => {
  // 离开精修页/片段：这一轮编辑到此为止 —— 把欠的那次后处理交代给页面（页面自己决定去哪跑）
  commitEdit('离开页面')
  window.removeEventListener('keydown', onTimelineKey)
  document.removeEventListener('pointerdown', closeFxMenu, true)
  if (userScrollTimer) clearTimeout(userScrollTimer)
  if (undoTimer) { clearTimeout(undoTimer); undoTimer = null }
  // 还有没落盘的改动（<700ms 的窗口）：**先落盘再走**，别把最后几下编辑丢掉
  // ——页面那边的后处理是按磁盘上的 polish.toml 算的，丢一次就等于那几处白改
  if (saveTimer) { clearTimeout(saveTimer); flushSave() }
  if (previewTimer) clearTimeout(previewTimer)
  drag = null
})

onMounted(async () => {
  window.addEventListener('keydown', onTimelineKey)
  await loadData()
  loadPickers()
})

/**
 * 播放头跟随：**只在"正在播放"时跟，而且只在播放头要跑出可视区时才滚、只滚刚刚好**。
 *
 * 两轮反馈合起来的要求：①"只有把指针拖出显示进度外才移动视角，而不是乱飞"；
 * ②"拖动还是不主动变化，只能播放或者自己移动时间轴来移动"。
 * 所以：**不播放 → 一个像素都不动**（拖动标尺跳转、点块跳转、拖块改时间都不许动视角），
 * 外加拖动过程中（`drag` 块拖拽 / `rulerDragging` 标尺划动）与用户手动滚动后的 2 秒内也不动。
 * 别退回"每帧把播放头钉在 30% 处"（`scrollLeft = t*scale - 30%宽`）——预览是 rAF 驱动的，
 * 那等于一帧一滚，画面永远在飘。
 *
 * 坐标用**标尺轨道的实际左边**换算：时间轴最左边是 sticky 的标签列（96px），
 * 直接拿 `t * scale` 当可视坐标会差一个标签列的宽度。
 */
watch(() => props.currentTime, (t) => {
  if (!props.playing) return // 不播放不跟：拖动/跳转都不许自己动
  if (userScrolling.value || rulerDragging || drag) return
  const scroller = bodyRef.value
  const track = rulerTrackRef.value
  if (!scroller || !track || !scale.value) return
  const viewW = scroller.clientWidth
  if (!viewW) return
  const baseX = track.getBoundingClientRect().left - scroller.getBoundingClientRect().left
  const headX = baseX + (Number(t) || 0) * scale.value
  const margin = Math.min(140, Math.max(40, viewW * 0.12))
  if (headX < margin) {
    scroller.scrollLeft += headX - margin // 跑出左边（或贴边）→ 放到左余量线
  } else if (headX > viewW - margin) {
    scroller.scrollLeft += headX - (viewW - margin) // 跑出右边 → 放到右余量线
  }
  // 在可视区里 → 不动（这是重点：别每帧重定位）
})

defineExpose({ reload, save: flushSave, segments, subtitles, muteRanges })

watch(() => props.dir, async (_now, before) => {
  // 换片段＝这一轮编辑结束：先按**旧目录**把欠的那次后处理交代出去，再加载新片段的数据
  // （props.dir 此时已经是新值，所以必须显式传 before，否则会跑到刚切过去的片段上）
  commitEdit('切换片段', before)
  // ⚠ **必须先清空**：这是"换到了另一个片段"，旧数组属于上一个片段。若新片段读取失败而数组
  // 还留着旧内容，用户的一次编辑就会把**上一个片段的分段/字幕**写进新片段的目录。
  // 清空 + `loadError` 拦住写入，两道一起才安全。
  resetTimelineState()
  await loadData()
  loadPickers()
})

/**
 * 从磁盘读一份时间轴数据。**返回是否成功**，失败时不动现有内容、只置 `loadError`。
 *
 * ⚠️ 不能再用 `apiOr(…, null, …)` 兜底：它把"读不到"变成 null，于是编辑器显示空时间轴、
 * 脏标记还被清掉，接着用户拖一下分段轨就把 `segments: []` PUT 回磁盘 —— 分段就这么没了。
 * 失败必须与"真的为空"分开：**失败一律不写盘**（`flushSave` 开头拦）。
 */
async function loadData() {
  if (!props.dir) return false
  let cfg = null
  try {
    cfg = await fetchProject(props.pid, props.dir, true, '读取时间轴数据')
  } catch (e) {
    loadError.value = e.message || String(e)
    addClient('warn', '时间轴数据读取失败，已暂停保存：' + loadError.value, 'timeline')
    return false
  }
  loadError.value = ''
  segments.value = cfg?.segments || []
  llmStatus.value = cfg?._llm_status && Array.isArray(cfg._llm_status.segments) ? cfg._llm_status : null
  muteRanges.value = Array.isArray(cfg?.mute_ranges) ? cfg.mute_ranges.map(m => ({ start: Number(m.start) || 0, end: Number(m.end) || 0 })) : []
  applySilenceState(cfg)
  const last = cfg?.segments?.[cfg.segments.length - 1]
  duration.value = cfg?.duration || last?.end || 60
  loudness.value = cfg?.loudness_curve || []
  // 磁盘上的数据就是"已保存"的状态：清掉脏标记与状态文字
  dirty.segments = dirty.subtitles = dirty.mute = false
  saveAttempts = 0
  saveState.value = ''

  // 字幕：**404「暂无字幕文件」是正常空态**（没转写文件的片段就是没有字幕），只有其它错误
  // 才算读失败 —— 否则每个没有字幕的片段都会挂一条"读取失败，已暂停保存"。
  try {
    const subRes = await fetchSubtitles(props.pid, props.dir, '读取字幕')
    subtitles.value = subRes?.segments || []
  } catch (e) {
    if (e.status === 404) {
      subtitles.value = []
    } else {
      loadError.value = e.message || String(e)
      addClient('warn', '字幕读取失败，已暂停保存：' + loadError.value, 'timeline')
      return false
    }
  }
  return true
}
</script>

<style scoped>
.timeline { width:100%; }
/* 头部一行（标题/提示 + 去静音）；原来标题在卡片头、去静音单独占一行，共省 ~70px */
.tl-head { display:flex; align-items:center; justify-content:space-between; gap:10px;
  flex-wrap:wrap; margin-bottom:6px; }
.tl-head-title { display:flex; align-items:center; gap:8px; flex-wrap:wrap; min-width:0; }
.tl-head-title :deep(b) { font-size:13px; }
/* 没有分段时的提示行：整条时间轴照常渲染，这里只给一句提示（按钮已删，见模板注释） */
.tl-no-seg { display:flex; align-items:center; gap:10px; flex-wrap:wrap; margin:4px 0 2px; }
.timeline-body { flex:1; min-width:0; display:flex; overflow-x:auto; align-items:stretch; }
.timeline-labels { flex:none; position:sticky; left:0; z-index:6; display:flex; flex-direction:column; background:var(--card); box-shadow:2px 0 8px rgba(90,60,70,.10); }
.timeline-tracks { flex:none; min-width:0; }
/* 泳道 34 → 26px（7 条泳道省 ~56px）；块 24 → 20px */
.lane { height:26px; border-bottom:1px solid var(--line-soft); position:relative;
  display:flex; align-items:center; gap:10px; padding:0 12px; box-sizing:border-box; }
.lane-label { width:96px; height:26px; box-sizing:border-box; flex:none; display:flex; align-items:center; justify-content:space-between; padding:0 8px; font-size:11.5px; color:var(--ink-dim); border-bottom:1px solid var(--line-soft); }
.add-lane { background:none; border:none; color:var(--ink-faint); cursor:pointer; font-size:14px; line-height:1; padding:2px 4px; }
.add-lane:hover { color:var(--main-deep); }
.lane-adds { display:flex; align-items:center; gap:2px; }
.mute-add { color:#ef4444; }
.mute-add:hover { color:#b91c1c; }
.lane-track { position:relative; height:26px; }
/* 响度包络线：**必须压在不透明的字幕块之上**（块 z-index:2、选中块 3），否则说话时段
   （也就是最该看电平的时候）整段被块盖住 —— 这是"波形看不见"的根因。
   仍在播放头（z-index:5）之下。`pointer-events:none` 保持不变：不挡拖块、不挡选中。 */
.wave-layer { position:absolute; top:0; left:0; pointer-events:none; opacity:1; z-index:4; }
.playhead { position:absolute; top:0; bottom:0; width:2px; background:var(--main); z-index:5; pointer-events:none; }
.block { position:absolute; top:3px; height:20px; border-radius:4px; color:#fff; font-size:10px; line-height:20px; padding:0 6px; white-space:nowrap; overflow:hidden; cursor:grab; user-select:none; z-index:2; }
.block.selected { outline:2px solid #fff; z-index:3; }
 .resize { position:absolute; right:0; top:0; width:8px; height:100%; cursor:ew-resize; background:rgba(255,255,255,0.35); border-radius:0 4px 4px 0; }
.resize-l { left:0; right:auto; border-radius:4px 0 0 4px; }
.ruler-label { height:20px; font-size:10px; }
.ruler-lane { height:20px; border-bottom:none; }
.ruler-track { height:20px; cursor:col-resize; }
.ruler-tick { position:absolute; top:0; bottom:0; width:1px; background:rgba(0,0,0,0.14); }
.ruler-time { position:absolute; top:1px; left:4px; font-size:9px; color:var(--ink-dim); white-space:nowrap; pointer-events:none; }
.ruler-playhead { position:absolute; top:0; bottom:0; width:2px; background:var(--main); z-index:5; pointer-events:none; }
.block.type-subtitles { background:rgba(59,130,246,0.35) !important; color:#0f172a; font-weight:700; opacity:1; text-shadow:0 0 2px #fff, 0 0 2px #fff; }
.block.type-mutes { background:rgba(239,68,68,0.42) !important; color:#7f1d1d; font-weight:700; }
/* 上次 LLM 生成没产出事件的分段：琥珀色 + 虚框（颜色是用户要求的"标出来"，虚框照顾色觉） */
.block.seg-gen-failed { background:#f59e0b !important; color:#4a2c00; font-weight:700; outline:1.5px dashed #b45309; }
/* 分段属性里的"上次生成"状态块 */
.seg-gen { border-top:1px solid var(--line-soft); padding-top:6px; }
.chip.warn { background:#fde68a; color:#7c4a03; }
/* 属性面板已 Teleport 到页面（视频左侧），这些样式跟着元素走（scoped 属性保留） */
.props-panel { width:232px; flex:none; background:var(--card); border:1px solid var(--line); border-radius:12px; padding:10px; overflow:auto; }
.props-empty { border:1.5px dashed var(--line); border-radius:10px; padding:12px 10px; }
/* 覆盖层未设色档时的退化说明（终色此刻实为 transparent，取色器只能显示黑，必须解释一句） */
.mini-hint { margin:6px 0 0; padding:5px 7px; background:var(--bg-soft); border-radius:6px;
  font-size:11px; line-height:1.5; color:var(--ink-faint); }
.props-panel label { margin-top:6px; }
.props-panel .row { margin-top:4px; }
.props-panel input[type="color"] { height: 26px; padding: 0 2px; }
.props-panel input, .props-panel select, .props-panel textarea { font-size: 12px; }
/* 只读类型展示：类型由轨道决定，这里不给下拉 */
.type-ro { display:flex; align-items:baseline; gap:6px; margin-top:6px; }
.type-ro b { font-size: 12.5px; }
/* 手工时间状态行（贴纸/运镜）：chip + 说明 +「跟随字幕」 */
.manual-row { display:flex; align-items:center; gap:6px; flex-wrap:wrap; }
/* 运镜「手持摇摆」开关行：紧跟景别下拉，禁用态表示当前景别不满足门槛 */
.ck-row { display:flex; align-items:center; gap:5px; margin-top:8px; font-size:12px;
  color:var(--ink); cursor:pointer; }
.ck-row input[type="checkbox"]:disabled { cursor:not-allowed; }
.ck-row:has(input:disabled) { color:var(--ink-faint); cursor:not-allowed; }
/* 视觉特效轨道 ＋ 的细分类型菜单（fixed：不随时间轴滚动、不被卡片裁切） */
.fx-menu { position:fixed; z-index:40; min-width:132px; padding:4px; background:var(--card);
  border:1px solid var(--line); border-radius:10px; box-shadow:0 8px 22px rgba(90,60,70,.18); }
.fx-menu-t { padding:2px 8px 4px; }
.fx-menu-item { display:flex; justify-content:space-between; gap:8px; width:100%; padding:3px 8px;
  border:none; border-radius:6px; background:none; color:var(--ink); font-size:12px; cursor:pointer; text-align:left; }
.fx-menu-item:hover { background:var(--bg-soft); color:var(--main-deep); }
</style>
