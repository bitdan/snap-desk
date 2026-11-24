# 由完整场景与窗口探针的结果生成 README 用的性能图（SVG，跟随系统浅色 / 深色），中英文各一张。
# 用法：./docs/performance/chart.ps1 -RunDir ./docs/performance/results/<时间戳>
param(
    [Parameter(Mandatory)][string]$RunDir,
    [string]$OutDir = (Join-Path $PSScriptRoot '..\assets')
)
$ErrorActionPreference = 'Stop'
$run = Get-Content (Join-Path $RunDir 'run.json') -Raw | ConvertFrom-Json
$rows = @(Import-Csv (Join-Path $RunDir 'samples.csv') | Where-Object { [int]$_.pid -eq [int]$run.pid } |
    ForEach-Object { [pscustomobject]@{ t = [double]$_.elapsed; mb = [double]$_.privateWorkingSetMB; phase = $_.phase } } | Sort-Object t)
$inv = [Globalization.CultureInfo]::InvariantCulture
function F([double]$v) { $v.ToString('0.#', $inv) }
function Esc([string]$s) { [Security.SecurityElement]::Escape($s) }

# 图上的指标与测试结果一致：常驻取 idle-start 中位数，结束空闲取 idle-released 中位数。
function Median([double[]]$v) { $s = $v | Sort-Object; $m = [int][Math]::Floor($s.Count / 2); if ($s.Count % 2) { $s[$m] } else { ($s[$m - 1] + $s[$m]) / 2 } }
$idleStart = [Math]::Round((Median ($rows | Where-Object phase -eq 'idle-start').mb))
$idleEnd = [Math]::Round((Median ($rows | Where-Object phase -eq 'idle-released').mb))
# 响应耗时取自应用日志，口径同 summarize.ps1：浮层为 gpui.action → gpui.overlay_shown，OCR 为引擎自报 duration_ms，
# 翻译为 gpui.ocr_finished → gpui.translation_finished（DeepL 往返 + 排版）。
$log = Get-Content (Join-Path $RunDir 'app.log')
function LogTime($line) { [DateTimeOffset]::Parse($line.Substring(0, 29)) }
$overlay = [Collections.Generic.List[double]]::new(); $ocr = [Collections.Generic.List[double]]::new(); $translate = [Collections.Generic.List[double]]::new()
$lastAction = $null; $lastOcr = $null
foreach ($line in $log) {
    if ($line -match 'event=gpui\.action action=(capture|ocr) ') { $lastAction = LogTime $line }
    elseif ($line -match 'event=gpui\.overlay_shown' -and $lastAction) { $overlay.Add(((LogTime $line) - $lastAction).TotalMilliseconds); $lastAction = $null }
    if ($line -match 'event=ocr\.finished .*engine=wechat-ocr .*duration_ms=(\d+)') { $ocr.Add([double]$Matches[1]) }
    if ($line -match 'event=gpui\.ocr_finished') { $lastOcr = LogTime $line }
    elseif ($line -match 'event=gpui\.translation_finished' -and $lastOcr) { $translate.Add(((LogTime $line) - $lastOcr).TotalMilliseconds); $lastOcr = $null }
}
function Sec([double]$ms) { ($ms / 1000).ToString('0.00', $inv) }
$overlaySec = Sec (Median $overlay); $ocrSec = Sec (Median $ocr); $translateSec = Sec (Median $translate)

function Span($phases) { $p = @($rows | Where-Object { $phases -contains $_.phase }); @($p[0].t, $p[-1].t) }

