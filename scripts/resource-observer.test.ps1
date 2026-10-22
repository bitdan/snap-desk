# 无需 Pester、模型、编译或实际截图；验证发现顺序、退出残留和 PID 复用边界。
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'resource-observer.psm1') -Force
function New-SnapshotProcess([int]$Id, [int]$Parent, [int]$Birth, [string]$Name = 'child.exe') {
    [pscustomobject]@{ ProcessId = $Id; ParentProcessId = $Parent; CreationDate = [datetime]::new(2026, 1, 1).AddSeconds($Birth); Name = $Name }
}
function Assert-Ids($Actual, [int[]]$Expected, [string]$Case) {
    $actualIds = @($Actual | ForEach-Object { [int]$_.ProcessId } | Sort-Object)
    if (($actualIds -join ',') -ne (($Expected | Sort-Object) -join ',')) { throw "${Case}: unexpected IDs $actualIds" }
}
$root = New-SnapshotProcess 10 1 1 'snap-desk.exe'
$webview = New-SnapshotProcess 20 10 2 'msedgewebview2.exe'
$worker = New-SnapshotProcess 30 10 3 'snap-translator-worker.exe'
$renderer = New-SnapshotProcess 40 20 4 'msedgewebview2.exe'
$unrelated = New-SnapshotProcess 50 1 5 'msedgewebview2.exe'
$rootKey = Get-ProcessIdentity $root
$known = @{ $rootKey = $root }
Assert-Ids @(Select-AppProcessTree @($renderer, $unrelated, $worker, $webview, $root) $known) @(10, 20, 30, 40) 'unordered tree'
Assert-Ids @(Select-AppProcessTree @($renderer, $worker, $unrelated) $known) @(30, 40) 'surviving descendants'
$lateChild = New-SnapshotProcess 60 30 6
Assert-Ids @(Select-AppProcessTree @($lateChild, $worker) $known) @(30, 60) 'child after root exit'
$reusedRoot = New-SnapshotProcess 10 1 10
$wrongChild = New-SnapshotProcess 70 10 11
Assert-Ids @(Select-AppProcessTree @($reusedRoot, $wrongChild, $worker) $known) @(30) 'reused parent PID'
$reusedWorker = New-SnapshotProcess 30 1 12
Assert-Ids @(Select-AppProcessTree @($reusedWorker) $known) @() 'reused descendant PID'
$olderChild = New-SnapshotProcess 80 10 0
Assert-Ids @(Select-AppProcessTree @($olderChild, $root) $known) @(10) 'child predates parent'
Assert-Ids @(Select-AppProcessTree @() $known) @() 'all exited'
$unknownBirth = [pscustomobject]@{ ProcessId = 0; ParentProcessId = 0; CreationDate = $null; Name = 'System' }
Assert-Ids @(Select-AppProcessTree @($unknownBirth, $root) $known) @(10) 'unavailable creation time'
foreach ($case in @(@($root, 'rust-main'), @($webview, 'webview2'), @($worker, 'translation-worker'), @($lateChild, 'other-child'))) {
    if ((Get-AppProcessCategory $case[0] $rootKey) -ne $case[1]) { throw 'incorrect category' }
}
Write-Host 'PASS: process tree discovery, exit, PID reuse, creation order and categories'

# 使用合成 CIM 快照跑完整落盘路径：不启动应用，不读取真实进程，也不触发编译。
foreach ($process in @($root, $webview, $worker)) {
    $process | Add-Member WorkingSetSize 4096
    $process | Add-Member PrivatePageCount 2048
    $process | Add-Member KernelModeTime 10000000
    $process | Add-Member UserModeTime 10000000
    $process | Add-Member ThreadCount 2
    $process | Add-Member HandleCount 5
    $process | Add-Member ExecutablePath 'C:\test\snap-desk.exe'
}
$script:snapshotNumber = 0
function Get-CimInstance {
    param($ClassName, $Filter, $Property)
    if ($Filter) { return $root }
    $script:snapshotNumber++
    if ($script:snapshotNumber -eq 1) { return @($root, $webview, $worker) }
    if ($script:snapshotNumber -eq 2) { return @($worker) }
    return @()
}
$testDirectory = Join-Path (Split-Path $PSScriptRoot -Parent) ('src-tauri/target/resource-observer-tests/' + [guid]::NewGuid().ToString('N'))
try {
    & (Join-Path $PSScriptRoot 'observe-app-resources.ps1') -ProcessId 10 -DurationSeconds 3 -IntervalMilliseconds 200 -ExitGraceSeconds 1 -OutputDirectory $testDirectory -Label synthetic
    $run = Get-ChildItem -LiteralPath $testDirectory -Directory | Select-Object -First 1
    $totals = @(Import-Csv -LiteralPath (Join-Path $run.FullName 'totals.csv'))
    $first = $totals | Where-Object { $_.sample -eq '0' -and $_.category -eq 'total' }
    if ($first.processes -ne '3' -or $first.privateBytes -ne '6144' -or $first.workingSetBytes -ne '12288') { throw 'incorrect initial aggregate' }
    $survivor = $totals | Where-Object { $_.sample -eq '1' -and $_.category -eq 'translation-worker' }
    if ($survivor.processes -ne '1') { throw 'surviving worker was lost' }
    $last = @($totals | Where-Object { $_.category -eq 'total' })[-1]
    if ($last.processes -ne '0' -or $last.privateBytes -ne '0') { throw 'missing zero sample after exit' }
    $details = @(Get-Content -LiteralPath (Join-Path $run.FullName 'processes.jsonl') | ForEach-Object { $_ | ConvertFrom-Json })
    if ($details.Count -ne 4 -or $details[3].cpuPercent -ne 0) { throw 'incorrect detailed records or CPU delta' }
    $metadata = Get-Content -LiteralPath (Join-Path $run.FullName 'metadata.json') -Raw | ConvertFrom-Json
    if (!$metadata.rootExitObserved -or $metadata.samples -lt 3) { throw 'missing exit metadata' }
    Write-Host 'PASS: sampler output, CPU delta, totals, worker survival and exit zeros'
} finally {
    Remove-Item Function:Get-CimInstance
}
