#!/usr/bin/env node
// P1 门禁：**DOM 腿的运镜缩放 = 底座求值器的运镜缩放**（逐值）。
//
// # 为什么需要它
//
// 精修预览有两条腿：Dhampir 底座（wasm，**出片同源**）与 DOM 自绘层（`PolishPreview.vue`）。
// 底座的运镜**已经在 `dhampir_project_frame` 里算好了**（`layers[].transform.scale` ——
// 实测它逐帧跟随 keyframe 插值），而 DOM 腿**不读它**：它自己在
// `engine.js::cameraAt`（倍率）+ `handheldSway`（摇摆）里再算一遍，见
// `PolishPreview.vue::videoStyle` 的 `camScale = f.zoom * sway.scale`。
//
// 同一件事两份实现 ⇒ 漂了就"预览里镜头是 1.6、成片里是 1.5"，而两边各自看都正常。
// 这条门禁把**两边对同一个受控工程的值**钉在一起。
//
// # 怎么对（受控工程，不依赖真片段）
//
// 拿真工程的外壳（assets/view/render_hints），把视频层换成**一个受控 keyframe**：
// `scale = 档位默认倍率`、全程恒定。于是：
//   base 给 `transform.scale` = 那个档位的倍率
//   DOM 给 `camFactor(档位)`（`engine.js`，与 Rust `CameraStyle::factor` 同表）
// 两者必须**逐值相等**。四个档位都试 —— 这正好也覆盖了"预览与成片用不同倍率"这个老伤。
//
// ⚠️ 这条判据**不断言行为等价**（那要跑 Vue）；它钉的是"两条腿读的是同一张倍率表"。
//
// 运行：node scripts/check-dom-camera-parity.mjs
//      node scripts/check-dom-camera-parity.mjs --mutate   # 变异自证

