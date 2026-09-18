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
    // 代理到 `vtrim server` 的默认端口（26320）；server 换 --port 时这里要跟着改
    proxy: {
      '/api': 'http://127.0.0.1:26320',
      '/media': 'http://127.0.0.1:26320'
    }
  }
})
