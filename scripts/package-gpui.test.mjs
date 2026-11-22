import assert from 'node:assert/strict'
import { test } from 'node:test'
import { executableNames, gpuiBundleConfig } from './package-gpui.mjs'

test('GPUI 打包沿用发布清单的目标，去掉独立 EXE 标记处理并跳过 WebView2', () => {
  const windows = gpuiBundleConfig('win32', {})
  assert.deepEqual(windows.bundle.targets, ['nsis'])
  assert.equal(windows.build, undefined)
  assert.deepEqual(windows.bundle.windows, { webviewInstallMode: { type: 'skip' } })
  assert.equal(windows.bundle.createUpdaterArtifacts, false)
  assert.equal(gpuiBundleConfig('win32', { TAURI_SIGNING_PRIVATE_KEY: 'k' }).bundle.createUpdaterArtifacts, true)
  const mac = gpuiBundleConfig('darwin', {})
  assert.deepEqual(mac.bundle.targets, ['dmg', 'app'])
  assert.equal(mac.bundle.windows, undefined)
  assert.deepEqual(gpuiBundleConfig('linux', {}).bundle.targets, ['appimage', 'deb'])
  assert.equal(gpuiBundleConfig('freebsd', {}), null)
})

test('主程序以 Tauri 包的二进制名交给打包器', () => {
  assert.deepEqual(executableNames('win32'), { built: 'snap-desk-gpui.exe', bundled: 'snap-desk.exe' })
  assert.deepEqual(executableNames('linux'), { built: 'snap-desk-gpui', bundled: 'snap-desk' })
})
