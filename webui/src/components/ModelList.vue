<template>
  <div>
    <div class="model-row" v-for="m in models" :key="m.name">
      <div>
        <div class="model-name">{{ m.name }}</div>
        <div class="muted small">{{ m.size_label }} · {{ m.note }}</div>
      </div>
      <div class="model-action">
        <button v-if="!m.installed" class="btn btn-primary" @click="download(m)">
          <Icon name="download" :size="13" /> 下载
        </button>
        <span v-else class="chip g"><Icon name="check" :size="11" /> 已安装</span>
      </div>
    </div>
    <div v-if="!models.length" class="empty small">暂无模型</div>
  </div>
</template>

<script setup>
import { api } from '../api/client'
import { toast } from '../stores/toast'
import Icon from './Icon.vue'

defineProps({ models: { type: Array, default: () => [] } })
const emit = defineEmits(['changed'])

async function download(m) {
  try {
    await api('/api/models/download', { method: 'POST' })
    toast('全部 ASR 模型下载已启动（约 1.5GB）')
    emit('changed')
  } catch (e) { toast(e.message, 'err') }
}
</script>

<style scoped>
.model-row { display:flex; align-items:center; justify-content:space-between; padding:12px 4px; border-bottom:1px dashed var(--line-soft); }
.model-row:last-child { border-bottom:none; }
.model-name { font-weight:700; font-size:15px; }
.model-action { flex:none; }
</style>
