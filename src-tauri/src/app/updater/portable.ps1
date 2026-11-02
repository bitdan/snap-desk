# 参数仅来自已验签的原生更新流程；等待原进程退出后才触碰当前 EXE。
$ErrorActionPreference = 'Stop'
$updateTarget = $env:SNAP_UPDATE_TARGET
$updateSource = $env:SNAP_UPDATE_SOURCE
$updateDirectory = [IO.Path]::GetDirectoryName($updateSource)
$updateBackup = Join-Path $updateDirectory 'old.exe'
$updateReady = Join-Path $updateDirectory 'ready'
$updateError = Join-Path $updateDirectory 'error.txt'
$updateReplaced = $false
try {
    # 提前取得原进程句柄，避免退出后同一 PID 被其他进程复用。
    $updateProcess = Get-Process -Id ([int]$env:SNAP_UPDATE_PARENT) -ErrorAction Stop
    $null = $updateProcess.Handle
    [IO.File]::WriteAllText($updateReady, 'ready')
    if (-not $updateProcess.WaitForExit(120000)) { throw '等待程序退出超时，已保留原版本。' }
    if ((Get-FileHash -LiteralPath $updateSource -Algorithm SHA256).Hash -ne $env:SNAP_UPDATE_HASH) {
        throw '暂存更新包校验失败，已保留原版本。'
    }
    # 杀毒软件可能短暂占用 EXE；有限重试，Replace 同时保留旧版供启动失败时回滚。
    for ($updateAttempt = 0; $updateAttempt -lt 30; $updateAttempt++) {
        try {
            [IO.File]::Replace($updateSource, $updateTarget, $updateBackup)
            $updateReplaced = $true
            break
        } catch {
            if ($updateAttempt -eq 29) { throw }
            Start-Sleep -Milliseconds 500
        }
    }
    Start-Process -FilePath $updateTarget -WorkingDirectory ([IO.Path]::GetDirectoryName($updateTarget)) -WindowStyle Hidden -ErrorAction Stop
    # 启动成功即提交更新；清理失败不能再回滚正在运行的新程序。
    $updateReplaced = $false
    Remove-Item -LiteralPath $updateBackup -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $updateReady -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $updateDirectory -ErrorAction SilentlyContinue
} catch {
    [IO.File]::WriteAllText($updateError, $_.Exception.Message)
    if ($updateReplaced -and (Test-Path -LiteralPath $updateBackup)) {
        [IO.File]::Replace($updateBackup, $updateTarget, $null)
    }
    # 超时时旧进程可能仍在运行，不额外启动第二个实例。
    if ($updateProcess -and $updateProcess.HasExited) {
        Start-Process -FilePath $updateTarget -ArgumentList '--update-failed' -WorkingDirectory ([IO.Path]::GetDirectoryName($updateTarget)) -WindowStyle Hidden
    }
}
