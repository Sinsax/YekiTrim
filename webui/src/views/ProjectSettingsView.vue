<template>
  <div>
    <div class="card">
      <h2>项目配置 vtrim.toml</h2>
      <p class="muted small">保存后阶段推导会立即刷新。</p>
      <textarea v-model="raw" rows="18" class="mono"></textarea>
      <div class="row" style="margin-top:10px">
        <button class="btn btn-primary" @click="save" :disabled="!loaded">保存</button>
        <button class="btn" @click="load">重新加载</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api/client'
import { toast } from '../stores/toast'

const route = useRoute()
const cfg = route.params.cfg || ''
const cfgQ = encodeURIComponent(cfg)
const raw = ref('')
// 只有成功读到 vtrim.toml 原文才允许保存：否则会把空文件写回去
const loaded = ref(false)

async function load() {
  try {
    const d = await api(`/api/project-data/config?config=${cfgQ}`)
    raw.value = d.vtrim_raw || ''
    loaded.value = true
  } catch (e) {
    loaded.value = false
    toast(e.message, 'err')
  }
}

async function save() {
  if (!loaded.value) { toast('配置尚未加载成功，已禁止保存', 'err'); return }
  if (!raw.value.trim()) { toast('配置内容为空，已禁止保存', 'err'); return }
  try {
    await api(`/api/project-data/config?config=${cfgQ}`, { method: 'PUT', body: JSON.stringify({ raw: raw.value }) })
    toast('项目配置已保存')
  } catch (e) {
    toast(e.message, 'err')
  }
}

onMounted(load)
</script>

<style scoped>
button:disabled { opacity:.45; cursor:not-allowed; }
</style>
