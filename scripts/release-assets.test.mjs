import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { RELEASE_ASSETS, bundleConfig, collectReleaseAssets } from './release-assets.mjs'

test('附件后缀可唯一识别，更新目标互不重复', () => {
  for (const asset of RELEASE_ASSETS) {
    const others = RELEASE_ASSETS.filter(other => other !== asset)
    assert.ok(others.every(other => !asset.suffix.endsWith(other.suffix)), asset.suffix)
    // 统一文件名必须以登记的后缀结尾，否则发布脚本和更新清单找不到它。
    if (asset.name) assert.ok(asset.name('1.2.3').endsWith(asset.suffix), asset.suffix)
  }
  const targets = RELEASE_ASSETS.filter(asset => asset.target).map(asset => asset.target)
  assert.equal(new Set(targets).size, targets.length)
  // 与 src-tauri/src/app/updater.rs 的平台和分发形式一一对应。
  assert.deepEqual(targets, ['windows-x86_64-nsis', 'windows-x86_64-standalone', 'darwin-aarch64-app', 'linux-x86_64-appimage'])
})

test('按平台选择打包目标、独立 EXE 钩子和临时签名', () => {
  const windows = bundleConfig('win32', { TAURI_SIGNING_PRIVATE_KEY: 'key', SNAP_DESK_UPDATER_PUBLIC_KEY: ' public \n' })
  assert.deepEqual(windows.bundle, { targets: ['nsis'], createUpdaterArtifacts: true })
  assert.equal(windows.plugins.updater.pubkey, 'public')
  assert.match(windows.build.beforeBundleCommand, /prepare-standalone/)
  const mac = bundleConfig('darwin', {})
  assert.deepEqual(mac.bundle, { targets: ['dmg', 'app'], createUpdaterArtifacts: false, macOS: { signingIdentity: '-' } })
  assert.equal(mac.build, undefined)
  assert.equal(bundleConfig('darwin', { APPLE_SIGNING_IDENTITY: 'Developer ID Application: Example' }).bundle.macOS, undefined)
  assert.deepEqual(bundleConfig('linux', {}).bundle.targets, ['appimage', 'deb'])
  assert.equal(bundleConfig('freebsd', {}), null)
})

test('收集产物时统一命名，缺包、残留旧包或架构不符时停止', t => {
  const root = mkdtempSync(join(tmpdir(), 'snap-assets-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const release = join(root, 'release'), output = join(root, 'out')
  for (const directory of ['bundle/dmg', 'bundle/macos', 'bundle/nsis']) mkdirSync(join(release, directory), { recursive: true })
  writeFileSync(join(release, 'bundle/dmg/Snap Desk_1.2.3_aarch64.dmg'), 'dmg')
  writeFileSync(join(release, 'bundle/macos/Snap Desk.app.tar.gz'), 'app')
  const mac = { platform: 'darwin', arch: 'arm64', version: '1.2.3', releaseDirectory: release, output }
  collectReleaseAssets(mac)
  assert.deepEqual(readdirSync(output).sort(), ['snap-desk-v1.2.3-macos-arm64.app.tar.gz', 'snap-desk-v1.2.3-macos-arm64.dmg'])
  assert.throws(() => collectReleaseAssets({ ...mac, arch: 'x64' }), /架构/)
  assert.throws(() => collectReleaseAssets({ ...mac, platform: 'freebsd' }), /不在发布范围/)
  writeFileSync(join(release, 'bundle/dmg/Snap Desk_1.2.2_aarch64.dmg'), 'stale')
  assert.throws(() => collectReleaseAssets(mac), /必须恰好有一个/)

  // NSIS 沿用打包器文件名；独立 EXE 只认当前版本，旧版本残留不参与。
  const windows = { platform: 'win32', arch: 'x64', version: '1.2.3', releaseDirectory: release, output: join(root, 'windows') }
  writeFileSync(join(release, 'bundle/nsis/Snap Desk_1.2.3_x64-setup.exe'), 'nsis')
  writeFileSync(join(release, 'snap-desk-v1.2.2-windows-x64-standalone.exe'), 'stale')
  assert.throws(() => collectReleaseAssets(windows), /独立 EXE/)
  writeFileSync(join(release, 'snap-desk-v1.2.3-windows-x64-standalone.exe'), 'exe')
  collectReleaseAssets(windows)
  assert.deepEqual(readdirSync(windows.output).sort(), ['Snap Desk_1.2.3_x64-setup.exe', 'snap-desk-v1.2.3-windows-x64-standalone.exe'])
})