import { readFileSync, existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const MUTATE = process.argv.includes('--mutate')

const PKG = process.env.DHAMPIR_PKG
  || 'F:/para/Code/Dhampir/dist/dhampir-0.4.0+901bb6a/preview/pkg'
const DOC = join(root, '.workbuddy', 'yekitrim-handover', 'final.doc.json')

let failures = 0
const check = (name, ok, detail) => {
  if (ok) console.log(`  ok   ${name}`)
  else { console.log(`  FAIL ${name}${detail ? '\n         ' + detail : ''}`); failures += 1 }
}

if (!existsSync(DOC) || !existsSync(PKG + '/dhampir_wasm.js')) {
  console.log('  SKIP 素材/底座包缺失 —— 无法对拍（不是通过）')
  process.exit(0)
}

const mod = await import('file:///' + PKG + '/dhampir_wasm.js')
await mod.default({ module_or_path: readFileSync(PKG + '/dhampir_wasm_bg.wasm') })
const eng = await import('file:///' + join(root, 'webui', 'src', 'polish', 'engine.js').replace(/\\/g, '/'))
const shell = JSON.parse(readFileSync(DOC, 'utf8'))

/** 受控工程：一个视频层，scale 恒定 = 给定值（模拟"这一档的默认倍率"）。 */
function makeProject(scale) {
  return {
    project_schema: 1,
    meta: shell.meta,
    assets: shell.assets,
    timeline: {
      schema: 4,
      timebase: { num: 60, den: 1 },
      markers: [],
      tracks: [{
        id: 'v1', kind: 'video',
        layers: [{
          id: 'main-0', start: 0, end: 300,
          source: { asset_id: 'clip.mp4', source_in: 0 },
          transform: { x: 0, y: 0, scale, rotation: 0 },
          opacity: 1, blend: 'normal', enabled: true, effects: [],
          keyframes: [
            { frame: 0, target: 'scale', value: scale, easing: 'linear' },
            { frame: 120, target: 'scale', value: scale, easing: 'linear' }
          ]
        }]
      }]
    },
    view: shell.view,
    render_hints: shell.render_hints
  }
}

/** 档位 → 期望倍率（`engine.js` 的 `camFactor`，与 Rust `CameraStyle::factor` 同表）。 */
const ZOOMS = ['特写', '近', '中', '远']

console.log('\n① 底座求值器 vs DOM 腿倍率表（逐档位）')
for (const z of ZOOMS) {
  const domFactor = eng.camFactor(z)
  if (typeof domFactor !== 'number' || !isFinite(domFactor)) {
    check(`DOM 腿 camFactor('${z}') 是个有限数`, false, `实得 ${domFactor}`)
    continue
  }
  const proj = makeProject(domFactor)
  const opened = JSON.parse(mod.dhampir_project_open(JSON.stringify(proj)))
  if (opened.ok !== true) {
    check(`受控工程（${z}）载入成功`, false, JSON.stringify(opened).slice(0, 200))
    continue
  }
  const payload = JSON.parse(mod.dhampir_project_frame(60))
  const layer = (payload.layers || [])[0]
  const baseScale = layer?.transform?.scale
  check(
    `档位「${z}」：base transform.scale = DOM camFactor（${domFactor}）`,
    typeof baseScale === 'number' && Math.abs(baseScale - domFactor) < 1e-6,
    `base=${baseScale} dom=${domFactor}`
  )
}

console.log('\n② 底座确实在**按 keyframe 求值**（不然上面那条是恒等式的巧合）')
{
  const proj = makeProject(1.5)
  proj.timeline.tracks[0].layers[0].keyframes = [
    { frame: 0, target: 'scale', value: 1.5, easing: 'linear' },
    { frame: 120, target: 'scale', value: 1.0, easing: 'linear' }
  ]
  mod.dhampir_project_open(JSON.stringify(proj))
  const s0 = JSON.parse(mod.dhampir_project_frame(0)).layers[0].transform.scale
  const s60 = JSON.parse(mod.dhampir_project_frame(60)).layers[0].transform.scale
  const s119 = JSON.parse(mod.dhampir_project_frame(119)).layers[0].transform.scale
  check('起始帧 = 起值 1.5', Math.abs(s0 - 1.5) < 1e-6, `实得 ${s0}`)
  check('中段**真的插值**了（1.5 → 1.0 之间，不是任何一个端点）', s60 > 1.0 && s60 < 1.5, `实得 ${s60}`)
  check('末段趋近终值 1.0', s119 > 1.0 && s119 < s60, `实得 ${s119}（前一句是 ${s60}）`)
}

if (MUTATE) {
  console.log('\n── 变异自证 ──')
  // 变异：把 DOM 腿的倍率表改一个值 → 上面①必须翻面
  const orig = eng.camFactor
  const badFactor = (z) => (z === '近' ? 1.8 : orig(z))
  let drifted = 0
  for (const z of ZOOMS) {
    const domFactor = badFactor(z)
    mod.dhampir_project_open(JSON.stringify(makeProject(orig(z))))
    const baseScale = JSON.parse(mod.dhampir_project_frame(60)).layers[0].transform.scale
    if (Math.abs(baseScale - domFactor) >= 1e-6) drifted += 1
  }
  check('[变异] DOM 倍率表把「近」改成 1.8 → 判据应当翻面', drifted === 1, `drifted=${drifted}（期望正好 1 档）`)
  // 变异：keyframe 不再插值 → ②必须翻面
  {
    const proj = makeProject(1.5)
    proj.timeline.tracks[0].layers[0].keyframes = [
      { frame: 0, target: 'scale', value: 1.5, easing: 'linear' },
      { frame: 120, target: 'scale', value: 1.5, easing: 'linear' }
    ]
    mod.dhampir_project_open(JSON.stringify(proj))
    const flat = JSON.parse(mod.dhampir_project_frame(60)).layers[0].transform.scale
    check('[变异] keyframe 压平后「真的插值了」应当翻面', !(flat > 1.0 && flat < 1.5), `实得 ${flat}`)
  }
}

console.log(failures ? `\n✗ ${failures} 项` : '\n全部通过')
process.exit(failures ? 1 : 0)
