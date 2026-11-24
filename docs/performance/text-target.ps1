# 固定位置显示一段英文文本，作为 OCR / 翻译的框选目标；窗口位置与 run-scenarios.ps1 的框选坐标对应。
param([int]$X = 160, [int]$Y = 120, [int]$Width = 1100, [int]$Height = 640)
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -Namespace Native -Name Dpi -MemberDefinition '[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();'
[Native.Dpi]::SetProcessDPIAware() | Out-Null
$text = @'
Snap Desk performance sample, 2026 edition.
Screenshots are captured from every monitor and frozen before the selection starts.
The recognition window reuses the same image for text recognition and translation.
WeChat OCR runs in a separate worker process that exits after five idle minutes.
DeepL receives only the recognized text, never the screenshot itself.
Translated lines are drawn over the original image at the same positions.
Memory usage is sampled for the main process and every child process.
Working set, private bytes, and shared GPU memory are reported separately.
Each scenario is repeated so that warm and cold runs can be compared.
The clipboard history keeps thumbnails instead of full images in memory.
Closing a window should return the memory to the idle baseline.
Integrated graphics allocate textures from shared system memory.
Results depend on the screen size, the scaling factor, and the drivers.
Numbers below were measured on a single 1920 x 1080 display at 100 percent.
'@
$form = [Windows.Forms.Form]@{
    FormBorderStyle = 'None'; StartPosition = 'Manual'; TopMost = $true; ShowInTaskbar = $false
    Location = [Drawing.Point]::new($X, $Y); Size = [Drawing.Size]::new($Width, $Height); BackColor = 'White'
}
$label = [Windows.Forms.Label]@{
    Dock = 'Fill'; Text = $text; ForeColor = 'Black'; Padding = [Windows.Forms.Padding]::new(24)
    Font = [Drawing.Font]::new('Segoe UI', 16)
}
$form.Controls.Add($label)
[Windows.Forms.Application]::Run($form)
