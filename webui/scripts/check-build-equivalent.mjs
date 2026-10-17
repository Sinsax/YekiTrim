#!/usr/bin/env node
// 无子进程的构建等效校验：用 @vue/compiler-sfc 编译每个 .vue，用 @babel/parser 解析每个 .js，
// 并校验相对 import 目标存在。等价于 `vite build` 会抓到的那几类错误（模板解析 / 脚本语法 / 导入缺失），
// 但**不 spawn 子进程** —— 本沙箱禁止 spawn（esbuild 服务起不来）。
import { readFileSync, existsSync, readdirSync, statSync } from 'node:fs'
import { join, dirname, resolve } from 'node:path'
import { createRequire } from 'node:module'

const root = process.cwd()
const require = createRequire(join(root, 'webui', 'package.json'))
const sfc = require('@vue/compiler-sfc')
const babel = require('@babel/parser')

let files = 0, errors = 0
const walk = (d) => {
  for (const e of readdirSync(d, { withFileTypes: true })) {
    const p = join(d, e.name)
    if (e.isDirectory()) { if (e.name !== 'node_modules' && e.name !== 'dist') walk(p) }
    else if (/\.(vue|js|mjs)$/.test(e.name)) check(p)
  }
}

const rel = (p) => p.slice(root.length + 1).replace(/\\/g, '/')

function checkImports(src, file) {
  for (const m of src.matchAll(/from\s+['"](\.[^'"]+)['"]/g)) {
    const spec = m[1]
    const base = resolve(dirname(file), spec)
    const cands = [base, base + '.js', base + '.mjs', base + '.vue', base + '/index.js']
    if (!cands.some(existsSync)) {
      console.log('  FAIL ' + rel(file) + ' → 找不到模块 ' + spec)
      errors++
    }
  }
}

function check(file) {
  files++
  const src = readFileSync(file, 'utf8')
  if (file.endsWith('.vue')) {
    const { descriptor, errors: e1 } = sfc.parse(src, { filename: file })
    if (e1.length) { for (const e of e1) { console.log('  FAIL ' + rel(file) + ' 模板/解析: ' + e.message); errors++ } return }
    if (descriptor.script || descriptor.scriptSetup) {
      try {
        const s = sfc.compileScript(descriptor, { id: 'x' })
        if (descriptor.template) {
          const t = sfc.compileTemplate({
            source: descriptor.template.content, filename: file, id: 'x',
            compilerOptions: { bindingMetadata: s.bindings }
          })
          for (const e of t.errors) { console.log('  FAIL ' + rel(file) + ' 模板编译: ' + (e.message || e)); errors++ }
        }
        // 脚本语法：编译产物再过一遍 babel 解析
        babel.parse(s.content, { sourceType: 'module', plugins: ['typescript', 'jsx'] })
      } catch (e) { console.log('  FAIL ' + rel(file) + ' 脚本: ' + e.message); errors++ }
    }
    checkImports(src, file)
  } else {
    try { babel.parse(src, { sourceType: 'module', plugins: ['jsx'] }) }
    catch (e) { console.log('  FAIL ' + rel(file) + ' 语法: ' + e.message); errors++ }
    checkImports(src, file)
  }
}

walk(join(root, 'webui', 'src'))
console.log(errors ? '\n✗ ' + errors + ' 处（共 ' + files + ' 个文件）' : '\n✓ 全部通过（' + files + ' 个文件）')
process.exit(errors ? 1 : 0)
