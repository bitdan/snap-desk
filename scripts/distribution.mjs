// 包管理器分发清单：由正式版 GitHub Release 的附件生成 Scoop、Homebrew Cask、WinGet 与 AUR 的安装描述。
// 附件名与 scripts/release-assets.mjs 的清单一致；哈希按实际附件计算，发布流程（.github/workflows/distribute.yml）
// 再把清单推送到各渠道。测试版不进入包管理器。
//
// 渠道与安装方式：
//   Scoop    独立 EXE，清单放在 tap 仓库的 bucket/ 下  scoop bucket add snap-desk https://github.com/bitdan/homebrew-snap-desk
//   Homebrew DMG，Cask 放在同一仓库的 Casks/ 下         brew install --cask bitdan/snap-desk/snap-desk
//   WinGet   NSIS 安装包，向 microsoft/winget-pkgs 提 PR  winget install bitdan.SnapDesk
//   AUR      deb 解包，snap-desk-bin                     yay -S snap-desk-bin
//
// 用法：node scripts/distribution.mjs <版本> <附件目录> <输出目录>
import { createHash } from 'node:crypto'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { RELEASE_ASSETS } from './release-assets.mjs'

export const REPOSITORY = 'bitdan/snap-desk'
export const WINGET_ID = 'bitdan.SnapDesk'
const HOMEPAGE = `https://github.com/${REPOSITORY}`
const DESCRIPTION = 'Screenshot, pin, OCR and translation tool'

/** 分发用到的附件（按 release-assets 的唯一后缀查找）。 */
export const DISTRIBUTED = {
  standalone: '-standalone.exe',
  setup: '-setup.exe',
  dmg: '-macos-arm64.dmg',
  deb: '-linux-x64.deb',
}

/** 正式版才分发：WinGet 与 Homebrew 不接受预发布版本，AUR 的 pkgver 也不能含连字符。 */
export function isStable(version) {
  return /^\d+\.\d+\.\d+$/.test(version)
}

export function assetName(key, version) {
  const asset = RELEASE_ASSETS.find(a => a.suffix === DISTRIBUTED[key])
  if (!asset) throw new Error(`发布清单中没有 ${DISTRIBUTED[key]}`)
  return asset.name ? asset.name(version) : asset.match(version)
}

export function assetUrl(key, version) {
  return `${HOMEPAGE}/releases/download/v${version}/${assetName(key, version)}`
}

/** Scoop：独立 EXE 改名为 snap-desk.exe；checkver / autoupdate 让 `scoop update` 也能自行发现新版本。 */
export function scoopManifest(version, hashes) {
  const template = (v) => `${HOMEPAGE}/releases/download/v${v}/snap-desk-v${v}-windows-x64-standalone.exe#/snap-desk.exe`
  return JSON.stringify({
    version,
    description: DESCRIPTION,
    homepage: HOMEPAGE,
    license: 'GPL-3.0-only',
    architecture: { '64bit': { url: template(version), hash: hashes.standalone } },
    bin: 'snap-desk.exe',
    shortcuts: [['snap-desk.exe', 'Snap Desk']],
    checkver: { github: HOMEPAGE },
    autoupdate: { architecture: { '64bit': { url: template('$version') } } },
  }, null, 2) + '\n'
}

/** Homebrew Cask：只有 Apple 芯片版本；应用自带更新器，标记 auto_updates。 */
export function homebrewCask(version, hashes) {
  return `cask "snap-desk" do
  version "${version}"
  sha256 "${hashes.dmg}"

  url "${HOMEPAGE}/releases/download/v#{version}/snap-desk-v#{version}-macos-arm64.dmg"
  name "Snap Desk"
  desc "${DESCRIPTION}"
  homepage "${HOMEPAGE}"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates true
  depends_on arch: :arm64
  depends_on macos: ">= :ventura"

  app "Snap Desk.app"

  # 应用使用自签名证书、未经公证：去掉下载隔离标记，否则 Gatekeeper 会提示“已损坏”而无法打开。
  postflight do
    system_command "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "#{appdir}/Snap Desk.app"]
  end

  zap trash: [
    "~/Library/Application Support/io.snapdesk.app",
    "~/Library/Caches/io.snapdesk.app",
    "~/Library/Logs/io.snapdesk.app",
  ]
end
`
}

