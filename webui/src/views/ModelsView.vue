<template>
  <div>
    <div class="page-head">
      <div><h1>模型</h1><div class="sub">ASR 模型管理</div></div>
      <div class="actions"><button class="btn" @click="downloadAll">下载全部</button></div>
    </div>
    <div class="grid cols-3">
      <div v-for="m in models" :key="m.name" class="pcard">
        <div class="head"><div class="sticker mint">🎙</div><div><h3>{{ m.name }}</h3><div class="path">{{ m.size_label }}</div></div></div>
        <div class="chips"><span class="chip" :class="m.installed ? 'g' : 'n'">{{ m.installed ? '已安装' : '未安装' }}</span></div>
        <div class="foot"><span class="name"></span><button class="mini-btn" @click="downloadAll">下载</button></div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { api } from '../api/client'
import { toast } from '../stores/toast'
const models = ref([])
async function load() {
  try { models.value = await api('/api/models') } catch (e) {}
}
async function downloadAll() {
  try {
    await api('/api/models/download', { method: 'POST' })
    toast('下载已启动')
  } catch (e) { toast(e.message) }
}
onMounted(load)
</script>
