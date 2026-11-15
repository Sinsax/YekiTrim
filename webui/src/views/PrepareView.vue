<template>
  <div>
    <!-- 项目不存在（服务端 404）：空态 + 回首页 / 重新导入。
         ⚠️ 其它读取错误**不许**伪装成空态（见 views/AGENTS.md「读失败」），走下面的错误横幅 + 重试。 -->
    <AppEmptyState
      v-if="notFound"
      icon="film"
      title="项目不存在或已移除"
      description="可能已被移除，或链接里的项目 id 不对。重新导入同一个目录即可再绑回来。"
      primary="回首页"
      primary-to="/workbench"
      secondary="重新导入"
      @secondary="goImport"
    />

    <template v-else>
      <div v-if="loadError" class="error-banner">
        <Icon name="alert" :size="15" />
        <span class="flex-1">{{ loadError }}</span>
        <button class="mini-btn flex-none" title="重新读取这个项目的状态与素材" @click="load">重试</button>
      </div>

      <!-- LLM 未配置：**不拦路**（检测照跑，退化按弹幕/字幕热度 + 一个整段），只是提前说清结果。 -->
      <div v-if="llmMissing" class="llm-banner" title="在「设置 → AI 模型」里配好接口与密钥后，检测会按话题切成多个亮点">
        <Icon name="alert" :size="14" />
        <span class="flex-1" title="没配 AI 时只按弹幕/字幕热度查找事件，可能只得到一个整段；配置后可提高分段质量">未配置 AI：只按弹幕/字幕热度查找事件</span>
        <router-link class="mini-btn flex-none" to="/settings/llm">去配置 AI</router-link>
      </div>

      <!-- 一键处理完成：亮点/片段数 + 下一步。 -->
      <NextStepBar
        v-if="completed"
        :title="completedText"
        cta="去挑片段"
        :to="slicesPath"
      />
      <div v-if="completedHint" class="hint completed-hint" :title="completedHintTitle">{{ completedHint }}</div>

        <div class="card flow-card mb-12">
          <!-- 一行 = 复选节点 + 主按钮；运行中第二行 = 进度条 + jobLine + 停止/任务。
               旧版「标题 / 按钮 / 进度行 / 任务盒」四层堆叠说的都是同一件事 —— 用户口径：太长，压紧凑。
               ⚠️ 只读 `hasTranscript/hasEvents/hasClips` 与任务态，不改主按钮状态机。 -->
          <div class="prep-steps" data-cap="prepare.stage-select">
            <template v-for="(st, i) in prepSteps" :key="st.key">
              <div class="pstep">
                <!-- 阶段可选 v2（用户口径：文字变亮变暗就是设置）：亮 = 纳入一键处理、暗 = 排除；
                     点亮后面的自动带上前面，熄灭前面的连带熄灭后面（累计）。剪出片段默认暗（费时费性能）。 -->
                <button type="button" class="pstep-tag"
                        :class="{ on: stageSel[st.key], dim: !stageSel[st.key], done: st.state === 'done', active: st.state === 'active' }"
                        :disabled="busy"
                        :title="stageSel[st.key] ? '已纳入一键处理，点击排除' : '点击纳入一键处理'"
                        @click="toggleStage(st.key)">
                  {{ st.label }}<span v-if="st.state === 'done'" class="pstep-done"> ✓</span>
                </button>
              </div>
              <span v-if="i < prepSteps.length - 1" class="pstep-line" :class="{ on: st.state === 'done' }" aria-hidden="true"></span>
            </template>
            <!-- ⚠️ 刚跑完这一次时主按钮不渲染：上面的 NextStepBar 已经是「去挑片段」，
                 两颗同目的地的主按钮并排 = 一页两个主行动。重新跑走「处理选项 → 重新查找事件」。
                 `data-cap` 仍只有源码里这一处（check-simplify 数的是锚点，不是渲染结果）。 -->
            <button
              v-if="!completed && !jobRunning"
              class="btn btn-primary run-btn"
              data-cap="prepare.run-all"
              :disabled="primaryDisabled"
              :title="primaryTitle"
              @click="onPrimary"
            >{{ primaryLabel }}</button>
            <span v-else-if="completed" class="muted small flex-none">已完成 ✓</span>
          </div>
          <!-- 运行中第二行：进度条 + jobLine + 停止/任务 —— 全卡唯一的运行态渲染处 -->
          <div v-if="jobRunning" class="prep-run">
            <div class="progress flex-1"><div :style="{ width: jobPct + '%' }"></div></div>
            <span class="muted small prep-run-label">{{ jobLine(job) || '运行中' }}</span>
            <button class="mini-btn danger flex-none" :disabled="canceling" title="停止当前任务" @click="cancelJob">
              <Icon name="x" :size="10" /> 停止
            </button>
            <button class="mini-btn flex-none" title="打开任务抽屉" @click="openJobs()">任务</button>
          </div>
          <!-- 失败/取消必须可见（成功由步进 ✓ 与「去挑片段」表达，不重复渲染） -->
          <div v-if="job.id && !jobRunning && (job.status === 'failed' || job.status === 'cancelled')" class="error-banner">
            <Icon name="alert" :size="14" />
            <span class="flex-1">{{ statusText(job.status) }}：{{ job.message || job.error || '无详情' }}</span>
            <button class="mini-btn flex-none" @click="openJobs()">任务</button>
          </div>
        </div>
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

        <!-- 字幕轨道：默认展开（用户口径）；收起后只留标题行，要拖块/改字再展开。
             展开状态首帧同步读 localStorage（`useDisclosure`）—— 别改成 v-show，轨道挂载时要量宽度。 -->
        <div class="card mt-12">
          <div class="row spread mb-6">
            <b class="t m-0">字幕 {{ segments.length }} 条</b>
            <span class="row" style="gap:8px">
              <span class="muted small" :class="{ err: saveState === 'error' }">{{ saveHint }}</span>
              <button
                class="mini-btn flex-none"
                data-cap="prepare.subtitle-track"
                :title="subsOpen ? '点块在上方属性条改文本/入点/出点，改动自动保存；收起不会丢内容' : '展开后能拖块改起止、改文本（改动自动保存）'"
                @click="toggleSubs"
              >{{ subsOpen ? '收起编辑' : '展开编辑' }}</button>
            </span>
          </div>
          <SubtitleTrack
            v-if="subsOpen"
            :disabled="writeBlocked"
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

      <!-- 右：识别（一块卡，高级项默认折叠）。主按钮紧跟状态 chips，其余收进「处理选项」。 -->
      <div class="side-col">
        <!-- 运行任务日志（用户口径：右侧搬日志方便盯进度）：详情回填最近 50 行 + SSE Log 事件实时追加 -->
        <div class="card joblog-card">
          <div class="row spread mb-8">
            <b class="t m-0">任务日志</b>
            <span class="muted small" v-if="jobLogs.length">{{ jobLogs.length }} 行</span>
          </div>
          <div class="joblog" ref="joblogEl">
            <div v-if="!jobLogs.length" class="empty small">暂无任务日志 —— 起一个任务后这里会实时滚动</div>
            <div v-for="(l, i) in jobLogs" :key="i" class="joblog-line" :class="l.level">
              <span class="mono">{{ l.message }}</span>
            </div>
          </div>
        </div>
        <div class="card">
          <!-- 处理选项：ASR 模式 / 主播名 / 分步处理 / 弹幕与字幕清单 / 重新处理。默认收起。 -->
          <details class="adv-blk materials-blk" :open="materialsOpen" @toggle="syncMaterials">
            <summary data-cap="prepare.materials" title="ASR 模式、主播名、分步处理、弹幕/字幕清单与重新处理">处理选项</summary>

          <!-- ASR 模式：写进本工程 `yekitrim.toml` 的 `[asr] mode`（merge 时覆盖全局配置，
               接口走 `PUT /api/project-data/config {patch}` —— 保留注释与其他 section）。
               **两种模式各只能出一次产物**（用户要求"输出一次绑定一次、只能两次"）：
               跑过的模式槽就占了（`assets.slots`，与后端 `assets::slot_bound` 同口径），
               那一格的「跑 ASR」置灰；两格都占满时模式下拉也失去意义（一并置灰）。 -->
          <div class="asr-row" :title="asrTip + '。' + asrHint">
            <span class="asr-name">ASR 模式</span>
            <select
              v-model="asrMode"
              class="asr-select"
              data-cap="prepare.asr-mode"
              :disabled="!canPickAsr || busy"
              @change="saveAsrMode"
            >
              <option value="hybrid">hybrid · 更准（双模型）</option>
              <option value="single">single · 更快（单模型）</option>
            </select>
            <!-- 只识别 = 只跑 ASR 转写（**不接检测**）：换模式再出一份字幕的入口。
                 两种模式各一次，跑过的模式槽就占了（`canRunAsr`）。 -->
            <button
              class="mini-btn flex-none"
              data-cap="prepare.run-step"
              :disabled="!canRunAsr || busy"
              :title="asrRunTitle"
              @click="runAsr"
            >{{ canRunAsr ? '只识别' : '已跑过' }}</button>
          </div>
          <!-- 主播名：全局 `[detection] vtuber_name` 只是**默认值**，每个工程可以自己写一份
               （写进 `yekitrim.toml` 的 `[detection] vtuber_name`，merge 时覆盖全局）。
               ⚠️ `merge` 对**空串不覆盖** ⇒ 清空就是"恢复跟随全局"，不需要额外的开关。 -->
          <div class="asr-row">
            <span class="asr-name">主播名</span>
            <input
              v-model="vtuberName"
              class="fld-input flex-1"
              placeholder="留空 = 跟随全局"
              :title="vtuberHint + ' · 生效：' + (effectiveVtuber || '未设')"
              :disabled="busy"
              @change="saveVtuberName"
            >
          </div>

          <!-- 分步：事件检测（只跑检测、不剪片段）。「只识别」在上面 ASR 那一行（只转写）。 -->
          <button
            class="mini-btn block-btn mt-8"
            :disabled="!hasTranscript || !hasVideo || busy"
            title="只跑事件检测，不剪片段"
            @click="runDetectOnly"
          >事件检测</button>

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
              :title="dmHint"
              empty-text="还没选弹幕"
              @pick="openPicker('danmaku')"
              @activate="activate('danmaku', $event)"
              @remove="removeItem('danmaku', $event)"
            />
            <MaterialList
              class="mt-10"
              :items="txItems"
              kind="transcript"
              :busy="busy"
              :title="assetsHint"
              :empty-text="txEmptyText"
              @pick="openPicker('transcript')"
              @activate="activate('transcript', $event)"
              @remove="removeItem('transcript', $event)"
            />
          </div>

            <!-- 重新查找事件：只重跑事件检测（整份覆盖 events.json），不重切片段。
                 已剪片段不受影响；要重切去「挑片段」页点「一键剪出全部」。 -->
            <button
              v-if="hasEvents || hasClips"
              class="btn btn-danger block-btn mt-6"
              data-cap="prepare.redetect"
              :disabled="busy || !hasVideo"
              :title="redetectTitle"
              @click="onRedetect"
            >重新查找事件</button>
          </details>
        </div>
        <div class="card">
          <!-- 手动切片：整块原样保留（判据 ⑳/㉔/㉕/㉙ 以这里的结构与缩进为锚，别重排）。 -->
          <details class="adv-blk manual-blk" :open="manualOpen" @toggle="syncManual">
            <summary data-cap="prepare.manual-slice" title="自己标入点/出点：多组 = 分段剪、合并输出">手动切片</summary>
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
          </details>
        </div>
      </div>
    </div>
    </template>

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
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Icon from '../components/Icon.vue'
import AppModal from '../components/AppModal.vue'
import AppEmptyState from '../components/AppEmptyState.vue'
import NextStepBar from '../components/NextStepBar.vue'
import ServerDirPicker from '../components/ServerDirPicker.vue'
import SubtitleTrack from '../components/prepare/SubtitleTrack.vue'
import MaterialList from '../components/prepare/MaterialList.vue'
import { projectData } from '../api/projectData'
import { healthPayload, refreshHealth } from '../stores/health'
import { humanBytes } from '../stores/project'
import { isAdmin } from '../stores/auth'
import { watchJob, jobsApi, kindText, statusText, busyOn, outKey, jobLine, QUEUED_HINT } from '../api/jobs'
import { useJobs, onJobDone } from '../composables/useJobs'
import { useDisclosure } from '../composables/useDisclosure'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import { openJobs } from '../stores/ui'
import { segColor, EVENT_TYPES, TYPE_ZH, mediaUrl, redetectWarning, isEditableKeyEvent } from '../utils'

