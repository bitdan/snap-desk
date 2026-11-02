// 构建前用小文件验证更新签名，避免等待完整 Rust 构建后才发现密钥或密码配置错误。
import { createHash, createPublicKey, verify } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { readVersion } from './sync-version.mjs'

const cli = createRequire(import.meta.url).resolve('@tauri-apps/cli/tauri.js')

/** 验证 Tauri 的 Minisign 预哈希签名及受保护的版本注释，不输出密钥内容。 */
export function verifyUpdaterSignature(payload, encodedSignature, encodedPublicKey, version) {
  const publicLines = Buffer.from(encodedPublicKey.trim(), 'base64').toString('utf8').trim().split(/\r?\n/)
  const key = Buffer.from(publicLines[1] ?? '', 'base64')
  const lines = Buffer.from(encodedSignature.trim(), 'base64').toString('utf8').trim().split(/\r?\n/)
  const signature = Buffer.from(lines[1] ?? '', 'base64')
  const globalSignature = Buffer.from(lines[3] ?? '', 'base64')
  if (key.length !== 42 || signature.length !== 74 || globalSignature.length !== 64
    || key.subarray(0, 2).toString() !== 'Ed' || signature.subarray(0, 2).toString() !== 'ED'
    || !signature.subarray(2, 10).equals(key.subarray(2, 10)) || !lines[2]?.startsWith('trusted comment: ')) {
    throw new Error('签名公钥不匹配或密钥格式无效，请核对 SNAP_DESK_UPDATER_PUBLIC_KEY。')
  }
  const trusted = lines[2].slice('trusted comment: '.length)
  if (!trusted.split('\t').includes(`version:${version}`)) throw new Error('更新签名未绑定当前应用版本。')
  const publicKey = createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), key.subarray(10)]), format: 'der', type: 'spki' })
  const rawSignature = signature.subarray(10)
  if (!verify(null, createHash('blake2b512').update(payload).digest(), publicKey, rawSignature)
    || !verify(null, Buffer.concat([rawSignature, Buffer.from(trusted)]), publicKey, globalSignature)) {
    throw new Error('更新签名验证失败，请核对发布公钥与私钥是否成对。')
  }
}

export function checkReleaseSigning(source = process.env, version = readVersion()) {
  const env = { ...source }
  const privateKey = env.TAURI_SIGNING_PRIVATE_KEY?.trim()
  const publicKey = env.SNAP_DESK_UPDATER_PUBLIC_KEY?.trim()
  if (!privateKey || !publicKey) throw new Error('请配置 TAURI_SIGNING_PRIVATE_KEY Secret 和 SNAP_DESK_UPDATER_PUBLIC_KEY Variable。')
  env.TAURI_SIGNING_PRIVATE_KEY = privateKey
  if (!env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD) delete env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD
  const directory = mkdtempSync(join(tmpdir(), 'snap-signing-check-'))
  try {
    const probe = join(directory, 'probe.txt')
    const payload = Buffer.from(`Snap Desk signing check ${version}`)
    writeFileSync(probe, payload)
    const result = spawnSync(process.execPath, [cli, 'signer', 'sign', '--app-version', version, probe], {
      env, input: '', encoding: 'utf8', windowsHide: true,
    })
    // 不转发签名器输出，避免 CI 日志出现敏感配置；提示直接指向需要核对的字段。
    if (result.error || result.status !== 0) throw new Error('更新签名预检失败：私钥内容或密码不匹配。无密码密钥请删除 TAURI_SIGNING_PRIVATE_KEY_PASSWORD Secret，并重新复制 snap-desk.key 的完整内容。')
    verifyUpdaterSignature(payload, readFileSync(`${probe}.sig`, 'utf8'), publicKey, version)
  } finally { rmSync(directory, { recursive: true, force: true }) }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { checkReleaseSigning(); console.log('更新签名预检通过：密钥可解密，公私钥匹配。') }
  catch (error) { console.error(error.message); process.exitCode = 1 }
}
