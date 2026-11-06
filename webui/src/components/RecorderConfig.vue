<template>
  <div>
    <!-- 总闸 + 状态 + 动作 -->
    <div class="row spread mb-8">
      <div class="row">
        <span class="chip" :class="stateChip.cls">{{ stateChip.text }}</span>
        <label class="row" style="gap:6px;cursor:pointer;margin:0">
          <input v-model="enabled" type="checkbox" style="width:auto" :disabled="busy || !loaded" @change="toggleEnabled">
          <span>启用采集（录播姬 / 通用 Webhook）</span>
        </label>
      </div>
      <div class="row">
        <button class="mini-btn primary" :disabled="busy || !loaded" @click="save">保存</button>
        <button class="mini-btn" :disabled="busy || !loaded || dirty || !recorderTestable"
                :title="dirty ? '有未保存修改，先保存再测试' : (!recorderTestable ? '录播姬未配置 API 地址；不用录播姬可忽略' : '只测试录播姬；读的是已保存的配置（改动先保存）')"
                @click="test">测试录播姬</button>
      </div>
    </div>

    <div class="hint mb-8" title="录播姬 / 通用 Webhook（biliLive-tools 等）录完一场推事件，YekiTrim 自动入库成项目。保存/切换立即生效，不需要重启。">
      录播姬 / 通用 Webhook 录完自动入库；改动立即生效
    </div>

    <div class="hint mb-8">
      录播姬：{{ recorderState }} · 通用 Webhook：{{ webhookState }}
      <span v-if="dirty" class="warn-text"> · 有未保存修改（测试/发布前先保存）</span>
    </div>

    <!-- 读失败**照实说 + 重试**：以前只 toast 一下，页面永远停在「配置读取中…」且保存禁用 -->
    <div v-if="loadError" class="error-banner">
      <Icon name="alert" :size="15" /> {{ loadError }}
      <button class="mini-btn flex-none" :disabled="busy" @click="load">重试</button>
    </div>
    <div v-else-if="!loaded" class="row muted small"><span class="spinner"></span> 配置读取中…</div>

    <template v-else>
      <!-- 关闭时只留开关与一句说明（用户要求：开启后才展开表单） -->
      <div v-if="!enabled" class="hint" title="兜底扫描也不跑；打开上面的开关即可填入连接信息">
        已关闭：webhook 一律拒收
      </div>

      <template v-else>
        <div class="form-section">
        <div class="form-section-title">录播姬（可选） · 连接</div>
        <div class="rc-grid">
          <div class="span-2">
            <label title="录播姬的输出目录，YekiTrim 进程要读得到">录像目录 root</label>
            <div class="row">
              <input v-model.trim="form.root" class="flex-1" placeholder="例如 /mnt/nas/rec 或 D:/rec">
              <button class="mini-btn" @click="showPicker = true">浏览…</button>
            </div>
          </div>

          <div>
            <label>API 地址 api_base</label>
            <input v-model.trim="form.api_base" placeholder="http://127.0.0.1:2356">
            <div class="hint" title="留空 = 未对接，用户不能自助加直播间">录播姬 WebUI 的地址</div>
          </div>
          <div>
            <label>用户名 api_user</label>
            <input v-model.trim="form.api_user" placeholder="留空 = 录播姬未设密码">
            <div class="hint">对应录播姬的 BREC_HTTP_BASIC_USER</div>
          </div>

          <div>
            <label>密码 api_pass</label>
            <div class="row">
              <input v-model="form.api_pass" class="flex-1" type="password" :placeholder="passPlaceholder">
              <button class="mini-btn" :disabled="!passSet || busy" @click="clearPass">清除</button>
            </div>
            <div class="hint">不回显已保存的密码；<b>留空 = 不改</b></div>
          </div>
        </div>
        </div>

        <div class="form-section">
        <div class="form-section-title">录播姬（可选） · 入库</div>
        <div class="rc-grid">
          <div>
            <label>来源白名单 allow_ips</label>
            <input v-model.trim="allowIpsText" placeholder="留空 = 不限来源">
            <div class="hint">逗号分隔，只支持<b>单个 IP</b>（不支持 <span class="mono">192.168.1.0/24</span> 网段）</div>
          </div>

          <div class="span-2">
            <label>入库后自动跑到</label>
            <select v-model="form.auto_pipeline">
              <option v-for="p in PIPELINES" :key="p.value" :value="p.value">{{ p.label }} —— {{ p.hint }}</option>
            </select>
          </div>
        </div>
        </div>

        <div class="form-section">
        <div class="form-section-title">通用 Webhook（biliLive-tools / 其它录制软件）</div>
        <div class="rc-grid">
          <div class="span-2">
            <label title="录制软件把文件写到的宿主机根目录；事件里的路径必须落在它下面">录像目录 root</label>
            <div class="row">
              <input v-model.trim="form.webhook_root" class="flex-1" placeholder="例如 /home/me/records">
              <button class="mini-btn" @click="showWebhookPicker = true">浏览…</button>
            </div>
          </div>
          <div>
            <label title="事件给的是容器内路径时填它；宿主机路径留空即可">容器路径前缀 container_root</label>
            <input v-model.trim="form.webhook_container_root" placeholder="例如 /app/video（留空 = 事件直接给宿主机路径）">
          </div>
          <div>
            <label>建项目后自动跑到</label>
            <select v-model="form.webhook_auto_pipeline">
              <option v-for="p in PIPELINES" :key="p.value" :value="p.value">{{ p.label }} —— {{ p.hint }}</option>
            </select>
            <div class="hint">默认 none：先建项目，自己决定要不要跑 ASR</div>
          </div>
        </div>
        <div class="hint mt-6" title="biliLive-tools：设置 → 事件订阅填下面的地址；它的 type=file_completed 会被识别。其它软件按 {event:'file_completed', file:'/path/to.mp4', danmaku?, room_id?} 发即可。">
          兼容 biliLive-tools 的「事件订阅」；其它软件发一条 <span class="mono">file_completed</span> 即可
        </div>
        <div class="hint mt-6">
          这是<b>入站</b>接口，没有「测试连接」；保存后用下面的地址从录制软件发一条事件即可。
        </div>
        </div>

        <div class="form-section">
        <div class="form-section-title">接收入口</div>

        <div v-if="warnings.length" class="hint mt-8">
          <div v-for="(w, i) in warnings" :key="i">⚠ {{ w }}</div>
        </div>

        <div class="rc-webhook">
          <!-- ⚠️ 只有**真的拿到 token 明文**（刚生成/重新生成那一次）才显示完整地址与「复制」。
               以前没 token 时也把 .../<token> 摆出来并给复制按钮 —— 复制出来的是个不可用的 URL
               （token 是占位符，不是真值）。有了地址就不再单独复制，避免两颗按钮做同一件事。 -->
          <template v-if="newToken">
            <label>录播姬里填的 Webhook 地址</label>
            <div class="row">
              <input class="mono flex-1" :value="webhookUrlNewToken" readonly @focus="$event.target.select()">
              <button class="mini-btn" @click="copy(webhookUrlNewToken)">复制</button>
            </div>
            <label class="mt-8">通用 Webhook 地址（biliLive-tools / 自定义脚本）</label>
            <div class="row">
              <input class="mono flex-1" :value="webhookUrlNewTokenGeneric" readonly @focus="$event.target.select()">
              <button class="mini-btn" @click="copy(webhookUrlNewTokenGeneric)">复制</button>
            </div>
            <div class="hint mt-4">地址里带 token，<b>只显示这一次</b>；离开本页就看不到，忘了就重新生成</div>
          </template>
          <div v-else class="hint" title="地址路径固定；token 明文只在生成时回一次（服务端只存哈希）">
            地址形如 <span class="mono">{{ webhookPathTemplate }}</span>
            （通用入口 <span class="mono">{{ webhookPathTemplateGeneric }}</span>）；
            {{ tokenConfigured ? 'token 已配置但明文不回显 —— 需要地址就重新生成' : '生成 token 后这里会显示完整地址' }}
          </div>
          <div v-if="isLocalhost && lanIp" class="hint mt-6">
            地址已自动使用局域网 IP <b class="mono">{{ lanIp }}</b>；Docker / 另一台机器可直接用。
          </div>
          <div v-else-if="isLocalhost" class="hint warn-text mt-6">
            当前页面是本机地址，且没探测到局域网 IP：Docker / 另一台机器请把 127.0.0.1 换成这台机器的局域网 IP。
          </div>
          <div class="row mt-6">
            <button class="mini-btn primary" :disabled="busy" @click="rotate">
              {{ tokenConfigured ? '重新生成 token' : '生成 token' }}
            </button>
            <span class="muted small">
              {{ tokenConfigured
                ? 'token 已配置；明文只在生成那一次显示'
                : '尚未生成 token：webhook 会拒绝所有请求' }}
            </span>
          </div>
          <div v-if="newToken" class="rc-token mt-8">
            <span class="small" title="请立刻填进录制软件的 Webhook 设置"><b>只显示这一次</b>，离开本页就看不到</span>
            <input class="mono" :value="newToken" readonly @focus="$event.target.select()">
          </div>
          <div class="hint mt-6" title="Docker/另一台机器用上面的局域网 IP；关开关保留 token；重新生成才会让旧的立刻失效。">
            地址已自动用<b>局域网 IP</b>；关开关保留 token
          </div>
        </div>

        <div v-if="testResult" class="hint mt-8">{{ testResult }}</div>
        <div class="hint mt-8">
          「测试录播姬」读的是<b>已保存</b>的配置（改完先保存再测）；通用 Webhook 是<b>入站</b>接口，不需要测试。
          <span class="mono">finalize_idle_mins</span>／<span class="mono">move_into_managed</span>／
          <span class="mono">concat_files</span>／<span class="mono">extensions</span>
          不在表单里，用「编辑源文件」改。
        </div>
        </div>
      </template>
    </template>

    <AppModal :show="showPicker" title="选择录像目录（录播姬的输出目录）" @close="showPicker = false">
      <ServerDirPicker mode="dir" :start-dir="form.root" @pick-dir="pickDir" />
      <template #footer>
        <button class="btn" @click="showPicker = false">关闭</button>
      </template>
    </AppModal>

    <AppModal :show="showWebhookPicker" title="选择录像目录（通用 Webhook 的宿主机根目录）" @close="showWebhookPicker = false">
      <ServerDirPicker mode="dir" :start-dir="form.webhook_root" @pick-dir="pickWebhookDir" />
      <template #footer>
        <button class="btn" @click="showWebhookPicker = false">关闭</button>
      </template>
    </AppModal>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api/client'
