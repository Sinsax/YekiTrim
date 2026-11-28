<template>
  <div class="card card-blk mb-12">
    <!-- 头部（计数 + 刷新）**始终可见**：收起时也能一眼看到库里有多少东西。
         折叠状态走 useDisclosure（localStorage `yekitrim.ui.home.mediaLibrary`），默认收起。 -->
    <div class="row spread mb-8">
      <button
        class="ml-head"
        type="button"
        data-cap="home.media-library"
        :aria-expanded="libOpen ? 'true' : 'false'"
        @click="toggleLib"
      >
        <Icon name="image" :size="15" />
        <b class="t m-0">素材库</b>
        <span class="muted small">{{ countText }}</span>
        <span class="muted small ml-open">{{ libOpen ? '收起' : '展开' }}</span>
        <Icon :name="libOpen ? 'chevron-down' : 'chevron-right'" :size="13" />
      </button>
      <div class="row" style="flex-wrap:nowrap">
        <button class="mini-btn" :disabled="loading" @click="load(true)">
          <span v-if="loading" class="spinner"></span> 刷新
        </button>
      </div>
    </div>

    <!-- **文件夹管理器布局**：左=类型导航（象形 + 名称 + 计数），右=该类型的素材网格。
         用户口径：「素材库ui改成文件夹管理器类似的感觉，左侧选择类型」「统一一下导入方式，
         都需要预览效果」。三块内容因此共用同一套"网格 + 点击放大 + 拖拽导入"交互 ——
         以前它们各写一套（视频只有一行文字、字体/贴纸各自成卡），同一个动作三种手感。 -->
    <div v-show="libOpen" class="ml-body">
      <!-- 音效页走**另一套布局**（用户口径 2026-11-27：「将左侧的内容换到右上，右上的内容顺延往下，
           左侧腾出空间为包」）：
             左 = **包列表**（占满整条左列，与贴纸页的包墙同一个心智）
             右上 = 选中组的变体 + 试听（原来在右栏的全套）
             右下 = **包内音效组**（原来在右栏上半、现在顺延往下）
           ⚠️ 类型导航在音效页要**并进左列顶部**（否则左列被包占了、切不回别的页签）。 -->
      <!-- 音效页 = **贴纸那种包卡片网格**（2026-11-27 用户口径）：
             左 = **包卡片网格**（一行一个包，与贴纸页的包墙同一个形状）
             右上 = 原来的类型导航（它从这里开始，往下顺延）
             右下 = 选中包里的音效组 → 再往下是变体试听
           ⚠️ 音效页**不走下面那条通用三栏**（导航|网格|右栏）：包是"素材本身"，
             所以它占的是**网格**那一格，不是右栏 —— 别把包做成窄竖列（那是上一版的错）。 -->
      <!-- **音效页 = 与贴纸页同一套三栏**（这是本仓的素材库布局，音效不许另造一套）：
             左  nav.ml-nav（128px 定宽）  = **音效库**列表：内建「小森平」+ 用户导入的包
             中  section.ml-main（自适应） = 当前库的**内容**：一级（大类）chips + 二级（小类）chips + 音效组
             右  aside.ml-side（clamp）   = 选中音效组的**变体 + 试听**
           ⚠️ **左侧是"库的位置"，不是分类**：大类/小类全在中间 → 右侧展开，左列永远只放库
              （用户原话：「因为日后需要添加新的音效包，你左边直接沾满时何意味」）。
           ⚠️ 别再用 `.ml-sfx-*` 那套平行样式：包卡片、标签栏、右栏都用贴纸页的
              `.ml-nav` / `.ml-main` / `.ml-side` / `.ml-tagbar`。 -->
      <template v-if="kind === 'sfx'">
        <nav class="ml-nav ml-nav-packs">
          <button
            v-for="p in sfxPacks"
            :key="p.id"
            class="ml-pack-card"
            :class="{ on: sfxPack === p.id }"
            :title="p.deletable ? `${p.name} —— ${p.count} 组（导入包，可删）` : `${p.name} —— ${p.count} 组（内建预设）`"
            @click="openPack(p.id)"
          >
            <span class="ml-pack-ico"><Icon name="volume" :size="17" /></span>
            <span class="ml-pack-txt">
              <span class="ml-pack-name">{{ p.name }}</span>
              <span class="muted small">{{ p.count }} 组{{ p.deletable ? ' · 导入' : ' · 内建' }}</span>
            </span>
            <!-- 「未构建」= 没有 `pack.toml`：能试听、能手选，但 **LLM 生成事件时读不到**
                 （候选只列有清单的包）。与贴纸页同一枚角标同一套口径。 -->
            <span
              v-if="p.deletable && p.built === false"
              class="ml-badge warn"
              title="还没构建：LLM 生成事件时读不到这个包（点右侧的「构建」）"
            >未构建</span>
            <span
              v-if="p.deletable"
              class="ml-pack-del"
              title="删掉这个导入包"
              @click.stop="doDeleteSfxPack(p.id)"
            >×</span>
          </button>
          <!-- 导入：库的位置长一个，不是把大类铺上来 -->
          <input ref="sfxPackInput" type="file" accept=".zip" hidden @change="onSfxPackPick">
          <button class="ml-pack-add" :disabled="sfxPackBusy" @click="sfxPackInput?.click()">
            <span v-if="sfxPackBusy" class="spinner"></span>
            <Icon v-else name="plus" :size="14" />
            <span>{{ sfxPackBusy ? '导入中…' : '导入音效包' }}</span>
          </button>
        </nav>

        <section class="ml-main">
          <div v-if="!sfxPack" class="ml-side-empty muted small">
            点左边的音效库，在这里浏览
          </div>
          <template v-else>
            <div class="row spread mb-8" style="flex-wrap:nowrap;gap:8px">
              <b class="m-0">{{ sfxPackName }}</b>
              <span class="muted small">{{ sfxShown.length }} / {{ sfxPackEntries.length }} 组</span>
            </div>

            <!-- **未构建提示 + 构建按钮**（导入包才有）：
                 没清单 = LLM 生成事件时读不到这个包。文案只说"是什么 / 点什么"，
                 细节（花时间与额度）进 `title`。 -->
            <div v-if="sfxPackIsUser && !sfxPackBuilt" class="ml-sfx-unbuilt">
              <span class="muted small">还没构建：LLM 生成事件时读不到这个包</span>
              <button
                class="mini-btn"
                :disabled="sfxBuildBusy"
                :title="`让 LLM 读文件名，从内建 ${sfxTagCount} 个 tag 里挑（要花时间与额度）`"
                @click="doBuildSfxPack"
              >
                <span v-if="sfxBuildBusy" class="spinner"></span>
                {{ sfxBuildBusy ? '构建中…' : '构建' }}
              </button>
            </div>

            <div class="row" style="flex-wrap:nowrap;gap:8px">
              <input
                v-model="sfxFilter"
                class="ml-sfx-filter"
                placeholder="筛选：名称 / id / tag / 分组"
              >
              <button v-if="sfxFilter || sfxCat || sfxGroup" class="mini-btn" @click="sfxFilter = ''; sfxCat = ''; sfxGroup = ''">清空</button>
            </div>

            <!-- **一级细分 = 大类**：横向滚动**单行**（不换行、不堆高度）。
                 8 个 chip 铺两行会把中间栏上半截吃掉，改成一行可横滚后高度固定 32px。 -->
            <div v-if="sfxCats.length > 1" class="ml-chipwrap mt-8">
              <div ref="sfxCatRow" class="ml-chiprow">
              <button class="ml-tag" :class="{ on: !sfxCat }" @click="sfxCat = ''; sfxGroup = ''">
                全部 <span class="ml-tag-n">{{ sfxLibTotal }}</span>
              </button>
              <button
                v-for="c in sfxCats"
                :key="c.cat"
                class="ml-tag"
                :class="{ on: sfxCat === c.cat }"
                :title="`只看「${c.cat}」`"
                @click="sfxCat = sfxCat === c.cat ? '' : c.cat; sfxGroup = ''"
              >{{ c.cat }} <span class="ml-tag-n">{{ c.count }}</span></button>
              </div>
              <button class="ml-chipnav" title="往左" @click="scrollChips(sfxCatRow, -1)">‹</button>
              <button class="ml-chipnav right" title="往右" @click="scrollChips(sfxCatRow, 1)">›</button>
            </div>

            <!-- **二级细分 = 小类 / zip 子目录**：同样单行横滚。31 个小类铺开会占五六行。 -->
            <div v-if="sfxGroups.length > 1" class="ml-chipwrap">
              <div ref="sfxGroupRow" class="ml-chiprow">
              <button class="ml-tag" :class="{ on: !sfxGroup }" @click="sfxGroup = ''">
                全部 <span class="ml-tag-n">{{ sfxCatEntries.length }}</span>
              </button>
              <button
                v-for="g in sfxGroups"
                :key="g.group"
                class="ml-tag"
                :class="{ on: sfxGroup === g.group }"
                :title="`只看「${g.group}」`"
                @click="sfxGroup = sfxGroup === g.group ? '' : g.group"
              >{{ g.group }} <span class="ml-tag-n">{{ g.count }}</span></button>
              </div>
              <button class="ml-chipnav" title="往左" @click="scrollChips(sfxGroupRow, -1)">‹</button>
              <button class="ml-chipnav right" title="往右" @click="scrollChips(sfxGroupRow, 1)">›</button>
            </div>

            <p v-if="!sfxShown.length && !loading" class="empty small mt-8">
              没有匹配的音效 —— 换个词或清掉筛选
            </p>
            <!-- **音效组 = 紧凑卡片网格**：音效没有画面可看，所以卡片只放
                 名字 + 分组 + 变体数，比行列表省一半竖向空间（一屏能看 30+ 组）。 -->
            <div v-else class="ml-sfx-cards mt-8">
              <button
                v-for="e in sfxShown"
                :key="e.id"
                class="ml-sfx-card"
                :class="{ on: preview && preview.id === e.id, off: e.blacklisted }"
                :title="`${e.id}${(e.tags || []).length ? ' · ' + e.tags.join(' / ') : ''}`"
                @click="pickSfxEntry(e)"
              >
                <span class="ml-sfx-card-name">{{ e.label || e.id }}</span>
                <!-- 副信息**两行**：分组一行、变体数一行。
                     ⚠️ 挤在一行时两段都会截断成「门·房间·… 1…」，什么都不剩。 -->
                <span class="ml-sfx-card-meta">
                  <span class="ml-sfx-card-sub muted small">{{ e.subgroup || e.group || '—' }}</span>
                  <span class="ml-sfx-card-n muted small">{{ (e.variants || []).length }} 变体</span>
                </span>
                <span v-if="e.blacklisted" class="ml-badge">已屏蔽</span>
              </button>
            </div>
          </template>
        </section>

        <aside class="ml-side ml-sfx-side">
          <div v-if="!preview || preview.type !== 'sfx'" class="ml-side-empty muted small">
            点中间的一条音效，在这里试听它的各个变体
          </div>
          <template v-else>
            <div class="row spread mb-8" style="flex-wrap:nowrap">
              <b class="m-0">{{ preview.name }}</b>
              <span class="muted small">{{ preview.files.length }} 个变体</span>
            </div>
            <!-- 分类与 tag 压成**一行**（原来是一整段，长 tag 会把标题区撑高） -->
            <p class="ml-sfx-meta muted small m-0 mb-8" :title="preview.tags.join(' / ')">
              {{ preview.cat }}<template v-if="preview.tags.length"> · {{ preview.tags.join(' · ') }}</template>
            </p>
            <!-- 变体 = 卡片：试听按钮在卡片右侧，点卡片本身也能播。
                 变体通常 1~5 条，列表形式会在宽栏里拉成一条细长线；卡片把它们收成方块。 -->
            <div class="ml-variant-cards">
              <button
                v-for="(f, i) in preview.files"
                :key="f.stem"
                class="ml-variant-card"
                :class="{ on: sfxPlaying === f.stem }"
                :disabled="sfxBusy === f.stem"
                :title="f.stem"
                @click="playVariant(f)"
              >
                <span class="ml-variant-card-i">{{ i + 1 }}</span>
                <span class="ml-variant-card-name mono">{{ f.stem }}</span>
                <Icon :name="sfxPlaying === f.stem ? 'pause' : sfxBusy === f.stem ? 'clock' : 'play'" :size="13" />
                <span class="ml-variant-card-state">
                  {{ sfxBusy === f.stem ? '抓取中' : sfxPlaying === f.stem ? '停止' : '试听' }}
                </span>
              </button>
            </div>
            <p v-if="sfxError" class="error-banner mt-8">{{ sfxError }}</p>
            <div class="row mt-12" style="flex-wrap:nowrap;gap:8px">
              <button class="mini-btn" :disabled="busy" @click="toggleBlacklist(preview)">
                {{ preview.blacklisted ? '解除屏蔽' : '屏蔽这一组' }}
              </button>
              <span class="muted small">
                {{ preview.blacklisted
                  ? '当前被屏蔽：LLM 不会选它，规则命中也会被跳过'
                  : '屏蔽后 LLM 与兜底规则都不再选它（可随时解除）' }}
              </span>
            </div>
          </template>
        </aside>
      </template>
      <template v-else>
      <nav class="ml-nav">
        <button
          v-for="t in TABS"
          :key="t.key"
          class="ml-tab"
          :class="{ on: kind === t.key }"
          :title="t.hint"
          @click="switchKind(t.key)"
        >
          <Icon :name="t.icon" :size="15" class="ml-ico" />
          <span class="ml-lab">{{ t.label }}</span>
          <span class="ml-cnt">{{ counts[t.key] }}</span>
        </button>

        <!-- 字体页的「样例文字」放**左侧类型栏**（用户口径：字体不需要右侧栏，
             样例文字输入框放左侧即可）—— 它是"这一类素材的显示参数"，跟着导航走比
             挤在右上角更顺：选了字体、视线不用跨过整个网格去改样例。 -->
        <label v-if="kind === 'font'" class="ml-sample-wrap">
          <span class="muted small">样例文字</span>
          <input
            v-model="testText"
            class="ml-sample"
            placeholder="留空 = 永 Aa 123"
          >
        </label>
      </nav>

      <section class="ml-main">
        <div class="row spread mb-8">
          <span class="muted small">{{ hint }}</span>
          <div class="row" style="flex-wrap:nowrap">
            <!-- 音效页没有导入（内建预设、不可增删）：`importLabel` 给空串 ⇒ 整颗按钮不渲染 -->
            <button v-if="importLabel" class="mini-btn primary" :disabled="busy" @click="fileInput?.click()">
              <Icon name="plus" :size="13" /> {{ importLabel }}
            </button>
          </div>
        </div>

        <!-- 音效页的筛选与大类栏**不在这里**：音效走上面那套独立布局
             （左=包墙 / 右上=变体 / 右下=包内组），筛选框贴包墙那一侧。
             这里只服务 视频 / 贴纸 / 字体 三页。 -->

        <!-- **统一导入区**：三种素材都是"拖进来 / 点选文件"。
             ⚠️ 视频**没有上传**（它可能是几个 GB）—— 拖拽只用来提示放哪个目录，
             真正的入口是"放进目录 + 刷新"，所以这里对视频显示目录而不是打开文件框。
             这个差别必须显式写出来，否则用户拖一个 2GB 视频进来会以为界面坏了。
             ⚠️ 音效页**整块不渲染**（内建预设：既没有目录可看、也没有文件可收）——
                给它留一块只写"不可拖入"的空框比没有更让人困惑。 -->
        <div
          v-if="kind !== 'sfx'"
          class="ml-drop"
          :class="{ over: dragOver, off: kind === 'video' }"
          @dragover.prevent="onDragOver"
          @dragleave.prevent="onDragLeave"
          @drop.prevent="onDrop"
          @click="kind === 'video' ? null : fileInput?.click()"
        >
          <input ref="fileInput" type="file" :accept="accept" multiple hidden @change="onFilePick">
          <Icon :name="kind === 'video' ? 'folder' : 'plus'" :size="16" />
          <div class="ml-drop-t">
            <b v-if="kind === 'video'">视频直接放进这个目录，再点「刷新」</b>
            <b v-else-if="busy">{{ importLabel }}中…（{{ doneCount }}/{{ totalCount }}）</b>
            <b v-else>{{ dropText }}</b>
            <span class="hint ml-dir mono">{{ root || '读取中…' }}</span>
          </div>
        </div>

        <div v-if="error" class="error-banner mt-8"><Icon name="alert" :size="15" /> {{ error }}</div>

        <p v-if="!items.length && !loading && !error" class="empty small mt-8">
          {{ kind === 'sfx' && (sfxFilter.trim() || sfxCat)
            ? '没有匹配的音效包 —— 换个词试试，或点「全部」去掉大类筛选'
            : emptyText }}
        </p>

        <!-- **统一预览网格**：缩略图 + 点击放大。视频的缩略图是一张真帧（服务端截），
             所以四种素材在这里长得一样，用户不用学四种看法。
             网格上方那排小按钮 = 该类型的**管理动作**（默认 / 构建 / 屏蔽 / 删除），
             悬停或聚焦该格才出现 —— 常显会把 108px 的格子塞满，而它们是低频操作。 -->
        <div v-else class="ml-grid mt-8">
          <div
            v-for="it in items"
            :key="it.key"
            class="ml-cell-wrap"
            :class="{ sel: preview && preview.type === it.type && preview.name === it.name }"
          >
            <button class="ml-cell" :title="it.title" @click="open(it)">
              <span class="ml-thumb">
                <!-- 字体：没有缩略图，直接用该字体现场渲染样例字（就是"预览效果"本身） -->
                <span
                  v-if="kind === 'font'"
                  class="ml-font"
                  :style="{ fontFamily: it.stack }"
                >{{ it.sample }}</span>
                <!-- 视频：服务端截的帧。截不出来（容器怪异/没装 ffmpeg）时退成图标，别留白块。
                     `@error` 把 thumb 清空 ⇒ 自动落到下面那个 Icon 分支，不在界面上留一个裂图。 -->
                <img v-else-if="it.thumb" :src="it.thumb" alt="" loading="lazy" @error="it.thumb = ''">
                <!-- 音效：没有画面可看 ⇒ 用喇叭图标（`it.variants` 有值的那一档），别的类型仍用图片图标 -->
                <Icon
                  v-else
                  :name="kind === 'video' ? 'video' : it.type === 'sfx' ? 'volume' : 'image'"
                  :size="22"
                />
                <span v-if="it.badge" class="ml-badge">{{ it.badge }}</span>
                <!-- ⚠️ 第二个徽标位（2026-10-16）：字体页的「无粗体」与「默认」可能同时成立，
                     挤进同一个 `it.badge` 会丢掉一条信息。解释一律进 `title`（行内 ≤20 字）。 -->
                <span v-if="it.badge2" class="ml-badge warn" :title="it.badge2Hint">{{ it.badge2 }}</span>
              </span>
              <span class="ml-name">{{ it.name }}</span>
              <span class="ml-sub muted">{{ it.sub }}</span>
            </button>
            <div v-if="it.actions && it.actions.length" class="ml-acts">
              <button
                v-for="a in it.actions"
                :key="a.label"
                class="ml-act"
                :title="a.hint"
                :disabled="busy"
                @click.stop="a.run()"
              >{{ a.label }}</button>
            </div>
          </div>
        </div>
      </section>

      <!-- **右侧预览栏**（用户口径 2026-10-10 最终版）：
           · 视频 —— 点左边在这里**直接播放**，并显示文件名/时长/大小；
           · 贴纸 —— 在这里**列表显示**整包；
           · 字体 —— **没有右栏**（样例文字输入框在左侧），字体用网格里现场渲染的样例看效果。
           ⚠️ 音效页**不走这条右栏**（2026-11-27 改版）：它有自己的三栏布局
              （左=包墙 / 右上=变体 / 右下=包内组），见模板开头那个 `kind === 'sfx'` 分支。
           ⚠️ 显示条件是 `kind !== 'font'`（两个类型共用一条右栏，靠 `kind` 切内容）。
           ⚠️ 用户中途试过"视频改弹窗、右栏只留贴纸"，又改回来了 —— **别再自动改回去**。 -->
      <aside v-if="kind !== 'font' && kind !== 'sfx'" class="ml-side">
        <template v-if="kind === 'video'">
          <div v-if="!preview" class="ml-side-empty muted small">
            点左边的视频，在这里预览播放
          </div>
          <template v-else>
            <!-- **外围框 + 里面按比例的视频**（用户口径：「加外围框，其实不冲突吧」—— 确实不冲突）：
                 框负责 `min-height` / `max-height`，视频在框内按自身比例撑满，
                 互不干涉 ⇒ 既能限制高度，又不会因为"框比视频高"而留下黑边。 -->
            <div class="ml-video-frame" :style="videoFrameInline">
              <!-- ⚠️ 比例**由视频自己报**（`loadedmetadata` 时读 `videoWidth/videoHeight`），
                   不写死 —— 以前写死 `16/9`，而库里的 `op.mp4` 是 320×240(4:3) ⇒ 框就是错的。
                   `@loadedmetadata` 而不是 `mounted`：那时才有真实分辨率。 -->
              <video
                :src="preview.url"
                controls
                class="ml-side-video"
                @loadedmetadata="onVideoMeta($event)"
              ></video>
            </div>
            <!-- 用户口径：「信息作为两排排版」（原来 9 行一路竖着排，又把栏撑高、又不好扫）。
                 **两列**：左列是"这个文件是什么"（名称/文件/时长/大小），
                 右列是"它有什么技术规格"（分辨率/编码/帧率/码率/音轨）—— 分栏本身就是分组。
                 ⚠️ 每一项都可能是 `—`：探不出来时**如实显示 —**，不留空、更不编一个数。 -->
            <div class="ml-side-info">
              <div class="ml-info-col">
                <div class="ml-info-row"><span class="muted">名称</span><b>{{ preview.name }}</b></div>
                <div class="ml-info-row"><span class="muted">文件</span><span class="mono ml-ell" :title="preview.file">{{ preview.file }}</span></div>
                <div class="ml-info-row"><span class="muted">时长</span><b>{{ preview.durationText }}</b></div>
                <div class="ml-info-row"><span class="muted">大小</span><b>{{ preview.sizeText }}</b></div>
              </div>
              <div class="ml-info-col">
                <div class="ml-info-row"><span class="muted">分辨率</span><b>{{ preview.dimText }}</b></div>
                <div class="ml-info-row"><span class="muted">编码</span><b>{{ preview.codecText }}</b></div>
                <div class="ml-info-row"><span class="muted">帧率</span><b>{{ preview.fpsText }}</b></div>
                <div class="ml-info-row"><span class="muted">码率</span><b>{{ preview.bitrateText }}</b></div>
                <div class="ml-info-row"><span class="muted">音轨</span><b>{{ preview.audioText }}</b></div>
              </div>
            </div>
          </template>
        </template>


        <template v-else>
        <div v-if="!preview" class="ml-side-empty muted small">
          点左边的贴纸包，在这里列出包内所有图
        </div>
        <template v-else>
          <div class="row spread mb-8" style="flex-wrap:nowrap">
            <b class="m-0">{{ preview.name }}</b>
            <span class="muted small">
              <!-- 筛选中时把"看到的 / 总共"都说出来 —— 只显示一个数会让人以为图少了 -->
              {{ preview.activeTag ? `${shownFiles.length} / ${preview.files.length} 张` : `${preview.files.length} 张` }}
            </span>
          </div>

          <!-- **标签筛选**：点一个标签只看那一类，再点一次取消。
               标签是**包内文件自己的** tags 聚合出来的（见 `selectItem`），
               所以"有哪些标签"永远和"图实际带了什么标签"一致。 -->
          <div v-if="preview.tagList.length" class="ml-tagbar">
            <button
              class="ml-tag"
              :class="{ on: !preview.activeTag }"
              @click="preview.activeTag = null"
            >全部 <span class="ml-tag-n">{{ preview.files.length }}</span></button>
            <button
              v-for="t in preview.tagList"
              :key="t"
              class="ml-tag"
              :class="{ on: preview.activeTag === t }"
              :title="`只看带「${t}」的图`"
              @click="preview.activeTag = preview.activeTag === t ? null : t"
            >{{ t }} <span class="ml-tag-n">{{ tagCount(t) }}</span></button>
          </div>

          <p v-if="!preview.files.length" class="muted small m-0">这个包里没有图片。</p>
          <p v-else-if="!shownFiles.length" class="muted small m-0">
            这个包里没有带「{{ preview.activeTag }}」标签的图。
          </p>
          <!-- ⚠️ `loading="lazy"`：一个包可能有 200+ 张（ysyk 就是 210），
               不懒加载会一次性全解码，整页卡住。 -->
          <div v-else class="ml-side-grid">
            <img
              v-for="f in shownFiles"
              :key="f.name"
              :src="f.url"
              :alt="f.name"
              :title="f.tags.length ? `${f.name} · ${f.tags.join(' / ')}` : f.name"
              loading="lazy"
              @click="openStickerImage(f)"
            >
          </div>
        </template>
        </template>
      </aside>
      </template>
    </div>

    <!-- 音效试听播放器：**一个** `<audio>` 复用（同时只响一条）。
         ⚠️ 隐藏但要留在 DOM 里 —— 用 `Audio()` 对象自建会绕开这里的错误提示。
         `preload="none"`：只在用户点「试听」时才设 src，别在页面加载时就去抓 1213 个源。 -->
    <audio
      ref="sfxAudio"
      preload="none"
      @ended="sfxPlaying = ''"
      @error="sfxPlaying = ''"
    ></audio>

    <!-- 点击放大：视频真播（用 `<video>` + Range，服务端那条取文件路已支持）、
         字体放大看字形、贴纸看大图。三者在同一个弹窗里，位置/关闭方式一致。 -->
    <AppModal v-if="zoom" :title="zoom.title" @close="zoom = null">
      <div class="ml-zoom">
        <video
          v-if="zoom.type === 'video'"
          :src="zoom.url"
          controls
          autoplay
          class="ml-zoom-video"
        ></video>
        <img v-else-if="zoom.type === 'sticker'" :src="zoom.url" alt="" class="ml-zoom-img">
        <div v-else class="ml-zoom-font">
          <p class="ml-zoom-line" :style="{ fontFamily: zoom.stack }">{{ zoom.sample }}</p>
          <p class="ml-zoom-meta muted small">{{ zoom.meta }}</p>
        </div>
        <p v-if="zoom.detail" class="hint m-0 mt-8">{{ zoom.detail }}</p>
      </div>
    </AppModal>
  </div>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import Icon from './Icon.vue'
