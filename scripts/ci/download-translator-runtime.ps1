# 应用/模型流水线共用同一份受审锁；旁边下载的 .sha256 不能替代仓库信任根。
param([string]$RuntimeDir = 'src-tauri/translator-runtime')
$ErrorActionPreference = 'Stop'
$env:PYTHONUTF8 = '1'
$env:PYTHONIOENCODING = 'utf-8'
$lock = Get-Content -LiteralPath "$PSScriptRoot/translation-engine.json" -Raw | ConvertFrom-Json
if ($lock.tag -notmatch '^translation-engine-v4\.6\.0-r[1-9][0-9]*$' -or $lock.sha256 -cnotmatch '^[0-9a-f]{64}$') {
    throw '请先构建、审核引擎，并将真实 ZIP SHA-256 写入 scripts/ci/translation-engine.json。'
}
if ($lock.runtime -ne '4.6.0' -or $lock.protocol -ne 1 -or $lock.target -ne 'windows-x64' -or $lock.asset -ne 'translator-runtime-windows-x64.zip') {
    throw '引擎锁与当前应用的 runtime/协议/架构/附件契约不兼容。'
}
if (Test-Path -LiteralPath $RuntimeDir) { throw 'runtime 目录必须不存在，避免混入旧 DLL。' }
$destination = [System.IO.Path]::GetFullPath($RuntimeDir)
$runtimeParent = [System.IO.Path]::GetDirectoryName($destination)
[System.IO.Directory]::CreateDirectory($runtimeParent) | Out-Null
# 与目标目录同卷，避免 TEMP 在 C:、工作区在 D:/F: 时目录原子移动失败。
$download = Join-Path $runtimeParent ('.translator-download-' + [guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $download | Out-Null
try {
    gh release download $lock.tag --repo $env:GH_REPO --pattern $lock.asset --dir $download
    if ($LASTEXITCODE -ne 0) { throw '下载锁定引擎失败。' }
    $zip = Join-Path $download $lock.asset
    if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant() -cne $lock.sha256) {
        throw '翻译引擎 ZIP 摘要不匹配。'
    }
    # 解压前拒绝目录穿越、重复成员和超大压缩包。
    # 先在临时目录完成全部检查；失败不留下最终 runtime 目录，允许直接重试。
    $staging = Join-Path $download 'runtime'
    python "$PSScriptRoot/translator-runtime.py" extract $zip $staging
    if ($LASTEXITCODE -ne 0) { throw '引擎解压失败。' }
    & "$PSScriptRoot/verify-translator-runtime.ps1" -RuntimeDir $staging -EngineTag $lock.tag
    # Directory.Move 拒绝覆盖已有目录，不会把文件混入另一份 runtime。
    [System.IO.Directory]::Move($staging, $destination)
} finally {
    # download 是本脚本生成的临时 UUID 目录，仅清理这个已解析的路径。
    $resolved = [System.IO.Path]::GetFullPath($download)
    if ([System.IO.Path]::GetDirectoryName($resolved) -ne $runtimeParent -or [System.IO.Path]::GetFileName($resolved) -notmatch '^\.translator-download-[0-9a-f-]{36}$') { throw '临时路径越界。' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
