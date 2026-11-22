<template>
  <!-- 片段切换卡（只做跳转，所以做小）：缩略图 + 状态 chip 贴角 + 一行标题。
       **横排一行、超出横向滚动**（用户 2026-11-22 口径）；「2 行」那档已删 —— 列方向填充
       会把片段排成 1/3、2/4 竖着读，而且把整条撑高。 -->
  <div class="frag-cards">
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
      <!-- ⚠️ **空状态不挂 chip**：`''` 时留一枚空胶囊会占位、还会被读成"某种状态"。 -->
      <span v-if="r.state" class="chip frag-state" :class="r.chip">{{ r.state }}</span>
      <span
        v-if="r.segments > 1"
        class="chip frag-seg"
        :title="rangesHint(r.ranges)"
      >{{ r.segments }} 段</span>
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
// 精修页的片段切换卡：小卡片（只做跳转）、横排一行（父级负责展开/收起整条）
import { rangesHint } from '../../utils'
defineProps({
  rows: { type: Array, default: () => [] },
  current: { type: String, default: '' }
})
const emit = defineEmits(['goto', 'init'])
</script>

<style scoped>
/* 「N 段」标记（多段片段）：贴着状态 chip 排在它右边 */
/* ⚠️ `--pink-bg` 在 tokens.css 里**不存在**（写它=静默丢弃整条 background）⇒ 用 `--main-soft`。 */
.frag-seg { position:absolute; top:6px; left:34px; background:var(--main-soft); color:var(--pink-text); }
.frag-cards { display:grid; grid-auto-flow:column; gap:8px; overflow-x:auto; padding:2px;
  /* 列方向排列时，缺省 justify-content:normal(=stretch) 会把 auto 宽的列**撑满整行** ——
     只有两张卡时它们会被推到两端，看着"隔了老远"。夹到左端、列宽取内容宽。 */
  justify-content:start; grid-auto-columns:max-content; }
.frag-card { position:relative; width:108px; border:1.5px solid var(--line); border-radius:10px;
  padding:5px; background:var(--card); cursor:pointer; transition:.15s; }
.frag-card:hover { border-color:var(--main); transform:translateY(-1px); }
.frag-card.on { border-color:var(--main); background:var(--main-soft); box-shadow:0 4px 14px rgba(232,104,56,.25); }
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
