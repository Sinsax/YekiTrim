<template>
  <div>
    <!-- 总闸 + 生成入口 -->
    <div class="row spread mb-8" style="gap:8px">
      <div class="row" style="gap:8px">
        <span class="chip" :class="enabled ? 'g' : 'n'">注册{{ enabled ? '已开放' : '已关闭' }}</span>
        <span class="muted small">{{ openCount }} 个可用邀请码</span>
      </div>
      <div class="row" style="gap:6px">
        <button class="mini-btn" :disabled="busy" @click="toggleRegistration">
          {{ enabled ? '关闭注册' : '开启注册' }}
        </button>
        <button class="mini-btn primary" :disabled="busy" @click="openCreate">
          <Icon name="plus" :size="11" /> 生成邀请码
        </button>
      </div>
    </div>

    <div class="hint mb-8" title="一次性 token，一个码只能绑一个账号；明文存在服务端，管理员可随时复制重发。用过即标「已使用」并显示绑给了谁。被邀请人在登录页点「用邀请码注册」，填用户名/密码/邀请码即可（角色固定普通用户）。">
      邀请码一次性，一码绑一个账号
    </div>

    <div v-if="rows.length" class="invite-list">
      <div v-for="c in rows" :key="c.id" class="invite-row">
        <!-- 明文一直在这儿（管理员权限，用户要求）：随时复制再发一次 -->
        <input class="mono invite-token" :value="c.token" readonly @focus="$event.target.select()">
        <button class="mini-btn" title="复制邀请码" @click="copy(c.token)">复制</button>
        <span class="chip" :class="chipOf(c)">{{ c.used ? '已使用' : '未使用' }}</span>
        <span class="muted small invite-note" :title="c.note || ''">{{ c.note || '—' }}</span>
        <span class="muted small invite-bound" :title="c.used_at ? fmtTime(c.used_at) : ''">{{ boundTo(c) }}</span>
        <span class="muted small mono invite-time">{{ fmtTime(c.created_at) }}</span>
        <button class="mini-btn" :disabled="busy || c.used || c.revoked" title="撤销（未使用的码才需要）"
                @click="revoke(c)">撤销</button>
      </div>
    </div>
    <div v-else-if="!busy" class="muted small">还没有邀请码</div>

    <!-- 生成：备注 / 有效期 / 数量 -->
    <AppModal :show="showCreate" title="生成邀请码" @close="showCreate = false">
      <div class="row">
        <div class="flex-1 minw-120"><label>备注（给谁用，列表里认人）</label><input v-model="note" placeholder="例如：小明"></div>
        <div class="minw-90"><label>有效天数</label><input v-model.number="days" type="number" min="0" placeholder="留空=永久"></div>
        <div class="minw-90"><label>数量</label><input v-model.number="count" type="number" min="1" max="50"></div>
      </div>
      <p class="hint mt-6" title="生成后会自动开启注册开关；一个码只能绑一个账号">明文保存在服务端，可随时复制</p>
      <template #footer>
        <button class="btn" @click="showCreate = false">取消</button>
        <button class="btn btn-primary" :disabled="busy" @click="create">生成</button>
      </template>
    </AppModal>

  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api/client'
import { toast } from '../stores/toast'
import { confirmDialog } from '../stores/dialog'
import Icon from './Icon.vue'
import AppModal from './AppModal.vue'

const rows = ref([])
const enabled = ref(false)
const busy = ref(false)
const showCreate = ref(false)
const note = ref('')
const days = ref(null)
const count = ref(1)

const openCount = computed(() => rows.value.filter((r) => r.open).length)

function fmtTime(secs) {
  if (!secs) return '—'
  const d = new Date(secs * 1000)
  const p = (n) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

/** 绑定信息：谁、什么时候。未使用就显示有效期/撤销状态。 */
function boundTo(c) {
  if (c.used_at) return `绑定 ${c.used_by_name || (c.used_by ? '账号 #' + c.used_by : '已注销账号')} · ${fmtTime(c.used_at)}`
  if (c.revoked) return '已撤销'
  if (c.expires_at) return c.open ? `有效至 ${fmtTime(c.expires_at)}` : '已过期'
  return '待使用'
}
function chipOf(c) {
  if (c.used) return 'b'
  if (c.revoked) return 'r'
  return c.open ? 'g' : 'n'
}

async function load() {
  try {
    const d = await api('/api/admin/invites')
    rows.value = d?.invites || []
    enabled.value = d?.registration_enabled === true
  } catch (e) {
    toast('邀请码读取失败：' + e.message, 'err')
  }
}

function openCreate() {
  note.value = ''
  days.value = null
  count.value = 1
  showCreate.value = true
}

async function create() {
  if (busy.value) return
  busy.value = true
  try {
    const body = { count: Number(count.value) || 1 }
    if (note.value.trim()) body.note = note.value.trim()
    if (Number(days.value) > 0) body.days = Number(days.value)
    const d = await api('/api/admin/invites', { method: 'POST', body: JSON.stringify(body) })
    enabled.value = true
    showCreate.value = false
    toast(`已生成 ${(d?.codes || []).length} 个邀请码（注册已开启，列表里可随时复制）`)
    await load()
  } catch (e) {
    toast('生成失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
}

async function revoke(c) {
  const ok = await confirmDialog(`撤销 ${c.token}？撤销后这个码不能再用于注册（已经注册的账号不受影响）。`, {
    title: '撤销邀请码？',
    danger: true
  })
  if (!ok) return
  busy.value = true
  try {
    await api(`/api/admin/invites/${c.id}`, { method: 'DELETE' })
    toast('已撤销')
    await load()
  } catch (e) {
    toast('撤销失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
}

async function toggleRegistration() {
  busy.value = true
  try {
    const d = await api('/api/admin/invites/registration', {
      method: 'PUT',
      body: JSON.stringify({ enabled: !enabled.value })
    })
    enabled.value = d?.registration_enabled === true
    toast(enabled.value ? '注册已开启' : '注册已关闭')
  } catch (e) {
    toast('切换失败：' + e.message, 'err')
  } finally {
    busy.value = false
  }
}

async function copy(text) {
  try {
    await navigator.clipboard.writeText(text)
    toast('已复制')
  } catch {
    toast(text)
  }
}

onMounted(load)
</script>

<style scoped>
.invite-list { max-height:280px; overflow:auto; }
/* 允许换行：窄窗口下宁可折成两行，也不要横向撑破卡片 */
.invite-row { display:flex; align-items:center; gap:8px; padding:5px 0; flex-wrap:wrap; border-bottom:1px dashed var(--line-soft); }
.invite-row:last-child { border-bottom:none; }
/* 明文 token：等宽、可选中复制；窄屏收缩 */
.invite-token { flex:none; width:360px; max-width:38%; padding:4px 8px; font-size:11.5px; }
.invite-note { flex:1; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.invite-bound { flex:none; white-space:nowrap; }
.invite-time { flex:none; }
</style>
