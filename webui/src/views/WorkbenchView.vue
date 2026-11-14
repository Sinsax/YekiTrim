<template>
  <div class="wb-root">
    <div class="page-head wb-head">
      <!-- 页标题在吸顶上下文栏里（`App.vue` 的 `<h1>首页</h1>`）：这里只留一句引导语，
           不再重复同名标题（原来是「首页」上下各一遍，占掉两行垂直空间）。 -->
      <p class="wb-lead">把一段录像变成可发布的片段</p>
      <div class="actions">
        <!-- 运行状况有问题时放一枚短提示（点开跳到卡片并展开）；常驻状态在页尾的运行状况行。 -->
        <button v-if="doctorProblem" class="mini-btn doctor-chip" :title="doctorIssues.join('；')"
                @click="openDoctor">
          <Icon name="alert" :size="13" /> 运行状况：{{ doctorIssues.length }} 项待处理
        </button>
        <button class="btn" :disabled="refreshing" title="重新读取项目、运行状况与环境检查" @click="refresh">
          <span v-if="refreshing" class="spinner"></span>
          <Icon v-else name="refresh" :size="14" />
          {{ refreshing ? '刷新中' : '刷新' }}
        </button>
      </div>
    </div>

    <div v-if="wb.error" class="error-banner"><Icon name="alert" :size="15" /> {{ wb.error }}</div>

    <!-- 「开始使用」页 = 环境检查 + 运行状况；「工作台」页 = 继续上次 → 添加录像 → 项目 →
         最近片段 → 素材库。功能一个都没删：检查项折叠、添加向导折叠、素材库折叠。 -->

    <!-- v7（用户口径）：**不再分「开始使用 / 工作台」两页** —— 环境检查只在没配好时占地方，
         配好之后整页直接是工作台。原来两个页签还带一个副作用：每次切回来都要 v-show 两套 DOM。 -->

    <!-- 环境状态：**仅在未就绪时出现**（配好就彻底不占首屏）。 -->
    <div v-if="!checking && !setupReady" class="card card-blk mb-12 start-hero">
      <Icon :name="setupReady ? 'check' : 'alert'" :size="16" class="sh-ic" :class="{ ok: setupReady }" />
      <div class="sh-main">
        <b class="sh-title">{{ heroTitle }}</b>
        <span class="muted small">{{ heroSub }}</span>
      </div>
      <button class="mini-btn" :disabled="checking" @click="recheck">重新检查</button>
    </div>

    <!-- ═════ 开始使用：运行环境 / 字幕模型 / AI 模型（可选）/ 第一段录像 ═════ -->
    <!-- 就绪后这张卡不再渲染（顶部状态卡已经说清）；只有缺项时才展开逐条明细 -->
    <SetupChecklist
      v-if="!setupReady"
      head="检查项"
      :health="health"
      :doctor="doctor"
      :models="models"
      :project-count="(wb.projects || []).length"
      :loading="checking"
      @open-doctor="openDoctor"
      @open-wizard="openWizard"
      @go-llm="goLlm"
      @refresh="recheck"
    />
    <!-- ═════ 运行状况：这台机器上「干活要用的东西」齐不齐 ═════
         ffmpeg / ffprobe 是剪切与出片的外部依赖，ASR 模型是转写的前提，两者都是
         **全机共享的程序级资产**（跟"谁在登录"无关）—— 所以它们不该住在账号页。
         数据来自 `GET /api/doctor`（外部命令是否可用 + 模型清单）与 `GET /api/models`
         （装在哪一份、多大、怎么下）。写操作（下载）仅管理员，由 ModelList 自己门控。
         ⚠️ **默认折叠成一行**：以前"有问题就 order:-1 冒到页标题之上"，一个可选模型
         没下载就让 300px 的卡片占满首屏、主功能被压到第二屏（用户截图实测）。 -->
    <div id="doctor-card" ref="doctorCard" class="card card-blk doctor-card">
      <button class="doctor-head" data-cap="home.doctor"
              :aria-expanded="doctorOpen ? 'true' : 'false'" @click="doctorOpen = !doctorOpen">
        <Icon name="cpu" :size="15" />
        <b class="t" style="margin:0">运行状况</b>
        <span v-if="doctorIssues.length" class="chip a">{{ doctorIssues.length }} 项待处理</span>
        <span v-else-if="doctor" class="chip g">一切就绪</span>
        <span v-else class="muted small">读取中…</span>
        <span v-if="doctor" class="muted small" title="由服务端在当前进程里实测，不是缓存的旧值">已实测</span>
        <span class="flex-1"></span>
        <span class="muted small">{{ doctorOpen ? '收起' : '展开' }}</span>
        <Icon :name="doctorOpen ? 'chevron-down' : 'chevron-right'" :size="13" />
      </button>
      <div v-show="doctorOpen" class="mt-8">
        <div v-if="doctor" class="row mb-8">
          <span class="chip" :class="doctor.ffmpeg ? 'g' : 'n'">ffmpeg {{ doctor.ffmpeg ? '就绪' : '缺失' }}</span>
          <span class="chip" :class="doctor.ffprobe ? 'g' : 'n'">ffprobe {{ doctor.ffprobe ? '就绪' : '缺失' }}</span>
          <span class="muted small" :title="doctor.models_dir">模型目录</span>
        </div>
        <div v-if="doctorError" class="error-banner mb-8">{{ doctorError }}</div>
        <ModelList :models="models" @changed="loadDoctor" />
      </div>
    </div>

    <!-- 概览 + 继续上次：**同一行**（2026-11-13 用户口径）。
         左边是动作（继续哪个项目 / 下一步做什么），右边是四项读数，最右是主按钮；
         窄屏自动换行（读数整行铺开，不挤压项目名）。
         ⚠️ 片段与出片只能从 per-pid 缓存算，没加载过时显示 `—`（别报 0 冒充"一片都没有"）。 -->
    <div class="card card-blk mb-12 wb-top" :class="{ 'wb-top--empty': !continueInfo }">
      <router-link
        v-if="continueInfo"
        class="wb-top-continue"
        data-cap="home.continue"
        :to="`/p/${continueInfo.pid}/${continueInfo.step}`"
      >
        <span class="cc-icon"><Icon name="play" :size="15" /></span>
        <span class="cc-body">
          <b class="cc-title" :title="continueInfo.name">继续：{{ continueInfo.name }}</b>
          <span class="muted small" :title="continueInfo.tech">下一步：{{ continueInfo.label }}</span>
        </span>
      </router-link>

      <div class="wb-top-stats">
        <UiStat :value="(wb.projects || []).length" label="项目" />
        <UiStat :value="totals.known ? totals.clips : '—'" label="片段" />
        <UiStat :value="totals.known ? totals.exported : '—'" label="已出片" tone="ok" />
        <UiStat :value="fmtBytes(totals.bytes)" label="占用" />
      </div>

      <router-link v-if="continueInfo" class="btn btn-primary cc-go"
                   :to="`/p/${continueInfo.pid}/${continueInfo.step}`">继续 →</router-link>
    </div>

    <!-- ═════ 切片总览：全部项目的切片卡片（一面墙 + 筛选/搜索；空时不占地方） ═════ -->
    <HomeClipsOverview />

    <!-- ═════ 添加录像：三个来源磁贴，点哪个只开哪个 ═════
         用户反馈"弹窗里三列并排还是不方便"：现在把**来源本身**做成首屏可见的磁贴，
         点一个只打开那一个来源的表单，不再"先开弹窗、再从三列里挑"。 -->
    <!-- ═════ 添加录像：**左选来源 / 右填参数**（用户口径：做成设置页那种两栏） ═════
         不再"磁贴 → 弹窗 → 表单"三层跳转：来源本身就是左栏的一项，点它右栏原地换成那个来源的表单。 -->
    <section ref="addCard" class="card card-blk mb-12 add-src" data-cap="home.add-project">
      <div class="row spread mb-8">
        <b class="t m-0">添加录像</b>
        <span class="muted small">左边选来源，右边填参数</span>
      </div>
      <div class="add-split">
        <nav class="add-nav" aria-label="录像来源">
          <button v-if="isAdmin" type="button" class="add-nav-item"
                  :class="{ on: addSource === 'local' }"
                  data-cap="home.source-local" @click="openSource('local')">
            <Icon name="folder" :size="16" />
            <span class="add-nav-body"><b>服务器文件</b><span>已有录像或已有工程</span></span>
          </button>
          <button type="button" class="add-nav-item"
                  :class="{ on: addSource === 'recorder', off: recConfigured === false }"
                  data-cap="home.source-recorder" @click="openSource('recorder')">
            <Icon name="download" :size="16" />
            <span class="add-nav-body"><b>录播接入</b><span>{{ recConfigured === false ? '播软件未配置' : '挑一场录像' }}</span></span>
          </button>
          <button type="button" class="add-nav-item"
                  :class="{ on: addSource === 'bili', off: dl.ready === false }"
                  data-cap="home.source-bilibili" @click="openSource('bili')">
            <Icon name="film" :size="16" />
            <span class="add-nav-body"><b>B站链接</b><span>{{ dl.ready === false ? '下载器未就绪' : '粘贴链接下载' }}</span></span>
          </button>
        </nav>
        <div class="add-body">


        <!-- ① 服务器文件（仅管理员可见；非管理员连来源按钮都没有）
             两种选法**都要**（用户口径："能输入选择 + 弹出文件浏览器选择"，是**增加**不是替代）：
               · **输入选择** = 下面这份内联 ServerDirPicker（可直接填/粘贴服务器绝对路径，
                 就地列目录、点文件、用快捷入口）—— 与字体库那份同一套。
               · **弹出浏览** = 上面那颗按钮开大窗（`AppModal wide`），深目录导航时视野更宽。
             两者写同一个已选状态；弹出层额外用**草稿态**，只有点「选定」才写回（否则
             "进去逛一圈又关掉"会把中途路过的目录当成选定）。 -->
        <!-- 服务器文件：**左 = 目录操作（浏览 / 选文件）· 右 = 按钮与注意事项**。
             用户口径："左边是目录操作，右边是各种按钮和注意事项"。 -->
        <!-- 服务器文件 = **文件管理器**：常用位置 / 双栏浏览器 / 操作区，纵向三段。
             浏览器就是原来「弹出浏览」里那个组件，只把 `ServerFileBrowser` 直接嵌在这（不再弹窗）。 -->
        <div v-if="addSource === 'local'" class="local-manager">
          <div class="local-pins">
            <span class="muted small flex-none">常用位置</span>

            <!-- 预设位置：服务端给的快捷入口（我的托管目录…），**不可移除** -->
            <span v-for="r in presetRoots" :key="'preset:' + r.path"
                  class="pin-chip preset" :class="{ on: r.path === localDir }"
                  :title="r.path + '（预设位置，不可移除）'">
              <Icon name="home" :size="11" class="pin-ic" />
              <button type="button" class="pin-main" @click="goPin(r.path)">{{ r.label }}</button>
            </span>

            <!-- 用户钉固的位置：右侧带显式 ×（右键也能取消，但那不是唯一入口） -->
            <span v-for="p in pinnedDirs" :key="'user:' + p"
                  class="pin-chip" :class="{ on: p === localDir }" :title="p">
              <button type="button" class="pin-main" @click="goPin(p)">{{ pinLabel(p) }}</button>
              <button type="button" class="pin-x" :title="'取消钉固：' + p"
                      :aria-label="'取消钉固 ' + p" @click.stop="unpinDir(p)">
                <Icon name="x" :size="10" />
              </button>
            </span>

            <button type="button" class="mini-btn pin-add"
                    :disabled="!localDir || pinnedDirs.includes(localDir) || isPresetDir(localDir)"
                    :title="!localDir ? '先在浏览器里进入一个目录'
                      : isPresetDir(localDir) ? '这个位置已经是预设，不用再钉'
                      : pinnedDirs.includes(localDir) ? '这个位置已经钉过了'
                      : '把当前目录钉到常用位置'"
                    @click="pinCurrent">★ 钉固当前</button>
          </div>

          <div class="local-browser-body">
            <ServerFileBrowser
              :start-dir="localDir || projectsRootEffective"
              :selected="localInput"
              :exts="LOCAL_EXTS"
              :project-file="PROJECT_FILE"
              hide-roots
              @update:dir="onLocalDir"
              @update:listing="localListing = $event"
              @pick-file="pickLocalFile"
              @update:roots="presetRoots = $event || []"
            />
          </div>

          <div class="local-ops">
            <div v-if="localDir" class="local-state">
              <div v-if="localHasConfig && localManualVideo" class="warn-text">
                已有工程，将<b>重新初始化</b>（要确认）
              </div>
              <div v-else-if="localHasConfig" class="ok-text">
                检测到已有工程（<span class="mono">{{ PROJECT_FILE }}</span>）—— 将<b>直接添加</b>
              </div>
              <div v-else-if="localEffectiveVideo" class="ellip">
                输入视频 <span class="mono">{{ localEffectiveVideo }}</span>
                <span v-if="!localManualVideo" class="muted">（自动取第一个）</span>
              </div>
              <div v-else-if="!localListing.failed" class="warn-text">这个目录没有可用视频</div>
            </div>
            <div class="local-actions">
              <span class="local-where ellip mono" :title="localDir">
                {{ localDir ? '建在 ' + localDir : '还没选目录' }}
              </span>
              <button class="btn btn-primary flex-none" :disabled="creating || !canCreateLocal"
                      :title="canCreateLocal ? '按选中的目录建项目' : '先选一个含视频的目录，或含 yekitrim.toml 的工程目录'"
                      @click="createFromLocal">
                <span v-if="creating" class="spinner"></span> {{ localAddMode ? '添加项目' : '新建项目' }}
              </button>
            </div>
          </div>
        </div>

        <!-- ② 录播接入：上=房间，下=该房间的录像（可添加；已添加的置灰防重复） -->
        <div v-else-if="addSource === 'recorder'" class="add-pane add-col-recorder" :class="{ off: recConfigured === false }">
          <div v-if="recLoading" class="row muted small add-pane-wide"><span class="spinner"></span> 读取房间…</div>
          <!-- 连不上 / 有问题：**上=原因，下=两个动作**（2026-11-14 用户口径"应该显示重新连接和跳转设置，
               然后排版为上下两行"）。以前是一行里塞「原因 + 重试」——挤，而且只有一个出口：
               真实故障十有八九是地址/端口/账号配错，最该给的入口恰恰是「去设置」。 -->
          <div v-else-if="recError" class="error-banner add-pane-wide rec-fail">
            <span class="rec-fail-msg">
              <Icon name="alert" :size="15" />
              <span class="rec-fail-text">{{ recError }}</span>
            </span>
            <span class="rec-fail-acts">
              <button class="mini-btn" :disabled="recLoading" @click="loadRecObjects">重新连接</button>
              <router-link v-if="isAdmin" class="mini-btn" to="/settings/integrations">去设置</router-link>
            </span>
          </div>
          <div v-else-if="recConfigured === false" class="room-down add-pane-wide rec-fail">
            <span class="rec-fail-msg">
              <Icon name="alert" :size="15" />
              <span class="rec-fail-text">
                <b>录播软件未配置</b>
                <span class="muted small">去「设置 → 接入」选 biliLive-tools 或录播姬并填地址。</span>
              </span>
            </span>
            <span class="rec-fail-acts">
              <button class="mini-btn" :disabled="recLoading" @click="loadRecObjects">重新连接</button>
              <router-link v-if="isAdmin" class="mini-btn" to="/settings/integrations">去设置</router-link>
            </span>
          </div>
          <template v-else>
            <!-- 左：房间。列表本身用 `.add-nav-item` 同一套观感（与来源导航一致） -->
            <div class="add-pane-left">
              <div class="pane-col-head">
                <span>房间</span>
                <span class="val">{{ recObjects.length }}</span>
              </div>
              <div class="room-list">
                <button v-for="o in recObjects" :key="o.id" type="button"
                        class="room-item" :class="{ on: recSelected && recSelected.id === o.id }"
                        :title="o.title || o.id" @click="selectRoom(o)">
                  <span class="room-name">{{ o.title || o.id }}</span>
                  <span class="room-count">{{ o.files }}</span>
                </button>
                <div v-if="!recObjects.length" class="muted small">还没有房间</div>
              </div>
            </div>

            <!-- 右：该房间的录像 = **表格**（一行放得下大小/时间/状态，比原来的列表密得多） -->
            <div class="add-pane-right">
              <div class="pane-col-head">
                <span>录像</span>
                <span class="val">{{ recSelected ? (recSelected.title || recSelected.id) : '未选择房间' }}</span>
              </div>
              <div v-if="recRecLoading" class="row muted small"><span class="spinner"></span> 读取录像…</div>
              <div v-else-if="recRecError" class="hint">{{ recRecError }}</div>
              <UiEmpty v-else-if="!recSelected" inline icon="download" title="先选一个房间"
                       description="左边列出录播软件里的房间，选中后这里显示它的录像" />
              <div v-else class="rec-cards">
                <!-- 一场录像 = 一张卡（用户口径：表格一行一条太占高度、录播一多难找）。
                     卡片网格一行两张，标题两行截断，底部一行「大小 · 时间」+ 添加。 -->
                <article v-for="r in recRecordings" :key="r.dir"
                         class="rec-card" :class="{ added: r.added }" :title="r.dir">
                  <div class="rec-card-name">{{ r.name || r.dir }}</div>
                  <div class="rec-card-foot">
                    <span class="rec-card-meta">{{ fmtBytes(r.size) }} · {{ fmtDate(r.mtime) }}</span>
                    <button class="mini-btn" :class="{ primary: !r.added }" :disabled="r.added"
                            :title="r.added ? '已经添加过，不再重复建项目' : '按这场录像建项目'"
                            @click="addRecording(r)">
                      {{ r.added ? '已添加' : '添加' }}
                    </button>
                  </div>
                </article>
                <div v-if="!recRecordings.length" class="muted small rec-none">这个房间还没有录像</div>
              </div>
            </div>
          </template>
        </div>

        <!-- ③ B 站链接：粘贴 → 解析预览 → 勾选分P → 下载 -->
        <!-- B站链接 = **上下结构**（用户口径）：上面输入+解析，下面出结果与下载。
             原来左右两列时左边输入框孤零零、右边一大片空，很难看。 -->
        <div v-else-if="addSource === 'bili'" class="bili-pane">
          <!-- 前置条件（下载器未装）——跨整宽的横幅，不参与上下排布 -->
          <div v-if="dl.ready === false" class="bili-gate">
            <div class="row items-start">
              <Icon name="alert" :size="15" />
              <div class="flex-1">
                <b>下载器未就绪</b>
                <div class="muted small mt-4">
                  B 站下载需要 <span class="mono">yt-dlp</span>（{{ dl.license || 'Unlicense' }}，
                  {{ dl.pinned_version ? '版本 ' + dl.pinned_version : '内置' }}）。
                  <template v-if="isAdmin">点右边按钮下载并校验（约 40 MB，只下一次）。</template>
                  <template v-else>请让管理员在「设置 → 采集与下载」里安装</template>
                </div>
                <div v-if="dlError" class="error-banner mt-6">{{ dlError }}</div>
              </div>
              <button v-if="isAdmin" class="btn btn-primary" :disabled="dlBusy" @click="installDownloader">
                <span v-if="dlBusy" class="spinner"></span> 下载并校验
              </button>
            </div>
          </div>

          <template v-else>
            <!-- 存放位置：默认位置本来就可用，只说清楚存哪，不拦下载 -->
            <div v-if="isAdmin && rootConfigured === false" class="bili-hint">
              <Icon name="folder" :size="14" />
              <span class="flex-1">
                下载会存到默认位置 <span class="mono">{{ projectsRootEffective || '（程序目录下的 projects）' }}</span>
                <span class="muted">· 可在「设置 → 下载」改成别的目录</span>
              </span>
              <router-link class="mini-btn" to="/settings/integrations">更改</router-link>
            </div>

            <!-- ① 上：输入 + 解析 -->
            <div class="bili-input">
              <div class="row">
                <input
                  v-model="biliUrl"
                  class="flex-1"
                  :disabled="biliBusy"
                  placeholder="粘贴视频链接或 BV 号，例如 https://www.bilibili.com/video/BV1xx  /  BV1xx"
                  @keyup.enter="resolveBili"
                >
                <button class="btn btn-primary flex-none" :disabled="biliBusy || !biliUrl.trim()" @click="resolveBili">
                  <span v-if="biliBusy && !biliInfo" class="spinner"></span> 解析
                </button>
              </div>
              <div class="row spread bili-input-hint">
                <span class="muted small" title="番剧 / 课程 / UP 主空间 / 收藏夹等批量链接不支持，认不出来会明确报错">
                  只支持单个视频与它的分P
                </span>
                <button v-if="biliInfo" class="mini-btn" @click="resetBili">换个链接</button>
              </div>
              <div v-if="biliError" class="error-banner mt-6"><Icon name="alert" :size="15" /> {{ biliError }}</div>
            </div>

            <!-- ② 下：视频信息（防下错）+ 分P 勾选 + 下载 -->
            <div class="bili-result">
              <UiEmpty v-if="!biliInfo" inline icon="film" title="还没有解析结果"
                       description="在上面粘贴链接后点「解析」，这里会列出视频信息与分P" />

              <template v-else>
                <div class="bili-info">
                  <img v-if="biliInfo.cover" class="bili-cover" :src="biliInfo.cover" alt="" referrerpolicy="no-referrer">
                  <div class="bili-meta">
                    <b class="bili-title" :title="biliInfo.title">{{ biliInfo.title }}</b>
                    <div class="muted small">
                      {{ biliInfo.uploader || '（未知 UP 主）' }}
                      <template v-if="biliInfo.duration_secs"> · {{ fmtClock(biliInfo.duration_secs) }}</template>
                      <template v-if="biliInfo.danmaku_total"> · 弹幕 {{ fmtCount(biliInfo.danmaku_total) }}</template>
                      <template v-if="biliInfo.view_total"> · 播放 {{ fmtCount(biliInfo.view_total) }}</template>
                      · 清晰度 {{ biliInfo.quality }}
                    </div>
                  </div>
                  <div v-if="biliInfo.pages.length > 1" class="row bili-pick-tools">
                    <button class="mini-btn" @click="pickAllPages">全选</button>
                    <button class="mini-btn" @click="pickedPages = []">清空</button>
                  </div>
                </div>

                <div class="p-cards">
                  <label v-for="p in biliInfo.pages" :key="p.index"
                         class="p-card" :class="{ on: pickedPages.includes(p.index) }"
                         :title="p.part || `P${p.index}`">
                    <input type="checkbox" :value="p.index" v-model="pickedPages">
                    <span class="p-card-head">
                      <span class="p-card-num">P{{ p.index }}</span>
                      <span class="p-card-dur">{{ p.duration_secs ? fmtClock(p.duration_secs) : '—' }}</span>
                    </span>
                    <span class="p-card-name">{{ p.part || '（无标题）' }}</span>
                    <span class="p-card-est">{{ p.estimate_bytes ? fmtBytes(p.estimate_bytes) : '—' }}</span>
                  </label>
                </div>

                <div class="bili-submit">
                  <span class="bili-submit-sum">
                    已选 <b>{{ pickedPages.length }}</b> 个 · 预计 <b>{{ fmtBytes(pickedEstimate) }}</b>
                    <span v-if="!pickedPages.length" class="muted">（先勾选）</span>
                  </span>
                  <button
                    class="btn btn-primary"
                    :disabled="biliSubmitting || !pickedPages.length"
                    title="下载完成后自动建项目；弹幕会一并抓取（事件检测的热度排序要用）"
                    @click="submitBili"
                  >
                    <span v-if="biliSubmitting" class="spinner"></span>
                    开始下载{{ pickedPages.length > 1 ? `（${pickedPages.length} 个）` : '' }}
                  </button>
                </div>

                <!-- 下载进度：每条 = 一 P，状态来自 `/api/ingest/records` 轮询。
                     ⚠️ 没有字节级进度（后端只统计已完成文件），未完成时走不确定动画条。 -->
                <div v-if="biliJobs.length" class="bili-jobs">
                  <div class="row spread">
                    <b class="small">下载任务（{{ biliJobs.length }}）</b>
                    <router-link class="mini-btn" to="/ingest">全部记录</router-link>
                  </div>
                  <div v-for="j in biliJobs" :key="j.session_id" class="bili-job">
                    <div class="bili-job-row">
                      <span class="bili-job-name ellip" :title="j.title">P{{ j.page }} {{ j.title }}</span>
                      <span class="bili-job-meta muted small">
                        <template v-if="j.bytes">{{ fmtBytes(j.bytes) }} · </template>{{ j.state_label || j.state }}
                      </span>
                      <span class="chip" :class="biliJobChip(j)">{{ j.state_label || '排队中' }}</span>
                      <router-link v-if="j.project_id" class="mini-btn"
                                   :to="`/p/${j.project_id}/prepare`">打开项目</router-link>
                    </div>
                    <div class="progress" :class="{ done: biliJobPct(j) === 100, failed: j.state === 'failed' }">
                      <div v-if="biliJobPct(j) === 100" style="width:100%"></div>
                      <div v-else class="indet"></div>
                    </div>
                    <div v-if="j.error" class="small err ellip" :title="j.error">{{ j.error }}</div>
                  </div>
                  <div v-for="sk in (biliSubmitted?.skipped || [])" :key="sk.session_id"
                       class="muted small">P{{ sk.page }} 已跳过（{{ sk.reason }}）</div>
                </div>
              </template>
            </div>
          </template>
        </div>
        </div>
      </div>
    </section>

    <!-- 素材库 = **视频 / 贴纸 / 字体 三合一**，**文件夹管理器**布局（左侧选类型、右侧网格）。
         用户 2026-10-09：「素材库ui改成文件夹管理器类似的感觉，左侧选择类型视频，贴纸，字体」
         +「统一一下导入方式，都需要预览效果」。
         ⚠️ 2026-10-09 起素材库是**用户级资产**：三种素材的导入/删除**登录用户都能做**
         （`is-admin` 保留只为兼容传参，界面本身不再按它门控）。
         ⚠️ **别再退回"三块各自实现"** —— 那正是用户要改掉的（同一个动作三种手感）。 -->
    <MediaLibrary :is-admin="isAdmin" />
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Icon from '../components/Icon.vue'
import ServerDirPicker, { VIDEO_EXTS } from '../components/ServerDirPicker.vue'
import AppModal from '../components/AppModal.vue'
import AppEmptyState from '../components/AppEmptyState.vue'
import ServerFileBrowser from '../components/ServerFileBrowser.vue'
import SetupChecklist from '../components/SetupChecklist.vue'
import MediaLibrary from '../components/MediaLibrary.vue'
import HomeClipsOverview from '../components/HomeClipsOverview.vue'
import ModelList from '../components/ModelList.vue'
import UiStat from '../components/ui/UiStat.vue'
import UiEmpty from '../components/ui/UiEmpty.vue'
import { authState } from '../stores/auth'
import { localNoAuthKnown, healthPayload, refreshHealth } from '../stores/health'
import { createLocalProject } from '../stores/project'

