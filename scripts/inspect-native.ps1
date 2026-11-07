param(
    [Parameter(Mandatory = $true)][int]$TargetProcessId,
    [string]$OutputPath = '.tools/native-probe.png'
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding $false
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$output = [IO.Path]::GetFullPath((Join-Path $projectRoot $OutputPath))
if (-not $output.StartsWith($projectRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Output must stay inside the project.'
}
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Drawing
$process = Get-Process -Id $TargetProcessId
$handle = $process.MainWindowHandle
if ($handle -eq [IntPtr]::Zero) { throw 'Target process has no visible main window.' }
$element = [System.Windows.Automation.AutomationElement]::FromHandle($handle)
$bounds = $element.Current.BoundingRectangle
$nodes = $element.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
$controls = foreach ($node in $nodes) {
    $current = $node.Current
    [PSCustomObject]@{
        Name = $current.Name
        Type = $current.ControlType.ProgrammaticName
        Id = $current.AutomationId
        Bounds = $current.BoundingRectangle.ToString()
        Enabled = $current.IsEnabled
    }
}
$controls | ConvertTo-Json -Depth 3
if ($bounds.Width -le 0 -or $bounds.Height -le 0) { throw 'Window has empty bounds.' }
[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($output)) | Out-Null
$bitmap = New-Object System.Drawing.Bitmap ([int]$bounds.Width), ([int]$bounds.Height)
$graphics = [Drawing.Graphics]::FromImage($bitmap)
try {
    $graphics.CopyFromScreen([int]$bounds.X, [int]$bounds.Y, 0, 0, $bitmap.Size)
    $bitmap.Save($output, [Drawing.Imaging.ImageFormat]::Png)
} finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}
Write-Output "Screenshot: $output"
