#!/usr/bin/env node
// scripts/gate.mjs —— 统一门禁执行入口（整改计划 P1 · T5 / T8）
//
// 设计与**不可妥协**的点（来源：docs/plan/remediation-plan-2026-11-24.md §T5 / §T8）：
//
//  1. 脚本**自动发现**，不写死清单：
//       scripts/check-*.mjs        （node 跑）
//       webui/scripts/check-*.mjs  （node 跑）
//       scripts/check_*.py         （python3 跑，注意是**下划线**）
//     新增一个 check-*.mjs 就自动进门禁 —— 手写清单一定会漂。
//  2. 守卫自检**也自动发现**：读脚本文本，出现 --self-test 就多跑一次 --self-test，
//     出现 --mutate 就多跑一次 --mutate。
//     为什么：check-effect-registry 的自检曾经烂了很久（断言写死"15 个类型"而词汇表早已 11 个），
//     原因只有一句 —— "主检查进了 gates、自检没进"。
//  3. **任何"没跑"都必须显式打印成 SKIP**（本任务唯一不可妥协的判据）。SKIP 只来自三处：
//       ① 脚本自己打印的 SKIP 行（**原样带走脚本自己说的"要准备什么"**）
//       ② 退出码 3（本仓约定：缺样本 = exit 3）
//       ③ 超时（--timeout，默认 300s）
//     SKIP 永不算通过，也不受 baseline 影响。
//     教训：yekitrim-compare.mjs 曾因参数白名单漏了 --golden 而**静默换参照**，"数字还都在阈值内"。
//     静默跳过比不跑更糟。
//  4. --changed [base]（默认 base=HEAD）：git diff --name-only <base> → 按下面 MAPPING 选脚本。
//     映射表**就写在本文件里**（不另开配置文件），每条都写了"为什么"。只改前端时**不跑 Rust 测试**，
//     并且会显式打印 SKIP cargo test -p yekitrim-server（未触及 crates/）。
//  5. --all：全量（脚本 + 自检/mutate + 结构门禁 + cargo；--no-cargo 只跑脚本）。
//     每项独立超时，超时按 SKIP 显式打印（不是"通过"）。
//  6. --check-structure：把 python3 scripts/check_module_map.py 与
//     python3 scripts/check_module_boundaries.py 包成同一个子动作（T8）。
//  7. 汇总表：通过 N / 跳过 M / 失败 K（基线 X），每行带脚本名 + 耗时。
//     退出码 = 是否存在**非基线**失败（0 = 干净，1 = 有红，2 = 用法/配置错）。
//  8. baseline：scripts/gate-baseline.json，形如
//       { "scripts/check-xxx.mjs": "本机底座 host_api 低一版（已知红）" }
//     **每条都必须带非空理由字符串**（空理由的 baseline 就是第二个"静默跳过"）。
//     只准登记**环境性 / 本机不具备**的失败；**判据真红不许登记** —— 真红就改代码。
//
// 用法：见 printUsage()（node scripts/gate.mjs --help）
//
// ⚠️ 本脚本**不往仓库里写任何东西**（失败日志只写 /tmp）。跑完 git status 必须和跑之前一样。
// ⚠️ 零依赖：只用 node 内置模块（node:fs / node:path / node:child_process / node:url）。