/** WinGet 多文件清单（version / installer / defaultLocale），由 wingetcreate submit 提交到 winget-pkgs。 */
export function wingetManifests(version, hashes) {
  const head = `PackageIdentifier: ${WINGET_ID}\nPackageVersion: ${version}\n`
  const tail = 'ManifestVersion: 1.9.0\n'
  return {
    [`${WINGET_ID}.yaml`]: `${head}DefaultLocale: en-US\nManifestType: version\n${tail}`,
    [`${WINGET_ID}.installer.yaml`]: `${head}InstallerType: nullsoft
Scope: user
InstallModes:
- interactive
- silent
UpgradeBehavior: install
Installers:
- Architecture: x64
  InstallerUrl: ${assetUrl('setup', version)}
  InstallerSha256: ${hashes.setup.toUpperCase()}
ManifestType: installer
${tail}`,
    [`${WINGET_ID}.locale.en-US.yaml`]: `${head}PackageLocale: en-US
Publisher: bitdan
PublisherUrl: https://github.com/bitdan
PackageName: Snap Desk
PackageUrl: ${HOMEPAGE}
License: GPL-3.0-only
LicenseUrl: ${HOMEPAGE}/blob/master/LICENSE
ShortDescription: ${DESCRIPTION}
Description: Free and open-source screenshot and screen recording tool with scrolling capture, annotation, pins, OCR, translation and clipboard history.
Tags:
- screenshot
- screen-recorder
- ocr
- translation
- clipboard
ReleaseNotesUrl: ${HOMEPAGE}/releases/tag/v${version}
ManifestType: defaultLocale
${tail}`,
  }
}

/** AUR 运行依赖：与 deb 的 Depends（scripts/package-gpui.mjs）对应的 Arch 包名。 */
const AUR_DEPENDS = ['pipewire', 'libxcb', 'libxrandr', 'dbus', 'gstreamer', 'gst-plugins-base', 'gst-plugins-good',
  'libxkbcommon', 'libxkbcommon-x11', 'fontconfig', 'vulkan-icd-loader']

/** AUR snap-desk-bin：直接解包发布的 deb（主程序、桌面文件与图标都在 data.tar 里）。 */
export function aurPackage(version, hashes) {
  const deb = `snap-desk-${version}.deb`
  const pkgbuild = `# Maintainer: bitdan
pkgname=snap-desk-bin
pkgver=${version}
pkgrel=1
pkgdesc='${DESCRIPTION}'
arch=('x86_64')
url='${HOMEPAGE}'
license=('GPL-3.0-only')
depends=(${AUR_DEPENDS.map(d => `'${d}'`).join(' ')})
provides=('snap-desk')
conflicts=('snap-desk')
options=('!strip' '!debug')
source=("${deb}::${HOMEPAGE}/releases/download/v\${pkgver}/snap-desk-v\${pkgver}-linux-x64.deb")
sha256sums=('${hashes.deb}')
noextract=("${deb}")

package() {
  bsdtar -xOf "${deb}" 'data.tar.*' | bsdtar -xf - -C "$pkgdir"
}
`
  const srcinfo = `pkgbase = snap-desk-bin
\tpkgdesc = ${DESCRIPTION}
\tpkgver = ${version}
\tpkgrel = 1
\turl = ${HOMEPAGE}
\tarch = x86_64
\tlicense = GPL-3.0-only
${AUR_DEPENDS.map(d => `\tdepends = ${d}`).join('\n')}
\tprovides = snap-desk
\tconflicts = snap-desk
\toptions = !strip
\toptions = !debug
\tnoextract = ${deb}
\tsource = ${deb}::${assetUrl('deb', version)}
\tsha256sums = ${hashes.deb}

pkgname = snap-desk-bin
`
  return { PKGBUILD: pkgbuild, '.SRCINFO': srcinfo }
}

/** 读取附件目录中分发所需的附件并计算 SHA-256（小写十六进制）。 */
export function hashAssets(directory, version) {
  return Object.fromEntries(Object.keys(DISTRIBUTED).map(key => {
    const bytes = readFileSync(join(directory, assetName(key, version)))
    return [key, createHash('sha256').update(bytes).digest('hex')]
  }))
}

/** 把全部渠道的清单写到输出目录：scoop/、homebrew/、winget/、aur/。 */
export function writeDistribution(version, hashes, output) {
  const write = (dir, files) => {
    mkdirSync(join(output, dir), { recursive: true })
    for (const [name, content] of Object.entries(files)) writeFileSync(join(output, dir, name), content)
  }
  write('scoop', { 'snap-desk.json': scoopManifest(version, hashes) })
  write('homebrew', { 'snap-desk.rb': homebrewCask(version, hashes) })
  write('winget', wingetManifests(version, hashes))
  write('aur', aurPackage(version, hashes))
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [version, assets, output] = process.argv.slice(2)
  if (!version || !assets || !output) throw new Error('用法：node scripts/distribution.mjs <版本> <附件目录> <输出目录>')
  if (!isStable(version)) throw new Error(`${version} 是测试版，不分发到包管理器。`)
  writeDistribution(version, hashAssets(resolve(assets), version), resolve(output))
  console.log(`分发清单：${resolve(output)}`)
}
