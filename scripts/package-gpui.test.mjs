import assert from 'node:assert/strict'
import { test } from 'node:test'
import { resolve } from 'node:path'
import { executableNames, packagerConfig } from './package-gpui.mjs'

const root = resolve('/repo')
const releaseDirectory = resolve(root, 'src-tauri/target/release')
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
  assert.equal(config('freebsd'), null)
})

test('macOS 无开发者证书时使用临时签名', () => {
  assert.equal(config('darwin').macos.signingIdentity, '-')
  assert.equal(config('darwin', { APPLE_SIGNING_IDENTITY: 'Developer ID Application: Example' }).macos.signingIdentity, 'Developer ID Application: Example')
})

test('构建产物复制为主程序名交给打包器', () => {
  assert.deepEqual(executableNames('win32'), { built: 'snap-desk-gpui.exe', bundled: 'snap-desk.exe' })
  assert.deepEqual(executableNames('linux'), { built: 'snap-desk-gpui', bundled: 'snap-desk' })
})
