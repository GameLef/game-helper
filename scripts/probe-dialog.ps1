param([Parameter(Mandatory = $true)][int]$TargetProcessId)
$ErrorActionPreference = 'Stop'
$env:TEMP = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../.tools/tmp'))
$env:TMP = $env:TEMP
[IO.Directory]::CreateDirectory($env:TEMP) | Out-Null
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class NativeDialogProbe {
    [DllImport("user32.dll")] public static extern IntPtr GetLastActivePopup(IntPtr owner);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder text, int capacity);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint message, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
}
'@
$handle = (Get-Process -Id $TargetProcessId).MainWindowHandle
$root = [Windows.Automation.AutomationElement]::FromHandle($handle)
$openName = -join [char[]]@(0x6253,0x5F00,0x6587,0x4EF6,0x2026)
$condition = New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::NameProperty), $openName
$openButton = $root.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
if ($null -eq $openButton) { throw 'File dialog button missing.' }
$openButton.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
$dialog = [IntPtr]::Zero
$deadline = [DateTime]::UtcNow.AddSeconds(5)
do {
    $popup = [NativeDialogProbe]::GetLastActivePopup($handle)
    $class = New-Object Text.StringBuilder 128
    [uint32]$ownerProcess = 0
    [NativeDialogProbe]::GetWindowThreadProcessId($popup, [ref]$ownerProcess) | Out-Null
    [NativeDialogProbe]::GetClassName($popup, $class, $class.Capacity) | Out-Null
    if ($popup -ne $handle -and $ownerProcess -eq $TargetProcessId -and $class.ToString() -eq '#32770') { $dialog = $popup; break }
    Start-Sleep -Milliseconds 100
} while ([DateTime]::UtcNow -lt $deadline)
if ($dialog -eq [IntPtr]::Zero) { throw 'Owned native dialog not found; check incomplete.' }
# IDCANCEL is sent only to the verified owned standard dialog, never another app.
if (-not [NativeDialogProbe]::PostMessage($dialog, 0x111, [IntPtr]2, [IntPtr]::Zero)) { throw 'Cancel request failed.' }
$deadline = [DateTime]::UtcNow.AddSeconds(3)
while ([NativeDialogProbe]::IsWindow($dialog) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
if ([NativeDialogProbe]::IsWindow($dialog)) { throw 'Dialog did not close after cancellation.' }
Write-Output 'Native file dialog opened and cancelled without selecting or reading a file.'
