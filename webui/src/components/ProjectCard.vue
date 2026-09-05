<template>
  <div class="pcard" @click="emit('open', p)">
    <span v-if="p.stage >= 4" class="ribbon">已完成</span>
    <div class="head">
      <div class="sticker" :class="stickerColor(p.name)">{{ stickerIcon(p.name) }}</div>
      <div>
        <h3>{{ p.name }}</h3>
        <div class="path">{{ p.input_file || p.dir }}</div>
      </div>
    </div>
    <Steps :stage="p.stage || 0" />
    <div class="chips mt-8">
      <span class="chip" :class="p.stage >= 4 ? 'g' : p.stage > 0 ? 'p' : 'n'">Stage {{ p.stage }}</span>
      <span class="chip p">{{ p.clip_videos || 0 }} 片段</span>
      <span v-if="p.has_danmaku" class="chip b">弹幕 ✓</span>
    </div>
    <div class="foot">
      <span class="name">{{ p.dir }}</span>
      <button class="mini-btn" @click.stop="emit('open', p)">进入 <Icon name="arrow-right" :size="11" /></button>
      <button v-if="removable" class="mini-btn danger" @click.stop="emit('remove', p)">删除</button>
    </div>
  </div>
</template>

<script setup>
import Steps from './Steps.vue'
import Icon from './Icon.vue'
import { stickerColor, stickerIcon } from '../utils'

defineProps({
  p: { type: Object, required: true },
  removable: { type: Boolean, default: false }
})
const emit = defineEmits(['open', 'remove'])
</script>
