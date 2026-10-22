# 只观察已构建应用，不编译、不结束进程、不主动整理工作集；兼容 Windows PowerShell 5.1。
[CmdletBinding(DefaultParameterSetName = 'Attach')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Attach')][ValidateRange(1, 2147483647)][int]$ProcessId,
    [Parameter(Mandatory, ParameterSetName = 'Launch')][string]$Binary,
    [ValidateRange(1, 86400)][int]$DurationSeconds = 900,
    [ValidateRange(200, 60000)][int]$IntervalMilliseconds = 1000,
    [ValidateRange(0, 300)][int]$ExitGraceSeconds = 30,
    [ValidatePattern('^[A-Za-z0-9_-]+$')][string]$Label = 'baseline',
    [string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'resource-observer.psm1') -Force
if (!$OutputDirectory) {
    $OutputDirectory = Join-Path (Split-Path $PSScriptRoot -Parent) 'src-tauri/target/resource-observations'
}
# 每次运行独立目录，避免第二次运行覆盖基线。
$runDirectory = Join-Path $OutputDirectory ("{0}-{1}-{2}" -f $Label, (Get-Date -Format 'yyyyMMdd-HHmmss'), [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $runDirectory -Force | Out-Null
$phasePath = Join-Path $runDirectory 'phase.txt'
Set-Content -LiteralPath $phasePath -Value 'cold-start' -Encoding UTF8
Write-Host "采样目录：$runDirectory"
Write-Host "操作阶段：在另一个终端修改 $phasePath；脚本不会操作或关闭应用。"

if ($PSCmdlet.ParameterSetName -eq 'Launch') {
    $binaryPath = (Resolve-Path -LiteralPath $Binary).Path
    $launched = Start-Process -FilePath $binaryPath -WorkingDirectory (Split-Path $binaryPath -Parent) -WindowStyle Hidden -PassThru
    $ProcessId = $launched.Id
}
$rootProcess = Get-CimInstance Win32_Process -Filter "ProcessId = $ProcessId"
if (!$rootProcess) { throw "目标进程已退出；单实例应用请使用 -ProcessId 附加到已有主进程。" }
if ($rootProcess.Name -ine 'snap-desk.exe') { throw '请指定 snap-desk.exe 的 Rust 主进程，不能指定 WebView2、测试程序或启动器。' }
$rootIdentity = Get-ProcessIdentity $rootProcess
$known = @{ $rootIdentity = $rootProcess }
$previousCpu = @{}
$logicalProcessors = [Environment]::ProcessorCount
$started = [DateTime]::UtcNow
$metadata = [ordered]@{
    schemaVersion = 1; startedUtc = $started.ToString('o'); rootProcessId = $ProcessId
    rootIdentity = $rootIdentity; binary = $rootProcess.ExecutablePath
    label = $Label; intervalMilliseconds = $IntervalMilliseconds; durationSeconds = $DurationSeconds
    exitGraceSeconds = $ExitGraceSeconds; logicalProcessors = $logicalProcessors
    os = [Environment]::OSVersion.VersionString; powershell = $PSVersionTable.PSVersion.ToString()
}
$metadata | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $runDirectory 'metadata.json') -Encoding UTF8
$watch = [Diagnostics.Stopwatch]::StartNew()
$rootExitedAt = $null
$sampleIndex = 0
$writer = [IO.StreamWriter]::new((Join-Path $runDirectory 'processes.jsonl'), $false, [Text.UTF8Encoding]::new($false))
$summaryPath = Join-Path $runDirectory 'totals.csv'
try {
    while ($watch.Elapsed.TotalSeconds -lt $DurationSeconds) {
        $cycleStart = $watch.Elapsed.TotalMilliseconds
        # 需要全系统父子关系，但只落盘本应用进程；不读取命令行、窗口标题或用户内容。
        $snapshot = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, CreationDate, Name, WorkingSetSize, PrivatePageCount, KernelModeTime, UserModeTime, ThreadCount, HandleCount)
        $tree = @(Select-AppProcessTree -Snapshot $snapshot -Known $known)
        $elapsed = $watch.Elapsed.TotalSeconds
        $now = [DateTime]::UtcNow.ToString('o')
        $phase = (Get-Content -LiteralPath $phasePath -Raw -ErrorAction SilentlyContinue)
        if ([string]::IsNullOrWhiteSpace($phase)) { $phase = 'unlabelled' }
        $phase = $phase.Trim()
        $rows = @(
            foreach ($item in $tree) {
                $identity = Get-ProcessIdentity $item
                $cpuTicks = [double]$item.KernelModeTime + [double]$item.UserModeTime
                $cpuPercent = $null
                if ($previousCpu.ContainsKey($identity)) {
                    $previous = $previousCpu[$identity]
                    $deltaSeconds = $elapsed - $previous.elapsed
                    if ($deltaSeconds -gt 0) {
                        $cpuPercent = [Math]::Round([Math]::Max(0, $cpuTicks - $previous.ticks) / 1e7 / $deltaSeconds / $logicalProcessors * 100, 3)
                    }
                }
                $previousCpu[$identity] = @{ elapsed = $elapsed; ticks = $cpuTicks }
                [pscustomobject]@{
                    sample = $sampleIndex; utc = $now; elapsedSeconds = [Math]::Round($elapsed, 3); phase = $phase
                    identity = $identity; processId = [int]$item.ProcessId; parentProcessId = [int]$item.ParentProcessId
                    category = Get-AppProcessCategory -Process $item -RootIdentity $rootIdentity
                    name = $item.Name; workingSetBytes = [long]$item.WorkingSetSize
                    privateBytes = [long]$item.PrivatePageCount; cpuPercent = $cpuPercent
                    threads = [int]$item.ThreadCount; handles = [int]$item.HandleCount
                }
            }
        )
        foreach ($row in $rows) { $writer.WriteLine(($row | ConvertTo-Json -Compress)) }
        $writer.Flush()
        # 零值也写入，才能看见 Worker 退出和整个进程树退出后的资源归零。
        foreach ($category in @('rust-main', 'webview2', 'translation-worker', 'other-child', 'total')) {
            $group = @($rows | Where-Object { $category -eq 'total' -or $_.category -eq $category })
            $cpuSamples = @($group | Where-Object { $null -ne $_.cpuPercent })
            [pscustomobject]@{
                sample = $sampleIndex; utc = $now; elapsedSeconds = [Math]::Round($elapsed, 3); phase = $phase
                category = $category; processes = $group.Count
                workingSetBytes = [long](($group | Measure-Object workingSetBytes -Sum).Sum)
                privateBytes = [long](($group | Measure-Object privateBytes -Sum).Sum)
                cpuPercent = if ($cpuSamples.Count) { ($cpuSamples | Measure-Object cpuPercent -Sum).Sum } else { $null }
                threads = [long](($group | Measure-Object threads -Sum).Sum)
                handles = [long](($group | Measure-Object handles -Sum).Sum)
            } | Export-Csv -LiteralPath $summaryPath -Append -NoTypeInformation -Encoding UTF8
        }
        $rootAlive = @($tree | Where-Object { (Get-ProcessIdentity $_) -eq $rootIdentity }).Count -gt 0
        if (!$rootAlive -and $null -eq $rootExitedAt) { $rootExitedAt = $elapsed }
        if ($null -ne $rootExitedAt -and $elapsed - $rootExitedAt -ge $ExitGraceSeconds) { break }
        $sampleIndex++
        $remaining = $IntervalMilliseconds - ($watch.Elapsed.TotalMilliseconds - $cycleStart)
        if ($remaining -gt 0) { Start-Sleep -Milliseconds ([int]$remaining) }
    }
} finally {
    $writer.Dispose()
    $metadata['finishedUtc'] = [DateTime]::UtcNow.ToString('o')
    $metadata['rootExitObserved'] = $null -ne $rootExitedAt
    $metadata['samples'] = $sampleIndex + 1
    $metadata | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $runDirectory 'metadata.json') -Encoding UTF8
    Write-Host "采样结束：$runDirectory（到达时限不代表应用或子进程已退出，请核对 totals.csv）"
}
