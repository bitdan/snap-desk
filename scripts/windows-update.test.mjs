import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createUpdateManifest } from './windows-update.mjs'

test('双分发清单分别指向同一版本，缺包或签名版本不一致时停止发布', t => {
  const root = mkdtempSync(join(tmpdir(), 'snap-update-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const options = { version: '0.2.0-beta.2', repository: 'bitdan/snap-desk',
    nsis: join(root, 'Snap.Desk_0.2.0-beta.2_x64-setup.exe'),
    standalone: join(root, 'snap-desk-v0.2.0-beta.2-windows-x64-standalone.exe') }
  const signature = version => Buffer.from(`untrusted comment: test\nsig\ntrusted comment: timestamp:1\tfile:app\tversion:${version}\nsig`).toString('base64')
  for (const file of [options.nsis, options.standalone]) {
    writeFileSync(file, 'fixture')
    writeFileSync(`${file}.sig`, signature(options.version))
  }
  const result = createUpdateManifest(options)
  assert.deepEqual(Object.keys(result.platforms), ['windows-x86_64-nsis', 'windows-x86_64-standalone'])
  assert.match(result.platforms['windows-x86_64-standalone'].url, /\/v0\.2\.0-beta\.2\/.*-standalone\.exe$/)
  assert.match(result.platforms['windows-x86_64-nsis'].url, /\/v0\.2\.0-beta\.2\/.*-setup\.exe$/)
  assert.throws(() => createUpdateManifest({ ...options, nsis: undefined }))
  assert.throws(() => createUpdateManifest({ ...options, standalone: undefined }))
  writeFileSync(`${options.standalone}.sig`, signature('0.2.0-beta.20'))
  assert.throws(() => createUpdateManifest(options), /未绑定应用版本/)
  assert.throws(() => createUpdateManifest({ ...options, repository: '../other/repo' }))
})
