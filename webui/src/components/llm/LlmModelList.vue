<script setup>
// LlmModelList — 「模型配置」右栏下半：**某一个接口**的模型清单。
//
// 用户口径（2026-11-27，第三次收敛）：
//  · 按钮叫「**获取模型**」；拿到的候选**显示在表格上面**（在模型行/删除之前），点「加入」才落盘。
//  · 拿到之后模型 id 那一格是**可下拉的输入框**（`<datalist>`）—— 能选已有的，也**永远能手打**；
//    获取失败就**行内照实说**，手打这条照旧可用（两条路都得在）。
//  · 每行**只有 id + 显示名**：档位 / 思考强度 / 输入模态 / 上下文 / 额外参数/overrides 全部不在这里
//    （前几项归链路那边；额外参数与 overrides 已按用户口径整块删掉）。
//
// 两条硬口径（沿用 L3）：
//  ① 获取**只填建议、不改生效值**：候选进上面那块，点「加入」才写进清单（保存后才落 llm.toml）。
//  ② **清单里的值说了算**：链路只能从这张表里选。
//
// ⚠️ models 是父层的 reactive **全量数组**（本组件按 provider 过滤后原地增删改），保存仍走父层的 saveLlm。
// ⚠️ **别把探针的 caps 写进 `catalog`**：catalog 是清单形状（input = ["text"] / levels = [{level,value}]），
//    caps 是能力面（input = {text=true}）—— 写进去文件合法、**反序列化才炸**，整份 llm.toml 会被拒收
//    （2026-11-27 真机事故）。现在这条路径整个不存在了。
import { computed, reactive, ref } from 'vue'
import Icon from '../Icon.vue'
import { confirmDialog } from '../../stores/dialog'
import { MODEL_SOURCE_ZH, fetchModels } from '../../api/llm'

const props = defineProps({
  /** 父层 reactive 的 `form.llm.models`（**全量**，本组件按 providerId 过滤） */
  models: { type: Array, required: true },
  /** 这一栏画哪一份接口的模型 */
  providerId: { type: String, default: '' },
  /** 该接口的地址 / 草稿密钥（探针用：没保存的也能探） */
  baseUrl: { type: String, default: '' },
  apiKey: { type: String, default: '' }
})
const emit = defineEmits(['probe'])

/** 这一份接口名下的行（同一个对象引用 —— 删除时用 indexOf 找回全量里的位置） */
const rows = computed(() => props.models.filter((m) => String(m.provider || '') === String(props.providerId || '')))
/** 过滤词（按 id / 显示名，大小写不敏感） */
const filter = ref('')
const shown = computed(() => {
  const q = String(filter.value || '').trim().toLowerCase()
  const list = rows.value
  return q ? list.filter((m) => String(m.id || '').toLowerCase().includes(q) || String(m.label || '').toLowerCase().includes(q)) : list
})

/** 获取结果：{ state: idle|loading|done|error, models: [], note } —— **失败 ≠ 空列表** */
const remote = reactive({ state: 'idle', models: [], note: '' })
const fetching = ref(false)
/** datalist 的 id：每个接口一份，别让两份接口的候选串台 */
const listId = computed(() => 'ml-cand-' + (String(props.providerId || '') || 'x'))
const chosen = (id) => rows.value.some((m) => String(m.id || '').trim() === String(id || '').trim())

/** 「获取模型」：按**这一份接口**的地址探一次（表命中不联网；表外才真发） */
async function doFetch() {
  if (fetching.value) return
  const base = String(props.baseUrl || '').trim()
  if (!base) { remote.state = 'error'; remote.note = '先在这份接口上面把 API 地址填好'; return }
  fetching.value = true
  remote.state = 'loading'
  remote.note = ''
  try {
    const r = await fetchModels(base, props.apiKey, props.providerId)
    remote.state = 'done'
    remote.models = Array.isArray(r.models) ? r.models : []
    remote.note = '来源：' + (MODEL_SOURCE_ZH[r.source] || r.source) + '（' + remote.models.length + ' 支）'
      + (r.note ? ' · ' + r.note : '')
    // 能力面交给父层（连接卡的能力 chip / 链路档位候选），**不落盘**
    emit('probe', { providerId: props.providerId, models: remote.models })
  } catch (e) {
    remote.state = 'error'
    remote.models = []
    remote.note = (e && e.message) ? e.message : String(e)
  } finally {
    fetching.value = false
  }
}

/** 加入清单：只写 id / 显示名 / 归属 */
function add(id) {
  const m = (remote.models || []).find((x) => String(x.id || '') === String(id || ''))
  if (!m || chosen(m.id)) return
  props.models.push({
    id: String(m.id || '').trim(),
    provider: String(props.providerId || ''),
    label: String(m.name || '')
  })
}
/** 手填一行（获取失败 / 服务器说不清时的兜底；也是"清单里没有这一支"的正常入口） */
function addManual() {
  if (!props.providerId) return
  props.models.push({ id: '', provider: String(props.providerId || ''), label: '' })
}
async function removeModel(m) {
  const ok = await confirmDialog('从模型清单里删除「' + (m.id || '(未命名)') + '」？（保存后生效）',
    { title: '删除模型', danger: true })
  if (!ok) return
  const i = props.models.indexOf(m)
  if (i >= 0) props.models.splice(i, 1)
}
</script>

