<template>
  <Teleport to="body">
    <div v-if="s.show" class="modal-backdrop" @click.self="settleDialog(false)">
      <div class="modal dialog-modal" @keydown.enter.prevent="settleDialog(true)">
        <h2 class="dialog-title">{{ s.title }}</h2>
        <div class="dialog-msg">{{ s.message }}</div>
        <input v-if="s.mode === 'prompt'" ref="inputRef" v-model="s.value" :placeholder="s.placeholder" class="mt-8">
        <div class="row mt-12">
          <button class="btn" @click="settleDialog(false)">取消</button>
          <button class="btn" :class="s.danger ? 'btn-danger' : 'btn-primary'" @click="settleDialog(true)">
            {{ s.mode === 'prompt' ? '确定' : '确认' }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<script setup>
import { ref, watch, nextTick } from 'vue'
import { dialogState as s, settleDialog } from '../stores/dialog'

const inputRef = ref(null)

watch(() => s.show, async (v) => {
  if (v && s.mode === 'prompt') {
    await nextTick()
    inputRef.value && inputRef.value.focus()
  }
})
</script>

<style scoped>
.dialog-modal { width:min(94vw,420px); }
.dialog-title { font-size:16px; margin-bottom:6px; }
.dialog-msg { font-size:13px; color:var(--ink-dim); white-space:pre-wrap; word-break:break-word; }
</style>
