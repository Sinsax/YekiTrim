<script setup>
// LlmModelCombobox — 模型选择下拉（**两处共用**）：右栏「默认模型」与链路表「模型」格。
//
// 为什么抽出来：datalist 在多数浏览器里点不开、不能分组、不能标状态；两处各写一套又会漂。
//
// 三条行为：
//  ① **聚焦先列全部**（typed=false），敲字才过滤 —— 只按输入框现值过滤 = 只能看到当前那一支。
//  ② 每条带「清单内 / 未加入」标记；点选/回车选中 emit `pick`（父层决定要不要顺手写进清单）。
//  ③ 手打一个清单里没有的 id 后**失焦/回车**也 emit `commit` —— 父层据此把它写进清单，
//     避免"想用一支新模型还要先回模型配置页加一遍"。
import { computed, ref } from 'vue'

const props = defineProps({
  modelValue: { type: String, default: '' },
  /** [{ id, label, inList }] —— inList=false = 探测到但还没进清单 */
  options: { type: Array, default: () => [] },
  placeholder: { type: String, default: '' },
  ariaLabel: { type: String, default: '模型' },
  id: { type: String, default: '' },
  /** 候选上限（太长的清单在表格里会把下拉撑爆） */
  max: { type: Number, default: 12 }
})
const emit = defineEmits(['update:modelValue', 'pick', 'commit'])

const open = ref(false)
const activeIdx = ref(0)
const typed = ref(false)
const shown = computed(() => {
  const q = typed.value ? String(props.modelValue || '').trim().toLowerCase() : ''
  const all = Array.isArray(props.options) ? props.options : []
  const hit = q ? all.filter((m) => String(m.id || '').toLowerCase().includes(q)) : all
  return hit.slice(0, props.max)
})
/** 聚焦那一刻的值：失焦时只有**改过**才 commit（否则 tab 穿一遍就会把"接口默认模型"写进清单） */
let initial = ''
function openAll() { typed.value = false; open.value = true; activeIdx.value = 0; initial = String(props.modelValue || '') }
function onInput(e) {
  emit('update:modelValue', e.target.value)
  typed.value = true
  open.value = true
  activeIdx.value = 0
}
function close() { open.value = false }
/** blur 先于 click ⇒ 延后收，别让鼠标点空；同时把**手打的值**交出去 */
function onBlur() {
  setTimeout(() => {
    if (!open.value) return
    open.value = false
  }, 120)
  const v = String(props.modelValue || '').trim()
  if (v && v !== initial) emit('commit', v)
}
function move(step) {
  if (!open.value) { openAll(); return }
  const n = shown.value.length
  if (!n) return
  activeIdx.value = (activeIdx.value + step + n) % n
}
/** Esc：只有**下拉开着**时才拦（先收下拉，再按一次才轮到底下的"取消编辑"） */
function onEsc(e) {
  if (!open.value) return
  e.stopPropagation()
  close()
}
function choose(m) {
  if (!m) return
  emit('update:modelValue', String(m.id || ''))
  emit('pick', m)
  close()
}
function onEnter() {
  if (!open.value) { openAll(); return }
  const m = shown.value[activeIdx.value]
  if (m) choose(m)
  else { close(); const v = String(props.modelValue || '').trim(); if (v) emit('commit', v) }
}
</script>

<template>
  <div class="mc">
    <input :id="id || undefined" :value="modelValue" type="text" spellcheck="false" autocomplete="off"
           :aria-label="ariaLabel" :placeholder="placeholder"
           @input="onInput" @focus="openAll" @blur="onBlur"
           @keydown.down.prevent="move(1)" @keydown.up.prevent="move(-1)"
           @keydown.enter.prevent="onEnter" @keydown.esc="onEsc">
    <ul v-if="open && shown.length" class="mc-sug" role="listbox" :aria-label="ariaLabel + '候选'">
      <li v-for="(m, i) in shown" :key="m.id" role="option"
          :class="{ on: i === activeIdx }" :aria-selected="i === activeIdx ? 'true' : 'false'"
          @mousedown.prevent="choose(m)" @mouseenter="activeIdx = i">
        <span class="mc-id">{{ m.id }}</span>
        <span class="mc-tag" :class="{ add: !m.inList }">{{ m.inList ? '清单内' : '未加入' }}</span>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.mc { position:relative; min-width:0; flex:1 1 auto; }
.mc > input { width:100%; min-width:0; }
.mc-sug { position:absolute; z-index:30; left:0; right:0; top:calc(100% + 2px); margin:0; padding:3px;
  list-style:none; max-height:240px; overflow:auto; background:var(--card);
  border:1px solid var(--line); border-radius:var(--r-sm); box-shadow:var(--e-2); }
.mc-sug li { display:flex; align-items:center; gap:8px; min-height:26px; padding:2px 6px;
  border-radius:6px; cursor:pointer; font-size:12px; }
.mc-sug li.on { background:var(--bg-soft); }
.mc-id { flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  font-family:var(--font-mono, monospace); }
.mc-tag { flex:none; font-size:9.5px; line-height:1; padding:3px 5px; border-radius:999px;
  background:var(--bg-soft); border:1px solid var(--line-soft); color:var(--ink-faint); }
.mc-tag.add { background:var(--main-soft); border-color:var(--main-glow); color:var(--pink-text); }
</style>
