<template>
  <div>
    <div class="page-head">
      <div><h1>运行监控</h1><div class="sub">任务队列 / 进度 / 实时状态</div></div>
    </div>
    <div class="card">
      <JobList actions />
    </div>
  </div>
</template>

<script setup>
import JobList from '../components/JobList.vue'
</script>