import AppModal from './AppModal.vue'
import { api } from '../api/client'
// 首页素材库默认**收起**（只留头部一行计数），展开状态持久化。
import { useDisclosure } from '../composables/useDisclosure'
// ⚠️ 内建表情的 CDN 地址**只有一份**（engine.js::emojiGifUrl，与出片模板那句逐字同形）——
//    别在这里再拼一遍 'https://fonts.gstatic.com/...'。
import { emojiGifUrl } from '../polish/engine'
import {
  assetUrl,
  deleteFont,
  FONT_UPLOAD_MAX_BYTES,
  setDefaultFont,
  uploadFont,
  videoThumbUrl,
} from '../api/polish'
// 音效页要的三条（2026-11-27）：目录（连被屏蔽的一起）、试听 URL、屏蔽写回。
// ⚠️ 试听**必须**走 `sfxPreviewUrl`（服务端抓取并缓存到 <data>/cache/sfx/），
//    别在这里拼 taira-komori.net 的地址 —— 站方要求不转载音效 URL。
import { fetchSfxCatalog, putSfxBlacklist, sfxPreviewUrl } from '../api/polish'
import { watchJob } from '../api/jobs'
import { stickersApi, DEFAULT_UPLOAD_MAX_BYTES } from '../api/stickers'
// ⚠️ **`hasBoldFace` 必须一起引**（2026-10-18 真 bug）：字体分支的「无粗体」徽标用它算判据，
//    而这里以前只引了 `ensureFontFace` ⇒ 切到「字体」页时 `items` 抛 ReferenceError ⇒
//    **Vue 把整块子树卸掉，素材库直接消失**（用户报的就是这个）。
//    ⚠️ 静态门禁抓不到这一类：esbuild 不为未定义全局变量报错，而
//    `check-layout-invariants.mjs` 那条「模板标识符必须有定义」只扫**模板**，这里是 `<script setup>` 里。
//    兜它的是 headless 的 `webui/scripts/check-polish-page-loads.mjs`（含素材库三个页签）。
import { ensureFontFace, hasBoldFace } from '../polish/fonts'
import { confirmDialog } from '../stores/dialog'
import { toast } from '../stores/toast'

