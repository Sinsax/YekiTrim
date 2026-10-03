import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  base: './',
  build: {
    outDir: 'dist',
    emptyOutDir: true
  },
  server: {
    port: 5173,
    // 代理到 `yekitrim server` 的默认端口。⚠️ 这个数是 `yekitrim-service/src/doctor.rs::DEFAULT_PORT`
    // 的**镜像**（唯一权威在那份 Rust 常量里 + 它的单测）；server 换 --port 时用 YEKITRIM_PORT 覆盖这里，
    // 别再在代码里写第二处字面量。
    proxy: {
      '/api': `http://127.0.0.1:${process.env.YEKITRIM_PORT || 26320}`,
      '/media': `http://127.0.0.1:${process.env.YEKITRIM_PORT || 26320}`
    }
  }
})
