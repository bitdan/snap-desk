# 只读采样已运行应用的完整进程树；不创建贴图、不修改剪贴板、不读取命令行或窗口内容。
param(
    [Parameter(Mandatory = $true)][int]$RootProcessId,
    [ValidateSet(0, 1, 4, 8, 12)][int]$PinCount = 0,
    [ValidateSet('baseline', 'visible', 'hidden', 'closed', 'scroll', 'export')][string]$Stage = 'baseline',
    [ValidateRange(1, 300)][int]$Seconds = 5,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../src-tauri/target/benchmark-results'),
    [switch]$LibraryOnly
)
$ErrorActionPreference = 'Stop'

function Get-CreationKey { param($Value) ([datetime]$Value).ToUniversalTime().Ticks }

function Select-ProcessTree {
    param([object[]]$Rows, [int]$RootId, [hashtable]$Known)
    # PID 配合创建时间，既保留父进程退出后的已知子进程，也不把复用 PID 的其他应用算进来。
    $selected = @{}
    foreach ($row in $Rows) {
        $id = [int]$row.ProcessId
        $identity = Get-CreationKey $row.CreationDate
        if ($Known.ContainsKey($id) -and $Known[$id] -eq $identity) { $selected[$id] = $row }
    }
    do {
        $added = $false
        foreach ($row in $Rows) {
            $id = [int]$row.ProcessId
            if (-not $selected.ContainsKey($id) -and $selected.ContainsKey([int]$row.ParentProcessId)) {
                # 子进程不能早于当前父进程，避免父 PID 被复用后误关联旧进程。
                if ($row.CreationDate -lt $selected[[int]$row.ParentProcessId].CreationDate) { continue }
                $selected[$id] = $row; $Known[$id] = Get-CreationKey $row.CreationDate; $added = $true
            }
        }
    } while ($added)
    @($selected.Values)
}
if ($LibraryOnly) { return }

$root = Get-CimInstance Win32_Process -Filter "ProcessId = $RootProcessId"
if (-not $root) { throw "Root process $RootProcessId is not running" }
$known = @{ $RootProcessId = (Get-CreationKey $root.CreationDate) }
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$output = Join-Path $OutputDirectory ("tree-{0}-{1}-{2}-{3}.jsonl" -f $RootProcessId, $PinCount, $Stage, [guid]::NewGuid().ToString('N'))
$watch = [Diagnostics.Stopwatch]::StartNew()
while ($watch.Elapsed.TotalSeconds -lt $Seconds) {
    $sampleStarted = $watch.Elapsed.TotalMilliseconds
    $rows = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, CreationDate, Name)
    $tree = @(Select-ProcessTree -Rows $rows -RootId $RootProcessId -Known $known)
    $privateSets = @{}
    # 此计数器可能不可用；缺失时记录 null，不能把未知占用写成零。
    try { foreach ($entry in Get-CimInstance Win32_PerfFormattedData_PerfProc_Process) { $privateSets[[int]$entry.IDProcess] = $entry.WorkingSetPrivate } } catch { }
    $samples = foreach ($row in $tree) {
        try {
            $process = Get-Process -Id $row.ProcessId -ErrorAction Stop
            # 拓扑采样后进程也可能退出并复用 PID，再次核对创建时间。
            if ([Math]::Abs(($process.StartTime.ToUniversalTime() - $row.CreationDate.ToUniversalTime()).TotalMilliseconds) -gt 1) { continue }
            [pscustomobject]@{ pid = $process.Id; role = $(if ($process.Id -eq $RootProcessId) { 'main' } elseif ($row.Name -eq 'msedgewebview2.exe') { 'webview2' } else { 'child' });
                privateBytes = $process.PrivateMemorySize64; workingSetBytes = $process.WorkingSet64;
                privateWorkingSetBytes = $(if ($privateSets.ContainsKey($process.Id)) { $privateSets[$process.Id] } else { $null }) }
        } catch { }
    }
    $samples = @($samples)
    $privateSetComplete = @($samples | Where-Object { $null -eq $_.privateWorkingSetBytes }).Count -eq 0
    [ordered]@{ elapsedMs = [Math]::Round($watch.Elapsed.TotalMilliseconds); sampleDurationMs = [Math]::Round($watch.Elapsed.TotalMilliseconds - $sampleStarted); pinCountLabel = $PinCount; stage = $Stage;
        rootAlive = @($samples | Where-Object { $_.pid -eq $RootProcessId }).Count -gt 0;
        processes = $samples; processCount = $samples.Count;
        privateBytes = ($samples | Measure-Object privateBytes -Sum).Sum;
        workingSetBytes = ($samples | Measure-Object workingSetBytes -Sum).Sum;
        privateWorkingSetBytes = $(if ($privateSetComplete) { ($samples | Measure-Object privateWorkingSetBytes -Sum).Sum } else { $null });
        workingSetIncludesSharedPages = $true } | ConvertTo-Json -Depth 5 -Compress | Add-Content -LiteralPath $output -Encoding utf8
    Start-Sleep -Milliseconds 500
}
Write-Output $output
