// 固定使用同一个 Cargo 目录，复用依赖和增量缓存；编译前请从托盘退出应用。
import { spawnSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { resolve } from 'node:path'
import { rustEnvironment } from './rust-env.mjs'
import { syncVersion } from './sync-version.mjs'

const root = fileURLToPath(new URL('../', import.meta.url))
const require = createRequire(import.meta.url)
// 直接用 Node 执行本地 CLI，不经过 shell 拼接，Windows 路径带空格也能正确传递。
const cli = require.resolve('@tauri-apps/cli/tauri.js')
const target = resolve(root, 'src-tauri', 'target')
syncVersion(root)
const buildArgs = process.argv.slice(2)
// 翻译引擎在设置中按需下载或导入，不再随主程序打包，也不依赖本机准备的 runtime。
// Windows 正式版和测试版统一生成 NSIS，独立 EXE 直接复用主程序。
// 本地无签名密钥时允许正常打包，CI 提供密钥后才生成可在线安装的签名产物。
if (process.platform === 'win32') {
  buildArgs.unshift('--config', JSON.stringify({ bundle: {
    targets: ['nsis'],
    createUpdaterArtifacts: Boolean(process.env.TAURI_SIGNING_PRIVATE_KEY),
  } }))
}
const env = rustEnvironment()
console.log(`构建输出：${target}`)
console.log('请先从托盘退出 Snap Desk，避免 Windows 阻止覆盖正在运行的 EXE。')
// 系统编码 API 由 Windows 提供；构建脚本不下载或捆绑外部编码程序。
console.log('Windows 录屏使用系统 Media Foundation；无需附带外部编码程序。')
const result = spawnSync(process.execPath, [cli, 'build', ...buildArgs], {
  cwd: root,
  env: { ...env, CARGO_TARGET_DIR: target },
  stdio: 'inherit',
  windowsHide: true,
})
if (result.error) throw result.error
process.exit(result.status ?? 1)
