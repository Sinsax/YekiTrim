<template>
  <div class="wb-root">
    <div class="page-head">
      <div>
        <h1>工作台</h1>
        <div class="sub">把录像变成项目</div>
      </div>
      <div class="actions">
        <button class="btn" @click="refresh"><Icon name="refresh" :size="14" /> 刷新</button>
      </div>
    </div>

    <div v-if="wb.error" class="error-banner"><Icon name="alert" :size="15" /> {{ wb.error }}</div>

    <!-- ═════ 切片总览：全部项目的切片卡片预览（进行中 / 已完成导出 两个列表） ═════ -->
    <HomeClipsOverview />

    <!-- ═════ 新增项目：先选来源，再走对应导入方式 ═════ -->
    <div class="card card-blk mb-12">
      <div class="row spread mb-8">
        <b class="t m-0"><Icon name="plus" :size="15" /> 新增项目</b>
        <span class="muted small">选一个来源，下面切换成对应的导入方式</span>
      </div>

      <div class="wiz-srcs">
        <div
          v-for="s in sources"
          :key="s.key"
          class="wiz-src"
          :class="{ on: source === s.key, off: s.disabled }"
          :title="s.desc"
          @click="!s.disabled && (source = s.key)"
        >
          <Icon :name="s.icon" :size="15" />
          <b>{{ s.title }}</b>
          <span v-if="s.badge" class="chip n">{{ s.badge }}</span>
        </div>
      </div>

      <!-- ① 服务器文件（仅管理员可见；非管理员连来源按钮都没有）
           两种选法**都要**（用户口径："能输入选择 + 弹出文件浏览器选择"，是**增加**不是替代）：
             · **输入选择** = 下面这份内联 ServerDirPicker（可直接填/粘贴服务器绝对路径，
               就地列目录、点文件、用快捷入口）—— 与字体库那份同一套。
             · **弹出浏览** = 上面那颗按钮开大窗（`AppModal wide`），深目录导航时视野更宽。
           两者写同一个已选状态；弹出层额外用**草稿态**，只有点「选定」才写回（否则
           "进去逛一圈又关掉"会把中途路过的目录当成选定）。 -->
      <!-- 服务器文件：**左 = 目录操作（浏览 / 选文件）· 右 = 按钮与注意事项**。
           用户口径："左边是目录操作，右边是各种按钮和注意事项"。 -->
      <div v-if="source === 'local' && isAdmin" class="src-body src-split">
        <div class="src-col">
          <ServerDirPicker
            mode="file"
            :start-dir="localDir"
            :selected="localInput"
            :exts="LOCAL_EXTS"
            :project-file="PROJECT_FILE"
            dir-label="项目目录（服务器上的绝对路径，可直接填或粘贴）"
            dir-placeholder="例如 F:/media/2025-09-12"
            @update:dir="onLocalDir"
            @update:listing="localListing = $event"
            @pick-file="pickLocalFile"
          />
        </div>

        <!-- 右栏：说明 + 当前状态 + 两个动作按钮，竖排 -->
        <div class="src-col src-side">
          <p class="muted small">只读服务器上的文件，不接受上传</p>
          <template v-if="localDir">
            <div class="small mono ellip" :title="localDir">项目将建在 {{ localDir }}</div>
            <!-- 已有工程：关掉选择器之后仍然看得见这个状态 -->
            <div v-if="localHasConfig" class="small ok-text">
              检测到已有工程（<span class="mono">{{ PROJECT_FILE }}</span>）—— 将<b>直接添加</b>
            </div>
            <!-- ⚠️ "重建"警告**必须排在"显示输入视频"前面**：两者条件同时成立，
                 写成 v-else-if 挂在后面就永远轮不到它。 -->
            <div v-if="localHasConfig && localManualVideo" class="small warn-text">
              已有工程，将<b>重新初始化</b>（要确认）
            </div>
            <!-- ⚠️ 必须显示"实际会用的那个视频"：后端在 input 为空时自动挑第一个 -->
            <div v-if="localEffectiveVideo" class="small">
              输入视频 <span class="mono">{{ localEffectiveVideo }}</span>
              <span v-if="!localManualVideo" class="muted">（自动选取目录里第一个视频）</span>
            </div>
            <div v-else-if="!localHasConfig && !localListing.failed" class="small warn-text">
              这个目录没有可用视频
            </div>
          </template>
          <button class="btn" @click="openLocalPicker">
            <Icon name="folder" :size="14" /> 弹出浏览…
          </button>
          <button class="btn btn-primary"
                  :disabled="creating || !canCreateLocal" @click="createFromLocal">
            <span v-if="creating" class="spinner"></span> {{ localAddMode ? '添加项目' : '新建项目' }}
          </button>
        </div>

        <!-- 弹出浏览：**双栏文件浏览器**，草稿态 —— 点「选定」才写回已选。 -->
        <AppModal v-if="pickerOpen" :show="true" wide title="浏览服务器上的录像"
                          @close="cancelLocalPick">
                  <ServerFileBrowser
                    :start-dir="localDir"
                    :selected="draftInput"
                    :exts="LOCAL_EXTS"
                    :project-file="PROJECT_FILE"
                    @update:dir="onDraftDir"
                    @update:listing="draftListing = $event"
                    @pick-file="pickDraftFile"
                  />
                  <template #footer>
                    <div class="flex-1 small">
                      <template v-if="draftDir">
                        <span class="mono">{{ draftDir }}</span>
                        <template v-if="draftEffectiveVideo">
                          <br>输入视频 <span class="mono">{{ draftEffectiveVideo }}</span>
                          <span v-if="!draftInput" class="muted">（自动选取目录里第一个视频）</span>
                        </template>
                        <template v-else>
                          <br><span class="warn-text">这个目录里没有可用的视频，选一个别的</span>
                        </template>
                      </template>
                      <span v-else class="muted">还没选目录</span>
                    </div>
                    <button class="btn" @click="cancelLocalPick">取消</button>
                    <button class="btn btn-primary" :disabled="!draftCanConfirm" @click="confirmLocalPick">选定</button>
                  </template>
                </AppModal>
      </div>

      <!-- ② 录播姬录制：待处理的会话 + 扫描 -->
      <div v-if="source === 'recorder'" class="src-body">
        <!-- 用户唯一要做的：填直播间。未对接录播姬时整块置灰，且**不给管理员向的配置指引** -->
        <div class="room-add" :class="{ off: roomsAvailable === false }">
          <div class="row">
            <input
              v-model="roomInput"
              class="flex-1"
              :disabled="roomsAvailable !== true || roomBusy"
              placeholder="直播间号或链接，例如 12345 / https://live.bilibili.com/12345"
              @keyup.enter="addRoom"
            >
            <button class="btn btn-primary" :disabled="roomsAvailable !== true || roomBusy || !roomInput.trim()" @click="addRoom">
              <span v-if="roomBusy" class="spinner"></span> 添加
            </button>
          </div>
          <div v-if="roomsAvailable !== null" class="hint mt-4">
            <template v-if="roomsAvailable">开播自动录制并入库到你的项目</template>
            <template v-else>录播姬未接入，暂时不能添加</template>
          </div>
        </div>

        <!-- 我登记的直播间：每行可展开看该房间录了哪些文件（单个导入） -->
        <div v-if="rooms.length" class="room-list">
          <div v-for="r in rooms" :key="r.room_id" class="room-item">
            <div class="room-row">
              <button class="mini-btn room-toggle" :title="expanded[r.room_id] ? '收起录像' : '展开录像'"
                      :aria-expanded="expanded[r.room_id] ? 'true' : 'false'" @click="toggleRoom(r)">
                {{ expanded[r.room_id] ? '▾' : '▸' }}
              </button>
              <b class="mono room-id">{{ r.room_id }}</b>
              <span class="muted small room-title" :title="r.title">{{ r.title || '（未取到标题）' }}</span>
              <span class="chip" :class="roomChip(r)">{{ roomState(r) }}</span>
              <span v-if="recOf(r).files" class="muted small room-rec"
                    :title="recOf(r).latest ? '最新 ' + fmtTime(recOf(r).latest) : ''">
                已录 {{ recOf(r).files }} 个 · {{ fmtBytes(recOf(r).bytes) }}
              </span>
              <button class="mini-btn danger" :disabled="roomBusy" @click="removeRoom(r)">删除</button>
            </div>

            <div v-if="expanded[r.room_id]" class="room-files">
              <div v-if="roomFiles[r.room_id]?.loading" class="row muted small">
                <span class="spinner"></span> 读取录像目录…
              </div>
              <div v-else-if="roomFiles[r.room_id]?.error" class="error-banner">{{ roomFiles[r.room_id].error }}</div>
              <template v-else>
                <div v-if="!roomFiles[r.room_id]?.files.length" class="muted small">
                  录像目录里没有这个房间的文件
                </div>
                <div v-else class="row spread file-list-head">
                  <span class="muted small">共 {{ roomFiles[r.room_id].total }} 个录像</span>
                  <button class="mini-btn" :disabled="roomFiles[r.room_id].loading" @click="loadRoomFiles(r, true)">刷新</button>
                </div>
                <div v-for="f in roomFiles[r.room_id]?.files || []" :key="f.rel" class="rec-row">
                  <span class="mono small rec-name" :title="f.rel">{{ f.name }}</span>
                  <span class="muted small rec-meta">{{ fmtBytes(f.size) }} · {{ fmtTime(f.mtime) }}</span>
                  <span class="chip" :class="recChip(f)">{{ f.state_label }}</span>
                  <button v-if="f.state === 'new'" class="mini-btn primary"
                          :disabled="importing[f.rel]" @click="importFile(r, f)">
                    <span v-if="importing[f.rel]" class="spinner"></span> 导入
                  </button>
                  <router-link v-else-if="f.project_id" class="mini-btn" :to="`/p/${f.project_id}/slices`">打开项目</router-link>
                </div>
                <div v-if="roomFiles[r.room_id].shown < roomFiles[r.room_id].total" class="muted small mt-6">
                  只显示了最新的 {{ roomFiles[r.room_id].shown }} 个。
                </div>
              </template>
            </div>
          </div>
        </div>
        <div v-else-if="!roomsLoading" class="muted small mt-8">
          还没有登记直播间，填一个房间号即可
        </div>

        <!-- 管理员：待分配兜底 + 扫描 + 接入状态；普通用户完全不显示这一块 -->
        <template v-if="isAdmin">
          <div class="row mt-12">
            <button class="btn" :disabled="scanning" @click="scanNow">
              <span v-if="scanning" class="spinner"></span> 扫描录制目录
            </button>
            <router-link class="mini-btn" to="/ingest">全部记录与导入规则</router-link>
            <button class="mini-btn" :disabled="recorderTesting" @click="testRecorder">
              <span v-if="recorderTesting" class="spinner"></span> 测试录播姬连接
            </button>
          </div>
          <div v-if="recorderInfo" class="hint mt-6">{{ recorderInfo }}</div>
          <div v-if="ingestReady === false" class="hint mt-6">
            录播姬尚未接入：在 <b>设置页 →「录播姬（采集）」</b> 里填录像目录与 API 地址、生成 webhook token
            （命令行等价：<span class="mono">yekitrim ingest token</span> +
            <span class="mono">[ingest.recorder] root</span>）。
            未归属或入库失败的录像在 <router-link to="/ingest">全部记录与导入规则</router-link> 里处理。
          </div>
        </template>
      </div>

      <!-- ③ B 站链接：粘贴 → 解析预览 → 勾选分P → 下载 -->
      <div v-if="source === 'bilibili'" class="src-body">
        <!-- 下载器没装：先解决这个（管理员一键装，普通用户只能等） -->
        <div v-if="dl.ready === false" class="bili-gate">
          <div class="row items-start">
            <Icon name="alert" :size="15" />
            <div class="flex-1">
              <b>下载器未就绪</b>
              <div class="muted small mt-4">
                B 站下载需要 <span class="mono">yt-dlp</span>（{{ dl.license || 'Unlicense' }}，
                {{ dl.pinned_version ? '版本 ' + dl.pinned_version : '内置' }}）。
                <template v-if="isAdmin">点右边按钮下载并校验（约 17 MB，只下一次）。</template>
                <template v-else>请让管理员在「设置 → 系统」里安装</template>
              </div>
              <div v-if="dlError" class="error-banner mt-6">{{ dlError }}</div>
            </div>
            <button v-if="isAdmin" class="btn btn-primary" :disabled="dlBusy" @click="installDownloader">
              <span v-if="dlBusy" class="spinner"></span> 下载并校验
            </button>
          </div>
        </div>

      <template v-else>
        <!-- 存放位置没配置：管理员给去处（仅管理员可见这张卡）；普通用户提交时后端会拒绝并提示联系管理员 -->
        <div v-if="isAdmin && rootConfigured === false" class="bili-gate">
          <div class="row items-start">
            <Icon name="alert" :size="15" />
            <div class="flex-1">
              <b>尚未配置项目存放位置</b>
              <div class="muted small mt-4">
                下载前需要先确定项目落到哪个目录。去
                <router-link to="/settings/integrations">设置 → 下载</router-link>
                里配置「项目存放位置」后即可开始下载。
              </div>
            </div>
            <router-link class="btn btn-primary" to="/settings/integrations">去配置</router-link>
          </div>
        </div>
        <div class="row">
            <input
              v-model="biliUrl"
              class="flex-1"
              :disabled="biliBusy"
              placeholder="粘贴视频链接或 BV 号，例如 https://www.bilibili.com/video/BV1xx  /  BV1xx"
              @keyup.enter="resolveBili"
            >
            <button class="btn btn-primary" :disabled="biliBusy || !biliUrl.trim()" @click="resolveBili">
              <span v-if="biliBusy && !biliInfo" class="spinner"></span> 解析
            </button>
          </div>
          <div v-if="biliError" class="error-banner mt-8"><Icon name="alert" :size="15" /> {{ biliError }}</div>
          <div v-if="biliHint" class="hint mt-6">{{ biliHint }}</div>

          <!-- 预览卡 -->
          <div v-if="biliInfo" class="bili-preview">
            <img v-if="biliInfo.cover" class="bili-cover" :src="biliInfo.cover" alt="" referrerpolicy="no-referrer">
            <div class="bili-meta flex-1">
              <b class="bili-title" :title="biliInfo.title">{{ biliInfo.title }}</b>
              <div class="muted small mt-4">
                {{ biliInfo.uploader || '（未知 UP 主）' }}
                <template v-if="biliInfo.duration_secs"> · {{ fmtClock(biliInfo.duration_secs) }}</template>
                <template v-if="biliInfo.danmaku_total"> · 弹幕 {{ fmtCount(biliInfo.danmaku_total) }}</template>
                <template v-if="biliInfo.view_total"> · 播放 {{ fmtCount(biliInfo.view_total) }}</template>
              </div>
              <div class="muted small mt-4">
                {{ biliInfo.pages.length }} 个分P · 已选 {{ pickedPages.length }} 个 ·
                预计占用 <b>{{ fmtBytes(pickedEstimate) }}</b>
                <span class="muted">（清晰度上限 {{ biliInfo.quality }}，在设置页改）</span>
              </div>
            </div>
          </div>

          <!-- 多P 勾选：默认全不选（P 多的时候全下会撑爆配额） -->
          <div v-if="biliInfo?.pages?.length" class="bili-pages">
            <div class="row spread bili-pages-head">
              <span class="muted small">
                <template v-if="biliInfo.pages.length > 1">选择要下载的分P</template>
                <template v-else>单 P 视频</template>
              </span>
              <div v-if="biliInfo.pages.length > 1" class="row" style="gap:6px">
                <button class="mini-btn" @click="pickAllPages">全选</button>
                <button class="mini-btn" @click="pickedPages = []">清空</button>
              </div>
            </div>
            <label
              v-for="p in biliInfo.pages"
              :key="p.index"
              class="bili-page"
              :class="{ on: pickedPages.includes(p.index) }"
            >
              <input type="checkbox" :value="p.index" v-model="pickedPages">
              <span class="mono bili-pnum">P{{ p.index }}</span>
              <span class="bili-pname flex-1" :title="p.part">{{ p.part || '（无标题）' }}</span>
              <span class="muted small">{{ p.duration_secs ? fmtClock(p.duration_secs) : '-' }}</span>
              <span class="muted small bili-pest">{{ p.estimate_bytes ? fmtBytes(p.estimate_bytes) : '-' }}</span>
            </label>
          </div>

          <div v-if="biliInfo" class="row mt-8">
            <span class="muted small flex-1">
              下载完成后自动建项目；<b>弹幕会一并抓取</b>（事件检测的热度排序要用）。
              <template v-if="!pickedPages.length">请先勾选分P。</template>
            </span>
            <button
              class="btn btn-primary"
              :disabled="biliSubmitting || !pickedPages.length || (isAdmin && rootConfigured === false)"
              @click="submitBili"
            >
              <span v-if="biliSubmitting" class="spinner"></span>
              开始下载{{ pickedPages.length > 1 ? `（${pickedPages.length} 个）` : '' }}
            </button>
          </div>

          <!-- 已提交的结果 -->
          <div v-if="biliSubmitted" class="bili-done">
            <div class="row spread">
              <b class="small">已提交 {{ biliSubmitted.accepted.length }} 个下载</b>
              <router-link class="mini-btn" to="/ingest">查看进度</router-link>
            </div>
            <div v-for="a in biliSubmitted.accepted" :key="a.session_id" class="muted small mt-4">
              · P{{ a.page }} {{ a.title }}
            </div>
            <div v-for="s in biliSubmitted.skipped" :key="s.session_id" class="muted small mt-4">
              · P{{ s.page }} 已跳过（{{ s.reason }}）
            </div>
          </div>

          <p class="muted small mt-8" title="番剧 / 课程 / UP 主空间 / 收藏夹等批量链接不支持，认不出来会明确报错">
            只支持单个视频与它的分P
          </p>
        </template>
      </div>
    </div>

    <!-- ⚠️ 位置由 `doctorProblem` 决定：**正常排最底部、有问题排最上面**（用户口径）——
         靠 flex `order` 换位，而不是把这段 markup 抄两份。 ═════ -->
    <!-- ═════ 运行状况：这台机器上「干活要用的东西」齐不齐 ═════
         ffmpeg / ffprobe 是剪切与出片的外部依赖，ASR 模型是转写的前提，两者都是
         **全机共享的程序级资产**（跟"谁在登录"无关）—— 所以它们不该住在账号页。
         数据来自 `GET /api/doctor`（外部命令是否可用 + 模型清单）与 `GET /api/models`
         （装在哪一份、多大、怎么下）。写操作（下载）仅管理员，由 ModelList 自己门控。 -->
    <div class="card card-blk mb-12" :style="{ order: doctorProblem ? -1 : 99 }">
      <div class="row spread mb-8">
        <b class="t" style="margin:0">运行状况</b>
        <span class="muted small" title="由服务端在当前进程里实测，不是缓存的旧值">
          {{ doctor ? '已实测' : '读取中…' }}
        </span>
      </div>
      <div v-if="doctor" class="row mb-8">
        <span class="chip" :class="doctor.ffmpeg ? 'g' : 'n'">ffmpeg {{ doctor.ffmpeg ? '就绪' : '缺失' }}</span>
        <span class="chip" :class="doctor.ffprobe ? 'g' : 'n'">ffprobe {{ doctor.ffprobe ? '就绪' : '缺失' }}</span>
        <span class="muted small" :title="doctor.models_dir">模型目录</span>
      </div>
      <div v-if="doctorError" class="error-banner mb-8">{{ doctorError }}</div>
      <ModelList :models="models" @changed="loadDoctor" />
    </div>

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
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import Icon from '../components/Icon.vue'
import ServerDirPicker, { VIDEO_EXTS } from '../components/ServerDirPicker.vue'
import AppModal from '../components/AppModal.vue'
import ServerFileBrowser from '../components/ServerFileBrowser.vue'
import MediaLibrary from '../components/MediaLibrary.vue'
import HomeClipsOverview from '../components/HomeClipsOverview.vue'
import ModelList from '../components/ModelList.vue'
import { authState } from '../stores/auth'
import { localNoAuthKnown } from '../stores/health'
import { createLocalProject } from '../stores/project'

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
import { fmtBytes, fmtTime, ingestState, setReady } from '../stores/ingest'
import { fmtClock, fmtCount } from '../utils'
import { loadProjects, wb } from '../stores/workbench'
import { api } from '../api/client'

