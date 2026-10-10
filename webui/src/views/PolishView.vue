<template>
  <div>
    <!-- 紧凑页头一行：chip + 片段标题（竖高省给预览；哪个片段由面包屑/片段卡说明） -->
    <div class="page-head compact-head">
      <span class="chip p">精修</span>
      <h1 class="m-0">{{ dir ? title : '选择要精修的片段' }}</h1>
      <span v-if="!dir" class="sub m-0">从片段库选一个片段开始精修</span>
      <div class="actions"></div>
    </div>

    <!-- 切片列表读不到：**必须与"真的没有切片"分开**（以前失败被 catch 成空数组，
         页面显示"还没有可精修的切片"，看着像切片被删了） -->
    <div v-if="clipsError" class="error-banner">
      <Icon name="alert" :size="15" />
      <span>切片列表读取失败：{{ clipsError }}</span>
      <button class="mini-btn ml-auto" @click="load({ silent: true })">重试</button>
    </div>

    <!-- 无切片：空态引导（不报错不白屏）。用 v-else-if 接在错误条后面：读取失败时**只**出错
         —— 否则它会被下面的 v-else 当成"有内容"而把整个编辑器壳渲染出来 -->
    <div v-else-if="!clips.length && !loading" class="card card-blk" style="max-width:520px;margin:48px auto;text-align:center;padding:36px">
      <b style="font-size:15px">还没有可精修的切片</b>
      <p class="small muted" style="margin:8px 0 16px">先到「切片」页生成片段，再回来精修</p>
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
          <button class="mini-btn ml-auto" @click="load()">重试</button>
        </div>
        <p class="muted small m-0">工程文件可能还在，先重试，别点初始化。</p>
      </div>

      <!-- 列表模式：没有上次操作记录 → 只显示片段列表 + 空态，视频/编辑器不加载 -->
      <div v-else-if="!clipName" class="card card-blk">
        <div class="row spread mb-8">
          <h2 class="m-0 card-h2"><Icon name="list" :size="15" /> 片段（{{ clips.length }}）</h2>
          <span class="muted small">点击一个片段进入精修编辑器</span>
        </div>
        <FragmentList :rows="fragRows" :current="clipName" @goto="gotoClip" @init="initClipByName" />
        <div class="empty small mt-12">未选择片段 · 选中后才加载编辑器</div>
      </div>

      <!-- 未初始化：初始化卡 + 片段列表（否则无法切换/初始化其他片段） -->
      <div v-else-if="!ready" class="grid cols-main">
        <div class="card">
          <h2>未初始化精修项目</h2>
          <p class="muted small">解析字幕与弹幕，创建工程文件</p>
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
          <button class="mini-btn ml-auto" @click="load({ silent: true })">重试</button>
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
              <span v-else class="muted small">字幕 · 贴纸 · 运镜 · 弹幕 · 音效</span>
            </div>
            <!-- 左：属性面板列（**一个**面板位置，两种内容：轨道设置 / 时间轴块的属性）
                 右：预览舞台
                 ⚠️ `#tl-props-host` 必须是**空容器**：TimelineEditor 用 Teleport 把块的属性面板
                 投进它（Teleport 直接往目标里插节点，跟页面自己的子节点混在一起会被 patch 顶掉）。
                 轨道设置那块由本页面渲染，两者用 `openLane` 互斥 —— 同一时刻只有一份面板。 -->
            <div class="preview-row">
              <!-- ⚠️ 轨道设置面板**已整块搬到右栏常驻**（用户 2026-10-06：「还是把轨道设置放在右侧常驻」）。
                   这里只剩时间轴块的属性面板挂载点 —— `#tl-props-host` 必须是**空容器**：
                   TimelineEditor 用 Teleport 直接往目标里插节点，与本页面子节点混在一起会被 patch 顶掉。 -->
              <div class="props-host">
                <div id="tl-props-host"></div>
              </div>

              <div class="stage-col">
                <PolishPreview ref="previewRef" :key="previewKey" :pid="pid" :dir="dir" :config="cfg" :persisted-config="persistedCfg" :subtitles="subs" :persisted-subtitles="persistedSubs" :danmaku="dm" :meta-duration="cfg?.duration || 0" :font-fallback="fontFallback" :emoji-family="fontEmoji" :select-mode="!!selectRegion" :select-rect="selRect" :select-label="selLabel" :point-mode="pointMode" :point="pointEdit ? draftPos : stickerPos" :point-label="'贴纸落点 · ' + (pointEdit ? draftPosPct : stickerPosPct)" @time="onPreviewTime" @playing="onPreviewPlaying" @update:select-rect="onSelectRect" @update:point="onPointPick" />
                <div v-if="selectRegion" class="hint mt-8">拖框体平移 / 拖角缩放 / 空白重画</div>
                <div v-else-if="pointEdit" class="hint mt-8">点预览选新落点，右侧「保存落点」生效</div>
              </div>
            </div>
          </div>

          <!-- 右栏 = **一块卡片、两个页签**（用户 2026-10-06 第五轮口径，**这一版才对**）：
               「项目设置」= **轨道设置的全部内容 + LLM 剪辑 / 渲染导出 / 任务栏**（用户原话：
               「轨道设置内的东西和项目设置合并」—— 它们**是同一页**，不是一个叠一个）；
               「画面尺寸」= 横屏·竖屏 / 缩放比例 / 主画面位置 / 背景。
               ⚠️ **只有一块卡片**：之前拆成两三块独立卡片，右栏堆到 237 行，**把时间轴挤出首屏**
               （用户：「你分这么多区域，都把时间轴都挤走了」）。加内容前先看总高度。
               ⚠️ 页签**没有**「项目设置页 →」那种跳转链接（用户 2026-10-06：「删除，不需要这个」）。
               ⚠️ 必须全在**同一个 grid 子节点** `.right-col` 里：`.edit-grid` 是两列
               （`cols-main` = `1fr 300px`），摊成多个子节点时后面的会充当"第三列"
               **掉到下一行占满整宽**。判据 ⑰ 钉着。 -->
          <div class="right-col">
            <div class="card">
              <!-- **三个页签独占最上面一行**（用户口径：「三个标签页放在最上面，删除最上面
                   项目设置这个大文字」）。原先标题与页签挤在同一行、还占掉一半宽度。
                   ⚠️ 那个 `<h2>项目设置</h2>` 已删 —— 页签名本身就说清了这是什么，
                      再挂一个大标题属于同一件事说两遍（本卡既有纪律）。
                   ⚠️ 用 `flex:1` 让三颗**等宽铺满**：右栏 300px 内容宽 266px，三颗各自
                      约 84px，中文四个字（12px×4=48px）+ padding 22px + 边框 2px = 72px
                      ⇒ 放得下，不会溢出。 -->
              <div class="side-tabs">
                <button class="mini-btn" :class="{ selected: sideTab === 'project' }" @click="sideTab = 'project'">项目设置</button>
                <button class="mini-btn" :class="{ selected: sideTab === 'canvas' }" @click="sideTab = 'canvas'">画面尺寸</button>
                <button class="mini-btn" :class="{ selected: sideTab === 'intro' }" @click="sideTab = 'intro'">片头片尾</button>
              </div>

              <template v-if="sideTab === 'project'">
  <!-- **所有轨道的设置都在这一个面板里**（用户 2026-10-06：点哪个轨道都是同一个面板）。
       点轨道名 = 打开面板 + 滚到那一段 + 标题高亮。
       ⚠️ **只留控件**（用户 2026-10-06 第二轮：解释文字全删、能删的都删了）：
       不要 `hint` 行、不要「· 说明」后缀、不要计数行 —— 需要解释就写进 `title`（悬停可见）。
       ⚠️ 也不许写成按 `openLane` 条件渲染的链（判据 ⑰ 钉着这一条，带变异自证）。 -->

  <!-- ⚠️ 「分段 / 屏蔽」那条**纯信息**的分组已删（用户 2026-10-06：仅信息展示的内容删除）。
       它只显示两个计数（分段 N / 屏蔽 N），没有任何控件 —— 段数在时间轴泳道上直接数得出来，
       两个数字放在设置面板里既不能改也不指向任何操作。**别再捡回来。** -->

