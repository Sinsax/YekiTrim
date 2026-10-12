<template>
  <div ref="rootRef" class="timeline" tabindex="0">
    <!-- 头部一行：调用方给的标题/提示（slot）+ 去静音 + 重新后处理 + 保存状态。
         这样页面不用再单独放一个卡片标题行 —— 时间轴本来就占竖高（用户要求精简）。 -->
    <div class="tl-head">
      <div class="tl-head-title"><slot name="head" /></div>
      <div class="row">
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
             删除是"双击即生效 + 250ms 落盘 + 随后自动后处理"，误删点完就已经不可逆了。 -->
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
      <button class="mini-btn ml-auto" :disabled="loadRetrying" @click="retryLoad">
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
    <!-- 元素轨道的 ＋：先选「文字 / 贴纸」再加（明细见 `addElementAt` 的注释） -->
    <div v-if="elementMenu" class="fx-menu" :style="{ left: elementMenu.x + 'px', top: elementMenu.y + 'px' }">
      <div class="fx-menu-t muted small">在播放头处添加</div>
      <button class="fx-menu-item" title="大字标题 / 章节 / 注释，由后处理排版落点"
              @click="addElementAt('text')">
        文字<span class="muted">kind=text</span>
      </button>
      <button class="fx-menu-item" title="按关键词从贴纸库取图"
              @click="addElementAt('sticker')">
        贴纸<span class="muted">kind=sticker</span>
      </button>
    </div>
    <!-- 「加入成片」菜单（§6.3 第 ② 条的三个落位）。fixed 定位：不受时间轴横向滚动与卡片裁切影响。
         入口只有一个（成片轨标签上的 ＋）—— 用户在源轴上框选/选中之后，决定这段插到成片的哪里。 -->
    <div v-if="edlMenu" class="fx-menu edl-menu" :style="{ left: edlMenu.x + 'px', top: edlMenu.y + 'px' }">
      <div class="fx-menu-t muted small">把选中的源区间加到成片</div>
      <button class="fx-menu-item" title="作为成片的第一段（冷开场）" @click="addEdl('head')">
        插到片头<span class="muted">冷开场</span>
      </button>
      <button class="fx-menu-item" title="插到成片轨当前播放头所在的那一段之后" @click="addEdl('at')">
        插到播放头<span class="muted">当前位置</span>
      </button>
      <button class="fx-menu-item" title="追加到成片末尾" @click="addEdl('tail')">
        追加到片尾<span class="muted">末尾</span>
      </button>
    </div>
    <!-- 属性面板：Teleport 到页面给的挂载点。⚠️ 必须 `v-if="teleportTo"` 门控 ——
         目标在**挂载时**解析，那时子树还没进 document，直接 `:to` 会整块不渲染。 -->
    <Teleport v-if="teleportTo" :to="teleportTo">
      <!-- ⚠️ 轨道设置在页面的那一份面板里（PolishView 与它共用 `.props-host` 这一列）：
           轨道设置打开时这里整块让位，两份面板不并存（用户口径：所有轨道合并到一个属性面板）。 -->
      <div v-if="!laneOpen" class="props-panel card" @input="onPanelEdit" @change="onPanelCommit">
        <div class="row spread">
          <b>{{ panelTitle }}</b>
          <button v-if="selected || edlSelected >= 0" class="mini-btn" @click="selected = null; edlSelected = -1">关闭</button>
        </div>
        <!-- 成片段的面板（§6.3 第 ③⑤ 条）。它与源轴块的面板**互斥**共存（选成片段就清 selected）。 -->
        <template v-if="edlSelBlock">
          <div class="row spread mt-4">
            <span class="chip p">成片第 {{ edlSelBlock.index + 1 }} 段</span>
            <span class="muted small">共 {{ edlTotalSecs.toFixed(2) }}s</span>
          </div>
          <div class="edl-sum">
            成片 {{ edlSelBlock.outStartSecs.toFixed(2) }}s → {{ edlSelBlock.outEndSecs.toFixed(2) }}s
            <span class="muted">（{{ (edlSelBlock.outEndSecs - edlSelBlock.outStartSecs).toFixed(2) }}s）</span>
          </div>
          <!-- 源区间 = 这一段播源素材的哪一块。⚠️ 草稿态：改完按「应用」才写进 clips
               （与「固定落点」那条口径一致：拖控件时不该把半成品落盘）。 -->
          <div class="row">
            <div class="flex-1"><label>源起（秒）</label>
              <input type="number" step="0.01" min="0" :value="edlSpanStart" @input="startEdlSpanEdit(); edlDraftSpan.start = $event.target.value"></div>
            <div class="flex-1"><label>源止（秒）</label>
              <input type="number" step="0.01" min="0" :value="edlSpanEnd" @input="startEdlSpanEdit(); edlDraftSpan.end = $event.target.value"></div>
          </div>
          <div class="row mt-6 gap-4">
            <button class="mini-btn" :disabled="!edlDraftSpan" @click="applyEdlSpan">应用区间</button>
            <button v-if="edlDraftSpan" class="mini-btn" @click="cancelEdlSpanEdit">取消</button>
            <button class="mini-btn" title="预览跳到这一段在源时间轴上的起点" @click="edlSeekSource">定位</button>
          </div>
          <label>速率</label>
          <div class="row edl-speeds">
            <button v-for="sp in EDL_SPEEDS" :key="sp.value" class="mini-btn"
                    :class="{ selected: Math.abs(edlSelBlock.speed - sp.value) < 0.001 }"
                    :disabled="!sp.enabled" :title="sp.title"
                    @click="edlSetSpeed(edlSelBlock.index, sp.value)">{{ sp.label }}</button>
          </div>
          <!-- ⚠️ 变速**第 4 批（task-7）已接通**：画面缩段长（转译器，与 `resolve_clips` 逐值一致）、
               音轨逐段挂 `atempo`（时域伸缩，**基频不变**）。所以这里不再有置灰档 ——
               留一条说明是因为"变速到底改了什么"在界面上看不出来（块只是变宽/变窄）。 -->
          <p class="mini-hint" title="画面按 speed 缩段长、音轨按 atempo 时域伸缩（音调不变）；源素材那一段的起点不受影响">
            变速同时改画面与音轨：档速 > 1 块变窄、< 1 变宽；音调不变
          </p>
          <label>名字（可选）</label>
          <input :value="edlSelBlock.label" placeholder="例如：爆点重放"
                 @input="edlSetLabel(edlSelBlock.index, $event.target.value)">
          <!-- 转场（第 7 批 / task-10）：叠化 = 与**前一段**交叠淡化（画面跨轨重叠 +
               音轨交叉淡化）。⚠️ 成片的第一段没有"前一段"可叠，所以那一段不给按钮。 -->
          <label>入场转场</label>
          <div class="row edl-speeds">
            <button class="mini-btn"
                    :class="{ selected: !edlSelBlock.transition }"
                    :disabled="edlSelBlock.index === 0"
                    :title="edlSelBlock.index === 0
                      ? '成片第一段没有上一段可叠，只能硬切'
                      : '直接切到本段（默认）'"
                    @click="edlSetTransition(edlSelBlock.index, EDL_TRANSITION_CUT)">硬切</button>
            <button class="mini-btn"
                    :class="{ selected: edlSelBlock.transition === 'dissolve' }"
                    :disabled="edlSelBlock.index === 0"
                    :title="edlSelBlock.index === 0
                      ? '成片第一段没有上一段可叠，只能硬切'
                      : '与上一段重叠 0.5s 淡化：画面跨轨叠化、原声交叉淡化'"
                    @click="edlSetTransition(edlSelBlock.index, EDL_TRANSITION_DISSOLVE)">叠化</button>
          </div>
          <p class="mini-hint" title="叠化 = 画面把本段挪到上层、用 0.5s 不透明度淡入盖住上一段；原声在同一窗口里交叉淡化">
            叠化：与上一段重叠 0.5s 淡化；过于短的段会退回硬切
          </p>
          <div class="row spread mt-6 small">
            <span class="muted">来源</span>
            <span class="chip" :class="edlSelBlock.sourceKind === 'asset' ? 'warn' : 'n'">{{ edlSelBlock.sourceLabel }}</span>
          </div>
          <p class="mini-hint" title="块与块首尾相接：源时间上不连续的地方，在成片里就是一次跳切">块首尾相接 = 跳切</p>
          <div class="row mt-8">
            <button class="mini-btn" @click="edlDuplicate(edlSelBlock.index)">重放一次</button>
            <button class="mini-btn danger" @click="edlDelete(edlSelBlock.index)">删除</button>
            <span class="muted small ml-auto">{{ saveText }}</span>
          </div>
        </template>
        <div v-else-if="!selected" class="props-empty muted small">点时间轴上的块，在这里改属性</div>
        <template v-else-if="selItem">
          <!-- ── 标配：开始 / 结束 / 入点 / 出点 / 定位（所有类型都有） ── -->
          <div class="row">
            <div class="flex-1"><label>开始（秒）</label>
              <input v-model.number="selStart" type="number" step="0.01" min="0"></div>
            <div class="flex-1"><label>结束（秒）</label>
              <input v-model.number="selEnd" type="number" step="0.01" min="0"></div>
          </div>
          <div class="row mt-6 gap-4">
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
              ? '后处理不改写它'
              : '后处理会吸附到最近字幕' }}</span>
            <button v-if="selItem.manual" class="mini-btn" title="交回自动处理：下一次后处理会把它吸附到最近字幕（数量超出额度时也可能被抽稀）" @click="followSubtitles">跟随字幕</button>
          </div>

          <!-- ── 类型专属 ── -->
          <template v-if="selected.kind === 'seg'">
            <label>备注</label>
            <input v-model="selItem.note" placeholder="分段备注">
            <!-- §8 叙事角色：**只读展示**。
                 以前这个字段在界面上**完全看不见** —— 而它并不只是个标签：
                 ① LLM 按它选配方（`hook` = 钩子、`punch` = 爆点/名场面，见 llm/segment.rs 的 match）；
                 ② `process` 的 Step 1g 守卫按它纠偏（`enforce_narrative_shape`：hook 至多一段、
                    punch 至少一段）。
                 ⇒ 用户看得到"这一段被当成什么角色"，才解释得通"为什么这段特效多、那段几乎没动"。
                 ⚠️ **不做成可编辑下拉**：手工改 role 会让它和 LLM 的配方、守卫的假设脱节，
                    而这两者都不是即时重算的 —— 那会造出一个"改了没反应"的控件。
                    要改就重跑 LLM（那本来就是 role 的生产者）。 -->
            <label title="叙事角色：由 LLM 判定，决定这一段的特效配方与守卫规则。要改请重跑 LLM 剪辑">叙事角色</label>
            <div class="role-readonly" :title="roleHint">{{ roleLabel }}</div>
            <!-- ⚠️ 这个标签以前写作「情绪」——**名字是错的**，会让人以为它背后是 §10 的情绪轨道。
                 它绑的是 `fx`（**演出密度**）：`low/med/high` 决定这一段塞多少特效/音效。
                 真正的情绪轨道是 `process` 算出来的 `data/emotion.json`（观众情绪），
                 与这里**没有关系**，而且它**目前还没有界面**。
                 ⇒ 标签照实写「演出密度」；混用「情绪」会让两件事互相冒名。 -->
            <label>演出密度</label>
            <select v-model="selItem.fx">
              <option value="low">low · 安静叙述</option>
              <option value="med">med · 日常</option>
              <option value="high">high · 爆点</option>
            </select>

            <!-- 最近一次 LLM 生成：这一段成/败 + 只重做这一段。
                 整片重做很贵（一次 LLM 调用一段），而且限流时总会有一两段落空；
                 以前失败**完全不可见**，用户只看到"这段 20 秒没事件"。 -->
            <div class="seg-gen mt-6">
              <div class="row spread small">
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
              <button class="mini-btn mt-6 w-full jc-center"
                      :disabled="reprocessBusy"
                      title="只重新生成这一段的事件（其余分段、筛选词/高亮词都不动）；失败会保留该段原事件"
                      @click="emit('redo-segment', selected.si)">
                <Icon name="sparkles" :size="11" /> 只重做本段事件
              </button>
            </div>

            <!-- 系统理由（§1）：后处理在这段上**自动**改过什么、凭什么。
                 只读 —— 它描述"已经发生的事"，改它没有效果（要改就改上面的起止/景别）。 -->
            <div v-if="segDecisions.length" class="seg-why mt-6">
              <div class="row spread small">
                <span class="muted">系统做得对吗</span>
                <span class="chip n">{{ segDecisions.length }} 条自动决策</span>
              </div>
              <div v-for="(d, i) in segDecisions" :key="i" class="why-row" :title="d.detail || ''">
                <span class="why-dot" :class="d.kind === 'boundary_sentence_end' ? 'b' : 'o'"></span>
                <span class="why-text">{{ d.text }}</span>
                <span v-if="d.detail" class="muted small mono why-detail">{{ d.detail }}</span>
              </div>
              <div class="mini-hint">
                后处理每次重跑都会重算这些理由；要改结果请直接拖动上面的分段边界。
              </div>
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
              <!-- 运镜速度（§16）：只在**平滑**下有意义 —— 硬切本来就是 0 秒，
                   同时写以 cut 为准（转译器与模板都这么判）。所以硬切时置灰、并写明原因，
                   而不是让它看着能点、点了没反应。 -->
              <label :title="selItem.cut ? '硬切没有过渡，速度不起作用' : '推近/拉远的快慢'">推进速度</label>
              <select
                :value="selItem.speed || 'med'"
                :disabled="selItem.cut === true"
                @change="setCameraSpeed(selItem, $event.target.value)"
              >
                <option value="med">默认（跟镜头时长）</option>
                <option value="snap">极快（0.1s，像怼上去）</option>
                <option value="fast">较快（0.25s）</option>
                <option value="slow">缓慢（1.5s）</option>
              </select>
              <label>景别</label>
              <select :value="selItem.zoom" @change="setZoom(selItem, $event.target.value)">
                <option value="远">远（画中画）</option>
                <option value="中">中景</option>
                <option value="近">近景</option>
                <option value="特写">特写（怼脸）</option>
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
            </template>

            <!-- 贴纸：keyword 一级 → 贴纸二级 -->
            <!-- 统一视觉元素：kind 分派 text / sticker -->
            <template v-else-if="selItem.type === 'element'">
              <label>元素种类</label>
              <select :value="selItem.kind || 'text'" @change="setElementKind(selItem, $event.target.value)">
                <option value="text">文字（大字标题 / 章节 / 注释）</option>
                <option value="sticker">贴纸（按关键词取图）</option>
              </select>

              <!-- 文字支：内容 + 样式。**和贴纸一样只有"是什么"，没有"摆哪"**——
                   ⚠️ 别再退回"纵向位置"下拉：位置一律由后处理按安全落点算
                   （与贴纸那句「位置 x/y 由后处理算」同一条口径），
                   下同的那句 hint 就是给用户看的唯一说明。 -->
              <template v-if="(selItem.kind || 'text') === 'text'">
                <label>文字内容</label>
                <input :value="selItem.text || ''" placeholder="要显示的字"
                       @input="selItem.text = $event.target.value">
                <label>样式</label>
                <select :value="selItem.style || 'comment'" @change="selItem.style = $event.target.value">
                  <option v-for="s in TEXT_STYLES" :key="s.value" :value="s.value">{{ s.label }}</option>
                </select>
                <!-- 动效（39 档，2026-10-13）。
                     ⚠️ 空串 = 不动效 ⇒ **不写 `anim` 键**（老工程逐字节不变）。
                     ⚠️ **认不出的档名必须仍然显示出来**（下面那条 `v-if`）：
                        它若被 `v-model` 悄悄归成空串，用户改一下别的字段就把那个
                        错档名**永久抹掉**了（保存时写的是下拉的值）—— 而报告里的
                        CLAMP 已经告诉过他"这一条不会动"，得让他看得见是哪一条。 -->
                <label>动效</label>
                <select :value="selItem.anim || ''" @change="selItem.anim = $event.target.value || null">
                  <optgroup v-for="g in ANIM_GROUPS" :key="g.group" :label="g.group">
                    <option v-for="a in g.items" :key="a.value" :value="a.value">{{ a.label }}</option>
                  </optgroup>
                  <option v-if="selItem.anim && !ANIM_KNOWN.includes(selItem.anim)"
                          :value="selItem.anim">未知档（{{ selItem.anim }}）</option>
                </select>
                <div v-if="selItem.anim && !ANIM_KNOWN.includes(selItem.anim)" class="hint">
                  这个动效档名不认识，成片里不会动。
                </div>
                <div class="hint">位置 x/y 由后处理算（每次重跑覆盖）。</div>
              </template>

              <!-- 贴纸支：复用已有的贴纸选择器 -->
              <template v-else>
                <StickerPicker :lib="stickerLib" :pid="pid" :dir="dir"
                               :pack="selItem.text || ''" :tag="selItem.sticker_tag || ''"
                               :name="selItem.name || ''" :source="selItem.source || ''"
                               @pick="onStickerPick" />
                <div class="hint">位置 x/y 由后处理算（每次重跑覆盖）。</div>
              </template>
            </template>

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
                <p v-if="!(selItem.colors && selItem.colors.length >= 2)" class="mini-hint" title="按「该色 → 透明」渲染；改动任一色档即成为两档渐变">
                  未设色档，按单色渐变渲染
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
                <label>强度（{{ selItem.intensity ?? 14 }}）</label>
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
            <span class="muted small ml-auto">{{ saveText }}</span>
          </div>
        </template>
      </div>
    </Teleport>

    <!-- 没有分段时**整条时间轴照常渲染**，这里只给一句提示（不再放「新建分段」按钮：
         用户指出那是为「删空分段后整块消失」那个 bug 加的临时入口，bug 已修，位置让给去静音）。
         以前是 `v-if` 把整块换成"暂无时间轴数据，请先运行 LLM 自动生成分段" —— 删掉最后一段后
         时间轴直接消失，连字幕/屏蔽都没法改（用户报过）。 -->
    <div v-if="!segments.length" class="tl-no-seg">
      <span class="muted small">还没有分段，先跑一次「LLM 剪辑」</span>
    </div>
    <div ref="bodyRef" class="timeline-body" @wheel.passive="markUserScroll" @pointerdown="markUserScroll">
      <div class="timeline-labels">
        <div v-for="lane in lanes" :key="'label-' + lane.type" class="lane-label">
          <!-- 眼睛 = 这一类渲不渲染（**事件一个字都不动**，点亮即恢复）。判据由 Rust 注入
               （`_lanes`，见 PolishView 的 laneEnabled）：竖屏素材/画布下运镜与视觉特效默认灭。
               ⚠️ 「分段 / 屏蔽」是结构轨（分段是容器、屏蔽是剪切标记），没有"整类不渲染"这种语义，
               所以它没有眼睛 —— 别给它补一个假装有作用的按钮。 -->
          <button v-if="lane.lane" class="lane-eye" :class="{ off: !laneEnabled(lane.lane) }"
                  :title="laneEyeTitle(lane)" @click.stop="emit('toggle-lane', lane.lane)">
            <Icon :name="laneEnabled(lane.lane) ? 'eye' : 'eye-off'" :size="12" />
          </button>
          <span v-else class="lane-eye lane-eye-none"
                title="分段 / 屏蔽是结构轨：分段是容器、屏蔽是剪切标记，没有「整类不渲染」这种语义">·</span>
          <!-- 轨道名可点 = **这一条轨道的设置**（与时间轴上的块共用同一个属性面板） -->
          <button class="lane-name" :class="{ on: laneOpen === lane.type }"
                  :title="lane.label + '：点这里改这条轨道的设置'"
                  @click.stop="emit('lane-open', lane.type)">{{ lane.label }}</button>
          <div class="lane-adds">
            <button v-if="lane.type !== 'segments'" class="add-lane"
                    :title="lane.type === 'fx' ? '在播放头处添加视觉特效（先选细分类型）' : '在播放头处添加'"
                    @click.stop="onLaneAdd(lane.type, $event)"><Icon name="plus" :size="11" /></button>
            <button v-else class="add-lane mute-add" title="在播放头处添加屏蔽区间" @click="addMuteAtCurrent"><Icon name="scissors" :size="11" /></button>
          </div>
        </div>
        <!-- 成片轨的标签列（**只有开了 EDL 才出现**；老工程这里什么都没有 ⇒ 界面零变化）。
             ⚠️ 它是**第二条时间轴**，不是第 8 条源泳道：上面 7 条的 x 坐标是源时间、
             永不被 EDL 影响（§6.1）。所以这一格用 `.edl-label` 单独的类名（多一点视觉分隔），
             也刻意**不给眼睛** —— 它没有"整类不渲染"这种语义，EDL 空着就是这条路没开。 -->
        <div v-if="edlOn" class="lane-label edl-label">
          <span class="lane-eye lane-eye-none" title="成片时间线：源时间 → 成片时间的映射（不是第 8 条源泳道）">▸</span>
          <span class="lane-name edl-label-name" title="成片时间线（EDL）：块与块首尾相接，宽度 = 源区间 ÷ 速率，所以变速看得见">成片</span>
          <div class="lane-adds">
            <button class="add-lane" title="把源时间轴上选中的一段加进成片（插片头 / 插播放头 / 追加片尾）"
                    @click.stop="openEdlMenu($event)"><Icon name="plus" :size="11" /></button>
          </div>
        </div>
        <div class="lane-label ruler-label">时间</div>
    </div>
    <div ref="tracksRef" class="timeline-tracks">
      <div v-for="lane in lanes" :key="'track-' + lane.type" class="lane">
        <div class="lane-track" :style="{ width: trackWidth + 'px' }">
          <svg v-if="lane.type === 'subtitles' && wavePath" class="wave-layer" :width="trackWidth" :height="WAVE_H">
            <!-- 双层描边：这条线要同时压在不透明的字幕块（`#3b82f6`）**和**近白背景
                 （`--bg` #F7F3F0）上，单色必然在一边糊掉 —— 深色 halo 负责近白底上的边，
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
            <!-- §10 情绪徽标：并进分段块（不单开泳道，见 `segEmotion`） -->
            <span v-if="block.emo" class="emo-badge" :style="{ background: block.emo.color }"
                  :title="block.emo.title">{{ block.emo.label }}</span>
            {{ block.label }}
          </div>
        </div>
      </div>
      <!-- ── 成片时间轴（EDL）：**第二条轴**，与上面 7 条源泳道并列（§6.2 双轨视图）──
           ⚠️ 三条铁律：
           ① 它的横向位置走 **`edlTrackWidth`（成片时长）**，不是 `trackWidth`（源时长）——
              成片通常比源**短**（跳切/屏蔽）或**长**（重放/慢放），共用源宽会把块画到格子外面；
           ② 块的左右位置走 **`outStartSecs`**（成片时间），**不是 `source_start`**（源时间）。
              后者看着更省事，但那样"重放段"会与正片叠在同一处、慢放段宽度也不会变 —— 正是
              §6.2 要消灭的"看不见映射"；
           ③ 上面 7 条的 x 坐标**一个像素都不因 EDL 改变**（`laneBlocks` 只读源时间，见 §6.1）。
           默认隐藏：`edlOn` 为假时整块不渲染（老工程界面逐字节不变）。 -->
      <div v-if="edlOn" class="lane edl-lane">
        <div class="lane-track edl-track" :style="{ width: edlTrackWidth + 'px' }">
          <!-- 播放头：按**映射后的成片时间**画（两轨共享同一个播放头，但成片轨显示映射后的位置，
               §6.2 第 3 条）。落在被删掉的源区间里时 `edlHeadLeft` 是 null ⇒ 不画
               （那一段在成片里不存在，画在 0 或末尾都是在撒谎）。 -->
          <div v-if="edlHeadLeft !== null" class="playhead" :style="{ left: edlHeadLeft + 'px' }"></div>
          <div
            v-for="b in edlTrackBlocks"
            :key="'edl-' + b.index"
            class="block edl-block"
            :class="{ selected: edlSelected === b.index, 'edl-bad': b.invalid, 'edl-asset': b.sourceKind === 'asset' }"
            :style="{ left: b.outLeft + 'px', width: b.outWidth + 'px' }"
            :title="edlBlockTitle(b)"
            @pointerdown.stop="edlSelected = b.index; selected = null"
            @dblclick.stop="edlDelete(b.index)"
          >
            <span class="resize resize-l" title="拖这里改这一段的源起点" @pointerdown.stop="startEdlDrag($event, b, 'resize-l')"></span>
            <!-- §6.5：来源角标 —— 素材库来源与"本条片段"必须一眼分得开。
                 数据模型（第 6 批）加上 `source_kind`/`asset_id` 后这里自动跟上，
                 判据只在 `polish/edl.js::edlSourceKind` 一处。 -->
            <span class="edl-badge" :class="b.sourceKind === 'asset' ? 'asset' : 'clip'">{{ b.sourceLabel }}</span>
            <!-- §6.5+第 7 批：转场角标 —— **叠化是"段与段之间"的事**，扫成片轨时要能一眼
                 看出哪几段之间有叠化（光靠面板里选中态看不出来）。
                 判据只在 `polish/edl.js::edlTransition` 一处，且**第一段不显示**
                 （没有上一段可叠，出片时会退化成硬切）。 -->
            <span v-if="b.transition === 'dissolve'" class="edl-xf" title="入场叠化：与上一段重叠 0.5s 淡化">≈</span>
            <span class="edl-lab">{{ b.label || EDL_KINDS[b.kind] || ('第 ' + (b.index + 1) + ' 段') }}</span>
            <span class="edl-sp">{{ b.speed === 1 ? '×1.0' : '×' + b.speed }}</span>
            <button class="edl-rep" title="把这一段再放一次（复制并排在其后；连点几次就是重放几次）"
                    @pointerdown.stop @click.stop="edlDuplicate(b.index)">重放</button>
            <span class="resize" title="拖这里改这一段的源终点" @pointerdown.stop="startEdlDrag($event, b, 'resize-r')"></span>
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
import { TIMELINE_COLORS, isEditableKeyEvent } from '../utils'
import {
  fetchProject, fetchSubtitles, fetchEmotion, fetchSfxCatalog, fetchStickerLibrary,
  saveProjectPatch, saveSubtitles, runSilence,
} from '../api/polish'
// EDL 的**共享只读派生** + 编辑动作（算式只有一处：`engine.js::edlOutStart`，
// 判据 `node scripts/check-edl-invariants.mjs`）。本组件只消费，不重算。
import {
  EDL_SPEEDS, EDL_KINDS, MIN_CLIP_SECS, EDL_SPEED_MIN, EDL_SPEED_MAX, edlBlocks, edlOutSecs, edlVisible,
  EDL_TRANSITION_CUT, EDL_TRANSITION_DISSOLVE, setClipTransition,
  addRangeToEdl, duplicateClipAt, removeClipAt,
  setClipSpan, setClipSpeed, setClipLabel,
  clipIndexAtOut, clipIndexAtSource, edlOutSecsAtSource,
} from '../polish/edl'

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
  propsTarget: { type: String, default: '#tl-props-host' },
  /** 当前打开的是哪条轨道的设置（空串 = 没打开）；由页面持有（它才是设置数据的家） */
  laneOpen: { type: String, default: '' },
  /** 六条轨道的开关（Rust 注入的 `_lanes`，已解析）：眼睛的亮/灭只读它，别在前端再推一遍 */
  lanesOn: { type: Object, default: () => ({}) },
  /** 源素材**自己的**帧率（帧号域累加的分母，见 `polish/edl.js`）。
   *  ⚠️ 与"工程网格"（`timeline.timebase`，有 30fps 下限）**不是一回事**：电台那种 5fps 素材
   *  的网格会被抬到 30fps，用网格帧率去比 `edlOutStart` 就是 6 倍错位。
   *  拿不到就按 30（与 `PolishPreview` 的回落同一个值），**别自己再写一份推导**。 */
  srcFps: { type: Number, default: 30 }
})
// 三个信号，**含义各不相同，别互相替代**：
//   `preview` = 本地改动**立刻**镜像给页面（预览实时刷新，**不落盘**、不重挂载）
//   `saved`   = **这批改动已经落盘**（磁盘上那份是最新的了）—— Dhampir 那条腿据此重载
//   `done`    = **一次编辑真的做完了**（面板控件确定 / 取消选中 / 切片段 / 离开页面），
//                页面据此跑一次后处理；它保证**已在 `saved` 之后**（见 `commitEdit`）。
// ⚠️ `saved` 与 `done` **不是**"落盘成功就算完成"：落盘了你还可能在改这个事件，
//    所以后处理仍要等 `done`。两个信号各自服务一个消费者。
const emit = defineEmits(['seek', 'toggle', 'preview', 'saved', 'done', 'reprocess', 'retried', 'redo-segment', 'lane-open', 'toggle-lane'])
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
/**
 * 系统理由（§1）：后处理在**整片**上自动做过的决策，来自 `polish.toml` 的 `[[decisions]]`。
 *
 * ⚠️ 它是**只读回执**，不是配置 —— 改它没有任何效果（后端每趟 process 重算覆盖）。
 * 面板里那条提示（"要改结果请拖动分段边界"）就是防用户以为这里能改。
 */
