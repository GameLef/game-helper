param(
 [string]$ProjectRoot=(Get-Location).Path,
 [string]$Executable='target\debug\game-helper.exe'
)
# Uses a separate native --probe process as its disposable capture target.
# No daemon or route playback is started. Save only into the acceptance sandbox.
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=New-Object Text.UTF8Encoding $false
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class PickerProbeNative {
 [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
 [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h,int x,int y,int w,int height,bool repaint);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern bool SetWindowText(IntPtr h,string text);
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref Point p);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out Rect r);
 [DllImport("user32.dll")] public static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);
}
'@
[PickerProbeNative]::SetThreadDpiAwarenessContext([IntPtr](-4))|Out-Null
$exe=Join-Path $ProjectRoot $Executable
$appRoot=Join-Path $ProjectRoot '.tools\acceptance sandbox'
$fixtureStem='picker-'+[guid]::NewGuid().ToString('N')
$targetTitle='GH Picker Acceptance '+[guid]::NewGuid().ToString('N')
$targetProcess=$null
$pickerProcess=$null
function Wait-Window($process) {
 for($i=0;$i -lt 50;$i++) {
  $process.Refresh()
  if($process.HasExited){throw 'Test process exited before creating its window'}
  if($process.MainWindowHandle -ne [IntPtr]::Zero){break}
  Start-Sleep -Milliseconds 100
 }
 # Debug executables may initially report their console, before the native HWND.
 Start-Sleep -Milliseconds 700
 $process.Refresh()
 if($process.MainWindowHandle -eq [IntPtr]::Zero){throw 'Test process has no window'}
}
function Target-Point {
 $position=New-Object PickerProbeNative+Point
 $position.X=100; $position.Y=100
 if(-not [PickerProbeNative]::ClientToScreen($targetProcess.MainWindowHandle,[ref]$position)){throw 'Cannot locate test client area'}
 $position
}
function Buttons {
 $condition=New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty),([Windows.Automation.ControlType]::Button)
 $root.FindAll([Windows.Automation.TreeScope]::Descendants,$condition)
}
function Click($text) {
 $button=@(Buttons | Where-Object {$_.Current.Name -eq $text})|Select-Object -First 1
 if($null -eq $button){throw "Button unavailable: $text"}
 $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
 Start-Sleep -Milliseconds 300
}
function Edits {
 $condition=New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty),([Windows.Automation.ControlType]::Edit)
 $root.FindAll([Windows.Automation.TreeScope]::Descendants,$condition)
}
try {
 $targetProcess=Start-Process -FilePath $exe -ArgumentList '--probe' -PassThru
 Wait-Window $targetProcess
 [PickerProbeNative]::SetWindowText($targetProcess.MainWindowHandle,$targetTitle)|Out-Null
 [PickerProbeNative]::MoveWindow($targetProcess.MainWindowHandle,80,80,500,400,$true)|Out-Null
 $pickerProcess=Start-Process -FilePath $exe -ArgumentList @('--picker','--app-root',('"'+$appRoot+'"')) -PassThru
 Wait-Window $pickerProcess
 $root=[Windows.Automation.AutomationElement]::FromHandle($pickerProcess.MainWindowHandle)
 $button=$null
 for($i=0;$i -lt 50;$i++) {
  $button=@(Buttons | Where-Object {$_.Current.Name.Contains($targetTitle)})|Select-Object -First 1
  if($null -ne $button){break}
  Start-Sleep -Milliseconds 100
 }
 if($null -eq $button){throw 'Dedicated native target window missing from picker list'}
 $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
 Start-Sleep -Milliseconds 300
 $condition=New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::AutomationIdProperty),'coordinates'
 $coordinates=$root.FindFirst([Windows.Automation.TreeScope]::Descendants,$condition)
 if($coordinates.Current.Name.Contains('未选择')) {
  # Exercise the visible crosshair if the target's list row was clipped.
  Click '准星选择（3秒后取光标下窗口）'
  [PickerProbeNative]::SetForegroundWindow($targetProcess.MainWindowHandle)|Out-Null
  $position=Target-Point
  [PickerProbeNative]::SetCursorPos($position.X,$position.Y)|Out-Null
  Start-Sleep -Milliseconds 3400
 }
 Click '启用 Ctrl+Alt+P'
 if(@(Buttons | Where-Object {$_.Current.Name -eq '释放 Ctrl+Alt+P'}).Count -ne 1){throw 'Global hotkey registration failed/conflicted'}
 [PickerProbeNative]::SetForegroundWindow($targetProcess.MainWindowHandle)|Out-Null
 $position=Target-Point
 [PickerProbeNative]::SetCursorPos($position.X,$position.Y)|Out-Null
 Start-Sleep -Milliseconds 150
 foreach($key in @(0x11,0x12,0x50)){[PickerProbeNative]::keybd_event($key,0,0,[UIntPtr]::Zero);Start-Sleep -Milliseconds 50}
 foreach($key in @(0x50,0x12,0x11)){[PickerProbeNative]::keybd_event($key,0,2,[UIntPtr]::Zero);Start-Sleep -Milliseconds 30}
 Start-Sleep -Milliseconds 500
 $edits=@(Edits)
 if($edits.Count -ne 3){throw "Expected exactly one sampled point, got $($edits.Count-2)"}
 $edits[0].GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($fixtureStem)
 $edits[2].GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('captured-once')
 Click '保存路线'
 $cache=Join-Path $appRoot ('routes\_cache\'+$fixtureStem+'.json')
 $route=[IO.File]::ReadAllText($cache)|ConvertFrom-Json
 if($route.points.Count -ne 1 -or $route.points[0].name -ne 'captured-once'){throw 'Picker save lost the sampled point or name edit'}
 if($route.points[0].PSObject.Properties.Name -contains 'pos'){throw 'Picker persisted obsolete point coordinates'}
 if($route.points[0].operations.Count -ne 1 -or $route.points[0].operations[0].type -ne 'click'){throw 'Picker did not create the captured click action'}
 $client=New-Object PickerProbeNative+Rect
 if(-not [PickerProbeNative]::GetClientRect($targetProcess.MainWindowHandle,[ref]$client)){throw 'Cannot read native target client bounds'}
 $expected=@((100.0/($client.Right-1)),(100.0/($client.Bottom-1)))
 $actual=$route.points[0].operations[0].pos
 if([Math]::Abs($actual[0]-$expected[0]) -gt 0.000001 -or [Math]::Abs($actual[1]-$expected[1]) -gt 0.000001){throw 'Captured click position changed'}
 $yamlText=[IO.File]::ReadAllText((Join-Path $appRoot ('routes\'+$fixtureStem+'.yaml')))
 if($yamlText -match '(?m)^  pos:'){throw 'Picker YAML contains obsolete point coordinates'}
 Click '释放 Ctrl+Alt+P'
 $pickerProcess.CloseMainWindow()|Out-Null
 if(-not $pickerProcess.WaitForExit(5000)){throw 'Saved picker did not exit cleanly'}
 Write-Output 'Passed: native target selection, global capture while picker unfocused, point-name edit, default click at captured coordinates, YAML/cache without point-level pos, hotkey release and clean close. No playback requested.'
} finally {
 foreach($key in @(0x50,0x12,0x11)){[PickerProbeNative]::keybd_event($key,0,2,[UIntPtr]::Zero)}
 foreach($process in @($pickerProcess,$targetProcess)) {
  if($null -ne $process -and -not $process.HasExited) {
   $process.CloseMainWindow()|Out-Null
   if(-not $process.WaitForExit(1000)){$process.Kill()}
  }
 }
}
