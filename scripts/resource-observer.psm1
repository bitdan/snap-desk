# 进程树采样的纯数据逻辑；独立于 CIM，便于用合成快照验证 PID 复用和退出场景。
Set-StrictMode -Version Latest

function Get-ProcessIdentity {
    param([Parameter(Mandatory)]$Process)
    return "$($Process.ProcessId):$($Process.CreationDate.ToUniversalTime().Ticks)"
}

function Select-AppProcessTree {
    param(
        [Parameter(Mandatory)][AllowEmptyCollection()][object[]]$Snapshot,
        [Parameter(Mandatory)][hashtable]$Known
    )
    $byId = @{}
    # Windows 的系统/受保护进程可能没有创建时间，不属于可安全认领的应用子进程。
    $Snapshot = @($Snapshot | Where-Object { $null -ne $_.CreationDate })
    foreach ($item in $Snapshot) { $byId[[int]$item.ProcessId] = $item }
    # 一轮快照顺序不保证父进程在前，因此迭代到不再发现新子进程。
    do {
        $changed = $false
        foreach ($item in $Snapshot) {
            $identity = Get-ProcessIdentity $item
            if ($Known.ContainsKey($identity)) { continue }
            $parentId = [int]$item.ParentProcessId
            $parents = @($Known.Values | Where-Object {
                [int]$_.ProcessId -eq $parentId -and $_.CreationDate -le $item.CreationDate
            })
            foreach ($parent in $parents) {
                # 父进程退出后仍跟踪它的子进程；PID 被系统复用时不可认领无关进程。
                if ($byId.ContainsKey($parentId) -and
                    (Get-ProcessIdentity $byId[$parentId]) -ne (Get-ProcessIdentity $parent)) { continue }
                $Known[$identity] = $item
                $changed = $true
                break
            }
        }
    } while ($changed)
    foreach ($item in $Snapshot) {
        if ($Known.ContainsKey((Get-ProcessIdentity $item))) { $item }
    }
}

function Get-AppProcessCategory {
    param([Parameter(Mandatory)]$Process, [Parameter(Mandatory)][string]$RootIdentity)
    if ((Get-ProcessIdentity $Process) -eq $RootIdentity) { return 'rust-main' }
    if ($Process.Name -ieq 'msedgewebview2.exe') { return 'webview2' }
    if ($Process.Name -ieq 'snap-translator-worker.exe') { return 'translation-worker' }
    return 'other-child'
}

Export-ModuleMember -Function Get-ProcessIdentity, Select-AppProcessTree, Get-AppProcessCategory
