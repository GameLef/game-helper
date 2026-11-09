#requires -Version 7.0
[CmdletBinding()]
param([string]$AutoHotkey = '')
# Real input is restricted to an owned, empty WinForms target. No Rust GUI is launched.
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($AutoHotkey)) { $AutoHotkey = Join-Path $projectRoot 'ahk\runtime\AutoHotkey64.exe' }
$AutoHotkey = [IO.Path]::GetFullPath($AutoHotkey)
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
    foreach ($name in @('lib\executor.ahk', 'lib\JSON.ahk')) {
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
 Point pressed;
 bool dragging;
 public DragSurface(string title,string path) {
  output=path; Text=title; StartPosition=FormStartPosition.Manual; Location=new Point(40,40); ClientSize=new Size(800,480); TopMost=true; BackColor=Color.FromArgb(33,40,48);
 }
 void Record(string kind, MouseEventArgs e) { File.AppendAllText(output,DateTime.UtcNow.Ticks+","+kind+","+e.X+","+e.Y+","+(int)e.Button+Environment.NewLine); }
 protected override void OnMouseDown(MouseEventArgs e) { base.OnMouseDown(e); pressed=e.Location; dragging=false; Record("down",e); }
 protected override void OnMouseUp(MouseEventArgs e) { base.OnMouseUp(e); Record("up",e); }
 protected override void OnMouseMove(MouseEventArgs e) {
  base.OnMouseMove(e);
  if(e.Button!=MouseButtons.Left) return;
  int dx=e.X-pressed.X, dy=e.Y-pressed.Y;
  if(!dragging && dx*dx+dy*dy>=64) { dragging=true; Record("drag_start",e); }
  Record("move",e);
 }
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
    function Verify-Drag([string]$Name, [int]$Before, [bool]$Identical=$false, [int]$PreDistance=16, [int]$PreDelay=80, [int]$ExpectedX=639, [int]$ExpectedY=383) {
        Wait-For "$Name mouse up" { @(Events).Count -gt $Before -and @(Events)[-1].kind -eq 'up' }
        $all = @(Events)
        $ev = @($all | Select-Object -Skip $Before)
        $down = @($ev | Where-Object kind -eq down)
        $up = @($ev | Where-Object kind -eq up)
        Assert ($down.Count -eq 1 -and $up.Count -eq 1) "$Name expected one left down/up"
        Assert ($down[0].button -eq 1048576 -and $up[0].button -eq 1048576) "$Name expected left button"
        $endX = if ($Identical) {160} else {$ExpectedX}
        $endY = if ($Identical) {96} else {$ExpectedY}
        Assert ($down[0].x -eq 160 -and $down[0].y -eq 96) "$Name wrong start"
        Assert ($up[0].x -eq $endX -and $up[0].y -eq $endY) "$Name wrong endpoint"
        $start = @($ev | Where-Object kind -eq drag_start)
        if ($Identical) {
            Assert ($start.Count -eq 0) "$Name unexpectedly started dragging"
        } elseif ($PreDistance -ge 8) {
            Assert ($start.Count -eq 1) "$Name did not cross the simulated map drag threshold"
            $startDistance = [math]::Sqrt([math]::Pow($start[0].x-160,2)+[math]::Pow($start[0].y-96,2))
            Assert ($startDistance -le ($PreDistance+1)) "$Name map drag began only during main movement"
            $length = [math]::Sqrt([math]::Pow($endX-160,2)+[math]::Pow($endY-96,2))
            $ratio = [math]::Min($PreDistance/$length,1)
            $preX = [int][math]::Round(160+($endX-160)*$ratio)
            $preY = [int][math]::Round(96+($endY-96)*$ratio)
            $preEvents = @($ev | Where-Object { $_.kind -eq 'move' -and $_.x -eq $preX -and $_.y -eq $preY })
            Assert ($preEvents.Count -gt 0) "$Name missing predrag endpoint"
            $pressWait = ($preEvents[0].ticks-$down[0].ticks)/10000
            Assert ($pressWait -ge 40) "$Name did not wait for the initial press"
            $afterPre = @($ev | Where-Object { $_.ticks -gt $preEvents[0].ticks -and $_.kind -in @('move','up') -and ($_.x -ne $preX -or $_.y -ne $preY -or $_.kind -eq 'up') })
            Assert ($afterPre.Count -gt 0) "$Name missing movement or release after predrag"
            $settleMs = ($afterPre[0].ticks-$preEvents[0].ticks)/10000
            Assert ($settleMs -ge ($PreDelay-20)) "$Name continued before the predrag pause elapsed: $settleMs ms"
        }
        Assert (([NativeDragSmoke]::GetAsyncKeyState(1) -band 0x8000) -eq 0) "$Name left remains down"
        $ev | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $fixture "$Name.events.json") -Encoding utf8
        $result = [pscustomobject]@{name=$Name;downUpMs=[math]::Round(($up[0].ticks-$down[0].ticks)/10000,2);moveEvents=@($ev | Where-Object kind -eq move).Count;leftReleased=$true;endX=$up[0].x;endY=$up[0].y}
        $results.Add($result)
        return $result
    }
    $daemonProcess = Launch $AutoHotkey @('/ErrorStdOut',('"'+(Join-Path $fixture 'ahk\daemon.ahk')+'"')) 'daemon'
    $script:daemonHwnd = [IntPtr]::Zero
    Wait-For 'isolated daemon hidden window' { $script:daemonHwnd=[NativeDragSmoke]::FindWindow([IntPtr]::Zero,$daemonTitle); $script:daemonHwnd -ne [IntPtr]::Zero -or $daemonProcess.HasExited }
    Assert (-not $daemonProcess.HasExited) 'Isolated daemon exited during startup'
    [NativeDragSmoke]::GetWindowThreadProcessId($daemonHwnd,[ref]$windowPid) | Out-Null
    Assert ($windowPid -eq $daemonProcess.Id) 'Daemon HWND ownership mismatch'
    $ping = [NativeDragSmoke]::RegisterWindowMessage('GameHelperDaemon_Ping')
    function Daemon-Idle {
        [UIntPtr]$reply = [UIntPtr]::Zero
        $sent = [NativeDragSmoke]::SendMessageTimeout($daemonHwnd,$ping,[IntPtr]::Zero,[IntPtr]::Zero,2,3000,[ref]$reply)
        Assert ($sent -ne [IntPtr]::Zero) 'Isolated daemon Ping failed'
        Assert (($reply.ToUInt64() -shr 1) -eq 0) 'Isolated daemon registered unexpected route hotkeys'
        return ($reply.ToUInt64() -band 1) -eq 0
    }
    Wait-For 'isolated daemon ready' { Daemon-Idle }
    foreach ($case in @(
        @{name='daemon-default';speed=$null},
        @{name='daemon-speed0';speed=0},
        @{name='daemon-speed40';speed=40},
        @{name='daemon-identical';speed=15;identical=$true},
        @{name='daemon-disabled';speed=0;distance=0},
        @{name='daemon-short';speed=0;short=$true},
        @{name='daemon-custom';speed=0;distance=32;delay=180;preSpeed=20},
        @{name='daemon-zero-delay';speed=0;delay=0}
    )) {
        $op = @{type='drag_native';from=@(0.2,0.2);to=@(0.8,0.8)}
        if ($null -ne $case.speed) { $op.speed=$case.speed }
        if ($case.identical) { $op.to=@(0.2,0.2) }
        if ($case.short) { $op.to=@(0.21,0.21) }
        if ($case.ContainsKey('distance')) { $op.pre_drag_distance=$case.distance }
        if ($case.ContainsKey('delay')) { $op.pre_drag_delay=$case.delay }
        if ($case.ContainsKey('preSpeed')) { $op.pre_drag_speed=$case.preSpeed }
        $expectedDistance = if ($case.ContainsKey('distance')) {$case.distance} else {16}
        $expectedDelay = if ($case.ContainsKey('delay')) {$case.delay} else {80}
        Route $case.name @($op)
        Foreground-Target
        $before = @(Events).Count
        Wait-For "$($case.name) daemon idle before request" { Daemon-Idle }
        $payload = @{route=$case.name;startAt=1;only=1} | ConvertTo-Json -Compress
        $accepted = [NativeDragSmoke]::Run($daemonHwnd,$payload)
        Assert ($accepted -eq 1) "$($case.name) daemon rejected the run request"
        $expectedX = if ($case.short) {168} else {639}
        $expectedY = if ($case.short) {101} else {383}
        $null = Verify-Drag $case.name $before ([bool]$case.identical) $expectedDistance $expectedDelay $expectedX $expectedY
        Wait-For "$($case.name) daemon idle after playback" { Daemon-Idle } 20000
    }
    $fast = $results | Where-Object name -eq daemon-speed0
    $slow = $results | Where-Object name -eq daemon-speed40
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
stopStage := "", stopOffset := 0, stopTriggered := false
StopAtStage() {
    global stopStage, stopOffset, stopTriggered
    recent := SubStr(FileRead(EXE_LogFile, "UTF-8"), stopOffset + 1)
    if RegExMatch(recent, "s).*drag_native stage=(\w+)", &match) && match[1] = stopStage {
        SetTimer(StopAtStage, 0)
        stopTriggered := true
        EXE_StopAllHandler()
    }
}
stopCases := Map()
for wanted in ["press_wait", "pre_move", "pre_wait", "main_move"] {
    stopStage := wanted
    stopTriggered := false
    stopOffset := StrLen(FileRead(EXE_LogFile, "UTF-8"))
    route["name"] := "stop-" wanted
    op := route["points"][1]["operations"][1]
    op["pre_drag_distance"] := 160
    op["pre_drag_speed"] := 40
    op["pre_drag_delay"] := 400
    SetTimer(StopAtStage, 10)
    ok := EXE_RunRoute(route)
    SetTimer(StopAtStage, 0)
    recent := SubStr(FileRead(EXE_LogFile, "UTF-8"), stopOffset + 1)
    stopCases[wanted] := Map(
        "triggered", stopTriggered,
        "returnsFalse", !ok,
        "leftReleased", !GetKeyState("LButton"),
        "restoresInput", A_SendMode = "Input",
        "clearsRunning", !EXE_Running,
        "correctStage", InStr(recent, "stopped stage=" wanted) > 0,
        "skipsNextAction", InStr(recent, "STOP at point#1 op#2") > 0 && !InStr(recent, "op key {F23}")
    )
}
report["stopCases"] := stopCases
; A logging failure after mouse-down is a real exception path through cleanup.
savedLogFile := EXE_LogFile
BreakLogOnHold() {
    global EXE_LogFile
    if GetKeyState("LButton") {
        SetTimer(BreakLogOnHold, 0)
        EXE_LogFile := A_ScriptDir ; FileAppend cannot write to a directory.
    }
}
caught := false
SetTimer(BreakLogOnHold, 10)
try {
    EXE_ExecuteOp(Map("type", "drag_native", "from", [0.2,0.2], "to", [0.8,0.8]), client)
} catch {
    caught := true
} finally {
    SetTimer(BreakLogOnHold, 0)
    EXE_LogFile := savedLogFile
}
report["exceptionCaught"] := caught
report["exceptionLeftReleased"] := !GetKeyState("LButton")
report["exceptionRestoresInput"] := A_SendMode = "Input"
FileAppend(JSON.stringify(report),A_ScriptDir "\harness-result.json","UTF-8")
ExitApp(0)
'@
    Route 'harness-stop' @(@{type='drag_native';from=@(0.2,0.2);to=@(0.8,0.8);speed=60},@{type='key';key='F23'})
    Foreground-Target
    $harness = Launch $AutoHotkey @('/ErrorStdOut',('"'+(Join-Path $fixture 'harness.ahk')+'"'),('"'+(Join-Path $fixture 'routes\_cache\harness-stop.json')+'"')) 'harness'
    Wait-For 'deterministic executor harness exit' { $harness.HasExited } 20000
    Assert ($harness.ExitCode -eq 0) 'Harness failed: inspect harness.stderr.txt and stdout'
    $harnessReport = [IO.File]::ReadAllText((Join-Path $fixture 'harness-result.json')) | ConvertFrom-Json
    foreach ($key in @('defaultRestoresInput','restoresPlay','earlyStopRestoresInput','exceptionCaught','exceptionLeftReleased','exceptionRestoresInput')) { Assert $harnessReport.$key "Harness assertion failed: $key" }
    foreach ($stage in @('press_wait','pre_move','pre_wait','main_move')) {
        foreach ($key in @('triggered','returnsFalse','leftReleased','restoresInput','clearsRunning','correctStage','skipsNextAction')) {
            Assert $harnessReport.stopCases.$stage.$key "Harness stop assertion failed: $stage.$key"
        }
    }
    $harnessLog = @(Get-ChildItem -LiteralPath (Join-Path $fixture 'logs') -Filter '*.log' | Sort-Object LastWriteTime)[-1]
    $logText = [IO.File]::ReadAllText($harnessLog.FullName)
    Assert ($logText.Contains('STOP at point#1 op#2')) 'Stop did not skip the following action'
    Assert (-not $logText.Contains('op key {F23}')) 'Following action executed despite stop'
    Assert ($logText.Contains('speed=2')) 'Omitted native speed did not resolve to 2'
    Copy-Item -LiteralPath $harnessLog.FullName -Destination (Join-Path $fixture 'harness-executor.log')
    Start-Sleep -Milliseconds 1100
    Route 'daemon-native' @(@{type='drag_native';from=@(0.2,0.2);to=@(0.8,0.8);speed=20})
    Foreground-Target
    Wait-For 'isolated daemon idle before final playback' { Daemon-Idle }
    Foreground-Target
    $before = @(Events).Count
    $accepted = [NativeDragSmoke]::Run($daemonHwnd,'{"route":"daemon-native","startAt":1,"only":0}')
    Assert ($accepted -eq 1) 'Daemon WM_COPYDATA run was rejected'
    $null = Verify-Drag 'daemon-native' $before
    Assert ([NativeDragSmoke]::FindWindow([IntPtr]::Zero,'game_helper_daemon_v2') -eq $existingDaemon) 'Existing user daemon window changed'
    $summary = [ordered]@{passed=$true;fixture=$fixture;interpreter=$AutoHotkey;targetPid=$target.Id;daemonPid=$daemonProcess.Id;existingDaemonUnchanged=$true;daemonHash=(Get-FileHash (Join-Path $fixture 'ahk\daemon.ahk')).Hash;executorHash=(Get-FileHash (Join-Path $fixture 'ahk\lib\executor.ahk')).Hash;cases=@($results);harness=$harnessReport}
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
