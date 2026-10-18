// headless 夹具页：挂**真的** StickerPicker（不吃 mock），父组件写回语义与 TimelineEditor::onStickerPick 一致。
import { createApp, h, ref } from 'vue'
import StickerPicker from '../../../webui/src/components/polish/StickerPicker.vue'
import '../../../webui/src/styles/tokens.css'
import '../../../webui/src/styles/app.css'
import { LIB, EMOJI } from './fixture.js'

// 事件状态（= TimelineEditor 传给选择器的四个 prop）
const pack = ref('')
const tag = ref('')
const name = ref('')
const source = ref('')
const lib = ref(LIB)
const libError = ref('')

// 与父组件 onStickerPick 逐条同义（keyword 对贴纸态元素落 text；这里只看 props 四元组）
function onPick(patch) {
  if (patch.keyword !== undefined) pack.value = patch.keyword || ''
  if (patch.sticker_tag !== undefined) tag.value = patch.sticker_tag || ''
  if (patch.name !== undefined) name.value = patch.name || ''
  if (patch.source !== undefined) source.value = patch.source || ''
}

const app = createApp({
  render() {
    // 宿主 = **真**的 .props-panel 容器（TimelineEditor scoped：width:232px + padding:10px + border:1px，
    // 全局 border-box ⇒ 行内容宽 **210px**；266 那个数是右栏别的卡片、不是这里）
    return h('div', { class: 'panel', style: 'width:232px;padding:10px;border:1px solid var(--line);box-sizing:border-box' }, [
      h(StickerPicker, {
        lib: lib.value, libError: libError.value, pid: 1, dir: 'fixture/clip-01',
        pack: pack.value, tag: tag.value, name: name.value, source: source.value,
        onPick, onRetryLib: () => {},
      }),
    ])
  },
})
app.mount('#app')

const rect = (el) => { const b = el.getBoundingClientRect(); return { w: +b.width.toFixed(2), h: +b.height.toFixed(2) } }
/** 量几何 + 顺手把可见文案带回去（证明这一次点击真的把界面推进到了那一页）。 */
window.__H = {
  pack, tag, name, source, lib, libError,
  setLib(v) { lib.value = v },
  setLibError(v) { libError.value = v },
  /** 清成"打开一个新事件"（四个 prop 全空 ⇒ 组件自己回到第 1 页） */
  reset() { pack.value = ''; tag.value = ''; name.value = ''; source.value = '' },
  click(sel, i) {
    const els = document.querySelectorAll(sel)
    const el = els[i || 0]
    if (!el) return false
    el.click()
    return true
  },
  count(sel) { return document.querySelectorAll(sel).length },
  measure() {
    const q = (s) => document.querySelector(s)
    const sp = q('.sp'), stack = q('.stack'), body = q('.body')
    const cells = [...document.querySelectorAll('.grid3 .cell')]
    const rows = [...document.querySelectorAll('.srow')]
    return {
      sp: sp ? rect(sp) : null,
      stack: stack ? rect(stack) : null,
      body: body ? rect(body) : null,
      panel: q('.panel') ? rect(q('.panel')) : null,
      rows: rows.map((r) => ({ h: +r.getBoundingClientRect().height.toFixed(2), text: r.innerText.replace(/\s+/g, ' ').trim() })),
      cells: cells.length,
      cellW: cells.length ? +cells[0].getBoundingClientRect().width.toFixed(2) : 0,
      emCells: document.querySelectorAll('.em-cell').length,
      emW: document.querySelector('.em-cell') ? +document.querySelector('.em-cell').getBoundingClientRect().width.toFixed(2) : 0,
      bodyScroll: body ? { sh: body.scrollHeight, ch: body.clientHeight } : null,
      // 标题行：18px 写死 ⇒ 内容换行就会被挤爆（压到正文上）—— 断言 sh <= h
      secH: (() => {
        const s = document.querySelector('.sec-h')
        return s ? { h: +s.getBoundingClientRect().height.toFixed(2), sh: s.scrollHeight, sw: s.scrollWidth, cw: s.clientWidth } : null
      })(),
      overflowX: sp ? sp.scrollWidth - sp.clientWidth : 0,
      page: (sp ? sp.innerText.replace(/\s+/g, ' ').trim().slice(0, 90) : ''),
    }
  },
}
window.__FIX = { LIB, EMOJI }