/** 常用位置（钉固目录）：localStorage 存绝对路径列表，最多 8 个 */
const PIN_KEY = 'yekitrim.local.pins'
function readPins() {
  try {
    const raw = JSON.parse(localStorage.getItem(PIN_KEY) || '[]')
    return Array.isArray(raw) ? raw.filter((x) => typeof x === 'string' && x) : []
  } catch { return [] }
}
const pinnedDirs = ref(readPins())
/**
 * **预设位置**：服务端返回的快捷入口（如「我的托管目录」= 项目托管根）。
 * 它们与用户钉固的目录并列显示，但**不可移除** —— 那是"程序给定的落点"，
 * 删掉之后用户会以为系统没有可用目录。
 */
const presetRoots = ref([])
function savePins() {
  try { localStorage.setItem(PIN_KEY, JSON.stringify(pinnedDirs.value)) } catch { /* 隐私模式忽略 */ }
}
/** 钉固**当前目录**（重复不追加；最多留最近 8 个） */
/** 当前目录是否已经是"预设位置"（预设的不能重复钉固、也不能移除） */
function isPresetDir(d) {
  return presetRoots.value.some((r) => r.path === d)
}
function pinCurrent() {
  const d = String(localDir.value || '').trim()
  if (!d || pinnedDirs.value.includes(d) || isPresetDir(d)) return
  pinnedDirs.value = [...pinnedDirs.value, d].slice(-8)
  savePins()
}
function unpinDir(d) {
  pinnedDirs.value = pinnedDirs.value.filter((x) => x !== d)
  savePins()
}
/** chip 上只显示末级目录名（完整路径在 title） */
function pinLabel(p) {
  const parts = String(p).split(/[\\/]/).filter(Boolean)
  return parts[parts.length - 1] || p
}
/** 点常用位置 = 跳过去（组件 watch startDir 会跟着 go） */
function goPin(d) {
  if (d === localDir.value) return
  localDir.value = d
  localInput.value = ''
  localListing.value = { dir: d, files: [], dirs: [] }
}

