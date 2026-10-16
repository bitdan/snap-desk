# 单独启动已构建的 release 测试程序，采样该进程的内存高水位；不包含 Cargo 编译开销。
param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [Parameter(Mandatory = $true)][ValidatePattern('^[A-Za-z0-9_:]+$')][string]$Test,
    [ValidatePattern('^[A-Za-z0-9_-]+$')][string]$Label = 'baseline'
)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
$outputDir = Join-Path $taskRoot 'src-tauri/target/perf-results'
New-Item -ItemType Directory -Force $outputDir | Out-Null
$stem = "$Label-$($Test.Replace('::', '-'))"
$stdoutPath = Join-Path $outputDir "$stem.stdout.log"
$stderrPath = Join-Path $outputDir "$stem.stderr.log"
$binaryPath = (Resolve-Path -LiteralPath $Binary).Path
$watch = [System.Diagnostics.Stopwatch]::StartNew()
$process = Start-Process -FilePath $binaryPath -ArgumentList @($Test, '--exact', '--ignored', '--nocapture', '--test-threads=1') -WorkingDirectory (Join-Path $taskRoot 'src-tauri') -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
# Windows PowerShell 5 需要提前持有句柄，否则短进程退出后 ExitCode 可能变为 null。
$process.Handle | Out-Null
$peak = 0L
$peakPrivate = 0L
while (!$process.HasExited) {
    $process.Refresh()
    if (!$process.HasExited) {
        $peak = [Math]::Max($peak, $process.PeakWorkingSet64)
        $peakPrivate = [Math]::Max($peakPrivate, $process.PrivateMemorySize64)
    }
    Start-Sleep -Milliseconds 100
}
$process.WaitForExit()
$watch.Stop()
Get-Content -LiteralPath $stdoutPath
Get-Content -LiteralPath $stderrPath
$result = [pscustomobject]@{
    test = $Test; label = $Label; elapsedSeconds = [Math]::Round($watch.Elapsed.TotalSeconds, 2)
    peakWorkingSetMiB = [Math]::Round($peak / 1MB, 1)
    sampledPeakPrivateMiB = [Math]::Round($peakPrivate / 1MB, 1)
    exitCode = $process.ExitCode
}
$result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $outputDir "$stem.json") -Encoding UTF8
$result | Format-List
if ($process.ExitCode -ne 0) { throw "Native benchmark failed: $Test" }
