<template>
  <div>
    <div class="page-head">
      <div><h1>总览</h1><div class="sub">环境 · 模型 · 项目工作台</div></div>
      <div class="actions"><button class="btn btn-primary" @click="showNew = true">＋ 新建项目</button></div>
    </div>

    <div class="card">
      <h2>环境与模型</h2>
      <div class="chips">
        <span class="chip" :class="health.ffmpeg ? 'g' : 'r'">ffmpeg {{ health.ffmpeg ? '✓' : '✗' }}</span>
        <span class="chip" :class="health.ffprobe ? 'g' : 'r'">ffprobe {{ health.ffprobe ? '✓' : '✗' }}</span>
        <span class="chip" :class="health.llm ? 'g' : 'n'">LLM {{ health.llm ? '✓' : '未配置' }}</span>
      </div>
      <div class="chips" style="margin-top:8px">
        <span v-for="m in models" :key="m.name" class="chip" :class="m.installed ? 'g' : 'n'">{{ m.name }} {{ m.installed ? '已装' : '未装' }}</span>
      </div>
      <div class="row" style="margin-top:8px"><button class="btn" @click="openDoctor">环境检查</button></div>
    </div>

    <div v-if="doctor" class="modal-backdrop" @click.self="doctor = null">
      <div class="modal">
        <h2>环境检查</h2>
        <div class="row"><b>ffmpeg</b><span class="chip" :class="doctor.ffmpeg ? 'g' : 'r'">{{ doctor.ffmpeg ? '✓' : '✗' }}</span></div>
        <div class="row"><b>ffprobe</b><span class="chip" :class="doctor.ffprobe ? 'g' : 'r'">{{ doctor.ffprobe ? '✓' : '✗' }}</span></div>
        <div class="row"><b>模型目录</b><span class="muted small mono">{{ doctor.models_dir }}</span></div>
        <div class="chips" style="margin-top:8px">
          <span v-for="m in doctor.models || []" :key="m.name" class="chip" :class="m.exists ? 'g' : 'n'">{{ m.name }} {{ m.exists ? '已装' : '未装' }}</span>
        </div>
        <div class="row" style="margin-top:12px"><button class="btn" @click="doctor = null">关闭</button></div>
      </div>
    </div>

    <div class="section-title">项目</div>
    <div class="grid cols-4">
      <div v-for="p in projects" :key="p.config_file" class="pcard" @click="openProject(p)">
        <div class="head">
          <div class="sticker pink">{{ p.name?.charAt(0) || 'P' }}</div>
          <div>
            <h3>{{ p.name }}</h3>
            <div class="path">{{ p.input_file }}</div>
          </div>
        </div>
        <div class="chips">
          <span class="chip" :class="p.stage >= 4 ? 'g' : p.stage > 0 ? 'p' : 'n'">Stage {{ p.stage }}</span>
          <span class="chip p">{{ p.clip_videos || 0 }} 片段</span>
        </div>
        <div class="foot"><span class="name">{{ p.dir }}</span><button class="mini-btn">进入 ▸</button></div>
      </div>
      <div v-if="!projects.length" class="card muted">暂无项目</div>
    </div>

    <div class="section-title">运行任务</div>
    <div class="card">
      <div v-if="!jobs.length" class="muted">暂无任务</div>
      <div v-for="j in jobs" :key="j.id" style="padding:7px 0;border-bottom:1px dashed var(--line-soft)">
        <div class="row spread"><b>{{ j.kind }}</b><span class="chip" :class="j.status === 'failed' ? 'r' : j.status === 'running' ? 'b' : 'g'">{{ j.status }}</span></div>
        <div class="small muted">{{ j.message }}</div>
      </div>
    </div>

    <div v-if="showNew" class="modal-backdrop" @click.self="showNew = false">
      <div class="modal">
        <h2>新建项目</h2>
        <label>视频文件路径</label>
        <input v-model="newInput" placeholder="选择或填写 mp4/mkv...">
        <label>弹幕文件（可选）</label>
        <input v-model="newDanmaku" placeholder="自动查找同名 .ass">
        <label>输出目录</label>
        <input v-model="newOutput" value="./output">
        <div class="row" style="margin-top:12px">
          <button class="btn btn-primary" @click="createProject">初始化</button>
          <button class="btn" @click="showNew = false">取消</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { api } from '../api/client'
import { toast } from '../stores/toast'

const router = useRouter()
const health = ref({})
const models = ref([])
const projects = ref([])
const jobs = ref([])
const doctor = ref(null)
const showNew = ref(false)
const newInput = ref('')
const newDanmaku = ref('')
const newOutput = ref('./output')

async function load() {
  try { health.value = await api('/api/health') } catch (e) {}
  try { models.value = await api('/api/models') } catch (e) {}
  try { const d = await api('/api/project/list'); projects.value = d.projects || [] } catch (e) {}
  try { jobs.value = await api('/api/jobs') } catch (e) { jobs.value = [] }
}

async function openDoctor() {
  try { doctor.value = await api('/api/doctor') } catch (e) { toast(e.message) }
}

async function createProject() {
  if (!newInput.value) { toast('请填写视频路径'); return }
  try {
    await api('/api/project/init', { method: 'POST', body: JSON.stringify({
      input: newInput.value,
      danmaku: newDanmaku.value || null,
      output_dir: newOutput.value || './output'
    }) })
    showNew.value = false
    toast('项目已创建')
    load()
  } catch (e) { toast(e.message) }
}

function openProject(p) {
  router.push(`/project/${encodeURIComponent(p.config_file)}/preview`)
}

onMounted(load)
</script>
