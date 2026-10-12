import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
  clearScreen: false,
  // 独立工具窗也必须作为入口输出，确保开发环境与打包后的页面路径一致。
  build: {
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL('./index.html', import.meta.url)),
        recordingControls: fileURLToPath(new URL('./recording-controls.html', import.meta.url)),
      },
    },
  },
  server: { port: 1420, strictPort: true, watch: { ignored: ['**/src-tauri/**'] } },
})