const route = useRoute()
const router = useRouter()
const pid = computed(() => route.params.pid || '')
/**
 * 本实例所属的项目 id（**字符串快照**）。
 *
 * ⚠️ 写盘要用它、不要用 `pid.value`：`App.vue` 用 `:key="pid"` 让视图换项目时重挂，而
 *    `onUnmounted` 里路由**已经是新项目**了 —— 那时读 `pid.value` 会把 A 的字幕写进 B。
 *    同一个实例终身只服务一个 pid，所以快照与 `pid` 恒等；读路径继续用 `pid`。
 */
const ownPid = String(route.params.pid || '')

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
/** 首次状态还没读回来（网络盘上可能要几秒）：这期间别把主按钮写成「还没有输入视频」。 */
const statusLoading = ref(true)
/** 服务端 404（项目不存在 / 无权访问）：走空态，**不当成普通读取失败**（见 views/AGENTS.md「读失败」）。 */
const notFound = ref(false)
/**
 * 字幕**写闸**：读失败 / 项目不存在时基线不可信，一个字节都不许写。
 *
 * 与精修时间轴的 `flushSave`（`TimelineEditor.vue`）同一条口径：读失败时内存里可能是空数组，
 * 任何一次落盘都会把磁盘上真实的转写稿整份覆盖成"当前看到的这几条"（2026-09-16 那类事故）。
 * 404 不算：服务端语义就是"还没转写"，那是正常空态，新建字幕要能存。
 */
