<template>
  <div class="pcard" @click="emit('open', p)">
    <span v-if="p.stage >= 4" class="ribbon">已完成</span>
    <span v-if="p.missing" class="ribbon warn">缺失</span>
    <div class="head">
      <div class="sticker" :class="stickerColor(p.name)">{{ stickerIcon(p.name) }}</div>
      <div>
        <h3>{{ p.name }}</h3>
        <div class="path">{{ sourceLabel(p) }}</div>
      </div>
    </div>
    <Steps :stage="p.stage || 0" />
    <div class="chips mt-8">
      <span class="chip" :class="p.stage >= 4 ? 'g' : p.stage > 0 ? 'p' : 'n'">Stage {{ p.stage }}</span>
      <span class="chip" :class="p.has_clips ? 'g' : 'n'">{{ p.has_clips ? '已切片' : '未切片' }}</span>
      <span class="chip b">{{ humanBytes(p.bytes) }}</span>
      <span v-if="p.room" class="chip a">房间 {{ p.room }}</span>
    </div>
    <div class="foot">
      <span class="name">#{{ p.id }}</span>
      <button class="mini-btn" @click.stop="emit('open', p)">进入 <Icon name="arrow-right" :size="11" /></button>
      <button
        v-if="removable && p.source_bytes > 0"
        class="mini-btn"
        title="删除源录播（保留切片/字幕/事件/封面）"
        @click.stop="emit('remove-source', p)"
      >删源</button>
      <button v-if="removable" class="mini-btn danger" @click.stop="emit('remove', p)">删除</button>
    </div>
  </div>
</template>

<script setup>
import Steps from './Steps.vue'
import Icon from './Icon.vue'
import { stickerColor, stickerIcon } from '../utils'
import { humanBytes, sourceLabel } from '../stores/project'

defineProps({
  p: { type: Object, required: true },
  removable: { type: Boolean, default: false }
})
const emit = defineEmits(['open', 'remove', 'remove-source'])
</script>

<style scoped>
.ribbon.warn { background:var(--amber-soft); color:var(--amber-deep); }
</style>
