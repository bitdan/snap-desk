# 串行执行内存场景：冷启动空闲 → 截图浮层 → 剪贴板历史 → WeChat OCR → 截图翻译（WeChat OCR + DeepL）
# → 空闲释放 → 多轮循环检查泄漏。会结束并重启正在运行的 Snap Desk，并模拟鼠标框选；运行期间不要操作鼠标键盘。
# 使用当前设置（OCR 选 WeChat OCR、翻译选 DeepL、开启译文覆盖）；截图翻译通过点击截图工具栏的“截图翻译”按钮触发，
# 按钮坐标随选区固定，先用 -Calibrate 截一张图确认后通过 -TranslateButton 传入。锁屏时无法截图与模拟输入，脚本会直接退出。
param(
    [string]$Exe,
    [switch]$Calibrate,
    [int[]]$TranslateButton,
    [string]$OutDir = (Join-Path $PSScriptRoot "results\$(Get-Date -Format 'yyyyMMdd-HHmmss')"),
    [int]$WarmRuns = 5,
    [int]$LeakCycles = 10,
    [int]$IdleReleaseSeconds = 330
)
$ErrorActionPreference = 'Stop'
Add-Type -Namespace Native -Name Input -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, System.UIntPtr e);
[DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint f, System.UIntPtr e);
[DllImport("user32.dll", SetLastError = true)] public static extern System.IntPtr OpenInputDesktop(uint f, bool i, uint a);
[DllImport("user32.dll")] public static extern bool CloseDesktop(System.IntPtr d);
'@
[Native.Input]::SetProcessDPIAware() | Out-Null
$desktop = [Native.Input]::OpenInputDesktop(0, $false, 0x0100)
if ($desktop -eq [IntPtr]::Zero) { throw '当前处于锁屏或安全桌面，无法截图与模拟输入；解锁后再运行' }
[Native.Input]::CloseDesktop($desktop) | Out-Null
if (!$Calibrate -and $TranslateButton.Count -ne 2) { throw '请先用 -Calibrate 确认“截图翻译”按钮位置，再以 -TranslateButton x,y 传入' }

$running = Get-Process | Where-Object { $_.Path -and $_.Name -like 'snap-desk*' } | Select-Object -First 1
if (!$Exe) { if (!$running) { throw '未找到运行中的 Snap Desk，请用 -Exe 指定程序路径' }; $Exe = $running.Path }
New-Item -ItemType Directory -Force $OutDir | Out-Null
$log = Join-Path $env:LOCALAPPDATA 'io.snapdesk.app\logs\snap-desk-gpui.log'
$phaseFile = Join-Path $OutDir 'phase.txt'

$settings = Get-Content (Join-Path $env:APPDATA 'io.snapdesk.app\settings.json') -Raw | ConvertFrom-Json
if ($settings.ocr.provider -ne 'wechat-ocr' -or $settings.ai.provider -ne 'deepl' -or $settings.screenshotAfter -ne 'none') {
    throw '需要设置：OCR 引擎 WeChat OCR、翻译服务 DeepL、截图后动作“无”'
}

# 采样脚本同时在读，写入冲突时稍后重试。
function Set-Phase([string]$name) {
    foreach ($i in 1..50) { try { [IO.File]::WriteAllText($phaseFile, $name); break } catch { Start-Sleep -Milliseconds 20 } }
    Write-Host "[$(Get-Date -Format HH:mm:ss)] $name" }