$work = @('capture-overlay', 'after-capture', 'clipboard-history', 'after-history', 'ocr-wechat-cold', 'ocr-wechat-warm', 'translate-deepl')
$text = @{
    zh = @{
        title = '实测性能'; env = 'Windows 10 · i7-7700 · 集成显卡 · 1080p'
        tiles = @(
            @($idleStart, 'MB', '托盘常驻内存'), @($overlaySec, '秒', '截图浮层弹出'),
            @($ocrSec, '秒', '离线识别一屏文字'), @('≈0', 'MB', '反复使用的内存增长'))
        chart = '连续使用 13 分钟的内存（任务管理器“内存”列）'; unit = '分钟'
        bands = @(@('截图 · 剪贴板 · 识别 · 翻译 ×11', $work),
            @('混合循环 ×10', @('leak-cycle')))
        start = "启动 $idleStart MB"; end = "空闲 $idleEnd MB"
        speed = '响应耗时（每个点为一次操作，竖线为中位数）'; median = '中位'; sec = '秒'; times = '次'
        rows = @(@('截图浮层弹出', $overlay), @('离线识字（WeChat OCR）', $ocr), @('DeepL 在线翻译（含网络）', $translate))
        note = 'WeChat OCR 在独立进程中按需启动，空闲 5 分钟自动退出，不计入上图。'
    }
    en = @{
        title = 'Measured performance'; env = 'Windows 10 · i7-7700 · integrated GPU · 1080p'
        tiles = @(
            @($idleStart, 'MB', 'Memory in the tray'), @($overlaySec, 's', 'Capture overlay opens'),
            @($ocrSec, 's', 'Offline OCR, one screen'), @('≈0', 'MB', 'Growth with repeated use'))
        chart = 'Memory over 13 minutes of continuous use (Task Manager “Memory”)'; unit = 'min'
        bands = @(@('Capture · OCR ×11', $work),
            @('Mixed loop ×10', @('leak-cycle')))
        start = "Start $idleStart MB"; end = "Idle $idleEnd MB"
        speed = 'Response time (each dot is one run, bar is the median)'; median = 'median'; sec = 's'; times = 'runs'
        rows = @(@('Capture overlay opens', $overlay), @('Offline OCR (WeChat OCR)', $ocr), @('DeepL translation (network)', $translate))
        note = 'WeChat OCR runs in a separate process on demand and exits after 5 idle minutes; not included above.'
    }
}

$W = 960; $H = 880
$x0 = 80; $x1 = 900; $y0 = 300; $y1 = 510
$tMax = [Math]::Ceiling($rows[-1].t / 60) * 60; $mbMax = 120
function X([double]$t) { $x0 + ($x1 - $x0) * $t / $tMax }
function Y([double]$mb) { $y1 - ($y1 - $y0) * [Math]::Min($mb, $mbMax) / $mbMax }