<!-- ── 三组**并排成三列**（用户 2026-10-06 第六轮：「三个区域放在同一行」）──
     顺序 = 字幕 / 弹幕 · 贴纸 · 区域（贴纸排在「区域」上面）。
     ⚠️ 右栏只有 300px，三列各约 92px：**列内控件都必须能窄到不溢出**（`min-width:0` +
     `flex-wrap`）。改动后要跑 CDP 探针量横向溢出（水平 scrollWidth > clientWidth 即算坏）。
     ⚠️ 组的 `:ref` 仍按**轨道名**挂（`camera` 与 `danmaku_area` 共用「区域」那组）。 -->
  <div class="lane-group" :class="{ on: openLane === 'subtitles' }"
       :ref="(el) => (laneGroupEls.subtitles = el)">
    <div class="lane-group-h">字幕 / 弹幕</div>
    <!-- **「字幕字体」标签 + 两颗「→默认」同一行**（用户口径：「位置→默认 / 大小→默认
         这俩个放在字幕字体旁边」= 就贴在"字幕字体"这四个字旁边）。
         ⚠️ 这两颗按钮**与字体无关** —— `位置→默认` 管的是下面「字幕位置」那条滑杆、
            `大小→默认` 管的是「字幕大小」那个数字框，只是位置挪到了这里（名字保持原样，
            各自的 `title` 写清了管什么：本卡纪律「行内文案 ≤20 字，解释进 title」）。
         ⚠️ 与下面那个字幕样式块共用 `styleWriteBlocked`：读配置失败时整块只读，
            这两颗也必须一起禁用，否则会"拿默认值把磁盘上的样式清成默认"。
         ⚠️ 宽度实测：标签 50px + 两颗各 84px + 间距 12px = 230px ≤ 行宽 266px。 -->
    <div class="row ss-font-row">
      <span class="small muted flex-none">字幕字体</span>
      <button class="mini-btn flex-none" :disabled="styleWriteBlocked || !ready || !subtitlePosSet"
              :title="subtitlePosSet ? '清除位置设置，回到历史落点公式' : '当前就是未设置'"
              @click="restoreSubtitlePos">位置→默认</button>
      <button class="mini-btn flex-none" :disabled="styleWriteBlocked || !ready || !subtitleSizeSet"
              :title="subtitleSizeSet ? '清除字号设置，回到默认 72' : '当前就是未设置'"
              @click="restoreSubtitleSize">大小→默认</button>
    </div>
    <select class="fld-input" :value="subtitleFont" :disabled="fontSaving || !ready" @change="onSubtitleFontChange">
      <option v-for="o in fontOptions" :key="o.value" :value="o.value">{{ o.label }}</option>
    </select>
    <div v-if="fontsError" class="row small mt-4 gap-6">
      <span class="muted">字体库读取失败：{{ fontsError }}</span>
      <button class="mini-btn" @click="loadFonts">重试</button>
    </div>

    <!-- ── 字幕样式（用户 2026-10 点名四项：颜色 / 描边 / 大小 / 位置）。
         ⚠️ 全在**本组内**（`docs/plan/next-version-subtitle-style.md` §3.2 方案 A）。
         ⚠️ **读配置失败 ⇒ 整块变成只读 + 错误条**（`styleWriteBlocked`）：
            这些控件的初值就是从 `cfg` 来的，而 `cfg` 读失败时为 null ——
            不断写盘等于"拿默认值把磁盘上的样式清成默认"（本仓"静默丢改动"的老根因）。
         ⚠️ 行内文案 ≤20 字，解释一律进 `title`（右栏卡片行内容宽只有 266px）。 -->
    <div v-if="styleReadError" class="row small mt-6 gap-6">
      <span class="muted">样式读取失败：{{ styleReadError }}</span>
      <button class="mini-btn" @click="load({ silent: true })">重试</button>
    </div>
    <div class="ss-grid" :class="{ off: styleWriteBlocked || !ready }">
      <!-- **两列两行**（用户口径：「改为两行的排版」）：
           第 1 行 = 两个**颜色**（字幕 / 描边），第 2 行 = 两个**数值**（描边宽度 / 大小）。
           按"控件同类"分组，而不是按语义配对 —— 颜色框与数字框宽度差很远，混排会参差不齐。
           ⚠️ 每行两列、每列 ~130px（行内容宽 266px）：标签 62px + 控件；
              颜色框 34px 够，数字框给 `flex:1` 吃掉剩余宽度。 -->
      <div class="ss-row-2">
        <div class="ss-row">
          <span class="ss-lbl">字幕颜色</span>
          <input type="color" class="ss-color" :value="subtitleColor"
                 :disabled="styleWriteBlocked || !ready" title="字幕正文颜色"
                 @change="onSubtitleColorChange">
        </div>
        <div class="ss-row" :title="strokeColorHint">
          <span class="ss-lbl">描边颜色</span>
          <input type="color" class="ss-color" :value="subtitleStrokeColor"
                 :disabled="!strokeEditable || !ready" title="字幕描边颜色"
                 @change="onSubtitleStrokeColorChange">
        </div>
      </div>
      <div class="ss-row-2">
        <div class="ss-row" :title="strokeWidthHint">
          <!-- ⚠️ 原先这里挂着一个「近似」角标，**两列排版下放不下**（实测：文字 44px +
               角标 24px + 间距 3px = 71px，而列里只有 44px ⇒ 角标被裁掉一半）。
               而且 `strokeWidthHint` 的悬停提示里**本来就写着**「近似值：底座只有描边开/关…」
               —— 角标属于同一件事说两遍。删掉角标，保悬停提示。
               ⚠️ `.ss-tag` 的样式**留着**（别顺手删）：它是通用角标类，别处可能还在用。 -->
          <span class="ss-lbl">描边宽度</span>
          <input type="number" class="rv-input ss-num" inputmode="numeric" min="0" max="24" step="1"
                 :value="subtitleStrokeWidth" :disabled="!strokeEditable || !ready"
                 aria-label="字幕描边宽度（像素，近似）" @change="onSubtitleStrokeWidthChange">
          <span class="mono small flex-none">px</span>
        </div>
        <div class="ss-row" :title="'字幕字号（px）。默认 72'">
          <span class="ss-lbl">字幕大小</span>
          <input type="number" class="rv-input ss-num" inputmode="numeric" min="8" max="400" step="1"
                 :value="subtitleSize" :disabled="styleWriteBlocked || !ready"
                 aria-label="字幕大小（像素）" @change="onSubtitleSizeChange">
          <span class="mono small flex-none">px</span>
        </div>
      </div>
      <div class="ss-row full" :title="positionHint">
        <span class="ss-lbl">字幕位置</span>
        <input type="range" class="flex-1 ss-range" min="0" max="50" step="0.5"
               :value="subtitlePosPct" :disabled="styleWriteBlocked || !ready"
               :title="positionHint" @input="onSubtitlePosInput" @change="onSubtitlePosChange">
        <input type="number" class="rv-input" inputmode="decimal" min="0" max="50" step="0.5"
               :value="subtitlePosPct" :disabled="styleWriteBlocked || !ready"
               aria-label="字幕距底百分比" title="可直接填写（0 ~ 50）" @change="onSubtitlePosType">
        <span class="mono small flex-none">%</span>
      </div>
    </div>
    <!-- ⚠️ 「恢复默认」那两颗按钮**已移到「字幕字体」那一行**（用户口径：「位置→默认 /
         大小→默认这俩个放在字幕字体旁边」）。别在这里加回来 —— 判据 ⑰ 钉着"它们必须在
         `.ss-font-row` 里、且全页各只出现一次"。
         口径不变：取色器/数字框天生只能产出具体值，没有显式恢复动作用户就永远回不到"没设过"
         （而 `position_pct` 是 `Option`、"不写"与"写 12"在成片侧**语义不同**）。
         ⚠️ 对 size/color/stroke 这三项，"默认" = **不发键**（保持磁盘原样）——
            serde 默认值与"用户显式写了 72"在前端**区分不了**，发回 72 就是撒谎。 -->

    <div class="small muted mt-6">弹幕字体</div>
    <select class="fld-input" :value="danmakuFont" :disabled="fontSaving || !ready" @change="onDanmakuFontChange">
      <option v-for="o in danmakuFontOptions" :key="o.value" :value="o.value">{{ o.label }}</option>
    </select>
  
    <div class="row mt-6" style="gap:6px;align-items:center">
      <span class="small muted flex-none">偏移</span>
      <input type="range" class="flex-1" min="-10" max="10" step="0.5" :value="dmOffset"
             :disabled="dmOffsetSaving || !ready" :title="'弹幕时间偏移：' + dmOffset + 's，负 = 提前出现，正 = 推后（默认 -3）'"
             @input="onDanmakuOffsetInput" @change="onDanmakuOffsetChange">
      <input class="rv-input" type="number" inputmode="decimal" min="-10" max="10" step="0.5"
             :value="dmOffset" :disabled="dmOffsetSaving || !ready" aria-label="弹幕时间偏移（秒）"
             title="可直接填写（-10 ~ 10），手填的值不吸附滑块步进" @change="onDanmakuOffsetType($event)">
      <span class="mono small flex-none">s</span>
    </div>
  
    <label class="ck-row mt-6" :title="lanesOn.danmaku ? '当前显示；点一下整条不渲染（素材一个字都不动）' : '当前不显示；点一下恢复'">
      <input type="checkbox" :checked="lanesOn.danmaku === true" :disabled="stageBusy || !ready"
             @change="toggleLane('danmaku')">
      显示弹幕
    </label>
  </div>
  <!-- ── **区域设置** = 贴纸来源 + 区域框选 + 落点模式（用户 2026-10-06 第八轮：
       「贴纸和区域合并，显示为区域设置」）。原来「贴纸」与「区域」是两组，改的是同一批东西
       （预览上的框与落点、以及贴纸落在哪儿），合并成一组。
       行结构（用户给的样子）：
         区域设置
         [emoji][ysyk][…]                      ← 贴纸来源（多选开关）
         [弹幕位置][人物位置][自动]              ← 三颗同一行；「自动」点一下切 自动/固定
       ⚠️ ref 同时挂给 `sticker` / `camera` / `danmaku_area` 三个轨道名：点时间轴上这几条轨道
       的名字都该滚到**这一组**（它们现在共用同一组控件）。 -->
  <div class="lane-group" :class="{ on: openLane === 'sticker' || openLane === 'camera' || openLane === 'danmaku' }"
       :ref="(el) => { laneGroupEls.sticker = el; laneGroupEls.camera = el; laneGroupEls.danmaku_area = el }">
    <div class="lane-group-h">区域设置</div>
    <div class="ss-options">
      <button v-for="src in allStickerOptions" :key="src.name" class="ss-opt"
              :class="{ on: stickerSource.includes(src.name) }"
              :title="'贴纸来源：' + src.name + (src.tags?.length ? '（tags: ' + src.tags.join(' / ') + '）' : (src.description ? '（' + src.description + '）' : '')) + ' —— 点一下开/关'"
              @click="toggleStickerSource(src.name)">
        <b>{{ src.name }}</b>
      </button>
      <div v-if="stickerSourcesError" class="row small gap-6">
        <span class="muted">读取失败：{{ stickerSourcesError }}</span>
        <button class="mini-btn" @click="loadStickerSources">重试</button>
      </div>
      <div v-else-if="!allStickerOptions.length" class="muted small">贴纸库为空：往 stickers/ 放贴纸包</div>
    </div>

    <!-- ⚠️ **三颗同一行**（用户 2026-10-06 第八轮）：弹幕位置 / 人物位置 / 自动。
         前两颗是预览上的自由选框（互斥，同一时刻只有一份），第三颗是贴纸落点模式开关。 -->
    <div class="row mt-6">
      <button class="mini-btn flex-1 jc-center" :class="{ selected: selectRegion === 'danmaku_area' }" :disabled="!ready"
              title="在预览上框选弹幕显示区域（供运镜参考，也是贴纸的避让障碍）" @click="toggleSelect('danmaku_area')">
        <Icon name="crop" :size="11" /> 弹幕位置
      </button>
      <button class="mini-btn flex-1 jc-center" :class="{ selected: selectRegion === 'liver' }" :disabled="!ready"
              title="在预览上框选主播人物：运镜聚焦到框中心，也是贴纸的避让障碍（改完要重跑一次「重新后处理」才落到事件上）" @click="toggleSelect('liver')">
        <Icon name="crop" :size="11" /> 人物位置
      </button>
      <button class="mini-btn flex-1 jc-center"
              :class="{ selected: stickerCfg.position_mode === 'manual' }" :disabled="stickerBusy || !ready"
              :title="stickerCfg.position_mode === 'manual' ? '固定落点：全部贴纸都落在同一点，切到固定后在预览上点选' : '自动避让：按安全区算法逐条找位置'"
              @click="toggleStickerMode">
        <Icon name="pin" :size="11" /> {{ stickerCfg.position_mode === 'manual' ? '固定' : '自动' }}
      </button>
    </div>

    <!-- 运镜强度（总方案 §18-C）：**只暴露配置、不改默认**。
         三档的语义在 `[style.camera]` 里（中/近/画中画），这张表被四条渲染路径共用
         （render-core / 模板 canvas / engine.js / 转译器）⇒ 改了这里预览与成片**同时**变。
         ⚠️ 硬切（`cut`）与远景不受这个值影响 —— 它们是"不去"而不是"缩放"。 -->
    <template>
      <div class="mono small mt-6">运镜强度</div>
      <label v-for="z in zoomFields" :key="z.key" class="row gap-6 mt-4" style="align-items:center">
        <span class="small" style="width:52px">{{ z.label }}</span>
        <input type="range" class="flex-1" :min="z.min" :max="z.max" step="0.05"
               :value="zoomValue(z.key)" :disabled="styleSaving || !ready"
               :title="z.title"
               @input="onZoomInput(z.key, $event)"
               @change="onZoomCommit(z.key, $event)" />
        <span class="mono small" style="width:34px;text-align:right">{{ zoomValue(z.key).toFixed(2) }}</span>
      </label>
      <div class="muted small mt-4">越大越近；预览与成片同时生效</div>
    </template>

    <template v-if="selectRegion">
      <div class="mono small mt-4">{{ rectText }}</div>
      <div class="row mt-4 gap-6">
        <button class="mini-btn primary" @click="applySelect"><Icon name="check" :size="11" /> 保存选框</button>
        <button class="mini-btn" :disabled="!hasFreeRect" @click="clearSelect">清除</button>
      </div>
    </template>
    <template v-if="stickerCfg.position_mode === 'manual' && !selectRegion">
      <div class="row mt-4 gap-6">
        <button v-if="!pointEdit" class="mini-btn flex-1 jc-center"
                :disabled="stickerBusy || !ready" title="在预览上点选贴纸的新落点（草稿态，保存后才写 polish.toml）"
                @click="startPointEdit">
          <Icon name="pin" :size="11" /> 改落点
        </button>
        <div class="mono small">{{ pointEdit ? draftPosPct : stickerPosPct }}</div>
      </div>
      <div v-if="pointEdit" class="row mt-4 gap-6">
        <button class="mini-btn primary" :disabled="stickerBusy" @click="savePointEdit"><Icon name="check" :size="11" /> 保存落点</button>
        <button class="mini-btn" :disabled="stickerBusy" @click="cancelPointEdit">取消</button>
      </div>
    </template>
  </div>

  <!-- ⚠️ 「音效」分组已整块删除（用户 2026-10-06：删除音效的音量设置）。
       它里面只有一条全局默认音量滑杆；事件自己的 `volume` / `sfx_volume` 优先，
       所以这条滑杆改的是"剩下那些没单独设过的事件"，用户看不出效果。
       ⚠️ **音效轨本身（眼睛 / `[stage.lanes].sfx`）没删** —— 删的只是这个面板分组。
       要改全局默认音量走 `polish.toml` 的 `[sfx] volume`。**别再把这组捡回来。** -->

                <div class="lane-sep"></div>
            <!-- LLM 剪辑：重新分段 + 重新生成全部事件。
                 失败的分段会在时间轴上标色、可单独重做那一段。 -->
            <div class="row" style="gap:6px">
              <button class="btn flex-1 jc-center" :disabled="scopeBusy"
                      :title="scopeBusy ? '该片段正在' + scopeBusyText : '全部重做：重新分段 + 重新生成全部事件（失败的分段会在时间轴上标色，可单独重做那一段）'"
                      @click="runLlmJob"><Icon name="sparkles" :size="12" /> LLM 剪辑</button>
            </div>

            <!-- 出片 -->
            <button class="btn btn-primary mt-6 w-full jc-center" :disabled="scopeBusy" :title="scopeBusy ? '该片段正在' + scopeBusyText : ''" @click="openRenderModal"><Icon name="film" :size="13" /> 渲染导出</button>

            <!-- 别人（别的视图/标签页）占用这个片段时也提示一句，别让按钮变灰得没头没脑 -->
            <div v-if="!jobRunning && scopeBusyJob" class="hint mt-8">
              该片段正在「{{ kindText(scopeBusyJob.kind) }}」，请稍候
            </div>

            <!-- 任务状态栏：**常驻**（不再用 `v-if` 按任务有无出现/消失），只报「谁在跑 + 状态 + 进度」。
                 同一件事不说三遍（jobLabel + chip + 「任务完成」），也不回显服务端日志（用户要求）。 -->
            <!-- 任务栏 = **可折叠**（用户 2026-10-06：「任务改成折叠的模式，默认折叠，只显示当前是什么
                 任务，展开显示详情，展开时不要挤占下方控件，是浮在上层的」）。
                 ① 默认折叠：**一行** = 任务名 + 状态 chip（就是"当前是什么任务"）；
                 ② 展开：`position:absolute` 浮在上层，**不参与布局** ⇒ 不挤占下方控件
                    （所以下面那块卡片要 `position:relative` 当定位锚）；
                 ③ 任务在跑时，折叠态也带进度条 —— 不然折叠了就看不到进展。 -->
            <div class="job-wrap">
              <button class="job-head" :class="{ open: jobOpen }" @click="jobOpen = !jobOpen">
                <b class="flex-1 ellipsis">{{ job.kind ? jobLabel(job.kind) : '任务' }}</b>
                <span class="chip" :class="jobChipClass">{{ jobStatusText }}</span>
                <Icon name="chevron-down" :size="11" class="job-caret" />
              </button>
              <div v-if="jobOpen" class="row spread small mt-6">
                <span v-if="jobRunning" class="muted small mono">{{ jobTimeText }}</span>
                <button v-if="jobRunning" class="mini-btn danger" :disabled="canceling" @click="cancelJob"><Icon name="x" :size="10" /> 中断</button>
                <span v-if="!jobRunning" class="muted small">空闲</span>
              </div>
              <div v-if="jobOpen && (job.error || (jobRunning && job.log))" class="muted small mt-6 mono job-log">{{ job.error || job.log }}</div>
            </div>

              </template>

              <!-- ⚠️ **片头片尾是独立页签**（用户口径：「片头片尾放在一个单独的页面」）——
                   它原先挤在「项目设置」那一段的末尾，跟字幕样式/贴纸等混在一起。
                   现在与「项目设置 / 画面尺寸」并列成第三个页签，同一位置切换。
                   标记值用 `'intro'`（不是 `'introOutro'`）：与 `INTRO_OUTRO_SIDES` 的
                   `key` 无关，只是页签名。 -->
              <template v-else-if="sideTab === 'intro'">
                  <!-- ── 片头 / 片尾（`[intro_outro]`）────────────────────────────────────
       用户口径：「片头片尾在项目设置内可以选择是否开启，然后可以选择素材库内的视频内容」。
       每一侧 = 一个开关 + 一个素材下拉 + 入点/出点（可裁剪，缺省播完整段）。
       ⚠️ **"开"的判据就是"选没选素材"**（Rust `IntroOutroClip::is_enabled`）——
          不另设一个能和 `asset_id` 打架的 `enabled` 布尔：两个真源必然分叉。
       ⚠️ 素材库是**全局**的（`<程序目录>/stickers/videos/`），下拉开一次就复用（`videoLibrary`）。 -->
  <div class="lane-group" :class="{ on: openLane === 'intro' || openLane === 'outro' }">
    <div class="lane-group-h">片头 / 片尾</div>
    <template v-for="s in INTRO_OUTRO_SIDES" :key="s.key">
      <div class="io-row">
        <label class="io-head">
          <input type="checkbox" :checked="!!introOutro[s.key].asset_id"
                 :disabled="ioBusy || !ready"
                 @change="onIntroOutroToggle(s.key, $event.target.checked)" />
          <span>{{ s.label }}</span>
        </label>
        <template v-if="introOutro[s.key].asset_id">
          <select class="fld-input" :value="introOutro[s.key].asset_id"
                  :disabled="ioBusy || !ready"
                  @change="onIntroOutroAsset(s.key, $event.target.value)">
            <!-- ⚠️ 探不出时长的**照列但禁用**（别隐藏）：用户放进去了却在界面里找不到，
                 比"能用但报错"更难查 -->
            <option v-for="v in videoLibrary" :key="v.name" :value="v.name"
                    :disabled="v.duration == null">
              {{ v.name }}{{ v.duration == null ? '（读不出时长）' : '' }}
            </option>
          </select>
          <div class="io-trim">
            <span class="muted small">入</span>
            <input class="fld-input io-num" type="number" min="0" step="0.1"
                   :value="introOutro[s.key].in_sec" :disabled="ioBusy || !ready"
                   title="从素材的第几秒开始（0 = 从头）"
                   @change="onIntroOutroTrim(s.key, 'in_sec', $event.target.value)" />
            <span class="muted small">出</span>
            <input class="fld-input io-num" type="number" min="0" step="0.1"
                   :value="introOutro[s.key].out_sec" :disabled="ioBusy || !ready"
                   title="到素材的第几秒结束；0 = 播到素材结尾"
                   @change="onIntroOutroTrim(s.key, 'out_sec', $event.target.value)" />
            <span class="muted small io-dur">{{ introOutroSpan(s.key) }}</span>
          </div>
        </template>
        <p v-else class="muted small m-0">未启用</p>
      </div>
    </template>
    <p v-if="ioError" class="small m-0 mt-4 io-err">{{ ioError }}</p>
  </div>
              </template>

              <template v-else>
            <!-- 缩放比例 / 主画面位置（五档吸附）/ 背景（`[stage]`）。
                 值存 polish.toml 的 [canvas] / [stage]；出片与预览共用同一份推导
                 （Rust `resolve_stage` → `_canvas`），这里只做提交 + 重取，不在前端重算。 -->
            <div class="row mb-6 gap-6">
              <button v-for="o in orientationOptions" :key="o.value" class="mini-btn flex-1 jc-center"
                      :class="{ selected: (canvasCfg.orientation || 'landscape') === o.value }" :disabled="stageBusy || !ready"
                      @click="patchCanvas({ orientation: o.value })">{{ o.label }}</button>
            </div>

            <div class="row mb-6">
              <span class="small muted flex-none">缩放比例</span>
              <input type="range" class="flex-1" min="0.1" max="4" step="0.01" :value="stageScaleNum"
                     :disabled="stageBusy || !ready"
                     @change="patchStage({ scale: Number($event.target.value) })">
              <input type="number" class="mono small" style="width:4.4em" min="0.1" max="4" step="0.01"
                     :value="stageScaleNum" :disabled="stageBusy || !ready"
                     @change="patchStage({ scale: clampScale($event.target.value) })">
            </div>

            <!-- 主画面位置 = **两轴各一条五档**（0/25/50/75/100）。贴满的那条轴没有余量
                 （`free_*` = 0⁻），那条滑杆**置灰并写明原因** —— 否则用户拖了没反应只能猜。
                 同比铺满且没放大时整组隐藏（那时两条都无效），放大后自动出现。 -->
            <template v-if="showStageGeom">
              <div class="small muted mb-4"
                   title="两轴各自在可平移余量里五档吸附；放大（scale&gt;1）后同一条滑杆变成裁切取景">主画面位置</div>
              <template v-for="ax in positionAxes" :key="ax.key">
                <div class="row mb-4">
                  <span class="small muted flex-none" style="width:2.6em">{{ ax.label }}</span>
                  <input type="range" class="flex-1" min="0" max="100" step="25" :value="ax.pct"
                         :disabled="stageBusy || !ready || ax.inert" :title="ax.inert ? ax.hint : ''"
                         @change="patchStage({ [ax.key]: Number($event.target.value) })">
                  <span class="mono small" style="flex:none;width:3.2em">{{ ax.inert ? '—' : ax.text }}</span>
                </div>
                <div v-if="ax.inert" class="small muted mb-4">{{ ax.hint }}</div>
                </template>
              </template>
              </template>
            </div>
          </div>
        </div>
        <!-- 时间轴编辑器独占一整行（在预览/生成两栏之下）：横向空间全给刻度与泳道。
             标题/提示由 slot 交给组件，省掉独立卡片头；去静音只在这块，右侧不再重复。
             改动走「本地即时镜像 → 预览实时刷新 → 防抖落盘 → 防抖后处理」这条链。 -->
        <div class="card tl-card mt-16">
          <TimelineEditor ref="timelineRef" :key="'tl' + segKey" :dir="dir" :pid="pid" :current-time="currentTime" :playing="previewPlaying" :reprocess-busy="scopeBusy" :lane-open="openLane" :lanes-on="lanesOn" :src-fps="srcFps" @seek="seekTo" @toggle="togglePreviewPlay" @preview="onTimelinePreview" @saved="onEditSaved" @done="onEditDone" @reprocess="runProcessJob" @retried="onTimelineRetried" @redo-segment="redoSegment" @lane-open="onLaneOpen" @toggle-lane="toggleLane">
            <template #head>
              <b>时间轴编辑器</b>
              <span class="muted small" title="特效轨先选细分类型；改动立即保存，后处理在结束编辑时跑">拖动移动 / 边缘缩放 / 双击删除 / ＋ 在播放头添加</span>
            </template>
          </TimelineEditor>
        </div>

        <div class="mt-16"><CoverDesigner :key="'cv' + coverKey" :dir="dir" :pid="pid" @changed="refreshClipEntry(true)" /></div>

        <!-- 产物整行：成片 + 封面 + 输出目录 -->
        <div class="card mt-16 artifact-bar">
          <div class="row spread">
            <div class="row">
              <b class="t m-0">产物</b>
              <a v-if="renderedUrl && !renderedFailed" class="mini-btn" :href="renderedUrl" download :title="renderedFile">
                <Icon name="download" :size="12" /> 成片 {{ renderedFile }}
              </a>
              <span v-else class="muted small">未渲染</span>
              <a v-if="coverUrl && !coverFailed" class="mini-btn" :href="coverUrl" download :title="coverFile">
                <Icon name="download" :size="12" /> 封面 {{ coverFile }}
              </a>
              <span v-else class="muted small">无封面</span>
            </div>
            <!-- 打开输出目录：**只在本机（本地免登录）**渲染 —— 服务端模式下这个端点也不存在
                 （远端的人点一下、服务器上弹窗口是不可接受的）。 -->
            <button v-if="isLocal" class="mini-btn" @click="openOutDir">打开输出目录</button>
          </div>
          <div class="row mt-8">
            <img v-if="coverUrl && !coverFailed" :src="coverUrl" class="cover-thumb" alt="封面" @error="coverFailed = true">
            <video v-if="renderedUrl && !renderedFailed" :src="renderedUrl" controls playsinline class="video-bar" @error="renderedFailed = true"></video>
            <div v-if="!(coverUrl && !coverFailed) && !(renderedUrl && !renderedFailed)" class="muted small">渲染导出或生成封面后在这里查看</div>
          </div>
        </div>
      </template>
    </template>

    <!-- 渲染导出弹窗 -->
    <AppModal :show="renderModal" title="渲染导出" @close="renderModal = false">
      <label>输出文件（项目内相对路径，留空用默认）</label>
      <input v-model="renderOutput" placeholder="output/clips/clip-01/export.mp4">
      <p class="hint mt-6">
        只接受项目内相对路径<span class="muted">（同名自动改名）</span> <span class="mono">名字_1.mp4</span>。
      </p>
      <!-- 出片通道下拉已删：出片只有底座一条腿（本仓那条 headless Chrome 兜底腿 2026-10-01
           整条删除），留一个只有一个选项的下拉只会让人以为"还有得选、只是我没选对"。 -->
      <!-- 就绪位（M3）：底座不可用时**在点渲染之前**就说明白，而不是任务跑到一半才失败。
           只报"能不能用 + 什么原因"，列出的路径挂在 title 里（不占行内宽度）。 -->
      <p v-if="dhampirStatus && !dhampirStatus.ready" class="hint mt-6 warn">
        底座当前不可用：{{ dhampirProblems }}
      </p>
      <p v-else-if="dhampirStatus && dhampirStatus.ready" class="hint mt-6" :title="dhampirDetail">
        底座就绪 ✓（画面归它、音轨仍由本仓混）
      </p>
      <p v-else class="hint mt-6" title="出片走独立的 Dhampir 底座（本机 GPU + ffmpeg，画面归它、音轨仍由本仓混）；底座不可用会明确报错，不静默换路">
        底座未就绪
      </p>
      <template #footer>
        <button class="btn btn-primary" :disabled="scopeBusy" @click="doRender">开始渲染</button>
        <button class="btn" @click="renderModal = false">取消</button>
      </template>
    </AppModal>
  </div>
