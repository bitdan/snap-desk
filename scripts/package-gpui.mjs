// GPUI 版打包：Cargo 构建 snap-desk-gpui，以 Tauri 打包器期望的主程序名放入同一 target 目录，再用
// `tauri bundle` 生成安装包。安装包沿用 tauri.conf.json 的产品名、标识与安装目录，旧版用户在线更新或
// 重新安装时原地替换；更新签名（TAURI_SIGNING_PRIVATE_KEY）与发布附件清单（release-assets.mjs）不变。
//
// 用法：node scripts/package-gpui.mjs [--debug]
import { spawnSync } from 'node:child_process'
import { copyFileSync, existsSync } from 'node:fs'
import { createRequire } from 'node:module'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { bundleConfig } from './release-assets.mjs'
import { rustEnvironment } from './rust-env.mjs'
import { readVersion, syncVersion } from './sync-version.mjs'

/**
 * 传给 `tauri bundle --config` 的配置：在发布清单的平台配置上，去掉 Tauri 版保留独立 EXE 的打包前命令
 * （GPUI 主程序没有安装类型标记，独立 EXE 直接复制），并且不检查或安装 WebView2（GPUI 版不需要）。
 */
export function gpuiBundleConfig(platform, env = process.env) {
  const config = bundleConfig(platform, env)
  if (!config) return null
  delete config.build
  if (platform === 'win32') config.bundle.windows = { webviewInstallMode: { type: 'skip' } }
  return config
}

/** 打包器按 Cargo 包 snap-desk 的二进制名查找主程序。 */
export function executableNames(platform) {
  const suffix = platform === 'win32' ? '.exe' : ''
  return { built: `snap-desk-gpui${suffix}`, bundled: `snap-desk${suffix}` }
}

function run(command, args, options) {
  const result = spawnSync(command, args, { stdio: 'inherit', windowsHide: true, ...options })
  if (result.error) throw result.error
  if (result.status !== 0) process.exit(result.status ?? 1)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL('../', import.meta.url))
  const require = createRequire(import.meta.url)
  const cli = require.resolve('@tauri-apps/cli/tauri.js')
  const target = resolve(root, 'src-tauri', 'target')
  const debug = process.argv.includes('--debug')
  const profile = debug ? 'debug' : 'release'
  syncVersion(root)
  const env = { ...rustEnvironment(), CARGO_TARGET_DIR: target }
  console.log(`构建输出：${target}`)
  if (process.platform === 'win32') console.log('请先从托盘退出 Snap Desk，避免 Windows 阻止覆盖正在运行的 EXE。')
  run('cargo', ['build', '-p', 'snap-desk-gpui', '--locked', ...(debug ? [] : ['--release'])], { cwd: resolve(root, 'src-tauri'), env })

  const names = executableNames(process.platform)
  const built = resolve(target, profile, names.built)
  if (!existsSync(built)) throw new Error(`未找到构建产物：${built}`)
  copyFileSync(built, resolve(target, profile, names.bundled))
  if (process.platform === 'win32') {
    const standalone = resolve(target, profile, `snap-desk-v${readVersion()}-windows-x64-standalone.exe`)
    copyFileSync(built, standalone)
    console.log(`独立 EXE：${standalone}`)
  }

  const config = gpuiBundleConfig(process.platform)
  if (!config) {
    console.log(`当前平台 ${process.platform} 不在发布范围内，只构建主程序：${built}`)
    process.exit(0)
  }
  run(process.execPath, [cli, 'bundle', '--config', JSON.stringify(config), ...(debug ? ['--debug'] : [])], { cwd: root, env })
}
