param([Parameter(Mandatory)][string]$Tag, [Parameter(Mandatory)][string]$NotesFile, [switch]$Recover)
$ErrorActionPreference = 'Stop'

# 草稿没有可用的 /releases/tags 地址；始终从列表确定 ID，上传完成后才公开。
function Find-Release {
    $pages = gh api "repos/$env:GITHUB_REPOSITORY/releases" --paginate --slurp
    if ($LASTEXITCODE -ne 0) { throw '读取发布列表失败。' }
    $releasesByTag = @((($pages -join "`n") | ConvertFrom-Json) | ForEach-Object { $_ } | Where-Object tag_name -eq $Tag)
    if ($releasesByTag.Count -gt 1) { throw '同一标签对应多个 Release，停止发布。' }
    if ($releasesByTag.Count -eq 1) { return $releasesByTag[0] }
    return $null
}

$version = (Get-Content package.json -Raw | ConvertFrom-Json).version
if ($Tag -notmatch '^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$' -or $Tag -ne "v$version") { throw '发布标签与应用版本不一致。' }
$release = Find-Release
if ($release -and -not $release.draft) { throw '此版本已公开，不能覆盖发布附件。' }
$directory = 'src-tauri/target/release'
$installerDirectory = "$directory/bundle/nsis"
$standalone = "$directory/snap-desk-$Tag-windows-x64-standalone.exe"

if ($Recover) {
    if (-not $release) { throw '找不到待恢复的 Release 草稿。' }
    # 只恢复发布工具变更，应用与锁定依赖必须和标签完全一致，避免重签来源不一致的包。
    $changes = git diff "$Tag..HEAD" -- src src-tauri package.json package-lock.json
    if ($LASTEXITCODE -ne 0 -or $changes) { throw '应用源码已变化，不能复用旧版本安装包。' }
    if ($PSVersionTable.PSVersion -lt [version]'7.4') { throw '二进制下载需要 PowerShell 7.4 或更高版本。' }
    New-Item -ItemType Directory -Force -Path $installerDirectory | Out-Null
    foreach ($suffix in @('-setup.exe', '-standalone.exe')) {
        $assets = @($release.assets | Where-Object { $_.name.EndsWith($suffix) })
        if ($assets.Count -ne 1) { throw "草稿缺少唯一的 $suffix 安装包。" }
        $asset = $assets[0]
        if ([IO.Path]::GetFileName($asset.name) -ne $asset.name -or $asset.name -match '[/\\]') { throw '附件名称无效。' }
        $parent = if ($suffix -eq '-setup.exe') { $installerDirectory } else { $directory }
        $output = Join-Path $parent $asset.name
        gh api "repos/$env:GITHUB_REPOSITORY/releases/assets/$($asset.id)" -H 'Accept: application/octet-stream' > $output
        if ($LASTEXITCODE -ne 0) { throw '下载草稿安装包失败。' }
        $digest = 'sha256:' + (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($digest -ne $asset.digest -or (Get-Item -LiteralPath $output).Length -ne $asset.size) { throw '草稿安装包哈希或尺寸不匹配。' }
    }
} elseif ($release) { throw 'Release 草稿已存在，请使用恢复流程继续发布。' }

$installers = @(Get-ChildItem -Path "$installerDirectory/*-setup.exe" -File)
if ($installers.Count -ne 1 -or -not (Test-Path -LiteralPath $standalone -PathType Leaf)) { throw '缺少唯一的 NSIS 安装包或独立 EXE。' }
foreach ($file in @($installers[0].FullName, $standalone)) {
    node node_modules/@tauri-apps/cli/tauri.js signer sign --app-version $version $file
    if ($LASTEXITCODE -ne 0) { throw '安装包签名失败。' }
}

if (-not $release) {
    $isTest = $Tag.Contains('-')
    $channel = if ($isTest) { '测试版' } else { '正式版' }
    $arguments = @('release', 'create', $Tag, '--draft', '--verify-tag', '--title', "Snap Desk $Tag（$channel）", '--generate-notes', '--notes-file', $NotesFile)
    if ($isTest) { $arguments += '--prerelease' }
    $arguments += @($installers[0].FullName, $standalone, 'LICENSE')
    gh @arguments
    if ($LASTEXITCODE -ne 0) { throw '创建 Release 草稿失败。' }
    $release = Find-Release
    if (-not $release -or -not $release.draft) { throw '未找到刚创建的 Release 草稿。' }
}

$notes = "$directory/update-release-notes.txt"
$assets = "$directory/update-assets.json"
$manifest = "$directory/updater.json"
$release.body | Set-Content -LiteralPath $notes -Encoding utf8
ConvertTo-Json -InputObject @($release.assets) -Depth 10 | Set-Content -LiteralPath $assets -Encoding utf8
node scripts/windows-update.mjs $installers[0].FullName $standalone $notes $manifest $assets
if ($LASTEXITCODE -ne 0) { throw '生成更新清单失败。' }
gh release upload $Tag $manifest "$($installers[0].FullName).sig" "$standalone.sig" --clobber
if ($LASTEXITCODE -ne 0) { throw '上传更新清单和签名失败。' }
$latest = if ($Tag.Contains('-')) { 'false' } else { 'true' }
gh api --method PATCH "repos/$env:GITHUB_REPOSITORY/releases/$($release.id)" -F draft=false -f "make_latest=$latest" --jq '.html_url'
if ($LASTEXITCODE -ne 0) { throw '公开 Release 失败。' }