const decisions = ref([])
const subtitles = ref([])
/** §10 情绪轨道的**原始**响应（`{window_secs, windows}`）；`null` = 没有这个产物。
 *  ⚠️ 存原始值而不是算好的块：`trackWidth` 会随窗口宽度变化，块的像素位置必须在
 *     computed 里重算（与 `laneBlocks` 同一条理由）。 */
const emotionRaw = ref(null)
const muteRanges = ref([])
/**
 * **成片时间线（EDL）的 `clips[]`** —— 只存这一份（`polish.toml` 的 `[[edl.clips]]` 镜像）。
 * ⚠️ `out_start` **永不落盘、永不进这个数组**（§2.2 规则 1：落盘就有两个真源）——
 * 它由 `edlBlocks` 现算（唯一的算式在 `engine.js::edlOutStart`）。
 */
const edlClips = ref([])
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

// ── §8 叙事角色（只读展示）──────────────────────────────────────────────────
// ⚠️ 取值与文案必须与 `llm/segment.rs` 的 `match segment.role()` **对得上** ——
//    那边的两档配方就是这里的 tooltip 正文。**加档位要两边一起加**
//    （`hook`/`punch` 之外的串是"其余"：提示词对它们**一个字都不说**，
//     所以这里也**不编造**公式，只如实说"按默认配方"）。
const ROLE_TEXT = {
  hook: {
    label: 'hook · 钩子',
    hint: '片头最抓人的一段：允许大字元素 + 强音效 + 硬切运镜，但整段只放 1~2 个亮点。'
      + '守卫：全片至多一段 hook（多余会被降级）。',
  },
  punch: {
    label: 'punch · 爆点 / 名场面',
    hint: '全片最值得剪出来的地方：在预算内上足近景 / 画中画运镜 + shake + flash + 音效 + 贴纸。'
      + '守卫：全片至少一段 punch（没有就从最长的 fx=high 段提拔）。',
  },
}
const roleLabel = computed(() => {
  const r = String(selItem.value?.role ?? '').trim()
  if (!r) return '未标注（按默认配方）'
  return ROLE_TEXT[r]?.label || `${r}（认不出，按默认配方）`
})
const roleHint = computed(() => {
  const r = String(selItem.value?.role ?? '').trim()
  return ROLE_TEXT[r]?.hint || '这一段没有角色标注：提示词对"其余段"一个字都不说，沿用默认配方（几乎不放特效，守住静）。'
})

