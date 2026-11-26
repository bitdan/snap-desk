import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { downloadManifest, writeDistribution } from './distribution.mjs'

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
})

test('正式版带有完整的 SHA-256 才能生成下载清单', () => {
  assert.throws(() => downloadManifest('3.12.2-beta.1', hashes))
  assert.throws(() => downloadManifest(version, { ...hashes, dmg: '' }))
})

test('终端安装 TSV 使用与 JSON 一致的附件地址和校验值', t => {
  const directory = mkdtempSync(join(tmpdir(), 'snap-desk-download-test-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  writeDistribution(version, hashes, directory)
  const manifest = JSON.parse(readFileSync(join(directory, 'downloads.json'), 'utf8'))
  const rows = readFileSync(join(directory, 'downloads.tsv'), 'utf8').trimEnd().split('\n')
  assert.equal(rows.length, 5)
  for (const row of rows) {
    const [key, url, sha256] = row.split('\t')
    assert.deepEqual({ url, sha256 }, manifest.assets[key])
  }
})
