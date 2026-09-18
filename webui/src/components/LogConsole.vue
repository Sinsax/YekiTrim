<template>
  <div>
    <!-- 工具条：级别档 + 过滤 + 自动滚动 + 清空 + 连接状态 -->
    <div class="row spread mb-8" style="gap:8px">
      <div class="row" style="gap:6px">
        <button v-for="f in LEVELS" :key="f.key" class="mini-btn"
                :class="{ selected: logState.levelKey === f.key }"
                :title="f.hint" @click="logState.levelKey = f.key">{{ f.label }}</button>
        <span class="muted small">{{ shown.length }} / {{ logState.records.length }} 条</span>
      </div>
      <div class="row" style="gap:6px">
        <input v-model="logState.keyword" class="log-filter" placeholder="过滤关键字…">
        <button class="mini-btn" :class="{ selected: logState.autoscroll }"
                title="新日志自动滚到底部（手动往上翻会自动关掉）"
                @click="logState.autoscroll = !logState.autoscroll">自动滚动</button>
        <button class="mini-btn" title="只清空这份本地缓冲" @click="clear()">
          <Icon name="trash" :size="11" /> 清空
        </button>
        <span class="chip" :class="status.cls">{{ status.text }}</span>
      </div>
    </div>

    <div ref="bodyRef" class="log-body mono" @scroll="onScroll">
      <div v-if="!shown.length" class="muted small log-blank">
        {{ logState.records.length ? '当前过滤条件下没有日志' : '暂无日志。跑个任务、或点一次「测试连接」就会冒出来。' }}
      </div>
      <div v-for="r in shown" :key="r.id" class="log-line" :class="'lv-' + lv(r.level)">
        <span class="log-time">{{ r.time }}</span>
        <span class="log-lv">{{ lv(r.level).toUpperCase() }}</span>
        <span class="log-src" :title="r.target">{{ r.source === 'server' ? (r.target || 'server') : 'webui' }}</span>
        <span class="log-text">{{ r.text }}</span>
      </div>
    </div>

    <p class="hint mt-6">
      服务端日志同时写到 <span class="mono">&lt;data_dir&gt;/logs/vtrim-YYYYMMDD.log</span>
      （可用 <span class="mono">vtrim doctor</span> 查路径）。面板默认显示「信息」以上；
      想看逐条访问日志点「调试」——高频接口（视频 / 轮询）的成功记录要
      <span class="mono">RUST_LOG=vtrim=debug</span> 启动服务端才会有。
    </p>
  </div>
</template>

<script setup>
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import Icon from './Icon.vue'
import { attach, clear, detach, levelRank, logState } from '../stores/log'

const LEVELS = [
  { key: 'all', label: '全部', hint: '不过滤（含 trace）' },
  { key: 'debug', label: '调试', hint: '含成功请求等调试细节' },
  { key: 'info', label: '信息', hint: '常规运行信息（默认）' },
  { key: 'warn', label: '警告', hint: '警告与错误' },
  { key: 'error', label: '错误', hint: '只看错误' },
]

const bodyRef = ref(null)

function lv(level) {
  const l = String(level || 'info').toLowerCase()
  return l === 'trace' ? 'debug' : l
}

const shown = computed(() => {
  const min = LEVELS.find((f) => f.key === logState.levelKey)?.key === 'all'
    ? -1
    : levelRank(logState.levelKey)
  const kw = logState.keyword.trim().toLowerCase()
  return logState.records.filter((r) => {
    if (levelRank(r.level) < min) return false
    if (!kw) return true
    return (r.text + ' ' + r.target + ' ' + r.level).toLowerCase().includes(kw)
  })
})

const status = computed(() => {
  if (!logState.available) return { cls: 'r', text: '无权限' }
  if (logState.connected) return { cls: 'g', text: '实时' }
  return { cls: 'a', text: '重连中' }
})

function scrollToBottom() {
  const el = bodyRef.value
  if (el) el.scrollTop = el.scrollHeight
}

// 新条目到达：开了自动滚动就贴底（在 nextTick 里等 DOM 更新完再滚）
watch(() => shown.value.length, () => {
  if (!logState.autoscroll) return
  nextTick(scrollToBottom)
})

/** 手动往上翻 = 不想被打断，自动关掉自动滚动；翻回底部再打开。 */
function onScroll() {
  const el = bodyRef.value
  if (!el) return
  const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24
  logState.autoscroll = atBottom
}

onMounted(async () => {
  await attach()
  nextTick(scrollToBottom)
})
onUnmounted(() => detach())
</script>

<style scoped>
/* 面板专属：定高滚动体 + 一行一条的等宽排版（样稿里没有的类，写在这里） */
.log-filter { width:180px; padding:4px 10px; font-size:12px; }
.log-body {
  height:340px; overflow:auto; background:var(--bg-soft); border:1px solid var(--line);
  border-radius:var(--r-sm); padding:8px 10px; font-size:12px; line-height:1.6;
}
.log-blank { padding:16px 4px; }
.log-line { display:flex; gap:8px; align-items:baseline; white-space:pre-wrap; word-break:break-word; }
.log-time { color:var(--ink-faint); flex:none; }
.log-lv { flex:none; width:44px; font-weight:600; }
.log-src { flex:none; max-width:96px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  color:var(--ink-faint); }
.log-text { flex:1; min-width:0; }
.lv-debug { color:var(--ink-faint); }
.lv-info .log-lv { color:var(--blue-deep); }
.lv-warn { color:var(--amber-deep); }
.lv-warn .log-lv { color:var(--amber-deep); }
.lv-error { color:var(--red-deep); }
.lv-error .log-lv { color:var(--red-deep); }
</style>
