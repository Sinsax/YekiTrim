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
        <div class="hint">要可读可写：项目就建在录像目录里</div>
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
        <span v-if="testResult" class="muted small">{{ testResult }}</span>
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
            <label>容器路径前缀</label>
            <input v-model.trim="form.container_root" placeholder="/app/video">
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
const testResult = ref('')
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
  return form.value.api_base && form.value.root ? { cls: 'g', text: '已配置' } : { cls: 'n', text: '未配置' }
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
      recorder.container_root = form.value.container_root
      if (form.value.api_key !== null) recorder.api_key = form.value.api_key
    } else {
      recorder.api_user = form.value.api_user
      if (form.value.api_pass !== null) recorder.api_pass = form.value.api_pass
    }
    const d = await ingestApi.saveConfig({ recorder })
    applyView(d)
    toast('已保存')
  } catch (e) {
    toast('保存失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
}

async function test() {
  testResult.value = '测试中…'
  try {
    const d = await api('/api/admin/recorder/test', { method: 'POST', body: '{}' })
    testResult.value = `连接成功：${d?.rooms ?? 0} 个录制对象`
    toast('连接正常')
  } catch (e) {
    testResult.value = '连接失败：' + e.message
    toast('连接失败', 'err')
  }
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
.rc-adv summary:hover { color:var(--pink-text); }
</style>
