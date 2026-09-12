<template>
  <div>
    <div class="page-head">
      <div><h1>模型</h1><div class="sub">ASR 模型管理</div></div>
    </div>
    <div class="card">
      <div class="model-row" v-for="m in models" :key="m.name">
        <div>
          <div class="model-name">{{ m.name }}</div>
          <div class="muted small">{{ m.size_label }} · {{ m.note }}</div>
        </div>
        <div class="model-action">
          <button v-if="!m.installed" class="btn btn-primary" @click="downloadOne(m)">下载</button>
          <span v-else class="chip g">已安装</span>
        </div>
      </div>
      <div v-if="!models.length" class="muted">暂无模型</div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { api } from '../api/client'
import { toast } from '../stores/toast'

const models = ref([])

async function load() {
  try {
    const list = await api('/api/models')
    models.value = (list || []).filter(m => m.name !== 'firered-zh')
  } catch (e) {}
}

async function downloadOne(m) {
  try {
    await api('/api/models/download', { method: 'POST' })
    toast('全部 ASR 模型下载已启动（约 1.5GB）')
  } catch (e) { toast(e.message, 'err') }
}

onMounted(load)
</script>

<style scoped>
.model-row { display:flex; align-items:center; justify-content:space-between; padding:12px 4px; border-bottom:1px dashed var(--line-soft); }
.model-row:last-child { border-bottom:none; }
.model-name { font-weight:700; font-size:15px; }
.model-action { flex:none; }
</style>
