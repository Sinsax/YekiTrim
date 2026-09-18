<template>
  <div>
    <!-- 总闸 + 状态 + 动作 -->
    <div class="row spread mb-8" style="gap:8px">
      <div class="row" style="gap:10px">
        <span class="chip" :class="stateChip.cls">{{ stateChip.text }}</span>
        <label class="row" style="gap:6px;cursor:pointer;margin:0">
          <input v-model="enabled" type="checkbox" style="width:auto" :disabled="busy || !loaded" @change="toggleEnabled">
          <span>启用录播姬采集</span>
        </label>
      </div>
      <div class="row" style="gap:6px">
        <button class="mini-btn primary" :disabled="busy || !loaded" @click="save">保存</button>
        <button class="mini-btn" :disabled="busy || !loaded" @click="test">测试连接</button>
      </div>
    </div>

    <div class="hint mb-8">
      录播姬（BililiveRecorder）录完一场推 webhook 给 V-Trim，V-Trim 按房间归属自动入库成项目。
      「启用」= 打开接收入口 <span class="mono">/api/ingest/bililive/&lt;token&gt;</span>；
      <b>保存/切换立即生效，不需要重启</b>。
    </div>

    <div v-if="!loaded" class="muted small">配置读取中…</div>

    <template v-else>
      <!-- 关闭时只留开关与一句说明（用户要求：开启后才展开表单） -->
      <div v-if="!enabled" class="hint">
        当前已关闭：webhook 一律拒收，兜底扫描也不跑。打开上面的开关即可填入连接信息。
      </div>

      <template v-else>
        <div class="rc-grid">
          <div class="span-2">
            <label>录像目录 root（录播姬的输出目录，V-Trim 进程要读得到）</label>
            <div class="row" style="gap:8px">
              <input v-model.trim="form.root" class="flex-1" placeholder="例如 D:/rec 或 \\NAS\rec">
              <button class="mini-btn" @click="showPicker = true">浏览…</button>
            </div>
          </div>

          <div>
            <label>API 地址 api_base</label>
            <input v-model.trim="form.api_base" placeholder="http://127.0.0.1:2356">
            <div class="hint">录播姬 WebUI 的地址；留空 = 未对接（用户不能自助加直播间）</div>
          </div>
          <div>
            <label>用户名 api_user</label>
            <input v-model.trim="form.api_user" placeholder="留空 = 录播姬未设密码">
            <div class="hint">对应录播姬的 BREC_HTTP_BASIC_USER</div>
          </div>

          <div>
            <label>密码 api_pass</label>
            <div class="row" style="gap:6px">
              <input v-model="form.api_pass" class="flex-1" type="password" :placeholder="passPlaceholder">
              <button class="mini-btn" :disabled="!passSet || busy" @click="clearPass">清除</button>
            </div>
            <div class="hint">不回显已保存的密码；<b>留空 = 不改</b></div>
          </div>
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

        <div v-if="warnings.length" class="hint mt-8">
          <div v-for="(w, i) in warnings" :key="i">⚠ {{ w }}</div>
        </div>

        <div class="rc-webhook mt-12">
          <label>录播姬里填的 Webhook 地址</label>
          <div class="row" style="gap:8px">
            <input class="mono flex-1" :value="webhookUrl" readonly @focus="$event.target.select()">
            <button class="mini-btn" @click="copy(webhookUrl)">复制</button>
          </div>
          <div class="row mt-6" style="gap:8px">
            <button class="mini-btn primary" :disabled="busy" @click="rotate">
              {{ tokenConfigured ? '重新生成 token' : '生成 token' }}
            </button>
            <span class="muted small">
              {{ tokenConfigured
                ? 'token 已配置；明文只在生成那一次显示，忘了就重新生成'
                : '尚未生成 token：webhook 会拒绝所有请求' }}
            </span>
          </div>
          <div v-if="newToken" class="rc-token mt-8">
            <div class="row spread" style="gap:8px">
              <span class="small"><b>只显示这一次</b>：请立刻填进录播姬的 Webhook 设置，离开本页就看不到了</span>
              <button class="mini-btn" @click="copy(webhookUrlNewToken)">复制完整地址</button>
            </div>
            <input class="mono" :value="newToken" readonly @focus="$event.target.select()">
          </div>
          <div class="hint mt-6">
            录播姬不在本机时，把地址里的主机名换成 V-Trim 主机的 <b>局域网 IP</b>（录播姬要能访问到）。
            关掉上面的开关会<b>保留</b> token；重新生成才会让旧的立刻失效。
          </div>
        </div>

        <div v-if="testResult" class="hint mt-8">{{ testResult }}</div>
        <div class="hint mt-8">
          测试连接读的是<b>已保存</b>的配置（改完先保存再测）。
          <span class="mono">finalize_idle_mins</span>／<span class="mono">move_into_managed</span>／
          <span class="mono">concat_files</span>／<span class="mono">extensions</span>
          不在表单里，需要时用上面的「编辑源文件」改。
        </div>
      </template>
    </template>

    <AppModal :show="showPicker" title="选择录像目录（录播姬的输出目录）" @close="showPicker = false">
      <ServerDirPicker mode="dir" :start-dir="form.root" @pick-dir="pickDir" />
      <template #footer>
        <button class="btn" @click="showPicker = false">关闭</button>
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
import ServerDirPicker from './ServerDirPicker.vue'

const loaded = ref(false)
const busy = ref(false)
const enabled = ref(false)
const tokenConfigured = ref(false)
const passSet = ref(false)
const allowIpsText = ref('')
const warnings = ref([])
const newToken = ref('')
const testResult = ref('')
const showPicker = ref(false)

// `api_pass` 用 `null` 表示"用户没碰过" → 保存时**不发这个键**。
// 发空串等于"清空密码"，所以两者必须在数据上区分开（服务端语义也按这个来）。
const form = ref({ root: '', api_base: '', api_user: '', api_pass: null, auto_pipeline: 'detect' })

const passPlaceholder = computed(() => (passSet.value ? '已设置，留空不改' : '未设置'))
const webhookUrl = computed(
  () => location.origin + '/api/ingest/bililive/' + (newToken.value || '<token>')
)
const webhookUrlNewToken = computed(() => location.origin + '/api/ingest/bililive/' + newToken.value)

/** 状态 chip：就绪 = 开关开着 **且** 有录像目录（与服务端 `/api/health.ingest_ready` 同判据）。 */
const stateChip = computed(() => {
  if (!enabled.value) return { cls: 'n', text: '采集已关闭' }
  return form.value.root
    ? { cls: 'g', text: '采集已就绪' }
    : { cls: 'a', text: '已启用 · 缺录像目录' }
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
  passSet.value = r.api_pass_set === true
  allowIpsText.value = (d?.allow_ips || []).join(', ')
}

async function load() {
  try {
    applyView(await ingestApi.config())
    loaded.value = true
  } catch (e) {
    toast('采集配置读取失败：' + e.message, 'err')
  }
}

async function toggleEnabled() {
  const next = enabled.value
  busy.value = true
  try {
    const d = await ingestApi.saveConfig({ webhook_enabled: next })
    enabled.value = d?.webhook_enabled === true
    tokenConfigured.value = d?.token_configured === true
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
    const d = await ingestApi.saveConfig({ allow_ips: parseIps(allowIpsText.value), recorder })
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
      '重新生成会让**旧 token 立刻失效**：录播姬那边的 Webhook 地址要一起改成新的。继续？',
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