/** 工程标志文件（与后端 `path::PROJECT_CONFIG_FILE` 同名） */
const PROJECT_FILE = 'yekitrim.toml'
/**
 * 「服务器文件」这条来源**只允许选两类**：工程文件（= 直接添加已有工程）与视频文件
 * （= 当输入视频 / 重建）。其余一律非可选（选择器会标「不可选」）。
 * ⚠️ 必须让 `toml` 也在白名单里：否则 `yekitrim.toml` 那行是"看得见点不动"，
 * 而"选工程文件直接加载工程"正是这条路的一半语义。
 */
const LOCAL_EXTS = [...VIDEO_EXTS, 'toml']
const isVideoName = (n) => VIDEO_EXTS.includes(String(n || '').split('.').pop().toLowerCase())
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import { ingestApi } from '../api/ingest'
import { fmtBytes, fmtTime, ingestState, loadRecords, setReady } from '../stores/ingest'
import { fmtClock, fmtCount, nextStep, stageInfo, STEPS } from '../utils'
import {
  cacheOf,
  loadProjects,
  polishedCountOf,
  sortProjectsNewestFirst,
  uncutEventsOf,
  wb
} from '../stores/workbench'
import { api } from '../api/client'

// ---- 首页两页签 ----
// 「开始使用」= 环境检查 + 运行状况；「工作台」= 项目 / 素材 / 直接干活。
// 默认页由**环境是否配好**决定：第一次进来（或还有待处理项）先看第一页，配好之后直接进第二页。
// ⚠️ 用户点过页签之后就不再自动翻页（`tabTouched`），否则刷新一次就把人弹走。
/** 环境/模型/项目都就绪 = 可以直接干活（判据与 SetupChecklist 的 allReady 一致）。 */
const setupReady = computed(() => {
  const h = health.value
  const d = doctor.value
  if (!h || !d) return false
  const envOk = !!h.ffmpeg && !!h.ffprobe && !!h.dhampir_ready
  const modelsOk = !(d.models || []).some((m) => !m.installed)
  return envOk && modelsOk && (wb.projects || []).length > 0
})

/** 还缺哪几项（只数必做的三项；AI 模型是可选项，不进这里）。 */
const setupMissing = computed(() => {
  const h = health.value
  const d = doctor.value
  if (!h || !d) return []
  const out = []
  if (!h.ffmpeg || !h.ffprobe || !h.dhampir_ready) out.push('运行环境')
  if ((d.models || []).some((m) => !m.installed)) out.push('字幕模型')
  if (!(wb.projects || []).length) out.push('第一段录像')
  return out
})
const heroTitle = computed(() => {
  if (checking.value) return '正在检查环境…'
  if (setupReady.value) return '环境已就绪'
  return setupMissing.value.length ? `还差 ${setupMissing.value.length} 项没配好` : '还没配好'
})
const heroSub = computed(() => (setupReady.value
  ? '运行环境与字幕模型都齐了，去「工作台」建项目 / 挑片段。'
  : '按下面的检查项逐条处理，配好就能开工。'))

const isAdmin = computed(() => authState.user?.role === 'admin')
const router = useRouter()
const route = useRoute()

// ---- 继续上次项目 ----
//
// 优先 `wb.last`（上次打开过的项目），认不出来（已移除 / 本地存储残留）就用最新加入的。
// 详情只在校验通过（`cache.status` 非空）时才给 `nextStep`，否则按项目 stage 粗判。
const continueProject = computed(() => {
  const list = wb.projects || []
  if (!list.length) return null
  const last = wb.last?.pid
    ? list.find((p) => String(p.id) === String(wb.last.pid))
    : null
  return last || sortProjectsNewestFirst(list)[0] || null
})
const continueInfo = computed(() => {
  const p = continueProject.value
  if (!p) return null
  const c = cacheOf(p.id)
  const detail = c.status
    ? { clips: c.clips, uncutEvents: uncutEventsOf(p.id), polishedCount: polishedCountOf(p.id) }
    : null
  const n = nextStep(p, detail)
  const tech = STEPS.find((x) => x.key === n.step)?.tech || ''
  return { pid: p.id, name: p.name || ('项目 #' + p.id), step: n.step, label: n.label, tech }
})

// ---- 添加录像：**弹窗向导**（v6 起三个来源面板不再常驻首页）----
// 常驻三列会把首屏撑到 ~2000px：真正的主行动「继续上次」被挤到第一屏之外，
// 而"添加录像"绝大多数时候一天只点一两次。改成入口卡 + 弹窗后首屏 ~900px。
const addSource = ref('')
function openSource(k) {
  addSource.value = k
}
/** 首次进页面就给一个合理默认（否则右栏是空的，看起来像没加载出来） */
function defaultSource() {
  if (addSource.value) return addSource.value
  if (isAdmin.value) return 'local'
  if (recConfigured.value !== false) return 'recorder'
  return 'bili'
}
function openAdd() {
  // 首页只有一页：不切页，直接滚到「添加录像」并选中一个来源（左栏 + 右栏一起露出来）
  addSource.value = defaultSource()
  requestAnimationFrame(() => addCard.value?.scrollIntoView({ block: 'center', behavior: 'smooth' }))
}
/** 兼容旧调用点（`SetupChecklist` 的 open-wizard、`?add=1`） */
function openWizard() {
  openAdd()
}

