$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'measure-process-tree.ps1') -RootProcessId 10 -LibraryOnly
function Row($id, $parent, $created) { [pscustomobject]@{ ProcessId = $id; ParentProcessId = $parent; CreationDate = $created } }
$known = @{ 10 = (Get-CreationKey '2026-10-28T10:00:00') }
$rows = @( (Row 10 1 '2026-10-28T10:00:00'), (Row 11 10 '2026-10-28T10:00:01'), (Row 12 11 '2026-10-28T10:00:02'), (Row 99 1 '2026-10-28T09:00:00') )
$tree = @(Select-ProcessTree -Rows $rows -RootId 10 -Known $known)
if ($tree.Count -ne 3) { throw 'Did not collect complete descendant tree' }
$tree = @(Select-ProcessTree -Rows @($rows[2], $rows[3]) -RootId 10 -Known $known)
if ($tree.Count -ne 1 -or $tree[0].ProcessId -ne 12) { throw 'Lost known child after parent exit' }
$tree = @(Select-ProcessTree -Rows @((Row 10 1 '2026-10-28T11:00:00'), (Row 13 10 '2026-10-28T11:00:01')) -RootId 10 -Known $known)
if ($tree.Count -ne 0) { throw 'PID reuse associated an unrelated application' }
$tree = @(Select-ProcessTree -Rows @($rows[0], (Row 14 10 '2026-10-28T09:00:00')) -RootId 10 -Known $known)
if ($tree.Count -ne 1) { throw 'Child predating parent must be excluded' }
$tree = @(Select-ProcessTree -Rows @((Row 10 1 '2026-10-28T10:00:00.500')) -RootId 10 -Known $known)
if ($tree.Count -ne 0) { throw 'Subsecond PID reuse must be excluded' }
Write-Output 'Process tree topology tests passed (5 cases)'
