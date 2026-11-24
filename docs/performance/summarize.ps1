# 汇总 run-scenarios.ps1 的结果目录：按阶段统计进程树内存，并从应用日志提取识别 / 翻译耗时，输出 Markdown 表格。
param([Parameter(Mandatory)][string]$RunDir)
$ErrorActionPreference = 'Stop'
$run = Get-Content (Join-Path $RunDir 'run.json') -Raw | ConvertFrom-Json
$rows = Import-Csv (Join-Path $RunDir 'samples.csv')
foreach ($r in $rows) { foreach ($k in 'elapsed', 'workingSetMB', 'privateWorkingSetMB', 'privateMB', 'gpuSharedMB') { $r.$k = [double]$r.$k } }

function Get-Median([double[]]$v) { if (!$v.Count) { return $null }; $s = $v | Sort-Object; $m = [int][Math]::Floor($s.Count / 2); if ($s.Count % 2) { $s[$m] } else { ($s[$m - 1] + $s[$m]) / 2 } }
function Format-Mb($v) { if ($null -eq $v) { '—' } else { '{0:N0}' -f $v } }

# 同一时刻（elapsed）的所有进程合为一个样本。
$samples = $rows | Group-Object elapsed | ForEach-Object {
    $g = $_.Group
    $main = $g | Where-Object { [int]$_.pid -eq [int]$run.pid }
    $child = @($g | Where-Object { [int]$_.pid -ne [int]$run.pid })
    [pscustomobject]@{
        elapsed = $g[0].elapsed; phase = $g[0].phase
        mainWs = [double]$main.workingSetMB; mainPws = [double]$main.privateWorkingSetMB; mainPriv = [double]$main.privateMB; mainGpu = [double]$main.gpuSharedMB
        childWs = [double](($child | Measure-Object workingSetMB -Sum).Sum); childPriv = [double](($child | Measure-Object privateMB -Sum).Sum)
        children = ($child | ForEach-Object name | Sort-Object -Unique) -join ' '
    }
}
$order = [ordered]@{}
foreach ($s in $samples | Sort-Object elapsed) { if (!$order.Contains($s.phase)) { $order[$s.phase] = $true } }

'## 按阶段的内存（MiB；中位数 / 峰值）'
''
'| 阶段 | 样本 | 主进程专用工作集 | 主进程工作集 | 主进程私有 | 主进程 GPU 共享 | 子进程工作集 | 子进程私有 | 子进程 |'
'| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |'
foreach ($phase in $order.Keys) {
    $p = @($samples | Where-Object phase -eq $phase)
    $cells = foreach ($k in 'mainPws', 'mainWs', 'mainPriv', 'mainGpu', 'childWs', 'childPriv') {
        $v = [double[]]($p | ForEach-Object $k)
        "$(Format-Mb (Get-Median $v)) / $(Format-Mb ($v | Measure-Object -Maximum).Maximum)"
    }
    $names = ($p | ForEach-Object { $_.children -split ' ' } | Where-Object { $_ } | Sort-Object -Unique) -join ', '
    "| $phase | $($p.Count) | $($cells -join ' | ') | $names |"
}

# 日志耗时：WeChat OCR 用引擎自报的 duration_ms；翻译取 ocr_finished → translation_finished 的间隔（DeepL 往返 + 排版）。
$log = Get-Content (Join-Path $RunDir 'app.log')
function Get-Time($line) { [DateTimeOffset]::Parse($line.Substring(0, 29)) }
$ocr = @($log | Where-Object { $_ -match 'event=ocr\.finished .*engine=wechat-ocr .*duration_ms=(\d+)' } | ForEach-Object { [int]($_ -replace '.*duration_ms=(\d+).*', '$1') })
$lines = @($log | Where-Object { $_ -match 'event=ocr\.finished' } | ForEach-Object { [int]($_ -replace '.*lines=(\d+).*', '$1') })
$translate = @()
$lastOcr = $null
foreach ($line in $log) {
    if ($line -match 'event=gpui\.ocr_finished') { $lastOcr = Get-Time $line }
    elseif ($line -match 'event=gpui\.translation_finished' -and $lastOcr) { $translate += ((Get-Time $line) - $lastOcr).TotalMilliseconds; $lastOcr = $null }
}
$ready = @()
$lastAction = $null
foreach ($line in $log) {
    if ($line -match 'event=gpui\.action action=(capture|ocr) ') { $lastAction = Get-Time $line }
    elseif ($line -match 'event=gpui\.overlay_shown' -and $lastAction) { $ready += ((Get-Time $line) - $lastAction).TotalMilliseconds; $lastAction = $null }
}
''
'## 耗时（ms）'
''
'| 项目 | 次数 | 首次 | 中位数 | 最大 |'
'| --- | ---: | ---: | ---: | ---: |'
foreach ($item in @(@('截图浮层显示（动作 → overlay_shown）', $ready), @('WeChat OCR 识别（引擎自报）', $ocr), @('DeepL 翻译（OCR 完成 → 译文覆盖完成）', $translate))) {
    $v = [double[]]$item[1]
    if ($v.Count) { "| $($item[0]) | $($v.Count) | $('{0:N0}' -f $v[0]) | $('{0:N0}' -f (Get-Median $v)) | $('{0:N0}' -f ($v | Measure-Object -Maximum).Maximum) |" }
    else { "| $($item[0]) | 0 | — | — | — |" }
}
''
"识别行数：$(($lines | Sort-Object -Unique) -join ', ')；失败事件：$(@($log | Where-Object { $_ -match 'failed|ERROR' }).Count) 条。"
