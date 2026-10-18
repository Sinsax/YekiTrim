// polish/diag.js — A3 预览诊断埋点：`window.__dhampirDiag`（规格 docs/plan/polish-refactor-spec-2026-10-19.md §3 A3）。
//
// 为什么要有它：黑屏类问题**没有堆栈**（画布是 WebGL，读不回来），只能靠"当时那几个数"归因。
//   · `canvasGen`：**canvas 元素身份计数（重挂即 +1）** —— A4 断言（与 B1）要抓的就是它。
//   · `lastPresentAt`：最近一次"画面上屏"的时刻（Dhampir：`engine.seek` 画完；DOM：源在走且
//     readyState≥2 的那一帧）。HUD 上显示成 **present 延迟 = now - lastPresentAt**。
//   · `frameNull`：引擎这一帧**没有对应帧**（当前秒落在被掐掉的源区间里）。
//   · `contextLost`：WebGL 上下文丢了（`webglcontextlost`）。
//   · `busy` / `maskOn` / `activeSourceReadyState` / `t`：一帧在飞 / 定格遮罩 / 活跃源就绪度 / 当前秒。
//
// ⚠️ **必须是模块级单例**（不能写在组件的 setup 里）：`canvasGen` 要抓的正是"组件被重挂" ——
//    计数器写在实例里会跟着一起归零，**永远读到 1**，那这个埋点就白加了。
// ⚠️ **只记录，不改时序**：没有任何一处读这些字段去做判断（HUD 也只是显示）。`lastPresentAt`
//    写在"这一帧已经画完"之后 —— 它不是新的同步点。
const diag = {
  t: 0,
  lastPresentAt: 0,
  activeSourceReadyState: 0,
  busy: false,
  maskOn: false,
  frameNull: false,
  contextLost: false,
  canvasGen: 1
}

/** 已经见过的那块 canvas。**不进 diag 对象**：埋点的形状按规格钉死为上面 8 个键。 */
let lastCanvasEl = null
const wiredCanvases = new WeakSet()

if (typeof window !== 'undefined') window.__dhampirDiag = diag

/** 每帧更新**易变的那几个**（零分配 —— 这条循环是 60Hz 的）。 */
export function diagFrame(t, readyState, busy, maskOn, frameNull) {
  diag.t = t
  diag.activeSourceReadyState = readyState
  diag.busy = busy
  diag.maskOn = maskOn
  diag.frameNull = frameNull
}

/** 一帧真的上屏了：记时刻（Dhampir = `engine.seek` 画完；DOM = 源在出帧）。 */
export function diagPresent() {
  diag.lastPresentAt = performance.now()
}

/**
 * 记下"现在这块 canvas 是哪一块"。**换过元素就 +1**（第一次见到不算重挂：挂载那一刻本来就从
 * 没有到有一块）。重挂要留一条 warn —— 底座一页只 attach 一次、没有 detach，新块永远画不出东西。
 */
export function diagTouchCanvas(el) {
  if (!el || el === lastCanvasEl) return diag.canvasGen
  if (lastCanvasEl !== null) {
    diag.canvasGen += 1
    console.warn('[dhampir] #dhampir-canvas 被重挂了：canvasGen=' + diag.canvasGen
      + '（底座一页只 attach 一次、没有 detach ⇒ 新块画不出东西；见 webui/src/components/AGENTS.md）')
  }
  lastCanvasEl = el
  wireContextLost(el)
  return diag.canvasGen
}

/** 上下文丢失只记一次（同一个元素只挂一个监听）。 */
function wireContextLost(el) {
  if (wiredCanvases.has(el)) return
  wiredCanvases.add(el)
  el.addEventListener('webglcontextlost', () => { diag.contextLost = true }, false)
  el.addEventListener('webglcontextrestored', () => { diag.contextLost = false }, false)
}

/** `?diag=1`（也认 hash 里带的，如 `#/p/1/polish/clip-01?diag=1`）⇒ 渲染右下角 HUD。 */
export function diagHudEnabled() {
  if (typeof location === 'undefined') return false
  return /[?&]diag=1(?:&|$)/.test(location.search) || /[?&]diag=1(?:&|$)/.test(location.hash)
}

/** 给 HUD / 探针读的那一份（与 `window.__dhampirDiag` 是同一个对象）。 */
export const dhampirDiag = diag