import { spawn, spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(HERE, '..')
const BASELINE_PATH = join(HERE, 'gate-baseline.json')
const DEFAULT_TIMEOUT_SEC = 300
const MAX_CAPTURE = 400_000 // 单次输出最多留这么多字符（超了丢最早的）
const KEEP_TAIL = 200_000
const DEFAULT_LOG_DIR = '/tmp/yekitrim-gate-logs'

// ════════════════════════════════════════════════════════════════════════════
// CLI
// ════════════════════════════════════════════════════════════════════════════

class UsageError extends Error {}

function printUsage() {
  console.log('scripts/gate.mjs —— 统一门禁执行入口（零依赖，只用 node 内置模块）')
  console.log('')
  console.log('用法：')
  console.log('  node scripts/gate.mjs --all                     全量：所有 check-*.mjs / check_*.py + 自检/mutate')
  console.log('                                                  + 结构门禁 + cargo（--no-cargo 只跑脚本）')
  console.log('  node scripts/gate.mjs --changed [base]          按 git diff --name-only <base> 选（默认 base=HEAD）')
  console.log('  node scripts/gate.mjs --check-structure         只跑结构门禁（check_module_map.py + check_module_boundaries.py）')
  console.log('  node scripts/gate.mjs --changed --check-structure   两个子动作叠加（T8 的验收命令）')
  console.log('  node scripts/gate.mjs --list ...                只打印"会跑什么 / 为什么"，不执行')
  console.log('  node scripts/gate.mjs --files <p1,p2,...>       手工指定改动面（等价 --changed，但不用 git；测映射用）')
  console.log('')
  console.log('可选：')
  console.log('  --timeout <秒>     每项独立超时（默认 ' + DEFAULT_TIMEOUT_SEC + '；超时记 SKIP，不是通过）')
  console.log('  --jobs <n>         并发度（默认 1 = 串行；脚本之间可能抢端口/临时目录，非必要别开）')
  console.log('  --with-cargo       强制带上 cargo 项   --no-cargo  强制不带（两种都会打印 SKIP 行）')
  console.log('  --only <子串>      只跑 key 含该子串的项（可重复；被过滤掉的会打印计数，不计入结论）')
  console.log('  --logs <目录>      失败/跳过项的输出落盘目录（默认 ' + DEFAULT_LOG_DIR + '，**不写仓库**）')
  console.log('')
  console.log('退出码：0 = 没有非基线失败；1 = 有非基线失败；2 = 用法/配置错（含 baseline 文件格式错）')
  console.log('无参数 = --changed（默认 base=HEAD）')
}

function parseArgs(argv) {
  const opt = {
    all: false,
    changed: null,
    structure: false,
    files: null,
    timeoutSec: DEFAULT_TIMEOUT_SEC,
    jobs: 1,
    list: false,
    only: [],
    logs: DEFAULT_LOG_DIR,
    cargoMode: 'auto', // auto | on | off
    help: false,
  }
  const need = (i, name) => {
    if (i + 1 >= argv.length) throw new UsageError(name + ' 需要一个参数')
    return argv[i + 1]
  }
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    const eq = a.indexOf('=')
    const flag = eq > 0 ? a.slice(0, eq) : a
    const inline = eq > 0 ? a.slice(eq + 1) : null
    switch (flag) {
      case '-h':
      case '--help':
        opt.help = true
        break
      case '--all':
        opt.all = true
        break
      case '--changed': {
        if (inline !== null) opt.changed = inline || 'HEAD'
        else if (argv[i + 1] && !argv[i + 1].startsWith('-')) {
          opt.changed = argv[i + 1]
          i++
        } else opt.changed = 'HEAD'
        break
      }
      case '--check-structure':
        opt.structure = true
        break
      case '--files': {
        const v = inline !== null ? inline : need(i, '--files')
        if (inline === null) i++
        opt.files = v.split(',').map((s) => s.trim()).filter(Boolean)
        break
      }
      case '--timeout': {
        const v = inline !== null ? inline : need(i, '--timeout')
        if (inline === null) i++
        const n = Number(v)
        if (!Number.isFinite(n) || n <= 0) throw new UsageError('--timeout 需要正数秒：' + v)
        opt.timeoutSec = n
        break
      }
      case '--jobs': {
        const v = inline !== null ? inline : need(i, '--jobs')
        if (inline === null) i++
        const n = Number(v)
        if (!Number.isInteger(n) || n < 1) throw new UsageError('--jobs 需要 >=1 的整数：' + v)
        opt.jobs = n
        break
      }
      case '--list':
        opt.list = true
        break
      case '--only': {
        const v = inline !== null ? inline : need(i, '--only')
        if (inline === null) i++
        opt.only.push(v)
        break
      }
      case '--logs': {
        const v = inline !== null ? inline : need(i, '--logs')
        if (inline === null) i++
        opt.logs = v
        break
      }
      case '--with-cargo':
        opt.cargoMode = 'on'
        break
      case '--no-cargo':
        opt.cargoMode = 'off'
        break
      default:
        throw new UsageError('未知参数：' + a + '（--help 看用法）')
    }
  }
  if (!opt.all && opt.changed === null && !opt.structure && opt.files === null) {
    // 无参 = --changed（默认 base=HEAD）：一个入口就能跑"这次改动该跑什么"
    opt.changed = 'HEAD'
  }
  return opt
}

// ════════════════════════════════════════════════════════════════════════════
// 发现：脚本 + 自检能力（都从文件系统 / 脚本文本里读，不写死清单）
// ════════════════════════════════════════════════════════════════════════════

function discoverScripts() {
  const specs = []
  const scan = (dir, re, cmd) => {
    const abs = join(ROOT, dir)
    if (!existsSync(abs)) return
    for (const name of readdirSync(abs).sort()) {
      if (!re.test(name)) continue
      const rel = dir + '/' + name
      let text = ''
      try {
        text = readFileSync(join(ROOT, rel), 'utf8')
      } catch {
        text = ''
      }
      specs.push({
        id: rel,
        file: rel,
        cmd,
        // --self-test / --mutate 的支持与否**只从脚本文本里判断**（文档说了不算数）
        selfTest: /--self-test/.test(text),
        mutate: /--mutate/.test(text),
      })
    }
  }
  scan('scripts', /^check-.*\.mjs$/, 'node')
  scan('webui/scripts', /^check-.*\.mjs$/, 'node')
  scan('scripts', /^check_.*\.py$/, 'python3') // 下划线！python 侧是 check_module_*.py
  return specs
}

// ════════════════════════════════════════════════════════════════════════════
// 映射表：改动路径 → 要跑什么。**每条都要写为什么**（写在这里，不另开配置文件）
// ════════════════════════════════════════════════════════════════════════════

