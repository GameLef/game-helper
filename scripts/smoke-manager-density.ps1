#requires -Version 7.0
param(
    [string]$ProjectRoot = (Split-Path $PSScriptRoot -Parent),
    [string]$Executable = 'target\release\game-helper.exe'
)
# Owns one native manager and unique data root. No daemon, playback or game input.
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ManagerDensityNative {
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
[ManagerDensityNative]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
$ProjectRoot = [IO.Path]::GetFullPath($ProjectRoot)
$exe = if ([IO.Path]::IsPathRooted($Executable)) { $Executable } else { Join-Path $ProjectRoot $Executable }
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Build the native executable first: $exe" }
$appRoot = Join-Path $ProjectRoot ('.tools\manager-density-' + [guid]::NewGuid().ToString('N'))
$routes = Join-Path $appRoot 'routes'
[IO.Directory]::CreateDirectory($routes) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $appRoot 'ahk')) | Out-Null
$utf8 = [Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText((Join-Path $appRoot 'ahk\daemon.ahk'), '; Inert density smoke fixture; never executed.', $utf8)
[IO.File]::WriteAllText((Join-Path $appRoot 'config.json'), '{"routesDir":"routes","autoStartDaemon":false,"autoReloadOnSave":false,"hotkeys":{"stopAll":"F12","bindings":[]}}', $utf8)
$longStem = 'Density_' + ('long_' * 18)
$longRoute = '用于检查工具栏省略和保存按钮可见性的长路线名称' * 12
$longPoint = '用于检查动作工具栏换行的长点位名称' * 14
$errorPoint = '错误信息必须完整保留' * 90 + '完整错误末尾标记'
$manager = $null
$root = $null
$scope = [Windows.Automation.TreeScope]::Descendants

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
    } elseif ($patterns -contains 'TogglePatternIdentifiers.Pattern') {
        $Control.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    } elseif ($patterns -contains 'SelectionItemPatternIdentifiers.Pattern') {
        $Control.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    } elseif ($patterns -contains 'ExpandCollapsePatternIdentifiers.Pattern') {
        $pattern = $Control.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern)
        if ($pattern.Current.ExpandCollapseState -eq [Windows.Automation.ExpandCollapseState]::Collapsed) { $pattern.Expand() } else { $pattern.Collapse() }
    } else { throw ('Unsupported invocation target: ' + $Control.Current.Name + '; ' + ($patterns -join ', ')) }
    Start-Sleep -Milliseconds 125
}
function Invoke-Name([string]$Name) { Invoke-Node (Named $Name) }
function Invoke-Id([string]$Id) { Invoke-Node (Need-Node $Id) }
function Wait-Ready {
    Wait-Until 'manager ready' {
        $status = Node 'manager-status'
        $null -ne $status -and $status.Current.Name -notmatch '正在加载|正在处理' -and $null -eq (Node 'route-loading-shield')
    }
}
function Open-Route([string]$Stem) {
    Invoke-Id ('route-' + $Stem)
    Wait-Ready
    if ($null -ne (Node 'confirmation-dialog')) { throw "Unexpected dirty confirmation when opening $Stem" }
    Need-Node 'workspace-header' | Out-Null
}
function Scale { [ManagerDensityNative]::GetDpiForWindow($manager.MainWindowHandle) / 96.0 }
function Resize-Manager([int]$Width, [int]$Height) {
    $scale = Scale
    $client = [ManagerDensityNative+Rect]::new()
    $outer = [ManagerDensityNative+Rect]::new()
    if (-not [ManagerDensityNative]::GetClientRect($manager.MainWindowHandle, [ref]$client) -or -not [ManagerDensityNative]::GetWindowRect($manager.MainWindowHandle, [ref]$outer)) { throw 'Cannot read owned manager bounds' }
    $frameWidth = ($outer.Right - $outer.Left) - ($client.Right - $client.Left)
    $frameHeight = ($outer.Bottom - $outer.Top) - ($client.Bottom - $client.Top)
    if (-not [ManagerDensityNative]::MoveWindow($manager.MainWindowHandle, 25, 25, ([int][Math]::Round($Width * $scale) + $frameWidth), ([int][Math]::Round($Height * $scale) + $frameHeight), $true)) { throw 'Cannot resize owned manager' }
    Start-Sleep -Milliseconds 300
    [ManagerDensityNative]::GetClientRect($manager.MainWindowHandle, [ref]$client) | Out-Null
    if ([Math]::Abs(($client.Right - $client.Left) / $scale - $Width) -gt 1 -or [Math]::Abs(($client.Bottom - $client.Top) / $scale - $Height) -gt 1) { throw "Client size did not reach ${Width}x${Height}" }
}
function In-Bounds($Control, $Bounds) {
    if ($null -eq $Control -or $Control.Current.IsOffscreen) { return $false }
    $r = $Control.Current.BoundingRectangle
    -not $r.IsEmpty -and $r.Width -gt 0 -and $r.Height -gt 0 -and $r.Left -ge ($Bounds.Left - 1) -and $r.Right -le ($Bounds.Right + 1) -and $r.Top -ge ($Bounds.Top - 1) -and $r.Bottom -le ($Bounds.Bottom + 1)
}
function Assert-NamedVisible([string]$Name) {
    if (-not (In-Bounds (Named $Name) $root.Current.BoundingRectangle)) { throw "Control outside owned window: $Name" }
}
function Assert-Layout([string]$Label, [bool]$DisclosureOpen = $false, [bool]$Welcome = $false) {
    $bounds = $root.Current.BoundingRectangle
    $scale = Scale
    $metrics = [ordered]@{ label = $Label; dpi = [int]($scale * 96) }
    foreach ($id in @('service-header', 'execution-bar', 'manager-status')) {
        $control = Need-Node $id
        if (-not (In-Bounds $control $bounds)) { throw "Workspace chrome clipped: $id" }
        $metrics[$id] = $control.Current.BoundingRectangle.Height / $scale
    }
    if ($metrics['service-header'] -gt 37 -or $metrics['execution-bar'] -gt 44 -or $metrics['manager-status'] -gt 28) { throw "Chrome height budget exceeded: $($metrics | ConvertTo-Json -Compress)" }
    if (-not $Welcome) {
        $header = Need-Node 'workspace-header'
        if (-not (In-Bounds $header $bounds)) { throw 'Route header clipped' }
        $metrics['workspace-header'] = $header.Current.BoundingRectangle.Height / $scale
        if (-not $DisclosureOpen -and $metrics['workspace-header'] -gt 88) { throw 'Normal route header exceeds 88 DIP' }
        foreach ($name in @('保存', '点位编辑', 'YAML 源码', '路线属性', '文件操作', '起始点', '路线热键', '保存绑定', '试跑选中点')) { Assert-NamedVisible $name }
        foreach ($pair in @(@('start-field', '试跑选中点'), @('hotkey-binding-field', '保存绑定'))) {
            $inputBounds = (Need-Node $pair[0]).Current.BoundingRectangle
            $buttonBounds = (Named $pair[1]).Current.BoundingRectangle
            $metrics[$pair[0] + '-height'] = $inputBounds.Height / $scale
            if ([Math]::Abs($inputBounds.Height - $buttonBounds.Height) / $scale -gt 1) { throw "Execution input/button height mismatch: $($pair[0])" }
        }
        $fileBounds = (Need-Node 'route-context-file').Current.BoundingRectangle
        $savedBounds = (Need-Node 'route-save-state').Current.BoundingRectangle
        $metrics['file-state-gap'] = ($savedBounds.Left - $fileBounds.Right) / $scale
        if ([Math]::Abs($metrics['file-state-gap'] - 8) -gt 1 -or [Math]::Abs($savedBounds.Top - $fileBounds.Top) / $scale -gt 1) { throw 'Save state must follow filename with an 8 DIP gap on the same line' }
        if (-not (In-Bounds (Node 'route-save-state') $header.Current.BoundingRectangle)) { throw 'Save state clipped' }
    }
    if ($null -ne (Node 'route-properties')) {
        $panel = Node 'route-properties'
        if (-not (In-Bounds $panel $bounds)) { throw 'Expanded route properties clipped' }
        $metrics['route-properties'] = $panel.Current.BoundingRectangle.Height / $scale
        if ($metrics['route-properties'] -gt 160.5) { throw 'Expanded properties exceed 160 DIP' }
    }
    if ($null -ne (Node 'operation-list')) { $metrics['operation-list'] = (Node 'operation-list').Current.BoundingRectangle.Height / $scale }
    [IO.File]::WriteAllText((Join-Path $appRoot ($Label + '-metrics.json')), ($metrics | ConvertTo-Json), $utf8)
    Write-Output ($metrics | ConvertTo-Json -Compress)
}
function Foreground-Owned {
    [ManagerDensityNative]::SetForegroundWindow($manager.MainWindowHandle) | Out-Null
    Wait-Until 'owned manager foreground' { [ManagerDensityNative]::GetForegroundWindow() -eq $manager.MainWindowHandle }
}
function Capture([string]$Label) {
    Foreground-Owned
    Start-Sleep -Milliseconds 175
    $bounds = $root.Current.BoundingRectangle
    $bitmap = [Drawing.Bitmap]::new([int]$bounds.Width, [int]$bounds.Height)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $path = Join-Path $appRoot ($Label + '.png')
    try {
        $graphics.CopyFromScreen([int]$bounds.Left, [int]$bounds.Top, 0, 0, $bitmap.Size)
        $bitmap.Save($path, [Drawing.Imaging.ImageFormat]::Png)
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
    Write-Output "Native screenshot: $path"
}
function Assert-Fill($Control, [string]$Expected, [string]$Label) {
    $r = $Control.Current.BoundingRectangle
    $bitmap = [Drawing.Bitmap]::new(1, 1)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen([int]($r.Left + 6 * (Scale)), [int]($r.Top + 4 * (Scale)), 0, 0, $bitmap.Size)
        $actual = $bitmap.GetPixel(0, 0).ToArgb().ToString('X8').Substring(2)
        if ($actual -cne $Expected) { throw "Unexpected fill for ${Label}: expected #$Expected, actual #$actual" }
        Write-Output "Native fill ${Label}: #$actual"
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
}
function Hover-Control($Control) {
    $r = $Control.Current.BoundingRectangle
    [ManagerDensityNative]::SetCursorPos([int]($r.Right - 8), [int]($r.Top + $r.Height / 2)) | Out-Null
    Start-Sleep -Milliseconds 180
}
function Assert-WorkspaceColors {
    Foreground-Owned
    $bounds = $root.Current.BoundingRectangle
    [ManagerDensityNative]::SetCursorPos([int]($bounds.Left + 15), [int]($bounds.Top + 15)) | Out-Null
    Start-Sleep -Milliseconds 180
    $save = Named '保存'
    $route = Need-Node ('route-' + $longStem)
    $point = Need-Node 'point-0'
    Assert-Fill $save 'FBA1DB' 'save-normal'
    Hover-Control $save
    Assert-Fill $save 'F78BCF' 'save-hover'
    foreach ($item in @(@($route, 'route-selected'), @($point, 'point-selected'))) {
        Assert-Fill $item[0] 'F8C4E4' $item[1]
        Hover-Control $item[0]
        Assert-Fill $item[0] 'F8C4E4' ($item[1] + '-hover')
    }
    (Need-Node 'start').SetFocus()
    Start-Sleep -Milliseconds 180
    Assert-Fill $route 'F8C4E4' 'route-selected-input-focused'
    Assert-Fill $point 'F8C4E4' 'point-selected-input-focused'
}
function Reveal-Property([string]$Id) {
    $panel = Need-Node 'route-properties'
    $control = Need-Node $Id
    if (In-Bounds $control $panel.Current.BoundingRectangle) { return }
    $patterns = @($control.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName })
    if ($patterns -contains 'ScrollItemPatternIdentifiers.Pattern') {
        $control.GetCurrentPattern([Windows.Automation.ScrollItemPattern]::Pattern).ScrollIntoView()
        Start-Sleep -Milliseconds 125
    }
    for ($step = 0; $step -lt 16; $step++) {
        $viewport = (Need-Node 'route-properties').Current.BoundingRectangle
        $control = Need-Node $Id
        if (In-Bounds $control $viewport) { return }
        Foreground-Owned
        [ManagerDensityNative]::SetCursorPos([int]($viewport.Right - 20), [int]($viewport.Top + $viewport.Height / 2)) | Out-Null
        $direction = if ($control.Current.BoundingRectangle.Top -lt $viewport.Top) { 120 } else { -120 }
        [ManagerDensityNative]::mouse_event(0x0800, 0, 0, $direction, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 90
    }
    throw "Cannot reveal property $Id inside bounded property panel"
}

try {
    Write-Output "Isolated density sandbox: $appRoot"
    $manager = Start-Process -FilePath $exe -ArgumentList @('--app-root', ('"' + $appRoot + '"')) -PassThru
    Wait-Until 'native manager window' {
        $manager.Refresh()
        if ($manager.HasExited) { throw 'Owned manager exited before opening its window' }
        if ($manager.MainWindowHandle -eq [IntPtr]::Zero) { return $false }
        try {
            $script:root = [Windows.Automation.AutomationElement]::FromHandle($manager.MainWindowHandle)
            $null -ne (Node 'manager-status') -and $null -ne (Named '路线库还是空的')
        } catch { $false }
    } 15000
    Wait-Ready
    Resize-Manager 1280 900
    Assert-NamedVisible '打开取点工具'
    Assert-NamedVisible '路线库还是空的'
    Assert-Layout 'welcome-empty-library' $false $true

    $fixture = [ordered]@{
        format = 'route/v1'; name = $longRoute; note = 'Isolated density acceptance'
        target = @{ process = 'never-playback.exe'; windowClass = 'DensityFixture'; title = '离线目标窗口' }
        points = @(@{ id = 1; name = $longPoint; operations = @(@{ type = 'wait'; name = '布局检查动作'; ms = 1 }) })
    }
    [IO.File]::WriteAllText((Join-Path $routes ($longStem + '.yaml')), ($fixture | ConvertTo-Json -Depth 10), $utf8)
    $fixture.name = '零点位路线'
    $fixture.points = @()
    [IO.File]::WriteAllText((Join-Path $routes 'Zero.yaml'), ($fixture | ConvertTo-Json -Depth 10), $utf8)
    # A valid YAML/JSON document with invalid route schema gives a deterministic
    # long error containing the complete point name, without editing the source.
    $fixture.name = '本地校验失败路线'
    $fixture.points = @(@{ id = 0; name = $errorPoint; operations = @() })
    [IO.File]::WriteAllText((Join-Path $routes 'InvalidYaml.yaml'), ($fixture | ConvertTo-Json -Depth 10), $utf8)
    Invoke-Name '刷新'
    Wait-Until 'density fixture library refreshed' { $null -ne (Node ('route-' + $longStem)) }
    Open-Route $longStem
    Need-Node 'operation-toolbar' | Out-Null
    Assert-Layout 'long-name-default'
    Assert-WorkspaceColors
    $pointToggle = Need-Node 'point-info-toggle'
    if (-not (In-Bounds $pointToggle $root.Current.BoundingRectangle)) { throw 'Long point name displaced point controls' }
    Capture 'long-name-default'

    Resize-Manager 900 640
    if ($null -ne (Node ('route-' + $longStem))) { throw 'Library did not automatically collapse at 900 DIP' }
    Assert-Layout 'long-name-compact-auto'
    Invoke-Name '路线库'
    Wait-Until 'compact library manually reopened' { $null -ne (Node ('route-' + $longStem)) }
    Assert-Layout 'long-name-compact-library-open'
    Invoke-Name '路线属性'
    Need-Node 'route-properties' | Out-Null
    Assert-Layout 'properties-compact' $true
    Reveal-Property 'title'
    $value = (Need-Node 'title').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value
    if ($value -cne '离线目标窗口') { throw 'Scrolled target title lost its original value' }
    Capture 'properties-compact'
    Invoke-Name '路线属性'
    Wait-Until 'route properties collapsed' { $null -eq (Node 'route-properties') }

    Invoke-Name '文件操作'
    foreach ($name in @('导出', '定位文件', '删除路线', 'YAML 源码')) { Assert-NamedVisible $name }
    Assert-Layout 'files-compact' $true
    Invoke-Name '文件操作'
    Invoke-Name 'YAML 源码'
    Wait-Until 'valid route source editor shown' { $null -ne (Named '路线 YAML 源码') }
    Assert-Layout 'yaml-compact'
    Invoke-Name '点位编辑'
    Need-Node 'operation-toolbar' | Out-Null
    Assert-Layout 'form-roundtrip-compact'

    Open-Route 'Zero'
    Assert-NamedVisible '还没有点位'
    Assert-NamedVisible '选择一个点位'
    Assert-Layout 'zero-points-compact'
    Open-Route 'InvalidYaml'
    Wait-Until 'invalid route opens as YAML' { $null -ne (Named '路线 YAML 源码') }
    Invoke-Name '点位编辑'
    Wait-Until 'long local validation error' { (Need-Node 'manager-status').Current.Name.Contains($errorPoint) }
    $status = Need-Node 'manager-status'
    if ($status.Current.Name.Length -lt $errorPoint.Length -or $status.Current.Name -notmatch 'id 必须是正整数' -or $status.Current.Name -match '[\r\n]') { throw 'Long validation error was truncated or exposed as multiple lines' }
    if ($null -eq (Named '路线 YAML 源码')) { throw 'Failed form switch discarded the YAML editor' }
    Assert-Layout 'yaml-error-compact'
    Capture 'yaml-error-compact'

    Open-Route 'Zero'
    Invoke-Name '新建路线'
    $stem = Need-Node 'stem'
    $stem.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('DraftDensity')
    Invoke-Name '创建草稿'
    Need-Node 'route-properties' | Out-Null
    Assert-Layout 'draft-missing-target-compact' $true
    Reveal-Property 'process'
    if (Test-Path -LiteralPath (Join-Path $routes 'DraftDensity.yaml')) { throw 'Creating a draft unexpectedly wrote a route file' }
    Write-Output 'Passed: empty welcome/library, long route/point/file names, default and compact auto/manual library layouts, bounded properties/title visibility, file disclosure, form/YAML roundtrip, zero points, complete long validation error in a single-line status bar, and missing-target draft guidance. No playback requested.'
} catch {
    if ($null -ne $root) {
        try {
            $tree = @($root.FindAll($scope, [Windows.Automation.Condition]::TrueCondition) | ForEach-Object {
                [pscustomobject]@{ Id = $_.Current.AutomationId; Name = $_.Current.Name; Type = $_.Current.ControlType.ProgrammaticName; Enabled = $_.Current.IsEnabled; Offscreen = $_.Current.IsOffscreen; Bounds = $_.Current.BoundingRectangle.ToString(); Patterns = @($_.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName }) }
            })
            [IO.File]::WriteAllText((Join-Path $appRoot 'failure-tree.json'), (ConvertTo-Json -InputObject $tree -Depth 4), $utf8)
        } catch { Write-Warning ('Cannot dump failure accessibility tree: ' + $_.Exception.Message) }
    }
    throw
} finally {
    if ($null -ne $manager) {
        $manager.Refresh()
        if (-not $manager.HasExited) {
            $manager.CloseMainWindow() | Out-Null
            if (-not $manager.WaitForExit(1000)) { $manager.Kill(); $manager.WaitForExit() }
        }
    }
}
