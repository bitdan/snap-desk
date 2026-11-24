# 采样 Snap Desk（GPUI）主进程及其全部子进程的内存、CPU 与 GPU 显存；只读，不操作应用。
# 阶段名读自 phase.txt（由 run-scenarios.ps1 写入），出现 stop.txt 时结束。
param(
    [Parameter(Mandatory)][int]$RootPid,
    [Parameter(Mandatory)][string]$OutDir,
    [ValidateRange(200, 10000)][int]$IntervalMs = 500
)
$ErrorActionPreference = 'Stop'
$phaseFile = Join-Path $OutDir 'phase.txt'
$stopFile = Join-Path $OutDir 'stop.txt'
$csv = Join-Path $OutDir 'samples.csv'
$cpus = [Environment]::ProcessorCount

# 身份 = PID + 创建时间，防止 PID 复用后误认无关进程。
function Get-Identity($p) { "$($p.ProcessId):$($p.CreationDate.ToUniversalTime().Ticks)" }

$root = Get-CimInstance Win32_Process -Filter "ProcessId = $RootPid"
if (!$root) { throw "进程 $RootPid 不存在" }
$known = @{ (Get-Identity $root) = $root }
$prevCpu = @{}
$lastPhase = 'unlabelled'
$watch = [Diagnostics.Stopwatch]::StartNew()
$writer = [IO.StreamWriter]::new($csv, $false, [Text.UTF8Encoding]::new($false))
$writer.WriteLine('elapsed,phase,pid,name,workingSetMB,privateWorkingSetMB,privateMB,cpuPercent,threads,handles,gpuSharedMB,gpuDedicatedMB')
try {
    while (!(Test-Path $stopFile)) {
        $cycle = $watch.ElapsedMilliseconds
        $snapshot = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, CreationDate, Name, WorkingSetSize, PrivatePageCount, KernelModeTime, UserModeTime, ThreadCount, HandleCount |
            Where-Object { $null -ne $_.CreationDate })
        # 快照顺序不保证父进程在前，迭代到不再发现新的子进程。
        do {
            $changed = $false
            foreach ($p in $snapshot) {
                $id = Get-Identity $p
                if ($known.ContainsKey($id)) { continue }
                $parent = $known.Values | Where-Object { $_.ProcessId -eq $p.ParentProcessId -and $_.CreationDate -le $p.CreationDate } | Select-Object -First 1
                if ($parent) { $known[$id] = $p; $changed = $true }
            }
        } while ($changed)
        $tree = @($snapshot | Where-Object { $known.ContainsKey((Get-Identity $_)) })
        $gpu = @{}
        if ($tree.Count) {
            $paths = foreach ($p in $tree) {
                "\GPU Process Memory(pid_$($p.ProcessId)_*)\Shared Usage"
                "\GPU Process Memory(pid_$($p.ProcessId)_*)\Dedicated Usage"
            }
            # 专用工作集即任务管理器“内存”列；按进程名取实例，再用 ID Process 对回 PID。
            $paths += '\Process(snap-desk*)\Working Set - Private', '\Process(snap-desk*)\ID Process'
            try {
                $samples = (Get-Counter $paths -ErrorAction SilentlyContinue).CounterSamples
                $ids = @{}
                foreach ($s in $samples | Where-Object { $_.Path -like '*\id process' }) { $ids[$s.InstanceName] = [int]$s.CookedValue }
                foreach ($s in $samples | Where-Object { $_.Path -like '*\working set - private' }) {
                    if ($ids.ContainsKey($s.InstanceName)) { $gpu["$($ids[$s.InstanceName]):privateWs"] = $s.CookedValue }
                }
                foreach ($s in $samples) {
                    if ($s.Path -match 'pid_(\d+)_.*\\(shared|dedicated) usage$') {
                        $key = "$($Matches[1]):$($Matches[2])"
                        $gpu[$key] = [double]$gpu[$key] + $s.CookedValue
                    }
                }
            } catch {}
        }
        $elapsed = $watch.Elapsed.TotalSeconds
        $phase = try {
            $fs = [IO.File]::Open($phaseFile, 'Open', 'Read', 'ReadWrite')
            try { [IO.StreamReader]::new($fs).ReadToEnd().Trim() } finally { $fs.Dispose() }
        } catch { $lastPhase }
        if (!$phase) { $phase = $lastPhase }
        $lastPhase = $phase
        foreach ($p in $tree) {
            $id = Get-Identity $p
            $ticks = [double]$p.KernelModeTime + [double]$p.UserModeTime
            $cpu = ''
            if ($prevCpu.ContainsKey($id)) {
                $dt = $elapsed - $prevCpu[$id].t
                if ($dt -gt 0) { $cpu = [Math]::Round([Math]::Max(0, $ticks - $prevCpu[$id].ticks) / 1e7 / $dt / $cpus * 100, 2) }
            }
            $prevCpu[$id] = @{ t = $elapsed; ticks = $ticks }
            $privateWs = $gpu["$($p.ProcessId):privateWs"]
            $writer.WriteLine(('{0:F2},{1},{2},{3},{4:F1},{5},{6:F1},{7},{8},{9},{10:F1},{11:F1}' -f $elapsed, $phase, $p.ProcessId, $p.Name,
                ($p.WorkingSetSize / 1MB), ($null -eq $privateWs ? '' : ('{0:F1}' -f ($privateWs / 1MB))), ($p.PrivatePageCount / 1MB), $cpu, $p.ThreadCount, $p.HandleCount,
                ([double]$gpu["$($p.ProcessId):shared"] / 1MB), ([double]$gpu["$($p.ProcessId):dedicated"] / 1MB)))
        }
        $writer.Flush()
        $rest = $IntervalMs - ($watch.ElapsedMilliseconds - $cycle)
        if ($rest -gt 0) { Start-Sleep -Milliseconds $rest }
    }
} finally { $writer.Dispose() }