import { ingestApi, PIPELINES } from '../api/ingest'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import AppModal from './AppModal.vue'
import Icon from './Icon.vue'
import ServerDirPicker from './ServerDirPicker.vue'

const loaded = ref(false)
/** 配置读失败（与「正在读」分开：以前两者都渲染成「配置读取中…」） */
const loadError = ref('')
const busy = ref(false)
const enabled = ref(false)
/** 服务端算的通用 Webhook 就绪位（webhook.enabled = 开关 && token && webhook.root） */
const webhookEnabled = ref(false)
const tokenConfigured = ref(false)
const passSet = ref(false)
const allowIpsText = ref('')
const warnings = ref([])
const newToken = ref('')
const testResult = ref('')
const showPicker = ref(false)
const showWebhookPicker = ref(false)
const lanIp = ref('')
const savedSnapshot = ref('')

// `api_pass` 用 `null` 表示"用户没碰过" → 保存时**不发这个键**。
// 发空串等于"清空密码"，所以两者必须在数据上区分开（服务端语义也按这个来）。
const form = ref({
  root: '', api_base: '', api_user: '', api_pass: null, auto_pipeline: 'detect',
  webhook_root: '', webhook_container_root: '', webhook_auto_pipeline: 'none'
})

const passPlaceholder = computed(() => (passSet.value ? '已设置，留空不改' : '未设置'))
/** 本机页面（127.0.0.1）不能当 Docker/其它机器的回调地址；有 lan_ip 就自动换掉。 */
const isLocalhost = computed(() => ['127.0.0.1', 'localhost', '::1', '[::1]'].includes(location.hostname))
const webOrigin = computed(() => {
  if (isLocalhost.value && lanIp.value) {
    const port = location.port ? ':' + location.port : ''
    return (location.protocol === 'https:' ? 'https://' : 'http://') + lanIp.value + port
  }
  return location.origin
})
/** 没拿到 token 明文时只给**路径模板**（<token> 是占位符，不提供复制 —— 复制它没用）。 */
const webhookPathTemplate = computed(() => webOrigin.value + '/api/ingest/bililive/<token>')
const webhookPathTemplateGeneric = computed(() => webOrigin.value + '/api/ingest/webhook/<token>')
const webhookUrlNewToken = computed(() => webOrigin.value + '/api/ingest/bililive/' + newToken.value)
const webhookUrlNewTokenGeneric = computed(() => webOrigin.value + '/api/ingest/webhook/' + newToken.value)

