<template>
  <div>
    <div class="row spread mb-8">
      <span class="muted small">
        {{ rows.length }} 个账号 · 按已登记的项目占用统计
      </span>
      <button class="mini-btn" :disabled="busy" @click="load">刷新</button>
    </div>

    <div class="hint mb-8" title="0 表示跟随「程序配置」里的全局默认配额；账号级与全局级都是 0 时才真的不限。留空保存 = 恢复跟随全局。配额按已登记项目的源录播 + 产物统计，不含手工丢进项目目录的文件。">
      配额按已登记项目统计，0 = 跟随全局默认
    </div>

    <div v-if="rows.length" class="quota-list">
      <div v-for="u in rows" :key="u.id" class="quota-row">
        <span class="quota-name" :title="u.username">{{ u.username }}</span>
        <span class="chip" :class="u.role === 'admin' ? 'g' : 'n'">{{ u.role === 'admin' ? '管理员' : '普通' }}</span>
        <span v-if="u.disabled" class="chip n">已禁用</span>
        <!-- 管理员不受配额约束（他要能收拾一个已超额的账号），所以直接说明而不是给个改不动的框 -->
        <template v-if="u.role === 'admin'">
          <span class="muted small quota-note" title="管理员豁免配额：否则没人能清理一个已经超额的账号">不受配额限制</span>
        </template>
        <template v-else>
          <input
            v-model="draft[u.id]"
            class="mono quota-input"
            :placeholder="String(defaultQuota)"
            :disabled="busy"
            title="字节数；留空或 0 = 跟随全局默认"
            @keyup.enter="save(u)"
          >
          <span class="muted small quota-unit">字节</span>
          <span class="muted small quota-cur" :title="'当前值：' + fmtBytes(u.quota_bytes)">
            {{ u.quota_bytes > 0 ? fmtBytes(u.quota_bytes) : '跟随全局' }}
          </span>
          <button class="mini-btn primary" :disabled="busy || !dirty(u)" @click="save(u)">保存</button>
        </template>
      </div>
    </div>
    <div v-else-if="!busy" class="muted small">还没有账号</div>
    <div v-else class="muted small">读取中…</div>
  </div>
</template>

<script setup>
// 账号配额管理（**仅 admin**）。
//
// ⚠️ 服务端的 0 语义是「**跟随全局默认**」，不是「不限」—— 文案不能写成"0 = 无限"，
// 那是两个不同的承诺。只有账号级与全局级都为 0 时才真的不限（见
// `yekitrim-service/src/projects.rs::effective_quota_bytes`）。
//
// ⚠️ 管理员**豁免**配额（`check_capacity` 与手工建项目那条路都跳过 admin）：
// 他要能删项目来收拾一个已经超额的账号。所以管理员那行不给输入框，直接说明。
import { ref, reactive, onMounted } from 'vue'
import { authApi } from '../api/auth'
import { toast } from '../stores/toast'

const rows = ref([])
const busy = ref(false)
// 草稿按账号 id 存（键是 id 不是用户名 —— 改名不该串台）
const draft = reactive({})
// 全局默认配额：只为给输入框一个 placeholder 提示"跟随落在哪个数上"。
// ⚠️ 拿不到就显示 0（= 不限），**不要**因为读不到就编一个数出来。
const defaultQuota = ref(0)

/** 人类可读字节（与服务端 `disk::human_bytes` 同口径，5 档）。 */
function fmtBytes(n) {
  const v = Number(n) || 0
  if (v < 1024) return `${v} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let x = v
  let i = -1
  while (x >= 1024 && i + 1 < units.length) {
    x /= 1024
    i += 1
  }
  return `${x.toFixed(1)} ${units[i]}`
}

async function load() {
  busy.value = true
  try {
    const d = await authApi.listUsers()
    rows.value = Array.isArray(d?.users) ? d.users : []
    for (const u of rows.value) {
      draft[u.id] = u.quota_bytes > 0 ? String(u.quota_bytes) : ''
    }
  } catch (e) {
    toast(`读取账号失败：${e.message}`, 'err')
  } finally {
    busy.value = false
  }
  // 全局默认另读一次（读不到就保持 0，不编数）
  try {
    const cfg = await (await import('../api/client')).api('/api/config')
    const q = Number(cfg?.global_config?.storage?.default_quota_bytes)
    defaultQuota.value = Number.isFinite(q) && q > 0 ? q : 0
  } catch {
    defaultQuota.value = 0
  }
}

function dirty(u) {
  const cur = u.quota_bytes > 0 ? String(u.quota_bytes) : ''
  return String(draft[u.id] ?? '').trim() !== cur
}

async function save(u) {
  const raw = String(draft[u.id] ?? '').trim()
  // 留空 = 回到「跟随全局」= 0。**不把空当 0 字节的硬配额**。
  const n = raw === '' ? 0 : Number(raw)
  if (!Number.isInteger(n) || n < 0) {
    toast('配额要填非负整数（留空 = 跟随全局）', 'err')
    return
  }
  busy.value = true
  try {
    const d = await authApi.setQuota(u.username, n)
    u.quota_bytes = Number(d?.quota_bytes ?? n)
    draft[u.id] = u.quota_bytes > 0 ? String(u.quota_bytes) : ''
    toast(`已保存 ${u.username} 的配额`, 'ok')
  } catch (e) {
    toast(`保存失败：${e.message}`, 'err')
  } finally {
    busy.value = false
  }
}

onMounted(load)
</script>

<style scoped>
.quota-list { display: flex; flex-direction: column; }
.quota-row {
  display: flex; align-items: center; gap: 8px;
  padding: 6px 0; border-top: 1px solid var(--line-soft);
}
.quota-row:first-child { border-top: 0; }
/* 名字给固定一份宽度，后面的控件才会对齐成一列 */
.quota-name { flex: 0 0 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.quota-input { width: 110px; flex: none; }
.quota-unit { flex: none; }
.quota-cur { flex: 0 0 90px; text-align: right; }
.quota-note { flex: 1 1 auto; }
</style>
