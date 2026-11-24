# 窗口开关探针：对每个场景重复“打开 → 关闭”，记录主进程私有内存、工作集、句柄、GDI/USER 对象与可见窗口数，
# 用第 3 次之后的平均增量判断是否泄漏（前两次包含一次性初始化）。会重启 Snap Desk；剪贴板贴图场景会改写系统剪贴板。
# 运行期间不要操作键盘鼠标；锁屏时无法截图与模拟输入。
param(
    [string]$Exe,
    [int]$Runs = 8,
    [string[]]$Scenarios = @('capture-cancel', 'ocr-window', 'text-translate', 'clipboard-history', 'clipboard-pin', 'capture-pin', 'shelf'),
    [string]$OutDir = (Join-Path $PSScriptRoot "results\probe-$(Get-Date -Format 'yyyyMMdd-HHmmss')")
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -Namespace Native -Name Probe -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, System.UIntPtr e);
[DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint f, System.UIntPtr e);
[DllImport("user32.dll", SetLastError = true)] public static extern System.IntPtr OpenInputDesktop(uint f, bool i, uint a);
[DllImport("user32.dll")] public static extern bool CloseDesktop(System.IntPtr d);
[DllImport("user32.dll")] public static extern uint GetGuiResources(System.IntPtr h, uint flags);
public delegate bool EnumProc(System.IntPtr h, System.IntPtr l);
[DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, System.IntPtr l);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr h, out uint pid);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(System.IntPtr h);
public static int VisibleWindows(uint pid) {
    int n = 0;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid && IsWindowVisible(h)) n++; return true; }, System.IntPtr.Zero);
    return n;
}
'@
[Native.Probe]::SetProcessDPIAware() | Out-Null
$desktop = [Native.Probe]::OpenInputDesktop(0, $false, 0x0100)
if ($desktop -eq [IntPtr]::Zero) { throw '当前处于锁屏或安全桌面，解锁后再运行' }
[Native.Probe]::CloseDesktop($desktop) | Out-Null

$running = Get-Process | Where-Object { $_.Path -and $_.Name -like 'snap-desk*' } | Select-Object -First 1
if (!$Exe) { if (!$running) { throw '未找到运行中的 Snap Desk，请用 -Exe 指定程序路径' }; $Exe = $running.Path }
New-Item -ItemType Directory -Force $OutDir | Out-Null
$log = Join-Path $env:LOCALAPPDATA 'io.snapdesk.app\logs\snap-desk-gpui.log'

