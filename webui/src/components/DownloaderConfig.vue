<template>
  <div>
    <div class="row spread mb-8">
      <b class="t" style="margin:0"><Icon name="link" :size="15" /> B站链接下载（P4）</b>
      <span class="chip" :class="stateChip.cls">{{ stateChip.text }}</span>
    </div>

    <div v-if="!loaded" class="row muted small"><span class="spinner"></span> 读取中…</div>

    <template v-else>
      <!-- ═══ 下载器本体：装没装 / 装哪儿 / 是不是钉死的那一份 ═══ -->
      <div class="dl-box">
        <div class="row" style="gap:8px; align-items:flex-start">
          <Icon name="download" :size="15" />
          <div class="flex-1">
            <b class="small">{{ status.kind || 'yt-dlp' }}</b>
            <div class="muted small mt-4">
              <template v-if="status.ready">
                已就绪 · <span class="mono">{{ status.source }}</span>
                <template v-if="status.verified"> · sha256 校验通过</template>
                <template v-else> · <span class="warn-t">未走托管校验</span>（配置指定或 PATH 里的副本）</template>
              </template>
              <template v-else>
                未安装。点右边按钮下载（约 17 MB，按 sha256 校验）
              </template>
            </div>
            <div class="muted small mt-4">
              许可 <span class="mono">{{ status.license }}</span> · 钉死版本
              <span class="mono">{{ status.pinned_version }}</span>
              —— 随发行物分发不引入 copyleft 义务。
            </div>
          </div>
          <button class="btn" :disabled="installBusy" @click="install">
            <span v-if="installBusy" class="spinner"></span>
            {{ status.ready ? '重新下载并校验' : '下载并校验' }}
          </button>
        </div>
      </div>

      <!-- ═══ 引擎与路径（逃生舱） ═══ -->
      <div class="grid2 mt-12">
        <label class="fld">
          <span class="lbl">引擎</span>
          <select v-model="form.engine">
            <option value="yt-dlp">内置 yt-dlp（推荐：版本与哈希已钉死）</option>
            <option value="__custom__">自备可执行文件…</option>
          </select>
          <span class="muted small">
            指向你自己更新的 yt-dlp
            <b>不随发行物分发</b>，因此不进 THIRD-PARTY-NOTICES。
          </span>
        </label>

        <label v-if="form.engine === '__custom__'" class="fld">
          <span class="lbl">可执行文件路径</span>
          <div class="row" style="gap:6px">
            <input v-model="form.ytdlp_path" class="flex-1 mono" placeholder="例如 D:\tools\yt-dlp.exe">
            <button class="mini-btn" @click="pickTarget = 'exe'; showPicker = true">浏览…</button>
          </div>
          <span class="muted small" title="本项目的下载参数按 yt-dlp 的命令行契约写">需兼容 yt-dlp 命令行契约</span>
        </label>
      </div>

      <!-- ═══ 存放位置：下载与入库的**共同**落点（属于 [storage]，不是 B站专属） ═══ -->
      <label class="fld mt-12">
        <span class="lbl">项目存放位置</span>
        <div class="row" style="gap:6px">
          <input
            v-model.trim="form.projects_root"
            class="flex-1 mono"
            placeholder="留空 = 程序目录下的 .yekitrim/projects"
          >
          <button class="mini-btn" @click="pickTarget = 'dir'; showPicker = true">浏览…</button>
          <button v-if="form.projects_root" class="mini-btn" @click="form.projects_root = ''">恢复默认</button>
        </div>
        <span class="muted small">
          <b>下载与入库都落在这里</b>，布局是
          <span class="mono">&lt;这里&gt;/users/&lt;账号&gt;/&lt;房间或 BV 号&gt;/&lt;时间戳&gt;-&lt;pid&gt;</span>。
          录播姬入库与本机导入共用这个根
        </span>
        <span class="muted small">
          当前生效：<span class="mono">{{ effectiveRoot }}</span>
          <span class="muted" title="不存在时保存会创建；只影响新建项目">（留空 = 程序目录）</span>
        </span>
      </label>

      <!-- ═══ 下载参数 ═══ -->
      <div class="grid2 mt-12">
        <label class="fld">
          <span class="lbl">清晰度上限</span>
          <select v-model="form.max_quality">
            <option v-for="q in QUALITIES" :key="q.value" :value="q.value">{{ q.label }}</option>
          </select>
          <span class="muted small">
            下载一律偏好 <span class="mono">H.264</span>（默认会选到 HEVC，WebUI 预览放不出来）。
          </span>
        </label>

        <label class="fld">
          <span class="lbl">限速（KB/s）</span>
          <input v-model.number="form.rate_limit_kbps" type="number" min="0" step="128">
          <span class="muted small">0 = 不限。带宽紧张或想给 B站留点余地时用。</span>
        </label>

        <label class="fld">
          <span class="lbl">同时下载数</span>
          <input v-model.number="form.max_concurrent_downloads" type="number" min="1" max="8">
          <span class="muted small">
            <b>建议保持 1</b>：B站的 412 风控与 IP 频次相关，调大更容易被拦。
          </span>
        </label>

        <label class="fld">
          <span class="lbl">卡死判定（秒）</span>
          <input v-model.number="form.idle_timeout_secs" type="number" min="30" step="30">
          <span class="muted small">
            连续这么久<b>没有任何输出</b>就判卡死并 kill
          </span>
        </label>
      </div>

      <!-- ═══ cookie（清晰度/风控） ═══ -->
      <div class="fld mt-12">
        <span class="lbl">B站 cookie 文件</span>
        <div class="row" style="gap:6px">
          <input v-model="form.cookie_file" class="flex-1 mono" placeholder="留空 = 不带 cookie（清晰度封顶 480/720p）">
          <button class="mini-btn" @click="pickTarget = 'cookie'; showPicker = true">浏览…</button>
          <button v-if="form.cookie_file" class="mini-btn" @click="form.cookie_file = ''">清除</button>
        </div>
        <span class="muted small">
          必须是 <b>Netscape 格式</b>，JSON 会被静默忽略
          <span class="muted">（部分视频没有它直接下不了）</span>
        </span>
      </div>

      <div class="row mt-12" style="gap:8px">
        <button class="btn btn-primary" :disabled="busy" @click="save">
          <span v-if="busy" class="spinner"></span> 保存
        </button>
        <button class="btn" :disabled="busy" @click="load">重新读取</button>
      </div>
      <div v-if="warnings.length" class="hint mt-8">
        <div v-for="(w, i) in warnings" :key="i">· {{ w }}</div>
      </div>
      <p class="muted small mt-8">
        只支持单个视频与它的分P；选集/课程等不支持
      </p>
    </template>

    <ServerDirPicker
      v-if="showPicker"
      :mode="pickTarget === 'dir' ? 'dir' : 'file'"
      :start-dir="pickDir"
      :selected="pickTarget === 'dir' ? '' : (pickTarget === 'cookie' ? form.cookie_file : form.ytdlp_path)"
      dir-label="目录（项目与下载的存放位置，绝对路径）"
      @update:dir="pickDir = $event"
      @pick-file="onPicked"
      @pick-dir="onPicked"
    />
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import Icon from './Icon.vue'
import ServerDirPicker from './ServerDirPicker.vue'
import { ingestApi } from '../api/ingest'
import { toast } from '../stores/toast'

