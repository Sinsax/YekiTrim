<template>
  <!-- 一类输入素材（弹幕 / 字幕）的**清单**：点「切换」改生效值，点「删除」从列表移除。
       两类的结构完全相同（用户要求"显示和交互和字幕一致"），差别只有两个标记：
       - ASR 链路产物：`ASR·<mode>` 徽标 + 不可删（它代表那个模式已经用掉的一次转写名额）；
       - 外部字幕：不是本工程的 ASR 产物（弹幕不显示 —— 弹幕本来就在工程根，标出来是噪音）。
       生效值的那条钉「使用中」而不是留一个「切换」按钮（切到自己等于空操作）。 -->
  <div class="mat-blk">
    <div class="mat-head">
      <span class="mat-label">{{ label }}</span>
      <span class="muted small flex-1">{{ items.length }} 个候选</span>
      <button class="mini-btn" :title="pickTitle" @click="$emit('pick')">选择…</button>
    </div>
    <div v-if="!items.length" class="mat-empty">{{ emptyText }}</div>
    <div v-else class="mat-list">
      <div
        v-for="it in items"
        :key="it.path"
        class="mat-row"
        :class="{ on: it.active, dead: !it.exists }"
      >
        <span
          v-if="it.is_asr"
          class="chip a"
          :title="`${it.mode} 模式的 ASR 产物 —— 它就是那个模式用掉的转写名额，不能删除`"
        >ASR·{{ it.mode }}</span>
        <span class="mono flex-1 mat-name" :title="itemTitle(it)">{{ it.name }}</span>
        <span v-if="!it.exists" class="chip r" title="配置指的是它，但文件已经不在了">文件不存在</span>
        <span
          v-else-if="it.external && !it.is_asr && kind === 'transcript'"
          class="chip a"
          title="外部字幕：不是这个工程的 ASR 产物"
        >外部</span>
        <span class="muted small mat-size">{{ humanBytes(it.size) }}</span>
        <span v-if="it.active" class="chip g" title="当前生效的就是这一份">使用中</span>
        <button
          v-else
          class="mini-btn"
          :disabled="!it.exists || busy"
          :title="it.exists ? '切换成这一份' : '文件不存在'"
          @click="$emit('activate', it)"
        >切换</button>
        <button
          v-if="it.removable"
          class="mini-btn danger"
          title="从列表移除（只改选择，不删磁盘上的文件）"
          @click="$emit('remove', it)"
        >删除</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { humanBytes } from '../../stores/project'

const props = defineProps({
  /** `[project.materials]` 的一类清单（服务端 `AssetStatus.items`，ASR 产物在前） */
  items: { type: Array, default: () => [] },
  /** `'danmaku'` | `'transcript'`（只影响"外部字幕"那个徽标与文案） */
  kind: { type: String, default: 'transcript' },
  /** 项目作用域有任务在跑：切换/删除都别动（会和管线抢同一份 vtrim.toml） */
  busy: { type: Boolean, default: false },
  emptyText: { type: String, default: '' }
})

defineEmits(['pick', 'activate', 'remove'])

const label = props.kind === 'danmaku' ? '弹幕' : '字幕'
const pickTitle = props.kind === 'danmaku'
  ? '从项目目录里选一个弹幕文件（选中后加进列表并立即生效）'
  : '从项目目录里选一份字幕文件（选中后加进列表并立即生效）'

/** 列表行只显示文件名（长路径会把行撑爆），完整绝对路径放 title。 */
function itemTitle(it) {
  const abs = String(it?.abs || '').trim()
  if (!abs) return it?.path || ''
  return abs + (it.exists ? '' : '（文件不存在）')
}
</script>

<style scoped>
/* 清单：最多 4 行可见（超出滚动），否则两块素材能把右列撑得比视频还高 */
.mat-blk { border-top:1px dashed var(--line); padding-top:9px; }
.mat-head { display:flex; align-items:center; gap:6px; margin-bottom:6px; }
.mat-label { flex:none; font-size:12px; font-weight:600; color:var(--ink-dim); }
.mat-head .mini-btn { flex:none; }
.mat-list { max-height:118px; overflow:auto; }
.mat-empty { font-size:12px; color:var(--ink-dim); padding:6px 8px; border:1px dashed var(--line);
  border-radius:var(--r-sm); }
.mat-row { display:flex; align-items:center; gap:6px; padding:4px 6px; border-radius:var(--r-sm);
  border:1px solid transparent; }
.mat-row + .mat-row { border-top:1px dashed var(--line); border-radius:0; }
.mat-row:hover { background:var(--bg-soft); }
/* 生效值：左侧一条竖线 + 淡底，扫一眼就知道现在用的是哪条 */
.mat-row.on { background:var(--green-soft); border-color:var(--green); border-radius:var(--r-sm); }
.mat-row.dead .mat-name { opacity:.6; text-decoration:line-through; }
.mat-name { font-size:11px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.mat-size { flex:none; }
/* chip 与按钮都钉 `flex:none`（`.mini-btn` 默认可收缩，窄栏里会被压成一条） */
.mat-row .chip, .mat-row .mini-btn { flex:none; }
</style>
