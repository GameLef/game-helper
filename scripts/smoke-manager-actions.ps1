#requires -Version 7.0
param(
    [string]$ProjectRoot = (Split-Path $PSScriptRoot -Parent),
    [string]$Executable = 'target\debug\game-helper.exe',
    [switch]$CaptureScreenshots
)
# Owns one native manager and a unique data root. Never starts a daemon or playback.
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ManagerActionsNative {
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
[ManagerActionsNative]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
$ProjectRoot = [IO.Path]::GetFullPath($ProjectRoot)
$exe = if ([IO.Path]::IsPathRooted($Executable)) { $Executable } else { Join-Path $ProjectRoot $Executable }
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Build the native executable first: $exe" }
$appRoot = Join-Path $ProjectRoot ('.tools\manager-actions-' + [guid]::NewGuid().ToString('N'))
$routes = Join-Path $appRoot 'routes'
[IO.Directory]::CreateDirectory($routes) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $appRoot 'ahk')) | Out-Null
$utf8 = [Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText((Join-Path $appRoot 'ahk\daemon.ahk'), '; Inert action smoke fixture. Never executed.', $utf8)
[IO.File]::WriteAllText((Join-Path $appRoot 'config.json'), '{"routesDir":"routes","autoStartDaemon":false,"autoReloadOnSave":false,"hotkeys":{"stopAll":"F12","bindings":[]}}', $utf8)
$kinds = @('click', 'key', 'press', 'wheel', 'drag', 'wait', 'drag_native')
$expected = [Collections.Generic.List[object]]::new()
for ($i = 0; $i -lt 36; $i++) {
    $kind = $kinds[$i % $kinds.Count]
    $operation = [ordered]@{ type = $kind; name = "预置动作 $($i + 1) $kind" }
    switch ($kind) {
        'click' { $operation.pos = @(0.123456789, 0.876543219); $operation.button = 'right'; $operation.clicks = 2 }
        'key' { $operation.key = 'm'; $operation.up = $true }
        'press' { $operation.key = 'w'; $operation.ms = 800 }
        'wheel' { $operation.delta = -5 }
        'drag' { $operation.from = @(0.1, 0.1); $operation.to = @(0.2, 0.2); $operation.duration = 150 }
        'wait' { $operation.ms = 100 }
        'drag_native' { $operation.from = @(0.3, 0.4); $operation.to = @(0.7, 0.8); $operation.speed = 12; $operation.jitter = 2 }
    }
    $expected.Add([pscustomobject]$operation)
}
$fixture = [ordered]@{
    format = 'route/v1'; name = 'Actions'; note = 'Isolated native action acceptance'
    target = @{ process = 'never-playback.exe' }
    points = @(
        @{ id = 42; name = '需要滚动查看的长动作序列'; note = '这段点位信息应随列表滚出视区，而添加工具栏应始终可见'; operations = $expected.ToArray() },
        @{ id = 7; name = '空点位'; note = ''; operations = @() }
    )
}
# JSON is a YAML subset, so this also exercises loading a noncanonical source.
$routePath = Join-Path $routes 'Actions.yaml'
[IO.File]::WriteAllText($routePath, ($fixture | ConvertTo-Json -Depth 12), $utf8)
$fixture.name = 'Other'
[IO.File]::WriteAllText((Join-Path $routes 'Other.yaml'), ($fixture | ConvertTo-Json -Depth 12), $utf8)
$originalSource = [IO.File]::ReadAllText($routePath)
# The source intentionally omits predrag fields; the form must show the defaults.
foreach ($operation in $expected) {
    if ($operation.type -eq 'drag_native') {
        $operation | Add-Member pre_drag_distance 16
        $operation | Add-Member pre_drag_speed 10
        $operation | Add-Member pre_drag_delay 80
    }
}
$manager = $null
$root = $null
$scope = [Windows.Automation.TreeScope]::Descendants
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()

function Wait-Until([string]$Description, [scriptblock]$Predicate, [int]$TimeoutMs = 8000) {
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
    if ($null -eq $Control) { throw 'Missing action control' }
    if (-not $Control.Current.IsEnabled) { throw ('Disabled action control: ' + $Control.Current.Name) }
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
    } elseif ($Control.Current.ControlType -eq [Windows.Automation.ControlType]::Button) {
        $rect = $Control.Current.BoundingRectangle
        if ($rect.IsEmpty -or $Control.Current.IsOffscreen) { throw 'Cannot click an offscreen native dropdown trigger' }
        [ManagerActionsNative]::SetForegroundWindow($manager.MainWindowHandle) | Out-Null
        [ManagerActionsNative]::SetCursorPos([int]($rect.Left + $rect.Width / 2), [int]($rect.Top + $rect.Height / 2)) | Out-Null
        [ManagerActionsNative]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
        [ManagerActionsNative]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    } else { throw ('Unsupported action control: ' + $Control.Current.Name + ' [' + $Control.Current.AutomationId + ']; type=' + $Control.Current.ControlType.ProgrammaticName + '; patterns=' + ($patterns -join ', ')) }
    Start-Sleep -Milliseconds 125
}
function Invoke-Id([string]$Id) {
    if ($Id -match '^operation-(toggle|drag|insert)-\d+$') { Scroll-To $Id 1 }
    Invoke-Node (Need-Node $Id)
}
function Invoke-Name([string]$Name) { Invoke-Node (Named $Name) }
function Input-Value([string]$Id) {
    (Need-Node $Id).GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value
}
function Set-Input([string]$Id, [string]$Text) {
    (Need-Node $Id).GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($Text)
    Start-Sleep -Milliseconds 100
}
function Wait-Ready {
    Wait-Until 'manager ready' {
        $status = Node 'manager-status'
        $add = Node 'operation-add-menu'
        $null -ne $status -and $status.Current.Name -notmatch '正在加载|正在处理' -and $null -eq (Node 'route-loading-shield') -and $null -ne (Node 'operation-toolbar') -and $null -ne $add -and $add.Current.IsEnabled
    }
}
function Open-Route([string]$Stem) {
    Wait-Until "enabled route $Stem" { $route = Node ('route-' + $Stem); $null -ne $route -and $route.Current.IsEnabled }
    Invoke-Id ('route-' + $Stem)
    if ($null -ne (Node 'confirmation-dialog')) { throw "Unexpected unsaved confirmation while opening $Stem" }
    Wait-Ready
    Wait-Until "first action visible immediately after opening $Stem" {
        $bounds = (Need-Node 'operation-list').Current.BoundingRectangle
        (In-Bounds 'operation-toggle-0' $bounds)
    }
}
function Start-Manager {
    $script:manager = Start-Process -FilePath $exe -ArgumentList @('--app-root', ('"' + $appRoot + '"')) -PassThru
    $owned.Add($manager)
    Wait-Until 'native manager and enabled route library' {
        $manager.Refresh()
        if ($manager.HasExited) { throw 'Owned manager exited before opening its window' }
        if ($manager.MainWindowHandle -eq [IntPtr]::Zero) { return $false }
        try {
            $candidate = [Windows.Automation.AutomationElement]::FromHandle($manager.MainWindowHandle)
            if ($candidate.Current.Name -ne '路线管理') { return $false }
            $script:root = $candidate
            $status = Node 'manager-status'
            $route = Node 'route-Actions'
            $null -ne $status -and $null -ne $route -and $route.Current.IsEnabled -and $status.Current.Name -notmatch '正在加载|正在处理'
        } catch { $false }
    } 15000
    [ManagerActionsNative]::SetForegroundWindow($manager.MainWindowHandle) | Out-Null
    Open-Route 'Actions'
}
function Resize-Manager([int]$Width, [int]$Height) {
    $scale = [ManagerActionsNative]::GetDpiForWindow($manager.MainWindowHandle) / 96.0
    $client = [ManagerActionsNative+Rect]::new()
    $outer = [ManagerActionsNative+Rect]::new()
    if (-not [ManagerActionsNative]::GetClientRect($manager.MainWindowHandle, [ref]$client) -or -not [ManagerActionsNative]::GetWindowRect($manager.MainWindowHandle, [ref]$outer)) { throw 'Cannot read owned manager bounds' }
    $frameWidth = ($outer.Right - $outer.Left) - ($client.Right - $client.Left)
    $frameHeight = ($outer.Bottom - $outer.Top) - ($client.Bottom - $client.Top)
    if (-not [ManagerActionsNative]::MoveWindow($manager.MainWindowHandle, 30, 30, ([int][Math]::Round($Width * $scale) + $frameWidth), ([int][Math]::Round($Height * $scale) + $frameHeight), $true)) { throw 'Cannot resize owned manager' }
    Start-Sleep -Milliseconds 300
    $actual = [ManagerActionsNative+Rect]::new()
    [ManagerActionsNative]::GetClientRect($manager.MainWindowHandle, [ref]$actual) | Out-Null
    if ([Math]::Abs(($actual.Right - $actual.Left) / $scale - $Width) -gt 1 -or [Math]::Abs(($actual.Bottom - $actual.Top) / $scale - $Height) -gt 1) { throw "Client size did not reach ${Width}x${Height}" }
}
function In-Bounds([string]$Id, $Bounds) {
    $control = Node $Id
    if ($null -eq $control -or $control.Current.IsOffscreen) { return $false }
    $rect = $control.Current.BoundingRectangle
    -not $rect.IsEmpty -and $rect.Width -gt 0 -and $rect.Height -gt 0 -and $rect.Left -ge ($Bounds.Left - 1) -and $rect.Right -le ($Bounds.Right + 1) -and $rect.Top -ge ($Bounds.Top - 1) -and $rect.Bottom -le ($Bounds.Bottom + 1)
}
function Assert-InList([string]$Id) {
    $bounds = (Need-Node 'operation-list').Current.BoundingRectangle
    if (-not (In-Bounds $Id $bounds)) { throw "Action control outside scrolling viewport: $Id; viewport=$bounds; control=$((Need-Node $Id).Current.BoundingRectangle)" }
}
function Assert-Toolbar {
    $windowBounds = $root.Current.BoundingRectangle
    $toolbar = (Need-Node 'operation-toolbar').Current.BoundingRectangle
    foreach ($id in @('operation-toolbar', 'operation-toggle-all', 'point-info-toggle', 'operation-add-menu')) {
        if (-not (In-Bounds $id $windowBounds)) { throw "Fixed action control outside window: $id" }
        if ($id -ne 'operation-toolbar' -and -not (In-Bounds $id $toolbar)) { throw "Action control clipped by toolbar: $id" }
    }
    $list = (Need-Node 'operation-list').Current.BoundingRectangle
    if ($list.Top -lt ($toolbar.Bottom - 1) -or $list.Height -lt 80) { throw "Toolbar obscures action viewport or leaves insufficient list space: $toolbar; $list" }
}
function Record-Layout([string]$Label, [bool]$RequireListShare = $true) {
    $toolbar = (Need-Node 'operation-toolbar').Current.BoundingRectangle
    $list = (Need-Node 'operation-list').Current.BoundingRectangle
    $inspectorHeight = $list.Bottom - $toolbar.Top
    $share = $list.Height / $inspectorHeight
    $scale = [ManagerActionsNative]::GetDpiForWindow($manager.MainWindowHandle) / 96.0
    $windowBounds = $root.Current.BoundingRectangle
    $measurements = [ordered]@{ label = $Label; dpi = [int]($scale * 96); actionToolbar = $toolbar.Height / $scale; actionList = $list.Height / $scale }
    foreach ($id in @('service-header', 'workspace-header', 'execution-bar', 'manager-status')) {
        if (-not (In-Bounds $id $windowBounds)) { throw "Workspace chrome outside window: $id" }
        $measurements[$id] = (Need-Node $id).Current.BoundingRectangle.Height / $scale
    }
    foreach ($name in @('保存', '起始点', '试跑选中点', '路线热键', '保存起始点')) {
        $control = Named $name
        if ($null -eq $control) { throw "Missing compact command: $name" }
        $rect = $control.Current.BoundingRectangle
        if ($rect.IsEmpty -or $rect.Left -lt $windowBounds.Left -or $rect.Right -gt $windowBounds.Right -or $rect.Top -lt $windowBounds.Top -or $rect.Bottom -gt $windowBounds.Bottom) { throw "Compact command outside window: $name $rect" }
    }
    foreach ($id in @('hotkey-set', 'hotkey-clear', 'hotkey-current')) {
        if (-not (In-Bounds $id $windowBounds)) { throw "Hotkey command outside window: $id" }
    }
    if ($measurements['workspace-header'] -gt 88 -or $measurements['execution-bar'] -gt 44 -or $measurements['manager-status'] -gt 28 -or $measurements['service-header'] -gt 37) { throw "Workspace chrome exceeds compact height budgets: $($measurements | ConvertTo-Json -Compress)" }
    if ($Label -eq 'manager-default' -and ($measurements.actionToolbar -gt 68 -or $measurements.actionList -lt 598)) { throw 'Default layout did not recover the required 80px of action-list height' }
    if ($Label -eq 'manager-compact' -and $measurements.actionList -lt 306) { throw 'Compact layout did not recover the required 48px of action-list height' }
    [IO.File]::WriteAllText((Join-Path $appRoot ($Label + '-metrics.json')), ($measurements | ConvertTo-Json), $utf8)
    Write-Output ('Layout {0}: toolbar={1:F0}x{2:F0}px; list={3:F0}x{4:F0}px; inspectorHeight={5:F0}px; listShare={6:P1}' -f $Label, $toolbar.Width, $toolbar.Height, $list.Width, $list.Height, $inspectorHeight, $share)
    Write-Output ('Workspace chrome: ' + ($measurements | ConvertTo-Json -Compress))
    if ($RequireListShare -and $share -lt 0.60) { throw "Action list occupies less than 60% of inspector height: $Label ($share)" }
}
function Capture-Layout([string]$Label) {
    if (-not $CaptureScreenshots) { return }
    [ManagerActionsNative]::SetForegroundWindow($manager.MainWindowHandle) | Out-Null
    Start-Sleep -Milliseconds 150
    $bounds = $root.Current.BoundingRectangle
    if ($bounds.IsEmpty) { throw 'Cannot capture empty manager window' }
    $bitmap = [Drawing.Bitmap]::new([int]$bounds.Width, [int]$bounds.Height)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $path = Join-Path $appRoot ($Label + '.png')
    try {
        $graphics.CopyFromScreen([int]$bounds.Left, [int]$bounds.Top, 0, 0, $bitmap.Size)
        $bitmap.Save($path, [Drawing.Imaging.ImageFormat]::Png)
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
    Write-Output "Native screenshot: $path"
}
function Wheel([int]$Delta) {
    [ManagerActionsNative]::SetForegroundWindow($manager.MainWindowHandle) | Out-Null
    Wait-Until 'owned manager foreground' { [ManagerActionsNative]::GetForegroundWindow() -eq $manager.MainWindowHandle }
    $bounds = (Need-Node 'operation-list').Current.BoundingRectangle
    [ManagerActionsNative]::SetCursorPos([int]($bounds.Right - 25), [int]($bounds.Top + $bounds.Height / 2)) | Out-Null
    [ManagerActionsNative]::mouse_event(0x0800, 0, 0, $Delta, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 60
}
function Scroll-To([string]$Id, [int]$Direction) {
    $previous = ''
    $stagnant = 0
    for ($step = 0; $step -lt 80; $step++) {
        $bounds = (Need-Node 'operation-list').Current.BoundingRectangle
        if (In-Bounds $Id $bounds) { return }
        $rect = (Need-Node $Id).Current.BoundingRectangle
        $signature = "$rect;$bounds"
        $stagnant = if ($signature -ceq $previous) { $stagnant + 1 } else { 0 }
        if ($stagnant -ge 4) { throw "Scrolling stalled before $Id; target=$rect; viewport=$bounds" }
        $previous = $signature
        $distance = if (-not $rect.IsEmpty -and $rect.Top -lt $bounds.Top) { $rect.Top - $bounds.Top } elseif (-not $rect.IsEmpty) { $rect.Bottom - $bounds.Bottom } else { -$Direction * $bounds.Height * 2 }
        $sign = if ($distance -lt 0) { 1 } else { -1 }
        $stepSize = if ([Math]::Abs($distance) -gt $bounds.Height) { 1200 } else { 120 }
        Wheel ($sign * $stepSize)
    }
    throw "Could not scroll to $Id"
}
function Expanded-Names {
    @($root.FindAll($scope, [Windows.Automation.Condition]::TrueCondition) | Where-Object { $_.Current.AutomationId -match '^operation-\d+-name$' })
}
function Set-AllExpanded([bool]$Expanded) {
    $count = @(Expanded-Names).Count
    if (-not $Expanded) {
        if ($count -gt 0) { Invoke-Id 'operation-toggle-all' }
    } elseif ($count -ne $expected.Count) {
        if ($count -gt 0) { Invoke-Id 'operation-toggle-all' }
        Invoke-Id 'operation-toggle-all'
    }
    $wanted = if ($Expanded) { $expected.Count } else { 0 }
    Wait-Until "expanded action input count $wanted" { @(Expanded-Names).Count -eq $wanted }
}
function Save-Route {
    Invoke-Name '保存'
    if ($null -ne (Named '备份并保存')) { Invoke-Name '备份并保存' }
    Wait-Ready
    Wait-Until 'cache written and clean route' {
        (Test-Path -LiteralPath (Join-Path $routes '_cache\Actions.json')) -and $null -eq (Named '有未保存修改') -and $null -eq (Node 'confirmation-dialog')
    }
}
function Assert-Cache {
    $cached = [IO.File]::ReadAllText((Join-Path $routes '_cache\Actions.json')) | ConvertFrom-Json
    if ($cached.format -cne 'route/v1' -or $cached.points[0].id -ne 42 -or $cached.points[1].id -ne 7) { throw 'Route or point identity changed' }
    if (@($cached.points[0].operations).Count -ne $expected.Count -or @($cached.points[1].operations).Count -ne 0) { throw 'Unexpected action count or cross-point mutation' }
    for ($i = 0; $i -lt $expected.Count; $i++) {
        foreach ($property in $expected[$i].PSObject.Properties) {
            $actualProperty = $cached.points[0].operations[$i].PSObject.Properties[$property.Name]
            if ($null -eq $actualProperty) { throw "Missing persisted operations[$i].$($property.Name)" }
            $actual = ConvertTo-Json -InputObject $actualProperty.Value -Depth 8 -Compress
            $wanted = ConvertTo-Json -InputObject $property.Value -Depth 8 -Compress
            if ($actual -cne $wanted) { throw "Persisted operations[$i].$($property.Name) mismatch: actual=$actual expected=$wanted" }
        }
    }
}
function Assert-FirstNames {
    for ($i = 0; $i -lt $kinds.Count; $i++) {
        if ((Input-Value "operation-$i-name") -cne $expected[$i].name) { throw "Action name $i lost after rebuilding controls" }
    }
    if ((Input-Value 'operation-0-clicks') -ne '3') { throw 'Edited click parameter lost' }
    for ($i = 0; $i -lt $expected.Count; $i++) {
        if ($expected[$i].type -eq 'drag_native') {
            foreach ($key in @('speed','pre_drag_distance','pre_drag_speed','pre_drag_delay')) {
                if ((Input-Value "operation-$i-$key") -ne [string]$expected[$i].$key) {
                    throw "Native drag $key at $i lost after rebuilding controls"
                }
            }
        }
    }
}
function Action-Button([string]$Prefix, [int]$Number) {
    Scroll-To ('operation-toggle-' + ($Number - 1)) 1
    $pattern = '^' + [regex]::Escape($Prefix + $Number) + '(?:\s|$)'
    $buttons = @($root.FindAll($scope, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Button)) | Where-Object { $_.Current.Name -match $pattern })
    if ($buttons.Count -ne 1) { throw "Expected one action button: $Prefix$Number; found $($buttons.Count)" }
    $buttons[0]
}
function Add-Operation([string]$Kind, [string]$Entry = 'operation-add-menu') {
    Invoke-Id $Entry
    Invoke-Id ('operation-add-' + $Kind)
}
function Move-Operation([int]$From, [int]$To) {
    Scroll-To "operation-drag-$From" 1
    Invoke-Id "operation-drag-$From"
    Set-Input 'operation-move-position' ([string]($To + 1))
    Invoke-Id 'operation-move-confirm'
}
function Drag-Operation([int]$From, [int]$To, [string]$Release = 'drop', [int]$EdgeHold = 0) {
    Scroll-To "operation-drag-$From" 1
    $source = (Need-Node "operation-drag-$From").Current.BoundingRectangle
    $target = (Need-Node "operation-panel-$To").Current.BoundingRectangle
    $viewport = (Need-Node 'operation-list').Current.BoundingRectangle
    $x = [int]($source.Left + $source.Width / 2)
    $y = [int]($source.Top + $source.Height / 2)
    $endX = [int]($viewport.Left + $viewport.Width / 2)
    $endY = if ($From -lt $To) { [int]([Math]::Min($target.Bottom - 3, $viewport.Bottom - 18)) } else { [int]([Math]::Max($target.Top + 3, $viewport.Top + 18)) }
    if ($EdgeHold -gt 0) { $endY = [int]($viewport.Bottom - 2) }
    [ManagerActionsNative]::SetForegroundWindow($manager.MainWindowHandle) | Out-Null
    [ManagerActionsNative]::SetCursorPos($x, $y) | Out-Null
    [ManagerActionsNative]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    try {
        for ($step = 1; $step -le 16; $step++) {
            [ManagerActionsNative]::SetCursorPos([int]($x + ($endX - $x) * $step / 16), [int]($y + ($endY - $y) * $step / 16)) | Out-Null
            Start-Sleep -Milliseconds 25
        }
        if ($EdgeHold -gt 0) { Start-Sleep -Milliseconds $EdgeHold }
        if ($Release -eq 'escape') { [Windows.Forms.SendKeys]::SendWait('{ESC}') }
        if ($Release -eq 'outside') { [ManagerActionsNative]::SetCursorPos([int]($viewport.Left - 20), [int]($viewport.Top + 80)) | Out-Null }
    } finally { [ManagerActionsNative]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero) }
    Start-Sleep -Milliseconds 200
}
function Assert-MixedDisclosure {
    if ($null -ne (Node 'operation-0-name') -or $null -eq (Node 'operation-1-name')) { throw 'Mixed disclosure state was not restored' }
}
try {
    Write-Output "Isolated manager action sandbox: $appRoot"
    Start-Manager
    Resize-Manager 1280 900
    if ($null -ne (Node 'point_name') -or $null -ne (Node 'point_note')) { throw 'Point metadata should start collapsed' }
    Assert-Toolbar
    Capture-Layout 'manager-startup'
    Invoke-Id 'operation-toggle-0'
    Wait-Until 'first action collapsed' { $null -eq (Node 'operation-0-name') }
    Invoke-Id 'operation-toggle-0'
    Wait-Until 'first action expanded' { $null -ne (Node 'operation-0-name') }
    Set-AllExpanded $false
    Set-AllExpanded $true
    Invoke-Id 'point-info-toggle'
    Assert-InList 'point_name'
    Invoke-Id 'point-info-toggle'
    if ($null -ne (Named '有未保存修改') -or [IO.File]::ReadAllText($routePath) -cne $originalSource) { throw 'Disclosure controls dirtied or wrote the route' }
    $originalRouteTime = [IO.File]::GetLastWriteTimeUtc($routePath)
    $configPath = Join-Path $appRoot 'config.json'
    $originalConfig = [IO.File]::ReadAllText($configPath)
    $originalConfigTime = [IO.File]::GetLastWriteTimeUtc($configPath)
    Invoke-Id 'operation-toggle-0'
    Assert-MixedDisclosure
    Invoke-Id 'point-1'
    Invoke-Id 'point-0'
    Assert-MixedDisclosure
    Open-Route 'Other'
    Open-Route 'Actions'
    Assert-MixedDisclosure
    Wait-Until 'separate UI state created' { Test-Path -LiteralPath (Join-Path $appRoot 'state\manager-ui.json') }
    $manager.CloseMainWindow() | Out-Null
    if (-not $manager.WaitForExit(5000)) { throw 'UI state flush did not close clean manager' }
    Start-Manager
    Assert-MixedDisclosure
    if ([IO.File]::ReadAllText($routePath) -cne $originalSource -or [IO.File]::GetLastWriteTimeUtc($routePath) -ne $originalRouteTime) { throw 'Disclosure wrote route contents or mtime' }
    if ([IO.File]::ReadAllText($configPath) -cne $originalConfig -or [IO.File]::GetLastWriteTimeUtc($configPath) -ne $originalConfigTime) { throw 'Disclosure wrote config contents or mtime' }
    if (Test-Path -LiteralPath (Join-Path $routes '_cache\Actions.json')) { throw 'Disclosure generated execution cache' }
    Set-AllExpanded $true

    foreach ($size in @(@(1280, 900), @(900, 640))) {
        Resize-Manager $size[0] $size[1]
        # In compact mode the library must be hidden so the editor gets its intended width.
        if ($size[0] -eq 900 -and $null -ne (Node 'route-Actions')) { Invoke-Name '路线库' }
        Assert-Toolbar
        $label = if ($size[0] -eq 900) { 'manager-compact' } else { 'manager-default' }
        Record-Layout $label
        Scroll-To 'operation-toggle-0' 1
        Capture-Layout $label
        if ($size[0] -eq 900) {
            Invoke-Name '路线库'
            Assert-Toolbar
            Record-Layout 'manager-compact-library-open' $false
            Capture-Layout 'manager-compact-library-open'
            Invoke-Name '路线库'
            Assert-Toolbar
        }
        $before = (Node 'operation-toolbar').Current.BoundingRectangle
        Invoke-Id 'point-info-toggle'
        Scroll-To 'point_name' 1
        Assert-InList 'point_name'
        Scroll-To 'operation-35-name' -1
        $after = (Node 'operation-toolbar').Current.BoundingRectangle
        if ([Math]::Abs($before.Top - $after.Top) -gt 1 -or [Math]::Abs($before.Bottom - $after.Bottom) -gt 1) { throw 'Adding toolbar moved while scrolling the action list' }
        Assert-Toolbar
        if (In-Bounds 'point_name' (Node 'operation-list').Current.BoundingRectangle) { throw 'Expanded point metadata did not scroll away' }
        Invoke-Id 'point-info-toggle'
        if ($null -ne (Node 'point_name') -or $null -ne (Node 'point_note')) { throw 'Collapsed point metadata remains rendered' }
    }

    # Add every kind while at the end of the long list at minimum client size.
    foreach ($kind in $kinds) {
        $index = $expected.Count
        Add-Operation $kind
        Assert-Toolbar
        Assert-InList "operation-toggle-$index"
        Assert-InList "operation-$index-name"
        $name = "新增中文动作 $kind"
        Set-Input "operation-$index-name" $name
        $operation = [ordered]@{ type = $kind; name = $name }
        switch ($kind) {
            'click' { $operation.pos = @(0.123456789, 0.876543219); $operation.button = 'left'; $operation.clicks = 1 }
            'key' { $operation.key = 'w'; $operation.up = $false }
            'press' { $operation.key = 'w'; $operation.ms = 500 }
            'wheel' { $operation.delta = 1 }
            'drag' { $operation.from = @(0.123456789, 0.876543219); $operation.to = @(0.123456789, 0.876543219); $operation.duration = 200 }
            'wait' { $operation.ms = 100 }
            'drag_native' {
                $operation.from = @(0.123456789, 0.876543219); $operation.to = @(0.123456789, 0.876543219); $operation.speed = 2
                if ((Input-Value "operation-$index-speed") -ne '2') { throw 'New native drag must default to speed 2' }
                Set-Input "operation-$index-speed" '37'
                $operation.speed = 37
                if ((Input-Value "operation-$index-speed") -ne '37') { throw 'New native drag speed edit was not applied' }
                foreach ($setting in @(
                    @{key='pre_drag_distance';default=16;edited=24},
                    @{key='pre_drag_speed';default=10;edited=7},
                    @{key='pre_drag_delay';default=80;edited=120}
                )) {
                    if ((Input-Value "operation-$index-$($setting.key)") -ne [string]$setting.default) { throw "Wrong predrag default: $($setting.key)" }
                    Set-Input "operation-$index-$($setting.key)" ([string]$setting.edited)
                    $operation[$setting.key] = $setting.edited
                }
            }
        }
        $expected.Add([pscustomobject]$operation)
    }

    Resize-Manager 1280 900
    Set-AllExpanded $true
    Scroll-To 'operation-toggle-0' 1
    $names = @('命名动作一 "打开仓库"', "命名动作二 '地图'", '命名动作三 按住前进', '命名动作四 向下滚动', '命名动作五 拖到目标', '命名动作六 等待加载 🦀', '命名动作七 原生拖动')
    for ($i = 0; $i -lt $kinds.Count; $i++) {
        Set-Input "operation-$i-name" $names[$i]
        $expected[$i].name = $names[$i]
    }
    Set-Input 'operation-0-clicks' '3'
    $expected[0].clicks = 3
    Set-Input 'operation-6-pre_drag_distance' '0'
    $expected[6].pre_drag_distance = 0
    Scroll-To 'operation-toggle-0' 1
    Invoke-Id 'operation-toggle-0'
    Wait-Until 'edited action collapsed before moving' { $null -eq (Node 'operation-0-name') }
    Drag-Operation 0 1
    $first = $expected[0]; $expected[0] = $expected[1]; $expected[1] = $first
    Wait-Until 'moved collapsed action identity and name' { $null -eq (Node 'operation-1-name') -and (Input-Value 'operation-0-name') -ceq $names[1] }
    Move-Operation 1 0
    $first = $expected[0]; $expected[0] = $expected[1]; $expected[1] = $first
    if ($null -ne (Node 'operation-0-name')) { throw 'Collapsed state did not follow action back upward' }
    Invoke-Id 'operation-toggle-0'
    Assert-FirstNames
    Scroll-To 'operation-toggle-7' -1
    Invoke-Id 'operation-toggle-7'
    Invoke-Node (Action-Button '删除动作 ' 8)
    $expected.RemoveAt(7)
    Assert-FirstNames
    Save-Route
    Assert-Cache

    # Insert immediately below a collapsed action; unrelated states must stay intact.
    Scroll-To 'operation-toggle-0' 1
    Invoke-Id 'operation-toggle-0'
    Add-Operation 'wait' 'operation-insert-0'
    Set-Input 'operation-1-name' '中间插入的等待'
    $expected.Insert(1, [pscustomobject]@{type='wait'; name='中间插入的等待'; ms=100})
    if ($null -ne (Node 'operation-0-name')) { throw 'Insertion expanded unrelated collapsed action' }
    Save-Route
    Assert-Cache
    Invoke-Node (Action-Button '删除动作 ' 2)
    $expected.RemoveAt(1)
    if ($null -ne (Node 'operation-0-name')) { throw 'Deleting middle insertion expanded original' }
    Scroll-To 'operation-toggle-0' 1
    Invoke-Id 'operation-toggle-0'
    if ($null -eq (Node 'operation-0-name')) { throw 'Explicit expand after middle delete failed' }

    # Copy is independent and inherits disclosure states; remove it after checking persistence.
    Scroll-To 'operation-toggle-0' 1
    if ($null -eq (Node 'operation-0-name')) { throw 'Original action should be expanded after deleting inserted action' }
    Invoke-Id 'operation-toggle-0'
    Assert-MixedDisclosure
    Invoke-Id 'copy-point'
    Assert-MixedDisclosure
    Invoke-Id 'operation-toggle-0'
    Set-Input 'operation-0-name' '仅修改副本的动作'
    Save-Route
    $copyCache = [IO.File]::ReadAllText((Join-Path $routes '_cache\Actions.json')) | ConvertFrom-Json
    if ($copyCache.points.Count -ne 3 -or $copyCache.points[1].id -in @(42,7) -or $copyCache.points[1].name -notmatch '副本' -or $copyCache.points[0].operations[0].name -cne $expected[0].name -or $copyCache.points[1].operations[0].name -cne '仅修改副本的动作') { throw 'Copied point identity or operation independence failed' }
    Invoke-Id 'point-0'
    Assert-MixedDisclosure
    Invoke-Id 'point-1'
    if ($null -eq (Node 'operation-0-name')) { throw 'Copy and original share disclosure state' }
    Invoke-Name '删除'
    Invoke-Id 'point-0'
    Invoke-Id 'operation-toggle-0'
    Save-Route
    Assert-Cache
    Capture-Layout 'manager-new-actions'

    # Actual cancel and edge-scroll gestures on collapsed variable-length lists.
    Set-AllExpanded $false
    $beforeCancel = [IO.File]::ReadAllText($routePath)
    Drag-Operation 0 1 'escape'
    Drag-Operation 0 1 'outside'
    if ([IO.File]::ReadAllText($routePath) -cne $beforeCancel -or $null -ne (Named '有未保存修改')) { throw 'Cancelled drag changed the route' }
    Drag-Operation 0 1 'drop' 950
    Save-Route
    $edgeCache = [IO.File]::ReadAllText((Join-Path $routes '_cache\Actions.json')) | ConvertFrom-Json
    $movedIndex = -1
    for ($i = 0; $i -lt $edgeCache.points[0].operations.Count; $i++) { if ($edgeCache.points[0].operations[$i].name -ceq $expected[0].name) { $movedIndex = $i; break } }
    if ($movedIndex -lt 3) { throw "Edge auto-scroll did not move source beyond initial rows: $movedIndex" }
    $moved = $expected[0]; $expected.RemoveAt(0); $expected.Insert($movedIndex, $moved)
    Assert-Cache
    if ($null -ne (Node "operation-$movedIndex-name")) { throw 'Collapsed state was lost during edge scrolling' }
    Move-Operation $movedIndex 0
    $expected.RemoveAt($movedIndex); $expected.Insert(0, $moved)
    Save-Route
    Assert-Cache
    Set-AllExpanded $true

    Invoke-Name 'YAML 源码'
    $source = $root.FindFirst($scope, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, '路线 YAML 源码'))
    if ($null -eq $source) { throw 'YAML source editor missing' }
    $yaml = $source.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value
    foreach ($prefix in @('命名动作一', '命名动作二', '命名动作三', '命名动作四', '命名动作五', '命名动作六', '命名动作七')) {
        if (-not $yaml.Contains($prefix)) { throw "Named action missing from YAML: $prefix" }
    }
    Invoke-Name '点位编辑'
    Set-AllExpanded $true
    Assert-FirstNames
    Invoke-Id 'point-1'
    if ($null -ne (Node 'operation-0-name')) { throw 'Empty point inherited action controls' }
    Add-Operation 'wait'
    Wait-Until 'sole action added' { $null -ne (Node 'operation-0-name') }
    [ManagerActionsNative]::SetForegroundWindow($manager.MainWindowHandle) | Out-Null
    Wait-Until 'manager foreground before keyboard delete' { [ManagerActionsNative]::GetForegroundWindow() -eq $manager.MainWindowHandle }
    Invoke-Node (Action-Button '删除动作 ' 1)
    Wait-Until 'sole action removed' {
        $null -eq (Node 'operation-0-name') -and $null -eq (Node 'operation-toggle-0')
    }
    # Activate the restored target with the keyboard; this verifies actual focus
    # behavior even when the UIA provider reports HasKeyboardFocus=false.
    [Windows.Forms.SendKeys]::SendWait('{ENTER}')
    Wait-Until 'keyboard activation after deleting sole action' { $null -ne (Node 'point_name') }
    Assert-InList 'point_name'
    Invoke-Id 'point-info-toggle'
    Invoke-Id 'point-0'
    Set-AllExpanded $true
    Assert-FirstNames
    Save-Route
    Assert-Cache
    # The compact manual library choice persists; reopen it only if necessary.
    if ($null -eq (Node 'route-Other')) { Invoke-Name '路线库' }
    Open-Route 'Other'
    Open-Route 'Actions'
    Set-AllExpanded $true
    Assert-FirstNames
    $manager.CloseMainWindow() | Out-Null
    if (-not $manager.WaitForExit(5000)) { throw 'Clean manager did not close before restart' }
    Start-Manager
    if ($null -ne (Node 'point_name')) { throw 'Point metadata did not reset collapsed after restart' }
    Set-AllExpanded $true
    Assert-FirstNames
    Assert-Cache
    # External corruption is preserved and exposed separately from route saving.
    $uiPath = Join-Path $appRoot 'state\manager-ui.json'
    [IO.File]::WriteAllText($uiPath, '{broken json', $utf8)
    Invoke-Id 'operation-toggle-0'
    Wait-Until 'UI persistence conflict warning' { $null -ne (Node 'manager-ui-warning') }
    Invoke-Id 'manager-ui-retry'
    Wait-Until 'corrupt-state retry fails without replacement' { (Node 'manager-ui-warning').Current.Name -match '重试失败' }
    if ([IO.File]::ReadAllText($uiPath) -cne '{broken json') { throw 'Corrupt UI state file was overwritten' }
    $manager.CloseMainWindow() | Out-Null
    Wait-Until 'failed state close fallback' { $null -ne (Node 'manager-ui-close-anyway') }
    Invoke-Id 'manager-ui-close-anyway'
    if (-not $manager.WaitForExit(5000)) { throw 'Explicit close-anyway failed' }
    Write-Output 'Passed: 36 mixed actions, clean disclosure controls, metadata scroll/collapse, compact add menus with all seven kinds including native drag at 1280x900 and 900x640, adding every kind at list end, native speed default 2 edited to 37, Chinese action names and parameters, actual native drag + move dialog, middle insertion, independent point copying, disclosure persistence through point/route switches and process restart, no route/config/cache writes for disclosure. No playback requested.'
} catch {
    if ($null -ne $root) {
        try {
            $tree = @($root.FindAll($scope, [Windows.Automation.Condition]::TrueCondition) | ForEach-Object {
                [pscustomobject]@{ Id = $_.Current.AutomationId; Name = $_.Current.Name; ControlType = $_.Current.ControlType.ProgrammaticName; Patterns = @($_.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName }); Focused = $_.Current.HasKeyboardFocus; Enabled = $_.Current.IsEnabled; Offscreen = $_.Current.IsOffscreen; Bounds = $_.Current.BoundingRectangle.ToString() }
            })
            [IO.File]::WriteAllText((Join-Path $appRoot 'failure-tree.json'), (ConvertTo-Json -InputObject $tree -Depth 4), $utf8)
        } catch { Write-Warning ('Could not write diagnostic accessibility tree: ' + $_.Exception.Message) }
    }
    throw
} finally {
    foreach ($process in $owned) {
        $process.Refresh()
        if (-not $process.HasExited) {
            $process.CloseMainWindow() | Out-Null
            if (-not $process.WaitForExit(1000)) { $process.Kill(); $process.WaitForExit() }
        }
    }
}
