<template>
  <div class="app-shell">
    <header class="topbar">
      <a class="brand" href="#/">
        <div class="brand-logo">V</div>
        <div>
          <div class="brand-name">V-TRIM</div>
          <div class="brand-sub">切片管理台</div>
        </div>
      </a>
      <nav class="nav-pills">
        <router-link class="nav-pill" to="/">总览</router-link>
        <router-link class="nav-pill" to="/projects">项目</router-link>
        <router-link class="nav-pill" to="/models">模型</router-link>
        <router-link class="nav-pill" to="/polish">精修</router-link>
        <router-link class="nav-pill" to="/settings">设置</router-link>
      </nav>
      <div class="top-actions">
        <button class="bell" @click="toggleJobs">🔔<i v-if="hasRunning" class="red-dot"></i></button>
        <router-link class="bell" to="/jobs">列表</router-link>
        <span class="avatar">P</span>
      </div>
    </header>
    <main class="wrap">
      <router-view />
    </main>
    <ToastHost />
    <div v-if="showJobs" class="job-drawer">
      <div class="row spread"><b>运行任务</b><button class="mini-btn" @click="showJobs = false">关闭</button></div>
      <div v-if="!jobs.length" class="muted small" style="margin-top:8px">暂无任务</div>
      <div v-for="j in jobs" :key="j.id" style="padding:7px 0;border-bottom:1px dashed var(--line-soft)">
        <div class="row spread"><b>{{ j.kind }}</b><span class="chip" :class="j.status === 'failed' ? 'r' : j.status === 'running' ? 'b' : 'g'">{{ j.status }}</span></div>
        <div class="small muted">{{ j.message }}</div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { api } from './api/client'
import ToastHost from './components/ToastHost.vue'

const jobs = ref([])
const showJobs = ref(false)
const hasRunning = computed(() => jobs.value.some(j => j.status === 'running' || j.status === 'queued'))
let timer = null

async function loadJobs() {
  try { jobs.value = await api('/api/jobs') } catch (e) { jobs.value = [] }
}

function toggleJobs() {
  showJobs.value = !showJobs.value
  if (showJobs.value) loadJobs()
}

onMounted(() => {
  loadJobs()
  timer = setInterval(loadJobs, 3000)
})

onUnmounted(() => {
  if (timer) clearInterval(timer)
})
</script>

<style scoped>
.app-shell { min-height:100vh; }
.topbar { position:sticky; top:0; z-index:30; display:flex; align-items:center; gap:12px; padding:10px 20px; background:rgba(255,247,243,.88); backdrop-filter:blur(12px); border-bottom:1px solid var(--line); flex-wrap:wrap; }
.brand { display:flex; align-items:center; gap:10px; text-decoration:none; color:inherit; }
.brand-logo { width:36px; height:36px; border-radius:50%; background:linear-gradient(135deg,var(--main),#FBB9D4); display:flex; align-items:center; justify-content:center; font-size:17px; color:#fff; box-shadow:0 4px 12px rgba(238,111,166,.4); }
.brand-name { font-size:15px; font-weight:700; color:#E86FA5; line-height:1.2; }
.brand-sub { font-size:9px; color:var(--ink-faint); letter-spacing:.1em; }
.nav-pills { display:flex; gap:3px; background:var(--card); border:1px solid var(--line); border-radius:999px; padding:4px; box-shadow:0 2px 10px rgba(90,60,70,.05); }
.nav-pill { padding:6px 15px; border-radius:999px; font-size:13px; color:var(--ink-dim); cursor:pointer; text-decoration:none; }
.nav-pill.router-link-active { background:linear-gradient(135deg,var(--main),var(--main-deep)); color:#fff; font-weight:600; }
.nav-pill:hover { background:var(--bg-soft); color:var(--ink); }
.top-actions { display:flex; gap:8px; align-items:center; margin-left:auto; }
.bell { position:relative; width:34px; height:34px; border-radius:50%; background:var(--card); border:1px solid var(--line); display:flex; align-items:center; justify-content:center; text-decoration:none; cursor:pointer; }
.red-dot { position:absolute; top:3px; right:3px; width:8px; height:8px; border-radius:50%; background:var(--red); border:2px solid var(--bg); }
.avatar { width:34px; height:34px; border-radius:50%; background:linear-gradient(135deg,var(--blue),#A9C2F6); display:flex; align-items:center; justify-content:center; color:#fff; }
.wrap { max-width:1360px; margin:0 auto; padding:20px 20px 80px; position:relative; z-index:1; }
.job-drawer { position:fixed; top:60px; right:16px; width:320px; max-height:70vh; overflow:auto; background:var(--card); border:1px solid var(--line); border-radius:16px; padding:14px; box-shadow:var(--shadow-hover); z-index:90; }
</style>
