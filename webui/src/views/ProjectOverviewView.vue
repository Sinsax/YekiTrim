<template>
  <div>
    <div class="card">
      <div class="row spread">
        <h2 class="m-0">项目信息</h2>
        <span class="chip" :class="status.current_stage >= 4 ? 'g' : status.current_stage > 0 ? 'p' : 'n'">Stage {{ status.current_stage }}</span>
      </div>
      <Steps :stage="status.current_stage || 0" size="large" />
      <div class="row mt-10">
        <button class="mini-btn" @click="run('extract')">提取</button>
        <button class="mini-btn" @click="run('transcribe')">转写</button>
        <button class="mini-btn" @click="run('detect')">检测</button>
        <button class="mini-btn" @click="run('cut')">剪切</button>
        <button class="btn btn-primary" @click="run('process')">全流程</button>
      </div>
    </div>

    <div class="card mt-16">
      <h2>产物</h2>
      <div v-if="!status.files?.length" class="empty small">暂无产物</div>
      <div v-for="f in status.files" :key="f" class="mono small file-line">{{ f }}</div>
    </div>

    <div class="card mt-16">
      <h2>运行任务</h2>
      <JobList />
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { api } from '../api/client'
import { toast } from '../stores/toast'
import Steps from '../components/Steps.vue'
import JobList from '../components/JobList.vue'

const status = ref({})

async function load() {
  try { status.value = await api('/api/project/status') } catch {}
}

async function run(stage) {
  try {
    await api('/api/pipeline/stage', { method: 'POST', body: JSON.stringify({ stage, resume: 'auto' }) })
    toast('已启动 ' + stage)
    load()
  } catch (e) { toast(e.message, 'err') }
}

onMounted(load)
</script>

<style scoped>
.file-line { padding:3px 0; border-bottom:1px dashed var(--line-soft); }
</style>
