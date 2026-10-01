<template>
  <div class="card" id="cover-design">
    <div class="row spread">
      <h2 class="card-h2 m-0"><Icon name="image" :size="16" /> 封面设计器</h2>
      <button class="btn btn-primary" :disabled="generating || !!clipBusyJob"
              :title="clipBusyJob ? '该片段正在' + kindText(clipBusyJob.kind) : ''"
              @click="generateCover()">{{ generating ? '生成中…' : '生成封面' }}</button>
    </div>
    <!-- 配置没读回来 → `configLoaded` 这道写入门闸继续关着，这个面板之后**不会**再保存任何改动。
         必须摆到最显眼处并给重试：否则就是"改了十分钟、关掉再打开全没了"，还没有一句解释。 -->
    <div v-if="configLoadError" class="error-banner">
      <Icon name="alert" :size="15" />
      <span>封面配置读取失败，<b>改动不会被保存</b>：{{ configLoadError }}</span>
      <button class="mini-btn" style="margin-left:auto" @click="reloadCoverConfig">重试</button>
    </div>
    <div class="designer-grid">
      <div class="stage-wrap" ref="stageWrapRef">
        <div ref="stageRef" class="stage" :class="'grab-' + dragTarget">
          <!-- ⚠️ 每一块的 **z-index 与显隐都来自图层栈**（`layerStyle()` → `blockZ`/`layerVisible`），
               与出片侧 `cover.rs::plan_cover_layers` 是同一套规则（filter 一层 = 暗角+渐变两块相邻，
               特效各占一个槽位）。**别再把 z-index 写回 CSS** —— 那是"界面说的和出片做的不一样"的入口。 -->
          <div class="stage-bg" :style="layerStyle('bg', { background: bgColor })">
            <img v-if="(bgMode === 'blur' || bgMode === 'image') && bgImageOk" class="stage-bg-img"
                 :class="{ 'bg-blur': bgMode === 'blur' }" :src="bgImageSrc"
                 :style="bgMode === 'blur' ? bgBlurStyle : null" alt="" @error="onBgImageError">
          </div>
          <!-- 帧层 = 「视频预览」层本体，**常驻的播放器**（2026-10 用户口径修正）。
               ⚠️ **不再用"选中该层才渲染 video"的门控** —— 那是两个真 bug 的来源：
               ① 选中别的层就没法拖进度条找帧；② 刚选好的帧一切层就跳回静态图（user 报的
               「选了其他图层立马又给我改回去」）。播放器是**全局取景 UI**，与"当前选中哪个层"无关；
               层选中只决定右侧属性面板显示什么。
               同帧契约没变、也不靠藏起播放器来守：出图链路是 ffmpeg 按 `frame_pos × 时长` 截帧
               写 `_cover_frame.jpg` → 塞进 `cover.html` → 出图程序渲染；**前端从来不是帧源**，
               它只负责让用户**选时间点**。故：① 打开时 `seekToFramePos()` 把播放头对到
               `frame_pos`（与出片同一帧）；② 「生成封面/导出关键帧」按当前时间重截并写回
               `frame_pos`；③ 拖动/播放本身**不写盘**。
               `_cover_frame.jpg` 的 <img> 仍是兜底：视频取不到（文件缺/编码坏）时顶上，
               与出片模板 `#frame > img` 逐字同源。 -->
          <div class="stage-media" :style="layerStyle('frame')">
            <video ref="videoRef" class="stage-video-live" :style="mediaStyle" :src="videoSrc"
                   @timeupdate="onVideoTime" @loadedmetadata="onMeta" @error="onVideoError"
                   @play="playing = true" @pause="playing = false"></video>
            <img v-if="!videoOk && frameImageOk" class="stage-frame-img" :style="mediaStyle" :src="frameImageSrc" alt="" @error="onFrameImageError">
          </div>
          <!-- 每个特效 = 一个**槽位**（槽位只管 z-index，内层才是效果本身）：必须与出片那份 HTML 同构，
               否则 overlay 的混合 / blur 的 backdrop 会与成片对不上（`isolation` 只给 overlay 那一种）。 -->
          <div class="fx-layer">
            <div v-for="fx in fxList" :key="fx.uid" class="fx-slot" :class="{ 'fx-slot-iso': fx.type === 'overlay' }"
                 :style="layerStyle('fx:' + fx.uid)">
              <div class="fx-item" :class="'fx-' + fx.type" :style="fxStyle(fx)">{{ fx.type === 'emoji' ? (fx.text || '🙂') : '' }}</div>
            </div>
          </div>
          <div class="gradient-layer" :style="layerStyle('overlay', gradientStyle)"></div>
          <div class="vignette-layer" :style="layerStyle('vignette', { opacity: vignette })"></div>
          <div class="title-wrap" :style="layerStyle('title', { left: textX + '%', top: textY + '%', width: textWidth + '%' })">
            <div ref="titleTextRef" class="title-text" :style="titleStyle" v-html="titleHtml"></div>
          </div>
          <div class="stage-gesture-layer" :title="dragHint"
               @pointerdown="onStagePointerDown" @wheel.prevent="onStageWheel"></div>
          <!-- 安全区线框（纯辅助线，只画在设计器里，**不在 cover.html 出片模板里**）：
               封面画布恒 1920×1080 横版，所以两个框都按横版算：
               16:9 框 = 四周等比内缩 5%（1824×1026），4:3 框 = 满高居中（1440×1080）。
               压在手势层之上（z7）才看得出标题有没有越线；pointer-events:none 所以不挡拖动。 -->
          <div v-if="showGuide169 || showGuide43" class="guide-layer">
            <div v-if="showGuide169" class="guide-box guide-169"><span class="guide-tag">16:9 安全区</span></div>
            <div v-if="showGuide43" class="guide-box guide-43"><span class="guide-tag">4:3 裁切区</span></div>
          </div>
        </div>
        <!-- 安全区的显隐搬进右侧图层列表（组「安全区」里的两行，各带一只眼睛）——
             这里不再放一份 chip，同一件事不说两遍。 -->
        <!-- 取景搬运工（**全局**，与"当前选中哪个层"无关 —— 选文字/特效时照样能拖进度条找帧）：
             播放/暂停 + 进度条 + 时间。落盘只在「导出关键帧 / 生成封面」发生，拖动本身不写盘。 -->
        <div class="seek-row mt-4">
          <button class="mini-btn" @click="togglePlay">{{ playing ? '暂停' : '播放' }}</button>
          <input class="flex-1 seek-range" type="range" min="0" :max="duration || 1" step="0.1" :value="time" @input="onSeekInput">
          <span class="muted small mono seek-time">{{ fmtTime(time) }} / {{ fmtTime(duration) }}</span>
        </div>
      </div>
      <!-- 右侧 = **图层属性面板（上）+ 图层组列表（下）**（PS 口径；用户要求把设置栏与图层栏换位）。
           三块各自滚动，整列高度由 `syncControlsBound()` 限到与左侧预览等高。
           `tabindex` 是为了 Ctrl/Cmd+[ / ] 那套图层快捷键（绑在面板上，别挂 window：精修页有别的全局键）。 -->
      <div class="controls" ref="controlsRef" tabindex="0" @keydown="onPanelKey">
        <div class="lyr-head">
          <span class="lyr-title">图层</span>
          <span class="lyr-sub">上 = 更靠前</span>
          <span v-if="saveError" class="save-line err" :title="saveError">保存失败</span>
          <span v-else-if="savingConfig" class="save-line">保存中…</span>
        </div>

        <!-- 属性面板：选谁改谁（安全区只给显隐）。
             ⚠️ 类名必须叫 `.lyr-props`：`styles/app.css` 有一条**全局** `.pane { display:none }`
             （设置页那几个 tab 面板靠 `.on` 才显示），叫 `.pane` 会让整块属性面板直接隐身
             —— 与 `.stage-video` 那两条黑边同源（全局规则穿透组件）。 -->
        <div class="lyr-props">
          <div v-if="!sel" class="lyr-props-note">点上面任意一层，这里显示它的设置</div>

          <template v-else-if="sel.kind === 'guide'">
            <div class="lyr-props-h">{{ sel.name }}</div>
            <div class="lyr-props-note">只画在设计器里，不进成片。只能隐藏，不能删除或改尺寸。</div>
            <div class="mini-row mt-4">
              <button class="mini-btn" :class="{ selected: sel.visible }" @click="toggleGuide(sel.guide)">
                {{ sel.visible ? '隐藏线框' : '显示线框' }}
              </button>
            </div>
          </template>

          <template v-else-if="sel.kind === 'video'">
            <div class="lyr-props-h">视频预览</div>
            <div class="lyr-props-note">基础层：封面用的那一帧。缩放 / 偏移都作用在这一层（不可删除）。</div>
            <div class="mini-row mt-4">
              <span class="mini-label">时间</span>
              <input v-model.number="time" type="number" step="0.1" class="w-time" title="填秒数回车，画面跳到那一帧" @change="seekToFramePos">
              <button class="mini-btn" @click="takeCurrent">取当前帧</button>
              <button class="mini-btn" @click="exportFrame">导出关键帧</button>
            </div>
            <div class="mini-grid mt-4">
              <div class="mini-field"><label>帧缩放</label><input type="range" min="0.5" max="3" step="0.05" v-model.number="coverZoom"><span class="rv">{{ Math.round(coverZoom * 100) }}%</span></div>
              <div class="mini-field"><button class="mini-btn" style="width:100%;justify-content:center" @click="resetFrame">复位</button></div>
            </div>
            <div class="mini-grid mt-4">
              <div class="mini-field"><label>偏移</label><span class="rv">X {{ Math.round(coverOffsetX) }} · Y {{ Math.round(coverOffsetY) }}</span></div>
              <div class="drag-hint" title="拖动预览 = 移动这一层 · 滚轮 = 缩放">拖预览移动 · 滚轮缩放</div>
            </div>
            <div class="lyr-props-note mt-4" title="出片不再删除这张图，下次出片与设计器看到的还是同一帧">「导出关键帧」把它固化成出片用的那张图。</div>
          </template>

          <template v-else-if="sel.kind === 'bg'">
            <div class="lyr-props-h">背景</div>
            <div class="lyr-props-note">垫在视频预览之下：帧缩小或拖开时才看得到。</div>
            <div class="mini-row mt-4">
              <button v-for="m in BG_MODES" :key="m.key" class="mini-btn seg-btn"
                      :class="{ selected: bgMode === m.key }" :title="m.hint"
                      @click="bgMode = m.key">{{ m.label }}</button>
            </div>
            <div v-if="bgMode === 'color'" class="mini-grid mt-4">
              <div class="mini-field"><label>颜色</label><input v-model="bgColor" type="color">
                <input v-model="bgColorText" class="hex-input" maxlength="7" spellcheck="false" placeholder="#000000"
                       title="直接填 16 进制码（#1a2b3c 或 #abc），回车/失焦生效"
                       @change="commitBgColorText" @keyup.enter="commitBgColorText" @blur="commitBgColorText">
              </div>
            </div>
            <div v-else-if="bgMode === 'blur'" class="mini-grid mt-4">
              <div class="mini-field"><label>模糊倍率</label><input type="range" min="0.2" max="4" step="0.1" v-model.number="blurScale"><span class="rv">×{{ Number(blurScale).toFixed(1) }}</span></div>
            </div>
            <div v-else class="mini-row mt-4">
              <button class="mini-btn" @click="fileInput.click()">导入背景图片</button>
              <span class="muted small">{{ bgImageOk ? '已导入 _cover_bg.jpg' : '还没导入（先点左边选一张）' }}</span>
            </div>
            <input ref="fileInput" type="file" accept="image/*" class="hidden-file" @change="importBg">
            <div v-if="bgMode === 'blur' && bgFrameTried && !bgImageOk" class="muted small mt-4">
              背景截帧没生成出来（出片会自己截，这里先用手动「导出关键帧」补一张）
            </div>
            <div class="lyr-props-note mt-4" title="导入的图写在 _cover_bg.jpg，不会覆盖视频预览那一层">导入的背景图不会覆盖视频预览层。</div>
          </template>

          <template v-else-if="sel.kind === 'filter'">
            <div class="lyr-props-h">滤镜</div>
            <div class="lyr-props-note">渐变 + 暗角，压在视频预览之上、文字与特效之下。</div>
            <div class="mini-grid mt-4">
              <div class="mini-field"><label>渐变</label>
                <select v-model="gradientDir" title="渐变底色取自「设置 · 字幕」的高亮色，这里只改方向">
                  <option value="left">左</option>
                  <option value="right">右</option>
                  <option value="top">上</option>
                  <option value="bottom">下</option>
                  <option value="radial">径向</option>
                  <option value="none">无</option>
                </select>
              </div>
              <div class="mini-field"><label>暗角</label><input type="range" min="0" max="1" step="0.02" v-model.number="vignette"><span class="rv">{{ Number(vignette).toFixed(2) }}</span></div>
            </div>
            <div class="lyr-props-note mt-4" title="渐变选「无」且暗角为 0 时，这一层就等于没有">两项都关掉，这一层就等于没有。</div>
          </template>

          <template v-else-if="sel.kind === 'text'">
            <div class="lyr-props-h">文字</div>
            <!-- 分三区（用户样稿）：**位置大小 / 排版 / 高亮 tag**；每个数值都是「滑块 + 行尾可填数字」
                 （滑块有 step 栅格，手填才落得到栅格外）。 -->
            <div class="lyr-sec">位置大小</div>
            <div class="txt-top mt-4">
              <!-- 标题文本框：固定 5 行高、不可拖拽缩放（内容超出在框内滚动）；字段样式用设计基准 .fld-input。
                   ⚠️ 标题**不做自动换行**（渲染侧 `white-space:nowrap` + 预览 `pre-line`+`max-content`）：
                   每行就是你敲的一行，长了就自己手动断行（这也正是「手动断行」这个字段的本意）。 -->
              <textarea v-model="titleLinesText" class="fld-input title-lines" rows="5" placeholder="封面标题，每行一段"></textarea>
              <div class="txt-side">
                <button class="mini-btn" :disabled="generatingTitle" title="只用 clip.srt + polish.toml 逐个事件生成" @click="generateTitleByLlm">{{ generatingTitle ? '生成中…' : 'LLM 生成标题' }}</button>
                <div class="mini-field"><label>X</label><input type="range" min="0" max="100" step="1" v-model.number="textX"><input class="rv-input" type="number" min="0" max="100" step="1" :value="textX" title="可直接填写（0 ~ 100）" @change="numBind(textX, (v) => (textX = v), $event, 0, 100)"></div>
                <div class="mini-field"><label>Y</label><input type="range" min="0" max="100" step="1" v-model.number="textY"><input class="rv-input" type="number" min="0" max="100" step="1" :value="textY" title="可直接填写（0 ~ 100）" @change="numBind(textY, (v) => (textY = v), $event, 0, 100)"></div>
                <!-- 封面字体：字体库（程序目录 resources/fonts/）里的字体。空 = 系统字体栈。 -->
                <div class="mini-field"><label>字体</label>
                  <select :value="coverFont" :disabled="!configLoaded" @change="onCoverFontChange">
                    <option v-for="o in fontOptions" :key="o.value" :value="o.value">{{ o.label }}</option>
                  </select>
                </div>
                <div class="mini-field"><label>字号</label><input type="range" min="24" max="240" step="2" v-model.number="fontSize"><input class="rv-input" type="number" min="24" max="240" step="2" :value="fontSize" title="可直接填写（24 ~ 240）" @change="numBind(fontSize, (v) => (fontSize = v), $event, 24, 240)"></div>
              </div>
            </div>
            <!-- 首次拿到标题时自动排一次版并放进 4:3 安全区；放不下就在这儿说出来（不静默） -->
            <div v-if="titleFitHint" class="muted small mt-4 fit-hint">{{ titleFitHint }}</div>

            <div class="lyr-sec">排版</div>
            <div class="txt-grid mt-4">
              <div class="mini-field"><label>对齐</label>
                <select v-model="align">
                  <option value="left">左</option>
                  <option value="center">中</option>
                  <option value="right">右</option>
                </select>
              </div>
              <div class="mini-field"><label>行距</label><input type="range" min="0.8" max="2.6" step="0.05" v-model.number="lineHeight"><input class="rv-input" type="number" min="0.8" max="2.6" step="0.05" :value="lineHeight" title="可直接填写（0.8 ~ 2.6）" @change="numBind(lineHeight, (v) => (lineHeight = v), $event, 0.8, 2.6)"></div>
              <div class="mini-field"><label>描边</label><input type="range" min="0" max="60" step="1" v-model.number="stroke"><input class="rv-input" type="number" min="0" max="60" step="1" :value="stroke" title="可直接填写（0 ~ 60）" @change="numBind(stroke, (v) => (stroke = v), $event, 0, 60)"></div>
              <div class="mini-field"><label>字距</label><input type="range" min="-10" max="60" step="1" v-model.number="letterSpacing"><input class="rv-input" type="number" min="-10" max="60" step="1" :value="letterSpacing" title="可直接填写（-10 ~ 60）" @change="numBind(letterSpacing, (v) => (letterSpacing = v), $event, -10, 60)"></div>
            </div>
            <label class="mini-field check mt-4"><input v-model="textShadow" type="checkbox"><span>文字阴影 {{ textShadow ? '开' : '关' }}</span></label>

            <!-- 高亮 tag：高亮色 与 关键词输入**同一行各占一半**（用户要求） -->
            <div class="lyr-sec">高亮 tag</div>
            <div class="txt-grid mt-4">
              <div class="mini-field"><label>高亮色</label><input v-model="hlColor" type="color"></div>
              <div class="mini-row">
                <input v-model="highlightInput" class="kw-input" placeholder="关键词，回车添加" @keyup.enter="addHighlightKeyword">
                <button class="mini-btn" @click="addHighlightKeyword"><Icon name="plus" :size="11" /> 添加</button>
              </div>
            </div>
            <!-- 高亮字号：只作用于高亮词那一段（不改字体、不改整块标题的字号）。
                 落到 `--hl-fs`，出片模板的 `.hl` 读同一个变量 ⇒ 预览与成片同口径。 -->
            <div class="mini-field mt-4"><label>高亮字号</label><input type="range" min="0.5" max="2" step="0.05" v-model.number="hlFontScale" title="只改高亮关键词那一段的字号，不动字体与整块标题字号"><input class="rv-input" type="number" min="0.5" max="2" step="0.05" :value="hlFontScale" title="可直接填写（0.5 ~ 2 倍）" @change="numBind(hlFontScale, (v) => (hlFontScale = v), $event, 0.5, 2)"></div>
            <!-- 高亮词：出片认的是 `cover.highlight_keywords`，**它为空时会回退用 polish.toml 的
                 `[filter] highlight`**（`cover.rs::cover_highlight_words`）—— 所以这里必须把回退那一串
                 也显示出来，否则"看着没设置、其实在生效"（用户报过「高亮词没有实际使用上」）。 -->
            <div class="muted small mt-4 hl-note" :title="hlSourceTitle">{{ hlSourceHint }}</div>
            <div v-if="highlightKeywords.length" class="hl-chips mt-4">
              <span v-for="(kw, i) in highlightKeywords" :key="i" class="hl-chip">
                {{ kw }}
                <button class="hl-x" title="删除" @click="removeHighlightKeyword(i)">×</button>
              </span>
            </div>
          </template>

          <!-- 特效 / 表情层：出片里同属 `#fx-extra`，靠数组顺序叠；`amount` 在表情那支是"大小"。
               属性用 `v-for` 包一层（`fx` 是 v-for 别名），这样 v-model 写的是那一份真实数据。 -->
          <template v-else>
            <div class="lyr-props-h">{{ fxTypeName(sel.type) }}</div>
            <div v-for="fx in selFxRows" :key="fx.uid">
              <template v-if="fx.type === 'emoji'">
                <div class="mini-row mt-4">
                  <input v-model="fx.text" class="hex-input grow" maxlength="12" placeholder="🙂" title="要贴的表情，可直接粘贴 emoji（最多 12 个字符）">
                </div>
                <div class="emoji-picks mt-4">
                  <button v-for="e in EMOJI_PICKS" :key="e" class="emoji-pick" :title="'用 ' + e" @click="fx.text = e">{{ e }}</button>
                </div>
              </template>
              <div class="mini-grid mt-4">
                <div class="mini-field"><label>{{ fx.type === 'emoji' ? '大小' : '强度' }}</label><input type="range" min="0" max="1" step="0.05" v-model.number="fx.amount"><span class="rv">{{ Number(fx.amount).toFixed(2) }}</span></div>
                <div class="mini-field"><label>X</label><input type="range" min="0" max="100" step="1" v-model.number="fx.x"><span class="rv">{{ fx.x }}%</span></div>
              </div>
              <div class="mini-grid mt-4">
                <div class="mini-field"><label>Y</label><input type="range" min="0" max="100" step="1" v-model.number="fx.y"><span class="rv">{{ fx.y }}%</span></div>
                <div v-if="fx.type === 'overlay'" class="mini-field"><label>颜色</label><input type="color" v-model="fx.color"></div>
              </div>
            </div>
            <div class="lyr-props-note mt-4">{{ fxHint(sel.type) }}</div>
          </template>
        </div>

        <!-- 图层组列表（**上 = 画得更靠前**）：安全区在最上、只可隐藏（不进 `[cover] layers`）；
             下面就是 `[cover] layers` 的**倒序**（栈底在最下），顺序可以任意拖。
             出片按同一个栈注入 z-index，所以列表里看到的先后就是成片里的先后。 -->
        <div class="lyr-list" ref="listRef">
          <div class="lyr-grp">安全区</div>
          <div v-for="g in guideLayers" :key="g.key" class="lyr" :class="{ sel: selectedKey === g.key }" @click="selectLayer(g.key)">
            <span class="lyr-drag off" title="辅助线不参与图层序">⠿</span>
            <button class="lyr-eye" :class="{ off: !g.visible }" :title="g.visible ? '隐藏这条线' : '显示这条线'" @click.stop="toggleGuide(g.guide)">
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <template v-if="g.visible"><path d="M2 12s3.6-6.5 10-6.5S22 12 22 12s-3.6 6.5-10 6.5S2 12 2 12Z" /><circle cx="12" cy="12" r="2.6" /></template>
                <template v-else><path d="M4 4l16 16" /><path d="M9.9 5.7A9.7 9.7 0 0 1 12 5.5c6.4 0 10 6.5 10 6.5a18 18 0 0 1-3.3 3.9M6.5 7.6A18 18 0 0 0 2 12s3.6 6.5 10 6.5c1.1 0 2.1-.2 3-.5" /></template>
              </svg>
            </button>
            <span class="lyr-th th-guide"></span>
            <span class="lyr-nm">{{ g.name }}</span>
            <span class="lyr-lk" title="只画在设计器里，不进成片；只能隐藏">辅助线</span>
          </div>
          <div class="lyr-grp">封面图层<span class="lyr-grp-h">拖动 ⠿ 调序</span></div>
          <div v-for="l in contentLayers" :key="l.key"
               class="lyr"
               :class="{ sel: selectedKey === l.key, hidden: !l.visible, dragging: dragKey === l.key, 'drop-before': dropAt === l.key + ':before', 'drop-after': dropAt === l.key + ':after' }"
               :draggable="!l.locked"
               @click="selectLayer(l.key)"
               @dragstart="onDragStart(l, $event)" @dragover="onDragOver(l, $event)" @drop="onDrop(l, $event)" @dragend="onDragEnd">
            <span class="lyr-drag" :class="{ off: l.locked }" :title="l.locked ? '视频预览层不给调序（它是最下面那一层）' : '按住拖动：调整上下顺序'">⠿</span>
            <button class="lyr-eye" :class="{ off: !l.visible }" :title="l.visible ? '隐藏这一层（数据都留着）' : '显示这一层'" @click.stop="toggleVisible(l.key)">
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <template v-if="l.visible"><path d="M2 12s3.6-6.5 10-6.5S22 12 22 12s-3.6 6.5-10 6.5S2 12 2 12Z" /><circle cx="12" cy="12" r="2.6" /></template>
                <template v-else><path d="M4 4l16 16" /><path d="M9.9 5.7A9.7 9.7 0 0 1 12 5.5c6.4 0 10 6.5 10 6.5a18 18 0 0 1-3.3 3.9M6.5 7.6A18 18 0 0 0 2 12s3.6 6.5 10 6.5c1.1 0 2.1-.2 3-.5" /></template>
              </svg>
            </button>
            <span class="lyr-th" :class="'th-' + l.th">{{ l.glyph }}</span>
            <span class="lyr-nm">{{ l.name }}</span>
            <span v-if="l.locked" class="lyr-lk" title="封面用的那一帧：可以隐藏，但不给删/调序">锁定</span>
            <span class="lyr-tools">
              <button v-if="!l.locked" class="ico-btn del" title="删除这一层（属性还在，加回来就恢复）" @click.stop="removeLayer(l)"><Icon name="trash" :size="12" /></button>
            </span>
          </div>
        </div>

        <!-- 底部工具条（PS 的图层面板底部那一排）：新建图层 + 一句用法提示 -->
        <div class="lyr-bar">
          <div class="add-wrap" ref="addWrapRef">
            <button class="mini-btn" :class="{ selected: addOpen }" title="新建图层：先选一种类型"
                    @click="addOpen = !addOpen"><Icon name="plus" :size="11" /> 新建图层</button>
            <!-- 类型选择器：**可叠加**的六种（每种能加任意多个）+ **单层**的三种（每种只有一层）。
                 单层那三种已经在栈里且可见时置灰；被隐藏时点一下＝直接打开它。
                 视频预览与两条安全区不在这里：前者是基础层、后者只是辅助线。 -->
            <div v-if="addOpen" class="add-menu">
              <div class="add-sec">可叠加</div>
              <div class="add-grid">
                <button v-for="t in ADD_MULTI" :key="t.kind" class="add-item" :title="t.hint" @click="addLayer(t.kind)">
                  <span class="add-gl" :class="'gl-' + t.kind">{{ t.glyph }}</span>{{ t.label }}
                </button>
              </div>
              <div class="add-sec">单层</div>
              <div class="add-grid">
                <!-- 已经有的一层**不置灰**：点它 = 选中并打开（隐藏时）—— 比"灰着不给点"有用 -->
                <button v-for="t in ADD_SINGLE" :key="t.kind" class="add-item" :class="{ has: hasLayer(t.kind) }"
                        :title="singleHint(t)" @click="addLayer(t.kind)">
                  <span class="add-gl" :class="'gl-' + t.kind">{{ t.glyph }}</span>{{ t.label }}
                  <span v-if="hasLayer(t.kind)" class="add-flag">已有</span>
                </button>
              </div>
              <div class="add-foot" title="视频预览层用封面那一帧（时间 / 缩放 / 偏移都在它的属性里），两条安全区是只画在设计器里的辅助线；三者都不在这里">视频预览与安全区固定</div>
            </div>
          </div>
          <span class="lyr-bar-hint">眼睛＝隐藏 · 拖动＝调序</span>
        </div>
      </div>

    </div>
  </div>
  <!-- 渲染通道弹窗**已删除**（2026-10-01）：封面只剩 Takumi 一条通道，本仓那条 headless Chrome
       截图腿整条删掉了。只剩一个按钮的弹窗不再是"选择"，只是多点一下才出图。
       Takumi 不可用时（没编 feature / 字体库为空）由下面这行说清，同时把按钮置灰。 -->
  <p v-if="coverCap.loaded && !coverCap.ready" class="hint warn">{{ coverCap.why }}</p>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { toast } from '../stores/toast'
