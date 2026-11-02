// MSI 使用纯数字版本；应用更新始终按原始 SemVer 比较，避免测试版与正式版混淆。
import { readFileSync, statSync, writeFileSync } from 'node:fs'
import { basename, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { readVersion } from './sync-version.mjs'

export function wixVersion(version) {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:-(alpha|beta|rc)\.([1-9]\d*))?$/.exec(version)
  if (!match) throw new Error('Windows 发布版本须为 x.y.z 或 x.y.z-alpha.N / beta.N / rc.N。')
  const [, major, minor, patch, channel, sequence] = match
  if (+major > 255 || +minor > 255 || +patch > 65535 || (sequence && +sequence > 9999)) {
    throw new Error('版本超过 MSI 数字字段范围，测试版序号须为 1～9999。')
  }
  const build = channel ? ({ alpha: 0, beta: 10000, rc: 20000 }[channel] + +sequence) : 65535
  // MSI 的产品升级只比较前三段，Tauri 的 WiX 模板允许同产品版本升级；
  // 第四段只用于区分包，是否升级由客户端原始 SemVer 和签名版本共同裁决。
  return `${major}.${minor}.${patch}.${build}`
}

export function createUpdateManifest({ version, repository, nsis, msi, standalone, notes = '', date = new Date().toISOString() }) {
  wixVersion(version)
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) throw new Error('GitHub 仓库标识无效。')
  const platforms = {}
  for (const [kind, file, suffix] of [['nsis', nsis, '-setup.exe'], ['msi', msi, '.msi'], ['standalone', standalone, '-standalone.exe']]) {
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
  const [nsis, msi, standalone, notesFile, output] = process.argv.slice(2)
  if (!output) throw new Error('用法：node scripts/windows-update.mjs <nsis> <msi> <standalone> <notes> <output>')
  const manifest = createUpdateManifest({ version: readVersion(), repository: process.env.GITHUB_REPOSITORY,
    nsis, msi, standalone, notes: readFileSync(notesFile, 'utf8') })
  writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`)
}
