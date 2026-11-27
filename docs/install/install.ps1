# 从正式版清单定位 NSIS 安装包，校验后运行原有安装程序。
& {
    $ErrorActionPreference = 'Stop'

    function Get-SnapDeskAsset([string]$ManifestUrl) {
        $manifest = & curl.exe -fsL --connect-timeout 8 --max-time 20 $ManifestUrl
        if ($LASTEXITCODE -ne 0) { return $null }
        $asset = $manifest | Where-Object { $_ -match '^setup\s' } | Select-Object -First 1
        if (!$asset) { return $null }
        $parts = $asset.TrimEnd().Split([char]9)
        if ($parts.Count -ne 3 -or $parts[2] -cnotmatch '^[0-9a-f]{64}$') { return $null }
        $pattern = '^https://github\.com/bitdan/snap-desk/releases/download/v(?<version>\d+\.\d+\.\d+)/snap-desk-v\k<version>-windows-x64-setup\.exe$'
        if ($parts[0] -cne 'setup' -or $parts[1] -cnotmatch $pattern) { return $null }
        return @{ Url = $parts[1]; Sha256 = $parts[2] }
    }

    $architecture = if ($env:PROCESSOR_ARCHITEW6432) {
        $env:PROCESSOR_ARCHITEW6432
    } elseif ($env:PROCESSOR_ARCHITECTURE) {
        $env:PROCESSOR_ARCHITECTURE
    } else {
        [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    }
    if ([Environment]::OSVersion.Platform -ne 'Win32NT' -or $architecture -notin 'AMD64', 'X64') {
        throw 'Snap Desk requires x64 Windows.'
    }
    # 官网清单在每次发版后重新部署；raw.githubusercontent.com 在部分网络下不可达，tap 仓库的副本只作备用。
    $asset = Get-SnapDeskAsset 'https://bitdan.github.io/snap-desk/install/downloads.tsv'
    if (!$asset) {
        $asset = Get-SnapDeskAsset 'https://raw.githubusercontent.com/bitdan/homebrew-snap-desk/HEAD/downloads.tsv'
    }
    if (!$asset) { throw 'Could not resolve a stable download. See GitHub Releases.' }

    $directory = Join-Path ([IO.Path]::GetTempPath()) ('snap-desk-' + [Guid]::NewGuid())
    $null = New-Item -ItemType Directory -Path $directory
    try {
        $installer = Join-Path $directory 'snap-desk-setup.exe'
        & curl.exe -fLS --output $installer $asset.Url
        if ($LASTEXITCODE -ne 0) { throw 'Download failed.' }
        if ((Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash -ine $asset.Sha256) { throw 'SHA-256 mismatch.' }
        $process = Start-Process -FilePath $installer -ArgumentList '/S' -Wait -PassThru
        if ($process.ExitCode -ne 0) { throw "Snap Desk installer exited with code $($process.ExitCode)." }
        Write-Output 'Snap Desk installed.'
    } finally {
        Remove-Item -LiteralPath $installer -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $directory -ErrorAction SilentlyContinue
    }
}