const writeBlocked = computed(() => notFound.value || !!loadError.value)
/** `/api/health.llm === false`：只多一句提示，**不挡任何操作**（检测会退化成热度 + 一个整段）。 */
const llmMissing = ref(false)
/** 三块可折叠区的展开状态：首帧同步读 localStorage（`useDisclosure`），避免先展开再收起闪一下。
 *  ⚠️ 字幕轨与手动切片**默认展开**（用户口径）；键带 `.v2`，避免旧版默认收起时写下的 '0' 继续压着。 */
const { open: materialsOpen, set: setMaterialsOpen } = useDisclosure('prepare.materials', false)
const { open: manualOpen, set: setManualOpen } = useDisclosure('prepare.manualSlice.v2', true)
const { open: subsOpen, set: setSubsOpen } = useDisclosure('prepare.subtitles.v2', true)
/** 阶段可选（spec A，2026-11-15 用户口径）：步进区三枚复选框，累计语义 ——
 *  默认只勾前三阶段（识别声音+查找事件），剪出片段不勾（剪辑太费时间和性能）。
 *  勾后面的自动带上前面；取消前面的连带取消后面。持久化 localStorage。 */
const STAGE_SEL_KEY = 'yekitrim.prepare.stages'
/** 一键处理成功后的结果（亮点数 / 片段数），给顶部「下一步」条用；读不到就保持 null（不报假数）。 */
const completed = ref(null)
/** 「停止」防连点。 */
const canceling = ref(false)
/** 当前阶段（服务端 `stage_started` 给的英文名）：单一 process 任务靠它把 chips 打到对应那一格。 */
const stageNow = ref('')

function normalizeStageSel(out) {
  if (out.cut) { out.detect = true; out.transcribe = true }
  if (out.detect) out.transcribe = true
  if (!out.transcribe) { out.detect = false; out.cut = false }
  if (!out.detect) out.cut = false
  return out
}
function readStageSel() {
  const def = { transcribe: true, detect: true, cut: false }
  try {
    const raw = JSON.parse(localStorage.getItem(STAGE_SEL_KEY) || 'null')
    if (!raw || typeof raw !== 'object') return def
    return normalizeStageSel({
      transcribe: raw.transcribe === true,
      detect: raw.detect === true,
      cut: raw.cut === true
    })
  } catch {
    return def
  }
}
const stageSel = reactive(readStageSel())
/** 点击切换（变亮变暗就是设置）：点亮后面自动带上前面，熄灭前面的连带熄灭后面（累计）。 */
function toggleStage(key) {
  if (busy.value) return
  const order = ['transcribe', 'detect', 'cut']
  const at = order.indexOf(key)
  const on = !stageSel[key]
  for (let i = 0; i < order.length; i++) stageSel[order[i]] = on ? i <= at : i < at
  try {
    localStorage.setItem(STAGE_SEL_KEY, JSON.stringify({
      transcribe: stageSel.transcribe, detect: stageSel.detect, cut: stageSel.cut
    }))
  } catch {}
}
/** 旧口径兼容：autoCut = 「剪出片段」勾选 —— 主按钮两路语义不变（开走一键全流程、关走两段式）。 */
const autoCut = computed(() => stageSel.cut)