/** 两条通道各自的状态（顶部那行，不再用一句「采集已就绪」糊过去）。 */
const recorderState = computed(() => {
  if (!form.value.root) return '未配置'
  return form.value.api_base ? '已配置' : '已填目录 · 未填 API'
})
const webhookState = computed(() => {
  if (!form.value.webhook_root) return '未配置'
  return tokenConfigured.value ? '已配置' : '已填目录 · 缺 token'
})
/** 「测试录播姬」只有在填了 API 地址时才有意义；只配通用 Webhook 的用户别点出一个必然失败。 */
const recorderTestable = computed(() => !!form.value.api_base)
/** 保存快照：用来提示「有未保存修改」，避免用户直接点测试/发布时读到旧配置。 */
function snapshot() {
  return JSON.stringify({
    root: form.value.root,
    api_base: form.value.api_base,
    api_user: form.value.api_user,
    api_pass: form.value.api_pass,
    auto_pipeline: form.value.auto_pipeline,
    webhook_root: form.value.webhook_root,
    webhook_container_root: form.value.webhook_container_root,
    webhook_auto_pipeline: form.value.webhook_auto_pipeline
  })
}
const dirty = computed(() => !!savedSnapshot.value && snapshot() !== savedSnapshot.value)

/** 状态 chip：**按通道分开说**。
 *  · 录播姬 = 开关开着且有 recorder.root（与服务端 /api/health.ingest_ready 同判据）；
 *  · 通用 Webhook = 服务端算的 webhook.enabled（还要 token）。
 *  ⚠️ 以前写成「root 或 webhook.root 任一」就报「采集已就绪」：只配了通用 Webhook 时这里说就绪，
 *  而首页 / 入库页按 ingest_ready 说「录播姬未接入」，同一屏两个结论（用户截图实测）。 */