const QUALITIES = [
  { value: '1080p', label: '1080p（推荐）' },
  { value: '720p', label: '720p' },
  { value: '480p', label: '480p' },
  { value: '360p', label: '360p' },
  { value: 'best', label: '不限（best，按源最高）' }
]

const loaded = ref(false)
const busy = ref(false)
const installBusy = ref(false)
const warnings = ref([])
const showPicker = ref(false)
const pickTarget = ref('exe')
/// 选择器当前所在目录（用户在里面翻目录时要留住，否则每点一次都跳回根）
const pickDir = ref('')

/// 下载器就绪位（独立端点，**响应里不含路径**）
const status = ref({ ready: null, verified: false, kind: '', pinned_version: '', license: '', source: '' })

/// 项目存放位置**解析后**的真实落点（服务端算好回传；留空时是程序目录下的 .yekitrim/projects）。
/// 前端不自己拼这个路径 —— 它会随程序目录变化，拼出来必然和实际不一致。
const effectiveRoot = ref('')

const form = ref({
  engine: 'yt-dlp',
  ytdlp_path: '',
  max_quality: '1080p',
  rate_limit_kbps: 0,
  max_concurrent_downloads: 1,
  cookie_file: '',
  idle_timeout_secs: 120,
  projects_root: ''
})