const panelTitle = computed(() => {
  if (edlSelected.value >= 0) return '成片段属性'
  if (!selected.value) return '属性'
  return { seg: '分段属性', event: '事件属性', sub: '字幕属性', mute: '屏蔽属性' }[selected.value.kind] || '属性'
})

// **效果词汇表派生**（2026-10-01）：这四张表与两个分组原先手写在这里，
// 而同一份词汇表散在 15 处代码里（见 spec/effects.json 头注释与 docs/plan/effect-pipeline-unification.md）。
// 现在**只读那一份权威**：加/删效果改 `spec/effects.json`，这里自动跟上；
// `scripts/check-effect-registry.mjs` 仍会检查"读没读"与"锚点在不在"。
import EFFECT_SPEC from '../../../spec/effects.json'

const EFFECTS = EFFECT_SPEC.effects
/** 视觉特效泳道的**细分类型**（顺序 = 面板下拉与 ＋ 菜单的顺序 = 词汇表里的顺序）。
 *  ⚠️ 按 `lane` 而不是 `kind` 派生：D3 之后「覆盖层」并进了这条泳道，而 `overlay` 的
 *  `kind` 仍是 `durable`（那是**语义**，用于时长上限与 LLM 提示词分组）。两者别混。 */
const FX_TYPES = EFFECTS.filter((e) => (e.lane ?? 'fx') === 'fx').map((e) => e.id)
/** 类型中文名（只用于展示；可选性由轨道决定，见 `isFxType`）。 */
const TYPE_LABELS = Object.fromEntries(EFFECTS.map((e) => [e.id, e.label]))

/** §5 E4a 文字元素的四档样式。**必须与出片侧 `TEXT_STYLE_FALLBACK` 同集合**
 *  （`tools/polish-to-dhampir.mjs`：hook / chapter / comment / count）——
 *  这里是**选项**，那里是**权威**；漏一档的症状是"界面里选不到那个样式"。
 *  ⚠️ 数值（字号/描边/颜色）**不在这里抄一份**：那是出片侧的活，界面上抄了就会漂。 */
const TEXT_STYLES = [
  { value: 'hook', label: '钩子（最大字）' },
  { value: 'chapter', label: '章节' },
  { value: 'comment', label: '注释（默认）' },
  { value: 'count', label: '计数' }
]