function Send-Action([string]$action) { Start-Process $Exe -ArgumentList "--$action" -WindowStyle Hidden }
function Send-Escape { [Native.Probe]::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 40; [Native.Probe]::keybd_event(0x1B, 0, 2, [UIntPtr]::Zero) }
function Invoke-Drag {
    [Native.Probe]::SetCursorPos(170, 130) | Out-Null; Start-Sleep -Milliseconds 150
    [Native.Probe]::mouse_event(0x2, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 80
    foreach ($i in 1..20) { [Native.Probe]::SetCursorPos(170 + 1080 * $i / 20, 130 + 620 * $i / 20) | Out-Null; Start-Sleep -Milliseconds 15 }
    [Native.Probe]::mouse_event(0x4, 0, 0, 0, [UIntPtr]::Zero)
}
# 把框选目标区域的截图放进剪贴板，供“剪贴板贴图”使用（需要 STA 线程，pwsh 默认即是）。
function Set-ClipboardImage {
    $bmp = [Drawing.Bitmap]::new(1080, 620); $g = [Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen(170, 130, 0, 0, $bmp.Size); $g.Dispose()
    [Windows.Forms.Clipboard]::SetImage($bmp); $bmp.Dispose()
}
function Get-Mem {
    $p = Get-Process -Id $script:appPid
    # 专用工作集（任务管理器“内存”列）。
    $pws = (Get-Counter "\Process($($p.Name))\Working Set - Private" -ErrorAction SilentlyContinue).CounterSamples | Select-Object -First 1
    [pscustomobject]@{
        privateWsMB = if ($pws) { [Math]::Round($pws.CookedValue / 1MB, 1) } else { $null }
        privateMB = [Math]::Round($p.PrivateMemorySize64 / 1MB, 1); workingSetMB = [Math]::Round($p.WorkingSet64 / 1MB, 1)
        handles = $p.HandleCount; gdi = [Native.Probe]::GetGuiResources($p.Handle, 0); user = [Native.Probe]::GetGuiResources($p.Handle, 1)
        threads = $p.Threads.Count; windows = [Native.Probe]::VisibleWindows([uint32]$script:appPid)
    }
}
# 每个场景：一次完整的打开 → 关闭，结束时应回到无窗口状态。
$steps = @{
    'capture-cancel'    = { Send-Action 'capture'; Start-Sleep -Milliseconds 1200; Invoke-Drag; Start-Sleep -Milliseconds 800; Send-Escape; Start-Sleep -Milliseconds 300; Send-Escape }
    'ocr-window'        = { Send-Action 'ocr'; Start-Sleep -Milliseconds 1200; Invoke-Drag; Start-Sleep -Seconds 4; Send-Escape }
    'text-translate'    = { Send-Action 'translate'; Start-Sleep -Seconds 2; Send-Escape }
    'clipboard-history' = { Send-Action 'history'; Start-Sleep -Seconds 2; Send-Escape }
    'clipboard-pin'     = { Set-ClipboardImage; Start-Sleep -Milliseconds 500; Send-Action 'pin'; Start-Sleep -Seconds 2; Send-Escape }
    'capture-pin'       = { Send-Action 'capture'; Start-Sleep -Milliseconds 1200; Invoke-Drag; Start-Sleep -Milliseconds 800; Send-Action 'pin'; Start-Sleep -Seconds 2; Send-Escape }
    'shelf'             = { Send-Action 'shelf'; Start-Sleep -Seconds 2; Send-Action 'shelf' }
}

$target = Start-Process pwsh -ArgumentList '-NoProfile', '-File', (Join-Path $PSScriptRoot 'text-target.ps1') -WindowStyle Hidden -PassThru
if ($running) { Stop-Process -Id $running.Id -Force; Start-Sleep -Seconds 3 }
$logStart = (Get-Item $log).Length
$script:appPid = (Start-Process $Exe -PassThru).Id
$points = [Collections.Generic.List[object]]::new()
try {
    Start-Sleep -Seconds 8
    $m = Get-Mem; $m | Add-Member scenario 'startup'; $m | Add-Member run 0; $points.Add($m)
    foreach ($scenario in $Scenarios) {
        Write-Host "[$(Get-Date -Format HH:mm:ss)] $scenario"
        foreach ($n in 1..$Runs) {
            & $steps[$scenario]
            Start-Sleep -Seconds 2
            $m = Get-Mem; $m | Add-Member scenario $scenario; $m | Add-Member run $n; $points.Add($m)
        }
    }
} finally {
    Stop-Process -Id $target.Id -ErrorAction SilentlyContinue
    $points | Export-Csv (Join-Path $OutDir 'points.csv') -NoTypeInformation -Encoding utf8
    $fs = [IO.File]::Open($log, 'Open', 'Read', 'ReadWrite')
    try { $fs.Seek($logStart, 'Begin') | Out-Null; [IO.StreamReader]::new($fs).ReadToEnd() | Set-Content (Join-Path $OutDir 'app.log') } finally { $fs.Dispose() }
    # 汇总：第 3 次之后的平均增量（MiB / 次、个 / 次）；windows 为关闭后仍可见的窗口数。
    $summary = foreach ($group in $points | Where-Object run -gt 0 | Group-Object scenario) {
        $g = @($group.Group); $a = $g[1]; $b = $g[-1]; $k = [Math]::Max(1, $g.Count - 2)
        [pscustomobject]@{
            scenario = $group.Name; lastPrivateWs = $b.privateWsMB; firstPrivate = $g[0].privateMB; lastPrivate = $b.privateMB
            privatePerRun = [Math]::Round(($b.privateMB - $a.privateMB) / $k, 1); workingSetPerRun = [Math]::Round(($b.workingSetMB - $a.workingSetMB) / $k, 1)
            handlesPerRun = [Math]::Round(($b.handles - $a.handles) / $k, 1); gdiPerRun = [Math]::Round(($b.gdi - $a.gdi) / $k, 1)
            userPerRun = [Math]::Round(($b.user - $a.user) / $k, 1); windowsLeft = $b.windows
        }
    }
    $summary | Export-Csv (Join-Path $OutDir 'summary.csv') -NoTypeInformation -Encoding utf8
    $summary | Format-Table -AutoSize | Out-String -Width 220
    Write-Host "结果：$OutDir（Snap Desk 保持运行）"
}