foreach ($lang in 'zh', 'en') {
    $L = $text[$lang]
    $o = [Text.StringBuilder]::new()
    $null = $o.AppendLine(@"
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 $W $H" width="$W" height="$H" role="img" aria-label="$(Esc $L.title)">
<style>
  .bg { fill: #f8f7f2 } .ink { fill: #1b1b19 } .muted { fill: #62625c } .rule { stroke: #dcd9cf } .axis { stroke: #1b1b19 }
  .band { fill: #1b1b19; fill-opacity: .045 } .line { stroke: #3d52c9 } .dot { fill: #3d52c9; stroke: #f8f7f2 } .accent { stroke: #d2452d }
  .sample { fill: #3d52c9; fill-opacity: .35 } .median { fill: #1b1b19 }
  @media (prefers-color-scheme: dark) {
    .bg { fill: #171714 } .ink { fill: #efede4 } .muted { fill: #a3a197 } .rule { stroke: #34332d } .axis { stroke: #efede4 }
    .band { fill: #efede4; fill-opacity: .06 } .line { stroke: #8496f5 } .dot { fill: #8496f5; stroke: #171714 } .accent { stroke: #ee6a52 }
    .sample { fill: #8496f5; fill-opacity: .4 } .median { fill: #efede4 }
  }
  text { font-family: system-ui, -apple-system, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif }
  .serif { font-family: Georgia, "Noto Serif SC", "Songti SC", "SimSun", serif }
  .mono { font-family: ui-monospace, "Cascadia Mono", Consolas, monospace }
</style>
<rect class="bg" width="$W" height="$H" rx="12"/>
<text class="ink serif" x="40" y="56" font-size="28">$(Esc $L.title)</text>
<text class="muted mono" x="40" y="80" font-size="12">$(Esc $L.env)</text>
"@)
    # 四个指标：上方一条短色线，大号数字 + 单位，下方说明。
    $tw = 205; $gap = 15
    for ($i = 0; $i -lt 4; $i++) {
        $tile = $L.tiles[$i]; $tx = 40 + $i * ($tw + $gap)
        $null = $o.AppendLine("<line class=`"rule`" x1=`"$tx`" y1=`"112`" x2=`"$($tx + $tw)`" y2=`"112`" stroke-width=`"1`"/>")
        $null = $o.AppendLine("<line class=`"accent`" x1=`"$tx`" y1=`"112`" x2=`"$($tx + 28)`" y2=`"112`" stroke-width=`"3`"/>")
        $null = $o.AppendLine("<text class=`"ink serif`" x=`"$tx`" y=`"172`" font-size=`"46`">$(Esc $tile[0])<tspan class=`"muted`" font-size=`"18`" dx=`"6`">$(Esc $tile[1])</tspan></text>")
        $null = $o.AppendLine("<text class=`"muted`" x=`"$tx`" y=`"202`" font-size=`"14`">$(Esc $tile[2])</text>")
    }
    $null = $o.AppendLine("<text class=`"ink`" x=`"40`" y=`"256`" font-size=`"15`" font-weight=`"600`">$(Esc $L.chart)</text>")
    # 阶段底色与名称。
    foreach ($band in $L.bands) {
        $s = Span $band[1]; $bx = X $s[0]; $bw = (X $s[1]) - $bx
        $null = $o.AppendLine("<rect class=`"band`" x=`"$(F $bx)`" y=`"$y0`" width=`"$(F $bw)`" height=`"$($y1 - $y0)`"/>")
        $null = $o.AppendLine("<text class=`"muted mono`" x=`"$(F ($bx + 6))`" y=`"$($y0 - 8)`" font-size=`"11`">$(Esc $band[0])</text>")
    }
    # 网格与坐标轴。
    foreach ($mb in 0, 30, 60, 90, 120) {
        $gy = F (Y $mb)
        if ($mb) { $null = $o.AppendLine("<line class=`"rule`" x1=`"$x0`" y1=`"$gy`" x2=`"$x1`" y2=`"$gy`" stroke-width=`"1`"/>") }
        $null = $o.AppendLine("<text class=`"muted mono`" x=`"$($x0 - 10)`" y=`"$(F ((Y $mb) + 4))`" font-size=`"11`" text-anchor=`"end`">$mb</text>")
    }
    $null = $o.AppendLine("<text class=`"muted mono`" x=`"$($x0 - 10)`" y=`"$($y0 - 8)`" font-size=`"11`" text-anchor=`"end`">MB</text>")
    $null = $o.AppendLine("<line class=`"axis`" x1=`"$x0`" y1=`"$y1`" x2=`"$x1`" y2=`"$y1`" stroke-width=`"1`"/>")
    for ($m = 0; $m -lt $tMax / 60; $m++) {
        if ($m % 2) { continue }
        $null = $o.AppendLine("<text class=`"muted mono`" x=`"$(F (X ($m * 60)))`" y=`"$($y1 + 18)`" font-size=`"11`" text-anchor=`"middle`">$m</text>")
    }
    $null = $o.AppendLine("<text class=`"muted mono`" x=`"$x1`" y=`"$($y1 + 18)`" font-size=`"11`" text-anchor=`"end`">$(Esc $L.unit)</text>")
    # 曲线与首尾标注。
    $pts = ($rows | ForEach-Object { "$(F (X $_.t)),$(F (Y $_.mb))" }) -join ' '
    $null = $o.AppendLine("<polyline class=`"line`" points=`"$pts`" fill=`"none`" stroke-width=`"2`" stroke-linejoin=`"round`"/>")
    $first = $rows[0]; $last = $rows[-1]
    foreach ($p in @(@($first, $L.start, 'start', 10), @($last, $L.end, 'end', -10))) {
        $px = X $p[0].t; $py = Y $p[0].mb
        $null = $o.AppendLine("<circle class=`"dot`" cx=`"$(F $px)`" cy=`"$(F $py)`" r=`"4.5`" stroke-width=`"2`"/>")
        $null = $o.AppendLine("<text class=`"ink`" x=`"$(F ($px + $p[3]))`" y=`"$(F ($py - 12))`" font-size=`"13`" font-weight=`"600`" text-anchor=`"$($p[2])`">$(Esc $p[1])</text>")
    }
    # 响应耗时：每行一项操作，点为单次耗时，竖线为中位数，共用 0–3 秒横轴。
    $sx0 = 270; $sx1 = 900; $sMax = 3000
    function SX([double]$ms) { $sx0 + ($sx1 - $sx0) * [Math]::Min($ms, $sMax) / $sMax }
    $null = $o.AppendLine("<text class=`"ink`" x=`"40`" y=`"590`" font-size=`"15`" font-weight=`"600`">$(Esc $L.speed)</text>")
    $axisY = 810
    foreach ($tick in 0, 500, 1000, 1500, 2000, 2500, 3000) {
        $tx = F (SX $tick)
        if ($tick) { $null = $o.AppendLine("<line class=`"rule`" x1=`"$tx`" y1=`"612`" x2=`"$tx`" y2=`"$axisY`" stroke-width=`"1`"/>") }
        $null = $o.AppendLine("<text class=`"muted mono`" x=`"$tx`" y=`"$($axisY + 18)`" font-size=`"11`" text-anchor=`"middle`">$(($tick / 1000).ToString('0.#', $inv))</text>")
    }
    $null = $o.AppendLine("<text class=`"muted mono`" x=`"$($sx1 + 4)`" y=`"$($axisY + 18)`" font-size=`"11`">$(Esc $L.sec)</text>")
    $null = $o.AppendLine("<line class=`"axis`" x1=`"$sx0`" y1=`"$axisY`" x2=`"$sx1`" y2=`"$axisY`" stroke-width=`"1`"/>")
    for ($i = 0; $i -lt 3; $i++) {
        $label = $L.rows[$i][0]; $v = [double[]]$L.rows[$i][1]; $cy = 650 + $i * 60
        $med = Median $v; $mx = SX $med
        $null = $o.AppendLine("<text class=`"ink`" x=`"40`" y=`"$($cy + 1)`" font-size=`"14`">$(Esc $label)</text>")
        $null = $o.AppendLine("<text class=`"muted mono`" x=`"40`" y=`"$($cy + 18)`" font-size=`"11`">$($v.Count) $(Esc $L.times)</text>")
        foreach ($ms in $v) { $null = $o.AppendLine("<circle class=`"sample`" cx=`"$(F (SX $ms))`" cy=`"$cy`" r=`"4`"/>") }
        $null = $o.AppendLine("<rect class=`"median`" x=`"$(F ($mx - 1.5))`" y=`"$($cy - 11)`" width=`"3`" height=`"22`" rx=`"1.5`"/>")
        $null = $o.AppendLine("<text class=`"ink`" x=`"$(F $mx)`" y=`"$($cy - 17)`" font-size=`"12`" font-weight=`"600`" text-anchor=`"middle`">$(Esc $L.median) $(Sec $med) $(Esc $L.sec)</text>")
    }
    $null = $o.AppendLine("<text class=`"muted`" x=`"40`" y=`"$($H - 24)`" font-size=`"12`">$(Esc $L.note)</text>")
    $null = $o.AppendLine('</svg>')
    $name = if ($lang -eq 'zh') { 'performance.svg' } else { 'performance.en.svg' }
    $path = Join-Path $OutDir $name
    [IO.File]::WriteAllText($path, $o.ToString(), [Text.UTF8Encoding]::new($false))
    Write-Host $path
}
