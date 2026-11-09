#requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$BundleDir)
# Copies are disposable. No input is sent and no production daemon receives IPC.
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$bundle=[IO.Path]::GetFullPath($BundleDir)
if(-not $bundle.StartsWith((Join-Path $root 'dist')+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'Bundle must be inside project dist'}
$manifest=Get-Content -LiteralPath (Join-Path $bundle 'checksums.json') -Raw -Encoding UTF8 | ConvertFrom-Json
foreach($entry in $manifest){if((Get-FileHash -LiteralPath (Join-Path $bundle $entry.Path) -Algorithm SHA256).Hash -ne $entry.SHA256){throw ('Checksum mismatch: '+$entry.Path)}}
$destination=Join-Path $root ('.tools\便携 测试-'+[guid]::NewGuid().ToString('N'))
Copy-Item -LiteralPath $bundle -Destination $destination -Recurse
$working=Join-Path $destination 'unrelated working directory'
New-Item -ItemType Directory -Path $working -Force | Out-Null
$utf8=[Text.UTF8Encoding]::new($false)
$configPath=Join-Path $destination 'config.json'
$config=Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
# Manager's compiled IPC title is global. Keep it read-only even if a user's service exists.
$config.autoStartDaemon=$false
$config.autoReloadOnSave=$false
$config.hotkeys.bindings=@()
$config.hotkeys.stopAll='^!+F24'
$config | Add-Member -NotePropertyName ahkPath -NotePropertyValue (Join-Path $destination 'missing legacy installation\AutoHotkey64.exe') -Force
[IO.File]::WriteAllText($configPath,($config | ConvertTo-Json -Depth 30),$utf8)
$runtime=Join-Path $destination 'ahk\runtime\AutoHotkey64.exe'
foreach($file in @($runtime,(Join-Path $destination 'ahk\runtime\license.txt'),(Join-Path $destination 'ahk\runtime\source.zip'),(Join-Path $destination 'ahk\runtime\metadata.json'))){if(-not (Test-Path -LiteralPath $file -PathType Leaf)){throw "Missing bundled runtime asset: $file"}}
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class PortableDaemonSmoke {
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(IntPtr cls,string title);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint pid);
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern uint RegisterWindowMessage(string name);
 [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr SendMessageTimeout(IntPtr hwnd,uint msg,IntPtr w,IntPtr l,uint flags,uint timeout,out UIntPtr result);
}
'@
function Wait-For([string]$Label,[scriptblock]$Check,[int]$Timeout=10000){
 $watch=[Diagnostics.Stopwatch]::StartNew()
 do {if(& $Check){return};Start-Sleep -Milliseconds 100} while($watch.ElapsedMilliseconds -lt $Timeout)
 throw "Timed out: $Label; evidence: $destination"
}
$production=[PortableDaemonSmoke]::FindWindow([IntPtr]::Zero,'game_helper_daemon_v2')
$processes=[Collections.Generic.List[Diagnostics.Process]]::new()
$daemonProcess=$null
$windows=[Collections.Generic.List[object]]::new()
function Save-PortableFailureUi {
 foreach($entry in $windows){
  try {
   $ownedWindow=$entry.process
   $ownedWindow.Refresh()
   if($ownedWindow.HasExited -or $ownedWindow.MainWindowHandle -eq [IntPtr]::Zero){continue}
   $diagnosticRoot=[Windows.Automation.AutomationElement]::FromHandle($ownedWindow.MainWindowHandle)
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
   $prefix='failure-uia-'+$entry.mode
   [IO.File]::WriteAllText((Join-Path $destination ($prefix+'.json')),(ConvertTo-Json -InputObject @($rows.ToArray()) -Depth 8),$utf8)
   $lines=@($rows | ForEach-Object {if($_.Contains('unavailable')){'Unavailable: '+$_.unavailable}else{'['+$_.runtimeId+'] parent=['+$_.parentRuntimeId+'] '+$_.controlType+' id="'+$_.automationId+'" name="'+$_.name+'" enabled='+$_.enabled+' offscreen='+$_.offscreen+' focus='+$_.focus+' bounds='+$_.bounds+' patterns='+($_.patterns -join ', ')}})
   [IO.File]::WriteAllText((Join-Path $destination ($prefix+'.txt')),($lines -join [Environment]::NewLine),$utf8)
  } catch {
   [IO.File]::WriteAllText((Join-Path $destination ('failure-uia-'+$entry.mode+'-dump-error.txt')),($_ | Out-String),$utf8)
  }
 }
}
try {
 foreach($mode in @('manager','picker')) {
  $options=@{FilePath=(Join-Path $destination 'game-helper.exe');WorkingDirectory=$working;PassThru=$true}
  if($mode -eq 'picker'){$options.ArgumentList='--picker'}
  $p=Start-Process @options
  $processes.Add($p)
  $windows.Add(@{mode=$mode;process=$p})
  Wait-For "$mode native window" {$p.Refresh();$p.HasExited -or $p.MainWindowHandle -ne [IntPtr]::Zero}
  if($p.HasExited){throw "$mode exited before opening a window"}
  [PortableDaemonSmoke]::SetForegroundWindow($p.MainWindowHandle) | Out-Null
  Wait-For "$mode foreground" {[PortableDaemonSmoke]::GetForegroundWindow() -eq $p.MainWindowHandle}
  $element=[Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
  # Prime the provider after activation, then reacquire it on every control poll.
  $null=$element.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
  $condition=New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty),([Windows.Automation.ControlType]::Edit)
  Wait-For "$mode accessible controls" {
   $p.Refresh()
   if($p.HasExited){throw "$mode exited while waiting for accessible controls"}
   $element=[Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
   $scope=[Windows.Automation.TreeScope]::Descendants
   if($mode -eq 'manager'){
    $status=$element.FindFirst($scope,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty,'manager-status'))
    return $null -ne $status -and $status.Current.Name -notmatch '正在加载|正在处理' -and $element.FindAll($scope,$condition).Count -ge 2
   }
   # Picker coordinates are read-only groups; verify its actual controls and
   # initial no-target behavior rather than assuming multiple editable fields.
   $nodes=@{}
   foreach($id in @('picker-root','coordinates','picker-status','hotkey','capture','picker-search','target-summary')){
    $node=$element.FindFirst($scope,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty,$id))
    if($null -eq $node){return $false}
    $nodes[$id]=$node
   }
   $searchSupportsValue=@($nodes['picker-search'].GetSupportedPatterns() | ForEach-Object {$_.ProgrammaticName}) -contains 'ValuePatternIdentifiers.Pattern'
   return $searchSupportsValue -and -not $nodes['capture'].Current.IsEnabled -and $nodes['target-summary'].Current.Name -match '尚未选择目标窗口'
  }
  Write-Output "Observed: $mode opened from a Chinese/space path with a different working directory."
 }
 # Run only the copy's interpreter and uniquely named script. No download, registry lookup,
 # installed interpreter, target route, or execution request is used in this offline probe.
 $title='PortableDaemon_'+[guid]::NewGuid().ToString('N')
 $daemonScript=Join-Path $destination 'ahk\daemon.ahk'
 $script=[IO.File]::ReadAllText($daemonScript)
 if(-not $script.Contains('"game_helper_daemon_v2"')){throw 'Daemon title marker changed; cannot isolate portable probe'}
 $script=$script.Replace('#SingleInstance Force','#SingleInstance Off').Replace('"game_helper_daemon_v2"',('"'+$title+'"'))
 [IO.File]::WriteAllText($daemonScript,$script,$utf8)
 $daemonProcess=Start-Process -FilePath $runtime -ArgumentList @('/ErrorStdOut',('"'+$daemonScript+'"')) -WorkingDirectory $working -RedirectStandardOutput (Join-Path $destination 'daemon.stdout.txt') -RedirectStandardError (Join-Path $destination 'daemon.stderr.txt') -PassThru
 $processes.Add($daemonProcess)
 $script:probeWindow=[IntPtr]::Zero
 Wait-For 'copied AHK service' {$script:probeWindow=[PortableDaemonSmoke]::FindWindow([IntPtr]::Zero,$title);$daemonProcess.Refresh();$probeWindow -ne [IntPtr]::Zero -or $daemonProcess.HasExited}
 if($daemonProcess.HasExited){throw 'Bundled AHK service exited; inspect daemon.stderr.txt'}
 [uint32]$owner=0
 [PortableDaemonSmoke]::GetWindowThreadProcessId($probeWindow,[ref]$owner) | Out-Null
 if($owner -ne $daemonProcess.Id){throw 'Probe HWND ownership mismatch'}
 $actual=$daemonProcess.MainModule.FileName
 if(-not [string]::Equals([IO.Path]::GetFullPath($actual),[IO.Path]::GetFullPath($runtime),[StringComparison]::OrdinalIgnoreCase)){throw "Unexpected interpreter: $actual"}
 [UIntPtr]$reply=[UIntPtr]::Zero
 $ping=[PortableDaemonSmoke]::RegisterWindowMessage('GameHelperDaemon_Ping')
 $sent=[PortableDaemonSmoke]::SendMessageTimeout($probeWindow,$ping,[IntPtr]::Zero,[IntPtr]::Zero,2,3000,[ref]$reply)
 if($sent -eq [IntPtr]::Zero -or $reply.ToUInt64() -ne 0){throw 'Copied daemon not idle or registered unexpected route hotkeys'}
 $stop=[PortableDaemonSmoke]::RegisterWindowMessage('GameHelperDaemon_Stop')
 $sent=[PortableDaemonSmoke]::SendMessageTimeout($probeWindow,$stop,[IntPtr]::Zero,[IntPtr]::Zero,2,3000,[ref]$reply)
 if($sent -eq [IntPtr]::Zero -or $reply.ToUInt64() -ne 1 -or -not $daemonProcess.WaitForExit(5000)){throw 'Copied daemon did not stop cleanly'}
 if([PortableDaemonSmoke]::FindWindow([IntPtr]::Zero,'game_helper_daemon_v2') -ne $production){throw 'Production daemon window changed'}
 [ordered]@{passed=$true;fixture=$destination;interpreter=$actual;invalidLegacyPath=$config.ahkPath;daemonPid=$daemonProcess.Id;productionDaemonUnchanged=$true;offlineProbe='Explicit bundled interpreter, no network or installation lookup; Rust resolver covered separately'} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $destination 'portable-summary.json') -Encoding UTF8
} catch {
 $failureRecord=$_
 $failureText=($failureRecord | Format-List * -Force | Out-String)+[Environment]::NewLine+'ScriptStackTrace:'+ [Environment]::NewLine+$failureRecord.ScriptStackTrace+[Environment]::NewLine+'Position:'+ [Environment]::NewLine+$failureRecord.InvocationInfo.PositionMessage
 [IO.File]::WriteAllText((Join-Path $destination 'failure.txt'),$failureText,$utf8)
 Save-PortableFailureUi
 throw
} finally {
 foreach($p in $processes){
  $p.Refresh()
  if(-not $p.HasExited){
   if($null -ne $daemonProcess -and $p.Id -eq $daemonProcess.Id){$p.Kill();$null=$p.WaitForExit(3000)}
   else {$p.CloseMainWindow() | Out-Null;if(-not $p.WaitForExit(5000)){$p.Kill();$null=$p.WaitForExit(3000);Write-Warning 'Owned portable window required forced cleanup'}}
  }
  $p.Dispose()
 }
 Write-Output "Evidence retained: $destination"
}
Write-Output 'Bundle checksums matched; manager/picker opened; the bundled AHK service answered Ping and Stop with no playback.'
