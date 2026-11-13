// 固定使用同一个 Cargo 目录，复用依赖和增量缓存；编译前请从托盘退出应用。
import { spawnSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { resolve } from 'node:path'
import { bundleConfig } from './release-assets.mjs'
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
// 各平台的打包目标与签名产物由发布附件清单统一决定：Windows 为 NSIS + 独立 EXE，
// macOS 为 .app + DMG，Linux 为 AppImage + deb。
const config = bundleConfig(process.platform)
if (config) buildArgs.unshift('--config', JSON.stringify(config))
const env = rustEnvironment()
console.log(`构建输出：${target}`)
if (process.platform === 'win32') {
  console.log('请先从托盘退出 Snap Desk，避免 Windows 阻止覆盖正在运行的 EXE。')
  // 系统编码 API 由 Windows 提供；构建脚本不下载或捆绑外部编码程序。
  console.log('Windows 录屏使用系统 Media Foundation；无需附带外部编码程序。')
}
const result = spawnSync(process.execPath, [cli, 'build', ...buildArgs], {
  cwd: root,
  env: { ...env, CARGO_TARGET_DIR: target,
    SNAP_DESK_BUILD_EXECUTABLE: resolve(target, buildArgs.includes('--debug') ? 'debug' : 'release', process.platform === 'win32' ? 'snap-desk.exe' : 'snap-desk') },
  stdio: 'inherit',
  windowsHide: true,
})
if (result.error) throw result.error
process.exit(result.status ?? 1)
