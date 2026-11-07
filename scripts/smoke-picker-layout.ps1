#requires -Version 7.0
param(
    [string]$ProjectRoot = (Split-Path $PSScriptRoot -Parent),
    [string]$Executable = 'target\debug\game-helper.exe',
    [string]$ScreenshotDirectory = 'docs\assets',
    [ValidatePattern('^[A-Za-z0-9_-]+$')][string]$ScreenshotPrefix = 'native-picker'
)
# Owns an isolated sandbox, picker and --probe; never starts playback.
# Run after the parent build is ready. Screenshots use GDI CopyFromScreen.
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class PickerLayoutNative {
 [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
 [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
 [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd,out Rect rect);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd,out Rect rect);
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hwnd,ref Point point);
 [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hwnd,int x,int y,int width,int height,bool repaint);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern bool SetWindowText(IntPtr hwnd,string text);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);
}
'@
[PickerLayoutNative]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
$ProjectRoot = [IO.Path]::GetFullPath($ProjectRoot)
$exe = if ([IO.Path]::IsPathRooted($Executable)) { $Executable } else { Join-Path $ProjectRoot $Executable }
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Build the native executable first: $exe" }
$output = if ([IO.Path]::IsPathRooted($ScreenshotDirectory)) { [IO.Path]::GetFullPath($ScreenshotDirectory) } else { [IO.Path]::GetFullPath((Join-Path $ProjectRoot $ScreenshotDirectory)) }
$token = [guid]::NewGuid().ToString('N')
$appRoot = Join-Path $ProjectRoot ('.tools\picker-layout-' + $token)
[IO.Directory]::CreateDirectory((Join-Path $appRoot 'routes')) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $appRoot 'ahk')) | Out-Null
[IO.File]::WriteAllText((Join-Path $appRoot 'ahk\daemon.ahk'), '; Inert picker layout fixture. Never executed.', [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText((Join-Path $appRoot 'config.json'), '{"routesDir":"routes","autoStartDaemon":false,"autoReloadOnSave":false,"hotkeys":{"stopAll":"F12","bindings":[]}}', [Text.UTF8Encoding]::new($false))
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()
$picker = $null
$probe = $null
$root = $null
$clipboardBefore = $null
$restoreClipboard = $false
$scope = [Windows.Automation.TreeScope]::Descendants
function Wait-Until([string]$Description, [scriptblock]$Predicate, [int]$TimeoutMs = 8000) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    do {
        if (& $Predicate) { return }
        Start-Sleep -Milliseconds 100
    } while ($watch.ElapsedMilliseconds -lt $TimeoutMs)
    throw "Timed out: $Description"
}
function Wait-Window($Process) {
    Wait-Until 'owned native window' {
        $Process.Refresh()
        if ($Process.HasExited) { throw 'Owned layout process exited before creating a window' }
        $Process.MainWindowHandle -ne [IntPtr]::Zero
    }
    Start-Sleep -Milliseconds 700
    $Process.Refresh()
    if ($Process.HasExited -or $Process.MainWindowHandle -eq [IntPtr]::Zero) { throw 'Owned process has no native window' }
}
function Node([string]$Id) {
    if ($root.Current.AutomationId -eq $Id) { return $root }
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, $Id)
    $root.FindFirst($scope, $condition)
}
function Visible([string]$Id) {
    $node = Node $Id
    $null -ne $node -and -not $node.Current.IsOffscreen -and -not $node.Current.BoundingRectangle.IsEmpty -and $node.Current.BoundingRectangle.Width -gt 0 -and $node.Current.BoundingRectangle.Height -gt 0
}
function Invoke-Id([string]$Id) {
    [PickerLayoutNative]::SetForegroundWindow($picker.MainWindowHandle) | Out-Null
    Start-Sleep -Milliseconds 100
    Wait-Until "enabled control $Id" { $node = Node $Id; $null -ne $node -and $node.Current.IsEnabled }
    (Node $Id).GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 200
}
function Set-Value([string]$Id, [string]$Text) {
    Wait-Until "input $Id" { $null -ne (Node $Id) }
    (Node $Id).GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($Text)
    Start-Sleep -Milliseconds 150
}
function Assert-Bounds([string[]]$Ids) {
    $bounds = $root.Current.BoundingRectangle
    foreach ($id in $Ids) {
        if (-not (Visible $id)) { throw "Critical control hidden: $id" }
        $rect = (Node $id).Current.BoundingRectangle
        if ($rect.Left -lt ($bounds.Left - 1) -or $rect.Right -gt ($bounds.Right + 1) -or $rect.Top -lt ($bounds.Top - 1) -or $rect.Bottom -gt ($bounds.Bottom + 1)) { throw "Control outside picker: $id $rect; window $bounds" }
    }
}
function Capture-Hotkey([int]$Id, [string]$Name) {
    [PickerLayoutNative]::SetForegroundWindow($probe.MainWindowHandle) | Out-Null
    Wait-Until 'owned probe foreground for stress capture' { [PickerLayoutNative]::GetForegroundWindow() -eq $probe.MainWindowHandle }
    $point = [PickerLayoutNative+Point]::new()
    $point.X = 100 + (($Id * 13) % 250)
    $point.Y = 90 + (($Id * 17) % 150)
    if (-not [PickerLayoutNative]::ClientToScreen($probe.MainWindowHandle, [ref]$point)) { throw 'Cannot locate dedicated stress probe client' }
    if (-not [PickerLayoutNative]::SetCursorPos($point.X, $point.Y)) { throw 'Cannot position cursor on dedicated stress probe' }
    try {
        foreach ($key in @(0x11, 0x12, 0x50)) {
            [PickerLayoutNative]::keybd_event($key, 0, 0, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 40
        }
    } finally {
        foreach ($key in @(0x50, 0x12, 0x11)) { [PickerLayoutNative]::keybd_event($key, 0, 2, [UIntPtr]::Zero) }
    }
    Wait-Until "stress point $Id captured" { $null -ne (Node ('point-name-' + $Id)) }
    Set-Value ('point-name-' + $Id) $Name
}
function Assert-NewestPoint([int]$Id) {
    $rowIds = @('point-name-' + $Id; 'copy-coordinate-' + $Id; 'copy-point-' + $Id; 'remove-point-' + $Id)
    Assert-Bounds ($rowIds + @('picker-status', 'clear-picker'))
    $viewport = (Node 'picker-points').Current.BoundingRectangle
    foreach ($control in $rowIds) {
        $rect = (Node $control).Current.BoundingRectangle
        if ($rect.Top -lt ($viewport.Top - 1) -or $rect.Bottom -gt ($viewport.Bottom + 1) -or $rect.Left -lt ($viewport.Left - 1) -or $rect.Right -gt ($viewport.Right + 1)) { throw "Newest point control $control is clipped by the scrolling list: $rect; viewport $viewport" }
    }
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Edit)
    $count = @($root.FindAll($scope, $condition) | Where-Object { $_.Current.AutomationId -match '^point-name-\d+$' }).Count
    if ($count -ne $Id) { throw "Stress capture expected $Id rows, got $count" }
    Assert-CopiedCoordinates $Id
}
function Assert-CopiedCoordinates([int]$Id) {
    $client = [PickerLayoutNative+Rect]::new()
    if (-not [PickerLayoutNative]::GetClientRect($probe.MainWindowHandle, [ref]$client)) { throw 'Cannot read dedicated layout probe client bounds' }
    $expected = @(((100.0 + (($Id * 13) % 250)) / ($client.Right - 1)), ((90.0 + (($Id * 17) % 150)) / ($client.Bottom - 1)))
    $copies = @()
    foreach ($control in @('copy-coordinate-' + $Id; 'copy-point-' + $Id)) {
        $sentinel = 'picker-layout-clipboard-' + [guid]::NewGuid().ToString('N')
        Set-Clipboard -Value $sentinel
        Invoke-Id $control
        Wait-Until "layout coordinate JSON copied by $control" { ([string](Get-Clipboard -Raw)) -cne $sentinel }
        $text = [string](Get-Clipboard -Raw)
        if ($text -cnotmatch '^\[([01]\.\d{6}), ([01]\.\d{6})\]$') { throw "Layout copy did not provide six-decimal coordinate JSON: $text" }
        $pair = ConvertFrom-Json -InputObject $text -NoEnumerate
        if ($pair -isnot [array] -or $pair.Count -ne 2) { throw 'Layout copy did not provide a coordinate array' }
        for ($i = 0; $i -lt 2; $i++) {
            if ([Math]::Abs($pair[$i] - $expected[$i]) -gt 0.000001) { throw "Layout copy changed captured precision: $control" }
        }
        $copies += $text
    }
    if ($copies[0] -cne $copies[1]) { throw 'Layout coordinate click and copy button disagree' }
}
function Resize-Picker([int]$Width, [int]$Height) {
    $scale = [PickerLayoutNative]::GetDpiForWindow($picker.MainWindowHandle) / 96.0
    $client = [PickerLayoutNative+Rect]::new()
    $outer = [PickerLayoutNative+Rect]::new()
    if (-not [PickerLayoutNative]::GetClientRect($picker.MainWindowHandle, [ref]$client) -or -not [PickerLayoutNative]::GetWindowRect($picker.MainWindowHandle, [ref]$outer)) { throw 'Cannot read picker bounds' }
    $frameWidth = ($outer.Right - $outer.Left) - ($client.Right - $client.Left)
    $frameHeight = ($outer.Bottom - $outer.Top) - ($client.Bottom - $client.Top)
    if (-not [PickerLayoutNative]::MoveWindow($picker.MainWindowHandle, 40, 40, ([int][Math]::Round($Width * $scale) + $frameWidth), ([int][Math]::Round($Height * $scale) + $frameHeight), $true)) { throw 'Cannot resize picker' }
    Start-Sleep -Milliseconds 400
    $actual = [PickerLayoutNative+Rect]::new()
    [PickerLayoutNative]::GetClientRect($picker.MainWindowHandle, [ref]$actual) | Out-Null
    if ([Math]::Abs(($actual.Right - $actual.Left) / $scale - $Width) -gt 1 -or [Math]::Abs(($actual.Bottom - $actual.Top) / $scale - $Height) -gt 1) { throw "Client size did not reach ${Width}x${Height}" }
}
function Capture([string]$Suffix) {
    [PickerLayoutNative]::SetForegroundWindow($picker.MainWindowHandle) | Out-Null
    Start-Sleep -Milliseconds 250
    $bounds = $root.Current.BoundingRectangle
    if ($bounds.Width -le 0 -or $bounds.Height -le 0) { throw 'Cannot capture an empty picker' }
    [IO.Directory]::CreateDirectory($output) | Out-Null
    $path = Join-Path $output ($ScreenshotPrefix + '-' + $Suffix + '.png')
    $bitmap = [Drawing.Bitmap]::new([int]$bounds.Width, [int]$bounds.Height)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen([int]$bounds.Left, [int]$bounds.Top, 0, 0, $bitmap.Size)
        $bitmap.Save($path, [Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
    Write-Output "Screenshot: $path"
}
try {
    Write-Output "Isolated picker layout sandbox: $appRoot"
    try { $clipboardBefore = Get-Clipboard -Raw; $restoreClipboard = $true } catch { }
    $probe = Start-Process -FilePath $exe -ArgumentList '--probe' -PassThru
    $owned.Add($probe)
    Wait-Window $probe
    $targetTitle = '北风港口 · 临时采点目标 · 资源采集与仓库巡查窗口'
    [PickerLayoutNative]::SetWindowText($probe.MainWindowHandle, $targetTitle) | Out-Null
    [PickerLayoutNative]::MoveWindow($probe.MainWindowHandle, 80, 80, 500, 400, $true) | Out-Null
    $picker = Start-Process -FilePath $exe -ArgumentList @('--picker', '--app-root', ('"' + $appRoot + '"')) -PassThru
    $owned.Add($picker)
    Wait-Window $picker
    $root = [Windows.Automation.AutomationElement]::FromHandle($picker.MainWindowHandle)
    Wait-Until 'native picker title' { $root.Current.Name -eq '取点工具' }
    # Crosshair always targets the owned probe, even if its list row is offscreen.
    Invoke-Id 'crosshair'
    Wait-Until 'selection countdown' { $null -ne (Node 'picker-countdown') }
    [PickerLayoutNative]::SetForegroundWindow($probe.MainWindowHandle) | Out-Null
    $position = [PickerLayoutNative+Point]::new()
    $position.X = 100; $position.Y = 100
    if (-not [PickerLayoutNative]::ClientToScreen($probe.MainWindowHandle, [ref]$position)) { throw 'Cannot locate layout probe client area' }
    [PickerLayoutNative]::SetCursorPos($position.X, $position.Y) | Out-Null
    Wait-Until 'owned probe selected' { $node = Node 'target-summary'; $null -ne $node -and $node.Current.Name.Contains($targetTitle) } 6000
    Invoke-Id 'capture'
    [PickerLayoutNative]::SetForegroundWindow($probe.MainWindowHandle) | Out-Null
    [PickerLayoutNative]::SetCursorPos($position.X, $position.Y) | Out-Null
    Wait-Until 'layout sample point' { $null -ne (Node 'point-name-1') } 6000
    Set-Value 'point-name-1' '起点 · 港口入口'
    # Only the owned demo title may appear in screenshots intended for docs.
    Set-Value 'picker-search' $targetTitle
    Invoke-Id 'hotkey'
    Wait-Until 'stress hotkey enabled' { (Node 'hotkey').Current.Name -eq '释放 Ctrl+Alt+P' }
    $sampleNames = @('木桥南端', '海风码头', '鱼市入口', '旧仓库', '货运栈桥', '灯塔路口', '海岸木箱', '船厂门口', '港口集市', '石阶转角', '瞭望平台', '补给营地', '树林边缘', '山道岔路', '矿石采集点', '滨海小径', '返回港口')
    for ($id = 2; $id -le 18; $id++) { Capture-Hotkey $id ('采集点 · ' + $sampleNames[$id - 2]) }
    [PickerLayoutNative]::SetForegroundWindow($picker.MainWindowHandle) | Out-Null
    $critical = @('picker-root', 'picker-points', 'coordinates', 'picker-status', 'target-summary', 'screen-coordinate', 'client-coordinate', 'normalized-coordinate', 'hotkey', 'capture', 'toggle-targets', 'clear-picker')
    $sidebar = @('picker-targets', 'picker-search', 'refresh-windows', 'crosshair')
    Resize-Picker 1120 800
    Wait-Until 'default target sidebar visible' { Visible 'picker-search' }
    Assert-Bounds ($critical + $sidebar)
    Assert-NewestPoint 18
    $sidebarWidth = (Node 'picker-targets').Current.BoundingRectangle.Width / ([PickerLayoutNative]::GetDpiForWindow($picker.MainWindowHandle) / 96.0)
    # The list's content width can exclude the sidebar's horizontal padding.
    if ($sidebarWidth -gt 281 -or $sidebarWidth -lt 220) { throw "Target sidebar is not approximately 280 logical pixels: $sidebarWidth" }
    Capture 'default'
    Resize-Picker 1000 800
    Wait-Until 'sidebar visible at 1000px boundary' { Visible 'picker-search' }
    Assert-Bounds ($critical + $sidebar)
    Resize-Picker 999 800
    Wait-Until 'sidebar auto-hides below 1000px' { -not (Visible 'picker-search') }
    Assert-Bounds $critical
    Resize-Picker 900 640
    Wait-Until 'minimum target sidebar hidden' { -not (Visible 'picker-search') }
    Capture-Hotkey 19 '终点 · 返回港口'
    Invoke-Id 'hotkey'
    Wait-Until 'stress hotkey released' { (Node 'hotkey').Current.Name -eq '启用 Ctrl+Alt+P' }
    Assert-Bounds $critical
    Assert-NewestPoint 19
    Capture 'compact'
    Invoke-Id 'toggle-targets'
    Wait-Until 'compact target sidebar opens' { Visible 'picker-search' }
    Assert-Bounds @('picker-search', 'refresh-windows', 'toggle-targets', 'clear-picker', 'copy-coordinate-19', 'copy-point-19')
    Invoke-Id 'toggle-targets'
    Wait-Until 'compact target sidebar closes' { -not (Visible 'picker-search') }
    Resize-Picker 1120 800
    # A manual compact toggle may persist; reopen only if the control is hidden.
    if (-not (Visible 'picker-search')) { Invoke-Id 'toggle-targets' }
    Wait-Until 'default sidebar restored' { Visible 'picker-search' }
    Invoke-Id 'clear-picker'
    Wait-Until 'clear confirmation' { $null -ne (Node 'picker-confirmation') }
    Assert-Bounds @('picker-confirmation', 'confirm-clear', 'cancel-confirmation')
    Capture 'confirmation'
    Invoke-Id 'cancel-confirmation'
    Wait-Until 'clear confirmation cancelled' { $null -eq (Node 'picker-confirmation') }
    Write-Output ('Passed: 1120x800 default, 1000/999 sidebar boundary, 900x640 minimum, compact sidebar toggle, 19 Chinese named points with complete newest row and both JSON coordinate-copy entries in viewport, critical UIA bounds and confirmation. DPI: ' + [PickerLayoutNative]::GetDpiForWindow($picker.MainWindowHandle))
} finally {
    foreach ($key in @(0x50, 0x12, 0x11)) { [PickerLayoutNative]::keybd_event($key, 0, 2, [UIntPtr]::Zero) }
    foreach ($process in $owned) {
        $process.Refresh()
        if (-not $process.HasExited) {
            $process.CloseMainWindow() | Out-Null
            if (-not $process.WaitForExit(1000)) { $process.Kill(); $process.WaitForExit(5000) | Out-Null }
        }
        $process.Dispose()
    }
    if ($restoreClipboard) {
        try { if ($null -eq $clipboardBefore) { Set-Clipboard -Value '' } else { Set-Clipboard -Value $clipboardBefore } } catch { Write-Warning 'Could not restore clipboard text' }
    }
    Write-Output "Sandbox retained for inspection: $appRoot"
}
