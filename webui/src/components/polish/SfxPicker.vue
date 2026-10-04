<script setup>
// 音效选择器（时间轴属性面板里用）：sfx 事件名 / 特效配声都从目录里**选**，不再手打 id。
// 数据来自 `GET /api/polish/sfx-catalog`（编译进二进制的目录，已按全局 [sfx] blacklist 过滤）。
import { computed, ref } from 'vue'

const props = defineProps({
  entries: { type: Array, default: () => [] }, // [{id, label, cat, tags}]
  value: { type: String, default: '' },
  placeholder: { type: String, default: '筛选音效…' },
})
const emit = defineEmits(['pick'])

const filter = ref('')
const open = ref(false)

const shown = computed(() => {
  const kw = String(filter.value || '').trim().toLowerCase()
  const list = props.entries || []
  const hit = kw
    ? list.filter((e) => (`${e.id} ${e.label} ${e.cat} ${(e.tags || []).join(' ')}`).toLowerCase().includes(kw))
    : list
  return hit.slice(0, 60)
})

function pick(id) {
  emit('pick', id)
  open.value = false
  filter.value = ''
}
</script>

<template>
  <div class="sfxp">
    <div class="row gap-6">
      <input class="sfxp-cur mono flex-1" :value="value" readonly :placeholder="placeholder"
             @focus="open = true" @click="open = !open" @input.stop @change.stop>
      <button v-if="value" class="mini-btn" title="清空" @click="emit('pick', '')"><i>×</i></button>
    </div>
    <template v-if="open">
      <input v-model="filter" class="sfxp-filter" :placeholder="placeholder" @click.stop @input.stop @change.stop>
      <div class="sfxp-list">
        <div v-for="e in shown" :key="e.id" class="sfxp-item" :class="{ on: e.id === value }"
             :title="e.cat + ' · ' + (e.tags || []).join('/')" @click="pick(e.id)">
          <b>{{ e.label || e.id }}</b><span class="muted small mono">{{ e.id }}</span>
        </div>
        <div v-if="!shown.length" class="muted small pad">没有匹配的音效（全局 [sfx] blacklist 里的不列）</div>
      </div>
    </template>
    <div v-if="value && !open" class="hint mono">{{ value }}</div>
  </div>
</template>

<style scoped>
.sfxp { margin-top: 4px; }
.sfxp-cur { cursor: pointer; background: var(--bg-soft); }
.sfxp-filter { margin-top: 4px; }
.sfxp-list { max-height: 168px; overflow: auto; border: 1px solid var(--line); border-radius: 8px; margin-top: 4px; }
.sfxp-item { display: flex; justify-content: space-between; gap: 6px; padding: 3px 8px; font-size: 11.5px; cursor: pointer; }
.sfxp-item span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sfxp-item:hover { background: var(--bg-soft); }
.sfxp-item.on { background: var(--main-soft, rgba(229,140,150,.15)); }
.pad { padding: 6px 8px; }
</style>
