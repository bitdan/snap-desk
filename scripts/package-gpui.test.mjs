import assert from 'node:assert/strict'
import { test } from 'node:test'
import { resolve } from 'node:path'
import { appRunPath, executableNames, packagerConfig, parseOptions, standaloneName } from './package-gpui.mjs'
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