const isAdmin = computed(() => authState.user?.role === 'admin')

// 来源选择：不同来源 → 不同导入方式（本地浏览 / 录播姬待处理 / B站链接）
// 「服务器文件」是 admin-only 来源（`/api/fs/list`、`kind:"local"` 都对非管理员 403），
// 所以非管理员**整条不显示** —— 不给一个点不动、只写着"仅管理员"的入口。
/**
 * 默认来源。本地模式（`--local-no-auth` 单机形态）默认落在**服务器文件** ——
 * 那才是"把盘上已有的录像变成项目"的主路径；录播姬是给接了采集的人用的。
 * ⚠️ `localNoAuthKnown()` 能同步拿到，是因为路由守卫在**任何视图挂载之前**已经 await 过它
 * （见 router.js）—— 否则这里只能等异步，会先闪一下录播姬再跳过来。
 */
const source = ref(localNoAuthKnown() ? 'local' : 'recorder')
const sources = computed(() => {
  const list = []
  if (isAdmin.value) {
    list.push({ key: 'local', title: '服务器文件', desc: '浏览服务器上已有的录像，直接建项目', icon: 'folder', badge: '' })
  }
  list.push(
    {
      key: 'recorder',
      title: '录播姬录制',
      desc: '录完自动入库；历史/漏推送可扫描导入',
      icon: 'download',
      badge: ''
    },
    {
      key: 'bilibili',
      title: 'B 站链接',
      desc: '粘贴视频地址，下载完成后自动建项目（弹幕一并抓取）',
      icon: 'link',
      badge: ''
    }
  )
  return list
})
// 选中的来源必须始终是"看得见且可用"的那一个（角色变化 / 列表变化时自动落到第一个可用的）
watch(sources, (list) => {
  if (!list.some((s) => s.key === source.value && !s.disabled)) {
    source.value = (list.find((s) => !s.disabled) || list[0])?.key || 'recorder'
  }
}, { immediate: true })

