// 属性面板「字段密度」的几何夹具（与选择器 / 标配行共用这个夹具目录与同一条 headless 管道）。
//
// ⚠️ 标签**不从这里抄**：由 scripts/check-sticker-picker-layout.mjs 从 TimelineEditor.vue
//    抽出来传进来（改标签不会让这条判据失真）。
// ⚠️ 这里量的是**排版**（高度/宽度/溢出），不量颜色 —— 两套面板的内容逐项相同，
//    只有 CSS 不同：`is-dense` 走 app.css 的两列网格，`is-legacy` 用本文件复刻的旧规则。
import '../../../webui/src/styles/tokens.css'
import '../../../webui/src/styles/app.css'

function makePanel(cls) {
  const host = document.createElement('div')
  host.className = 'props-panel ' + cls
  // ⚠️ 232px 是**组件 scoped 样式**里的宽度（app.css 里没有）—— 夹具必须自己给，
  //    否则面板铺满整页（量出来的宽度/比例全错，第一版就是这么错的）。
  host.style.width = '232px'
  document.getElementById('app').appendChild(host)
  return host
}
const dense = makePanel('is-dense')
const legacy = makePanel('is-legacy')

// 改造前的规则（label 换行 + 控件独占一行）—— 只在这一页里生效。
const legacyCss = document.createElement('style')
legacyCss.textContent = '.is-legacy{display:block} .is-legacy>label{display:block;margin:6px 0 4px} .is-legacy>select{width:100%} .is-legacy>.hint{display:block}'
document.head.appendChild(legacyCss)

function fill(host, labels) {
  host.innerHTML = ''
  for (const t of labels) {
    const l = document.createElement('label')
    l.textContent = t
    host.appendChild(l)
    if (t === '角度') {
      const row = document.createElement('div')
      row.className = 'angle-row'
      const r = document.createElement('input')
      r.type = 'range'; r.min = '-7'; r.max = '7'; r.step = '0.5'; r.value = '-6'
      const v = document.createElement('span')
      v.className = 'angle-val'; v.textContent = '-6°'
      row.appendChild(r); row.appendChild(v)
      host.appendChild(row)
    } else {
      const s = document.createElement('select')
      for (const o of ['不设', '弹出', '淡入淡出', '倾斜 · 标准', '竖排', '抖动', '逐字变大']) {
        const op = document.createElement('option'); op.textContent = o; s.appendChild(op)
      }
      host.appendChild(s)
    }
  }
  const h = document.createElement('div')
  h.className = 'hint'
  h.textContent = '这个档名不认识，成片里不会动。'
  host.appendChild(h)
}


// ⚠️ **面板里出现过的每种"行形状"**（2026-10-16）：两列网格是**全局**作用在 `.props-panel` 的
//    直接子元素上的 ⇒ 只量三轴块不够，得把每种形状都摆一遍、把通用不变量一起钉住：
//    ① 标签 + 下拉 ② 标签 + 文本输入 ③ 标签 + 数字输入 ④ 标签 + 颜色 ⑤ 标签 + textarea
//    （textarea 故意要**整行**：它在第 2 列的 160px 里没法用）⑥ 自己排好布局的 `.row`（按钮行）
//    ⑦ `.pos-row`（标签 + 滑杆 + 数字）。
function fillShapes(host) {
  host.innerHTML = ''
  const pair = (label, el) => {
    const l = document.createElement('label')
    l.textContent = label
    host.appendChild(l)
    host.appendChild(el)
  }
  const sel = document.createElement('select')
  ;['不设', '弹出'].forEach((o) => { const op = document.createElement('option'); op.textContent = o; sel.appendChild(op) })
  pair('排版', sel)
  const txt = document.createElement('input'); txt.type = 'text'; txt.value = '关键词'
  pair('关键词', txt)
  const num = document.createElement('input'); num.type = 'number'; num.value = '12'
  pair('时长', num)
  const col = document.createElement('input'); col.type = 'color'; col.value = '#dcbda0'
  pair('颜色', col)
  const ta = document.createElement('textarea'); ta.value = '一行文字'
  pair('文字内容', ta)
  const row = document.createElement('div')
  row.className = 'row spread'
  ;['入点', '出点', '定位'].forEach((t) => {
    const b = document.createElement('button'); b.className = 'mini-btn'; b.textContent = t; row.appendChild(b)
  })
  host.appendChild(row)
  const pos = document.createElement('div')
  pos.className = 'pos-row'
  const tag = document.createElement('span'); tag.className = 'pos-tag'; tag.textContent = 'X'
  const rng = document.createElement('input'); rng.className = 'pos-range'; rng.type = 'range'; rng.min = '0'; rng.max = '100'
  const pnum = document.createElement('input'); pnum.className = 'pos-num'; pnum.type = 'number'
  pos.appendChild(tag); pos.appendChild(rng); pos.appendChild(pnum)
  host.appendChild(pos)
}
function stat(host) {
  const r = host.getBoundingClientRect()
  const sels = [...host.querySelectorAll('select')].map((s) => +s.getBoundingClientRect().width.toFixed(2))
  const labels = [...host.querySelectorAll(':scope > label')].map((l) => ({
    t: l.textContent, w: +l.getBoundingClientRect().width.toFixed(2), need: l.scrollWidth,
  }))
  // 角度那一行是**控件列里唯一的 flex 行**（滑杆 + 度数 + 复位）：单独量它的右边界，
  // 以及"它是不是与自己的标签同一行"（两列网格的意义所在）。
  const ang = host.querySelector('.angle-row')
  const rng = host.querySelector('input[type=range]')
  const angleRight = ang ? +ang.getBoundingClientRect().right.toFixed(2) : null
  const rangeRight = rng ? +rng.getBoundingClientRect().right.toFixed(2) : null
  const angleLabel = [...host.querySelectorAll(':scope > label')]
    .find((l) => l.textContent.trim() === '角度')
  const box = (el) => (el ? {
    top: +el.getBoundingClientRect().top.toFixed(2),
    bottom: +el.getBoundingClientRect().bottom.toFixed(2),
  } : null)
  const angleLabelBox = box(angleLabel)
  const angleRowBox = box(ang)
  // ⚠️ **同一行**比的是**垂直中心**，不是顶边：标签是 `align-self:center`，
  //    它的顶边天生比行顶低（第一版比顶边，判据自摆乌龙）。
  const angleInline = !!(angleLabelBox && angleRowBox)
    && Math.abs((angleRowBox.top + angleRowBox.bottom) / 2
      - (angleLabelBox.top + angleLabelBox.bottom) / 2) <= 2
  return {
    panelRight: +r.right.toFixed(2), angleRight, rangeRight, angleInline, angleLabelBox, angleRowBox,
    h: +r.height.toFixed(2), w: +r.width.toFixed(2),
    sels, minSel: sels.length ? Math.min.apply(null, sels) : 0, labels,
    scrollW: host.scrollWidth, clientW: host.clientWidth,
  }
}
window.__FORM = {
  shapes() { fillShapes(dense); fillShapes(legacy) },
  set(labels) { fill(dense, labels); fill(legacy, labels) },
  measure() { return { dense: stat(dense), legacy: stat(legacy) } },
}
