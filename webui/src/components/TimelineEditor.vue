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
    <!-- ⚠️ **「文字 / 贴纸」二选一菜单已删除**（2026-10-14，用户口径「前端编辑器也要分离」）。
         它存在的唯一理由是"元素轨同时装两种东西"；现在轨道已经分开了
         （「文字」轨 / 「贴纸」轨），各自的 ＋ 直接建对应 kind 的元素 —— 见 `onLaneAdd`。
         ⚠️ 别再加回来：那会把"在文字轨上点＋，却被问要不要贴纸"这个多余的选择带回来。 -->
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
          <!-- ⚠️ 行内文案按仓里的预算（≤20 字）压过：28 字那条拆成"行内说是什么 + title 说条件" -->
          <p class="mini-hint" title="叠化 = 画面把本段挪到上层、用 0.5s 不透明度淡入盖住上一段；原声在同一窗口里交叉淡化；过短的段会自动退回硬切">
            叠化：重叠 0.5s 淡化
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
            <!-- ⚠️ 标签只留两字（2026-10-15 用户口径：「整合为一行」+「设为」删去）：
                 完整说法在 title 里。写成「设为入点/设为出点」时五颗按钮在 266px 里会折成两行。 -->
            <button class="mini-btn" title="把「开始」设为当前播放时间" @click="setInPoint">入点</button>
            <button class="mini-btn" title="把「结束」设为当前播放时间" @click="setOutPoint">出点</button>
            <button class="mini-btn" title="预览跳到这个块的开始" @click="seekToStart">定位</button>
            <button v-if="canSplit" class="mini-btn" title="从当前播放时间拆成两块" @click="splitSelected">拆分</button>
            <!-- 「复制」按钮**已删**（2026-10-15 用户口径「不需要复制」）：只被它用的 duplicateSelected 一并删了。
                 要复制一条事件的做法仍是"新建一条再改时间"（新建同样打 manual 标记、不被后处理改写）。 -->
          </div>
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

            <!-- ⚠️⚠️ **「系统做得对吗」整块已删**（2026-10-15 用户口径）。
                 它把 `polish.toml` 的 `[[decisions]]` 原样铺在分段属性里 —— 三行说明
                 （"首段补了一条 0.5s 的黑场淡入" / "往后改/删都归你" /
                 "后处理每次重跑都会重算这些理由"）全是**给开发者看的**，
                 而且**只读**（改它没有任何效果）。用户的原话是这一长串要删掉。
                 ⚠️ `[[decisions]]` 这个**数据**还在（`process` 仍写、仍是排障凭据），
                     删的只是**界面上的展示**。要再看就走 `polish.toml` 或日志。
                 ⚠️ 别把它挪到别处再挂出来 —— 用户的判断是"这类信息不该出现在编辑器里"。 -->
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
              <!-- ⚠️⚠️ **`element` 不显示词汇表那句「元素（文字 / 贴纸）」**（2026-10-15 用户口径：
                   「因为元素(原文字+贴纸)已分离，请查找元素相关文案进行修改」）。
                   轨道分家之后，"元素"这个**类别**在界面上已经不存在了 —— 用户看到的
                   只有「文字」和「贴纸」两类。再显示"元素（文字 / 贴纸）"，等于把
                   **已经合并掉的两个东西又并成一个名字**摆在他面前。
                   ⇒ element 事件按它**真实的 `kind`** 显示「文字」或「贴纸」（认不出就照实说）。
                   ⚠️ 别改 `spec/effects.json` 的 `label` —— 那是**词汇表/文档**用的
                      （`docs/polish-events-reference.md` 等按它生成），这里只改**界面**口径。 -->
              <template v-if="selItem.type === 'element'">
                <b>{{ elementKindOf(selItem) === 'text' ? '文字'
                     : elementKindOf(selItem) === 'sticker' ? '贴纸'
                     : '认不出的类型' }}</b>
                <!-- ⚠️⚠️ **这里只回显 `kind`，不再写 `element · <kind>`**（2026-10-15）。
                     ① `element` 是**磁盘上的载体名**（两条泳道共用 `type:"element"`），不是界面类别名 ——
                        把它摆在「文字 / 贴纸」后面，等于又用一个合并名把刚分开的两样捆回去
                        （与上面那条「不显示词汇表那句」同一条口径）。
                     ② mono 位**保留**：与其它类型「中文名 + 原始值」的形状一致（如「运镜 camera」），
                        只是元素这里的原始值是 `kind` —— `element` 对两种都成立、没有信息量。
                     ⚠️ 判据 `scripts/check-editor-lane-split.mjs` ⑥ 钉着这一行。 -->
                <span class="muted small mono">{{ selItem.kind || '(空)' }}</span>
              </template>
              <template v-else>
                <b>{{ TYPE_LABELS[selItem.type] || selItem.type }}</b>
                <span class="muted small mono">{{ selItem.type }}</span>
              </template>
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
              <!-- 荷兰角（地平线倾斜）：`tilt` 一直是**活的**字段（出片模板 `cam.tilt` 与转译器
                   `dutchFrames` 都在消费它），但此前是靠"自动派生"冒出来的**裸控件** ——
                   没有标签、没有区间提示，用户不知道那几个数字框是干什么的。
                   ⚠️ 旧注释一度说"转译器不读它"，**已过期**；别再据此把它藏掉。
                   ⚠️ 单位是**度**，正=顺时针倾斜；0 = 水平。 -->
              <label>地平线倾斜（{{ Number(selItem.tilt ?? 0) }}°）</label>
              <div class="pos-row">
                <input class="pos-range" type="range" min="-15" max="15" step="0.5"
                       :value="Number(selItem.tilt ?? 0)"
                       title="荷兰角：负=逆时针，正=顺时针；0 = 水平"
                       @input="selItem.tilt = Number($event.target.value)">
                <input class="pos-num" type="number" min="-15" max="15" step="0.5"
                       :value="Number(selItem.tilt ?? 0)"
                       @change="selItem.tilt = Number($event.target.value)">
              </div>
              <!-- 其余专用块没覆盖的字段（新增字段时它们自动出现，不用改这里 —— 见 `autoFields` 的口径）。
                   ⚠️ skip 表要与上面手写的控件逐一对应，否则那个字段**出两遍**（判据 ㉜）。 -->
              <template v-for="f in autoFields('camera', ['cut', 'speed', 'zoom', 'handheld', 'anchor', 'anchor_to', 'tilt'])" :key="f.name">
                <FieldControl :ev="selItem" :f="f" />
              </template>
            </template>

            <!-- 贴纸：行递进三段（包 → 分类 → 贴纸），见 polish/StickerPicker.vue -->
            <!-- 统一视觉元素：kind 分派 text / sticker -->
            <template v-else-if="selItem.type === 'element'">
              <!-- **逐块手动落点**：点「改落点」→ 在预览上**拖动** → 按「保存落点」才生效（草稿态）。 -->
              <div class="row mt-4 gap-6">
                <button v-if="!manualPick" class="mini-btn flex-1 jc-center"
                        title="在预览上拖动这一块的位置（草稿态，保存后才落盘）" @click="startManualPick">改落点</button>
                <div v-else class="mono small">{{ manualDraftPct }}</div>
              </div>
              <div v-if="manualPick" class="row mt-4 gap-6">
                <button class="mini-btn primary" title="把这一块的中心定在这个位置（写入 manual_pos）"
                        @click="saveManualPos">保存落点</button>
                <button class="mini-btn" @click="cancelManualPick">取消</button>
              </div>
              <div v-else-if="evManualPos" class="row mt-4 gap-6">
                <button class="mini-btn" title="清掉手动落点，交回自动（安全区算法）" @click="clearManualPos">恢复自动</button>
              </div>
              <div v-if="manualPick" class="row mt-4 gap-6">
                <button class="mini-btn primary" title="把这一块的中心定在这个位置（写入 manual_pos）"
                        @click="saveManualPos">保存落点</button>
                <button class="mini-btn" @click="cancelManualPick">取消</button>
              </div>
              <div v-else-if="evManualPos" class="row mt-4 gap-6">
                <button class="mini-btn" title="清掉手动落点，交回自动（安全区算法）" @click="clearManualPos">恢复自动</button>
              </div>
              <!-- ---- ⚠️⚠️ **「元素种类」下拉已删除**（2026-10-14）----
                   用户口径：「**前端编辑器也要分离**」。

                   那个下拉存在的唯一理由是"元素轨同时装两种东西"——
                   用户加完还得在这儿**再声明一次**它是什么。现在**轨道已经分开了**
                   （「文字」轨 / 「贴纸」轨），而泳道归属**就是**那个声明
                   ⇒ 再问一遍是多余，而且**允许把一条贴纸改成文字**会让它
                   悄悄换到另一条轨上（用户没有要求过这种能力）。

                   ⚠️ `setElementKind` **保留**（下面还有 `addElementAt` 在用，
                      而且判据 `check-*` 也钉着它的载荷切换口径）—— 只是**面板里不再暴露**。
                   ⚠️ kind 认不出时（`⚠` 那一支）**照实显示**，别默认成文字。 -->
              <template v-if="elementKindOf(selItem) === 'unknown'">
                <!-- ⚠️ 标签**不再叫「元素种类」**（2026-10-15）：轨道已经分家
                     （文字轨 / 贴纸轨），"种类"由**轨道**决定、面板里根本没有这个选择器了。
                     这一支是**数据完好性提示**（kind 认不出 ⇒ 不知道该按文字还是贴纸渲染），
                     叫回「元素种类」会让用户去找一个不存在的下拉。 -->
                <label>识别不了的类型</label>
                <div class="muted small">⚠ 认不出的 kind：{{ selItem.kind || '(空)' }}（按原样保留）</div>
              </template>

              <!-- 文字支：内容 + 样式。**和贴纸一样只有"是什么"，没有"摆哪"**——
                   ⚠️ 别再退回"纵向位置"下拉：位置一律由后处理按安全落点算
                   （与贴纸那句「位置 x/y 由后处理算」同一条口径），
                   下同的那句 hint 就是给用户看的唯一说明。 -->
              <template v-if="elementKindOf(selItem) === 'text'">
                <label>文字内容</label>
                <input :value="selItem.text || ''" placeholder="要显示的字"
                       @input="selItem.text = $event.target.value">
                <!-- ============ 颜色（命名预设，2026-10-13）============
                     ⚠️ **这不是样式档位那套已否决的四档**（那个是 `selItem.style`，
                        见下面那段注释）—— 这是**颜色**这一个维度，与 `style_anim`/
                        `effect_anim` 正交，所以它有自己的槽位。

                     ⚠️ **空 = 不设**（走 `style` 档位的默认色），必须排第一、且文案与
                        动效栏的「不设」/样式栏的「清除」都不冲突 —— 这里用「默认色」，
                        因为它表达的是"不指定颜色"而不是"清掉某种效果"。

                     ⚠️ **选项从 `ELEMENT_COLORS` 来**（唯一一份前端清单），而那份清单
                        的权威是 Rust `placement.rs::ElementColor`，由
                        `scripts/check-element-color.mjs` 钉着四处同值。
                        ⚠️ 写死在这里的话，加一档时要改五处、而漏掉哪一处都不报错。 -->
                <label>颜色</label>
                <select :value="selItem.color || ''" @change="selItem.color = $event.target.value || undefined">
                  <option value="">默认色</option>
                  <option v-for="c in ELEMENT_COLORS" :key="c.value" :value="c.value">{{ c.label }}</option>
                </select>
                <div v-if="selItem.color && !ELEMENT_COLOR_VALUES.includes(selItem.color)" class="hint">
                  这个颜色名不认识，成片里会用默认色。
                </div>
                <!-- ============ 两个**独立可叠加**的设置 ============
                     用户口径（2026-10-13）：「两个设置，有各自的细分，是可叠加的」。
                     即「竖排 **+** 抖动」两个都要成立，不是二选一。

                     ⚠️⚠️ **不是"三级互斥选择器"** —— 那一版（类型→种类→细分，写同一个
                        `anim`）**做错过并回退过**：它选样式会把动效清掉。这里两条
                        下拉**各写各的字段**（`style_anim` / `effect_anim`）。

                     ⚠⚠ **没有 `both` 了**（2026-10-14 三轴收敛）—— 每一档**只属一条轴**，
                        三条轴的档名集合**两两不相交**。下拉里只列**本轴自己**的档，
                        不再有“同一个名字两栏都出现”。（旧名单 `scatter_shake` /
                        `slam_squash` / `hop_*` / `pop_perchar` / `arc_*` 都已被删或合并。）
                        ⚠ 旧句子里说“只放一边会让用户以为另一半不存在”——
                        那句话的前提（一档真的给两边参数）已经没了，现在反过来：
                        **一档出现在两栏里就是错的**。
                     ⚠️ **认不出的档名必须仍然显示出来**（下面那两条 `v-if`）：
                        它若被悄悄归成"不动效"，用户改一下别的字段就把那个错档名
                        **永久抹掉**了 —— 而报告里的 CLAMP 已经告诉过他这一条不会动，
                        得让他看得见是哪一条。

                     ⚠️⚠️ **`selItem.style`（字幕样式，hook/chapter/comment/count）
                        那个下拉已经删掉**，不是漏了。用户口径：那套四档**早已被否决**
                        （2026-10-13），现在只有「样式（形态）× 动效」这一对槽位。
                        **别把它加回来** —— 它被否决过一次，又被人看见挂在面板上，
                        等于"把否定的方案留在上面"。底下 `TEXT_STYLES` 已一并删除；
                        ⚠️ `Event.style` **这个字段还在**（字幕层仍用它），
                        只是不再由这个面板提供入口。 -->
                <label>排版</label>
                <select :value="styleKind" @change="setKindOnAxis('style', $event.target.value)"
                  title="管字**排成什么形状**（歪斜、竖排、逐字斜扫）。不随时间变。">
                  <option v-for="k in kindsOnAxis('style')" :key="k.kind" :value="k.kind">{{ kindLabelOn('style', k) }}</option>
                  <option v-if="styleKind === ''" value="">未知档（{{ selItem.style_anim }}）</option>
                </select>
                <!-- 细分：认不出档名时 `styleVariants` 是空数组 ⇒ 这个下拉整条不渲染 -->
                <template v-if="styleVariants.length > 1">
                  <select :value="selItem.style_anim || ''" @change="setVariantOnAxis('style', $event.target.value)">
                    <option v-for="a in styleVariants" :key="a.value" :value="a.value">{{ a.label }}</option>
                  </select>
                </template>
                <!-- ---- 排版角度（2026-10-14，用户口径：倾斜与斜扫要能调角度）----
                     ⚠️ **只在 `tilt` / `arc` 两档出现**（`showAngle`）—— 别的排版档
                        没有旋转语义，显示了用户会以为它能调（调了不生效）。
                     ⚠️ 区间**必须与转译器的 `LAYOUT_ANGLE_MIN/MAX` 同值**，
                        由 `scripts/check-anim-layout-angle.mjs` 逐值钉着。 -->
                <template v-if="showAngle">
                  <label>角度</label>
                  <div class="angle-row">
                    <input type="range" :min="LAYOUT_ANGLE_MIN" :max="LAYOUT_ANGLE_MAX" step="0.5"
                      :value="layoutAngle" @input="setLayoutAngle($event.target.value)"
                      title="倾斜/斜扫的角度（度）。负号 = 反方向。">
                    <span class="angle-val">{{ layoutAngle }}°</span>
                    <!-- ⚠️ 「默认」只在**显式设过**时才可点：没设过时它本来就是默认值，
                         按钮看着能动却什么都不改变，用户会以为坏了。
                         ⚠️ 它写的是 `null`（键不存在），**不是 0** —— 0 是一个合法的
                         用户选择（"我要它正着"），与"没填"是两回事。 -->
                    <button v-if="layoutAngleSet" type="button" class="angle-reset"
                      @click="resetLayoutAngle()" title="去掉角度设置，回到该档的默认角度">默认</button>
                  </div>
                </template>
                <div v-if="selItem.style_anim && !ANIM_KNOWN.includes(selItem.style_anim)" class="hint">
                  这个档名不认识。
                </div>

                <!-- 过渡轴**只有一个下拉框**（2026-10-15 用户口径，报过不止一次）。

                     用户原话：「过渡只留**弹出**和**淡入淡出**」「过渡：过渡：弹出，不觉得很蠢吗」。

                     病根：这里原先照抄了**排版轴**那套「种类 + 细分」两级结构
                     （种类框走 `kindsOnAxis('transition')`、细分框走 `transitionVariants`）。
                     而两级是给排版轴设计的 —— 那里一个种类下辖多档（倾斜 → 标准 / 小角度…），
                     种类栏说"这一类是什么"、细分栏说"哪一支"，**两级各有信息**。

                     过渡轴只有 `pop` / `fade` **两档、没有细分维度** ⇒ 硬套两级之后：
                       · 种类框：属于这条轴的只有 `transition` 这个**种类本身**与 `none`（不设）
                         ⇒ 它只能把自己重复一遍，显示成「过渡」；
                       · 细分框：`transitionVariants` 恰好 `length > 1` ⇒ 渲染出来显示「弹出」。
                     于是面板上就是「过渡 / 弹出」—— **一个框什么都没选，另一个框才是真正的值**。

                     ⇒ 过渡轴直接**一个下拉**，选项就是两档本身。
                     ⚠️ 值仍然写 `transition_anim`，**取值一个字都不变**（`pop` / `fade` /
                        空串 = 不设）—— 这是**纯界面重排**，老工程产物逐字节不变（R4）。
                     ⚠️ 别再把 `kindsOnAxis('transition')` / `transitionVariants` 用回来 ——
                        那两个计算属性还在（排版/动效支仍在用），但**过渡支不许再走两级**。 -->
                <label>过渡</label>
                <select :value="selItem.transition_anim || ''" @change="setVariantOnAxis('transition', $event.target.value)"
                  title="管字**怎么进来、怎么出去**（入场/出场效果）。与动效可以同时设。">
                  <option v-for="a in TRANSITION_OPTIONS" :key="a.value" :value="a.value">{{ a.label }}</option>
                  <!-- ⚠️⚠️ 这一项**不是第三个选项**，是"磁盘上的值认不出/为空"时的**兜底显示**
                       （2026-10-15 用户口径：过渡只留 `弹出` + `淡入淡出` 两项）。

                       为什么必须有它：老工程与后处理生成的事件**大多没有 `transition_anim`**
                       （空串），而 `<select>` 的 `:value` 匹配不上任何一项时**浏览器会显示第一项** ——
                       表现就是"这条我从来没设过，面板却写着弹出"。那是撒谎，
                       用户会以为自己设过（本仓对这类"静默伪装"零容忍）。
                       ⇒ 兜底项照实说"没设过"：**只在磁盘值认不出/为空时出现**，
                          用户一旦选任一项就固化成 `pop` / `fade`，它自己消失。

                       ⚠️ 文案**不许写"未知档"**：空串是**正常状态**（没设过），不是坏数据 ——
                          写成"未知档"会让用户以为工程坏了。真正的**认不出的档名**
                          （既不是空、也不在 `ANIM_KNOWN` 里）由下面那行 `.hint` 单独报。 -->
                  <option v-if="transitionKind === ''" value="">
                    {{ selItem.transition_anim ? '未知档（' + selItem.transition_anim + '）' : '未设置' }}
                  </option>
                </select>
                <div v-if="selItem.transition_anim && !ANIM_KNOWN.includes(selItem.transition_anim)" class="hint">
                  这个档名不认识。
                </div>

                <label>动效</label>
                <select :value="effectKind" @change="setKindOnAxis('effect', $event.target.value)"
                  title="管字**在场的时候怎么动**（持续型）。与过渡可以同时设。">
                  <option v-for="k in kindsOnAxis('effect')" :key="k.kind" :value="k.kind">{{ kindLabelOn('effect', k) }}</option>
                  <option v-if="effectKind === ''" value="">未知档（{{ selItem.effect_anim }}）</option>
                </select>
                <template v-if="effectVariants.length > 1">
                  <select :value="selItem.effect_anim || ''" @change="setVariantOnAxis('effect', $event.target.value)">
                    <option v-for="a in effectVariants" :key="a.value" :value="a.value">{{ a.label }}</option>
                  </select>
                </template>
                <div v-if="selItem.effect_anim && !ANIM_KNOWN.includes(selItem.effect_anim)" class="hint">
                  这个档名不认识，成片里不会动。
                </div>
                <!-- ⚠️ 三轴里只剩「排版 / 过渡 / 动效」，**没有"两栏都有"这回事了** ——
                     `kindLabelOn` 的后缀与 `setKindOnAxis` 的同步都随之删除。
                     用户报的「样式和动效内容过于重复」就是那 11 档 `both` 造成的。 -->
              </template>

              <!-- 贴纸支：复用已有的贴纸选择器。
                   ⚠️ 判据从"`kind === 'text'` 的 else"改成**显式认 `sticker`**
                   —— 上面那个 `unknown` 支已经先把认不出的拦走了，
                   这里再写 `v-else` 会把它当成贴纸（把坏数据伪装成正常元素）。 -->
              <template v-else-if="elementKindOf(selItem) === 'sticker'">
                <StickerPicker :lib="stickerLib" :lib-error="stickerLibError" :pid="pid" :dir="dir"
                               :pack="selItem.text || ''" :tag="selItem.sticker_tag || ''"
                               :name="selItem.name || ''" :source="selItem.source || ''"
                               @pick="onStickerPick" @retry-lib="loadPickers" />
              </template>
              <!-- ============ 位置（X / Y **滑杆**）============
                   ⚠️⚠️ **只有手动改过才写盘**（2026-10-15）：留空 = 不设 = 由后处理按安全落点算
                   （贴纸 Step 2b / 文字 Step 1d3）。这是**唯一**能手动钉住单个元素位置的地方，
                   而"不设"必须保持默认 —— 一写值就把该元素从自动落点里摘出来。
                   ⚠️ 文案用「自动 / 手动」，别用「清除」：它表达的是"不指定"而不是"清掉某种效果"。
                   ⚠️ 两个载体的 `x`/`y` 类型不同（`Element` 是 `Option<String>`、`Sticker` 是
                      `String`）—— 这里只在 element 支出现，写值一律走 `setElementPos`。 -->
              <div class="row spread">
                <label class="pos-lbl">位置</label>
                <button v-if="hasElementPos(selItem)" class="mini-btn"
                        title="改回自动落点（后处理按人物/字幕/弹幕避让算）"
                        @click="clearElementPos()">自动</button>
                <span v-else class="muted small">自动落点</span>
              </div>
              <div class="pos-row">
                <span class="pos-tag">X</span>
                <input class="pos-range" type="range" min="0" max="100" step="0.5"
                       :value="posSlider(selItem.x)"
                       title="横向 0~100%（0 贴左 / 100 贴右）"
                       @input="setElementPos('x', $event.target.value)">
                <input class="pos-num" type="number" min="0" max="100" step="1"
                       :value="pctOf(selItem.x)" placeholder="—"
                       title="也可以直接填数字（0~100）"
                       @change="setElementPos('x', $event.target.value)">
              </div>
              <div class="pos-row">
                <span class="pos-tag">Y</span>
                <input class="pos-range" type="range" min="0" max="100" step="0.5"
                       :value="posSlider(selItem.y)"
                       title="纵向 0~100%（0 贴上 / 100 贴下）"
                       @input="setElementPos('y', $event.target.value)">
                <input class="pos-num" type="number" min="0" max="100" step="1"
                       :value="pctOf(selItem.y)" placeholder="—"
                       title="也可以直接填数字（0~100）"
                       @change="setElementPos('y', $event.target.value)">
              </div>
              <div class="hint">拖动即钉住这一条，不再自动避让。点「自动」交回给后处理。</div>

              <!-- 词汇表里专用块没覆盖的字段（文字/贴纸各自还有几个）。
                   ⚠️⚠️ **skip 表必须与上面专用块真正渲染的字段逐一对应**（2026-10-15 用户报
                   「一堆冗余参数」）：文字支在上面已经渲染了 `style_anim` / `transition_anim` /
                   `effect_anim` / `layout_angle_deg`（排版/过渡/动效三轴 + 角度），
                   而这张表**漏了这四个** ⇒ `autoFields` 把它们**又渲染一遍**，
                   面板上是「排版/过渡/动效」各出两遍。
                   ⚠️ 判据：`check-layout-invariants.mjs` ㉜ 会**算**这张表与专用块的差集，
                      漏一个就红（别再靠人眼对）。 -->
              <template v-for="f in autoFields(selItem.type, ELEMENT_DEDICATED)" :key="f.name">
                <FieldControl :ev="selItem" :f="f" />
              </template>
            </template>

            <template v-else-if="selItem.type === 'sticker'">
              <!-- **逐块手动落点**：点了它就在预览上点一下，这一块的**中心**落到那儿；手动之后自动落点不再影响它。 -->
              <div class="row mt-4 gap-6">
                <button class="mini-btn" title="在预览上点一下，把这一块的中心放到那儿"
                        @click="startManualPick">改落点</button>
                <button v-if="evManualPos" class="mini-btn" title="清掉手动落点，交回自动（安全区算法）"
                        @click="clearManualPos">恢复自动</button>
                <span v-if="manualPick" class="muted small">在预览上点一下…</span>
              </div>
              <StickerPicker :lib="stickerLib" :lib-error="stickerLibError" :pid="pid" :dir="dir"
                             :pack="selItem.keyword || ''" :tag="selItem.sticker_tag || ''"
                             :name="selItem.name || ''" :source="selItem.source || ''"
                             @pick="onStickerPick" @retry-lib="loadPickers" />
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
                <!-- 渐变角度：**只对线性有意义**（径向没有方向）。
                     它此前是"自动派生"冒出来的裸控件（没有标签、没有条件），
                     与 `形状` 的关系看不出来 —— 现在是它的下一行、并只在 linear 时出现。
                     ⚠️ 单位口径：界面 0=向上、顺时针（CSS 约定）；转译器按 `base = css − 90` 换算，
                        别在这里自己减 90（那会减两遍）。 -->
                <template v-if="selItem.shape !== 'radial'">
                  <label>角度</label>
                  <div class="pos-row">
                    <input class="pos-range" type="range" min="0" max="360" step="1"
                           :value="Number(selItem.angle ?? 135)"
                           title="渐变方向：0=向上，顺时针"
                           @input="selItem.angle = Number($event.target.value)">
                    <input class="pos-num" type="number" min="0" max="360" step="1"
                           :value="Number(selItem.angle ?? 135)"
                           @change="selItem.angle = Number($event.target.value)">
                  </div>
                </template>
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
              <template v-for="f in autoFields('overlay', ['overlay_type', 'shape', 'color', 'colors', 'opacity', 'angle'])" :key="f.name">
                <FieldControl :ev="selItem" :f="f" />
              </template>
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

            <!-- ⚠️⚠️ **兜底块**：上面每个专用块都只渲染它自己那几个字段，
                 而「这个类型在词汇表里还有哪些字段没人渲染」**只有这里知道**。
                 删掉它 = 那些字段从界面上**整类消失**（用户看不见、也改不了），
                 而且**不报错** —— 判据 ㉛ ② 钉着这一块。
                 ⚠️ 只传 type、**不传 skip**：专用块已渲染的字段由 `DEDICATED_FIELDS`
                    与各自的 skip 表挡掉（两者的并集 = 面板真正手写的那些）。 -->
            <!-- ⚠️⚠️⚠️ **兜底块只许给"没有专用支"的类型兜底**（2026-10-15 二次修）

                 它存在的**唯一理由**是：新加的字段类型若还没有专用块，也不至于在面板上消失。
                 而**有任何专用支的类型**，上面那支已经把它的字段渲染过了 —— 再跑一遍 `autoFields`
                 就是**每个字段出两遍**。

                 ⚠️ 第一版只排除了 `element`（那是我当时在改的那个类型），于是**别的类型照旧重复**。
                    用户当场贴出运镜面板：手写的「方式/推进速度/景别/手持摇摆/聚焦/移到」
                    下面又原样列出 `zoom 中 / anchor liver / anchor_to / cut / handheld / speed / tilt` ——
                    **每个都是一遍**。实测同病的还有 `overlay`（5 个字段）。
                    ⚠️ 教训：**"这个类型有专用支吗"要按全集判，不是按手头这一个判** ——
                       当时我要是问一句"还有谁有专用支"，就不会只修 element。

                 ⚠️ 判别依据 = `hasDedicatedBranch`（有专用支的类型**必须**与模板里的
                    `selItem.type === '…'` 分支逐一对应，判据 ㉞ 会算差集）。
                 ⚠️ 别退回去写成 `!== 'element' && !== 'camera' && …` 的硬编码长串：
                    下次加一个专用支的人**必然忘记同步它**，而症状是静默的重复渲染。 -->
            <template v-if="!hasDedicatedBranch(selItem.type)"
                      v-for="f in autoFields(selItem.type)" :key="'auto-' + f.name">
              <FieldControl :ev="selItem" :f="f" />
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
        <div v-for="lane in lanes" :key="'label-' + laneKey(lane)" class="lane-label">
          <!-- 眼睛 = 这一类渲不渲染（**事件一个字都不动**，点亮即恢复）。判据由 Rust 注入
               （`_lanes`，见 PolishView 的 laneEnabled）：竖屏素材/画布下运镜与视觉特效默认灭。
               ⚠️ 「分段 / 屏蔽」是结构轨（分段是容器、屏蔽是剪切标记），没有"整类不渲染"这种语义，
               所以它没有眼睛 —— 别给它补一个假装有作用的按钮。 -->
          <!-- ⚠️ 眼睛的键是**轨道名**（`lane.lane`），不是界面 id。
               「文字」与「贴纸」磁盘上是同一条轨（都写 `sticker`）⇒ 两枚眼睛**读同一个开关**：
               点任一枚两枚一起变。这是**对的**（"文字不渲染"与"贴纸不渲染"本就是同一件事，
               落盘也只有一个 `[stage.lanes].sticker`）；别为"两条能分别开关"去造第二个落盘键。 -->
          <button v-if="lane.lane" class="lane-eye" :class="{ off: !laneEnabled(lane.lane) }"
                  :title="laneEyeTitle(lane)" @click.stop="emit('toggle-lane', lane.lane)">
            <Icon :name="laneEnabled(lane.lane) ? 'eye' : 'eye-off'" :size="12" />
          </button>
          <!-- ⚠️⚠️ **「文字」轨也要一枚真的眼睛**（2026-10-15 用户口径）。
               之前这里是一枚**说明性占位**（`⇄` / `type` 图标，不可点），理由写的是
               "两条泳道磁盘上是同一条轨 ⇒ 两枚眼睛管同一个开关，点哪枚另一枚都变"。
               **那个理由站不住**：`laneEnabled` 的键就是轨道名 `lane.lane`（两条都是
               `'sticker'`），共用同一个开关**恰恰意味着两枚眼睛读数一致、谁点都生效** ——
               那正是用户要的（"文字不渲染"和"贴纸不渲染"本来就是同一件事）。
               占位符带来的真实体验是"文字轨前面有个坏掉的符号、还点不动"。
               ⇒ 与其余轨道走**同一条** `<button class="lane-eye">` 分支。
               ⚠️ 别再加回 `sharedEye` 占位支（那个字段也已随之删除）。 -->
          <span v-else class="lane-eye lane-eye-none"
                title="分段 / 屏蔽是结构轨：分段是容器、屏蔽是剪切标记，没有「整类不渲染」这种语义">·</span>
          <!-- 轨道名可点 = **这一条轨道的设置**（与时间轴上的块共用同一个属性面板） -->
          <button class="lane-name" :class="{ on: laneOpen === laneTypeOf(lane) }"
                  :title="lane.label + '：点这里改这条轨道的设置'"
                  @click.stop="emit('lane-open', laneTypeOf(lane))">{{ lane.label }}</button>
          <div class="lane-adds">
            <button v-if="lane.type !== 'segments'" class="add-lane"
                    :title="lane.type === 'fx' ? '在播放头处添加视觉特效（先选细分类型）' : '在播放头处添加'"
                    @click.stop="onLaneAdd(laneKey(lane), $event)"><Icon name="plus" :size="11" /></button>
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
        <!-- 时间（标尺）：**没有轨道设置面板**（点名字没得开），但热度曲线有自己那枚眼睛 ——
             与其他轨道同一形状、同一位置，**默认常显**（用户口径 2026-10）。 -->
        <div class="lane-label ruler-label" :class="{ 'heat-on': heatOn }">
          <button class="lane-eye" :class="{ off: !heatOn }" :title="heatEyeTitle" @click.stop="toggleHeat">
            <Icon :name="heatOn ? 'eye' : 'eye-off'" :size="12" />
          </button>
          <span class="ruler-name">时间</span>
          <span v-if="heatOn" class="ruler-readout" :title="heatTip">{{ heatReadout }}</span>
        </div>
    </div>
    <div ref="tracksRef" class="timeline-tracks">
      <div v-for="lane in lanes" :key="'track-' + laneKey(lane)" class="lane">
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
            :key="laneKey(lane) + '-' + i"
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
      <div class="lane ruler-lane" :class="{ 'heat-on': heatOn }">
        <div ref="rulerTrackRef" class="lane-track ruler-track" :class="{ 'heat-on': heatOn }"
             :style="{ width: trackWidth + 'px' }"
             @pointerdown="onRulerPointerDown" @pointermove="onRulerHover" @pointerleave="onRulerLeave">
          <!-- 热度曲线：**叠在标尺带里**（不是第 8 条泳道）。
               ⚠️ `pointer-events:none` 不能去掉 —— 标尺的 pointerdown 是 seek / 拖播放头，
                  曲线挡一下就是"点不中"（同 `.wave-layer` 那条纪律）。
               ⚠️ 纵轴是**相对本片最热那一刻**（见 `heatPath` 的注释），读数因此也说"相对"。 -->
          <svg v-if="heatOn && heatPath" class="heat-layer" :width="trackWidth" :height="HEAT_H">
            <path class="heat-halo" :d="heatPath" />
            <path class="heat-core" :d="heatPath" />
          </svg>
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
import { ref, computed, onMounted, onUnmounted, watch, nextTick, h, defineComponent } from 'vue'
import { confirmDialog } from '../stores/dialog'
import { addClient } from '../stores/log'
import { toast } from '../stores/toast'
import Icon from './Icon.vue'
import StickerPicker from './polish/StickerPicker.vue'
import SfxPicker from './polish/SfxPicker.vue'
import { TIMELINE_COLORS, isEditableKeyEvent, clipMediaUrl } from '../utils'
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
// 热度曲线：**算法只有一份**（`webui/src/polish/heat.js`：按字 / 按条 / 自基准归一 / 权重都在那里），
// 本组件只负责「取数 → 冻成快照 → 画在标尺带里 → 给个读数」。别再在这里算第二遍。
import { heatSeries, heatBucketAt, heatDisplay } from '../polish/heat'

