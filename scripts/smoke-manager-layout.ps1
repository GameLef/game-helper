param([Parameter(Mandatory=$true)][int]$TargetProcessId)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=New-Object Text.UTF8Encoding $false
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class DesignWindow {
 [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hwnd,int x,int y,int width,int height,bool repaint);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr value);
}
'@
[DesignWindow]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
$process=Get-Process -Id $TargetProcessId
$root=[Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
$scope=[Windows.Automation.TreeScope]::Descendants
function Node($property,$value) {
 $condition=New-Object Windows.Automation.PropertyCondition $property,$value
 $root.FindFirst($scope,$condition)
}
function Click-Name($name) {
 $node=Node ([Windows.Automation.AutomationElement]::NameProperty) $name
 if($null -eq $node){throw "Missing control: $name"}
 $node.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
 Start-Sleep -Milliseconds 200
}
function Click-Id($id) {
 $node=Node ([Windows.Automation.AutomationElement]::AutomationIdProperty) $id
 if($null -eq $node){throw "Missing control id: $id"}
 $node.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
 Start-Sleep -Milliseconds 200
}
function Assert-Control($name) {
 $node=Node ([Windows.Automation.AutomationElement]::NameProperty) $name
 if($null -eq $node){throw "Missing critical control: $name"}
 $r=$node.Current.BoundingRectangle
 $b=$root.Current.BoundingRectangle
 if($r.IsEmpty -or $r.Left -lt $b.Left -or $r.Right -gt $b.Right -or $r.Top -lt $b.Top -or $r.Bottom -gt $b.Bottom){throw "Control outside window: $name $r, window $b"}
}
function Assert-ControlId($id) {
  $node=Node ([Windows.Automation.AutomationElement]::AutomationIdProperty) $id
  if($null -eq $node){throw "Missing critical control id: $id"}
  $r=$node.Current.BoundingRectangle
  $b=$root.Current.BoundingRectangle
  if($r.IsEmpty -or $r.Left -lt $b.Left -or $r.Right -gt $b.Right -or $r.Top -lt $b.Top -or $r.Bottom -gt $b.Bottom){throw "Control outside window: $id $r, window $b"}
}
function Assert-Absent($name) {
 if($null -ne (Node ([Windows.Automation.AutomationElement]::NameProperty) $name)){throw "Removed control is still present: $name"}
}
function Capture($name) {
 $output='.tools/'+$name+'.png'
 & (Join-Path $PSScriptRoot 'inspect-native.ps1') -TargetProcessId $TargetProcessId -OutputPath $output | Out-File (Join-Path $PSScriptRoot ('../.tools/'+$name+'-tree.log')) -Encoding utf8
}
[DesignWindow]::SetForegroundWindow($process.MainWindowHandle)|Out-Null
[DesignWindow]::MoveWindow($process.MainWindowHandle,40,40,1280,900,$true)|Out-Null
Start-Sleep -Milliseconds 350
Click-Id 'route-Smoke'
foreach($removed in @('Game Helper','路线工作台','归一化 X','归一化 Y','坐标范围 0–1')){Assert-Absent $removed}
if($root.Current.Name -match 'Game Helper'){throw 'Brand remains in native title'}
Assert-Control '保存'
Assert-Absent '执行路线'
Assert-Control '起始点'
Assert-Control '保存起始点'
Assert-Control '试跑选中点'
foreach($id in @('hotkey-set','hotkey-clear','hotkey-current')){Assert-ControlId $id}
Click-Id 'point-0'
if((Node ([Windows.Automation.AutomationElement]::NameProperty) '上移').Current.IsEnabled){throw 'First point must not move upward'}
Capture 'manager-default'
Click-Id 'point-1'
if((Node ([Windows.Automation.AutomationElement]::NameProperty) '下移').Current.IsEnabled){throw 'Last point must not move downward'}
if($null -eq (Node ([Windows.Automation.AutomationElement]::NameProperty) '暂无动作')){throw 'Empty operation state unavailable'}
[DesignWindow]::MoveWindow($process.MainWindowHandle,40,40,900,640,$true)|Out-Null
Start-Sleep -Milliseconds 350
if($null -ne (Node ([Windows.Automation.AutomationElement]::NameProperty) '搜索名称或文件名')){throw 'Compact layout did not collapse route library'}
Assert-Control '保存'
Assert-Absent '执行路线'
Assert-Control '起始点'
Assert-Control '保存起始点'
Assert-Control '试跑选中点'
foreach($id in @('hotkey-set','hotkey-clear','hotkey-current')){Assert-ControlId $id}
Assert-Control '保存起始点'
Capture 'manager-compact'
Click-Name '路线库'
if($null -eq (Node ([Windows.Automation.AutomationElement]::NameProperty) '搜索名称或文件名')){throw 'Compact route library toggle unavailable'}
Assert-Control '保存'
Click-Name '路线库'
[DesignWindow]::MoveWindow($process.MainWindowHandle,40,40,1280,900,$true)|Out-Null
Start-Sleep -Milliseconds 250
# Explicitly reopen it after the compact manual toggle.
Click-Name '路线库'
Click-Id 'point-0'
Write-Output ('Observed: default and 900x640 bounds, compact library collapse/reopen, point sorting boundaries, empty actions; actual window DPI '+[DesignWindow]::GetDpiForWindow($process.MainWindowHandle))