// ---- /api/health（首页 checklist 与来源默认都读它；缓存只在 stores/health.js）----
const health = ref(null)
const checking = ref(true)
async function loadHealth(force = false) {
  // 首次挂载优先用 store 里已有的载荷（路由守卫刚问过 `/api/health`），避免白打一次；
  // 从设置页回来 / 用户点刷新 / 「重新检查」走 force，保证拿到最新状态。
  const h = force ? await refreshHealth() : (healthPayload() || await refreshHealth())
  if (!h) return
  health.value = h
  // 采集就绪位顺手同步（原先在 onMounted 里直接读 /api/health，现在统一走 store）
  setReady(h.ingest_ready !== false)
  // 下载器就绪位顺便从 health 拿一次（省一个请求），细节（版本/许可）由 loadDownloader 补
  if (h.downloader) {
    Object.assign(dl, {
      ready: h.downloader.ready === true,
      verified: h.downloader.verified === true,
      kind: h.downloader.kind || ''
    })
  }
}
/** checklist 的「重新检查」：置 loading（它会把 checklist 收成一行"正在检查"）。 */
async function recheck() {
  checking.value = true
  await Promise.allSettled([loadDoctor(), loadHealth(true)])
  checking.value = false
}
function goLlm() {
  // 设置页是 admin-only；普通用户点过来只给一句解释，别让路由守卫把它弹回首页。
  if (isAdmin.value) router.push('/settings/llm')
  else toast('AI 模型由管理员配置')
}

// 来源选择：不同来源 → 不同导入方式（本地浏览 / 录播姬待处理 / B站链接）。
// 状态声明放在 `dl` 之后（默认来源要等下载器/录播姬的可用位回来才能定）。
// 「服务器文件」是 admin-only 来源（`/api/fs/list`、`kind:"local"` 都对非管理员 403），
// 所以非管理员**整条不显示** —— 不给一个点不动、只写着"仅管理员"的入口。

// ---- ② 我的直播间（录播姬）----
const rooms = ref([])
/// null = 还没问到（别先闪一下「未接入」）；true/false = 录播姬可用位
const roomsAvailable = ref(null)
const roomsLoading = ref(false)
const roomInput = ref('')
const roomBusy = ref(false)
const recorderTesting = ref(false)
const recorderInfo = ref('')
/** 直播间读取失败（与"录播姬没接入"分开：读失败不许伪装成未接入） */
const roomsError = ref('')
/** 录播姬为什么连不上（仅 admin 可见：不向普通用户泄露管理配置） */
const roomsReason = ref('')
/** 录播模块（biliLive-tools / 录播姬 二选一）的录制对象 */
const recConfigured = ref(null)
const recObjects = ref([])
const recError = ref('')
const recLoading = ref(false)
/** 选中房间 → 下列表显示它的录像 */
const recSelected = ref(null)
const recRecordings = ref([])
const recRecLoading = ref(false)
const recRecError = ref('')

async function loadRecObjects() {
  recLoading.value = true
  recError.value = ''
  try {
    const d = await api('/api/recorder/objects')
    recConfigured.value = d?.configured === true
    recObjects.value = d?.objects || []
    if (d?.error) recError.value = d.error
  } catch (e) {
    recError.value = '录播读取失败：' + (e?.message || '')
    recConfigured.value = null
  } finally {
    recLoading.value = false
  }
}

async function selectRoom(o) {
  recSelected.value = o
  recRecordings.value = []
  await loadRecordings()
}

async function loadRecordings() {
  if (!recSelected.value) return
  recRecLoading.value = true
  recRecError.value = ''
  try {
    const d = await api(`/api/recorder/recordings?room_id=${encodeURIComponent(recSelected.value.id)}`)
    recRecordings.value = d?.recordings || []
  } catch (e) {
    recRecError.value = '录像读取失败：' + (e?.message || '')
  } finally {
    recRecLoading.value = false
  }
}

/** 按录像目录建项目；`added` 由服务端按已有工程判定，前端再置灰防重复 */
async function addRecording(r) {
  if (!r?.dir || r.added) return
  try {
    const res = await api('/api/projects', {
      method: 'POST',
      body: JSON.stringify({ source: { kind: 'local', dir: r.dir, input: '', danmaku: '', output_dir: '', overwrite: false } })
    })
    r.added = true
    toast(res?.id ? '已添加' : '已存在，未重复添加')
    await loadProjects()
    if (res?.id) window.location.hash = `#/p/${res.id}/prepare`
  } catch (e) {
    toast('添加失败：' + e.message, 'err')
  }
}

