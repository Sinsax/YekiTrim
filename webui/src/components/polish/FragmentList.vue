<template>
  <!-- 片段切换卡（只做跳转，所以做小）：缩略图 + 状态 chip 贴角 + 一行标题；
       cardRows=1 一行可横向滚，=2 折成两行（列方向填充，仍横向滚） -->
  <div class="frag-cards" :class="'rows-' + cardRows">
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
      <span class="chip frag-state" :class="r.chip">{{ r.state }}</span>
      <b class="frag-title">{{ i + 1 }} · {{ r.title }}</b>
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
// 精修页的片段切换卡：小卡片（只做跳转）、可 1 行/2 行显示（父级给 cardRows）
defineProps({
  rows: { type: Array, default: () => [] },
  current: { type: String, default: '' },
  /** 显示几行：1（默认，横向滚动）| 2（折两行，仍横向滚动） */
  cardRows: { type: Number, default: 1 }
})
const emit = defineEmits(['goto', 'init'])
</script>

<style scoped>
.frag-cards { display:grid; grid-auto-flow:column; gap:8px; overflow-x:auto; padding:2px;
  /* 列方向排列时，缺省 justify-content:normal(=stretch) 会把 auto 宽的列**撑满整行** ——
     只有两张卡时它们会被推到两端，看着"隔了老远"。夹到左端、列宽取内容宽。 */
  justify-content:start; grid-auto-columns:max-content; }
.frag-cards.rows-1 { grid-template-rows:auto; }
.frag-cards.rows-2 { grid-template-rows:repeat(2, auto); }
.frag-card { position:relative; width:108px; border:1.5px solid var(--line); border-radius:10px;
  padding:5px; background:var(--card); cursor:pointer; transition:.15s; }
.frag-card:hover { border-color:var(--main); transform:translateY(-1px); }
.frag-card.on { border-color:var(--main); background:var(--main-soft); box-shadow:0 4px 14px rgba(216,99,83,.25); }
.frag-thumb { width:100%; aspect-ratio:16/9; object-fit:cover; border-radius:6px;
  background:var(--bg-soft); display:block; }
.frag-thumb-empty { display:flex; align-items:center; justify-content:center;
  color:var(--ink-faint); font-size:10px; }
/* 状态 chip 压在缩略图左上角：省掉一整行的竖高 */
.frag-state { position:absolute; top:8px; left:8px; font-size:9.5px; padding:1px 7px; }
.frag-title { display:block; margin-top:4px; font-size:10.5px; line-height:1.5;
  overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.frag-card.on .frag-title { color:var(--pink-text); }
.frag-init { margin-top:4px; width:100%; padding:1px 6px; font-size:10.5px; }
</style>
