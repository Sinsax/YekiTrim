<template>
  <div>
    <div class="row spread mb-8">
      <div class="row" style="gap:8px">
        <span class="chip" :class="stateChip.cls">{{ stateChip.text }}</span>
        <span class="muted small">录播软件二选一：只接一个</span>
      </div>
      <button class="mini-btn" :disabled="busy || !loaded" @click="save">保存</button>
    </div>

    <div v-if="loadError" class="error-banner">
      <Icon name="alert" :size="15" /> {{ loadError }}
      <button class="mini-btn flex-none" :disabled="busy" @click="load">重试</button>
    </div>
    <div v-else-if="!loaded" class="row muted small"><span class="spinner"></span> 读取中…</div>

    <div v-else class="rc-form">
      <div v-if="warnings.length" class="span2 warn-banner">
        <Icon name="alert" :size="15" />
        <div class="warn-body">
          <div v-for="(w, i) in warnings" :key="i">{{ w }}</div>
        </div>
      </div>
      <div class="span2">
        <label>录播软件</label>
        <select v-model="form.kind" @change="onKindChange">
          <option value="biliLiveTools">biliLive-tools</option>
          <option value="bililive">录播姬（BililiveRecorder）</option>
        </select>
      </div>

      <div>
        <label>{{ isBlt ? '录像根目录' : '录像目录' }}</label>
        <div class="row">
          <input v-model.trim="form.root" class="flex-1" placeholder="/home/me/video">
          <button class="mini-btn" @click="showPicker = true">浏览…</button>
        </div>
        <div class="hint">
          本机能打开的路径；跨机填共享地址
          <span class="mono" title="例：\\192.168.1.20\rec —— 不要填对方机器上的 /rec，也不要用映射盘符（服务进程常看不到）">\\录播机\共享</span>
        </div>
      </div>

      <div>
        <label>{{ isBlt ? 'API 地址' : '录播姬地址' }}</label>
        <input v-model.trim="form.api_base"
               :placeholder="isBlt ? 'http://127.0.0.1:18010' : 'http://127.0.0.1:2356'">
      </div>

      <div v-if="isBlt">
        <label>访问密钥</label>
        <input v-model="form.api_key" type="password" name="blt-api-key" autocomplete="new-password"
               :placeholder="apiKeyPlaceholder">
      </div>

      <div class="row mt-8">
        <button class="mini-btn" :disabled="busy || !form.api_base" @click="test">测试连接</button>
        <span v-if="!testResult" class="muted small">接口与录像目录各测一次</span>
      </div>
      <div v-if="testResult" class="test-out">
        <div class="test-line" :class="testResult.apiOk ? 'ok' : 'bad'">
          接口：{{ testResult.apiText }}
        </div>
        <div class="test-line" :class="testResult.rootOk ? 'ok' : 'bad'">
          录像目录：{{ testResult.rootText }}
        </div>
        <div v-if="testResult.mappingText" class="test-line" :class="testResult.mappingOk ? 'ok' : 'bad'">
          路径映射：{{ testResult.mappingText }}
          <button v-if="testResult.mappingFill" class="mini-btn" style="margin-left:6px" @click="useMapping">填入</button>
        </div>
        <div v-if="testResult.rootSuggest" class="test-line bad">
          建议录像目录改成 {{ testResult.rootSuggest.path }}
          <button class="mini-btn" style="margin-left:6px" @click="useSuggestedRoot">改用</button>
        </div>
        <div v-if="testResult.hint" class="hint">{{ testResult.hint }}</div>
      </div>

      <details class="rc-adv span2 mt-8">
        <summary>高级</summary>
        <div class="rc-form mt-8">
          <div v-if="!isBlt">
            <label>用户名</label>
            <input v-model.trim="form.api_user" placeholder="留空 = 未设密码">
          </div>
          <div v-if="!isBlt">
            <label>密码</label>
            <div class="row">
              <input v-model="form.api_pass" class="flex-1" type="password" :placeholder="passPlaceholder">
              <button class="mini-btn" :disabled="!passSet || busy" @click="clearPass">清除</button>
            </div>
          </div>
          <div v-if="isBlt">
            <label>容器路径前缀（留空 = 自动识别）</label>
            <input v-model.trim="form.container_root" placeholder="留空 = 自动识别">
          </div>
        </div>
      </details>
    </div>

    <AppModal :show="showPicker" title="选择录像目录" @close="showPicker = false">
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
import { ingestApi } from '../api/ingest'
import { toast } from '../stores/toast'
import AppModal from './AppModal.vue'
import Icon from './Icon.vue'
import ServerDirPicker from './ServerDirPicker.vue'