import { addClient } from '../stores/log'
import Icon from './Icon.vue'
import { clipMediaUrl, fmtTime } from '../utils'
import { watchJob, busyOn, clipKey, kindText } from '../api/jobs'
import { useJobs } from '../composables/useJobs'
import {
  fetchFonts, fetchProject,
  coverFrame, coverBackground, coverTitle, coverConfig, runCover,
} from '../api/polish'
import { api } from '../api/client'
import { ensureFontFace } from '../polish/fonts'

const props = defineProps({
  dir: { type: String, required: true },
  /** 项目 id（拼资源键用，与后端 server/scope_key.rs 一致）；缺省时不做作用域门控 */
  pid: { type: [String, Number], default: '' }
})
const emit = defineEmits(['changed'])
const generating = ref(false)
// 封面**只有 Takumi 一条通道**（本仓那条 headless Chrome 截图腿 2026-10-01 整条删除）——
// 通道选择弹窗与它的 state 一并删掉了；别再凭"当年有个弹窗"把它加回来。
// 服务端能力位（`/api/health.cover_static`）：没编 feature / 字体库为空时把「生成封面」置灰，
// 并在卡片里**说清为什么**（宁可点不动，也别点了才在任务日志里才知道）。
const coverCap = ref({ loaded: false, ready: false, why: "" })

async function loadCoverCap() {
  try {
    const h = await api('/api/health')
    const c = h && h.cover_static
    if (!c) { coverCap.value = { loaded: true, ready: false, why: "服务端没报这一位（版本不一致？）" }; return }
    const why = c.ready
      ? ""
      : (!c.compiled
        ? "这个二进制没编 `cover-takumi` feature（重编：cargo build -p vtrim-cli --features vtrim-polish/cover-takumi）"
        : `字体库是空的：把字体放进 ${c.fonts_dir}`)
    coverCap.value = { loaded: true, ready: !!c.ready, why }
  } catch (e) {
    coverCap.value = { loaded: true, ready: false, why: "读能力位失败：" + (e && e.message ? e.message : e) }
  }
}
// 封面与渲染/LLM/后处理写同一批文件（polish.toml / cover.jpg / _cover_frame.jpg），
// 只在该片段被占用时禁用「生成封面」；改参数是同步保存，随时可存。
const { jobs: allJobs } = useJobs()
const clipBusyJob = computed(() => {
  // 反斜杠用 charCode 拼，别在字符串里写转义（切片页 sameDir 同款写法）
  const name = String(props.dir || '').split(String.fromCharCode(92)).join('/').split('/').filter(Boolean).pop() || ''
  const pid = props.pid
  if (!name || !pid) return null
  return busyOn(allJobs.value, [clipKey(pid, name)])
})
const generatingTitle = ref(false)
/**
 * 封面配置是否已成功读回。**它同时是一道写入门闸**（读失败时用默认值写回 = 把磁盘上的
 * 配置清成默认），所以不能随便放开 —— 但正因为是门闸，读失败就必须喊出来：以前 `onMounted`
 * 用裸 `catch {}` 收尾，configLoaded 永远停在 false，于是这个面板里**之后所有改动都不再保存**，
 * 界面上一个字都不提示（用户花十分钟调封面，关掉再开全没了）。现在配合下面的 error banner。
 */
const configLoaded = ref(false)
const configLoadError = ref('')
const saveError = ref('')
const savingConfig = ref(false)
const stageRef = ref(null)
const stageWrapRef = ref(null)
const controlsRef = ref(null)
const stageScale = ref(1)
let resizeRo = null
const videoSrc = ref('')
const videoRef = ref(null)
const fileInput = ref(null)
const titleLines = ref([])
/** `.title-text` 元素本身（自动排版要量它的 rect；量 `.title-wrap` 会量到那个更宽的容器） */
const titleTextRef = ref(null)
/** 自动排版放不下时给用户的一句提示（放不下就说出来，别静默） */
const titleFitHint = ref('')
/** 封面关键帧的时间点（`[cover] frame_pos`，0~1 的比例）—— 出片按它截帧，
 *  设计器必须**按同一个值反推出秒数**去 seek，否则打开时视频停在 0 秒、成片却是另一帧。 */
const framePos = ref(0.4)
const time = ref(0)
const textX = ref(35)
const textY = ref(50)
const textWidth = ref(62)
const fontSize = ref(120)
const stroke = ref(20)
const hlColor = ref('#ffcc00')
// 高亮词的相对字号（1.0 = 与标题同号）；落到根变量 --hl-fs，出片模板的 .hl 读同一个变量
const hlFontScale = ref(1)
const highlightKeywords = ref([])
/** 高亮词现在**是回退来的**（`cover.highlight_keywords` 为空 → 用 `[filter] highlight`）。
 *  只为了让面板说实话：用户一增删就固化成封面自己的那份。 */
const hlFromFilter = ref(false)
const hlSourceHint = computed(() => {
  if (hlFromFilter.value) return '沿用「关键词过滤」里的高亮词，改一下就固化到封面'
  return highlightKeywords.value.length ? '这些词会在标题里高亮' : '还没设置高亮词（标题不会高亮）'
})
const hlSourceTitle = computed(() => (hlFromFilter.value
  ? '封面的 highlight_keywords 是空的，出片回退用 polish.toml 的 [filter] highlight（「设置 · 关键词过滤」那串）—— 面板照实把它们显示出来；在这里增删任意一个词，这批词就会写进封面自己的 highlight_keywords'
  : '写进 polish.toml 的 [cover] highlight_keywords；出片按它给标题上高亮色（词为空时回退 [filter] highlight）'))

/** 行尾数字框：手填只在 [lo, hi] 上夹、**不吸附 step**（滑块才有栅格）；清空/非数字还原显示、不写模型。 */
function numBind(cur, set, e, lo, hi) {
  const raw = String(e.target.value ?? '').trim()
  const n = Number(raw)
  if (raw === '' || !Number.isFinite(n)) {
    e.target.value = String(cur)
    return
  }
  const c = Math.min(hi, Math.max(lo, n))
  set(c)
  // 夹回来的值可能等于原值（填 999 被夹到 100）→ 模型没变、Vue 不会回写 DOM，这里手动同步一次
  e.target.value = String(c)
}

