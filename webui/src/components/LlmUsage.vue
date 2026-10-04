<template>
  <div>
    <div class="row spread mb-8" style="gap:8px">
      <div class="row" style="gap:6px">
        <button v-for="d in RANGES" :key="d" class="mini-btn" :class="{ primary: days === d }"
                :disabled="busy" @click="pick(d)">
          {{ d }} 天
        </button>
      </div>
      <button class="mini-btn" :disabled="busy" @click="load">
        <Icon name="refresh" :size="11" /> 刷新
      </button>
    </div>

    <div class="hint mb-8" title="每次 LLM 调用记一行（采集点在客户端唯一出口，重试算同一次）：链路/项目/模型/输入·输出 token/前缀缓存命中/缓存率/耗时/成败；失败也记，方便对着「运行日志」排错。缓存率 = 缓存命中 ÷ 输入 tokens（只算输入侧、按 token 加权）—— 一次任务的第一个窗口必然 0%，单发链路（封面文案等）也天然 0%，所以要按「按链路」那张表看（同一链路越高 = 公共前缀越稳）。输入没报 usage 时显示 —。">
      每次 LLM 调用记一行，失败也记
    </div>

    <div v-if="busy && !loaded" class="muted small">读取中…</div>

    <div v-else-if="!loaded && err" class="empty">
      读取失败：{{ err }}<br>
      <span class="muted small">如果这是刚更新的版本，重启一次服务端（<span class="mono">yekitrim server</span>）即可。</span>
    </div>

    <template v-else-if="totals.calls">
      <div class="usage-totals mb-8">
        <div class="usage-num"><b>{{ totals.calls }}</b><span>调用次数</span></div>
        <div class="usage-num"><b>{{ fmtInt(totals.prompt_tokens) }}</b><span>输入 tokens</span></div>
        <div class="usage-num"><b>{{ fmtInt(totals.completion_tokens) }}</b><span>输出 tokens</span></div>
        <div class="usage-num"><b>{{ fmtInt(totals.cached_tokens) }}</b><span>缓存命中 tokens</span></div>
        <div class="usage-num"><b>{{ fmtRate(totals) }}</b><span>缓存率</span></div>
        <div class="usage-num"><b>{{ avgSecs(totals) }}</b><span>平均耗时</span></div>
        <div class="usage-num"><b :class="{ warn: totals.failed > 0 }">{{ totals.failed }}</b><span>失败</span></div>
      </div>

      <div class="row mb-6" style="gap:6px">
        <button v-for="g in GROUPS" :key="g.key" class="mini-btn" :class="{ primary: group === g.key }"
                @click="group = g.key">{{ g.label }}</button>
      </div>

      <div class="tb wrap">
        <table>
          <thead>
            <tr>
              <th>{{ groupLabel }}</th>
              <th class="num">调用</th>
              <th class="num">失败</th>
              <th class="num">输入</th>
              <th class="num">输出</th>
              <th class="num">缓存命中</th>
              <th class="num" title="缓存命中 ÷ 输入 tokens（只算输入侧、按 token 加权）">缓存率</th>
              <th class="num">均耗时</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="r in rows" :key="r.key">
              <td :title="r.key">{{ r.key }}</td>
              <td class="num">{{ r.calls }}</td>
              <td class="num">{{ r.failed }}</td>
              <td class="num">{{ fmtInt(r.prompt_tokens) }}</td>
              <td class="num">{{ fmtInt(r.completion_tokens) }}</td>
              <td class="num">{{ fmtInt(r.cached_tokens) }}</td>
              <td class="num" :title="rateTitle(r)">{{ fmtRate(r) }}</td>
              <td class="num">{{ avgSecs(r) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <div class="muted small mt-6">
        统计最近 {{ days }} 天 · 数据在服务端账本（<span class="mono">llm_usage</span> 表），命令行同样可看：<span class="mono">yekitrim usage --days {{ days }}</span>
      </div>
    </template>

    <div v-else-if="!busy" class="empty">
      还没有记录。跑一次事件检测或精修 LLM（<span class="mono">yekitrim detect</span> / 精修页「LLM 剪辑」）就会有的。
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api/client'
import Icon from './Icon.vue'

const RANGES = [7, 30, 90]
const GROUPS = [
  { key: 'by_label', label: '按链路' },
  { key: 'by_project', label: '按项目' },
  { key: 'by_day', label: '按天' },
  { key: 'by_model', label: '按模型' }
]

const days = ref(30)
const group = ref('by_label')
const busy = ref(false)
const loaded = ref(false)
const err = ref('')
const summary = ref({})

const totals = computed(() => summary.value.totals || { calls: 0, failed: 0 })
const rows = computed(() => summary.value[group.value] || [])
const groupLabel = computed(() => (GROUPS.find((g) => g.key === group.value) || {}).label || '')

function fmtInt(n) {
  const v = Number(n) || 0
  return v.toLocaleString('en-US')
}
function avgSecs(g) {
  const calls = Number(g?.calls) || 0
  if (!calls) return '—'
  return (Number(g.latency_ms || 0) / 1000 / calls).toFixed(1) + 's'
}
/** 缓存率（%）= 缓存命中 ÷ 输入 tokens —— 只算输入侧、**先求和再相除**
 *  （后端给的已经是 SUM 聚合值，分母不掺输出 token）。
 *  输入为 0（老中转没报 usage、或失败行 token 全 0）时是 null，显示 — 而不是 0%。 */
function cacheRate(g) {
  const prompt = Number(g?.prompt_tokens) || 0
  if (prompt <= 0) return null
  return ((Number(g?.cached_tokens) || 0) / prompt) * 100
}
function fmtRate(g) {
  const r = cacheRate(g)
  return r == null ? '—' : r.toFixed(1) + '%'
}
function rateTitle(g) {
  const prompt = Number(g?.prompt_tokens) || 0
  if (prompt <= 0) return '这些调用没有报输入 token（老中转或失败行），算不出比率'
  return `${fmtInt(g.cached_tokens)} / ${fmtInt(prompt)} 输入 tokens 命中了前缀缓存`
}

async function load() {
  busy.value = true
  try {
    const d = await api(`/api/admin/usage?days=${days.value}`, { label: '读取 LLM 用量' })
    summary.value = d || {}
    loaded.value = true
    err.value = ''
  } catch (e) {
    // api() 已经写进日志面板了；这里只把"读不到"和"没有记录"分开显示
    err.value = (e && e.message) || '接口不可用'
  } finally {
    busy.value = false
  }
}
function pick(d) {
  days.value = d
  load()
}

onMounted(load)
</script>

<style scoped>
/* 总量块：与 InviteManager / 样稿同一套卡片与色板，只做这一页的排布 */
.usage-totals {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(104px, 1fr));
  gap: 8px;
}
.usage-num {
  background: var(--bg-soft);
  border-radius: var(--r-sm);
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.usage-num b { font-size: 16px; color: var(--ink); }
.usage-num b.warn { color: var(--red-deep); }
.usage-num span { font-size: 11px; color: var(--ink-dim); }
</style>