/** **39 档动效**（`docs/plan/text-styles-39-完成计划.md` §一·五，2026-10-13）。
 *
 *  ⚠️ **档名的权威在 `tools/polish-to-dhampir.mjs::ANIM_PRESETS`**（底座字段映射在那）。
 *  两处**必须同集合** —— 界面能选、转译器不认识 ⇒ 用户选了却**什么都不发生**，
 *  而报告里只有一行 CLAMP。判据：`scripts/check-anim-presets-parity.mjs`（逐档对拍 + 变异自证）。
 *
 *  ⚠️ 空串 = **不动效**（不写 `anim` 键）。必须排第一 —— 「不选」是最常见的状态
 *  （老工程与后处理生成的事件都没有这个键）。
 *
 *  ⚠️ 39 档平铺成一个下拉会没法用 ⇒ 分组成 `<optgroup>`。 */
const ANIM_GROUPS = [
  { group: '不动', items: [{ value: '', label: '（不动效）' }] },
  {
    group: '倾斜',
    items: [
      { value: 'tilt_std', label: '倾斜 · 标准' },
      { value: 'tilt_sm', label: '倾斜 · 小角度' },
      { value: 'tilt_rev', label: '倾斜 · 反方向' }
    ]
  },
  {
    group: '抖动',
    items: [
      { value: 'shake_std', label: '抖动 · 标准' },
      { value: 'shake_slow', label: '抖动 · 慢频' },
      { value: 'shake_lg', label: '抖动 · 强' }
    ]
  },
  {
    group: '摇摆',
    items: [
      { value: 'sway_std', label: '摇摆 · 标准' },
      { value: 'sway_lg', label: '摇摆 · 大角度' },
      { value: 'sway_slow', label: '摇摆 · 慢摆' }
    ]
  },
  {
    group: '砸入',
    items: [
      { value: 'slam_std', label: '砸入 · 标准' },
      { value: 'slam_lg', label: '砸入 · 重' },
      { value: 'slam_squash', label: '砸入 · 带压扁' }
    ]
  },
  {
    group: '收缩',
    items: [
      { value: 'shrink_in_std', label: '收缩 · 标准' },
      { value: 'fade_only', label: '收缩 · 只淡出' },
      { value: 'shrink_in_fast', label: '收缩 · 快速抽走' }
    ]
  },
  {
    group: '弹出',
    items: [
      { value: 'pop_std', label: '弹出 · 标准' },
      { value: 'pop_soft', label: '弹出 · 软' }
    ]
  },
  // ===== C 组：逐字（14 档）=====
  //
  // ⚠️ 这一组的档位在**底座**侧走的是逐字路径（每个字一张位图），
  //    与上面那几组"整块"的走不同分支。用户看到的是同一个下拉。
  // ⚠️ 档名与 `engine.js::ANIM_PRESETS` / 转译器 `ANIM_PRESETS` **必须逐字相同**
  //    （三处同集合由 `node scripts/check-anim-presets-parity.mjs` 钉住）。
  {
    group: '弧形（逐字）',
    items: [
      { value: 'arc_up', label: '弧形 · 上拱' },
      { value: 'arc_wave', label: '弧形 · 波浪' },
      { value: 'arc_down', label: '弧形 · 下垂' }
    ]
  },
  {
    group: '大小字（逐字）',
    items: [
      { value: 'size_last', label: '大小字 · 末字重音' },
      { value: 'size_first', label: '大小字 · 首字重音' },
      { value: 'size_mid', label: '大小字 · 中字重音' }
    ]
  },
  {
    group: '抖散（逐字）',
    items: [
      { value: 'scatter_sm', label: '抖散 · 轻' },
      { value: 'scatter_lg', label: '抖散 · 重' },
      { value: 'scatter_shake', label: '抖散 · 带抖动' }
    ]
  },
  {
    group: '变小（逐字）',
    items: [
      { value: 'shrink_desc', label: '变小 · 递减' },
      { value: 'shrink_drop', label: '变小 · 骤降' },
      { value: 'shrink_tail', label: '变小 · 尾音' }
    ]
  },
  {
    group: '放大（逐字）',
    items: [
      { value: 'grow_asc', label: '放大 · 递增' },
      { value: 'grow_burst', label: '放大 · 爆开' },
      { value: 'grow_first', label: '放大 · 首字引爆' }
    ]
  }
]

/** 全部档名（**不含**空串）—— 给"认不出的档名"兜底用。 */
const ANIM_KNOWN = ANIM_GROUPS.flatMap((g) => g.items.map((i) => i.value)).filter((v) => v !== '')

/** §5 E4a：切换 element 的 `kind`。**必须同时换掉载荷字段** ——
 *  `text` 支读 `text`、`sticker` 支读 `keyword`（老变体）与 `text`（贴纸包名，
 *  见 `Event::Element` 的文档注释）。只改 `kind` 而不动载荷 = 切过去是一个空元素。
 *  ⚠️ 反方向（切回 text）**不清 text** —— 用户可能只是点错了，清掉就等于让他重打一遍。 */
function setElementKind(ev, kind) {
  ev.kind = kind
  if (kind === 'sticker') {
    // 贴纸关键词在 `text` 上（与 `Event::Sticker.keyword` 语义相同）
    if (ev.text === undefined) ev.text = ''
    delete ev.style
    delete ev.pos
  } else {
    if (ev.style === undefined) ev.style = 'comment'
    if (ev.pos === undefined) ev.pos = 'bottom'
  }
}
/** 能挂音效的类型（`SFX_CAPABLE`）。 */
const SFX_CAPABLE = EFFECTS.filter((e) => e.sfx_capable === true).map((e) => e.id)
/** 属性面板的两组（`ui` 分组；原先两组数组也在这里手写）。
 *  ⚠️ `ui` **必须是数组**（词汇表门禁会校验；2026-10-01 出过一次事故：`overlay.ui` 写成对象 `{}`
 *  ⇒ 这里 `.includes` 抛 `(intermediate value).includes is not a function` ⇒ 编辑器 setup 直接抛错）。
 *  这里再加一层防御：拿到非数组就当"没有分组"，别让一个坏词条连累整块界面。 */
const uiGroup = (name) =>
  EFFECTS.filter((e) => (Array.isArray(e.ui) ? e.ui : []).includes(name)).map((e) => e.id)

/** 轨道成员由词汇表的 `lane` 派生（D3：overlay 已并入「视觉特效」泳道，覆盖层泳道删除）。 */
const LANE_TYPES = (lane) => (lane === 'fx' ? FX_TYPES : EFFECTS.filter((e) => (e.lane ?? 'fx') === lane).map((e) => e.id))

const LANE_DEFS = [
  // `lane` = 这一条轨道对应 `[stage.lanes]` 里的哪个开关（**没有** = 结构轨、不给眼睛）。
  // ⚠️ 名字与 Rust `Lane::key` 一字不差；`check-layout-invariants.mjs` ⑯ 钉着这张表。
  { type: 'segments', label: '分段 / 屏蔽', color: TIMELINE_COLORS.segments, resizable: true },
  { type: 'subtitles', label: '字幕', lane: 'subtitles', color: 'rgba(59,130,246,0.35)', resizable: true },
  { type: 'camera', label: '运镜', lane: 'camera', color: TIMELINE_COLORS.camera, types: LANE_TYPES('camera'), resizable: true },
  // ⚠️ 轨头标签是「元素」不是「贴纸」（§5 E1）。这条轨**同时装 `sticker` 与 `element`**
  //    （`LANE_TYPES('sticker')` → `['sticker','element']`，与 `spec/effects.json` 的座位表一致），
  //    所以叫「贴纸」会让"往这条轨加一个元素"变成一件靠猜的事 ——
  //    用户看到的标签必须涵盖它能装的东西。
  //    ⚠️ `lane` 这个键**仍然是 `sticker`**（磁盘上的键名、Rust `Lane::key` 与
  //    `LANE_OF_TYPE` 都按它走）—— 只改**显示标签**，不做数据迁移。
  { type: 'sticker', label: '元素', lane: 'sticker', color: TIMELINE_COLORS.sticker, types: LANE_TYPES('sticker'), resizable: true },
  { type: 'fx', label: '视觉特效', lane: 'effects', color: TIMELINE_COLORS.flash, types: LANE_TYPES('fx'), resizable: true },
  { type: 'sfx', label: '音效', lane: 'sfx', color: TIMELINE_COLORS.sfx, types: LANE_TYPES('sfx'), resizable: true }
]
const lanes = LANE_DEFS

// ── 成片时间线（EDL）：双轨视图的第二条轴 ────────────────────────
// **默认隐藏**：`edl.clips` 为空时不渲染任何东西（老工程界面零变化，§6.2 第 4 条）。
// ⚠️ 空 clips 在后端**不是**"什么都不播"—— `ProjectFile::resolve_edl` 会拿 `mute_ranges` 的
// 补集兜底（§2.2 规则 3）。那是**业务分流**，住 Rust 一处；前端**不重算**它，
// 只把"空"读成"这条路没开"。
const edlOn = computed(() => edlVisible({ edl: { clips: edlClips.value } }))
/** 成片轨的块（纯派生：源区间、成片轴上的起点与宽、来源角标）。 */
const edlTrackBlocks = computed(() => edlBlocks({ edl: { clips: edlClips.value } }, props.srcFps, scale.value))
/** 成片总时长（秒）= 最后一段的 out_end。⚠️ 不是源时长，也不是各段源长之和（变速后两者都不对）。 */
const edlSecs = computed(() => edlOutSecs({ edl: { clips: edlClips.value } }, props.srcFps))
/** 成片轨的像素宽（两轴共用同一个横向尺度 `scale`，所以"一秒"在两轴上一样长）。 */
const edlTrackWidth = computed(() => Math.max(edlSecs.value * scale.value, 120))
/**
 * 播放头在**成片轴**上的位置。落在"不在任何成片段里的源区间"（被删掉的那段）时 `null` ——
 * 那一段在成片里不存在，这时**不画**成片轨的播放头（画在 0 或末尾都是在撒谎）。
 */
const edlHeadLeft = computed(() => {
  if (!edlOn.value) return null
  const out = edlOutSecsAtSource(edlTrackBlocks.value, Number(props.currentTime) || 0)
  return out === null ? null : out * scale.value
})
/** 选中的那一段（EDL 块）—— 属性面板的一支；与 `selected` 互斥。 */
const edlSelected = ref(-1)
const edlSelBlock = computed(() => (edlSelected.value >= 0 ? edlTrackBlocks.value[edlSelected.value] || null : null))
const edlTotalSecs = computed(() => {
  const r = edlTrackBlocks.value.filter((b) => !b.invalid)
  return r.length ? r[r.length - 1].outEndSecs : 0
})
/** 面板两个输入框显示的值：草稿优先（用户正在填），否则照块的真值。 */
const edlSpanStart = computed(() => (edlDraftSpan.value ? edlDraftSpan.value.start : (edlSelBlock.value ? round2(edlSelBlock.value.source_start) : 0)))
const edlSpanEnd = computed(() => (edlDraftSpan.value ? edlDraftSpan.value.end : (edlSelBlock.value ? round2(edlSelBlock.value.source_end) : 0)))
/** 「加入成片」菜单（fixed 定位，不受时间轴横向滚动影响；与 `fxMenu` 同款）。 */
const edlMenu = ref(null)
/** 面板里手动填的那一段的源区间（`null` = 跟随块；非 null = 草稿态，见 `commitEdlSpan`）。 */
const edlDraftSpan = ref(null)

/** 一条轨道的眼睛当前亮不亮。**只读 Rust 注入的解析结果**（`_lanes`）——
 *  默认值（素材竖/画布竖 ⇒ 运镜与视觉特效默认关）只住 Rust `ProjectFile::lane_default` 一处，
 *  前端自己再推一遍就必然两边不一致（而且不报错）。读不到注入就当亮着（宁可多画）。 */