const stateChip = computed(() => {
  if (!enabled.value) return { cls: 'n', text: '采集已关闭' }
  // 没 token 时两个入口都收不了：别再说「已就绪」
  if (!tokenConfigured.value) return { cls: 'a', text: '已启用 · 缺 token' }
  // 表单有没保存的改动时，服务端仍按旧配置工作 —— 别用「已就绪」掩盖这一点
  if (dirty.value) return { cls: 'a', text: '采集已启用 · 有未保存修改' }
  if (form.value.root || webhookEnabled.value) return { cls: 'g', text: '采集已启用' }
  return { cls: 'a', text: '已启用 · 缺录像目录' }
})

/** 用服务端返回的视图刷新表单（**只在加载与保存后调**：切换开关时调会把没保存的输入冲掉）。 */
function applyView(d) {
  enabled.value = d?.webhook_enabled === true
  tokenConfigured.value = d?.token_configured === true
  const r = d?.recorder || {}
  form.value.root = r.root || ''
  form.value.api_base = r.api_base || ''
  form.value.api_user = r.api_user || ''
  form.value.api_pass = null // 不回显
  form.value.auto_pipeline = r.auto_pipeline || 'detect'
  const w = d?.webhook || {}
  form.value.webhook_root = w.root || ''
  form.value.webhook_container_root = w.container_root || ''
  form.value.webhook_auto_pipeline = w.auto_pipeline || 'none'
  webhookEnabled.value = w.enabled === true
  passSet.value = r.api_pass_set === true
  allowIpsText.value = (d?.allow_ips || []).join(', ')
  lanIp.value = d?.lan_ip || ''
  savedSnapshot.value = snapshot()
}

async function load() {
  loadError.value = ''
  try {
    applyView(await ingestApi.config())
    loaded.value = true
  } catch (e) {
    loadError.value = '采集配置读取失败：' + (e?.message || '')
  }
}

