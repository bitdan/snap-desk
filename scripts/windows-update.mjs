// NSIS 与独立 EXE 共用应用更新清单，安装目标彼此独立。
import { readFileSync, statSync, writeFileSync } from 'node:fs'
import { basename, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { readVersion } from './sync-version.mjs'

export function createUpdateManifest({ version, repository, nsis, standalone, notes = '', date = new Date().toISOString() }) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) throw new Error('GitHub 仓库标识无效。')
  const platforms = {}
  for (const [kind, file, suffix] of [['nsis', nsis, '-setup.exe'], ['standalone', standalone, '-standalone.exe']]) {
    if (!file || !basename(file).endsWith(suffix) || !statSync(file).isFile()) throw new Error(`缺少 ${kind} 更新产物。`)
    const signature = readFileSync(`${file}.sig`, 'utf8').trim()
    const decoded = Buffer.from(signature, 'base64').toString('utf8')
    // 必须使用 --app-version 签名，客户端拒绝把旧包伪装成新版本。
    const trusted = decoded.split(/\r?\n/).find(line => line.startsWith('trusted comment: ')) ?? ''
    if (!trusted.slice('trusted comment: '.length).split('\t').includes(`version:${version}`)) throw new Error(`${kind} 签名未绑定应用版本 ${version}。`)
    platforms[`windows-x86_64-${kind}`] = {
      url: `https://github.com/${repository}/releases/download/v${version}/${encodeURIComponent(basename(file))}`,
      signature,
    }
  }
  return { version, notes, pub_date: date, platforms }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [nsis, standalone, notesFile, output] = process.argv.slice(2)
  if (!output) throw new Error('用法：node scripts/windows-update.mjs <nsis> <standalone> <notes> <output>')
  const manifest = createUpdateManifest({ version: readVersion(), repository: process.env.GITHUB_REPOSITORY,
    nsis, standalone, notes: readFileSync(notesFile, 'utf8') })
  writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`)
}
