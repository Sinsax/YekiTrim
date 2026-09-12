<template>
  <div>
    <div v-if="!jobs.length" class="empty small">暂无任务</div>
    <div v-for="j in jobs" :key="j.id" class="job-row">
      <div class="row spread">
        <b>{{ kindText(j.kind) }}</b>
        <span class="chip" :class="statusClass(j.status)">{{ statusText(j.status) }}</span>
      </div>
      <div v-if="!((j.status === 'succeeded' && j.message === '完成') || (j.status === 'cancelled' && j.message === '已取消'))" class="small muted">
        {{ j.message }}{{ j.total > 0 ? ' · ' + Math.round(j.current * 100 / j.total) + '%' : '' }}
      </div>
      <div v-if="j.status === 'running' && j.total > 0" class="progress mt-6">
        <div :style="{ width: Math.round(j.current * 100 / j.total) + '%' }"></div>
      </div>
      <div v-if="actions && isActive(j)" class="row mt-6">
        <button class="mini-btn danger" @click="cancel(j)">取消</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { api } from '../api/client'
import { toast } from '../stores/toast'
import { useJobs } from '../composables/useJobs'
import { statusClass } from '../utils'

defineProps({ actions: { type: Boolean, default: false } })

const { jobs, reload } = useJobs()

const KIND_ZH = {
  cut_all: '一键全切',
  cut_one: '单事件剪切',
  silence: '去静音',
  correct: 'LLM 订正',
  pipeline: '流水线',
  stage_extract: '音频提取',
  stage_transcribe: '语音转文字',
  stage_detect: '事件检测',
  stage_cut: '剪切',
  init: '初始化',
  llm: 'LLM 生成',
  process: '后处理',
  render: '渲染',
  cover: '封面生成',
  polish_init: '初始化精修',
  polish_llm: 'LLM 生成',
  polish_process: '后处理',
  polish_render: '渲染',
  polish_cover: '封面生成',
  download_models: '下载模型',
}
const STATUS_ZH = {
  queued: '排队中',
  running: '运行中',
  succeeded: '完成',
  failed: '失败',
  cancelled: '已取消',
}
function kindText(k) { return KIND_ZH[k] || k }
function statusText(s) { return STATUS_ZH[s] || s }

function isActive(j) {
  return j.status === 'running' || j.status === 'queued'
}

async function cancel(j) {
  try {
    await api('/api/jobs/' + j.id + '/cancel', { method: 'POST' })
    toast('已请求取消')
    reload()
  } catch (e) { toast(e.message, 'err') }
}
</script>
