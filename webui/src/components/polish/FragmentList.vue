<template>
  <div class="frag-cards" :class="{ compact }">
    <div
      v-for="(r, i) in rows"
      :key="r.name"
      class="frag-card"
      :class="{ on: r.name === current }"
      :title="'点击切换到「' + r.title + '」'"
      @click="emit('goto', r.name)"
    >
      <img v-if="r.thumb" :src="r.thumb" class="frag-thumb" alt="" @error="$event.target.style.display = 'none'">
      <div v-else class="frag-thumb frag-thumb-empty">无缩略图</div>
      <!-- 紧凑态：状态 chip 压到缩略图角上，省掉一行的竖高 -->
      <span class="chip frag-state" :class="r.chip">{{ r.state }}</span>
      <div class="frag-meta">
        <b class="frag-title">{{ i + 1 }} · {{ r.title }}</b>
        <span v-if="!compact" class="chip" :class="r.chip">{{ r.state }}</span>
      </div>
      <button
        v-if="!r.initialized"
        class="mini-btn frag-init"
        title="解析字幕/弹幕并创建 polish.toml"
        @click.stop="emit('init', r.name)"
      >初始化</button>
    </div>
  </div>
</template>

<script setup>
// 精修页的片段卡片行（整行置顶 / 未初始化态侧栏共用一份标记）。
// compact：编辑器态的紧凑条（缩略图小一号 + 状态 chip 贴角），为了给「预览 + 时间轴」腾竖高。
defineProps({
  rows: { type: Array, default: () => [] },
  current: { type: String, default: '' },
  compact: { type: Boolean, default: false }
})
const emit = defineEmits(['goto', 'init'])
</script>

<style scoped>
.frag-cards { display:flex; gap:10px; overflow-x:auto; padding:2px; }
.frag-card { position:relative; flex:none; width:172px; border:1.5px solid var(--line); border-radius:12px;
  padding:8px; cursor:pointer; background:var(--card); transition:.15s; }
.frag-card:hover { border-color:var(--main); transform:translateY(-1px); }
.frag-card.on { border-color:var(--main); background:var(--main-soft); box-shadow:0 4px 14px rgba(238,111,166,.25); }
.frag-thumb { width:100%; aspect-ratio:16/9; object-fit:cover; border-radius:8px;
  background:var(--bg-soft); display:block; }
.frag-thumb-empty { display:flex; align-items:center; justify-content:center;
  color:var(--ink-faint); font-size:11px; }
.frag-meta { display:flex; align-items:center; gap:6px; margin-top:6px; }
.frag-title { flex:1; min-width:0; font-size:12px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.frag-card.on .frag-title { color:var(--pink-text); }
.frag-init { margin-top:6px; width:100%; }
.frag-state { position:absolute; top:12px; left:12px; display:none; font-size:10px; }
.frag-cards.compact { gap:8px; }
.frag-cards.compact .frag-card { width:132px; padding:6px; border-radius:10px; }
.frag-cards.compact .frag-thumb { border-radius:6px; }
.frag-cards.compact .frag-meta { margin-top:4px; }
.frag-cards.compact .frag-title { font-size:11px; }
.frag-cards.compact .frag-init { margin-top:4px; padding:2px 8px; font-size:11px; }
.frag-cards.compact .frag-state { display:inline-flex; top:10px; left:10px; }
</style>
