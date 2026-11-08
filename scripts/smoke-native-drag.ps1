#requires -Version 7.0
[CmdletBinding()]
param([string]$AutoHotkey = 'C:\Program Files\AutoHotkey\v2\AutoHotkey64.exe')
# Real input is restricted to an owned, empty WinForms target. No Rust GUI is launched.
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$fixture = Join-Path $projectRoot ('.tools\native-drag-' + [guid]::NewGuid().ToString('N'))
$utf8 = [Text.UTF8Encoding]::new($false)
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()
$results = [Collections.Generic.List[object]]::new()
$targetTitle = 'NativeDragTarget_' + [guid]::NewGuid().ToString('N')
$daemonTitle = 'NativeDragDaemon_' + [guid]::NewGuid().ToString('N')
foreach ($folder in @('ahk\lib', 'routes\_cache', 'logs')) { [IO.Directory]::CreateDirectory((Join-Path $fixture $folder)) | Out-Null }
function Put([string]$Name, [string]$Text) { [IO.File]::WriteAllText((Join-Path $fixture $Name), $Text, $utf8) }
function Wait-For([string]$Description, [scriptblock]$Check, [int]$Timeout = 10000) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    do { if (& $Check) { return }; Start-Sleep -Milliseconds 40 } while ($watch.ElapsedMilliseconds -lt $Timeout)
    throw "Timeout: $Description; evidence: $fixture"
}
function Launch([string]$File, [string[]]$Arguments, [string]$Label) {
    $p = Start-Process -FilePath $File -ArgumentList $Arguments -WorkingDirectory $fixture -RedirectStandardOutput (Join-Path $fixture "$Label.stdout.txt") -RedirectStandardError (Join-Path $fixture "$Label.stderr.txt") -PassThru
    $owned.Add($p)
    return $p
}
function Assert([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class NativeDragSmoke {
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(IntPtr cls,string title);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint pid);
 [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int key);
 [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint x,uint y,uint data,UIntPtr extra);
 [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point p);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd,out Rect r);
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hwnd,ref Point p);
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern uint RegisterWindowMessage(string name);
 [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr SendMessageTimeout(IntPtr hwnd,uint msg,IntPtr w,IntPtr l,uint flags,uint timeout,out UIntPtr result);
 [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
 [StructLayout(LayoutKind.Sequential)] struct CopyData { public UIntPtr Data; public int Length; public IntPtr Text; }
 public static ulong Run(IntPtr hwnd,string json) {
  byte[] bytes=System.Text.Encoding.UTF8.GetBytes(json+"\0");
  IntPtr text=Marshal.AllocHGlobal(bytes.Length), ptr=Marshal.AllocHGlobal(Marshal.SizeOf<CopyData>());
  try { Marshal.Copy(bytes,0,text,bytes.Length); Marshal.StructureToPtr(new CopyData {Data=(UIntPtr)1,Length=bytes.Length,Text=text},ptr,false); UIntPtr result;
   if(SendMessageTimeout(hwnd,0x4a,IntPtr.Zero,ptr,2,3000,out result)==IntPtr.Zero) throw new Exception("WM_COPYDATA timed out"); return result.ToUInt64();
  } finally {Marshal.FreeHGlobal(ptr);Marshal.FreeHGlobal(text);}
 }
}
'@
$previousForeground = [NativeDragSmoke]::GetForegroundWindow()
$previousCursor = [NativeDragSmoke+Point]::new()
[NativeDragSmoke]::GetCursorPos([ref]$previousCursor) | Out-Null
$existingDaemon = [NativeDragSmoke]::FindWindow([IntPtr]::Zero, 'game_helper_daemon_v2')
$target = $null
$targetHwnd = [IntPtr]::Zero
try {
    Assert (Test-Path -LiteralPath $AutoHotkey) 'AutoHotkey v2 executable missing'
    foreach ($name in @('runner.ahk', 'lib\executor.ahk', 'lib\JSON.ahk')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot "ahk\$name") -Destination (Join-Path $fixture "ahk\$name")
    }
    $daemon = [IO.File]::ReadAllText((Join-Path $projectRoot 'ahk\daemon.ahk'))
    $daemon = $daemon.Replace('#SingleInstance Force', '#SingleInstance Off').Replace('"game_helper_daemon_v2"', ('"' + $daemonTitle + '"'))
    Put 'ahk\daemon.ahk' $daemon
    Put 'config.json' '{"hotkeys":{"stopAll":"^!+F24","bindings":[]}}'
    # External WinForms process: unlike AHK Gui(), this has a bounded startup and a plain empty surface.
    Put 'target.ps1' @'
param([string]$Title,[string]$Output,[string]$Ready)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies @('System.Windows.Forms','System.Drawing','System.Drawing.Primitives','System.ComponentModel.Primitives') @"
using System;
using System.IO;
using System.Windows.Forms;
using System.Drawing;
public class DragSurface : Form {
 readonly string output;
 public DragSurface(string title,string path) {
  output=path; Text=title; StartPosition=FormStartPosition.Manual; Location=new Point(40,40); ClientSize=new Size(800,480); TopMost=true; BackColor=Color.FromArgb(33,40,48);
 }
 void Record(string kind, MouseEventArgs e) { File.AppendAllText(output,DateTime.UtcNow.Ticks+","+kind+","+e.X+","+e.Y+","+(int)e.Button+Environment.NewLine); }
 protected override void OnMouseDown(MouseEventArgs e) { base.OnMouseDown(e); Record("down",e); }
 protected override void OnMouseUp(MouseEventArgs e) { base.OnMouseUp(e); Record("up",e); }
 protected override void OnMouseMove(MouseEventArgs e) { base.OnMouseMove(e); if(e.Button==MouseButtons.Left) Record("move",e); }
}
"@
[Windows.Forms.Application]::EnableVisualStyles()
$form=[DragSurface]::new($Title,$Output)
$form.add_Shown({ [IO.File]::WriteAllText($Ready, [string]$form.Handle); $form.Activate() }.GetNewClosure())
[Windows.Forms.Application]::Run($form)
'@
    $eventsPath = Join-Path $fixture 'events.csv'
    Put 'events.csv' ''
    $readyPath = Join-Path $fixture 'target.ready'
    $target = Launch (Get-Process -Id $PID).Path @('-NoProfile','-STA','-File',('"' + (Join-Path $fixture 'target.ps1') + '"'),'-Title',$targetTitle,'-Output',('"' + $eventsPath + '"'),'-Ready',('"' + $readyPath + '"')) 'target'
    Wait-For 'owned target ready' { (Test-Path -LiteralPath $readyPath) -or $target.HasExited }
    Assert (-not $target.HasExited) 'Target exited: inspect target.stderr.txt'
    $targetHwnd = [IntPtr][long]([IO.File]::ReadAllText($readyPath))
    [uint32]$windowPid = 0
    [NativeDragSmoke]::GetWindowThreadProcessId($targetHwnd,[ref]$windowPid) | Out-Null
    Assert ($windowPid -eq $target.Id) 'Target HWND ownership mismatch'
    function Foreground-Target {
        Assert (-not $target.HasExited) 'Owned target is no longer running'
        [NativeDragSmoke]::SetForegroundWindow($targetHwnd) | Out-Null
        Wait-For 'owned target foreground' { [NativeDragSmoke]::GetForegroundWindow() -eq $targetHwnd }
    }
    function Events { @(Get-Content -LiteralPath $eventsPath | Where-Object { $_ } | ForEach-Object { $p=$_ -split ','; [pscustomobject]@{ticks=[long]$p[0];kind=$p[1];x=[int]$p[2];y=[int]$p[3];button=[int]$p[4]} }) }
    function Route([string]$Name, $Operations) {
        $route = @{format='route/v1'; name=$Name; target=@{title=$targetTitle};points=@(@{id=1;name='isolated';operations=@($Operations)})}
        Put "routes\_cache\$Name.json" ($route | ConvertTo-Json -Depth 15)
    }
    function Verify-Drag([string]$Name, [int]$Before, [bool]$Identical=$false) {
        Wait-For "$Name mouse up" { @(Events).Count -gt $Before -and @(Events)[-1].kind -eq 'up' }
        $all = @(Events)
        $ev = @($all | Select-Object -Skip $Before)
        $down = @($ev | Where-Object kind -eq down)
        $up = @($ev | Where-Object kind -eq up)
        Assert ($down.Count -eq 1 -and $up.Count -eq 1) "$Name expected one left down/up"
        Assert ($down[0].button -eq 1048576 -and $up[0].button -eq 1048576) "$Name expected left button"
        $endX = if ($Identical) {160} else {639}
        $endY = if ($Identical) {96} else {383}
        Assert ($down[0].x -eq 160 -and $down[0].y -eq 96) "$Name wrong start"
        Assert ($up[0].x -eq $endX -and $up[0].y -eq $endY) "$Name wrong endpoint"
        Assert (([NativeDragSmoke]::GetAsyncKeyState(1) -band 0x8000) -eq 0) "$Name left remains down"
        $ev | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $fixture "$Name.events.json") -Encoding utf8
        $result = [pscustomobject]@{name=$Name;downUpMs=[math]::Round(($up[0].ticks-$down[0].ticks)/10000,2);moveEvents=@($ev | Where-Object kind -eq move).Count;leftReleased=$true;endX=$up[0].x;endY=$up[0].y}
        $results.Add($result)
        return $result
    }
    foreach ($case in @(@{name='runner-default';speed=$null},@{name='runner-speed0';speed=0},@{name='runner-speed40';speed=40},@{name='runner-identical';speed=15})) {
        $op = @{type='drag_native';from=@(0.2,0.2);to=@(0.8,0.8)}
        if ($null -ne $case.speed) { $op.speed=$case.speed }
        if ($case.name -eq 'runner-identical') { $op.to=@(0.2,0.2) }
        Route $case.name @($op)
        Foreground-Target
        $before = @(Events).Count
        $p = Launch $AutoHotkey @('/ErrorStdOut',('"'+(Join-Path $fixture 'ahk\runner.ahk')+'"'),('"'+(Join-Path $fixture "routes\_cache\$($case.name).json")+'"'),'--stop','^!+F24') $case.name
        Wait-For "$($case.name) exit" { $p.HasExited } 20000
        Assert ($p.ExitCode -eq 0) "$($case.name) runner failed: $($p.ExitCode)"
        $null = Verify-Drag $case.name $before ($case.name -eq 'runner-identical')
        Start-Sleep -Milliseconds 1100 # runner log names have second precision; preserve each case separately.
    }
    $fast = $results | Where-Object name -eq runner-speed0
    $slow = $results | Where-Object name -eq runner-speed40
    Assert ($slow.moveEvents -gt ($fast.moveEvents+3)) 'Nonzero native speed did not expose intermediate cursor positions'
    Assert ($slow.downUpMs -gt ($fast.downUpMs+100)) 'Nonzero speed did not take measurably longer than speed 0'
    # This harness includes the unmodified real executor and checks per-thread send mode and stop semantics.
    Put 'harness.ahk' @'
