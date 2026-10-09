<template>
  <div class="card card-blk mb-12">
    <div class="row spread mb-8">
      <b class="t m-0"><Icon name="image" :size="15" /> 素材库</b>
      <div class="row">
        <button v-for="t in TABS" :key="t.key" class="mini-btn"
                :class="{ selected: tab === t.key }"
                :title="t.hint"
                @click="tab = t.key">{{ t.label }}</button>
      </div>
    </div>

    <!-- ⚠️ **三合一只是 UI 上的合并**（用户原话：「素材库将视频素材，字体，贴纸，合并一下，
         只是ui上合并，本质只要做视频素材的功能」）—— 所以这一层只做页签切换，
         三块内容仍然是各自原来的实现（`StickerPacks` / `FontLibrary` 一行未改）。
         别在这里重写它们的内部逻辑：那会把两份实现变成三份。 -->
    <p class="hint m-0 mb-8">{{ activeHint }}</p>

    <div v-show="tab === 'video'">
      <div class="row spread mb-8">
        <span class="muted small">
          {{ videos.length }} 个视频
          <template v-if="!videos.length">· 把一个视频文件放进下面的目录即可用</template>
        </span>
        <button class="mini-btn" :disabled="loading" @click="load">
          <span v-if="loading" class="spinner"></span> 刷新
        </button>
      </div>

      <div class="hint">
        视频库：<span class="mono">{{ root }}</span>
        <span class="muted">片头 / 片尾从这里挑（项目设置内开启）</span>
      </div>

      <div v-if="error" class="error-banner mt-8"><Icon name="alert" :size="15" /> {{ error }}</div>

      <div v-if="!videos.length && !loading && !error" class="empty small mt-8">
        还没有视频素材 —— 把 mp4 / webm / mkv 放进上面那个目录，再点「刷新」
      </div>

      <div v-else class="vid-list mt-8">
        <div v-for="v in videos" :key="v.name" class="vid-row">
          <span class="mono vid-name">{{ v.name }}</span>
          <span class="muted small">{{ v.file }}</span>
          <!-- ⚠️ 探不出时长的**照列不隐藏**：用户放进去了却看不到，比"能用但标出来"更难查 -->
          <span v-if="v.duration == null" class="chip a" title="ffprobe 读不出时长，片头/片尾里不能选它">
            读不出时长
          </span>
          <span v-else class="chip n">{{ v.duration.toFixed(1) }}s</span>
        </div>
      </div>
    </div>

    <!-- 字体 / 贴纸：保持各自原有的实现与权限门控（仅 admin） -->
    <div v-show="tab === 'font'">
      <FontLibrary v-if="isAdmin" />
      <p v-else class="empty small m-0">字体库是程序级资产，只有管理员能管理。</p>
    </div>
    <div v-show="tab === 'sticker'">
      <StickerPacks v-if="isAdmin" />
      <p v-else class="empty small m-0">表情包是程序级资产，只有管理员能管理。</p>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import Icon from './Icon.vue'
import FontLibrary from './FontLibrary.vue'
import StickerPacks from './StickerPacks.vue'
import { api } from '../api/client'

defineProps({ isAdmin: { type: Boolean, default: false } })

const TABS = [
  { key: 'video', label: '视频', hint: '片头 / 片尾的素材来源' },
  { key: 'font', label: '字体', hint: '字幕与封面可选的字体' },
  { key: 'sticker', label: '贴纸', hint: '精修时 LLM 按 tag 选的贴纸' },
]
const tab = ref('video')

const activeHint = computed(() => TABS.find((t) => t.key === tab.value)?.hint || '')

const videos = ref([])
const root = ref('')
const loading = ref(false)
const error = ref('')

// ⚠️ 素材库是**全局**资产（不在任何项目里），所以这条请求**不带 `{project, dir}` 作用域** ——
// 与字体/贴纸库同规矩。别给 `api()` 硬塞一个作用域：那会去查一个跟素材库无关的项目。
async function load() {
  loading.value = true
  error.value = ''
  try {
    const r = await api('/api/polish/video-library', { label: '读取视频素材库' })
    videos.value = r?.videos || []
    root.value = r?.root || ''
  } catch (e) {
    // ⚠️ 读失败**不伪装成空库**（否则用户会以为素材丢了、反复重放文件）
    error.value = '视频素材库读取失败：' + e.message
  } finally {
    loading.value = false
  }
}

onMounted(load)
</script>

<style scoped>
.vid-list { display:flex; flex-direction:column; gap:4px; }
.vid-row { display:flex; align-items:center; gap:8px; font-size:12px; }
.vid-name { font-weight:600; }
.vid-row .chip { margin-left:auto; flex:none; }
</style>