function laneEnabled(lane) {
  const v = props.lanesOn ? props.lanesOn[lane] : undefined
  return typeof v === 'boolean' ? v : true
}
function laneEyeTitle(lane) {
  return laneEnabled(lane.lane)
    ? `${lane.label}：渲染中。点一下关掉（整类不渲染，事件仍留在时间轴上）`
    : `${lane.label}：不渲染（竖屏素材默认关）。点一下打开`
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
// **也派生自词汇表**（与 Rust `Event::default_end` / `fx.rs::transient_default_duration` 逐值一致 ——
// 门禁会把这三处钉在一起，见 `scripts/check-effect-registry.mjs` 的值级检查）。
const DEFAULT_SPAN = Object.fromEntries(EFFECTS.map((e) => [e.id, e.default_span]))
const INTENSITY_TYPES = uiGroup('intensity')
const COLOR_TYPES = uiGroup('color')
/** 时间会被后处理吸附/延长的类型（有 `manual` 字段的那两类）。 */
const MANUAL_TYPES = ['sticker', 'camera']

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
/**
 * 运镜速度（§16）：`med` 是**默认档、等于"没写过"**，所以选它要**删字段**而不是写 `speed="med"`——
 * 写上去会让 `polish.toml` 多出一个键，而存量零回归（R4）的判据正是"没设过就不出现这个键"。
 * 转译器两侧走同一条分支（`cameraTransitionFrames` 里 `med` 与缺省合并），所以删掉它行为不变、
 * 文件也不变。与 `setHandheld` 的「关掉即删字段」同一条纪律。
 */
function setCameraSpeed(ev, v) {
  if (!v || v === 'med') delete ev.speed
  else ev.speed = v
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
  // §5 E4a：`element` 是**按 `kind` 分派**的两种东西（文字走排版 / 贴纸走贴纸图），
  // 泳道上必须一眼分得出 —— 否则两块都写 "element"，用户看不出哪个是文字哪个是贴纸。
  // 旧代码没有这一支，直接落到 `return ev.type` ⇒ 两种元素统统显示字面量 "element"。
  if (ev.type === 'element') {
    const k = ev.kind || 'text'
    // ⚠️ `kind` 可能是 LLM 漏写/写错的值（Rust 侧 `#[serde(default)]` 兜空串）。
    // 认不出时**照实说**，别默认成"文字" —— 那会把一个坏数据伪装成正常元素。
    if (k === 'text') {
      const t = (ev.text || '').trim()
      return t ? (t.length > 14 ? t.slice(0, 14) + '…' : t) : '文字'
    }
    if (k === 'sticker') return (ev.text || '').trim() || ev.name || '贴纸'
    return '⚠' + k
  }
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

/**
 * 当前选中分段**相关的**系统理由（§1）。
 *
 * 归属判据分两种（理由自己不带"属于哪一段" —— 后处理是按整片算的）：
 * - **带 `at` 的**（边界类）：切点落在这一段的 `[start, end]` 里就算它的；
 * - **不带 `at` 的**（整片级，如片头淡入 / 画中画降级）：只挂在**首段**上 ——
 *   否则用户点每一段都会看到同一句"补了片头淡入"，很快就不看了。
 */
const segDecisions = computed(() => {
  if (selected.value?.kind !== 'seg') return []
  const seg = segments.value[selected.value.si]
  if (!seg) return []
  const isFirst = selected.value.si === 0
  return decisions.value.filter((d) => {
    if (d.at === null || d.at === undefined) return isFirst
    const t = Number(d.at)
    return Number.isFinite(t) && t >= seg.start - 0.001 && t <= seg.end + 0.001
  })
})

/** unix 秒 → `MM-DD HH:MM`（本地时区），用于"整片最近一次生成"那行。 */function fmtTs(ts) {
  const d = new Date(Number(ts) * 1000)
  if (Number.isNaN(d.getTime())) return ''
  const p = (n) => String(n).padStart(2, '0')
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

/** §10 情绪轨道的四维色。**只影响观感，不参与任何计算** —— 选色的唯一要求是
 *  四个维度在同一行上**一眼分得开**（色相分开），亮度也别差太远（否则深底上看不见浅的那个）。 */
const EMOTION_COLORS = { joy: '#f59e0b', shock: '#ef4444', laugh: '#22c55e', awe: '#8b5cf6' }
const EMOTION_ZH = { joy: '喜', shock: '惊', laugh: '笑', awe: '叹' }

/** §10 情绪轨道的显示块（源时间 → 像素）。
 *
 *  ⚠️ **占主导的那个维度画出来，四个都塞进去就成了一团糊**：`Emotion::dominant()`
 *     是 Rust 那边的**唯一一份**判定（平局按固定顺序 `shock > laugh > awe > joy`，
 *     为的是同一份工程两次跑出同样结果）。前端**不重算**那个顺序 ——
 *     重算一份必然漂移，而漂移的表现是"预览标红、成片按别的配方"，不报错。
 *     ⇒ 这里只按**数值大小**挑最大者；平局时按同一张固定顺序，与 Rust 对齐。
 *
 *  ⚠️ 不透明度按 `count`（参与判定的弹幕条数）走：**3 条弹幕判出的"震惊"与 300 条
 *     不是一回事**。条的窗口画淡一点，让"这里其实没多少弹幕"一眼看得出来 ——
 *     只画一个纯色块会把它伪装成同样可信。 */
/** §10 情绪 —— **单个分段**最强的那一窗（给分段块当徽标）。
 *
 *  ⚠️ 语义：情绪是**这一段**的属性（"这段为什么吵"），不是第 8 条泳道。
 *    单开一行会让同一信息在时间轴上出现两次、还白占 26px（整块按 7 条泳道定的高度）。
 *  ⚠️ 只取**与 `[seg.start, seg.end)` 真重叠**的窗口，且取最强的那一窗 ——
 *    一个分段可能横跨好几个窗口，全画会糊成一片。
 *  ⚠️ 没有窗口 / 强度 0 ⇒ 返回 `null`：**不显示**（不许画一个"中性"的占位，
 *    那会让人以为"这里判过了、是平静"，而真相是"这里没数据"）。 */
function segEmotion(seg) {
  const ws = emotionWindows.value
  if (!ws || !ws.length) return null
  const a = Number(seg.start) || 0
  const b = Number(seg.end) || a
  let best = null
  for (const w of ws) {
    const s = w.start
    const e = s + (Number(emotionRaw.value?.window_secs) || 0)
    if (e <= a || s >= b) continue // 不重叠
    if (!best || w.strength > best.strength) best = w
  }
  return best
}

const emotionWindows = computed(() => {
  const raw = emotionRaw.value
  const ws = Array.isArray(raw?.windows) ? raw.windows : []
  const ORDER = ['shock', 'laugh', 'awe', 'joy']
  return ws
    .map((w) => {
      const em = w?.emotion || {}
      const start = Number(w?.start) || 0
      const dur = Number(raw?.window_secs) || 0
      let best = ORDER[0], bestV = -1
      for (const k of ORDER) {
        const v = Number(em[k]) || 0
        // 严格大于：ORDER 的先后即平局优先级，与 Rust `dominant()` 同序
        if (v > bestV) { best = k; bestV = v }
      }
      if (!(bestV > 0)) return null // 四个维度全 0 = 这一窗什么都没判出来，不画
      const count = Number(w?.count) || 0
      const hits = Array.isArray(w?.hits) ? w.hits.slice(0, 6).join('、') : ''
      return {
        start,
        left: start * scale.value,
        width: Math.max(dur * scale.value, 4),
        // 3 条起画实，越多越实；上限 0.85 免得盖住时间刻度
        opacity: Math.min(0.85, 0.35 + Math.min(count, 30) / 30 * 0.5),
        color: EMOTION_COLORS[best] || '#64748b',
        label: EMOTION_ZH[best] || best,
        // 最强维度的强度 —— `segEmotion` 用它挑"这一段最值得说的那一窗"
        strength: bestV,
        title: `情绪 ${EMOTION_ZH[best] || best}（${best}）· ${start.toFixed(2)}s 起 ${dur}s`
          + `\n强度 ${bestV.toFixed(3)} · 参与判定 ${count} 条弹幕`
          + (hits ? `\n命中词：${hits}` : '\n命中词：无（按颜色/模式判的）'),
      }
    })
    .filter(Boolean)
})

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
      // §10 情绪徽标（并进分段块，不单开泳道）—— 见 `segEmotion`
      const emo = segEmotion(seg)
      out.push({
        kind: 'seg', si,
        time: seg.start,
        left: seg.start * scale.value,
        width: Math.max((seg.end - seg.start) * scale.value, 8),
        label: seg.note || ('分段 ' + (si + 1)),
        emo,
        title: '分段 ' + Number(seg.start).toFixed(1) + 's → ' + Number(seg.end).toFixed(1) + 's'
          + (emo ? `\n${emo.title}` : '')
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
        // §5 E4a：`element` 的两种 `kind` 用**不同颜色** —— 只靠 label 区分太弱
        // （泳道块很窄，字常被截断）。文字沿用元素轨底色，贴纸换成贴纸色系。
        color: (type === 'element' && (ev.kind || 'text') === 'sticker')
          ? '#7c3aed'
          : (TIMELINE_COLORS[type] || lane.color),
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
  // 选中时间轴上的块 ⇒ 关掉轨道设置（两块面板共用一列，属性面板同一时刻只显示一份）
  if (props.laneOpen) emit('lane-open', '')
  // 成片段与源块的面板**互斥**共存（同一列）：选源块就把成片段让出去。
  edlSelected.value = -1
  if (block.kind === 'sub') selected.value = { kind: 'sub', lane: lane.type, subIndex: block.subIndex }
  else if (block.kind === 'mute') selected.value = { kind: 'mute', lane: lane.type, muteIndex: block.muteIndex }
  else selected.value = { kind: block.kind, lane: lane.type, si: block.si, ei: block.ei }
}
/** 块/选中 → 落盘分组（决定 PUT 到哪个接口、以及要不要跑后处理）。 */
const dirtyKindOf = (kind) => (kind === 'sub' ? 'subtitles' : kind === 'mute' ? 'mute' : kind === 'edl' ? 'edl' : 'segments')

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
const dirty = { segments: false, subtitles: false, mute: false, edl: false }
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

/// 落盘防抖窗口（毫秒）。
///
/// ⚠️ 它**不是**"编辑完成"的信号：真正的完成信号是 `commitEdit` / 取消选中 / 切片段 / 卸载前
/// 那几次 `flushSave()` 挂的 `onceSaved`（见下方 `commitEdit` 的长注释）。
/// 250ms 而不是 700ms —— 触发点都是**离散**改动（控件走 `change` 而非 `input`；拖动只在松手时算一次），
/// 所以窗口只需盖住"连点几下"的合并，而"松手 → 真的写进磁盘"是用户手感的全部来源：
/// 700ms 时它还要再叠上预览侧的重载防抖，一次编辑要一秒以上才在画面上收口。
/// **别再往上抬**；要动先量一遍"松手 → 预览跟上"的端到端时间。
const SAVE_DEBOUNCE_MS = 250

function touch(kind) {
  markDirty(kind)
  clearTimeout(saveTimer)
  saveTimer = setTimeout(flushSave, SAVE_DEBOUNCE_MS)
}
/**
 * 把"这次编辑已经落盘了"这个名字挂到**当前这一轮**保存上。
 *
 * 为什么需要它：页面要拿"落盘完成"当 Dhampir 那条腿的重载信号 —— 工程是**从磁盘
 * `polish.toml` 转译**出来的，抢在 PUT 之前去读只会拿回旧内容（画面闪回改动前）。
 *
 * 注册时若手上已经没有待落的改动（比如上一次 flushSave 刚好跑完），**立刻回调** ——
 * 否则那个"完成"信号会永远等一个不会到来的保存，页面就一直不重载。
 */
let onSavedHook = null
function onceSaved(fn) {
  const pending = !!saveTimer || dirty.segments || dirty.subtitles || dirty.mute || dirty.edl || saving
  if (!pending) { fn(); return }
  onSavedHook = fn
}
/**
 * 「这批改动已经落盘」——发给页面，让 Dhampir 那条腿据此重载（它只认磁盘）。
 *
 * ⚠️ 与 `preview`（本地镜像、不落盘）是**两个不同的信号**，别合并：预览要立刻变，
 * 而"重新拉工程"必须等到磁盘真被写过。
 */
function fireSavedHook() {
  const fn = onSavedHook
  onSavedHook = null
  if (fn) fn()
  emit('saved')
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
  if (dirty.edl) payload.edl = { clips: edlClips.value }
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

/**
 * 「编辑完成」= **这次改动已经落盘**，然后才通知页面。
 *
 * ⚠️ **顺序不能反**（2026-10-02 修）：以前这里是把 `done` 当场发出去，而落盘是
 * `touch()` 排的 250ms 定时器 —— **`done` 抢在落盘前面**。页面收到"完成了"就带着
 * "磁盘上已有这次改动"的假设去干活（跑 `process`、以及重新拉 Dhampir 工程），
 * 而磁盘上那时还是旧的。
 *
 * 以前侥幸没出事，纯粹是时序巧合：`process` 前面还压着 1.5s 防抖 > 250ms 落盘。
 * 一旦有消费者不等那 1.5s（Dhampir 热重载就是），它立刻读到旧内容 ——
 * 表现正是"拖完松手，画面先闪回改动前的样子"。
 *
 * 所以 `done` 挂到"落盘成功"上；页面那边的防抖与后处理顺序都不用动。
 */
function commitEdit(reason, dirOverride) {
  if (!editTouched) return
  editTouched = false
  const payload = { dir: dirOverride || props.dir, reason }
  // 切片段时 props.dir 已经是新的，必须显式传**旧目录**，否则后处理会跑到新片段上
  onceSaved(() => emit('done', payload))
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
  // 成片段：两端对齐到 2 位小数、夹到源时长内，并保证 `end > start`。
  // ⚠️ **不重排**（数组顺序 = 成片播放顺序，§2.1 —— 排序会改掉成片本身）。
  // ⚠️ 这里**只归一**，不做"删掉非法段"那种破坏性动作：非法段由 UI 标红，让用户自己决定
  //（后端 `resolve_clips` 会跳过它，不 panic）。
  if (dirty.edl) {
    const lim = getDuration()
    edlClips.value = edlClips.value.map((c) => {
      const out = { ...c }
      const sp = Number(out.speed)
      out.speed = Number.isFinite(sp) && sp > 0 ? Math.min(EDL_SPEED_MAX, Math.max(EDL_SPEED_MIN, sp)) : 1.0
      let st = Number(out.source_start)
      let en = Number(out.source_end)
      if (!Number.isFinite(st)) st = 0
      if (!Number.isFinite(en)) en = st + MIN_CLIP_SECS
      st = Math.max(0, Math.min(lim, st))
      en = Math.max(0, Math.min(lim, en))
      out.source_start = round2(st)
      out.source_end = round2(en)
      return out
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
  const edls = dirty.edl ? { clips: edlClips.value } : null
  // 没有待落的改动 = 磁盘上已经是最新的 ⇒ 等在这里的那个「完成」信号没有保存可等了，
  // 当场放行（否则页面会为一个**本来就已落盘**的改动一直不重载）。
  if (!segs && !mutes && !subs && !edls) { fireSavedHook(); return }
  normalizeLocal()
  saving = true
  saveState.value = 'saving'
  try {
    if (segs || mutes || edls) {
      const patch = {}
      if (segs) patch.segments = segs
      if (mutes) patch.mute_ranges = mutes
      // 成片时间线（`[[edl.clips]]`）：整个 `edl` 块提交（键缺席 = 不动磁盘）。
      // ⚠️ 删空最后一段时提交的是 `{ clips: [] }` —— 后端 `ProjectFile.edl` 字段上的
      // `skip_serializing_if = "Edl::is_empty"` 会让它**一个字节都不写**（R4）。
      // 前端**不做**这层过滤：序列化口径只有后端一处，前端再滤一遍就是第二个真源。
      if (edls) patch.edl = edls
      await saveProjectPatch(props.pid, props.dir, patch)
    }
    if (subs) {
      await saveSubtitles(props.pid, props.dir, subs)
    }
    dirty.segments = dirty.subtitles = dirty.mute = dirty.edl = false
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
    // 「落盘完成」的信号在这一刻发出去（成功才发；失败留在 dirty 上等退避重试，
    // 由那一次成功的 flushSave 补发 —— 页面不会为一个没落盘的改动去重载）。
    if (!dirty.segments && !dirty.subtitles && !dirty.mute && !dirty.edl && !loadError.value) fireSavedHook()
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
 * `process` 跑完后调它来同步，可只要当时在 250ms 防抖窗口里、或用户正拖着块，这次同步就被
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
    const pending = !!saveTimer || dirty.segments || dirty.subtitles || dirty.mute || dirty.edl
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
  // ⚠️ 成片轨同样**必须清空**：旧片段的 clips 留着，下一次编辑就会把它写进新片段的
  // `polish.toml`（与上面那三行同一条"跨片段串写"的引信）。
  edlClips.value = []
  edlSelected.value = -1
  edlDraftSpan.value = null
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
/** 本次拖拽是否**真的**产生了位移（纯点击不算一次编辑 → 不触发后处理）。 */
let dragMoved = false

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
      // 位移超过 1px 才算"真的拖过"（手抖 / 点一下不该触发一次后处理）
      if (Math.abs(evt.clientX - startX) > 1) dragMoved = true
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
      if (Math.abs(evt.clientX - startX) > 1) dragMoved = true
      const ns = Math.max(0, Math.min(origEnd - minLen, origStart + delta))
      put(ns, origEnd)
      markDirty(dk)
    },
    'resize-r'(evt) {
      const delta = (evt.clientX - startX) / scale.value
      if (Math.abs(evt.clientX - startX) > 1) dragMoved = true
      const ne = Math.max(origStart + minLen, Math.min(dur, origEnd + delta))
      put(origStart, ne)
      markDirty(dk)
    }
  }[mode]

  if (!apply) return
  drag = { move: apply }
  dragMoved = false
  const onMove = (evt) => { if (drag) drag.move(evt) }
  const onUp = (evt) => {
    removeListeners()
    const moved = dragMoved
    drag = null
    dragMoved = false
    touch(dk)
    // ⚠️ **拖拽松手 = 一次编辑做完了 → 必须通知页面跑后处理**（2026-10-01 修）。
    //
    // 只 `touch(dk)` 是不够的：它只落盘，而后处理（贴纸落点 / 锚点坐标 / 画中画守卫）
    // 要等 `commitEdit` 发出的 `done`。修之前这里**没有** commitEdit，于是拖完一个事件
    // 后处理**一直不跑** —— 而拖拽是改事件时间最常见的方式。
    //
    // 当时唯一能救回来的路径是"取消选中"（`watch(selected)`），但 `startPointer` 一进来就
    // `selectBlock`，松手时那一项**仍然选中着**；模板里又没有"点空白取消选中"
    // （只有属性面板那颗「关闭」按钮与删除会清空 selected）。所以实际表现是：
    // 拖完事件 → 画面上的贴纸/运镜仍是旧坐标 → 用户以为"后处理坏了"，
    // 直到**切片段或离开页面**才补跑一次（那时才 emit done）。
    //
    // 只在**真的动过**时才通知：一次点击（pointerdown+up 没位移）不该触发后处理。
    if (moved) commitEdit('拖拽结束')
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
  // §5 E4a：`element` 必须带上 `kind` —— 它是**按 kind 分派**的（text 走排版、sticker 走贴纸图）。
  // ⚠️ 以前这里没有这一支 ⇒ 新建出来的 element 是 `{type,time,end}`，
  //    `kind` 缺失 ⇒ 反序列化时靠 `#[serde(default)]` 变空串 ⇒ 出片侧按"认不出的 kind"处理。
  //    而 `kind = "text"` 且 `text` 为空时，转译器会 `drop('文字元素没有 text')` ——
  //    也就是用户"新建了一个元素，界面上什么都没有、成片里也什么都没有"。
  //    默认给 `text`：它是最常用的一支，且空文本会在属性面板里立刻被看见。
  else if (type === 'element') { ev.kind = 'text'; ev.text = ''; ev.style = 'comment'; ev.pos = 'bottom' }
  else if (type === 'overlay') { ev.overlay_type = 'color'; ev.color = '#ff3232'; ev.opacity = 0.3 }
  else if (type === 'sfx') { ev.name = ''; ev.volume = 0.3 }
  else if (INTENSITY_TYPES.includes(type)) ev.intensity = type === 'shake' ? 14 : 30
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

/**
 * 贴纸选择器选完之后写回**当前选中的事件**。
 *
 * ⚠️⚠️ **必须同时认 `sticker` 与 `element(kind=sticker)` 两种载体**：
 * 贴纸有两代写法并存（`Event::Sticker` 是老的、`Event::Element{kind:"sticker"}` 是 §5 E2 迁进来的），
 * 而两者的**关键词字段名不一样**：
 *   · `type === 'sticker'` → 关键词在 **`keyword`**
 *   · `type === 'element'` + `kind === 'sticker'` → 关键词在 **`text`**（Rust `Element::text` 的文档写明）
 *
 * 老代码只认 `ev.type !== 'sticker'` 就 `return`，于是**在元素轨上选好贴纸图之后什么都没发生**
 * （用户报的「选完没反应」）—— 选项看着选中了、`touch()` 也没跑，一刷新全空。
 * ⚠️ 别退回"只写 `ev.keyword`"：`Element` 根本没有这个字段，写了也是给对象挂一个
 * 会被 serde 丢掉的野键（**不报错、不落盘**）。
 */
function onStickerPick(patch) {
  const ev = selectedEv.value
  if (!ev) return
  const isSticker = ev.type === 'sticker'
  const isElementSticker = ev.type === 'element' && (ev.kind || 'text') === 'sticker'
  if (!isSticker && !isElementSticker) return
  // 关键词落在哪个字段由载体决定（见上）
  if (patch.keyword !== undefined) {
    if (isSticker) ev.keyword = patch.keyword || ''
    else ev.text = patch.keyword || ''
  }
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
  // ⚠️ **元素轨必须给菜单、不能直接加**：这条轨道装 `sticker` 与 `element` 两种事件类型，
  //    直接 `addEventAt('sticker')` 会让 `element`（即"文字"那一支）**永远造不出来**。
  //    与视觉特效同规格（类型由轨道分配，面板里只允许在同轨道的细分类型之间换）。
  if (laneType === 'sticker') { openElementMenu(evt); return }
  addEventAt(laneType)
}

const fxMenu = ref(null) // { x, y } | null —— 视觉特效的细分类型菜单（fixed 定位，不受时间轴滚动影响）
const elementMenu = ref(null) // 同上，元素轨的「文字 / 贴纸」二选一

function openElementMenu(evt) {
  const r = evt.currentTarget.getBoundingClientRect()
  elementMenu.value = { x: Math.round(r.left), y: Math.round(r.bottom + 4) }
  document.addEventListener('pointerdown', closeElementMenu, true)
}
function closeElementMenu(e) {
  if (e && e.target && e.target.closest && e.target.closest('.fx-menu')) return
  if (!elementMenu.value) return
  elementMenu.value = null
  document.removeEventListener('pointerdown', closeElementMenu, true)
}
/** 在播放头处加一个**指定 kind** 的元素（`text` 走排版、`sticker` 走贴纸图）。 */
function addElementAt(kind) {
  closeElementMenu()
  // ⚠️⚠️ **两种 kind 都建 `element`**（差别只在 `kind` 字段）。
  //
  // 旧代码写的是 `kind === 'text' ? 'element' : 'sticker'` —— 选「贴纸」会建出一条
  // **老式 `type:"sticker"` 事件**（`Event::Sticker`），于是：
  //   ① 属性面板走的是 `.type==='sticker'` 那一支，**根本没有「元素种类」下拉**
  //      ⇒ 用户点的明明是「元素轨 → 贴纸」，「文字/贴纸」的切换却不存在（用户报的正是这个）；
  //   ② 磁盘上留下的是 `type:"sticker"`，而不是元素轨该有的 `type:"element"`。
  // 两个变体在 Rust 侧**长期共存**（老工程全是 `sticker`，§5 E2 只是"迁进来"、没删老变体），
  // 所以这不是"数据坏了"—— 而是**新建时选错了载体**，等于每次点贴纸都退回旧写法。
  // ⇒ 元素轨的 ＋ 一律产 `element`，老 `sticker` 事件只读进来、继续能用。
  const before = segments.value.reduce((n, s) => n + s.events.length, 0)
  addEventAt('element')
  // `addEventAt` 在"没有分段"或越界时**不加**（它会自己 toast）——
  // 这时别去改一个不存在的事件（下标 -1 会写到上一段的最后一条上）。
  const after = segments.value.reduce((n, s) => n + s.events.length, 0)
  if (after === before) return
  // `defaultEvent` 给的是 `kind='text'`；这里按用户选的 kind 落到正确载荷上。
  // `addEventAt` 会把新块设为选中（`selected` = `{kind:'event', si, ei}`），
  // 所以 `selectedEv` 拿到的就是刚建的那条 —— **别在这儿重算一遍下标**（下标算法只住 `addEventAt`）。
  if (kind === 'sticker') {
    const ev = selectedEv.value
    if (ev && ev.type === 'element') setElementKind(ev, 'sticker')
  }
}

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

// ── 成片时间线（EDL）：加入 / 复制 / 拖缘 / 变速 / 删除 ───────────
// 全部改变的是 `edlClips`（`polish.toml` 的 `[[edl.clips]]`），**一个源时间的字段都不碰**
// （§6.1：源轴 7 条泳道的 x 坐标永不被 EDL 影响）。
//
// 落盘走 `saveProjectPatch({ edl: { clips } })` —— ⚠️ 它经 `polish_project_put_locked` 的
// **逐字段白名单**：`edl` 那一支是 task-4 加的（本批之前 PUT 会静默丢掉它、还回 `{"ok":true}`）。
// 所以这里的改动**必须**能真写进 `polish.toml`，否则就是"看着能存、实际没存"。
//
// ⚠️ 空 EDL 一个字节都不许写：判据挂在后端 `ProjectFile.edl` 字段的
// `skip_serializing_if = "Edl::is_empty"` 上（**不是**只靠 `Edl::clips` 自己那层 ——
// toml 照样会写出空的 `[edl]` 段头）。前端这边只保证"用户删空最后一段时提交的是空数组"，
// 序列化那一步归后端，**不要**在前端替它过滤（那会变成第二个真源）。

/** 当前框选/选中要加入成片的那段源区间：优先「分段 / 屏蔽」轨上的选中块，否则播放头前 3 秒的窗口。 */
function edlSourceRange() {
  const it = selItem.value
  if (it && selected.value) {
    const k = selected.value.kind
    if (k === 'seg') {
      const s = Number(it.start) || 0
      const e = Number(it.end) || s
      if (e - s >= 0.05) return { start: round2(s), end: round2(e) }
    }
    if (k === 'seg' && it.events) return null
    if (k === 'sub' || k === 'mute') {
      const s = Number(it.start) || 0
      const e = Number(it.end) || s
      if (e - s >= 0.05) return { start: round2(s), end: round2(e) }
    }
    if (k === 'event') {
      const s = Number(it.time) || 0
      const e = s + evDur(it)
      if (e - s >= 0.05) return { start: round2(s), end: round2(e) }
    }
  }
  const dur = getDuration()
  const t = Math.max(0, Number(props.currentTime) || 0)
  const end = Math.min(dur, t + 3)
  if (end - t < 0.05) return null
  return { start: round2(t), end: round2(end) }
}

/** 打开「加入成片」菜单（固定定位在触发按钮下方）。 */
function openEdlMenu(evt) {
  const r = evt.currentTarget.getBoundingClientRect()
  edlMenu.value = { x: Math.round(Math.min(r.left, window.innerWidth - 180)), y: Math.round(r.bottom + 4) }
  document.addEventListener('pointerdown', closeEdlMenu, true)
}
function closeEdlMenu(e) {
  if (e && e.target && e.target.closest && e.target.closest('.edl-menu')) return
  if (!edlMenu.value) return
  edlMenu.value = null
  document.removeEventListener('pointerdown', closeEdlMenu, true)
}

/**
 * 「加入成片」的三个入口（§6.3 第 ② 条）。
 *  · `head` 插到片头 · `at` 插到当前播放头 · `tail` 追加到片尾
 */
function addEdl(where) {
  closeEdlMenu()
  const r = edlSourceRange()
  if (!r) { toast('先在源时间轴上选一段（分段/字幕/屏蔽块，或把播放头停在起点）', 'err'); return }
  const at = where === 'at' ? clipIndexAtSource(edlTrackBlocks.value, Number(props.currentTime) || 0) + 1 : -1
  const next = addRangeToEdl(edlClips.value, r.start, r.end, getDuration(), where, at)
  if (next.length === edlClips.value.length) { toast('这一段太短，没有加进去', 'err'); return }
  edlClips.value = next
  edlSelected.value = next.length - 1
  selected.value = null // 面板让位给这一段（两份面板同一时刻只显示一份）
  touch('edl')
  toast(`已加入成片：${r.start.toFixed(2)}s → ${r.end.toFixed(2)}s`)
}

/** 「×2 重放」= 复制这一段并排在其后（§6.3 第 ④ 条）。连点几次就是重放几次。 */
function edlDuplicate(i) {
  const next = duplicateClipAt(edlClips.value, i)
  if (next.length === edlClips.value.length) return
  edlClips.value = next
  edlSelected.value = i + 1
  touch('edl')
}

/** 双击块 = 删除这一段（与其余轨道同一套交互：双击即删 + 「撤销删除」兜底）。 */
function edlDelete(i) {
  if (i < 0 || i >= edlClips.value.length) return
  const clip = edlClips.value[i]
  edlClips.value = removeClipAt(edlClips.value, i)
  if (edlSelected.value === i) edlSelected.value = -1
  else if (edlSelected.value > i) edlSelected.value -= 1
  rememberDeleted('成片段', clip, (item) => {
    const arr = [...edlClips.value]
    insertAt(arr, i, item)
    edlClips.value = arr
    touch('edl')
  })
  touch('edl')
}

/** 拖块的**左缘/右缘**改的是 `source_start` / `source_end`（§6.3 第 ③ 条）。 */
function startEdlDrag(e, block, mode) {
  if (block.invalid) return
  e.preventDefault()
  e.stopPropagation()
  edlSelected.value = block.index
  selected.value = null
  const startX = e.clientX
  const dur = getDuration()
  const i = block.index
  const origStart = block.source_start
  const origEnd = block.source_end
  let moved = false
  const apply = (evt) => {
    const delta = (evt.clientX - startX) / scale.value
    if (Math.abs(evt.clientX - startX) > 1) moved = true
    if (mode === 'resize-l') {
      edlClips.value = setClipSpan(edlClips.value, i, Math.min(origEnd - MIN_CLIP_SECS, origStart + delta), origEnd, dur)
    } else {
      edlClips.value = setClipSpan(edlClips.value, i, origStart, Math.max(origStart + MIN_CLIP_SECS, origEnd + delta), dur)
    }
    touch('edl')
  }
  const onMove = (evt) => apply(evt)
  const onUp = () => {
    window.removeEventListener('pointermove', onMove)
    window.removeEventListener('pointerup', onUp)
    window.removeEventListener('pointercancel', onUp)
    if (moved) commitEdit('成片段拖拽结束')
  }
  window.addEventListener('pointermove', onMove)
  window.addEventListener('pointerup', onUp)
  window.addEventListener('pointercancel', onUp)
}

/** 面板改速率（档位按钮）。后端第 4 批起全部档位可用（见 `EDL_SPEEDS`）。 */
function edlSetSpeed(i, v) {
  edlClips.value = setClipSpeed(edlClips.value, i, v)
  touch('edl')
}

/** 面板填的两端（**草稿态**：必须按「应用」才写进 clips，与"固定落点"那条口径一致）。 */
function startEdlSpanEdit() {
  const b = edlSelBlock.value
  if (!b) return
  edlDraftSpan.value = { start: round2(b.source_start), end: round2(b.source_end) }
}
function applyEdlSpan() {
  const d = edlDraftSpan.value
  const b = edlSelBlock.value
  if (!d || !b) return
  const next = setClipSpan(edlClips.value, b.index, Number(d.start), Number(d.end), getDuration())
  if (next === edlClips.value) return
  edlClips.value = next
  edlDraftSpan.value = null
  touch('edl')
  commitEdit('成片段改区间')
}
function cancelEdlSpanEdit() { edlDraftSpan.value = null }

/** 面板改显示名（可选字段，清空即删键）。 */
function edlSetLabel(i, v) {
  edlClips.value = setClipLabel(edlClips.value, i, v)
  touch('edl')
}
/**
 * 面板改**入场转场**（第 7 批 / task-10）：`'cut'` = 硬切（**删键**，缺省即硬切）、
 * `'dissolve'` = 与上一段重叠 0.5s 淡化。
 *
 * ⚠️ **硬切写"删键"而不是写 `transition_in = "cut"`**：字段缺席与显式 `"cut"` 在
 * Rust / 转译器 / 这里三处**同口径**（都是硬切），但删键能让老工程与"没动过转场"的工程
 * **逐字节相同**（R4）—— 写一个显式 `"cut"` 会让每次点一下按钮都在 `polish.toml` 里多一行。
 */
function edlSetTransition(i, v) {
  edlClips.value = setClipTransition(edlClips.value, i, v)
  touch('edl')
}
/** 面板「定位」：把预览跳到这一段**在源时间轴上**的起点（预览是源时间驱动的）。 */
function edlSeekSource() {
  const b = edlSelBlock.value
  if (b) seek(b.source_start)
}
/** 鼠标停在成片轨上时给一条"成片 x 秒 = 源 x 秒"的提示（§6.2 第 3 条）。 */
function edlBlockTitle(b) {
  const sp = b.speed === 1 ? '×1.0' : '×' + b.speed
  return [
    `成片第 ${b.outStartSecs.toFixed(2)}s → ${b.outEndSecs.toFixed(2)}s（共 ${(b.outEndSecs - b.outStartSecs).toFixed(2)}s）`,
    `源 ${b.source_start.toFixed(2)}s → ${b.source_end.toFixed(2)}s · 速率 ${sp}`,
    b.label ? `名字：${b.label}` : '',
    b.transition === 'dissolve' ? '入场：叠化（与上一段重叠 0.5s 淡化）' : '',
    b.invalid ? '⚠ 这一段不合法（结束必须晚于开始、速率必须为正），出片会跳过它' : '',
    '拖左右缘改源区间 · 双击删除',
  ].filter(Boolean).join('\n')
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
  // 同一份判据（`utils.js`）：输入框/下拉/富文本/输入法合成中一律不接管
  if (isEditableKeyEvent(e)) return
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
  document.removeEventListener('pointerdown', closeEdlMenu, true)
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

defineExpose({ reload, save: flushSave, segments, subtitles, muteRanges, edlClips })

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
  // §1 系统理由：缺这个键 = 老工程 / 没跑过后处理 ⇒ 空数组（不是错误）。
  decisions.value = Array.isArray(cfg?.decisions) ? cfg.decisions : []
  muteRanges.value = Array.isArray(cfg?.mute_ranges) ? cfg.mute_ranges.map(m => ({ start: Number(m.start) || 0, end: Number(m.end) || 0 })) : []
  // 成片时间线：**缺 `[edl]` 段 = 老工程 = 空数组**（后端 `serde(default)`）。
  // ⚠️ 空数组在这里读成"这条路没开"，**不是**"什么都不播" —— 补集兜底住 `resolve_edl` 一处。
  edlClips.value = Array.isArray(cfg?.edl?.clips) ? cfg.edl.clips.map(c => ({ ...c })) : []
  edlSelected.value = -1
  edlDraftSpan.value = null
  applySilenceState(cfg)
  const last = cfg?.segments?.[cfg.segments.length - 1]
  duration.value = cfg?.duration || last?.end || 60
  loudness.value = cfg?.loudness_curve || []
  // 磁盘上的数据就是"已保存"的状态：清掉脏标记与状态文字
  dirty.segments = dirty.subtitles = dirty.mute = dirty.edl = false
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

  // §10 情绪轨道：**404 同样是正常空态**（没有弹幕 / 没跑过 process 的工程就没有这个产物）。
  // ⚠️ 它**不影响 loadError / 保存闸门** —— 这是一条只读观察轨，读不到只是"没得看"，
  //    与"基线不可信、必须暂停写盘"是两件事，别把它并进上面那条失败路径。
  // ⚠️ 其它错误也**只 warn 不打断**：理由同上（它不参与任何写路径）。
  try {
    const emoRes = await fetchEmotion(props.pid, props.dir, '读取情绪轨道')
    emotionRaw.value = emoRes || null
  } catch (e) {
    emotionRaw.value = null
    if (e.status !== 404) addClient('warn', '情绪轨道读取失败（不影响编辑）：' + (e.message || e), 'timeline')
  }
  return true
}
</script>

<style scoped>
.timeline { width:100%; }
/* §8 叙事角色的**只读**展示（不是输入框）：样式做成"一行文字"而不是控件，
   免得用户以为它能点/能改（它确实不能 —— 见模板注释里的理由）。 */
.role-readonly { font-size:12px; color:var(--ink-dim); background:var(--bg-soft);
  border:1px solid var(--line-soft); border-radius:6px; padding:4px 8px; cursor:default; }
/* 头部一行（标题/提示 + 去静音）；原来标题在卡片头、去静音单独占一行，共省 ~70px */
.tl-head { display:flex; align-items:center; justify-content:space-between; gap:10px;
  flex-wrap:wrap; margin-bottom:6px; }
.tl-head-title { display:flex; align-items:center; gap:8px; flex-wrap:wrap; min-width:0; }
.tl-head-title :deep(b) { font-size:13px; }
/* 没有分段时的提示行：整条时间轴照常渲染，这里只给一句提示（按钮已删，见模板注释） */
.tl-no-seg { display:flex; align-items:center; gap:10px; flex-wrap:wrap; margin:4px 0 2px; }
.timeline-body { flex:1; min-width:0; display:flex; overflow-x:auto; align-items:stretch; }
.timeline-labels { flex:none; position:sticky; left:0; z-index:6; display:flex; flex-direction:column; background:var(--card); box-shadow:2px 0 8px rgba(32,32,32,.10); }
.timeline-tracks { flex:none; min-width:0; }
/* 泳道 34 → 26px（7 条泳道省 ~56px）；块 24 → 20px */
.lane { height:26px; border-bottom:1px solid var(--line-soft); position:relative;
  display:flex; align-items:center; gap:10px; padding:0 12px; box-sizing:border-box; }
.lane-label { width:124px; height:26px; box-sizing:border-box; flex:none; display:flex; align-items:center; gap:4px; padding:0 8px; font-size:11.5px; color:var(--ink-dim); border-bottom:1px solid var(--line-soft); }
/* 眼睛：13px 见方的一枚小按钮。灭的时候整枚变淡（**不隐藏** —— 用户要看见"这条轨道被关了"） */
.lane-eye { flex:none; display:flex; align-items:center; justify-content:center; width:16px; height:16px;
  border-radius:4px; color:var(--main-deep); background:var(--bg-soft); cursor:pointer; }
.lane-eye:hover { background:var(--main-soft); }
.lane-eye.off { color:var(--ink-faint); background:transparent; }
.lane-eye-none { background:transparent; color:var(--line); cursor:default; }
/* 轨道名：点它 = 这条轨道的设置。`min-width:0` + 省略号是必需的 —— 轨道名一长（分段 / 屏蔽）
   会把这一列撑破，而这一列是 `flex:none` 的定宽列。 */
.lane-name { flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  text-align:left; background:none; border:none; padding:0; font-size:11.5px; color:inherit; cursor:pointer; }
.lane-name:hover { color:var(--main-deep); }
.lane-name.on { color:var(--pink-text); font-weight:700; }
.add-lane { background:none; border:none; color:var(--ink-faint); cursor:pointer; font-size:14px; line-height:1; padding:2px 4px; }
.add-lane:hover { color:var(--main-deep); }
.lane-adds { display:flex; align-items:center; gap:2px; }
.mute-add { color:#ef4444; }
.mute-add:hover { color:#b91c1c; }
.lane-track { position:relative; height:26px; }
/* ── 成片轨（EDL）：**第二条时间轴**，26px 与源泳道等高（不挤上面那 7 条）──
   底色与源泳道区分开（源 = 素材、成片 = 结果），块与块首尾相接（§6.4 方案 A：空隙 = 不播）。 */
.edl-lane { background:var(--bg-soft); }
/* §10 情绪徽标 —— 挂在**分段块**里（不再单开一条泳道）。
   ⚠️ 不写死颜色：四维各一色，由 `block.emo.color` 行内给。 */
.emo-badge { flex:none; margin-right:5px; padding:0 4px; border-radius:3px;
  font-size:9px; line-height:14px; color:#fff;
  text-shadow:0 1px 1px rgba(0,0,0,.45); }
.edl-track { background:repeating-linear-gradient(90deg, transparent 0 39px, var(--line-soft) 39px 40px); }
.edl-label { background:var(--bg-soft); }
.edl-label-name { color:var(--main-deep); }
/* 成片段：底色与源泳道的块刻意不同（源是蓝/红/橙一族，成片走靛紫），
   这样"哪条轨上的块"不用读文字就能分辨。 */
.edl-block { background:#6d5bd0; top:3px; height:20px; padding:0 4px; cursor:default;
  display:flex; align-items:center; gap:4px; }
.edl-block.selected { outline:2px solid #fff; z-index:3; }
/* §6.5：**素材库来源的段必须视觉可分**（不同底色 + 来源角标）——
   它是最贵的一类错误（成片里突然出现别的视频，预览却看不出来）。 */
.edl-block.edl-asset { background:#0f766e; }
/* 非法段（倒置 / 零长 / 速率非正）：出片会跳过它，必须当场看得出来 */
.edl-block.edl-bad { background:#b91c1c; outline:1.5px dashed #fca5a5; }
.edl-lab { flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.edl-sp { flex:none; opacity:.85; font-variant-numeric:tabular-nums; }
.edl-badge { flex:none; font-size:9px; line-height:14px; padding:0 4px; border-radius:3px;
  background:rgba(255,255,255,.28); }
.edl-badge.asset { background:#fde68a; color:#7c4a03; font-weight:700; }
/* 转场角标（第 7 批）：只有叠化才画。⚠️ 必须 `flex:none` —— 块窄的时候它会被
   可收缩的兄弟（`.edl-lab`）挤没，而"挤没"就等于"用户看不出这一段有叠化"。 */
.edl-xf { flex:none; font-size:11px; line-height:14px; padding:0 3px; border-radius:3px;
  background:#a5b4fc; color:#1e1b4b; font-weight:700; }
.edl-rep { flex:none; border:none; border-radius:3px; padding:0 4px; height:14px; line-height:14px;
  font-size:9px; cursor:pointer; background:rgba(255,255,255,.32); color:#fff; }
.edl-rep:hover { background:rgba(255,255,255,.55); color:#1e1b4b; }
/* 面板里的成片摘要行 + 速率档位 */
.edl-sum { margin-top:4px; font-size:12px; color:var(--ink); font-variant-numeric:tabular-nums; }
.edl-speeds { flex-wrap:wrap; gap:4px; }
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
/* §1 系统理由：只读回执。左缘一道竖线把它与"可编辑的字段"在视觉上分开 ——
   用户点进来的第一反应是"这能不能改"，答案是不能（改结果请拖边界）。 */
.seg-why { border-left: 2px solid var(--line, var(--bg-soft)); padding-left: 8px; }
.seg-why .chip { font-size: 10px; padding: 1px 6px; }
.why-row { display:flex; align-items:baseline; gap:6px; margin-top:5px; }
/* 圆点：边界类（品牌橙）与其它（金发调）—— 让用户扫一眼就分得出是哪一类决策。
   ⚠️ 只用 tokens 里**存在**的变量：本仓没有 `--warn`（写过一次，静默拿到 fallback）。 */
.why-dot { flex:none; width:6px; height:6px; border-radius:50%; margin-top:1px;
  background: var(--ink-faint); }
.why-dot.b { background: var(--main); }
.why-dot.o { background: var(--tan-deep); }
/* ⚠️ 文案是**人话**、可能比较长，必须允许折行（右栏内容宽只有 266px） */
.why-text { flex:1 1 auto; min-width:0; font-size: 11px; line-height:1.5; color: var(--ink); }
.why-detail { flex:none; font-size:10px; white-space:nowrap; }
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
  border:1px solid var(--line); border-radius:10px; box-shadow:0 8px 22px var(--scrim-soft); }
.fx-menu-t { padding:2px 8px 4px; }
.fx-menu-item { display:flex; justify-content:space-between; gap:8px; width:100%; padding:3px 8px;
  border:none; border-radius:6px; background:none; color:var(--ink); font-size:12px; cursor:pointer; text-align:left; }
.fx-menu-item:hover { background:var(--bg-soft); color:var(--main-deep); }
</style>