// ---- 封面字体（字体库：用户固定目录 ~/.vtrim/fonts，读侧另有老安装回落）----
// 值存 polish.toml 的 `[cover] font`：空 = 兜底字体栈（首项随库里默认走，老工程就是这个状态），
// 非空 = 库里的字体 id。出片由 cover.rs 注入 @font-face，这里负责**让设计器预览也认得它**。
const fonts = ref([])
const coverFont = ref('')
const fontOptions = computed(() => {
  // ⚠️ **不再有「（系统默认）」这一项**（2026-10-02，与字幕字体下拉同一口径）：
  // 自带字体已内嵌、库里永远有字可用，再留一个"回到系统栈"的选择只会让人选到更差的观感。
  // ⚠️ **空值同理不再作为可选字体出现**（2026-10-02 用户要求「只显示已有的字体」）：它是老工程
  // 的"没选过"状态，不是一种字体。当前值真为空时补一条**占位**项照实说明，选了具体字体即消失。
  const opts = []
  for (const f of fonts.value) {
    // ⚠️ 只列**真能用**的：`.ttc`（浏览器 @font-face 不认）与没下载的自带字体都不进下拉；
    // 默认字体带 `[默认]` 标记（与字幕字体下拉 / 字体库管理页同一份 `is_default`，三处联动）。
    // ⚠️ **emoji 字体不进这个下拉**（2026-10-02 职责分离）：封面上的 emoji 由
    // `cover.rs::emoji_font_stack()` **自动补充**（库里那支排在字体栈尾部 / 注册序最后），
    // 用户选它当正文只会把没字形的中文交给它。字幕字体下拉同一条判据。
    if (f.installed === false || /\.ttc$/i.test(f.file || '') || f.emoji) continue
    const base = f.family && f.family !== f.id ? `${f.family}（${f.id}）` : f.id
    opts.push({ value: f.id, label: f.is_default ? `${base}[默认]` : base })
  }
  const cur = coverFont.value
  if (!cur) opts.unshift({ value: '', label: '未指定 · 用默认文字字体', placeholder: true })
  else if (!fonts.value.some((f) => f.id === cur)) opts.push({ value: cur, label: `${cur}（不在字体库）` })
  return opts
})
/** 选中的即浏览器里要用的字族名；未选时交给全站 `--font`（不写 fontFamily，保持继承） */
const coverFontFamily = computed(() => (coverFont.value ? `"${coverFont.value}"` : ''))

async function loadFonts() {
  try {
    const r = await fetchFonts('读取字体库')
    fonts.value = r.fonts || []
  } catch (e) {
    addClient('warn', '字体库读取失败，封面字体只能手填：' + (e.message || e), 'cover')
  }
}

/** 换封面字体：改的是 ref，落盘交给下面那条防抖保存（与其它封面参数同一条路） */
function onCoverFontChange(e) {
  const next = e.target.value
  coverFont.value = next
  // 先让浏览器认得它，预览才会立刻换字形（出片侧的 @font-face 由 cover.rs 注入）
  if (next) ensureFontFace(props.pid, props.dir, next, { onDebug: (m) => addClient('debug', m, 'cover') })
}
const highlightInput = ref('')
const letterSpacing = ref(8)
const lineHeight = ref(1.7)
const textShadow = ref(true)
const gradientDir = ref('left')
const vignette = ref(0.08)
const align = ref('left')
/** 字幕高亮色（`[style.subtitle] highlight_color`）—— 出片的渐变底色由它推导，预览要跟着它 */
const subtitleHlColor = ref('')
const layout = ref({})
// ---- 封面画布：**恒 1920×1080 横版**（不跟随 [canvas] orientation —— 平台封面就是横的）。
// 「素材原比例」指封面里的**帧层**（.stage-video / cover.html #frame）用 object-fit:contain
// 完整显示、不被裁切；画布尺寸本身与项目方向无关。
const COVER_W = 1920
const COVER_H = 1080
const duration = ref(0)
const playing = ref(false)
/** 背景模式：color（16 进制纯色，默认黑）/ blur（当前帧模糊平铺）—— 只有这两种。
 *  背景是**帧之下的一层**：帧缩小/平移时露出，不替换帧（用户要求）。 */
const bgMode = ref('color')
const bgColor = ref('#000000')
/** 手填的 16 进制码（与取色器双向同步；只认 #rgb / #rrggbb） */
const bgColorText = ref('#000000')
const blurScale = ref(1)
/** 模糊背景用的是出片同一张 `_cover_frame.jpg`（预览=出片同一份素材，不是另一条时间轴） */
const bgImageOk = ref(true)
/** 那张截帧不在时自动补一次（出片结束会删掉它，所以平时确实没有这个文件） */
const bgFrameTried = ref(false)
/** 帧层（`_cover_frame.jpg`）能不能显示。缺文件时置 false，由 onMounted / exportFrame 补出来。
 *  ⚠️ 帧层与背景层**共用同一个文件**（`COVER_FRAME_IMG`），但各有一条加载状态：
 *  背景层那条是 blur 模式专用的兜底（`bgFrameTried`），这条是帧层自己的。 */
const frameImageOk = ref(true)
/** 帧层的补帧只做一次（与 `bgFrameTried` 同款，避免 404 死循环） */
const frameFrameTried = ref(false)
/** 导出关键帧后自增，给 URL 去缓存 */
const frameStamp = ref(0)
const BG_MODES = [
  { key: 'color', label: '纯色', hint: '16 进制颜色铺满背景（默认黑）；帧缩小/平移时才露出来' },
  { key: 'blur', label: '当前帧模糊', hint: '关键帧（当前帧）模糊后平铺铺满，可调模糊倍率' },
  { key: 'image', label: '自定义图片', hint: '导入的图片铺满背景（写在 _cover_bg.jpg，不覆盖关键帧）' }
]
/** 导入背景图后自增，给 `_cover_bg.jpg` 的 URL 去缓存 */
const bgStamp = ref(0)
/** 帧层（当前帧清晰图）的位移/缩放：拖动预览 = 平移，滚轮 = 缩放（用户要求：按之前的逻辑） */
const coverZoom = ref(1)
const coverOffsetX = ref(0)
const coverOffsetY = ref(0)
const stageDrag = ref(null)
const fxList = ref([])
/** 安全区线框的开关（纯设计器辅助线，不进出片模板、不进 cover.toml）。
 *  偏好**在 setup 里同步读** —— 放进 computed 或 onMounted 会先画一帧再切，闪一下
 *  （与 `stores/ui.js::readSideCollapsed` 同一个教训）。两个开关各自独立，可以只看 4:3。 */
const LS_GUIDES = 'vtrim.ui.coverGuides'
const showGuide169 = ref(readGuides().includes('169'))
const showGuide43 = ref(readGuides().includes('43'))

function readGuides() {
  try {
    return String(localStorage.getItem(LS_GUIDES) ?? '169|43')
  } catch {
    return '169|43'
  }
}

function toggleGuide(key) {
  const flag = key === '169' ? showGuide169 : showGuide43
  flag.value = !flag.value
  const on = []
  if (showGuide169.value) on.push('169')
  if (showGuide43.value) on.push('43')
  try { localStorage.setItem(LS_GUIDES, on.join('|')) } catch {}
}

// ===== 图层栈（PS 式面板：左预览 / 右上属性 / 右下图层列表）=====
//
// `[cover] layers` 就是**真图层栈**（从下到上）：顺序与可见性都落盘，出片侧由
// `cover.rs::resolve_cover_stack` / `plan_cover_layers` 注入 z-index。**这里那份判定必须与它同口径**，
// 否则就是"列表里这么排、成片里那么画"—— 这类 bug 不报错，只能比对成片才发现。
//
// 语义（与 PS 对齐；与 v1 那套"用属性字段模拟开关"完全不同）：
//   · **隐藏**（眼睛）= `visible:false`：层还在、属性一个字节不动，只是不画；
//   · **删除**（垃圾桶）= 从栈里摘掉；固定层的属性照样留着，加回来就恢复。
//     例外：特效/表情的"属性"就是它自己那一条 `[[cover.effects]]`，删它等于删那条数据。
//
// 后端解析的四条容错（缺项补齐 / 未知 id 忽略 / 重复只认第一次 / 没列出的特效仍渲染）在
// `resolveStack()` 里照抄了一份，**两边必须一起改**。
const stack = ref([])
const selectedKey = ref('title')
/** 项目名：文字层被删掉后再加回来时，拿它按标点回退拆行（与 `cover.rs` 同一套规则） */
const projectName = ref('')
/** 特效 id：**落盘**进 `[[cover.effects]].id`，栈用 `fx:<id>` 引用它。
 *  用短随机串而不是自增序号 —— 序号会"这次会话删过一条、下次会话又从 1 数起"撞车，
 *  撞了之后栈里两条同 id 会被解析成一条（PS 里就是图层凭空少一个）。 */
const newFxId = () => 'f' + Math.random().toString(36).slice(2, 8)

/** 特效/表情在列表里的缩略字（emoji 那支直接用贴的那个字符） */
const FX_GLYPH = { glow: '✦', blur: '◍', vignette: '◐', overlay: '▨', scanline: '▤' }
const fxHints = {
  glow: '一团暖色光晕，x/y 是圆心',
  blur: '局部模糊：压住不想露的地方',
  vignette: '一处压暗，视线往 x/y 聚',
  overlay: '整幅叠一层颜色（overlay 混合）',
  scanline: '横向扫描线',
  emoji: '贴一个表情：x/y 是字符中心'
}
function fxHint(type) { return fxHints[type] || '' }

/** 新建图层里**可叠加**的六种（顺序 = 菜单里从上到下）—— 数据落进 `[[cover.effects]]` */
const ADD_MULTI = [
  { kind: 'emoji', label: '表情', glyph: '🙂', hint: '贴一个表情（可多个），默认落在主播身上' },
  { kind: 'glow', label: '光晕', glyph: '✦', hint: '一团暖色光晕' },
  { kind: 'blur', label: '模糊', glyph: '◍', hint: '局部模糊（适合挡掉不想露的地方）' },
  { kind: 'vignette', label: '暗角点', glyph: '◐', hint: '一处压暗' },
  { kind: 'overlay', label: '颜色覆盖', glyph: '▨', hint: '整幅叠一层颜色（overlay 混合）' },
  { kind: 'scanline', label: '扫描线', glyph: '▤', hint: '横向扫描线' }
]
/** 新建图层里**单层**的三种：每种只有一层（已经在栈里且可见 → 菜单里置灰）。
 *  加上这两组就是"当前设计器里所有能编辑的类型" —— 视频预览与安全区是固定层，不在菜单里。 */
const ADD_SINGLE = [
  { kind: 'bg', layerId: 'bg', label: '背景', glyph: '背', hint: '垫在视频预览之下的衬底' },
  { kind: 'filter', layerId: 'filter', label: '滤镜', glyph: '滤', hint: '渐变 + 暗角' },
  { kind: 'text', layerId: 'title', label: '文字', glyph: '文', hint: '封面标题（带 LLM 生成 / 高亮）' }
]

/** 图层 id → 属性面板/列表要用的元信息（`kind` 决定右上显示哪一屏属性） */
const LAYER_META = {
  bg: { kind: 'bg', name: '背景', glyph: '背', th: 'bg' },
  frame: { kind: 'video', name: '视频预览', glyph: '视', th: 'video', locked: true },
  filter: { kind: 'filter', name: '滤镜', glyph: '滤', th: 'filter' },
  title: { kind: 'text', name: '文字', glyph: '文', th: 'text' }
}

/** 出片侧同一份默认序（`cover.rs::default_cover_stack`）：bg < frame < filter < 每个特效 < title */
function defaultOrderIds() {
  return ['bg', 'frame', 'filter', ...fxList.value.map((f) => 'fx:' + f.uid), 'title']
}

/** 手写的 `fx:#<i>`（出片侧给"没有 id 的老特效"合成的槽位）也认 —— 两边口径一致 */
function fxAlias(id) {
  if (!id || !id.startsWith('fx:#')) return id
  const f = fxList.value[Number(id.slice(4))]
  return f ? 'fx:' + f.uid : id
}

/** `[cover] layers` → 完整栈。与 `cover.rs::resolve_cover_stack` 同口径：
 *  缺项按默认序的相对位置补回来、未知 id 忽略、重复只认第一次；没列出的特效仍然渲染。 */
function resolveStack(raw) {
  const order = defaultOrderIds()
  const out = []
  const seen = new Set()
  for (const e of Array.isArray(raw) ? raw : []) {
    const id = fxAlias(String(e?.id ?? '').trim())
    if (!id || seen.has(id) || !order.includes(id)) continue
    seen.add(id)
    out.push({ id, visible: e?.visible !== false })
  }
  order.forEach((id, di) => {
    if (seen.has(id)) return
    let pos = 0
    for (let i = di - 1; i >= 0; i--) {
      const p = out.findIndex((l) => l.id === order[i])
      if (p >= 0) { pos = p + 1; break }
    }
    seen.add(id)
    out.splice(pos, 0, { id, visible: true })
  })
  return out
}

/** 按默认序的相对位置插一层（`addLayer` 用；与 `resolveStack` 的补齐规则同一套） */
function insertByOrder(list, id) {
  const order = defaultOrderIds()
  const di = order.indexOf(id)
  for (let i = di - 1; i >= 0; i--) {
    const pos = list.findIndex((l) => l.id === order[i])
    if (pos >= 0) return [...list.slice(0, pos + 1), { id, visible: true }, ...list.slice(pos + 1)]
  }
  return [{ id, visible: true }, ...list]
}

const guideLayers = computed(() => [
  { key: 'guide-169', kind: 'guide', guide: '169', name: '16:9 安全区', visible: showGuide169.value },
  { key: 'guide-43', kind: 'guide', guide: '43', name: '4:3 裁切区', visible: showGuide43.value }
])

/** 内容层：**上 = 画得更靠前**（列表顶 = 出片里最后画的）→ 栈的倒序；隐藏的层照样在列表里（置灰） */
const contentLayers = computed(() => {
  const out = []
  for (let i = stack.value.length - 1; i >= 0; i--) {
    const item = stack.value[i]
    const visible = item.visible !== false
    if (item.id.startsWith('fx:')) {
      const fx = fxList.value.find((f) => 'fx:' + f.uid === item.id)
      if (!fx) continue
      out.push({
        key: item.id, kind: 'fx', fx, type: fx.type, visible, th: 'fx', draggable: true,
        glyph: fx.type === 'emoji' ? (fx.text || '🙂') : (FX_GLYPH[fx.type] || '✦'),
        name: fxNames[fx.type] || fx.type
      })
      continue
    }
    const meta = LAYER_META[item.id]
    if (!meta) continue
    out.push({
      key: item.id, kind: meta.kind, visible, th: meta.th, glyph: meta.glyph, name: meta.name,
      locked: !!meta.locked, draggable: !meta.locked
    })
  }
  return out
})

const layers = computed(() => [...guideLayers.value, ...contentLayers.value])
const sel = computed(() => layers.value.find((l) => l.key === selectedKey.value) || null)
/** 属性面板里那个特效：用数组包一层，好让模板的 v-model 写在 v-for 别名上（写的是同一份数据） */
const selFxRows = computed(() => (sel.value?.kind === 'fx' ? [sel.value.fx] : []))

/** 块 → z-index（与 `cover.rs::plan_cover_layers` 同口径：从下到上 1,2,3…；filter 展开成 暗角+渐变） */
const blockZ = computed(() => {
  const z = {}
  let n = 1
  for (const l of stack.value) {
    if (l.id === 'bg') z.bg = n++
    else if (l.id === 'frame') z.frame = n++
    else if (l.id === 'filter') { z.vignette = n++; z.overlay = n++ }
    else if (l.id === 'title') z.title = n++
    else if (l.id.startsWith('fx:')) z[l.id] = n++
  }
  return z
})

const layerVisible = computed(() => {
  const m = {}
  for (const l of stack.value) {
    if (l.id === 'filter') { m.vignette = l.visible !== false; m.overlay = l.visible !== false }
    else m[l.id] = l.visible !== false
  }
  return m
})

/** 舞台上一块的样式：z-index + 显隐（⚠️ CSS 里**别再写**内容层的 z-index，否则两边打架） */
function layerStyle(key, extra) {
  return Object.assign(
    { zIndex: blockZ.value[key] || 1, display: layerVisible.value[key] === false ? 'none' : '' },
    extra || {}
  )
}