function fmtDate(ts) {
  if (!ts) return ''
  const d = new Date(ts * 1000)
  const p = (n) => String(n).padStart(2, '0')
  return `${d.getMonth() + 1}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

/** 通用 Webhook（biliLive-tools）是否已启用/** 通用 Webhook（biliLive-tools）是否已启用 —— 用来解释"为什么这里不用加直播间" */
const webhookReady = ref(false)

/** 我的直播间 + 录播姬可用位。普通用户的响应里没有 `reason`（不暴露管理配置）。 */
async function loadRooms() {
  roomsLoading.value = true
  roomsError.value = ''
  try {
    const d = await api('/api/rooms')
    rooms.value = d?.rooms || []
    roomsAvailable.value = d?.available === true
    if (isAdmin.value && d?.reason) {
      recorderInfo.value = d.reason
      roomsReason.value = d.reason
    }
  } catch (e) {
    // ⚠️ 读失败 ≠ 录播姬未接入：置 null（"还不知道"）并单独出错误条 + 重试，
    // 别让一次网络抖动把整块换成"录播姬未接入，暂时不能添加"。
    roomsAvailable.value = null
    roomsError.value = '直播间读取失败：' + (e?.message || '')
  } finally {
    roomsLoading.value = false
  }
}

async function addRoom() {
  const v = roomInput.value.trim()
  if (!v || roomBusy.value || !roomsAvailable.value) return
  roomBusy.value = true
  try {
    await api('/api/rooms', { method: 'POST', body: JSON.stringify({ url: v }) })
    roomInput.value = ''
    toast('已添加直播间，开播时会自动录制')
    await loadRooms()
  } catch (e) {
    toast('添加失败：' + e.message, 'err')
  } finally {
    roomBusy.value = false
  }
}

async function removeRoom(r) {
  const ok = await confirmDialog(`删除直播间 ${r.room_id}？删除后不再自动录制（已录的片段与项目不受影响）。`, {
    title: '删除直播间？',
    danger: true
  })
  if (!ok) return
  roomBusy.value = true
  try {
    await api(`/api/rooms/${encodeURIComponent(r.room_id)}`, { method: 'DELETE' })
    toast('已删除')
    await loadRooms()
  } catch (e) {
    toast('删除失败：' + e.message, 'err')
  } finally {
    roomBusy.value = false
  }
}

function roomState(r) {
  if (r.recording) return '录制中'
  if (r.streaming) return '在播'
  if (!r.in_recorder) return '未在录播姬'
  return '未开播'
}
function roomChip(r) {
  if (r.recording) return 'r'
  if (r.streaming) return 'g'
  return 'n'
}

// ---- 房间行展开：该房间在录像目录里的文件（单个导入）----

/** 房间的录像汇总（服务端随 /api/rooms 一起回；没有就是 0）。 */
function recOf(r) {
  const rec = r?.recordings || {}
  return { files: rec.files || 0, bytes: rec.bytes || 0, latest: rec.latest || 0 }
}

/** 展开状态 + 每个房间的文件列表缓存（懒加载：点开才去读目录）。 */
const expanded = reactive({})
const roomFiles = reactive({})
const importing = reactive({})

async function loadRoomFiles(r, force = false) {
  const id = r.room_id
  if (roomFiles[id]?.loading) return
  if (roomFiles[id] && !force) return
  roomFiles[id] = { loading: true, error: '', files: [], total: 0, shown: 0 }
  try {
    const d = await api(`/api/rooms/${encodeURIComponent(id)}/recordings`, { label: '读取该房间的录像' })
    roomFiles[id] = {
      loading: false,
      error: '',
      files: d?.files || [],
      total: d?.total || 0,
      shown: d?.shown || 0
    }
  } catch (e) {
    roomFiles[id] = { loading: false, error: e.message, files: [], total: 0, shown: 0 }
  }
}

async function toggleRoom(r) {
  const id = r.room_id
  expanded[id] = !expanded[id]
  if (expanded[id]) await loadRoomFiles(r)
}

function recChip(f) {
  if (f.state === 'imported') return 'g'
  if (f.state === 'failed') return 'r'
  if (f.state === 'importing') return 'b'
  return 'n'
}

/** 导入**单个**录像：服务端建会话 + 归属本人 + 跑 auto_pipeline。 */
async function importFile(r, f) {
  if (importing[f.rel]) return
  importing[f.rel] = true
  try {
    await api(`/api/rooms/${encodeURIComponent(r.room_id)}/recordings/import`, {
      method: 'POST',
      body: JSON.stringify({ rel: f.rel }),
      label: '导入录像'
    })
    toast('已交给入库流程（跑完会出现在你的项目里）')
    await Promise.all([loadRoomFiles(r, true), loadRooms()])
  } catch (e) {
    toast('导入失败：' + e.message, 'err')
    await loadRoomFiles(r, true)
  } finally {
    importing[f.rel] = false
  }
}

async function testRecorder() {
  recorderTesting.value = true
  try {
    const d = await api('/api/admin/recorder/test', { method: 'POST', body: '{}' })
    recorderInfo.value = d?.ok
      ? `录播姬已连接 ${d.base}${d.version ? ' · v' + d.version : ''} · ${d.rooms} 个房间`
      : `未连接：${d?.error || '未知错误'}`
    toast(d?.ok ? '连接正常' : '连接失败', d?.ok ? '' : 'err')
  } catch (e) {
    toast('连接失败：' + e.message, 'err')
  } finally {
    recorderTesting.value = false
  }
}

// ---- ① 服务器文件（浏览逻辑在 ServerDirPicker，与「表情包 → 添加」共用一份）----
//
// **两种选法并存**（用户口径：输入选择 + 弹出浏览，是增加不是替代）：
//   · 内联 ServerDirPicker = 可直接填/粘贴服务器绝对路径 + 就地列目录，写的是**已选**状态。
//   · 弹出层 AppModal    = 深目录导航视野更宽；它用**草稿**状态，点「选定」才写回已选。
// ⚠️ 弹出层必须是草稿：直接改已选的话，"进去逛一圈又关掉"会把**中途路过的目录**当成选定，
//    用户随后点「新建项目」建的就不是他最后确认的那个。
const localDir = ref('')
const localInput = ref('')
const creating = ref(false)
/** 已选目录的列表快照（由 picker 的 update:listing 给）：判"这个目录有没有可用视频" */
const localListing = ref({ dir: '', files: [], dirs: [] })

const pickerOpen = ref(false)
const draftDir = ref('')
const draftInput = ref('')
const draftListing = ref({ dir: '', files: [], dirs: [] })

/** 目录里第一个可用视频 —— 与后端 `find_first_video` 同口径（input 为空时后端就取它） */
const firstVideoIn = (l) =>
  (l.files || []).find((f) => VIDEO_EXTS.includes(String(f).split('.').pop().toLowerCase())) || ''

const localFirstVideo = computed(() => firstVideoIn(localListing.value))

/**
 * ⚠️ 判据只认**用户手动点的那个视频**，不再 `|| localFirstVideo`。
 *
 * 因为"目录里第一个视频"在**已有工程**的目录里是个陷阱：那样会把「添加已有工程」
 * 悄悄变成「拿第一个视频重建」。用户没点文件时，意图就是"添加这个工程"。
 */
const localManualVideo = computed(() => isVideoName(localInput.value))
/** 选中的是不是那个工程文件本身（点它 = 添加这个工程） */
const localPickedConfig = computed(() => String(localInput.value || '').toLowerCase() === PROJECT_FILE)
/** 这个目录是不是**已有工程**（列表里有 `yekitrim.toml`）*/
const localHasConfig = computed(() =>
  (localListing.value.files || []).some((f) => String(f).toLowerCase() === PROJECT_FILE)
)
/**
 * 按钮语义：
 *  · 已有工程 + 没手动挑视频 ⇒ **添加项目**（后端只登记/打开，不覆盖）
 *  · 其余（待初始化目录，或已有工程里又挑了视频 ⇒ 重建）⇒ **新建项目**
 */
const localAddMode = computed(() => localHasConfig.value && !localManualVideo.value)
/** 展示用的"实际会用的视频"：只认手动选的；没有就显示自动兜底的那个（待初始化目录才有意义） */
const localEffectiveVideo = computed(() =>
  localManualVideo.value ? localInput.value : localHasConfig.value ? '' : localFirstVideo.value
)
/**
 * 能提交 = 有目录 + （已有工程 ⇒ 不用视频 ｜ 否则 ⇒ 必须有可用视频）。
 * ⚠️ 老实现是"一律必须有视频"，于是**只有 yekitrim.toml、没有视频的工程目录永远提交不了** ——
 * 而那种目录恰恰是"直接添加"最该走通的形态。
 */
const canCreateLocal = computed(() =>
  !!localDir.value && (localHasConfig.value || !!localEffectiveVideo.value)
)

const draftFirstVideo = computed(() => firstVideoIn(draftListing.value))
const draftManualVideo = computed(() => isVideoName(draftInput.value))
const draftEffectiveVideo = computed(() => (draftManualVideo.value ? draftInput.value : draftFirstVideo.value))
const draftCanConfirm = computed(() => !!draftDir.value && !!draftEffectiveVideo.value)

function openLocalPicker() {
  draftDir.value = localDir.value
  draftInput.value = ''
  draftListing.value = { dir: localDir.value, files: [], dirs: [] }
  pickerOpen.value = true
}

function cancelLocalPick() {
  pickerOpen.value = false
}

/** 选定：草稿写回已选。⚠️ listing 必须一起带过来 —— 页面判"有无可用视频"靠它。 */
function confirmLocalPick() {
  localDir.value = draftDir.value
  localInput.value = draftInput.value
  localListing.value = { ...draftListing.value, dir: draftDir.value }
  pickerOpen.value = false
}

/**
 * 换了浏览目录就把已点的文件清掉。
 * ⚠️ 不清会提交"新目录 + 旧文件名"：后端 `jail_existing(新目录, 旧名)` 要么报一句
 * 莫名其妙的「视频不可用」，要么**在新目录里撞上同名文件而静默绑错**。
 * （`PrepareView.vue::onPickerDir` 是同一条规矩，两处别只改一处。）
 */
function onLocalDir(dir) {
  localDir.value = dir
  localInput.value = ''
  localListing.value = { dir, files: [], dirs: [] }
}

/** 工程标志文件（`yekitrim.toml`）—— 点它 = "添加这个已有工程" */
function isProjectName(n) {
  return String(n || '').toLowerCase() === PROJECT_FILE.toLowerCase()
}
/** 只有视频与工程文件可点；其余（弹幕/字幕/图片…）在卡片上置灰 */
function isSelectableFile(n) {
  return isVideoName(n) || isProjectName(n)
}
function fileIconOf(n) {
  if (isVideoName(n)) return 'film'
  return isProjectName(n) ? 'settings' : 'file'
}
function fileTagOf(n) {
  if (isVideoName(n)) return '视频'
  return isProjectName(n) ? '工程' : '不可选'
}

function pickLocalFile(name) {
  if (!isSelectableFile(name)) return
  localInput.value = localInput.value === name ? '' : name
}

/** 弹出层里换目录同理（只是改草稿，不动已选） */
function onDraftDir(dir) {
  draftDir.value = dir
  draftInput.value = ''
  draftListing.value = { dir, files: [], dirs: [] }
}

function pickDraftFile(name) {
  draftInput.value = draftInput.value === name ? '' : name
}

async function createFromLocal() {
  // 选中的文件若已不在当前目录（列表刷新/换目录），回退成"不指定"——直接提交旧名字只会换来 400
  const stillThere = (localListing.value.files || []).includes(localInput.value) ? localInput.value : ''
  // 点中的是工程文件本身 ⇒ 它的含义是"添加这个工程"，不是"拿它当输入视频"，
  // 所以**不把它当 input 交上去**（后端在已有工程分支里也根本不看 input）
  const picked = localPickedConfig.value ? '' : stillThere
  const manualVideo = isVideoName(picked)

  // ⚠️ 二次确认：目录里已有工程、用户又手动挑了一个视频 ——
  // 这是唯一会**覆盖**已有工程配置的路径（"新建"会重写 yekitrim.toml）。
  // 没有这道确认，用户手改过的工程设置会被一次误点冲掉。
  let overwrite = false
  if (localHasConfig.value && manualVideo) {
    const yes = await confirmDialog(
      `这个目录里已经有一个工程：${localDir.value}

` +
      `继续会用它「重新初始化」：现有的 yekitrim.toml 会被重写，输入视频换成你选的「${picked}」。

` +
      `只想添加这个已有工程的话，点「取消」，然后取消选中的那个视频文件。`,
      { title: '这个目录已有工程', danger: true }
    )
    if (!yes) return
    overwrite = true
  }

  creating.value = true
  try {
    const r = await createLocalProject({ dir: localDir.value, input: picked, overwrite })
    toast(overwrite ? '项目已重新初始化' : localAddMode.value ? '已添加工程' : '项目已创建')
    // 建成之后清空选择：否则留在原地的旧目录+旧文件会被误当成"下一次要建的"
    localInput.value = ''
    await loadProjects()
    if (r?.id) window.location.hash = `#/p/${r.id}/prepare`
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    creating.value = false
  }
}

// ---- ③ B站链接（P4）----
//
// 交互形状：粘贴 → 解析（薄壳，不落库）→ 勾选分P → 提交（每 P 一个会话/项目）。
// 下载与建项目都在服务端 worker 里跑，这里只负责"提交"，进度去采集记录页看。
const dl = reactive({
  /** null = 还没问到（别先闪一下"未就绪"）；true/false = 下载器可用位 */
  ready: null,
  verified: false,
  kind: '',
  pinned_version: '',
  license: ''
})
const dlBusy = ref(false)
const dlError = ref('')

const biliUrl = ref('')
const biliBusy = ref(false)
const biliError = ref('')
const biliHint = ref('')
const biliInfo = ref(null)
/// 选中的分P。**默认全不选**：23P 全下会直接撑爆配额，必须让用户主动选。
const pickedPages = ref([])
const biliSubmitting = ref(false)
const biliSubmitted = ref(null)

/**
 * 已提交的下载任务（每 P 一条），用于**进度显示**。
 *
 * ⚠️ 后端**没有字节级下载进度**：`/api/ingest/records` 的 `bytes` 只统计**已完成**文件
 * （`closed_files`），下载中的 `.part` 不在表里。所以这里只显示**状态**（排队/下载/入库/完成/失败）
 * + 不确定进度条，**不编造百分比**。想要真实百分比得让服务端把 `.part` 的当前大小也报上来。
 */
const biliJobs = ref([])
let biliPollTimer = null

function biliJobPct(j) {
  if (j.state === 'imported' || j.project_id) return 100
  if (j.state === 'failed') return 100
  return 0           // 0 = 走不确定动画条，别假装知道进度
}
function biliJobChip(j) {
  if (j.state === 'failed') return 'r'
  if (j.state === 'imported' || j.project_id) return 'g'
  return 'b'
}
/** 某个 session 连续这么多次不在列表里就放弃它（F2）：`/api/ingest/records?limit=50` 只回最新
 *  50 条，历史一多 / 会话被清理后旧 session 永远查不到 ⇒ `pending` 永不归零 ⇒ 轮询直到离开
 *  页面都不停（还每次往运行日志写一条 debug）。 */
const BILI_MISS_LIMIT = 8
const biliMissCount = new Map()
/** 轮询兜底上限：就算一直有 pending 也别无限轮询（25 分钟）。 */
const BILI_POLL_MAX_MS = 25 * 60 * 1000
let biliPollStartedAt = 0

/** 一轮轮询：按 session_id 把状态刷进 `biliJobs`；全部到终态就停。 */
async function pollBiliJobs() {
  if (!biliJobs.value.length) { stopBiliPoll(); return }
  if (biliPollStartedAt && Date.now() - biliPollStartedAt > BILI_POLL_MAX_MS) {
    stopBiliPoll()
    toast('下载进度查询超时，请到「采集」页看最新状态', 'warn')
    return
  }
  try {
    const d = await api('/api/ingest/records?limit=50', { label: '查询下载进度' })
    const byId = new Map((d?.records || []).map((r) => [String(r.session_id), r]))
    let pending = 0
    for (const j of biliJobs.value) {
      const r = byId.get(String(j.session_id))
      if (!r) {
        // 查不到：计 miss；连续 N 次仍查不到就按「查无此会话」放弃这一条（不再无限 pending）
        const miss = (biliMissCount.get(j.session_id) || 0) + 1
        biliMissCount.set(j.session_id, miss)
        if (miss >= BILI_MISS_LIMIT) {
          j.state = 'failed'
          j.state_label = '状态未知（会话已不在列表里）'
          j.error = j.error || '查询不到这条下载的进度（可能已被清理），请到「采集」页确认'
        } else {
          pending += 1
        }
        continue
      }
      biliMissCount.delete(j.session_id)
      j.state = r.state || ''
      j.state_label = r.state_label || ''
      j.bytes = r.bytes || 0
      j.project_id = r.project_id || null
      j.error = r.error || ''
      if (!(r.state === 'imported' || r.project_id || r.state === 'failed')) pending += 1
    }
    if (!pending) {
      stopBiliPoll()
      if (biliJobs.value.some((j) => j.state === 'imported' || j.project_id)) {
        toast('下载完成，已自动入库')
        await loadProjects()
      }
    }
  } catch { /* 轮询失败不打断用户，下一轮再试 */ }
}
function startBiliPoll() {
  stopBiliPoll()
  biliMissCount.clear()
  biliPollStartedAt = Date.now()
  biliPollTimer = setInterval(pollBiliJobs, 2500)
  pollBiliJobs()
}
function stopBiliPoll() {
  if (biliPollTimer) { clearInterval(biliPollTimer); biliPollTimer = null }
}
/// 存放位置是否已显式配置（`[storage] projects_root`）。null = 还没问到 / 查询失败
/// （此时**不拦**，让后端兜底——前端查不到就别替后端做决定）。
/// 只有管理员会查（config 是 admin-only 端点），普通用户提交时由后端按角色给文案。
const rootConfigured = ref(null)
/**
 * **项目存放位置的"实际生效值"**（后端 `/api/admin/ingest` 的 `projects_root_effective`）。
 * ⚠️ `projects_root` 留空**不等于不能下载** —— 服务端会回退到 `<程序目录>/.yekitrim/projects`
 * （`yekitrim_service::projects::projects_root`）。之前前端把"没显式配置"当成"不可用"，
 * 于是默认位置明明可用却拦着不让下（用户报的就是这个）。现在只提示、不拦。
 */
