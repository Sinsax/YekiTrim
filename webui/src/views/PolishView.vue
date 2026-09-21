<template>
  <div>
    <!-- 紧凑页头一行：chip + 片段标题（竖高省给预览；哪个片段由面包屑/片段卡说明） -->
    <div class="page-head compact-head">
      <span class="chip p">精修</span>
      <h1 class="m-0">{{ dir ? title : '选择要精修的片段' }}</h1>
      <span v-if="!dir" class="sub m-0">从片段库选择一个已剪切片段进入精修编辑器</span>
      <div class="actions"></div>
    </div>

    <!-- 切片列表读不到：**必须与"真的没有切片"分开**（以前失败被 catch 成空数组，
         页面显示"还没有可精修的切片"，看着像切片被删了） -->
    <div v-if="clipsError" class="error-banner">
      <Icon name="alert" :size="15" />
      <span>切片列表读取失败：{{ clipsError }}</span>
      <button class="mini-btn" style="margin-left:auto" @click="load({ silent: true })">重试</button>
    </div>

    <!-- 无切片：空态引导（不报错不白屏）。用 v-else-if 接在错误条后面：读取失败时**只**出错
         —— 否则它会被下面的 v-else 当成"有内容"而把整个编辑器壳渲染出来 -->
    <div v-else-if="!clips.length && !loading" class="card card-blk" style="max-width:520px;margin:48px auto;text-align:center;padding:36px">
      <b style="font-size:15px">还没有可精修的切片</b>
      <p class="small muted" style="margin:8px 0 16px">先在「切片」页生成片段，再回到这里逐段精修</p>
      <router-link class="btn btn-primary" :to="`/p/${pid}/slices`">去切片</router-link>
    </div>

    <template v-else>
      <div v-if="loading" class="card"><div class="skeleton"></div></div>

      <!-- 片段配置读不到、手上又**没有**可用数据 → 错误卡，**必须排在下一条「未初始化」前面**。
           以前读失败会让 ready 停在 false，渲染出来就是「未初始化精修项目」+ 一颗「初始化」按钮 ——
           用户会把一个已存在的工程当成没初始化、再点一次初始化。读取失败要用失败的样子出现。
           带 `!ready` 是因为：已经有数据时的读取失败（静默刷新）**不许卸载编辑器**
           （那样预览视频、选框与滚动位置全丢，见下一条分支里的横幅）。 -->
      <div v-else-if="cfgError && !ready" class="card card-blk" style="max-width:560px">
        <div class="error-banner">
          <Icon name="alert" :size="15" />
          <span>片段配置读取失败：{{ cfgError }}</span>
          <button class="mini-btn" style="margin-left:auto" @click="load()">重试</button>
        </div>
        <p class="muted small m-0">这不是「未初始化」—— 工程文件可能还在，只是这次没读到。重试之前别点初始化。</p>
      </div>

      <!-- 列表模式：没有上次操作记录 → 只显示片段列表 + 空态，视频/编辑器不加载 -->
      <div v-else-if="!clipName" class="card card-blk">
        <div class="row spread mb-8">
          <h2 class="m-0 card-h2"><Icon name="list" :size="15" /> 片段（{{ clips.length }}）</h2>
          <span class="muted small">点击一个片段进入精修编辑器</span>
        </div>
        <FragmentList :rows="fragRows" :current="clipName" @goto="gotoClip" @init="initClipByName" />
        <div class="empty small mt-12">未选择片段：编辑器与视频会在选中片段后加载。</div>
      </div>

      <!-- 未初始化：初始化卡 + 片段列表（否则无法切换/初始化其他片段） -->
      <div v-else-if="!ready" class="grid cols-main">
        <div class="card">
          <h2>未初始化精修项目</h2>
          <p class="muted small">将解析字幕/弹幕并创建 polish.toml。</p>
          <div class="row mt-8">
            <button class="btn btn-primary" :disabled="scopeBusy" :title="scopeBusy ? '该片段正在' + scopeBusyText : ''" @click="initProject">
              <Icon name="sparkles" :size="14" /> 初始化
            </button>
          </div>
        </div>
        <div class="card card-blk">
          <div class="row spread mb-8">
            <h2 class="m-0 card-h2"><Icon name="list" :size="15" /> 片段（{{ clips.length }}）</h2>
            <span class="muted small">点击切换，或在行内初始化</span>
          </div>
          <FragmentList :rows="fragRows" :current="clipName" @goto="gotoClip" @init="initClipByName" />
        </div>
      </div>

      <template v-else>
        <!-- 已经有数据时的读取失败（静默刷新 / 任务后重载）：**只加一条横幅，不卸载编辑器** ——
             换掉子树会丢预览播放位置、选框状态与滚动位置。数据仍是上一次成功读到的那份。 -->
        <div v-if="cfgError" class="error-banner">
          <Icon name="alert" :size="15" />
          <span>片段配置刷新失败，显示的是上一次的数据：{{ cfgError }}</span>
          <button class="mini-btn" style="margin-left:auto" @click="load({ silent: true })">重试</button>
        </div>

        <!-- 片段卡（小卡，只做跳转）：默认一行，可展开两行 -->
        <div class="card frag-strip">
          <div class="frag-strip-row">
            <FragmentList :rows="fragRows" :current="clipName" :card-rows="ui.fragRows"
                          @goto="gotoClip" @init="initClipByName" />
            <button class="mini-btn frag-more" :title="ui.fragRows === 2 ? '收成一行' : '展开成两行'"
                    @click="toggleFragRows">
              {{ ui.fragRows === 2 ? '收起' : '展开' }}
            </button>
          </div>
        </div>

        <!-- 左列 = 预览（属性面板在左、舞台在右）；右栏 = 生成与导出 -->
        <div class="grid cols-main mt-16 edit-grid">
          <div class="card">
            <div class="row spread">
              <h2 class="m-0 card-h2"><Icon name="video" :size="16" /> 视频预览（preview 效果）</h2>
              <span v-if="selectRegion" class="chip p">选框模式 · 运镜已暂停</span>
              <span v-else-if="pointEdit" class="chip p">点选落点 · 未保存</span>
              <span v-else class="muted small">字幕 / 贴纸 / 运镜 / 弹幕 / 音效实时预览</span>
            </div>
            <!-- 左：属性面板挂载点（TimelineEditor 用 Teleport 把属性面板投进来，与时间轴分离）
                 右：预览舞台 -->
            <div class="preview-row">
              <div id="tl-props-host" class="props-host"></div>
              <div class="stage-col">
                <PolishPreview ref="previewRef" :key="previewKey" :pid="pid" :dir="dir" :config="cfg" :subtitles="subs" :danmaku="dm" :meta-duration="cfg?.duration || 0" :wgpu-render="wgpuRender" :select-mode="!!selectRegion" :select-rect="selRect" :select-label="selLabel" :point-mode="pointMode" :point="pointEdit ? draftPos : stickerPos" :point-label="'贴纸落点 · ' + (pointEdit ? draftPosPct : stickerPosPct)" @time="onPreviewTime" @playing="onPreviewPlaying" @update:select-rect="onSelectRect" @update:point="onPointPick" />
                <div v-if="selectRegion" class="hint mt-8">拖框体平移、拖 8 个角/边缩放、空白拖拽重画；右侧「区域与落点」里保存或清除。</div>
                <div v-else-if="pointEdit" class="hint mt-8">点一下预览即为新落点（未保存）；右侧「区域与落点」里保存或取消。</div>
              </div>
            </div>
          </div>

          <div class="card">
            <div class="row spread mb-8">
              <h2 class="m-0 card-h2"><Icon name="sparkles" :size="15" /> 生成与导出</h2>
              <div class="row" style="gap:4px">
                <button class="mini-btn" :class="{ selected: genTab === 'export' }" @click="genTab = 'export'">导出</button>
                <button class="mini-btn" :class="{ selected: genTab === 'stage' }" @click="genTab = 'stage'">画面构成</button>
              </div>
            </div>

            <template v-if="genTab === 'export'">
            <!-- LLM 剪辑（参数：区域编辑 / 贴纸来源）—— 整块在「渲染导出」上方 -->
            <div class="gen-block">
              <div class="row spread mb-6">
                <b class="t" style="margin:0"><Icon name="sparkles" :size="12" /> LLM 剪辑</b>
                <span class="muted small">下列为参数</span>
              </div>

              <!-- 参数 1：区域与落点 —— 人物 / 弹幕区域 / 贴纸落点 同一行。
                   三者的交互是同一种（在左侧预览上取坐标），所以并列一行；
                   贴纸那格是"自动 / 固定"二选一，切到固定后再点预览取点。 -->
              <div class="small muted mb-4">区域与落点<span class="hint"> · 在预览上点选</span></div>
              <div class="row mb-6" style="gap:6px">
                <button class="mini-btn" style="flex:1;justify-content:center" :class="{ selected: selectRegion === 'liver' }" :disabled="!ready"
                        title="框选主播人物：运镜聚焦到框中心" @click="toggleSelect('liver')">
                  <Icon name="crop" :size="11" /> 人物
                </button>
                <button class="mini-btn" style="flex:1;justify-content:center" :class="{ selected: selectRegion === 'danmaku_area' }" :disabled="!ready"
                        title="框选弹幕显示区域（供运镜参考）" @click="toggleSelect('danmaku_area')">
                  <Icon name="crop" :size="11" /> 弹幕
                </button>
                <button class="mini-btn" style="flex:1;justify-content:center"
                        :class="{ selected: stickerCfg.position_mode === 'manual' }" :disabled="stickerBusy || !ready"
                        :title="stickerCfg.position_mode === 'manual' ? '固定落点：全部贴纸都落在同一点，切到固定后在预览上点选' : '自动避让：按安全区算法逐条找位置'"
                        @click="toggleStickerMode">
                  <Icon name="pin" :size="11" /> {{ stickerCfg.position_mode === 'manual' ? '固定' : '自动' }}
                </button>
              </div>
              <template v-if="selectRegion">
                <div class="row spread small mb-4">
                  <span class="mono">{{ rectText }}</span>
                  <span v-if="hasFreeRect" class="chip p">自由选框</span>
                  <span v-else class="chip n">九宫格默认</span>
                </div>
                <div class="row mb-6" style="gap:6px">
                  <!-- 选框保存是"同步加锁写 polish.toml"，与任务不冲突：任务在跑时照样能存，
                       随后的重算（process）会排到该片段空闲后自动补跑（reprocessLayout） -->
                  <button class="mini-btn primary" @click="applySelect"><Icon name="check" :size="11" /> 保存选框</button>
                  <button class="mini-btn" :disabled="!hasFreeRect" @click="clearSelect">清除选框</button>
                </div>
                <div class="hint mb-6">{{ regionHint }}</div>
              </template>
              <template v-else-if="stickerCfg.position_mode === 'manual'">
                <!-- 固定落点是**草稿态**：在预览上点选只改 draft，「保存落点」才写 polish.toml。
                     以前是"固定模式即常驻点选、点哪儿存哪儿"，看一眼预览就会改坏配置。 -->
                <div class="row spread small mb-4">
                  <span class="mono">{{ pointEdit ? draftPosPct : stickerPosPct }}</span>
                  <span v-if="pointEdit" class="chip p">编辑中 · 未保存</span>
                </div>
                <div class="row mb-6" style="gap:6px">
                  <template v-if="pointEdit">
                    <button class="mini-btn primary" :disabled="stickerBusy" @click="savePointEdit"><Icon name="check" :size="11" /> 保存落点</button>
                    <button class="mini-btn" :disabled="stickerBusy" @click="cancelPointEdit">取消</button>
                  </template>
                  <button v-else class="mini-btn" :disabled="stickerBusy || !ready" @click="startPointEdit"><Icon name="pin" :size="11" /> 改落点</button>
                </div>
                <div class="hint mb-6">{{ pointEdit ? '在左侧预览里点一下选新落点（贴纸中心对齐该点），保存后生效。' : '全部贴纸的中心都落在这一点。' }}</div>
              </template>

              <!-- 参数 2：贴纸来源。只保留一组可点 chip：亮 = 启用，灰 = 停用。 -->
              <div class="small muted mb-4">贴纸来源<span class="hint"> · LLM 可用的贴图来源</span></div>
              <div class="ss-options">
                <button
                  v-for="src in allStickerOptions"
                  :key="src.name"
                  class="ss-opt"
                  :class="{ on: stickerSource.includes(src.name) }"
                  :title="src.tags?.length ? 'tags: ' + src.tags.join(' / ') : (src.description || '')"
                  @click="toggleStickerSource(src.name)"
                >
                  <b>{{ src.name }}</b>
                  <span v-if="src.tags?.length" class="ss-opt-meta">{{ src.tags.length }} tags</span>
                  <span v-else-if="src.builtin" class="ss-opt-meta">内置</span>
                </button>
                <!-- "读不到"与"库是空的"必须分开：前者不该让用户去翻 stickers/ 目录 -->
                <div v-if="stickerSourcesError" class="row small" style="gap:6px;align-items:center">
                  <span class="muted">贴纸来源读取失败：{{ stickerSourcesError }}</span>
                  <button class="mini-btn" @click="loadStickerSources">重试</button>
                </div>
                <div v-else-if="!allStickerOptions.length" class="muted small">贴纸库为空（程序目录 stickers/ 下放置贴纸包）</div>
              </div>

              <!-- 参数 3：字幕字体（字体库 = 程序目录 resources/fonts/）-->
              <div class="small muted mb-4">字幕字体<span class="hint"> · 出片与预览的字幕</span></div>
              <select class="fld-input" :value="subtitleFont" :disabled="fontSaving || !ready" @change="onSubtitleFontChange">
                <option v-for="o in fontOptions" :key="o.value" :value="o.value">{{ o.label }}</option>
              </select>
              <div v-if="fontsError" class="row small mt-4" style="gap:6px;align-items:center">
                <span class="muted">字体库读取失败：{{ fontsError }}</span>
                <button class="mini-btn" @click="loadFonts">重试</button>
              </div>
              <!-- root 只有 admin 拿得到（服务器安装路径不给普通用户），非 admin 不显示这一段 -->
              <div v-else class="hint mt-4">
                库内 {{ fonts.length }} 个字体<span v-if="fontsRoot"> · 目录 <span class="mono">{{ fontsRoot }}</span></span>
              </div>

              <!-- 动作（去静音只在时间轴头部，那里还带「撤销」，此处不再重复） -->
              <button class="btn mt-12" style="width:100%;justify-content:center" :disabled="scopeBusy" :title="scopeBusy ? '该片段正在' + scopeBusyText : '全部重做：重新分段 + 重新生成全部事件（失败的分段会在时间轴上标色，可单独重做那一段）'" @click="runLlmJob"><Icon name="sparkles" :size="13" /> LLM 剪辑（全部重做）</button>
              <div class="hint mt-4">
                想只补某一段？在时间轴上点那一段，属性面板里有「只重做本段事件」；
                没产出事件的段会标成琥珀色。
              </div>
            </div>

            <!-- 导出（在 LLM 参数块下方） -->
            <button class="btn btn-primary mt-12" style="width:100%;justify-content:center" :disabled="scopeBusy" :title="scopeBusy ? '该片段正在' + scopeBusyText : ''" @click="renderModal = true"><Icon name="film" :size="13" /> 渲染导出</button>

            <!-- 别人（别的视图/标签页）占用这个片段时也提示一句，别让按钮变灰得没头没脑 -->
            <div v-if="!jobRunning && scopeBusyJob" class="hint mt-8">
              该片段正在「{{ kindText(scopeBusyJob.kind) }}」，等它结束再做这一步。
            </div>
            </template>

            <!-- 画面构成页签：画面尺寸 / 主画面位置 / 背景。值存 polish.toml 的 [canvas] / [stage]，
                 出片与预览共用同一份推导（Rust `resolve_stage` → `_canvas`），
                 所以这里改完预览当场就变，且与成片一致 -->
            <template v-else>
              <div class="small muted mb-4">画面尺寸<span class="hint"> · 画布方向，立即保存</span></div>
              <div class="row mb-6" style="gap:6px">
                <button v-for="o in orientationOptions" :key="o.value" class="mini-btn" style="flex:1;justify-content:center"
                        :class="{ selected: (canvasCfg.orientation || 'landscape') === o.value }" :disabled="stageBusy || !ready"
                        @click="patchCanvas({ orientation: o.value })">{{ o.label }}</button>
              </div>
              <div class="hint mb-8">{{ canvasSummary }}。切方向后位置选项会跟着换方向（左中右 ↔ 上中下），预览舞台与出片画布会立即跟随（预览需重取配置，自动刷新）。改过主画面位置或方向后，自动贴纸落点要重跑后处理才更新。</div>

              <div class="small muted mb-4">主画面位置<span class="hint"> · 比例一致时铺满，位置看不出效果</span></div>
              <div class="row mb-8" style="gap:6px">
                <button v-for="p in positionOptions" :key="p.value" class="mini-btn" style="flex:1;justify-content:center"
                        :class="{ selected: stageCfg.position === p.value }" :disabled="stageBusy || !ready"
                        @click="patchStage({ position: p.value })">{{ p.label }}</button>
              </div>

              <div class="small muted mb-4">背景<span class="hint"> · 主画面让出来的边补什么；竖屏一直生效（震动等位移会露边），横屏仅留边时生效</span></div>
              <div class="row mb-6" style="gap:6px">
                <button v-for="b in bgOptions" :key="b.value" class="mini-btn" style="flex:1;justify-content:center"
                        :class="{ selected: stageBgDisplay === b.value }" :disabled="stageBusy || !ready"
                        @click="patchStage(b.value === 'color' ? { bg_mode: 'color', bg_color: stageBgColor } : { bg_mode: b.value })">{{ b.label }}</button>
              </div>
              <div v-if="stageBgDisplay === 'color'" class="row mb-6" style="gap:8px;align-items:center">
                <input type="color" style="width:46px;height:26px;padding:0;border:1px solid var(--line);border-radius:6px;background:none"
                       :value="stageBgColor" :disabled="stageBusy || !ready"
                       @change="patchStage({ bg_mode: 'color', bg_color: $event.target.value })">
                <span class="mono small">{{ stageBgColor }}</span>
              </div>
              <div v-if="stageCfg.bg_mode === 'blur'" class="row mb-6" style="gap:8px;align-items:center">
                <span class="small muted" style="flex:none">模糊强度</span>
                <input type="range" class="flex-1" min="0.2" max="4" step="0.1" :value="stageCfg.blur_scale ?? 1"
                       :disabled="stageBusy || !ready" @change="patchStage({ blur_scale: Number($event.target.value) })">
                <span class="mono small">{{ Number(stageCfg.blur_scale ?? 1).toFixed(1) }}×</span>
              </div>

              <div class="row mb-6" style="gap:8px;align-items:center">
                <span class="small muted" style="flex:none">主画面缩放</span>
                <input type="range" class="flex-1" min="0.5" max="2" step="0.05" :value="stageCfg.scale ?? 1"
                       :disabled="stageBusy || !ready" @change="patchStage({ scale: Number($event.target.value) })">
                <span class="mono small">{{ Number(stageCfg.scale ?? 1).toFixed(2) }}×</span>
              </div>
              <div class="row mb-8" style="gap:8px;align-items:center">
                <span class="small muted" style="flex:none">额外留白</span>
                <input type="range" class="flex-1" min="0" max="200" step="2" :value="stageCfg.padding ?? 0"
                       :disabled="stageBusy || !ready" @change="patchStage({ padding: Number($event.target.value) })">
                <span class="mono small">{{ Number(stageCfg.padding ?? 0) }}px</span>
              </div>
              <div class="hint">{{ stageHint }}</div>
            </template>
            <div v-if="job.id && (jobRunning || jobFinished)" class="job-box mt-8">
              <div class="row spread small">
                <b>{{ jobLabel(job.kind) }}</b>
                <div class="row">
                  <span v-if="jobRunning" class="muted small mono">运行 {{ jobTimeText }}</span>
                  <button v-if="jobRunning" class="mini-btn danger" :disabled="canceling" @click="cancelJob"><Icon name="x" :size="10" /> 中断</button>
                  <span class="chip" :class="jobChipClass">{{ jobStatusText }}</span>
                </div>
              </div>
              <div v-if="job.total > 0" class="progress mt-6"><div :style="{ width: pctJob + '%' }"></div></div>
              <div class="muted small mt-6 mono">{{ job.error || job.log || (job.status === 'succeeded' ? '任务完成' : jobRunning ? '运行中…' : '') }}</div>
            </div>
          </div>
        </div>

        <!-- 时间轴编辑器独占一整行（在预览/生成两栏之下）：横向空间全给刻度与泳道。
             标题/提示由 slot 交给组件，省掉独立卡片头；去静音只在这块，右侧不再重复。
             改动走「本地即时镜像 → 预览实时刷新 → 防抖落盘 → 防抖后处理」这条链。 -->
        <div class="card tl-card mt-16">
          <TimelineEditor ref="timelineRef" :key="'tl' + segKey" :dir="dir" :pid="pid" :current-time="currentTime" :playing="previewPlaying" :reprocess-busy="scopeBusy" @seek="seekTo" @toggle="togglePreviewPlay" @preview="onTimelinePreview" @done="onEditDone" @reprocess="runProcessJob" @retried="onTimelineRetried" @redo-segment="redoSegment">
            <template #head>
              <b>时间轴编辑器</b>
              <span class="muted small">拖拽移动 / 左右缘缩放 / 双击删除 / 泳道 ＋ 在播放头处添加（特效轨先选细分类型）· 改动立即保存 · 后处理在结束编辑时跑</span>
            </template>
          </TimelineEditor>
        </div>

        <div class="mt-16"><CoverDesigner :key="'cv' + coverKey" :dir="dir" :pid="pid" @changed="refreshClipEntry(true)" /></div>

        <!-- 产物整行：成片 + 封面 + 输出目录 -->
        <div class="card mt-16 artifact-bar">
          <div class="row spread">
            <div class="row" style="gap:10px">
              <b class="t" style="margin:0">产物</b>
              <a v-if="renderedUrl && !renderedFailed" class="mini-btn" :href="renderedUrl" download :title="renderedFile">
                <Icon name="download" :size="12" /> 成片 {{ renderedFile }}
              </a>
              <span v-else class="muted small">未渲染</span>
              <a v-if="coverUrl && !coverFailed" class="mini-btn" :href="coverUrl" download :title="coverFile">
                <Icon name="download" :size="12" /> 封面 {{ coverFile }}
              </a>
              <span v-else class="muted small">无封面</span>
            </div>
            <button class="mini-btn" @click="copyOutDir">打开输出目录（复制路径）</button>
          </div>
          <div class="row mt-8" style="gap:12px">
            <img v-if="coverUrl && !coverFailed" :src="coverUrl" class="cover-thumb" alt="封面" @error="coverFailed = true">
            <video v-if="renderedUrl && !renderedFailed" :src="renderedUrl" controls playsinline class="video-bar" @error="renderedFailed = true"></video>
            <div v-if="!(coverUrl && !coverFailed) && !(renderedUrl && !renderedFailed)" class="muted small">渲染导出或生成封面后在这里查看</div>
          </div>
        </div>
      </template>
    </template>

    <!-- 渲染导出弹窗 -->
    <AppModal :show="renderModal" title="渲染导出" @close="renderModal = false">
      <label>输出文件（**项目内相对路径**，留空使用 polish.toml 的 project.output）</label>
      <input v-model="renderOutput" placeholder="output/clips/clip-01/export.mp4">
      <p class="hint mt-6">
        只接受项目内相对路径；绝对路径会被服务端拒绝（P2 起不再允许任意输出位置）。
        <b>同名文件不会被覆盖</b>：已经存在时自动写成 <span class="mono">名字_1.mp4</span> /
        <span class="mono">_2.mp4</span>…（与封面同一套规则），产物栏显示的就是最新那份。
      </p>
      <template #footer>
        <button class="btn btn-primary" :disabled="scopeBusy" @click="doRender">开始渲染</button>
        <button class="btn" @click="renderModal = false">取消</button>
      </template>
    </AppModal>
  </div>
