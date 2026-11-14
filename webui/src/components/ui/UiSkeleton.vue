<template>
  <!-- 骨架屏：`line / card / grid / table` 四档 + 数量。
       旧写法是到处 `<div class="skeleton"></div>`，高度与圆角随上下文漂移。 -->
  <div class="uskeleton" :class="`v-${variant}`" :style="styleObj" aria-hidden="true">
    <div
      v-for="i in count"
      :key="i"
      class="skeleton"
      :class="{ line: variant === 'line', card: variant === 'card', grid: variant === 'grid' }"
      :style="itemStyle"
    ></div>
  </div>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  /** 'line' | 'card' | 'grid' | 'block' */
  variant: { type: String, default: 'block' },
  count: { type: Number, default: 1 },
  /** 高度（数字 = px）；不给就用各档默认 */
  height: { type: [Number, String], default: 0 },
  /** grid 档的列宽 */
  min: { type: Number, default: 220 }
})

const styleObj = computed(() => (
  props.variant === 'grid'
    ? { display: 'grid', gap: 'var(--s-3)', gridTemplateColumns: `repeat(auto-fill, minmax(${props.min}px, 1fr))` }
    : { display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }
))
const itemStyle = computed(() => (props.height ? { height: `${props.height}px`, minHeight: `${props.height}px` } : {}))
</script>

<style scoped>
.skeleton.card { min-height:120px; border-radius:var(--r-md); }
.skeleton.grid { min-height:0; aspect-ratio:16/10; border-radius:var(--r-md); }
</style>
