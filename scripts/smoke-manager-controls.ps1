param([string]$Executable = 'target\debug\game-helper.exe')
# Own isolated app-root; never sends input to a target app or invokes playback.
$ErrorActionPreference='Stop'
$projectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$fixture=Join-Path $projectRoot ('.tools\manager-controls-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path (Join-Path $fixture 'routes'),(Join-Path $fixture 'ahk') | Out-Null
Copy-Item -Path (Join-Path $projectRoot 'ahk\*') -Destination (Join-Path $fixture 'ahk') -Recurse -Force
$utf8=New-Object Text.UTF8Encoding $false
$config='{"routesDir":"routes","hotkeys":{"stopAll":"F12","bindings":[{"hotkey":"F9","route":"Controls","startAt":2}]},"autoStartDaemon":false,"autoReloadOnSave":false}'
[IO.File]::WriteAllText((Join-Path $fixture 'config.json'),$config,$utf8)
$source=@'
format: route/v1
name: Controls
note: isolated controls acceptance
target:
  process: notepad.exe
points:
  - id: 42
    name: first
    pos: [0.25, 0.5]
    operations:
      - type: click
        pos: [0.3, 0.6]
      - type: key
        key: w
  - id: 7
    name: second
    pos: [0.75, 0.5]
    operations: []
'@
[IO.File]::WriteAllText((Join-Path $fixture 'routes\Controls.yaml'),$source,$utf8)
[IO.File]::WriteAllText((Join-Path $fixture 'routes\Other.yaml'),$source.Replace('name: Controls','name: Other'),$utf8)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ControlsWindow {
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hwnd,int x,int y,int width,int height,bool repaint);
}
'@
$managerProcess=Start-Process -FilePath (Join-Path $projectRoot $Executable) -ArgumentList @('--app-root',('"'+$fixture+'"')) -PassThru
$scope=[Windows.Automation.TreeScope]::Descendants
function Find-Node($property,$value) {
 $root.FindFirst($scope,(New-Object Windows.Automation.PropertyCondition $property,$value))
}
function By-Id($id) { Find-Node ([Windows.Automation.AutomationElement]::AutomationIdProperty) $id }
function By-Name($name) { Find-Node ([Windows.Automation.AutomationElement]::NameProperty) $name }
function Click-Node($node) {
 if($null -eq $node){throw 'Missing click target'}
 $patterns=@($node.GetSupportedPatterns() | ForEach-Object {$_.ProgrammaticName})
 if($patterns -contains 'InvokePatternIdentifiers.Pattern') {
  $node.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
 } elseif($patterns -contains 'TogglePatternIdentifiers.Pattern') {
  $node.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
 } elseif($patterns -contains 'ExpandCollapsePatternIdentifiers.Pattern') {
  $node.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
 } elseif($patterns -contains 'SelectionItemPatternIdentifiers.Pattern') {
  $node.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
 } else {throw ('No activation pattern: '+$node.Current.Name+'; '+($patterns -join ', '))}
 Start-Sleep -Milliseconds 150
}
function Ensure-PointInfo($id) {
 if($id -in @('point_name','point_note') -and $null -eq (By-Id $id)) {
  Click-Node (By-Id 'point-info-toggle')
 }
}
function Set-Input($id,$text) {
 Ensure-PointInfo $id
 $node=By-Id $id
 if($null -eq $node){throw "Missing input $id"}
 $node.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($text)
 Start-Sleep -Milliseconds 100
}
function Input-Value($id) {
 Ensure-PointInfo $id
 (By-Id $id).GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value
}
function Wait-Ready {
 for($i=0;$i -lt 50;$i++) {
  $node=By-Id 'manager-status'
  if($null -ne $node -and $node.Current.Name -notmatch '正在加载|正在处理' -and $null -eq (By-Id 'route-loading-shield')){return}
  Start-Sleep -Milliseconds 100
 }
 throw ('Manager did not become ready: '+(($root.FindAll($scope,[Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name+' ['+$_.Current.AutomationId+']' }) -join '; '))
}
function Save-Route {
 Click-Node (By-Name '保存')
 if($null -ne (By-Name '备份并保存')){Click-Node (By-Name '备份并保存')}
 Wait-Ready
}
function Read-Cache { Get-Content -Raw -Encoding utf8 (Join-Path $fixture 'routes\_cache\Controls.json') | ConvertFrom-Json }
function Choose($id,$offset) {
 $node=By-Id $id
 if($null -eq $node){throw "Missing select $id"}
 $combo=$node.FindFirst($scope,(New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty),([Windows.Automation.ControlType]::ComboBox)))
 if($null -eq $combo){throw "Missing combobox inside $id"}
 $choices=if($id -like '*-button'){@('左键','右键','中键')}else{@('按下','松开')}
 $current=$combo.GetCurrentPropertyValue([Windows.Automation.ValuePattern]::ValueProperty)
 $currentIndex=[array]::IndexOf($choices,$current)
 if($currentIndex -lt 0){throw "Unexpected dropdown value: $current"}
 [ControlsWindow]::SetForegroundWindow($managerProcess.MainWindowHandle)|Out-Null
 $combo.SetFocus()
 [Windows.Forms.SendKeys]::SendWait('{ENTER}')
 Start-Sleep -Milliseconds 200
 $direction=if($offset -gt $currentIndex){'{DOWN}'}else{'{UP}'}
 for($i=0;$i -lt [Math]::Abs($offset-$currentIndex);$i++){[Windows.Forms.SendKeys]::SendWait($direction)}
 [Windows.Forms.SendKeys]::SendWait('{ENTER}')
 Start-Sleep -Milliseconds 200
 $actual=$combo.GetCurrentPropertyValue([Windows.Automation.ValuePattern]::ValueProperty)
 if($actual -ne $choices[$offset]){throw "Dropdown did not commit: $id expected $($choices[$offset]), actual $actual"}
}
try {
 for($i=0;$i -lt 50;$i++) {
  $managerProcess.Refresh()
  if($managerProcess.MainWindowHandle -ne [IntPtr]::Zero){break}
  Start-Sleep -Milliseconds 100
 }
 Start-Sleep -Milliseconds 500
 $managerProcess.Refresh()
 $root=[Windows.Automation.AutomationElement]::FromHandle($managerProcess.MainWindowHandle)
 $null=$root.FindAll($scope,[Windows.Automation.Condition]::TrueCondition)
 [ControlsWindow]::SetForegroundWindow($managerProcess.MainWindowHandle)|Out-Null
 [ControlsWindow]::MoveWindow($managerProcess.MainWindowHandle,20,20,1280,1000,$true)|Out-Null
 Start-Sleep -Milliseconds 250
 Wait-Ready
 Click-Node (By-Id 'route-Controls')
 Wait-Ready
 if($root.Current.Name -match 'Game Helper'){throw 'Brand remains in native title'}
 foreach($name in @('执行路线','归一化 X','归一化 Y','坐标范围 0–1','Game Helper','路线工作台')) {
  if($null -ne (By-Name $name)){throw "Removed control remains: $name"}
 }
 if((Input-Value 'start') -ne '2'){throw 'Saved startAt not restored'}
 if($null -eq (By-Id 'operation-0-button') -or $null -eq (By-Id 'operation-1-up')){throw 'Missing operation select'}
 & (Join-Path $PSScriptRoot 'inspect-native.ps1') -TargetProcessId $managerProcess.Id -OutputPath '.tools/manager-controls.png' | Out-File (Join-Path $fixture 'controls-tree.log') -Encoding utf8
 # Defaults and correct persisted JSON types.
 Save-Route
 $cached=Read-Cache
 if($cached.points[0].operations[0].button -cne 'left' -or $cached.points[0].operations[1].up -isnot [bool] -or $cached.points[0].operations[1].up){throw 'Default choices changed'}
 foreach($case in @(@(1,'right',1,$true),@(2,'middle',0,$false),@(0,'left',1,$true))) {
  Choose 'operation-0-button' $case[0]
  Choose 'operation-1-up' $case[2]
  Save-Route
  $cached=Read-Cache
  if($cached.points[0].operations[0].button -cne $case[1] -or $cached.points[0].operations[1].up -isnot [bool] -or $cached.points[0].operations[1].up -ne $case[3]){throw ('Choice save mismatch: '+($cached|ConvertTo-Json -Depth 10 -Compress))}
 }
 # Rebuilding operation controls must retain committed choices and types.
 Click-Node (By-Id 'operation-drag-0')
 Set-Input 'operation-move-position' '2'
 Click-Node (By-Id 'operation-move-confirm')
 Save-Route
 $cached=Read-Cache
 if($cached.points[0].operations[0].type -ne 'key' -or -not $cached.points[0].operations[0].up -or $cached.points[0].operations[1].button -ne 'left'){throw 'Moving an operation lost choices'}
 Click-Node (By-Id 'operation-drag-1')
 Set-Input 'operation-move-position' '1'
 Click-Node (By-Id 'operation-move-confirm')
 Click-Node (By-Id 'point-1')
 Click-Node (By-Id 'point-0')
 Click-Node (By-Name 'YAML 源码')
 $editCondition=New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty),([Windows.Automation.ControlType]::Edit)
 $yaml=@($root.FindAll($scope,$editCondition) | Where-Object {$_.Current.Name -eq '路线 YAML 源码'}) | Select-Object -First 1
 if($null -eq $yaml){throw 'YAML editor not found'}
 $yamlText=$yaml.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value
 if($yamlText -notmatch 'up: true' -or $yamlText -notmatch 'button: left'){throw 'Choices lost during form/YAML round-trip'}
 Click-Node (By-Name '点位编辑')
 $buttonCombo=By-Name '动作 1 鼠标按键'
 $buttonCombo.SetFocus()
 [Windows.Forms.SendKeys]::SendWait('{ENTER}')
 [Windows.Forms.SendKeys]::SendWait('{DOWN}')
 [Windows.Forms.SendKeys]::SendWait('{ESC}')
 Start-Sleep -Milliseconds 200
 if($buttonCombo.GetCurrentPropertyValue([Windows.Automation.ValuePattern]::ValueProperty) -ne '左键'){throw 'Escape committed a pending option'}
 Save-Route
 Set-Input 'point_name' '中文点位'
 Save-Route
 $cached=Read-Cache
 if($cached.points[0].PSObject.Properties.Name -contains 'pos' -or $cached.points[1].PSObject.Properties.Name -contains 'pos' -or $cached.points[0].operations[0].pos[0] -ne 0.3 -or $cached.points[0].operations[0].pos[1] -ne 0.6){throw 'Point coordinates persisted or action coordinates changed'}
 $generatedYaml=[IO.File]::ReadAllText((Join-Path $fixture 'routes\Controls.yaml'))
 if($generatedYaml -match '(?m)^  pos:'){throw 'Generated YAML contains a point-level pos'}
 Click-Node (By-Id 'route-Other'); Wait-Ready
 if((Input-Value 'start') -ne '1'){throw 'Unbound route inherited previous startAt'}
 Click-Node (By-Id 'route-Controls'); Wait-Ready
 if((Input-Value 'start') -ne '2'){throw 'Switch did not restore startAt'}
 Set-Input 'point_note' 'unsaved'
 Click-Node (By-Id 'route-Controls')
 if($null -ne (By-Id 'confirmation-dialog')){throw 'Current-route click should not prompt'}
 Click-Node (By-Id 'route-Other')
 Click-Node (By-Name '取消')
 if((Input-Value 'point_note') -ne 'unsaved'){throw 'Cancel lost unsaved text'}
 Click-Node (By-Id 'route-Other')
 Click-Node (By-Name '放弃并继续'); Wait-Ready
 if((Input-Value 'start') -ne '1'){throw 'Discard did not switch'}
 Click-Node (By-Id 'route-Controls'); Wait-Ready
 if((Input-Value 'point_note') -eq 'unsaved'){throw 'Discard was persisted'}
 Set-Input 'point_note' 'saved-switch'
 Click-Node (By-Id 'route-Other')
 Click-Node (By-Name '保存后继续')
 if($null -ne (By-Name '备份并保存')){Click-Node (By-Name '备份并保存')}
 Wait-Ready
 if((Input-Value 'start') -ne '1'){throw 'Save-and-continue did not switch'}
 Click-Node (By-Id 'route-Controls'); Wait-Ready
 if((Input-Value 'point_note') -ne 'saved-switch'){throw 'Save-and-continue lost edits'}
 # StartAt binding persists without starting the daemon or invoking playback.
 Set-Input 'start' '1'
 Click-Node (By-Name '保存绑定'); Wait-Ready
 $binding=(Get-Content -Raw -Encoding utf8 (Join-Path $fixture 'config.json')|ConvertFrom-Json).hotkeys.bindings[0]
 if($binding.startAt -ne 1){throw 'StartAt binding not saved'}
 Set-Input 'start' '999'
 Click-Node (By-Name '保存绑定')
 if((By-Id 'manager-status').Current.Name -notmatch '起始位置超出'){throw 'Invalid startAt not rejected'}
 # A missing file must not clear the displayed route.
 $missingSource=(Resolve-Path (Join-Path $fixture 'routes\Other.yaml')).Path
 $missingTarget=[IO.Path]::GetFullPath((Join-Path $fixture ('routes\Other.'+[guid]::NewGuid().ToString('N')+'.missing')))
 if($missingSource -ne (Join-Path $fixture 'routes\Other.yaml') -or -not $missingTarget.StartsWith((Join-Path $fixture 'routes\'),[StringComparison]::OrdinalIgnoreCase)){throw 'Unexpected fixture move paths'}
 Write-Output "Verified isolated move: $missingSource -> $missingTarget"
 [IO.File]::Move($missingSource,$missingTarget)
 Click-Node (By-Id 'route-Other'); Wait-Ready
 if((Input-Value 'point_name') -ne '中文点位'){throw 'Failed open cleared old document'}
 Write-Output 'Passed: titles and removed fields, Chinese dropdown defaults/all choices/JSON types, action coordinate retention and no point-level pos, route switching/discard/cancel/open failure, startAt restore/default/save/range validation. No playback requested.'
} finally {
 if(-not $managerProcess.HasExited){
  $managerProcess.CloseMainWindow()|Out-Null
  if(-not $managerProcess.WaitForExit(1000)){$managerProcess.Kill()}
 }
}
