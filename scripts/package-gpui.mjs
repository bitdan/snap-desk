// GPUI 版打包：Cargo 构建 snap-desk-gpui，复制为主程序 snap-desk(.exe)，再用 cargo-packager 生成安装包；
// macOS 另把 .app 压成在线更新用的 .app.tar.gz。更新签名不在这里做，由发布流程对最终附件统一签名
// （scripts/release-signing.mjs）。产物在 target/release/packages，Windows 独立 EXE 在 target/release。
//
// 需要先安装 cargo-packager：cargo install cargo-packager --version 0.11.8 --locked
// 用法：node scripts/package-gpui.mjs [--debug] [--standalone]
//   --standalone：只构建并生成 Windows 独立 EXE，不生成安装包，也不需要 cargo-packager（npm run desktop:standalone）。
import { spawnSync } from 'node:child_process'
import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { PACKAGES_DIRECTORY, packageFormats } from './release-assets.mjs'
import { rustEnvironment } from './rust-env.mjs'
import { readVersion, syncVersion } from './sync-version.mjs'

export const PACKAGER_VERSION = '0.11.8'

/** cargo-packager 0.11.8 打包 AppImage 时使用的 AppRun（x86_64），与它内置的下载地址相同。 */
export const APPRUN_URL = 'https://github.com/tauri-apps/binary-releases/releases/download/apprun-old/AppRun-x86_64'

/**
 * cargo-packager 在 Linux 上的工具缓存里的 AppRun 路径（dirs::cache_dir()/.cargo-packager/AppImage）。
 * cargo-packager 下载时把 AppRun 设为 0764 并原样复制进 AppImage，只有属主能执行：AppImage 目录测试以普通用户
 * 在 firejail 中启动，得到 “AppRun: Permission denied”。打包前放好 0755 的 AppRun，cargo-packager 见文件已存在便不再下载。
 */
export function appRunPath(env = process.env, home = homedir()) {
  const cache = env.XDG_CACHE_HOME?.startsWith('/') ? env.XDG_CACHE_HOME : resolve(home, '.cache')
  return resolve(cache, '.cargo-packager', 'AppImage', 'AppRun-x86_64')
}

async function prepareAppRun(path) {
  if (!existsSync(path)) {
    const response = await fetch(APPRUN_URL)
    if (!response.ok) throw new Error(`下载 AppRun 失败：HTTP ${response.status}`)
    mkdirSync(dirname(path), { recursive: true })
    writeFileSync(path, Buffer.from(await response.arrayBuffer()))
  }
  chmodSync(path, 0o755)
  console.log(`AppRun（0755）：${path}`)
}

/** Linux deb 的运行依赖：录屏（PipeWire/GStreamer）、X11 截图与输入、密钥存储、GPUI 渲染（Vulkan、字体）、托盘（AppIndicator）。 */
const DEB_DEPENDS = [
  'libpipewire-0.3-0', 'libxcb1', 'libxcb-shm0', 'libxcb-xinput0', 'libxrandr2', 'libdbus-1-3',
  'libgstreamer1.0-0', 'libgstreamer-plugins-base1.0-0', 'gstreamer1.0-plugins-base', 'gstreamer1.0-plugins-good',
  'gstreamer1.0-pulseaudio', 'libxkbcommon0', 'libxkbcommon-x11-0', 'libfontconfig1', 'libvulkan1',
  'libayatana-appindicator3-1',
]

/**
 * AppImage 额外打包的库：托盘（tray-icon）运行时以 dlopen 加载 Ayatana AppIndicator，
 * linuxdeploy 只按链接关系收集依赖，看不到它；缺少时程序启动即退出。其余依赖由 linuxdeploy 自动收集。
 */
const APPIMAGE_LIBS = ['libayatana-appindicator3.so.1', 'libayatana-indicator3.so.7', 'libayatana-ido3-0.4.so.0',
  'libdbusmenu-glib.so.4', 'libdbusmenu-gtk3.so.4']

/** 命令行选项：`--debug` 用调试构建，`--standalone` 只生成独立 EXE、跳过 cargo-packager。 */
export function parseOptions(argv) {
  return { debug: argv.includes('--debug'), standalone: argv.includes('--standalone') }
}