/** `<details>` 的开关状态与 localStorage 双向同步（原生 toggle 事件）。 */
function syncMaterials(e) { if (e?.target) setMaterialsOpen(e.target.open) }
function syncManual(e) { if (e?.target) setManualOpen(e.target.open) }
function toggleSubs() { setSubsOpen(!subsOpen.value) }

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
  // 读失败时写着"已保存"是骗人的：优先级与 TimelineEditor 的 `saveText` 一致（读失败 > 保存中）。
  if (writeBlocked.value) return '读取失败，已暂停保存'
  if (saveState.value === 'saving') return '保存中…'
  if (saveState.value === 'error') return '保存失败：' + saveError.value
  if (saveState.value === 'saved') return '已保存 ' + saveAt.value
  return subsOpen.value ? '点块编辑' : '展开编辑'
})

const jobRunning = computed(() => job.status === 'running' || job.status === 'queued')
const jobPct = computed(() => (job.total > 0 ? Math.round((job.current * 100) / job.total) : 0))
const runningStage = computed(() => (jobRunning.value ? jobStage.value : ''))
const jobStage = computed(() => {
  const k = job.kind || ''
  if (k === 'stage_detect') return 'detect'
  if (k === 'stage_transcribe') return 'transcribe'
  if (k === 'stage_cut') return 'cut'
  // 一键处理（process）= extract + transcribe + detect + cut 一个任务：阶段只能从 `stage_started` 认
  if (k === 'stage_process' || k === 'pipeline') {
    if (stageNow.value === 'EventDetector') return 'detect'
    if (stageNow.value === 'Transcriber') return 'transcribe'
    if (stageNow.value === 'Cutter') return 'cut'
  }
  return ''
})
// 项目级作用域（`p{pid}:out`）：转写/检测/订正/剪切都写 output/*，互相排队。
// 别的项目在跑、或某个片段在精修，都不会让这个按钮变灰（并行能力）。
// ⚠️ 快照过期（`stale`）时**降级放行**：一次读取故障不该把按钮永久锁死（见 components/AGENTS.md）。
const { jobs: allJobs, reload: reloadJobs, stale: jobsStale } = useJobs()
const scopeBusyJob = computed(() => (jobsStale.value ? null : busyOn(allJobs.value, [outKey(pid.value)])))
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
  ? `当前 transcript.json（${segments.value.length} 段）`
  : '还没有字幕'))
/** 已有检测结果（`status.files` 里带「事件」那一项）→ 主按钮去挑片段 / 重新处理要二次确认 */
const hasEvents = computed(() => hasFile('事件'))
/** 已有剪切产物（`clips/clips_order.txt`）→ 重跑全流程前也要确认（片段不会被删，但会被重切） */
const hasClips = computed(() => hasFile('片段顺序'))
/** 有没有输入视频：没有时主按钮禁用（「还没有输入视频」）。 */
const hasVideo = computed(() => !!status.value?.input_file)
/** 切片页路径（主按钮与「下一步」条共用）。 */
const slicesPath = computed(() => `/p/${encodeURIComponent(pid.value)}/slices`)
/** 转写成功后自动接着跑检测（两段之间不留手动步骤） */
const chainDetect = ref(false)

// ---- 主按钮状态机（没有事件 → 一键处理 / 去挑片段；运行中禁用）----
/**
 * 项目状态**读失败**（非 404）。
 *
 * ⚠️ 此时 `status` 还是 null ⇒ `hasVideo` 一定是 false —— 但那是「不知道」而不是「没有视频」。
 * 原来主按钮在这种情况下显示「还没有输入视频」并置灰，用户看到的是"我的视频呢"，
 * 而真正的下一步是**重试**（错误横幅就在顶上）。判据必须带 `!status.value`：
 * 已经有旧快照时静默刷新失败只加横幅，不能把主行动锁成"缺输入"。
 */
