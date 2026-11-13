// 各平台发布附件的唯一清单：构建配置、收集、签名、更新清单和恢复流程都从这里读取，
// 新增平台或分发形式时只改这一处，并同步 src-tauri/src/app/updater.rs 的目标与后缀。
import { copyFileSync, mkdirSync, readdirSync, statSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { readVersion } from './sync-version.mjs'

/**
 * platform/arch：产出该附件的 Node 平台与架构；bundle：Tauri 打包目标。
 * source：相对 target/release 的目录；match：打包器输出文件的后缀，可按版本生成以排除旧版本残留。
 * suffix：发布附件名后缀，在全部附件中互不为后缀，可据此唯一识别。
 * name：发布时的统一文件名；缺省沿用打包器文件名（已发布的 Windows 客户端依赖原后缀）。
 * target：在线更新清单目标；缺省表示只供手动下载，不签名。
 */
export const RELEASE_ASSETS = [
  { platform: 'win32', arch: 'x64', bundle: 'nsis', source: 'bundle/nsis', match: '-setup.exe', suffix: '-setup.exe',
    target: 'windows-x86_64-nsis', label: 'Windows x64 NSIS 安装包' },
  { platform: 'win32', arch: 'x64', source: '.', match: version => `snap-desk-v${version}-windows-x64-standalone.exe`, suffix: '-standalone.exe',
    target: 'windows-x86_64-standalone', label: 'Windows x64 独立 EXE' },
  { platform: 'darwin', arch: 'arm64', bundle: 'dmg', source: 'bundle/dmg', match: '.dmg', suffix: '-macos-arm64.dmg',
    name: version => `snap-desk-v${version}-macos-arm64.dmg`, label: 'macOS Apple 芯片 DMG' },
  // 更新器整体替换 .app 目录，使用打包器在生成签名产物时一并输出的压缩包。
  { platform: 'darwin', arch: 'arm64', bundle: 'app', source: 'bundle/macos', match: '.app.tar.gz', suffix: '-macos-arm64.app.tar.gz',
    name: version => `snap-desk-v${version}-macos-arm64.app.tar.gz`, target: 'darwin-aarch64-app', label: 'macOS 应用更新包' },
  { platform: 'linux', arch: 'x64', bundle: 'appimage', source: 'bundle/appimage', match: '.AppImage', suffix: '-linux-x64.AppImage',
    name: version => `snap-desk-v${version}-linux-x64.AppImage`, target: 'linux-x86_64-appimage', label: 'Linux x64 AppImage' },
  { platform: 'linux', arch: 'x64', bundle: 'deb', source: 'bundle/deb', match: '.deb', suffix: '-linux-x64.deb',
    name: version => `snap-desk-v${version}-linux-x64.deb`, label: 'Linux x64 deb 安装包' },
]

/** 当前平台需要打包的 Tauri 目标；不在发布范围内的平台返回空列表，由调用方决定如何处理。 */
export function bundleTargets(platform) {
  return RELEASE_ASSETS.filter(asset => asset.platform === platform && asset.bundle).map(asset => asset.bundle)
}

/**
 * 传给 `tauri build --config` 的平台打包配置；不在发布范围内的平台返回 null，沿用 tauri.conf.json。
 * 本地无签名密钥时允许正常打包，CI 提供密钥后才生成可在线安装的签名产物。
 */
export function bundleConfig(platform, env = process.env) {
  const targets = bundleTargets(platform)
  if (!targets.length) return null
  const config = {
    bundle: { targets, createUpdaterArtifacts: Boolean(env.TAURI_SIGNING_PRIVATE_KEY) },
    plugins: { updater: { pubkey: env.SNAP_DESK_UPDATER_PUBLIC_KEY?.trim() ?? '' } },
  }
  // Tauri 打包时会原地改写主程序的安装类型标记，独立 EXE 必须在打包前另存。
  if (platform === 'win32') config.build = { beforeBundleCommand: 'node scripts/prepare-standalone.mjs' }
  // Apple 芯片拒绝运行未签名的应用；未提供开发者证书时使用临时签名（ad-hoc），
  // 用户首次打开仍需在系统设置中放行。配置了 APPLE_SIGNING_IDENTITY 后由 Tauri 使用正式证书。
  if (platform === 'darwin' && !env.APPLE_SIGNING_IDENTITY) config.bundle.macOS = { signingIdentity: '-' }
  return config
}

/** 在目录中找到唯一匹配后缀的文件；缺失或重复都停止，避免把旧版本残留当成本次产物发布。 */
export function findAsset(directory, suffix, label) {
  let names = []
  try { names = readdirSync(directory).filter(name => name.endsWith(suffix) && statSync(join(directory, name)).isFile()) }
  catch (error) { if (error.code !== 'ENOENT') throw error }
  if (names.length !== 1) throw new Error(`${label} 必须恰好有一个，实际找到 ${names.length} 个：${directory}`)
  return join(directory, names[0])
}

/** 把本机构建产物按统一名称收集到一个目录，供流水线跨系统汇总后一次发布。 */
export function collectReleaseAssets({ platform, arch, version, releaseDirectory, output }) {
  const assets = RELEASE_ASSETS.filter(asset => asset.platform === platform)
  if (!assets.length) throw new Error(`当前平台 ${platform} 不在发布范围内。`)
  // 附件名写死了架构；在其他架构的机器上构建会得到名不副实的包。
  if (assets.some(asset => asset.arch !== arch)) throw new Error(`当前架构 ${arch} 与 ${platform} 的发布架构不一致。`)
  mkdirSync(output, { recursive: true })
  return assets.map(asset => {
    const source = findAsset(resolve(releaseDirectory, asset.source), typeof asset.match === 'function' ? asset.match(version) : asset.match, asset.label)
    const destination = join(output, asset.name ? asset.name(version) : source.split(/[\\/]/).at(-1))
    copyFileSync(source, destination)
    return destination
  })
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [command, output] = process.argv.slice(2)
  if (command === 'list') {
    // PowerShell 发布脚本读取同一份清单，不在脚本里重复维护后缀。
    console.log(JSON.stringify(RELEASE_ASSETS.map(({ suffix, label, target }) => ({ suffix, label, update: Boolean(target) }))))
  } else if (command === 'collect' && output) {
    const root = fileURLToPath(new URL('../', import.meta.url))
    const files = collectReleaseAssets({ platform: process.platform, arch: process.arch, version: readVersion(),
      releaseDirectory: resolve(root, 'src-tauri', 'target', 'release'), output: resolve(output) })
    for (const file of files) console.log(`已收集：${file}`)
  } else {
    throw new Error('用法：node scripts/release-assets.mjs list | collect <output>')
  }
}
