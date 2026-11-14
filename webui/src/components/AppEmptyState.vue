<template>
  <div class="empty-state card card-blk">
    <div v-if="icon" class="es-icon"><Icon :name="icon" :size="22" /></div>
    <b class="es-title">{{ title }}</b>
    <p v-if="description" class="es-desc">{{ description }}</p>
    <div v-if="primary || secondary" class="es-actions">
      <router-link v-if="primaryTo" class="btn btn-primary" :to="primaryTo">{{ primary }}</router-link>
      <button v-else-if="primary" class="btn btn-primary" @click="$emit('primary')">{{ primary }}</button>
      <router-link v-if="secondaryTo" class="btn" :to="secondaryTo">{{ secondary }}</router-link>
      <button v-else-if="secondary" class="btn" @click="$emit('secondary')">{{ secondary }}</button>
    </div>
  </div>
</template>

<script setup>
// 统一空态：无项目 / 项目 404 / 无可用来源 / 无片段 / 未初始化。
// 只负责"长什么样"；点哪个动作由父层给 to 或监听 primary/secondary 事件。
import Icon from './Icon.vue'

defineProps({
  icon: { type: String, default: '' },
  title: { type: String, required: true },
  description: { type: String, default: '' },
  primary: { type: String, default: '' },
  secondary: { type: String, default: '' },
  primaryTo: { type: [String, Object], default: '' },
  secondaryTo: { type: [String, Object], default: '' }
})
defineEmits(['primary', 'secondary'])
</script>

<style scoped>
.empty-state {
  max-width: 560px;
  margin: 36px auto;
  padding: 30px 28px;
  text-align: center;
}
.es-icon {
  width: 48px;
  height: 48px;
  margin: 0 auto 12px;
  border-radius: var(--r-full);
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--main-soft);
  color: var(--pink-text);
  box-shadow: 0 0 0 6px var(--bg-soft);
}
/* 标题用 --ink 而不是 --main-deep：空态是"页面状态"，不是品牌强调；
   橙色标题在暗色底上反而会跟主按钮抢注意力。 */
.es-title { display: block; font-size: 15px; font-weight: 650; color: var(--ink); }
.es-desc { margin: 8px auto 0; max-width: 42ch; color: var(--ink-dim); font-size: 12.5px; line-height: 1.7; }
.es-actions {
  display: flex;
  gap: 8px;
  justify-content: center;
  flex-wrap: wrap;
  margin-top: 16px;
}
</style>
