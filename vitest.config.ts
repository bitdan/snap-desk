import { defineConfig, mergeConfig } from 'vitest/config'
import viteConfig from './vite.config'

// scripts/*.test.mjs 使用 node:test，由 npm test 的第二步运行。
// 限定前端测试目录，同时避免扫描庞大的 Rust target/ 构建目录。
export default mergeConfig(viteConfig, defineConfig({
  test: { include: ['tests/**/*.test.ts'] },
}))
