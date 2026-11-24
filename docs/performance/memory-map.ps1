# 按类型统计进程已提交内存（私有 / 映像 / 映射），列出最大的私有区域与已加载模块数；只读。
param([Parameter(Mandatory)][int]$ProcessId, [int]$Top = 8)
Add-Type -Namespace Native -Name Vm -MemberDefinition @'
[StructLayout(LayoutKind.Sequential)] public struct MBI {
    public System.IntPtr BaseAddress; public System.IntPtr AllocationBase; public uint AllocationProtect; public ushort PartitionId;
    public System.IntPtr RegionSize; public uint State; public uint Protect; public uint Type; }
[DllImport("kernel32.dll")] public static extern System.IntPtr OpenProcess(uint access, bool inherit, int pid);
[DllImport("kernel32.dll")] public static extern bool CloseHandle(System.IntPtr h);
[DllImport("kernel32.dll")] public static extern System.IntPtr VirtualQueryEx(System.IntPtr h, System.IntPtr addr, out MBI info, System.IntPtr len);
'@
$h = [Native.Vm]::OpenProcess(0x0400 -bor 0x0010, $false, $ProcessId)
if ($h -eq [IntPtr]::Zero) { throw "无法打开进程 $ProcessId" }
$totals = @{ private = 0L; image = 0L; mapped = 0L }
$allocations = @{}
$addr = [IntPtr]::Zero
$info = New-Object Native.Vm+MBI
$size = [IntPtr][Runtime.InteropServices.Marshal]::SizeOf($info)
try {
    while ([Native.Vm]::VirtualQueryEx($h, $addr, [ref]$info, $size) -ne [IntPtr]::Zero) {
        $len = $info.RegionSize.ToInt64()
        if ($info.State -eq 0x1000) {   # MEM_COMMIT
            switch ($info.Type) {
                0x20000 { $totals.private += $len; $key = $info.AllocationBase.ToInt64(); $allocations[$key] = [long]$allocations[$key] + $len }
                0x1000000 { $totals.image += $len }
                0x40000 { $totals.mapped += $len }
            }
        }
        $next = $info.BaseAddress.ToInt64() + $len
        if ($next -le $addr.ToInt64() -or $next -ge 0x7FFFFFFFFFFF) { break }
        $addr = [IntPtr]$next
    }
} finally { [Native.Vm]::CloseHandle($h) | Out-Null }
$p = Get-Process -Id $ProcessId
[pscustomobject]@{
    privateCommitMB = [Math]::Round($totals.private / 1MB, 1); imageMB = [Math]::Round($totals.image / 1MB, 1); mappedMB = [Math]::Round($totals.mapped / 1MB, 1)
    workingSetMB = [Math]::Round($p.WorkingSet64 / 1MB, 1); privateBytesMB = [Math]::Round($p.PrivateMemorySize64 / 1MB, 1)
    modules = $p.Modules.Count; threads = $p.Threads.Count
    largestPrivateMB = ($allocations.Values | Sort-Object -Descending | Select-Object -First $Top | ForEach-Object { [Math]::Round($_ / 1MB, 1) }) -join ' '
}
