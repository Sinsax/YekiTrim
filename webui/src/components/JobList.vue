<template>
  <div>
    <!-- 三态必须分开：以前"读取失败"和"真的没有任务"都渲染成「暂无任务」，
         用户既看不出是故障、也没有重试入口（那正是"队列加载不出来"的观感） -->
    <div v-if="lastError" class="empty small">
      任务列表读取失败：{{ lastError }}{{ shown.length ? '（显示的是最后一份快照）' : '' }}
      <div class="mt-6"><button class="mini-btn" @click="reload()">重试</button></div>
    </div>
    <div v-else-if="!shown.length" class="empty small">{{ loaded ? '暂无任务' : '读取中…' }}</div>
    <div v-for="j in shown" :key="j.id" class="job-row">
      <div class="row spread">
        <b>{{ kindText(j.kind) }}</b>
        <!-- title 说明档位与来源（悬停可见）；排队的细节在下行，别塞进 chip（全应用共用） -->
        <span class="chip" :class="statusClass(j.status)" :title="classTitle(j)">{{ statusText(j.status) }}</span>
      </div>
      <!-- 排队中的任务：报档位/上限/同档名次，替掉那句没信息量的 create 文案（"准备中"） -->
      <div v-if="j.status === 'queued'" class="small muted">{{ queueDetail(j) }}</div>
      <!-- 进行时行统一走 api/jobs.js::jobLine（spec §1：唯一实现，别在这里再拼一遍） -->
      <div v-else-if="showLine(j)" class="small muted">{{ jobLine(j) }}</div>
      <!-- 久无进展分两档（v2 L3）：真卡（stalled = 进度值连续 5 分钟没动）用醒目色；
           只是"没有新事件"用弱提示 —— 静默不等于卡住（检测阶段曾整条链路零事件）。 -->
      <div v-if="stuckNote(j)" class="small" :class="isStalled(j) ? 'stuck-hard' : 'muted'">
        {{ stuckNote(j) }}
      </div>
      <div v-if="j.status === 'running' && j.total > 0" class="progress mt-6">
        <div :style="{ width: Math.round(j.current * 100 / j.total) + '%' }"></div>
      </div>
      <div v-if="actions && isActive(j)" class="row mt-6">
        <button class="mini-btn danger" @click="cancel(j)">取消</button>
      </div>
    </div>
    <div v-if="limit && (jobs.value || []).length > shown.length" class="muted small mt-8">
      仅显示最近 {{ shown.length }} 条（点「查看全部」看完整历史）
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { toast } from '../stores/toast'
import { useJobs } from '../composables/useJobs'
import { classTitle, isActive, isStalled, jobLine, jobsApi, kindText, queueDetail, statusClass, statusText, stuckNote } from '../api/jobs'

const props = defineProps({
  actions: { type: Boolean, default: false },
  limit: { type: Number, default: 0 }
})
const emit = defineEmits(['changed'])

const { jobs, reload, lastError, loaded } = useJobs()
const shown = computed(() => (props.limit > 0 ? (jobs.value || []).slice(0, props.limit) : jobs.value || []))

function showLine(j) {
  const quiet = (j.status === 'succeeded' && j.message === '完成') || (j.status === 'cancelled' && j.message === '已取消')
  return !quiet
}

async function cancel(j) {
  try {
    await jobsApi.cancel(j.id)
    toast('已请求取消')
    reload()
    emit('changed')
  } catch (e) {
    toast(e.message, 'err')
  }
}
</script>

<style scoped>
/* 真卡（stalled）：进度值连续 5 分钟没动 —— 用醒目色，区别于"只是没事件"的弱提示 */
.stuck-hard { color: var(--amber-deep); }
</style>