</template>

<script setup>
import { ref, computed, reactive, onMounted, onUnmounted, watch, nextTick } from 'vue'
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
import { projectData } from '../api/projectData'
import {
  fetchProject, fetchSubtitles, saveProjectPatch,
  initPolish, runLlm, runLlmSegment, runProcess, runRender,
  fetchDhampirStatus, fetchDhampirProject, fetchStickerSources, fetchFonts,
  fetchVideoLibrary,
  openPath, REGION_LABELS,
} from '../api/polish'
import { localNoAuthKnown } from '../stores/health'
import { ensureFontFace } from '../polish/fonts'
import { freeRect, regionRect } from '../polish/engine'
import { jobsApi, watchJob, busyOn, clipKey, kindText, QUEUED_HINT } from '../api/jobs'
import { useJobs } from '../composables/useJobs'

const route = useRoute()
const router = useRouter()
const routePid = route.params.pid || ''
const pid = ref(routePid)
const dir = ref('')
const clipName = computed(() => route.params.clip || '')
/** 当前已经加载完成的片段名：用于跳过 `load()` 自己 `router.replace` 触发的 watch。 */
const loadedClip = ref('')
// 记住最后操作过的片段：下次从侧栏进精修页自动回到它（每项目一个键）
const LAST_CLIP_KEY = computed(() => 'yekitrim.polish.last.' + pid.value)
watch(clipName, (name) => {
  if (name) { try { localStorage.setItem(LAST_CLIP_KEY.value, name) } catch {} }
  // 片段切换只改路由，必须在这里触发加载；但 `load()` 自己 replace 路由时不要再触发一次。
  if (name === loadedClip.value) return
  // ⚠️ **已经在编辑器里就别再走"骨架屏那一路"**（`load()` 非静默 ⇒ `loading=true` ⇒ 模板里那条
  // `v-if="loading"` 把整棵编辑器换成骨架屏并**卸载**它）。卸载的代价不只是滚动位置：
  // `PolishPreview` 里那块 `<canvas id="dhampir-canvas">` 一换节点就完了 —— 底座**一页只 attach
  // 一次、没有 detach** ⇒ 新 canvas 从没被 present 过 ⇒ **画面全黑，只能 F5**
  // （用户 2026-10-06 报的「在精修列表内切换精修视频，不会重载 Dhampir 导致黑屏」就是这个）。
  // 所以：编辑器已经在时**静默加载**，canvas 原地留着；Dhampir 那条腿由 `PolishPreview`
  // 自己 `watch(props.dir)` 换片段（见那边的 `switchDhampirClip`）。
  const hadEditor = ready.value
  if (hadEditor) resetClipUiState()
  load({ silent: hadEditor })
})

/** 切片段时**必须自己清掉**的那几件"按片段记账"的界面状态。
 *
 * ⚠️ 以前这些是**靠"整棵编辑器重挂"顺手清零**的；现在切片段不再重挂（上面那条），
 * 不清就会拿**上一个片段**的草稿去写**这个**片段 —— 最坏的是贴纸固定落点（点一下「保存落点」
 * 就把上个片段的坐标写进新片段）。
 *
 * 各组件各自的责任（**别在这里重复做**）：
 *   · `TimelineEditor`：自己 `watch(props.dir)` → `commitEdit` + `resetTimelineState` + `loadData`
 *     （所以**不能**靠 `segKey++` 重挂它 —— 那条路不会 `commitEdit`，会把没落盘的改动丢掉）；
 *   · `PolishPreview`：自己 `watch(props.dir)` 换片段（**绝不能**重挂 —— canvas！）；
 *   · `CoverDesigner`：**只能重挂** —— 它的配置只在 `onMounted` 读一次、没有 dir 监听，
 *     不重挂就是"看着上个片段的封面、一改就写进这个片段"（`coverKey++`）。
 * 任务状态栏（`job` / `jobClose`）**刻意不动**：那是全项目共享的队列显示，不是片段级草稿
 * （与改动前的行为一致）。 */
