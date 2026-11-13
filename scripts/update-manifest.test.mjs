import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtempSync, rmSync, unlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createUpdateManifest } from './update-manifest.mjs'

const version = '0.2.0-beta.2'
const prefix = `https://github.com/bitdan/snap-desk/releases/download/v${version}/`
const signature = signed => Buffer.from(`untrusted comment: test\nsig\ntrusted comment: timestamp:1\tfile:app\tversion:${signed}\nsig`).toString('base64')
// GitHub 把附件名中的空格规范化为点；本地 NSIS 文件名保留空格，其余附件已是统一名称。
const names = {
  nsis: `Snap Desk_${version}_x64-setup.exe`,
  standalone: `snap-desk-v${version}-windows-x64-standalone.exe`,
  app: `snap-desk-v${version}-macos-arm64.app.tar.gz`,
  appimage: `snap-desk-v${version}-linux-x64.AppImage`,
}

function fixture(t) {
  const directory = mkdtempSync(join(tmpdir(), 'snap-update-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  for (const name of Object.values(names)) {
    writeFileSync(join(directory, name), 'fixture')
    writeFileSync(join(directory, `${name}.sig`), signature(version))
  }
  // 仅供手动下载的附件不进入清单，也不要求签名。
  for (const name of [`snap-desk-v${version}-macos-arm64.dmg`, `snap-desk-v${version}-linux-x64.deb`]) writeFileSync(join(directory, name), 'fixture')
  const uploaded = Object.values(names).map(name => name.replaceAll(' ', '.'))
  return { directory, options: { version, repository: 'bitdan/snap-desk', directory },
    assets: uploaded.map(name => ({ name, browser_download_url: prefix + name })) }
}

test('各平台清单目标指向同一版本的对应更新包', t => {
  const { options, assets } = fixture(t)
  const result = createUpdateManifest(options)
  assert.deepEqual(Object.keys(result.platforms), ['windows-x86_64-nsis', 'windows-x86_64-standalone', 'darwin-aarch64-app', 'linux-x86_64-appimage'])
  assert.equal(result.platforms['windows-x86_64-standalone'].url, prefix + names.standalone)
  assert.equal(result.platforms['darwin-aarch64-app'].url, prefix + names.app)
  assert.equal(result.platforms['linux-x86_64-appimage'].url, prefix + names.appimage)
  // 已发布客户端按这两个后缀核对包类型，文件名规则不能变。
  assert.match(result.platforms['windows-x86_64-nsis'].url, /-setup\.exe$/)
  assert.match(result.platforms['windows-x86_64-standalone'].url, /-standalone\.exe$/)
  const published = createUpdateManifest({ ...options, assets })
  assert.equal(published.platforms['windows-x86_64-nsis'].url, `${prefix}Snap.Desk_${version}_x64-setup.exe`)
  assert.equal(published.platforms['windows-x86_64-nsis'].signature, result.platforms['windows-x86_64-nsis'].signature)
  const draft = assets.map(asset => ({ ...asset, browser_download_url: asset.browser_download_url.replace(`/v${version}/`, '/untagged-933b61e2e9b2b416bd8c/') }))
  assert.deepEqual(createUpdateManifest({ ...options, assets: draft }).platforms, published.platforms)
})

test('附件地址、数量或仓库不符时停止发布', t => {
  const { options, assets } = fixture(t)
  const [first, ...rest] = assets
  for (const address of [first.browser_download_url.replace(`/v${version}/`, '/v0.1.0/'), `${first.browser_download_url}?download=1`,
    first.browser_download_url.replace('/bitdan/', '/other/'), 'https://example.com/file']) {
    assert.throws(() => createUpdateManifest({ ...options, assets: [{ ...first, browser_download_url: address }, ...rest] }), /下载地址无效/)
  }
  assert.throws(() => createUpdateManifest({ ...options, assets: [] }), /必须包含一个/)
  assert.throws(() => createUpdateManifest({ ...options, assets: [...assets, first] }), /必须包含一个/)
  assert.throws(() => createUpdateManifest({ ...options, assets: rest }), /必须包含一个/)
  assert.throws(() => createUpdateManifest({ ...options, repository: '../other/repo' }), /仓库标识无效/)
})

test('任一平台缺少更新包、签名或签名版本不符时停止发布', t => {
  const { directory, options } = fixture(t)
  writeFileSync(join(directory, `${names.appimage}.sig`), signature('0.2.0-beta.20'))
  assert.throws(() => createUpdateManifest(options), /未绑定应用版本/)
  unlinkSync(join(directory, `${names.appimage}.sig`))
  assert.throws(() => createUpdateManifest(options), /ENOENT/)
  unlinkSync(join(directory, names.app))
  assert.throws(() => createUpdateManifest(options), /必须恰好有一个/)
})