const props = defineProps({
  /** 预览里**刚点的那一下**（`{x, y, seq}`；`seq` 每次 +1 ⇒ 用来触发"写入选中事件"）。
   *  归页面管（它拿得到预览的 `update:point`），本组件只**消费**它。 */
  pointResult: { type: Object, default: null },
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
const emit = defineEmits(['point-pick', 'seek', 'toggle', 'preview', 'saved', 'done', 'reprocess', 'retried', 'redo-segment', 'lane-open', 'toggle-lane'])
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
// ⚠️ **`decisions` ref 已删**（2026-10-15）：它只服务「系统做得对吗」那块，那块已下架。
//    数据本身照旧在 `polish.toml` 的 `[[decisions]]` 里（`process` 每趟重算覆盖）。
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

// ── 热度曲线（标尺带）──────────────────────────────────────────
/**
 * 曲线叠在**底部「时间」标尺带**里 —— **不是第 8 条泳道**（"不要新增泳道"是硬口径，
 * 见 `docs/webui-components.md` ①.1）。标尺带因此长高 `HEAT_H`(16px)，关掉曲线就缩回去。
 *
 * ⚠️ **算一次就冻住**：用户口径 2026-10「实时只是说视频的实时，按 `polish init` 时的数据计算即可，
 *    不做随编辑即时重算」。所以 `heatRaw` **只在 `loadData()` 里建一次** ——
 *    拖动字幕块**不该**让它变。想"跟着编辑变"就得把 `subtitles` 接进 computed，
 *    那正是这条口径明确不要的东西（也顺带省掉每次拖拽重算全片桶的开销）。
 * ⚠️ 口径（按字 / 按条 / 自基准归一 / 权重 / 桶宽）**只在 `polish/heat.js` 一份**，
 *    连带与 `process/punch.rs` 的对拍一起钉在 `scripts/check-heat-curve.mjs`。
 */
const heatRaw = ref(null)
/** 弹幕的 `start`（秒）。**只留这一个字段** —— 文本/颜色与本曲线无关。
 *  来源 = `data/danmaku.json`（`polish init`/`parse` 解析出来的那份，与 `process` 的爆点判据同源）。 */
const heatDanmaku = ref([])
/** 热度带高度。用户口径 2026-10：先是「参考响度曲线，小点没关系」（初值 12），
 *  随后要「起伏大一点」⇒ **12 → 16**（振幅的像素数直接翻三成）。
 *  ⚠️ 改这个数要**三处一起改**：这里、三条 `.heat-on` 高度（= 20 + `HEAT_H`）、刻度下移量。 */
const HEAT_H = 16
const HEAT_PAD = 1
/** 每列多少像素（列数 = 轨道宽度 ÷ 这个值）—— 与波形同一条做法，理由见 `wavePath`。 */
const HEAT_COL_STEP = 2
const HEAT_PREF_KEY = 'yekitrim.ui.heatCurve'
/** 默认**常显**（用户口径 2026-10：参考其他轨道那枚眼睛，默认开）。 */
function readHeatPref() {
  try { return localStorage.getItem(HEAT_PREF_KEY) !== '0' } catch { return true }
}
function saveHeatPref(on) {
  try { localStorage.setItem(HEAT_PREF_KEY, on ? '1' : '0') } catch {}
}
const heatOn = ref(readHeatPref())
/** 鼠标停在标尺上的时刻（`null` = 没停，读数跟播放头走）。 */
const heatHoverT = ref(null)
/** 读数取哪一桶：优先鼠标处，否则**播放头**（"视频的实时热度"就是这一档）。 */
const heatProbe = computed(() => heatBucketAt(
  heatRaw.value,
  heatHoverT.value == null ? props.currentTime : heatHoverT.value,
))
const heatPeak = computed(() => {
  const p = Number(heatRaw.value && heatRaw.value.peak)
  return Number.isFinite(p) && p > 0 ? p : 0
})
/**
 * 曲线路径（`M…L…`，未闭合、只描边）—— 与波形同一套做法：**按像素列取该列的峰值**，
 * 顶点位置永远跟着缩放走（柱状图缩小糊成一片、放大出缝）。
 *
 * ⚠️ 纵轴是**相对本片最热那一刻**（`heat / peak` ⇒ 峰值顶格）。理由：合成值要两路同时在峰值才
 * 到 1.0，用绝对刻度会让整条线常年压在带子下半截、看着像"一直不热"。读数因此也说"相对"
 * （`heatTip` 里写明 100% = 本片最热的那一桶）。
 */
const heatPath = computed(() => {
  const s = heatRaw.value
  if (!s || !heatOn.value) return ''
  const peak = heatPeak.value
  const win = Number(s.windowSecs) || 0
  if (peak <= 0 || win <= 0) return ''
  const sc = scale.value || 1
  // 只画到内容结束为止：`trackWidth` 有 600px 下限，片段很短时轨道比内容宽，那一段没有数据
  const width = Math.min(trackWidth.value, Math.ceil((duration.value || 0) * sc) + HEAT_COL_STEP)
  const yOf = (v) => (HEAT_H - HEAT_PAD) - Math.max(0, Math.min(1, v)) * (HEAT_H - HEAT_PAD * 2)
  const parts = []
  let firstY = null
  for (let x = 0; x <= width; x += HEAT_COL_STEP) {
    const i0 = Math.max(0, Math.floor(x / sc / win))
    if (i0 > s.count - 1) break
    let i1 = Math.floor((x + HEAT_COL_STEP) / sc / win)
    if (i1 < i0) i1 = i0
    if (i1 > s.count - 1) i1 = s.count - 1
    let v = 0
    for (let i = i0; i <= i1; i++) if (s.heat[i] > v) v = s.heat[i]
    // 纵轴 = 自基准归一 → **显示对比拉伸**（读数走同一个映射，见 heat.js 的 HEAT_DISPLAY_GAMMA）
    const y = yOf(heatDisplay(v / peak))
    if (!parts.length) {
      firstY = y
      parts.push('M' + x.toFixed(0) + ' ' + y.toFixed(2))
    } else {
      parts.push('L' + x.toFixed(0) + ' ' + y.toFixed(2))
    }
  }
  if (parts.length === 1) parts.push('L' + HEAT_COL_STEP + ' ' + firstY.toFixed(2))
  return parts.length > 1 ? parts.join('') : ''
})

function toggleHeat() {
  heatOn.value = !heatOn.value
  saveHeatPref(heatOn.value)
  if (!heatOn.value) heatHoverT.value = null
}
/** 标尺上的悬停 → 读数。⚠️ 别在这里 seek：seek 是 `pointerdown`（`onRulerPointerDown`）独占的事。 */
function onRulerHover(e) {
  if (!heatOn.value || !heatRaw.value) return
  const el = rulerTrackRef.value
  if (!el) return
  heatHoverT.value = (e.clientX - el.getBoundingClientRect().left) / (scale.value || 1)
}
function onRulerLeave() {
  heatHoverT.value = null
}
/** `m:ss`（读数里只放得下这一种） */
function heatClock(t) {
  const v = Math.max(0, Number(t) || 0)
  return Math.floor(v / 60) + ':' + String(Math.floor(v % 60)).padStart(2, '0')
}
/** 标签列里的那个数：**相对本片峰值**的百分比（与曲线同一映射 ⇒ 读数 = 曲线的高度）。 */
const heatReadout = computed(() => {
  if (!heatOn.value) return ''
  const b = heatProbe.value
  if (!b || heatPeak.value <= 0) return '—'
  return Math.round(heatDisplay(b.heat / heatPeak.value) * 100) + '%'
})
const heatTip = computed(() => {
  const s = heatRaw.value
  if (!s) return '热度曲线：这一段还没有解析数据（没有字幕也没有弹幕）'
  const base = '弹幕按条（×0.6）+ 字幕按字量（×0.4），各自按本片段最大值归一；按解析时的数据算一次，不随编辑变。'
  if (heatPeak.value <= 0) return '热度曲线：这一段没有字幕也没有弹幕 —— 算不出热度。' + base
  const b = heatProbe.value
  const where = heatHoverT.value == null ? '播放头' : '鼠标处'
  const noDm = s.hasDanmaku ? '' : '（本片段没有弹幕 ⇒ 只按字幕算）'
  return where + ' ' + heatClock(b.start) + '：热度 ' + Math.round(heatDisplay(b.heat / heatPeak.value) * 100)
    + '%（相对本片峰值；纵轴做过 x² 对比拉伸，100% = 最热的那一桶） · 弹幕 ' + b.dm + ' 条 · 字幕 '
    + b.sub.toFixed(1) + ' 字/秒 ' + noDm
    + '\n桶宽 ' + s.windowSecs + 's，共 ' + s.count + ' 桶。' + base
})
const heatEyeTitle = computed(() => (heatOn.value
  ? '热度曲线：当前显示（弹幕按条 + 字幕按字量）。点一下隐藏，标尺带跟着缩回 20px'
  : '热度曲线：当前隐藏。点一下显示'))

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
    hint: '片头最抓人的一段：允许大字 + 强音效 + 硬切运镜，但整段只放 1~2 个亮点。'
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

/** **39 档动效**（`docs/plan/text-styles-39-完成计划.md` §一·五，2026-10-13）。
 *
 *  # 两级结构：**种类（kind）→ 细分（variant）**（2026-10-13 改）
 *
 *  以前是**一个下拉 + `<optgroup>`**，40 项平铺，而且种类与细分挤在**同一个
 *  label 字符串**里（`'倾斜 · 标准'`）—— 用户报「还是这个」，说的是这件事。
 *  改成两级：先选**种类**（倾斜 / 抖动 / …），再选**这一类的细分**（标准 / 小角度 / 反方向）。
 *
 *  ⚠️ ⚠️ **写回 `selItem.anim` 的取值**与改造前**逐字相同**（`tilt_std` 还是 `tilt_std`）。
 *     这是**纯界面重排**，toml 键集与取值一个都不许变（R4：老工程产物逐字节不变）。
 *
 *  ⚠️ **`{ value: '...' }` 的字面形态必须保留** —— `check-anim-presets-parity.mjs`
 *     用正则从这里扒出全部档名，与转译器 `ANIM_PRESETS`（权威）逐档对拍。
 *     改成别的形状会让那条对拍**静默失效**（扒不到 ⇒ 两侧都空 ⇒ 全绿）。
 *
 *  ⚠️ 空串 = **不设**（不写那个键）。必须**排第一** ——「不选」是最常见的状态
 *  （老工程与后处理生成的事件都没有这个键）。
 *
 *  ⚠️ **这一项只在「动效」那一栏有意义**（"不额外加动效"）。**样式栏不列它** ——
 *     见 `kindsOnAxis` 的注释：形态那一栏的缺省是"不清空"（保留原档），
 *     不是"选一个叫'不动效'的形状"。 */
const ANIM_KINDS = [
  // ---- 不设 / 清除：单一状态，没有细分 ----
  //  ⚠️ 每一栏的"空"语义**不一样**，所以显示名也不一样（见 `kindLabelOn`）：
  //     排版栏 = 「默认」（回到没有任何排版变换）；过渡栏 / 动效栏 = 「不设」。
  //     ⚠️ 过渡栏 2026-10-15 之前叫「默认弹出」—— 那个叫法随它那套两级结构一起删了。
  //  ⚠️ 三栏的"空"落盘时**都写空串** —— 差别只在显示。
  {
    kind: 'none',
    label: '不设',
    axis: 'none',
    variants: [{ value: '', label: '不设' }]
  },

  // ===== 排版轴（用户口径：「形态改为排版」）=====
  //
  // ⚠️⚠️ **倾斜与斜扫的角度是可调的**（2026-10-14，用户明确要求）——
  //    档位本身**不带角度**，角度走**事件字段** `layout_angle_deg`。
  //    面板上给一个**角度滑杆**，只在选中 `tilt` / `arc` 时显示。
  //    改档位**不会**重置角度（两者正交）。
  //    ⚠️ 滑杆区间必须与转译器那对常量**同值**（`LAYOUT_ANGLE_MIN/MAX` 就在下面，
  //       那边是权威；`scripts/check-anim-layout-angle.mjs` 逐值钉着两端一致）。
  //       ⚠️ 不一致的症状：「前端能拉到 45、后端夹到 30」⇒ 用户看到 45、成片 30、**不报错**。
  {
    kind: 'tilt',
    label: '倾斜',
    axis: 'style',
    // ⚠️ 只有这一档与 `arc` 显示角度滑杆（见 `showAngle`）。
    angle: true,
    variants: [{ value: 'tilt', label: '标准' }]
  },
  {
    kind: 'arc',
    // ⚠️⚠️ **标签曾经写「弧形（扇形）」—— 那是个做不到的承诺**（2026-10-14 改）。
    //
    //    实测稳态（t=3.0，入场早已结束）各字角度：**-14° -14° -14° … -14°**
    //    —— 每个字**同一个角度**，也就是"整行统一倾斜"，**与 `tilt` 档观感分不出来**。
    //    而真弧该是 -14 / -10 / -6 / -2 / +2 / +6 / +10 / +14（各字不同）。
    //
    //    真因是**原语缺口**：`text_perchar_anim.rs:345` 的
    //      `rotation_deg: mix(rotation_from, rotation_to)`  —— **没有 `rank` 项**，
    //    所以"第 i 个字转 i 度"表达不出来（位移那两项 `spread_*` 才乘 `rank`，
    //    而它是线性混合 ⇒ 只能做单调的斜排，做不了抛物线的弧）。
    //    ⚠️ 预览与成片**两边一致地做不出**，所以这不是漂移，是能力缺口。
    //    需求与验收判据见 `docs/plan/dhampir-rotation-spread-request.md`。
    //
    //    ⇒ 在底座补上 `rotation_spread` 之前，标签只能说**它真做到的**：
    //      入场时逐字扫过、稳态是一行统一倾斜。**别写回"弧形"。**
    label: '斜扫（非真弧）',
    axis: 'style',
    angle: true,
    // ⚠️ 弧形只用角度的**绝对值**当幅度，**方向由档位定**（缺省向上弯）。
    //    所以这里没有"上拱/下垂"两支了 —— 想换方向就把角度填成负的。
    variants: [{ value: 'arc', label: '标准' }]
  },
  {
    kind: 'vtext',
    label: '竖排',
    axis: 'style',
    // ⚠️ 竖排是**轨级排版方向**（底座 `SubtitleStyle::vertical`），不是逐字变换。
    // ⚠️ 原来「细楷 / 粗体 / 逐字变小」是**三档**，其中两档曾被实测抓出
    //    "产物逐字节相同"。收敛成一条：用户清单里"竖排"就是一条，没有细/粗这一维。
    variants: [{ value: 'vtext', label: '标准' }]
  },

  // ===== 过渡轴（新独立出来的一轴，用户口径：「入场出场的效果」）=====
  //
  // ⚠️ 过渡与动效是**两个维度**，可以并存：
  //    过渡管"怎么进来、怎么出去"（一次性、有窗口），动效管"在场时怎么动"（持续型）。
  //    「淡入出场 + 全程抖动」是合法组合。
  {
    kind: 'transition',
    label: '过渡',
    axis: 'transition',
    variants: [
      // ⚠️ 「弹出」是**视觉默认**（用户口径：「默认为弹出」）—— 但落盘仍写**空串**，
      //    因为"没配"与"显式配了弹出"在产物里必须是两个状态（R4）。
      { value: 'pop', label: '弹出' },
      { value: 'fade', label: '淡入淡出' }
    ]
  },

  // ===== 动效轴（用户口径：「只保留抖动和逐字变化」，另加心跳）=====
  {
    kind: 'shake',
    label: '抖动',
    axis: 'effect',
    variants: [{ value: 'shake', label: '标准' }]
  },
  {
    // ⚠️ **把原来 C 组（逐字变换）+ D 组（逐字错峰）合并成一条**（用户口径：
    //    「合并逐字的效果作为一个动效，名为逐字变化」），下辖两支。
    // ⚠️ 两支**必须产出不同参数** —— 本仓有过"两个预设产物逐字节相同"的实测事故。
    //    由 `scripts/check-anim-presets-distinct.mjs` 钉着。
    kind: 'perchar',
    label: '逐字变化',
    axis: 'effect',
    variants: [
      { value: 'perchar_big', label: '逐字变大' },
      { value: 'perchar_small', label: '逐字变小' }
    ]
  },
  {
    // ⚠️ **新增档**（用户口径：「动效需要增加一个心跳，扑通扑通的心跳感」）。
    // ⚠️⚠️ **底座侧的双峰脉冲还没实现**（现有原语只有单次/正弦，见转译器里
    //    `heartbeat` 那段的注释）。当前观感 = 一次较缓的脉冲，不是真正的"扑通扑通"。
    //    这一档先落参数与入口，**底座能力到位后再补**。
    kind: 'heartbeat',
    label: '心跳',
    axis: 'effect',
    variants: [{ value: 'heartbeat', label: '标准' }]
  }
]

/** **过渡轴的选项** —— 就这两档（2026-10-15 用户口径，反复强调过）。

 *  ⚠️⚠️ **这一轴只有一个下拉框，盒子就是这两项**（见模板里过渡那段的注释）：
 *    「过渡 / 弹出」那种"一个框什么都没选、另一个框才是值"的两级结构已被删除。
 *
 *  ⚠️⚠️ **`value` 必须与 `ANIM_KINDS` 里 `transition` 那一支的 `variants[].value`
 *     逐字相同**（`pop` / `fade`）—— 它们就是 `transition_anim` 的落盘值。
 *     `scripts/check-anim-presets-parity.mjs` 用正则从源码里扒档名与转译器
 *     `ANIM_PRESETS`（权威）对拍；改形状会让那条对拍**静默失效**（扒不到 ⇒ 两侧都空 ⇒ 全绿）。
 *
 *  ⚠️ **空串 = 不设**，仍然是一个**合法且必要**的选项：
 *    老工程与后处理生成的事件都没有这个键，用户也必须能把设过的档**清掉**
 *  ⚠️⚠️ **只有两项**（2026-10-15 用户口径，两次强调：「过渡只留两个」`弹出` + `淡入淡出`）。
 *    下拉里**没有**「不设」那一项 —— 那一栏是三选一，不是"清掉某个效果"的地方。
 *
 *  ⚠️ 老工程/别处写进来的空串（= 没设过）**不在这份清单里**，由紧随其后的那条
 *     `v-if="transitionKind === ''"` 的兜底分支负责显示（文案见模板里那段注释：
 *     空值显示「未设置」、认不出的档名才显示那个带原值的提示）。
 *     它**只在磁盘上的值认不出/为空时**渲染，用来照实显示"这条没设过"，
 *     用户一选任一项就固化成 pop / fade。`kindLabelOn` 里过渡栏那条「默认弹出」
 *     说法已随两级结构一起删除。 */
const TRANSITION_OPTIONS = [
  { value: 'pop', label: '弹出' },
  { value: 'fade', label: '淡入淡出' }
]

/** **排版角度的允许区间**（2026-10-14）。
 *
 *  ⚠️⚠️ **权威在 `tools/polish-to-dhampir.mjs` 的 `LAYOUT_ANGLE_MIN/MAX`** ——
 *     这里是为了让前端滑杆有个 `min`/`max` 而**逐值抄了一份**（前端不能 import
 *     `tools/` 下的 ESM）。**两端同值由 `scripts/check-anim-layout-angle.mjs` 钉着**，
 *     照本仓 `check-camera-zoom-parity.mjs` 的先例（"三档默认倍率五处逐值一致"）。
 *
 *  ⚠️ 不一致的症状是**双向**的，且都不报错：
 *    * 前端区间比后端宽 ⇒ 用户能拉到 45、成片被夹到 30（他看到的和自己设的不一样）；
 *    * 前端比后端窄 ⇒ 用户根本调不到后端允许的值。
 */
const LAYOUT_ANGLE_MIN = -30
const LAYOUT_ANGLE_MAX = 30

const ANIM_GROUPS = ANIM_KINDS.map((k) => ({
  group: k.label,
  items: k.variants.map((v) => ({ value: v.value, label: v.label }))
}))

/** 全部档名（**不含**空串）—— 给"认不出的档名"兜底用。 */
const ANIM_KNOWN = ANIM_KINDS.flatMap((k) => k.variants.map((v) => v.value)).filter((v) => v !== '')

/** 档名 → 它属于哪个**种类**（选中一条时反查，让两级控件显示当前值）。 */
const ANIM_KIND_OF = new Map(
  ANIM_KINDS.flatMap((k) => k.variants.map((v) => [v.value, k.kind]))
)

/** **文字颜色的命名预设**（`Event::Element.color`，2026-10-13）。
 *
 *  ⚠️⚠️ **权威是 Rust `placement.rs::ElementColor`**（名字 ↔ RGBA 都在那边，
 *  因为出片那条腿走 `process` 算好的 RGBA）。这里只是**下拉的展示清单**，
 *  两边同值由 `scripts/check-element-color.mjs` 钉着（它从 Rust 源码里读真源再逐个比）。
 *
 *  ⚠️ 前端**不参与颜色换算**：预览读的是 `process` 写回串里的 RGBA
 *  （`engine.js::parseTextSource` 的 `p[3]`/`p[4]`），这个下拉只写**名字**。
 *  在这里再写一份 RGBA 表 = 第二份颜色来源，`black` 那条"描边成对翻白"必然漏掉。
 *
 *  ⚠️ `label` 里的情绪词与 `llm/segment.rs` 的提示词**同源同义** ——
 *  面板上写的用途要让用户能预期 LLM 会怎么用它（两处措辞漂了不报错，但用户会困惑
 *  "为什么 LLM 老用红色"）。 */
const ELEMENT_COLORS = [
  { value: 'white', label: '白 · 默认' },
  { value: 'pink', label: '粉 · 可爱调侃' },
  { value: 'red', label: '红 · 愤怒警报' },
  { value: 'yellow', label: '黄 · 吐槽旁白' },
  { value: 'cyan', label: '青 · 冷静疑问' },
  { value: 'green', label: '绿 · 肯定通过' },
  { value: 'black', label: '黑 · 仅浅底衬' }
]
const ELEMENT_COLOR_VALUES = ELEMENT_COLORS.map((c) => c.value)

// ============================================================================
// 两个**独立且可叠加**的设置：样式（形态）× 动效（时间）
// ============================================================================
//
// 用户口径（2026-10-13）：「两个设置，有各自的细分，是可叠加的」。
//
// ⚠️⚠️ **这不是"三级互斥选择器"** —— 那一版（类型 → 种类 → 细分，写同一个
//    `anim`）**已经做错并回退过**：它选样式会把动效**清掉**，而用户要的是
//    「竖排 **+** 抖动」同时成立。
//
// 为什么能叠加：底座 `Layer` 上 `transform`（静态形态）/ `keyframes`（时间）/
// `per_char`（逐字）是**三个独立字段**，两条路读的字段**完全不重叠** ⇒
// 两个档名喂进去不会互相打架（`crates/yekitrim-render-core/src/types.rs` 写着
// 「与 `style` **正交** … 两者能自由组合」）。
//
// ⚠️ `axis` 的三档含义（**2026-10-14 三轴收敛后的当前形状**）：
//   * `'style'`      —— **只**给形态字段（`tilt` / `arc` / `vtext`）
//   * `'transition'` —— **只**给进出场时间字段（`pop` / `fade`）
//   * `'effect'`     —— **只**给动效字段（`shake` / `perchar_big` / `perchar_small` / `heartbeat`）
//
// ⚠⚠ **没有 `'both'`，也没有 `'none'` 这个轴值**—— 那是 39 档时代的东西。
//    收敛后**每一档只属一条轴**，三条轴的档名集合**两两不相交**
//    （判据 `check-anim-attribution.mjs`：表 ↔ `animAxisOf` ↔ **实测产物** 三者一致）。
//    ⇒ 下拉里只列**本轴自己**的档，不再有“同一个名字两栏都出现”。
//
// ⚠ “空”那一项**不是一条轴**：`kind === 'none'` 表示“不指定”，
//    **三栏都要有**（下面 `kindsOnAxis` 里 `k.kind === 'none'` 那一条）。
//    ⚠ 它在排版栏显示成「**默认**」、在另外两栏显示成「不设」—— 见 `kindLabelOn`。

/** 某一轴上的全部**种类**。**每一档只属一条轴**（收敛后没有 `both`）。
 *
 *  ⚠️ **“不指定”（`kind === 'none'`）在排版栏显示成「默认」，在另外两栏显示成「不设」**
 *     —— 见模板与 `kindLabelOn`。
 *
 *     第一版把它当“两条轴都作缺省项”、两栏都叫「不动效」，说明还写着
 *     “这一条文字不做任何动效” ⇒ 用户看到的「样式」栏第一项是「不动效」 ——
 *     一个**形状**的下拉里出现了“什么都不做”，文案还自己在说“动效”。用户当场指出这件事。
 *
 *     ⇒ 各栏的“空”语义**本来就不一样**（现在的叫法以 `kindLabelOn` 为准）：
 *       · 排版栏 = 「**默认**」（回到出厂排版，不是一个形状）；
 *       · 过渡 / 动效栏 = 「**不设**」（不额外加，是一个有意为之的选择）。
 *     ⚠️ 三栏都要留这一项：若没有它，用户**选错了就再也清不掉**
 *        （只能靠手改 toml）。 */
const kindsOnAxis = (axis) => ANIM_KINDS
  // ⚠️ `axis === 'none'` 那一项（「不设」/「默认」）**三栏都进**。
  // ⚠️⚠️ **2026-10-14 三轴收敛后没有 `both` 了** —— 每条档位**只属于一条轴**
  //    （原来 `both` 的那 11 档要么被删、要么被并进唯一一条轴）。
  //    ⇒ 这里不再有 `|| k.axis === 'both'`，`setKindOnAxis` 里那套"同族才同步
  //      另一轴"的逻辑也一并删掉（它的存在理由就是 `both`）。
  .filter((k) => k.kind === 'none' || k.axis === axis)
  // ⚠️ **“空”那一项永远排最后** —— 它出现在选项列表第一项时，
  //    用户会把它当成一个正常的档位来读（他就是这么读的）。
  .sort((a, b) => (a.kind === 'none' ? 1 : 0) - (b.kind === 'none' ? 1 : 0))

/** 某一栏里，一个种类**显示**成什么。
 *
 *  · “空”那一项按轴分开叫法：**排版栏 → 「默认」**（回到没有任何排版变换，是一个**状态**）；
 *    **过渡栏 / 动效栏 → 「不设」**（不额外加，一个有意为之的选择）。
 *    ⚠️ 过渡栏原先叫「默认弹出」（"缺省 = 弹出"）—— 2026-10-15 收敛成单框后已删，
 *       缺省由 `defaultEvent` 落盘 `transition_anim='pop'` 表达，不靠标签暗示。
 *    ⚠️ 别把排版栏也写成同一个词：排版栏写「不设」会让人以为“有一个排版叫不作排版”。
 *    （⚠ 旧叫法里排版栏叫「清除」 —— 已改为「默认」：它是一个**状态**，不是一个**操作**。）
 *
 *  · ⚠️⚠️ **`axis === 'both'` 的那几档，两栏都出现 ⇒ 必须带轴后缀**（2026-10-14）。
 *
 *    # 为什么（用户报「样式和动效内容过于重复，是不是去重后有些没清理」）
 *
 *    实测：**39 档，每档唯一归属一个种类，跨种类重复 0 条** —— 面板一个档都没丢
 *    （`check-anim-presets-parity.mjs` 47 项钉着）。
 *
 *    但归属表是 `形态 15 · 动效 12 · **两者 11** · 无 1` —— 那 11 档**真的两条轴
 *    都给参数**（例：`slam_*` 既给 `squashY`（形态）又给时间曲线（动效））。
 *    ⇒ 两栏都列它们是**必需的**（否则那 11 档会有一半参数在界面上没有入口），
 *      可两栏里显示**同一个名字**，用户读到的是"这两栏重复了一半"。
 *
 *    ⇒ 两栏都出现的档，标签带上**它在这一栏管什么**：「砸入 · 形态」/「砸入 · 动效」。
 *      一眼看出是"同一个档的两个半边"，而不是"没清干净的重复项"。
 *
 *    ⚠️ **`variants[].value` 与 `ANIM_KINDS[].kind` 一个都不许动** ——
 *       `check-anim-presets-parity.mjs` / `check-anim-dropdown.mjs` 用正则从这里
 *       扒档名，改形状会让对拍**静默失效**（扒不到 ⇒ 两侧都空 ⇒ 全绿）。
 *       这里只改**显示**，不动数据。 */
// ⚠️⚠️ **`AXIS_SUFFIX` 已删**（2026-10-14 三轴收敛）。
//
// 它是**上一版**为"`both` 的档在两栏都出现"打的补丁：同一个档名在两栏里
// 显示成「砸入 · 形态」/「砸入 · 动效」，让用户看出是"同一个档的两个半边"。
//
// 收敛后**没有任何档属于两条轴** ⇒ 两栏不可能出现同一个名字 ⇒ 后缀失去意义。
// ⚠️ 别把它当"有用的保险"留着：一旦留着，下一个人会以为"档可以属于两轴"，
//    而那正是用户报的「样式和动效内容过于重复」的根源。
const kindLabelOn = (axis, k) => {
  // ⚠️ "空"那一项三栏的**语义不同**，叫法上排版栏独一份：
  //    · 排版栏 = 「默认」（回到没有任何排版变换 —— 它是一个**状态**）
  //    · 过渡栏 = 「不设」（不额外加过渡 —— 一个有意为之的选择）
  //    · 动效栏 = 「不设」（不额外加动效 —— 一个有意为之的选择）
  //    ⚠️ 别把排版栏写成「不设」：会让人以为"有一个排版叫不作排版"。
  if (k.kind === 'none') {
    if (axis === 'style') return '默认'
    // 过渡栏在 2026-10-15 之前叫「默认弹出」，已改回「不设」：过渡轴不再走
    // 种类/细分两级（见模板里过渡那段），标签不必再暗示缺省是弹出 —— 那件事
    // 由 "defaultEvent" 落盘的 transition_anim = 'pop' 表达（用户口径：弹出为默认）。
    return '不设'
  }
  return k.label
}

/** 某一轴上"当前选中的种类"。⚠️ 认不出的档名 ⇒ `''`（模板走"未知"那一支：
 *  **不许把它悄悄归成"无"**，否则用户改一下别的字段就把错档名永久抹掉了）。 */
const kindOnAxis = (cur) => {
  if (!cur) return 'none'
  return ANIM_KIND_OF.has(cur) ? ANIM_KIND_OF.get(cur) : ''
}

/** 某轴当前值（`selItem` 上那两个字段之一）。 */
const AXIS_FIELDS = { style: 'style_anim', transition: 'transition_anim', effect: 'effect_anim' }
const axisField = (axis) => AXIS_FIELDS[axis]

const styleKind = computed(() => kindOnAxis(selItem.value?.style_anim ?? ''))
const effectKind = computed(() => kindOnAxis(selItem.value?.effect_anim ?? ''))
const transitionKind = computed(() => kindOnAxis(selItem.value?.transition_anim ?? ''))

/** 某轴当前种类下的细分清单（认不出档名时给空数组 ⇒ 那个细分下拉整条不渲染）。 */
const variantsOnAxis = (axis, kind) => {
  const k = kindsOnAxis(axis).find((x) => x.kind === kind)
  return k ? k.variants : []
}
const styleVariants = computed(() => variantsOnAxis('style', styleKind.value))
const effectVariants = computed(() => variantsOnAxis('effect', effectKind.value))
const transitionVariants = computed(() => variantsOnAxis('transition', transitionKind.value))

// ---- 排版角度滑杆（2026-10-14）------------------------------------------
//
// 用户口径：「**倾斜和弧形需要有角度参数可调整**」。角度走**事件字段**
// `layout_angle_deg`（`Option<f64>`，度），与档名**正交**：改档位不重置角度。
//
// ⚠️ **只有 `tilt` / `arc` 两档显示这个滑杆** —— 别的排版档（`vtext`）没有旋转语义，
//    显示了用户会以为它能调（调了不生效，而界面看着是生效的）。
const showAngle = computed(() => {
  const k = ANIM_KINDS.find((x) => x.kind === styleKind.value)
  return !!(k && k.angle)
})

/** 滑杆当前值：**没填过时显示档位的缺省角度**（那是它现在的实际表现）。
 *
 *  ⚠️ 这里**只读不写** —— 显示缺省值 ≠ 把缺省值落盘。用户没动滑杆时
 *     `layout_angle_deg` 必须**仍然是 `null`**（R4：没配的字段一个键都不多写）。
 */
const ANGLE_DEFAULT_BY_KIND = { tilt: -6, arc: -14 }
const layoutAngle = computed(() => {
  const v = selItem.value?.layout_angle_deg
  if (v === null || v === undefined) return ANGLE_DEFAULT_BY_KIND[styleKind.value] ?? 0
  return Number(v)
})
/** 用户是否**显式设过**角度（决定面板上要不要给一个"回到默认"的入口）。 */
const layoutAngleSet = computed(
  () => selItem.value?.layout_angle_deg !== null && selItem.value?.layout_angle_deg !== undefined,
)

/** 拖滑杆 ⇒ 写事件字段。
 *  ⚠️⚠️ **写 `null` 而不是 `0`** 才能表达"回到默认"：`0` 是一个合法的
 *     用户选择（"我要它正着"），而 `null` 是"这个键不存在"。
 *     —— 这两者混起来正是本仓"三态"那条老坑。 */
function setLayoutAngle(v) {
  if (!selItem.value) return
  const n = Number(v)
  selItem.value.layout_angle_deg = Number.isFinite(n) ? n : null
}
/** 「回到默认」= 把键删掉（走档位的缺省角度）。 */
function resetLayoutAngle() {
  if (!selItem.value) return
  selItem.value.layout_angle_deg = null
}


// ---- 单个元素的位置（X / Y 百分比，2026-10-15）----------------------------
//
// 用户口径：「保留一个 xy 位置滑块，方便手动修改**单个事件**的位置（仅限贴纸和文字层）」。
//
// ⚠️ 语义是**三态**，别退化成两态：
//    · `undefined`/`null`/空串 = **没设** ⇒ 后处理按安全落点算（贴纸 Step 2b / 文字 Step 1d3）；
//    · `"76%"`                  = **手动钉住** ⇒ 自动落点不再管它。
//    所以「清空输入框」必须写 `undefined`（键消失），**不能写成 `0`** ——
//    `0` 是一个合法的用户选择（"贴左上角"），和"没设"完全不是一回事。
//
// ⚠️ 两个载体的字段类型不同：`Element` 的 `x`/`y` 是 `Option<String>`、
//    `Sticker` 的是 `String`。本控件**只在 element 支**出现，写值统一走 `setElementPos`。

/** 输入框里显示的数字（去掉 `%`）。没设过 ⇒ 空串（占位符「自动」会显示出来）。 */
function pctOf(v) {
  if (v === null || v === undefined || v === '') return ''
  const n = parseFloat(String(v).replace(/%$/, ''))
  return Number.isFinite(n) ? String(Math.round(n * 10) / 10) : ''
}

/** 滑杆的位置：没设过时**停在 50**（画面中心），而不是 0 ——
 *  0 是"贴左上角"这个真实选择，用 0 表示"没设"会让用户以为元素已经钉在角上了。
 *  ⚠️ 这里**只读不写**：滑杆显示 50 ≠ 落盘 50（R4：没配的键一个都不多写）。 */
function posSlider(v) {
  const s = pctOf(v)
  return s === '' ? 50 : s
}

/** 这个元素有没有被手动钉过位置（决定要不要给「自动」按钮）。 */
function hasElementPos(ev) {
  const on = (v) => v !== null && v !== undefined && v !== ''
  return !!(ev && (on(ev.x) || on(ev.y)))
}

/** 写一格位置。**空/非数字 ⇒ 删键**（回到自动落点），夹到 [0,100]。 */
function setElementPos(axis, raw) {
  if (!selItem.value) return
  const s = String(raw ?? '').trim()
  if (s === '') {
    selItem.value[axis] = undefined
    return
  }
  const n = parseFloat(s)
  if (!Number.isFinite(n)) {
    selItem.value[axis] = undefined
    return
  }
  const clamped = Math.min(100, Math.max(0, n))
  selItem.value[axis] = `${clamped}%`
}

/** 「自动」= 把两轴都删掉（后处理重新接管落点）。 */
function clearElementPos() {
  if (!selItem.value) return
  selItem.value.x = undefined
  selItem.value.y = undefined
}

// ⚠️ **`hintOnAxis` / `styleHint` / `effectHint` 已删**（用户口径 2026-10-13）：
//    每个种类下面那行小字（如「这一条文字不做任何动效」）是**多余的** ——
//    选项标签本身已经说清了是什么，"点了会发生什么"不需要再写一遍。
//    ⚠️ 别加回来：`ANIM_KINDS` 里的 `hint` 字段也一并去掉了，
//       留着字段却没人读，下一个人会以为它还在生效。

/** 切**种类**：落到该类的**第一个细分**（"无"那类只有空串一项）。
 *
 *  ⚠️ 必须落成 `null` 而不是 `''`（空串是一个合法的**坏值**，`null` 才是
 *     "这个键不存在"）。判据 `check-anim-presets-parity.mjs` ⑥ 钉着这一条。
 *
 *  ⚠️⚠️ **只动自己那一轴** —— 这就是"可叠加"与"互斥"的分界线。
 *     第一版在这里顺手写了另一轴 ⇒ 用户选样式就把动效清掉了。
 *     ⚠️ 判据 `check-anim-dropdown.mjs` 钉着"两轴各写各的"。 */
function setKindOnAxis(axis, kind) {
  const k = kindsOnAxis(axis).find((x) => x.kind === kind)
  if (!k || !k.variants.length) return
  const v = k.variants[0].value
  // ⚠️⚠️ **"同步另一轴"那整套已删**（2026-10-14 三轴收敛）。
  //
  //    它是为 `axis === 'both'` 的档写的补丁：一个档在两轴都给参数 ⇒ 用户从这一栏
  //    选它时，另一栏也得设上，否则"带压扁"只生效半个。
  //    （历史：那一版在这里顺手写另一轴，结果**选样式把动效清掉了**；
  //      修法是"另一轴同族才同步" —— 即原来那段 `sync` 判据。）
  //
  //    收敛后**每条档位只属于一条轴** ⇒ 没有"半个效果"这回事 ⇒ 整套逻辑失去前提。
  //    ⚠️ 别把它当"保险"留着：留着会让"三轴可独立叠加"这条不变量看起来还依赖
  //       "同族"这种隐式耦合，下一个人改档位表时必然要重新推一遍。
  //    ⚠️ 三轴可叠加现在由**字段不重叠**保证（转译器 `ANIM_PRESETS` 的 `sort`
  //       + `scripts/check-anim-two-axis.mjs`），不靠这里的同步。
  selItem.value[axisField(axis)] = v || null
}

/** 切**细分**（同一条上那个 `|| null` 的理由）。⚠️ 同样**只动自己那一轴**。 */
function setVariantOnAxis(axis, value) {
  // ⚠️ 同 `setKindOnAxis`：**"同族才同步另一轴"那套已随 `both` 一起删**
  //    （2026-10-14 三轴收敛）。理由见那边。
  //
  // ⚠️ 唯一保留的是那个 **`|| null`** —— 空串是一个合法的**坏值**，
  //    `null` 才是"这个键不存在"。判据 `check-anim-presets-parity.mjs` 钉着它。
  selItem.value[axisField(axis)] = value || null
}

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

/** `element` 的**种类** —— `'text'` / `'sticker'` / **`'unknown'`**。
 *
 *  ⚠️⚠️ **认不出时必须是 `'unknown'`，不许默认成 `'text'`**。
 *  `kind` 可能是 LLM 漏写/写错的值（Rust 侧 `#[serde(default)]` 兜空串），
 *  默认成文字会把**坏数据伪装成正常元素** —— 用户看到的是一个能编辑的文字框，
 *  出片侧却按"认不出的 kind"处理。照实报出来，用户才有机会自己发现。
 *
 *  ⚠️ 与 `labelFor` 里那条判据**同口径**（那边返回 `'⚠' + k` 显示在泳道上）——
 *  两处必须一致，否则会出现"泳道上写着认不出、面板里却当成文字"。
 *
 *  ⚠️ 空串算 `'text'`：那是**没写过的默认值**（面板新建的就是它），
 *  与"写了个认不出的值"是两回事。 */
function elementKindOf(ev) {
  const k = String(ev?.kind ?? '').trim().toLowerCase()
  if (k === 'text' || k === '') return 'text'
  if (k === 'sticker') return 'sticker'
  return 'unknown'
}
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
  // ⚠️⚠️ **「元素」轨在界面上拆成两条：文字 / 贴纸**（2026-10-14，用户口径
  //    「前端编辑器也要分离」）。
  //
  //    历史：这条轨原先叫「元素」，**同时装 `sticker` 与 `element`** 两种事件
  //    （`LANE_TYPES('sticker')` → `['sticker','element']`，与 `spec/effects.json`
  //    的座位表一致）。混装的代价是"＋"必须弹一个「文字 / 贴纸」二选一菜单 ——
  //    而用户本来就是在**某一条轨**上点的加号，那个选择是多余的。
  //
  //    ⇒ 现在拆成两条泳道（靠上面 `kinds` 过滤）。
  //    ⚠️ 磁盘上它们**仍然是同一条 `sticker` 轨道** —— `lane` 照旧写 `sticker`
  //       （Rust `Lane::key` 与四处 `LANE_OF_TYPE` 都按它走），**不做数据迁移**。
  //       拆的只是**界面上的两条泳道**。
  //
  // ⚠️ 两条的 `type` / `types` **完全一样**（都是 `['sticker','element']`）——
  //    分开它们的只有 `kinds`。所以**别的按 `lane.type` 找轨道的代码会同时命中两条**
  //    （`lane.type` 不再唯一）。凡是"按 type 索引泳道"的地方都要用 `lane.id`：
  //    `id` 是**界面专用**的唯一键，与磁盘无关（见下面 `laneOpen` / `:key` 的用法）。
  { id: 'text', type: 'sticker', kinds: ['text', ''],
    label: '文字', lane: 'sticker', color: TIMELINE_COLORS.sticker, types: LANE_TYPES('sticker'), resizable: true },
  { id: 'sticker', type: 'sticker', kinds: ['sticker'],
    label: '贴纸', lane: 'sticker', color: TIMELINE_COLORS.sticker, types: LANE_TYPES('sticker'), resizable: true },
  { type: 'fx', label: '视觉特效', lane: 'effects', color: TIMELINE_COLORS.flash, types: LANE_TYPES('fx'), resizable: true },
  { type: 'sfx', label: '音效', lane: 'sfx', color: TIMELINE_COLORS.sfx, types: LANE_TYPES('sfx'), resizable: true }
]
const lanes = LANE_DEFS

/** 泳道的**界面唯一键**（`id` 优先，没有就用 `type`）。
 *
 *  ⚠️⚠️ **元素轨拆成「文字 / 贴纸」两条之后，`lane.type` 不再唯一** ——
 *  两条的 `type` 都是 `'sticker'`（它们磁盘上确实是同一条轨）。
 *  于是凡是**当唯一键用**的地方（`v-for` 的 `:key`、`laneOpen`、选中态判等、
 *  `emit('lane-open')`）都**必须走 `laneKey`**，否则两条泳道会：
 *    * 共用同一个 `:key` ⇒ Vue 复用错节点（折叠/高亮串台）；
 *    * 共用同一个 `laneOpen` ⇒ 张开一条、另一条跟着张；
 *    * 选中态判等时**两条都亮**。
 *
 *  ⚠️ **别把 `laneKey` 用在数据面**：`laneBlocks` 的 `types` 过滤 / `LANE_OF_TYPE` /
 *  Rust `Lane::key` 那些地方要的是**真实的轨道名**（`'sticker'`），不是界面 id。 */
const laneKey = (lane) => lane.id || lane.type

/** 泳道的**轨道名**（发给外面的那个值）。
 *
 *  ⚠️ 与 [`laneKey`] **不是一回事**，别混：
 *    * `laneKey` = 纯界面 id（`v-for` 的 `:key`、选中态判等）—— 文字轨与贴纸轨**必须不同**；
 *    * `laneTypeOf` = **磁盘上的轨道名**（两条都是 `'sticker'`）—— `emit('lane-open')`
 *      发的是它，因为 `PolishView` 拿它去查"该滚到哪一组控件"
 *      （`laneGroupEls[lane]`，那张表按**轨道名**索引）。
 *
 *  ⚠️ **`emit('lane-open')` 必须发 `laneTypeOf`**：发 `laneKey` 的话，
 *    点「文字」轨（id 是 `text`）会去查 `laneGroupEls.text` —— **不存在**
 *    ⇒ 退回面板顶部，用户看到的是"点轨道名没反应"。而两条轨共用同一组「区域设置」，
 *    发的本来就该是同一个轨道名。 */
const laneTypeOf = (lane) => lane.type

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

// ────────────── 属性面板的**字段控件**（全部由词汇表派生，2026-10-15）──────────────
//
// 口径（与 `scripts/check-layout-invariants.mjs` ㉛ 逐条对应，改之前先读那段注释）：
// **加一个字段 = 改 `spec/effects.json` 的 `fields[]` 一行** —— 这里不再出现字段名清单。
//
// ⚠️ `DEDICATED_FIELDS` 是**排除清单**：那几类字段由属性面板的**专用块**亲自渲染
//    （色板 / 素材选择器 / 类型下拉…），走自动派生反而是两个控件写同一个键
//    （写回时后者赢，静默丢设置）。⚠️ **别把词汇表里的标量字段加进来** —— ㉛ 钉着。
//
// ⚠️ 反过来：**专用块没覆盖到的字段必须靠 `autoFields` 兜底露出来**。
//    兜底块被删 = 那一整类字段从界面上**整类消失**，而且不报错。

/** 类型 → 该类型的**词汇表字段**（`effects[].fields` 原样）。 */
const TYPE_FIELDS = Object.fromEntries(
  EFFECTS.map((e) => [e.id, Array.isArray(e.fields) ? e.fields : []])
)

/** 专用块自己渲染的字段名（不进自动派生）。 */
const DEDICATED_FIELDS = new Set([
  'sfx', 'sfx_volume', 'color', 'intensity', 'manual', 'end',
  'keyword', 'name', 'source', 'sticker_tag', 'x', 'y', 'text', 'kind', 'volume'
])

/** 字段该用哪种控件。显式 `ui` 优先，其次按类型串猜（`Option<f64>` → number）。 */
function controlKind(f) {
  const t = String(f.type || '')
  if (f.ui === 'switch' || t === 'bool') return 'switch'
  if (f.ui === 'select' || Array.isArray(f.enum)) return 'select'
  if (t.includes('f64')) return 'number'
  return 'text'
}

/** 哪些事件类型在属性面板里有**专用支**（= 手写了控件的那几支）。
 *
 *  ⚠️⚠️ **这张表是"兜底块该不该跑"的唯一判据**（2026-10-15 二次修）。
 *  兜底块（`autoFields(selItem.type)`）只该给**没有专用支**的类型用；
 *  对有专用支的类型再跑一遍 = **每个字段出两遍**。
 *
 *  ⚠️ 症状是**静默的**：没有报错，只是面板上同一组控件原样列两遍。
 *     实测踩过两次 —— 第一版只排除了当时正在改的 `element`，
 *     而 `camera`（7 个字段）与 `overlay`（5 个）照旧重复，用户当场贴图。
 *
 *  ⚠️ **加了新的专用支，必须往这里加一行** —— 判据 ㉞ 会拿它对模板里
 *     真实的 `selItem.type === '…'` 分支算差集，漏了当场红（别再靠人眼）。 */
const DEDICATED_BRANCH_TYPES = ['camera', 'element', 'overlay', 'sfx', 'sticker']

/** 这个类型在面板里有没有专用支（有 ⇒ 兜底块**不许**再渲染它的字段）。 */
const hasDedicatedBranch = (type) => DEDICATED_BRANCH_TYPES.includes(type)

/** `element` 支**已经手写渲染**的字段（那支的 skip 表，**唯一一份**）。
 *
 *  ⚠️⚠️ 这张表是"专用块渲染了什么"的**声明**，漏一个字段的症状就是**它出两遍**
 *  （2026-10-15 用户报的「一堆冗余参数」正是 `style_anim` / `transition_anim` /
 *  `effect_anim` / `layout_angle_deg` 四个漏在表外）。
 *  ⚠️ 加/删专用块里的控件时**必须同时改这张表** —— 判据 ㉜ 会算差集，漏了当场红。
 *  ⚠️ 别把它并进 `DEDICATED_FIELDS`：那个是**全局**排除清单（color/intensity/volume 等
 *     专用块字段），而这张是 **element 专属**的（`text`/`sticker_tag` 在别的类型里是自动字段）。 */
const ELEMENT_DEDICATED = [
  'kind',            // 类型由轨道决定，面板只在认不出时给提示
  'text',            // 文字支的正文输入框
  'sticker_tag',     // StickerPicker 的「分类」那一级
  'name',            // ↑
  'source',          // ↑
  'color',           // 文字颜色下拉
  'style_anim',      // 「排版」下拉
  'transition_anim', // 「过渡」下拉
  'effect_anim',     // 「动效」下拉
  'layout_angle_deg',// 倾斜/弧形角度滑杆
  'x', 'y',          // 位置控件（下面那一对）
]

/** 该类型**没被专用块吃掉、也没被 `skip` 点名**的字段（= 应该自动生控件的那些）。
 *
 *  ⚠️⚠️ **`panel: false` 的字段一律不给控件**（2026-10-15 用户口径：「一堆冗余参数」，
 *  这些应该是后处理自动处理的、或者不该有的）。标记在 `spec/effects.json` 的字段上，
 *  与它是不是"合法字段"是两件事 —— 这些字段**权威仍在词汇表里**（转译器读、老工程写），
 *  只是**编辑器不提供入口**。给它们控件比没有更糟：
 *    · `anim` —— **只读迁移**字段，写回是 no-op ⇒ 用户改完看着生效、成片一个字都不动；
 *    · `pos`  —— 2026-10-12 已被用户从界面撤掉，默认 Auto，重新暴露 = 把撤掉的能力带回来。
 *  判据：`webui/scripts/check-layout-invariants.mjs` ㉝。 */
const autoFields = (type, skip = []) => {
  const s = new Set(skip)
  return (TYPE_FIELDS[type] ?? []).filter(
    (f) => f && f.name && f.panel !== false && !DEDICATED_FIELDS.has(f.name) && !s.has(f.name),
  )
}

// ⚠️ 用 `defineComponent` + 渲染函数写：同一字段要在两种布局里出（bool 走紧排行、
//    其余走 label+控件），写成模板会散成十几个 `v-if` 分支。
const FieldControl = defineComponent({
  name: 'FieldControl',
  props: { ev: { type: Object, required: true }, f: { type: Object, required: true } },
  setup(props) {
    // ⚠️ **空串要写 `undefined` 而不是 `''`**：字段多是 `Option<...>` + `skip_serializing_if`，
    //    写空串会被 serde 读成 `Some("")`，而那正是「无法识别」那个报错的源头。
    const write = (v) => { props.ev[props.f.name] = v === '' ? undefined : v }
    return () => {
      const f = props.f
      const cur = props.ev[f.name]
      const kind = controlKind(f)
      const label = h('label', f.label || f.name)
      if (kind === 'switch') {
        return h('label', { class: 'fld-bool' }, [
          h('input', {
            type: 'checkbox',
            checked: cur === true,
            onChange: (e) => { props.ev[f.name] = e.target.checked }
          }),
          ' ' + f.name
        ])
      }
      if (kind === 'select') {
        return [label, h('select', {
          value: cur ?? f.default ?? '',
          onChange: (e) => write(e.target.value)
        }, [
          h('option', { value: '' }, '（缺省）'),
          ...(f.enum || []).map((v) => h('option', { value: v }, v))
        ])]
      }
      if (kind === 'number') {
        const shown = cur ?? f.default ?? '缺省'
        return [
          h('label', `${f.name}（${shown}）`),
          h('input', {
            type: 'number',
            value: cur ?? '',
            min: f.min ?? undefined,
            max: f.max ?? undefined,
            step: f.step ?? 0.05,
            onInput: (e) => write(e.target.value === '' ? '' : Number(e.target.value))
          })
        ]
      }
      return [label, h('input', { type: 'text', value: cur ?? '', onChange: (e) => write(e.target.value) })]
    }
  }
})

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
/** 贴纸库读取失败的原因（非空 ⇒ 选择器里出错误条）。
 *  ⚠️ 与贴纸来源 / 素材清单同一条纪律：读失败**不许**退化成「库是空的」那个假空态 ——
 *  写 `stickerLib.value = []` 会让「接口挂了」与「一张贴纸都没放」长得一模一样。 */
const stickerLibError = ref('')
const sfxEntries = ref([])
async function loadPickers() {
  try {
    const r = await fetchStickerLibrary(props.pid, props.dir, '读取贴纸库')
    stickerLib.value = r?.packs || []
    stickerLibError.value = ''
  } catch (e) {
    // 列表保留上一次那份，只把原因挂出去（失败时清空 = 用户以为自己的贴纸库被清空了）
    stickerLibError.value = (e && e.message) || String(e)
  }
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

// ⚠️ **`segDecisions` 已删**（2026-10-15）：它是「系统做得对吗」那块的数据源，
//    随那块一起下架。`[[decisions]]` 数据仍在 `polish.toml` 里（process 仍写），
//    只是编辑器不再展示 —— 別为了"信息别浪费"把它换个地方挂回来。

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
      // ---- ⚠️⚠️ **文字 / 贴纸在同一盘的「两条泳道」上分开**（2026-10-14）----
      //
      // 用户口径：「**前端编辑器也要分离**」。
      //
      // `element` 与 `sticker` 在**磁盘上是同一条轨**（`Lane::Sticker` ⇒ `"sticker"`，
      // 四处 `LANE_OF_TYPE` 镜像都这么映射）—— 那个**不能改**（改了就要求数据迁移，
      // 而老工程里全是 `type:"sticker"`）。
      //
      // 但**界面上**它们该是两条轨：它们的载荷、属性面板、能选的东西**完全不同**
      // （文字有排版/描边/逐字，贴纸只有图和落点）。混在一条轨上的代价是
      // "＋" 必须弹一个「文字 / 贴纸」二选一菜单 —— 那是**替用户做了一次多余的选择**，
      // 而用户本来就是在某一条轨上点的加号。
      //
      // ⇒ `lane.kinds` = **在 UI 上再分一次**（`types` 管事件类型，`kinds` 管元素种类）。
      //   不写 `kinds` 的泳道 = 不过滤（行为与从前逐字节相同，R4）。
      if (lane.kinds) {
        const k = type === 'element' ? (ev.kind || 'text') : (type === 'sticker' ? 'sticker' : null)
        if (k !== null && !lane.kinds.includes(k)) return
      }
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
  // ⚠️ 用 `laneKey`（不是 `lane.type`）—— 元素轨拆成两条之后 `type` 不再唯一
  if (!s || s.lane !== laneKey(lane)) return false
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
  if (block.kind === 'sub') selected.value = { kind: 'sub', lane: laneKey(lane), subIndex: block.subIndex }
  else if (block.kind === 'mute') selected.value = { kind: 'mute', lane: laneKey(lane), muteIndex: block.muteIndex }
  else selected.value = { kind: block.kind, lane: laneKey(lane), si: block.si, ei: block.ei }
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

/** **逐块手动落点**（2026-10-16 用户口径：「贴纸和文字层都需要一个可以给每个单独块手动选择位置的功能，
 *  自动是全局的，手动过后自动无法影响」）。
 *
 *  `manual_pos` = 这一块的**中心**百分比（与 `[sticker] position` 同基准；落盘的 `x`/`y` 是左上角，
 *  换算在 Rust `placement::manual_pos_to_box_px` 一处做）。点一下预览 ⇒ 写进**选中那条**事件；
 *  「恢复自动」＝删键 ⇒ 下次 `process` 的自动落点重新接管（`process` 见到 `manual_pos` 时求解器不参与）。
 *
 *  ⚠️ 事件数据归**本组件**（它是落盘方）⇒ 点选态住在这里；预览那边只负责"要不要接收点击"，
 *  由页面经 `point-pick` 转达 —— 别把落盘散到页面上去（那是"两份写同一份文件"的老坑）。 */
const manualPick = ref(false)
/** 拖动中的**草稿**（盒子中心百分比）。⚠️ 只在这里攒着，**按「保存落点」才写进事件** ——
 *  用户口径（2026-10-17）：「可以拖动位置，面板上确定位置，而不是点一下就确定了」。 */
const manualDraft = ref({ x: 50, y: 50 })
const manualDraftPct = computed(() => `${Math.round(manualDraft.value.x)}% ${Math.round(manualDraft.value.y)}%`)
const evManualPos = computed(() => String(selectedEv.value?.manual_pos || ''))

/** 进编辑态：草稿从已有 `manual_pos` 起；没有就退到事件当前的 `x`/`y` 当近似起点（随即可拖）。 */
function startManualPick() {
  const ev = selectedEv.value
  if (!ev) return
  const mp = String(ev.manual_pos || '').trim()
  const m = /^(-?\d+(?:\.\d+)?)%?\s+(-?\d+(?:\.\d+)?)%?$/.exec(mp)
  if (m) manualDraft.value = { x: Number(m[1]), y: Number(m[2]) }
  else {
    const gx = parseFloat(String(ev.x ?? '').replace(/%$/, ''))
    const gy = parseFloat(String(ev.y ?? '').replace(/%$/, ''))
    manualDraft.value = { x: Number.isFinite(gx) ? gx : 50, y: Number.isFinite(gy) ? gy : 50 }
  }
  manualPick.value = true
  emit('point-pick', true)
}

/** **只有这里**写 `manual_pos`（面板确认 ✔）；写完退出编辑态。 */
function saveManualPos() {
  const ev = selectedEv.value
  if (ev) {
    ev.manual_pos = manualDraftPct.value
    touch(dirtyKindOf(selected.value?.kind))
  }
  cancelManualPick()
}

function cancelManualPick() {
  manualPick.value = false
  emit('point-pick', false)
}

/** 「恢复自动」：把键**删掉**（不是写空串 —— 空串是"有值但认不出"，会被当成脏值告警）。 */
function clearManualPos() {
  const ev = selectedEv.value
  if (!ev || !('manual_pos' in ev)) return
  delete ev.manual_pos
  touch(dirtyKindOf(selected.value?.kind))
}

// 预览上的**拖动**只更新草稿（`seq` 每 +1 代表一次新的指针位置）—— **绝不落盘** ✔
watch(() => props.pointResult?.seq, () => {
  if (!manualPick.value || !props.pointResult) return
  manualDraft.value = { x: props.pointResult.x, y: props.pointResult.y }
})
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
  // ⚠️ 热度曲线同样是"属于上一个片段"的东西：弹幕数组与已算好的序列一起清，
  //    否则新片段的曲线会**照旧用旧片段的弹幕**画出来（跨片段串写，且不报错）。
  heatRaw.value = null
  heatDanmaku.value = []
  heatHoverT.value = null
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
// ⚠️ `spanHint` **整块已删**（2026-10-15 用户口径）：贴纸那句（"淡出 0.3s / 入场 0.35s / 浮起 0.4s"）与
// 瞬时特效那句（"跨度夹到 0.1~0.6s"）用户都不要。面板里不再有任何跨类型的时间说明行。

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
  //
  // ⚠️⚠️ **`transition_anim` 必须显式写 `'pop'`**（2026-10-15，用户口径：
  //    「过渡只有弹出和淡入淡出这两个选项，其中**弹出为默认也就是初始化时的参数**」）。
  //
  //    以前这里**不写**这个键 ⇒ 新建的文字层过渡是**空串** ⇒ 落到转译器
  //    `motionFields` 里 `on = !!preset || !!stylePreset || !!transitionPreset` 为 **false**
  //    ⇒ 走 `pick()` 的**回落全局**那一支。后果有两个，且都不报错：
  //      · 工程**没有** `[elements]` 段（最常见）⇒ `popInOn` 为假 ⇒ 三个 `scale_in_*`
  //        一个都不写 ⇒ **字根本不弹**，而面板上那一栏写着「默认弹出」（见 `kindLabelOn`）——
  //        界面在撒谎；
  //      · 工程**有** `[elements]` ⇒ 这条字**静默继承**全局那份（可能是抖动、可能是心跳、
  //        也可能什么都不加），面板上照样写着「默认弹出」。
  //
  //    ⇒ 初始化就把 `'pop'` **写实**：`on` 变 true ⇒ 不再回落全局，
  //      而 `ANIM_PRESETS.pop` 那一组（0.3 → 1.0 / 350ms）就是「弹出」本身。
  //    ⚠️ 这与 R4 **不冲突**：`defaultEvent` 只在**新建**（`addEventAt`）与**改类型**
  //       （`changeEventType`）两条路上跑，**存量工程的事件一个字都不碰** ——
  //       R4 判据钉的是"老工程序列化回来不许多键"
  //       （`yekitrim-render-core/src/types.rs::element_axes_roundtrip`），那条仍然绿。
  //    ⚠️ 别改成"新建时不写、只在显示时假装是弹出"：那正是上面那两个静默后果的来源。
  else if (type === 'element') {
    ev.kind = 'text'
    ev.text = ''
    ev.style = 'comment'
    ev.pos = 'bottom'
    ev.transition_anim = 'pop'
  }
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
function onLaneAdd(laneKeyOrType, evt) {
  const laneType = laneKeyOrType
  if (laneType === 'subtitles') { addSubtitleAtCurrent(); return }
  if (laneType === 'segments') { addMuteAtCurrent(); return }
  if (laneType === 'fx') { openFxMenu(evt); return }
  // ---- ⚠️ **文字轨 / 贴纸轨：直接加，不再弹二选一菜单**（2026-10-14）----
  //
  // 以前只有一条「元素」轨，它同时装 `sticker` 与 `element` 两种事件
  // ⇒ 点＋必须先问"你要文字还是贴纸"（`openElementMenu`）。
  // 现在**轨道本身已经分开了** —— 用户在"文字"轨上点＋，意思就是文字；
  // 再弹一次菜单是**把已经知道的事再问一遍**。
  //
  // ⚠️ 两种都建 `type: 'element'`（差别只在 `kind`）—— 与旧口径一致，
  //    别退回 `type:'sticker'` 那条老载体（见 `addElementAt` 的注释）。
  if (laneType === 'text') { addElementAt('text'); return }
  if (laneType === 'sticker') { addElementAt('sticker'); return }
  addEventAt(laneType)
}

const fxMenu = ref(null) // { x, y } | null —— 视觉特效的细分类型菜单（fixed 定位，不受时间轴滚动影响）

/** 在播放头处加一个**指定 kind** 的元素（`text` 走排版、`sticker` 走贴纸图）。 */
function addElementAt(kind) {
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

  // 热度曲线的**弹幕那一路**：`data/danmaku.json`（`polish init`/`parse` 写的那份，与 `process`
  // 的爆点判据同源）。三条纪律与下面那条情绪轨道同规格：
  //   ① 404 = **正常空态**（老工程的片段目录里根本没有这份文件），不是错误；
  //   ② 其它失败**只 warn、不打断**，也**不进 `loadError`** —— 它不参与任何写路径；
  //   ③ 不许静默降级：读不到弹幕时曲线只按字幕算，这件事写在 `heatTip` 里（不是悄悄变一条线）。
  // ⚠️ 走 `clipMediaUrl` + 裸 `fetch`（与 `PolishView`/`PolishPreview` 同一条既有做法）：
  //    这是媒体文件、不是 `api()` 那条 JSON 通道。`/api/media/*` 带 ETag，
  //    预览那条腿已经拉过同一份，这里通常是一次 304。
  heatDanmaku.value = []
  try {
    const dmUrl = clipMediaUrl(props.pid, props.dir, 'data/danmaku.json')
    if (dmUrl) {
      const dmRes = await fetch(dmUrl)
      if (dmRes.ok) {
        const rows = await dmRes.json()
        // **只留 `start`**：文本/颜色与本曲线无关，没必要把一份几百 KB 的弹幕整份留在内存里
        heatDanmaku.value = (Array.isArray(rows) ? rows : [])
          .map((d) => Number(d && d.start))
          .filter((v) => Number.isFinite(v))
      } else if (dmRes.status !== 404) {
        addClient('warn', '弹幕读取失败（热度曲线只用字幕）：HTTP ' + dmRes.status, 'timeline')
      }
    }
  } catch (e) {
    addClient('warn', '弹幕读取失败（热度曲线只用字幕）：' + (e.message || e), 'timeline')
  }
  // 曲线 = **这一趟装载时的快照**（用户口径 2026-10：不随编辑重算，见 `heatRaw` 的注释）。
  // 放在字幕读完之后：它吃的就是刚读进来的那份字幕。
  heatRaw.value = heatSeries({
    subtitles: subtitles.value,
    danmakuStarts: heatDanmaku.value,
    duration: duration.value,
  })
  if (!heatRaw.value.hasDanmaku) {
    addClient('debug', '热度曲线：本片段没有弹幕，只按字幕算', 'timeline')
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
/* ---- 排版角度滑杆（2026-10-14）------------------------------------------
   ⚠️ 与设置页的 `.slf-range`（宽度 = 格子 1/3）**不是一个场景**：这里属性面板窄，
   `33.333%` 会让滑条短到拖不准 ⇒ 让它 `flex:1` 占满剩余宽度，数值与「默认」
   按钮各自 `flex:none`（否则数值变成两位数/三位数时，滑条宽度会跳一下）。
   ⚠️ 数字用**等宽**（`tabular-nums`）：`-30` 与 `-6` 位数不同，不等宽会让
   滑条宽度随值抖动。 */
.angle-row { display:flex; align-items:center; gap:6px; }
.angle-row input[type=range] { flex:1 1 auto; min-width:0; }
.angle-val { flex:none; min-width:38px; text-align:right; font-size:12px;
  font-variant-numeric:tabular-nums; color:var(--ink-dim); }
.angle-reset { flex:none; font-size:11px; padding:2px 6px; cursor:pointer;
  background:var(--bg-soft); border:1px solid var(--line-soft); border-radius:5px; color:var(--ink-dim); }
.angle-reset:hover { background:var(--main-soft); color:var(--main-deep); }
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
/* 热度曲线开着时标尺带长高 `HEAT_H`(16px)：上 16px 是曲线，下 20px 是原来的刻度与时间标签。
   ⚠️ 关掉曲线要**缩回 20px**（不留空槽）⇒ 下面这三条 `.heat-on` 必须成对，少一条就与轨道列错位。 */
.ruler-label.heat-on { height:36px; }
.ruler-lane { height:20px; border-bottom:none; }
.ruler-lane.heat-on { height:36px; }
.ruler-track { height:20px; cursor:col-resize; }
.ruler-track.heat-on { height:36px; }
/* 曲线：带子刻意**不高**（用户口径 2026-10：参考字幕泳道上那条响度曲线），但"小"指的是它是条窄带、
   **不是起伏看不出来** —— 所以要幅度时先动 `HEAT_H` 与显示对比拉伸（`heat.js`），别加泳道。
   色值一律走 tokens（两套主题成对）；halo 用 `--card` 把曲线从刻度线上「抠」出来
   —— 标尺底色就是卡片色，单色线压着刻度容易在竖线附近糊掉。 */
.heat-layer { position:absolute; left:0; top:0; pointer-events:none; z-index:3; }
.heat-core { fill:none; stroke:var(--main); stroke-width:1.4; stroke-linejoin:round; stroke-linecap:round; }
.heat-halo { fill:none; stroke:var(--card); stroke-width:3; stroke-linejoin:round; stroke-linecap:round; }
.ruler-tick { position:absolute; top:0; bottom:0; width:1px; background:rgba(0,0,0,0.14); }
.ruler-track.heat-on .ruler-tick { top:16px; }
.ruler-time { position:absolute; top:1px; left:4px; font-size:9px; color:var(--ink-dim); white-space:nowrap; pointer-events:none; }
/* ⚠️ **标签不许再叠一次位移**：`.ruler-time` 的 containing block 就是 `.ruler-tick`（Span 在绝对定位的
   Div 里）—— 刻度已经整体下移 16px，标签保持原来那个 `top:1px` 就正好落在曲线下面那 20px 里。
   再补一条 `top:13px` 就是 16 + 13 + 字号 9 ≈ 38px > 36px 带高 ⇒ **标签被裁掉半行**（2026-10 实测，
   静态判据和"刻度下移了 16px"的读数都拦不住它，是截图放大才看出来的）。 */
.ruler-name { flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
/* 读数：悬停时 = 鼠标处那一桶，否则 = **播放头**那一桶（"视频的实时热度"就是这一档）。 */
.ruler-readout { flex:none; margin-left:auto; font-size:10px; color:var(--ink-dim); font-variant-numeric:tabular-nums; }
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
/* 「系统做得对吗」那块的 `.seg-why` / `.why-*` 样式已随之删除（2026-10-15）。 */
/* ⚠️ **标签不再要上外边距**（2026-10-16）：面板已是两列网格（`app.css` 里 `.props-panel`），
   控件就在标签右边 ⇒ 那 6px 只会把每一行推高、还和 app.css 的 `margin:0` **同优先级打擂台**
   （scoped 注入更晚 ⇒ 实际生效的是这 6px，而几何夹具只加载 app.css、量不到 ⇒ 夹具比现实乐观）。 */
/* ⚠️ 只认**直接子元素**（与 `app.css` 的网格规则同口径）：包在 `.row` / `.pos-row` 这类
   自己排好布局的 div 里的标签**不动**（它们的 6px 本来就不该被这条规则连带清零）。 */
.props-panel > label { margin:0; }
.props-panel .row { margin-top:4px; }
.props-panel input[type="color"] { height: 26px; padding: 0 2px; }
.props-panel input, .props-panel select, .props-panel textarea { font-size: 12px; }
/* 只读类型展示：类型由轨道决定，这里不给下拉 */
.type-ro { display:flex; align-items:baseline; gap:6px; margin-top:6px; }
.type-ro b { font-size: 12.5px; }
/* 手工时间状态行（贴纸/运镜）：chip + 说明 +「跟随字幕」 */
.manual-row { display:flex; align-items:center; gap:6px; flex-wrap:wrap; }
/* 元素位置 X/Y 滑杆（2026-10-15）：`[X] ——o———— [ 76 ]` 一行。
   ⚠️ `min-width:0` 必须给：滑杆是 `flex:1 1 0`，父容器不让它收缩时
   数字框会被挤出右栏（右栏内容宽只有 266px，实测就够放这三格）。 */
.pos-lbl { margin-top:0 !important; }
.pos-row { display:flex; align-items:center; gap:6px; margin-top:5px; }
.pos-tag { flex:none; width:11px; font-size:11px; color:var(--ink-dim); text-align:center; }
.pos-range { flex:1 1 0; min-width:0; height:16px; margin:0; accent-color:var(--main); }
.pos-num { flex:none; width:52px; text-align:center; padding:2px 4px; }
.pos-row .mini-btn { flex:none; }
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