const projectsRootEffective = ref('')

/// 只统计选中的 P（预览卡上的"预计占用"要跟着勾选实时变）。
const pickedEstimate = computed(() => {
  const info = biliInfo.value
  if (!info) return 0
  return info.pages
    .filter((p) => pickedPages.value.includes(p.index))
    .reduce((sum, p) => sum + (p.estimate_bytes || 0), 0)
})

async function loadDownloader() {
  try {
    const d = await ingestApi.downloaderStatus()
    Object.assign(dl, {
      ready: d?.ready === true,
      verified: d?.verified === true,
      kind: d?.kind || '',
      pinned_version: d?.pinned_version || '',
      license: d?.license || ''
    })
  } catch {
    // 读不到就当作"未知"，不拦用户（提交时服务端还会拦一次）
    dl.ready = null
  }
}

async function installDownloader() {
  dlBusy.value = true
  dlError.value = ''
  try {
    const r = await ingestApi.installDownloader()
    await loadDownloader()
    toast(r?.verified ? '下载器已安装并通过校验' : '下载器已安装（未通过 sha256 校验，请检查）', r?.verified ? 'ok' : 'err')
  } catch (e) {
    dlError.value = e?.message || '安装失败'
    toast(dlError.value, 'err')
  } finally {
    dlBusy.value = false
  }
}

async function resolveBili() {
  biliBusy.value = true
  biliError.value = ''
  biliHint.value = ''
  biliSubmitted.value = null
  try {
    const d = await ingestApi.resolveBilibili(biliUrl.value.trim())
    biliInfo.value = d
    // 单P 默认勾上（只有一个选项时让用户再点一次是多余的）；
    // 多P 一律留空 —— 由用户自己决定下哪几 P
    pickedPages.value = d.pages.length === 1 ? [d.pages[0].index] : []
    if (d.pages.length > 1) {
      biliHint.value = `这是个 ${d.pages.length} 分P 的合集，选好要下载的 P（每个 P 会建成一个项目）。`
    }
  } catch (e) {
    biliInfo.value = null
    pickedPages.value = []
    biliError.value = e?.message || '解析失败'
  } finally {
    biliBusy.value = false
  }
}

/** 「换个链接」：回到未解析态（不重新请求，只清本地结果） */
function resetBili() {
  biliInfo.value = null
  pickedPages.value = []
  biliError.value = ''
  biliHint.value = ''
  biliSubmitted.value = null
}

function pickAllPages() {
  pickedPages.value = (biliInfo.value?.pages || []).map((p) => p.index)
}

async function submitBili() {
  biliSubmitting.value = true
  biliError.value = ''
  try {
    const r = await ingestApi.submitBilibili(biliUrl.value.trim(), pickedPages.value)
    biliSubmitted.value = r
    const n = r?.accepted?.length || 0
    // 把这次提交的每条登记成"待跟踪任务"，随后轮询状态
    biliJobs.value = (r?.accepted || []).map((a) => ({
      session_id: a.session_id, page: a.page, title: a.title,
      state: 'queued', state_label: '排队中', bytes: 0, project_id: null, error: ''
    }))
    if (biliJobs.value.length) startBiliPoll()
    toast(n ? `已提交 ${n} 个下载，完成后自动建项目` : '没有新的下载（都已存在项目）')
    // 提交后会话就在采集记录页里，提示用户去哪儿看进度
    if (n) await loadRecords().catch(() => {})
  } catch (e) {
    biliError.value = e?.message || '提交失败'
    toast(biliError.value, 'err')
  } finally {
    biliSubmitting.value = false
  }
}

// ---- ② 录播姬录制 ----
const scanning = ref(false)
const ingestReady = computed(() => ingestState.ready)

async function scanNow() {
  scanning.value = true
  try {
    const r = await ingestApi.scanNow()
    toast(`新登记 ${r?.registered ?? 0} 个完整录像`)
  } catch (e) {
    toast(e.message, 'err')
  } finally {
    scanning.value = false
  }
}

/**
 * 页头「刷新」= 项目 + 运行状况 + 环境检查一起重来（用户点它就是"有什么不对"）。
 * ⚠️ 必须防连点：慢盘上一次要几秒，不置忙态时用户会连点三下 ⇒ 三套并发请求
 * （`loadProjects` 有单飞，`loadDoctor`/`loadHealth` 没有）。
 */
const refreshing = ref(false)
async function refresh() {
  if (refreshing.value) return
  refreshing.value = true
  try {
    await loadProjects()
    await Promise.allSettled([loadDoctor(), loadHealth(true)])
  } finally {
    refreshing.value = false
  }
}

/**
 * 运行状况（doctor）：`GET /api/doctor` 的 `{ffmpeg, ffprobe, models_dir, models}` +
 * `GET /api/models` 的富列表（`size_label` / `note` / `source_label`，ModelList 要靠它渲染）。
 *
 * ⚠️ 两个请求**一起发、一起刷**：doctor 里的 `models[].installed` 是"装没装"的权威判据，
 * 而 ModelList 要的是富列表 —— 只刷一个会出现"点了下载、按钮没变"的错位。
 * ⚠️ 读失败**照实报**（`doctorError`），不要静默留空 —— 这整块的存在意义就是"告诉你缺什么"。
 */
const doctor = ref(null)
const models = ref([])
const doctorError = ref('')

async function loadDoctor() {
  try {
    const [d, list] = await Promise.all([
      api('/api/doctor', { label: '读取运行状况' }),
      api('/api/models', { label: '读取模型列表' })
    ])
    doctor.value = d || null
    models.value = list || []
    doctorError.value = ''
  } catch (e) {
    doctorError.value = '运行状况读取失败：' + (e?.message || '')
  }
}

/**
 * 运行状况待处理项（页头短提示与卡片 title 共用；空 = 一切就绪）。
 * 判据 = 外部命令缺失 · 读取失败 · **有模型没装**。
 * ⚠️ 还没读到时**不算问题**（`doctor` 为 null）—— 否则首帧会先冒一条再消失。
 */
const doctorIssues = computed(() => {
  const out = []
  if (doctorError.value) out.push('运行状况读取失败')
  const d = doctor.value
  if (d) {
    if (!d.ffmpeg) out.push('缺 ffmpeg')
    if (!d.ffprobe) out.push('缺 ffprobe')
  }
  // ⚠️ 出片底座在 `/api/health.dhampir_ready`（不在 doctor 里）：
  // 只查 ffmpeg/ffprobe 会让页头「N 项待处理」漏报，而底座缺失时出片是直接失败的。
  const h = health.value
  if (h && !h.dhampir_ready) out.push('缺出片底座')
  const missing = models.value.filter((m) => !m.installed)
  if (missing.length) out.push('模型未下载：' + missing.map((m) => m.name).join('、'))
  return out
})
const doctorProblem = computed(() => doctorIssues.value.length > 0)
/** 卡片**默认折叠**成一行状态条；页头那枚提示点开时再展开并滚过去。 */
const doctorOpen = ref(false)
const doctorCard = ref(null)
function openDoctor() {
  // 单页布局：直接展开并滚过去
  doctorOpen.value = true
  requestAnimationFrame(() => doctorCard.value?.scrollIntoView({ block: 'nearest', behavior: 'smooth' }))
}

/** 统计条读数：片段/出片只能从 per-pid 缓存算，没加载过就显示 `—`（别报 0 冒充"没有片段"）。 */
const totals = computed(() => {
  let clips = 0
  let exported = 0
  let bytes = 0
  let known = 0
  for (const p of wb.projects || []) {
    bytes += Number(p.bytes) || 0
    const c = cacheOf(p.id)
    if (!c.status) continue
    known += 1
    const list = c.clips || []
    clips += list.length
    for (const x of list) {
      const m = c.polishMap?.[x.dir]
      if (m?.rendered || m?.cover) exported += 1
    }
  }
  return { clips, exported, bytes, known }
})

/**
 * 首屏装载。
 * ⚠️ `?add=1`（预处理页 404 的「重新导入」带回来的）：直接选中一个来源并滚过去，
 *    然后把参数清掉（否则刷新/后退还会再触发一次）。
 * ⚠️ 默认来源要等 `loadRecObjects` / `loadDownloader` 的可用位回来才定得准，所以放在最后。
 */
onUnmounted(stopBiliPoll)

onMounted(async () => {
  if (String(route.query.add || '') === '1') {
    openAdd()
    const q = { ...route.query }
    delete q.add
    router.replace({ path: route.path, query: q, hash: route.hash }).catch(() => {})
  }
  await loadProjects()
  // doctor + health 一起等：未就绪时那条状态卡的首帧应当是"正在检查"，别先报缺项再消失
  await Promise.allSettled([loadDoctor(), loadHealth()])
  checking.value = false
  await loadRecObjects()
  await loadDownloader()
  if (isAdmin.value) {
    try {
      const d = await ingestApi.config()
      rootConfigured.value = !!(d?.storage?.projects_root || '').trim()
      projectsRootEffective.value = d?.storage?.projects_root_effective || d?.projects_root_effective || ''
      webhookReady.value = d?.webhook?.enabled === true || d?.webhook_enabled === true
    } catch {}
  }
  // 右栏不能是空的：可用位回来之后再定默认来源
  addSource.value = defaultSource()
})

// 从设置页回来要重查（组件被 keep-alive / 复用时 onMounted 不会重跑）。
// 看 name 而不是 fullPath：清 `?add=1` 只改 query，不该触发整轮检查。
watch(() => route.name, (to, from) => {
  if (to !== 'workbench' || to === from) return
  // 设置页可能刚改过模型 / 下载器 / LLM：回首页时静默重查 doctor + health（不置 loading）
  loadDoctor()
  loadHealth(true)
  if (isAdmin.value) {
    ingestApi.config()
      .then((d) => {
        rootConfigured.value = !!(d?.storage?.projects_root || '').trim()
        projectsRootEffective.value = d?.storage?.projects_root_effective || d?.projects_root_effective || ''
      })
      .catch(() => {})
  }
})
</script>

<style scoped>
/* ── 来源面板内部：**左选 / 右果**（与外层「左选来源 / 右填参数」同一套逻辑） ──
   `--pane-left` / `--pane-right` 由各面板覆写：服务器文件 1fr|300、录播接入 260|1fr、
   B站 1fr|320。≤1100px 折单列（阅读顺序仍是左→右）。 */
.add-pane { display:grid;
  grid-template-columns:var(--pane-left, minmax(0,1fr)) var(--pane-right, 300px);
  gap:var(--s-4); align-items:stretch;
  height:100%; min-height:0; }
