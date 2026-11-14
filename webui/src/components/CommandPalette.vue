<template>
  <Teleport to="body">
    <div v-if="show" class="cmdk-mask" @click.self="close">
      <div
        ref="rootRef"
        class="cmdk"
        role="dialog"
        aria-modal="true"
        aria-label="快速跳转"
        tabindex="-1"
      >
        <div class="cmdk-head">
          <Icon name="search" :size="15" />
          <input
            ref="inputRef"
            v-model="q"
            class="cmdk-input"
            type="text"
            placeholder="跳到项目或页面…"
            autocomplete="off"
            spellcheck="false"
            aria-label="搜索项目或页面"
            @keydown.down.prevent="move(1)"
            @keydown.up.prevent="move(-1)"
            @keydown.enter.prevent="go(active)"
          >
          <UiKbd label="Esc" />
        </div>

        <div class="cmdk-list" role="listbox">
          <button
            v-for="(it, i) in list"
            :key="it.key"
            type="button"
            role="option"
            :aria-selected="i === active ? 'true' : 'false'"
            class="cmdk-item"
            :class="{ on: i === active }"
            @mousemove="active = i"
            @click="go(i)"
          >
            <Icon :name="it.icon" :size="14" class="cmdk-ic" />
            <span class="cmdk-label">{{ it.label }}</span>
            <span v-if="it.hint" class="cmdk-hint">{{ it.hint }}</span>
          </button>
          <div v-if="!list.length" class="cmdk-empty">没有匹配项</div>
        </div>

        <div class="cmdk-foot">
          <span><UiKbd label="↑" /><UiKbd label="↓" /> 选择</span>
          <span><UiKbd label="Enter" /> 打开</span>
          <span><UiKbd label="Esc" /> 关闭</span>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<script setup>
// 全局快速跳转（⌘K / Ctrl+K）。
//
// 为什么值得单独做一个：项目一多，侧栏滚动找项目比"敲两个字"慢一个数量级；
// 而且它把「首页 / 设置 / 入库」这类全局入口从"记住位置"变成"搜索即可"。
//
// ⚠️ 只读 store、只做跳转 —— 不发请求、不改任何状态（除了自己的输入框）。
// ⚠️ Esc / 焦点圈定 / 滚动锁 走 `useOverlay`，别在这里另写一份。
import { computed, nextTick, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import Icon from './Icon.vue'
import UiKbd from './ui/UiKbd.vue'
import { useOverlay } from '../composables/useOverlay'
import { wb } from '../stores/workbench'
import { nextStep, STEPS } from '../utils'

const props = defineProps({ show: { type: Boolean, default: false } })
const emit = defineEmits(['close'])

const router = useRouter()
const rootRef = ref(null)
const inputRef = ref(null)
const q = ref('')
const active = ref(0)

/** 项目下一步（与首页「项目」行同一套判据：缓存里没 status 就按 stage 粗判） */
function stepOf(p) {
  const n = nextStep(p, null)
  return n?.step || 'prepare'
}
function stepLabel(p) {
  const key = stepOf(p)
  return STEPS.find((s) => s.key === key)?.name || '预处理'
}

const PAGES = computed(() => {
  const list = [
    { key: 'page-workbench', label: '首页', icon: 'home', to: '/workbench', hint: '工作台' },
    // 一级文案按术语表用「采集」，旧词「入库」进提示（docs/plan/webui-simplification-2026-11.md）
    { key: 'page-ingest', label: '采集', icon: 'download', to: '/ingest', hint: '入库 · 录制导入' }
  ]
  // 普通账号也能进「处理 / AI 模型」两个子页（系统 / 接入仍 admin），设置入口因此对所有人可见
  list.push({ key: 'page-settings', label: '设置', icon: 'settings', to: '/settings', hint: '处理 / AI 模型 / 接入 / 系统' })
  return list
})

const all = computed(() => {
  const projects = (wb.projects || []).map((p) => ({
    key: `proj-${p.id}`,
    label: p.name || `项目 #${p.id}`,
    icon: 'folder',
    hint: stepLabel(p),
    to: `/p/${encodeURIComponent(p.id)}/${stepOf(p)}`
  }))
  return [...PAGES.value, ...projects]
})

const list = computed(() => {
  const kw = q.value.trim().toLowerCase()
  if (!kw) return all.value
  return all.value.filter((it) => String(it.label).toLowerCase().includes(kw)
    || String(it.hint || '').toLowerCase().includes(kw))
})

// 结果集变了就把光标拨回第一项（否则它会停在一个不存在的下标上）
watch(list, () => { active.value = 0 })

watch(() => props.show, async (v) => {
  if (!v) return
  q.value = ''
  active.value = 0
  await nextTick()
  inputRef.value?.focus()
})

function move(d) {
  const n = list.value.length
  if (!n) return
  active.value = (active.value + d + n) % n
}

function go(i) {
  const it = list.value[i]
  if (!it) return
  emit('close')
  router.push(it.to).catch(() => {})
}

function close() {
  emit('close')
}

useOverlay({
  isOpen: computed(() => props.show),
  onClose: close,
  rootRef,
  initialFocus: () => inputRef.value
})
</script>

<style scoped>
.cmdk-mask { position:fixed; inset:0; z-index:90; background:var(--scrim);
  display:flex; align-items:flex-start; justify-content:center; padding:12vh 16px 16px; }
.cmdk { width:min(560px,100%); max-height:min(60vh,520px); display:flex; flex-direction:column;
  background:var(--card); border:1px solid var(--line); border-radius:var(--r-lg);
  box-shadow:var(--e-3); overflow:hidden; animation:cmdk-in var(--dur-base) var(--ease); }
@keyframes cmdk-in { from { opacity:0; transform:translateY(-8px) scale(.99); } to { opacity:1; transform:none; } }
.cmdk-head { display:flex; align-items:center; gap:8px; padding:12px 14px;
  border-bottom:1px solid var(--line-soft); color:var(--ink-faint); }
.cmdk-input { flex:1; min-width:0; border:none; background:none; outline:none;
  font-size:var(--fs-md); color:var(--ink); padding:2px 0; }
.cmdk-input::placeholder { color:var(--ink-faint); }
.cmdk-list { flex:1; min-height:0; overflow-y:auto; padding:6px; }
.cmdk-item { display:flex; align-items:center; gap:9px; width:100%; padding:8px 10px;
  border:none; background:none; border-radius:var(--r-sm); text-align:left; cursor:pointer;
  color:var(--ink); font-size:var(--fs-base); }
.cmdk-item.on { background:var(--surface-2); }
.cmdk-ic { flex:none; color:var(--ink-faint); }
.cmdk-item.on .cmdk-ic { color:var(--pink-text); }
.cmdk-label { flex:1; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.cmdk-hint { flex:none; font-size:var(--fs-xs); color:var(--ink-faint); }
.cmdk-empty { padding:24px; text-align:center; font-size:var(--fs-sm); color:var(--ink-faint); }
.cmdk-foot { display:flex; gap:14px; padding:8px 14px; border-top:1px solid var(--line-soft);
  background:var(--surface-2); font-size:var(--fs-xs); color:var(--ink-faint); }
.cmdk-foot span { display:inline-flex; align-items:center; gap:4px; }
@media (max-width:640px) {
  .cmdk-mask { padding:8vh 10px 10px; }
  .cmdk-foot { display:none; }
}
@media (prefers-reduced-motion: reduce) { .cmdk { animation:none; } }
</style>
