param([string]$ProjectRoot=(Get-Location).Path)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=New-Object Text.UTF8Encoding $false
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class PickerProbeNative {
 [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr context);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);
}
'@
[PickerProbeNative]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null
$target=New-Object Windows.Forms.Form
$target.Text='GH Picker Acceptance '+[guid]::NewGuid().ToString('N')
$target.Size=New-Object Drawing.Size 500,400
$target.StartPosition='Manual'
$target.Location=New-Object Drawing.Point 80,80
$target.Show()
[Windows.Forms.Application]::DoEvents()
$exe=Join-Path $ProjectRoot 'target\debug\game-helper.exe'
$appRoot=Join-Path $ProjectRoot '.tools\acceptance sandbox'
$fixtureStem='picker-'+[guid]::NewGuid().ToString('N')
$p=Start-Process -FilePath $exe -ArgumentList @('--picker','--app-root',('"'+$appRoot+'"')) -PassThru
try {
 for($i=0;$i -lt 40 -and $p.MainWindowHandle -eq [IntPtr]::Zero;$i++){Start-Sleep -Milliseconds 100; $p.Refresh(); [Windows.Forms.Application]::DoEvents()}
 $root=[Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
 function Buttons { $c=New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty),([Windows.Automation.ControlType]::Button); $root.FindAll([Windows.Automation.TreeScope]::Descendants,$c) }
 function Click($text) { $b=@(Buttons | Where-Object {$_.Current.Name -eq $text})|Select-Object -First 1; if($null -eq $b){throw "Button unavailable: $text"}; $b.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke(); Start-Sleep -Milliseconds 300; [Windows.Forms.Application]::DoEvents() }
 function Edits { $c=New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty),([Windows.Automation.ControlType]::Edit); $root.FindAll([Windows.Automation.TreeScope]::Descendants,$c) }
 Start-Sleep -Milliseconds 400
 $b=@(Buttons | Where-Object {$_.Current.Name.Contains($target.Text)})|Select-Object -First 1
 if($null -eq $b){throw 'Dedicated target window missing from picker list'}
 $b.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
 Start-Sleep -Milliseconds 400
 $condition=New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::AutomationIdProperty),'coordinates'
 $coordinates=$root.FindFirst([Windows.Automation.TreeScope]::Descendants,$condition)
 if($coordinates.Current.Name.Contains('未选择')) {
   # Off-screen UIA Invoke may be clipped. Exercise the visible crosshair path.
   Click '准星选择（3秒后取光标下窗口）'
   [PickerProbeNative]::SetForegroundWindow($target.Handle)|Out-Null
   $crosshairPoint=$target.PointToScreen((New-Object Drawing.Point 100,100))
   [PickerProbeNative]::SetCursorPos($crosshairPoint.X,$crosshairPoint.Y)|Out-Null
   for($j=0;$j -lt 35;$j++){[Windows.Forms.Application]::DoEvents();Start-Sleep -Milliseconds 100}
 }
 Click '启用 Ctrl+Alt+P'
 if(@(Buttons | Where-Object {$_.Current.Name -eq '释放 Ctrl+Alt+P'}).Count -ne 1){throw 'Global hotkey registration failed/conflicted'}
 [PickerProbeNative]::SetForegroundWindow($target.Handle)|Out-Null
 $point=$target.PointToScreen((New-Object Drawing.Point 100,100))
 [PickerProbeNative]::SetCursorPos($point.X,$point.Y)|Out-Null
 [Windows.Forms.Application]::DoEvents()
 Start-Sleep -Milliseconds 150
 foreach($key in @(0x11,0x12,0x50)){[PickerProbeNative]::keybd_event($key,0,0,[UIntPtr]::Zero);Start-Sleep -Milliseconds 50}
 foreach($key in @(0x50,0x12,0x11)){[PickerProbeNative]::keybd_event($key,0,2,[UIntPtr]::Zero);Start-Sleep -Milliseconds 30}
 Start-Sleep -Milliseconds 500
 [Windows.Forms.Application]::DoEvents()
 $edits=@(Edits)
 if($edits.Count -ne 3){$nodes=$root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition);$details=@($nodes | Where-Object {$_.Current.AutomationId -in @('picker-status','coordinates')} | ForEach-Object {$_.Current.Name});throw "Expected exactly 1 sampled point, got $($edits.Count-2); $details"}
 $edits[0].GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($fixtureStem)
 $edits[2].GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('captured-once')
 Click '保存路线'
 $cache=Join-Path $appRoot ('routes\_cache\'+$fixtureStem+'.json')
 $route=[IO.File]::ReadAllText($cache)|ConvertFrom-Json
 if($route.points.Count -ne 1 -or $route.points[0].name -ne 'captured-once'){throw 'Picker save lost the sampled point or name edit'}
 if($route.points[0].pos[0] -lt 0 -or $route.points[0].pos[0] -gt 1){throw 'Out-of-range coordinates'}
 Click '释放 Ctrl+Alt+P'
 $p.CloseMainWindow()|Out-Null
 if(-not $p.WaitForExit(5000)){throw 'Saved picker did not exit cleanly'}
 Write-Output 'Observed: dedicated target selection, global capture while picker unfocused (exactly one point), point-name edit, YAML/cache save, hotkey release and clean close.'
} finally {
 foreach($key in @(0x50,0x12,0x11)){[PickerProbeNative]::keybd_event($key,0,2,[UIntPtr]::Zero)}
 $target.Close(); $target.Dispose()
 if(-not $p.HasExited){$p.CloseMainWindow()|Out-Null}
}