const loaded = ref(false)
const loadError = ref('')
const busy = ref(false)
const passSet = ref(false)
const apiKeySet = ref(false)
/** 测试连接的结果：接口与录像目录**分开报**（旧版只显示「连接成功：N 个录制对象」，目录死了也报成功） */
const testResult = ref(null)
/** 保存时后端算出的 warnings（目录不可达 / 只读 / 空目录…）—— 以前收到就丢了 */
const warnings = ref([])
/** 上次目录校验结论（`/api/admin/ingest` 的 `recorder.root_status`，null = 还没测过） */
const rootStatus = ref(null)
const showPicker = ref(false)
const form = ref({
  kind: 'biliLiveTools',
  root: '', api_base: '', api_user: '', api_pass: null, api_key: null, container_root: ''
})

const isBlt = computed(() => form.value.kind === 'biliLiveTools')
const passPlaceholder = computed(() => (passSet.value ? '已设置，留空不改' : '未设置'))
const apiKeyPlaceholder = computed(() => (apiKeySet.value ? '已设置（留空 = 不修改）' : '未设置'))
const stateChip = computed(() => {
  if (!loaded.value && !loadError.value) return { cls: 'n', text: '读取中' }
  if (loadError.value) return { cls: 'n', text: '读取失败' }
  if (!form.value.api_base || !form.value.root) return { cls: 'n', text: '未配置' }
  // 目录不可用不再显示成「已配置」：那正是跨机报障时最容易误导人的一档
  if (rootStatus.value && rootStatus.value.ok === false) return { cls: 'n', text: '目录不可用' }
  return { cls: 'g', text: '已配置' }
})

function onKindChange() {
  if (isBlt.value && (!form.value.api_base || form.value.api_base.includes(':2356'))) {
    form.value.api_base = 'http://127.0.0.1:18010'
  } else if (!isBlt.value && (!form.value.api_base || form.value.api_base.includes(':18010'))) {
    form.value.api_base = 'http://127.0.0.1:2356'
  }
}

function applyView(d) {
  const src = d?.recorder || {}
  form.value.kind = String(src.kind || '').toLowerCase() === 'bililive-tools' ? 'biliLiveTools' : 'bililive'
  form.value.root = src.root || ''
  form.value.api_base = src.api_base || ''
  form.value.api_user = src.api_user || ''
  form.value.container_root = src.container_root || ''
  form.value.api_pass = null
  form.value.api_key = null
  rootStatus.value = src.root_status || null
  passSet.value = src.api_pass_set === true
  apiKeySet.value = src.api_key_set === true
}

async function load() {
  loadError.value = ''
  try {
    applyView(await ingestApi.config())
    loaded.value = true
  } catch (e) {
    loadError.value = '配置读取失败：' + (e?.message || '')
  }
}