function Send-Action([string]$action) { Start-Process $Exe -ArgumentList "--$action" -WindowStyle Hidden }
function Send-Escape { [Native.Input]::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 40; [Native.Input]::keybd_event(0x1B, 0, 2, [UIntPtr]::Zero) }
function Invoke-Drag([int]$x1, [int]$y1, [int]$x2, [int]$y2) {
    [Native.Input]::SetCursorPos($x1, $y1) | Out-Null; Start-Sleep -Milliseconds 150
    [Native.Input]::mouse_event(0x2, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 80
    foreach ($i in 1..20) { [Native.Input]::SetCursorPos($x1 + ($x2 - $x1) * $i / 20, $y1 + ($y2 - $y1) * $i / 20) | Out-Null; Start-Sleep -Milliseconds 15 }
    [Native.Input]::mouse_event(0x4, 0, 0, 0, [UIntPtr]::Zero)
}
# 从 $script:logMark 起等日志出现指定事件；返回是否等到。
function Wait-Log([string]$pattern, [int]$timeoutSeconds = 30) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.Elapsed.TotalSeconds -lt $timeoutSeconds) {
        Start-Sleep -Milliseconds 200
        $fs = [IO.File]::Open($log, 'Open', 'Read', 'ReadWrite')
        try { $fs.Seek($script:logMark, 'Begin') | Out-Null; $tail = [IO.StreamReader]::new($fs).ReadToEnd() } finally { $fs.Dispose() }
        if ($tail -match $pattern) { $script:logMark = (Get-Item $log).Length; return $true }
    }
    Write-Warning "超时未等到 $pattern"; return $false
}
function Invoke-Click([int]$x, [int]$y) {
    [Native.Input]::SetCursorPos($x, $y) | Out-Null; Start-Sleep -Milliseconds 120
    [Native.Input]::mouse_event(0x2, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 60
    [Native.Input]::mouse_event(0x4, 0, 0, 0, [UIntPtr]::Zero)
}
function Save-Screen([string]$path) {
    Add-Type -AssemblyName System.Drawing
    $bmp = [Drawing.Bitmap]::new(1920, 1080); $g = [Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen(0, 0, 0, 0, $bmp.Size); $bmp.Save($path); $g.Dispose(); $bmp.Dispose()
}
# ocr：Alt+C 同款动作，框选完成即识别；translate：普通截图框选后点工具栏“截图翻译”。
function Invoke-Recognize([string]$kind, [string]$doneEvent, [string]$phase) {
    Set-Phase $phase
    $script:logMark = (Get-Item $log).Length
    Send-Action ($kind -eq 'ocr' ? 'ocr' : 'capture')
    Wait-Log 'gpui\.capture_ready|gpui\.overlay_shown' 10 | Out-Null
    Start-Sleep -Milliseconds 600
    Invoke-Drag 170 130 1250 750
    if ($kind -eq 'translate') { Start-Sleep -Milliseconds 500; Invoke-Click $TranslateButton[0] $TranslateButton[1] }
    Wait-Log $doneEvent 40 | Out-Null
    Start-Sleep -Seconds 3        # 结果窗口停留，观察打开状态的占用
    Send-Escape                   # 结果窗口获得焦点，Esc 关闭
    Start-Sleep -Seconds 3
}

# 1. 结束旧实例，启动框选目标与采样
if ($running) { Stop-Process -Id $running.Id -Force; Start-Sleep -Seconds 3 }
$target = Start-Process pwsh -ArgumentList '-NoProfile', '-File', (Join-Path $PSScriptRoot 'text-target.ps1') -WindowStyle Hidden -PassThru
$logStart = (Get-Item $log).Length
Set-Phase 'cold-start'
$app = Start-Process $Exe -PassThru
$appPid = $app.Id
if ($Calibrate) {
    # 截下框选后的工具栏，确认“截图翻译”按钮坐标；不采样。
    Start-Sleep -Seconds 5; $script:logMark = (Get-Item $log).Length
    Send-Action 'capture'; Wait-Log 'gpui\.overlay_shown' 10 | Out-Null; Start-Sleep -Milliseconds 600
    Invoke-Drag 170 130 1250 750; Start-Sleep -Seconds 1
    Save-Screen (Join-Path $OutDir 'calibrate.png'); Send-Escape; Start-Sleep -Milliseconds 500; Send-Escape
    Stop-Process -Id $target.Id -ErrorAction SilentlyContinue
    Write-Host "已保存：$(Join-Path $OutDir 'calibrate.png')"; return
}
$script:logMark = $logStart
$sampler = Start-Process pwsh -ArgumentList '-NoProfile', '-File', (Join-Path $PSScriptRoot 'sampler.ps1'), '-RootPid', $appPid, '-OutDir', $OutDir -WindowStyle Hidden -PassThru
@{ exe = $Exe; pid = $appPid; started = (Get-Date).ToString('o'); logStart = $logStart } | ConvertTo-Json | Set-Content (Join-Path $OutDir 'run.json')
try {
    Start-Sleep -Seconds 10; Set-Phase 'idle-start'; Start-Sleep -Seconds 50

    Set-Phase 'capture-overlay'; Send-Action 'capture'; Start-Sleep -Seconds 6; Send-Escape
    Set-Phase 'after-capture'; Start-Sleep -Seconds 10

    Set-Phase 'clipboard-history'; Send-Action 'history'; Start-Sleep -Seconds 6; Send-Escape
    Set-Phase 'after-history'; Start-Sleep -Seconds 10

    Invoke-Recognize 'ocr' 'gpui\.ocr_finished|gpui\.ocr_failed' 'ocr-wechat-cold'
    foreach ($i in 1..$WarmRuns) { Invoke-Recognize 'ocr' 'gpui\.ocr_finished|gpui\.ocr_failed' 'ocr-wechat-warm' }
    foreach ($i in 1..$WarmRuns) { Invoke-Recognize 'translate' 'gpui\.translation_finished|gpui\.translation_failed' 'translate-deepl' }
    Set-Phase 'idle-after-recognize'; Start-Sleep -Seconds 60

    foreach ($i in 1..$LeakCycles) {
        Invoke-Recognize 'translate' 'gpui\.translation_finished|gpui\.translation_failed' 'leak-cycle'
        Send-Action 'capture'; Start-Sleep -Seconds 2; Send-Escape; Start-Sleep -Seconds 2
        Send-Action 'history'; Start-Sleep -Seconds 2; Send-Escape; Start-Sleep -Seconds 2
    }
    Set-Phase 'idle-after-cycles'; Start-Sleep -Seconds 60
    # WeChat OCR 空闲 5 分钟断开连接，子进程退出
    Set-Phase 'idle-wechat-release'; $script:logMark = (Get-Item $log).Length
    Wait-Log 'ocr\.engine_unloaded' $IdleReleaseSeconds | Out-Null
    Set-Phase 'idle-released'; Start-Sleep -Seconds 30
} finally {
    New-Item (Join-Path $OutDir 'stop.txt') -Force | Out-Null
    $sampler.WaitForExit(15000) | Out-Null
    Stop-Process -Id $target.Id -ErrorAction SilentlyContinue
    # 重新启动一份，恢复测试前的托盘常驻状态
    Stop-Process -Id $appPid -Force -ErrorAction SilentlyContinue; Start-Sleep -Seconds 2
    Start-Process $Exe | Out-Null
    $fs = [IO.File]::Open($log, 'Open', 'Read', 'ReadWrite')
    try { $fs.Seek($logStart, 'Begin') | Out-Null; [IO.StreamReader]::new($fs).ReadToEnd() | Set-Content (Join-Path $OutDir 'app.log') } finally { $fs.Dispose() }
    Write-Host "完成：$OutDir"
}
