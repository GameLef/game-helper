#requires -Version 7.0
param(
    [string]$ProjectRoot = (Split-Path $PSScriptRoot -Parent),
    [string]$Executable = 'target\release\game-helper.exe',
    [switch]$CaptureScreenshots
)
# Owns only the managers it starts and one GUID app root. No daemon or playback.
# Requires an interactive Windows desktop and a build with native point drag handles.
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ManagerPointOrderingNative {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr value);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr window,int x,int y,int width,int height,bool repaint);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint dx,uint dy,int data,UIntPtr extra);
}
'@
[ManagerPointOrderingNative]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
$ProjectRoot = [IO.Path]::GetFullPath($ProjectRoot)
$exe = if ([IO.Path]::IsPathRooted($Executable)) { $Executable } else { Join-Path $ProjectRoot $Executable }
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Build the native executable first: $exe" }
$appRoot = Join-Path $ProjectRoot ('.tools\manager-point-ordering-' + [guid]::NewGuid().ToString('N'))
$routes = Join-Path $appRoot 'routes'
[IO.Directory]::CreateDirectory($routes) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $appRoot 'ahk')) | Out-Null
$utf8 = [Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText((Join-Path $appRoot 'ahk\daemon.ahk'), '; Inert point-ordering smoke fixture. Never executed.', $utf8)
$configPath = Join-Path $appRoot 'config.json'
[IO.File]::WriteAllText($configPath, '{"routesDir":"routes","autoStartDaemon":false,"autoReloadOnSave":false,"hotkeys":{"stopAll":"F12","bindings":[]}}', $utf8)
$expected = [Collections.Generic.List[object]]::new()
for ($i = 0; $i -lt 40; $i++) {
    # Three occurrences of ID 77 deliberately have different names and actions.
    $duplicate = $i -in @(4, 6, 25)
    $operations = @()
    if ($duplicate -or $i % 5 -ne 3) {
        $operations += [pscustomobject]@{ type = 'wait'; name = ('等待 {0:D2}-A' -f $i); ms = 100 + $i }
        if ($duplicate -or $i % 3 -eq 0) {
            $operations += [pscustomobject]@{ type = 'wait'; name = ('等待 {0:D2}-B' -f $i); ms = 200 + $i }
        } elseif ($i % 3 -eq 1) {
            $operations += [pscustomobject]@{ type = 'wheel'; name = ('滚动 {0:D2}' -f $i); delta = -2 }
        }
    }
    $expected.Add([pscustomobject]@{
        id = $(if ($duplicate) { 77 } else { 1000 + $i })
        name = ('排序点位 {0:D2}' -f $i); note = "点位内容 $i"; operations = $operations
    })
}
$anchor = $expected[4]
$otherDuplicate = $expected[6]
$thirdDuplicate = $expected[25]
$fixture = [ordered]@{
    format = 'route/v1'; name = 'Point ordering'; note = 'Isolated native point ordering acceptance'
    target = @{ process = 'never-playback.exe' }; points = $expected.ToArray()
}
$routePath = Join-Path $routes 'Ordering.yaml'
$cachePath = Join-Path $routes '_cache\Ordering.json'
# JSON is valid YAML; the first structured save must use the normalization dialog.
[IO.File]::WriteAllText($routePath, ($fixture | ConvertTo-Json -Depth 12), $utf8)
$fixture.name = 'Other'
$fixture.points = @(@{ id = 1; name = '其他路线点位'; operations = @(@{ type = 'wait'; name = '其他等待'; ms = 1 }) })
[IO.File]::WriteAllText((Join-Path $routes 'Other.yaml'), ($fixture | ConvertTo-Json -Depth 12), $utf8)
$originalSource = [IO.File]::ReadAllText($routePath)
$originalRouteTime = [IO.File]::GetLastWriteTimeUtc($routePath)
$originalConfig = [IO.File]::ReadAllText($configPath)
$originalConfigTime = [IO.File]::GetLastWriteTimeUtc($configPath)
$manager = $null
$root = $null
$selectedPoint = $expected[0]
$scope = [Windows.Automation.TreeScope]::Descendants
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()
$steps = [Collections.Generic.List[object]]::new()
$normalizationDialogs = 0
$transcribing = $false
$result = [ordered]@{ status = 'running'; appRoot = $appRoot; executable = $exe; screenshots = [bool]$CaptureScreenshots }

function Wait-Until([string]$Description, [scriptblock]$Predicate, [int]$TimeoutMs = 10000) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    do {
        if (& $Predicate) { return }
        Start-Sleep -Milliseconds 75
    } while ($watch.ElapsedMilliseconds -lt $TimeoutMs)
    throw "Timed out: $Description"
}
function Node([string]$Id) {
    $root.FindFirst($scope, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, $Id))
}
function Named([string]$Name) {
    $root.FindFirst($scope, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $Name))
}
function Need-Node([string]$Id) {
    Wait-Until "control $Id" { $null -ne (Node $Id) }
    Node $Id
}
function Invoke-Node($Control) {
    if ($null -eq $Control) { throw 'Missing invocation target' }
    if (-not $Control.Current.IsEnabled) { throw ('Disabled control: ' + $Control.Current.Name) }
    $patterns = @($Control.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName })
    if ($patterns -contains 'InvokePatternIdentifiers.Pattern') {
        $Control.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    } elseif ($patterns -contains 'ExpandCollapsePatternIdentifiers.Pattern') {
        $pattern = $Control.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern)
        if ($pattern.Current.ExpandCollapseState -eq [Windows.Automation.ExpandCollapseState]::Collapsed) { $pattern.Expand() } else { $pattern.Collapse() }
    } elseif ($patterns -contains 'TogglePatternIdentifiers.Pattern') {
        $Control.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    } elseif ($patterns -contains 'SelectionItemPatternIdentifiers.Pattern') {
        $Control.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    } else { throw ('Unsupported invocation target: ' + $Control.Current.AutomationId + '; ' + ($patterns -join ', ')) }
    Start-Sleep -Milliseconds 125
}
function Invoke-Id([string]$Id) { Invoke-Node (Need-Node $Id) }
function Invoke-Name([string]$Name) { Invoke-Node (Named $Name) }
function Input-Value([string]$Id) { (Need-Node $Id).GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value }
function Set-Input([string]$Id, [string]$Text) {
    (Need-Node $Id).GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($Text)
    Start-Sleep -Milliseconds 100
}
function Foreground-Owned {
    [ManagerPointOrderingNative]::SetForegroundWindow($manager.MainWindowHandle) | Out-Null
    Wait-Until 'owned manager foreground' { [ManagerPointOrderingNative]::GetForegroundWindow() -eq $manager.MainWindowHandle }
}
function Wait-Ready {
    Wait-Until 'manager ready' {
        $status = Node 'manager-status'
        $null -ne $status -and $status.Current.Name -notmatch '正在加载|正在处理' -and $null -eq (Node 'route-loading-shield')
    }
}
function Ensure-Library {
    if ($null -eq (Node 'route-Ordering')) { Invoke-Name '路线库' }
    Need-Node 'route-Ordering' | Out-Null
}
function Open-Route([string]$Stem) {
    Ensure-Library
    Invoke-Id ('route-' + $Stem)
    if ($null -ne (Node 'confirmation-dialog')) { throw "Unexpected dirty confirmation when opening $Stem" }
    Wait-Ready
    Need-Node 'operation-toolbar' | Out-Null
}
function Start-Manager {
    $script:manager = Start-Process -FilePath $exe -ArgumentList @('--app-root', ('"' + $appRoot + '"')) -PassThru
    $owned.Add($manager)
    Wait-Until 'native manager window' {
        $manager.Refresh()
        if ($manager.HasExited) { throw 'Owned manager exited before opening its window' }
        if ($manager.MainWindowHandle -eq [IntPtr]::Zero) { return $false }
        try {
            $candidate = [Windows.Automation.AutomationElement]::FromHandle($manager.MainWindowHandle)
            if ($candidate.Current.Name -ne '路线管理') { return $false }
            $script:root = $candidate
            $null -ne (Node 'manager-status')
        } catch { $false }
    } 15000
    Wait-Ready
    Foreground-Owned
    Open-Route 'Ordering'
    $script:selectedPoint = $expected[0]
}
function Stop-Manager {
    $manager.CloseMainWindow() | Out-Null
    if (-not $manager.WaitForExit(5000)) { throw 'Owned clean manager did not close and flush UI state before restart' }
}
function Scale { [ManagerPointOrderingNative]::GetDpiForWindow($manager.MainWindowHandle) / 96.0 }
function Resize-Manager([int]$Width, [int]$Height) {
    $scale = Scale
    $client = [ManagerPointOrderingNative+Rect]::new()
    $outer = [ManagerPointOrderingNative+Rect]::new()
    if (-not [ManagerPointOrderingNative]::GetClientRect($manager.MainWindowHandle, [ref]$client) -or -not [ManagerPointOrderingNative]::GetWindowRect($manager.MainWindowHandle, [ref]$outer)) { throw 'Cannot read owned manager bounds' }
    $frameWidth = ($outer.Right - $outer.Left) - ($client.Right - $client.Left)
    $frameHeight = ($outer.Bottom - $outer.Top) - ($client.Bottom - $client.Top)
    if (-not [ManagerPointOrderingNative]::MoveWindow($manager.MainWindowHandle, 25, 25, ([int][Math]::Round($Width * $scale) + $frameWidth), ([int][Math]::Round($Height * $scale) + $frameHeight), $true)) { throw 'Cannot resize owned manager' }
    Start-Sleep -Milliseconds 300
    [ManagerPointOrderingNative]::GetClientRect($manager.MainWindowHandle, [ref]$client) | Out-Null
    if ([Math]::Abs(($client.Right - $client.Left) / $scale - $Width) -gt 1 -or [Math]::Abs(($client.Bottom - $client.Top) / $scale - $Height) -gt 1) { throw "Client size did not reach ${Width}x${Height}" }
}
function In-Bounds($Control, $Bounds) {
    if ($null -eq $Control -or $Control.Current.IsOffscreen) { return $false }
    $r = $Control.Current.BoundingRectangle
    -not $r.IsEmpty -and $r.Width -gt 0 -and $r.Height -gt 0 -and $r.Left -ge ($Bounds.Left - 1) -and $r.Right -le ($Bounds.Right + 1) -and $r.Top -ge ($Bounds.Top - 1) -and $r.Bottom -le ($Bounds.Bottom + 1)
}
function Reveal-Point([int]$Index) {
    $id = 'point-drag-' + $Index
    $control = Need-Node $id
    if (@($control.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName }) -contains 'ScrollItemPatternIdentifiers.Pattern') {
        $control.GetCurrentPattern([Windows.Automation.ScrollItemPattern]::Pattern).ScrollIntoView()
        Start-Sleep -Milliseconds 100
    }
    for ($step = 0; $step -lt 80; $step++) {
        $viewport = (Need-Node 'points').Current.BoundingRectangle
        $control = Need-Node $id
        if (In-Bounds $control $viewport) { return }
        $r = $control.Current.BoundingRectangle
        $amount = if ($r.Top -lt $viewport.Top) { 120 } else { -120 }
        Foreground-Owned
        [ManagerPointOrderingNative]::SetCursorPos([int]($viewport.Left + $viewport.Width / 2), [int]($viewport.Top + $viewport.Height / 2)) | Out-Null
        [ManagerPointOrderingNative]::mouse_event(0x0800, 0, 0, $amount, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 75
    }
    throw "Cannot reveal point $Index in points viewport"
}
function Assert-Order {
    for ($i = 0; $i -lt $expected.Count; $i++) {
        $name = (Need-Node ('point-' + $i)).Current.Name
        $wanted = "第 $($i + 1) 点 $($expected[$i].name)，$(@($expected[$i].operations).Count) 个动作"
        if ($name -cne $wanted) { throw "Point order/name/count mismatch at ${i}: actual=$name expected=$wanted" }
    }
    if ($null -ne (Node ('point-' + $expected.Count))) { throw 'Unexpected extra point' }
}
function Ensure-PointInfo {
    if ($null -eq (Node 'point_name')) { Invoke-Id 'point-info-toggle' }
}
function Assert-Selected {
    Ensure-PointInfo
    $index = $expected.IndexOf($selectedPoint)
    if ($index -lt 0 -or (Input-Value 'point_name') -cne $selectedPoint.name -or (Input-Value 'point_note') -cne $selectedPoint.note) { throw "Selected point metadata changed; expected index=$index name=$($selectedPoint.name)" }
    $control = Need-Node ('point-' + $index)
    if (@($control.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName }) -contains 'SelectionItemPatternIdentifiers.Pattern') {
        if (-not $control.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Current.IsSelected) { throw "Expected selected point index $index" }
    }
    if ((Named '上移').Current.IsEnabled -ne ($index -gt 0) -or (Named '下移').Current.IsEnabled -ne ($index -lt $expected.Count - 1)) { throw "Bottom move buttons track the wrong selected index: $index" }
}
function Select-Point($Point) {
    $index = $expected.IndexOf($Point)
    Reveal-Point $index
    Invoke-Id ('point-' + $index)
    $script:selectedPoint = $Point
    Assert-Selected
}
function Assert-Disclosure($Point, [int[]]$Collapsed) {
    Select-Point $Point
    for ($i = 0; $i -lt @($Point.operations).Count; $i++) {
        Need-Node ('operation-toggle-' + $i) | Out-Null
        $input = Node "operation-$i-name"
        if (($null -eq $input) -ne ($i -in $Collapsed)) { throw "Disclosure state moved to the wrong point/action: $($Point.name), action $i" }
        if ($null -ne $input) {
            if ((Input-Value "operation-$i-name") -cne $Point.operations[$i].name) { throw "Action content lost for $($Point.name), action $i" }
            if ($Point.operations[$i].type -eq 'wait' -and (Input-Value "operation-$i-ms") -ne [string]$Point.operations[$i].ms) { throw "Wait duration lost for $($Point.name), action $i" }
        }
    }
}
function Move-Expected([int]$From, [int]$To) {
    $point = $expected[$From]
    $expected.RemoveAt($From)
    $expected.Insert($To, $point)
}
function Record-Step([string]$Label, $Details = $null) {
    $entry = [ordered]@{ label = $Label; selectedIndex = $expected.IndexOf($selectedPoint); selectedName = $selectedPoint.name; details = $Details }
    $steps.Add($entry)
    Write-Output ($entry | ConvertTo-Json -Depth 6 -Compress)
}
function Capture([string]$Label) {
    if (-not $CaptureScreenshots) { return }
    Foreground-Owned
    $bounds = $root.Current.BoundingRectangle
    $bitmap = [Drawing.Bitmap]::new([int]$bounds.Width, [int]$bounds.Height)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen([int]$bounds.Left, [int]$bounds.Top, 0, 0, $bitmap.Size)
        $bitmap.Save((Join-Path $appRoot ($Label + '.png')), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
}
function Assert-Geometry([string]$Label) {
    Reveal-Point 0
    $viewport = Need-Node 'points'
    $bounds = $viewport.Current.BoundingRectangle
    if (-not (In-Bounds $viewport $root.Current.BoundingRectangle)) { throw 'Points viewport clipped by window' }
    $all = @($viewport.FindAll($scope, [Windows.Automation.Condition]::TrueCondition))
    foreach ($pattern in @('^point-\d+$', '^point-drag-\d+$', '^point-row-\d+$')) {
        if (@($all | Where-Object { $_.Current.AutomationId -match $pattern }).Count -ne 40) { throw "Expected 40 accessible controls matching $pattern" }
    }
    $visible = @()
    $walker = [Windows.Automation.TreeWalker]::RawViewWalker
    for ($i = 0; $i -lt 40; $i++) {
        $button = Need-Node "point-$i"
        $handle = Need-Node "point-drag-$i"
        $row = Need-Node "point-row-$i"
        foreach ($control in @($button, $handle)) {
            if ($walker.GetParent($control).Current.AutomationId -cne "point-row-$i") { throw "Point select and drag handle must be siblings inside point-row-$i" }
        }
        if (-not (In-Bounds $button $bounds)) { continue }
        $r = $row.Current.BoundingRectangle
        $b = $button.Current.BoundingRectangle
        $h = $handle.Current.BoundingRectangle
        if ([Math]::Abs($b.Height / (Scale) - 36) -gt 1 -or [Math]::Abs($r.Height / (Scale) - 36) -gt 1) { throw "Point $i no longer has a 36 DIP select button/row" }
        if (-not (In-Bounds $handle $r) -or -not (In-Bounds $button $r) -or -not (In-Bounds $row $bounds)) { throw "Point controls clipped in row $i" }
        if ([Math]::Min($b.Right, $h.Right) - [Math]::Max($b.Left, $h.Left) -gt 1) { throw "Drag handle overlaps point select button at $i" }
        $visible += [pscustomobject]@{ index = $i; top = $r.Top / (Scale); bottom = $r.Bottom / (Scale); selectHeight = $b.Height / (Scale); handleWidth = $h.Width / (Scale) }
    }
    if ($visible.Count -lt 3) { throw 'Too few complete visible point rows' }
    for ($i = 1; $i -lt $visible.Count; $i++) {
        if ([Math]::Abs($visible[$i].top - $visible[$i - 1].bottom - 2) -gt 1) { throw 'Point row gap must remain 2 DIP' }
    }
    Record-Step $Label @{ dpi = [int]((Scale) * 96); viewportWidth = $bounds.Width / (Scale); viewportHeight = $bounds.Height / (Scale); rows = $visible }
    Capture $Label
}
function Assert-OriginalClean {
    Assert-Order
    if ($null -ne (Named '有未保存修改') -or $null -ne (Node 'confirmation-dialog')) { throw 'No-op/cancel/disclosure dirtied the route' }
    if ([IO.File]::ReadAllText($routePath) -cne $originalSource -or [IO.File]::GetLastWriteTimeUtc($routePath) -ne $originalRouteTime) { throw 'Clean gesture wrote route contents or mtime' }
    if ([IO.File]::ReadAllText($configPath) -cne $originalConfig -or [IO.File]::GetLastWriteTimeUtc($configPath) -ne $originalConfigTime) { throw 'Clean gesture wrote config contents or mtime' }
    if (Test-Path -LiteralPath $cachePath) { throw 'Clean gesture generated execution cache' }
}
function Pointer-Move([int]$X, [int]$Y) {
    if (-not [ManagerPointOrderingNative]::SetCursorPos($X, $Y)) { throw 'Cannot move native pointer on interactive desktop' }
}
function Pointer-Down { [ManagerPointOrderingNative]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero) }
function Pointer-Up { [ManagerPointOrderingNative]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero) }
function Move-PointerPath([int]$X, [int]$Y, [int]$EndX, [int]$EndY) {
    for ($step = 1; $step -le 16; $step++) {
        Pointer-Move ([int]($X + ($EndX - $X) * $step / 16)) ([int]($Y + ($EndY - $Y) * $step / 16))
        Start-Sleep -Milliseconds 25
    }
}
function Drag-PointSplitter([int]$DeltaDip) {
    $viewport = (Need-Node 'points').Current.BoundingRectangle
    $x = [int]($viewport.Right + (Scale))
    $y = [int]($viewport.Top + $viewport.Height / 2)
    Foreground-Owned
    Pointer-Move $x $y
    Pointer-Down
    try { Move-PointerPath $x $y ([int]($x + $DeltaDip * (Scale))) $y } finally { Pointer-Up }
    Start-Sleep -Milliseconds 200
}
function Assert-MinimumPointPane([string]$Label) {
    $before = (Need-Node 'points').Current.BoundingRectangle.Width / (Scale)
    Drag-PointSplitter -160
    $minimum = (Need-Node 'points').Current.BoundingRectangle.Width / (Scale)
    # The viewport can exclude the one-DIP panel border.
    if ([Math]::Abs($minimum - 210) -gt 2) { throw "Point pane did not reach its 210 DIP minimum: $minimum" }
    Assert-Geometry $Label
    Drag-PointSplitter ([int][Math]::Round($before - $minimum))
    $restored = (Need-Node 'points').Current.BoundingRectangle.Width / (Scale)
    if ([Math]::Abs($restored - $before) -gt 2) { throw "Point pane width was not restored: $before -> $restored" }
}
function Cancel-DragByDeactivation {
    Reveal-Point 1
    $source = (Need-Node 'point-drag-1').Current.BoundingRectangle
    $target = (Need-Node 'point-row-3').Current.BoundingRectangle
    $x = [int]($source.Left + $source.Width / 2)
    $y = [int]($source.Top + $source.Height / 2)
    $endX = [int]($target.Left + $target.Width * 0.6)
    $endY = [int]($target.Bottom - 4 * (Scale))
    $form = [Windows.Forms.Form]::new()
    $form.Text = 'Owned point smoke cancellation target'
    $form.StartPosition = [Windows.Forms.FormStartPosition]::Manual
    $windowBounds = $root.Current.BoundingRectangle
    $form.SetBounds([int]($windowBounds.Left + 80 * (Scale)), [int]($windowBounds.Top + 80 * (Scale)), [int](260 * (Scale)), [int](150 * (Scale)))
    $form.ShowInTaskbar = $false
    $form.TopMost = $true
    Foreground-Owned
    Pointer-Move $x $y
    Pointer-Down
    try {
        Move-PointerPath $x $y $endX $endY
        Wait-Until 'point drop indicator before owned-window deactivation' { $null -ne (Node 'point-drop-indicator') }
        $form.Show()
        $form.Activate()
        [ManagerPointOrderingNative]::SetForegroundWindow($form.Handle) | Out-Null
        Wait-Until 'owned temporary form foreground' {
            [Windows.Forms.Application]::DoEvents()
            [ManagerPointOrderingNative]::GetForegroundWindow() -eq $form.Handle
        }
        Pointer-Move ([int]($form.Left + $form.Width / 2)) ([int]($form.Top + $form.Height / 2))
        Start-Sleep -Milliseconds 150
    } finally {
        Pointer-Up
        $form.Close()
        $form.Dispose()
    }
    Foreground-Owned
    # Pointer motion after reactivation must not resurrect a captured drag.
    $viewport = (Need-Node 'points').Current.BoundingRectangle
    Pointer-Move ([int]($viewport.Left + $viewport.Width / 2)) ([int]($viewport.Top + 90 * (Scale)))
    Start-Sleep -Milliseconds 175
    Wait-Until 'deactivated point drag stays cancelled' { $null -eq (Node 'point-drop-indicator') }
    Assert-Order
    Assert-Selected
    Reveal-Point 2
    $ordinary = (Need-Node 'point-2').Current.BoundingRectangle
    Pointer-Move ([int]($ordinary.Left + $ordinary.Width / 2)) ([int]($ordinary.Top + $ordinary.Height / 2))
    Pointer-Down
    try { Start-Sleep -Milliseconds 60 } finally { Pointer-Up }
    Start-Sleep -Milliseconds 175
    $script:selectedPoint = $expected[2]
    Assert-Selected
    Assert-OriginalClean
    Record-Step 'native-cancel-owned-window-deactivation-clean'
}
function Click-Handle([int]$Index) {
    Reveal-Point $Index
    Foreground-Owned
    $r = (Need-Node "point-drag-$Index").Current.BoundingRectangle
    Pointer-Move ([int]($r.Left + $r.Width / 2)) ([int]($r.Top + $r.Height / 2))
    Pointer-Down
    try { Start-Sleep -Milliseconds 60 } finally { Pointer-Up }
    Start-Sleep -Milliseconds 175
}
function Drag-Point([int]$From, [int]$To, [ValidateSet('drop', 'escape', 'outside')][string]$Release = 'drop') {
    Reveal-Point $To
    Reveal-Point $From
    $viewport = (Need-Node 'points').Current.BoundingRectangle
    if (-not (In-Bounds (Need-Node "point-row-$To") $viewport)) { throw "Drag target $To must be visible with source $From" }
    $source = (Need-Node "point-drag-$From").Current.BoundingRectangle
    $target = (Need-Node "point-row-$To").Current.BoundingRectangle
    $x = [int]($source.Left + $source.Width / 2)
    $y = [int]($source.Top + $source.Height / 2)
    $endX = [int]($target.Left + $target.Width * 0.6)
    $endY = if ($From -lt $To) { [int]($target.Bottom - 4 * (Scale)) } elseif ($From -gt $To) { [int]($target.Top + 4 * (Scale)) } else { [int]($target.Top + $target.Height / 2) }
    Foreground-Owned
    Pointer-Move $x $y
    Pointer-Down
    try {
        Move-PointerPath $x $y $endX $endY
        if ($Release -eq 'escape') { [Windows.Forms.SendKeys]::SendWait('{ESC}') }
        # Release in the owned action pane, outside the points viewport.
        if ($Release -eq 'outside') { Pointer-Move ([int]($viewport.Right + 24 * (Scale))) ([int]($viewport.Top + 80 * (Scale))) }
    } finally { Pointer-Up }
    Start-Sleep -Milliseconds 200
}
function Drag-Edge([int]$From, [ValidateSet('up', 'down')][string]$Direction) {
    Reveal-Point $From
    $viewport = (Need-Node 'points').Current.BoundingRectangle
    $visibleBefore = @(for ($i = 0; $i -lt 40; $i++) { if (In-Bounds (Need-Node "point-row-$i") $viewport) { $i } })
    $point = $expected[$From]
    $source = (Need-Node "point-drag-$From").Current.BoundingRectangle
    $x = [int]($source.Left + $source.Width / 2)
    $y = [int]($source.Top + $source.Height / 2)
    $endX = [int]($viewport.Left + $viewport.Width / 2)
    $endY = if ($Direction -eq 'down') { [int]($viewport.Bottom - 3 * (Scale)) } else { [int]($viewport.Top + 3 * (Scale)) }
    $terminal = if ($Direction -eq 'down') { 39 } else { 0 }
    $reached = $false
    Foreground-Owned
    Pointer-Move $x $y
    Pointer-Down
    try {
        Move-PointerPath $x $y $endX $endY
        # No wheel input during drag: reaching the far endpoint proves auto-scroll.
        for ($step = 0; $step -lt 100; $step++) {
            Pointer-Move ($endX + $step % 2) $endY
            Start-Sleep -Milliseconds 75
            if (In-Bounds (Need-Node "point-row-$terminal") (Need-Node 'points').Current.BoundingRectangle) { $reached = $true; break }
        }
        if (-not $reached) { throw "Point drag did not auto-scroll $Direction to endpoint $terminal" }
        Start-Sleep -Milliseconds 150
    } finally { Pointer-Up }
    Start-Sleep -Milliseconds 200
    $to = -1
    for ($i = 0; $i -lt 40; $i++) {
        if ((Need-Node "point-$i").Current.Name.Contains($point.name + '，')) { $to = $i; break }
    }
    if ($to -lt 0 -or ($Direction -eq 'down' -and $to -le $visibleBefore[-1]) -or ($Direction -eq 'up' -and $to -ge $visibleBefore[0])) { throw "Edge drag failed to move beyond initial viewport: $From -> $to ($Direction)" }
    Move-Expected $From $to
    Assert-Order
    Assert-Selected
    Record-Step ('edge-scroll-' + $Direction) @{ from = $From; to = $to; initiallyVisible = $visibleBefore; endpointReached = $reached }
}
function Assert-CacheAndSource {
    $cached = [IO.File]::ReadAllText($cachePath) | ConvertFrom-Json
    if ($cached.format -cne 'route/v1' -or $cached.name -cne 'Point ordering' -or $cached.points.Count -ne 40) { throw 'Wrong cache route/count' }
    $source = [IO.File]::ReadAllText($routePath)
    $previousNameOffset = -1
    for ($i = 0; $i -lt 40; $i++) {
        $actual = $cached.points[$i]
        $wanted = $expected[$i]
        if ($actual.id -ne $wanted.id -or $actual.name -cne $wanted.name -or $actual.note -cne $wanted.note -or @($actual.operations).Count -ne @($wanted.operations).Count) { throw "Persisted point identity/order/content mismatch at $i" }
        for ($j = 0; $j -lt @($wanted.operations).Count; $j++) {
            foreach ($property in $wanted.operations[$j].PSObject.Properties) {
                $found = $actual.operations[$j].PSObject.Properties[$property.Name]
                if ($null -eq $found -or (ConvertTo-Json -InputObject $found.Value -Compress) -cne (ConvertTo-Json -InputObject $property.Value -Compress)) { throw "Persisted point $i action $j property $($property.Name) mismatch" }
            }
        }
        # Names are unique and plain scalars in this fixture. Restart below checks
        # the saved YAML through the application's parser, independently of cache.
        $offset = $source.IndexOf($wanted.name, [StringComparison]::Ordinal)
        if ($offset -le $previousNameOffset) { throw "Saved YAML point name/order mismatch at $i" }
        $previousNameOffset = $offset
    }
}
function Save-Route([bool]$RequireNormalization = $false) {
    Invoke-Name '保存'
    Wait-Until 'save confirmation or completed save' { $null -ne (Named '备份并保存') -or ((Test-Path -LiteralPath $cachePath) -and $null -eq (Named '有未保存修改')) }
    if ($null -ne (Named '备份并保存')) {
        $script:normalizationDialogs++
        Invoke-Name '备份并保存'
    } elseif ($RequireNormalization) { throw 'Initial structured save skipped the normalization confirmation' }
    Wait-Ready
    Wait-Until 'saved cache and clean route' { (Test-Path -LiteralPath $cachePath) -and $null -eq (Named '有未保存修改') -and $null -eq (Node 'confirmation-dialog') }
    Assert-Order
    Assert-Selected
    Assert-CacheAndSource
}