async function save() {
  if (busy.value) return
  busy.value = true
  try {
    const recorder = {
      kind: isBlt.value ? 'bililive-tools' : 'bililive',
      root: form.value.root,
      api_base: form.value.api_base
    }
    if (isBlt.value) {
      // 留空 = 交给后端自动识别（`ingest/config.rs::fill_container_root`）。
      // ⚠️ 这里**不能**发空串：空串在 PATCH 语义里是"清空"，会把自动补齐挡掉。
      if (form.value.container_root) recorder.container_root = form.value.container_root
      if (form.value.api_key !== null) recorder.api_key = form.value.api_key
    } else {
      recorder.api_user = form.value.api_user
      if (form.value.api_pass !== null) recorder.api_pass = form.value.api_pass
    }
    const d = await ingestApi.saveConfig({ recorder })
    applyView(d)
    // 后端已经算出「目录不可达 / 只读 / 空目录」这些结论，别丢：保存成功不等于能入库
    warnings.value = d?.warnings || []
    toast(warnings.value.length ? '已保存，但有需要注意的地方' : '已保存')
  } catch (e) {
    toast('保存失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
}

/** 目录校验结论 → 一句人话（状态码由后端给，**前端不自己判路径**）。 */
const ROOT_STATUS_TEXT = {
  not_configured: '未填写',
  foreign_unix_path: '填的是录播机上的路径，本机读不到',
  relative: '必须是绝对路径',
  missing: '打不开（不存在 / 共享没挂上）',
  not_a_directory: '这是文件，不是目录',
  denied: '没有权限',
  not_writable: '可读但不可写',
  ok: '可读写',
}
function rootStatusText(root) {
  if (!root || !root.status) return '未测'
  const base = ROOT_STATUS_TEXT[root.status] || root.status
  if (root.ok && root.entries != null) return `${base}（${root.entries} 项）`
  return base
}

async function test() {
  testResult.value = { apiOk: false, rootOk: false, apiText: '测试中…', rootText: '测试中…', hint: '' }
  try {
    // ⚠️ 把**当前输入**一起送过去（2026-11-24 起）：只测已存配置的话，
    // 「改完密钥 → 点测试 → 报 401」实际验的是旧值，锅会被甩给"网络/对方服务"
    // （实测踩过：biliLive-tools 的 passKey 被 BILILIVE_TOOLS_PASSKEY 覆盖）。
    // `root` 同理：跨机换共享路径时，先测再存才不冤枉人。
    const payload = {
      kind: isBlt.value ? 'bililive-tools' : 'bililive',
      api_base: form.value.api_base
    }
    if (form.value.root) payload.root = form.value.root
    if (isBlt.value) {
      if (form.value.api_key) payload.api_key = form.value.api_key
    } else {
      if (form.value.api_user) payload.api_user = form.value.api_user
      if (form.value.api_pass) payload.api_pass = form.value.api_pass
    }
    const d = await api('/api/admin/recorder/test', { method: 'POST', body: JSON.stringify(payload) })
    // ③ 路径映射（仅 biliLive-tools）：后端会自动推导 container_root，这里只是**建议**。
    const map = d?.mapping
    let mappingText = ''
    let mappingOk = false
    let mappingFill = ''
    if (map && (map.save_path || map.container_root)) {
      if (map.ok) {
        mappingOk = true
        mappingText = `${map.container_root}（${map.matched}/${map.probed} 条对得上）`
        // 与当前表单不一致时才给「填入」——别让按钮点了没变化
        if ((form.value.container_root || '') !== map.container_root) mappingFill = map.container_root
      } else {
        mappingText = map.probed
          ? `${map.probed} 条历史都映射不到当前目录 —— 录像目录可能选到了房间子目录`
          : '拿不到历史样本，无法自动识别（先确认接口与密钥）'
      }
    }
    const sug = d?.root_suggestion
    testResult.value = {
      apiOk,
      rootOk,
      apiText: apiOk
        ? `${d?.rooms ?? 0} 个录制对象${d?.version ? ' · v' + d.version : ''}`
        : (d?.api?.error || '连不上'),
      rootText: rootStatusText(d?.root),
      hint: rootOk ? '' : (d?.root?.hint || ''),
      mappingText,
      mappingOk,
      mappingFill,
      rootSuggest: sug && sug.ok ? sug : null,
    }
    // 测试结果同时更新「已配置 / 目录不可用」那枚 chip，不用等保存
    if (d?.root) rootStatus.value = d.root
    if (apiOk && rootOk) toast('连接正常')
    else if (!apiOk) toast('接口连不上', 'err')
  } catch (e) {
    testResult.value = { apiOk: false, rootOk: false, apiText: e.message || '请求失败', rootText: '未测', hint: '' }
    toast('连接失败', 'err')
  }
}


/** 「填入」自动识别的路径映射（只填表单，仍需点保存才落盘）。 */
function useMapping() {
  const v = testResult.value?.mappingFill
  if (!v) return
  form.value.container_root = v
  toast('已填入自动识别的路径映射，记得保存')
}

/** 「改用」建议的录像目录（root 选深了一层时）。 */
function useSuggestedRoot() {
  const p = testResult.value?.rootSuggest?.path
  if (!p) return
  form.value.root = p
  toast('已改用建议的录像目录，记得保存后重新测试')
}

function clearPass() {
  form.value.api_pass = ''
  passSet.value = false
  toast('已标记清除密码，记得点「保存」')
}
function pickDir(dir) {
  form.value.root = dir || ''
  showPicker.value = false
}

onMounted(load)
</script>

<style scoped>
.rc-form { display:grid; grid-template-columns:1fr 1fr; gap:10px 14px; }
.rc-form .span2 { grid-column:1 / -1; }
.rc-form > div { min-width:0; }
@media (max-width:900px) { .rc-form { grid-template-columns:1fr; } }
.rc-adv summary { cursor:pointer; font-size:12.5px; color:var(--ink-dim); }
.test-out { grid-column:1 / -1; display:flex; flex-direction:column; gap:4px; margin-top:6px; }
.test-line { font-size:12.5px; color:var(--ink-dim); }
.test-line.ok { color:var(--green-deep); }
.test-line.bad { color:var(--pink-text); }
.warn-banner { display:flex; gap:8px; align-items:flex-start; padding:8px 10px; border-radius:8px; background:var(--bg-soft); color:var(--ink); font-size:12.5px; }
.warn-body { display:flex; flex-direction:column; gap:4px; min-width:0; }
.rc-adv summary:hover { color:var(--pink-text); }
</style>
