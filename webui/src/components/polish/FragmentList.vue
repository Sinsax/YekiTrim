<template>
  <div class="frag-list">
    <div
      v-for="(r, i) in rows"
      :key="r.name"
      class="frag-row"
      :class="{ on: r.name === current }"
      @click="emit('goto', r.name)"
    >
      <span class="frag-title">{{ i + 1 }} · {{ r.title }}</span>
      <span class="chip" :class="r.chip">{{ r.state }}</span>
      <button
        v-if="!r.initialized"
        class="mini-btn"
        title="解析字幕/弹幕并创建 polish.toml"
        @click.stop="emit('init', r.name)"
      >初始化</button>
    </div>
  </div>
</template>

<script setup>
// 精修页的片段列表（列表模式 / 未初始化态 / 编辑器右侧共用一份标记）
defineProps({
  rows: { type: Array, default: () => [] },
  current: { type: String, default: '' }
})
const emit = defineEmits(['goto', 'init'])
</script>
