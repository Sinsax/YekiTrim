<template>
  <div>
    <div class="model-row" v-for="m in models" :key="m.name">
      <div class="model-main">
        <div class="model-name">
          {{ m.name }}
          <span v-if="m.source_label" class="chip" :class="m.installed ? 'g' : 'n'">{{ m.source_label }}</span>
        </div>
        <div class="muted small">{{ m.size_label }} · {{ m.note }}</div>
        <!-- 实际路径：判"装没装"跟运行时同一套查找顺序（local_model_dir → 数据目录），
             所以这里要如实显示**用到的那一份**在哪，不然会出现"界面说没下、其实在用"的错位 -->
        <div class="muted small mono model-path" :title="m.dir">{{ m.dir }}</div>
      </div>
      <div class="model-action">
        <button v-if="!m.installed" class="btn btn-primary" :disabled="!!dlBusy"
                :title="dlBusy ? '正在' + kindText(dlBusy.kind) + '：模型目录被占用（转写也会写它）' : ''"
                @click="download(m)">
          <Icon name="download" :size="13" /> 下载
        </button>
        <span v-else class="chip g"><Icon name="check" :size="11" /> 已安装</span>
      </div>
    </div>
    <div v-if="!models.length" class="empty small">暂无模型</div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { api } from '../api/client'
import { toast } from '../stores/toast'
import Icon from './Icon.vue'
import { busyOn, MODELS_KEY, kindText, QUEUED_HINT } from '../api/jobs'
import { useJobs } from '../composables/useJobs'

defineProps({ models: { type: Array, default: () => [] } })
const emit = defineEmits(['changed'])

// 模型目录是全局共享资源（`models` 键）：下载与任何会 ensure_models 的转写互斥（排队）。
const { jobs: allJobs } = useJobs()
const dlBusy = computed(() => busyOn(allJobs.value, [MODELS_KEY]))

async function download(m) {
  if (dlBusy.value) { toast('模型目录正被' + kindText(dlBusy.value.kind) + '占用，请稍候', 'err'); return }
  try {
    const r = await api('/api/models/download', { method: 'POST' })
    toast(r?.queued ? QUEUED_HINT : '全部 ASR 模型下载已启动（约 1.1GB）')
    emit('changed')
  } catch (e) { toast(e.message, 'err') }
}
</script>

<style scoped>
.model-row { display:flex; align-items:center; justify-content:space-between; gap:12px; padding:12px 4px; border-bottom:1px dashed var(--line-soft); }
.model-row:last-child { border-bottom:none; }
.model-main { min-width:0; }
.model-name { font-weight:700; font-size:15px; display:flex; align-items:center; gap:6px; }
.model-path { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.model-action { flex:none; }
.hint { font-size:12px; color:var(--ink-dim); line-height:1.6; }
</style>
