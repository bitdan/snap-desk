import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { resolve } from 'node:path'
import { appRunPath, executableNames, importedLibraries, packagerConfig, parseOptions, standaloneName, vcRuntimeImports } from './package-gpui.mjs'
import { RELEASE_ASSETS } from './release-assets.mjs'

const root = resolve('/repo')
const releaseDirectory = resolve(root, 'target/release')
const config = (platform, env = {}) => packagerConfig({ platform, root, releaseDirectory, version: '1.2.3-beta.1', env })

test('按发布清单选择 cargo-packager 格式，主程序与输出目录固定', () => {
  const windows = config('win32')
  assert.deepEqual(windows.formats, ['nsis'])
  assert.equal(windows.identifier, 'io.snapdesk.app')
  assert.equal(windows.version, '1.2.3-beta.1')
  assert.deepEqual(windows.binaries, [{ path: 'snap-desk.exe', main: true }])
  assert.equal(windows.binariesDir, releaseDirectory)
  assert.equal(windows.outDir, resolve(releaseDirectory, 'packages'))
  assert.deepEqual(windows.nsis, { installMode: 'currentUser' })
  assert.equal(windows.macos, undefined)
  assert.deepEqual(config('darwin').formats, ['dmg', 'app'])
  assert.deepEqual(config('linux').formats, ['appimage', 'deb'])
  assert.ok(config('linux').deb.depends.includes('libvulkan1'))
  // 托盘以 dlopen 加载 AppIndicator：deb 声明依赖，AppImage 显式打包。
  assert.ok(config('linux').deb.depends.includes('libayatana-appindicator3-1'))
  assert.ok(config('linux').appimage.libs.includes('libayatana-appindicator3.so.1'))
  assert.equal(config('win32').appimage, undefined)
  assert.equal(config('freebsd'), null)
})

test('macOS 无开发者证书时使用临时签名', () => {
  assert.equal(config('darwin').macos.signingIdentity, '-')
  assert.equal(config('darwin', { APPLE_SIGNING_IDENTITY: 'Developer ID Application: Example' }).macos.signingIdentity, 'Developer ID Application: Example')
  assert.equal(config('darwin').macos.entitlements, resolve(root, 'resources/Entitlements.plist'))
  assert.equal(config('darwin').macos.infoPlistPath, resolve(root, 'resources/Info.plist'))
})

test('图标与随附许可证取自 resources/', () => {
  const windows = config('win32')
  assert.ok(windows.icons.every(icon => icon.startsWith(resolve(root, 'resources/icons'))))
  assert.ok(windows.resources.some(item => item.src === resolve(root, 'resources/licenses/LXGW-OFL.txt')))
})

test('构建产物复制为主程序名交给打包器', () => {
  assert.deepEqual(executableNames('win32'), { built: 'snap-desk-gpui.exe', bundled: 'snap-desk.exe' })
  assert.deepEqual(executableNames('linux'), { built: 'snap-desk-gpui', bundled: 'snap-desk' })
})

test('AppRun 放在 cargo-packager 的 Linux 工具缓存里', () => {
  assert.equal(appRunPath({}, '/home/runner'), resolve('/home/runner/.cache/.cargo-packager/AppImage/AppRun-x86_64'))
  assert.equal(appRunPath({ XDG_CACHE_HOME: '/var/cache/ci' }, '/home/runner'), resolve('/var/cache/ci/.cargo-packager/AppImage/AppRun-x86_64'))
  // 与 dirs::cache_dir 一致：相对路径的 XDG_CACHE_HOME 无效，回退到 ~/.cache。
  assert.equal(appRunPath({ XDG_CACHE_HOME: 'cache' }, '/home/runner'), resolve('/home/runner/.cache/.cargo-packager/AppImage/AppRun-x86_64'))
})

test('--standalone 只生成独立 EXE，文件名与发布附件清单一致', () => {
  assert.deepEqual(parseOptions([]), { debug: false, standalone: false })
  assert.deepEqual(parseOptions(['--standalone']), { debug: false, standalone: true })
  assert.deepEqual(parseOptions(['--debug', '--standalone']), { debug: true, standalone: true })
  const asset = RELEASE_ASSETS.find(item => item.target === 'windows-x86_64-standalone')
  assert.equal(standaloneName('1.2.3-beta.1'), asset.match('1.2.3-beta.1'))
})

/** 只含一个节和导入表的最小 64 位 PE：节从文件偏移 0x200 映射到内存地址 0x1000，导入描述符在节开头。 */
function executable(libraries) {
  const image = Buffer.alloc(0x400)
  image.write('MZ', 0, 'latin1')
  image.writeUInt32LE(0x40, 0x3c)
  image.write('PE\0\0', 0x40, 'latin1')
  image.writeUInt16LE(1, 0x46)
  image.writeUInt16LE(240, 0x54)
  image.writeUInt16LE(0x20b, 0x58)
  image.writeUInt32LE(0x1000, 0x58 + 120)
  const section = 0x58 + 240
  image.writeUInt32LE(0x200, section + 8)
  image.writeUInt32LE(0x1000, section + 12)
  image.writeUInt32LE(0x200, section + 16)
  image.writeUInt32LE(0x200, section + 20)
  let name = 0x200 + (libraries.length + 1) * 20
  libraries.forEach((library, index) => {
    image.writeUInt32LE(name - 0x200 + 0x1000, 0x200 + index * 20 + 12)
    name += image.write(`${library}\0`, name, 'latin1')
  })
  return image
}

test('打包前从导入表找出需要另装的 VC++ 运行库', () => {
  const libraries = importedLibraries(executable(['KERNEL32.dll', 'VCRUNTIME140.dll', 'MSVCP140_1.dll', 'api-ms-win-crt-heap-l1-1-0.dll']))
  assert.deepEqual(libraries, ['kernel32.dll', 'vcruntime140.dll', 'msvcp140_1.dll', 'api-ms-win-crt-heap-l1-1-0.dll'])
  assert.deepEqual(vcRuntimeImports(libraries), ['vcruntime140.dll', 'msvcp140_1.dll'])
  assert.deepEqual(vcRuntimeImports(importedLibraries(executable(['KERNEL32.dll', 'd3d11.dll']))), [])
  assert.throws(() => importedLibraries(Buffer.alloc(0x100)), /PE/)
})

test('导入表解析适用于真实的 Windows 程序', { skip: process.platform !== 'win32' }, () => {
  assert.ok(importedLibraries(readFileSync(process.execPath)).includes('kernel32.dll'))
})
