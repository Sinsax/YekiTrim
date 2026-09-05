<template>
  <div>
    <div class="page-head">
      <div><h1>设置</h1><div class="sub">程序配置 / 工作区 / 模型 / LLM</div></div>
    </div>

    <div class="card">
      <h2>程序配置 .vtrim/config.toml</h2>
      <textarea v-model="globalRaw" rows="12" class="mono"></textarea>
      <div class="row" style="margin-top:10px"><button class="btn btn-primary" @click="saveGlobal">保存程序配置</button></div>
    </div>

    <div class="card">
      <h2>LLM 配置 .vtrim/llm.toml</h2>
      <textarea v-model="llmRaw" rows="10" class="mono"></textarea>
      <div class="row" style="margin-top:10px">
        <button class="btn btn-primary" @click="saveLlm">保存 LLM</button>
        <button class="btn" @click="testLlm">测试连接</button>
        <span class="muted small" id="llmResult">{{ llmResult }}</span>
      </div>
    </div>

    <div class="card">
      <h2>模型管理</h2>
      <div class="chips">
        <span v-for="m in models" :key="m.name" class="chip" :class="m.installed ? 'g' : 'n'">{{ m.name }} {{ m.installed ? '已装' : '未装' }}</span>
      </div>
      <button class="btn" style="margin-top:8px" @click="downloadModels">下载缺失模型</button>
    </div>

    <div class="card">
      <h2>工作区</h2>
      <div class="row">
        <div style="flex:1"><label>工作区根目录</label><input v-model="wsRoot"></div>
        <button class="btn btn-primary" @click="saveWs">保存</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { api } from '../api/client'
import { toast } from '../stores/toast'

const globalRaw = ref('')
const llmRaw = ref('')
const wsRoot = ref('')
const models = ref([])
const llmResult = ref('')

async function load() {
  try {
    const d = await api('/api/config')
    globalRaw.value = d.global_raw || ''
    llmRaw.value = d.llm_raw || ''
    wsRoot.value = d.root_dir || ''
  } catch (e) {}
  try { models.value = await api('/api/models') } catch (e) {}
}

async function saveGlobal() {
  try {
    await api('/api/config/global', { method: 'PUT', body: JSON.stringify({ raw: globalRaw.value }) })
    toast('程序配置已保存')
  } catch (e) { toast(e.message) }
}

async function saveLlm() {
  try {
    await api('/api/config/llm', { method: 'PUT', body: JSON.stringify({ raw: llmRaw.value }) })
    toast('LLM 配置已保存')
  } catch (e) { toast(e.message) }
}

async function testLlm() {
  llmResult.value = '测试中…'
  try {
    const r = await api('/api/llm/test', { method: 'POST' })
    llmResult.value = '连接成功 ' + r.latency_ms + 'ms'
  } catch (e) {
    llmResult.value = e.message
  }
}

async function downloadModels() {
  try {
    await api('/api/models/download', { method: 'POST' })
    toast('下载已启动')
  } catch (e) { toast(e.message) }
}

async function saveWs() {
  try {
    await api('/api/project/workspace', { method: 'POST', body: JSON.stringify({ root: wsRoot.value }) })
    toast('工作区已保存')
  } catch (e) { toast(e.message) }
}

onMounted(load)
</script>
