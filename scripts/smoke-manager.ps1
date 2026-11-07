param([Parameter(Mandatory=$true)][int]$TargetProcessId)
# Run only against the isolated acceptance sandbox. Never invokes playback.
# Keep this script encoded as UTF-8 with BOM for Windows PowerShell 5.1.
$ErrorActionPreference='Stop'
$projectRoot=if($PSScriptRoot){[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))}else{(Get-Location).Path}
[Console]::OutputEncoding=New-Object Text.UTF8Encoding $false
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$p=Get-Process -Id $TargetProcessId
$root=[Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
$scope=[Windows.Automation.TreeScope]::Descendants
function Nodes($type) {
    $condition=New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty),$type
    $root.FindAll($scope,$condition)
}
function Click($name) {
    $button=@(Nodes ([Windows.Automation.ControlType]::Button) | Where-Object {$_.Current.Name -eq $name}) | Select-Object -First 1
    if($null -eq $button){throw "Button not found: $name"}
    $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 350
}
function Values {
    foreach($edit in (Nodes ([Windows.Automation.ControlType]::Edit))) {
        try { $value=$edit.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern); [PSCustomObject]@{Node=$edit; Pattern=$value; Id=$edit.Current.AutomationId; Name=$edit.Current.Name; Text=$value.Current.Value} } catch {}
    }
}
function Wait-Value($predicate,$description) {
    for($wait=0;$wait -lt 50;$wait++) {
        $value=@(Values | Where-Object $predicate) | Select-Object -First 1
        if($null -ne $value){return $value}
        Start-Sleep -Milliseconds 100
    }
    throw ($description+'; inputs: '+((Values | Select-Object Id,Name,Text | ConvertTo-Json -Compress)))
}
function Name-Input {
    # A tab switch may retain or reset the collapsed state. Do not blindly toggle it.
    $name=@(Values | Where-Object {$_.Id -eq 'name'}) | Select-Object -First 1
    if($null -eq $name){Click '路线属性'}
    Wait-Value {$_.Id -eq 'name'} 'Route name input unavailable after expanding properties'
}
$route=@(Nodes ([Windows.Automation.ControlType]::Button) | Where-Object {$_.Current.Name.StartsWith('Smoke ')}) | Select-Object -First 1
if($null -eq $route){throw 'Isolated Smoke route not found'}
$route.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
Start-Sleep -Milliseconds 500
$name=Name-Input
$expected=-join [char[]]@(0x539F,0x751F,0x4E2D,0x6587)
$name.Pattern.SetValue($expected)
Click 'YAML 源码'
$source=Wait-Value {$_.Text.Contains('route/v1')} 'YAML editor unavailable'
$valid=$source.Pattern.Current.Value
if(-not $valid.Contains($expected)){throw 'Form-to-YAML lost Unicode name'}
$source.Pattern.SetValue('format: [')
Click '点位编辑'
$source=Wait-Value {$_.Text -eq 'format: ['} 'Invalid YAML input lost when switching views'
$source.Pattern.SetValue($valid)
Click '点位编辑'
$name=Name-Input
if($name.Pattern.Current.Value -ne $expected){throw 'YAML-to-form lost Unicode name'}
Click 'YAML 源码'
Click '保存'
$path=Join-Path $projectRoot '.tools\acceptance sandbox\routes\Smoke.yaml'
$text=[IO.File]::ReadAllText($path)
if(-not $text.Contains($expected)){throw 'Explicit save did not persist Unicode name'}
$cache=Join-Path $projectRoot '.tools\acceptance sandbox\routes\_cache\Smoke.json'
$cached=[IO.File]::ReadAllText($cache)|ConvertFrom-Json
if($cached.name -ne $expected -or $cached.points[0].id -ne 42){throw 'Cache contract mismatch'}
Write-Output 'Observed: native form/YAML Unicode round-trip, broken YAML retained on failed tab switch, explicit source save and executable cache.'
$source=Wait-Value {$_.Text.Contains('route/v1')} 'YAML editor unavailable after saving'
$unsaved=$valid+"`n# unsaved-close-probe`n"
$source.Pattern.SetValue($unsaved)
$p.CloseMainWindow() | Out-Null
Start-Sleep -Milliseconds 300
$p.Refresh()
if($p.HasExited){throw 'Dirty window closed without protection'}
Click '取消'
$p.Refresh()
if($p.HasExited){throw 'Cancel closed the dirty window'}
$source=Wait-Value {$_.Text -eq $unsaved} 'Cancel close lost unsaved YAML'
$p.CloseMainWindow() | Out-Null
Start-Sleep -Milliseconds 300
$p.Refresh()
if($p.HasExited){throw 'Second dirty close bypassed protection'}
Click '放弃并继续'
if(-not $p.WaitForExit(5000)){throw 'Explicit discard failed to close window'}
Write-Output 'Observed: dirty close blocked, cancel retained unsaved YAML, and explicit discard closed the native window.'