function resetClipUiState() {
  // ⚠️ **不要在这里 `coverKey++`**（2026-10-06 事故）：这一句发生在 `load()` 之前，而
  // `dir` 是 `doLoad` 里 `await loadClips()` **之后**才更新的 ⇒ 重挂出来的封面设计器
  // 挂载时拿到的还是**上一个片段的 dir**，读的是上一份 `[cover]`，而它的 600ms 防抖保存
  // 用的是更新后的 `props.dir` ⇒ **把上一个片段的整套封面配置写进这个片段**
  // （实测：两个片段的 `[cover]` 段 1073 字节逐字节相同、标题一模一样）。
  // 现在改由 `CoverDesigner` 自己 `watch(props.dir)` 换片段（见那边的说明）。
  selectRegion.value = ''
  pointEdit.value = false
  draftPos.value = { x: 50, y: 50 }
  renderModal.value = false
  renderOutput.value = ''
  canceling.value = false
  previewPlaying.value = false
  currentTime.value = 0
  openLane.value = ''
}
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
/** 封面设计器重挂载计数（封面任务完成后重挂拉新图） */
const coverKey = ref(0)
const renderModal = ref(false)
const renderOutput = ref('')
// 出片通道这个选择项**已删除**（2026-10-01）：出片只剩底座一条腿，本仓那条 headless
// Chrome 兜底腿整条删掉了，留一个只有一个选项的下拉只会让人以为"还有得选、只是我没选对"。
/** 底座出片通道的就绪位（M3）：打开渲染弹窗时抓一次；`null` = 还没问到（不显示任何提示）。 */
const dhampirStatus = ref(null)
const dhampirProblems = computed(() =>
  (dhampirStatus.value?.problems ?? []).map((p) => String(p).split('（')[0]).join('；'),
)
const dhampirDetail = computed(() =>
  [
    dhampirStatus.value?.binary ? `二进制：${dhampirStatus.value.binary}` : '',
    dhampirStatus.value?.translator ? `转译器：${dhampirStatus.value.translator}` : '',
    dhampirStatus.value?.font ? `字体：${dhampirStatus.value.font}` : '',
    dhampirStatus.value?.font_bold
      ? `字体粗体：${dhampirStatus.value.font_bold}`
      : dhampirStatus.value?.font
        ? '字体粗体：没有（700/600 的字重会退回常规体，成片笔画会比预览细）'
        : '',
    dhampirStatus.value?.node_version ? `Node：${dhampirStatus.value.node_version}` : '',
    dhampirStatus.value?.contract_version ? `project_schema=${dhampirStatus.value.contract_version}` : '',
  ]
    .filter(Boolean)
    .join('\n'),
)
/** 打开"渲染导出"时顺带问一次底座就绪位 —— 失败不影响出片流程（提示而已，`api()` 已记日志）。 */
async function openRenderModal() {
  renderModal.value = true
  dhampirStatus.value = null
  try {
    dhampirStatus.value = await fetchDhampirStatus(pid.value, dir.value)
  } catch {
    // 就绪位拿不到 = 不显示提示（点渲染时该报的错照旧会报）
  }
}
const timelineRef = ref(null)
const previewRef = ref(null)

// ---- 源素材帧率（成片轨的帧号域分母）----
// ⚠️ **不是**工程网格帧率（`timeline.timebase`，转译器给它 30fps 下限）：电台那种 5fps 素材
// 的网格会被抬到 30fps，拿网格帧率去算 `edlOutStart` 就是 6 倍错位，而且**不报错**。
// 真源是转译工程里那条视频素材自己的 `timebase`（与 `PolishPreview` 的 `dhampirSourceFps`
// **同一个值**，回落链也逐级相同：素材时基 → 工程网格 → 30）。
const srcFps = ref(30)
/** 是否拿到了真的源帧率（没有 ⇒ 成片轨的帧号不可信，面板里要如实说一句） */
const srcFpsKnown = ref(false)
async function loadSrcFps() {
  try {
    const res = await fetchDhampirProject(pid.value, dir.value)
    const project = res && res.project
    if (!project) return
    const tb = project.timeline && project.timeline.timebase
    const seqFps = tb && tb.num > 0 && tb.den > 0 ? tb.num / tb.den : 30
    const asset = ((project.assets || []).find((a) => a.kind === 'video')) || {}
    const atb = asset.timebase || {}
    srcFps.value = atb.num > 0 && atb.den > 0 ? atb.num / atb.den : seqFps
    srcFpsKnown.value = true
  } catch (e) {
    // 静默回落会让成片轨的 out_start 整体错位 —— 必须留痕。**但不拦页**：转译要跑 node，
    // 底座没装好的机器上它本来就拿不到，而那时成片轨也还没开。
    srcFpsKnown.value = false
    addClient('debug', '源帧率取不到（成片轨按 30fps 显示）：' + (e && e.message ? e.message : e), 'polish')
  }
}

const job = reactive({ id: '', kind: '', status: '', current: 0, total: 0, log: '', error: '' })
let jobClose = null
let jobTimer = null
let canceling = ref(false)
const jobStartMs = ref(0)
const jobElapsed = ref(0)

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
const jobStatusText = computed(() => {
  if (!job.id) return '空闲' // 状态栏常驻：没任务时也要有状态，不许整块消失
  return { queued: '排队中', running: '运行中', succeeded: '完成', failed: '失败', cancelled: '已取消' }[job.status] || job.status
})
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

// ---- 字幕字体（字体库 = <数据目录>/resources/fonts）----
// 值存 polish.toml 的 `[style.subtitle] font`，**逐片段**生效（每个片段一份 polish.toml）。
// 空串 = 回兜底字体栈；非空 = 字体库里的 id（文件名去扩展名）。
// 字体只影响预览与出片渲染 —— 不进任务、不触发 process（贴纸/运镜那些重算不读它）。
const fonts = ref([]) // 库里的字体（只读接口）
const fontsRoot = ref('') // 库目录（只有 admin 拿得到；非 admin 不显示那一行）
// 动态兜底字体栈（服务端算：首项随库里默认那支走）—— 透传给预览，`.cap` 栈与成片同源。
// 「设为默认」曾经对预览无效（预览写死 LXGW 开头的栈），就是这行的由来。
const fontFallback = ref('')
// emoji 补充槽位（服务端自动选，至多一支；库里没有就是空串）—— 透传给预览拼在栈尾。
const fontEmoji = ref('')
const fontsError = ref('')
const fontSaving = ref(false)
const subtitleFont = computed(() => cfg.value?.style?.subtitle?.font || '')
// 弹幕时间偏移（精修工程参数，[style.danmaku] offset_secs，默认 -3 = 整体提前 3 秒）。
// 老工程 / 手写 toml 缺这个键时按 serde 默认 -3 显示。
const dmOffset = computed(() => {
  const raw = cfg.value?.style?.danmaku?.offset_secs
  return typeof raw === 'number' && Number.isFinite(raw) ? raw : -3
})
const dmOffsetSaving = ref(false)
/** 下拉选项：**只有库内字体**（2026-10-02 起不再有「（系统默认）」这一项）。
 *
 *  ⚠️ **为什么去掉「（系统默认）」**：自带字体已改成**内嵌**（首次启动就装好），库永远是"有字可用"的
 *  —— 再留一个"回到系统字体栈"的选项，等于给用户一个**必然更差**的选择（系统栈里的
 *  ⚠️ **默认位（`is_default`）的作用（2026-10-06 更正）**：原来这里写的是「默认字体只做
 *  **初始化设定**：老工程没显式选过字体时由它兜底」—— **那句两个方向都错**：
 *  ① 初始化当时**根本不读**它（`cmd_init` 写死 `Noto Sans SC`）⇒ 用户设了默认，新切出的片段照旧不是它；
 *  ② "老工程没显式选过字体"这种工程几乎不存在（init 总会显式写 `font`）。
 *  现在的事实：① `polish init` **会**把默认位写进新工程的 `[style.subtitle] font`；
 *  ② `font` 为空的工程，渲染栈首由它决定。两条判据都由 `fonts::default_text_family` 供给。
 *  ⚠️ **空值不再作为可选字体出现**（2026-10-02 用户要求「这里应该只显示已有的字体」）：
 *  老工程没选过字体时配置里就是空串，那是**合法状态**，但它是「初始化」的表达、不是一种字体，
 *  摆在列表里会让人以为「跟随默认」是第四种字体。它仍会通过**占位项**照实显示（见下面 `if (!cur)`），
 *  用户一旦选了具体字体就再也看不到它。
 *
 *  ⚠️ 只列**真能用**的：`.ttc`（浏览器 @font-face 不认）与没装的自带字体都不进下拉，
 *  否则就是"选得出来、用不了"。
 *  ⚠️ **emoji 字体不进这个下拉**（字体职责分离）：它管"表情"、由系统**自动补充**
 *  （服务端 `fonts::emoji_font` 选一支，经 `/api/polish/fonts` 的 `emoji_family` 透传给预览），
 *  用户不该把一支没有中文字形的字体选成正文 —— 那会让整条字幕变成豆腐块。
 *  服务端 `set_default` / `resolve_font_plan` 也各挡一道（配置是文本文件，用户能手改）。 */
