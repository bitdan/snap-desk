import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createUpdateManifest, wixVersion } from './windows-update.mjs'

test('MSI 支持测试版本和正式版本，同时拒绝不可表示的版本', () => {
  assert.equal(wixVersion('0.2.0-beta.2'), '0.2.0.10002')
  assert.equal(wixVersion('0.2.0-rc.1'), '0.2.0.20001')
  assert.equal(wixVersion('0.2.0'), '0.2.0.65535')
  for (const version of ['0.2.0-beta.10000', '256.0.0', '0.0.65536', '0.2.0-preview', '0.2.0-beta.0']) {
    assert.throws(() => wixVersion(version))
  }
})

test('三种分发清单分别指向同一版本，缺包或签名版本不一致时停止发布', t => {
  const root = mkdtempSync(join(tmpdir(), 'snap-update-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const options = { version: '0.2.0-beta.2', repository: 'bitdan/snap-desk',
    nsis: join(root, 'Snap.Desk_0.2.0-beta.2_x64-setup.exe'), msi: join(root, 'Snap Desk.msi'),
    standalone: join(root, 'snap-desk-v0.2.0-beta.2-windows-x64-standalone.exe') }
  const signature = version => Buffer.from(`untrusted comment: test\nsig\ntrusted comment: timestamp:1\tfile:app\tversion:${version}\nsig`).toString('base64')
  for (const file of [options.nsis, options.msi, options.standalone]) {
    writeFileSync(file, 'fixture')
    writeFileSync(`${file}.sig`, signature(options.version))
  }
  const result = createUpdateManifest(options)
  assert.deepEqual(Object.keys(result.platforms), ['windows-x86_64-nsis', 'windows-x86_64-msi', 'windows-x86_64-standalone'])
  assert.match(result.platforms['windows-x86_64-msi'].url, /Snap%20Desk\.msi$/)
  assert.match(result.platforms['windows-x86_64-nsis'].url, /\/v0\.2\.0-beta\.2\/.*-setup\.exe$/)
  assert.throws(() => createUpdateManifest({ ...options, msi: undefined }))
  writeFileSync(`${options.msi}.sig`, signature('0.2.0-beta.20'))
  assert.throws(() => createUpdateManifest(options), /未绑定应用版本/)
  assert.throws(() => createUpdateManifest({ ...options, repository: '../other/repo' }))
})