/** 菜单里"单层三种"的层 id */
const SINGLE_ID = { bg: 'bg', filter: 'filter', text: 'title' }
function hasLayer(kind) {
  const it = stack.value.find((l) => l.id === SINGLE_ID[kind])
  return !!it && it.visible !== false
}
function singleHint(t) {
  const it = stack.value.find((l) => l.id === t.layerId)
  if (!it) return t.hint
  return it.visible === false ? '这一层被隐藏了，点一下打开它' : '这一层已经有了'
}

function selectLayer(key) {
  selectedKey.value = key
  addOpen.value = false
}

/** 新建图层：
 *  · 可叠加的（表情 + 五种特效）→ 往 `effects` 加一条（带稳定 id）并**追加到栈顶**（PS：新建在上）；
 *  · 单层的三种 → 已经在栈里就只是"打开它"，否则按默认序插回去（属性字段一个都没动，内容还在）。 */
function addLayer(kind) {
  addOpen.value = false
  const single = ADD_SINGLE.find((t) => t.kind === kind)
  if (single) {
    const item = stack.value.find((l) => l.id === single.layerId)
    if (item) item.visible = true
    else stack.value = insertByOrder(stack.value, single.layerId)
    selectedKey.value = single.layerId
    nextTick(() => syncControlsBound())
    return
  }
  const type = ADD_MULTI.some((t) => t.kind === kind) ? kind : 'glow'
  const at = type === 'emoji' ? liverOrigin() : { x: 50, y: 50 }
  const uid = newFxId()
  fxList.value.push({
    uid,
    type,
    amount: type === 'emoji' ? 0.5 : 0.3,
    x: Math.round(at.x),
    y: Math.round(at.y),
    color: '#000000',
    text: type === 'emoji' ? '😆' : ''
  })
  stack.value = [...stack.value, { id: 'fx:' + uid, visible: true }]
  selectedKey.value = 'fx:' + uid
  nextTick(() => syncControlsBound())
}

/** 删除图层：从栈里摘掉（固定层的属性全留 → 加回来就恢复）。
 *  特效/表情的"属性"就是 `[[cover.effects]]` 那一条，所以连它一起删。
 *  ⚠️ 视频预览（`locked`）不给删：它就是封面那一帧，只能隐藏。 */
function removeLayer(l) {
  if (!l || l.locked) return
  if (l.kind === 'fx') fxList.value = fxList.value.filter((f) => 'fx:' + f.uid !== l.key)
  stack.value = stack.value.filter((x) => x.id !== l.key)
  addOpen.value = false
  nextTick(() => syncControlsBound())
}

/** 眼睛：**非破坏性隐藏**（属性全留，出片只是 `display:none`） */
function toggleVisible(key) {
  const item = stack.value.find((l) => l.id === key)
  if (item) item.visible = item.visible === false
}

// ===== 拖动调序（HTML5 DnD；列表显示序 = 栈的倒序，所以两头都要翻一次）=====
const dragKey = ref('')
const dropAt = ref('')
function onDragStart(l, e) {
  if (!l.draggable) { e.preventDefault(); return }
  dragKey.value = l.key
  dropAt.value = ''
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = 'move'
    e.dataTransfer.setData('text/plain', l.key)   // Firefox 必须有 dataTransfer 才会真开始拖
  }
}
function onDragOver(l, e) {
  if (!dragKey.value || dragKey.value === l.key || l.locked) return
  e.preventDefault()
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move'
  const r = e.currentTarget.getBoundingClientRect()
  dropAt.value = l.key + (e.clientY < r.top + r.height / 2 ? ':before' : ':after')
}
function onDrop(l, e) {
  e.preventDefault()
  const from = dragKey.value
  const at = dropAt.value
  onDragEnd()
  if (!from || !at || from === l.key) return
  moveLayerTo(from, l.key, at.endsWith(':before'))
}
function onDragEnd() {
  dragKey.value = ''
  dropAt.value = ''
}

/** 把 `fromKey` 拖到 `targetKey` 的上/下（都在**显示序**里算，再翻回栈序） */
function moveLayerTo(fromKey, targetKey, before) {
  const top = [...stack.value].reverse()
  const from = top.findIndex((x) => x.id === fromKey)
  if (from < 0) return
  const [item] = top.splice(from, 1)
  let to = top.findIndex((x) => x.id === targetKey)
  if (to < 0) return
  if (!before) to += 1
  top.splice(to, 0, item)
  stack.value = top.reverse()
  selectedKey.value = fromKey
}

/** 快捷键（PS 味）：Ctrl/Cmd+[ / ] 上下移一层、加 Shift 置顶/置底、Delete 删掉选中层。
 *  绑在面板容器上（`@keydown`），别挂 window —— 精修页还有别的全局键。 */
function onPanelKey(e) {
  const id = selectedKey.value
  if (!id || id.startsWith('guide') || LAYER_META[id]?.locked) return
  if ((e.ctrlKey || e.metaKey) && (e.key === ']' || e.key === '[')) {
    e.preventDefault()
    moveLayerBy(id, e.key === ']' ? 1 : -1, e.shiftKey)
  } else if ((e.key === 'Delete' || e.key === 'Backspace') && !e.ctrlKey && !e.metaKey) {
    e.preventDefault()
    removeLayer(layers.value.find((l) => l.key === id))
  }
}
function moveLayerBy(id, delta, toEnd) {
  const top = [...stack.value].reverse()
  const i = top.findIndex((x) => x.id === id)
  if (i < 0) return
  const [item] = top.splice(i, 1)
  const idx = toEnd ? (delta > 0 ? 0 : top.length) : Math.max(0, Math.min(top.length, i - delta))
  top.splice(idx, 0, item)
  stack.value = top.reverse()
}

/** 「新建图层」的菜单：点空白处关掉（按钮与菜单都在 `addWrapRef` 里，用捕获阶段先判归属） */
const addOpen = ref(false)
const addWrapRef = ref(null)
function onDocPointerDown(e) {
  if (!addOpen.value) return
  const wrap = addWrapRef.value
  if (wrap && !wrap.contains(e.target)) addOpen.value = false
}

// 选中的那一层没了（删掉 / 栈被改短）→ 落到文字（有的话）或栈顶，别让属性面板空着
watch(
  () => layers.value.map((l) => l.key).join('|'),
  () => {
    if (layers.value.some((l) => l.key === selectedKey.value)) return
    selectedKey.value = contentLayers.value.find((l) => l.key === 'title')?.key || contentLayers.value[0]?.key || 'frame'
  }
)

let saveConfigTimer = null
let saveConfigPending = false
const titleLinesText = computed({
  get: () => titleLines.value.join('\n'),
  set: (v) => { titleLines.value = String(v || '').split('\n') }
})

function escapeHtml(s) {
  return String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]))
}

/** 没有 `title_lines` 时的回退：与 `cover.rs` 一致 —— 砍掉 `【主播名】` 前缀，再按中英标点拆行。 */
function splitTitleFallback(raw) {
  const clean = String(raw || '').split('【')[0]
  return clean
    .split(/[，。！？、,.!]/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0)
}

function highlightTitleLine(line, kws) {
  let html = escapeHtml(line)
  for (const kw of kws) {
    const target = escapeHtml(kw).toLowerCase()
    if (!target) continue
    let out = ''
    let rest = html
    let idx = rest.toLowerCase().indexOf(target)
    while (idx >= 0) {
      out += rest.slice(0, idx) + '<span class="hl">' + rest.slice(idx, idx + target.length) + '</span>'
      rest = rest.slice(idx + target.length)
      idx = rest.toLowerCase().indexOf(target)
    }
    html = out + rest
  }
  return html
}

const titleHtml = computed(() => {
  const lines = titleLines.value.length ? titleLines.value : ['']
  const kws = highlightKeywords.value.filter(Boolean)
  return lines.map(line => highlightTitleLine(line, kws)).join('\n')
})

const fxNames = { glow: '光晕', blur: '模糊', vignette: '暗角点', overlay: '颜色覆盖', scanline: '扫描线', emoji: '表情' }
function fxTypeName(t) { return fxNames[t] || t }
/** 常用表情：点一下换字符（也能直接往输入框里粘任意 emoji） */
const EMOJI_PICKS = ['😆', '😂', '😭', '🥺', '😮', '❤️', '🔥', '✨']
/** emoji 的尺寸区间（px，1920×1080 画布）——与 Rust `cover.rs` 的 COVER_EMOJI_* 同源 */
const COVER_EMOJI_MIN_PX = 48
const COVER_EMOJI_MAX_PX = 288
/** `#rrggbb` → `r,g,b`（特效颜色要拼进 rgba()，与 Rust `hex_to_rgb` 同一套）。 */
function fxHexToRgb(hex) {
  const h = String(hex || '').trim().replace('#', '')
  const full = h.length === 3 ? h.split('').map((c) => c + c).join('') : h
  if (!/^[0-9a-fA-F]{6}$/.test(full)) return '0,0,0'
  return [full.slice(0, 2), full.slice(2, 4), full.slice(4, 6)]
    .map((c) => parseInt(c, 16))
    .join(',')
}

function fxStyle(fx) {
  const s = stageScale.value || 1
  const amount = Number(fx.amount) || 0.3
  const x = Number(fx.x ?? 50)
  const y = Number(fx.y ?? 50)
  if (fx.type === 'emoji') {
    // x/y 是字符中心（定位/居中靠 .fx-emoji 的 CSS），字号按舞台缩放；
    // 阴影同样按舞台缩放（出片是 1920 画布上的 px，模板写的是 0 6px 14px）
    return {
      left: `${x}%`,
      top: `${y}%`,
      fontSize: `${Math.round((COVER_EMOJI_MIN_PX + amount * (COVER_EMOJI_MAX_PX - COVER_EMOJI_MIN_PX)) * s)}px`,
      filter: `drop-shadow(0 ${(6 * s).toFixed(1)}px ${(14 * s).toFixed(1)}px rgba(0,0,0,0.45))`
    }
  }
  if (fx.type === 'glow') return {
    background: `radial-gradient(circle at ${x}% ${y}%, rgba(255,220,180,${Math.min(0.9, amount)}) 0%, transparent 60%)`
  }
  if (fx.type === 'blur') return {
    // 与 Rust `cover_fx_html` 的 blur 分支同公式：`1 + amount×14` px（画布 px → 按舞台缩放）
    backdropFilter: `blur(${((1 + amount * 14) * s).toFixed(1)}px)`,
    WebkitMaskImage: `radial-gradient(circle at ${x}% ${y}%, black 0%, transparent 60%)`,
    maskImage: `radial-gradient(circle at ${x}% ${y}%, black 0%, transparent 60%)`
  }
  if (fx.type === 'vignette') return {
    background: `radial-gradient(circle at ${x}% ${y}%, transparent 40%, rgba(0,0,0,${Math.min(1, amount)}) 100%)`
  }
  if (fx.type === 'overlay') {
    // 与 Rust 同一实现：颜色直接编成 rgba（不透明度 = amount，无额外夹取）+ overlay 混合，
    // 不是"铺个色块再调元素 opacity"
    return {
      background: `rgba(${fxHexToRgb(fx.color)},${amount})`,
      mixBlendMode: 'overlay'
    }
  }
  if (fx.type === 'scanline') return {
    background: `repeating-linear-gradient(0deg, rgba(255,255,255,${Math.min(0.5, amount * 0.45)}) 0 1px, transparent 1px 4px)`
  }
  return {}
}


/** 文字阴影：数值与出片模板 `cover_shadow_css` 一致（`0 5px 22px rgba(0,0,0,0.78)`），
 *  再按舞台缩放 —— 出片是 1920 画布上的 px，预览舞台是缩小的，不缩放就会看着更糊。 */
function coverShadowCss(s) {
  if (!textShadow.value) return 'none'
  return `0 ${(5 * s).toFixed(1)}px ${(22 * s).toFixed(1)}px rgba(0,0,0,0.78)`
}

const titleStyle = computed(() => {
  const s = stageScale.value || 1
  const ls = Number(letterSpacing.value) || 0
  return {
    fontSize: Math.max(12, Math.round((fontSize.value || 120) * s)) + 'px',
    // 选中的封面字体（空 = 不写，继续继承全站 `--font`，与出片模板的默认栈一致）
    fontFamily: coverFontFamily.value || undefined,
    letterSpacing: (ls * s).toFixed(1) + 'px',
    lineHeight: Number(lineHeight.value) || 1.7,
    // 描边色与模板一致（`#2a2522`，不是半透明黑）
    WebkitTextStroke: ((stroke.value || 20) * s).toFixed(1) + 'px #2a2522',
    textShadow: coverShadowCss(s),
    '--hlc': hlColor.value,
    '--hl-fs': (Number(hlFontScale.value) || 1) + 'em',
    textAlign: align.value || 'left'
  }
})

// ===== 标题安全区自动排版 =====
//
// 目标矩形 = 设计器里那条 **4:3 裁切区** 线框（`.guide-43`）。线框是 `height:100%` +
// `aspect-ratio:4/3` 居中，所以在 1920×1080 画布上它是 1440 宽、左右各留 240px、上下贴边。
// 标题放进去的含义：封面被平台裁成 4:3 时一个字都不会掉。
// （封面画布恒 1920×1080 横版，不随项目方向变。）
// 安全区目前**只存在于设计器**（`cover.rs` / `cover.html` 完全不知道它），所以这件事只能在
// 前端做 —— 好在设计器用的是与出片同一套 CSS（字体栈、line-height、text-align、
// `-webkit-text-stroke`），能量到真实排版。
const SAFE_X0 = 240
const SAFE_X1 = 1680
/** 字号下限，与「字号」滑杆的 min 一致 */
const TITLE_FONT_MIN = 24

/** 内缩一圈安全边距（画布 px）：`-webkit-text-stroke` 是沿字形轮廓**向外**画的，
 *  所以要留半个描边宽度；再留 12px 吸收「设计器用的字体」与「出片 headless Chrome
 *  能拿到的字体」不一致带来的排版差 —— 这个功能的定位是"把初始排版调好"，
 *  不是像素级契约，所以宁可保守。 */
function titleSafeMargin() {
  return 12 + (Number(stroke.value) || 0) / 2
}

/**
 * 首次拿到标题时把排版调好并放进安全区（只动 **字号 / 位置**，以及在确实换行过多时
 * 放宽一次文字区宽度）。对齐 / 行距 / 字距 / 描边 / 颜色 / 高亮词一律不碰。
 *
 * ⚠️ **只在「标题被获取」时调用**：`loadCoverConfig` 发现磁盘上没有 `title_lines`
 * （新工程，靠 `project.name` 回退拆行）、以及「LLM 生成标题」拿到新标题之后。
 * 手动操作（改文字 / 拖滑杆 / 点预设 / 拖拽）**一律不调** —— 否则用户每动一下都要跟
 * 自动排版打架，这正是"仅限第一次"要防的。
 */
