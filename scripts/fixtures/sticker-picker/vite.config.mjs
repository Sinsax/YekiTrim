// headless 夹具的构建配置（跑法：scripts/check-sticker-picker-layout.mjs）。
// 夹具住在 scripts/ 下，所以 vue 与 @vitejs/plugin-vue 都要**按绝对路径**从 webui/node_modules 取。
import { createRequire } from 'node:module'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { dirname, join } from 'node:path'

const HERE = dirname(fileURLToPath(import.meta.url))
const WEBUI = join(HERE, '..', '..', '..', 'webui')
const req = createRequire(join(WEBUI, 'package.json'))
const mod = await import(pathToFileURL(req.resolve('@vitejs/plugin-vue')).href)
const vue = mod.default || mod

export default {
  root: HERE,
  base: './',
  plugins: [vue()],
  // 夹具的裸 import 'vue' 靠这条别名落到 webui 那份（从夹具目录往上走找不到 node_modules）
  resolve: { alias: { vue: join(WEBUI, 'node_modules', 'vue') } },
  build: {
    outDir: join(WEBUI, 'node_modules', '.sticker-picker-harness'),
    emptyOutDir: true,
    target: 'esnext',
    minify: false,
    // ⚠️ 多页：vite **默认只构建 root/index.html** —— 不写这条，panel-row.html 不会进产物，
    //    症状是"夹具页起不来"（page 404 / __ROW 永不出现），而不是构建报错。
    rollupOptions: { input: { picker: join(HERE, 'index.html'), row: join(HERE, 'panel-row.html'), form: join(HERE, 'panel-form.html') } },
  },
}
