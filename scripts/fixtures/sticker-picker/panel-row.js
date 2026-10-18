// 属性面板「标配行」的几何夹具（与贴纸选择器共用这个夹具目录 / 同一套 headless 管道）。
//
// ⚠️ 标签与行的 class **都不从这里抄**：由 scripts/check-sticker-picker-layout.mjs 从
//    TimelineEditor.vue 那条 row 里**抽出来**传进来（改标签/改间距都不会让这条判据失真）。
import '../../../webui/src/styles/tokens.css'
import '../../../webui/src/styles/app.css'

// 宿主 = **真** .props-panel（232px 外框 + 10px 内边距 + 1px 边框，全局 border-box ⇒ 内容 210px）
const host = document.getElementById('app')
host.style.cssText = 'width:232px;padding:10px;border:1px solid var(--line);box-sizing:border-box;background:var(--card)'
const row = document.createElement('div')
host.appendChild(row)

window.__ROW = {
  set(spec) {
    row.className = spec.rowClass
    row.innerHTML = ''
    for (const t of spec.labels) {
      const b = document.createElement('button')
      b.className = 'mini-btn'
      b.textContent = t
      row.appendChild(b)
    }
  },
  measure() {
    const r = row.getBoundingClientRect()
    const bs = [...row.querySelectorAll('button')].map((b) => ({
      t: b.textContent, w: +b.getBoundingClientRect().width.toFixed(2), h: +b.getBoundingClientRect().height.toFixed(2),
    }))
    return {
      rowW: +r.width.toFixed(2), rowH: +r.height.toFixed(2),
      btnH: bs[0] ? bs[0].h : 0, sum: +bs.reduce((a, x) => a + x.w, 0).toFixed(2),
      n: bs.length, btns: bs, scrollW: row.scrollWidth, clientW: row.clientWidth,
    }
  },
}
