#requires -Version 7.0
[CmdletBinding()]
param([string]$Executable='target\debug\game-helper.exe')
# UI-only acceptance: no AHK service is started, no replay is requested, and key
# capture uses messages addressed only to the owned manager HWND, never SendInput.
$ErrorActionPreference='Stop'
$projectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$exe=if([IO.Path]::IsPathRooted($Executable)){$Executable}else{Join-Path $projectRoot $Executable}
$fixture=Join-Path $projectRoot ('.tools\manager-hotkeys-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $fixture 'routes'),(Join-Path $fixture 'ahk') -Force | Out-Null
Copy-Item -Path (Join-Path $projectRoot 'ahk\*') -Destination (Join-Path $fixture 'ahk') -Recurse
$utf8=[Text.UTF8Encoding]::new($false)
$configPath=Join-Path $fixture 'config.json'
$config=@{routesDir='routes';autoStartDaemon=$false;autoReloadOnSave=$false;hotkeys=@{stopAll='^!+F24';bindings=@(
 @{route='HotkeyA';hotkey='F9';action='run_route';startAt=2;enabled=$false},
 @{route='HotkeyB';hotkey='F9';action='run_route';startAt=1;enabled=$false},
 @{route='Legacy';hotkey='F8';action='run_route';startAt=1}
)}}
[IO.File]::WriteAllText($configPath,($config | ConvertTo-Json -Depth 10),$utf8)
foreach($stem in @('HotkeyA','HotkeyB','Capture','Legacy')){
 $yaml="format: route/v1`nname: $stem`ntarget: {title: NeverReplay_$([guid]::NewGuid().ToString('N'))}`npoints:`n  - id: 1`n    name: first`n    operations: []`n  - id: 2`n    name: second`n    operations: []`n"
 [IO.File]::WriteAllText((Join-Path $fixture "routes\$stem.yaml"),$yaml,$utf8)
}
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class HotkeyUiSmoke {
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(IntPtr cls,string title);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd,uint message,IntPtr key,IntPtr data);
 [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hwnd,int x,int y,int width,int height,bool repaint);
}
'@
$scope=[Windows.Automation.TreeScope]::Descendants
$script:manager=$null
$script:uiRoot=$null
function Wait-For([string]$Label,[scriptblock]$Check,[int]$Timeout=10000){
 $watch=[Diagnostics.Stopwatch]::StartNew()
 do {if(& $Check){return};Start-Sleep -Milliseconds 80} while($watch.ElapsedMilliseconds -lt $Timeout)
 throw "Timed out: $Label; evidence: $fixture"
}
function By-Id([string]$Id){
 if($null -eq $script:manager){return $null}
 $script:manager.Refresh()
 if($script:manager.HasExited){throw "Owned manager exited while finding $Id"}
 if($script:manager.MainWindowHandle -eq [IntPtr]::Zero){return $null}
 # Reacquire the provider on every lookup, including each Wait-Ready poll.
 # A manager restart can reuse an HWND while an earlier UIA root is still cached.
 $script:uiRoot=[Windows.Automation.AutomationElement]::FromHandle($script:manager.MainWindowHandle)
 $script:uiRoot.FindFirst($scope,(New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::AutomationIdProperty),$Id))
}
function Assert([bool]$Value,[string]$Message){if(-not $Value){throw $Message}}
function Focus-Manager {
 Assert ($null -ne $script:manager -and -not $script:manager.HasExited) 'Owned manager is no longer running'
 [HotkeyUiSmoke]::SetForegroundWindow($script:manager.MainWindowHandle) | Out-Null
 Wait-For 'owned manager foreground' {[HotkeyUiSmoke]::GetForegroundWindow() -eq $script:manager.MainWindowHandle}
}
function Save-FailureUiTree {
 # Best effort diagnostics must never replace the original acceptance failure.
 try {
  $diagnosticRoot=$script:uiRoot
  if($null -ne $script:manager -and -not $script:manager.HasExited -and $script:manager.MainWindowHandle -ne [IntPtr]::Zero){
   $diagnosticRoot=[Windows.Automation.AutomationElement]::FromHandle($script:manager.MainWindowHandle)
  }
  if($null -eq $diagnosticRoot){throw 'Manager UIA root unavailable'}
  $walker=[Windows.Automation.TreeWalker]::RawViewWalker
  $rows=[Collections.Generic.List[object]]::new()
  foreach($node in $diagnosticRoot.FindAll([Windows.Automation.TreeScope]::Subtree,[Windows.Automation.Condition]::TrueCondition)){
   try {
    $current=$node.Current
    $parent=$walker.GetParent($node)
    $parentId=if($null -ne $parent){$parent.GetRuntimeId() -join '.'}else{''}
    $rows.Add([ordered]@{runtimeId=($node.GetRuntimeId() -join '.');parentRuntimeId=$parentId;automationId=$current.AutomationId;name=$current.Name;controlType=$current.ControlType.ProgrammaticName;enabled=$current.IsEnabled;offscreen=$current.IsOffscreen;focus=$current.HasKeyboardFocus;bounds=$current.BoundingRectangle.ToString();patterns=@($node.GetSupportedPatterns() | ForEach-Object {$_.ProgrammaticName})})
   } catch {$rows.Add([ordered]@{unavailable=$_.Exception.Message})}
  }
  [IO.File]::WriteAllText((Join-Path $fixture 'failure-uia-tree.json'),(ConvertTo-Json -InputObject @($rows.ToArray()) -Depth 8),$utf8)
  $lines=@($rows | ForEach-Object {if($_.Contains('unavailable')){'Unavailable: '+$_.unavailable}else{'['+$_.runtimeId+'] parent=['+$_.parentRuntimeId+'] '+$_.controlType+' id="'+$_.automationId+'" name="'+$_.name+'" enabled='+$_.enabled+' offscreen='+$_.offscreen+' focus='+$_.focus+' bounds='+$_.bounds+' patterns='+($_.patterns -join ', ')}})
  [IO.File]::WriteAllText((Join-Path $fixture 'failure-uia-tree.txt'),($lines -join [Environment]::NewLine),$utf8)
 } catch {
  [IO.File]::WriteAllText((Join-Path $fixture 'failure-uia-dump-error.txt'),($_ | Out-String),$utf8)
 }
}
function Wait-Ready {
 Wait-For 'manager ready' {
  $status=By-Id 'manager-status'
  $null -ne $status -and $status.Current.Name -notmatch '正在加载|正在处理' -and $null -eq (By-Id 'route-loading-shield')
 }
}
function Click-Node($Node){
 Assert ($null -ne $Node) 'Missing UI action target'
 $Node.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
 Start-Sleep -Milliseconds 120
}
function Click-Id([string]$Id){Click-Node (By-Id $Id)}
function Read-Config {Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json}
function Binding([string]$Stem){(Read-Config).hotkeys.bindings | Where-Object route -eq $Stem | Select-Object -First 1}
function Switch-Pattern([string]$Stem){
 $wrapper=By-Id "route-hotkey-$Stem"
 Assert ($null -ne $wrapper) "Missing route switch: $Stem"
 $nodes=@($wrapper)+@($wrapper.FindAll($scope,[Windows.Automation.Condition]::TrueCondition))
 foreach($node in $nodes){
  if(@($node.GetSupportedPatterns() | ForEach-Object {$_.ProgrammaticName}) -contains 'TogglePatternIdentifiers.Pattern'){
   return $node.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
  }
 }
 throw "No TogglePattern in route-hotkey-$Stem"
}
function Assert-Switch([string]$Stem,[bool]$Enabled){
 $expected=if($Enabled){[Windows.Automation.ToggleState]::On}else{[Windows.Automation.ToggleState]::Off}
 Assert ((Switch-Pattern $Stem).Current.ToggleState -eq $expected) "Wrong visible switch state for $Stem"
}
function Toggle-Route([string]$Stem){(Switch-Pattern $Stem).Toggle();Start-Sleep -Milliseconds 150;Wait-Ready}
function Assert-Owners([string]$Owner){
 Wait-For "F9 owner $Owner" {
  $a=Binding 'HotkeyA';$b=Binding 'HotkeyB'
  $a.enabled -eq ($Owner -eq 'HotkeyA') -and $b.enabled -eq ($Owner -eq 'HotkeyB')
 }
 Assert-Switch 'HotkeyA' ($Owner -eq 'HotkeyA')
 Assert-Switch 'HotkeyB' ($Owner -eq 'HotkeyB')
}
function Start-Manager {
 $script:uiRoot=$null
 $script:manager=Start-Process -FilePath $exe -ArgumentList @('--app-root',('"'+$fixture+'"')) -WorkingDirectory $projectRoot -PassThru
 Wait-For 'owned manager window' {$script:manager.Refresh();$script:manager.HasExited -or $script:manager.MainWindowHandle -ne [IntPtr]::Zero}
 Assert (-not $script:manager.HasExited) 'Manager exited during startup'
 [HotkeyUiSmoke]::MoveWindow($script:manager.MainWindowHandle,20,20,1280,1000,$true) | Out-Null
 Focus-Manager
 $script:uiRoot=[Windows.Automation.AutomationElement]::FromHandle($script:manager.MainWindowHandle)
 Wait-Ready
}
function Stop-Manager {
 if($null -ne $script:manager){
  $script:manager.Refresh()
  if(-not $script:manager.HasExited){$script:manager.CloseMainWindow() | Out-Null;Assert ($script:manager.WaitForExit(5000)) 'Manager did not close cleanly'}
  $script:manager.Dispose();$script:manager=$null;$script:uiRoot=$null
 }
}
function Record-F10 {
 Focus-Manager
 Click-Id 'hotkey-set'
 Wait-For 'capture dialog' {$null -ne (By-Id 'hotkey-dialog')}
 Focus-Manager
 Start-Sleep -Milliseconds 200 # capture arms after the opening click/keys are released
 Assert ([HotkeyUiSmoke]::PostMessage($script:manager.MainWindowHandle,0x0100,[IntPtr]121,[IntPtr]0x00440001)) 'F10 keydown post failed'
 Assert ([HotkeyUiSmoke]::PostMessage($script:manager.MainWindowHandle,0x0101,[IntPtr]121,[IntPtr](-1069285375))) 'F10 keyup post failed'
 Wait-For 'automatic capture save' {$null -eq (By-Id 'hotkey-dialog') -and $null -ne (Binding 'Capture')}
 Wait-Ready
 $binding=Binding 'Capture'
 Assert ($binding.hotkey -ieq 'F10' -and $binding.enabled -eq $false) 'New capture did not save F10 disabled'
 Assert-Switch 'Capture' $false
}
try {
 Assert ([HotkeyUiSmoke]::FindWindow([IntPtr]::Zero,'game_helper_daemon_v2') -eq [IntPtr]::Zero) 'Offline UI acceptance requires no production daemon; no service was stopped or contacted'
 Start-Manager
 Click-Id 'route-HotkeyA';Wait-Ready
 Assert-Switch 'Legacy' $true
 Assert-Owners ''
 Assert ($null -eq (By-Id 'hotkey')) 'Removed manual hotkey input remains'
 $start=By-Id 'start'
 Assert ($start.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value -eq '2') 'A startAt was not restored'
 Toggle-Route 'HotkeyA';Assert-Owners 'HotkeyA'
 Toggle-Route 'HotkeyB';Assert-Owners 'HotkeyB'
 Assert ((By-Id 'start').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value -eq '2') 'Switch activation selected a different route'
 Toggle-Route 'HotkeyB';Assert-Owners ''
 Toggle-Route 'HotkeyA';Assert-Owners 'HotkeyA'
 Click-Id 'route-Capture';Wait-Ready
 Assert (-not (By-Id 'only').Current.IsEnabled) 'Trial playback remained enabled without a service'
 Record-F10
 $before=[IO.File]::ReadAllText($configPath)
 Click-Id 'hotkey-set';Wait-For 'capture dialog for cancel' {$null -ne (By-Id 'hotkey-dialog')}
 Click-Id 'hotkey-capture-cancel'
 Wait-For 'capture canceled' {$null -eq (By-Id 'hotkey-dialog')}
 Assert ([IO.File]::ReadAllText($configPath) -ceq $before) 'Cancel changed binding config'
 Click-Id 'hotkey-clear';Wait-Ready
 Wait-For 'binding cleared' {$null -eq (Binding 'Capture')}
 Assert-Switch 'Capture' $false
 Record-F10
 (By-Id 'start').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('2')
 Click-Id 'bind';Wait-Ready
 Wait-For 'startAt saved separately' {(Binding 'Capture').startAt -eq 2}
 Assert ((Binding 'Capture').hotkey -ieq 'F10' -and (Binding 'Capture').enabled -eq $false) 'Saving startAt changed hotkey or enabled state'
 Stop-Manager
 Start-Manager
 Assert-Owners 'HotkeyA'
 Click-Id 'route-Capture';Wait-Ready
 Assert-Switch 'Capture' $false
 Assert ($null -ne (By-Id 'hotkey-current')) 'Current hotkey label missing after restart'
 Assert ((By-Id 'start').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value -eq '2') 'Saved startAt missing after restart'
 $saved=Read-Config
 Assert (-not $saved.autoStartDaemon) 'UI acceptance unexpectedly enabled service autostart'
 [ordered]@{passed=$true;fixture=$fixture;executable=$exe;capture='F10 via owned HWND only';lastEnabledWins=$true;restartRestored=$true;playbackRequested=$false} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $fixture 'summary.json') -Encoding UTF8
} catch {
 $failureRecord=$_
 $failureText=($failureRecord | Format-List * -Force | Out-String)+[Environment]::NewLine+'ScriptStackTrace:'+ [Environment]::NewLine+$failureRecord.ScriptStackTrace+[Environment]::NewLine+'Position:'+ [Environment]::NewLine+$failureRecord.InvocationInfo.PositionMessage
 [IO.File]::WriteAllText((Join-Path $fixture 'failure.txt'),$failureText,$utf8)
 Save-FailureUiTree
 throw
} finally {
 if($null -ne $script:manager){
  $script:manager.Refresh()
  if(-not $script:manager.HasExited){$script:manager.CloseMainWindow() | Out-Null;if(-not $script:manager.WaitForExit(5000)){$script:manager.Kill();$null=$script:manager.WaitForExit(3000)}}
  $script:manager.Dispose()
 }
 Write-Output "Evidence retained: $fixture"
}
Write-Output 'UI hotkey capture, cancellation, clearing, persisted switches, last-enabled ownership, and restart checks passed without playback.'