async function fitTitleIntoSafeArea() {
  const stage = stageRef.value
  const el = titleTextRef.value
  // 没有任何非空行就不用排（空标题量出来是 0 高，排了也没意义）
  if (!stage || !el || !titleLines.value.some((l) => String(l ?? '').trim())) return
  // 字体没就绪时量到的是回退字体的宽度，排完照样会偏
  try { await document.fonts?.ready } catch {}
  await nextTick()

  const s = stageScale.value || 1
  const stageW = stage.clientWidth
  const stageH = stage.clientHeight
  if (!(stageW > 0) || !(stageH > 0)) return

  const m = titleSafeMargin()
  const left = (SAFE_X0 + m) * s
  const right = (SAFE_X1 - m) * s
  const top = m * s
  const bottom = (COVER_H - m) * s
  const maxW = Math.max(1, right - left)
  const maxH = Math.max(1, bottom - top)

  // 量的是**文字本身**的范围（用 Range 盖住文本节点），不是 `.title-text` 那个块级盒子的宽度。
  // ⚠️ 差别很大：设计器里 `.title-text` 是块级元素、宽度 = `.title-wrap` 的 62%；而出片模板
  // `cover.html` 的 `#title-wrap` 是 flex 列、`#title` 会收缩到内容宽。两者**视觉一致**
  // （左/中/右三种对齐下文字落点相同），但盒宽差好几百 px。
  // 直接量盒子会把"居中/右对齐的短标题"误判成溢出，再硬把 `text_x` 拽到 50% 附近 ——
  // 那是把本来没问题的排版改坏。量文字就没这个问题。
  const measure = () => {
    const sb = stage.getBoundingClientRect()
    let bb = null
    try {
      const range = document.createRange()
      range.selectNodeContents(el)
      const r = range.getBoundingClientRect()
      if (r.width > 0 && r.height > 0) bb = r
    } catch { /* 取不到就退回元素盒子 */ }
    if (!bb) bb = el.getBoundingClientRect()
    return { w: bb.width, h: bb.height, l: bb.left - sb.left, t: bb.top - sb.top }
  }
  const fits = (b) => b.w <= maxW + 0.5 && b.h <= maxH + 0.5

  // ① 字号：超出就按「两个方向里更紧的那个」比例缩。要迭代 —— 字号一变换行数会跳变，
  //    所以每轮都必须重新量，不能一次算完。
  for (let i = 0; i < 6; i++) {
    const b = measure()
    if (fits(b)) break
    const cur = Number(fontSize.value) || 120
    if (cur <= TITLE_FONT_MIN) break          // 到头了，剩下的交给 ③ 与下面的告警
    const k = Math.min(maxW / Math.max(b.w, 1), maxH / Math.max(b.h, 1)) * 0.98
    const next = Math.max(TITLE_FONT_MIN, Math.floor((cur * k) / 2) * 2)
    if (next >= cur) break                    // 比例算下来缩不动（k ≈ 1），没必要空转
    fontSize.value = next
    await nextTick()
  }

  // ② 最后手段：横向有余、却**因为换行太多而偏高**时，把文字区放宽一档（放宽 → 每行装更多
  //    → 行数变少 → 变矮）。只在确实变矮时才留下，否则原样退回去。
  let b = measure()
  if (!fits(b) && b.h > maxH && b.w <= maxW) {
    const before = textWidth.value
    const wider = Math.min(96, Math.max(before + 6, Math.ceil((maxW / s / W) * 100)))
    if (wider > before) {
      textWidth.value = wider
      await nextTick()
      b = measure()
      if (!fits(b)) {
        textWidth.value = before
        await nextTick()
        b = measure()
      }
    }
  }

  // ③ 位置：把整块挪进安全区。`textX/textY` 是 `.title-wrap` 的**中心点**
  //    （`translate(-50%,-50%)`），所以平移中心即整体平移；量的是文字本身（`.title-text`），
  //    不会因为外层那个 62% 宽的容器把本来在框内的标题白白挪走。
  let dx = 0
  let dy = 0
  if (b.w > maxW) dx = (left + right) / 2 - (b.l + b.w / 2)   // 比框还宽：只能居中
  else if (b.l < left) dx = left - b.l
  else if (b.l + b.w > right) dx = right - (b.l + b.w)
  if (b.h > maxH) dy = (top + bottom) / 2 - (b.t + b.h / 2)
  else if (b.t < top) dy = top - b.t
  else if (b.t + b.h > bottom) dy = bottom - (b.t + b.h)
  if (dx || dy) {
    const pct = (v) => Math.round(Math.min(100, Math.max(0, v)) * 10) / 10
    textX.value = pct(Number(textX.value) + (dx / stageW) * 100)
    textY.value = pct(Number(textY.value) + (dy / stageH) * 100)
    await nextTick()
    b = measure()
  }

  // 告警只看**最终状态**：中途缩不动不等于失败（可能其实已经放得下了）。
  // 判定 = 「到最小字号仍然溢出」，也就是现在这一刻还没进框。
  const relaxed = !fits(b)
  titleFitHint.value = relaxed
    ? '标题按最小字号仍放不进安全区，已按最小溢出摆放 —— 建议缩短标题或调窄文字区'
    : ''
  if (relaxed) console.warn('[cover] 标题按最小字号仍放不进安全区', b, { maxW, maxH })
}

/** 渐变底色：与 `cover.rs::build_cover_html` **同一套推导**（从字幕高亮色反推一个暗色调）。
 *  以前这里写死 `rgba(20,10,40,…)`，而出片是按 `[style.subtitle] highlight_color` 算的 ——
 *  改一个字幕高亮色，出片的渐变色调就变了，设计器却纹丝不动（用户报"设计器和出片不一致"）。 */
