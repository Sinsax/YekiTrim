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
        <router-link class="nav-pill" to="/settings">设置</router-link>
      </nav>
      <div class="top-actions">
        <button class="bell" title="运行任务" @click="showJobs = !showJobs">
          <Icon name="bell" :size="16" />
          <i v-if="hasRunning" class="red-dot"></i>
        </button>
      </div>
    </header>
    <main class="wrap">
      <router-view />
    </main>
    <ToastHost />
    <AppDialog />
    <div v-if="showJobs" class="job-drawer">
      <div class="row spread mb-8"><b>运行任务</b><button class="mini-btn" @click="showJobs = false">关闭</button></div>
      <JobList />
    </div>
  </div>
</template>

<script setup>
import { ref } from 'vue'
import Icon from './components/Icon.vue'
import ToastHost from './components/ToastHost.vue'
import AppDialog from './components/AppDialog.vue'
import JobList from './components/JobList.vue'
import { useJobs } from './composables/useJobs'

const { hasRunning } = useJobs()
const showJobs = ref(false)
</script>

<style scoped>
.app-shell { min-height:100vh; }
.topbar { position:sticky; top:0; z-index:30; display:flex; align-items:center; gap:12px; padding:10px 20px; background:rgba(255,247,243,.88); backdrop-filter:blur(12px); border-bottom:1px solid var(--line); flex-wrap:wrap; }
.brand { display:flex; align-items:center; gap:10px; text-decoration:none; color:inherit; }
.brand-logo { width:36px; height:36px; border-radius:50%; background:linear-gradient(135deg,var(--main),var(--pink-light)); display:flex; align-items:center; justify-content:center; font-size:17px; color:#fff; box-shadow:var(--shadow-pink); }
.brand-name { font-size:15px; font-weight:700; color:var(--main-deep); line-height:1.2; }
.brand-sub { font-size:9px; color:var(--ink-faint); letter-spacing:.1em; }
.nav-pills { display:flex; gap:3px; background:var(--card); border:1px solid var(--line); border-radius:var(--r-full); padding:4px; box-shadow:0 2px 10px rgba(90,60,70,.05); }
.nav-pill { padding:6px 15px; border-radius:var(--r-full); font-size:13px; color:var(--ink-dim); cursor:pointer; text-decoration:none; }
.nav-pill.router-link-active { background:linear-gradient(135deg,var(--main),var(--main-deep)); color:#fff; font-weight:600; }
.nav-pill:hover { background:var(--bg-soft); color:var(--ink); }
.top-actions { display:flex; gap:8px; align-items:center; margin-left:auto; }
.bell { position:relative; width:34px; height:34px; border-radius:50%; background:var(--card); border:1px solid var(--line); color:var(--ink-dim); display:flex; align-items:center; justify-content:center; text-decoration:none; cursor:pointer; transition:all .15s; }
.bell:hover { color:var(--main-deep); border-color:var(--main); }
.red-dot { position:absolute; top:3px; right:3px; width:8px; height:8px; border-radius:50%; background:var(--red); border:2px solid var(--bg); }
.wrap { max-width:1360px; margin:0 auto; padding:20px 20px 80px; position:relative; z-index:1; }
.job-drawer { position:fixed; top:60px; right:16px; width:320px; max-height:70vh; overflow:auto; background:var(--card); border:1px solid var(--line); border-radius:var(--r); padding:14px; box-shadow:var(--shadow-hover); z-index:90; }
</style>