defineProps({ isAdmin: { type: Boolean, default: false } })

/** 首页整块折叠（默认收起）。body 用 `v-show`，收起时仍保留已加载状态与选中项。 */
const { open: libOpen, toggle: toggleLib } = useDisclosure('home.mediaLibrary', false)

const TABS = [
  { key: 'video', label: '视频', icon: 'video', hint: '片头 / 片尾的素材来源' },
  { key: 'sticker', label: '贴纸', icon: 'image', hint: '精修时 LLM 按 tag 选的贴纸' },
  // 音效（2026-11-27）：内建**预设**库，不可删除、不可上传 —— 它是编译期内嵌的目录
  // （`assets/sfx_catalog.json`，1213 组），点开条目在这里试听、可屏蔽整组。
  // ⚠️ **别再给它加"导入/删除"**：这一版刻意只做预设（用户口径），
  //    自定义音效包是下一步，届时要先定包协议与存储位置。
  { key: 'sfx', label: '音效', icon: 'volume', hint: '内建音效库：试听 / 屏蔽（不可删除）' },
  { key: 'font', label: '字体', icon: 'type', hint: '字幕与封面可选的字体' },
]
const kind = ref('video')
const tab = computed(() => TABS.find((t) => t.key === kind.value) || TABS[0])
const hint = computed(() => tab.value.hint)
/** 音效页没有"导入"（那一页只读、只做筛选与试听）——给空串，模板据此不渲染主按钮。 */
const importLabel = computed(
  () => ({ video: '导入视频', sticker: '导入贴纸包', font: '导入字体' })[kind.value] || '',
)
const dropText = computed(
  () => ({ sticker: '把 zip 压缩包拖到这里（或点选文件）', font: '把字体文件拖到这里（或点选文件）' })[kind.value] || '',
)
const emptyText = computed(
  () => ({
    video: '还没有视频素材 —— 把 mp4 / webm / mkv 放进上面那个目录，再点「刷新」',
    sticker: '还没有贴纸包 —— 点「导入贴纸包」上传一个 zip（里面是图片）',
    font: '还没有字体 —— 点「导入字体」上传 .ttf / .otf',
    sfx: '音效库读取中…（它是内建目录，正常情况下一定有内容）',
  })[kind.value],
)
const accept = computed(() => (kind.value === 'font' ? '.ttf,.otf' : kind.value === 'sticker' ? '.zip' : '*'))

/** 样例文字：空 = 各卡片用默认样张。中英数字混排才看得出字形/字重差异。 */
const testText = ref('')
const SAMPLE = '永 Aa 123'
const sampleOf = () => testText.value.trim() || SAMPLE

/** 三种素材各自的列表状态（切类型不互清 —— 切回来是即时的）。 */
const videos = ref([])
const packs = ref([])
/** 音效预设库（2026-11-27）：**连被屏蔽的一起**拿（`include_blacklisted`）——
 *  这一页的职责之一就是"看见并取消屏蔽"，服务端默认过滤掉的那份会让人无从解除。
 *  `sfxBlacklist` 是服务端回的**当前那份**（整份覆盖写的基线，见 `toggleBlacklist`）。 */
const sfxEntries = ref([])
const sfxBlacklist = ref([])
/** 试听播放器：**一个** `<audio>` 复用（同时只响一条，换一条先停旧的）。
 *  ⚠️ 不用 `Audio()` 对象自己管：那会绕开模板里的错误提示，失败时用户只看到"点了没反应"。 */
const sfxAudio = ref(null)
const sfxPlaying = ref('')   // 正在响的变体键
const sfxBusy = ref('')      // 正在向服务端取字节的那一条（首次点会抓源站，几百毫秒）
const sfxError = ref('')
/** 音效页的筛选词。1213 组平铺进网格是**没法用**的（一屏十几格、要翻几十屏），
 *  所以这一页必须能按词收窄。匹配 id / 中文名 / tag / 大类（与编辑器 `SfxPicker` 同口径）。 */
const sfxFilter = ref('')
/** 当前点开的**音效包**（= 服务端 `pack` 字段，即 catalog 的 `subcat`）。
 *
 *  音效页按**包**分组，和贴纸页同一个模型：网格里一行一个包，点开在右栏列这一包里的音效组，
 *  再点一条才列变体试听。`null` = 还没点开任何包。
 *
 *  ⚠️ 别退回"1213 组平铺"：那正是这一版改掉的东西（120 格上限就是为了压住它）。
 *  ⚠️ 包名走服务端下发的 `pack`（`sfx_catalog::pack_of` 是唯一兜底处），
 *     前端**不许**自己写 `subcat || '其他'` —— 两处各写一次必然漂成两个包名。 */
const sfxPack = ref(null)
/** 左栏的**大类**筛选（catalog 的 `cat`，7 个）。`''` = 全部大类。 */
const sfxCat = ref('')
/** **音效库列表**（`GET /api/polish/sfx-catalog` 的 `packs`）：
 *  内建预设「小森平」一条 + 用户导入的各一条。左侧只渲染它 —— 左侧是"库的位置"。 */
const sfxPacks = ref([])
/** 当前包内的**细分组**筛选（贴纸页那排 tag）。`''` = 整包。 */
const sfxGroup = ref('')
/** 音效 tag 词表（`GET /sfx-catalog` 顶层 `tags`，`[(tag, 组数)]`）。
 *
 *  ⚠️ 它是**内建封闭词表**：用户包的 tag 只能从这里面选（构建时的提示词与校验都读它）。
 *  这里只用于显示「从 N 个 tag 里挑」，**不做任何本地过滤**（校验唯一一份在 Rust）。 */
const sfxTags = ref([])
/** 导入包的构建上传态（与导入同一套：无进度条，只有「构建中…」）。 */
const sfxBuildBusy = ref(false)
/** 导入包的上传态（与贴纸上传同一套：无进度条，只有「导入中…」）。 */
const sfxPackBusy = ref(false)
const sfxPackInput = ref(null)
/** 两级 chips 行（横向滚动钮用；两个 ref 分开，不能共用一个）。 */
const sfxCatRow = ref(null)
const sfxGroupRow = ref(null)
/** chips 行横滚一屏的 70%（‹ › 按钮）。 */
function scrollChips(el, dir) {
  if (!el) return
  el.scrollBy({ left: dir * el.clientWidth * 0.7, behavior: 'smooth' })
}
/** 内建表情目录：`[{tag, codepoints:[…]}]` —— 881 条按 **85 个 tag** 分组，
 *  而**同一个码点会挂在多个 tag 下**（那是分类视图，不是互斥分区）。 */
const emojiGroups = ref([])
/** 包封面：`{包名: 图片URL}`。**列表接口不带文件名**（只回 `count`），所以封面要现拉 detail。
 *
 * ⚠️ 缓存在这里、**每个包只拉一次**：`items` 是 computed，每次重算都会调 `coverOf`，
 * 不缓存就是"每渲染一次发一轮请求"（素材库一多就是几十连发）。
 * ⚠️ 拉失败**留空**（渲染成占位），不要拿别人的图顶上 —— 张冠李戴比没有图更难发现。 */
const covers = reactive({})
function coverOf(pack) {
  // 内建包：封面就是第一张动图（没有 detail 可拉，也别去拉 —— 那是一次注定 404 的请求）
  if (pack === EMOJI_PACK) return emojiCover.value
  if (covers[pack] !== undefined) return covers[pack]
  covers[pack] = '' // 占位，同时充当"已在拉"的标记
  stickersApi
    .detail(pack)
    .then((d) => {
      const first = (d?.files || [])[0]?.name
      covers[pack] = first ? stickersApi.fileUrl(pack, first) : ''
    })
    .catch(() => {
      covers[pack] = ''
    })
  return ''
}
/* ---------------- 右侧预览栏 ---------------- */

/** 当前选中的素材（视频对象 / 贴纸包对象）。`null` = 还没选。 */
const preview = ref(null)

/** 切类型：**顺手清掉选中**。不清的话，从"贴纸"切到"视频"时右栏还挂着上一个贴纸包
 * （右栏是按 `kind` 渲染的，数据却是上一个类型的）—— 会显示成"视频页里列着一堆图"。 */
function switchKind(k) {
  if (kind.value === k) return
  // 离开音效页先停掉试听：否则切走之后那条还在响，而界面上没有任何"正在播"的指示。
  stopSfx()
  kind.value = k
  // 右栏服务视频与贴纸；切到字体时整条不渲染，而且两类数据形状不同 ——
  // 不清的话从贴纸切到视频，右栏会拿着 `{files:[…]}` 去渲染 `<video>`。
  preview.value = null
  // 音效页的"当前包"也要清：从别的页切回来时不该还记得上次点开了哪个包
  // （用户会以为那是默认展开的）。大类筛选**保留** —— 它更像"我正看这一类"的意图。
  if (k === 'sfx') {
    sfxCat.value = ''
    sfxGroup.value = ''
    // 切进来就落在第一个库（内建「小森平」）—— `load()` 只在 mount 时跑，
    // 那时 `kind` 还是 video，所以默认选中必须在这里补，否则中间一栏是空的。
    if (!sfxPack.value && sfxPacks.value.length) sfxPack.value = sfxPacks.value[0].id
  }
}

