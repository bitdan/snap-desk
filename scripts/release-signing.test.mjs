import assert from 'node:assert/strict'
import { test } from 'node:test'
import { spawnSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { checkReleaseSigning, verifyUpdaterSignature } from './release-signing.mjs'

test('无密码密钥可签名并验签，错误密码、公钥、版本和篡改内容均被拒绝', t => {
  const directory = mkdtempSync(join(tmpdir(), 'snap-signing-test-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const cli = createRequire(import.meta.url).resolve('@tauri-apps/cli/tauri.js')
  const path = join(directory, 'test.key')
  const generation = spawnSync(process.execPath, [cli, 'signer', 'generate', '--ci', '-w', path], { encoding: 'utf8', windowsHide: true })
  assert.equal(generation.status, 0, '临时测试密钥生成失败')
  // 测试密钥只保存在临时目录，不输出到测试日志或仓库。
  const env = { ...process.env, TAURI_SIGNING_PRIVATE_KEY: readFileSync(path, 'utf8'),
    SNAP_DESK_UPDATER_PUBLIC_KEY: readFileSync(`${path}.pub`, 'utf8') }
  delete env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD
  assert.doesNotThrow(() => checkReleaseSigning(env, '0.2.0-beta.3'))
  assert.doesNotThrow(() => checkReleaseSigning({ ...env, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: '' }, '0.2.0-beta.3'))
  assert.throws(() => checkReleaseSigning({ ...env, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'wrong-password' }), /预检失败/)
  assert.throws(() => checkReleaseSigning({ ...env, SNAP_DESK_UPDATER_PUBLIC_KEY: 'invalid' }), /公钥不匹配/)
  const probe = join(directory, 'payload.txt')
  writeFileSync(probe, 'payload')
  const signing = spawnSync(process.execPath, [cli, 'signer', 'sign', '--app-version', '0.2.0-beta.3', probe], { env, input: '', encoding: 'utf8', windowsHide: true })
  assert.equal(signing.status, 0, '临时测试签名失败')
  const signature = readFileSync(`${probe}.sig`, 'utf8')
  assert.throws(() => verifyUpdaterSignature(Buffer.from('modified'), signature, env.SNAP_DESK_UPDATER_PUBLIC_KEY, '0.2.0-beta.3'), /签名验证失败/)
  assert.throws(() => verifyUpdaterSignature(Buffer.from('payload'), signature, env.SNAP_DESK_UPDATER_PUBLIC_KEY, '0.2.0-beta.4'), /未绑定当前应用版本/)
})