/** Windows 独立 EXE 的文件名，与发布附件清单（release-assets.mjs）一致。 */
export function standaloneName(version) {
  return `snap-desk-v${version}-windows-x64-standalone.exe`
}

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
  if (platform === 'linux') {
    config.deb = { depends: DEB_DEPENDS }
    config.appimage = { libs: APPIMAGE_LIBS }
  }
  return config
}

/**
 * 读取 64 位 PE 文件导入表里的 DLL 名（转为小写）。只看普通导入表：Rust 不会延迟加载 C 运行库。
 * 偏移按 PE32+ 布局：COFF 头紧跟 “PE\0\0” 签名，可选头的数据目录从第 112 字节开始，导入表是第 2 项。
 */
export function importedLibraries(image) {
  const pe = image.readUInt32LE(0x3c)
  if (image.toString('latin1', pe, pe + 4) !== 'PE\0\0') throw new Error('不是 PE 文件')
  const optional = pe + 24
  if (image.readUInt16LE(optional) !== 0x20b) throw new Error('只支持 64 位 PE 文件')
  const sections = optional + image.readUInt16LE(pe + 20)
  const sectionCount = image.readUInt16LE(pe + 6)
  // 导入表里记录的是内存地址（RVA），按节表换算成文件偏移。
  const fileOffset = rva => {
    for (let index = 0; index < sectionCount; index++) {
      const section = sections + index * 40
      const address = image.readUInt32LE(section + 12)
      const size = Math.max(image.readUInt32LE(section + 8), image.readUInt32LE(section + 16))
      if (rva >= address && rva < address + size) return rva - address + image.readUInt32LE(section + 20)
    }
    throw new Error(`导入表地址 0x${rva.toString(16)} 不在任何节内`)
  }
  const importTable = image.readUInt32LE(optional + 120)
  if (!importTable) return []
  const names = []
  // 每个导入描述符 20 字节，第 12 字节是 DLL 名的地址；全零描述符表示结束。
  for (let entry = fileOffset(importTable); image.readUInt32LE(entry + 12); entry += 20) {
    const start = fileOffset(image.readUInt32LE(entry + 12))
    names.push(image.toString('latin1', start, image.indexOf(0, start)).toLowerCase())
  }
  return names
}

/** 需要另装 Visual C++ 运行库的导入；api-ms-win-crt-* 属于系统自带的 UCRT，不算在内。 */
export function vcRuntimeImports(libraries) {
  return libraries.filter(name => /^(vcruntime|msvcp)\d/.test(name))
}

function run(command, args, options) {
  const result = spawnSync(command, args, { stdio: 'inherit', windowsHide: true, ...options })
  if (result.error) throw result.error
  if (result.status !== 0) process.exit(result.status ?? 1)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL('../', import.meta.url))
  const target = resolve(root, 'target')
  const { debug, standalone: standaloneOnly } = parseOptions(process.argv.slice(2))
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
  if (process.platform === 'win32') {
    // 开发机通常装有 VC++ 运行库，缺少静态链接也能正常启动，只能在打包时检查导入表发现。
    const runtime = vcRuntimeImports(importedLibraries(readFileSync(built)))
    if (runtime.length) {
      throw new Error(`主程序依赖 ${runtime.join('、')}，未装 Visual C++ 运行库的电脑无法启动；检查 .cargo/config.toml 的 crt-static 设置，并确认没有设置 RUSTFLAGS`)
    }
  }
  copyFileSync(built, resolve(releaseDirectory, names.bundled))
  if (process.platform === 'win32') {
    const standalone = resolve(releaseDirectory, standaloneName(version))
    copyFileSync(built, standalone)
    console.log(`独立 EXE：${standalone}`)
  }
  if (standaloneOnly) {
    // 独立 EXE 就是发布版主程序本身，只有 Windows 作为单独附件发布；其他平台直接给出主程序位置。
    if (process.platform !== 'win32') console.log(`独立 EXE 只在 Windows 发布，当前平台的主程序：${built}`)
    process.exit(0)
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
  if (process.platform === 'linux') await prepareAppRun(appRunPath())
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
