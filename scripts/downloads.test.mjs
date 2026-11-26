import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { test } from 'node:test'
import { downloadManifest } from './distribution.mjs'
import { downloadCommand, parseDownloads } from '../docs/assets/downloads.js'

const version = '3.12.2'
const hashes = Object.fromEntries(['setup', 'standalone', 'dmg', 'appimage', 'deb']
  .map((key, index) => [key, String(index + 1).repeat(64)]))

test('下载清单只包含五种公开安装产物，每种都有版本化地址和 SHA-256', () => {
  const manifest = downloadManifest(version, hashes)
  assert.equal(manifest.version, version)
  assert.deepEqual(Object.keys(manifest.assets).sort(), ['appimage', 'deb', 'dmg', 'setup', 'standalone'])
  assert.deepEqual(manifest.assets.appimage, {
    url: 'https://github.com/bitdan/snap-desk/releases/download/v3.12.2/snap-desk-v3.12.2-linux-x64.AppImage',
    sha256: hashes.appimage,
  })
  assert.deepEqual(parseDownloads(manifest), manifest)
})

test('官网拒绝测试版、缺少校验值或被替换为另一主机及版本的下载地址', () => {
  const manifest = downloadManifest(version, hashes)
  assert.throws(() => parseDownloads({ ...manifest, version: '3.12.2-beta.1' }))
  for (const url of [
    manifest.assets.setup.url.replace('github.com', 'example.org'),
    manifest.assets.setup.url.replaceAll('3.12.2', '3.12.1'),
    manifest.assets.setup.url + "?x=';whoami",
  ]) {
    assert.throws(() => parseDownloads({
      ...manifest, assets: { ...manifest.assets, setup: { ...manifest.assets.setup, url } },
    }))
  }
  assert.throws(() => downloadManifest(version, { ...hashes, dmg: '' }))
})

test('官网自带的清单可以生成对应平台的校验命令', () => {
  const fixture = JSON.parse(readFileSync(new URL('../docs/_data/downloads.json', import.meta.url)))
  const { assets } = parseDownloads(fixture)
  assert.match(downloadCommand('setup', assets.setup), /Get-FileHash.*\.exe/)
  assert.match(downloadCommand('dmg', assets.dmg), /shasum -a 256 -c/)
  assert.match(downloadCommand('appimage', assets.appimage), /sha256sum -c.*chmod \+x/)
  assert.match(downloadCommand('deb', assets.deb), /sha256sum -c/)
})

test('Unix 下载失败时，即使命令用于条件判断也会停止，且不会留下文件', t => {
  let bash = '/bin/bash'
  if (process.platform === 'win32') {
    const git = spawnSync('git', ['--exec-path'], { encoding: 'utf8' })
    if (git.status !== 0) return t.skip('Git Bash is unavailable')
    bash = resolve(git.stdout.trim(), '../../../bin/bash.exe')
  }
  if (!existsSync(bash)) return t.skip('Bash is unavailable')
  const directory = mkdtempSync(join(tmpdir(), 'snap-desk-download-test-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const { dmg } = downloadManifest(version, hashes).assets
  const command = downloadCommand('dmg', dmg)
  const script = 'curl() { return 22; }; shasum() { cat >/dev/null; }; if eval "$1"; then exit 0; else exit "$?"; fi'
  const result = spawnSync(bash, ['-c', script, 'bash', command], { cwd: directory, encoding: 'utf8' })
  assert.ifError(result.error)
  assert.equal(result.status, 22, result.stdout + result.stderr)
  assert.deepEqual(readdirSync(directory), [])
})
