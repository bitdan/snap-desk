import assert from 'node:assert/strict'
import { test } from 'node:test'
import { assetName, DISTRIBUTED } from './distribution.mjs'
import { releaseDownloads } from './site-downloads.mjs'

const version = '3.12.2'
const digests = Object.fromEntries(Object.keys(DISTRIBUTED).map((key, index) => [key, String(index + 1).repeat(64)]))

/** 与 GitHub Release 接口相同的结构：分发附件之外还有签名、更新包等其他附件。 */
function release(overrides = {}) {
  const assets = Object.keys(DISTRIBUTED).map(key => ({ name: assetName(key, version), digest: `sha256:${digests[key]}` }))
  assets.push({ name: `${assetName('setup', version)}.sig`, digest: `sha256:${'f'.repeat(64)}` }, { name: 'updater.json', digest: null })
  return { tag_name: `v${version}`, draft: false, prerelease: false, assets, ...overrides }
}

test('官网下载清单取自最新正式版附件的 SHA-256', () => {
  const downloads = releaseDownloads(release())
  assert.equal(downloads.version, version)
  assert.deepEqual(Object.keys(downloads.assets).sort(), Object.keys(DISTRIBUTED).sort())
  assert.deepEqual(downloads.assets.setup, {
    url: `https://github.com/bitdan/snap-desk/releases/download/v${version}/snap-desk-v${version}-windows-x64-setup.exe`,
    sha256: digests.setup,
  })
})

test('缺少附件、digest 或不是正式版时停止部署', () => {
  const withoutDmg = release()
  withoutDmg.assets = withoutDmg.assets.filter(asset => !asset.name.endsWith('.dmg'))
  assert.throws(() => releaseDownloads(withoutDmg), /macos-arm64\.dmg/)
  const withoutDigest = release()
  withoutDigest.assets[0].digest = null
  assert.throws(() => releaseDownloads(withoutDigest), /SHA-256/)
  assert.throws(() => releaseDownloads(release({ prerelease: true })), /正式版/)
  assert.throws(() => releaseDownloads(release({ draft: true })), /正式版/)
  assert.throws(() => releaseDownloads({ ...release(), tag_name: 'v3.12.3-beta.1' }), /SHA-256|正式版/)
})