const fontOptions = computed(() => {
  const opts = []
  for (const f of fonts.value) {
    if (f.installed === false || /\.ttc$/i.test(f.file || '') || f.emoji) continue
    const base = f.family && f.family !== f.id ? `${f.family}（${f.id}）` : f.id
    opts.push({ value: f.id, label: f.is_default ? `${base}[默认]` : base })
  }
  const cur = subtitleFont.value
  // ⚠️ 空值只是**占位**、不是「一种可选字体」（老工程从没选过字体时就是它）：
  // 显示成「未指定 · 用默认文字字体」而非列表里的一档 —— 用户选任何具体字体后它自然消失。
  // 这样列表内容 = **库里已有的字体**，与用户口径一致。
  if (!cur) opts.unshift({ value: '', label: '未指定 · 用默认文字字体', placeholder: true })
  else if (!fonts.value.some((f) => f.id === cur)) opts.push({ value: cur, label: `${cur}（不在字体库）` })
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
  liver: '框住主播，运镜聚焦框中心',
  danmaku_area: '框住弹幕显示区（供运镜参考）'
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
    toast('选框已保存；该片段忙，稍后自动重算')
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
// 产物**只**用后端返回的实际文件名（含 _1/_2 版本）：没返回就是**还没渲染过**。
//
// ⚠️ 以前这里回退到一个固定名（`vtuber-clip.mp4`）—— 那个文件通常**不存在**，
// 于是页面挂上一个指向 404 的 `<video>`。而**媒体元素会无限重试**，
// 把浏览器对同一主机的连接数（HTTP/1.1 是 6 条）占住：
// 表现是**其它接口再也排不上队** —— `/api/jobs`（页面每 5~10s 轮询）稳定 10s 超时，
// `engine.js` 模块偶发载入失败，而**服务端日志显示它对每个请求都很快**（200/几十 ms）。
// 排查代价：重启服务、强刷新都无效（问题在浏览器侧），看起来像"服务端卡死"。
// 实测触发：`GET /api/media/2/output/clips/clip-25/vtuber-clip.mp4 → 404`，
// 同时 `/api/jobs → 请求超时（10000ms）` 连续出现。
const renderedFile = computed(() => clipEntry.value?.rendered_file || null)
const renderedUrl = computed(() => (renderedFile.value
  ? clipMediaUrl(pid.value, dir.value, renderedFile.value)
  : null))

const coverFailed = ref(false)
const renderedFailed = ref(false)
const coverFile = computed(() => clipEntry.value?.cover_file || 'cover.jpg')
const coverUrl = computed(() => clipMediaUrl(pid.value, dir.value, coverFile.value))

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

/**
 * 「磁盘上那份配置」—— Dhampir 热重载只认它（见 `PolishPreview` 的 `persistedConfig`）。
 *
 * 与 `cfg` 的**唯一区别**是它保证已落盘。两条更新时机：
 *   ① 时间轴/面板真的把改动 PUT 上去了（`onEditSaved`，由 TimelineEditor 的 `@saved` 来）；
 *   ② 任务跑完 `load({silent:true})` 从盘上读回新配置（下面 `doLoad` 里同步）。
 * 拖动过程中的 `onTimelinePreview` **不碰它** —— 那时磁盘上还是旧内容，
 * 拿它去触发重载只会让画面闪回改动前的样子。
 */
const persistedCfg = ref(null)

/**
 * 「磁盘上那份**字幕**」——与 `persistedCfg` 同一个道理，但要单独有一个，因为
 * **字幕根本不在 `polish.toml` 里**（它住 `data/subtitles.json`）：只改字幕时
 * `persistedCfg` 的内容指纹**一个字节都不变**，Dhampir 热重载的判据于是认为"什么都没变"、
 * 直接 `return` —— 用户看到的就是「改完字幕预览不刷新」。
 *
 * 两条更新时机与 `persistedCfg` 一致：① 编辑器落盘完成（`onEditSaved`）；
 * ② 任务跑完从盘上读回（下面 `doLoad` 里同步）。拖动中的 `onTimelinePreview` 不碰它。
 * 存的是**深拷贝**：镜像数组会被编辑器就地改，直接存引用等于"没落盘的内容也进了指纹"。
 */
const persistedSubs = ref(null)

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
      // **这就是"磁盘上那份"**：刚从盘上读回来的，天然已落盘 ⇒ Dhampir 热重载的
      // 第二条触发时机（任务跑完 `after: load()`，含 `process` 的校准结果）。
      persistedCfg.value = cfg.value
      title.value = cfg.value?.project?.name || cfg.value?.source?.video || '精修编辑器'
      ready.value = true
      cfgError.value = ''
      // 源素材帧率（成片轨 `edlOutStart` 的分母）。**失败不拦页** —— 拿不到就用 30，
      // 但必须**记一条**（静默回落会让成片轨的帧号整体错位，而它不报错）。
      loadSrcFps()
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
      // 刚从盘上读回来 ⇒ 这份字幕天然已落盘，交给预览当"磁盘版本"
      persistedSubs.value = subs.value.map((s) => ({ ...s }))
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
  return projectData.thumbUrl(pid.value, rank)
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
    if (!pid.value) { clips.value = []; clipsError.value = ''; return }
    const [list, polish] = await Promise.all([
      projectData.clips(pid.value, '读取切片列表'),
      projectData.polishMap(pid.value, '读取精修状态')
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
  // **多段片段**的段数（`clip.json` 的 `ranges`）：卡片上打「N 段」标记（>1 才算）
  const segs = Array.isArray(c.meta?.ranges) ? c.meta.ranges.length : 0
  return {
    name: c.name,
    title: c.meta?.title || c.name,
    state,
    chip: state === '已导出' ? 'g' : state === '精修中' ? 'p' : 'n',
    segments: segs > 1 ? segs : 0,
    // 各段源区间（按时间）：卡片上的悬停 title 用 `rangesHint` 一次列全（>1 段才有）
    ranges: segs > 1 ? c.meta.ranges : [],
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
/** 时间轴/面板「落盘完成」：把当前镜像当成磁盘版本交给预览。 */
function onEditSaved() {
  persistedCfg.value = cfg.value
  // 字幕走的是另一个文件（`data/subtitles.json`），所以要单独递一份 —— 不然
  // "只改字幕"这种编辑在指纹里看不出来（见 `persistedSubs` 的说明）。
  // 深拷贝：`subs` 是编辑器手里的镜像，不拷贝的话下一次未落盘的拖动会把它改掉。
  persistedSubs.value = subs.value.map((s) => ({ ...s }))
}

function onTimelinePreview(patch) {
  if (patch.segments && cfg.value) cfg.value = { ...cfg.value, segments: patch.segments }
  if (patch.subtitles) subs.value = patch.subtitles
  if (patch.mute_ranges && cfg.value) cfg.value = { ...cfg.value, mute_ranges: patch.mute_ranges }
  // 成片时间线（EDL）—— 与上面三条同一条链：**本地镜像、不落盘**。
  // 落盘由编辑器自己 debounce PUT 完成（`polish_project_put_locked` 的 `edl` 白名单支）。
  if (patch.edl && cfg.value) cfg.value = { ...cfg.value, edl: patch.edl }
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
  const list = pid.value ? await apiOr(projectData.paths.polishMap(pid.value), [], '刷新片段状态') : []
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
          // ⚠️ **任务完成后绝不重挂预览**（2026-10-05 修「LLM 剪辑后黑屏」）。
          //
          // 底座的 `dhampir_project_attach()` 是**一页只接一次**的语义
          //（`dhampir-wasm/src/timeline_host.rs`：`PROJECT_HOST` 已存在就直接返回
          // `{"already":true}`，**没有 detach 导出**），而 wasm 实例是页面级单例
          //（`mod.default()` 是 wasm-bindgen 的幂等 init）。所以只要 `#dhampir-canvas`
          // 这个**元素被换掉**（`:key` 重挂），新 canvas 就再也没被 attach 过 ——
          // 引擎继续把帧画在**已卸载的旧 canvas** 上，新 canvas 露出自己的 CSS
          // `background:#000` ⇒ **整块黑**，而工具条 fps/帧号照涨、控制台无报错。
          // 刷整页能好，是因为那样才有新的 wasm 实例（`PROJECT_HOST = None`）。
          //
          // 任务跑完要刷新的东西由**就地表驱动**：`after` 里的 `load({silent:true})`
          // 换掉 `persistedCfg` ⇒ `PolishPreview` 就地热重载（`reloadDhampirProject`，
          // 它存在就是为了"改效果/挪事件时间不用刷新页面"）。顺带白拿"播放位置不丢"。
          // 需要重挂的只有"换片段/换工程"这种**媒体真的换了**的场合 —— 那条走路由/`ready`。
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

/** 是否**本机**（本地免登录模式）—— 路由守卫在任何视图挂载之前已经 await 过 `localNoAuth()`，
 *  所以这里同步拿得到（与 `WorkbenchView` 的默认来源同一套口径）。 */
const isLocal = localNoAuthKnown()

/** 打开输出目录：浏览器打不开本地目录，交给**与本机同进程服务端**去弹文件管理器。
 *  ⚠️ 服务端模式下这颗按钮不渲染（端点也 404）—— 这里不做"退化成复制路径"的分支，
 *  留一个"点了没反应"的按钮比没有更糟；打开失败时把路径贴进提示，用户还能手动复制。 */
async function openOutDir() {
  const p = dir.value || ''
  if (!p) return
  try {
    await openPath(p)
  } catch (e) {
    toast('打开目录失败：' + e.message + '（' + p + '）', 'err')
  }
}

function doRender() {
  renderModal.value = false
  // `output_file` 必须是**项目内相对路径**（服务端 jail 校验）；留空则用 polish.toml 里的 project.output
  // ⚠️ 渲染与封面已解耦（2026-10-02）：导出只出成片，不再自动重生成封面 ——
  // 所以这里**不碰 coverKey**（封面设计器/产物图保持用户上一版封面，要更新去点「生成封面」）
  startJob('render', () => runRender(pid.value, dir.value, renderOutput.value.trim()), { after: () => { load({ silent: true }) } })
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
    fontFallback.value = r.fallback_stack || ''
    fontEmoji.value = r.emoji_family || ''
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
    toast(next ? `字幕字体已改为「${next}」` : '字幕字体已恢复为未指定（用默认文字字体）')
  } catch (err) {
    toast(err.message, 'err')
    // 保存失败要让下拉回到原值（否则界面在撒谎）
    await load({ silent: true })
  } finally {
    fontSaving.value = false
  }
}

// ---- 字幕样式（颜色 / 描边 / 大小 / 位置）----
// 落地全是 `polish.toml` 的 `[style.subtitle]`，**逐片段**生效（每个片段一份 polish.toml）。
// 这些字段**只影响渲染与预览**（process 的贴纸/运镜重算不读它们）⇒ 保存即生效、不用重跑任务。
//
// ⚠️ **为什么每一项都要"是否设过"的判据**：`size`/`color`/`stroke` 在配置里有 serde 默认值，
//    前端读回来的 72 / #dcbda0 / "12px #403c3b" **分不出**"用户显式写的就是它"与"从没写过"。
//    而 `position_pct` 是 `Option`：**不写 = 走历史那条写死的落点公式**、写了 = 走百分比，
//    两者在成片侧**语义不同**（`120/1080 = 11.11% ≠ 12%`）。
//    ⇒ 所以：**只有用户真的改过才发键**，没改一个键都不发 —— 老工程 polish.toml 逐字节不动。
//
// ⚠️ **读失败 ⇒ 暂停写入**（本仓"静默丢改动"的老根因）：`cfg` 读失败时为 null，
//    此时控件显示的会是兜底值、而任何一次写盘都会把磁盘上的样式清成默认。
//    闸门 = `styleWriteBlocked`（照 `TimelineEditor::loadError` 那套）。

/** 样式块的写入门闸：配置还没读到（或读到的是假空态）时**一个键都不许写**。 */
const styleWriteBlocked = computed(() => !!cfgError.value || !cfg.value)
/** 配置读取失败的原因（与 `cfgError` 同源，这里只是给样式块一条自己的可见错误条）。
 *  ⚠️ 不许伪装成正常：读不到就出错误条 + 重试，绝不拿默认值写回磁盘。 */
const styleReadError = computed(() => (styleWriteBlocked.value ? (cfgError.value || '尚未读到配置') : ''))

/** 字幕正文颜色。缺省 = 配置默认（`#dcbda0`），照实显示但**不算"用户设过"**。 */
const subtitleColor = computed(() => cfg.value?.style?.subtitle?.color || '#dcbda0')

/** `stroke` 是**一个字符串装两个值**（格式 `"{宽}px {颜色}"`，默认 `"12px #403c3b"`）。
 *  ⚠️ 拆/拼都要**容错**：手改坏的 toml 不该让整个面板报错，更不该被我们**写坏格式**。
 *  解析不出来 ⇒ 回退到默认（12px / #403c3b）+ `strokeParsed=false`，
 *  此时两个控件**只显示、不许写**（否则一写就把用户原来的怪格式覆盖掉了）。 */
const STROKE_DEFAULT_WIDTH = 12
const STROKE_DEFAULT_COLOR = '#403c3b'
/** `"{宽}px {颜色}"` → `{w, c}`；认不出返回 null（调用方回退默认）。
 *
 *  ⚠️ **颜色统一规整成 6 位**（`toSixHex`）：读盘时若手写的是 `12px #fff`，
 *  转译器那句 `match(/#[0-9a-fA-F]{6}/)` 匹配不到 → **静默用默认色**。
 *  这里展开成 `#ffffff` 后，用户一改宽度/颜色就会把它**写成转译器认得的形式**（顺手修好）。
 *  只读显示时也照实显示展开后的值（与成片一致），不留"看着是白、出片是灰"的错觉。 */
function parseStroke(raw) {
  if (typeof raw !== 'string') return null
  const m = raw.trim().match(/^(-?\d+(?:\.\d+)?)\s*px\s+(#[0-9a-fA-F]{3,8})$/)
  if (!m) return null
  const w = Number(m[1])
  if (!Number.isFinite(w) || w < 0) return null
  const c = toSixHex(m[2])
  if (!c) return null
  return { w, c }
}
/** 当前 stroke 的解析结果；null = 格式认不出（→ 两个控件退化成只读回退显示）。 */
const subtitleStrokeParsed = computed(() => parseStroke(cfg.value?.style?.subtitle?.stroke))
const subtitleStrokeWidth = computed(() =>
  subtitleStrokeParsed.value ? subtitleStrokeParsed.value.w : STROKE_DEFAULT_WIDTH)
const subtitleStrokeColor = computed(() =>
  subtitleStrokeParsed.value ? subtitleStrokeParsed.value.c : STROKE_DEFAULT_COLOR)
/** 两个描边控件可否写：⚠️ 格式认不出 / 从没设过描边时**都不许写** ——
 *  写下去会用一个规整的新串覆盖掉用户原来的（可能是手写的）格式，而且不报错。 */
const strokeEditable = computed(() => !styleWriteBlocked.value && subtitleStrokeParsed.value !== null)
/** 描边那两项的悬停说明（`title`，解释走这里而不是行内文案）。 */
const strokeColorHint = computed(() => {
  if (!cfg.value?.style?.subtitle?.stroke) return '未设置描边（用默认 12px #403c3b）'
  if (!subtitleStrokeParsed.value) return '当前描边格式认不出，只读显示；要改请手改 polish.toml'
  return '字幕描边的颜色。颜色与宽度同存一个键（"{宽}px {颜色}"）'
})
const strokeWidthHint = computed(() => {
  if (!cfg.value?.style?.subtitle?.stroke) return '未设置描边（用默认 12px #403c3b）'
  if (!subtitleStrokeParsed.value) return '当前描边格式认不出，只读显示；要改请手改 polish.toml'
  return '近似值：底座只有描边开/关，成片侧可能不随宽度线性变化；预览侧是精确的'
})

/** 字幕字号（px）。默认 72。 */
const subtitleSize = computed(() => {
  const v = cfg.value?.style?.subtitle?.size
  return typeof v === 'number' && Number.isFinite(v) ? v : 72
})
/** `size` 是否被显式设过（同 color：区分不了就按"没设过"处理，避免凭空写键）。 */
const subtitleSizeSet = computed(() => typeof cfg.value?.style?.subtitle?.size === 'number')

/** 字幕垂直位置（距底百分比，0~50）。默认 12 —— 但**默认值不等于"设过"**：
 *  `position_pct` 缺省是 `null`（走历史公式），要区分开，否则一进页面就把它钉成 12。 */
const subtitlePosSet = computed(() => {
  const v = cfg.value?.style?.subtitle?.position_pct
  return typeof v === 'number' && Number.isFinite(v)
})
const subtitlePosPct = computed(() => (subtitlePosSet.value ? cfg.value.style.subtitle.position_pct : 12))
const positionHint = computed(() =>
  subtitlePosSet.value
    ? '字幕带底边距画布底边的百分比（0 ~ 50）；点「位置→默认」可回到未设置'
    : '未设置：走历史落点公式（与 12% 略有差别）。拖动即改为按百分比')

const styleSaving = ref(false)

/** 提交样式 patch：成功后把镜像与 Dhampir 那条腿一起刷新，失败则回读磁盘（界面不许撒谎）。
 *  ⚠️ 写盘走 `PUT /api/polish/project`（服务端 `update_polish_toml` 的逐字段白名单），
 *     不做"开头 load、结尾整份写回"。 */
async function saveSubtitleStyle(patch, okMsg) {
  if (styleWriteBlocked.value) { toast('配置未读到，已暂停保存', 'err'); return }
  styleSaving.value = true
  try {
    await saveProjectPatch(pid.value, dir.value, patch)
    // 已落盘 ⇒ 同步给 Dhampir 那条腿（它只认磁盘，认的是 persistedConfig）+ 重取配置
    await load({ silent: true })
    persistedCfg.value = cfg.value
    if (okMsg) toast(okMsg)
  } catch (err) {
    toast(err.message, 'err')
    // 保存失败要让控件回到磁盘上的值（否则界面在撒谎）
    await load({ silent: true })
  } finally {
    styleSaving.value = false
  }
}

/** 把任意 hex 规整成 **6 位**（`#rrggbb`）。
 *
 *  ⚠️ **为什么必须补到 6 位**（2026-10 跨层核对发现，实测）：转译器的
 *  `tools/polish-to-dhampir.mjs::parseStroke` **只认 6 位** —— 它那句是
 *  `raw.match(/#[0-9a-fA-F]{6}/)`。而我们若把 `#fff` 原样写进 stroke，
 *  转译器**匹配不到颜色** → 静默回落到**默认色**（`#403c3b` 深灰）：
 *  用户选了白色描边，成片却是深灰、**不报错**。
 *  所以这里统一展开（`#abc` → `#aabbcc`、`#abcd` → 去 alpha 取 RGB），
 *  与 `hexColor()` 的展开口径一致。 */
function toSixHex(c) {
  const s = String(c || '').trim()
  const m = s.match(/^#?([0-9a-fA-F]{3,8})$/)
  if (!m) return null
  let h = m[1]
  if (h.length === 3 || h.length === 4) h = [...h].map((ch) => ch + ch).join('')
  if (h.length === 8) h = h.slice(0, 6) // 带 alpha 的取 RGB（stroke 只有一色，底色统一 alpha=255）
  if (h.length !== 6) return null
  return '#' + h.toLowerCase()
}

/** 把 `{w, c}` 拼回 stroke 那个字符串（格式 `"{宽}px {颜色}"`，与默认值同形）。
 *  ⚠️ 宽度取整 + clamp、颜色一律**规整成 6 位 hex** —— 这是**唯一**的写入口，
 *  别在别处再拼一次（拼歪了就是把用户的描边写坏，而且不报错）。 */
function composeStroke(w, c) {
  const raw = Number(w)
  const width = Number.isFinite(raw) ? Math.max(0, Math.min(24, Math.round(raw))) : STROKE_DEFAULT_WIDTH
  const color = toSixHex(c) || STROKE_DEFAULT_COLOR
  return `${width}px ${color}`
}

/** 字幕颜色。 */
async function onSubtitleColorChange(e) {
  const v = String(e.target.value || '').trim()
  if (!/^#[0-9a-fA-F]{3,8}$/.test(v) || v.toLowerCase() === subtitleColor.value.toLowerCase()) return
  await saveSubtitleStyle({ subtitle_color: v.toLowerCase() }, '字幕颜色已保存')
}

/** 描边颜色：只换 stroke 的**颜色部分**，宽度沿用当前解析值（认不出时不写）。 */
async function onSubtitleStrokeColorChange(e) {
  const cur = subtitleStrokeParsed.value
  if (!cur) { toast('当前描边格式认不出，已拒绝写入', 'err'); return }
  const v = String(e.target.value || '').trim()
  if (!/^#[0-9a-fA-F]{3,8}$/.test(v) || v.toLowerCase() === cur.c.toLowerCase()) return
  await saveSubtitleStyle({ subtitle_stroke: composeStroke(cur.w, v) }, '描边颜色已保存')
}

/** 描边宽度：只换 stroke 的**宽度部分**。⚠️ 成片侧是近似值（底座只有描边开/关）。 */
async function onSubtitleStrokeWidthChange(e) {
  const cur = subtitleStrokeParsed.value
  if (!cur) { toast('当前描边格式认不出，已拒绝写入', 'err'); return }
  const raw = String(e.target.value ?? '').trim()
  const n = Number(raw)
  if (raw === '' || !Number.isFinite(n)) { e.target.value = String(cur.w); return }
  const next = composeStroke(n, cur.c)
  e.target.value = String(Math.max(0, Math.min(24, Math.round(n))))
  if (next === composeStroke(cur.w, cur.c)) return
  await saveSubtitleStyle({ subtitle_stroke: next }, '描边宽度已保存（成片侧为近似）')
}

/** 字幕大小。⚠️ 清空 / 非数字一律还原显示、**不写盘**（否则退格清空的一瞬间就把值变成 8）。 */
async function onSubtitleSizeChange(e) {
  const raw = String(e.target.value ?? '').trim()
  const n = Number(raw)
  if (raw === '' || !Number.isFinite(n)) { e.target.value = String(subtitleSize.value); return }
  const next = Math.min(400, Math.max(8, Math.round(n)))
  e.target.value = String(next)
  if (subtitleSizeSet.value && next === cfg.value.style.subtitle.size) return
  await saveSubtitleStyle({ subtitle_size: next }, `字幕大小已保存（${next}px）`)
}

/** 位置滑杆：拖动只更新内存镜像（预览当场重算），松手才落盘。 */
function onSubtitlePosInput(e) {
  const n = Number(e.target.value)
  if (!Number.isFinite(n)) return
  const v = Math.min(50, Math.max(0, n))
  cfg.value = { ...cfg.value, style: { ...(cfg.value?.style || {}), subtitle: { ...(cfg.value?.style?.subtitle || {}), position_pct: v } } }
}

// ── 运镜强度（§18-C）───────────────────────────────────────────────
//
// 只暴露配置、**不改默认**。默认值就是 `[style.camera]` 里的历史值（中 1.3 / 近 1.5 /
// 画中画 0.55），没写过这个键的工程读出来还是这三个数 ⇒ 存量工程观感零变化（R4）。
//
// ⚠️ **夹取范围与后端 `api/polish.rs` 的 `camera` 分支必须一致**。两边不一致的症状是
// "滑块拖到 3.0、保存后弹回 2.5"（后端夹了、前端没夹）或反过来"前端夹了、后端存了 5.0"。
// 上下限的依据（露边 + 语义）见后端那段注释 —— 这里只镜像，别在别处再写一份。
const ZOOM_FIELDS = [
  { key: 'zoom_closeup', label: '特写', min: 1.0, max: 3.0, title: '特写缩放（camera zoom=特写）。默认 1.8，比近景更近一档' },
  { key: 'zoom_close', label: '近景', min: 1.0, max: 3.0, title: '近景特写缩放（camera zoom=近）。默认 1.5，越大越近' },
  { key: 'zoom_mid', label: '中景', min: 1.0, max: 3.0, title: '中景缩放（camera zoom=中）。默认 1.3' },
  { key: 'zoom_wide', label: '画中画', min: 0.05, max: 1.0, title: '画中画缩放（camera zoom=远），小于 1 时四周留黑边。默认 0.55' },
]
const zoomFields = ZOOM_FIELDS

/** 读当前生效值；没写过这个键就回落到历史默认（与后端同一批数）。 */
const ZOOM_DEFAULTS = { zoom_closeup: 1.8, zoom_close: 1.5, zoom_mid: 1.3, zoom_wide: 0.55 }
function zoomValue(key) {
  const v = cfg.value?.style?.camera?.[key]
  return Number.isFinite(v) ? v : ZOOM_DEFAULTS[key]
}

/** 拖动只更新内存镜像（预览当场重算），松手才落盘 —— 与字幕位置滑杆同一套理由。 */
function onZoomInput(key, e) {
  const n = Number(e.target.value)
  if (!Number.isFinite(n)) return
  cfg.value = {
    ...cfg.value,
    style: { ...(cfg.value?.style || {}), camera: { ...(cfg.value?.style?.camera || {}), [key]: n } },
  }
}

/** 松手 → 落盘。三档一起提交（后端那个分支收整块 `camera`）。 */
async function onZoomCommit(key, e) {
  const n = Number(e.target.value)
  if (!Number.isFinite(n)) return
  const f = ZOOM_FIELDS.find((x) => x.key === key)
  const next = Math.min(f.max, Math.max(f.min, Math.round(n * 100) / 100))
  const camera = {
    zoom_mid: zoomValue('zoom_mid'),
    zoom_close: zoomValue('zoom_close'),
    zoom_wide: zoomValue('zoom_wide'),
    [key]: next,
  }
  if (next === zoomValue(key) && camera[key] === cfg.value?.style?.camera?.[key]) return
  await saveSubtitleStyle({ camera }, `${f.label}强度已保存（${next.toFixed(2)}）`)
}

/** 位置滑杆松手 → 落盘。 */
async function onSubtitlePosChange(e) {
  const n = Number(e.target.value)
  if (!Number.isFinite(n)) return
  const v = Math.min(50, Math.max(0, n))
  if (subtitlePosSet.value && v === cfg.value.style.subtitle.position_pct) return
  await saveSubtitleStyle({ subtitle_position_pct: v }, `字幕位置已保存（距底 ${v}%）`)
}

/** 位置数字框直填：清空 / 非数字还原、不写。 */
async function onSubtitlePosType(e) {
  const raw = String(e.target.value ?? '').trim()
  const n = Number(raw)
  if (raw === '' || !Number.isFinite(n)) { e.target.value = String(subtitlePosPct.value); return }
  const v = Math.min(50, Math.max(0, n))
  e.target.value = String(v)
  if (subtitlePosSet.value && v === cfg.value.style.subtitle.position_pct) return
  await saveSubtitleStyle({ subtitle_position_pct: v }, `字幕位置已保存（距底 ${v}%）`)
}

/** 「位置→默认」：**发 `null` = 显式删除该键** ⇒ 回到历史落点公式（与"写 12"语义不同）。 */
async function restoreSubtitlePos() {
  if (!subtitlePosSet.value) return
  await saveSubtitleStyle({ subtitle_position_pct: null }, '字幕位置已恢复为未设置（走历史落点）')
}

/** 「大小→默认」：⚠️ 配置里 **区分不了**"显式写了 72"与"从没写过"（serde 默认值就是 72），
 *  所以这里**不能发 72** —— 发出去就是把一个用户可能从没设过的值钉死成显式字段。
 *  诚实做法：提示手动改回，**不假装能精确回退**（比假装能回退更诚实）。 */
function restoreSubtitleSize() {
  toast('字号无法精确回退（72 是默认值，配置区分不了"没写过"与"写了 72"）；请手动改回 72')
}

/** 弹幕时间偏移（精修工程参数，[style.danmaku] offset_secs）。
 *  拖动（input）只更新内存镜像 cfg → DOM/GSAP 预览即时重算（不落盘、不触发 Dhampir 重载）；
 *  松手（change）才落盘 + 更新 persistedCfg 触发 Dhampir 那条腿重拉（贴纸来源那套流程）。 */
function onDanmakuOffsetInput(e) {
  const next = Number(e.target.value)
  if (!Number.isFinite(next) || next === dmOffset.value) return
  cfg.value = { ...cfg.value, style: { ...(cfg.value?.style || {}), danmaku: { ...(cfg.value?.style?.danmaku || {}), offset_secs: next } } }
}

/** 行尾数字框直填：只在 [-10, 10] 上夹、**不吸附滑块步进**（滑块才有栅格）；
 *  清空/非数字还原显示、不写模型（否则退格清空的一瞬间就把值变成 0）。
 *  夹回来的值可能等于原值（填 99 被夹到 10）→ 模型没变、Vue 不会回写 DOM，手动同步一次。 */
function onDanmakuOffsetType(e) {
  const raw = String(e.target.value ?? '').trim()
  const n = Number(raw)
  if (raw === '' || !Number.isFinite(n)) {
    e.target.value = String(dmOffset.value)
    return
  }
  const next = Math.min(10, Math.max(-10, n))
  e.target.value = String(next)
  cfg.value = { ...cfg.value, style: { ...(cfg.value?.style || {}), danmaku: { ...(cfg.value?.style?.danmaku || {}), offset_secs: next } } }
  saveDanmakuOffset(next)
}

/** 落盘 + 触发 Dhampir 腿重载（滑块 change 与直填框共用）。 */
async function saveDanmakuOffset(next) {
  dmOffsetSaving.value = true
  try {
    await saveProjectPatch(pid.value, dir.value, { danmaku_offset_secs: next })
    cfg.value = { ...cfg.value, style: { ...(cfg.value?.style || {}), danmaku: { ...(cfg.value?.style?.danmaku || {}), offset_secs: next } } }
    // 已落盘 ⇒ 同步给 Dhampir 那条腿（它只认磁盘，认的是 persistedConfig）。
    persistedCfg.value = cfg.value
    toast(`弹幕偏移已保存（${next}s）`)
  } catch (err) {
    toast(err.message, 'err')
    // 保存失败要让滑块/数字框回到磁盘上的值（否则界面在撒谎）
    await load({ silent: true })
  } finally {
    dmOffsetSaving.value = false
  }
}

/** 弹幕字体（`[style.danmaku] font`）：与字幕字体同一套 —— 写 id、空串 = 跟随字幕那条字体栈。
 *  ⚠️ 它还决定弹幕的**丈量宽度**（滚动时长按宽度算）：预览 DOM、模板 canvas、转译器 ASS 三处
 *  都必须用同一个族，否则弹幕没滚到边就消失。 */
const danmakuFont = computed(() => cfg.value?.style?.danmaku?.font || '')
/** 与字幕字体共用一个库；只多一项「跟字幕」（空串 = 继承字幕那条字体栈）。 */
const danmakuFontOptions = computed(() => {
  const known = new Set(fonts.value.map((f) => f.id))
  const opts = [{ value: '', label: '跟字幕' }]
  for (const o of fontOptions.value) {
    if (o.placeholder || o.value === '') continue
    if (!known.has(o.value) && o.value !== danmakuFont.value) continue
    opts.push(o)
  }
  const cur = danmakuFont.value
  if (cur && !known.has(cur) && !opts.some((o) => o.value === cur)) opts.push({ value: cur, label: `${cur}（不在字体库）` })
  return opts
})
async function onDanmakuFontChange(e) {
  const next = e.target.value
  if (next === danmakuFont.value) return
  fontSaving.value = true
  try {
    await saveProjectPatch(pid.value, dir.value, { danmaku_font: next })
    if (next) await ensureFontFace(pid.value, dir.value, next, { onDebug: (m) => addClient('debug', m, 'polish') })
    cfg.value = { ...cfg.value, style: { ...(cfg.value?.style || {}), danmaku: { ...(cfg.value?.style?.danmaku || {}), font: next } } }
    // 已落盘 ⇒ 同步给 Dhampir 那条腿（它只认磁盘）
    persistedCfg.value = cfg.value
    await load({ silent: true })
    toast(next ? `弹幕字体已改为「${next}」` : '弹幕字体已恢复为跟随字幕')
  } catch (err) {
    toast(err.message, 'err')
    await load({ silent: true })
  } finally {
    fontSaving.value = false
  }
}

// ⚠️ 「音效」那一段（`sfxCfg` / `sfxBusy` / `patchSfx`）已随那个面板分组一起删除
// （用户 2026-10-06：删除音效的音量设置）。全局默认音量仍在 `polish.toml` 的 `[sfx] volume`，
// 只是界面上不再有那条滑杆 —— 事件自己的 `volume` / `sfx_volume` 优先，这条滑杆本来就改不动它们。

async function onDanmakuOffsetChange(e) {
  // ⚠️ 这里**不能**拿 next 与 dmOffset.value 比较来去重：input 阶段已经把镜像
  // 更新成 next 了，那样比永远相等 ⇒ 永不落盘。change 本身只在值被改动后触发，
  // 偶发的原值 change 就是多一次幂等保存，无害。
  const next = Number(e.target.value)
  if (!Number.isFinite(next)) return
  await saveDanmakuOffset(next)
}

async function patchStickerSource(list) {
  try {
    await saveProjectPatch(pid.value, dir.value, { sticker_source: list })
    cfg.value = { ...cfg.value, sticker_source: list }
    // 已落盘 ⇒ 同步给 Dhampir 那条腿（它只认磁盘）。这里原先**只有** `cfg.value`，
    // 于是"贴纸来源"这类改动永远触不到引擎重载。
    persistedCfg.value = cfg.value
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
// 右栏**没有页签了**（2026-10-06）：原来「导出 / 画面构成」两半合并成一块面板，
// 而"导出"那一半的设置（字幕字体 / 弹幕字体与偏移 / 贴纸来源 / 区域与落点）全在**轨道设置**里。
const stageBusy = ref(false)
const stageCfg = computed(() => cfg.value?.stage || {
  bg_mode: 'blur', bg_color: '#000000', blur_scale: 1, scale: 1, padding: 0,
})

// ---- 主画面位置：**两轴各一条五档**（0/25/50/75/100），唯一权威是契约里的两轴百分比 ----
// 老工程的单值 `position_pct` / 老枚举 `position` 由 Rust `resolve_stage` 一次性折算到两轴
// （`auto` → 50 居中），所以这里只读 `_canvas.stage.position_x_pct` / `position_y_pct`。
const AXIS_LABELS = {
  position_x_pct: ['靠左', '偏左', '居中', '偏右', '靠右'],
  position_y_pct: ['靠上', '偏上', '居中', '偏下', '靠下'],
}
/** 两轴的**五档滑杆**。`free_x` / `free_y`（该轴可平移余量）由 Rust `resolve_stage` 算出来 ——
 *  界面**只读它**决定置灰，绝不自己按 `axis`/`scale` 再推一遍（那是第二份公式）。
 *  `free == 0` = 画面正好贴满那条轴 ⇒ 滑杆无效；置灰 + 行内写明原因（放大后才有效）。 */
const positionAxes = computed(() => ['position_x_pct', 'position_y_pct'].map((key) => {
  const st = cfg.value?._canvas?.stage || {}
  const pctRaw = Number(st[key])
  const pct = Number.isFinite(pctRaw) ? Math.max(0, Math.min(100, pctRaw)) : 50
  const free = Number(key === 'position_x_pct' ? st.free_x : st.free_y)
  const inert = Number.isFinite(free) && free === 0
  const i = Math.max(0, Math.min(4, Math.round(pct / 25)))
  const horizontal = key === 'position_x_pct'
  return {
    key,
    pct,
    inert,
    label: horizontal ? '水平' : '垂直',
    text: AXIS_LABELS[key][i],
    hint: (horizontal ? '横向' : '纵向') + '已贴满，放大后才能平移',
  }
}))
/** 源与画布**同比**（交叉相乘；与 Rust 推 axis 同一条判据）。读不到源尺寸 ⇒ 不算同比（照常显示）。 */
const sameRatio = computed(() => {
  const c = cfg.value?._canvas
  const s = c?.src
  if (!c || !s?.width || !s?.height) return false
  return s.width * c.height === s.height * c.width
})
/** 位置那组：同比且**没放大**时整组隐藏（铺满 ⇒ 两条滑杆都无效、调了看不出效果）；
 *  ⚠️ **放大后必须出现** —— 否则 `scale>1` 的裁切取景能力在这类工程里根本没有入口。
 *  缩放那条**不在这组里**（任何比例下都有用：>1 = 放大裁切），常驻。 */
const showStageGeom = computed(() => !sameRatio.value || stageScaleNum.value !== 1)
/** 背景：契约说 `bg=none`（横画布铺满、画面铺满画布）时调它看不出效果 ⇒ 隐藏；
 *  竖画布铺满仍铺衬底（`resolve_stage` 里那条画布方向的门控），照常显示。 */
const stageBgVisible = computed(() => {
  const st = cfg.value?._canvas?.stage
  if (!st) return true
  return (st.bg || 'none') !== 'none'
})
// ---- 轨道显示开关（`[stage.lanes]`：时间轴每条轨道前那枚眼睛）----
// 「哪一类渲不渲染」的**唯一判据在 Rust**（`ProjectFile::lane_enabled`）：显式开关优先，
// 缺字段时按 素材竖/画布竖 取默认（运镜与视觉特效默认关、音效跟画布、字幕弹幕贴纸恒开）。
// 所以这里读的是 Rust 注入的 **`_lanes`**（已解析），前端**不再自己推一遍默认值** ——
// 推第二遍就必然两边不一致，而且不报错（user 口径：竖屏素材默认关运镜/特效，但要看得见、点得开）。
const lanesOn = computed(() => cfg.value?._lanes || {})
/** 点眼睛 = 把这条轨道的显式开关翻面（`[stage]` 整块提交，与其它画面构成项同一条链）。 */
function toggleLane(lane) {
  const cur = lanesOn.value[lane] === true
  patchStage({ lanes: { ...(stageCfg.value.lanes || {}), [lane]: !cur } })
}
/** 时间轴上点轨道名 ⇒ 打开「轨道设置」面板（**一个面板装全部轨道**），并滚到那一段。
 *
 * ⚠️ `openLane` 的语义是"**焦点**"（哪一段被高亮 + 往哪儿滚），不是"显示哪一段"：
 * 面板里**六段全在**，点哪个轨道都看同一个面板（用户 2026-10-06 口径："点哪个轨道都是
 * 同一个面板，这样好操作"）。再点同一条轨道 = 收起（回到块的属性面板）。 */
const openLane = ref('')
/** 右栏那块卡片的页签：'project' = 项目设置（含 LLM 剪辑 / 渲染导出 / 任务栏），
 *  'canvas' = 画面尺寸（缩放 / 主画面位置 / 背景），
 *  'intro' = 片头片尾（用户 2026-10：「片头片尾放在一个单独的页面」）。
 *  **三个页签同一个位置**切换 —— 用户 2026-10-06：「画面尺寸改成之前的是说切换面板的那种，
 *  位置和项目设置是一个位置」。 */
const sideTab = ref('project')
/** 任务栏展开态：**默认折叠**（用户 2026-10-06：「默认折叠，只显示当前是什么任务」）。
 *  展开时绝对定位浮在上层，不参与布局 ⇒ 不挤占下方控件。 */
const jobOpen = ref(false)
/** 每段 DOM（函数 ref）——滚到它用。普通对象即可（不参与渲染）。 */
const laneGroupEls = {}
function onLaneOpen(lane) {
  if (!lane || lane === openLane.value) {
    openLane.value = ''
    return
  }
  openLane.value = lane
  // 滚到那一段（`block:'nearest'` = 只在看不见时滚，不会把已经可见的那段顶走）
  nextTick(() => {
    // ⚠️ `segments`（结构轨的纯信息组已删）与 `sfx`（音量组已删）**没有对应分组**：
    //    点这两条轨道的名字仍要打开面板，但没东西可滚 —— 查不到就退回面板顶部，
    //    别停在上一次的位置让标题高亮指向别处。
    const el = laneGroupEls[lane] || laneGroupEls.__top
    if (el && typeof el.scrollIntoView === 'function') el.scrollIntoView({ block: 'nearest' })
  })
}
// ⚠️ 这里原来还有两张"视觉特效 / 音效计数"的表（从 `spec/effects.json` 派生）。
// 用户 2026-10-06 第二轮："特效事件/这条轨道上还没有特效事件/开关只影响渲染…… 音效事件 0 条/
// 逐条在事件的属性面板里改 这两个根本没用" ⇒ **整块删掉**（连读数一起）：面板只留控件。
const bgOptions = [
  { value: 'color', label: '纯色' },
  { value: 'blur', label: '模糊' },
]
/** 背景按钮的高亮态：'black' 并入纯色（渲染结果一样），`auto`/认不出的值都当**模糊**
 *  —— 与 Rust `stage_bg_mode` 的等价关系一致（2026-10-02 删掉「自动」档之后）。 */
const stageBgDisplay = computed(() => {
  const m = String(stageCfg.value.bg_mode || '').toLowerCase()
  return m === 'color' || m === 'black' ? 'color' : 'blur'
})
const stageBgColor = computed(() => stageCfg.value.bg_color || '#000000')
/** 缩放：滑块与数字框同源；手填的值夹 `[0.1, 4]`、非数字不写（与 `resolve_stage` 的夹取一致） */
const stageScaleNum = computed(() => {
  const v = Number(stageCfg.value.scale)
  return Number.isFinite(v) && v > 0 ? Math.max(0.1, Math.min(4, v)) : 1
})
function clampScale(v) {
  const n = Number(v)
  if (!Number.isFinite(n)) return stageScaleNum.value
  return Math.max(0.1, Math.min(4, n))
}
/** 保存 [stage]：与「字幕字体」同一套 —— 先乐观改本地配置（控件当场响应），再静默重取拿到新的 `_canvas` */
async function patchStage(patch) {
  if (!ready.value || stageBusy.value) return
  const next = { ...stageCfg.value, ...patch }
  // ⚠️ `null`/`undefined` 必须在提交前摘掉：老工程没有两轴百分比（`_canvas` 拿不到时是 50 的
  // 兜底值，不是 null），而 `patch_config_file` 是按值深合并的，透传 `null` 会写进 TOML
  // （TOML 没有 null 这种值）。
  for (const k of Object.keys(next)) {
    if (next[k] === null || next[k] === undefined) delete next[k]
  }
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

// ---- 片头 / 片尾（[intro_outro]）----
//
// 用户口径：「片头片尾在项目设置内可以选择是否开启，然后可以选择素材库内的视频内容」，
// 且入出点可裁剪、缺省播完整段。
//
// ⚠️ 「开」= 选了素材（`asset_id` 非空），**没有第二个 `enabled` 字段** ——
//    两个真源必然分叉（Rust 侧 `IntroOutroClip::is_enabled` 是唯一判据）。
// ⚠️ 写盘走 `saveProjectPatch`（与 [canvas] 同一条路），因为 `[intro_outro]` 是
//    `polish.toml` 的一节、由 `ProjectFile` 反序列化 —— 不走它就得自己拼 toml。
// ⚠️ `in_sec/out_sec` 缺省 0 表示「整段/播到结尾」，所以**新建一侧时不要填 0 以外的值**。
const INTRO_OUTRO_SIDES = [
  { key: 'intro', label: '片头' },
  { key: 'outro', label: '片尾' }
]
const ioBusy = ref(false)
const ioError = ref('')
const videoLibrary = ref([])

const introOutro = computed(() => ({
  intro: { asset_id: '', in_sec: 0, out_sec: 0, ...(cfg.value?.intro_outro?.intro || {}) },
  outro: { asset_id: '', in_sec: 0, out_sec: 0, ...(cfg.value?.intro_outro?.outro || {}) }
}))

/** 素材库下拉的数据源。读失败**不伪装成空库**（否则用户会以为素材丢了）。 */
async function loadVideoLibrary() {
  try {
    const r = await fetchVideoLibrary(pid.value, dir.value, '读取视频素材库')
    videoLibrary.value = r?.videos || []
  } catch (e) {
    videoLibrary.value = []
    ioError.value = '素材库读取失败：' + e.message
  }
}

/** 这一侧最终会播多久（给用户看的人话）。素材时长未知时给 `—`。 */
function introOutroSpan(side) {
  const cur = introOutro.value[side]
  const asset = videoLibrary.value.find((v) => v.name === cur.asset_id)
  if (!asset || asset.duration == null) return '—'
  const inS = Math.max(0, Number(cur.in_sec) || 0)
  // 与 Rust `IntroOutroClip::trim` 同口径：出点 <= 入点 / <= 0 ⇒ 播到素材结尾
  const outS = Number(cur.out_sec) > inS ? Math.min(Number(cur.out_sec), asset.duration) : asset.duration
  return `${(outS - inS).toFixed(1)}s`
}

/** 把一侧写回 `polish.toml`。`next` = 该侧完整对象（缺省那侧给空对象）。 */
async function saveIntroOutro(side, next) {
  if (!ready.value || ioBusy.value) return
  ioBusy.value = true
  ioError.value = ''
  // ⚠️ 只发改的那一侧：patch 是深合并，另一侧原样保留（发整节会把对侧也覆盖成当前快照）
  const payload = { intro_outro: { [side]: next } }
  try {
    await saveProjectPatch(pid.value, dir.value, payload)
    await load({ silent: true })
  } catch (e) {
    ioError.value = '片头/片尾保存失败：' + e.message
    await load({ silent: true })
  } finally {
    ioBusy.value = false
  }
}

/** 勾/取消勾。取消 = **清空素材**（判据只有"有没有素材"，见上面的 ⚠️）。 */
async function onIntroOutroToggle(side, on) {
  const cur = introOutro.value[side]
  if (on) {
    // 开：默认挑库里的第一个可用素材（用户口径是"开启后可选择"，所以开就有个值，
    // 不然勾了以后还是空的、看着像没生效）。库里一个都没有就提示并回退。
    const first = videoLibrary.value.find((v) => v.duration != null)
    if (!first) {
      ioError.value = '素材库里还没有可用的视频 —— 先把文件放进素材库再来开片头/片尾'
      return
    }
    if (cur.asset_id) return
    await saveIntroOutro(side, { asset_id: first.name, in_sec: 0, out_sec: 0 })
  } else {
    if (!cur.asset_id) return
    await saveIntroOutro(side, { asset_id: '', in_sec: 0, out_sec: 0 })
  }
}

async function onIntroOutroAsset(side, name) {
  const cur = introOutro.value[side]
  // 换素材时**保留入出点**（用户调好的 0.5–2.5 换一支片子往往还想用同一段），
  // 越界由 Rust `trim` 夹住，不会出错。
  await saveIntroOutro(side, { asset_id: name, in_sec: cur.in_sec, out_sec: cur.out_sec })
}

async function onIntroOutroTrim(side, field, raw) {
  const cur = introOutro.value[side]
  const n = parseFloat(raw)
  // 非数字/负数一律回 0（= 缺省语义），别把 NaN 写进 toml
  const v = Number.isFinite(n) && n > 0 ? Number(n.toFixed(3)) : 0
  if (v === cur[field]) return
  await saveIntroOutro(side, { ...cur, [field]: v })
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
  loadVideoLibrary()
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
/* 右栏那一列：两块卡片上下常驻（`min-width:0` 让内部的长控件能正常收缩，不被 grid 顶宽） */
.right-col { display:flex; flex-direction:column; gap:12px; min-width:0; }
/* 预览卡内部：属性面板（时间轴 Teleport 过来的）在左，预览舞台在右 */
.preview-row { display:flex; gap:12px; align-items:stretch; margin-top:8px; }
/* 挂载点定宽 + relative；面板绝对定位撑满它 —— 属性表单再长也只在面板内部滚动，
   不会把整行/预览卡撑高（否则时间轴会被顶出首屏） */
.props-host { position:relative; flex:none; width:232px; min-width:0; }
/* 两块面板（时间轴块的属性 / 轨道设置）共用这一列：都绝对定位撑满 ⇒ 谁在都不会把这一行顶高。
   `:deep(.props-panel)` 管的是 TimelineEditor **Teleport 进来**的那份；`.lane-panel` 是本页面
   自己渲染的 —— 它的面板底子（背景/边框/内边距/滚动）必须在这里再写一遍：TimelineEditor 那份
   是 scoped 的（带 data-v 属性），套不到本页面的元素上。 */
/* ⚠️ 2026-10-06 第四轮：**轨道设置面板已整块搬到右栏常驻** —— 它不再住在这个挂载点里、
   也不是覆盖层，所以 `.props-host .lane-panel` 的绝对定位那条**已删**（留着会让它脱离文档流、
   右栏那张卡高度塌成 0）。`:deep(.props-panel)` 只留给 TimelineEditor Teleport 进来的那份。 */
.props-host :deep(.props-panel) { position:absolute; inset:0; width:auto; }
/* 右栏卡片里的轨道设置正文（紧凑间距按 2026-10-06 第二轮口径收过：只留控件） */
.lane-panel label { margin-top:6px; }
.lane-panel .row { margin-top:4px; }
.lane-panel input, .lane-panel select { font-size:12px; }
.lane-panel .ck-row { display:flex; align-items:center; gap:5px; font-size:12px; cursor:pointer; }
/* 分组：每段一条细线 + 标题；点轨道名时那一段的标题高亮（`on`） */
.lane-group { margin-top:7px; padding-top:7px; border-top:1px solid var(--line-soft); }
.lane-group:first-of-type { border-top:none; margin-top:0; padding-top:0; }
/* 片头/片尾：一侧一行（勾选 + 素材下拉 + 入出点）。右栏只有 300px，
   所以入出点那行必须能换行、数字框要定宽（`min-width:0` 让 select 不被顶宽）。 */
.io-row { margin-top:5px; }
.io-head { display:flex; align-items:center; gap:5px; font-size:12px; cursor:pointer; }
.io-row .fld-input { width:100%; min-width:0; margin-top:3px; font-size:12px; }
.io-trim { display:flex; align-items:center; gap:4px; margin-top:3px; flex-wrap:wrap; }
.io-num { width:62px; flex:none; }
.io-dur { margin-left:auto; }
.io-err { color:var(--red); }
/* ⚠️ 第六轮我一度把三组做成 `.lane-cols` 三列并排 —— **是误读**（用户第七轮：「整体还是以行来
   排版」）。并三列时每列只有 83px，两颗框选按钮横排溢出 91>83、字体下拉被截断，全是被那个误解
   逼出来的。**别再引回 `.lane-cols`**：三组上下堆叠成行，每组占满 300px。 */
.lane-group-h { font-size:11px; font-weight:700; color:var(--ink-dim); letter-spacing:.02em; margin-bottom:3px; }
.lane-group.on > .lane-group-h { color:var(--pink-text); }
.lane-group.on > .lane-group-h::before { content:'▸ '; color:var(--main-deep); }
/* 「字幕字体」标签 + 两颗「→默认」同一行（用户口径：放在"字幕字体"旁边）。
   ⚠️ `min-width:0` 每层都要：flex 项默认 `min-width:auto`，中文标签 + 两颗按钮的总宽
   会被当成最小宽度而横向溢出。两颗按钮 `flex:none`（文字别再被压窄到换行）。 */
.ss-font-row { display:flex; align-items:center; gap:6px; min-width:0; margin-bottom:4px; }
.ss-font-row > * { min-width:0; }
/* 字幕样式控件（用户 2026-10 四项 + 描边拆两件）。
   ⚠️ 右栏定宽 300px、卡片 1px 边框 + 16px 内边距 ⇒ **行内容宽只有 266px**（判据 ⑰ 的预算）。
   标签列定宽 62px（"描边宽度"+「近似」角标最宽那行），余下 ~200px 给控件。
   ⚠️ **每一层都要 `min-width:0`**：`.ss-grid` 是 flex 列容器，不写的话它的自动最小尺寸
   会被子项的内容撑开（flex 项默认 `min-width:auto`），整块就往右溢出。 */
/* 右栏卡片的**页签行**（项目设置 / 画面尺寸 / 片头片尾）—— 独占最上面一行、三颗等宽。
   ⚠️ `min-width:0` 每层都要：flex 项默认 `min-width:auto`，中文页签名会把盒子撑到内容宽
   （三颗加起来 > 266px）从而横向溢出。 */
.side-tabs { display:flex; gap:6px; margin-bottom:8px; min-width:0; }
.side-tabs > .mini-btn { flex:1 1 0; min-width:0; justify-content:center; padding:4px 6px; }
.ss-grid { display:flex; flex-direction:column; gap:4px; margin-top:6px; min-width:0; }
/* 读失败 / 未就绪时整块降透明度（配合 :disabled，让"现在不能改"看得出来） */
.ss-grid.off { opacity:.55; }
.ss-row { display:flex; align-items:center; gap:6px; min-width:0; }
/* **两列一行**（用户口径：「改为两行的排版」）。
   ⚠️ 两个坑，都实测过：
   ① 两列各 130px，而标签 `62px` 是**为单列**定的 —— 照搬会让数字框只剩 ~40px（显示不下 "400"）。
      所以列内标签收窄到 **44px**（"描边宽度"+「近似」角标仍能放下），控件拿回 ~80px。
   ② `min-width:0` 必须在**每一层**：`.ss-row` 是 flex 项，flex 的默认 `min-width:auto`
      会被数字框的 `size` 属性撑开，把右列顶出容器（本卡早就为同类问题写过警告）。 */
.ss-row-2 { display:grid; grid-template-columns:1fr 1fr; gap:6px; min-width:0; }
.ss-row-2 > .ss-row { min-width:0; }
/* ⚠️⚠️ 这里**不能只写 `.ss-row-2 .ss-lbl`**：`.ss-lbl` 的基础规则写在文件**后面**
   （见下面那条 `width:62px`），同特异性下**后者胜** ⇒ 62px 生效、两列里标签吃掉一大半，
   数字框被挤成"要么 6ch 要么撑满整行"（实测 `numBoxW:[266,266]` —— 正是用户看到的"排版有问题"）。
   用 `>` 加深一层特异性，压过后面那条基础规则。 */
.ss-row-2 > .ss-row > .ss-lbl { width:44px; }
/* 两列态下的数字框：吃掉标签与单位之后的剩余宽度。
   ⚠️⚠️ **必须显式覆盖 `width`** —— `.rv-input`（同特异性 0,1,0，但写在文件**后面**）
   带着 `flex:none; width:6ch`，源码顺序在后者胜 ⇒ 只写 `flex:1 1 auto` **根本不起作用**，
   框仍被钉在 6ch（≈38px，装不下带 padding 的 `400`）。
   这正是"两列排版有问题"的直接原因：我上一版的几何自测用的是**独立复刻页**，
   那里没有 `.rv-input` 这两条，于是量出假通过（63px）。**用例必须带上真实样式。** */
/* ⚠️⚠️ **必须比 `.rv-input` 更特指**，不能只写 `.ss-num`：
   `.rv-input` 的 `flex:0 0 auto; width:46px`（另一条是 `6ch`）写在文件**后面**，
   同特异性下后者胜 ⇒ `.ss-num` 的 `flex:1 1 auto` 被完全盖掉。
   实测（带 `data-v-*` 的真实渲染）：数字框只有 **32px**、`flexGrow:0`，
   `400` 显示不全 —— 这就是用户说的"两列的排版有问题"。
   加一层 `.ss-row-2 > .ss-row > ` 提高特异性，压过那条基础规则。 */
.ss-row-2 > .ss-row > .ss-num { flex:1 1 auto; width:auto; min-width:0; }
.ss-lbl { flex:none; width:62px; font-size:11px; color:var(--ink-dim); white-space:nowrap;
          display:flex; align-items:center; gap:3px; }
/* 「近似」角标：描边宽度成片侧受底座限制（只有描边开/关），必须如实标注。 */
.ss-tag { flex:none; font-style:normal; font-size:9px; line-height:1; padding:1px 3px; border-radius:3px;
          background:var(--main-soft); color:var(--pink-text); }
/* 取色器：`input[type=color]` 默认 22×22 左右带内边距白边，这里压成方方正正一小块 */
.ss-color { flex:none; width:34px; height:20px; padding:0; border:1px solid var(--line);
            border-radius:var(--r-sm); background:var(--card); cursor:pointer; }
.ss-color:disabled { cursor:not-allowed; }
.ss-color::-webkit-color-swatch-wrapper { padding:1px; }
.ss-color::-webkit-color-swatch { border:none; border-radius:2px; }
/* 位置行的滑块：占满标签后的剩余宽度（与弹幕偏移那行同一个写法） */
.ss-range { min-width:0; }
/* 位置行（滑杆 + 数字框 + 单位）三件挤在 266px 内：滑杆 `flex-1` 吃掉剩余宽度，
   数字框与单位 `flex:none` 保持可用宽度。
   ⚠️ 实测（`.workbuddy/check-subtitle-style-geom.mjs`）：不额外收窄时，这行会把末尾的 `%`
   顶到 **right=283**（内容宽只有 266）—— 溢出 17px、而整行 `scrollWidth` 仍是 266（被 flex 吞了），
   所以**只看 scrollWidth 查不出来**，必须逐个控件量右边界。把数字框在这行单独收窄到 5ch
   （`.rv-input` 默认 6ch）后刚好落回预算内。 */
.ss-row.full > .rv-input { width:5ch; }
.ss-row.full > .ss-range { flex:1 1 auto; }
.stage-col { flex:1; min-width:0; }
/* 「区域与落点」的四颗取坐标按钮（人物 / 弹幕 / 固定 / 改落点）必须同一行：
   右栏是定宽 300px 的 grid 轨道，卡片 1px 边框 + 16px 内边距 → 行内容宽 266px；
   `.mini-btn` 默认 `padding:4px 11px` 时四颗最小宽度合计 282px（11px 图标 + 4px 内间距 +
   汉字 12px/字），必然折成 3+1（第四颗单独占满一行）。本行单独收窄到 4px 8px + gap 4px
   → 合计 252px、余量 14px。四颗仍是行内 `flex:1`，会均分整行，肉眼观感不受影响
   （同 CoverDesigner.vue 的 `.preset-row .mini-btn`）。宽度是按 `getBoundingClientRect`
   在真实 dist 样式下实测的，改这行之前先重测。 */
.region-row { gap: 4px; }
.region-row .mini-btn { padding: 4px 8px; }
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
/* 任务栏（可折叠，用户 2026-10-06 第六轮）：默认只一行（任务名 + 状态 chip）。
   展开时的详情**绝对定位浮在上层** ⇒ 不参与布局、不挤占下方控件（`.card` 是它的定位锚）。 */
.job-wrap { margin-top:8px; }
.job-head { display:flex; align-items:center; gap:6px; width:100%; text-align:left;
  border:1px solid var(--line); border-radius:var(--r-sm); padding:6px 9px;
  background:var(--bg-soft); color:var(--ink); font:inherit; font-size:12px; cursor:pointer; }
.job-head:hover { border-color:var(--main); }
.job-head .ellipsis { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.job-caret { flex:none; opacity:.6; transition:transform .15s; }
.job-head.open .job-caret { transform:rotate(180deg); }
/* 展开的详情 = 浮层：`.job-wrap` 当定位锚（relative），详情 `top:100%` 贴着头往下浮，
   **不占布局** ⇒ 下方控件一个都不动（用户：「展开时不要挤占下方控件，是浮在上层的」）。
   ⚠️ 别把 `position:absolute` 写到 `.job-wrap` 上 —— 那样整条任务栏会逃出卡片、
   跑到页面底部去（实测 top 从 520 变 1930）。 */
.job-wrap { position:relative; }
.job-wrap > .row, .job-wrap > .job-log { position:absolute; top:100%; left:0; right:0; z-index:6;
  border:1px solid var(--line); border-top:none; border-radius:0 0 var(--r-sm) var(--r-sm);
  background:var(--card); padding:7px 9px; margin:0;
  box-shadow:0 6px 16px rgba(0,0,0,.16); }
.job-wrap > .job-log { max-height:120px; overflow:auto; }

/* 成片与封面统一为同尺寸 16:9 盒子（源都是 1920×1080，比例异常时裁切对齐） */
.artifact-bar .cover-thumb,
.artifact-bar .video-bar {
  width: 320px; aspect-ratio: 16/9; object-fit: cover; object-position: center;
  max-height: none; margin-top: 0; flex: none;
  border-radius: var(--r-sm); border: 1px solid var(--line); background: #000; display: block;
}
.cover-img { width:100%; border-radius:var(--r-sm); border:1px solid var(--line); display:block; }
.thumb { width:100%; aspect-ratio:16/9; object-fit:cover; border-radius:var(--r-sm); margin-top:10px; border:1px solid var(--line); }
/* 弹幕偏移的行尾数字框（照 CoverDesigner 的 .rv-input）：常态隐形、hover/focus 现出可编辑 */
.rv-input { flex:none; width:6ch; padding:1px 3px; text-align:right; font-size:10.5px; font-family:monospace;
  background:transparent; border:1px solid transparent; border-radius:var(--r-sm); color:var(--ink); }
.rv-input:hover { border-color:var(--line); background:var(--card); }
.rv-input:focus { border-color:var(--main); box-shadow:0 0 0 3px var(--main-soft); }
.rv-input:invalid { box-shadow:none; }
.rv-input:invalid:focus { box-shadow:0 0 0 3px var(--main-soft); }
.rv-input::-webkit-outer-spin-button, .rv-input::-webkit-inner-spin-button { -webkit-appearance:none; margin:0; }
.rv-input { -moz-appearance:textfield; appearance:textfield; }
.rv-input:disabled { opacity:.5; cursor:not-allowed; }
</style>