.add-pane > .add-pane-wide { grid-column:1 / -1; }
/* 列内小标题：**紧贴内容**的一行，不再单独占一整行（原来那行表头把有限的高度又吃掉一块） */
.pane-col-head { display:flex; align-items:baseline; justify-content:space-between; gap:8px;
  min-width:0; margin-bottom:2px; font-size:var(--fs-xs); color:var(--ink-faint); }
.pane-col-head .val { color:var(--ink-dim); font-weight:var(--fw-medium);
  min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.add-pane-left, .add-pane-right { display:flex; flex-direction:column; gap:6px; min-width:0; min-height:0; }
.add-pane-right .btn { width:100%; justify-content:center; }
.add-pane-right .create-bar { display:block; }
/* 服务器文件：目录导航天然窄（260），文件卡片需要宽度铺成多列 —— 与录播同一套「左窄右宽」 */
.add-col-local { --pane-left:260px; --pane-right:minmax(0,1fr); }
/* 录播接入：房间列表天然窄，录像行需要宽度放「名称 · 大小 · 添加」 */
.add-col-recorder { --pane-left:260px; --pane-right:minmax(0,1fr); }
/* B站：右侧分P 列表要固定宽度，左侧留给输入与预览 */
.add-col-bilibili { --pane-right:320px; }
@media (max-width:1100px) {
  .add-pane, .add-col-recorder, .add-col-bilibili {
    grid-template-columns:minmax(0,1fr); gap:var(--s-3);
  }
}
/* 首页根：flex 列只为让各块保持顺序稳定 */
.wb-root { display:flex; flex-direction:column; }
/* 页头：引导语 + 右侧动作；标题已上移到吸顶栏 */
.wb-head { align-items:center; margin-bottom:10px; }
.wb-lead { margin:0; font-size:13px; color:var(--ink-dim); }
/* 概览 + 继续上次：同一行（左动作 / 右读数 / 右端主按钮）。
   ⚠️ 别再拆回两张卡：用户口径"合并为同一行"（2026-11-13）。 */
.wb-top { display:flex; align-items:center; gap:var(--s-5); flex-wrap:wrap; }
.wb-top-continue {
  display:flex; align-items:center; gap:12px; min-width:0; flex:1 1 300px;
  text-decoration:none; color:inherit; transition:transform .15s;
}
.wb-top-continue:hover { transform:translateY(-1px); }
.wb-top-continue:hover .cc-title { color:var(--pink-text); }
.wb-top-stats { display:flex; align-items:center; gap:var(--s-5); margin-left:auto; flex:none; }
/* 没有可继续的项目时（新装/项目全删）：别在左边留半张空卡 —— 读数直接铺满整行 */
.wb-top--empty .wb-top-stats { margin-left:0; width:100%; justify-content:space-between; }
@media (max-width:860px) {
  .wb-top-stats { margin-left:0; width:100%; gap:var(--s-3); justify-content:space-between; }
}
.cc-icon {
  width:32px; height:32px; flex:none; border-radius:var(--r-full);
  display:flex; align-items:center; justify-content:center;
  background:var(--main-soft); color:var(--pink-text);
}
.cc-body { flex:1 1 auto; min-width:0; display:flex; flex-direction:column; gap:2px; }
/* 项目名是主信息（--ink），"下一步"才是辅助（muted）。原来整条标题都用 --main-deep，
   于是"继续哪个项目"和"下一步做什么"看起来一样重。 */
.cc-title {
  display:block; font-size:14px; font-weight:650; color:var(--ink);
  overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
}
.cc-go { flex:none; }
/* 添加录像：头部按钮 + 折叠向导 */
.add-head { display:flex; align-items:center; gap:10px; flex-wrap:wrap; }
.wiz-body { margin-top:2px; }
/* 不可用来源**仍可点**（点开看原因/设置入口）；全局 `.wiz-src.off` 是 not-allowed，这里盖掉 */
.wiz-src.off { cursor:pointer; }
/* 房间列表：与外层「来源导航」同一套观感（圆角 10、悬停 surface-2、选中 main-soft + 描边），
   原来是自己一套「黑底 + 右对齐数字」，和全站其它列表对不上。 */
.room-list { display:flex; flex-direction:column; gap:2px; min-height:0; overflow-y:auto; }
.room-item { display:flex; align-items:center; justify-content:space-between; gap:8px;
  width:100%; padding:7px 10px; text-align:left; cursor:pointer;
  border:1px solid transparent; border-radius:var(--r-sm); background:none; color:var(--ink-dim);
  transition:background-color var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease); }
.room-item:hover { background:var(--surface-2); color:var(--ink); }
.room-item.on { background:var(--main-soft); border-color:var(--main-glow); color:var(--ink); }
.room-name { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  font-size:var(--fs-base); font-weight:var(--fw-medium); }
.room-item.on .room-name { color:var(--pink-text); font-weight:var(--fw-semi); }
.room-count { flex:none; min-width:20px; padding:0 6px; border-radius:var(--r-full); text-align:center;
  background:var(--surface-2); color:var(--ink-faint); font-size:var(--fs-xs); line-height:16px;
  font-variant-numeric:tabular-nums; }
.room-item.on .room-count { background:var(--card); color:var(--pink-text); }

/* ── 服务器文件：文件卡片（与录像卡片同一套观感） ── */
.file-cards { display:grid; grid-template-columns:repeat(auto-fill,minmax(240px,1fr));
  gap:var(--s-2); min-height:0; overflow-y:auto; flex:1 1 auto; align-content:start; }
.file-card { display:flex; align-items:center; gap:7px; min-width:0; padding:7px 9px;
  border:1px solid var(--line); border-radius:var(--r-sm); background:var(--card);
  color:var(--ink); text-align:left; cursor:pointer;
  transition:border-color var(--dur-fast) var(--ease), background-color var(--dur-fast) var(--ease); }
.file-card:hover { border-color:var(--main-glow); }
.file-card.on { border-color:var(--main); background:var(--main-soft); }
.file-card.on .file-card-name { color:var(--pink-text); font-weight:var(--fw-semi); }
.file-card.dim { opacity:.55; cursor:not-allowed; }
.file-card-ic { flex:none; display:flex; color:var(--ink-faint); }
.file-card.on .file-card-ic { color:var(--pink-text); }
.file-card.project .file-card-ic { color:var(--green-deep); }
.file-card-name { flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  font-family:var(--mono); font-size:var(--fs-xs); }
.file-card-tag { flex:none; font-size:10px; color:var(--ink-faint); }
/* ── 服务器文件 = 文件管理器：常用位置 / 浏览器 / 操作区 三段纵向 ── */
.local-manager { display:grid; grid-template-rows:auto minmax(0,1fr) auto;
  gap:var(--s-2); height:100%; min-height:0; }
/* 常用位置条：钉固的目录 chip + 「★ 钉固当前」 */
.local-pins { display:flex; align-items:center; gap:6px; flex-wrap:wrap; }
/* chip = 容器（里面是「名称按钮 + ×」两个按钮，不能嵌套 button） */
.pin-chip { display:inline-flex; align-items:center; max-width:220px;
  border:1px solid var(--line); border-radius:var(--r-full); background:var(--card);
  color:var(--ink-dim); font-size:var(--fs-xs); white-space:nowrap; overflow:hidden;
  transition:color var(--dur-fast) var(--ease), border-color var(--dur-fast) var(--ease),
             background-color var(--dur-fast) var(--ease); }
.pin-chip:hover { border-color:var(--main); }
.pin-main { display:inline-flex; align-items:center; gap:4px; min-width:0; padding:3px 4px 3px 10px;
  border:none; background:none; color:inherit; font-size:inherit; cursor:pointer;
  overflow:hidden; text-overflow:ellipsis; }
.pin-chip:not(:has(.pin-x)) .pin-main { padding-right:10px; }
.pin-main:hover { color:var(--pink-text); }
/* 预设位置：带小房子图标 + 锁定的视觉（没有 ×，鼠标手势也不一样） */
.pin-chip.preset { border-style:dashed; }
.pin-chip.preset .pin-ic { flex:none; margin-left:8px; color:var(--ink-faint); }
.pin-chip.preset:hover { border-color:var(--line); }
.pin-chip.on { background:var(--main-soft); border-color:var(--main); color:var(--pink-text);
  font-weight:var(--fw-semi); }
/* 移除钉固：显式的小 ×，不再只靠右键 */
.pin-x { display:inline-flex; align-items:center; justify-content:center; flex:none;
  width:16px; height:16px; margin:0 4px 0 0; border-radius:50%;
  border:none; background:none; color:var(--ink-faint); cursor:pointer; }
.pin-x:hover { background:var(--red-soft); color:var(--red-deep); }
.pin-add { flex:none; }
/* 浏览器撑满中间那段 */
.local-browser-body { min-height:0; display:flex; }
.local-browser-body :deep(.sfb) { flex:1 1 auto; min-height:0; }
.local-browser-body :deep(.sfb-body) { flex:1 1 auto; min-height:0; }
.local-browser-body :deep(.sfb-tree),
.local-browser-body :deep(.sfb-files) { min-height:0; }
.local-browser-body :deep(.sfb-list) { overflow:auto; }
/* 操作区固定在底部 */
.local-ops { display:flex; flex-direction:column; gap:6px; padding-top:var(--s-2);
  border-top:1px dashed var(--line-soft); }
/* 状态行：一句话说清"会不会重建 / 用哪个视频"，不再堆大段说明 */
.local-state { display:flex; flex-direction:column; gap:2px; font-size:var(--fs-xs); color:var(--ink-dim); }
.local-state .ok-text { color:var(--green-deep); }
.local-state .warn-text { color:var(--amber-deep); }
/* 操作区：左边"建在哪"，右边主按钮 */
.local-actions { display:flex; align-items:center; gap:var(--s-2); }
.local-where { flex:1 1 auto; min-width:0; font-size:var(--fs-xs); color:var(--ink-faint); }
.local-open { margin-top:auto; width:100%; justify-content:center; }

/* 录像卡片：一行两张，名称两行截断，底部一行「大小 · 时间」+ 添加。
   ⚠️ 别再退回表格 —— 一行一条在 16 条以上时要滚很久才找到目标（用户截图反馈）。 */
.rec-cards { display:grid; grid-template-columns:repeat(auto-fill,minmax(210px,1fr));
  gap:var(--s-2); min-height:0; overflow-y:auto; flex:1 1 auto; align-content:start; }
.rec-card { display:flex; flex-direction:column; gap:6px; min-width:0;
  padding:9px 10px; border:1px solid var(--line); border-radius:var(--r-sm);
  background:var(--card); transition:border-color var(--dur-fast) var(--ease); }
.rec-card:hover { border-color:var(--main-glow); }
.rec-card.added { background:var(--surface-2); }
.rec-card-name { min-width:0; display:-webkit-box; -webkit-line-clamp:2; -webkit-box-orient:vertical;
  overflow:hidden; font-family:var(--mono); font-size:var(--fs-xs); line-height:1.5;
  color:var(--ink); word-break:break-all; }
.rec-card.added .rec-card-name { color:var(--ink-faint); }
.rec-card-foot { display:flex; align-items:center; gap:6px; margin-top:auto; }
.rec-card-meta { flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  font-size:var(--fs-xs); color:var(--ink-faint); font-variant-numeric:tabular-nums; }
.rec-card .mini-btn { flex:none; }
.rec-none { padding:10px 2px; }

