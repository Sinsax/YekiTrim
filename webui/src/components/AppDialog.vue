<template>
  <Teleport to="body">
    <div v-if="s.show" class="modal-backdrop" @click.self="cancel">
      <div
        ref="rootRef"
        class="modal dialog-modal"
        :role="s.danger ? 'alertdialog' : 'dialog'"
        aria-modal="true"
        :aria-labelledby="titleId"
        :aria-describedby="msgId"
        tabindex="-1"
      >
        <h2 :id="titleId" class="dialog-title">{{ s.title }}</h2>
        <div :id="msgId" class="dialog-msg">{{ s.message }}</div>
        <input
          v-if="s.mode === 'prompt'"
          ref="inputRef"
          v-model="s.value"
          :placeholder="s.placeholder"
          class="mt-8 dialog-input"
          @keydown.enter.prevent="confirm"
        >
        <div class="row mt-12 dialog-actions">
          <button ref="cancelRef" class="btn" type="button" @click="cancel">取消</button>
          <button class="btn" type="button" :class="s.danger ? 'btn-danger' : 'btn-primary'" @click="confirm">
            {{ s.mode === 'prompt' ? '确定' : '确认' }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<script setup>
import { computed, ref } from 'vue'
import { dialogState as s, settleDialog } from '../stores/dialog'
import { useOverlay } from '../composables/useOverlay'

// 每个实例一个稳定 id（`aria-labelledby` / `aria-describedby` 指向真实节点）
const titleId = `yt-dialog-title-${Math.random().toString(36).slice(2, 9)}`
const msgId = `yt-dialog-msg-${Math.random().toString(36).slice(2, 9)}`

const rootRef = ref(null)
const inputRef = ref(null)
const cancelRef = ref(null)
const isOpen = computed(() => s.show)

function cancel() {
  settleDialog(false)
}
function confirm() {
  settleDialog(true)
}

/**
 * 初始焦点：
 *  - prompt ⇒ 输入框（进去就能打字）
 *  - 其余（含危险确认）⇒ **取消**按钮。
 *
 * ⚠️ 原来整个弹层挂着 `@keydown.enter.prevent="settleDialog(true)"`：焦点落在任何地方按回车
 * 都会确认，危险操作（删源录播 / 移除项目）也能被一次误触回车点掉。现在只有 prompt 的输入框
 * 允许 Enter 提交，其余必须真的 Tab 到「确认」再按。
 */
useOverlay({
  isOpen,
  onClose: cancel,
  rootRef,
  initialFocus: () => (s.mode === 'prompt' ? inputRef.value : cancelRef.value)
})
</script>

<style scoped>
.dialog-modal { width:min(94vw,420px); }
.dialog-title { font-size:16px; margin-bottom:6px; }
.dialog-msg { font-size:13px; color:var(--ink-dim); white-space:pre-wrap; word-break:break-word; }
/* 危险确认的主按钮与普通确认拉开距离：危险在右、普通在左，视觉重量交给「取消」 */
.dialog-actions { display:flex; gap:8px; justify-content:flex-end; }
</style>
