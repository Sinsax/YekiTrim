<template>
  <div class="next-step card">
    <div class="ns-body">
      <b class="ns-title">{{ title }}</b>
      <span v-if="desc" class="muted small">{{ desc }}</span>
    </div>
    <router-link v-if="to" class="btn btn-primary ns-cta" :to="to" :data-cap="dataCap || null">{{ cta }}</router-link>
    <button v-else class="btn btn-primary ns-cta" :data-cap="dataCap || null" @click="$emit('action')">{{ cta }}</button>
  </div>
</template>

<script setup>
// 「下一步」条：识别完成、切片完成两处共用。
// 有 to 就渲染 router-link，没有就点一下 emit('action')（让父层决定怎么跳）。
defineProps({
  title: { type: String, required: true },
  desc: { type: String, default: '' },
  cta: { type: String, required: true },
  to: { type: [String, Object], default: '' },
  /** 能力清单锚点：挂到真正的 CTA 上（不是根 div），便于运行期探针点它。 */
  dataCap: { type: String, default: '' }
})
defineEmits(['action'])
</script>

<style scoped>
/* 「下一步」条：整页唯一的完成态 CTA，所以给它左侧一条主色强调条 + 更实的底，
   同时**不加阴影**（它贴在内容流里，浮起来会像一张可点的卡片）。 */
.next-step {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  margin-bottom: 12px;
  border-color: var(--main-glow);
  background: var(--main-soft);
  border-left: 3px solid var(--main);
}
.ns-body { display: flex; align-items: baseline; gap: 10px; flex: 1; min-width: 0; flex-wrap: wrap; }
.ns-title { color: var(--ink); font-weight: 650; font-size: 13px; }
.ns-cta { flex: none; }
@media (max-width:560px) {
  .next-step { flex-wrap: wrap; }
  .ns-cta { width: 100%; justify-content: center; }
}
</style>