<template>
  <div class="ml">
    <div class="ml-bar">
      <span class="ml-title">模型<span class="ml-cnt">{{ rows.length }} 支</span></span>
      <span class="ml-hint" title="「哪个任务用哪支」里的候选只来自这张表">链路只能从这里选</span>
      <span class="ml-spacer"></span>
      <input v-model="filter" class="ml-filter" placeholder="过滤…" aria-label="过滤本接口的模型">
      <button class="ml-mini" type="button" :disabled="fetching || !providerId"
              title="按这个接口的地址取一次模型清单（内置目录命中就不联网）"
              @click="doFetch">{{ fetching ? '获取中…' : '获取模型' }}</button>
      <button class="ml-mini" type="button" :disabled="!providerId" title="手填一行（清单里没有这一支时用）"
              @click="addManual">+ 添加模型</button>
    </div>

    <!-- 获取结果 / 失败：都在表格**上面**一行说清（不占模型行，也不挡删除列） -->
    <div v-if="remote.state === 'loading'" class="ml-muted ml-res">获取中…</div>
    <div v-else-if="remote.state === 'error'" class="ml-err ml-res" data-ml-fetch="error">获取失败：{{ remote.note }}</div>
    <div v-else-if="remote.state === 'done'" class="ml-res ml-cands">
      <span class="ml-muted">{{ remote.note }}</span>
      <button v-for="c in remote.models" :key="c.id" class="ml-cand" type="button"
              :disabled="chosen(c.id)"
              :title="c.name ? c.id + ' · ' + c.name : c.id"
              @click="add(c.id)">{{ c.id }}<span v-if="chosen(c.id)" class="ml-muted"> · 已在清单</span></button>
      <span v-if="!remote.models.length" class="ml-muted">服务器没返回模型 —— 点「+ 添加模型」手打一行。</span>
    </div>

    <table v-if="rows.length" class="ml-tbl">
      <thead>
        <tr>
          <th class="ml-c-id">模型 id</th>
          <th class="ml-c-label">显示名</th>
          <th class="ml-c-act"></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="(m, i) in shown" :key="i">
          <!-- 拿到候选之后这里就是下拉（能选已有的）；<datalist> 同时允许**直接手打** -->
          <td>
            <input v-model.trim="m.id" class="ml-input" :list="listId"
                   placeholder="如 deepseek-chat" spellcheck="false">
          </td>
          <td><input v-model="m.label" class="ml-input" placeholder="（可空，空则用 id）"></td>
          <td class="ml-act">
            <button class="ml-ico danger" title="从清单删除（保存后生效）" @click="removeModel(m)">
              <Icon name="trash" :size="12" />
            </button>
          </td>
        </tr>
      </tbody>
    </table>
    <datalist :id="listId">
      <option v-for="c in remote.models" :key="c.id" :value="c.id">{{ c.name || c.id }}</option>
    </datalist>

    <div v-if="!rows.length" class="ml-empty">
      这一份接口还没有模型 —— 点上面「获取模型」挑一支，或「+ 添加模型」手打一行。
    </div>
  </div>
</template>

<style scoped>
.ml { display:flex; flex-direction:column; gap:8px; min-width:0; }
.ml-bar { display:flex; align-items:center; gap:8px; flex-wrap:wrap; }
.ml-title { font-size:12px; font-weight:600; }
.ml-cnt { margin-left:6px; font-size:11px; color:var(--muted); font-weight:400; }
.ml-hint { font-size:11px; color:var(--muted); }
.ml-spacer { flex:1 1 auto; }
.ml-filter { width:120px; font-size:11px; padding:3px 6px; }
.ml-mini { font-size:11px; padding:3px 8px; }
.ml-res { font-size:11px; line-height:1.6; }
.ml-cands { display:flex; align-items:center; gap:6px; flex-wrap:wrap; }
.ml-cand { font-size:11px; padding:2px 8px; border-radius:999px; border:1px solid var(--line);
  background:var(--surface-2); color:var(--fg); cursor:pointer; font-family:var(--mono, monospace); }
.ml-cand:disabled { opacity:.55; cursor:default; }
.ml-err { font-size:11px; color:var(--danger, #e5484d); }
.ml-tbl { width:100%; border-collapse:collapse; font-size:12px; }
.ml-tbl th { text-align:left; font-weight:500; font-size:11px; color:var(--muted);
  padding:4px 6px; border-bottom:1px solid var(--line); }
.ml-tbl td { padding:4px 6px; border-bottom:1px solid var(--line-dim, var(--line)); }
.ml-c-label { width:200px; }
.ml-c-act { width:44px; }
.ml-input { width:100%; font-size:12px; padding:3px 6px; }
.ml-act { text-align:right; }
.ml-ico { border:0; background:none; color:var(--muted); cursor:pointer; padding:2px 4px; border-radius:var(--r-sm); }
.ml-ico.danger:hover { color:var(--danger, #e5484d); }
.ml-empty { font-size:12px; color:var(--muted); padding:8px 2px; }
</style>