// ---- ② 我的直播间（录播姬）----
const rooms = ref([])
/// null = 还没问到（别先闪一下「未接入」）；true/false = 录播姬可用位
const roomsAvailable = ref(null)
const roomsLoading = ref(false)
const roomInput = ref('')
const roomBusy = ref(false)
const recorderTesting = ref(false)
const recorderInfo = ref('')

/** 我的直播间 + 录播姬可用位。普通用户的响应里没有 `reason`（不暴露管理配置）。 */
async function loadRooms() {
  roomsLoading.value = true
  try {
    const d = await api('/api/rooms')
    rooms.value = d?.rooms || []
    roomsAvailable.value = d?.available === true
    if (isAdmin.value && d?.reason) recorderInfo.value = d.reason
  } catch (e) {
    rooms.value = []
    roomsAvailable.value = false
    if (isAdmin.value) recorderInfo.value = '读取直播间失败：' + (e?.message || '')
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

function pickLocalFile(name) {
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
/// 存放位置是否已显式配置（`[storage] projects_root`）。null = 还没问到 / 查询失败
/// （此时**不拦**，让后端兜底——前端查不到就别替后端做决定）。
/// 只有管理员会查（config 是 admin-only 端点），普通用户提交时由后端按角色给文案。
const rootConfigured = ref(null)

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

async function refresh() {
  await loadProjects()
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

/**
 * 运行状况"有没有问题"—— 决定它在首页的位置（用户口径：**正常沉底、有问题冒头**）。
 *
 * 判据 = 外部命令缺失 · 读取失败 · **有模型没装**。三者都会让"跑一遍管线"走不通，
 * 所以都算问题。⚠️ 还没读到时**不算问题**（`doctor` 为 null）—— 否则首帧会先冒头再沉下去。
 */
const doctorProblem = computed(() => {
  if (doctorError.value) return true
  const d = doctor.value
  if (!d) return false
  if (!d.ffmpeg || !d.ffprobe) return true
  return models.value.some((m) => !m.installed)
})

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

onMounted(async () => {
  await loadProjects()
  loadDoctor()
  try {
    const h = await api('/api/health')
    setReady(h?.ingest_ready !== false)
    // 下载器就绪位顺便从 health 拿一次（省一个请求），再单独查一次细节（版本/许可）
    if (h?.downloader) Object.assign(dl, { ready: h.downloader.ready === true, verified: h.downloader.verified === true, kind: h.downloader.kind || '' })
  } catch {} // /api/health 是高频成功路径，失败由 api() 记日志即可（不挡首屏）
  await loadRooms()
  await loadDownloader()
  // 管理员顺路查一次存放位置是否已配置（config 是 admin-only；失败不拦，后端兜底）
  if (isAdmin.value) {
    try {
      const d = await ingestApi.config()
      rootConfigured.value = !!(d?.storage?.projects_root || '').trim()
    } catch {}
  }
})

// 从设置页回来要重查（组件被 keep-alive / 复用时 onMounted 不会重跑）
const route = useRoute()
watch(() => route.fullPath, (to, from) => {
  if (isAdmin.value && to !== from && String(to).startsWith('/workbench')) {
    ingestApi.config()
      .then((d) => { rootConfigured.value = !!(d?.storage?.projects_root || '').trim() })
      .catch(() => {})
  }
})
</script>

<style scoped>
/* 来源选择已改紧凑一行，说明走 title 悬停 */
.src-body { margin-top:14px; border-top:1px dashed var(--line-soft); padding-top:14px; }
/* 服务器文件：左目录操作 / 右按钮与注意事项（用户口径）。窄屏收成单列。 */
.src-split { display:grid; grid-template-columns:minmax(0,1fr) minmax(240px,300px); gap:0 20px; align-items:start; }
@media (max-width:1100px) { .src-split { grid-template-columns:minmax(0,1fr); gap:14px 0; } }
.src-col { min-width:0; }
/* 右栏：竖排；按钮占满一列宽（横排时会挤成两行） */
.src-side { display:flex; flex-direction:column; gap:8px; }
.src-side .btn { width:100%; justify-content:center; }
.src-side .create-bar { display:block; }
/* 首页根：flex 列只为让「运行状况」用 order 换位（正常沉底、有问题冒头） */
.wb-root { display:flex; flex-direction:column; }
/* 「配置直播间」：录播姬不可用时整块置灰（用户只看到"暂时不能添加"，看不到管理配置） */
.room-add.off { opacity:.55; }
.room-list { margin-top:10px; }
.room-item { border-bottom:1px dashed var(--line-soft); }
.room-item:last-child { border-bottom:none; }
.room-row { display:flex; align-items:center; gap:10px; padding:6px 0; }
.room-toggle { flex:none; width:22px; padding:2px 0; text-align:center; }
.room-id { flex:none; font-size:13px; }
.room-title { flex:1; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.room-rec { flex:none; white-space:nowrap; }
/* 展开区：贴在房间行下面，左侧缩进对齐房间号 */
.room-files { margin:2px 0 8px 32px; padding-left:10px; border-left:2px solid var(--line-soft); }
.file-list-head { padding-bottom:2px; }
.rec-row { display:flex; align-items:center; gap:8px; padding:4px 0; flex-wrap:wrap; }
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
/* 分P 列表：可滚，P 多时不把页面撑长 */
.bili-pages {
  margin-top:10px; max-height:280px; overflow-y:auto;
  border:1px solid var(--line-soft); border-radius:8px; padding:4px 8px;
}
.bili-pages-head { padding:4px 0 6px; position:sticky; top:0; background:var(--bg, transparent); z-index:1; }
.bili-page {
  display:flex; align-items:center; gap:10px; padding:5px 4px; cursor:pointer;
  border-radius:6px; border-bottom:1px dashed var(--line-soft);
}
.bili-page:last-child { border-bottom:none; }
.bili-page:hover { background:var(--bg-soft, transparent); }
.bili-page.on { background:var(--bg-soft, transparent); }
.bili-pnum { flex:none; font-size:12px; min-width:34px; }
.bili-pname { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:13px; }
.bili-pest { flex:none; min-width:64px; text-align:right; }
.bili-done {
  margin-top:10px; padding:10px 12px; border:1px solid var(--line-soft);
  border-radius:8px; background:var(--bg-soft, transparent);
}
</style>
