// 按需清理 Rust 构建产物；不与 build 串联，日常编译保留缓存会更快。
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { resolve } from 'node:path'
import { rustEnvironment } from './rust-env.mjs'

const root = fileURLToPath(new URL('../', import.meta.url))
const target = resolve(root, 'src-tauri', 'target')
const env = rustEnvironment()
console.log(`清理构建目录：${target}`)
console.log('请先退出 Snap Desk 并停止编译。该目录内的 EXE、安装包和旧版本子目录也会清除。')
// 显式指定目录，不受终端当前路径或旧 CARGO_TARGET_DIR 设置影响。
// 使用 Cargo 自带的 clean，不自行拼接 shell 删除命令；源码和用户配置不在清理范围内。
const result = spawnSync('cargo', ['clean', '--manifest-path', resolve(root, 'src-tauri', 'Cargo.toml'), '--target-dir', target], {
  cwd: root,
  env,
  stdio: 'inherit',
  windowsHide: true,
})
if (result.error) throw result.error
process.exit(result.status ?? 1)
