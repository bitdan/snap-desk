// NSIS 与独立 EXE 共用应用更新清单，安装目标彼此独立。
import { readFileSync, statSync, writeFileSync } from 'node:fs'
import { basename, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { readVersion } from './sync-version.mjs'

export function createUpdateManifest({ version, repository, nsis, standalone, notes = '', date = new Date().toISOString(), assets }) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) throw new Error('GitHub 仓库标识无效。')
  if (assets !== undefined && !Array.isArray(assets)) throw new Error('发布附件列表无效。')
  const platforms = {}
  for (const [kind, file, suffix] of [['nsis', nsis, '-setup.exe'], ['standalone', standalone, '-standalone.exe']]) {
    if (!file || !basename(file).endsWith(suffix) || !statSync(file).isFile()) throw new Error(`缺少 ${kind} 更新产物。`)
    const signature = readFileSync(`${file}.sig`, 'utf8').trim()
    const decoded = Buffer.from(signature, 'base64').toString('utf8')
    // 必须使用 --app-version 签名，客户端拒绝把旧包伪装成新版本。
    const trusted = decoded.split(/\r?\n/).find(line => line.startsWith('trusted comment: ')) ?? ''
    if (!trusted.slice('trusted comment: '.length).split('\t').includes(`version:${version}`)) throw new Error(`${kind} 签名未绑定应用版本 ${version}。`)
    // GitHub 上传时会重命名含空格的文件；以已上传附件为准，不能从本地文件名猜下载地址。
    const uploaded = assets?.filter(asset => typeof asset.name === 'string' && asset.name.endsWith(suffix))
    if (uploaded && uploaded.length !== 1) throw new Error(`发布附件必须包含一个 ${kind} 产物。`)
    const prefix = `https://github.com/${repository}/releases/download/v${version}/`
    const url = uploaded ? uploaded[0].browser_download_url : `${prefix}${encodeURIComponent(basename(file))}`
    if (typeof url !== 'string' || !url.startsWith(prefix)
      || (uploaded && decodeURIComponent(url.slice(prefix.length)) !== uploaded[0].name)) {
      throw new Error(`${kind} 发布附件下载地址无效。`)
    }
    platforms[`windows-x86_64-${kind}`] = {
      url,
      signature,
    }
  }
  return { version, notes, pub_date: date, platforms }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [nsis, standalone, notesFile, output, assetsFile] = process.argv.slice(2)
  if (!output) throw new Error('用法：node scripts/windows-update.mjs <nsis> <standalone> <notes> <output> [assets-json]')
  const manifest = createUpdateManifest({ version: readVersion(), repository: process.env.GITHUB_REPOSITORY,
    nsis, standalone, notes: readFileSync(notesFile, 'utf8'),
    assets: assetsFile ? JSON.parse(readFileSync(assetsFile, 'utf8').replace(/^\uFEFF/, '')) : undefined })
  writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`)
}
