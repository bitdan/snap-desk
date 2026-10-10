// 根据项目版本隔离 Cargo 输出，避免新版本编译覆盖正在运行的旧 EXE。
import { readFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { resolve } from 'node:path'

const root = fileURLToPath(new URL('../', import.meta.url))
const { version } = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8'))
if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('构建版本必须为数字格式，例如 0.1.3')
const require = createRequire(import.meta.url)
// 直接用 Node 执行本地 CLI，不经过 shell 拼接，Windows 路径带空格也能正确传递。
const cli = require.resolve('@tauri-apps/cli/tauri.js')
const target = resolve(root, 'src-tauri', 'target', `v${version}`)
console.log(`构建输出：${target}`)
const result = spawnSync(process.execPath, [cli, 'build', ...process.argv.slice(2)], {
  cwd: root,
  env: { ...process.env, CARGO_TARGET_DIR: target },
  stdio: 'inherit',
  windowsHide: true,
})
if (result.error) throw result.error
process.exit(result.status ?? 1)
