// GPUI 版打包：Cargo 构建 snap-desk-gpui，复制为主程序 snap-desk(.exe)，再用 cargo-packager 生成安装包；
// macOS 另把 .app 压成在线更新用的 .app.tar.gz。更新签名不在这里做，由发布流程对最终附件统一签名
// （scripts/release-signing.mjs）。产物在 target/release/packages，Windows 独立 EXE 在 target/release。
//
// 需要先安装 cargo-packager：cargo install cargo-packager --version 0.11.8 --locked
// 用法：node scripts/package-gpui.mjs [--debug]
import { spawnSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, rmSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { PACKAGES_DIRECTORY, packageFormats } from './release-assets.mjs'
import { rustEnvironment } from './rust-env.mjs'
import { readVersion, syncVersion } from './sync-version.mjs'

export const PACKAGER_VERSION = '0.11.8'

/** Linux deb 的运行依赖：录屏（PipeWire/GStreamer）、X11 截图与输入、密钥存储、GPUI 渲染（Vulkan、字体）。 */
const DEB_DEPENDS = [
  'libpipewire-0.3-0', 'libxcb1', 'libxcb-shm0', 'libxcb-xinput0', 'libxrandr2', 'libdbus-1-3',
  'libgstreamer1.0-0', 'libgstreamer-plugins-base1.0-0', 'gstreamer1.0-plugins-base', 'gstreamer1.0-plugins-good',
  'gstreamer1.0-pulseaudio', 'libxkbcommon0', 'libxkbcommon-x11-0', 'libfontconfig1', 'libvulkan1',
]

/** 构建产物名与交给打包器的主程序名。 */
export function executableNames(platform) {
  const suffix = platform === 'win32' ? '.exe' : ''
  return { built: `snap-desk-gpui${suffix}`, bundled: `snap-desk${suffix}` }
}

/**
 * cargo-packager 配置（JSON 字段为 camelCase）；不在发布范围内的平台返回 null。
 * Apple 芯片拒绝运行未签名的应用。CI 用固定的自签名证书（.github/actions/macos-signing 设置 APPLE_SIGNING_IDENTITY），
 * 未提供 APPLE_SIGNING_IDENTITY 时使用临时签名（ad-hoc），
 * 用户首次打开仍需在系统设置中放行。
 */
export function packagerConfig({ platform, root, releaseDirectory, version, env = process.env }) {
  const formats = packageFormats(platform)
  if (!formats.length) return null
  const path = (...parts) => resolve(root, ...parts)
  const config = {
    productName: 'Snap Desk',
    version,
    identifier: 'io.snapdesk.app',
    description: 'Screenshot, pin, OCR and translation tool',
    homepage: 'https://github.com/bitdan/snap-desk',
    category: 'Utility',
    formats,
    outDir: resolve(releaseDirectory, PACKAGES_DIRECTORY),
    binariesDir: releaseDirectory,
    binaries: [{ path: executableNames(platform).bundled, main: true }],
    licenseFile: path('LICENSE'),
    icons: ['32x32.png', '128x128.png', '128x128@2x.png', 'icon.icns', 'icon.ico'].map(name => path('resources', 'icons', name)),
    resources: [
      { src: path('LICENSE'), target: 'LICENSE' },
      { src: path('resources', 'licenses', 'LXGW-OFL.txt'), target: 'licenses/LXGW-OFL.txt' },
    ],
  }
  if (platform === 'win32') config.nsis = { installMode: 'currentUser' }
  if (platform === 'darwin') {
    // Info.plist 合并麦克风用途说明；缺少时系统会在录屏访问麦克风时直接终止进程。
    config.macos = { minimumSystemVersion: '13.3', entitlements: path('resources', 'Entitlements.plist'),
      infoPlistPath: path('resources', 'Info.plist'), signingIdentity: env.APPLE_SIGNING_IDENTITY || '-' }
  }
  if (platform === 'linux') config.deb = { depends: DEB_DEPENDS }
  return config
}

function run(command, args, options) {
  const result = spawnSync(command, args, { stdio: 'inherit', windowsHide: true, ...options })
  if (result.error) throw result.error
  if (result.status !== 0) process.exit(result.status ?? 1)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL('../', import.meta.url))
  const target = resolve(root, 'target')
  const debug = process.argv.includes('--debug')
  const releaseDirectory = resolve(target, debug ? 'debug' : 'release')
  syncVersion(root)
  const version = readVersion()
  const env = { ...rustEnvironment(), CARGO_TARGET_DIR: target }
  console.log(`构建输出：${target}`)
  if (process.platform === 'win32') console.log('请先从托盘退出 Snap Desk，避免 Windows 阻止覆盖正在运行的 EXE。')
  run('cargo', ['build', '-p', 'snap-desk-gpui', '--locked', ...(debug ? [] : ['--release'])], { cwd: root, env })

  const names = executableNames(process.platform)
  const built = resolve(releaseDirectory, names.built)
  if (!existsSync(built)) throw new Error(`未找到构建产物：${built}`)
  copyFileSync(built, resolve(releaseDirectory, names.bundled))
  if (process.platform === 'win32') {
    const standalone = resolve(releaseDirectory, `snap-desk-v${version}-windows-x64-standalone.exe`)
    copyFileSync(built, standalone)
    console.log(`独立 EXE：${standalone}`)
  }

  const config = packagerConfig({ platform: process.platform, root, releaseDirectory, version })
  if (!config) {
    console.log(`当前平台 ${process.platform} 不在发布范围内，只构建主程序：${built}`)
    process.exit(0)
  }
  // 先清空输出目录，收集附件时不会误取旧版本残留。
  rmSync(config.outDir, { recursive: true, force: true })
  mkdirSync(config.outDir, { recursive: true })
  const packager = spawnSync('cargo', ['packager', '--version'], { env, encoding: 'utf8', windowsHide: true })
  if (packager.status !== 0 || !packager.stdout.includes(PACKAGER_VERSION)) {
    throw new Error(`需要 cargo-packager ${PACKAGER_VERSION}：cargo install cargo-packager --version ${PACKAGER_VERSION} --locked`)
  }
  // 以 JSON 字符串传入配置：传文件路径时 cargo-packager 会切换到文件所在目录，Windows 上对规范化路径会失败。
  run('cargo', ['packager', '--config', JSON.stringify(config)], { cwd: root, env })

  if (process.platform === 'darwin') {
    const archive = resolve(config.outDir, `snap-desk_${version}_aarch64.app.tar.gz`)
    // 包内第一层为 “Snap Desk.app/”，与更新器的解压逻辑一致。
    run('tar', ['-czf', archive, '-C', config.outDir, `${config.productName}.app`])
    console.log(`更新包：${archive}`)
  }
  console.log(`安装包目录：${config.outDir}`)
}