/** 秒 → `m:ss`（视频时长）。探不出来时给 `—`，**不猜**。 */
function fmtDuration(sec) {
  if (!sec || sec <= 0) return '—'
  const t = Math.round(sec)
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`
}

/** 当前视频的宽高比（`null` = 还没拿到，用 16:9 兜底免得框塌成 0 高）。 */
const videoRatio = ref(null)

/** 框的内联样式：把视频比例写给**框**。
 *
 * ⚠️ **比例必须落在框上，不能只靠视频自己撑**：`.ml-side` 是 column flex，
 * 框没有确定高度时只写 `min/max-height` 会让它缩到 `min-height`（实测被钉在 200px，
 * 加宽栏完全没效果）。给框一个 `aspect-ratio` 之后高度就有依据了，
 * 再由 `min/max-height` 夹住上下限。
 *
 * ⚠️ 换视频时要**重置**，否则新视频先按上一个的比例闪一下（`null` ⇒ 16:9 兜底）。 */
const videoFrameInline = computed(() => {
  const r = videoRatio.value
  if (!r) return {}
  const style = { aspectRatio: String(r) }
  // ⚠️ **只有"不窄于 1:1"的视频才给宽度下限**：竖屏需要的是"窄而高"的框
  // （宽度由 `max-height` 反推），强行给 `min-width` 会让框比视频宽 ⇒ 两侧黑边（实测 24px）。
  if (r >= 1) style.minWidth = 'min(100%, 380px)'
  return style
})

/** `<video>` 报到真实分辨率 ⇒ 记下比例。拿不到就保持 `null`（兜底 16:9）。 */
function onVideoMeta(e) {
  const v = e?.target
  if (v?.videoWidth > 0 && v?.videoHeight > 0) {
    videoRatio.value = v.videoWidth / v.videoHeight
  }
}

/** 帧率 → `30 fps` / `29.97 fps`。**整数不显示小数**（`30 fps` 比 `30.00 fps` 清爽），
 * 探不出来给 `—`，**不猜**（`0 fps` 是编的）。 */
function fmtFps(v) {
  if (!v || v <= 0) return '—'
  // 取整后再比：`29.9999` 这种浮点误差不该显示成 `29.9999 fps`
  return `${Math.abs(v - Math.round(v)) < 0.005 ? Math.round(v) : v.toFixed(2)} fps`
}

/** 点一个贴纸包 → 拉整包图，填进右栏。
 *
 * ⚠️ **只服务贴纸**：视频已改回弹窗（用户口径），不在这里处理 —— 别再把视频分支加回来，
 * 那会让"右栏"同时承担两种形态。
 * ⚠️ **贴纸要现拉 detail**（列表只有 `count`）：整包的图只在这一刻拉一次。
 * ⚠️ 落结果前核对 `preview.value?.name`，否则点得快时慢响应会覆盖新选中。 */
async function selectItem(it) {
  // 音效：右栏列变体 + 试听。**没有 detail 接口要拉**（列表里已经带 `variants`）⇒ 同步填。
  // ⚠️ 走的是包内**组**（`pickSfxEntry`），网格里那层是**包**、由 `open` 直接处理。
  if (it.type === 'sfx') {
    pickSfxEntry(it)
    return
  }
  if (it.type === 'video') {
    videoRatio.value = null // 换视频先清比例，等 loadedmetadata 再报（避免按上一个的比例闪一下）
    // ⚠️ **别拿 `it.key` 去 `videos` 里反查** —— `videos.value` 里放的是**接口原始对象**
    // （只有 `{name, file, duration}`，**没有 `key`**），`it.key` 是 `items` 计算时才造出来的。
    // 曾经写成 `videos.value.find(x => x.key === it.key)` ⇒ 永远 `undefined` ⇒
    // **点了视频右栏什么都不出现**。「有字段名就读」之前先 `Object.keys()` 看一眼。
    // `it` 自己就带齐了要用的字段，直接用。
    if (typeof it.file !== 'string' || !it.file) return
    preview.value = {
      type: 'video',
      name: it.name,
      file: it.file,
      url: it.url,
      durationText: fmtDuration(it.duration),
      sizeText: '…',
      // 技术信息直接来自列表接口（服务端已一次 ffprobe 探全），点选无延迟。
      // ⚠️ 每一项都可能缺失 ⇒ 显示侧逐项兜底成 `—`，**不要在这里编默认值**。
      dimText: it.width && it.height ? `${it.width}×${it.height}` : '—',
      codecText: it.codec ? String(it.codec).toUpperCase() : '—',
      fpsText: fmtFps(it.fps),
      bitrateText: it.bitrate ? `${(it.bitrate / 1e6).toFixed(1)} Mbps` : '—',
      audioText: it.has_audio === true ? '有' : it.has_audio === false ? '无（静音）' : '—',
    }
    // 大小是"锦上添花"：拿不到就显示 —，**不能因为探不到就让整条选中失败**。
    // ⚠️ 落结果前核对还是不是同一条，防止点得快时慢响应覆盖新选中。
    try {
      const res = await fetch(it.url, { method: 'HEAD' })
      const len = Number(res.headers.get('content-length') || 0)
      if (preview.value?.file === it.file) preview.value.sizeText = fmtBytes(len)
    } catch {
      if (preview.value?.file === it.file) preview.value.sizeText = '—'
    }
    return
  }
  // ⚠️ **内建 `emoji` 包没有 detail 接口可拉**（磁盘上没有这个包）—— 包内文件现从目录造，
  //    形状与下面那条**完全一致**（`{name, url, tags}`）⇒ 右栏那份渲染一个字都不用改，
  //    标签筛选栏也白拿（你那 85 个分类各自命中多少张，都是现算的）。
  //    ⚠️ 别在这里也走一次 `stickersApi.detail('emoji')` —— 那是一次注定 404 的请求。
  if (it.emoji) {
    preview.value = { type: 'sticker', name: EMOJI_PACK, files: [], tagList: [], activeTag: null, builtin: true }
    // **必须是 CDN 动图**（与出片模板的 CDN 回退、`engine.js::emojiGifUrl` 同一个地址）——
    // 静态字符那一版正是用户报的「polish编辑器内有，但是是静态的」。
    preview.value.files = emojiAll.value.map((e) => ({ name: e.cp, url: emojiGifUrl(e.cp), tags: e.tags }))
    const n = {}
    for (const f of preview.value.files) for (const t of f.tags) n[t] = (n[t] || 0) + 1
    preview.value.tagList = Object.keys(n).sort((a, b) => n[b] - n[a] || a.localeCompare(b, 'zh'))
    return
  }
  preview.value = { type: 'sticker', name: it.name, files: [], tagList: [], activeTag: null }
  try {
    const d = await stickersApi.detail(it.name)
    if (preview.value?.name !== it.name) return // 期间又点了别的包 ⇒ 丢弃这次结果
    // ⚠️ **把 `tags` 一起带上**（detail 接口本来就回了，以前丢掉了）——
    // 右栏的标签筛选全靠它。`tags` 缺失时给 `[]`，那张图就只在"全部"里出现。
    preview.value.files = (d?.files || []).map((f) => ({
      name: f.name,
      url: stickersApi.fileUrl(it.name, f.name),
      tags: Array.isArray(f.tags) ? f.tags : [],
    }))
    // 标签清单**从包内文件现算**，不用列表接口那份 —— 两份数据来源不同会打架
    // （列表的 `tags[].count` 是"命中文件数"，这里要的是"有哪些标签"）。
    // 顺序按命中数降序、同数按字面，保证同一份数据每次渲染顺序一致。
    const n = {}
    for (const f of preview.value.files) for (const t of f.tags) n[t] = (n[t] || 0) + 1
    preview.value.tagList = Object.keys(n).sort((a, b) => n[b] - n[a] || a.localeCompare(b, 'zh'))
  } catch (e) {
    if (preview.value?.name === it.name) {
      preview.value.files = []
      toast('读取包内图片失败：' + e.message, 'err')
    }
  }
}

/** 右栏当前该显示的贴纸（按标签筛过）。`activeTag` 为空 ⇒ 整包。 */
const shownFiles = computed(() => {
  const p = preview.value
  if (!p || p.type !== 'sticker') return []
  if (!p.activeTag) return p.files
  return p.files.filter((f) => f.tags.includes(p.activeTag))
})

/** 某个标签在这个包里命中多少张（标签栏上那个小数字）。 */
function tagCount(t) {
  const p = preview.value
  if (!p || p.type !== 'sticker') return 0
  return p.files.filter((f) => f.tags.includes(t)).length
}

/** [`coverOf`] 的 await 版（点开时用：要拿到**确定**结果，不能拿占位空串）。 */
async function coverOfAsync(pack) {
  if (covers[pack]) return covers[pack]
  try {
    const d = await stickersApi.detail(pack)
    const first = (d?.files || [])[0]?.name
    covers[pack] = first ? stickersApi.fileUrl(pack, first) : ''
  } catch {
    covers[pack] = ''
  }
  return covers[pack]
}
const fonts = ref([])
// ⚠️ **每种素材各有各的根**，别只留一个 —— 视频/字体/贴纸的库是三棵不同的树
// （`<数据>/resources/{videos,fonts,stickers}`，多用户下再套一层 `users/<uid>/`）。
// 以前这里只有一份 `root`、且**永远取 video-library 那个**，于是切到"字体"页时
// 提示语照旧显示视频目录（"把字体文件拖到这里"下面写着 `…\resources\videos`）——
// **只是显示错了，写入是对的**（后端各按各的 kind 落盘）。这类"看着不对但不报错"
// 最容易让人以为传错地方，所以三个根分开存。
const roots = reactive({ video: '', font: '', sticker: '' })
const root = computed(() => roots[kind.value] || '')
const loading = ref(false)
const error = ref('')
/** **内建 emoji 包**：不是磁盘上的目录，而是编译期内嵌的 `assets/emoji_catalog.json`
 *  （881 条，走 `GET /api/polish/emoji-catalog`）；`emojiAll` 是它**去重后**的全量
 *  （码点 → 它挂着的那些 tag）。
 *
 *  ⚠️⚠️ **它是「贴纸页里的一个包」，不是第五种素材类型**（2026-10-17 用户订正：
 *  「emoji 这个表情是作为贴纸包存在的，你怎么给我单开一页？」）。编辑器那边它就是这么存在的 ——
 *  `StickerPicker` 的包列表里有一行固定的 `emoji`（「表情 · 内建」）。所以素材库照同一个模型：
 *  **贴纸页多一行 `emoji` 包**，点开在右栏列整包（与 ysyk 那条完全同一条路），
 *  只是包内文件来自目录、不是磁盘。**别再给它单开一个类型页**。
 *
 *  ⚠️ 汇总要去重：同一个码点会挂在多个 tag 下（那是分类视图，不是互斥分区），
 *  直接拼各组会重复（目录 881 条而各组之和 > 881）。 */
const EMOJI_PACK = 'emoji'
const emojiAll = computed(() => {
  const byCp = new Map()
  for (const g of emojiGroups.value) {
    for (const cp of (g.codepoints || [])) {
      let e = byCp.get(cp)
      if (!e) { e = { cp, tags: [] }; byCp.set(cp, e) }
      if (g.tag && !e.tags.includes(g.tag)) e.tags.push(g.tag)
    }
  }
  return [...byCp.values()]
})
/** 内建包的那张封面（第一张动图）—— 它没有 `stickersApi.detail` 可拉。 */
const emojiCover = computed(() => (emojiAll.value.length ? emojiGifUrl(emojiAll.value[0].cp) : ''))
const counts = computed(() => ({
  video: videos.value.length,
  // ⚠️ **内建包也算一个包**（网格里确实多一行）—— 不算的话「列表 2 行、计数 1」自相矛盾。
  sticker: packs.value.length + (emojiAll.value.length ? 1 : 0),
  // 音效按**组**计数（1213 组；变体是组内的事，塞进计数会变成 2118 而让人误解）。
  sfx: sfxEntries.value.length,
  font: installedFontCount.value,
}))
const countText = computed(
  () => `${counts.value.video} 视频 · ${counts.value.sticker} 贴纸包 · ${counts.value.sfx} 音效 · ${counts.value.font} 字体`,
)

/** 已装字体数（`installed === false` 是"清单里有但没落盘"的自带字体，不该算可用）。 */
const installedFontCount = computed(() => fonts.value.filter((f) => f.installed !== false).length)

/**
 * 当前类型的网格数据。**三种素材被折成同一个形状**（`key/name/sub/title/thumb…`），
 * 所以模板里那一个网格能画三种东西 —— 加第四种素材时只改这里。
 */
/** 音效页：按 `sfxFilter` 收窄后的条目（空筛选 = 全部）。
/** 当前库的**全部条目**（未筛选）—— 计数与层级都从它派生。 */
const sfxPackEntries = computed(() => sfxEntries.value.filter((e) => e.pack === sfxPack.value))

/** 当前库名（左上角标题用）。 */
const sfxPackName = computed(() => {
  const p = sfxPacks.value.find((x) => x.id === sfxPack.value)
  return p ? p.name : sfxPack.value || ''
})

/** 当前库的**服务端元信息**（`built` 位等）；预设包也有这一条（恒 `built: true`）。 */
const sfxPackMeta = computed(() => sfxPacks.value.find((x) => x.id === sfxPack.value) || null)

/** 当前库是不是**导入包**（只有导入包能构建/能删）。 */
const sfxPackIsUser = computed(() => !!sfxPackMeta.value?.deletable)

/** 当前导入包有没有清单（`pack.toml`）。没有 ⇒ LLM 生成事件时读不到这个包。 */
const sfxPackBuilt = computed(() => sfxPackMeta.value?.built !== false)

/** 构建时要提示「从多少个 tag 里挑」—— 词表规模从目录接口的 `tags` 现取，不写死。 */
const sfxTagCount = computed(() => sfxTags.value.length)

/** 词筛选（只作用于当前库）。 */
const sfxFiltered = computed(() => {
  const kw = String(sfxFilter.value || '').trim().toLowerCase()
  if (!kw) return sfxPackEntries.value
  return sfxPackEntries.value.filter((e) =>
    `${e.id} ${e.label} ${e.group || ''} ${e.subgroup || ''} ${(e.tags || []).join(' ')}`.toLowerCase().includes(kw))
})

/** 库内总数（"全部"那个 chip 上的数字 = **词筛后**，与下面两级同口径）。 */
const sfxLibTotal = computed(() => sfxFiltered.value.length)

/** **一级细分** = 大类（`group`）。导入包没有这一级 ⇒ 只有一项「其他」，前端据此不渲染整块。 */
const sfxCats = computed(() => {
  const n = new Map()
  for (const e of sfxFiltered.value) {
    const g = e.group || ''
    if (!g) continue
    n.set(g, (n.get(g) || 0) + 1)
  }
  return [...n.entries()].map(([cat, count]) => ({ cat, count }))
    .sort((a, b) => catRank(a.cat) - catRank(b.cat))
})

/** 大类排序（`①`…`⑦` 的顺序；不认识的一律排最后）。 */
function catRank(c) {
  const m = String(c || '').match(/^([①②③④⑤⑥⑦])/)
  return m ? '①②③④⑤⑥⑦'.indexOf(m[1]) : 99
}

/** 选中大类后的条目（没选 = 整个库）。 */
const sfxCatEntries = computed(() => (sfxCat.value ? sfxFiltered.value.filter((e) => (e.group || '') === sfxCat.value) : sfxFiltered.value))

/** **二级细分** = 小类 / zip 子目录（`subgroup`）。 */
const sfxGroups = computed(() => {
  const n = new Map()
  for (const e of sfxCatEntries.value) {
    const g = e.subgroup || ''
    if (!g) continue
    n.set(g, (n.get(g) || 0) + 1)
  }
  return [...n.entries()].map(([group, count]) => ({ group, count }))
    .sort((a, b) => b.count - a.count)
})

/** 最终列出的音效组（两级 chips 都生效）。 */
const sfxShown = computed(() => (
  sfxGroup.value ? sfxCatEntries.value.filter((e) => (e.subgroup || '') === sfxGroup.value) : sfxCatEntries.value
))

const items = computed(() => {
  if (kind.value === 'video') {
    return videos.value.map((v) => ({
      key: v.name,
      name: v.name,
      sub: v.duration == null ? '读不出时长' : `${v.duration.toFixed(1)}s`,
      // ⚠️ 探不出时长的**照列不隐藏**（用户放进去了却看不到，比"能用但标出来"更难查），
      // 只是把后果说清楚。
      title: v.duration == null ? `${v.file}（ffprobe 读不出时长，片头/片尾里不能选它）` : v.file,
      badge: v.duration == null ? '不可用' : null,
      thumb: videoThumbUrl(v.file),
      url: assetUrl('video', v.file),
      // 右栏要用：文件名（显示 + 探大小）、时长（`m:ss`）与技术信息（服务端已探好）
      file: v.file,
      duration: v.duration,
      width: v.width,
      height: v.height,
      codec: v.codec,
      fps: v.fps,
      bitrate: v.bitrate,
      has_audio: v.has_audio,
      type: 'video',
      detail: v.duration == null ? 'ffprobe 读不出时长，片头/片尾里选不了它。' : null,
    }))
  }
  if (kind.value === 'font') {
    return fonts.value.map((f) => ({
      key: f.id,
      name: f.display_name || f.id,
      // ⚠️ **副标题以前是 `f.file`**（2026-10-16 用户报「字体列表有点紊乱」）：v2 布局下每一条的
      //    `file` 都是 `font.ttf` ⇒ 三张卡片副标题**一模一样**，看不出区别（看着还像坏了）。
      //    改成有信息量的来源：自带 / 导入（老平铺布局与未安装照旧单独标）。
      sub: f.installed === false ? '未安装' : f.flat ? '老布局' : f.builtin ? '自带' : '导入',
      title: f.id,
      badge: f.is_default ? '默认' : null,
      // ⚠️ **「无粗体」要说出来**（2026-10-16）：库里没有 `<族名> Bold` 时，字幕/元素的
      //    700·600 字重会被底座**退回常规体**（它不合成粗体）—— 而预览这一侧也显式关掉了
      //    合成（`.cap/.dm/.text-el { font-synthesis:none }`）⇒ 两边一致地细。
      //    以前这件事**只有出片日志里那句 warn 知道**：用户在界面上选 900，看着没变化，
      //    没有任何提示。判据与 Rust `find_font` 同源（`hasBoldFace`）。
      // ⚠️ **emoji 字体与未安装的不挂这枚徽标**（2026-10-16）：
      //    · emoji 字体不参与正文的字重（它只补表情的字形），标「无粗体」纯属噪音；
      //    · 还没落盘的条目谈"有没有粗体"没有意义（同族粗体可能就在下载列表里）。
      badge2: f.emoji || f.installed === false || hasBoldFace(fonts.value, f) ? null : '无粗体',
      badge2Hint: '库里没有「' + (f.family || f.id) + ' Bold」这一支 ⇒ 字幕与文字的 700·600 字重会退回常规体' +
        '（预览与成片一致地细）。要真粗体：导入一支同族名的 Bold 字体。',
      stack: `"${f.family || f.id}", system-ui, sans-serif`,
      sample: sampleOf(),
      meta: `族名 ${f.family || f.id} · ${f.installed === false ? '还没落盘' : (f.bytes ? fmtBytes(f.bytes) : '')}`,
      type: 'font',
      // ⚠️ 自带字体**不可删**（服务端回 403）—— 前端就别给那颗按钮，而不是让用户点了再报错。
      // 判据是 `builtin`，与 `fonts::remove_font` 同源。
      // ⚠️ **emoji 字体不给「设为默认」**（用户报「设为默认失败…NotoColorEmoji-CBDT」）：
      // emoji 是**独立的补充槽位**，不参与"正文默认位"的竞争 —— `scan_all` 出口就把它
      // 强制成 `is_default = false`（`fonts.rs` 的 A4 第三道拦截）。给它按钮 =
      // 点下去必然失败，而且会走到 `dir_for_id` 那条**误导性**的报错
      // （"不在目录布局里…先重新导入一次"，把这支好好的自带字体说成坏了）。
      // 判据 `f.emoji` 由接口直接给出，与 `set_default` 里 `meta.emoji` 同源。
      actions: [
        f.is_default || f.installed === false || f.emoji
          ? null
          : { label: '默认', hint: '设为默认字体（只影响新工程的推荐位）', run: () => doSetDefault(f) },
        f.builtin ? null : { label: '删除', hint: '从字体库删除', run: () => doDeleteFont(f) },
      ].filter(Boolean),
    }))
  }
  // 贴纸：一行一个包（不是一张图一个格）—— 包内可能几百张，铺成格子会卡；
  // 这里用包里第一张当封面，点开看整包。
  const rows = packs.value.map((p) => {
    return {
      key: p.name,
      name: p.name,
      // ⚠️ **列表接口只回 `count`，没有 `files`**（`files` 只有 `detail` 才回）。
      // 这里以前读 `(p.files || []).length` ⇒ **永远是 0**，界面显示"0 张"，
      // 而磁盘上其实有几百张（实测 ysyk = 210 张）。点开还会误报"这个贴纸包里没有图片"。
      sub: `${p.count || 0} 张${p.built ? '' : ' · 未构建'}`,
      title: p.description || p.name,
      badge: p.built ? null : '未构建',
      // 封面图要**现取**（列表不带文件名）：`coverOf` 只拉一次 detail 并缓存。
      thumb: coverOf(p.name),
      url: coverOf(p.name),
      type: 'sticker',
      detail: p.built ? null : '未构建：图能看，但 LLM 生成事件时读不到这个包。',
      actions: [
        p.built ? null : { label: '构建', hint: '让 LLM 读文件名生成语义 tag（要花时间与额度）', run: () => doBuildPack(p) },
        { label: '删除', hint: '删掉整个包（目录 + 清单 + 索引项）', run: () => doDeletePack(p) },
      ].filter(Boolean),
    }
  })
  // ⚠️ **末尾多一行内建 `emoji` 包**（2026-10-17，用户订正：emoji 是**作为贴纸包**存在的）。
  //    位置与编辑器 `StickerPicker` 的包列表一致（真实包在前、`emoji` 在最后）。
  //    它没有 `<包名>.toml`、也不在磁盘上 ⇒ `sub`/`title` 现算，`actions` 为空
  //    （「构建」「删除」对一条不存在于磁盘的包都不适用 —— 别给点了必然失败的按钮）。
  if (emojiAll.value.length) {
    rows.push({
      key: EMOJI_PACK,
      name: EMOJI_PACK,
      sub: emojiAll.value.length + ' 张 · 内建',
      title: '内建 Noto 动态表情：编译期内嵌、不联网',
      badge: null,
      thumb: emojiCover.value,
      url: emojiCover.value,
      type: 'sticker',
      emoji: true,
      detail: '内建：编译期内嵌、不联网 —— 精修里选「表情」来源时用的就是这一套。',
      actions: [],
    })
  }
  return rows
})

/**
 * 给字体库里每一支注册 FontFace —— **样例字要用它**。
 *
 * ⚠️ 幂等：`ensureFontFace` 自己按「名字|dir」缓存，重复调用不会重复取字节。
 * ⚠️ 只跳过**没落盘**的条目（`installed === false`）：`.ttc` 也试着注册 —— 认不出时浏览器
 *    会自己退回 `system-ui`（与下拉里"不进列表"那条判据不冲突：这里只是**展示**）。
 * ⚠️ 库接口是**程序级**的（与工程无关）⇒ `pid` 传 `null`、`dir` 传空串。
 */
async function registerLibraryFonts() {
  for (const f of fonts.value) {
    if (f.installed === false) continue
    const name = f.family || f.id
    await ensureFontFace(null, '', name, { onDebug: (m) => console.debug('[字体库]', m) })
  }
}

/** 切到「字体」页时才注册（视频/贴纸页不必为几 MB 字体买单）。 */
watch(kind, (k) => { if (k === 'font') registerLibraryFonts() })

async function load(force = false) {
  loading.value = true
  error.value = ''
  try {
    // ⚠️ 素材库是**全局**资产（不在任何项目里），所以这三条请求**都不带 `{project, dir}` 作用域**
    // —— 别给 `api()` 硬塞一个作用域：那会去查一个跟素材库无关的项目。
    const [v, s, f, em, sx] = await Promise.all([
      api('/api/polish/video-library', { label: '读取视频素材库' }),
      stickersApi.list(),
      api('/api/polish/fonts', { label: '读取字体库' }),
      // ⚠️ **内建 `emoji` 包的内容**（贴纸页里那一行）：编译期内嵌、不属于任何工程 ⇒
      //    这条**不需要作用域**，别给它塞 {project, dir}（那会去查一个跟素材库无关的工程）。
      //    形状 {groups:[{tag, codepoints}]}。
      api('/api/polish/emoji-catalog', { label: '读取内建表情包' }),
      // ⚠️ 音效**要连被屏蔽的一起拿**（`includeBlacklisted`）：这一页要能"看见并解除屏蔽"，
      //    服务端默认那份过滤掉了被屏蔽的 ⇒ 界面上永远没有可恢复的条目。
      //    `pid`/`dir` 传空：音效库是程序级资产，跟工程无关。
      fetchSfxCatalog(null, '', '读取音效库', { includeBlacklisted: true }),
    ])
    videos.value = v?.videos || []
    packs.value = s?.packs || []
    fonts.value = f?.fonts || []
    emojiGroups.value = em?.groups || []
    sfxEntries.value = sx?.entries || []
    sfxBlacklist.value = sx?.blacklist || []
    sfxPacks.value = sx?.packs || []
    sfxTags.value = sx?.tags || []
    // ⚠️ **字体页的样例字要真的用那支字体渲染**（2026-10-16 修，用户报「字体列表有点紊乱」）：
    //    卡片上的 `it.stack` 写的是**族名**，而这一页**从未注册过 FontFace** ⇒ 浏览器根本不认识
    //    那些族名 ⇒ **每张卡片都用系统字体画样例**（三张卡看着一模一样，换样例文字也没区别）。
    //    注册名必须与 `stack` 里那个名字**逐字相同**（都是内部族名）—— 注册 id 而 CSS 用族名
    //    等于没注册（同一个坑在字幕预览那边也踩过）。
    if (kind.value === 'font') await registerLibraryFonts()
    // 音效页：载入后默认落在第一个库（内建「小森平」）—— 否则右栏一片空白等着用户点。
    if (kind.value === 'sfx' && !sfxPack.value && sfxPacks.value.length) {
      sfxPack.value = sfxPacks.value[0].id
    }
    // 三个根各取各的（都只对 admin 非空 —— 服务端刻意不给非 admin 回服务器路径）
    roots.video = v?.root || ''
    roots.sticker = s?.root || ''
    roots.font = f?.root || ''
    if (force) toast('素材库已刷新')
  } catch (e) {
    // ⚠️ 读失败**不伪装成空库**（否则用户会以为素材丢了、反复重放文件）。
    error.value = '素材库读取失败：' + e.message
  } finally {
    loading.value = false
  }
}

/** 点左列的一个**音效包** → 右列换成"这包里的组"（顺手清掉上一包的选中组）。
 *
 *  ⚠️ 只有音效页有"包"这一层；视频/贴纸/字体的 `open(it)` 是另一条路（见 [`open`]）。
 *  ⚠️ 切包必须 `stopSfx()`：上一包的那条还在响、而界面上已经看不到它了。
 */
function openPack(id) {
  stopSfx()
  sfxError.value = ''
  sfxPack.value = String(id || '')
  sfxCat.value = ''
  sfxGroup.value = ''
  preview.value = null
}

async function open(it) {
  // **字体没有右栏**（用户口径）：直接弹窗看大字形。
  if (it.type === 'font') {
    zoom.value = { type: 'font', title: it.name, stack: it.stack, sample: it.sample, meta: it.meta, detail: it.detail }
    return
  }
  // 视频与贴纸都进右侧预览栏（视频在那儿直接播、贴纸在那儿列整包）。
  // ⚠️ 视频**不弹窗** —— 用户试过弹窗那条路又改回来了，别再自动改回去。
  // ⚠️ 音效不经过这里：它走 `openPack`（包在模板里直接绑），别把两个入口合并 ——
  //    `items` 里音效那档是**包**、`pickSfxEntry` 收的是**组**，形状不同。
  await selectItem(it)
}

/** 从右栏的贴纸列表里点开一张 —— 大图仍走弹窗（列表格子小，看不清细节）。 */
function openStickerImage(f) {
  zoom.value = { type: 'sticker', title: f.name, url: f.url, detail: preview.value?.name }
}

/* ---------------- 音效：试听与屏蔽 ---------------- */

/** 在右栏的左列里点一条**音效组** → `preview` 换成它，右列列变体。
 *
 *  ⚠️ 与 [`open`] 分工：`open` 处理网格里的**包**，这里处理包内的**组**。
 *     两者都叫"点一下"，但一步是"进包"、一步是"选音效"，别合并。
 */
function pickSfxEntry(e) {
  stopSfx()
  sfxError.value = ''
  preview.value = {
    type: 'sfx',
    id: e.id,
    name: e.label || e.id,
    cat: e.cat,
    pack: e.pack,
    tags: e.tags || [],
    blacklisted: !!e.blacklisted,
    variants: e.variants || [],
    files: (e.variants || []).map((stem) => ({ stem, url: sfxPreviewUrl(e.id, stem) })),
  }
}

/** 停掉当前的试听（换一条 / 切页 / 选中别的条目时都要调）。 */
function stopSfx() {
  const a = sfxAudio.value
  if (a) {
    a.pause()
    a.removeAttribute('src')
    a.load()
  }
  sfxPlaying.value = ''
  sfxBusy.value = ''
}

/** 点一条变体：正在响的同一条 = 停；否则换源播放。
 *
 * ⚠️ **`src` 指向服务端 `/sfx-preview`，不是源站**：第一次点会有一次抓取的延迟
 * （服务端抓到 `<data>/cache/sfx/` 之后就是本地 Range，很快），所以先亮 `sfxBusy`。
 * ⚠️ 失败（源站 404 / 断网）要**说清楚是哪一条听不到**，别只清状态 —— 那看起来像按钮坏了。 */
async function playVariant(f) {
  if (sfxPlaying.value === f.stem && !sfxBusy.value) {
    stopSfx()
    return
  }
  stopSfx()
  sfxError.value = ''
  sfxBusy.value = f.stem
  const a = sfxAudio.value
  if (!a) return
  a.src = f.url
  try {
    await a.play()
    sfxPlaying.value = f.stem
  } catch (e) {
    // `NotAllowedError` 是浏览器自动播放策略（这里由用户点击触发，正常不会撞上）；
    // 其余多半是 404（试听源抓不到）—— 两种情况都落到同一句提示里，别吞掉。
    sfxError.value = `这一条试听不可用（${f.stem}）：${e?.message || '加载失败'}。` +
      '源站可能暂时不可达，稍后再点一次。'
  } finally {
    sfxBusy.value = ''
  }
}

/** 屏蔽 / 解除屏蔽**整组**。
 *
 * 写的是全局 `[sfx] blacklist`（按账号，`config.toml`），语义见 `sfx_catalog::is_blacklisted`：
 * 精确 id 屏蔽一组、`tag:xxx` 屏蔽整族。这里只做"这一组"的开关，所以直接用 id。
 *
 * ⚠️ **整份覆盖**：服务端要求传完整数组（它不做单条增删），所以基线是
 * `sfxBlacklist`（目录接口回的当前那份），改完再回写本地那份 —— 不重新拉目录
 * （那会把用户正在看的位置与筛选冲掉）。
 *
 * ⚠️ 收的是**音效组**（`preview` 或包内条目），不是包 —— 屏蔽的粒度是"一组音效"
 * （`blacklist` 里存的是组 id）。包墙上的「N 屏蔽」徽标是从 `sfxEntries` 现算的，
 * 所以这里只要把**条目本身**翻面，徽标会自动跟着变。
 */
async function toggleBlacklist(p) {
  if (!p || p.type !== 'sfx') return
  const next = new Set(sfxBlacklist.value)
  const nowBlocked = !p.blacklisted
  if (p.blacklisted) next.delete(p.id)
  else next.add(p.id)
  busy.value = true
  try {
    await putSfxBlacklist([...next])
    sfxBlacklist.value = [...next]
    // 目录里那一条同步翻面（包墙的「N 屏蔽」徽标、右栏的按钮文案都读它）。
    // ⚠️ 改的是 `sfxEntries` 里的**那个对象**，不是 `preview` 的副本 —— 两边
    //    各存一份就会漂（右栏解除后包墙还显示"1 屏蔽"）。
    for (const e of sfxEntries.value) {
      if (e.id === p.id) e.blacklisted = nowBlocked
    }
    p.blacklisted = nowBlocked
    toast(nowBlocked ? `已屏蔽「${p.name}」` : `已解除屏蔽「${p.name}」`)
    // ⚠️ 屏蔽变化会影响**编辑器**里可选的音效（同一份全局黑名单）——
    //    重新拉一次编辑器那份由它自己下次打开时处理，这里不跨组件广播。
  } catch (e) {
    toast('保存音效屏蔽失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
}

/** 导入音效包：zip 原始字节 POST 到 `/api/polish/sfx-packs/upload?name=`。
 *
 * ⚠️ 包名先过前端正则（与 `sanitize_pack_name` 同一条规则）—— 超限/非法名服务端回
 *    400 + JSON，但那时用户已经等了一次上传，不如当场说清楚。
 * ⚠️ 上传**没有进度条**（`fetch` 拿不到上传进度）：只显示「导入中…」，与贴纸上传同一口径。
 */
async function onSfxPackPick(ev) {
  const file = ev?.target?.files?.[0]
  if (ev?.target) ev.target.value = ''   // 同一文件再选一次也要触发
  if (!file) return
  // 包名：zip 文件名去扩展名
  const raw = String(file.name || '').replace(/\.zip$/i, '')
  if (!/^[\p{L}\p{N}_-]{1,32}$/u.test(raw)) {
    toast(`包名不合法：「${raw}」。用 1~32 个字母/数字/下划线/短横线', 'err`)
    return
  }
  sfxPackBusy.value = true
  try {
    // ⚠️ 走 `api()`（不是裸 fetch）：它统一带 CSRF 头、超时、失败日志与 `ok:false` 判定。
    //    贴纸上传 (`stickersApi.uploadPack`) 也是这么做的 —— 两条上传路径必须同一套口径。
    const j = await api(`/api/polish/sfx-packs/upload?name=${encodeURIComponent(raw)}`, {
      method: 'POST',
      body: file,
      headers: { 'Content-Type': 'application/octet-stream' },
      label: `导入音效包 ${raw}`,
      timeout: 10 * 60 * 1000,
    })
    // `transcode_failed` 照实说：那几个保留了原格式，出片会按真实扩展名找（不是错误）
    const extra = [
      j?.skipped ? `跳过 ${j.skipped} 项` : '',
      j?.transcode_failed ? `${j.transcode_failed} 个未转 mp3` : '',
    ].filter(Boolean).join('，')
    toast(`已导入「${raw}」：${j?.copied ?? 0} 个音效${extra ? `（${extra}）` : ''}`)
    await load()
  } catch (e) {
    toast('导入音效包失败：' + (e.message || e), 'err')
  } finally {
    sfxPackBusy.value = false
  }
}

/** 把一次 watchJob 订阅包成 promise：终态 succeeded 才 resolve，其余按错误抛。
 *
 * ⚠️ 与 `HomeClipsOverview.vue` 里那份同形（那边是局部函数，没有共享导出）。
 *    订阅句柄必须由调用方在卸载时关掉 —— 这里把它登记进 `sfxJobClosers`。
 */
const sfxJobClosers = []
function waitJobDone(id) {
  if (!id) return Promise.resolve(null)
  return new Promise((resolve, reject) => {
    let settled = false
    const w = watchJob(id, {
      onDone(s) {
        if (settled) return
        settled = true
        if (s?.status === 'succeeded') resolve(s)
        else reject(new Error(s?.message || s?.error || '任务' + (s?.status || '异常结束')))
      },
    })
    sfxJobClosers.push(() => w.close())
  })
}

/** 用 LLM 构建当前导入包的 `pack.toml`（与贴纸的「构建」同一套：起任务 → 等完成 → 重拉目录）。
 *
 * ⚠️ 它是**任务**（`POST /polish/sfx-packs/:name/build` 回 `{id}`），不是同步请求：
 *    整批 LLM 调用可能几十秒。按钮只置 `sfxBuildBusy`，完成后 `load()` 重新拿目录
 *    （`built` 位与条目上的 tag 都会跟着变）。
 * ⚠️ 失败要说清原因（服务端把 `{e:#}` 打进任务日志，这里 toast 一句就够 —— 详细原因在任务抽屉里）。
 */
async function doBuildSfxPack() {
  const name = sfxPack.value
  if (!name || sfxBuildBusy.value) return
  sfxBuildBusy.value = true
  try {
    const j = await api(`/api/polish/sfx-packs/${encodeURIComponent(name)}/build`, {
      method: 'POST',
      label: `构建音效包 ${name}`,
      timeout: 10 * 60 * 1000,
    })
    toast(j?.id ? '已开始构建，完成后会自动刷新' : '构建任务已提交')
    await waitJobDone(j?.id)
    await load()
  } catch (e) {
    toast('构建音效包失败：' + (e.message || e), 'err')
  } finally {
    sfxBuildBusy.value = false
  }
}

/** 删除导入包（预设包没有删除钮：模板只给 `p.deletable` 的库渲染 `×`）。 */
async function doDeleteSfxPack(name) {
  const okDelete = await confirmDialog(`删掉音效包「${name}」？包里的音频文件会一起删除，不能恢复。`)
  if (!okDelete) return
  busy.value = true
  try {
    await api(`/api/polish/sfx-packs/${encodeURIComponent(name)}`, {
      method: 'DELETE',
      label: `删除音效包 ${name}`,
    })
    toast(`已删除音效包「${name}」`)
    if (sfxPack.value === name) { stopSfx(); sfxPack.value = null; preview.value = null }
    await load()
  } catch (e) {
    toast('删除音效包失败：' + (e.message || e), 'err')
  } finally {
    busy.value = false
  }
}

const zoom = ref(null)

/* ---------------- 统一导入 ---------------- */

const dragOver = ref(false)
const busy = ref(false)
const doneCount = ref(0)
const totalCount = ref(0)
const fileInput = ref(null)

function onDragOver() {
  if (kind.value === 'video') return
  dragOver.value = true
}
function onDragLeave() {
  dragOver.value = false
}

async function onDrop(e) {
  dragOver.value = false
  if (kind.value === 'video') return
  const files = [...(e.dataTransfer?.files || [])]
  if (files.length) await importFiles(files)
}
async function onFilePick(e) {
  const files = [...(e.target?.files || [])]
  e.target.value = ''
  if (files.length) await importFiles(files)
}

/** 按当前类型分派导入。⚠️ 三条路的前端预检**必须在这里**（超限时服务端回 413 + 纯文本，
 *  而 `client.js` 只认 JSON，会把"文件太大"报成"接口不存在或返回格式错误"）。 */
async function importFiles(files) {
  if (kind.value === 'font') return importFonts(files)
  return importStickerZip(files[0])
}

async function importFonts(files) {
  const good = files.filter((f) => /\.(ttf|otf)$/i.test(f.name))
  const bad = files.filter((f) => !/\.(ttf|otf)$/i.test(f.name))
  if (bad.length) {
    toast(`已跳过 ${bad.length} 个非字体文件（只收 .ttf / .otf）`, 'warn')
  }
  if (!good.length) return
  const tooBig = good.find((f) => f.size > FONT_UPLOAD_MAX_BYTES)
  if (tooBig) {
    toast(`${tooBig.name} 超过 ${Math.round(FONT_UPLOAD_MAX_BYTES / 1024 / 1024)} MB 上限`, 'err')
    return
  }
  busy.value = true
  totalCount.value = good.length
  doneCount.value = 0
  const failed = []
  try {
    for (const f of good) {
      try {
        await uploadFont(f)
      } catch (e) {
        failed.push(`${f.name}：${e.message}`)
      }
      doneCount.value += 1
    }
  } finally {
    busy.value = false
  }
  await load()
  report(failed, `已导入 ${good.length - failed.length} 支字体`)
}

async function importStickerZip(file) {
  if (!file) return
  if (!/\.zip$/i.test(file.name)) {
    toast('贴纸包要传 zip 压缩包', 'err')
    return
  }
  // 上限是配置项（config.toml 的 [stickers] upload_max_mb，默认 1024 MiB）——
  // ⚠️ 必须现取，不能写死：前端写死那个数会和后端配置漂（旧版写死 64 MiB，
  //    用户 500 MB 的包连预检都过不去）。取不到才退回同一份默认值，服务端仍会挡。
  let maxBytes = DEFAULT_UPLOAD_MAX_BYTES
  try {
    const r = await stickersApi.limits()
    if (r && Number(r.upload_max_bytes) > 0) maxBytes = Number(r.upload_max_bytes)
  } catch (e) {
    // 读不到上限不该让上传直接失败：交给服务端判，超限会回 JSON 413
  }
  if (file.size > maxBytes) {
    toast(`${file.name} 超过 ${Math.round(maxBytes / 1024 / 1024)} MB 上限`, 'err')
    return
  }
  const name = file.name.replace(/\.zip$/i, '')
  busy.value = true
  try {
    // 一律**不**顺带构建：构建是 LLM 任务、要花时间与额度，让用户在列表里显式点。
    // （上传成功与构建任务是两件事 —— 构建起不来不该让上传显示成失败。）
    await stickersApi.uploadPack(file, name, false)
  } catch (e) {
    toast('导入失败：' + e.message, 'err')
    return
  } finally {
    busy.value = false
  }
  await load()
  toast(`已导入贴纸包「${name}」，可在列表里点「构建」让 LLM 生成 tag`)
}

/** 统一汇报：成功的走 toast，失败的**逐条列出来**（只报"部分失败"等于没说）。 */
function report(failed, okText) {
  if (!failed.length) return toast(okText)
  toast(`${okText}，${failed.length} 个失败：${failed.join('；')}`, 'err')
}

/* ---------------- 管理动作 ---------------- */

async function doSetDefault(f) {
  busy.value = true
  try {
    await setDefaultFont(f.id)
  } catch (e) {
    return toast('设为默认失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
  await load()
  toast(`已把「${f.display_name || f.id}」设为默认（只影响新工程）`)
}

async function doDeleteFont(f) {
  // ⚠️ 二次确认：字体是"导入后不易得"的东西（几十 MB、还要去下载），误点一次就得重来。
  if (!(await confirmDialog(`删除字体「${f.display_name || f.id}」？`, { danger: true })))
    return
  busy.value = true
  try {
    await deleteFont(f.id)
  } catch (e) {
    return toast('删除失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
  await load()
  toast('字体已删除')
}

async function doBuildPack(p) {
  busy.value = true
  try {
    // 走任务队列（kind `sticker_build`），所以这里只是"派出去"，不等待完成。
    await stickersApi.build(p.name)
  } catch (e) {
    return toast('构建失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
  toast(`已派发「${p.name}」的构建任务，可在任务列表里看进度`)
}

async function doDeletePack(p) {
  if (!(await confirmDialog(`删除贴纸包「${p.name}」？包内 ${p.count || 0} 张图会一起删掉。`, { danger: true })))
    return
  busy.value = true
  try {
    await stickersApi.remove(p.name)
  } catch (e) {
    return toast('删除失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
  await load()
  toast('贴纸包已删除')
}

// 字节数 → 人话。⚠️ **全组件只有这一份**（以前我另加过一份，撞了 `Identifier already declared`）——
// 要改单位就改这里，别在别处再写一个。
const fmtBytes = (n) =>
  !n || n < 0
    ? '—' // 探不到（HEAD 失败 / 元数据读不出）就**照实说没拿到**，不显示 "0 KB" 那种假数
    : n > 1024 * 1024
      ? (n / 1024 / 1024).toFixed(1) + ' MB'
      : Math.round(n / 1024) + ' KB'

onMounted(() => load())

// 组件卸载时停掉试听：`<audio>` 不会随 DOM 消失而停（它是唯一一个会"继续响"的元素）。
// 同一件事故在精修预览那边也踩过（`check-preview-audio.mjs` 的 ⑤/⑩ 两条）。
onBeforeUnmount(() => {
  // ⚠️ 构建任务订阅必须退订：不退订会随反复挂载攒出一串陈旧回调，
  //    任务结束时同时各打一整套 load()（与 useJobs 那条同一条纪律）。
  while (sfxJobClosers.length) sfxJobClosers.pop()()
  stopSfx()
})
</script>

<style scoped>
/* 头部：整行可点开合；计数一直在（收起时也知道库里有多少东西）。 */
.ml-head {
  display:flex; align-items:center; gap:8px; min-width:0;
  padding:0; border:none; background:none; color:inherit; cursor:pointer; text-align:left;
}
.ml-head:hover b { color:var(--main-deep); }
.ml-head b { white-space:nowrap; }
.ml-open { flex:none; }
/* 左导航 + 右内容：导航定宽、内容自适应。窄屏堆成上下两段（见下 media query）。 */
.ml-body { display:flex; gap:12px; align-items:flex-start; }
.ml-nav { display:flex; flex-direction:column; gap:4px; flex:none; width:128px; }
.ml-main { flex:1 1 auto; min-width:0; }

/* ---------- 右侧预览栏 ----------
   三栏：类型导航（128px）| 素材网格（自适应）| 预览栏（定宽）。
   ⚠️ 预览栏**定宽**而不是自适应：视频播放器需要一个稳定的宽高比参考，
   宽度一变、同一段视频每换一次选中就跳一次尺寸。 */
.ml-side {
  /* 宽度：300 → 600（用户：「宽度翻倍」）→ **自适应**（用户：「播放器的高度还是没变」）。
     ⚠️ **不能写死像素**：视频高度 = 栏宽 ÷ 宽高比，所以想让画面更高就得让栏更宽 ——
     但写死 860px 在窄窗口里**只会横向溢出**，右侧被切掉、还得拖滚动条
     （实测：窗口 744px 时文档宽 1012px，就是这个毛病，用户截图里播放器被截断正是它）。
     改成"占窗口的 62%、至少 380px、至多 860px"：
       · 窗口够宽 ⇒ 拿满 860px（画面最高，这是用户要的）；
       · 窗口窄 ⇒ 跟着缩，**永远不溢出**。 */
  flex:none; width:clamp(380px, 62%, 860px);
  display:flex; flex-direction:column; gap:8px;
  border-left:1px solid var(--line); padding-left:12px;
  min-height:220px;
}
/* ---------- 音效页：与贴纸页**同一套三栏** ----------
   左 `.ml-nav`（128px，这里放宽到 176px 放库卡片）| 中 `.ml-main` | 右 `.ml-side`（clamp）。
   ⚠️ **左侧是"库的位置"**：现在只有内建「小森平」，用户导入的包一张张往下长。
     别把大类/小类铺到左侧 —— 那会把库的位置占满，新库没地方放（用户口径）。
   ⚠️ 大类（一级）与小类（二级）都是中间那栏的 chips（`.ml-tagbar`，贴纸同款），
     音效组是中间那栏的行列表，变体在右栏（`.ml-side` 复用，尺寸与贴纸页一致）。 */
.ml-nav-packs { width:176px; gap:6px; }

/* 库卡片：图标 + 名字/数量 + （导入包的）删除钮。竖排、矮，一屏能放十几个。 */
.ml-pack-card {
  position:relative;
  display:flex; align-items:center; gap:8px; width:100%;
  padding:8px 8px; border:1px solid var(--line); border-radius:8px;
  background:var(--card); color:inherit; font:inherit;
  cursor:pointer; text-align:left; min-width:0;
}
.ml-pack-card:hover { border-color:var(--main); }
.ml-pack-card.on { border-color:var(--main); background:var(--main-soft); }
.ml-pack-ico {
  flex:none; display:flex; align-items:center; justify-content:center;
  width:28px; height:28px; border-radius:6px; background:var(--bg-soft);
}
.ml-pack-txt { display:flex; flex-direction:column; gap:1px; min-width:0; flex:1 1 auto; }
.ml-pack-name {
  font-size:12px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
}
/* 删除钮：只在导入包上出现，靠右上角；⚠️ 用 `<span>` 不是 `<button>` —— 它嵌在
   库卡片 `<button>` 里，嵌套 button 是非法 HTML（浏览器会拆开、点击落到外层）。 */
.ml-pack-del {
  flex:none; width:16px; height:16px; line-height:14px; text-align:center;
  border-radius:50%; color:var(--muted); font-size:14px; cursor:pointer;
}
.ml-pack-del:hover { background:var(--bg-soft); color:var(--main); }

/* 「导入音效包」：虚线框，与上面的库卡片同尺寸 —— 它是"库的位置还空着"的显式入口 */
.ml-pack-add {
  display:flex; align-items:center; justify-content:center; gap:6px;
  width:100%; padding:7px 8px; font-size:12px;
  border:1px dashed var(--line); border-radius:8px;
  background:transparent; color:var(--muted); font:inherit; cursor:pointer;
}
.ml-pack-add:hover:not(:disabled) { border-color:var(--main); color:var(--main); }
.ml-pack-add:disabled { cursor:default; opacity:.6; }

/* **两级 chips = 横向滚动单行**（2026-11-28 用户口径：「tag 也需要整理一下」）。
   ⚠️ 不用 `.ml-tagbar`（那个是 wrap 多行：一级 8 个 + 二级最多 31 个会堆五六行，
   把中间栏上半截吃光）。这里固定一行高、横向滚动，高度不随分类数量变。 */
.ml-chiprow {
  display:flex; flex-wrap:nowrap; gap:4px; align-items:center;
  overflow-x:auto; overflow-y:hidden;
  height:32px; flex:none;
  /* ⚠️ 右端留一条渐隐，暗示"还能往右滚" —— 单行横滚最大的问题是
     用户不知道后面还有东西（实测二级 31 个 chip 时看着像只有 8 个）。 */
  -webkit-mask-image:linear-gradient(to right, #000 calc(100% - 24px), transparent);
  mask-image:linear-gradient(to right, #000 calc(100% - 24px), transparent);
  scrollbar-width:thin;
  scrollbar-color:var(--line) transparent;
}
.ml-chiprow .ml-tag { flex:none; }
.ml-chiprow + .ml-chiprow { margin-top:2px; }

/* chips 行外层：放左右滚动钮（渐隐只能"暗示"，31 个分类时用户根本不知道后面还有） */
.ml-chipwrap { position:relative; display:flex; align-items:center; }
.ml-chipwrap .ml-chiprow { flex:1 1 auto; min-width:0; }
.ml-chipnav {
  flex:none; width:18px; height:24px; line-height:22px;
  border:1px solid var(--line); border-radius:6px;
  background:var(--card); color:var(--muted);
  font:inherit; font-size:13px; cursor:pointer; padding:0;
}
.ml-chipnav:hover { border-color:var(--main); color:var(--main); }
.ml-chipnav.right { margin-left:4px; }
.ml-chipnav:not(.right) { margin-right:4px; }

/* **音效组 = 紧凑卡片网格**（2026-11-28）。
   ⚠️ 行列表在 300px 宽的中栏里一屏只放十几条，分组一多就是一条无边长带；
      卡片按 3~4 列排，一屏能看 30+ 组。音效没有画面，所以卡片只放文字。 */
.ml-sfx-cards {
  display:grid;
  grid-template-columns:repeat(auto-fill, minmax(148px, 1fr));
  gap:6px;
  max-height:min(46vh, 460px); overflow-y:auto;
  padding-right:2px;
}
.ml-sfx-card {
  position:relative;
  display:flex; flex-direction:column; gap:3px;
  padding:7px 8px; border:1px solid var(--line); border-radius:8px;
  background:var(--card); color:inherit; font:inherit;
  cursor:pointer; text-align:left; overflow:hidden; min-width:0;
}
.ml-sfx-card:hover { border-color:var(--main); }
.ml-sfx-card.on { border-color:var(--main); background:var(--main-soft); }
.ml-sfx-card.off { opacity:.6; }
.ml-sfx-card-name {
  font-size:12px; line-height:1.35;
  overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
}
/* 副信息一行：分组名可收缩，变体数不收缩（数字比名字重要，不能被挤没） */
.ml-sfx-card-meta { display:flex; gap:6px; align-items:baseline; min-width:0; }
.ml-sfx-card-sub { flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.ml-sfx-card-n { flex:none; font-variant-numeric:tabular-nums; }
/* 卡片上的「已屏蔽」徽标缩到角上（网格卡片小，默认徽标会盖住名字） */
.ml-sfx-card .ml-badge { position:absolute; top:4px; right:4px; font-size:9px; }

/* **右栏（音效）**：变体通常只有 1~5 条 ⇒ 不需要视频那档 62% 宽。
   收窄到 380px（与贴纸右栏的下限一致），把宽度还给中间的卡片网格。 */
.ml-sfx-side { width:clamp(300px, 30%, 420px); }

/* 选中组的分类 / tag：**压成一行**（超长省略，全文进 `title`）。
   原来是一整段自动换行，tag 一多就把标题区撑到四五行。 */
.ml-sfx-meta {
  overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
}

/* **变体 = 卡片**：序号 + 键名 + 播放状态，整张卡可点。
   ⚠️ 变体是"同一条音效的不同版本"，通常 1~5 条；列表形式在宽栏里会拉成细长线。 */
.ml-variant-cards {
  display:flex; flex-direction:column; gap:4px;
  max-height:min(42vh, 420px); overflow-y:auto; padding-right:2px;
}
.ml-variant-card {
  display:flex; align-items:center; gap:8px;
  padding:7px 9px; border:1px solid var(--line); border-radius:8px;
  background:var(--card); color:inherit; font:inherit;
  cursor:pointer; text-align:left; min-width:0;
}
.ml-variant-card:hover:not(:disabled) { border-color:var(--main); }
.ml-variant-card:disabled { cursor:default; opacity:.65; }
.ml-variant-card.on { border-color:var(--main); background:var(--main-soft); }
.ml-variant-card-i {
  flex:none; width:18px; text-align:right;
  font-size:11px; font-variant-numeric:tabular-nums; opacity:.55;
}
.ml-variant-card-name {
  flex:1 1 auto; min-width:0;
  overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  font-size:11.5px;
}
.ml-variant-card-state { flex:none; font-size:11px; opacity:.75; }

.ml-side-empty {
  display:flex; align-items:center; justify-content:center;
  min-height:160px; text-align:center; padding:0 12px;
  border:1px dashed var(--line); border-radius:8px;
}
/* 视频（右栏里的那只）。
   ⚠️ **尺寸要由视频自己决定，不能用 CSS 猜比例**（用户口径：「播放器要显示正确的尺寸」）：
     · `width:100%` —— 铺满栏宽；
     · `height:auto` —— 高度按视频**内在宽高比**算；
     · **不要写 `aspect-ratio`** —— 那是猜。我上一版写死 `16/9`，而库里的 `op.mp4`
       是 320×240(4:3) ⇒ 画面框比例就是错的（这条正是用户报的"尺寸不对"）。
     · **不要写 `object-fit`** —— 不写时元素本身就被视频撑成正确比例，
       黑边根本不会出现；写 `contain` 反而会在比例算错时留边。
     · `flex:none` —— `.ml-side` 是 column flex 容器，不加会被压扁（默认 `flex-shrink:1`）。
     · ⚠️ **必须显式解除 `max-height`** —— 这是"播放器高度怎么都改不了"的**真正原因**：
       `styles/app.css:241` 有一条**全局** `video { max-height:170px }`（给工程里那些小预览用的），
       它**依旧命中**这个元素 —— 我给自己的类设了 `width/height` 却**没管 `max-height`**，
       于是无论栏宽 300 / 600 / 860，播放器**永远只有 170px 高**。
       实测（宽度 860 与 460 各放一只）：两者都是 `848x170` / `448x170` —— 高度被钉死在 170。
       这也解释了为什么"加宽栏却没变高"：宽度确实变了，只是被 max-height 截回来了。
   实测：op.mp4(320×240) 在 860 栏下 → 860×636；probe.mp4(1920×1080) → 860×484。 */

/* 视频的**外围框**：负责"能有多大 / 至少多大"，不参与比例计算。
   · `max-height:70vh` —— 用户口径的上限。竖屏长视频（720×1280）在 860 栏下会到 1529px，
     不收住会把信息区顶到几屏之外、得滚动才看得见。
   · `min-height:200px` —— 用户口径的下限。窄窗口下 4:3 只算出 ~150px 时，
     不至于缩成一条缝（这条正是"起码能正常显示"）。
   · `display:flex` + `justify-content:center` —— 视频按比例缩进框里时**居中**，
     而不是贴着左边、右边空一块。 */
.ml-video-frame {
  flex:none; width:100%;
  /* 比例由 JS 写在行内 `aspect-ratio`（见 `videoFrameStyle`）—— 框**就是视频的形状**，
     所以不会有黑边。
     · `max-height:70vh`（用户口径的上限）：竖屏长视频在 860 栏下会到 1529px，
       不收住会把信息区顶出几屏。收住后框变矮、**宽度也跟着按比例收**（`aspect-ratio` 的效果），
       于是仍然没有黑边 —— 这正是"上限"和"无黑边"能同时成立的原因。
     · `min-height:200px`（用户口径的下限）：窄窗口下 4:3 只算出 ~150px 时不至于缩成一条缝。
       ⚠️ 上限与下限冲突时（如竖屏在很矮的窗口里）浏览器取 min 优先，框会略高于 70vh，
       这是有意的 —— "起码能正常显示"优先于严格上限。 */
  /* ⚠️ **`width:100%` + `aspect-ratio` 在"被 max-height 夹住"时会破功**：
     竖屏 720×1280 想要的框是 634×1127，但 `max-height:70vh` 把它压到 634 高 ——
     而 `width:100%` 是**硬性**的 ⇒ 宽度不跟着缩 ⇒ 视频只能缩到框中间，
     两侧留下 504px 黑边（实测就是这个数）。
     改成 `width:auto` + `height:100%` 也不行（框又会塌）。
     正解：**宽度交给 `aspect-ratio` 推**（`width:auto` + `height:auto` 由 max-height 决定，
     但宽度没有约束源）。所以给框加 `max-width:100%` 并把宽度放开为 `auto`，
     由 `aspect-ratio` + `max-height` 共同算出宽度。 */
  /* ⚠️ 宽度要 `auto` —— 只有 `auto` 才能让 `aspect-ratio` 真正接管宽度
     （竖屏那支靠 `max-height` 反推出一个**较窄**的框，`width:100%` 会挡住它 ⇒ 504px 黑边）。
     ⚠️ 但纯 `auto` 会让**小视频**的框塌到原生尺寸（320×240 的 `op.mp4` 框只有 320 宽，
     本该铺满栏宽）。所以给一个宽度下限 —— **只对"宽高比 ≥ 1"的视频加**
     （见模板上的 `style`：竖屏不能有这个下限，否则框比视频宽 ⇒ 24px 黑边，实测过）。
     `align-self:center`：宽框居中，不贴左边。 */
  width:auto;
  min-height:200px; max-height:70vh; max-width:100%;
  align-self:center;
  display:flex; align-items:center; justify-content:center;
  background:#000; border-radius:8px; overflow:hidden;
}
/* 框里的视频：**宽高都跟着框，并保持自身比例**。
   ⚠️ 关键在 `max-width:100%` + `max-height:100%` + `object-fit:contain` 这一组：
     两者同时成立时浏览器**按比例缩放**（不会拉伸），于是
       · 框比视频"方" ⇒ 视频按高度贴满、左右留极窄的边（框本来就贴合，几乎看不见）；
       · 框比视频"扁" ⇒ 视频按宽度贴满。
     这就是"完全覆盖视频、不要黑边"与"高度有上下限"能同时成立的原因：
     **框管尺寸、视频管比例**，两件事分开。
   ⚠️ `max-height:none` 仍然必要：全局 `styles/app.css:241` 的 `video{max-height:170px}`
     会命中这里 —— 那正是"播放器高度怎么都改不了"的真正原因（实测 860 栏下也只有 170px 高）。
     它由本规则里的 `max-height:100%` 覆盖（scoped 选择器特异性更高），这里再显式写一次防止
     将来有人调优先级时又漏掉。 */
.ml-side-video {
  display:block; margin:0;
  /* 框的形状已经等于视频比例了，视频**填满框**即可 —— `object-fit:contain` 是保险：
     万一比例还没报上来（兜底 16:9）时视频也不会被拉变形，只会暂时留一点边。 */
  width:100%; height:100%;
  object-fit:contain;
  border-radius:8px; background:transparent;
  /* ⚠️ **必写**：全局 `styles/app.css:241` 有 `video{max-height:170px}`，
     它正是"播放器高度怎么都改不了"的真正原因（实测 860 栏下也只有 170px 高）。 */
  max-height:none;
}
/* 信息**两列**（用户口径：「信息作为两排排版」）。
   ⚠️ `align-items:flex-start`：两列行数不同（左 4 行、右 5 行），不写会被拉成等高、
   左列多出一截空白。`gap:20px` 是列间距，比行间距大，让人一眼看出是**两栏**不是一串。 */
.ml-side-info { flex:none; display:flex; gap:20px; align-items:flex-start; }
.ml-info-col { flex:1 1 0; min-width:0; display:flex; flex-direction:column; gap:5px; }
.ml-info-row { display:flex; gap:8px; align-items:baseline; font-size:12px; }
.ml-info-row > .muted { flex:none; width:42px; }
.ml-info-row > b, .ml-info-row > span:not(.muted) { min-width:0; overflow-wrap:anywhere; }
.ml-ell { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }

/* 标签筛选栏。
   ⚠️ `flex-wrap:wrap` + **不给 max-height**：ysyk 有 34 个标签，写死高度会
   把标签藏起来（用户根本不知道还有别的标签）。让它自然换行、整栏跟着长。
   标签多时这一块会比较高，所以**排在图片列表上面**，图片列表自己滚（见 `.ml-side-grid`）。 */
.ml-tagbar { display:flex; flex-wrap:wrap; gap:4px; flex:none; }
.ml-tag {
  display:inline-flex; align-items:center; gap:4px;
  padding:2px 7px; border-radius:999px; cursor:pointer;
  font-size:11px; line-height:1.6;
  border:1px solid var(--line); background:transparent; color:inherit;
}
.ml-tag:hover { border-color:var(--main); }
/* 选中态：**底色 + 边框一起变**，只变边框在小标签上几乎看不出来。 */
.ml-tag.on { border-color:var(--main); background:var(--main-soft); color:var(--main); }
.ml-tag-n { opacity:.6; font-size:10px; }

/* 包内图片列表：多列小图，可滚。一个包 200+ 张（ysyk = 210），
   不给最大高度会把整页撑到几屏长、左栏的网格被挤出视野。
   8 列是跟着 860px 的栏宽定的：列数少了每张图被拉得很大、一屏反而看得更少。 */
.ml-side-grid {
  display:grid; grid-template-columns:repeat(8, 1fr); gap:4px;
  max-height:460px; overflow-y:auto; padding-right:2px;
}
.ml-side-grid img {
  width:100%; aspect-ratio:1; object-fit:contain; cursor:zoom-in;
  border-radius:6px; background:var(--bg-soft); display:block;
}
.ml-side-grid img:hover { outline:1px solid var(--main); }

/* 音效右栏：变体列表（一行 = 序号 + 变体键 + 试听按钮）。
   变体键往往是英文目录名那种长串，所以 `min-width:0` + 省略号是必需的，
   否则一行会把整栏撑宽、把试听按钮挤出可视区。 */
.ml-variant-list {
  display:flex; flex-direction:column; gap:4px;
  max-height:420px; overflow-y:auto; padding-right:2px;
}
.ml-variant {
  display:flex; align-items:center; gap:8px;
  padding:4px 6px; border:1px solid var(--line); border-radius:8px;
}
.ml-variant-i { flex:none; width:18px; text-align:right; font-variant-numeric:tabular-nums; }
.ml-variant-name {
  flex:1 1 auto; min-width:0;
  overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  font-size:11.5px;
}

/* 音效页的筛选框：铺满主区宽度，与其它页的 `.ml-drop` 同一套圆角/边框口径。 */
.ml-sfx-filter {
  width:100%; margin-bottom:8px;
  padding:5px 8px; font-size:12px;
  border:1px solid var(--line); border-radius:8px;
  background:var(--bg-soft); color:inherit;
}
/* 「未构建」提示条：一行 + 一颗按钮，高度固定不挤压下面的 chips 区。 */
.ml-sfx-unbuilt {
  display:flex; align-items:center; justify-content:space-between; gap:8px;
  margin-bottom:8px; padding:5px 8px;
  border:1px dashed var(--line); border-radius:8px;
  background:var(--bg-soft);
}
.ml-sfx-filter:focus { outline:none; border-color:var(--main); }

/* 选中态：左网格里当前在右栏显示的那一条。
   ⚠️ 必须**比 `:hover` 更特指**，否则鼠标一移开就看不见选的是谁。 */
.ml-cell-wrap.sel .ml-cell { border-color:var(--main); background:var(--main-soft); }

/* 左栏里的「样例文字」：跟类型导航同一列，不上浮到主区右上角。 */
.ml-sample-wrap { display:flex; flex-direction:column; gap:4px; margin-top:8px; padding:0 2px; }
.ml-sample-wrap .ml-sample { width:100%; }

.ml-tab {
  display:flex; align-items:center; gap:8px;
  padding:7px 10px; border-radius:7px; cursor:pointer;
  border:1px solid transparent; background:transparent;
  color:var(--text); font-size:13px; text-align:left;
}
.ml-tab:hover { background:var(--bg-soft); }
.ml-tab.on { background:var(--main-soft); border-color:var(--main); color:var(--pink-text); font-weight:600; }
.ml-ico { flex:none; }
.ml-lab { flex:1 1 auto; min-width:0; }
/* ⚠️ 计数必须 `flex:none`：写成可收缩时名字一长就被挤成一条竖线（同 `.src-badge` 那条教训）。 */
.ml-cnt { flex:none; font-size:11px; opacity:.65; font-variant-numeric:tabular-nums; }

/* 导入区：与网格同宽，虚线框。视频那档 `off` = 不响应拖拽（它不走上传）。 */
.ml-drop {
  display:flex; align-items:center; gap:10px;
  padding:10px 12px; border-radius:8px;
  border:1px dashed var(--line); background:var(--bg-soft);
  cursor:pointer;
}
.ml-drop.over { border-color:var(--main); background:var(--main-soft); }
.ml-drop.off { cursor:default; }
.ml-drop-t { display:flex; flex-direction:column; gap:2px; min-width:0; }
.ml-dir { overflow-wrap:anywhere; }

/* 网格：自适应列宽（`minmax` 让窄屏自动减列，不用媒体查询）。 */
.ml-grid { display:grid; grid-template-columns:repeat(auto-fill, minmax(108px, 1fr)); gap:8px; }
.ml-cell {
  display:flex; flex-direction:column; gap:4px; padding:6px;
  border:1px solid var(--line); border-radius:8px; background:var(--card);
  cursor:pointer; text-align:left; overflow:hidden;
}
.ml-cell-wrap { position:relative; display:flex; }
.ml-cell-wrap .ml-cell { flex:1 1 auto; min-width:0; }
.ml-cell:hover { border-color:var(--main); }
/* 管理动作条：默认隐身、悬停/键盘聚焦才出来（常显会把 108px 的格子塞满）。 */
.ml-acts { position:absolute; top:4px; right:4px; display:flex; gap:3px; opacity:0; transition:opacity .12s; }
.ml-cell-wrap:hover .ml-acts,
.ml-cell-wrap:focus-within .ml-acts { opacity:1; }
.ml-act {
  font-size:10px; padding:1px 5px; border-radius:4px; cursor:pointer;
  border:1px solid var(--line); background:var(--card); color:var(--ink-dim);
}
.ml-act:hover { border-color:var(--main); color:var(--pink-text); }
.ml-thumb {
  position:relative; display:flex; align-items:center; justify-content:center;
  height:72px; border-radius:6px; background:var(--bg-soft); overflow:hidden;
}
.ml-thumb img { max-width:100%; max-height:100%; object-fit:contain; }
/* 样张**整条都要看得见**（2026-10-19，用户报「字体列表显示的有问题」）：以前是 white-space:nowrap
   + 容器 overflow:hidden，97px 的格子放 186px 的样张 ⇒ 右边**半个字被硬裁、连省略号都没有**
   （截图 .workbuddy/fontlist/crop-three-cards.png）。现在允许折行、居中收在 72px 高度内
   —— 宁可换一行，不切字形。 */
.ml-font { font-size:22px; line-height:1.15; text-align:center; white-space:normal; overflow-wrap:anywhere; max-height:100%; overflow:hidden; }
.ml-badge {
  position:absolute; right:3px; bottom:3px;
  font-size:10px; padding:0 4px; border-radius:4px;
  background:var(--main); color:#fff;
}
/* ⚠️ 「无粗体」那枚挪到**左下角**（`.ml-badge` 是右下角绝对定位的，两枚同角会叠成一枚 ——
   第一版就是这样，白写一条信息）。配色走仓里既有的警示口径（`--amber-soft` / `--amber-deep`，
   两个主题都有定义；`App.vue` 的 `.pwd-banner` 与 `app.css` 的 `.chip.a` 同一套）。 */
.ml-badge.warn { right:auto; left:3px; background:var(--amber-soft); color:var(--amber-deep); }
.ml-name { font-size:12px; font-weight:600; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.ml-sub { font-size:11px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }

.ml-sample { width:150px; }
.ml-zoom { display:flex; flex-direction:column; align-items:center; gap:6px; }
.ml-zoom-video { max-width:100%; max-height:60vh; background:#000; border-radius:6px; }
.ml-zoom-img { max-width:100%; max-height:60vh; object-fit:contain; }
.ml-zoom-line { margin:0; font-size:34px; line-height:1.3; }
.ml-zoom-meta { margin:0; }

/* 窄屏：导航转成横排一行（128px 定宽在手机上是纯浪费）。 */
@media (max-width: 720px) {
  .ml-body { flex-direction:column; }
  .ml-nav { flex-direction:row; width:auto; flex-wrap:wrap; }
  /* 预览栏转成整行、并去掉那条竖分隔线（横排时它变成一条突兀的顶边） */
  .ml-side {
    /* 竖排是**上下**两段，此时"栏宽"= 整行宽：`clamp(…, 62%, …)` 在这宽度下会算出
       一个比可视区窄的怪值（62% 是相对**父容器**，竖排后父容器是整个内容区）——
       直接放开成 100%。 */
    width:100%;
    border-left:none; border-top:1px solid var(--line);
    padding-left:0; padding-top:12px;
  }
  .ml-side-grid { grid-template-columns:repeat(auto-fill, minmax(64px, 1fr)); max-height:320px; }
  /* 竖排后栏宽 = 整行，但**窄窗口**下两列挤成一团（每列只剩一百多像素）⇒ 掉回一列。 */
  .ml-side-info { flex-direction:column; gap:5px; }
}

/* 中等宽度：栏还没到 860px，两列各自只剩 ~200px，`文件` 那种长值会被压得很窄。
   这时**并成一列**更好读 —— 用容器查询做不到（父容器是 flex 项，宽度由 clamp 决定），
   所以按窗口宽断一次。 */
@media (max-width: 1100px) {
  .ml-side-info { flex-direction:column; gap:5px; }
}
</style>
