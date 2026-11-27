// 官网下载清单：Pages 部署时（.github/workflows/pages.yml）按最新正式版 Release 生成 docs/_data/downloads.json，
// 再由 docs/install/downloads.tsv 渲染成终端安装脚本读取的清单。文件不提交到仓库，发版后重新部署即可更新。
// SHA-256 取自 GitHub 为每个附件记录的 digest（上传时由 GitHub 计算），不需要下载附件。
//
// 用法：gh api repos/bitdan/snap-desk/releases/latest > release.json
//       node scripts/site-downloads.mjs release.json docs/_data/downloads.json
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { DISTRIBUTED, assetName, downloadManifest } from './distribution.mjs'

/** 从 Release 接口数据取出分发附件的 SHA-256，生成与包管理器渠道一致的下载清单。 */
export function releaseDownloads(release) {
  if (release.draft || release.prerelease) throw new Error(`官网只收录已公开的正式版：${release.tag_name}`)
  const version = String(release.tag_name).replace(/^v/, '')
  const hashes = Object.fromEntries(Object.keys(DISTRIBUTED).map(key => {
    const name = assetName(key, version)
    const digest = release.assets.find(asset => asset.name === name)?.digest
    const sha256 = /^sha256:([a-f0-9]{64})$/.exec(digest ?? '')?.[1]
    if (!sha256) throw new Error(`${release.tag_name} 缺少附件或 SHA-256：${name}`)
    return [key, sha256]
  }))
  return downloadManifest(version, hashes)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [input, output] = process.argv.slice(2)
  if (!input || !output) throw new Error('用法：node scripts/site-downloads.mjs <release.json> <输出文件>')
  const downloads = releaseDownloads(JSON.parse(readFileSync(input, 'utf8')))
  mkdirSync(dirname(resolve(output)), { recursive: true })
  writeFileSync(output, JSON.stringify(downloads, null, 2) + '\n')
  console.log(`官网下载清单：${downloads.version} → ${output}`)
}
