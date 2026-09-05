<template>
  <div>
    <div class="page-head">
      <div><h1>项目</h1><div class="sub">多项目工作区</div></div>
      <div class="actions"><button class="btn btn-primary" @click="showNew = true">＋ 新建项目</button></div>
    </div>
    <div class="grid cols-3">
      <div v-for="p in projects" :key="p.config_file" class="pcard" @click="openProject(p)">
        <div class="head">
          <div class="sticker pink">📺</div>
          <div>
            <h3>{{ p.name }}</h3>
            <div class="path">{{ p.input_file }}</div>
          </div>
        </div>
        <div class="chips">
          <span class="chip p">Stage {{ p.stage }}</span>
          <span class="chip p">{{ p.clip_videos || 0 }} 片段</span>
        </div>
        <div class="foot">
          <span class="name">{{ p.dir }}</span>
          <button class="mini-btn" @click.stop="openProject(p)">进入 ▸</button>
          <button class="mini-btn" @click.stop="removeProject(p)">删除</button>
        </div>
      </div>
      <div v-if="!projects.length" class="card muted">暂无项目</div>
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
import { api } from '../api/client'
import { toast } from '../stores/toast'
import { useRouter } from 'vue-router'

const projects = ref([])
const router = useRouter()
const showNew = ref(false)
const newInput = ref('')
const newDanmaku = ref('')
const newOutput = ref('./output')

async function load() {
  try {
    const d = await api('/api/project/list')
    projects.value = d.projects || []
  } catch (e) {
    projects.value = []
  }
}

function openProject(p) {
  router.push(`/project/${encodeURIComponent(p.config_file)}/preview`)
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

async function removeProject(p) {
  if (!confirm('只删除 vtrim.toml，不会触碰媒体文件。确认？')) return
  try {
    await api('/api/project/delete', { method: 'POST', body: JSON.stringify({ config_path: p.config_file }) })
    load()
  } catch (e) { toast(e.message) }
}

onMounted(load)
</script>
