#requires -Version 7.0
param([string]$Executable = 'target\release\game-helper.exe')
# Copies in an isolated picker, then performs real Ctrl+V in an isolated manager.
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$exe = if ([IO.Path]::IsPathRooted($Executable)) { $Executable } else { Join-Path $projectRoot $Executable }
$fixture = Join-Path $projectRoot ('.tools\picker-coordinates-' + [guid]::NewGuid().ToString('N'))
foreach ($folder in @('routes', 'state', 'ahk')) { [IO.Directory]::CreateDirectory((Join-Path $fixture $folder)) | Out-Null }
$utf8 = [Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText((Join-Path $fixture 'ahk\daemon.ahk'), '; Inert fixture; no playback.', $utf8)
[IO.File]::WriteAllText((Join-Path $fixture 'config.json'), '{"autoStartDaemon":false,"autoReloadOnSave":false,"hotkeys":{"stopAll":"F12","bindings":[]}}', $utf8)
[IO.File]::WriteAllText((Join-Path $fixture 'state\picker.json'), '{"format":"picker/v1","lastTarget":{"process":"game.exe","windowClass":"Game","title":"坐标粘贴测试"},"targets":[{"target":{"process":"game.exe","windowClass":"Game","title":"坐标粘贴测试"},"points":[{"id":1,"name":"复制坐标","position":[0.123456789,0.987654321]}]}]}', $utf8)
$route = @'
format: route/v1
name: Coordinates
note: isolated clipboard acceptance
target:
  process: game.exe
points:
  - id: 1
    name: 点击测试
    note: ''
    operations:
      - type: click
        pos: [0.5, 0.5]
        button: left
        clicks: 1
'@
[IO.File]::WriteAllText((Join-Path $fixture 'routes\Coordinates.yaml'), $route, $utf8)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class PickerClipboardNative {
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
}
'@
$scope = [Windows.Automation.TreeScope]::Descendants
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()
$root = $null
$clipboard = Get-Clipboard -Raw
function Wait-Until([string]$Description, [scriptblock]$Predicate) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    do { if (& $Predicate) { return }; Start-Sleep -Milliseconds 100 } while ($watch.ElapsedMilliseconds -lt 8000)
    throw "Timed out: $Description"
}
function Node([string]$Id) { $root.FindFirst($scope, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, $Id)) }
function Named([string]$Name) { $root.FindFirst($scope, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $Name)) }
function Invoke-Node($Node) {
    if ($null -eq $Node) { throw 'Missing control' }
    $Node.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
}
function Launch([bool]$Picker) {
    $arguments = @('--app-root', ('"' + $fixture + '"'))
    if ($Picker) { $arguments += '--picker' }
    $process = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru
    $owned.Add($process)
    Wait-Until 'native window' { $process.Refresh(); $process.MainWindowHandle -ne [IntPtr]::Zero }
    Start-Sleep -Milliseconds 700
    $process.Refresh()
    $script:root = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
    $process
}
try {
    $picker = Launch $true
    Wait-Until 'restored point' { $null -ne (Node 'copy-coordinate-1') }
    Invoke-Node (Node 'copy-coordinate-1')
    Wait-Until 'coordinate copied' { [string](Get-Clipboard -Raw) -eq '[0.123457, 0.987654]' }
    $picker.CloseMainWindow() | Out-Null
    if (-not $picker.WaitForExit(5000)) { throw 'Picker did not close after coordinate copy' }
    $manager = Launch $false
    Wait-Until 'manager route ready' { $null -ne (Node 'route-Coordinates') }
    Invoke-Node (Node 'route-Coordinates')
    Wait-Until 'click position field' { $null -ne (Node 'operation-0-pos') }
    [PickerClipboardNative]::SetForegroundWindow($manager.MainWindowHandle) | Out-Null
    Wait-Until 'manager foreground' { [PickerClipboardNative]::GetForegroundWindow() -eq $manager.MainWindowHandle }
    (Node 'operation-0-pos').SetFocus()
    [Windows.Forms.SendKeys]::SendWait('^a')
    [Windows.Forms.SendKeys]::SendWait('^v')
    Wait-Until 'actual Ctrl+V value' { (Node 'operation-0-pos').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value -eq '[0.123457, 0.987654]' }
    Invoke-Node (Named '保存')
    Start-Sleep -Milliseconds 300
    if ($null -ne (Named '备份并保存')) { Invoke-Node (Named '备份并保存') }
    $cache = Join-Path $fixture 'routes\_cache\Coordinates.json'
    Wait-Until 'saved action coordinates' {
        if (-not (Test-Path -LiteralPath $cache)) { return $false }
        try {
            $position = ([IO.File]::ReadAllText($cache) | ConvertFrom-Json).points[0].operations[0].pos
            $position.Count -eq 2 -and $position[0] -eq 0.123457 -and $position[1] -eq 0.987654
        } catch { $false }
    }
    $manager.CloseMainWindow() | Out-Null
    if (-not $manager.WaitForExit(5000)) { throw 'Saved manager did not close' }
    $manager = Launch $false
    Wait-Until 'reopened route ready' { $null -ne (Node 'route-Coordinates') }
    Invoke-Node (Node 'route-Coordinates')
    Wait-Until 'reopened coordinates' {
        $node = Node 'operation-0-pos'
        if ($null -eq $node) { return $false }
        try {
            $position = $node.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value | ConvertFrom-Json
            $position.Count -eq 2 -and $position[0] -eq 0.123457 -and $position[1] -eq 0.987654
        } catch { $false }
    }
    Write-Output 'Passed: picker coordinate click copies a JSON pair; real Ctrl+V into manager click position; saved route/cache and restarted manager retain coordinates. No playback requested.'
} finally {
    foreach ($process in $owned) {
        $process.Refresh()
        if (-not $process.HasExited) { $process.CloseMainWindow() | Out-Null; if (-not $process.WaitForExit(1000)) { $process.Kill(); $process.WaitForExit(5000) | Out-Null } }
        $process.Dispose()
    }
    if ($null -eq $clipboard) { Set-Clipboard -Value '' } else { Set-Clipboard -Value $clipboard }
    Write-Output "Sandbox retained for inspection: $fixture"
}