function coverGradientRgb() {
  const hex = String(subtitleHlColor.value || '').trim()
  if (!/^#[0-9a-fA-F]{3,6}$/.test(hex)) return [20, 10, 40]
  const h = hex.replace('#', '')
  const full = h.length === 3 ? h.split('').map((c) => c + c).join('') : h
  const r = (parseInt(full.slice(0, 2), 16) || 0) / 255
  const g = (parseInt(full.slice(2, 4), 16) || 0) / 255
  const b = (parseInt(full.slice(4, 6), 16) || 0) / 255
  return [
    Math.max(10, Math.floor((1 - r) * 30)),
    Math.max(10, Math.floor((1 - g) * 25)),
    Math.max(15, Math.floor((1 - b) * 50))
  ]
}

const gradientStyle = computed(() => {
  const dir = gradientDir.value
  if (dir === 'none') return { display: 'none' }
  const [r, g, b] = coverGradientRgb()
  const from = `rgba(${r},${g},${b},0.55)`
  const to = `rgba(${r},${g},${b},0.15)`
  if (dir === 'radial') return { background: `radial-gradient(ellipse at 30% 50%, ${from} 0%, ${to} 60%, transparent 100%)` }
  if (dir === 'right') return { background: `linear-gradient(270deg, ${from} 0%, ${to} 50%, transparent 100%)` }
  if (dir === 'top') return { background: `linear-gradient(180deg, ${from} 0%, ${to} 50%, transparent 100%)` }
  if (dir === 'bottom') return { background: `linear-gradient(0deg, ${from} 0%, ${to} 50%, transparent 100%)` }
  return { background: `linear-gradient(90deg, ${from} 0%, ${to} 50%, transparent 100%)` }
})


// 背景只有两种模式，这里的公式必须与 Rust `cover.rs` 保持一致
/** 模糊平铺的基准半径（px，1920×1080 画布）——与 Rust `cover.rs::COVER_BLUR_BASE_PX` 同源，
    改一边必须改另一边。 */
const COVER_BLUR_BASE_PX = 28
/** 模糊倍率 → { px, imgScale }：与 Rust `cover_blur_css` 同一套公式（含 0.2~4 夹取）。
    imgScale 用于把模糊溢出的边缘推出画面（blur 的扩散约等于半径的 3 倍）。 */
function coverBlurCss(scale) {
  const s = Math.min(4, Math.max(0.2, Number(scale) || 1))
  const px = COVER_BLUR_BASE_PX * s
  return { px, imgScale: Math.round((1 + px / 150) * 1000) / 1000 }
}

/** 背景模糊层（出片同一张 `_cover_frame.jpg`）的样式；帧层在它之上，互不影响。 */
const bgBlurStyle = computed(() => {
  const { px, imgScale } = coverBlurCss(blurScale.value)
  return {
    filter: `blur(${(px * (stageScale.value || 1)).toFixed(1)}px)`,
    transform: `scale(${imgScale})`,
    transformOrigin: 'center center'
  }
})
// 背景层的图源：blur 用关键帧 `_cover_frame.jpg`，image 用导入的 `_cover_bg.jpg`
// —— 两个文件分开，导入背景不会盖掉「关键帧层」（与 Rust 的 COVER_FRAME_IMG / COVER_BG_IMG 同源）
const bgImageSrc = computed(() => {
  if (!props.dir) return ''
  if (bgMode.value === 'image') return clipMediaUrl(props.pid, props.dir, '_cover_bg.jpg') + '?t=' + bgStamp.value
  return clipMediaUrl(props.pid, props.dir, '_cover_frame.jpg') + '?t=' + frameStamp.value
})

/** 帧层的图源 = `_cover_frame.jpg`（**与出片模板同源**，见模板里那段注释）。
 *  与背景层 blur 模式读的是**同一个文件**，但这是两层各自的消费点，别合并成一份状态：
 *  背景层缺图只是「模糊底没出来」，帧层缺图是「封面主体没了」，两者的兜底与提示都不同。 */
const frameImageSrc = computed(() => {
  if (!props.dir) return ''
  return clipMediaUrl(props.pid, props.dir, '_cover_frame.jpg') + '?t=' + frameStamp.value
})

/**
 * 帧层图缺失（`_cover_frame.jpg` 不在）。
 * ⚠️ 与背景层那条的差别：**这里不能只补一次就罢手** —— 帧层是封面主体，缺了整张封面就是空的，
 * 必须按 `[cover] frame_pos` 把关键帧补出来（与出片 `cover.rs` 自己截的那一帧**同一个时间点**）。
 * 补过了还失败就置 `frameImageOk=false` 并提示用「导出关键帧」手动指定一帧。
 */
async function onFrameImageError() {
  frameImageOk.value = false
  if (frameFrameTried.value) return
  frameFrameTried.value = true
  try {
    // 不带 time ⇒ 服务端按 `[cover] frame_pos` 截，与出片自己要截的是同一帧
    await coverFrame(props.pid, props.dir)
    frameStamp.value++
    frameImageOk.value = true
  } catch (e) {
    console.warn('帧层关键帧生成失败', e)
  }
}

/**
 * 背景截帧（`_cover_frame.jpg`）**平时不存在**：出片写完 cover.jpg 就把它删了，
 * 所以「当前帧模糊」一进设计器基本必然 404 → 图被 v-if 摘掉，看起来"没运行"。
 * 这里补一次：不带 time 调截帧接口，服务端按 `[cover] frame_pos` 截 —— 与出片自己要截的
 * 是**同一帧**，不会偷偷改渲染用的时间点。补过还失败就不再重试（避免死循环），给一句提示。
 */
async function onBgImageError() {
  bgImageOk.value = false
  // image 模式缺图 = 还没导入，别去补关键帧（那是另一层的东西）
  if (bgMode.value === 'image' || bgFrameTried.value) return
  bgFrameTried.value = true
  try {
    await coverFrame(props.pid, props.dir)
    frameStamp.value++
    bgImageOk.value = true
  } catch (e) {
    console.warn('生成背景截帧失败', e)
  }
}

// 换背景模式时重试一次（上一次的失败状态不该黏住）
watch(bgMode, () => {
  bgImageOk.value = true
  bgFrameTried.value = false
})

/** 手填 16 进制码：认 #rgb / #rrggbb（不带 # 也认），展开成小写 #rrggbb；非法就退回原值。 */
function commitBgColorText() {
  const raw = String(bgColorText.value || '').trim()
  const hex = raw.startsWith('#') ? raw.slice(1) : raw
  if (!/^[0-9a-fA-F]+$/.test(hex) || (hex.length !== 3 && hex.length !== 6)) {
    toast('颜色要写 16 进制码，如 #1a2b3c 或 #abc', 'err')
    bgColorText.value = bgColor.value
    return
  }
  const full = hex.length === 3 ? hex.split('').map((c) => c + c).join('') : hex
  const next = ('#' + full).toLowerCase()
  bgColorText.value = next
  bgColor.value = next
}

// 取色器改颜色时同步到文本框（文本框 → 取色器走 commitBgColorText）
watch(bgColor, (v) => { bgColorText.value = v }, { immediate: true })

// 帧层（当前帧）的位移/缩放：与后端 cover.rs::cover_frame_vars 同一套公式 ——
// 原点取 layout.liver 的聚焦点，基准 1.4（liver 在右半边）/ 1.3，再乘「帧缩放」。
const GRID_CENTERS = {
  'top-left': '16.67% 16.67%', top: '50% 16.67%', 'top-right': '83.34% 16.67%',
  'mid-left': '16.67% 50%', center: '50% 50%', 'mid-right': '83.34% 50%',
  'bot-left': '16.67% 83.33%', bottom: '50% 83.33%', 'bot-right': '83.34% 83.33%'
}

/**
 * 主播（`layout.liver`）在画面里的百分比中心。优先序与 `vtrim-render-core::region_origin`、
 * `webui/src/polish/engine.js::regionOrigin` **完全一致**：
 * **自由选框（画过框 → 框中心）> origin_grid > origin > grid[0]**，都没有 → `65% 50%`。
 * 帧层拿它当聚焦原点；**emoji 默认就贴在这个点上**（那个点是帧层变换的不动点，缩放帧也不跑偏）。
 * ⚠️ 以前这里漏了自由选框：画过人物框之后，出片按框中心聚焦，预览还按九宫格 —— 又一处不一致。
 */
function liverOrigin() {
  const reg = layout.value?.liver || {}
  const tl = reg.top_left
  const br = reg.bottom_right
  if (tl && br) {
    const x0 = parseFloat(String(tl.x ?? ''))
    const y0 = parseFloat(String(tl.y ?? ''))
    const x1 = parseFloat(String(br.x ?? ''))
    const y1 = parseFloat(String(br.y ?? ''))
    // 与 Rust `free_rect` 同样的有效性判断（框太小 = 没框）
    if ([x0, y0, x1, y1].every(Number.isFinite) && Math.abs(x1 - x0) >= 0.5 && Math.abs(y1 - y0) >= 0.5) {
      return { x: (x0 + x1) / 2, y: (y0 + y1) / 2 }
    }
  }
  let origin = ''
  if (reg.origin_grid && GRID_CENTERS[reg.origin_grid]) {
    origin = GRID_CENTERS[reg.origin_grid]
  } else if (reg.origin) {
    origin = reg.origin
  } else if (reg.grid && reg.grid.length && GRID_CENTERS[reg.grid[0]]) {
    origin = GRID_CENTERS[reg.grid[0]]
  }
  const parts = String(origin).split(/\s+/)
  const x = parseFloat(parts[0])
  const y = parseFloat(parts[1])
  return { x: Number.isFinite(x) ? x : 65, y: Number.isFinite(y) ? y : 50 }
}

const mediaStyle = computed(() => {
  const { x: baseX, y: baseY } = liverOrigin()
  const baseZoom = baseX >= 50 ? 1.4 : 1.3
  const zoom = (baseZoom || 1) * (coverZoom.value || 1)
  return {
    transformOrigin: baseX + '% ' + baseY + '%',
    transform: `translate(${coverOffsetX.value || 0}px, ${coverOffsetY.value || 0}px) scale(${zoom})`
  }
})

/** 帧复位：缩放回 100%、偏移归零（拖飞了能一键回来）。 */
function resetFrame() {
  coverZoom.value = 1
  coverOffsetX.value = 0
  coverOffsetY.value = 0
}

function takeCurrent() {
  const v = videoRef.value
  if (v) time.value = Math.round((v.currentTime || 0) * 10) / 10
}

async function exportFrame() {
  try {
    const at = Number(time.value) || 0
    await coverFrame(props.pid, props.dir, at)
    // ⚠️ **必须把时间点写回 `[cover] frame_pos`**：出片只在 `_cover_frame.jpg` 不存在时才按
    // frame_pos 重新截帧，文件在的时候**直接用这张图** —— 所以只要写回这个比例，
    // 设计器截的这一帧与出片用的那一帧就永远是同一张（不写回的话：换了帧、出片还是老位置）
    if (duration.value > 0) {
      const pos = Math.min(1, Math.max(0, at / duration.value))
      framePos.value = pos
      await coverConfig(props.pid, props.dir, { frame_pos: pos })
    }
    // 出片的模糊背景就是这张文件：去掉缓存让预览立刻换成新截的那一帧
    frameStamp.value++
    bgImageOk.value = true
    frameImageOk.value = true
    toast('关键帧已导出')
  } catch (e) { toast(e.message) }
}

async function importBg(e) {
  const file = e.target.files[0]
  if (!file) return
  const reader = new FileReader()
  reader.onload = async () => {
    try {
      const base64 = String(reader.result).split(',')[1]
      await coverBackground(props.pid, props.dir, base64)
      // 服务端写的是 `_cover_bg.jpg` + bg_mode=image：本地同步切过去并刷新图
      bgMode.value = 'image'
      bgStamp.value++
      bgImageOk.value = true
      toast('背景已导入（关键帧层不受影响）')
    } catch (err) { toast(err.message) }
  }
  reader.readAsDataURL(file)
}

async function generateTitleByLlm() {
  if (generatingTitle.value) return
  generatingTitle.value = true
  try {
    const r = await coverTitle(props.pid, props.dir)
    const lines = r?.title_lines || []
    if (lines.length) {
      titleLines.value = lines
      // 标题内容整体换了一批 → 重新排一次并放回安全区（行数/字数变了，旧字号不再适用）。
      // 这是"标题被获取"的第二种情形；手动改文字/拖滑杆**不走这里**。
      await fitTitleIntoSafeArea()
    } else {
      toast('LLM 未返回标题')
    }
  } catch (e) {
    toast('LLM 生成标题失败：' + (e.message || e), 'err')
  } finally {
    generatingTitle.value = false
  }
}

function addHighlightKeyword() {
  const kw = highlightInput.value.trim()
  if (!kw) return
  if (!highlightKeywords.value.includes(kw)) {
    highlightKeywords.value = [...highlightKeywords.value, kw]
  }
  // 手一改就是"封面自己那份"了（否则 payload 为空、出片仍旧走回退）
  hlFromFilter.value = false
  highlightInput.value = ''
}

function removeHighlightKeyword(i) {
  highlightKeywords.value.splice(i, 1)
  hlFromFilter.value = false
}

function coverConfigPayload() {
  return {
    // 关键帧时间点：与 `_cover_frame.jpg` **成对**保存 —— 只存图不存点，下次出片若重新截帧
    // （文件被删/换工程）就会用回旧比例，图与点脱钩
    frame_pos: framePos.value,
    title_lines: titleLines.value,
    text_width: textWidth.value / 100,
    text_x: textX.value + '%',
    text_y: textY.value + '%',
    font_size: fontSize.value,
    // 封面字体：字体库里的 id，空串 = 系统字体栈（cover.rs 就是这么解释的）
    font: coverFont.value,
    stroke: stroke.value,
    hl_color: hlColor.value,
    hl_font_scale: Number(hlFontScale.value) || 1,
    // 高亮词：**还在沿用 `[filter] highlight` 时就照旧写空数组**（空 = 出片回退用它），
    // 这样"打开一次设计器"不会把继承关系冻成封面自己的（用户以后在设置页改关键词，封面还能跟着走）。
    // 用户一增删，`hlFromFilter` 变 false，写的就是封面自己那份。
    highlight_keywords: hlFromFilter.value ? [] : highlightKeywords.value,
    letter_spacing: letterSpacing.value + 'px',
    line_height: lineHeight.value,
    text_shadow: textShadow.value,
    shadow_dir: textShadow.value ? 'center' : 'none',
    shadow_intensity: textShadow.value ? 1 : 0,
    gradient_dir: gradientDir.value,
    vignette: vignette.value,
    text_align: align.value,
    effects: fxList.value.map(f => ({
      // 稳定 id：`[cover] layers` 用 `fx:<id>` 指向这一条（老数据没 id 的，读回时补一个）
      id: f.uid,
      kind: f.type,
      amount: Number(f.amount) || 0.3,
      x: Number(f.x) || 50,
      y: Number(f.y) || 50,
      color: f.color || '#000000',
      // 只有 emoji 用；服务端渲染前还会再转义一次
      text: f.type === 'emoji' ? String(f.text || '').slice(0, 12) : ''
    })),
    // 图层栈（从下到上）：**顺序与可见性只住在这里**。服务端按它注入 z-index，
    // 老工程没有这段 = 默认栈（成片逐字节不变）。
    layers: stack.value.map(l => ({ id: l.id, visible: l.visible !== false })),
    bg_mode: bgMode.value,
    // 颜色只接受 #rrggbb（服务端还会再净化一次）：它是直接进 HTML 模板的字符串
    bg_color: /^#[0-9a-fA-F]{6}$/.test(bgColor.value) ? bgColor.value.toLowerCase() : '#000000',
    blur_scale: Math.min(4, Math.max(0.2, Number(blurScale.value) || 1)),
    // 帧层（当前帧）的位移/缩放：出片写进 #frame img 的 transform，与背景层无关
    bg_x: Math.round((coverOffsetX.value || 0) / (stageScale.value || 1) * 10) / 10,
    bg_y: Math.round((coverOffsetY.value || 0) / (stageScale.value || 1) * 10) / 10,
    bg_zoom: coverZoom.value || 1
  }
}

async function persistCoverConfig() {
  if (!configLoaded.value) return
  if (savingConfig.value) { saveConfigPending = true; return }
  savingConfig.value = true
  try {
    await coverConfig(props.pid, props.dir, coverConfigPayload())
    saveError.value = ''
  } catch (e) {
    // 以前只 `console.warn` + 一次 toast：600ms 防抖下连着改会刷屏，而且 toast 一闪就没了。
    // 改成和「保存中…」同一处的常驻文字（成功即清），失败就一直是它。
    saveError.value = e.message || String(e)
    console.warn('保存封面设置失败', e)
  } finally {
    savingConfig.value = false
    if (saveConfigPending) {
      saveConfigPending = false
      persistCoverConfig()
    }
  }
}

function scheduleCoverConfigSave() {
  if (!configLoaded.value) return
  if (saveConfigTimer) clearTimeout(saveConfigTimer)
  saveConfigTimer = setTimeout(() => {
    saveConfigTimer = null
    persistCoverConfig()
  }, 600)
}

watch(
  () => [
    JSON.stringify(titleLines.value),
    textX.value, textY.value, textWidth.value,
    fontSize.value, stroke.value,
    hlColor.value, JSON.stringify(highlightKeywords.value),
    // 封面字体（`[cover] font`）也走这条防抖保存 —— 换了字体不落盘，就只是设计器里看着变了
    coverFont.value,
    letterSpacing.value, lineHeight.value,
    textShadow.value,
    gradientDir.value, vignette.value,
    align.value,
    JSON.stringify(fxList.value),
    // 图层栈（顺序 + 可见性）：拖一下/点一下眼睛就走同一条 600ms 防抖保存
    JSON.stringify(stack.value),
    bgMode.value, bgColor.value, blurScale.value,
    coverOffsetX.value, coverOffsetY.value, coverZoom.value,
    // 关键帧时间点：导出关键帧/取当前帧改了它也要落盘（与图成对）
    framePos.value
  ],
  scheduleCoverConfigSave
)

// 「生成封面」直接出图（`@click="generateCover()"`）—— 原来这里有个选通道的弹窗，
// 通道只剩 Takumi 之后就删掉了。`loadCoverCap()` 仍由 `onMounted` 调（能力位用来置灰 + 说明原因）。

async function generateCover() {
  if (generating.value) return
  if (clipBusyJob.value) { toast('该片段正在' + kindText(clipBusyJob.value.kind) + '，请等它结束', 'err'); return }
  generating.value = true
  try {
    // 出片要截的关键帧 = 设计器里选的这一帧：**无条件**按当前进度刷新一次，并把时间点写回
    // `[cover] frame_pos`。
    // ⚠️ 这里原来是 `if (Number(time.value) > 0)`：用户没拖过进度条（time 恒为 0）时直接跳过，
    // 于是「我在设计器里选的这一帧」从来没进过产物，出片照旧用旧/默认 frame_pos 的那张图。
    // 0 秒是合法取值，不能拿它当「没设置」——与「音量 0 是合法值」是同一条教训。
    const frameAt = Number(time.value) || 0
    await coverFrame(props.pid, props.dir, frameAt)
    if (duration.value > 0) {
      framePos.value = Math.min(1, Math.max(0, frameAt / duration.value))
    }
    await coverConfig(props.pid, props.dir, coverConfigPayload())
    // 通道不传：只剩 Takumi 一个值，`cover.rs` 那条默认链（显式 > `VTEDIT_COVER_RENDERER` >
    // takumi）是唯一权威，前端不抄第二份。
    const coverRes = await runCover(props.pid, props.dir)
    if (coverRes && coverRes.id) {
      await new Promise((resolve, reject) => {
        // `watchJob()` 返回 `{ close() }` 对象（不是函数），别直接当函数调
        const watcher = watchJob(coverRes.id, {
          onDone(summary) {
            watcher.close()
            if (summary.status === 'succeeded') resolve()
            else reject(new Error(summary.error || summary.message || '封面生成失败'))
          }
        })
      })
    }
    emit('changed')
    toast('封面已生成')
  } catch (e) {
    console.error('生成封面失败', e)
    toast('生成封面失败：' + (e.message || e), 'err')
  } finally {
    generating.value = false
  }
}


function onVideoTime() {
  const v = videoRef.value
  if (v) time.value = Math.round((v.currentTime || 0) * 10) / 10
}
/** `frame_pos × 时长` 就是出片截帧的那个时间点（与 `cover.rs` / `api/polish.rs` 同一条公式）。
 *  ⚠️ 时长要等 `loadedmetadata` 才有，而配置可能先回来、也可能后回来 —— 两边都调一次，
 *  谁后到谁负责把进度条对上（`duration.value` 为 0 时什么都不做，等那次 loadedmetadata）。 */
function seekToFramePos() {
  const v = videoRef.value
  if (!v || !duration.value) return
  const t = Math.min(duration.value, Math.max(0, framePos.value * duration.value))
  v.currentTime = t
  time.value = Math.round(t * 10) / 10
}

/** 取不到视频源（片段被删/编码不支持）⇒ 退回静态关键帧，别留黑屏。 */
function onVideoError() {
  videoOk.value = false
}
function onMeta() {
  const v = videoRef.value
  if (!v) return
  videoOk.value = true
  duration.value = v.duration || 0
  // 元数据到手才知道时长 ⇒ 把进度条对到出片要截的那一帧（config 先到时由这里补）
  seekToFramePos()
}
function togglePlay() {
  const v = videoRef.value
  if (!v) return
  if (v.paused) v.play().catch(() => {})
  else v.pause()
}
function onSeekInput(e) {
  const v = videoRef.value
  if (!v || !duration.value) return
  const t = Number(e.target.value) || 0
  v.currentTime = t
  time.value = Math.round(t * 10) / 10
}
/** 舞台播放器能不能用。取不到视频（文件缺 / 编码坏）时置 false → 静态图兜底顶上，
 *  设计器照常可用（老形态），不会留一块黑。加载成功（loadedmetadata）即复位。
 *  ⚠️ 与「选中哪个层」**无关**：播放器是全局取景 UI（见模板注释里的口径修正）。 */
const videoOk = ref(true)
function updateStageScale() {
  const w = stageRef.value ? stageRef.value.clientWidth : 0
  // 基准 = 封面画布宽 1920：字号、描边、位移这些"画布 px"预览与出片必须同一把尺。
  stageScale.value = w > 0 ? w / COVER_W : 1
}

/** 画布拖动改哪一层 = **当前选中哪一层**（用户口径："选中哪个图层就可以手动拖动"）：
 *  · 视频预览 → `bg_x/bg_y`（画布 px）· 滚轮 = `bg_zoom`
 *  · 文字 → `text_x/text_y`（百分比，中心点）
 *  · 特效/表情 → `x/y`（百分比）· 滚轮 = `amount`（表情＝大小、其它＝强度）
 *  · 滤镜 / 背景 / 安全区 → 没有位置可拖（光标 not-allowed，`title` 里说明） */
const dragTarget = computed(() => {
  const id = selectedKey.value
  if (id === 'frame') return 'frame'
  if (id === 'title') return 'title'
  if (id && id.startsWith('fx:')) return 'fx'
  return 'none'
})
const dragHint = computed(() => {
  if (dragTarget.value === 'frame') return '拖动 = 移动「视频预览」这一层 · 滚轮 = 缩放'
  if (dragTarget.value === 'title') return '拖动 = 移动「文字」这一层（Shift 约束单轴）'
  if (dragTarget.value === 'fx') return '拖动 = 移动这一层 · 滚轮 = 改强度 / 大小'
  return '这一层没有位置可拖（滤镜 / 背景 / 安全区先选别的层）'
})

const clampPct = (v) => Math.round(Math.min(100, Math.max(0, v)) * 10) / 10
const selectedFx = () => (selectedKey.value.startsWith('fx:') ? fxList.value.find((f) => 'fx:' + f.uid === selectedKey.value) : null)

/** 拖动预览 = 移动**当前选中的那一层**；松手让自动保存链把它写进 polish.toml。 */
function onStagePointerDown(e) {
  if (e.button !== 0) return
  const target = dragTarget.value
  if (target === 'none') return
  e.preventDefault()
  const fx = target === 'fx' ? selectedFx() : null
  if (target === 'fx' && !fx) return
  const startX = e.clientX
  const startY = e.clientY
  const start = {
    ox: coverOffsetX.value,
    oy: coverOffsetY.value,
    tx: textX.value,
    ty: textY.value,
    fx: fx ? { x: Number(fx.x) || 0, y: Number(fx.y) || 0 } : null
  }
  let axis = ''   // Shift = 单轴约束（PS 手感）：锁定开始拖动那一刻的主方向
  const onMove = (ev) => {
    let dx = ev.clientX - startX
    let dy = ev.clientY - startY
    if (ev.shiftKey) {
      if (!axis) axis = Math.abs(dx) >= Math.abs(dy) ? 'x' : 'y'
      if (axis === 'x') dy = 0
      else dx = 0
    }
    if (target === 'frame') {
      coverOffsetX.value = start.ox + dx
      coverOffsetY.value = start.oy + dy
      return
    }
    const s = stageRef.value
    if (!s) return
    if (target === 'title') {
      textX.value = clampPct(start.tx + (dx / s.clientWidth) * 100)
      textY.value = clampPct(start.ty + (dy / s.clientHeight) * 100)
    } else if (fx) {
      fx.x = clampPct(start.fx.x + (dx / s.clientWidth) * 100)
      fx.y = clampPct(start.fx.y + (dy / s.clientHeight) * 100)
    }
  }
  const onUp = () => {
    window.removeEventListener('pointermove', onMove)
    window.removeEventListener('pointerup', onUp)
    window.removeEventListener('pointercancel', onUp)
    stageDrag.value = null
  }
  stageDrag.value = { onMove, onUp }
  window.addEventListener('pointermove', onMove)
  window.addEventListener('pointerup', onUp)
  window.addEventListener('pointercancel', onUp)
}

/** 滚轮 = 当前选中层的"量"：视频预览缩放（0.5~3，与接口写入侧夹取一致）/ 特效强度（0~1）。 */
function onStageWheel(e) {
  if (dragTarget.value === 'frame') {
    const factor = e.deltaY > 0 ? 0.9 : 1.1
    coverZoom.value = Math.min(3, Math.max(0.5, coverZoom.value * factor))
  } else if (dragTarget.value === 'fx') {
    const fx = selectedFx()
    if (!fx) return
    const step = e.deltaY > 0 ? -0.05 : 0.05
    fx.amount = Math.min(1, Math.max(0, Math.round(((Number(fx.amount) || 0) + step) * 100) / 100))
  }
}

// 右侧面板与左侧预览（含播放条）等高，且**必须给确定高度**（`height`，不是 `max-height`）：
// `height:auto` 的 flex 列没有"剩余空间"可分，属性面板会长成内容高度、图层列表跟着上下跳；
// 定了高之后 `.lyr-list` 的 `flex-basis:50%` 才有百分比基准 —— 列表被**钉在面板下半部**。
// 窄屏单列时解除限制（让内容自己撑）。
//
// ⚠️ 那是个**下限**（`CONTROLS_MIN_H`），不是纯粹"等于预览高度"：面板里放不下时能压的只有
// `.lyr-props`（下限 120px），`.lyr-list` 是 `flex:0 0 50%`（不可压）、工具条 `flex:none` ——
// 再挤就是 `.controls{overflow:hidden}` 把**最底下那条工具条**（"新建图层"按钮）裁掉，
// 而窗口越窄预览越矮、面板跟着越矮（预览高度 = 宽度 × 9/16）。实测：外层内容宽 ~936px 时
// 面板 288px 高、工具条被裁掉 46px（按钮整个看不见）。宁可让面板比预览长一点。
const CONTROLS_MIN_H = 380

function syncControlsBound() {
  const ctl = controlsRef.value
  if (!ctl) return
  if (!stageWrapRef.value || window.matchMedia('(max-width: 900px)').matches) {
    ctl.style.height = ''
    return
  }
  ctl.style.height = Math.max(stageWrapRef.value.offsetHeight, CONTROLS_MIN_H) + 'px'
}

/**
 * 读一份封面配置。**失败必须可见**：`configLoaded` 是写入门闸（读失败时拿默认值写回会把
 * 磁盘配置清成默认），它一停在 false，面板里所有改动就静默不保存了 —— 所以失败时摆出
 * 错误条 + 「重试」，而不是只写一行 console。
 *
 * 返回值 = **这次是不是「首次拿到标题」**（磁盘上没有 `title_lines`，只能靠 `project.name`
 * 回退拆行）—— 调用方据此决定要不要跑一次自动排版。
 */
async function loadCoverConfig(retrying = false) {
  try {
    const cfg = await fetchProject(props.pid, props.dir, false, '读取封面配置')
    layout.value = cfg?.layout || {}
    updateStageScale()
    syncControlsBound()
    videoSrc.value = clipMediaUrl(props.pid, props.dir, cfg?.source?.video || 'clip.mp4')
    // 标题行：与 `cover.rs::build_cover_html` 同一套回退规则 —— 没有 title_lines 时按标点
    // 把 `project.name` 拆行，并砍掉 `【主播名】` 后缀（以前这里直接用整串当一行，
    // 于是"设计器里一行、出片四行"）
    // ⚠️ 这个判据同时是「已有封面」的判据：磁盘上有 title_lines 就说明排过/存过，
    // 自动排版**一个字段都不碰**（用户要求：不动已有封面）。
    const hadSavedTitle = !!(cfg?.cover?.title_lines && cfg.cover.title_lines.length)
    projectName.value = cfg?.project?.name || ''
    titleLines.value = hadSavedTitle
      ? [...cfg.cover.title_lines]
      : splitTitleFallback(cfg?.project?.name || '')
    const parsePct = (v, fallback) => {
      const n = parseFloat(String(v ?? '').replace('%', ''))
      return Number.isFinite(n) ? n : fallback
    }
    textX.value = parsePct(cfg?.cover?.text_x, 35)
    textY.value = parsePct(cfg?.cover?.text_y, 50)
    textWidth.value = Math.round((cfg?.cover?.text_width ?? 0.62) * 100)
    fontSize.value = cfg?.cover?.font_size || 120
    coverFont.value = cfg?.cover?.font || ''
    // 关键帧的时间点：出片按 `frame_pos × 时长` 截帧，设计器必须用**同一个值**去 seek 舞台播放器，
    // 否则「打开设计器 = 第 0 秒那一帧、出片 = frame_pos 那一帧」（两边都不报错，只能比对产物发现）。
    // 播放器常驻舞台 ⇒ 这里 seek 立即可见；若元数据还没到（duration 为 0）则由 onMeta 补一次。
    framePos.value = Math.min(1, Math.max(0, Number(cfg?.cover?.frame_pos ?? 0.4)))
    seekToFramePos()
    if (coverFont.value) ensureFontFace(props.pid, props.dir, coverFont.value, { onDebug: (m) => addClient('debug', m, 'cover') })
    stroke.value = cfg?.cover?.stroke || 20
    hlColor.value = cfg?.cover?.hl_color || '#ffcc00'
    hlFontScale.value = Number(cfg?.cover?.hl_font_scale) || 1
    // 高亮词：与 `cover.rs::cover_highlight_words` **同一套回退** —— 封面自己的为空时用
    // `[filter] highlight`（「设置 · 关键词过滤」里那串）。以前只显示封面那份，于是"出片在给标题上高亮、
    // 面板里却是空的"（用户报过「高亮词没有实际使用上」）。这里照实显示，改一下就固化。
    const coverHl = Array.isArray(cfg?.cover?.highlight_keywords) ? cfg.cover.highlight_keywords.filter(Boolean) : []
    const filterHl = Array.isArray(cfg?.filter?.highlight) ? cfg.filter.highlight.filter(Boolean) : []
    highlightKeywords.value = coverHl.length ? [...coverHl] : [...filterHl]
    hlFromFilter.value = !coverHl.length && filterHl.length > 0
    letterSpacing.value = parseFloat(String(cfg?.cover?.letter_spacing ?? '8').replace('px', '')) || 8
    lineHeight.value = cfg?.cover?.line_height ?? 1.7
    textShadow.value = cfg?.cover?.text_shadow ?? !(cfg?.cover?.shadow_dir === 'none' || (cfg?.cover?.shadow_intensity ?? 1) <= 0)
    gradientDir.value = cfg?.cover?.gradient_dir || 'left'
    vignette.value = cfg?.cover?.vignette ?? 0.08
    // 默认左对齐 —— 与后端 `default_cover_text_align()`、默认预设「左文」保持一致
    align.value = cfg?.cover?.text_align || 'left'
    // 渐变底色的色相来自**字幕**的高亮色（`cover.rs::build_cover_html` 就是这么取的），
    // 不是封面自己的 hl_color —— 预览必须跟它同源，否则出片的渐变跟设计器不是一个色调
    subtitleHlColor.value = cfg?.style?.subtitle?.highlight_color || ''
    // 背景模式：只有 color / blur，其它值（含老配置的 frame）一律纯色，与 Rust 侧同一套规则
    const mode = String(cfg?.cover?.bg_mode || '').toLowerCase()
    bgMode.value = mode === 'blur' ? 'blur' : (mode === 'image' ? 'image' : 'color')
    bgColor.value = /^#[0-9a-fA-F]{6}$/.test(cfg?.cover?.bg_color || '') ? cfg.cover.bg_color.toLowerCase() : '#000000'
    blurScale.value = Math.min(4, Math.max(0.2, Number(cfg?.cover?.blur_scale) || 1))
    // 帧层（当前帧）的位移/缩放：服务端存的是 1920 画布上的 px，预览按舞台缩放到显示尺寸
    coverOffsetX.value = (Number(cfg?.cover?.bg_x) || 0) * stageScale.value
    coverOffsetY.value = (Number(cfg?.cover?.bg_y) || 0) * stageScale.value
    coverZoom.value = Math.min(3, Math.max(0.5, Number(cfg?.cover?.bg_zoom) || 1))
    if (Array.isArray(cfg?.cover?.effects)) {
      fxList.value = cfg.cover.effects.map(e => ({
        // 稳定 id（落盘进 `[[cover.effects]].id`，栈用 `fx:<id>` 引用它）：
        // 老数据没有 id → 这里补一个，落盘后栈就认得它了
        uid: String(e.id || '').trim() || newFxId(),
        type: e.kind || e.type,
        amount: e.amount ?? 0.3,
        x: e.x ?? 50,
        y: e.y ?? 50,
        color: e.color || '#000000',
        text: e.text || ''
      }))
    }
    // 图层栈：照 `cover.rs::resolve_cover_stack` 的同一套容错解出来（缺项补齐 / 未知忽略 / 重复只认第一次）。
    // 老工程没有 `layers` → 得到默认栈（与改造前的出片序一致）。
    stack.value = resolveStack(cfg?.cover?.layers)
    // 默认选「文字」（最常改的就是它）；它不在栈里就落到栈顶那一层
    if (!layers.value.some((l) => l.key === selectedKey.value)) {
      selectedKey.value = contentLayers.value.find((l) => l.key === 'title')?.key || contentLayers.value[0]?.key || 'frame'
    }
    configLoaded.value = true
    configLoadError.value = ''
    return !hadSavedTitle
  } catch (e) {
    // 读不到 → 保持未加载（写入门闸继续关着），但**必须说出来**：不说的话用户以为改动存上了，
    // 实际上这个面板从此不再往磁盘写任何东西。
    configLoaded.value = false
    configLoadError.value = e.message || String(e)
    addClient('warn', '封面配置读取失败，改动不会被保存：' + configLoadError.value, 'cover')
    if (retrying) toast('封面配置仍然读取失败：' + configLoadError.value, 'err')
    return false
  }
}

/** 错误条上的「重试」：读成功且这次是首次拿到标题时，照样排一次（失败那轮什么都没写盘） */
async function reloadCoverConfig() {
  if (await loadCoverConfig(true)) await fitTitleIntoSafeArea()
}

onMounted(async () => {
  updateStageScale()
  syncControlsBound()
  resizeRo = new ResizeObserver(() => { updateStageScale(); syncControlsBound() })
  if (stageWrapRef.value) resizeRo.observe(stageWrapRef.value)
  videoSrc.value = clipMediaUrl(props.pid, props.dir, 'clip.mp4')
  // 舞台播放器**常驻**（与选中哪个层无关）：loadedmetadata 触发 onMeta 拿到 duration，
  // 再由 seekToFramePos 把播放头对到 frame_pos —— 打开设计器看到的就是出片要截的那一帧。
  // 「新建图层」菜单：点空白处关掉。捕获阶段监听，点别的按钮时先关菜单再走它自己的逻辑
  document.addEventListener('pointerdown', onDocPointerDown, true)
  loadFonts()
  // 首次拿到标题（磁盘上没有 title_lines）→ 把排版调好并放进 4:3 安全区。
  // 排完的改动由下面那个 600ms 防抖 watch 自动落盘，所以只跑这一次。
  if (await loadCoverConfig()) await fitTitleIntoSafeArea()
})

onUnmounted(() => {
  if (saveConfigTimer) {
    clearTimeout(saveConfigTimer)
    saveConfigTimer = null
  }
  if (configLoaded.value) persistCoverConfig()
  document.removeEventListener('pointerdown', onDocPointerDown, true)
  if (resizeRo) {
    resizeRo.disconnect()
    resizeRo = null
  }
  // 拖到一半被卸载（切片段/关页面）时把 window 上的监听摘掉，别留着往空组件写
  if (stageDrag.value) {
    window.removeEventListener('pointermove', stageDrag.value.onMove)
    window.removeEventListener('pointerup', stageDrag.value.onUp)
    window.removeEventListener('pointercancel', stageDrag.value.onUp)
    stageDrag.value = null
  }
})
</script>

<style scoped>
/* 自动排版放不下时的常驻提示（琥珀色 = 警告，不是错误：结果仍然可用）。
   保存失败/保存中在面板头那一行的 `.save-line`（600ms 防抖下用 toast 会刷屏，而且一闪就没了）。 */
.fit-hint { color:var(--amber-deep); }
/* 列宽比例（用户要求：封面设计区小一点、右侧面板宽一点）：右栏 = `minmax(440px, 40%)`。
   440px 是**下限**（图层行的「拖柄/眼睛/缩略/名字/徽标/垃圾桶」与文字面板的"两列一行"要放得下，
   改前是 372px，一格只有 ~170px、标签会折行）；宽屏按 40% 跟着长（1920 窗口 ≈ 630px）。
   左栏是 `1fr`，右栏一变宽预览就自动缩小 —— 预览高度 = 宽度 × 9/16，所以它同时决定右栏的高度基准
   （`syncControlsBound`，还带一个 `CONTROLS_MIN_H` 下限），别只改一边。
   实测（外层内容宽 1146px，harness 量 `gridTemplateColumns`）：改前 755/376 → 改后 672/458
   （预览 −11%，面板 +22%）。 */
.designer-grid { display:grid; grid-template-columns:minmax(0,1fr) minmax(440px, 40%); gap:16px; align-items:start; }
/* 右栏 = 图层组列表 + 选中层的属性面板（PS 式）：两块**各自滚动**，
   整列高度由 JS（`syncControlsBound`）限到与左侧预览等高（预览矮到放不下时给 `CONTROLS_MIN_H` 下限）
   —— 所以这里是 overflow:hidden，滚动交给里面的 .lyr-list / .lyr-props，
   别再给这一层加 overflow-y:auto（会长出两条滚动条）。 */
.controls { display:flex; flex-direction:column; gap:8px; min-width:0; overflow:hidden; }
/* 标题文本框：5 行固定高度由 `rows="5"` + 这里的 line-height 决定（不随内容长高），
   resize:none 去掉拖拽手柄。别再往这里加 font-size/padding 覆盖 ——
   字段外观用设计基准 .fld-input（app.css：bg-soft + 6px/10px + 12.5px）。 */
.title-lines { line-height:1.5; resize:none; }
.mini-row { display:flex; align-items:center; gap:6px; min-width:0; }
/* 分段切换（背景模式）：三个按钮等宽铺满一行，别用 .mini-field（那是一条「标签+控件」的窄行） */
.mini-label { flex:none; font-size:11px; color:var(--ink-dim); white-space:nowrap; }
.seg-btn { flex:1; justify-content:center; padding:3px 0; }
.mini-grid { display:grid; grid-template-columns:1fr 1fr; gap:4px 10px; }
/* 属性面板里的**分区标题**（用户样稿：位置大小 / 排版 / 高亮 tag）：小字 + 一条发丝线 */
.lyr-sec { display:flex; align-items:center; gap:6px; margin-top:10px;
  font-size:10.5px; font-weight:700; color:var(--ink-faint); letter-spacing:.04em; }
.lyr-sec::after { content:''; flex:1; height:1px; background:var(--line); }
/* 文字属性：左文本框 / 右一列紧凑控件（**各占 50%**，用户要求：以前右列钉死 168px）；
   下面两列一行（对齐·行距 / 描边·字距）。两侧都用 `minmax(0,·)` —— `1fr` 的自动下限是 auto，
   文本框的固有宽度会把轨道顶出去。 */
.txt-top { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr); gap:8px; align-items:start; }
.txt-side { display:flex; flex-direction:column; gap:4px; min-width:0; }
.txt-side > .mini-btn { justify-content:center; }
.txt-grid { display:grid; grid-template-columns:1fr 1fr; gap:4px 10px; align-items:center; }
/* 行尾的**可填**读数（紧凑版 `SliderField`）：常态与只读的 `.rv` 同貌，hover/focus 才现出输入框。
   为什么不用 `SliderField.vue`：那边 `@container (max-width:220px)` 会把标签挤到第二行，
   而这里的格子只有 ~170px（一行两列）—— 一样的"滑块 + 可填数字"，只是更紧凑。 */
