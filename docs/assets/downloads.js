const manifestUrl = 'https://raw.githubusercontent.com/bitdan/homebrew-snap-desk/HEAD/downloads.json'
const releaseRoot = 'https://github.com/bitdan/snap-desk/releases/download/'
const formats = ['setup', 'standalone', 'dmg', 'appimage', 'deb']

// 官网也校验远程清单。文件名和地址会写进终端命令，只允许同版本的官方附件。
export function parseDownloads({ version, assets }) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Invalid stable version')
  const prefix = `${releaseRoot}v${version}/snap-desk-v${version}-`
  for (const key of formats) {
    const asset = assets?.[key]
    if (typeof asset?.url !== 'string' || !asset.url.startsWith(prefix)
      || !/^[A-Za-z0-9][A-Za-z0-9.-]*$/.test(asset.url.slice(prefix.length))
      || typeof asset.sha256 !== 'string' || !/^[a-f0-9]{64}$/.test(asset.sha256)) {
      throw new Error(`Invalid download: ${key}`)
    }
  }
  return { version, assets }
}

// 先下载到临时文件，核对摘要后才保存为可识别的安装包。
export function downloadCommand(key, { url, sha256 }) {
  const file = url.split('/').at(-1)
  if (key === 'setup' || key === 'standalone') {
    return [
      "& { $ErrorActionPreference = 'Stop';",
      `$destination = Join-Path $PWD '${file}';`,
      "$tmp = Join-Path $PWD ('.snap-desk-' + [Guid]::NewGuid());",
      `try { Invoke-WebRequest -UseBasicParsing -Uri '${url}' -OutFile $tmp;`,
      `if ((Get-FileHash -LiteralPath $tmp -Algorithm SHA256).Hash -ine '${sha256}') { throw 'SHA-256 mismatch' };`,
      `[IO.File]::Move($tmp, $destination); Write-Output 'Saved ${file}'`,
      '} finally { Remove-Item -LiteralPath $tmp -ErrorAction SilentlyContinue } }',
    ].join(' ')
  }

  const checksum = key === 'dmg' ? 'shasum -a 256 -c' : 'sha256sum -c'
  const permissions = key === 'appimage' ? 'chmod +x "$tmp"' : 'chmod a+r "$tmp"'
  // 用 && 逐步传递失败，命令处于 if 或其他条件语境时也会正确退出。
  return `(${[
    'tmp=$(mktemp ./.snap-desk.XXXXXXXX)',
    `trap 'rm -f "$tmp"' 0`,
    `curl -fL -o "$tmp" '${url}'`,
    `printf '%s  %s\\n' '${sha256}' "$tmp" | ${checksum}`,
    permissions,
    `test ! -e './${file}'`,
    `ln "$tmp" './${file}'`,
    `printf '%s\\n' 'Saved ${file}'`,
  ].join(' && ')})`
}

function applyDownloads(section, { version, assets }) {
  section.querySelector('[data-download-version]').textContent = `v${version}`
  for (const row of section.querySelectorAll('[data-download-format]')) {
    const key = row.dataset.downloadFormat
    const asset = assets[key]
    row.querySelector('a').href = asset.url
    row.querySelector('textarea').value = downloadCommand(key, asset)
  }
}

async function refreshDownloads(section, initialVersion) {
  try {
    const response = await fetch(manifestUrl, { signal: AbortSignal.timeout(5000) })
    if (!response.ok) return
    const manifest = parseDownloads(await response.json())
    const current = initialVersion.split('.').map(Number)
    const incoming = manifest.version.split('.').map(Number)
    const firstChange = incoming.findIndex((part, index) => part !== current[index])
    if (firstChange === -1 || incoming[firstChange] > current[firstChange]) applyDownloads(section, manifest)
  } catch {
    // 网络失败时继续提供官网随页面发布的已验证版本。
  }
}

const section = globalThis.document?.querySelector('#download')
if (section) {
  const initial = parseDownloads(JSON.parse(section.querySelector('[data-downloads]').textContent))
  applyDownloads(section, initial)
  for (const row of section.querySelectorAll('[data-download-format]')) {
    const button = row.querySelector('button')
    const field = row.querySelector('textarea')
    const status = row.querySelector('[role=status]')
    let clearStatus
    button.addEventListener('click', async () => {
      clearTimeout(clearStatus)
      status.textContent = ''
      try {
        await navigator.clipboard.writeText(field.value)
        status.textContent = section.dataset.copied
        clearStatus = setTimeout(() => { status.textContent = '' }, 2500)
      } catch {
        status.textContent = section.dataset.copyFailed
        field.hidden = false
        field.focus()
        field.select()
      }
    })
    button.hidden = false
  }
  void refreshDownloads(section, initial.version)
}
