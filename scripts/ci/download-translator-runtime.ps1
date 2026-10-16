# 应用/模型流水线共用同一份受审锁；旁边下载的 .sha256 不能替代仓库信任根。
param([string]$RuntimeDir = 'src-tauri/translator-runtime')
$ErrorActionPreference = 'Stop'
$lock = Get-Content -LiteralPath "$PSScriptRoot/translation-engine.json" -Raw | ConvertFrom-Json
if ($lock.tag -notmatch '^translation-engine-v4\.6\.0-r[1-9][0-9]*$' -or $lock.sha256 -cnotmatch '^[0-9a-f]{64}$') {
    throw '请先构建、审核引擎，并将真实 ZIP SHA-256 写入 scripts/ci/translation-engine.json。'
}
if ($lock.runtime -ne '4.6.0' -or $lock.protocol -ne 1 -or $lock.target -ne 'windows-x64' -or $lock.asset -ne 'translator-runtime-windows-x64.zip') {
    throw '引擎锁与当前应用的 runtime/协议/架构/附件契约不兼容。'
}
if (Test-Path -LiteralPath $RuntimeDir) { throw 'runtime 目录必须不存在，避免混入旧 DLL。' }
$download = Join-Path ([System.IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $download | Out-Null
try {
    gh release download $lock.tag --repo $env:GH_REPO --pattern $lock.asset --dir $download
    if ($LASTEXITCODE -ne 0) { throw '下载锁定引擎失败。' }
    $zip = Join-Path $download $lock.asset
    if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant() -cne $lock.sha256) {
        throw '翻译引擎 ZIP 摘要不匹配。'
    }
    # 解压前拒绝目录穿越、重复成员和超大压缩包。
    python "$PSScriptRoot/translator-runtime.py" extract $zip $RuntimeDir
    if ($LASTEXITCODE -ne 0) { throw '引擎解压失败。' }
    & "$PSScriptRoot/verify-translator-runtime.ps1" -RuntimeDir $RuntimeDir -EngineTag $lock.tag
} finally {
    # download 是本脚本生成的临时 UUID 目录，仅清理这个已解析的路径。
    $tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
    $resolved = [System.IO.Path]::GetFullPath($download)
    if (-not $resolved.StartsWith($tempRoot, [System.StringComparison]::OrdinalIgnoreCase)) { throw '临时路径越界。' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