.rv-input { flex:none; width:46px; padding:1px 3px; text-align:right; font-size:10.5px; font-family:monospace;
  color:var(--ink); background:transparent; border:1px solid transparent; border-radius:var(--r-sm); }
.rv-input:hover { border-color:var(--line); background:var(--card); }
.rv-input:focus { border-color:var(--main); box-shadow:0 0 0 3px var(--main-soft); }
/* 手填允许落在 step 栅格之外（那是"可填"的意义），别让浏览器把它画成非法状态 */
.rv-input:invalid { box-shadow:none; }
.rv-input:invalid:focus { box-shadow:0 0 0 3px var(--main-soft); }
/* spinner 在 46px 宽里会把数字挤出去：步进交给 ↑/↓ 键 */
.rv-input::-webkit-outer-spin-button, .rv-input::-webkit-inner-spin-button { -webkit-appearance:none; margin:0; }
.rv-input { -moz-appearance:textfield; appearance:textfield; }
.hl-note { line-height:1.5; }
.mini-field { display:flex; align-items:center; gap:5px; min-width:0; }
.mini-field > label { flex:none; margin:0; font-size:11px; color:var(--ink-dim); white-space:nowrap; }
.mini-field > input[type=range] { flex:1; min-width:0; padding:0; height:18px; background:transparent; border:none; box-shadow:none; accent-color:var(--main); cursor:pointer; }
.mini-field > select { flex:1; min-width:0; width:auto; padding:2px 6px; font-size:11.5px; border-radius:6px; }
.mini-field > input[type=color] { flex:1; min-width:0; width:auto; height:22px; padding:1px 2px; border-radius:6px; }
/* 手填 16 进制码：与取色器并排，窄列也能看清 6 位 */
.hex-input { flex:1; min-width:0; padding:2px 6px; font-size:11.5px; font-family:monospace; }
.mini-field.check { gap:4px; font-size:11.5px; color:var(--ink-dim); cursor:pointer; white-space:nowrap; }
.mini-field.check input[type=checkbox] { width:auto; margin:0; flex:none; }
.kw-input { flex:1; min-width:0; padding:4px 8px; font-size:12px; }
.grow { flex:1; min-width:0; }
.rv { flex:none; min-width:32px; text-align:right; font-size:10.5px; color:var(--ink-faint); font-family:monospace; }
/* ⚠️ 内容层的 z-index **不在这里写**：它来自图层栈（`layerStyle()` 的行内样式，与出片侧
   `cover.rs::plan_cover_layers` 同一套规则）。这里写死一个会让"列表里这么排、成片里那么画"。
   只有**设计器专属**的两层（拖拽层 / 安全区线框）固定压在最上面。 */
