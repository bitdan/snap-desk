// 固定使用同一个 Cargo 目录，复用依赖和增量缓存；编译前请从托盘退出应用。
import { spawnSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { resolve } from 'node:path'

const root = fileURLToPath(new URL('../', import.meta.url))
const require = createRequire(import.meta.url)
// 直接用 Node 执行本地 CLI，不经过 shell 拼接，Windows 路径带空格也能正确传递。
const cli = require.resolve('@tauri-apps/cli/tauri.js')
const target = resolve(root, 'src-tauri', 'target')
console.log(`构建输出：${target}`)
console.log('请先从托盘退出 Snap Desk，避免 Windows 阻止覆盖正在运行的 EXE。')
const result = spawnSync(process.execPath, [cli, 'build', ...process.argv.slice(2)], {
  cwd: root,
  env: { ...process.env, CARGO_TARGET_DIR: target },
  stdio: 'inherit',
  windowsHide: true,
})
if (result.error) throw result.error
process.exit(result.status ?? 1)
