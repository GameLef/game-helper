#requires -Version 7.0
param(
    [string]$ProjectRoot = (Split-Path $PSScriptRoot -Parent),
    [string]$Executable = 'target\debug\game-helper.exe'
)
# Offline fixture only: no game window, capture hotkey, daemon, or playback.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class PickerOrderNative {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd,out Rect rect);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd,out Rect rect);
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hwnd,int x,int y,int width,int height,bool repaint);
}
'@
[PickerOrderNative]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
$ProjectRoot = [IO.Path]::GetFullPath($ProjectRoot)
$exe = if ([IO.Path]::IsPathRooted($Executable)) { $Executable } else { Join-Path $ProjectRoot $Executable }
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Build the native executable first: $exe" }
$appRoot = Join-Path $ProjectRoot ('.tools\picker-ordering-' + [guid]::NewGuid().ToString('N'))
foreach ($directory in @('routes', 'ahk', 'state')) {
    [IO.Directory]::CreateDirectory((Join-Path $appRoot $directory)) | Out-Null
}
$utf8 = [Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText((Join-Path $appRoot 'ahk\daemon.ahk'), '; Inert offline ordering fixture.', $utf8)
[IO.File]::WriteAllText((Join-Path $appRoot 'config.json'), '{"routesDir":"routes","autoStartDaemon":false,"autoReloadOnSave":false,"hotkeys":{"stopAll":"F12","bindings":[]}}', $utf8)
$primary = @{ process = 'ordering-fixture.exe'; windowClass = 'PickerOrderFixture'; title = '离线排序 · 中文点位' }
$secondary = @{ process = 'ordering-fixture.exe'; windowClass = 'PickerOrderFixture'; title = '另一分组 · 单点' }
$seedPoints = @(1..20 | ForEach-Object {
    @{ id = $_; name = "采集点 $_ · 长名称测试"; position = @((0.123456789 + $_ / 1000.0), (0.987654321 - $_ / 1000.0)) }
})
$singlePoint = @{ id = 101; name = '保留的单点'; position = @(0.314159265358, 0.271828182845) }
$statePath = Join-Path $appRoot 'state\picker.json'
$initial = @{ format = 'picker/v1'; lastTarget = $primary; targets = @(
    @{ target = $primary; points = $seedPoints },
    @{ target = $secondary; points = @($singlePoint) }
) }
[IO.File]::WriteAllText($statePath, ($initial | ConvertTo-Json -Depth 10), $utf8)
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()
$pickerProcess = $null
$root = $null
$scope = [Windows.Automation.TreeScope]::Descendants
$clipboardBefore = $null
$restoreClipboard = $false
function Wait-Until([string]$Description, [scriptblock]$Predicate, [int]$TimeoutMs = 8000) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    do {
        if (& $Predicate) { return }
        Start-Sleep -Milliseconds 50
    } while ($watch.ElapsedMilliseconds -lt $TimeoutMs)
    throw "Timed out: $Description"
}
function Node([string]$Id) {
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, $Id)
    $root.FindFirst($scope, $condition)
}
function Start-Picker {
    $script:pickerProcess = Start-Process -FilePath $exe -ArgumentList @('--picker', '--app-root', ('"' + $appRoot + '"')) -PassThru
    $owned.Add($pickerProcess)
    Wait-Until 'owned picker window' {
        $pickerProcess.Refresh()
        if ($pickerProcess.HasExited) { throw 'Owned picker exited before opening a window' }
        $pickerProcess.MainWindowHandle -ne [IntPtr]::Zero
    }
    Wait-Until 'offline points restored' {
        # AccessKit can initialize after the first HWND is exposed. Reacquire
        # the provider until its content tree is available rather than keeping
        # the initial empty AutomationElement for every readiness attempt.
        $pickerProcess.Refresh()
        $script:root = [Windows.Automation.AutomationElement]::FromHandle($pickerProcess.MainWindowHandle)
        $null = $root.FindAll($scope, [Windows.Automation.Condition]::TrueCondition)
        $null -ne (Node 'point-name-20') -or $null -ne (Node 'point-name-101')
    }
}
function Close-Picker {
    $pickerProcess.Refresh()
    if (-not $pickerProcess.CloseMainWindow()) { throw 'Cannot request owned picker close' }
    Wait-Until 'owned picker saved and closed' { $pickerProcess.HasExited }
}
function Invoke-Id([string]$Id) {
    Wait-Until "enabled control $Id" { $node = Node $Id; $null -ne $node -and $node.Current.IsEnabled }
    (Node $Id).GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 60
}
function Set-Value([string]$Id, [string]$Text) {
    (Node $Id).GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($Text)
}
function Input-Value([string]$Id) {
    (Node $Id).GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value
}
function Assert-Enabled([string]$Id, [bool]$Expected) {
    $node = Node $Id
    if ($null -eq $node -or $node.Current.IsEnabled -ne $Expected) { throw "Incorrect enabled state: $Id expected $Expected" }
}
function Read-State { [IO.File]::ReadAllText($statePath) | ConvertFrom-Json }
function Target-Group($State, [string]$Title) { @($State.targets | Where-Object { $_.target.title -ceq $Title })[0] }
function Wait-Order([int[]]$Expected) {
    Wait-Until ('persisted order ' + ($Expected -join ',')) {
        $saved = Target-Group (Read-State) $primary.title
        (@($saved.points.id) -join ',') -ceq ($Expected -join ',')
    }
}
function Assert-PrecisionAndIsolation {
    $state = Read-State
    $other = Target-Group $state $secondary.title
    if ($other.points.Count -ne 1 -or $other.points[0].id -ne 101 -or $other.points[0].name -cne $singlePoint.name) { throw 'Sorting modified another target group' }
    $current = Target-Group $state $primary.title
    foreach ($point in $current.points) {
        $seed = @($seedPoints | Where-Object { $_.id -eq $point.id })[0]
        # PowerShell and serde JSON decoders may differ by one binary ULP;
        # this tolerance still detects any six-decimal rounding of the fixture.
        if ($null -eq $seed -or [Math]::Abs($point.position[0] - $seed.position[0]) -gt 1e-15 -or [Math]::Abs($point.position[1] - $seed.position[1]) -gt 1e-15) { throw "Sorting changed point identity or precision: $($point.id)" }
    }
    if ([Math]::Abs($other.points[0].position[0] - $singlePoint.position[0]) -gt 1e-15 -or [Math]::Abs($other.points[0].position[1] - $singlePoint.position[1]) -gt 1e-15) { throw 'Sorting changed other target coordinates' }
}
function Assert-RowVisible([int]$Id) {
    $viewport = (Node 'picker-points').Current.BoundingRectangle
    foreach ($control in @("captured-point-$Id", "point-name-$Id", "copy-coordinate-$Id", "move-point-up-$Id", "move-point-down-$Id", "copy-point-$Id", "remove-point-$Id")) {
        $node = Node $control
        if ($null -eq $node -or $node.Current.IsOffscreen -or $node.Current.BoundingRectangle.IsEmpty) { throw "Moved row control is hidden: $control" }
        $rect = $node.Current.BoundingRectangle
        if ($rect.Left -lt $viewport.Left - 1 -or $rect.Right -gt $viewport.Right + 1 -or $rect.Top -lt $viewport.Top - 1 -or $rect.Bottom -gt $viewport.Bottom + 1) {
            throw "Moved row control outside list viewport: $control $rect; viewport $viewport"
        }
    }
    if ((Node "point-name-$Id").Current.BoundingRectangle.Width -lt 120) { throw 'Compact actions squeezed name input below 120px' }
}
function Resize-Picker([int]$Width, [int]$Height) {
    $scale = [PickerOrderNative]::GetDpiForWindow($pickerProcess.MainWindowHandle) / 96.0
    $client = [PickerOrderNative+Rect]::new()
    $outer = [PickerOrderNative+Rect]::new()
    if (-not [PickerOrderNative]::GetClientRect($pickerProcess.MainWindowHandle, [ref]$client) -or -not [PickerOrderNative]::GetWindowRect($pickerProcess.MainWindowHandle, [ref]$outer)) { throw 'Cannot read picker bounds' }
    $frameWidth = ($outer.Right - $outer.Left) - ($client.Right - $client.Left)
    $frameHeight = ($outer.Bottom - $outer.Top) - ($client.Bottom - $client.Top)
    if (-not [PickerOrderNative]::MoveWindow($pickerProcess.MainWindowHandle, 30, 30, ([int][Math]::Round($Width * $scale) + $frameWidth), ([int][Math]::Round($Height * $scale) + $frameHeight), $true)) { throw 'Cannot resize owned picker' }
    Start-Sleep -Milliseconds 250
    $actual = [PickerOrderNative+Rect]::new()
    if (-not [PickerOrderNative]::GetClientRect($pickerProcess.MainWindowHandle, [ref]$actual)) { throw 'Cannot verify resized picker client' }
    $logicalWidth = ($actual.Right - $actual.Left) / $scale
    $logicalHeight = ($actual.Bottom - $actual.Top) / $scale
    if ([Math]::Abs($logicalWidth - $Width) -gt 1 -or [Math]::Abs($logicalHeight - $Height) -gt 1) { throw "Picker client did not reach ${Width}x${Height}: ${logicalWidth}x${logicalHeight}" }
    Write-Output "Verified picker client ${Width}x${Height}; DPI $([PickerOrderNative]::GetDpiForWindow($pickerProcess.MainWindowHandle))"
}
try {
    Write-Output "Isolated offline ordering sandbox: $appRoot"
    try { $clipboardBefore = Get-Clipboard -Raw; $restoreClipboard = $true } catch { }
    Start-Picker
    Resize-Picker 1120 800
    Assert-Enabled 'move-point-up-1' $false
    Assert-Enabled 'move-point-down-1' $true
    Assert-Enabled 'move-point-up-20' $true
    Assert-Enabled 'move-point-down-20' $false
    Assert-RowVisible 20
    # Pending name edits must be captured by the immediate sorting save.
    Set-Value 'point-name-20' '排序前尚未防抖保存的名称'
    Invoke-Id 'move-point-up-20'
    Wait-Order (@(1..18) + @(20, 19))
    Assert-RowVisible 20
    if ((Target-Group (Read-State) $primary.title).points[18].name -cne '排序前尚未防抖保存的名称') { throw 'Move lost pending name edit' }
    # Delete the swapped neighbor, then edit the survivor. This catches a stale
    # point_subscriptions order that would unsubscribe the surviving input.
    Invoke-Id 'remove-point-19'
    Wait-Order (@(1..18) + @(20))
    Set-Value 'point-name-20' '删除相邻行后继续编辑'
    Wait-Until 'surviving name subscription saves' {
        (Target-Group (Read-State) $primary.title).points[-1].name -ceq '删除相邻行后继续编辑'
    }
    for ($step = 0; $step -lt 18; $step++) {
        Invoke-Id 'move-point-up-20'
        Assert-RowVisible 20
    }
    Wait-Order (@(20) + @(1..18))
    Assert-Enabled 'move-point-up-20' $false
    Assert-Enabled 'move-point-down-20' $true
    Assert-Enabled 'move-point-down-18' $false
    # A down/up pair checks both directions without changing the final order.
    Invoke-Id 'move-point-down-20'
    Assert-RowVisible 20
    Invoke-Id 'move-point-up-20'
    Assert-RowVisible 20
    Wait-Order (@(20) + @(1..18))
    Assert-PrecisionAndIsolation
    Resize-Picker 900 640
    $sidebar = Node 'picker-search'
    if ($null -eq $sidebar -or $sidebar.Current.IsOffscreen -or $sidebar.Current.BoundingRectangle.IsEmpty) { Invoke-Id 'toggle-targets' }
    Wait-Until 'compact sidebar manually opened' { $node = Node 'picker-search'; $null -ne $node -and -not $node.Current.IsOffscreen }
    Assert-RowVisible 20
    $nameBounds = (Node 'point-name-20').Current.BoundingRectangle
    $actionBounds = (Node 'move-point-down-20').Current.BoundingRectangle
    if ($actionBounds.Top -lt $nameBounds.Bottom - 1) { throw 'Narrow layout did not move actions to a second line' }
    foreach ($copy in @('copy-coordinate-20', 'copy-point-20')) {
        Set-Clipboard -Value 'picker-order-sentinel'
        Invoke-Id $copy
        Wait-Until "copied coordinates by $copy" { [string](Get-Clipboard -Raw) -cne 'picker-order-sentinel' }
        $pair = ConvertFrom-Json -InputObject ([string](Get-Clipboard -Raw)) -NoEnumerate
        $seed = $seedPoints[19]
        if ($pair -isnot [array] -or $pair.Count -ne 2) { throw 'Copy did not return a coordinate pair' }
        for ($coordinate = 0; $coordinate -lt 2; $coordinate++) {
            if ([Math]::Abs($pair[$coordinate] - $seed.position[$coordinate]) -gt 0.000001) { throw 'Sorted point copied wrong coordinates' }
        }
    }
    # A sharing failure must leave the moved row in memory until retry saves it.
    $stateLock = [IO.File]::Open($statePath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
    try {
        Invoke-Id 'move-point-down-20'
        Wait-Until 'sorting save failure exposes retry' { $node = Node 'retry-persistence'; $null -ne $node -and $node.Current.IsEnabled }
        if ((Node 'captured-point-20').Current.Name -notmatch '^第 2 个点位 ') { throw 'Failed save reverted in-memory order' }
        if ((Input-Value 'point-name-20') -cne '删除相邻行后继续编辑') { throw 'Failed save lost point input' }
        Assert-RowVisible 20
    } finally {
        $stateLock.Dispose()
    }
    Invoke-Id 'retry-persistence'
    Wait-Order (@(1, 20) + @(2..18))
    Wait-Until 'retry clears sorting save error' { $null -eq (Node 'retry-persistence') }
    Assert-PrecisionAndIsolation
    Invoke-Id 'move-point-up-20'
    Wait-Order (@(20) + @(1..18))
    Assert-RowVisible 20
    # Crosshair is cancelled before it selects anything; no external app input.
    Invoke-Id 'crosshair'
    Wait-Until 'selection countdown visible' { $null -ne (Node 'picker-countdown') }
    Assert-Enabled 'move-point-down-20' $false
    Assert-Enabled 'move-point-up-1' $false
    Invoke-Id 'cancel-countdown'
    Assert-Enabled 'move-point-down-20' $true
    Invoke-Id 'clear-picker'
    Wait-Until 'clear confirmation visible' { $null -ne (Node 'picker-confirmation') }
    Assert-Enabled 'move-point-down-20' $false
    Invoke-Id 'cancel-confirmation'
    Assert-Enabled 'move-point-down-20' $true
    # Closing immediately after a fresh name edit must flush the latest state.
    Set-Value 'point-name-20' '重启后仍然保留的排序名称'
    Close-Picker
    Wait-Order (@(20) + @(1..18))
    Assert-PrecisionAndIsolation
    Start-Picker
    if ((Input-Value 'point-name-20') -cne '重启后仍然保留的排序名称') { throw 'Restart lost latest point name' }
    if ((Node 'captured-point-20').Current.Name -notmatch '^第 1 个点位 ') { throw 'Restart did not restore sorted display order' }
    Close-Picker
    # Switch the isolated offline lastTarget on disk while our picker is closed.
    $saved = Read-State
    $saved.lastTarget = $secondary
    [IO.File]::WriteAllText($statePath, ($saved | ConvertTo-Json -Depth 10), $utf8)
    Start-Picker
    Assert-Enabled 'move-point-up-101' $false
    Assert-Enabled 'move-point-down-101' $false
    if ((Input-Value 'point-name-101') -cne $singlePoint.name) { throw 'Single-point target changed' }
    Close-Picker
    Wait-Order (@(20) + @(1..18))
    Assert-PrecisionAndIsolation
    Write-Output 'Passed: ordering boundaries, both directions, pending edits, paired subscriptions after neighbor deletion, stable ids/precision, moved-row visibility, 900x640 sidebar-open action layout, both coordinate-copy entries, save failure with retained order and retry, countdown/modal lock, close/restart persistence, single-point disable and target isolation.'
} catch {
    $failure = $_
    try {
        $pickerProcess.Refresh()
        $diagnosticRoot = [Windows.Automation.AutomationElement]::FromHandle($pickerProcess.MainWindowHandle)
        $diagnostic = @($diagnosticRoot.FindAll($scope, [Windows.Automation.Condition]::TrueCondition) | ForEach-Object {
            [pscustomobject]@{
                Id = $_.Current.AutomationId
                Name = $_.Current.Name
                Type = $_.Current.ControlType.ProgrammaticName
                Bounds = $_.Current.BoundingRectangle.ToString()
                Enabled = $_.Current.IsEnabled
                Offscreen = $_.Current.IsOffscreen
            }
        })
        $diagnosticPath = Join-Path $appRoot 'failure-uia.json'
        [IO.File]::WriteAllText($diagnosticPath, ($diagnostic | ConvertTo-Json -Depth 5), $utf8)
        Write-Output "Failure UIA tree: $diagnosticPath; root: $($diagnosticRoot.Current.Name); handle: $($pickerProcess.MainWindowHandle)"
    } catch {
        Write-Warning "Could not capture UIA diagnostic: $_"
    }
    throw $failure
} finally {
    foreach ($process in $owned) {
        $process.Refresh()
        if (-not $process.HasExited) {
            $process.CloseMainWindow() | Out-Null
            if (-not $process.WaitForExit(2000)) { $process.Kill(); $process.WaitForExit(5000) | Out-Null }
        }
        $process.Dispose()
    }
    if ($restoreClipboard) {
        try { Set-Clipboard -Value $(if ($null -eq $clipboardBefore) { '' } else { $clipboardBefore }) } catch { Write-Warning 'Could not restore clipboard text' }
    }
    Write-Output "Sandbox retained for inspection: $appRoot"
}