const stateChip = computed(() => {
  if (status.value.ready === null) return { cls: 'n', text: '状态未知' }
  return status.value.ready ? { cls: 'g', text: '下载器已就绪' } : { cls: 'a', text: '未安装下载器' }
})

/** 服务端把 `[ingest] downloader` 存成一个**字符串**；自备路径时才是别的值。 */
function applyView(d) {
  const x = d?.download || {}
  const raw = (x.engine || 'yt-dlp').trim()
  const builtin = raw === '' || raw.toLowerCase() === 'yt-dlp' || raw.toLowerCase() === 'ytdlp'
  form.value.engine = builtin ? 'yt-dlp' : '__custom__'
  // 内置但配了 ytdlp_path：那是"覆盖二进制位置"，路径仍然要显示出来
  form.value.ytdlp_path = builtin ? x.ytdlp_path || '' : raw
  form.value.max_quality = x.max_quality || '1080p'
  form.value.rate_limit_kbps = x.rate_limit_kbps ?? 0
  form.value.max_concurrent_downloads = x.max_concurrent_downloads ?? 1
  form.value.cookie_file = x.cookie_file || ''
  form.value.idle_timeout_secs = x.idle_timeout_secs ?? 120
  // 存放位置在 `[storage]`（不属于 `[ingest]`）：下载与入库共用一个根
  const st = d?.storage || {}
  form.value.projects_root = st.projects_root || ''
  effectiveRoot.value = st.projects_root_effective || ''
}

async function loadStatus() {
  try {
    const d = await ingestApi.downloaderStatus()
    status.value = {
      ready: d?.ready === true,
      verified: d?.verified === true,
      kind: d?.kind || '',
      pinned_version: d?.pinned_version || '',
      license: d?.license || '',
      source: ''
    }
  } catch {
    status.value.ready = null
  }
}

async function load() {
  try {
    applyView(await ingestApi.config())
    await loadStatus()
    loaded.value = true
  } catch (e) {
    toast('下载配置读取失败：' + e.message, 'err')
  }
}

async function install() {
  installBusy.value = true
  try {
    const r = await ingestApi.installDownloader()
    await loadStatus()
    toast(r?.verified ? '下载器已安装并通过 sha256 校验' : '下载器已安装，但未通过托管校验', r?.verified ? 'ok' : 'err')
  } catch (e) {
    toast('安装失败：' + e.message, 'err')
  } finally {
    installBusy.value = false
  }
}

function onPicked(name) {
  if (pickTarget.value === 'cookie') form.value.cookie_file = name
  else if (pickTarget.value === 'dir') form.value.projects_root = name
  else form.value.ytdlp_path = name
  showPicker.value = false
}

async function save() {
  if (busy.value) return
  busy.value = true
  try {
    const custom = form.value.engine === '__custom__'
    const d = await ingestApi.saveConfig({
      download: {
        // 引擎名：选内置就写 "yt-dlp"（服务端据此走托管副本）；
        // 选自备时把路径直接当引擎名传 —— 服务端把它当"绝对路径"解析（见 DownloaderKind::parse）。
        engine: custom ? form.value.ytdlp_path : 'yt-dlp',
        ytdlp_path: custom ? '' : form.value.ytdlp_path,
        max_quality: form.value.max_quality,
        rate_limit_kbps: Number(form.value.rate_limit_kbps) || 0,
        max_concurrent_downloads: Number(form.value.max_concurrent_downloads) || 1,
        cookie_file: form.value.cookie_file,
        idle_timeout_secs: Number(form.value.idle_timeout_secs) || 120
      },
      // 存放位置走 `[storage]`（服务端会先真的把目录建出来，建不成会 400 并给原因）
      storage: {
        projects_root: form.value.projects_root
      }
    })
    applyView(d)
    warnings.value = d?.warnings || []
    await loadStatus()
    toast('下载配置已保存')
  } catch (e) {
    toast('保存失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
}

onMounted(load)
</script>

<style scoped>
.dl-box {
  padding:10px 12px; border:1px solid var(--line-soft); border-radius:8px;
  background:var(--bg-soft, transparent);
}
.grid2 { display:grid; grid-template-columns:repeat(auto-fit, minmax(260px, 1fr)); gap:12px 16px; }
.fld { display:flex; flex-direction:column; gap:4px; }
.lbl { font-size:12px; font-weight:600; }
.fld input, .fld select { width:100%; }
.warn-t { color:var(--warn, #d18b00); }
</style>
