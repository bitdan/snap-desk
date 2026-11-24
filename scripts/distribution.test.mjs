import assert from 'node:assert/strict'
import { test } from 'node:test'
import { aurPackage, assetUrl, homebrewCask, isStable, scoopManifest, wingetManifests, WINGET_ID } from './distribution.mjs'

const hashes = { standalone: 'a'.repeat(64), setup: 'b'.repeat(64), dmg: 'c'.repeat(64), deb: 'd'.repeat(64) }

test('只有正式版进入包管理器', () => {
  assert.equal(isStable('1.2.3'), true)
  assert.equal(isStable('1.2.3-beta.1'), false)
})

test('附件地址与发布清单的文件名一致', () => {
  assert.equal(assetUrl('setup', '1.2.3'), 'https://github.com/bitdan/snap-desk/releases/download/v1.2.3/snap-desk-v1.2.3-windows-x64-setup.exe')
  assert.equal(assetUrl('standalone', '1.2.3'), 'https://github.com/bitdan/snap-desk/releases/download/v1.2.3/snap-desk-v1.2.3-windows-x64-standalone.exe')
  assert.equal(assetUrl('deb', '1.2.3'), 'https://github.com/bitdan/snap-desk/releases/download/v1.2.3/snap-desk-v1.2.3-linux-x64.deb')
})

test('Scoop 用独立 EXE 并支持 autoupdate', () => {
  const manifest = JSON.parse(scoopManifest('1.2.3', hashes))
  assert.equal(manifest.version, '1.2.3')
  assert.equal(manifest.architecture['64bit'].hash, hashes.standalone)
  assert.match(manifest.architecture['64bit'].url, /v1\.2\.3-windows-x64-standalone\.exe#\/snap-desk\.exe$/)
  assert.match(manifest.autoupdate.architecture['64bit'].url, /\$version/)
})

test('Homebrew Cask 指向 DMG 并带哈希', () => {
  const cask = homebrewCask('1.2.3', hashes)
  assert.match(cask, /version "1\.2\.3"/)
  assert.match(cask, new RegExp(`sha256 "${hashes.dmg}"`))
  assert.match(cask, /snap-desk-v#\{version\}-macos-arm64\.dmg/)
})

test('WinGet 清单包含三个文件且哈希为大写', () => {
  const files = wingetManifests('1.2.3', hashes)
  assert.deepEqual(Object.keys(files).sort(), [`${WINGET_ID}.installer.yaml`, `${WINGET_ID}.locale.en-US.yaml`, `${WINGET_ID}.yaml`].sort())
  assert.match(files[`${WINGET_ID}.installer.yaml`], new RegExp(`InstallerSha256: ${hashes.setup.toUpperCase()}`))
})

test('WinGet 清单以对应类型的 schema 注释开头', () => {
  const files = wingetManifests('1.2.3', hashes)
  const header = (type) => new RegExp(`^# yaml-language-server: \\$schema=https://aka\\.ms/winget-manifest\\.${type}\\.1\\.9\\.0\\.schema\\.json\\n`)
  assert.match(files[`${WINGET_ID}.yaml`], header('version'))
  assert.match(files[`${WINGET_ID}.installer.yaml`], header('installer'))
  assert.match(files[`${WINGET_ID}.locale.en-US.yaml`], header('defaultLocale'))
})

test('AUR PKGBUILD 与 .SRCINFO 版本和哈希一致', () => {
  const pkg = aurPackage('1.2.3', hashes)
  assert.match(pkg.PKGBUILD, /pkgver=1\.2\.3/)
  assert.match(pkg.PKGBUILD, new RegExp(`sha256sums=\\('${hashes.deb}'\\)`))
  assert.match(pkg['.SRCINFO'], /pkgver = 1\.2\.3/)
  assert.match(pkg['.SRCINFO'], new RegExp(`sha256sums = ${hashes.deb}`))
})