const statusUnavailable = computed(() => !statusLoading.value && !status.value && !!loadError.value)
const primaryDisabled = computed(() => {
  if (jobRunning.value) return true
  if (statusLoading.value) return true // 状态还没读回来：先别下结论
  if (statusUnavailable.value) return true // 读失败 ≠ 空项目：置灰并让人去重试
  if (!hasVideo.value) return true // 没有输入视频：什么都跑不了
  if (hasEvents.value) return false // 已识别：随时能去挑片段
  if (busy.value) return true
  // 一个字幕都没有、而当前 ASR 模式的槽已经占了：这次转写后端也会 409（先选字幕或换模式）
  if (!hasTranscript.value && !canRunAsr.value) return true
  // 阶段可选：勾选组合无可跑项 → 没有主行动（识别已完成且不跑查找事件等）
  if (!stageSel.transcribe && !stageSel.detect && !stageSel.cut) return true
  if (hasTranscript.value && !stageSel.detect) return true
  return false
})
const primaryLabel = computed(() => {
  if (runningStage.value === 'transcribe') return `转写中 ${jobPct.value}%`
  if (runningStage.value === 'detect') return `检测中 ${jobPct.value}%`
  if (runningStage.value === 'cut') return `剪切中 ${jobPct.value}%`
  if (jobRunning.value) return '处理中…'
  if (statusLoading.value) return '读取中…'
  if (statusUnavailable.value) return '状态不可用'
  if (!hasVideo.value) return '还没有输入视频'
  if (hasEvents.value) return '去挑片段 →'
  if (!hasTranscript.value && !canRunAsr.value) return '先选一份字幕'
  if (!stageSel.transcribe && !stageSel.detect && !stageSel.cut) return '未勾选阶段'
  if (hasTranscript.value && !stageSel.detect) return '识别已完成'
  return autoCut.value ? '开始处理' : '开始识别'
})
const primaryTitle = computed(() => {
  if (statusLoading.value) return '正在读取这个项目的状态…'
  if (statusUnavailable.value) return '项目状态没读出来 —— 点上面的「重试」重新读取（不代表没有视频）'
  if (!hasVideo.value) return '还没有输入视频：先去项目设置里填 input_file（或跑一次音频提取）'
  if (hasEvents.value) return '已经找到亮点：去切片页挑要发的片段'
  if (!hasTranscript.value && !canRunAsr.value) {
    return `「${asrMode.value}」模式的 ASR 已经跑过 —— 从处理选项的字幕列表里选一份，或换个模式再跑`
  }
  const modelHint = hasTranscript.value ? '' : '；首次会下载模型（hybrid 879 MB / single 165 MB）'
  if (autoCut.value) return `一次点击跑完：转写 → 查找事件 → 剪出片段（已完成的阶段会跳过）${modelHint}`
  return `一次点击跑完：转写 → 查找事件（不剪片段，片段到切片页自己挑）${modelHint}`
})
/** 一键处理成功后的「下一步」文案：片段数只有在这次真的切了片段时才报（分步跑只报亮点数）。 */
const completedText = computed(() => {
  const c = completed.value
  if (!c) return ''
  if (!c.cut) return `已找到 ${c.events} 个亮点`
  return `已找到 ${c.events} 个亮点 · 已剪出 ${c.clips} 个片段`
})
/** 只找到一条事件时补一句 —— 没配 AI 的那次通常就是「完整录像」一整段。 */
const completedHint = computed(() => {
  const c = completed.value
  if (!c || c.events !== 1) return ''
  return /^完整录像/.test(c.firstLabel)
    ? '只得到一个整段，配置 AI 后可细分'
    : '只找到一个亮点，可重新处理再试'
})
/** 步骤条的三段（只用于展示；真正的主行动仍由 `primaryLabel` 状态机决定）。 */
const prepSteps = computed(() => {
  const tr = hasTranscript.value
  const ev = hasEvents.value
  const cl = hasClips.value
  const run = runningStage.value
  const pct = jobPct.value
  return [
    {
      key: 'transcribe', label: '识别声音',
      state: run === 'transcribe' ? 'active' : tr ? 'done' : 'todo',
      sub: run === 'transcribe' ? `进行中 ${pct}%` : tr ? '已完成' : '待开始'
    },
    {
      key: 'detect', label: '查找事件',
      state: run === 'detect' ? 'active' : ev ? 'done' : tr ? 'active' : 'todo',
      sub: run === 'detect' ? `进行中 ${pct}%` : ev ? '已完成' : '待开始'
    },
    {
      key: 'cut', label: '剪出片段',
      state: run === 'cut' ? 'active' : cl ? 'done' : 'todo',
      sub: run === 'cut' ? `进行中 ${pct}%` : cl ? '已剪出' : '待开始'
    }
  ]
})

const completedHintTitle = computed(() => (completed.value && /^完整录像/.test(completed.value.firstLabel)
  ? '未配置 AI 模型（或模型没识别出话题）时，检测只产出一个「完整录像」事件。'
  : '检测只产出一条事件时的提示。'))
/** 「重新查找事件」：只重跑检测。已有片段时不去动 events.json，直接把人送到挑片段页补切。 */
const redetectTitle = '只重跑事件检测：整份覆盖 events.json，不重切片段；已剪片段不受影响'


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
  if (!used.length) return '还没有 ASR 产物：用处理选项里的「只识别」生成，或自己选一份字幕文件'
  return `ASR 产物已出（${used.join(' / ')}）—— 每种模式各一次，要再跑就换模式`
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
  if (key === 'detect') return hasFile('事件') ? '✓' : '待开始'
  if (key === 'cut') return hasFile('片段顺序') ? '✓' : '待开始'
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
  if (!status.value) statusLoading.value = true
  try {
    const st = await projectData.status(pid.value, '读取项目状态')
    notFound.value = false
    statusLoading.value = false
    status.value = st
    inputName.value = st.input_file || ''
    src.value = mediaUrl(pid.value, st.input_file)
  } catch (e) {
    // ⚠️ **404 = 项目不存在/无权访问**：走整页空态（回首页 / 重新导入），别再接着读配置/字幕 ——
    // 那些请求也只会各 404 一次。其余错误仍走错误横幅 + 重试（读失败不许伪装成空态）。
    if (e?.status === 404) {
      notFound.value = true
      status.value = null
      segments.value = []
      src.value = ''
      inputName.value = ''
      statusLoading.value = false
      return
    }
    statusLoading.value = false
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
    if (e?.status !== 404) loadError.value = loadError.value || ('字幕读取失败：' + (e?.message || e))
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
  // 写闸：读失败时连模型都不接（基线不可信，接下去的任何落盘都是覆盖真稿）。
  if (writeBlocked.value) return
  segments.value = next
  if (opts.transient) return
  clearTimeout(saveTimer)
  saveTimer = setTimeout(saveSubs, opts.committed ? 0 : 500)
}

/** 正在飞的那次保存。**串行化**用：`saveState='saving'` 只是显示，不会阻止第二次 PUT ——
 *  慢盘上"先发的旧请求后到"会把新稿盖回旧内容。 */
let saveInflight = null
/** 保存期间又来了新改动 ⇒ 这一次落地后用最新值再存一次（保证"最后一次值"最后落盘）。 */
let saveAgain = false

