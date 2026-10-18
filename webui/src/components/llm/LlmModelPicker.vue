<script setup>
// LlmModelPicker — 模型选择：可搜索下拉（候选 = 接口回的 + 手打）+ 一颗「获取模型」。
//
// 三件事都要在界面上如实说（这个块存在的理由就是消灭"点了没反应"）：
//  ① 这次清单是「内置目录（不联网）」还是「联网探测（真发一次 /models）」—— 按钮上带，状态行里写全；
//  ② 探失败**显示原因**（服务端 502 的 error 原文：key 不对 / 这个地址没有 /models / 连不上）；
//  ③ 连上了但没报模型（200 + 空列表 + note）= **不是失败**，照实说，并且照样能手打。
//
// 候选由本组件自己取（api/llm.js::fetchModels）；父层只管选中的那个 id（可手打 = 自由文本）。
import { computed, ref, watch } from 'vue'
import { fetchModels, MODEL_SOURCE_ZH } from '../../api/llm'

const props = defineProps({
  /** 草稿 key（用户刚填、还没保存）—— 探针要用它，否则会拿旧 key 换一个 401。 */
  apiKey: { type: String, default: '' },
  modelValue: { type: String, default: '' },
  baseUrl: { type: String, default: '' },
  disabled: { type: Boolean, default: false }
})
const emit = defineEmits(['update:modelValue', 'fetched'])

const text = ref(String(props.modelValue || ''))
watch(() => props.modelValue, (v) => {
  const s = String(v || '')
  if (s !== text.value) text.value = s
})

const models = ref([])
const source = ref('')
const note = ref('')
const error = ref('')
const busy = ref(false)
const open = ref(false)
const active = ref(-1)

const sourceShort = computed(() => (source.value === 'probe' ? '联网探测' : (source.value === 'catalog' ? '内置目录' : '')))
const btnText = computed(() => {
  if (busy.value) return '获取中…'
  return sourceShort.value ? '获取模型（' + sourceShort.value + '）' : '获取模型'
})
const btnTitle = computed(() =>
  '按 base_url 取模型清单：内置目录命中就**不联网**；没命中才真发一次 /models')
const inputTitle = computed(() =>
  '可直接手打模型 id（候选之外的模型也能填）；下拉按输入内容过滤')

/** 过滤后的候选（空输入 = 全部）；上限 60 条，避免一次挂几百个 DOM 节点 */
const filtered = computed(() => {
  const q = text.value.trim().toLowerCase()
  const list = q
    ? models.value.filter((m) => String(m.id).toLowerCase().includes(q) || String(m.name || '').toLowerCase().includes(q))
    : models.value
  return list.slice(0, 60)
})
const visible = computed(() => (open.value ? filtered.value : []))

const statusText = computed(() => {
  if (!source.value) return ''
  const head = '来源：' + (MODEL_SOURCE_ZH[source.value] || source.value) + ' · 候选 ' + models.value.length + ' 个'
  return note.value ? head + ' · ' + note.value : head
})

function onInput(e) {
  text.value = e.target.value
  emit('update:modelValue', text.value)
  open.value = true
  active.value = -1
}
function move(d) {
  if (!filtered.value.length) return
  open.value = true
  const n = filtered.value.length
  active.value = ((active.value + d) % n + n) % n
}
function onEnter() {
  if (active.value >= 0 && filtered.value[active.value]) pick(filtered.value[active.value])
  else open.value = false
}
function pick(m) {
  text.value = String(m.id)
  emit('update:modelValue', text.value)
  open.value = false
  active.value = -1
}

async function refresh() {
  const base = String(props.baseUrl || '').trim()
  if (!base) { error.value = '先填 base_url 再获取模型'; return }
  busy.value = true
  error.value = ''
  try {
    const r = await fetchModels(base, props.apiKey)
    models.value = r.models
    source.value = r.source
    note.value = r.note || ''
    open.value = true
    emit('fetched', r)
  } catch (e) {
    // ⚠️ **候选保留**（读取失败不等于没有数据），把原因写在下面那行
    error.value = '获取失败：' + ((e && e.message) ? e.message : String(e))
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="lmp">
    <div class="lmp-row">
      <input
        class="lmp-input" type="text" role="combobox" aria-autocomplete="list"
        aria-controls="lmp-list" :aria-expanded="open ? 'true' : 'false'"
        :value="text" :disabled="disabled" :title="inputTitle"
        placeholder="模型 id（可手打）"
        @input="onInput" @focus="open = true" @blur="open = false"
        @keydown.down.prevent="move(1)" @keydown.up.prevent="move(-1)"
        @keydown.enter.prevent="onEnter" @keydown.esc="open = false">
      <button class="btn lmp-get" type="button" :disabled="disabled || busy || !baseUrl"
              :title="btnTitle" @click="refresh">{{ btnText }}</button>
    </div>

    <ul v-if="visible.length" id="lmp-list" class="lmp-list" role="listbox">
      <li v-for="(m, i) in visible" :key="m.id" class="lmp-item" role="option"
          :aria-selected="i === active ? 'true' : 'false'" :class="{ on: i === active }"
          :title="m.name ? m.id + ' · ' + m.name : m.id"
          @mousedown.prevent="pick(m)" @mousemove="active = i">
        <span class="lmp-id">{{ m.id }}</span>
        <span v-if="m.name" class="muted small lmp-name">{{ m.name }}</span>
      </li>
    </ul>

    <div v-if="error" class="lmp-err" role="alert">{{ error }}</div>
    <div v-else-if="statusText" class="muted small lmp-src" :title="statusText">{{ statusText }}</div>
  </div>
</template>

<style scoped>
.lmp { display:flex; flex-direction:column; gap:4px; min-width:0; }
.lmp-row { display:flex; align-items:center; gap:8px; min-width:0; }
.lmp-input { flex:1 1 auto; min-width:0; }
.lmp-get { flex:none; white-space:nowrap; }
/* 候选面板：定高滚动，别把下面的「生成」组顶下去 */
.lmp-list { list-style:none; margin:0; padding:4px; max-height:216px; overflow:auto;
  border:1px solid var(--line); border-radius:var(--r-sm); background:var(--card); }
.lmp-item { display:flex; align-items:baseline; gap:8px; padding:3px 6px;
  border-radius:var(--r-sm); cursor:pointer; min-width:0; }
.lmp-item.on { background:var(--main-soft); }
.lmp-id { font-size:12.5px; color:var(--ink); overflow-wrap:anywhere; }
.lmp-name { min-width:0; overflow-wrap:anywhere; }
/* 失败原因必须看得见（红色 + 常驻，不是一闪就没的 toast） */
.lmp-err { font-size:12px; line-height:1.5; color:var(--pink-text); overflow-wrap:anywhere; }
.lmp-src { overflow-wrap:anywhere; }
</style>
