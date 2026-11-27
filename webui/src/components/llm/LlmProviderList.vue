<script setup>
// LlmProviderList — 「模型配置」子页**左栏**：接口（provider）清单。
//
// 用户口径（2026-11-26）：「模型配置必须要能配置模型，所有东西都裹在折叠里，每次添加模型都要翻找」——
// 于是这一页改成左右结构：左 = 接口清单（这份组件），右 = 该接口的连接信息 + 它的模型清单。
//
// 三条纪律：
//  ① 点一行 = **选它来编辑**（emit select），**不改 [llm] active**。"当前使用"是另一件事，
//     由行尾那枚「用」徽标 + 右栏的「设为当前使用」按钮表达 —— 以前新建一份接口会把 active 一起改掉，
//     等于"加个备用接口顺手把全局换了"（真踩过的坑）。
//  ② 行里只放**一眼要判断的**三样：名字、地址 host、有没有密钥；模型数给一个，别的进 title。
//  ③ 删除不在这里 —— 删除会连带摘模型/清链路，放右栏底部并二次确认（别让一行悬停就能毁掉配置）。
const props = defineProps({
  /** 父层 reactive 的 `form.llm.providers` */
  providers: { type: Array, default: () => [] },
  /** 父层 reactive 的 `form.llm.models`（只用来数"这一份有几支模型"） */
  models: { type: Array, default: () => [] },
  /** 正在编辑哪一份（本地状态，不落盘） */
  editingId: { type: String, default: '' },
  /** 当前生效的是哪一份（= [llm] active） */
  activeId: { type: String, default: '' }
})
defineEmits(['select', 'add'])

/** base_url → host（只给一眼能认出来的那一截，路径与协议头都去掉） */
function hostOf(url) {
  const s = String(url || '').trim()
  if (!s) return ''
  return s.replace(/^[a-z]+:\/\//i, '').split(/[/?#]/)[0]
}
const countFor = (id) => (props.models || []).filter((m) => String(m.provider || '') === String(id || '')).length
</script>

<template>
  <div class="pl">
    <div class="pl-head">
      <b>接口</b>
      <span class="pl-n">{{ providers.length }} 份</span>
    </div>

    <div class="pl-list" role="listbox" aria-label="接口清单">
      <button v-for="p in providers" :key="p.id" type="button" class="pl-row"
              :class="{ on: p.id === editingId }" role="option"
              :aria-selected="p.id === editingId ? 'true' : 'false'"
              :title="(p.name || p.id) + ' · ' + (p.base_url || '还没填地址')"
              @click="$emit('select', p.id)">
        <span class="pl-main">
          <span class="pl-name">{{ p.name || p.id }}</span>
          <span class="pl-sub">{{ hostOf(p.base_url) || '还没填地址' }}</span>
        </span>
        <span class="pl-side">
          <span v-if="p.id === activeId" class="pl-using" title="当前使用：没在「哪个任务用哪支」里单独指定的任务都用它">用</span>
          <span class="pl-dot" :class="{ on: !!p.api_key_set }"
                :title="p.api_key_set ? '已配密钥' : '还没配密钥'"></span>
          <span class="pl-cnt">{{ countFor(p.id) }} 支</span>
        </span>
      </button>
      <div v-if="!providers.length" class="pl-empty">还没有接口 —— 点下面「+ 新建接口」</div>
    </div>

    <button type="button" class="pl-add" title="新建一份接口（只切到编辑，不改变当前使用的那份）"
            @click="$emit('add')">+ 新建接口</button>
  </div>
</template>

<style scoped>
.pl { display:flex; flex-direction:column; gap:6px; min-width:0; }
.pl-head { display:flex; align-items:baseline; gap:6px; padding:0 2px; }
.pl-head b { font-size:13px; font-weight:700; }
.pl-n { font-size:10.5px; color:var(--ink-faint); }
.pl-list { display:flex; flex-direction:column; gap:2px; min-width:0; }
/* 一行两份信息：上面名字、下面 host；右侧是状态（用 / 密钥 / 模型数） */
.pl-row { display:flex; align-items:center; gap:6px; width:100%; min-width:0; text-align:left;
  padding:6px 8px; border-radius:var(--r-sm); border:1px solid transparent; background:transparent;
  color:var(--ink); cursor:pointer; }
.pl-row:hover { background:var(--bg-soft); }
.pl-row.on { background:var(--main-soft); border-color:var(--main-glow); }
.pl-main { display:flex; flex-direction:column; gap:1px; min-width:0; flex:1 1 auto; }
.pl-name { font-size:12.5px; font-weight:600; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.pl-row.on .pl-name { color:var(--pink-text); }
.pl-sub { font-size:10.5px; color:var(--ink-faint); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.pl-side { display:flex; align-items:center; gap:5px; flex:none; }
.pl-using { font-size:9.5px; line-height:1; padding:3px 5px; border-radius:999px;
  background:var(--green-soft, var(--bg-soft)); color:var(--green-deep); font-weight:600; }
.pl-dot { width:6px; height:6px; border-radius:50%; background:var(--ink-faint); }
.pl-dot.on { background:#3fa46a; }
.pl-cnt { font-size:10px; color:var(--ink-faint); white-space:nowrap; }
.pl-empty { font-size:11px; color:var(--ink-dim); padding:8px; border:1px dashed var(--line-soft);
  border-radius:var(--r-sm); line-height:1.5; }
.pl-add { align-self:stretch; height:28px; font-size:12px; border-radius:var(--r-sm);
  border:1px dashed var(--line); background:var(--card); color:var(--ink-dim); cursor:pointer; }
.pl-add:hover { border-color:var(--main); color:var(--main); border-style:solid; }
</style>
