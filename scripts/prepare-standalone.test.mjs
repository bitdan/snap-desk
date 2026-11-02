import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { preserveStandalone } from './prepare-standalone.mjs'

test('保留打包前的未知类型标记，重试时只复用内容完全相同的独立副本', t => {
  const directory = mkdtempSync(join(tmpdir(), 'snap-standalone-test-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const source = join(directory, 'snap-desk.exe'), target = join(directory, 'standalone.exe')
  const original = Buffer.from('MZ-original-__TAURI_BUNDLE_TYPE_VAR_UNK-__TAURI_BUNDLE_TYPE_VAR_NSS-end')
  writeFileSync(source, original)
  preserveStandalone(source, target)
  assert.deepEqual(readFileSync(target), original)
  const packaged = Buffer.from(original.toString().replace('__TAURI_BUNDLE_TYPE_VAR_UNK', '__TAURI_BUNDLE_TYPE_VAR_NSS'))
  writeFileSync(source, packaged)
  assert.doesNotThrow(() => preserveStandalone(source, target))
  assert.deepEqual(readFileSync(target), original)
  writeFileSync(source, Buffer.concat([packaged, Buffer.from('new-build')]))
  assert.throws(() => preserveStandalone(source, target), /无法确认独立 EXE/)
  assert.deepEqual(readFileSync(target), original)
})
