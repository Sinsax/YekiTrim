<script setup>
// LlmProviders — 「高级配置 → 接口管理」：**只做清单与归属**（增 / 删 / 改名 / 换协议 / 设为当前）。
//
// ⚠️ **不再编辑连接字段**（地址 / 密钥 / 模型）：它们在「① 当前接口」里，而且**只有当前那一份**能编
//    （同一件事说两遍是这一轮重构要消灭的形状之一）。要改某一份的地址：先「设为当前」再回上面改。
// ⚠️ **没有模型弹层**（2026-11 用户口径）：模型只有两个入口 —— ① 选当前模型、④ 模型列表管清单。
//    历史：这里曾有行内 popover + 「管理模型」+ 已选/API 侧双列，三处弹层是"模型选择三处入口"的病根。
// ⚠️ 不落盘：本组件原地改父层的 reactive 数组，保存仍走父层的 saveLlm（buildLlmPatch + PUT + 回读）。
import Icon from '../Icon.vue'
import { protocolLabel } from './protocols.js'

const props = defineProps({
  /** 父层 reactive 的 `form.llm.providers`（本组件原地改 name / protocol） */
  providers: { type: Array, required: true },
  /** 当前在用的接口 id（= [llm] active） */
  activeId: { type: String, default: '' },
  /** API 协议清单（后端 /config/llm/protocols；拿不到时父层给内置清单） */
  protocols: { type: Array, default: () => [] },
  /** 'server' | 'builtin' —— 界面上照实标清单来源 */
  protocolsSource: { type: String, default: '' }
})
const emit = defineEmits(['update:activeId', 'add', 'remove'])
</script>

<template>
  <div class="pv">
    <div v-for="p in providers" :key="p.id" class="pv-row" :data-pv-id="p.id">
      <input class="pv-active" type="radio" :checked="activeId === p.id" title="设为当前使用这一份"
             @change="emit('update:activeId', p.id)">
      <input class="pv-name-input" v-model="p.name" :placeholder="p.id" :title="'显示名（id = ' + p.id + '）'">
      <select class="pv-proto" :value="p.protocol || (protocols[0] && protocols[0].id) || ''"
              :title="'请求怎么发（协议注册表）。清单来源：' + (protocolsSource === 'server' ? '后端' : '内置')"
              @change="p.protocol = $event.target.value">
        <option v-for="o in protocols" :key="o.id" :value="o.id">{{ o.label || o.id }}</option>
      </select>
      <span class="pv-url" :title="p.base_url || '还没填地址'">{{ p.base_url || '（未填地址）' }}</span>
      <span class="pv-dot" :class="{ on: !!p.api_key_set }"
            :title="p.api_key_set ? '已配置 API 密钥' : '还没配 API 密钥'"></span>
      <span class="pv-spacer"></span>
      <button class="pv-ico danger" title="删除这一份接口（它的模型也会一并从清单里摘掉）"
              @click="emit('remove', p.id)"><Icon name="trash" :size="12" /></button>
    </div>

    <div v-if="!providers.length" class="pv-note">还没有接口。新建一份，再回上面「当前接口」填地址与密钥。</div>

    <div class="pv-foot">
      <button class="pv-mini" title="再加一份独立接口（各带自己的地址 / 密钥 / 模型）"
              @click="emit('add')">+ 新建接口</button>
      <span class="pv-spacer"></span>
      <span class="pv-muted"
            :title="protocolsSource === 'server' ? '协议清单来自后端注册表' : '协议清单是内置的（后端还没提供）'">
        协议清单：{{ protocolsSource === 'server' ? '后端' : '内置' }}（{{ protocols.length }} 条）
      </span>
    </div>
  </div>
</template>

<style scoped>
/* 紧凑口径：一行一件事、行高 ≤32px，次要项（地址）只读显示 + title 看全文 */
.pv { display:flex; flex-direction:column; gap:2px; min-width:0; }
.pv-row {
  display:flex; align-items:center; gap:6px; height:32px; min-width:0;
  padding:0 4px; border-radius:var(--r-sm); border:1px solid transparent;
}
.pv-row:hover { background:var(--bg-soft); }
/* ⚠️ app.css 里 `input,select{width:100%}` 对 **radio** 也生效（只有 checkbox 被豁免）——
   不写 width:auto 的话它撑满整行、把同一排的控件全推到右边外（实测：行 1254px 里 radio 占 1250px）。 */
.pv-active { margin:0; flex:none; width:auto; min-width:13px; height:auto; }
.pv-name-input { flex:0 1 200px; max-width:220px; min-width:80px; height:24px; padding:2px 6px;
  font-size:12px; font-weight:600; }
.pv-proto { flex:none; width:130px; height:24px; padding:1px 4px; font-size:11.5px; }
.pv-url { flex:1 1 auto; min-width:0; font-size:11px; color:var(--ink-faint);
  overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.pv-dot { flex:none; width:7px; height:7px; border-radius:50%; background:var(--ink-faint); }
.pv-dot.on { background:#3fa46a; }
.pv-spacer { flex:1 1 auto; min-width:4px; }
.pv-ico { flex:none; width:22px; height:22px; padding:0; display:inline-flex; align-items:center;
  justify-content:center; border:1px solid transparent; background:transparent; border-radius:var(--r-sm);
  color:var(--ink-dim); cursor:pointer; }
.pv-ico:hover { background:var(--card); border-color:var(--line); color:var(--ink); }
.pv-ico.danger:hover { color:var(--pink-text); border-color:var(--pink-text); }
.pv-foot { display:flex; align-items:center; gap:8px; min-height:30px; min-width:0; padding:0 4px; }
.pv-mini { flex:none; height:24px; padding:0 7px; font-size:11.5px; border-radius:var(--r-sm);
  border:1px solid var(--line); background:var(--card); color:var(--ink); cursor:pointer; }
.pv-mini:hover { border-color:var(--main); color:var(--main); }
.pv-muted { font-size:11px; color:var(--ink-faint); min-width:0; overflow:hidden;
  text-overflow:ellipsis; white-space:nowrap; }
.pv-note { font-size:11px; line-height:1.5; color:var(--ink-dim); padding:0 4px 2px; overflow-wrap:anywhere; }
</style>