try {
    Start-Transcript -Path (Join-Path $appRoot 'output.log') | Out-Null
    $transcribing = $true
    Write-Output "Isolated point-ordering artifacts: $appRoot"
    Start-Manager
    foreach ($size in @(@(1280, 900), @(900, 640))) {
        Resize-Manager $size[0] $size[1]
        Assert-Geometry "rows-$($size[0])x$($size[1])"
        Assert-MinimumPointPane "rows-$($size[0])x$($size[1])-pane-minimum-210"
        if ($size[0] -eq 900) {
            # Manually open the library too, exercising the narrower editor layout.
            Ensure-Library
            Assert-Geometry 'rows-900x640-library-open'
            Invoke-Name '路线库'
        }
    }
    Resize-Manager 1280 900
    Select-Point $anchor
    Invoke-Id 'operation-toggle-0'
    Assert-Disclosure $anchor @(0)
    Select-Point $otherDuplicate
    Invoke-Id 'operation-toggle-1'
    Assert-Disclosure $otherDuplicate @(1)
    Select-Point $anchor

    # Every clean gesture is checked separately, before any document edit/save.
    Drag-Point 1 1
    Assert-OriginalClean
    Assert-Selected
    Record-Step 'native-no-op-clean'
    foreach ($release in @('escape', 'outside')) {
        Drag-Point 1 3 $release
        Assert-OriginalClean
        Assert-Selected
        Record-Step ('native-cancel-' + $release + '-clean')
    }
    Cancel-DragByDeactivation
    Open-Route 'Other'
    Open-Route 'Ordering'
    Assert-OriginalClean
    Assert-Disclosure $anchor @(0)
    Assert-Disclosure $otherDuplicate @(1)

    # Move the selected point in both directions, then use bottom buttons to
    # verify the selected index followed it, rather than merely its editor text.
    Select-Point $anchor
    $from = $expected.IndexOf($anchor)
    Drag-Point $from ($from + 2)
    Move-Expected $from ($from + 2)
    Assert-Order
    Assert-Selected
    Invoke-Name '上移'
    $from = $expected.IndexOf($anchor)
    Move-Expected $from ($from - 1)
    Assert-Order
    Assert-Selected
    Invoke-Name '下移'
    $from = $expected.IndexOf($anchor)
    Move-Expected $from ($from + 1)
    Assert-Order
    Assert-Selected
    $from = $expected.IndexOf($anchor)
    Drag-Point $from ($from - 2)
    Move-Expected $from ($from - 2)
    Assert-Order
    Assert-Selected
    Assert-Disclosure $anchor @(0)
    Record-Step 'selected-native-down-up-and-bottom-buttons'

    # Keep edits pending while another occurrence of the same ID crosses the
    # selected point. Dragging an unselected handle must never select its point.
    Set-Input 'point_name' '排序点位 04 待提交名称'
    $anchor.name = '排序点位 04 待提交名称'
    Set-Input 'operation-1-name' '待提交等待动作 04-B'
    Set-Input 'operation-1-ms' '987'
    $anchor.operations[1].name = '待提交等待动作 04-B'
    $anchor.operations[1].ms = 987
    $from = $expected.IndexOf($otherDuplicate)
    $to = $expected.IndexOf($anchor) - 1
    Drag-Point $from $to
    Move-Expected $from $to
    Assert-Order
    Assert-Selected
    if ((Input-Value 'operation-1-name') -cne $anchor.operations[1].name -or (Input-Value 'operation-1-ms') -ne '987' -or $null -ne (Node 'operation-0-name')) { throw 'Crossing unselected drag lost pending edits or disclosure' }
    Record-Step 'unselected-duplicate-crosses-selected-with-pending-edits'
    Save-Route $true
    Assert-Disclosure $anchor @(0)
    Assert-Disclosure $otherDuplicate @(1)
    Assert-Disclosure $thirdDuplicate @()

    # A real handle click must select and focus it. Send the first Alt+arrow
    # immediately, before opening metadata could move focus to another control.
    $keyboardPoint = $expected[8]
    Click-Handle 8
    $script:selectedPoint = $keyboardPoint
    Foreground-Owned
    [Windows.Forms.SendKeys]::SendWait('%{DOWN}')
    Start-Sleep -Milliseconds 175
    Move-Expected 8 9
    Assert-Order
    Assert-Selected
    (Need-Node 'point-drag-9').SetFocus()
    [Windows.Forms.SendKeys]::SendWait('%{UP}')
    Start-Sleep -Milliseconds 175
    Move-Expected 9 8
    Assert-Order
    Assert-Selected
    # Focus an unselected handle independently: arrows must move that handle's
    # point while leaving the selected point and pending editor on keyboardPoint.
    $focusedPoint = $expected[10]
    Reveal-Point 10
    (Need-Node 'point-drag-10').SetFocus()
    [Windows.Forms.SendKeys]::SendWait('%{UP}')
    Start-Sleep -Milliseconds 175
    Move-Expected 10 9
    Assert-Order
    Assert-Selected
    Invoke-Name '下移'
    $from = $expected.IndexOf($keyboardPoint)
    Move-Expected $from ($from + 1)
    Assert-Order
    Assert-Selected
    Record-Step 'handle-click-focus-alt-arrows-and-current-selection-button' @{ independentlyFocusedPoint = $focusedPoint.name }
    Save-Route

    Resize-Manager 900 640
    if ($null -ne (Node 'route-Ordering')) { Invoke-Name '路线库' }
    Select-Point $anchor
    $edgePoint = $expected[0]
    Drag-Edge 0 'down'
    Save-Route
    Drag-Edge ($expected.IndexOf($edgePoint)) 'up'
    Save-Route
    Assert-Disclosure $anchor @(0)
    Assert-Disclosure $otherDuplicate @(1)
    Assert-Disclosure $thirdDuplicate @()
    Capture 'point-ordering-after-edge-scroll'

    # Boundary shortcuts are also no-ops and must leave the saved route clean.
    Click-Handle 0
    $script:selectedPoint = $expected[0]
    Foreground-Owned
    [Windows.Forms.SendKeys]::SendWait('%{UP}')
    Start-Sleep -Milliseconds 175
    Assert-Order
    Assert-Selected
    Click-Handle 39
    $script:selectedPoint = $expected[39]
    [Windows.Forms.SendKeys]::SendWait('%{DOWN}')
    Start-Sleep -Milliseconds 175
    Assert-Order
    Assert-Selected
    if ($null -ne (Named '有未保存修改')) { throw 'Boundary keyboard no-op dirtied route' }
    Record-Step 'first-last-keyboard-and-bottom-disabled-states'

    Resize-Manager 1280 900
    Open-Route 'Other'
    Open-Route 'Ordering'
    Assert-Order
    Assert-Disclosure $anchor @(0)
    Assert-Disclosure $otherDuplicate @(1)
    Assert-Disclosure $thirdDuplicate @()
    Wait-Until 'separate manager UI state written' { Test-Path -LiteralPath (Join-Path $appRoot 'state\manager-ui.json') }
    $savedSource = [IO.File]::ReadAllText($routePath)
    $savedTime = [IO.File]::GetLastWriteTimeUtc($routePath)
    Stop-Manager
    Start-Manager
    Assert-Order
    Assert-Disclosure $anchor @(0)
    Assert-Disclosure $otherDuplicate @(1)
    Assert-Disclosure $thirdDuplicate @()
    Assert-CacheAndSource
    if ([IO.File]::ReadAllText($routePath) -cne $savedSource -or [IO.File]::GetLastWriteTimeUtc($routePath) -ne $savedTime -or $null -ne (Named '有未保存修改')) { throw 'Restart/disclosure rewrote or dirtied saved route' }
    if ([IO.File]::ReadAllText($configPath) -cne $originalConfig -or [IO.File]::GetLastWriteTimeUtc($configPath) -ne $originalConfigTime) { throw 'Ordering smoke unexpectedly changed config' }
    Record-Step 'save-cache-yaml-route-switch-and-restart-disclosure'
    Capture 'point-ordering-restarted'
    $result.status = 'passed'
    Write-Output 'Passed: 40 mixed points, duplicate IDs, sibling handles and 36 DIP rows at 1280x900 and 900x640, native up/down/no-op/cancel gestures, pending edits across unselected crossing, selected-index tracking, handle click/focus and Alt+arrows, bottom buttons, both edge auto-scroll directions, save normalization/cache/YAML validation, and disclosure persistence through save/route switch/restart. No playback requested.'
} catch {
    $result.status = 'failed'
    $result.error = $_.ToString()
    if ($null -ne $root) {
        try {
            $tree = @($root.FindAll($scope, [Windows.Automation.Condition]::TrueCondition) | ForEach-Object {
                [pscustomobject]@{ Id = $_.Current.AutomationId; Name = $_.Current.Name; Type = $_.Current.ControlType.ProgrammaticName; Enabled = $_.Current.IsEnabled; Offscreen = $_.Current.IsOffscreen; Focused = $_.Current.HasKeyboardFocus; Bounds = $_.Current.BoundingRectangle.ToString(); Patterns = @($_.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName }) }
            })
            [IO.File]::WriteAllText((Join-Path $appRoot 'failure-tree.json'), (ConvertTo-Json -InputObject $tree -Depth 4), $utf8)
            Capture 'point-ordering-failure'
        } catch { Write-Warning ('Cannot capture failure diagnostics: ' + $_.Exception.Message) }
    }
    throw
} finally {
    # Never discover/stop processes by image name; retain every isolated artifact.
    foreach ($process in $owned) {
        try {
            $process.Refresh()
            if (-not $process.HasExited) {
                $process.CloseMainWindow() | Out-Null
                if (-not $process.WaitForExit(1000)) { $process.Kill(); $process.WaitForExit() }
            }
        } catch { Write-Warning ('Cannot close owned manager PID ' + $process.Id + ': ' + $_.Exception.Message) }
    }
    $result.ownedProcessIds = @($owned | ForEach-Object { $_.Id })
    $result.normalizationDialogs = $normalizationDialogs
    $result.steps = $steps.ToArray()
    $result.expectedPoints = $expected.ToArray()
    [IO.File]::WriteAllText((Join-Path $appRoot 'result.json'), ($result | ConvertTo-Json -Depth 16), $utf8)
    Write-Output "Point-ordering result: $(Join-Path $appRoot 'result.json')"
    if ($transcribing) { Stop-Transcript | Out-Null }
}