.stage { position:relative; width:100%; aspect-ratio:16/9; background:#000; border-radius:14px; overflow:hidden; }
.stage-bg { position:absolute; inset:0; overflow:hidden; }
.stage-bg-img { display:block; width:100%; height:100%; object-fit:cover; transform-origin:center center; }
/* 帧层容器：z-index 走行内（图层栈），这里只摆位置 */
.stage-media { position:absolute; inset:0; overflow:hidden; }
/* object-fit:contain —— 素材**保持原比例**完整显示（与出片模板 #frame img 同规则）：
   竖屏素材在 16:9 封面上左右留边（露出背景层），不再被 cover 裁掉上下。
   ⚠️ `background:transparent` / `border-radius:0` 是**必需的覆盖**，不是装饰：
   `styles/app.css` 有一条全局元素选择器 `video { … background:#000; border-radius:var(--r-sm) }`
   （给剪辑卡片那类视频缩略图用的）。类选择器只覆盖自己写了的属性，这两条会**穿透**到本元素：
   contain 让出来的左右留白区由**元素自身背景**填充 → 竖屏素材在设计器里是两条**纯黑边**，
   还压在 `.stage-bg`（模糊背景层）之上，把背景层整个盖住（出片用 <img>、无 app.css，所以只有
   设计器这样 —— 用户报过"设计器素材左右黑边、出片正常"）。改这段前先跑
   `node webui/scripts/check-cover-stage.mjs`。 */
/* 帧层的图 = `_cover_frame.jpg`，与出片模板 #frame img **同一条规则**（object-fit:contain 保持原比例）。
   ⚠️ 这里**不需要**那条 `background:transparent` 覆盖（那是给 `video` 元素用的，`app.css` 的全局
   规则只命中 `video{…}`）—— 但必须是 contain，否则竖素材会被裁掉。 */
.stage-frame-img { position:absolute; inset:0; width:100%; height:100%; object-fit:contain; display:block; }
/* 舞台内实况播放器 = 帧层本体（**常驻**，与选中哪个层无关；用户直接在舞台上播/拖找帧）：
   盒子与静态兜底帧完全一致（inset:0 + contain + mediaStyle 变换共用）—— 切换时除内容外零跳变。
   ⚠️ `background:transparent` / `border-radius:0` 是**必需的覆盖**，不是装饰：
   `styles/app.css` 全局 `video{}`（黑底/圆角/max-height:170px/margin-top:6px 的卡片缩略图口径）
   会穿透 —— 黑边会盖住下层模糊背景（用户报过"设计器素材左右黑边、出片正常"）。 */
.stage-video-live { position:absolute; inset:0; width:100%; height:100%; object-fit:contain;
  background:transparent; border-radius:0; max-height:none; margin:0; display:block; }
/* 特效/表情：**每个特效一个槽位**（`.fx-slot` 承载 z-index，内层 `.fx-item` 才是效果本身）——
   与出片那份 HTML 同构：`overlay` 的 `isolation` 只加在它自己的槽位上（保持改造前"不真混合"的观感），
   `blur` 的槽位**不能**加 isolation（它的 backdrop-filter 要靠下层的画面做背景）。
   ⚠️ `.fx-layer` 不能有 z-index：一有就是 stacking context，槽位的 z-index 全被锁死在一层里。 */
.fx-layer { position:absolute; inset:0; pointer-events:none; }
.fx-slot { position:absolute; inset:0; }
.fx-slot-iso { isolation:isolate; }
.fx-item { position:absolute; inset:0; }
/* 表情特效：不是铺满的图层，而是一个点 —— 位置/字号/阴影走行内样式，居中在这。
   字体栈与出片模板（cover.rs 的 EMOJI_FONT）保持一致，别落到中文字体上。 */
.fx-item.fx-emoji { inset:auto; transform:translate(-50%,-50%); line-height:1;
  font-family:'Segoe UI Emoji','Apple Color Emoji','Noto Color Emoji',sans-serif; }
.emoji-picks { display:flex; flex-wrap:wrap; gap:4px; }
.emoji-pick { background:var(--bg-soft); border:1px solid var(--line); border-radius:6px;
  font-size:15px; line-height:1; padding:3px 6px; cursor:pointer; }
.emoji-pick:hover { border-color:var(--main); }
/* 渐变 / 暗角 / 标题：z-index 一律走行内（图层栈）。
   ⚠️ 别再在这里写死图层序 —— 出片的序由 `[cover] layers` 决定，这里写死就是又一次"两边不一样"。 */
.vignette-layer { position:absolute; inset:0; pointer-events:none; background:radial-gradient(ellipse at 55% 45%, transparent 50%, rgba(0,0,0,1) 100%); }
.gradient-layer { position:absolute; inset:0; pointer-events:none; }
.title-wrap { position:absolute; width:62%; transform:translate(-50%,-50%); text-align:center; pointer-events:none; }
/* 拖拽/滚轮层压在最上面（标题是 pointer-events:none，不挡操作）。
   `grab-none` = 当前选中的那层没有位置可拖（滤镜/背景/安全区）→ 光标提示"拖不动"。 */
.stage-gesture-layer { position:absolute; inset:0; z-index:900; cursor:grab; }
.stage-gesture-layer:active { cursor:grabbing; }
.stage.grab-none .stage-gesture-layer { cursor:not-allowed; }
/* 安全区线框：只是辅助线，不参与"设计器 ⇄ 出片镜像"那一套 —— 所以线宽与字号用**预览 px**
   （不乘 stageScale），也不做任何 JS 换算，纯 CSS 相对尺寸跟着舞台一起缩放。
   白线 + 1px 深色外圈（box-shadow 扩张）：封面素材亮暗不定，单色线总会在某一类画面上糊掉。 */
.guide-layer { position:absolute; inset:0; pointer-events:none; z-index:950; }
.guide-box { position:absolute; left:50%; top:50%; transform:translate(-50%,-50%);
  border:1px solid rgba(255,255,255,.85); box-shadow:0 0 0 1px rgba(0,0,0,.45); }
/* 16:9 安全区 = 等比缩到 95%（1920×1080 → 1824×1026）。别写成 `inset:5%` ——
   CSS 的 inset 百分比横向按父宽、纵向按父高解析，看着像等比其实不是。 */
.guide-169 { width:95%; height:95%; }
/* 4:3 裁切区 = 满高居中（1080×4/3 = 1440 宽）。宽度交给 aspect-ratio 算，不用 JS。 */
.guide-43 { height:100%; aspect-ratio:4/3; }
/* 标签放框**内侧**左上角：4:3 框是满高，框外没有空间 */
.guide-tag { position:absolute; left:4px; top:4px; font-size:10px; line-height:1.5; padding:0 5px;
  border-radius:999px; background:rgba(0,0,0,.55); color:#fff; white-space:nowrap; }
.drag-hint { font-size:10.5px; color:var(--ink-faint); white-space:nowrap; cursor:default; }
/* ⚠️ 标题**不做自动换行**（用户要求）：`pre-line` 保留文本框里的换行、`width:max-content` 让盒子长到内容宽
   —— 于是永不折行，与出片模板 `#title{white-space:nowrap}` 同口径（模板那边靠 `<br>` 断行）。
   文字超宽就溢出（`.stage{overflow:hidden}` 与出片 `#page` 一样裁掉）—— 要放进画面就手动断行或缩字号。 */
.title-text { color:#f0e6d3; font-weight:900; line-height:1.7; paint-order:stroke fill; white-space:pre-line; width:max-content; }
/* ⚠️ 高亮词必须用 `:deep()`：`.hl` 那段 HTML 是 `highlightTitleLine()` 拼出来、经 **`v-html`** 注入的，
   Vue 的 scoped 编译**不会**给 v-html 内容打 `data-v-xxx` —— 写成 `.title-text .hl` 编译成
   `.title-text .hl[data-v-xxx]`，**永远匹配不上**（实测 dist CSS 就是这么生成的）。
   症状很具迷惑性：字体照旧会变（`fontFamily` 在父级 `.title-text` 上，span 继承），只有**高亮色**
   静默失效、退成继承来的 `#f0e6d3` —— 用户报的「关键词字体被改了、高亮色没显示」。
   出片侧没这个问题：`cover.rs` 注入 <span class="hl"> 到模板的**全局** <style> 里。 */
.title-text :deep(.hl) { color:var(--hlc, #ffcc00); font-size:var(--hl-fs, 1em); -webkit-text-stroke:inherit; paint-order:stroke fill; }
.seek-row { display:flex; align-items:center; gap:8px; }.seek-range { flex:1; min-width:0; padding:0; }
.seek-time { flex:none; min-width:120px; text-align:right; }
.w-time { width:90px; flex:none; }
.hidden-file { display:none; }
.hl-chips { display:flex; flex-wrap:wrap; gap:6px; }
.hl-chip { display:inline-flex; align-items:center; gap:4px; background:var(--bg-soft); border:1px solid var(--line); border-radius:999px; padding:3px 8px; font-size:12px; color:var(--ink-dim); }
.hl-x { background:none; border:none; color:var(--ink-faint); cursor:pointer; font-size:14px; line-height:1; padding:0; }
.hl-x:hover { color:var(--red); }
/* ── 图层组列表（PS 式面板）────────────────────────────────────────────────
   面板头（新建图层 + 保存状态）/ 列表 / 属性三段；列表与属性各自滚动，整列高度由 JS 限高。 */
.lyr-head { display:flex; align-items:center; gap:6px; flex:none; }
.lyr-title { flex:none; font-size:12px; font-weight:700; color:var(--ink); }
.lyr-sub { font-size:10.5px; color:var(--ink-faint); }
/* 底部工具条（PS 的图层面板底部那一排）：新建图层在左边，右边一句用法提示 */
.lyr-bar { display:flex; align-items:center; gap:6px; flex:none; }
.lyr-bar-hint { margin-left:auto; font-size:10.5px; color:var(--ink-faint); }
.add-wrap { position:relative; }
/* 新建图层的**类型选择器**：可叠加的六种 + 单层的三种（已经有了就置灰）。
   按钮上只有「图标 + 两三个字」，细节全在 title 里（行内文案有长度预算，别写长句）。 */
/* 新建图层的类型选择器：可叠加的六种 + 单层的三种（已经有了就置灰）。
   按钮上只有「图标 + 两三个字」，细节全在 title 里（行内文案有长度预算）。 */
.add-menu { position:absolute; z-index:20; bottom:calc(100% + 6px); left:0; width:230px; padding:8px;
  border:1px solid var(--line); border-radius:10px; background:var(--card); box-shadow:0 10px 28px rgba(0,0,0,.14); }
.add-sec { font-size:10.5px; font-weight:700; color:var(--ink-faint); padding:2px 0 4px; }
.add-grid { display:grid; grid-template-columns:1fr 1fr; gap:4px; margin-bottom:6px; }
.add-item { display:inline-flex; align-items:center; gap:5px; padding:4px 6px; font-size:12px; text-align:left;
  background:var(--bg-soft); border:1px solid var(--line); border-radius:7px; color:var(--ink-dim); cursor:pointer; }
.add-item:hover:not(:disabled) { border-color:var(--main); color:var(--pink-text); }
.add-item:disabled { opacity:.45; cursor:default; }
/* 「已有」小旗：单层三种已经在栈里时的标记（点它 = 选中/打开那一层，不是灰掉不给点） */
.add-flag { margin-left:auto; font-size:9.5px; color:var(--ink-faint); }
.add-gl { flex:none; width:17px; height:17px; border-radius:5px; display:inline-flex; align-items:center; justify-content:center;
  font-size:11px; line-height:1; background:var(--card); border:1px solid var(--line); }
.add-gl.gl-emoji { font-family:'Segoe UI Emoji','Apple Color Emoji','Noto Color Emoji',sans-serif; }
.gl-glow { color:var(--amber-deep); } .gl-blur { color:var(--blue-deep); }
.gl-vignette { color:var(--ink-dim); } .gl-overlay { color:var(--pink-text); }
.gl-scanline { color:var(--ink-faint); } .gl-bg { color:var(--blue-deep); }
.gl-filter { color:var(--green-deep); } .gl-text { color:var(--pink-text); }
.add-foot { font-size:10.5px; color:var(--ink-faint); }
/* 保存状态属于"整份封面配置"（不属于哪一层），所以挂在面板头这一行 */
.save-line { margin-left:auto; flex:none; font-size:11px; color:var(--ink-faint); }
.save-line.err { color:var(--red-deep); }

/* 图层列表：**固定在面板下半部**（用户要求：位置不随属性面板内容跳动）。
   `flex:0 0 50%` 的百分比基准 = 面板的确定高度（JS 给的 `height`，见 `syncControlsBound`）；
   所以属性面板内容多少都只影响它自己（内部滚动），列表的上下位置恒定。
   ⚠️ **`flex-shrink` 必须是 0**：写成 `0 1 50%` 试过 —— 属性面板的 flex-basis 是**内容高度**
   （文字那一屏 ~800px），基准之和早就溢出，于是列表也被按 basis 比例收缩（实测 422px 的面板里
   列表只剩 120px），而且**换选中层时列表位置跟着跳**（334 → 243）—— 正是这条规则要防的那件事。 */
.lyr-list { flex:0 0 50%; min-height:72px; overflow-y:auto;
  border:1px solid var(--line); border-radius:10px; background:var(--card); scrollbar-width:thin; }
.lyr-list::-webkit-scrollbar { width:6px; }
.lyr-list::-webkit-scrollbar-thumb { background:var(--line); border-radius:3px; }
.lyr-grp { position:sticky; top:0; z-index:1; display:flex; align-items:center; gap:6px; padding:4px 8px 2px;
  font-size:10.5px; font-weight:700; color:var(--ink-faint); background:var(--card); }
.lyr-grp-h { font-weight:400; }
.lyr { position:relative; display:flex; align-items:center; gap:6px; padding:3px 6px; cursor:pointer;
  border-left:3px solid transparent; }
.lyr:hover { background:var(--bg-soft); }
.lyr.sel { background:var(--main-soft); border-left-color:var(--main); }
/* 隐藏的层：整行变淡（但点得到、拖得动）—— PS 里眼睛一关也是这样 */
.lyr.hidden .lyr-nm, .lyr.hidden .lyr-th { opacity:.45; }
.lyr.dragging { opacity:.5; }
.lyr.drop-before::before, .lyr.drop-after::after { content:''; position:absolute; left:0; right:0; height:2px;
  background:var(--main); }
.lyr.drop-before::before { top:-1px; }
.lyr.drop-after::after { bottom:-1px; }
/* 拖柄：鼠标移到行上才显形（与删除按钮同一档），锁定的层置灰不给拖 */
.lyr-drag { flex:none; width:12px; font-size:11px; line-height:1; color:var(--ink-faint); cursor:grab; opacity:0; }
.lyr:hover .lyr-drag, .lyr.sel .lyr-drag { opacity:1; }
.lyr-drag.off { cursor:default; opacity:.25; }
.lyr-eye { flex:none; width:18px; height:18px; display:inline-flex; align-items:center; justify-content:center; }
.lyr-eye { padding:0; background:none; border:none; cursor:pointer; color:var(--ink-dim); }
.lyr-eye:hover { color:var(--main); }
.lyr-eye.off { opacity:.55; color:var(--ink-faint); }
.lyr-eye svg { width:15px; height:15px; fill:none; stroke:currentColor; stroke-width:1.7; stroke-linecap:round; stroke-linejoin:round; }
.lyr-th { flex:none; width:20px; height:20px; border-radius:5px; display:inline-flex; align-items:center; justify-content:center;
  font-size:11px; line-height:1; background:var(--bg-soft); border:1px solid var(--line); overflow:hidden; }
.th-video { color:var(--blue-deep); } .th-bg { color:var(--ink-dim); } .th-filter { color:var(--green-deep); }
.th-text { color:var(--pink-text); }
/* 特效/表情的缩略：emoji 那一支直接显示用户贴的字符，所以字体栈要带 emoji */
.th-fx { color:var(--amber-deep); font-family:'Segoe UI Emoji','Apple Color Emoji','Noto Color Emoji',sans-serif; }
.th-guide { border-style:dashed; background:transparent; }
.lyr-nm { flex:1 1 auto; min-width:0; font-size:12px; color:var(--ink); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.lyr-lk { flex:none; font-size:10px; color:var(--ink-faint); border:1px solid var(--line); border-radius:999px; padding:0 5px; }
.lyr-tools { flex:none; display:inline-flex; align-items:center; gap:2px; opacity:0; }
.lyr:hover .lyr-tools, .lyr.sel .lyr-tools { opacity:1; }
.ico-btn { display:inline-flex; align-items:center; justify-content:center; width:20px; height:20px; padding:0;
  background:none; border:1px solid transparent; border-radius:5px; color:var(--ink-faint); cursor:pointer; }
.ico-btn:hover:not(:disabled) { border-color:var(--line); color:var(--ink); background:var(--card); }
.ico-btn:disabled { opacity:.3; cursor:default; }
.ico-btn.del:hover { color:var(--red-deep); border-color:var(--red); background:var(--red-soft); }

/* 属性面板（选谁改谁）：在上、自己内部滚。
   ⚠️ 类名不能叫 `.pane`：`styles/app.css` 有全局 `.pane { display:none }`（设置页 tab 用），
   叫那个名字整块面板会隐身 —— harness 里就是这条抓出来的（与 `.stage-video` 黑边同源）。
   ⚠️ `min-height` 也不能省：`.controls` 的高度被 JS 限到与预览等高，文字层那一屏内容比空间高得多，
   flex 收缩按 flex-basis 比例来 —— 没有下限时面板会被压成 0 高。下限给到 120px 后，
   先被压缩的是下面那个图层列表，属性面板始终留得住一屏。 */
.lyr-props { flex:1 1 auto; min-height:120px; overflow-y:auto; padding:8px 10px;
  border:1px solid var(--line); border-radius:10px; background:var(--card); scrollbar-width:thin; }
.lyr-props::-webkit-scrollbar { width:6px; }
.lyr-props::-webkit-scrollbar-thumb { background:var(--line); border-radius:3px; }
.lyr-props-h { font-size:12px; font-weight:700; color:var(--ink); }
.lyr-props-note { font-size:10.5px; line-height:1.5; color:var(--ink-faint); }
@media (max-width: 900px) {
  .designer-grid { grid-template-columns:1fr; }
  /* 单列时不再定高/定比例：让内容自己撑（列表给个上限就够） */
  .controls { height:auto !important; overflow:visible; }
  .lyr-list { flex:0 0 auto; max-height:260px; }
  /* 窄屏：文字面板的"文本框 + 右侧控件列"改上下排 */
  .txt-top { grid-template-columns:1fr; }
}
</style>