/* 录播姬不可用：提示块（因不可用而整块不渲染时，这一块是唯一入口） */
.room-down { display:flex; align-items:center; gap:10px; padding:9px 12px; flex-wrap:wrap;
  border:1px dashed var(--line-soft); border-radius:8px; background:var(--bg-soft); font-size:13px; }
/* 连不上/未配置：**上下两行**（上=原因，下=动作）。用户口径 2026-11-14。
   ⚠️ 别再收回一行：原因文本很长（带 URL 与 errno），挤在同一行会把按钮顶到看不见的位置。 */
.rec-fail { flex-direction:column; align-items:stretch; gap:8px;
  /* ⚠️ 必须 align-self:start：父级 `.add-pane` 是 `align-items:stretch` 的 grid，
     不写的话这一块会被拉满整列高度 —— 一小段报错撑成一大片红底色（实测）。 */
  align-self:start; }
.rec-fail-msg { display:flex; align-items:flex-start; gap:7px; min-width:0; }
.rec-fail-msg > svg { flex:none; margin-top:2px; }
.rec-fail-text { display:flex; flex-direction:column; gap:2px; min-width:0; line-height:1.5;
  overflow-wrap:anywhere; }
.rec-fail-acts { display:flex; gap:6px; flex-wrap:wrap; }
/* 运行状况：折叠成一行状态条；展开才显示模型清单（默认不占首屏） */
.doctor-card { margin-top:4px; }
.doctor-head { display:flex; align-items:center; gap:8px; width:100%; padding:0;
  background:none; border:none; text-align:left; cursor:pointer; color:inherit; }
.doctor-head:hover .t { color:var(--main-deep); }
.room-list { margin-top:10px; }
.room-item { border-bottom:1px dashed var(--line-soft); }
.room-item:last-child { border-bottom:none; }
.room-row { display:flex; align-items:center; gap:10px; padding:6px 0; }
.room-toggle { flex:none; width:22px; padding:2px 0; text-align:center; }
.room-id { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:12.5px; }
.room-title { flex:1; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.room-rec { flex:none; white-space:nowrap; }
/* 展开区：贴在房间行下面，左侧缩进对齐房间号 */
.room-files { margin:2px 0 8px 32px; padding-left:10px; border-left:2px solid var(--line-soft); }
.file-list-head { padding-bottom:2px; }
.rec-name { flex:1; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.rec-meta { flex:none; white-space:nowrap; }

/* ── B站链接向导 ── */
/* 下载器未就绪：整块给"怎么解决"，但不给普通用户一个点不动的安装按钮 */
.bili-gate {
  padding:10px 12px; border:1px solid var(--line-soft); border-radius:8px;
  background:var(--bg-soft, transparent);
}
.bili-preview { display:flex; gap:12px; margin-top:12px; align-items:flex-start; }
.bili-cover {
  width:160px; flex:none; border-radius:6px; border:1px solid var(--line-soft);
  aspect-ratio:16/9; object-fit:cover; background:var(--bg-soft, transparent);
}
.bili-meta { min-width:0; }
.bili-title {
  display:block; font-size:14px; line-height:1.4;
  overflow:hidden; text-overflow:ellipsis; display:-webkit-box;
  -webkit-line-clamp:2; -webkit-box-orient:vertical;
}
/* ── B站：**上下结构**（上=输入+解析，下=信息+分P+下载） ── */
.bili-pane { display:flex; flex-direction:column; gap:var(--s-3); height:100%; min-height:0; }
.bili-input { display:flex; flex-direction:column; gap:6px; flex:none; }
.bili-input .row { gap:8px; }
.bili-input-hint { gap:8px; }
.bili-result { display:flex; flex-direction:column; gap:var(--s-2); flex:1 1 auto; min-height:0; }
/* 视频信息（防下错）：封面 + 标题 + 元信息一行 */
.bili-info { display:flex; align-items:flex-start; gap:12px; padding:10px 12px; flex:none;
  border:1px solid var(--line); border-radius:var(--r-sm); background:var(--card); }
.bili-meta { display:flex; flex-direction:column; gap:4px; min-width:0; flex:1 1 auto; }
.bili-pick-tools { flex:none; gap:6px; align-self:flex-start; }

/* ── 分P 卡片（与录像卡片同一套观感） ── */
.p-tools { margin-top:2px; }
.p-cards { display:grid; grid-template-columns:repeat(auto-fill,minmax(150px,1fr));
  gap:var(--s-2); min-height:0; overflow-y:auto; flex:1 1 auto; align-content:start; }
.p-card { display:flex; flex-direction:column; gap:3px; min-width:0; padding:8px 10px;
  border:1px solid var(--line); border-radius:var(--r-sm); background:var(--card); cursor:pointer;
  transition:border-color var(--dur-fast) var(--ease), background-color var(--dur-fast) var(--ease); }
.p-card:hover { border-color:var(--main-glow); }
.p-card.on { border-color:var(--main); background:var(--main-soft); }
/* 勾选框放在标题行左侧，避免整卡一个 checkbox 把排版撑歪 */
.p-card input[type="checkbox"] { position:absolute; opacity:0; pointer-events:none; }
.p-card-head { display:flex; align-items:baseline; justify-content:space-between; gap:6px; }
.p-card-num { font-family:var(--mono); font-size:var(--fs-xs); font-weight:var(--fw-semi);
  color:var(--ink-dim); }
.p-card.on .p-card-num { color:var(--pink-text); }
.p-card-dur { font-size:10px; color:var(--ink-faint); font-variant-numeric:tabular-nums; }
.p-card-name { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  font-size:var(--fs-sm); color:var(--ink); }
.p-card-est { font-size:10px; color:var(--ink-faint); font-variant-numeric:tabular-nums; }
.bili-submit { display:flex; align-items:center; gap:var(--s-2); flex-wrap:wrap;
  margin-top:auto; padding-top:var(--s-2); border-top:1px dashed var(--line-soft); }
.bili-submit-sum { flex:1 1 auto; min-width:0; font-size:var(--fs-xs); color:var(--ink-dim); }
.bili-submit-sum b { color:var(--ink); }
/* ── 下载进度（每条 = 一 P） ── */
.bili-jobs { display:flex; flex-direction:column; gap:6px; flex:none;
  padding:8px 10px; border:1px solid var(--line-soft); border-radius:var(--r-sm); background:var(--surface-2); }
.bili-job { display:flex; flex-direction:column; gap:4px; }
.bili-job-row { display:flex; align-items:center; gap:8px; min-width:0; }
.bili-job-name { flex:1 1 auto; min-width:0; font-size:var(--fs-sm); color:var(--ink); }
.bili-job-meta { flex:none; white-space:nowrap; }
.bili-job .chip { flex:none; }
/* 不确定进度条：没有字节级进度时用它，**别编造百分比** */
.bili-job .progress > .indet { width:35%; animation:bili-indet 1.1s ease-in-out infinite; }
@keyframes bili-indet {
  0%   { margin-left:0; }
  50%  { margin-left:65%; }
  100% { margin-left:0; }
}
.bili-job .progress.done > div { background:var(--green); }
.bili-job .progress.failed > div { background:var(--red); }

.bili-done {
  margin-top:10px; padding:10px 12px; border:1px solid var(--line-soft);
  border-radius:8px; background:var(--bg-soft, transparent);
}

/* ── 添加录像：四列（宽屏 4 / 中屏 2 / 窄屏 1）；未配置的列灰底 ── */
/* 同一行三列：本地导入 / 录播接入 / 下载；窄屏自动堆叠 */
/* 三档：宽屏三列并排 → 中屏两列（第三列换行，而不是挤成竖条）→ 窄屏单列。
   ⚠️ 原来只有一个 1200px 断点直接塌成单列，1024–1200 这一段把三个来源全部竖排、
   首屏被撑得极长（真正的"添加录像"按钮滚到第二屏）。 */
/* ⚠️ `align-items:start` 而不是 `stretch`：三个来源的内容天生不等高，
   拉伸只会让最短的那列（录播接入）留出一大块空白底，看起来像没加载出来。 */
.add-grid { display:grid; grid-template-columns:repeat(3, minmax(0,1fr)); gap:12px; align-items:start; }
@media (max-width:1280px) { .add-grid { grid-template-columns:repeat(2, minmax(0,1fr)); } }
@media (max-width:900px) { .add-grid { grid-template-columns:1fr; } }
.add-col-local.off, .add-col-recorder.off, .add-col-bilibili.off { opacity:.75; }

/* 录播接入列：上房间 / 下录像 */
/* 录像列表吃掉本列剩余高度：列高跟随整行（本地列最高），超出就滚动 */
.rec-list { flex: 1 1 0; min-height: 120px; overflow-y: auto; padding-right: 4px; }

/* ── 添加录像：左选来源 / 右填参数（与设置页同一套两栏手感） ── */
.add-split { display:grid; grid-template-columns:196px minmax(0,1fr); gap:var(--s-5); align-items:start; }
.add-nav { display:flex; flex-direction:column; gap:2px; }
.add-nav-item { display:flex; align-items:flex-start; gap:9px; padding:8px 10px; text-align:left;
  border:1px solid transparent; border-radius:var(--r-sm); background:none; cursor:pointer;
  color:var(--ink-dim); transition:background-color var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease); }
.add-nav-item > svg { flex:none; margin-top:2px; }
.add-nav-body { display:flex; flex-direction:column; gap:1px; min-width:0; }
.add-nav-body b { font-size:var(--fs-base); font-weight:var(--fw-medium); }
.add-nav-body span { font-size:var(--fs-xs); color:var(--ink-faint); line-height:1.4; }
.add-nav-item:hover { background:var(--surface-2); color:var(--ink); }
.add-nav-item.on { background:var(--main-soft); border-color:var(--main-glow); color:var(--ink); }
.add-nav-item.on b { color:var(--pink-text); font-weight:var(--fw-semi); }
.add-nav-item.off { opacity:.7; }
/* 三个来源面板**必须同一高度**：切换来源时卡片高度乱跳是很明显的观感问题。
   做法：容器定高、每个面板 `height:100%`，面板内部自己分配（浏览器/卡片区吃掉余量）。 */
.add-body { min-width:0; display:flex; flex-direction:column;
  height:min(520px, 64vh); min-height:400px; }
.add-body > * { flex:1 1 auto; min-height:0; }
/* 右栏只放一个来源的表单，宽度铺满 */

@media (max-width:1000px) {
  .add-split { grid-template-columns:1fr; gap:var(--s-3); }
  .add-nav { flex-direction:row; gap:6px; overflow-x:auto; scrollbar-width:none; }
  .add-nav::-webkit-scrollbar { display:none; }
  .add-nav-item { flex:none; white-space:nowrap; }
  .add-nav-body span { display:none; }
}

/* ── 开始使用：顶部环境状态卡 ── */
.start-hero { display:flex; align-items:center; gap:12px; padding:14px 16px; }
.sh-ic { flex:none; color:var(--amber-deep); }
.sh-ic.ok { color:var(--green); }
.sh-main { flex:1 1 auto; min-width:0; display:flex; flex-direction:column; gap:3px; }
.sh-title { font-size:14px; }
@media (max-width:640px) { .start-hero { flex-wrap:wrap; } .start-hero .btn, .start-hero .mini-btn { margin-left:28px; } }

</style>
