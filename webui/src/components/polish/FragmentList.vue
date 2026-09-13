<template>
  <!-- chips：编辑器态的紧凑片段条（一行 chip，给「预览 + 时间轴」腾竖高） -->
  <div v-if="chips" class="frag-chips">
    <span
      v-for="(r, i) in rows"
      :key="r.name"
      class="frag-chip"
      :class="{ on: r.name === current }"
      :title="'点击切换到「' + r.title + '」'"
      @click="emit('goto', r.name)"
    >
      <b>{{ i + 1 }}</b>
      <span class="nm">{{ r.title }}</span>
      <span class="chip" :class="r.chip">{{ r.state }}</span>
      <button
        v-if="!r.initialized"
        class="mini-btn frag-chip-init"
        title="解析字幕/弹幕并创建 polish.toml"
        @click.stop="emit('init', r.name)"
      >初始化</button>
    </span>
  </div>

  <div v-else class="frag-cards">
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
      <div class="frag-meta">
        <b class="frag-title">{{ i + 1 }} · {{ r.title }}</b>
        <span class="chip" :class="r.chip">{{ r.state }}</span>
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
// 精修页的片段切换器（两种形态共用一份标记）：
//   cards = 卡片墙（列表态 / 未初始化态，带缩略图）
//   chips = 编辑器态的一行 chip（编号 + 标题 + 状态），省 ~100px 竖高给预览与时间轴
defineProps({
  rows: { type: Array, default: () => [] },
  current: { type: String, default: '' },
  chips: { type: Boolean, default: false }
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

.frag-chips { display:flex; gap:6px; overflow-x:auto; padding:2px; }
.frag-chip { flex:none; display:inline-flex; align-items:center; gap:6px; padding:2px 8px 2px 4px;
  border:1px solid var(--line); border-radius:var(--r-full); background:var(--card);
  cursor:pointer; font-size:12px; transition:.15s; }
.frag-chip:hover { border-color:var(--main); }
.frag-chip.on { border-color:var(--main); background:var(--main-soft); }
.frag-chip.on .nm { color:var(--pink-text); font-weight:600; }
.frag-chip b { width:18px; height:18px; flex:none; border-radius:50%; background:var(--bg-soft); color:var(--ink-dim);
  display:flex; align-items:center; justify-content:center; font-size:10px; }
.frag-chip.on b { background:var(--main); color:#fff; }
.frag-chip .nm { max-width:190px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.frag-chip-init { padding:1px 8px; font-size:11px; }
</style>