/** 真正落盘。参数是**快照**：卸载/切项目时必须由调用方显式给 pid 与 segments。 */
async function persistSubs(pidArg, segmentsArg) {
  if (!pidArg) return
  saveState.value = 'saving'
  try {
    await projectData.saveTranscript(pidArg, segmentsArg)
    saveState.value = 'saved'
    saveAt.value = new Date().toLocaleTimeString('zh-CN', { hour12: false })
    saveError.value = ''
  } catch (e) {
    saveState.value = 'error'
    saveError.value = e.message || '未知错误'
    toast('字幕保存失败：' + (e.message || ''), 'err')
  }
}

async function saveSubs() {
  // ⚠️ 读失败 / 项目不存在 ⇒ 禁止写盘（见 `writeBlocked`）。404 是空态，不拦。
  if (writeBlocked.value) return
  // 串行化：在飞时只记一笔"还要再存"，不并发发第二个 PUT。
  if (saveInflight) { saveAgain = true; return saveInflight }
  saveInflight = persistSubs(ownPid, segments.value).finally(() => { saveInflight = null })
  try {
    await saveInflight
  } finally {
    if (saveAgain) { saveAgain = false; saveSubs() }
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

/** 「只识别」：**只转写**（不接检测）—— 换模式再出一份字幕的入口。
 *
 *  跑完产物登记进字幕列表并置为生效值（服务端 `assets::bind_asr_artifact`），
 *  要不要接着检测由用户决定（主按钮 / 事件检测）。
 *  同一模式跑过就不再跑（`canRunAsr`，服务端对 `stage=transcribe` 也按模式 409 兜底）。 */
async function runAsr() {
  if (busy.value) { toast('该项目正在' + busyText.value + '，请稍候'); return }
  if (!canRunAsr.value) {
    toast(`「${asrMode.value}」模式的 ASR 已经跑过 —— 换个模式再跑，或直接在字幕列表里选一份`)
    return
  }
  chainDetect.value = false
  completed.value = null
  await startStage('transcribe')
}

/** 「事件检测」：只跑事件检测（不剪片段）—— 有字幕就能随时重跑，已剪片段不受影响。 */
async function runDetectOnly() {
  if (busy.value) { toast('该项目正在' + busyText.value + '，请稍候'); return }
  if (!hasTranscript.value) { toast('还没有字幕，先「只识别」或选一份字幕'); return }
  chainDetect.value = false
  completed.value = null
  await startStage('detect')
}

/** 处理选项里的「重新查找事件」：有已剪片段时不重跑检测（会让 events 与片段错位），
 *  直接去挑片段页补切；没有片段才重跑检测（`startStage` 自带二次确认）。 */
function onRedetect() {
  if (hasClips.value) {
    toast('已有剪好的片段：去「挑片段」页重新检测或补切')
    router.push(slicesPath.value)
    return
  }
  runDetectOnly()
}

/** 主按钮：已有亮点 → 直接去切片页；否则按「自动剪出」开关走一键处理或两段式。 */
async function onPrimary() {
  if (!hasVideo.value) return
  if (hasEvents.value) { goSlices(); return }
  if (autoCut.value) await runAll()
  else await runPrepare()
}

function goSlices() {
  router.push(slicesPath.value)
}

/** 一键处理：**一个任务**跑完 识别 → 查找事件 → 剪出片段（后端 `stage=process` 会跳过已完成阶段）。
 *  已有亮点/片段时二次确认 —— 文案（`redetectWarning`）写明片段不会被删。 */
async function runAll() {
  if (busy.value) { toast('该项目正在' + busyText.value + '，请稍候'); return }
  if (hasClips.value) {
    toast('已有剪好的片段：去「挑片段」页重新检测或补切')
    router.push(slicesPath.value)
    return
  }
  if (hasEvents.value) {
    const ok = await confirmDialog(redetectWarning(), { title: '覆盖上一次的处理结果？', danger: true })
    if (!ok) return
  }
  completed.value = null
  try {
    const r = await projectData.pipeline(pid.value, { stage: 'process', resume: 'auto' })
    attachJob(r.id, 'stage_process', !!r.queued)
    if (r.queued) toast(QUEUED_HINT)
  } catch (e) {
    toast(e.message, 'err')
  }
}

/** 两段式（「自动剪出片段」关掉时）：没字幕先转写（完成后自动接检测），已有字幕就只检测。
 *
 *  ⚠️ 这里**不会再起第二次转写**：只要工程里已经有转写（ASR 产物或选中的外部字幕），
 *  这个按钮只做（重新）检测。想再出一份 ASR 产物走「只识别」（每种模式各一次）。 */
async function runPrepare() {
  if (busy.value) { toast('该项目正在' + busyText.value + '，请稍候'); return }
  completed.value = null
  if (!hasTranscript.value) {
    // 查找事件勾选才自动接检测（阶段可选：用户可以只跑识别）
    chainDetect.value = stageSel.detect
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

/** 阶段英文名 → 行内中文（服务端 `stage_started` 给的是 `AudioExtractor` / `Transcriber` / …）。 */
function stageText(name) {
  const map = { AudioExtractor: '提取音频', Transcriber: '转写', EventDetector: '检测', Cutter: '剪出片段' }
  return map[name] || '处理'
}

/** 右栏「任务日志」：当前任务的日志行（详情回填最近 50 行 + SSE Log 事件实时追加，上限 200 行）。 */
const jobLogs = ref([])
const joblogEl = ref(null)
watch(() => jobLogs.value.length, async () => {
  await nextTick()
  if (joblogEl.value) joblogEl.value.scrollTop = joblogEl.value.scrollHeight
})

function attachJob(id, kind, queued = false) {
  Object.assign(job, { id, kind, status: queued ? 'queued' : 'running', current: 0, total: 0, message: queued ? '排队中' : '已启动' })
  stageNow.value = ''
  // 右栏日志：先清空，再用任务详情回填最近日志（SSE Log 事件随后实时追加）
  jobLogs.value = []
  jobsApi.detail(id).then((j) => {
    jobLogs.value = (j?.logs || []).slice(-200).map((l) => ({ level: l.level, message: l.message }))
  }).catch(() => {})
  // `watchJob()` 返回 `{ close() }` 对象（不是函数）：当函数调会抛错，
  // 且下面的订阅就不会执行 → 任务永远停在「运行中」。
  jobClose?.close?.()
  jobClose = watchJob(id, {
    onEvent(e) {
      if (e.type === 'progress') { job.current = e.current; job.total = e.total }
      if (e.type === 'log') {
        job.message = e.message
        jobLogs.value.push({ level: e.level, message: e.message })
        if (jobLogs.value.length > 200) jobLogs.value.splice(0, jobLogs.value.length - 200)
      }
      if (e.type === 'stage_started') {
        stageNow.value = e.stage || ''
        job.message = stageText(e.stage) + ' 运行中'
      }
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
      stageNow.value = ''
      reloadJobs() // 别等 3 秒轮询：作用域门控要立刻松开
      if (s.status === 'succeeded') toast('任务完成')
      else if (s.status === 'cancelled') toast('任务已停止')
      else toast('任务失败：' + (s.error || s.message || '未知错误'), 'err')
      await load()
      // 转写成功 → 接着起检测任务（转写失败就别往下走，留着让用户处理）
      const chained = chainDetect.value && kind === 'stage_transcribe'
      // "素材已切换"那句提示只在**检测真的跑了**之后才过期：单独跑一次「只识别」
      // （不接检测）没有消费用户切过的弹幕/字幕，提示要留着。
      if (s.status === 'succeeded' && (chained || kind === 'stage_detect')) assetDirty.value = false
      // 成功 → 取结果数字给「下一步」条；分步跑（只识别/事件检测）不报片段数
      if (s.status === 'succeeded' && kind === 'stage_process') await captureResult(true)
      else if (s.status === 'succeeded' && !chained && kind === 'stage_detect') await captureResult(false)
      if (chained) {
        chainDetect.value = false
        if (s.status === 'succeeded') startStage('detect')
      }
    }
  })
  toast('已启动')
}

/** 停任务：走 `/api/jobs/:id/cancel`（服务端在阶段边界生效）。 */
async function cancelJob() {
  if (!job.id || canceling.value) return
  canceling.value = true
  try {
    await jobsApi.cancel(job.id)
    toast('已请求停止')
  } catch (e) {
    toast(e.message || '停止失败', 'err')
  } finally {
    canceling.value = false
  }
}

/** 取结果数字（亮点 / 片段）。读不到就保持 `null` —— 别把"读失败"报成「已找到 0 个」。 */
async function captureResult(cut) {
  try {
    const [ev, cl] = await Promise.all([
      projectData.events(pid.value, '读取事件列表').catch((e) => {
        if (e?.status === 404) return { events: [] }
        throw e
      }),
      projectData.clips(pid.value, '读取切片列表').catch((e) => {
        if (e?.status === 404) return []
        throw e
      })
    ])
    const list = (ev && ev.events) || []
    completed.value = {
      events: list.length,
      clips: (cl || []).length,
      firstLabel: String(list[0]?.label || ''),
      cut: !!cut
    }
  } catch {
    completed.value = null
  }
}

/** `/api/health.llm`：**读 `stores/health.js` 的唯一缓存**（首页 checklist 同源），别各拉一次。
 *  失败时回落 null（不显示横幅 = 不误报"没配"），不挡页面。 */
async function loadHealth() {
  const h = healthPayload() || await refreshHealth()
  llmMissing.value = h?.llm === false
}

/** 404 空态：去首页的「添加录像」重新导入（同一个目录会绑回同一个工程）。 */
function goImport() {
  router.push({ path: '/workbench', query: { add: '1' } })
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

// 这个项目的管线任务在**别处**跑完（或本页重新挂载后才结束）时，状态 chips / 素材也要跟着刷新。
// 本页自己发起的任务由 `watchJob` 的 onDone 处理（这里按 id 跳过，免得同一次结束刷两遍）。
// ⚠️ 必须用返回的退订函数在 onUnmounted 里退订：`router-view :key=pid` 会让本视图反复挂载，
// 不退订就会攒出一串陈旧回调 —— 任务一结束它们同时各打一整套 project-data 请求（见 useJobs.js）。
const offJobDone = onJobDone((j) => {
  if (!j || String(j.project ?? '') !== String(pid.value)) return
  if (j.id && j.id === job.id) return
  if (jobRunning.value) return
  if (!/^(stage_|pipeline$|cut_)/.test(String(j.kind || ''))) return
  load()
})

function onKey(e) {
  if (isEditableKeyEvent(e)) return
  // 按钮 / 链接 / `<summary>` 自己的 Space 激活别被播放快捷键抢走（折叠区要能键盘展开）
  const tag = String(e.target?.tagName || '').toUpperCase()
  if (tag === 'BUTTON' || tag === 'A' || tag === 'SUMMARY') return
  if (e.code === 'Space') {
    e.preventDefault()
    togglePlay()
  }
}

onMounted(() => {
  load()
  loadHealth()
  window.addEventListener('keydown', onKey)
})
onUnmounted(() => {
  jobClose?.close?.()
  offJobDone()
  // ⚠️ **卸载前把待落盘的改动发出去**：`onTrackSave` 有 500ms 防抖，切路由/切项目时只
  //    `clearTimeout` 会把最后一批改动（尤其"改字"）直接丢掉。SPA 内切路由 `fetch` 不会被取消
  //    （整页关闭才需要 sendBeacon/keepalive，那属于另一条路）。
  //    pid/segments 由 `saveSubs` 走 `ownPid` 快照 —— 卸载时 `route` 已指向新项目，别读 `pid.value`。
  if (saveTimer) {
    clearTimeout(saveTimer)
    saveTimer = null
    saveSubs()
  }
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
.side-col { display:flex; flex-direction:column; gap:12px; align-self:start; min-width:0; }
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
.run-btn { justify-content:center; }
/* 识别流程卡：一行 = 复选节点 + 主按钮；运行中第二行 = 进度 + 停止/任务（旧四层堆叠已压扁）。 */
/* ── 步骤条（识别 → 查找事件 → 剪出片段）：flow-card 内可伸缩的一段，不是独立卡片 ── */
.prep-steps { display:flex; align-items:center; gap:6px; flex:0 1 auto; min-width:0; flex-wrap:wrap; }
.prep-run { display:flex; align-items:center; gap:8px; flex:1 1 240px; min-width:220px; }
.prep-run-label { font-size:13px; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
/* 右栏任务日志：mono 小字滚动区（详情回填 + SSE Log 事件实时追加，自动滚到底） */
.joblog { max-height:320px; overflow-y:auto; padding:6px 8px;
  font-family:var(--mono, ui-monospace, SFMono-Regular, Menlo, monospace); font-size:11px; line-height:1.6;
  border:1px solid var(--line-soft); border-radius:var(--r-sm); background:var(--bg-soft); }
.joblog-line { padding:1px 0; border-bottom:1px dashed var(--line-soft); overflow-wrap:anywhere; }
.joblog-line:last-child { border-bottom:none; }
.joblog-line.error { color:var(--red-deep); }
.joblog-line.warn { color:var(--amber-deep); }
.pstep { display:flex; align-items:center; min-width:0; flex:none; }
/* 阶段开关 pill：亮（主色底+粉字+实线）= 纳入；暗（灰字+虚线框）= 排除 —— 变亮变暗即设置 */
.pstep-tag { padding:5px 16px; border-radius:var(--r-full); font-size:13.5px; font-weight:var(--fw-semi);
  border:1.5px solid var(--line); background:transparent; color:var(--ink-faint);
  cursor:pointer; transition:border-color var(--dur-fast) var(--ease), background var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease);
  line-height:1.4; white-space:nowrap; }
.pstep-tag.dim { border-style:dashed; }
.pstep-tag:hover:not(:disabled) { border-color:var(--ink-dim); color:var(--ink-dim); }
.pstep-tag.on { border-color:var(--main); background:var(--main-soft); color:var(--pink-text); }
.pstep-tag.on:hover:not(:disabled) { border-color:var(--main-glow, var(--main)); }
.pstep-tag.done { border-color:var(--green); background:var(--green-soft); color:var(--green-deep); }
.pstep-tag.active { border-color:var(--main); color:var(--pink-text); }
.pstep-done { color:inherit; }
.pstep-tag:disabled { opacity:.5; cursor:not-allowed; }
.pstep-line { flex:0 0 36px; height:2px; border-radius:2px; background:var(--line); }
.pstep-line.on { background:var(--green); }
@media (max-width:900px) {
  .prep-steps { flex-basis:100%; order:3; overflow-x:auto; }
}

.flow-card { display:flex; flex-wrap:wrap; align-items:center; gap:8px 12px; }
.flow-card > .row { flex:1 1 auto; margin:0 !important; }
.flow-card > .run-btn { width:auto; min-width:150px; margin:0 0 0 auto; }
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
/* ---- 简化后的披露区 / 横幅（右列只剩一块卡：主按钮 + 两个 <details>） ---- */
/* 「处理选项」默认收起、「手动切片」默认展开；观感与设置页 `.form-adv` 同一套。 */
.adv-blk { margin-top:12px; border-top:1px dashed var(--line-soft); padding-top:6px; }
/* 处理选项 / 手动切片各自在右栏卡片里：不再重复顶部分隔线 / 上边距。 */
.materials-blk, .manual-blk { margin-top:0; border-top:none; padding-top:0; }
.adv-blk > summary { cursor:pointer; font-weight:700; font-size:13px; color:var(--ink-dim);
  display:flex; align-items:center; gap:8px; padding:4px 0; list-style:none; }
.adv-blk > summary::-webkit-details-marker { display:none; }
.adv-blk > summary::before { content:"▸"; color:var(--ink-faint); flex:none; }
.adv-blk[open] > summary::before { content:"▾"; }
.adv-blk > summary:hover { color:var(--ink); }
/* 「自动剪出片段」开关行 */
.opt-row { display:flex; align-items:center; gap:8px; font-size:12.5px; color:var(--ink); cursor:pointer; }
.opt-row input[type="checkbox"] { width:auto; flex:none; margin:0; }
.opt-row.dis { opacity:.55; cursor:not-allowed; }
/* 「重新处理」：占满一行的危险按钮（不是主 CTA） */
.block-btn { width:100%; justify-content:center; }
/* LLM 未配置：整页一行琥珀色提示，不拦任何操作 */
.llm-banner { display:flex; align-items:center; gap:8px; flex-wrap:wrap; margin-bottom:12px;
  padding:8px 12px; border-radius:var(--r-sm); background:var(--amber-soft); color:var(--amber-deep);
  border:1px solid var(--amber); font-size:12.5px; }
/* 完成后的单条提示（只找到一条事件时） */
.completed-hint { margin:-6px 0 10px; }
</style>