function buildMapping(specs) {
  const ids = new Set(specs.map((s) => s.id))
  const webuiAll = specs.filter((s) => s.id.startsWith('webui/scripts/')).map((s) => s.id)
  // "parity" 一族：与底座 / DOM 双腿逐值对拍的守卫（T5 原文点名 scripts/check-*parity*.mjs）
  const parity = specs.filter((s) => /^scripts\/check-.*parity.*\.mjs$/.test(s.id)).map((s) => s.id)
  const translator = 'scripts/check-yekitrim-translator.mjs'
  const effect = 'scripts/check-effect-registry.mjs'
  const wiring = 'scripts/check-dhampir-wiring.mjs'
  const llmModels = 'scripts/check-llm-models.mjs'
  const pick = (...lists) => {
    const out = []
    for (const id of lists.flat()) if (ids.has(id) && !out.includes(id)) out.push(id)
    return out
  }

  const CARGO_CHECK = ['check', '--workspace', '--all-targets']
  const CARGO_SERVER = ['test', '-p', 'yekitrim-server']
  const CARGO_SERVICE = ['test', '-p', 'yekitrim-service']
  const CARGO_CORE = ['test', '-p', 'yekitrim-core']
  const CARGO_CORE_DANMAKU = ['test', '-p', 'yekitrim-core', '--lib', 'danmaku']
  const CARGO_POLISH = ['test', '-p', 'yekitrim-polish']
  const CARGO_RENDER_CORE = ['test', '-p', 'yekitrim-render-core']
  const CARGO_LLM = ['test', '-p', 'yekitrim-llm']

  // ⚠️ 映射表就是本文件里这一张表：加一条 = 加一个 (match, scripts/cargo/structure/builtins, why)
  const MAPPING = [
    {
      name: 'webui 前端源码',
      match: [/^webui\/src\//],
      scripts: webuiAll,
      builtins: ['webui-dist-pairing'],
      why:
        'WebUI 的跨端契约守卫全在 webui/scripts/check-*.mjs（色档 / 折行 / 布局 / 覆盖层 / 像素 / 构建等效）；' +
        '它们守的是"预览与成片不一致"这一族事故 —— 界面上看不出来，只有机器判据能抓。改前端必须全跑。',
    },
    {
      name: 'webui 构建面（依赖 / 入口 / 脚本）',
      match: [/^webui\/(package(-lock)?\.json|vite\.config\.[A-Za-z]+|index\.html|tsconfig[^/]*\.json|scripts\/)/],
      scripts: webuiAll,
      why:
        '构建脚本 / 依赖 / 入口变了 ⇒ 所有前端守卫的输入都变了' +
        '（含 check-build-equivalent 用来解析 .vue/.js 的 @vue/compiler-sfc、@babel/parser 口径）。',
    },
    {
      name: 'webui 构建产物 dist',
      match: [/^webui\/dist\//],
      scripts: webuiAll,
      builtins: ['webui-dist-pairing'],
      why: 'dist 是 release 的编译期输入（server/build.rs 内嵌）；dist 变了意味着必须和 src 同批提交，所以把前端守卫全跑一遍。',
    },
    {
      name: 'polish crate（出片 / 精修引擎）',
      match: [/^crates\/yekitrim-polish\//],
      scripts: pick(webuiAll, parity, translator, effect),
      cargo: [CARGO_POLISH, CARGO_CHECK],
      why:
        'polish 里有出片权威模板 templates/index.html 与 LLM 批量循环：' +
        '① 模板被 webui/scripts/check-overlay|caption-wrap|color-parity 逐值对拍（跨端漂移）；' +
        '② 新增批量循环必须交出取消探针（边界门禁的成对判据）；③ 效果锚点登记在 spec/effects.json。',
    },
    {
      name: 'render-core crate（时间线类型 / 数学）',
      match: [/^crates\/yekitrim-render-core\//],
      scripts: pick(webuiAll, effect),
      cargo: [CARGO_RENDER_CORE, CARGO_CHECK],
      why: 'overlay_stops 等色档算法与预览 / 成片三份实现同源（check-overlay 的第一条就是它），改这里前端守卫必须一起跑。',
    },
    {
      name: 'core / llm crate（管线 / ASR / 检测 / 剪切 / 传输层）',
      match: [/^crates\/yekitrim-core\//, /^crates\/yekitrim-llm\//],
      scripts: pick(translator),
      cargo: [CARGO_CORE],
      why: '弹幕在屏窗口与转译器逐档对拍（check-yekitrim-translator 的 clamp_display_windows），改 core 要一起跑。',
    },
    {
      name: 'core 弹幕子模块',
      match: [/^crates\/yekitrim-core\/src\/danmaku\//],
      scripts: pick(translator),
      cargo: [CARGO_CORE_DANMAKU, CARGO_CORE],
      why: 'danmaku 的 ASS / XML 解析 + 密度 + 在屏窗口是转译器的对拍对象；根卡「验证」节点名要求 cargo test -p yekitrim-core --lib danmaku。',
    },
    {
      name: 'server crate（HTTP / 任务队列 / 宿主）',
      match: [/^crates\/yekitrim-server\//],
      scripts: pick(wiring),
      cargo: [CARGO_SERVER, CARGO_CHECK],
      why:
        'routes.rs 与前端 HTTP 路径隔着编译边界（check-dhampir-wiring 守的就是这条缝：' +
        'handler 少个 /api、服务端用例全绿、只有真浏览器 404 —— 我在这条缝里死过一次）。',
    },
    {
      name: 'service crate（领域逻辑 / DB / 账本）',
      match: [/^crates\/yekitrim-service\//],
      cargo: [CARGO_SERVICE, CARGO_CHECK],
      why: '领域逻辑的正确性只有单测能兜（db / 项目 / 容量 / 下载器 / B站），根卡「验证」节点名要求它。',
    },
    {
      name: 'cli / xtask',
      match: [/^crates\/(yekitrim-cli|xtask)\//],
      cargo: [CARGO_CHECK],
      why: 'cli 只剩入口、xtask 是构建 / 打包门禁：改动面靠 workspace 全量编译暴露（xtask 刻意只依赖 std+sha2+zip，不跑 node/python）。',
    },
    {
      name: 'workspace 依赖清单',
      match: [/^Cargo\.(toml|lock)$/, /^crates\/[^/]+\/Cargo\.toml$/],
      cargo: [CARGO_CHECK],
      why: '依赖一变，全工作区都可能编译不过 / 进新警告（根卡要求 0 warnings），所以跑 cargo check --workspace --all-targets。',
    },
    {
      name: '转译器 tools/',
      match: [/^tools\//],
      scripts: pick(translator),
      why: 'check-yekitrim-translator 的被测对象就是 tools/polish-to-dhampir.mjs（348 曲线 + 变异自证）；改了转译器不跑它等于没验。',
    },
    {
      name: '效果词汇表 spec/',
      match: [/^spec\//],
      scripts: pick(effect),
      why: 'spec/effects.json 是效果词汇表的唯一权威（11 类型 × 每通道锚点 + 数值）；改它必须过 check-effect-registry 的两向判据。',
    },
    {
      name: 'LLM 能力表源 vendor/pi-ai-models/',
      match: [/^vendor\/pi-ai-models\//],
      scripts: pick(llmModels),
      why: '产物 vs 入库源的一致性只有 check-llm-models 能查（缺源时它自己的 SKIP 会带原因）。',
    },
    {
      name: '模块卡 / 模块注册表',
      match: [/^docs\/module-map\.json$/, /(^|\/)AGENTS\.md$/],
      structure: true,
      why: '模块卡 / 注册表是结构门禁的输入：覆盖率、预算（根文件 20KB / 最坏链 65536B）、四节骨架、索引一致、⚠️ 行数下限全靠它。',
    },
    {
      name: '换行符约定',
      match: [/^\.gitattributes$/],
      structure: true,
      why: 'CRLF 卫生判据在 check_module_boundaries 里（.gitattributes 的 eol=lf 只在检出时生效，工作副本变 CRLF 会让变异自证型脚本静默失效）。',
    },
    {
      name: '门禁脚本自身 scripts/',
      match: [/^scripts\//],
      structure: true,
      why: '改门禁 = 改判据，先跑两条结构门禁（自证判据没被写松）；如果改的就是某个 check-*.mjs，它自己也会被选中。',
    },
    {
      name: '文档 docs/',
      match: [/^docs\//],
      structure: true,
      why: 'docs/module-map.json 在 docs/ 下，且根卡 20KB 预算与 AGENTS.md 索引一致性属于结构门禁；文档改动本身没有别的机器判据。',
    },
  ]

  // 每一个 cargo 项守的是哪条路径 —— --changed 没选中它们时，SKIP 行要说出"未触及 <路径>"
  const CARGO_CANON = [
    { args: CARGO_CHECK, path: 'crates/' },
    { args: CARGO_SERVER, path: 'crates/yekitrim-server/' },
    { args: CARGO_SERVICE, path: 'crates/yekitrim-service/' },
    { args: CARGO_CORE, path: 'crates/yekitrim-core/' },
    { args: CARGO_CORE_DANMAKU, path: 'crates/yekitrim-core/src/danmaku/' },
    { args: CARGO_POLISH, path: 'crates/yekitrim-polish/' },
    { args: CARGO_RENDER_CORE, path: 'crates/yekitrim-render-core/' },
    { args: CARGO_LLM, path: 'crates/yekitrim-llm/' },
  ]

  return { MAPPING, CARGO_CANON }
}

// ════════════════════════════════════════════════════════════════════════════
// 选中：把"改动面"或 "--all" 翻译成待跑项
// ════════════════════════════════════════════════════════════════════════════

function scriptItems(spec) {
  const items = [{ key: spec.id, label: spec.id, kind: 'script', cmd: spec.cmd, args: [spec.file], phase: 'main' }]
  // 自检 / 变异：自动发现 —— 脚本自己的文本里支持就一定要跑（"主检查进了 gates、自检没进"的旧伤）
  if (spec.selfTest) {
    items.push({
      key: spec.id + '#self-test',
      label: spec.id + ' --self-test',
      kind: 'script',
      cmd: spec.cmd,
      args: [spec.file, '--self-test'],
      phase: 'self-test',
    })
  }
  if (spec.mutate) {
    items.push({
      key: spec.id + '#mutate',
      label: spec.id + ' --mutate',
      kind: 'script',
      cmd: spec.cmd,
      args: [spec.file, '--mutate'],
      phase: 'mutate',
    })
  }
  return items
}

function cargoItem(args) {
  return { key: 'cargo ' + args.join(' '), label: 'cargo ' + args.join(' '), kind: 'cargo', cmd: 'cargo', args, phase: 'cargo' }
}

function gitLines(args) {
  const r = spawnSync('git', args, { cwd: ROOT, encoding: 'utf8' })
  if (r.status !== 0) {
    throw new UsageError('git ' + args.join(' ') + ' 失败：' + ((r.stderr || '').trim() || '（无 stderr）'))
  }
  return r.stdout.split('\n').map((s) => s.trim()).filter(Boolean)
}

function changedFilesFromGit(base) {
  // git diff <base> 只看已跟踪文件（含暂存 / 未暂存）；未跟踪的新文件另外补进来，
  // 否则"新加一个 check-*.mjs 但还没 git add"会被静默漏掉。
  const tracked = gitLines(['diff', '--name-only', base])
  const untracked = gitLines(['ls-files', '--others', '--exclude-standard'])
  return { files: [...new Set([...tracked, ...untracked])], base, untracked }
}

function worktreeChangedFiles() {
  const r = spawnSync('git', ['status', '--porcelain'], { cwd: ROOT, encoding: 'utf8' })
  if (r.status !== 0) return []
  return r.stdout
    .split('\n')
    .filter(Boolean)
    .map((l) => l.slice(3).trim())
    .map((p) => (p.includes(' -> ') ? p.split(' -> ').pop().trim() : p))
    .filter(Boolean)
}

// webui/dist 与 webui/src 的**成对判据**（计划里"npm --prefix webui run build 的指纹校验"）
// 为什么不做真指纹：真跑 vite build 会**重写入库的 webui/dist**（本脚本不许往仓库里写东西），
// 而把 src 哈希存到哪儿都得先往仓库里写一个基线文件。改成"改动面成对"判据：
// src 有源文件改动而 dist 一个都没动 ⇒ 要么忘了重建、要么准备分开提交 —— 两种都该红。
const SRC_FILE_RE = /^webui\/src\/.*\.(js|mjs|ts|vue|css|html|json|svg|woff2?)$/
const DIST_FILE_RE = /^webui\/dist\//

function distPairing(files) {
  const src = files.filter((f) => SRC_FILE_RE.test(f))
  const dist = files.filter((f) => DIST_FILE_RE.test(f))
  if (!src.length) return { status: 'skip', reason: '不适用：本次没有 webui/src 源文件改动（不用校验 dist 同步）' }
  if (dist.length) return { status: 'pass', reason: 'src ' + src.length + ' 个 + dist ' + dist.length + ' 个同批改动' }
  return {
    status: 'fail',
    reason:
      '改了 ' + src.length + ' 个 webui/src 源文件但 webui/dist 一个都没变 —— ' +
      '跑 npm --prefix webui run build 并把 dist 与 src 一起提交（dist 是 release 的编译期输入，缺失/过期即编译失败）',
  }
}

const STRUCTURE_SPECS = [
  { id: 'scripts/check_module_map.py' },
  { id: 'scripts/check_module_boundaries.py' },
]

function select(opt, specs) {
  const { MAPPING, CARGO_CANON } = buildMapping(specs)
  const specById = new Map(specs.map((s) => [s.id, s]))
  const selected = new Map()

  const put = (item, reason) => {
    const cur = selected.get(item.key)
    if (cur) {
      if (reason && !cur.reasons.includes(reason)) cur.reasons.push(reason)
    } else {
      selected.set(item.key, { ...item, reasons: reason ? [reason] : [] })
    }
  }
  const addScript = (id, reason) => {
    const spec = specById.get(id)
    if (!spec) return
    for (const it of scriptItems(spec)) put(it, reason)
  }
  const addCargo = (args, reason) => put(cargoItem(args), reason)
  const addBuiltin = (name, reason) => {
    const label = name === 'webui-dist-pairing' ? 'webui/dist ↔ webui/src 成对判据' : name
    put({ key: 'builtin:' + name, label, kind: 'builtin', builtin: name, cmd: '', args: [], phase: 'builtin' }, reason)
  }
  const addStructure = (reason) => {
    for (const s of STRUCTURE_SPECS) addScript(s.id, reason)
  }

  const unmatched = []
  const mappingLines = []
  const noteRule = (rule) => {
    const line =
      rule.name +
      '（脚本 ' + (rule.scripts || []).length +
      ' / cargo ' + (rule.cargo || []).length +
      (rule.structure ? ' / 结构门禁' : '') +
      ((rule.builtins || []).length ? ' / ' + rule.builtins.join(',') : '') +
      '）'
    if (!mappingLines.includes(line)) mappingLines.push(line)
  }

  const runMapping = (files, sourceLabel) => {
    for (const file of files) {
      let hit = false
      for (const rule of MAPPING) {
        if (!rule.match.some((re) => re.test(file))) continue
        hit = true
        const reason = sourceLabel + ':' + file + ' → ' + rule.name
        for (const id of rule.scripts || []) addScript(id, reason)
        // 改的如果就是某个门禁脚本自己，它必须被跑（"改了判据却不跑判据"是最容易自欺的一步）
        if (/^(scripts|webui\/scripts)\/check-.*\.(mjs|py)$/.test(file)) addScript(file, reason + '（被改的脚本自己）')
        for (const args of rule.cargo || []) addCargo(args, reason)
        for (const b of rule.builtins || []) addBuiltin(b, reason)
        if (rule.structure) addStructure(reason)
        noteRule(rule)
      }
      if (!hit) unmatched.push(file)
    }
  }

  // "没跑"的项也要进汇总表（status=skip + 理由），不许从列表里消失
  const putSkip = (item, reason) => put({ ...item, skippedReason: reason })

  let files = []
  let fileSource = ''
  if (opt.all) {
    for (const spec of specs) addScript(spec.id, '--all')
    addStructure('--all')
    if (opt.cargoMode === 'on') {
      for (const c of CARGO_CANON) addCargo(c.args, '--with-cargo')
    } else {
      // --all 的语义是"全部**脚本** + 自检/mutate"；Rust 测试默认不跑（很大且依赖工作树状态）。
      // 但绝不静默：逐个打印 SKIP 行，理由写清"用 --with-cargo 或 --changed 打开"。
      for (const c of CARGO_CANON) {
        putSkip(cargoItem(c.args), '--all 默认不跑 cargo（--with-cargo 打开；--changed 会按改动面选）—— 没跑就是没跑，不算通过')
      }
    }
    addBuiltin('webui-dist-pairing', '--all（按工作副本改动面比）')
    files = worktreeChangedFiles()
    fileSource = '--all（全量脚本；dist 成对判据按工作副本 git status --porcelain 比）'
    mappingLines.push('--all：全部发现到的脚本 + 自检/mutate + 结构门禁（cargo 用 --with-cargo）')
  } else {
    if (opt.files) {
      files = opt.files
      fileSource = '--files（手工指定改动面）'
      runMapping(files, '--files')
    }
    if (opt.changed) {
      const g = changedFilesFromGit(opt.changed)
      files = g.files
      fileSource = 'git diff --name-only ' + g.base + '（+ 未跟踪 ' + g.untracked.length + ' 个）'
      runMapping(g.files, 'git diff ' + g.base)
    }
    if (opt.structure) {
      addStructure('--check-structure')
      mappingLines.push('结构门禁（--check-structure：check_module_map.py + check_module_boundaries.py）')
    }
    if (unmatched.length) {
      // 未映射的改动面**不许静默**：回退到结构门禁，并显式点名是哪几个文件
      addStructure('未映射 → 回退结构门禁')
      mappingLines.push('未映射改动面 ' + unmatched.length + ' 个 → 回退到结构门禁')
    }
    // 没被选中的 cargo 项**必须显式打印** SKIP —— T8 验收就是这一条：
    // 只改前端时不许跑 Rust 测试，但要写出 SKIP cargo test -p yekitrim-server（未触及 crates/）
    for (const c of CARGO_CANON) {
      const key = 'cargo ' + c.args.join(' ')
      if (selected.has(key)) continue
      const why =
        opt.cargoMode === 'off'
          ? '--no-cargo：显式不跑（不算通过）'
          : '未触及 ' + c.path + '（本次改动面没有它）'
      putSkip(cargoItem(c.args), why)
    }
    // 没被选中的脚本：用一行说清数量（逐条刷屏没人看），--list 能看全量
    const notPicked = specs.filter((s) => !selected.has(s.id)).length
    if (notPicked) {
      putSkip(
        { key: '(未被选中的脚本)', label: '其余 ' + notPicked + ' 个脚本（本次改动面未选中）', kind: 'note', cmd: '', args: [], phase: 'note' },
        '--changed/--files 只跑映射到改动面的脚本；--all 才是全量',
      )
    }
  }

  return { items: [...selected.values()], mappingLines, unmatched, fileSource, files }
}

// ════════════════════════════════════════════════════════════════════════════
// 执行 + 判定（SKIP 的三条来源都写在这里）
// ════════════════════════════════════════════════════════════════════════════

const ANSI_RE = /\u001b\[[0-9;]*m/g

function stripAnsi(s) {
  return s.replace(ANSI_RE, '')
}

// 脚本自己打印的 SKIP 行 —— **连它接着说的"要准备什么"一起带走**。
// 缺 fixture 时那几行才是有用的信息（例：check-cover-takumi 的 "· 需要：带 cover-takumi feature 的
// yekitrim.exe …"）。只留第一行 SKIP 等于把"怎么让它跑起来"丢掉了。
const PREP_LINE_RE = /^[·•*>-]?\s*(需要|要准备|准备|先 |加 --)/
function findSkipLine(out) {
  const lines = stripAnsi(out).split('\n')
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim()
    if (!line) continue
    // 本仓约定：行首（可带 · / • 等记号）的 SKIP，或行内出现 SKIP：/SKIP:
    const bare = line.replace(/^[·•*>|+-]+\s*/, '')
    if (!(/^SKIP\b/i.test(bare) || /SKIP\s*[：:]/.test(bare))) continue
    const extra = []
    for (let j = i + 1; j < lines.length && extra.length < 3; j++) {
      const next = lines[j].trim()
      if (!next) continue
      if (!PREP_LINE_RE.test(next)) break
      extra.push(next)
    }
    const joined = [line, ...extra].join(' ｜ ')
    return joined.length > 400 ? joined.slice(0, 400) + ' …' : joined
  }
  return null
}

function tailReason(out) {
  const lines = stripAnsi(out)
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean)
  const pick = lines.slice(-3).join(' | ')
  return pick.length > 400 ? pick.slice(0, 400) + ' …' : pick
}

function killTree(child) {
  try {
    if (process.platform !== 'win32' && child.pid) process.kill(-child.pid, 'SIGKILL')
    else child.kill('SIGKILL')
  } catch {
    try {
      child.kill('SIGKILL')
    } catch {
      /* 已经没了 */
    }
  }
}

// ⚠️ **逐项超时覆盖（下限，秒）**：有的门禁天生分钟级，`--timeout × 2` 也不够 —— 在这里点名，
//    别用 `--timeout` 全局放大（那等于给**每个**脚本都发 20 分钟）。
//
//    今天只有一项，参数来自 2026-11-24 实测：
//      `scripts/check-yekitrim-translator.mjs --mutate` = **109 处变异**（逐条真改源码 + 重新 import
//      变异体 + 跑同一份 348+ 项检查），**空载 203s**；而首轮 `--all` 里它在 600s 预算内只跑到
//      第 41 处（机器同时有 cargo / 别的 teammate 在跑，每处慢 4~8 倍）⇒ 被 SIGKILL、记成 SKIP。
//      ⚠️ SKIP 不是通过：那等于「最有价值的那条自证」长期没跑（本仓已经栽过'自检没进 gates'）。
//      取 1200s ≈ 空载 6 倍余量；**若仍被 SIGKILL，下一步不是继续加超时** —— 而是
//      ① 给它 batching（脚本侧要先支持 `--only=<起>-<止>`，**目前并没有**；`--only` 是 gate 自己的过滤参数）
//      或 ② 在调度器里给它「独占」标志（默认 `--jobs 1` 已串行，但机器上可能有别的重活）。
const TIMEOUT_OVERRIDES = {
  'scripts/check-yekitrim-translator.mjs#mutate': 1200,
}

// 每项的**有效**超时：--mutate 相位默认 ×2（变异台要真改源码、逐个加载变异体跑同一份检查，
// 59→**109** 处变异分钟级是正常的；用同一个 300s 把它卡成 SKIP = 让最有价值的自证永远"没跑"）。
// 覆盖表按**下限**生效（`--timeout` 给得更大时以它为准）。
function effectiveTimeout(item, timeoutSec) {
  const base = item.phase === 'mutate' ? timeoutSec * 2 : timeoutSec
  const floor = TIMEOUT_OVERRIDES[item.key]
  return floor === undefined ? base : Math.max(base, floor)
}

function runItem(item, timeoutSec) {
  const timeoutMs = Math.round(timeoutSec * 1000)
  const t0 = Date.now()
  return new Promise((done) => {
    let out = ''
    let timedOut = false
    let spawnError = null
    let child
    try {
      child = spawn(item.cmd, item.args, {
        cwd: ROOT,
        stdio: ['ignore', 'pipe', 'pipe'],
        detached: process.platform !== 'win32',
        env: { ...process.env, NO_COLOR: '1', FORCE_COLOR: '0', CLICOLOR: '0' },
      })
    } catch (e) {
      done({ code: null, signal: null, timedOut: false, out: 'spawn 失败：' + (e && e.message), ms: 0, spawnError: e })
      return
    }
    const onData = (buf) => {
      out += buf.toString('utf8')
      if (out.length > MAX_CAPTURE) out = out.slice(-KEEP_TAIL)
    }
    child.stdout.on('data', onData)
    child.stderr.on('data', onData)
    const timer = setTimeout(() => {
      timedOut = true
      killTree(child)
    }, timeoutMs)
    child.on('error', (e) => {
      spawnError = e
      out += '\nspawn 错误：' + (e && e.message)
    })
    child.on('close', (code, signal) => {
      clearTimeout(timer)
      done({ code, signal, timedOut, out, ms: Date.now() - t0, spawnError })
    })
  })
}

function classify(res, item, timeoutSec) {
  if (res.timedOut) {
    return { status: 'skip', reason: '超时 ' + timeoutSec + 's 被 SIGKILL（--timeout 可调；按 SKIP 记，绝不算通过）' }
  }
  const skipLine = findSkipLine(res.out)
  if (res.code === 3) {
    return { status: 'skip', reason: skipLine ? skipLine + '（脚本退出码 3）' : '脚本退出码 3（本仓约定：缺样本 / 缺前置 ⇒ 没跑）' }
  }
  if (res.code === 0) {
    // 主判据过了，但脚本自己说了"有一块没跑" ⇒ 整项记 SKIP（静默跳过比不跑更糟）
    if (skipLine) return { status: 'skip', reason: skipLine }
    return { status: 'pass', reason: '' }
  }
  const why = res.spawnError
    ? '无法启动 ' + item.cmd + '：' + res.spawnError.message
    : '退出码 ' + res.code + (res.signal ? '（信号 ' + res.signal + '）' : '')
  return { status: 'fail', reason: why + '：' + tailReason(res.out) }
}

function loadBaseline() {
  if (!existsSync(BASELINE_PATH)) return {}
  let raw
  try {
    raw = JSON.parse(readFileSync(BASELINE_PATH, 'utf8'))
  } catch (e) {
    throw new UsageError(BASELINE_PATH + ' 不是合法 JSON：' + e.message)
  }
  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new UsageError('scripts/gate-baseline.json 必须是对象：{"<key>": "非空理由"}')
  }
  for (const [k, v] of Object.entries(raw)) {
    if (typeof v !== 'string' || !v.trim()) {
      // 空理由的 baseline = 第二个"静默跳过"，直接当用法错误
      throw new UsageError('scripts/gate-baseline.json 里 "' + k + '" 的理由为空 —— 每条基线必须写清"为什么本机跑不了 / 已知红"')
    }
  }
  return raw
}

const ICON = { pass: '✓ PASS ', skip: '· SKIP ', fail: '✗ FAIL ', baseline: '⊙ BASE ' }

function dispWidth(s) {
  // 中文按 2 列算，让汇总表在等宽终端里对齐
  let w = 0
  for (const ch of s) w += /[\u1100-\u115F\u2E80-\uA4CF\uAC00-\uD7A3\uF900-\uFAFF\uFE30-\uFE6F\uFF00-\uFF60\uFFE0-\uFFE6]/.test(ch) ? 2 : 1
  return w
}

function pad(s, n) {
  return s + ' '.repeat(Math.max(0, n - dispWidth(s)))
}

function fmtSec(ms) {
  return (ms / 1000).toFixed(1) + 's'
}

function summarize(rows, baselineHits, baselineAll) {
  const counts = { pass: 0, skip: 0, fail: 0, baseline: 0 }
  for (const r of rows) counts[r.status]++
  const totalMs = rows.reduce((a, r) => a + r.ms, 0)
  const width = Math.min(64, Math.max(28, ...rows.map((r) => dispWidth(r.label) + 2)))

  console.log('')
  console.log('════════════════════════ 汇总 ════════════════════════')
  console.log(
    '通过 ' + counts.pass +
      ' / 跳过 ' + counts.skip +
      ' / 失败 ' + counts.fail +
      '（另有基线 ' + counts.baseline + ' 条已知红）' +
      '  共 ' + rows.length + ' 项 · 合计 ' + fmtSec(totalMs),
  )
  console.log('')
  for (const r of rows) {
    const tail = r.reason ? '  ← ' + r.reason : ''
    console.log(ICON[r.status] + ' ' + pad(r.label, width) + ' ' + fmtSec(r.ms).padStart(7) + tail)
  }
  const missed = Object.keys(baselineAll).filter((k) => !baselineHits.has(k))
  if (missed.length) {
    console.log('')
    console.log('· 基线里 ' + missed.length + ' 条这次没命中（可能已修好，可删）：' + missed.join(', '))
  }
  console.log('')
  const failed = rows.filter((r) => r.status === 'fail')
  if (failed.length) {
    console.log('✗ 非基线失败 ' + failed.length + ' 项：' + failed.map((r) => r.label).join(', '))
    console.log('  要么改代码，要么（仅限环境性 / 本机不具备）登记进 scripts/gate-baseline.json 并写清理由。')
  } else {
    console.log('✓ 没有非基线失败（基线内的已知红不计；SKIP 永远不算通过）')
  }
  return counts
}

async function runPool(items, opt, baseline, files) {
  const rows = []
  const baselineHits = new Set()
  const writeLog = (key, out) => {
    try {
      mkdirSync(opt.logs, { recursive: true })
      const p = join(opt.logs, key.replace(/[^A-Za-z0-9._-]+/g, '_') + '.log')
      writeFileSync(p, out)
      return p
    } catch {
      return null
    }
  }

  const one = async (item) => {
    if (item.skippedReason) return { status: 'skip', reason: item.skippedReason, ms: 0 }
    if (item.kind === 'builtin') {
      if (!opt.all && !opt.changed) {
        return { status: 'skip', reason: '--files 是手工改动面（没有可比的对侧）；dist 同批判据要用 --changed 或 --all 才成立', ms: 0 }
      }
      const r = distPairing(files)
      return { status: r.status, reason: r.reason, ms: 0 }
    }
    const budget = effectiveTimeout(item, opt.timeoutSec)
    const res = await runItem(item, budget)
    const c = classify(res, item, budget)
    let status = c.status
    let reason = c.reason
    if (status === 'fail' && Object.prototype.hasOwnProperty.call(baseline, item.key)) {
      status = 'baseline'
      baselineHits.add(item.key)
      reason = '基线：' + baseline[item.key]
    }
    if (status !== 'pass') {
      const logPath = writeLog(item.key, res.out)
      if (logPath) reason += '（完整输出：' + logPath + '）'
    }
    return { status, reason, ms: res.ms }
  }

  let next = 0
  const worker = async () => {
    while (true) {
      const i = next++
      if (i >= items.length) return
      const item = items[i]
      const r = await one(item)
      rows[i] = { label: item.label, key: item.key, status: r.status, reason: r.reason, ms: r.ms }
      // 逐项进度走 stderr（stdout 只留最后那张汇总表；两者都在时汇总表在末尾且是权威）
      process.stderr.write('  ' + ICON[r.status] + pad(item.label, 52) + ' ' + fmtSec(r.ms).padStart(7) + (r.reason ? '  ← ' + r.reason : '') + '\n')
    }
  }
  await Promise.all(Array.from({ length: Math.min(opt.jobs, items.length) }, worker))
  return { rows, baselineHits }
}

// ════════════════════════════════════════════════════════════════════════════
// main
// ════════════════════════════════════════════════════════════════════════════

async function main() {
  const opt = parseArgs(process.argv.slice(2))
  if (opt.help) {
    printUsage()
    return 0
  }

  const specs = discoverScripts()
  const nNode = specs.filter((s) => s.cmd === 'node').length
  const nPy = specs.filter((s) => s.cmd === 'python3').length
  const nSelf = specs.filter((s) => s.selfTest).length
  const nMut = specs.filter((s) => s.mutate).length
  console.log(
    'scripts/gate.mjs —— 发现 ' + specs.length + ' 个门禁脚本（node ' + nNode + ' / python3 ' + nPy + '）' +
      '；自动识别自检 ' + nSelf + ' 个、变异 ' + nMut + ' 个；每项超时 ' + opt.timeoutSec + 's，并发 ' + opt.jobs,
  )

  const baseline = loadBaseline()
  const sel = select(opt, specs)
  console.log('选中方式：' + (sel.fileSource || '（没有改动面）'))
  if (sel.mappingLines.length) {
    console.log('映射规则：')
    for (const l of sel.mappingLines) console.log('  · ' + l)
  }
  if (sel.unmatched.length) {
    console.log('未映射到具体脚本的改动面 ' + sel.unmatched.length + ' 个（已回退到结构门禁）：')
    for (const f of sel.unmatched.slice(0, 20)) console.log('  ? ' + f)
    if (sel.unmatched.length > 20) console.log('  … 另有 ' + (sel.unmatched.length - 20) + ' 个')
  }

  let items = sel.items
  if (opt.only.length) {
    const before = items.length
    items = items.filter((it) => opt.only.some((s) => it.key.includes(s)))
    console.log('--only 过滤：' + before + ' → ' + items.length + ' 项（未跑的 ' + (before - items.length) + ' 项不计入本次结论）')
  }
  if (items.length === 0) {
    console.log('没有选中任何项 —— 什么都没跑。这不算通过。')
    return 0
  }

  console.log('')
  console.log('── 待跑 ' + items.length + ' 项 ──')
  if (opt.list) {
    for (const it of items) console.log('  · ' + it.key + (it.reasons.length ? '   [' + it.reasons[0] + ']' : ''))
    console.log('')
    console.log('--list：只列不跑。')
    return 0
  }

  const { rows, baselineHits } = await runPool(items, opt, baseline, sel.files)
  summarize(rows, baselineHits, baseline)
  return rows.some((r) => r.status === 'fail') ? 1 : 0
}

main()
  .then((code) => process.exit(code))
  .catch((e) => {
    if (e instanceof UsageError) {
      console.error('✗ ' + e.message)
      process.exit(2)
    }
    console.error(e)
    process.exit(2)
  })
