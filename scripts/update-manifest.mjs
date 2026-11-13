// 各平台的更新包共用一份应用更新清单；每种分发形式对应独立目标，互不替代。
import { readFileSync, writeFileSync } from 'node:fs'
import { basename, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { RELEASE_ASSETS, findAsset } from './release-assets.mjs'
import { readVersion } from './sync-version.mjs'

/** directory 为汇总后的发布附件目录；任一平台缺少更新包或签名都停止，不发布残缺清单。 */
export function createUpdateManifest({ version, repository, directory, notes = '', date = new Date().toISOString(), assets }) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) throw new Error('GitHub 仓库标识无效。')
  if (assets !== undefined && !Array.isArray(assets)) throw new Error('发布附件列表无效。')
  const platforms = {}
  for (const { target, suffix, label } of RELEASE_ASSETS.filter(asset => asset.target)) {
    const file = findAsset(directory, suffix, label)
    const signature = readFileSync(`${file}.sig`, 'utf8').trim()
    const decoded = Buffer.from(signature, 'base64').toString('utf8')
    // 必须使用 --app-version 签名，客户端拒绝把旧包伪装成新版本。
    const trusted = decoded.split(/\r?\n/).find(line => line.startsWith('trusted comment: ')) ?? ''
    if (!trusted.slice('trusted comment: '.length).split('\t').includes(`version:${version}`)) throw new Error(`${label} 签名未绑定应用版本 ${version}。`)
    // GitHub 上传时会重命名含空格的文件；以已上传附件为准，不能从本地文件名猜下载地址。
    const uploaded = assets?.filter(asset => typeof asset.name === 'string' && asset.name.endsWith(suffix))
    if (uploaded && uploaded.length !== 1) throw new Error(`发布附件必须包含一个 ${label}。`)
    const prefix = `https://github.com/${repository}/releases/download/v${version}/`
    let url = `${prefix}${encodeURIComponent(basename(file))}`
    if (uploaded) {
      const asset = uploaded[0]
      let address
      try { address = new URL(asset.browser_download_url) } catch { throw new Error(`${label} 发布附件下载地址无效。`) }
      const path = address.pathname.split('/')
      // 草稿地址使用 untagged-* 临时标识；保留 GitHub 规范化后的附件名，预写正式发布地址。
      const tag = path[5]
      if (address.origin !== 'https://github.com' || path.slice(1, 5).join('/') !== `${repository}/releases/download`
        || path.length !== 7 || (tag !== `v${version}` && !/^untagged-[0-9a-f]+$/.test(tag))
        || decodeURIComponent(path[6]) !== asset.name || address.search || address.hash
        || asset.name.includes('/') || asset.name.includes('\\')) {
        throw new Error(`${label} 发布附件下载地址无效。`)
      }
      url = `${prefix}${encodeURIComponent(asset.name)}`
    }
    platforms[target] = { url, signature }
  }
  return { version, notes, pub_date: date, platforms }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [directory, notesFile, output, assetsFile] = process.argv.slice(2)
  if (!output) throw new Error('用法：node scripts/update-manifest.mjs <assets-directory> <notes> <output> [assets-json]')
  const manifest = createUpdateManifest({ version: readVersion(), repository: process.env.GITHUB_REPOSITORY,
    directory, notes: readFileSync(notesFile, 'utf8'),
    assets: assetsFile ? JSON.parse(readFileSync(assetsFile, 'utf8').replace(/^﻿/, '')) : undefined })
  writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`)
}
