import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { PACKAGES_DIRECTORY, RELEASE_ASSETS, collectReleaseAssets, packageFormats } from './release-assets.mjs'

test('附件后缀可唯一识别，更新目标互不重复', () => {
  for (const asset of RELEASE_ASSETS) {
    const others = RELEASE_ASSETS.filter(other => other !== asset)
    assert.ok(others.every(other => !asset.suffix.endsWith(other.suffix)), asset.suffix)
    // 统一文件名必须以登记的后缀结尾，否则发布脚本和更新清单找不到它。
    if (asset.name) assert.ok(asset.name('1.2.3').endsWith(asset.suffix), asset.suffix)
  }
  const targets = RELEASE_ASSETS.filter(asset => asset.target).map(asset => asset.target)
  assert.equal(new Set(targets).size, targets.length)
  // 与 apps/desktop-gpui/src/updater.rs 的平台和分发形式一一对应。
  assert.deepEqual(targets, ['windows-x86_64-nsis', 'windows-x86_64-standalone', 'darwin-aarch64-app', 'linux-x86_64-appimage'])
})

test('按平台列出 cargo-packager 格式', () => {
  assert.deepEqual(packageFormats('win32'), ['nsis'])
  assert.deepEqual(packageFormats('darwin'), ['dmg', 'app'])
  assert.deepEqual(packageFormats('linux'), ['appimage', 'deb'])
  assert.deepEqual(packageFormats('freebsd'), [])
})

test('收集产物时统一命名，缺包、残留旧包或架构不符时停止', t => {
  const root = mkdtempSync(join(tmpdir(), 'snap-assets-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const release = join(root, 'release'), output = join(root, 'out')
  const packages = join(release, PACKAGES_DIRECTORY)
  mkdirSync(packages, { recursive: true })
  writeFileSync(join(packages, 'snap-desk_1.2.3_aarch64.dmg'), 'dmg')
  writeFileSync(join(packages, 'snap-desk_1.2.3_aarch64.app.tar.gz'), 'app')
  const mac = { platform: 'darwin', arch: 'arm64', version: '1.2.3', releaseDirectory: release, output }
  collectReleaseAssets(mac)
  assert.deepEqual(readdirSync(output).sort(), ['snap-desk-v1.2.3-macos-arm64.app.tar.gz', 'snap-desk-v1.2.3-macos-arm64.dmg'])
  assert.throws(() => collectReleaseAssets({ ...mac, arch: 'x64' }), /架构/)
  assert.throws(() => collectReleaseAssets({ ...mac, platform: 'freebsd' }), /不在发布范围/)
  writeFileSync(join(packages, 'snap-desk_1.2.2_aarch64.dmg'), 'stale')
  assert.throws(() => collectReleaseAssets(mac), /必须恰好有一个/)

  // 独立 EXE 只认当前版本，旧版本残留不参与。
  const windows = { platform: 'win32', arch: 'x64', version: '1.2.3', releaseDirectory: release, output: join(root, 'windows') }
  writeFileSync(join(packages, 'snap-desk_1.2.3_x64-setup.exe'), 'nsis')
  writeFileSync(join(release, 'snap-desk-v1.2.2-windows-x64-standalone.exe'), 'stale')
  assert.throws(() => collectReleaseAssets(windows), /独立 EXE/)
  writeFileSync(join(release, 'snap-desk-v1.2.3-windows-x64-standalone.exe'), 'exe')
  collectReleaseAssets(windows)
  assert.deepEqual(readdirSync(windows.output).sort(), ['snap-desk-v1.2.3-windows-x64-setup.exe', 'snap-desk-v1.2.3-windows-x64-standalone.exe'])
})