async function toggleEnabled() {
  const next = enabled.value
  busy.value = true
  try {
    const d = await ingestApi.saveConfig({ webhook_enabled: next })
    enabled.value = d?.webhook_enabled === true
    tokenConfigured.value = d?.token_configured === true
    // 只更新服务端算的通道位（**不 applyView**：那会把用户没保存的输入冲掉）
    webhookEnabled.value = d?.webhook?.enabled === true
    toast(next ? '已开启采集（webhook 开始接收）' : '已关闭采集（token 保留）')
  } catch (e) {
    enabled.value = !next // 失败回滚开关，别让界面显示成已生效
    toast('切换失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
}

/** 白名单输入：逗号/空白分隔（服务端还会再校验一次必须是 IP 字面量）。 */
function parseIps(text) {
  return String(text || '')
    .split(/[,\s]+/)
    .map((s) => s.trim())
    .filter(Boolean)
}

async function save() {
  if (busy.value) return
  busy.value = true
  try {
    const recorder = {
      root: form.value.root,
      api_base: form.value.api_base,
      api_user: form.value.api_user,
      auto_pipeline: form.value.auto_pipeline
    }
    if (form.value.api_pass !== null) recorder.api_pass = form.value.api_pass
    const webhook = {
      root: form.value.webhook_root,
      container_root: form.value.webhook_container_root,
      auto_pipeline: form.value.webhook_auto_pipeline
    }
    const d = await ingestApi.saveConfig({ allow_ips: parseIps(allowIpsText.value), recorder, webhook })
    applyView(d)
    warnings.value = d?.warnings || []
    toast('采集配置已保存')
  } catch (e) {
    toast('保存失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
}

function clearPass() {
  form.value.api_pass = ''
  passSet.value = false
  warnings.value = []
  toast('已标记清除密码，记得点「保存」')
}

async function rotate() {
  if (tokenConfigured.value) {
    const ok = await confirmDialog(
      '重新生成会让旧 token 立刻失效：录制软件那边的 Webhook 地址要一起改成新的。继续？',
      { title: '重新生成 token？', danger: true }
    )
    if (!ok) return
  }
  busy.value = true
  try {
    const d = await ingestApi.rotateToken('generate')
    newToken.value = d?.token || ''
    tokenConfigured.value = d?.token_configured === true
    enabled.value = d?.webhook_enabled === true
    webhookEnabled.value = d?.webhook?.enabled === true
    toast('已生成新 token（明文只显示这一次）')
  } catch (e) {
    toast('生成失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
}

async function test() {
  testResult.value = '测试中…'
  try {
    const d = await api('/api/admin/recorder/test', { method: 'POST', body: '{}' })
    testResult.value = `连接成功：${d?.base || ''}${d?.version ? ' · v' + d.version : ''} · ${d?.rooms ?? 0} 个房间`
    toast('录播姬连接正常')
  } catch (e) {
    // 未配置时服务端回 `{ok:false}`，`api()` 会抛 —— 文案就是原因（如"未配置 api_base"）
    testResult.value = '连接失败：' + e.message
    toast('连接失败', 'err')
  }
}

function pickDir(dir) {
  form.value.root = dir || ''
  showPicker.value = false
}

function pickWebhookDir(dir) {
  form.value.webhook_root = dir || ''
  showWebhookPicker.value = false
}

async function copy(text) {
  try {
    await navigator.clipboard.writeText(text)
    toast('已复制')
  } catch {
    toast(text)
  }
}

onMounted(load)
</script>

<style scoped>
/* 这一卡专属的两列布局（基准类里没有）；窄屏回落单列 */
.rc-grid { display:grid; grid-template-columns:1fr 1fr; gap:10px 14px; }
.rc-grid .span-2 { grid-column:1 / -1; }
@media (max-width:900px) {
  .rc-grid { grid-template-columns:1fr; }
  .rc-grid .span-2 { grid-column:auto; }
}
/* webhook 区：与上面的连接信息隔开一层 */
.rc-webhook { border-top:1px dashed var(--line-soft); padding-top:10px; }
.rc-token { background:var(--bg-soft); border-radius:var(--r-sm); padding:8px 10px; }
.rc-token input { margin-top:6px; }
</style>