#Requires AutoHotkey v2.0
#SingleInstance Off
#Include ahk\lib\JSON.ahk
#Include ahk\lib\executor.ahk
EXE_Setup(A_ScriptDir)
SetMouseDelay(0)
route := JSON.parse(FileRead(A_Args[1], "UTF-8"))
hwnd := EXE_FindTargetWindow(route["target"])
WinActivate("ahk_id " hwnd)
if !WinWaitActive("ahk_id " hwnd,,3)
    ExitApp(4)
WinGetClientPos(&x,&y,&w,&h,"ahk_id " hwnd)
client := {x:x,y:y,w:w,h:h}
report := Map("version",A_AhkVersion)
SendMode("Input")
EXE_ExecuteOp(Map("type","drag_native","from",[0.2,0.2],"to",[0.8,0.8]),client)
report["defaultRestoresInput"] := A_SendMode = "Input"
SendMode("Play")
EXE_ExecuteOp(Map("type","drag_native","from",[0.2,0.2],"to",[0.2,0.2],"speed",0),client)
report["restoresPlay"] := A_SendMode = "Play"
SendMode("Input")
EXE_StopFlag := true
EXE_ExecuteOp(Map("type","drag_native","from",[0.2,0.2],"to",[0.8,0.8],"speed",0),client)
report["earlyStopRestoresInput"] := A_SendMode = "Input"
EXE_StopFlag := false
report["exceptionPathCoverage"] := "Not induced; reviewed nested finally only"
StopOnHold() {
    if GetKeyState("LButton") {
        SetTimer(StopOnHold,0)
        EXE_StopAllHandler()
    }
}
SetTimer(StopOnHold,10)
ok := EXE_RunRoute(route)
SetTimer(StopOnHold,0)
report["stopRouteReturnsFalse"] := !ok
report["stopLeftReleased"] := !GetKeyState("LButton")
report["stopRestoresInput"] := A_SendMode = "Input"
report["stopClearsRunning"] := !EXE_Running
FileAppend(JSON.stringify(report),A_ScriptDir "\harness-result.json","UTF-8")
ExitApp(0)
'@
    Route 'harness-stop' @(@{type='drag_native';from=@(0.2,0.2);to=@(0.8,0.8);speed=60},@{type='key';key='F23'})
    Foreground-Target
    $harness = Launch $AutoHotkey @('/ErrorStdOut',('"'+(Join-Path $fixture 'harness.ahk')+'"'),('"'+(Join-Path $fixture 'routes\_cache\harness-stop.json')+'"')) 'harness'
    Wait-For 'deterministic executor harness exit' { $harness.HasExited } 20000
    Assert ($harness.ExitCode -eq 0) 'Harness failed: inspect harness.stderr.txt and stdout'
    $harnessReport = [IO.File]::ReadAllText((Join-Path $fixture 'harness-result.json')) | ConvertFrom-Json
    foreach ($key in @('defaultRestoresInput','restoresPlay','earlyStopRestoresInput','stopRouteReturnsFalse','stopLeftReleased','stopRestoresInput','stopClearsRunning')) { Assert $harnessReport.$key "Harness assertion failed: $key" }
    $harnessLog = @(Get-ChildItem -LiteralPath (Join-Path $fixture 'logs') -Filter '*.log' | Sort-Object LastWriteTime)[-1]
    $logText = [IO.File]::ReadAllText($harnessLog.FullName)
    Assert ($logText.Contains('STOP at point#1 op#2')) 'Stop did not skip the following action'
    Assert (-not $logText.Contains('op key {F23}')) 'Following action executed despite stop'
    Assert ($logText.Contains('speed=2')) 'Omitted native speed did not resolve to 2'
    Copy-Item -LiteralPath $harnessLog.FullName -Destination (Join-Path $fixture 'harness-executor.log')
    Start-Sleep -Milliseconds 1100
    Route 'daemon-native' @(@{type='drag_native';from=@(0.2,0.2);to=@(0.8,0.8);speed=20})
    Foreground-Target
    $daemonProcess = Launch $AutoHotkey @('/ErrorStdOut',('"'+(Join-Path $fixture 'ahk\daemon.ahk')+'"')) 'daemon'
    $script:daemonHwnd = [IntPtr]::Zero
    Wait-For 'isolated daemon hidden window' { $script:daemonHwnd=[NativeDragSmoke]::FindWindow([IntPtr]::Zero,$daemonTitle); $script:daemonHwnd -ne [IntPtr]::Zero -or $daemonProcess.HasExited }
    Assert (-not $daemonProcess.HasExited) 'Isolated daemon exited during startup'
    [NativeDragSmoke]::GetWindowThreadProcessId($daemonHwnd,[ref]$windowPid) | Out-Null
    Assert ($windowPid -eq $daemonProcess.Id) 'Daemon HWND ownership mismatch'
    $ping = [NativeDragSmoke]::RegisterWindowMessage('GameHelperDaemon_Ping')
    [UIntPtr]$pingReply = [UIntPtr]::Zero
    $pingOk = [NativeDragSmoke]::SendMessageTimeout($daemonHwnd,$ping,[IntPtr]::Zero,[IntPtr]::Zero,2,3000,[ref]$pingReply)
    Assert ($pingOk -ne [IntPtr]::Zero -and $pingReply.ToUInt64() -eq 0) 'Daemon ping failed or regular hotkeys registered'
    Foreground-Target
    $before = @(Events).Count
    $accepted = [NativeDragSmoke]::Run($daemonHwnd,'{"route":"daemon-native","startAt":1,"only":0}')
    Assert ($accepted -eq 1) 'Daemon WM_COPYDATA run was rejected'
    $null = Verify-Drag 'daemon-native' $before
    Assert ([NativeDragSmoke]::FindWindow([IntPtr]::Zero,'game_helper_daemon_v2') -eq $existingDaemon) 'Existing user daemon window changed'
    $summary = [ordered]@{passed=$true;fixture=$fixture;interpreter=$AutoHotkey;targetPid=$target.Id;daemonPid=$daemonProcess.Id;existingDaemonUnchanged=$true;runnerHash=(Get-FileHash (Join-Path $fixture 'ahk\runner.ahk')).Hash;executorHash=(Get-FileHash (Join-Path $fixture 'ahk\lib\executor.ahk')).Hash;cases=@($results);harness=$harnessReport}
    Put 'summary.json' ($summary | ConvertTo-Json -Depth 12)
    $summary | ConvertTo-Json -Depth 12
} catch {
    Put 'failure.txt' ($_ | Out-String)
    throw
} finally {
    # Never kill by executable or global daemon title: only Process objects created above.
    [NativeDragSmoke]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    foreach ($p in $owned) {
        $p.Refresh()
        if (-not $p.HasExited) { $p.Kill(); $null=$p.WaitForExit(3000) }
        $p.Dispose()
    }
    [NativeDragSmoke]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    [NativeDragSmoke]::SetCursorPos($previousCursor.X,$previousCursor.Y) | Out-Null
    if ($previousForeground -ne [IntPtr]::Zero) { [NativeDragSmoke]::SetForegroundWindow($previousForeground) | Out-Null }
    Write-Output "Evidence retained: $fixture"
}