</template>

<script setup>
import { ref, computed, reactive, onMounted, onUnmounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import { ui, toggleFragRows } from '../stores/ui'
import Icon from '../components/Icon.vue'
import AppModal from '../components/AppModal.vue'
import PolishPreview from '../components/PolishPreview.vue'
import TimelineEditor from '../components/TimelineEditor.vue'
import FragmentList from '../components/polish/FragmentList.vue'
import CoverDesigner from '../components/CoverDesigner.vue'
import { clipMediaUrl } from '../utils'
import { api, apiOr } from '../api/client'
import { addClient } from '../stores/log'
import { scopeQ } from '../api/scope'
import {
  fetchProject, fetchSubtitles, saveProjectPatch,
  initPolish, runLlm, runLlmSegment, runProcess, runRender,
  fetchStickerSources, fetchFonts,
  REGION_LABELS,
} from '../api/polish'
import { ensureFontFace } from '../polish/fonts'
import { freeRect, regionRect } from '../polish/engine'
import { jobsApi, watchJob, busyOn, clipKey, kindText, QUEUED_HINT } from '../api/jobs'
import { useJobs } from '../composables/useJobs'

const route = useRoute()
const router = useRouter()
const routePid = route.params.pid || ''
const pid = ref(routePid)
const routeCfg = scopeQ(pid.value)
const dir = ref('')
const clipName = computed(() => route.params.clip || '')
/** 当前已经加载完成的片段名：用于跳过 `load()` 自己 `router.replace` 触发的 watch。 */
const loadedClip = ref('')
// 记住最后操作过的片段：下次从侧栏进精修页自动回到它（每项目一个键）
const LAST_CLIP_KEY = computed(() => 'vtrim.polish.last.' + pid.value)
watch(clipName, (name) => {
  if (name) { try { localStorage.setItem(LAST_CLIP_KEY.value, name) } catch {} }
  // 片段切换只改路由，必须在这里触发加载；但 `load()` 自己 replace 路由时不要再触发一次。
  if (name === loadedClip.value) return
  load()
})
const title = ref('精修编辑器')
const loading = ref(false)
const ready = ref(false)
const cfg = ref(null)
const subs = ref([])
const dm = ref([])
const clips = ref([])
/** 切片列表读取失败的原因（非空 → 界面走错误条，**不进"还没有可精修的切片"那个空态**）。
 *  以前失败被 `catch { clips = [] }` 伪装成空态，用户会以为切片丢了。 */
const clipsError = ref('')
/** 当前片段的配置读取失败的原因（非空 → 走错误条，**不落到「未初始化精修项目」**） */
const cfgError = ref('')
const clipEntry = ref(null)
/** 片段目录（规范化）→ `/api/project-data/polish` 行，供片段列表显示状态 */
const polishByDir = ref({})
const currentTime = ref(0)
const previewKey = ref(0)
/** 时间轴重挂载计数（仅当磁盘上的 segments/屏蔽轨道被服务端改写：LLM 剪辑 / 初始化 / 去静音）。
 *  静默刷新不再卸载编辑器，所以需要重挂载的组件自己带 key。 */
const segKey = ref(0)
/** 封面设计器重挂载计数（渲染导出会重新生成封面文件） */
const coverKey = ref(0)
const renderModal = ref(false)
const renderOutput = ref('')
const timelineRef = ref(null)
const previewRef = ref(null)

const job = reactive({ id: '', kind: '', status: '', current: 0, total: 0, log: '', error: '' })
let jobClose = null
let jobTimer = null
let canceling = ref(false)
const jobStartMs = ref(0)
const jobElapsed = ref(0)
// 服务端是否带 --features wgpu-render 构建（决定 /api/polish/render-frame 是否可用，缺失时 501）
const wgpuRender = ref(false)

const jobRunning = computed(() => job.status === 'running' || job.status === 'queued')

// ---- 作用域互斥（并行能力的关键）----
// 后端按「资源键」互斥：同片段的渲染/LLM/后处理/封面互斥，不同片段、不同项目、别的
// 项目正在转写……都不影响这一页。这里用同一套键判断"我这个片段忙不忙"，
// 只有自己片段被占用时才禁用按钮（以前是"任何地方有任务在跑就全灰"）。
const { jobs: allJobs, reload: reloadJobs } = useJobs()
const myKeys = computed(() =>
  dir.value && clipName.value ? [clipKey(pid.value, clipName.value)] : []
)
/** 占着这个片段的活动任务（可能是别处发起的，比如切片页的重切、另一个标签页） */
const scopeBusyJob = computed(() => busyOn(allJobs.value, myKeys.value))
const scopeBusy = computed(() => !!scopeBusyJob.value || jobRunning.value)
/** 谁在占用（按钮 title 与提示文案用） */
const scopeBusyText = computed(() => {
  if (jobRunning.value) return jobLabel(job.kind)
  return scopeBusyJob.value ? kindText(scopeBusyJob.value.kind) : ''
})
const pctJob = computed(() => job.total > 0 ? Math.round(job.current * 100 / job.total) : 0)
const jobFinished = computed(() => ['succeeded', 'failed', 'cancelled'].includes(job.status))
const jobStatusText = computed(() => ({
  queued: '排队中', running: '运行中',
  succeeded: '完成', failed: '失败', cancelled: '已取消'
}[job.status] || job.status))
const jobChipClass = computed(() => {
  if (job.status === 'failed' || job.status === 'cancelled') return 'r'
  if (job.status === 'succeeded') return 'g'
  return jobRunning.value ? 'b' : 'n'
})
const jobTimeText = computed(() => {
  const sec = Math.max(0, Math.floor(jobElapsed.value))
  if (sec < 60) return sec + 's'
  const m = Math.floor(sec / 60)
  const s = sec % 60
  return m + 'm' + (s ? s + 's' : '')
})
// 贴纸配置 = polish.toml 顶层 sticker_source（LLM 生成时可选的贴图来源池）
const stickerSource = computed(() => cfg.value?.sticker_source || [])
const stickerSources = ref([]) // 全局可用来源（emoji + 贴纸库包）
/** 贴纸来源列表读取失败的原因（非空 → 不显示"贴纸库为空"这个假空态） */
const stickerSourcesError = ref('')
/** 实际渲染的选项 = 全局可用来源 + 已配置但当前库里没有的来源（后者仍能点掉）。 */
const allStickerOptions = computed(() => {
  const known = new Set(stickerSources.value.map((s) => s.name))
  const extra = stickerSource.value
    .filter((n) => !known.has(n))
    .map((n) => ({ name: n, builtin: false, tags: [], description: '已配置，但当前贴纸库里没有这个来源' }))
  return [...stickerSources.value, ...extra]
})

// ---- 字幕字体（字体库：程序目录 resources/fonts/）----
// 值存 polish.toml 的 `[style.subtitle] font`，**逐片段**生效（每个片段一份 polish.toml）。
// 空串 = 回系统字体栈；非空 = 字体库里的 id（文件名去扩展名）。
// 字体只影响预览与出片渲染 —— 不进任务、不触发 process（贴纸/运镜那些重算不读它）。
const fonts = ref([]) // 库里的字体（只读接口）
const fontsRoot = ref('') // 库目录（只有 admin 拿得到；非 admin 不显示那一行）
const fontsError = ref('')
const fontSaving = ref(false)
const subtitleFont = computed(() => cfg.value?.style?.subtitle?.font || '')
/** 下拉选项：`（系统默认）` + 库内字体；当前值不在库里时补一条，免得 select 落到空态 */
const fontOptions = computed(() => {
  const opts = [{ value: '', label: '（系统默认）' }]
  for (const f of fonts.value) {
    opts.push({ value: f.id, label: f.family && f.family !== f.id ? `${f.family}（${f.id}）` : f.id })
  }
  const cur = subtitleFont.value
  if (cur && !fonts.value.some((f) => f.id === cur)) opts.push({ value: cur, label: `${cur}（不在字体库）` })
  return opts
})

// ---- 布局框选（取代九宫格）：只有「人物」「弹幕区域」两个区域 ----
// 贴纸落点由 process 按 layout.sticker_area 计算，不在 UI 里框选（要改就手改 polish.toml）。
const FRAMEABLE = ['liver', 'danmaku_area']
/** 正在框选的区域；'' = 不在框选模式（一个 ref 同时表达"选谁"和"是否在框选"） */
const selectRegion = ref('')
/** 编辑中的选框（整帧百分比），仅在框选模式下随拖拽更新 */
const selRect = ref({ x0: 0, y0: 0, x1: 100, y1: 100 })
const curRegion = computed(() => (cfg.value?.layout || {})[selectRegion.value] || null)
/** 该区域是否已有自由选框（否则回退到九宫格 grid） */
const hasFreeRect = computed(() => !!freeRect(curRegion.value))
const selLabel = computed(() => REGION_LABELS[selectRegion.value] || selectRegion.value)
const REGION_HINTS = {
  liver: '框住主播画面：运镜聚焦到框中心（比九宫格更准）。',
  danmaku_area: '框住直播间弹幕显示区域（供运镜参考）。'
}
const regionHint = computed(() => REGION_HINTS[selectRegion.value] || '')
/** 展示用矩形：框选模式看编辑值，否则看已保存的选框 / 九宫格包围盒 */
const shownRect = computed(() => {
  if (selectRegion.value) return selRect.value
  return regionRect(curRegion.value) || { x0: 0, y0: 0, x1: 100, y1: 100 }
})
const rectText = computed(() => {
  const r = shownRect.value
  const f = (v) => (Math.round((Number(v) || 0) * 10) / 10) + '%'
  return `${f(r.x0)}, ${f(r.y0)} → ${f(r.x1)}, ${f(r.y1)}`
})

/** 当前区域的选框作为编辑初值（已是自由选框则用现值，否则取九宫格包围盒）。 */
function loadSelRect() {
  selRect.value = regionRect(curRegion.value) || { x0: 0, y0: 0, x1: 100, y1: 100 }
}

function onSelectRect(r) {
  selRect.value = r
}

/** 点区域按钮：同一个再点一次 = 退出框选；点另一个 = 切换区域。 */
function toggleSelect(region) {
  if (selectRegion.value === region) { selectRegion.value = ''; return }
  selectRegion.value = FRAMEABLE.includes(region) ? region : ''
  if (!selectRegion.value) return
  // 与贴纸点选互斥：框选优先，进框选就丢弃点选草稿（避免两层抢指针）
  cancelPointEdit()
  loadSelRect()
  // 定格当前帧，避免边播边框选
  previewRef.value?.pause?.()
}

const pctStr = (v) => (Math.round(Math.min(100, Math.max(0, Number(v) || 0)) * 100) / 100).toFixed(2) + '%'

/**
 * 落盘 layout 改动后重跑 process。
 *
 * 这两个框是 **process 的输入**，不是渲染的输入：Step 2b 按 `layout.liver` /
 * `layout.danmaku_area` 反选安全区、重写每个贴纸的 `x/y`；Step 2d 把 `anchor`
 * 解析成 `origin`/`to_origin`（每次运行都覆盖）。渲染侧只认这些结果坐标
 * （`index.html` 的 `ce.origin || liverOrigin` 里，读 layout 的只是"事件没写 origin"的兜底），
 * 所以不改框就跑渲染，画面还是旧焦点 —— 渲染内部虽然会先跑一次 process 兜底，
 * 但在那之前编辑器/预览显示的都是旧坐标，看着像"没生效"。
 *
 * process 无 LLM、幂等（每次都按当前 layout 全量重算），与时间轴保存走同一条路。
 */
function reprocessLayout() {
  if (scopeBusy.value) {
    // 选框已经存进 polish.toml 了（保存是同步加锁写，随时可存）；重算等片段空下来
    markPendingProcess('选框保存时该片段正忙')
    toast('选框已保存；该片段正在' + scopeBusyText.value + '，结束后会自动重算')
    return
  }
  toast('选框已保存，正在重算运镜与贴纸落点…')
  startJob('process', () => runProcess(pid.value, dir.value), { after: () => load({ silent: true }) })
}

/** 保存选框：写入 layout[区域].top_left/bottom_right（读取时优先于 grid），随后重算。 */
async function applySelect() {
  try {
    const lay = { ...(cfg.value?.layout || {}) }
    const region = { ...(lay[selectRegion.value] || {}) }
    const r = selRect.value
    region.top_left = { x: pctStr(Math.min(r.x0, r.x1)), y: pctStr(Math.min(r.y0, r.y1)) }
    region.bottom_right = { x: pctStr(Math.max(r.x0, r.x1)), y: pctStr(Math.max(r.y0, r.y1)) }
    lay[selectRegion.value] = region
    await saveProjectPatch(pid.value, dir.value, { layout: lay })
    cfg.value = { ...cfg.value, layout: lay }
    selectRegion.value = ''
    reprocessLayout()
  } catch (e) {
    toast(e.message, 'err')
  }
}

/** 清除选框：回到该区域的九宫格 grid（不动 grid 本身，可逆）。同样是布局改动 → 一并重算。 */
async function clearSelect() {
  try {
    const lay = { ...(cfg.value?.layout || {}) }
    const region = { ...(lay[selectRegion.value] || {}) }
    delete region.top_left
    delete region.bottom_right
    lay[selectRegion.value] = region
    await saveProjectPatch(pid.value, dir.value, { layout: lay })
    cfg.value = { ...cfg.value, layout: lay }
    loadSelRect()
    reprocessLayout()
  } catch (e) {
    toast(e.message, 'err')
  }
}

// 重新加载配置后同步选框（框选模式中不打断拖拽）
watch(() => cfg.value?.layout, () => { if (!selectRegion.value) loadSelRect() })
// 产物优先使用后端返回的实际文件名（含 _1/_2 版本），未返回时回退固定名
const coverFailed = ref(false)
const renderedFailed = ref(false)
const coverFile = computed(() => clipEntry.value?.cover_file || 'cover.jpg')
const renderedFile = computed(() => clipEntry.value?.rendered_file || 'vtuber-clip.mp4')
const coverUrl = computed(() => clipMediaUrl(pid.value, dir.value, coverFile.value))
const renderedUrl = computed(() => clipMediaUrl(pid.value, dir.value, renderedFile.value))

function jobLabel(kind) {
  return { llm: 'LLM 生成', render: '渲染导出', process: '后处理', init: '初始化' }[kind] || kind
}

function onPreviewTime(t) {
  currentTime.value = Number(t) || 0
}
/** 预览是不是在播（时间轴的跟随只看这个：拖动/跳转不跟，用户要求） */
const previewPlaying = ref(false)
function onPreviewPlaying(v) {
  previewPlaying.value = !!v
}
function seekTo(t) {
  // 字幕行点击传的是字幕对象；时间轴/进度条传的是数字秒
  const raw = (t && typeof t === 'object') ? (t.start ?? t.end) : t
  const n = Number(raw)
  currentTime.value = Number.isFinite(n) ? Math.round(n * 100) / 100 : 0
  previewRef.value?.seekTo(currentTime.value)
}
function togglePreviewPlay() {
  previewRef.value?.togglePlay()
}

/**
 * 加载当前片段。
 *
 * `silent`（任务完成后的刷新）= 不闪骨架屏、不卸载编辑器、不丢滚动位置，失败时保留旧数据；
 * 首次进入 / 切换片段仍走显式 loading（那本来就该换内容）。
 * 非 silent 时 `loading` 会把整个编辑器换成骨架屏（`v-if="loading"`），
 * 编辑器一旦被卸载，预览视频与滚动位置都会重置——看起来就像浏览器刷新了。
 */
let loadInflight = null
let loadQueued = null

/** 单飞：并发调用合并成一次；完成后如果期间又有人请求，再按最后一次参数补跑一次。 */
function load(opts = {}) {
  if (loadInflight) {
    loadQueued = opts
    return loadInflight
  }
  loadInflight = doLoad(opts).finally(() => {
    loadInflight = null
    if (loadQueued) {
      const next = loadQueued
      loadQueued = null
      load(next)
    }
  })
  return loadInflight
}

async function doLoad(opts = {}) {
  const silent = !!(opts && opts.silent)
  // loading 只在这里拨一次（loadClips 不再自己开关），避免中间出现 editor↔骨架屏 的抖动
  if (!silent) loading.value = true
  try {
    await loadClips()
    if (!clips.value.length) {
      dir.value = ''
      loadedClip.value = ''
      ready.value = false
      cfg.value = null
      return
    }
    // 无 :clip → 自动打开**上次操作过的**片段；没有记录（或已失效）→
    // 片段列表 + 空态（视频/编辑器都不加载），不再随手挑一个默认片段。
    let wanted = clipName.value
    if (!wanted) {
      let last = null
      try { last = localStorage.getItem(LAST_CLIP_KEY.value) } catch {}
      if (last && clips.value.some((c) => c.name === last)) wanted = last
    }
    const entry = wanted ? clips.value.find((c) => c.name === wanted) : null
    if (!entry) {
      if (clipName.value) toast(`片段 ${clipName.value} 不存在，请从列表重新选择`, 'err')
      dir.value = ''
      loadedClip.value = ''
      ready.value = false
      cfg.value = null
      return
    }
    dir.value = entry.dir || clipDirOf(wanted)
    loadedClip.value = wanted
    if (clipName.value !== wanted) {
      router.replace({ name: 'project-polish-clip', params: { pid: pid.value, clip: wanted } })
    }
    try {
      cfg.value = await fetchProject(pid.value, dir.value, true)
      title.value = cfg.value?.project?.name || cfg.value?.source?.video || '精修编辑器'
      ready.value = true
      cfgError.value = ''
    } catch (e) {
      // 静默刷新失败：保留旧画面但必须报出来，否则用户以为改动已生效
      // 非静默（首次进入/切片段）失败时 `ready=false` 会渲染成**「未初始化精修项目」** ——
      // 那是"读不到"，不是"没初始化"：用户会去点「初始化」把一个已存在的工程重做一遍。
      // 所以记下原因，模板优先显示错误条（见 `cfgError`）。
      cfgError.value = e.message || String(e)
      if (!silent) ready.value = false
      else toast('刷新失败：' + e.message, 'err')
    }
    if (dir.value) {
      try {
        subs.value = (await fetchSubtitles(pid.value, dir.value, '读取字幕')).segments || []
      } catch { if (!silent) subs.value = [] }
      subs.value.sort((a, b) => (Number(a.start) || 0) - (Number(b.start) || 0))
      try {
        const r = await fetch(clipMediaUrl(pid.value, dir.value, 'data/danmaku.json'))
        if (r.ok) dm.value = await r.json()
      } catch (e) {
        // 弹幕文件缺失是常态（老工程没有 data/danmaku.json）：只记 debug，不刷屏
        addClient('debug', `读取弹幕失败（可能没有弹幕文件）：${e.message}`, 'polish')
        if (!silent) dm.value = []
      }
    }
    await refreshClipEntry()
  } finally {
    if (!silent) loading.value = false
  }
}

function thumbSrc(c) {
  const rank = String(c.name || '').replace('clip-', '')
  return `/api/project-data/clips/${encodeURIComponent(rank)}/thumb?${routeCfg}`
}

function clipDirOf(name) {
  const hit = clips.value.find((c) => c.name === name)
  return hit?.dir || name
}

/**
 * 片段排序键：优先 `clip.json` 的 `rank`（新片段目录名是标题 slug，不能再解析 `clip-NN`），
 * 旧数据没有 rank 时才回退解析 `clip-NN`，都没有就排到最后。
 */
function rankOfClip(c) {
  const rank = Number(c?.meta?.rank)
  if (Number.isFinite(rank) && rank > 0) return rank
  const name = String(c?.name || '')
  if (!name.startsWith('clip-')) return Number.MAX_SAFE_INTEGER
  const n = Number(name.slice('clip-'.length))
  return Number.isFinite(n) ? n : Number.MAX_SAFE_INTEGER
}

/** 片段列表 + 每片段的精修状态。loading 由调用方（load / 任务回调）自行管理，这里不动。 */
async function loadClips() {
  // ⚠️ 以前这里 `apiOr(…, [])` + `catch { clips = [] }`：读失败退化成空数组，界面上就是
  // "还没有可精修的切片" —— 和"真的一个切片都没有"长得一模一样，用户会以为切片没了。
  // 现在失败**保留上一次的列表**（别清空）+ 置错误条，空态卡也靠 clipsError 排除掉这种假空。
  try {
    if (!routeCfg) { clips.value = []; clipsError.value = ''; return }
    const [list, polish] = await Promise.all([
      api(`/api/project-data/clips?${routeCfg}`, { label: '读取切片列表' }),
      api(`/api/project-data/polish?${routeCfg}`, { label: '读取精修状态' })
    ])
    clips.value = (Array.isArray(list) ? list : [])
      .filter((c) => c.video_exists)
      .sort((a, b) => rankOfClip(a) - rankOfClip(b))
    const m = {}
    ;(polish || []).forEach((p) => { m[String(p.dir || '').replace(/\\/g, '/').toLowerCase()] = p })
    polishByDir.value = m
    clipsError.value = ''
  } catch (e) {
    clipsError.value = e.message || String(e)
  }
}

/** 片段列表行：`N · 标题` + 状态 chip（未初始化 / 精修中 / 已导出）。
 *  ⚠ 与切片页卡片上那枚 chip 同一套用词（那边是 `isExported`）：**别在这里叫「已精修」** ——
 *  「已精修 / 精修中」并列正是用户报过的那种歧义，现在全应用统一成「已导出」。
 *  判据也一致：出过片**或**出过封面都算。 */
function clipRow(c) {
  const dir = String(c.dir || '').replace(/\\/g, '/').toLowerCase()
  const p = polishByDir.value[dir] || null
  const initialized = !!p?.polish_ready
  const state = !initialized ? '未初始化' : (p.rendered || p.cover ? '已导出' : '精修中')
  return {
    name: c.name,
    title: c.meta?.title || c.name,
    state,
    chip: state === '已导出' ? 'g' : state === '精修中' ? 'p' : 'n',
    initialized
  }
}

async function initClip(c) {
  // 绝对目录（服务端 scope 守卫校验归属），不要自己算相对前缀
  startJob('init', () => initPolish(pid.value, c.dir), { after: loadClips })
  toast('正在初始化 ' + c.name)
}

const fragRows = computed(() => clips.value.map((c) => ({ name: c.name, thumb: thumbSrc(c), ...clipRow(c) })))

function initClipByName(name) {
  const c = clips.value.find((x) => x.name === name)
  if (c) initClip(c)
}

function gotoClip(name) {
  router.push({ name: 'project-polish-clip', params: { pid: pid.value, clip: name } })
}

// ---- 时间轴编辑 → 预览 → 落盘 → 后处理 ----

/**
 * 本地改动**立刻**镜像进预览的输入（cfg.segments / subs / cfg.mute_ranges），
 * **不 bump previewKey、不重挂载** —— 视频保持播放位置，拖动时画面当场就变。
 */
function onTimelinePreview(patch) {
  if (patch.segments && cfg.value) cfg.value = { ...cfg.value, segments: patch.segments }
  if (patch.subtitles) subs.value = patch.subtitles
  if (patch.mute_ranges && cfg.value) cfg.value = { ...cfg.value, mute_ranges: patch.mute_ranges }
}

let processTimer = null
let processPendingDir = ''
let processPendingReason = ''
/** 页面已卸载（离开精修页）：之后发起的后处理只发任务，不再往这个组件上挂轮询/刷新 */
let pageGone = false

/**
 * 时间轴「编辑完成」：跑一次后处理（贴纸落点 / 运镜锚点坐标 / 画中画守卫都是它算出来的）。
 *
 * **编辑过程中一律不跑** —— 编辑器只在"一次操作真的做完"时发 `done`（面板控件确定 /
 * 取消选中 / 切片段 / 离开页面）。以前是"落盘成功后 1.2s 自动跑"，于是你刚点「＋」加完贴纸、
 * 还在面板里填 keyword 的时候它就插进来了：半成品被当成成品处理，数量抽稀把新贴纸删掉，
 * 用户看到的是"我自己加的东西过两秒就没了"。
 */
function onEditDone(payload) {
  const target = (payload && payload.dir) || dir.value
  if (!target) return
  processPendingDir = target
  processPendingReason = (payload && payload.reason) || ''
  if (processTimer) clearTimeout(processTimer)
  // 合并防抖：连续几次"完成"（改一个字段 → 取消选中 → 又点回去）只跑一次
  processTimer = setTimeout(() => {
    processTimer = null
    const t = processPendingDir
    const why = processPendingReason
    processPendingDir = ''
    processPendingReason = ''
    runProcessFor(t, why)
  }, 1500)
}

/**
 * 给某个目录跑一次后处理。
 * `target` 已经**不是当前片段**（切走了 / 离开了页面）时：只把任务发出去，不占用当前页面的
 * 任务状态、也不刷新当前页面 —— 服务端按目录排队，跑完就完了（页面都卸载了，再挂轮询只会漏）。
 */
async function runProcessFor(target, reason) {
  if (!target) return
  if (pageGone || target !== dir.value) {
    addClient('debug', `编辑完成（${reason || ''}）：为上一个片段补跑一次后处理`, 'polish')
    runProcess(pid.value, target).catch((e) => addClient('warn', '补跑后处理失败：' + e.message, 'polish'))
    return
  }
  // 本片段忙（自己的任务、或别处发起/排队的任务）：不塞队列，只记 pending，
  // 等作用域空下来自动补跑一次（改动都在 polish.toml 里，那次 process 一并算进去）。
  if (scopeBusy.value) {
    // 兜底：本地可能停在"运行中"而服务端早就跑完了（订阅恰好错过终态）。
    // 直接问一次服务端；真结束了就放行，别让自动后处理从此静默失效。
    const done = await jobActuallyDone()
    if (!done || scopeBusy.value) {
      markPendingProcess(`编辑完成时该片段正忙（${reason || ''}）`)
      return
    }
  }
  startJob('process', () => runProcess(pid.value, target), {
    silent: true,
    keepPreview: true,
    after: async () => {
      if (pageGone) return
      await load({ silent: true })
      // 后处理会改写事件（贴纸吸附/落点、锚点解析、画中画守卫）——把结果同步回编辑器
      await timelineRef.value?.reload?.()
    }
  })
}

/** 本地以为在跑，去服务端核实一下；已完成则把本地任务状态收敛掉并返回 true。 */
async function jobActuallyDone() {
  if (!job.id) return true
  try {
    const j = await jobsApi.detail(job.id)
    if (['succeeded', 'failed', 'cancelled'].includes(j.status)) {
      job.status = j.status
      closeJobWatcher()
      stopJobTimer()
      return true
    }
    return false
  } catch {
    return false // 问不到就按"忙"处理，宁可跳过
  }
}

/**
 * 关掉当前任务的订阅。
 * ⚠️ `watchJob()` 返回的是 `{ close() }` **对象**，不是函数 ——
 * 当函数调会抛 `jobClose is not a function`（压缩后就是「ve is not a function」），
 * 而且 `startJob` 里那处就在 `jobClose = watchJob(...)` 上一行：抛错后新任务永远订阅不上，
 * 界面从此停在「运行中」（自动后处理也会被 jobRunning 挡掉）。别再写回 `jobClose?.()`。
 */
function closeJobWatcher() {
  if (!jobClose) return
  jobClose.close()
  jobClose = null
}

/**
 * 待补跑的后处理：本片段忙的时候不往队列里塞 process（一次编辑风暴会塞进十几条），
 * 只记一个标记，等它空下来自动跑一次 —— 那次 process 会读磁盘上最新的 polish.toml，
 * 期间所有改动一并生效。
 *
 * 存的是 **dir**（不是片段名）：切走之后欠的那次仍要跑到**原来那个片段**上。
 */
const pendingProcess = ref('')

function markPendingProcess(reason) {
  pendingProcess.value = dir.value
  addClient('debug', `后处理推迟到本片段空闲后自动补跑（${reason}）`, 'polish')
}

/** 片段空闲时补跑一次被推迟的后处理（silent：不重挂预览、不丢播放位置） */
function flushPendingProcess() {
  const target = pendingProcess.value
  if (!target) return
  if (target === dir.value && (scopeBusy.value || !ready.value)) return
  pendingProcess.value = ''
  runProcessFor(target, '空闲补跑')
}

// 作用域一空下来就补跑（job 列表由 /api/stream + 3 秒轮询刷新）
watch(scopeBusy, (v) => { if (!v) flushPendingProcess() })

/** 「重新后处理」按钮 */
function runProcessJob() {
  if (scopeBusy.value) { toast('该片段正在' + scopeBusyText.value + '，请稍候', 'err'); return }
  startJob('process', () => runProcess(pid.value, dir.value), {
    // keepPreview：只重算派生数据（贴纸落点/锚点坐标），预览不重挂载、播放位置不丢
    keepPreview: true,
    after: async () => { await load({ silent: true }); await timelineRef.value?.reload?.() }
  })
}

/**
 * 时间轴自己的「重试」成功了：它读的就是页面那条 `/api/polish/project`，所以页面上那条
 * 「显示的是上一次的数据」此刻已经过期 —— 不清掉它就成了"界面在说谎"（明明好了还挂着报错）。
 * 用 silent 重拉一次，不卸载编辑器。
 */
function onTimelineRetried() {
  if (cfgError.value) load({ silent: true })
}

async function refreshClipEntry(force = false) {
  coverFailed.value = false
  renderedFailed.value = false
  const norm = (p) => String(p || '').replace(/\\/g, '/').toLowerCase()
  const key = norm(dir.value)
  // `loadClips()` 刚拉过整张 polish 表；正常情况下直接用缓存，只有封面保存等确实需要
  // 刷新产物状态时才 force 重拉，避免每次 load 都多打一次 `/api/project-data/polish`。
  if (!force && polishByDir.value[key]) {
    clipEntry.value = polishByDir.value[key]
    return
  }
  const list = routeCfg ? await apiOr(`/api/project-data/polish?${routeCfg}`, [], '刷新片段状态') : []
  clipEntry.value = (list || []).find((p) => norm(p.dir) === key) || null
}

// ---- 任务驱动 ----

function startJobTimer() {
  stopJobTimer()
  jobStartMs.value = Date.now()
  jobElapsed.value = 0
  jobTimer = setInterval(() => {
    if (!jobRunning.value) return
    jobElapsed.value = Math.floor((Date.now() - jobStartMs.value) / 1000)
  }, 1000)
}

function stopJobTimer() {
  if (jobTimer) {
    clearInterval(jobTimer)
    jobTimer = null
  }
  if (jobStartMs.value) {
    jobElapsed.value = Math.max(jobElapsed.value, Math.floor((Date.now() - jobStartMs.value) / 1000))
  }
}

async function cancelJob() {
  if (!job.id || !jobRunning.value) return
  canceling.value = true
  try {
    await api('/api/jobs/' + job.id + '/cancel', { method: 'POST' })
    // cancel 接口已同步把任务置为 cancelled；关掉订阅避免后台迟到的完成事件再把它改回成功
    closeJobWatcher()
    job.status = 'cancelled'
    stopJobTimer()
    toast('任务已中断')
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    canceling.value = false
  }
}

function startJob(kind, call, { after, silent, keepPreview } = {}) {
  if (scopeBusy.value) { toast('该片段正在' + scopeBusyText.value + '，请稍候', 'err'); return Promise.resolve(false) }
  return call().then((r) => {
    Object.assign(job, { id: r.id, kind, status: r.queued ? 'queued' : 'running', current: 0, total: 0, log: '', error: '' })
    // 服务端作用域冲突或本档名额满时是**排队**而不是报错：告诉用户轮到时自动开始，
    // 具体排在第几位看任务抽屉（档位/名次由服务端给，前端不猜）
    if (r.queued && !silent) toast(QUEUED_HINT)
    startJobTimer()
    // ⚠️ 这里以前写的是 `jobClose?.()`（把 `{close}` 对象当函数调）——它会抛错，
    // 导致下面这行订阅根本不执行、任务永远停在「运行中」。见 closeJobWatcher 的注释。
    closeJobWatcher()
    jobClose = watchJob(r.id, {
      onEvent(e) {
        if (e.type === 'log') job.log = e.message
        else if (e.type === 'progress') { job.current = e.current; job.total = e.total }
        else if (e.type === 'failed') job.error = e.error
      },
      onSummary(s) {
        job.status = s.status
        job.current = s.current
        job.total = s.total
      },
      onDone(s) {
        job.status = s.status
        stopJobTimer()
        // 全局任务列表最多滞后 3 秒：立刻拉一次，作用域门控与侧栏徽标才能马上松开
        reloadJobs()
        if (s.status === 'succeeded') {
          if (!silent) toast(jobLabel(kind) + '完成')
          // 重挂载预览会把播放位置打回 0：编辑触发的后处理（silent / keepPreview）不重挂，
          // 靠 load({silent:true}) 换掉 cfg 让预览自己重算索引——位置与滚动都不丢。
          if (!silent && !keepPreview) previewKey.value++
          after && after()
        } else if (s.status === 'failed') {
          toast(jobLabel(kind) + '失败：' + (job.error || s.message || '未知错误'), 'err')
          after && after()
        } else if (s.status === 'cancelled') {
          toast(jobLabel(kind) + '已取消')
          after && after()
        }
      }
    })
    return true
  }).catch((e) => { toast(e.message, 'err'); return false })
}

async function initProject() {
  const ok = await confirmDialog('将解析字幕/弹幕并创建 polish.toml', { title: '初始化精修项目？' })
  if (!ok) return
  startJob('init', () => initPolish(pid.value, dir.value), { after: () => { segKey.value++; load({ silent: true }) } })
}

async function runLlmJob() {
  // 「LLM 剪辑」= **全部重做**（用户要求）：清掉断点续传、重新分段、重新生成全部事件。
  // 会覆盖当前的段与事件（含手工加的），所以先确认；失败的段会在时间轴上标琥珀色，
  // 想只补某一段就用分段属性里的「只重做本段事件」。
  const ok = await confirmDialog(
    '将重新分段并重做全部事件：当前的段与事件（含手工添加的贴纸/运镜）都会被替换。\n'
    + '失败的分段会在时间轴上标出来，可以之后单独重做那一段。',
    { title: 'LLM 剪辑（全部重做）？' }
  )
  if (!ok) return
  startJob('llm', () => runLlm(pid.value, dir.value, true), { after: () => { segKey.value++; load({ silent: true }) } })
}

/** 只重做某一段的事件（时间轴 → 选中分段 → 属性面板里的按钮）。 */
function redoSegment(index) {
  if (index === undefined || index === null) return
  startJob('llm_segment', () => runLlmSegment(pid.value, dir.value, index), {
    after: () => { segKey.value++; load({ silent: true }) }
  })
}

/** 打开输出目录：浏览器无法打开本地目录，复制路径并提示。 */
async function copyOutDir() {
  const p = dir.value || ''
  if (!p) return
  try {
    await navigator.clipboard.writeText(p)
    toast('输出目录路径已复制')
  } catch {
    toast(p)
  }
}

function doRender() {
  renderModal.value = false
  // `output_file` 必须是**项目内相对路径**（服务端 jail 校验）；留空则用 polish.toml 里的 project.output
  // 渲染会重新生成封面文件 → 封面设计器重挂载拉新图
  startJob('render', () => runRender(pid.value, dir.value, renderOutput.value.trim()), { after: () => { coverKey.value++; load({ silent: true }) } })
}

// 时间轴的保存与后处理见上面「时间轴编辑 → 预览 → 落盘 → 后处理」（编辑器自己防抖落盘）

// 九宫格布局已由「布局框选」取代：区域矩形一律走 layout[region].top_left/bottom_right

// 添加字幕：开始 = 当前播放时间，结束 = +3s
// ---- 贴纸配置（sticker_source，直存 polish.toml）----
async function loadStickerSources() {
  // 同 loadClips：失败不能退化成"贴纸库为空"（那是个假空态，用户会去查库而不是查服务）
  try {
    stickerSources.value = (await fetchStickerSources(pid.value, dir.value, '读取贴纸来源')).sources || []
    stickerSourcesError.value = ''
  } catch (e) {
    stickerSourcesError.value = e.message || String(e)
  }
}

// ---- 字体库（字幕字体下拉）----
async function loadFonts() {
  try {
    const r = await fetchFonts('读取字体库')
    fonts.value = r.fonts || []
    fontsRoot.value = r.root || ''
    fontsError.value = ''
  } catch (e) {
    // 与贴纸来源同规矩：读不到就是读不到，别伪装成"库是空的"
    fontsError.value = e.message || String(e)
  }
}

/** 换字幕字体：只改 polish.toml 的一个字段，保存后重取配置让预览/时间轴跟着变。
 *  **不重挂载预览**（keepPreview 那套只在任务路径上有）—— cfg 变了 PolishPreview 就地重算。 */
async function onSubtitleFontChange(e) {
  const next = e.target.value
  const prev = subtitleFont.value
  if (next === prev) return
  fontSaving.value = true
  try {
    await saveProjectPatch(pid.value, dir.value, { subtitle_font: next })
    // 先让浏览器认得这个字体（DOM 预览靠它），再刷新配置
    if (next) await ensureFontFace(pid.value, dir.value, next, { onDebug: (m) => addClient('debug', m, 'polish') })
    cfg.value = { ...cfg.value, style: { ...(cfg.value?.style || {}), subtitle: { ...(cfg.value?.style?.subtitle || {}), font: next } } }
    await load({ silent: true })
    toast(next ? `字幕字体已改为「${next}」` : '字幕字体已回到系统默认')
  } catch (err) {
    toast(err.message, 'err')
    // 保存失败要让下拉回到原值（否则界面在撒谎）
    await load({ silent: true })
  } finally {
    fontSaving.value = false
  }
}

// /api/health 的 wgpu_render：非 true（含 undefined）都视为未启用
async function loadHealth() {
  try {
    const h = await api('/api/health')
    wgpuRender.value = h?.wgpu_render === true
  } catch { wgpuRender.value = false }
}

async function patchStickerSource(list) {
  try {
    await saveProjectPatch(pid.value, dir.value, { sticker_source: list })
    cfg.value = { ...cfg.value, sticker_source: list }
    toast('贴纸配置已保存')
  } catch (e) { toast(e.message, 'err') }
}

function toggleStickerSource(name) {
  if (stickerSource.value.includes(name)) patchStickerSource(stickerSource.value.filter((x) => x !== name))
  else patchStickerSource([...stickerSource.value, name])
}

// ---- 画面构成（[stage]：主画面位置 / 背景）----
// 值存 polish.toml；渲染侧由 Rust `ProjectFile::resolve_stage` 推导成 `_canvas.stage`，
// 这里**只做提交 + 重取**，不在前端重算 —— 否则又会多出一套推导，两边迟早不一致。
/** 右栏页签：导出（LLM 参数 + 渲染）/ 画面构成（尺寸 + 位置 + 背景），头部标签切换 */
const genTab = ref('export')
const stageBusy = ref(false)
const stageCfg = computed(() => cfg.value?.stage || {
  bg_mode: 'auto', bg_color: '#000000', blur_scale: 1, position: 'auto', scale: 1, padding: 0
})
const isPortraitCanvas = computed(() => (cfg.value?.canvas?.orientation || 'landscape') === 'portrait')
// 位置的方向不跨用（与 Rust `stage_position` 同规矩）：横画布只给 左/中/右，竖画布只给 上/中/下
const positionOptions = computed(() => (isPortraitCanvas.value
  ? [{ value: 'auto', label: '自动' }, { value: 'top', label: '靠上' }, { value: 'center', label: '居中' }, { value: 'bottom', label: '靠下' }]
  : [{ value: 'auto', label: '自动' }, { value: 'left', label: '靠左' }, { value: 'center', label: '居中' }, { value: 'right', label: '靠右' }]))
const bgOptions = [
  { value: 'auto', label: '自动' },
  { value: 'color', label: '纯色' },
  { value: 'blur', label: '模糊' }
]
/** 「纯黑」已并入纯色：bg_mode='black' 与 无背景 渲染结果一样（预览露出 #player 黑底；
 *  出片 canvas 路径上 none = 不画，透明落进 JPEG 也是黑），按钮组不再单列；
 *  存量 black 在界面上显示为 纯色 + #000000，选中纯色时才落成 color。 */
const stageBgDisplay = computed(() => (stageCfg.value.bg_mode === 'black' ? 'color' : (stageCfg.value.bg_mode || 'auto')))
const stageBgColor = computed(() => stageCfg.value.bg_color || '#000000')
const canvasSummary = computed(() => {
  const c = cfg.value?._canvas
  if (!c) return ''
  const s = c.src || {}
  return (c.orientation === 'portrait' ? '竖屏 1080×1920' : '横屏 1920×1080')
    + (s.width ? ` · 源 ${s.width}×${s.height}` : '')
})
/** 源与画布同比时位置没有可见效果；但背景层仍会生效（竖屏铺满 + 特效位移会露边）——
 *  直接说明，别让人白调半天，也别让人以为背景没接线。 */
const stageHint = computed(() => {
  const c = cfg.value?._canvas
  if (!c) return '保存后即可在预览里看到效果。'
  const s = c.src || {}
  if (!s.width || !s.height) return '拿不到源视频尺寸（文件可能缺失）→ 暂时按铺满处理。'
  const f = c.stage || {}
  const bgLabel = { none: '无背景', black: '纯黑', color: '纯色', blur: '当前帧模糊' }[f.bg] || ''
  const srcAr = s.width / s.height
  const canvasAr = c.width / c.height
  if (Math.abs(srcAr - canvasAr) / canvasAr <= 0.005) {
    return `源与画布同比（${s.width}×${s.height} → ${c.width}×${c.height}）：主画面直接铺满，位置不生效；`
      + `背景（${bgLabel}）只在特效把画面挪开时露出来。`
  }
  const posLabel = { left: '靠左', right: '靠右', center: '居中', top: '靠上', bottom: '靠下' }[f.position] || '居中'
  return srcAr < canvasAr
    ? `源比画布更窄 → 主画面${posLabel}，左右留边（${bgLabel}）。`
    : `源比画布更扁 → 主画面${posLabel}，上下留边（${bgLabel}）。`
})

/** 保存 [stage]：与「字幕字体」同一套 —— 先乐观改本地配置（控件当场响应），再静默重取拿到新的 `_canvas` */
async function patchStage(patch) {
  if (!ready.value || stageBusy.value) return
  const next = { ...stageCfg.value, ...patch }
  stageBusy.value = true
  try {
    await saveProjectPatch(pid.value, dir.value, { stage: next })
    cfg.value = { ...cfg.value, stage: next }
    await load({ silent: true })
  } catch (e) {
    toast('画面构成保存失败：' + e.message, 'err')
    await load({ silent: true })
  } finally {
    stageBusy.value = false
  }
}

// ---- 画面尺寸（[canvas]：orientation 横屏/竖屏）----
// 与 [stage] 同一套提交链。切方向后 Rust `stage_position` 会把方向不合法的 position
// 净化回 auto（横=左中右 / 竖=上中下），前端的 positionOptions 也按方向换选项。
// P3 已落地：重取配置拿到新 `_canvas`（1080×1920 / 1920×1080）后，
// 预览舞台（PolishPreview 的 --stage-ar）、出片窗口（Chrome --window-size）
// 与模板/引擎的全部几何都会随画布参数化生效。
const canvasCfg = computed(() => cfg.value?.canvas || { orientation: 'landscape' })
const orientationOptions = [
  { value: 'landscape', label: '横屏 1920×1080' },
  { value: 'portrait', label: '竖屏 1080×1920' }
]
async function patchCanvas(patch) {
  if (!ready.value || stageBusy.value) return
  const next = { ...canvasCfg.value, ...patch }
  stageBusy.value = true
  try {
    await saveProjectPatch(pid.value, dir.value, { canvas: next })
    cfg.value = { ...cfg.value, canvas: next }
    await load({ silent: true })
  } catch (e) {
    toast('画面尺寸保存失败：' + e.message, 'err')
    await load({ silent: true })
  } finally {
    stageBusy.value = false
  }
}

// ---- 贴纸落点（[sticker]：auto 安全区算法 / manual 固定一点）----
// 与 [stage] 同一套：只提交 + 重取，落点换算全在 Rust（`_canvas.sticker`）。
// 渲染侧在**出片时**分支（模板 `stickerAnchor` / engine.js `stickerAnchor`），
// 所以切模式不用重跑 process —— events 里的逐条 x/y 一直保留着。
const stickerBusy = ref(false)
const stickerCfg = computed(() => cfg.value?.sticker || { position_mode: 'auto', position: '50% 50%' })

/** `"50% 50%"` → `{x:50,y:50}`；解析不出来回落 50（与渲染侧同一个兜底） */
function parsePos(s) {
  const p = String(s || '50% 50%').trim().split(/\s+/)
  const a = parseFloat(p[0])
  const b = parseFloat(p.length > 1 ? p[1] : p[0])
  return { x: Number.isFinite(a) ? a : 50, y: Number.isFinite(b) ? b : 50 }
}
/** `{x,y}` → `"53% 48%"`（落盘格式：整数百分比，与九宫格时代一致） */
const fmtPos = (p) => `${p.x}% ${p.y}%`

/** 已保存的落点（贴纸盒子中心，整帧百分比） */
const stickerPos = computed(() => parsePos(stickerCfg.value.position))
const stickerPosPct = computed(() => fmtPos(stickerPos.value))

/**
 * 点选落点 = **草稿态**：进入编辑 → 在预览上点 → 保存 / 取消。
 *
 * 以前是「固定模式即常驻点选，点哪儿存哪儿」—— 于是"只是想看一眼预览"就改了配置，
 * 用户要求补保存/取消。现在与区域框选同一套路（见 `applySelect`）：
 * 编辑态里的点选只改 `draftPos`，只有「保存落点」才落盘，「取消」直接还原。
 * 注意 `draftPos` 是**贴纸盒子中心**，与已保存的 `[sticker] position` 同一基准
 * （auto 的事件 x/y 才是盒子左上角 —— 两个键两种基准，不要"统一"）。
 */
const pointEdit = ref(false)
const draftPos = ref({ x: 50, y: 50 })
const draftPosPct = computed(() => fmtPos(draftPos.value))

/**
 * 预览点选层是否生效：只在编辑态（且不在框选态）显示。
 * 与区域框选互斥 —— 两者都是"在预览上取坐标"，框选优先，避免两层抢指针。
 */
const pointMode = computed(() => pointEdit.value && !selectRegion.value)

/** 进入点选编辑态：草稿以已保存值为初值（取消即等于没动过）。 */
function startPointEdit() {
  if (stickerBusy.value || !ready.value) return
  if (selectRegion.value) selectRegion.value = ''   // 与框选互斥
  draftPos.value = { ...stickerPos.value }
  pointEdit.value = true
  // 定格当前帧，避免边播边点选
  previewRef.value?.pause?.()
}

/** 取消点选：丢弃草稿，回到已保存值。 */
function cancelPointEdit() {
  pointEdit.value = false
  draftPos.value = { ...stickerPos.value }
}

/** 保存落点：写 `[sticker] position`（不重跑 process —— 渲染侧出片时才读它）。 */
async function savePointEdit() {
  const p = draftPos.value
  if (await patchSticker({ position: fmtPos(p) })) cancelPointEdit()
}

/**
 * 贴纸落点这一格：自动 ⇄ 固定。
 *
 * 「固定」= 全部贴纸落在同一点，用户要求**像区域编辑那样直接在预览里点**。
 * 切到固定 = 本来就是要去选一个点，所以顺手进编辑态（省一次点击）；
 * 切回自动则丢弃草稿。
 */
async function toggleStickerMode() {
  if (stickerBusy.value || !ready.value) return
  if (stickerCfg.value.position_mode === 'manual') {
    cancelPointEdit()
    await patchSticker({ position_mode: 'auto' })
    return
  }
  if (await patchSticker({ position_mode: 'manual' })) startPointEdit()
}

/** 预览点选落点 → 只改草稿（保存/取消在右侧「区域与落点」里）。 */
function onPointPick(p) {
  const x = Math.round(Math.min(100, Math.max(0, Number(p?.x) || 0)))
  const y = Math.round(Math.min(100, Math.max(0, Number(p?.y) || 0)))
  draftPos.value = { x, y }
}

/** @returns {Promise<boolean>} 是否保存成功（失败时不丢草稿，见 `savePointEdit`） */
async function patchSticker(patch) {
  if (!ready.value || stickerBusy.value) return false
  const next = { ...stickerCfg.value, ...patch }
  stickerBusy.value = true
  try {
    await saveProjectPatch(pid.value, dir.value, { sticker: next })
    cfg.value = { ...cfg.value, sticker: next }
    await load({ silent: true })
    return true
  } catch (e) {
    toast('贴纸落点保存失败：' + e.message, 'err')
    await load({ silent: true })
    return false
  } finally {
    stickerBusy.value = false
  }
}

// 切片段时退出编辑态：草稿属于上一个片段，不能跨片段保存
watch(() => loadedClip.value, () => cancelPointEdit())

function onCoverError() {
  coverFailed.value = true
}
function onRenderedError() {
  renderedFailed.value = true
}

onMounted(() => {
  load()
  loadStickerSources()
  loadFonts()
  loadHealth()
})
onUnmounted(() => {
  pageGone = true
  stopJobTimer()
  closeJobWatcher()
})
</script>

<style scoped>
/* 页面专属布局（样稿无同名类）：编辑区（预览左 / 生成与导出右栏）、独占一行的时间轴、
   LLM 剪辑参数块、紧凑片段条、贴纸来源选择器、产物体 */
/* 两栏各取自然高度（右栏不跟着预览拉长）；时间轴在下一行整行铺开 —— 它的横向空间
   不再被右栏吃掉（刻度/泳道多出约 300px），编辑手感与刻度精度都更好 */
.edit-grid { align-items:start; }
/* 预览卡内部：属性面板（时间轴 Teleport 过来的）在左，预览舞台在右 */
.preview-row { display:flex; gap:12px; align-items:stretch; margin-top:8px; }
/* 挂载点定宽 + relative；面板绝对定位撑满它 —— 属性表单再长也只在面板内部滚动，
   不会把整行/预览卡撑高（否则时间轴会被顶出首屏） */
.props-host { position:relative; flex:none; width:232px; min-width:0; }
.props-host :deep(.props-panel) { position:absolute; inset:0; width:auto; }
.stage-col { flex:1; min-width:0; }
.frag-strip { padding:8px 10px; }
.frag-strip-row { display:flex; gap:8px; align-items:flex-start; }
.frag-strip-row :deep(.frag-cards) { flex:1; min-width:0; }
.frag-more { flex:none; }
/* 紧凑页头：一行放完 chip + 标题（原设计是 26px 标题 + sub 两行，这里为竖高让路） */
.compact-head { align-items:center; margin-bottom:8px; }
.compact-head h1 { font-size:19px; }
/* 时间轴卡：自身 padding 收窄（内容已经很紧凑，竖高按 26px/泳道算过） */
.tl-card { padding:12px; }
.gen-block { border:1px solid var(--line); border-radius:var(--r-sm); padding:10px 12px; background:var(--bg-soft); }
/* 贴纸来源：一排可点 chip。亮 = 启用，灰 = 停用（不要再出现第二份已选列表）。 */
.ss-options { display:flex; flex-wrap:wrap; gap:6px; }
.ss-opt { display:inline-flex; align-items:center; gap:5px; padding:4px 10px; border-radius:var(--r-full);
  border:1px solid var(--line); background:var(--bg-soft); color:var(--ink-faint);
  font-size:12px; cursor:pointer; transition:.15s; opacity:.7; }
.ss-opt:hover { border-color:var(--main); color:var(--pink-text); opacity:1; }
.ss-opt.on { background:var(--main-soft); border-color:var(--main); color:var(--main-deep);
  font-weight:600; opacity:1; }
.ss-opt-meta { font-size:10px; opacity:.75; }
.job-box { border:1px solid var(--line); border-radius:var(--r-sm); padding:8px 10px; background:var(--bg-soft); }
/* 成片与封面统一为同尺寸 16:9 盒子（源都是 1920×1080，比例异常时裁切对齐） */
.artifact-bar .cover-thumb,
.artifact-bar .video-bar {
  width: 320px; aspect-ratio: 16/9; object-fit: cover; object-position: center;
  max-height: none; margin-top: 0; flex: none;
  border-radius: var(--r-sm); border: 1px solid var(--line); background: #000; display: block;
}
.cover-img { width:100%; border-radius:var(--r-sm); border:1px solid var(--line); display:block; }
.thumb { width:100%; aspect-ratio:16/9; object-fit:cover; border-radius:var(--r-sm); margin-top:10px; border:1px solid var(--line); }
</style>
