#requires -Version 7.0
param(
    [string]$ProjectRoot = (Split-Path $PSScriptRoot -Parent),
    [string]$Executable = 'target\debug\game-helper.exe'
)
# Run with pwsh (UTF-8), after the parent has built the approved native picker.
# This script owns its --probe windows and unique .tools sandbox. No playback.
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class PickerProbeNative {
 [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
 [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h,int x,int y,int w,int height,bool repaint);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern bool SetWindowText(IntPtr h,string text);
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref Point p);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out Rect r);
 [DllImport("user32.dll")] public static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);
}
'@
[PickerProbeNative]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
$ProjectRoot = [IO.Path]::GetFullPath($ProjectRoot)
$exe = if ([IO.Path]::IsPathRooted($Executable)) { $Executable } else { Join-Path $ProjectRoot $Executable }
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Build the native executable first: $exe" }
$token = [guid]::NewGuid().ToString('N')
$appRoot = Join-Path $ProjectRoot ('.tools\picker-smoke-' + $token)
$routes = Join-Path $appRoot 'routes'
[IO.Directory]::CreateDirectory($routes) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $appRoot 'ahk')) | Out-Null
[IO.File]::WriteAllText((Join-Path $appRoot 'ahk\daemon.ahk'), '; Inert picker smoke fixture. Never executed.', [Text.UTF8Encoding]::new($false))
$configText = '{"routesDir":"routes","autoStartDaemon":false,"autoReloadOnSave":false,"hotkeys":{"stopAll":"F12","bindings":[]}}'
[IO.File]::WriteAllText((Join-Path $appRoot 'config.json'), $configText, [Text.UTF8Encoding]::new($false))
$fixtureStem = 'picker-' + $token
$routeName = '原生取点验收路线'
$pointName = '起点·中文名称'
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()
$pickerProcess = $null
$targetProcess = $null
$root = $null
$scope = [Windows.Automation.TreeScope]::Descendants
$clipboardBefore = $null
$restoreClipboard = $false

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
        if ($Process.HasExited) { throw 'Owned test process exited before creating its window' }
        $Process.MainWindowHandle -ne [IntPtr]::Zero
    }
    # A debug executable can briefly report its console before the native HWND.
    Start-Sleep -Milliseconds 700
    $Process.Refresh()
    if ($Process.HasExited -or $Process.MainWindowHandle -eq [IntPtr]::Zero) { throw 'Owned process has no native window' }
}
function Node([string]$Id) {
    if ($root.Current.AutomationId -eq $Id) { return $root }
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, $Id)
    $root.FindFirst($scope, $condition)
}
function Need-Node([string]$Id) {
    Wait-Until "control $Id" { $null -ne (Node $Id) }
    Node $Id
}
function Invoke-Id([string]$Id) {
    [PickerProbeNative]::SetForegroundWindow($pickerProcess.MainWindowHandle) | Out-Null
    Wait-Until 'picker foreground for UIA invoke' { [PickerProbeNative]::GetForegroundWindow() -eq $pickerProcess.MainWindowHandle }
    Wait-Until "enabled control $Id" { $n = Node $Id; $null -ne $n -and $n.Current.IsEnabled }
    (Node $Id).GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 150
}
function Set-Value([string]$Id, [string]$Text) {
    $node = Need-Node $Id
    $node.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($Text)
    Wait-Until "input $Id updates" { (Get-Value $Id) -eq $Text }
    Start-Sleep -Milliseconds 100
}
function Get-Value([string]$Id) {
    $node = Node $Id
    if ($null -eq $node) { return $null }
    $node.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value
}
function Get-PointIds {
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Edit)
    foreach ($node in $root.FindAll($scope, $condition)) {
        if ($node.Current.AutomationId -match '^point-name-(\d+)$') { [int]$Matches[1] }
    }
}
function Wait-Points([int[]]$Ids) {
    $expected = (@($Ids | Sort-Object) -join ',')
    Wait-Until "point IDs [$expected]" { (@(Get-PointIds | Sort-Object) -join ',') -eq $expected }
}
function Assert-Points([int[]]$Ids) {
    $actual = @(Get-PointIds | Sort-Object) -join ','
    $expected = @($Ids | Sort-Object) -join ','
    if ($actual -ne $expected) { throw "Point IDs changed: expected [$expected], got [$actual]" }
}
function Press-Key([byte]$Key) {
    try { [PickerProbeNative]::keybd_event($Key, 0, 0, [UIntPtr]::Zero) }
    finally { [PickerProbeNative]::keybd_event($Key, 0, 2, [UIntPtr]::Zero) }
}
function Wait-Focus([string]$Id) {
    Wait-Until "keyboard focus on $Id" {
        $focused = [Windows.Automation.AutomationElement]::FocusedElement
        $null -ne $focused -and $focused.Current.AutomationId -eq $Id
    }
}
function Press-Tab([bool]$Backwards = $false) {
    try {
        if ($Backwards) { [PickerProbeNative]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero) }
        Press-Key 0x09
    } finally {
        [PickerProbeNative]::keybd_event(0x09, 0, 2, [UIntPtr]::Zero)
        [PickerProbeNative]::keybd_event(0x10, 0, 2, [UIntPtr]::Zero)
    }
}
function Escape-Picker {
    [PickerProbeNative]::SetForegroundWindow($pickerProcess.MainWindowHandle) | Out-Null
    Wait-Until 'picker foreground before Escape' { [PickerProbeNative]::GetForegroundWindow() -eq $pickerProcess.MainWindowHandle }
    Press-Key 0x1B
    Start-Sleep -Milliseconds 150
}
function Target-Point {
    $targetProcess.Refresh()
    if ($targetProcess.HasExited) { throw 'Cannot position cursor on an exited probe' }
    $position = [PickerProbeNative+Point]::new()
    $position.X = 100; $position.Y = 100
    if (-not [PickerProbeNative]::ClientToScreen($targetProcess.MainWindowHandle, [ref]$position)) { throw 'Cannot locate dedicated probe client area' }
    $position
}
function Focus-Probe {
    [PickerProbeNative]::SetForegroundWindow($targetProcess.MainWindowHandle) | Out-Null
    Wait-Until 'dedicated probe foreground (picker unfocused)' { [PickerProbeNative]::GetForegroundWindow() -eq $targetProcess.MainWindowHandle }
    $position = Target-Point
    # Cursor movement is restricted to the owned --probe client area.
    if (-not [PickerProbeNative]::SetCursorPos($position.X, $position.Y)) { throw 'Cannot move cursor to dedicated probe' }
    Start-Sleep -Milliseconds 100
}
function Hotkey-Capture {
    Focus-Probe
    try {
        foreach ($key in @(0x11, 0x12, 0x50)) {
            [PickerProbeNative]::keybd_event($key, 0, 0, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 40
        }
    } finally {
        foreach ($key in @(0x50, 0x12, 0x11)) { [PickerProbeNative]::keybd_event($key, 0, 2, [UIntPtr]::Zero) }
    }
}
function Start-Probe([string]$Title) {
    $process = Start-Process -FilePath $exe -ArgumentList '--probe' -PassThru
    $owned.Add($process)
    Wait-Window $process
    [PickerProbeNative]::SetWindowText($process.MainWindowHandle, $Title) | Out-Null
    [PickerProbeNative]::MoveWindow($process.MainWindowHandle, 80, 80, 500, 400, $true) | Out-Null
    $process
}
function Select-Probe([string]$Title) {
    Set-Value 'picker-search' ''
    Invoke-Id 'refresh-windows'
    $targetId = 'target-' + $targetProcess.MainWindowHandle.ToInt64()
    Wait-Until 'dedicated native target in list' { $null -ne (Node $targetId) }
    Set-Value 'picker-search' ('no-match-' + $token)
    Wait-Until 'search filters unrelated target' { $null -eq (Node $targetId) }
    Set-Value 'picker-search' $Title
    Wait-Until 'search finds exact dedicated target' { $null -ne (Node $targetId) }
    Invoke-Id $targetId
    # GPUI may expose an offscreen list row whose Invoke is a no-op.
    Start-Sleep -Milliseconds 350
    $summary = Node 'target-summary'
    if ($null -eq $summary -or -not $summary.Current.Name.Contains($Title)) {
        Invoke-Id 'crosshair'
        $null = Need-Node 'picker-countdown'
        Focus-Probe
        Wait-Until 'crosshair chooses dedicated probe' {
            $summary = Node 'target-summary'
            $null -ne $summary -and $summary.Current.Name.Contains($Title)
        } 6000
    }
    Set-Value 'picker-search' ('no-match-' + $token)
    if (-not (Need-Node 'target-summary').Current.Name.Contains($Title)) { throw 'Filtering cleared the selected target' }
    Set-Value 'picker-search' ''
}
function Capture-Delayed([int[]]$ExpectedIds) {
    Invoke-Id 'capture'
    $null = Need-Node 'picker-countdown'
    Focus-Probe
    Wait-Points $ExpectedIds
    Wait-Until 'capture countdown disappears' { $null -eq (Node 'picker-countdown') }
}
function Wait-Confirmation([string]$Action) {
    $null = Need-Node 'picker-confirmation'
    $null = Need-Node $Action
}
function Assert-NoConfirmation {
    Wait-Until 'confirmation dismissed' { $null -eq (Node 'picker-confirmation') }
}
function Read-Cache([string]$Stem) {
    [IO.File]::ReadAllText((Join-Path $routes ('_cache\' + $Stem + '.json'))) | ConvertFrom-Json
}
function Wait-Saved([string]$Stem, [int[]]$Ids, [string]$Name) {
    $cache = Join-Path $routes ('_cache\' + $Stem + '.json')
    $expected = @($Ids | Sort-Object) -join ','
    Wait-Until "YAML and cache save for $Stem" {
        if (-not (Test-Path -LiteralPath $cache -PathType Leaf) -or -not (Test-Path -LiteralPath (Join-Path $routes ($Stem + '.yaml')) -PathType Leaf)) { return $false }
        try {
            $route = Read-Cache $Stem
            $route.name -eq $Name -and (@($route.points | ForEach-Object { $_.id } | Sort-Object) -join ',') -eq $expected
        } catch { $false }
    }
    Wait-Until 'save completes' { $n = Node 'save-picker'; $null -ne $n -and $n.Current.IsEnabled }
}
function Assert-Contract($Route, [int]$PointId, [string]$Name, [double[]]$Expected) {
    $point = @($Route.points | Where-Object { $_.id -eq $PointId })
    if ($point.Count -ne 1 -or $point[0].name -ne $Name) { throw 'Picker cache lost stable point ID or Unicode name' }
    if ($point[0].PSObject.Properties.Name -contains 'pos') { throw 'Picker persisted obsolete point-level coordinates' }
    if ($point[0].operations.Count -ne 1 -or $point[0].operations[0].type -ne 'click') { throw 'Picker did not create one click operation' }
    $actual = $point[0].operations[0].pos
    if ([Math]::Abs($actual[0] - $Expected[0]) -gt 0.000001 -or [Math]::Abs($actual[1] - $Expected[1]) -gt 0.000001) { throw 'Unfocused capture lost normalized client precision' }
}

try {
    Write-Output "Isolated picker sandbox: $appRoot"
    try { $clipboardBefore = Get-Clipboard -Raw; $restoreClipboard = $true } catch { }
    $targetTitle = 'GH Picker Acceptance ' + $token
    $targetProcess = Start-Probe $targetTitle
    $pickerProcess = Start-Process -FilePath $exe -ArgumentList @('--picker', '--app-root', ('"' + $appRoot + '"')) -PassThru
    $owned.Add($pickerProcess)
    Wait-Window $pickerProcess
    $root = [Windows.Automation.AutomationElement]::FromHandle($pickerProcess.MainWindowHandle)
    Wait-Until 'native picker title' { $root.Current.Name -eq '取点工具' }
    foreach ($id in @('picker-root', 'picker-targets', 'picker-points', 'coordinates', 'picker-status', 'picker-search', 'picker-stem', 'picker-name', 'target-summary', 'screen-coordinate', 'client-coordinate', 'normalized-coordinate')) { $null = Need-Node $id }
    Select-Probe $targetTitle
    Set-Value 'picker-stem' $fixtureStem
    Set-Value 'picker-name' $routeName
    Invoke-Id 'hotkey'
    Wait-Until 'global hotkey enabled' { (Node 'hotkey').Current.Name -eq '释放 Ctrl+Alt+P' }
    Hotkey-Capture
    Wait-Points @(1)
    Set-Value 'point-name-1' $pointName
    $client = [PickerProbeNative+Rect]::new()
    if (-not [PickerProbeNative]::GetClientRect($targetProcess.MainWindowHandle, [ref]$client)) { throw 'Cannot read dedicated probe client bounds' }
    $expected = @((100.0 / ($client.Right - 1)), (100.0 / ($client.Bottom - 1)))
    Invoke-Id 'save-picker'
    Wait-Saved $fixtureStem @(1) $routeName
    $route = Read-Cache $fixtureStem
    Assert-Contract $route 1 $pointName $expected
    $yaml = [IO.File]::ReadAllText((Join-Path $routes ($fixtureStem + '.yaml')))
    if (-not $yaml.Contains($routeName) -or -not $yaml.Contains($pointName) -or $yaml -match '(?m)^  pos:') { throw 'Picker YAML lost Unicode or contains obsolete point coordinates' }
    if ($route.target.title -ne $targetTitle) { throw 'Saved target is not the owned probe' }

    Invoke-Id 'copy-point-1'
    Wait-Until 'copied point YAML on clipboard' { [string](Get-Clipboard -Raw) -match '(?m)^id: 1\r?$' }
    $copied = [string](Get-Clipboard -Raw)
    $copyPosition = [regex]::Match($copied, 'pos: \[([0-9.]+),\s*([0-9.]+)\]')
    if (-not $copied.Contains($pointName) -or $copied -notmatch '(?m)^operations:' -or $copied -notmatch 'type: click' -or -not $copyPosition.Success) { throw 'Copy did not provide named click-operation YAML' }
    $copyX = [double]::Parse($copyPosition.Groups[1].Value, [Globalization.CultureInfo]::InvariantCulture)
    $copyY = [double]::Parse($copyPosition.Groups[2].Value, [Globalization.CultureInfo]::InvariantCulture)
    if ([Math]::Abs($copyX - $expected[0]) -gt 0.000001 -or [Math]::Abs($copyY - $expected[1]) -gt 0.000001) { throw 'Clipboard coordinates differ from captured point' }

    Invoke-Id 'capture'
    $null = Need-Node 'picker-countdown'
    Invoke-Id 'cancel-countdown'
    Wait-Until 'button cancels countdown' { $null -eq (Node 'picker-countdown') }
    Focus-Probe
    Start-Sleep -Milliseconds 2600
    Assert-Points @(1)
    Invoke-Id 'capture'
    $null = Need-Node 'picker-countdown'
    Escape-Picker
    Wait-Until 'Escape cancels countdown' { $null -eq (Node 'picker-countdown') }
    Focus-Probe
    Start-Sleep -Milliseconds 2600
    Assert-Points @(1)
    Invoke-Id 'crosshair'
    $null = Need-Node 'picker-countdown'
    Invoke-Id 'cancel-countdown'
    Wait-Until 'crosshair countdown cancelled' { $null -eq (Node 'picker-countdown') }
    Focus-Probe
    Start-Sleep -Milliseconds 3400
    Assert-Points @(1)
    if (-not (Need-Node 'target-summary').Current.Name.Contains($targetTitle)) { throw 'Cancelled crosshair changed selected target after expiry' }
    Capture-Delayed @(1, 2)
    Invoke-Id 'remove-point-1'
    Wait-Points @(2)
    Capture-Delayed @(2, 3)
    Invoke-Id 'clear-picker'
    Wait-Confirmation 'confirm-clear'
    Wait-Focus 'cancel-confirmation'
    Press-Tab
    Wait-Focus 'confirm-clear'
    Press-Tab
    Wait-Focus 'cancel-confirmation'
    Press-Tab $true
    Wait-Focus 'confirm-clear'
    Press-Tab $true
    Wait-Focus 'cancel-confirmation'
    Hotkey-Capture
    Start-Sleep -Milliseconds 500
    Assert-Points @(2, 3)
    Wait-Confirmation 'confirm-clear'
    Invoke-Id 'cancel-confirmation'
    Assert-NoConfirmation
    Assert-Points @(2, 3)
    Invoke-Id 'clear-picker'
    Wait-Confirmation 'confirm-clear'
    Escape-Picker
    Assert-NoConfirmation
    Assert-Points @(2, 3)

    # Destroy only our probe; never send input or move the cursor onto other apps.
    $targetProcess.Kill()
    if (-not $targetProcess.WaitForExit(5000)) { throw 'Owned probe did not exit' }
    [PickerProbeNative]::SetForegroundWindow($pickerProcess.MainWindowHandle) | Out-Null
    try {
        foreach ($key in @(0x11, 0x12, 0x50)) { [PickerProbeNative]::keybd_event($key, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 40 }
    } finally {
        foreach ($key in @(0x50, 0x12, 0x11)) { [PickerProbeNative]::keybd_event($key, 0, 2, [UIntPtr]::Zero) }
    }
    Wait-Until 'lost target reported' { ((Node 'coordinates').Current.Name + ' ' + (Node 'picker-status').Current.Name) -match '失效|不可用|失败' }
    Start-Sleep -Milliseconds 400
    Assert-Points @(2, 3)
    Invoke-Id 'clear-picker'
    Wait-Confirmation 'confirm-clear'
    Invoke-Id 'confirm-clear'
    Assert-NoConfirmation
    Wait-Points @()
    $targetTitle = 'GH Picker Replacement ' + $token
    $targetProcess = Start-Probe $targetTitle
    Select-Probe $targetTitle
    Invoke-Id 'save-picker'
    Wait-Saved $fixtureStem @() $routeName
    if ((Read-Cache $fixtureStem).points.Count -ne 0) { throw 'Zero-point route failed the core contract' }
    Capture-Delayed @(1)
    Set-Value 'point-name-1' $pointName

    # An existing .yaml alone must trigger overwrite confirmation and preserve bytes on cancel.
    $conflictStem = $fixtureStem + '-conflict'
    $conflictPath = Join-Path $routes ($conflictStem + '.yaml')
    $original = $yaml + "`n# overwrite-original-" + $token + "`n"
    [IO.File]::WriteAllText($conflictPath, $original, [Text.UTF8Encoding]::new($false))
    Set-Value 'picker-stem' $conflictStem
    Invoke-Id 'save-picker'
    Wait-Confirmation 'confirm-overwrite'
    Invoke-Id 'cancel-confirmation'
    Assert-NoConfirmation
    if ([IO.File]::ReadAllText($conflictPath) -ne $original -or (Test-Path -LiteralPath (Join-Path $routes ('_cache\' + $conflictStem + '.json')))) { throw 'Cancelled overwrite wrote YAML or cache' }
    Invoke-Id 'save-picker'
    Wait-Confirmation 'confirm-overwrite'
    Escape-Picker
    Assert-NoConfirmation
    if ([IO.File]::ReadAllText($conflictPath) -ne $original) { throw 'Escape committed overwrite' }
    Invoke-Id 'save-picker'
    Wait-Confirmation 'confirm-overwrite'
    Invoke-Id 'confirm-overwrite'
    Wait-Saved $conflictStem @(1) $routeName
    $backups = @(Get-ChildItem -LiteralPath (Join-Path $routes '.backups') -Filter ($conflictStem + '.yaml.*.bak') -File)
    if (@($backups | Where-Object { [IO.File]::ReadAllText($_.FullName) -eq $original }).Count -ne 1) { throw 'Overwrite did not retain the exact original backup' }

    Invoke-Id 'hotkey'
    Wait-Until 'global hotkey released' { (Node 'hotkey').Current.Name -eq '启用 Ctrl+Alt+P' }
    $closingName = $routeName + '·关闭保存'
    Set-Value 'picker-name' $closingName
    $pickerProcess.CloseMainWindow() | Out-Null
    Wait-Confirmation 'save-close'
    Escape-Picker
    Assert-NoConfirmation
    if ((Get-Value 'picker-name') -ne $closingName) { throw 'Escape lost dirty route name' }
    $pickerProcess.CloseMainWindow() | Out-Null
    Wait-Confirmation 'save-close'
    Invoke-Id 'cancel-confirmation'
    Assert-NoConfirmation
    if ((Get-Value 'picker-name') -ne $closingName) { throw 'Close cancellation lost dirty route name' }
    Set-Value 'picker-stem' '..\outside'
    $pickerProcess.CloseMainWindow() | Out-Null
    Wait-Confirmation 'save-close'
    Invoke-Id 'save-close'
    Wait-Until 'failed save-close reports failure' { (Node 'picker-status').Current.Name -match '文件名|非法|无效|不允许|路径|失败|不能' }
    $pickerProcess.Refresh()
    if ($pickerProcess.HasExited -or (Get-Value 'picker-stem') -ne '..\outside') { throw 'Failed save-close exited or lost dirty filename' }
    if ($null -ne (Node 'picker-confirmation')) { Invoke-Id 'cancel-confirmation'; Assert-NoConfirmation }
    $closeStem = $fixtureStem + '-close'
    Set-Value 'picker-stem' $closeStem
    $pickerProcess.CloseMainWindow() | Out-Null
    Wait-Confirmation 'save-close'
    Invoke-Id 'save-close'
    if (-not $pickerProcess.WaitForExit(8000)) { throw 'Save-close did not close after successful YAML/cache save' }
    $closed = Read-Cache $closeStem
    if ($closed.name -ne $closingName -or $closed.points.Count -ne 1 -or $closed.points[0].name -ne $pointName) { throw 'Save-close exited before the full Unicode route/cache was persisted' }
    if (-not [IO.File]::ReadAllText((Join-Path $routes ($closeStem + '.yaml'))).Contains($closingName)) { throw 'Save-close lost YAML route name' }
    if ([IO.File]::ReadAllText((Join-Path $appRoot 'config.json')) -ne $configText) { throw 'Picker unexpectedly changed sandbox daemon configuration' }
    Write-Output 'Passed: isolated native selection/search, unfocused Ctrl+Alt+P precision, Unicode YAML/cache and clipboard, countdown button/Escape cancellation past expiry, delayed capture, stable point IDs/delete, clear cancel/confirm and zero points, lost target, overwrite cancel/backup, dirty-close cancel/failed-save recovery/save-close. No playback requested.'
} finally {
    foreach ($key in @(0x50, 0x12, 0x11, 0x1B, 0x09, 0x10)) { [PickerProbeNative]::keybd_event($key, 0, 2, [UIntPtr]::Zero) }
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
