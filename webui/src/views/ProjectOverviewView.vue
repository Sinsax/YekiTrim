<template>
  <div>
    <div class="card">
      <h2>项目信息</h2>
      <div class="row small muted">当前 Stage {{ status.current_stage }}</div>
      <div class="steps">
        <template v-for="i in 4" :key="i">
          <span class="dot" :class="i < status.current_stage ? 'done' : i === status.current_stage ? 'now' : 'wait'">{{ i < status.current_stage ? '✓' : i }}</span>
          <span v-if="i < 4" class="sl" :class="i < status.current_stage ? 'on' : ''"></span>
        </template>
      </div>
      <div class="row" style="margin-top:10px">
        <button class="mini-btn" @click="run('extract')">提取</button>
        <button class="mini-btn" @click="run('transcribe')">转写</button>
        <button class="mini-btn" @click="run('detect')">检测</button>
        <button class="mini-btn" @click="run('cut')">剪切</button>
      </div>
    </div>

    <div class="card">
      <h2>产物</h2>
      <div v-if="!status.files?.length" class="muted">暂无产物</div>
      <div v-for="f in status.files" :key="f" class="mono small">{{ f }}</div>
    </div>

    <div class="card">
      <h2>运行任务</h2>
      <div v-if="!jobs.length" class="muted">暂无任务</div>
      <div v-for="j in jobs" :key="j.id" style="padding:7px 0;border-bottom:1px dashed var(--line-soft)">
        <div class="row spread"><b>{{ j.kind }}</b><span class="chip" :class="j.status === 'failed' ? 'r' : j.status === 'running' ? 'b' : 'g'">{{ j.status }}</span></div>
        <div class="small muted">{{ j.message }}</div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { api } from '../api/client'
import { toast } from '../stores/toast'

const status = ref({})
const jobs = ref([])

async function load() {
  try { status.value = await api('/api/project/status') } catch (e) {}
  try { jobs.value = await api('/api/jobs') } catch (e) { jobs.value = [] }
}

async function run(stage) {
  try {
    await api('/api/pipeline/stage', { method: 'POST', body: JSON.stringify({ stage, resume: 'auto' }) })
    toast('已启动 ' + stage)
    load()
  } catch (e) { toast(e.message) }
}

onMounted(load)
</script>

<style scoped>
.steps { display:flex; align-items:center; gap:4px; margin:10px 0 4px; }
.dot { width:20px; height:20px; border-radius:50%; display:flex; align-items:center; justify-content:center; font-size:10px; font-weight:700; }
.dot.done { background:var(--green-soft); color:var(--green); }
.dot.now { background:var(--main); color:#fff; box-shadow:0 0 0 3px var(--main-soft); }
.dot.wait { background:var(--card); border:2px dashed var(--ink-faint); color:var(--ink-faint); }
.sl { flex:1; height:5px; border-radius:3px; background:var(--line-soft); }
.sl.on { background:var(--main-glow); }
</style>
